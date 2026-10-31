//! The parts of the Scryer desktop wrapper that are the same on every platform.
//!
//! Both wrappers supervise the same `scryer` server process, poll the same
//! readiness surface, and answer the same question about which links belong
//! inside the app window. Keeping those rules here is what keeps the
//! platform modules down to window and menu plumbing, and it is the only way
//! the two platforms can be relied on to behave identically.
#![allow(
    dead_code,
    reason = "the Windows and macOS wrappers each use a subset of this module, and neither is compiled on other platforms"
)]

#[path = "../environment_file.rs"]
mod environment_file;
use std::io::{Read, Seek, SeekFrom, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};
use std::thread;
use std::time::{Duration, Instant};

/// The port the wrapper starts `scryer` on, and the port it expects the UI to
/// answer on. Users who want a different port run the server themselves; the
/// wrapper owns this one.
///
/// This is the port earlier Scryer desktop builds used, kept so an in-place
/// upgrade keeps answering where the user's bookmarks point.
pub(crate) const DEFAULT_PORT: u16 = 8080;

/// How long a start or restart waits for the server to answer before the
/// wrapper reports the failure to the user. Scryer's first start applies
/// database migrations, so this is the 45s the shipped tray already allowed
/// rather than the shorter budget a server with no bootstrap work needs.
pub(crate) const SERVER_READY_TIMEOUT: Duration = Duration::from_secs(45);

/// How long a stop waits after asking the server to shut down (`SIGTERM` on
/// Unix, a console `CTRL_C_EVENT` on Windows) before killing it. The server's
/// own graceful teardown is what needs the time here; a process that ignores
/// the request must not be able to strand the wrapper.
const GRACEFUL_STOP_TIMEOUT: Duration = Duration::from_secs(10);

/// The names the wrapper and the server are installed under. Both binaries
/// ship in the same directory, which is what lets the wrapper find the server
/// without a configured path.
#[cfg(windows)]
const SERVER_EXECUTABLE: &str = "scryer.exe";
#[cfg(windows)]
const WRAPPER_EXECUTABLE: &str = "scryer-tray.exe";
#[cfg(not(windows))]
const SERVER_EXECUTABLE: &str = "scryer";
#[cfg(not(windows))]
const WRAPPER_EXECUTABLE: &str = "scryer-tray";

fn configure_environment(
    command: &mut Command,
    profile: &Path,
    inherited: impl IntoIterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
) -> Result<(), String> {
    let inherited: Vec<_> = inherited.into_iter().collect();
    let configured = environment_file::read(&profile.join(".env"))?;
    for name in inherited
        .iter()
        .map(|(name, _)| name.to_string_lossy())
        .chain(
            configured
                .iter()
                .map(|(name, _)| std::borrow::Cow::Borrowed(name.as_str())),
        )
    {
        if environment_file::desktop_owned(&name) {
            return Err(format!(
                "{name} is managed by the desktop app; remove this override or run the standalone server"
            ));
        }
    }
    for (name, value) in configured {
        if !inherited.iter().any(|(key, _)| {
            if cfg!(windows) {
                key.to_string_lossy().eq_ignore_ascii_case(&name)
            } else {
                key == name.as_str()
            }
        }) {
            command.env(name, value);
        }
    }
    Ok(())
}

/// The origin the app window is allowed to stay inside.
///
/// The wrapper owns the loopback listener and root base path.
pub(crate) fn app_origin(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

/// The document the app window opens.
pub(crate) fn app_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}/")
}

/// Whether a restore is running right now. The record alone is not enough: a
/// server that crashed mid-restore leaves an active phase on disk. Only an
/// active phase whose owning process is a live Scryer server counts; an
/// unreadable, stale or ownerless record never holds the wrapper open.
fn restore_processing_active(profile: &Path, owner_is_live_server: impl Fn(u32) -> bool) -> bool {
    let Ok(bytes) = std::fs::read(profile.join("restore-job.json")) else {
        return false;
    };
    let Ok(record) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return false;
    };
    let active = matches!(
        record.get("phase").and_then(serde_json::Value::as_str),
        Some("inspecting" | "draining" | "restoring" | "cancelling" | "restarting")
    );
    active
        && record
            .get("pid")
            .and_then(serde_json::Value::as_u64)
            .and_then(|pid| u32::try_from(pid).ok())
            .is_some_and(owner_is_live_server)
}

fn readiness_deadline_expired(
    profile: &Path,
    now: Instant,
    deadline: Instant,
    owner_is_live_server: impl Fn(u32) -> bool,
) -> bool {
    now >= deadline && !restore_processing_active(profile, owner_is_live_server)
}

/// Whether `pid` is a running server from this installation.
#[cfg(windows)]
fn restore_owner_is_live_server(pid: u32) -> bool {
    installation_dir().is_ok_and(|install_dir| {
        application_updater::windows_process::find_product_processes(
            &[SERVER_EXECUTABLE],
            &install_dir,
            &[],
        )
        .contains(&pid)
    })
}

/// Whether `pid` is a running server from this installation.
#[cfg(unix)]
fn restore_owner_is_live_server(pid: u32) -> bool {
    let Some(image) = process_image(pid) else {
        return false;
    };
    let Ok(wrapper) = std::env::current_exe() else {
        return false;
    };
    let expected = wrapper.with_file_name(SERVER_EXECUTABLE);
    let canonical = |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.into());
    canonical(&image) == canonical(&expected)
}

#[cfg(not(any(windows, unix)))]
fn restore_owner_is_live_server(_pid: u32) -> bool {
    false
}

/// The executable a live process runs, or `None` when it is not running.
#[cfg(target_os = "macos")]
fn process_image(pid: u32) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    let pid = libc::c_int::try_from(pid).ok()?;
    let mut buffer = vec![0_u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // SAFETY: The buffer is writable for the length passed; the call writes
    // at most that many bytes and returns how many it wrote.
    let written =
        unsafe { libc::proc_pidpath(pid, buffer.as_mut_ptr().cast(), buffer.len() as u32) };
    let written = usize::try_from(written).ok().filter(|length| *length > 0)?;
    Some(PathBuf::from(std::ffi::OsStr::from_bytes(
        &buffer[..written],
    )))
}

/// The executable a live process runs, or `None` when it is not running.
#[cfg(all(unix, not(target_os = "macos")))]
fn process_image(pid: u32) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/exe")).ok()
}

/// Where the desktop wrapper keeps the server's configuration, database and
/// logs. This is deliberately not the portable layout the tarball uses: a
/// wrapper install writes to per-user application data, and a portable install
/// writes beside itself, and the two must never collide.
#[cfg(windows)]
pub(crate) fn desktop_profile_dir() -> Result<PathBuf, String> {
    let local_app_data = std::env::var_os("LOCALAPPDATA")
        .ok_or_else(|| "LOCALAPPDATA is not set; cannot locate Scryer desktop data".to_string())?;
    Ok(desktop_profile_dir_from(Path::new(&local_app_data)))
}

#[cfg(target_os = "macos")]
pub(crate) fn desktop_profile_dir() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME")
        .ok_or_else(|| "HOME is not set; cannot locate Scryer desktop data".to_string())?;
    Ok(desktop_profile_dir_from(
        &Path::new(&home).join("Library").join("Application Support"),
    ))
}

/// The vendor/product suffix both platforms append to their per-user data
/// root, split out so the layout can be asserted without an environment.
///
/// `ScryerMedia\Scryer` is the layout the shipped Windows tray already writes,
/// and an upgrade must keep finding the same profile.
pub(crate) fn desktop_profile_dir_from(application_data: &Path) -> PathBuf {
    application_data
        .join(PROFILE_VENDOR_DIR)
        .join(PROFILE_PRODUCT_DIR)
}

const PROFILE_VENDOR_DIR: &str = "ScryerMedia";
const PROFILE_PRODUCT_DIR: &str = "Scryer";

/// Whether a navigation the app window is about to perform belongs in the
/// user's browser instead.
///
/// Only absolute `http`/`https` navigations are redirected. Everything else —
/// `about:blank` while a webview initializes, `data:` and `blob:` documents
/// the app itself creates, and schemes the platform webview already refuses —
/// is left to the webview, because handing those to the shell would either do
/// nothing or hand the user's shell a document the app generated.
pub(crate) fn opens_in_external_browser(app_origin: &str, url: &str) -> bool {
    match http_origin(url) {
        Some(origin) => !origin.eq_ignore_ascii_case(app_origin),
        None => false,
    }
}

/// The `scheme://authority` prefix of an absolute `http`/`https` URL.
///
/// This is deliberately a prefix slice rather than a parsed origin: the
/// comparison above is against a string this process built, so anything that
/// is not byte-for-byte (case-insensitively) the same origin — a different
/// port, a host alias, userinfo smuggled into the authority — is correctly
/// treated as somewhere else.
fn http_origin(url: &str) -> Option<&str> {
    let (scheme, rest) = url.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return None;
    }
    let authority_len = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    Some(&url[..scheme.len() + "://".len() + authority_len])
}

/// Poll until the server answers, or the timeout expires.
pub(crate) fn wait_for_server(port: u16, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if server_ready(port) {
            return true;
        }
        thread::sleep(Duration::from_millis(250));
    }
    false
}

/// Whether the server is serving the UI yet.
///
/// A connected socket is not enough: the listener binds before the SPA is
/// mounted, so the wrapper would open a window on an error page. Asking for
/// the document itself is the only readiness signal that means what the user
/// is about to see is there.
///
/// A `200` is not enough either. The wrapper owns a fixed port, and anything
/// else on the machine can be listening on it; accepting whatever answers
/// would skip starting the bundled server and load a stranger's page into the
/// window. The document has to be one of Scryer's.
pub(crate) fn server_ready(port: u16) -> bool {
    probe_port(port) == PortProbe::Scryer
}

/// What one probe of the wrapper's port found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PortProbe {
    /// Nothing answered: the port is free, or a server is still starting. The
    /// server binds its listener before it serves, so a connection accepted in
    /// that window simply gets no response and lands here too.
    NotAnswering,
    /// Something answered, and it was not Scryer's entry page.
    Foreign,
    /// Scryer's entry page came back.
    Scryer,
}

pub(crate) fn probe_port(port: u16) -> PortProbe {
    let request = format!(
        "GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAccept: text/html\r\nConnection: close\r\n\r\n"
    );
    match http_exchange(port, request.as_bytes(), READY_PROBE_TIMEOUT) {
        None => PortProbe::NotAnswering,
        Some(response) if is_scryer_document(&response) => PortProbe::Scryer,
        Some(_) => PortProbe::Foreign,
    }
}

/// How long one readiness probe waits on the socket. Short: the probe runs on
/// a timer while the splash screen is up, and a server that takes longer than
/// this to hand over its entry page is not ready.
const READY_PROBE_TIMEOUT: Duration = Duration::from_millis(500);

/// Whether a response to `GET /` is Scryer's entry page rather than some other
/// program's.
///
/// Every document the server answers `/` with — the SPA shell (`Scryer`), the
/// bootstrap splash (`scryer — starting`) and the bootstrap failure page
/// (`scryer — error`) — carries the product name in its title, and the
/// server's own tests pin those titles. The title is the one thing all of them
/// share that a listener which merely happens to be on the port would not
/// produce. The match is case-insensitive because the splash pages spell the
/// product in lower case.
pub(crate) fn is_scryer_document(response: &HttpResponse) -> bool {
    if response.status != 200 {
        return false;
    }
    let Ok(body) = std::str::from_utf8(&response.body) else {
        return false;
    };
    html_title(body).is_some_and(|title| title.to_ascii_lowercase().contains("scryer"))
}

/// The text of the first `<title>` element, if the document has one.
fn html_title(html: &str) -> Option<&str> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title>")? + "<title>".len();
    let end = lower[start..].find("</title>")? + start;
    Some(html[start..end].trim())
}

// ---------------------------------------------------------------------------
// The snapshot the menu-bar popover and the tray flyout show
// ---------------------------------------------------------------------------

/// How many rows the popover shows. The popover is a glance, not a page.
pub(crate) const POPOVER_ROWS: usize = 5;

/// The width of the popover, in points.
pub(crate) const POPOVER_WIDTH: f64 = 300.0;

/// How long a popover fetch waits on the server before it gives up. Short: a
/// hover that has already ended must not leave a socket open behind it.
const STATUS_FETCH_TIMEOUT: Duration = Duration::from_millis(1500);

/// A response body larger than this is a server the wrapper does not
/// understand, not a status snapshot.
const RESPONSE_LIMIT: usize = 1 << 20;

/// One row of the popover: a label, a state, and a progress bar.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PopoverRow {
    pub(crate) name: String,
    /// The state, in the casing a person reads.
    pub(crate) state: String,
    pub(crate) progress_percent: f64,
}

/// Everything the popover draws.
///
/// `status` is absent exactly when there is no server state to describe — the
/// server did not answer, or it answered with something this wrapper does not
/// recognise — because in those states a status line would be inventing one.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PopoverContent {
    pub(crate) status: Option<String>,
    pub(crate) rows: Vec<PopoverRow>,
    /// Shown instead of rows when there are none.
    pub(crate) message: Option<String>,
}

impl PopoverContent {
    fn message_only(message: &str) -> Self {
        Self {
            status: None,
            rows: Vec::new(),
            message: Some(message.to_string()),
        }
    }

    /// The server is not answering on the port at all.
    pub(crate) fn offline() -> Self {
        Self::message_only("Scryer isn't running")
    }

    /// Something is answering the port, but not with a health snapshot this
    /// build understands. The window is where the user finds out what.
    fn unavailable() -> Self {
        Self::message_only("Status unavailable — open Scryer")
    }
}

/// Ask the running server how it is doing.
///
/// `GET /health` is the one surface a local, unauthenticated peer is entitled
/// to: it is the same probe every orchestrator, the splash page and the web
/// client's restart overlay poll, it carries no user data, and it requires no
/// session. The wrapper deliberately asks for nothing else — the library and
/// queue live behind Scryer's authentication, and a native process is not the
/// web client, so there is no honest way for it to read them. See the module
/// docs in `tray.rs` for what that costs the popover.
pub(crate) fn fetch_popover_content(port: u16) -> PopoverContent {
    let request = format!(
        "GET /health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\
         Accept: application/json\r\nConnection: close\r\n\r\n"
    );
    // Only a connection that never produced a response means the server is
    // gone; anything that answered is running, whatever it answered with.
    let Some(response) = http_exchange(port, request.as_bytes(), STATUS_FETCH_TIMEOUT) else {
        return PopoverContent::offline();
    };
    std::str::from_utf8(&response.body)
        .ok()
        .and_then(popover_content_from_health)
        .unwrap_or_else(PopoverContent::unavailable)
}

/// Map a `GET /health` body onto what the popover draws.
///
/// Returns `None` only when the payload is not a health snapshot at all. An
/// unknown phase is carried through rather than dropped: a server newer than
/// this wrapper must still be described, not reported as broken.
pub(crate) fn popover_content_from_health(body: &str) -> Option<PopoverContent> {
    #[derive(serde::Deserialize)]
    struct Health {
        status: String,
        #[serde(default)]
        ready: bool,
        #[serde(default)]
        message: Option<String>,
        #[serde(default)]
        migrations: Option<Migrations>,
    }
    #[derive(serde::Deserialize)]
    struct Migrations {
        #[serde(default)]
        completed: u64,
        #[serde(default)]
        total: u64,
    }

    let health: Health = serde_json::from_str(body).ok()?;
    match health.status.as_str() {
        "ok" if health.ready => Some(PopoverContent {
            status: Some("Running".to_string()),
            rows: Vec::new(),
            message: Some("Open Scryer for the full view".to_string()),
        }),
        "migrating" => {
            let rows = health
                .migrations
                .filter(|migrations| migrations.total > 0)
                .map(|migrations| {
                    #[allow(
                        clippy::cast_precision_loss,
                        reason = "a migration count is far below the f64 integer range"
                    )]
                    let percent = (migrations.completed as f64 / migrations.total as f64) * 100.0;
                    vec![PopoverRow {
                        name: format!(
                            "Database update {}/{}",
                            migrations.completed, migrations.total
                        ),
                        state: "Updating".to_string(),
                        progress_percent: clamp_percent(percent),
                    }]
                })
                .unwrap_or_default();
            let message = rows.is_empty().then(|| "Updating the database".to_string());
            Some(PopoverContent {
                status: Some("Starting".to_string()),
                rows,
                message,
            })
        }
        "error" => Some(PopoverContent {
            status: Some("Failed to start".to_string()),
            rows: Vec::new(),
            message: Some(
                health
                    .message
                    .unwrap_or_else(|| "Open Logs for the reason".to_string()),
            ),
        }),
        // A phase this build has not heard of, or `ok` before the API is
        // serving: say what the server said rather than guess.
        other => Some(PopoverContent {
            status: Some(humanize_state(other)),
            rows: Vec::new(),
            message: Some("Open Scryer for the full view".to_string()),
        }),
    }
}

/// One request and one response on a connection the server closes.
///
/// `Connection: close` is what makes reading to EOF a complete response, so
/// this needs no keep-alive framing of its own.
fn http_exchange(port: u16, request: &[u8], timeout: Duration) -> Option<HttpResponse> {
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&address, timeout).ok()?;
    stream.set_read_timeout(Some(timeout)).ok()?;
    stream.set_write_timeout(Some(timeout)).ok()?;
    stream.write_all(request).ok()?;

    let mut raw = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            break;
        }
        raw.extend_from_slice(&chunk[..read]);
        if raw.len() > RESPONSE_LIMIT {
            return None;
        }
    }
    parse_http_response(&raw)
}

/// A response split into the three parts the wrapper reads.
#[derive(Debug, PartialEq)]
pub(crate) struct HttpResponse {
    pub(crate) status: u16,
    /// Field names as received; every lookup here is case-insensitive.
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

/// Split a raw response, decoding a chunked body if that is how it arrived.
///
/// Only the two framings axum produces are handled — a declared length and
/// chunked — because a response with neither is one this wrapper did not ask
/// for.
pub(crate) fn parse_http_response(raw: &[u8]) -> Option<HttpResponse> {
    let split = raw.windows(4).position(|window| window == b"\r\n\r\n")?;
    let head = std::str::from_utf8(&raw[..split]).ok()?;
    let body = &raw[split + 4..];

    let mut lines = head.split("\r\n");
    let status = lines
        .next()?
        .split(' ')
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())?;
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_string(), value.trim().to_string()))
        .collect();

    let body = if header_value(&headers, "transfer-encoding")
        .is_some_and(|value| value.to_ascii_lowercase().contains("chunked"))
    {
        decode_chunked(body)?
    } else {
        body.to_vec()
    };
    Some(HttpResponse {
        status,
        headers,
        body,
    })
}

/// The first value for a header name, matched case-insensitively.
fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(field, _)| field.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

/// Reassemble a `Transfer-Encoding: chunked` body.
pub(crate) fn decode_chunked(body: &[u8]) -> Option<Vec<u8>> {
    let mut decoded = Vec::new();
    let mut rest = body;
    loop {
        let end = rest.windows(2).position(|window| window == b"\r\n")?;
        let header = std::str::from_utf8(&rest[..end]).ok()?;
        // A chunk size may carry extensions after a semicolon; nothing here
        // uses them, but they are legal and must not be parsed as digits.
        let size = usize::from_str_radix(header.split(';').next()?.trim(), 16).ok()?;
        rest = rest.get(end + 2..)?;
        if size == 0 {
            return Some(decoded);
        }
        decoded.extend_from_slice(rest.get(..size)?);
        // The CRLF that terminates the chunk data.
        rest = rest.get(size + 2..)?;
    }
}

/// `FETCHING_REPAIR_DATA` is not a label. Unknown phases go through the same
/// transformation rather than being dropped.
fn humanize_state(state: &str) -> String {
    let words = state.trim().replace('_', " ").to_ascii_lowercase();
    let mut characters = words.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().collect::<String>() + characters.as_str(),
        None => "Unknown".to_string(),
    }
}

fn clamp_percent(percent: f64) -> f64 {
    if percent.is_finite() {
        percent.clamp(0.0, 100.0)
    } else {
        0.0
    }
}

/// The secondary line under a row's progress bar.
pub(crate) fn row_detail(row: &PopoverRow) -> String {
    format!("{} · {}%", row.state, row.progress_percent.round())
}

use crate::bundle_relaunch::is_bundle_relaunch_exit;

/// What one poll of the supervised server found.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SupervisedServer {
    /// Still running, or never owned by this wrapper.
    Running,
    /// The server applied an application-bundle upgrade and asked the wrapper
    /// to relaunch the app from the replaced bundle.
    RelaunchRequested,
    /// The server exited for some other reason.
    Exited(ExitStatus),
}

/// The `.app` bundle the running wrapper was launched from, if it was.
///
/// Derived from this process's own path rather than anything the server said,
/// so a relaunch can only ever target the bundle the user actually started.
#[cfg(target_os = "macos")]
pub(crate) fn running_app_bundle() -> Option<PathBuf> {
    let wrapper = std::env::current_exe().ok()?;
    let resolved = std::fs::canonicalize(&wrapper).unwrap_or(wrapper);
    application_updater::installation::macos_app_bundle_path(&resolved).map(Path::to_path_buf)
}

/// Start the updated bundle once this wrapper has exited.
///
/// The wrapper holds the single-instance lock until its process ends, and a
/// second instance that finds the lock taken hands off to the first and exits.
/// Opening the bundle directly would race this wrapper's own shutdown and, when
/// it lost, leave nothing running. So a detached shell waits for this process
/// to go, then opens the bundle. The bundle path travels as an argument, never
/// as script text.
#[cfg(target_os = "macos")]
pub(crate) fn relaunch_app_bundle(bundle: &Path) -> Result<(), String> {
    use std::process::Stdio;

    const WAIT_THEN_OPEN: &str = "while /bin/kill -0 \"$1\" 2>/dev/null; do /bin/sleep 0.2; done; exec /usr/bin/open -n \"$2\"";
    Command::new("/bin/sh")
        .arg("-c")
        .arg(WAIT_THEN_OPEN)
        .arg("scryer-relaunch")
        .arg(std::process::id().to_string())
        .arg(bundle)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("failed to relaunch {}: {error}", bundle.display()))
}

/// The `scryer` server process the wrapper owns.
///
/// The wrapper is the parent of the server it started, so this is also what
/// guarantees the server goes away when the user quits the wrapper. It
/// deliberately tolerates a server it did not start: a user who already has
/// `scryer` running on the port gets the same window, and the wrapper simply
/// has no child to supervise.
pub(crate) struct ServerSupervisor {
    profile_dir: PathBuf,
    port: u16,
    server: Option<Child>,
    /// The kill-on-close job every server this wrapper starts is placed in, so
    /// a wrapper that dies without stopping its server (killed from Task
    /// Manager, crashed, or ended by a frozen installer) cannot leave an
    /// orphan holding the installed executables open. Created on the first
    /// start and held for the wrapper's lifetime; closing it is what kills.
    #[cfg(windows)]
    job: Option<windows_job::KillOnCloseJob>,
    /// How long the log was when the owned server was started, so a failed
    /// start is reported with its own error rather than an earlier run's.
    log_offset: u64,
    /// Decides whether a restore record's owning process is a live server.
    /// Replaceable so tests can stand in for the shipped server binary.
    restore_owner_probe: fn(u32) -> bool,
}

impl ServerSupervisor {
    pub(crate) fn new(profile_dir: PathBuf, port: u16) -> Self {
        Self {
            profile_dir,
            port,
            server: None,
            #[cfg(windows)]
            job: None,
            log_offset: 0,
            restore_owner_probe: restore_owner_is_live_server,
        }
    }

    pub(crate) fn profile_dir(&self) -> &Path {
        &self.profile_dir
    }

    fn restore_active(&self) -> bool {
        restore_processing_active(&self.profile_dir, self.restore_owner_probe)
    }

    pub(crate) fn ensure_restore_idle(&self) -> Result<(), String> {
        if self.restore_active() {
            Err("A backup restore is still running. Keep Scryer open until it finishes.".into())
        } else {
            Ok(())
        }
    }

    pub(crate) fn logs_dir(&self) -> PathBuf {
        self.profile_dir.join("logs")
    }

    fn log_file(&self) -> PathBuf {
        self.logs_dir().join("scryer.log")
    }

    pub(crate) fn port(&self) -> u16 {
        self.port
    }

    /// Whether the wrapper started the server that is running.
    pub(crate) fn owns_server(&self) -> bool {
        self.server.is_some()
    }

    /// Create the profile the server is about to be pointed at. The server
    /// creates its own subdirectories, but it is started with an explicit log
    /// file path, and it cannot create the directory that path lives in.
    pub(crate) fn ensure_profile_dirs(&self) -> Result<(), String> {
        std::fs::create_dir_all(self.logs_dir()).map_err(|error| {
            format!(
                "failed to create Scryer desktop profile at {}: {error}",
                self.profile_dir.display()
            )
        })
    }

    /// Start the server unless something is already serving the port.
    ///
    /// Returning early on a live port is what makes this idempotent for every
    /// caller — menu item, window open, and login start all funnel through
    /// here — but it is also why a caller that has just torn the server down
    /// must wait for the old process to disappear first.
    pub(crate) fn start(&mut self) -> Result<(), String> {
        match probe_port(self.port) {
            PortProbe::Scryer => return Ok(()),
            // Spawning a server here would only make it fail to bind, and the
            // user would be told Scryer timed out rather than what is wrong.
            PortProbe::Foreign => {
                return Err(format!(
                    "port {} is already in use by a program that is not Scryer; stop it, or run Scryer yourself with SCRYER_BIND set to another port",
                    self.port
                ));
            }
            PortProbe::NotAnswering => {}
        }
        if let Some(child) = self.server.as_mut() {
            match child.try_wait() {
                Ok(None) => return Ok(()),
                Ok(Some(_)) => self.server = None,
                Err(error) => {
                    return Err(format!("failed to check Scryer server status: {error}"));
                }
            }
        }

        let server_executable = self.server_executable()?;
        let log_file = self.log_file();
        self.log_offset = std::fs::metadata(&log_file).map_or(0, |metadata| metadata.len());
        let mut command = Command::new(&server_executable);
        configure_environment(&mut command, &self.profile_dir, std::env::vars_os())?;
        command
            .arg("--data-dir")
            .arg(&self.profile_dir)
            .arg("--log-file")
            .arg(&log_file)
            .env("SCRYER_BIND", format!("127.0.0.1:{}", self.port))
            // The wrapper owns the window the user is about to look at; the
            // server must not also open a browser tab.
            .env("SCRYER_OPEN_BROWSER", "false")
            .env("SCRYER_TRAY_SUPERVISED", "1");
        configure_server_console(&mut command, &self.logs_dir())?;
        configure_server_command(&mut command);
        let child = command.spawn().map_err(|error| {
            format!(
                "failed to start Scryer from {}: {error}",
                server_executable.display()
            )
        })?;
        #[cfg(windows)]
        self.place_in_job(&child);
        self.server = Some(child);
        Ok(())
    }

    /// Put a freshly started server in the wrapper's kill-on-close job.
    ///
    /// A server outside the job still runs and is still stopped by
    /// [`Self::stop`]; it only loses the guarantee that it dies with the
    /// wrapper, so a failure here is not a failed start.
    #[cfg(windows)]
    fn place_in_job(&mut self, child: &Child) {
        if self.job.is_none() {
            self.job = windows_job::KillOnCloseJob::new().ok();
        }
        if let Some(job) = self.job.as_ref() {
            let _ = job.assign(child);
        }
    }

    /// Stop the server this wrapper started: ask it to shut down, then kill
    /// it if it has not within [`GRACEFUL_STOP_TIMEOUT`]. A server it did not
    /// start is left to [`Self::stop_unowned_servers`].
    pub(crate) fn stop(&mut self) -> Result<(), String> {
        let Some(mut child) = self.server.take() else {
            return Ok(());
        };
        if child
            .try_wait()
            .map_err(|error| format!("failed to check Scryer server status: {error}"))?
            .is_none()
        {
            if self.restore_active() {
                self.server = Some(child);
                return Err(
                    "A backup restore is still running. Keep Scryer open until it finishes.".into(),
                );
            }
            if request_graceful_stop(&mut child)? {
                return Ok(());
            }
            child
                .kill()
                .map_err(|error| format!("failed to stop Scryer server: {error}"))?;
            child
                .wait()
                .map_err(|error| format!("failed to wait for Scryer server exit: {error}"))?;
        }
        Ok(())
    }

    /// Stop every Scryer server running from this installation that the
    /// wrapper did not start — the server it adopted on start-up, or one
    /// orphaned by an earlier wrapper that was killed. Each is asked to shut
    /// down, then terminated once `policy` runs out. Returns the ids of
    /// servers still running afterwards.
    #[cfg(windows)]
    pub(crate) fn stop_unowned_servers(
        &self,
        policy: application_updater::windows_process::StopPolicy,
    ) -> Result<Vec<u32>, String> {
        self.ensure_restore_idle()?;
        let install_dir = installation_dir()?;
        let owned: Vec<u32> = self.server.iter().map(Child::id).collect();
        Ok(
            application_updater::windows_process::stop_product_processes(
                &[SERVER_EXECUTABLE],
                &install_dir,
                &owned,
                policy,
            )
            .remaining,
        )
    }

    /// Stop and start again, from a user action.
    pub(crate) fn restart(&mut self) -> Result<(), String> {
        self.stop()?;
        self.start()?;
        self.wait_until_ready()
    }

    /// Wait for the server to answer. An owned server that exits first is
    /// reported with the error it logged: a server that cannot start says why
    /// in its log, and waiting out the timeout would only hide it.
    pub(crate) fn wait_until_ready(&mut self) -> Result<(), String> {
        let deadline = Instant::now() + SERVER_READY_TIMEOUT;
        loop {
            if server_ready(self.port) {
                return Ok(());
            }
            if let Some(status) = self.exited_server()? {
                return Err(self.start_failure(status));
            }
            if readiness_deadline_expired(
                &self.profile_dir,
                Instant::now(),
                deadline,
                self.restore_owner_probe,
            ) {
                return Err(format!(
                    "timed out waiting for Scryer to become ready at {}",
                    app_origin(self.port)
                ));
            }
            thread::sleep(Duration::from_millis(250));
        }
    }

    /// What the supervised server is doing right now.
    ///
    /// This is the whole of the server→wrapper channel for a bundle upgrade.
    /// It is deliberately not a socket, a port or a file: the only thing read
    /// here is the exit status of a child *this process started*, which cannot
    /// be produced by anything else on the machine, needs no authentication of
    /// its own, and adds no listening surface.
    pub(crate) fn poll_supervised_server(&mut self) -> Result<SupervisedServer, String> {
        match self.exited_server()? {
            None => Ok(SupervisedServer::Running),
            Some(status) if is_bundle_relaunch_exit(status) => {
                Ok(SupervisedServer::RelaunchRequested)
            }
            Some(status) => Ok(SupervisedServer::Exited(status)),
        }
    }

    /// How the owned server exited, once it has.
    fn exited_server(&mut self) -> Result<Option<ExitStatus>, String> {
        let Some(child) = self.server.as_mut() else {
            return Ok(None);
        };
        let status = child
            .try_wait()
            .map_err(|error| format!("failed to check Scryer server status: {error}"))?;
        if status.is_some() {
            self.server = None;
        }
        Ok(status)
    }

    fn start_failure(&self, status: ExitStatus) -> String {
        let log_file = self.log_file();
        let reason = last_logged_error(&log_file, self.log_offset)
            .unwrap_or_else(|| format!("the server exited ({status})"));
        format!(
            "Scryer could not start: {reason}\n\nThe log is at {}",
            log_file.display()
        )
    }

    /// Wait for the running server to disappear, bounded so a process that
    /// never exits cannot strand the wrapper. The owned child is the reliable
    /// signal; when the wrapper does not own one, the port answering is the
    /// only evidence left. Falls back to the kill path on timeout.
    pub(crate) fn wait_for_exit(&mut self, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            match self.server.as_mut() {
                Some(child) => match child.try_wait() {
                    Ok(Some(_)) => {
                        self.server = None;
                        return;
                    }
                    Ok(None) => {}
                    Err(_) => break,
                },
                None => {
                    if !server_ready(self.port) {
                        return;
                    }
                }
            }
            thread::sleep(Duration::from_millis(250));
        }
        let _ = self.stop();
    }

    /// The server ships beside the wrapper, so its path is derived rather than
    /// searched: picking up a `scryer` from `PATH` would silently run a
    /// different install than the one the user launched.
    fn server_executable(&self) -> Result<PathBuf, String> {
        let wrapper = std::env::current_exe()
            .map_err(|error| format!("failed to resolve {WRAPPER_EXECUTABLE} path: {error}"))?;
        let server = wrapper.with_file_name(SERVER_EXECUTABLE);
        if !server.is_file() {
            return Err(format!(
                "{SERVER_EXECUTABLE} was not found beside {WRAPPER_EXECUTABLE} at {}",
                server.display()
            ));
        }
        Ok(server)
    }
}

/// The directory this wrapper runs from, which is the installation both
/// executables ship in.
#[cfg(windows)]
pub(crate) fn installation_dir() -> Result<PathBuf, String> {
    let wrapper = std::env::current_exe()
        .map_err(|error| format!("failed to resolve {WRAPPER_EXECUTABLE} path: {error}"))?;
    wrapper
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| format!("{} has no parent directory", wrapper.display()))
}

/// The executables this installation runs: the server and this wrapper.
#[cfg(windows)]
pub(crate) const INSTALLED_EXECUTABLES: [&str; 2] = [SERVER_EXECUTABLE, WRAPPER_EXECUTABLE];

/// How much of the end of the log a failed start reads. The error that
/// stopped the server is the last thing it wrote.
const LOG_TAIL_BYTES: u64 = 64 * 1024;

/// The last error the server logged at or after `offset` in `log_file`.
fn last_logged_error(log_file: &Path, offset: u64) -> Option<String> {
    let mut file = std::fs::File::open(log_file).ok()?;
    let len = file.metadata().ok()?.len();
    // A log the server rotated as it opened it starts again at zero.
    let offset = if offset > len { 0 } else { offset };
    file.seek(SeekFrom::Start(
        offset.max(len.saturating_sub(LOG_TAIL_BYTES)),
    ))
    .ok()?;
    let mut tail = Vec::new();
    file.read_to_end(&mut tail).ok()?;
    String::from_utf8_lossy(&tail)
        .lines()
        .rev()
        .find_map(logged_error_message)
}

/// The message of a log line written at ERROR.
///
/// Scryer's default log format is the JSON one (`SCRYER_LOG_FORMAT=json`), so
/// that is read first; the `text` format is read too, because a user who set
/// it must still get a useful failure instead of a bare exit code.
fn logged_error_message(line: &str) -> Option<String> {
    json_logged_error_message(line).or_else(|| text_logged_error_message(line))
}

/// `{"timestamp":…,"level":"ERROR","target":"scryer","fields":{"message":"…"}}`
fn json_logged_error_message(line: &str) -> Option<String> {
    let record: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    if record.get("level")?.as_str()? != "ERROR" {
        return None;
    }
    let message = record.get("fields")?.get("message")?.as_str()?.trim();
    (!message.is_empty()).then(|| message.to_string())
}

/// `2026-09-11T10:00:00.000-04:00 ERROR scryer: …`, without its timestamp,
/// level and target.
fn text_logged_error_message(line: &str) -> Option<String> {
    let (timestamp, rest) = line.split_once(" ERROR ")?;
    // The level follows the timestamp; a line whose message mentions ERROR
    // has more than a timestamp before it.
    if timestamp.trim().contains(' ') {
        return None;
    }
    // The target is a module path, so it has no spaces.
    let message = match rest.split_once(": ") {
        Some((target, message)) if !target.contains(' ') => message,
        _ => rest,
    };
    Some(message.trim().to_string())
}

/// Folders an uninstall keeps while they hold anything. Scryer's library and
/// download paths are configured outside the profile, but backups default to
/// living inside it, and a backup is the user's.
const KEPT_PROFILE_FOLDERS: [&str; 1] = ["backups"];

/// Remove the desktop profile — the database, logs, WebView2 data and every
/// other file Scryer keeps there — except folders holding the user's own data
/// that still hold anything. The profile and its vendor folder go too once
/// empty. Returns what could not be removed.
pub(crate) fn remove_desktop_profile(profile_dir: &Path) -> Vec<String> {
    // The caller is the uninstall custom action, and a recursive delete that
    // ran anywhere else would be unrecoverable. Nothing but the desktop
    // profile's own shape is accepted, whatever the caller passed.
    if !is_desktop_profile_shape(profile_dir) {
        return vec![format!(
            "refusing to clean up {}: it is not a Scryer desktop profile directory",
            profile_dir.display()
        )];
    }
    let entries = match std::fs::read_dir(profile_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => return vec![format!("cannot read {}: {error}", profile_dir.display())],
    };
    let mut problems = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let kept = entry.file_name().to_str().is_some_and(|name| {
            KEPT_PROFILE_FOLDERS
                .iter()
                .any(|kept| name.eq_ignore_ascii_case(kept))
        });
        if kept {
            // Refused, as intended, while the folder holds anything.
            let _ = std::fs::remove_dir(&path);
            continue;
        }
        match remove_path(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => problems.push(format!("cannot remove {}: {error}", path.display())),
        }
    }
    let _ = std::fs::remove_dir(profile_dir);
    if let Some(vendor_dir) = profile_dir.parent() {
        let _ = std::fs::remove_dir(vendor_dir);
    }
    problems
}

/// Whether `path` has the shape of the desktop profile: an absolute path whose
/// last two components are the vendor and product folders
/// [`desktop_profile_dir`] builds, with no `..` anywhere in it.
///
/// This is deliberately a shape test rather than a comparison against
/// [`desktop_profile_dir`]: it is the only check the tests can make on a
/// temporary directory, and it is the property that actually matters — the
/// cleanup must not be able to escape into a media library, which lives
/// outside the profile and is never named by it. Entries inside the profile
/// are removed with `symlink_metadata`, so a junction or symlink planted there
/// is unlinked rather than followed out of the profile.
fn is_desktop_profile_shape(path: &Path) -> bool {
    if !path.is_absolute() {
        return false;
    }
    if path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return false;
    }
    let mut tail = path
        .components()
        .rev()
        .filter_map(|component| match component {
            std::path::Component::Normal(name) => name.to_str(),
            _ => None,
        });
    let product = tail.next();
    let vendor = tail.next();
    product.is_some_and(|name| name.eq_ignore_ascii_case(PROFILE_PRODUCT_DIR))
        && vendor.is_some_and(|name| name.eq_ignore_ascii_case(PROFILE_VENDOR_DIR))
}

/// Remove a file, or a folder and everything below it. A link is removed
/// itself, never what it points at.
fn remove_path(path: &Path) -> std::io::Result<()> {
    if std::fs::symlink_metadata(path)?.file_type().is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        // Windows removes a link to a folder as a folder.
        std::fs::remove_file(path).or_else(|_| std::fs::remove_dir(path))
    }
}

/// Preserve operator-only process output when the desktop has no console.
/// This stream is separate from tracing and the web-accessible log ring.
fn configure_server_console(command: &mut Command, logs_dir: &Path) -> Result<(), String> {
    let path = logs_dir.join("console.log");
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let output = options.open(&path).map_err(|error| {
        format!(
            "failed to capture server console at {}: {error}",
            path.display()
        )
    })?;
    command.stderr(output);
    Ok(())
}

/// Keep the server out of the user's face. On Windows a console subsystem
/// child would flash a window on every start; on macOS the child inherits the
/// wrapper's already-windowless session and needs nothing.
///
/// `CREATE_NO_WINDOW` provides no console for control events. The private
/// stdin pipe instead lets the wrapper request shutdown by closing its end.
#[cfg(windows)]
fn configure_server_command(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

    command
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(std::process::Stdio::piped());
}

#[cfg(not(windows))]
fn configure_server_command(_command: &mut Command) {}

#[cfg(windows)]
mod windows_job {
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;
    use std::ptr;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_BREAKAWAY_OK,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectExtendedLimitInformation, SetInformationJobObject,
    };

    /// A job object that kills every process in it when its last handle
    /// closes — when the wrapper exits for any reason, including being killed.
    ///
    /// The handle is created without inheritance, so no child holds it open
    /// past the wrapper. Breakaway is allowed: the upgrade helper the server
    /// starts asks to break away, because it must outlive the wrapper it is
    /// about to stop.
    pub(super) struct KillOnCloseJob(HANDLE);

    impl KillOnCloseJob {
        pub(super) fn new() -> Result<Self, String> {
            // SAFETY: Null attributes (non-inheritable) and no name.
            let handle = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
            if handle.is_null() {
                return Err(format!(
                    "failed to create the Scryer server job object: {}",
                    std::io::Error::last_os_error()
                ));
            }
            let job = Self(handle);
            // SAFETY: Zero is the documented initializer for this structure.
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
            limits.BasicLimitInformation.LimitFlags =
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_BREAKAWAY_OK;
            // SAFETY: `limits` is a fully initialized structure of the size
            // passed, and the job handle is open.
            let ok = unsafe {
                SetInformationJobObject(
                    job.0,
                    JobObjectExtendedLimitInformation,
                    (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
            };
            if ok == 0 {
                return Err(format!(
                    "failed to configure the Scryer server job object: {}",
                    std::io::Error::last_os_error()
                ));
            }
            Ok(job)
        }

        pub(super) fn assign(&self, child: &Child) -> Result<(), String> {
            // SAFETY: Both handles are open: the job by this value, the
            // process by `child`.
            let ok = unsafe { AssignProcessToJobObject(self.0, child.as_raw_handle() as HANDLE) };
            if ok == 0 {
                return Err(format!(
                    "failed to place the Scryer server in its job object: {}",
                    std::io::Error::last_os_error()
                ));
            }
            Ok(())
        }
    }

    impl Drop for KillOnCloseJob {
        fn drop(&mut self) {
            // SAFETY: This value owns the job handle.
            unsafe { CloseHandle(self.0) };
        }
    }
}

/// Ask the server to shut down cleanly, and report whether it did.
///
/// The server writes a database on every job; killing it outright is safe but
/// throws away the flush it would otherwise do, so the request comes first and
/// the caller only escalates when this returns false. On Unix the request is
/// `SIGTERM`; on Windows closing the private stdin pipe signals the server's
/// supervised shutdown listener, without requiring a console.
fn request_graceful_stop(child: &mut Child) -> Result<bool, String> {
    #[cfg(unix)]
    {
        // SAFETY: `child` is a live process this wrapper started, so its PID
        // is still ours to signal and cannot have been recycled.
        unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGTERM) };
    }
    #[cfg(windows)]
    {
        let Some(stdin) = child.stdin.take() else {
            return Ok(false);
        };
        drop(stdin);
    }

    let deadline = Instant::now() + GRACEFUL_STOP_TIMEOUT;
    while Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(_)) => return Ok(true),
            Ok(None) => {}
            Err(error) => return Err(format!("failed to check Scryer server status: {error}")),
        }
        thread::sleep(Duration::from_millis(100));
    }
    Ok(false)
}

/// A single-page HTTP server used only by `--webview-smoke`.
///
/// The smoke test has to prove the webview stack renders a real network
/// document, and it has to do that on a machine where no Scryer server is
/// installed or running — so it serves its own.
pub(crate) fn start_smoke_server() -> Result<u16, String> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|error| format!("failed to bind the smoke test HTTP server: {error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("failed to read the smoke test HTTP server port: {error}"))?
        .port();

    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
            // The request is drained rather than parsed: every request gets the
            // same document, and the only thing that matters is that the socket
            // is not closed before the client finished writing.
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request);
            let _ = stream.write_all(SMOKE_RESPONSE.as_bytes());
            let _ = stream.flush();
        }
    });

    Ok(port)
}

const SMOKE_BODY: &str = "<!doctype html><title>Scryer webview smoke</title><p>ok</p>";

/// Written verbatim, including the length, so the smoke server needs no HTTP
/// implementation at all.
const SMOKE_RESPONSE: &str = concat!(
    "HTTP/1.1 200 OK\r\n",
    "Content-Type: text/html; charset=utf-8\r\n",
    "Content-Length: 59\r\n",
    "Connection: close\r\n",
    "\r\n",
    "<!doctype html><title>Scryer webview smoke</title><p>ok</p>",
);

/// How long the smoke test waits for the webview to report a result before it
/// declares the wiring broken. Generous, because a cold WebView2 or WebKit
/// process launch on a loaded CI machine is slow.
pub(crate) const SMOKE_TIMEOUT: Duration = Duration::from_secs(90);

/// The line CI greps for. Printed to stdout only on success.
pub(crate) const SMOKE_SUCCESS_LINE: &str = "webview-smoke: ok";

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::path::{Path, PathBuf};

    use super::{
        HttpResponse, PROFILE_PRODUCT_DIR, PROFILE_VENDOR_DIR, PopoverContent, PopoverRow,
        SMOKE_BODY, SMOKE_RESPONSE, app_origin, app_url, configure_environment, decode_chunked,
        desktop_profile_dir_from, http_origin, is_scryer_document, last_logged_error,
        logged_error_message, opens_in_external_browser, parse_http_response,
        popover_content_from_health, remove_desktop_profile, row_detail,
    };

    #[test]
    fn background_restore_desktop_deadline_waits_only_for_a_live_restore_owner() {
        let profile = tempfile::tempdir().unwrap();
        let record = profile.path().join("restore-job.json");
        let started = std::time::Instant::now();
        let deadline = started + super::SERVER_READY_TIMEOUT;
        let later = started + std::time::Duration::from_secs(7200);
        let live_owner = std::process::id();
        let is_live = |pid: u32| pid == std::process::id();
        let supervisor = || {
            let mut supervisor = super::ServerSupervisor::new(profile.path().to_owned(), 0);
            supervisor.restore_owner_probe = |pid| pid == std::process::id();
            supervisor
        };
        assert!(super::readiness_deadline_expired(
            profile.path(),
            later,
            deadline,
            is_live
        ));
        for phase in [
            "inspecting",
            "draining",
            "restoring",
            "cancelling",
            "restarting",
        ] {
            std::fs::write(
                &record,
                serde_json::json!({"phase": phase, "pid": live_owner}).to_string(),
            )
            .unwrap();
            assert!(!super::readiness_deadline_expired(
                profile.path(),
                later,
                deadline,
                is_live
            ));
            assert!(supervisor().ensure_restore_idle().is_err());
            // The same phase left behind by a server that is gone, or by a
            // record that names no owner, must not hold the wrapper open.
            std::fs::write(
                &record,
                serde_json::json!({"phase": phase, "pid": live_owner.wrapping_add(1)}).to_string(),
            )
            .unwrap();
            assert!(super::readiness_deadline_expired(
                profile.path(),
                later,
                deadline,
                is_live
            ));
            supervisor().ensure_restore_idle().unwrap();
            std::fs::write(&record, serde_json::json!({"phase": phase}).to_string()).unwrap();
            supervisor().ensure_restore_idle().unwrap();
        }
        // A damaged record is not evidence that a restore is running.
        std::fs::write(&record, b"incomplete").unwrap();
        assert!(super::readiness_deadline_expired(
            profile.path(),
            later,
            deadline,
            is_live
        ));
        supervisor().ensure_restore_idle().unwrap();
        for phase in ["complete", "failed", "interrupted"] {
            std::fs::write(
                &record,
                serde_json::json!({"phase": phase, "pid": live_owner}).to_string(),
            )
            .unwrap();
            assert!(super::readiness_deadline_expired(
                profile.path(),
                later,
                deadline,
                is_live
            ));
            assert!(!super::readiness_deadline_expired(
                profile.path(),
                started,
                deadline,
                is_live
            ));
        }
    }

    #[cfg(unix)]
    #[test]
    fn background_restore_owner_probe_rejects_processes_that_are_not_the_server() {
        // The test binary is alive but is not the server shipped beside it.
        assert!(!super::restore_owner_is_live_server(std::process::id()));
        assert!(!super::restore_owner_is_live_server(u32::MAX));
    }

    #[cfg(windows)]
    #[test]
    fn windowless_shutdown_child() {
        use std::io::{Read, Write};
        if std::env::var("SCRYER_TEST_SHUTDOWN_CHILD").as_deref() != Ok("1") {
            return;
        }
        println!("shutdown-child-ready");
        std::io::stdout().flush().unwrap();
        assert_eq!(std::io::stdin().read(&mut [0]).unwrap(), 0);
        println!("shutdown-child-finished");
    }

    #[cfg(windows)]
    #[test]
    fn background_restore_windows_supervisor_preserves_active_child() {
        use std::io::{BufRead, BufReader};
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "shared::tests::windowless_shutdown_child",
                "--nocapture",
            ])
            .env("SCRYER_TEST_SHUTDOWN_CHILD", "1")
            .stdout(std::process::Stdio::piped());
        super::configure_server_command(&mut command);
        let mut child = command.spawn().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            let mut finished = false;
            for line in BufReader::new(stdout).lines() {
                let line = line.unwrap();
                if line.contains("shutdown-child-ready") {
                    let _ = ready_tx.send(());
                }
                finished |= line.contains("shutdown-child-finished");
            }
            finished
        });
        let ready = ready_rx
            .recv_timeout(std::time::Duration::from_secs(60))
            .is_ok();
        if !ready {
            let _ = child.kill();
            let _ = child.wait();
            reader.join().unwrap();
            panic!("synthetic child did not signal readiness");
        }

        let profile = tempfile::tempdir().unwrap();
        let coordinator = scryer_application::RestoreCoordinator::default();
        let (attempt, _) = coordinator
            .start(
                profile.path(),
                "fixture",
                scryer_application::RestoreJobKind::Apply,
            )
            .unwrap();
        let mut supervisor = super::ServerSupervisor::new(profile.path().to_owned(), 0);
        // The coordinator above runs in this test process, so it is the live
        // restore owner the record names.
        supervisor.restore_owner_probe = |pid| pid == std::process::id();
        supervisor.place_in_job(&child);
        supervisor.server = Some(child);
        assert!(supervisor.ensure_restore_idle().is_err());
        assert!(supervisor.stop().is_err());
        assert!(
            supervisor
                .server
                .as_mut()
                .unwrap()
                .try_wait()
                .unwrap()
                .is_none()
        );
        coordinator
            .transition(
                &attempt.id,
                scryer_application::RestoreJobPhase::Failed,
                Some(scryer_application::restore_refusal::RESTORE_DEADLINE_EXCEEDED),
            )
            .unwrap();
        supervisor.ensure_restore_idle().unwrap();
        supervisor.stop().unwrap();
        assert!(
            reader.join().unwrap(),
            "child must finish through the graceful shutdown protocol"
        );
        assert!(!supervisor.owns_server());
    }

    // -- the environment the supervised server is not allowed to inherit ------

    #[test]
    fn desktop_console_child() {
        if std::env::var("SCRYER_TEST_DESKTOP_CONSOLE_CHILD").as_deref() != Ok("1") {
            return;
        }
        use std::io::Write;
        writeln!(std::io::stderr(), "Scryer one-time setup code: ABC-234").unwrap();
    }

    #[tokio::test]
    async fn desktop_console_captures_setup_output_without_overwriting_previous_runs() {
        let dir = tempfile::tempdir().unwrap();
        for _ in 0..2 {
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "shared::tests::desktop_console_child",
                    "--nocapture",
                ])
                .env("SCRYER_TEST_DESKTOP_CONSOLE_CHILD", "1")
                .stdout(std::process::Stdio::piped());
            super::configure_server_console(&mut command, dir.path()).unwrap();
            super::configure_server_command(&mut command);
            let output = tokio::time::timeout(
                std::time::Duration::from_secs(60),
                tokio::process::Command::from(command)
                    .kill_on_drop(true)
                    .spawn()
                    .unwrap()
                    .wait_with_output(),
            )
            .await
            .unwrap()
            .unwrap();
            assert!(output.status.success(), "{output:?}");
            assert!(output.stderr.is_empty());
        }
        let output_path = dir.path().join("console.log");
        assert_eq!(
            std::fs::read_to_string(&output_path).unwrap(),
            "Scryer one-time setup code: ABC-234\n".repeat(2)
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(output_path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn desktop_environment_child() {
        if std::env::var("SCRYER_TEST_DESKTOP_ENV_CHILD").as_deref() != Ok("1") {
            return;
        }
        assert_eq!(
            std::env::var("SCRYER_RATE_LIMIT_TRUSTED_PROXY_IPS").unwrap(),
            "127.0.0.1"
        );
        assert_eq!(std::env::var("SCRYER_AUTH_ENABLED").unwrap(), "true");
        println!("desktop-environment-child: verified");
    }

    #[tokio::test]
    async fn desktop_environment_reaches_the_child_process() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(".env"),
            "SCRYER_RATE_LIMIT_TRUSTED_PROXY_IPS=127.0.0.1\nSCRYER_AUTH_ENABLED=false\n",
        )
        .unwrap();
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "shared::tests::desktop_environment_child",
                "--nocapture",
            ])
            .env("SCRYER_TEST_DESKTOP_ENV_CHILD", "1")
            .env("SCRYER_AUTH_ENABLED", "true");
        configure_environment(
            &mut command,
            dir.path(),
            vec![("SCRYER_AUTH_ENABLED".into(), "true".into())],
        )
        .unwrap();
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(60),
            tokio::process::Command::from(command)
                .kill_on_drop(true)
                .output(),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("desktop-environment-child: verified")
        );
    }

    #[test]
    fn desktop_environment_preserves_inherited_precedence_and_reloads_profile() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        std::fs::write(
            &path,
            "SCRYER_AUTH_ENABLED=true\nSCRYER_RATE_LIMIT_TRUSTED_PROXY_IPS=127.0.0.1\n",
        )
        .unwrap();
        let inherited = vec![("SCRYER_AUTH_ENABLED".into(), "false".into())];
        let mut command = std::process::Command::new("scryer");
        configure_environment(&mut command, dir.path(), inherited).unwrap();
        let values: Vec<_> = command.get_envs().collect();
        assert_eq!(
            values,
            vec![(
                OsStr::new("SCRYER_RATE_LIMIT_TRUSTED_PROXY_IPS"),
                Some(OsStr::new("127.0.0.1"))
            )]
        );
        std::fs::write(&path, "SCRYER_RATE_LIMIT_TRUSTED_PROXY_IPS=::1\n").unwrap();
        let mut restarted = std::process::Command::new("scryer");
        configure_environment(&mut restarted, dir.path(), vec![]).unwrap();
        assert_eq!(
            restarted.get_envs().next().unwrap().1,
            Some(OsStr::new("::1"))
        );
    }

    #[test]
    fn desktop_environment_rejects_managed_overrides_from_either_source() {
        let dir = tempfile::tempdir().unwrap();
        let mut command = std::process::Command::new("scryer");
        let error = configure_environment(
            &mut command,
            dir.path(),
            vec![("SCRYER_DB_URL".into(), "private-value".into())],
        )
        .unwrap_err();
        assert!(error.contains("SCRYER_DB_URL"));
        assert!(!error.contains("private-value"));
        std::fs::write(dir.path().join(".env"), "SCRYER_BIND=0.0.0.0:8080").unwrap();
        assert!(configure_environment(&mut command, dir.path(), vec![]).is_err());
    }

    // -- the desktop profile --------------------------------------------------

    /// The desktop profile is its own directory under the platform's
    /// application-data root, never the portable working directory an
    /// unpackaged `scryer` would use. The expectation is built with `join`
    /// rather than spelled out, because this module now also compiles for
    /// macOS, where the separator is not a backslash.
    #[test]
    fn desktop_profile_is_isolated_from_legacy_portable_state() {
        let application_data = Path::new(concat!(r"C:\", r"Users\example\AppData\Local"));
        assert_eq!(
            desktop_profile_dir_from(application_data),
            application_data.join("ScryerMedia").join("Scryer")
        );
        assert_ne!(
            desktop_profile_dir_from(application_data),
            application_data.to_path_buf()
        );
    }

    // -- a failed start reports what the server logged ------------------------

    const EARLIER_RUN: &str = concat!(
        r#"{"timestamp":"2026-09-11T09:00:00.000-04:00","level":"ERROR","target":"scryer","fields":{"message":"an earlier run's failure"}}"#,
        "\n",
    );

    #[test]
    fn a_failed_start_reports_the_error_it_logged() {
        let dir = tempfile::tempdir().unwrap();
        let log_file = dir.path().join("scryer.log");
        let this_run = concat!(
            r#"{"timestamp":"2026-09-11T10:00:00.000-04:00","level":"INFO","target":"scryer","fields":{"message":"opening database"}}"#,
            "\n",
            r#"{"timestamp":"2026-09-11T10:00:00.100-04:00","level":"ERROR","target":"scryer","fields":{"message":"failed to open database: database is locked"}}"#,
            "\n",
            r#"{"timestamp":"2026-09-11T10:00:00.200-04:00","level":"INFO","target":"scryer","fields":{"message":"shutting down"}}"#,
            "\n",
        );
        std::fs::write(&log_file, format!("{EARLIER_RUN}{this_run}")).unwrap();

        assert_eq!(
            last_logged_error(&log_file, EARLIER_RUN.len() as u64).as_deref(),
            Some("failed to open database: database is locked")
        );
    }

    #[test]
    fn a_failed_start_that_logged_no_error_is_not_blamed_on_an_earlier_run() {
        let dir = tempfile::tempdir().unwrap();
        let log_file = dir.path().join("scryer.log");
        let this_run = concat!(
            r#"{"timestamp":"2026-09-11T10:00:00.000-04:00","level":"INFO","target":"scryer","fields":{"message":"opening database"}}"#,
            "\n",
        );
        std::fs::write(&log_file, format!("{EARLIER_RUN}{this_run}")).unwrap();

        assert_eq!(last_logged_error(&log_file, EARLIER_RUN.len() as u64), None);
    }

    #[test]
    fn a_log_rotated_by_the_failed_start_is_read_from_its_beginning() {
        let dir = tempfile::tempdir().unwrap();
        let log_file = dir.path().join("scryer.log");
        std::fs::write(
            &log_file,
            concat!(
                r#"{"timestamp":"2026-09-11T10:00:00.000-04:00","level":"ERROR","target":"scryer","fields":{"message":"the port is in use"}}"#,
                "\n",
            ),
        )
        .unwrap();

        assert_eq!(
            last_logged_error(&log_file, 10 * 1024 * 1024).as_deref(),
            Some("the port is in use")
        );
    }

    #[test]
    fn only_error_lines_carry_a_start_failure() {
        assert_eq!(
            logged_error_message(
                r#"{"timestamp":"2026-09-11T10:00:00.000-04:00","level":"WARN","target":"scryer","fields":{"message":"slow disk"}}"#
            ),
            None
        );
        assert_eq!(
            logged_error_message(
                r#"{"timestamp":"2026-09-11T10:00:00.000-04:00","level":"INFO","target":"scryer","fields":{"message":"the server said ERROR and went on"}}"#
            ),
            None
        );
    }

    #[test]
    fn a_text_format_log_still_yields_its_error() {
        assert_eq!(
            logged_error_message(
                r"2026-09-11T10:00:00.000-04:00 ERROR scryer::init: cannot create data-dir (D:\Scryer): access is denied"
            )
            .as_deref(),
            Some(r"cannot create data-dir (D:\Scryer): access is denied")
        );
        assert_eq!(
            logged_error_message("2026-09-11T10:00:00.000-04:00  WARN scryer: slow disk"),
            None
        );
    }

    // -- uninstall ------------------------------------------------------------

    #[test]
    fn uninstall_keeps_backup_folders_that_hold_files() {
        let root = tempfile::tempdir().unwrap();
        let profile = desktop_profile_dir_from(root.path());
        let backup = profile.join("backups").join("scryer-backup-1.zip");
        std::fs::create_dir_all(backup.parent().unwrap()).unwrap();
        std::fs::write(&backup, b"backup").unwrap();
        std::fs::create_dir_all(profile.join("logs")).unwrap();
        std::fs::create_dir_all(profile.join("WebView2").join("EBWebView")).unwrap();
        std::fs::create_dir_all(profile.join("cache")).unwrap();
        for file in [
            "scryer.db",
            "scryer.db-wal",
            "scryer.db-shm",
            "encryption.key",
            "jwt-signing-secret",
        ] {
            std::fs::write(profile.join(file), b"state").unwrap();
        }
        std::fs::write(profile.join("logs").join("scryer.log"), b"log").unwrap();

        assert!(remove_desktop_profile(&profile).is_empty());

        assert!(backup.is_file());
        let left = std::fs::read_dir(&profile)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(left, vec!["backups".to_string()]);
    }

    /// The uninstall custom action deletes recursively, so the one thing it
    /// must never do is run somewhere that is not the profile. A media library
    /// lives outside the profile and is never named by it; these are the paths
    /// a bad argument or a tampered environment could otherwise produce.
    #[test]
    fn uninstall_refuses_any_path_that_is_not_the_desktop_profile() {
        let root = tempfile::tempdir().unwrap();
        let library = root.path().join("Media");
        std::fs::create_dir_all(library.join("Shows")).unwrap();

        let refused_paths: [PathBuf; 5] = [
            library.clone(),
            root.path().to_path_buf(),
            // The vendor folder itself, one level above the profile.
            root.path().join(PROFILE_VENDOR_DIR),
            // The right tail, reached by climbing out of somewhere else.
            root.path()
                .join("Media")
                .join("..")
                .join(PROFILE_VENDOR_DIR)
                .join(PROFILE_PRODUCT_DIR),
            // A relative path, which would resolve against whatever working
            // directory the installer happened to hand the custom action.
            PathBuf::from(PROFILE_VENDOR_DIR).join(PROFILE_PRODUCT_DIR),
        ];
        for refused in refused_paths {
            let problems = remove_desktop_profile(&refused);
            assert_eq!(
                problems.len(),
                1,
                "expected {} to be refused",
                refused.display()
            );
            assert!(problems[0].contains("refusing to clean up"));
        }
        assert!(library.join("Shows").is_dir());
    }

    #[test]
    fn uninstall_removes_an_emptied_profile_and_its_vendor_folder() {
        let root = tempfile::tempdir().unwrap();
        let profile = desktop_profile_dir_from(root.path());
        std::fs::create_dir_all(profile.join("backups")).unwrap();
        std::fs::write(profile.join("scryer.db"), b"state").unwrap();

        assert!(remove_desktop_profile(&profile).is_empty());

        assert!(!root.path().join("ScryerMedia").exists());
        assert!(root.path().is_dir());
    }

    #[test]
    fn uninstall_without_a_profile_has_nothing_to_do() {
        let root = tempfile::tempdir().unwrap();
        assert!(remove_desktop_profile(&desktop_profile_dir_from(root.path())).is_empty());
    }

    // -- which links stay in the app window -----------------------------------

    #[test]
    fn the_app_window_stays_on_the_local_server() {
        let origin = app_origin(8080);
        assert_eq!(origin, "http://127.0.0.1:8080");
        assert_eq!(app_url(8080), "http://127.0.0.1:8080/");

        for inside in [
            "http://127.0.0.1:8080",
            "http://127.0.0.1:8080/",
            "http://127.0.0.1:8080/library",
            "http://127.0.0.1:8080/graphql?query=1",
            "http://127.0.0.1:8080/#/settings",
            "HTTP://127.0.0.1:8080/library",
        ] {
            assert!(
                !opens_in_external_browser(&origin, inside),
                "{inside} should stay in the app window"
            );
        }
    }

    #[test]
    fn links_off_the_local_server_go_to_the_browser() {
        let origin = app_origin(8080);

        for outside in [
            "https://example.invalid/docs",
            // Same host and port, different scheme: not the app.
            "https://127.0.0.1:8080/",
            // Same host, different port: somebody else's server.
            "http://127.0.0.1:8081/",
            // A host alias is still a different origin, and the app never
            // links to itself that way.
            "http://localhost:8080/",
            // Userinfo cannot be used to smuggle the origin check.
            "http://127.0.0.1:8080@example.invalid/",
        ] {
            assert!(
                opens_in_external_browser(&origin, outside),
                "{outside} should open in the browser"
            );
        }
    }

    #[test]
    fn non_http_documents_are_left_to_the_webview() {
        let origin = app_origin(8080);

        for internal in [
            "about:blank",
            "data:text/html,<p>hi</p>",
            "blob:http://127.0.0.1:8080/9d1f",
            "javascript:void(0)",
            "",
        ] {
            assert!(
                !opens_in_external_browser(&origin, internal),
                "{internal} should be left to the webview"
            );
            assert_eq!(http_origin(internal), None);
        }
    }

    // -- readiness ------------------------------------------------------------

    #[test]
    fn the_smoke_response_declares_its_own_length() {
        let (headers, body) = SMOKE_RESPONSE
            .split_once("\r\n\r\n")
            .expect("smoke response has a header block");
        assert_eq!(body, SMOKE_BODY);
        assert!(
            headers.contains(&format!("Content-Length: {}", SMOKE_BODY.len())),
            "declared length must match the body: {headers}"
        );
    }

    fn document(status: u16, body: &str) -> HttpResponse {
        HttpResponse {
            status,
            headers: vec![("content-type".to_string(), "text/html".to_string())],
            body: body.as_bytes().to_vec(),
        }
    }

    #[test]
    fn every_page_the_server_answers_the_root_with_counts_as_ready() {
        for body in [
            "<!doctype html><html><head><title>Scryer</title></head><body></body></html>",
            // The bootstrap splash and its failure page, whose titles the
            // server's own tests pin in lower case.
            "<!doctype html><html><head>\n<title>scryer — starting</title>\n</head></html>",
            "<html><head><TITLE>scryer — error</TITLE></head></html>",
        ] {
            assert!(is_scryer_document(&document(200, body)), "{body}");
        }
    }

    #[test]
    fn a_stranger_on_the_port_is_not_a_ready_server() {
        // Another local program answering 200 must not be mistaken for Scryer,
        // or the wrapper would skip starting its own server and show its page.
        assert!(!is_scryer_document(&document(
            200,
            "<html><head><title>Grafana</title></head><body></body></html>"
        )));
        assert!(!is_scryer_document(&document(200, "{\"status\":\"ok\"}")));
        assert!(!is_scryer_document(&document(200, "")));
    }

    #[test]
    fn a_scryer_page_that_is_not_a_200_is_not_ready_yet() {
        assert!(!is_scryer_document(&document(
            503,
            "<html><head><title>Scryer</title></head></html>"
        )));
    }

    #[test]
    fn the_smoke_document_is_a_scryer_page() {
        let response = parse_http_response(SMOKE_RESPONSE.as_bytes()).unwrap();
        assert!(is_scryer_document(&response));
    }

    // -- HTTP framing ---------------------------------------------------------

    #[test]
    fn a_declared_length_response_splits_into_status_headers_and_body() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                    Content-Length: 2\r\n\r\n{}";
        let response = parse_http_response(raw).expect("a well-formed response parses");

        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"{}");
        assert_eq!(
            response
                .headers
                .iter()
                .find(|(field, _)| field == "Content-Type")
                .map(|(_, value)| value.as_str()),
            Some("application/json")
        );
    }

    #[test]
    fn a_chunked_response_is_reassembled() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n\
                    5\r\nhello\r\n6;ext=1\r\n world\r\n0\r\n\r\n";
        let response = parse_http_response(raw).expect("a chunked response parses");

        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"hello world");
    }

    #[test]
    fn a_truncated_chunked_body_is_rejected_rather_than_guessed() {
        assert_eq!(decode_chunked(b"5\r\nhel"), None);
        assert_eq!(decode_chunked(b"zz\r\n"), None);
        // No terminating zero chunk: the response never finished arriving.
        assert_eq!(decode_chunked(b"5\r\nhello\r\n"), None);
    }

    #[test]
    fn a_response_without_a_header_block_is_not_a_response() {
        assert_eq!(parse_http_response(b"HTTP/1.1 200 OK\r\n"), None);
        assert_eq!(parse_http_response(b""), None);
    }

    // -- the popover's status snapshot ---------------------------------------

    #[test]
    fn a_ready_server_reads_as_running() {
        let content = popover_content_from_health(r#"{"status":"ok","ready":true}"#)
            .expect("a health body maps");

        assert_eq!(content.status.as_deref(), Some("Running"));
        assert!(content.rows.is_empty());
        assert_eq!(
            content.message.as_deref(),
            Some("Open Scryer for the full view")
        );
    }

    #[test]
    fn a_migrating_server_shows_its_progress() {
        let content = popover_content_from_health(
            r#"{"status":"migrating","ready":false,"migrations":{"completed":3,"total":12}}"#,
        )
        .expect("a health body maps");

        assert_eq!(content.status.as_deref(), Some("Starting"));
        assert_eq!(content.message, None);
        assert_eq!(content.rows.len(), 1);
        assert_eq!(content.rows[0].name, "Database update 3/12");
        assert_eq!(content.rows[0].state, "Updating");
        assert_eq!(content.rows[0].progress_percent, 25.0);
    }

    #[test]
    fn a_migrating_server_that_has_not_counted_yet_says_only_that_it_is_starting() {
        let content = popover_content_from_health(
            r#"{"status":"migrating","ready":false,"migrations":null}"#,
        )
        .expect("a health body maps");

        assert_eq!(content.status.as_deref(), Some("Starting"));
        assert!(content.rows.is_empty());
        assert_eq!(content.message.as_deref(), Some("Updating the database"));
    }

    #[test]
    fn a_failed_bootstrap_reports_the_servers_own_message() {
        let content = popover_content_from_health(
            r#"{"status":"error","ready":false,"message":"database is locked"}"#,
        )
        .expect("a health body maps");

        assert_eq!(content.status.as_deref(), Some("Failed to start"));
        assert_eq!(content.message.as_deref(), Some("database is locked"));
    }

    #[test]
    fn a_phase_this_build_has_never_heard_of_is_still_described() {
        let content = popover_content_from_health(r#"{"status":"RE_INDEXING","ready":false}"#)
            .expect("a health body maps");

        assert_eq!(content.status.as_deref(), Some("Re indexing"));
    }

    #[test]
    fn a_body_that_is_not_a_health_snapshot_maps_to_nothing() {
        for body in ["{}", "not json", r#"{"data":{}}"#, ""] {
            assert_eq!(popover_content_from_health(body), None, "{body}");
        }
    }

    #[test]
    fn the_offline_and_unavailable_states_carry_no_status_line() {
        let offline = PopoverContent::offline();
        assert_eq!(offline.status, None);
        assert_eq!(offline.message.as_deref(), Some("Scryer isn't running"));
        assert!(offline.rows.is_empty());

        let unavailable = PopoverContent::unavailable();
        assert_eq!(unavailable.status, None);
        assert_eq!(
            unavailable.message.as_deref(),
            Some("Status unavailable — open Scryer")
        );
        // The offline message is reserved for a port nobody answered; anything
        // that replied is running, whatever it replied with.
        assert_ne!(unavailable, offline);
    }

    #[test]
    fn a_row_detail_reads_state_then_percent() {
        let row = PopoverRow {
            name: "Database update 3/12".to_string(),
            state: "Updating".to_string(),
            progress_percent: 42.5,
        };
        assert_eq!(row_detail(&row), "Updating · 43%");
    }

    // -- the server's relaunch request ---------------------------------------

    /// The request is a specific exit code and nothing else. An ordinary
    /// failure, a panic, a clean exit and a signal must all stay ordinary
    /// exits, or a crashing server would relaunch the app in a loop.
    #[cfg(unix)]
    #[test]
    fn only_the_relaunch_exit_code_asks_the_wrapper_to_relaunch() {
        use std::os::unix::process::ExitStatusExt;

        use crate::bundle_relaunch::{BUNDLE_RELAUNCH_EXIT_CODE, is_bundle_relaunch_exit};

        let exited = |code: i32| std::process::ExitStatus::from_raw(code << 8);
        assert!(is_bundle_relaunch_exit(exited(BUNDLE_RELAUNCH_EXIT_CODE)));
        for code in [0, 1, 2, 70, 86, 88, 101, 255] {
            assert!(
                !is_bundle_relaunch_exit(exited(code)),
                "exit code {code} must not be read as a relaunch request"
            );
        }
        // Killed by signal 87: no exit code at all.
        assert!(!is_bundle_relaunch_exit(
            std::process::ExitStatus::from_raw(BUNDLE_RELAUNCH_EXIT_CODE)
        ));
    }
}

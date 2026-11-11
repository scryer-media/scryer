//! Download the diagnostics bundle: logs, metrics and a settings and
//! environment summary in one sanitized `.tar.zst`, built in memory.
//!
//! The application layer owns the bundle and its sanitizer. This route adds
//! what only the binary knows (the log buffer and log file, the metrics
//! recorder, process, operating-system and installation facts) and the values
//! that identify the host, so the sanitizer can replace them.

use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Json;
use axum::body::Body;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use scryer_application::application_upgrade::InstallationAssessment;
use scryer_application::diagnostics::{DiagnosticsHostInputs, DiagnosticsMetrics};
use scryer_application::{AppError, JwtSessionScope};
use serde_json::{Map, Value, json};

use crate::error_reasons::{
    HTTP_DIAGNOSTICS_AUTHENTICATION_REQUIRED, HTTP_DIAGNOSTICS_BUNDLE_FAILED,
    HTTP_DIAGNOSTICS_PERMISSION_DENIED,
};
use crate::log_buffer::LogRingBuffer;
use crate::middleware::{AuthState, resolve_actor};

pub(crate) const DIAGNOSTICS_BUNDLE_PATH: &str = "/api/diagnostics/bundle";

/// Lines taken from the in-memory log buffer: all of it.
const BUFFERED_LOG_LINES: usize = usize::MAX;

/// `SCRYER_*` variables whose value is written as-is. Every other variable is
/// reported by name with the value `<set>`.
const ENV_VALUE_ALLOWLIST: &[&str] = &[
    "SCRYER_LOG_FORMAT",
    "SCRYER_METRICS",
    "SCRYER_AUTH_ENABLED",
    "SCRYER_PACKAGE",
];

/// Facts `main` captures before the application bootstraps.
#[derive(Clone, Debug)]
pub(crate) struct DiagnosticsProcessFacts {
    pub started_at: DateTime<Utc>,
    /// The active log file, when file logging opened.
    pub log_file: Option<PathBuf>,
}

/// Everything the route needs from the binary, captured once at bootstrap.
pub(crate) struct DiagnosticsHost {
    pub process: DiagnosticsProcessFacts,
    pub log_ring_buffer: LogRingBuffer,
    pub metrics: Option<metrics_exporter_prometheus::PrometheusHandle>,
    pub installation: InstallationAssessment,
    pub data_dir: PathBuf,
    pub listening: SocketAddr,
    pub base_path_configured: bool,
    /// The SQLite database file, when the datastore is SQLite.
    pub sqlite_path: Option<PathBuf>,
}

#[derive(Clone)]
pub(crate) struct DiagnosticsRouteState {
    pub auth: AuthState,
    pub host: Arc<DiagnosticsHost>,
}

pub(crate) async fn download_diagnostics_bundle_handler(
    State(state): State<DiagnosticsRouteState>,
    headers: HeaderMap,
    ConnectInfo(remote_addr): ConnectInfo<SocketAddr>,
) -> Response {
    let Ok(Some(actor)) = resolve_actor(&state.auth, &headers, Some(remote_addr)).await else {
        return refusal(
            StatusCode::UNAUTHORIZED,
            HTTP_DIAGNOSTICS_AUTHENTICATION_REQUIRED,
            "authentication is required to download diagnostics",
        );
    };
    // A step-up or enrollment session must not be able to pull diagnostics.
    if actor.token_claims.session_scope != JwtSessionScope::Full {
        return refusal(
            StatusCode::UNAUTHORIZED,
            HTTP_DIAGNOSTICS_AUTHENTICATION_REQUIRED,
            "authentication is required to download diagnostics",
        );
    }

    let host = state.host.clone();
    let auth = state.auth.auth_runtime.snapshot();
    let inputs = match tokio::task::spawn_blocking(move || host_inputs(&host, &auth)).await {
        Ok(inputs) => inputs,
        Err(error) => {
            tracing::warn!(%error, "diagnostics host facts task failed");
            return bundle_failed();
        }
    };

    match state
        .auth
        .app
        .build_diagnostics_bundle(&actor.user, inputs)
        .await
    {
        Ok(bundle) => {
            let mut response = Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "application/zstd");
            if let Ok(value) = HeaderValue::from_str(&content_disposition(&bundle.file_name)) {
                response = response.header(header::CONTENT_DISPOSITION, value);
            }
            response
                .header(header::CONTENT_LENGTH, bundle.bytes.len())
                .header(header::CACHE_CONTROL, "no-store")
                .body(Body::from(bundle.bytes))
                .unwrap_or_else(|_| bundle_failed())
        }
        // A permission refusal is not an authentication failure; a 401 would
        // read as an expired session to the web client.
        Err(AppError::Unauthorized(_) | AppError::UnauthorizedReason { .. }) => refusal(
            StatusCode::FORBIDDEN,
            HTTP_DIAGNOSTICS_PERMISSION_DENIED,
            "downloading diagnostics requires the system settings permission",
        ),
        Err(error) => {
            tracing::warn!(%error, "diagnostics bundle failed");
            bundle_failed()
        }
    }
}

fn bundle_failed() -> Response {
    refusal(
        StatusCode::INTERNAL_SERVER_ERROR,
        HTTP_DIAGNOSTICS_BUNDLE_FAILED,
        "the diagnostics bundle could not be built",
    )
}

fn refusal(status: StatusCode, reason: &'static str, message: &'static str) -> Response {
    let mut response =
        (status, Json(json!({ "error": message, "reason": reason }))).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// `attachment` with both filename forms (RFC 6266).
fn content_disposition(file_name: &str) -> String {
    let ascii = file_name
        .chars()
        .map(|character| {
            if character.is_ascii_graphic() && character != '\\' && character != '"' {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    let encoded = utf8_percent_encode(file_name, NON_ALPHANUMERIC);
    format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{encoded}")
}

fn host_inputs(
    host: &DiagnosticsHost,
    auth: &scryer_interface::context::AuthRuntimeStateSnapshot,
) -> DiagnosticsHostInputs {
    let generated_at = Utc::now();
    let variables = std::env::vars_os()
        .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
        .collect::<Vec<_>>();
    let mut facts = HostFacts::default();
    facts.collect(host, auth, &variables, generated_at);

    DiagnosticsHostInputs {
        buffered_log_lines: host.log_ring_buffer.snapshot(BUFFERED_LOG_LINES),
        log_file: host.process.log_file.clone(),
        metrics: match &host.metrics {
            Some(handle) => DiagnosticsMetrics::Rendered(handle.render()),
            None => DiagnosticsMetrics::Disabled,
        },
        environment: facts.environment,
        environment_omissions: facts.omissions,
        sensitive_values: facts.sensitive_values,
        sensitive_paths: facts.sensitive_paths,
        version: env!("CARGO_PKG_VERSION").to_string(),
        generated_at,
    }
}

#[derive(Default)]
struct HostFacts {
    environment: Map<String, Value>,
    omissions: Vec<(String, String)>,
    sensitive_values: Vec<String>,
    sensitive_paths: Vec<(String, String)>,
}

impl HostFacts {
    fn omit(&mut self, item: &str, reason: &str) {
        self.omissions.push((item.to_string(), reason.to_string()));
    }

    fn path(&mut self, pseudonym: &str, path: &Path) {
        if let Some(text) = path.to_str() {
            self.sensitive_paths
                .push((pseudonym.to_string(), text.to_string()));
        }
    }

    fn collect(
        &mut self,
        host: &DiagnosticsHost,
        auth: &scryer_interface::context::AuthRuntimeStateSnapshot,
        variables: &[(String, String)],
        generated_at: DateTime<Utc>,
    ) {
        let variable = |name: &str| env_value(variables, name);

        self.environment.insert(
            "build".to_string(),
            json!({
                "version": env!("CARGO_PKG_VERSION"),
                "profile": if cfg!(debug_assertions) { "debug" } else { "release" },
                "build_lane": env!("SCRYER_COMPILED_BUILD_LANE"),
                "target_os": std::env::consts::OS,
                "target_arch": std::env::consts::ARCH,
                "target_family": std::env::consts::FAMILY,
            }),
        );
        self.omit("build.commit", "not_embedded_in_build");
        self.omit("wasmtime_version", "not_exposed_by_build");

        let assessment = &host.installation;
        self.environment.insert(
            "installation".to_string(),
            json!({
                "kind": snake_case(&format!("{:?}", assessment.kind)),
                "upgrade_owner": snake_case(&format!("{:?}", assessment.owner)),
                "self_upgrade_eligible": assessment.eligible,
                "self_upgrade_reason": snake_case(&format!("{:?}", assessment.reason)),
                "tray_supervised": assessment.tray_supervised,
                "package": variable("SCRYER_PACKAGE").map(safe_token),
                "self_upgrade_disabled_by_environment": variable("SCRYER_DISABLE_SELF_UPGRADE").is_some(),
            }),
        );

        self.container(variables);
        let systemd_unit = systemd_unit(variables, std::process::id());
        if systemd_unit.is_none() {
            self.omit("launcher.systemd_unit", "systemd_exec_pid_not_set");
        }
        self.environment.insert(
            "launcher".to_string(),
            json!({
                "systemd_unit": systemd_unit,
                "launchd_job": cfg!(target_os = "macos") && launchd_job(variables, parent_process_id()),
                "windows_tray_supervised": assessment.tray_supervised,
            }),
        );

        self.operating_system();

        let uptime = (generated_at - host.process.started_at)
            .num_seconds()
            .max(0);
        let mut process = Map::new();
        process.insert(
            "started_at".to_string(),
            json!(
                host.process
                    .started_at
                    .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
            ),
        );
        process.insert("uptime_seconds".to_string(), json!(uptime));
        match peak_resident_bytes() {
            Some(bytes) => {
                process.insert("peak_resident_bytes".to_string(), json!(bytes));
            }
            None => self.omit(
                "process.peak_resident_bytes",
                "not_portable_on_this_platform",
            ),
        }
        self.environment
            .insert("process".to_string(), Value::Object(process));

        match &host.sqlite_path {
            Some(path) => {
                let size = |suffix: &str| {
                    let mut name = path.as_os_str().to_owned();
                    name.push(suffix);
                    std::fs::metadata(PathBuf::from(name))
                        .ok()
                        .map(|metadata| metadata.len())
                };
                self.environment.insert(
                    "datastore_files".to_string(),
                    json!({
                        "database_bytes": size(""),
                        "wal_bytes": size("-wal"),
                        "shared_memory_bytes": size("-shm"),
                    }),
                );
                if let Some(parent) = path.parent() {
                    self.path("database-dir", parent);
                }
            }
            None => self.omit("datastore_files", "not_sqlite"),
        }

        let listening = host.listening;
        let tls_enabled = ["SCRYER_TLS_CERT", "SCRYER_TLS_KEY"]
            .iter()
            .all(|name| variable(name).is_some());
        self.environment.insert(
            "network".to_string(),
            json!({
                "bind_port": listening.port(),
                "bind_address_kind": address_kind(listening.ip()),
                "tls_enabled": tls_enabled,
                "base_path_configured": host.base_path_configured,
                "public_url_from_environment": variable("SCRYER_PUBLIC_URL").is_some(),
                "cors_origins_configured": variable("SCRYER_CORS_ALLOWED_ORIGINS").is_some(),
            }),
        );
        if !listening.ip().is_loopback() && !listening.ip().is_unspecified() {
            self.sensitive_values.push(listening.ip().to_string());
        }

        self.environment.insert(
            "auth".to_string(),
            json!({
                "mode": if auth.effective_form_login_enabled { "authenticated" } else { "authless" },
                "form_login_enabled": auth.form_login_enabled,
                "authless_local_access": auth.skip_login_for_local_ips,
                "passkey_enabled": auth.passkey_enabled,
                "webauthn_configured": auth.webauthn_configured,
                "environment_override_active": auth.env_override_active,
            }),
        );

        self.environment.insert(
            "metrics".to_string(),
            json!({
                "enabled": host.metrics.is_some(),
                "renderable": host.metrics.is_some(),
            }),
        );
        self.environment.insert(
            "logging".to_string(),
            json!({
                "file_logging_enabled": host.process.log_file.is_some(),
                "log_filter": variable("RUST_LOG").map(safe_log_filter),
            }),
        );

        let offset = chrono::Local::now().offset().local_minus_utc();
        let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .find_map(|name| variable(name))
            .map(safe_token);
        self.environment.insert(
            "time_and_locale".to_string(),
            json!({
                "utc_offset_seconds": offset,
                "locale": locale,
            }),
        );

        self.scryer_variables(variables);
        self.identifying_values(host, variables);
    }

    fn container(&mut self, variables: &[(String, String)]) {
        let variable = |name: &str| env_value(variables, name);
        let mut hints = Vec::new();
        if cfg!(target_os = "linux") {
            for source in ["/proc/1/cgroup", "/proc/self/cgroup"] {
                if let Ok(text) = std::fs::read_to_string(source) {
                    for token in ["docker", "kubepods", "containerd", "libpod", "lxc"] {
                        if text.contains(token) && !hints.contains(&token) {
                            hints.push(token);
                        }
                    }
                }
            }
        }
        let container_variable = variable("container").map(|value| {
            match value {
                "docker" | "podman" | "oci" | "lxc" | "systemd-nspawn" => value,
                _ => "<set>",
            }
            .to_string()
        });
        self.environment.insert(
            "container".to_string(),
            json!({
                "docker_env_file": Path::new("/.dockerenv").exists(),
                "podman_env_file": Path::new("/run/.containerenv").exists(),
                "container_variable": container_variable,
                "kubernetes": variable("KUBERNETES_SERVICE_HOST").is_some(),
                "cgroup_hints": hints,
            }),
        );
    }

    fn operating_system(&mut self) {
        let mut os = Map::new();
        os.insert("family".to_string(), json!(std::env::consts::OS));
        os.insert(
            "path_style".to_string(),
            json!(if cfg!(windows) { "windows" } else { "unix" }),
        );
        os.insert(
            "cpu_count".to_string(),
            json!(std::thread::available_parallelism().map_or(0, |count| count.get())),
        );
        match os_release() {
            Some((name, version)) => {
                os.insert("name".to_string(), json!(name));
                os.insert("version".to_string(), json!(version));
            }
            None => self.omit("os.version", "not_portable_on_this_platform"),
        }
        match kernel_release() {
            Some(release) => {
                os.insert("kernel".to_string(), json!(release));
            }
            None => self.omit("os.kernel", "not_portable_on_this_platform"),
        }
        match total_memory_bytes() {
            Some(bytes) => {
                os.insert("total_memory_bytes".to_string(), json!(bytes));
            }
            None => self.omit("os.total_memory_bytes", "not_portable_on_this_platform"),
        }
        self.environment.insert("os".to_string(), Value::Object(os));
    }

    /// Every `SCRYER_*` variable by name; values only for the allowlist.
    fn scryer_variables(&mut self, variables: &[(String, String)]) {
        let mut names = variables
            .iter()
            .filter(|(name, value)| name.starts_with("SCRYER_") && !value.trim().is_empty())
            .collect::<Vec<_>>();
        names.sort_by(|left, right| left.0.cmp(&right.0));
        let mut reported = Map::new();
        for (name, value) in names {
            let shown = if ENV_VALUE_ALLOWLIST.contains(&name.as_str()) {
                safe_token(value)
            } else {
                "<set>".to_string()
            };
            reported.insert(name.clone(), json!(shown));
            if !ENV_VALUE_ALLOWLIST.contains(&name.as_str()) {
                for part in value.split([',', ';']) {
                    let part = part.trim();
                    if looks_absolute_path(part) {
                        self.sensitive_paths
                            .push(("env-path".to_string(), part.to_string()));
                    } else if identifying_value(part) {
                        self.sensitive_values.push(part.to_string());
                    }
                }
                if identifying_value(value.trim()) {
                    self.sensitive_values.push(value.trim().to_string());
                }
            }
        }
        self.environment
            .insert("scryer_variables".to_string(), Value::Object(reported));
    }

    /// Values that identify this host and that nothing else harvests.
    fn identifying_values(&mut self, host: &DiagnosticsHost, variables: &[(String, String)]) {
        let variable = |name: &str| env_value(variables, name);
        self.path("data-dir", &host.data_dir);
        if let Some(parent) = host.process.log_file.as_deref().and_then(Path::parent) {
            self.path("log-dir", parent);
        }
        if let Some(log_file) = host.process.log_file.as_deref() {
            self.path("log-file", log_file);
        }
        if let Some(directory) = std::env::current_exe()
            .ok()
            .as_deref()
            .and_then(Path::parent)
        {
            self.path("executable-dir", directory);
        }
        if let Ok(directory) = std::env::current_dir() {
            self.path("working-dir", &directory);
        }
        for name in [
            "HOME",
            "USERPROFILE",
            "TMPDIR",
            "TEMP",
            "TMP",
            "APPDATA",
            "LOCALAPPDATA",
        ] {
            if let Some(value) = variable(name) {
                self.sensitive_paths
                    .push((name.to_ascii_lowercase(), value.to_string()));
            }
        }
        for name in ["SCRYER_TLS_CERT", "SCRYER_TLS_KEY"] {
            if let Some(value) = variable(name) {
                self.sensitive_paths
                    .push(("tls-file".to_string(), value.to_string()));
            }
        }
        for name in [
            "USER",
            "LOGNAME",
            "USERNAME",
            "HOSTNAME",
            "COMPUTERNAME",
            "USERDOMAIN",
        ] {
            if let Some(value) = variable(name)
                && identifying_account_name(value)
            {
                self.sensitive_values.push(value.to_string());
            }
        }
        if let Some(name) = host_name()
            && identifying_account_name(&name)
            && !name.eq_ignore_ascii_case("localhost")
        {
            // The short name too: logs and prompts often print only that.
            if let Some((short, _)) = name.split_once('.')
                && identifying_account_name(short)
            {
                self.sensitive_values.push(short.to_string());
            }
            self.sensitive_values.push(name);
        }
    }
}

/// The non-blank value of one environment variable.
fn env_value<'v>(variables: &'v [(String, String)], name: &str) -> Option<&'v str> {
    variables
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
        .filter(|value| !value.trim().is_empty())
}

/// A value worth replacing wherever it appears: long enough to be specific,
/// and not a number, a boolean or a common setting word that would erase
/// unrelated text.
fn identifying_value(value: &str) -> bool {
    const COMMON: &[&str] = &[
        "true",
        "false",
        "yes",
        "no",
        "on",
        "off",
        "json",
        "text",
        "enabled",
        "disabled",
        "info",
        "debug",
        "warn",
        "error",
        "trace",
        "none",
        "auto",
        "default",
        "localhost",
    ];
    let value = value.trim();
    value.chars().count() >= 3
        && value.parse::<f64>().is_err()
        && !COMMON.contains(&value.to_ascii_lowercase().as_str())
}

/// Whether this process is the main process of a systemd unit. Every process
/// a unit starts inherits `INVOCATION_ID`, including a shell in a terminal
/// that a user service opened, so only `SYSTEMD_EXEC_PID` (systemd 248 and
/// later), compared with this process id, tells the unit's own process apart.
/// `None` when a unit is present but systemd is too old to say.
fn systemd_unit(variables: &[(String, String)], pid: u32) -> Option<bool> {
    if let Some(exec_pid) = env_value(variables, "SYSTEMD_EXEC_PID") {
        return Some(exec_pid.trim().parse::<u32>() == Ok(pid));
    }
    if env_value(variables, "INVOCATION_ID").is_some() {
        return None;
    }
    Some(false)
}

/// Whether launchd started this process as its own job. Children of a GUI
/// app or of another job inherit `XPC_SERVICE_NAME`, so the variable alone
/// does not say; launchd (process 1) must also be the direct parent.
fn launchd_job(variables: &[(String, String)], parent_pid: u32) -> bool {
    parent_pid == 1 && env_value(variables, "XPC_SERVICE_NAME").is_some_and(|name| name != "0")
}

#[cfg(unix)]
fn parent_process_id() -> u32 {
    // SAFETY: `getppid` has no preconditions and cannot fail.
    u32::try_from(unsafe { libc::getppid() }).unwrap_or(0)
}

#[cfg(not(unix))]
fn parent_process_id() -> u32 {
    0
}

/// Account, domain and host names that operating systems, images and
/// installers hand out by default. They identify nobody, and replacing them
/// would erase the same word everywhere in the bundle.
const GENERIC_ACCOUNT_NAMES: &[&str] = &[
    "root",
    "admin",
    "administrator",
    "user",
    "nobody",
    "system",
    "daemon",
    "docker",
    "scryer",
    "abc",
    "app",
    "default",
    "guest",
    "ubuntu",
    "debian",
    "node",
    "runner",
    "container",
    "www-data",
    "workgroup",
];

/// An OS account, domain or host name worth replacing: identifying, and not
/// one of the [`GENERIC_ACCOUNT_NAMES`].
fn identifying_account_name(value: &str) -> bool {
    identifying_value(value)
        && !GENERIC_ACCOUNT_NAMES
            .iter()
            .any(|generic| generic.eq_ignore_ascii_case(value.trim()))
}

fn looks_absolute_path(value: &str) -> bool {
    value.starts_with('/')
        || value.starts_with("\\\\")
        || value
            .as_bytes()
            .get(1..3)
            .is_some_and(|rest| rest[0] == b':' && (rest[1] == b'\\' || rest[1] == b'/'))
}

/// A short code-like value, or `<set>` when it has any other shape.
fn safe_token(value: &str) -> String {
    let value = value.trim();
    if !value.is_empty()
        && value.len() <= 32
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '@'))
    {
        value.to_string()
    } else {
        "<set>".to_string()
    }
}

/// A `RUST_LOG`-style filter: module paths and levels only.
fn safe_log_filter(value: &str) -> String {
    let value = value.trim();
    if value.len() <= 256
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '=' | ',' | ':'))
    {
        value.to_string()
    } else {
        "<set>".to_string()
    }
}

fn snake_case(debug: &str) -> String {
    let mut out = String::with_capacity(debug.len() + 4);
    for (index, character) in debug.chars().enumerate() {
        if !character.is_ascii_alphanumeric() {
            break;
        }
        if character.is_ascii_uppercase() && index > 0 {
            out.push('_');
        }
        out.push(character.to_ascii_lowercase());
    }
    out
}

fn address_kind(address: IpAddr) -> &'static str {
    if address.is_unspecified() {
        "unspecified"
    } else if address.is_loopback() {
        "loopback"
    } else {
        match address {
            IpAddr::V4(v4) if v4.is_private() || v4.is_link_local() => "private",
            IpAddr::V6(v6) if v6.is_unique_local() || v6.is_unicast_link_local() => "private",
            _ => "other",
        }
    }
}

#[cfg(target_os = "linux")]
fn os_release() -> Option<(String, String)> {
    let text = std::fs::read_to_string("/etc/os-release")
        .or_else(|_| std::fs::read_to_string("/usr/lib/os-release"))
        .ok()?;
    let field = |key: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
            .map(|value| safe_token(value.trim_matches('"')))
    };
    Some((field("ID")?, field("VERSION_ID").unwrap_or_default()))
}

#[cfg(target_os = "macos")]
fn os_release() -> Option<(String, String)> {
    Some((
        "macos".to_string(),
        safe_token(&sysctl_string("kern.osproductversion")?),
    ))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn os_release() -> Option<(String, String)> {
    None
}

#[cfg(unix)]
fn kernel_release() -> Option<String> {
    // SAFETY: `uname` fills the zeroed struct; each field is NUL-terminated.
    let mut name: libc::utsname = unsafe { std::mem::zeroed() };
    if unsafe { libc::uname(&mut name) } != 0 {
        return None;
    }
    let release = unsafe { std::ffi::CStr::from_ptr(name.release.as_ptr()) };
    Some(safe_token(&release.to_string_lossy()))
}

#[cfg(not(unix))]
fn kernel_release() -> Option<String> {
    None
}

#[cfg(unix)]
fn host_name() -> Option<String> {
    let mut buffer = [0 as libc::c_char; 256];
    // SAFETY: the buffer is writable for its full length; the result is
    // NUL-terminated within it on success.
    if unsafe { libc::gethostname(buffer.as_mut_ptr(), buffer.len()) } != 0 {
        return None;
    }
    let name = unsafe { std::ffi::CStr::from_ptr(buffer.as_ptr()) };
    let name = name.to_string_lossy().trim().to_string();
    (!name.is_empty()).then_some(name)
}

#[cfg(not(unix))]
fn host_name() -> Option<String> {
    std::env::var("COMPUTERNAME").ok()
}

#[cfg(target_os = "linux")]
fn total_memory_bytes() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let kib = text
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))?
        .trim()
        .trim_end_matches("kB")
        .trim()
        .parse::<u64>()
        .ok()?;
    Some(kib * 1024)
}

#[cfg(target_os = "macos")]
fn total_memory_bytes() -> Option<u64> {
    let mut value: u64 = 0;
    let mut size = std::mem::size_of::<u64>();
    // SAFETY: `hw.memsize` is a u64 and `value` is a writable u64.
    let result = unsafe {
        libc::sysctlbyname(
            c"hw.memsize".as_ptr(),
            (&mut value as *mut u64).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    (result == 0).then_some(value)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn total_memory_bytes() -> Option<u64> {
    None
}

#[cfg(target_os = "macos")]
fn sysctl_string(name: &str) -> Option<String> {
    let name = std::ffi::CString::new(name).ok()?;
    let mut buffer = [0u8; 64];
    let mut size = buffer.len();
    // SAFETY: the buffer is writable for `size` bytes.
    let result = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            buffer.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if result != 0 {
        return None;
    }
    let text = std::ffi::CStr::from_bytes_until_nul(&buffer[..size.min(buffer.len())]).ok()?;
    Some(text.to_string_lossy().into_owned())
}

#[cfg(unix)]
fn peak_resident_bytes() -> Option<u64> {
    // SAFETY: `getrusage` fills the zeroed struct.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) } != 0 {
        return None;
    }
    let peak = u64::try_from(usage.ru_maxrss).ok()?;
    // Linux reports kibibytes, macOS bytes.
    Some(if cfg!(target_os = "macos") {
        peak
    } else {
        peak * 1024
    })
}

#[cfg(not(unix))]
fn peak_resident_bytes() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::middleware::integration_test_common as common;
    use crate::middleware::{AuthlessWebClientProofState, WebSocketOriginPolicy};
    use crate::rate_limit::ScryerRateLimiter;

    use axum::Router;
    use axum::routing::get;
    use scryer_domain::AppPermissionMask;
    use std::net::Ipv4Addr;
    use tower::ServiceExt as _;

    #[test]
    fn identifying_values_skip_numbers_booleans_and_setting_words() {
        for value in ["1", "true", "json", "8080", "0.5", "ON", "ab"] {
            assert!(!identifying_value(value), "{value}");
        }
        for value in ["nas-box", "operator", "s3cr3t-value"] {
            assert!(identifying_value(value), "{value}");
        }
    }

    fn test_host(data_dir: &Path) -> DiagnosticsHost {
        DiagnosticsHost {
            process: DiagnosticsProcessFacts {
                started_at: Utc::now(),
                log_file: None,
            },
            log_ring_buffer: LogRingBuffer::new(16),
            metrics: None,
            installation: InstallationAssessment::default(),
            data_dir: data_dir.to_path_buf(),
            listening: SocketAddr::from((Ipv4Addr::LOCALHOST, 8080)),
            base_path_configured: false,
            sqlite_path: None,
        }
    }

    fn account_variables(name: &str) -> Vec<(String, String)> {
        ["USER", "LOGNAME", "USERNAME"]
            .iter()
            .map(|variable| (variable.to_string(), name.to_string()))
            .collect()
    }

    fn sanitizer_for(facts: &HostFacts) -> scryer_application::diagnostics::Sanitizer {
        let mut known = scryer_application::diagnostics::KnownValues::new();
        for value in &facts.sensitive_values {
            known.secret(value);
        }
        for (pseudonym, path) in &facts.sensitive_paths {
            known.path(pseudonym, path);
        }
        known.into_sanitizer()
    }

    #[test]
    fn generic_account_names_are_not_registered_and_leave_the_bundle_intact() {
        let data_dir = tempfile::tempdir().expect("data dir");
        let host = test_host(data_dir.path());
        for generic in ["root", "Docker", "abc"] {
            let mut facts = HostFacts::default();
            facts.identifying_values(&host, &account_variables(generic));
            assert!(
                !facts
                    .sensitive_values
                    .iter()
                    .any(|value| value.eq_ignore_ascii_case(generic)),
                "{generic} was registered"
            );
            let sanitizer = sanitizer_for(&facts);
            let original = json!({
                "filesystems": {
                    "roots": [{ "root": "root-1", "role": "library_root" }],
                    "same_filesystem_pairs": [["root-1", "download-root-1"]],
                },
                "container": { "docker_env_file": false, "cgroup_hints": ["docker"] },
            });
            let mut environment = original.clone();
            sanitizer.json_values(&mut environment);
            assert_eq!(environment, original, "{generic}");
            let help = "# HELP scryer_rule_fields_total Rules that read the root field\n";
            assert_eq!(sanitizer.metrics(help), help, "{generic}");
        }
    }

    #[test]
    fn a_distinctive_account_name_is_replaced_everywhere() {
        let data_dir = tempfile::tempdir().expect("data dir");
        let host = test_host(data_dir.path());
        let mut facts = HostFacts::default();
        facts.identifying_values(&host, &account_variables("zephyrine_ops"));
        assert!(
            facts
                .sensitive_values
                .iter()
                .any(|value| value == "zephyrine_ops")
        );
        let sanitizer = sanitizer_for(&facts);
        let mut environment = json!({ "process": { "owner_hint": "zephyrine_ops" } });
        sanitizer.json_values(&mut environment);
        assert_eq!(environment["process"]["owner_hint"], "<redacted>");
        let help = "# HELP scryer_synthetic_total Started by zephyrine_ops\n";
        assert!(!sanitizer.metrics(help).contains("zephyrine_ops"));
        for line in [
            "2026-11-10T12:00:00Z  INFO scryer: started by Zephyrine_Ops in a shell".to_string(),
            json!({
                "timestamp": "2026-11-10T12:00:01Z",
                "level": "INFO",
                "fields": { "message": "running as zephyrine_ops", "account": "zephyrine_ops" },
            })
            .to_string(),
        ] {
            let sanitized = sanitizer.log_line(&line);
            assert!(
                !sanitized.to_lowercase().contains("zephyrine_ops"),
                "{sanitized}"
            );
        }
    }

    #[test]
    fn systemd_unit_needs_this_process_to_be_the_units_own() {
        let vars = |pairs: &[(&str, &str)]| {
            pairs
                .iter()
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect::<Vec<_>>()
        };
        assert_eq!(systemd_unit(&vars(&[]), 4242), Some(false));
        assert_eq!(
            systemd_unit(
                &vars(&[("INVOCATION_ID", "0123abcd"), ("SYSTEMD_EXEC_PID", "4242")]),
                4242
            ),
            Some(true)
        );
        // Inherited from a unit further up the process tree.
        assert_eq!(
            systemd_unit(
                &vars(&[("INVOCATION_ID", "0123abcd"), ("SYSTEMD_EXEC_PID", "77")]),
                4242
            ),
            Some(false)
        );
        // A unit, but a systemd too old to say whose.
        assert_eq!(
            systemd_unit(&vars(&[("INVOCATION_ID", "0123abcd")]), 4242),
            None
        );

        let service = vars(&[("XPC_SERVICE_NAME", "synthetic.scryer.job")]);
        assert!(launchd_job(&service, 1));
        // A child of a desktop app inherits the app's service name.
        assert!(!launchd_job(
            &vars(&[("XPC_SERVICE_NAME", "application.synthetic.desktop.1.2")]),
            5150
        ));
        assert!(!launchd_job(&vars(&[("XPC_SERVICE_NAME", "0")]), 1));
        assert!(!launchd_job(&vars(&[]), 1));
    }

    #[test]
    fn allowlisted_values_keep_only_code_like_shapes() {
        assert_eq!(safe_token("json"), "json");
        assert_eq!(safe_token("docker"), "docker");
        assert_eq!(safe_token("/srv/scryer"), "<set>");
        assert_eq!(safe_token("http://nas.example:1"), "<set>");
        assert_eq!(safe_log_filter("info,scryer=debug"), "info,scryer=debug");
        assert_eq!(safe_log_filter("info /home/x"), "<set>");
    }

    #[test]
    fn debug_labels_become_snake_case() {
        assert_eq!(snake_case("MacosAppBundle"), "macos_app_bundle");
        assert_eq!(snake_case("Docker"), "docker");
        assert_eq!(snake_case("InApp"), "in_app");
    }

    #[test]
    fn scryer_variables_report_names_and_register_values() {
        let mut facts = HostFacts::default();
        facts.scryer_variables(&[
            ("SCRYER_LOG_FORMAT".to_string(), "json".to_string()),
            (
                "SCRYER_DATA_DIR".to_string(),
                "/srv/canary-data".to_string(),
            ),
            (
                "SCRYER_DB_PASSWORD".to_string(),
                "canary-db-secret".to_string(),
            ),
            ("SCRYER_METRICS".to_string(), "true".to_string()),
            ("OTHER".to_string(), "ignored-value".to_string()),
        ]);
        let reported = facts.environment["scryer_variables"].as_object().unwrap();
        assert_eq!(reported["SCRYER_LOG_FORMAT"], "json");
        assert_eq!(reported["SCRYER_METRICS"], "true");
        assert_eq!(reported["SCRYER_DATA_DIR"], "<set>");
        assert_eq!(reported["SCRYER_DB_PASSWORD"], "<set>");
        assert!(!reported.contains_key("OTHER"));
        assert!(
            facts
                .sensitive_paths
                .iter()
                .any(|(_, path)| path == "/srv/canary-data")
        );
        assert!(
            facts
                .sensitive_values
                .iter()
                .any(|value| value == "canary-db-secret")
        );
        assert!(!facts.sensitive_values.iter().any(|value| value == "true"));
    }

    #[test]
    fn content_disposition_names_the_bundle() {
        assert_eq!(
            content_disposition("scryer-diagnostics-0.22.3-20261110T120000Z.tar.zst"),
            "attachment; filename=\"scryer-diagnostics-0.22.3-20261110T120000Z.tar.zst\"; \
             filename*=UTF-8''scryer%2Ddiagnostics%2D0%2E22%2E3%2D20261110T120000Z%2Etar%2Ezst"
        );
    }

    #[tokio::test]
    async fn diagnostics_route_gates_on_a_full_session_and_the_settings_permission() {
        let context = common::TestContext::new().await;
        let admin = context
            .app
            .find_or_create_default_user()
            .await
            .expect("default administrator");
        let ordinary = context
            .app
            .create_user(
                &admin,
                "diagnostics-ordinary".into(),
                "ordinary-password".into(),
                AppPermissionMask::NONE,
                Vec::new(),
            )
            .await
            .expect("create ordinary actor");
        let ordinary_token = context
            .app
            .issue_access_token(&ordinary)
            .await
            .expect("issue ordinary token");
        let admin_token = context
            .app
            .issue_access_token(&admin)
            .await
            .expect("issue administrator token");

        let auth = AuthState {
            app: context.app.clone(),
            schema: context.schema.clone(),
            // The shared fixture leaves authless local access on, which would
            // resolve a default administrator for an anonymous request.
            auth_runtime: scryer_interface::context::AuthRuntimeStateHandle::new(
                scryer_interface::context::AuthRuntimeStateSnapshot {
                    form_login_enabled: true,
                    skip_login_for_local_ips: false,
                    effective_form_login_enabled: true,
                    webauthn_configured: false,
                    passkey_enabled: false,
                    env_override_active: false,
                    env_override_description: None,
                    epoch: 1,
                },
            ),
            rate_limiter: ScryerRateLimiter::from_env(Default::default()),
            ws_origin_policy: WebSocketOriginPolicy::default(),
            list_account_origin_policy: Default::default(),
            authless_web_client_proof: AuthlessWebClientProofState::new(),
        };
        let data_dir = tempfile::tempdir().expect("data dir");
        let state = DiagnosticsRouteState {
            auth,
            host: Arc::new(DiagnosticsHost {
                process: DiagnosticsProcessFacts {
                    started_at: Utc::now(),
                    log_file: None,
                },
                log_ring_buffer: LogRingBuffer::new(16),
                metrics: None,
                installation: InstallationAssessment::default(),
                data_dir: data_dir.path().to_path_buf(),
                listening: SocketAddr::from((Ipv4Addr::UNSPECIFIED, 8080)),
                base_path_configured: false,
                sqlite_path: None,
            }),
        };
        let router = Router::new()
            .route(
                DIAGNOSTICS_BUNDLE_PATH,
                get(download_diagnostics_bundle_handler),
            )
            .with_state(state);
        let request = |token: Option<&str>| {
            let mut request = axum::http::Request::builder()
                .method("GET")
                .uri(DIAGNOSTICS_BUNDLE_PATH)
                .body(Body::empty())
                .expect("diagnostics request");
            if let Some(token) = token {
                request.headers_mut().insert(
                    header::AUTHORIZATION,
                    HeaderValue::from_str(&format!("Bearer {token}")).expect("authorization"),
                );
            }
            request
                .extensions_mut()
                .insert(ConnectInfo(SocketAddr::from((Ipv4Addr::LOCALHOST, 3000))));
            request
        };
        let reason = |response: Response| async move {
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("refusal body");
            serde_json::from_slice::<Value>(&bytes).expect("refusal json")["reason"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        };

        let response = router
            .clone()
            .oneshot(request(None))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            reason(response).await,
            HTTP_DIAGNOSTICS_AUTHENTICATION_REQUIRED
        );

        let response = router
            .clone()
            .oneshot(request(Some(ordinary_token.as_str())))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(reason(response).await, HTTP_DIAGNOSTICS_PERMISSION_DENIED);

        let response = router
            .clone()
            .oneshot(request(Some(admin_token.as_str())))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            HeaderValue::from_static("application/zstd")
        );
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            HeaderValue::from_static("no-store")
        );
        let disposition = response.headers()[header::CONTENT_DISPOSITION]
            .to_str()
            .expect("disposition")
            .to_string();
        assert!(
            disposition.starts_with("attachment; filename=\"scryer-diagnostics-"),
            "{disposition}"
        );
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("bundle body");
        // The zstd frame magic number.
        assert_eq!(&bytes[..4], &[0x28, 0xB5, 0x2F, 0xFD]);
    }
}

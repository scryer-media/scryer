use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use clap::{Args, Parser, Subcommand};
use serde::Deserialize;
use sha2::{Digest, Sha256};
#[cfg(unix)]
use signal_hook::consts::signal::{SIGINT, SIGTERM};
#[cfg(unix)]
use signal_hook::iterator::{Handle as SignalHandle, Signals};
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use xtask_support::{TaskContext, ok, run_status, step, warn};

mod media_fixtures;
mod oauth_dev_flow;
mod profile;

const BACKEND_SHUTDOWN_GRACE_PERIOD: std::time::Duration = std::time::Duration::from_secs(5);
const DEFAULT_SERVE_BIND: &str = "127.0.0.1:18080";
const DEFAULT_SERVE_FRONTEND_PORT: u16 = 3000;
const DEFAULT_SERVE_BACKEND_RUST_MIN_STACK: &str = "16777216";
const XTASK_SERVE_DEV_API_KEY_SEED: &str = "admin|xtask-serve|ska_AAAAAAAAAAAAAAAAAAAAAA.AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA|never";
const BUILTIN_VERSION_MANIFEST_PATH: &str = "crates/scryer-plugins/builtin-versions.json";
const BUILTIN_ASSET_DIR: &str = "crates/scryer-plugins/builtins";

struct ServeBuiltinAssetSpec {
    plugin_id: &'static str,
    artifact_stem: &'static str,
}

const SERVE_BUILTIN_ASSETS: &[ServeBuiltinAssetSpec] = &[
    ServeBuiltinAssetSpec {
        plugin_id: "newznab",
        artifact_stem: "newznab_indexer",
    },
    ServeBuiltinAssetSpec {
        plugin_id: "torznab",
        artifact_stem: "torznab_indexer",
    },
];

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ServeBuiltinVersionManifest {
    schema_version: u32,
    plugins: BTreeMap<String, ServeBuiltinVersionPin>,
}

#[derive(Deserialize)]
struct ServeBuiltinVersionPin {
    version: String,
}

#[derive(Deserialize)]
struct ServeBuiltinDescriptor {
    id: String,
    version: String,
}

#[cfg(unix)]
struct SignalForwarder {
    handle: SignalHandle,
    process_groups: Arc<Mutex<Vec<u32>>>,
    shutdown_requested: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

#[cfg(unix)]
impl Drop for SignalForwarder {
    fn drop(&mut self) {
        self.handle.close();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(not(unix))]
struct SignalForwarder;

#[cfg(unix)]
impl SignalForwarder {
    fn replace_process_groups(&self, process_ids: &[u32]) {
        if let Ok(mut groups) = self.process_groups.lock() {
            groups.clear();
            groups.extend(process_ids.iter().copied());
        }
    }

    fn shutdown_requested(&self) -> bool {
        self.shutdown_requested.load(Ordering::SeqCst)
    }
}

#[cfg(not(unix))]
impl SignalForwarder {
    fn replace_process_groups(&self, _process_ids: &[u32]) {}

    fn shutdown_requested(&self) -> bool {
        false
    }
}

#[derive(Parser)]
#[command(name = "cargo xtask")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Release(ReleaseArgs),
    Builtins(BuiltinsArgs),
    TrashGuides(TrashGuidesArgs),
    Migrations(MigrationsArgs),
    MediaFixtures(media_fixtures::MediaFixturesArgs),
    BuildTestPluginFixture,
    #[command(name = "oauth")]
    OAuth(OAuthArgs),
    Sdk(SdkArgs),
    Ci(CiArgs),
    Serve(ServeArgs),
    Profile(ProfileArgs),
}

#[derive(Args)]
struct ReleaseArgs {
    #[arg(long, conflicts_with_all = ["minor", "patch", "version"])]
    major: bool,
    #[arg(long, conflicts_with_all = ["major", "patch", "version"])]
    minor: bool,
    #[arg(long, conflicts_with_all = ["major", "minor", "version"])]
    patch: bool,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    allow_graphql_dangerous: bool,
    version: Option<String>,
}

#[derive(Args)]
struct BuiltinsArgs {
    #[command(subcommand)]
    command: BuiltinsCommand,
}

#[derive(Args)]
struct TrashGuidesArgs {
    #[command(subcommand)]
    command: TrashGuidesCommand,
}

#[derive(Subcommand)]
enum TrashGuidesCommand {
    Sync,
}

#[derive(Subcommand)]
enum BuiltinsCommand {
    Sync,
    Materialize,
}

#[derive(Args)]
struct MigrationsArgs {
    #[command(subcommand)]
    command: MigrationsCommand,
}

#[derive(Subcommand)]
enum MigrationsCommand {
    Rebaseline(RebaselineArgs),
}

#[derive(Args, Clone)]
struct RebaselineArgs {
    #[arg(long)]
    through: i64,
    #[arg(long)]
    force: bool,
}

#[derive(Args)]
struct SdkArgs {
    #[command(subcommand)]
    command: SdkCommand,
}

#[derive(Args)]
struct OAuthArgs {
    #[command(subcommand)]
    command: OAuthCommand,
}

#[derive(Subcommand)]
enum OAuthCommand {
    DevFlow(oauth_dev_flow::OAuthDevFlowArgs),
}

#[derive(Subcommand)]
enum SdkCommand {
    Release(SdkReleaseArgs),
}

#[derive(Args)]
struct SdkReleaseArgs {
    version: String,
    #[arg(long)]
    dry_run: bool,
}

#[derive(Args)]
struct CiArgs {
    #[command(subcommand)]
    command: CiCommand,
}

#[derive(Subcommand)]
enum CiCommand {
    Clippy(ClippyArgs),
    Winget(WingetArgs),
}

#[derive(Args)]
struct ClippyArgs {
    #[arg(long)]
    linux_only: bool,
}

#[derive(Args)]
struct WingetArgs {
    #[arg(long)]
    version: String,
    #[arg(long)]
    tag: Option<String>,
    #[arg(long, default_value = "scryer-media/scryer")]
    repository: String,
    #[arg(long, default_value = "release-artifacts")]
    artifacts_dir: PathBuf,
    #[arg(long, default_value = "target/winget")]
    output_dir: PathBuf,
    #[arg(long)]
    release_date: Option<String>,
}

#[derive(Args)]
struct ServeArgs {
    #[arg(
        long,
        default_value = DEFAULT_SERVE_BIND,
        help = "Bind address for the locally hosted Scryer debug server"
    )]
    bind: String,
    #[arg(
        long,
        default_value_t = DEFAULT_SERVE_FRONTEND_PORT,
        help = "Port for the Vite dev server with hot reload"
    )]
    frontend_port: u16,
    #[arg(
        long,
        help = "Run xtask serve against a managed PostgreSQL Docker container instead of the default SQLite datastore"
    )]
    postgres: bool,
    #[arg(long, help = "Reset the selected datastore before starting Scryer")]
    clean: bool,
}

#[derive(Args)]
struct ProfileArgs {
    #[command(subcommand)]
    command: ProfileCommand,
}

#[derive(Subcommand)]
enum ProfileCommand {
    Hotpaths(ProfileHotpathsArgs),
}

#[derive(Args)]
struct ProfileHotpathsArgs {
    duration_seconds: Option<String>,
    interval_seconds: Option<String>,
}

#[derive(Clone, Copy)]
enum ServeMode {
    PreserveDatabase,
    CleanDatabase,
}

#[derive(Clone, Copy)]
enum ServeDatastoreKind {
    Sqlite,
    Postgres,
}

struct ServeDatastore {
    kind: ServeDatastoreKind,
    envs: Vec<(String, String)>,
    location: String,
}

struct ServePostgresConfig {
    image: String,
    container_name: String,
    volume_name: String,
    host_port: u16,
    database: String,
    user: String,
    password: String,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let ctx = TaskContext::new();

    match cli.command {
        Commands::Release(args) => delegate_release(&ctx, &args),
        Commands::Builtins(args) => delegate_builtins(&ctx, &args),
        Commands::TrashGuides(args) => delegate_trash_guides(&ctx, &args),
        Commands::Migrations(args) => delegate_migrations(&ctx, &args),
        Commands::MediaFixtures(args) => match args.command {
            media_fixtures::MediaFixturesCommand::Generate(args) => {
                media_fixtures::generate(&ctx, &args)
            }
        },
        Commands::BuildTestPluginFixture => build_test_plugin_fixture(&ctx),
        Commands::OAuth(args) => match args.command {
            OAuthCommand::DevFlow(args) => oauth_dev_flow::run(&ctx, args),
        },
        Commands::Sdk(args) => delegate_sdk(&ctx, &args),
        Commands::Ci(args) => delegate_ci(&ctx, &args),
        Commands::Serve(args) => {
            let mode = if args.clean {
                ServeMode::CleanDatabase
            } else {
                ServeMode::PreserveDatabase
            };
            serve_local_scryer(&ctx, args, mode)
        }
        Commands::Profile(args) => match args.command {
            ProfileCommand::Hotpaths(args) => profile_hotpaths(&ctx, args),
        },
    }
}

fn delegate_release(ctx: &TaskContext, args: &ReleaseArgs) -> Result<()> {
    let mut forwarded = vec!["release".to_string()];
    if args.major {
        forwarded.push("--major".to_string());
    }
    if args.minor {
        forwarded.push("--minor".to_string());
    }
    if args.patch {
        forwarded.push("--patch".to_string());
    }
    if args.dry_run {
        forwarded.push("--dry-run".to_string());
    }
    if args.allow_graphql_dangerous {
        forwarded.push("--allow-graphql-dangerous".to_string());
    }
    if let Some(version) = &args.version {
        forwarded.push(version.clone());
    }
    delegate_to_package(ctx, "xtask-release", &forwarded)
}

fn delegate_builtins(ctx: &TaskContext, args: &BuiltinsArgs) -> Result<()> {
    let forwarded = match args.command {
        BuiltinsCommand::Sync => vec!["builtins".to_string(), "sync".to_string()],
        BuiltinsCommand::Materialize => vec!["builtins".to_string(), "materialize".to_string()],
    };
    delegate_to_package(ctx, "xtask-release", &forwarded)
}

fn delegate_trash_guides(ctx: &TaskContext, args: &TrashGuidesArgs) -> Result<()> {
    let forwarded = match args.command {
        TrashGuidesCommand::Sync => vec!["sync".to_string()],
    };
    delegate_to_package(ctx, "xtask-trash-guides", &forwarded)
}

fn delegate_sdk(ctx: &TaskContext, args: &SdkArgs) -> Result<()> {
    let mut forwarded = vec!["sdk".to_string()];
    match &args.command {
        SdkCommand::Release(release) => {
            forwarded.push("release".to_string());
            forwarded.push(release.version.clone());
            if release.dry_run {
                forwarded.push("--dry-run".to_string());
            }
        }
    }
    delegate_to_package(ctx, "xtask-release", &forwarded)
}

fn delegate_ci(ctx: &TaskContext, args: &CiArgs) -> Result<()> {
    let mut forwarded = vec!["ci".to_string()];
    match &args.command {
        CiCommand::Clippy(clippy) => {
            forwarded.push("clippy".to_string());
            if clippy.linux_only {
                forwarded.push("--linux-only".to_string());
            }
        }
        CiCommand::Winget(winget) => {
            forwarded.push("winget".to_string());
            forwarded.push("--version".to_string());
            forwarded.push(winget.version.clone());
            if let Some(tag) = &winget.tag {
                forwarded.push("--tag".to_string());
                forwarded.push(tag.clone());
            }
            forwarded.push("--repository".to_string());
            forwarded.push(winget.repository.clone());
            forwarded.push("--artifacts-dir".to_string());
            forwarded.push(winget.artifacts_dir.to_string_lossy().into_owned());
            forwarded.push("--output-dir".to_string());
            forwarded.push(winget.output_dir.to_string_lossy().into_owned());
            if let Some(release_date) = &winget.release_date {
                forwarded.push("--release-date".to_string());
                forwarded.push(release_date.clone());
            }
        }
    }
    delegate_to_package(ctx, "xtask-release", &forwarded)
}

fn build_test_plugin_fixture(ctx: &TaskContext) -> Result<()> {
    let manifest = ctx.repo_root.join("test-plugins/test-indexer/Cargo.toml");
    let build_target = ctx.repo_root.join("target/test-plugin-build");
    let fixtures_dir = ctx.repo_root.join("target/test-plugin-fixtures");
    let fixture_dir = fixtures_dir.join("test-indexer");

    step("Building test indexer WebAssembly fixture from source");
    let mut locate_rustc = ctx.command_in("rustup", &ctx.repo_root);
    locate_rustc.args(["which", "rustc", "--toolchain", "1.97.1"]);
    let rustc = PathBuf::from(run_capture(&mut locate_rustc)?.trim());
    let mut build = ctx.command_in("rustup", &ctx.repo_root);
    build
        .args([
            "run",
            "1.97.1",
            "cargo",
            "build",
            "--locked",
            "--release",
            "--target",
            "wasm32-unknown-unknown",
            "--manifest-path",
        ])
        .arg(&manifest)
        .arg("--target-dir")
        .arg(&build_target)
        .env("RUSTC", rustc)
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS");
    run_checked(&mut build)?;

    let built_wasm = build_target.join("wasm32-unknown-unknown/release/test_indexer.wasm");
    if !built_wasm.is_file() {
        bail!(
            "test indexer build did not produce expected artifact: {}",
            built_wasm.display()
        );
    }

    fs::create_dir_all(&fixture_dir)
        .with_context(|| format!("failed to create {}", fixture_dir.display()))?;
    let fixture_wasm = fixture_dir.join("plugin.wasm");
    fs::copy(&built_wasm, &fixture_wasm).with_context(|| {
        format!(
            "failed to copy test indexer fixture from {} to {}",
            built_wasm.display(),
            fixture_wasm.display()
        )
    })?;

    if let Some(nextest_env) = std::env::var_os("NEXTEST_ENV") {
        let mut nextest_env = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&nextest_env)
            .context("failed to open NEXTEST_ENV for the generated fixture path")?;
        writeln!(
            nextest_env,
            "SCRYER_TEST_PLUGIN_FIXTURES_DIR={}",
            fixtures_dir.display()
        )
        .context("failed to export the generated fixture path through NEXTEST_ENV")?;
    }

    ok(format!(
        "Generated test plugin fixture at {}",
        fixture_wasm.display()
    ));
    Ok(())
}

fn delegate_migrations(ctx: &TaskContext, args: &MigrationsArgs) -> Result<()> {
    let mut forwarded = Vec::new();
    match &args.command {
        MigrationsCommand::Rebaseline(rebaseline) => {
            forwarded.push("rebaseline".to_string());
            forwarded.push("--through".to_string());
            forwarded.push(rebaseline.through.to_string());
            if rebaseline.force {
                forwarded.push("--force".to_string());
            }
        }
    }
    delegate_to_package(ctx, "xtask-migrations", &forwarded)
}

fn delegate_to_package(ctx: &TaskContext, package: &str, forwarded: &[String]) -> Result<()> {
    let mut command = ctx.command_in("cargo", &ctx.repo_root);
    command
        .arg("run")
        .arg("--locked")
        .arg("-p")
        .arg(package)
        .arg("--")
        .args(forwarded);
    run_checked(&mut command)
}

fn profile_hotpaths(ctx: &TaskContext, args: ProfileHotpathsArgs) -> Result<()> {
    profile::run(ctx, args)
}

fn tail_file(path: &Path, lines: usize) -> Result<String> {
    let content = fs::read_to_string(path).unwrap_or_default();
    let collected = content.lines().rev().take(lines).collect::<Vec<_>>();
    Ok(collected.into_iter().rev().collect::<Vec<_>>().join("\n"))
}

enum BackendStartupOutcome {
    Ready,
    Interrupted,
}

fn wait_for_local_backend(
    backend: &mut std::process::Child,
    port: u16,
    log_path: &Path,
    signal_forwarder: &SignalForwarder,
) -> Result<BackendStartupOutcome> {
    let address = format!("127.0.0.1:{port}");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    while std::time::Instant::now() < deadline {
        if signal_forwarder.shutdown_requested() {
            return Ok(BackendStartupOutcome::Interrupted);
        }

        if let Some(status) = backend.try_wait()? {
            let tail = tail_file(log_path, 50)?;
            bail!(
                "Scryer failed to start on http://{address}/ (status: {status}). Tail of {}:\n{tail}",
                log_path.display()
            );
        }

        if backend_ready_looks_ok(port) {
            return Ok(BackendStartupOutcome::Ready);
        }

        thread::sleep(std::time::Duration::from_millis(250));
    }

    let tail = tail_file(log_path, 50)?;
    bail!(
        "Timed out waiting for Scryer readiness on http://{address}/graphql. Tail of {}:\n{tail}",
        log_path.display()
    )
}

enum ServeWaitOutcome {
    FrontendExited(std::process::ExitStatus),
    Interrupted,
}

fn wait_for_serve_processes(
    backend: &mut Child,
    frontend: &mut Child,
    backend_log_path: &Path,
    signal_forwarder: &SignalForwarder,
) -> Result<ServeWaitOutcome> {
    loop {
        if signal_forwarder.shutdown_requested() {
            return Ok(ServeWaitOutcome::Interrupted);
        }

        if let Some(status) = backend.try_wait()? {
            let tail = tail_file(backend_log_path, 50)?;
            bail!(
                "Scryer backend exited while xtask serve was running (status: {status}). Tail of {}:\n{tail}",
                backend_log_path.display()
            );
        }

        if let Some(status) = frontend.try_wait()? {
            return Ok(ServeWaitOutcome::FrontendExited(status));
        }

        thread::sleep(std::time::Duration::from_millis(100));
    }
}

fn backend_ready_looks_ok(port: u16) -> bool {
    backend_health_looks_ok(port) && backend_graphql_looks_ready(port)
}

fn backend_health_looks_ok(port: u16) -> bool {
    http_request(
        port,
        &format!(
            "GET /health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAccept: application/json\r\nConnection: close\r\n\r\n"
        ),
    )
    .and_then(|(status_line, body)| {
        if !status_line.contains(" 200 ") {
            return None;
        }
        serde_json::from_str::<serde_json::Value>(&body).ok()
    })
    .and_then(|payload| {
        payload
            .get("status")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    })
    .as_deref()
        == Some("ok")
}

fn backend_graphql_looks_ready(port: u16) -> bool {
    if backend_graphql_auth_runtime_state_looks_ready(port, None) {
        return true;
    }

    authless_web_client_proof(port).is_some_and(|(cookie, proof)| {
        backend_graphql_auth_runtime_state_looks_ready(port, Some((&cookie, &proof)))
    })
}

fn backend_graphql_auth_runtime_state_looks_ready(
    port: u16,
    authless_headers: Option<(&str, &str)>,
) -> bool {
    let body = r#"{"query":"query { authRuntimeState { effectiveFormLoginEnabled skipLoginForLocalIps } }"}"#;
    let authless_headers = authless_headers
        .map(|(cookie, proof)| format!("Cookie: {cookie}\r\nx-scryer-web-client: {proof}\r\n"))
        .unwrap_or_default();
    let request = format!(
        "POST /graphql HTTP/1.1\r\n\
Host: 127.0.0.1:{port}\r\n\
Accept: application/json\r\n\
Content-Type: application/json\r\n\
{authless_headers}\
Content-Length: {content_length}\r\n\
Connection: close\r\n\r\n\
{body}",
        content_length = body.len(),
    );

    http_request(port, &request)
        .and_then(|(status_line, body)| {
            if !status_line.contains(" 200 ") {
                return None;
            }
            serde_json::from_str::<serde_json::Value>(&body).ok()
        })
        .and_then(|payload| {
            payload
                .get("data")
                .and_then(|data| data.get("authRuntimeState"))
                .cloned()
        })
        .is_some()
}

fn authless_web_client_proof(port: u16) -> Option<(String, String)> {
    let request = format!(
        "GET /authless-client HTTP/1.1\r\n\
Host: 127.0.0.1:{port}\r\n\
Accept: application/json\r\n\
Connection: close\r\n\r\n"
    );
    let (_, headers, body) = http_request_parts(port, &request)?;
    let cookie = authless_client_cookie(&headers)?;
    let proof = serde_json::from_str::<serde_json::Value>(&body)
        .ok()?
        .get("proof")
        .and_then(serde_json::Value::as_str)?
        .to_string();
    Some((cookie, proof))
}

fn authless_client_cookie(headers: &str) -> Option<String> {
    headers.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("set-cookie")
            .then(|| {
                value
                    .trim()
                    .split(';')
                    .next()
                    .unwrap_or_default()
                    .to_string()
            })
            .filter(|cookie| !cookie.is_empty())
    })
}

fn http_request(port: u16, request: &str) -> Option<(String, String)> {
    let (status_line, _, body) = http_request_parts(port, request)?;
    Some((status_line, body))
}

fn http_request_parts(port: u16, request: &str) -> Option<(String, String, String)> {
    let Ok(mut stream) = std::net::TcpStream::connect(("127.0.0.1", port)) else {
        return None;
    };
    let timeout = Some(std::time::Duration::from_millis(500));
    if stream.set_read_timeout(timeout).is_err() || stream.set_write_timeout(timeout).is_err() {
        return None;
    }
    if write!(stream, "{request}").is_err() {
        return None;
    }

    let mut response = String::new();
    if std::io::Read::read_to_string(&mut stream, &mut response).is_err() {
        return None;
    }

    let (headers, body) = response.split_once("\r\n\r\n")?;
    let mut header_lines = headers.lines();
    let status_line = header_lines.next()?;

    let body = if headers
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        decode_chunked_http_body(body)?
    } else {
        body.to_string()
    };

    Some((status_line.to_string(), headers.to_string(), body))
}

fn decode_chunked_http_body(body: &str) -> Option<String> {
    let mut decoded = String::new();
    let mut rest = body;

    loop {
        let (size_line, after_size_line) = rest.split_once("\r\n")?;
        let size = usize::from_str_radix(size_line.trim(), 16).ok()?;
        if size == 0 {
            return Some(decoded);
        }
        if after_size_line.len() < size + 2 {
            return None;
        }
        decoded.push_str(&after_size_line[..size]);
        rest = &after_size_line[size + 2..];
    }
}

fn backend_port(bind: &str) -> Result<u16> {
    let (_, port) = bind
        .rsplit_once(':')
        .ok_or_else(|| anyhow!("bind address must include a port: {bind}"))?;
    port.parse::<u16>()
        .with_context(|| format!("invalid port in bind address: {bind}"))
}

fn resolve_frontend_port(preferred: u16) -> Result<u16> {
    for offset in 0..=20u16 {
        let Some(candidate) = preferred.checked_add(offset) else {
            break;
        };
        if std::net::TcpListener::bind((std::net::Ipv4Addr::UNSPECIFIED, candidate)).is_ok() {
            return Ok(candidate);
        }
    }

    bail!("could not find an open Vite dev-server port starting at {preferred}")
}

fn serve_db_path() -> Result<PathBuf> {
    #[cfg(target_os = "macos")]
    let base_dir = std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join("Library/Application Support/scryer"))
        .unwrap_or_else(|| PathBuf::from("./scryer"));

    #[cfg(target_os = "linux")]
    let base_dir = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join(".local/share"))
        })
        .map(|base| base.join("scryer"))
        .unwrap_or_else(|| PathBuf::from("./scryer"));

    #[cfg(target_os = "windows")]
    let base_dir = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .map(|base| base.join("scryer"))
        .unwrap_or_else(|| PathBuf::from("./scryer"));

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    let base_dir = PathBuf::from("./scryer");

    let db_dir = base_dir.join("xtask");
    fs::create_dir_all(&db_dir)?;
    Ok(db_dir.join("scryer.db"))
}

fn reset_serve_database(db_path: &Path) -> Result<()> {
    let cleanup_targets = [
        db_path.to_path_buf(),
        PathBuf::from(format!("{}-wal", db_path.display())),
        PathBuf::from(format!("{}-shm", db_path.display())),
    ];

    step(format!(
        "Removing xtask serve database files under {}",
        db_path.display()
    ));
    for path in cleanup_targets {
        if path.exists() {
            fs::remove_file(&path)
                .with_context(|| format!("failed to remove {}", path.display()))?;
        }
    }
    ok("xtask serve database reset");
    Ok(())
}

fn serve_postgres_config() -> Result<ServePostgresConfig> {
    let host_port = std::env::var("SCRYER_XTASK_POSTGRES_PORT")
        .ok()
        .map(|value| {
            value.parse::<u16>().with_context(|| {
                format!("SCRYER_XTASK_POSTGRES_PORT must be a valid port, got {value}")
            })
        })
        .transpose()?
        .unwrap_or(55432);
    Ok(ServePostgresConfig {
        image: std::env::var("SCRYER_XTASK_POSTGRES_IMAGE")
            .unwrap_or_else(|_| "postgres:18".to_string()),
        container_name: std::env::var("SCRYER_XTASK_POSTGRES_CONTAINER")
            .unwrap_or_else(|_| "scryer-xtask-postgres".to_string()),
        volume_name: std::env::var("SCRYER_XTASK_POSTGRES_VOLUME")
            .unwrap_or_else(|_| "scryer-xtask-postgres-data".to_string()),
        host_port,
        database: std::env::var("SCRYER_XTASK_POSTGRES_DB")
            .unwrap_or_else(|_| "scryer".to_string()),
        user: std::env::var("SCRYER_XTASK_POSTGRES_USER").unwrap_or_else(|_| "scryer".to_string()),
        password: std::env::var("SCRYER_XTASK_POSTGRES_PASSWORD")
            .unwrap_or_else(|_| "scryer-dev-password".to_string()),
    })
}

fn docker_container_exists(ctx: &TaskContext, container_name: &str) -> Result<bool> {
    let mut inspect = ctx.command("docker");
    inspect.args(["container", "inspect", container_name]);
    Ok(inspect.output()?.status.success())
}

fn docker_volume_exists(ctx: &TaskContext, volume_name: &str) -> Result<bool> {
    let mut inspect = ctx.command("docker");
    inspect.args(["volume", "inspect", volume_name]);
    Ok(inspect.output()?.status.success())
}

fn reset_serve_postgres(ctx: &TaskContext, config: &ServePostgresConfig) -> Result<()> {
    step(format!(
        "Resetting xtask serve PostgreSQL container {} and volume {}",
        config.container_name, config.volume_name
    ));
    if docker_container_exists(ctx, &config.container_name)? {
        let mut rm = ctx.command("docker");
        rm.args(["rm", "-f", &config.container_name]);
        run_checked(&mut rm)?;
    }
    if docker_volume_exists(ctx, &config.volume_name)? {
        let mut rm = ctx.command("docker");
        rm.args(["volume", "rm", "-f", &config.volume_name]);
        run_checked(&mut rm)?;
    }
    ok("xtask serve PostgreSQL state reset");
    Ok(())
}

fn wait_for_serve_postgres(ctx: &TaskContext, config: &ServePostgresConfig) -> Result<()> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while std::time::Instant::now() < deadline {
        if matches!(
            docker_inspect_state(&config.container_name)?.as_deref(),
            Some("exited" | "dead")
        ) {
            warn(format!(
                "PostgreSQL container {} exited before becoming ready",
                config.container_name
            ));
            log_container_failure(&config.container_name)?;
            bail!(
                "PostgreSQL container {} exited before becoming ready",
                config.container_name
            );
        }

        let pg_url = format!(
            "postgresql://{}@127.0.0.1/{}?sslmode=disable",
            config.user, config.database
        );
        let mut psql = ctx.command("docker");
        psql.args([
            "exec",
            &config.container_name,
            "env",
            &format!("PGPASSWORD={}", config.password),
            "psql",
            &pg_url,
            "-c",
            "SELECT 1",
        ]);
        if run_status(&mut psql)?.success() {
            return Ok(());
        }

        thread::sleep(std::time::Duration::from_millis(500));
    }

    warn(format!(
        "Timed out waiting for PostgreSQL container {} to become ready",
        config.container_name
    ));
    log_container_failure(&config.container_name)?;
    bail!(
        "Timed out waiting for PostgreSQL container {} to become ready",
        config.container_name
    );
}

fn ensure_serve_postgres(ctx: &TaskContext, mode: ServeMode) -> Result<ServeDatastore> {
    require_command("docker")?;
    let config = serve_postgres_config()?;

    if matches!(mode, ServeMode::CleanDatabase) {
        reset_serve_postgres(ctx, &config)?;
    }

    if docker_container_exists(ctx, &config.container_name)? {
        let state = docker_inspect_state(&config.container_name)?;
        if !matches!(state.as_deref(), Some("running")) {
            step(format!(
                "Starting managed PostgreSQL container {}",
                config.container_name
            ));
            let mut start = ctx.command("docker");
            start.args(["start", &config.container_name]);
            run_checked(&mut start)?;
        } else {
            ok(format!(
                "Reusing managed PostgreSQL container {}",
                config.container_name
            ));
        }
    } else {
        step(format!(
            "Creating managed PostgreSQL container {} from {}",
            config.container_name, config.image
        ));
        let mut run = ctx.command("docker");
        run.args([
            "run",
            "-d",
            "--name",
            &config.container_name,
            "-e",
            &format!("POSTGRES_DB={}", config.database),
            "-e",
            &format!("POSTGRES_USER={}", config.user),
            "-e",
            &format!("POSTGRES_PASSWORD={}", config.password),
            "-p",
            &format!("{}:5432", config.host_port),
            "-v",
            &format!("{}:/var/lib/postgresql", config.volume_name),
            &config.image,
        ]);
        run_checked(&mut run)?;
    }

    step(format!(
        "Waiting for PostgreSQL on 127.0.0.1:{}",
        config.host_port
    ));
    wait_for_serve_postgres(ctx, &config)?;
    ok(format!(
        "Managed PostgreSQL is ready in container {}",
        config.container_name
    ));

    Ok(ServeDatastore {
        kind: ServeDatastoreKind::Postgres,
        envs: vec![
            (
                "SCRYER_DB_URL".to_string(),
                format!(
                    "postgres://127.0.0.1:{}/{}?sslmode=disable",
                    config.host_port, config.database
                ),
            ),
            ("SCRYER_DB_USER".to_string(), config.user.clone()),
            ("SCRYER_DB_PASSWORD".to_string(), config.password.clone()),
        ],
        location: format!(
            "postgres://127.0.0.1:{}/{}?sslmode=disable (container={}, volume={})",
            config.host_port, config.database, config.container_name, config.volume_name
        ),
    })
}

fn prepare_serve_datastore(
    ctx: &TaskContext,
    args: &ServeArgs,
    mode: ServeMode,
) -> Result<ServeDatastore> {
    if args.postgres {
        return ensure_serve_postgres(ctx, mode);
    }

    let db_path = serve_db_path()?;
    if matches!(mode, ServeMode::CleanDatabase) {
        reset_serve_database(&db_path)?;
    }
    let db_url = format!("sqlite://{}", db_path.display());
    Ok(ServeDatastore {
        kind: ServeDatastoreKind::Sqlite,
        envs: vec![("SCRYER_DB_PATH".to_string(), db_url)],
        location: db_path.display().to_string(),
    })
}

fn serve_encryption_key() -> String {
    let digest = Sha256::digest(b"scryer-xtask-dev-encryption-key");
    base64::engine::general_purpose::STANDARD.encode(digest)
}

fn dotenv_or_process_env(dotenv_envs: &[(String, String)], key: &str) -> Option<String> {
    std::env::var(key).ok().or_else(|| {
        dotenv_envs.iter().find_map(|(dotenv_key, value)| {
            (dotenv_key == key && !value.trim().is_empty()).then(|| value.clone())
        })
    })
}

fn ensure_frontend_dependencies(ctx: &TaskContext, web_dir: &Path) -> Result<()> {
    step("Syncing frontend dependencies for Vite dev server");
    let mut install = ctx.command_in("npm", web_dir);
    install.args(["install", "--no-fund", "--no-audit"]);
    run_status(&mut install).with_context(|| {
        format!(
            "failed to install frontend dependencies in {}",
            web_dir.display()
        )
    })?;
    ok("Frontend dependencies are up to date");
    Ok(())
}

fn ensure_serve_builtin_plugins(ctx: &TaskContext) -> Result<()> {
    if serve_builtin_assets_are_current(ctx)? {
        ok("Built-in plugin assets are current");
        return Ok(());
    }

    step("Built-in plugin assets are missing or stale; materializing pinned versions");
    delegate_to_package(
        ctx,
        "xtask-release",
        &["builtins".to_string(), "materialize".to_string()],
    )?;

    if !serve_builtin_assets_are_current(ctx)? {
        bail!("built-in plugin materialization completed without current assets");
    }

    ok("Built-in plugin assets are current");
    Ok(())
}

fn serve_builtin_assets_are_current(ctx: &TaskContext) -> Result<bool> {
    let manifest_path = ctx.path(BUILTIN_VERSION_MANIFEST_PATH);
    let manifest = fs::read(&manifest_path)
        .with_context(|| format!("failed to read {}", manifest_path.display()))
        .and_then(|bytes| {
            serde_json::from_slice::<ServeBuiltinVersionManifest>(&bytes)
                .with_context(|| format!("failed to parse {}", manifest_path.display()))
        })?;

    Ok(manifest.schema_version == 2
        && serve_builtin_assets_match_manifest(&ctx.path(BUILTIN_ASSET_DIR), &manifest))
}

fn serve_builtin_assets_match_manifest(
    asset_dir: &Path,
    manifest: &ServeBuiltinVersionManifest,
) -> bool {
    SERVE_BUILTIN_ASSETS.iter().all(|spec| {
        let Some(pin) = manifest.plugins.get(spec.plugin_id) else {
            return false;
        };
        let wasm = asset_dir.join(format!("{}.wasm.zst", spec.artifact_stem));
        let descriptor_path = asset_dir.join(format!("{}.descriptor.json", spec.artifact_stem));
        let description = asset_dir.join(format!("{}.description.txt", spec.artifact_stem));
        if !nonempty_file(&wasm) || !nonempty_file(&descriptor_path) || !nonempty_file(&description)
        {
            return false;
        }

        serde_json::from_slice::<ServeBuiltinDescriptor>(
            &fs::read(descriptor_path).unwrap_or_default(),
        )
        .is_ok_and(|descriptor| {
            descriptor.id == spec.plugin_id
                && descriptor.version.trim_start_matches('v') == pin.version.trim_start_matches('v')
        })
    })
}

fn nonempty_file(path: &Path) -> bool {
    path.is_file() && fs::metadata(path).is_ok_and(|metadata| metadata.len() > 0)
}

fn serve_local_scryer(ctx: &TaskContext, args: ServeArgs, mode: ServeMode) -> Result<()> {
    ensure_serve_builtin_plugins(ctx)?;
    require_command("npm")?;

    let env_file = ctx.path(".env");
    let mut dotenv_envs = Vec::new();
    if env_file.is_file() {
        let dotenv_iter = dotenvy::from_path_iter(&env_file)
            .with_context(|| format!("failed to read {}", env_file.display()))?;
        for entry in dotenv_iter {
            let (key, value) =
                entry.with_context(|| format!("failed to parse {}", env_file.display()))?;
            dotenv_envs.push((key, value));
        }
    }

    let web_dir = ctx.path("apps/scryer-web");
    ensure_frontend_dependencies(ctx, &web_dir)?;

    let backend_port = backend_port(&args.bind)?;
    let frontend_port = resolve_frontend_port(args.frontend_port)?;
    let backend_url = format!("http://127.0.0.1:{backend_port}");
    let frontend_url = format!("http://localhost:{frontend_port}");
    let vite_use_polling =
        std::env::var("SCRYER_VITE_USE_POLLING").unwrap_or_else(|_| "true".to_string());
    let vite_poll_interval =
        std::env::var("SCRYER_VITE_POLL_INTERVAL_MS").unwrap_or_else(|_| "250".to_string());
    let backend_rust_min_stack = dotenv_or_process_env(&dotenv_envs, "RUST_MIN_STACK")
        .unwrap_or_else(|| DEFAULT_SERVE_BACKEND_RUST_MIN_STACK.to_string());
    let metrics =
        dotenv_or_process_env(&dotenv_envs, "SCRYER_METRICS").unwrap_or_else(|| "true".to_string());
    let encryption_key = serve_encryption_key();
    let webauthn_rp_id = dotenv_or_process_env(&dotenv_envs, "SCRYER_WEBAUTHN_RP_ID")
        .unwrap_or_else(|| "localhost".to_string());
    let webauthn_rp_origin = dotenv_or_process_env(&dotenv_envs, "SCRYER_WEBAUTHN_RP_ORIGIN")
        .unwrap_or_else(|| frontend_url.clone());
    let backend_binary = ctx.path("target/debug/scryer");
    let backend_log = PathBuf::from(
        std::env::var("SCRYER_DEV_BACKEND_LOG")
            .unwrap_or_else(|_| "/tmp/scryer-dev-backend.log".to_string()),
    );
    if let Some(parent) = backend_log.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&backend_log, "")?;

    step("Building Scryer backend");
    let mut build = ctx.command_in("cargo", &ctx.repo_root);
    build.args(["build", "--locked", "-p", "scryer"]);
    run_checked(&mut build)?;

    let datastore = prepare_serve_datastore(ctx, &args, mode)?;

    step(format!(
        "Starting Scryer backend from {} on {}",
        backend_binary.display(),
        args.bind
    ));
    if env_file.is_file() {
        ok(format!(
            "Loaded runtime environment from {}",
            env_file.display()
        ));
    }
    if frontend_port != args.frontend_port {
        warn(format!(
            "frontend port {} is busy; using {} for the Vite dev server",
            args.frontend_port, frontend_port
        ));
    }
    println!("   Vite dev server: {frontend_url}");
    println!("   Vite file watch: polling={vite_use_polling} interval_ms={vite_poll_interval}");
    println!("   Keychain: disabled for xtask serve");
    println!("   Development API key: seeded for admin (label: xtask-serve)");
    println!("   Backend RUST_MIN_STACK: {backend_rust_min_stack}");
    println!("   Metrics: {metrics} (/metrics)");
    println!("   WebAuthn RP ID: {webauthn_rp_id}");
    println!("   WebAuthn RP origin: {webauthn_rp_origin}");
    match datastore.kind {
        ServeDatastoreKind::Sqlite => println!("   Datastore: SQLite ({})", datastore.location),
        ServeDatastoreKind::Postgres => {
            println!("   Datastore: PostgreSQL ({})", datastore.location)
        }
    }

    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&backend_log)?;
    let log_err = log.try_clone()?;
    let mut serve = ctx.command(&backend_binary);
    configure_child_process_group(&mut serve);
    for (key, value) in &dotenv_envs {
        serve.env(key, value);
    }
    serve
        .env_remove("SCRYER_DB_URL")
        .env_remove("SCRYER_DB_PATH")
        .env_remove("SCRYER_DB_USER")
        .env_remove("SCRYER_DB_PASSWORD")
        .env_remove("SCRYER_DB_PASSWORD_FILE");
    for (key, value) in &datastore.envs {
        serve.env(key, value);
    }
    serve
        .env("SCRYER_DISABLE_PLATFORM_KEYSTORE", "1")
        .env("SCRYER_DEV_API_KEYS", XTASK_SERVE_DEV_API_KEY_SEED)
        .env("SCRYER_ENCRYPTION_KEY", &encryption_key)
        .env("SCRYER_METRICS", &metrics)
        .env("SCRYER_OPEN_BROWSER", "false")
        .env("SCRYER_WEB_UI_URL", &frontend_url)
        .env("SCRYER_WEBAUTHN_RP_ID", &webauthn_rp_id)
        .env("SCRYER_WEBAUTHN_RP_ORIGIN", &webauthn_rp_origin)
        .env("SCRYER_BIND", &args.bind)
        .env("RUST_MIN_STACK", &backend_rust_min_stack)
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err));
    let mut backend = serve.spawn()?;
    let signal_forwarder = install_signal_forwarder(&[backend.id()])?;
    match wait_for_local_backend(&mut backend, backend_port, &backend_log, &signal_forwarder) {
        Ok(BackendStartupOutcome::Ready) => {}
        Ok(BackendStartupOutcome::Interrupted) => {
            drop(signal_forwarder);
            terminate_child_process_group(&mut backend);
            return Ok(());
        }
        Err(error) => {
            drop(signal_forwarder);
            terminate_child_process_group(&mut backend);
            return Err(error);
        }
    }

    println!("==> Scryer backend ready");
    println!("    Backend:  {backend_url}");
    println!("    Frontend: {frontend_url}");
    println!("    Datastore: {}", datastore.location);
    println!("    Log:      tail -f {}", backend_log.display());
    println!();
    println!("==> Starting Vite dev server with live updates...");

    let mut vite = ctx.command_in("npm", &web_dir);
    configure_child_process_group(&mut vite);
    vite.env("SCRYER_DEV_PROXY_TARGET", &backend_url)
        .env("SCRYER_VITE_USE_POLLING", &vite_use_polling)
        .env("SCRYER_VITE_POLL_INTERVAL_MS", &vite_poll_interval)
        .args([
            "run",
            "dev",
            "--",
            "--host",
            "0.0.0.0",
            "--strictPort",
            "--port",
            &frontend_port.to_string(),
        ]);
    let mut vite = vite.spawn()?;
    signal_forwarder.replace_process_groups(&[backend.id(), vite.id()]);

    let result = wait_for_serve_processes(&mut backend, &mut vite, &backend_log, &signal_forwarder);

    drop(signal_forwarder);
    terminate_child_process_group(&mut vite);
    terminate_child_process_group(&mut backend);

    match result? {
        ServeWaitOutcome::Interrupted => Ok(()),
        ServeWaitOutcome::FrontendExited(status) if status.success() => Ok(()),
        ServeWaitOutcome::FrontendExited(status) => {
            bail!("Vite dev server exited with status {status}")
        }
    }
}

#[cfg(unix)]
fn configure_child_process_group(command: &mut Command) {
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) == 0 {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        });
    }
}

#[cfg(not(unix))]
fn configure_child_process_group(_command: &mut Command) {}

#[cfg(unix)]
fn install_signal_forwarder(process_ids: &[u32]) -> Result<SignalForwarder> {
    let mut signals = Signals::new([SIGINT, SIGTERM])?;
    let handle = signals.handle();
    let process_groups = Arc::new(Mutex::new(process_ids.to_vec()));
    let process_groups_for_thread = Arc::clone(&process_groups);
    let shutdown_requested = Arc::new(AtomicBool::new(false));
    let shutdown_requested_for_thread = Arc::clone(&shutdown_requested);
    let thread = thread::spawn(move || {
        for signal in signals.forever() {
            shutdown_requested_for_thread.store(true, Ordering::SeqCst);
            let process_groups = process_groups_for_thread
                .lock()
                .map(|groups| groups.clone())
                .unwrap_or_default();
            for process_id in process_groups {
                let _ = signal_process_group(process_id, signal);
            }
        }
    });
    Ok(SignalForwarder {
        handle,
        process_groups,
        shutdown_requested,
        thread: Some(thread),
    })
}

#[cfg(not(unix))]
fn install_signal_forwarder(_process_ids: &[u32]) -> Result<SignalForwarder> {
    Ok(SignalForwarder)
}

fn terminate_child_process_group(child: &mut Child) {
    #[cfg(unix)]
    {
        let process_id = child.id();
        let _ = signal_process_group(process_id, SIGINT);
        if wait_for_child_exit(child, BACKEND_SHUTDOWN_GRACE_PERIOD) {
            return;
        }
        let _ = signal_process_group(process_id, libc::SIGKILL);
        let _ = child.wait();
    }

    #[cfg(not(unix))]
    {
        let _ = child.kill();
        let _ = child.wait();
    }
}

#[cfg(unix)]
fn wait_for_child_exit(backend: &mut Child, timeout: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match backend.try_wait() {
            Ok(Some(_)) => return true,
            Ok(None) if std::time::Instant::now() < deadline => {
                thread::sleep(std::time::Duration::from_millis(100));
            }
            Ok(None) => return false,
            Err(_) => return false,
        }
    }
}

#[cfg(unix)]
fn signal_process_group(process_id: u32, signal: i32) -> io::Result<()> {
    let process_group = i32::try_from(process_id)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "process id overflow"))?;
    let result = unsafe { libc::kill(-process_group, signal) };
    if result == 0 {
        return Ok(());
    }

    let error = io::Error::last_os_error();
    if matches!(error.raw_os_error(), Some(libc::ESRCH)) {
        return Ok(());
    }
    Err(error)
}

fn docker_inspect_state(container: &str) -> Result<Option<String>> {
    let output = Command::new("docker")
        .args(["inspect", "--format", "{{.State.Status}}", container])
        .output()?;
    if !output.status.success() {
        return Ok(None);
    }

    Ok(Some(
        String::from_utf8_lossy(&output.stdout).trim().to_owned(),
    ))
}

fn log_container_failure(container: &str) -> Result<()> {
    eprintln!("Recent logs for {container}:");
    let mut command = Command::new("docker");
    command.args(["logs", "--tail", "200", container]);
    let _ = run_status(&mut command);
    Ok(())
}

pub(crate) fn require_command(command: &str) -> Result<()> {
    xtask_support::require_command(command)
}

pub(crate) fn run_checked(command: &mut Command) -> Result<()> {
    xtask_support::run_checked(command)
}

pub(crate) fn run_capture(command: &mut Command) -> Result<String> {
    xtask_support::run_capture(command)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(version: &str) -> ServeBuiltinVersionManifest {
        ServeBuiltinVersionManifest {
            schema_version: 2,
            plugins: SERVE_BUILTIN_ASSETS
                .iter()
                .map(|spec| {
                    (
                        spec.plugin_id.to_string(),
                        ServeBuiltinVersionPin {
                            version: version.to_string(),
                        },
                    )
                })
                .collect(),
        }
    }

    fn write_builtin_assets(asset_dir: &Path, version: &str) {
        fs::create_dir_all(asset_dir).unwrap();
        for spec in SERVE_BUILTIN_ASSETS {
            fs::write(
                asset_dir.join(format!("{}.wasm.zst", spec.artifact_stem)),
                b"wasm",
            )
            .unwrap();
            fs::write(
                asset_dir.join(format!("{}.descriptor.json", spec.artifact_stem)),
                serde_json::json!({ "id": spec.plugin_id, "version": version }).to_string(),
            )
            .unwrap();
            fs::write(
                asset_dir.join(format!("{}.description.txt", spec.artifact_stem)),
                "description",
            )
            .unwrap();
        }
    }

    #[test]
    fn builtin_preflight_accepts_matching_assets() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = manifest("2.0.1");
        write_builtin_assets(temp.path(), "2.0.1");

        assert!(serve_builtin_assets_match_manifest(temp.path(), &manifest));
    }

    #[test]
    fn builtin_preflight_requires_materialization_for_missing_or_stale_assets() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = manifest("2.0.1");
        write_builtin_assets(temp.path(), "2.0.0");

        assert!(!serve_builtin_assets_match_manifest(temp.path(), &manifest));

        write_builtin_assets(temp.path(), "2.0.1");
        fs::remove_file(temp.path().join("torznab_indexer.wasm.zst")).unwrap();

        assert!(!serve_builtin_assets_match_manifest(temp.path(), &manifest));
    }
}

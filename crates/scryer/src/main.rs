// async-graphql schema expansion exceeded the default macro recursion depth.
#![recursion_limit = "256"]

mod backup_routes;
mod base_path;
mod http_error;
mod init;
mod log_buffer;
mod middleware;
mod oauth_routes;
mod rate_limit;
mod settings_bootstrap;
mod splash;
mod startup_auth;
mod startup_migrations;
mod ui_assets;

use std::ffi::OsString;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use axum::Router;
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use scryer_application::{
    AUTO_BACKUP_POST_UPGRADE_PENDING_VERSION_KEY, AppUseCase, ArchiveExtractorPluginProvider,
    AutoBackupRunOutcome, DownloadClientPluginProvider, DownloadQueuePollerOptions, FacetRegistry,
    IndexerPluginProvider, JobTriggerSource, MovieFacetHandler, NotificationPluginProvider,
    PLUGIN_HTTP_CA_BUNDLE_PEM_KEY, PluginHttpTrustConfigRuntime, PluginInstallationRepository,
    RUNTIME_PLUGIN_LOAD_CONCURRENCY, RuntimePluginLoad, SETTINGS_SCOPE_SYSTEM, SeriesFacetHandler,
    SubtitlePluginProvider, SystemInfoProvider, TitleImageKind, TitleImageRepository,
    load_runtime_plugin_from_persisted_installation_payload, start_background_acquisition_poller,
    start_background_auto_backup_scheduler, start_background_download_delete_poller,
    start_background_library_refresh_loop, start_background_manual_import_poller,
    start_background_subtitle_poller, start_background_title_hydration_loop,
    start_background_title_image_loop, start_download_queue_poller_with_options,
    start_notification_dispatcher,
    tracked_downloads::{
        BridgedClientTypesHandle, TrackedDownloadHandle, TrackedDownloadSnapshotIngestHandle,
    },
};
use scryer_infrastructure::{
    BuiltinDownloadClientConnectionTester, DatastoreAssembly, DatastoreConfig,
    DatastoreCustomizationStore, DatastoreEngine, FileSystemLibraryRenamer,
    FileSystemLibraryScanner, FileSystemStagedNzbStore, ImageProxyRuntime, MetadataGatewayClient,
    MigrationMode, MultiIndexerSearchClient, PrioritizedDownloadClientRouter, SettingsStore,
    SmgEnrollmentConfig, resolve_datastore_config_from_env,
    restore_backup_bundle_to_datastore_path, start_weaver_bridge_supervisor, validate_datastore,
};
use scryer_interface::context::{
    AuthRuntimeStateHandle, AuthRuntimeStateSnapshot, RestoreContext, RestoreDatastoreConfig,
    RestoreDatastoreEngine, RestoreDatastoreHandle, RestoreMigrationMode, RestoreRestartHandle,
    RestoreSqliteDatastoreRequest,
};
use scryer_interface::{LogBuffer, build_schema_with_log_buffer_and_restore};
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tower_http::compression::CompressionLayer;
use url::Url;
use webauthn_rs::WebauthnBuilder;

use backup_routes::{
    BackupRouteState, download_backup_handler, finalize_pending_restore_if_present,
};
use base_path::BasePath;
use middleware::{
    AuthState, AuthlessAccessAllowlist, AuthlessAccessGuardState, AuthlessAccessPolicy,
    AuthlessWebClientProofRouteState, AuthlessWebClientProofState, CorsConfig,
    UNAUTHENTICATED_PUBLIC_ACCESS_ALLOWLIST_ENV, WebSocketOriginPolicy,
    authless_web_client_proof_handler, cors_handler, enforce_authless_access_guard,
    graphql_handler, graphql_ws_handler, health_handler, rate_limit_http_api,
};
use oauth_routes::{OAuthRouteState, oauth_router};
use rate_limit::ScryerRateLimiter;
use settings_bootstrap::{
    MOVIES_PATH_KEY, SERIES_PATH_KEY, extract_pending_migration_ids,
    migrate_legacy_download_client_default_category_settings,
    migrate_legacy_download_client_routing_settings, normalize_media_path_setting,
    normalize_quality_profile_settings, parse_migration_mode, seed_service_setting_definitions,
    seed_service_settings_from_environment,
};
use splash::{BootstrapStatus, SplashState, build_splash_router};
use startup_auth::ensure_admin_password_configured;
use ui_assets::{UiAssetMode, ui_asset_mode, ui_fallback};

include!(concat!(env!("OUT_DIR"), "/smg_build_assets.rs"));

const VERSION: &str = env!("CARGO_PKG_VERSION");
const LEGACY_NZBGEEK_PLUGIN_ID: &str = "nzbgeek";
const RECOVERY_ADMIN_PASSWORD_ENV: &str = "SCRYER_RECOVERY_ADMIN_PASSWORD";
const ALLOW_UNAUTHENTICATED_PUBLIC_ACCESS_ENV: &str = "SCRYER_ALLOW_UNAUTHENTICATED_PUBLIC_ACCESS";
const LOG_FILE_ENV: &str = "SCRYER_LOG_FILE";

fn compiled_binary_lane() -> scryer_runtime_info::BinaryLane {
    scryer_runtime_info::BinaryLane::parse(env!("SCRYER_COMPILED_BUILD_LANE"))
        .expect("SCRYER_COMPILED_BUILD_LANE must be emitted by build.rs")
}

fn spawn_plugin_catalog_refresh_task(app_use_case: AppUseCase) {
    tokio::spawn(async move {
        if let Err(error) = app_use_case.refresh_plugin_catalog_internal().await {
            tracing::warn!(error = %error, "failed to refresh plugin catalog in background");
            return;
        }
        if let Err(error) = app_use_case
            .migrate_nzbgeek_builtin_to_official_internal()
            .await
        {
            tracing::warn!(error = %error, "failed to migrate nzbgeek builtin plugin");
        }
    });
}

fn spawn_sigstore_trust_root_prime_task(app_use_case: AppUseCase) {
    tokio::spawn(async move {
        loop {
            match app_use_case.prime_plugin_trust_roots_internal().await {
                Ok(()) => {
                    tracing::info!("sigstore trust roots primed");
                    break;
                }
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        "failed to prime sigstore trust roots; retrying in 5 minutes"
                    );
                    tokio::time::sleep(Duration::from_secs(300)).await;
                }
            }
        }
    });
}

fn restore_datastore_config(config: &DatastoreConfig) -> RestoreDatastoreConfig {
    RestoreDatastoreConfig {
        engine: match config.engine {
            DatastoreEngine::Sqlite => RestoreDatastoreEngine::Sqlite,
            DatastoreEngine::Postgres => RestoreDatastoreEngine::Postgres,
        },
        migration_mode: match config.migration_mode {
            MigrationMode::ValidateOnly => RestoreMigrationMode::ValidateOnly,
            MigrationMode::Apply => RestoreMigrationMode::Apply,
        },
    }
}

fn restore_migration_mode_to_infra(mode: RestoreMigrationMode) -> MigrationMode {
    match mode {
        RestoreMigrationMode::ValidateOnly => MigrationMode::ValidateOnly,
        RestoreMigrationMode::Apply => MigrationMode::Apply,
    }
}

fn restore_datastore_handle() -> RestoreDatastoreHandle {
    RestoreDatastoreHandle::new(|request: RestoreSqliteDatastoreRequest| {
        let RestoreSqliteDatastoreRequest {
            target_db_path,
            migration_mode,
            bundle_path,
            passphrase,
        } = request;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| {
                scryer_application::AppError::Repository(format!(
                    "failed to start restore runtime: {error}"
                ))
            })?;

        runtime.block_on(restore_backup_bundle_to_datastore_path(
            &target_db_path,
            restore_migration_mode_to_infra(migration_mode),
            &bundle_path,
            passphrase.as_deref(),
        ))
    })
}

fn plugin_type_belongs_to_indexer_family(plugin_type: &str) -> bool {
    matches!(
        plugin_type,
        "indexer" | "usenet_indexer" | "torrent_indexer"
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AuthModeConfig {
    env_override_form_login_enabled: Option<bool>,
    env_override_description: Option<String>,
    used_legacy_dev_auto_login: bool,
    recovery_admin_password_set: bool,
    allow_unauthenticated_public_access: bool,
}

impl AuthModeConfig {
    fn env_override_active(&self) -> bool {
        self.env_override_form_login_enabled.is_some()
    }

    fn effective_form_login_enabled(&self, saved_form_login_enabled: bool) -> bool {
        self.env_override_form_login_enabled
            .unwrap_or(saved_form_login_enabled)
    }

    fn effective_skip_login_for_local_ips(&self, saved_skip_login_for_local_ips: bool) -> bool {
        saved_skip_login_for_local_ips && !self.recovery_active()
    }

    fn recovery_active(&self) -> bool {
        self.recovery_admin_password_set
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RestartSpec {
    executable: PathBuf,
    args: Vec<OsString>,
    env: Vec<(OsString, OsString)>,
    current_dir: PathBuf,
}

fn restart_spec_from_parts(
    executable: PathBuf,
    args: Vec<OsString>,
    env: Vec<(OsString, OsString)>,
    current_dir: PathBuf,
) -> RestartSpec {
    RestartSpec {
        executable,
        args,
        env,
        current_dir,
    }
}

fn current_restart_spec() -> io::Result<RestartSpec> {
    let executable = std::env::current_exe()?;
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    let env = std::env::vars_os().collect::<Vec<_>>();
    let current_dir = std::env::current_dir().or_else(|_| {
        executable.parent().map(PathBuf::from).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "failed to determine current directory for restart",
            )
        })
    })?;
    Ok(restart_spec_from_parts(executable, args, env, current_dir))
}

#[cfg(unix)]
fn restart_current_process(spec: &RestartSpec) -> io::Result<()> {
    use std::os::unix::process::CommandExt;

    let mut command = Command::new(&spec.executable);
    command.current_dir(&spec.current_dir);
    command.args(&spec.args);
    command.env_clear();
    for (key, value) in &spec.env {
        command.env(key, value);
    }
    Err(command.exec())
}

#[cfg(not(unix))]
fn restart_current_process(spec: &RestartSpec) -> io::Result<()> {
    let mut command = Command::new(&spec.executable);
    command.current_dir(&spec.current_dir);
    command.args(&spec.args);
    command.env_clear();
    for (key, value) in &spec.env {
        command.env(key, value);
    }
    let _child = command.spawn()?;
    std::process::exit(0);
}

#[derive(Clone)]
struct SelfRestartController {
    inner: Arc<SelfRestartControllerInner>,
}

struct SelfRestartControllerInner {
    delay: Duration,
    scheduled: AtomicBool,
    launcher: Arc<dyn Fn() -> io::Result<()> + Send + Sync>,
}

impl SelfRestartController {
    fn new(delay: Duration) -> io::Result<Self> {
        let spec = Arc::new(current_restart_spec()?);
        Ok(Self::with_launcher(
            delay,
            Arc::new(move || restart_current_process(&spec)),
        ))
    }

    fn with_launcher(
        delay: Duration,
        launcher: Arc<dyn Fn() -> io::Result<()> + Send + Sync>,
    ) -> Self {
        Self {
            inner: Arc::new(SelfRestartControllerInner {
                delay,
                scheduled: AtomicBool::new(false),
                launcher,
            }),
        }
    }

    fn handle(&self) -> RestoreRestartHandle {
        let controller = self.clone();
        RestoreRestartHandle::new(move || controller.schedule_restart())
    }

    fn schedule_restart(&self) {
        if self.inner.scheduled.swap(true, Ordering::SeqCst) {
            tracing::info!("restore restart already scheduled");
            return;
        }

        let inner = self.inner.clone();
        std::thread::spawn(move || {
            std::thread::sleep(inner.delay);
            if let Err(error) = (inner.launcher)() {
                tracing::error!(error = %error, "failed to restart after restore");
                inner.scheduled.store(false, Ordering::SeqCst);
            }
        });
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum VersionLifecycle {
    FirstRun,
    Unchanged,
    Upgraded { previous: String },
}

fn log_smg_version_incompatibility(
    incompat: &scryer_infrastructure::smg_enrollment::VersionIncompatible,
) {
    let env = if std::path::Path::new("/.dockerenv").exists() {
        "docker"
    } else {
        "binary"
    };
    let blocked = incompat.status.eq_ignore_ascii_case("blocked");
    let level_message = if blocked {
        "SMG upgrade required"
    } else {
        "SMG upgrade recommended"
    };

    if blocked {
        tracing::error!(
            status = %incompat.status,
            minimum_version = %incompat.minimum_version,
            your_version = %incompat.your_version,
            upgrade_deadline = ?incompat.upgrade_deadline,
            "{level_message}: {}",
            incompat.message
        );
    } else {
        tracing::warn!(
            status = %incompat.status,
            minimum_version = %incompat.minimum_version,
            your_version = %incompat.your_version,
            upgrade_deadline = ?incompat.upgrade_deadline,
            "{level_message}: {}",
            incompat.message
        );
    }

    if env == "docker" {
        if blocked {
            tracing::error!(
                "To upgrade, pull the latest image and restart:\n  docker pull ghcr.io/scryer-media/scryer:latest\n  docker compose up -d"
            );
        } else {
            tracing::warn!(
                "To upgrade, pull the latest image and restart:\n  docker pull ghcr.io/scryer-media/scryer:latest\n  docker compose up -d"
            );
        }
    } else if blocked {
        tracing::error!(
            "Download the latest release from:\n  https://github.com/scryer-media/scryer/releases/latest"
        );
    } else {
        tracing::warn!(
            "Download the latest release from:\n  https://github.com/scryer-media/scryer/releases/latest"
        );
    }
}

fn install_panic_logging_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let payload = panic_info
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| {
                panic_info
                    .payload()
                    .downcast_ref::<String>()
                    .map(String::as_str)
            })
            .unwrap_or("<non-string panic payload>");
        let location = panic_info
            .location()
            .map(|location| {
                format!(
                    "{}:{}:{}",
                    location.file(),
                    location.line(),
                    location.column()
                )
            })
            .unwrap_or_else(|| "<unknown location>".to_string());

        tracing::error!(
            panic_payload = %payload,
            panic_location = %location,
            "scryer process panicked"
        );
        default_hook(panic_info);
    }));
}

#[tokio::main]
async fn main() {
    // Phase 1: Extract startup path flags before subcommand dispatch.
    let mut args: Vec<String> = std::env::args().collect();
    let data_dir_override = match extract_data_dir(&mut args) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };
    let log_file_override = match extract_log_file(&mut args) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };

    // Phase 2: Handle CLI subcommands before any startup work.
    // args[0] is the binary name; subcommand (if any) is args[1].
    if let Some(arg) = args.get(1) {
        match arg.as_str() {
            "init" => {
                init::run_init(args);
                return;
            }
            "--generate-key" => {
                let key = scryer_infrastructure::encryption::EncryptionKey::generate();
                println!("{}", key.to_base64());
                return;
            }
            "--version" | "-V" => {
                println!("scryer {VERSION}");
                return;
            }
            other => {
                eprintln!("unknown argument: {other}");
                eprintln!(
                    "usage: scryer [--data-dir <path>] [--log-file <path>] [init | --generate-key | --version]"
                );
                std::process::exit(1);
            }
        }
    }

    let data_dir = resolve_data_dir(data_dir_override.as_deref());
    let wasmtime_cache_dir = resolve_wasmtime_cache_dir(data_dir_override.as_deref(), &data_dir);

    if let Err(error) = scryer_plugins::initialize_wasm_runtime_at(&wasmtime_cache_dir) {
        eprintln!("failed to initialize required WASM plugin cache: {error}");
        std::process::exit(1);
    }

    load_env_file(Some(&data_dir), false);

    scryer_outbound_http::install_default_rustls_provider();

    let log_ring_buffer = log_buffer::LogRingBuffer::with_default_capacity();
    let log_file_config = resolve_log_file_config(
        log_file_override.as_deref(),
        normalize_env_option(LOG_FILE_ENV).as_deref(),
        &data_dir,
        default_windows_log_file_path(),
    );
    let log_file_writer = match log_file_config.as_ref() {
        Some(config) => match log_buffer::open_log_file(&config.path) {
            Ok(writer) => Some(writer),
            Err(error) if config.explicit => {
                eprintln!(
                    "failed to open Scryer log file at {}: {error}",
                    config.path.display()
                );
                std::process::exit(1);
            }
            Err(error) => {
                eprintln!(
                    "warning: failed to open default Scryer log file at {}: {error}; continuing with console and in-app logs",
                    config.path.display()
                );
                None
            }
        },
        None => None,
    };
    let file_logging_path = log_file_writer
        .as_ref()
        .and_then(|_| log_file_config.as_ref().map(|config| config.path.clone()));

    {
        use tracing_subscriber::layer::SubscriberExt;
        use tracing_subscriber::util::SubscriberInitExt;

        let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

        let stdout_layer = tracing_subscriber::fmt::layer();
        let buffer_layer = tracing_subscriber::fmt::layer()
            .with_writer(log_buffer::LogBufferWriter::new(log_ring_buffer.clone()))
            .with_ansi(false);
        let file_layer = log_file_writer.map(|writer| {
            tracing_subscriber::fmt::layer()
                .with_writer(writer)
                .with_ansi(false)
        });

        tracing_subscriber::registry()
            .with(env_filter)
            .with(stdout_layer)
            .with(buffer_layer)
            .with(file_layer)
            .init();
    }
    install_panic_logging_hook();
    if let Some(path) = file_logging_path.as_ref() {
        tracing::info!(path = %path.display(), "file logging enabled");
    }

    let migration_mode = parse_migration_mode(std::env::var("SCRYER_DB_MIGRATION_MODE").ok());
    let pre_restore_datastore_config =
        match resolve_datastore_config_from_env(data_dir.clone(), migration_mode) {
            Ok(config) => config,
            Err(error) => {
                tracing::error!("{error}");
                std::process::exit(1);
            }
        };

    let finalized_pending_restore =
        match finalize_pending_restore_if_present(&data_dir, &pre_restore_datastore_config).await {
            Ok(finalized) => finalized,
            Err(error) => {
                tracing::error!(error = %error, "failed to finalize pending restore");
                std::process::exit(1);
            }
        };

    load_env_file(Some(&data_dir), true);

    let datastore_config = match resolve_datastore_config_from_env(data_dir.clone(), migration_mode)
    {
        Ok(config) => config,
        Err(error) => {
            tracing::error!(error = %error, "failed to resolve datastore configuration");
            std::process::exit(1);
        }
    };
    tracing::info!(
        engine = datastore_config.engine.as_str(),
        config_source = datastore_config.source.as_str(),
        database_url = datastore_config.safe_database_url(),
        "datastore configuration resolved"
    );

    // Ensure the database directory exists for SQLite file URLs.
    if matches!(datastore_config.engine, DatastoreEngine::Sqlite)
        && let Some(path) = datastore_config.database_url.strip_prefix("sqlite://")
        && let Some(parent) = std::path::Path::new(path).parent()
    {
        let _ = std::fs::create_dir_all(parent);
    }
    let jwt_issuer = std::env::var("SCRYER_JWT_ISSUER").unwrap_or_else(|_| "scryer".to_string());
    let jwt_access_ttl_seconds = parse_env_u64("SCRYER_JWT_ACCESS_TTL_SECONDS", 86_400);
    let bind = std::env::var("SCRYER_BIND").unwrap_or_else(|_| "127.0.0.1:8080".to_string());
    let base_path = BasePath::from_env();

    // Install Prometheus metrics recorder when enabled.
    // The `metrics` crate uses a global facade — once installed, `metrics::counter!()`
    // calls from any crate resolve to this recorder. When not installed, they are no-ops.
    let metrics_handle = if std::env::var("SCRYER_METRICS")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
    {
        let handle = metrics_exporter_prometheus::PrometheusBuilder::new()
            .install_recorder()
            .expect("failed to install prometheus metrics recorder");
        tracing::info!("prometheus metrics enabled at /metrics");
        Some(handle)
    } else {
        None
    };

    tracing::info!(version = VERSION, "starting scryer");

    // ValidateOnly mode: check for pending migrations and exit immediately (no server).
    if matches!(migration_mode, MigrationMode::ValidateOnly) {
        run_validate_only(datastore_config).await;
        return;
    }

    // Read TLS config from env vars (available before DB bootstrap).
    let tls_cert_path = normalize_env_option("SCRYER_TLS_CERT");
    let tls_key_path = normalize_env_option("SCRYER_TLS_KEY");

    // Create the watch channel for bootstrap status communication.
    let (status_tx, status_rx) = watch::channel(BootstrapStatus::Migrating);
    let splash_state = SplashState { status_rx };
    let cors = CorsConfig::from_env();
    let splash_app = build_splash_router(splash_state, cors.clone(), base_path.clone());

    let cors_allow_all = cors.allow_all || cors.allowed_origins.iter().any(|origin| origin == "*");
    if cors_allow_all {
        tracing::warn!("CORS configured with wildcard origin(s)");
    } else if cors.allowed_origins.is_empty() {
        tracing::info!("CORS configured for same-origin requests only");
    } else {
        tracing::info!(origins = ?cors.allowed_origins, "CORS configured with explicit origin list");
    }

    let addr: SocketAddr = bind.parse().expect("invalid bind address");
    let shutdown_token = CancellationToken::new();
    let startup_base_path = base_path.clone();
    let bootstrap_base_path = base_path.clone();

    // Spawn the full application bootstrap in the background.
    let bootstrap_shutdown = shutdown_token.clone();
    let bootstrap_bind = bind.clone();
    let runtime_handle = tokio::runtime::Handle::current();
    std::thread::Builder::new()
        .name("scryer-bootstrap".to_string())
        .spawn(move || {
            runtime_handle.block_on(async move {
                match bootstrap_application(
                    datastore_config,
                    migration_mode,
                    finalized_pending_restore,
                    jwt_issuer,
                    jwt_access_ttl_seconds,
                    bootstrap_bind,
                    cors,
                    bootstrap_shutdown,
                    log_ring_buffer,
                    metrics_handle,
                    data_dir,
                    bootstrap_base_path,
                )
                .await
                {
                    Ok(router) => {
                        let _ = status_tx.send(BootstrapStatus::Ready(router));
                    }
                    Err(error) => {
                        tracing::error!(error = %error, "application bootstrap failed");
                        let _ = status_tx.send(BootstrapStatus::Failed(error.to_string()));
                    }
                }
            });
        })
        .expect("failed to spawn bootstrap thread");

    // Start serving immediately — splash handlers delegate to the full app once ready.
    match (tls_cert_path, tls_key_path) {
        (Some(cert_path), Some(key_path)) => {
            let rustls_config =
                axum_server::tls_rustls::RustlsConfig::from_pem_file(&cert_path, &key_path)
                    .await
                    .unwrap_or_else(|error| {
                        panic!(
                            "failed to load TLS certificates (cert={}, key={}): {error}",
                            cert_path, key_path
                        );
                    });
            let handle = axum_server::Handle::new();
            let shutdown_handle = handle.clone();
            let shutdown_token_tls = shutdown_token.clone();
            tokio::spawn(async move {
                shutdown_signal(shutdown_token_tls).await;
                shutdown_handle.graceful_shutdown(Some(std::time::Duration::from_secs(5)));
            });
            tracing::info!("scryer service listening on {addr} with TLS");
            let url = format!("https://{addr}{}", startup_base_path.ui_root());
            tracing::info!("open the web UI at {url}");
            maybe_open_browser(&url);
            if let Err(error) = axum_server::bind_rustls(addr, rustls_config)
                .handle(handle)
                .serve(splash_app.into_make_service_with_connect_info::<SocketAddr>())
                .await
            {
                tracing::error!(error = %error, "TLS server failed");
                std::process::exit(1);
            }
        }
        (Some(_), None) | (None, Some(_)) => {
            panic!("both SCRYER_TLS_CERT and SCRYER_TLS_KEY must be set for TLS, or neither");
        }
        (None, None) => {
            let listener = TcpListener::bind(addr)
                .await
                .expect("failed to bind address");
            tracing::info!(
                "scryer service listening on {}",
                listener.local_addr().expect("bound addr")
            );
            let url = format!("http://{addr}{}", startup_base_path.ui_root());
            tracing::info!("open the web UI at {url}");
            maybe_open_browser(&url);
            if let Err(error) = axum::serve(
                listener,
                splash_app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(shutdown_signal(shutdown_token.clone()))
            .await
            {
                tracing::error!(error = %error, "server failed");
                std::process::exit(1);
            }
        }
    }
}

/// Runs the full application bootstrap: DB init, migrations, service construction, and router
/// building. Returns the fully-constructed Axum router or an error.
#[expect(clippy::too_many_arguments)]
async fn bootstrap_application(
    datastore_config: DatastoreConfig,
    _migration_mode: MigrationMode,
    finalized_pending_restore: bool,
    jwt_issuer: String,
    jwt_access_ttl_seconds: u64,
    _bind: String,
    cors: CorsConfig,
    shutdown_token: CancellationToken,
    log_ring_buffer: log_buffer::LogRingBuffer,
    metrics_handle: Option<metrics_exporter_prometheus::PrometheusHandle>,
    data_dir: PathBuf,
    base_path: BasePath,
) -> Result<Router, Box<dyn std::error::Error + Send + Sync>> {
    let bootstrap_start = std::time::Instant::now();

    let t = std::time::Instant::now();
    let backup_datastore_config = datastore_config.clone();
    let datastore = DatastoreAssembly::connect(datastore_config)
        .await
        .map_err(|e| format!("failed to initialize datastore services: {e}"))?;
    let bootstrap_settings_store = datastore.settings_store();
    let bootstrap_quality_profile_store = datastore.quality_profile_store();
    let datastore_info = bootstrap_settings_store
        .datastore_info()
        .await
        .map_err(|e| format!("failed to read datastore info: {e}"))?;
    tracing::info!(
        elapsed_ms = %t.elapsed().as_millis(),
        engine = datastore_info.engine,
        migration_key = ?datastore_info.current_migration_key,
        "database initialized"
    );

    let t = std::time::Instant::now();
    seed_service_setting_definitions(bootstrap_settings_store.clone())
        .await
        .map_err(|e| format!("failed to seed service setting definitions: {e}"))?;
    tracing::info!(elapsed_ms = %t.elapsed().as_millis(), "setting definitions seeded");

    // Bootstrap encryption master key (env > keystore > legacy DB migration > auto-generate).
    let t = std::time::Instant::now();
    let encryption_bootstrap = datastore
        .bootstrap_encryption()
        .await
        .map_err(|e| {
            let credential_context = desktop_credential_namespace(&data_dir).map_or_else(
                String::new,
                |namespace| format!("; Windows Credential Manager namespace: {namespace}"),
            );
            format!(
                "failed to bootstrap datastore encryption for data directory {}{credential_context}: {e}",
                data_dir.display()
            )
        })?;
    if encryption_bootstrap.migrated_indexer_configs > 0 {
        tracing::info!(
            migrated = encryption_bootstrap.migrated_indexer_configs,
            "migrated legacy indexer base/api fields into config_json"
        );
    }
    if encryption_bootstrap.encrypted_release_attempt_source_passwords > 0
        || encryption_bootstrap.encrypted_pending_release_source_passwords > 0
    {
        tracing::info!(
            release_attempts = encryption_bootstrap.encrypted_release_attempt_source_passwords,
            pending_releases = encryption_bootstrap.encrypted_pending_release_source_passwords,
            "encrypted legacy release source passwords"
        );
    }
    tracing::info!(elapsed_ms = %t.elapsed().as_millis(), "encryption bootstrapped");

    // Detect version upgrades by comparing with last-run version stored in DB
    let version_lifecycle = check_version_upgrade(bootstrap_settings_store.clone()).await;
    record_post_upgrade_auto_backup_pending_if_needed(
        bootstrap_settings_store.clone(),
        &version_lifecycle,
    )
    .await;
    startup_migrations::_0001_legacy_history_retention_forever_override::clear_legacy_history_retention_forever_override(
        bootstrap_settings_store.clone(),
    )
    .await;

    let t = std::time::Instant::now();
    if let Err(error) =
        seed_service_settings_from_environment(bootstrap_settings_store.clone()).await
    {
        tracing::warn!(
            error = %error,
            "failed to persist optional settings from environment"
        );
    }
    if let Err(error) =
        migrate_legacy_download_client_routing_settings(bootstrap_settings_store.clone()).await
    {
        tracing::warn!(
            error = %error,
            "failed to migrate legacy download client routing settings during bootstrap"
        );
    }

    if let Err(error) =
        migrate_legacy_download_client_default_category_settings(bootstrap_settings_store.clone())
            .await
    {
        tracing::warn!(
            error = %error,
            "failed to migrate legacy download client default category settings during bootstrap"
        );
    }
    tracing::info!(elapsed_ms = %t.elapsed().as_millis(), "environment settings synced");

    let t = std::time::Instant::now();
    if let Err(error) = normalize_media_path_setting(
        bootstrap_settings_store.clone(),
        MOVIES_PATH_KEY.to_string(),
    )
    .await
    {
        tracing::warn!(
            error = %error,
            "failed to normalize media movies.path setting during bootstrap"
        );
    }

    if let Err(error) = normalize_media_path_setting(
        bootstrap_settings_store.clone(),
        SERIES_PATH_KEY.to_string(),
    )
    .await
    {
        tracing::warn!(
            error = %error,
            "failed to normalize media series.path setting during bootstrap"
        );
    }

    // Construct the facet registry early so scope IDs are available for settings bootstrap.
    let mut registry = FacetRegistry::new();
    registry.register(Arc::new(MovieFacetHandler));
    registry.register(Arc::new(SeriesFacetHandler::new(
        scryer_domain::MediaFacet::Series,
    )));
    registry.register(Arc::new(SeriesFacetHandler::new(
        scryer_domain::MediaFacet::Anime,
    )));
    let facet_registry = Arc::new(registry);

    if let Err(error) = normalize_quality_profile_settings(
        bootstrap_settings_store.clone(),
        bootstrap_quality_profile_store.clone(),
        facet_registry
            .facet_ids()
            .into_iter()
            .map(str::to_string)
            .collect(),
    )
    .await
    {
        tracing::warn!(
            error = %error,
            "failed to normalize quality profile settings during bootstrap"
        );
    }
    tracing::info!(elapsed_ms = %t.elapsed().as_millis(), "settings normalized");

    tracing::info!(elapsed_ms = %bootstrap_start.elapsed().as_millis(), "bootstrap complete");

    let indexer_configs = datastore.indexer_configs();
    let download_client_configs = datastore.download_client_configs();
    let subtitle_provider_configs = datastore.subtitle_provider_configs();
    let settings_for_router = datastore.settings();
    let plugin_http_runtime = Arc::new(scryer_plugins::shared_plugin_http_runtime());
    let plugin_http_ca_bundle_pem = match settings_for_router
        .get_setting_json(SETTINGS_SCOPE_SYSTEM, PLUGIN_HTTP_CA_BUNDLE_PEM_KEY, None)
        .await
        .map_err(|error| format!("failed to load plugin HTTP trusted certificates: {error}"))?
    {
        Some(value_json) => match serde_json::from_str::<String>(&value_json) {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!(
                    key = PLUGIN_HTTP_CA_BUNDLE_PEM_KEY,
                    error = %error,
                    "failed to decode stored plugin HTTP trusted certificate bundle; ignoring"
                );
                String::new()
            }
        },
        None => String::new(),
    };
    plugin_http_runtime
        .set_plugin_http_ca_bundle_pem(plugin_http_ca_bundle_pem)
        .map_err(|error| {
            format!("failed to initialize plugin HTTP trusted certificates: {error}")
        })?;
    let customization_store = datastore.customization_store();
    let staged_nzb_store = Arc::new(
        FileSystemStagedNzbStore::new_with_startup_purge(datastore.staged_nzb_path(), true)
            .await
            .map_err(|e| format!("failed to initialize staged nzb store: {e}"))?,
    );
    let staged_nzb_pipeline_limit = Arc::new(tokio::sync::Semaphore::new(4));
    bootstrap_plugin_installations(&customization_store, finalized_pending_restore)
        .await
        .map_err(|e| format!("failed to bootstrap plugin installations: {e}"))?;
    let (runtime_plugins, disabled_builtin_plugins) =
        load_runtime_plugin_state(&customization_store)
            .await
            .map_err(|e| format!("failed to load runtime plugin state: {e}"))?;
    scryer_plugins::schedule_plugin_rehydration(&runtime_plugins, &disabled_builtin_plugins);
    let indexer_runtime_plugins = runtime_plugins
        .iter()
        .filter(|plugin| plugin_type_belongs_to_indexer_family(plugin.descriptor.plugin_type()))
        .cloned()
        .collect::<Vec<_>>();
    let download_client_runtime_plugins = runtime_plugins
        .iter()
        .filter(|plugin| plugin.descriptor.plugin_type() == "download_client")
        .cloned()
        .collect::<Vec<_>>();
    let subtitle_runtime_plugins = runtime_plugins
        .iter()
        .filter(|plugin| plugin.descriptor.plugin_type() == "subtitle_provider")
        .cloned()
        .collect::<Vec<_>>();
    let archive_extractor_runtime_plugins = runtime_plugins
        .iter()
        .filter(|plugin| plugin.descriptor.plugin_type() == "archive_extractor")
        .cloned()
        .collect::<Vec<_>>();
    let notification_runtime_plugins = runtime_plugins
        .iter()
        .filter(|plugin| plugin.descriptor.plugin_type() == "notification")
        .cloned()
        .collect::<Vec<_>>();
    let download_client_plugin_provider: Arc<dyn DownloadClientPluginProvider> =
        Arc::new(scryer_plugins::DynamicDownloadClientPluginProvider::new(
            scryer_plugins::build_download_client_plugin_provider_from_runtime_plugins(
                &download_client_runtime_plugins,
                &disabled_builtin_plugins,
            ),
        ));
    let download_client = Arc::new(
        PrioritizedDownloadClientRouter::new(
            download_client_configs.clone(),
            settings_for_router.clone(),
            staged_nzb_store.clone(),
            staged_nzb_pipeline_limit.clone(),
            Some(download_client_plugin_provider.clone()),
        )
        .with_indexer_config_repositories(
            indexer_configs.clone(),
            datastore.indexer_proxy_configs(),
        ),
    );
    let indexer_stats = datastore.indexer_stats_tracker();
    let indexer_learning = datastore.indexer_search_learning_repository();
    let upstream_scheduler = datastore
        .upstream_scheduler()
        .await
        .map_err(|e| format!("failed to initialize upstream scheduler: {e}"))?;

    let dynamic_provider = Arc::new(scryer_plugins::DynamicPluginProvider::new(
        scryer_plugins::build_indexer_plugin_provider_from_runtime_plugins(
            &indexer_runtime_plugins,
            &disabled_builtin_plugins,
        ),
    ));
    let plugin_provider: Arc<dyn IndexerPluginProvider> = Arc::new(
        scryer_infrastructure::NativeProwlarrIndexerProvider::new(dynamic_provider),
    );
    let subtitle_plugin_provider: Arc<dyn SubtitlePluginProvider> =
        Arc::new(scryer_plugins::DynamicSubtitlePluginProvider::new(
            scryer_plugins::build_subtitle_plugin_provider_from_runtime_plugins(
                &subtitle_runtime_plugins,
                &disabled_builtin_plugins,
            ),
        ));
    let archive_extractor_plugin_provider: Arc<dyn ArchiveExtractorPluginProvider> =
        Arc::new(scryer_plugins::DynamicArchiveExtractorPluginProvider::new(
            scryer_plugins::build_archive_extractor_plugin_provider_from_runtime_plugins(
                &archive_extractor_runtime_plugins,
                &disabled_builtin_plugins,
            ),
        ));

    let indexer_client = MultiIndexerSearchClient::new(
        indexer_configs.clone(),
        indexer_stats.clone(),
        plugin_provider.clone(),
    )
    .with_indexer_proxy_config_repository(datastore.indexer_proxy_configs())
    .with_search_learning_repository(indexer_learning)
    .with_upstream_scheduler(upstream_scheduler.clone());

    let indexer_client = Arc::new(indexer_client);
    let title_images_for_route: Arc<dyn TitleImageRepository> = datastore.title_images();
    let image_proxy_runtime = Arc::new(ImageProxyRuntime::new(
        datastore.image_proxy(),
        title_images_for_route.clone(),
        &data_dir,
    ));
    let metadata_gateway_url = std::env::var("SCRYER_METADATA_GATEWAY_GRAPHQL_URL")
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| SMG_GRAPHQL_URL.map(String::from))
        .unwrap_or_else(|| "http://127.0.0.1:8090/graphql".to_string());
    let smg_registration_secret = SMG_REGISTRATION_SECRET
        .map(String::from)
        .or_else(|| std::env::var("SCRYER_SMG_REGISTRATION_SECRET").ok())
        .filter(|s| !s.is_empty());

    // Prefer an explicit managed JWT signing secret so restored bundles can
    // preserve instance identity across environments. Fall back to the SMG
    // registration secret or a persistent local secret for fresh installs.
    let jwt_signing_salt = match normalize_env_option("SCRYER_JWT_SIGNING_SECRET") {
        Some(secret) => secret,
        None => match &smg_registration_secret {
            Some(secret) => secret.clone(),
            None => {
                tracing::warn!(
                    "no SMG registration secret available; using persistent local JWT salt"
                );
                load_or_create_persistent_jwt_signing_salt(&data_dir)?
            }
        },
    };

    let metadata_gateway = Arc::new(datastore.metadata_gateway_client(
        metadata_gateway_url,
        SmgEnrollmentConfig {
            registration_secret: smg_registration_secret,
        },
    ));
    let library_scanner = Arc::new(FileSystemLibraryScanner::new());
    let library_renamer = Arc::new(FileSystemLibraryRenamer::new());

    let (tracked_download_tx, tracked_download_rx) = tokio::sync::mpsc::channel(64);
    let (tracked_download_snapshot_tx, tracked_download_snapshot_rx) =
        tokio::sync::mpsc::channel(64);
    let tracked_download_snapshot_ingest =
        TrackedDownloadSnapshotIngestHandle::new(tracked_download_snapshot_tx);

    // Warm up SMG enrollment so the mTLS client is ready before the first real
    // metadata query, and check for version incompatibility.
    let metadata_gateway_for_warmup = metadata_gateway.clone();
    tokio::spawn(async move {
        if let Some(incompat) = metadata_gateway_for_warmup.warm_enrollment().await {
            log_smg_version_incompatibility(&incompat);
        }
        if !metadata_gateway_for_warmup.compatibility_polling_enabled() {
            return;
        }

        let phase = loop {
            match metadata_gateway_for_warmup
                .version_compatibility_poll_phase()
                .await
            {
                Ok(phase) => break phase,
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        "failed to derive SMG version compatibility poll phase"
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(15 * 60)).await;
                }
            }
        };

        let mut minimum_delay = MetadataGatewayClient::version_compatibility_startup_guard();
        loop {
            let delay =
                MetadataGatewayClient::next_version_compatibility_poll_delay(phase, minimum_delay);
            minimum_delay = std::time::Duration::from_secs(0);
            tokio::time::sleep(delay).await;

            match metadata_gateway_for_warmup
                .refresh_version_compatibility(true)
                .await
            {
                Ok(Some(incompat)) => log_smg_version_incompatibility(&incompat),
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(error = %error, "SMG version compatibility refresh failed");
                }
            }
        }
    });

    let notif_provider = scryer_plugins::DynamicNotificationPluginProvider::new(
        scryer_plugins::build_notification_plugin_provider_from_runtime_plugins(
            &notification_runtime_plugins,
            &disabled_builtin_plugins,
        ),
    );
    let services = datastore
        .app_services_builder(indexer_client, download_client)
        .with_runtime_environment(
            compiled_binary_lane(),
            data_dir.clone(),
            scryer_plugins::detect_supported_plugin_required_features(),
        )
        .with_smg_registration_secret(
            SMG_REGISTRATION_SECRET
                .map(String::from)
                .or_else(|| std::env::var("SCRYER_SMG_REGISTRATION_SECRET").ok())
                .filter(|value| !value.is_empty()),
        )
        .with_smg_gateway_url(Some(
            std::env::var("SCRYER_METADATA_GATEWAY_GRAPHQL_URL")
                .ok()
                .filter(|value| !value.is_empty())
                .or_else(|| SMG_GRAPHQL_URL.map(String::from))
                .unwrap_or_else(|| "http://127.0.0.1:8090/graphql".to_string()),
        ))
        .with_metadata_gateway(metadata_gateway)
        .with_image_proxy_cache_control(image_proxy_runtime.clone())
        .with_library_scanner(library_scanner)
        .with_library_renamer(library_renamer)
        .with_file_importer(Arc::new(scryer_infrastructure::FsFileImporter::new()))
        .with_staged_nzb_store(staged_nzb_store)
        .with_staged_nzb_pipeline_limit(staged_nzb_pipeline_limit)
        .with_indexer_stats(indexer_stats)
        .with_upstream_scheduler(upstream_scheduler.clone())
        .with_indexer_caps_refresher(Arc::new(
            scryer_infrastructure::indexer_caps::DirectNabCapsSnapshotRefresher::new()
                .with_upstream_scheduler(upstream_scheduler.clone()),
        ))
        .with_plugin_http_trust_runtime(plugin_http_runtime)
        .with_plugin_provider(plugin_provider)
        .with_builtin_download_client_connection_tester(Arc::new(
            BuiltinDownloadClientConnectionTester,
        ))
        .with_download_client_plugin_provider(download_client_plugin_provider.clone())
        .with_subtitle_provider_configs(subtitle_provider_configs)
        .with_subtitle_plugin_provider(subtitle_plugin_provider)
        .with_archive_extractor_plugin_provider(archive_extractor_plugin_provider)
        .with_notification_provider(Arc::new(notif_provider))
        .with_plugin_descriptor_loader(Arc::new(scryer_plugins::WasmPluginDescriptorLoader))
        .with_tracked_download_handle(TrackedDownloadHandle::new(tracked_download_tx))
        .build();

    let webauthn = build_webauthn_runtime();
    let webauthn_configured = webauthn.is_some();
    let app_use_case = AppUseCase::new_with_webauthn(
        services,
        scryer_application::JwtAuthConfig {
            issuer: jwt_issuer,
            access_ttl_seconds: jwt_access_ttl_seconds as usize,
            jwt_signing_salt,
        },
        facet_registry,
        webauthn,
    );
    if let Err(error) = app_use_case.sync_image_cache_runtime_limit().await {
        tracing::warn!(error = %error, "failed to apply configured image cache limit");
    }
    if let Err(error) = image_proxy_runtime.prune().await {
        tracing::warn!(error = %error, "failed to run startup image cache maintenance");
    }
    {
        let source_repository = app_use_case.image_proxy_repository();
        let source_flush_shutdown = shutdown_token.child_token();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            interval.tick().await;
            loop {
                tokio::select! {
                    _ = source_flush_shutdown.cancelled() => {
                        if let Err(error) = source_repository.flush_image_proxy_sources().await {
                            tracing::warn!(error = %error, "final image proxy source flush failed");
                        }
                        break;
                    }
                    _ = interval.tick() => {
                        if let Err(error) = source_repository.flush_image_proxy_sources().await {
                            tracing::warn!(error = %error, "periodic image proxy source flush failed");
                        }
                    }
                }
            }
        });
    }
    {
        let runtime = image_proxy_runtime.clone();
        let maintenance_shutdown = shutdown_token.child_token();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(6 * 60 * 60));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            interval.tick().await;
            loop {
                tokio::select! {
                    _ = maintenance_shutdown.cancelled() => break,
                    _ = interval.tick() => {
                        if let Err(error) = runtime.prune().await {
                            tracing::warn!(error = %error, "periodic image cache maintenance failed");
                        }
                    }
                }
            }
        });
    }
    tokio::spawn(flush_upstream_scheduler_after_shutdown(
        shutdown_token.child_token(),
        {
            let app_use_case = app_use_case.clone();
            move || async move { app_use_case.flush_upstream_scheduler().await }
        },
    ));
    tracing::info!(
        build_lane = %app_use_case.runtime_build_lane(),
        build_class = %app_use_case.runtime_build_class(),
        os = std::env::consts::OS,
        arch = std::env::consts::ARCH,
        "initialized runtime build identity"
    );
    app_use_case.warm_runtime_performance();
    if finalized_pending_restore {
        tracing::info!("recovering restored plugins from remote catalogs");
        if let Err(error) = app_use_case
            .recover_restored_plugins_after_backup_restore()
            .await
        {
            tracing::error!(error = %error, "failed to recover restored plugins");
            std::process::exit(1);
        }
    }
    let previous_version = match &version_lifecycle {
        VersionLifecycle::Upgraded { previous } => Some(previous.as_str()),
        VersionLifecycle::FirstRun | VersionLifecycle::Unchanged => None,
    };
    startup_migrations::_0002_enhanced_subsync_plugin_016::migrate_enhanced_subsync_plugin_for_016_upgrade(
        &app_use_case,
        bootstrap_settings_store.clone(),
        previous_version,
        VERSION,
    )
    .await;
    startup_migrations::_0003_title_image_artwork_url_refresh::refresh_title_image_artwork_urls_for_upgrade(
        &app_use_case,
        bootstrap_settings_store.clone(),
        previous_version,
        VERSION,
    )
    .await;
    startup_migrations::_0004_auto_backup_missing_key_disable::disable_auto_backups_without_key(
        bootstrap_settings_store.clone(),
    )
    .await;
    startup_migrations::_0005_title_metadata_rehydration_017::rehydrate_title_metadata_for_017_upgrade(
        &app_use_case,
        bootstrap_settings_store.clone(),
        VERSION,
    )
    .await;
    spawn_post_upgrade_auto_backup_if_pending(
        app_use_case.clone(),
        bootstrap_settings_store.clone(),
    )
    .await;
    if let Err(error) = app_use_case
        .repair_legacy_jellyfin_external_account_invites()
        .await
    {
        tracing::warn!(
            error = %error,
            "failed to repair legacy Jellyfin external account invites"
        );
    }

    app_use_case.connect_library_scan_tracker().await;
    spawn_sigstore_trust_root_prime_task(app_use_case.clone());
    spawn_plugin_catalog_refresh_task(app_use_case.clone());

    // A persisted running job run whose worker died in a previous process is
    // unfinishable; fail those rows before any poller can wait on them forever.
    if let Err(e) = app_use_case.reconcile_interrupted_job_runs().await {
        tracing::warn!(error = %e, "failed to reconcile interrupted job runs on startup");
    }

    if let Err(e) = app_use_case.reconcile_default_library_roots().await {
        tracing::warn!(error = %e, "failed to reconcile default library roots on startup");
    }

    if let Err(e) = app_use_case.migrate_legacy_persona_preferences().await {
        tracing::warn!(error = %e, "failed to migrate legacy persona preferences on startup");
    }
    if let Err(e) = app_use_case
        .migrate_canonical_audio_persona_settings()
        .await
    {
        tracing::warn!(error = %e, "failed to migrate canonical audio/persona settings on startup");
    }
    if let Err(e) = app_use_case
        .reconcile_and_activate_managed_trash_rule_packs()
        .await
    {
        tracing::warn!(
            error = %e,
            "failed to reconcile managed TRaSH rule packs; managed packs stay excluded from the active rules engine until the next successful reconciliation"
        );
    }
    if let Err(e) = app_use_case
        .migrate_legacy_opensubtitles_provider_config()
        .await
    {
        tracing::warn!(
            error = %e,
            "failed to migrate legacy opensubtitles settings into subtitle provider configs on startup"
        );
    }
    if let Err(e) = app_use_case.reconcile_indexer_configs().await {
        tracing::warn!(error = %e, "failed to reconcile indexer configs on startup");
    }
    if let Err(e) = app_use_case
        .ensure_indexer_routing_entries_for_existing_indexers()
        .await
    {
        tracing::warn!(error = %e, "failed to ensure indexer routing on startup");
    }
    if let Err(e) = app_use_case.normalize_routing_settings().await {
        tracing::warn!(error = %e, "failed to normalize routing settings on startup");
    }
    if let Err(e) = app_use_case
        .refresh_owned_download_client_categories()
        .await
    {
        tracing::warn!(
            error = %e,
            "failed to build download-client category ownership snapshot on startup; foreign-download visibility will fail open"
        );
    }
    let restore_restart_controller = SelfRestartController::new(Duration::from_millis(250))
        .map_err(|error| format!("failed to prepare restore restart controller: {error}"))?;

    let auth_mode = resolve_auth_mode_from_env()?;
    app_use_case.set_recovery_admin_login_enabled(auth_mode.recovery_active());
    if auth_mode.recovery_active() {
        let recovery_password = normalize_env_option(RECOVERY_ADMIN_PASSWORD_ENV)
            .ok_or_else(|| format!("{RECOVERY_ADMIN_PASSWORD_ENV} was set but empty"))?;
        tracing::warn!(
            env = RECOVERY_ADMIN_PASSWORD_ENV,
            "instance recovery mode is active; form login is enabled for this boot and the reserved recovery-admin account will be created or repaired"
        );
        app_use_case
            .recover_reserved_admin_access(&recovery_password)
            .await
            .map_err(|error| format!("failed to recover recovery-admin access: {error}"))?;
        tracing::warn!(
            env = RECOVERY_ADMIN_PASSWORD_ENV,
            "instance recovery mode created or repaired recovery-admin; remove this environment variable and restart after regaining control"
        );
    }

    let saved_security_settings = app_use_case
        .security_settings()
        .await
        .map_err(|error| format!("failed to load security settings: {error}"))?;
    let effective_form_login_enabled =
        auth_mode.effective_form_login_enabled(saved_security_settings.form_login_enabled);
    let effective_skip_login_for_local_ips = auth_mode
        .effective_skip_login_for_local_ips(saved_security_settings.skip_login_for_local_ips);
    let auth_runtime = AuthRuntimeStateHandle::new(AuthRuntimeStateSnapshot {
        form_login_enabled: saved_security_settings.form_login_enabled,
        skip_login_for_local_ips: effective_skip_login_for_local_ips,
        effective_form_login_enabled,
        webauthn_configured,
        passkey_enabled: webauthn_configured && effective_form_login_enabled,
        env_override_active: auth_mode.env_override_active(),
        env_override_description: auth_mode.env_override_description.clone(),
        epoch: 0,
    });
    let log_buf_snapshot = log_ring_buffer.clone();
    let log_buf_subscribe = log_ring_buffer.clone();
    let schema = build_schema_with_log_buffer_and_restore(
        app_use_case.clone(),
        auth_runtime.clone(),
        Some(LogBuffer::new(
            move |limit| log_buf_snapshot.snapshot(limit),
            move || log_buf_subscribe.subscribe(),
        )),
        Some(RestoreContext {
            data_dir: data_dir.clone(),
            datastore_config: restore_datastore_config(&backup_datastore_config),
            datastore: restore_datastore_handle(),
            restart: restore_restart_controller.handle(),
        }),
    );
    let authless_access_allowlist_raw =
        normalize_env_option(UNAUTHENTICATED_PUBLIC_ACCESS_ALLOWLIST_ENV).unwrap_or_default();
    let authless_access_allowlist_env_configured =
        comma_separated_env_has_entries(&authless_access_allowlist_raw);
    let authless_access_allowlist = AuthlessAccessAllowlist::parse(&authless_access_allowlist_raw);
    validate_unauthenticated_public_access_allowlist_config(
        authless_access_allowlist_env_configured,
        authless_access_allowlist.is_configured(),
    )
    .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { error.into() })?;
    if auth_mode.used_legacy_dev_auto_login {
        tracing::warn!(
            "SCRYER_DEV_AUTO_LOGIN is deprecated; use SCRYER_AUTH_ENABLED=false instead"
        );
    }
    if auth_runtime.snapshot().effective_form_login_enabled {
        if auth_mode.recovery_active() {
            tracing::warn!(
                env = RECOVERY_ADMIN_PASSWORD_ENV,
                "running with authentication enabled in recovery mode; recovery-admin credentials are available for this boot"
            );
        } else {
            tracing::info!("running with authentication enabled");
            ensure_admin_password_configured(&app_use_case).await?;
        }
    } else {
        app_use_case
            .find_or_create_default_user()
            .await
            .map_err(|error| {
                format!("failed to ensure default admin for disabled-auth mode: {error}")
            })?;
        if auth_mode.allow_unauthenticated_public_access
            || authless_access_allowlist.is_configured()
        {
            if authless_access_allowlist.is_configured() {
                if auth_mode.allow_unauthenticated_public_access {
                    tracing::warn!(
                        env = ALLOW_UNAUTHENTICATED_PUBLIC_ACCESS_ENV,
                        allowlist_env = UNAUTHENTICATED_PUBLIC_ACCESS_ALLOWLIST_ENV,
                        "public unauthenticated access is explicitly enabled and narrowed by allowlist; matching clients will act as admin"
                    );
                } else {
                    tracing::warn!(
                        allowlist_env = UNAUTHENTICATED_PUBLIC_ACCESS_ALLOWLIST_ENV,
                        "public unauthenticated access is enabled by allowlist; matching clients will act as admin"
                    );
                }
            } else {
                tracing::warn!(
                    env = ALLOW_UNAUTHENTICATED_PUBLIC_ACCESS_ENV,
                    "public unauthenticated access is explicitly enabled; all reachable clients will act as admin"
                );
            }
        } else {
            tracing::warn!(
                env = ALLOW_UNAUTHENTICATED_PUBLIC_ACCESS_ENV,
                "running with authentication disabled; private/local clients act as admin and public clients are rejected unless explicitly allowed"
            );
        }
    }
    // Bridge coverage is decided at runtime, not at boot. Resolving the
    // weaver subscription bridge once at startup meant a weaver client added
    // or promoted after boot (every fresh install) never got realtime
    // coverage until the next restart — it silently ran on interval polling,
    // which cannot see weaver's sub-second job lifetimes in the active-only
    // queue facade. The supervisor starts/stops the bridge as weaver
    // becomes/stops being the primary enabled client, and the poller reads
    // this shared handle every tick to stand down (or resume) in lockstep.
    let bridged_client_types = BridgedClientTypesHandle::new();
    let poller_options = DownloadQueuePollerOptions {
        bridged_client_types: bridged_client_types.clone(),
        ..DownloadQueuePollerOptions::default()
    };

    // Run the generic download queue poller for polling-based clients. Weaver
    // is excluded while its self-contained subscription bridge is active.
    tokio::spawn(start_download_queue_poller_with_options(
        app_use_case.clone(),
        shutdown_token.child_token(),
        tracked_download_rx,
        tracked_download_snapshot_rx,
        poller_options,
    ));
    tokio::spawn(start_weaver_bridge_supervisor(
        shutdown_token.child_token(),
        app_use_case.clone(),
        tracked_download_snapshot_ingest,
        bridged_client_types,
    ));
    tokio::spawn(start_background_acquisition_poller(
        app_use_case.clone(),
        shutdown_token.child_token(),
    ));
    tokio::spawn(start_background_library_refresh_loop(
        app_use_case.clone(),
        shutdown_token.child_token(),
    ));
    tokio::spawn(start_background_title_hydration_loop(
        app_use_case.clone(),
        shutdown_token.child_token(),
    ));
    tokio::spawn(start_background_title_image_loop(
        app_use_case.clone(),
        shutdown_token.child_token(),
    ));
    tokio::spawn(start_notification_dispatcher(
        app_use_case.clone(),
        shutdown_token.child_token(),
    ));
    tokio::spawn(start_background_subtitle_poller(
        app_use_case.clone(),
        shutdown_token.child_token(),
    ));
    tokio::spawn(start_background_auto_backup_scheduler(
        app_use_case.clone(),
        shutdown_token.child_token(),
    ));
    tokio::spawn(start_background_manual_import_poller(
        app_use_case.clone(),
        shutdown_token.child_token(),
    ));
    tokio::spawn(start_background_download_delete_poller(
        app_use_case.clone(),
        shutdown_token.child_token(),
    ));
    app_use_case.wake_title_image_loops();

    let rate_limiter = ScryerRateLimiter::from_env();
    let authless_access_policy = AuthlessAccessPolicy {
        allow_unauthenticated_public_access: auth_mode.allow_unauthenticated_public_access,
    };
    let authless_web_client_proof = AuthlessWebClientProofState::new();
    let auth_state = AuthState {
        app: app_use_case.clone(),
        schema: schema.clone(),
        auth_runtime: auth_runtime.clone(),
        rate_limiter: rate_limiter.clone(),
        ws_origin_policy: WebSocketOriginPolicy::from_env(&cors),
        authless_web_client_proof: authless_web_client_proof.clone(),
    };
    let authless_access_guard_state = AuthlessAccessGuardState {
        auth_runtime: auth_runtime.clone(),
        policy: authless_access_policy,
        allowlist: authless_access_allowlist.clone(),
    };
    let authless_web_client_proof_route_state = AuthlessWebClientProofRouteState {
        auth_runtime: auth_runtime.clone(),
        policy: authless_access_policy,
        proof: authless_web_client_proof.clone(),
        allowlist: authless_access_allowlist,
    };

    let cors_for_layer = cors.clone();
    let backup_route_state = BackupRouteState {
        app: app_use_case.clone(),
    };
    let oauth_route_state = OAuthRouteState {
        app: app_use_case.clone(),
        base_path: base_path.clone(),
        auth_runtime: auth_runtime.clone(),
    };
    let ws_auth_state = auth_state.clone();

    // WebSocket route must be outside CompressionLayer — compression wraps the
    // 101 upgrade response body and injects Content-Encoding, breaking the
    // WebSocket handshake.
    let ws_router = Router::new().route(
        "/graphql/ws",
        get(graphql_ws_handler).with_state(ws_auth_state),
    );

    let mut compressed_router = Router::new()
        .route("/health", get(health_handler))
        .route(
            "/authless-client",
            get(authless_web_client_proof_handler)
                .with_state(authless_web_client_proof_route_state),
        )
        .merge(oauth_router(oauth_route_state))
        .route("/oauth/authorize", get(ui_fallback))
        .route(
            "/graphql",
            post(graphql_handler).with_state(auth_state.clone()),
        )
        .route(
            "/images/media/{token}/{variant}",
            get(image_proxy_handler).with_state(image_proxy_runtime),
        )
        .route(
            "/images/titles/{title_id}/{kind}/{variant}",
            get(title_image_handler).with_state(title_images_for_route),
        )
        .route(
            "/backups/{filename}/download",
            get(download_backup_handler).with_state(backup_route_state),
        )
        .fallback(get(ui_fallback))
        .layer(axum::middleware::from_fn_with_state(
            auth_state.clone(),
            rate_limit_http_api,
        ))
        .layer(CompressionLayer::new().zstd(true).br(true).gzip(true));

    if let Some(ref handle) = metrics_handle {
        let h = handle.clone();
        compressed_router = compressed_router.route(
            "/metrics",
            get(move || {
                let h = h.clone();
                async move { h.render() }
            }),
        );
    }

    let app = ws_router
        .merge(compressed_router)
        .layer(axum::middleware::from_fn_with_state(
            authless_access_guard_state,
            enforce_authless_access_guard,
        ))
        .layer(axum::middleware::from_fn(move |request, next| {
            cors_handler(request, next, cors_for_layer.clone())
        }));

    match ui_asset_mode() {
        UiAssetMode::Filesystem(dist_dir) => {
            if Path::new(dist_dir).exists() {
                tracing::info!(path = %dist_dir.display(), "serving web UI from filesystem path");
            } else {
                tracing::warn!(
                    path = %dist_dir.display(),
                    "configured web UI path does not exist; serving fallback root notice"
                );
            }
        }
        UiAssetMode::Embedded => {
            tracing::info!("serving web UI from embedded assets bundled into this binary");
        }
        UiAssetMode::Fallback => {
            tracing::warn!("no web UI assets found; serving fallback root notice");
        }
    }

    Ok(app)
}

async fn image_proxy_handler(
    State(runtime): State<Arc<ImageProxyRuntime>>,
    headers: HeaderMap,
    AxumPath((token, variant)): AxumPath<(String, String)>,
) -> Response {
    let blob = runtime.resolve(&token, &variant).await;
    let bare_etag = blob.etag.trim_matches('"');
    let cache_control = if blob.fallback {
        HeaderValue::from_static("public, max-age=60")
    } else {
        HeaderValue::from_static("public, max-age=86400, must-revalidate")
    };
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| if_none_match_matches(value, &blob.etag, bare_etag))
    {
        let mut response = StatusCode::NOT_MODIFIED.into_response();
        let headers = response.headers_mut();
        if let Ok(value) = HeaderValue::from_str(&blob.etag) {
            headers.insert(header::ETAG, value);
        }
        headers.insert(header::CACHE_CONTROL, cache_control);
        return response;
    }

    let body_len = blob.bytes.len();
    let mut response = blob.bytes.into_response();
    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(&blob.content_type) {
        headers.insert(header::CONTENT_TYPE, value);
    }
    if let Ok(value) = HeaderValue::from_str(&body_len.to_string()) {
        headers.insert(header::CONTENT_LENGTH, value);
    }
    if let Ok(value) = HeaderValue::from_str(&blob.etag) {
        headers.insert(header::ETAG, value);
    }
    headers.insert(header::CACHE_CONTROL, cache_control);
    response
}

async fn title_image_handler(
    State(repository): State<Arc<dyn TitleImageRepository>>,
    Query(query): Query<std::collections::HashMap<String, String>>,
    headers: HeaderMap,
    AxumPath((title_id, kind, variant)): AxumPath<(String, String, String)>,
) -> Response {
    let Some(kind) = TitleImageKind::parse(&kind) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let blob = match repository
        .get_title_image_blob(&title_id, kind, &variant)
        .await
    {
        Ok(Some(blob)) => blob,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(error) => {
            tracing::warn!(
                error = %error,
                title_id = %title_id,
                kind = kind.as_str(),
                variant = %variant,
                "failed to serve title image"
            );
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let etag = title_image_digest_value(&blob.etag).to_string();
    let quoted_etag = format!("\"{etag}\"");
    let cache_control = title_image_cache_control(&etag, query.get("v").map(String::as_str));
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| if_none_match_matches(value, &quoted_etag, &etag))
    {
        let mut response = StatusCode::NOT_MODIFIED.into_response();
        let headers = response.headers_mut();
        if let Ok(value) = HeaderValue::from_str(&quoted_etag) {
            headers.insert(header::ETAG, value);
        }
        headers.insert(header::CACHE_CONTROL, cache_control);
        return response;
    }

    let body_len = blob.bytes.len();
    let mut response = blob.bytes.into_response();
    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(&blob.content_type) {
        headers.insert(header::CONTENT_TYPE, value);
    }
    if let Ok(value) = HeaderValue::from_str(&body_len.to_string()) {
        headers.insert(header::CONTENT_LENGTH, value);
    }
    if let Ok(value) = HeaderValue::from_str(&quoted_etag) {
        headers.insert(header::ETAG, value);
    }
    headers.insert(header::CACHE_CONTROL, cache_control);
    response
}

fn title_image_cache_control(etag: &str, query_version: Option<&str>) -> HeaderValue {
    let expected_version = title_image_version_from_etag(etag);
    if query_version.is_some_and(|version| version == expected_version.as_str()) {
        HeaderValue::from_static("public, max-age=31536000, immutable")
    } else {
        HeaderValue::from_static("public, max-age=0, must-revalidate")
    }
}

fn title_image_version_from_etag(etag: &str) -> String {
    title_image_digest_value(etag).chars().take(16).collect()
}

fn title_image_digest_value(value: &str) -> &str {
    value
        .split_once(':')
        .map(|(_, digest)| digest)
        .unwrap_or(value)
}

fn if_none_match_matches(raw_header: &str, quoted_etag: &str, bare_etag: &str) -> bool {
    raw_header.split(',').map(str::trim).any(|candidate| {
        candidate == "*"
            || candidate == quoted_etag
            || candidate == bare_etag
            || candidate
                .strip_prefix("W/")
                .is_some_and(|weak| weak == quoted_etag || weak == bare_etag)
    })
}

/// ValidateOnly mode: check for pending migrations and exit.
async fn run_validate_only(config: DatastoreConfig) {
    tracing::info!(
        engine = config.engine.as_str(),
        config_source = config.source.as_str(),
        database_url = config.safe_database_url(),
        "validating datastore"
    );
    match validate_datastore(config).await {
        Ok(_) => {}
        Err(error) => {
            let message = error.to_string();
            if let Some(pending) = extract_pending_migration_ids(&message) {
                for migration_id in pending {
                    eprintln!("{migration_id}");
                }
            } else {
                eprintln!("{error}");
            }
            std::process::exit(1);
        }
    }
}

async fn flush_upstream_scheduler_after_shutdown<F, Fut, E>(token: CancellationToken, flush: F)
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<(), E>>,
    E: std::fmt::Display,
{
    token.cancelled().await;
    match flush().await {
        Ok(()) => tracing::debug!("flushed upstream scheduler state during shutdown"),
        Err(error) => {
            tracing::warn!(%error, "failed to flush upstream scheduler state during shutdown")
        }
    }
}

async fn shutdown_signal(token: CancellationToken) {
    let ctrl_c = tokio::signal::ctrl_c();

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            tracing::info!("received SIGINT, shutting down");
        }
        _ = terminate => {
            tracing::info!("received SIGTERM, shutting down");
        }
        _ = token.cancelled() => {}
    }
    token.cancel();

    // Hard exit if graceful shutdown takes too long.
    tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        tracing::warn!("graceful shutdown timed out, forcing exit");
        std::process::exit(0);
    });
}

/// Extract `--data-dir <path>` or `--data-dir=<path>` from the arg list,
/// removing those elements so the remaining args are clean for subcommand dispatch.
fn extract_data_dir(args: &mut Vec<String>) -> Result<Option<PathBuf>, String> {
    extract_path_arg(args, "--data-dir")
}

/// Extract `--log-file <path>` or `--log-file=<path>` from the arg list,
/// removing those elements so the remaining args are clean for subcommand dispatch.
fn extract_log_file(args: &mut Vec<String>) -> Result<Option<PathBuf>, String> {
    extract_path_arg(args, "--log-file")
}

fn extract_path_arg(args: &mut Vec<String>, flag: &str) -> Result<Option<PathBuf>, String> {
    let mut i = 1; // skip binary name
    let equals_flag = format!("{flag}=");
    while i < args.len() {
        if args[i] == flag {
            args.remove(i);
            if i < args.len() {
                if is_startup_arg_token(&args[i]) {
                    return Err(format!("{flag} requires a path argument"));
                }
                return Ok(Some(PathBuf::from(args.remove(i))));
            }
            return Err(format!("{flag} requires a path argument"));
        } else if let Some(value) = args[i].strip_prefix(&equals_flag) {
            if value.is_empty() {
                return Err(format!("{flag} requires a path argument"));
            }
            let path = PathBuf::from(value);
            args.remove(i);
            return Ok(Some(path));
        } else {
            i += 1;
        }
    }
    Ok(None)
}

fn is_startup_arg_token(value: &str) -> bool {
    matches!(
        value,
        "--data-dir" | "--log-file" | "--generate-key" | "--version" | "-V" | "init"
    ) || value.starts_with("--data-dir=")
        || value.starts_with("--log-file=")
}

/// Resolve the data directory from CLI flag or platform default.
///
/// Priority: `--data-dir` flag > platform default via `directories` crate.
/// The env var `SCRYER_DB_PATH` can still override the *database path* specifically,
/// but the data directory itself is resolved here.
fn resolve_data_dir(cli_override: Option<&Path>) -> PathBuf {
    if let Some(dir) = cli_override {
        return dir.to_path_buf();
    }
    directories::ProjectDirs::from("", "", "scryer")
        .map(|p| p.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn desktop_credential_namespace(data_dir: &Path) -> Option<&'static str> {
    #[cfg(windows)]
    {
        let profile_name = data_dir.file_name()?.to_string_lossy();
        let vendor_name = data_dir.parent()?.file_name()?.to_string_lossy();
        if profile_name.eq_ignore_ascii_case("Scryer")
            && vendor_name.eq_ignore_ascii_case("ScryerMedia")
        {
            return Some("ScryerMedia.Scryer.Desktop.v1");
        }
    }

    #[cfg(not(windows))]
    let _ = data_dir;

    None
}

/// Resolve the private native-code cache independently from the persistent
/// database directory. On Windows the database remains in Roaming AppData for
/// backward compatibility, while Wasmtime artifacts stay in Local AppData so
/// they never roam between incompatible machines. Explicit `--data-dir`
/// instances keep their cache below that directory on every platform.
fn resolve_wasmtime_cache_dir(data_dir_override: Option<&Path>, data_dir: &Path) -> PathBuf {
    if data_dir_override.is_some() {
        return data_dir.join("cache").join("wasmtime");
    }

    #[cfg(windows)]
    {
        if let Some(project_dirs) = directories::ProjectDirs::from("", "", "scryer") {
            return project_dirs.data_local_dir().join("cache").join("wasmtime");
        }
    }

    data_dir.join("cache").join("wasmtime")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedLogFileConfig {
    path: PathBuf,
    explicit: bool,
}

fn resolve_log_file_config(
    cli_override: Option<&Path>,
    env_override: Option<&str>,
    data_dir: &Path,
    default_path: Option<PathBuf>,
) -> Option<ResolvedLogFileConfig> {
    if let Some(path) = cli_override {
        return Some(ResolvedLogFileConfig {
            path: resolve_explicit_log_file_path(path, data_dir),
            explicit: true,
        });
    }

    if let Some(path) = env_override {
        return Some(ResolvedLogFileConfig {
            path: resolve_explicit_log_file_path(Path::new(path), data_dir),
            explicit: true,
        });
    }

    default_path.map(|path| ResolvedLogFileConfig {
        path,
        explicit: false,
    })
}

fn resolve_explicit_log_file_path(path: &Path, data_dir: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        data_dir.join(path)
    }
}

fn default_windows_log_file_path() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        return directories::BaseDirs::new().map(|dirs| {
            dirs.data_local_dir()
                .join("scryer")
                .join("logs")
                .join("scryer.log")
        });
    }

    #[cfg(not(windows))]
    {
        None
    }
}

fn load_env_file(data_dir: Option<&Path>, include_managed_instance_secrets: bool) {
    // Load in reverse priority order: dotenvy skips vars already set, so the
    // last file loaded has lowest priority.  Load the crate-local file first
    // (highest priority), then cwd .env, then data-dir .env (lowest priority).
    let candidates = ["crates/scryer/.env", ".env"];
    let mut loaded = false;
    for candidate in candidates {
        if Path::new(candidate).exists() {
            let _ = dotenvy::from_path(candidate);
            loaded = true;
        }
    }
    // Also load .env from the data directory (lowest priority).
    if let Some(dir) = data_dir {
        let env_path = dir.join(".env");
        if env_path.exists() {
            let _ = dotenvy::from_path(env_path);
            loaded = true;
        }
        if include_managed_instance_secrets {
            let secrets_path = dir.join("instance-secrets.env");
            if secrets_path.exists() {
                let _ = dotenvy::from_path_override(secrets_path);
                loaded = true;
            }
        }
    }
    if !loaded {
        let _ = dotenvy::dotenv();
    }
}

/// Open the user's default browser when running natively (not in Docker).
/// Controlled by `SCRYER_OPEN_BROWSER` env var: "false" disables, default is auto-detect.
fn maybe_open_browser(url: &str) {
    // Respect explicit opt-out.
    if let Ok(val) = std::env::var("SCRYER_OPEN_BROWSER")
        && matches!(
            val.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        )
    {
        return;
    }
    // Skip in containers (Docker sets /.dockerenv).
    if Path::new("/.dockerenv").exists() {
        return;
    }
    if let Err(err) = open::that(url) {
        tracing::debug!(error = %err, "could not open browser");
    }
}

pub(crate) fn normalize_env_option(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn comma_separated_env_has_entries(value: &str) -> bool {
    value.split(',').any(|entry| !entry.trim().is_empty())
}

fn build_webauthn_runtime() -> Option<Arc<webauthn_rs::Webauthn>> {
    let rp_id = normalize_env_option("SCRYER_WEBAUTHN_RP_ID");
    let rp_origin = normalize_env_option("SCRYER_WEBAUTHN_RP_ORIGIN");
    let rp_name =
        normalize_env_option("SCRYER_WEBAUTHN_RP_NAME").unwrap_or_else(|| "Scryer".to_string());

    match (rp_id, rp_origin) {
        (Some(rp_id), Some(rp_origin)) => {
            let origin = match Url::parse(&rp_origin) {
                Ok(origin) => origin,
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        "disabling passkeys because SCRYER_WEBAUTHN_RP_ORIGIN is invalid"
                    );
                    return None;
                }
            };

            let builder = match WebauthnBuilder::new(&rp_id, &origin) {
                Ok(builder) => builder,
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        "disabling passkeys because the WebAuthn RP config is invalid"
                    );
                    return None;
                }
            }
            .rp_name(&rp_name);

            match builder.build() {
                Ok(runtime) => Some(Arc::new(runtime)),
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        "disabling passkeys because the WebAuthn runtime could not be built"
                    );
                    None
                }
            }
        }
        (Some(_), None) | (None, Some(_)) => {
            tracing::warn!(
                "disabling passkeys because SCRYER_WEBAUTHN_RP_ID and SCRYER_WEBAUTHN_RP_ORIGIN must both be set"
            );
            None
        }
        (None, None) => None,
    }
}

fn load_or_create_persistent_jwt_signing_salt(data_dir: &Path) -> std::io::Result<String> {
    use aws_lc_rs::rand::{SecureRandom, SystemRandom};
    use std::io::Write;

    std::fs::create_dir_all(data_dir)?;
    let path = data_dir.join("jwt-signing-secret");
    match std::fs::read_to_string(&path) {
        Ok(existing) => {
            let existing = existing.trim();
            if !existing.is_empty() {
                return Ok(existing.to_string());
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    let rng = SystemRandom::new();
    let mut bytes = [0_u8; 32];
    rng.fill(&mut bytes)
        .map_err(|_| std::io::Error::other("failed to generate JWT signing secret"))?;
    let secret = format!("scryer-jwt-v1-{}", hex_bytes(&bytes));

    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(mut file) => {
            file.write_all(secret.as_bytes())?;
            file.write_all(b"\n")?;
            Ok(secret)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let existing = std::fs::read_to_string(&path)?;
            let existing = existing.trim();
            if existing.is_empty() {
                Err(std::io::Error::other("JWT signing secret file is empty"))
            } else {
                Ok(existing.to_string())
            }
        }
        Err(error) => Err(error),
    }
}

fn hex_bytes(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        let _ = write!(&mut encoded, "{byte:02x}");
    }
    encoded
}

fn parse_env_bool_value(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "y" | "on" => Some(true),
        "0" | "false" | "no" | "n" | "off" => Some(false),
        _ => None,
    }
}

fn validate_unauthenticated_public_access_allowlist_config(
    allowlist_env_configured: bool,
    allowlist_has_valid_entries: bool,
) -> Result<(), String> {
    if allowlist_env_configured && !allowlist_has_valid_entries {
        return Err(format!(
            "{UNAUTHENTICATED_PUBLIC_ACCESS_ALLOWLIST_ENV} is set but contains no valid IP, CIDR, or DNS entries; fix the allowlist or unset it"
        ));
    }
    Ok(())
}

fn resolve_auth_mode(
    auth_enabled_raw: Option<&str>,
    legacy_dev_auto_login_raw: Option<&str>,
    recovery_admin_password_raw: Option<&str>,
    allow_unauthenticated_public_access_raw: Option<&str>,
) -> Result<AuthModeConfig, String> {
    let used_legacy_dev_auto_login = matches!(
        legacy_dev_auto_login_raw.and_then(parse_env_bool_value),
        Some(true)
    );
    let allow_unauthenticated_public_access = matches!(
        allow_unauthenticated_public_access_raw.and_then(parse_env_bool_value),
        Some(true)
    );

    if recovery_admin_password_raw
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
    {
        if allow_unauthenticated_public_access {
            return Err(format!(
                "{ALLOW_UNAUTHENTICATED_PUBLIC_ACCESS_ENV}=true cannot be used with {RECOVERY_ADMIN_PASSWORD_ENV}; recovery mode is private/local only"
            ));
        }
        return Ok(AuthModeConfig {
            env_override_form_login_enabled: Some(true),
            env_override_description: Some(format!("{RECOVERY_ADMIN_PASSWORD_ENV}=set")),
            used_legacy_dev_auto_login: false,
            recovery_admin_password_set: true,
            allow_unauthenticated_public_access: false,
        });
    }

    if let Some(auth_enabled) = auth_enabled_raw.and_then(parse_env_bool_value) {
        return Ok(AuthModeConfig {
            env_override_form_login_enabled: Some(auth_enabled),
            env_override_description: Some(format!("SCRYER_AUTH_ENABLED={auth_enabled}")),
            used_legacy_dev_auto_login: false,
            recovery_admin_password_set: false,
            allow_unauthenticated_public_access,
        });
    }

    Ok(AuthModeConfig {
        env_override_form_login_enabled: used_legacy_dev_auto_login.then_some(false),
        env_override_description: used_legacy_dev_auto_login
            .then_some("SCRYER_DEV_AUTO_LOGIN=true".to_string()),
        used_legacy_dev_auto_login,
        recovery_admin_password_set: false,
        allow_unauthenticated_public_access,
    })
}

fn resolve_auth_mode_from_env() -> Result<AuthModeConfig, String> {
    resolve_auth_mode(
        normalize_env_option("SCRYER_AUTH_ENABLED").as_deref(),
        normalize_env_option("SCRYER_DEV_AUTO_LOGIN").as_deref(),
        normalize_env_option(RECOVERY_ADMIN_PASSWORD_ENV).as_deref(),
        normalize_env_option(ALLOW_UNAUTHENTICATED_PUBLIC_ACCESS_ENV).as_deref(),
    )
}

fn parse_env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn parse_optional_setting_string(value_json: &str) -> Option<String> {
    serde_json::from_str::<Option<String>>(value_json)
        .ok()
        .flatten()
        .or_else(|| serde_json::from_str::<String>(value_json).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

async fn read_post_upgrade_auto_backup_pending_version(
    settings_store: Arc<SettingsStore>,
) -> Option<String> {
    match settings_store
        .get_setting_with_defaults(
            SETTINGS_SCOPE_SYSTEM,
            AUTO_BACKUP_POST_UPGRADE_PENDING_VERSION_KEY,
            None,
        )
        .await
    {
        Ok(Some(record)) => parse_optional_setting_string(&record.effective_value_json),
        Ok(None) => None,
        Err(error) => {
            tracing::warn!(
                error = %error,
                "failed to read post-upgrade automatic backup marker"
            );
            None
        }
    }
}

async fn record_post_upgrade_auto_backup_pending_if_needed(
    settings_store: Arc<SettingsStore>,
    lifecycle: &VersionLifecycle,
) {
    if !matches!(lifecycle, VersionLifecycle::Upgraded { .. }) {
        return;
    }

    let value_json = serde_json::to_string(VERSION).expect("VERSION serializes");
    if let Err(error) = settings_store
        .upsert_setting_value(
            SETTINGS_SCOPE_SYSTEM,
            AUTO_BACKUP_POST_UPGRADE_PENDING_VERSION_KEY,
            None,
            value_json,
            "system",
            None,
        )
        .await
    {
        tracing::warn!(
            error = %error,
            "failed to record post-upgrade automatic backup marker"
        );
    }
}

async fn clear_post_upgrade_auto_backup_pending_version(
    settings_store: Arc<SettingsStore>,
    reason: &'static str,
) {
    if let Err(error) = settings_store
        .delete_setting_value(
            SETTINGS_SCOPE_SYSTEM,
            AUTO_BACKUP_POST_UPGRADE_PENDING_VERSION_KEY,
            None,
        )
        .await
    {
        tracing::warn!(
            error = %error,
            reason,
            "failed to clear post-upgrade automatic backup marker"
        );
    }
}

async fn spawn_post_upgrade_auto_backup_if_pending(
    app: AppUseCase,
    settings_store: Arc<SettingsStore>,
) {
    let pending_version =
        read_post_upgrade_auto_backup_pending_version(settings_store.clone()).await;
    if pending_version.as_deref() != Some(VERSION) {
        return;
    }

    let settings = match app.auto_backup_settings().await {
        Ok(settings) => settings,
        Err(error) => {
            tracing::warn!(
                error = %error,
                "failed to read automatic backup settings for post-upgrade backup"
            );
            return;
        }
    };
    if !settings.enabled || !settings.auto_backup_key_present {
        clear_post_upgrade_auto_backup_pending_version(settings_store, "auto backup not eligible")
            .await;
        return;
    }

    tokio::spawn(async move {
        match app
            .run_scheduled_auto_backup_job_now(JobTriggerSource::SystemInternal)
            .await
        {
            Ok(AutoBackupRunOutcome::Created { info, .. }) => {
                clear_post_upgrade_auto_backup_pending_version(
                    settings_store,
                    "post-upgrade automatic backup created",
                )
                .await;
                tracing::info!(
                    filename = %info.filename,
                    "post-upgrade automatic backup completed"
                );
            }
            Ok(AutoBackupRunOutcome::Skipped { reason }) => {
                tracing::warn!(
                    reason,
                    "post-upgrade automatic backup skipped; will retry on next startup"
                );
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "post-upgrade automatic backup failed; will retry on next startup"
                );
            }
        }
    });
}

async fn check_version_upgrade(settings_store: Arc<SettingsStore>) -> VersionLifecycle {
    const SCOPE: &str = "system";
    const KEY: &str = "last_run_version";

    let previous = settings_store
        .get_setting_with_defaults(SCOPE, KEY, None)
        .await
        .ok()
        .flatten()
        .and_then(|r| r.value_json)
        .and_then(|v| serde_json::from_str::<String>(&v).ok());

    let lifecycle = match previous.as_deref() {
        Some(prev) if prev == VERSION => {
            tracing::debug!(version = VERSION, "version unchanged");
            VersionLifecycle::Unchanged
        }
        Some(prev) => {
            tracing::info!(
                previous_version = prev,
                current_version = VERSION,
                "upgraded from {prev} to {VERSION}"
            );
            VersionLifecycle::Upgraded {
                previous: prev.to_string(),
            }
        }
        None => {
            tracing::info!(version = VERSION, "first run — recording version");
            VersionLifecycle::FirstRun
        }
    };

    let version_json = serde_json::to_string(VERSION).unwrap();
    if let Err(error) = settings_store
        .upsert_setting_value(SCOPE, KEY, None, version_json, "system", None)
        .await
    {
        tracing::warn!(error = %error, "failed to persist last_run_version");
    }

    lifecycle
}

pub(crate) fn normalize_env_option_with_legacy<'a>(
    names: impl IntoIterator<Item = &'a str>,
) -> Option<String> {
    for name in names {
        if let Some(value) = normalize_env_option(name) {
            return Some(value);
        }
    }

    None
}

fn runtime_normalized_constraint(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|constraint| !constraint.is_empty())
        .map(str::to_string)
}

fn runtime_installation_is_host_blocked(installation: &scryer_domain::PluginInstallation) -> bool {
    runtime_normalized_constraint(installation.scryer_constraint.as_deref()).is_some_and(
        |constraint| {
            scryer_plugins::host_version_matches_constraint(env!("CARGO_PKG_VERSION"), &constraint)
                .map(|matches| !matches)
                .unwrap_or(true)
        },
    )
}

#[derive(Debug)]
struct RuntimePluginSdkContractFailure {
    plugin_id: String,
    version: String,
    sdk_version: String,
    sdk_constraint: String,
    error: String,
}

type RuntimePluginLoadInput = (
    scryer_domain::PluginInstallation,
    Option<scryer_domain::PersistedPluginWasmPayload>,
);
type RuntimePluginLoadCandidate = (
    scryer_domain::PluginInstallation,
    scryer_domain::PersistedPluginWasmPayload,
);

fn runtime_installation_sdk_contract_failure(
    installation: &scryer_domain::PluginInstallation,
) -> Option<RuntimePluginSdkContractFailure> {
    scryer_plugins::validate_sdk_contract(
        installation.plugin_id.as_str(),
        installation.sdk_version.as_str(),
        installation.sdk_constraint.as_str(),
        scryer_plugins::SDK_VERSION,
    )
    .err()
    .map(|error| RuntimePluginSdkContractFailure {
        plugin_id: installation.plugin_id.clone(),
        version: installation.version.clone(),
        sdk_version: installation.sdk_version.clone(),
        sdk_constraint: installation.sdk_constraint.clone(),
        error: error.to_string(),
    })
}

fn log_runtime_plugin_sdk_contract_failures(failures: &[RuntimePluginSdkContractFailure]) {
    if failures.is_empty() {
        return;
    }

    let plugin_ids = failures
        .iter()
        .map(|failure| failure.plugin_id.as_str())
        .collect::<Vec<_>>()
        .join(",");
    tracing::warn!(
        plugin_count = failures.len(),
        plugin_ids = %plugin_ids,
        "skipping installed plugins with incompatible sdk contracts; upgrade them from Plugins"
    );
    for failure in failures {
        tracing::debug!(
            plugin_id = failure.plugin_id.as_str(),
            version = failure.version.as_str(),
            sdk_version = failure.sdk_version.as_str(),
            sdk_constraint = failure.sdk_constraint.as_str(),
            error = failure.error.as_str(),
            "installed plugin sdk contract is incompatible with this host"
        );
    }
}

fn collect_runtime_plugin_load_candidates(
    enabled_plugins: Vec<RuntimePluginLoadInput>,
) -> (
    Vec<RuntimePluginLoadCandidate>,
    Vec<RuntimePluginSdkContractFailure>,
) {
    let mut pending_plugins = Vec::new();
    let mut sdk_contract_failures = Vec::new();

    for (installation, payload) in enabled_plugins {
        if !matches!(
            installation.source_kind,
            scryer_domain::PluginSourceKind::Downloaded
                | scryer_domain::PluginSourceKind::Community
                | scryer_domain::PluginSourceKind::Manual
        ) {
            continue;
        }
        if let Some(failure) = runtime_installation_sdk_contract_failure(&installation) {
            sdk_contract_failures.push(failure);
            continue;
        }
        if runtime_installation_is_host_blocked(&installation) {
            continue;
        }
        if let Some(payload) = payload {
            pending_plugins.push((installation, payload));
        }
    }

    (pending_plugins, sdk_contract_failures)
}

async fn load_runtime_external_plugin_entry(
    installation: &scryer_domain::PluginInstallation,
    payload: scryer_domain::PersistedPluginWasmPayload,
) -> Option<RuntimePluginLoad> {
    match load_runtime_plugin_from_persisted_installation_payload(installation, &payload).await {
        Ok(runtime_plugin) => Some(runtime_plugin),
        Err(error) => {
            tracing::warn!(
                plugin_id = installation.plugin_id.as_str(),
                version = installation.version.as_str(),
                error = %error,
                "skipping installed plugin after persisted payload validation failed at startup"
            );
            None
        }
    }
}

async fn load_runtime_plugin_state(
    customization_store: &DatastoreCustomizationStore,
) -> Result<(Vec<RuntimePluginLoad>, Vec<String>), String> {
    let enabled_plugins = customization_store
        .get_enabled_plugin_wasm_bytes()
        .await
        .map_err(|error| error.to_string())?;
    let mut runtime_plugins = Vec::new();
    let (pending_plugins, sdk_contract_failures) =
        collect_runtime_plugin_load_candidates(enabled_plugins);
    log_runtime_plugin_sdk_contract_failures(&sdk_contract_failures);
    let mut pending_plugins = pending_plugins.into_iter();
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..RUNTIME_PLUGIN_LOAD_CONCURRENCY {
        let Some((installation, payload)) = pending_plugins.next() else {
            break;
        };
        tasks
            .spawn(async move { load_runtime_external_plugin_entry(&installation, payload).await });
    }
    while let Some(result) = tasks.join_next().await {
        let loaded =
            result.map_err(|error| format!("startup plugin load task panicked: {error}"))?;
        if let Some(entry) = loaded {
            runtime_plugins.push(entry);
        }
        if let Some((installation, payload)) = pending_plugins.next() {
            tasks.spawn(
                async move { load_runtime_external_plugin_entry(&installation, payload).await },
            );
        }
    }

    let disabled_builtin_plugins = customization_store
        .list_plugin_installations()
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter(|installation| installation.is_builtin && !installation.is_enabled)
        .map(|installation| installation.provider_type)
        .collect::<Vec<_>>();

    Ok((runtime_plugins, disabled_builtin_plugins))
}

async fn bootstrap_plugin_installations(
    customization_store: &DatastoreCustomizationStore,
    finalized_pending_restore: bool,
) -> Result<(), String> {
    let removed = customization_store
        .delete_incompatible_external_plugin_installations(finalized_pending_restore)
        .await
        .map_err(|error| error.to_string())?;
    for plugin_id in removed {
        tracing::warn!(
            plugin_id = plugin_id.as_str(),
            "removed incompatible legacy external plugin installation during startup bootstrap"
        );
    }

    seed_builtin_plugin_installations(customization_store).await
}

fn preserves_legacy_nzbgeek_builtin_for_catalog_migration(
    installation: &scryer_domain::PluginInstallation,
) -> bool {
    installation.plugin_id == LEGACY_NZBGEEK_PLUGIN_ID
        && installation.is_builtin
        && installation.source_kind == scryer_domain::PluginSourceKind::Bundled
}

async fn seed_builtin_plugin_installations(
    customization_store: &DatastoreCustomizationStore,
) -> Result<(), String> {
    struct BuiltinPluginSeed {
        name: String,
        description: String,
        version: String,
        sdk_version: String,
        sdk_constraint: String,
        plugin_type: String,
        provider_type: String,
    }

    let mut builtins = Vec::new();

    let indexer_provider = scryer_plugins::build_indexer_plugin_provider(&[], &[]);
    for provider_type in indexer_provider.builtin_provider_types() {
        let provider_key = provider_type.trim().to_ascii_lowercase();
        let Some(name) = indexer_provider.plugin_name_for_provider(&provider_key) else {
            continue;
        };
        let description = indexer_provider
            .plugin_description_for_provider(&provider_key)
            .unwrap_or_default();
        let Some(version) = indexer_provider.plugin_version_for_provider(&provider_key) else {
            continue;
        };
        let Some(sdk_version) = indexer_provider.plugin_sdk_version_for_provider(&provider_key)
        else {
            continue;
        };
        let Some(sdk_constraint) =
            indexer_provider.plugin_sdk_constraint_for_provider(&provider_key)
        else {
            continue;
        };
        let plugin_type = indexer_provider
            .plugin_type_for_provider(&provider_key)
            .unwrap_or_else(|| "indexer".to_string());
        builtins.push(BuiltinPluginSeed {
            name,
            description,
            version,
            sdk_version,
            sdk_constraint,
            plugin_type,
            provider_type: provider_key,
        });
    }

    let subtitle_provider = scryer_plugins::build_subtitle_plugin_provider(&[], &[]);
    for provider_type in subtitle_provider.builtin_provider_types() {
        let provider_key = provider_type.trim().to_ascii_lowercase();
        let Some(name) = subtitle_provider.plugin_name_for_provider(&provider_key) else {
            continue;
        };
        let description = subtitle_provider
            .plugin_description_for_provider(&provider_key)
            .unwrap_or_default();
        let Some(version) = subtitle_provider.plugin_version_for_provider(&provider_key) else {
            continue;
        };
        let Some(sdk_version) = subtitle_provider.plugin_sdk_version_for_provider(&provider_key)
        else {
            continue;
        };
        let Some(sdk_constraint) =
            subtitle_provider.plugin_sdk_constraint_for_provider(&provider_key)
        else {
            continue;
        };
        builtins.push(BuiltinPluginSeed {
            name,
            description,
            version,
            sdk_version,
            sdk_constraint,
            plugin_type: "subtitle_provider".to_string(),
            provider_type: provider_key,
        });
    }

    let download_client_provider = scryer_plugins::build_download_client_plugin_provider(&[], &[]);
    for provider_type in download_client_provider.builtin_provider_types() {
        let provider_key = provider_type.trim().to_ascii_lowercase();
        let Some(name) = download_client_provider.plugin_name_for_provider(&provider_key) else {
            continue;
        };
        let description = download_client_provider
            .plugin_description_for_provider(&provider_key)
            .unwrap_or_default();
        let Some(version) = download_client_provider.plugin_version_for_provider(&provider_key)
        else {
            continue;
        };
        let Some(sdk_version) =
            download_client_provider.plugin_sdk_version_for_provider(&provider_key)
        else {
            continue;
        };
        let Some(sdk_constraint) =
            download_client_provider.plugin_sdk_constraint_for_provider(&provider_key)
        else {
            continue;
        };
        builtins.push(BuiltinPluginSeed {
            name,
            description,
            version,
            sdk_version,
            sdk_constraint,
            plugin_type: "download_client".to_string(),
            provider_type: provider_key,
        });
    }

    let notification_provider = scryer_plugins::build_notification_plugin_provider(&[], &[]);
    for provider_type in notification_provider.builtin_provider_types() {
        let provider_key = provider_type.trim().to_ascii_lowercase();
        let Some(name) = notification_provider.plugin_name_for_provider(&provider_key) else {
            continue;
        };
        let description = notification_provider
            .plugin_description_for_provider(&provider_key)
            .unwrap_or_default();
        let Some(version) = notification_provider.plugin_version_for_provider(&provider_key) else {
            continue;
        };
        let Some(sdk_version) =
            notification_provider.plugin_sdk_version_for_provider(&provider_key)
        else {
            continue;
        };
        let Some(sdk_constraint) =
            notification_provider.plugin_sdk_constraint_for_provider(&provider_key)
        else {
            continue;
        };
        builtins.push(BuiltinPluginSeed {
            name,
            description,
            version,
            sdk_version,
            sdk_constraint,
            plugin_type: "notification".to_string(),
            provider_type: provider_key,
        });
    }

    let builtin_lookup_key = |plugin_type: &str, provider_type: &str| {
        let family = match plugin_type {
            "indexer" | "usenet_indexer" | "torrent_indexer" => "indexer",
            other => other,
        };
        format!("{family}::{}", provider_type.trim().to_ascii_lowercase())
    };

    let builtin_keys = builtins
        .iter()
        .map(|builtin| builtin_lookup_key(&builtin.plugin_type, &builtin.provider_type))
        .collect::<std::collections::HashSet<_>>();

    for builtin in builtins {
        customization_store
            .seed_builtin(
                &builtin.provider_type,
                &builtin.name,
                &builtin.description,
                &builtin.version,
                &builtin.sdk_version,
                &builtin.sdk_constraint,
                &builtin.plugin_type,
                &builtin.provider_type,
            )
            .await
            .map_err(|error| error.to_string())?;
    }

    let stale_builtin_plugin_ids = customization_store
        .list_plugin_installations()
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter(|installation| {
            installation.is_builtin
                && !preserves_legacy_nzbgeek_builtin_for_catalog_migration(installation)
                && !builtin_keys.contains(&builtin_lookup_key(
                    &installation.plugin_type,
                    &installation.provider_type,
                ))
        })
        .map(|installation| installation.plugin_id)
        .collect::<Vec<_>>();

    for plugin_id in stale_builtin_plugin_ids {
        customization_store
            .delete_plugin_installation(&plugin_id)
            .await
            .map_err(|error| error.to_string())?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        ALLOW_UNAUTHENTICATED_PUBLIC_ACCESS_ENV, AuthModeConfig, RECOVERY_ADMIN_PASSWORD_ENV,
        ResolvedLogFileConfig, SelfRestartController, UNAUTHENTICATED_PUBLIC_ACCESS_ALLOWLIST_ENV,
        bootstrap_plugin_installations, collect_runtime_plugin_load_candidates,
        comma_separated_env_has_entries, extract_data_dir, extract_log_file,
        flush_upstream_scheduler_after_shutdown, load_runtime_plugin_state, resolve_auth_mode,
        resolve_log_file_config, resolve_wasmtime_cache_dir, restart_spec_from_parts,
        title_image_handler, validate_unauthenticated_public_access_allowlist_config,
    };
    use chrono::Utc;
    use std::ffi::OsString;
    use std::io;
    use std::path::PathBuf;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use std::time::Duration;

    use crate::base_path::{BasePath, mount_router};
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use axum::routing::get;
    use scryer_application::{
        AppResult, PluginInstallationRepository, TitleImageBlob, TitleImageKind,
        TitleImageRepository, TitleImageSourceResult, TitleImageSyncTask,
    };
    use scryer_infrastructure::{DatastoreCustomizationStore, SqliteServices};
    use tempfile::tempdir;
    use tower::ServiceExt;

    #[tokio::test]
    async fn shutdown_flush_waiter_flushes_scheduler_after_cancellation() {
        let token = tokio_util::sync::CancellationToken::new();
        let flush_count = Arc::new(AtomicUsize::new(0));
        let flush_count_for_task = Arc::clone(&flush_count);
        let handle = tokio::spawn(flush_upstream_scheduler_after_shutdown(
            token.clone(),
            move || async move {
                flush_count_for_task.fetch_add(1, Ordering::SeqCst);
                Ok::<(), &'static str>(())
            },
        ));

        tokio::task::yield_now().await;
        assert_eq!(flush_count.load(Ordering::SeqCst), 0);

        token.cancel();
        tokio::time::timeout(Duration::from_secs(1), handle)
            .await
            .expect("shutdown flush waiter should finish")
            .expect("shutdown flush task should not panic");

        assert_eq!(flush_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn restart_spec_builder_preserves_executable_args_and_env() {
        let spec = restart_spec_from_parts(
            "/usr/local/bin/scryer".into(),
            vec![OsString::from("--data-dir"), OsString::from("/config")],
            vec![(OsString::from("SCRYER_MODE"), OsString::from("restore"))],
            "/opt/scryer".into(),
        );

        assert_eq!(
            spec.executable,
            std::path::PathBuf::from("/usr/local/bin/scryer")
        );
        assert_eq!(
            spec.args,
            vec![OsString::from("--data-dir"), OsString::from("/config")]
        );
        assert_eq!(
            spec.env,
            vec![(OsString::from("SCRYER_MODE"), OsString::from("restore"))]
        );
        assert_eq!(spec.current_dir, std::path::PathBuf::from("/opt/scryer"));
    }

    #[test]
    fn restart_controller_allows_retry_when_relaunch_fails() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let attempts_handle = attempts.clone();
        let controller = SelfRestartController::with_launcher(
            Duration::from_millis(0),
            Arc::new(move || {
                attempts_handle.fetch_add(1, Ordering::SeqCst);
                Err(io::Error::other("boom"))
            }),
        );

        controller.schedule_restart();
        std::thread::sleep(Duration::from_millis(40));
        controller.schedule_restart();
        std::thread::sleep(Duration::from_millis(40));

        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn extract_log_file_removes_space_form_before_subcommand_dispatch() {
        let mut args = vec![
            "scryer".to_string(),
            "--log-file".to_string(),
            "logs/scryer.log".to_string(),
            "--version".to_string(),
        ];

        let path = extract_log_file(&mut args)
            .expect("extract log file")
            .expect("log file path");

        assert_eq!(path, PathBuf::from("logs/scryer.log"));
        assert_eq!(args, vec!["scryer".to_string(), "--version".to_string()]);
    }

    #[test]
    fn extract_log_file_removes_equals_form_before_subcommand_dispatch() {
        let mut args = vec![
            "scryer".to_string(),
            "--log-file=logs/scryer.log".to_string(),
            "init".to_string(),
        ];

        let path = extract_log_file(&mut args)
            .expect("extract log file")
            .expect("log file path");

        assert_eq!(path, PathBuf::from("logs/scryer.log"));
        assert_eq!(args, vec!["scryer".to_string(), "init".to_string()]);
    }

    #[test]
    fn extract_log_file_reports_missing_value() {
        let mut args = vec!["scryer".to_string(), "--log-file".to_string()];

        let error = extract_log_file(&mut args).expect_err("missing value should fail");

        assert!(error.contains("--log-file requires a path argument"));
    }

    #[test]
    fn extract_log_file_rejects_version_flag_as_value() {
        let mut args = vec![
            "scryer".to_string(),
            "--log-file".to_string(),
            "--version".to_string(),
        ];

        let error = extract_log_file(&mut args).expect_err("flag value should fail");

        assert!(error.contains("--log-file requires a path argument"));
    }

    #[test]
    fn extract_log_file_rejects_subcommand_as_value() {
        let mut args = vec![
            "scryer".to_string(),
            "--log-file".to_string(),
            "init".to_string(),
        ];

        let error = extract_log_file(&mut args).expect_err("subcommand value should fail");

        assert!(error.contains("--log-file requires a path argument"));
    }

    #[test]
    fn extract_data_dir_rejects_log_file_flag_as_value() {
        let mut args = vec![
            "scryer".to_string(),
            "--data-dir".to_string(),
            "--log-file".to_string(),
            "logs/scryer.log".to_string(),
        ];

        let error = extract_data_dir(&mut args).expect_err("flag value should fail");

        assert!(error.contains("--data-dir requires a path argument"));
    }

    #[test]
    fn extract_log_file_reports_empty_equals_value() {
        let mut args = vec!["scryer".to_string(), "--log-file=".to_string()];

        let error = extract_log_file(&mut args).expect_err("empty value should fail");

        assert!(error.contains("--log-file requires a path argument"));
    }

    #[test]
    fn extract_data_dir_still_removes_data_dir_flag() {
        let mut args = vec![
            "scryer".to_string(),
            "--data-dir".to_string(),
            "/config".to_string(),
            "--version".to_string(),
        ];

        let path = extract_data_dir(&mut args)
            .expect("extract data dir")
            .expect("data dir path");

        assert_eq!(path, PathBuf::from("/config"));
        assert_eq!(args, vec!["scryer".to_string(), "--version".to_string()]);
    }

    #[test]
    fn explicit_data_dir_keeps_wasmtime_cache_in_the_instance_directory() {
        let data_dir = PathBuf::from("/instance-data");
        assert_eq!(
            resolve_wasmtime_cache_dir(Some(data_dir.as_path()), &data_dir),
            data_dir.join("cache").join("wasmtime")
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn default_wasmtime_cache_is_below_the_platform_data_dir() {
        let data_dir = PathBuf::from("/platform-data");
        assert_eq!(
            resolve_wasmtime_cache_dir(None, &data_dir),
            data_dir.join("cache").join("wasmtime")
        );
    }

    #[cfg(windows)]
    #[test]
    fn default_wasmtime_cache_uses_local_app_data() {
        let data_dir = PathBuf::from(r"C:\Roaming\scryer\data");
        let resolved = resolve_wasmtime_cache_dir(None, &data_dir);
        let expected = directories::ProjectDirs::from("", "", "scryer")
            .map(|project_dirs| project_dirs.data_local_dir().join("cache").join("wasmtime"))
            .unwrap_or_else(|| data_dir.join("cache").join("wasmtime"));
        assert_eq!(resolved, expected);
    }

    #[test]
    fn log_file_config_prefers_cli_then_env_then_default() {
        let dir = tempdir().expect("tempdir");
        let data_dir = dir.path().join("data");
        let default = dir.path().join("default").join("scryer.log");

        assert_eq!(
            resolve_log_file_config(
                Some(std::path::Path::new("cli.log")),
                Some("env.log"),
                &data_dir,
                Some(default.clone())
            ),
            Some(ResolvedLogFileConfig {
                path: data_dir.join("cli.log"),
                explicit: true,
            })
        );
        assert_eq!(
            resolve_log_file_config(None, Some("env.log"), &data_dir, Some(default.clone())),
            Some(ResolvedLogFileConfig {
                path: data_dir.join("env.log"),
                explicit: true,
            })
        );
        assert_eq!(
            resolve_log_file_config(None, None, &data_dir, Some(default.clone())),
            Some(ResolvedLogFileConfig {
                path: default,
                explicit: false,
            })
        );
    }

    #[test]
    fn log_file_config_has_no_default_when_default_path_is_absent() {
        let dir = tempdir().expect("tempdir");
        let data_dir = dir.path().join("data");

        assert_eq!(resolve_log_file_config(None, None, &data_dir, None), None);
    }

    #[test]
    fn log_file_config_keeps_absolute_explicit_paths() {
        let dir = tempdir().expect("tempdir");
        let data_dir = dir.path().join("data");
        let absolute = dir.path().join("scryer.log");

        assert_eq!(
            resolve_log_file_config(Some(&absolute), Some("env.log"), &data_dir, None),
            Some(ResolvedLogFileConfig {
                path: absolute,
                explicit: true,
            })
        );
    }

    #[derive(Default)]
    struct MockTitleImageRepository {
        blob: Option<TitleImageBlob>,
    }

    #[async_graphql::async_trait::async_trait]
    impl TitleImageRepository for MockTitleImageRepository {
        async fn list_title_image_refresh_work(
            &self,
            _limit: usize,
            _skipped: &[TitleImageSyncTask],
        ) -> AppResult<Vec<TitleImageSyncTask>> {
            Ok(Vec::new())
        }

        async fn clear_title_image_cache(&self) -> AppResult<()> {
            Ok(())
        }

        async fn upsert_title_image_source_result(
            &self,
            _title_id: &str,
            _result: TitleImageSourceResult,
            _event: Option<scryer_domain::NewDomainEvent>,
        ) -> AppResult<Option<scryer_domain::DomainEvent>> {
            Ok(None)
        }

        async fn get_title_image_blob(
            &self,
            _title_id: &str,
            _kind: TitleImageKind,
            _variant_key: &str,
        ) -> AppResult<Option<TitleImageBlob>> {
            Ok(self.blob.clone())
        }
    }

    #[test]
    fn auth_defaults_to_disabled() {
        assert_eq!(
            resolve_auth_mode(None, None, None, None).expect("auth mode"),
            AuthModeConfig {
                env_override_form_login_enabled: None,
                env_override_description: None,
                used_legacy_dev_auto_login: false,
                recovery_admin_password_set: false,
                allow_unauthenticated_public_access: false,
            }
        );
    }

    #[test]
    fn explicit_auth_enabled_wins() {
        assert_eq!(
            resolve_auth_mode(Some("true"), Some("true"), None, None).expect("auth mode"),
            AuthModeConfig {
                env_override_form_login_enabled: Some(true),
                env_override_description: Some("SCRYER_AUTH_ENABLED=true".to_string()),
                used_legacy_dev_auto_login: false,
                recovery_admin_password_set: false,
                allow_unauthenticated_public_access: false,
            }
        );
    }

    #[tokio::test]
    async fn bootstrap_preserves_legacy_nzbgeek_builtin_for_catalog_migration() {
        let dir = tempdir().expect("tempdir");
        let db_path = dir.path().join("plugins.db");
        let services = SqliteServices::new(db_path.to_string_lossy())
            .await
            .unwrap();
        let customization = DatastoreCustomizationStore::new(services.datastore());
        let now = Utc::now();

        customization
            .create_plugin_installation(
                &scryer_domain::PluginInstallation {
                    id: scryer_domain::Id::new().0,
                    plugin_id: "nzbgeek".to_string(),
                    name: "NZBGeek Indexer".to_string(),
                    description: "legacy builtin".to_string(),
                    version: "0.2.10".to_string(),
                    sdk_version: "1.3.0".to_string(),
                    sdk_constraint: ">=1.3.0, <1.4.0".to_string(),
                    scryer_constraint: None,
                    plugin_type: "indexer".to_string(),
                    provider_type: "nzbgeek".to_string(),
                    source_kind: scryer_domain::PluginSourceKind::Bundled,
                    is_enabled: true,
                    is_builtin: true,
                    wasm_encoding: scryer_domain::PluginWasmEncoding::Identity,
                    wasm_digest_algo: None,
                    source_url: None,
                    support_tier: scryer_domain::PluginSupportTier::Official,
                    publisher: Some("scryer".to_string()),
                    docs_url: None,
                    source_repo: None,
                    manifest_url: None,
                    wasm_digest: None,
                    artifact_digest: None,
                    descriptor_json: None,
                    installed_at: now,
                    updated_at: now,
                },
                None,
            )
            .await
            .expect("seed legacy nzbgeek builtin row");

        bootstrap_plugin_installations(&customization, false)
            .await
            .expect("bootstrap plugin installations");

        let installation = customization
            .get_plugin_installation("nzbgeek")
            .await
            .expect("read plugin installation")
            .expect("legacy nzbgeek builtin should be preserved for catalog migration");
        assert!(installation.is_builtin);
        assert_eq!(
            installation.source_kind,
            scryer_domain::PluginSourceKind::Bundled
        );
    }

    #[test]
    fn runtime_plugin_load_candidates_collect_sdk_contract_failures() {
        fn installation(
            plugin_id: &str,
            sdk_version: &str,
            sdk_constraint: &str,
        ) -> scryer_domain::PluginInstallation {
            let now = Utc::now();
            scryer_domain::PluginInstallation {
                id: scryer_domain::Id::new().0,
                plugin_id: plugin_id.to_string(),
                name: plugin_id.to_string(),
                description: plugin_id.to_string(),
                version: "0.1.0".to_string(),
                sdk_version: sdk_version.to_string(),
                sdk_constraint: sdk_constraint.to_string(),
                scryer_constraint: None,
                plugin_type: "notification".to_string(),
                provider_type: plugin_id.to_string(),
                source_kind: scryer_domain::PluginSourceKind::Downloaded,
                is_enabled: true,
                is_builtin: false,
                wasm_encoding: scryer_domain::PluginWasmEncoding::Identity,
                wasm_digest_algo: None,
                source_url: Some(format!("https://example.com/{plugin_id}.wasm")),
                support_tier: scryer_domain::PluginSupportTier::Official,
                publisher: Some("scryer".to_string()),
                docs_url: None,
                source_repo: None,
                manifest_url: None,
                wasm_digest: None,
                artifact_digest: None,
                descriptor_json: None,
                installed_at: now,
                updated_at: now,
            }
        }

        let payload = scryer_domain::PersistedPluginWasmPayload {
            encoding: scryer_domain::PluginWasmEncoding::Identity,
            bytes: vec![1, 2, 3, 4],
        };
        let (pending, failures) = collect_runtime_plugin_load_candidates(vec![
            (
                installation("legacy-email", "1.6.0", ">=1.6.0, <1.7.0"),
                Some(payload.clone()),
            ),
            (
                installation(
                    "current-email",
                    scryer_plugins::SDK_VERSION,
                    &scryer_plugins::sdk_constraint_or_legacy(scryer_plugins::SDK_VERSION, ""),
                ),
                Some(payload),
            ),
        ]);

        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].0.plugin_id, "current-email");
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].plugin_id, "legacy-email");
    }

    #[tokio::test]
    async fn runtime_plugin_state_succeeds_after_bootstrap_deletes_legacy_external_rows() {
        let dir = tempdir().expect("tempdir");
        let db_path = dir.path().join("plugins.db");
        let services = SqliteServices::new(db_path.to_string_lossy())
            .await
            .unwrap();
        let customization = DatastoreCustomizationStore::new(services.datastore());
        let now = Utc::now();

        customization
            .create_plugin_installation(
                &scryer_domain::PluginInstallation {
                    id: scryer_domain::Id::new().0,
                    plugin_id: "legacy".to_string(),
                    name: "Legacy".to_string(),
                    description: "legacy plugin".to_string(),
                    version: "0.1.0".to_string(),
                    sdk_version: "1.3.0".to_string(),
                    sdk_constraint: ">=1.3.0, <1.4.0".to_string(),
                    scryer_constraint: None,
                    plugin_type: "notification".to_string(),
                    provider_type: "legacy".to_string(),
                    source_kind: scryer_domain::PluginSourceKind::Downloaded,
                    is_enabled: true,
                    is_builtin: false,
                    wasm_encoding: scryer_domain::PluginWasmEncoding::Identity,
                    wasm_digest_algo: None,
                    source_url: Some("https://example.com/legacy.wasm".to_string()),
                    support_tier: scryer_domain::PluginSupportTier::Official,
                    publisher: None,
                    docs_url: None,
                    source_repo: None,
                    manifest_url: None,
                    wasm_digest: None,
                    artifact_digest: None,
                    descriptor_json: None,
                    installed_at: now,
                    updated_at: now,
                },
                Some(&[1_u8, 2, 3]),
            )
            .await
            .expect("seed legacy plugin row");

        bootstrap_plugin_installations(&customization, false)
            .await
            .expect("bootstrap plugin installations");

        let (runtime_plugins, disabled_builtins) = load_runtime_plugin_state(&customization)
            .await
            .expect("load runtime plugin state");

        assert!(runtime_plugins.is_empty());
        assert!(disabled_builtins.is_empty());
        assert!(
            customization
                .get_plugin_installation("legacy")
                .await
                .expect("read plugin installation")
                .is_none()
        );
    }

    #[tokio::test]
    async fn bootstrap_preserves_restored_downloaded_plugin_rows_until_recovery() {
        let dir = tempdir().expect("tempdir");
        let db_path = dir.path().join("plugins.db");
        let services = SqliteServices::new(db_path.to_string_lossy())
            .await
            .unwrap();
        let customization = DatastoreCustomizationStore::new(services.datastore());
        let now = Utc::now();

        customization
            .create_plugin_installation(
                &scryer_domain::PluginInstallation {
                    id: scryer_domain::Id::new().0,
                    plugin_id: "email".to_string(),
                    name: "Email".to_string(),
                    description: "Email notifications".to_string(),
                    version: "0.1.9".to_string(),
                    sdk_version: scryer_plugins::SDK_VERSION.to_string(),
                    sdk_constraint: scryer_plugins::sdk_constraint_or_legacy(
                        scryer_plugins::SDK_VERSION,
                        "",
                    ),
                    scryer_constraint: None,
                    plugin_type: "notification".to_string(),
                    provider_type: "email".to_string(),
                    source_kind: scryer_domain::PluginSourceKind::Downloaded,
                    is_enabled: true,
                    is_builtin: false,
                    wasm_encoding: scryer_domain::PluginWasmEncoding::Zstd,
                    wasm_digest_algo: Some("blake3".to_string()),
                    source_url: Some("https://example.com/email.wasm.zst".to_string()),
                    support_tier: scryer_domain::PluginSupportTier::Official,
                    publisher: Some("scryer".to_string()),
                    docs_url: Some("https://example.com/email/docs".to_string()),
                    source_repo: Some("https://github.com/scryer-media/scryer-plugins".to_string()),
                    manifest_url: Some("https://example.com/email.manifest.json".to_string()),
                    wasm_digest: Some("a".repeat(64)),
                    artifact_digest: Some("blake3:abcd".to_string()),
                    descriptor_json: Some(
                        r#"{"id":"email","name":"Email","version":"0.1.9","sdk_version":"1.6.0","sdk_constraint":">=1.6.0, <2.0.0","socket_permissions":[],"provider":{"kind":"notification","provider_type":"email","provider_aliases":[],"config_fields":[],"allowed_hosts":[],"default_base_url":null,"capabilities":{"supported_events":[]}}}"#.to_string(),
                    ),
                    installed_at: now,
                    updated_at: now,
                },
                None,
            )
            .await
            .expect("seed restored downloaded plugin row");

        bootstrap_plugin_installations(&customization, true)
            .await
            .expect("bootstrap plugin installations");

        assert!(
            customization
                .get_plugin_installation("email")
                .await
                .expect("read plugin installation")
                .is_some()
        );

        let (runtime_plugins, disabled_builtins) = load_runtime_plugin_state(&customization)
            .await
            .expect("load runtime plugin state");

        assert!(runtime_plugins.is_empty());
        assert!(disabled_builtins.is_empty());
    }

    #[tokio::test]
    async fn runtime_plugin_state_skips_corrupted_external_plugin_rows() {
        let dir = tempdir().expect("tempdir");
        let db_path = dir.path().join("plugins.db");
        let services = SqliteServices::new(db_path.to_string_lossy())
            .await
            .unwrap();
        let customization = DatastoreCustomizationStore::new(services.datastore());
        let now = Utc::now();
        let compressed = vec![
            0x28, 0xb5, 0x2f, 0xfd, 0x24, 0x00, 0x01, 0x00, 0x00, 0x99, 0xe9, 0xd8, 0x51,
        ];

        customization
            .create_plugin_installation(
                &scryer_domain::PluginInstallation {
                    id: scryer_domain::Id::new().0,
                    plugin_id: "corrupt".to_string(),
                    name: "Corrupt".to_string(),
                    description: "corrupt plugin".to_string(),
                    version: "0.1.0".to_string(),
                    sdk_version: scryer_plugins::SDK_VERSION.to_string(),
                    sdk_constraint: scryer_plugins::sdk_constraint_or_legacy(
                        scryer_plugins::SDK_VERSION,
                        "",
                    ),
                    scryer_constraint: None,
                    plugin_type: "notification".to_string(),
                    provider_type: "corrupt".to_string(),
                    source_kind: scryer_domain::PluginSourceKind::Downloaded,
                    is_enabled: true,
                    is_builtin: false,
                    wasm_encoding: scryer_domain::PluginWasmEncoding::Zstd,
                    wasm_digest_algo: Some("blake3".to_string()),
                    source_url: Some("https://example.com/corrupt.wasm.zst".to_string()),
                    support_tier: scryer_domain::PluginSupportTier::Official,
                    publisher: None,
                    docs_url: None,
                    source_repo: None,
                    manifest_url: None,
                    wasm_digest: Some("deadbeef".to_string()),
                    artifact_digest: Some("blake3:abcd".to_string()),
                    descriptor_json: Some(
                        r#"{"id":"corrupt","name":"Corrupt","version":"0.1.0","sdk_version":"1.6.0","sdk_constraint":">=1.6.0, <2.0.0","socket_permissions":[],"provider":{"kind":"notification","provider_type":"corrupt","provider_aliases":[],"config_fields":[],"allowed_hosts":[],"default_base_url":null,"capabilities":{"supported_events":[]}}}"#.to_string(),
                    ),
                    installed_at: now,
                    updated_at: now,
                },
                Some(compressed.as_slice()),
            )
            .await
            .expect("seed corrupt plugin row");

        bootstrap_plugin_installations(&customization, false)
            .await
            .expect("bootstrap plugin installations");

        let (runtime_plugins, disabled_builtins) = load_runtime_plugin_state(&customization)
            .await
            .expect("load runtime plugin state");

        assert!(runtime_plugins.is_empty());
        assert!(disabled_builtins.is_empty());
        assert!(
            customization
                .get_plugin_installation("corrupt")
                .await
                .expect("read plugin installation")
                .is_some()
        );
    }

    #[test]
    fn explicit_auth_disabled_wins_over_legacy_alias() {
        assert_eq!(
            resolve_auth_mode(Some("false"), Some("true"), None, None).expect("auth mode"),
            AuthModeConfig {
                env_override_form_login_enabled: Some(false),
                env_override_description: Some("SCRYER_AUTH_ENABLED=false".to_string()),
                used_legacy_dev_auto_login: false,
                recovery_admin_password_set: false,
                allow_unauthenticated_public_access: false,
            }
        );
    }

    #[test]
    fn legacy_dev_auto_login_disables_auth_when_new_flag_absent() {
        assert_eq!(
            resolve_auth_mode(None, Some("true"), None, None).expect("auth mode"),
            AuthModeConfig {
                env_override_form_login_enabled: Some(false),
                env_override_description: Some("SCRYER_DEV_AUTO_LOGIN=true".to_string()),
                used_legacy_dev_auto_login: true,
                recovery_admin_password_set: false,
                allow_unauthenticated_public_access: false,
            }
        );
    }

    #[test]
    fn invalid_auth_flag_falls_back_to_default_disabled() {
        assert_eq!(
            resolve_auth_mode(Some("garbage"), None, None, None).expect("auth mode"),
            AuthModeConfig {
                env_override_form_login_enabled: None,
                env_override_description: None,
                used_legacy_dev_auto_login: false,
                recovery_admin_password_set: false,
                allow_unauthenticated_public_access: false,
            }
        );
    }

    #[test]
    fn recovery_admin_password_forces_form_login_and_suppresses_local_bypass() {
        for (auth_enabled, legacy_dev_auto_login) in [
            (Some("true"), None),
            (Some("false"), None),
            (None, Some("true")),
        ] {
            let auth_mode = resolve_auth_mode(
                auth_enabled,
                legacy_dev_auto_login,
                Some("new-password"),
                None,
            )
            .expect("auth mode");
            assert_eq!(
                auth_mode,
                AuthModeConfig {
                    env_override_form_login_enabled: Some(true),
                    env_override_description: Some(format!("{RECOVERY_ADMIN_PASSWORD_ENV}=set")),
                    used_legacy_dev_auto_login: false,
                    recovery_admin_password_set: true,
                    allow_unauthenticated_public_access: false,
                }
            );
            assert!(auth_mode.effective_form_login_enabled(false));
            assert!(!auth_mode.effective_skip_login_for_local_ips(true));
            assert!(!auth_mode.effective_skip_login_for_local_ips(false));
        }
    }

    #[test]
    fn non_recovery_mode_preserves_saved_local_bypass() {
        let auth_mode = resolve_auth_mode(Some("true"), None, None, None).expect("auth mode");
        assert!(auth_mode.effective_skip_login_for_local_ips(true));
        assert!(!auth_mode.effective_skip_login_for_local_ips(false));
        assert!(
            auth_mode.effective_form_login_enabled(false),
            "explicit auth enabled should still force form login"
        );
    }

    #[test]
    fn unauthenticated_public_access_override_is_recorded_without_recovery() {
        assert_eq!(
            resolve_auth_mode(None, None, None, Some("true")).expect("auth mode"),
            AuthModeConfig {
                env_override_form_login_enabled: None,
                env_override_description: None,
                used_legacy_dev_auto_login: false,
                recovery_admin_password_set: false,
                allow_unauthenticated_public_access: true,
            }
        );
    }

    #[test]
    fn unauthenticated_public_access_override_is_rejected_with_recovery() {
        let error = resolve_auth_mode(None, None, Some("new-password"), Some("true"))
            .expect_err("recovery and public unauthenticated access conflict");

        assert!(error.contains(ALLOW_UNAUTHENTICATED_PUBLIC_ACCESS_ENV));
        assert!(error.contains(RECOVERY_ADMIN_PASSWORD_ENV));
    }

    #[test]
    fn unauthenticated_public_access_allowlist_accepts_without_public_access_override() {
        validate_unauthenticated_public_access_allowlist_config(true, true)
            .expect("valid allowlist should imply narrowed public access");
    }

    #[test]
    fn unauthenticated_public_access_allowlist_rejects_no_valid_entries() {
        let error = validate_unauthenticated_public_access_allowlist_config(true, false)
            .expect_err("allowlist with no valid entries should be rejected");

        assert!(error.contains(UNAUTHENTICATED_PUBLIC_ACCESS_ALLOWLIST_ENV));
        assert!(error.contains("no valid IP, CIDR, or DNS entries"));
    }

    #[test]
    fn unauthenticated_public_access_allowlist_accepts_public_access_override() {
        validate_unauthenticated_public_access_allowlist_config(true, true)
            .expect("allowlist should narrow public access override");
    }

    #[test]
    fn unauthenticated_public_access_allowlist_accepts_unset_allowlist() {
        validate_unauthenticated_public_access_allowlist_config(false, false)
            .expect("unset allowlist keeps broad public access override");
    }

    #[test]
    fn comma_separated_env_entries_ignore_empty_items() {
        assert!(!comma_separated_env_has_entries(" ,, "));
        assert!(comma_separated_env_has_entries(",,home.example.test,"));
    }

    #[tokio::test]
    async fn title_image_route_serves_cached_bytes_with_headers() {
        let repo: Arc<dyn TitleImageRepository> = Arc::new(MockTitleImageRepository {
            blob: Some(TitleImageBlob {
                content_type: "image/avif".to_string(),
                etag: "blake3:abc123def4567890abc123def4567890".to_string(),
                bytes: vec![1, 2, 3, 4],
            }),
        });
        let app = Router::new().route(
            "/images/titles/{title_id}/{kind}/{variant}",
            get(title_image_handler).with_state(repo),
        );

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/images/titles/title-1/poster/w500?v=abc123def4567890")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "image/avif"
        );
        assert_eq!(
            response.headers().get(header::ETAG).unwrap(),
            "\"abc123def4567890abc123def4567890\""
        );
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL).unwrap(),
            "public, max-age=31536000, immutable"
        );
    }

    #[tokio::test]
    async fn title_image_route_revalidates_unversioned_or_mismatched_variant_urls() {
        let repo: Arc<dyn TitleImageRepository> = Arc::new(MockTitleImageRepository {
            blob: Some(TitleImageBlob {
                content_type: "image/avif".to_string(),
                etag: "blake3:abc123def4567890abc123def4567890".to_string(),
                bytes: vec![1, 2, 3, 4],
            }),
        });
        let app = Router::new().route(
            "/images/titles/{title_id}/{kind}/{variant}",
            get(title_image_handler).with_state(repo),
        );

        for uri in [
            "/images/titles/title-1/poster/w70",
            "/images/titles/title-1/poster/w70?v=w250digest",
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(uri)
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response");

            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.headers().get(header::CACHE_CONTROL).unwrap(),
                "public, max-age=0, must-revalidate"
            );
        }
    }

    #[tokio::test]
    async fn title_image_route_returns_not_found_for_missing_images() {
        let repo: Arc<dyn TitleImageRepository> = Arc::new(MockTitleImageRepository::default());
        let app = Router::new().route(
            "/images/titles/{title_id}/{kind}/{variant}",
            get(title_image_handler).with_state(repo),
        );

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/images/titles/title-1/poster/w500")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let app = Router::new().route(
            "/images/titles/{title_id}/{kind}/{variant}",
            get(title_image_handler).with_state(Arc::new(MockTitleImageRepository::default())),
        );
        for uri in [
            "/images/titles/title-1/poster/original",
            "/images/titles/title-1/fanart/master",
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(uri)
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response");
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }
    }

    #[tokio::test]
    async fn title_image_route_returns_not_modified_for_matching_etag() {
        let repo: Arc<dyn TitleImageRepository> = Arc::new(MockTitleImageRepository {
            blob: Some(TitleImageBlob {
                content_type: "image/avif".to_string(),
                etag: "abc123".to_string(),
                bytes: vec![1, 2, 3, 4],
            }),
        });
        let app = Router::new().route(
            "/images/titles/{title_id}/{kind}/{variant}",
            get(title_image_handler).with_state(repo),
        );

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/images/titles/title-1/poster/w500?v=abc123")
                    .header(header::IF_NONE_MATCH, "\"abc123\"")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(response.headers().get(header::ETAG).unwrap(), "\"abc123\"");
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL).unwrap(),
            "public, max-age=31536000, immutable"
        );
    }

    #[tokio::test]
    async fn title_image_route_serves_under_prefixed_base_path() {
        let repo: Arc<dyn TitleImageRepository> = Arc::new(MockTitleImageRepository {
            blob: Some(TitleImageBlob {
                content_type: "image/avif".to_string(),
                etag: "abc123".to_string(),
                bytes: vec![1, 2, 3, 4],
            }),
        });
        let app = mount_router(
            Router::new().route(
                "/images/titles/{title_id}/{kind}/{variant}",
                get(title_image_handler).with_state(repo),
            ),
            &BasePath::from_raw(Some("/scryer/")),
        );

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/scryer/images/titles/title-1/poster/w500")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
    }
}

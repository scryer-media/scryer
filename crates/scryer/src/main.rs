mod admin_routes;
mod base_path;
mod dev_seed;
mod init;
mod log_buffer;
mod middleware;
mod settings_bootstrap;
mod splash;
mod ui_assets;

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use axum::Router;
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use scryer_application::{
    AppServices, AppUseCase, DownloadClientPluginProvider, FacetRegistry, IndexerPluginProvider,
    MovieFacetHandler, SeriesFacetHandler, TitleImageKind, TitleImageRepository,
    start_background_acquisition_poller, start_background_banner_loop,
    start_background_fanart_loop, start_background_hydration_loop, start_background_poster_loop,
    start_background_subtitle_poller, start_download_queue_poller, start_notification_dispatcher,
};
use scryer_infrastructure::{
    FileSystemLibraryRenamer, FileSystemLibraryScanner, MetadataGatewayClient, MigrationMode,
    MultiIndexerSearchClient, NzbgetDownloadClient, PrioritizedDownloadClientRouter,
    SmgEnrollmentConfig, SqliteServices, SqliteTitleImageProcessor, WeaverDownloadClient,
    start_weaver_subscription_bridge,
};
use scryer_interface::{LogBuffer, build_schema_with_log_buffer};
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tower_http::compression::CompressionLayer;

use admin_routes::{
    AdminSettingsQuery, admin_migrations_handler, admin_settings_list, bootstrap_admin_password,
    seed_indexer_configs_from_env,
};
use base_path::BasePath;
use middleware::{
    AuthState, CorsConfig, cors_handler, graphiql_handler, graphql_handler, graphql_ws_handler,
    health_handler,
};
use settings_bootstrap::{
    MOVIES_PATH_KEY, SERIES_PATH_KEY, extract_pending_migration_ids, load_service_runtime_settings,
    migrate_legacy_download_client_default_category_settings,
    migrate_legacy_download_client_routing_settings, normalize_media_path_setting,
    normalize_quality_profile_settings, parse_migration_mode, seed_service_setting_definitions,
    seed_service_settings_from_environment,
};
use splash::{BootstrapStatus, SplashState, build_splash_router};
use ui_assets::{UiAssetMode, ui_asset_mode, ui_fallback};

include!(concat!(env!("OUT_DIR"), "/smg_build_assets.rs"));

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AuthModeConfig {
    auth_enabled: bool,
    used_legacy_dev_auto_login: bool,
}

#[tokio::main]
async fn main() {
    // Handle CLI subcommands before any startup work
    if let Some(arg) = std::env::args().nth(1) {
        match arg.as_str() {
            "init" => {
                init::run_init(std::env::args().collect());
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
                eprintln!("usage: scryer [init | --generate-key | --version]");
                std::process::exit(1);
            }
        }
    }

    load_env_file();

    // Install ring as the default rustls crypto provider (needed for TLS support)
    let _ = rustls::crypto::ring::default_provider().install_default();

    let db_path = std::env::var("SCRYER_DB_PATH")
        .unwrap_or_else(|_| "sqlite://file::memory:?mode=memory&cache=shared".to_string());
    let jwt_issuer = std::env::var("SCRYER_JWT_ISSUER").unwrap_or_else(|_| "scryer".to_string());
    let jwt_access_ttl_seconds = parse_env_u64("SCRYER_JWT_ACCESS_TTL_SECONDS", 86_400);
    let migration_mode = parse_migration_mode(std::env::var("SCRYER_DB_MIGRATION_MODE").ok());
    let bind = std::env::var("SCRYER_BIND").unwrap_or_else(|_| "127.0.0.1:8080".to_string());
    let base_path = BasePath::from_env();

    let log_ring_buffer = log_buffer::LogRingBuffer::with_default_capacity();

    {
        use tracing_subscriber::layer::SubscriberExt;
        use tracing_subscriber::util::SubscriberInitExt;

        let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

        let stdout_layer = tracing_subscriber::fmt::layer();
        let buffer_layer = tracing_subscriber::fmt::layer()
            .with_writer(log_buffer::LogBufferWriter::new(log_ring_buffer.clone()))
            .with_ansi(false);

        tracing_subscriber::registry()
            .with(env_filter)
            .with(stdout_layer)
            .with(buffer_layer)
            .init();
    }

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
        run_validate_only(&db_path, migration_mode).await;
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
    } else {
        tracing::info!(origins = ?cors.allowed_origins, "CORS configured with explicit origin list");
    }

    let addr: SocketAddr = bind.parse().expect("invalid bind address");
    let shutdown_token = CancellationToken::new();
    let startup_base_path = base_path.clone();

    // Spawn the full application bootstrap in the background.
    let bootstrap_shutdown = shutdown_token.clone();
    let bootstrap_bind = bind.clone();
    tokio::spawn(async move {
        match bootstrap_application(
            db_path,
            migration_mode,
            jwt_issuer,
            jwt_access_ttl_seconds,
            bootstrap_bind,
            cors,
            bootstrap_shutdown,
            log_ring_buffer,
            metrics_handle,
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
            tracing::info!(
                "open the web UI at https://{addr}{}",
                startup_base_path.ui_root()
            );
            if let Err(error) = axum_server::bind_rustls(addr, rustls_config)
                .handle(handle)
                .serve(splash_app.into_make_service())
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
            tracing::info!(
                "open the web UI at http://{addr}{}",
                startup_base_path.ui_root()
            );
            if let Err(error) = axum::serve(listener, splash_app)
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
    db_path: String,
    migration_mode: MigrationMode,
    jwt_issuer: String,
    jwt_access_ttl_seconds: u64,
    bind: String,
    cors: CorsConfig,
    shutdown_token: CancellationToken,
    log_ring_buffer: log_buffer::LogRingBuffer,
    metrics_handle: Option<metrics_exporter_prometheus::PrometheusHandle>,
) -> Result<Router, Box<dyn std::error::Error + Send + Sync>> {
    let bootstrap_start = std::time::Instant::now();

    let t = std::time::Instant::now();
    let db = SqliteServices::new_with_mode(db_path.clone(), migration_mode)
        .await
        .map_err(|e| format!("failed to initialize sqlite services: {e}"))?;
    tracing::info!(elapsed_ms = %t.elapsed().as_millis(), "database initialized");

    let t = std::time::Instant::now();
    seed_service_setting_definitions(&db)
        .await
        .map_err(|e| format!("failed to seed service setting definitions: {e}"))?;
    tracing::info!(elapsed_ms = %t.elapsed().as_millis(), "setting definitions seeded");

    // Bootstrap encryption master key (env > DB > auto-generate).
    // This runs before set_encryption_key so the master key itself is stored unencrypted.
    let t = std::time::Instant::now();
    let encryption_key = scryer_infrastructure::encryption::ensure_encryption_key(&db)
        .await
        .map_err(|e| format!("failed to ensure encryption master key: {e}"))?;

    // Activate encryption for all subsequent DB operations
    db.set_encryption_key(encryption_key)
        .await
        .map_err(|e| format!("failed to set encryption key on DB worker: {e}"))?;
    tracing::info!(elapsed_ms = %t.elapsed().as_millis(), "encryption bootstrapped");

    // Detect version upgrades by comparing with last-run version stored in DB
    check_version_upgrade(&db).await;

    let t = std::time::Instant::now();
    if let Err(error) = seed_service_settings_from_environment(&db).await {
        tracing::warn!(
            error = %error,
            "failed to persist optional settings from environment"
        );
    }
    if let Err(error) = migrate_legacy_download_client_routing_settings(&db).await {
        tracing::warn!(
            error = %error,
            "failed to migrate legacy download client routing settings during bootstrap"
        );
    }

    if let Err(error) = migrate_legacy_download_client_default_category_settings(&db).await {
        tracing::warn!(
            error = %error,
            "failed to migrate legacy download client default category settings during bootstrap"
        );
    }
    tracing::info!(elapsed_ms = %t.elapsed().as_millis(), "environment settings synced");

    let t = std::time::Instant::now();
    if let Err(error) = normalize_media_path_setting(&db, MOVIES_PATH_KEY).await {
        tracing::warn!(
            error = %error,
            "failed to normalize media movies.path setting during bootstrap"
        );
    }

    if let Err(error) = normalize_media_path_setting(&db, SERIES_PATH_KEY).await {
        tracing::warn!(
            error = %error,
            "failed to normalize media series.path setting during bootstrap"
        );
    }

    // Construct the facet registry early so scope IDs are available for settings bootstrap.
    let mut registry = FacetRegistry::new();
    registry.register(Arc::new(MovieFacetHandler));
    registry.register(Arc::new(SeriesFacetHandler::new(
        scryer_domain::MediaFacet::Tv,
    )));
    registry.register(Arc::new(SeriesFacetHandler::new(
        scryer_domain::MediaFacet::Anime,
    )));
    let facet_registry = Arc::new(registry);

    if let Err(error) = normalize_quality_profile_settings(&db, &facet_registry.facet_ids()).await {
        tracing::warn!(
            error = %error,
            "failed to normalize quality profile settings during bootstrap"
        );
    }
    tracing::info!(elapsed_ms = %t.elapsed().as_millis(), "settings normalized");

    // Bootstrap JWT HMAC secret (env > DB > auto-generate, persisted)
    let t = std::time::Instant::now();
    let env_jwt_secret = normalize_env_option("SCRYER_JWT_HMAC_SECRET");
    let jwt_hmac_secret =
        scryer_infrastructure::jwt_keys::ensure_jwt_hmac_secret(&db, env_jwt_secret)
            .await
            .map_err(|e| format!("failed to ensure JWT HMAC secret: {e}"))?;
    tracing::info!(elapsed_ms = %t.elapsed().as_millis(), "JWT secret bootstrapped");

    let t = std::time::Instant::now();
    let runtime_settings = load_service_runtime_settings(&db)
        .await
        .map_err(|e| format!("failed to load service runtime settings: {e}"))?;
    tracing::info!(elapsed_ms = %t.elapsed().as_millis(), "runtime settings loaded");

    tracing::info!(elapsed_ms = %bootstrap_start.elapsed().as_millis(), "bootstrap complete");

    let titles = Arc::new(db.clone());
    let users = Arc::new(db.clone());
    let events = Arc::new(db.clone());
    let shows = Arc::new(db.clone());
    let indexer_configs: Arc<dyn scryer_application::IndexerConfigRepository> =
        Arc::new(db.clone());
    let release_attempts = Arc::new(db.clone());
    let download_client_configs = Arc::new(db.clone());
    let settings_for_router: Arc<dyn scryer_application::SettingsRepository> = Arc::new(db.clone());
    let download_client_plugin_provider: Arc<dyn DownloadClientPluginProvider> =
        Arc::new(scryer_plugins::DynamicDownloadClientPluginProvider::new(
            scryer_plugins::WasmDownloadClientPluginProvider::empty(),
        ));
    let fallback_download_client = Arc::new(NzbgetDownloadClient::new(
        runtime_settings.nzbget_url,
        runtime_settings.nzbget_username,
        runtime_settings.nzbget_password,
        runtime_settings.nzbget_dupe_mode,
    ));
    let download_client = Arc::new(PrioritizedDownloadClientRouter::new(
        download_client_configs.clone(),
        settings_for_router,
        fallback_download_client,
        Some(download_client_plugin_provider.clone()),
    ));
    let indexer_stats: Arc<dyn scryer_application::IndexerStatsTracker> =
        Arc::new(scryer_infrastructure::InMemoryIndexerStatsTracker::new());

    // Load WASM indexer plugins: external plugins dir first, then built-in plugins.
    // Built-in plugins (nzbgeek, newznab) are always available; external plugins
    // with the same provider_type override the built-in.
    let plugins_dir = std::env::var("SCRYER_PLUGINS_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::path::Path::new(&db_path)
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .join("plugins")
        });
    let external_provider = if plugins_dir.is_dir() {
        match scryer_plugins::load_indexer_plugins(&plugins_dir) {
            Ok(provider) => {
                let types = provider.available_provider_types();
                if !types.is_empty() {
                    tracing::info!(
                        plugins_dir = %plugins_dir.display(),
                        provider_types = ?types,
                        "loaded external WASM indexer plugins"
                    );
                }
                provider
            }
            Err(e) => {
                tracing::warn!(
                    plugins_dir = %plugins_dir.display(),
                    error = %e,
                    "failed to load external WASM indexer plugins"
                );
                scryer_plugins::WasmIndexerPluginProvider::empty()
            }
        }
    } else {
        scryer_plugins::WasmIndexerPluginProvider::empty()
    };
    let initial_provider = external_provider
        .with_builtin(scryer_plugins::builtins::NZBGEEK_WASM)
        .with_builtin(scryer_plugins::builtins::NEWZNAB_WASM);
    let dynamic_provider = scryer_plugins::DynamicPluginProvider::new(initial_provider);
    let plugin_provider: Arc<dyn IndexerPluginProvider> = Arc::new(dynamic_provider);

    let indexer_client = MultiIndexerSearchClient::new(
        indexer_configs.clone(),
        indexer_stats.clone(),
        plugin_provider.clone(),
    );

    let indexer_client = Arc::new(indexer_client);
    let title_image_processor = Arc::new(SqliteTitleImageProcessor::new());
    let title_images_for_route: Arc<dyn TitleImageRepository> = Arc::new(db.clone());
    let metadata_gateway_url = std::env::var("SCRYER_METADATA_GATEWAY_GRAPHQL_URL")
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| SMG_GRAPHQL_URL.map(String::from))
        .unwrap_or_else(|| "http://127.0.0.1:8090/graphql".to_string());
    // TODO: Remove SCRYER_METADATA_GATEWAY_INSECURE once the gateway has proper TLS certificates.
    let metadata_gateway_insecure = std::env::var("SCRYER_METADATA_GATEWAY_INSECURE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let smg_registration_secret = SMG_REGISTRATION_SECRET
        .map(String::from)
        .or_else(|| std::env::var("SCRYER_SMG_REGISTRATION_SECRET").ok())
        .filter(|s| !s.is_empty());
    let smg_ca_cert = SMG_CA_CERT
        .map(String::from)
        .or_else(|| std::env::var("SCRYER_SMG_CA_CERT").ok())
        .filter(|s| !s.is_empty());
    let metadata_gateway = Arc::new(MetadataGatewayClient::new(
        metadata_gateway_url,
        metadata_gateway_insecure,
        db.clone(),
        SmgEnrollmentConfig {
            registration_secret: smg_registration_secret,
            ca_cert: smg_ca_cert,
        },
    ));
    let library_scanner = Arc::new(FileSystemLibraryScanner::new());
    let library_renamer = Arc::new(FileSystemLibraryRenamer::new());

    let mut services = AppServices::with_default_channels(
        titles,
        shows,
        users,
        events,
        indexer_configs,
        indexer_client,
        download_client,
        download_client_configs,
        release_attempts,
        Arc::new(db.clone()),
        Arc::new(db.clone()),
        db_path.clone(),
    );
    services.metadata_gateway = metadata_gateway.clone();

    // Warm up SMG enrollment so the mTLS client is ready before the first real
    // metadata query, and check for version incompatibility.
    tokio::spawn(async move {
        if let Some(incompat) = metadata_gateway.warm_enrollment().await {
            let env = if std::path::Path::new("/.dockerenv").exists() {
                "docker"
            } else {
                "binary"
            };
            tracing::error!(
                minimum_version = %incompat.minimum_version,
                your_version = %incompat.your_version,
                "INCOMPATIBLE VERSION: {}",
                incompat.message
            );
            if env == "docker" {
                tracing::error!(
                    "To upgrade, pull the latest image and restart:\n  docker pull ghcr.io/scryer-media/scryer:latest\n  docker compose up -d"
                );
            } else {
                tracing::error!(
                    "Download the latest release from:\n  https://github.com/scryer-media/scryer/releases/latest"
                );
            }
        }
    });

    services.library_scanner = library_scanner;
    services.library_renamer = library_renamer;
    services.download_submissions = Arc::new(db.clone());
    services.imports = Arc::new(db.clone());
    services.file_importer = Arc::new(scryer_infrastructure::FsFileImporter::new());
    services.media_files = Arc::new(db.clone());
    services.wanted_items = Arc::new(db.clone());
    services.pending_releases = Arc::new(db.clone());
    services.title_history = Arc::new(db.clone());
    services.blocklist_repo = Arc::new(db.clone());
    services.rule_sets = Arc::new(db.clone());
    services.pp_scripts = Arc::new(db.clone());
    services.plugin_installations = Arc::new(db.clone());
    services.system_info = Arc::new(db.clone());
    services.title_images = Arc::new(db.clone());
    services.title_image_processor = title_image_processor;
    services.housekeeping = Arc::new(db.clone());
    services.subtitle_downloads = Arc::new(db.clone());
    services.indexer_stats = indexer_stats;
    services.plugin_provider = Some(plugin_provider);
    services.download_client_plugin_provider = Some(download_client_plugin_provider.clone());
    services.notification_channels = Some(Arc::new(db.clone()));
    services.notification_subscriptions = Some(Arc::new(db.clone()));

    // Load notification WASM plugins (same pattern as indexer plugins)
    let notif_provider = scryer_plugins::DynamicNotificationPluginProvider::new(
        scryer_plugins::WasmNotificationPluginProvider::empty(),
    );
    services.notification_provider = Some(Arc::new(notif_provider));

    let app_use_case = AppUseCase::new(
        services,
        scryer_application::JwtAuthConfig {
            issuer: jwt_issuer,
            access_ttl_seconds: jwt_access_ttl_seconds as usize,
            jwt_hmac_secret,
        },
        facet_registry,
    );

    // Seed built-in plugin rows and rebuild provider from DB state.
    // This ensures user enable/disable toggles are respected after restart.
    if let Err(e) = app_use_case.seed_builtin_plugins().await {
        tracing::warn!(error = %e, "failed to seed built-in plugin installations");
    }
    if let Err(e) = app_use_case.rebuild_plugin_provider().await {
        tracing::warn!(error = %e, "failed to rebuild plugin provider from DB state");
    }
    if let Err(e) = app_use_case.reconcile_indexer_configs().await {
        tracing::warn!(error = %e, "failed to reconcile indexer configs on startup");
    }
    if let Err(e) = app_use_case.refresh_plugin_registry_internal().await {
        tracing::warn!(error = %e, "failed to refresh plugin registry on startup");
    }

    let auth_mode = resolve_auth_mode_from_env();
    let log_buf_snapshot = log_ring_buffer.clone();
    let log_buf_subscribe = log_ring_buffer.clone();
    let schema = build_schema_with_log_buffer(
        app_use_case.clone(),
        db.clone(),
        auth_mode.auth_enabled,
        Some(LogBuffer::new(
            move |limit| log_buf_snapshot.snapshot(limit),
            move || log_buf_subscribe.subscribe(),
        )),
    );
    // Use push-based WebSocket subscription for weaver; fall back to HTTP
    // polling for NZBGet/SABnzbd.
    match resolve_weaver_ws_url(&app_use_case).await {
        Some((ws_url, api_key)) => {
            tracing::info!(
                url = ws_url.as_str(),
                "using weaver subscription bridge for download queue"
            );
            tokio::spawn(start_weaver_subscription_bridge(
                app_use_case.clone(),
                shutdown_token.child_token(),
                ws_url,
                api_key,
            ));
        }
        None => {
            tokio::spawn(start_download_queue_poller(
                app_use_case.clone(),
                shutdown_token.child_token(),
            ));
        }
    }
    tokio::spawn(start_background_acquisition_poller(
        app_use_case.clone(),
        shutdown_token.child_token(),
    ));
    tokio::spawn(start_background_hydration_loop(
        app_use_case.clone(),
        shutdown_token.child_token(),
    ));
    tokio::spawn(start_background_poster_loop(
        app_use_case.clone(),
        shutdown_token.child_token(),
    ));
    tokio::spawn(start_background_banner_loop(
        app_use_case.clone(),
        shutdown_token.child_token(),
    ));
    tokio::spawn(start_background_fanart_loop(
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
    app_use_case.services.poster_wake.notify_one();
    app_use_case.services.banner_wake.notify_one();
    app_use_case.services.fanart_wake.notify_one();

    if let Err(error) = seed_indexer_configs_from_env(&app_use_case).await {
        tracing::warn!(error = %error, "failed to seed indexer configs from environment");
    }

    if let Err(error) = dev_seed::apply_dev_seed(&app_use_case, &db).await {
        tracing::warn!(error = %error, "failed to apply dev seed file");
    }
    if auth_mode.used_legacy_dev_auto_login {
        tracing::warn!(
            "SCRYER_DEV_AUTO_LOGIN is deprecated; use SCRYER_AUTH_ENABLED=false instead"
        );
    }
    if auth_mode.auth_enabled {
        tracing::info!("running with authentication enabled");
        bootstrap_admin_password(&app_use_case).await;
    } else {
        let addr: SocketAddr = bind.parse().expect("invalid bind address");
        if !addr.ip().is_loopback() && !addr.ip().is_unspecified() {
            tracing::warn!(
                bind = %bind,
                "authentication is disabled on a non-loopback bind address; all requests will act as admin"
            );
        }
        tracing::warn!("running with authentication disabled; all requests act as admin");
    }

    let auth_state = AuthState {
        app: app_use_case.clone(),
        schema: schema.clone(),
        auth_enabled: auth_mode.auth_enabled,
    };

    let cors_for_layer = cors.clone();
    let admin_migrations_db = db.clone();
    let admin_settings_db = db.clone();
    let admin_settings_app = app_use_case.clone();
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
        .route("/graphiql", get(graphiql_handler))
        .route("/graphql", post(graphql_handler).with_state(auth_state))
        .route(
            "/images/titles/{title_id}/{kind}/{variant}",
            get(title_image_handler).with_state(title_images_for_route),
        )
        .route(
            "/admin/migrations",
            get(move || admin_migrations_handler(admin_migrations_db.clone())),
        )
        .route(
            "/admin/settings",
            get(
                move |headers: HeaderMap, Query(query): Query<AdminSettingsQuery>| {
                    admin_settings_list(
                        admin_settings_db.clone(),
                        admin_settings_app.clone(),
                        auth_mode.auth_enabled,
                        headers,
                        query,
                    )
                },
            ),
        )
        .fallback(get(ui_fallback))
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

async fn title_image_handler(
    State(repository): State<Arc<dyn TitleImageRepository>>,
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

    let quoted_etag = format!("\"{}\"", blob.etag);
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| if_none_match_matches(value, &quoted_etag, &blob.etag))
    {
        let mut response = StatusCode::NOT_MODIFIED.into_response();
        let headers = response.headers_mut();
        if let Ok(value) = HeaderValue::from_str(&quoted_etag) {
            headers.insert(header::ETAG, value);
        }
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=31536000, immutable"),
        );
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
    if let Ok(value) = HeaderValue::from_str(&format!("\"{}\"", blob.etag)) {
        headers.insert(header::ETAG, value);
    }
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );
    response
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
async fn run_validate_only(db_path: &str, migration_mode: MigrationMode) {
    match SqliteServices::new_with_mode(db_path, migration_mode).await {
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

fn load_env_file() {
    // Load in reverse priority order: dotenvy skips vars already set, so the
    // last file loaded has lowest priority.  Load the crate-local file first
    // (higher priority), then the repo-root file (lower priority / template).
    let candidates = ["crates/scryer/.env", ".env"];
    let mut loaded = false;
    for candidate in candidates {
        if Path::new(candidate).exists() {
            let _ = dotenvy::from_path(candidate);
            loaded = true;
        }
    }
    if !loaded {
        let _ = dotenvy::dotenv();
    }
}

pub(crate) fn normalize_env_option(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn parse_env_bool_value(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "y" | "on" => Some(true),
        "0" | "false" | "no" | "n" | "off" => Some(false),
        _ => None,
    }
}

fn resolve_auth_mode(
    auth_enabled_raw: Option<&str>,
    legacy_dev_auto_login_raw: Option<&str>,
) -> AuthModeConfig {
    if let Some(auth_enabled) = auth_enabled_raw.and_then(parse_env_bool_value) {
        return AuthModeConfig {
            auth_enabled,
            used_legacy_dev_auto_login: false,
        };
    }

    let used_legacy_dev_auto_login = matches!(
        legacy_dev_auto_login_raw.and_then(parse_env_bool_value),
        Some(true)
    );

    AuthModeConfig {
        auth_enabled: false,
        used_legacy_dev_auto_login,
    }
}

fn resolve_auth_mode_from_env() -> AuthModeConfig {
    resolve_auth_mode(
        normalize_env_option("SCRYER_AUTH_ENABLED").as_deref(),
        normalize_env_option("SCRYER_DEV_AUTO_LOGIN").as_deref(),
    )
}

fn parse_env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

async fn check_version_upgrade(db: &SqliteServices) {
    const SCOPE: &str = "system";
    const KEY: &str = "last_run_version";

    let previous = db
        .get_setting_with_defaults(SCOPE, KEY, None)
        .await
        .ok()
        .flatten()
        .and_then(|r| r.value_json)
        .and_then(|v| serde_json::from_str::<String>(&v).ok());

    match previous.as_deref() {
        Some(prev) if prev == VERSION => {
            tracing::debug!(version = VERSION, "version unchanged");
        }
        Some(prev) => {
            tracing::info!(
                previous_version = prev,
                current_version = VERSION,
                "upgraded from {prev} to {VERSION}"
            );
        }
        None => {
            tracing::info!(version = VERSION, "first run — recording version");
        }
    }

    let version_json = serde_json::to_string(VERSION).unwrap();
    if let Err(error) = db
        .upsert_setting_value(SCOPE, KEY, None, version_json, "system", None)
        .await
    {
        tracing::warn!(error = %error, "failed to persist last_run_version");
    }
}

#[cfg(test)]
mod tests {
    use super::{AuthModeConfig, resolve_auth_mode, title_image_handler};
    use std::sync::Arc;

    use crate::base_path::{BasePath, mount_router};
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use axum::routing::get;
    use scryer_application::{
        AppResult, TitleImageBlob, TitleImageKind, TitleImageReplacement, TitleImageRepository,
        TitleImageSyncTask,
    };
    use tower::ServiceExt;

    #[derive(Default)]
    struct MockTitleImageRepository {
        blob: Option<TitleImageBlob>,
    }

    #[async_graphql::async_trait::async_trait]
    impl TitleImageRepository for MockTitleImageRepository {
        async fn list_titles_requiring_image_refresh(
            &self,
            _kind: TitleImageKind,
            _limit: usize,
        ) -> AppResult<Vec<TitleImageSyncTask>> {
            Ok(Vec::new())
        }

        async fn replace_title_image(
            &self,
            _title_id: &str,
            _replacement: TitleImageReplacement,
        ) -> AppResult<()> {
            Ok(())
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
            resolve_auth_mode(None, None),
            AuthModeConfig {
                auth_enabled: false,
                used_legacy_dev_auto_login: false,
            }
        );
    }

    #[test]
    fn explicit_auth_enabled_wins() {
        assert_eq!(
            resolve_auth_mode(Some("true"), Some("true")),
            AuthModeConfig {
                auth_enabled: true,
                used_legacy_dev_auto_login: false,
            }
        );
    }

    #[test]
    fn explicit_auth_disabled_wins_over_legacy_alias() {
        assert_eq!(
            resolve_auth_mode(Some("false"), Some("true")),
            AuthModeConfig {
                auth_enabled: false,
                used_legacy_dev_auto_login: false,
            }
        );
    }

    #[test]
    fn legacy_dev_auto_login_disables_auth_when_new_flag_absent() {
        assert_eq!(
            resolve_auth_mode(None, Some("true")),
            AuthModeConfig {
                auth_enabled: false,
                used_legacy_dev_auto_login: true,
            }
        );
    }

    #[test]
    fn invalid_auth_flag_falls_back_to_default_disabled() {
        assert_eq!(
            resolve_auth_mode(Some("garbage"), None),
            AuthModeConfig {
                auth_enabled: false,
                used_legacy_dev_auto_login: false,
            }
        );
    }

    #[tokio::test]
    async fn title_image_route_serves_cached_bytes_with_headers() {
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
                    .uri("/images/titles/title-1/poster/w500")
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
        assert_eq!(response.headers().get(header::ETAG).unwrap(), "\"abc123\"");
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL).unwrap(),
            "public, max-age=31536000, immutable"
        );
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
                    .uri("/images/titles/title-1/poster/w500")
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

/// Check if the primary download client is weaver and return its WebSocket URL and API key.
async fn resolve_weaver_ws_url(app: &AppUseCase) -> Option<(String, Option<String>)> {
    let configs = app.services.download_client_configs.list(None).await.ok()?;
    let primary = configs
        .into_iter()
        .filter(|c| c.is_enabled)
        .min_by_key(|c| c.client_priority)?;

    if !primary.client_type.trim().eq_ignore_ascii_case("weaver") {
        return None;
    }

    let client = WeaverDownloadClient::from_config(&primary).ok()?;
    Some((client.ws_url(), client.api_key().map(str::to_string)))
}

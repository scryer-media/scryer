#![allow(dead_code)]

use std::sync::Arc;

use async_graphql_axum::GraphQLRequest;
use axum::Router;
use axum::extract::State;
use axum::routing::post;
use tokio::net::TcpListener;
use wiremock::MockServer;

use scryer_application::{
    AppServices, AppUseCase, FacetRegistry, IndexerPluginProvider, JwtAuthConfig,
    MovieFacetHandler, SeriesFacetHandler,
};
use scryer_infrastructure::{
    FileSystemLibraryScanner, FileSystemStagedNzbStore, MetadataGatewayClient,
    MultiIndexerSearchClient, NzbgetDownloadClient, SmgEnrollmentConfig, SqliteServices,
};
use scryer_interface::{ApiSchema, build_schema};

/// Shared integration-test context.
///
/// Boots wiremock servers for external APIs, in-memory SQLite, real
/// infrastructure clients pointed at wiremock, a full `AppUseCase`,
/// GraphQL schema, and an axum server on a random port.
pub struct TestContext {
    pub nzbget_server: MockServer,
    pub nzbgeek_server: MockServer,
    pub smg_server: MockServer,
    /// Base URL of the test axum server (e.g. `http://127.0.0.1:12345`).
    pub app_url: String,
    pub schema: ApiSchema,
    pub app: AppUseCase,
    pub db: SqliteServices,
    pub staged_nzb_store: Arc<FileSystemStagedNzbStore>,
    pub staged_nzb_dir: tempfile::TempDir,
}

impl TestContext {
    pub async fn new() -> Self {
        // Start wiremock mock servers for each external API
        let nzbget_server = MockServer::start().await;
        let nzbgeek_server = MockServer::start().await;
        let smg_server = MockServer::start().await;

        // In-memory SQLite with migrations applied
        let db = SqliteServices::new(":memory:")
            .await
            .expect("failed to create in-memory SQLite");
        let staged_nzb_dir = tempfile::TempDir::new().expect("failed to create staged nzb tempdir");
        let staged_nzb_store = Arc::new(
            FileSystemStagedNzbStore::new(staged_nzb_dir.path())
                .await
                .expect("failed to create staged nzb store"),
        );
        let staged_nzb_pipeline_limit = Arc::new(tokio::sync::Semaphore::new(4));

        // Real clients pointed at wiremock URLs
        let nzbget = NzbgetDownloadClient::with_staged_nzb_store(
            nzbget_server.uri(),
            Some("test-user".to_string()),
            Some("test-pass".to_string()),
            "SCORE".to_string(),
            staged_nzb_store.clone(),
            staged_nzb_pipeline_limit.clone(),
        );

        // Build indexer client backed by built-in WASM plugins (using DynamicPluginProvider
        // so reload_plugins works in integration tests)
        let plugin_provider: Arc<dyn IndexerPluginProvider> =
            Arc::new(scryer_plugins::DynamicPluginProvider::new(
                scryer_plugins::WasmIndexerPluginProvider::empty()
                    .with_builtin(scryer_plugins::builtins::NZBGEEK_WASM)
                    .with_builtin(scryer_plugins::builtins::NEWZNAB_WASM),
            ));
        let indexer_stats: Arc<dyn scryer_application::IndexerStatsTracker> = Arc::new(
            scryer_infrastructure::InMemoryIndexerStatsTracker::new(None),
        );
        let indexer_client = MultiIndexerSearchClient::new(
            Arc::new(db.clone()),
            indexer_stats.clone(),
            plugin_provider.clone(),
        );

        let metadata_gateway = MetadataGatewayClient::new(
            format!("{}/graphql", smg_server.uri()),
            true, // accept invalid certs (wiremock is plain HTTP)
            db.clone(),
            SmgEnrollmentConfig {
                registration_secret: None,
                ca_cert: None,
            },
        );

        // Build repository implementations — SqliteServices implements all repository traits
        let titles: Arc<dyn scryer_application::TitleRepository> = Arc::new(db.clone());
        let shows: Arc<dyn scryer_application::ShowRepository> = Arc::new(db.clone());
        let users: Arc<dyn scryer_application::UserRepository> = Arc::new(db.clone());
        let events: Arc<dyn scryer_application::EventRepository> = Arc::new(db.clone());
        let indexer_configs: Arc<dyn scryer_application::IndexerConfigRepository> =
            Arc::new(db.clone());
        let download_client_configs: Arc<dyn scryer_application::DownloadClientConfigRepository> =
            Arc::new(db.clone());
        let release_attempts: Arc<dyn scryer_application::ReleaseAttemptRepository> =
            Arc::new(db.clone());
        let settings: Arc<dyn scryer_application::SettingsRepository> = Arc::new(db.clone());
        let quality_profiles: Arc<dyn scryer_application::QualityProfileRepository> =
            Arc::new(db.clone());

        let mut services = AppServices::with_default_channels(
            titles,
            shows,
            users,
            events,
            indexer_configs,
            Arc::new(indexer_client),
            Arc::new(nzbget),
            download_client_configs,
            release_attempts,
            settings,
            quality_profiles,
            ":memory:".to_string(),
        );
        services.metadata_gateway = Arc::new(metadata_gateway);
        services.library_scanner = Arc::new(FileSystemLibraryScanner::new());
        services.media_files = Arc::new(db.clone());
        services.indexer_stats = indexer_stats;
        services.plugin_provider = Some(plugin_provider);
        services.plugin_installations = Arc::new(db.clone());
        services.rule_sets = Arc::new(db.clone());
        services.acquisition_state = Arc::new(db.clone());
        services.wanted_items = Arc::new(db.clone());
        services.download_submissions = Arc::new(db.clone());
        services.pending_releases = Arc::new(db.clone());
        services.pp_scripts = Arc::new(db.clone());
        services.staged_nzb_store = staged_nzb_store.clone();
        services.staged_nzb_pipeline_limit = staged_nzb_pipeline_limit;

        // Facet registry with all built-in facets
        let mut registry = FacetRegistry::new();
        registry.register(Arc::new(MovieFacetHandler));
        registry.register(Arc::new(SeriesFacetHandler::new(
            scryer_domain::MediaFacet::Series,
        )));
        registry.register(Arc::new(SeriesFacetHandler::new(
            scryer_domain::MediaFacet::Anime,
        )));
        let facet_registry = Arc::new(registry);

        let app = AppUseCase::new(
            services,
            JwtAuthConfig {
                issuer: "scryer-test".to_string(),
                access_ttl_seconds: 3600,
                jwt_signing_salt: "test-salt".to_string(),
            },
            facet_registry,
        );

        // Build the GraphQL schema with authentication disabled.
        let schema = build_schema(app.clone(), db.clone(), false);

        // Start axum server on a random port
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("failed to bind test server");
        let addr = listener.local_addr().expect("failed to get local addr");
        let app_url = format!("http://{addr}");

        let router = build_test_router(app.clone(), schema.clone());
        tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("test server failed");
        });

        Self {
            nzbget_server,
            nzbgeek_server,
            smg_server,
            app_url,
            schema,
            app,
            db,
            staged_nzb_store,
            staged_nzb_dir,
        }
    }

    /// URL for the GraphQL endpoint.
    pub fn graphql_url(&self) -> String {
        format!("{}/graphql", self.app_url)
    }

    /// Build a reqwest client suitable for hitting the test server.
    pub fn http_client(&self) -> reqwest::Client {
        reqwest::Client::builder()
            .build()
            .expect("failed to build reqwest client")
    }
}

/// Load a fixture file relative to the workspace `tests/fixtures/` directory.
pub fn load_fixture(path: &str) -> String {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let fixture_path = std::path::Path::new(manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests")
        .join("fixtures")
        .join(path);
    std::fs::read_to_string(&fixture_path)
        .unwrap_or_else(|e| panic!("failed to load fixture {}: {e}", fixture_path.display()))
}

/// Build a minimal axum router with a GraphQL endpoint and authentication disabled.
fn build_test_router(app: AppUseCase, schema: ApiSchema) -> Router {
    Router::new().route(
        "/graphql",
        post(test_graphql_handler).with_state((app, schema)),
    )
}

/// Minimal GraphQL handler that replicates auth-disabled default-user injection.
async fn test_graphql_handler(
    State((app, schema)): State<(AppUseCase, ApiSchema)>,
    req: GraphQLRequest,
) -> async_graphql_axum::GraphQLResponse {
    let user = app.find_or_create_default_user().await.ok();
    let mut request = req.into_inner();
    if let Some(u) = user {
        request = request.data(u);
    }
    schema.execute(request).await.into()
}

use std::sync::Arc;

use async_graphql_axum::GraphQLRequest;
use axum::extract::State;
use axum::routing::post;
use axum::Router;
use tokio::net::TcpListener;
use wiremock::MockServer;

use scryer_application::{
    AppServices, AppUseCase, FacetRegistry, JwtAuthConfig, MovieFacetHandler, SeriesFacetHandler,
};
use scryer_infrastructure::{
    MetadataGatewayClient, NzbGeekSearchClient, NzbgetDownloadClient, SmgEnrollmentConfig,
    SqliteServices,
};
use scryer_interface::{build_schema, ApiSchema};

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

        // Seed the JWT setting definition so ensure_jwt_keys can persist keys
        db.ensure_setting_definition(
            "security",
            "system",
            "jwt.private_key",
            "string",
            "\"\"",
            true,
            None,
        )
        .await
        .expect("failed to seed jwt setting definition");

        // Generate JWT key pair via the standard bootstrap path
        let (jwt_ec_private_pem, jwt_ec_public_pem) =
            scryer_infrastructure::jwt_keys::ensure_jwt_keys(&db, None)
                .await
                .expect("failed to generate JWT keys");

        // Real clients pointed at wiremock URLs
        let nzbget = NzbgetDownloadClient::new(
            nzbget_server.uri(),
            Some("test-user".to_string()),
            Some("test-pass".to_string()),
            "SCORE".to_string(),
        );

        let nzbgeek = NzbGeekSearchClient::new(
            Some("test-api-key".to_string()),
            Some(nzbgeek_server.uri()),
            0,  // no rate-limit delay in tests
            1,  // base backoff
            1,  // max backoff
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
            Arc::new(nzbgeek),
            Arc::new(nzbget),
            download_client_configs,
            release_attempts,
            settings,
            quality_profiles,
            ":memory:".to_string(),
        );
        services.metadata_gateway = Arc::new(metadata_gateway);
        services.rule_sets = Arc::new(db.clone());

        // Facet registry with all built-in facets
        let mut registry = FacetRegistry::new();
        registry.register(Arc::new(MovieFacetHandler));
        registry.register(Arc::new(SeriesFacetHandler::new(
            scryer_domain::MediaFacet::Tv,
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
                jwt_ec_private_pem,
                jwt_ec_public_pem,
            },
            facet_registry,
        );

        // Build the GraphQL schema with dev_auto_login enabled
        let schema = build_schema(app.clone(), db.clone(), true);

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

/// Build a minimal axum router with a GraphQL endpoint and dev_auto_login.
fn build_test_router(app: AppUseCase, schema: ApiSchema) -> Router {
    Router::new().route(
        "/graphql",
        post(test_graphql_handler).with_state((app, schema)),
    )
}

/// Minimal GraphQL handler that replicates dev_auto_login behavior.
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

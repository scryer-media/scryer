//! End-to-end integration test that runs the full search pipeline
//! (discovery → multi-indexer → WASM plugins → HTTP) and captures
//! every outbound URL from real plugin binaries.
//!
//! Run with: cargo nextest run -E 'test(multi_indexer_url_trace)' --success-output immediate

mod common;

use std::sync::Arc;

use scryer_application::{
    AppServices, AppUseCase, FacetRegistry, IndexerPluginProvider, JwtAuthConfig,
    MovieFacetHandler, SearchMode, SeriesFacetHandler,
};
use scryer_domain::{Entitlement, User};
use scryer_infrastructure::{
    FileSystemLibraryScanner, InMemoryIndexerStatsTracker, MultiIndexerSearchClient, SqliteServices,
};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOSHO_EMPTY: &str = "[]";
const NEWZNAB_EMPTY: &str = r#"{"channel":{"item":[]}}"#;

/// Build a full AppUseCase with AnimeTosho, NZBGeek, and Torznab plugins,
/// each backed by its own wiremock server. Creates indexer configs in SQLite
/// so the multi-indexer discovers them at search time.
async fn setup() -> (
    AppUseCase,
    User,
    MockServer, // tosho
    MockServer, // nzbgeek
    MockServer, // torznab
) {
    let tosho_server = MockServer::start().await;
    let nzbgeek_server = MockServer::start().await;
    let torznab_server = MockServer::start().await;

    // Mount catch-all empty responses
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string(TOSHO_EMPTY))
        .mount(&tosho_server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string(NEWZNAB_EMPTY))
        .mount(&nzbgeek_server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string(NEWZNAB_EMPTY))
        .mount(&torznab_server)
        .await;

    let db = SqliteServices::new(":memory:")
        .await
        .expect("in-memory SQLite");

    // Load all three indexer plugins
    let plugin_provider: Arc<dyn IndexerPluginProvider> =
        Arc::new(scryer_plugins::DynamicPluginProvider::new(
            scryer_plugins::WasmIndexerPluginProvider::empty()
                .with_builtin(scryer_plugins::builtins::ANIMETOSHO_WASM)
                .with_builtin(scryer_plugins::builtins::NZBGEEK_WASM)
                .with_builtin(scryer_plugins::builtins::TORZNAB_WASM),
        ));

    let indexer_stats: Arc<dyn scryer_application::IndexerStatsTracker> =
        Arc::new(InMemoryIndexerStatsTracker::new(None));

    let indexer_client = MultiIndexerSearchClient::new(
        Arc::new(db.clone()),
        indexer_stats.clone(),
        plugin_provider.clone(),
    );

    // Create indexer configs in SQLite so the multi-indexer finds them
    use scryer_application::IndexerConfigRepository;
    let now = chrono::Utc::now();
    for config in [
        scryer_domain::IndexerConfig {
            id: "tosho-1".into(),
            name: "AnimeTosho".into(),
            provider_type: "animetosho".into(),
            base_url: tosho_server.uri(),
            api_key_encrypted: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            rate_limit_seconds: Some(0),
            rate_limit_burst: None,
            disabled_until: None,
            last_health_status: None,
            last_error_at: None,
            config_json: None,
            created_at: now,
            updated_at: now,
        },
        scryer_domain::IndexerConfig {
            id: "nzbgeek-1".into(),
            name: "NZBGeek".into(),
            provider_type: "nzbgeek".into(),
            base_url: format!("{}/api", nzbgeek_server.uri()),
            api_key_encrypted: Some("test-api-key".into()),
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            rate_limit_seconds: Some(0),
            rate_limit_burst: None,
            disabled_until: None,
            last_health_status: None,
            last_error_at: None,
            config_json: None,
            created_at: now,
            updated_at: now,
        },
        scryer_domain::IndexerConfig {
            id: "torznab-1".into(),
            name: "Torznab".into(),
            provider_type: "torznab".into(),
            base_url: format!("{}/api", torznab_server.uri()),
            api_key_encrypted: Some("test-api-key".into()),
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            rate_limit_seconds: Some(0),
            rate_limit_burst: None,
            disabled_until: None,
            last_health_status: None,
            last_error_at: None,
            config_json: None,
            created_at: now,
            updated_at: now,
        },
    ] {
        db.create(config).await.expect("create indexer config");
    }

    // Build a minimal download client so AppServices doesn't panic
    let nzbget = scryer_infrastructure::NzbgetDownloadClient::new(
        "http://localhost:1".to_string(),
        None,
        None,
        "SCORE".to_string(),
    );

    let smg = scryer_infrastructure::MetadataGatewayClient::new(
        "http://localhost:2/graphql".to_string(),
        true,
        db.clone(),
        scryer_infrastructure::SmgEnrollmentConfig {
            registration_secret: None,
            ca_cert: None,
        },
    );

    let titles: Arc<dyn scryer_application::TitleRepository> = Arc::new(db.clone());
    let shows: Arc<dyn scryer_application::ShowRepository> = Arc::new(db.clone());
    let users: Arc<dyn scryer_application::UserRepository> = Arc::new(db.clone());
    let events: Arc<dyn scryer_application::EventRepository> = Arc::new(db.clone());
    let indexer_configs_repo: Arc<dyn scryer_application::IndexerConfigRepository> =
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
        indexer_configs_repo,
        Arc::new(indexer_client),
        Arc::new(nzbget),
        download_client_configs,
        release_attempts,
        settings,
        quality_profiles,
        ":memory:".to_string(),
    );
    services.metadata_gateway = Arc::new(smg);
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

    let mut registry = FacetRegistry::new();
    registry.register(Arc::new(MovieFacetHandler));
    registry.register(Arc::new(SeriesFacetHandler::new(
        scryer_domain::MediaFacet::Series,
    )));
    registry.register(Arc::new(SeriesFacetHandler::new(
        scryer_domain::MediaFacet::Anime,
    )));

    let app = AppUseCase::new(
        services,
        JwtAuthConfig {
            issuer: "scryer-test".into(),
            access_ttl_seconds: 3600,
            jwt_signing_salt: "test-salt".into(),
        },
        Arc::new(registry),
    );

    // Create a test user with ViewCatalog entitlement
    let user = User {
        id: "test-user".into(),
        username: "tester".into(),
        password_hash: None,
        entitlements: vec![Entitlement::ViewCatalog],
    };

    (app, user, tosho_server, nzbgeek_server, torznab_server)
}

async fn captured_urls(server: &MockServer) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .map(|r| r.url.to_string())
        .collect()
}

fn print_urls(label: &str, urls: &[String]) {
    if urls.is_empty() {
        println!("  {label}: (no calls)");
    } else {
        for url in urls {
            println!("  {label}: {url}");
        }
    }
}

fn print_summary(tosho: &[String], nzbgeek: &[String], torznab: &[String]) {
    print_urls("AnimeTosho", tosho);
    print_urls("NZBGeek", nzbgeek);
    print_urls("Torznab", torznab);
    println!(
        "  Total HTTP calls: {}",
        tosho.len() + nzbgeek.len() + torznab.len()
    );
}

// ---------------------------------------------------------------------------
// Demon Slayer S02E03 — anime episode, end-to-end through discovery layer
// ---------------------------------------------------------------------------

#[tokio::test]
async fn multi_indexer_url_trace_anime_episode() {
    let (app, user, tosho, nzbgeek, torznab) = setup().await;

    let _results = app
        .search_indexers_episode(
            &user,
            "Demon Slayer".into(),
            "02".into(),
            "03".into(),
            None,                  // imdb_id
            Some("348545".into()), // tvdb_id
            Some("1535".into()),   // anidb_id
            Some("anime".into()),  // category
            None,                  // absolute_episode
        )
        .await
        .expect("search should succeed");

    println!("\n=== Demon Slayer S02E03 (anime, anidb=1535, tvdb=348545) ===");
    print_summary(
        &captured_urls(&tosho).await,
        &captured_urls(&nzbgeek).await,
        &captured_urls(&torznab).await,
    );
}

// ---------------------------------------------------------------------------
// Breaking Bad S05E01 — regular TV series
// ---------------------------------------------------------------------------

#[tokio::test]
async fn multi_indexer_url_trace_tv_episode() {
    let (app, user, tosho, nzbgeek, torznab) = setup().await;

    let _results = app
        .search_indexers_episode(
            &user,
            "Breaking Bad".into(),
            "05".into(),
            "01".into(),
            None,
            Some("81189".into()), // tvdb_id
            None,                 // anidb_id
            Some("series".into()),
            None,
        )
        .await
        .expect("search should succeed");

    println!("\n=== Breaking Bad S05E01 (series, tvdb=81189) ===");
    print_summary(
        &captured_urls(&tosho).await,
        &captured_urls(&nzbgeek).await,
        &captured_urls(&torznab).await,
    );
}

// ---------------------------------------------------------------------------
// The Matrix — movie with imdb_id only
// ---------------------------------------------------------------------------

#[tokio::test]
async fn multi_indexer_url_trace_movie() {
    let (app, user, tosho, nzbgeek, torznab) = setup().await;

    let _results = app
        .search_indexers(
            &user,
            "The Matrix".into(),
            Some("tt0133093".into()), // imdb_id
            None,                     // tvdb_id
            None,                     // anidb_id
            Some("movie".into()),
        )
        .await
        .expect("search should succeed");

    println!("\n=== The Matrix (movie, imdb=tt0133093) ===");
    print_summary(
        &captured_urls(&tosho).await,
        &captured_urls(&nzbgeek).await,
        &captured_urls(&torznab).await,
    );
}

// ---------------------------------------------------------------------------
// Spirited Away — movie with imdb_id + anidb_id (from metadata hydration)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn multi_indexer_url_trace_movie_spirited_away() {
    let (app, user, tosho, nzbgeek, torznab) = setup().await;

    let _results = app
        .search_indexers(
            &user,
            "Spirited Away".into(),
            Some("tt0245429".into()), // imdb_id
            None,                     // tvdb_id
            Some("112".into()),       // anidb_id
            Some("movie".into()),
        )
        .await
        .expect("search should succeed");

    println!("\n=== Spirited Away (movie, imdb=tt0245429, anidb=112) ===");
    print_summary(
        &captured_urls(&tosho).await,
        &captured_urls(&nzbgeek).await,
        &captured_urls(&torznab).await,
    );
}

// ---------------------------------------------------------------------------
// Demon Slayer Season 2 pack — background acquisition path
// (season=Some, episode=None, query=title name only)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn multi_indexer_url_trace_season_pack() {
    let (app, _user, tosho, nzbgeek, torznab) = setup().await;

    // Call the indexer client directly as the background acquisition loop would.
    // season=Some(2), episode=None signals a season pack search.
    // The acquisition loop builds "Title S02" as the query.
    let _results = app
        .services
        .indexer_client
        .search(
            "Demon Slayer S02".into(), // title + S02 for freetext matching
            std::collections::HashMap::from([
                ("tvdb_id".to_string(), "348545".to_string()),
                ("anidb_id".to_string(), "1535".to_string()),
            ]),
            Some("anime".into()), // category
            Some("anime".into()), // facet
            None,                 // newznab_categories
            None,                 // indexer_routing
            SearchMode::Auto,
            Some(2), // season
            None,    // episode=None → season pack
            None,    // absolute_episode
            vec![],
        )
        .await
        .expect("search should succeed");

    println!(
        "\n=== Demon Slayer Season 2 Pack (anime, tvdb=348545, anidb=1535, season=2, ep=None) ==="
    );
    print_summary(
        &captured_urls(&tosho).await,
        &captured_urls(&nzbgeek).await,
        &captured_urls(&torznab).await,
    );
}

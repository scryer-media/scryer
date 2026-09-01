#![recursion_limit = "256"]

mod common;

use std::collections::HashMap;
use std::sync::Arc;

use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, ResponseTemplate};

use common::{TestContext, load_fixture};
use scryer_application::{IndexerClient, IndexerPluginProvider, MetadataSearchQuery, SearchMode};
use scryer_domain::{LibraryPermissionMask, User, UserAuthorization};

fn admin() -> User {
    User {
        id: scryer_domain::Id::new().0,
        username: "admin".to_string(),
        password_hash: None,
        password_change_required: false,
        account_kind: Default::default(),
        authorization: UserAuthorization {
            actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
            loaded: true,
            default_library: LibraryPermissionMask::from_permissions([
                scryer_domain::LibraryPermission::View,
            ]),
            ..Default::default()
        },
    }
}

/// Create an IndexerClient backed by the built-in Newznab WASM plugin,
/// configured to talk to the given wiremock URI.
fn new_nzbgeek_client(uri: &str) -> Arc<dyn IndexerClient> {
    let provider = scryer_plugins::WasmIndexerPluginProvider::empty()
        .with_builtin_asset(scryer_plugins::builtins::NEWZNAB);
    let config = scryer_domain::IndexerConfig {
        id: "test-newznab".to_string(),
        name: "Test Newznab".to_string(),
        provider_type: "newznab".to_string(),
        base_url: uri.to_string(),
        api_key_encrypted: None,
        is_enabled: true,
        enable_interactive_search: true,
        enable_auto_search: true,
        indexer_proxy_config_id: None,
        download_client_id: None,
        seeding_profile_id: None,
        managed_parent_config_id: None,
        managed_child_key: None,
        managed_metadata_json: None,
        caps_snapshot_json: None,
        rate_limit_seconds: None,
        rate_limit_burst: None,
        disabled_until: None,
        last_health_status: None,
        last_error_message: None,
        last_error_at: None,
        config_json: Some(
            serde_json::json!({
                "base_url": uri,
                "api_key": "test-api-key",
            })
            .to_string(),
        ),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    provider
        .client_for_provider(&config)
        .expect("should create newznab WASM client")
}

fn search_ids(
    imdb_id: Option<&str>,
    tvdb_id: Option<&str>,
    anidb_id: Option<&str>,
) -> HashMap<String, String> {
    let mut ids = HashMap::new();
    if let Some(imdb_id) = imdb_id {
        ids.insert("imdb_id".to_string(), imdb_id.to_string());
    }
    if let Some(tvdb_id) = tvdb_id {
        ids.insert("tvdb_id".to_string(), tvdb_id.to_string());
    }
    if let Some(anidb_id) = anidb_id {
        ids.insert("anidb_id".to_string(), anidb_id.to_string());
    }
    ids
}

// ---------------------------------------------------------------------------
// Movie search
// ---------------------------------------------------------------------------

#[tokio::test]
async fn nzbgeek_search_movie_by_category() {
    let ctx = TestContext::new().await;
    Mock::given(method("GET"))
        .and(path("/api"))
        .and(query_param("apikey", "test-api-key"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("nzbgeek/search_movie.json")),
        )
        .mount(&ctx.nzbgeek_server)
        .await;

    let results = new_nzbgeek_client(&ctx.nzbgeek_server.uri())
        .search(
            "Test Movie".to_string(),
            search_ids(Some("tt1234567"), None, None),
            Some("movie".to_string()),
            Some("movie".to_string()),
            None,
            None,
            None,
            SearchMode::Interactive,
            scryer_application::IndexerErrorOperation::InteractiveSearch,
            None,
            None,
            None, // absolute_episode
            vec![],
            None,
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .expect("search should succeed")
        .results;

    // Verify the first request was a structured movie search with IMDB ID
    let requests = ctx
        .nzbgeek_server
        .received_requests()
        .await
        .expect("should capture search request");
    assert!(
        !requests.is_empty(),
        "at least one request should have been made"
    );
    let query: std::collections::HashMap<String, String> = requests[0]
        .url
        .query_pairs()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    assert_eq!(query.get("t").map(String::as_str), Some("movie"));
    assert_eq!(query.get("q").map(String::as_str), None);
    assert_eq!(query.get("imdbid").map(String::as_str), Some("001234567"));
    assert_eq!(query.get("o").map(String::as_str), Some("json"));
    assert_eq!(query.get("extended").map(String::as_str), Some("1"));

    assert_eq!(results.len(), 2);
    assert!(
        results[0].title.contains("2160p"),
        "first result should be 4K"
    );
}

#[tokio::test]
async fn nzbgeek_search_movie_extracts_size() {
    let ctx = TestContext::new().await;
    Mock::given(method("GET"))
        .and(path("/api"))
        .and(query_param("t", "movie"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("nzbgeek/search_movie.json")),
        )
        .mount(&ctx.nzbgeek_server)
        .await;

    let results = new_nzbgeek_client(&ctx.nzbgeek_server.uri())
        .search(
            "Test".to_string(),
            search_ids(None, None, None),
            Some("movie".to_string()),
            Some("movie".to_string()),
            None,
            None,
            None,
            SearchMode::Interactive,
            scryer_application::IndexerErrorOperation::InteractiveSearch,
            None,
            None,
            None, // absolute_episode
            vec![],
            None,
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .unwrap()
        .results;

    assert!(
        results[0].size_bytes.unwrap_or(0) > 0,
        "size_bytes should be parsed from enclosure length"
    );
}

#[tokio::test]
async fn nzbgeek_search_movie_extracts_download_url() {
    let ctx = TestContext::new().await;
    Mock::given(method("GET"))
        .and(path("/api"))
        .and(query_param("t", "movie"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("nzbgeek/search_movie.json")),
        )
        .mount(&ctx.nzbgeek_server)
        .await;

    let results = new_nzbgeek_client(&ctx.nzbgeek_server.uri())
        .search(
            "Test".to_string(),
            search_ids(None, None, None),
            Some("movie".to_string()),
            Some("movie".to_string()),
            None,
            None,
            None,
            SearchMode::Interactive,
            scryer_application::IndexerErrorOperation::InteractiveSearch,
            None,
            None,
            None, // absolute_episode
            vec![],
            None,
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .unwrap()
        .results;

    assert!(
        results[0].download_url.is_some(),
        "download_url should be extracted from enclosure"
    );
    assert!(
        results[0].download_url.as_ref().unwrap().contains("t=get"),
        "download_url should point to NZB endpoint"
    );
}

// ---------------------------------------------------------------------------
// Series search
// ---------------------------------------------------------------------------

#[tokio::test]
async fn nzbgeek_search_series_by_category() {
    let ctx = TestContext::new().await;
    Mock::given(method("GET"))
        .and(path("/api"))
        .and(query_param("t", "tvsearch"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("nzbgeek/search_tv.json")),
        )
        .mount(&ctx.nzbgeek_server)
        .await;

    let results = new_nzbgeek_client(&ctx.nzbgeek_server.uri())
        .search(
            "Test Show".to_string(),
            search_ids(None, Some("345678"), None),
            Some("series".to_string()),
            Some("series".to_string()),
            None,
            None,
            None,
            SearchMode::Interactive,
            scryer_application::IndexerErrorOperation::InteractiveSearch,
            None,
            None,
            None, // absolute_episode
            vec![],
            None,
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .expect("series search should succeed")
        .results;

    assert!(!results.is_empty(), "series search should return results");
}

#[tokio::test]
async fn nzbgeek_search_series_endpoint_by_anime_category() {
    let ctx = TestContext::new().await;
    // "anime" category should also use the structured episodic search endpoint.
    Mock::given(method("GET"))
        .and(path("/api"))
        .and(query_param("t", "tvsearch"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("nzbgeek/search_tv.json")),
        )
        .mount(&ctx.nzbgeek_server)
        .await;

    let results = new_nzbgeek_client(&ctx.nzbgeek_server.uri())
        .search(
            "Anime Title".to_string(),
            search_ids(None, Some("999"), None),
            Some("anime".to_string()),
            Some("anime".to_string()),
            None,
            None,
            None,
            SearchMode::Interactive,
            scryer_application::IndexerErrorOperation::InteractiveSearch,
            None,
            None,
            None, // absolute_episode
            vec![],
            None,
            tokio_util::sync::CancellationToken::new(),
        )
        .await;

    assert!(
        results.is_ok(),
        "anime category should use tvsearch: {:?}",
        results.err()
    );
}

#[tokio::test]
async fn nzbgeek_search_series_endpoint_by_series_category() {
    let ctx = TestContext::new().await;
    Mock::given(method("GET"))
        .and(path("/api"))
        .and(query_param("t", "tvsearch"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("nzbgeek/search_tv.json")),
        )
        .mount(&ctx.nzbgeek_server)
        .await;

    let results = new_nzbgeek_client(&ctx.nzbgeek_server.uri())
        .search(
            "Series Title".to_string(),
            search_ids(None, Some("123"), None),
            Some("series".to_string()),
            Some("series".to_string()),
            None,
            None,
            None,
            SearchMode::Interactive,
            scryer_application::IndexerErrorOperation::InteractiveSearch,
            None,
            None,
            None, // absolute_episode
            vec![],
            None,
            tokio_util::sync::CancellationToken::new(),
        )
        .await;

    assert!(results.is_ok(), "series category should use tvsearch");
}

// ---------------------------------------------------------------------------
// Search type inference (no explicit category)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn nzbgeek_search_infers_movie_from_imdb_id() {
    let ctx = TestContext::new().await;
    // Without category, imdb_id presence should trigger t=movie
    Mock::given(method("GET"))
        .and(path("/api"))
        .and(query_param("t", "movie"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("nzbgeek/search_movie.json")),
        )
        .mount(&ctx.nzbgeek_server)
        .await;

    let results = new_nzbgeek_client(&ctx.nzbgeek_server.uri())
        .search(
            "Test".to_string(),
            search_ids(Some("tt1234567"), None, None),
            None, // no category
            None,
            None,
            None,
            None,
            SearchMode::Interactive,
            scryer_application::IndexerErrorOperation::InteractiveSearch,
            None,
            None,
            None, // absolute_episode
            vec![],
            None,
            tokio_util::sync::CancellationToken::new(),
        )
        .await;

    assert!(
        results.is_ok(),
        "should infer movie from imdb_id: {:?}",
        results.err()
    );
}

#[tokio::test]
async fn nzbgeek_search_infers_series_endpoint_from_tvdb_id() {
    let ctx = TestContext::new().await;
    Mock::given(method("GET"))
        .and(path("/api"))
        .and(query_param("t", "tvsearch"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("nzbgeek/search_tv.json")),
        )
        .mount(&ctx.nzbgeek_server)
        .await;

    let results = new_nzbgeek_client(&ctx.nzbgeek_server.uri())
        .search(
            "Test".to_string(),
            search_ids(None, Some("345678"), None),
            None, // no category
            None,
            None,
            None,
            None,
            SearchMode::Interactive,
            scryer_application::IndexerErrorOperation::InteractiveSearch,
            None,
            None,
            None, // absolute_episode
            vec![],
            None,
            tokio_util::sync::CancellationToken::new(),
        )
        .await;

    assert!(
        results.is_ok(),
        "should infer tvsearch from tvdb_id: {:?}",
        results.err()
    );
}

#[tokio::test]
async fn nzbgeek_search_generic_without_ids() {
    let ctx = TestContext::new().await;
    // Without category or IDs, should use t=search
    Mock::given(method("GET"))
        .and(path("/api"))
        .and(query_param("t", "search"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("nzbgeek/search_movie.json")),
        )
        .mount(&ctx.nzbgeek_server)
        .await;

    let results = new_nzbgeek_client(&ctx.nzbgeek_server.uri())
        .search(
            "Test".to_string(),
            search_ids(None, None, None),
            None,
            None,
            None,
            None,
            None,
            SearchMode::Interactive,
            scryer_application::IndexerErrorOperation::InteractiveSearch,
            None,
            None,
            None, // absolute_episode
            vec![],
            None,
            tokio_util::sync::CancellationToken::new(),
        )
        .await;

    assert!(
        results.is_ok(),
        "generic search should work: {:?}",
        results.err()
    );
}

// ---------------------------------------------------------------------------
// Empty / missing results
// ---------------------------------------------------------------------------

#[tokio::test]
async fn nzbgeek_search_empty_results() {
    let ctx = TestContext::new().await;
    Mock::given(method("GET"))
        .and(path("/api"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("nzbgeek/search_empty.json")),
        )
        .mount(&ctx.nzbgeek_server)
        .await;

    let results = new_nzbgeek_client(&ctx.nzbgeek_server.uri())
        .search(
            "Nonexistent".to_string(),
            search_ids(None, None, None),
            Some("movie".to_string()),
            Some("movie".to_string()),
            None,
            None,
            None,
            SearchMode::Interactive,
            scryer_application::IndexerErrorOperation::InteractiveSearch,
            None,
            None,
            None, // absolute_episode
            vec![],
            None,
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .expect("empty search should succeed")
        .results;

    assert!(results.is_empty());
}

#[tokio::test]
async fn nzbgeek_search_single_item_response() {
    let ctx = TestContext::new().await;
    // API can return a single item as an object instead of an array
    Mock::given(method("GET"))
        .and(path("/api"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(load_fixture("nzbgeek/search_single_item.json")),
        )
        .mount(&ctx.nzbgeek_server)
        .await;

    let results = new_nzbgeek_client(&ctx.nzbgeek_server.uri())
        .search(
            "Test".to_string(),
            search_ids(None, None, None),
            Some("movie".to_string()),
            Some("movie".to_string()),
            None,
            None,
            None,
            SearchMode::Interactive,
            scryer_application::IndexerErrorOperation::InteractiveSearch,
            None,
            None,
            None, // absolute_episode
            vec![],
            None,
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .expect("single-item response should parse correctly")
        .results;

    assert_eq!(results.len(), 1, "should parse single item response");
    assert!(results[0].title.contains("2160p"));
}

// ---------------------------------------------------------------------------
// Error handling
// ---------------------------------------------------------------------------

#[tokio::test]
async fn nzbgeek_search_no_api_key_fails() {
    let ctx = TestContext::new().await;
    let provider = scryer_plugins::WasmIndexerPluginProvider::empty()
        .with_builtin_asset(scryer_plugins::builtins::NEWZNAB);
    let config = scryer_domain::IndexerConfig {
        id: "test-no-key".to_string(),
        name: "Test No Key".to_string(),
        provider_type: "newznab".to_string(),
        base_url: ctx.nzbgeek_server.uri(),
        api_key_encrypted: None,
        is_enabled: true,
        enable_interactive_search: true,
        enable_auto_search: true,
        indexer_proxy_config_id: None,
        download_client_id: None,
        seeding_profile_id: None,
        managed_parent_config_id: None,
        managed_child_key: None,
        managed_metadata_json: None,
        caps_snapshot_json: None,
        rate_limit_seconds: None,
        rate_limit_burst: None,
        disabled_until: None,
        last_health_status: None,
        last_error_message: None,
        last_error_at: None,
        config_json: Some(
            serde_json::json!({
                "base_url": ctx.nzbgeek_server.uri(),
            })
            .to_string(),
        ),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    let client = provider
        .client_for_provider(&config)
        .expect("should create client");

    let results = client
        .search(
            "Test".to_string(),
            search_ids(None, None, None),
            None,
            None,
            None,
            None,
            None,
            SearchMode::Interactive,
            scryer_application::IndexerErrorOperation::InteractiveSearch,
            None,
            None,
            None, // absolute_episode
            vec![],
            None,
            tokio_util::sync::CancellationToken::new(),
        )
        .await;

    assert!(results.is_err(), "should fail without API key");
}

#[tokio::test]
async fn nzbgeek_search_http_error() {
    let ctx = TestContext::new().await;
    Mock::given(method("GET"))
        .and(path("/api"))
        .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
        .mount(&ctx.nzbgeek_server)
        .await;

    let results = new_nzbgeek_client(&ctx.nzbgeek_server.uri())
        .search(
            "Test".to_string(),
            search_ids(None, None, None),
            Some("movie".to_string()),
            Some("movie".to_string()),
            None,
            None,
            None,
            SearchMode::Interactive,
            scryer_application::IndexerErrorOperation::InteractiveSearch,
            None,
            None,
            None, // absolute_episode
            vec![],
            None,
            tokio_util::sync::CancellationToken::new(),
        )
        .await;

    assert!(results.is_err(), "should fail on HTTP 401");
}

#[tokio::test]
async fn nzbgeek_search_rate_limited() {
    let ctx = TestContext::new().await;
    Mock::given(method("GET"))
        .and(path("/api"))
        .respond_with(
            ResponseTemplate::new(429)
                .set_body_string(load_fixture("nzbgeek/error_rate_limit.json"))
                .insert_header("Retry-After", "1"),
        )
        .mount(&ctx.nzbgeek_server)
        .await;

    let results = new_nzbgeek_client(&ctx.nzbgeek_server.uri())
        .search(
            "Test".to_string(),
            search_ids(None, None, None),
            Some("movie".to_string()),
            Some("movie".to_string()),
            None,
            None,
            None,
            SearchMode::Interactive,
            scryer_application::IndexerErrorOperation::InteractiveSearch,
            None,
            None,
            None, // absolute_episode
            vec![],
            None,
            tokio_util::sync::CancellationToken::new(),
        )
        .await;

    assert!(results.is_err(), "should fail on rate limit");
}

#[tokio::test]
async fn nzbgeek_search_server_error_is_deferred() {
    let ctx = TestContext::new().await;

    // Strategy orchestration owns generic fallback. The direct plugin client
    // instead reports a typed upstream deferral for a provider 500.
    // Keep a viable fallback response mounted so a client-side fallback would
    // turn this test red by returning results.
    Mock::given(method("GET"))
        .and(path("/api"))
        .and(query_param("t", "search"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("nzbgeek/search_movie.json")),
        )
        .mount(&ctx.nzbgeek_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/api"))
        .and(query_param("t", "movie"))
        .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
        .mount(&ctx.nzbgeek_server)
        .await;

    let results = new_nzbgeek_client(&ctx.nzbgeek_server.uri())
        .search(
            "Test Movie".to_string(),
            search_ids(Some("tt1234567"), None, None),
            Some("movie".to_string()),
            Some("movie".to_string()),
            None,
            None,
            None,
            SearchMode::Interactive,
            scryer_application::IndexerErrorOperation::InteractiveSearch,
            None,
            None,
            None, // absolute_episode
            vec![],
            None,
            tokio_util::sync::CancellationToken::new(),
        )
        .await;

    let error = results.expect_err("a provider 500 should defer the direct search");
    assert!(
        matches!(
            &error,
            scryer_application::AppError::TemporaryUnavailable {
                retry_after: None,
                ..
            }
        ),
        "expected a recoverable upstream deferral: {error:?}"
    );
    assert!(
        error.to_string().contains("UpstreamFailure"),
        "expected the upstream failure reason: {error}"
    );
}

#[tokio::test]
async fn nzbgeek_search_empty_query_and_no_ids_fails() {
    let ctx = TestContext::new().await;
    let client = new_nzbgeek_client(&ctx.nzbgeek_server.uri());

    let results = client
        .search(
            "".to_string(), // empty query
            search_ids(None, None, None),
            None,
            None,
            None,
            None,
            None,
            SearchMode::Interactive,
            scryer_application::IndexerErrorOperation::InteractiveSearch,
            None,
            None,
            None, // absolute_episode
            vec![],
            None,
            tokio_util::sync::CancellationToken::new(),
        )
        .await;

    // Should return empty results or error when no query/ids
    assert!(
        results.is_err() || results.unwrap().results.is_empty(),
        "empty query with no IDs should fail or return empty"
    );
}

// ---------------------------------------------------------------------------
// Metadata extraction
// ---------------------------------------------------------------------------

#[tokio::test]
async fn newznab_search_extracts_standard_metadata_attributes() {
    let ctx = TestContext::new().await;
    Mock::given(method("GET"))
        .and(path("/api"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(load_fixture("nzbgeek/search_single_item.json")),
        )
        .mount(&ctx.nzbgeek_server)
        .await;

    let results = new_nzbgeek_client(&ctx.nzbgeek_server.uri())
        .search(
            "Test".to_string(),
            search_ids(None, None, None),
            Some("movie".to_string()),
            Some("movie".to_string()),
            None,
            None,
            None,
            SearchMode::Interactive,
            scryer_application::IndexerErrorOperation::InteractiveSearch,
            None,
            None,
            None, // absolute_episode
            vec![],
            None,
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .unwrap()
        .results;

    let result = &results[0];
    assert_eq!(result.indexer_grabs, Some(128), "grabs should be parsed");
    assert!(
        result.indexer_languages.is_some(),
        "languages should be parsed"
    );
}

// ---------------------------------------------------------------------------
// MetadataGateway (SMG) client
// ---------------------------------------------------------------------------

fn is_search_titles_request(request: &wiremock::Request) -> bool {
    request
        .url
        .query_pairs()
        .any(|(name, value)| name == "operationName" && value == "SearchTitles")
        || request
            .body_json::<serde_json::Value>()
            .ok()
            .is_some_and(|body| {
                body.get("operationName")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|operation_name| operation_name == "SearchTitles")
            })
}

async fn mount_movie_search_metadata_mocks(ctx: &TestContext, legacy_fixture_path: &str) {
    let legacy_fixture = load_fixture(legacy_fixture_path);
    let titles_fixture = load_fixture("smg/search_titles.json");
    let get_titles_fixture = titles_fixture.clone();
    Mock::given(method("GET"))
        .and(path("/graphql"))
        .and(query_param("operationName", "SearchTitles"))
        .respond_with(ResponseTemplate::new(200).set_body_string(get_titles_fixture))
        .with_priority(1)
        .mount(&ctx.smg_server)
        .await;
    let post_titles_fixture = titles_fixture.clone();
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(is_search_titles_request)
        .respond_with(ResponseTemplate::new(200).set_body_string(post_titles_fixture))
        .with_priority(1)
        .mount(&ctx.smg_server)
        .await;
    Mock::given(method("GET"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(legacy_fixture.clone()))
        .with_priority(100)
        .mount(&ctx.smg_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(legacy_fixture))
        .with_priority(100)
        .mount(&ctx.smg_server)
        .await;
}

#[tokio::test]
async fn smg_search_tvdb() {
    let ctx = TestContext::new().await;
    Mock::given(method("GET"))
        .and(path("/graphql"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("smg/search_tvdb.json")),
        )
        .mount(&ctx.smg_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("smg/search_tvdb.json")),
        )
        .mount(&ctx.smg_server)
        .await;

    let results = ctx
        .app
        .search_metadata_tvdb(&admin(), "Test Movie", "movie", Some(2024))
        .await
        .expect("search_tvdb should succeed");

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].name, "Test Movie Title");
    assert_eq!(results[0].year, Some(2024));
}

#[tokio::test]
async fn smg_search_tvdb_rich() {
    let ctx = TestContext::new().await;
    mount_movie_search_metadata_mocks(&ctx, "smg/search_tvdb_rich.json").await;

    let results = ctx
        .app
        .search_metadata(&admin(), "Test Movie", "movie", 25, "eng", None)
        .await
        .expect("search_tvdb_rich should succeed");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "Test Movie Title");
    assert!(
        results[0].poster_url.is_some(),
        "rich search should have poster"
    );
    assert!(
        results[0].overview.is_some(),
        "rich search should have overview"
    );
    assert_eq!(results[0].year, Some(2024));
}

#[tokio::test]
async fn smg_search_tvdb_rich_includes_year_hint() {
    let ctx = TestContext::new().await;
    let titles_fixture = load_fixture("smg/search_titles.json");
    Mock::given(method("GET"))
        .and(path("/graphql"))
        .and(query_param("operationName", "SearchTitles"))
        .and(query_param(
            "variables",
            r#"{"query":"Test Movie","kind":"movie","limit":25,"language":"eng","year":2024}"#,
        ))
        .respond_with(ResponseTemplate::new(200).set_body_string(titles_fixture))
        .expect(1)
        .with_priority(1)
        .mount(&ctx.smg_server)
        .await;
    mount_movie_search_metadata_mocks(&ctx, "smg/search_tvdb_rich.json").await;

    let results = ctx
        .app
        .search_metadata(&admin(), "Test Movie", "movie", 25, "eng", Some(2024))
        .await
        .expect("search_tvdb_rich with year should succeed");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "Test Movie Title");
    assert!(results[0].poster_url.is_some());
    assert!(results[0].overview.is_some());
    assert_eq!(results[0].year, Some(2024));
}

#[tokio::test]
async fn smg_search_tvdb_batch_uses_dedicated_post_query() {
    let ctx = TestContext::new().await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("smg/search_tvdb_batch.json")),
        )
        .with_priority(1)
        .mount(&ctx.smg_server)
        .await;

    let results = ctx
        .app
        .search_metadata_batch(
            &admin(),
            &[
                MetadataSearchQuery {
                    query: "  Test Movie  ".to_string(),
                    type_hint: "movie".to_string(),
                    year: Some(2024),
                    imdb_id: None,
                    tmdb_id: None,
                    tvdb_id: None,
                },
                MetadataSearchQuery {
                    query: "Test Movie".to_string(),
                    type_hint: "movie".to_string(),
                    year: Some(2024),
                    imdb_id: None,
                    tmdb_id: None,
                    tvdb_id: None,
                },
                MetadataSearchQuery {
                    query: "Test Series".to_string(),
                    type_hint: "series".to_string(),
                    year: None,
                    imdb_id: None,
                    tmdb_id: None,
                    tvdb_id: None,
                },
                MetadataSearchQuery {
                    query: "   ".to_string(),
                    type_hint: "movie".to_string(),
                    year: None,
                    imdb_id: None,
                    tmdb_id: None,
                    tvdb_id: None,
                },
            ],
            "spa",
        )
        .await
        .expect("search_tvdb_batch should succeed");

    assert_eq!(results.len(), 2);

    let movie_key = MetadataSearchQuery {
        query: "Test Movie".to_string(),
        type_hint: "movie".to_string(),
        year: Some(2024),
        imdb_id: None,
        tmdb_id: None,
        tvdb_id: None,
    };
    let series_key = MetadataSearchQuery {
        query: "Test Series".to_string(),
        type_hint: "series".to_string(),
        year: None,
        imdb_id: None,
        tmdb_id: None,
        tvdb_id: None,
    };

    assert_eq!(results[&movie_key].len(), 2);
    assert_eq!(results[&movie_key][0].name, "Test Movie Title");
    assert_eq!(results[&series_key].len(), 1);
    assert_eq!(results[&series_key][0].name, "Test Series Title");

    let requests = ctx
        .smg_server
        .received_requests()
        .await
        .expect("should capture SMG requests");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method.as_str(), "POST");

    let body: serde_json::Value =
        serde_json::from_slice(&requests[0].body).expect("request body should be JSON");
    let query = body
        .get("query")
        .and_then(serde_json::Value::as_str)
        .expect("query string should be present");
    assert!(query.contains("searchTvdbBatch"));
    assert!(!query.contains("q0: searchTvdb"));
    assert!(query.contains("$language"));

    let request_inputs = body
        .pointer("/variables/requests")
        .and_then(serde_json::Value::as_array)
        .expect("batch requests should be present");
    assert_eq!(request_inputs.len(), 2);
    assert_eq!(
        body.pointer("/variables/language")
            .and_then(serde_json::Value::as_str),
        Some("spa")
    );
    assert_eq!(request_inputs[0]["query"], "Test Movie");
    assert_eq!(request_inputs[0]["type"], "movie");
    assert_eq!(request_inputs[0]["year"], 2024);
    assert_eq!(request_inputs[0]["limit"], 10);
    assert_eq!(request_inputs[1]["query"], "Test Series");
    assert_eq!(request_inputs[1]["type"], "series");
    assert!(request_inputs[1].get("year").is_none());
}

#[tokio::test]
async fn smg_get_movie() {
    let ctx = TestContext::new().await;
    Mock::given(method("GET"))
        .and(path("/graphql"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("smg/get_movie.json")),
        )
        .mount(&ctx.smg_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("smg/get_movie.json")),
        )
        .mount(&ctx.smg_server)
        .await;

    let movie = ctx
        .app
        .get_metadata_movie(&admin(), 123456, "eng")
        .await
        .expect("get_movie should succeed");

    assert_eq!(movie.name, "Test Movie Title");
    assert_eq!(movie.year, Some(2024));
    assert_eq!(movie.runtime_minutes, 142);
}

#[tokio::test]
async fn smg_get_series() {
    let ctx = TestContext::new().await;
    Mock::given(method("GET"))
        .and(path("/graphql"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("smg/get_series.json")),
        )
        .mount(&ctx.smg_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("smg/get_series.json")),
        )
        .mount(&ctx.smg_server)
        .await;

    let series = ctx
        .app
        .get_metadata_series(&admin(), 345678, "eng")
        .await
        .expect("get_series should succeed");

    assert_eq!(series.name, "Test Show Name");
    assert_eq!(series.seasons.len(), 2);
    assert_eq!(series.episodes.len(), 3);
}

#[tokio::test]
async fn smg_handles_server_error() {
    let ctx = TestContext::new().await;
    Mock::given(method("GET"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
        .mount(&ctx.smg_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
        .mount(&ctx.smg_server)
        .await;

    let result = ctx
        .app
        .search_metadata_tvdb(&admin(), "Test", "movie", None)
        .await;

    assert!(result.is_err(), "should fail on 500");
}

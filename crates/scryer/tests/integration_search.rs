mod common;

use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, ResponseTemplate};

use common::{load_fixture, TestContext};
use scryer_application::{IndexerClient, MetadataGateway, SearchMode};

fn new_nzbgeek_client(uri: &str) -> scryer_infrastructure::NzbGeekSearchClient {
    scryer_infrastructure::NzbGeekSearchClient::new(
        Some("test-api-key".to_string()),
        Some(uri.to_string()),
        0, // no rate-limit delay in tests
        1,
        1,
    )
}

// ---------------------------------------------------------------------------
// Movie search
// ---------------------------------------------------------------------------

#[tokio::test]
async fn nzbgeek_search_movie_by_category() {
    let ctx = TestContext::new().await;
    Mock::given(method("GET"))
        .and(path("/api"))
        .and(query_param("t", "movie"))
        .and(query_param("apikey", "test-api-key"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("nzbgeek/search_movie.json")),
        )
        .mount(&ctx.nzbgeek_server)
        .await;

    let results = new_nzbgeek_client(&ctx.nzbgeek_server.uri())
        .search(
            "Test Movie".to_string(),
            Some("tt1234567".to_string()),
            None,
            Some("movie".to_string()),
            None,
            None,
            100,
            SearchMode::Interactive,
        )
        .await
        .expect("search should succeed");

    assert_eq!(results.len(), 2);
    assert!(results[0].title.contains("2160p"), "first result should be 4K");
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
            None,
            None,
            Some("movie".to_string()),
            None,
            None,
            100,
            SearchMode::Interactive,
        )
        .await
        .unwrap();

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
            None,
            None,
            Some("movie".to_string()),
            None,
            None,
            100,
            SearchMode::Interactive,
        )
        .await
        .unwrap();

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
// TV search
// ---------------------------------------------------------------------------

#[tokio::test]
async fn nzbgeek_search_tv_by_category() {
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
            None,
            Some("345678".to_string()),
            Some("tv".to_string()),
            None,
            None,
            100,
            SearchMode::Interactive,
        )
        .await
        .expect("TV search should succeed");

    assert_eq!(results.len(), 2);
}

#[tokio::test]
async fn nzbgeek_search_tv_by_anime_category() {
    let ctx = TestContext::new().await;
    // "anime" category should also use tvsearch
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
            None,
            Some("999".to_string()),
            Some("anime".to_string()),
            None,
            None,
            100,
            SearchMode::Interactive,
        )
        .await;

    assert!(results.is_ok(), "anime category should use tvsearch: {:?}", results.err());
}

#[tokio::test]
async fn nzbgeek_search_tv_by_series_category() {
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
            None,
            Some("123".to_string()),
            Some("series".to_string()),
            None,
            None,
            100,
            SearchMode::Interactive,
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
            Some("tt1234567".to_string()),
            None,
            None, // no category
            None,
            None,
            100,
            SearchMode::Interactive,
        )
        .await;

    assert!(results.is_ok(), "should infer movie from imdb_id: {:?}", results.err());
}

#[tokio::test]
async fn nzbgeek_search_infers_tvsearch_from_tvdb_id() {
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
            None,
            Some("345678".to_string()),
            None, // no category
            None,
            None,
            100,
            SearchMode::Interactive,
        )
        .await;

    assert!(results.is_ok(), "should infer tvsearch from tvdb_id: {:?}", results.err());
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
            None,
            None,
            None,
            None,
            None,
            100,
            SearchMode::Interactive,
        )
        .await;

    assert!(results.is_ok(), "generic search should work: {:?}", results.err());
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
            None,
            None,
            Some("movie".to_string()),
            None,
            None,
            100,
            SearchMode::Interactive,
        )
        .await
        .expect("empty search should succeed");

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
            None,
            None,
            Some("movie".to_string()),
            None,
            None,
            100,
            SearchMode::Interactive,
        )
        .await
        .expect("single-item response should parse correctly");

    assert_eq!(results.len(), 1, "should parse single item response");
    assert!(results[0].title.contains("2160p"));
}

// ---------------------------------------------------------------------------
// Error handling
// ---------------------------------------------------------------------------

#[tokio::test]
async fn nzbgeek_search_no_api_key_fails() {
    let ctx = TestContext::new().await;
    let client = scryer_infrastructure::NzbGeekSearchClient::new(
        None,
        Some(ctx.nzbgeek_server.uri()),
        0,
        1,
        1,
    );

    let results = client
        .search(
            "Test".to_string(),
            None,
            None,
            None,
            None,
            None,
            100,
            SearchMode::Interactive,
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
            None,
            None,
            Some("movie".to_string()),
            None,
            None,
            100,
            SearchMode::Interactive,
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
                .insert_header("Retry-After", "60"),
        )
        .mount(&ctx.nzbgeek_server)
        .await;

    let results = new_nzbgeek_client(&ctx.nzbgeek_server.uri())
        .search(
            "Test".to_string(),
            None,
            None,
            Some("movie".to_string()),
            None,
            None,
            100,
            SearchMode::Interactive,
        )
        .await;

    assert!(results.is_err(), "should fail on rate limit");
}

#[tokio::test]
async fn nzbgeek_search_server_error_fallback() {
    let ctx = TestContext::new().await;
    let _call_count = 0u32;

    // First call (movie search) returns 500, second (fallback search) returns results.
    // Since wiremock mocks are matched in reverse order, mount the fallback first.
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
            Some("tt1234567".to_string()),
            None,
            Some("movie".to_string()),
            None,
            None,
            100,
            SearchMode::Interactive,
        )
        .await;

    // The client should fall back to t=search on 500
    assert!(
        results.is_ok(),
        "should fall back to generic search on 500: {:?}",
        results.err()
    );
}

#[tokio::test]
async fn nzbgeek_search_empty_query_and_no_ids_fails() {
    let ctx = TestContext::new().await;
    let client = new_nzbgeek_client(&ctx.nzbgeek_server.uri());

    let results = client
        .search(
            "".to_string(), // empty query
            None,
            None,
            None,
            None,
            None,
            100,
            SearchMode::Interactive,
        )
        .await;

    // Should return empty results or error when no query/ids
    assert!(
        results.is_err() || results.unwrap().is_empty(),
        "empty query with no IDs should fail or return empty"
    );
}

// ---------------------------------------------------------------------------
// Metadata extraction
// ---------------------------------------------------------------------------

#[tokio::test]
async fn nzbgeek_search_extracts_metadata_attributes() {
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
            None,
            None,
            Some("movie".to_string()),
            None,
            None,
            100,
            SearchMode::Interactive,
        )
        .await
        .unwrap();

    let result = &results[0];
    assert_eq!(result.thumbs_up, Some(42), "thumbsup should be parsed");
    assert_eq!(result.thumbs_down, Some(3), "thumbsdown should be parsed");
    assert_eq!(result.nzbgeek_grabs, Some(128), "grabs should be parsed");
    assert!(result.nzbgeek_languages.is_some(), "languages should be parsed");
}

// ---------------------------------------------------------------------------
// MetadataGateway (SMG) client
// ---------------------------------------------------------------------------

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
        .services
        .metadata_gateway
        .search_tvdb("Test Movie", "movie")
        .await
        .expect("search_tvdb should succeed");

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].name, "Test Movie Title");
    assert_eq!(results[0].year, Some(2024));
}

#[tokio::test]
async fn smg_search_tvdb_rich() {
    let ctx = TestContext::new().await;
    Mock::given(method("GET"))
        .and(path("/graphql"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(load_fixture("smg/search_tvdb_rich.json")),
        )
        .mount(&ctx.smg_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(load_fixture("smg/search_tvdb_rich.json")),
        )
        .mount(&ctx.smg_server)
        .await;

    let results = ctx
        .app
        .services
        .metadata_gateway
        .search_tvdb_rich("Test Movie", "movie", 25, "eng")
        .await
        .expect("search_tvdb_rich should succeed");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "Test Movie Title");
    assert!(results[0].poster_url.is_some(), "rich search should have poster");
    assert!(results[0].overview.is_some(), "rich search should have overview");
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
        .services
        .metadata_gateway
        .get_movie(123456, "eng")
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
        .services
        .metadata_gateway
        .get_series(345678, "eng")
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
        .services
        .metadata_gateway
        .search_tvdb("Test", "movie")
        .await;

    assert!(result.is_err(), "should fail on 500");
}

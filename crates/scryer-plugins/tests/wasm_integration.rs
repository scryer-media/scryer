use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::sync::{Arc, Once};
use std::time::{Duration, Instant};

use chrono::Utc;
use scryer_application::{IndexerPluginProvider, SubtitlePluginProvider};
use scryer_domain::IndexerConfig;

static TEST_WASM_RUNTIME: Once = Once::new();

fn initialize_wasm_runtime_for_tests() {
    TEST_WASM_RUNTIME.call_once(|| {
        // Nextest gives each test a process, so this test-only cache is shared
        // across the suite instead of recompiling the same modules per test.
        let cache_dir = std::env::temp_dir().join("scryer-wasmtime-integration-cache");
        scryer_plugins::initialize_wasm_runtime_at(cache_dir)
            .expect("test Wasmtime cache must initialize");
    });
}

fn fixtures_dir() -> std::path::PathBuf {
    initialize_wasm_runtime_for_tests();
    std::env::var_os("SCRYER_TEST_PLUGIN_FIXTURES_DIR")
        .map(std::path::PathBuf::from)
        .expect("cargo nextest must generate the test plugin fixture before running this binary")
}

fn test_config(provider_type: &str) -> IndexerConfig {
    initialize_wasm_runtime_for_tests();
    IndexerConfig {
        id: "idx-1".to_string(),
        name: "Test".to_string(),
        provider_type: provider_type.to_string(),
        base_url: "https://example.com".to_string(),
        api_key_encrypted: None,
        rate_limit_seconds: None,
        rate_limit_burst: None,
        disabled_until: None,
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
        last_health_status: None,
        last_error_message: None,
        last_error_at: None,
        config_json: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

#[test]
fn load_test_indexer_plugin() {
    let fixtures_dir = fixtures_dir();
    let provider = scryer_plugins::load_indexer_plugins(&fixtures_dir).unwrap();

    let types = provider.available_provider_types();
    assert_eq!(types, vec!["test"]);
}

#[test]
fn test_indexer_creates_client() {
    let fixtures_dir = fixtures_dir();
    let provider = scryer_plugins::load_indexer_plugins(&fixtures_dir).unwrap();

    let client = provider.client_for_provider(&test_config("test"));
    assert!(
        client.is_some(),
        "should create a client for provider_type 'test'"
    );
}

#[test]
fn unknown_provider_returns_none() {
    let fixtures_dir = fixtures_dir();
    let provider = scryer_plugins::load_indexer_plugins(&fixtures_dir).unwrap();

    assert!(
        provider
            .client_for_provider(&test_config("nonexistent"))
            .is_none()
    );
}

#[tokio::test]
async fn test_indexer_search() {
    let fixtures_dir = fixtures_dir();
    let provider = scryer_plugins::load_indexer_plugins(&fixtures_dir).unwrap();

    let client = provider.client_for_provider(&test_config("test")).unwrap();

    use scryer_application::SearchMode;
    let results = client
        .search(
            "Glass Harbor Part Two".to_string(),
            std::collections::HashMap::new(),
            None,
            None,
            None,
            None,
            None,
            SearchMode::Auto,
            scryer_application::IndexerErrorOperation::AutomaticSearch,
            None,
            None,
            None,
            vec![],
            None,
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .unwrap()
        .results;

    assert_eq!(results.len(), 1);
    let r = &results[0];
    assert!(r.title.contains("Glass Harbor Part Two"));
    assert_eq!(r.size_bytes, Some(8_000_000_000));
    assert!(r.source.contains("Test"));
}

#[test]
fn empty_dir_loads_no_plugins() {
    let tmp = tempfile::tempdir().unwrap();
    let provider = scryer_plugins::load_indexer_plugins(tmp.path()).unwrap();
    assert!(provider.available_provider_types().is_empty());
}

#[test]
fn scoring_policies_empty_for_test_plugin() {
    let fixtures_dir = fixtures_dir();
    let provider = scryer_plugins::load_indexer_plugins(&fixtures_dir).unwrap();
    // The test-indexer fixture has no scoring policies
    assert!(provider.scoring_policies().is_empty());
}

// ── WasmIndexerPluginProvider builder tests ──────────────────────────────────

#[test]
fn builtin_provider_exposes_expected_metadata_and_supports_removal() {
    let provider = scryer_plugins::WasmIndexerPluginProvider::empty()
        .with_builtin_asset(scryer_plugins::builtins::NEWZNAB)
        .with_builtin_asset(scryer_plugins::builtins::TORZNAB);

    let mut types = provider.available_provider_types();
    types.sort();
    assert!(
        types.contains(&"newznab".to_string()),
        "newznab should register"
    );
    assert!(
        types.contains(&"torznab".to_string()),
        "torznab should register"
    );

    assert!(
        provider.plugin_name_for_provider("torznab").is_some(),
        "torznab should have a plugin name"
    );
    assert!(
        provider.plugin_name_for_provider("newznab").is_some(),
        "newznab should have a plugin name"
    );
    assert_eq!(
        provider.default_base_url_for_provider("newznab").as_deref(),
        None,
        "newznab should not expose a default base URL"
    );
    let newznab_fields = provider.config_fields_for_provider("newznab");
    assert_eq!(
        newznab_fields
            .iter()
            .find(|field| field.key == "base_url")
            .and_then(|field| field.default_value.as_deref()),
        None,
        "newznab base_url field should not carry a builtin default URL"
    );
    assert!(
        provider.default_base_url_for_provider("torznab").is_none(),
        "torznab should not expose a default base URL"
    );

    let newznab_capabilities = provider.capabilities_for_provider("newznab");
    assert_eq!(
        newznab_capabilities.protocols,
        vec![scryer_domain::IndexerProtocolCapability::Usenet]
    );
    assert!(
        newznab_capabilities
            .feed_modes
            .contains(&scryer_domain::IndexerFeedModeCapability::Recent),
        "newznab builtin should expose recent-feed capability metadata"
    );
    assert!(
        newznab_capabilities
            .response_features
            .as_ref()
            .is_some_and(|features| features.grabs && features.comments),
        "newznab builtin should expose nested response feature metadata"
    );

    let trimmed = provider.without_provider_type("newznab");
    let trimmed_types = trimmed.available_provider_types();
    assert!(
        !trimmed_types.contains(&"newznab".to_string()),
        "without_provider_type should drop newznab"
    );
    assert!(
        trimmed_types.contains(&"torznab".to_string()),
        "without_provider_type should leave torznab intact"
    );
}

#[test]
fn external_overrides_builtin_same_provider() {
    initialize_wasm_runtime_for_tests();
    let wasm_bytes =
        scryer_plugins::builtins::decode_builtin_wasm(scryer_plugins::builtins::NEWZNAB)
            .expect("builtin WASM should decode");
    let provider = scryer_plugins::WasmIndexerPluginProvider::empty()
        .with_external_bytes(&wasm_bytes)
        .with_builtin_asset(scryer_plugins::builtins::NEWZNAB);

    // Only one entry for "newznab", not duplicated.
    let types = provider.available_provider_types();
    assert_eq!(
        types.iter().filter(|t| *t == "newznab").count(),
        1,
        "builtin should not duplicate external"
    );
}

#[test]
fn invalid_wasm_bytes_silently_skipped() {
    let provider = scryer_plugins::WasmIndexerPluginProvider::empty()
        .with_external_bytes(b"this is not valid wasm");

    assert!(
        provider.available_provider_types().is_empty(),
        "invalid WASM should be skipped"
    );
}

#[test]
fn invalid_bytes_dont_affect_valid() {
    let provider = scryer_plugins::WasmIndexerPluginProvider::empty()
        .with_builtin_asset(scryer_plugins::builtins::NEWZNAB)
        .with_external_bytes(b"garbage");

    let types = provider.available_provider_types();
    assert!(
        types.contains(&"newznab".to_string()),
        "valid builtin should survive despite garbage external"
    );
}

#[test]
fn subtitle_builtins_are_empty() {
    let provider = scryer_plugins::WasmSubtitlePluginProvider::empty();
    assert!(
        provider.builtin_provider_types().is_empty(),
        "subtitle builtins should now be catalog-only"
    );
}

#[test]
fn newznab_family_builtins_include_rss_search_path() {
    for (name, asset) in [
        ("newznab", scryer_plugins::builtins::NEWZNAB),
        ("torznab", scryer_plugins::builtins::TORZNAB),
    ] {
        let wasm_bytes = scryer_plugins::builtins::decode_builtin_wasm(asset)
            .expect("builtin WASM should decode");
        assert!(
            bytes_contain(&wasm_bytes, b"rss_search: fetching recent releases"),
            "{name} builtin WASM is missing the RSS search path"
        );
    }
}

#[tokio::test]
async fn newznab_builtin_rss_search_uses_category_only_request() {
    let (base_url, request_rx) = spawn_newznab_response_server();
    let provider = scryer_plugins::WasmIndexerPluginProvider::empty()
        .with_builtin_asset(scryer_plugins::builtins::NEWZNAB);

    let mut config = test_config("newznab");
    config.config_json = Some(
        serde_json::json!({
            "base_url": base_url,
            "api_key": "test-key",
        })
        .to_string(),
    );

    let client = provider.client_for_provider(&config).unwrap();
    let response = client
        .search(
            String::new(),
            std::collections::HashMap::new(),
            None,
            Some("series".to_string()),
            None,
            Some(vec!["5000".to_string()]),
            None,
            scryer_application::SearchMode::Auto,
            scryer_application::IndexerErrorOperation::AutomaticSearch,
            None,
            None,
            None,
            vec![],
            None,
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .unwrap();

    assert_eq!(response.results.len(), 1);
    assert_eq!(
        response.results[0].title,
        "Example.Show.S01E01.1080p.WEB-DL"
    );

    let request = request_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("mock Newznab server should receive a request");
    assert!(request.contains("GET /api?"), "request was {request}");
    assert!(request.contains("t=tvsearch"), "request was {request}");
    assert!(request.contains("cat=5000"), "request was {request}");
    assert!(
        !request.contains("&q=") && !request.contains("?q="),
        "RSS request should not include q=: {request}"
    );
}

#[tokio::test]
async fn newznab_builtin_preserves_prowlarr_429_description_and_retry_after() {
    let body = r#"<?xml version="1.0" encoding="UTF-8"?>
<error code="429" description="User configurable Indexer Query Limit of 100 in last 1 hour(s) reached." />"#;
    let (base_url, request_rx) = spawn_newznab_raw_response_server(
        "429 Too Many Requests",
        &["Content-Type: application/rss+xml", "Retry-After: 321"],
        body,
    );
    let provider = scryer_plugins::WasmIndexerPluginProvider::empty()
        .with_builtin_asset(scryer_plugins::builtins::NEWZNAB);

    let mut config = test_config("newznab");
    config.managed_parent_config_id = Some("prowlarr-parent".to_string());
    config.managed_child_key = Some("42".to_string());
    config.config_json = Some(
        serde_json::json!({
            "base_url": base_url,
            "api_key": "test-key",
        })
        .to_string(),
    );

    let client = provider.client_for_provider(&config).unwrap();
    let error = client
        .search(
            "scryer connection test".to_string(),
            std::collections::HashMap::new(),
            None,
            None,
            None,
            None,
            None,
            scryer_application::SearchMode::Interactive,
            scryer_application::IndexerErrorOperation::InteractiveSearch,
            None,
            None,
            None,
            vec![],
            None,
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .expect_err("Prowlarr's 429 should remain a failed child search")
        .to_string();

    assert!(
        error.contains("User configurable Indexer Query Limit of 100 in last 1 hour(s) reached."),
        "error was {error}"
    );
    assert!(error.contains("retry after 321s"), "error was {error}");
    assert!(
        !error.contains("stopped after"),
        "the host should preserve Prowlarr's real reason instead of the guest's generic error: {error}"
    );
    request_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("mock Prowlarr server should receive a Newznab request");
}

#[tokio::test]
async fn newznab_builtin_search_extracts_password_hints() {
    let body = r#"{"channel":{"item":[{"title":"Protected.Release.1080p.WEB-DL","guid":"guid-1","link":"http://example.test/info","enclosure":{"@attributes":{"url":"http://example.test/download.nzb","length":"12345","type":"application/x-nzb"}},"attr":[{"@attributes":{"name":"password","value":" archive-password "}}]}]}}"#;
    let (base_url, request_rx) = spawn_newznab_response_server_with_body(body);
    let provider = scryer_plugins::WasmIndexerPluginProvider::empty()
        .with_builtin_asset(scryer_plugins::builtins::NEWZNAB);

    let mut config = test_config("newznab");
    config.config_json = Some(
        serde_json::json!({
            "base_url": base_url,
            "api_key": "test-key",
        })
        .to_string(),
    );

    let client = provider.client_for_provider(&config).unwrap();
    let response = client
        .search(
            "protected release".to_string(),
            std::collections::HashMap::new(),
            None,
            None,
            None,
            None,
            None,
            scryer_application::SearchMode::Interactive,
            scryer_application::IndexerErrorOperation::InteractiveSearch,
            None,
            None,
            None,
            vec![],
            None,
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .unwrap();

    assert_eq!(response.results.len(), 1);
    assert_eq!(
        response.results[0].password_hint.as_deref(),
        Some("archive-password")
    );
    assert_eq!(
        response.results[0]
            .extra
            .get("password")
            .and_then(|value| value.as_str()),
        Some("archive-password")
    );

    request_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("mock Newznab server should receive a request");
}

#[tokio::test]
async fn newznab_builtin_search_treats_password_flags_as_protected_only() {
    let body = r#"{"channel":{"item":[{"title":"Flagged.Release.1080p.WEB-DL","guid":"guid-flag","link":"http://example.test/info","enclosure":{"@attributes":{"url":"http://example.test/download.nzb","length":"12345","type":"application/x-nzb"}},"attr":[{"@attributes":{"name":"password","value":"1"}}]}]}}"#;
    let (base_url, request_rx) = spawn_newznab_response_server_with_body(body);
    let provider = scryer_plugins::WasmIndexerPluginProvider::empty()
        .with_builtin_asset(scryer_plugins::builtins::NEWZNAB);

    let mut config = test_config("newznab");
    config.config_json = Some(
        serde_json::json!({
            "base_url": base_url,
            "api_key": "test-key",
        })
        .to_string(),
    );

    let client = provider.client_for_provider(&config).unwrap();
    let response = client
        .search(
            "flagged release".to_string(),
            std::collections::HashMap::new(),
            None,
            None,
            None,
            None,
            None,
            scryer_application::SearchMode::Interactive,
            scryer_application::IndexerErrorOperation::InteractiveSearch,
            None,
            None,
            None,
            vec![],
            None,
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .unwrap();

    assert_eq!(response.results.len(), 1);
    assert_eq!(response.results[0].password_hint, None);
    assert!(!response.results[0].extra.contains_key("password"));
    assert_eq!(
        response.results[0]
            .extra
            .get("password_protected")
            .and_then(|value| value.as_bool()),
        Some(true)
    );

    request_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("mock Newznab server should receive a request");
}

#[tokio::test]
async fn newznab_builtin_full_search_trims_whitespace_padded_config_values() {
    let request = run_newznab_builtin_full_search(Some(" ?foo=bar baz&zap=1 ")).await;

    assert!(request.starts_with("GET /api?"), "request was {request}");
    assert_eq!(
        request_query_value(&request, "apikey").as_deref(),
        Some("test-key")
    );
    assert_eq!(
        request_query_value(&request, "q").as_deref(),
        Some("scryer connection test")
    );
    assert_eq!(
        request_query_value(&request, "foo").as_deref(),
        Some("bar baz")
    );
    assert_eq!(request_query_value(&request, "zap").as_deref(), Some("1"));
}

#[tokio::test]
async fn newznab_builtin_full_search_succeeds_without_additional_params() {
    let request = run_newznab_builtin_full_search(None).await;

    assert!(request.starts_with("GET /api?"), "request was {request}");
    assert_eq!(
        request_query_value(&request, "apikey").as_deref(),
        Some("test-key")
    );
    assert_eq!(
        request_query_value(&request, "q").as_deref(),
        Some("scryer connection test")
    );
    assert_eq!(request_query_value(&request, "foo").as_deref(), None);
    assert_eq!(request_query_value(&request, "zap").as_deref(), None);
}

#[tokio::test]
async fn newznab_builtin_full_search_accepts_percent_encoded_additional_params() {
    let request = run_newznab_builtin_full_search(Some(" ?foo=bar%20baz&zap=1 ")).await;

    assert!(request.starts_with("GET /api?"), "request was {request}");
    assert_eq!(
        request_query_value(&request, "apikey").as_deref(),
        Some("test-key")
    );
    assert_eq!(
        request_query_value(&request, "q").as_deref(),
        Some("scryer connection test")
    );
    assert_eq!(
        request_query_value(&request, "foo").as_deref(),
        Some("bar baz"),
        "request was {request}"
    );
    assert_eq!(
        request_query_value(&request, "zap").as_deref(),
        Some("1"),
        "request was {request}"
    );
}

#[tokio::test]
async fn newznab_builtin_full_search_accepts_ampersand_prefixed_additional_params() {
    let request = run_newznab_builtin_full_search(Some(" &foo=bar%20baz&zap=1 ")).await;

    assert!(request.starts_with("GET /api?"), "request was {request}");
    assert_eq!(
        request_query_value(&request, "apikey").as_deref(),
        Some("test-key")
    );
    assert_eq!(
        request_query_value(&request, "q").as_deref(),
        Some("scryer connection test")
    );
    assert_eq!(
        request_query_value(&request, "foo").as_deref(),
        Some("bar baz"),
        "request was {request}"
    );
    assert_eq!(
        request_query_value(&request, "zap").as_deref(),
        Some("1"),
        "request was {request}"
    );
}

#[tokio::test]
async fn newznab_builtin_full_search_accepts_unprefixed_additional_params() {
    let request = run_newznab_builtin_full_search(Some(" foo=bar%20baz&zap=1 ")).await;

    assert!(request.starts_with("GET /api?"), "request was {request}");
    assert_eq!(
        request_query_value(&request, "apikey").as_deref(),
        Some("test-key")
    );
    assert_eq!(
        request_query_value(&request, "q").as_deref(),
        Some("scryer connection test")
    );
    assert_eq!(
        request_query_value(&request, "foo").as_deref(),
        Some("bar baz"),
        "request was {request}"
    );
    assert_eq!(
        request_query_value(&request, "zap").as_deref(),
        Some("1"),
        "request was {request}"
    );
}

#[tokio::test]
async fn newznab_builtin_full_search_canonicalizes_query_bearing_connection_urls() {
    let (base_url, request_rx) = spawn_newznab_response_server();
    let provider = scryer_plugins::WasmIndexerPluginProvider::empty()
        .with_builtin_asset(scryer_plugins::builtins::NEWZNAB);

    let mut config = test_config("newznab");
    config.config_json = Some(
        serde_json::json!({
            "base_url": format!(
                "{}/api?t=search&q=legacy+query&attrs=poster&apikey=test-key",
                base_url
            ),
            "api_key": "test-key",
            "api_path": "/api",
        })
        .to_string(),
    );

    let client = provider.client_for_provider(&config).unwrap();
    let response = client
        .search(
            "scryer connection test".to_string(),
            std::collections::HashMap::new(),
            None,
            None,
            None,
            None,
            None,
            scryer_application::SearchMode::Interactive,
            scryer_application::IndexerErrorOperation::InteractiveSearch,
            None,
            None,
            None,
            vec![],
            None,
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .unwrap();

    assert_eq!(response.results.len(), 1);

    let request = request_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("mock Newznab server should receive a request");
    assert!(request.starts_with("GET /api?"), "request was {request}");
    assert_eq!(
        request_query_value(&request, "q").as_deref(),
        Some("scryer connection test"),
        "request was {request}"
    );
    assert_eq!(
        request_query_value(&request, "attrs").as_deref(),
        Some("poster"),
        "request was {request}"
    );
    assert!(
        !request.contains("legacy+query") && !request.contains("legacy%20query"),
        "request was {request}"
    );
}

async fn run_newznab_builtin_full_search(additional_params: Option<&str>) -> String {
    let (base_url, request_rx) = spawn_newznab_response_server();
    let provider = scryer_plugins::WasmIndexerPluginProvider::empty()
        .with_builtin_asset(scryer_plugins::builtins::NEWZNAB);

    let mut config = test_config("newznab");
    let mut config_json = serde_json::Map::from_iter([
        (
            "base_url".to_string(),
            serde_json::Value::String(format!("  {base_url}/  ")),
        ),
        (
            "api_key".to_string(),
            serde_json::Value::String(" test-key \n".to_string()),
        ),
        (
            "api_path".to_string(),
            serde_json::Value::String(" /api ".to_string()),
        ),
    ]);
    if let Some(additional_params) = additional_params {
        config_json.insert(
            "additional_params".to_string(),
            serde_json::Value::String(additional_params.to_string()),
        );
    }
    config.config_json = Some(serde_json::Value::Object(config_json).to_string());

    let client = provider.client_for_provider(&config).unwrap();
    let response = client
        .search(
            "scryer connection test".to_string(),
            std::collections::HashMap::new(),
            None,
            None,
            None,
            None,
            None,
            scryer_application::SearchMode::Interactive,
            scryer_application::IndexerErrorOperation::InteractiveSearch,
            None,
            None,
            None,
            vec![],
            None,
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .unwrap();

    assert_eq!(response.results.len(), 1);

    request_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("mock Newznab server should receive a request")
}

fn bytes_contain(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn request_query_value(request: &str, key: &str) -> Option<String> {
    let request_line = request.lines().next()?;
    let path = request_line.split_whitespace().nth(1)?;
    let url = url::Url::parse(&format!("http://example.test{path}")).ok()?;
    url.query_pairs()
        .find_map(|(candidate, value)| (candidate == key).then(|| value.into_owned()))
}

fn spawn_newznab_response_server() -> (String, mpsc::Receiver<String>) {
    let body = r#"{"channel":{"item":[{"title":"Example.Show.S01E01.1080p.WEB-DL","guid":"guid-1","link":"http://example.test/info","enclosure":{"@attributes":{"url":"http://example.test/download.nzb","length":"12345","type":"application/x-nzb"}},"attr":[{"@attributes":{"name":"grabs","value":"4"}}]}]}}"#;
    spawn_newznab_response_server_with_body(body)
}

fn spawn_newznab_response_server_with_body(body: &'static str) -> (String, mpsc::Receiver<String>) {
    spawn_newznab_raw_response_server("200 OK", &["Content-Type: application/json"], body)
}

fn spawn_newznab_raw_response_server(
    status: &'static str,
    headers: &'static [&'static str],
    body: &'static str,
) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let (request_tx, request_rx) = mpsc::channel();

    std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut buffer = [0_u8; 8192];
                    let bytes_read = stream.read(&mut buffer).unwrap_or(0);
                    let request = String::from_utf8_lossy(&buffer[..bytes_read]).to_string();
                    let _ = request_tx.send(request);

                    let headers = headers.join("\r\n");
                    let response = format!(
                        "HTTP/1.1 {status}\r\n{headers}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes());
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => break,
            }
        }
    });

    (format!("http://{address}"), request_rx)
}

// ── DynamicPluginProvider tests ──────────────────────────────────────────────

#[test]
fn dynamic_delegates_available_types() {
    let inner = scryer_plugins::WasmIndexerPluginProvider::empty()
        .with_builtin_asset(scryer_plugins::builtins::NEWZNAB);
    let dynamic = scryer_plugins::DynamicPluginProvider::new(inner);

    let types = dynamic.available_provider_types();
    assert!(types.contains(&"newznab".to_string()));
}

#[test]
fn dynamic_provider_reload_behaviour() {
    let inner = scryer_plugins::WasmIndexerPluginProvider::empty()
        .with_builtin_asset(scryer_plugins::builtins::NEWZNAB)
        .with_builtin_asset(scryer_plugins::builtins::TORZNAB);
    let dynamic = scryer_plugins::DynamicPluginProvider::new(inner);

    assert_eq!(
        dynamic.available_provider_types().len(),
        2,
        "dynamic should initially expose both builtins"
    );

    // reload_plugins disables a single provider while keeping the rest.
    dynamic
        .reload_plugins(&[], &["newznab".to_string()])
        .unwrap();
    let after_disable = dynamic.available_provider_types();
    assert!(
        !after_disable.contains(&"newznab".to_string()),
        "newznab should be disabled after reload_plugins"
    );
    assert!(
        after_disable.contains(&"torznab".to_string()),
        "torznab should remain after reload_plugins"
    );

    // reload swaps the inner provider entirely; an empty provider clears all.
    dynamic.reload(scryer_plugins::WasmIndexerPluginProvider::empty());
    assert!(
        dynamic.available_provider_types().is_empty(),
        "after reload with empty provider, no types should remain"
    );
}

#[test]
fn dynamic_client_cache_hit() {
    let inner = scryer_plugins::WasmIndexerPluginProvider::empty()
        .with_builtin_asset(scryer_plugins::builtins::NEWZNAB);
    let dynamic = scryer_plugins::DynamicPluginProvider::new(inner);

    let config = test_config("newznab");
    let c1 = dynamic.client_for_provider(&config).unwrap();
    let c2 = dynamic.client_for_provider(&config).unwrap();
    assert!(
        Arc::ptr_eq(&c1, &c2),
        "same config should return cached client"
    );
}

#[test]
fn dynamic_client_cache_miss_on_updated_at() {
    let inner = scryer_plugins::WasmIndexerPluginProvider::empty()
        .with_builtin_asset(scryer_plugins::builtins::NEWZNAB);
    let dynamic = scryer_plugins::DynamicPluginProvider::new(inner);

    let mut config1 = test_config("newznab");
    let c1 = dynamic.client_for_provider(&config1).unwrap();

    // Change updated_at to simulate a config update
    config1.updated_at = Utc::now() + chrono::Duration::seconds(10);
    let c2 = dynamic.client_for_provider(&config1).unwrap();
    let c3 = dynamic.client_for_provider(&config1).unwrap();

    assert!(
        !Arc::ptr_eq(&c1, &c2),
        "different updated_at should produce a new client"
    );
    assert!(
        Arc::ptr_eq(&c2, &c3),
        "rebuilt revision should become the cached client"
    );
}

// ── Builder validation tests ─────────────────────────────────────────────────

#[test]
fn builtin_with_valid_descriptor_loads() {
    let provider = scryer_plugins::WasmIndexerPluginProvider::empty()
        .with_builtin_asset(scryer_plugins::builtins::NEWZNAB);

    assert!(
        provider
            .available_provider_types()
            .contains(&"newznab".to_string()),
        "NEWZNAB builtin should register as 'newznab'"
    );
}

#[test]
fn plugin_capabilities_accessible() {
    let fixtures_dir = fixtures_dir();
    let provider = scryer_plugins::load_indexer_plugins(&fixtures_dir).unwrap();

    let caps = provider.capabilities_for_provider("test");
    assert!(caps.rss, "rss capability should default to true");
    // The test plugin should have some capabilities declared
    // (the default is all-true, so at minimum search should be true)
    assert!(caps.search, "search capability should be true");
}

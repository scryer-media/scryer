//! Run with `--run-ignored only` and the package paths below. These are the
//! optimized, descriptor-embedded artifacts emitted by `xtask plugin validate`,
//! not raw compiler output or simulated plugin responses.

use scryer_application::{
    IndexerPluginProvider, IndexerSearchPlanRequest, IndexerSearchStrategyEventSink,
    IndexerSearchStrategyRequest, PluginDescriptorLoader,
};
use scryer_domain::IndexerConfig;
use scryer_plugins::{WasmIndexerPluginProvider, WasmPluginDescriptorLoader};
use std::{collections::HashMap, time::Duration};
use tokio_util::sync::CancellationToken;
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate, matchers::method};

struct Pages;
impl Respond for Pages {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let q: HashMap<_, _> = request.url.query_pairs().collect();
        if q.get("t").is_some_and(|v| v == "caps") {
            return ResponseTemplate::new(200).set_body_raw(
                r#"<caps><limits default="100" max="100"/><searching><search available="yes" supportedParams="q"/><tv-search available="yes" supportedParams="q,tvdbid,season,ep"/></searching></caps>"#,
                "application/xml",
            );
        }
        let offset: usize = q.get("offset").and_then(|v| v.parse().ok()).unwrap_or(0);
        let items = (offset..(offset + 100).min(250)).map(|i| format!(
            "<item><title>Example.Show.S01E01.1080p.WEB-DL-GROUP{i}</title><guid>release-{i}</guid><link>http://example.invalid/{i}</link><enclosure url=\"http://example.invalid/{i}\" length=\"1000000000\" type=\"application/x-nzb\"/><newznab:attr name=\"size\" value=\"1000000000\"/></item>"
        )).collect::<String>();
        ResponseTemplate::new(200).set_body_raw(format!(
            "<rss xmlns:newznab=\"http://www.newznab.com/DTD/2010/feeds/attributes/\"><channel><newznab:response offset=\"{offset}\" total=\"250\"/>{items}</channel></rss>"
        ), "application/xml")
    }
}

fn config(provider: &str, base: &str) -> IndexerConfig {
    IndexerConfig {
        id: "packaged-indexer".into(),
        name: "Packaged fixture".into(),
        provider_type: provider.into(),
        base_url: base.into(),
        api_key_encrypted: None,
        rate_limit_seconds: None,
        rate_limit_burst: None,
        max_queries_per_minute: None,
        disabled_until: None,
        is_enabled: true,
        enable_interactive_search: true,
        enable_auto_search: true,
        proxy_config_id: None,
        download_client_id: None,
        seeding_profile_id: None,
        managed_parent_config_id: None,
        managed_child_key: None,
        managed_metadata_json: None,
        caps_snapshot_json: None,
        last_health_status: None,
        last_error_message: None,
        last_error_at: None,
        config_json: Some(serde_json::json!({"base_url":base,"api_key":"fixture-key"}).to_string()),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

fn strategy(cursor: Option<String>) -> IndexerSearchStrategyRequest {
    IndexerSearchStrategyRequest {
        title_context: Default::default(),
        strategy_id: "fixture-search".into(),
        labels: vec!["freetext".into()],
        query: "Example Show".into(),
        ids: HashMap::new(),
        category: None,
        facet: Some("series".into()),
        id_search_facet: None,
        newznab_categories: None,
        season: None,
        episode: None,
        absolute_episode: None,
        year: None,
        tagged_aliases: vec![],
        rss_catch_up: None,
        page_cursor: cursor,
        page_budget: Some(2),
        result_limit: None,
    }
}

fn bytes(variable: &str) -> Vec<u8> {
    let path = std::env::var_os(variable)
        .unwrap_or_else(|| panic!("set {variable} to a packaged WASM artifact"));
    std::fs::read(path).expect("packaged WASM must exist")
}

fn initialize() {
    scryer_plugins::initialize_wasm_runtime_at(
        std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("packaged-pagination-cache"),
    )
    .expect("runtime initializes");
}

#[tokio::test]
#[ignore = "requires SCRYER_TEST_NEWZNAB_PACKAGE and SCRYER_TEST_TORZNAB_PACKAGE from xtask plugin validate"]
async fn packaged_plugins_follow_pages_and_resume_after_reloading() {
    initialize();
    for (kind, variable) in [
        ("newznab", "SCRYER_TEST_NEWZNAB_PACKAGE"),
        ("torznab", "SCRYER_TEST_TORZNAB_PACKAGE"),
    ] {
        let wasm = bytes(variable);
        let descriptor = WasmPluginDescriptorLoader
            .load_descriptor_from_wasm_bytes(&wasm)
            .unwrap();
        let serialized = serde_json::to_value(descriptor).unwrap();
        assert_eq!(
            serialized["provider"]["capabilities"]["limits"]["paged_search"],
            true
        );
        assert_eq!(serialized["provider"]["search_semantics_version"], 3);
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(Pages)
            .mount(&server)
            .await;
        let cfg = config(kind, &server.uri());
        let mut cursor = None;
        let mut titles = std::collections::HashSet::new();
        for (expected_count, complete) in [(200, false), (50, true)] {
            // Recreate the loader and client: continuation comes from the saved
            // cursor, not a guest instance or host in-memory paging state.
            let provider = WasmIndexerPluginProvider::empty().with_external_bytes(&wasm);
            let client = provider.client_for_provider(&cfg).unwrap();
            assert!(
                client.search_plan_capability().is_none(),
                "paged artifact must select the cursor-following path"
            );
            let response = tokio::time::timeout(
                Duration::from_secs(60),
                client.search_strategy(
                    strategy(cursor.take()),
                    scryer_application::SearchMode::Auto,
                    scryer_application::IndexerErrorOperation::AutomaticSearch,
                    CancellationToken::new(),
                ),
            )
            .await
            .expect("search deadline")
            .expect("packaged search");
            assert_eq!(response.results.len(), expected_count, "{kind}");
            assert_eq!(response.completion.is_complete(), complete, "{kind}");
            if !complete {
                assert_eq!(response.next_cursor.as_deref(), Some("offset:200"));
            }
            cursor = response.next_cursor;
            titles.extend(response.results.into_iter().map(|r| r.title));
        }
        assert!(cursor.is_none());
        assert_eq!(titles.len(), 250);
        let offsets: Vec<usize> = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| !r.url.query_pairs().any(|(k, v)| k == "t" && v == "caps"))
            .map(|r| {
                r.url
                    .query_pairs()
                    .find(|(k, _)| k == "offset")
                    .map_or(0, |(_, v)| v.parse().unwrap())
            })
            .collect();
        assert_eq!(
            offsets,
            vec![0, 100, 200],
            "{kind}: no repeated first page or skipped provider page"
        );
    }
}

#[tokio::test]
#[ignore = "requires SCRYER_TEST_BROKEN_NEWZNAB_PACKAGE pointing to published Newznab 3.0.2"]
async fn published_plugin_with_missing_capability_cannot_claim_complete_with_a_cursor() {
    initialize();
    let wasm = bytes("SCRYER_TEST_BROKEN_NEWZNAB_PACKAGE");
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(Pages)
        .mount(&server)
        .await;
    let provider = WasmIndexerPluginProvider::empty().with_external_bytes(&wasm);
    let client = provider
        .client_for_provider(&config("newznab", &server.uri()))
        .unwrap();
    assert!(client.search_plan_capability().is_some());
    let (sender, mut receiver) = tokio::sync::mpsc::channel(2);
    tokio::time::timeout(
        Duration::from_secs(60),
        client.search_plan(
            IndexerSearchPlanRequest {
                plan_id: "fixture-plan".into(),
                strategies: vec![strategy(None)],
            },
            scryer_application::SearchMode::Auto,
            scryer_application::IndexerErrorOperation::AutomaticSearch,
            CancellationToken::new(),
            IndexerSearchStrategyEventSink::new(sender),
        ),
    )
    .await
    .expect("plan deadline")
    .expect("plan response");
    let event = tokio::time::timeout(Duration::from_secs(60), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    let response = event.response.unwrap();
    assert_eq!(response.results.len(), 100);
    assert_eq!(response.next_cursor.as_deref(), Some("offset:100"));
    assert!(
        !response.completion.is_complete(),
        "a successful first page must never converge the search"
    );
    let response = tokio::time::timeout(
        Duration::from_secs(60),
        client.search_strategy(
            strategy(None),
            scryer_application::SearchMode::Auto,
            scryer_application::IndexerErrorOperation::AutomaticSearch,
            CancellationToken::new(),
        ),
    )
    .await
    .expect("strategy deadline")
    .expect("strategy response");
    assert_eq!(response.results.len(), 100);
    assert_eq!(response.next_cursor.as_deref(), Some("offset:100"));
    assert!(!response.completion.is_complete());
}

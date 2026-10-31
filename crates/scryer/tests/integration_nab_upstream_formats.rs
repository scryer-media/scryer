#![recursion_limit = "256"]

use std::{collections::HashMap, time::Duration};

mod common;
#[allow(
    dead_code,
    reason = "this integration test imports only part of the application module"
)]
#[path = "../src/nab_api/mod.rs"]
mod nab_api;

use scryer_application::{
    IndexerClient, IndexerConfigRepository, IndexerPluginProvider, RawTextSearchPage,
    RawTextSearchRequest,
};
use scryer_domain::IndexerConfig;
use scryer_infrastructure_acquisition::indexers::nab_feed::{
    NabFeedItem, NabFeedPage, NabProtocol, write_xml_feed,
};
use scryer_plugins::WasmIndexerPluginProvider;
use tokio_util::sync::CancellationToken;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, query_param},
};

// The feed writer binds the protocol namespace on each preserved attribute.
// Check both the qualified name and that local binding here; the serializer's
// NsReader tests separately validate namespace resolution and XML escaping.
fn protocol_attributes(xml: &str) -> Vec<(String, String)> {
    xml.split("<newznab:attr ")
        .skip(1)
        .filter_map(|tail| {
            let tag = tail.split_once("/>")?.0;
            if !tag.contains("xmlns:newznab=\"http://www.newznab.com/DTD/2010/feeds/attributes/\"")
            {
                return None;
            }
            let field = |name: &str| {
                tag.split_once(&format!(" {name}=\""))
                    .and_then(|(_, tail)| tail.split_once('"'))
                    .map(|(value, _)| value.to_owned())
            };
            Some((field("name")?, field("value")?))
        })
        .collect()
}

fn config(base: &str) -> IndexerConfig {
    let now = chrono::Utc::now();
    IndexerConfig {
        id: "json-upstream".into(),
        name: "JSON upstream fixture".into(),
        provider_type: "newznab".into(),
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
        last_health_status: None,
        last_error_code: None,
        last_error_message: None,
        last_error_at: None,
        caps_snapshot_json: Some(
            serde_json::json!({
                "parser_version": scryer_domain::INDEXER_CAPS_PARSER_VERSION,
                "cache_fetched_at": now.to_rfc3339(), "searching_present": true,
                "limits_default": 100, "limits_max": 100,
                "search": {"available": true, "supported_params": ["q", "cat"]}
            })
            .to_string(),
        ),
        config_json: Some(
            serde_json::json!({"base_url": base, "api_key": "fixture-upstream-key"}).to_string(),
        ),
        created_at: now,
        updated_at: now,
    }
}

/// The real shared route, principal, coordinator and packaged adapter, with a
/// JSON upstream and XML caller. No codec-only shortcut stands in for HTTP.
#[tokio::test]
#[ignore = "requires SCRYER_TEST_NEWZNAB_PACKAGE from local xtask plugin validate"]
async fn authenticated_http_json_upstream_preserves_xml_metadata_and_stable_links() {
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use scryer_application::{SettingsRepository, indexer_clients::*, testing::AppUseCaseTestExt};
    use scryer_infrastructure_acquisition::indexers::{
        client_store::IndexerClientStore, config_store::IndexerConfigStore,
        search_client::MultiIndexerSearchClient,
    };
    use scryer_infrastructure_crypto::{
        EncryptionKey, artifact_reference::ArtifactReferenceCipher,
    };
    use std::sync::Arc;
    use tower::ServiceExt;

    common::initialize_wasm_runtime_for_tests();
    let ctx = common::TestContext::new().await;
    ctx.settings_store
        .batch_ensure_setting_definitions(vec![
            scryer_infrastructure_sql::types::SettingDefinitionSeed {
                category: "ui".into(),
                scope: scryer_application::SETTINGS_SCOPE_SYSTEM.into(),
                key_name: scryer_application::EXPERIMENTAL_FEATURES_ENABLED_KEY.into(),
                data_type: "boolean".into(),
                default_value_json: "true".into(),
                is_sensitive: false,
                validation_json: None,
            },
        ])
        .await
        .unwrap();
    ctx.settings_store
        .upsert_setting_json(
            scryer_application::SETTINGS_SCOPE_SYSTEM,
            scryer_application::EXPERIMENTAL_FEATURES_ENABLED_KEY,
            None,
            "true".into(),
            "test",
            None,
        )
        .await
        .unwrap();
    let package = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(std::env::var_os("SCRYER_TEST_NEWZNAB_PACKAGE").unwrap());
    let provider = Arc::new(
        WasmIndexerPluginProvider::empty().with_external_bytes(&std::fs::read(package).unwrap()),
    );
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(query_param("t", "search"))
        .and(query_param("q", "software")).and(query_param("cat", "4000"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "channel":{"item":[{"title":"Software & tools","guid":"stable-release",
                "enclosure":{"@attributes":{"url":"https://upstream.example/get?apikey=upstream-secret",
                    "length":"1234","type":"application/x-nzb"}},
                "attr":[{"@attributes":{"name":"category","value":"4000"}},
                    {"@attributes":{"name":"custom","value":"001.00"}},
                    {"@attributes":{"name":"custom","value":"second & <third>"}},
                    {"@attributes":{"name":"large","value":"\"".repeat(1024 * 1024)}},
                    {"@attributes":{"name":"password","value":"release-password"}}]}]}
        }))).expect(2).mount(&server).await;
    let configs = Arc::new(IndexerConfigStore::new(
        ctx.db.datastore(),
        ctx.db.encryption_key_state(),
    ));
    let source = config(&server.uri());
    configs.create(source.clone()).await.unwrap();
    let clients = Arc::new(IndexerClientStore::new(ctx.db.datastore()));
    let key = issue_indexer_client_key("test-salt").unwrap();
    let now = chrono::Utc::now();
    let mut client = IndexerClientRecord {
        id: "json-consumer".into(),
        configuration: IndexerClientConfiguration {
            name: "JSON consumer fixture".into(),
            enabled: true,
            protocols: vec![IndexerClientProtocol::Newznab],
            grants: vec![IndexerClientGrant {
                indexer_id: source.id,
                categories: Some(vec!["4000".into()]),
            }],
        },
        lookup_id: Some(key.lookup_id),
        secret_hash: Some(key.secret_hash),
        key_generation: 1,
        revision: 1,
        last_used_at: None,
        last_outcome_code: None,
        created_at: now,
        updated_at: now,
    };
    clients.save(&client, None).await.unwrap();
    let multi = Arc::new(MultiIndexerSearchClient::new(
        configs,
        ctx.indexer_stats.clone(),
        provider.clone(),
    ));
    let app = ctx
        .app
        .with_indexer_client_for_test(multi)
        .with_test_overrides(|builder| {
            builder
                .with_indexer_client_repository(clients.clone())
                .with_plugin_provider(provider)
        });
    let cipher = Arc::new(ArtifactReferenceCipher::new(&EncryptionKey::from_bytes(
        [42; 32],
    )));
    let router = nab_api::router(nab_api::NabState {
        app,
        cipher: cipher.clone(),
        base_path: String::new(),
        tls: false,
    });
    let mut previous = None;
    let mut retrieval = String::new();
    for _ in 0..2 {
        let request = Request::builder()
            .uri("/newznab/api?t=search&q=software&cat=4000")
            .header("host", "scryer.example")
            .header("x-api-key", &key.raw_key)
            .body(Body::empty())
            .unwrap();
        let response =
            tokio::time::timeout(Duration::from_secs(180), router.clone().oneshot(request))
                .await
                .unwrap()
                .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = tokio::time::timeout(
            Duration::from_secs(60),
            to_bytes(response.into_body(), 8 * 1024 * 1024),
        )
        .await
        .unwrap()
        .unwrap();
        let xml = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(xml.contains("<title>Software &amp; tools</title>"));
        let attributes = protocol_attributes(&xml);
        assert_eq!(
            attributes
                .iter()
                .filter(|(name, _)| name == "custom")
                .count(),
            2
        );
        assert!(attributes.contains(&("custom".into(), "001.00".into())));
        assert_eq!(
            xml.matches("&quot;").count(),
            1024 * 1024,
            "a complete MiB-sized attribute must survive raw admission and XML streaming"
        );
        assert!(xml.contains("value=\"001.00\"") && xml.contains("value=\"release-password\""));
        assert!(!xml.contains("upstream-secret") && !xml.contains(&key.raw_key));
        retrieval = xml
            .split("<link>")
            .nth(1)
            .unwrap()
            .split("</link>")
            .next()
            .unwrap()
            .replace("&amp;", "&");
        assert!(retrieval.starts_with("http://scryer.example/newznab/api?t=get&id=nab1."));
        if let Some(previous) = previous {
            assert_eq!(
                xml, previous,
                "repeated RSS reads keep identities and enclosure URLs"
            );
        }
        previous = Some(xml);
    }
    assert_eq!(
        server.received_requests().await.unwrap().len(),
        2,
        "no caps probe or unrelated request"
    );
    // The reference remains decryptable independently of the page cache, but
    // removing its source grant refuses the grab before any upstream I/O.
    let url = url::Url::parse(&retrieval).unwrap();
    let reference = url
        .query_pairs()
        .find(|(name, _)| name == "id")
        .unwrap()
        .1
        .into_owned();
    let plaintext = cipher.open(&reference).unwrap();
    assert!(
        String::from_utf8(plaintext)
            .unwrap()
            .contains("upstream-secret")
    );
    client.configuration.grants.clear();
    client.revision += 1;
    clients.save(&client, Some(1)).await.unwrap();
    let request = Request::builder()
        .uri(format!("{}?{}", url.path(), url.query().unwrap()))
        .body(Body::empty())
        .unwrap();
    let response = tokio::time::timeout(Duration::from_secs(60), router.oneshot(request))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

/// This fixture crosses HTTP JSON parsing, the packaged WASM boundary, host
/// mapping, and XML emission. The authenticated client endpoint fixture is
/// separate so this test also isolates adapter/representation regressions.
#[tokio::test]
#[ignore = "requires SCRYER_TEST_NEWZNAB_PACKAGE from local xtask plugin validate"]
async fn real_json_upstream_search_emits_xml_and_uses_host_caps() {
    common::initialize_wasm_runtime_for_tests();
    let wasm_path = std::env::var_os("SCRYER_TEST_NEWZNAB_PACKAGE").expect("packaged Newznab path");
    let wasm_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(wasm_path);
    let wasm = std::fs::read(wasm_path).expect("packaged Newznab bytes");
    let provider = WasmIndexerPluginProvider::empty().with_external_bytes(&wasm);
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(query_param("t", "search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "channel": {"item": [{
                "title": "Software & tools", "guid": "upstream-release",
                "enclosure": {"@attributes": {"url": "https://upstream.example/get?apikey=upstream-secret",
                    "length": "1234", "type": "application/x-nzb"}},
                "category": ["4000", "Software & tools"],
                "attr": [
                    {"@attributes": {"name": "category", "value": "4000"}},
                    {"@attributes": {"name": "category", "value": "6000"}},
                    {"@attributes": {"name": "category", "value": "100123"}},
                    {"@attributes": {"name": "custom", "value": "001.00"}},
                    {"@attributes": {"name": "custom", "value": "second & <third>"}},
                    {"@attributes": {"name": "password", "value": "release-password"}}
                ]
            }]}
        }))).expect(1).mount(&server).await;
    let client = provider
        .client_for_provider(&config(&server.uri()))
        .unwrap();
    let response = tokio::time::timeout(
        Duration::from_secs(180),
        client.search_raw_text(
            RawTextSearchRequest {
                query: "software".into(),
                categories: vec!["4000".into(), "6000".into(), "100123".into()],
                limit: Some(100),
                page: Some(RawTextSearchPage { cursor: None }),
                nab: Some(
                    serde_json::from_value(serde_json::json!({"operation": "search"})).unwrap(),
                ),
                ..Default::default()
            },
            CancellationToken::new(),
        ),
    )
    .await
    .expect("bounded fixture invocation")
    .expect("JSON upstream search");
    assert_eq!(response.results.len(), 1);
    let result = &response.results[0];
    assert_eq!(result.response_attributes.original.len(), 8);
    let mut output = Vec::new();
    write_xml_feed(
        &mut output,
        NabProtocol::Newznab,
        &NabFeedPage {
            offset: 0,
            total: None,
            generated_at: chrono::DateTime::UNIX_EPOCH,
        },
        [NabFeedItem {
            result,
            external_guid: "public-release",
            retrieval_url: "https://scryer.example/newznab/api?t=get&id=reference",
        }],
    )
    .unwrap();
    let xml = String::from_utf8(output).unwrap();
    assert!(xml.starts_with("<?xml"));
    assert!(xml.contains("<title>Software &amp; tools</title>"));
    let attributes = protocol_attributes(&xml);
    for value in ["4000", "6000", "100123", "001.00", "release-password"] {
        assert!(
            attributes.iter().any(|(_, actual)| actual == value),
            "missing namespaced {value}"
        );
    }
    assert!(xml.contains("name=\"custom\" value=\"second &amp; &lt;third&gt;\""));
    assert_eq!(xml.matches("name=\"custom\"").count(), 2);
    assert!(xml.contains("<category>Software &amp; tools</category>"));
    assert!(!xml.contains("upstream-secret"));
    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests.len(),
        1,
        "host caps must avoid an upstream caps request or broader fallback"
    );
    let query: HashMap<_, _> = requests[0].url.query_pairs().collect();
    assert_eq!(query.get("q").map(|value| value.as_ref()), Some("software"));
    assert_eq!(
        query.get("cat").map(|value| value.as_ref()),
        Some("4000,6000,100123")
    );
    assert_eq!(
        query.get("apikey").map(|value| value.as_ref()),
        Some("fixture-upstream-key")
    );
}

/// A first typed search must reach the shared accounted resolver before a
/// missing or stale snapshot can classify the operation as unsupported.
#[tokio::test]
#[ignore = "requires SCRYER_TEST_NEWZNAB_PACKAGE from local xtask plugin validate"]
async fn typed_json_search_refreshes_missing_or_stale_host_caps_once_before_wasm_dispatch() {
    use scryer_infrastructure_acquisition::indexers::{
        caps::DirectNabCapsSnapshotRefresher, config_store::IndexerConfigStore,
        search_client::MultiIndexerSearchClient, stats::InMemoryIndexerStatsTracker,
    };
    use std::sync::Arc;
    common::initialize_wasm_runtime_for_tests();
    let package = std::env::var_os("SCRYER_TEST_NEWZNAB_PACKAGE").expect("packaged Newznab path");
    let package = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(package);
    let provider = Arc::new(
        WasmIndexerPluginProvider::empty().with_external_bytes(&std::fs::read(package).unwrap()),
    );
    for stale in [false, true] {
        let ctx = common::TestContext::new().await;
        let store = Arc::new(IndexerConfigStore::new(
            ctx.db.datastore(),
            ctx.db.encryption_key_state(),
        ));
        let stats = Arc::new(InMemoryIndexerStatsTracker::new(None));
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(query_param("t", "caps"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                "<caps><limits default=\"100\" max=\"100\"/><searching><tv-search available=\"yes\" supportedParams=\"q,tvdbid\"/></searching></caps>"
            )).expect(1).mount(&server).await;
        Mock::given(method("GET"))
            .and(query_param("t", "tvsearch"))
            .and(query_param("tvdbid", "12345"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "channel": {"item": [{"title": "Typed JSON release", "guid": "typed-json-release"}]}
            })))
            .expect(2)
            .mount(&server)
            .await;
        let mut source = config(&server.uri());
        source.caps_snapshot_json = stale.then(|| {
            serde_json::json!({
                "parser_version": scryer_domain::INDEXER_CAPS_PARSER_VERSION,
                "cache_fetched_at": "2000-01-01T00:00:00Z", "searching_present": true,
                "search": {"available": true, "supported_params": ["q"]}
            })
            .to_string()
        });
        let id = source.id.clone();
        store.create(source).await.unwrap();
        let refresher = Arc::new(
            DirectNabCapsSnapshotRefresher::new().with_indexer_stats_tracker(stats.clone()),
        );
        let multi = MultiIndexerSearchClient::new(store.clone(), stats, provider.clone())
            .with_caps_resolution(
                Arc::new(scryer_application::CapsRequestCache::default()),
                refresher,
            );
        let request = RawTextSearchRequest {
            nab: Some(
                serde_json::from_value(serde_json::json!({
                    "operation": "tv", "parameters": {"tvdbid": "12345"}
                }))
                .unwrap(),
            ),
            indexer_ids: Some(std::collections::HashSet::from([id.clone()])),
            page: Some(Default::default()),
            ..Default::default()
        };
        for _ in 0..2 {
            let response = tokio::time::timeout(
                Duration::from_secs(180),
                multi.search_raw_text(request.clone(), CancellationToken::new()),
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(response.results.len(), 1);
            assert_eq!(response.results[0].title, "Typed JSON release");
        }
        let stored = store.get_by_id(&id).await.unwrap().unwrap();
        let resolved = scryer_application::cached_indexer_caps(&stored, chrono::Utc::now());
        assert_eq!(
            resolved.source,
            scryer_application::IndexerCapsSource::Fresh
        );
        assert!(resolved.snapshot.unwrap().tv_search.unwrap().available);
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 3);
        assert_eq!(
            requests[0]
                .url
                .query_pairs()
                .find(|(name, _)| name == "t")
                .unwrap()
                .1,
            "caps"
        );
        for request in &requests[1..] {
            assert!(
                !request.url.query_pairs().any(|(name, _)| name == "q"),
                "no broader text fallback"
            );
        }
    }
}

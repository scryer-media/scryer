use std::path::Path;
use std::sync::Arc;

use chrono::Utc;
use scryer_application::IndexerPluginProvider;
use scryer_domain::IndexerConfig;

fn test_config(provider_type: &str) -> IndexerConfig {
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
        last_health_status: None,
        last_error_at: None,
        config_json: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

#[test]
fn load_test_indexer_plugin() {
    let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let provider = scryer_plugins::load_indexer_plugins(&fixtures_dir).unwrap();

    let types = provider.available_provider_types();
    assert_eq!(types, vec!["test"]);
}

#[test]
fn test_indexer_creates_client() {
    let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let provider = scryer_plugins::load_indexer_plugins(&fixtures_dir).unwrap();

    let client = provider.client_for_provider(&test_config("test"));
    assert!(
        client.is_some(),
        "should create a client for provider_type 'test'"
    );
}

#[test]
fn unknown_provider_returns_none() {
    let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let provider = scryer_plugins::load_indexer_plugins(&fixtures_dir).unwrap();

    assert!(
        provider
            .client_for_provider(&test_config("nonexistent"))
            .is_none()
    );
}

#[tokio::test]
async fn test_indexer_search() {
    let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let provider = scryer_plugins::load_indexer_plugins(&fixtures_dir).unwrap();

    let client = provider.client_for_provider(&test_config("test")).unwrap();

    use scryer_application::SearchMode;
    let results = client
        .search(
            "Dune Part Two".to_string(),
            std::collections::HashMap::new(),
            None,
            None,
            None,
            None,
            SearchMode::Auto,
            None,
            None,
            None,
            vec![],
        )
        .await
        .unwrap()
        .results;

    assert_eq!(results.len(), 1);
    let r = &results[0];
    assert!(r.title.contains("Dune Part Two"));
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
    let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let provider = scryer_plugins::load_indexer_plugins(&fixtures_dir).unwrap();
    // The test-indexer fixture has no scoring policies
    assert!(provider.scoring_policies().is_empty());
}

// ── WasmIndexerPluginProvider builder tests ──────────────────────────────────

#[test]
fn builtin_loads_nzbgeek_and_newznab() {
    let provider = scryer_plugins::WasmIndexerPluginProvider::empty()
        .with_builtin(scryer_plugins::builtins::NZBGEEK_WASM)
        .with_builtin(scryer_plugins::builtins::NEWZNAB_WASM);

    let mut types = provider.available_provider_types();
    types.sort();
    assert!(types.contains(&"nzbgeek".to_string()));
    assert!(types.contains(&"newznab".to_string()));
}

#[test]
fn external_overrides_builtin_same_provider() {
    // Load test fixture (provider_type = "test") as external, then try
    // loading it again as builtin — builtin should be skipped.
    let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let wasm_bytes = std::fs::read(fixtures_dir.join("test-indexer/plugin.wasm")).unwrap();

    let provider = scryer_plugins::WasmIndexerPluginProvider::empty()
        .with_external_bytes(&wasm_bytes)
        .with_builtin(&wasm_bytes); // same provider_type — should be skipped

    // Only one entry for "test", not duplicated
    let types = provider.available_provider_types();
    assert_eq!(
        types.iter().filter(|t| *t == "test").count(),
        1,
        "builtin should not duplicate external"
    );
}

#[test]
fn without_provider_type_removes() {
    let provider = scryer_plugins::WasmIndexerPluginProvider::empty()
        .with_builtin(scryer_plugins::builtins::NZBGEEK_WASM)
        .with_builtin(scryer_plugins::builtins::NEWZNAB_WASM)
        .without_provider_type("nzbgeek");

    let types = provider.available_provider_types();
    assert!(!types.contains(&"nzbgeek".to_string()));
    assert!(types.contains(&"newznab".to_string()));
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
        .with_builtin(scryer_plugins::builtins::NZBGEEK_WASM)
        .with_external_bytes(b"garbage");

    let types = provider.available_provider_types();
    assert!(
        types.contains(&"nzbgeek".to_string()),
        "valid builtin should survive despite garbage external"
    );
}

#[test]
fn plugin_name_and_default_url_accessible() {
    let provider = scryer_plugins::WasmIndexerPluginProvider::empty()
        .with_builtin(scryer_plugins::builtins::NZBGEEK_WASM)
        .with_builtin(scryer_plugins::builtins::NEWZNAB_WASM);

    assert!(
        provider.plugin_name_for_provider("nzbgeek").is_some(),
        "nzbgeek should have a plugin name"
    );
    assert!(
        provider.plugin_name_for_provider("newznab").is_some(),
        "newznab should have a plugin name"
    );
    assert_eq!(
        provider.default_base_url_for_provider("nzbgeek").as_deref(),
        Some("https://api.nzbgeek.info")
    );
    assert!(provider.default_base_url_for_provider("newznab").is_none());
}

#[test]
fn dognzb_hides_generic_newznab_config_fields() {
    let provider = scryer_plugins::WasmIndexerPluginProvider::empty()
        .with_builtin(scryer_plugins::builtins::DOGNZB_WASM);

    assert_eq!(
        provider.default_base_url_for_provider("dognzb").as_deref(),
        Some("https://api.dognzb.cr")
    );

    let fields = provider.config_fields_for_provider("dognzb");
    let field_keys: Vec<&str> = fields.iter().map(|field| field.key.as_str()).collect();

    assert!(
        !field_keys.contains(&"api_path"),
        "DogNZB should not expose api_path"
    );
    assert!(
        !field_keys.contains(&"additional_params"),
        "DogNZB should not expose additional_params"
    );
}

// ── DynamicPluginProvider tests ──────────────────────────────────────────────

#[test]
fn dynamic_delegates_available_types() {
    let inner = scryer_plugins::WasmIndexerPluginProvider::empty()
        .with_builtin(scryer_plugins::builtins::NZBGEEK_WASM);
    let dynamic = scryer_plugins::DynamicPluginProvider::new(inner);

    let types = dynamic.available_provider_types();
    assert!(types.contains(&"nzbgeek".to_string()));
}

#[test]
fn dynamic_reload_clears_cache() {
    let inner = scryer_plugins::WasmIndexerPluginProvider::empty()
        .with_builtin(scryer_plugins::builtins::NZBGEEK_WASM)
        .with_builtin(scryer_plugins::builtins::NEWZNAB_WASM);
    let dynamic = scryer_plugins::DynamicPluginProvider::new(inner);

    assert_eq!(dynamic.available_provider_types().len(), 2);

    // Reload with empty provider
    dynamic.reload(scryer_plugins::WasmIndexerPluginProvider::empty());

    assert!(
        dynamic.available_provider_types().is_empty(),
        "after reload with empty, should have no types"
    );
}

#[test]
fn dynamic_reload_plugins_disables_builtin() {
    let inner = scryer_plugins::WasmIndexerPluginProvider::empty()
        .with_builtin(scryer_plugins::builtins::NZBGEEK_WASM)
        .with_builtin(scryer_plugins::builtins::NEWZNAB_WASM);
    let dynamic = scryer_plugins::DynamicPluginProvider::new(inner);

    // Use the trait method to reload with nzbgeek disabled
    dynamic
        .reload_plugins(&[], &["nzbgeek".to_string()])
        .unwrap();

    let types = dynamic.available_provider_types();
    assert!(
        !types.contains(&"nzbgeek".to_string()),
        "nzbgeek should be disabled"
    );
    assert!(
        types.contains(&"newznab".to_string()),
        "newznab should remain"
    );
}

#[test]
fn dynamic_client_cache_hit() {
    let inner = scryer_plugins::WasmIndexerPluginProvider::empty()
        .with_builtin(scryer_plugins::builtins::NZBGEEK_WASM);
    let dynamic = scryer_plugins::DynamicPluginProvider::new(inner);

    let config = test_config("nzbgeek");
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
        .with_builtin(scryer_plugins::builtins::NZBGEEK_WASM);
    let dynamic = scryer_plugins::DynamicPluginProvider::new(inner);

    let mut config1 = test_config("nzbgeek");
    let c1 = dynamic.client_for_provider(&config1).unwrap();

    // Change updated_at to simulate a config update
    config1.updated_at = Utc::now() + chrono::Duration::seconds(10);
    let c2 = dynamic.client_for_provider(&config1).unwrap();

    assert!(
        !Arc::ptr_eq(&c1, &c2),
        "different updated_at should produce a new client"
    );
}

// ── Builder validation tests ─────────────────────────────────────────────────

#[test]
fn builtin_with_valid_descriptor_loads() {
    let provider = scryer_plugins::WasmIndexerPluginProvider::empty()
        .with_builtin(scryer_plugins::builtins::NZBGEEK_WASM);

    assert!(
        provider
            .available_provider_types()
            .contains(&"nzbgeek".to_string()),
        "NZBGEEK_WASM should register as 'nzbgeek'"
    );
}

#[test]
fn plugin_capabilities_accessible() {
    let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let provider = scryer_plugins::load_indexer_plugins(&fixtures_dir).unwrap();

    let caps = provider.capabilities_for_provider("test");
    assert!(caps.rss, "rss capability should default to true");
    // The test plugin should have some capabilities declared
    // (the default is all-true, so at minimum search should be true)
    assert!(caps.search, "search capability should be true");
}

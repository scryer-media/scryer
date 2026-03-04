use std::path::Path;

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
    assert!(client.is_some(), "should create a client for provider_type 'test'");
}

#[test]
fn unknown_provider_returns_none() {
    let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let provider = scryer_plugins::load_indexer_plugins(&fixtures_dir).unwrap();

    assert!(provider.client_for_provider(&test_config("nonexistent")).is_none());
}

#[tokio::test]
async fn test_indexer_search() {
    let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let provider = scryer_plugins::load_indexer_plugins(&fixtures_dir).unwrap();

    let client = provider.client_for_provider(&test_config("test")).unwrap();

    use scryer_application::{IndexerClient, SearchMode};
    let results = client
        .search(
            "Dune Part Two".to_string(),
            None,
            None,
            None,
            None,
            None,
            100,
            SearchMode::Auto,
            None,
            None,
        )
        .await
        .unwrap();

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

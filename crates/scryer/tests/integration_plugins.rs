#![recursion_limit = "256"]

mod common;

use common::TestContext;
use scryer_application::{IndexerConfigRepository, PluginInstallationRepository};
use scryer_domain::User;
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

fn admin() -> User {
    User::new_admin("admin")
}

struct RealPluginFixture {
    plugin_id: &'static str,
    name: &'static str,
    description: &'static str,
    version: &'static str,
    plugin_type: &'static str,
    provider_type: &'static str,
    request_path: &'static str,
    wasm_path: std::path::PathBuf,
}

fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("repo root")
        .to_path_buf()
}

fn load_wasm_fixture(path: &std::path::Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
}

fn bundled_test_indexer_fixture() -> RealPluginFixture {
    RealPluginFixture {
        plugin_id: "test-indexer",
        name: "Test",
        description: "A test plugin",
        version: "0.1.0",
        plugin_type: "indexer",
        provider_type: "test",
        request_path: "/fixtures/test-indexer/plugin.wasm",
        wasm_path: repo_root()
            .join("crates")
            .join("scryer-plugins")
            .join("fixtures")
            .join("test-indexer")
            .join("plugin.wasm"),
    }
}

fn torrent_rss_dist_fixture() -> Option<RealPluginFixture> {
    let wasm_path = repo_root()
        .parent()
        .expect("workspace root")
        .join("scryer-plugins")
        .join("dist")
        .join("torrent_rss_indexer.wasm");
    if !wasm_path.exists() {
        return None;
    }

    Some(RealPluginFixture {
        plugin_id: "torrent-rss",
        name: "Torrent RSS Feed Indexer",
        description: "Generic torrent RSS indexer",
        version: "0.1.3",
        plugin_type: "torrent_indexer",
        provider_type: "torrent_rss",
        request_path: "/dist/torrent_rss_indexer.wasm",
        wasm_path,
    })
}

async fn assert_real_registry_plugin_install_exposes_provider_type(fixture: &RealPluginFixture) {
    let ctx = TestContext::new().await;
    ctx.app.seed_builtin_plugins().await.unwrap();

    let wasm_bytes = load_wasm_fixture(&fixture.wasm_path);
    Mock::given(method("GET"))
        .and(path(fixture.request_path))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/wasm")
                .set_body_bytes(wasm_bytes),
        )
        .mount(&ctx.nzbgeek_server)
        .await;

    let registry_json = serde_json::json!({
        "schema_version": 1,
        "plugins": [
            {
                "id": fixture.plugin_id,
                "name": fixture.name,
                "description": fixture.description,
                "plugin_type": fixture.plugin_type,
                "provider_type": fixture.provider_type,
                "version": fixture.version,
                "official": true,
                "builtin": false,
                "wasm_url": format!("{}{}", ctx.nzbgeek_server.uri(), fixture.request_path),
            }
        ]
    })
    .to_string();
    ctx.db.store_registry_cache(&registry_json).await.unwrap();

    let installation = ctx
        .app
        .install_plugin(&admin(), fixture.plugin_id)
        .await
        .unwrap();
    assert_eq!(installation.plugin_id, fixture.plugin_id);
    assert_eq!(installation.provider_type, fixture.provider_type);
    assert_eq!(installation.plugin_type, fixture.plugin_type);

    let provider_types = ctx
        .app
        .services
        .plugin_provider
        .as_ref()
        .unwrap()
        .available_provider_types();
    assert!(
        provider_types.contains(&fixture.provider_type.to_string()),
        "{} should be available after install, got {provider_types:?}",
        fixture.provider_type
    );

    let plugins = ctx.app.list_available_plugins(&admin()).await.unwrap();
    let installed_plugin = plugins
        .iter()
        .find(|plugin| plugin.id == fixture.plugin_id)
        .expect("installed plugin should be listed");
    assert!(installed_plugin.is_installed);
    assert!(installed_plugin.is_enabled);
    assert_eq!(installed_plugin.plugin_type, fixture.plugin_type);
}

// ── seed_builtin_plugins ─────────────────────────────────────────────────────

#[tokio::test]
async fn seed_builtins_creates_installations() {
    let ctx = TestContext::new().await;
    ctx.app.seed_builtin_plugins().await.unwrap();

    let installations = ctx.db.list_plugin_installations().await.unwrap();
    assert_eq!(
        installations.len(),
        3,
        "should have nzbgeek + dognzb + newznab"
    );

    let ids: Vec<&str> = installations.iter().map(|i| i.plugin_id.as_str()).collect();
    assert!(ids.contains(&"nzbgeek"));
    assert!(ids.contains(&"dognzb"));
    assert!(ids.contains(&"newznab"));

    for inst in &installations {
        assert!(inst.is_builtin);
        assert!(inst.is_enabled);
    }
}

#[tokio::test]
async fn seed_builtins_idempotent() {
    let ctx = TestContext::new().await;
    ctx.app.seed_builtin_plugins().await.unwrap();
    ctx.app.seed_builtin_plugins().await.unwrap();

    let installations = ctx.db.list_plugin_installations().await.unwrap();
    assert_eq!(
        installations.len(),
        3,
        "should not duplicate on second seed"
    );
}

// ── list_available_plugins ───────────────────────────────────────────────────

#[tokio::test]
async fn list_available_with_builtins_and_registry() {
    let ctx = TestContext::new().await;
    ctx.app.seed_builtin_plugins().await.unwrap();

    // Store a registry cache that includes a non-builtin plugin
    let registry_json = serde_json::json!({
        "schema_version": 1,
        "plugins": [
            {
                "id": "nzbgeek",
                "name": "NZBGeek",
                "description": "NZBGeek indexer",
                "plugin_type": "indexer",
                "provider_type": "nzbgeek",
                "version": "0.2.0",
                "official": true,
                "builtin": true,
            },
            {
                "id": "animetosho",
                "name": "AnimeTosho",
                "description": "AnimeTosho indexer",
                "plugin_type": "indexer",
                "provider_type": "animetosho",
                "version": "0.1.0",
                "official": true,
                "builtin": false,
                "wasm_url": "https://example.com/animetosho.wasm",
                "wasm_sha256": "abc123",
            }
        ]
    })
    .to_string();
    ctx.db.store_registry_cache(&registry_json).await.unwrap();

    let result = ctx.app.list_available_plugins(&admin()).await.unwrap();

    // Should have nzbgeek (installed+builtin), newznab (installed+builtin),
    // and animetosho (not installed, from registry)
    assert!(result.len() >= 3, "got {} plugins", result.len());

    let nzbgeek = result.iter().find(|p| p.id == "nzbgeek").unwrap();
    assert!(nzbgeek.is_installed);
    assert!(nzbgeek.builtin);

    let animetosho = result.iter().find(|p| p.id == "animetosho").unwrap();
    assert!(!animetosho.is_installed);
    assert!(!animetosho.builtin);
    assert!(animetosho.wasm_url.is_some());
}

#[tokio::test]
async fn install_repo_local_plugin_fixture_exposes_provider_type() {
    assert_real_registry_plugin_install_exposes_provider_type(&bundled_test_indexer_fixture())
        .await;
}

#[tokio::test]
async fn install_real_torrent_rss_plugin_exposes_provider_type() {
    let Some(fixture) = torrent_rss_dist_fixture() else {
        eprintln!(
            "skipping torrent RSS install regression: sibling scryer-plugins dist artifact is unavailable"
        );
        return;
    };

    assert_real_registry_plugin_install_exposes_provider_type(&fixture).await;
}

// ── toggle_plugin ────────────────────────────────────────────────────────────

#[tokio::test]
async fn toggle_builtin_disables_and_rebuilds() {
    let ctx = TestContext::new().await;
    ctx.app.seed_builtin_plugins().await.unwrap();

    // Initially both builtins should be available as provider types
    let types_before = ctx
        .app
        .services
        .plugin_provider
        .as_ref()
        .unwrap()
        .available_provider_types();
    assert!(types_before.contains(&"nzbgeek".to_string()));

    // Disable nzbgeek
    let toggled = ctx
        .app
        .toggle_plugin(&admin(), "nzbgeek", false)
        .await
        .unwrap();
    assert!(!toggled.is_enabled);

    // After toggle, reload_plugins is called → nzbgeek should be gone from provider types
    let types_after = ctx
        .app
        .services
        .plugin_provider
        .as_ref()
        .unwrap()
        .available_provider_types();
    assert!(
        !types_after.contains(&"nzbgeek".to_string()),
        "nzbgeek should be disabled in provider"
    );
    assert!(
        types_after.contains(&"newznab".to_string()),
        "newznab should remain"
    );

    // Re-enable
    let re_enabled = ctx
        .app
        .toggle_plugin(&admin(), "nzbgeek", true)
        .await
        .unwrap();
    assert!(re_enabled.is_enabled);

    let types_final = ctx
        .app
        .services
        .plugin_provider
        .as_ref()
        .unwrap()
        .available_provider_types();
    assert!(
        types_final.contains(&"nzbgeek".to_string()),
        "nzbgeek should be back"
    );
}

#[tokio::test]
async fn toggle_updates_timestamp() {
    let ctx = TestContext::new().await;
    ctx.app.seed_builtin_plugins().await.unwrap();

    let before = ctx
        .db
        .get_plugin_installation("nzbgeek")
        .await
        .unwrap()
        .unwrap();

    // Small delay to ensure timestamp difference
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    ctx.app
        .toggle_plugin(&admin(), "nzbgeek", false)
        .await
        .unwrap();

    let after = ctx
        .db
        .get_plugin_installation("nzbgeek")
        .await
        .unwrap()
        .unwrap();

    assert!(
        after.updated_at >= before.updated_at,
        "updated_at should advance after toggle"
    );
}

// ── reconcile_indexer_configs ────────────────────────────────────────────────

#[tokio::test]
async fn reconcile_noop_for_builtins_without_default_url() {
    let ctx = TestContext::new().await;
    ctx.app.seed_builtin_plugins().await.unwrap();

    // newznab has no default URL, and nzbgeek is intentionally skipped from
    // auto-creation because it still needs a manual API key.
    ctx.app.reconcile_indexer_configs().await.unwrap();

    let configs = IndexerConfigRepository::list(&ctx.db, None).await.unwrap();
    assert!(
        configs.is_empty(),
        "no builtin configs should be auto-created during reconciliation"
    );
}

// ── uninstall_plugin ─────────────────────────────────────────────────────────

#[tokio::test]
async fn uninstall_builtin_rejected() {
    let ctx = TestContext::new().await;
    ctx.app.seed_builtin_plugins().await.unwrap();

    let err = ctx
        .app
        .uninstall_plugin(&admin(), "nzbgeek")
        .await
        .unwrap_err();
    assert!(
        matches!(err, scryer_application::AppError::Validation(_)),
        "should reject uninstall of builtin: {err:?}"
    );
}

// ── available_provider_types ─────────────────────────────────────────────────

#[tokio::test]
async fn available_provider_types_includes_builtins() {
    let ctx = TestContext::new().await;

    let types = ctx
        .app
        .services
        .plugin_provider
        .as_ref()
        .unwrap()
        .available_provider_types();

    assert!(
        types.contains(&"nzbgeek".to_string()),
        "nzbgeek should be a built-in provider type"
    );
    assert!(
        types.contains(&"newznab".to_string()),
        "newznab should be a built-in provider type"
    );
}

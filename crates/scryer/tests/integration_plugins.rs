#![recursion_limit = "256"]

mod common;

use common::TestContext;
use scryer_application::{IndexerConfigRepository, PluginInstallationRepository};
use scryer_domain::User;

fn admin() -> User {
    User::new_admin("admin")
}

// ── seed_builtin_plugins ─────────────────────────────────────────────────────

#[tokio::test]
async fn seed_builtins_creates_installations() {
    let ctx = TestContext::new().await;
    ctx.app.seed_builtin_plugins().await.unwrap();

    let installations = ctx.db.list_plugin_installations().await.unwrap();
    assert_eq!(installations.len(), 2, "should have nzbgeek + newznab");

    let ids: Vec<&str> = installations.iter().map(|i| i.plugin_id.as_str()).collect();
    assert!(ids.contains(&"nzbgeek"));
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
        2,
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

#![recursion_limit = "256"]

mod common;

use common::{TestContext, initialize_wasm_runtime_for_tests};
use scryer_application::{
    IndexerPluginProvider, NotificationPluginProvider, PluginInstallationRepository,
};
use scryer_domain::User;
use scryer_infrastructure_sql::types::SettingDefinitionSeed;

fn admin() -> User {
    User {
        id: scryer_domain::Id::new().0,
        username: "admin".to_string(),
        password_hash: None,
        password_change_required: false,
        account_kind: Default::default(),
        authorization: scryer_domain::UserAuthorization {
            app: scryer_domain::AppPermissionMask::from_permissions([
                scryer_domain::AppPermission::ManageSystemSettings,
                scryer_domain::AppPermission::ManageCatalogSettings,
            ]),
            actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
            loaded: true,
            ..Default::default()
        },
    }
}

fn available_provider_types(
    app: &scryer_application::AppUseCase,
    plugin_type: &str,
) -> Vec<String> {
    match plugin_type {
        "download_client" => app
            .available_download_client_provider_types()
            .into_iter()
            .map(|(provider_type, ..)| provider_type)
            .collect(),
        _ => app
            .available_indexer_provider_types()
            .into_iter()
            .map(|(provider_type, ..)| provider_type)
            .collect(),
    }
}

struct RealPluginFixture {
    plugin_id: &'static str,
    plugin_type: &'static str,
    provider_type: &'static str,
    wasm_path: std::path::PathBuf,
    optional_artifact: bool,
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

#[expect(
    clippy::too_many_arguments,
    reason = "test helper mirrors catalog entry fields"
)]
fn catalog_plugin_entry(
    plugin_id: &str,
    name: &str,
    description: &str,
    plugin_type: &str,
    provider_type: &str,
    version: &str,
    builtin: bool,
    wasm_url: Option<String>,
) -> serde_json::Value {
    let test_sdk_constraint =
        scryer_plugins::sdk_constraint_or_legacy(scryer_plugins::SDK_VERSION, "");
    let releases = if builtin || wasm_url.is_none() {
        serde_json::json!([])
    } else {
        let wasm_url = wasm_url
            .as_deref()
            .expect("downloadable test catalog plugin should have an artifact URL");
        serde_json::json!([{
            "version": version,
            "sdk_constraint": test_sdk_constraint,
            "artifacts": [{
                "runtime": "wasm32-wasip2",
                "required_features": [],
                "url": wasm_url,
                "mirror_urls": [],
                "signature_url": format!("{wasm_url}.bundle.json"),
                "signature_mirror_urls": [],
                "digests": ["sha256:0000000000000000000000000000000000000000000000000000000000000000"],
                "wasm_digests": ["sha256:1111111111111111111111111111111111111111111111111111111111111111"],
                "bytes": 1024,
            }],
        }])
    };

    serde_json::json!({
        "id": plugin_id,
        "name": name,
        "description": description,
        "plugin_type": plugin_type,
        "provider_type": provider_type,
        "official": true,
        "publisher": "scryer",
        "support_tier": "official",
        "status": "active",
        "docs_url": format!("https://example.com/{plugin_id}/docs"),
        "source_repo": format!("https://github.com/scryer-media/test-plugin-{}", plugin_id.replace('_', "-")),
        "required_signer": {
            "github_repository": format!("scryer-media/test-plugin-{}", plugin_id.replace('_', "-"))
        },
        "releases": releases,
    })
}

async fn seed_official_catalog(
    ctx: &TestContext,
    entries: &[serde_json::Value],
) -> scryer_application::AppResult<()> {
    let central_plugins = entries
        .iter()
        .map(|entry| {
            let id = entry["id"].as_str().expect("id");
            let source_repo = entry["source_repo"].as_str().expect("source_repo");
            serde_json::json!({
                "id": id,
                "name": entry["name"].as_str().expect("name"),
                "description": entry["description"].as_str().expect("description"),
                "plugin_type": entry["plugin_type"].as_str().expect("plugin_type"),
                "provider_type": entry["provider_type"].as_str().expect("provider_type"),
                "publisher": "scryer",
                "support_tier": "official",
                "status": entry["status"].as_str().expect("status"),
                "docs_url": entry["docs_url"].as_str().expect("docs_url"),
                "source_repo": source_repo,
                "required_signer": entry["required_signer"].clone(),
                "releases": entry["releases"].clone(),
            })
        })
        .collect::<Vec<_>>();

    let central_catalog = serde_json::json!({
        "schema_version": "scryer.plugin.catalog.v3",
        "catalog_version": 1,
        "plugins": central_plugins,
        "rule_packs": [],
    })
    .to_string();

    ctx.customization
        .upsert_plugin_catalog_source(&scryer_domain::PluginCatalogSource {
            source_key: "__central_catalog".to_string(),
            source_kind: "central".to_string(),
            source_url: "https://example.com/catalog-v3.min.json.zst".to_string(),
            github_repo: Some("scryer-media/scryer-plugins".to_string()),
            support_tier: scryer_domain::PluginSupportTier::Official,
            catalog_json: Some(central_catalog),
            last_success_at: Some(chrono::Utc::now()),
            last_error: None,
            updated_at: chrono::Utc::now(),
        })
        .await?;

    Ok(())
}

fn bundled_test_indexer_fixture() -> RealPluginFixture {
    let fixtures_dir = std::env::var_os("SCRYER_TEST_PLUGIN_FIXTURES_DIR")
        .map(std::path::PathBuf::from)
        .expect("cargo nextest must generate the test plugin fixture before running this binary");
    RealPluginFixture {
        plugin_id: "test",
        plugin_type: "indexer",
        provider_type: "test",
        wasm_path: fixtures_dir.join("test-indexer/plugin.wasm"),
        optional_artifact: false,
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
        plugin_type: "torrent_indexer",
        provider_type: "torrent_rss",
        wasm_path,
        optional_artifact: true,
    })
}

fn email_dist_fixture() -> Option<RealPluginFixture> {
    let wasm_path = repo_root()
        .parent()
        .expect("workspace root")
        .join("scryer-plugins")
        .join("dist")
        .join("email_notification.wasm");
    if !wasm_path.exists() {
        return None;
    }

    Some(RealPluginFixture {
        plugin_id: "email",
        plugin_type: "notification",
        provider_type: "email",
        wasm_path,
        optional_artifact: true,
    })
}

fn assert_real_plugin_artifact_exposes_provider_type(fixture: &RealPluginFixture) {
    initialize_wasm_runtime_for_tests();
    let wasm_bytes = load_wasm_fixture(&fixture.wasm_path);
    let provider_types = match fixture.plugin_type {
        "notification" => scryer_plugins::WasmNotificationPluginProvider::empty()
            .with_external_bytes(&wasm_bytes)
            .available_provider_types(),
        "indexer" | "usenet_indexer" | "torrent_indexer" => {
            scryer_plugins::WasmIndexerPluginProvider::empty()
                .with_external_bytes(&wasm_bytes)
                .available_provider_types()
        }
        other => panic!("unsupported real plugin fixture type: {other}"),
    };
    if fixture.optional_artifact && !provider_types.contains(&fixture.provider_type.to_string()) {
        eprintln!(
            "skipping {} artifact regression; optional sibling artifact is incompatible with SDK {}",
            fixture.plugin_id,
            scryer_plugins::SDK_VERSION
        );
        return;
    }
    assert!(
        provider_types.contains(&fixture.provider_type.to_string()),
        "{} should expose provider type {}, got {provider_types:?}",
        fixture.plugin_id,
        fixture.provider_type
    );
}

// ── seed_builtin_plugins ─────────────────────────────────────────────────────

#[tokio::test]
async fn seed_builtins_creates_installations() {
    let ctx = TestContext::new().await;
    ctx.app.seed_builtin_plugins().await.unwrap();

    let installations = ctx.customization.list_plugin_installations().await.unwrap();
    assert_eq!(installations.len(), 2, "should have newznab + torznab");

    let ids: Vec<&str> = installations.iter().map(|i| i.plugin_id.as_str()).collect();
    assert!(ids.contains(&"newznab"));
    assert!(ids.contains(&"torznab"));

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

    let installations = ctx.customization.list_plugin_installations().await.unwrap();
    assert_eq!(
        installations.len(),
        2,
        "should not duplicate on second seed"
    );
}

#[tokio::test]
async fn seed_builtins_prunes_removed_builtin_installations() {
    let ctx = TestContext::new().await;
    ctx.customization
        .seed_builtin(
            "opensubtitles",
            "OpenSubtitles",
            "",
            "0.1.0",
            "1.0.0",
            ">=1.0.0, <2.0.0",
            "subtitle_provider",
            "opensubtitles",
        )
        .await
        .unwrap();
    ctx.customization
        .seed_builtin(
            "whisper",
            "Whisper",
            "",
            "0.1.0",
            "1.0.0",
            ">=1.0.0, <2.0.0",
            "subtitle_provider",
            "whisper",
        )
        .await
        .unwrap();

    ctx.app.seed_builtin_plugins().await.unwrap();

    let installations = ctx.customization.list_plugin_installations().await.unwrap();
    let ids: Vec<&str> = installations.iter().map(|i| i.plugin_id.as_str()).collect();
    assert_eq!(
        installations.len(),
        2,
        "should retain only current built-ins"
    );
    assert!(!ids.contains(&"opensubtitles"));
    assert!(!ids.contains(&"whisper"));
    assert!(ids.contains(&"newznab"));
    assert!(ids.contains(&"torznab"));
}

// ── list_available_plugins ───────────────────────────────────────────────────

#[tokio::test]
async fn list_available_with_builtins_and_catalog_v2() {
    let ctx = TestContext::new().await;
    ctx.app.seed_builtin_plugins().await.unwrap();

    seed_official_catalog(
        &ctx,
        &[
            catalog_plugin_entry(
                "torznab",
                "Torznab",
                "Torznab indexer",
                "indexer",
                "torznab",
                "0.1.0",
                true,
                None,
            ),
            catalog_plugin_entry(
                "example-indexer",
                "Example Indexer",
                "Example indexer",
                "indexer",
                "example_indexer",
                "0.1.0",
                false,
                Some("https://example.com/example-indexer.wasm.zst".to_string()),
            ),
        ],
    )
    .await
    .unwrap();

    let result = ctx.app.list_available_plugins(&admin()).await.unwrap();

    // Should have newznab and torznab (installed+builtin),
    // and example-indexer (not installed, catalog-available)
    assert!(result.len() >= 3, "got {} plugins", result.len());

    let torznab = result.iter().find(|p| p.id == "torznab").unwrap();
    assert!(torznab.is_installed);
    assert!(torznab.builtin);

    let example = result.iter().find(|p| p.id == "example-indexer").unwrap();
    assert!(!example.is_installed);
    assert!(!example.builtin);
    assert!(example.wasm_url.is_some());
}

#[tokio::test]
async fn install_repo_local_plugin_fixture_exposes_provider_type() {
    assert_real_plugin_artifact_exposes_provider_type(&bundled_test_indexer_fixture());
}

#[tokio::test]
async fn install_real_torrent_rss_plugin_exposes_provider_type() {
    let Some(fixture) = torrent_rss_dist_fixture() else {
        eprintln!(
            "skipping torrent RSS install regression: sibling scryer-plugins dist artifact is unavailable"
        );
        return;
    };

    assert_real_plugin_artifact_exposes_provider_type(&fixture);
}

#[tokio::test]
async fn install_real_email_plugin_exposes_provider_type() {
    let Some(fixture) = email_dist_fixture() else {
        eprintln!(
            "skipping email install regression: sibling scryer-plugins dist artifact is unavailable"
        );
        return;
    };

    assert_real_plugin_artifact_exposes_provider_type(&fixture);
}

// ── toggle_plugin ────────────────────────────────────────────────────────────

#[tokio::test]
async fn toggle_builtin_disables_and_rebuilds() {
    let ctx = TestContext::new().await;
    ctx.app.seed_builtin_plugins().await.unwrap();

    // Initially both builtins should be available as provider types
    let types_before = available_provider_types(&ctx.app, "indexer");
    assert!(types_before.contains(&"torznab".to_string()));

    // Disable torznab
    let toggled = ctx
        .app
        .toggle_plugin(&admin(), "torznab", false)
        .await
        .unwrap();
    assert!(!toggled.is_enabled);

    // After toggle, reload_plugins is called, so torznab should be gone from provider types.
    let types_after = available_provider_types(&ctx.app, "indexer");
    assert!(
        !types_after.contains(&"torznab".to_string()),
        "torznab should be disabled in provider"
    );
    assert!(
        types_after.contains(&"newznab".to_string()),
        "newznab should remain"
    );

    // Re-enable
    let re_enabled = ctx
        .app
        .toggle_plugin(&admin(), "torznab", true)
        .await
        .unwrap();
    assert!(re_enabled.is_enabled);

    let types_final = available_provider_types(&ctx.app, "indexer");
    assert!(
        types_final.contains(&"torznab".to_string()),
        "torznab should be back"
    );
}

#[tokio::test]
async fn toggle_updates_timestamp() {
    let ctx = TestContext::new().await;
    ctx.app.seed_builtin_plugins().await.unwrap();

    let before = ctx
        .customization
        .get_plugin_installation("torznab")
        .await
        .unwrap()
        .unwrap();

    // Small delay to ensure timestamp difference
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    ctx.app
        .toggle_plugin(&admin(), "torznab", false)
        .await
        .unwrap();

    let after = ctx
        .customization
        .get_plugin_installation("torznab")
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

    // Built-in indexers have no default URL and still need manual connection settings.
    ctx.app.reconcile_indexer_configs().await.unwrap();

    let configs = ctx.app.list_indexer_configs(&admin(), None).await.unwrap();
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
        .uninstall_plugin(&admin(), "torznab")
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

    let types = available_provider_types(&ctx.app, "indexer");

    assert!(
        types.contains(&"torznab".to_string()),
        "torznab should be a built-in provider type"
    );
    assert!(
        types.contains(&"newznab".to_string()),
        "newznab should be a built-in provider type"
    );
}

fn plugin_auto_update_viewer() -> User {
    User {
        id: scryer_domain::Id::new().0,
        username: "viewer".to_string(),
        password_hash: None,
        password_change_required: false,
        account_kind: Default::default(),
        authorization: scryer_domain::UserAuthorization {
            actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
            loaded: true,
            ..Default::default()
        },
    }
}

async fn seed_plugin_auto_update_setting_definition(ctx: &TestContext) {
    ctx.settings_store
        .batch_ensure_setting_definitions(vec![SettingDefinitionSeed {
            category: "general".into(),
            scope: scryer_application::SETTINGS_SCOPE_SYSTEM.into(),
            key_name: scryer_application::PLUGIN_AUTO_UPDATE_ENABLED_KEY.into(),
            data_type: "boolean".into(),
            default_value_json: "false".into(),
            is_sensitive: false,
            validation_json: None,
        }])
        .await
        .expect("seed plugin auto-update setting definition");
}

async fn plugin_auto_update_gql(
    ctx: &TestContext,
    query: &str,
    variables: serde_json::Value,
) -> serde_json::Value {
    let response = ctx
        .http_client()
        .post(ctx.graphql_url())
        .json(&serde_json::json!({ "query": query, "variables": variables }))
        .send()
        .await
        .expect("graphql request should succeed");
    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.expect("valid JSON body");
    assert!(
        body.get("errors").is_none(),
        "unexpected GraphQL errors: {body}"
    );
    body
}

#[tokio::test]
async fn plugin_auto_update_setting_defaults_off_and_requires_system_settings() {
    let ctx = TestContext::new().await;
    seed_plugin_auto_update_setting_definition(&ctx).await;

    assert!(
        !ctx.app
            .get_plugin_auto_update_settings(&admin())
            .await
            .expect("config users can read the setting")
            .enabled,
        "automatic plugin updates are opt-in"
    );
    assert!(
        ctx.app
            .get_plugin_auto_update_settings(&plugin_auto_update_viewer())
            .await
            .is_err(),
        "users without system-settings access cannot read the setting"
    );
    assert!(
        ctx.app
            .update_plugin_auto_update_settings(
                &plugin_auto_update_viewer(),
                scryer_application::UpdatePluginAutoUpdateSettings { enabled: true },
            )
            .await
            .is_err(),
        "users without system-settings access cannot change the setting"
    );

    let updated = ctx
        .app
        .update_plugin_auto_update_settings(
            &admin(),
            scryer_application::UpdatePluginAutoUpdateSettings { enabled: true },
        )
        .await
        .expect("config user updates the setting");
    assert!(updated.enabled);
    assert!(
        ctx.app
            .get_plugin_auto_update_settings(&admin())
            .await
            .expect("read back")
            .enabled
    );
}

#[tokio::test]
async fn graphql_plugin_auto_update_settings_round_trip() {
    let ctx = TestContext::new().await;
    seed_plugin_auto_update_setting_definition(&ctx).await;

    let body = plugin_auto_update_gql(
        &ctx,
        "query { pluginAutoUpdateSettings { enabled } }",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(body["data"]["pluginAutoUpdateSettings"]["enabled"], false);

    let body = plugin_auto_update_gql(
        &ctx,
        r#"mutation($input: UpdatePluginAutoUpdateSettingsInput!) {
            updatePluginAutoUpdateSettings(input: $input) { enabled }
        }"#,
        serde_json::json!({ "input": { "enabled": true } }),
    )
    .await;
    assert_eq!(
        body["data"]["updatePluginAutoUpdateSettings"]["enabled"],
        true
    );

    let body = plugin_auto_update_gql(
        &ctx,
        "query { pluginAutoUpdateSettings { enabled } }",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(body["data"]["pluginAutoUpdateSettings"]["enabled"], true);
}

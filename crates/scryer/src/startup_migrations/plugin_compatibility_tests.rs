use super::*;
use scryer_application::testing::AppUseCaseTestExt;
use scryer_domain::{PluginInstallation, PluginSourceKind, PluginSupportTier, PluginWasmEncoding};
use scryer_infrastructure_sql::runtime::{SqlArg, SqlRuntime};
use std::sync::Arc;

#[path = "../../tests/common/mod.rs"]
mod common;

const LEGACY_MODULE: &[u8] = b"\0asm\x01\0\0\0";

fn builtin_load() -> RuntimePluginLoad {
    let asset = scryer_plugins::builtins::INDEXER_BUILTINS[0];
    RuntimePluginLoad {
        descriptor: serde_json::from_str(asset.descriptor_json).unwrap(),
        wasm_bytes: Vec::new(),
        first_party: true,
    }
}

fn old_installation(id: &str, manual: bool) -> PluginInstallation {
    let mut runtime = builtin_load();
    let mut json = serde_json::to_value(&runtime.descriptor).unwrap();
    json["id"] = id.into();
    if manual {
        json["provider"]["provider_type"] = id.into();
    }
    runtime.descriptor = serde_json::from_value(json).unwrap();
    let descriptor = runtime.descriptor;
    let now = chrono::Utc::now();
    PluginInstallation {
        id: format!("installation-{id}"),
        plugin_id: id.into(),
        name: descriptor.name.clone(),
        description: "Legacy installation retained during upgrade".into(),
        version: descriptor.version.clone(),
        sdk_version: descriptor.sdk_version.clone(),
        sdk_constraint: descriptor.sdk_constraint.clone(),
        scryer_constraint: None,
        plugin_type: descriptor.plugin_type().into(),
        provider_type: descriptor.provider_type().into(),
        source_kind: if manual {
            PluginSourceKind::Manual
        } else {
            PluginSourceKind::Downloaded
        },
        is_enabled: true,
        is_builtin: false,
        wasm_encoding: PluginWasmEncoding::Identity,
        wasm_digest_algo: Some("blake3".into()),
        wasm_digest: Some(scryer_application::plugin_wasm_blake3_digest(LEGACY_MODULE)),
        artifact_digest: None,
        source_url: None,
        support_tier: if manual {
            PluginSupportTier::Unverified
        } else {
            PluginSupportTier::Official
        },
        publisher: None,
        docs_url: None,
        source_repo: None,
        manifest_url: None,
        descriptor_json: Some(serde_json::to_string(&descriptor).unwrap()),
        installed_at: now,
        updated_at: now,
    }
}

#[tokio::test]
async fn plugin_compatibility_startup_recovers_bundled_match_and_retains_blocked_manual_artifact() {
    let ctx = common::TestContext::new().await;
    let datastore = ctx.db.datastore();
    let store = DatastoreCustomizationStore::new(datastore.clone());
    let builtin = builtin_load().descriptor;
    let mut original = old_installation(&builtin.id, false);
    original.is_enabled = false;
    let manual = old_installation("legacy-manual", true);
    for installation in [&original, &manual] {
        store
            .create_plugin_installation(installation, Some(LEGACY_MODULE))
            .await
            .unwrap();
        SqlRuntime::execute_write(&datastore, "seed_provider_configuration",
            "INSERT INTO indexers (id, name, provider_type, base_url, api_key_encrypted, config_json, created_at, updated_at)
             VALUES ({}, {}, {}, 'https://indexer.test', 'retained-test-credential', {}, {}, {})",
            vec![SqlArg::Text(format!("indexer-{}", installation.plugin_id)), SqlArg::Text(installation.name.clone()),
                SqlArg::Text(installation.provider_type.clone()), SqlArg::Text("{}".into()), SqlArg::Timestamp(chrono::Utc::now()), SqlArg::Timestamp(chrono::Utc::now())])
            .await.unwrap();
    }
    let app = ctx.app.with_test_overrides(|builder| {
        builder
            .with_plugin_installation_store(Arc::new(store.clone()))
            .with_plugin_descriptor_loader(Arc::new(scryer_plugins::WasmPluginDescriptorLoader))
    });
    assert!(!migrate(&app, &datastore, &store).await.unwrap());
    let repaired = store
        .get_plugin_installation(&builtin.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(repaired.id, original.id);
    assert!(!repaired.is_enabled);
    assert_eq!(repaired.source_kind, original.source_kind);
    assert_ne!(
        store
            .get_plugin_installation_wasm_payload(&builtin.id)
            .await
            .unwrap()
            .unwrap()
            .bytes,
        LEGACY_MODULE
    );
    assert_eq!(
        serde_json::to_value(
            store
                .get_plugin_installation(&manual.plugin_id)
                .await
                .unwrap()
        )
        .unwrap(),
        serde_json::to_value(Some(manual.clone())).unwrap()
    );
    assert_eq!(
        store
            .get_plugin_installation_wasm_payload(&manual.plugin_id)
            .await
            .unwrap()
            .unwrap()
            .bytes,
        LEGACY_MODULE
    );
    assert!(!migrate(&app, &datastore, &store).await.unwrap());
    let blockers = SqlRuntime::fetch_all(datastore.read_exec(),
        "SELECT status FROM application_compatibility_journal WHERE migration_id = {} AND subject_id = {}",
        &[SqlArg::Text(ID.into()), SqlArg::Text(manual.id.clone())]).await.unwrap();
    assert_eq!(blockers.len(), 1);
    assert_eq!(blockers[0].text("status").unwrap(), "blocked");
    let mut timestamp_only = manual.clone();
    timestamp_only.updated_at += chrono::Duration::seconds(1);
    store
        .update_plugin_installation(&timestamp_only, None)
        .await
        .unwrap();
    assert!(!migrate(&app, &datastore, &store).await.unwrap());
    let repeated = SqlRuntime::fetch_all(datastore.read_exec(),
        "SELECT original_metadata FROM application_compatibility_journal WHERE migration_id = {} AND subject_id = {}",
        &[SqlArg::Text(ID.into()), SqlArg::Text(manual.id.clone())]).await.unwrap();
    assert_eq!(
        repeated.len(),
        1,
        "timestamp-only reseeding must reuse the journal entry"
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&repeated[0].text("original_metadata").unwrap())
            .unwrap(),
        serde_json::to_value(&manual).unwrap()
    );
    let configs = SqlRuntime::fetch_all(
        datastore.read_exec(),
        "SELECT provider_type, api_key_encrypted, config_json, is_enabled FROM indexers",
        &[],
    )
    .await
    .unwrap();
    assert_eq!(configs.len(), 2);
    for config in configs {
        assert_eq!(
            config.text("api_key_encrypted").unwrap(),
            "retained-test-credential"
        );
        assert_eq!(config.text("config_json").unwrap(), "{}");
        assert!(config.bool("is_enabled").unwrap());
        assert!(
            [
                original.provider_type.as_str(),
                manual.provider_type.as_str()
            ]
            .contains(&config.text("provider_type").unwrap().as_str())
        );
    }
    let actor = app.find_or_create_default_user().await.unwrap();
    let visible = app.list_available_plugins(&actor).await.unwrap();
    let blocked = visible
        .iter()
        .find(|plugin| plugin.id == manual.plugin_id)
        .expect("manual blocker must be operator visible");
    assert!(blocked.is_enabled);
    assert!(blocked.blocked_reason.is_some());
}

#[tokio::test]
async fn plugin_compatibility_store_failure_keeps_original_bytes_and_retry_converges() {
    let ctx = common::TestContext::new().await;
    let datastore = ctx.db.datastore();
    let store = DatastoreCustomizationStore::new(datastore.clone());
    let builtin = builtin_load().descriptor;
    let original = old_installation(&builtin.id, false);
    store
        .create_plugin_installation(&original, Some(LEGACY_MODULE))
        .await
        .unwrap();
    let app = ctx.app.with_test_overrides(|builder| {
        builder
            .with_plugin_installation_store(Arc::new(store.clone()))
            .with_plugin_descriptor_loader(Arc::new(scryer_plugins::WasmPluginDescriptorLoader))
    });
    SqlRuntime::execute_write(
        &datastore,
        "inject_plugin_write_failure",
        "CREATE TRIGGER reject_plugin_update BEFORE UPDATE ON plugin_installations
         BEGIN SELECT RAISE(ABORT, 'synthetic plugin write failure'); END",
        vec![],
    )
    .await
    .unwrap();
    assert!(!migrate(&app, &datastore, &store).await.unwrap());
    assert_eq!(
        serde_json::to_value(
            store
                .get_plugin_installation(&original.plugin_id)
                .await
                .unwrap()
        )
        .unwrap(),
        serde_json::to_value(Some(original.clone())).unwrap()
    );
    assert_eq!(
        store
            .get_plugin_installation_wasm_payload(&original.plugin_id)
            .await
            .unwrap()
            .unwrap()
            .bytes,
        LEGACY_MODULE
    );
    SqlRuntime::execute_write(
        &datastore,
        "remove_plugin_write_failure",
        "DROP TRIGGER reject_plugin_update",
        vec![],
    )
    .await
    .unwrap();
    assert!(migrate(&app, &datastore, &store).await.unwrap());
    assert!(migrate(&app, &datastore, &store).await.unwrap());
}

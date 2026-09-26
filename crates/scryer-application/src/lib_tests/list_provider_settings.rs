//! Server-wide list provider values: who may change them, how they are stored,
//! and that a secret never comes back out.

use std::collections::BTreeMap;

use super::*;
use crate::lists::test_support::{PROVIDER, ScriptedLists, ScriptedProvider};
use scryer_plugin_sdk::{ConfigFieldDef, ConfigFieldType};

fn field(key: &str, field_type: ConfigFieldType) -> ConfigFieldDef {
    serde_json::from_value(serde_json::json!({
        "key": key,
        "label": key,
        "field_type": field_type,
    }))
    .expect("config field")
}

fn harness_with_provider() -> MediaRequestTestHarness {
    let lists = ScriptedLists::with_config_fields(vec![
        field("instance_key", ConfigFieldType::Password),
        field("region", ConfigFieldType::String),
    ]);
    bootstrap_media_request_app_with_list_plugins(Arc::new(ScriptedProvider(lists)))
}

fn list_manager() -> User {
    let mut manager = User::new_admin("list-manager");
    manager.authorization = scryer_domain::UserAuthorization {
        app: AppPermissionMask::MANAGE_LISTS,
        loaded: true,
        ..Default::default()
    };
    manager
}

#[tokio::test]
async fn stored_values_reach_the_provider_and_a_secret_stays_hidden() {
    let harness = harness_with_provider();
    let manager = list_manager();

    let view = harness
        .app
        .update_list_provider_settings(
            &manager,
            &PROVIDER.to_ascii_uppercase(),
            BTreeMap::from([
                (
                    "instance_key".to_string(),
                    Some("synthetic-instance-key".to_string()),
                ),
                ("region".to_string(), Some("north".to_string())),
            ]),
        )
        .await
        .expect("update");
    assert_eq!(view.provider_type, PROVIDER);
    assert!(view.fields[0].is_set);
    assert_eq!(view.fields[0].value, None);

    let listed = harness
        .app
        .list_provider_settings(&manager)
        .await
        .expect("list");
    assert_eq!(listed, vec![view]);

    let configs = harness.app.load_list_provider_configs().await;
    let passed = configs.for_provider(harness.app.services.lists.plugins.as_ref(), PROVIDER);
    assert_eq!(
        passed.get("instance_key").map(String::as_str),
        Some("synthetic-instance-key")
    );
    assert_eq!(
        harness.app.list_provider_config_for(PROVIDER).await,
        passed,
        "a preview reads the same values a sync does"
    );

    // Leaving the secret out of an edit keeps it.
    harness
        .app
        .update_list_provider_settings(
            &manager,
            PROVIDER,
            BTreeMap::from([("region".to_string(), None)]),
        )
        .await
        .expect("second update");
    let passed = harness.app.list_provider_config_for(PROVIDER).await;
    assert_eq!(
        passed,
        BTreeMap::from([(
            "instance_key".to_string(),
            "synthetic-instance-key".to_string()
        )])
    );
}

#[tokio::test]
async fn only_a_list_manager_reads_or_changes_provider_values() {
    let harness = harness_with_provider();

    let read = harness.app.list_provider_settings(&harness.user).await;
    assert!(matches!(read, Err(AppError::Unauthorized(_))), "{read:?}");
    let write = harness
        .app
        .update_list_provider_settings(
            &harness.user,
            PROVIDER,
            BTreeMap::from([("region".to_string(), Some("north".to_string()))]),
        )
        .await;
    assert!(matches!(write, Err(AppError::Unauthorized(_))), "{write:?}");
    assert!(
        harness
            .app
            .list_provider_config_for(PROVIDER)
            .await
            .is_empty(),
        "a refused change stores nothing"
    );
}

#[tokio::test]
async fn an_unknown_provider_is_not_found() {
    let harness = harness_with_provider();
    let result = harness
        .app
        .update_list_provider_settings(&list_manager(), "unknown-provider", BTreeMap::new())
        .await;
    assert!(matches!(result, Err(AppError::NotFound(_))), "{result:?}");
}

#[tokio::test]
async fn the_catalog_shows_whether_a_value_is_stored_only_to_a_list_manager() {
    let harness = harness_with_provider();
    let manager = list_manager();
    harness
        .app
        .update_list_provider_settings(
            &manager,
            PROVIDER,
            BTreeMap::from([(
                "instance_key".to_string(),
                Some("synthetic-instance-key".to_string()),
            )]),
        )
        .await
        .expect("update");

    let fields_for = |manifests: Vec<crate::lists::catalog::ListProviderManifest>| {
        manifests
            .into_iter()
            .find(|manifest| manifest.provider_type == PROVIDER)
            .expect("provider in catalog")
            .config_fields
    };

    let managed = fields_for(
        harness
            .app
            .list_provider_catalog(&manager)
            .await
            .expect("catalog"),
    );
    let keys: Vec<&str> = managed.iter().map(|field| field.key.as_str()).collect();
    assert_eq!(keys, ["instance_key", "region"]);
    assert!(managed[0].secret && managed[0].is_set);
    assert!(!managed[1].is_set);
    assert!(managed.iter().all(|field| field.value.is_none()));

    let viewed = fields_for(
        harness
            .app
            .list_provider_catalog(&harness.user)
            .await
            .expect("catalog"),
    );
    assert_eq!(viewed.len(), 2);
    assert!(
        viewed
            .iter()
            .all(|field| !field.is_set && field.value.is_none())
    );
}

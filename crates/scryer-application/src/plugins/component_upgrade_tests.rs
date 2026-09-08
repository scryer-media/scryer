use super::*;

#[test]
fn component_upgrade_catalog_replacements_must_preserve_publisher_source_and_identity() {
    let catalog_entry: CatalogV3PluginEntry = serde_json::from_value(catalog_entry_with_releases(
        "alpha",
        vec![catalog_v3_release(
            "0.2.0",
            "https://example.test/alpha.wasm.zst",
            None,
        )],
    ))
    .unwrap();
    let release = catalog_entry.releases[0].clone();
    let artifact = release.artifacts[0].clone();
    let resolved = CatalogPluginResolution {
        catalog_entry,
        release,
        artifact,
        source_kind: PluginSourceKind::Downloaded,
        effective_support_tier: PluginSupportTier::Official,
        github_repo: GitHubRepo {
            owner: "scryer-media".into(),
            name: "test-plugin-alpha".into(),
        },
    };
    let mut installation = official_catalog_installation("alpha", "0.1.0");
    installation.publisher = Some(resolved.catalog_entry.publisher.clone());
    installation.source_repo = Some(resolved.catalog_entry.source_repo.clone());
    assert!(component_replacement_has_same_origin(
        &installation,
        &resolved
    ));
    for changed in 0..6 {
        let mut candidate = resolved.clone();
        match changed {
            0 => candidate.catalog_entry.publisher = "different publisher".into(),
            1 => {
                candidate.catalog_entry.source_repo = "https://example.test/different-source".into()
            }
            2 => candidate.catalog_entry.provider_type = "different-provider".into(),
            3 => candidate.catalog_entry.plugin_type = "notification".into(),
            4 => candidate.source_kind = PluginSourceKind::Community,
            _ => candidate.effective_support_tier = PluginSupportTier::Unverified,
        }
        assert!(
            !component_replacement_has_same_origin(&installation, &candidate),
            "changed identity field {changed} must be refused"
        );
    }
}

#[tokio::test]
async fn component_upgrade_preserves_identity_origin_and_disabled_preference_and_is_repeatable() {
    for enabled in [false, true] {
        let h = bootstrap_plugins(Some(MockPluginProvider::new()));
        let mut original = official_catalog_installation("alpha", "0.1.0");
        original.is_enabled = enabled;
        original.publisher = Some("original publisher".into());
        original.source_repo = Some("original repository".into());
        original.source_url = Some("https://example.test/original.wasm".into());
        seed_auto_update_installation(&h, original.clone()).await;
        let mut replacement = make_runtime_plugin_load("alpha", "indexer", "alpha");
        replacement.wasm_bytes = b"compatible replacement".to_vec();
        h.plugin_descriptor_loader
            .register(&replacement.wasm_bytes, replacement.descriptor.clone());

        assert!(
            h.app
                .repair_plugin_component_installation(original.clone(), Some(replacement.clone()))
                .await
                .unwrap()
        );
        let repaired = h
            .plugin_repo
            .get_plugin_installation("alpha")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(repaired.id, original.id);
        assert_eq!(repaired.is_enabled, enabled);
        assert_eq!(repaired.source_kind, original.source_kind);
        assert_eq!(repaired.support_tier, original.support_tier);
        assert_eq!(repaired.publisher, original.publisher);
        assert_eq!(repaired.source_repo, original.source_repo);
        assert_eq!(repaired.source_url, original.source_url);
        assert_eq!(repaired.installed_at, original.installed_at);
        assert_eq!(
            h.plugin_repo
                .get_plugin_installation_wasm_payload("alpha")
                .await
                .unwrap()
                .unwrap()
                .bytes,
            replacement.wasm_bytes
        );
        assert!(
            !h.app
                .repair_plugin_component_installation(repaired.clone(), Some(replacement))
                .await
                .unwrap()
        );
        assert_eq!(
            serde_json::to_value(
                h.plugin_repo
                    .get_plugin_installation("alpha")
                    .await
                    .unwrap()
            )
            .unwrap(),
            serde_json::to_value(Some(repaired)).unwrap()
        );
    }
}

#[tokio::test]
async fn component_upgrade_rejects_invalid_artifact_without_mutating_original() {
    let h = bootstrap_plugins(None);
    let original = official_catalog_installation("alpha", "0.1.0");
    seed_auto_update_installation(&h, original.clone()).await;
    let replacement = make_runtime_plugin_load("alpha", "indexer", "alpha");
    // No descriptor registration: the runtime loader rejects these bytes.
    assert!(
        h.app
            .repair_plugin_component_installation(original.clone(), Some(replacement))
            .await
            .is_err()
    );
    assert_eq!(
        serde_json::to_value(
            h.plugin_repo
                .get_plugin_installation("alpha")
                .await
                .unwrap()
        )
        .unwrap(),
        serde_json::to_value(Some(original)).unwrap()
    );
    assert_eq!(
        h.plugin_repo
            .get_plugin_installation_wasm_payload("alpha")
            .await
            .unwrap()
            .unwrap()
            .bytes,
        AUTO_UPDATE_WASM_BYTES
    );
}

#[tokio::test]
async fn component_upgrade_does_not_replace_manual_installation_with_same_named_builtin() {
    let h = bootstrap_plugins(None);
    let mut original = make_installation("alpha", "0.1.0", false, true);
    original.source_kind = PluginSourceKind::Manual;
    h.plugin_repo
        .create_plugin_installation(&original, Some(AUTO_UPDATE_WASM_BYTES))
        .await
        .unwrap();
    let replacement = make_runtime_plugin_load("alpha", "indexer", "alpha");
    h.plugin_descriptor_loader
        .register(&replacement.wasm_bytes, replacement.descriptor.clone());
    assert!(
        h.app
            .repair_plugin_component_installation(original.clone(), Some(replacement))
            .await
            .is_err()
    );
    assert_eq!(
        serde_json::to_value(
            h.plugin_repo
                .get_plugin_installation("alpha")
                .await
                .unwrap()
        )
        .unwrap(),
        serde_json::to_value(Some(original)).unwrap()
    );
}

#[tokio::test]
async fn component_upgrade_rejects_descriptor_mismatch_and_wrong_provider_family() {
    for wrong_family in [false, true] {
        let h = bootstrap_plugins(None);
        let original = official_catalog_installation("alpha", "0.1.0");
        seed_auto_update_installation(&h, original.clone()).await;
        let mut replacement = make_runtime_plugin_load(
            "alpha",
            if wrong_family {
                "notification"
            } else {
                "indexer"
            },
            "alpha",
        );
        replacement.wasm_bytes = b"candidate".to_vec();
        let mut embedded = replacement.descriptor.clone();
        embedded.name = "different embedded metadata".into();
        h.plugin_descriptor_loader
            .register(&replacement.wasm_bytes, embedded);
        assert!(
            h.app
                .repair_plugin_component_installation(original.clone(), Some(replacement))
                .await
                .is_err()
        );
        assert_eq!(
            serde_json::to_value(
                h.plugin_repo
                    .get_plugin_installation("alpha")
                    .await
                    .unwrap()
            )
            .unwrap(),
            serde_json::to_value(Some(original)).unwrap()
        );
    }
}

#[tokio::test]
async fn component_upgrade_blocker_survives_unrelated_mutation_and_clears_after_valid_repair() {
    let h = bootstrap_plugins(None);
    let original = official_catalog_installation("alpha", "0.1.0");
    seed_auto_update_installation(&h, original.clone()).await;
    h.app
        .set_plugin_component_blocker(&original, Some("legacy artifact".into()))
        .await
        .unwrap();
    h.app
        .finalize_runtime_plugin_mutation("indexer", false)
        .await
        .unwrap();
    assert!(
        h.app
            .runtime
            .plugins
            .compatibility_blockers
            .read()
            .await
            .contains_key("alpha")
    );
    let mut replacement = make_runtime_plugin_load("alpha", "indexer", "alpha");
    replacement.wasm_bytes = b"replacement".to_vec();
    h.plugin_descriptor_loader
        .register(&replacement.wasm_bytes, replacement.descriptor.clone());
    h.app
        .repair_plugin_component_installation(original, Some(replacement))
        .await
        .unwrap();
    h.app
        .finalize_runtime_plugin_mutation("indexer", false)
        .await
        .unwrap();
    assert!(
        !h.app
            .runtime
            .plugins
            .compatibility_blockers
            .read()
            .await
            .contains_key("alpha")
    );
}

#[tokio::test]
async fn component_upgrade_unrelated_mutation_preserves_incompatible_host_and_sdk_blockers() {
    for host_constraint in [true, false] {
        let h = bootstrap_plugins(None);
        let original = official_catalog_installation("alpha", "0.1.0");
        seed_auto_update_installation(&h, original.clone()).await;
        let replacement = make_runtime_plugin_load("alpha", "indexer", "alpha");
        h.plugin_descriptor_loader
            .register(&replacement.wasm_bytes, replacement.descriptor.clone());
        h.app
            .repair_plugin_component_installation(original, Some(replacement))
            .await
            .unwrap();
        let mut installation = h
            .plugin_repo
            .get_plugin_installation("alpha")
            .await
            .unwrap()
            .unwrap();
        if host_constraint {
            installation.scryer_constraint = Some(">=999.0.0".into());
        } else {
            installation.sdk_constraint = ">=999.0.0".into();
        }
        h.plugin_repo
            .update_plugin_installation(&installation, None)
            .await
            .unwrap();
        h.app
            .set_plugin_component_blocker(&installation, Some("incompatible contract".into()))
            .await
            .unwrap();
        h.app
            .finalize_runtime_plugin_mutation("notification", false)
            .await
            .unwrap();
        assert!(
            h.app
                .runtime
                .plugins
                .compatibility_blockers
                .read()
                .await
                .contains_key("alpha")
        );
        if host_constraint {
            installation.scryer_constraint = None;
        } else {
            installation.sdk_constraint = plugin_descriptor_sdk_constraint(
                &serde_json::from_str(installation.descriptor_json.as_deref().unwrap()).unwrap(),
            );
        }
        h.plugin_repo
            .update_plugin_installation(&installation, None)
            .await
            .unwrap();
        h.app
            .finalize_runtime_plugin_mutation("notification", false)
            .await
            .unwrap();
        assert!(
            !h.app
                .runtime
                .plugins
                .compatibility_blockers
                .read()
                .await
                .contains_key("alpha")
        );
    }
}

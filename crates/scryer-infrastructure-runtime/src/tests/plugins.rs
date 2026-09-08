use super::*;

#[tokio::test]
async fn sqlite_can_initialize() {
    let db = std::env::temp_dir().join(format!(
        "scryer_store_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy()).await.unwrap();
    let users = UserRepository::list_all(&user_store(&services))
        .await
        .expect("query should return users after initialization");

    assert!(!users.is_empty());
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn seed_builtin_refreshes_existing_builtin_metadata_without_resetting_enabled_state() {
    let (services, db) = temp_services("scryer_plugin_builtin_refresh").await;
    let customization = PluginStore::new(services.datastore());

    customization
        .seed_builtin(
            "newznab",
            "Old Newznab",
            "old description",
            "0.1.0",
            "1.3.0",
            ">=1.3.0, <1.4.0",
            "indexer",
            "newznab",
        )
        .await
        .expect("initial builtin seed should succeed");

    let mut installation = customization
        .get_plugin_installation("newznab")
        .await
        .expect("load seeded builtin")
        .expect("builtin installation should exist");
    installation.is_enabled = false;
    customization
        .update_plugin_installation(&installation, None)
        .await
        .expect("disable builtin installation");

    customization
        .seed_builtin(
            "newznab",
            "Newznab Indexer",
            "new description",
            "0.2.2",
            "1.3.0",
            ">=1.3.0, <1.4.0",
            "usenet_indexer",
            "newznab",
        )
        .await
        .expect("refresh builtin seed should succeed");

    let refreshed = customization
        .get_plugin_installation("newznab")
        .await
        .expect("load refreshed builtin")
        .expect("refreshed builtin installation should exist");
    assert_eq!(refreshed.name, "Newznab Indexer");
    assert_eq!(refreshed.description, "new description");
    assert_eq!(refreshed.version, "0.2.2");
    assert_eq!(refreshed.plugin_type, "usenet_indexer");
    assert_eq!(refreshed.provider_type, "newznab");
    assert!(!refreshed.is_enabled);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn reverting_downloaded_builtin_clears_downloaded_artifact_state() {
    let (services, db) = temp_services("scryer_plugin_builtin_revert").await;
    let customization = PluginStore::new(services.datastore());
    let now = Utc::now();
    let installation = scryer_domain::PluginInstallation {
        id: scryer_domain::Id::new().0,
        plugin_id: "newznab".to_string(),
        name: "Newznab".to_string(),
        description: "downloaded override".to_string(),
        version: "0.2.2".to_string(),
        sdk_version: "1.3.0".to_string(),
        sdk_constraint: ">=1.3.0, <1.4.0".to_string(),
        scryer_constraint: None,
        plugin_type: "usenet_indexer".to_string(),
        provider_type: "newznab".to_string(),
        is_enabled: true,
        is_builtin: true,
        source_kind: scryer_domain::PluginSourceKind::Downloaded,
        wasm_encoding: scryer_domain::PluginWasmEncoding::Zstd,
        wasm_digest_algo: Some("blake3".to_string()),
        source_url: Some("https://example.com/newznab-0.2.2.wasm".to_string()),
        support_tier: scryer_domain::PluginSupportTier::Official,
        publisher: None,
        docs_url: None,
        source_repo: None,
        manifest_url: None,
        wasm_digest: Some("abc123".to_string()),
        artifact_digest: None,
        descriptor_json: Some(test_descriptor_json(
            "newznab",
            "0.2.2",
            "usenet_indexer",
            "newznab",
        )),
        installed_at: now,
        updated_at: now,
    };

    customization
        .create_plugin_installation(&installation, Some(&[1_u8, 2, 3]))
        .await
        .expect("seed downloaded builtin override");

    let mut reverted = installation.clone();
    reverted.source_kind = scryer_domain::PluginSourceKind::Bundled;
    reverted.wasm_encoding = scryer_domain::PluginWasmEncoding::Identity;
    reverted.wasm_digest_algo = None;
    reverted.source_url = None;
    reverted.wasm_digest = None;

    let reverted = customization
        .update_plugin_installation(&reverted, None)
        .await
        .expect("revert builtin override");

    assert_eq!(
        reverted.source_kind,
        scryer_domain::PluginSourceKind::Bundled
    );
    assert_eq!(
        reverted.wasm_encoding,
        scryer_domain::PluginWasmEncoding::Identity
    );
    assert!(reverted.wasm_digest_algo.is_none());
    assert!(reverted.wasm_digest.is_none());
    assert!(reverted.source_url.is_none());

    let enabled = customization
        .get_enabled_plugin_wasm_bytes()
        .await
        .expect("list enabled plugin wasm bytes");
    let (_, wasm_bytes) = enabled
        .into_iter()
        .find(|(item, _)| item.plugin_id == "newznab")
        .expect("reverted builtin should remain installed");
    assert!(wasm_bytes.is_none());

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn cleanup_deletes_legacy_external_plugin_rows_and_preserves_builtins() {
    let (services, db) = temp_services("scryer_plugin_cleanup_legacy").await;
    let customization = PluginStore::new(services.datastore());
    let now = Utc::now();

    let legacy_external = scryer_domain::PluginInstallation {
        id: scryer_domain::Id::new().0,
        plugin_id: "legacy-external".to_string(),
        name: "Legacy External".to_string(),
        description: "old registry install".to_string(),
        version: "0.1.0".to_string(),
        sdk_version: "1.3.0".to_string(),
        sdk_constraint: ">=1.3.0, <1.4.0".to_string(),
        scryer_constraint: None,
        plugin_type: "notification".to_string(),
        provider_type: "legacy_external".to_string(),
        is_enabled: true,
        is_builtin: false,
        source_kind: scryer_domain::PluginSourceKind::Downloaded,
        wasm_encoding: scryer_domain::PluginWasmEncoding::Identity,
        wasm_digest_algo: None,
        source_url: Some("https://example.com/legacy.wasm".to_string()),
        support_tier: scryer_domain::PluginSupportTier::Official,
        publisher: None,
        docs_url: None,
        source_repo: None,
        manifest_url: None,
        wasm_digest: None,
        artifact_digest: None,
        descriptor_json: None,
        installed_at: now,
        updated_at: now,
    };
    customization
        .create_plugin_installation(&legacy_external, Some(&[1_u8, 2, 3]))
        .await
        .expect("seed legacy external install");

    customization
        .seed_builtin(
            "newznab",
            "Newznab",
            "builtin seed",
            "0.2.0",
            "1.3.0",
            ">=1.3.0, <1.4.0",
            "usenet_indexer",
            "newznab",
        )
        .await
        .expect("seed builtin install");

    let removed = customization
        .delete_incompatible_external_plugin_installations(false)
        .await
        .expect("cleanup incompatible external installs");

    assert_eq!(removed, vec!["legacy-external".to_string()]);
    assert!(
        customization
            .get_plugin_installation("legacy-external")
            .await
            .expect("read legacy install")
            .is_none()
    );
    assert!(
        customization
            .get_plugin_installation("newznab")
            .await
            .expect("read builtin install")
            .is_some()
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn legacy_external_rows_remain_visible_for_repair_and_do_not_block_reinstall() {
    let (services, db) = temp_services("scryer_plugin_reinstall_legacy").await;
    let customization = PluginStore::new(services.datastore());
    let now = Utc::now();

    let legacy_external = scryer_domain::PluginInstallation {
        id: scryer_domain::Id::new().0,
        plugin_id: "email".to_string(),
        name: "Legacy Email".to_string(),
        description: "old registry install".to_string(),
        version: "0.1.0".to_string(),
        sdk_version: "1.3.0".to_string(),
        sdk_constraint: ">=1.3.0, <1.4.0".to_string(),
        scryer_constraint: None,
        plugin_type: "notification".to_string(),
        provider_type: "email".to_string(),
        is_enabled: true,
        is_builtin: false,
        source_kind: scryer_domain::PluginSourceKind::Downloaded,
        wasm_encoding: scryer_domain::PluginWasmEncoding::Identity,
        wasm_digest_algo: None,
        source_url: Some("https://example.com/legacy-email.wasm".to_string()),
        support_tier: scryer_domain::PluginSupportTier::Official,
        publisher: None,
        docs_url: None,
        source_repo: None,
        manifest_url: None,
        wasm_digest: None,
        artifact_digest: None,
        descriptor_json: None,
        installed_at: now,
        updated_at: now,
    };
    customization
        .create_plugin_installation(&legacy_external, Some(&[1_u8, 2, 3]))
        .await
        .expect("seed legacy external install");

    let visible = customization
        .get_plugin_installation("email")
        .await
        .expect("read legacy repair target")
        .expect("legacy installation must remain visible");
    assert_eq!(visible.id, legacy_external.id);
    assert_eq!(visible.source_url, legacy_external.source_url);
    assert!(
        customization
            .list_plugin_installations()
            .await
            .expect("list plugin installations")
            .into_iter()
            .any(|installation| installation.plugin_id == "email")
    );

    let compressed = zstd::encode_all(&b"catalog plugin bytes"[..], 1).expect("compress plugin");
    let replacement = scryer_domain::PluginInstallation {
        id: scryer_domain::Id::new().0,
        plugin_id: "email".to_string(),
        name: "Email".to_string(),
        description: "catalog install".to_string(),
        version: "1.0.0".to_string(),
        sdk_version: "1.3.0".to_string(),
        sdk_constraint: ">=1.3.0, <1.4.0".to_string(),
        scryer_constraint: None,
        plugin_type: "notification".to_string(),
        provider_type: "email".to_string(),
        is_enabled: true,
        is_builtin: false,
        source_kind: scryer_domain::PluginSourceKind::Downloaded,
        wasm_encoding: scryer_domain::PluginWasmEncoding::Zstd,
        wasm_digest_algo: Some("blake3".to_string()),
        source_url: Some("https://example.com/catalog-email.wasm.zst".to_string()),
        support_tier: scryer_domain::PluginSupportTier::Official,
        publisher: Some("Scryer".to_string()),
        docs_url: Some("https://example.com/docs".to_string()),
        source_repo: Some("https://github.com/example/email".to_string()),
        manifest_url: Some("https://example.com/email.manifest.json".to_string()),
        wasm_digest: Some("abcdef0123456789".to_string()),
        artifact_digest: Some("digest:artifact".to_string()),
        descriptor_json: Some(test_descriptor_json(
            "email",
            "1.0.0",
            "notification",
            "email",
        )),
        installed_at: now,
        updated_at: now,
    };

    let created = customization
        .create_plugin_installation(&replacement, Some(&compressed))
        .await
        .expect("create replacement install");
    assert_eq!(created.version, "1.0.0");
    assert_eq!(
        created.wasm_encoding,
        scryer_domain::PluginWasmEncoding::Zstd
    );
    assert_eq!(created.wasm_digest_algo.as_deref(), Some("blake3"));

    let row_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM plugin_installations WHERE plugin_id = 'email'",
    )
    .fetch_one(&services.pool)
    .await
    .expect("count email rows");
    assert_eq!(row_count, 1);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn plugin_catalog_source_rows_survive_cleanup() {
    let (services, db) = temp_services("scryer_plugin_catalog_source_shape").await;
    let customization = PluginStore::new(services.datastore());
    let source = scryer_domain::PluginCatalogSource {
        source_key: "__central_catalog_v2".to_string(),
        source_kind: "central".to_string(),
        source_url: "https://example.com/catalog-v2.min.json.zst".to_string(),
        github_repo: Some("scryer-media/scryer-plugins".to_string()),
        support_tier: scryer_domain::PluginSupportTier::Official,
        catalog_json: Some(
            r#"{"schema_version":"scryer.plugin.catalog.v2","plugins":[],"rule_packs":[]}"#
                .to_string(),
        ),
        last_success_at: Some(Utc::now()),
        last_error: None,
        updated_at: Utc::now(),
    };

    customization
        .upsert_plugin_catalog_source(&source)
        .await
        .expect("store plugin catalog source");

    assert_eq!(
        customization
            .get_plugin_catalog_source("__central_catalog_v2")
            .await
            .expect("read plugin catalog source"),
        Some(source.clone())
    );

    let removed = customization
        .delete_incompatible_external_plugin_installations(false)
        .await
        .expect("cleanup incompatible external installs");
    assert!(removed.is_empty());
    assert_eq!(
        customization
            .get_plugin_catalog_source("__central_catalog_v2")
            .await
            .expect("read plugin catalog source after cleanup"),
        Some(source.clone())
    );

    let row = sqlx::query(
        "SELECT source_kind, support_tier FROM plugin_catalog_sources WHERE source_key = '__central_catalog_v2'",
    )
    .fetch_one(&services.pool)
    .await
    .expect("load plugin catalog source row");
    assert_eq!(row.get::<String, _>("source_kind"), "central");
    assert_eq!(row.get::<String, _>("support_tier"), "official");

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn enabled_plugin_payloads_preserve_zstd_encoding() {
    let (services, db) = temp_services("scryer_plugin_payload_encoding").await;
    let customization = PluginStore::new(services.datastore());
    let now = Utc::now();
    let wasm_bytes = b"hello compressed plugin";
    let compressed = zstd::encode_all(&wasm_bytes[..], 1).expect("compress plugin bytes");

    let installation = scryer_domain::PluginInstallation {
        id: scryer_domain::Id::new().0,
        plugin_id: "email".to_string(),
        name: "Email".to_string(),
        description: "catalog install".to_string(),
        version: "0.1.2".to_string(),
        sdk_version: "1.6.0".to_string(),
        sdk_constraint: ">=1.6.0, <2.0.0".to_string(),
        scryer_constraint: None,
        plugin_type: "notification".to_string(),
        provider_type: "email".to_string(),
        is_enabled: true,
        is_builtin: false,
        source_kind: scryer_domain::PluginSourceKind::Downloaded,
        wasm_encoding: scryer_domain::PluginWasmEncoding::Zstd,
        wasm_digest_algo: Some("blake3".to_string()),
        source_url: Some("https://example.com/email/plugin.wasm.zst".to_string()),
        support_tier: scryer_domain::PluginSupportTier::Official,
        publisher: Some("scryer-media".to_string()),
        docs_url: None,
        source_repo: Some("https://github.com/scryer-media/scryer-plugins".to_string()),
        manifest_url: Some("https://example.com/email/plugin.manifest.json".to_string()),
        wasm_digest: Some(
            scryer_application::plugin_wasm_blake3_digest(wasm_bytes)
                .split_once(':')
                .expect("digest should include an algorithm prefix")
                .1
                .to_string(),
        ),
        artifact_digest: Some("blake3:abcd".to_string()),
        descriptor_json: Some(test_descriptor_json(
            "email",
            "0.1.2",
            "notification",
            "email",
        )),
        installed_at: now,
        updated_at: now,
    };

    customization
        .create_plugin_installation(&installation, Some(compressed.as_slice()))
        .await
        .expect("seed encoded plugin install");

    let enabled = customization
        .get_enabled_plugin_wasm_bytes()
        .await
        .expect("list enabled plugin payloads");
    let (_, payload) = enabled
        .into_iter()
        .find(|(item, _)| item.plugin_id == "email")
        .expect("email installation should be present");
    let payload = payload.expect("payload should be present");

    assert_eq!(payload.encoding, scryer_domain::PluginWasmEncoding::Zstd);
    assert_eq!(payload.bytes, compressed);

    let _ = std::fs::remove_file(db);
}

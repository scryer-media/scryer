use super::*;

fn download_client(
    id: &str,
    client_type: &str,
    is_enabled: bool,
) -> scryer_domain::DownloadClientConfig {
    let now = Utc::now();
    scryer_domain::DownloadClientConfig {
        id: id.to_string(),
        name: format!("Client {id}"),
        client_type: client_type.to_string(),
        config_json: "{}".to_string(),
        is_enabled,
        status: scryer_domain::DownloadClientStatus::Healthy,
        last_error: None,
        last_seen_at: None,
        client_priority: 0,
        created_at: now,
        updated_at: now,
        proxy_config_id: None,
    }
}

struct FixedIndexerManagementClient {
    plan: IndexerSyncPlan,
}

#[async_trait]
impl IndexerManagementClient for FixedIndexerManagementClient {
    async fn validate_connection(&self) -> AppResult<IndexerValidationResult> {
        Ok(IndexerValidationResult::default())
    }

    async fn plan_sync(&self, _parent_config_id: &str) -> AppResult<IndexerSyncPlan> {
        Ok(self.plan.clone())
    }

    fn name(&self) -> &str {
        "fixed-management-client"
    }
}

#[tokio::test]
async fn indexer_mapping_set_clear_idempotence_and_compatibility_contract() {
    let mut managed_child = synthetic_direct_nab_indexer_config("idx-child", "nzbgeek");
    managed_child.managed_parent_config_id = Some("prowlarr-parent".to_string());
    let prowlarr_parent = synthetic_direct_nab_indexer_config("prowlarr-parent", "prowlarr");
    let unknown = synthetic_direct_nab_indexer_config("idx-unknown", "generic");
    let (app, admin) = bootstrap_with_search_settings_indexer_and_configs(
        Arc::new(StoredSettingsRepo::default()),
        Arc::new(MockIndexerClient),
        vec![
            synthetic_direct_nab_indexer_config("idx-usenet", "nzbgeek"),
            managed_child,
            prowlarr_parent,
            unknown,
        ],
    );
    for client in [
        download_client("sab-disabled", "sabnzbd", false),
        download_client("torrent", "qbittorrent", true),
    ] {
        app.services
            .integrations
            .download_client_configs
            .create(client)
            .await
            .expect("client should insert");
    }

    let mapped = app
        .set_indexer_download_client_mapping(&admin, "idx-usenet", Some("  sab-disabled  "))
        .await
        .expect("compatible disabled clients remain assignable");
    assert_eq!(mapped.download_client_id.as_deref(), Some("sab-disabled"));

    let idempotent = app
        .set_indexer_download_client_mapping(&admin, "idx-usenet", Some("sab-disabled"))
        .await
        .expect("idempotent mapping should succeed");
    assert_eq!(idempotent.updated_at, mapped.updated_at);

    let child = app
        .set_indexer_download_client_mapping(&admin, "idx-child", Some("sab-disabled"))
        .await
        .expect("managed children support local mappings");
    assert_eq!(child.download_client_id.as_deref(), Some("sab-disabled"));

    let incompatible = app
        .set_indexer_download_client_mapping(&admin, "idx-usenet", Some("torrent"))
        .await
        .expect_err("Usenet indexer must reject torrent-only client");
    assert!(
        matches!(incompatible, AppError::Validation(message) if message.contains("does not support"))
    );

    let unknown_protocol = app
        .set_indexer_download_client_mapping(&admin, "idx-unknown", Some("sab-disabled"))
        .await
        .expect_err("unknown protocol indexer must reject mapping");
    assert!(
        matches!(unknown_protocol, AppError::Validation(message) if message.contains("does not declare"))
    );

    let parent = app
        .set_indexer_download_client_mapping(&admin, "prowlarr-parent", Some("sab-disabled"))
        .await
        .expect_err("Prowlarr management parent must reject mapping");
    assert!(
        matches!(parent, AppError::Validation(message) if message.contains("management parents"))
    );

    let catalog = app
        .get_indexer_download_client_mapping_catalog(&admin)
        .await
        .expect("mapping catalog should load");
    let usenet = catalog
        .indexers
        .iter()
        .find(|indexer| indexer.id == "idx-usenet")
        .expect("Usenet catalog row");
    assert_eq!(usenet.protocol_families, vec!["usenet"]);
    assert!(
        usenet
            .compatible_client_ids
            .contains(&"sab-disabled".to_string())
    );
    assert!(
        !usenet
            .compatible_client_ids
            .contains(&"torrent".to_string())
    );

    let cleared = app
        .set_indexer_download_client_mapping(&admin, "idx-usenet", Some("   "))
        .await
        .expect("empty normalized client id should clear mapping");
    assert_eq!(cleared.download_client_id, None);
}

#[tokio::test]
async fn indexer_create_and_update_persist_mapping_atomically() {
    let starting_indexer = synthetic_direct_nab_indexer_config("idx", "torrent_rss");
    let (app, admin) = bootstrap_with_search_settings_indexer_and_configs(
        Arc::new(StoredSettingsRepo::default()),
        Arc::new(MockIndexerClient),
        vec![starting_indexer],
    );
    app.services
        .integrations
        .download_client_configs
        .create(download_client("weaver", "weaver", true))
        .await
        .expect("Weaver should insert");

    let automatic = app
        .create_indexer_config(
            &admin,
            NewIndexerConfig {
                name: "Automatic NZBGeek".to_string(),
                provider_type: "nzbgeek".to_string(),
                rate_limit_seconds: None,
                rate_limit_burst: None,
                is_enabled: true,
                enable_interactive_search: true,
                enable_auto_search: true,
                proxy_config_id: None,
                download_client_id: None,
                config_json: Some(
                    serde_json::json!({
                        "base_url": "https://api.nzbgeek.info",
                        "api_key": "secret"
                    })
                    .to_string(),
                ),
            },
        )
        .await
        .expect("Automatic create should succeed");
    assert_eq!(automatic.download_client_id, None);

    let mapped = app
        .create_indexer_config(
            &admin,
            NewIndexerConfig {
                name: "Mapped NZBGeek".to_string(),
                provider_type: "nzbgeek".to_string(),
                rate_limit_seconds: None,
                rate_limit_burst: None,
                is_enabled: true,
                enable_interactive_search: true,
                enable_auto_search: true,
                proxy_config_id: None,
                download_client_id: Some("weaver".to_string()),
                config_json: Some(
                    serde_json::json!({
                        "base_url": "https://api.nzbgeek.info",
                        "api_key": "secret"
                    })
                    .to_string(),
                ),
            },
        )
        .await
        .expect("mapped create should succeed");
    assert_eq!(mapped.download_client_id.as_deref(), Some("weaver"));

    let incompatible = app
        .update_indexer_config(
            &admin,
            IndexerConfigUpdate {
                id: "idx".to_string(),
                name: Some("Must Not Persist".to_string()),
                download_client_id: Some(Some("weaver".to_string())),
                ..Default::default()
            },
        )
        .await
        .expect_err("torrent provider must reject a Usenet-only client");
    assert!(matches!(incompatible, AppError::Validation(_)));
    let unchanged = app
        .get_indexer_config(&admin, "idx")
        .await
        .expect("lookup should succeed")
        .expect("indexer should exist");
    assert_ne!(unchanged.name, "Must Not Persist");
    assert_eq!(unchanged.download_client_id, None);

    let switched = app
        .update_indexer_config(
            &admin,
            IndexerConfigUpdate {
                id: "idx".to_string(),
                provider_type: Some("nzbgeek".to_string()),
                download_client_id: Some(Some("weaver".to_string())),
                config_json: Some(
                    serde_json::json!({
                        "base_url": "https://api.nzbgeek.info",
                        "api_key": "secret"
                    })
                    .to_string(),
                ),
                ..Default::default()
            },
        )
        .await
        .expect("provider and mapping should change together");
    assert_eq!(switched.provider_type, "nzbgeek");
    assert_eq!(switched.download_client_id.as_deref(), Some("weaver"));

    let renamed = app
        .update_indexer_config(
            &admin,
            IndexerConfigUpdate {
                id: "idx".to_string(),
                name: Some("Renamed".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("omitting the mapping should preserve it");
    assert_eq!(renamed.download_client_id.as_deref(), Some("weaver"));

    let cleared = app
        .update_indexer_config(
            &admin,
            IndexerConfigUpdate {
                id: "idx".to_string(),
                download_client_id: Some(None),
                ..Default::default()
            },
        )
        .await
        .expect("explicit null should select Automatic");
    assert_eq!(cleared.download_client_id, None);

    let catalog = app
        .get_indexer_download_client_mapping_catalog(&admin)
        .await
        .expect("catalog should load");
    let nzbgeek = catalog
        .provider_compatibility
        .iter()
        .find(|provider| provider.provider_type == "nzbgeek")
        .expect("NZBGeek compatibility should be present");
    assert!(
        nzbgeek
            .compatible_client_ids
            .contains(&"weaver".to_string())
    );
    let prowlarr = catalog
        .provider_compatibility
        .iter()
        .find(|provider| provider.provider_type == "prowlarr")
        .expect("Prowlarr compatibility should be present");
    assert!(!prowlarr.supports_mapping);
}

#[tokio::test]
async fn managed_child_mapping_survives_sync_and_foreign_client_id_stays_opaque() {
    let mut parent = synthetic_direct_nab_indexer_config("prowlarr-parent", "prowlarr");
    parent.enable_interactive_search = false;
    parent.enable_auto_search = false;
    let mut child = synthetic_direct_nab_indexer_config("managed-child", "nzbgeek");
    child.managed_parent_config_id = Some(parent.id.clone());
    child.managed_child_key = Some("prowlarr-indexer-7".to_string());

    let management_client: Arc<dyn IndexerManagementClient> =
        Arc::new(FixedIndexerManagementClient {
            plan: IndexerSyncPlan {
                children: vec![ManagedIndexerChildPlan {
                    child_key: "prowlarr-indexer-7".to_string(),
                    name: "Synced Child".to_string(),
                    provider_type: "nzbgeek".to_string(),
                    config_json: serde_json::json!({
                        "base_url": "https://child.example.invalid/api",
                        "api_key": "synced-secret"
                    })
                    .to_string(),
                    is_enabled: true,
                    enable_interactive_search: true,
                    enable_auto_search: true,
                    managed_metadata_json: Some(
                        serde_json::json!({"downloadClientId": 91}).to_string(),
                    ),
                    caps_snapshot_json: None,
                    routing_scopes: vec![],
                }],
            },
        });
    let (app, admin) = bootstrap_with_search_settings_indexer_configs_and_management(
        Arc::new(StoredSettingsRepo::default()),
        Arc::new(MockIndexerClient),
        vec![parent, child],
        Some(management_client),
    );
    app.services
        .integrations
        .download_client_configs
        .create(download_client("sab", "sabnzbd", true))
        .await
        .expect("client should insert");
    app.set_indexer_download_client_mapping(&admin, "managed-child", Some("sab"))
        .await
        .expect("managed child mapping should save");

    let result = app
        .sync_indexer_config(&admin, "prowlarr-parent")
        .await
        .expect("Prowlarr child sync should succeed");
    assert_eq!(result.updated_ids, vec!["managed-child".to_string()]);
    let synced = app
        .services
        .integrations
        .indexer_configs
        .get_by_id("managed-child")
        .await
        .expect("child lookup should succeed")
        .expect("child should remain");
    assert_eq!(synced.name, "Synced Child");
    assert_eq!(synced.download_client_id.as_deref(), Some("sab"));
    let metadata: serde_json::Value = serde_json::from_str(
        synced
            .managed_metadata_json
            .as_deref()
            .expect("managed metadata should remain opaque"),
    )
    .expect("managed metadata should be valid JSON");
    assert_eq!(metadata["downloadClientId"], 91);
}

#[tokio::test]
async fn indexer_mapping_requires_system_settings_permission() {
    let (app, _admin) = bootstrap_with_search_settings_indexer_and_configs(
        Arc::new(StoredSettingsRepo::default()),
        Arc::new(MockIndexerClient),
        vec![synthetic_direct_nab_indexer_config("idx-usenet", "nzbgeek")],
    );
    app.services
        .integrations
        .download_client_configs
        .create(download_client("sab", "sabnzbd", true))
        .await
        .expect("client should insert");
    let actor = test_user_with_app_permissions("viewer", AppPermissionMask::default());

    let error = app
        .set_indexer_download_client_mapping(&actor, "idx-usenet", Some("sab"))
        .await
        .expect_err("mapping requires system settings permission");
    assert!(matches!(error, AppError::Unauthorized(_)));
}

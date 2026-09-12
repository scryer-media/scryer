use super::*;

#[tokio::test]
async fn remove_completed_download_defaults_true_when_scope_has_no_saved_entry() {
    // Legacy-compat coverage: a stored scope JSON exists but does not include
    // an entry for "weaver". Read path must fall back to the canonical
    // defaults (`removeCompleted=true`, `removeFailed=true`). New installs
    // converge on fully-materialized entries via `normalize_routing_settings`.
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
            "movie",
            &serde_json::json!({
                "other-client": {
                    "enabled": true,
                    "removeCompleted": false,
                    "removeFailed": true
                }
            })
            .to_string(),
        )
        .await;

    let (app, _) =
        bootstrap_with_search_settings_and_indexer(settings, Arc::new(MockIndexerClient));

    assert!(
        app.should_remove_completed_download(None, &MediaFacet::Movie, "weaver")
            .await
    );
    assert!(
        app.should_remove_failed_download(None, &MediaFacet::Movie, "weaver")
            .await
    );
}

#[tokio::test]
async fn library_cleanup_routing_override_beats_facet_cleanup_flags() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);

    settings
        .set_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
            "movie",
            &serde_json::json!({
                "weaver": {
                    "enabled": true,
                    "removeCompleted": true,
                    "removeFailed": false
                }
            })
            .to_string(),
        )
        .await;
    settings
        .set_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
            &movie_library_id,
            &serde_json::json!({
                "weaver": {
                    "enabled": true,
                    "removeCompleted": false,
                    "removeFailed": true
                }
            })
            .to_string(),
        )
        .await;

    let (app, _) =
        bootstrap_with_search_settings_and_indexer(settings, Arc::new(MockIndexerClient));

    assert!(
        !app.should_remove_completed_download(
            Some(movie_library_id.as_str()),
            &MediaFacet::Movie,
            "weaver"
        )
        .await
    );
    assert!(
        app.should_remove_failed_download(
            Some(movie_library_id.as_str()),
            &MediaFacet::Movie,
            "weaver"
        )
        .await
    );
}

#[tokio::test]
async fn category_admission_snapshot_keeps_current_legacy_shadowed_and_moved_routes() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    settings
        .set_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
            "movie",
            &serde_json::json!({
                "active-client": { "enabled": true, "category": " Current RSS " },
                "shadowed-client": { "enabled": false, "category": "Shadowed RSS" }
            })
            .to_string(),
        )
        .await;
    settings
        .set_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY,
            "movie",
            &serde_json::json!({
                "legacy-client": { "enabled": true, "category": "Legacy RSS" }
            })
            .to_string(),
        )
        .await;
    settings
        .set_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
            &movie_library_id,
            &serde_json::json!({
                "removed-client": { "enabled": false, "category": "Moved RSS" }
            })
            .to_string(),
        )
        .await;

    let (app, _) =
        bootstrap_with_search_settings_and_indexer(settings, Arc::new(MockIndexerClient));
    app.refresh_download_client_category_admission()
        .await
        .expect("load category admission snapshot");
    let snapshot = app
        .download_client_category_admission_snapshot()
        .await
        .expect("category admission snapshot");

    for category in ["current rss", "LEGACY RSS", " shadowed rss ", "moved rss"] {
        assert!(
            snapshot.knows_category(category),
            "missing category {category}"
        );
    }
}

#[tokio::test]
async fn library_settings_download_client_routing_override_normalizes_current_clients_and_hydrates_new_ones()
 {
    let settings = Arc::new(StoredSettingsRepo::default());
    let (app, user) =
        bootstrap_with_search_settings_and_indexer(settings.clone(), Arc::new(MockIndexerClient));
    let movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);

    let primary = create_enabled_download_client_config(&app, &user, "Primary", "weaver").await;
    let secondary =
        create_enabled_download_client_config(&app, &user, "Secondary", "sabnzbd").await;

    app.update_library_settings(
        &user,
        &movie_library_id,
        LibrarySettingsOverrideDraft {
            download_client_routing: Some(vec![DownloadClientRoutingSettingsEntry {
                seeding_profile_id: None,
                client_id: primary.id.clone(),
                enabled: true,
                category: Some("movies".to_string()),
                recent_queue_priority: Some("high".to_string()),
                older_queue_priority: Some("low".to_string()),
                remove_completed: false,
                remove_failed: true,
            }]),
            ..empty_library_settings_override()
        },
    )
    .await
    .expect("library routing override should save");

    let raw = settings
        .get_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
            &movie_library_id,
        )
        .await
        .expect("saved library routing JSON");
    let parsed: serde_json::Value =
        serde_json::from_str(&raw).expect("saved library routing JSON should parse");
    assert_eq!(
        parsed[secondary.id.as_str()]["enabled"],
        serde_json::json!(false),
        "saving a library override should materialize current missing clients as disabled",
    );

    let tertiary = create_enabled_download_client_config(&app, &user, "Tertiary", "nzbget").await;
    let library_settings = app
        .get_library_settings(&user, &movie_library_id)
        .await
        .expect("library settings should reload");
    let routing = library_settings
        .download_client_routing_override
        .expect("library override should be present");

    assert_eq!(routing[0].client_id, primary.id);
    assert_eq!(routing[0].category.as_deref(), Some("movies"));
    assert_eq!(routing[0].recent_queue_priority.as_deref(), Some("high"));
    assert_eq!(routing[0].older_queue_priority.as_deref(), Some("low"));
    assert!(!routing[0].remove_completed);
    assert!(routing[0].remove_failed);

    let secondary_entry = routing
        .iter()
        .find(|entry| entry.client_id == secondary.id)
        .expect("secondary client should be present");
    assert!(!secondary_entry.enabled);
    assert_eq!(secondary_entry.category, None);
    assert_eq!(secondary_entry.recent_queue_priority, None);
    assert_eq!(secondary_entry.older_queue_priority, None);
    assert!(secondary_entry.remove_completed);
    assert!(secondary_entry.remove_failed);

    let tertiary_entry = routing
        .iter()
        .find(|entry| entry.client_id == tertiary.id)
        .expect("newly added client should be hydrated as disabled");
    assert!(!tertiary_entry.enabled);
    assert_eq!(tertiary_entry.category, None);
    assert_eq!(tertiary_entry.recent_queue_priority, None);
    assert_eq!(tertiary_entry.older_queue_priority, None);
    assert!(tertiary_entry.remove_completed);
    assert!(tertiary_entry.remove_failed);
}

#[tokio::test]
async fn library_settings_download_client_routing_override_reads_legacy_key_and_clears_it_when_reset()
 {
    let settings = Arc::new(StoredSettingsRepo::default());
    let (app, user) =
        bootstrap_with_search_settings_and_indexer(settings.clone(), Arc::new(MockIndexerClient));
    let movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);

    let primary = create_enabled_download_client_config(&app, &user, "Primary", "weaver").await;
    let secondary =
        create_enabled_download_client_config(&app, &user, "Secondary", "sabnzbd").await;

    settings
        .set_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY,
            &movie_library_id,
            &serde_json::json!({
                primary.id.as_str(): {
                    "enabled": true,
                    "category": "movies",
                    "recentQueuePriority": "high",
                    "olderQueuePriority": "low",
                    "removeCompleted": false,
                    "removeFailed": true
                }
            })
            .to_string(),
        )
        .await;

    let library_settings = app
        .get_library_settings(&user, &movie_library_id)
        .await
        .expect("library settings should read legacy routing override");
    let routing = library_settings
        .download_client_routing_override
        .expect("legacy routing override should be surfaced");

    let primary_entry = routing
        .iter()
        .find(|entry| entry.client_id == primary.id)
        .expect("primary client should be present");
    assert!(primary_entry.enabled);
    assert_eq!(primary_entry.category.as_deref(), Some("movies"));
    assert_eq!(primary_entry.recent_queue_priority.as_deref(), Some("high"));
    assert_eq!(primary_entry.older_queue_priority.as_deref(), Some("low"));
    assert!(!primary_entry.remove_completed);
    assert!(primary_entry.remove_failed);

    let secondary_entry = routing
        .iter()
        .find(|entry| entry.client_id == secondary.id)
        .expect("missing clients should hydrate as disabled");
    assert!(!secondary_entry.enabled);

    app.update_library_settings(&user, &movie_library_id, empty_library_settings_override())
        .await
        .expect("resetting library settings should succeed");

    assert!(
        settings
            .get_scoped_value(
                SETTINGS_SCOPE_SYSTEM,
                DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
                &movie_library_id,
            )
            .await
            .is_none(),
        "resetting should remove the canonical library override",
    );
    assert!(
        settings
            .get_scoped_value(
                SETTINGS_SCOPE_SYSTEM,
                LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY,
                &movie_library_id,
            )
            .await
            .is_none(),
        "resetting should remove the legacy library override too",
    );
}

#[tokio::test]
async fn library_settings_download_client_routing_override_ignores_invalid_json() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let (app, user) =
        bootstrap_with_search_settings_and_indexer(settings.clone(), Arc::new(MockIndexerClient));
    let movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);

    settings
        .set_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
            &movie_library_id,
            "[]",
        )
        .await;

    let library_settings = app
        .get_library_settings(&user, &movie_library_id)
        .await
        .expect("library settings should load");

    assert!(
        library_settings.download_client_routing_override.is_none(),
        "invalid library routing JSON should be ignored instead of materialized as a disabled override",
    );
}

#[tokio::test]
async fn ensure_download_client_routing_entry_for_client_writes_full_default_entry() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let (app, actor) =
        bootstrap_with_search_settings_and_indexer(settings.clone(), Arc::new(MockIndexerClient));

    app.ensure_download_client_routing_entry_for_client(&actor, "weaver")
        .await
        .expect("ensure routing entry");

    for scope_id in ["movie", "series", "anime"] {
        let raw = settings
            .get_scoped_value(
                SETTINGS_SCOPE_SYSTEM,
                DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
                scope_id,
            )
            .await
            .unwrap_or_else(|| panic!("expected routing JSON for scope {scope_id}"));
        let parsed: serde_json::Value = serde_json::from_str(&raw).expect("routing JSON parses");
        let entry = parsed
            .get("weaver")
            .and_then(|v| v.as_object())
            .unwrap_or_else(|| panic!("expected weaver entry for scope {scope_id}"));
        assert_eq!(entry.get("enabled"), Some(&serde_json::json!(true)));
        assert_eq!(entry.get("category"), Some(&serde_json::json!("")));
        assert_eq!(
            entry.get("recentQueuePriority"),
            Some(&serde_json::json!(""))
        );
        assert_eq!(
            entry.get("olderQueuePriority"),
            Some(&serde_json::json!(""))
        );
        assert_eq!(entry.get("removeCompleted"), Some(&serde_json::json!(true)));
        assert_eq!(entry.get("removeFailed"), Some(&serde_json::json!(true)));
        assert!(entry.contains_key("priority"));
    }
}

#[tokio::test]
async fn normalize_routing_settings_backfills_partial_legacy_download_client_json() {
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
            "movie",
            &serde_json::json!({
                "weaver": {
                    "enabled": true,
                    "category": "",
                    "priority": 1
                }
            })
            .to_string(),
        )
        .await;

    let (app, _) =
        bootstrap_with_search_settings_and_indexer(settings.clone(), Arc::new(MockIndexerClient));

    app.normalize_routing_settings()
        .await
        .expect("normalize routing settings");

    let raw = settings
        .get_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
            "movie",
        )
        .await
        .expect("routing JSON present after normalize");
    let parsed: serde_json::Value = serde_json::from_str(&raw).expect("routing JSON parses");
    let entry = parsed
        .get("weaver")
        .and_then(|v| v.as_object())
        .expect("weaver entry");
    assert_eq!(entry.get("removeCompleted"), Some(&serde_json::json!(true)));
    assert_eq!(entry.get("removeFailed"), Some(&serde_json::json!(true)));
    assert_eq!(
        entry.get("recentQueuePriority"),
        Some(&serde_json::json!(""))
    );
    assert_eq!(
        entry.get("olderQueuePriority"),
        Some(&serde_json::json!(""))
    );
    // Existing explicit values must not be overwritten.
    assert_eq!(entry.get("priority"), Some(&serde_json::json!(1)));
    assert_eq!(entry.get("category"), Some(&serde_json::json!("")));
}

#[tokio::test]
async fn normalize_routing_settings_is_idempotent_for_complete_entries() {
    // Pre-seed a fully-normalized entry with non-default values. The normalize
    // pass must not overwrite explicit values back to canonical defaults.
    let original = serde_json::json!({
        "weaver": {
            "enabled": false,
            "category": "movies",
            "recentQueuePriority": "high",
            "olderQueuePriority": "low",
            "removeCompleted": false,
            "removeFailed": true,
            "priority": 7
        }
    })
    .to_string();
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
            "movie",
            &original,
        )
        .await;

    let (app, _) =
        bootstrap_with_search_settings_and_indexer(settings.clone(), Arc::new(MockIndexerClient));

    app.normalize_routing_settings()
        .await
        .expect("first normalize");
    app.normalize_routing_settings()
        .await
        .expect("second normalize");

    let after = settings
        .get_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
            "movie",
        )
        .await
        .expect("routing JSON present");
    let parsed: serde_json::Value = serde_json::from_str(&after).expect("routing JSON parses");
    let entry = parsed
        .get("weaver")
        .and_then(|v| v.as_object())
        .expect("weaver entry");
    assert_eq!(entry.get("enabled"), Some(&serde_json::json!(false)));
    assert_eq!(entry.get("category"), Some(&serde_json::json!("movies")));
    assert_eq!(
        entry.get("recentQueuePriority"),
        Some(&serde_json::json!("high"))
    );
    assert_eq!(
        entry.get("olderQueuePriority"),
        Some(&serde_json::json!("low"))
    );
    assert_eq!(
        entry.get("removeCompleted"),
        Some(&serde_json::json!(false))
    );
    assert_eq!(entry.get("removeFailed"), Some(&serde_json::json!(true)));
    assert_eq!(entry.get("priority"), Some(&serde_json::json!(7)));
}

#[tokio::test]
async fn ensure_indexer_routing_entry_for_indexer_writes_full_default_entry() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let (app, actor) =
        bootstrap_with_search_settings_and_indexer(settings.clone(), Arc::new(MockIndexerClient));

    app.ensure_indexer_routing_entry_for_indexer(&actor, "indexer-1")
        .await
        .expect("ensure indexer routing entry");

    for (scope_id, expected_categories) in [
        ("movie", serde_json::json!(["2000"])),
        ("series", serde_json::json!(["5000"])),
        ("anime", serde_json::json!(["5070"])),
    ] {
        let raw = settings
            .get_scoped_value(
                SETTINGS_SCOPE_SYSTEM,
                INDEXER_ROUTING_SETTINGS_KEY,
                scope_id,
            )
            .await
            .unwrap_or_else(|| panic!("expected indexer routing JSON for scope {scope_id}"));
        let parsed: serde_json::Value =
            serde_json::from_str(&raw).expect("indexer routing JSON parses");
        let entry = parsed
            .get("indexer-1")
            .and_then(|v| v.as_object())
            .unwrap_or_else(|| panic!("expected indexer-1 entry for scope {scope_id}"));
        assert_eq!(entry.get("enabled"), Some(&serde_json::json!(true)));
        assert_eq!(entry.get("categories"), Some(&expected_categories));
        assert!(entry.contains_key("priority"));
    }
}

#[tokio::test]
async fn create_indexer_config_writes_default_routing_entries() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let (app, actor) =
        bootstrap_with_search_settings_and_indexer(settings.clone(), Arc::new(MockIndexerClient));

    let created = app
        .create_indexer_config(
            &actor,
            NewIndexerConfig {
                name: "NZBGeek".to_string(),
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
                        "api_key": "0123456789abcdef"
                    })
                    .to_string(),
                ),
            },
        )
        .await
        .expect("create indexer config");

    let raw = settings
        .get_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            INDEXER_ROUTING_SETTINGS_KEY,
            "series",
        )
        .await
        .expect("series indexer routing JSON present");
    let parsed: serde_json::Value = serde_json::from_str(&raw).expect("routing JSON parses");
    let entry = parsed
        .get(&created.id)
        .and_then(|value| value.as_object())
        .expect("created indexer routing entry");
    assert_eq!(entry.get("enabled"), Some(&serde_json::json!(true)));
    assert_eq!(entry.get("categories"), Some(&serde_json::json!(["5000"])));
    assert!(entry.contains_key("priority"));
}

#[tokio::test]
async fn ensure_indexer_routing_entries_for_existing_indexers_backfills_missing_rows() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let (app, _) =
        bootstrap_with_search_settings_and_indexer(settings.clone(), Arc::new(MockIndexerClient));

    app.services
        .integrations
        .indexer_configs
        .create(IndexerConfig {
            id: "existing-indexer".to_string(),
            name: "NZBGeek".to_string(),
            provider_type: "nzbgeek".to_string(),
            base_url: "https://api.nzbgeek.info".to_string(),
            api_key_encrypted: None,
            rate_limit_seconds: None,
            rate_limit_burst: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            config_json: Some(
                serde_json::json!({
                    "base_url": "https://api.nzbgeek.info",
                    "api_key": "0123456789abcdef"
                })
                .to_string(),
            ),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .expect("seed existing indexer config");

    app.ensure_indexer_routing_entries_for_existing_indexers()
        .await
        .expect("ensure existing indexer routing");

    let raw = settings
        .get_scoped_value(SETTINGS_SCOPE_SYSTEM, INDEXER_ROUTING_SETTINGS_KEY, "anime")
        .await
        .expect("anime indexer routing JSON present");
    let parsed: serde_json::Value = serde_json::from_str(&raw).expect("routing JSON parses");
    let entry = parsed
        .get("existing-indexer")
        .and_then(|value| value.as_object())
        .expect("existing indexer routing entry");
    assert_eq!(entry.get("categories"), Some(&serde_json::json!(["5070"])));
}

#[tokio::test]
async fn normalize_routing_settings_backfills_missing_indexer_categories_from_scope() {
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            INDEXER_ROUTING_SETTINGS_KEY,
            "anime",
            &serde_json::json!({
                "indexer-1": {
                    "enabled": true
                }
            })
            .to_string(),
        )
        .await;

    let (app, _) =
        bootstrap_with_search_settings_and_indexer(settings.clone(), Arc::new(MockIndexerClient));

    app.normalize_routing_settings()
        .await
        .expect("normalize indexer routing");

    let raw = settings
        .get_scoped_value(SETTINGS_SCOPE_SYSTEM, INDEXER_ROUTING_SETTINGS_KEY, "anime")
        .await
        .expect("indexer routing JSON present after normalize");
    let parsed: serde_json::Value = serde_json::from_str(&raw).expect("routing JSON parses");
    let entry = parsed
        .get("indexer-1")
        .and_then(|value| value.as_object())
        .expect("indexer-1 entry");
    assert_eq!(entry.get("categories"), Some(&serde_json::json!(["5070"])));
    assert!(entry.contains_key("priority"));
}

#[tokio::test]
async fn normalize_routing_settings_backfills_partial_legacy_indexer_json() {
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            INDEXER_ROUTING_SETTINGS_KEY,
            "movie",
            &serde_json::json!({
                "indexer-1": {
                    "categories": ["2000"]
                }
            })
            .to_string(),
        )
        .await;

    let (app, _) =
        bootstrap_with_search_settings_and_indexer(settings.clone(), Arc::new(MockIndexerClient));

    app.normalize_routing_settings()
        .await
        .expect("normalize indexer routing");

    let raw = settings
        .get_scoped_value(SETTINGS_SCOPE_SYSTEM, INDEXER_ROUTING_SETTINGS_KEY, "movie")
        .await
        .expect("indexer routing JSON present after normalize");
    let parsed: serde_json::Value =
        serde_json::from_str(&raw).expect("indexer routing JSON parses");
    let entry = parsed
        .get("indexer-1")
        .and_then(|v| v.as_object())
        .expect("indexer-1 entry");
    assert_eq!(entry.get("enabled"), Some(&serde_json::json!(true)));
    assert!(entry.contains_key("priority"));
    // Existing categories must not be overwritten.
    assert_eq!(entry.get("categories"), Some(&serde_json::json!(["2000"])));
}

#[tokio::test]
async fn normalize_routing_settings_assigns_distinct_priorities_to_multiple_indexers() {
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            INDEXER_ROUTING_SETTINGS_KEY,
            "series",
            &serde_json::json!({
                "indexer-1": {
                    "enabled": true,
                    "categories": ["5000"]
                },
                "indexer-2": {
                    "enabled": true,
                    "categories": ["5000"]
                }
            })
            .to_string(),
        )
        .await;

    let (app, _) =
        bootstrap_with_search_settings_and_indexer(settings.clone(), Arc::new(MockIndexerClient));

    app.normalize_routing_settings()
        .await
        .expect("normalize indexer routing");

    let raw = settings
        .get_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            INDEXER_ROUTING_SETTINGS_KEY,
            "series",
        )
        .await
        .expect("indexer routing JSON present after normalize");
    let parsed: serde_json::Value =
        serde_json::from_str(&raw).expect("indexer routing JSON parses");
    let first_priority = parsed["indexer-1"]["priority"]
        .as_i64()
        .expect("indexer-1 priority");
    let second_priority = parsed["indexer-2"]["priority"]
        .as_i64()
        .expect("indexer-2 priority");

    assert_ne!(first_priority, second_priority);
}

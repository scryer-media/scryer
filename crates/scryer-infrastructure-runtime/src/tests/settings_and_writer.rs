use super::*;

#[tokio::test]
async fn serialized_writer_handles_settings_batch_and_encrypted_upserts() {
    let (services, db) = temp_services("scryer_settings_writer").await;
    services
        .set_encryption_key(crate::encryption::EncryptionKey::from_bytes([7; 32]))
        .await
        .expect("encryption key should set");
    let settings = SettingsStore::new(services.datastore(), services.encryption_key_state());

    settings
        .batch_ensure_setting_definitions(vec![crate::types::SettingDefinitionSeed {
            category: "general".to_string(),
            scope: "system".to_string(),
            key_name: "secret.value".to_string(),
            data_type: "string".to_string(),
            default_value_json: "\"default\"".to_string(),
            is_sensitive: true,
            validation_json: None,
        }])
        .await
        .expect("definitions should seed");

    settings
        .batch_upsert_settings_if_not_overridden(vec![(
            "system".to_string(),
            "secret.value".to_string(),
            "\"seeded\"".to_string(),
            "migration".to_string(),
        )])
        .await
        .expect("batch upsert should succeed");

    let seeded = settings
        .get_setting_with_defaults("system", "secret.value", None)
        .await
        .expect("seeded setting should load")
        .expect("seeded setting should exist");
    assert_eq!(seeded.effective_value_json, "\"seeded\"");

    let updated = settings
        .upsert_setting_value(
            "system",
            "secret.value",
            None,
            "\"overridden\"",
            "user",
            None,
        )
        .await
        .expect("direct upsert should succeed");
    assert_eq!(updated.effective_value_json, "\"overridden\"");

    settings
        .delete_setting_value("system", "secret.value", None)
        .await
        .expect("delete override should succeed");

    let reverted = settings
        .get_setting_with_defaults("system", "secret.value", None)
        .await
        .expect("setting should still load")
        .expect("setting should still exist");
    assert_eq!(reverted.effective_value_json, "\"default\"");

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn settings_with_defaults_store_reads_scoped_overrides() {
    let (services, db) = temp_services("scryer_settings_parity").await;
    let encryption_key = crate::encryption::EncryptionKey::from_bytes([11; 32]);
    services
        .set_encryption_key(encryption_key.clone())
        .await
        .expect("encryption key should set");
    let settings = SettingsStore::new(services.datastore(), services.encryption_key_state());

    settings
        .batch_ensure_setting_definitions(vec![
            crate::types::SettingDefinitionSeed {
                category: "general".to_string(),
                scope: "system".to_string(),
                key_name: "secret.global".to_string(),
                data_type: "string".to_string(),
                default_value_json: "\"default-global\"".to_string(),
                is_sensitive: true,
                validation_json: None,
            },
            crate::types::SettingDefinitionSeed {
                category: "general".to_string(),
                scope: "system".to_string(),
                key_name: "secret.scoped".to_string(),
                data_type: "string".to_string(),
                default_value_json: "\"default-scoped\"".to_string(),
                is_sensitive: true,
                validation_json: None,
            },
        ])
        .await
        .expect("definitions should seed");

    settings
        .upsert_setting_value(
            "system",
            "secret.global",
            None,
            "\"overridden-global\"",
            "user",
            None,
        )
        .await
        .expect("global override should succeed");
    settings
        .upsert_setting_value(
            "system",
            "secret.scoped",
            Some("movie".to_string()),
            "\"overridden-scoped\"",
            "user",
            None,
        )
        .await
        .expect("scoped override should succeed");

    let query_rows = settings
        .list_settings_with_defaults("system", Some("movie".to_string()))
        .await
        .expect("settings should load");

    let summarize = |rows: Vec<crate::types::SettingsValueRecord>| {
        let mut summary = rows
            .into_iter()
            .map(|row| {
                (
                    row.key_name,
                    row.scope_id,
                    row.effective_value_json,
                    row.value_json,
                    row.source,
                    row.is_sensitive,
                )
            })
            .collect::<Vec<_>>();
        summary.sort_by(|left, right| left.0.cmp(&right.0));
        summary
    };
    assert!(summarize(query_rows).contains(&(
        "secret.scoped".to_string(),
        Some("movie".to_string()),
        "\"overridden-scoped\"".to_string(),
        Some("\"overridden-scoped\"".to_string()),
        Some("user".to_string()),
        true,
    )));

    let query_record = settings
        .get_setting_with_defaults("system", "secret.scoped", Some("movie".to_string()))
        .await
        .expect("single setting should load");
    let summarize_record = |row: Option<crate::types::SettingsValueRecord>| {
        row.map(|record| {
            (
                record.key_name,
                record.scope_id,
                record.effective_value_json,
                record.value_json,
                record.source,
                record.is_sensitive,
            )
        })
    };
    assert_eq!(
        summarize_record(query_record),
        Some((
            "secret.scoped".to_string(),
            Some("movie".to_string()),
            "\"overridden-scoped\"".to_string(),
            Some("\"overridden-scoped\"".to_string()),
            Some("user".to_string()),
            true,
        ))
    );
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn explicit_setting_query_skips_definition_defaults_for_missing_scopes() {
    let (services, db) = temp_services("scryer_settings_explicit").await;
    let settings = SettingsStore::new(services.datastore(), services.encryption_key_state());

    settings
        .batch_ensure_setting_definitions(vec![crate::types::SettingDefinitionSeed {
            category: "media".to_string(),
            scope: "system".to_string(),
            key_name: "quality.profile_id".to_string(),
            data_type: "string".to_string(),
            default_value_json: "\"4k\"".to_string(),
            is_sensitive: false,
            validation_json: None,
        }])
        .await
        .expect("definitions should seed");

    settings
        .upsert_setting_value(
            "system",
            "quality.profile_id",
            Some("series".to_string()),
            "\"wizard-series\"",
            "user",
            None,
        )
        .await
        .expect("facet override should save");

    let inherited = settings
        .get_setting_json(
            "system",
            "quality.profile_id",
            Some("series_default_library".to_string()),
        )
        .await
        .expect("inherited lookup should succeed");
    assert_eq!(inherited.as_deref(), Some("\"4k\""));

    let explicit = settings
        .get_setting_json_explicit(
            "system",
            "quality.profile_id",
            Some("series_default_library".to_string()),
        )
        .await
        .expect("explicit lookup should succeed");
    assert_eq!(explicit, None);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn sqlite_library_scoped_download_client_routing_round_trips_explicit_json() {
    let (services, db) = temp_services("scryer_library_download_client_routing").await;
    let settings = SettingsStore::new(services.datastore(), services.encryption_key_state());

    settings
        .batch_ensure_setting_definitions(vec![crate::types::SettingDefinitionSeed {
            category: "media".to_string(),
            scope: "system".to_string(),
            key_name: "download_client.routing".to_string(),
            data_type: "json".to_string(),
            default_value_json: "{}".to_string(),
            is_sensitive: false,
            validation_json: None,
        }])
        .await
        .expect("definitions should seed");

    let library_id = "series_default_library";
    let value_json = serde_json::json!({
        "weaver": {
            "enabled": true,
            "category": "series",
            "recentQueuePriority": "high",
            "olderQueuePriority": "normal",
            "removeCompleted": true,
            "removeFailed": false
        }
    })
    .to_string();

    SettingsRepository::upsert_setting_json(
        &settings,
        "system",
        "download_client.routing",
        Some(library_id.to_string()),
        value_json.clone(),
        "test",
        None,
    )
    .await
    .expect("library-scoped routing should save");

    let explicit = SettingsRepository::get_setting_json_explicit(
        &settings,
        "system",
        "download_client.routing",
        Some(library_id.to_string()),
    )
    .await
    .expect("explicit lookup should succeed");
    assert_eq!(explicit.as_deref(), Some(value_json.as_str()));

    let default_lookup = SettingsRepository::get_setting_json(
        &settings,
        "system",
        "download_client.routing",
        Some("another_library".to_string()),
    )
    .await
    .expect("default lookup should succeed");
    assert_eq!(default_lookup.as_deref(), Some("{}"));

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn serialized_writer_handles_notification_channel_and_subscription_round_trip() {
    let (services, db) = temp_services("scryer_notification_writer").await;
    services
        .set_encryption_key(crate::encryption::EncryptionKey::from_bytes([9; 32]))
        .await
        .expect("encryption key should set");
    let store = NotificationStore::new(services.datastore(), services.encryption_key_state());
    let now = Utc::now();

    let channel = NotificationChannelConfig {
        id: "channel-1".to_string(),
        name: "Discord".to_string(),
        channel_type: ChannelType::parse("discord").expect("channel type"),
        config_json: r#"{"url":"https://example.com/webhook"}"#.to_string(),
        media_server_connection_id: None,
        is_enabled: true,
        created_at: now,
        updated_at: now,
    };
    NotificationChannelRepository::create_channel(&store, channel.clone())
        .await
        .expect("channel should create");

    let fetched = NotificationChannelRepository::get_channel(&store, &channel.id)
        .await
        .expect("channel lookup should succeed")
        .expect("channel should exist");
    assert_eq!(fetched.config_json, channel.config_json);

    let updated_channel = NotificationChannelConfig {
        name: "Discord Alerts".to_string(),
        config_json: r#"{"url":"https://example.com/updated"}"#.to_string(),
        is_enabled: false,
        updated_at: Utc::now(),
        ..fetched.clone()
    };
    let updated = NotificationChannelRepository::update_channel(&store, updated_channel.clone())
        .await
        .expect("channel should update");
    assert_eq!(updated.name, "Discord Alerts");
    assert_eq!(updated.config_json, updated_channel.config_json);

    let subscription = NotificationSubscription {
        id: "subscription-1".to_string(),
        channel_id: Some(updated.id.clone()),
        target_kind: scryer_domain::NotificationTargetKind::PluginChannel,
        target_id: updated.id.clone(),
        event_type: NotificationEventType::ImportComplete,
        scope: "global".to_string(),
        scope_id: None,
        is_enabled: true,
        created_at: now,
        updated_at: now,
    };
    NotificationSubscriptionRepository::create_subscription(&store, subscription.clone())
        .await
        .expect("subscription should create");

    let updated_subscription = NotificationSubscription {
        is_enabled: false,
        updated_at: Utc::now(),
        ..subscription.clone()
    };
    NotificationSubscriptionRepository::update_subscription(&store, updated_subscription.clone())
        .await
        .expect("subscription should update");

    let later_subscription = NotificationSubscription {
        id: "subscription-2".to_string(),
        scope: "movie".to_string(),
        scope_id: Some("title-1".to_string()),
        created_at: now + chrono::Duration::seconds(1),
        updated_at: now + chrono::Duration::seconds(1),
        ..subscription.clone()
    };
    NotificationSubscriptionRepository::create_subscription(&store, later_subscription.clone())
        .await
        .expect("second subscription should create");

    let by_event = NotificationSubscriptionRepository::list_subscriptions_for_event(
        &store,
        NotificationEventType::ImportComplete,
    )
    .await
    .expect("event subscriptions should load");
    assert_eq!(by_event.len(), 2);
    assert_eq!(by_event[0].id, later_subscription.id);
    assert_eq!(by_event[1].id, subscription.id);
    assert!(
        !by_event[1].is_enabled,
        "event listing should preserve disabled rows for dispatcher-side filtering"
    );

    let by_channel =
        NotificationSubscriptionRepository::list_subscriptions_for_channel(&store, &updated.id)
            .await
            .expect("subscription list should load");
    assert_eq!(by_channel.len(), 2);

    NotificationSubscriptionRepository::delete_subscription(&store, &subscription.id)
        .await
        .expect("subscription should delete");
    NotificationSubscriptionRepository::delete_subscription(&store, &later_subscription.id)
        .await
        .expect("second subscription should delete");
    assert!(matches!(
        NotificationSubscriptionRepository::delete_subscription(&store, &subscription.id).await,
        Err(scryer_application::AppError::NotFound(_))
    ));
    NotificationChannelRepository::delete_channel(&store, &updated.id)
        .await
        .expect("channel should delete");
    assert!(matches!(
        NotificationChannelRepository::delete_channel(&store, &updated.id).await,
        Err(scryer_application::AppError::NotFound(_))
    ));

    let remaining =
        NotificationSubscriptionRepository::list_subscriptions_for_channel(&store, &updated.id)
            .await
            .expect("subscription list should still load");
    assert!(remaining.is_empty());
    assert!(
        NotificationChannelRepository::get_channel(&store, &updated.id)
            .await
            .expect("channel lookup should succeed")
            .is_none()
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn serialized_writer_handles_download_client_reorder() {
    let (services, db) = temp_services("scryer_download_client_writer").await;
    let store =
        DownloadClientConfigStore::new(services.datastore(), services.encryption_key_state());
    let now = Utc::now();

    let client_a = DownloadClientConfig {
        id: "client-a".to_string(),
        name: "Client A".to_string(),
        client_type: "weaver".to_string(),
        config_json: "{}".to_string(),
        client_priority: 0,
        is_enabled: true,
        status: DownloadClientStatus::Healthy,
        last_error: None,
        last_seen_at: None,
        created_at: now,
        updated_at: now,
    };
    let client_b = DownloadClientConfig {
        id: "client-b".to_string(),
        name: "Client B".to_string(),
        client_type: "sabnzbd".to_string(),
        config_json: "{}".to_string(),
        client_priority: 1,
        is_enabled: true,
        status: DownloadClientStatus::Healthy,
        last_error: None,
        last_seen_at: None,
        created_at: now,
        updated_at: now,
    };

    DownloadClientConfigRepository::create(&store, client_a.clone())
        .await
        .expect("first client should create");
    DownloadClientConfigRepository::create(&store, client_b.clone())
        .await
        .expect("second client should create");

    DownloadClientConfigRepository::reorder(&store, vec![client_b.id.clone(), client_a.id.clone()])
        .await
        .expect("reorder should succeed");

    let ordered = DownloadClientConfigRepository::list(&store, None)
        .await
        .expect("clients should list");
    let ordered_ids: Vec<String> = ordered.into_iter().map(|client| client.id).collect();
    assert_eq!(ordered_ids, vec![client_b.id, client_a.id]);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn serialized_writer_handles_release_attempts_and_vacuum_into() {
    let (services, db) = temp_services("scryer_release_writer").await;
    services
        .set_encryption_key(crate::encryption::EncryptionKey::from_bytes([17; 32]))
        .await
        .expect("encryption key should configure");
    let release_store = ReleaseStore::new(services.datastore(), services.encryption_key_state());

    ReleaseAttemptRepository::record_release_attempt(
        &release_store,
        None,
        Some("weaver".to_string()),
        Some("Farwander.S08E05".to_string()),
        ReleaseDownloadAttemptOutcome::Failed,
        Some("boom".to_string()),
        Some("secret".to_string()),
    )
    .await
    .expect("release attempt should record");

    let failures = ReleaseAttemptRepository::list_failed_release_signatures(&release_store, 10)
        .await
        .expect("failed signatures should list");
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].source_hint.as_deref(), Some("weaver"));

    let latest_password = ReleaseAttemptRepository::get_latest_source_password(
        &release_store,
        None,
        Some("weaver"),
        Some("Farwander.S08E05"),
    )
    .await
    .expect("latest password should load");
    assert_eq!(latest_password.as_deref(), Some("secret"));

    let stored_password: String =
        sqlx::query_scalar("SELECT source_password FROM release_download_attempts LIMIT 1")
            .fetch_one(services.pool())
            .await
            .expect("stored password should load");
    assert!(crate::encryption::is_encrypted(&stored_password));
    assert_ne!(stored_password, "secret");

    let vacuum_dest = std::env::temp_dir().join(format!(
        "scryer_release_writer_copy_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    services
        .vacuum_into(vacuum_dest.to_string_lossy().as_ref())
        .await
        .expect("vacuum into should succeed");
    assert!(vacuum_dest.exists());

    let _ = std::fs::remove_file(vacuum_dest);
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn source_password_writes_are_encrypted_at_rest_sqlite() {
    let (services, db) = temp_services("scryer_source_password_write").await;
    services
        .set_encryption_key(crate::encryption::EncryptionKey::from_bytes([20; 32]))
        .await
        .expect("encryption key should configure");

    let release_store = ReleaseStore::new(services.datastore(), services.encryption_key_state());
    ReleaseAttemptRepository::record_release_attempt(
        &release_store,
        None,
        Some("weaver".to_string()),
        Some("Encrypted.Release".to_string()),
        ReleaseDownloadAttemptOutcome::Success,
        None,
        Some("release-secret".to_string()),
    )
    .await
    .expect("release attempt should record");

    let now = Utc::now().to_rfc3339();
    let pending_store =
        PendingReleaseStore::new(services.datastore(), services.encryption_key_state());
    let pending_release = scryer_application::PendingRelease {
        id: "pending-encrypted".to_string(),
        wanted_item_id: "wanted-encrypted".to_string(),
        title_id: "title-encrypted".to_string(),
        release_title: "Encrypted Pending".to_string(),
        release_url: Some("https://example.invalid/encrypted.nzb".to_string()),
        source_kind: None,
        release_size_bytes: Some(42),
        release_score: 100,
        scoring_log_json: None,
        indexer_source: Some("weaver".to_string()),
        indexer_id: None,
        release_guid: Some("guid-encrypted".to_string()),
        added_at: now.clone(),
        last_observed_at: now.clone(),
        delay_until: now,
        status: scryer_application::PendingReleaseStatus::Waiting,
        grabbed_at: None,
        source_password: Some("pending-secret".to_string()),
        published_at: None,
        info_hash: None,
        seed_minimums: Default::default(),
        seeders: None,
        release_identity: "guid:weaver:guid-encrypted".to_string(),
        coverage_identity: "scope:wanted-encrypted".to_string(),
        role: scryer_application::PendingReleaseRole::Primary,
        last_decision_code: None,
        release_age_unknown: false,
    };
    PendingReleaseRepository::insert_pending_release(&pending_store, &pending_release)
        .await
        .expect("pending release should insert");

    let stored_release_password: String =
        sqlx::query_scalar("SELECT source_password FROM release_download_attempts LIMIT 1")
            .fetch_one(services.pool())
            .await
            .expect("stored release password should load");
    let stored_pending_password: String = sqlx::query_scalar(
        "SELECT source_password FROM pending_releases WHERE id = 'pending-encrypted'",
    )
    .fetch_one(services.pool())
    .await
    .expect("stored pending password should load");
    assert!(crate::encryption::is_encrypted(&stored_release_password));
    assert!(crate::encryption::is_encrypted(&stored_pending_password));
    assert_ne!(stored_release_password, "release-secret");
    assert_ne!(stored_pending_password, "pending-secret");

    let latest_password = ReleaseAttemptRepository::get_latest_source_password(
        &release_store,
        None,
        Some("weaver"),
        Some("Encrypted.Release"),
    )
    .await
    .expect("latest password should load");
    assert_eq!(latest_password.as_deref(), Some("release-secret"));

    let loaded_pending =
        PendingReleaseRepository::get_pending_release(&pending_store, "pending-encrypted")
            .await
            .expect("pending release should load")
            .expect("pending release should exist");
    assert_eq!(
        loaded_pending.source_password.as_deref(),
        Some("pending-secret")
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn source_password_backfill_encrypts_legacy_sqlite_rows() {
    let (services, db) = temp_services("scryer_source_password_backfill").await;
    services
        .set_encryption_key(crate::encryption::EncryptionKey::from_bytes([18; 32]))
        .await
        .expect("encryption key should configure");

    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO release_download_attempts
         (id, title_id, source_hint, source_title, outcome, error_message, source_password,
          attempted_at, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("legacy-attempt")
    .bind(None::<String>)
    .bind("weaver")
    .bind("Legacy.Release")
    .bind("success")
    .bind(None::<String>)
    .bind("legacy-secret")
    .bind(&now)
    .bind(&now)
    .bind(&now)
    .execute(services.pool())
    .await
    .expect("legacy release attempt should insert");

    sqlx::query(
        "INSERT INTO pending_releases
         (id, wanted_item_id, title_id, release_title, release_score, added_at, delay_until,
          status, source_password)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("legacy-pending")
    .bind("wanted-1")
    .bind("title-1")
    .bind("Legacy.Pending")
    .bind(100_i64)
    .bind(&now)
    .bind(&now)
    .bind("waiting")
    .bind("pending-secret")
    .execute(services.pool())
    .await
    .expect("legacy pending release should insert");

    let release_store = ReleaseStore::new(services.datastore(), services.encryption_key_state());
    let pending_store =
        PendingReleaseStore::new(services.datastore(), services.encryption_key_state());

    let release_updates = release_store
        .backfill_source_passwords()
        .await
        .expect("release source passwords should backfill");
    let pending_updates = pending_store
        .backfill_source_passwords()
        .await
        .expect("pending source passwords should backfill");
    assert_eq!(release_updates, 1);
    assert_eq!(pending_updates, 1);

    let stored_release_password: String = sqlx::query_scalar(
        "SELECT source_password FROM release_download_attempts WHERE id = 'legacy-attempt'",
    )
    .fetch_one(services.pool())
    .await
    .expect("stored release password should load");
    let stored_pending_password: String = sqlx::query_scalar(
        "SELECT source_password FROM pending_releases WHERE id = 'legacy-pending'",
    )
    .fetch_one(services.pool())
    .await
    .expect("stored pending password should load");
    assert!(crate::encryption::is_encrypted(&stored_release_password));
    assert!(crate::encryption::is_encrypted(&stored_pending_password));
    assert_ne!(stored_release_password, "legacy-secret");
    assert_ne!(stored_pending_password, "pending-secret");

    let latest_password = ReleaseAttemptRepository::get_latest_source_password(
        &release_store,
        None,
        Some("weaver"),
        Some("Legacy.Release"),
    )
    .await
    .expect("latest password should load");
    assert_eq!(latest_password.as_deref(), Some("legacy-secret"));

    let pending_release =
        PendingReleaseRepository::get_pending_release(&pending_store, "legacy-pending")
            .await
            .expect("pending release should load")
            .expect("pending release should exist");
    assert_eq!(
        pending_release.source_password.as_deref(),
        Some("pending-secret")
    );

    assert_eq!(
        release_store
            .backfill_source_passwords()
            .await
            .expect("release source password backfill should be idempotent"),
        0
    );
    assert_eq!(
        pending_store
            .backfill_source_passwords()
            .await
            .expect("pending source password backfill should be idempotent"),
        0
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn source_password_backfill_encrypts_legacy_postgres_rows() -> AppResult<()> {
    let Some(raw_url) = std::env::var("SCRYER_TEST_POSTGRES_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    else {
        eprintln!(
            "skipping PostgreSQL source password backfill test; SCRYER_TEST_POSTGRES_URL is not set"
        );
        return Ok(());
    };

    let admin_pool = sqlx::PgPool::connect(&raw_url)
        .await
        .map_err(|error| AppError::Repository(format!("failed to connect to postgres: {error}")))?;
    let schema = format!(
        "scryer_test_{}_{}",
        std::process::id(),
        Id::new().0.replace('-', "_")
    );

    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin_pool)
        .await
        .map_err(|error| AppError::Repository(format!("failed to create schema: {error}")))?;

    let result = async {
        let mut url = url::Url::parse(&raw_url)
            .map_err(|error| AppError::Validation(format!("invalid postgres test URL: {error}")))?;
        url.query_pairs_mut()
            .append_pair("options", &format!("-csearch_path={schema}"));
        let services =
            crate::PostgresServices::new_with_mode(url.to_string(), crate::MigrationMode::Apply)
                .await?;
        services
            .set_encryption_key(crate::encryption::EncryptionKey::from_bytes([19; 32]))
            .await?;

        let now = Utc::now();
        sqlx::query(
            "INSERT INTO release_download_attempts
             (id, title_id, source_hint, source_title, outcome, error_message, source_password,
              attempted_at, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind("legacy-attempt-pg")
        .bind(None::<String>)
        .bind("weaver")
        .bind("Legacy.Release.Pg")
        .bind("success")
        .bind(None::<String>)
        .bind("legacy-pg-secret")
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(services.pool())
        .await
        .map_err(|error| {
            AppError::Repository(format!("failed to insert release attempt: {error}"))
        })?;

        sqlx::query(
            "INSERT INTO pending_releases
             (id, wanted_item_id, title_id, release_title, release_score, added_at, delay_until,
              status, source_password)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind("legacy-pending-pg")
        .bind("wanted-pg")
        .bind("title-pg")
        .bind("Legacy.Pending.Pg")
        .bind(100_i32)
        .bind(now)
        .bind(now)
        .bind("waiting")
        .bind("pending-pg-secret")
        .execute(services.pool())
        .await
        .map_err(|error| {
            AppError::Repository(format!("failed to insert pending release: {error}"))
        })?;

        let release_store =
            ReleaseStore::new(services.datastore(), services.encryption_key_state());
        let pending_store =
            PendingReleaseStore::new(services.datastore(), services.encryption_key_state());

        assert_eq!(release_store.backfill_source_passwords().await?, 1);
        assert_eq!(pending_store.backfill_source_passwords().await?, 1);

        let stored_release_password: String = sqlx::query_scalar(
            "SELECT source_password FROM release_download_attempts WHERE id = $1",
        )
        .bind("legacy-attempt-pg")
        .fetch_one(services.pool())
        .await
        .map_err(|error| {
            AppError::Repository(format!("failed to load release password: {error}"))
        })?;
        let stored_pending_password: String =
            sqlx::query_scalar("SELECT source_password FROM pending_releases WHERE id = $1")
                .bind("legacy-pending-pg")
                .fetch_one(services.pool())
                .await
                .map_err(|error| {
                    AppError::Repository(format!("failed to load pending password: {error}"))
                })?;
        assert!(crate::encryption::is_encrypted(&stored_release_password));
        assert!(crate::encryption::is_encrypted(&stored_pending_password));
        assert_ne!(stored_release_password, "legacy-pg-secret");
        assert_ne!(stored_pending_password, "pending-pg-secret");

        let latest_password = ReleaseAttemptRepository::get_latest_source_password(
            &release_store,
            None,
            Some("weaver"),
            Some("Legacy.Release.Pg"),
        )
        .await?;
        assert_eq!(latest_password.as_deref(), Some("legacy-pg-secret"));

        let pending_release =
            PendingReleaseRepository::get_pending_release(&pending_store, "legacy-pending-pg")
                .await?
                .expect("pending release should exist");
        assert_eq!(
            pending_release.source_password.as_deref(),
            Some("pending-pg-secret")
        );

        services.pool().close().await;
        Ok(())
    }
    .await;

    let cleanup = sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&admin_pool)
        .await;
    admin_pool.close().await;
    cleanup.map_err(|error| AppError::Repository(format!("failed to drop schema: {error}")))?;
    result
}

#[tokio::test]
async fn release_attempt_queries_dedupe_failed_signatures_by_normalized_source_title() {
    let (services, db) = temp_services("scryer_release_dedupe").await;
    let release_store = ReleaseStore::new(services.datastore(), services.encryption_key_state());
    let catalog = title_store(&services);

    catalog
        .create_or_get_existing(make_test_title("title-1", None))
        .await
        .expect("title should insert");

    ReleaseAttemptRepository::record_release_attempt(
        &release_store,
        Some("title-1".to_string()),
        Some("weaver-1".to_string()),
        Some("Pals.S05.720p.BluRay.DD5.1.x264-NTb".to_string()),
        ReleaseDownloadAttemptOutcome::Failed,
        Some("boom-1".to_string()),
        None,
    )
    .await
    .expect("first release attempt should record");
    ReleaseAttemptRepository::record_release_attempt(
        &release_store,
        Some("title-1".to_string()),
        Some("weaver-2".to_string()),
        Some(" pals.s05.720p.bluray.dd5.1.x264-ntb ".to_string()),
        ReleaseDownloadAttemptOutcome::Failed,
        Some("boom-2".to_string()),
        None,
    )
    .await
    .expect("second release attempt should record");

    let failures = ReleaseAttemptRepository::list_failed_release_signatures(&release_store, 10)
        .await
        .expect("failed signatures should list");
    assert_eq!(failures.len(), 1);

    let title_failures = ReleaseAttemptRepository::list_failed_release_signatures_for_title(
        &release_store,
        "title-1",
        10,
    )
    .await
    .expect("title failed signatures should list");
    assert_eq!(title_failures.len(), 1);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn release_attempt_queries_exclude_pending_attempts_from_failed_signatures() {
    let (services, db) = temp_services("scryer_release_pending_excluded").await;
    let release_store = ReleaseStore::new(services.datastore(), services.encryption_key_state());
    let catalog = title_store(&services);

    catalog
        .create_or_get_existing(make_test_title("title-pending", None))
        .await
        .expect("title should insert");

    ReleaseAttemptRepository::record_release_attempt(
        &release_store,
        Some("title-pending".to_string()),
        Some("client-unavailable".to_string()),
        Some("Deferred.Movie.2024.1080p.WEB-DL-GRP".to_string()),
        ReleaseDownloadAttemptOutcome::Pending,
        Some("download client unavailable".to_string()),
        None,
    )
    .await
    .expect("pending release attempt should record");

    let failures = ReleaseAttemptRepository::list_failed_release_signatures(&release_store, 10)
        .await
        .expect("failed signatures should list");
    assert!(failures.is_empty());

    let title_failures = ReleaseAttemptRepository::list_failed_release_signatures_for_title(
        &release_store,
        "title-pending",
        10,
    )
    .await
    .expect("title failed signatures should list");
    assert!(title_failures.is_empty());

    ReleaseAttemptRepository::record_release_attempt(
        &release_store,
        Some("title-pending".to_string()),
        Some("release-rejected".to_string()),
        Some("Rejected.Movie.2024.1080p.WEB-DL-GRP".to_string()),
        ReleaseDownloadAttemptOutcome::Failed,
        Some("release rejected".to_string()),
        None,
    )
    .await
    .expect("failed release attempt should record");

    let failures = ReleaseAttemptRepository::list_failed_release_signatures(&release_store, 10)
        .await
        .expect("failed signatures should list");
    assert_eq!(failures.len(), 1);
    assert_eq!(
        failures[0].source_title.as_deref(),
        Some("Rejected.Movie.2024.1080p.WEB-DL-GRP")
    );

    let _ = std::fs::remove_file(db);
}

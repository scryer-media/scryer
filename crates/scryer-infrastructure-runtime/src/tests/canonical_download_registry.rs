use super::*;
use scryer_application::{
    ClientJobLocator, DownloadRegistryRepository, DownloadSubmissionIdentity,
    DownloadSubmissionPurpose, DownloadSubmissionRepository, ObservationResolution,
    ObservedClientJob, PersistedSeedGoals, SeedGoalResolutionSource,
};

#[derive(Debug, PartialEq, Eq)]
#[expect(dead_code, reason = "retained canonical registry snapshot fixture")]
struct CanonicalSnapshot {
    download_id: String,
    origin: String,
    client_config_id: Option<String>,
    client_type_snapshot: Option<String>,
    client_name_snapshot: Option<String>,
    native_item_id: Option<String>,
    is_ended: bool,
}

fn submission(item_id: &str, title_id: &str) -> DownloadSubmission {
    DownloadSubmission {
        download_id: scryer_domain::download_identity::DownloadId::new(),
        scope: SubmissionScope::Title,
        title_id: title_id.to_string(),
        facet: "series".to_string(),
        download_client_id: Some("client-one".to_string()),
        download_client_type: "qbittorrent".to_string(),
        download_client_item_id: item_id.to_string(),
        source_hint: None,
        source_provider_id: None,
        source_provider_name: None,
        source_kind: None,
        source_title: None,
        info_hash: None,
        release_size_bytes: None,
        request_signature: None,
        purpose: DownloadSubmissionPurpose::Standard,
    }
}

fn seed_goals() -> PersistedSeedGoals {
    PersistedSeedGoals {
        seeding_profile_id: Some("profile-one".to_string()),
        seed_goal_ratio: Some(2.0),
        seed_goal_seconds: Some(7_200),
        never_remove: true,
        goal_met_action: Some(scryer_domain::SeedGoalMetAction::StopSeeding),
        post_import_tracking: scryer_domain::PostImportTracking::Park,
        resolution_source: SeedGoalResolutionSource::Indexer,
        info_hash: Some("ABCDEF0123456789ABCDEF0123456789ABCDEF01".to_string()),
    }
}

async fn assert_accepted_grab_lifecycle(client_type: &str, item_id: &str, wire_token: bool) {
    let db = std::env::temp_dir().join(format!(
        "scryer_accepted_grab_{client_type}_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("database should migrate through the canonical binding schema");
    sqlx::query(
        "INSERT INTO download_clients (
                id, name, client_type, config_json, created_at, updated_at
             ) VALUES ('client-one', 'Primary Client', ?1, '{}', ?2, ?2)",
    )
    .bind(client_type)
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(services.pool())
    .await
    .expect("configured client should insert");

    let submissions = DownloadSubmissionStore::new(services.datastore());
    let registry = DownloadRegistryStore::new(services.datastore());
    let requested_download_id = scryer_domain::download_identity::DownloadId::new();
    let locator = ClientJobLocator::new(Some("client-one"), client_type, item_id);
    let mut accepted = submission(item_id, "title-one");
    accepted.download_id = requested_download_id;
    accepted.download_client_type = client_type.to_string();

    let disposition = submissions
        .record_submission_with_identity(
            accepted,
            DownloadSubmissionIdentity {
                download_id: Some(requested_download_id.to_wire()),
            },
            Some(seed_goals()),
        )
        .await
        .expect("accepted submission and seed goals should persist atomically");
    assert_eq!(
        disposition,
        scryer_application::CanonicalDownloadIdentityDisposition::Requested
    );
    let effective_download_id = requested_download_id;

    let stored = submissions
        .find_by_client_item_id(&locator)
        .await
        .expect("submission lookup should succeed")
        .expect("accepted submission should exist");
    assert_eq!(stored.download_id, effective_download_id);

    let observation = ObservedClientJob {
        locator: locator.clone(),
        wire_token: wire_token.then(|| effective_download_id.to_wire()),
        observed_name: Some("accepted release".to_string()),
        observed_at: chrono::Utc::now(),
    };
    assert_eq!(
        registry
            .resolve_observation(&observation)
            .await
            .expect("first observation should reuse the accepted binding"),
        ObservationResolution::Resolved {
            download_id: effective_download_id,
            newly_foreign: false,
            attached: false,
        }
    );
    assert_eq!(
        registry
            .load_download(&effective_download_id)
            .await
            .expect("accepted parent should load")
            .expect("accepted parent should exist")
            .origin,
        scryer_application::DownloadOrigin::ScryerSubmission
    );
    let binding = registry
        .load_binding(&effective_download_id)
        .await
        .expect("accepted binding should load")
        .expect("accepted binding should exist");
    assert_eq!(
        binding.client_name_snapshot.as_deref(),
        Some("Primary Client")
    );
    assert_eq!(binding.native_item_id.as_deref(), Some(item_id));
    let foreign_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM downloads WHERE origin = 'foreign_observation'")
            .fetch_one(services.pool())
            .await
            .expect("foreign-row count should load");
    assert_eq!(foreign_rows, 0);

    let persisted_goals = submissions
        .get_seed_goals_for_download(Some(&effective_download_id), &locator)
        .await
        .expect("goals should load through the tracked entry id")
        .expect("grab-time goals should remain on the canonical submission");
    assert_eq!(persisted_goals.seed_goal_ratio, Some(2.0));
    assert_eq!(persisted_goals.seed_goal_seconds, Some(7_200));
    assert!(persisted_goals.never_remove);

    submissions
        .update_tracked_state(&locator, "downloading")
        .await
        .expect("tracked state should use the accepted binding");
    assert_eq!(
        submissions
            .get_tracked_state(&locator)
            .await
            .expect("tracked state should load"),
        Some("downloading".to_string())
    );

    drop(services);
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn nzbget_accepted_grabs_keep_the_token_bound_submission_id() {
    assert_accepted_grab_lifecycle("nzbget", "NZBGet-42", true).await;
}

#[tokio::test]
async fn sabnzbd_accepted_grabs_keep_the_nzo_bound_submission_id() {
    assert_accepted_grab_lifecycle("sabnzbd", "SABnzbd_nzo_42", false).await;
}

#[tokio::test]
async fn qbittorrent_accepted_grabs_keep_the_hash_bound_submission_id() {
    assert_accepted_grab_lifecycle(
        "qbittorrent",
        "0123456789abcdef0123456789abcdef01234567",
        false,
    )
    .await;
}

#[tokio::test]
async fn plugin_accepted_grabs_keep_the_native_id_bound_submission_id() {
    assert_accepted_grab_lifecycle("plugin", "plugin-native-42", false).await;
}

#[tokio::test]
async fn accepted_grab_reports_foreign_adoption_without_mutating_the_foreign_job() {
    let db = std::env::temp_dir().join(format!(
        "scryer_accepted_grab_adopts_foreign_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("database should migrate through the canonical binding schema");
    sqlx::query(
        "INSERT INTO download_clients (
            id, name, client_type, config_json, created_at, updated_at
         ) VALUES ('client-one', 'Primary qBittorrent', 'qbittorrent', '{}', ?1, ?1)",
    )
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(services.pool())
    .await
    .expect("configured client should insert");

    let submissions = DownloadSubmissionStore::new(services.datastore());
    let registry = DownloadRegistryStore::new(services.datastore());
    let locator = ClientJobLocator::new(Some("client-one"), "qbittorrent", "reused-hash");
    let ObservationResolution::Resolved {
        download_id: foreign_download_id,
        newly_foreign,
        ..
    } = registry
        .resolve_observation(&ObservedClientJob {
            locator: locator.clone(),
            wire_token: None,
            observed_name: Some("previously foreign torrent".to_string()),
            observed_at: chrono::Utc::now(),
        })
        .await
        .expect("foreign observation should resolve")
    else {
        panic!("foreign observation should produce a resolved identity");
    };
    assert!(newly_foreign);

    let requested_download_id = scryer_domain::download_identity::DownloadId::new();
    let mut accepted = submission("reused-hash", "title-one");
    accepted.download_id = requested_download_id;
    let disposition = submissions
        .record_submission_with_identity(
            accepted,
            DownloadSubmissionIdentity {
                download_id: Some(requested_download_id.to_wire()),
            },
            Some(seed_goals()),
        )
        .await
        .expect("accepted submission should report the adopted canonical id");
    assert_eq!(
        disposition,
        scryer_application::CanonicalDownloadIdentityDisposition::AdoptedExisting {
            download_id: foreign_download_id,
        }
    );

    assert!(
        submissions
            .find_by_client_item_id(&locator)
            .await
            .expect("submission lookup should succeed")
            .is_none()
    );
    assert_eq!(
        registry
            .load_download(&foreign_download_id)
            .await
            .expect("adopted parent should load")
            .expect("adopted parent should exist")
            .origin,
        scryer_application::DownloadOrigin::ForeignObservation
    );
    assert!(
        submissions
            .get_seed_goals_for_download(Some(&foreign_download_id), &locator)
            .await
            .expect("seed-goal lookup should succeed")
            .is_none()
    );
    let binding_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM download_client_bindings
          WHERE client_config_id = 'client-one'
            AND client_type_snapshot = 'qbittorrent'
            AND native_item_id = 'reused-hash'",
    )
    .fetch_one(services.pool())
    .await
    .expect("binding count should load");
    assert_eq!(binding_count, 1);

    drop(services);
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn ambiguous_submissions_attach_only_when_the_observation_is_unambiguous() {
    let db = std::env::temp_dir().join(format!(
        "scryer_ambiguous_download_observation_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("database should migrate through 0179");
    sqlx::query(
        "INSERT INTO download_clients (
            id, name, client_type, config_json, created_at, updated_at
         ) VALUES ('client-one', 'Primary qBittorrent', 'qbittorrent', '{}', ?1, ?1)",
    )
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(services.pool())
    .await
    .expect("configured client should insert");
    let submissions = DownloadSubmissionStore::new(services.datastore());
    let registry = DownloadRegistryStore::new(services.datastore());

    let mut first = submission("unused-first", "title-first");
    first.source_title = Some("Paper Lantern".to_string());
    let first_id = first.download_id;
    submissions
        .record_ambiguous_submission(first)
        .await
        .expect("ambiguous submission should be stored unbound");

    let queue_observation = ObservedClientJob {
        locator: ClientJobLocator::new(Some("client-one"), "qbittorrent", "native-first"),
        wire_token: None,
        observed_name: Some("  paper lantern  ".to_string()),
        observed_at: chrono::Utc::now(),
    };
    assert_eq!(
        registry
            .resolve_observation(&queue_observation)
            .await
            .expect("queue observation should attach"),
        ObservationResolution::Resolved {
            download_id: first_id,
            newly_foreign: false,
            attached: true,
        }
    );
    let completed_observation = ObservedClientJob {
        observed_name: Some("unrelated completed display name".to_string()),
        observed_at: chrono::Utc::now(),
        ..queue_observation.clone()
    };
    assert_eq!(
        registry
            .resolve_observation(&completed_observation)
            .await
            .expect("completed observation should use its locator"),
        ObservationResolution::Resolved {
            download_id: first_id,
            newly_foreign: false,
            attached: false,
        }
    );

    let mut second = submission("unused-second", "title-second");
    second.source_title = Some("Paper Lantern".to_string());
    let second_id = second.download_id;
    submissions
        .record_ambiguous_submission(second)
        .await
        .expect("second ambiguous submission should store");
    let mut third = submission("unused-third", "title-third");
    third.source_title = Some("Paper Lantern".to_string());
    submissions
        .record_ambiguous_submission(third)
        .await
        .expect("third ambiguous submission should store");

    let collision_observation = ObservedClientJob {
        locator: ClientJobLocator::new(Some("client-one"), "qbittorrent", "native-collision"),
        wire_token: None,
        observed_name: Some("paper lantern".to_string()),
        observed_at: chrono::Utc::now(),
    };
    let ObservationResolution::Resolved {
        download_id: collision_id,
        newly_foreign,
        attached,
    } = registry
        .resolve_observation(&collision_observation)
        .await
        .expect("ambiguous name collision should fall through to foreign")
    else {
        panic!("ambiguous name collision should produce a resolved identity");
    };
    assert!(newly_foreign);
    assert!(!attached);
    assert_ne!(collision_id, second_id);

    let token_observation = ObservedClientJob {
        locator: ClientJobLocator::new(Some("client-one"), "qbittorrent", "native-second"),
        wire_token: Some(second_id.to_wire()),
        observed_name: Some("does not need to match".to_string()),
        observed_at: chrono::Utc::now(),
    };
    assert_eq!(
        registry
            .resolve_observation(&token_observation)
            .await
            .expect("token should choose its own unbound submission"),
        ObservationResolution::Resolved {
            download_id: second_id,
            newly_foreign: false,
            attached: true,
        }
    );

    drop(services);
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn readding_a_completed_delete_locator_creates_a_new_canonical_submission() {
    let db = std::env::temp_dir().join(format!(
        "scryer_readded_download_locator_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("database should migrate through 0180");
    sqlx::query(
        "INSERT INTO download_clients (
            id, name, client_type, config_json, created_at, updated_at
         ) VALUES ('client-one', 'Primary qBittorrent', 'qbittorrent', '{}', ?1, ?1)",
    )
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(services.pool())
    .await
    .expect("configured client should insert");

    let submissions = DownloadSubmissionStore::new(services.datastore());
    let registry = DownloadRegistryStore::new(services.datastore());
    let locator = ClientJobLocator::new(Some("client-one"), "qbittorrent", "reused-native-id");
    let first_id =
        scryer_domain::download_identity::DownloadId::parse("00000000-0000-4000-8000-000000000001")
            .expect("first fixed download id should parse");
    let second_id =
        scryer_domain::download_identity::DownloadId::parse("00000000-0000-4000-8000-000000000002")
            .expect("second fixed download id should parse");

    for download_id in [first_id, second_id] {
        let observation = ObservedClientJob {
            locator: locator.clone(),
            wire_token: Some(download_id.to_wire()),
            observed_name: Some("reused torrent".to_string()),
            observed_at: chrono::Utc::now(),
        };
        if download_id == first_id {
            registry
                .resolve_observation(&observation)
                .await
                .expect("first submission binding should resolve");
            let mut first = submission("reused-native-id", "title-first");
            first.download_id = first_id;
            submissions
                .record_submission(first)
                .await
                .expect("first submission should persist");

            // This is the store-level action the completed delete-command path
            // invokes after the client has removed the native item.
            registry
                .end_binding(&first_id)
                .await
                .expect("completed delete should end the first binding");
        } else {
            registry
                .resolve_observation(&observation)
                .await
                .expect("re-added submission binding should resolve");
            let mut second = submission("reused-native-id", "title-second");
            second.download_id = second_id;
            submissions
                .record_submission(second)
                .await
                .expect("re-added submission should persist");
        }
    }

    let submission_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM download_submissions
          WHERE download_client_id = 'client-one'
            AND download_client_type = 'qbittorrent'
            AND download_client_item_id = 'reused-native-id'",
    )
    .fetch_one(services.pool())
    .await
    .expect("coexisting submissions should count");
    assert_eq!(submission_count, 2);

    let bindings: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT download_id, ended_at
           FROM download_client_bindings
          WHERE download_id IN (?1, ?2)
          ORDER BY download_id",
    )
    .bind(first_id.to_string())
    .bind(second_id.to_string())
    .fetch_all(services.pool())
    .await
    .expect("bindings should load");
    assert_eq!(bindings.len(), 2);
    assert_eq!(bindings[0].0, first_id.to_string());
    assert!(bindings[0].1.is_some());
    assert_eq!(bindings[1], (second_id.to_string(), None));

    let projected = submissions
        .list_for_client_items(&[locator])
        .await
        .expect("tuple projection should load");
    assert_eq!(projected.len(), 1);
    assert_eq!(projected[0].download_id, second_id);

    drop(services);
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn active_binding_reconcile_query_is_client_scoped_recency_filtered_and_bounded() {
    let db = std::env::temp_dir().join(format!(
        "scryer_active_binding_reconcile_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("database should migrate through the canonical binding schema");
    let registry = DownloadRegistryStore::new(services.datastore());
    let old = chrono::Utc::now() - chrono::Duration::minutes(11);
    let fresh = chrono::Utc::now();

    let observe = |client_id: &str, item_id: &str| ObservedClientJob {
        locator: ClientJobLocator::new(Some(client_id), "qbittorrent", item_id),
        wire_token: None,
        observed_name: Some(item_id.to_string()),
        observed_at: old,
    };
    let eligible = match registry
        .resolve_observation(&observe("client-one", "eligible-old"))
        .await
        .expect("eligible observation should resolve")
    {
        ObservationResolution::Resolved { download_id, .. } => download_id,
        ObservationResolution::Conflict { .. } => {
            panic!("eligible observation should not conflict")
        }
        ObservationResolution::BindingAlreadyEnded => {
            panic!("eligible observation should not use an ended binding")
        }
    };
    let recently_seen = match registry
        .resolve_observation(&observe("client-one", "recently-seen"))
        .await
        .expect("recent observation should resolve")
    {
        ObservationResolution::Resolved { download_id, .. } => download_id,
        ObservationResolution::Conflict { .. } => panic!("recent observation should not conflict"),
        ObservationResolution::BindingAlreadyEnded => {
            panic!("recent observation should not use an ended binding")
        }
    };
    let other_client = match registry
        .resolve_observation(&observe("client-two", "other-client"))
        .await
        .expect("other-client observation should resolve")
    {
        ObservationResolution::Resolved { download_id, .. } => download_id,
        ObservationResolution::Conflict { .. } => {
            panic!("other-client observation should not conflict")
        }
        ObservationResolution::BindingAlreadyEnded => {
            panic!("other-client observation should not use an ended binding")
        }
    };
    let ended = match registry
        .resolve_observation(&observe("client-one", "already-ended"))
        .await
        .expect("ended observation should resolve")
    {
        ObservationResolution::Resolved { download_id, .. } => download_id,
        ObservationResolution::Conflict { .. } => panic!("ended observation should not conflict"),
        ObservationResolution::BindingAlreadyEnded => {
            panic!("ended observation should resolve before its binding ends")
        }
    };
    registry
        .end_binding(&ended)
        .await
        .expect("binding should end");

    sqlx::query(
        "UPDATE download_client_bindings
         SET created_at = ?1, last_seen_at = ?2
         WHERE download_id = ?3",
    )
    .bind(old.to_rfc3339())
    .bind(old.to_rfc3339())
    .bind(eligible.to_string())
    .execute(services.pool())
    .await
    .expect("eligible binding should age");
    sqlx::query(
        "UPDATE download_client_bindings
         SET created_at = ?1, last_seen_at = ?2
         WHERE download_id = ?3",
    )
    .bind(old.to_rfc3339())
    .bind(fresh.to_rfc3339())
    .bind(recently_seen.to_string())
    .execute(services.pool())
    .await
    .expect("recent binding should update");
    sqlx::query(
        "UPDATE download_client_bindings
         SET created_at = ?1, last_seen_at = ?2
         WHERE download_id = ?3",
    )
    .bind(old.to_rfc3339())
    .bind(old.to_rfc3339())
    .bind(other_client.to_string())
    .execute(services.pool())
    .await
    .expect("other-client binding should age");

    let candidates = registry
        .list_active_bindings_for_client_before(
            "client-one",
            "qBittorrent",
            chrono::Utc::now() - chrono::Duration::minutes(10),
            1,
        )
        .await
        .expect("reconcile candidates should load");
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].download_id, eligible);
    assert_eq!(
        candidates[0].native_item_id.as_deref(),
        Some("eligible-old")
    );

    drop(services);
    let _ = std::fs::remove_file(db);
}

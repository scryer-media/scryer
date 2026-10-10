use scryer_application::{DownloadCleanupClaim, DownloadSubmissionRepository};
use scryer_domain::{TrackedDownloadState, download_identity::DownloadId};

async fn retry_fixture() -> (
    ImportStore,
    super::super::download_submission_store::DownloadSubmissionStore,
    ImportRetryClaim,
    chrono::DateTime<Utc>,
) {
    let store = store().await;
    let StoreDatastore::Sqlite { pool, .. } = &store.datastore else {
        panic!("SQLite fixture")
    };
    sqlx::raw_sql("CREATE TABLE downloads (id TEXT PRIMARY KEY);
        CREATE TABLE download_submissions (id TEXT PRIMARY KEY, download_client_id TEXT, download_client_type TEXT,
            download_client_item_id TEXT, title_id TEXT, facet TEXT, source_title TEXT, tracked_state TEXT);
        CREATE TABLE download_clients (id TEXT PRIMARY KEY, client_type TEXT);
        CREATE TABLE download_client_bindings (download_id TEXT PRIMARY KEY, client_config_id TEXT,
            client_type_snapshot TEXT, native_item_id TEXT, ended_at TEXT);
        CREATE TABLE download_identity_states (id TEXT PRIMARY KEY, identity_key TEXT UNIQUE, canonical_download_id TEXT,
            client_id TEXT, client_type TEXT, download_client_item_id TEXT, tracked_state TEXT, reason TEXT, detail TEXT,
            created_at TEXT, updated_at TEXT);")
        .execute(pool).await.unwrap();
    sqlx::raw_sql(include_str!(
        "../../../../scryer/src/db/migrations/0211_durable_download_cleanup.sql"
    ))
    .execute(pool)
    .await
    .unwrap();
    let download_id = DownloadId::new();
    sqlx::query("INSERT INTO downloads (id) VALUES (?)")
        .bind(download_id.to_string())
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO download_submissions VALUES (?, 'client-1', 'nzbget', 'job-1', 'title-1', 'movie', 'release', 'failed')")
        .bind(download_id.to_string()).execute(pool).await.unwrap();
    let id = store
        .queue_import_request_with_identity_for_download(
            source_identity("job-1"),
            "movie_download".into(),
            "{}".into(),
            None,
            Some(&download_id),
        )
        .await
        .unwrap();
    store
        .update_import_status(
            &id,
            ImportStatus::Skipped,
            Some(r#"{"error_message":"original rejection"}"#.into()),
        )
        .await
        .unwrap();
    let record = store.get_import_by_id(&id).await.unwrap().unwrap();
    let expected = chrono::DateTime::parse_from_rfc3339(&record.updated_at)
        .unwrap()
        .with_timezone(&Utc);
    let claim = ImportRetryClaim {
        download_id,
        import_id: id,
        attempt_id: "attempt-1".into(),
        started_at: Utc::now(),
        source: source_identity("job-1"),
        previous_result_json: record.result_json,
    };
    let cleanup = super::super::download_submission_store::DownloadSubmissionStore::new(
        store.datastore.clone(),
    );
    SqlRuntime::run_in_transaction(&store.datastore, "test_cleanup", move |tx| {
        let id = download_id.to_string();
        Box::pin(async move {
            super::super::download_submission_store::enqueue_cleanup_tx(tx, &id, "failed").await
        })
    })
    .await
    .unwrap();
    (store, cleanup, claim, expected)
}

#[tokio::test]
async fn archive_preservation_hold_survives_completion_and_restart_until_released() {
    let (store, cleanup, claim, _) = retry_fixture().await;
    store
        .set_archive_processing_pending(&claim.import_id, true)
        .await
        .unwrap();
    store
        .update_import_status(
            &claim.import_id,
            ImportStatus::Completed,
            Some(r#"{"decision":"imported"}"#.into()),
        )
        .await
        .unwrap();
    let recovered = ImportStore::new(store.datastore.clone());
    assert!(
        recovered
            .archive_processing_pending_for_download(&claim.download_id)
            .await
            .unwrap()
    );
    assert!(matches!(
        cleanup
            .claim_download_cleanup(&claim.download_id)
            .await
            .unwrap(),
        DownloadCleanupClaim::Deferred
    ));
    recovered
        .set_archive_processing_pending(&claim.import_id, false)
        .await
        .unwrap();
    assert!(
        !recovered
            .archive_processing_pending_for_download(&claim.download_id)
            .await
            .unwrap()
    );
    assert!(matches!(
        cleanup
            .claim_download_cleanup(&claim.download_id)
            .await
            .unwrap(),
        DownloadCleanupClaim::Claimed(_)
    ));
}

#[tokio::test]
async fn archive_preservation_hold_survives_retry_payload_replacement() {
    let (store, cleanup, claim, expected) = retry_fixture().await;
    store
        .set_archive_processing_pending(&claim.import_id, true)
        .await
        .unwrap();
    assert!(
        store
            .claim_import_retry(&claim, expected, "{\"replacement\":true}")
            .await
            .unwrap()
            .is_claimed()
    );
    assert!(
        store
            .archive_processing_pending_for_download(&claim.download_id)
            .await
            .unwrap()
    );
    retry_record_result(&store, &claim).await;
    assert!(
        store
            .finish_import_retry(&claim, TrackedDownloadState::ImportBlocked, None, None)
            .await
            .unwrap()
            .is_finalized()
    );
    assert!(matches!(
        cleanup
            .claim_download_cleanup(&claim.download_id)
            .await
            .unwrap(),
        DownloadCleanupClaim::Deferred
    ));
}

#[tokio::test]
async fn archive_preservation_refuses_active_cleanup_but_accepts_retained_attempts() {
    let (store, cleanup, claim, _) = retry_fixture().await;
    assert!(matches!(
        cleanup
            .claim_download_cleanup(&claim.download_id)
            .await
            .unwrap(),
        DownloadCleanupClaim::Claimed(_)
    ));
    assert!(
        store
            .set_archive_processing_pending(&claim.import_id, true)
            .await
            .is_err()
    );
    cleanup
        .finish_download_cleanup(&claim.download_id, "policy_retained", true, 0, 0, None)
        .await
        .unwrap();
    store
        .set_archive_processing_pending(&claim.import_id, true)
        .await
        .unwrap();
    assert!(
        store
            .archive_processing_pending_for_download(&claim.download_id)
            .await
            .unwrap()
    );
}

async fn retry_record_result(store: &ImportStore, claim: &ImportRetryClaim) {
    store
        .update_import_status(
            &claim.import_id,
            ImportStatus::Completed,
            Some(r#"{"decision":"imported"}"#.into()),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn import_retry_claim_defers_cleanup_and_preserves_blocked_source_checkpoint() {
    let (store, cleanup, claim, expected) = retry_fixture().await;
    cleanup
        .checkpoint_download_cleanup_payload(&claim.download_id, "payload-retained")
        .await
        .unwrap();
    assert!(
        store
            .claim_import_retry(&claim, expected, "{}")
            .await
            .unwrap()
            .is_claimed()
    );
    assert!(matches!(
        cleanup
            .claim_download_cleanup(&claim.download_id)
            .await
            .unwrap(),
        DownloadCleanupClaim::Deferred
    ));
    retry_record_result(&store, &claim).await;
    assert!(
        store
            .finish_import_retry(
                &claim,
                TrackedDownloadState::ImportBlocked,
                Some("after_import"),
                Some("quality hold")
            )
            .await
            .unwrap()
            .is_finalized()
    );
    assert!(matches!(
        cleanup
            .claim_download_cleanup(&claim.download_id)
            .await
            .unwrap(),
        DownloadCleanupClaim::Deferred
    ));
    let StoreDatastore::Sqlite { pool, .. } = &store.datastore else {
        unreachable!()
    };
    let row: (String, String) =
        sqlx::query_as("SELECT payload_checkpoint, status FROM download_cleanup")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(row, ("payload-retained".into(), "pending".into()));
    let record = store
        .get_import_by_id(&claim.import_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.status, ImportStatus::Skipped);
    assert!(record.result_json.unwrap().contains("original rejection"));
}

#[tokio::test]
async fn import_retry_cleanup_first_keeps_failed_removal_and_rejects_claim() {
    let (store, cleanup, claim, expected) = retry_fixture().await;
    assert!(matches!(
        cleanup
            .claim_download_cleanup(&claim.download_id)
            .await
            .unwrap(),
        DownloadCleanupClaim::Claimed(_)
    ));
    assert!(
        !store
            .claim_import_retry(&claim, expected, "{}")
            .await
            .unwrap()
            .is_claimed()
    );
    assert_eq!(
        store
            .get_import_by_id(&claim.import_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        ImportStatus::Skipped
    );
    // An expired lease is not evidence that its worker stopped touching files.
    let StoreDatastore::Sqlite { pool, .. } = &store.datastore else {
        unreachable!()
    };
    sqlx::query("UPDATE download_cleanup SET lease_until = '2000-01-01T00:00:00Z'")
        .execute(pool)
        .await
        .unwrap();
    assert!(
        !store
            .claim_import_retry(&claim, expected, "{}")
            .await
            .unwrap()
            .is_claimed()
    );
}

#[tokio::test]
async fn import_retry_concurrent_claims_have_one_owner_and_fence_obsolete_results() {
    let (store, _, claim, expected) = retry_fixture().await;
    let mut second = claim.clone();
    second.attempt_id = "attempt-2".into();
    let (first, next) = tokio::join!(
        store.claim_import_retry(&claim, expected, "{}"),
        store.claim_import_retry(&second, expected, "{}")
    );
    assert_ne!(first.unwrap(), next.unwrap());
    let owner = store
        .get_import_retry_claim(&claim.download_id)
        .await
        .unwrap()
        .unwrap();
    let stale = if owner.attempt_id == claim.attempt_id {
        &second
    } else {
        &claim
    };
    assert!(
        !store
            .finish_import_retry(stale, TrackedDownloadState::Imported, None, None)
            .await
            .unwrap()
            .is_finalized()
    );
    retry_record_result(&store, &owner).await;
    assert!(
        store
            .finish_import_retry(&owner, TrackedDownloadState::Imported, None, None)
            .await
            .unwrap()
            .is_finalized()
    );
    assert!(
        store
            .get_import_retry_claim(&claim.download_id)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn import_retry_finalization_failure_retains_marker_across_repository_restart() {
    let (store, cleanup, claim, expected) = retry_fixture().await;
    assert!(
        store
            .claim_import_retry(&claim, expected, "{\"resolved_title\":\"current\"}")
            .await
            .unwrap()
            .is_claimed()
    );
    retry_record_result(&store, &claim).await;
    let StoreDatastore::Sqlite { pool, .. } = &store.datastore else {
        unreachable!()
    };
    sqlx::raw_sql(
        "CREATE TRIGGER fail_retry_finish BEFORE UPDATE ON download_identity_states
        WHEN NEW.reason IS NULL BEGIN SELECT RAISE(ABORT, 'injected finalization failure'); END;",
    )
    .execute(pool)
    .await
    .unwrap();
    assert!(
        store
            .finish_import_retry(&claim, TrackedDownloadState::Imported, None, None)
            .await
            .is_err()
    );
    assert!(matches!(
        cleanup
            .claim_download_cleanup(&claim.download_id)
            .await
            .unwrap(),
        DownloadCleanupClaim::Deferred
    ));
    let restarted = ImportStore::new(store.datastore.clone());
    let pending = restarted
        .list_import_retry_recovery(None, 25)
        .await
        .unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].attempt_id, claim.attempt_id);
    assert!(
        restarted
            .list_import_retry_recovery(Some(&claim.download_id), 25)
            .await
            .unwrap()
            .is_empty()
    );
    sqlx::raw_sql("DROP TRIGGER fail_retry_finish")
        .execute(pool)
        .await
        .unwrap();
    assert!(
        restarted
            .finish_import_retry(&claim, TrackedDownloadState::Imported, None, None)
            .await
            .unwrap()
            .is_finalized()
    );
    assert!(
        matches!(cleanup.claim_download_cleanup(&claim.download_id).await.unwrap(), DownloadCleanupClaim::Claimed(record) if record.tracked_state == "imported")
    );
}

#[tokio::test]
async fn import_retry_rejects_reused_native_id_and_fences_tracker_writes() {
    let (store, submissions, claim, expected) = retry_fixture().await;
    let StoreDatastore::Sqlite { pool, .. } = &store.datastore else {
        unreachable!()
    };
    sqlx::query(
        "INSERT INTO download_client_bindings VALUES (?, 'client-1', 'nzbget', 'job-1', NULL)",
    )
    .bind(DownloadId::new().to_string())
    .execute(pool)
    .await
    .unwrap();
    assert!(
        !store
            .claim_import_retry(&claim, expected, "{}")
            .await
            .unwrap()
            .is_claimed()
    );
    sqlx::query("UPDATE download_client_bindings SET ended_at = '2000-01-01T00:00:00Z'")
        .execute(pool)
        .await
        .unwrap();
    assert!(
        store
            .claim_import_retry(&claim, expected, "{}")
            .await
            .unwrap()
            .is_claimed()
    );
    assert!(
        submissions
            .record_identity_tracked_state_for_download(
                Some(&claim.download_id),
                &scryer_application::DownloadSubmissionIdentity::default(),
                Some(&claim.source),
                "failed",
                Some("import_gate_rejected"),
                None
            )
            .await
            .is_err()
    );
    assert_eq!(
        store
            .get_import_retry_claim(&claim.download_id)
            .await
            .unwrap()
            .unwrap()
            .attempt_id,
        claim.attempt_id
    );
}

#[tokio::test]
async fn import_retry_claim_failure_rolls_back_processing_and_preserves_rejection() {
    let (store, _, claim, expected) = retry_fixture().await;
    let StoreDatastore::Sqlite { pool, .. } = &store.datastore else {
        unreachable!()
    };
    sqlx::raw_sql(
        "CREATE TRIGGER fail_retry_claim BEFORE INSERT ON download_identity_states
        BEGIN SELECT RAISE(ABORT, 'injected claim failure'); END;",
    )
    .execute(pool)
    .await
    .unwrap();
    assert!(
        store
            .claim_import_retry(&claim, expected, "{}")
            .await
            .is_err()
    );
    let record = store
        .get_import_by_id(&claim.import_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.status, ImportStatus::Skipped);
    assert!(record.result_json.unwrap().contains("original rejection"));
    assert!(
        store
            .list_import_retry_recovery(None, 25)
            .await
            .unwrap()
            .is_empty()
    );
}

async fn second_import(store: &ImportStore, claim: &ImportRetryClaim, import_type: &str) -> String {
    store
        .queue_import_request_with_identity_for_download(
            claim.source.clone(),
            import_type.into(),
            "{}".into(),
            None,
            Some(&claim.download_id),
        )
        .await
        .unwrap()
}

async fn complete_held(store: &ImportStore, import_id: &str, reason: &str) {
    store
        .set_archive_processing_pending(import_id, true)
        .await
        .unwrap();
    store
        .record_archive_hold_reason(import_id, reason)
        .await
        .unwrap();
    store
        .update_import_status(
            import_id,
            ImportStatus::Completed,
            Some(r#"{"decision":"imported"}"#.into()),
        )
        .await
        .unwrap();
}

async fn payload_of(store: &ImportStore, import_id: &str) -> serde_json::Value {
    let record = store.get_import_by_id(import_id).await.unwrap().unwrap();
    serde_json::from_str(&record.payload_json).unwrap()
}

#[tokio::test]
async fn hold_reason_only_annotates_an_existing_hold() {
    let (store, _cleanup, claim, _) = retry_fixture().await;
    store
        .record_archive_hold_reason(&claim.import_id, "subtitles_pending")
        .await
        .unwrap();
    assert!(
        payload_of(&store, &claim.import_id).await
            [scryer_application::ARCHIVE_HOLD_REASON_PAYLOAD_KEY]
            .is_null()
    );
    complete_held(&store, &claim.import_id, "archive_extraction_failed").await;
    assert_eq!(
        payload_of(&store, &claim.import_id).await
            [scryer_application::ARCHIVE_HOLD_REASON_PAYLOAD_KEY],
        "archive_extraction_failed"
    );
}

async fn hold_reason(store: &ImportStore, import_id: &str) -> Option<String> {
    payload_of(store, import_id).await[scryer_application::ARCHIVE_HOLD_REASON_PAYLOAD_KEY]
        .as_str()
        .map(str::to_string)
}

#[tokio::test]
async fn hold_reason_keeps_the_most_severe_reason_recorded() {
    let (store, _cleanup, claim, _) = retry_fixture().await;
    store
        .set_archive_processing_pending(&claim.import_id, true)
        .await
        .unwrap();
    store
        .record_archive_hold_reason(&claim.import_id, "subtitles_pending")
        .await
        .unwrap();
    store
        .record_archive_hold_reason(&claim.import_id, "archive_extraction_failed")
        .await
        .unwrap();
    assert_eq!(
        hold_reason(&store, &claim.import_id).await.as_deref(),
        Some("archive_extraction_failed")
    );
    // A milder reason recorded later never downgrades the hold.
    store
        .record_archive_hold_reason(&claim.import_id, "subtitles_pending")
        .await
        .unwrap();
    store
        .record_archive_hold_reason(&claim.import_id, "source_cleanup_incomplete")
        .await
        .unwrap();
    assert_eq!(
        hold_reason(&store, &claim.import_id).await.as_deref(),
        Some("archive_extraction_failed")
    );
}

#[tokio::test]
async fn import_retry_carries_the_hold_reason_forward() {
    let (store, _cleanup, claim, expected) = retry_fixture().await;
    store
        .set_archive_processing_pending(&claim.import_id, true)
        .await
        .unwrap();
    store
        .record_archive_hold_reason(&claim.import_id, "archive_extraction_failed")
        .await
        .unwrap();
    let expected = {
        let record = store
            .get_import_by_id(&claim.import_id)
            .await
            .unwrap()
            .unwrap();
        chrono::DateTime::parse_from_rfc3339(&record.updated_at)
            .map(|updated| updated.with_timezone(&Utc))
            .unwrap_or(expected)
    };

    assert!(
        store
            .claim_import_retry(&claim, expected, "{\"replacement\":true}")
            .await
            .unwrap()
            .is_claimed()
    );

    let payload = payload_of(&store, &claim.import_id).await;
    assert_eq!(payload["replacement"], true);
    assert_eq!(payload["archive_processing_pending"], true);
    assert_eq!(
        payload[scryer_application::ARCHIVE_HOLD_REASON_PAYLOAD_KEY],
        "archive_extraction_failed"
    );
}

#[tokio::test]
async fn hold_release_without_a_canonical_download_is_refused_and_changes_nothing() {
    let (store, cleanup, claim, _) = retry_fixture().await;
    complete_held(&store, &claim.import_id, "subtitles_pending").await;

    let error = store
        .release_archive_holds(&claim.source, None, &[claim.import_id.clone()])
        .await
        .expect_err("nothing serializes the release against the cleanup claim");

    assert!(matches!(error, AppError::Validation(_)), "{error:?}");
    let payload = payload_of(&store, &claim.import_id).await;
    assert_eq!(payload["archive_processing_pending"], true);
    assert_eq!(
        payload[scryer_application::ARCHIVE_HOLD_REASON_PAYLOAD_KEY],
        "subtitles_pending"
    );
    assert!(matches!(
        cleanup
            .claim_download_cleanup(&claim.download_id)
            .await
            .unwrap(),
        DownloadCleanupClaim::Deferred
    ));
}

#[tokio::test]
async fn released_holds_clear_together_and_let_the_cleanup_claim_proceed() {
    let (store, cleanup, claim, _) = retry_fixture().await;
    let other = second_import(&store, &claim, "manual_import").await;
    complete_held(&store, &claim.import_id, "subtitles_pending").await;
    complete_held(&store, &other, "source_cleanup_incomplete").await;
    assert!(matches!(
        cleanup
            .claim_download_cleanup(&claim.download_id)
            .await
            .unwrap(),
        DownloadCleanupClaim::Deferred
    ));
    store
        .release_archive_holds(
            &claim.source,
            Some(&claim.download_id),
            &[claim.import_id.clone(), other.clone()],
        )
        .await
        .unwrap();
    for id in [&claim.import_id, &other] {
        let payload = payload_of(&store, id).await;
        assert!(payload["archive_processing_pending"].is_null());
        assert!(payload[scryer_application::ARCHIVE_HOLD_REASON_PAYLOAD_KEY].is_null());
        assert_eq!(
            payload[scryer_application::ARCHIVE_HOLD_RELEASED_PAYLOAD_KEY],
            true
        );
    }
    // Releasing again is a no-op, so an interrupted release can resume.
    store
        .release_archive_holds(
            &claim.source,
            Some(&claim.download_id),
            &[claim.import_id.clone(), other.clone()],
        )
        .await
        .unwrap();
    assert!(matches!(
        cleanup
            .claim_download_cleanup(&claim.download_id)
            .await
            .unwrap(),
        DownloadCleanupClaim::Claimed(_)
    ));
}

#[tokio::test]
async fn hold_release_refused_for_an_unlisted_held_import_changes_nothing() {
    let (store, cleanup, claim, _) = retry_fixture().await;
    let other = second_import(&store, &claim, "manual_import").await;
    complete_held(&store, &claim.import_id, "subtitles_pending").await;
    complete_held(&store, &other, "archive_extraction_failed").await;
    assert!(
        store
            .release_archive_holds(
                &claim.source,
                Some(&claim.download_id),
                &[claim.import_id.clone()]
            )
            .await
            .is_err()
    );
    for id in [&claim.import_id, &other] {
        assert_eq!(
            payload_of(&store, id).await["archive_processing_pending"],
            true
        );
    }
    assert!(matches!(
        cleanup
            .claim_download_cleanup(&claim.download_id)
            .await
            .unwrap(),
        DownloadCleanupClaim::Deferred
    ));
}

#[tokio::test]
async fn hold_release_refused_while_an_import_of_the_download_is_pending() {
    let (store, cleanup, claim, _) = retry_fixture().await;
    complete_held(&store, &claim.import_id, "subtitles_pending").await;
    let _pending = second_import(&store, &claim, "manual_import").await;
    assert!(
        store
            .release_archive_holds(
                &claim.source,
                Some(&claim.download_id),
                &[claim.import_id.clone()]
            )
            .await
            .is_err()
    );
    assert_eq!(
        payload_of(&store, &claim.import_id).await["archive_processing_pending"],
        true
    );
    assert!(matches!(
        cleanup
            .claim_download_cleanup(&claim.download_id)
            .await
            .unwrap(),
        DownloadCleanupClaim::Deferred
    ));
}

#[tokio::test]
async fn cleanup_claim_waits_for_a_queued_manual_import_of_the_download() {
    let (store, cleanup, claim, _) = retry_fixture().await;
    let manual = second_import(&store, &claim, "manual_import").await;
    assert!(matches!(
        cleanup
            .claim_download_cleanup(&claim.download_id)
            .await
            .unwrap(),
        DownloadCleanupClaim::Deferred
    ));
    store
        .update_import_status(
            &manual,
            ImportStatus::Completed,
            Some(r#"{"decision":"imported"}"#.into()),
        )
        .await
        .unwrap();
    assert!(matches!(
        cleanup
            .claim_download_cleanup(&claim.download_id)
            .await
            .unwrap(),
        DownloadCleanupClaim::Claimed(_)
    ));
}

#[tokio::test]
async fn open_manual_selection_roots_lists_only_unconsumed_selections() {
    let store = store().await;
    for (id, root, workspace, consumed) in [
        (
            "open",
            "/synthetic/open-root",
            Some("/synthetic/open-workspace"),
            None,
        ),
        ("loose", "/synthetic/loose-root", None, None),
        (
            "consumed",
            "/synthetic/consumed-root",
            Some("/synthetic/consumed-workspace"),
            Some(Utc::now()),
        ),
    ] {
        SqlRuntime::execute(
            store.datastore.read_exec(),
            "INSERT INTO manual_import_selections (id, actor_user_id, title_id, source_client_id, source_system,
               source_ref, trusted_source_root, archive_workspace_root, consumed_at, created_at, updated_at)
             VALUES ({}, 'user-1', 'title-1', 'client-1', 'nzbget', 'job-1', {}, {}, {}, {}, {})",
            &[
                SqlArg::Text(id.into()),
                SqlArg::Text(root.into()),
                SqlArg::OptText(workspace.map(str::to_string)),
                SqlArg::OptTimestamp(consumed),
                SqlArg::Timestamp(Utc::now()),
                SqlArg::Timestamp(Utc::now()),
            ],
        )
        .await
        .unwrap();
    }
    let mut roots = store.open_manual_selection_roots().await.unwrap();
    roots.sort();
    assert_eq!(
        roots,
        [
            "/synthetic/loose-root",
            "/synthetic/open-root",
            "/synthetic/open-workspace"
        ]
    );
}

#[tokio::test]
async fn nzbget_password_holds_block_actual_cleanup_claim_after_store_restart() {
    for reason in [
        scryer_application::DOWNLOAD_PASSWORD_REQUIRED_REASON,
        scryer_application::DOWNLOAD_PASSWORD_RETRY_REASON,
        scryer_application::DOWNLOAD_PASSWORD_AMBIGUOUS_REASON,
    ] {
        let (store, _cleanup, claim, _) = retry_fixture().await;
        let StoreDatastore::Sqlite { pool, .. } = &store.datastore else {
            panic!("SQLite fixture")
        };
        sqlx::query("INSERT INTO download_identity_states (id, identity_key, canonical_download_id, tracked_state, reason) VALUES ('hold', 'hold', ?, 'failed', ?)")
            .bind(claim.download_id.to_string()).bind(reason).execute(pool).await.unwrap();
        for _ in 0..2 {
            let recovered = super::super::download_submission_store::DownloadSubmissionStore::new(
                store.datastore.clone(),
            );
            assert!(
                matches!(
                    recovered
                        .claim_download_cleanup(&claim.download_id)
                        .await
                        .unwrap(),
                    DownloadCleanupClaim::Deferred
                ),
                "{reason}"
            );
        }
        let attempts: i64 =
            sqlx::query_scalar("SELECT attempts FROM download_cleanup WHERE download_id = ?")
                .bind(claim.download_id.to_string())
                .fetch_one(pool)
                .await
                .unwrap();
        assert_eq!(
            attempts, 0,
            "a password hold must not spend cleanup attempts"
        );
    }
}

use super::*;

#[tokio::test]
async fn list_imports_for_identities_handles_multiple_pairs() {
    let db = std::env::temp_dir().join(format!(
        "scryer_import_sources_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let workflow = ImportStore::new(services.datastore());

    workflow
        .queue_import_request(
            ClientJobLocator::new(Some("client-a"), "weaver", "10000"),
            ImportType::ManualImport.as_str().to_string(),
            "{}".to_string(),
        )
        .await
        .expect("first import should queue");
    workflow
        .queue_import_request(
            ClientJobLocator::new(Some("client-b"), "weaver", "10001"),
            ImportType::ManualImport.as_str().to_string(),
            "{}".to_string(),
        )
        .await
        .expect("second import should queue");

    let records = workflow
        .list_imports_for_identities(&[
            ClientJobLocator::new(Some("client-a"), "weaver", "10000"),
            ClientJobLocator::new(Some("client-b"), "weaver", "10001"),
        ])
        .await
        .expect("batch lookup should succeed");

    assert_eq!(records.len(), 2);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn queue_import_request_reuses_existing_row_for_same_identity() {
    let (pool, workflow) = import_store_test_harness(1).await;
    let identity = ClientJobLocator::new(Some("client-a"), "weaver", "10000");

    let first_id = workflow
        .queue_import_request(
            identity.clone(),
            ImportType::ManualImport.as_str().to_string(),
            "{\"attempt\":1}".to_string(),
        )
        .await
        .expect("first import should queue");
    workflow
        .update_import_status(
            &first_id,
            ImportStatus::Completed,
            Some("{\"result\":\"done\"}".to_string()),
        )
        .await
        .expect("import status should update");

    let second_id = workflow
        .queue_import_request(
            identity,
            ImportType::ManualImport.as_str().to_string(),
            "{\"attempt\":2}".to_string(),
        )
        .await
        .expect("second import should requeue");

    assert_eq!(second_id, first_id);

    let record = workflow
        .get_import_by_id(&second_id)
        .await
        .expect("import lookup should succeed")
        .expect("import should exist");
    assert_eq!(record.status, ImportStatus::Pending);
    assert_eq!(record.payload_json, "{\"attempt\":2}");
    assert_eq!(record.result_json, None);

    let row_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM imports
         WHERE COALESCE(source_client_id, '') = ?
           AND source_system = ?
           AND source_ref = ?
           AND import_type = ?",
    )
    .bind("client-a")
    .bind("weaver")
    .bind("10000")
    .bind(ImportType::ManualImport.as_str())
    .fetch_one(&pool)
    .await
    .expect("import count should load");
    assert_eq!(row_count, 1);
}

#[tokio::test]
async fn queue_import_request_with_download_id_reuses_active_row_only() {
    let (pool, workflow) = import_store_test_harness(1).await;
    let download_identity = DownloadSubmissionIdentity {
        download_id: Some("scryer-download:active-dedupe".to_string()),
    };

    let first_id = workflow
        .queue_import_request_with_identity(
            ClientJobLocator::new(Some("client-a"), "weaver", "job-a"),
            ImportType::MovieDownload.as_str().to_string(),
            "{\"attempt\":1}".to_string(),
            Some(download_identity.clone()),
        )
        .await
        .expect("first durable import should queue");
    let second_id = workflow
        .queue_import_request_with_identity(
            ClientJobLocator::new(Some("client-a"), "weaver", "job-b"),
            ImportType::SeriesDownload.as_str().to_string(),
            "{\"attempt\":2}".to_string(),
            Some(download_identity.clone()),
        )
        .await
        .expect("active duplicate durable import should reuse existing row");

    assert_eq!(second_id, first_id);
    let active_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM imports
         WHERE COALESCE(source_client_id, '') = 'client-a'
           AND source_system = 'weaver'
           AND download_id = 'scryer-download:active-dedupe'
           AND status IN ('pending', 'running', 'processing')",
    )
    .fetch_one(&pool)
    .await
    .expect("active import count should load");
    assert_eq!(active_count, 1);

    workflow
        .update_import_status(&first_id, ImportStatus::Completed, None)
        .await
        .expect("first durable import should complete");
    let third_id = workflow
        .queue_import_request_with_identity(
            ClientJobLocator::new(Some("client-a"), "weaver", "job-c"),
            ImportType::MovieDownload.as_str().to_string(),
            "{\"attempt\":3}".to_string(),
            Some(download_identity),
        )
        .await
        .expect("completed durable import should not block a new active row");

    assert_ne!(third_id, first_id);
    let total_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM imports
         WHERE COALESCE(source_client_id, '') = 'client-a'
           AND source_system = 'weaver'
           AND download_id = 'scryer-download:active-dedupe'",
    )
    .fetch_one(&pool)
    .await
    .expect("total import count should load");
    assert_eq!(total_count, 2);
}

#[tokio::test]
async fn queue_import_request_with_download_id_scopes_active_rows_by_client_and_source() {
    let (pool, workflow) = import_store_test_harness(1).await;
    let download_identity = DownloadSubmissionIdentity {
        download_id: Some("scryer-download:scoped-active".to_string()),
    };

    let client_a_id = workflow
        .queue_import_request_with_identity(
            ClientJobLocator::new(Some("client-a"), "weaver", "job-a"),
            ImportType::MovieDownload.as_str().to_string(),
            "{}".to_string(),
            Some(download_identity.clone()),
        )
        .await
        .expect("client-a import should queue");
    let client_b_id = workflow
        .queue_import_request_with_identity(
            ClientJobLocator::new(Some("client-b"), "weaver", "job-b"),
            ImportType::MovieDownload.as_str().to_string(),
            "{}".to_string(),
            Some(download_identity.clone()),
        )
        .await
        .expect("client-b import should queue");
    let other_source_id = workflow
        .queue_import_request_with_identity(
            ClientJobLocator::new(Some("client-a"), "sabnzbd", "job-c"),
            ImportType::MovieDownload.as_str().to_string(),
            "{}".to_string(),
            Some(download_identity),
        )
        .await
        .expect("other source import should queue");

    assert_ne!(client_a_id, client_b_id);
    assert_ne!(client_a_id, other_source_id);
    assert_ne!(client_b_id, other_source_id);

    let active_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM imports
         WHERE download_id = 'scryer-download:scoped-active'
           AND status IN ('pending', 'running', 'processing')",
    )
    .fetch_one(&pool)
    .await
    .expect("active import count should load");
    assert_eq!(active_count, 3);
}

#[tokio::test]
async fn stale_processing_recovery_respects_transfer_progress_heartbeat() {
    let (pool, workflow) = import_store_test_harness(1).await;
    let threshold_seconds = 45 * 60;

    let stale_id = workflow
        .queue_import_request(
            ClientJobLocator::new(Some("client-a"), "weaver", "stale-job"),
            ImportType::MovieDownload.as_str().to_string(),
            "{}".to_string(),
        )
        .await
        .expect("stale import should queue");
    workflow
        .update_import_status(&stale_id, ImportStatus::Processing, None)
        .await
        .expect("stale import should start processing");

    let heartbeat_id = workflow
        .queue_import_request(
            ClientJobLocator::new(Some("client-a"), "weaver", "heartbeat-job"),
            ImportType::MovieDownload.as_str().to_string(),
            "{}".to_string(),
        )
        .await
        .expect("heartbeat import should queue");
    workflow
        .update_import_status(&heartbeat_id, ImportStatus::Processing, None)
        .await
        .expect("heartbeat import should start processing");

    let old_timestamp =
        (Utc::now() - chrono::Duration::seconds(threshold_seconds + 60)).to_rfc3339();
    sqlx::query("UPDATE imports SET updated_at = ? WHERE id IN (?, ?)")
        .bind(&old_timestamp)
        .bind(&stale_id)
        .bind(&heartbeat_id)
        .execute(&pool)
        .await
        .expect("imports should be aged");

    workflow
        .update_import_transfer_progress(
            &heartbeat_id,
            scryer_domain::ImportTransferPhase::Copying,
            1024,
            4096,
        )
        .await
        .expect("heartbeat progress should update import freshness");

    let recovered = workflow
        .recover_stale_processing_imports(threshold_seconds)
        .await
        .expect("stale processing recovery should run");
    assert_eq!(recovered, 1);

    let stale = workflow
        .get_import_by_id(&stale_id)
        .await
        .expect("stale import should load")
        .expect("stale import should exist");
    let heartbeat = workflow
        .get_import_by_id(&heartbeat_id)
        .await
        .expect("heartbeat import should load")
        .expect("heartbeat import should exist");

    assert_eq!(stale.status, ImportStatus::Failed);
    assert_eq!(heartbeat.status, ImportStatus::Processing);
}

#[tokio::test]
async fn active_download_identity_unique_index_blocks_duplicate_active_rows() {
    let (pool, _) = import_store_test_harness(1).await;
    let now = Utc::now().to_rfc3339();
    let insert_sql = "INSERT INTO imports
        (id, source_client_id, source_system, source_ref, import_type, status, payload_json, download_id, created_at, updated_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)";

    sqlx::query(insert_sql)
        .bind("active-index-first")
        .bind("client-a")
        .bind("weaver")
        .bind("job-a")
        .bind(ImportType::MovieDownload.as_str())
        .bind(ImportStatus::Pending.as_str())
        .bind("{}")
        .bind("scryer-download:index-guard")
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("first active import should insert");

    let duplicate = sqlx::query(insert_sql)
        .bind("active-index-second")
        .bind("client-a")
        .bind("weaver")
        .bind("job-b")
        .bind(ImportType::SeriesDownload.as_str())
        .bind(ImportStatus::Running.as_str())
        .bind("{}")
        .bind("scryer-download:index-guard")
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await;
    assert!(duplicate.is_err());

    sqlx::query("UPDATE imports SET status = ? WHERE id = ?")
        .bind(ImportStatus::Completed.as_str())
        .bind("active-index-first")
        .execute(&pool)
        .await
        .expect("first active import should complete");

    sqlx::query(insert_sql)
        .bind("active-index-second")
        .bind("client-a")
        .bind("weaver")
        .bind("job-b")
        .bind(ImportType::SeriesDownload.as_str())
        .bind(ImportStatus::Pending.as_str())
        .bind("{}")
        .bind("scryer-download:index-guard")
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("completed import should not block a new active row");
}

#[test]
fn download_submission_lookup_chunks_and_deduplicates_client_items() {
    let mut client_items = (0..805)
        .map(|idx| ClientJobLocator::new(None, "weaver", format!("job-{idx}")))
        .collect::<Vec<_>>();
    client_items.push(ClientJobLocator::new(None, "weaver", "job-12"));
    client_items.push(ClientJobLocator::new(None, "weaver", "job-400"));

    let chunks = crate::workflow_store::chunk_download_submission_client_items(&client_items);

    assert_eq!(chunks.len(), 3);
    assert_eq!(chunks[0].len(), 400);
    assert_eq!(chunks[1].len(), 400);
    assert_eq!(chunks[2].len(), 5);
    assert_eq!(
        chunks[0][12],
        ClientJobLocator::new(None, "weaver", "job-12")
    );
    assert_eq!(
        chunks
            .iter()
            .flat_map(|chunk| chunk.iter())
            .filter(|identity| identity.client_type == "weaver" && identity.item_id == "job-12")
            .count(),
        1
    );
}

#[tokio::test]
async fn list_download_submissions_for_client_items_handles_large_batched_lookup() {
    let db = std::env::temp_dir().join(format!(
        "scryer_download_submission_sources_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let workflow = DownloadSubmissionStore::new(services.datastore());

    for idx in 0..805 {
        workflow
            .record_submission(DownloadSubmission {
                download_id: scryer_domain::download_identity::DownloadId::new(),
                title_id: format!("title-{idx}"),
                purpose: scryer_application::DownloadSubmissionPurpose::Standard,
                facet: "movie".to_string(),
                download_client_id: None,
                download_client_type: "weaver".to_string(),
                download_client_item_id: format!("job-{idx}"),
                source_hint: None,
                source_provider_id: None,
                source_provider_name: None,
                source_kind: None,
                source_title: Some(format!("Release {idx}")),
                info_hash: None,
                release_size_bytes: None,
                request_signature: None,
                scope: SubmissionScope::Title,
            })
            .await
            .expect("record submission should succeed");
    }

    let mut lookup = (0..805)
        .map(|idx| ClientJobLocator::new(None, "weaver", format!("job-{idx}")))
        .collect::<Vec<_>>();
    lookup.push(ClientJobLocator::new(None, "weaver", "job-12"));
    lookup.push(ClientJobLocator::new(None, "weaver", "job-400"));

    let records = workflow
        .list_for_client_items(&lookup)
        .await
        .expect("batched lookup should succeed");

    assert_eq!(records.len(), 805);
    assert!(records.iter().any(|record| {
        record.download_client_type == "weaver" && record.download_client_item_id == "job-804"
    }));

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn download_submission_identity_does_not_fall_back_to_legacy_rows() {
    let db = std::env::temp_dir().join(format!(
        "scryer_download_submission_identity_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let workflow = DownloadSubmissionStore::new(services.datastore());

    workflow
        .record_submission(DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: "legacy-title".to_string(),
            purpose: scryer_application::DownloadSubmissionPurpose::Standard,
            facet: "movie".to_string(),
            download_client_id: None,
            download_client_type: "weaver".to_string(),
            download_client_item_id: "shared-job".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Legacy Release".to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("legacy submission should persist");

    let exact_client_lookup = workflow
        .find_by_client_item_id(&ClientJobLocator::new(
            Some("client-a"),
            "weaver",
            "shared-job",
        ))
        .await
        .expect("exact client lookup should succeed");
    assert!(exact_client_lookup.is_none());

    let legacy_lookup = workflow
        .find_by_client_item_id(&ClientJobLocator::new(None, "weaver", "shared-job"))
        .await
        .expect("legacy lookup should succeed")
        .expect("legacy row should still be discoverable by a legacy identity");
    assert_eq!(legacy_lookup.title_id, "legacy-title");

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn record_download_submission_persists_episode_set_scope() {
    let db = std::env::temp_dir().join(format!(
        "scryer_download_submission_episode_set_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let workflow = DownloadSubmissionStore::new(services.datastore());

    workflow
        .record_submission(DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: "title-1".to_string(),
            purpose: scryer_application::DownloadSubmissionPurpose::Standard,
            facet: "anime".to_string(),
            download_client_id: Some("client-a".to_string()),
            download_client_type: "weaver".to_string(),
            download_client_item_id: "job-range".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("RASCAL 01-13".to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: SubmissionScope::EpisodeSet {
                episode_ids: vec!["ep-13".to_string(), "ep-1".to_string()],
            },
        })
        .await
        .expect("record submission should succeed");

    let record = workflow
        .find_by_client_item_id(&ClientJobLocator::new(
            Some("client-a"),
            "weaver",
            "job-range",
        ))
        .await
        .expect("lookup should succeed")
        .expect("submission should exist");

    assert_eq!(
        record.scope,
        SubmissionScope::EpisodeSet {
            episode_ids: vec!["ep-1".to_string(), "ep-13".to_string()]
        }
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn download_submission_signature_lookup_matches_scope() {
    let db = std::env::temp_dir().join(format!(
        "scryer_download_submission_signature_scope_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let workflow = DownloadSubmissionStore::new(services.datastore());

    for (episode_id, item_id) in [("episode-1", "job-1"), ("episode-2", "job-2")] {
        workflow
            .record_submission(DownloadSubmission {
                download_id: scryer_domain::download_identity::DownloadId::new(),
                title_id: "title-1".to_string(),
                purpose: scryer_application::DownloadSubmissionPurpose::AdditionalFile,
                facet: "series".to_string(),
                download_client_id: Some("client-a".to_string()),
                download_client_type: "weaver".to_string(),
                download_client_item_id: item_id.to_string(),
                source_hint: Some("https://example.invalid/same-release.nzb".to_string()),
                source_provider_id: None,
                source_provider_name: None,
                source_kind: None,
                source_title: Some("Same.Release.S01E01.1080p.WEB-DL".to_string()),
                info_hash: None,
                release_size_bytes: None,
                request_signature: Some("same-signature".to_string()),
                scope: SubmissionScope::Episode {
                    episode_id: episode_id.to_string(),
                },
            })
            .await
            .expect("record submission should succeed");
    }

    let episode_two_scope = SubmissionScope::Episode {
        episode_id: "episode-2".to_string(),
    };
    let episode_two = workflow
        .find_by_title_and_request_signature(
            "title-1",
            "same-signature",
            scryer_application::DownloadSubmissionPurpose::AdditionalFile,
            &episode_two_scope,
        )
        .await
        .expect("signature lookup should succeed")
        .expect("episode-two submission should match");
    assert_eq!(episode_two.download_client_item_id, "job-2");

    let episode_one_scope = SubmissionScope::Episode {
        episode_id: "episode-1".to_string(),
    };
    let episode_one = workflow
        .find_by_title_and_request_signature(
            "title-1",
            "same-signature",
            scryer_application::DownloadSubmissionPurpose::AdditionalFile,
            &episode_one_scope,
        )
        .await
        .expect("signature lookup should succeed")
        .expect("episode-one submission should match");
    assert_eq!(episode_one.download_client_item_id, "job-1");

    let collection_scope = SubmissionScope::Collection {
        collection_id: "season-1".to_string(),
    };
    let collection = workflow
        .find_by_title_and_request_signature(
            "title-1",
            "same-signature",
            scryer_application::DownloadSubmissionPurpose::AdditionalFile,
            &collection_scope,
        )
        .await
        .expect("signature lookup should succeed");
    assert!(collection.is_none());

    let _ = std::fs::remove_file(db);
}

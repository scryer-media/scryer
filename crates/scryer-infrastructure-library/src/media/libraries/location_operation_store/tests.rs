use super::*;
use scryer_application::location::{
    model::{LocationExecutionMode, LocationOperationType, VerificationDepth},
    resolution::{FileResolution, ResolutionDisposition},
};

#[tokio::test]
async fn durable_resolutions_round_trip_update_and_remain_scoped_to_requested_file_page() {
    scryer_infrastructure_datastore::register_spellfix_auto_extension().unwrap();
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    scryer_infrastructure_datastore::migrations::replay_source_catalog_for_fresh_install(
        &pool, None, true,
    )
    .await
    .unwrap();
    let store = LocationOperationStore::new(StoreDatastore::Sqlite {
        pool,
        writer_gate: std::sync::Arc::new(tokio::sync::Mutex::new(())),
    });
    let now = chrono::Utc::now();
    let operation = LocationOperation {
        id: "op".into(),
        operation_type: LocationOperationType::RootMove,
        mode: LocationExecutionMode::UserMovedFiles,
        state: LocationOperationState::Queued,
        initiated_by_user_id: None,
        source_library_id: None,
        destination_library_id: None,
        source_root_id: None,
        destination_root_id: None,
        plan_fingerprint: "fingerprint".into(),
        verification_depth: VerificationDepth::Full,
        verification_fallback_count: 0,
        counters: Default::default(),
        detail: None,
        job_run_id: None,
        workflow_operation_id: None,
        cancel_requested: false,
        cancel_requested_at: None,
        confirmed_at: Some(now),
        started_at: None,
        created_at: now,
        updated_at: now,
        completed_at: None,
    };
    store
        .create_location_operation(&operation, None)
        .await
        .unwrap();
    assert_eq!(
        store
            .get_location_operation("op")
            .await
            .unwrap()
            .unwrap()
            .mode,
        LocationExecutionMode::UserMovedFiles
    );
    let mut record = FileResolution {
        operation_id: "op".into(),
        title_id: "title".into(),
        source_path: String::new(),
        original_destination: "/destination/season.nfo".into(),
        destination_path: "/destination/season (from Anime).nfo".into(),
        disposition: ResolutionDisposition::Preserved,
        completed: false,
        source_version: Some("source-version".into()),
        destination_version: None,
        identity_hash: None,
        warning: Some("Both versions were preserved.".into()),
        reason_code: Some("incoming_preserved".into()),
    };
    for index in 0..121 {
        record.source_path = format!("/source/{index}.nfo");
        store.save_file_resolution(&record).await.unwrap();
    }
    record.completed = true;
    record.destination_version = Some("destination-version".into());
    record.identity_hash = Some("completed-proof".into());
    store.save_file_resolution(&record).await.unwrap();
    assert_eq!(
        store.file_resolutions("op", "title").await.unwrap().len(),
        121
    );
    assert_eq!(
        store
            .file_resolutions_for_sources("op", "title", &[record.source_path.clone()])
            .await
            .unwrap(),
        vec![record]
    );
    let sources: Vec<_> = (50..100)
        .map(|index| format!("/source/{index}.nfo"))
        .collect();
    assert_eq!(
        store
            .file_resolutions_for_sources("op", "title", &sources)
            .await
            .unwrap()
            .len(),
        50
    );
    assert!(
        store
            .file_resolutions_for_sources("op", "other-title", &sources)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .file_resolutions_for_sources("op", "title", &[])
            .await
            .unwrap()
            .is_empty()
    );
}

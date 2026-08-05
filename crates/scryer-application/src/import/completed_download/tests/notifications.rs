use super::*;

#[tokio::test]
async fn check_emits_manual_interaction_notification_once() {
    let existing_dir = std::env::temp_dir().join(format!("scryer-completed-path-{}", Id::new().0));
    std::fs::create_dir_all(&existing_dir).expect("create temp dir");
    std::fs::write(existing_dir.join("episode.mkv"), b"video").expect("write video file");
    let completed = build_completed_download(
        "Unknown.Show.S01.Complete.1080p",
        existing_dir.to_string_lossy().as_ref(),
        None,
    );
    let download_client = Arc::new(TestDownloadClient {
        completed_downloads: Arc::new(Mutex::new(vec![CompletedDownload {
            download_client_item_id: "dl-2".to_string(),
            ..completed
        }])),
        completed_download_calls: Arc::new(AtomicUsize::new(0)),
        recent_completed_download_calls: Arc::new(AtomicUsize::new(0)),
        scoped_recent_completed_calls: Arc::new(Mutex::new(Vec::new())),
    });
    let app = build_app_with_download_client(vec![], vec![], vec![], vec![], download_client);
    let mut actor = User::new_admin("admin");
    actor.authorization = scryer_domain::UserAuthorization {
        app: scryer_domain::AppPermissionMask::from_permissions([
            scryer_domain::AppPermission::ManageSystemSettings,
        ]),
        default_library: scryer_domain::LibraryPermissionMask::from_permissions([
            scryer_domain::LibraryPermission::View,
        ]),
        actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
        loaded: true,
        ..Default::default()
    };
    let mut td = TrackedDownload {
        id: "nzbget:unmatched".to_string(),
        client_id: "client-1".to_string(),
        client_type: "nzbget".to_string(),
        client_item: DownloadQueueItem {
            id: Id::new().0,
            title_id: None,
            episode_id: None,
            title_name: "Unknown.Show.S01.Complete.1080p".to_string(),
            facet: Some("series".to_string()),
            category: None,
            client_id: "client-1".to_string(),
            client_name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            state: DownloadQueueState::Completed,
            progress_percent: 100,
            import_transfer_phase: None,
            import_transfer_bytes: None,
            import_transfer_total_bytes: None,
            import_transfer_started_at: None,
            import_transfer_updated_at: None,
            size_bytes: None,
            remaining_seconds: None,
            queued_at: None,
            last_updated_at: None,
            attention_required: false,
            attention_reason: None,
            download_client_item_id: "dl-2".to_string(),
            download_id: None,
            import_status: None,
            import_error_code: None,
            import_error_message: None,
            imported_at: None,
            delete_status: None,
            delete_error_message: None,
            source_provider: None,
            is_scryer_origin: false,
            tracked_state: None,
            tracked_status: None,
            tracked_status_messages: vec![],
            tracked_match_type: None,
        },
        state: TrackedDownloadState::Downloading,
        status: TrackedDownloadStatus::Ok,
        status_messages: vec![],
        title_id: None,
        facet: Some("series".to_string()),
        source_title: None,
        indexer: None,
        added_at: None,
        notified_manual_interaction: false,
        match_type: TitleMatchType::Unmatched,
        is_trackable: true,
        import_attempted: false,
        waiting_for_completed_history: false,
        path_missing_since: None,
        no_video_import_retry: None,
        import_hold: None,
        skip_reacquire_on_failure: false,
        snapshot_missing_since: None,
    };

    check(&app, &mut td).await;
    check(&app, &mut td).await;

    assert_eq!(td.state, TrackedDownloadState::ImportBlocked);
    assert!(td.notified_manual_interaction);

    let activity = app.recent_activity(&actor, 10, 0).await.unwrap();
    assert_eq!(activity.len(), 1);
    assert_eq!(activity[0].kind, ActivityKind::ImportRejected);
    assert!(activity[0].message.contains("couldn't be matched"));

    std::fs::remove_dir_all(&existing_dir).expect("remove temp dir");
}

use super::*;

#[tokio::test]
async fn list_download_queue_does_not_treat_stub_submission_as_origin() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "SABnzbd".to_string(),
            client_type: "sabnzbd".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    download_submissions
        .record_submission(DownloadSubmission {
            title_id: String::new(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: String::new(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "sabnzbd".to_string(),
            download_client_item_id: "foreign-stub".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Foreign Download".to_string()),
            request_signature: None,
            scope: SubmissionScope::Orphan,
        })
        .await
        .expect("record stub submission");

    *download_client.queue_items.lock().await = vec![DownloadQueueItem {
        id: "foreign-stub".to_string(),
        title_id: None,
        episode_id: None,
        title_name: "Foreign Download".to_string(),
        facet: None,
        category: None,
        client_id: "primary".to_string(),
        client_name: "Primary".to_string(),
        client_type: "sabnzbd".to_string(),
        state: DownloadQueueState::Queued,
        progress_percent: 0,
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
        download_client_item_id: "foreign-stub".to_string(),
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
        tracked_status_messages: Vec::new(),
        tracked_match_type: None,
    }];

    let items = app
        .list_download_queue(&user, true, false, false, DownloadActivityFilter::All)
        .await
        .expect("list queue");

    assert_eq!(items.len(), 1);
    assert!(!items[0].is_scryer_origin);
    assert!(items[0].title_id.is_none());
    assert!(items[0].facet.is_none());
}

#[tokio::test]
async fn list_download_queue_uses_live_queue_only_for_all_activity() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );
    let import_repo = Arc::new(TrackingImportRepo::default());
    let app = base_app.with_test_overrides(|services| services.with_imports(import_repo.clone()));

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    *download_client.history_items.lock().await = vec![DownloadQueueItem {
        id: "history-1".to_string(),
        title_id: None,
        episode_id: None,
        title_name: "History Download".to_string(),
        facet: None,
        category: None,
        client_id: "primary".to_string(),
        client_name: "Primary".to_string(),
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
        last_updated_at: Some("100".to_string()),
        attention_required: false,
        attention_reason: None,
        download_client_item_id: "history-1".to_string(),
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
        tracked_status_messages: Vec::new(),
        tracked_match_type: None,
    }];

    let items = app
        .list_download_queue(&user, true, false, false, DownloadActivityFilter::All)
        .await
        .expect("list queue should succeed");

    assert!(items.is_empty());
    assert_eq!(*download_client.history_calls.lock().await, 0);
    assert!(
        download_client
            .recent_activity_calls
            .lock()
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn list_download_queue_for_title_uses_title_scoped_client_query() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    app.services
        .catalog
        .titles
        .create(make_due_hydration_title("title-1", MediaFacet::Series, 1))
        .await
        .expect("seed title");

    download_submissions
        .record_submission(DownloadSubmission {
            title_id: "title-1".to_string(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "series".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "job-1".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Title Scoped Download".to_string()),
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record submission");

    *download_client.queue_items.lock().await = vec![DownloadQueueItem {
        id: "job-1".to_string(),
        title_id: None,
        episode_id: None,
        title_name: "Title Scoped Download".to_string(),
        facet: None,
        category: None,
        client_id: "primary".to_string(),
        client_name: "Primary".to_string(),
        client_type: "nzbget".to_string(),
        state: DownloadQueueState::Queued,
        progress_percent: 0,
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
        download_client_item_id: "job-1".to_string(),
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
        tracked_status_messages: Vec::new(),
        tracked_match_type: None,
    }];

    let items = app
        .list_download_queue_for_title(
            &user,
            "title-1",
            false,
            false,
            false,
            DownloadActivityFilter::All,
        )
        .await
        .expect("title-scoped queue should load");

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].download_client_item_id, "job-1");
    assert_eq!(items[0].title_id.as_deref(), Some("title-1"));
    assert_eq!(*download_client.queue_calls.lock().await, 0);
    assert_eq!(
        download_client
            .queue_for_title_calls
            .lock()
            .await
            .as_slice(),
        &["title-1".to_string()]
    );
    assert!(
        download_client
            .recent_activity_for_title_calls
            .lock()
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn list_download_import_page_returns_only_import_rows_for_selected_filter() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );
    let import_repo = Arc::new(TrackingImportRepo::default());
    let app = base_app.with_test_overrides(|services| services.with_imports(import_repo.clone()));

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    let mut importing =
        queue_history_fixture_item("importing-1", DownloadQueueState::Completed, 40);
    importing.import_status = Some(ImportStatus::Running);

    let mut pending =
        queue_history_fixture_item("pending-1", DownloadQueueState::ImportPending, 30);
    pending.tracked_state = Some(TrackedDownloadState::ImportPending);

    let mut blocked = queue_history_fixture_item("blocked-1", DownloadQueueState::Completed, 20);
    blocked.tracked_state = Some(TrackedDownloadState::ImportBlocked);

    let mut foreign_blocked =
        queue_history_fixture_item("foreign-blocked-1", DownloadQueueState::Completed, 25);
    foreign_blocked.is_scryer_origin = false;
    foreign_blocked.tracked_state = Some(TrackedDownloadState::ImportBlocked);

    let failed = queue_history_fixture_item("failed-1", DownloadQueueState::Failed, 10);
    let completed = queue_history_fixture_item("completed-1", DownloadQueueState::Completed, 5);

    *download_client.history_items.lock().await = vec![
        completed,
        failed,
        foreign_blocked,
        blocked.clone(),
        pending,
        importing,
    ];

    let page = app
        .list_download_import_page(&user, 50, 0, DownloadImportFilter::Blocked)
        .await
        .expect("import page should load");

    // A non-Scryer-origin row is still eligible for manual assignment unless
    // the runtime classifier or configured category ownership proves that it
    // belongs to another app.
    assert!(!page.has_more);
    assert_eq!(page.total_count, 2);
    assert_eq!(page.items.len(), 2);
    let blocked_ids = page
        .items
        .iter()
        .map(|item| item.download_client_item_id.as_str())
        .collect::<Vec<_>>();
    assert!(blocked_ids.contains(&"blocked-1"));
    assert!(blocked_ids.contains(&"foreign-blocked-1"));
    for item in &page.items {
        assert_eq!(
            crate::integration::derive_download_queue_display_state(item),
            DownloadDisplayState::ImportBlocked
        );
    }

    let count = app
        .count_download_import_items(&user, DownloadImportFilter::Blocked)
        .await
        .expect("import count should load");
    // Must agree with page.total_count or the import badge drifts from the list.
    assert_eq!(count, 2);
}

#[tokio::test]
async fn count_download_import_items_matches_selected_filter() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );
    let import_repo = Arc::new(TrackingImportRepo::default());
    let app = base_app.with_test_overrides(|services| services.with_imports(import_repo.clone()));

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    let mut importing =
        queue_history_fixture_item("importing-1", DownloadQueueState::Completed, 40);
    importing.import_status = Some(ImportStatus::Running);

    let mut pending =
        queue_history_fixture_item("pending-1", DownloadQueueState::ImportPending, 30);
    pending.tracked_state = Some(TrackedDownloadState::ImportPending);

    let mut blocked = queue_history_fixture_item("blocked-1", DownloadQueueState::Completed, 20);
    blocked.tracked_state = Some(TrackedDownloadState::ImportBlocked);

    let failed = queue_history_fixture_item("failed-1", DownloadQueueState::Failed, 10);
    let completed = queue_history_fixture_item("completed-1", DownloadQueueState::Completed, 5);

    *download_client.history_items.lock().await =
        vec![completed, failed, blocked, pending.clone(), importing];

    let all_page = app
        .list_download_import_page(&user, 50, 0, DownloadImportFilter::All)
        .await
        .expect("all import page");
    let all_count = app
        .count_download_import_items(&user, DownloadImportFilter::All)
        .await
        .expect("all import count");
    let pending_count = app
        .count_download_import_items(&user, DownloadImportFilter::Pending)
        .await
        .expect("pending import count");

    assert_eq!(all_count, all_page.total_count as i64);
    assert_eq!(pending_count, 1);
    assert_eq!(pending.download_client_item_id, "pending-1");
}

#[tokio::test]
async fn download_import_blocked_includes_snapshot_only_item_when_history_is_empty() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) =
        bootstrap_with_cleanup_tracking(download_client, download_submissions, pending_releases);

    create_enabled_download_client_config(&app, &user, "NZBGet", "nzbget").await;

    let blocked =
        queue_history_fixture_item("blocked-snapshot-1", DownloadQueueState::Completed, 20);
    insert_tracked_download_snapshot(
        &app,
        "blocked-snapshot-1",
        TrackedDownloadState::ImportBlocked,
        blocked,
    )
    .await;

    let page = app
        .list_download_import_page(&user, 50, 0, DownloadImportFilter::Blocked)
        .await
        .expect("blocked import page should include snapshot-only tracked rows");
    let count = app
        .count_download_import_items(&user, DownloadImportFilter::Blocked)
        .await
        .expect("blocked import count should include snapshot-only tracked rows");

    assert_eq!(page.total_count, 1);
    assert_eq!(count, 1);
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].download_client_item_id, "blocked-snapshot-1");
    assert_eq!(page.items[0].state, DownloadQueueState::Completed);
    assert_eq!(page.items[0].import_status, None);
    assert_eq!(
        page.items[0].tracked_state,
        Some(TrackedDownloadState::ImportBlocked)
    );
    assert_eq!(
        crate::integration::derive_download_queue_display_state(&page.items[0]),
        DownloadDisplayState::ImportBlocked
    );
}

#[tokio::test]
async fn download_import_all_includes_snapshot_only_pending_and_importing_items() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) =
        bootstrap_with_cleanup_tracking(download_client, download_submissions, pending_releases);

    create_enabled_download_client_config(&app, &user, "NZBGet", "nzbget").await;

    insert_tracked_download_snapshot(
        &app,
        "pending-snapshot-1",
        TrackedDownloadState::ImportPending,
        queue_history_fixture_item("pending-snapshot-1", DownloadQueueState::Completed, 30),
    )
    .await;
    insert_tracked_download_snapshot(
        &app,
        "importing-snapshot-1",
        TrackedDownloadState::Importing,
        queue_history_fixture_item("importing-snapshot-1", DownloadQueueState::Completed, 40),
    )
    .await;

    let page = app
        .list_download_import_page(&user, 50, 0, DownloadImportFilter::All)
        .await
        .expect("all import page should include snapshot-only tracked rows");

    assert_eq!(page.total_count, 2);
    let pending = page
        .items
        .iter()
        .find(|item| item.download_client_item_id == "pending-snapshot-1")
        .expect("pending snapshot row");
    assert_eq!(pending.state, DownloadQueueState::ImportPending);
    assert_eq!(pending.import_status, None);
    assert_eq!(
        crate::integration::derive_download_queue_display_state(pending),
        DownloadDisplayState::ImportPending
    );

    let importing = page
        .items
        .iter()
        .find(|item| item.download_client_item_id == "importing-snapshot-1")
        .expect("importing snapshot row");
    assert_eq!(importing.state, DownloadQueueState::Completed);
    assert_eq!(importing.import_status, Some(ImportStatus::Running));
    assert_eq!(
        crate::integration::derive_download_queue_display_state(importing),
        DownloadDisplayState::Importing
    );
}

#[tokio::test]
async fn synthetic_download_import_rows_are_enriched_from_submissions_before_permission_filtering()
{
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, admin) = bootstrap_with_cleanup_tracking(
        download_client,
        download_submissions.clone(),
        pending_releases,
    );

    create_enabled_download_client_config(&app, &admin, "NZBGet", "nzbget").await;
    let title = app
        .add_title(
            &admin,
            NewTitle {
                name: "Manual Import Visibility".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                root_folder_id: None,
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title for scoped visibility");
    let scoped_actor = library_permission_user(
        "resolver",
        &title.library_id,
        &[scryer_domain::LibraryPermission::ResolveImports],
    );

    let mut blocked =
        queue_history_fixture_item("blocked-submission-1", DownloadQueueState::Completed, 20);
    blocked.title_id = None;
    blocked.facet = None;
    insert_tracked_download_snapshot(
        &app,
        "blocked-submission-1",
        TrackedDownloadState::ImportBlocked,
        blocked,
    )
    .await;
    download_submissions
        .record_submission(DownloadSubmission {
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "movie".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "blocked-submission-1".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Manual Import Visibility".to_string()),
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record download submission");

    let page = app
        .list_download_import_page(&scoped_actor, 50, 0, DownloadImportFilter::Blocked)
        .await
        .expect("scoped actor should see submission-enriched snapshot row");

    assert_eq!(page.total_count, 1);
    assert_eq!(page.items.len(), 1);
    assert_eq!(
        page.items[0].download_client_item_id,
        "blocked-submission-1"
    );
    assert_eq!(page.items[0].title_id.as_deref(), Some(title.id.as_str()));
    assert_eq!(page.items[0].facet.as_deref(), Some("movie"));
}

#[tokio::test]
async fn find_download_queue_scope_ignores_stale_submission_titles() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Scope Regression Movie".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                root_folder_id: None,
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create visible title");

    let mut blocked = queue_history_fixture_item("blocked-1", DownloadQueueState::Completed, 20);
    blocked.title_id = Some(title.id.clone());
    blocked.title_name = title.name.clone();
    blocked.facet = Some("movie".to_string());
    blocked.tracked_state = Some(TrackedDownloadState::ImportBlocked);
    *download_client.history_items.lock().await = vec![blocked];

    download_submissions
        .record_submission(DownloadSubmission {
            title_id: "missing-title".to_string(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "movie".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "blocked-1".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Fixture blocked-1".to_string()),
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record stale submission");

    let scope = app
        .find_download_queue_scope(&user, Some("primary"), "nzbget", "blocked-1")
        .await
        .expect("stale scope lookup should not fail");
    assert!(scope.is_none());

    let page = app
        .list_download_import_page(&user, 50, 0, DownloadImportFilter::All)
        .await
        .expect("download import page should still load");
    assert_eq!(page.total_count, 1);
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].download_client_item_id, "blocked-1");
}

#[tokio::test]
async fn find_download_queue_scope_returns_orphan_without_title_lookup() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client,
        download_submissions.clone(),
        pending_releases,
    );

    download_submissions
        .record_submission(DownloadSubmission {
            title_id: String::new(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "anime".to_string(),
            download_client_id: Some("weaver-primary".to_string()),
            download_client_type: "weaver".to_string(),
            download_client_item_id: "foreign-10000".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Foreign Weaver Download".to_string()),
            request_signature: None,
            scope: SubmissionScope::Orphan,
        })
        .await
        .expect("record orphan submission");

    let scope = app
        .find_download_queue_scope(&user, Some("weaver-primary"), "weaver", "foreign-10000")
        .await
        .expect("orphan scope lookup should not require a title");

    assert!(matches!(scope, Some(SubmissionScope::Orphan)));
}

struct TrackedTitleAssignmentFixture {
    app: AppUseCase,
    user: User,
    submissions: Arc<TrackingDownloadSubmissionRepo>,
    title: Title,
    tracker: crate::tracked_downloads::TrackedDownloadService,
    tracked_id: String,
    submission: DownloadSubmission,
}

async fn tracked_title_assignment_fixture() -> TrackedTitleAssignmentFixture {
    let download_client = Arc::new(StubDownloadClient::default());
    let submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) =
        bootstrap_with_cleanup_tracking(download_client, submissions.clone(), pending_releases);
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Manual Assignment".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create assignment title");
    let client_id = "weaver-primary";
    let item_id = "foreign-10000";
    let mut queue_item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 100);
    queue_item.client_id = client_id.to_string();
    queue_item.client_name = "Weaver".to_string();
    queue_item.client_type = "weaver".to_string();
    queue_item.title_id = None;
    queue_item.facet = None;
    let tracked_id =
        crate::tracked_downloads::tracked_download_id(Some(client_id), "weaver", item_id);
    let mut tracker = crate::tracked_downloads::TrackedDownloadService::new();
    tracker.insert_for_tests(crate::tracked_downloads::TrackedDownload {
        id: tracked_id.clone(),
        client_id: client_id.to_string(),
        client_type: "weaver".to_string(),
        client_item: queue_item,
        state: TrackedDownloadState::ImportBlocked,
        status: scryer_domain::TrackedDownloadStatus::Warning,
        status_messages: vec!["title required".to_string()],
        title_id: None,
        facet: None,
        source_title: Some("Foreign.Download".to_string()),
        indexer: None,
        added_at: None,
        notified_manual_interaction: false,
        match_type: scryer_domain::TitleMatchType::Unmatched,
        is_trackable: true,
        import_attempted: false,
        waiting_for_completed_history: false,
        path_missing_since: None,
        no_video_import_retry: None,
        foreign_import_classification: None,
        skip_reacquire_on_failure: false,
    });
    let submission = DownloadSubmission {
        title_id: title.id.clone(),
        purpose: DownloadSubmissionPurpose::Standard,
        facet: "movie".to_string(),
        download_client_id: Some(client_id.to_string()),
        download_client_type: "weaver".to_string(),
        download_client_item_id: item_id.to_string(),
        source_hint: None,
        source_provider_id: None,
        source_provider_name: None,
        source_kind: None,
        source_title: Some(title.name.clone()),
        request_signature: None,
        scope: SubmissionScope::Title,
    };

    TrackedTitleAssignmentFixture {
        app,
        user,
        submissions,
        title,
        tracker,
        tracked_id,
        submission,
    }
}

#[tokio::test]
async fn assign_tracked_download_title_serializes_submission_and_runtime_assignment() {
    let mut fixture = tracked_title_assignment_fixture().await;
    let actor_snapshot = crate::domain_events::DomainEventActor::from(&fixture.user)
        .into_download_submission_actor_snapshot();

    crate::integration::workflow::assign_tracked_download_title_command(
        &fixture.app,
        &mut fixture.tracker,
        &HashSet::new(),
        fixture.tracked_id.clone(),
        fixture.title.clone(),
        fixture.submission.clone(),
        actor_snapshot,
    )
    .await
    .expect("assignment command should succeed");

    let submissions = fixture.submissions.store.lock().await;
    assert_eq!(submissions.len(), 1);
    assert_eq!(submissions[0].title_id, fixture.submission.title_id);
    assert_eq!(
        submissions[0].download_client_item_id,
        fixture.submission.download_client_item_id
    );
    drop(submissions);
    let tracked = fixture
        .tracker
        .find(&fixture.tracked_id)
        .expect("tracked download remains available");
    assert_eq!(tracked.title_id.as_deref(), Some(fixture.title.id.as_str()));
    assert_eq!(tracked.facet.as_deref(), Some("movie"));
    assert_eq!(
        tracked.match_type,
        scryer_domain::TitleMatchType::Submission
    );
    assert_eq!(tracked.state, TrackedDownloadState::ImportBlocked);
}

#[tokio::test]
async fn assign_tracked_download_title_busy_rejection_persists_nothing() {
    let mut fixture = tracked_title_assignment_fixture().await;
    let actor_snapshot = crate::domain_events::DomainEventActor::from(&fixture.user)
        .into_download_submission_actor_snapshot();
    let in_flight = HashSet::from([fixture.tracked_id.clone()]);

    let error = crate::integration::workflow::assign_tracked_download_title_command(
        &fixture.app,
        &mut fixture.tracker,
        &in_flight,
        fixture.tracked_id.clone(),
        fixture.title,
        fixture.submission,
        actor_snapshot,
    )
    .await
    .expect_err("busy assignment should fail");

    assert!(matches!(error, AppError::Validation(_)));
    assert!(fixture.submissions.store.lock().await.is_empty());
    assert!(
        fixture
            .tracker
            .find(&fixture.tracked_id)
            .is_some_and(|tracked| tracked.title_id.is_none())
    );
}

#[tokio::test]
async fn assign_tracked_download_title_missing_rejection_persists_nothing() {
    let fixture = tracked_title_assignment_fixture().await;
    let actor_snapshot = crate::domain_events::DomainEventActor::from(&fixture.user)
        .into_download_submission_actor_snapshot();
    let mut empty_tracker = crate::tracked_downloads::TrackedDownloadService::new();

    let error = crate::integration::workflow::assign_tracked_download_title_command(
        &fixture.app,
        &mut empty_tracker,
        &HashSet::new(),
        fixture.tracked_id,
        fixture.title,
        fixture.submission,
        actor_snapshot,
    )
    .await
    .expect_err("missing assignment should fail");

    assert!(matches!(error, AppError::NotFound(_)));
    assert!(fixture.submissions.store.lock().await.is_empty());
}

#[tokio::test]
async fn assign_tracked_download_title_persistence_failure_keeps_runtime_unchanged() {
    let mut fixture = tracked_title_assignment_fixture().await;
    *fixture.submissions.record_submission_error.lock().await =
        Some("submission store unavailable".to_string());
    let actor_snapshot = crate::domain_events::DomainEventActor::from(&fixture.user)
        .into_download_submission_actor_snapshot();

    let error = crate::integration::workflow::assign_tracked_download_title_command(
        &fixture.app,
        &mut fixture.tracker,
        &HashSet::new(),
        fixture.tracked_id.clone(),
        fixture.title,
        fixture.submission,
        actor_snapshot,
    )
    .await
    .expect_err("persistence failure should fail assignment");

    assert!(matches!(error, AppError::Repository(_)));
    assert!(fixture.submissions.store.lock().await.is_empty());
    let tracked = fixture
        .tracker
        .find(&fixture.tracked_id)
        .expect("tracked download remains available");
    assert!(tracked.title_id.is_none());
    assert_eq!(tracked.match_type, scryer_domain::TitleMatchType::Unmatched);
    assert_eq!(tracked.state, TrackedDownloadState::ImportBlocked);
}

#[tokio::test]
async fn list_download_import_page_returns_promptly_when_tracked_snapshot_handle_never_replies() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (tracked_download_tx, _blocked_rx) = tokio::sync::mpsc::channel(1);
    let (app, user) = bootstrap_with_cleanup_tracking_and_tracked_handle(
        download_client.clone(),
        download_submissions,
        pending_releases,
        crate::tracked_downloads::TrackedDownloadHandle::new(tracked_download_tx),
    );

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    *download_client.history_items.lock().await = vec![queue_history_fixture_item(
        "pending-1",
        DownloadQueueState::ImportPending,
        40,
    )];

    let page = timeout(
        Duration::from_millis(100),
        app.list_download_import_page(&user, 50, 0, DownloadImportFilter::All),
    )
    .await
    .expect("download import page should stay responsive even when the tracked snapshot handle is wedged")
    .expect("download import page should load");

    assert_eq!(page.total_count, 1);
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].download_client_item_id, "pending-1");
}

#[tokio::test]
async fn list_download_import_page_uses_runtime_tracked_snapshot_cache() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (tracked_download_tx, _blocked_rx) = tokio::sync::mpsc::channel(1);
    let (app, user) = bootstrap_with_cleanup_tracking_and_tracked_handle(
        download_client.clone(),
        download_submissions,
        pending_releases,
        crate::tracked_downloads::TrackedDownloadHandle::new(tracked_download_tx),
    );

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    let history_item = queue_history_fixture_item("completed-1", DownloadQueueState::Completed, 40);
    *download_client.history_items.lock().await = vec![history_item.clone()];

    let tracked_id =
        crate::tracked_downloads::tracked_download_id(Some("primary"), "nzbget", "completed-1");
    app.runtime
        .acquisition
        .tracked_download_snapshot
        .write()
        .await
        .insert(
            tracked_id,
            crate::tracked_downloads::TrackedDownloadQueueMetadata {
                client_item: history_item,
                client_id: "primary".to_string(),
                client_type: "nzbget".to_string(),
                title_id: Some("title-1".to_string()),
                facet: Some("series".to_string()),
                source_title: Some("Cached Release".to_string()),
                state: TrackedDownloadState::ImportBlocked,
                status: scryer_domain::TrackedDownloadStatus::Warning,
                status_messages: vec!["moving files to nas".to_string()],
                match_type: scryer_domain::TitleMatchType::Submission,
                foreign_import_classification: None,
            },
        );

    let page = timeout(
        Duration::from_millis(100),
        app.list_download_import_page(&user, 50, 0, DownloadImportFilter::All),
    )
    .await
    .expect("download import page should stay responsive with cached tracked metadata")
    .expect("download import page should load");

    assert_eq!(page.items.len(), 1);
    assert_eq!(
        page.items[0].tracked_state,
        Some(TrackedDownloadState::ImportBlocked)
    );
    assert_eq!(
        page.items[0].tracked_status,
        Some(scryer_domain::TrackedDownloadStatus::Warning)
    );
    assert_eq!(
        page.items[0].tracked_status_messages,
        vec!["moving files to nas".to_string()]
    );
    assert_eq!(page.items[0].title_id.as_deref(), Some("title-1"));
}

#[tokio::test]
async fn foreign_runtime_classification_hides_queue_import_and_history_with_aligned_import_count() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );

    create_enabled_download_client_config(&app, &user, "NZBGet", "nzbget").await;

    let mut foreign_item =
        queue_history_fixture_item("foreign-classified-1", DownloadQueueState::Completed, 20);
    foreign_item.client_id = "primary".to_string();
    foreign_item.client_type = "nzbget".to_string();
    foreign_item.category = Some("movie".to_string());
    *download_client.queue_items.lock().await = vec![foreign_item.clone()];
    *download_client.history_items.lock().await = vec![foreign_item.clone()];

    let tracked_id = crate::tracked_downloads::tracked_download_id(
        Some("primary"),
        "nzbget",
        "foreign-classified-1",
    );
    app.runtime
        .acquisition
        .tracked_download_snapshot
        .write()
        .await
        .insert(
            tracked_id,
            crate::tracked_downloads::TrackedDownloadQueueMetadata {
                client_item: foreign_item,
                client_id: "primary".to_string(),
                client_type: "nzbget".to_string(),
                title_id: None,
                facet: None,
                source_title: None,
                state: TrackedDownloadState::ImportBlocked,
                status: scryer_domain::TrackedDownloadStatus::Ok,
                status_messages: Vec::new(),
                match_type: scryer_domain::TitleMatchType::Unmatched,
                foreign_import_classification: Some(
                    crate::tracked_downloads::ForeignDownloadClassification::DroneParameter,
                ),
            },
        );

    let import_page = app
        .list_download_import_page(&user, 50, 0, DownloadImportFilter::All)
        .await
        .expect("import page should load");
    let import_count = app
        .count_download_import_items(&user, DownloadImportFilter::All)
        .await
        .expect("import count should load");
    let queue = app
        .list_download_queue(&user, true, false, true, DownloadActivityFilter::All)
        .await
        .expect("queue should load");
    let history = app
        .list_download_history_page(
            &user,
            50,
            0,
            Some(vec![DownloadHistoryFilter::All]),
            None,
            false,
            None,
        )
        .await
        .expect("history should load");

    assert_eq!(import_page.total_count, 0);
    assert_eq!(import_count, 0);
    assert!(queue.is_empty());
    assert_eq!(history.total_count, 0);
    assert!(history.available_clients.is_empty());
}

#[tokio::test]
async fn list_download_import_page_degrades_promptly_for_limit_one_count_reads_when_snapshot_cache_is_contended()
 {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    let mut importing =
        queue_history_fixture_item("importing-1", DownloadQueueState::Completed, 40);
    importing.import_status = Some(ImportStatus::Processing);
    *download_client.history_items.lock().await = vec![importing];

    let _snapshot_guard = app
        .runtime
        .acquisition
        .tracked_download_snapshot
        .write()
        .await;

    let page = timeout(
        Duration::from_millis(100),
        app.list_download_import_page(&user, 1, 0, DownloadImportFilter::All),
    )
    .await
    .expect("limit-one count-style download import read should degrade instead of blocking")
    .expect("download import page should load");

    assert_eq!(page.total_count, 1);
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].download_client_item_id, "importing-1");
    assert_eq!(page.items[0].import_status, Some(ImportStatus::Processing));
    assert_eq!(page.items[0].tracked_state, None);
}

#[tokio::test]
async fn download_import_page_renders_importing_state_from_runtime_snapshot() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (tracked_download_tx, _blocked_rx) = tokio::sync::mpsc::channel(1);
    let (app, user) = bootstrap_with_cleanup_tracking_and_tracked_handle(
        download_client.clone(),
        download_submissions,
        pending_releases,
        crate::tracked_downloads::TrackedDownloadHandle::new(tracked_download_tx),
    );

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    let item_id = "blocked-worker-1";
    let mut history_item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);
    history_item.import_status = Some(ImportStatus::Processing);
    *download_client.history_items.lock().await = vec![history_item.clone()];

    let tracked_id =
        crate::tracked_downloads::tracked_download_id(Some("primary"), "nzbget", item_id);
    app.runtime
        .acquisition
        .tracked_download_snapshot
        .write()
        .await
        .insert(
            tracked_id,
            crate::tracked_downloads::TrackedDownloadQueueMetadata {
                client_item: history_item,
                client_id: "primary".to_string(),
                client_type: "nzbget".to_string(),
                title_id: Some("title-1".to_string()),
                facet: Some("movie".to_string()),
                source_title: Some("Fixture blocked-worker-1".to_string()),
                state: TrackedDownloadState::Importing,
                status: scryer_domain::TrackedDownloadStatus::Ok,
                status_messages: vec!["Moving files to library.".to_string()],
                match_type: scryer_domain::TitleMatchType::Submission,
                foreign_import_classification: None,
            },
        );

    let page = app
        .list_download_import_page(&user, 1, 0, DownloadImportFilter::All)
        .await
        .expect("download import page should load");

    assert_eq!(page.total_count, 1);
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].download_client_item_id, item_id);
    assert_eq!(
        page.items[0].tracked_state,
        Some(TrackedDownloadState::Importing)
    );
    assert_eq!(page.items[0].import_status, Some(ImportStatus::Processing));
    assert_eq!(
        crate::integration::derive_download_queue_display_state(&page.items[0]),
        DownloadDisplayState::Importing
    );
    assert_eq!(
        page.items[0].tracked_status_messages,
        vec!["Moving files to library.".to_string()]
    );
}

#[tokio::test]
async fn download_queue_poller_retries_imported_cleanup_from_facet_routing_until_delete_succeeds() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );

    let config =
        create_enabled_download_client_config(&app, &user, "Primary NZBGet", "nzbget").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, false).await;

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Imported Cleanup Retry".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create monitored movie title");

    let item_id = "imported-cleanup-1";
    let download_id = "download-id-imported-cleanup-1";
    let mut history_item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);
    history_item.client_id = config.id.clone();
    history_item.client_name = config.name.clone();
    history_item.title_id = Some(title.id.clone());
    history_item.title_name = title.name.clone();
    history_item.facet = Some("movie".to_string());
    history_item.download_id = Some(download_id.to_string());
    let tracked_id = crate::tracked_downloads::tracked_download_id_for_item(&history_item);
    *download_client.history_items.lock().await = vec![history_item];

    let submission_identity = DownloadSubmissionIdentity {
        download_id: Some(download_id.to_string()),
    };
    let submission_source_identity =
        DownloadSourceIdentity::new(Some(config.id.as_str()), "nzbget", item_id);
    download_submissions
        .record_submission_with_identity(
            DownloadSubmission {
                title_id: title.id.clone(),
                purpose: crate::DownloadSubmissionPurpose::Standard,
                facet: "movie".to_string(),
                download_client_id: Some(config.id.clone()),
                download_client_type: "nzbget".to_string(),
                download_client_item_id: item_id.to_string(),
                source_hint: None,
                source_provider_id: None,
                source_provider_name: None,
                source_kind: None,
                source_title: Some(title.name.clone()),
                request_signature: None,
                scope: SubmissionScope::Title,
            },
            submission_identity,
        )
        .await
        .expect("seed owned download submission identity");
    download_submissions
        .update_tracked_state(
            &submission_source_identity,
            TrackedDownloadState::Imported.as_str(),
        )
        .await
        .expect("seed imported tracked state");

    download_client
        .set_delete_error(Some("repository: delete failed"))
        .await;

    let (_tx, tracked_download_rx) = tokio::sync::mpsc::channel(8);
    let (_snapshot_tx, snapshot_rx) = tokio::sync::mpsc::channel(1);
    let token = tokio_util::sync::CancellationToken::new();
    let poller = tokio::spawn(
        crate::integration::start_download_queue_poller_with_options(
            app.clone(),
            token.child_token(),
            tracked_download_rx,
            snapshot_rx,
            crate::integration::DownloadQueuePollerOptions {
                interval: Duration::from_millis(50),
                ..Default::default()
            },
        ),
    );

    timeout(Duration::from_secs(5), async {
        loop {
            if app
                .runtime
                .acquisition
                .tracked_download_snapshot
                .read()
                .await
                .get(&tracked_id)
                .is_some_and(|metadata| metadata.state == TrackedDownloadState::Imported)
            {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("tracked imported item should stay visible after retryable delete failure");
    assert!(download_client.deleted_items.lock().await.is_empty());

    let mut pushed_out_history = (0..105)
        .map(|index| {
            let mut item = queue_history_fixture_item(
                &format!("recent-history-{index}"),
                DownloadQueueState::Completed,
                1_000 - index as i64,
            );
            item.client_id = config.id.clone();
            item.client_name = config.name.clone();
            item
        })
        .collect::<Vec<_>>();
    let mut hidden_target = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 1);
    hidden_target.client_id = config.id.clone();
    hidden_target.client_name = config.name.clone();
    hidden_target.title_id = Some(title.id.clone());
    hidden_target.title_name = title.name.clone();
    hidden_target.facet = Some("movie".to_string());
    hidden_target.download_id = Some(download_id.to_string());
    pushed_out_history.push(hidden_target);
    *download_client.history_items.lock().await = pushed_out_history;

    download_client.set_delete_error(None).await;

    timeout(Duration::from_secs(5), async {
        loop {
            if !download_client.deleted_requests.lock().await.is_empty() {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("poller should retry imported cleanup on the next cycle");

    assert_eq!(
        download_client.deleted_requests.lock().await.clone(),
        vec![(Some(config.id.clone()), None, item_id.to_string(), true)]
    );

    timeout(Duration::from_secs(5), async {
        loop {
            if !app
                .runtime
                .acquisition
                .tracked_download_snapshot
                .read()
                .await
                .contains_key(&tracked_id)
            {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("tracked imported item should disappear once cleanup succeeds");

    token.cancel();
    poller
        .await
        .expect("download queue poller should stop cleanly");
}

#[tokio::test]
async fn external_weaver_snapshot_uses_tracked_runtime_and_provided_completed_rows() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );

    let config =
        create_enabled_download_client_config(&app, &user, "Primary Weaver", "weaver").await;

    let (_command_tx, tracked_download_rx) = tokio::sync::mpsc::channel(8);
    let (snapshot_tx, snapshot_rx) = tokio::sync::mpsc::channel(8);
    let ingest = crate::tracked_downloads::TrackedDownloadSnapshotIngestHandle::new(snapshot_tx);
    let token = tokio_util::sync::CancellationToken::new();
    let poller = tokio::spawn(
        crate::integration::start_download_queue_poller_with_options(
            app.clone(),
            token.child_token(),
            tracked_download_rx,
            snapshot_rx,
            crate::integration::DownloadQueuePollerOptions {
                interval: Duration::from_secs(60),
                excluded_client_types: vec!["weaver".to_string()],
            },
        ),
    );

    let item_id = "weaver-external-1";
    let download_id = "weaver-download-1";
    let mut item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);
    item.client_id = config.id.clone();
    item.client_name = config.name.clone();
    item.client_type = "weaver".to_string();
    item.download_id = Some(download_id.to_string());
    let tracked_id = crate::tracked_downloads::tracked_download_id_for_item(&item);

    ingest
        .publish(crate::tracked_downloads::TrackedDownloadSnapshotUpdate {
            scope: crate::tracked_downloads::TrackedDownloadSnapshotScope::Delta,
            items: vec![item.clone()],
            completed_downloads: Vec::new(),
            actor_id: None,
        })
        .await
        .expect("publish missing-history update");

    timeout(Duration::from_secs(5), async {
        loop {
            if app
                .runtime
                .acquisition
                .tracked_download_snapshot
                .read()
                .await
                .get(&tracked_id)
                .is_some_and(|metadata| metadata.state == TrackedDownloadState::ImportPending)
            {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("external snapshot should enter retryable waiting state");

    let mut completed =
        completed_download_fixture_item(item_id, "title-1", item.title_name.as_str(), "");
    completed.client_id = config.id.clone();
    completed.client_type = "weaver".to_string();
    completed.download_id = Some(download_id.to_string());

    ingest
        .publish(crate::tracked_downloads::TrackedDownloadSnapshotUpdate {
            scope: crate::tracked_downloads::TrackedDownloadSnapshotScope::Delta,
            items: vec![item],
            completed_downloads: vec![completed],
            actor_id: None,
        })
        .await
        .expect("publish provided completed update");

    timeout(Duration::from_secs(5), async {
        loop {
            if app
                .runtime
                .acquisition
                .tracked_download_snapshot
                .read()
                .await
                .get(&tracked_id)
                .is_some_and(|metadata| {
                    metadata.state == TrackedDownloadState::Downloading
                        && metadata.status_messages.iter().any(|message| {
                            message.contains("Completed download path is not available yet")
                        })
                })
            {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("provided completed row should drive normal tracked evaluation");

    assert_eq!(*download_client.completed_download_calls.lock().await, 0);

    token.cancel();
    poller
        .await
        .expect("download queue poller should stop cleanly");
}

#[tokio::test]
async fn external_weaver_missing_history_retries_from_tracked_runtime() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );
    let import_repo = Arc::new(TrackingImportRepo::default());
    let app = base_app.with_test_overrides(|services| services.with_imports(import_repo.clone()));

    let config =
        create_enabled_download_client_config(&app, &user, "Primary Weaver", "weaver").await;
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Managed Weaver Retry".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create monitored movie title");

    let (_command_tx, tracked_download_rx) = tokio::sync::mpsc::channel(8);
    let (snapshot_tx, snapshot_rx) = tokio::sync::mpsc::channel(8);
    let ingest = crate::tracked_downloads::TrackedDownloadSnapshotIngestHandle::new(snapshot_tx);
    let token = tokio_util::sync::CancellationToken::new();
    let poller = tokio::spawn(
        crate::integration::start_download_queue_poller_with_options(
            app.clone(),
            token.child_token(),
            tracked_download_rx,
            snapshot_rx,
            crate::integration::DownloadQueuePollerOptions {
                interval: Duration::from_millis(50),
                excluded_client_types: vec!["weaver".to_string()],
            },
        ),
    );

    let item_id = "weaver-external-retry-1";
    let download_id = "weaver-download-retry-1";
    let mut item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);
    item.client_id = config.id.clone();
    item.client_name = config.name.clone();
    item.client_type = "weaver".to_string();
    item.download_id = Some(download_id.to_string());
    item.title_id = Some(title.id.clone());
    item.title_name = title.name.clone();
    item.facet = Some("movie".to_string());
    let tracked_id = crate::tracked_downloads::tracked_download_id_for_item(&item);
    *download_client.recent_completed_downloads.lock().await = Some(Vec::new());

    ingest
        .publish(crate::tracked_downloads::TrackedDownloadSnapshotUpdate {
            scope: crate::tracked_downloads::TrackedDownloadSnapshotScope::Delta,
            items: vec![item.clone()],
            completed_downloads: Vec::new(),
            actor_id: None,
        })
        .await
        .expect("publish missing-history update");

    timeout(Duration::from_secs(5), async {
        loop {
            let snapshot_state = app
                .runtime
                .acquisition
                .tracked_download_snapshot
                .read()
                .await
                .get(&tracked_id)
                .map(|metadata| metadata.state);
            let retry_called = !download_client
                .recent_completed_download_calls
                .lock()
                .await
                .is_empty();
            if snapshot_state == Some(TrackedDownloadState::ImportPending) && retry_called {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("missing history should remain retryable and be retried centrally");

    let queue_events = app
        .services
        .events
        .domain_events
        .list(&DomainEventFilter {
            event_types: Some(vec![DomainEventType::DownloadQueueItemUpserted]),
            title_id: None,
            facet: None,
            after_sequence: Some(0),
            before_sequence: None,
            limit: 100,
        })
        .await
        .expect("queue upsert events should load");
    assert!(queue_events.iter().any(|event| {
        matches!(
            &event.payload,
            DomainEventPayload::DownloadQueueItemUpserted(data)
                if data.item.download_client_item_id == item_id
                    && data.item.tracked_state == Some(TrackedDownloadState::ImportPending)
        )
    }));

    let source_dir = tempfile::tempdir().expect("source tempdir");
    let mut completed = completed_download_fixture_item(
        item_id,
        &title.id,
        item.title_name.as_str(),
        source_dir.path().to_string_lossy().as_ref(),
    );
    completed.client_id = config.id.clone();
    completed.client_type = "weaver".to_string();
    completed.download_id = None;
    *download_client.recent_completed_downloads.lock().await = Some(vec![completed]);

    timeout(Duration::from_secs(5), async {
        loop {
            if import_repo
                .records
                .lock()
                .await
                .iter()
                .any(|record| record.source_ref == item_id && record.source_system == "weaver")
            {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("central retry should dispatch once exact source history appears");

    assert_eq!(*download_client.completed_download_calls.lock().await, 0);
    assert!(
        !download_client
            .recent_completed_download_calls
            .lock()
            .await
            .is_empty()
    );

    token.cancel();
    poller
        .await
        .expect("download queue poller should stop cleanly");
}

#[tokio::test]
async fn external_weaver_aged_out_history_recovers_via_targeted_lookup() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );
    let import_repo = Arc::new(TrackingImportRepo::default());
    let app = base_app.with_test_overrides(|services| services.with_imports(import_repo.clone()));

    let config =
        create_enabled_download_client_config(&app, &user, "Primary Weaver", "weaver").await;
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Aged Out Weaver Retry".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create monitored movie title");

    let (_command_tx, tracked_download_rx) = tokio::sync::mpsc::channel(8);
    let (snapshot_tx, snapshot_rx) = tokio::sync::mpsc::channel(8);
    let ingest = crate::tracked_downloads::TrackedDownloadSnapshotIngestHandle::new(snapshot_tx);
    let token = tokio_util::sync::CancellationToken::new();
    let poller = tokio::spawn(
        crate::integration::start_download_queue_poller_with_options(
            app.clone(),
            token.child_token(),
            tracked_download_rx,
            snapshot_rx,
            crate::integration::DownloadQueuePollerOptions {
                interval: Duration::from_millis(50),
                excluded_client_types: vec!["weaver".to_string()],
            },
        ),
    );

    let item_id = "weaver-aged-out-1";
    let download_id = "weaver-download-aged-1";
    let mut item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);
    item.client_id = config.id.clone();
    item.client_name = config.name.clone();
    item.client_type = "weaver".to_string();
    item.download_id = Some(download_id.to_string());
    item.title_id = Some(title.id.clone());
    item.title_name = title.name.clone();
    item.facet = Some("movie".to_string());
    let tracked_id = crate::tracked_downloads::tracked_download_id_for_item(&item);
    // The recent window never contains the row — it aged out of the bounded
    // listing while the item sat in the waiting state.
    *download_client.recent_completed_downloads.lock().await = Some(Vec::new());

    ingest
        .publish(crate::tracked_downloads::TrackedDownloadSnapshotUpdate {
            scope: crate::tracked_downloads::TrackedDownloadSnapshotScope::Delta,
            items: vec![item.clone()],
            completed_downloads: Vec::new(),
            actor_id: None,
        })
        .await
        .expect("publish missing-history update");

    timeout(Duration::from_secs(5), async {
        loop {
            if app
                .runtime
                .acquisition
                .tracked_download_snapshot
                .read()
                .await
                .get(&tracked_id)
                .is_some_and(|metadata| metadata.state == TrackedDownloadState::ImportPending)
            {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("missing history should enter the retryable waiting state");

    let source_dir = tempfile::tempdir().expect("source tempdir");
    let mut completed = completed_download_fixture_item(
        item_id,
        &title.id,
        item.title_name.as_str(),
        source_dir.path().to_string_lossy().as_ref(),
    );
    completed.client_id = config.id.clone();
    completed.client_type = "weaver".to_string();
    completed.download_id = None;
    download_client
        .targeted_completed_downloads
        .lock()
        .await
        .insert(item_id.to_string(), completed);

    timeout(Duration::from_secs(5), async {
        loop {
            if import_repo
                .records
                .lock()
                .await
                .iter()
                .any(|record| record.source_ref == item_id && record.source_system == "weaver")
            {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("targeted lookup should recover an item missing from the recent window");

    assert!(
        download_client
            .targeted_completed_download_calls
            .lock()
            .await
            .iter()
            .any(|reference| reference == item_id),
        "retry should issue a targeted per-item lookup"
    );
    assert_eq!(*download_client.completed_download_calls.lock().await, 0);

    token.cancel();
    poller
        .await
        .expect("download queue poller should stop cleanly");
}

#[tokio::test]
async fn excluded_client_history_reconciliation_imports_missed_completion() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );
    let import_repo = Arc::new(TrackingImportRepo::default());
    let app = base_app.with_test_overrides(|services| services.with_imports(import_repo.clone()));

    let config =
        create_enabled_download_client_config(&app, &user, "Primary Weaver", "weaver").await;
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Reconciled Weaver Import".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create monitored movie title");

    let item_id = "weaver-swallowed-1";
    let download_id = "weaver-download-swallowed-1";
    let mut item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);
    item.client_id = config.id.clone();
    item.client_name = config.name.clone();
    item.client_type = "weaver".to_string();
    item.download_id = Some(download_id.to_string());
    item.title_id = Some(title.id.clone());
    item.title_name = title.name.clone();
    item.facet = Some("movie".to_string());

    let source_dir = tempfile::tempdir().expect("source tempdir");
    let mut completed = completed_download_fixture_item(
        item_id,
        &title.id,
        item.title_name.as_str(),
        source_dir.path().to_string_lossy().as_ref(),
    );
    completed.client_id = config.id.clone();
    completed.client_type = "weaver".to_string();
    completed.download_id = None;

    // The completion event was never delivered: no bridge delta is published.
    // Only the client's history knows about the item.
    *download_client.history_items.lock().await = vec![item.clone()];
    *download_client.recent_completed_downloads.lock().await = Some(vec![completed]);

    let (_command_tx, tracked_download_rx) = tokio::sync::mpsc::channel(8);
    let (_snapshot_tx, snapshot_rx) = tokio::sync::mpsc::channel(8);
    let token = tokio_util::sync::CancellationToken::new();
    let poller = tokio::spawn(
        crate::integration::start_download_queue_poller_with_options(
            app.clone(),
            token.child_token(),
            tracked_download_rx,
            snapshot_rx,
            crate::integration::DownloadQueuePollerOptions {
                interval: Duration::from_millis(50),
                excluded_client_types: vec!["weaver".to_string()],
            },
        ),
    );

    timeout(Duration::from_secs(5), async {
        loop {
            if import_repo
                .records
                .lock()
                .await
                .iter()
                .any(|record| record.source_ref == item_id && record.source_system == "weaver")
            {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("history reconciliation should import a completion the bridge never announced");

    assert!(
        !download_client
            .recent_activity_calls
            .lock()
            .await
            .is_empty(),
        "reconciliation should list the excluded client's recent history"
    );

    token.cancel();
    poller
        .await
        .expect("download queue poller should stop cleanly");
}

#[tokio::test]
async fn excluded_client_history_reconciliation_skips_stale_completions() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );
    let import_repo = Arc::new(TrackingImportRepo::default());
    let app = base_app.with_test_overrides(|services| services.with_imports(import_repo.clone()));

    let config =
        create_enabled_download_client_config(&app, &user, "Primary Weaver", "weaver").await;
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Stale Weaver Backlog".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create monitored movie title");

    let item_id = "weaver-stale-backlog-1";
    let mut item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);
    item.client_id = config.id.clone();
    item.client_name = config.name.clone();
    item.client_type = "weaver".to_string();
    item.title_id = Some(title.id.clone());
    item.title_name = title.name.clone();
    item.facet = Some("movie".to_string());

    let source_dir = tempfile::tempdir().expect("source tempdir");
    let mut completed = completed_download_fixture_item(
        item_id,
        &title.id,
        item.title_name.as_str(),
        source_dir.path().to_string_lossy().as_ref(),
    );
    completed.client_id = config.id.clone();
    completed.client_type = "weaver".to_string();
    completed.download_id = None;
    // Retained history from long before this Scryer version was deployed: the
    // sweep must leave it for an explicit backfill instead of importing it.
    completed.completed_at = Some(Utc::now() - chrono::Duration::days(30));

    *download_client.history_items.lock().await = vec![item];
    *download_client.recent_completed_downloads.lock().await = Some(vec![completed]);

    let (_command_tx, tracked_download_rx) = tokio::sync::mpsc::channel(8);
    let (_snapshot_tx, snapshot_rx) = tokio::sync::mpsc::channel(8);
    let token = tokio_util::sync::CancellationToken::new();
    let poller = tokio::spawn(
        crate::integration::start_download_queue_poller_with_options(
            app.clone(),
            token.child_token(),
            tracked_download_rx,
            snapshot_rx,
            crate::integration::DownloadQueuePollerOptions {
                interval: Duration::from_millis(50),
                excluded_client_types: vec!["weaver".to_string()],
            },
        ),
    );

    // Wait for the sweep to actually read the client's history, then give the
    // dispatch path time to act on it. The positive-path test shows an eligible
    // row dispatches within the same cycle, so a still-empty import repo here
    // means the age filter — not a missed sweep — kept it out.
    timeout(Duration::from_secs(5), async {
        loop {
            if !download_client
                .recent_activity_calls
                .lock()
                .await
                .is_empty()
            {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("reconciliation should list the excluded client's history");
    sleep(Duration::from_millis(500)).await;

    assert!(
        import_repo.records.lock().await.is_empty(),
        "a completion older than the reconcile window must not be auto-imported"
    );

    token.cancel();
    poller
        .await
        .expect("download queue poller should stop cleanly");
}

#[tokio::test]
async fn blocked_import_outcome_is_persisted_durably() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );

    let config =
        create_enabled_download_client_config(&app, &user, "Primary Weaver", "weaver").await;

    let (_command_tx, tracked_download_rx) = tokio::sync::mpsc::channel(8);
    let (snapshot_tx, snapshot_rx) = tokio::sync::mpsc::channel(8);
    let ingest = crate::tracked_downloads::TrackedDownloadSnapshotIngestHandle::new(snapshot_tx);
    let token = tokio_util::sync::CancellationToken::new();
    let poller = tokio::spawn(
        crate::integration::start_download_queue_poller_with_options(
            app.clone(),
            token.child_token(),
            tracked_download_rx,
            snapshot_rx,
            crate::integration::DownloadQueuePollerOptions {
                interval: Duration::from_secs(60),
                excluded_client_types: vec!["weaver".to_string()],
            },
        ),
    );

    let item_id = "weaver-foreign-junk-1";
    let download_id = "ext-dl-junk-1";
    let mut item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);
    item.client_id = config.id.clone();
    item.client_name = config.name.clone();
    item.client_type = "weaver".to_string();
    item.download_id = Some(download_id.to_string());
    item.title_id = None;
    item.title_name = "Zxqv Unknown Show S01E01".to_string();
    item.facet = None;
    item.is_scryer_origin = false;
    let tracked_id = crate::tracked_downloads::tracked_download_id_for_item(&item);

    let source_dir = tempfile::tempdir().expect("source tempdir");
    let mut completed = completed_download_fixture_item(
        item_id,
        "",
        item.title_name.as_str(),
        source_dir.path().to_string_lossy().as_ref(),
    );
    completed.client_id = config.id.clone();
    completed.client_type = "weaver".to_string();
    completed.download_id = Some(download_id.to_string());
    completed.parameters.clear();

    ingest
        .publish(crate::tracked_downloads::TrackedDownloadSnapshotUpdate {
            scope: crate::tracked_downloads::TrackedDownloadSnapshotScope::Delta,
            items: vec![item],
            completed_downloads: vec![completed],
            actor_id: None,
        })
        .await
        .expect("publish unmatched completed update");

    timeout(Duration::from_secs(5), async {
        loop {
            if app
                .runtime
                .acquisition
                .tracked_download_snapshot
                .read()
                .await
                .get(&tracked_id)
                .is_some_and(|metadata| metadata.state == TrackedDownloadState::ImportBlocked)
            {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("unmatched foreign completion should block for manual review");

    timeout(Duration::from_secs(5), async {
        loop {
            let tracked_recorded = download_submissions
                .tracked_states
                .lock()
                .await
                .values()
                .any(|state| state == "import_blocked");
            let identity_recorded = download_submissions
                .identity_states
                .lock()
                .await
                .values()
                .any(|state| state == "import_blocked");
            if tracked_recorded && identity_recorded {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("blocked outcome should be persisted to submissions and identity states");

    token.cancel();
    poller
        .await
        .expect("download queue poller should stop cleanly");
}

#[tokio::test]
async fn external_weaver_idless_missing_history_retries_and_dispatches_without_second_delta() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );
    let import_repo = Arc::new(TrackingImportRepo::default());
    let app = base_app.with_test_overrides(|services| services.with_imports(import_repo.clone()));

    let config =
        create_enabled_download_client_config(&app, &user, "Primary Weaver", "weaver").await;
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Idless Weaver Retry".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create monitored movie title");

    let (_command_tx, tracked_download_rx) = tokio::sync::mpsc::channel(8);
    let (snapshot_tx, snapshot_rx) = tokio::sync::mpsc::channel(8);
    let ingest = crate::tracked_downloads::TrackedDownloadSnapshotIngestHandle::new(snapshot_tx);
    let token = tokio_util::sync::CancellationToken::new();
    let poller = tokio::spawn(
        crate::integration::start_download_queue_poller_with_options(
            app.clone(),
            token.child_token(),
            tracked_download_rx,
            snapshot_rx,
            crate::integration::DownloadQueuePollerOptions {
                interval: Duration::from_millis(50),
                excluded_client_types: vec!["weaver".to_string()],
            },
        ),
    );

    let item_id = "weaver-idless-retry-1";
    let mut item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);
    item.client_id = config.id.clone();
    item.client_name = config.name.clone();
    item.client_type = "weaver".to_string();
    item.download_id = None;
    item.title_id = Some(title.id.clone());
    item.title_name = title.name.clone();
    item.facet = Some("movie".to_string());
    item.is_scryer_origin = false;
    let tracked_id = crate::tracked_downloads::tracked_download_id_for_item(&item);
    *download_client.recent_completed_downloads.lock().await = Some(Vec::new());

    ingest
        .publish(crate::tracked_downloads::TrackedDownloadSnapshotUpdate {
            scope: crate::tracked_downloads::TrackedDownloadSnapshotScope::Delta,
            items: vec![item.clone()],
            completed_downloads: Vec::new(),
            actor_id: None,
        })
        .await
        .expect("publish idless missing-history update");

    timeout(Duration::from_secs(5), async {
        loop {
            let snapshot = app
                .runtime
                .acquisition
                .tracked_download_snapshot
                .read()
                .await;
            let waiting = snapshot.get(&tracked_id).is_some_and(|metadata| {
                metadata.state == TrackedDownloadState::ImportPending
                    && metadata
                        .status_messages
                        .iter()
                        .any(|message| message.contains("waiting for client history"))
            });
            drop(snapshot);
            let retry_called = !download_client
                .recent_completed_download_calls
                .lock()
                .await
                .is_empty();
            if waiting && retry_called {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("idless completed item should wait for recent completed history");
    assert!(import_repo.records.lock().await.is_empty());

    let source_dir = tempfile::tempdir().expect("source tempdir");
    let mut completed = completed_download_fixture_item(
        item_id,
        &title.id,
        item.title_name.as_str(),
        source_dir.path().to_string_lossy().as_ref(),
    );
    completed.client_id = config.id.clone();
    completed.client_type = "weaver".to_string();
    completed.download_id = None;
    *download_client.recent_completed_downloads.lock().await = Some(vec![completed]);

    timeout(Duration::from_secs(5), async {
        loop {
            if import_repo
                .records
                .lock()
                .await
                .iter()
                .any(|record| record.source_ref == item_id && record.source_system == "weaver")
            {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("idless retry should revalidate and dispatch import without another delta");

    token.cancel();
    poller
        .await
        .expect("download queue poller should stop cleanly");
}

#[tokio::test]
async fn external_weaver_path_wait_retries_from_tracked_runtime_without_second_delta() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );
    let import_repo = Arc::new(TrackingImportRepo::default());
    let app = base_app.with_test_overrides(|services| services.with_imports(import_repo.clone()));

    let config =
        create_enabled_download_client_config(&app, &user, "Primary Weaver", "weaver").await;
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Path Wait Weaver Retry".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create monitored movie title");

    let (_command_tx, tracked_download_rx) = tokio::sync::mpsc::channel(8);
    let (snapshot_tx, snapshot_rx) = tokio::sync::mpsc::channel(8);
    let ingest = crate::tracked_downloads::TrackedDownloadSnapshotIngestHandle::new(snapshot_tx);
    let token = tokio_util::sync::CancellationToken::new();
    let poller = tokio::spawn(
        crate::integration::start_download_queue_poller_with_options(
            app.clone(),
            token.child_token(),
            tracked_download_rx,
            snapshot_rx,
            crate::integration::DownloadQueuePollerOptions {
                interval: Duration::from_millis(50),
                excluded_client_types: vec!["weaver".to_string()],
            },
        ),
    );

    let item_id = "weaver-path-wait-1";
    let mut item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);
    item.client_id = config.id.clone();
    item.client_name = config.name.clone();
    item.client_type = "weaver".to_string();
    item.download_id = None;
    item.title_id = Some(title.id.clone());
    item.title_name = title.name.clone();
    item.facet = Some("movie".to_string());
    item.is_scryer_origin = false;
    let tracked_id = crate::tracked_downloads::tracked_download_id_for_item(&item);
    *download_client.recent_completed_downloads.lock().await = Some(Vec::new());

    let missing_root = tempfile::tempdir().expect("missing root tempdir");
    let missing_path = missing_root.path().join("not-visible-yet");
    let mut missing_completed = completed_download_fixture_item(
        item_id,
        &title.id,
        item.title_name.as_str(),
        missing_path.to_string_lossy().as_ref(),
    );
    missing_completed.client_id = config.id.clone();
    missing_completed.client_type = "weaver".to_string();
    missing_completed.download_id = None;

    ingest
        .publish(crate::tracked_downloads::TrackedDownloadSnapshotUpdate {
            scope: crate::tracked_downloads::TrackedDownloadSnapshotScope::Delta,
            items: vec![item.clone()],
            completed_downloads: vec![missing_completed],
            actor_id: None,
        })
        .await
        .expect("publish path-wait completed update");

    timeout(Duration::from_secs(5), async {
        loop {
            if app
                .runtime
                .acquisition
                .tracked_download_snapshot
                .read()
                .await
                .get(&tracked_id)
                .is_some_and(|metadata| {
                    metadata.state == TrackedDownloadState::Downloading
                        && metadata.status_messages.iter().any(|message| {
                            message.contains("Completed download path is not available yet")
                        })
                })
            {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("completed item should enter path wait");
    assert!(import_repo.records.lock().await.is_empty());

    let source_dir = tempfile::tempdir().expect("source tempdir");
    let mut ready_completed = completed_download_fixture_item(
        item_id,
        &title.id,
        item.title_name.as_str(),
        source_dir.path().to_string_lossy().as_ref(),
    );
    ready_completed.client_id = config.id.clone();
    ready_completed.client_type = "weaver".to_string();
    ready_completed.download_id = None;
    *download_client.recent_completed_downloads.lock().await = Some(vec![ready_completed]);

    timeout(Duration::from_secs(5), async {
        loop {
            if import_repo
                .records
                .lock()
                .await
                .iter()
                .filter(|record| record.source_ref == item_id && record.source_system == "weaver")
                .count()
                == 1
            {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("path-wait retry should dispatch once path appears without second delta");

    assert_eq!(*download_client.completed_download_calls.lock().await, 0);

    token.cancel();
    poller
        .await
        .expect("download queue poller should stop cleanly");
}

#[tokio::test]
async fn external_weaver_idless_bad_foreign_item_blocks_after_history_retry() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );

    let config =
        create_enabled_download_client_config(&app, &user, "Primary Weaver", "weaver").await;

    let (_command_tx, tracked_download_rx) = tokio::sync::mpsc::channel(8);
    let (snapshot_tx, snapshot_rx) = tokio::sync::mpsc::channel(8);
    let ingest = crate::tracked_downloads::TrackedDownloadSnapshotIngestHandle::new(snapshot_tx);
    let token = tokio_util::sync::CancellationToken::new();
    let poller = tokio::spawn(
        crate::integration::start_download_queue_poller_with_options(
            app.clone(),
            token.child_token(),
            tracked_download_rx,
            snapshot_rx,
            crate::integration::DownloadQueuePollerOptions {
                interval: Duration::from_millis(50),
                excluded_client_types: vec!["weaver".to_string()],
            },
        ),
    );

    let item_id = "weaver-idless-bad-1";
    let mut item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);
    item.client_id = config.id.clone();
    item.client_name = config.name.clone();
    item.client_type = "weaver".to_string();
    item.download_id = None;
    item.title_id = None;
    item.title_name = "Unmatched.Foreign.Download.2026.1080p".to_string();
    item.facet = None;
    item.is_scryer_origin = false;
    let tracked_id = crate::tracked_downloads::tracked_download_id_for_item(&item);
    *download_client.recent_completed_downloads.lock().await = Some(Vec::new());

    ingest
        .publish(crate::tracked_downloads::TrackedDownloadSnapshotUpdate {
            scope: crate::tracked_downloads::TrackedDownloadSnapshotScope::Delta,
            items: vec![item.clone()],
            completed_downloads: Vec::new(),
            actor_id: None,
        })
        .await
        .expect("publish unsafe idless missing-history update");

    timeout(Duration::from_secs(5), async {
        loop {
            if app
                .runtime
                .acquisition
                .tracked_download_snapshot
                .read()
                .await
                .get(&tracked_id)
                .is_some_and(|metadata| {
                    metadata.state == TrackedDownloadState::ImportPending
                        && metadata
                            .status_messages
                            .iter()
                            .any(|message| message.contains("waiting for client history"))
                })
            {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("unsafe idless item should wait for history first");

    let source_dir = tempfile::tempdir().expect("source tempdir");
    let mut completed = completed_download_fixture_item(
        item_id,
        "",
        item.title_name.as_str(),
        source_dir.path().to_string_lossy().as_ref(),
    );
    completed.client_id = config.id.clone();
    completed.client_type = "weaver".to_string();
    completed.download_id = None;
    completed.category = Some("foreign".to_string());
    completed.parameters.clear();
    *download_client.recent_completed_downloads.lock().await = Some(vec![completed]);

    timeout(Duration::from_secs(5), async {
        loop {
            if app
                .runtime
                .acquisition
                .tracked_download_snapshot
                .read()
                .await
                .get(&tracked_id)
                .is_some_and(|metadata| {
                    metadata.state == TrackedDownloadState::ImportBlocked
                        && metadata
                            .status_messages
                            .iter()
                            .any(|message| message.contains("couldn't be matched"))
                })
            {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("unsafe idless item should revalidate to manual block, not auto-import");

    assert_eq!(*download_client.completed_download_calls.lock().await, 0);

    token.cancel();
    poller
        .await
        .expect("download queue poller should stop cleanly");
}

#[tokio::test]
async fn failed_tracked_cleanup_uses_facet_routing_and_exact_client_id() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );

    let config =
        create_enabled_download_client_config(&app, &user, "Series NZBGet", "nzbget").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, false, true).await;

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Failed Cleanup".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create monitored movie title");

    let item_id = "failed-cleanup-1";
    let mut history_item = queue_history_fixture_item(item_id, DownloadQueueState::Failed, 40);
    history_item.client_id = config.id.clone();
    history_item.client_name = config.name.clone();
    history_item.title_id = Some(title.id.clone());
    history_item.title_name = title.name.clone();
    history_item.facet = Some("movie".to_string());
    let tracked = crate::tracked_downloads::TrackedDownload {
        id: crate::tracked_downloads::tracked_download_id(
            Some(config.id.as_str()),
            "nzbget",
            item_id,
        ),
        client_id: config.id.clone(),
        client_type: "nzbget".to_string(),
        client_item: history_item,
        state: TrackedDownloadState::Failed,
        status: scryer_domain::TrackedDownloadStatus::Ok,
        status_messages: Vec::new(),
        title_id: Some(title.id.clone()),
        facet: Some("movie".to_string()),
        source_title: Some(title.name.clone()),
        indexer: None,
        added_at: None,
        notified_manual_interaction: false,
        match_type: scryer_domain::TitleMatchType::Submission,
        is_trackable: true,
        import_attempted: true,
        waiting_for_completed_history: false,
        path_missing_since: None,
        no_video_import_retry: None,
        foreign_import_classification: None,
        skip_reacquire_on_failure: false,
    };

    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::Failed,
    )
    .await;

    assert_eq!(
        outcome,
        crate::import::import::TerminalDownloadCleanupOutcome::Removed
    );
    assert_eq!(
        download_client.deleted_requests.lock().await.clone(),
        vec![(Some(config.id.clone()), None, item_id.to_string(), true)]
    );
}

#[tokio::test]
async fn try_import_completed_downloads_removes_already_imported_history_with_exact_client_id() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );
    let import_repo = Arc::new(TrackingImportRepo::default());
    let app = base_app.with_test_overrides(|services| services.with_imports(import_repo.clone()));

    let config =
        create_enabled_download_client_config(&app, &user, "Primary NZBGet", "nzbget").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, false).await;

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Legacy Cleanup".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create monitored movie title");

    let now = Utc::now().to_rfc3339();
    let item_id = "legacy-completed-1";
    download_submissions
        .record_submission(DownloadSubmission {
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "movie".to_string(),
            download_client_id: Some(config.id.clone()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: item_id.to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Legacy.Cleanup.2026.1080p.WEB-DL".to_string()),
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("seed compatible legacy submission");
    import_repo.records.lock().await.push(ImportRecord {
        id: Id::new().0,
        source_client_id: Some(config.id.clone()),
        source_system: "nzbget".to_string(),
        source_ref: item_id.to_string(),
        import_type: ImportType::MovieDownload,
        status: ImportStatus::Completed,
        payload_json: String::new(),
        result_json: None,
        download_id: None,
        import_transfer_phase: None,
        import_transfer_bytes: None,
        import_transfer_total_bytes: None,
        import_transfer_started_at: None,
        import_transfer_updated_at: None,
        started_at: Some(now.clone()),
        finished_at: Some(now.clone()),
        created_at: now.clone(),
        updated_at: now,
    });

    let mut item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);
    item.client_id = config.id.clone();
    item.client_name = config.name.clone();
    item.title_id = Some(title.id.clone());
    item.title_name = title.name.clone();
    item.facet = Some("movie".to_string());

    let dir = tempfile::tempdir().expect("tempdir");
    let mut completed = completed_download_fixture_item(
        item_id,
        &title.id,
        "Legacy.Cleanup.2026.1080p.WEB-DL",
        dir.path().to_string_lossy().as_ref(),
    );
    completed.client_id = config.id.clone();
    *download_client.completed_downloads.lock().await = vec![completed];

    let processed =
        crate::import::import::try_import_completed_downloads(&app, &user, &[item]).await;

    assert!(processed.contains(item_id));
    assert_eq!(
        download_client.deleted_requests.lock().await.clone(),
        vec![(Some(config.id.clone()), None, item_id.to_string(), true)]
    );
}

#[tokio::test]
async fn try_import_completed_downloads_leaves_already_imported_item_unprocessed_when_completed_download_is_missing()
 {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );
    let import_repo = Arc::new(TrackingImportRepo::default());
    let app = base_app.with_test_overrides(|services| services.with_imports(import_repo.clone()));

    let now = Utc::now().to_rfc3339();
    let item_id = "legacy-missing-completed-1";
    import_repo.records.lock().await.push(ImportRecord {
        id: Id::new().0,
        source_client_id: Some("client-1".to_string()),
        source_system: "nzbget".to_string(),
        source_ref: item_id.to_string(),
        import_type: ImportType::MovieDownload,
        status: ImportStatus::Completed,
        payload_json: String::new(),
        result_json: None,
        download_id: None,
        import_transfer_phase: None,
        import_transfer_bytes: None,
        import_transfer_total_bytes: None,
        import_transfer_started_at: None,
        import_transfer_updated_at: None,
        started_at: Some(now.clone()),
        finished_at: Some(now.clone()),
        created_at: now.clone(),
        updated_at: now,
    });

    let item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);

    let processed =
        crate::import::import::try_import_completed_downloads(&app, &user, &[item]).await;

    assert!(!processed.contains(item_id));
    assert!(download_client.deleted_requests.lock().await.is_empty());
}

#[tokio::test]
async fn try_import_recent_completed_downloads_uses_recent_lookup_and_preserves_processed_ids() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );

    let item_id = "recent-completed-processed";
    let mut item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);
    item.client_id = "weaver-client".to_string();
    item.client_type = "weaver".to_string();

    let dir = tempfile::tempdir().expect("tempdir");
    let mut completed = completed_download_fixture_item(
        item_id,
        "title-current",
        "Recent.Completed.2026.1080p.WEB-DL",
        dir.path().to_string_lossy().as_ref(),
    );
    completed.client_id = item.client_id.clone();
    completed.client_type = item.client_type.clone();
    completed.parameters.clear();
    *download_client.recent_completed_downloads.lock().await = Some(vec![completed]);

    let processed =
        crate::import::import::try_import_recent_completed_downloads(&app, &user, &[item]).await;

    assert!(processed.contains(item_id));
    assert_eq!(*download_client.completed_download_calls.lock().await, 0);
    assert_eq!(
        download_client
            .recent_completed_download_calls
            .lock()
            .await
            .clone(),
        vec![crate::DOWNLOAD_QUEUE_RECENT_COMPLETED_LIMIT]
    );
}

#[tokio::test]
async fn try_import_provided_completed_downloads_uses_provided_rows_and_preserves_processed_ids() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );

    let item_id = "targeted-completed-processed";
    let mut item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);
    item.client_id = "weaver-client".to_string();
    item.client_type = "weaver".to_string();

    let dir = tempfile::tempdir().expect("tempdir");
    let mut completed = completed_download_fixture_item(
        item_id,
        "title-current",
        "Targeted.Completed.2026.1080p.WEB-DL",
        dir.path().to_string_lossy().as_ref(),
    );
    completed.client_id = item.client_id.clone();
    completed.client_type = item.client_type.clone();
    completed.parameters.clear();
    let processed = crate::import::import::try_import_provided_completed_downloads(
        &app,
        &user,
        &[item],
        vec![completed],
    )
    .await;

    assert!(processed.contains(item_id));
    assert_eq!(*download_client.completed_download_calls.lock().await, 0);
    assert!(
        download_client
            .recent_completed_download_calls
            .lock()
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn try_import_recent_completed_downloads_defers_missing_recent_history_without_blocking() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );

    let item_id = "recent-missing-completed";
    let download_id = "scryer-download:recent-missing";
    let mut item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);
    item.client_id = "weaver-client".to_string();
    item.client_type = "weaver".to_string();
    item.download_id = Some(download_id.to_string());
    let mut full_history_completed = completed_download_fixture_item(
        item_id,
        "title-current",
        "Recent.Missing.2026.1080p.WEB-DL",
        "/tmp/would-have-been-found-by-full-history",
    );
    full_history_completed.client_id = item.client_id.clone();
    full_history_completed.client_type = item.client_type.clone();
    *download_client.completed_downloads.lock().await = vec![full_history_completed];
    *download_client.recent_completed_downloads.lock().await = Some(Vec::new());

    let processed =
        crate::import::import::try_import_recent_completed_downloads(&app, &user, &[item]).await;

    assert!(!processed.contains(item_id));
    assert_eq!(*download_client.completed_download_calls.lock().await, 0);
    assert_eq!(
        download_client
            .recent_completed_download_calls
            .lock()
            .await
            .clone(),
        vec![crate::DOWNLOAD_QUEUE_RECENT_COMPLETED_LIMIT]
    );
    let state = download_submissions
        .get_identity_tracked_state(
            &DownloadSubmissionIdentity {
                download_id: Some(download_id.to_string()),
            },
            Some(&DownloadSourceIdentity::new(
                Some("weaver-client"),
                "weaver",
                item_id,
            )),
        )
        .await
        .expect("identity state lookup");
    assert!(state.is_none());
}

#[tokio::test]
async fn try_import_provided_completed_downloads_defers_missing_history_without_blocking() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );

    let item_id = "targeted-missing-completed";
    let download_id = "scryer-download:targeted-missing";
    let mut item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);
    item.client_id = "weaver-client".to_string();
    item.client_type = "weaver".to_string();
    item.download_id = Some(download_id.to_string());

    let mut full_history_completed = completed_download_fixture_item(
        item_id,
        "title-current",
        "Targeted.Missing.2026.1080p.WEB-DL",
        "/tmp/would-have-been-found-by-full-history",
    );
    full_history_completed.client_id = item.client_id.clone();
    full_history_completed.client_type = item.client_type.clone();
    *download_client.completed_downloads.lock().await = vec![full_history_completed];

    let processed = crate::import::import::try_import_provided_completed_downloads(
        &app,
        &user,
        &[item],
        vec![],
    )
    .await;

    assert!(!processed.contains(item_id));
    assert_eq!(*download_client.completed_download_calls.lock().await, 0);
    assert!(
        download_client
            .recent_completed_download_calls
            .lock()
            .await
            .is_empty()
    );
    let state = download_submissions
        .get_identity_tracked_state(
            &DownloadSubmissionIdentity {
                download_id: Some(download_id.to_string()),
            },
            Some(&DownloadSourceIdentity::new(
                Some("weaver-client"),
                "weaver",
                item_id,
            )),
        )
        .await
        .expect("identity state lookup");
    assert!(state.is_none());
}

#[tokio::test]
async fn try_import_completed_downloads_still_blocks_missing_full_history_identity() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );

    let item_id = "full-missing-completed";
    let download_id = "scryer-download:full-missing";
    let mut item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);
    item.client_id = "weaver-client".to_string();
    item.client_type = "weaver".to_string();
    item.download_id = Some(download_id.to_string());

    let processed =
        crate::import::import::try_import_completed_downloads(&app, &user, &[item]).await;

    assert!(!processed.contains(item_id));
    assert_eq!(*download_client.completed_download_calls.lock().await, 1);
    assert!(
        download_client
            .recent_completed_download_calls
            .lock()
            .await
            .is_empty()
    );
    let state = download_submissions
        .get_identity_tracked_state(
            &DownloadSubmissionIdentity {
                download_id: Some(download_id.to_string()),
            },
            Some(&DownloadSourceIdentity::new(
                Some("weaver-client"),
                "weaver",
                item_id,
            )),
        )
        .await
        .expect("identity state lookup");
    assert_eq!(
        state.as_deref(),
        Some(TrackedDownloadState::ImportBlocked.as_str())
    );
}

#[tokio::test]
async fn import_completed_download_ignores_stale_item_id_import_when_request_identity_is_fresh() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client,
        download_submissions.clone(),
        pending_releases,
    );
    let import_repo = Arc::new(TrackingImportRepo::default());
    let app = base_app.with_test_overrides(|services| services.with_imports(import_repo.clone()));

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Fresh Identity".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let item_id = "10010";
    let client_id = "weaver-client";
    let now = Utc::now().to_rfc3339();
    import_repo.records.lock().await.push(ImportRecord {
        id: Id::new().0,
        source_client_id: Some(client_id.to_string()),
        source_system: "weaver".to_string(),
        source_ref: item_id.to_string(),
        import_type: ImportType::MovieDownload,
        status: ImportStatus::Completed,
        payload_json: String::new(),
        result_json: None,
        download_id: None,
        import_transfer_phase: None,
        import_transfer_bytes: None,
        import_transfer_total_bytes: None,
        import_transfer_started_at: None,
        import_transfer_updated_at: None,
        started_at: Some(now.clone()),
        finished_at: Some(now.clone()),
        created_at: now.clone(),
        updated_at: now,
    });

    download_submissions
        .record_submission_with_identity(
            DownloadSubmission {
                title_id: title.id.clone(),
                purpose: crate::DownloadSubmissionPurpose::Standard,
                facet: "movie".to_string(),
                download_client_id: Some(client_id.to_string()),
                download_client_type: "weaver".to_string(),
                download_client_item_id: item_id.to_string(),
                source_hint: None,
                source_provider_id: None,
                source_provider_name: None,
                source_kind: None,
                source_title: Some("Fresh.Identity.2026.1080p.WEB-DL".to_string()),
                request_signature: None,
                scope: SubmissionScope::Title,
            },
            DownloadSubmissionIdentity {
                download_id: Some("scryer-download:fresh".to_string()),
            },
        )
        .await
        .expect("record fresh identity");

    let dir = tempfile::tempdir().expect("tempdir");
    let mut completed = completed_download_fixture_item(
        item_id,
        &title.id,
        "Fresh.Identity.2026.1080p.WEB-DL",
        dir.path().to_string_lossy().as_ref(),
    );
    completed.client_id = client_id.to_string();
    completed.client_type = "weaver".to_string();
    completed.parameters.push((
        "*scryer_download_id".to_string(),
        "scryer-download:fresh".to_string(),
    ));

    let result = crate::import::import::import_completed_download(&app, &user, &completed)
        .await
        .expect("completed import should run");

    assert_ne!(result.skip_reason, Some(ImportSkipReason::AlreadyImported));
    assert_eq!(result.skip_reason, Some(ImportSkipReason::NoVideoFiles));
}

#[tokio::test]
async fn import_series_duplicate_destination_requires_catalog_for_already_imported() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) =
        bootstrap_with_cleanup_tracking(download_client, download_submissions, pending_releases);
    let media_files = Arc::new(MockMediaFileRepo::default());
    let import_repo = Arc::new(TrackingImportRepo::default());
    let app = base_app.with_test_overrides(|services| {
        services
            .with_imports(import_repo)
            .with_file_importer(Arc::new(CopyingFileImporter))
            .with_media_files(media_files)
    });

    let library_dir = tempfile::tempdir().expect("library tempdir");
    let title_folder = library_dir.path().join("Import Series Skip");
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Import Series Skip".to_string(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create series title");
    app.services
        .catalog
        .titles
        .set_folder_path(&title.id, &title_folder.to_string_lossy())
        .await
        .expect("set title folder path");
    let collection = app
        .create_collection(
            &user,
            title.id.clone(),
            "season".into(),
            "1".into(),
            Some("Season 1".into()),
            None,
            Some("1".into()),
            Some("1".into()),
        )
        .await
        .expect("create season collection");
    let episode = app
        .create_episode(
            &user,
            title.id.clone(),
            Some(collection.id),
            "standard".into(),
            Some("1".into()),
            Some("1".into()),
            Some("S01E01".into()),
            Some("Pilot".into()),
            None,
            Some(1_500),
            false,
            false,
        )
        .await
        .expect("create episode");

    let source_root = tempfile::tempdir().expect("source tempdir");
    let source_dir = source_root.path().join("download");
    tokio::fs::create_dir_all(&source_dir)
        .await
        .expect("create source directory");
    let release_name = "Import.Series.Skip.S01E01.1080p.WEB-DL";
    let source_path = source_dir.join(format!("{release_name}.mkv"));
    let duplicate_size = 128 * 1024 * 1024;
    std::fs::File::create(&source_path)
        .and_then(|file| file.set_len(duplicate_size))
        .expect("write source video");

    let stored_title = app
        .services
        .catalog
        .titles
        .get_by_id(&title.id)
        .await
        .expect("load stored title")
        .expect("stored title exists");
    let path_settings = crate::import_workflow::resolve_import_paths(&app, &stored_title)
        .await
        .expect("resolve import paths");
    let parsed = crate::parse_release_metadata(release_name);
    let dest_path = crate::import_workflow::episode_import_dest_path(
        &stored_title,
        &parsed,
        "mkv",
        &source_path,
        &title_folder,
        path_settings.rename_enabled,
        &path_settings.rename_template,
        &path_settings.season_folder_template,
        &path_settings.specials_folder_template,
        1,
        "1",
        None,
        episode.title.as_deref(),
        parsed.quality.as_deref(),
    );
    tokio::fs::create_dir_all(dest_path.parent().expect("destination parent"))
        .await
        .expect("create destination parent");
    std::fs::File::create(&dest_path)
        .and_then(|file| file.set_len(duplicate_size))
        .expect("write duplicate destination");

    let mut completed = completed_download_fixture_item(
        "series-skip-1",
        &title.id,
        release_name,
        &source_path.to_string_lossy(),
    );
    completed.client_type = "weaver".to_string();
    completed.client_id = "weaver-client".to_string();
    completed.category = Some("series".to_string());
    completed.parameters = vec![
        ("*scryer_title_id".to_string(), title.id.clone()),
        ("*scryer_facet".to_string(), "series".to_string()),
    ];

    let result = crate::import::import::import_completed_download(&app, &user, &completed)
        .await
        .expect("completed series import should run");

    assert_eq!(
        result.decision,
        scryer_domain::ImportDecision::Skipped,
        "unexpected import result: {result:?}"
    );
    assert_eq!(
        result.episode_ids,
        vec![episode.id.clone()],
        "unexpected import result: {result:?}"
    );
    assert_eq!(result.skip_reason, Some(ImportSkipReason::DuplicateFile));
    assert!(
        result
            .error_message
            .as_deref()
            .is_some_and(|message| message.contains("destination exists with identical size")),
        "expected duplicate destination message, got {:?}",
        result.error_message
    );

    let retry_result = crate::import::import::import_completed_download(&app, &user, &completed)
        .await
        .expect("uncataloged duplicate retry should run");
    assert_eq!(
        retry_result.skip_reason,
        Some(ImportSkipReason::DuplicateFile),
        "uncataloged duplicate retry should not become already imported: {retry_result:?}"
    );

    app.services
        .library
        .media_files
        .insert_media_file(&crate::InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: crate::stored_paths::path_to_stored_string(&dest_path),
            size_bytes: duplicate_size as i64,
            role: crate::MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .expect("insert catalog row for duplicate destination");

    let mut cataloged_completed = completed.clone();
    cataloged_completed.download_client_item_id = "series-skip-2".to_string();
    let cataloged_result =
        crate::import::import::import_completed_download(&app, &user, &cataloged_completed)
            .await
            .expect("cataloged completed series import should run");

    assert_eq!(
        cataloged_result.decision,
        scryer_domain::ImportDecision::Skipped,
        "unexpected import result: {cataloged_result:?}"
    );
    assert_eq!(
        cataloged_result.episode_ids,
        vec![episode.id.clone()],
        "unexpected import result: {cataloged_result:?}"
    );
    assert_eq!(
        cataloged_result.skip_reason,
        Some(ImportSkipReason::AlreadyImported)
    );
}

/// Every video file that landed under `root`, so a test can prove that a
/// rejected pack member never reached the library.
fn library_video_file_names(root: &Path) -> Vec<String> {
    let mut names = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("mkv")
                && let Some(name) = path.file_name().and_then(|name| name.to_str())
            {
                names.push(name.to_string());
            }
        }
    }
    names
}

struct FailClosedPackFixture {
    app: AppUseCase,
    user: User,
    title: scryer_domain::Title,
    episode: Episode,
    library_dir: tempfile::TempDir,
    import_repo: Arc<TrackingImportRepo>,
    import_artifacts: Arc<RecordingImportArtifactRepo>,
}

/// A monitored series with exactly one catalogued episode (S01E01), wired to
/// recording import repositories so pack members can be asserted one by one.
async fn fail_closed_pack_fixture() -> FailClosedPackFixture {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) =
        bootstrap_with_cleanup_tracking(download_client, download_submissions, pending_releases);
    let import_repo = Arc::new(TrackingImportRepo::default());
    let import_artifacts = Arc::new(RecordingImportArtifactRepo::default());
    let app = base_app.with_test_overrides(|services| {
        services
            .with_imports(import_repo.clone())
            .with_import_artifacts(import_artifacts.clone())
            .with_file_importer(Arc::new(CopyingFileImporter))
            .with_media_files(Arc::new(MockMediaFileRepo::default()))
    });
    app.services
        .identity
        .users
        .create(user.clone())
        .await
        .expect("seed import actor");

    let library_dir = tempfile::tempdir().expect("library tempdir");
    let title_folder = library_dir.path().join("Fail Closed Pack");
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Fail Closed Pack".to_string(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create series title");
    app.services
        .catalog
        .titles
        .set_folder_path(&title.id, &title_folder.to_string_lossy())
        .await
        .expect("set title folder path");
    let collection = app
        .create_collection(
            &user,
            title.id.clone(),
            "season".into(),
            "1".into(),
            Some("Season 1".into()),
            None,
            Some("1".into()),
            Some("1".into()),
        )
        .await
        .expect("create season collection");
    let episode = app
        .create_episode(
            &user,
            title.id.clone(),
            Some(collection.id),
            "standard".into(),
            Some("1".into()),
            Some("1".into()),
            Some("S01E01".into()),
            Some("Pilot".into()),
            None,
            Some(1_500),
            false,
            false,
        )
        .await
        .expect("create episode");

    FailClosedPackFixture {
        app,
        user,
        title,
        episode,
        library_dir,
        import_repo,
        import_artifacts,
    }
}

fn write_pack_video(dir: &Path, file_name: &str) -> std::path::PathBuf {
    let path = dir.join(file_name);
    std::fs::File::create(&path)
        .expect("create source video")
        .set_len(51 * 1024 * 1024)
        .expect("size source video above sample threshold");
    path
}

fn series_pack_completed_download(
    item_id: &str,
    title_id: &str,
    release_name: &str,
    source_dir: &Path,
) -> CompletedDownload {
    let mut completed = completed_download_fixture_item(
        item_id,
        title_id,
        release_name,
        &source_dir.to_string_lossy(),
    );
    completed.category = Some("series".to_string());
    completed.parameters = vec![
        ("*scryer_title_id".to_string(), title_id.to_string()),
        ("*scryer_facet".to_string(), "series".to_string()),
    ];
    completed
}

#[tokio::test]
async fn automatic_season_pack_import_rejects_member_without_a_matching_episode() {
    let FailClosedPackFixture {
        app,
        user,
        title,
        episode,
        library_dir,
        import_repo,
        import_artifacts,
    } = fail_closed_pack_fixture().await;

    let source_dir = tempfile::tempdir().expect("source tempdir");
    let matched_file = write_pack_video(
        source_dir.path(),
        "Fail.Closed.Pack.S01E01.1080p.WEB-DL.mkv",
    );
    let unmatched_file = write_pack_video(
        source_dir.path(),
        "Fail.Closed.Pack.S02E09.1080p.WEB-DL.mkv",
    );

    let completed = series_pack_completed_download(
        "fail-closed-pack-1",
        &title.id,
        "Fail.Closed.Pack.S01.1080p.WEB-DL",
        source_dir.path(),
    );
    let result = crate::import::import::import_completed_download(&app, &user, &completed)
        .await
        .expect("completed season pack import should run");

    // The unmatched member is rejected on its own, without transfer, and
    // without any record that could later be resolved as a library file.
    assert!(
        unmatched_file.exists(),
        "rejected file must stay in the completed-download directory"
    );
    let library_files = library_video_file_names(library_dir.path());
    assert!(
        library_files.iter().all(|name| !name.contains("S02E09")),
        "unmatched episode reached the library: {library_files:?}"
    );

    let media_files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    assert!(
        media_files
            .iter()
            .all(|file| !file.file_path.contains("S02E09")),
        "unmatched episode was catalogued: {media_files:?}"
    );
    assert!(
        media_files.iter().all(|file| file.episode_id.is_some()),
        "unlinked media file was created: {media_files:?}"
    );

    let rejected_artifacts = import_artifacts
        .artifacts_for_file("fail.closed.pack.s02e09.1080p.web-dl.mkv")
        .await;
    assert_eq!(
        rejected_artifacts.len(),
        1,
        "unexpected artifacts: {rejected_artifacts:?}"
    );
    assert_eq!(rejected_artifacts[0].result, "rejected");
    assert_eq!(
        rejected_artifacts[0].reason_code.as_deref(),
        Some("episode_not_found_for_title")
    );
    assert_eq!(rejected_artifacts[0].episode_id, None);
    assert_eq!(rejected_artifacts[0].imported_media_file_id, None);

    // Partial-pack behaviour: the catalogued episode still imports, because the
    // rejection is per file, not per pack. Synthetic pack members cannot be
    // probed, so with `runtime-media-analysis` enabled the matched file is
    // rejected by the sample gate for an unrelated reason and only the
    // fail-closed assertions above apply.
    if cfg!(not(feature = "runtime-media-analysis")) {
        assert_eq!(
            result.decision,
            scryer_domain::ImportDecision::Imported,
            "unexpected import result: {result:?}"
        );
        assert_eq!(
            result.episode_ids,
            vec![episode.id.clone()],
            "unexpected import result: {result:?}"
        );
        assert!(
            result.error_message.as_deref().is_some_and(|message| {
                message.contains("1 imported, 0 skipped, 1 rejected, 0 failed")
            }),
            "expected partial pack summary, got {:?}",
            result.error_message
        );
        assert_eq!(
            library_files.len(),
            1,
            "matched episode should be the only library file: {library_files:?}"
        );
        assert!(
            library_files[0].contains("S01E01"),
            "unexpected library file: {library_files:?}"
        );
        assert_eq!(
            media_files.len(),
            1,
            "unexpected media files: {media_files:?}"
        );
        assert_eq!(
            media_files[0].episode_id.as_deref(),
            Some(episode.id.as_str()),
            "unexpected media files: {media_files:?}"
        );

        let imported_artifacts = import_artifacts
            .artifacts_for_file("fail.closed.pack.s01e01.1080p.web-dl.mkv")
            .await;
        assert_eq!(
            imported_artifacts.len(),
            1,
            "unexpected artifacts: {imported_artifacts:?}"
        );
        assert_eq!(imported_artifacts[0].result, "imported");
        assert_eq!(
            imported_artifacts[0].episode_id.as_deref(),
            Some(episode.id.as_str())
        );

        // A rejected pack member must not leave the import pending for a later
        // sweep to pick up again.
        let statuses: Vec<ImportStatus> = import_repo
            .records
            .lock()
            .await
            .iter()
            .map(|record| record.status)
            .collect();
        assert_eq!(statuses, vec![ImportStatus::Completed]);
    }
    assert!(matched_file.exists());
}

#[tokio::test]
async fn automatic_import_rejects_download_whose_only_file_matches_no_episode() {
    let FailClosedPackFixture {
        app,
        user,
        title,
        library_dir,
        import_repo,
        import_artifacts,
        ..
    } = fail_closed_pack_fixture().await;

    let source_dir = tempfile::tempdir().expect("source tempdir");
    let unmatched_file = write_pack_video(
        source_dir.path(),
        "Fail.Closed.Pack.S02E09.1080p.WEB-DL.mkv",
    );

    let completed = series_pack_completed_download(
        "fail-closed-pack-2",
        &title.id,
        "Fail.Closed.Pack.S02E09.1080p.WEB-DL",
        source_dir.path(),
    );
    let result = crate::import::import::import_completed_download(&app, &user, &completed)
        .await
        .expect("completed unmatched import should run");

    assert_eq!(
        result.decision,
        scryer_domain::ImportDecision::Rejected,
        "unexpected import result: {result:?}"
    );
    assert_eq!(result.skip_reason, Some(ImportSkipReason::PolicyMismatch));
    assert!(
        result
            .error_message
            .as_deref()
            .is_some_and(|message| message.contains("file resolves to no episode of this title")),
        "expected fail-closed rejection message, got {:?}",
        result.error_message
    );
    assert!(
        result.episode_ids.is_empty(),
        "unexpected result: {result:?}"
    );

    assert!(unmatched_file.exists(), "rejected file must not be moved");
    assert!(
        library_video_file_names(library_dir.path()).is_empty(),
        "no file may reach the library"
    );
    assert!(
        app.services
            .library
            .media_files
            .list_media_files_for_title(&title.id)
            .await
            .expect("list media files")
            .is_empty()
    );

    let rejected_artifacts = import_artifacts
        .artifacts_for_file("fail.closed.pack.s02e09.1080p.web-dl.mkv")
        .await;
    assert_eq!(
        rejected_artifacts.len(),
        1,
        "unexpected artifacts: {rejected_artifacts:?}"
    );
    assert_eq!(rejected_artifacts[0].result, "rejected");
    assert_eq!(
        rejected_artifacts[0].reason_code.as_deref(),
        Some("episode_not_found_for_title")
    );

    // Terminal: the rejection is permanent, so nothing stays pending.
    let statuses: Vec<ImportStatus> = import_repo
        .records
        .lock()
        .await
        .iter()
        .map(|record| record.status)
        .collect();
    assert_eq!(statuses, vec![ImportStatus::Failed]);
}

#[tokio::test]
async fn manual_import_still_accepts_file_whose_name_matches_no_episode() {
    // Manual imports resolve the target from the operator's mapping, not from
    // the file name, so the automatic fail-closed rejection must not reach them.
    let FailClosedPackFixture {
        app,
        user,
        title,
        episode,
        ..
    } = fail_closed_pack_fixture().await;

    let source_dir = tempfile::tempdir().expect("source tempdir");
    let source_file = write_pack_video(
        source_dir.path(),
        "Fail.Closed.Pack.S02E09.1080p.WEB-DL.mkv",
    );

    let results = crate::import_workflow::execute_manual_import(
        &app,
        &user,
        "manual-import-no-episode-match",
        &title.id,
        None,
        vec![ManualImportFileMapping {
            file_path: source_file.to_string_lossy().into_owned(),
            episode_id: Some(episode.id.clone()),
            series_movie_link_id: None,
            quality: Some("1080p".to_string()),
        }],
        Some(std::fs::canonicalize(source_dir.path()).expect("canonical source root")),
    )
    .await
    .expect("execute manual import");
    assert!(results.iter().all(|result| result.success), "{results:?}");

    let media_files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    assert_eq!(
        media_files.len(),
        1,
        "unexpected media files: {media_files:?}"
    );
    assert_eq!(
        media_files[0].episode_id.as_deref(),
        Some(episode.id.as_str())
    );
}

#[tokio::test]
async fn try_import_completed_downloads_blocks_ambiguous_download_id_instead_of_legacy_item_id() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );
    let app = base_app;

    let client_id = "weaver-client";
    let item_id = "10010";
    let mut item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);
    item.client_id = client_id.to_string();
    item.client_type = "weaver".to_string();
    item.title_id = Some("title-current".to_string());
    item.title_name = "Fresh Identity".to_string();
    item.facet = Some("movie".to_string());
    item.download_id = Some("scryer-download:ambiguous".to_string());

    let dir = tempfile::tempdir().expect("tempdir");
    let mut completed = completed_download_fixture_item(
        item_id,
        "title-current",
        "Fresh.Identity.2026.1080p.WEB-DL",
        dir.path().to_string_lossy().as_ref(),
    );
    completed.client_id = client_id.to_string();
    completed.client_type = "weaver".to_string();
    completed.parameters.clear();
    completed.parameters.push((
        "*scryer_download_id".to_string(),
        "scryer-download:ambiguous".to_string(),
    ));
    *download_client.completed_downloads.lock().await = vec![completed];

    for (submitted_item_id, submitted_title_id) in
        [(item_id, "title-current"), ("other-item", "title-other")]
    {
        download_submissions
            .record_submission_with_identity(
                DownloadSubmission {
                    title_id: submitted_title_id.to_string(),
                    purpose: crate::DownloadSubmissionPurpose::Standard,
                    facet: "movie".to_string(),
                    download_client_id: Some(client_id.to_string()),
                    download_client_type: "weaver".to_string(),
                    download_client_item_id: submitted_item_id.to_string(),
                    source_hint: None,
                    source_provider_id: None,
                    source_provider_name: None,
                    source_kind: None,
                    source_title: Some("Fresh.Identity.2026.1080p.WEB-DL".to_string()),
                    request_signature: None,
                    scope: SubmissionScope::Title,
                },
                DownloadSubmissionIdentity {
                    download_id: Some("scryer-download:ambiguous".to_string()),
                },
            )
            .await
            .expect("record ambiguous identity");
    }

    let processed =
        crate::import::import::try_import_completed_downloads(&app, &user, &[item]).await;

    assert!(!processed.contains(item_id));
    assert!(download_client.deleted_requests.lock().await.is_empty());
    let state = download_submissions
        .get_identity_tracked_state(
            &DownloadSubmissionIdentity {
                download_id: Some("scryer-download:ambiguous".to_string()),
            },
            None,
        )
        .await
        .expect("identity state lookup");
    assert_eq!(
        state.as_deref(),
        Some(TrackedDownloadState::ImportBlocked.as_str())
    );
}

#[tokio::test]
async fn try_import_completed_downloads_blocks_missing_download_id_submission() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );

    let client_id = "weaver-client";
    let item_id = "missing-durable-identity";
    let mut item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);
    item.client_id = client_id.to_string();
    item.client_type = "weaver".to_string();
    item.title_id = Some("title-current".to_string());
    item.title_name = "Missing Durable".to_string();
    item.facet = Some("movie".to_string());

    let dir = tempfile::tempdir().expect("tempdir");
    let mut completed = completed_download_fixture_item(
        item_id,
        "title-current",
        "Missing.Durable.2026.1080p.WEB-DL",
        dir.path().to_string_lossy().as_ref(),
    );
    completed.client_id = client_id.to_string();
    completed.client_type = "weaver".to_string();
    completed.parameters.push((
        "*scryer_download_id".to_string(),
        "scryer-download:missing".to_string(),
    ));
    *download_client.completed_downloads.lock().await = vec![completed];

    let processed =
        crate::import::import::try_import_completed_downloads(&app, &user, &[item]).await;

    assert!(!processed.contains(item_id));
    assert!(download_client.deleted_requests.lock().await.is_empty());
    let state = download_submissions
        .get_identity_tracked_state(
            &DownloadSubmissionIdentity {
                download_id: Some("scryer-download:missing".to_string()),
            },
            None,
        )
        .await
        .expect("identity state lookup");
    assert_eq!(
        state.as_deref(),
        Some(TrackedDownloadState::ImportBlocked.as_str())
    );
}

#[tokio::test]
async fn manual_import_allows_explicit_title_for_missing_download_id_submission() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client,
        download_submissions.clone(),
        pending_releases,
    );
    let import_repo = Arc::new(TrackingImportRepo::default());
    let app = base_app.with_test_overrides(|services| services.with_imports(import_repo.clone()));

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Manual Identity Override".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let source_dir = tempfile::tempdir().expect("source tempdir");
    let mut completed = completed_download_fixture_item(
        "manual-missing-identity",
        &title.id,
        "Manual.Identity.Override.2026.1080p.WEB-DL",
        source_dir.path().to_string_lossy().as_ref(),
    );
    completed.client_id = "weaver-client".to_string();
    completed.client_type = "weaver".to_string();
    completed.parameters.clear();
    completed.parameters.push((
        "*scryer_download_id".to_string(),
        "scryer-e2e-manual-missing".to_string(),
    ));

    let result = app
        .trigger_manual_import(&user, &completed, Some(&title.id))
        .await
        .expect("manual import should honor the explicit title");

    assert_eq!(result.title_id.as_deref(), Some(title.id.as_str()));
    assert_eq!(result.skip_reason, Some(ImportSkipReason::NoVideoFiles));
    assert_ne!(
        result.skip_reason,
        Some(ImportSkipReason::UnresolvedIdentity)
    );
    assert_eq!(import_repo.records.lock().await.len(), 1);
    let identity_state = download_submissions
        .get_identity_tracked_state(
            &DownloadSubmissionIdentity {
                download_id: Some("scryer-e2e-manual-missing".to_string()),
            },
            None,
        )
        .await
        .expect("identity state lookup");
    assert_eq!(identity_state, None);
}

#[tokio::test]
async fn try_import_completed_downloads_imports_additional_series_movie_file_from_submission_scope()
{
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );
    let media_files = Arc::new(MockMediaFileRepo::default());
    let import_repo = Arc::new(TrackingImportRepo::default());
    let app = base_app.with_test_overrides(|services| {
        services
            .with_imports(import_repo.clone())
            .with_file_importer(Arc::new(CopyingFileImporter))
            .with_media_files(media_files.clone())
    });

    let config =
        create_enabled_download_client_config(&app, &user, "Primary NZBGet", "nzbget").await;
    let library_dir = tempfile::tempdir().expect("library tempdir");
    let title_folder = library_dir.path().join("Additional Series Movie Import");
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Additional Series Movie Import".to_string(),
                facet: MediaFacet::Anime,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create anime title");
    app.services
        .catalog
        .titles
        .set_folder_path(&title.id, &title_folder.to_string_lossy())
        .await
        .expect("set title folder path");
    let specials = app
        .create_collection(
            &user,
            title.id.clone(),
            "season".into(),
            "0".into(),
            Some("Specials".into()),
            None,
            Some("0".into()),
            Some("3".into()),
        )
        .await
        .expect("create specials collection");
    let linked_episode = app
        .create_episode(
            &user,
            title.id.clone(),
            Some(specials.id),
            "special".into(),
            Some("3".into()),
            Some("0".into()),
            Some("S00E03".into()),
            Some("Movie Special".into()),
            None,
            Some(6_600),
            false,
            false,
        )
        .await
        .expect("create linked special episode");

    let mut link_input = test_series_movie_link(
        &title.id,
        "Additional Series Movie Import: The Movie",
        Some(2026),
        None,
        Some("additional-series-movie-import"),
    );
    link_input.linked_episode_id = Some(linked_episode.id.clone());
    let link = app
        .services
        .catalog
        .shows
        .upsert_series_movie_link(link_input)
        .await
        .expect("create series movie link");

    let primary_file_id = app
        .services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: title_folder
                .join("Season 00")
                .join("Additional Series Movie Import - S00E03 - The Movie.mkv")
                .to_string_lossy()
                .into_owned(),
            size_bytes: 1_000,
            role: MediaFileRole::Primary,
            quality_label: Some("1080p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert existing primary file");
    app.services
        .library
        .media_files
        .link_file_to_series_movie(&primary_file_id, &link.id)
        .await
        .expect("link primary file to series movie");

    let item_id = "additional-series-movie-import-1";
    download_submissions
        .record_submission(DownloadSubmission {
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::AdditionalFile,
            facet: "anime".to_string(),
            download_client_id: Some(config.id.clone()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: item_id.to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: Some(DownloadSourceKind::NzbUrl),
            source_title: Some(
                "Additional.Series.Movie.Import.The.Movie.2026.1080p.BluRay.x264-Group".to_string(),
            ),
            request_signature: None,
            scope: SubmissionScope::SeriesMovie {
                series_movie_link_id: link.id.clone(),
            },
        })
        .await
        .expect("record additional series movie submission");

    let download_dir = tempfile::tempdir().expect("download tempdir");
    let source_file = download_dir
        .path()
        .join("Additional.Series.Movie.Import.The.Movie.2026.1080p.BluRay.x264-Group.mkv");
    std::fs::File::create(&source_file)
        .expect("create source video")
        .set_len(51 * 1024 * 1024)
        .expect("size source video above sample threshold");
    let mut completed = completed_download_fixture_item(
        item_id,
        &title.id,
        "Additional.Series.Movie.Import.The.Movie.2026.1080p.BluRay.x264-Group",
        download_dir.path().to_string_lossy().as_ref(),
    );
    completed.client_id = config.id.clone();
    completed.parameters.clear();
    *download_client.completed_downloads.lock().await = vec![completed];

    let mut item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);
    item.client_id = config.id.clone();
    item.client_name = config.name.clone();
    item.title_id = Some(title.id.clone());
    item.title_name = title.name.clone();
    item.facet = Some("anime".to_string());

    let processed =
        crate::import::import::try_import_completed_downloads(&app, &user, &[item]).await;
    assert!(
        processed.contains(item_id),
        "series movie additional completed download should be processed"
    );
    let import_records = import_repo.records.lock().await.clone();
    assert_eq!(import_records.len(), 1);
    assert_eq!(
        import_records[0].status,
        ImportStatus::Completed,
        "import result: {:?}",
        import_records[0].result_json
    );

    let files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    let primary_file = files
        .iter()
        .find(|file| file.id == primary_file_id)
        .expect("primary file remains");
    assert_eq!(primary_file.role, MediaFileRole::Primary);
    let additional_file = files
        .iter()
        .find(|file| file.id != primary_file_id)
        .expect("additional file imported");
    assert_eq!(additional_file.role, MediaFileRole::Additional);
    assert_eq!(
        additional_file.episode_id.as_deref(),
        Some(linked_episode.id.as_str())
    );
    assert_eq!(additional_file.series_movie_link_ids, vec![link.id]);
}

#[tokio::test]
async fn path_manual_import_can_target_series_movie_link() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) =
        bootstrap_with_cleanup_tracking(download_client, download_submissions, pending_releases);
    let media_files = Arc::new(MockMediaFileRepo::default());
    let app = base_app.with_test_overrides(|services| {
        services
            .with_file_importer(Arc::new(CopyingFileImporter))
            .with_media_files(media_files.clone())
    });
    app.services
        .identity
        .users
        .create(user.clone())
        .await
        .expect("seed manual import actor");

    let library_dir = tempfile::tempdir().expect("library tempdir");
    let title_folder = library_dir.path().join("Manual Series Movie Import");
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Manual Series Movie Import".to_string(),
                facet: MediaFacet::Anime,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create anime title");
    app.services
        .catalog
        .titles
        .set_folder_path(&title.id, &title_folder.to_string_lossy())
        .await
        .expect("set title folder path");

    let link = app
        .services
        .catalog
        .shows
        .upsert_series_movie_link(test_series_movie_link(
            &title.id,
            "Manual Series Movie Import: Case 3",
            Some(2026),
            None,
            Some("manual-series-movie-import-case-3"),
        ))
        .await
        .expect("create series movie link");

    let source_dir = tempfile::tempdir().expect("source tempdir");
    let source_file = source_dir
        .path()
        .join("Manual.Series.Movie.Import.Case.3.2026.1080p.WEB-DL.mkv");
    std::fs::File::create(&source_file)
        .expect("create source video")
        .set_len(51 * 1024 * 1024)
        .expect("size source video above sample threshold");

    let results = crate::import_workflow::execute_manual_import(
        &app,
        &user,
        "manual-import-series-movie",
        &title.id,
        None,
        vec![ManualImportFileMapping {
            file_path: source_file.to_string_lossy().into_owned(),
            episode_id: None,
            series_movie_link_id: Some(link.id.clone()),
            quality: Some("1080p".to_string()),
        }],
        Some(std::fs::canonicalize(source_dir.path()).expect("canonical source root")),
    )
    .await
    .expect("execute manual import");
    assert!(results.iter().all(|result| result.success), "{results:?}");

    let files = app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    let imported = files
        .iter()
        .find(|file| file.series_movie_link_ids.contains(&link.id))
        .expect("manual import linked media file to series movie");
    assert_eq!(imported.role, MediaFileRole::Primary);
    assert_eq!(imported.episode_id, None);
}

#[tokio::test]
async fn try_import_completed_downloads_blocks_origin_scope_conflict() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );
    let import_repo = Arc::new(TrackingImportRepo::default());
    let app = base_app.with_test_overrides(|services| services.with_imports(import_repo.clone()));

    let config =
        create_enabled_download_client_config(&app, &user, "Primary NZBGet", "nzbget").await;

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Scope Conflict".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let item_id = "origin-scope-conflict-1";
    download_submissions
        .record_submission(DownloadSubmission {
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "movie".to_string(),
            download_client_id: Some(config.id.clone()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: item_id.to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Scope.Conflict.2026.1080p.WEB-DL".to_string()),
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("seed durable submission");

    let mut item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);
    item.client_id = config.id.clone();
    item.client_name = config.name.clone();
    item.title_id = Some(title.id.clone());
    item.title_name = title.name.clone();
    item.facet = Some("movie".to_string());

    let dir = tempfile::tempdir().expect("tempdir");
    let mut completed = completed_download_fixture_item(
        item_id,
        "stale-title",
        "Scope.Conflict.2026.1080p.WEB-DL",
        dir.path().to_string_lossy().as_ref(),
    );
    completed.client_id = config.id.clone();
    *download_client.completed_downloads.lock().await = vec![completed];

    let processed =
        crate::import::import::try_import_completed_downloads(&app, &user, &[item]).await;

    assert!(!processed.contains(item_id));
    assert!(download_client.deleted_requests.lock().await.is_empty());
    assert!(import_repo.records.lock().await.is_empty());
    let state = download_submissions
        .get_tracked_state(&DownloadSourceIdentity::new(
            Some(config.id.as_str()),
            "nzbget",
            item_id,
        ))
        .await
        .expect("tracked state lookup");
    assert_eq!(
        state.as_deref(),
        Some(TrackedDownloadState::ImportBlocked.as_str())
    );
}

#[tokio::test]
async fn try_import_completed_downloads_dedupes_same_download_id_when_item_id_changes() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );

    let config = create_enabled_download_client_config(&app, &user, "Weaver", "weaver").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, false).await;

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Stable Identity".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let old_item_id = "stable-old-item";
    let new_item_id = "stable-new-item";
    let download_id = "scryer-download:stable";
    download_submissions
        .record_submission_with_identity(
            DownloadSubmission {
                title_id: title.id.clone(),
                purpose: crate::DownloadSubmissionPurpose::Standard,
                facet: "movie".to_string(),
                download_client_id: Some(config.id.clone()),
                download_client_type: "weaver".to_string(),
                download_client_item_id: old_item_id.to_string(),
                source_hint: None,
                source_provider_id: None,
                source_provider_name: None,
                source_kind: None,
                source_title: Some("Stable.Identity.2026.1080p.WEB-DL".to_string()),
                request_signature: None,
                scope: SubmissionScope::Title,
            },
            DownloadSubmissionIdentity {
                download_id: Some(download_id.to_string()),
            },
        )
        .await
        .expect("record identity");
    download_submissions
        .update_tracked_state(
            &DownloadSourceIdentity::new(Some(config.id.as_str()), "weaver", old_item_id),
            TrackedDownloadState::Imported.as_str(),
        )
        .await
        .expect("persist imported state");

    let mut item = queue_history_fixture_item(new_item_id, DownloadQueueState::Completed, 40);
    item.client_id = config.id.clone();
    item.client_name = config.name.clone();
    item.client_type = "weaver".to_string();
    item.title_id = Some(title.id.clone());
    item.title_name = title.name.clone();
    item.facet = Some("movie".to_string());

    let dir = tempfile::tempdir().expect("tempdir");
    let mut completed = completed_download_fixture_item(
        new_item_id,
        &title.id,
        "Stable.Identity.2026.1080p.WEB-DL",
        dir.path().to_string_lossy().as_ref(),
    );
    completed.client_id = config.id.clone();
    completed.client_type = "weaver".to_string();
    completed
        .parameters
        .push(("*scryer_download_id".to_string(), download_id.to_string()));
    *download_client.completed_downloads.lock().await = vec![completed];

    let processed =
        crate::import::import::try_import_completed_downloads(&app, &user, &[item]).await;

    assert!(processed.contains(new_item_id));
    assert_eq!(
        download_client.deleted_requests.lock().await.clone(),
        vec![(Some(config.id.clone()), None, new_item_id.to_string(), true)]
    );
}

#[tokio::test]
async fn try_import_completed_downloads_uses_download_submission_fallback_for_untagged_qbittorrent_history()
 {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );
    let import_repo = Arc::new(TrackingImportRepo::default());
    let app = base_app.with_test_overrides(|services| services.with_imports(import_repo.clone()));

    let config =
        seed_download_client_config(&app, "decypharr-qbit-cleanup", "Decypharr", "qbittorrent")
            .await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, false).await;

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Decypharr Cleanup".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create monitored movie title");

    let now = Utc::now().to_rfc3339();
    let item_id = "decypharr-untagged-cleanup-1";
    import_repo.records.lock().await.push(ImportRecord {
        id: Id::new().0,
        source_client_id: Some(config.id.clone()),
        source_system: "qbittorrent".to_string(),
        source_ref: item_id.to_string(),
        import_type: ImportType::MovieDownload,
        status: ImportStatus::Completed,
        payload_json: String::new(),
        result_json: None,
        download_id: None,
        import_transfer_phase: None,
        import_transfer_bytes: None,
        import_transfer_total_bytes: None,
        import_transfer_started_at: None,
        import_transfer_updated_at: None,
        started_at: Some(now.clone()),
        finished_at: Some(now.clone()),
        created_at: now.clone(),
        updated_at: now,
    });

    let mut item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);
    item.client_id = config.id.clone();
    item.client_name = config.name.clone();
    item.client_type = "qbittorrent".to_string();
    item.title_id = Some(title.id.clone());
    item.title_name = title.name.clone();
    item.facet = Some("movie".to_string());
    item.is_scryer_origin = false;

    let dir = tempfile::tempdir().expect("tempdir");
    let mut completed = completed_download_fixture_item(
        item_id,
        &title.id,
        "Harry.Potter.and.the.Prisoner.of.Azkaban.2004.BluRay.1080p.AV1.Opus-nAV1gator",
        dir.path().to_string_lossy().as_ref(),
    );
    completed.client_id = config.id.clone();
    completed.client_type = "qbittorrent".to_string();
    completed.category = Some("radarr".to_string());
    completed.parameters.clear();
    *download_client.completed_downloads.lock().await = vec![completed];

    download_submissions
        .record_submission(DownloadSubmission {
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "movie".to_string(),
            download_client_id: Some(config.id.clone()),
            download_client_type: "qbittorrent".to_string(),
            download_client_item_id: item_id.to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some(
                "Harry.Potter.and.the.Prisoner.of.Azkaban.2004.BluRay.1080p.AV1.Opus-nAV1gator"
                    .to_string(),
            ),
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("seed download submission");

    let processed =
        crate::import::import::try_import_completed_downloads(&app, &user, &[item]).await;

    assert!(processed.contains(item_id));
    assert_eq!(
        download_client.deleted_requests.lock().await.clone(),
        vec![(Some(config.id.clone()), None, item_id.to_string(), true)]
    );
}

#[tokio::test]
async fn try_import_completed_downloads_retries_terminal_cleanup_for_untagged_qbittorrent_history()
{
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );

    let config =
        seed_download_client_config(&app, "decypharr-qbit-retry", "Decypharr", "qbittorrent").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, false).await;

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Decypharr Retry".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create monitored movie title");

    let item_id = "decypharr-untagged-retry-1";
    let mut item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);
    item.client_id = config.id.clone();
    item.client_name = config.name.clone();
    item.client_type = "qbittorrent".to_string();
    item.title_id = Some(title.id.clone());
    item.title_name = title.name.clone();
    item.facet = Some("movie".to_string());
    item.is_scryer_origin = false;

    let dir = tempfile::tempdir().expect("tempdir");
    let mut completed = completed_download_fixture_item(
        item_id,
        &title.id,
        "Harry.Potter.and.the.Prisoner.of.Azkaban.2004.BluRay.1080p.AV1.Opus-nAV1gator",
        dir.path().to_string_lossy().as_ref(),
    );
    completed.client_id = config.id.clone();
    completed.client_type = "qbittorrent".to_string();
    completed.category = Some("radarr".to_string());
    completed.parameters.clear();
    *download_client.completed_downloads.lock().await = vec![completed];

    download_submissions
        .record_submission(DownloadSubmission {
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "movie".to_string(),
            download_client_id: Some(config.id.clone()),
            download_client_type: "qbittorrent".to_string(),
            download_client_item_id: item_id.to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some(
                "Harry.Potter.and.the.Prisoner.of.Azkaban.2004.BluRay.1080p.AV1.Opus-nAV1gator"
                    .to_string(),
            ),
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("seed download submission");
    download_submissions
        .update_tracked_state(
            &DownloadSourceIdentity::new(Some(config.id.as_str()), "qbittorrent", item_id),
            TrackedDownloadState::Imported.as_str(),
        )
        .await
        .expect("seed tracked state");

    let processed =
        crate::import::import::try_import_completed_downloads(&app, &user, &[item]).await;

    assert!(processed.contains(item_id));
    assert_eq!(
        download_client.deleted_requests.lock().await.clone(),
        vec![(Some(config.id.clone()), None, item_id.to_string(), true)]
    );
}

#[tokio::test]
async fn list_download_history_page_filters_terminal_rows_and_clamps_page_size_to_50() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    let mut history_items = (0..120)
        .map(|index| {
            queue_history_fixture_item(
                &format!("completed-{index}"),
                DownloadQueueState::Completed,
                10_000 - index as i64,
            )
        })
        .collect::<Vec<_>>();
    history_items.extend((0..5).map(|index| {
        queue_history_fixture_item(
            &format!("failed-{index}"),
            DownloadQueueState::Failed,
            20_000 - index as i64,
        )
    }));

    let mut blocked =
        queue_history_fixture_item("blocked-import", DownloadQueueState::Completed, 30_000);
    blocked.tracked_state = Some(TrackedDownloadState::ImportBlocked);
    history_items.push(blocked);

    *download_client.history_items.lock().await = history_items;

    let failed_page = app
        .list_download_history_page(
            &user,
            250,
            0,
            Some(vec![DownloadHistoryFilter::Failed]),
            None,
            false,
            None,
        )
        .await
        .expect("failed history page should load");
    assert_eq!(failed_page.total_count, 5);
    assert_eq!(failed_page.items.len(), 5);
    assert_eq!(failed_page.available_clients.len(), 1);
    assert!(
        failed_page
            .items
            .iter()
            .all(|item| item.state == DownloadQueueState::Failed)
    );
    assert!(!failed_page.has_more);

    let all_page = app
        .list_download_history_page(
            &user,
            250,
            0,
            Some(vec![DownloadHistoryFilter::All]),
            None,
            false,
            None,
        )
        .await
        .expect("all history page should load");
    assert_eq!(all_page.total_count, 125);
    assert_eq!(all_page.items.len(), 50);
    assert!(all_page.has_more);
    assert_eq!(
        all_page.items[0].download_client_item_id, "failed-0",
        "newest terminal rows should be returned first"
    );
    assert!(all_page.items.iter().all(
        |item| crate::integration::derive_download_queue_display_state(item)
            != DownloadDisplayState::ImportBlocked
    ));

    let client_filtered_page = app
        .list_download_history_page(
            &user,
            250,
            0,
            Some(vec![DownloadHistoryFilter::Failed]),
            Some(vec!["primary".to_string()]),
            false,
            None,
        )
        .await
        .expect("client filtered history page should load");
    assert_eq!(client_filtered_page.total_count, 5);
    assert_eq!(client_filtered_page.available_clients.len(), 1);
}

#[tokio::test]
async fn list_download_history_page_includes_tracked_terminal_rows_when_client_history_is_empty() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Tracked History Fixture".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                root_folder_id: None,
                min_availability: None,
                poster_url: None,
                year: Some(2012),
                overview: None,
                sort_title: None,
                slug: None,
                runtime_minutes: None,
                language: None,
                content_status: None,
            },
        )
        .await
        .expect("title should be added");

    let mut tracked_history_item =
        queue_history_fixture_item("tracked-terminal-1", DownloadQueueState::Completed, 50);
    tracked_history_item.client_id = "primary".to_string();
    tracked_history_item.client_name = "NZBGet".to_string();
    tracked_history_item.client_type = "nzbget".to_string();
    tracked_history_item.title_id = Some(title.id.clone());
    tracked_history_item.title_name = "Paper Lantern".to_string();

    let tracked_id = crate::tracked_downloads::tracked_download_id(
        Some("primary"),
        "nzbget",
        "tracked-terminal-1",
    );
    app.runtime
        .acquisition
        .tracked_download_snapshot
        .write()
        .await
        .insert(
            tracked_id,
            crate::tracked_downloads::TrackedDownloadQueueMetadata {
                client_item: tracked_history_item,
                client_id: "primary".to_string(),
                client_type: "nzbget".to_string(),
                title_id: Some(title.id.clone()),
                facet: Some("movie".to_string()),
                source_title: Some("Paper.Lantern.2012.720p.WEB-DL.AV1.AAC2.0-NTb".to_string()),
                state: TrackedDownloadState::Imported,
                status: scryer_domain::TrackedDownloadStatus::Ok,
                status_messages: Vec::new(),
                match_type: scryer_domain::TitleMatchType::Submission,
                foreign_import_classification: None,
            },
        );

    let page = app
        .list_download_history_page(
            &user,
            50,
            0,
            Some(vec![DownloadHistoryFilter::All]),
            None,
            false,
            None,
        )
        .await
        .expect("tracked terminal history page should load");

    assert_eq!(page.total_count, 1);
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].download_client_item_id, "tracked-terminal-1");
    assert_eq!(
        page.items[0].tracked_state,
        Some(TrackedDownloadState::Imported)
    );
    assert_eq!(page.items[0].import_status, Some(ImportStatus::Completed));
    assert_eq!(
        page.items[0].title_name,
        "Paper.Lantern.2012.720p.WEB-DL.AV1.AAC2.0-NTb"
    );
}

#[tokio::test]
async fn list_download_history_page_sorts_before_paginating() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    let history_items = (0..60)
        .map(|index| {
            let mut item = queue_history_fixture_item(
                &format!("sort-{index:02}"),
                DownloadQueueState::Completed,
                10_000 - index as i64,
            );
            item.title_name = format!("Title {:02}", 59 - index);
            item
        })
        .collect::<Vec<_>>();

    *download_client.history_items.lock().await = history_items;

    let first_page = app
        .list_download_history_page(
            &user,
            50,
            0,
            Some(vec![DownloadHistoryFilter::All]),
            None,
            false,
            Some(DownloadHistorySort {
                key: DownloadHistorySortKey::Title,
                direction: SortDirection::Asc,
            }),
        )
        .await
        .expect("sorted history page should load");

    let second_page = app
        .list_download_history_page(
            &user,
            50,
            50,
            Some(vec![DownloadHistoryFilter::All]),
            None,
            false,
            Some(DownloadHistorySort {
                key: DownloadHistorySortKey::Title,
                direction: SortDirection::Asc,
            }),
        )
        .await
        .expect("second sorted history page should load");

    assert_eq!(first_page.items.len(), 50);
    assert_eq!(second_page.items.len(), 10);
    assert_eq!(first_page.items[0].title_name, "Title 00");
    assert_eq!(first_page.items[49].title_name, "Title 49");
    assert_eq!(second_page.items[0].title_name, "Title 50");
}

#[tokio::test]
async fn list_download_history_page_can_limit_to_scryer_submitted_rows() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    let mut scryer_item =
        queue_history_fixture_item("scryer-item", DownloadQueueState::Completed, 100);
    scryer_item.client_id = "primary".to_string();
    scryer_item.client_name = "Primary".to_string();

    let mut external_item =
        queue_history_fixture_item("external-item", DownloadQueueState::Failed, 90);
    external_item.is_scryer_origin = false;
    external_item.client_id = "secondary".to_string();
    external_item.client_name = "Secondary".to_string();

    *download_client.history_items.lock().await = vec![scryer_item, external_item];

    let page = app
        .list_download_history_page(
            &user,
            50,
            0,
            Some(vec![DownloadHistoryFilter::All]),
            None,
            true,
            None,
        )
        .await
        .expect("scryer filtered history page should load");

    assert_eq!(page.total_count, 1);
    assert_eq!(page.items.len(), 1);
    assert!(page.items.iter().all(|item| item.is_scryer_origin));
    assert_eq!(page.available_clients.len(), 1);
    assert_eq!(page.available_clients[0].client_id, "primary");
}

#[tokio::test]
async fn recent_activity_and_history_ignore_operational_domain_events() {
    let (app, user) = bootstrap();

    app.add_title(
        &user,
        NewTitle {
            name: "Activity Filter Fixture".to_string(),
            facet: MediaFacet::Movie,
            monitored: true,
            tags: vec![],
            external_ids: vec![],
            root_folder_id: None,
            min_availability: None,
            poster_url: None,
            year: None,
            overview: None,
            sort_title: None,
            slug: None,
            runtime_minutes: None,
            language: None,
            content_status: None,
        },
    )
    .await
    .expect("title should be added");

    app.append_domain_event(crate::domain_events::new_global_domain_event(
        None,
        DomainEventPayload::JobRunStarted(JobRunStartedEventData {
            run_id: "job-run-1".to_string(),
            job_key: "rss_sync".to_string(),
            operation_type: "job".to_string(),
            trigger_source: "system_internal".to_string(),
        }),
    ))
    .await
    .expect("operational domain event should append");

    let activities = app
        .recent_activity(&user, 10, 0)
        .await
        .expect("recent activity should load");
    assert_eq!(activities.len(), 1);
    assert_eq!(activities[0].kind, ActivityKind::TitleAdded);

    let history = app
        .recent_events(&user, None, 10, 0)
        .await
        .expect("recent events should load");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].event_type, EventType::TitleAdded);

    let after_sequence = app
        .list_activity_events_after_sequence(&user, 0, 10)
        .await
        .expect("activity replay should load");
    assert_eq!(after_sequence.len(), 1);
    assert_eq!(after_sequence[0].1.kind, ActivityKind::TitleAdded);
}

#[tokio::test]
async fn download_queue_subscription_bootstraps_from_live_queue_without_history_events() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    *download_client.queue_items.lock().await = vec![DownloadQueueItem {
        id: "queue-1".to_string(),
        title_id: None,
        episode_id: None,
        title_name: "Foreign Queue Item".to_string(),
        facet: Some("movie".to_string()),
        category: None,
        client_id: "primary".to_string(),
        client_name: "NZBGet".to_string(),
        client_type: "nzbget".to_string(),
        state: DownloadQueueState::Queued,
        progress_percent: 10,
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
        download_client_item_id: "queue-1".to_string(),
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
        tracked_status_messages: Vec::new(),
        tracked_match_type: None,
    }];

    let mut receiver = app
        .subscribe_download_queue(&user)
        .expect("queue subscription should start");
    let snapshot = tokio::time::timeout(Duration::from_secs(1), receiver.recv())
        .await
        .expect("initial queue snapshot should arrive")
        .expect("queue subscription should stay open");

    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot[0].download_client_item_id, "queue-1");
    assert_eq!(snapshot[0].title_name, "Foreign Queue Item");
}

#[tokio::test]
async fn download_queue_subscription_sends_empty_bootstrap_snapshot() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) =
        bootstrap_with_cleanup_tracking(download_client, download_submissions, pending_releases);

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    let mut receiver = app
        .subscribe_download_queue(&user)
        .expect("queue subscription should start");
    let snapshot = tokio::time::timeout(Duration::from_secs(1), receiver.recv())
        .await
        .expect("initial empty queue snapshot should arrive")
        .expect("queue subscription should stay open");

    assert!(snapshot.is_empty());
}

#[tokio::test]
async fn queued_delete_poller_executes_client_delete() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_queue_commands = Arc::new(TrackingDownloadQueueCommandRepo::default());
    let command_id = download_queue_commands
        .seed_pending(None, "nzbget", "job-1", true)
        .await;
    let (app, _) =
        bootstrap_with_delete_queue(download_client.clone(), download_queue_commands.clone());
    let token = tokio_util::sync::CancellationToken::new();

    let handle = tokio::spawn(start_background_download_delete_poller(
        app,
        token.child_token(),
    ));

    let record = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let Some(record) = download_queue_commands.get(&command_id).await
                && record.status == scryer_domain::DownloadQueueDeleteStatus::Completed
            {
                break record;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("queued delete should complete");

    token.cancel();
    handle.await.expect("delete poller should stop cleanly");

    assert_eq!(
        record.status,
        scryer_domain::DownloadQueueDeleteStatus::Completed
    );
    assert_eq!(
        download_client.deleted_items.lock().await.clone(),
        vec![("job-1".to_string(), true)]
    );
}

#[tokio::test]
async fn queued_delete_poller_marks_failure_and_persists_error() {
    let download_client = Arc::new(StubDownloadClient::default());
    download_client
        .set_delete_error(Some("delete failed"))
        .await;
    let download_queue_commands = Arc::new(TrackingDownloadQueueCommandRepo::default());
    let command_id = download_queue_commands
        .seed_pending(None, "nzbget", "job-2", false)
        .await;
    let (app, _) =
        bootstrap_with_delete_queue(download_client.clone(), download_queue_commands.clone());
    let token = tokio_util::sync::CancellationToken::new();

    let handle = tokio::spawn(start_background_download_delete_poller(
        app,
        token.child_token(),
    ));

    let record = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let Some(record) = download_queue_commands.get(&command_id).await
                && record.status == scryer_domain::DownloadQueueDeleteStatus::Failed
            {
                break record;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("queued delete should fail");

    token.cancel();
    handle.await.expect("delete poller should stop cleanly");

    assert_eq!(
        record.status,
        scryer_domain::DownloadQueueDeleteStatus::Failed
    );
    assert_eq!(
        record.error_text.as_deref(),
        Some("repository: delete failed")
    );
    assert!(download_client.deleted_items.lock().await.is_empty());
}

#[tokio::test]
async fn ignore_tracked_download_uses_durable_fallback_idempotently() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client,
        download_submissions.clone(),
        pending_releases,
    );
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Durable Ignore Fallback".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");
    let source_identity = DownloadSourceIdentity::new(None, "nzbget", "evicted-job-1");
    download_submissions
        .record_submission_with_identity(
            DownloadSubmission {
                title_id: title.id,
                purpose: crate::DownloadSubmissionPurpose::Standard,
                facet: "movie".to_string(),
                download_client_id: None,
                download_client_type: "nzbget".to_string(),
                download_client_item_id: "evicted-job-1".to_string(),
                source_hint: Some("https://indexer.example/get?apikey=secret".to_string()),
                source_provider_id: Some("indexer-1".to_string()),
                source_provider_name: Some("Fixture Indexer".to_string()),
                source_kind: None,
                source_title: Some("Durable.Ignore.2026.1080p.WEB-DL".to_string()),
                request_signature: None,
                scope: SubmissionScope::Title,
            },
            DownloadSubmissionIdentity {
                download_id: Some("scryer-download:evicted-job-1".to_string()),
            },
        )
        .await
        .expect("record submission identity");

    assert!(matches!(
        crate::integration::workflow::finalize_scryer_download_ignored(
            &app,
            crate::domain_events::DomainEventActor::from(&user),
            source_identity.clone(),
        )
        .await
        .expect("durable fallback should ignore the evicted item"),
        crate::integration::workflow::FinalizeIgnoredOutcome::Finalized
    ));
    assert!(matches!(
        crate::integration::workflow::finalize_scryer_download_ignored(
            &app,
            crate::domain_events::DomainEventActor::from(&user),
            source_identity.clone(),
        )
        .await
        .expect("second ignore should be idempotent"),
        crate::integration::workflow::FinalizeIgnoredOutcome::Finalized
    ));

    let states = download_submissions.identity_states.lock().await;
    assert_eq!(states.len(), 1);
    assert!(
        states
            .values()
            .all(|state| state == TrackedDownloadState::Ignored.as_str())
    );
    drop(states);
    assert!(
        download_submissions
            .get_identity_tracked_state(
                &DownloadSubmissionIdentity {
                    download_id: Some("scryer-download:evicted-job-1".to_string()),
                },
                Some(&source_identity),
            )
            .await
            .expect("load durable state")
            .is_some_and(|state| state == TrackedDownloadState::Ignored.as_str())
    );
}

#[tokio::test]
async fn finalize_ignore_preserves_an_imported_outcome() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client,
        download_submissions.clone(),
        pending_releases,
    );
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Imported Then Deleted".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");
    let source_identity = DownloadSourceIdentity::new(None, "nzbget", "done-job-1");
    let identity = DownloadSubmissionIdentity {
        download_id: Some("scryer-download:done-job-1".to_string()),
    };
    download_submissions
        .record_submission_with_identity(
            DownloadSubmission {
                title_id: title.id,
                purpose: crate::DownloadSubmissionPurpose::Standard,
                facet: "movie".to_string(),
                download_client_id: None,
                download_client_type: "nzbget".to_string(),
                download_client_item_id: "done-job-1".to_string(),
                source_hint: None,
                source_provider_id: None,
                source_provider_name: Some("Fixture Indexer".to_string()),
                source_kind: None,
                source_title: Some("Imported.Then.Deleted.2026.1080p.WEB-DL".to_string()),
                request_signature: None,
                scope: SubmissionScope::Title,
            },
            identity.clone(),
        )
        .await
        .expect("record submission identity");
    download_submissions
        .record_identity_tracked_state(
            &identity,
            Some(&source_identity),
            TrackedDownloadState::Imported.as_str(),
            None,
            None,
        )
        .await
        .expect("record imported outcome");

    // Deleting the client's history entry afterwards is cleanup, not a change
    // of outcome: the imported state must survive and no ignore may be
    // recorded.
    assert!(matches!(
        crate::integration::workflow::finalize_scryer_download_ignored(
            &app,
            crate::domain_events::DomainEventActor::from(&user),
            source_identity.clone(),
        )
        .await
        .expect("finalize should preserve the imported outcome"),
        crate::integration::workflow::FinalizeIgnoredOutcome::PreservedTerminal(state)
            if state == TrackedDownloadState::Imported.as_str()
    ));
    assert!(
        download_submissions
            .get_identity_tracked_state(&identity, Some(&source_identity))
            .await
            .expect("load durable state")
            .is_some_and(|state| state == TrackedDownloadState::Imported.as_str())
    );
}

#[tokio::test]
async fn active_library_scans_and_subscription_use_runtime_tracker_state() {
    let (app, user) = bootstrap();

    let session = app
        .runtime
        .library
        .library_scan_tracker
        .start_session_with_id(
            "scan-runtime-1".to_string(),
            MediaFacet::Movie,
            LibraryScanMode::Full,
        )
        .await
        .expect("library scan session should start");

    let active = app
        .active_library_scans(&user)
        .await
        .expect("active scans should load");
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].session_id, session.session_id);

    let mut receiver = app
        .subscribe_library_scan_progress(&user)
        .await
        .expect("library scan subscription should start");
    let initial = tokio::time::timeout(Duration::from_secs(1), receiver.recv())
        .await
        .expect("initial library scan snapshot should arrive")
        .expect("library scan subscription should stay open");

    assert_eq!(initial.session_id, session.session_id);
    assert_eq!(initial.facet, session.facet);
}

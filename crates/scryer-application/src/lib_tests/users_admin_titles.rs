use super::*;

fn test_job_run_record(
    id: &str,
    job_key: JobKey,
    status: JobRunStatus,
    started_at: chrono::DateTime<chrono::Utc>,
    actor_user_id: Option<String>,
) -> JobRunRecord {
    let completed_at = status.is_terminal().then_some(started_at);
    JobRunRecord {
        id: id.to_string(),
        job_key,
        operation_type: format!("{}:test", job_key.as_str()),
        status,
        trigger_source: JobTriggerSource::Manual,
        actor_user_id,
        progress_json: None,
        summary_json: None,
        summary_text: None,
        error_text: None,
        started_at,
        completed_at,
        created_at: started_at,
        updated_at: started_at,
    }
}

#[tokio::test]
async fn create_user_and_list_users() {
    let (app, user) = bootstrap();

    let created = create_user_with_permissions(
        &app,
        &user,
        "editor",
        "password123",
        vec![
            TestPermissionPreset::CatalogView,
            TestPermissionPreset::TitleManagement,
        ],
    )
    .await
    .expect("create user");

    let users = app.list_users(&user).await.expect("list users");
    assert!(users.iter().any(|entry| entry.username == created.username));
    assert_eq!(users.len(), 1);
}

#[tokio::test]
async fn create_user_without_permission_grants_allows_manage_users_only_actor() {
    let (app, _) = bootstrap();
    let actor = test_user_with_app_permissions("user-admin", AppPermissionMask::MANAGE_USERS);

    let created = app
        .create_user(
            &actor,
            "plain-user".to_string(),
            "password123".to_string(),
            AppPermissionMask::NONE,
            Vec::new(),
        )
        .await
        .expect("create user without grants");

    assert_eq!(created.username, "plain-user");
}

#[tokio::test]
async fn create_user_with_app_permission_grants_requires_manage_permissions() {
    let (app, _) = bootstrap();
    let actor = test_user_with_app_permissions("user-admin", AppPermissionMask::MANAGE_USERS);

    let result = app
        .create_user(
            &actor,
            "privileged-user".to_string(),
            "password123".to_string(),
            AppPermissionMask::MANAGE_SYSTEM_SETTINGS,
            Vec::new(),
        )
        .await;

    assert!(matches!(
        result,
        Err(AppError::Unauthorized(_) | AppError::UnauthorizedReason { .. })
    ));
}

#[tokio::test]
async fn create_user_with_library_permission_grants_requires_manage_permissions() {
    let (app, _) = bootstrap();
    let actor = test_user_with_app_permissions("user-admin", AppPermissionMask::MANAGE_USERS);

    let result = app
        .create_user(
            &actor,
            "library-user".to_string(),
            "password123".to_string(),
            AppPermissionMask::NONE,
            vec![scryer_domain::LibraryGrant {
                user_id: String::new(),
                library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
                permissions: scryer_domain::LibraryPermissionMask::VIEW,
            }],
        )
        .await;

    assert!(matches!(
        result,
        Err(AppError::Unauthorized(_) | AppError::UnauthorizedReason { .. })
    ));
}

#[tokio::test]
async fn get_user_by_id_returns_created_user() {
    let (app, user) = bootstrap();

    let created = create_user_with_permissions(
        &app,
        &user,
        "viewer",
        "password123",
        vec![TestPermissionPreset::CatalogView],
    )
    .await
    .expect("create user");

    let found = app.get_user(&user, &created.id).await.expect("get user");

    assert!(found.is_some());
    let found = found.expect("user should exist");
    assert_eq!(found.id, created.id);
    assert_eq!(found.username, "viewer");
}

#[tokio::test]
async fn create_user_rejects_duplicate_username() {
    let (app, user) = bootstrap();

    let _created = create_user_with_permissions(
        &app,
        &user,
        "editor",
        "password123",
        vec![TestPermissionPreset::CatalogView],
    )
    .await
    .expect("first create");

    let second = create_user_with_permissions(
        &app,
        &user,
        "editor",
        "password123",
        vec![TestPermissionPreset::CatalogView],
    )
    .await;

    assert!(second.is_err());
}

#[tokio::test]
async fn create_user_rejects_recovery_admin_username() {
    let (app, user) = bootstrap();

    let result = app
        .create_user(
            &user,
            "recovery-admin".to_string(),
            "password123".to_string(),
            AppPermissionMask::NONE,
            Vec::new(),
        )
        .await;

    assert!(matches!(
        result,
        Err(AppError::Validation(_)
            | AppError::ValidationRefused { .. }
            | AppError::ValidationRefusedWithCount { .. })
    ));
}

#[tokio::test]
async fn create_user_rejects_anonymous_username() {
    let (app, user) = bootstrap();

    let result = app
        .create_user(
            &user,
            "Anonymous".to_string(),
            "password123".to_string(),
            AppPermissionMask::NONE,
            Vec::new(),
        )
        .await;

    let Err(AppError::ValidationRefused {
        message, reason, ..
    }) = result
    else {
        panic!("anonymous username should be reserved");
    };
    assert_eq!(
        reason,
        crate::error_reasons::security::ANONYMOUS_RESERVED_AUTHLESS_AUDIT_ATTRIBUTION
    );
    assert!(message.contains("anonymous is reserved"));
}

#[tokio::test]
async fn ensure_default_admin_rejects_anonymous_username() {
    let (app, _) = bootstrap();

    let result = app.ensure_default_admin("anonymous", "password123").await;

    let Err(AppError::ValidationRefused {
        message, reason, ..
    }) = result
    else {
        panic!("anonymous default admin username should be reserved");
    };
    assert_eq!(
        reason,
        crate::error_reasons::security::ANONYMOUS_RESERVED_AUTHLESS_AUDIT_ATTRIBUTION
    );
    assert!(message.contains("anonymous is reserved"));
}

#[tokio::test]
async fn delete_title_removes_title_from_catalog() {
    let (app, user) = bootstrap();

    let created = app
        .add_title(
            &user,
            NewTitle {
                name: "Delete Me".into(),
                facet: MediaFacet::Movie,
                monitored: false,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,

                ..Default::default()
            },
        )
        .await
        .expect("create title");

    app.services
        .catalog
        .libraries
        .set_app_permission_mask_for_user(&user.id, user.authorization.app)
        .await
        .expect("persist fixture administrator permissions");
    app.delete_title(&user, &created.id, false, None)
        .await
        .expect("delete title");

    let titles = app
        .list_titles_unpaged(&user, Some(MediaFacet::Movie), None, None)
        .await
        .expect("list titles");
    assert!(titles.is_empty());
}

#[tokio::test]
async fn start_delete_titles_job_requires_preview_for_disk_delete() {
    let (app, user) = bootstrap();

    let created = app
        .add_title(
            &user,
            NewTitle {
                name: "Keep Me".into(),
                facet: MediaFacet::Movie,
                monitored: false,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let result = app
        .start_delete_titles_job(
            &user,
            DeleteTitlesJobRequest {
                items: vec![DeleteTitlesJobItem {
                    title_id: created.id.clone(),
                    preview_fingerprint: None,
                }],
                delete_files_on_disk: true,
                typed_confirmation: None,
            },
        )
        .await;

    assert!(result.is_err());
    assert!(
        app.get_title(&user, &created.id)
            .await
            .expect("get title")
            .is_some()
    );
}

#[tokio::test]
async fn title_deletion_job_runs_are_visible_to_actor_without_system_settings() {
    let job_runs = Arc::new(RecordingJobRunRepo::default());
    let (base_app, admin) = bootstrap();
    let app = base_app.with_test_overrides(|services| services.with_job_runs(job_runs.clone()));
    let manager = create_user_with_permissions(
        &app,
        &admin,
        "title-manager",
        "password123",
        vec![TestPermissionPreset::TitleManagement],
    )
    .await
    .expect("create manager");
    let other_manager = create_user_with_permissions(
        &app,
        &admin,
        "other-title-manager",
        "password123",
        vec![TestPermissionPreset::TitleManagement],
    )
    .await
    .expect("create other manager");
    let now = chrono::Utc::now();
    let make_run = |id: &str, job_key: JobKey, actor_user_id: Option<String>| JobRunRecord {
        id: id.to_string(),
        job_key,
        operation_type: format!("{}:test", job_key.as_str()),
        status: JobRunStatus::Completed,
        trigger_source: JobTriggerSource::Manual,
        actor_user_id,
        progress_json: None,
        summary_json: None,
        summary_text: None,
        error_text: None,
        started_at: now,
        completed_at: Some(now),
        created_at: now,
        updated_at: now,
    };

    job_runs
        .seed(make_run(
            "own-title-delete",
            JobKey::TitleDeletion,
            Some(manager.id.clone()),
        ))
        .await;
    job_runs
        .seed(make_run(
            "other-title-delete",
            JobKey::TitleDeletion,
            Some(other_manager.id.clone()),
        ))
        .await;
    job_runs
        .seed(make_run(
            "own-housekeeping",
            JobKey::Housekeeping,
            Some(manager.id.clone()),
        ))
        .await;
    let mut current_library_media_run = make_run(
        "own-media-file-delete-current-library",
        JobKey::MediaFileDeletion,
        Some(manager.id.clone()),
    );
    current_library_media_run.operation_type = format!(
        "media_file_deletion:{}:file-current",
        scryer_domain::default_library_id_for_facet(&MediaFacet::Movie)
    );
    job_runs.seed(current_library_media_run).await;
    let mut revoked_library_media_run = make_run(
        "own-media-file-delete-revoked-library",
        JobKey::MediaFileDeletion,
        Some(manager.id.clone()),
    );
    revoked_library_media_run.operation_type =
        "media_file_deletion:revoked-library:file-revoked".to_string();
    job_runs.seed(revoked_library_media_run).await;

    let manager_runs = app
        .list_job_runs(&manager, JobKey::TitleDeletion, 10)
        .await
        .expect("manager can list own title deletion runs");
    assert_eq!(manager_runs.len(), 1);
    assert_eq!(manager_runs[0].id, "own-title-delete");

    let media_file_runs = app
        .list_job_runs(&manager, JobKey::MediaFileDeletion, 10)
        .await
        .expect("manager can list own scoped media file deletion runs");
    assert_eq!(media_file_runs.len(), 1);
    assert_eq!(
        media_file_runs[0].id,
        "own-media-file-delete-current-library"
    );

    let denied = app.list_job_runs(&manager, JobKey::Housekeeping, 10).await;
    assert!(matches!(
        denied,
        Err(AppError::Unauthorized(_) | AppError::UnauthorizedReason { .. })
    ));

    let admin_runs = app
        .list_job_runs(&admin, JobKey::TitleDeletion, 10)
        .await
        .expect("admin can list all title deletion runs");
    let admin_run_ids = admin_runs
        .into_iter()
        .map(|run| run.id)
        .collect::<HashSet<_>>();
    assert!(admin_run_ids.contains("own-title-delete"));
    assert!(admin_run_ids.contains("other-title-delete"));
}

#[tokio::test]
async fn library_managers_receive_only_their_scoped_interactive_job_events() {
    let (app, admin) = bootstrap();
    let test_password = uuid::Uuid::new_v4().to_string();
    let manager = create_user_with_permissions(
        &app,
        &admin,
        "interactive-job-manager",
        &test_password,
        vec![TestPermissionPreset::TitleManagement],
    )
    .await
    .expect("create manager");
    let other_manager = create_user_with_permissions(
        &app,
        &admin,
        "other-interactive-job-manager",
        &test_password,
        vec![TestPermissionPreset::TitleManagement],
    )
    .await
    .expect("create other manager");
    let now = chrono::Utc::now();
    let run = JobRunRecord {
        id: "active-media-file-delete".to_string(),
        job_key: JobKey::MediaFileDeletion,
        operation_type: format!(
            "media_file_deletion:{}:file-1",
            scryer_domain::default_library_id_for_facet(&MediaFacet::Movie)
        ),
        status: JobRunStatus::Running,
        trigger_source: JobTriggerSource::Manual,
        actor_user_id: Some(manager.id.clone()),
        progress_json: None,
        summary_json: None,
        summary_text: None,
        error_text: None,
        started_at: now,
        completed_at: None,
        created_at: now,
        updated_at: now,
    };
    app.runtime
        .jobs
        .job_run_tracker
        .upsert_active_run(JobRun::from_record(&run, None))
        .await;

    let active_runs = app
        .active_job_runs(&manager)
        .await
        .expect("manager can list own active interactive job");
    assert_eq!(active_runs.len(), 1);
    assert_eq!(active_runs[0].id, run.id);

    let mut manager_events = app
        .subscribe_job_run_events(&manager)
        .await
        .expect("manager can subscribe to own interactive jobs");
    let manager_event = within_deadline(
        "the manager's active interactive job event",
        manager_events.recv(),
    )
    .await
    .expect("job event should be delivered");
    assert_eq!(manager_event.id, run.id);

    let mut other_events = app
        .subscribe_job_run_events(&other_manager)
        .await
        .expect("other manager subscription should initialize");
    // A sentinel the other manager may see. The subscription sends its whole
    // replay in one synchronous burst, and the sentinel arrives either in that
    // replay or live after it, so once the sentinel is in hand everything the
    // replay could have leaked is already in the channel.
    let sentinel = JobRunRecord {
        id: "other-manager-sentinel".to_string(),
        operation_type: format!(
            "media_file_deletion:{}:file-2",
            scryer_domain::default_library_id_for_facet(&MediaFacet::Movie)
        ),
        actor_user_id: Some(other_manager.id.clone()),
        ..run.clone()
    };
    app.runtime
        .jobs
        .job_run_tracker
        .upsert_active_run(JobRun::from_record(&sentinel, None))
        .await;
    let mut seen = Vec::new();
    loop {
        let event = within_deadline(
            "the other manager's sentinel job event",
            other_events.recv(),
        )
        .await
        .expect("job event should be delivered");
        let is_sentinel = event.id == sentinel.id;
        seen.push(event.id);
        if is_sentinel {
            break;
        }
    }
    while let Ok(event) = other_events.try_recv() {
        seen.push(event.id);
    }
    assert!(
        !seen.contains(&run.id),
        "an interactive job must not be broadcast to a different manager: {seen:?}"
    );
}

#[tokio::test]
async fn active_job_runs_use_persisted_rows_without_domain_event_replay() {
    let job_runs = Arc::new(RecordingJobRunRepo::default());
    let domain_events = Arc::new(MockDomainEventRepo::default());
    let (base_app, admin) = bootstrap();
    let app = base_app.with_test_overrides(|services| {
        services
            .with_job_runs(job_runs.clone())
            .with_domain_events(domain_events.clone())
    });
    let now = chrono::Utc::now();

    job_runs
        .seed(test_job_run_record(
            "active-library-scan",
            JobKey::LibraryScanMovies,
            JobRunStatus::Running,
            now,
            Some(admin.id.clone()),
        ))
        .await;
    app.runtime
        .library
        .library_scan_tracker
        .start_session_with_id(
            "active-library-scan".to_string(),
            MediaFacet::Movie,
            LibraryScanMode::Full,
        )
        .await
        .expect("start library scan session");

    let runs = app
        .active_job_runs(&admin)
        .await
        .expect("list active job runs");

    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].id, "active-library-scan");
    assert_eq!(runs[0].status, JobRunStatus::Discovering);
    assert_eq!(
        runs[0]
            .library_scan_progress
            .as_ref()
            .map(|session| session.session_id.as_str()),
        Some("active-library-scan")
    );
    assert_eq!(
        job_runs.list_active_job_runs_calls.load(Ordering::SeqCst),
        1
    );
    assert_eq!(domain_events.list_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn active_job_runs_prefer_in_memory_tracker_rows() {
    let job_runs = Arc::new(RecordingJobRunRepo::default());
    let domain_events = Arc::new(MockDomainEventRepo::default());
    let (base_app, admin) = bootstrap();
    let app = base_app.with_test_overrides(|services| {
        services
            .with_job_runs(job_runs.clone())
            .with_domain_events(domain_events.clone())
    });
    let now = chrono::Utc::now();
    app.runtime
        .jobs
        .job_run_tracker
        .upsert_active_run(JobRun::from_record(
            &test_job_run_record(
                "tracked-housekeeping",
                JobKey::Housekeeping,
                JobRunStatus::Running,
                now,
                Some(admin.id.clone()),
            ),
            None,
        ))
        .await;

    let runs = app
        .active_job_runs(&admin)
        .await
        .expect("list active job runs");

    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].id, "tracked-housekeeping");
    assert_eq!(
        job_runs.list_active_job_runs_calls.load(Ordering::SeqCst),
        0
    );
    assert_eq!(domain_events.list_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn job_history_reads_do_not_replay_domain_events_when_tracker_is_empty() {
    let job_runs = Arc::new(RecordingJobRunRepo::default());
    let domain_events = Arc::new(MockDomainEventRepo::default());
    let (base_app, admin) = bootstrap();
    let app = base_app.with_test_overrides(|services| {
        services
            .with_job_runs(job_runs.clone())
            .with_domain_events(domain_events.clone())
    });
    let now = chrono::Utc::now();

    job_runs
        .seed(test_job_run_record(
            "older-housekeeping",
            JobKey::Housekeeping,
            JobRunStatus::Completed,
            now - chrono::Duration::seconds(30),
            Some(admin.id.clone()),
        ))
        .await;
    job_runs
        .seed(test_job_run_record(
            "newer-housekeeping",
            JobKey::Housekeeping,
            JobRunStatus::Completed,
            now,
            Some(admin.id.clone()),
        ))
        .await;

    let recent_runs = app
        .list_recent_job_runs(&admin, 50)
        .await
        .expect("list recent job runs");
    let job_runs_for_key = app
        .list_job_runs(&admin, JobKey::Housekeeping, 10)
        .await
        .expect("list per-job runs");

    assert_eq!(
        recent_runs
            .iter()
            .map(|run| run.id.as_str())
            .collect::<Vec<_>>(),
        vec!["newer-housekeeping", "older-housekeeping"]
    );
    assert_eq!(
        job_runs_for_key
            .iter()
            .map(|run| run.id.as_str())
            .collect::<Vec<_>>(),
        vec!["newer-housekeeping", "older-housekeeping"]
    );
    assert_eq!(job_runs.list_job_runs_calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        job_runs
            .list_job_runs_for_actor_calls
            .load(Ordering::SeqCst),
        0
    );
    assert_eq!(domain_events.list_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn latest_job_runs_report_each_job_even_when_another_fills_the_recent_window() {
    let job_runs = Arc::new(RecordingJobRunRepo::default());
    let (base_app, admin) = bootstrap();
    let app = base_app.with_test_overrides(|services| services.with_job_runs(job_runs.clone()));
    let now = chrono::Utc::now();

    job_runs
        .seed(test_job_run_record(
            "older-housekeeping",
            JobKey::Housekeeping,
            JobRunStatus::Completed,
            now - chrono::Duration::hours(2),
            None,
        ))
        .await;
    job_runs
        .seed(test_job_run_record(
            "newest-housekeeping",
            JobKey::Housekeeping,
            JobRunStatus::Completed,
            now - chrono::Duration::hours(1),
            None,
        ))
        .await;
    for index in 0..5 {
        job_runs
            .seed(test_job_run_record(
                &format!("rss-{index}"),
                JobKey::RssSync,
                JobRunStatus::Completed,
                now - chrono::Duration::minutes(index),
                None,
            ))
            .await;
    }

    let recent_runs = app
        .list_recent_job_runs(&admin, 3)
        .await
        .expect("list recent job runs");
    assert!(
        recent_runs.iter().all(|run| run.job_key == JobKey::RssSync),
        "the frequent job fills the recent window"
    );

    let mut latest = app
        .list_latest_job_runs(&admin)
        .await
        .expect("list latest job runs")
        .into_iter()
        .map(|run| (run.job_key, run.id))
        .collect::<Vec<_>>();
    latest.sort_by_key(|(job_key, _)| job_key.as_str());

    assert_eq!(
        latest,
        vec![
            (JobKey::Housekeeping, "newest-housekeeping".to_string()),
            (JobKey::RssSync, "rss-0".to_string()),
        ]
    );
}

#[tokio::test]
async fn actor_scoped_job_history_reads_do_not_replay_domain_events() {
    let job_runs = Arc::new(RecordingJobRunRepo::default());
    let domain_events = Arc::new(MockDomainEventRepo::default());
    let (base_app, admin) = bootstrap();
    let app = base_app.with_test_overrides(|services| {
        services
            .with_job_runs(job_runs.clone())
            .with_domain_events(domain_events.clone())
    });
    let manager = create_user_with_permissions(
        &app,
        &admin,
        "title-history-manager",
        "password123",
        vec![TestPermissionPreset::TitleManagement],
    )
    .await
    .expect("create manager");
    let other_manager = create_user_with_permissions(
        &app,
        &admin,
        "title-history-other-manager",
        "password123",
        vec![TestPermissionPreset::TitleManagement],
    )
    .await
    .expect("create other manager");
    let now = chrono::Utc::now();

    job_runs
        .seed(test_job_run_record(
            "manager-title-delete",
            JobKey::TitleDeletion,
            JobRunStatus::Completed,
            now,
            Some(manager.id.clone()),
        ))
        .await;
    job_runs
        .seed(test_job_run_record(
            "other-title-delete",
            JobKey::TitleDeletion,
            JobRunStatus::Completed,
            now,
            Some(other_manager.id.clone()),
        ))
        .await;

    let manager_runs = app
        .list_job_runs(&manager, JobKey::TitleDeletion, 10)
        .await
        .expect("manager can list own title deletion runs");

    assert_eq!(manager_runs.len(), 1);
    assert_eq!(manager_runs[0].id, "manager-title-delete");
    assert_eq!(job_runs.list_job_runs_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        job_runs
            .list_job_runs_for_actor_calls
            .load(Ordering::SeqCst),
        1
    );
    assert_eq!(domain_events.list_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn delete_title_leaves_active_submission_for_unmatched_import() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let download_queue_commands = Arc::new(TrackingDownloadQueueCommandRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking_and_queue_commands(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases.clone(),
        download_queue_commands.clone(),
    );

    let created = app
        .add_title(
            &user,
            NewTitle {
                name: "Delete Me".into(),
                facet: MediaFacet::Movie,
                monitored: false,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,

                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let active_submission = DownloadSubmission {
        download_id: scryer_domain::download_identity::DownloadId::new(),
        title_id: created.id.clone(),
        purpose: crate::DownloadSubmissionPurpose::Standard,
        facet: "movie".to_string(),
        download_client_id: Some("primary".to_string()),
        download_client_type: "sabnzbd".to_string(),
        download_client_item_id: "queue-active".to_string(),
        source_hint: None,
        source_provider_id: None,
        source_provider_name: None,
        source_kind: None,
        source_title: Some(created.name.clone()),
        info_hash: None,
        release_size_bytes: None,
        request_signature: None,
        scope: SubmissionScope::Title,
        release_listing_json: None,
    };
    let terminal_submission = DownloadSubmission {
        download_id: scryer_domain::download_identity::DownloadId::new(),
        title_id: created.id.clone(),
        purpose: crate::DownloadSubmissionPurpose::Standard,
        facet: "movie".to_string(),
        download_client_id: Some("primary".to_string()),
        download_client_type: "sabnzbd".to_string(),
        download_client_item_id: "queue-imported".to_string(),
        source_hint: None,
        source_provider_id: None,
        source_provider_name: None,
        source_kind: None,
        source_title: Some(created.name.clone()),
        info_hash: None,
        release_size_bytes: None,
        request_signature: None,
        scope: SubmissionScope::Title,
        release_listing_json: None,
    };
    download_submissions
        .record_submission(active_submission.clone())
        .await
        .expect("record active submission");
    download_submissions
        .update_tracked_state(
            &ClientJobLocator::from_submission(&active_submission),
            "downloading",
        )
        .await
        .expect("track active submission");
    download_submissions
        .record_submission(terminal_submission.clone())
        .await
        .expect("record terminal submission");
    download_submissions
        .update_tracked_state(
            &ClientJobLocator::from_submission(&terminal_submission),
            "imported",
        )
        .await
        .expect("track terminal submission");

    *download_client.queue_items.lock().await = vec![
        DownloadQueueItem {
            id: "queue-direct".to_string(),
            title_id: Some(created.id.clone()),
            episode_id: None,
            title_name: created.name.clone(),
            facet: Some("movie".to_string()),
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
            attention_reason_code: None,
            attention_reason: None,
            download_client_item_id: "queue-direct".to_string(),
            download_id: None,
            import_status: None,
            import_type: None,
            import_error_code: None,
            import_error_message: None,
            imported_at: None,
            delete_status: None,
            delete_error_message: None,
            source_provider: None,
            is_scryer_origin: true,
            tracked_state: None,
            tracked_status: None,
            tracked_status_messages: Vec::new(),
            tracked_match_type: None,
            seeding: None,
        },
        DownloadQueueItem {
            id: "queue-fallback".to_string(),
            title_id: None,
            episode_id: None,
            title_name: created.name.clone(),
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
            attention_reason_code: None,
            attention_reason: None,
            download_client_item_id: "queue-active".to_string(),
            download_id: None,
            import_status: None,
            import_type: None,
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
            seeding: None,
        },
        DownloadQueueItem {
            id: "queue-unrelated".to_string(),
            title_id: None,
            episode_id: None,
            title_name: "Other".to_string(),
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
            attention_reason_code: None,
            attention_reason: None,
            download_client_item_id: "queue-unrelated".to_string(),
            download_id: None,
            import_status: None,
            import_type: None,
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
            seeding: None,
        },
    ];

    app.services
        .catalog
        .libraries
        .set_app_permission_mask_for_user(&user.id, user.authorization.app)
        .await
        .expect("persist fixture administrator permissions");
    app.delete_title(&user, &created.id, false, None)
        .await
        .expect("delete title");

    assert_eq!(*download_client.queue_calls.lock().await, 0);
    assert_eq!(*download_client.history_calls.lock().await, 0);
    assert!(download_client.deleted_items.lock().await.is_empty());
    let queued_commands = download_queue_commands.queued.lock().await.clone();
    assert!(queued_commands.is_empty());
    assert_eq!(
        pending_releases.deleted_title_ids.lock().await.clone(),
        vec![created.id.clone()]
    );
    assert_eq!(
        download_submissions.deleted_title_ids.lock().await.clone(),
        vec![created.id.clone()]
    );
    assert!(
        download_submissions
            .store
            .lock()
            .await
            .iter()
            .all(|entry| entry.title_id != created.id)
    );
}

fn deleted_title_submission(
    title_id: &str,
    download_id: scryer_domain::download_identity::DownloadId,
    client_type: &str,
    item_id: &str,
) -> DownloadSubmission {
    DownloadSubmission {
        download_id,
        title_id: title_id.to_string(),
        purpose: crate::DownloadSubmissionPurpose::Standard,
        facet: "movie".to_string(),
        download_client_id: Some("primary".to_string()),
        download_client_type: client_type.to_string(),
        download_client_item_id: item_id.to_string(),
        source_hint: None,
        source_provider_id: None,
        source_provider_name: None,
        source_kind: None,
        source_title: Some("Synthetic.Movie.2031.1080p".to_string()),
        info_hash: None,
        release_size_bytes: None,
        request_signature: None,
        release_listing_json: None,
        scope: SubmissionScope::Title,
    }
}

fn canonical_identity(
    download_id: scryer_domain::download_identity::DownloadId,
) -> crate::DownloadSubmissionIdentity {
    crate::DownloadSubmissionIdentity {
        download_id: Some(download_id.to_wire()),
    }
}

fn synthetic_movie(name: &str) -> NewTitle {
    NewTitle {
        name: name.into(),
        facet: MediaFacet::Movie,
        monitored: false,
        tags: vec![],
        external_ids: vec![],
        min_availability: None,
        ..Default::default()
    }
}

#[tokio::test]
async fn delete_title_keeps_live_downloads_unmatched_and_retires_completed_ones() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let download_queue_commands = Arc::new(TrackingDownloadQueueCommandRepo::default());
    let registry = Arc::new(super::downloads::RecordingDownloadRegistry::default());
    let (tracked_tx, mut tracked_rx) = tokio::sync::mpsc::channel(4);
    let (base_app, user) = bootstrap_with_cleanup_tracking_and_queue_commands(
        download_client,
        download_submissions.clone(),
        pending_releases,
        download_queue_commands.clone(),
    );
    let app = base_app.with_test_overrides(|services| {
        services
            .with_download_registry(registry.clone())
            .with_tracked_download_handle(crate::tracked_downloads::TrackedDownloadHandle::new(
                tracked_tx,
            ))
    });

    let created = app
        .add_title(&user, synthetic_movie("Synthetic Deleted Movie"))
        .await
        .expect("create title");
    let imported_id = scryer_domain::download_identity::DownloadId::new();
    let active_id = scryer_domain::download_identity::DownloadId::new();
    let imported = deleted_title_submission(&created.id, imported_id, "sabnzbd", "job-imported");
    let active = deleted_title_submission(&created.id, active_id, "sabnzbd", "job-active");
    for (submission, state) in [(&imported, "imported"), (&active, "downloading")] {
        let locator = ClientJobLocator::from_submission(submission);
        download_submissions
            .record_submission(submission.clone())
            .await
            .expect("record submission");
        download_submissions
            .update_tracked_state(&locator, state)
            .await
            .expect("record tracked state");
        registry.bind(locator, submission.download_id).await;
    }
    download_submissions
        .record_identity_tracked_state_for_download(
            Some(&imported_id),
            &canonical_identity(imported_id),
            Some(&ClientJobLocator::from_submission(&imported)),
            "imported",
            None,
            None,
        )
        .await
        .expect("record terminal marker");

    let tracked_responder = tokio::spawn(async move {
        match tracked_rx.recv().await.expect("forget-for-title command") {
            crate::tracked_downloads::TrackedDownloadCommand::ForgetMany {
                mut title_ids,
                download_ids,
            } => (title_ids.pop().expect("title id"), download_ids),
            _ => panic!("unexpected tracked-download command"),
        }
    });

    app.services
        .catalog
        .libraries
        .set_app_permission_mask_for_user(&user.id, user.authorization.app)
        .await
        .expect("persist fixture administrator permissions");
    app.delete_title(&user, &created.id, false, None)
        .await
        .expect("delete title");
    let (forgotten_title_id, mut forgotten_ids) =
        within_deadline("the forget-for-title command", tracked_responder)
            .await
            .expect("responder task");

    let queued_commands = download_queue_commands.queued.lock().await.clone();
    assert!(queued_commands.is_empty());

    assert!(registry.is_ended(&imported_id).await);
    assert!(
        !registry.is_ended(&active_id).await,
        "active download binding stays live so the poller can adopt it as unmatched"
    );
    let mut expected_ids = vec![imported_id, active_id];
    expected_ids.sort_unstable();
    let mut deleted_marker_ids = download_submissions
        .deleted_identity_state_download_ids
        .lock()
        .await
        .clone();
    deleted_marker_ids.sort_unstable();
    assert_eq!(deleted_marker_ids, expected_ids);
    assert_eq!(
        download_submissions
            .get_identity_tracked_state_for_download(
                Some(&imported_id),
                &canonical_identity(imported_id),
                None,
            )
            .await
            .expect("read terminal marker"),
        None
    );
    assert_eq!(forgotten_title_id, created.id);
    forgotten_ids.sort_unstable();
    assert_eq!(forgotten_ids, expected_ids);
}

#[tokio::test]
async fn delete_title_then_regrab_of_the_same_torrent_tracks_a_fresh_downloading_row() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let download_queue_commands = Arc::new(TrackingDownloadQueueCommandRepo::default());
    let registry = Arc::new(super::downloads::RecordingDownloadRegistry::default());
    let tracker = Arc::new(tokio::sync::Mutex::new(
        crate::tracked_downloads::TrackedDownloadService::new(),
    ));
    let (tracked_tx, mut tracked_rx) = tokio::sync::mpsc::channel(4);
    let responder_tracker = tracker.clone();
    let (forgotten_tx, mut forgotten_rx) = tokio::sync::mpsc::channel(1);
    tokio::spawn(async move {
        while let Some(command) = tracked_rx.recv().await {
            match command {
                crate::tracked_downloads::TrackedDownloadCommand::ForgetMany {
                    title_ids,
                    download_ids,
                } => {
                    for title_id in title_ids {
                        responder_tracker
                            .lock()
                            .await
                            .forget_for_title(&title_id, &download_ids);
                    }
                    forgotten_tx.send(()).await.unwrap();
                }
                _ => panic!("unexpected tracked-download command"),
            }
        }
    });
    let (base_app, user) = bootstrap_with_cleanup_tracking_and_queue_commands(
        download_client,
        download_submissions.clone(),
        pending_releases,
        download_queue_commands,
    );
    let app = base_app.with_test_overrides(|services| {
        services
            .with_download_registry(registry.clone())
            .with_tracked_download_handle(crate::tracked_downloads::TrackedDownloadHandle::new(
                tracked_tx,
            ))
    });

    // One torrent, one client item: the info hash is the native id both times.
    let info_hash = "0123456789abcdef0123456789abcdef01234567";
    let client_item = || {
        let mut item = queue_history_fixture_item(info_hash, DownloadQueueState::Downloading, 1);
        item.client_type = "qbittorrent".to_string();
        item.title_id = None;
        item.facet = None;
        item
    };
    let locator = ClientJobLocator::new(Some("primary"), "qbittorrent", info_hash);

    let deleted = app
        .add_title(&user, synthetic_movie("Synthetic Deleted Movie"))
        .await
        .expect("create deleted title");
    let dead_id = scryer_domain::download_identity::DownloadId::new();
    download_submissions
        .record_submission(deleted_title_submission(
            &deleted.id,
            dead_id,
            "qbittorrent",
            info_hash,
        ))
        .await
        .expect("record dead submission");
    registry.bind(locator.clone(), dead_id).await;
    download_submissions
        .record_identity_tracked_state_for_download(
            Some(&dead_id),
            &canonical_identity(dead_id),
            Some(&locator),
            "imported",
            None,
            None,
        )
        .await
        .expect("record terminal marker");

    let first = tracker
        .lock()
        .await
        .track(&app, client_item())
        .await
        .expect("dead download is tracked");
    assert_eq!(first.download_id, dead_id);
    assert_eq!(
        tracker
            .lock()
            .await
            .get_by_download_id(dead_id)
            .expect("dead row")
            .state,
        scryer_domain::TrackedDownloadState::Imported,
        "the terminal marker is what revives the dead download's outcome"
    );

    app.services
        .catalog
        .libraries
        .set_app_permission_mask_for_user(&user.id, user.authorization.app)
        .await
        .expect("persist fixture administrator permissions");
    app.delete_title(&user, &deleted.id, false, None)
        .await
        .expect("delete title");

    within_deadline("tracker forget applied", forgotten_rx.recv())
        .await
        .expect("notification");
    assert!(tracker.lock().await.get_by_download_id(dead_id).is_none());
    assert!(registry.is_ended(&dead_id).await);
    assert_eq!(
        download_submissions
            .get_identity_tracked_state_for_download(
                Some(&dead_id),
                &canonical_identity(dead_id),
                Some(&locator),
            )
            .await
            .expect("read terminal marker"),
        None
    );

    // The same torrent grabbed again for a new title.
    let regrabbed = app
        .add_title(&user, synthetic_movie("Synthetic Regrabbed Movie"))
        .await
        .expect("create new title");
    let regrab_id = scryer_domain::download_identity::DownloadId::new();
    download_submissions
        .record_submission(deleted_title_submission(
            &regrabbed.id,
            regrab_id,
            "qbittorrent",
            info_hash,
        ))
        .await
        .expect("record re-grab submission");
    registry.bind(locator.clone(), regrab_id).await;

    let outcome = tracker
        .lock()
        .await
        .track(&app, client_item())
        .await
        .expect("re-grab is tracked");
    assert_eq!(outcome.download_id, regrab_id);
    assert!(outcome.newly_tracked);
    let tracker = tracker.lock().await;
    assert_eq!(tracker.get_all().len(), 1);
    let row = tracker.get_by_download_id(regrab_id).expect("re-grab row");
    assert_eq!(row.state, scryer_domain::TrackedDownloadState::Downloading);
    assert_eq!(row.title_id.as_deref(), Some(regrabbed.id.as_str()));
}

#[tokio::test]
async fn delete_title_keeps_bound_and_unbound_active_grabs_available_for_import() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    download_submissions
        .list_for_title_requires_item_id
        .store(true, Ordering::SeqCst);
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let download_queue_commands = Arc::new(TrackingDownloadQueueCommandRepo::default());
    let registry = Arc::new(super::downloads::RecordingDownloadRegistry::default());
    let (tracked_tx, mut tracked_rx) = tokio::sync::mpsc::channel(4);
    let (base_app, user) = bootstrap_with_cleanup_tracking_and_queue_commands(
        download_client,
        download_submissions.clone(),
        pending_releases,
        download_queue_commands.clone(),
    );
    let app = base_app.with_test_overrides(|services| {
        services
            .with_download_registry(registry.clone())
            .with_tracked_download_handle(crate::tracked_downloads::TrackedDownloadHandle::new(
                tracked_tx,
            ))
    });

    let created = app
        .add_title(&user, synthetic_movie("Synthetic Deleted Movie"))
        .await
        .expect("create title");
    let bound_id = scryer_domain::download_identity::DownloadId::new();
    let unbound_id = scryer_domain::download_identity::DownloadId::new();
    let bound = deleted_title_submission(&created.id, bound_id, "sabnzbd", "job-bound");
    let unbound = deleted_title_submission(&created.id, unbound_id, "sabnzbd", "");
    download_submissions
        .record_submission(bound.clone())
        .await
        .expect("record bound submission");
    download_submissions
        .update_tracked_state(&ClientJobLocator::from_submission(&bound), "downloading")
        .await
        .expect("track bound submission");
    registry
        .bind(ClientJobLocator::from_submission(&bound), bound_id)
        .await;
    download_submissions
        .record_submission(unbound)
        .await
        .expect("record unbound submission");

    let tracked_responder = tokio::spawn(async move {
        match tracked_rx.recv().await.expect("forget-for-title command") {
            crate::tracked_downloads::TrackedDownloadCommand::ForgetMany {
                download_ids, ..
            } => download_ids,
            _ => panic!("unexpected tracked-download command"),
        }
    });

    app.services
        .catalog
        .libraries
        .set_app_permission_mask_for_user(&user.id, user.authorization.app)
        .await
        .expect("persist fixture administrator permissions");
    app.delete_title(&user, &created.id, false, None)
        .await
        .expect("delete title");
    let mut forgotten_ids = within_deadline("the forget-for-title command", tracked_responder)
        .await
        .expect("responder task");

    assert!(
        !registry.is_ended(&bound_id).await,
        "active bound download stays live after title deletion"
    );
    assert!(!registry.is_ended(&unbound_id).await);
    let mut expected_ids = vec![bound_id, unbound_id];
    expected_ids.sort_unstable();
    forgotten_ids.sort_unstable();
    assert_eq!(forgotten_ids, expected_ids);
    let mut deleted_marker_ids = download_submissions
        .deleted_identity_state_download_ids
        .lock()
        .await
        .clone();
    deleted_marker_ids.sort_unstable();
    assert_eq!(deleted_marker_ids, expected_ids);
    let queued_commands = download_queue_commands.queued.lock().await.clone();
    assert!(queued_commands.is_empty());
}

/// The tracked-download commands a title merge sends, as the tests see them.
#[derive(Debug, PartialEq, Eq)]
enum MergeTrackedCommand {
    Reassign {
        from_title_id: String,
        to_title_id: String,
        facet: Option<String>,
        download_ids: Vec<scryer_domain::download_identity::DownloadId>,
    },
    Forget {
        title_id: String,
        download_ids: Vec<scryer_domain::download_identity::DownloadId>,
    },
}

/// Answer exactly `count` tracked-download commands, returning them in order
/// with their download ids sorted.
fn answer_merge_tracked_commands(
    mut tracked_rx: tokio::sync::mpsc::Receiver<crate::tracked_downloads::TrackedDownloadCommand>,
    count: usize,
) -> tokio::task::JoinHandle<Vec<MergeTrackedCommand>> {
    tokio::spawn(async move {
        let mut seen = Vec::new();
        for _ in 0..count {
            match tracked_rx.recv().await.expect("tracked-download command") {
                crate::tracked_downloads::TrackedDownloadCommand::ReassignTitle {
                    from_title_id,
                    to_title_id,
                    facet,
                    mut download_ids,
                    reply,
                } => {
                    let _ = reply.send(Ok(download_ids.clone()));
                    download_ids.sort_unstable();
                    seen.push(MergeTrackedCommand::Reassign {
                        from_title_id,
                        to_title_id,
                        facet,
                        download_ids,
                    });
                }
                crate::tracked_downloads::TrackedDownloadCommand::ForgetForTitle {
                    title_id,
                    mut download_ids,
                    reply,
                } => {
                    let _ = reply.send(Ok(download_ids.clone()));
                    download_ids.sort_unstable();
                    seen.push(MergeTrackedCommand::Forget {
                        title_id,
                        download_ids,
                    });
                }
                _ => panic!("unexpected tracked-download command"),
            }
        }
        seen
    })
}

/// Record a download of `title_id` the client is still holding: its
/// submission, tracked state, terminal marker, client binding, and a pending
/// cleanup row.
async fn record_held_download(
    download_submissions: &TrackingDownloadSubmissionRepo,
    registry: &super::downloads::RecordingDownloadRegistry,
    title_id: &str,
    item_id: &str,
    scope: SubmissionScope,
    tracked_state: &str,
) -> scryer_domain::download_identity::DownloadId {
    let download_id = scryer_domain::download_identity::DownloadId::new();
    let mut submission = deleted_title_submission(title_id, download_id, "qbittorrent", item_id);
    submission.scope = scope;
    let locator = ClientJobLocator::from_submission(&submission);
    download_submissions
        .record_submission(submission)
        .await
        .expect("record submission");
    download_submissions
        .update_tracked_state(&locator, tracked_state)
        .await
        .expect("record tracked state");
    download_submissions
        .record_identity_tracked_state_for_download(
            Some(&download_id),
            &canonical_identity(download_id),
            Some(&locator),
            tracked_state,
            None,
            None,
        )
        .await
        .expect("record terminal marker");
    registry.bind(locator, download_id).await;
    download_submissions
        .pending_cleanup
        .lock()
        .await
        .insert(download_id);
    download_id
}

async fn submission_of(
    download_submissions: &TrackingDownloadSubmissionRepo,
    download_id: scryer_domain::download_identity::DownloadId,
) -> Option<DownloadSubmission> {
    download_submissions
        .store
        .lock()
        .await
        .iter()
        .find(|entry| entry.download_id == download_id)
        .cloned()
}

#[tokio::test]
async fn merge_hands_the_source_titles_downloads_to_the_destination() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let download_queue_commands = Arc::new(TrackingDownloadQueueCommandRepo::default());
    let registry = Arc::new(super::downloads::RecordingDownloadRegistry::default());
    let (tracked_tx, tracked_rx) = tokio::sync::mpsc::channel(4);
    let (base_app, user) = bootstrap_with_cleanup_tracking_and_queue_commands(
        download_client.clone(),
        download_submissions.clone(),
        Arc::new(TrackingPendingReleaseRepo::default()),
        download_queue_commands.clone(),
    );
    let app = base_app.with_test_overrides(|services| {
        services
            .with_download_registry(registry.clone())
            .with_tracked_download_handle(crate::tracked_downloads::TrackedDownloadHandle::new(
                tracked_tx,
            ))
    });
    let source = app
        .add_title(&user, synthetic_movie("Synthetic Merge Source"))
        .await
        .expect("create source title");
    let destination = app
        .add_title(&user, synthetic_movie("Synthetic Merge Destination"))
        .await
        .expect("create destination title");
    let bystander = app
        .add_title(&user, synthetic_movie("Synthetic Merge Bystander"))
        .await
        .expect("create bystander title");

    let title_scoped = record_held_download(
        &download_submissions,
        &registry,
        &source.id,
        "merge-title-scoped",
        SubmissionScope::Title,
        "imported_seeding",
    )
    .await;
    let episode_scoped = record_held_download(
        &download_submissions,
        &registry,
        &source.id,
        "merge-episode-scoped",
        SubmissionScope::Episode {
            episode_id: "source-episode-1".to_string(),
        },
        "imported_seeding",
    )
    .await;
    let unrelated = record_held_download(
        &download_submissions,
        &registry,
        &bystander.id,
        "merge-bystander",
        SubmissionScope::Episode {
            episode_id: "source-episode-1".to_string(),
        },
        "imported_seeding",
    )
    .await;
    let map = crate::location::merge::map::MergeIdentityMap {
        source_title_id: source.id.clone(),
        destination_title_id: destination.id.clone(),
        episodes: [(
            "source-episode-1".to_string(),
            "destination-episode-1".to_string(),
        )]
        .into_iter()
        .collect(),
        ..Default::default()
    };

    let tracked_responder = answer_merge_tracked_commands(tracked_rx, 2);
    app.purge_merged_source_dependent_records(&map)
        .await
        .expect("retire the merged source title");
    let commands = within_deadline("the merge's tracked-download commands", tracked_responder)
        .await
        .expect("responder task");

    let mut moved_ids = vec![title_scoped, episode_scoped];
    moved_ids.sort_unstable();
    assert_eq!(
        commands,
        vec![
            MergeTrackedCommand::Reassign {
                from_title_id: source.id.clone(),
                to_title_id: destination.id.clone(),
                facet: Some("movie".to_string()),
                download_ids: moved_ids.clone(),
            },
            // The forget that retires the source's remaining cached rows names
            // no download: every one of them moved.
            MergeTrackedCommand::Forget {
                title_id: source.id.clone(),
                download_ids: Vec::new(),
            },
        ]
    );

    // The submissions belong to the destination, with their episode
    // reference carried through the merge's map.
    let moved_title_scoped = submission_of(&download_submissions, title_scoped)
        .await
        .expect("the title-scoped submission survives the merge");
    assert_eq!(moved_title_scoped.title_id, destination.id);
    assert_eq!(moved_title_scoped.scope, SubmissionScope::Title);
    let moved_episode_scoped = submission_of(&download_submissions, episode_scoped)
        .await
        .expect("the episode-scoped submission survives the merge");
    assert_eq!(moved_episode_scoped.title_id, destination.id);
    assert_eq!(
        moved_episode_scoped.scope,
        SubmissionScope::Episode {
            episode_id: "destination-episode-1".to_string(),
        }
    );

    // Bindings, tracked state and pending cleanup are kept, so the seeding
    // cleanup still removes the torrents once their goal is met.
    for download_id in [title_scoped, episode_scoped, unrelated] {
        assert!(!registry.is_ended(&download_id).await);
        assert!(
            download_submissions
                .pending_cleanup
                .lock()
                .await
                .contains(&download_id)
        );
        assert_eq!(
            download_submissions
                .get_identity_tracked_state_for_download(
                    Some(&download_id),
                    &canonical_identity(download_id),
                    None,
                )
                .await
                .expect("read terminal marker")
                .as_deref(),
            Some("imported_seeding")
        );
    }
    assert!(
        download_submissions
            .finished_cleanup
            .lock()
            .await
            .is_empty(),
        "no cleanup row is abandoned"
    );
    assert!(
        download_submissions
            .deleted_identity_state_download_ids
            .lock()
            .await
            .is_empty()
    );

    // The unrelated title's download is exactly as it was.
    let untouched = submission_of(&download_submissions, unrelated)
        .await
        .expect("the bystander's submission is kept");
    assert_eq!(untouched.title_id, bystander.id);
    assert_eq!(
        untouched.scope,
        SubmissionScope::Episode {
            episode_id: "source-episode-1".to_string(),
        }
    );
    assert!(
        download_submissions
            .reassigned
            .lock()
            .await
            .iter()
            .all(|reassignment| reassignment.references.download_id != unrelated)
    );

    // Nothing reached the client.
    assert!(download_queue_commands.queued.lock().await.is_empty());
    assert!(download_client.deleted_requests.lock().await.is_empty());
}

#[tokio::test]
async fn merge_retires_a_source_download_it_cannot_move_and_still_succeeds() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let download_queue_commands = Arc::new(TrackingDownloadQueueCommandRepo::default());
    let registry = Arc::new(super::downloads::RecordingDownloadRegistry::default());
    let (tracked_tx, tracked_rx) = tokio::sync::mpsc::channel(4);
    let (base_app, user) = bootstrap_with_cleanup_tracking_and_queue_commands(
        download_client.clone(),
        download_submissions.clone(),
        Arc::new(TrackingPendingReleaseRepo::default()),
        download_queue_commands.clone(),
    );
    let app = base_app.with_test_overrides(|services| {
        services
            .with_download_registry(registry.clone())
            .with_tracked_download_handle(crate::tracked_downloads::TrackedDownloadHandle::new(
                tracked_tx,
            ))
    });
    let source = app
        .add_title(&user, synthetic_movie("Synthetic Merge Source"))
        .await
        .expect("create source title");
    let destination = app
        .add_title(&user, synthetic_movie("Synthetic Merge Destination"))
        .await
        .expect("create destination title");

    let movable = record_held_download(
        &download_submissions,
        &registry,
        &source.id,
        "merge-movable",
        SubmissionScope::Title,
        "imported_seeding",
    )
    .await;
    // Its episode has no counterpart in the merge's map.
    let unmapped = record_held_download(
        &download_submissions,
        &registry,
        &source.id,
        "merge-unmapped",
        SubmissionScope::Episode {
            episode_id: "source-episode-unmapped".to_string(),
        },
        "imported_seeding",
    )
    .await;
    // Its move fails in the store. It is still active in the client, which
    // the delete path would cancel; a merge never does.
    let failing = record_held_download(
        &download_submissions,
        &registry,
        &source.id,
        "merge-failing",
        SubmissionScope::Title,
        "downloading",
    )
    .await;
    download_submissions
        .failing_reassignments
        .lock()
        .await
        .insert(failing);
    let map = crate::location::merge::map::MergeIdentityMap {
        source_title_id: source.id.clone(),
        destination_title_id: destination.id.clone(),
        ..Default::default()
    };

    let tracked_responder = answer_merge_tracked_commands(tracked_rx, 2);
    app.purge_merged_source_dependent_records(&map)
        .await
        .expect("a download that cannot move never fails the merge");
    let commands = within_deadline("the merge's tracked-download commands", tracked_responder)
        .await
        .expect("responder task");

    let mut retired_ids = vec![unmapped, failing];
    retired_ids.sort_unstable();
    assert_eq!(
        commands,
        vec![
            MergeTrackedCommand::Reassign {
                from_title_id: source.id.clone(),
                to_title_id: destination.id.clone(),
                facet: Some("movie".to_string()),
                download_ids: vec![movable],
            },
            MergeTrackedCommand::Forget {
                title_id: source.id.clone(),
                download_ids: retired_ids.clone(),
            },
        ]
    );

    // The downloads that could not move retire exactly as a deleted title's:
    // binding ended, terminal markers removed, cleanup settled as abandoned,
    // submission gone.
    for download_id in &retired_ids {
        assert!(registry.is_ended(download_id).await);
        assert!(
            submission_of(&download_submissions, *download_id)
                .await
                .is_none()
        );
    }
    let mut abandoned = download_submissions.finished_cleanup.lock().await.clone();
    abandoned.sort_unstable_by_key(|(download_id, _, _)| *download_id);
    assert_eq!(
        abandoned,
        retired_ids
            .iter()
            .map(|download_id| (*download_id, "cleanup_abandoned".to_string(), true))
            .collect::<Vec<_>>()
    );
    let mut deleted_marker_ids = download_submissions
        .deleted_identity_state_download_ids
        .lock()
        .await
        .clone();
    deleted_marker_ids.sort_unstable();
    assert_eq!(deleted_marker_ids, retired_ids);

    // The movable one still went to the destination with its cleanup pending.
    let moved = submission_of(&download_submissions, movable)
        .await
        .expect("the movable submission survives the merge");
    assert_eq!(moved.title_id, destination.id);
    assert!(!registry.is_ended(&movable).await);
    assert!(
        download_submissions
            .pending_cleanup
            .lock()
            .await
            .contains(&movable)
    );

    // No failure removes anything from the client.
    assert!(download_queue_commands.queued.lock().await.is_empty());
    assert!(download_client.deleted_requests.lock().await.is_empty());
}

async fn queue_synthetic_import(
    imports: &TrackingImportRepo,
    item_id: &str,
    payload: serde_json::Value,
    status: scryer_domain::ImportStatus,
    canonical: Option<scryer_domain::download_identity::DownloadId>,
) -> String {
    let id = imports
        .queue_import_request(
            ClientJobLocator::new(Some("primary"), "sabnzbd", item_id),
            "manual_import".to_string(),
            payload.to_string(),
        )
        .await
        .unwrap();
    if status != scryer_domain::ImportStatus::Pending {
        imports
            .update_import_status(&id, status, None)
            .await
            .unwrap();
    }
    if let Some(canonical) = canonical {
        imports
            .canonical_ids
            .lock()
            .await
            .insert(id.clone(), canonical);
    }
    id
}

#[tokio::test]
async fn title_delete_takes_its_pending_imports_and_password_retry_with_it() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("library");
    let folder = root.join("Synthetic Retry Movie");
    std::fs::create_dir_all(&folder).unwrap();
    let media = folder.join("movie.mkv");
    std::fs::write(&media, b"synthetic media").unwrap();
    let other_folder = root.join("Synthetic Bystander Movie");
    std::fs::create_dir_all(&other_folder).unwrap();
    let other_media = other_folder.join("movie.mkv");
    std::fs::write(&other_media, b"synthetic bystander").unwrap();
    let (base_app, user, _) = bootstrap_with_cutoff_projection_state(
        Arc::new(StoredSettingsRepo::default()),
        Arc::new(StoredQualityProfileRepo::default()),
        Arc::new(MockMediaFileRepo::default()),
    );
    let submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let registry = Arc::new(super::downloads::RecordingDownloadRegistry::default());
    let imports = Arc::new(TrackingImportRepo::default());
    let app = base_app.with_test_overrides(|services| {
        services
            .with_download_submissions(submissions.clone())
            .with_download_registry(registry.clone())
            .with_imports(imports.clone())
    });
    app.update_media_settings(
        &user,
        MediaFacet::Movie,
        empty_update_media_settings_with_roots(vec![build_root_folder_entry(&root, true)]),
    )
    .await
    .unwrap();
    let title = app
        .add_title(&user, synthetic_movie("Synthetic Retry Movie"))
        .await
        .unwrap();
    let other = app
        .add_title(&user, synthetic_movie("Synthetic Bystander Movie"))
        .await
        .unwrap();
    for (owner, path, file) in [
        (&title, &folder, &media),
        (&other, &other_folder, &other_media),
    ] {
        app.services
            .catalog
            .titles
            .set_folder_path(&owner.id, &path.to_string_lossy())
            .await
            .unwrap();
        app.services
            .library
            .media_files
            .insert_media_file(&InsertMediaFileInput {
                title_id: owner.id.clone(),
                file_path: file.to_string_lossy().into_owned(),
                size_bytes: 15,
                role: MediaFileRole::Primary,
                ..Default::default()
            })
            .await
            .unwrap();
    }

    // The deleted title: a download awaiting a password retry, plus a second
    // download whose import is being retried automatically.
    let retry_id = scryer_domain::download_identity::DownloadId::new();
    let auto_id = scryer_domain::download_identity::DownloadId::new();
    for (id, job) in [(retry_id, "retry-job"), (auto_id, "auto-job")] {
        submissions
            .record_submission(deleted_title_submission(&title.id, id, "sabnzbd", job))
            .await
            .unwrap();
        registry
            .bind(ClientJobLocator::new(Some("primary"), "sabnzbd", job), id)
            .await;
    }
    registry.password_retry_ids.lock().await.insert(retry_id);
    let password_import = queue_synthetic_import(
        &imports,
        "retry-job",
        serde_json::json!({}),
        scryer_domain::ImportStatus::Pending,
        None,
    )
    .await;
    let auto_import = queue_synthetic_import(
        &imports,
        "auto-job",
        serde_json::json!({}),
        scryer_domain::ImportStatus::Processing,
        Some(auto_id),
    )
    .await;
    let manual_import = queue_synthetic_import(
        &imports,
        "manual-job",
        serde_json::json!({ "title_id": title.id }),
        scryer_domain::ImportStatus::Pending,
        None,
    )
    .await;
    let finished_import = queue_synthetic_import(
        &imports,
        "manual-job",
        serde_json::json!({ "title_id": title.id }),
        scryer_domain::ImportStatus::Completed,
        None,
    )
    .await;
    *imports.manual_import_selection.lock().await = Some(crate::ManualImportSelection {
        id: "synthetic-selection".to_string(),
        actor_user_id: user.id.clone(),
        title_id: title.id.clone(),
        source_identity: ClientJobLocator::new(Some("primary"), "sabnzbd", "manual-job"),
        canonical_download_id: None,
        release_evidence_json: None,
        trusted_source_root: temp.path().to_string_lossy().into_owned(),
        archive_workspace_root: None,
        candidates: Vec::new(),
    });

    // The bystander title keeps its own fenced download and pending imports.
    let other_id = scryer_domain::download_identity::DownloadId::new();
    submissions
        .record_submission(deleted_title_submission(
            &other.id,
            other_id,
            "sabnzbd",
            "other-job",
        ))
        .await
        .unwrap();
    registry
        .bind(
            ClientJobLocator::new(Some("primary"), "sabnzbd", "other-job"),
            other_id,
        )
        .await;
    registry.password_retry_ids.lock().await.insert(other_id);
    let other_download_import = queue_synthetic_import(
        &imports,
        "other-job",
        serde_json::json!({}),
        scryer_domain::ImportStatus::Processing,
        Some(other_id),
    )
    .await;
    let other_manual_import = queue_synthetic_import(
        &imports,
        "other-manual-job",
        serde_json::json!({ "title_id": other.id }),
        scryer_domain::ImportStatus::Pending,
        None,
    )
    .await;

    let preview = app
        .preview_delete_title_files(&user, &title.id)
        .await
        .unwrap();
    app.delete_title(
        &user,
        &title.id,
        true,
        Some(DeleteExecutionConfirmation {
            preview_fingerprint: preview.fingerprint,
            typed_confirmation: None,
        }),
    )
    .await
    .unwrap();

    assert!(
        app.services
            .catalog
            .titles
            .get_by_id(&title.id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(!media.exists());
    assert!(registry.is_ended(&retry_id).await);
    assert!(registry.is_ended(&auto_id).await);
    assert!(!registry.password_retry_ids.lock().await.contains(&retry_id));
    let remaining = imports
        .records
        .lock()
        .await
        .iter()
        .map(|record| record.id.clone())
        .collect::<HashSet<_>>();
    for gone in [&password_import, &auto_import, &manual_import] {
        assert!(!remaining.contains(gone), "pending import {gone} survived");
    }
    assert!(
        remaining.contains(&finished_import),
        "finished import history is kept"
    );
    assert!(imports.manual_import_selection.lock().await.is_none());

    // Nothing of the bystander title changed.
    assert!(remaining.contains(&other_download_import));
    assert!(remaining.contains(&other_manual_import));
    assert_eq!(std::fs::read(&other_media).unwrap(), b"synthetic bystander");
    assert!(!registry.is_ended(&other_id).await);
    assert!(registry.password_retry_ids.lock().await.contains(&other_id));
    assert_eq!(
        submissions.list_for_title(&other.id).await.unwrap().len(),
        1
    );
    assert!(
        app.services
            .catalog
            .titles
            .get_by_id(&other.id)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn policy_title_cleanup_retires_password_retry_binding() {
    let (base_app, user) = bootstrap();
    let submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let registry = Arc::new(super::downloads::RecordingDownloadRegistry::default());
    let app = base_app.with_test_overrides(|services| {
        services
            .with_download_submissions(submissions.clone())
            .with_download_registry(registry.clone())
    });
    let title = app
        .add_title(&user, synthetic_movie("Synthetic Legacy Cleanup"))
        .await
        .unwrap();
    let id = scryer_domain::download_identity::DownloadId::new();
    submissions
        .record_submission(deleted_title_submission(
            &title.id,
            id,
            "sabnzbd",
            "retry-job",
        ))
        .await
        .unwrap();
    registry.password_retry_ids.lock().await.insert(id);
    // Library/policy cleanup retires a pending password retry with the title,
    // so no live binding outlives its submission.
    app.purge_title_dependent_records(&title.id, (&user).into())
        .await
        .unwrap();
    assert!(registry.is_ended(&id).await);
    assert!(!registry.password_retry_ids.lock().await.contains(&id));
    assert!(
        submissions
            .list_for_title(&title.id)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn password_retry_claim_and_title_delete_exclude_each_other() {
    within_deadline(
        "password_retry_claim_and_title_delete_exclude_each_other",
        async {
            let (base_app, user) = bootstrap();
            let submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
            let (tx, _rx) = tokio::sync::mpsc::channel(1);
            let app = base_app.with_test_overrides(|services| {
                services
                    .with_download_submissions(submissions.clone())
                    .with_tracked_download_handle(
                        crate::tracked_downloads::TrackedDownloadHandle::new(tx),
                    )
            });
            let title = app
                .add_title(&user, synthetic_movie("Synthetic Claim Guard"))
                .await
                .unwrap();
            app.services
                .catalog
                .libraries
                .set_app_permission_mask_for_user(&user.id, user.authorization.app)
                .await
                .unwrap();
            let id = scryer_domain::download_identity::DownloadId::new();
            let submission = deleted_title_submission(&title.id, id, "sabnzbd", "claim-job");
            let source = ClientJobLocator::from_submission(&submission);
            submissions.record_submission(submission).await.unwrap();
            let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
            let (release_tx, release_rx) = tokio::sync::oneshot::channel();
            *submissions.password_claim_gate.lock().await = Some((entered_tx, release_rx));
            let guard = app
                .runtime
                .jobs
                .interactive_operation_guards
                .try_acquire(&format!("title-delete:{}", title.id))
                .await
                .unwrap();
            let error = app
                .retry_download_password(&user, id, source.clone(), "synthetic-pass")
                .await
                .unwrap_err();
            assert_eq!(
                error.validation_reason(),
                Some(crate::title_delete::IN_PROGRESS)
            );
            assert!(
                submissions.password_claim_gate.lock().await.is_some(),
                "refused retry never reached claim storage"
            );
            drop(guard);
            let retry_app = app.clone();
            let retry_user = user.clone();
            let retry = tokio::spawn(async move {
                retry_app
                    .retry_download_password(&retry_user, id, source, "synthetic-pass")
                    .await
            });
            within_deadline("password claim admission entered", entered_rx)
                .await
                .unwrap();
            let error = app
                .delete_title(&user, &title.id, false, None)
                .await
                .unwrap_err();
            assert_eq!(
                error.validation_reason(),
                Some(crate::title_delete::IN_PROGRESS)
            );
            release_tx.send(()).unwrap();
            assert!(
                within_deadline("password claim finishes", retry)
                    .await
                    .unwrap()
                    .is_err()
            );
            assert!(
                app.services
                    .catalog
                    .titles
                    .get_by_id(&title.id)
                    .await
                    .unwrap()
                    .is_some()
            );
        },
    )
    .await;
}

#[tokio::test]
async fn explicit_queue_removal_recovers_password_retry_fenced_download() {
    within_deadline(
        "explicit_queue_removal_recovers_password_retry_fenced_download",
        async {
            let client = Arc::new(StubDownloadClient::default());
            // A job removed before observation can make any adapter return an error.
            // Local cancellation must still recover the durable retry fence.
            client
                .set_delete_error(Some("synthetic client job is absent"))
                .await;
            let submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
            let commands = Arc::new(TrackingDownloadQueueCommandRepo::default());
            let (base_app, user) = bootstrap_with_cleanup_tracking_and_queue_commands(
                client.clone(),
                submissions.clone(),
                Arc::new(TrackingPendingReleaseRepo::default()),
                commands.clone(),
            );
            let registry = Arc::new(super::downloads::RecordingDownloadRegistry::default());
            *submissions.linked_registry.lock().await = Some(registry.clone());
            let app = base_app
                .with_test_overrides(|services| services.with_download_registry(registry.clone()));
            app.services
                .catalog
                .libraries
                .set_app_permission_mask_for_user(&user.id, user.authorization.app)
                .await
                .unwrap();
            let title = app
                .add_title(&user, synthetic_movie("Synthetic Removed Retry"))
                .await
                .unwrap();
            let id = scryer_domain::download_identity::DownloadId::new();
            let submission =
                deleted_title_submission(&title.id, id, "sabnzbd", "removed-retry-job");
            let locator = ClientJobLocator::from_submission(&submission);
            submissions
                .password_retry_recovery
                .lock()
                .await
                .push(submission.clone());
            submissions.record_submission(submission).await.unwrap();
            registry.bind(locator.clone(), id).await;
            registry.password_retry_ids.lock().await.insert(id);
            assert_eq!(
                app.list_download_import_page(&user, 50, 0, DownloadImportFilter::All)
                    .await
                    .unwrap()
                    .items
                    .len(),
                1
            );
            let command = app
                .delete_download_queue_item(
                    &user,
                    locator.client_id.as_deref(),
                    &locator.client_type,
                    &locator.item_id,
                    false,
                )
                .await
                .unwrap();
            assert_eq!(command.canonical_download_id, Some(id));
            let token = tokio_util::sync::CancellationToken::new();
            let task = tokio::spawn(start_background_download_delete_poller(
                app.clone(),
                token.child_token(),
            ));
            crate::test_wait::wait_until("explicit retry cancellation completed", || async {
                commands.get(&command.id).await.is_some_and(|row| {
                    row.status == scryer_domain::DownloadQueueDeleteStatus::Completed
                })
            })
            .await;
            token.cancel();
            within_deadline("retry cancellation poller stops", task)
                .await
                .unwrap();
            assert!(
                registry.is_ended(&id).await,
                "explicit removal ends the fenced binding"
            );
            // The recovery row cannot say whether the job is still queued or
            // already in history, so the client is asked to remove it from both.
            assert_eq!(
                *client.delete_attempts.lock().await,
                vec![
                    ("removed-retry-job".to_string(), false),
                    ("removed-retry-job".to_string(), true),
                ]
            );
            assert!(
                submissions
                    .find_by_canonical_download_id(&id)
                    .await
                    .unwrap()
                    .is_none()
            );
            assert!(
                app.list_download_import_page(&user, 50, 0, DownloadImportFilter::All)
                    .await
                    .unwrap()
                    .items
                    .is_empty(),
                "successful queue removal invalidates a recovery-only cached Activity row"
            );
            app.delete_title(&user, &title.id, false, None)
                .await
                .unwrap();
            assert!(
                app.services
                    .catalog
                    .titles
                    .get_by_id(&title.id)
                    .await
                    .unwrap()
                    .is_none()
            );
        },
    )
    .await;
}

#[tokio::test]
async fn recovery_row_removal_requires_permission_in_the_titles_library() {
    within_deadline(
        "recovery_row_removal_requires_permission_in_the_titles_library",
        async {
            let client = Arc::new(StubDownloadClient::default());
            let submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
            let commands = Arc::new(TrackingDownloadQueueCommandRepo::default());
            let (base_app, user) = bootstrap_with_cleanup_tracking_and_queue_commands(
                client,
                submissions.clone(),
                Arc::new(TrackingPendingReleaseRepo::default()),
                commands.clone(),
            );
            let registry = Arc::new(super::downloads::RecordingDownloadRegistry::default());
            *submissions.linked_registry.lock().await = Some(registry.clone());
            let app = base_app
                .with_test_overrides(|services| services.with_download_registry(registry.clone()));
            let title = app
                .add_title(&user, synthetic_movie("Synthetic Guarded Retry"))
                .await
                .unwrap();
            let other_library_title = app
                .add_title(
                    &user,
                    NewTitle {
                        facet: MediaFacet::Series,
                        ..synthetic_movie("Synthetic Other Library")
                    },
                )
                .await
                .unwrap();
            assert_ne!(title.library_id, other_library_title.library_id);
            let id = scryer_domain::download_identity::DownloadId::new();
            let submission =
                deleted_title_submission(&title.id, id, "sabnzbd", "guarded-retry-job");
            let locator = ClientJobLocator::from_submission(&submission);
            submissions
                .password_retry_recovery
                .lock()
                .await
                .push(submission.clone());
            submissions.record_submission(submission).await.unwrap();
            registry.bind(locator.clone(), id).await;
            registry.password_retry_ids.lock().await.insert(id);

            let outsider = library_permission_user(
                "retry-outsider",
                &other_library_title.library_id,
                &[scryer_domain::LibraryPermission::ManageTitles],
            );
            let error = app
                .delete_download_queue_item(
                    &outsider,
                    locator.client_id.as_deref(),
                    &locator.client_type,
                    &locator.item_id,
                    false,
                )
                .await
                .unwrap_err();
            assert!(
                matches!(
                    error,
                    AppError::Unauthorized(_) | AppError::UnauthorizedReason { .. }
                ),
                "unexpected error: {error:?}"
            );
            assert_eq!(registry.download_id_for(&locator).await, Some(id));
            assert!(!registry.is_ended(&id).await);
            assert!(
                submissions
                    .find_by_canonical_download_id(&id)
                    .await
                    .unwrap()
                    .is_some()
            );

            let manager = library_permission_user(
                "retry-manager",
                &title.library_id,
                &[scryer_domain::LibraryPermission::ManageTitles],
            );
            let command = app
                .delete_download_queue_item(
                    &manager,
                    locator.client_id.as_deref(),
                    &locator.client_type,
                    &locator.item_id,
                    false,
                )
                .await
                .unwrap();
            assert_eq!(command.canonical_download_id, Some(id));
        },
    )
    .await;
}

#[tokio::test]
async fn password_retry_recovery_is_visible_with_empty_runtime_after_restart() {
    within_deadline("persisted retry recovery projection", async {
        let (base_app, user) = bootstrap();
        let submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
        let app = base_app.with_test_overrides(|services| {
            services.with_download_submissions(submissions.clone())
        });
        let title = app
            .add_title(&user, synthetic_movie("Synthetic Restart Retry"))
            .await
            .unwrap();
        let hidden = app
            .add_title(
                &user,
                NewTitle {
                    facet: MediaFacet::Series,
                    ..synthetic_movie("Synthetic Hidden Retry")
                },
            )
            .await
            .unwrap();
        let viewer = library_permission_user(
            "retry-viewer",
            &title.library_id,
            &[
                scryer_domain::LibraryPermission::View,
                scryer_domain::LibraryPermission::ResolveImports,
            ],
        );
        submissions
            .password_retry_recovery
            .lock()
            .await
            .push(deleted_title_submission(
                &hidden.id,
                scryer_domain::download_identity::DownloadId::new(),
                "sabnzbd",
                "hidden-retry-job",
            ));
        let id = scryer_domain::download_identity::DownloadId::new();
        submissions
            .password_retry_recovery
            .lock()
            .await
            .push(deleted_title_submission(
                &title.id,
                id,
                "sabnzbd",
                "missing-retry-job",
            ));
        assert!(
            app.runtime
                .acquisition
                .tracked_download_snapshot
                .read()
                .await
                .is_empty()
        );
        assert!(
            app.runtime
                .acquisition
                .download_queue_snapshot
                .snapshot()
                .await
                .items
                .is_empty()
        );
        for _ in 0..2 {
            let rows = app
                .list_download_import_page(&viewer, 50, 0, DownloadImportFilter::All)
                .await
                .unwrap()
                .items;
            assert_eq!(
                rows.len(),
                1,
                "initial rebuild and cache hit both retain recovery row"
            );
            assert_eq!(rows[0].download_id.as_deref(), Some(id.to_wire().as_str()));
            assert_eq!(rows[0].download_client_item_id, "missing-retry-job");
            assert_eq!(rows[0].client_id, "primary");
            assert_eq!(
                rows[0].tracked_state,
                Some(TrackedDownloadState::ImportBlocked)
            );
        }
    })
    .await;
}

#[tokio::test]
async fn empty_password_retry_recovery_is_not_requeried_on_native_revisions() {
    within_deadline("recovery query skipped on native revisions", async {
        let (base_app, user) = bootstrap();
        let submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
        let app = base_app.with_test_overrides(|services| {
            services.with_download_submissions(submissions.clone())
        });
        let reads = || {
            submissions
                .password_retry_recovery_reads
                .load(std::sync::atomic::Ordering::SeqCst)
        };
        let list = || async {
            app.list_download_import_page(&user, 50, 0, DownloadImportFilter::All)
                .await
                .unwrap()
                .items
        };
        assert!(list().await.is_empty());
        let initial_reads = reads();
        assert!(initial_reads >= 1, "first build queries recovery rows");
        let snapshot_cache = &app.runtime.acquisition.download_queue_snapshot;
        let revision = snapshot_cache.snapshot().await.revision;
        snapshot_cache.stage_success(Vec::new()).await;
        snapshot_cache.commit_pending_for_test().await;
        assert_ne!(snapshot_cache.snapshot().await.revision, revision);
        assert!(list().await.is_empty());
        assert_eq!(
            reads(),
            initial_reads,
            "a native-revision-only rebuild reuses the empty recovery result"
        );
        app.invalidate_download_queue_read_model().await;
        assert!(list().await.is_empty());
        assert_eq!(
            reads(),
            initial_reads + 1,
            "explicit invalidation re-queries recovery rows"
        );
    })
    .await;
}

#[tokio::test]
async fn password_retry_cache_tracks_claim_and_refusal_without_native_revision() {
    within_deadline("password retry read-model lifecycle", async {
        let client = Arc::new(StubDownloadClient::default());
        let submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
        submissions
            .simulate_password_retry
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let (base_app, user) = bootstrap_with_cleanup_tracking(
            client.clone(),
            submissions.clone(),
            Arc::new(TrackingPendingReleaseRepo::default()),
        );
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        let app = base_app.with_test_overrides(|services| {
            services.with_tracked_download_handle(
                crate::tracked_downloads::TrackedDownloadHandle::new(tx),
            )
        });
        let title = app
            .add_title(&user, synthetic_movie("Synthetic Retry Cache"))
            .await
            .unwrap();
        let id = scryer_domain::download_identity::DownloadId::new();
        let submission = deleted_title_submission(&title.id, id, "sabnzbd", "cache-retry-job");
        let locator = ClientJobLocator::from_submission(&submission);
        submissions.record_submission(submission).await.unwrap();
        assert!(
            app.list_download_import_page(&user, 50, 0, DownloadImportFilter::All)
                .await
                .unwrap()
                .items
                .is_empty()
        );
        let revision = app
            .runtime
            .acquisition
            .download_queue_snapshot
            .snapshot()
            .await
            .revision;
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        *client.password_retry_gate.lock().await = Some((entered_tx, release_rx));
        let retry_app = app.clone();
        let retry_user = user.clone();
        let retry = tokio::spawn(async move {
            retry_app
                .retry_download_password(&retry_user, id, locator, "synthetic-pass")
                .await
        });
        within_deadline("retry client entered", entered_rx)
            .await
            .unwrap();
        let held = app
            .list_download_import_page(&user, 50, 0, DownloadImportFilter::All)
            .await
            .unwrap();
        assert_eq!(
            held.items.len(),
            1,
            "new durable claim invalidates cached empty projection"
        );
        assert_eq!(
            held.items[0].download_id.as_deref(),
            Some(id.to_wire().as_str())
        );
        release_tx.send(()).unwrap();
        assert!(matches!(
            within_deadline("retry refusal completed", retry)
                .await
                .unwrap()
                .unwrap(),
            crate::DownloadClientRetryOutcome::Refused
        ));
        assert!(
            app.list_download_import_page(&user, 50, 0, DownloadImportFilter::All)
                .await
                .unwrap()
                .items
                .is_empty(),
            "definitive refusal invalidates cached recovery row"
        );
        assert_eq!(
            app.runtime
                .acquisition
                .download_queue_snapshot
                .snapshot()
                .await
                .revision,
            revision
        );
    })
    .await;
}

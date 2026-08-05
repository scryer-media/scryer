use super::*;

#[tokio::test]
async fn notification_broadcast_ignores_operational_domain_events() {
    let (app, _) = bootstrap();
    let mut receiver = app.runtime.events.notification_event_broadcast.subscribe();

    app.append_domain_event(crate::domain_events::new_global_domain_event(
        None,
        DomainEventPayload::JobRunStarted(JobRunStartedEventData {
            run_id: "run-1".to_string(),
            job_key: "rss_sync".to_string(),
            operation_type: "job".to_string(),
            trigger_source: "system_internal".to_string(),
        }),
    ))
    .await
    .expect("operational event should append");

    assert!(
        matches!(
            receiver.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
        ),
        "operational events should not wake notification dispatcher"
    );

    let notification = app
        .append_domain_event(crate::domain_events::new_global_domain_event(
            None,
            DomainEventPayload::TitleAdded(scryer_domain::TitleAddedEventData {
                title: scryer_domain::TitleContextSnapshot {
                    title_name: "Wake Fixture".to_string(),
                    facet: MediaFacet::Movie,
                    year: Some(2024),
                    poster_url: None,
                    external_ids: scryer_domain::DomainExternalIds::default(),
                },
            }),
        ))
        .await
        .expect("notification event should append");

    let wake = receiver
        .recv()
        .await
        .expect("notification wake should arrive after notification event");
    assert_eq!(wake, notification.sequence);
}

#[tokio::test]
async fn notification_broadcast_wakes_once_for_notification_batches() {
    let (app, _) = bootstrap();
    let mut receiver = app.runtime.events.notification_event_broadcast.subscribe();

    let stored = app
        .append_domain_events(vec![
            crate::domain_events::new_global_domain_event(
                None,
                DomainEventPayload::JobRunStarted(JobRunStartedEventData {
                    run_id: "run-1".to_string(),
                    job_key: "rss_sync".to_string(),
                    operation_type: "job".to_string(),
                    trigger_source: "system_internal".to_string(),
                }),
            ),
            crate::domain_events::new_global_domain_event(
                None,
                DomainEventPayload::TitleAdded(scryer_domain::TitleAddedEventData {
                    title: scryer_domain::TitleContextSnapshot {
                        title_name: "First Notification".to_string(),
                        facet: MediaFacet::Movie,
                        year: Some(2024),
                        poster_url: None,
                        external_ids: scryer_domain::DomainExternalIds::default(),
                    },
                }),
            ),
            crate::domain_events::new_global_domain_event(
                None,
                DomainEventPayload::ImportRejected(scryer_domain::ImportRejectedEventData {
                    title: Some(scryer_domain::TitleContextSnapshot {
                        title_name: "Second Notification".to_string(),
                        facet: MediaFacet::Movie,
                        year: Some(2024),
                        poster_url: None,
                        external_ids: scryer_domain::DomainExternalIds::default(),
                    }),
                    status: ImportStatus::Failed,
                    import_id: None,
                    source_system: Some("download_client".to_string()),
                    source_ref: Some("queue-2".to_string()),
                    source_title: Some("Second.Notification.1080p".to_string()),
                    source_path: Some("/downloads/example.mkv".to_string()),
                    dest_path: None,
                    quality: Some("1080p".to_string()),
                    reason: Some("not parsable".to_string()),
                    skip_reason: None,
                    episode_ids: Vec::new(),
                }),
            ),
            crate::domain_events::new_global_domain_event(
                None,
                DomainEventPayload::JobRunCompleted(JobRunCompletedEventData {
                    run_id: "run-1".to_string(),
                    job_key: "rss_sync".to_string(),
                    summary_text: Some("done".to_string()),
                }),
            ),
        ])
        .await
        .expect("batch should append");

    let wake = tokio::time::timeout(Duration::from_secs(1), receiver.recv())
        .await
        .expect("notification wake should arrive")
        .expect("notification broadcast should stay open");
    assert_eq!(
        wake,
        stored.last().expect("batch should have events").sequence,
        "mixed batches should publish a high-water wake hint"
    );

    assert!(
        matches!(
            receiver.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
        ),
        "a mixed batch should emit one notification wake, not one per notification event"
    );
}

#[tokio::test]
async fn acquisition_cycle_retries_standby_candidate_after_failed_grab() {
    let download_client = Arc::new(StubDownloadClient::default());
    let info_hash = "abcdef0123456789abcdef0123456789abcdef01";
    download_client.set_grab_info_hash(Some(info_hash)).await;
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases.clone(),
        wanted_items.clone(),
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Failure Recovery".into(),
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

    let wanted = AcquisitionScopeState {
        id: Id::new().0,
        title_id: title.id.clone(),
        title_name: Some(title.name.clone()),
        title_slug: None,
        title_facet: None,
        library_id: None,
        library_name: None,
        library_slug: None,
        episode_id: None,
        collection_id: None,
        series_movie_link_id: None,
        season_number: None,
        episode_number: None,
        media_type: "movie".to_string(),
        last_search_at: Some((Utc::now() - chrono::Duration::minutes(5)).to_rfc3339()),
        status: AcquisitionScopeStatus::Grabbed,
        grabbed_release: Some(
            serde_json::json!({
                "title": "Failed.Release.1080p.WEB-DL",
                "score": 100,
                "grabbed_at": Utc::now().to_rfc3339(),
            })
            .to_string(),
        ),
        current_score: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
    };
    wanted_items
        .upsert_acquisition_scope_state(&wanted)
        .await
        .expect("seed wanted item");

    pending_releases
        .insert_pending_release(&PendingRelease {
            id: Id::new().0,
            wanted_item_id: wanted.id.clone(),
            title_id: title.id.clone(),
            release_title: "Standby.Release.1080p.WEB-DL".to_string(),
            release_url: Some("https://example.com/standby.torrent".to_string()),
            source_kind: Some(DownloadSourceKind::TorrentFile),
            release_size_bytes: Some(1_000),
            release_score: 150,
            scoring_log_json: None,
            indexer_source: Some("torrent_rss".to_string()),
            release_guid: Some("guid-standby".to_string()),
            added_at: Utc::now().to_rfc3339(),
            delay_until: Utc::now().to_rfc3339(),
            status: PendingReleaseStatus::Standby,
            grabbed_at: None,
            source_password: None,
            published_at: Some(Utc::now().to_rfc3339()),
            info_hash: Some(info_hash.to_string()),
        })
        .await
        .expect("seed standby");

    download_submissions
        .record_submission(DownloadSubmission {
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "movie".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "failed-job".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Failed.Release.1080p.WEB-DL".to_string()),
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record failed submission");

    *download_client.history_items.lock().await = vec![failed_history_item(
        "failed-job",
        "Failed.Release.1080p.WEB-DL",
    )];

    app.run_convergence_cycle_once().await;

    let updated = wanted_items
        .get_acquisition_scope_state_by_id(&wanted.id)
        .await
        .expect("get wanted")
        .expect("wanted exists");
    assert_eq!(updated.status, AcquisitionScopeStatus::Grabbed);
    assert_eq!(updated.current_score, None);
    assert!(
        updated
            .grabbed_release
            .as_deref()
            .unwrap_or_default()
            .contains("Standby.Release.1080p.WEB-DL")
    );

    assert!(
        pending_releases
            .list_all_standby_pending_releases()
            .await
            .expect("list standby")
            .is_empty()
    );
    assert!(pending_releases.store.lock().await.iter().any(|release| {
        release.release_title == "Standby.Release.1080p.WEB-DL"
            && release.status == PendingReleaseStatus::Grabbed
    }));

    let submissions = download_submissions.store.lock().await.clone();
    assert!(submissions.iter().any(|submission| {
        submission.download_client_item_id == "failed-job"
            && submission.source_title.as_deref() == Some("Failed.Release.1080p.WEB-DL")
    }));
    assert_eq!(
        download_submissions
            .get_tracked_state(&DownloadSourceIdentity::new(
                Some("primary"),
                "nzbget",
                "failed-job",
            ))
            .await
            .expect("load tracked state")
            .as_deref(),
        Some("failed")
    );
    assert!(submissions.iter().any(|submission| {
        submission.download_client_item_id == format!("job-for-{}", title.id)
            && submission.source_title.as_deref() == Some("Standby.Release.1080p.WEB-DL")
            && submission.request_signature.as_deref()
                == Some(
                    "torrent_file|https://example.com/standby.torrent|Standby.Release.1080p.WEB-DL",
                )
    }));
    let identities = download_submissions.identities.lock().await;
    assert!(
        identities
            .values()
            .any(|identity| { identity.download_id.as_deref() == Some(info_hash) })
    );

    assert_eq!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .clone(),
        vec!["Standby.Release.1080p.WEB-DL".to_string()]
    );
}

#[tokio::test]
async fn tracked_download_failure_reuses_standby_recovery_policy() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases.clone(),
        wanted_items.clone(),
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Tracked Failure Recovery".into(),
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

    let wanted = AcquisitionScopeState {
        id: Id::new().0,
        title_id: title.id.clone(),
        title_name: Some(title.name.clone()),
        title_slug: None,
        title_facet: None,
        library_id: None,
        library_name: None,
        library_slug: None,
        episode_id: None,
        collection_id: None,
        series_movie_link_id: None,
        season_number: None,
        episode_number: None,
        media_type: "movie".to_string(),
        last_search_at: Some((Utc::now() - chrono::Duration::minutes(5)).to_rfc3339()),
        status: AcquisitionScopeStatus::Grabbed,
        grabbed_release: Some(
            serde_json::json!({
                "title": "Failed.Release.1080p.WEB-DL",
                "score": 100,
                "grabbed_at": Utc::now().to_rfc3339(),
            })
            .to_string(),
        ),
        current_score: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
    };
    wanted_items
        .upsert_acquisition_scope_state(&wanted)
        .await
        .expect("seed wanted item");

    pending_releases
        .insert_pending_release(&PendingRelease {
            id: Id::new().0,
            wanted_item_id: wanted.id.clone(),
            title_id: title.id.clone(),
            release_title: "Standby.Release.1080p.WEB-DL".to_string(),
            release_url: Some("https://example.com/standby.nzb".to_string()),
            source_kind: Some(DownloadSourceKind::NzbUrl),
            release_size_bytes: Some(1_000),
            release_score: 150,
            scoring_log_json: None,
            indexer_source: Some("nzbgeek".to_string()),
            release_guid: Some("guid-standby".to_string()),
            added_at: Utc::now().to_rfc3339(),
            delay_until: Utc::now().to_rfc3339(),
            status: PendingReleaseStatus::Standby,
            grabbed_at: None,
            source_password: None,
            published_at: Some(Utc::now().to_rfc3339()),
            info_hash: None,
        })
        .await
        .expect("seed standby");

    download_submissions
        .record_submission(DownloadSubmission {
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "movie".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "failed-job".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Failed.Release.1080p.WEB-DL".to_string()),
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record failed submission");

    let mut tracked_download = crate::tracked_downloads::TrackedDownload {
        id: "nzbget:failed-job".to_string(),
        client_id: "primary".to_string(),
        client_type: "nzbget".to_string(),
        client_item: failed_history_item("failed-job", "Failed.Release.1080p.WEB-DL"),
        state: scryer_domain::TrackedDownloadState::FailedPending,
        status: scryer_domain::TrackedDownloadStatus::Error,
        status_messages: Vec::new(),
        title_id: Some(title.id.clone()),
        facet: Some("movie".to_string()),
        source_title: Some("Failed.Release.1080p.WEB-DL".to_string()),
        indexer: None,
        added_at: None,
        notified_manual_interaction: false,
        match_type: scryer_domain::TitleMatchType::Submission,
        is_trackable: true,
        import_attempted: false,
        waiting_for_completed_history: false,
        path_missing_since: None,
        no_video_import_retry: None,
        foreign_import_classification: None,
        skip_reacquire_on_failure: false,
        snapshot_missing_since: None,
    };

    crate::failed_download_handler::process_failed(&app, &mut tracked_download).await;

    assert_eq!(
        tracked_download.state,
        scryer_domain::TrackedDownloadState::Failed
    );

    let updated = wanted_items
        .get_acquisition_scope_state_by_id(&wanted.id)
        .await
        .expect("get wanted")
        .expect("wanted exists");
    assert_eq!(updated.status, AcquisitionScopeStatus::Grabbed);
    assert!(
        updated
            .grabbed_release
            .as_deref()
            .unwrap_or_default()
            .contains("Standby.Release.1080p.WEB-DL")
    );

    assert!(
        pending_releases
            .list_all_standby_pending_releases()
            .await
            .expect("list standby")
            .is_empty()
    );
    assert!(pending_releases.store.lock().await.iter().any(|release| {
        release.release_title == "Standby.Release.1080p.WEB-DL"
            && release.status == PendingReleaseStatus::Grabbed
    }));

    let submissions = download_submissions.store.lock().await.clone();
    assert!(submissions.iter().any(|submission| {
        submission.download_client_item_id == "failed-job"
            && submission.source_title.as_deref() == Some("Failed.Release.1080p.WEB-DL")
    }));
    assert_eq!(
        download_submissions
            .get_tracked_state(&DownloadSourceIdentity::new(
                Some("primary"),
                "nzbget",
                "failed-job",
            ))
            .await
            .expect("load tracked state")
            .as_deref(),
        Some("failed")
    );
    assert!(submissions.iter().any(|submission| {
        submission.download_client_item_id == format!("job-for-{}", title.id)
            && submission.source_title.as_deref() == Some("Standby.Release.1080p.WEB-DL")
            && submission.request_signature.as_deref()
                == Some("nzb_url|https://example.com/standby.nzb|Standby.Release.1080p.WEB-DL")
    }));

    assert_eq!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .clone(),
        vec!["Standby.Release.1080p.WEB-DL".to_string()]
    );
}

#[tokio::test]
async fn tracked_download_failure_keeps_standby_when_submit_unavailable() {
    let download_client = Arc::new(StubDownloadClient::default());
    download_client
        .set_submit_error(Some(StubSubmitError::SubmitUnavailable(
            "download client unavailable".to_string(),
        )))
        .await;
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking(
        download_client,
        download_submissions.clone(),
        pending_releases.clone(),
        wanted_items.clone(),
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Tracked Failure Deferred Recovery".into(),
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

    let wanted = AcquisitionScopeState {
        id: Id::new().0,
        title_id: title.id.clone(),
        title_name: Some(title.name.clone()),
        title_slug: None,
        title_facet: None,
        library_id: None,
        library_name: None,
        library_slug: None,
        episode_id: None,
        collection_id: None,
        series_movie_link_id: None,
        season_number: None,
        episode_number: None,
        media_type: "movie".to_string(),
        last_search_at: Some((Utc::now() - chrono::Duration::minutes(5)).to_rfc3339()),
        status: AcquisitionScopeStatus::Grabbed,
        grabbed_release: Some(
            serde_json::json!({
                "title": "Failed.Release.1080p.WEB-DL",
                "score": 100,
                "grabbed_at": Utc::now().to_rfc3339(),
            })
            .to_string(),
        ),
        current_score: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
    };
    wanted_items
        .upsert_acquisition_scope_state(&wanted)
        .await
        .expect("seed wanted item");

    let standby = pending_movie_release(
        &wanted.id,
        &title,
        "Standby.Deferred.Release.1080p.WEB-DL",
        PendingReleaseStatus::Standby,
    );
    let standby_id = standby.id.clone();
    pending_releases
        .insert_pending_release(&standby)
        .await
        .expect("seed standby");

    download_submissions
        .record_submission(DownloadSubmission {
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "movie".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "failed-job".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Failed.Release.1080p.WEB-DL".to_string()),
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record failed submission");

    let outcome = crate::acquisition_workflow::process_download_failure(
        &app,
        crate::acquisition_workflow::DownloadFailureContext {
            wanted_item: Some(wanted.clone()),
            title_id: Some(title.id.clone()),
            client_id: "primary".to_string(),
            client_type: "nzbget".to_string(),
            client_name: Some("Primary".to_string()),
            client_item_id: "failed-job".to_string(),
            release_title: "Failed.Release.1080p.WEB-DL".to_string(),
            reason: "download failed".to_string(),
            remove_from_client_if_configured: false,
            skip_reacquire: false,
        },
        None,
    )
    .await;

    assert_eq!(
        outcome,
        crate::acquisition_workflow::FailureHandlingOutcome::RequeuedDeferred
    );
    assert_eq!(
        pending_releases
            .get_pending_release(&standby_id)
            .await
            .expect("load standby")
            .expect("standby exists")
            .status,
        PendingReleaseStatus::Standby
    );
    let updated_wanted = wanted_items
        .get_acquisition_scope_state_by_id(&wanted.id)
        .await
        .expect("load wanted")
        .expect("wanted exists");
    // A submit-unavailable failure defers to the standby recovery
    // rather than re-opening — the grabbed state row is untouched (no reopen,
    // no reschedule) while the standby is preserved for the retry.
    assert_eq!(updated_wanted.status, AcquisitionScopeStatus::Grabbed);
    assert!(
        !download_submissions
            .store
            .lock()
            .await
            .iter()
            .any(|submission| submission.source_title.as_deref()
                == Some("Standby.Deferred.Release.1080p.WEB-DL"))
    );
}

#[tokio::test]
async fn process_download_failure_returns_already_handled_for_duplicate_failed_download() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking(
        download_client,
        download_submissions.clone(),
        pending_releases,
        wanted_items.clone(),
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Duplicate Failed Download".into(),
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

    let wanted = AcquisitionScopeState {
        id: Id::new().0,
        title_id: title.id.clone(),
        title_name: Some(title.name.clone()),
        title_slug: None,
        title_facet: None,
        library_id: None,
        library_name: None,
        library_slug: None,
        episode_id: None,
        collection_id: None,
        series_movie_link_id: None,
        season_number: None,
        episode_number: None,
        media_type: "movie".to_string(),
        last_search_at: Some((Utc::now() - chrono::Duration::minutes(10)).to_rfc3339()),
        status: AcquisitionScopeStatus::Grabbed,
        grabbed_release: Some(
            serde_json::json!({
                "title": "Duplicate.Failed.Release.1080p.WEB-DL",
                "score": 100,
                "grabbed_at": Utc::now().to_rfc3339(),
            })
            .to_string(),
        ),
        current_score: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
    };
    wanted_items
        .upsert_acquisition_scope_state(&wanted)
        .await
        .expect("seed wanted item");
    let wanted_id = wanted.id.clone();

    download_submissions
        .record_submission(DownloadSubmission {
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "movie".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "failed-duplicate".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Duplicate.Failed.Release.1080p.WEB-DL".to_string()),
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record failed submission");

    let first = crate::acquisition_workflow::process_download_failure(
        &app,
        crate::acquisition_workflow::DownloadFailureContext {
            wanted_item: Some(wanted.clone()),
            title_id: Some(title.id.clone()),
            client_id: "primary".to_string(),
            client_type: "nzbget".to_string(),
            client_name: Some("Primary".to_string()),
            client_item_id: "failed-duplicate".to_string(),
            release_title: "Duplicate.Failed.Release.1080p.WEB-DL".to_string(),
            reason: "download failed".to_string(),
            remove_from_client_if_configured: false,
            skip_reacquire: false,
        },
        None,
    )
    .await;
    assert_ne!(
        first,
        crate::acquisition_workflow::FailureHandlingOutcome::AlreadyHandled
    );

    let second = crate::acquisition_workflow::process_download_failure(
        &app,
        crate::acquisition_workflow::DownloadFailureContext {
            wanted_item: Some(wanted),
            title_id: Some(title.id.clone()),
            client_id: "primary".to_string(),
            client_type: "nzbget".to_string(),
            client_name: Some("Primary".to_string()),
            client_item_id: "failed-duplicate".to_string(),
            release_title: "Duplicate.Failed.Release.1080p.WEB-DL".to_string(),
            reason: "download failed".to_string(),
            remove_from_client_if_configured: false,
            skip_reacquire: false,
        },
        None,
    )
    .await;
    assert_eq!(
        second,
        crate::acquisition_workflow::FailureHandlingOutcome::AlreadyHandled
    );

    assert_eq!(
        wanted_items.status_update_call_count_for(&wanted_id).await,
        1,
        "duplicate failure should not reschedule the wanted item twice"
    );

    let blocklist = app
        .services
        .workflow
        .blocklist_repo
        .list_for_title(&title.id, 10)
        .await
        .expect("list blocklist");
    assert_eq!(blocklist.len(), 1);
    assert_eq!(
        blocklist[0].download_id.as_deref(),
        Some("failed-duplicate")
    );

    let failed_attempts = app
        .services
        .workflow
        .release_attempts
        .list_failed_release_signatures_for_title(&title.id, 10)
        .await
        .expect("list failed release attempts");
    assert_eq!(failed_attempts.len(), 1);

    let history = app
        .list_title_history(
            &user,
            &TitleHistoryFilter {
                event_types: Some(vec![
                    TitleHistoryEventType::DownloadFailed,
                    TitleHistoryEventType::Blocklisted,
                ]),
                title_ids: Some(vec![title.id.clone()]),
                library_ids: None,
                title_search: None,
                download_id: Some("failed-duplicate".to_string()),
                episode_id: None,
                group_by_event: false,
                limit: 10,
                offset: 0,
            },
        )
        .await
        .expect("list title history");
    assert_eq!(history.total_count, 2);
}

#[tokio::test]
async fn process_download_failure_skip_reacquire_records_failure_without_due_search() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking(
        download_client,
        download_submissions.clone(),
        pending_releases,
        wanted_items.clone(),
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Manual Failed Only".into(),
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

    let wanted = AcquisitionScopeState {
        id: Id::new().0,
        title_id: title.id.clone(),
        title_name: Some(title.name.clone()),
        title_slug: None,
        title_facet: None,
        library_id: None,
        library_name: None,
        library_slug: None,
        episode_id: None,
        collection_id: None,
        series_movie_link_id: None,
        season_number: None,
        episode_number: None,
        media_type: "movie".to_string(),
        last_search_at: Some((Utc::now() - chrono::Duration::minutes(10)).to_rfc3339()),
        status: AcquisitionScopeStatus::Grabbed,
        grabbed_release: Some(
            serde_json::json!({
                "title": "Manual.Failed.Only.1080p.WEB-DL",
                "score": 100,
                "grabbed_at": Utc::now().to_rfc3339(),
            })
            .to_string(),
        ),
        current_score: Some(100),
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
    };
    wanted_items
        .upsert_acquisition_scope_state(&wanted)
        .await
        .expect("seed wanted item");

    download_submissions
        .record_submission(DownloadSubmission {
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "movie".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "failed-only".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Manual.Failed.Only.1080p.WEB-DL".to_string()),
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record failed submission");

    let outcome = crate::acquisition_workflow::process_download_failure(
        &app,
        crate::acquisition_workflow::DownloadFailureContext {
            wanted_item: Some(wanted.clone()),
            title_id: Some(title.id.clone()),
            client_id: "primary".to_string(),
            client_type: "nzbget".to_string(),
            client_name: Some("Primary".to_string()),
            client_item_id: "failed-only".to_string(),
            release_title: "Manual.Failed.Only.1080p.WEB-DL".to_string(),
            reason: "download failed".to_string(),
            remove_from_client_if_configured: false,
            skip_reacquire: true,
        },
        None,
    )
    .await;

    assert_eq!(
        outcome,
        crate::acquisition_workflow::FailureHandlingOutcome::RecordedNoReacquire
    );

    let updated_wanted = wanted_items
        .get_acquisition_scope_state_by_id(&wanted.id)
        .await
        .expect("get wanted")
        .expect("wanted item");
    assert_eq!(updated_wanted.status, AcquisitionScopeStatus::Wanted);
    assert!(updated_wanted.grabbed_release.is_none());

    let blocklist = app
        .services
        .workflow
        .blocklist_repo
        .list_for_title(&title.id, 10)
        .await
        .expect("list blocklist");
    assert_eq!(blocklist.len(), 1);
    assert_eq!(blocklist[0].download_id.as_deref(), Some("failed-only"));
}

#[tokio::test]
async fn process_download_failure_dedupes_same_release_title_across_client_item_ids() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking(
        download_client,
        download_submissions.clone(),
        pending_releases,
        wanted_items,
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Friends".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    for (item_id, source_hint, source_title) in [
        (
            "weaver-1",
            "weaver://job/weaver-1",
            "Friends.S05.720p.BluRay.DD5.1.x264-NTb",
        ),
        (
            "weaver-2",
            "weaver://job/weaver-2",
            " friends.s05.720p.bluray.dd5.1.x264-ntb ",
        ),
    ] {
        download_submissions
            .record_submission(DownloadSubmission {
                title_id: title.id.clone(),
                purpose: crate::DownloadSubmissionPurpose::Standard,
                facet: "series".to_string(),
                download_client_id: Some("primary".to_string()),
                download_client_type: "weaver".to_string(),
                download_client_item_id: item_id.to_string(),
                source_hint: Some(source_hint.to_string()),
                source_provider_id: None,
                source_provider_name: None,
                source_kind: None,
                source_title: Some(source_title.to_string()),
                request_signature: None,
                scope: SubmissionScope::Title,
            })
            .await
            .expect("record failed submission");
    }

    let first = crate::acquisition_workflow::process_download_failure(
        &app,
        crate::acquisition_workflow::DownloadFailureContext {
            wanted_item: None,
            title_id: Some(title.id.clone()),
            client_id: "primary".to_string(),
            client_type: "weaver".to_string(),
            client_name: Some("Primary".to_string()),
            client_item_id: "weaver-1".to_string(),
            release_title: "Friends".to_string(),
            reason: "download failed".to_string(),
            remove_from_client_if_configured: false,
            skip_reacquire: false,
        },
        None,
    )
    .await;
    assert_ne!(
        first,
        crate::acquisition_workflow::FailureHandlingOutcome::AlreadyHandled
    );

    let second = crate::acquisition_workflow::process_download_failure(
        &app,
        crate::acquisition_workflow::DownloadFailureContext {
            wanted_item: None,
            title_id: Some(title.id.clone()),
            client_id: "primary".to_string(),
            client_type: "weaver".to_string(),
            client_name: Some("Primary".to_string()),
            client_item_id: "weaver-2".to_string(),
            release_title: "Friends".to_string(),
            reason: "download failed".to_string(),
            remove_from_client_if_configured: false,
            skip_reacquire: false,
        },
        None,
    )
    .await;
    assert_eq!(
        second,
        crate::acquisition_workflow::FailureHandlingOutcome::AlreadyHandled
    );

    let blocklist = app
        .services
        .workflow
        .blocklist_repo
        .list_for_title(&title.id, 10)
        .await
        .expect("list blocklist");
    assert_eq!(blocklist.len(), 1);
    assert_eq!(
        blocklist[0].source_title.as_deref(),
        Some("friends.s05.720p.bluray.dd5.1.x264-ntb")
    );

    let failed_attempts = app
        .services
        .workflow
        .release_attempts
        .list_failed_release_signatures_for_title(&title.id, 10)
        .await
        .expect("list failed release attempts");
    assert_eq!(failed_attempts.len(), 1);
}

#[tokio::test]
async fn tracked_download_failure_prefers_tracked_source_title_for_blocklist_identity() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking(
        download_client,
        download_submissions.clone(),
        pending_releases,
        wanted_items,
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Friends".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    download_submissions
        .record_submission(DownloadSubmission {
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "series".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "weaver".to_string(),
            download_client_item_id: "job-1".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Friends.S05.720p.BluRay.DD5.1.x264-NTb".to_string()),
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record failed submission");

    let mut tracked_download = crate::tracked_downloads::TrackedDownload {
        id: "weaver:job-1".to_string(),
        client_id: "primary".to_string(),
        client_type: "weaver".to_string(),
        client_item: failed_history_item("job-1", "Friends"),
        state: scryer_domain::TrackedDownloadState::FailedPending,
        status: scryer_domain::TrackedDownloadStatus::Error,
        status_messages: Vec::new(),
        title_id: Some(title.id.clone()),
        facet: Some("series".to_string()),
        source_title: Some("Friends.S05.720p.BluRay.DD5.1.x264-NTb".to_string()),
        indexer: None,
        added_at: None,
        notified_manual_interaction: false,
        match_type: scryer_domain::TitleMatchType::Submission,
        is_trackable: true,
        import_attempted: false,
        waiting_for_completed_history: false,
        path_missing_since: None,
        no_video_import_retry: None,
        foreign_import_classification: None,
        skip_reacquire_on_failure: false,
        snapshot_missing_since: None,
    };

    crate::failed_download_handler::process_failed(&app, &mut tracked_download).await;

    assert_eq!(
        tracked_download.state,
        scryer_domain::TrackedDownloadState::Failed
    );

    let blocklist = app
        .services
        .workflow
        .blocklist_repo
        .list_for_title(&title.id, 10)
        .await
        .expect("list blocklist");
    assert_eq!(blocklist.len(), 1);
    assert_eq!(
        blocklist[0].source_title.as_deref(),
        Some("friends.s05.720p.bluray.dd5.1.x264-ntb")
    );

    let failed_attempts = app
        .services
        .workflow
        .release_attempts
        .list_failed_release_signatures_for_title(&title.id, 10)
        .await
        .expect("list failed release attempts");
    assert_eq!(failed_attempts.len(), 1);
    assert_eq!(
        failed_attempts[0].source_title.as_deref(),
        Some("friends.s05.720p.bluray.dd5.1.x264-ntb")
    );
}

#[tokio::test]
async fn parse_matched_foreign_failed_download_does_not_blocklist_or_requeue() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking(
        download_client,
        download_submissions,
        pending_releases,
        wanted_items.clone(),
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Foreign Failure Safety".into(),
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

    let wanted = AcquisitionScopeState {
        id: Id::new().0,
        title_id: title.id.clone(),
        title_name: Some(title.name.clone()),
        title_slug: None,
        title_facet: None,
        library_id: None,
        library_name: None,
        library_slug: None,
        episode_id: None,
        collection_id: None,
        series_movie_link_id: None,
        season_number: None,
        episode_number: None,
        media_type: "movie".to_string(),
        last_search_at: Some((Utc::now() - chrono::Duration::minutes(10)).to_rfc3339()),
        status: AcquisitionScopeStatus::Grabbed,
        grabbed_release: Some(
            serde_json::json!({
                "title": "Scryer.Grabbed.Release.1080p.WEB-DL",
                "score": 100,
                "grabbed_at": Utc::now().to_rfc3339(),
            })
            .to_string(),
        ),
        current_score: Some(100),
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
    };
    wanted_items
        .upsert_acquisition_scope_state(&wanted)
        .await
        .expect("seed wanted item");

    let mut client_item = failed_history_item(
        "foreign-failed-job",
        "Foreign.Failure.Safety.2024.1080p.WEB-DL",
    );
    client_item.is_scryer_origin = false;
    let mut tracked_download = crate::tracked_downloads::TrackedDownload {
        id: "nzbget:foreign-failed-job".to_string(),
        client_id: "primary".to_string(),
        client_type: "nzbget".to_string(),
        client_item,
        state: scryer_domain::TrackedDownloadState::FailedPending,
        status: scryer_domain::TrackedDownloadStatus::Error,
        status_messages: Vec::new(),
        title_id: Some(title.id.clone()),
        facet: Some("movie".to_string()),
        source_title: Some("Foreign.Failure.Safety.2024.1080p.WEB-DL".to_string()),
        indexer: None,
        added_at: None,
        notified_manual_interaction: false,
        match_type: scryer_domain::TitleMatchType::TitleParse,
        is_trackable: true,
        import_attempted: false,
        waiting_for_completed_history: false,
        path_missing_since: None,
        no_video_import_retry: None,
        foreign_import_classification: None,
        skip_reacquire_on_failure: true,
        snapshot_missing_since: None,
    };

    crate::failed_download_handler::process_failed(&app, &mut tracked_download).await;

    assert_eq!(
        tracked_download.state,
        scryer_domain::TrackedDownloadState::Downloading
    );
    assert!(!tracked_download.skip_reacquire_on_failure);
    assert_eq!(
        tracked_download.status,
        scryer_domain::TrackedDownloadStatus::Warning
    );
    assert!(
        tracked_download
            .status_messages
            .iter()
            .any(|message| message.contains("wasn't grabbed by Scryer"))
    );

    let updated = wanted_items
        .get_acquisition_scope_state_by_id(&wanted.id)
        .await
        .expect("get wanted")
        .expect("wanted exists");
    assert_eq!(updated.status, AcquisitionScopeStatus::Grabbed);
    assert_eq!(
        updated.grabbed_release.as_deref(),
        wanted.grabbed_release.as_deref()
    );
    assert_eq!(
        wanted_items.status_update_call_count_for(&wanted.id).await,
        0
    );

    let blocklist = app
        .services
        .workflow
        .blocklist_repo
        .list_for_title(&title.id, 10)
        .await
        .expect("list blocklist");
    assert!(blocklist.is_empty());
}

#[tokio::test]
async fn season_pack_failure_processed_twice_only_requeues_once_and_blocklists_once() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking(
        download_client,
        download_submissions.clone(),
        pending_releases,
        wanted_items.clone(),
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Season Pack Failure Recovery".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let season = app
        .services
        .catalog
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "7".to_string(),
            label: Some("Season 7".to_string()),
            ordered_path: None,
            narrative_order: Some("7".to_string()),
            first_episode_number: Some("23".to_string()),
            last_episode_number: Some("24".to_string()),
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create season");

    let mut expected_wanted_ids = Vec::new();
    for (episode_number, label) in [("23", "S07E23"), ("24", "S07E24")] {
        let episode = app
            .services
            .catalog
            .shows
            .create_episode(Episode {
                id: Id::new().0,
                title_id: title.id.clone(),
                collection_id: Some(season.id.clone()),
                episode_type: scryer_domain::EpisodeType::Standard,
                episode_number: Some(episode_number.to_string()),
                season_number: Some("7".to_string()),
                episode_label: Some(label.to_string()),
                title: Some(label.to_string()),
                air_date: Some("2024-01-01".to_string()),
                duration_seconds: Some(1_440),
                has_multi_audio: false,
                has_subtitle: false,
                is_filler: false,
                is_recap: false,
                absolute_number: None,
                overview: None,
                tvdb_id: None,
                image_url: None,
                monitored: true,
                created_at: Utc::now(),
            })
            .await
            .expect("create episode");

        let wanted = AcquisitionScopeState {
            id: Id::new().0,
            title_id: title.id.clone(),
            title_name: Some(title.name.clone()),
            title_slug: None,
            title_facet: None,
            library_id: None,
            library_name: None,
            library_slug: None,
            episode_id: Some(episode.id.clone()),
            collection_id: None,
            series_movie_link_id: None,
            season_number: Some("7".to_string()),
            episode_number: None,
            media_type: "episode".to_string(),
            last_search_at: Some((Utc::now() - chrono::Duration::minutes(30)).to_rfc3339()),
            status: AcquisitionScopeStatus::Wanted,
            grabbed_release: None,
            current_score: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        };
        expected_wanted_ids.push(wanted.id.clone());
        wanted_items
            .upsert_acquisition_scope_state(&wanted)
            .await
            .expect("seed episode wanted item");
    }

    download_submissions
        .record_submission(DownloadSubmission {
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "anime".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "failed-season-pack".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Season.Pack.Failure.Recovery.S07.1080p.WEB-DL".to_string()),
            request_signature: None,
            scope: SubmissionScope::Collection {
                collection_id: season.id.clone(),
            },
        })
        .await
        .expect("record failed season pack submission");

    let grabbed_wanted = wanted_items
        .get_acquisition_scope_state_by_id(
            expected_wanted_ids
                .first()
                .expect("expected wanted ids should contain seeded episodes"),
        )
        .await
        .expect("get grabbed wanted")
        .expect("grabbed wanted should exist");

    let first = crate::acquisition_workflow::process_download_failure(
        &app,
        crate::acquisition_workflow::DownloadFailureContext {
            wanted_item: Some(grabbed_wanted),
            title_id: Some(title.id.clone()),
            client_id: "primary".to_string(),
            client_type: "nzbget".to_string(),
            client_name: Some("Primary".to_string()),
            client_item_id: "failed-season-pack".to_string(),
            release_title: "Season.Pack.Failure.Recovery.S07.1080p.WEB-DL".to_string(),
            reason: "download failed".to_string(),
            remove_from_client_if_configured: false,
            skip_reacquire: false,
        },
        None,
    )
    .await;
    assert_eq!(
        first,
        crate::acquisition_workflow::FailureHandlingOutcome::RequeuedFreshSearch
    );

    let mut tracked_download = crate::tracked_downloads::TrackedDownload {
        id: "nzbget:failed-season-pack".to_string(),
        client_id: "primary".to_string(),
        client_type: "nzbget".to_string(),
        client_item: failed_history_item(
            "failed-season-pack",
            "Season.Pack.Failure.Recovery.S07.1080p.WEB-DL",
        ),
        state: scryer_domain::TrackedDownloadState::FailedPending,
        status: scryer_domain::TrackedDownloadStatus::Error,
        status_messages: Vec::new(),
        title_id: Some(title.id.clone()),
        facet: Some("anime".to_string()),
        source_title: Some("Season.Pack.Failure.Recovery.S07.1080p.WEB-DL".to_string()),
        indexer: None,
        added_at: None,
        notified_manual_interaction: false,
        match_type: scryer_domain::TitleMatchType::Submission,
        is_trackable: true,
        import_attempted: false,
        waiting_for_completed_history: false,
        path_missing_since: None,
        no_video_import_retry: None,
        foreign_import_classification: None,
        skip_reacquire_on_failure: false,
        snapshot_missing_since: None,
    };

    crate::failed_download_handler::process_failed(&app, &mut tracked_download).await;

    assert_eq!(
        tracked_download.state,
        scryer_domain::TrackedDownloadState::Failed
    );

    for wanted_id in &expected_wanted_ids {
        let wanted = wanted_items
            .get_acquisition_scope_state_by_id(wanted_id)
            .await
            .expect("get wanted item")
            .expect("wanted item exists");
        // The failed pack reopens each covered episode scope for
        // convergence (status back to `wanted`, grab cleared) instead of
        // rescheduling a cadence.
        assert_eq!(wanted.status, AcquisitionScopeStatus::Wanted);
        assert!(wanted.grabbed_release.is_none());
        assert_eq!(
            wanted_items.status_update_call_count_for(wanted_id).await,
            1,
            "duplicate season-pack failure should only requeue each episode once"
        );
    }

    let blocklist = app
        .services
        .workflow
        .blocklist_repo
        .list_for_title(&title.id, 10)
        .await
        .expect("list blocklist");
    assert_eq!(blocklist.len(), 1);
    assert_eq!(
        blocklist[0].download_id.as_deref(),
        Some("failed-season-pack")
    );

    let failed_attempts = app
        .services
        .workflow
        .release_attempts
        .list_failed_release_signatures_for_title(&title.id, 10)
        .await
        .expect("list failed release attempts");
    assert_eq!(failed_attempts.len(), 1);

    let history = app
        .list_title_history(
            &user,
            &TitleHistoryFilter {
                event_types: Some(vec![
                    TitleHistoryEventType::DownloadFailed,
                    TitleHistoryEventType::Blocklisted,
                ]),
                title_ids: Some(vec![title.id.clone()]),
                library_ids: None,
                title_search: None,
                download_id: Some("failed-season-pack".to_string()),
                episode_id: None,
                group_by_event: false,
                limit: 10,
                offset: 0,
            },
        )
        .await
        .expect("list title history");

    assert_eq!(history.total_count, 4);
    assert!(history.records.iter().any(|record| {
        record.event_type == TitleHistoryEventType::DownloadFailed
            && record.collection_id.as_deref() == Some(season.id.as_str())
            && record.download_id.as_deref() == Some("failed-season-pack")
            && record.client_id.as_deref() == Some("primary")
            && record.client_name.as_deref() == Some("Primary")
            && record
                .failure_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("re-opened season episodes"))
    }));
    assert!(history.records.iter().any(|record| {
        record.event_type == TitleHistoryEventType::Blocklisted
            && record.collection_id.as_deref() == Some(season.id.as_str())
            && record.download_id.as_deref() == Some("failed-season-pack")
            && record
                .blocklist_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("download client failure"))
    }));

    assert!(
        download_submissions
            .store
            .lock()
            .await
            .iter()
            .any(|submission| {
                submission.download_client_item_id == "failed-season-pack"
                    && submission.source_title.as_deref()
                        == Some("Season.Pack.Failure.Recovery.S07.1080p.WEB-DL")
            })
    );
    assert_eq!(
        download_submissions
            .get_tracked_state(&DownloadSourceIdentity::new(
                Some("primary"),
                "nzbget",
                "failed-season-pack",
            ))
            .await
            .expect("load tracked state")
            .as_deref(),
        Some("failed")
    );
}

#[tokio::test]
async fn acquisition_cycle_looks_up_submissions_once_per_title_for_grabbed_items() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking(
        download_client,
        download_submissions.clone(),
        pending_releases,
        wanted_items.clone(),
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Shared Grabbed Title".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    for (item_id, episode_id, release_title) in [
        ("wanted-1", "episode-1", "Shared.Release.S01E01"),
        ("wanted-2", "episode-2", "Shared.Release.S01E02"),
    ] {
        wanted_items
            .upsert_acquisition_scope_state(&AcquisitionScopeState {
                id: item_id.to_string(),
                title_id: title.id.clone(),
                title_name: Some(title.name.clone()),
                title_slug: None,
                title_facet: None,
                library_id: None,
                library_name: None,
                library_slug: None,
                episode_id: Some(episode_id.to_string()),
                collection_id: None,
                series_movie_link_id: None,
                season_number: Some("1".to_string()),
                episode_number: None,
                media_type: "episode".to_string(),
                last_search_at: Some((Utc::now() - chrono::Duration::minutes(5)).to_rfc3339()),
                status: AcquisitionScopeStatus::Grabbed,
                grabbed_release: Some(
                    serde_json::json!({
                        "title": release_title,
                        "score": 100,
                        "grabbed_at": Utc::now().to_rfc3339(),
                    })
                    .to_string(),
                ),
                current_score: None,
                latest_release_decision: None,
                mismatch_recovery_eligible: false,
                created_at: Utc::now().to_rfc3339(),
                updated_at: Utc::now().to_rfc3339(),
            })
            .await
            .expect("seed grabbed wanted item");
    }

    download_submissions
        .record_submission(DownloadSubmission {
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "anime".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "shared-job".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Shared.Release".to_string()),
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record shared submission");

    app.run_convergence_cycle_once().await;

    let calls = download_submissions
        .list_for_title_calls
        .lock()
        .await
        .clone();
    assert_eq!(calls, vec![title.id.clone()]);
}

#[tokio::test]
async fn acquisition_cycle_records_failed_collection_submission_once() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let indexer_client = Arc::new(TrackingIndexerClient::default());
    let (app, user) = bootstrap_with_acquisition_tracking_and_indexer(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
        wanted_items.clone(),
        indexer_client.clone(),
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
                name: "Shared Failed Season Pack".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let season = app
        .services
        .catalog
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: Some("1".to_string()),
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("2".to_string()),
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create season");

    let pack_title = "Shared Failed Season Pack.S01.1080p.WEB-DL";
    let grabbed_release = serde_json::json!({
        "title": pack_title,
        "score": 100,
        "grabbed_at": Utc::now().to_rfc3339(),
        "season_pack": true,
    })
    .to_string();

    let mut wanted_ids = Vec::new();
    for (episode_number, label) in [("1", "S01E01"), ("2", "S01E02")] {
        let episode = app
            .services
            .catalog
            .shows
            .create_episode(Episode {
                id: Id::new().0,
                title_id: title.id.clone(),
                collection_id: Some(season.id.clone()),
                episode_type: scryer_domain::EpisodeType::Standard,
                episode_number: Some(episode_number.to_string()),
                season_number: Some("1".to_string()),
                episode_label: Some(label.to_string()),
                title: Some(label.to_string()),
                air_date: Some("2024-01-01".to_string()),
                duration_seconds: Some(1_440),
                has_multi_audio: false,
                has_subtitle: false,
                is_filler: false,
                is_recap: false,
                absolute_number: None,
                overview: None,
                tvdb_id: None,
                image_url: None,
                monitored: true,
                created_at: Utc::now(),
            })
            .await
            .expect("create episode");

        let wanted_id = Id::new().0;
        wanted_ids.push(wanted_id.clone());
        wanted_items
            .upsert_acquisition_scope_state(&AcquisitionScopeState {
                id: wanted_id,
                title_id: title.id.clone(),
                title_name: Some(title.name.clone()),
                title_slug: None,
                title_facet: None,
                library_id: None,
                library_name: None,
                library_slug: None,
                episode_id: Some(episode.id.clone()),
                collection_id: Some(season.id.clone()),
                series_movie_link_id: None,
                season_number: Some("1".to_string()),
                episode_number: Some(episode_number.to_string()),
                media_type: "episode".to_string(),
                last_search_at: Some((Utc::now() - chrono::Duration::minutes(5)).to_rfc3339()),
                status: AcquisitionScopeStatus::Grabbed,
                grabbed_release: Some(grabbed_release.clone()),
                current_score: None,
                latest_release_decision: None,
                mismatch_recovery_eligible: false,
                created_at: Utc::now().to_rfc3339(),
                updated_at: Utc::now().to_rfc3339(),
            })
            .await
            .expect("seed grabbed wanted item");
    }

    download_submissions
        .record_submission(DownloadSubmission {
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "anime".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "shared-failed-season-pack".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some(pack_title.to_string()),
            request_signature: None,
            scope: SubmissionScope::Collection {
                collection_id: season.id.clone(),
            },
        })
        .await
        .expect("record failed collection submission");

    *download_client.history_items.lock().await = vec![DownloadQueueItem {
        title_id: Some(title.id.clone()),
        facet: Some("anime".to_string()),
        ..failed_history_item("shared-failed-season-pack", pack_title)
    }];

    app.run_convergence_cycle_once().await;

    let searches = indexer_client.searches.lock().await.clone();
    assert!(!searches.is_empty());
    assert!(
        searches
            .iter()
            .all(|search| search.season == Some(1) && search.episode.is_some())
    );

    let wanted_store = wanted_items.store.lock().await.clone();
    for wanted_id in wanted_ids {
        let wanted = wanted_store
            .iter()
            .find(|wanted| wanted.id == wanted_id)
            .expect("wanted item exists");
        assert_eq!(wanted.status, AcquisitionScopeStatus::Grabbed);
        assert!(wanted.grabbed_release.is_some());
    }

    let blocklist = app
        .list_title_release_blocklist(&user, &title.id, 10)
        .await
        .expect("list title release blocklist");
    assert_eq!(blocklist.len(), 1);
    assert!(
        blocklist[0]
            .source_title
            .as_deref()
            .is_some_and(|title| title.eq_ignore_ascii_case(pack_title))
    );
    assert!(
        blocklist[0]
            .error_message
            .as_deref()
            .is_some_and(|message| !message.trim().is_empty())
    );

    assert!(
        download_submissions
            .store
            .lock()
            .await
            .iter()
            .any(|submission| {
                submission.download_client_item_id == "shared-failed-season-pack"
                    && submission.source_title.as_deref() == Some(pack_title)
            })
    );
    assert_eq!(
        download_submissions
            .get_tracked_state(&DownloadSourceIdentity::new(
                Some("primary"),
                "nzbget",
                "shared-failed-season-pack",
            ))
            .await
            .expect("load tracked state")
            .as_deref(),
        Some("failed")
    );
}

#[tokio::test]
async fn acquisition_cycle_episode_submission_blocks_only_matching_episode() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let indexer_client = Arc::new(TrackingIndexerClient::default());
    let (app, user) = bootstrap_with_acquisition_tracking_and_indexer(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
        wanted_items.clone(),
        indexer_client.clone(),
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
                name: "Episode Blocking Scope".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let season_one = app
        .services
        .catalog
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: Some("1".to_string()),
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("1".to_string()),
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create season one");

    let season_two = app
        .services
        .catalog
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "2".to_string(),
            label: Some("Season 2".to_string()),
            ordered_path: None,
            narrative_order: Some("2".to_string()),
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("1".to_string()),
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create season two");

    let episode_one = app
        .services
        .catalog
        .shows
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(season_one.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E01".to_string()),
            title: Some("Season 1 Premiere".to_string()),
            air_date: Some("2024-01-01".to_string()),
            duration_seconds: Some(1_440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create episode one");

    let episode_two = app
        .services
        .catalog
        .shows
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(season_two.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("2".to_string()),
            episode_label: Some("S02E01".to_string()),
            title: Some("Season 2 Premiere".to_string()),
            air_date: Some("2025-01-01".to_string()),
            duration_seconds: Some(1_440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create episode two");

    for episode in [&episode_one, &episode_two] {
        wanted_items
            .upsert_acquisition_scope_state(&AcquisitionScopeState {
                id: Id::new().0,
                title_id: title.id.clone(),
                title_name: Some(title.name.clone()),
                title_slug: None,
                title_facet: None,
                library_id: None,
                library_name: None,
                library_slug: None,
                episode_id: Some(episode.id.clone()),
                collection_id: None,
                series_movie_link_id: None,
                season_number: episode.season_number.clone(),
                episode_number: None,
                media_type: "episode".to_string(),
                last_search_at: None,
                status: AcquisitionScopeStatus::Wanted,
                grabbed_release: None,
                current_score: None,
                latest_release_decision: None,
                mismatch_recovery_eligible: false,
                created_at: Utc::now().to_rfc3339(),
                updated_at: Utc::now().to_rfc3339(),
            })
            .await
            .expect("seed due episode wanted item");
    }

    download_submissions
        .record_submission(DownloadSubmission {
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "anime".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "episode-one-active".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Episode.Blocking.Scope.S01E01".to_string()),
            request_signature: None,
            scope: SubmissionScope::Episode {
                episode_id: episode_one.id.clone(),
            },
        })
        .await
        .expect("record active episode submission");

    *download_client.queue_items.lock().await = vec![DownloadQueueItem {
        id: "episode-one-active".to_string(),
        title_id: Some(title.id.clone()),
        episode_id: Some(episode_one.id.clone()),
        title_name: title.name.clone(),
        facet: Some("anime".to_string()),
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
        download_client_item_id: "episode-one-active".to_string(),
        download_id: None,
        import_status: None,
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
    }];

    app.run_convergence_cycle_once().await;

    let searches = indexer_client.searches.lock().await.clone();
    assert!(!searches.is_empty());
    assert!(
        searches
            .iter()
            .all(|search| search.season == Some(2) && search.episode == Some(1))
    );
}

#[tokio::test]
async fn acquisition_cycle_collection_submission_blocks_same_season_only() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let indexer_client = Arc::new(TrackingIndexerClient::default());
    let (app, user) = bootstrap_with_acquisition_tracking_and_indexer(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
        wanted_items.clone(),
        indexer_client.clone(),
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Season Pack Blocking Scope".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let season_one = app
        .services
        .catalog
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: Some("1".to_string()),
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("2".to_string()),
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create season one");

    let season_two = app
        .services
        .catalog
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "2".to_string(),
            label: Some("Season 2".to_string()),
            ordered_path: None,
            narrative_order: Some("2".to_string()),
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("1".to_string()),
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create season two");

    for (collection, season_number, episode_number, label) in [
        (&season_one, "1", "1", "S01E01"),
        (&season_one, "1", "2", "S01E02"),
        (&season_two, "2", "1", "S02E01"),
    ] {
        let episode = app
            .services
            .catalog
            .shows
            .create_episode(Episode {
                id: Id::new().0,
                title_id: title.id.clone(),
                collection_id: Some(collection.id.clone()),
                episode_type: scryer_domain::EpisodeType::Standard,
                episode_number: Some(episode_number.to_string()),
                season_number: Some(season_number.to_string()),
                episode_label: Some(label.to_string()),
                title: Some(label.to_string()),
                air_date: Some("2024-01-01".to_string()),
                duration_seconds: Some(1_440),
                has_multi_audio: false,
                has_subtitle: false,
                is_filler: false,
                is_recap: false,
                absolute_number: None,
                overview: None,
                tvdb_id: None,
                image_url: None,
                monitored: true,
                created_at: Utc::now(),
            })
            .await
            .expect("create episode");

        wanted_items
            .upsert_acquisition_scope_state(&AcquisitionScopeState {
                id: Id::new().0,
                title_id: title.id.clone(),
                title_name: Some(title.name.clone()),
                title_slug: None,
                title_facet: None,
                library_id: None,
                library_name: None,
                library_slug: None,
                episode_id: Some(episode.id.clone()),
                collection_id: None,
                series_movie_link_id: None,
                season_number: Some(season_number.to_string()),
                episode_number: None,
                media_type: "episode".to_string(),
                last_search_at: None,
                status: AcquisitionScopeStatus::Wanted,
                grabbed_release: None,
                current_score: None,
                latest_release_decision: None,
                mismatch_recovery_eligible: false,
                created_at: Utc::now().to_rfc3339(),
                updated_at: Utc::now().to_rfc3339(),
            })
            .await
            .expect("seed due episode wanted item");
    }

    download_submissions
        .record_submission(DownloadSubmission {
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "anime".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "season-one-pack".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Season.Pack.Blocking.Scope.S01".to_string()),
            request_signature: None,
            scope: SubmissionScope::Collection {
                collection_id: season_one.id.clone(),
            },
        })
        .await
        .expect("record active season pack submission");

    *download_client.queue_items.lock().await = vec![DownloadQueueItem {
        id: "season-one-pack".to_string(),
        title_id: Some(title.id.clone()),
        episode_id: None,
        title_name: title.name.clone(),
        facet: Some("anime".to_string()),
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
        download_client_item_id: "season-one-pack".to_string(),
        download_id: None,
        import_status: None,
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
    }];

    app.run_convergence_cycle_once().await;

    let searches = indexer_client.searches.lock().await.clone();
    assert!(!searches.is_empty());
    assert!(
        searches
            .iter()
            .all(|search| search.season == Some(2) && search.episode == Some(1))
    );
}

#[tokio::test]
async fn acquisition_cycle_falls_back_to_episode_grabs_when_season_pack_is_not_selected() {
    struct AutoGrabSeasonPackIndexerClient {
        searches: Arc<Mutex<Vec<RecordedIndexerSearch>>>,
    }

    #[async_trait]
    impl IndexerClient for AutoGrabSeasonPackIndexerClient {
        async fn search(
            &self,
            query: String,
            _ids: std::collections::HashMap<String, String>,
            _category: Option<String>,
            _facet: Option<String>,
            _id_search_facet: Option<String>,
            _newznab_categories: Option<Vec<String>>,
            _indexer_routing: Option<IndexerRoutingPlan>,
            _mode: SearchMode,
            season: Option<u32>,
            episode: Option<u32>,
            _absolute_episode: Option<u32>,
            _tagged_aliases: Vec<TaggedAlias>,
            _learning_context: Option<crate::IndexerSearchLearningContext>,
            _cancel_token: tokio_util::sync::CancellationToken,
        ) -> AppResult<IndexerSearchResponse> {
            self.searches.lock().await.push(RecordedIndexerSearch {
                query: query.clone(),
                season,
                episode,
            });

            let release_title = match (season, episode) {
                (Some(_season), Some(_episode)) => format!("{query}.1080p.WEB-DL"),
                (Some(season), None) => {
                    let season_token = format!(" S{season:02}");
                    let base_query = query.strip_suffix(&season_token).unwrap_or(query.as_str());
                    format!("{base_query} Season {season} - (1 - 2) [Typis]")
                }
                (None, _) => format!("{query}.2024.1080p.WEB-DL"),
            };
            let parsed_release_metadata = match (season, episode) {
                (Some(season), None) => {
                    let mut parsed = crate::parse_release_metadata(&release_title);
                    let mut episode_metadata = parsed.episode.unwrap_or_default();
                    episode_metadata.season = Some(season);
                    episode_metadata.full_season = true;
                    episode_metadata.release_type = crate::ParsedEpisodeReleaseType::SeasonPack;
                    parsed.episode = Some(crate::ParsedEpisodeMetadata { ..episode_metadata });
                    parsed
                }
                _ => crate::parse_release_metadata(&release_title),
            };
            let release_slug = release_title.replace([' ', '/'], ".");

            Ok(IndexerSearchResponse {
                indexer_outcomes: Vec::new(),
                results: vec![IndexerSearchResult {
                    indexer_id: None,
                    source: "nzbgeek".into(),
                    title: release_title.clone(),
                    link: Some(format!("https://example.invalid/info/{release_slug}")),
                    download_url: Some(format!(
                        "https://example.invalid/download/{release_slug}.nzb"
                    )),
                    source_kind: Some(DownloadSourceKind::NzbFile),
                    size_bytes: None,
                    published_at: Some("1970-01-01T00:00:00Z".into()),
                    thumbs_up: None,
                    thumbs_down: None,
                    indexer_languages: None,
                    indexer_subtitles: None,
                    indexer_grabs: None,
                    password_hint: None,
                    parsed_release_metadata: Some(parsed_release_metadata),
                    quality_profile_decision: Some(
                        crate::quality::profile::QualityProfileDecision {
                            release_score: 100,
                            scoring_log: Vec::new(),
                            allowed: true,
                            block_codes: Vec::new(),
                            preference_score: 100,
                        },
                    ),
                    extra: Default::default(),
                    response_attributes: Default::default(),
                    guid: Some(format!("guid-{release_slug}")),
                    info_url: Some(format!("https://example.invalid/info/{release_slug}")),
                    provenance: None,
                    auto_eligible: Some(true),
                    auto_decision_code: None,
                    auto_decision_summary: None,
                    candidate_token: None,
                    queue_scope: None,
                }],
                api_current: None,
                api_max: None,
                grab_current: None,
                grab_max: None,
            })
        }
    }

    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let recorded_searches = Arc::new(Mutex::new(Vec::new()));
    let indexer_client: Arc<dyn IndexerClient> = Arc::new(AutoGrabSeasonPackIndexerClient {
        searches: recorded_searches.clone(),
    });
    let (app, user) = bootstrap_with_acquisition_tracking_and_indexer(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
        wanted_items.clone(),
        indexer_client.clone(),
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
                name: "Emberfall".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let season = app
        .services
        .catalog
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: Some("1".to_string()),
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("2".to_string()),
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create season");

    let mut wanted_ids = Vec::new();
    for (episode_number, label) in [("1", "S01E01"), ("2", "S01E02")] {
        let episode = app
            .services
            .catalog
            .shows
            .create_episode(Episode {
                id: Id::new().0,
                title_id: title.id.clone(),
                collection_id: Some(season.id.clone()),
                episode_type: scryer_domain::EpisodeType::Standard,
                episode_number: Some(episode_number.to_string()),
                season_number: Some("1".to_string()),
                episode_label: Some(label.to_string()),
                title: Some(label.to_string()),
                air_date: Some("2024-01-01".to_string()),
                duration_seconds: Some(1_440),
                has_multi_audio: false,
                has_subtitle: false,
                is_filler: false,
                is_recap: false,
                absolute_number: None,
                overview: None,
                tvdb_id: None,
                image_url: None,
                monitored: true,
                created_at: Utc::now(),
            })
            .await
            .expect("create episode");

        let wanted_id = Id::new().0;
        wanted_ids.push(wanted_id.clone());
        wanted_items
            .upsert_acquisition_scope_state(&AcquisitionScopeState {
                id: wanted_id,
                title_id: title.id.clone(),
                title_name: Some(title.name.clone()),
                title_slug: None,
                title_facet: None,
                library_id: None,
                library_name: None,
                library_slug: None,
                episode_id: Some(episode.id.clone()),
                collection_id: Some(season.id.clone()),
                series_movie_link_id: None,
                season_number: Some("1".to_string()),
                episode_number: Some(episode_number.to_string()),
                media_type: "episode".to_string(),
                last_search_at: None,
                status: AcquisitionScopeStatus::Wanted,
                grabbed_release: None,
                current_score: None,
                latest_release_decision: None,
                mismatch_recovery_eligible: false,
                created_at: Utc::now().to_rfc3339(),
                updated_at: Utc::now().to_rfc3339(),
            })
            .await
            .expect("seed due wanted item");
    }

    app.run_convergence_cycle_once().await;

    let searches = recorded_searches.lock().await.clone();
    assert!(
        searches
            .iter()
            .any(|search| search.season == Some(1) && search.episode.is_none())
    );
    assert_eq!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .clone(),
        vec![
            "Emberfall S01E01.1080p.WEB-DL".to_string(),
            "Emberfall S01E02.1080p.WEB-DL".to_string(),
        ]
    );

    let submissions = download_submissions.store.lock().await.clone();
    assert!(!submissions.is_empty());

    let wanted_store = wanted_items.store.lock().await.clone();
    for wanted_id in wanted_ids {
        let wanted = wanted_store
            .iter()
            .find(|wanted| wanted.id == wanted_id)
            .expect("wanted item exists");
        assert_eq!(wanted.status, AcquisitionScopeStatus::Grabbed);
        let grabbed_release: serde_json::Value = serde_json::from_str(
            wanted
                .grabbed_release
                .as_deref()
                .expect("grabbed release recorded"),
        )
        .expect("grabbed release should parse");
        let expected_title = match wanted.episode_number.as_deref() {
            Some("1") => "Emberfall S01E01.1080p.WEB-DL",
            Some("2") => "Emberfall S01E02.1080p.WEB-DL",
            other => panic!("unexpected episode number: {other:?}"),
        };
        assert_eq!(grabbed_release["title"].as_str(), Some(expected_title));
        assert_ne!(grabbed_release["season_pack"].as_bool(), Some(true));
    }
}

#[tokio::test]
async fn acquisition_cycle_skips_recently_failed_season_pack_and_searches_episodes() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let indexer_client = Arc::new(TrackingIndexerClient::default());
    let (app, user) = bootstrap_with_acquisition_tracking_and_indexer(
        download_client,
        download_submissions,
        pending_releases,
        wanted_items.clone(),
        indexer_client.clone(),
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Recent Failed Season Pack".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let season = app
        .services
        .catalog
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "7".to_string(),
            label: Some("Season 7".to_string()),
            ordered_path: None,
            narrative_order: Some("7".to_string()),
            first_episode_number: Some("23".to_string()),
            last_episode_number: Some("24".to_string()),
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create season");

    for (episode_number, label) in [("23", "S07E23"), ("24", "S07E24")] {
        let episode = app
            .services
            .catalog
            .shows
            .create_episode(Episode {
                id: Id::new().0,
                title_id: title.id.clone(),
                collection_id: Some(season.id.clone()),
                episode_type: scryer_domain::EpisodeType::Standard,
                episode_number: Some(episode_number.to_string()),
                season_number: Some("7".to_string()),
                episode_label: Some(label.to_string()),
                title: Some(label.to_string()),
                air_date: Some("2024-01-01".to_string()),
                duration_seconds: Some(1_440),
                has_multi_audio: false,
                has_subtitle: false,
                is_filler: false,
                is_recap: false,
                absolute_number: None,
                overview: None,
                tvdb_id: None,
                image_url: None,
                monitored: true,
                created_at: Utc::now(),
            })
            .await
            .expect("create episode");

        wanted_items
            .upsert_acquisition_scope_state(&AcquisitionScopeState {
                id: Id::new().0,
                title_id: title.id.clone(),
                title_name: Some(title.name.clone()),
                title_slug: None,
                title_facet: None,
                library_id: None,
                library_name: None,
                library_slug: None,
                episode_id: Some(episode.id.clone()),
                collection_id: None,
                series_movie_link_id: None,
                season_number: Some("7".to_string()),
                episode_number: None,
                media_type: "episode".to_string(),
                last_search_at: None,
                status: AcquisitionScopeStatus::Wanted,
                grabbed_release: None,
                current_score: None,
                latest_release_decision: None,
                mismatch_recovery_eligible: false,
                created_at: Utc::now().to_rfc3339(),
                updated_at: Utc::now().to_rfc3339(),
            })
            .await
            .expect("seed due episode wanted item");
    }

    app.services
        .workflow
        .release_attempts
        .record_release_attempt(
            Some(title.id.clone()),
            None,
            Some("Recent.Failed.Season.Pack.S07.1080p.WEB-DL".to_string()),
            ReleaseDownloadAttemptOutcome::Failed,
            Some("download client failure: corrupt archive".to_string()),
            None,
        )
        .await
        .expect("record failed season pack attempt");

    app.run_convergence_cycle_once().await;

    let searches = indexer_client.searches.lock().await.clone();
    assert!(!searches.is_empty());
    assert!(searches.iter().all(|search| search.season == Some(7)));
    assert!(searches.iter().all(|search| search.episode.is_some()));
    assert!(
        !searches
            .iter()
            .any(|search| search.season == Some(7) && search.episode.is_none())
    );
}

#[tokio::test]
async fn acquisition_cycle_skips_recently_failed_season_pack_from_submission_release_title() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let indexer_client = Arc::new(TrackingIndexerClient::default());
    let (app, user) = bootstrap_with_acquisition_tracking_and_indexer(
        download_client,
        download_submissions.clone(),
        pending_releases,
        wanted_items.clone(),
        indexer_client.clone(),
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Friends".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let season = app
        .services
        .catalog
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "5".to_string(),
            label: Some("Season 5".to_string()),
            ordered_path: None,
            narrative_order: Some("5".to_string()),
            first_episode_number: Some("01".to_string()),
            last_episode_number: Some("02".to_string()),
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create season");

    let mut expected_wanted_ids = Vec::new();
    for (episode_number, label) in [("01", "S05E01"), ("02", "S05E02")] {
        let episode = app
            .services
            .catalog
            .shows
            .create_episode(Episode {
                id: Id::new().0,
                title_id: title.id.clone(),
                collection_id: Some(season.id.clone()),
                episode_type: scryer_domain::EpisodeType::Standard,
                episode_number: Some(episode_number.to_string()),
                season_number: Some("5".to_string()),
                episode_label: Some(label.to_string()),
                title: Some(label.to_string()),
                air_date: Some("1998-01-01".to_string()),
                duration_seconds: Some(1_440),
                has_multi_audio: false,
                has_subtitle: false,
                is_filler: false,
                is_recap: false,
                absolute_number: None,
                overview: None,
                tvdb_id: None,
                image_url: None,
                monitored: true,
                created_at: Utc::now(),
            })
            .await
            .expect("create episode");

        let wanted = AcquisitionScopeState {
            id: Id::new().0,
            title_id: title.id.clone(),
            title_name: Some(title.name.clone()),
            title_slug: None,
            title_facet: None,
            library_id: None,
            library_name: None,
            library_slug: None,
            episode_id: Some(episode.id.clone()),
            collection_id: None,
            series_movie_link_id: None,
            season_number: Some("5".to_string()),
            episode_number: None,
            media_type: "episode".to_string(),
            last_search_at: Some((Utc::now() - chrono::Duration::minutes(10)).to_rfc3339()),
            status: AcquisitionScopeStatus::Grabbed,
            grabbed_release: Some(
                serde_json::json!({
                    "title": "Friends.S05.720p.BluRay.DD5.1.x264-NTb",
                    "score": 100,
                    "grabbed_at": Utc::now().to_rfc3339(),
                    "season_pack": true,
                })
                .to_string(),
            ),
            current_score: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        };
        expected_wanted_ids.push(wanted.id.clone());
        wanted_items
            .upsert_acquisition_scope_state(&wanted)
            .await
            .expect("seed episode wanted item");
    }

    download_submissions
        .record_submission(DownloadSubmission {
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "series".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "weaver".to_string(),
            download_client_item_id: "weaver-season-pack-1".to_string(),
            source_hint: Some("weaver://job/weaver-season-pack-1".to_string()),
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Friends.S05.720p.BluRay.DD5.1.x264-NTb".to_string()),
            request_signature: Some(
                "nzb_url|https://example.com/friends-s05.nzb|Friends.S05.720p.BluRay.DD5.1.x264-NTb"
                    .to_string(),
            ),
            scope: SubmissionScope::Collection {
                collection_id: season.id.clone(),
            },
        })
        .await
        .expect("record failed season pack submission");

    let grabbed_wanted = wanted_items
        .get_acquisition_scope_state_by_id(
            expected_wanted_ids
                .first()
                .expect("expected wanted ids should contain seeded episodes"),
        )
        .await
        .expect("get grabbed wanted")
        .expect("grabbed wanted should exist");

    let outcome = crate::acquisition_workflow::process_download_failure(
        &app,
        crate::acquisition_workflow::DownloadFailureContext {
            wanted_item: Some(grabbed_wanted),
            title_id: Some(title.id.clone()),
            client_id: "primary".to_string(),
            client_type: "weaver".to_string(),
            client_name: Some("Primary".to_string()),
            client_item_id: "weaver-season-pack-1".to_string(),
            release_title: "Friends".to_string(),
            reason: "download failed".to_string(),
            remove_from_client_if_configured: false,
            skip_reacquire: false,
        },
        None,
    )
    .await;

    assert_eq!(
        outcome,
        crate::acquisition_workflow::FailureHandlingOutcome::RequeuedFreshSearch
    );

    let blocklist = app
        .services
        .workflow
        .blocklist_repo
        .list_for_title(&title.id, 10)
        .await
        .expect("list blocklist");
    assert_eq!(blocklist.len(), 1);
    assert_eq!(
        blocklist[0].source_title.as_deref(),
        Some("friends.s05.720p.bluray.dd5.1.x264-ntb")
    );

    app.run_convergence_cycle_once().await;

    let searches = indexer_client.searches.lock().await.clone();
    assert!(!searches.is_empty());
    assert!(searches.iter().all(|search| search.season == Some(5)));
    assert!(searches.iter().all(|search| search.episode.is_some()));
    assert!(
        !searches
            .iter()
            .any(|search| search.season == Some(5) && search.episode.is_none())
    );
}

#[tokio::test]
async fn acquisition_cycle_submit_unavailable_records_pending_without_failed_signature() {
    let release_title = "Deferred.Movie.2024.1080p.WEB-DL-GRP";
    let download_client = Arc::new(StubDownloadClient::default());
    download_client
        .set_submit_error(Some(StubSubmitError::SubmitUnavailable(
            "download client auth failed".to_string(),
        )))
        .await;
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let indexer_client = Arc::new(FixedReleaseIndexerClient::new(release_title));
    let (app, user, release_attempts) =
        bootstrap_with_acquisition_tracking_and_indexer_and_release_attempts(
            download_client.clone(),
            download_submissions.clone(),
            pending_releases,
            wanted_items.clone(),
            indexer_client,
        );

    let (title, _) =
        seed_movie_wanted_for_acquisition(&app, &user, &wanted_items, "Deferred Movie", 2024).await;

    app.run_convergence_cycle_once().await;

    assert_eq!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .as_slice(),
        &[release_title.to_string()]
    );
    assert!(download_submissions.store.lock().await.is_empty());

    let attempts = release_attempts.attempts.lock().await.clone();
    assert!(
        attempts
            .iter()
            .all(|attempt| attempt.outcome != ReleaseDownloadAttemptOutcome::Failed),
        "submit-unavailable attempts must not be recorded as failed: {:?}",
        attempts
            .iter()
            .map(|attempt| (&attempt.source_title, &attempt.outcome))
            .collect::<Vec<_>>()
    );
    assert!(attempts.iter().any(|attempt| {
        attempt.outcome == ReleaseDownloadAttemptOutcome::Pending
            && attempt
                .error_message
                .as_deref()
                .is_some_and(|message| message.contains("download client auth failed"))
    }));
    let failed = release_attempts
        .list_failed_release_signatures_for_title(&title.id, 10)
        .await
        .expect("list failed signatures");
    assert!(failed.is_empty());
}

#[tokio::test]
async fn season_pack_submit_unavailable_records_pending_without_failed_signature() {
    let release_title = "Deferred.Season.Pack.S01.1080p.WEB-DL-GRP";
    let download_client = Arc::new(StubDownloadClient::default());
    download_client
        .set_submit_error(Some(StubSubmitError::SubmitUnavailable(
            "download client timeout".to_string(),
        )))
        .await;
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let indexer_client = Arc::new(FixedReleaseIndexerClient::new(release_title));
    let (app, user, release_attempts) =
        bootstrap_with_acquisition_tracking_and_indexer_and_release_attempts(
            download_client.clone(),
            download_submissions.clone(),
            pending_releases,
            wanted_items.clone(),
            indexer_client,
        );

    let (title, _) = seed_anime_season_wanted_for_acquisition(
        &app,
        &user,
        &wanted_items,
        "Deferred Season Pack",
        1,
    )
    .await;

    app.run_convergence_cycle_once().await;

    assert!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .iter()
            .any(|title| title == release_title),
        "season-pack branch should submit the pack title"
    );
    assert!(download_submissions.store.lock().await.is_empty());

    let attempts = release_attempts.attempts.lock().await.clone();
    assert!(
        attempts
            .iter()
            .all(|attempt| attempt.outcome != ReleaseDownloadAttemptOutcome::Failed),
        "season-pack submit-unavailable attempts must not be recorded as failed: {:?}",
        attempts
            .iter()
            .map(|attempt| (&attempt.source_title, &attempt.outcome))
            .collect::<Vec<_>>()
    );
    let normalized_release_title = crate::normalize_release_attempt_title(Some(release_title));
    assert!(attempts.iter().any(|attempt| {
        attempt.source_title.as_deref() == normalized_release_title.as_deref()
            && attempt.outcome == ReleaseDownloadAttemptOutcome::Pending
            && attempt
                .error_message
                .as_deref()
                .is_some_and(|message| message.contains("download client timeout"))
    }));
    let failed = release_attempts
        .list_failed_release_signatures_for_title(&title.id, 10)
        .await
        .expect("list failed signatures");
    assert!(failed.is_empty());
}

#[tokio::test]
async fn acquisition_cycle_non_unavailable_submit_error_still_records_failed_signature() {
    let release_title = "Rejected.Movie.2024.1080p.WEB-DL-GRP";
    let download_client = Arc::new(StubDownloadClient::default());
    download_client
        .set_submit_error(Some(StubSubmitError::Validation(
            "release rejected by client routing".to_string(),
        )))
        .await;
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let indexer_client = Arc::new(FixedReleaseIndexerClient::new(release_title));
    let (app, user, release_attempts) =
        bootstrap_with_acquisition_tracking_and_indexer_and_release_attempts(
            download_client,
            download_submissions,
            pending_releases,
            wanted_items.clone(),
            indexer_client,
        );

    let (title, _) =
        seed_movie_wanted_for_acquisition(&app, &user, &wanted_items, "Rejected Movie", 2024).await;

    app.run_convergence_cycle_once().await;

    let failed = release_attempts
        .list_failed_release_signatures_for_title(&title.id, 10)
        .await
        .expect("list failed signatures");
    assert_eq!(failed.len(), 1);
    assert_eq!(
        failed[0].source_title.as_deref(),
        Some("rejected.movie.2024.1080p.web-dl-grp")
    );
}

#[tokio::test]
async fn acquisition_cycle_category_mismatch_veto_burns_the_release_without_submitting() {
    // Plan 136 §6 (D1): an NZB whose indexer-asserted category contradicts the
    // subject is never handed to the download client, and the veto must reach
    // the grab caller so the release is recorded Failed — that Failed attempt
    // is what feeds the blocklist and burns the release. TV-vs-movie is a true
    // contradiction; anime-vs-movie deliberately is NOT (anime films are
    // legitimately filed under `TV > Anime` — see the companion allow test).
    let release_title = "Counterfeit.Feature.2024.1080p.WEB-DL-GRP";
    let tv_categorized_nzb = br#"<?xml version="1.0" encoding="iso-8859-1" ?>
<nzb xmlns="http://www.newzbin.com/DTD/2003/nzb">
<head>
 <meta type="name">Counterfeit.Feature.2024.1080p.WEB-DL-GRP</meta>
 <meta type="category">TV &gt; HD</meta>
</head>
<file poster="poster@example.invalid" date="1700000000" subject="[1/1]"></file>
</nzb>"#;
    let download_client = Arc::new(StubDownloadClient::default());
    download_client
        .set_category_gate_nzb(Some(tv_categorized_nzb))
        .await;
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let indexer_client = Arc::new(FixedReleaseIndexerClient::new(release_title));
    let (app, user, release_attempts) =
        bootstrap_with_acquisition_tracking_and_indexer_and_release_attempts(
            download_client.clone(),
            download_submissions.clone(),
            pending_releases,
            wanted_items.clone(),
            indexer_client,
        );

    let (title, _) =
        seed_movie_wanted_for_acquisition(&app, &user, &wanted_items, "Counterfeit Feature", 2024)
            .await;

    app.run_convergence_cycle_once().await;

    let attempts = release_attempts.attempts.lock().await.clone();
    let vetoed = attempts
        .iter()
        .find(|attempt| attempt.outcome == ReleaseDownloadAttemptOutcome::Failed)
        .expect("a category veto must be recorded as a failed release attempt");
    assert!(
        vetoed
            .error_message
            .as_deref()
            .is_some_and(|message| message.contains("category_mismatch")),
        "the recorded failure must tell operators why: {:?}",
        vetoed.error_message
    );

    let failed = release_attempts
        .list_failed_release_signatures_for_title(&title.id, 10)
        .await
        .expect("list failed signatures");
    assert_eq!(
        failed.len(),
        1,
        "the vetoed release signature must be burned so it is never re-grabbed"
    );

    assert!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .is_empty(),
        "the download client must never receive a vetoed release"
    );
    assert!(
        download_submissions.store.lock().await.is_empty(),
        "a vetoed release must never be recorded as a download submission"
    );
}

#[tokio::test]
async fn acquisition_cycle_allows_anime_categorized_nzb_for_a_movie_subject() {
    // Post-review fix: `TV > Anime` is how indexers legitimately file anime
    // FILMS — the gate must not burn One Piece Film Red for being anime.
    let release_title = "One.Piece.Film.Red.2024.1080p.WEB-DL-GRP";
    let anime_categorized_nzb = br#"<?xml version="1.0" encoding="iso-8859-1" ?>
<nzb xmlns="http://www.newzbin.com/DTD/2003/nzb">
<head>
 <meta type="name">One.Piece.Film.Red.2024.1080p.WEB-DL-GRP</meta>
 <meta type="category">TV &gt; Anime</meta>
</head>
<file poster="poster@example.invalid" date="1700000000" subject="[1/1]"></file>
</nzb>"#;
    let download_client = Arc::new(StubDownloadClient::default());
    download_client
        .set_category_gate_nzb(Some(anime_categorized_nzb))
        .await;
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let indexer_client = Arc::new(FixedReleaseIndexerClient::new(release_title));
    let (app, user, release_attempts) =
        bootstrap_with_acquisition_tracking_and_indexer_and_release_attempts(
            download_client.clone(),
            download_submissions.clone(),
            pending_releases,
            wanted_items.clone(),
            indexer_client,
        );

    let (title, _) =
        seed_movie_wanted_for_acquisition(&app, &user, &wanted_items, "One Piece Film Red", 2024)
            .await;

    app.run_convergence_cycle_once().await;

    assert_eq!(
        download_client.submitted_release_titles.lock().await.len(),
        1,
        "an anime-categorized film must reach the download client"
    );
    let failed = release_attempts
        .list_failed_release_signatures_for_title(&title.id, 10)
        .await
        .expect("list failed signatures");
    assert!(
        failed.is_empty(),
        "nothing about this grab may be burned: {failed:?}"
    );
}

#[tokio::test]
async fn acquisition_cycle_rejected_submit_error_records_failed_signature_not_deferred() {
    // A DownloadSubmitRejected error (definitive SAB rejection) must flow into
    // the hard-failure path — recorded Failed and blocklist-worthy — never the
    // deferred/pending path reserved for unavailable/ambiguous submits.
    let release_title = "Rejected.Movie.2024.1080p.WEB-DL-GRP";
    let download_client = Arc::new(StubDownloadClient::default());
    download_client
        .set_submit_error(Some(StubSubmitError::Rejected(
            "sabnzbd rejected the nzb: Duplicate NZB".to_string(),
        )))
        .await;
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let indexer_client = Arc::new(FixedReleaseIndexerClient::new(release_title));
    let (app, user, release_attempts) =
        bootstrap_with_acquisition_tracking_and_indexer_and_release_attempts(
            download_client,
            download_submissions.clone(),
            pending_releases,
            wanted_items.clone(),
            indexer_client,
        );

    let (title, _) =
        seed_movie_wanted_for_acquisition(&app, &user, &wanted_items, "Rejected Movie", 2024).await;

    app.run_convergence_cycle_once().await;

    let failed = release_attempts
        .list_failed_release_signatures_for_title(&title.id, 10)
        .await
        .expect("list failed signatures");
    assert_eq!(
        failed.len(),
        1,
        "a rejected submit must record a failed (blocklist-worthy) signature"
    );
    assert_eq!(
        failed[0].source_title.as_deref(),
        Some("rejected.movie.2024.1080p.web-dl-grp")
    );
    // A definitive rejection records no download submission.
    assert!(
        download_submissions.store.lock().await.is_empty(),
        "a rejected submit must not record a download submission"
    );
}

#[tokio::test]
async fn acquisition_cycle_duplicate_url_does_not_mark_second_wanted_grabbed_without_submission() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let shared_url = "https://example.invalid/shared-duplicate.nzb";
    let indexer_client = Arc::new(SharedUrlMovieIndexerClient::new(shared_url));
    let (app, user, _) = bootstrap_with_acquisition_tracking_and_indexer_and_release_attempts(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
        wanted_items.clone(),
        indexer_client,
    );

    let (_, first_wanted_id) =
        seed_movie_wanted_for_acquisition(&app, &user, &wanted_items, "Deferred Movie", 2024).await;
    let (_, second_wanted_id) =
        seed_movie_wanted_for_acquisition(&app, &user, &wanted_items, "Rejected Movie", 2024).await;

    app.run_convergence_cycle_once().await;

    assert_eq!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .as_slice(),
        &["Deferred.Movie.2024.1080p.WEB-DL-GRP".to_string()]
    );

    let submissions = download_submissions.store.lock().await.clone();
    assert_eq!(submissions.len(), 1);
    assert_eq!(
        submissions[0].source_title.as_deref(),
        Some("Deferred.Movie.2024.1080p.WEB-DL-GRP")
    );

    let store = wanted_items.store.lock().await.clone();
    let first = store
        .iter()
        .find(|item| item.id == first_wanted_id)
        .expect("first wanted item");
    let second = store
        .iter()
        .find(|item| item.id == second_wanted_id)
        .expect("second wanted item");
    assert_eq!(first.status, AcquisitionScopeStatus::Grabbed);
    assert_eq!(second.status, AcquisitionScopeStatus::Wanted);
    assert!(
        store
            .iter()
            .filter_map(|item| item.grabbed_release.as_deref())
            .all(|grabbed_release| !grabbed_release.contains("deduplicated")),
        "duplicate URL handling must not write grabbed dedupe metadata"
    );

    let release_decisions = wanted_items.release_decisions.lock().await.clone();
    assert!(release_decisions.iter().any(|decision| {
        decision.wanted_item_id == second_wanted_id
            && decision.release_url.as_deref() == Some(shared_url)
            && decision.decision_code == "eligible"
    }));
}

#[tokio::test]
async fn insert_pending_release_normalizes_source_password_flags() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user, _) = bootstrap_with_acquisition_tracking_and_indexer_and_release_attempts(
        download_client,
        download_submissions,
        pending_releases.clone(),
        wanted_items.clone(),
        Arc::new(MockIndexerClient),
    );
    let (title, wanted_id) = seed_movie_wanted_for_acquisition(
        &app,
        &user,
        &wanted_items,
        "Pending Password Movie",
        2024,
    )
    .await;
    let wanted = wanted_items
        .get_acquisition_scope_state_by_id(&wanted_id)
        .await
        .expect("load wanted item")
        .expect("wanted item exists");

    for (index, (label, raw, expected)) in [
        ("one", Some("1"), None),
        ("true", Some("true"), None),
        ("protected", Some("protected"), None),
        ("zero", Some("0"), None),
        ("false", Some("false"), None),
        ("empty", Some("  "), None),
        ("real", Some("actual-password"), Some("actual-password")),
    ]
    .into_iter()
    .enumerate()
    {
        app.insert_pending_release(
            &wanted,
            &title,
            &format!("Pending.Password.{label}.2024.1080p-GRP"),
            Some("https://example.invalid/pending-password.nzb"),
            Some(DownloadSourceKind::NzbUrl),
            Some(1_000),
            1000 + index as i32,
            None,
            Some("test-indexer"),
            Some(label),
            10,
            raw,
            Some("2024-01-01T00:00:00Z"),
            None,
        )
        .await;

        let stored = pending_releases
            .store
            .lock()
            .await
            .iter()
            .find(|release| release.release_guid.as_deref() == Some(label))
            .cloned()
            .expect("pending release should be stored");
        assert_eq!(stored.source_password.as_deref(), expected);
    }
}

#[tokio::test]
async fn legacy_pending_release_placeholder_password_is_normalized_on_grab() {
    let release_title = "Legacy.Placeholder.Password.Movie.2024.1080p-GRP";
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user, release_attempts) =
        bootstrap_with_acquisition_tracking_and_indexer_and_release_attempts(
            download_client.clone(),
            download_submissions,
            pending_releases.clone(),
            wanted_items.clone(),
            Arc::new(MockIndexerClient),
        );
    let (title, wanted_id) = seed_movie_wanted_for_acquisition(
        &app,
        &user,
        &wanted_items,
        "Legacy Placeholder Password Movie",
        2024,
    )
    .await;
    let mut pending = pending_movie_release(
        &wanted_id,
        &title,
        release_title,
        PendingReleaseStatus::Waiting,
    );
    pending.source_password = Some("1".to_string());
    let pending_id = pending.id.clone();
    pending_releases
        .insert_pending_release(&pending)
        .await
        .expect("seed pending release");

    let grabbed = app
        .force_grab_pending_release(&user, &pending_id)
        .await
        .expect("force grab pending release");

    assert!(grabbed);
    assert_eq!(
        download_client
            .submitted_source_passwords
            .lock()
            .await
            .as_slice(),
        &[None]
    );
    assert!(
        release_attempts
            .attempts
            .lock()
            .await
            .iter()
            .all(|attempt| attempt.source_password.is_none())
    );
}

#[tokio::test]
async fn legacy_pending_release_real_password_is_preserved_on_grab() {
    let release_title = "Legacy.Real.Password.Movie.2024.1080p-GRP";
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user, release_attempts) =
        bootstrap_with_acquisition_tracking_and_indexer_and_release_attempts(
            download_client.clone(),
            download_submissions,
            pending_releases.clone(),
            wanted_items.clone(),
            Arc::new(MockIndexerClient),
        );
    let (title, wanted_id) = seed_movie_wanted_for_acquisition(
        &app,
        &user,
        &wanted_items,
        "Legacy Real Password Movie",
        2024,
    )
    .await;
    let mut pending = pending_movie_release(
        &wanted_id,
        &title,
        release_title,
        PendingReleaseStatus::Waiting,
    );
    pending.source_password = Some("actual-password".to_string());
    let pending_id = pending.id.clone();
    pending_releases
        .insert_pending_release(&pending)
        .await
        .expect("seed pending release");

    let grabbed = app
        .force_grab_pending_release(&user, &pending_id)
        .await
        .expect("force grab pending release");

    assert!(grabbed);
    let submitted_passwords = download_client.submitted_source_passwords.lock().await;
    assert_eq!(submitted_passwords.len(), 1);
    assert!(matches!(
        submitted_passwords.first().and_then(|password| password.as_deref()),
        Some(password) if password == "actual-password"
    ));
    assert!(
        release_attempts
            .attempts
            .lock()
            .await
            .iter()
            .any(|attempt| attempt.source_password.as_deref() == Some("actual-password"))
    );
}

#[tokio::test]
async fn pending_release_submit_unavailable_records_pending_without_failed_signature() {
    let release_title = "Pending.Deferred.Movie.2024.1080p.WEB-DL-GRP";
    let download_client = Arc::new(StubDownloadClient::default());
    download_client
        .set_submit_error(Some(StubSubmitError::SubmitUnavailable(
            "download client unavailable".to_string(),
        )))
        .await;
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user, release_attempts) =
        bootstrap_with_acquisition_tracking_and_indexer_and_release_attempts(
            download_client,
            download_submissions,
            pending_releases.clone(),
            wanted_items.clone(),
            Arc::new(MockIndexerClient),
        );
    let (title, wanted_id) = seed_movie_wanted_for_acquisition(
        &app,
        &user,
        &wanted_items,
        "Pending Deferred Movie",
        2024,
    )
    .await;
    let pending_id = Id::new().0;
    let now = Utc::now().to_rfc3339();
    pending_releases
        .insert_pending_release(&PendingRelease {
            id: pending_id.clone(),
            wanted_item_id: wanted_id,
            title_id: title.id.clone(),
            release_title: release_title.to_string(),
            release_url: Some("https://example.invalid/pending-deferred.nzb".to_string()),
            source_kind: Some(DownloadSourceKind::NzbUrl),
            release_size_bytes: Some(1_000),
            release_score: 1000,
            scoring_log_json: None,
            indexer_source: Some("test-indexer".to_string()),
            release_guid: Some("pending-deferred-guid".to_string()),
            added_at: now.clone(),
            delay_until: now.clone(),
            status: PendingReleaseStatus::Waiting,
            grabbed_at: None,
            source_password: None,
            published_at: Some(now),
            info_hash: None,
        })
        .await
        .expect("seed pending release");

    let grabbed = app
        .force_grab_pending_release(&user, &pending_id)
        .await
        .expect("force grab pending release");

    assert!(!grabbed);
    assert_eq!(
        pending_releases
            .get_pending_release(&pending_id)
            .await
            .expect("load pending release")
            .expect("pending release exists")
            .status,
        PendingReleaseStatus::Waiting
    );
    let attempts = release_attempts.attempts.lock().await.clone();
    assert!(
        attempts
            .iter()
            .all(|attempt| attempt.outcome != ReleaseDownloadAttemptOutcome::Failed)
    );
    let failed = release_attempts
        .list_failed_release_signatures_for_title(&title.id, 10)
        .await
        .expect("list failed signatures");
    assert!(failed.is_empty());
}

struct PendingStatusAssertingIndexerClient {
    pending_releases: Arc<TrackingPendingReleaseRepo>,
    searches: Arc<Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl IndexerClient for PendingStatusAssertingIndexerClient {
    async fn search(
        &self,
        query: String,
        _ids: std::collections::HashMap<String, String>,
        _category: Option<String>,
        _facet: Option<String>,
        _id_search_facet: Option<String>,
        _newznab_categories: Option<Vec<String>>,
        _indexer_routing: Option<IndexerRoutingPlan>,
        _mode: SearchMode,
        _season: Option<u32>,
        _episode: Option<u32>,
        _absolute_episode: Option<u32>,
        _tagged_aliases: Vec<TaggedAlias>,
        _learning_context: Option<crate::IndexerSearchLearningContext>,
        _cancel_token: tokio_util::sync::CancellationToken,
    ) -> AppResult<IndexerSearchResponse> {
        let pending_was_grabbed = self
            .pending_releases
            .store
            .lock()
            .await
            .iter()
            .any(|release| release.status == PendingReleaseStatus::Grabbed);
        assert!(
            pending_was_grabbed,
            "scheduled RSS must process due pending releases before fresh RSS search"
        );
        self.searches.lock().await.push(query.clone());

        Ok(IndexerSearchResponse {
            indexer_outcomes: Vec::new(),
            results: vec![IndexerSearchResult {
                indexer_id: None,
                source: "nzbgeek".into(),
                title: format!("{query}.2024.1080p.WEB-DL"),
                link: Some("https://example.invalid/info/rss-ordering".to_string()),
                download_url: Some("https://example.invalid/download/rss-ordering.nzb".to_string()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                size_bytes: None,
                published_at: Some("1970-01-01T00:00:00Z".into()),
                thumbs_up: None,
                thumbs_down: None,
                indexer_languages: None,
                indexer_subtitles: None,
                indexer_grabs: None,
                password_hint: None,
                parsed_release_metadata: Some(crate::parse_release_metadata(&query)),
                quality_profile_decision: None,
                extra: Default::default(),
                response_attributes: Default::default(),
                guid: Some("guid-rss-ordering".to_string()),
                info_url: Some("https://example.invalid/info/rss-ordering".to_string()),
                provenance: None,
                auto_eligible: None,
                auto_decision_code: None,
                auto_decision_summary: None,
                candidate_token: None,
                queue_scope: None,
            }],
            api_current: None,
            api_max: None,
            grab_current: None,
            grab_max: None,
        })
    }
}

#[tokio::test]
async fn scheduled_rss_processes_due_pending_releases_before_fetching_fresh_rss() {
    let pending_title = "Scheduled.Pending.Movie.2024.1080p.WEB-DL-GRP";
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let indexer_searches = Arc::new(Mutex::new(Vec::new()));
    let indexer_client = Arc::new(PendingStatusAssertingIndexerClient {
        pending_releases: pending_releases.clone(),
        searches: indexer_searches.clone(),
    });
    let (app, user) = bootstrap_with_acquisition_tracking_and_indexer(
        download_client,
        download_submissions.clone(),
        pending_releases.clone(),
        wanted_items.clone(),
        indexer_client.clone(),
    );
    let (title, wanted_id) = seed_movie_wanted_for_acquisition(
        &app,
        &user,
        &wanted_items,
        "Scheduled Pending Movie",
        2024,
    )
    .await;
    let pending = pending_movie_release(
        &wanted_id,
        &title,
        pending_title,
        PendingReleaseStatus::Waiting,
    );
    let pending_id = pending.id.clone();
    pending_releases
        .insert_pending_release(&pending)
        .await
        .expect("seed pending release");

    let report = app.run_scheduled_rss_sync().await.expect("run RSS sync");

    assert_eq!(report.releases_grabbed, 0);
    assert!(
        !indexer_searches.lock().await.is_empty(),
        "fresh RSS should still run after the pending pre-pass"
    );
    assert_eq!(
        pending_releases
            .get_pending_release(&pending_id)
            .await
            .expect("load pending release")
            .expect("pending release exists")
            .status,
        PendingReleaseStatus::Grabbed
    );
    assert!(
        download_submissions
            .store
            .lock()
            .await
            .iter()
            .any(|submission| submission.source_title.as_deref() == Some(pending_title))
    );
}

#[tokio::test]
async fn expired_pending_release_submit_unavailable_stays_waiting_and_retries() {
    let release_title = "Delayed.Deferred.Movie.2024.1080p.WEB-DL-GRP";
    let download_client = Arc::new(StubDownloadClient::default());
    download_client
        .set_submit_error(Some(StubSubmitError::SubmitUnavailable(
            "download client unavailable".to_string(),
        )))
        .await;
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user, release_attempts) =
        bootstrap_with_acquisition_tracking_and_indexer_and_release_attempts(
            download_client.clone(),
            download_submissions.clone(),
            pending_releases.clone(),
            wanted_items.clone(),
            Arc::new(MockIndexerClient),
        );
    let (title, wanted_id) = seed_movie_wanted_for_acquisition(
        &app,
        &user,
        &wanted_items,
        "Delayed Deferred Movie",
        2024,
    )
    .await;
    let pending = pending_movie_release(
        &wanted_id,
        &title,
        release_title,
        PendingReleaseStatus::Waiting,
    );
    let pending_id = pending.id.clone();
    pending_releases
        .insert_pending_release(&pending)
        .await
        .expect("seed pending release");

    let grabbed = app
        .process_expired_pending_releases()
        .await
        .expect("process expired pending releases");

    assert_eq!(grabbed, 0);
    assert_eq!(
        pending_releases
            .get_pending_release(&pending_id)
            .await
            .expect("load pending release")
            .expect("pending release exists")
            .status,
        PendingReleaseStatus::Waiting
    );
    assert!(download_submissions.store.lock().await.is_empty());
    let attempts = release_attempts.attempts.lock().await.clone();
    assert!(
        attempts
            .iter()
            .all(|attempt| attempt.outcome != ReleaseDownloadAttemptOutcome::Failed)
    );
    assert!(attempts.iter().any(|attempt| {
        attempt.source_title.as_deref() == Some(release_title)
            && attempt.outcome == ReleaseDownloadAttemptOutcome::Pending
            && attempt
                .error_message
                .as_deref()
                .is_some_and(|message| message.contains("download client unavailable"))
    }));
    let failed = release_attempts
        .list_failed_release_signatures_for_title(&title.id, 10)
        .await
        .expect("list failed signatures");
    assert!(failed.is_empty());

    download_client.set_submit_error(None).await;
    let grabbed = app
        .process_expired_pending_releases()
        .await
        .expect("retry expired pending releases");

    assert_eq!(grabbed, 1);
    assert_eq!(
        pending_releases
            .get_pending_release(&pending_id)
            .await
            .expect("load pending release")
            .expect("pending release exists")
            .status,
        PendingReleaseStatus::Grabbed
    );
    assert!(
        download_submissions
            .store
            .lock()
            .await
            .iter()
            .any(|submission| submission.source_title.as_deref() == Some(release_title))
    );
}

#[tokio::test]
async fn expired_pending_release_ambiguous_error_stays_waiting_and_retries() {
    // An ambiguous submit (the request may have been accepted but the response
    // was lost) must be deferred exactly like an unavailable client: the
    // pending release stays Waiting, records a Pending (not Failed) attempt,
    // and is never blocklisted — then retried successfully next cycle.
    let release_title = "Ambiguous.Deferred.Movie.2024.1080p.WEB-DL-GRP";
    let download_client = Arc::new(StubDownloadClient::default());
    download_client
        .set_submit_error(Some(StubSubmitError::Ambiguous(
            "sabnzbd addfile response was lost after the upload was sent".to_string(),
        )))
        .await;
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user, release_attempts) =
        bootstrap_with_acquisition_tracking_and_indexer_and_release_attempts(
            download_client.clone(),
            download_submissions.clone(),
            pending_releases.clone(),
            wanted_items.clone(),
            Arc::new(MockIndexerClient),
        );
    let (title, wanted_id) = seed_movie_wanted_for_acquisition(
        &app,
        &user,
        &wanted_items,
        "Ambiguous Deferred Movie",
        2024,
    )
    .await;
    let pending = pending_movie_release(
        &wanted_id,
        &title,
        release_title,
        PendingReleaseStatus::Waiting,
    );
    let pending_id = pending.id.clone();
    pending_releases
        .insert_pending_release(&pending)
        .await
        .expect("seed pending release");

    let grabbed = app
        .process_expired_pending_releases()
        .await
        .expect("process expired pending releases");

    assert_eq!(grabbed, 0);
    assert_eq!(
        pending_releases
            .get_pending_release(&pending_id)
            .await
            .expect("load pending release")
            .expect("pending release exists")
            .status,
        PendingReleaseStatus::Waiting
    );
    assert!(download_submissions.store.lock().await.is_empty());
    let attempts = release_attempts.attempts.lock().await.clone();
    assert!(
        attempts
            .iter()
            .all(|attempt| attempt.outcome != ReleaseDownloadAttemptOutcome::Failed)
    );
    assert!(attempts.iter().any(|attempt| {
        attempt.source_title.as_deref() == Some(release_title)
            && attempt.outcome == ReleaseDownloadAttemptOutcome::Pending
    }));
    let failed = release_attempts
        .list_failed_release_signatures_for_title(&title.id, 10)
        .await
        .expect("list failed signatures");
    assert!(failed.is_empty());

    download_client.set_submit_error(None).await;
    let grabbed = app
        .process_expired_pending_releases()
        .await
        .expect("retry expired pending releases");

    assert_eq!(grabbed, 1);
    assert_eq!(
        pending_releases
            .get_pending_release(&pending_id)
            .await
            .expect("load pending release")
            .expect("pending release exists")
            .status,
        PendingReleaseStatus::Grabbed
    );
}

#[tokio::test]
async fn expired_pending_release_non_unavailable_error_expires_release() {
    let release_title = "Delayed.Rejected.Movie.2024.1080p.WEB-DL-GRP";
    let download_client = Arc::new(StubDownloadClient::default());
    download_client
        .set_submit_error(Some(StubSubmitError::Validation(
            "release rejected by client routing".to_string(),
        )))
        .await;
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user, release_attempts) =
        bootstrap_with_acquisition_tracking_and_indexer_and_release_attempts(
            download_client,
            download_submissions,
            pending_releases.clone(),
            wanted_items.clone(),
            Arc::new(MockIndexerClient),
        );
    let (title, wanted_id) = seed_movie_wanted_for_acquisition(
        &app,
        &user,
        &wanted_items,
        "Delayed Rejected Movie",
        2024,
    )
    .await;
    let pending = pending_movie_release(
        &wanted_id,
        &title,
        release_title,
        PendingReleaseStatus::Waiting,
    );
    let pending_id = pending.id.clone();
    pending_releases
        .insert_pending_release(&pending)
        .await
        .expect("seed pending release");

    let grabbed = app
        .process_expired_pending_releases()
        .await
        .expect("process expired pending releases");

    assert_eq!(grabbed, 0);
    assert_eq!(
        pending_releases
            .get_pending_release(&pending_id)
            .await
            .expect("load pending release")
            .expect("pending release exists")
            .status,
        PendingReleaseStatus::Expired
    );
    let failed = release_attempts
        .list_failed_release_signatures_for_title(&title.id, 10)
        .await
        .expect("list failed signatures");
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].source_title.as_deref(), Some(release_title));
}

#[tokio::test]
async fn rss_submit_unavailable_records_pending_without_failed_signature() {
    let release_title = "Rss.Deferred.Movie.2024.1080p.WEB-DL-GRP";
    let download_client = Arc::new(StubDownloadClient::default());
    download_client
        .set_submit_error(Some(StubSubmitError::SubmitUnavailable(
            "download client api unavailable".to_string(),
        )))
        .await;
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let indexer_client = Arc::new(FixedReleaseIndexerClient::new(release_title));
    let (app, user, release_attempts) =
        bootstrap_with_acquisition_tracking_and_indexer_and_release_attempts(
            download_client,
            download_submissions.clone(),
            pending_releases,
            wanted_items.clone(),
            indexer_client,
        );
    let (title, _) =
        seed_movie_wanted_for_acquisition(&app, &user, &wanted_items, "Rss Deferred Movie", 2024)
            .await;

    let report = app.run_scheduled_rss_sync().await.expect("run RSS sync");

    assert_eq!(report.releases_fetched, 1);
    assert_eq!(report.releases_matched, 1);
    assert_eq!(report.releases_grabbed, 0);
    assert!(download_submissions.store.lock().await.is_empty());
    let attempts = release_attempts.attempts.lock().await.clone();
    assert!(
        attempts
            .iter()
            .all(|attempt| attempt.outcome != ReleaseDownloadAttemptOutcome::Failed)
    );
    let failed = release_attempts
        .list_failed_release_signatures_for_title(&title.id, 10)
        .await
        .expect("list failed signatures");
    assert!(failed.is_empty());
}

#[tokio::test]
async fn acquisition_cycle_submits_paperman_media_request_candidate() {
    let release_title = "Paperman.2012.720p.WEB-DL.AV1.AAC2.0-NTb";
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let indexer_client = Arc::new(FixedReleaseIndexerClient::new(release_title));
    let (app, user) = bootstrap_with_acquisition_tracking_and_indexer(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
        wanted_items.clone(),
        indexer_client,
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Paperman".into(),
                sort_title: Some("Paperman".into()),
                slug: Some("paperman".into()),
                facet: MediaFacet::Movie,
                monitored: true,
                year: Some(2012),
                external_ids: vec![
                    ExternalId {
                        source: "tvdb".to_string(),
                        value: "5890".to_string(),
                    },
                    ExternalId {
                        source: "imdb".to_string(),
                        value: "tt2388725".to_string(),
                    },
                ],
                content_status: Some("Released".to_string()),
                min_availability: Some("released".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("create Paperman movie");
    wanted_items
        .remember_title_facet(&title.id, MediaFacet::Movie)
        .await;
    let wanted_id = Id::new().0;
    wanted_items
        .upsert_acquisition_scope_state(&AcquisitionScopeState {
            id: wanted_id.clone(),
            title_id: title.id.clone(),
            title_name: Some(title.name.clone()),
            title_slug: title.slug.clone(),
            title_facet: Some(MediaFacet::Movie.as_str().to_string()),
            library_id: Some(title.library_id.clone()),
            library_name: Some("Movies".to_string()),
            library_slug: Some("movies".to_string()),
            episode_id: None,
            collection_id: None,
            series_movie_link_id: None,
            season_number: None,
            episode_number: None,
            media_type: "movie".to_string(),
            last_search_at: None,
            status: AcquisitionScopeStatus::Wanted,
            grabbed_release: None,
            current_score: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        })
        .await
        .expect("seed Paperman wanted item");

    app.run_convergence_cycle_once().await;

    assert_eq!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .as_slice(),
        &[release_title.to_string()]
    );
    let submissions = download_submissions.store.lock().await.clone();
    assert_eq!(submissions.len(), 1);
    assert_eq!(submissions[0].title_id, title.id);
    assert_eq!(submissions[0].source_title.as_deref(), Some(release_title));
    assert_eq!(submissions[0].scope, SubmissionScope::Title);

    let decisions = wanted_items.release_decisions.lock().await.clone();
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0].wanted_item_id, wanted_id);
    assert_eq!(decisions[0].release_title, release_title);
    assert_eq!(decisions[0].decision_code, "eligible");
}

#[tokio::test]
async fn acquisition_cycle_submits_bluey_episode_media_request_candidate() {
    let release_title = "Bluey.S01E01.720p.WEB-DL.AV1.AAC2.0-NTb";
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let indexer_client = Arc::new(FixedReleaseIndexerClient::new(release_title));
    let (app, user) = bootstrap_with_acquisition_tracking_and_indexer(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
        wanted_items.clone(),
        indexer_client,
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Bluey (2018)".into(),
                sort_title: Some("Bluey".into()),
                slug: Some("bluey-2018".into()),
                facet: MediaFacet::Series,
                monitored: true,
                year: Some(2018),
                external_ids: vec![ExternalId {
                    source: "tvdb".to_string(),
                    value: "353546".to_string(),
                }],
                content_status: Some("Continuing".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("create Bluey series");
    wanted_items
        .remember_title_facet(&title.id, MediaFacet::Series)
        .await;

    let season = Collection {
        id: Id::new().0,
        title_id: title.id.clone(),
        collection_type: CollectionType::Season,
        collection_index: "1".to_string(),
        label: Some("Season 1".to_string()),
        ordered_path: Some("S01".to_string()),
        narrative_order: None,
        first_episode_number: Some("1".to_string()),
        last_episode_number: Some("1".to_string()),
        monitored: true,
        created_at: Utc::now(),
    };
    app.services
        .catalog
        .shows
        .create_collection(season.clone())
        .await
        .expect("create Bluey season");

    let episode = Episode {
        id: Id::new().0,
        title_id: title.id.clone(),
        collection_id: Some(season.id.clone()),
        episode_type: EpisodeType::Standard,
        episode_number: Some("1".to_string()),
        season_number: Some("1".to_string()),
        episode_label: Some("S01E01".to_string()),
        title: Some("The Magic Xylophone".to_string()),
        air_date: Some("2018-10-01".to_string()),
        duration_seconds: Some(420),
        has_multi_audio: false,
        has_subtitle: false,
        is_filler: false,
        is_recap: false,
        absolute_number: Some("1".to_string()),
        overview: None,
        tvdb_id: Some("7214505".to_string()),
        image_url: None,
        monitored: true,
        created_at: Utc::now(),
    };
    app.services
        .catalog
        .shows
        .create_episode(episode.clone())
        .await
        .expect("create Bluey episode");

    let wanted_id = Id::new().0;
    wanted_items
        .upsert_acquisition_scope_state(&AcquisitionScopeState {
            id: wanted_id.clone(),
            title_id: title.id.clone(),
            title_name: Some(title.name.clone()),
            title_slug: title.slug.clone(),
            title_facet: Some(MediaFacet::Series.as_str().to_string()),
            library_id: Some(title.library_id.clone()),
            library_name: Some("Series".to_string()),
            library_slug: Some("series".to_string()),
            episode_id: Some(episode.id.clone()),
            collection_id: Some(season.id.clone()),
            series_movie_link_id: None,
            season_number: Some("1".to_string()),
            episode_number: None,
            media_type: "episode".to_string(),
            last_search_at: None,
            status: AcquisitionScopeStatus::Wanted,
            grabbed_release: None,
            current_score: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        })
        .await
        .expect("seed Bluey wanted item");

    app.run_convergence_cycle_once().await;

    assert_eq!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .as_slice(),
        &[release_title.to_string()]
    );
    let submissions = download_submissions.store.lock().await.clone();
    assert_eq!(submissions.len(), 1);
    assert_eq!(submissions[0].title_id, title.id);
    assert_eq!(submissions[0].source_title.as_deref(), Some(release_title));
    assert_eq!(
        submissions[0].scope,
        SubmissionScope::Episode {
            episode_id: episode.id.clone()
        }
    );

    let decisions = wanted_items.release_decisions.lock().await.clone();
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0].wanted_item_id, wanted_id);
    assert_eq!(decisions[0].release_title, release_title);
    assert_eq!(decisions[0].decision_code, "eligible");
}

#[tokio::test]
async fn acquisition_cycle_title_submission_still_blocks_movie_search() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let indexer_client = Arc::new(TrackingIndexerClient::default());
    let (app, user) = bootstrap_with_acquisition_tracking_and_indexer(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
        wanted_items.clone(),
        indexer_client.clone(),
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Movie Blocking Scope".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create movie");

    wanted_items
        .upsert_acquisition_scope_state(&AcquisitionScopeState {
            id: Id::new().0,
            title_id: title.id.clone(),
            title_name: Some(title.name.clone()),
            title_slug: None,
            title_facet: None,
            library_id: None,
            library_name: None,
            library_slug: None,
            episode_id: None,
            collection_id: None,
            series_movie_link_id: None,
            season_number: None,
            episode_number: None,
            media_type: "movie".to_string(),
            last_search_at: None,
            status: AcquisitionScopeStatus::Wanted,
            grabbed_release: None,
            current_score: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        })
        .await
        .expect("seed due movie wanted item");

    download_submissions
        .record_submission(DownloadSubmission {
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "movie".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "movie-active".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Movie.Blocking.Scope".to_string()),
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record active movie submission");

    *download_client.queue_items.lock().await = vec![DownloadQueueItem {
        id: "movie-active".to_string(),
        title_id: Some(title.id.clone()),
        episode_id: None,
        title_name: title.name.clone(),
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
        attention_reason: None,
        download_client_item_id: "movie-active".to_string(),
        download_id: None,
        import_status: None,
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
    }];

    app.run_convergence_cycle_once().await;

    assert!(indexer_client.searches.lock().await.is_empty());
}

#[tokio::test]
async fn acquisition_cycle_skips_due_search_when_no_download_clients_are_enabled() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let indexer_client = Arc::new(TrackingIndexerClient::default());
    let (app, user) = bootstrap_with_acquisition_tracking_and_indexer(
        download_client,
        download_submissions,
        pending_releases,
        wanted_items.clone(),
        indexer_client.clone(),
    );

    let default_client = app
        .list_download_client_configs(&user, None)
        .await
        .expect("list download client configs")
        .into_iter()
        .next()
        .expect("default download client");
    app.update_download_client_config(
        &user,
        crate::DownloadClientConfigUpdate {
            id: default_client.id.clone(),
            is_enabled: Some(false),
            ..Default::default()
        },
    )
    .await
    .expect("disable default download client");

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "No Downloader Search Gate".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create movie");
    wanted_items
        .remember_title_facet(&title.id, MediaFacet::Movie)
        .await;

    wanted_items
        .upsert_acquisition_scope_state(&AcquisitionScopeState {
            id: Id::new().0,
            title_id: title.id.clone(),
            title_name: Some(title.name.clone()),
            title_slug: None,
            title_facet: None,
            library_id: None,
            library_name: None,
            library_slug: None,
            episode_id: None,
            collection_id: None,
            series_movie_link_id: None,
            season_number: None,
            episode_number: None,
            media_type: "movie".to_string(),
            last_search_at: None,
            status: AcquisitionScopeStatus::Wanted,
            grabbed_release: None,
            current_score: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        })
        .await
        .expect("seed due movie wanted item");

    app.run_convergence_cycle_once().await;

    assert!(indexer_client.searches.lock().await.is_empty());
}

#[tokio::test]
async fn acquisition_cycle_active_anime_scan_does_not_block_due_movie_search() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let indexer_client = Arc::new(TrackingIndexerClient::default());
    let (app, user) = bootstrap_with_acquisition_tracking_and_indexer(
        download_client,
        download_submissions,
        pending_releases,
        wanted_items.clone(),
        indexer_client.clone(),
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Movie Survives Anime Scan".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create movie");
    wanted_items
        .remember_title_facet(&title.id, MediaFacet::Movie)
        .await;

    wanted_items
        .upsert_acquisition_scope_state(&AcquisitionScopeState {
            id: Id::new().0,
            title_id: title.id.clone(),
            title_name: Some(title.name.clone()),
            title_slug: None,
            title_facet: None,
            library_id: None,
            library_name: None,
            library_slug: None,
            episode_id: None,
            collection_id: None,
            series_movie_link_id: None,
            season_number: None,
            episode_number: None,
            media_type: "movie".to_string(),
            last_search_at: None,
            status: AcquisitionScopeStatus::Wanted,
            grabbed_release: None,
            current_score: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        })
        .await
        .expect("seed due movie wanted item");

    crate::acquisition_workflow::run_convergence_cycle_with_blocked_facets(
        &app,
        &[MediaFacet::Anime],
    )
    .await;

    let searches = indexer_client.searches.lock().await.clone();
    assert_eq!(searches.len(), 1);
    assert_eq!(searches[0].query, title.name);
    assert_eq!(searches[0].season, None);
    assert_eq!(searches[0].episode, None);
}

#[tokio::test]
async fn rss_sync_skips_indexer_search_when_no_download_clients_are_enabled() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let indexer_client = Arc::new(TrackingIndexerClient::default());
    let (app, user) = bootstrap_with_acquisition_tracking_and_indexer(
        download_client,
        download_submissions,
        pending_releases,
        wanted_items,
        indexer_client.clone(),
    );

    let default_client = app
        .list_download_client_configs(&user, None)
        .await
        .expect("list download client configs")
        .into_iter()
        .next()
        .expect("default download client");
    app.update_download_client_config(
        &user,
        crate::DownloadClientConfigUpdate {
            id: default_client.id.clone(),
            is_enabled: Some(false),
            ..Default::default()
        },
    )
    .await
    .expect("disable default download client");

    app.add_title(
        &user,
        NewTitle {
            name: "RSS Skip Without Downloader".into(),
            facet: MediaFacet::Movie,
            monitored: true,
            tags: vec![],
            external_ids: vec![],
            min_availability: None,
            ..Default::default()
        },
    )
    .await
    .expect("create monitored movie");

    let report = app.run_scheduled_rss_sync().await.expect("run RSS sync");

    assert!(indexer_client.searches.lock().await.is_empty());
    assert_eq!(report.releases_fetched, 0);
    assert_eq!(report.releases_matched, 0);
    assert_eq!(report.releases_grabbed, 0);
    assert_eq!(report.releases_held, 0);
}

#[tokio::test]
async fn acquisition_cycle_active_movie_scan_does_not_block_due_series_search() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let indexer_client = Arc::new(TrackingIndexerClient::default());
    let (app, user) = bootstrap_with_acquisition_tracking_and_indexer(
        download_client,
        download_submissions,
        pending_releases,
        wanted_items.clone(),
        indexer_client.clone(),
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Series Survives Movie Scan".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create series");
    wanted_items
        .remember_title_facet(&title.id, MediaFacet::Series)
        .await;

    let season_one = app
        .services
        .catalog
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: Some("1".to_string()),
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("1".to_string()),
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create season");

    let episode = app
        .services
        .catalog
        .shows
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(season_one.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E01".to_string()),
            title: Some("Pilot".to_string()),
            air_date: Some("2024-01-01".to_string()),
            duration_seconds: Some(1_440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create episode");

    wanted_items
        .upsert_acquisition_scope_state(&AcquisitionScopeState {
            id: Id::new().0,
            title_id: title.id.clone(),
            title_name: Some(title.name.clone()),
            title_slug: None,
            title_facet: None,
            library_id: None,
            library_name: None,
            library_slug: None,
            episode_id: Some(episode.id.clone()),
            collection_id: None,
            series_movie_link_id: None,
            season_number: Some("1".to_string()),
            episode_number: None,
            media_type: "episode".to_string(),
            last_search_at: None,
            status: AcquisitionScopeStatus::Wanted,
            grabbed_release: None,
            current_score: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        })
        .await
        .expect("seed due series wanted item");

    crate::acquisition_workflow::run_convergence_cycle_with_blocked_facets(
        &app,
        &[MediaFacet::Movie],
    )
    .await;

    let searches = indexer_client.searches.lock().await.clone();
    assert!(!searches.is_empty());
    assert!(
        searches
            .iter()
            .all(|search| search.query.contains(&title.name)
                && search.season == Some(1)
                && search.episode == Some(1))
    );
}

#[tokio::test]
async fn acquisition_cycle_active_series_scan_defers_due_series_search() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let indexer_client = Arc::new(TrackingIndexerClient::default());
    let (app, user) = bootstrap_with_acquisition_tracking_and_indexer(
        download_client,
        download_submissions,
        pending_releases,
        wanted_items.clone(),
        indexer_client.clone(),
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Series Deferred By Series Scan".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create series");
    wanted_items
        .remember_title_facet(&title.id, MediaFacet::Series)
        .await;

    let season_one = app
        .services
        .catalog
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: Some("1".to_string()),
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("1".to_string()),
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create season");

    let episode = app
        .services
        .catalog
        .shows
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(season_one.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E01".to_string()),
            title: Some("Pilot".to_string()),
            air_date: Some("2024-01-01".to_string()),
            duration_seconds: Some(1_440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create episode");

    wanted_items
        .upsert_acquisition_scope_state(&AcquisitionScopeState {
            id: Id::new().0,
            title_id: title.id.clone(),
            title_name: Some(title.name.clone()),
            title_slug: None,
            title_facet: None,
            library_id: None,
            library_name: None,
            library_slug: None,
            episode_id: Some(episode.id.clone()),
            collection_id: None,
            series_movie_link_id: None,
            season_number: Some("1".to_string()),
            episode_number: None,
            media_type: "episode".to_string(),
            last_search_at: None,
            status: AcquisitionScopeStatus::Wanted,
            grabbed_release: None,
            current_score: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        })
        .await
        .expect("seed due series wanted item");

    crate::acquisition_workflow::run_convergence_cycle_with_blocked_facets(
        &app,
        &[MediaFacet::Series],
    )
    .await;

    assert!(indexer_client.searches.lock().await.is_empty());
}

#[tokio::test]
async fn acquisition_cycle_retries_standby_candidate_during_unrelated_active_scan() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases.clone(),
        wanted_items.clone(),
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Failure Recovery During Scan".into(),
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
    wanted_items
        .remember_title_facet(&title.id, MediaFacet::Movie)
        .await;

    let wanted = AcquisitionScopeState {
        id: Id::new().0,
        title_id: title.id.clone(),
        title_name: Some(title.name.clone()),
        title_slug: None,
        title_facet: None,
        library_id: None,
        library_name: None,
        library_slug: None,
        episode_id: None,
        collection_id: None,
        series_movie_link_id: None,
        season_number: None,
        episode_number: None,
        media_type: "movie".to_string(),
        last_search_at: Some((Utc::now() - chrono::Duration::minutes(5)).to_rfc3339()),
        status: AcquisitionScopeStatus::Grabbed,
        grabbed_release: Some(
            serde_json::json!({
                "title": "Failed.Release.1080p.WEB-DL",
                "score": 100,
                "grabbed_at": Utc::now().to_rfc3339(),
            })
            .to_string(),
        ),
        current_score: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
    };
    wanted_items
        .upsert_acquisition_scope_state(&wanted)
        .await
        .expect("seed wanted item");

    pending_releases
        .insert_pending_release(&PendingRelease {
            id: Id::new().0,
            wanted_item_id: wanted.id.clone(),
            title_id: title.id.clone(),
            release_title: "Standby.Release.1080p.WEB-DL".to_string(),
            release_url: Some("https://example.com/standby.nzb".to_string()),
            source_kind: Some(DownloadSourceKind::NzbUrl),
            release_size_bytes: Some(1_000),
            release_score: 150,
            scoring_log_json: None,
            indexer_source: Some("nzbgeek".to_string()),
            release_guid: Some("guid-standby".to_string()),
            added_at: Utc::now().to_rfc3339(),
            delay_until: Utc::now().to_rfc3339(),
            status: PendingReleaseStatus::Standby,
            grabbed_at: None,
            source_password: None,
            published_at: Some(Utc::now().to_rfc3339()),
            info_hash: None,
        })
        .await
        .expect("seed standby");

    download_submissions
        .record_submission(DownloadSubmission {
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "movie".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "failed-job".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Failed.Release.1080p.WEB-DL".to_string()),
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record failed submission");

    *download_client.history_items.lock().await = vec![failed_history_item(
        "failed-job",
        "Failed.Release.1080p.WEB-DL",
    )];

    crate::acquisition_workflow::run_convergence_cycle_with_blocked_facets(
        &app,
        &[MediaFacet::Anime],
    )
    .await;

    assert_eq!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .clone(),
        vec!["Standby.Release.1080p.WEB-DL".to_string()]
    );
}

#[tokio::test]
async fn acquisition_cycle_prunes_stale_standby_rows_during_unrelated_active_scan() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking(
        download_client,
        download_submissions,
        pending_releases.clone(),
        wanted_items.clone(),
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Prune During Scan".into(),
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

    let wanted = AcquisitionScopeState {
        id: Id::new().0,
        title_id: title.id.clone(),
        title_name: Some(title.name.clone()),
        title_slug: None,
        title_facet: None,
        library_id: None,
        library_name: None,
        library_slug: None,
        episode_id: None,
        collection_id: None,
        series_movie_link_id: None,
        season_number: None,
        episode_number: None,
        media_type: "movie".to_string(),
        last_search_at: None,
        status: AcquisitionScopeStatus::Wanted,
        grabbed_release: None,
        current_score: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
    };
    wanted_items
        .upsert_acquisition_scope_state(&wanted)
        .await
        .expect("seed wanted item");

    pending_releases
        .insert_pending_release(&PendingRelease {
            id: Id::new().0,
            wanted_item_id: wanted.id.clone(),
            title_id: title.id.clone(),
            release_title: "Stale.Standby.Release".to_string(),
            release_url: Some("https://example.com/stale.nzb".to_string()),
            source_kind: Some(DownloadSourceKind::NzbUrl),
            release_size_bytes: None,
            release_score: 100,
            scoring_log_json: None,
            indexer_source: Some("nzbgeek".to_string()),
            release_guid: Some("guid-stale".to_string()),
            added_at: (Utc::now() - chrono::Duration::hours(30)).to_rfc3339(),
            delay_until: Utc::now().to_rfc3339(),
            status: PendingReleaseStatus::Standby,
            grabbed_at: None,
            source_password: None,
            published_at: None,
            info_hash: None,
        })
        .await
        .expect("seed stale standby");

    app.runtime
        .library
        .library_scan_tracker
        .start_session_with_id(
            "anime-scan-during-prune".to_string(),
            MediaFacet::Anime,
            LibraryScanMode::Full,
        )
        .await
        .expect("start anime scan");

    crate::acquisition_workflow::run_convergence_cycle_with_blocked_facets(
        &app,
        &[MediaFacet::Anime],
    )
    .await;

    assert!(
        pending_releases
            .list_all_standby_pending_releases()
            .await
            .expect("list standby")
            .is_empty()
    );
}

#[tokio::test]
async fn trigger_title_mismatch_recovery_search_requeues_only_mismatch_only_items() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking(
        download_client,
        download_submissions,
        pending_releases,
        wanted_items.clone(),
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Mismatch Recovery".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let recovery_item = AcquisitionScopeState {
        id: Id::new().0,
        title_id: title.id.clone(),
        title_name: Some(title.name.clone()),
        title_slug: None,
        title_facet: None,
        library_id: None,
        library_name: None,
        library_slug: None,
        episode_id: None,
        collection_id: None,
        series_movie_link_id: None,
        season_number: None,
        episode_number: None,
        media_type: "movie".to_string(),
        last_search_at: Some("2026-04-21T00:00:00Z".to_string()),
        status: AcquisitionScopeStatus::Wanted,
        grabbed_release: None,
        current_score: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
    };
    let untouched_item = AcquisitionScopeState {
        id: Id::new().0,
        title_id: title.id.clone(),
        title_name: Some(title.name.clone()),
        title_slug: None,
        title_facet: None,
        library_id: None,
        library_name: None,
        library_slug: None,
        episode_id: Some("episode-2".to_string()),
        collection_id: None,
        series_movie_link_id: None,
        season_number: Some("1".to_string()),
        episode_number: None,
        media_type: "episode".to_string(),
        last_search_at: Some("2026-04-21T00:00:00Z".to_string()),
        status: AcquisitionScopeStatus::Wanted,
        grabbed_release: None,
        current_score: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
    };
    wanted_items
        .upsert_acquisition_scope_state(&recovery_item)
        .await
        .expect("seed recovery item");
    wanted_items
        .upsert_acquisition_scope_state(&untouched_item)
        .await
        .expect("seed untouched item");

    for suffix in 0..3 {
        wanted_items
            .insert_release_decision(&ReleaseDecision {
                id: format!("decision-recovery-{suffix}"),
                wanted_item_id: recovery_item.id.clone(),
                title_id: title.id.clone(),
                release_title: format!("Mismatch.Release.{suffix}"),
                release_url: None,
                release_size_bytes: None,
                decision_code: "title_mismatch".to_string(),
                candidate_score: 100,
                current_score: None,
                score_delta: None,
                explanation_json: None,
                created_at: Utc::now().to_rfc3339(),
            })
            .await
            .expect("seed mismatch decision");
    }
    wanted_items
        .insert_release_decision(&ReleaseDecision {
            id: "decision-untouched-1".to_string(),
            wanted_item_id: untouched_item.id.clone(),
            title_id: title.id.clone(),
            release_title: "Mixed.Release".to_string(),
            release_url: None,
            release_size_bytes: None,
            decision_code: "title_mismatch".to_string(),
            candidate_score: 100,
            current_score: None,
            score_delta: None,
            explanation_json: None,
            created_at: Utc::now().to_rfc3339(),
        })
        .await
        .expect("seed mixed decision");
    wanted_items
        .insert_release_decision(&ReleaseDecision {
            id: "decision-untouched-2".to_string(),
            wanted_item_id: untouched_item.id.clone(),
            title_id: title.id.clone(),
            release_title: "Eligible.Release".to_string(),
            release_url: None,
            release_size_bytes: None,
            decision_code: "eligible".to_string(),
            candidate_score: 120,
            current_score: None,
            score_delta: None,
            explanation_json: None,
            created_at: Utc::now().to_rfc3339(),
        })
        .await
        .expect("seed non-mismatch decision");

    let queued = app
        .trigger_title_mismatch_recovery_search(&user, &title.id)
        .await
        .expect("trigger mismatch recovery");

    assert_eq!(queued, 1);

    // Mismatch recovery re-opens only the mismatch-only scope for
    // convergence (state row reset + coverage pruned); the eligible scope is
    // left untouched. The re-open is the sole state write on the recovery item.
    let updated_recovery = wanted_items
        .get_acquisition_scope_state_by_id(&recovery_item.id)
        .await
        .expect("load recovery item")
        .expect("recovery item exists");
    assert_eq!(updated_recovery.status, AcquisitionScopeStatus::Wanted);
    assert_eq!(
        wanted_items
            .status_update_call_count_for(&recovery_item.id)
            .await,
        1,
        "the mismatch-only scope is re-opened exactly once"
    );
    assert_eq!(
        wanted_items
            .status_update_call_count_for(&untouched_item.id)
            .await,
        0,
        "the eligible scope is never touched by mismatch recovery"
    );
}

#[tokio::test]
async fn acquisition_cycle_prunes_stale_standby_rows_for_non_grabbed_items() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking(
        download_client,
        download_submissions,
        pending_releases.clone(),
        wanted_items.clone(),
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Prune Me".into(),
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

    let wanted = AcquisitionScopeState {
        id: Id::new().0,
        title_id: title.id.clone(),
        title_name: Some(title.name.clone()),
        title_slug: None,
        title_facet: None,
        library_id: None,
        library_name: None,
        library_slug: None,
        episode_id: None,
        collection_id: None,
        series_movie_link_id: None,
        season_number: None,
        episode_number: None,
        media_type: "movie".to_string(),
        last_search_at: None,
        status: AcquisitionScopeStatus::Wanted,
        grabbed_release: None,
        current_score: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
    };
    wanted_items
        .upsert_acquisition_scope_state(&wanted)
        .await
        .expect("seed wanted item");

    pending_releases
        .insert_pending_release(&PendingRelease {
            id: Id::new().0,
            wanted_item_id: wanted.id.clone(),
            title_id: title.id.clone(),
            release_title: "Stale.Standby.Release".to_string(),
            release_url: Some("https://example.com/stale.nzb".to_string()),
            source_kind: Some(DownloadSourceKind::NzbUrl),
            release_size_bytes: None,
            release_score: 100,
            scoring_log_json: None,
            indexer_source: Some("nzbgeek".to_string()),
            release_guid: Some("guid-stale".to_string()),
            added_at: (Utc::now() - chrono::Duration::hours(30)).to_rfc3339(),
            delay_until: Utc::now().to_rfc3339(),
            status: PendingReleaseStatus::Standby,
            grabbed_at: None,
            source_password: None,
            published_at: None,
            info_hash: None,
        })
        .await
        .expect("seed stale standby");

    app.run_convergence_cycle_once().await;

    assert!(
        pending_releases
            .list_all_standby_pending_releases()
            .await
            .expect("list standby")
            .is_empty()
    );
}

// ── RSS match-time targets + pack-granularity grabs ────────────
//
// Target-ness is derived from library state at match time (monitored scope,
// missing or below cutoff); a pre-existing wanted row no longer gates the grab.
// The activity ledger row is materialized on the first anchored write, packs
// converge once at pack granularity, and paused scopes are never grabbed.

/// RSS acquisition bootstrap that exposes the media-file store and quality
/// profiles, so cutoff/upgrade target-ness can be driven from library state.
/// Mirrors `bootstrap_with_acquisition_tracking_and_indexer` otherwise.
async fn bootstrap_rss_with_media_files_and_profiles(
    download_client: Arc<StubDownloadClient>,
    download_submissions: Arc<TrackingDownloadSubmissionRepo>,
    pending_releases: Arc<TrackingPendingReleaseRepo>,
    acquisition_scope_states: Arc<TrackingAcquisitionScopeStateRepo>,
    media_files: Arc<MockMediaFileRepo>,
    quality_profiles: Arc<StoredQualityProfileRepo>,
    indexer_client: Arc<dyn IndexerClient>,
) -> (AppUseCase, User) {
    let titles = Arc::new(MockTitleRepo::default());
    let shows = Arc::new(MockShowRepo::default());
    let users = Arc::new(MockUserRepo::default());
    // RSS grabs and coverage need a routed indexer for the
    // scope; seed a synthetic direct-Newznab indexer the fake client answers for.
    let indexer_configs = Arc::new(MockIndexerConfigRepo {
        store: Arc::new(Mutex::new(vec![synthetic_direct_nab_indexer_config(
            "acquisition-indexer",
            "newznab",
        )])),
    });
    let download_client_configs = Arc::new(MockDownloadClientConfigRepo::default());
    download_client_configs
        .store
        .try_lock()
        .expect("download client config store should not be contended during bootstrap")
        .push(DownloadClientConfig {
            id: "background-search-default-client".to_string(),
            name: "Background Search Default Client".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 10_000,
            is_enabled: true,
            status: scryer_domain::DownloadClientStatus::Healthy,
            last_error: None,
            last_seen_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        });
    let release_attempts = Arc::new(MockReleaseAttemptRepo::default());
    let settings = Arc::new(StoredSettingsRepo::default());
    // Point profile resolution at the stored test profile (which shares the
    // default profile id) so its cutoff/upgrade criteria drive RSS target-ness.
    settings
        .set_value(
            SETTINGS_SCOPE_SYSTEM,
            QUALITY_PROFILE_ID_KEY,
            &format!("\"{}\"", crate::default_quality_profile_for_search().id),
        )
        .await;

    let services = AppServices::builder(
        titles,
        shows,
        users,
        indexer_configs,
        indexer_client,
        download_client,
        download_client_configs,
        release_attempts,
        settings,
        quality_profiles,
        String::new(),
    )
    .with_domain_events(Arc::new(MockDomainEventRepo::default()))
    .with_download_submissions(download_submissions.clone())
    .with_pending_releases(pending_releases.clone())
    .with_blocklist_repo(Arc::new(MockBlocklistRepo::default()))
    .with_libraries(Arc::new(MockLibraryRepo::default()))
    .with_media_files(media_files)
    .build_partial_for_tests();

    let mut registry = FacetRegistry::new();
    registry.register(Arc::new(MovieFacetHandler));
    registry.register(Arc::new(SeriesFacetHandler::new(
        scryer_domain::MediaFacet::Series,
    )));
    registry.register(Arc::new(SeriesFacetHandler::new(
        scryer_domain::MediaFacet::Anime,
    )));
    let app = AppUseCase::new(
        services,
        JwtAuthConfig {
            issuer: "scryer-test".to_string(),
            access_ttl_seconds: 3600,
            jwt_signing_salt: "test-salt".to_string(),
        },
        Arc::new(registry),
    );
    let app = app.with_test_overrides(|services| {
        services
            .with_acquisition_state(Arc::new(TrackingAcquisitionStateRepo {
                download_submissions,
                pending_releases,
                acquisition_scope_states: acquisition_scope_states.clone(),
            }))
            .with_acquisition_scope_states(acquisition_scope_states)
    });
    (app, test_admin_user())
}

/// A thinned acquisition-state row (post-RFC-119 shape) for seeding tests.
fn rfc119_wanted_state(
    title: &Title,
    media_type: &str,
    episode_id: Option<String>,
    status: AcquisitionScopeStatus,
) -> AcquisitionScopeState {
    let now = Utc::now().to_rfc3339();
    AcquisitionScopeState {
        id: Id::new().0,
        title_id: title.id.clone(),
        title_name: Some(title.name.clone()),
        title_slug: title.slug.clone(),
        title_facet: Some(title.facet.as_str().to_string()),
        library_id: Some(title.library_id.clone()),
        library_name: None,
        library_slug: None,
        episode_id,
        collection_id: None,
        series_movie_link_id: None,
        season_number: None,
        episode_number: None,
        media_type: media_type.to_string(),
        last_search_at: None,
        status,
        grabbed_release: None,
        current_score: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: now.clone(),
        updated_at: now,
    }
}

#[tokio::test]
async fn completing_absent_acquisition_state_row_does_not_materialize_completed_row() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking(
        download_client,
        download_submissions,
        pending_releases,
        wanted_items.clone(),
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Passive Import Fixture".into(),
                facet: MediaFacet::Movie,
                monitored: false,
                ..Default::default()
            },
        )
        .await
        .expect("create passive import title");

    crate::import_workflow::mark_wanted_completed(&app, &title.id, None, Some(1234)).await;

    assert!(
        wanted_items.store.lock().await.is_empty(),
        "passive completion must not synthesize acquisition-state rows; convergence derives targets from library state"
    );
}

/// A missing, monitored movie with NO acquisition-state row is upgraded from a
/// matching RSS release: the wanted-row gate is gone (§D5), and the grab
/// materializes the state row and transitions it to grabbed.
#[tokio::test]
async fn rss_grabs_missing_movie_with_no_wanted_row_and_creates_state_row() {
    let release_title = "Convergent.Skyline.2024.1080p.WEB-DL-GRP";
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let indexer_client = Arc::new(FixedReleaseIndexerClient::new(release_title));
    let (app, user) = bootstrap_with_acquisition_tracking_and_indexer(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
        wanted_items.clone(),
        indexer_client,
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Convergent Skyline".into(),
                sort_title: Some("Convergent Skyline".into()),
                slug: Some("convergent-skyline".into()),
                facet: MediaFacet::Movie,
                monitored: true,
                year: Some(2024),
                content_status: Some("Released".into()),
                min_availability: Some("announced".into()),
                ..Default::default()
            },
        )
        .await
        .expect("create monitored movie");
    wanted_items
        .remember_title_facet(&title.id, MediaFacet::Movie)
        .await;

    // No wanted row exists for this scope before the sync.
    assert!(
        wanted_items
            .get_acquisition_scope_state_for_title(&title.id, None)
            .await
            .expect("query wanted")
            .is_none()
    );

    let report = app.run_scheduled_rss_sync().await.expect("run RSS sync");

    assert_eq!(
        report.releases_grabbed, 1,
        "missing monitored movie is grabbed"
    );
    // The grab materialized the state row and transitioned it to grabbed.
    let seeded = wanted_items
        .get_acquisition_scope_state_for_title(&title.id, None)
        .await
        .expect("query wanted")
        .expect("state row materialized on grab");
    assert_eq!(seeded.status, AcquisitionScopeStatus::Grabbed);
    assert!(
        download_submissions
            .store
            .lock()
            .await
            .iter()
            .any(|submission| submission.title_id == title.id
                && submission.scope == SubmissionScope::Title),
        "movie grab records a title-scope submission"
    );
}

/// A paused scope is never grabbed even when it is a monitored, missing target
/// with a matching release (§D5 — user intent wins).
#[tokio::test]
async fn rss_does_not_grab_paused_scope() {
    let release_title = "Paused.Harbor.2024.1080p.WEB-DL-GRP";
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let indexer_client = Arc::new(FixedReleaseIndexerClient::new(release_title));
    let (app, user) = bootstrap_with_acquisition_tracking_and_indexer(
        download_client,
        download_submissions.clone(),
        pending_releases,
        wanted_items.clone(),
        indexer_client,
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Paused Harbor".into(),
                sort_title: Some("Paused Harbor".into()),
                slug: Some("paused-harbor".into()),
                facet: MediaFacet::Movie,
                monitored: true,
                year: Some(2024),
                content_status: Some("Released".into()),
                min_availability: Some("announced".into()),
                ..Default::default()
            },
        )
        .await
        .expect("create monitored movie");
    wanted_items
        .remember_title_facet(&title.id, MediaFacet::Movie)
        .await;
    wanted_items
        .upsert_acquisition_scope_state(&rfc119_wanted_state(
            &title,
            "movie",
            None,
            AcquisitionScopeStatus::Paused,
        ))
        .await
        .expect("seed paused state row");

    let report = app.run_scheduled_rss_sync().await.expect("run RSS sync");

    assert_eq!(report.releases_grabbed, 0, "paused scope is never grabbed");
    assert!(download_submissions.store.lock().await.is_empty());
}

/// A season pack matching two monitored, missing episodes is grabbed once at
/// pack granularity (§D5 #3): a single submission carrying the pack submission
/// scope, not one grab per member episode.
#[tokio::test]
async fn rss_grabs_season_pack_once_at_pack_granularity() {
    let release_title = "Cascade.Falls.S01.COMPLETE.1080p.WEB-DL-GRP";
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let indexer_client = Arc::new(FixedReleaseIndexerClient::new(release_title));
    let (app, user) = bootstrap_with_acquisition_tracking_and_indexer(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
        wanted_items.clone(),
        indexer_client,
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Cascade Falls".into(),
                sort_title: Some("Cascade Falls".into()),
                slug: Some("cascade-falls".into()),
                facet: MediaFacet::Series,
                monitored: true,
                content_status: Some("Continuing".into()),
                ..Default::default()
            },
        )
        .await
        .expect("create monitored series");
    wanted_items
        .remember_title_facet(&title.id, MediaFacet::Series)
        .await;

    let season = app
        .create_collection(
            &user,
            title.id.clone(),
            "season".into(),
            "1".into(),
            Some("Season 1".into()),
            None,
            Some("1".into()),
            Some("2".into()),
        )
        .await
        .expect("create season");
    let mut episode_ids = Vec::new();
    for episode_number in 1..=2 {
        let episode = app
            .create_episode(
                &user,
                title.id.clone(),
                Some(season.id.clone()),
                "standard".into(),
                Some("1".into()),
                Some(episode_number.to_string()),
                Some(format!("S01E{episode_number:02}")),
                Some(format!("S01E{episode_number:02}")),
                Some("2024-01-01".into()),
                Some(1_440),
                false,
                false,
            )
            .await
            .expect("create episode");
        episode_ids.push(episode.id);
    }

    // No per-episode wanted rows exist — the pack is a target purely from
    // library state (both members monitored + missing).
    let report = app.run_scheduled_rss_sync().await.expect("run RSS sync");

    assert_eq!(
        report.releases_grabbed, 1,
        "the pack is grabbed exactly once"
    );
    let submissions = download_submissions.store.lock().await.clone();
    assert_eq!(
        submissions.len(),
        1,
        "the pack is submitted once, not once per member episode"
    );
    // The pack submission carries the pack scope (season collection or the
    // covered episode set), never a single-episode scope.
    let scope = &submissions[0].scope;
    assert!(
        matches!(
            scope,
            SubmissionScope::Collection { .. } | SubmissionScope::EpisodeSet { .. }
        ),
        "pack submission uses a pack scope, got {scope:?}"
    );
}

/// A scope whose current file already meets the profile cutoff is not grabbed:
/// the cutoff early-return stays authoritative for "satisfied → skip" (§D5).
#[tokio::test]
async fn rss_does_not_grab_cutoff_met_movie() {
    let release_title = "Settled.Bay.2024.1080p.WEB-DL-GRP";
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let media_files = Arc::new(MockMediaFileRepo::default());
    let quality_profiles = Arc::new(StoredQualityProfileRepo::default());
    // A profile whose cutoff is 1080p: an existing 2160p file is at/above cutoff.
    let mut cutoff_profile = crate::default_quality_profile_for_search();
    cutoff_profile.criteria.cutoff_tier = Some("1080P".to_string());
    quality_profiles.set_profiles(vec![cutoff_profile]).await;
    let indexer_client = Arc::new(FixedReleaseIndexerClient::new(release_title));
    let (app, user) = bootstrap_rss_with_media_files_and_profiles(
        download_client,
        download_submissions.clone(),
        pending_releases,
        wanted_items.clone(),
        media_files.clone(),
        quality_profiles,
        indexer_client,
    )
    .await;

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Settled Bay".into(),
                sort_title: Some("Settled Bay".into()),
                slug: Some("settled-bay".into()),
                facet: MediaFacet::Movie,
                monitored: true,
                year: Some(2024),
                content_status: Some("Released".into()),
                min_availability: Some("announced".into()),
                ..Default::default()
            },
        )
        .await
        .expect("create monitored movie");
    wanted_items
        .remember_title_facet(&title.id, MediaFacet::Movie)
        .await;
    // Existing primary title-level file at 2160p — at/above the 1080p cutoff.
    media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            role: MediaFileRole::Primary,
            file_path: "/data/movies/settled-bay-2160p.mkv".to_string(),
            quality_label: Some("2160P".to_string()),
            acquisition_score: Some(10_000),
            ..Default::default()
        })
        .await
        .expect("insert cutoff-met file");

    let report = app.run_scheduled_rss_sync().await.expect("run RSS sync");

    assert_eq!(
        report.releases_grabbed, 0,
        "a cutoff-met scope is not grabbed"
    );
    assert!(download_submissions.store.lock().await.is_empty());
}

/// A below-cutoff scope with NO wanted row is upgraded from a matching release:
/// the derived target is missing-or-below-cutoff, and the absence of a state row
/// evaluates as `AcceptInitial` (no `current_score` baseline) so the release is
/// grabbed and the row is materialized.
#[tokio::test]
async fn rss_upgrades_below_cutoff_movie_with_no_wanted_row() {
    let release_title = "Rising.Tide.2024.2160p.WEB-DL-GRP";
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let media_files = Arc::new(MockMediaFileRepo::default());
    let quality_profiles = Arc::new(StoredQualityProfileRepo::default());
    // Cutoff at 2160p: a 720p file is below cutoff, so the scope is a target.
    let mut profile = crate::default_quality_profile_for_search();
    profile.criteria.cutoff_tier = Some("2160P".to_string());
    quality_profiles.set_profiles(vec![profile]).await;
    let indexer_client = Arc::new(FixedReleaseIndexerClient::new(release_title));
    let (app, user) = bootstrap_rss_with_media_files_and_profiles(
        download_client,
        download_submissions.clone(),
        pending_releases,
        wanted_items.clone(),
        media_files.clone(),
        quality_profiles,
        indexer_client,
    )
    .await;

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Rising Tide".into(),
                sort_title: Some("Rising Tide".into()),
                slug: Some("rising-tide".into()),
                facet: MediaFacet::Movie,
                monitored: true,
                year: Some(2024),
                content_status: Some("Released".into()),
                min_availability: Some("announced".into()),
                ..Default::default()
            },
        )
        .await
        .expect("create monitored movie");
    wanted_items
        .remember_title_facet(&title.id, MediaFacet::Movie)
        .await;
    // Existing 720p primary file — below the 2160p cutoff.
    media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            role: MediaFileRole::Primary,
            file_path: "/data/movies/rising-tide-720p.mkv".to_string(),
            quality_label: Some("720P".to_string()),
            acquisition_score: Some(100),
            ..Default::default()
        })
        .await
        .expect("insert below-cutoff file");

    assert!(
        wanted_items
            .get_acquisition_scope_state_for_title(&title.id, None)
            .await
            .expect("query wanted")
            .is_none()
    );

    let report = app.run_scheduled_rss_sync().await.expect("run RSS sync");

    assert_eq!(
        report.releases_grabbed, 1,
        "a below-cutoff scope with no row is upgraded"
    );
    let seeded = wanted_items
        .get_acquisition_scope_state_for_title(&title.id, None)
        .await
        .expect("query wanted")
        .expect("state row materialized on grab");
    assert_eq!(seeded.status, AcquisitionScopeStatus::Grabbed);
}

#[tokio::test]
async fn list_pending_releases_for_wanted_item_page_windows_and_counts() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user, _) = bootstrap_with_acquisition_tracking_and_indexer_and_release_attempts(
        download_client,
        download_submissions,
        pending_releases.clone(),
        wanted_items.clone(),
        Arc::new(MockIndexerClient),
    );
    let (title, wanted_id) =
        seed_movie_wanted_for_acquisition(&app, &user, &wanted_items, "Paged Pending Movie", 2024)
            .await;

    // Seed five waiting pending releases with descending scores so the
    // release_score-DESC page ordering is deterministic.
    for index in 0..5 {
        let mut pending = pending_movie_release(
            &wanted_id,
            &title,
            &format!("Paged.Pending.{index}.2024.1080p-GRP"),
            PendingReleaseStatus::Waiting,
        );
        pending.release_score = 1000 - index * 10;
        pending_releases
            .insert_pending_release(&pending)
            .await
            .expect("seed pending release");
    }

    // Page 2 (limit 2, offset 2) is the third and fourth highest-scored rows,
    // and the total reflects every matching row, not just the page.
    let (page, total) = app
        .list_pending_releases_for_wanted_item_page(&user, &wanted_id, 2, 2)
        .await
        .expect("page pending releases");
    assert_eq!(total, 5);
    assert_eq!(page.len(), 2);
    assert_eq!(page[0].release_score, 980);
    assert_eq!(page[1].release_score, 970);

    // The trailing page returns only the remaining row.
    let (tail, tail_total) = app
        .list_pending_releases_for_wanted_item_page(&user, &wanted_id, 2, 4)
        .await
        .expect("tail pending releases");
    assert_eq!(tail_total, 5);
    assert_eq!(tail.len(), 1);
    assert_eq!(tail[0].release_score, 960);
}

#[tokio::test]
async fn list_release_decisions_offset_paginates_within_title() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user, _) = bootstrap_with_acquisition_tracking_and_indexer_and_release_attempts(
        download_client,
        download_submissions,
        pending_releases,
        wanted_items.clone(),
        Arc::new(MockIndexerClient),
    );
    let (title, wanted_id) = seed_movie_wanted_for_acquisition(
        &app,
        &user,
        &wanted_items,
        "Paged Decisions Movie",
        2024,
    )
    .await;

    for index in 0..5 {
        wanted_items
            .insert_release_decision(&ReleaseDecision {
                id: format!("decision-page-{index}"),
                wanted_item_id: wanted_id.clone(),
                title_id: title.id.clone(),
                release_title: format!("Paged.Decision.{index}"),
                release_url: None,
                release_size_bytes: None,
                decision_code: "eligible".to_string(),
                candidate_score: 100,
                current_score: None,
                score_delta: None,
                explanation_json: None,
                created_at: Utc::now().to_rfc3339(),
            })
            .await
            .expect("seed release decision");
    }

    // limit 2, offset 2 skips the first window in storage rather than in memory.
    let (page, page_total) = app
        .list_release_decisions_page(
            &user,
            crate::ReleaseDecisionsQuery {
                wanted_item_id: None,
                title_id: Some(title.id.clone()),
                limit: 2,
                offset: 2,
            },
        )
        .await
        .expect("page release decisions");
    assert_eq!(page_total, 5);
    assert_eq!(page.len(), 2);
    assert_eq!(page[0].id, "decision-page-2");
    assert_eq!(page[1].id, "decision-page-3");

    let (tail, tail_total) = app
        .list_release_decisions_page(
            &user,
            crate::ReleaseDecisionsQuery {
                wanted_item_id: None,
                title_id: Some(title.id.clone()),
                limit: 2,
                offset: 4,
            },
        )
        .await
        .expect("tail release decisions");
    assert_eq!(tail_total, 5);
    assert_eq!(tail.len(), 1);
    assert_eq!(tail[0].id, "decision-page-4");
}

// ── Pillar A3: ambiguous-identity parking (NeedsReview) ──────────────────────

/// Returns quality-allowed releases for every search. The fixture can place an
/// unambiguous release ahead of the bare ambiguous release to cover the ordering
/// regression without changing the indexer's configured score.
struct AmbiguousIdentityIndexerClient {
    release_titles: Vec<String>,
}

#[async_trait]
impl IndexerClient for AmbiguousIdentityIndexerClient {
    async fn search(
        &self,
        _query: String,
        _ids: std::collections::HashMap<String, String>,
        _category: Option<String>,
        _facet: Option<String>,
        _id_search_facet: Option<String>,
        _newznab_categories: Option<Vec<String>>,
        _indexer_routing: Option<IndexerRoutingPlan>,
        _mode: SearchMode,
        _season: Option<u32>,
        _episode: Option<u32>,
        _absolute_episode: Option<u32>,
        _tagged_aliases: Vec<TaggedAlias>,
        _learning_context: Option<crate::IndexerSearchLearningContext>,
        _cancel_token: tokio_util::sync::CancellationToken,
    ) -> AppResult<IndexerSearchResponse> {
        Ok(IndexerSearchResponse {
            indexer_outcomes: Vec::new(),
            results: self
                .release_titles
                .iter()
                .map(|release_title| {
                    let release_slug = release_title.replace(' ', ".");
                    IndexerSearchResult {
                        indexer_id: None,
                        source: "nzbgeek".into(),
                        title: release_title.clone(),
                        link: Some(format!("https://example.invalid/info/{release_slug}")),
                        download_url: Some(format!(
                            "https://example.invalid/download/{release_slug}.nzb"
                        )),
                        source_kind: Some(DownloadSourceKind::NzbUrl),
                        size_bytes: Some(1_000_000_000),
                        published_at: Some("1970-01-01T00:00:00Z".into()),
                        thumbs_up: None,
                        thumbs_down: None,
                        indexer_languages: None,
                        indexer_subtitles: None,
                        indexer_grabs: None,
                        password_hint: None,
                        parsed_release_metadata: Some(crate::parse_release_metadata(release_title)),
                        quality_profile_decision: Some(
                            crate::quality::profile::QualityProfileDecision {
                                release_score: 100,
                                scoring_log: Vec::new(),
                                allowed: true,
                                block_codes: Vec::new(),
                                preference_score: 100,
                            },
                        ),
                        extra: Default::default(),
                        response_attributes: Default::default(),
                        guid: Some(format!("guid-{release_slug}")),
                        info_url: Some(format!("https://example.invalid/info/{release_slug}")),
                        provenance: None,
                        auto_eligible: None,
                        auto_decision_code: None,
                        auto_decision_summary: None,
                        candidate_token: None,
                        queue_scope: None,
                    }
                })
                .collect(),
            api_current: None,
            api_max: None,
            grab_current: None,
            grab_max: None,
        })
    }
}

/// Seeds the One Piece incident pair — two monitored library titles sharing the
/// canonical key `one piece` — and returns the app plus the wanted scope for the
/// live-action title.
async fn ambiguous_identity_fixture() -> (
    AppUseCase,
    User,
    scryer_domain::Title,
    String,
    Arc<TrackingPendingReleaseRepo>,
    Arc<MockReleaseAttemptRepo>,
    Arc<StubDownloadClient>,
) {
    let (app, user, title, wanted_id, pending_releases, release_attempts, download_client, _) =
        ambiguous_identity_fixture_with_releases(&["One.Piece.1080p.WEB-DL.x264-GRP"]).await;
    (
        app,
        user,
        title,
        wanted_id,
        pending_releases,
        release_attempts,
        download_client,
    )
}

async fn ambiguous_identity_fixture_with_releases(
    release_titles: &[&str],
) -> (
    AppUseCase,
    User,
    scryer_domain::Title,
    String,
    Arc<TrackingPendingReleaseRepo>,
    Arc<MockReleaseAttemptRepo>,
    Arc<StubDownloadClient>,
    Arc<TrackingAcquisitionScopeStateRepo>,
) {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let indexer_client: Arc<dyn IndexerClient> = Arc::new(AmbiguousIdentityIndexerClient {
        release_titles: release_titles
            .iter()
            .map(|title| (*title).to_string())
            .collect(),
    });
    let (app, user, release_attempts) =
        bootstrap_with_acquisition_tracking_and_indexer_and_release_attempts(
            download_client.clone(),
            download_submissions,
            pending_releases.clone(),
            wanted_items.clone(),
            indexer_client,
        );

    let (title, wanted_id) =
        seed_movie_wanted_for_acquisition(&app, &user, &wanted_items, "One Piece", 2023).await;
    // The collider: same canonical key, different work, no wanted row of its
    // own — it exists purely as library-local evidence that the name is shared.
    app.add_title(
        &user,
        NewTitle {
            name: "One Piece".into(),
            facet: MediaFacet::Movie,
            monitored: true,
            year: Some(1999),
            slug: Some("one-piece-anime".into()),
            content_status: Some("Released".to_string()),
            min_availability: Some("released".to_string()),
            ..Default::default()
        },
    )
    .await
    .expect("create colliding title");

    (
        app,
        user,
        title,
        wanted_id,
        pending_releases,
        release_attempts,
        download_client,
        wanted_items,
    )
}

#[tokio::test]
async fn convergence_cycle_parks_ambiguous_best_candidate_for_review() {
    let (app, _user, title, wanted_id, pending_releases, _release_attempts, download_client) =
        ambiguous_identity_fixture().await;

    app.run_convergence_cycle_once().await;

    let parked = pending_releases
        .list_pending_releases_for_title(&title.id)
        .await
        .expect("list pending releases for title");
    assert_eq!(
        parked.len(),
        1,
        "the best ambiguous candidate is parked once"
    );
    assert_eq!(parked[0].status, PendingReleaseStatus::NeedsReview);
    assert_eq!(parked[0].wanted_item_id, wanted_id);
    assert_eq!(parked[0].release_title, "One.Piece.1080p.WEB-DL.x264-GRP");
    assert!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .is_empty(),
        "an ambiguous candidate must never be submitted"
    );

    // Repeated cycles must not pile up review rows for the same scope.
    app.run_convergence_cycle_once().await;
    assert_eq!(
        pending_releases
            .list_pending_releases_for_title(&title.id)
            .await
            .expect("list pending releases for title")
            .len(),
        1
    );
}

#[tokio::test]
async fn convergence_cycle_parks_ambiguous_candidate_without_skipping_eligible_release() {
    let eligible = "One.Piece.2023.720p.WEB-DL.AV1.AAC2.0-NTb";
    let ambiguous = "One.Piece.1080p.WEB-DL.x264-GRP";

    for release_titles in [[eligible, ambiguous], [ambiguous, eligible]] {
        let (
            app,
            user,
            title,
            wanted_id,
            pending_releases,
            _release_attempts,
            download_client,
            wanted_items,
        ) = ambiguous_identity_fixture_with_releases(&release_titles).await;
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

        app.run_convergence_cycle_once().await;

        let parked = pending_releases
            .list_pending_releases_for_title(&title.id)
            .await
            .expect("list pending releases for title");
        assert_eq!(parked.len(), 1, "the bare release is still reviewable");
        assert_eq!(parked[0].status, PendingReleaseStatus::NeedsReview);
        assert_eq!(parked[0].wanted_item_id, wanted_id);
        assert_eq!(parked[0].release_title, ambiguous);
        let decisions = wanted_items.release_decisions.lock().await;
        let decision_codes = decisions
            .iter()
            .map(|decision| format!("{}:{}", decision.release_title, decision.decision_code))
            .collect::<Vec<_>>();
        assert_eq!(
            decision_codes,
            [format!("{eligible}:eligible")],
            "the eligible candidate must remain eligible"
        );
        drop(decisions);
        assert_eq!(
            download_client
                .submitted_release_titles
                .lock()
                .await
                .as_slice(),
            [eligible],
            "the eligible release still queues"
        );
    }
}

#[tokio::test]
async fn queue_best_release_parks_ambiguous_candidate_while_queuing_eligible_release() {
    let eligible = "One.Piece.2023.720p.WEB-DL.AV1.AAC2.0-NTb";
    let ambiguous = "One.Piece.1080p.WEB-DL.x264-GRP";

    for release_titles in [[eligible, ambiguous], [ambiguous, eligible]] {
        let (app, user, title, wanted_id, pending_releases, _release_attempts, download_client, _) =
            ambiguous_identity_fixture_with_releases(&release_titles).await;

        let outcome = app
            .queue_best_release(
                &user,
                &title.id,
                SubmissionScope::Title,
                SubmissionConflictPolicy::Abort,
            )
            .await
            .expect("queue best release");
        let QueueDownloadOutcome::Queued(queued) = outcome else {
            panic!("eligible release should queue without a conflict");
        };
        assert_eq!(queued.job_id, format!("job-for-{}", title.id));

        let parked = pending_releases
            .list_pending_releases_for_title(&title.id)
            .await
            .expect("list pending releases for title");
        assert_eq!(parked.len(), 1, "the ambiguous release is parked once");
        assert_eq!(parked[0].status, PendingReleaseStatus::NeedsReview);
        assert_eq!(parked[0].wanted_item_id, wanted_id);
        assert_eq!(parked[0].release_title, ambiguous);
        assert_eq!(
            download_client
                .submitted_release_titles
                .lock()
                .await
                .as_slice(),
            ["One Piece"],
            "the eligible candidate remains the queued release"
        );
    }
}

#[tokio::test]
async fn needs_review_pending_release_is_never_auto_promoted() {
    let (app, _user, title, _wanted_id, pending_releases, _release_attempts, download_client) =
        ambiguous_identity_fixture().await;

    app.run_convergence_cycle_once().await;
    let parked_id = pending_releases
        .list_pending_releases_for_title(&title.id)
        .await
        .expect("list pending releases for title")
        .first()
        .map(|release| release.id.clone())
        .expect("parked review row exists");

    let promoted = app
        .process_expired_pending_releases()
        .await
        .expect("process expired pending releases");

    assert_eq!(promoted, 0, "the delay processor must skip review rows");
    assert_eq!(
        pending_releases
            .get_pending_release(&parked_id)
            .await
            .expect("load parked release")
            .expect("parked release exists")
            .status,
        PendingReleaseStatus::NeedsReview
    );
    assert!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn dismissing_needs_review_pending_release_records_failed_attempt() {
    let (app, user, title, _wanted_id, pending_releases, release_attempts, _download_client) =
        ambiguous_identity_fixture().await;

    app.run_convergence_cycle_once().await;
    let parked_id = pending_releases
        .list_pending_releases_for_title(&title.id)
        .await
        .expect("list pending releases for title")
        .first()
        .map(|release| release.id.clone())
        .expect("parked review row exists");

    assert!(
        app.dismiss_pending_release(&user, &parked_id)
            .await
            .expect("dismiss parked review row")
    );

    assert_eq!(
        pending_releases
            .get_pending_release(&parked_id)
            .await
            .expect("load parked release")
            .expect("parked release exists")
            .status,
        PendingReleaseStatus::Dismissed
    );

    let failed = release_attempts
        .list_failed_release_signatures_for_title(&title.id, 10)
        .await
        .expect("list failed signatures");
    assert!(
        failed.iter().any(|attempt| {
            attempt.source_title.as_deref() == Some("One.Piece.1080p.WEB-DL.x264-GRP")
        }),
        "dismissing a review row must burn the release signature"
    );
}

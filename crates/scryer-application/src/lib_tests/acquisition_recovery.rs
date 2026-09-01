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
    let coverage = Arc::new(RecordingScopeIndexerCoverageRepo::default());
    let app = app
        .with_test_overrides(|builder| builder.with_scope_indexer_coverage_store(coverage.clone()));

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
        landed_bar: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
    };
    wanted_items
        .upsert_acquisition_scope_state(&wanted)
        .await
        .expect("seed wanted item");
    let scope_key = format!("title:{}", title.id);
    for indexer_id in ["indexer-a", "indexer-b"] {
        coverage
            .record_coverage(&scope_key, "movie", indexer_id, "fp")
            .await
            .expect("seed coverage");
    }

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
            indexer_id: None,
            release_guid: Some("guid-standby".to_string()),
            added_at: Utc::now().to_rfc3339(),
            last_observed_at: Utc::now().to_rfc3339(),
            delay_until: Utc::now().to_rfc3339(),
            status: PendingReleaseStatus::Standby,
            grabbed_at: None,
            source_password: None,
            published_at: Some(Utc::now().to_rfc3339()),
            info_hash: Some(info_hash.to_string()),
            seed_minimums: Default::default(),
            seeders: None,
            release_identity: "guid-standby".to_string(),
            coverage_identity: format!("scope:{}", wanted.id),
            role: crate::types::PendingReleaseRole::Fallback,
            last_decision_code: None,
            release_age_unknown: false,
        })
        .await
        .expect("seed standby");

    download_submissions
        .record_submission(DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "movie".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "failed-job".to_string(),
            source_hint: None,
            source_provider_id: Some("indexer-a".to_string()),
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Failed.Release.1080p.WEB-DL".to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record failed submission");

    *download_client.history_items.lock().await = vec![failed_history_item(
        "failed-job",
        "Failed.Release.1080p.WEB-DL",
    )];

    app.run_background_acquisition_cycle_once().await;

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
            .get_tracked_state(&ClientJobLocator::new(
                Some("primary"),
                "nzbget",
                "failed-job",
            ))
            .await
            .expect("load tracked state")
            .as_deref(),
        Some("failed")
    );
    let expected_signature = crate::helpers::normalize_release_selection_signature(
        Some("https://example.com/standby.torrent"),
        Some("Standby.Release.1080p.WEB-DL"),
        Some(DownloadSourceKind::TorrentFile),
    )
    .expect("standby signature");
    assert!(submissions.iter().any(|submission| {
        submission.download_client_item_id == format!("job-for-{}", title.id)
            && submission.source_title.as_deref() == Some("Standby.Release.1080p.WEB-DL")
            && submission.request_signature.as_deref() == Some(expected_signature.as_str())
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
    let mut covered = coverage.indexers_for_scope(&scope_key).await;
    covered.sort();
    assert_eq!(
        covered,
        vec!["indexer-a".to_string(), "indexer-b".to_string()],
        "a failure never touches coverage: the saved results were walked instead of re-searching"
    );
}

#[tokio::test]
async fn a_gone_standby_link_expires_and_grabs_the_next_row_in_the_same_walk() {
    let download_client = Arc::new(StubDownloadClient::default());
    download_client
        .set_submit_errors([StubSubmitError::SourceGone("HTTP 404: gone".to_string())])
        .await;
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases.clone(),
        wanted_items.clone(),
    );
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Gone Standby Link".into(),
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
        title_slug: title.slug.clone(),
        title_facet: Some("movie".to_string()),
        library_id: Some(title.library_id.clone()),
        library_name: None,
        library_slug: None,
        episode_id: None,
        collection_id: None,
        series_movie_link_id: None,
        season_number: None,
        episode_number: None,
        media_type: "movie".to_string(),
        last_search_at: Some((Utc::now() - chrono::Duration::minutes(5)).to_rfc3339()),
        status: AcquisitionScopeStatus::Wanted,
        grabbed_release: None,
        landed_bar: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
    };
    wanted_items
        .upsert_acquisition_scope_state(&wanted)
        .await
        .expect("seed wanted scope");
    let standby = |title_suffix: &str, score| PendingRelease {
        id: Id::new().0,
        wanted_item_id: wanted.id.clone(),
        title_id: title.id.clone(),
        release_title: format!("Gone.Standby.Link.2024.1080p.WEB-DL-{title_suffix}"),
        release_url: Some(format!("https://example.invalid/{title_suffix}.nzb")),
        source_kind: Some(DownloadSourceKind::NzbUrl),
        release_size_bytes: None,
        release_score: score,
        scoring_log_json: None,
        indexer_source: Some("fixture-indexer".to_string()),
        indexer_id: Some("fixture-indexer".to_string()),
        release_guid: Some(format!("guid-{title_suffix}")),
        added_at: Utc::now().to_rfc3339(),
        last_observed_at: Utc::now().to_rfc3339(),
        delay_until: Utc::now().to_rfc3339(),
        status: PendingReleaseStatus::Standby,
        grabbed_at: None,
        source_password: None,
        published_at: None,
        info_hash: None,
        seed_minimums: Default::default(),
        seeders: None,
        release_identity: format!("guid-{title_suffix}"),
        coverage_identity: format!("scope:{}", wanted.id),
        role: crate::types::PendingReleaseRole::Fallback,
        last_decision_code: None,
        release_age_unknown: false,
    };
    let gone = standby("GONE", 200);
    let usable = standby("USABLE", 100);
    pending_releases
        .insert_pending_release(&gone)
        .await
        .expect("seed gone row");
    pending_releases
        .insert_pending_release(&usable)
        .await
        .expect("seed usable row");

    app.run_background_acquisition_cycle_once().await;

    let rows = pending_releases.store.lock().await.clone();
    assert_eq!(
        rows.iter()
            .find(|row| row.id == gone.id)
            .expect("gone row exists")
            .status,
        PendingReleaseStatus::Expired
    );
    assert_eq!(
        rows.iter()
            .find(|row| row.id == usable.id)
            .expect("usable row exists")
            .status,
        PendingReleaseStatus::Grabbed
    );
    assert_eq!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .clone(),
        vec![gone.release_title.clone(), usable.release_title.clone()],
        "the row after a gone artifact must be attempted in the same walk"
    );
    assert!(
        app.services
            .workflow
            .blocklist_repo
            .list_for_title(&title.id, 10)
            .await
            .expect("list blocklist")
            .is_empty(),
        "a source-gone row is expired, never blocklisted"
    );
    assert!(
        app.services
            .workflow
            .release_attempts
            .list_failed_release_signatures_for_title(&title.id, 10)
            .await
            .expect("list failed attempts")
            .is_empty(),
        "a source-gone row records no failed attempt"
    );
}

#[tokio::test]
async fn standby_delay_parks_the_best_row_stops_the_walk_and_promotion_grabs_when_due() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases.clone(),
        wanted_items.clone(),
    );
    let (title, wanted_id) =
        seed_movie_wanted_for_acquisition(&app, &user, &wanted_items, "Standby Delay", 2024).await;
    app.services
        .config
        .settings
        .upsert_setting_json(
            SETTINGS_SCOPE_SYSTEM,
            DELAY_PROFILE_CATALOG_KEY,
            None,
            serde_json::json!([{
                "id": "standby-delay",
                "name": "Standby delay",
                "usenet_delay_minutes": 10,
            }])
            .to_string(),
            "test",
            None,
        )
        .await
        .expect("seed delay profile");
    let now = Utc::now();
    let mut best = pending_movie_release(
        &wanted_id,
        &title,
        "Standby.Delay.Best.2024.1080p.WEB-DL-GRP",
        PendingReleaseStatus::Standby,
    );
    best.added_at = now.to_rfc3339();
    best.published_at = Some(now.to_rfc3339());
    let mut worse = pending_movie_release(
        &wanted_id,
        &title,
        "Standby.Delay.Worse.2024.720p.WEB-DL-GRP",
        PendingReleaseStatus::Standby,
    );
    worse.release_score = best.release_score - 1;
    worse.added_at = now.to_rfc3339();
    worse.published_at = Some(now.to_rfc3339());
    pending_releases
        .insert_pending_release(&best)
        .await
        .expect("seed best standby row");
    pending_releases
        .insert_pending_release(&worse)
        .await
        .expect("seed worse standby row");
    let wanted = wanted_items
        .get_acquisition_scope_state_by_id(&wanted_id)
        .await
        .expect("load wanted scope")
        .expect("wanted scope exists");
    let snapshot = crate::acquisition_workflow::DownloadClientSnapshot::fetch(&app).await;

    assert_eq!(
        crate::acquisition_workflow::try_saved_candidates(
            &app, &wanted, None, None, &snapshot, &now,
        )
        .await,
        crate::acquisition_workflow::StandbyRecoveryOutcome::Parked {
            scope: Some(SubmissionScope::Title)
        }
    );
    let parked = pending_releases
        .get_pending_release(&best.id)
        .await
        .expect("load best row")
        .expect("best row exists");
    assert_eq!(parked.status, PendingReleaseStatus::Waiting);
    let delay_until = crate::quality_profile::parse_published_at(&parked.delay_until)
        .expect("parked delay timestamp");
    assert_eq!(delay_until, now + chrono::Duration::minutes(10));
    assert_eq!(
        pending_releases
            .get_pending_release(&worse.id)
            .await
            .expect("load worse row")
            .expect("worse row exists")
            .status,
        PendingReleaseStatus::Standby,
        "a held better row must stop the standby walk"
    );
    assert!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .is_empty(),
        "a delayed head must not submit itself or anything worse"
    );

    assert_eq!(
        crate::acquisition_workflow::try_saved_candidates(
            &app,
            &wanted,
            None,
            None,
            &snapshot,
            &(now + chrono::Duration::minutes(1)),
        )
        .await,
        crate::acquisition_workflow::StandbyRecoveryOutcome::Parked { scope: None },
        "the waiting head owns subsequent standby cycles"
    );
    assert!(
        matches!(
            app.try_grab_pending_release(
                &wanted,
                &parked,
                &(now + chrono::Duration::minutes(11)),
                crate::acquisition::pending::PendingGrabTrigger::Automatic,
            )
            .await
            .expect("promotion should resolve"),
            crate::acquisition::pending::PendingGrabOutcome::Grabbed {
                scope: SubmissionScope::Title
            }
        ),
        "the unchanged profile must allow promotion after delay_until"
    );
}

#[tokio::test]
async fn waiting_promotion_reparks_when_the_delay_profile_grows() {
    let download_client = Arc::new(StubDownloadClient::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking(
        download_client.clone(),
        Arc::new(TrackingDownloadSubmissionRepo::default()),
        pending_releases.clone(),
        wanted_items.clone(),
    );
    let (title, wanted_id) =
        seed_movie_wanted_for_acquisition(&app, &user, &wanted_items, "Growing Delay", 2024).await;
    let now = Utc::now();
    let mut waiting = pending_movie_release(
        &wanted_id,
        &title,
        "Growing.Delay.2024.1080p.WEB-DL-GRP",
        PendingReleaseStatus::Waiting,
    );
    waiting.added_at = now.to_rfc3339();
    waiting.published_at = Some(now.to_rfc3339());
    waiting.delay_until = (now + chrono::Duration::minutes(10)).to_rfc3339();
    pending_releases
        .insert_pending_release(&waiting)
        .await
        .expect("seed waiting release");
    app.services
        .config
        .settings
        .upsert_setting_json(
            SETTINGS_SCOPE_SYSTEM,
            DELAY_PROFILE_CATALOG_KEY,
            None,
            serde_json::json!([{
                "id": "grown-delay",
                "name": "Grown delay",
                "usenet_delay_minutes": 30,
            }])
            .to_string(),
            "test",
            None,
        )
        .await
        .expect("lengthen delay profile");
    let wanted = wanted_items
        .get_acquisition_scope_state_by_id(&wanted_id)
        .await
        .expect("load wanted")
        .expect("wanted exists");
    let promotion_time = now + chrono::Duration::minutes(11);

    assert_eq!(
        app.try_grab_pending_release(
            &wanted,
            &waiting,
            &promotion_time,
            crate::acquisition::pending::PendingGrabTrigger::Automatic,
        )
        .await
        .expect("promotion should resolve"),
        crate::acquisition::pending::PendingGrabOutcome::Parked
    );
    let reparking = pending_releases
        .get_pending_release(&waiting.id)
        .await
        .expect("load waiting row")
        .expect("waiting row exists");
    assert_eq!(reparking.status, PendingReleaseStatus::Waiting);
    assert_eq!(
        crate::quality_profile::parse_published_at(&reparking.delay_until)
            .expect("reparked delay timestamp"),
        now + chrono::Duration::minutes(30),
        "the longer profile extends only by the remaining delay"
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
async fn operator_pending_grab_ignores_the_delay_profile() {
    let download_client = Arc::new(StubDownloadClient::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking(
        download_client.clone(),
        Arc::new(TrackingDownloadSubmissionRepo::default()),
        pending_releases.clone(),
        wanted_items.clone(),
    );
    let (title, wanted_id) =
        seed_movie_wanted_for_acquisition(&app, &user, &wanted_items, "Operator Delay", 2024).await;
    app.services
        .config
        .settings
        .upsert_setting_json(
            SETTINGS_SCOPE_SYSTEM,
            DELAY_PROFILE_CATALOG_KEY,
            None,
            serde_json::json!([{
                "id": "operator-delay",
                "name": "Operator delay",
                "usenet_delay_minutes": 120,
            }])
            .to_string(),
            "test",
            None,
        )
        .await
        .expect("seed delay profile");
    let pending = pending_movie_release(
        &wanted_id,
        &title,
        "Operator.Delay.2024.1080p.WEB-DL-GRP",
        PendingReleaseStatus::Waiting,
    );
    pending_releases
        .insert_pending_release(&pending)
        .await
        .expect("seed pending release");
    let wanted = wanted_items
        .get_acquisition_scope_state_by_id(&wanted_id)
        .await
        .expect("load wanted")
        .expect("wanted exists");

    assert!(matches!(
        app.try_grab_pending_release(
            &wanted,
            &pending,
            &Utc::now(),
            crate::acquisition::pending::PendingGrabTrigger::Operator,
        )
        .await
        .expect("operator grab should resolve"),
        crate::acquisition::pending::PendingGrabOutcome::Grabbed {
            scope: SubmissionScope::Title
        }
    ));
    assert_eq!(
        pending_releases
            .get_pending_release(&pending.id)
            .await
            .expect("load pending row")
            .expect("pending row exists")
            .status,
        PendingReleaseStatus::Grabbed
    );
}

#[tokio::test]
async fn acquisition_failure_fallback_skips_failed_submission_for_another_episode() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
        wanted_items.clone(),
    );
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Scoped Failure Recovery".into(),
                facet: MediaFacet::Series,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .expect("create series title");
    let current_episode_id = "episode-12";
    let current_release = "Scoped.Failure.Recovery.S02E12.1080p.WEB-DL";
    let wanted = AcquisitionScopeState {
        id: Id::new().0,
        title_id: title.id.clone(),
        title_name: Some(title.name.clone()),
        title_slug: None,
        title_facet: Some("series".to_string()),
        library_id: None,
        library_name: None,
        library_slug: None,
        episode_id: Some(current_episode_id.to_string()),
        collection_id: None,
        series_movie_link_id: None,
        season_number: Some("2".to_string()),
        episode_number: Some("12".to_string()),
        media_type: "episode".to_string(),
        last_search_at: Some((Utc::now() - chrono::Duration::minutes(5)).to_rfc3339()),
        status: AcquisitionScopeStatus::Grabbed,
        grabbed_release: Some(
            serde_json::json!({
                "title": current_release,
                "score": 100,
                "grabbed_at": Utc::now().to_rfc3339(),
            })
            .to_string(),
        ),
        landed_bar: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
    };
    wanted_items
        .upsert_acquisition_scope_state(&wanted)
        .await
        .expect("seed grabbed episode");

    for (client_item_id, source_title, episode_id) in [
        (
            "old-failed-job",
            "Scoped.Failure.Recovery.S02E01.1080p.WEB-DL",
            "episode-1",
        ),
        ("current-failed-job", current_release, current_episode_id),
    ] {
        download_submissions
            .record_submission(DownloadSubmission {
                download_id: scryer_domain::download_identity::DownloadId::new(),
                title_id: title.id.clone(),
                purpose: crate::DownloadSubmissionPurpose::Standard,
                facet: "series".to_string(),
                download_client_id: Some("primary".to_string()),
                download_client_type: "nzbget".to_string(),
                download_client_item_id: client_item_id.to_string(),
                source_hint: None,
                source_provider_id: None,
                source_provider_name: None,
                source_kind: None,
                source_title: Some(source_title.to_string()),
                info_hash: None,
                release_size_bytes: None,
                request_signature: None,
                scope: SubmissionScope::Episode {
                    episode_id: episode_id.to_string(),
                },
            })
            .await
            .expect("record episode submission");
    }
    let mut old_failure = failed_history_item(
        "old-failed-job",
        "Scoped.Failure.Recovery.S02E01.1080p.WEB-DL",
    );
    old_failure.facet = Some("series".to_string());
    let mut current_failure = failed_history_item("current-failed-job", current_release);
    current_failure.facet = Some("series".to_string());
    *download_client.history_items.lock().await = vec![old_failure, current_failure];

    let old_outcome = crate::acquisition_workflow::process_download_failure(
        &app,
        crate::acquisition_workflow::DownloadFailureContext {
            wanted_item: None,
            title_id: Some(title.id.clone()),
            client_id: "primary".to_string(),
            client_type: "nzbget".to_string(),
            client_name: Some("Primary".to_string()),
            client_item_id: "old-failed-job".to_string(),
            release_title: "Scoped.Failure.Recovery.S02E01.1080p.WEB-DL".to_string(),
            reason: "old download failed".to_string(),
            remove_from_client_if_configured: false,
            skip_reacquire: true,
        },
    )
    .await;
    assert_eq!(
        old_outcome,
        crate::acquisition_workflow::FailureHandlingOutcome::RecordedNoReacquire
    );
    wanted_items
        .upsert_acquisition_scope_state(&wanted)
        .await
        .expect("restore current episode as grabbed after seeding old failure history");

    app.run_background_acquisition_cycle_once().await;

    assert_eq!(
        download_submissions
            .get_tracked_state(&ClientJobLocator::new(
                Some("primary"),
                "nzbget",
                "current-failed-job",
            ))
            .await
            .expect("load current failure state")
            .as_deref(),
        Some("failed")
    );
    assert_eq!(
        download_submissions
            .get_tracked_state(&ClientJobLocator::new(
                Some("primary"),
                "nzbget",
                "old-failed-job",
            ))
            .await
            .expect("load old failure state")
            .as_deref(),
        Some("failed")
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
        landed_bar: None,
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
            indexer_id: None,
            release_guid: Some("guid-standby".to_string()),
            added_at: Utc::now().to_rfc3339(),
            last_observed_at: Utc::now().to_rfc3339(),
            delay_until: Utc::now().to_rfc3339(),
            status: PendingReleaseStatus::Standby,
            grabbed_at: None,
            source_password: None,
            published_at: Some(Utc::now().to_rfc3339()),
            info_hash: None,
            seed_minimums: Default::default(),
            seeders: None,
            release_identity: "guid-standby".to_string(),
            coverage_identity: format!("scope:{}", wanted.id),
            role: crate::types::PendingReleaseRole::Fallback,
            last_decision_code: None,
            release_age_unknown: false,
        })
        .await
        .expect("seed standby");

    download_submissions
        .record_submission(DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
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
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record failed submission");

    let mut tracked_download = crate::tracked_downloads::TrackedDownload {
        download_id: scryer_domain::download_identity::DownloadId::new(),
        id: "nzbget:failed-job".to_string(),
        client_id: "primary".to_string(),
        client_type: "nzbget".to_string(),
        client_item: failed_history_item("failed-job", "Failed.Release.1080p.WEB-DL"),
        completed_source: None,
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
        import_execution_retry: None,
        import_hold: None,
        skip_reacquire_on_failure: false,
        burned_by_import_gate: false,
        snapshot_missing_since: None,
    };

    crate::failed_download_handler::process_failed(&app, &mut tracked_download).await;

    assert_eq!(
        tracked_download.state,
        scryer_domain::TrackedDownloadState::Failed
    );

    // The failure itself only blocklists and re-opens: the saved result is
    // still there, untouched, and the scope is `wanted` under its coverage.
    let reopened = wanted_items
        .get_acquisition_scope_state_by_id(&wanted.id)
        .await
        .expect("load wanted")
        .expect("wanted exists");
    assert_eq!(reopened.status, AcquisitionScopeStatus::Wanted);
    assert_eq!(
        pending_releases
            .list_all_standby_pending_releases()
            .await
            .expect("list standby")
            .len(),
        1
    );

    // The next cursor pass walks the saved results before any indexer query.
    download_client
        .set_snapshot_authoritative_client_ids(["primary".to_string()])
        .await;
    app.run_background_acquisition_cycle_once().await;

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
            .get_tracked_state(&ClientJobLocator::new(
                Some("primary"),
                "nzbget",
                "failed-job",
            ))
            .await
            .expect("load tracked state")
            .as_deref(),
        Some("failed")
    );
    let expected_signature = crate::helpers::normalize_release_selection_signature(
        Some("https://example.com/standby.nzb"),
        Some("Standby.Release.1080p.WEB-DL"),
        Some(DownloadSourceKind::NzbUrl),
    )
    .expect("standby signature");
    assert!(submissions.iter().any(|submission| {
        submission.download_client_item_id == format!("job-for-{}", title.id)
            && submission.source_title.as_deref() == Some("Standby.Release.1080p.WEB-DL")
            && submission.request_signature.as_deref() == Some(expected_signature.as_str())
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
        landed_bar: None,
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
            download_id: scryer_domain::download_identity::DownloadId::new(),
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
            info_hash: None,
            release_size_bytes: None,
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
    )
    .await;

    assert_eq!(
        outcome,
        crate::acquisition_workflow::FailureHandlingOutcome::Reopened
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
    // The failure re-opens the scope under its coverage and leaves the saved
    // result for the cursor; with the download client unavailable the cursor
    // keeps it pending rather than expiring it.
    assert_eq!(updated_wanted.status, AcquisitionScopeStatus::Wanted);
    app.run_background_acquisition_cycle_once().await;
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
    assert_eq!(updated_wanted.status, AcquisitionScopeStatus::Wanted);
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
    let coverage = Arc::new(RecordingScopeIndexerCoverageRepo::default());
    let app = app
        .with_test_overrides(|builder| builder.with_scope_indexer_coverage_store(coverage.clone()));

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
        landed_bar: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
    };
    wanted_items
        .upsert_acquisition_scope_state(&wanted)
        .await
        .expect("seed wanted item");
    let scope_key = format!("title:{}", title.id);
    for indexer_id in ["indexer-a", "indexer-b"] {
        coverage
            .record_coverage(&scope_key, "movie", indexer_id, "fp")
            .await
            .expect("seed coverage");
    }
    let wanted_id = wanted.id.clone();

    download_submissions
        .record_submission(DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "movie".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "failed-duplicate".to_string(),
            source_hint: None,
            source_provider_id: Some("indexer-a".to_string()),
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Duplicate.Failed.Release.1080p.WEB-DL".to_string()),
            info_hash: None,
            release_size_bytes: None,
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
    )
    .await;
    assert_eq!(
        first,
        crate::acquisition_workflow::FailureHandlingOutcome::Reopened
    );
    let reopened = wanted_items
        .get_acquisition_scope_state_by_id(&wanted_id)
        .await
        .expect("get reopened wanted item")
        .expect("reopened wanted item exists");
    assert_eq!(reopened.status, AcquisitionScopeStatus::Wanted);
    let mut covered = coverage.indexers_for_scope(&scope_key).await;
    covered.sort();
    assert_eq!(
        covered,
        vec!["indexer-a".to_string(), "indexer-b".to_string()],
        "a failure never touches coverage"
    );
    let blocklist_before_duplicate = app
        .services
        .workflow
        .blocklist_repo
        .list_for_title(&title.id, 10)
        .await
        .expect("list blocklist before duplicate handling");
    assert_eq!(blocklist_before_duplicate.len(), 1);

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
    assert_eq!(
        blocklist.len(),
        1,
        "the duplicate failure recorded no second row"
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
async fn operator_client_failure_is_recorded_without_reopening_scope() {
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
    let coverage = Arc::new(RecordingScopeIndexerCoverageRepo::default());
    let app = app
        .with_test_overrides(|builder| builder.with_scope_indexer_coverage_store(coverage.clone()));

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
        landed_bar: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
    };
    wanted_items
        .upsert_acquisition_scope_state(&wanted)
        .await
        .expect("seed wanted item");
    let scope_key = format!("title:{}", title.id);
    for indexer_id in ["indexer-a", "indexer-b"] {
        coverage
            .record_coverage(&scope_key, "movie", indexer_id, "fp")
            .await
            .expect("seed coverage");
    }

    download_submissions
        .record_submission(DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::OperatorQueued,
            facet: "movie".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "failed-only".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Manual.Failed.Only.1080p.WEB-DL".to_string()),
            info_hash: None,
            release_size_bytes: None,
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
            skip_reacquire: false,
        },
    )
    .await;

    assert_eq!(
        outcome,
        crate::acquisition_workflow::FailureHandlingOutcome::RecordedOnly
    );

    let updated_wanted = wanted_items
        .get_acquisition_scope_state_by_id(&wanted.id)
        .await
        .expect("get wanted")
        .expect("wanted item");
    assert_eq!(updated_wanted.status, AcquisitionScopeStatus::Grabbed);
    assert!(updated_wanted.grabbed_release.is_some());

    let blocklist = app
        .services
        .workflow
        .blocklist_repo
        .list_for_title(&title.id, 10)
        .await
        .expect("list blocklist");
    assert_eq!(blocklist.len(), 1);
    assert_eq!(
        coverage.indexers_for_scope(&scope_key).await,
        vec!["indexer-a".to_string(), "indexer-b".to_string()],
        "an operator failure preserves coverage and never schedules automatic recovery"
    );
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
                name: "Pals".into(),
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
            "Pals.S05.720p.BluRay.DD5.1.x264-NTb",
        ),
        (
            "weaver-2",
            "weaver://job/weaver-2",
            " pals.s05.720p.bluray.dd5.1.x264-ntb ",
        ),
    ] {
        download_submissions
            .record_submission(DownloadSubmission {
                download_id: scryer_domain::download_identity::DownloadId::new(),
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
                info_hash: None,
                release_size_bytes: None,
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
            release_title: "Pals".to_string(),
            reason: "download failed".to_string(),
            remove_from_client_if_configured: false,
            skip_reacquire: false,
        },
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
            release_title: "Pals".to_string(),
            reason: "download failed".to_string(),
            remove_from_client_if_configured: false,
            skip_reacquire: false,
        },
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
        blocklist[0].normalized_release_name,
        "pals.s05.720p.bluray.dd5.1.x264-ntb"
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
                name: "Pals".into(),
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
            download_id: scryer_domain::download_identity::DownloadId::new(),
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
            source_title: Some("Pals.S05.720p.BluRay.DD5.1.x264-NTb".to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record failed submission");

    let mut tracked_download = crate::tracked_downloads::TrackedDownload {
        download_id: scryer_domain::download_identity::DownloadId::new(),
        id: "weaver:job-1".to_string(),
        client_id: "primary".to_string(),
        client_type: "weaver".to_string(),
        client_item: failed_history_item("job-1", "Pals"),
        completed_source: None,
        state: scryer_domain::TrackedDownloadState::FailedPending,
        status: scryer_domain::TrackedDownloadStatus::Error,
        status_messages: Vec::new(),
        title_id: Some(title.id.clone()),
        facet: Some("series".to_string()),
        source_title: Some("Pals.S05.720p.BluRay.DD5.1.x264-NTb".to_string()),
        indexer: None,
        added_at: None,
        notified_manual_interaction: false,
        match_type: scryer_domain::TitleMatchType::Submission,
        is_trackable: true,
        import_attempted: false,
        waiting_for_completed_history: false,
        path_missing_since: None,
        no_video_import_retry: None,
        import_execution_retry: None,
        import_hold: None,
        skip_reacquire_on_failure: false,
        burned_by_import_gate: false,
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
        blocklist[0].normalized_release_name,
        "pals.s05.720p.bluray.dd5.1.x264-ntb"
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
        Some("pals.s05.720p.bluray.dd5.1.x264-ntb")
    );
}

#[tokio::test]
async fn parse_matched_observed_failed_download_does_not_blocklist_or_requeue() {
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
                name: "Observed Failure Safety".into(),
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
        landed_bar: None,
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
        "observed-failed-job",
        "Observed.Failure.Safety.2024.1080p.WEB-DL",
    );
    client_item.is_scryer_origin = false;
    let mut tracked_download = crate::tracked_downloads::TrackedDownload {
        download_id: scryer_domain::download_identity::DownloadId::new(),
        id: "nzbget:observed-failed-job".to_string(),
        client_id: "primary".to_string(),
        client_type: "nzbget".to_string(),
        client_item,
        completed_source: None,
        state: scryer_domain::TrackedDownloadState::FailedPending,
        status: scryer_domain::TrackedDownloadStatus::Error,
        status_messages: Vec::new(),
        title_id: Some(title.id.clone()),
        facet: Some("movie".to_string()),
        source_title: Some("Observed.Failure.Safety.2024.1080p.WEB-DL".to_string()),
        indexer: None,
        added_at: None,
        notified_manual_interaction: false,
        match_type: scryer_domain::TitleMatchType::TitleParse,
        is_trackable: true,
        import_attempted: false,
        waiting_for_completed_history: false,
        path_missing_since: None,
        no_video_import_retry: None,
        import_execution_retry: None,
        import_hold: None,
        skip_reacquire_on_failure: true,
        burned_by_import_gate: false,
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
    let coverage = Arc::new(RecordingScopeIndexerCoverageRepo::default());
    let app = app
        .with_test_overrides(|builder| builder.with_scope_indexer_coverage_store(coverage.clone()));

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
    let mut expected_episode_ids = Vec::new();
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
            collection_id: Some(season.id.clone()),
            series_movie_link_id: None,
            season_number: Some("7".to_string()),
            episode_number: None,
            media_type: "episode".to_string(),
            last_search_at: Some((Utc::now() - chrono::Duration::minutes(30)).to_rfc3339()),
            status: AcquisitionScopeStatus::Wanted,
            grabbed_release: None,
            landed_bar: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        };
        expected_wanted_ids.push(wanted.id.clone());
        expected_episode_ids.push(episode.id.clone());
        wanted_items
            .upsert_acquisition_scope_state(&wanted)
            .await
            .expect("seed episode wanted item");
    }
    let coverage_scope_keys: Vec<String> = expected_episode_ids
        .iter()
        .map(|episode_id| format!("episode:{episode_id}"))
        .chain(std::iter::once(format!("collection:{}", season.id)))
        .collect();
    for scope_key in &coverage_scope_keys {
        for indexer_id in ["indexer-a", "indexer-b"] {
            coverage
                .record_coverage(scope_key, "anime", indexer_id, "fp")
                .await
                .expect("seed coverage");
        }
    }

    download_submissions
        .record_submission(DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "anime".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "failed-season-pack".to_string(),
            source_hint: None,
            source_provider_id: Some("indexer-a".to_string()),
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Season.Pack.Failure.Recovery.S07.1080p.WEB-DL".to_string()),
            info_hash: None,
            release_size_bytes: None,
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
    )
    .await;
    assert_eq!(
        first,
        crate::acquisition_workflow::FailureHandlingOutcome::Reopened
    );
    for scope_key in &coverage_scope_keys {
        let mut covered = coverage.indexers_for_scope(scope_key).await;
        covered.sort();
        assert_eq!(
            covered,
            vec!["indexer-a".to_string(), "indexer-b".to_string()],
            "a failed pack never touches coverage for {scope_key}"
        );
    }

    let mut tracked_download = crate::tracked_downloads::TrackedDownload {
        download_id: scryer_domain::download_identity::DownloadId::new(),
        id: "nzbget:failed-season-pack".to_string(),
        client_id: "primary".to_string(),
        client_type: "nzbget".to_string(),
        client_item: failed_history_item(
            "failed-season-pack",
            "Season.Pack.Failure.Recovery.S07.1080p.WEB-DL",
        ),
        completed_source: None,
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
        import_execution_retry: None,
        import_hold: None,
        skip_reacquire_on_failure: false,
        burned_by_import_gate: false,
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
                .is_some_and(|reason| reason.contains("re-opened covered episodes"))
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
            .get_tracked_state(&ClientJobLocator::new(
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
async fn episode_set_pack_failure_reopens_only_its_covered_wanted_items() {
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
                name: "Episode Set Failure Recovery".into(),
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

    let covered = (1..=501)
        .map(|number| {
            (
                format!("covered-wanted-{number}"),
                format!("covered-episode-{number}"),
            )
        })
        .collect::<Vec<_>>();
    let unrelated = (
        "unrelated-wanted".to_string(),
        "unrelated-episode".to_string(),
    );
    for (wanted_id, episode_id) in covered.iter().chain(std::iter::once(&unrelated)) {
        wanted_items
            .upsert_acquisition_scope_state(&AcquisitionScopeState {
                id: wanted_id.clone(),
                title_id: title.id.clone(),
                title_name: Some(title.name.clone()),
                title_slug: None,
                title_facet: None,
                library_id: None,
                library_name: None,
                library_slug: None,
                episode_id: Some(episode_id.clone()),
                collection_id: None,
                series_movie_link_id: None,
                season_number: Some("1".to_string()),
                episode_number: None,
                media_type: "episode".to_string(),
                last_search_at: Some((Utc::now() - chrono::Duration::minutes(5)).to_rfc3339()),
                status: AcquisitionScopeStatus::Grabbed,
                grabbed_release: Some(
                    serde_json::json!({
                        "title": "Episode.Set.Pack.Failure.S01.1080p.WEB-DL",
                        "score": 100,
                        "grabbed_at": Utc::now().to_rfc3339(),
                    })
                    .to_string(),
                ),
                landed_bar: None,
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
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "anime".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "failed-episode-set-pack".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Episode.Set.Pack.Failure.S01.1080p.WEB-DL".to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: SubmissionScope::EpisodeSet {
                episode_ids: covered
                    .iter()
                    .map(|(_, episode_id)| episode_id.clone())
                    .collect(),
            },
        })
        .await
        .expect("record failed episode-set submission");

    let outcome = crate::acquisition_workflow::process_download_failure(
        &app,
        crate::acquisition_workflow::DownloadFailureContext {
            wanted_item: None,
            title_id: Some(title.id.clone()),
            client_id: "primary".to_string(),
            client_type: "nzbget".to_string(),
            client_name: Some("Primary".to_string()),
            client_item_id: "failed-episode-set-pack".to_string(),
            release_title: "Episode.Set.Pack.Failure.S01.1080p.WEB-DL".to_string(),
            reason: "download failed".to_string(),
            remove_from_client_if_configured: false,
            skip_reacquire: false,
        },
    )
    .await;

    assert_eq!(
        outcome,
        crate::acquisition_workflow::FailureHandlingOutcome::Reopened
    );
    for (wanted_id, _) in &covered {
        let wanted = wanted_items
            .get_acquisition_scope_state_by_id(wanted_id)
            .await
            .expect("get covered wanted item")
            .expect("covered wanted item exists");
        assert_eq!(wanted.status, AcquisitionScopeStatus::Wanted);
        assert!(wanted.grabbed_release.is_none());
    }

    let unaffected = wanted_items
        .get_acquisition_scope_state_by_id(&unrelated.0)
        .await
        .expect("get unrelated wanted item")
        .expect("unrelated wanted item exists");
    assert_eq!(unaffected.status, AcquisitionScopeStatus::Grabbed);
    assert!(unaffected.grabbed_release.is_some());
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
                landed_bar: None,
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
            download_id: scryer_domain::download_identity::DownloadId::new(),
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
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record shared submission");

    app.run_background_acquisition_cycle_once().await;

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
                landed_bar: None,
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
            download_id: scryer_domain::download_identity::DownloadId::new(),
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
            info_hash: None,
            release_size_bytes: None,
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

    app.run_background_acquisition_cycle_once().await;

    let searches = indexer_client.searches.lock().await.clone();
    assert!(!searches.is_empty());
    assert!(searches.iter().all(|search| {
        (search.season == Some(1) && search.episode.is_some())
            || (search.season.is_none() && search.episode.is_none())
    }));
    assert_eq!(
        searches
            .iter()
            .filter(|search| search.season.is_none() && search.episode.is_none())
            .count(),
        1,
        "the show receives one title-level series-pack lookup"
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
    assert!(blocklist[0].release_name.eq_ignore_ascii_case(pack_title));
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
            .get_tracked_state(&ClientJobLocator::new(
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
                landed_bar: None,
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
            download_id: scryer_domain::download_identity::DownloadId::new(),
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
            source_title: Some("Episode.Blocking.Scope.S01E01.1080p.WEB-DL".to_string()),
            info_hash: None,
            release_size_bytes: None,
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
        seeding: None,
    }];

    app.run_background_acquisition_cycle_once().await;

    // An active initial acquisition owns its empty episode scope. The sibling
    // episode remains independent and may still be searched.
    let searches = indexer_client.searches.lock().await.clone();
    assert!(
        !searches
            .iter()
            .any(|search| search.season == Some(1) && search.episode == Some(1)),
        "the scope with a download in flight was searched again: {searches:?}"
    );
    assert!(
        searches
            .iter()
            .any(|search| search.season == Some(2) && search.episode == Some(1)),
        "and its sibling is unaffected: {searches:?}"
    );

    assert!(
        !download_submissions
            .store
            .lock()
            .await
            .iter()
            .any(
                |submission| submission.download_client_item_id != "episode-one-active"
                    && matches!(
                        &submission.scope,
                        SubmissionScope::Episode { episode_id } if *episode_id == episode_one.id
                    )
            ),
        "an equal release must not be grabbed beside the one already downloading"
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
                landed_bar: None,
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
            download_id: scryer_domain::download_identity::DownloadId::new(),
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
            source_title: Some("Season.Pack.Blocking.Scope.S01.1080p.WEB-DL".to_string()),
            info_hash: None,
            release_size_bytes: None,
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
        seeding: None,
    }];

    app.run_background_acquisition_cycle_once().await;

    // An active initial season pack owns every empty scope in its season. The
    // neighbouring season remains independent and may still be searched.
    let searches = indexer_client.searches.lock().await.clone();
    assert!(
        !searches.iter().any(|search| search.season == Some(1)),
        "the season with a pack in flight was searched again: {searches:?}"
    );
    assert!(
        searches
            .iter()
            .any(|search| search.season == Some(2) && search.episode == Some(1)),
        "and the neighbouring season is unaffected: {searches:?}"
    );
}

#[tokio::test]
async fn acquisition_cycle_submits_one_hundred_episode_fallbacks_after_empty_pack_pass() {
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
            _operation: IndexerErrorOperation,
            season: Option<u32>,
            episode: Option<u32>,
            _absolute_episode: Option<u32>,
            _year: Option<i32>,
            _tagged_aliases: Vec<TaggedAlias>,
            _learning_context: Option<crate::IndexerSearchLearningContext>,
            _cancel_token: tokio_util::sync::CancellationToken,
        ) -> AppResult<IndexerSearchResponse> {
            self.searches.lock().await.push(RecordedIndexerSearch {
                query: query.clone(),
                season,
                episode,
            });
            if episode.is_none() {
                return Ok(IndexerSearchResponse {
                    completion: crate::IndexerSearchCompletion::Complete,

                    indexer_outcomes: Vec::new(),
                    results: Vec::new(),
                    api_current: None,
                    api_max: None,
                    grab_current: None,
                    grab_max: None,
                });
            }

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
                completion: crate::IndexerSearchCompletion::Complete,

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
                            tier_index: Some(0),
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
                    coverage_scope: None,
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
            last_episode_number: Some("100".to_string()),
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create season");

    let mut wanted_ids = Vec::new();
    for episode_number in 1..=100 {
        let episode_number = episode_number.to_string();
        let label = format!("S01E{episode_number:0>2}");
        let episode = app
            .services
            .catalog
            .shows
            .create_episode(Episode {
                id: Id::new().0,
                title_id: title.id.clone(),
                collection_id: Some(season.id.clone()),
                episode_type: scryer_domain::EpisodeType::Standard,
                episode_number: Some(episode_number.clone()),
                season_number: Some("1".to_string()),
                episode_label: Some(label.clone()),
                title: Some(label),
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
                episode_number: Some(episode_number),
                media_type: "episode".to_string(),
                last_search_at: None,
                status: AcquisitionScopeStatus::Wanted,
                grabbed_release: None,
                landed_bar: None,
                latest_release_decision: None,
                mismatch_recovery_eligible: false,
                created_at: Utc::now().to_rfc3339(),
                updated_at: Utc::now().to_rfc3339(),
            })
            .await
            .expect("seed due wanted item");
    }

    // One cycle is one default 60-second poll tick, so completing all 100 here
    // is stronger than the five-simulated-minute throughput requirement.
    app.run_background_acquisition_cycle_once().await;

    let searches = recorded_searches.lock().await.clone();
    assert!(
        searches
            .iter()
            .any(|search| search.season == Some(1) && search.episode.is_none())
    );
    let submitted_titles = download_client
        .submitted_release_titles
        .lock()
        .await
        .clone();
    assert_eq!(
        submitted_titles.len(),
        100,
        "searches: {searches:?}; submitted: {submitted_titles:?}"
    );
    for episode_number in 1..=100 {
        let expected_title = format!("Emberfall S01E{episode_number:02}.1080p.WEB-DL");
        assert!(
            submitted_titles.contains(&expected_title),
            "missing episode submission {expected_title}"
        );
    }

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
        let episode_number = wanted
            .episode_number
            .as_deref()
            .and_then(|value| value.parse::<u32>().ok())
            .expect("wanted episode number should be numeric");
        let expected_title = format!("Emberfall S01E{episode_number:02}.1080p.WEB-DL");
        assert_eq!(
            grabbed_release["title"].as_str(),
            Some(expected_title.as_str())
        );
        assert_ne!(grabbed_release["season_pack"].as_bool(), Some(true));
    }
}

async fn seed_series_pack_scope_fixture(
    indexer_client: Arc<TrackingIndexerClient>,
) -> (
    AppUseCase,
    Title,
    Arc<TrackingIndexerClient>,
    Vec<(u32, String)>,
) {
    seed_series_pack_scope_fixture_with_persisted_seasons(indexer_client, &[1, 2, 3, 4, 5]).await
}

async fn seed_series_pack_scope_fixture_with_persisted_seasons(
    indexer_client: Arc<TrackingIndexerClient>,
    persisted_seasons: &[u32],
) -> (
    AppUseCase,
    Title,
    Arc<TrackingIndexerClient>,
    Vec<(u32, String)>,
) {
    seed_series_pack_scope_fixture_with_download_client(
        indexer_client,
        persisted_seasons,
        Arc::new(StubDownloadClient::default().with_unique_job_ids()),
    )
    .await
}

async fn seed_series_pack_scope_fixture_with_download_client(
    indexer_client: Arc<TrackingIndexerClient>,
    persisted_seasons: &[u32],
    download_client: Arc<StubDownloadClient>,
) -> (
    AppUseCase,
    Title,
    Arc<TrackingIndexerClient>,
    Vec<(u32, String)>,
) {
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking_and_indexer(
        download_client,
        Arc::new(TrackingDownloadSubmissionRepo::default()),
        Arc::new(TrackingPendingReleaseRepo::default()),
        wanted_items.clone(),
        indexer_client.clone(),
    );
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Series Pack Scope".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create series-pack title");

    let mut episode_ids = Vec::new();
    for season_number in 1..=5 {
        let season = app
            .services
            .catalog
            .shows
            .create_collection(Collection {
                id: Id::new().0,
                title_id: title.id.clone(),
                collection_type: CollectionType::Season,
                collection_index: season_number.to_string(),
                label: Some(format!("Season {season_number}")),
                ordered_path: None,
                narrative_order: Some(season_number.to_string()),
                first_episode_number: Some("1".to_string()),
                last_episode_number: Some("1".to_string()),
                monitored: true,
                created_at: Utc::now(),
            })
            .await
            .expect("create series-pack season");
        let episode = app
            .services
            .catalog
            .shows
            .create_episode(Episode {
                id: Id::new().0,
                title_id: title.id.clone(),
                collection_id: Some(season.id.clone()),
                episode_type: scryer_domain::EpisodeType::Standard,
                episode_number: Some("1".to_string()),
                season_number: Some(season_number.to_string()),
                episode_label: Some(format!("S{season_number:02}E01")),
                title: Some(format!("S{season_number:02}E01")),
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
            .expect("create series-pack episode");
        if persisted_seasons.contains(&season_number) {
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
                    collection_id: Some(season.id),
                    series_movie_link_id: None,
                    season_number: Some(season_number.to_string()),
                    episode_number: Some("1".to_string()),
                    media_type: "episode".to_string(),
                    last_search_at: None,
                    status: AcquisitionScopeStatus::Wanted,
                    grabbed_release: None,
                    landed_bar: None,
                    latest_release_decision: None,
                    mismatch_recovery_eligible: false,
                    created_at: Utc::now().to_rfc3339(),
                    updated_at: Utc::now().to_rfc3339(),
                })
                .await
                .expect("seed series-pack wanted episode");
        }
        episode_ids.push((season_number, episode.id));
    }

    (app, title, indexer_client, episode_ids)
}

fn series_pack_anchor_standby(
    title: &Title,
    wanted_item_id: &str,
    release_title: &str,
) -> PendingRelease {
    let release_guid = format!("anchor-standby-{wanted_item_id}");
    PendingRelease {
        id: Id::new().0,
        wanted_item_id: wanted_item_id.to_string(),
        title_id: title.id.clone(),
        release_title: release_title.to_string(),
        release_url: Some("https://example.invalid/anchor-standby.nzb".to_string()),
        source_kind: Some(DownloadSourceKind::NzbUrl),
        release_size_bytes: None,
        release_score: 100,
        scoring_log_json: None,
        indexer_source: Some("nzbgeek".to_string()),
        indexer_id: None,
        release_guid: Some(release_guid.clone()),
        added_at: "2026-01-01T00:00:00Z".to_string(),
        last_observed_at: "2026-01-01T00:00:00Z".to_string(),
        delay_until: "2026-01-01T00:00:00Z".to_string(),
        status: PendingReleaseStatus::Standby,
        grabbed_at: None,
        source_password: None,
        published_at: None,
        info_hash: None,
        seed_minimums: Default::default(),
        seeders: None,
        release_identity: format!("guid:nzbgeek:{}", release_guid.to_ascii_lowercase()),
        coverage_identity: format!("scope:{wanted_item_id}"),
        role: crate::types::PendingReleaseRole::Fallback,
        last_decision_code: None,
        release_age_unknown: false,
    }
}

#[tokio::test]
async fn exhausted_series_pack_search_restores_anchor_episode_standby_rows() {
    let download_client = Arc::new(StubDownloadClient::default().with_unique_job_ids());
    download_client
        .set_submit_error(Some(StubSubmitError::Rejected(
            "series pack unavailable".to_string(),
        )))
        .await;
    let pack_title = "Series.Pack.Scope.S01-S04.1080p.WEB-DL-PACK".to_string();
    let indexer_client = Arc::new(
        TrackingIndexerClient::default()
            .with_title_pack_titles([pack_title])
            .failing_scoped_queries(),
    );
    let (app, title, _, _) = seed_series_pack_scope_fixture_with_download_client(
        indexer_client,
        &[1, 2, 3, 4, 5],
        download_client,
    )
    .await;
    let anchor_episode_id = app
        .services
        .catalog
        .shows
        .list_episodes_for_title(&title.id)
        .await
        .expect("list series-pack episodes")
        .into_iter()
        .find(|episode| {
            episode.episode_type == scryer_domain::EpisodeType::Standard
                && episode.monitored
                && episode.air_date.as_deref() == Some("2024-01-01")
        })
        .map(|episode| episode.id)
        .expect("series-pack anchor exists");
    let mut anchor = app
        .services
        .workflow
        .acquisition_scope_states
        .get_acquisition_scope_state_for_title(&title.id, Some(&anchor_episode_id))
        .await
        .expect("load anchor state")
        .expect("anchor state exists");
    // The episode lane normally walks this row before the title lane. Keep it
    // out of that first walk so this test exercises the pack lane's own
    // delete-and-exhaust path.
    anchor.status = AcquisitionScopeStatus::Grabbed;
    app.services
        .workflow
        .acquisition_scope_states
        .upsert_acquisition_scope_state(&anchor)
        .await
        .expect("hold anchor outside the episode standby walk");
    let original = series_pack_anchor_standby(
        &title,
        &anchor.id,
        "Series.Pack.Scope.S01E01.1080p.WEB-DL-ORIGINAL",
    );
    app.services
        .workflow
        .pending_releases
        .insert_pending_release(&original)
        .await
        .expect("seed anchor standby");

    app.run_background_acquisition_cycle_once().await;

    let standby = app
        .services
        .workflow
        .pending_releases
        .list_standby_pending_releases_for_wanted_item(&anchor.id)
        .await
        .expect("load restored anchor standby");
    assert_eq!(standby.len(), 1);
    let restored = standby.first().expect("restored standby exists");
    assert_eq!(restored.id, original.id);
    assert_eq!(restored.release_title, original.release_title);
    assert_eq!(restored.wanted_item_id, original.wanted_item_id);
}

#[tokio::test]
async fn in_flight_series_episodes_count_as_owned_for_the_pack_ratio_gate() {
    let pack_title = "Series.Pack.Scope.S01-S02.1080p.WEB-DL-PACK".to_string();
    let indexer_client =
        Arc::new(TrackingIndexerClient::default().with_title_pack_titles([pack_title.clone()]));
    let (app, title, _, episode_ids) = seed_series_pack_scope_fixture(indexer_client).await;
    let season_one_collection_id = app
        .services
        .catalog
        .shows
        .get_episode_by_id(
            episode_ids
                .iter()
                .find(|(season, _)| *season == 1)
                .map(|(_, episode_id)| episode_id)
                .expect("season one episode exists"),
        )
        .await
        .expect("load season one episode")
        .and_then(|episode| episode.collection_id)
        .expect("season one collection exists");
    let mut season_one_episode_ids = episode_ids
        .iter()
        .filter(|(season, _)| *season == 1)
        .map(|(_, episode_id)| episode_id.clone())
        .collect::<Vec<_>>();
    for episode_number in 2..=4 {
        let episode = app
            .services
            .catalog
            .shows
            .create_episode(Episode {
                id: Id::new().0,
                title_id: title.id.clone(),
                collection_id: Some(season_one_collection_id.clone()),
                episode_type: scryer_domain::EpisodeType::Standard,
                episode_number: Some(episode_number.to_string()),
                season_number: Some("1".to_string()),
                episode_label: Some(format!("S01E{episode_number:02}")),
                title: Some(format!("S01E{episode_number:02}")),
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
            .expect("create additional season one episode");
        season_one_episode_ids.push(episode.id);
    }
    for (season, episode_id) in episode_ids.iter().filter(|(season, _)| *season >= 3) {
        let file_id = app
            .services
            .library
            .media_files
            .insert_media_file(&crate::InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path: format!("/library/Series Pack Scope/S{season:02}E01.mkv"),
                size_bytes: 1_000_000,
                role: crate::MediaFileRole::Primary,
                ..Default::default()
            })
            .await
            .expect("seed owned episode file");
        app.services
            .library
            .media_files
            .link_file_to_episode(&file_id, episode_id)
            .await
            .expect("link owned episode file");
    }
    let active_submission = DownloadSubmission {
        download_id: scryer_domain::download_identity::DownloadId::new(),
        title_id: title.id.clone(),
        purpose: crate::DownloadSubmissionPurpose::Standard,
        facet: "anime".to_string(),
        download_client_id: Some("primary".to_string()),
        download_client_type: "nzbget".to_string(),
        download_client_item_id: "season-one-active".to_string(),
        source_hint: None,
        source_provider_id: None,
        source_provider_name: None,
        source_kind: Some(DownloadSourceKind::NzbUrl),
        source_title: Some("Series.Pack.Scope.S01.1080p.WEB-DL".to_string()),
        info_hash: None,
        release_size_bytes: None,
        request_signature: None,
        scope: SubmissionScope::EpisodeSet {
            episode_ids: season_one_episode_ids,
        },
    };
    let active_identity = ClientJobLocator::from_submission(&active_submission);
    app.services
        .workflow
        .download_submissions
        .record_submission(active_submission)
        .await
        .expect("record active season one submission");
    app.services
        .workflow
        .download_submissions
        .update_tracked_state(
            &active_identity,
            TrackedDownloadState::ImportPending.as_str(),
        )
        .await
        .expect("mark season one submission active");

    app.run_background_acquisition_cycle_once().await;

    let submissions = app
        .services
        .workflow
        .download_submissions
        .list_for_title(&title.id)
        .await
        .expect("list title submissions");
    assert!(
        !submissions
            .iter()
            .any(|submission| submission.source_title.as_deref() == Some(pack_title.as_str())),
        "the in-flight S01 scope leaves only one of eight episodes missing: {submissions:?}"
    );
}

#[tokio::test]
async fn series_pack_candidate_overlapping_an_earlier_cycle_claim_is_not_submitted() {
    let pack_title = "Series.Pack.Scope.S01-S02.1080p.WEB-DL-PACK".to_string();
    let indexer_client = Arc::new(
        TrackingIndexerClient::default()
            .with_title_pack_titles([pack_title.clone()])
            .failing_scoped_queries(),
    );
    let (app, title, _, episode_ids) =
        seed_series_pack_scope_fixture_with_persisted_seasons(indexer_client, &[1, 2]).await;
    let mut recovered_releases = Vec::new();
    for season in 1..=2 {
        let episode_id = episode_ids
            .iter()
            .find(|(candidate_season, _)| *candidate_season == season)
            .map(|(_, episode_id)| episode_id)
            .expect("claimed season episode exists");
        let wanted = app
            .services
            .workflow
            .acquisition_scope_states
            .get_acquisition_scope_state_for_title(&title.id, Some(episode_id))
            .await
            .expect("load claimed season wanted state")
            .expect("claimed season wanted state exists");
        let recovered = series_pack_anchor_standby(
            &title,
            &wanted.id,
            &format!("Series.Pack.Scope.S{season:02}E01.1080p.WEB-DL-RECOVERY"),
        );
        app.services
            .workflow
            .pending_releases
            .insert_pending_release(&recovered)
            .await
            .expect("seed recovered episode standby");
        recovered_releases.push(recovered);
    }

    app.run_background_acquisition_cycle_once().await;

    let submissions = app
        .services
        .workflow
        .download_submissions
        .list_for_title(&title.id)
        .await
        .expect("list title submissions");
    assert!(submissions.iter().any(|submission| {
        submission
            .source_title
            .as_deref()
            .is_some_and(|source_title| {
                recovered_releases
                    .iter()
                    .any(|recovered| recovered.release_title == source_title)
            })
    }));
    assert!(
        !submissions
            .iter()
            .any(|submission| submission.source_title.as_deref() == Some(pack_title.as_str())),
        "the S01-S02 pack overlaps S01 recovered earlier in this cycle"
    );
}

#[tokio::test]
async fn multi_season_series_pack_claims_only_its_episode_set_and_leaves_later_seasons_active() {
    let pack_title = "Series.Pack.Scope.S01-S04.1080p.WEB-DL-PACK".to_string();
    let indexer_client =
        Arc::new(TrackingIndexerClient::default().with_title_pack_titles([pack_title.clone()]));
    let (app, title, indexer_client, episode_ids) =
        seed_series_pack_scope_fixture(indexer_client).await;

    app.run_background_acquisition_cycle_once().await;

    let searches = indexer_client.searches.lock().await.clone();
    assert_eq!(
        searches
            .iter()
            .filter(|search| search.season.is_none() && search.episode.is_none())
            .count(),
        1,
        "the title pack lane must query a show only once per cycle"
    );
    assert!(
        searches
            .iter()
            .any(|search| search.season == Some(5) && search.episode == Some(1)),
        "S01-S04 may not suppress the uncovered S05 episode lane: {searches:?}"
    );

    let submissions = app
        .services
        .workflow
        .download_submissions
        .list_for_title(&title.id)
        .await
        .expect("list exact-scope submissions");
    let states = app
        .services
        .workflow
        .acquisition_scope_states
        .list_acquisition_scope_states_for_title_ids(std::slice::from_ref(&title.id))
        .await
        .expect("list exact-scope states");
    let series_submission = submissions
        .iter()
        .find(|submission| submission.source_title.as_deref() == Some(pack_title.as_str()))
        .unwrap_or_else(|| {
            panic!("series pack submission exists: {submissions:#?}; states: {states:#?}")
        });
    let expected_pack_ids = episode_ids
        .iter()
        .filter(|(season, _)| *season <= 4)
        .map(|(_, episode_id)| episode_id.clone())
        .collect::<HashSet<_>>();
    let SubmissionScope::EpisodeSet {
        episode_ids: submitted_episode_ids,
    } = &series_submission.scope
    else {
        panic!("series pack must submit an exact EpisodeSet scope");
    };
    assert_eq!(
        submitted_episode_ids
            .iter()
            .cloned()
            .collect::<HashSet<_>>(),
        expected_pack_ids
    );
    let season_five_episode_id = episode_ids
        .iter()
        .find(|(season, _)| *season == 5)
        .map(|(_, episode_id)| episode_id)
        .expect("season five episode exists");
    assert!(submissions.iter().any(|submission| {
        matches!(
            &submission.scope,
            SubmissionScope::Episode { episode_id } if episode_id == season_five_episode_id
        )
    }));
}

#[tokio::test]
async fn a_disjoint_series_pack_is_anchored_inside_its_own_episode_set() {
    let pack_title = "Series.Pack.Scope.S03-S04.1080p.WEB-DL-PACK".to_string();
    let indexer_client = Arc::new(
        TrackingIndexerClient::default()
            .with_title_pack_titles([pack_title])
            .failing_scoped_queries(),
    );
    let (app, title, _, episode_ids) =
        seed_series_pack_scope_fixture_with_persisted_seasons(indexer_client, &[1]).await;

    app.run_background_acquisition_cycle_once().await;

    let states = app
        .services
        .workflow
        .acquisition_scope_states
        .list_acquisition_scope_states_for_title_ids(std::slice::from_ref(&title.id))
        .await
        .expect("list exact-scope states");
    let state_for_season = |season| {
        let episode_id = episode_ids
            .iter()
            .find(|(candidate_season, _)| *candidate_season == season)
            .map(|(_, episode_id)| episode_id)
            .expect("fixture season exists");
        states
            .iter()
            .find(|state| state.episode_id.as_ref() == Some(episode_id))
    };

    assert_eq!(
        state_for_season(1).map(|state| state.status),
        Some(AcquisitionScopeStatus::Wanted)
    );
    assert!(state_for_season(2).is_none());
    assert_eq!(
        state_for_season(3).map(|state| state.status),
        Some(AcquisitionScopeStatus::Grabbed)
    );
    assert!(state_for_season(4).is_none());
}

#[tokio::test]
async fn series_pack_grab_persists_the_ranked_runner_up_in_shared_standby() {
    let pack_titles = [
        "Series.Pack.Scope.S01-S04.1080p.WEB-DL-PACKA".to_string(),
        "Series.Pack.Scope.S01-S04.1080p.WEB-DL-PACKB".to_string(),
        "Series.Pack.Scope.S01-S04.1080p.WEB-DL-PACKC".to_string(),
    ];
    let indexer_client =
        Arc::new(TrackingIndexerClient::default().with_title_pack_titles(pack_titles.clone()));
    let (app, title, _, _) = seed_series_pack_scope_fixture(indexer_client).await;

    app.run_background_acquisition_cycle_once().await;

    let submissions = app
        .services
        .workflow
        .download_submissions
        .list_for_title(&title.id)
        .await
        .expect("list series-pack submissions");
    let submitted_pack = submissions
        .iter()
        .filter_map(|submission| submission.source_title.as_deref())
        .find(|release_title| pack_titles.iter().any(|title| title == release_title))
        .expect("one series pack should be submitted");
    let mut standby = app
        .services
        .workflow
        .pending_releases
        .list_standby_pending_releases_for_title(&title.id)
        .await
        .expect("list saved series-pack candidates");
    standby.sort_by(|left, right| left.added_at.cmp(&right.added_at));

    assert_eq!(submitted_pack, pack_titles[0]);
    assert_eq!(standby.len(), 2, "the untried runner-ups must be durable");
    assert_eq!(standby[0].release_title, pack_titles[1]);
    assert_eq!(standby[1].release_title, pack_titles[2]);
}

#[tokio::test]
async fn overlapping_series_pack_standby_waits_while_a_disjoint_pack_can_run() {
    let pack_titles = [
        "Series.Pack.Scope.S01-S02.1080p.WEB-DL-PACKA".to_string(),
        "Series.Pack.Scope.S02-S03.1080p.WEB-DL-PACKB".to_string(),
        "Series.Pack.Scope.S03-S04.1080p.WEB-DL-PACKC".to_string(),
    ];
    let indexer_client =
        Arc::new(TrackingIndexerClient::default().with_title_pack_titles(pack_titles.clone()));
    let (app, title, _, _) = seed_series_pack_scope_fixture(indexer_client).await;

    app.run_background_acquisition_cycle_once().await;

    let submissions = app
        .services
        .workflow
        .download_submissions
        .list_for_title(&title.id)
        .await
        .expect("list series-pack submissions");
    let submitted_titles = submissions
        .iter()
        .filter_map(|submission| submission.source_title.as_deref())
        .collect::<HashSet<_>>();
    assert!(submitted_titles.contains(pack_titles[0].as_str()));
    assert!(!submitted_titles.contains(pack_titles[1].as_str()));
    assert!(submitted_titles.contains(pack_titles[2].as_str()));

    let standby = app
        .services
        .workflow
        .pending_releases
        .list_standby_pending_releases_for_title(&title.id)
        .await
        .expect("list overlapping standby");
    assert!(
        standby
            .iter()
            .any(|pending| pending.release_title == pack_titles[1]),
        "the overlapping candidate stays saved for failure recovery"
    );
}

#[tokio::test]
async fn one_missing_episode_does_not_trigger_the_series_pack_title_lane() {
    let indexer_client = Arc::new(TrackingIndexerClient::default().with_title_pack_titles([
        "Series.Pack.Scope.Complete.Series.1080p.WEB-DL-PACK".to_string(),
    ]));
    let (app, title, indexer_client, episode_ids) =
        seed_series_pack_scope_fixture(indexer_client).await;

    for (season, episode_id) in episode_ids.iter().take(4) {
        let file_id = app
            .services
            .library
            .media_files
            .insert_media_file(&crate::InsertMediaFileInput {
                title_id: title.id.clone(),
                file_path: format!("/library/Series Pack Scope/S{season:02}E01.mkv"),
                size_bytes: 1_000_000,
                role: crate::MediaFileRole::Primary,
                ..Default::default()
            })
            .await
            .expect("seed owned episode file");
        app.services
            .library
            .media_files
            .link_file_to_episode(&file_id, episode_id)
            .await
            .expect("link owned episode file");
    }

    app.run_background_acquisition_cycle_once().await;

    let searches = indexer_client.searches.lock().await.clone();
    assert!(
        searches
            .iter()
            .all(|search| search.season.is_some() || search.episode.is_some()),
        "one missing episode must not spend a bare-title pack query: {searches:?}"
    );
}

#[tokio::test]
async fn title_search_success_does_not_converge_an_episode_when_scoped_queries_fail() {
    let indexer_client = Arc::new(
        TrackingIndexerClient::default()
            .failing_scoped_queries()
            .reporting_routed_indexers_fired(),
    );
    let (app, title, indexer_client, _) =
        seed_recent_failed_season_pack_fixture_with_indexer(indexer_client).await;

    app.run_background_acquisition_cycle_once().await;
    let first_searches = indexer_client.searches.lock().await.clone();
    assert_eq!(
        first_searches
            .iter()
            .filter(|search| search.season.is_none() && search.episode.is_none())
            .count(),
        1,
        "the successful bare-title lookup is independently converged"
    );
    let first_scoped_count = first_searches
        .iter()
        .filter(|search| search.season.is_some() || search.episode.is_some())
        .count();
    assert!(first_scoped_count > 0, "fixture must fail scoped queries");
    let states = app
        .services
        .workflow
        .acquisition_scope_states
        .list_acquisition_scope_states_for_title_ids(std::slice::from_ref(&title.id))
        .await
        .expect("load failed-query episode states");
    assert!(
        states
            .iter()
            .filter(|state| state.media_type == "episode")
            .all(|state| state.last_search_at.is_none()),
        "a title lookup may not mutate an episode's search state"
    );

    app.run_background_acquisition_cycle_once().await;
    let second_searches = indexer_client.searches.lock().await.clone();
    assert!(
        second_searches
            .iter()
            .filter(|search| search.season.is_some() || search.episode.is_some())
            .count()
            > first_scoped_count,
        "failed episode and season queries must remain retryable"
    );
}

#[tokio::test]
async fn background_search_scopes_do_not_emit_search_completed_events() {
    let (app, _, _) = seed_recent_failed_season_pack_fixture().await;
    let domain_events = Arc::new(MockDomainEventRepo::default());
    let app = app.with_test_overrides(|builder| builder.with_domain_events(domain_events.clone()));

    app.run_background_acquisition_cycle_once().await;

    assert!(domain_events.events.lock().await.iter().all(|event| {
        !matches!(
            event.payload,
            DomainEventPayload::AcquisitionSearchCompleted(_)
        )
    }));
}

/// Anime title with two due Season 7 episodes (so the cycle attempts a season
/// pack first) and a tracking indexer that answers every query.
async fn seed_recent_failed_season_pack_fixture() -> (AppUseCase, Title, Arc<TrackingIndexerClient>)
{
    let (app, title, indexer_client, _) = seed_recent_failed_season_pack_fixture_with_indexer(
        Arc::new(TrackingIndexerClient::default()),
    )
    .await;
    (app, title, indexer_client)
}

async fn seed_recent_failed_season_pack_fixture_with_indexer(
    indexer_client: Arc<TrackingIndexerClient>,
) -> (
    AppUseCase,
    Title,
    Arc<TrackingIndexerClient>,
    Arc<StubDownloadClient>,
) {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking_and_indexer(
        download_client.clone(),
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
                landed_bar: None,
                latest_release_decision: None,
                mismatch_recovery_eligible: false,
                created_at: Utc::now().to_rfc3339(),
                updated_at: Utc::now().to_rfc3339(),
            })
            .await
            .expect("seed due episode wanted item");
    }

    (app, title, indexer_client, download_client)
}

#[tokio::test]
async fn season_pack_grab_saves_the_remaining_ranked_packs_as_standby() {
    let first_pack = "Recent.Failed.Season.Pack.S07.1080p.WEB-DL-FIRST".to_string();
    let second_pack = "Recent.Failed.Season.Pack.S07.1080p.WEB-DL-SECOND".to_string();
    let indexer_client = Arc::new(
        TrackingIndexerClient::default()
            .with_season_pack_titles([first_pack.clone(), second_pack.clone()]),
    );
    let (app, title, indexer_client, _) =
        seed_recent_failed_season_pack_fixture_with_indexer(indexer_client).await;

    app.run_background_acquisition_cycle_once().await;

    let standby = app
        .services
        .workflow
        .pending_releases
        .list_all_standby_pending_releases()
        .await
        .expect("list standby rows");
    assert_eq!(standby.len(), 1, "only the runner-up pack remains");
    assert_eq!(standby[0].title_id, title.id);
    assert_eq!(standby[0].release_title, second_pack);
    assert_eq!(standby[0].status, PendingReleaseStatus::Standby);
    assert!(
        !standby[0].wanted_item_id.is_empty(),
        "the remaining pack is keyed to the anchor episode scope"
    );
    let searches = indexer_client.searches.lock().await.clone();
    assert!(
        searches
            .iter()
            .any(|search| search.season == Some(7) && search.episode.is_none()),
        "fixture must drive the actual season-pack branch"
    );
    assert!(
        !searches.iter().any(|search| search.episode.is_some()),
        "a chosen season pack covers both due episodes without episode searches: {searches:?}"
    );
}

/// A season pack no longer wins simply by being the stage that ran first.
///
/// The season query surfaces an episode-shaped release the episode query never
/// would; that row is already in hand, so ranking it against the pack costs
/// nothing. When it is the better release the episode grab wins and the pack is
/// set aside — and no episode-scoped query is spent finding that out.
#[tokio::test]
async fn in_hand_episode_evidence_outranks_a_season_pack_without_a_new_query() {
    let pack = "Recent.Failed.Season.Pack.S07.720p.WEB-DL-PACK".to_string();
    let episode_release = "Recent.Failed.Season.Pack.S07E23.1080p.WEB-DL-EP".to_string();
    let indexer_client = Arc::new(
        TrackingIndexerClient::default()
            .with_season_pack_titles([pack.clone(), episode_release.clone()]),
    );
    let (app, title, indexer_client, download_client) =
        seed_recent_failed_season_pack_fixture_with_indexer(indexer_client).await;

    app.run_background_acquisition_cycle_once().await;

    let submitted = download_client
        .submitted_release_titles
        .lock()
        .await
        .clone();
    assert!(
        submitted.contains(&episode_release),
        "the better in-hand episode release must win arbitration: {submitted:?}"
    );
    assert!(
        !submitted.contains(&pack),
        "the pack covers a claimed episode and must be set aside: {submitted:?}"
    );

    let searches = indexer_client.searches.lock().await.clone();
    assert!(
        !searches.iter().any(|search| search.episode.is_some()),
        "comparing the pack against evidence already in hand may not spend a query: {searches:?}"
    );

    // The sibling episode had no evidence of its own and grabbed nothing. It
    // recorded no search either, so it is still a target next cycle rather than
    // a scope converged on a query it never ran.
    let states = app
        .services
        .workflow
        .acquisition_scope_states
        .list_acquisition_scope_states_for_title_ids(std::slice::from_ref(&title.id))
        .await
        .expect("load episode scope states");
    assert!(
        states
            .iter()
            .filter(|state| state.media_type == "episode" && state.grabbed_release.is_none())
            .all(|state| state.last_search_at.is_none()),
        "a scope that only re-ranked evidence in hand has not searched"
    );
}

/// With nothing already in hand for the covered episodes, the pack wins
/// unopposed — the default, and what happened before arbitration existed.
/// Buying a comparison would cost a query, and it is not worth one.
#[tokio::test]
async fn a_season_pack_wins_by_default_when_no_free_episode_evidence_exists() {
    let pack = "Recent.Failed.Season.Pack.S07.1080p.WEB-DL-PACK".to_string();
    let indexer_client =
        Arc::new(TrackingIndexerClient::default().with_season_pack_titles([pack.clone()]));
    let (app, _title, indexer_client, download_client) =
        seed_recent_failed_season_pack_fixture_with_indexer(indexer_client).await;

    app.run_background_acquisition_cycle_once().await;

    let submitted = download_client
        .submitted_release_titles
        .lock()
        .await
        .clone();
    assert!(
        submitted.contains(&pack),
        "with no evidence to rank against it, the pack still wins: {submitted:?}"
    );
    let searches = indexer_client.searches.lock().await.clone();
    assert!(
        !searches.iter().any(|search| search.episode.is_some()),
        "the covered episode scopes stay unsearched, exactly as before: {searches:?}"
    );
}

/// A winner that cannot be submitted does not take its episodes with it. The
/// next-best proposal for the same episodes gets its turn in the same cycle.
#[tokio::test]
async fn a_losing_pack_still_grabs_when_the_winning_episode_submit_fails() {
    let pack = "Recent.Failed.Season.Pack.S07.720p.WEB-DL-PACK".to_string();
    let episode_release = "Recent.Failed.Season.Pack.S07E23.1080p.WEB-DL-EP".to_string();
    let indexer_client = Arc::new(
        TrackingIndexerClient::default()
            .with_season_pack_titles([pack.clone(), episode_release.clone()]),
    );
    let (app, _title, _indexer_client, download_client) =
        seed_recent_failed_season_pack_fixture_with_indexer(indexer_client).await;
    // Only the first submit — the arbitration winner's — fails, and it fails
    // definitively: a retryable failure would suppress the whole route, which
    // the pack shares, and the point here is the episode set, not the route.
    download_client
        .set_submit_errors([StubSubmitError::Rejected(
            "client refused the release".to_string(),
        )])
        .await;

    app.run_background_acquisition_cycle_once().await;

    let submitted = download_client
        .submitted_release_titles
        .lock()
        .await
        .clone();
    assert!(
        submitted.contains(&episode_release),
        "the winner is attempted first: {submitted:?}"
    );
    assert!(
        submitted.contains(&pack),
        "a winner that claimed nothing must hand the pack its turn: {submitted:?}"
    );
}

#[tokio::test]
async fn failed_season_pack_walks_the_saved_runner_up_without_an_indexer_query() {
    let first_pack = "Recent.Failed.Season.Pack.S07.1080p.WEB-DL-FIRST".to_string();
    let second_pack = "Recent.Failed.Season.Pack.S07.1080p.WEB-DL-SECOND".to_string();
    let indexer_client = Arc::new(
        TrackingIndexerClient::default()
            .with_season_pack_titles([first_pack.clone(), second_pack.clone()]),
    );
    let (app, title, indexer_client, download_client) =
        seed_recent_failed_season_pack_fixture_with_indexer(indexer_client).await;
    app.run_background_acquisition_cycle_once().await;
    let runner_up = app
        .services
        .workflow
        .pending_releases
        .list_all_standby_pending_releases()
        .await
        .expect("load saved pack")
        .into_iter()
        .next()
        .expect("runner-up season pack was persisted");
    let failed_submission = app
        .services
        .workflow
        .download_submissions
        .list_for_title(&title.id)
        .await
        .expect("load first pack submission")
        .into_iter()
        .find(|submission| submission.source_title.as_deref() == Some(first_pack.as_str()))
        .expect("first pack submission exists");

    assert_eq!(
        crate::acquisition_workflow::process_download_failure(
            &app,
            crate::acquisition_workflow::DownloadFailureContext {
                wanted_item: None,
                title_id: Some(title.id.clone()),
                client_id: failed_submission
                    .download_client_id
                    .clone()
                    .unwrap_or_default(),
                client_type: failed_submission.download_client_type.clone(),
                client_name: Some("Primary".to_string()),
                client_item_id: failed_submission.download_client_item_id.clone(),
                release_title: first_pack.clone(),
                reason: "download failed".to_string(),
                remove_from_client_if_configured: false,
                skip_reacquire: false,
            },
        )
        .await,
        crate::acquisition_workflow::FailureHandlingOutcome::Reopened
    );
    download_client.queue_items.lock().await.clear();
    download_client
        .set_snapshot_authoritative_client_ids(["primary".to_string()])
        .await;
    indexer_client.searches.lock().await.clear();

    app.run_background_acquisition_cycle_once().await;

    assert!(
        indexer_client.searches.lock().await.is_empty(),
        "the reopened covered episode scopes must walk saved packs before any indexer query"
    );
    assert_eq!(
        app.services
            .workflow
            .pending_releases
            .get_pending_release(&runner_up.id)
            .await
            .expect("load runner-up")
            .expect("runner-up exists")
            .status,
        PendingReleaseStatus::Grabbed
    );
    let states = app
        .services
        .workflow
        .acquisition_scope_states
        .list_acquisition_scope_states_for_title_ids(std::slice::from_ref(&title.id))
        .await
        .expect("list covered episode scopes");
    assert!(
        states
            .iter()
            .filter(|state| state.media_type == "episode")
            .all(|state| state.status == AcquisitionScopeStatus::Grabbed),
        "the replacement pack covers every episode it matches: {states:?}"
    );
    assert_eq!(
        app.services
            .workflow
            .download_submissions
            .list_for_title(&title.id)
            .await
            .expect("list pack submissions")
            .iter()
            .filter(|submission| submission.source_title.as_deref() == Some(second_pack.as_str()))
            .count(),
        1,
        "two covered episode targets may not claim the same saved pack twice"
    );

    let failed_runner_up = app
        .services
        .workflow
        .download_submissions
        .list_for_title(&title.id)
        .await
        .expect("load runner-up submission")
        .into_iter()
        .find(|submission| submission.source_title.as_deref() == Some(second_pack.as_str()))
        .expect("runner-up submission exists");
    assert_eq!(
        crate::acquisition_workflow::process_download_failure(
            &app,
            crate::acquisition_workflow::DownloadFailureContext {
                wanted_item: None,
                title_id: Some(title.id.clone()),
                client_id: failed_runner_up
                    .download_client_id
                    .clone()
                    .unwrap_or_default(),
                client_type: failed_runner_up.download_client_type.clone(),
                client_name: Some("Primary".to_string()),
                client_item_id: failed_runner_up.download_client_item_id.clone(),
                release_title: second_pack.clone(),
                reason: "download failed".to_string(),
                remove_from_client_if_configured: false,
                skip_reacquire: false,
            },
        )
        .await,
        crate::acquisition_workflow::FailureHandlingOutcome::Reopened
    );
    download_client.queue_items.lock().await.clear();
    indexer_client.searches.lock().await.clear();
    app.run_background_acquisition_cycle_once().await;
    let searches_after_exhaustion = indexer_client.searches.lock().await.clone();
    assert!(
        searches_after_exhaustion.iter().all(|search| {
            search.episode.is_some() || (search.season.is_none() && search.episode.is_none())
        }),
        "the converged pack scope stays converged; only previously-uncovered episodes are searched: {searches_after_exhaustion:?}"
    );
    assert_eq!(
        searches_after_exhaustion
            .iter()
            .filter(|search| search.season.is_none() && search.episode.is_none())
            .count(),
        1,
        "the show receives one title-level series-pack lookup"
    );
    assert!(
        !searches_after_exhaustion.is_empty(),
        "the exhausted list leaves an uncovered episode eligible for its first search"
    );
}

#[tokio::test]
async fn a_waiting_season_pack_parks_a_covered_sibling_episode_walk() {
    let (app, title, _) = seed_recent_failed_season_pack_fixture().await;
    let episode_scopes = app
        .services
        .workflow
        .acquisition_scope_states
        .list_acquisition_scope_states_for_title_ids(std::slice::from_ref(&title.id))
        .await
        .expect("list episode scopes");
    let mut episode_scopes = episode_scopes
        .into_iter()
        .filter(|scope| scope.media_type == "episode")
        .collect::<Vec<_>>();
    episode_scopes.sort_by(|left, right| left.id.cmp(&right.id));
    let anchor = episode_scopes.first().expect("anchor episode");
    let sibling = episode_scopes.get(1).expect("sibling episode");
    let mut pack = pending_movie_release(
        &anchor.id,
        &title,
        "Recent.Failed.Season.Pack.S07.1080p.WEB-DL-WAITING",
        PendingReleaseStatus::Waiting,
    );
    pack.added_at = Utc::now().to_rfc3339();
    app.services
        .workflow
        .pending_releases
        .insert_pending_release(&pack)
        .await
        .expect("seed waiting season pack");
    let snapshot = crate::acquisition_workflow::DownloadClientSnapshot::fetch(&app).await;

    assert!(
        matches!(
            crate::acquisition_workflow::try_saved_candidates(
                &app,
                sibling,
                None,
                None,
                &snapshot,
                &Utc::now(),
            )
            .await,
            crate::acquisition_workflow::StandbyRecoveryOutcome::Parked { scope: Some(_) }
        ),
        "a waiting pack covering this episode owns the sibling's standby walk"
    );
}

#[tokio::test]
async fn acquisition_cycle_skips_recently_failed_season_pack_and_searches_episodes() {
    let (app, title, indexer_client) = seed_recent_failed_season_pack_fixture().await;

    // A recent hard failure of the pack is a per-title blocklist entry (every
    // hard failure writes one); the cooldown reads that entry's age.
    app.services
        .workflow
        .blocklist_repo
        .block(&NewBlocklistEntry {
            title_id: title.id.clone(),
            release_name: "recent.failed.season.pack.s07.1080p.web-dl".to_string(),
            indexer_id: String::new(),
            info_hash: None,
            reason: Some("download client failure: corrupt archive".to_string()),
        })
        .await
        .expect("record failed season pack blocklist entry");

    app.run_background_acquisition_cycle_once().await;

    let searches = indexer_client.searches.lock().await.clone();
    assert!(!searches.is_empty());
    assert!(searches.iter().all(|search| {
        (search.season == Some(7) && search.episode.is_some())
            || (search.season.is_none() && search.episode.is_none())
    }));
    assert_eq!(
        searches
            .iter()
            .filter(|search| search.season.is_none() && search.episode.is_none())
            .count(),
        1,
        "the show receives one title-level series-pack lookup"
    );
    assert!(
        !searches
            .iter()
            .any(|search| search.season == Some(7) && search.episode.is_none())
    );
}

#[tokio::test]
async fn acquisition_cycle_failed_attempt_history_alone_does_not_cool_down_season_packs() {
    // The failed-attempt log is history/audit only. A Failed attempt with no
    // blocklist entry (e.g. one whose entry the operator removed) must not put
    // the season pack on cooldown: the pack search runs as usual.
    let (app, title, indexer_client) = seed_recent_failed_season_pack_fixture().await;

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

    app.run_background_acquisition_cycle_once().await;

    let searches = indexer_client.searches.lock().await.clone();
    assert!(
        searches
            .iter()
            .any(|search| search.season == Some(7) && search.episode.is_none()),
        "without a blocklist entry the season pack must be searched: {searches:?}"
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
                name: "Pals".into(),
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
                    "title": "Pals.S05.720p.BluRay.DD5.1.x264-NTb",
                    "score": 100,
                    "grabbed_at": Utc::now().to_rfc3339(),
                    "season_pack": true,
                })
                .to_string(),
            ),
            landed_bar: None,
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
            download_id: scryer_domain::download_identity::DownloadId::new(),
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
            source_title: Some("Pals.S05.720p.BluRay.DD5.1.x264-NTb".to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: Some(
                "nzb_url|https://example.com/pals-s05.nzb|Pals.S05.720p.BluRay.DD5.1.x264-NTb"
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
            release_title: "Pals".to_string(),
            reason: "download failed".to_string(),
            remove_from_client_if_configured: false,
            skip_reacquire: false,
        },
    )
    .await;

    assert_eq!(
        outcome,
        crate::acquisition_workflow::FailureHandlingOutcome::Reopened
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
        blocklist[0].normalized_release_name,
        "pals.s05.720p.bluray.dd5.1.x264-ntb"
    );

    app.run_background_acquisition_cycle_once().await;

    let searches = indexer_client.searches.lock().await.clone();
    assert!(!searches.is_empty());
    assert!(searches.iter().all(|search| {
        (search.season == Some(5) && search.episode.is_some())
            || (search.season.is_none() && search.episode.is_none())
    }));
    assert_eq!(
        searches
            .iter()
            .filter(|search| search.season.is_none() && search.episode.is_none())
            .count(),
        1,
        "the show receives one title-level series-pack lookup"
    );
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

    app.run_background_acquisition_cycle_once().await;

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
    assert!(
        title_blocklist_entries(&app, &title.id).await.is_empty(),
        "a transient submit failure must never blocklist the release"
    );
}

#[tokio::test]
async fn automatic_search_parks_invalid_publication_time_for_age_review() {
    let release_title = "Unknown.Age.Movie.2024.1080p.WEB-DL-GRP";
    let download_client = Arc::new(StubDownloadClient::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let indexer_client = Arc::new(
        FixedReleaseIndexerClient::new(release_title).with_published_at("not-a-timestamp"),
    );
    let (app, user) = bootstrap_with_acquisition_tracking_and_indexer(
        download_client.clone(),
        Arc::new(TrackingDownloadSubmissionRepo::default()),
        pending_releases.clone(),
        wanted_items.clone(),
        indexer_client,
    );
    seed_movie_wanted_for_acquisition(&app, &user, &wanted_items, "Unknown Age Movie", 2024).await;
    app.services
        .config
        .settings
        .upsert_setting_json(
            SETTINGS_SCOPE_SYSTEM,
            DELAY_PROFILE_CATALOG_KEY,
            None,
            serde_json::json!([{
                "id": "unknown-age-search",
                "name": "Unknown age search",
                "usenet_delay_minutes": 120,
            }])
            .to_string(),
            "test",
            None,
        )
        .await
        .expect("seed delay profile");

    app.run_background_acquisition_cycle_once().await;

    assert!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .is_empty()
    );
    let parked = pending_releases.store.lock().await.clone();
    let row = parked
        .iter()
        .find(|release| release.release_title == release_title)
        .expect("invalid-age automatic-search release should be parked");
    assert_eq!(row.published_at, None);
    assert!(row.release_age_unknown);
    assert_eq!(
        row.last_decision_code.as_deref(),
        Some("release_age_unknown")
    );
    let added_at =
        crate::quality_profile::parse_published_at(&row.added_at).expect("valid first-seen time");
    let delay_until = crate::quality_profile::parse_published_at(&row.delay_until)
        .expect("valid escalation deadline");
    assert_eq!(delay_until, added_at + chrono::Duration::minutes(120));
}

#[tokio::test]
async fn automatic_search_parks_hard_minimum_age_until_publication_deadline() {
    let release_title = "Minimum.Age.Movie.2024.1080p.WEB-DL-GRP";
    let published_at = Utc::now();
    let download_client = Arc::new(StubDownloadClient::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let indexer_client = Arc::new(
        FixedReleaseIndexerClient::new(release_title).with_published_at(published_at.to_rfc3339()),
    );
    let (app, user) = bootstrap_with_acquisition_tracking_and_indexer(
        download_client.clone(),
        Arc::new(TrackingDownloadSubmissionRepo::default()),
        pending_releases.clone(),
        wanted_items.clone(),
        indexer_client,
    );
    seed_movie_wanted_for_acquisition(&app, &user, &wanted_items, "Minimum Age Movie", 2024).await;
    app.services
        .config
        .settings
        .upsert_setting_json(
            SETTINGS_SCOPE_SYSTEM,
            DELAY_PROFILE_CATALOG_KEY,
            None,
            serde_json::json!([{
                "id": "minimum-age-search",
                "name": "Minimum age search",
                "usenet_delay_minutes": 0,
                "min_age_minutes": 120,
            }])
            .to_string(),
            "test",
            None,
        )
        .await
        .expect("seed delay profile");

    app.run_background_acquisition_cycle_once().await;

    assert!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .is_empty()
    );
    let parked = pending_releases.store.lock().await.clone();
    let row = parked
        .iter()
        .find(|release| release.release_title == release_title)
        .expect("minimum-age automatic-search release should be parked");
    assert!(!row.release_age_unknown);
    assert_eq!(row.last_decision_code.as_deref(), Some("minimum_age"));
    assert_eq!(
        crate::quality_profile::parse_published_at(&row.delay_until)
            .expect("valid minimum-age deadline"),
        published_at + chrono::Duration::minutes(120)
    );
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

    app.run_background_acquisition_cycle_once().await;

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
    let normalized_release_title = crate::normalize_release_name(Some(release_title));
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
    assert!(
        title_blocklist_entries(&app, &title.id).await.is_empty(),
        "a transient season-pack submit failure must never blocklist the pack"
    );
}

#[tokio::test]
async fn season_pack_definitive_submit_error_records_failed_signature_and_blocklist_entry() {
    let release_title = "Rejected.Season.Pack.S01.1080p.WEB-DL-GRP";
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
        "Rejected Season Pack",
        1,
    )
    .await;

    app.run_background_acquisition_cycle_once().await;

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

    let normalized_release_title = crate::normalize_release_name(Some(release_title));
    let failed = release_attempts
        .list_failed_release_signatures_for_title(&title.id, 10)
        .await
        .expect("list failed signatures");
    assert!(
        failed
            .iter()
            .any(|entry| entry.source_title.as_deref() == normalized_release_title.as_deref()),
        "a definitive season-pack submit failure records a Failed attempt: {failed:?}"
    );
    let blocklist = title_blocklist_entries(&app, &title.id).await;
    let entry = blocklist
        .iter()
        .find(|entry| Some(&entry.normalized_release_name) == normalized_release_title.as_ref())
        .unwrap_or_else(|| {
            panic!("a definitive season-pack submit failure must blocklist the pack: {blocklist:?}")
        });
    assert!(
        entry.reason.as_deref().is_some_and(|reason| {
            reason.starts_with("season pack grab failed:") && reason.contains("Duplicate NZB")
        }),
        "the entry must say what happened: {:?}",
        entry.reason
    );
    assert!(
        !entry.indexer_id.is_empty(),
        "the entry must carry the pack's download source hint"
    );
}

#[tokio::test]
async fn season_pack_ambiguous_submit_error_defers_without_blocklist_entry() {
    // An ambiguous submit may have been accepted: the pack is treated as
    // possibly grabbed for this cycle, recorded Pending, and never blocklisted.
    let release_title = "Ambiguous.Season.Pack.S01.1080p.WEB-DL-GRP";
    let download_client = Arc::new(StubDownloadClient::default());
    download_client
        .set_submit_error(Some(StubSubmitError::Ambiguous(
            "sabnzbd addfile response was lost after the upload was sent".to_string(),
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
        "Ambiguous Season Pack",
        1,
    )
    .await;

    app.run_background_acquisition_cycle_once().await;

    assert!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .iter()
            .any(|title| title == release_title),
        "season-pack branch should submit the pack title"
    );
    let attempts = release_attempts.attempts.lock().await.clone();
    assert!(
        attempts
            .iter()
            .all(|attempt| attempt.outcome != ReleaseDownloadAttemptOutcome::Failed),
        "an ambiguous season-pack submit must not be recorded as failed: {:?}",
        attempts
            .iter()
            .map(|attempt| (&attempt.source_title, &attempt.outcome))
            .collect::<Vec<_>>()
    );
    let normalized_release_title = crate::normalize_release_name(Some(release_title));
    assert!(attempts.iter().any(|attempt| {
        attempt.source_title.as_deref() == normalized_release_title.as_deref()
            && attempt.outcome == ReleaseDownloadAttemptOutcome::Pending
    }));
    assert!(
        title_blocklist_entries(&app, &title.id).await.is_empty(),
        "an ambiguous season-pack submit must never blocklist the pack"
    );
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

    app.run_background_acquisition_cycle_once().await;

    let failed = release_attempts
        .list_failed_release_signatures_for_title(&title.id, 10)
        .await
        .expect("list failed signatures");
    assert_eq!(failed.len(), 1);
    assert_eq!(
        failed[0].source_title.as_deref(),
        Some("rejected.movie.2024.1080p.web-dl-grp")
    );
    // The definitive grab failure also burns the release for this title: the
    // blocklist entry (not the attempt) is what gates search, and it says why.
    let blocklist = title_blocklist_entries(&app, &title.id).await;
    assert_eq!(
        blocklist.len(),
        1,
        "expected one blocklist entry: {blocklist:?}"
    );
    assert_eq!(
        blocklist[0].normalized_release_name,
        "rejected.movie.2024.1080p.web-dl-grp"
    );
    assert!(
        blocklist[0].reason.as_deref().is_some_and(|reason| {
            reason.starts_with("grab failed:")
                && reason.contains("release rejected by client routing")
        }),
        "the entry must say what happened: {:?}",
        blocklist[0].reason
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

    app.run_background_acquisition_cycle_once().await;

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
    // The per-title blocklist entry is what actually burns the release for
    // search-time exclusion; the Failed attempt is the audit record.
    let blocklist = title_blocklist_entries(&app, &title.id).await;
    assert_eq!(
        blocklist.len(),
        1,
        "the vetoed release must be blocklisted for this title: {blocklist:?}"
    );
    assert_eq!(
        blocklist[0].normalized_release_name,
        "counterfeit.feature.2024.1080p.web-dl-grp"
    );
    assert!(
        blocklist[0]
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("category_mismatch")),
        "the entry must tell operators why: {:?}",
        blocklist[0].reason
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
    // FILMS — the gate must not burn Tide Chart Film Gold for being anime.
    let release_title = "Tide.Chart.Film.Gold.2024.1080p.WEB-DL-GRP";
    let anime_categorized_nzb = br#"<?xml version="1.0" encoding="iso-8859-1" ?>
<nzb xmlns="http://www.newzbin.com/DTD/2003/nzb">
<head>
 <meta type="name">Tide.Chart.Film.Gold.2024.1080p.WEB-DL-GRP</meta>
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
        seed_movie_wanted_for_acquisition(&app, &user, &wanted_items, "Tide Chart Film Gold", 2024)
            .await;

    app.run_background_acquisition_cycle_once().await;

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

    app.run_background_acquisition_cycle_once().await;

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
    let blocklist = title_blocklist_entries(&app, &title.id).await;
    assert_eq!(
        blocklist.len(),
        1,
        "a rejected submit must blocklist the release for this title: {blocklist:?}"
    );
    assert!(
        blocklist[0].reason.as_deref().is_some_and(|reason| {
            reason.starts_with("grab failed:") && reason.contains("Duplicate NZB")
        }),
        "the entry must say what happened: {:?}",
        blocklist[0].reason
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

    app.run_background_acquisition_cycle_once().await;

    let submitted_titles = download_client
        .submitted_release_titles
        .lock()
        .await
        .clone();
    assert_eq!(submitted_titles.len(), 1);
    assert!(matches!(
        submitted_titles[0].as_str(),
        "Deferred.Movie.2024.1080p.WEB-DL-GRP" | "Rejected.Movie.2024.1080p.WEB-DL-GRP"
    ));

    let submissions = download_submissions.store.lock().await.clone();
    assert_eq!(submissions.len(), 1);
    assert_eq!(
        submissions[0].source_title.as_deref(),
        Some(submitted_titles[0].as_str())
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
    assert_eq!(
        [first, second]
            .into_iter()
            .filter(|item| item.status == AcquisitionScopeStatus::Grabbed)
            .count(),
        1
    );
    assert_eq!(
        [first, second]
            .into_iter()
            .filter(|item| item.status == AcquisitionScopeStatus::Wanted)
            .count(),
        1,
        "the duplicate URL must not mark both wanted items grabbed"
    );
    assert!(
        store
            .iter()
            .filter_map(|item| item.grabbed_release.as_deref())
            .all(|grabbed_release| !grabbed_release.contains("deduplicated")),
        "duplicate URL handling must not write grabbed dedupe metadata"
    );

    let duplicate_wanted_id = if first.status == AcquisitionScopeStatus::Wanted {
        &first_wanted_id
    } else {
        &second_wanted_id
    };
    let release_decisions = wanted_items.release_decisions.lock().await.clone();
    assert!(release_decisions.iter().any(|decision| {
        decision.wanted_item_id == *duplicate_wanted_id
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
            None,
            Some(label),
            10,
            raw,
            Some("2024-01-01T00:00:00Z"),
            None,
            Default::default(),
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

/// Tracker minimums live in the indexer `extra` map, which the pending row does
/// not persist. Migration 0163 gave `pending_releases` four columns for them so
/// a delayed grab reaches the client with the same clamp inputs an immediate
/// grab gets — park time reads them off `extra`, grab time reads them back off
/// the row.
#[tokio::test]
async fn tracker_minimums_survive_the_pending_release_park_and_reach_the_grab() {
    let release_title = "Tracker.Minimums.Movie.2024.1080p-GRP";
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user, _release_attempts) =
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
        "Tracker Minimums Movie",
        2024,
    )
    .await;
    let wanted = wanted_items
        .store
        .lock()
        .await
        .iter()
        .find(|item| item.id == wanted_id)
        .cloned()
        .expect("wanted item should exist");

    // Shaped like `indexer_adapter.rs` writes them, including the stringified
    // attribute some Torznab proxies emit.
    let mut extra = std::collections::HashMap::new();
    extra.insert("minimum_seed_ratio".to_string(), serde_json::json!(1.5));
    extra.insert(
        "minimum_seed_time_minutes".to_string(),
        serde_json::json!(4320),
    );
    extra.insert("season_pack_seed_ratio".to_string(), serde_json::json!("2"));
    extra.insert(
        "season_pack_seed_time_minutes".to_string(),
        serde_json::json!(10080),
    );

    app.insert_pending_release(
        &wanted,
        &title,
        release_title,
        Some("https://example.invalid/tracker-minimums.nzb"),
        Some(DownloadSourceKind::NzbUrl),
        Some(1_000),
        1000,
        None,
        Some("test-indexer"),
        None,
        Some("tracker-minimums-guid"),
        10,
        None,
        Some("2024-01-01T00:00:00Z"),
        None,
        crate::ReleaseSeedMinimums::from_release_extra(&extra),
        crate::acquisition::seed_goals::seeders_from_extra(&extra),
    )
    .await;

    let parked = pending_releases
        .store
        .lock()
        .await
        .iter()
        .find(|release| release.release_guid.as_deref() == Some("tracker-minimums-guid"))
        .cloned()
        .expect("pending release should be parked");
    assert_eq!(parked.seed_minimums.min_seed_ratio, Some(1.5));
    assert_eq!(parked.seed_minimums.min_seed_time_minutes, Some(4320));
    assert_eq!(parked.seed_minimums.season_pack_seed_ratio, Some(2.0));
    assert_eq!(
        parked.seed_minimums.season_pack_seed_time_minutes,
        Some(10080)
    );

    let grabbed = app
        .force_grab_pending_release(&user, &parked.id)
        .await
        .expect("force grab pending release");
    assert!(grabbed);

    let submitted = download_client.submitted_seed_minimums.lock().await;
    assert_eq!(submitted.as_slice(), &[parked.seed_minimums]);
}

/// Rows parked before migration 0165 read back with every minimum `NULL`. The
/// grab must still go through — it simply falls back to the profile's own goals
/// with no tracker clamp.
#[tokio::test]
async fn pending_releases_parked_before_the_minimums_migration_still_grab() {
    let release_title = "Pre.Migration.Pending.Movie.2024.1080p-GRP";
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user, _release_attempts) =
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
        "Pre Migration Pending Movie",
        2024,
    )
    .await;
    // `pending_movie_release` mirrors the pre-0165 read-back shape: every
    // minimum `None`.
    let pending = pending_movie_release(
        &wanted_id,
        &title,
        release_title,
        PendingReleaseStatus::Waiting,
    );
    assert_eq!(pending.seed_minimums, crate::ReleaseSeedMinimums::default());
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
            .submitted_seed_minimums
            .lock()
            .await
            .as_slice(),
        &[crate::ReleaseSeedMinimums::default()]
    );
}

/// A tracker that declares no minimum, or declares a nonsense one, must not
/// park a clamp: zero and negative attributes are dropped rather than persisted
/// as a goal of zero.
#[tokio::test]
async fn non_positive_or_absent_release_minimums_are_not_parked() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user, _release_attempts) =
        bootstrap_with_acquisition_tracking_and_indexer_and_release_attempts(
            download_client.clone(),
            download_submissions,
            pending_releases.clone(),
            wanted_items.clone(),
            Arc::new(MockIndexerClient),
        );
    let (title, wanted_id) =
        seed_movie_wanted_for_acquisition(&app, &user, &wanted_items, "Junk Minimums Movie", 2024)
            .await;
    let wanted = wanted_items
        .store
        .lock()
        .await
        .iter()
        .find(|item| item.id == wanted_id)
        .cloned()
        .expect("wanted item should exist");

    let mut extra = std::collections::HashMap::new();
    extra.insert("minimum_seed_ratio".to_string(), serde_json::json!(0));
    extra.insert(
        "minimum_seed_time_minutes".to_string(),
        serde_json::json!(-1),
    );
    // `season_pack_*` absent entirely.

    app.insert_pending_release(
        &wanted,
        &title,
        "Junk.Minimums.Movie.2024.1080p-GRP",
        Some("https://example.invalid/junk-minimums.nzb"),
        Some(DownloadSourceKind::NzbUrl),
        Some(1_000),
        1000,
        None,
        Some("test-indexer"),
        None,
        Some("junk-minimums-guid"),
        10,
        None,
        Some("2024-01-01T00:00:00Z"),
        None,
        crate::ReleaseSeedMinimums::from_release_extra(&extra),
        crate::acquisition::seed_goals::seeders_from_extra(&extra),
    )
    .await;

    let parked = pending_releases
        .store
        .lock()
        .await
        .iter()
        .find(|release| release.release_guid.as_deref() == Some("junk-minimums-guid"))
        .cloned()
        .expect("pending release should be parked");
    assert_eq!(
        parked.seed_minimums,
        crate::ReleaseSeedMinimums::default(),
        "non-positive and absent tracker attributes must not become goals"
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
            wanted_item_id: wanted_id.clone(),
            title_id: title.id.clone(),
            release_title: release_title.to_string(),
            release_url: Some("https://example.invalid/pending-deferred.nzb".to_string()),
            source_kind: Some(DownloadSourceKind::NzbUrl),
            release_size_bytes: Some(1_000),
            release_score: 1000,
            scoring_log_json: None,
            indexer_source: Some("test-indexer".to_string()),
            indexer_id: None,
            release_guid: Some("pending-deferred-guid".to_string()),
            added_at: now.clone(),
            last_observed_at: now.clone(),
            delay_until: now.clone(),
            status: PendingReleaseStatus::Waiting,
            grabbed_at: None,
            source_password: None,
            published_at: Some(now),
            info_hash: None,
            seed_minimums: Default::default(),
            seeders: None,
            release_identity: "pending-deferred-guid".to_string(),
            coverage_identity: format!("scope:{wanted_id}"),
            role: crate::types::PendingReleaseRole::Primary,
            last_decision_code: None,
            release_age_unknown: false,
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
    assert!(
        title_blocklist_entries(&app, &title.id).await.is_empty(),
        "a transient pending-release submit failure must never blocklist the release"
    );
}

struct PendingStatusAssertingIndexerClient {
    pending_releases: Arc<TrackingPendingReleaseRepo>,
    searches: Arc<Mutex<Vec<String>>>,
    release_title: String,
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
        _operation: IndexerErrorOperation,
        _season: Option<u32>,
        _episode: Option<u32>,
        _absolute_episode: Option<u32>,
        _year: Option<i32>,
        _tagged_aliases: Vec<TaggedAlias>,
        _learning_context: Option<crate::IndexerSearchLearningContext>,
        _cancel_token: tokio_util::sync::CancellationToken,
    ) -> AppResult<IndexerSearchResponse> {
        let pending_is_still_waiting = self
            .pending_releases
            .store
            .lock()
            .await
            .iter()
            .any(|release| release.status == PendingReleaseStatus::Waiting);
        assert!(
            pending_is_still_waiting,
            "scheduled RSS must fetch fresh releases before deciding the merged pending set"
        );
        self.searches.lock().await.push(query.clone());

        Ok(IndexerSearchResponse {
            completion: crate::IndexerSearchCompletion::Complete,

            indexer_outcomes: Vec::new(),
            results: vec![IndexerSearchResult {
                indexer_id: None,
                source: "nzbgeek".into(),
                title: self.release_title.clone(),
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
                parsed_release_metadata: Some(crate::parse_release_metadata(&self.release_title)),
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
                coverage_scope: None,
            }],
            api_current: None,
            api_max: None,
            grab_current: None,
            grab_max: None,
        })
    }
}

#[tokio::test]
async fn scheduled_rss_fetches_before_deciding_due_pending_releases() {
    let pending_title = "Scheduled.Pending.Movie.2024.1080p.WEB-DL-GRP";
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let indexer_searches = Arc::new(Mutex::new(Vec::new()));
    let indexer_client = Arc::new(PendingStatusAssertingIndexerClient {
        pending_releases: pending_releases.clone(),
        searches: indexer_searches.clone(),
        release_title: pending_title.to_string(),
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
    let mut pending = pending_movie_release(
        &wanted_id,
        &title,
        pending_title,
        PendingReleaseStatus::Waiting,
    );
    pending.indexer_source = Some("nzbgeek".to_string());
    let pending_id = pending.id.clone();
    pending_releases
        .insert_pending_release(&pending)
        .await
        .expect("seed pending release");

    let report = app.run_scheduled_rss_sync().await.expect("run RSS sync");

    assert_eq!(
        report.releases_grabbed,
        1,
        "report={report:?}, pending={:?}, decisions={:?}",
        pending_releases.store.lock().await.as_slice(),
        wanted_items.release_decisions.lock().await.as_slice()
    );
    assert!(
        !indexer_searches.lock().await.is_empty(),
        "fresh RSS should run before the merged pending decision"
    );
    assert_eq!(
        pending_releases
            .get_pending_release(&pending_id)
            .await
            .expect("load pending release")
            .expect("pending release exists")
            .status,
        PendingReleaseStatus::Superseded
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
async fn expired_pending_release_ambiguous_error_stays_waiting_without_retry() {
    // An ambiguous submit (the request may have been accepted but the response
    // was lost) must be deferred exactly like an unavailable client: the
    // pending release stays Waiting, records a Pending (not Failed) attempt,
    // and is never blocklisted. Later cycles must not blindly repeat the
    // mutation while its acceptance remains uncertain.
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
    assert!(
        title_blocklist_entries(&app, &title.id).await.is_empty(),
        "an ambiguous submit must never blocklist the release"
    );

    download_client.set_submit_error(None).await;
    let grabbed = app
        .process_expired_pending_releases()
        .await
        .expect("retry expired pending releases");

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
    assert_eq!(
        download_client.submitted_release_titles.lock().await.len(),
        1
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
    // The definitive failure also burns the release for this title: the
    // blocklist entry (not the attempt) is what gates search, and it says why.
    let blocklist = title_blocklist_entries(&app, &title.id).await;
    assert_eq!(
        blocklist.len(),
        1,
        "expected one blocklist entry: {blocklist:?}"
    );
    assert_eq!(blocklist[0].release_name.as_str(), release_title);
    assert!(
        blocklist[0].reason.as_deref().is_some_and(|reason| {
            reason.starts_with("grab failed:")
                && reason.contains("release rejected by client routing")
        }),
        "the entry must say what happened: {:?}",
        blocklist[0].reason
    );
}

/// Park one torrent release with `seeders` reported, then promote it with the
/// floor set to `floor`. Returns the app, the pending id, and what the promoter
/// did with the row.
async fn promote_pending_torrent_with_seeders(
    release_title: &str,
    seeders: Option<i64>,
    floor: &str,
) -> (
    AppUseCase,
    User,
    Arc<TrackingPendingReleaseRepo>,
    Arc<TrackingDownloadSubmissionRepo>,
    String,
    u32,
) {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user, _release_attempts) =
        bootstrap_with_acquisition_tracking_and_indexer_and_release_attempts(
            download_client,
            download_submissions.clone(),
            pending_releases.clone(),
            wanted_items.clone(),
            Arc::new(MockIndexerClient),
        );
    let (title, wanted_id) =
        seed_movie_wanted_for_acquisition(&app, &user, &wanted_items, "Swarm Health Movie", 2024)
            .await;

    let mut pending = pending_movie_release(
        &wanted_id,
        &title,
        release_title,
        PendingReleaseStatus::Waiting,
    );
    pending.source_kind = Some(DownloadSourceKind::MagnetUri);
    pending.indexer_id = Some("acquisition-indexer".to_string());
    pending.seeders = seeders;
    let pending_id = pending.id.clone();
    pending_releases
        .insert_pending_release(&pending)
        .await
        .expect("seed pending release");

    // Raised *after* the release was parked: the whole point is that promotion
    // judges against the threshold in force now, not the one that applied then.
    app.services
        .config
        .settings
        .upsert_setting_json(
            SETTINGS_SCOPE_SYSTEM,
            MINIMUM_SEEDERS_FLOOR_SETTING_KEY,
            None,
            floor.to_string(),
            "test",
            None,
        )
        .await
        .expect("seed minimum-seeders floor");

    let grabbed = app
        .process_expired_pending_releases()
        .await
        .expect("process expired pending releases");

    (
        app,
        user,
        pending_releases,
        download_submissions,
        pending_id,
        grabbed,
    )
}

#[tokio::test]
async fn automatic_promotion_rejects_a_pending_release_below_the_current_minimum_seeders() {
    // Sonarr re-runs every specification over the pending list on each RSS sync
    // (RssSyncService.cs:42-46) using the seeders stored with the original
    // release; a swarm that died during the delay must not be grabbed.
    let (_app, _user, pending_releases, download_submissions, pending_id, grabbed) =
        promote_pending_torrent_with_seeders("Swarm.Dead.2024.1080p.WEB-DL-GRP", Some(2), "5")
            .await;

    assert_eq!(grabbed, 0);
    assert_eq!(
        pending_releases
            .get_pending_release(&pending_id)
            .await
            .expect("load pending release")
            .expect("pending release exists")
            .status,
        PendingReleaseStatus::Expired,
        "the automatic path expires what it will not grab, the same as every \
         other rejection it makes"
    );
    assert!(
        download_submissions.store.lock().await.is_empty(),
        "nothing may reach the download client"
    );
}

#[tokio::test]
async fn automatic_promotion_grabs_a_pending_release_at_the_current_minimum_seeders() {
    let (_app, _user, pending_releases, download_submissions, pending_id, grabbed) =
        promote_pending_torrent_with_seeders("Swarm.Exact.2024.1080p.WEB-DL-GRP", Some(5), "5")
            .await;

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
    assert!(!download_submissions.store.lock().await.is_empty());
}

#[tokio::test]
async fn automatic_promotion_grabs_a_pending_release_whose_seeders_are_unknown() {
    // Rows parked before migration 0169, and indexers that report no seeder
    // count, both read as unknown — and unknown is always eligible, exactly as
    // `TorrentSeedingSpecification` accepts on every ambiguity.
    let (_app, _user, pending_releases, _submissions, pending_id, grabbed) =
        promote_pending_torrent_with_seeders("Swarm.Unknown.2024.1080p.WEB-DL-GRP", None, "5")
            .await;

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
async fn an_operator_force_grab_bypasses_the_minimum_seeder_re_judge() {
    // Sonarr's manual grab runs no specifications at all: an operator who asks
    // for a release by name has overruled the automatic verdict.
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user, _release_attempts) =
        bootstrap_with_acquisition_tracking_and_indexer_and_release_attempts(
            download_client,
            download_submissions.clone(),
            pending_releases.clone(),
            wanted_items.clone(),
            Arc::new(MockIndexerClient),
        );
    let (title, wanted_id) = seed_movie_wanted_for_acquisition(
        &app,
        &user,
        &wanted_items,
        "Force Grab Swarm Movie",
        2024,
    )
    .await;
    let mut pending = pending_movie_release(
        &wanted_id,
        &title,
        "Swarm.Forced.2024.1080p.WEB-DL-GRP",
        PendingReleaseStatus::Waiting,
    );
    pending.source_kind = Some(DownloadSourceKind::MagnetUri);
    pending.indexer_id = Some("acquisition-indexer".to_string());
    pending.seeders = Some(0);
    let pending_id = pending.id.clone();
    pending_releases
        .insert_pending_release(&pending)
        .await
        .expect("seed pending release");
    app.services
        .config
        .settings
        .upsert_setting_json(
            SETTINGS_SCOPE_SYSTEM,
            MINIMUM_SEEDERS_FLOOR_SETTING_KEY,
            None,
            "5".to_string(),
            "test",
            None,
        )
        .await
        .expect("seed minimum-seeders floor");

    assert!(
        app.force_grab_pending_release(&user, &pending_id)
            .await
            .expect("force grab should succeed"),
        "the operator path must not re-judge the swarm"
    );
    assert!(!download_submissions.store.lock().await.is_empty());
}

#[tokio::test]
async fn standby_reacquisition_re_judges_the_swarm_before_grabbing() {
    // Standby recovery is an automatic grab with no operator in the loop, so it
    // applies current policy the same way delay expiry does: a stored candidate
    // whose swarm is now below the threshold is expired and the loop falls
    // through to the next one. Reacquiring into a dead swarm just fails again.
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
                name: "Standby Swarm Recovery".into(),
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
                "title": "Failed.Swarm.Release.1080p.WEB-DL",
                "score": 100,
                "grabbed_at": Utc::now().to_rfc3339(),
            })
            .to_string(),
        ),
        landed_bar: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
    };
    wanted_items
        .upsert_acquisition_scope_state(&wanted)
        .await
        .expect("seed wanted item");

    let standby = |release_title: &str, score: i32, seeders: Option<i64>| PendingRelease {
        id: Id::new().0,
        wanted_item_id: wanted.id.clone(),
        title_id: title.id.clone(),
        release_title: release_title.to_string(),
        release_url: Some(format!("https://example.com/{release_title}.torrent")),
        source_kind: Some(DownloadSourceKind::TorrentFile),
        release_size_bytes: Some(1_000),
        release_score: score,
        scoring_log_json: None,
        indexer_source: Some("torrent_rss".to_string()),
        indexer_id: Some("standby-indexer".to_string()),
        release_guid: Some(format!("guid-{release_title}")),
        added_at: Utc::now().to_rfc3339(),
        last_observed_at: Utc::now().to_rfc3339(),
        delay_until: Utc::now().to_rfc3339(),
        status: PendingReleaseStatus::Standby,
        grabbed_at: None,
        source_password: None,
        published_at: Some(Utc::now().to_rfc3339()),
        info_hash: None,
        seed_minimums: Default::default(),
        seeders,
        release_identity: format!("guid-{release_title}"),
        coverage_identity: format!("scope:{}", wanted.id),
        role: crate::types::PendingReleaseRole::Fallback,
        last_decision_code: None,
        release_age_unknown: false,
    };
    // Tried first (the test repo lists standby rows in insertion order).
    let dead = standby("Standby.Dead.Swarm.1080p.WEB-DL", 200, Some(1));
    let unknown = standby("Standby.Unknown.Swarm.1080p.WEB-DL", 150, None);
    let dead_id = dead.id.clone();
    let unknown_id = unknown.id.clone();
    pending_releases
        .insert_pending_release(&dead)
        .await
        .expect("seed dead-swarm standby");
    pending_releases
        .insert_pending_release(&unknown)
        .await
        .expect("seed unknown-swarm standby");

    // Raised after both were stored: the swarm is judged against the threshold
    // in force at recovery time.
    app.services
        .config
        .settings
        .upsert_setting_json(
            SETTINGS_SCOPE_SYSTEM,
            MINIMUM_SEEDERS_FLOOR_SETTING_KEY,
            None,
            "5".to_string(),
            "test",
            None,
        )
        .await
        .expect("seed minimum-seeders floor");

    download_submissions
        .record_submission(DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "movie".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "failed-swarm-job".to_string(),
            release_size_bytes: None,
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: Some("Failed.Swarm.Release.1080p.WEB-DL".to_string()),
            info_hash: None,
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record failed submission");

    *download_client.history_items.lock().await = vec![failed_history_item(
        "failed-swarm-job",
        "Failed.Swarm.Release.1080p.WEB-DL",
    )];

    app.run_background_acquisition_cycle_once().await;

    let stored = pending_releases.store.lock().await.clone();
    let status_of = |id: &str| {
        stored
            .iter()
            .find(|release| release.id == id)
            .map(|release| release.status)
            .expect("standby row exists")
    };
    assert_eq!(
        status_of(&dead_id),
        PendingReleaseStatus::Expired,
        "a standby below the current threshold must be expired, not reacquired"
    );
    assert_eq!(
        status_of(&unknown_id),
        PendingReleaseStatus::Grabbed,
        "an unknown seeder count stays eligible, so recovery falls through to it"
    );
    assert!(
        download_submissions
            .store
            .lock()
            .await
            .iter()
            .any(|submission| submission.source_title.as_deref()
                == Some("Standby.Unknown.Swarm.1080p.WEB-DL"))
    );
}

#[tokio::test]
async fn the_rss_park_path_stores_the_reported_seeder_count_on_the_pending_row() {
    // Binds the park site to the capture: promotion can only re-judge a swarm it
    // was told about, and every promotion test builds its row by hand, so
    // without this the four capture sites are unguarded.
    let release_title = "Rss.Delayed.Swarm.Movie.2024.1080p.WEB-DL-GRP";
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let indexer_client = Arc::new(
        FixedReleaseIndexerClient::new(release_title)
            .with_seeders(7)
            .with_published_at(Utc::now().to_rfc3339()),
    );
    let (app, user, _release_attempts) =
        bootstrap_with_acquisition_tracking_and_indexer_and_release_attempts(
            download_client,
            download_submissions.clone(),
            pending_releases.clone(),
            wanted_items.clone(),
            indexer_client,
        );
    let _title = add_rss_target_movie(&app, &user, &wanted_items, "Rss Delayed Swarm Movie").await;

    // A delay profile that holds this release, so the RSS cycle parks it instead
    // of grabbing it. The fixture's release is usenet-shaped because that is the
    // only client the acquisition harness enables; the capture under test reads
    // `extra["seeders"]` and is indifferent to the protocol, and the production
    // park site is the same single call either way.
    app.services
        .config
        .settings
        .upsert_setting_json(
            SETTINGS_SCOPE_SYSTEM,
            DELAY_PROFILE_CATALOG_KEY,
            None,
            serde_json::json!([{
                "id": "delay-torrents",
                "name": "Delay torrents",
                "usenet_delay_minutes": 120,
            }])
            .to_string(),
            "test",
            None,
        )
        .await
        .expect("seed delay profile catalog");

    let report = app.run_scheduled_rss_sync().await.expect("run RSS sync");

    assert_eq!(report.releases_grabbed, 0);
    assert_eq!(report.releases_held, 1);
    let parked = pending_releases.store.lock().await.clone();
    let row = parked
        .iter()
        .find(|release| release.release_title == release_title)
        .expect("the delayed release should have been parked");
    assert_eq!(
        row.seeders,
        Some(7),
        "the park site must persist the count the indexer reported"
    );
}

#[tokio::test]
async fn rss_treats_an_invalid_publication_timestamp_as_unknown_age() {
    let release_title = "Rss.Unknown.Age.Movie.2024.1080p.WEB-DL-GRP";
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let indexer_client = Arc::new(
        FixedReleaseIndexerClient::new(release_title).with_published_at("not-a-timestamp"),
    );
    let (app, user, _release_attempts) =
        bootstrap_with_acquisition_tracking_and_indexer_and_release_attempts(
            Arc::new(StubDownloadClient::default()),
            download_submissions.clone(),
            pending_releases.clone(),
            wanted_items.clone(),
            indexer_client,
        );
    let _title = add_rss_target_movie(&app, &user, &wanted_items, "Rss Unknown Age Movie").await;
    app.services
        .config
        .settings
        .upsert_setting_json(
            SETTINGS_SCOPE_SYSTEM,
            DELAY_PROFILE_CATALOG_KEY,
            None,
            serde_json::json!([{
                "id": "unknown-age",
                "name": "Unknown age",
                "usenet_delay_minutes": 120,
            }])
            .to_string(),
            "test",
            None,
        )
        .await
        .expect("seed delay profile catalog");

    let report = app.run_scheduled_rss_sync().await.expect("run RSS sync");

    assert_eq!(report.releases_grabbed, 0);
    assert_eq!(report.releases_held, 1);
    assert!(download_submissions.store.lock().await.is_empty());
    let parked = pending_releases.store.lock().await.clone();
    let row = parked
        .iter()
        .find(|release| release.release_title == release_title)
        .expect("the unknown-age release should have been parked");
    assert_eq!(row.published_at, None);
    assert!(row.release_age_unknown);
    assert_eq!(
        row.last_decision_code.as_deref(),
        Some("release_age_unknown")
    );
}

#[tokio::test]
async fn pending_release_grab_is_gated_by_the_blocklist_until_the_entry_is_cleared() {
    // A parked release whose title has a blocklist entry for it is rejected by
    // the pending grab gate; clearing the entry re-allows the grab immediately.
    let release_title = "Gated.Pending.Movie.2024.1080p.WEB-DL-GRP";
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user, _release_attempts) =
        bootstrap_with_acquisition_tracking_and_indexer_and_release_attempts(
            download_client.clone(),
            download_submissions.clone(),
            pending_releases.clone(),
            wanted_items.clone(),
            Arc::new(MockIndexerClient),
        );
    let (title, wanted_id) =
        seed_movie_wanted_for_acquisition(&app, &user, &wanted_items, "Gated Pending Movie", 2024)
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
    // Grab-path writers keep the indexer casing; the gate normalizes.
    app.services
        .workflow
        .blocklist_repo
        .block(&NewBlocklistEntry {
            title_id: title.id.clone(),
            release_name: release_title.to_ascii_uppercase(),
            indexer_id: String::new(),
            info_hash: None,
            reason: Some("download client failure: corrupt archive".to_string()),
        })
        .await
        .expect("seed blocklist entry");
    let entry_id = app
        .services
        .workflow
        .blocklist_repo
        .list_for_title(&title.id, 10)
        .await
        .expect("list blocklist")
        .first()
        .map(|entry| entry.id.clone())
        .expect("the seeded block is listed");

    let grabbed = app
        .force_grab_pending_release(&user, &pending_id)
        .await
        .expect("force grab pending release");
    assert!(!grabbed, "a blocklisted release must not be grabbed");
    assert!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .is_empty(),
        "the download client must never receive a blocklisted release"
    );

    app.clear_title_release_blocklist_entry(&user, &entry_id)
        .await
        .expect("clear blocklist entry");

    let grabbed = app
        .force_grab_pending_release(&user, &pending_id)
        .await
        .expect("force grab pending release after clearing the entry");
    assert!(
        grabbed,
        "clearing the entry must re-allow the grab immediately"
    );
    assert_eq!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .as_slice(),
        &[release_title.to_string()]
    );
}

/// A monitored, missing movie that RSS sync treats as an active target: no
/// pre-seeded wanted row (RSS derives target-ness from library state) and
/// `announced` availability so the undated title is not skipped as unreleased.
async fn add_rss_target_movie(
    app: &AppUseCase,
    user: &User,
    wanted_items: &Arc<TrackingAcquisitionScopeStateRepo>,
    name: &str,
) -> Title {
    let title = app
        .add_title(
            user,
            NewTitle {
                name: name.into(),
                sort_title: Some(name.into()),
                slug: Some(name.to_ascii_lowercase().replace(' ', "-")),
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
    title
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
    let title = add_rss_target_movie(&app, &user, &wanted_items, "Rss Deferred Movie").await;

    let report = app.run_scheduled_rss_sync().await.expect("run RSS sync");

    assert_eq!(report.releases_fetched, 1);
    assert_eq!(report.releases_matched, 1);
    assert_eq!(report.releases_grabbed, 0);
    assert!(download_submissions.store.lock().await.is_empty());
    let attempts = release_attempts.attempts.lock().await.clone();
    assert!(
        attempts
            .iter()
            .all(|attempt| attempt.outcome != ReleaseDownloadAttemptOutcome::Failed),
        "a transient RSS submit failure must not be recorded as failed: {:?}",
        attempts
            .iter()
            .map(|attempt| (&attempt.source_title, &attempt.outcome))
            .collect::<Vec<_>>()
    );
    let normalized_release_title = crate::normalize_release_name(Some(release_title));
    assert!(
        attempts.iter().any(|attempt| {
            attempt.source_title.as_deref() == normalized_release_title.as_deref()
                && attempt.outcome == ReleaseDownloadAttemptOutcome::Pending
                && attempt
                    .error_message
                    .as_deref()
                    .is_some_and(|message| message.contains("download client api unavailable"))
        }),
        "the RSS grab must have been attempted and recorded Pending: {:?}",
        attempts
            .iter()
            .map(|attempt| (&attempt.source_title, &attempt.outcome))
            .collect::<Vec<_>>()
    );
    let failed = release_attempts
        .list_failed_release_signatures_for_title(&title.id, 10)
        .await
        .expect("list failed signatures");
    assert!(failed.is_empty());
    assert!(
        title_blocklist_entries(&app, &title.id).await.is_empty(),
        "a transient RSS submit failure must never blocklist the release"
    );
}

#[tokio::test]
async fn rss_ambiguous_submit_error_defers_without_failed_signature_or_blocklist_entry() {
    // An ambiguous submit may have been accepted: Pending attempt, never
    // blocklisted — the same policy as the pending-release and auto-search paths.
    let release_title = "Rss.Ambiguous.Movie.2024.1080p.WEB-DL-GRP";
    let download_client = Arc::new(StubDownloadClient::default());
    download_client
        .set_submit_error(Some(StubSubmitError::Ambiguous(
            "sabnzbd addfile response was lost after the upload was sent".to_string(),
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
    let title = add_rss_target_movie(&app, &user, &wanted_items, "Rss Ambiguous Movie").await;

    let report = app.run_scheduled_rss_sync().await.expect("run RSS sync");

    assert_eq!(report.releases_matched, 1);
    assert_eq!(report.releases_grabbed, 0);
    assert!(download_submissions.store.lock().await.is_empty());
    let attempts = release_attempts.attempts.lock().await.clone();
    assert!(
        attempts
            .iter()
            .all(|attempt| attempt.outcome != ReleaseDownloadAttemptOutcome::Failed),
        "an ambiguous RSS submit must not be recorded as failed: {:?}",
        attempts
            .iter()
            .map(|attempt| (&attempt.source_title, &attempt.outcome))
            .collect::<Vec<_>>()
    );
    let normalized_release_title = crate::normalize_release_name(Some(release_title));
    assert!(attempts.iter().any(|attempt| {
        attempt.source_title.as_deref() == normalized_release_title.as_deref()
            && attempt.outcome == ReleaseDownloadAttemptOutcome::Pending
    }));
    assert!(
        title_blocklist_entries(&app, &title.id).await.is_empty(),
        "an ambiguous RSS submit must never blocklist the release"
    );
}

#[tokio::test]
async fn rss_definitive_submit_error_records_failed_signature_and_blocklist_entry() {
    let release_title = "Rss.Rejected.Movie.2024.1080p.WEB-DL-GRP";
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
    let title = add_rss_target_movie(&app, &user, &wanted_items, "Rss Rejected Movie").await;

    let report = app.run_scheduled_rss_sync().await.expect("run RSS sync");

    assert_eq!(report.releases_matched, 1);
    assert_eq!(report.releases_grabbed, 0);
    assert!(download_submissions.store.lock().await.is_empty());
    let normalized_release_title = crate::normalize_release_name(Some(release_title));
    let failed = release_attempts
        .list_failed_release_signatures_for_title(&title.id, 10)
        .await
        .expect("list failed signatures");
    assert_eq!(
        failed.len(),
        1,
        "a definitive RSS submit failure records a Failed attempt"
    );
    assert_eq!(
        failed[0].source_title.as_deref(),
        normalized_release_title.as_deref()
    );
    let blocklist = title_blocklist_entries(&app, &title.id).await;
    assert_eq!(
        blocklist.len(),
        1,
        "a definitive RSS submit failure must blocklist the release: {blocklist:?}"
    );
    assert_eq!(
        Some(blocklist[0].normalized_release_name.as_str()),
        normalized_release_title.as_deref(),
    );
    assert!(
        blocklist[0].reason.as_deref().is_some_and(|reason| {
            reason.starts_with("grab failed:") && reason.contains("Duplicate NZB")
        }),
        "the entry must say what happened: {:?}",
        blocklist[0].reason
    );
    assert!(
        !blocklist[0].indexer_id.is_empty(),
        "the entry must carry the release's download source hint"
    );
}

#[tokio::test]
async fn rss_grab_whose_submission_tracking_fails_remains_uncertain() {
    // The client accepted the job but the download submission could not be
    // persisted. The canonical coordinator retains the uncertain acceptance,
    // so RSS neither burns the release nor issues another client mutation.
    let release_title = "Rss.Untracked.Movie.2024.1080p.WEB-DL-GRP";
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    *download_submissions.record_submission_error.lock().await =
        Some("download_submissions write failed".to_string());
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
    let title = add_rss_target_movie(&app, &user, &wanted_items, "Rss Untracked Movie").await;

    let report = app.run_scheduled_rss_sync().await.expect("run RSS sync");

    assert_eq!(report.releases_matched, 1);
    assert_eq!(
        report.releases_grabbed, 0,
        "an untracked grab is not reported as grabbed"
    );
    assert_eq!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .as_slice(),
        &[release_title.to_string()],
        "the client did accept the job"
    );
    assert!(download_submissions.store.lock().await.is_empty());
    let failed = release_attempts
        .list_failed_release_signatures_for_title(&title.id, 10)
        .await
        .expect("list failed signatures");
    assert!(failed.is_empty());
    assert!(title_blocklist_entries(&app, &title.id).await.is_empty());

    let retry_report = app.run_scheduled_rss_sync().await.expect("retry RSS sync");
    assert_eq!(retry_report.releases_grabbed, 0);
    assert_eq!(
        download_client.submitted_release_titles.lock().await.len(),
        1,
        "an unresolved accepted submission must never be sent twice"
    );
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
            landed_bar: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        })
        .await
        .expect("seed Paperman wanted item");

    app.run_background_acquisition_cycle_once().await;

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
                name: "Bluey".into(),
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
            landed_bar: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        })
        .await
        .expect("seed Bluey wanted item");

    app.run_background_acquisition_cycle_once().await;

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
            landed_bar: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        })
        .await
        .expect("seed due movie wanted item");

    download_submissions
        .record_submission(DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
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
            source_title: Some("Movie.Blocking.Scope.2024.1080p.WEB-DL".to_string()),
            info_hash: None,
            release_size_bytes: None,
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
        seeding: None,
    }];

    app.run_background_acquisition_cycle_once().await;

    // An active initial title-scoped acquisition owns the empty movie scope.
    assert!(
        indexer_client.searches.lock().await.is_empty(),
        "the scope was searched while a download was in flight"
    );
    let decisions = wanted_items.release_decisions.lock().await.clone();
    assert!(decisions.is_empty());
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
            landed_bar: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        })
        .await
        .expect("seed due movie wanted item");

    app.run_background_acquisition_cycle_once().await;

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
            landed_bar: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        })
        .await
        .expect("seed due movie wanted item");

    crate::acquisition_workflow::run_background_acquisition_cycle_with_blocked_facets(
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
            landed_bar: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        })
        .await
        .expect("seed due series wanted item");

    crate::acquisition_workflow::run_background_acquisition_cycle_with_blocked_facets(
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
    assert_eq!(
        searches
            .iter()
            .filter(|search| search.season.is_none() && search.episode.is_none())
            .count(),
        0,
        "one missing episode must not trigger a title-level series-pack lookup"
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
            landed_bar: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        })
        .await
        .expect("seed due series wanted item");

    crate::acquisition_workflow::run_background_acquisition_cycle_with_blocked_facets(
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
        landed_bar: None,
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
            indexer_id: None,
            release_guid: Some("guid-standby".to_string()),
            added_at: Utc::now().to_rfc3339(),
            last_observed_at: Utc::now().to_rfc3339(),
            delay_until: Utc::now().to_rfc3339(),
            status: PendingReleaseStatus::Standby,
            grabbed_at: None,
            source_password: None,
            published_at: Some(Utc::now().to_rfc3339()),
            info_hash: None,
            seed_minimums: Default::default(),
            seeders: None,
            release_identity: "guid-standby".to_string(),
            coverage_identity: format!("scope:{}", wanted.id),
            role: crate::types::PendingReleaseRole::Fallback,
            last_decision_code: None,
            release_age_unknown: false,
        })
        .await
        .expect("seed standby");

    download_submissions
        .record_submission(DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
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
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record failed submission");

    *download_client.history_items.lock().await = vec![failed_history_item(
        "failed-job",
        "Failed.Release.1080p.WEB-DL",
    )];

    crate::acquisition_workflow::run_background_acquisition_cycle_with_blocked_facets(
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
async fn acquisition_cycle_keeps_an_old_saved_result_for_an_in_flight_grab() {
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
        status: AcquisitionScopeStatus::Grabbed,
        grabbed_release: Some(
            serde_json::json!({
                "title": "Prune.During.Scan.2024.1080p.WEB-DL",
                "score": 100,
                "grabbed_at": Utc::now().to_rfc3339(),
            })
            .to_string(),
        ),
        landed_bar: None,
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
            indexer_id: None,
            release_guid: Some("guid-stale".to_string()),
            added_at: (Utc::now() - chrono::Duration::hours(30)).to_rfc3339(),
            last_observed_at: Utc::now().to_rfc3339(),
            delay_until: Utc::now().to_rfc3339(),
            status: PendingReleaseStatus::Standby,
            grabbed_at: None,
            source_password: None,
            published_at: None,
            info_hash: None,
            seed_minimums: Default::default(),
            seeders: None,
            release_identity: "guid-stale".to_string(),
            coverage_identity: format!("scope:{}", wanted.id),
            role: crate::types::PendingReleaseRole::Fallback,
            last_decision_code: None,
            release_age_unknown: false,
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

    crate::acquisition_workflow::run_background_acquisition_cycle_with_blocked_facets(
        &app,
        &[MediaFacet::Anime],
    )
    .await;

    // Saved results have no age limit: the 30-hour-old row is still the
    // in-flight grab's next candidate, untouched by the cycle's pruning pass.
    let row = pending_releases
        .store
        .lock()
        .await
        .iter()
        .find(|release| release.release_title == "Stale.Standby.Release")
        .cloned()
        .expect("the old saved result is still there");
    assert_eq!(
        row.status,
        PendingReleaseStatus::Standby,
        "an old saved result is never aged out"
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
        landed_bar: None,
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
        landed_bar: None,
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
async fn acquisition_cycle_drops_saved_results_of_a_completed_scope() {
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
        status: AcquisitionScopeStatus::Completed,
        grabbed_release: None,
        landed_bar: None,
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
            indexer_id: None,
            release_guid: Some("guid-stale".to_string()),
            added_at: (Utc::now() - chrono::Duration::hours(30)).to_rfc3339(),
            last_observed_at: Utc::now().to_rfc3339(),
            delay_until: Utc::now().to_rfc3339(),
            status: PendingReleaseStatus::Standby,
            grabbed_at: None,
            source_password: None,
            published_at: None,
            info_hash: None,
            seed_minimums: Default::default(),
            seeders: None,
            release_identity: "guid-stale".to_string(),
            coverage_identity: format!("scope:{}", wanted.id),
            role: crate::types::PendingReleaseRole::Fallback,
            last_decision_code: None,
            release_age_unknown: false,
        })
        .await
        .expect("seed stale standby");

    app.run_background_acquisition_cycle_once().await;

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
) -> (AppUseCase, User, Arc<StoredSettingsRepo>) {
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
            &format!("\"{}\"", crate::builtin_4k_profile().id),
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
        settings.clone(),
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
                pending_releases,
                acquisition_scope_states: acquisition_scope_states.clone(),
            }))
            .with_acquisition_scope_states(acquisition_scope_states)
    });
    (app, test_admin_user(), settings)
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
        landed_bar: None,
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

    crate::import_workflow::mark_wanted_completed(&app, &title.id, None, true).await;

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

#[tokio::test]
async fn rss_library_quality_profile_overrides_global_profile() {
    let release_2160p = "Profiled.Horizon.2024.2160p.WEB-DL-GRP";
    let release_1080p = "Profiled.Horizon.2024.1080p.WEB-DL-GRP";
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let media_files = Arc::new(MockMediaFileRepo::default());
    let quality_profiles = Arc::new(StoredQualityProfileRepo::default());
    quality_profiles
        .set_profiles(vec![
            crate::builtin_4k_profile(),
            crate::builtin_1080p_profile(),
        ])
        .await;
    let indexer_client = Arc::new(MultiReleaseIndexerClient::new(vec![
        release_2160p,
        release_1080p,
    ]));
    let (app, user, settings) = bootstrap_rss_with_media_files_and_profiles(
        download_client,
        download_submissions.clone(),
        pending_releases,
        wanted_items.clone(),
        media_files,
        quality_profiles,
        indexer_client,
    )
    .await;

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Profiled Horizon".into(),
                sort_title: Some("Profiled Horizon".into()),
                slug: Some("profiled-horizon".into()),
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
    settings
        .set_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            QUALITY_PROFILE_ID_KEY,
            &title.library_id,
            "\"1080p\"",
        )
        .await;

    let report = app.run_scheduled_rss_sync().await.expect("run RSS sync");

    assert_eq!(report.releases_grabbed, 1);
    let submissions = download_submissions.store.lock().await.clone();
    assert_eq!(submissions.len(), 1);
    assert_eq!(submissions[0].source_title.as_deref(), Some(release_1080p));
}

#[tokio::test]
async fn add_and_queue_reuse_reconciles_quality_profile_before_submission() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let quality_profiles = Arc::new(StoredQualityProfileRepo::default());
    quality_profiles
        .set_profiles(vec![
            crate::builtin_4k_profile(),
            crate::builtin_1080p_profile(),
        ])
        .await;
    let (app, user, _) = bootstrap_rss_with_media_files_and_profiles(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
        wanted_items,
        Arc::new(MockMediaFileRepo::default()),
        quality_profiles,
        Arc::new(MockIndexerClient),
    )
    .await;
    let request = NewTitle {
        name: "Reconciled Queued Movie".into(),
        facet: MediaFacet::Movie,
        monitored: true,
        tags: vec!["scryer:quality-profile:4k".to_string()],
        external_ids: vec![ExternalId {
            source: "tvdb".to_string(),
            value: "765432".to_string(),
        }],
        ..Default::default()
    };
    let existing = app
        .add_title_with_outcome(&user, request.clone())
        .await
        .expect("seed 4k title");

    let outcome = app
        .add_title_and_queue_download_with_options_patch_outcome_in_library(
            &user,
            NewTitle {
                tags: vec![],
                ..request
            },
            existing.title.library_id.clone(),
            TitleOptionsPatch {
                quality_profile_id: Some(Some("1080p".to_string())),
                ..Default::default()
            },
            QueuedReleaseSelection {
                indexer_id: None,
                source_hint: Some("https://example.invalid/releases/reconciled.nzb".to_string()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Reconciled.Queued.Movie.2026.1080p.WEB-DL".to_string()),
                source_password: None,
                info_hash_hint: None,
                size_bytes: None,
                seeders: None,
            },
        )
        .await
        .expect("reused add and queue should succeed");

    assert!(outcome.reused_existing_title);
    assert_eq!(outcome.title.id, existing.title.id);
    assert!(
        outcome
            .title
            .tags
            .iter()
            .any(|tag| tag == "scryer:quality-profile:1080p")
    );
    assert!(
        !outcome
            .title
            .tags
            .iter()
            .any(|tag| tag == "scryer:quality-profile:4k")
    );
    assert_eq!(download_submissions.store.lock().await.len(), 1);
    assert_eq!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .as_slice(),
        &["Reconciled Queued Movie".to_string()]
    );
}

#[tokio::test]
async fn rss_reused_add_clears_4k_override_and_blocks_2160p_via_library_1080p() {
    let release_2160p = "Reused.Profiled.Horizon.2024.2160p.WEB-DL-GRP";
    let release_1080p = "Reused.Profiled.Horizon.2024.1080p.WEB-DL-GRP";
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let media_files = Arc::new(MockMediaFileRepo::default());
    let quality_profiles = Arc::new(StoredQualityProfileRepo::default());
    quality_profiles
        .set_profiles(vec![
            crate::builtin_4k_profile(),
            crate::builtin_1080p_profile(),
        ])
        .await;
    let indexer_client = Arc::new(MultiReleaseIndexerClient::new(vec![
        release_2160p,
        release_1080p,
    ]));
    let (app, user, settings) = bootstrap_rss_with_media_files_and_profiles(
        download_client,
        download_submissions.clone(),
        pending_releases,
        wanted_items.clone(),
        media_files,
        quality_profiles,
        indexer_client,
    )
    .await;
    let request = NewTitle {
        name: "Reused Profiled Horizon".into(),
        sort_title: Some("Reused Profiled Horizon".into()),
        slug: Some("reused-profiled-horizon".into()),
        facet: MediaFacet::Movie,
        monitored: true,
        tags: vec!["scryer:quality-profile:4k".to_string()],
        external_ids: vec![ExternalId {
            source: "tvdb".to_string(),
            value: "975310".to_string(),
        }],
        year: Some(2024),
        content_status: Some("Released".into()),
        min_availability: Some("announced".into()),
        ..Default::default()
    };
    let existing = app
        .add_title_with_outcome(&user, request.clone())
        .await
        .expect("seed 4k override");
    settings
        .set_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            QUALITY_PROFILE_ID_KEY,
            &existing.title.library_id,
            "\"1080p\"",
        )
        .await;

    let reused = app
        .add_title_with_options_patch_outcome_in_library(
            &user,
            NewTitle {
                tags: vec![],
                ..request
            },
            existing.title.library_id.clone(),
            TitleOptionsPatch {
                quality_profile_id: Some(None),
                ..Default::default()
            },
        )
        .await
        .expect("reuse should clear the explicit profile override");
    assert!(reused.reused_existing_title);
    assert_eq!(reused.title.id, existing.title.id);
    assert!(
        !reused
            .title
            .tags
            .iter()
            .any(|tag| tag.starts_with("scryer:quality-profile:"))
    );
    wanted_items
        .remember_title_facet(&reused.title.id, MediaFacet::Movie)
        .await;

    let report = app.run_scheduled_rss_sync().await.expect("run RSS sync");

    assert_eq!(report.releases_grabbed, 1);
    let submissions = download_submissions.store.lock().await.clone();
    assert_eq!(submissions.len(), 1);
    assert_eq!(submissions[0].source_title.as_deref(), Some(release_1080p));
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
    let mut cutoff_profile = crate::builtin_4k_profile();
    cutoff_profile.criteria.cutoff_tier = Some("1080P".to_string());
    quality_profiles.set_profiles(vec![cutoff_profile]).await;
    let indexer_client = Arc::new(FixedReleaseIndexerClient::new(release_title));
    let (app, user, _) = bootstrap_rss_with_media_files_and_profiles(
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
    let mut profile = crate::builtin_4k_profile();
    profile.criteria.cutoff_tier = Some("2160P".to_string());
    quality_profiles.set_profiles(vec![profile]).await;
    let indexer_client = Arc::new(FixedReleaseIndexerClient::new(release_title));
    let (app, user, _) = bootstrap_rss_with_media_files_and_profiles(
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
        _operation: IndexerErrorOperation,
        _season: Option<u32>,
        _episode: Option<u32>,
        _absolute_episode: Option<u32>,
        _year: Option<i32>,
        _tagged_aliases: Vec<TaggedAlias>,
        _learning_context: Option<crate::IndexerSearchLearningContext>,
        _cancel_token: tokio_util::sync::CancellationToken,
    ) -> AppResult<IndexerSearchResponse> {
        Ok(IndexerSearchResponse {
            completion: crate::IndexerSearchCompletion::Complete,

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
                                tier_index: Some(0),
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
                        coverage_scope: None,
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

/// Seeds the Tide Chart incident pair — two monitored library titles sharing the
/// canonical key `tide chart` — and returns the app plus the wanted scope for the
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
        ambiguous_identity_fixture_with_releases(&["Tide.Chart.1080p.WEB-DL.x264-GRP"]).await;
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
        seed_movie_wanted_for_acquisition(&app, &user, &wanted_items, "Tide Chart", 2023).await;
    // The collider: same canonical key, different work, no wanted row of its
    // own — it exists purely as library-local evidence that the name is shared.
    app.add_title(
        &user,
        NewTitle {
            name: "Tide Chart".into(),
            facet: MediaFacet::Movie,
            monitored: true,
            year: Some(1999),
            slug: Some("tide-chart-anime".into()),
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
async fn background_acquisition_parks_ambiguous_best_candidate_for_review() {
    let (app, _user, title, wanted_id, pending_releases, _release_attempts, download_client) =
        ambiguous_identity_fixture().await;

    app.run_background_acquisition_cycle_once().await;

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
    assert_eq!(parked[0].release_title, "Tide.Chart.1080p.WEB-DL.x264-GRP");
    assert!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .is_empty(),
        "an ambiguous candidate must never be submitted"
    );

    // Repeated cycles must not pile up review rows for the same scope.
    app.run_background_acquisition_cycle_once().await;
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
async fn background_acquisition_parks_ambiguous_candidate_without_skipping_eligible_release() {
    let eligible = "Tide.Chart.2023.720p.WEB-DL.AV1.AAC2.0-NTb";
    let ambiguous = "Tide.Chart.1080p.WEB-DL.x264-GRP";

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

        app.run_background_acquisition_cycle_once().await;

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
        // Results are ordered by quality tier before score (Sonarr's comparer
        // order), so the higher-resolution ambiguous release is now considered
        // — and recorded as ambiguous — before the eligible one. What matters is
        // that it is still parked rather than queued, and that the eligible
        // candidate is still recorded eligible and still the one submitted.
        assert!(
            decision_codes.contains(&format!("{eligible}:eligible")),
            "the eligible candidate must remain eligible, got {decision_codes:?}"
        );
        assert!(
            decision_codes
                .iter()
                .all(|code| code == &format!("{eligible}:eligible")
                    || code == &format!("{ambiguous}:ambiguous_identity")),
            "no candidate may be recorded with an unexpected code, got {decision_codes:?}"
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
    let eligible = "Tide.Chart.2023.720p.WEB-DL.AV1.AAC2.0-NTb";
    let ambiguous = "Tide.Chart.1080p.WEB-DL.x264-GRP";

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
            ["Tide Chart"],
            "the eligible candidate remains the queued release"
        );
    }
}

#[tokio::test]
async fn queue_best_release_materializes_missing_scope_before_parking_ambiguity() {
    let (
        app,
        user,
        title,
        _wanted_id,
        pending_releases,
        _release_attempts,
        _download_client,
        wanted_items,
    ) = ambiguous_identity_fixture_with_releases(&["Tide.Chart.1080p.WEB-DL.x264-GRP"]).await;
    wanted_items.store.lock().await.clear();

    for _ in 0..2 {
        let error = app
            .queue_best_release(
                &user,
                &title.id,
                SubmissionScope::Title,
                SubmissionConflictPolicy::Abort,
            )
            .await
            .expect_err("an ambiguous-only search has no auto-eligible release");
        assert!(error.to_string().contains("no auto-eligible release found"));
    }

    let states = wanted_items.store.lock().await.clone();
    assert_eq!(states.len(), 1, "the title scope is materialized once");
    assert_eq!(states[0].title_id, title.id);

    let parked = pending_releases
        .list_pending_releases_for_title(&title.id)
        .await
        .expect("list pending releases for title");
    assert_eq!(parked.len(), 1, "the ambiguous release is deduplicated");
    assert_eq!(parked[0].status, PendingReleaseStatus::NeedsReview);
    assert_eq!(parked[0].wanted_item_id, states[0].id);
}

#[tokio::test]
async fn needs_review_pending_release_is_never_auto_promoted() {
    let (app, _user, title, _wanted_id, pending_releases, _release_attempts, download_client) =
        ambiguous_identity_fixture().await;

    app.run_background_acquisition_cycle_once().await;
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

    app.run_background_acquisition_cycle_once().await;
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
            attempt.source_title.as_deref() == Some("Tide.Chart.1080p.WEB-DL.x264-GRP")
        }),
        "dismissing a review row must burn the release signature"
    );
    // The verdict is a hard failure: it also writes the per-title blocklist
    // entry that search-time exclusion consults (and the operator can remove).
    let blocklist = title_blocklist_entries(&app, &title.id).await;
    let entry = blocklist
        .iter()
        .find(|entry| entry.release_name == "Tide.Chart.1080p.WEB-DL.x264-GRP")
        .unwrap_or_else(|| {
            panic!("dismissing a review row must blocklist the release: {blocklist:?}")
        });
    assert_eq!(
        entry.reason.as_deref(),
        Some("dismissed from review: ambiguous identity")
    );
}

// ── TYPE-001: typed download-failover exhaustion ─────────────────────────────
//
// Retry-later is decided by the error TYPE alone. Every acquisition path below
// is exercised with the typed `DownloadSubmitFailoverExhausted` (defer: Pending
// attempt, no blocklist) and with the exact repository message the router used
// to emit before the variant existed (definitive: Failed attempt + blocklist),
// proving text no longer controls the outcome.

#[test]
fn download_submit_retryability_is_decided_by_type_not_text() {
    use crate::acquisition_decision_helpers::is_download_submit_unavailable_error;

    // Typed failover exhaustion: arbitrary, renamed, and prefixed payloads.
    for payload in [
        "",
        LEGACY_FAILOVER_REPOSITORY_MESSAGE,
        "renamed: every route was tried",
        "context: all prioritized download clients failed to enqueue this release; last client error: repository: boom",
    ] {
        assert!(
            is_download_submit_unavailable_error(&AppError::download_submit_failover_exhausted(
                payload
            )),
            "{payload:?}"
        );
    }
    assert!(is_download_submit_unavailable_error(
        &AppError::download_submit_unavailable("client submit unavailable")
    ));

    // The former repository message, wrapped renderings, and near-matches are
    // definitive failures.
    let typed_rendering =
        AppError::download_submit_failover_exhausted(LEGACY_FAILOVER_REPOSITORY_MESSAGE)
            .to_string();
    for text in [
        LEGACY_FAILOVER_REPOSITORY_MESSAGE.to_string(),
        format!("wrapped: {typed_rendering}"),
        format!("prefix {LEGACY_FAILOVER_REPOSITORY_MESSAGE}"),
        format!("{LEGACY_FAILOVER_REPOSITORY_MESSAGE} suffix"),
        "all prioritised download clients failed to enqueue this release".to_string(),
        "unrelated repository failure".to_string(),
    ] {
        assert!(
            !is_download_submit_unavailable_error(&AppError::Repository(text.clone())),
            "{text:?}"
        );
    }
    for other in [
        AppError::DownloadSubmitRejected(LEGACY_FAILOVER_REPOSITORY_MESSAGE.into()),
        AppError::DownloadSubmitAmbiguous(LEGACY_FAILOVER_REPOSITORY_MESSAGE.into()),
        AppError::Validation(LEGACY_FAILOVER_REPOSITORY_MESSAGE.into()),
        AppError::NotFound(LEGACY_FAILOVER_REPOSITORY_MESSAGE.into()),
    ] {
        assert!(!is_download_submit_unavailable_error(&other), "{other:?}");
    }

    // The variant survives the generic downgrade helper.
    assert!(matches!(
        AppError::download_submit_failover_exhausted("x").into_download_submit_unavailable(),
        AppError::DownloadSubmitFailoverExhausted(_)
    ));
}

#[test]
fn download_source_gone_is_not_retryable_or_downgraded() {
    use crate::acquisition_decision_helpers::is_download_submit_unavailable_error;

    let error = AppError::DownloadSourceGone("HTTP 410".to_string());
    assert!(error.is_download_source_gone());
    assert!(!error.is_retryable_download_submit_failure());
    assert!(!is_download_submit_unavailable_error(&error));
    assert!(matches!(
        error.into_download_submit_unavailable(),
        AppError::DownloadSourceGone(_)
    ));
}

/// Runs the auto-search (task-runner regular submission) path with the given
/// submit error and asserts the retry/blocklist decision.
async fn assert_auto_search_submit_decision(submit_error: StubSubmitError, expect_deferred: bool) {
    let release_title = "Typed.Failover.Movie.2024.1080p.WEB-DL-GRP";
    let download_client = Arc::new(StubDownloadClient::default());
    download_client.set_submit_error(Some(submit_error)).await;
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
        seed_movie_wanted_for_acquisition(&app, &user, &wanted_items, "Typed Failover Movie", 2024)
            .await;

    app.run_background_acquisition_cycle_once().await;

    assert_eq!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .as_slice(),
        &[release_title.to_string()]
    );
    assert_typed_submit_decision(
        &app,
        &release_attempts,
        &title.id,
        release_title,
        expect_deferred,
    )
    .await;
}

/// Runs the season-pack (task-runner pack submission) path.
async fn assert_season_pack_submit_decision(submit_error: StubSubmitError, expect_deferred: bool) {
    let release_title = "Typed.Failover.Pack.S01.1080p.WEB-DL-GRP";
    let download_client = Arc::new(StubDownloadClient::default());
    download_client.set_submit_error(Some(submit_error)).await;
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
        "Typed Failover Pack",
        1,
    )
    .await;

    app.run_background_acquisition_cycle_once().await;

    assert!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .iter()
            .any(|title| title == release_title),
        "season-pack branch should submit the pack title"
    );
    assert_typed_submit_decision(
        &app,
        &release_attempts,
        &title.id,
        release_title,
        expect_deferred,
    )
    .await;
}

/// Runs the pending-release (force grab) path.
async fn assert_pending_release_submit_decision(
    submit_error: StubSubmitError,
    expect_deferred: bool,
) {
    let release_title = "Typed.Failover.Pending.2024.1080p.WEB-DL-GRP";
    let download_client = Arc::new(StubDownloadClient::default());
    download_client.set_submit_error(Some(submit_error)).await;
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
        "Typed Failover Pending",
        2024,
    )
    .await;
    let pending_id = Id::new().0;
    let now = Utc::now().to_rfc3339();
    pending_releases
        .insert_pending_release(&PendingRelease {
            id: pending_id.clone(),
            wanted_item_id: wanted_id.clone(),
            title_id: title.id.clone(),
            release_title: release_title.to_string(),
            release_url: Some("https://example.invalid/typed-failover.nzb".to_string()),
            source_kind: Some(DownloadSourceKind::NzbUrl),
            release_size_bytes: Some(1_000),
            release_score: 1000,
            scoring_log_json: None,
            indexer_source: Some("test-indexer".to_string()),
            indexer_id: None,
            release_guid: Some("typed-failover-guid".to_string()),
            added_at: now.clone(),
            last_observed_at: now.clone(),
            delay_until: now.clone(),
            status: PendingReleaseStatus::Waiting,
            grabbed_at: None,
            source_password: None,
            published_at: Some(now),
            info_hash: None,
            seed_minimums: Default::default(),
            seeders: None,
            release_identity: "typed-failover-guid".to_string(),
            coverage_identity: format!("scope:{wanted_id}"),
            role: crate::types::PendingReleaseRole::Primary,
            last_decision_code: None,
            release_age_unknown: false,
        })
        .await
        .expect("seed pending release");

    let grabbed = app
        .force_grab_pending_release(&user, &pending_id)
        .await
        .expect("force grab pending release");
    assert!(!grabbed);

    let status = pending_releases
        .get_pending_release(&pending_id)
        .await
        .expect("load pending release")
        .expect("pending release exists")
        .status;
    if expect_deferred {
        assert_eq!(
            status,
            PendingReleaseStatus::Waiting,
            "typed failover exhaustion keeps the release waiting for the next cycle"
        );
    }
    assert_typed_submit_decision(
        &app,
        &release_attempts,
        &title.id,
        release_title,
        expect_deferred,
    )
    .await;
}

/// Runs the RSS sync path.
async fn assert_rss_submit_decision(submit_error: StubSubmitError, expect_deferred: bool) {
    let release_title = "Typed.Failover.Rss.2024.1080p.WEB-DL-GRP";
    let download_client = Arc::new(StubDownloadClient::default());
    download_client.set_submit_error(Some(submit_error)).await;
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
    let title = add_rss_target_movie(&app, &user, &wanted_items, "Typed Failover Rss").await;

    let report = app.run_scheduled_rss_sync().await.expect("run RSS sync");

    assert_eq!(report.releases_fetched, 1);
    assert_eq!(report.releases_matched, 1);
    assert_eq!(report.releases_grabbed, 0);
    assert!(download_submissions.store.lock().await.is_empty());
    assert_typed_submit_decision(
        &app,
        &release_attempts,
        &title.id,
        release_title,
        expect_deferred,
    )
    .await;
}

/// The shared decision every path must reach: deferred → a `Pending` attempt,
/// nothing failed, nothing blocklisted; definitive → a `Failed` signature and a
/// blocklist entry for the release.
async fn assert_typed_submit_decision(
    app: &AppUseCase,
    release_attempts: &MockReleaseAttemptRepo,
    title_id: &str,
    release_title: &str,
    expect_deferred: bool,
) {
    let normalized_release_title = crate::normalize_release_name(Some(release_title));
    let attempts = release_attempts.attempts.lock().await.clone();
    // Paths record the title raw or normalized; readers normalize, so do we.
    let outcomes = attempts
        .iter()
        .filter(|attempt| {
            crate::normalize_release_name(attempt.source_title.as_deref())
                == normalized_release_title
        })
        .map(|attempt| attempt.outcome.clone())
        .collect::<Vec<_>>();
    let failed = release_attempts
        .list_failed_release_signatures_for_title(title_id, 10)
        .await
        .expect("list failed signatures");
    let blocklist = title_blocklist_entries(app, title_id).await;
    if expect_deferred {
        assert!(
            !outcomes.is_empty()
                && outcomes
                    .iter()
                    .all(|outcome| *outcome == ReleaseDownloadAttemptOutcome::Pending),
            "typed failover exhaustion must record Pending only: {outcomes:?}"
        );
        assert!(failed.is_empty(), "{failed:?}");
        assert!(
            blocklist.is_empty(),
            "typed failover exhaustion must never blocklist: {blocklist:?}"
        );
    } else {
        assert!(
            outcomes.contains(&ReleaseDownloadAttemptOutcome::Failed),
            "the legacy failover text is a definitive failure: {outcomes:?}"
        );
        assert!(
            failed.iter().any(|entry| {
                crate::normalize_release_name(entry.source_title.as_deref())
                    == normalized_release_title
            }),
            "{failed:?}"
        );
        assert!(
            blocklist.iter().any(|entry| {
                Some(&entry.normalized_release_name) == normalized_release_title.as_ref()
            }),
            "the legacy failover text must blocklist like any definitive failure: {blocklist:?}"
        );
    }
}

fn typed_failover_exhausted() -> StubSubmitError {
    StubSubmitError::FailoverExhausted(
        "all prioritized download clients failed to enqueue this release; last client error: repository: client enqueue failed"
            .to_string(),
    )
}

fn legacy_failover_repository_text() -> StubSubmitError {
    StubSubmitError::Repository(LEGACY_FAILOVER_REPOSITORY_MESSAGE.to_string())
}

#[tokio::test]
async fn auto_search_defers_typed_failover_exhaustion() {
    assert_auto_search_submit_decision(typed_failover_exhausted(), true).await;
}

#[tokio::test]
async fn auto_search_defers_a_gone_source_without_blocklisting() {
    assert_auto_search_submit_decision(StubSubmitError::SourceGone("HTTP 410".to_string()), true)
        .await;
}

#[tokio::test]
async fn auto_search_treats_legacy_failover_text_as_definitive() {
    assert_auto_search_submit_decision(legacy_failover_repository_text(), false).await;
}

#[tokio::test]
async fn season_pack_defers_typed_failover_exhaustion() {
    assert_season_pack_submit_decision(typed_failover_exhausted(), true).await;
}

#[tokio::test]
async fn season_pack_treats_legacy_failover_text_as_definitive() {
    assert_season_pack_submit_decision(legacy_failover_repository_text(), false).await;
}

#[tokio::test]
async fn pending_release_defers_typed_failover_exhaustion() {
    assert_pending_release_submit_decision(typed_failover_exhausted(), true).await;
}

#[tokio::test]
async fn pending_release_treats_legacy_failover_text_as_definitive() {
    assert_pending_release_submit_decision(legacy_failover_repository_text(), false).await;
}

#[tokio::test]
async fn rss_defers_typed_failover_exhaustion() {
    assert_rss_submit_decision(typed_failover_exhausted(), true).await;
}

#[tokio::test]
async fn rss_treats_legacy_failover_text_as_definitive() {
    assert_rss_submit_decision(legacy_failover_repository_text(), false).await;
}

/// **D13/D20.** A parked release is re-scored against the *current* profile when
/// its delay elapses, not grabbed on the number it was parked with.
///
/// A delay profile can hold a release for hours, and the operator can edit a
/// quality profile in the meantime. The stored `release_score` was computed
/// under whatever profile, persona, rule packs and scoring algorithm were live
/// when it was parked; grabbing on it means fetching a release the library would
/// refuse if it saw it now. Sonarr re-runs its whole decision engine over
/// pending releases on every sync.
#[tokio::test]
async fn a_parked_release_the_profile_now_blocks_is_not_grabbed() {
    let download_client = Arc::new(StubDownloadClient::default());
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
                name: "Pending Rescore".into(),
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
        last_search_at: Some((Utc::now() - chrono::Duration::days(7)).to_rfc3339()),
        status: AcquisitionScopeStatus::Wanted,
        grabbed_release: None,
        landed_bar: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
    };
    wanted_items
        .upsert_acquisition_scope_state(&wanted)
        .await
        .expect("seed wanted item");

    let now = Utc::now();

    // A release the profile accepts: 1080p is in the built-in default's tiers.
    let allowed = pending_movie_release(
        &wanted.id,
        &title,
        "Pending.Rescore.2024.1080p.WEB-DL.H.264-GRP",
        PendingReleaseStatus::Waiting,
    );
    assert!(matches!(
        app.try_grab_pending_release(
            &wanted,
            &allowed,
            &now,
            crate::acquisition::pending::PendingGrabTrigger::Automatic,
        )
        .await
        .expect("pending grab should resolve"),
        crate::acquisition::pending::PendingGrabOutcome::Grabbed {
            scope: SubmissionScope::Title
        }
    ));

    // The same fixture, same generous stored `release_score`, but a quality the
    // profile does not list. Under the old code the stored number went straight
    // into admission and the release was grabbed; re-scoring finds
    // `quality_not_in_profile_tiers` and expires it.
    let blocked = pending_movie_release(
        &wanted.id,
        &title,
        "Pending.Rescore.2024.480p.WEB-DL.H.264-GRP",
        PendingReleaseStatus::Waiting,
    );
    assert_eq!(
        blocked.release_score, allowed.release_score,
        "fixture precondition: only the release name differs"
    );
    assert_eq!(
        app.try_grab_pending_release(
            &wanted,
            &blocked,
            &now,
            crate::acquisition::pending::PendingGrabTrigger::Automatic,
        )
        .await
        .expect("pending grab should resolve"),
        crate::acquisition::pending::PendingGrabOutcome::Rejected,
        "a release the current profile vetoes must expire, not be grabbed"
    );
}

// ── Plan 149 pre-release criticals ──────────────────────────────────────────

/// A season query that surfaces an episode-scoped release used to be cached and
/// substituted for the episode search, converging the episode scope on a query
/// it never ran. The season result stays reachable through standby; it no
/// longer replaces the query.
#[tokio::test]
async fn an_eligible_season_result_no_longer_replaces_the_episode_query() {
    let indexer_client = Arc::new(
        TrackingIndexerClient::default()
            .with_season_pack_titles(["Recent.Failed.Season.Pack.S07E23.1080p.WEB-DL".to_string()])
            // The substitution filtered cached candidates by routed indexer, so
            // the result has to carry the attribution a real response would.
            .stamping_indexer_ids(),
    );
    let (app, _title, indexer_client, _download_client) =
        seed_recent_failed_season_pack_fixture_with_indexer(indexer_client).await;

    app.run_background_acquisition_cycle_once().await;

    let searches = indexer_client.searches.lock().await.clone();
    assert!(
        searches
            .iter()
            .any(|search| search.season == Some(7) && search.episode == Some(23)),
        "the episode the season result covers must still spend its own query: {searches:?}"
    );
}

/// A pack the delay profile chose to *wait* on is not a pack that won. It parks
/// in `pending_releases` and leaves the episode lane free; only `AlreadyActive`
/// still suppresses.
#[tokio::test]
async fn a_delayed_season_pack_no_longer_suppresses_its_episode_search() {
    let pack = "Recent.Failed.Season.Pack.S07.1080p.WEB-DL-DELAYED".to_string();
    let indexer_client = Arc::new(
        TrackingIndexerClient::default()
            .with_season_pack_titles([pack.clone()])
            .with_published_at(Utc::now().to_rfc3339()),
    );
    let (app, _title, indexer_client, download_client) =
        seed_recent_failed_season_pack_fixture_with_indexer(indexer_client).await;
    app.services
        .config
        .settings
        .upsert_setting_json(
            SETTINGS_SCOPE_SYSTEM,
            DELAY_PROFILE_CATALOG_KEY,
            None,
            serde_json::json!([{
                "id": "delayed-pack",
                "name": "Delayed pack",
                "usenet_delay_minutes": 120,
            }])
            .to_string(),
            "test",
            None,
        )
        .await
        .expect("seed delay profile");

    app.run_background_acquisition_cycle_once().await;

    assert!(
        !download_client
            .submitted_release_titles
            .lock()
            .await
            .contains(&pack),
        "a delayed pack must not be submitted"
    );
    let searches = indexer_client.searches.lock().await.clone();
    assert!(
        searches
            .iter()
            .any(|search| search.season == Some(7) && search.episode.is_some()),
        "the delayed pack must not hold the episode lane for its whole window: {searches:?}"
    );
}

/// The walk stops after ten grab attempts. Its untried remainder used to be
/// discarded even though coverage was already recorded, leaving the scope
/// converged with no corpus.
#[tokio::test]
async fn a_walk_that_exhausts_its_grab_budget_retains_the_untried_remainder() {
    let releases = (1..=12)
        .map(|index| format!("Retention.Movie.2024.1080p.WEB-DL-G{index:02}"))
        .collect::<Vec<_>>();
    let indexer_client =
        Arc::new(TrackingIndexerClient::default().with_title_pack_titles(releases.clone()));
    let download_client = Arc::new(StubDownloadClient::default());
    download_client
        .set_submit_error(Some(StubSubmitError::Rejected("rejected".to_string())))
        .await;
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking_and_indexer(
        download_client.clone(),
        Arc::new(TrackingDownloadSubmissionRepo::default()),
        pending_releases.clone(),
        wanted_items.clone(),
        indexer_client,
    );
    let (title, _wanted_id) =
        seed_movie_wanted_for_acquisition(&app, &user, &wanted_items, "Retention Movie", 2024)
            .await;

    app.run_background_acquisition_cycle_once().await;

    let standby = pending_releases
        .list_all_standby_pending_releases()
        .await
        .expect("list standby");
    assert!(
        !standby.is_empty(),
        "a fired search that grabbed nothing must leave its remainder replayable"
    );
    let blocklisted = app.load_title_release_blocklist_signatures(&title.id).await;
    assert!(
        standby.iter().all(
            |release| !crate::app_usecase_discovery::is_release_blocklisted(
                release.indexer_id.as_deref(),
                &release.release_title,
                release.info_hash.as_deref(),
                &blocklisted,
            )
        ),
        "retention must not save releases the same walk just burned: {:?}",
        standby
            .iter()
            .map(|release| release.release_title.clone())
            .collect::<Vec<_>>()
    );
}

/// A standby row whose release is already downloading is covered *for now*.
/// Expiring it is what leaves the next failure with nothing to walk.
#[tokio::test]
async fn an_active_standby_row_survives_replay_instead_of_expiring() {
    let download_client = Arc::new(StubDownloadClient::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking(
        download_client.clone(),
        Arc::new(TrackingDownloadSubmissionRepo::default()),
        pending_releases.clone(),
        wanted_items.clone(),
    );
    let (title, wanted_id) =
        seed_movie_wanted_for_acquisition(&app, &user, &wanted_items, "Active Standby", 2024).await;
    let release_title = "Active.Standby.2024.1080p.WEB-DL-GRP";
    let standby = pending_movie_release(
        &wanted_id,
        &title,
        release_title,
        PendingReleaseStatus::Standby,
    );
    pending_releases
        .insert_pending_release(&standby)
        .await
        .expect("seed standby");
    let mut queue_item =
        queue_history_fixture_item("active-job", DownloadQueueState::Downloading, 0);
    queue_item.title_id = Some(title.id.clone());
    queue_item.title_name = release_title.to_string();
    download_client.queue_items.lock().await.push(queue_item);

    let wanted = wanted_items
        .get_acquisition_scope_state_by_id(&wanted_id)
        .await
        .expect("get wanted")
        .expect("wanted exists");
    let snapshot = crate::acquisition_workflow::DownloadClientSnapshot::fetch(&app).await;
    let outcome = crate::acquisition_workflow::try_saved_candidates(
        &app,
        &wanted,
        None,
        None,
        &snapshot,
        &Utc::now(),
    )
    .await;

    assert!(
        matches!(
            outcome,
            crate::acquisition_workflow::StandbyRecoveryOutcome::Active { .. }
        ),
        "an active release still covers the scope"
    );
    assert_eq!(
        pending_releases
            .get_pending_release(&standby.id)
            .await
            .expect("load standby")
            .expect("standby exists")
            .status,
        PendingReleaseStatus::Standby,
        "the row must survive for the failure that download may still become"
    );
}

/// The plan's headline case: every grab attempt fails because the client is
/// down. Those candidates are not bad releases, so the corpus must survive and
/// the next cycle must walk it without paying for another query.
#[tokio::test]
async fn a_client_down_cycle_retains_its_corpus_and_replays_it_next_cycle() {
    let releases = (1..=3)
        .map(|index| format!("Unavailable.Movie.2024.1080p.WEB-DL-G{index:02}"))
        .collect::<Vec<_>>();
    let indexer_client =
        Arc::new(TrackingIndexerClient::default().with_title_pack_titles(releases.clone()));
    let download_client = Arc::new(StubDownloadClient::default());
    download_client
        .set_submit_error(Some(StubSubmitError::SubmitUnavailable(
            "client down".to_string(),
        )))
        .await;
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking_and_indexer(
        download_client.clone(),
        Arc::new(TrackingDownloadSubmissionRepo::default()),
        pending_releases.clone(),
        wanted_items.clone(),
        indexer_client.clone(),
    );
    seed_movie_wanted_for_acquisition(&app, &user, &wanted_items, "Unavailable Movie", 2024).await;

    app.run_background_acquisition_cycle_once().await;

    let standby = pending_releases
        .list_all_standby_pending_releases()
        .await
        .expect("list standby");
    assert!(
        !standby.is_empty(),
        "a client-down cycle must keep the releases it could not submit"
    );

    // Client recovers. The saved corpus is walked before any indexer query.
    download_client.set_submit_error(None).await;
    indexer_client.searches.lock().await.clear();
    app.run_background_acquisition_cycle_once().await;

    assert!(
        indexer_client.searches.lock().await.is_empty(),
        "the retained corpus must be replayed without a new indexer query"
    );
    assert!(
        !download_client
            .submitted_release_titles
            .lock()
            .await
            .is_empty(),
        "the recovered client must grab from the retained corpus"
    );
}

/// Retention replaces the saved list, so a search that yields nothing keepable
/// must put back what the scope already had rather than erasing it.
#[tokio::test]
async fn a_search_with_nothing_keepable_restores_the_previous_corpus() {
    let indexer_client = Arc::new(
        TrackingIndexerClient::default()
            .with_title_pack_titles(["Totally.Unrelated.Thing.2024.1080p.WEB-DL".to_string()]),
    );
    let download_client = Arc::new(StubDownloadClient::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking_and_indexer(
        download_client.clone(),
        Arc::new(TrackingDownloadSubmissionRepo::default()),
        pending_releases.clone(),
        wanted_items.clone(),
        indexer_client,
    );
    let (title, wanted_id) =
        seed_movie_wanted_for_acquisition(&app, &user, &wanted_items, "Kept Corpus", 2024).await;
    let release_title = "Kept.Corpus.2024.1080p.WEB-DL-GRP";
    let saved = pending_movie_release(
        &wanted_id,
        &title,
        release_title,
        PendingReleaseStatus::Standby,
    );
    pending_releases
        .insert_pending_release(&saved)
        .await
        .expect("seed standby");
    // Blocklisted so the walk skips it and the search below still runs; the row
    // itself stays walkable, which is exactly the corpus that must survive.
    app.services
        .workflow
        .blocklist_repo
        .block(&NewBlocklistEntry {
            title_id: title.id.clone(),
            release_name: release_title.to_ascii_lowercase(),
            indexer_id: String::new(),
            info_hash: None,
            reason: Some("operator blocked".to_string()),
        })
        .await
        .expect("blocklist the saved release");

    app.run_background_acquisition_cycle_once().await;

    let standby = pending_releases
        .list_all_standby_pending_releases()
        .await
        .expect("list standby");
    assert!(
        standby
            .iter()
            .any(|release| release.release_title == release_title),
        "an all-rejected search must not erase the corpus the scope already had"
    );
}

/// A standby write that fails partway leaves the scope holding less than its
/// coverage claims, so the coverage has to be reopened.
#[tokio::test]
async fn a_partial_standby_write_reopens_the_scope_coverage() {
    let releases = (1..=3)
        .map(|index| format!("Partial.Movie.2024.1080p.WEB-DL-G{index:02}"))
        .collect::<Vec<_>>();
    let indexer_client = Arc::new(
        TrackingIndexerClient::default()
            .with_title_pack_titles(releases)
            .reporting_routed_indexers_fired(),
    );
    let download_client = Arc::new(StubDownloadClient::default());
    download_client
        .set_submit_error(Some(StubSubmitError::SubmitUnavailable(
            "client down".to_string(),
        )))
        .await;
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let coverage = Arc::new(RecordingScopeIndexerCoverageRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking_and_indexer(
        download_client.clone(),
        Arc::new(TrackingDownloadSubmissionRepo::default()),
        pending_releases.clone(),
        wanted_items.clone(),
        indexer_client,
    );
    let app = app
        .with_test_overrides(|builder| builder.with_scope_indexer_coverage_store(coverage.clone()));
    seed_movie_wanted_for_acquisition(&app, &user, &wanted_items, "Partial Movie", 2024).await;
    // The delete succeeds, the first insert succeeds, the rest fail.
    pending_releases.fail_standby_insert_after(1).await;

    app.run_background_acquisition_cycle_once().await;

    let remaining = coverage.recorded().await;
    assert!(
        remaining.is_empty(),
        "a partial standby write must reopen the coverage it just claimed: {remaining:?}"
    );
}

/// A release only the season query returns still has to reach the episode
/// scope. Deleting the substitution must not delete the results with it.
#[tokio::test]
async fn a_release_only_the_season_query_returns_still_reaches_the_episode() {
    let season_only = "Recent.Failed.Season.Pack.S07E23.1080p.WEB-DL-SEASONONLY".to_string();
    let indexer_client = Arc::new(
        TrackingIndexerClient::default()
            .with_season_pack_titles([season_only.clone()])
            .stamping_indexer_ids(),
    );
    let (app, _title, _indexer_client, download_client) =
        seed_recent_failed_season_pack_fixture_with_indexer(indexer_client).await;

    app.run_background_acquisition_cycle_once().await;

    let submitted = download_client
        .submitted_release_titles
        .lock()
        .await
        .clone();
    let standby = app
        .services
        .workflow
        .pending_releases
        .list_all_standby_pending_releases()
        .await
        .expect("list standby")
        .into_iter()
        .map(|release| release.release_title)
        .collect::<Vec<_>>();
    assert!(
        submitted.contains(&season_only) || standby.contains(&season_only),
        "the season-only result must be grabbable by the episode scope; \
         submitted={submitted:?} standby={standby:?}"
    );
}

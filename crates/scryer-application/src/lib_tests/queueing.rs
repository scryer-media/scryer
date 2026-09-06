use super::*;

#[tokio::test]
async fn add_title_and_queue_sends_download_job() {
    let (app, user) = bootstrap();
    let (title, job_id) = app
        .add_title_and_queue_download(
            &user,
            NewTitle {
                name: "Show One".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,

                ..Default::default()
            },
            QueuedReleaseSelection::default(),
        )
        .await
        .expect("title + queue should succeed");

    assert_eq!(job_id, format!("job-for-{}", title.id));
}

/// Proves the manual/interactive queue path reports the grab to the indexer
/// stats tracker. Deleting the `record_indexer_grab` call from
/// `catalog/workflow/queueing.rs` fails this test.
#[tokio::test]
async fn queueing_an_accepted_release_records_a_grab_for_its_indexer() {
    let (app, user, grabs) = bootstrap_with_grab_recorder();

    let (_title, _job_id) = app
        .add_title_and_queue_download(
            &user,
            NewTitle {
                name: "Grab Counted Show".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
            QueuedReleaseSelection {
                indexer_id: Some("idx-grab-counted".to_string()),
                source_hint: Some("https://indexer.test/release.nzb".to_string()),
                source_title: Some("Grab.Counted.Show.S01E01.1080p".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("title + queue should succeed");

    let recorded = grabs.lock().expect("grab log mutex").clone();
    assert_eq!(
        recorded.len(),
        1,
        "an accepted submission should record exactly one grab: {recorded:?}"
    );
    assert_eq!(recorded[0].0, "idx-grab-counted");
}

/// A submission with no indexer identity must not be bucketed under a
/// placeholder id, or the dashboard's per-indexer column stops being
/// attributable.
#[tokio::test]
async fn queueing_without_an_indexer_identity_records_no_grab() {
    let (app, user, grabs) = bootstrap_with_grab_recorder();

    app.add_title_and_queue_download(
        &user,
        NewTitle {
            name: "Unattributed Show".into(),
            facet: MediaFacet::Series,
            monitored: true,
            tags: vec![],
            external_ids: vec![],
            min_availability: None,
            ..Default::default()
        },
        QueuedReleaseSelection::default(),
    )
    .await
    .expect("title + queue should succeed");

    assert!(
        grabs.lock().expect("grab log mutex").is_empty(),
        "a submission with no indexer id must not be counted"
    );
}

#[tokio::test]
async fn add_title_with_outcome_returns_pending_and_reuses_existing_tvdb_title() {
    let (app, user) = bootstrap();
    let request = NewTitle {
        name: "Slow Hydration Movie".into(),
        facet: MediaFacet::Movie,
        monitored: true,
        tags: vec![],
        external_ids: vec![ExternalId {
            source: "tvdb".to_string(),
            value: "123456".to_string(),
        }],
        min_availability: None,
        ..Default::default()
    };

    let first = app
        .add_title_with_outcome(&user, request.clone())
        .await
        .expect("first add should succeed");
    assert_eq!(
        first.metadata_hydration_state,
        AddTitleHydrationState::Pending
    );
    assert!(!first.reused_existing_title);

    let second = app
        .add_title_with_outcome(&user, request)
        .await
        .expect("duplicate add should reuse existing title");
    assert_eq!(second.title.id, first.title.id);
    assert_eq!(
        second.metadata_hydration_state,
        AddTitleHydrationState::Pending
    );
    assert!(second.reused_existing_title);

    let titles = app
        .list_titles_unpaged(&user, Some(MediaFacet::Movie), None, None)
        .await
        .expect("titles should load");
    assert_eq!(titles.len(), 1);
}

#[tokio::test]
async fn add_title_and_queue_download_with_outcome_reuses_matching_queue_submission() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );
    let request = NewTitle {
        name: "Queued Once".into(),
        facet: MediaFacet::Movie,
        monitored: true,
        tags: vec![],
        external_ids: vec![ExternalId {
            source: "tvdb".to_string(),
            value: "654321".to_string(),
        }],
        min_availability: None,
        ..Default::default()
    };
    let queued_release = QueuedReleaseSelection {
        indexer_id: None,
        source_hint: Some("https://example.invalid/releases/queued-once.nzb".to_string()),
        source_kind: Some(DownloadSourceKind::NzbUrl),
        source_title: Some("Queued.Once.2026.1080p.WEB-DL".to_string()),
        source_password: None,
        info_hash_hint: None,
        size_bytes: None,
        seeders: None,
    };

    let first = app
        .add_title_and_queue_download_with_outcome(&user, request.clone(), queued_release.clone())
        .await
        .expect("first queued add should succeed");
    assert!(!first.reused_existing_title);
    assert!(!first.reused_queued_download);

    let second = app
        .add_title_and_queue_download_with_outcome(&user, request, queued_release)
        .await
        .expect("duplicate queued add should reuse existing queue submission");
    assert_eq!(second.title.id, first.title.id);
    assert_eq!(second.download_job_id, first.download_job_id);
    assert!(second.reused_existing_title);
    assert!(second.reused_queued_download);

    let submissions = download_submissions.store.lock().await.clone();
    let expected_signature = normalize_release_selection_signature(
        Some("https://example.invalid/releases/queued-once.nzb"),
        Some("Queued.Once.2026.1080p.WEB-DL"),
        Some(DownloadSourceKind::NzbUrl),
    );
    assert_eq!(submissions.len(), 1);
    assert_eq!(
        submissions[0].purpose,
        crate::DownloadSubmissionPurpose::OperatorQueued
    );
    assert_eq!(submissions[0].request_signature, expected_signature);
    assert_eq!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .as_slice(),
        &["Queued Once".to_string()]
    );
}

#[tokio::test]
async fn add_title_and_queue_download_records_accepted_torrent_hash_fingerprint() {
    let download_client = Arc::new(StubDownloadClient::default());
    let info_hash = "abcdef0123456789abcdef0123456789abcdef01";
    download_client.set_grab_info_hash(Some(info_hash)).await;
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client,
        download_submissions.clone(),
        pending_releases,
    );
    let request = NewTitle {
        name: "Queued Torrent".into(),
        facet: MediaFacet::Movie,
        monitored: true,
        tags: vec![],
        external_ids: vec![ExternalId {
            source: "tmdb".to_string(),
            value: "987654".to_string(),
        }],
        min_availability: None,
        ..Default::default()
    };
    let queued_release = QueuedReleaseSelection {
        indexer_id: None,
        source_hint: Some("https://example.invalid/releases/queued-torrent.torrent".to_string()),
        source_kind: Some(DownloadSourceKind::TorrentFile),
        source_title: Some("Queued.Torrent.2026.1080p.WEB-DL".to_string()),
        source_password: None,
        info_hash_hint: None,
        size_bytes: None,
        seeders: None,
    };

    app.add_title_and_queue_download_with_outcome(&user, request, queued_release)
        .await
        .expect("queued torrent add should succeed");

    let identities = download_submissions.identities.lock().await;
    assert_eq!(identities.len(), 1);
    let identity = identities.values().next().expect("submission identity");
    assert_eq!(identity.download_id.as_deref(), Some(info_hash));
}

#[tokio::test]
async fn queue_existing_title_download_reuses_matching_queue_submission() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Existing Queue".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![ExternalId {
                    source: "tvdb".to_string(),
                    value: "7654321".to_string(),
                }],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let queued_release = QueuedReleaseSelection {
        indexer_id: None,
        source_hint: Some(
            "https://example.invalid/releases/existing-queue.nzb?id=7&apikey=test-secret"
                .to_string(),
        ),
        source_kind: Some(DownloadSourceKind::NzbUrl),
        source_title: Some("Existing.Queue.2026.1080p.WEB-DL".to_string()),
        source_password: None,
        info_hash_hint: None,
        size_bytes: None,
        seeders: None,
    };

    let first = app
        .queue_existing_title_download(
            &user,
            &title.id,
            queued_release.clone(),
            SubmissionScope::Title,
            SubmissionConflictPolicy::Abort,
        )
        .await
        .expect("first queue should succeed");
    let QueueDownloadOutcome::Queued(first) = first else {
        panic!("first queue should not conflict");
    };
    *download_client.queue_items.lock().await = vec![queue_history_fixture_item(
        &first.job_id,
        DownloadQueueState::Queued,
        0,
    )];
    let second = app
        .queue_existing_title_download(
            &user,
            &title.id,
            queued_release,
            SubmissionScope::Title,
            SubmissionConflictPolicy::Abort,
        )
        .await
        .expect("second queue should reuse submission");
    let QueueDownloadOutcome::Queued(second) = second else {
        panic!("second queue should not conflict");
    };

    assert_eq!(second.job_id, first.job_id);
    assert!(second.reused_existing);
    assert_eq!(*download_client.queue_calls.lock().await, 1);
    assert_eq!(
        download_client
            .recent_activity_calls
            .lock()
            .await
            .as_slice(),
        &[100]
    );
    assert_eq!(
        download_submissions.list_for_title_calls.lock().await.len(),
        1
    );

    let submissions = download_submissions.store.lock().await.clone();
    let expected_signature = normalize_release_selection_signature(
        Some("https://example.invalid/releases/existing-queue.nzb?id=7&apikey=test-secret"),
        Some("Existing.Queue.2026.1080p.WEB-DL"),
        Some(DownloadSourceKind::NzbUrl),
    );
    assert_eq!(submissions.len(), 1);
    assert_eq!(submissions[0].title_id, title.id);
    assert_eq!(submissions[0].request_signature, expected_signature);
    assert_eq!(
        submissions[0].source_hint.as_deref(),
        Some("https://example.invalid/releases/existing-queue.nzb?id=7")
    );
    assert_eq!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .as_slice(),
        &["Existing Queue".to_string()]
    );
}

#[tokio::test]
async fn concurrent_queue_requests_for_one_title_submit_once() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Concurrent Queue".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .expect("create title");
    let release = QueuedReleaseSelection {
        source_hint: Some("https://example.invalid/releases/concurrent.nzb".to_string()),
        source_kind: Some(DownloadSourceKind::NzbUrl),
        source_title: Some("Concurrent.Queue.2026.1080p.WEB-DL".to_string()),
        ..Default::default()
    };
    let gate = Arc::new(tokio::sync::Notify::new());
    *download_client.submit_gate.lock().await = Some(gate.clone());
    let first_started = download_client.submit_started.clone().notified_owned();

    let first = tokio::spawn({
        let app = app.clone();
        let user = user.clone();
        let title_id = title.id.clone();
        let release = release.clone();
        async move {
            app.queue_existing_title_download(
                &user,
                &title_id,
                release,
                SubmissionScope::Title,
                SubmissionConflictPolicy::Abort,
            )
            .await
        }
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), first_started)
        .await
        .expect("first submission should reach the downloader gate");
    let second = tokio::spawn({
        let app = app.clone();
        let user = user.clone();
        let title_id = title.id.clone();
        async move {
            app.queue_existing_title_download(
                &user,
                &title_id,
                release,
                SubmissionScope::Title,
                SubmissionConflictPolicy::Abort,
            )
            .await
        }
    });
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    *download_client.submit_gate.lock().await = None;
    gate.notify_one();

    let first = tokio::time::timeout(std::time::Duration::from_secs(2), first)
        .await
        .expect("first queue task should complete")
        .expect("first task")
        .expect("first queue");
    let second = tokio::time::timeout(std::time::Duration::from_secs(2), second)
        .await
        .expect("second queue task should complete")
        .expect("second task")
        .expect("second queue");
    let QueueDownloadOutcome::Queued(first) = first else {
        panic!("first queue should not conflict");
    };
    let QueueDownloadOutcome::Queued(second) = second else {
        panic!("second queue should not conflict");
    };

    assert_ne!(first.reused_existing, second.reused_existing);
    assert_eq!(
        download_client.submitted_release_titles.lock().await.len(),
        1
    );
    let submissions = download_submissions.store.lock().await.clone();
    assert_eq!(submissions.len(), 1);
    assert_eq!(
        download_client.submitted_title_ids.lock().await.as_slice(),
        &[title.id]
    );
    assert_eq!(
        download_client
            .submitted_download_ids
            .lock()
            .await
            .as_slice(),
        &[Some(submissions[0].download_id)]
    );
}

#[tokio::test]
async fn concurrent_different_releases_for_one_scope_leave_the_second_as_a_conflict() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Concurrent Different Releases".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .expect("create title");
    let gate = Arc::new(tokio::sync::Notify::new());
    *download_client.submit_gate.lock().await = Some(gate.clone());
    let first_started = download_client.submit_started.clone().notified_owned();
    let first = tokio::spawn({
        let app = app.clone();
        let user = user.clone();
        let title_id = title.id.clone();
        async move {
            app.queue_existing_title_download(
                &user,
                &title_id,
                QueuedReleaseSelection {
                    source_hint: Some("https://example.invalid/releases/first.nzb".to_string()),
                    source_kind: Some(DownloadSourceKind::NzbUrl),
                    source_title: Some("First.Release.2026.1080p".to_string()),
                    ..Default::default()
                },
                SubmissionScope::Title,
                SubmissionConflictPolicy::Abort,
            )
            .await
        }
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), first_started)
        .await
        .expect("first submission should reach the downloader gate");
    let second = tokio::spawn({
        let app = app.clone();
        let user = user.clone();
        let title_id = title.id.clone();
        async move {
            app.queue_existing_title_download(
                &user,
                &title_id,
                QueuedReleaseSelection {
                    source_hint: Some("https://example.invalid/releases/second.nzb".to_string()),
                    source_kind: Some(DownloadSourceKind::NzbUrl),
                    source_title: Some("Second.Release.2026.1080p".to_string()),
                    ..Default::default()
                },
                SubmissionScope::Title,
                SubmissionConflictPolicy::Abort,
            )
            .await
        }
    });
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    *download_client.submit_gate.lock().await = None;
    gate.notify_one();

    let first = tokio::time::timeout(std::time::Duration::from_secs(2), first)
        .await
        .expect("first queue task should complete")
        .expect("first task")
        .expect("first queue");
    let second = tokio::time::timeout(std::time::Duration::from_secs(2), second)
        .await
        .expect("second queue task should complete")
        .expect("second task")
        .expect("second queue");
    assert!(matches!(first, QueueDownloadOutcome::Queued(_)));
    assert!(matches!(second, QueueDownloadOutcome::Conflict(_)));
    assert_eq!(
        download_client.submitted_release_titles.lock().await.len(),
        1
    );
    assert_eq!(download_submissions.store.lock().await.len(), 1);
}

#[tokio::test]
async fn a_settled_download_stops_conflicting_new_submissions_for_its_scope() {
    // The 30s cached submission state remembers an accepted download so a
    // repeat search cannot double-grab it while the client snapshot is stale.
    // Once that download settles, the terminal transition calls
    // `forget_settled_download`; without it, an upgrade queued inside the
    // cache window is refused as a phantom non-replaceable conflict even
    // though the first grab already imported.
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Settled Upgrade".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let first = app
        .queue_existing_title_download(
            &user,
            &title.id,
            QueuedReleaseSelection {
                source_hint: Some("https://example.invalid/releases/original.nzb".to_string()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Settled.Upgrade.2026.720p.WEB-DL".to_string()),
                ..Default::default()
            },
            SubmissionScope::Title,
            SubmissionConflictPolicy::Abort,
        )
        .await
        .expect("first queue");
    assert!(matches!(first, QueueDownloadOutcome::Queued(_)));

    // Inside the cache window the accepted set still blocks the scope: the
    // guard reports a synthetic queued, non-replaceable conflict.
    let blocked = app
        .queue_existing_title_download(
            &user,
            &title.id,
            QueuedReleaseSelection {
                source_hint: Some("https://example.invalid/releases/upgrade.nzb".to_string()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Settled.Upgrade.2026.1080p.WEB-DL".to_string()),
                ..Default::default()
            },
            SubmissionScope::Title,
            SubmissionConflictPolicy::Abort,
        )
        .await
        .expect("second queue outcome");
    let QueueDownloadOutcome::Conflict(conflict) = blocked else {
        panic!("an accepted in-flight download should conflict its scope");
    };
    assert_eq!(conflict.state, Some(DownloadQueueState::Queued));
    assert!(!conflict.replaceable);

    // The download settles: the client reports it completed, and the terminal
    // transition invalidates the guard (the production hook in
    // `finalize_tracked_terminal_state_with` calls this same method).
    let job_id = format!("job-for-{}", title.id);
    *download_client.queue_items.lock().await = vec![queue_history_fixture_item(
        &job_id,
        DownloadQueueState::Completed,
        0,
    )];
    app.runtime
        .acquisition
        .download_submission_guards
        .forget_settled_download(&title.id);

    let upgraded = app
        .queue_existing_title_download(
            &user,
            &title.id,
            QueuedReleaseSelection {
                source_hint: Some("https://example.invalid/releases/upgrade.nzb".to_string()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Settled.Upgrade.2026.1080p.WEB-DL".to_string()),
                ..Default::default()
            },
            SubmissionScope::Title,
            SubmissionConflictPolicy::Abort,
        )
        .await
        .expect("upgrade queue");
    assert!(
        matches!(upgraded, QueueDownloadOutcome::Queued(_)),
        "a settled download must not block an upgrade for its scope"
    );
    assert_eq!(
        download_client.submitted_release_titles.lock().await.len(),
        2
    );
}

#[tokio::test]
async fn queue_existing_title_download_submits_source_password_hint() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Protected Queue".into(),
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

    let outcome = app
        .queue_existing_title_download(
            &user,
            &title.id,
            QueuedReleaseSelection {
                indexer_id: None,
                source_hint: Some("https://example.invalid/releases/protected.nzb".to_string()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Protected.Queue.2026.1080p.WEB-DL".to_string()),
                source_password: Some(" archive-password ".to_string()),
                info_hash_hint: None,
                size_bytes: None,
                seeders: None,
            },
            SubmissionScope::Title,
            SubmissionConflictPolicy::Abort,
        )
        .await
        .expect("queue should succeed");
    let QueueDownloadOutcome::Queued(queued) = outcome else {
        panic!("queue should not conflict");
    };

    assert_eq!(
        queued.queued_release.source_password.as_deref(),
        Some("archive-password")
    );
    assert_eq!(
        download_client
            .submitted_source_passwords
            .lock()
            .await
            .as_slice(),
        &[Some("archive-password".to_string())]
    );
}

#[tokio::test]
async fn queue_existing_title_download_drops_source_password_flags() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );

    for (index, marker) in ["1", "true", "protected", "0", "false", "no", "  "]
        .into_iter()
        .enumerate()
    {
        let title = app
            .add_title(
                &user,
                NewTitle {
                    name: format!("Flag Queue {index}"),
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

        let outcome = app
            .queue_existing_title_download(
                &user,
                &title.id,
                QueuedReleaseSelection {
                    indexer_id: None,
                    source_hint: Some(format!("https://example.invalid/releases/flag-{index}.nzb")),
                    source_kind: Some(DownloadSourceKind::NzbUrl),
                    source_title: Some(format!("Flag.Queue.{index}.2026.1080p-WEB")),
                    source_password: Some(marker.to_string()),
                    info_hash_hint: None,
                    size_bytes: None,
                    seeders: None,
                },
                SubmissionScope::Title,
                SubmissionConflictPolicy::Abort,
            )
            .await
            .expect("queue should succeed");
        let QueueDownloadOutcome::Queued(queued) = outcome else {
            panic!("queue should not conflict");
        };
        assert_eq!(
            queued.queued_release.source_password, None,
            "marker {marker:?} should not be retained as a password"
        );
    }

    assert!(
        download_client
            .submitted_source_passwords
            .lock()
            .await
            .iter()
            .all(Option::is_none)
    );
}

#[tokio::test]
async fn queue_existing_title_download_episode_scope_records_grabbed_history_context() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) =
        bootstrap_with_cleanup_tracking(download_client, download_submissions, pending_releases);

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Episode Scope Queue".into(),
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
        .expect("create collection");
    let episode = app
        .create_episode(
            &user,
            title.id.clone(),
            Some(collection.id),
            "standard".into(),
            Some("1".into()),
            Some("1".into()),
            Some("S01E01".into()),
            Some("Queued Episode".into()),
            None,
            Some(1_500),
            false,
            false,
        )
        .await
        .expect("create episode");

    let source_hint = "https://example.invalid/releases/episode-scope-queue.nzb";
    let source_title = "Episode.Scope.Queue.S01E01.1080p.WEB-DL";
    let outcome = app
        .queue_existing_title_download(
            &user,
            &title.id,
            QueuedReleaseSelection {
                indexer_id: None,
                source_hint: Some(source_hint.to_string()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some(source_title.to_string()),
                source_password: None,
                info_hash_hint: None,
                size_bytes: None,
                seeders: None,
            },
            SubmissionScope::Episode {
                episode_id: episode.id.clone(),
            },
            SubmissionConflictPolicy::Abort,
        )
        .await
        .expect("queue episode release");
    let QueueDownloadOutcome::Queued(queued) = outcome else {
        panic!("queue should not conflict");
    };

    let events = app
        .services
        .events
        .domain_events
        .list(&DomainEventFilter {
            event_types: Some(vec![DomainEventType::ReleaseGrabbed]),
            title_id: Some(title.id.clone()),
            facet: None,
            after_sequence: Some(0),
            before_sequence: None,
            limit: 10,
        })
        .await
        .expect("release grabbed events should load");
    let grabbed = events
        .iter()
        .find_map(|event| match &event.payload {
            DomainEventPayload::ReleaseGrabbed(data) => Some(data),
            _ => None,
        })
        .expect("release grabbed event");

    assert_eq!(grabbed.source_title.as_deref(), Some(source_title));
    assert_eq!(grabbed.source_hint.as_deref(), Some(source_hint));
    assert_eq!(grabbed.download_id.as_deref(), Some(queued.job_id.as_str()));
    assert_eq!(grabbed.episode_ids, vec![episode.id]);
}

#[tokio::test]
async fn queue_existing_title_download_records_configured_provider_in_grabbed_history() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let acquisition_scope_states = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking(
        download_client,
        download_submissions.clone(),
        pending_releases,
        acquisition_scope_states,
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Provider History Queue".into(),
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
    let source_hint = "https://example.invalid/releases/provider-history.nzb";

    app.queue_existing_title_download(
        &user,
        &title.id,
        QueuedReleaseSelection {
            indexer_id: Some("acquisition-indexer".to_string()),
            source_hint: Some(source_hint.to_string()),
            source_kind: Some(DownloadSourceKind::NzbUrl),
            source_title: Some("Provider.History.2026.1080p.WEB-DL".to_string()),
            source_password: None,
            info_hash_hint: None,
            size_bytes: None,
            seeders: None,
        },
        SubmissionScope::Title,
        SubmissionConflictPolicy::Abort,
    )
    .await
    .expect("queue release");

    let events = app
        .services
        .events
        .domain_events
        .list(&DomainEventFilter {
            event_types: Some(vec![DomainEventType::ReleaseGrabbed]),
            title_id: Some(title.id.clone()),
            facet: None,
            after_sequence: Some(0),
            before_sequence: None,
            limit: 10,
        })
        .await
        .expect("release grabbed events should load");
    let grabbed = events
        .iter()
        .find_map(|event| match &event.payload {
            DomainEventPayload::ReleaseGrabbed(data) => Some(data),
            _ => None,
        })
        .expect("release grabbed event");
    assert_eq!(grabbed.source_hint.as_deref(), Some(source_hint));
    assert_eq!(
        grabbed.source_provider.as_deref(),
        Some("Synthetic newznab")
    );

    let submissions = download_submissions.store.lock().await;
    assert_eq!(submissions.len(), 1);
    assert_eq!(submissions[0].source_hint.as_deref(), Some(source_hint));
    assert_eq!(
        submissions[0].source_provider_id.as_deref(),
        Some("acquisition-indexer")
    );
    assert_eq!(
        submissions[0].source_provider_name.as_deref(),
        Some("Synthetic newznab")
    );
}

#[tokio::test]
async fn queue_existing_title_download_submit_unavailable_records_pending_without_blocklist() {
    let download_client = Arc::new(StubDownloadClient::default());
    download_client
        .set_submit_error(Some(StubSubmitError::SubmitUnavailable(
            "download client api unavailable".to_string(),
        )))
        .await;
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user, release_attempts) =
        bootstrap_with_acquisition_tracking_and_indexer_and_release_attempts(
            download_client,
            download_submissions.clone(),
            pending_releases,
            wanted_items,
            Arc::new(MockIndexerClient),
        );
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Manual Deferred Queue".into(),
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

    let error = app
        .queue_existing_title_download(
            &user,
            &title.id,
            QueuedReleaseSelection {
                indexer_id: None,
                source_hint: Some(
                    "https://example.invalid/releases/manual-deferred.nzb".to_string(),
                ),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Manual.Deferred.Queue.2026.1080p.WEB-DL".to_string()),
                source_password: None,
                info_hash_hint: None,
                size_bytes: None,
                seeders: None,
            },
            SubmissionScope::Title,
            SubmissionConflictPolicy::Abort,
        )
        .await
        .expect_err("submit unavailable should return an error to the caller");

    assert!(error.is_download_submit_unavailable());
    assert!(download_submissions.store.lock().await.is_empty());

    let attempts = release_attempts.attempts.lock().await.clone();
    assert!(
        attempts
            .iter()
            .all(|attempt| attempt.outcome != ReleaseDownloadAttemptOutcome::Failed),
        "manual submit-unavailable attempts must not be recorded as failed: {:?}",
        attempts
            .iter()
            .map(|attempt| (&attempt.source_title, &attempt.outcome))
            .collect::<Vec<_>>()
    );
    assert!(attempts.iter().any(|attempt| {
        attempt.source_title.as_deref() == Some("Manual.Deferred.Queue.2026.1080p.WEB-DL")
            && attempt.outcome == ReleaseDownloadAttemptOutcome::Pending
            && attempt
                .error_message
                .as_deref()
                .is_some_and(|message| message.contains("download client api unavailable"))
    }));
    let failed = release_attempts
        .list_failed_release_signatures_for_title(&title.id, 10)
        .await
        .expect("list failed signatures");
    assert!(failed.is_empty());

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
async fn queue_existing_title_download_definitive_submit_error_records_failed_and_blocklists() {
    let download_client = Arc::new(StubDownloadClient::default());
    download_client
        .set_submit_error(Some(StubSubmitError::Rejected(
            "sabnzbd rejected the nzb: Duplicate NZB".to_string(),
        )))
        .await;
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user, release_attempts) =
        bootstrap_with_acquisition_tracking_and_indexer_and_release_attempts(
            download_client,
            download_submissions.clone(),
            pending_releases,
            wanted_items,
            Arc::new(MockIndexerClient),
        );
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Manual Rejected Queue".into(),
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

    let error = app
        .queue_existing_title_download(
            &user,
            &title.id,
            QueuedReleaseSelection {
                indexer_id: None,
                source_hint: Some(
                    "https://example.invalid/releases/manual-rejected.nzb".to_string(),
                ),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Manual.Rejected.Queue.2026.1080p.WEB-DL".to_string()),
                source_password: None,
                info_hash_hint: None,
                size_bytes: None,
                seeders: None,
            },
            SubmissionScope::Title,
            SubmissionConflictPolicy::Abort,
        )
        .await
        .expect_err("a rejected submit should return an error to the caller");
    assert!(error.to_string().contains("Duplicate NZB"));
    assert!(download_submissions.store.lock().await.is_empty());

    let failed = release_attempts
        .list_failed_release_signatures_for_title(&title.id, 10)
        .await
        .expect("list failed signatures");
    assert_eq!(failed.len(), 1);
    assert_eq!(
        failed[0].source_title.as_deref(),
        Some("Manual.Rejected.Queue.2026.1080p.WEB-DL")
    );
    let blocklist = title_blocklist_entries(&app, &title.id).await;
    assert_eq!(
        blocklist.len(),
        1,
        "a definitive interactive submit failure must blocklist the release: {blocklist:?}"
    );
    assert_eq!(
        blocklist[0].release_name.as_str(),
        "Manual.Rejected.Queue.2026.1080p.WEB-DL"
    );
    assert_eq!(
        blocklist[0].normalized_release_name.as_str(),
        "manual.rejected.queue.2026.1080p.web-dl"
    );
    assert!(
        blocklist[0]
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("Duplicate NZB")),
        "the entry must say what happened: {:?}",
        blocklist[0].reason
    );
}

#[tokio::test]
async fn queue_existing_title_download_whose_submission_tracking_fails_remains_uncertain() {
    // The client accepted the job but the download submission could not be
    // persisted. The title-wide uncertain claim prevents a duplicate while a
    // later request retries persistence without another client mutation.
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    *download_submissions.record_submission_error.lock().await =
        Some("download_submissions write failed".to_string());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user, release_attempts) =
        bootstrap_with_acquisition_tracking_and_indexer_and_release_attempts(
            download_client.clone(),
            download_submissions.clone(),
            pending_releases,
            wanted_items,
            Arc::new(MockIndexerClient),
        );
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Manual Untracked Queue".into(),
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

    let error = app
        .queue_existing_title_download(
            &user,
            &title.id,
            QueuedReleaseSelection {
                indexer_id: None,
                source_hint: Some(
                    "https://example.invalid/releases/manual-untracked.nzb".to_string(),
                ),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Manual.Untracked.Queue.2026.1080p.WEB-DL".to_string()),
                source_password: None,
                info_hash_hint: None,
                size_bytes: None,
                seeders: None,
            },
            SubmissionScope::Title,
            SubmissionConflictPolicy::Abort,
        )
        .await
        .expect_err("a persistence failure should surface to the caller");
    assert!(
        error
            .to_string()
            .contains("download_submissions write failed")
    );
    assert_eq!(
        download_client.submitted_release_titles.lock().await.len(),
        1,
        "the client did accept the job"
    );
    assert!(download_submissions.store.lock().await.is_empty());

    let failed = release_attempts
        .list_failed_release_signatures_for_title(&title.id, 10)
        .await
        .expect("list failed signatures");
    assert!(failed.is_empty());
    let blocklist = title_blocklist_entries(&app, &title.id).await;
    assert!(blocklist.is_empty());

    *download_submissions.record_submission_error.lock().await = None;
    download_client.queue_items.lock().await.clear();
    let recovered = app
        .queue_existing_title_download(
            &user,
            &title.id,
            QueuedReleaseSelection {
                source_hint: Some(
                    "https://example.invalid/releases/manual-untracked.nzb".to_string(),
                ),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Manual.Untracked.Queue.2026.1080p.WEB-DL".to_string()),
                ..Default::default()
            },
            SubmissionScope::Title,
            SubmissionConflictPolicy::Abort,
        )
        .await
        .expect("the accepted mutation should become durable without another submit");
    let QueueDownloadOutcome::Queued(recovered) = recovered else {
        panic!("the recovered submission should be returned as queued");
    };
    assert!(recovered.reused_existing);
    assert_eq!(
        download_client.submitted_release_titles.lock().await.len(),
        1
    );
    assert_eq!(download_submissions.store.lock().await.len(), 1);
}

#[tokio::test]
async fn queue_existing_title_download_adopts_same_title_client_identity() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Adopted Queue".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .expect("create title");
    let existing_download_id = scryer_domain::download_identity::DownloadId::new();
    let existing_job_id = format!("job-for-{}", title.id);
    download_submissions
        .record_submission(DownloadSubmission {
            download_id: existing_download_id,
            title_id: title.id.clone(),
            facet: title.facet.as_str().to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: existing_job_id.clone(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: None,
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            purpose: crate::DownloadSubmissionPurpose::Standard,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record client-created seed binding");
    download_client
        .set_snapshot_authoritative_client_ids(["primary".to_string()])
        .await;

    let outcome = app
        .queue_existing_title_download(
            &user,
            &title.id,
            QueuedReleaseSelection {
                source_hint: Some("https://example.invalid/adopted.nzb".to_string()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Adopted.Queue.2026.1080p.WEB-DL".to_string()),
                ..Default::default()
            },
            SubmissionScope::Title,
            SubmissionConflictPolicy::Abort,
        )
        .await
        .expect("same-title client deduplication should reuse the canonical submission");
    let QueueDownloadOutcome::Queued(queued) = outcome else {
        panic!("adopted submission should be returned as queued");
    };

    assert!(queued.reused_existing);
    let submissions = download_submissions.store.lock().await;
    assert_eq!(submissions.len(), 1);
    assert_eq!(submissions[0].download_id, existing_download_id);
    assert_eq!(submissions[0].title_id, title.id);
    assert!(!submissions[0].download_client_item_id.is_empty());
    assert!(submissions[0].request_signature.is_some());
    drop(submissions);
    assert_eq!(download_client.submitted_download_ids.lock().await.len(), 1);
}

#[tokio::test]
async fn queue_existing_title_download_rejects_cross_title_client_identity() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );
    let owner = app
        .add_title(
            &user,
            NewTitle {
                name: "Canonical Owner".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .expect("create canonical owner");
    let contender = app
        .add_title(
            &user,
            NewTitle {
                name: "Canonical Contender".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .expect("create contender");
    let existing_download_id = scryer_domain::download_identity::DownloadId::new();
    let existing_job_id = format!("job-for-{}", contender.id);
    download_submissions
        .record_submission(DownloadSubmission {
            download_id: existing_download_id,
            title_id: owner.id.clone(),
            facet: owner.facet.as_str().to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: existing_job_id.clone(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: None,
            source_title: None,
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            purpose: crate::DownloadSubmissionPurpose::Standard,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record canonical owner submission");
    let error = app
        .queue_existing_title_download(
            &user,
            &contender.id,
            QueuedReleaseSelection {
                source_hint: Some("https://example.invalid/cross-title.nzb".to_string()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Cross.Title.2026.1080p.WEB-DL".to_string()),
                ..Default::default()
            },
            SubmissionScope::Title,
            SubmissionConflictPolicy::Abort,
        )
        .await
        .expect_err("cross-title canonical adoption must be rejected");

    assert!(matches!(error, AppError::DownloadSubmitRejected(_)));
    let submissions = download_submissions.store.lock().await;
    assert_eq!(submissions.len(), 1);
    assert_eq!(submissions[0].download_id, existing_download_id);
    assert_eq!(submissions[0].title_id, owner.id);
    assert!(!submissions[0].download_client_item_id.is_empty());
}

#[tokio::test]
async fn queue_existing_title_download_ignores_stale_matching_submission() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Stale Queue".into(),
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

    let queued_release = QueuedReleaseSelection {
        indexer_id: None,
        source_hint: Some("https://example.invalid/releases/stale-queue.nzb".to_string()),
        source_kind: Some(DownloadSourceKind::NzbUrl),
        source_title: Some("Stale.Queue.2026.1080p.WEB-DL".to_string()),
        source_password: None,
        info_hash_hint: None,
        size_bytes: None,
        seeders: None,
    };

    app.queue_existing_title_download(
        &user,
        &title.id,
        queued_release.clone(),
        SubmissionScope::Title,
        SubmissionConflictPolicy::Abort,
    )
    .await
    .expect("first queue should succeed");
    download_client.queue_items.lock().await.clear();
    download_submissions.store.lock().await[0].download_client_id = Some("primary".to_string());
    download_client
        .set_snapshot_authoritative_client_ids(["primary".to_string()])
        .await;

    let second = app
        .queue_existing_title_download(
            &user,
            &title.id,
            queued_release,
            SubmissionScope::Title,
            SubmissionConflictPolicy::Abort,
        )
        .await
        .expect("stale signature should not be reused");
    let QueueDownloadOutcome::Queued(second) = second else {
        panic!("stale signature should queue again");
    };

    assert!(!second.reused_existing);
    assert_eq!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .as_slice(),
        &["Stale Queue".to_string(), "Stale Queue".to_string()]
    );
}

#[tokio::test]
async fn queue_existing_title_download_requires_the_relevant_client_to_be_authoritative() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Partial Snapshot".into(),
                facet: MediaFacet::Movie,
                monitored: true,
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
            facet: "movie".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "missing-primary-job".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: Some(DownloadSourceKind::NzbUrl),
            source_title: Some("First.Release.2026.1080p".to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record prior submission");
    download_client
        .set_snapshot_authoritative_client_ids(["secondary".to_string()])
        .await;

    let error = app
        .queue_existing_title_download(
            &user,
            &title.id,
            QueuedReleaseSelection {
                source_hint: Some("https://example.invalid/second.nzb".to_string()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Second.Release.2026.1080p".to_string()),
                ..Default::default()
            },
            SubmissionScope::Title,
            SubmissionConflictPolicy::Abort,
        )
        .await
        .expect_err("another client's successful snapshot cannot prove absence");
    assert!(matches!(error, AppError::DownloadSubmitUnavailable(_)));
    assert!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .is_empty()
    );

    download_client
        .set_snapshot_authoritative_client_ids(["primary".to_string()])
        .await;
    app.queue_existing_title_download(
        &user,
        &title.id,
        QueuedReleaseSelection {
            source_hint: Some("https://example.invalid/second.nzb".to_string()),
            source_kind: Some(DownloadSourceKind::NzbUrl),
            source_title: Some("Second.Release.2026.1080p".to_string()),
            ..Default::default()
        },
        SubmissionScope::Title,
        SubmissionConflictPolicy::Abort,
    )
    .await
    .expect("authoritative absence should permit the new submission");
    assert_eq!(
        download_client.submitted_release_titles.lock().await.len(),
        1
    );
}

#[tokio::test]
async fn queue_existing_title_download_requires_authority_to_trust_a_terminal_item() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Partial Terminal Snapshot".into(),
                facet: MediaFacet::Movie,
                monitored: true,
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
            facet: "movie".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "failed-primary-job".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: Some(DownloadSourceKind::NzbUrl),
            source_title: Some("Failed.Release.2026.1080p".to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record prior submission");
    let mut failed_item =
        queue_history_fixture_item("failed-primary-job", DownloadQueueState::Failed, 0);
    failed_item.client_id = "primary".to_string();
    download_client.history_items.lock().await.push(failed_item);

    let selection = QueuedReleaseSelection {
        source_hint: Some("https://example.invalid/replacement.nzb".to_string()),
        source_kind: Some(DownloadSourceKind::NzbUrl),
        source_title: Some("Replacement.Release.2026.1080p".to_string()),
        ..Default::default()
    };
    download_client
        .set_snapshot_authoritative_client_ids(["secondary".to_string()])
        .await;
    let error = app
        .queue_existing_title_download(
            &user,
            &title.id,
            selection.clone(),
            SubmissionScope::Title,
            SubmissionConflictPolicy::Abort,
        )
        .await
        .expect_err("a terminal item from a partial client snapshot is not authoritative");
    assert!(matches!(error, AppError::DownloadSubmitUnavailable(_)));
    assert!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .is_empty()
    );

    download_client
        .set_snapshot_authoritative_client_ids(["primary".to_string()])
        .await;
    app.queue_existing_title_download(
        &user,
        &title.id,
        selection,
        SubmissionScope::Title,
        SubmissionConflictPolicy::Abort,
    )
    .await
    .expect("an authoritative terminal item permits a replacement submission");
    assert_eq!(
        download_client.submitted_release_titles.lock().await.len(),
        1
    );
}

#[tokio::test]
async fn queue_existing_title_download_blocks_a_durable_unbound_submission() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Unbound Submission".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .expect("create title");
    download_submissions
        .record_ambiguous_submission(DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "movie".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: String::new(),
            source_hint: Some("https://example.invalid/first.nzb".to_string()),
            source_provider_id: None,
            source_provider_name: None,
            source_kind: Some(DownloadSourceKind::NzbUrl),
            source_title: Some("First.Release.2026.1080p".to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: Some("first-signature".to_string()),
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record ambiguous submission");

    let error = app
        .queue_existing_title_download(
            &user,
            &title.id,
            QueuedReleaseSelection {
                source_hint: Some("https://example.invalid/second.nzb".to_string()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Second.Release.2026.1080p".to_string()),
                ..Default::default()
            },
            SubmissionScope::Title,
            SubmissionConflictPolicy::Abort,
        )
        .await
        .expect_err("unresolved acceptance must block another mutation");
    assert!(error.is_download_submit_ambiguous());
    assert!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn queue_existing_title_download_reports_scope_conflict() {
    // A warned download is still live in the client and is never cleaned up on
    // its own, so it has to block a duplicate grab exactly like a downloading
    // one — and stay replaceable, which is the operator's way out of a torrent
    // that is stuck. Sonarr's QueueSpecification skips only FailedPending.
    for state in [DownloadQueueState::Downloading, DownloadQueueState::Warning] {
        queue_existing_title_download_conflicts_for_state(state).await;
    }
}

async fn queue_existing_title_download_conflicts_for_state(state: DownloadQueueState) {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Blocked Queue".into(),
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
    download_submissions
        .record_submission(DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "movie".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "existing-job".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: Some(DownloadSourceKind::NzbUrl),
            source_title: Some("Blocked.Queue.2026.1080p.WEB-DL".to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record submission");
    *download_client.queue_items.lock().await =
        vec![queue_history_fixture_item("existing-job", state, 0)];

    let outcome = app
        .queue_existing_title_download(
            &user,
            &title.id,
            QueuedReleaseSelection {
                indexer_id: None,
                source_hint: Some("https://example.invalid/replacement.nzb".to_string()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Blocked.Queue.Replacement.2026.1080p.WEB-DL".to_string()),
                source_password: None,
                info_hash_hint: None,
                size_bytes: None,
                seeders: None,
            },
            SubmissionScope::Title,
            SubmissionConflictPolicy::Abort,
        )
        .await
        .expect("conflict should be returned as outcome");

    let QueueDownloadOutcome::Conflict(conflict) = outcome else {
        panic!("queue should conflict for {state:?}");
    };
    assert_eq!(
        conflict.download_client_item_id, "existing-job",
        "{state:?}"
    );
    assert!(conflict.replaceable, "{state:?}");
    assert!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .is_empty(),
        "{state:?} must not be duplicated"
    );
}

#[tokio::test]
async fn a_warned_download_still_counts_as_active_in_the_client_snapshot() {
    // The double-submit guard reads the client's own queue states. A warned
    // download is live work, so the automatic paths must see it as active
    // rather than searching for a replacement behind its back.
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, _user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );
    *download_client.queue_items.lock().await = vec![queue_history_fixture_item(
        "warned-job",
        DownloadQueueState::Warning,
        0,
    )];

    let snapshot = crate::acquisition_workflow::DownloadClientSnapshot::fetch(&app).await;

    // An unobservable queue answers "active" to everything, so the assertion
    // below would pass for the wrong reason without this guard.
    assert!(!snapshot.queue_listing_failed());
    assert!(snapshot.is_active("Fixture warned-job"));
}

#[tokio::test]
async fn queue_existing_title_download_additional_file_ignores_standard_blocker() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Additional Queue".into(),
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
    download_submissions
        .record_submission(DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "movie".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "existing-standard-job".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: Some(DownloadSourceKind::NzbUrl),
            source_title: Some("Additional.Queue.2026.1080p.WEB-DL".to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record standard submission");
    *download_client.queue_items.lock().await = vec![queue_history_fixture_item(
        "existing-standard-job",
        DownloadQueueState::Downloading,
        0,
    )];

    let outcome = app
        .queue_existing_title_download_with_purpose(
            &user,
            &title.id,
            QueuedReleaseSelection {
                indexer_id: None,
                source_hint: Some("https://example.invalid/directors-cut.nzb".to_string()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Additional.Queue.Directors.Cut.2026.1080p.WEB-DL".to_string()),
                source_password: None,
                info_hash_hint: None,
                size_bytes: None,
                seeders: None,
            },
            SubmissionScope::Title,
            SubmissionConflictPolicy::Abort,
            crate::DownloadSubmissionPurpose::AdditionalFile,
        )
        .await
        .expect("additional file queue should bypass standard blocker");

    let QueueDownloadOutcome::Queued(queued) = outcome else {
        panic!("additional file queue should not conflict with standard blocker");
    };
    assert!(!queued.reused_existing);
    assert_eq!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .as_slice(),
        &["Additional Queue".to_string()]
    );
    let submissions = download_submissions.store.lock().await.clone();
    assert_eq!(submissions.len(), 2);
    assert!(
        submissions
            .iter()
            .any(|submission| submission.purpose == crate::DownloadSubmissionPurpose::Standard)
    );
    assert!(submissions.iter().any(|submission| {
        submission.purpose == crate::DownloadSubmissionPurpose::AdditionalFile
            && submission.request_signature.is_some()
    }));
}

#[tokio::test]
async fn queue_existing_title_download_additional_file_supports_series_movie_scope() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Additional Series Movie".into(),
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
    let link = app
        .services
        .catalog
        .shows
        .upsert_series_movie_link(test_series_movie_link(
            &title.id,
            "Additional Series Movie: The Movie",
            Some(2026),
            None,
            Some("additional-series-movie"),
        ))
        .await
        .expect("create series movie link");
    let scope = SubmissionScope::SeriesMovie {
        series_movie_link_id: link.id.clone(),
    };
    download_submissions
        .record_submission(DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "anime".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "existing-series-movie-job".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: Some(DownloadSourceKind::NzbUrl),
            source_title: Some("Additional.Series.Movie.2026.1080p.WEB-DL".to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: scope.clone(),
        })
        .await
        .expect("record standard submission");
    *download_client.queue_items.lock().await = vec![queue_history_fixture_item(
        "existing-series-movie-job",
        DownloadQueueState::Downloading,
        0,
    )];

    let outcome = app
        .queue_existing_title_download_with_purpose(
            &user,
            &title.id,
            QueuedReleaseSelection {
                indexer_id: None,
                source_hint: Some("https://example.invalid/series-movie-extra.nzb".to_string()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some(
                    "Additional.Series.Movie.Commentary.2026.1080p.WEB-DL".to_string(),
                ),
                source_password: None,
                info_hash_hint: None,
                size_bytes: None,
                seeders: None,
            },
            scope.clone(),
            SubmissionConflictPolicy::Abort,
            crate::DownloadSubmissionPurpose::AdditionalFile,
        )
        .await
        .expect("additional file queue should allow series movie scope");

    let QueueDownloadOutcome::Queued(queued) = outcome else {
        panic!("additional series movie file queue should not conflict");
    };
    assert!(!queued.reused_existing);
    let submissions = download_submissions.store.lock().await.clone();
    assert_eq!(submissions.len(), 2);
    assert!(submissions.iter().any(|submission| {
        submission.purpose == crate::DownloadSubmissionPurpose::AdditionalFile
            && submission.scope == scope
            && submission.request_signature.is_some()
    }));
}

#[tokio::test]
async fn queue_existing_title_download_additional_file_dedupes_by_scope() {
    let download_client = Arc::new(StubDownloadClient::default().with_unique_job_ids());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Additional Episode Dedupe".into(),
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
    let queued_release = QueuedReleaseSelection {
        indexer_id: None,
        source_hint: Some("https://example.invalid/same-release.nzb".to_string()),
        source_kind: Some(DownloadSourceKind::NzbUrl),
        source_title: Some("Additional.Episode.Dedupe.S01E01.1080p.WEB-DL".to_string()),
        source_password: None,
        info_hash_hint: None,
        size_bytes: None,
        seeders: None,
    };

    for episode_id in ["episode-1", "episode-2"] {
        let outcome = app
            .queue_existing_title_download_with_purpose(
                &user,
                &title.id,
                queued_release.clone(),
                SubmissionScope::Episode {
                    episode_id: episode_id.to_string(),
                },
                SubmissionConflictPolicy::Abort,
                crate::DownloadSubmissionPurpose::AdditionalFile,
            )
            .await
            .expect("additional file queue should allow distinct episode scopes");
        let QueueDownloadOutcome::Queued(queued) = outcome else {
            panic!("additional file queue should not conflict");
        };
        assert!(
            !queued.reused_existing,
            "episode {episode_id} should queue independently"
        );
    }

    assert_eq!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .as_slice(),
        &["Additional Episode Dedupe", "Additional Episode Dedupe"]
    );
    let submissions = download_submissions.store.lock().await.clone();
    assert_eq!(submissions.len(), 2);
    assert!(submissions.iter().all(|submission| {
        submission.purpose == crate::DownloadSubmissionPurpose::AdditionalFile
    }));
    assert!(submissions.iter().any(|submission| {
        submission.scope
            == SubmissionScope::Episode {
                episode_id: "episode-1".to_string(),
            }
    }));
    assert!(submissions.iter().any(|submission| {
        submission.scope
            == SubmissionScope::Episode {
                episode_id: "episode-2".to_string(),
            }
    }));
    assert_eq!(
        submissions
            .iter()
            .filter_map(|submission| submission.request_signature.as_deref())
            .collect::<std::collections::HashSet<_>>()
            .len(),
        1,
        "same release should keep the same release signature"
    );
}

#[tokio::test]
async fn queue_existing_title_download_additional_file_rejects_collection_scope() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Additional Collection Reject".into(),
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

    let error = app
        .queue_existing_title_download_with_purpose(
            &user,
            &title.id,
            QueuedReleaseSelection {
                indexer_id: None,
                source_hint: Some("https://example.invalid/season-pack.nzb".to_string()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Additional.Collection.Reject.S01.1080p.WEB-DL".to_string()),
                source_password: None,
                info_hash_hint: None,
                size_bytes: None,
                seeders: None,
            },
            SubmissionScope::Collection {
                collection_id: "season-1".to_string(),
            },
            SubmissionConflictPolicy::Abort,
            crate::DownloadSubmissionPurpose::AdditionalFile,
        )
        .await
        .expect_err("collection scope should be rejected for additional files");

    assert!(
        error
            .to_string()
            .contains("additional-file queueing does not support collection scopes yet")
    );
    assert!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .is_empty()
    );
    assert!(download_submissions.store.lock().await.is_empty());
}

#[tokio::test]
async fn queue_existing_title_download_additional_file_rejects_non_movie_title_scopes() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );

    for (facet, name) in [
        (MediaFacet::Series, "Additional Series Reject"),
        (MediaFacet::Anime, "Additional Anime Reject"),
    ] {
        let title = app
            .add_title(
                &user,
                NewTitle {
                    name: name.into(),
                    facet,
                    monitored: true,
                    tags: vec![],
                    external_ids: vec![],
                    min_availability: None,
                    ..Default::default()
                },
            )
            .await
            .expect("create title");

        let error = app
            .queue_existing_title_download_with_purpose(
                &user,
                &title.id,
                QueuedReleaseSelection {
                    indexer_id: None,
                    source_hint: Some("https://example.invalid/title-scope.nzb".to_string()),
                    source_kind: Some(DownloadSourceKind::NzbUrl),
                    source_title: Some(format!("{}.2026.1080p.WEB-DL", name.replace(' ', "."))),
                    source_password: None,
                    info_hash_hint: None,
                    size_bytes: None,
                    seeders: None,
                },
                SubmissionScope::Title,
                SubmissionConflictPolicy::Abort,
                crate::DownloadSubmissionPurpose::AdditionalFile,
            )
            .await
            .expect_err("non-movie title scope should be rejected for additional files");

        assert!(
            error
                .to_string()
                .contains("additional-file title queueing supports only movie titles")
        );
    }

    assert!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .is_empty()
    );
    assert!(download_submissions.store.lock().await.is_empty());
}

#[tokio::test]
async fn queue_existing_title_download_additional_file_rejects_non_single_episode_scopes() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Additional Episode Scope Reject".into(),
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

    for (scope, expected) in [
        (
            SubmissionScope::EpisodeSet {
                episode_ids: vec!["episode-1".to_string(), "episode-2".to_string()],
            },
            "additional-file queueing supports only title and single-episode scopes",
        ),
        (
            SubmissionScope::Orphan,
            "additional-file queueing requires a title or episode scope",
        ),
    ] {
        let error = app
            .queue_existing_title_download_with_purpose(
                &user,
                &title.id,
                QueuedReleaseSelection {
                    indexer_id: None,
                    source_hint: Some("https://example.invalid/episode-pack.nzb".to_string()),
                    source_kind: Some(DownloadSourceKind::NzbUrl),
                    source_title: Some(
                        "Additional.Episode.Scope.Reject.S01.1080p.WEB-DL".to_string(),
                    ),
                    source_password: None,
                    info_hash_hint: None,
                    size_bytes: None,
                    seeders: None,
                },
                scope,
                SubmissionConflictPolicy::Abort,
                crate::DownloadSubmissionPurpose::AdditionalFile,
            )
            .await
            .expect_err("unsupported scope should be rejected for additional files");

        assert!(error.to_string().contains(expected));
    }

    assert!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .is_empty()
    );
    assert!(download_submissions.store.lock().await.is_empty());
}

#[tokio::test]
async fn queue_existing_title_download_replace_early_deletes_old_submission() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Replace Queue".into(),
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
    download_submissions
        .record_submission(DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "movie".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "old-job".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: Some(DownloadSourceKind::NzbUrl),
            source_title: Some("Replace.Queue.2026.1080p.WEB-DL".to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record submission");
    *download_client.queue_items.lock().await = vec![queue_history_fixture_item(
        "old-job",
        DownloadQueueState::Queued,
        0,
    )];

    let outcome = app
        .queue_existing_title_download(
            &user,
            &title.id,
            QueuedReleaseSelection {
                indexer_id: None,
                source_hint: Some("https://example.invalid/new.nzb".to_string()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Replace.Queue.New.2026.1080p.WEB-DL".to_string()),
                source_password: None,
                info_hash_hint: None,
                size_bytes: None,
                seeders: None,
            },
            SubmissionScope::Title,
            SubmissionConflictPolicy::ReplaceEarly,
        )
        .await
        .expect("replacement should succeed");

    let QueueDownloadOutcome::Queued(outcome) = outcome else {
        panic!("replacement should queue");
    };
    assert_eq!(outcome.job_id, format!("job-for-{}", title.id));
    assert_eq!(
        download_client.deleted_items.lock().await.as_slice(),
        &[("old-job".to_string(), false)]
    );
    let submissions = download_submissions.store.lock().await.clone();
    assert_eq!(submissions.len(), 1);
    assert_eq!(submissions[0].download_client_item_id, outcome.job_id);
}

#[tokio::test]
async fn queue_existing_title_download_replace_early_deletes_all_blockers() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Replace All Queue".into(),
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
    for job_id in ["old-job-a", "old-job-b"] {
        download_submissions
            .record_submission(DownloadSubmission {
                download_id: scryer_domain::download_identity::DownloadId::new(),
                title_id: title.id.clone(),
                purpose: crate::DownloadSubmissionPurpose::Standard,
                facet: "movie".to_string(),
                download_client_id: Some("primary".to_string()),
                download_client_type: "nzbget".to_string(),
                download_client_item_id: job_id.to_string(),
                source_hint: None,
                source_provider_id: None,
                source_provider_name: None,
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some(format!("Replace.All.Queue.{job_id}.2026.1080p.WEB-DL")),
                info_hash: None,
                release_size_bytes: None,
                request_signature: None,
                scope: SubmissionScope::Title,
            })
            .await
            .expect("record submission");
    }
    *download_client.queue_items.lock().await = vec![
        queue_history_fixture_item("old-job-a", DownloadQueueState::Queued, 0),
        queue_history_fixture_item("old-job-b", DownloadQueueState::Downloading, 0),
    ];

    let outcome = app
        .queue_existing_title_download(
            &user,
            &title.id,
            QueuedReleaseSelection {
                indexer_id: None,
                source_hint: Some("https://example.invalid/new-all.nzb".to_string()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Replace.All.Queue.New.2026.1080p.WEB-DL".to_string()),
                source_password: None,
                info_hash_hint: None,
                size_bytes: None,
                seeders: None,
            },
            SubmissionScope::Title,
            SubmissionConflictPolicy::ReplaceEarly,
        )
        .await
        .expect("replacement should succeed");

    let QueueDownloadOutcome::Queued(outcome) = outcome else {
        panic!("replacement should queue");
    };
    let mut deleted_items = download_client.deleted_items.lock().await.clone();
    deleted_items.sort();
    assert_eq!(
        deleted_items,
        vec![
            ("old-job-a".to_string(), false),
            ("old-job-b".to_string(), false),
        ]
    );
    let submissions = download_submissions.store.lock().await.clone();
    assert_eq!(submissions.len(), 1);
    assert_eq!(submissions[0].download_client_item_id, outcome.job_id);
}

#[tokio::test]
async fn commit_successful_grab_marks_covered_wanted_set_and_supersedes_pending_releases() {
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let repo = TrackingAcquisitionStateRepo {
        pending_releases: pending_releases.clone(),
        acquisition_scope_states: wanted_items.clone(),
    };
    let now = Utc::now().to_rfc3339();
    let title_id = "covered-title";
    let wanted_a = AcquisitionScopeState {
        id: "wanted-a".to_string(),
        title_id: title_id.to_string(),
        title_name: Some("Covered Title".to_string()),
        title_slug: None,
        title_facet: None,
        library_id: None,
        library_name: None,
        library_slug: None,
        episode_id: Some("episode-a".to_string()),
        collection_id: Some("season-1".to_string()),
        series_movie_link_id: None,
        season_number: Some("1".to_string()),
        episode_number: None,
        media_type: "series".to_string(),
        last_search_at: None,
        status: AcquisitionScopeStatus::Wanted,
        grabbed_release: None,
        landed_bar: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: now.clone(),
        updated_at: now.clone(),
    };
    let wanted_b = AcquisitionScopeState {
        id: "wanted-b".to_string(),
        episode_id: Some("episode-b".to_string()),
        ..wanted_a.clone()
    };
    let wanted_c = AcquisitionScopeState {
        id: "wanted-c".to_string(),
        episode_id: Some("episode-c".to_string()),
        ..wanted_a.clone()
    };
    for wanted in [&wanted_a, &wanted_b, &wanted_c] {
        wanted_items
            .upsert_acquisition_scope_state(wanted)
            .await
            .expect("seed wanted item");
    }

    for (id, wanted_item_id, status) in [
        ("pending-grabbed", "wanted-a", PendingReleaseStatus::Waiting),
        (
            "pending-a-sibling",
            "wanted-a",
            PendingReleaseStatus::Waiting,
        ),
        (
            "pending-b-waiting",
            "wanted-b",
            PendingReleaseStatus::Waiting,
        ),
        (
            "pending-b-standby",
            "wanted-b",
            PendingReleaseStatus::Standby,
        ),
        (
            "pending-c-uncovered",
            "wanted-c",
            PendingReleaseStatus::Waiting,
        ),
    ] {
        pending_releases
            .insert_pending_release(&PendingRelease {
                id: id.to_string(),
                wanted_item_id: wanted_item_id.to_string(),
                title_id: title_id.to_string(),
                release_title: format!("{id}.1080p.WEB-DL"),
                release_url: Some(format!("https://example.invalid/{id}.nzb")),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                release_size_bytes: Some(1_000),
                release_score: 100,
                scoring_log_json: None,
                indexer_source: Some("test-indexer".to_string()),
                indexer_id: None,
                release_guid: Some(format!("guid-{id}")),
                added_at: now.clone(),
                last_observed_at: now.clone(),
                delay_until: now.clone(),
                status,
                grabbed_at: None,
                source_password: None,
                published_at: Some(now.clone()),
                info_hash: None,
                seed_minimums: Default::default(),
                seeders: None,
                release_identity: format!("guid-{id}"),
                coverage_identity: format!("scope:{wanted_item_id}"),
                role: match status {
                    PendingReleaseStatus::Waiting => crate::types::PendingReleaseRole::Primary,
                    _ => crate::types::PendingReleaseRole::Fallback,
                },
                last_decision_code: None,
                release_age_unknown: false,
            })
            .await
            .expect("seed pending release");
    }

    repo.commit_successful_grab(&SuccessfulGrabCommit {
        wanted_item_id: wanted_a.id.clone(),
        covered_wanted_item_ids: vec![wanted_b.id.clone()],
        grabbed_release: "{\"title\":\"Covered.Release.1080p.WEB-DL\"}".to_string(),
        last_search_at: Some(now.clone()),
        grabbed_pending_release_id: Some("pending-grabbed".to_string()),
        grabbed_at: Some(now),
    })
    .await
    .expect("commit successful grab");

    let wanted_store = wanted_items.store.lock().await.clone();
    let status_for = |id: &str| {
        wanted_store
            .iter()
            .find(|wanted| wanted.id == id)
            .map(|wanted| wanted.status)
            .expect("wanted item exists")
    };
    assert_eq!(status_for("wanted-a"), AcquisitionScopeStatus::Grabbed);
    assert_eq!(status_for("wanted-b"), AcquisitionScopeStatus::Grabbed);
    assert_eq!(status_for("wanted-c"), AcquisitionScopeStatus::Wanted);

    let pending_store = pending_releases.store.lock().await.clone();
    let pending_status_for = |id: &str| {
        pending_store
            .iter()
            .find(|release| release.id == id)
            .map(|release| release.status)
            .expect("pending release exists")
    };
    assert_eq!(
        pending_status_for("pending-grabbed"),
        PendingReleaseStatus::Grabbed
    );
    assert_eq!(
        pending_status_for("pending-a-sibling"),
        PendingReleaseStatus::Superseded
    );
    assert_eq!(
        pending_status_for("pending-b-waiting"),
        PendingReleaseStatus::Superseded
    );
    assert_eq!(
        pending_status_for("pending-b-standby"),
        // Saved search results survive a sibling grab: they are the fallback if
        // that grab fails.
        PendingReleaseStatus::Standby
    );
    assert_eq!(
        pending_status_for("pending-c-uncovered"),
        PendingReleaseStatus::Waiting
    );
}

#[tokio::test]
async fn trigger_title_wanted_search_conflicts_before_seeding_movie_wanted_item() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Blocked Wanted Movie".into(),
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
    download_submissions
        .record_submission(DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "movie".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "movie-job".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: Some(DownloadSourceKind::NzbUrl),
            source_title: Some("Blocked.Wanted.Movie.2026.1080p.WEB-DL".to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record submission");
    *download_client.queue_items.lock().await = vec![queue_history_fixture_item(
        "movie-job",
        DownloadQueueState::Downloading,
        0,
    )];

    let outcome = app
        .trigger_title_wanted_search(&user, &title.id, SubmissionConflictPolicy::Abort)
        .await
        .expect("wanted search should return conflict");

    assert_eq!(outcome.queued_count, 0);
    assert_eq!(outcome.skipped_in_progress_count, 0);
    assert_eq!(
        outcome
            .conflict
            .as_ref()
            .map(|conflict| conflict.download_client_item_id.as_str()),
        Some("movie-job")
    );
    assert!(
        app.services
            .workflow
            .acquisition_scope_states
            .list_acquisition_scope_states(AcquisitionScopeStatesQuery {
                title_id: Some(title.id.clone()),
                limit: 100,
                ..AcquisitionScopeStatesQuery::default()
            })
            .await
            .expect("list wanted items")
            .is_empty()
    );
}

#[tokio::test]
async fn trigger_title_wanted_search_skips_conflicted_first_seed_episode_items() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Blocked Wanted Series".into(),
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
    let collection = app
        .create_collection(
            &user,
            title.id.clone(),
            "season".into(),
            "1".into(),
            Some("Season One".into()),
            None,
            Some("1".into()),
            Some("1".into()),
        )
        .await
        .expect("create collection");
    let episode = app
        .create_episode(
            &user,
            title.id.clone(),
            Some(collection.id.clone()),
            "standard".into(),
            Some("1".into()),
            Some("1".into()),
            Some("Pilot".into()),
            Some("Pilot".into()),
            None,
            Some(1_200),
            false,
            false,
        )
        .await
        .expect("create episode");

    download_submissions
        .record_submission(DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "series".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "episode-job".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: Some(DownloadSourceKind::NzbUrl),
            source_title: Some("Blocked.Wanted.Series.S01E01.1080p.WEB-DL".to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: SubmissionScope::Episode {
                episode_id: episode.id.clone(),
            },
        })
        .await
        .expect("record submission");
    *download_client.queue_items.lock().await = vec![queue_history_fixture_item(
        "episode-job",
        DownloadQueueState::Downloading,
        0,
    )];

    let outcome = app
        .trigger_title_wanted_search(&user, &title.id, SubmissionConflictPolicy::Abort)
        .await
        .expect("wanted search should skip blocked episode");

    assert_eq!(outcome.queued_count, 0);
    assert_eq!(outcome.skipped_in_progress_count, 1);
    assert_eq!(
        outcome
            .conflict
            .as_ref()
            .map(|conflict| conflict.download_client_item_id.as_str()),
        Some("episode-job")
    );
    let wanted_items = app
        .services
        .workflow
        .acquisition_scope_states
        .list_acquisition_scope_states(AcquisitionScopeStatesQuery {
            title_id: Some(title.id.clone()),
            limit: 100,
            ..AcquisitionScopeStatesQuery::default()
        })
        .await
        .expect("list wanted items");
    assert!(wanted_items.is_empty());
}

#[tokio::test]
async fn queue_replacement_release_from_candidate_token_marks_manual_replacement() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, admin) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );

    app.create_download_client_config(
        &admin,
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
            &admin,
            NewTitle {
                name: "Token Queue".into(),
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

    let (_created, authenticated_user) = create_authenticated_user(
        &app,
        &admin,
        "token_queue_user",
        "password123",
        vec![
            TestPermissionPreset::CatalogView,
            TestPermissionPreset::TitleManagement,
        ],
    )
    .await;

    let selection = QueuedReleaseSelection {
        indexer_id: None,
        source_hint: Some("https://example.invalid/token-queue.nzb".to_string()),
        source_kind: Some(DownloadSourceKind::NzbUrl),
        source_title: Some("Token.Queue.2026.1080p.WEB-DL".to_string()),
        source_password: None,
        info_hash_hint: Some("abcdef0123456789abcdef0123456789abcdef01".to_string()),
        size_bytes: None,
        seeders: None,
    };
    let candidate_token = app
        .issue_release_candidate_token(
            &authenticated_user,
            &title.id,
            &SubmissionScope::Title,
            &selection,
        )
        .await
        .expect("issue candidate token");

    let outcome = app
        .queue_replacement_release_from_candidate_token(
            &authenticated_user,
            &title.id,
            &candidate_token,
            SubmissionConflictPolicy::Abort,
            None,
        )
        .await
        .expect("queue replacement release from candidate token");
    let QueueDownloadOutcome::Queued(outcome) = outcome else {
        panic!("replacement queue should not conflict");
    };

    assert_eq!(outcome.job_id, format!("job-for-{}", title.id));
    assert_eq!(outcome.queued_release, selection);
    assert_eq!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .as_slice(),
        &["Token Queue".to_string()]
    );
    assert_eq!(
        download_client
            .submitted_info_hash_hints
            .lock()
            .await
            .as_slice(),
        &[Some("abcdef0123456789abcdef0123456789abcdef01".to_string())]
    );
    let submissions = download_submissions.store.lock().await.clone();
    assert_eq!(submissions.len(), 1);
    assert_eq!(
        submissions[0].purpose,
        crate::DownloadSubmissionPurpose::ManualReplacement
    );
}

#[tokio::test]
async fn queue_existing_title_download_additional_file_uses_signed_candidate_scope() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, admin) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );

    app.create_download_client_config(
        &admin,
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
            &admin,
            NewTitle {
                name: "Signed Episode Queue".into(),
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
    let (_created, authenticated_user) = create_authenticated_user(
        &app,
        &admin,
        "signed_episode_queue_user",
        "password123",
        vec![
            TestPermissionPreset::CatalogView,
            TestPermissionPreset::TitleManagement,
        ],
    )
    .await;
    let selection = QueuedReleaseSelection {
        indexer_id: None,
        source_hint: Some("https://example.invalid/signed-episode.nzb".to_string()),
        source_kind: Some(DownloadSourceKind::NzbUrl),
        source_title: Some("Signed.Episode.Queue.S01E01.1080p.WEB-DL".to_string()),
        source_password: None,
        info_hash_hint: None,
        size_bytes: None,
        seeders: None,
    };
    let signed_scope = SubmissionScope::Episode {
        episode_id: "episode-1".to_string(),
    };
    let candidate_token = app
        .issue_release_candidate_token(&authenticated_user, &title.id, &signed_scope, &selection)
        .await
        .expect("issue candidate token");

    let outcome = app
        .queue_existing_title_download_from_candidate_token_with_purpose(
            &authenticated_user,
            &title.id,
            &candidate_token,
            SubmissionScope::Collection {
                collection_id: "season-1".to_string(),
            },
            SubmissionConflictPolicy::Abort,
            crate::DownloadSubmissionPurpose::AdditionalFile,
            None,
        )
        .await
        .expect("signed single-episode scope should allow additional queue");
    let QueueDownloadOutcome::Queued(outcome) = outcome else {
        panic!("signed single-episode additional queue should not conflict");
    };

    assert_eq!(outcome.job_id, format!("job-for-{}", title.id));
    assert_eq!(outcome.queued_release, selection);
    assert_eq!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .as_slice(),
        &["Signed Episode Queue".to_string()]
    );
    let submissions = download_submissions.store.lock().await.clone();
    assert_eq!(submissions.len(), 1);
    assert_eq!(
        submissions[0].purpose,
        crate::DownloadSubmissionPurpose::AdditionalFile
    );
    assert_eq!(submissions[0].scope, signed_scope);
}

#[tokio::test]
async fn queue_existing_title_download_additional_file_rejects_signed_episode_set_scope() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, admin) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );

    let title = app
        .add_title(
            &admin,
            NewTitle {
                name: "Signed Episode Set Reject".into(),
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
    let (_created, authenticated_user) = create_authenticated_user(
        &app,
        &admin,
        "signed_episode_set_reject_user",
        "password123",
        vec![
            TestPermissionPreset::CatalogView,
            TestPermissionPreset::TitleManagement,
        ],
    )
    .await;
    let selection = QueuedReleaseSelection {
        indexer_id: None,
        source_hint: Some("https://example.invalid/signed-episode-set.nzb".to_string()),
        source_kind: Some(DownloadSourceKind::NzbUrl),
        source_title: Some("Signed.Episode.Set.Reject.S01.1080p.WEB-DL".to_string()),
        source_password: None,
        info_hash_hint: None,
        size_bytes: None,
        seeders: None,
    };
    let candidate_token = app
        .issue_release_candidate_token(
            &authenticated_user,
            &title.id,
            &SubmissionScope::EpisodeSet {
                episode_ids: vec!["episode-1".to_string(), "episode-2".to_string()],
            },
            &selection,
        )
        .await
        .expect("issue candidate token");

    let error = app
        .queue_existing_title_download_from_candidate_token_with_purpose(
            &authenticated_user,
            &title.id,
            &candidate_token,
            SubmissionScope::Episode {
                episode_id: "episode-1".to_string(),
            },
            SubmissionConflictPolicy::Abort,
            crate::DownloadSubmissionPurpose::AdditionalFile,
            None,
        )
        .await
        .expect_err("signed episode-set scope should be rejected for additional queue");

    assert!(
        error
            .to_string()
            .contains("additional-file queueing supports only title and single-episode scopes")
    );
    assert!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .is_empty()
    );
    assert!(download_submissions.store.lock().await.is_empty());
}

#[tokio::test]
async fn queue_best_release_prefers_first_auto_eligible_candidate() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let indexer_client = Arc::new(
        MultiReleaseIndexerClient::new(vec![
            "Wrong.Show.2026.1080p.WEB-DL",
            "Target.Show.2026.1080p.WEB-DL",
        ])
        .with_info_hash_hint("abcdef0123456789abcdef0123456789abcdef01"),
    );
    let (app, user) = bootstrap_with_cleanup_tracking_and_indexer(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
        indexer_client,
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
                name: "Target Show".into(),
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

    let job_id = app
        .queue_best_release(
            &user,
            &title.id,
            SubmissionScope::Title,
            SubmissionConflictPolicy::Abort,
        )
        .await
        .expect("queue best release");
    let QueueDownloadOutcome::Queued(job_id) = job_id else {
        panic!("best release should not conflict");
    };

    assert_eq!(job_id.job_id, format!("job-for-{}", title.id));
    assert_eq!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .clone(),
        vec!["Target Show".to_string()]
    );
    assert_eq!(
        download_client
            .submitted_info_hash_hints
            .lock()
            .await
            .as_slice(),
        &[Some("abcdef0123456789abcdef0123456789abcdef01".to_string())]
    );

    let submissions = download_submissions.store.lock().await.clone();
    assert_eq!(submissions.len(), 1);
    assert_eq!(
        submissions[0].source_title.as_deref(),
        Some("Target.Show.2026.1080p.WEB-DL")
    );
    assert_eq!(
        submissions[0].request_signature,
        crate::helpers::normalize_release_selection_signature(
            Some("https://example.invalid/download/1.nzb"),
            Some("Target.Show.2026.1080p.WEB-DL"),
            Some(DownloadSourceKind::NzbUrl),
        )
    );
}

#[tokio::test]
async fn queue_best_release_reports_auto_eligibility_reason_counts() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let indexer_client = Arc::new(MultiReleaseIndexerClient::new(vec![
        "Wrong.Show.2026.1080p.WEB-DL",
        "Other.Show.2026.720p.WEB-DL",
    ]));
    let (app, user) = bootstrap_with_cleanup_tracking_and_indexer(
        download_client.clone(),
        download_submissions,
        pending_releases,
        indexer_client,
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
                name: "Target Show".into(),
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

    let error = app
        .queue_best_release(
            &user,
            &title.id,
            SubmissionScope::Title,
            SubmissionConflictPolicy::Abort,
        )
        .await
        .expect_err("mismatched releases should not be auto-eligible");
    let crate::AppError::NoAutoEligibleRelease {
        candidate_count,
        reasons,
    } = error
    else {
        panic!("expected auto-eligibility diagnostics");
    };

    assert_eq!(candidate_count, 2);
    assert_eq!(
        reasons,
        vec![crate::AutoEligibilityReason {
            code: "title_mismatch".to_string(),
            summary: "release title does not match the target title".to_string(),
            count: 2,
            block_codes: Vec::new(),
        }]
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
async fn queue_best_release_reports_zero_auto_candidates() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let indexer_client = Arc::new(MultiReleaseIndexerClient::new(vec![]));
    let (app, user) = bootstrap_with_cleanup_tracking_and_indexer(
        download_client.clone(),
        download_submissions,
        pending_releases,
        indexer_client,
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Target Show".into(),
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

    let error = app
        .queue_best_release(
            &user,
            &title.id,
            SubmissionScope::Title,
            SubmissionConflictPolicy::Abort,
        )
        .await
        .expect_err("an empty search should not have an auto-eligible release");
    let crate::AppError::NoAutoEligibleRelease {
        candidate_count,
        reasons,
    } = error
    else {
        panic!("expected auto-eligibility diagnostics");
    };

    assert_eq!(candidate_count, 0);
    assert!(reasons.is_empty());
    assert!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn queue_best_release_supports_series_movie_scope() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let indexer_client = Arc::new(MultiReleaseIndexerClient::new(vec![
        "Wrong.Show.2024.1080p.WEB-DL",
        "Movie.1.2024.1080p.WEB-DL",
    ]));
    let (app, user) = bootstrap_with_cleanup_tracking_and_indexer(
        download_client,
        download_submissions.clone(),
        pending_releases,
        indexer_client,
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
                name: "Parent Series".into(),
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

    let link = app
        .services
        .catalog
        .shows
        .upsert_series_movie_link(test_series_movie_link(
            &title.id,
            "Movie 1",
            Some(2024),
            None,
            Some("movie-1"),
        ))
        .await
        .expect("create series movie link");

    let job_id = app
        .queue_best_release(
            &user,
            &title.id,
            SubmissionScope::SeriesMovie {
                series_movie_link_id: link.id.clone(),
            },
            SubmissionConflictPolicy::Abort,
        )
        .await
        .expect("queue best release for series movie");
    let QueueDownloadOutcome::Queued(job_id) = job_id else {
        panic!("best release should not conflict");
    };

    assert_eq!(job_id.job_id, format!("job-for-{}", title.id));

    let submissions = download_submissions.store.lock().await.clone();
    assert_eq!(submissions.len(), 1);
    assert_eq!(
        submissions[0].source_title.as_deref(),
        Some("Movie.1.2024.1080p.WEB-DL")
    );
    assert_eq!(
        submissions[0].scope,
        SubmissionScope::SeriesMovie {
            series_movie_link_id: link.id
        }
    );
}

#[tokio::test]
async fn resolve_release_search_subject_for_series_movie_uses_movie_entity_metadata() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let indexer_client = Arc::new(FixedReleaseIndexerClient::new(
        "Iron.Rail.2020.1080p.WEB-DL",
    ));
    let (app, user) = bootstrap_with_search_settings_and_indexer(settings, indexer_client);

    let mut title = app
        .add_title(
            &user,
            NewTitle {
                name: "Ember Saga".into(),
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
    title.aliases = vec!["Kage no Kotoba".to_string()];

    let mut link_input = test_series_movie_link(
        &title.id,
        "Ember Saga -Kage no Kotoba- The Movie: Iron Rail",
        Some(2020),
        Some("tt11032374"),
        Some("12345"),
    );
    link_input.movie.tmdb_id = Some("635302".to_string());
    link_input.movie.anidb_id = Some("15400".to_string());
    link_input.movie.mal_id = Some("40456".to_string());
    let link = app
        .services
        .catalog
        .shows
        .upsert_series_movie_link(link_input)
        .await
        .expect("create series movie link");

    let (search_title, subject) = app
        .resolve_release_search_subject_for_series_movie(&title, &link)
        .await
        .expect("resolve series movie subject");

    assert_eq!(
        search_title.name,
        "Ember Saga -Kage no Kotoba- The Movie: Iron Rail"
    );
    assert_eq!(search_title.year, Some(2020));
    assert_eq!(search_title.imdb_id.as_deref(), Some("tt11032374"));
    assert_eq!(subject.queries.len(), 1);
    assert!(
        subject.queries[0]
            .to_ascii_lowercase()
            .contains("iron rail"),
        "unexpected queries: {:?}",
        subject.queries
    );
    assert!(subject.queries[0].contains("2020"));
    assert!(
        search_title
            .aliases
            .iter()
            .any(|alias| alias.to_ascii_lowercase().contains("iron rail"))
    );
    assert!(
        search_title
            .tagged_aliases
            .iter()
            .any(|alias| alias.name.contains("The Movie: Iron Rail"))
    );
    assert_eq!(subject.category, "movie");
    assert_eq!(subject.owner_facet, MediaFacet::Anime);
    assert_eq!(subject.search_facet, MediaFacet::Movie);
    assert_eq!(subject.id_search_facet, Some(MediaFacet::Movie));
    assert_eq!(
        subject.newznab_categories,
        vec!["2000".to_string(), "5070".to_string()]
    );
    assert_eq!(subject.tvdb_id.as_deref(), Some("12345"));
    assert_eq!(subject.tmdb_id.as_deref(), Some("635302"));
    assert_eq!(subject.anidb_id.as_deref(), Some("15400"));
    assert_eq!(subject.mal_id.as_deref(), Some("40456"));
    assert_eq!(subject.imdb_id.as_deref(), Some("tt11032374"));
    assert_eq!(
        subject.submission_scope,
        SubmissionScope::SeriesMovie {
            series_movie_link_id: link.id,
        }
    );
}

#[tokio::test]
async fn series_movie_wanted_subject_uses_parent_owner_when_title_facet_is_missing() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let indexer_client = Arc::new(FixedReleaseIndexerClient::new(
        "Ember.Saga.Iron.Rail.2020.1080p.WEB-DL",
    ));
    let (app, user) = bootstrap_with_search_settings_and_indexer(settings, indexer_client);

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Ember Saga".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                tags: vec!["anime-hd".to_string()],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create anime title");

    let link_input = test_series_movie_link(
        &title.id,
        "Ember Saga -Kage no Kotoba- The Movie: Iron Rail",
        Some(2020),
        Some("tt11032374"),
        Some("12345"),
    );
    let link = app
        .services
        .catalog
        .shows
        .upsert_series_movie_link(link_input)
        .await
        .expect("create series movie link");
    let now = Utc::now().to_rfc3339();
    let wanted = AcquisitionScopeState {
        id: Id::new().0,
        title_id: title.id.clone(),
        title_name: Some(title.name.clone()),
        title_slug: title.slug.clone(),
        title_facet: None,
        library_id: Some(title.library_id.clone()),
        library_name: None,
        library_slug: None,
        episode_id: None,
        collection_id: None,
        series_movie_link_id: Some(link.id.clone()),
        season_number: Some("0".to_string()),
        episode_number: None,
        media_type: "series_movie".to_string(),
        last_search_at: None,
        status: AcquisitionScopeStatus::Wanted,
        grabbed_release: None,
        landed_bar: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: now.clone(),
        updated_at: now,
    };

    let search_title = app
        .release_search_title_for_wanted_item(&title, &wanted, None)
        .await;
    let subject = app
        .resolve_release_search_subject_for_wanted_item(&title, &search_title, &wanted, None)
        .await;

    assert_eq!(search_title.facet, MediaFacet::Movie);
    assert_eq!(subject.title_id, title.id);
    assert_eq!(subject.title_tags, vec!["anime-hd".to_string()]);
    assert_eq!(subject.owner_facet, MediaFacet::Anime);
    assert_eq!(subject.search_facet, MediaFacet::Movie);
    assert_eq!(subject.category, "movie");
    assert_eq!(
        subject.newznab_categories,
        vec!["2000".to_string(), "5070".to_string()]
    );
}

#[tokio::test]
async fn resolve_release_search_subject_for_series_owned_movie_keeps_movie_search_shape() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let indexer_client = Arc::new(FixedReleaseIndexerClient::new(
        "Series.Movie.2021.1080p.WEB-DL",
    ));
    let (app, user) = bootstrap_with_search_settings_and_indexer(settings, indexer_client);

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Example Series".into(),
                facet: MediaFacet::Series,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .expect("create series title");

    let link_input = test_series_movie_link(
        &title.id,
        "Example Series: The Movie",
        Some(2021),
        Some("tt12345678"),
        None,
    );
    let link = app
        .services
        .catalog
        .shows
        .upsert_series_movie_link(link_input)
        .await
        .expect("create series movie link");

    let (_search_title, subject) = app
        .resolve_release_search_subject_for_series_movie(&title, &link)
        .await
        .expect("resolve series-owned movie subject");

    assert_eq!(subject.category, "movie");
    assert_eq!(subject.owner_facet, MediaFacet::Series);
    assert_eq!(subject.search_facet, MediaFacet::Movie);
    assert_eq!(subject.id_search_facet, Some(MediaFacet::Movie));
    assert_eq!(subject.newznab_categories, vec!["2000".to_string()]);
}

#[tokio::test]
async fn search_indexers_for_series_movie_merges_categories_and_accepts_short_title_release() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let recording_client = Arc::new(RecordingCategoriesIndexerClient::new(
        "Ember.Saga.Iron.Rail.2020.1080p.WEB-DL",
    ));
    let (app, user) =
        bootstrap_with_search_settings_and_indexer(settings, recording_client.clone());
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
                name: "Ember Saga".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .expect("create anime title");

    let mut link_input = test_series_movie_link(
        &title.id,
        "Ember Saga -Kage no Kotoba- The Movie: Iron Rail",
        Some(2020),
        Some("tt11032374"),
        Some("12345"),
    );
    link_input.movie.tmdb_id = Some("635302".to_string());
    link_input.movie.anidb_id = Some("15400".to_string());
    link_input.movie.mal_id = Some("40456".to_string());
    let link = app
        .services
        .catalog
        .shows
        .upsert_series_movie_link(link_input)
        .await
        .expect("create series movie link");

    let results = app
        .search_indexers_for_series_movie(
            &user,
            title.id.clone(),
            link.id.clone(),
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .expect("series movie search should succeed");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].title, "Ember.Saga.Iron.Rail.2020.1080p.WEB-DL");

    let calls = recording_client.calls.lock().await.clone();
    let facets = calls
        .iter()
        .filter_map(|call| call.facet.clone())
        .collect::<HashSet<_>>();
    assert_eq!(facets, HashSet::from(["movie".to_string()]));
    assert!(calls.iter().all(|call| {
        call.id_search_facet.as_deref() == Some("movie")
            && call.newznab_categories.as_deref()
                == Some(["5070".to_string(), "2000".to_string()].as_slice())
    }));
    assert_eq!(calls.len(), 1);
    assert!(
        calls
            .iter()
            .all(|call| call.category.as_deref() == Some("movie"))
    );
    assert!(calls.iter().all(|call| {
        call.ids.get("imdb_id").map(String::as_str) == Some("tt11032374")
            && call.ids.get("tmdb_id").map(String::as_str) == Some("635302")
            && call.ids.get("tvdb_id").map(String::as_str) == Some("12345")
            && call.ids.get("anidb_id").map(String::as_str) == Some("15400")
            && call.ids.get("mal_id").map(String::as_str) == Some("40456")
    }));
    assert!(
        calls
            .iter()
            .any(|call| call.query.to_ascii_lowercase().contains("iron rail 2020"))
    );
}

/// Build a series-movie wanted item and resolve its search subject (a
/// `SeriesMovie` convergence scope) for the coverage write-hook tests.
async fn convergence_test_title_and_subject(
    app: &AppUseCase,
    user: &User,
) -> (
    Title,
    crate::acquisition_release_search::ResolvedReleaseSearchSubject,
) {
    let title = app
        .add_title(
            user,
            NewTitle {
                name: "Ember Saga".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                tags: vec!["anime-hd".to_string()],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create anime title");
    let link = app
        .services
        .catalog
        .shows
        .upsert_series_movie_link(test_series_movie_link(
            &title.id,
            "Ember Saga -Kage no Kotoba- The Movie: Iron Rail",
            Some(2020),
            Some("tt11032374"),
            Some("12345"),
        ))
        .await
        .expect("create series movie link");
    let now = Utc::now().to_rfc3339();
    let wanted = AcquisitionScopeState {
        id: Id::new().0,
        title_id: title.id.clone(),
        title_name: Some(title.name.clone()),
        title_slug: title.slug.clone(),
        title_facet: None,
        library_id: Some(title.library_id.clone()),
        library_name: None,
        library_slug: None,
        episode_id: None,
        collection_id: None,
        series_movie_link_id: Some(link.id.clone()),
        season_number: Some("0".to_string()),
        episode_number: None,
        media_type: "series_movie".to_string(),
        last_search_at: None,
        status: AcquisitionScopeStatus::Wanted,
        grabbed_release: None,
        landed_bar: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: now.clone(),
        updated_at: now,
    };
    let search_title = app
        .release_search_title_for_wanted_item(&title, &wanted, None)
        .await;
    let subject = app
        .resolve_release_search_subject_for_wanted_item(&title, &search_title, &wanted, None)
        .await;
    (title, subject)
}

/// Convergence is always on now: a scope is converged when
/// every routed indexer is covered under the current fingerprint. Resolves the
/// scope's coordinates and asks whether any routed indexer remains uncovered.
async fn scope_is_converged(
    app: &AppUseCase,
    title: &Title,
    subject: &crate::acquisition_release_search::ResolvedReleaseSearchSubject,
) -> bool {
    let Some(c) = app.resolve_scope_convergence(title, subject).await else {
        return false;
    };
    app.uncovered_indexers_for_scope(
        &c.scope_key,
        &c.facet,
        &c.fingerprint,
        &c.routed_indexer_ids,
    )
    .await
    .map(|u| u.is_empty())
    .unwrap_or(false)
}

#[tokio::test]
async fn background_search_records_scope_indexer_coverage() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let configs = vec![
        synthetic_direct_nab_indexer_config("indexer-a", "newznab"),
        synthetic_direct_nab_indexer_config("indexer-b", "newznab"),
    ];
    let (app, user) = bootstrap_with_search_settings_indexer_and_configs(
        settings,
        Arc::new(MockIndexerClient),
        configs,
    );
    let coverage = Arc::new(RecordingScopeIndexerCoverageRepo::new());
    let app = app
        .with_test_overrides(|builder| builder.with_scope_indexer_coverage_store(coverage.clone()));

    let (title, subject) = convergence_test_title_and_subject(&app, &user).await;
    let expected_scope_key = crate::acquisition::convergence::convergence_scope_key(
        &subject.submission_scope,
        &subject.title_id,
    )
    .expect("series-movie scope has a convergence key");

    app.record_search_coverage(
        &title,
        &subject,
        &["indexer-a".to_string(), "indexer-b".to_string()],
    )
    .await;

    let rows = coverage.recorded().await;
    let mut indexers: Vec<String> = rows.iter().map(|row| row.2.clone()).collect();
    indexers.sort();
    assert_eq!(
        indexers,
        vec!["indexer-a".to_string(), "indexer-b".to_string()],
        "coverage is recorded once per routed indexer"
    );
    assert!(
        rows.iter().all(|row| row.0 == expected_scope_key),
        "every row is keyed by the scope's convergence key"
    );
    let fingerprints: HashSet<String> = rows.iter().map(|row| row.3.clone()).collect();
    assert_eq!(
        fingerprints.len(),
        1,
        "a single search resolves to exactly one fingerprint"
    );
    assert!(
        !fingerprints.into_iter().next().unwrap().is_empty(),
        "the recorded fingerprint is non-empty"
    );
}

#[tokio::test]
async fn scope_converges_only_after_every_routed_indexer_is_covered() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let configs = vec![
        synthetic_direct_nab_indexer_config("indexer-a", "newznab"),
        synthetic_direct_nab_indexer_config("indexer-b", "newznab"),
    ];
    let (app, user) = bootstrap_with_search_settings_indexer_and_configs(
        settings,
        Arc::new(MockIndexerClient),
        configs,
    );
    let coverage = Arc::new(RecordingScopeIndexerCoverageRepo::new());
    let app = app
        .with_test_overrides(|builder| builder.with_scope_indexer_coverage_store(coverage.clone()));

    let (title, subject) = convergence_test_title_and_subject(&app, &user).await;
    let convergence = app
        .resolve_scope_convergence(&title, &subject)
        .await
        .expect("routed convergence coordinates");

    // A fresh scope is not converged, so the background path would search.
    assert!(
        !scope_is_converged(&app, &title, &subject).await,
        "a fresh scope with no coverage is not converged"
    );

    // Coverage on only one of the two routed indexers is still not enough.
    coverage
        .record_coverage(
            &convergence.scope_key,
            &convergence.facet,
            "indexer-a",
            &convergence.fingerprint,
        )
        .await
        .unwrap();
    assert!(
        !scope_is_converged(&app, &title, &subject).await,
        "partial coverage does not converge the scope"
    );

    // The write-hook records coverage for every routed indexer that fired; the
    // read-gate (using the same resolution) then recognises the scope as converged.
    app.record_search_coverage(&title, &subject, &convergence.routed_indexer_ids)
        .await;
    assert!(
        scope_is_converged(&app, &title, &subject).await,
        "scope converges once every routed indexer is covered under the current fingerprint"
    );
}

#[tokio::test]
async fn coverage_reopen_policies_preserve_the_required_indexers() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let configs = vec![
        synthetic_direct_nab_indexer_config("indexer-a", "newznab"),
        synthetic_direct_nab_indexer_config("indexer-b", "newznab"),
    ];
    let (app, user) = bootstrap_with_search_settings_indexer_and_configs(
        settings,
        Arc::new(MockIndexerClient),
        configs,
    );
    let coverage = Arc::new(RecordingScopeIndexerCoverageRepo::new());
    let app =
        app.with_test_overrides(|builder| builder.with_scope_indexer_coverage_store(coverage));
    let (title, subject) = convergence_test_title_and_subject(&app, &user).await;
    let convergence = app
        .resolve_scope_convergence(&title, &subject)
        .await
        .expect("routed convergence coordinates");

    app.record_search_coverage(&title, &subject, &convergence.routed_indexer_ids)
        .await;
    app.prune_scope_key_coverage(&convergence.scope_key, Some("indexer-a"))
        .await;
    assert_eq!(
        app.uncovered_indexers_for_scope(
            &convergence.scope_key,
            &convergence.facet,
            &convergence.fingerprint,
            &convergence.routed_indexer_ids,
        )
        .await
        .unwrap(),
        vec!["indexer-a".to_string()]
    );

    app.record_search_coverage(&title, &subject, &convergence.routed_indexer_ids)
        .await;
    let SubmissionScope::SeriesMovie {
        series_movie_link_id,
    } = &subject.submission_scope
    else {
        panic!("fixture must resolve a series-movie scope");
    };
    let item = AcquisitionScopeState {
        id: Id::new().0,
        title_id: title.id.clone(),
        title_name: Some(title.name.clone()),
        title_slug: title.slug.clone(),
        title_facet: None,
        library_id: Some(title.library_id.clone()),
        library_name: None,
        library_slug: None,
        episode_id: None,
        collection_id: None,
        series_movie_link_id: Some(series_movie_link_id.clone()),
        season_number: Some("0".to_string()),
        episode_number: None,
        media_type: "series_movie".to_string(),
        last_search_at: None,
        status: AcquisitionScopeStatus::Wanted,
        grabbed_release: None,
        landed_bar: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
    };

    app.reopen_wanted_scope_for_acquisition(
        &item,
        crate::acquisition::convergence::CoverageReopen::All,
    )
    .await;
    assert_eq!(
        app.uncovered_indexers_for_scope(
            &convergence.scope_key,
            &convergence.facet,
            &convergence.fingerprint,
            &convergence.routed_indexer_ids,
        )
        .await
        .unwrap(),
        vec!["indexer-a".to_string(), "indexer-b".to_string()]
    );

    app.record_search_coverage(&title, &subject, &convergence.routed_indexer_ids)
        .await;
    app.reopen_wanted_scope_for_acquisition(
        &item,
        crate::acquisition::convergence::CoverageReopen::Keep,
    )
    .await;
    assert!(
        app.uncovered_indexers_for_scope(
            &convergence.scope_key,
            &convergence.facet,
            &convergence.fingerprint,
            &convergence.routed_indexer_ids,
        )
        .await
        .unwrap()
        .is_empty(),
        "Keep preserves full coverage and leaves the scope converged"
    );
}

#[tokio::test]
async fn background_acquisition_requeries_only_the_pruned_indexer() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let indexer_client = Arc::new(
        FixedReleaseIndexerClient::new("Cursor Coverage Fixture.2024.1080p.WEB-DL")
            .with_fired_indexers(["indexer-a"])
            .with_empty_response(),
    );
    let (app, user) = bootstrap_with_acquisition_tracking_and_indexer(
        download_client,
        download_submissions,
        pending_releases,
        wanted_items.clone(),
        indexer_client.clone(),
    );
    app.services
        .integrations
        .indexer_configs
        .delete("acquisition-indexer")
        .await
        .expect("remove bootstrap indexer");
    for indexer_id in ["indexer-a", "indexer-b"] {
        app.services
            .integrations
            .indexer_configs
            .create(synthetic_direct_nab_indexer_config(indexer_id, "newznab"))
            .await
            .expect("create routed indexer");
    }
    let coverage = Arc::new(RecordingScopeIndexerCoverageRepo::new());
    let app = app
        .with_test_overrides(|builder| builder.with_scope_indexer_coverage_store(coverage.clone()));

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Cursor Coverage Fixture".into(),
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
        last_search_at: None,
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
        .expect("seed fileless wanted movie");
    let search_title = app
        .release_search_title_for_wanted_item(&title, &wanted, None)
        .await;
    let subject = app
        .resolve_release_search_subject_for_wanted_item(&title, &search_title, &wanted, None)
        .await;
    let convergence = app
        .resolve_scope_convergence(&title, &subject)
        .await
        .expect("resolve live convergence coordinates");
    app.record_search_coverage(&title, &subject, &convergence.routed_indexer_ids)
        .await;
    app.prune_scope_key_coverage(&convergence.scope_key, Some("indexer-a"))
        .await;
    let indexer_b_before = coverage
        .recorded()
        .await
        .into_iter()
        .find(|(_, _, indexer_id, _)| indexer_id == "indexer-b")
        .expect("indexer-b remains covered before the cursor runs");

    app.run_background_acquisition_cycle_once().await;

    assert_eq!(
        indexer_client.requested_indexer_id_sets().await,
        vec![vec!["indexer-a".to_string()]],
        "the cursor sent only the uncovered indexer through the routing plan"
    );
    let rows = coverage.recorded().await;
    assert!(
        rows.iter()
            .any(|(_, _, indexer_id, _)| indexer_id == "indexer-a"),
        "the cursor re-recorded coverage for the sole uncovered indexer"
    );
    assert!(
        rows.iter().any(|row| row == &indexer_b_before),
        "the covered peer row was untouched by the restricted cursor search"
    );
}

/// The failure loop never costs an indexer query. A grab that fails is
/// blocklisted and its scope re-opened under its existing coverage; the cursor
/// then walks the scope's saved search results in order, and once they are
/// exhausted the scope simply stays converged — no re-search.
#[tokio::test]
async fn a_failed_grab_walks_the_saved_search_results_without_querying_an_indexer() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let indexer_client = Arc::new(
        FixedReleaseIndexerClient::new("Saved Results Fixture.2024.1080p.WEB-DL")
            .with_fired_indexers(["indexer-a", "indexer-b"])
            .with_empty_response(),
    );
    let (app, user) = bootstrap_with_acquisition_tracking_and_indexer(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases.clone(),
        wanted_items.clone(),
        indexer_client.clone(),
    );
    app.services
        .integrations
        .indexer_configs
        .delete("acquisition-indexer")
        .await
        .expect("remove bootstrap indexer");
    for indexer_id in ["indexer-a", "indexer-b"] {
        app.services
            .integrations
            .indexer_configs
            .create(synthetic_direct_nab_indexer_config(indexer_id, "newznab"))
            .await
            .expect("create routed indexer");
    }
    let coverage = Arc::new(RecordingScopeIndexerCoverageRepo::new());
    let app = app
        .with_test_overrides(|builder| builder.with_scope_indexer_coverage_store(coverage.clone()));

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Saved Results Fixture".into(),
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
    let release = |suffix: &str| format!("Saved.Results.Fixture.2024.1080p.WEB-DL-{suffix}");

    // The scope was searched (both indexers covered), its best release grabbed,
    // and the two runners-up saved.
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
        last_search_at: Some(Utc::now().to_rfc3339()),
        status: AcquisitionScopeStatus::Grabbed,
        grabbed_release: Some(
            serde_json::json!({
                "title": release("FIRST"),
                "score": 300,
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
        .expect("seed grabbed wanted movie");
    let search_title = app
        .release_search_title_for_wanted_item(&title, &wanted, None)
        .await;
    let subject = app
        .resolve_release_search_subject_for_wanted_item(&title, &search_title, &wanted, None)
        .await;
    let convergence = app
        .resolve_scope_convergence(&title, &subject)
        .await
        .expect("resolve live convergence coordinates");
    app.record_search_coverage(&title, &subject, &convergence.routed_indexer_ids)
        .await;
    let covered_before = coverage.recorded().await;
    assert_eq!(covered_before.len(), 2, "fixture: both indexers covered");

    let saved = |suffix: &str, score: i32| PendingRelease {
        id: Id::new().0,
        wanted_item_id: wanted.id.clone(),
        title_id: title.id.clone(),
        release_title: release(suffix),
        release_url: Some(format!("https://example.com/{}.nzb", suffix.to_lowercase())),
        source_kind: Some(DownloadSourceKind::NzbUrl),
        release_size_bytes: None,
        release_score: score,
        scoring_log_json: None,
        indexer_source: Some("indexer-a".to_string()),
        indexer_id: Some("indexer-a".to_string()),
        release_guid: Some(format!("guid-{}", suffix.to_lowercase())),
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
        release_identity: format!("guid-{}", suffix.to_lowercase()),
        coverage_identity: format!("scope:{}", wanted.id),
        role: crate::types::PendingReleaseRole::Fallback,
        last_decision_code: None,
        release_age_unknown: false,
    };
    pending_releases
        .insert_pending_release(&saved("SECOND", 200))
        .await
        .expect("seed saved result");
    pending_releases
        .insert_pending_release(&saved("THIRD", 100))
        .await
        .expect("seed saved result");
    download_submissions
        .record_submission(DownloadSubmission {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            title_id: title.id.clone(),
            purpose: crate::DownloadSubmissionPurpose::Standard,
            facet: "movie".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "first-job".to_string(),
            source_hint: None,
            source_provider_id: Some("indexer-a".to_string()),
            source_provider_name: None,
            source_kind: None,
            source_title: Some(release("FIRST")),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record first grab");

    let wanted_now = || async {
        wanted_items
            .get_acquisition_scope_state_by_id(&wanted.id)
            .await
            .expect("load wanted")
            .expect("wanted exists")
    };
    let status_of = |suffix: &str| {
        let pending_releases = pending_releases.clone();
        let release_title = release(suffix);
        async move {
            pending_releases
                .store
                .lock()
                .await
                .iter()
                .find(|row| row.release_title == release_title)
                .map(|row| row.status)
        }
    };
    let fail = |client_id: String, client_type: String, client_item_id: String, suffix: &str| {
        crate::acquisition_workflow::DownloadFailureContext {
            wanted_item: None,
            title_id: Some(title.id.clone()),
            client_id,
            client_type,
            client_name: Some("Primary".to_string()),
            client_item_id,
            release_title: release(suffix),
            reason: "download failed".to_string(),
            remove_from_client_if_configured: false,
            skip_reacquire: false,
        }
    };
    let submission_for = |suffix: &str| {
        let download_submissions = download_submissions.clone();
        let release_title = release(suffix);
        async move {
            download_submissions
                .store
                .lock()
                .await
                .iter()
                .find(|submission| {
                    submission.source_title.as_deref() == Some(release_title.as_str())
                })
                .cloned()
                .expect("the cursor recorded the grab")
        }
    };

    // 1. The first grab fails: blocklisted, scope re-opened, coverage untouched.
    let outcome = crate::acquisition_workflow::process_download_failure(
        &app,
        fail(
            "primary".into(),
            "nzbget".into(),
            "first-job".into(),
            "FIRST",
        ),
    )
    .await;
    assert_eq!(
        outcome,
        crate::acquisition_workflow::FailureHandlingOutcome::Reopened
    );
    assert_eq!(wanted_now().await.status, AcquisitionScopeStatus::Wanted);
    assert_eq!(
        coverage.recorded().await,
        covered_before,
        "a failure never prunes coverage"
    );
    // The client no longer lists the failed job (the failure was processed).
    download_client.queue_items.lock().await.clear();
    download_client
        .set_snapshot_authoritative_client_ids(["primary".to_string()])
        .await;

    // 2. The cursor grabs the next saved result and queries nothing.
    app.run_background_acquisition_cycle_once().await;
    assert!(
        indexer_client.requested_indexer_id_sets().await.is_empty(),
        "no indexer query while saved results remain"
    );
    let after_first = wanted_now().await;
    assert_eq!(
        after_first.status,
        AcquisitionScopeStatus::Grabbed,
        "the cursor walked to the next saved result: {:?}",
        pending_releases
            .store
            .lock()
            .await
            .iter()
            .map(|row| (row.release_title.clone(), row.status))
            .collect::<Vec<_>>()
    );
    assert!(
        after_first
            .grabbed_release
            .as_deref()
            .unwrap_or_default()
            .contains(&release("SECOND")),
        "{:?}",
        after_first.grabbed_release
    );
    assert_eq!(
        status_of("SECOND").await,
        Some(PendingReleaseStatus::Grabbed)
    );
    assert_eq!(
        status_of("THIRD").await,
        Some(PendingReleaseStatus::Standby),
        "the rest of the list survives the grab"
    );

    // 3. That one fails too: the walk continues down the same list.
    let second = submission_for("SECOND").await;
    let outcome = crate::acquisition_workflow::process_download_failure(
        &app,
        fail(
            second.download_client_id.clone().unwrap_or_default(),
            second.download_client_type.clone(),
            second.download_client_item_id.clone(),
            "SECOND",
        ),
    )
    .await;
    assert_eq!(
        outcome,
        crate::acquisition_workflow::FailureHandlingOutcome::Reopened
    );
    download_client.queue_items.lock().await.clear();
    app.run_background_acquisition_cycle_once().await;
    assert!(indexer_client.requested_indexer_id_sets().await.is_empty());
    let after_second = wanted_now().await;
    assert_eq!(after_second.status, AcquisitionScopeStatus::Grabbed);
    assert!(
        after_second
            .grabbed_release
            .as_deref()
            .unwrap_or_default()
            .contains(&release("THIRD"))
    );
    assert_eq!(
        status_of("THIRD").await,
        Some(PendingReleaseStatus::Grabbed)
    );

    // 4. The last one fails: nothing saved remains, the scope stays converged
    //    under its untouched coverage, and still nothing was queried.
    let third = submission_for("THIRD").await;
    let outcome = crate::acquisition_workflow::process_download_failure(
        &app,
        fail(
            third.download_client_id.clone().unwrap_or_default(),
            third.download_client_type.clone(),
            third.download_client_item_id.clone(),
            "THIRD",
        ),
    )
    .await;
    assert_eq!(
        outcome,
        crate::acquisition_workflow::FailureHandlingOutcome::Reopened
    );
    download_client.queue_items.lock().await.clear();
    app.run_background_acquisition_cycle_once().await;
    assert!(
        indexer_client.requested_indexer_id_sets().await.is_empty(),
        "an exhausted list leaves the scope converged; no re-search"
    );
    assert_eq!(wanted_now().await.status, AcquisitionScopeStatus::Wanted);
    assert!(
        pending_releases
            .list_all_standby_pending_releases()
            .await
            .expect("list standby")
            .is_empty()
    );
    assert_eq!(coverage.recorded().await, covered_before);
}

/// Everything the search ranked below the grabbed release is saved — the whole
/// list, not a capped handful — so a failure can walk as far down as it needs.
#[tokio::test]
async fn a_grab_saves_every_remaining_eligible_search_result() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let titles: Vec<String> = (1..=8)
        .map(|index| format!("Saved.Everything.Fixture.2024.1080p.WEB-DL-G{index}"))
        .collect();
    let indexer_client = Arc::new(MultiReleaseIndexerClient::new(
        titles.iter().map(String::as_str).collect(),
    ));
    let (app, user) = bootstrap_with_acquisition_tracking_and_indexer(
        download_client,
        download_submissions,
        pending_releases.clone(),
        wanted_items.clone(),
        indexer_client,
    );
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Saved Everything Fixture".into(),
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
        last_search_at: None,
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
        .expect("seed fileless wanted movie");

    app.run_background_acquisition_cycle_once().await;

    let updated = wanted_items
        .get_acquisition_scope_state_by_id(&wanted.id)
        .await
        .expect("load wanted")
        .expect("wanted exists");
    assert_eq!(updated.status, AcquisitionScopeStatus::Grabbed);
    let saved: Vec<String> = pending_releases
        .store
        .lock()
        .await
        .iter()
        .filter(|row| {
            row.wanted_item_id == wanted.id && row.status == PendingReleaseStatus::Standby
        })
        .map(|row| row.release_title.clone())
        .collect();
    assert_eq!(
        saved.len(),
        titles.len() - 1,
        "every runner-up is saved, not a capped handful: {saved:?}"
    );
}

#[tokio::test]
async fn every_scoped_search_records_coverage_including_interactive() {
    // "A search is a search": search_and_evaluate_subject records coverage
    // for every caller, interactive included. This drives the real chokepoint and
    // asserts the fired indexers land in the coverage ledger regardless of mode.
    let settings = Arc::new(StoredSettingsRepo::default());
    let configs = vec![
        synthetic_direct_nab_indexer_config("indexer-a", "newznab"),
        synthetic_direct_nab_indexer_config("indexer-b", "newznab"),
    ];
    let indexer_client = Arc::new(
        FixedReleaseIndexerClient::new("Ember.Saga.Iron.Rail.2020.1080p.WEB-DL")
            .with_fired_indexers(["indexer-a", "indexer-b"]),
    );
    let (app, user) =
        bootstrap_with_search_settings_indexer_and_configs(settings, indexer_client, configs);
    let coverage = Arc::new(RecordingScopeIndexerCoverageRepo::new());
    let app = app
        .with_test_overrides(|builder| builder.with_scope_indexer_coverage_store(coverage.clone()));

    let (title, subject) = convergence_test_title_and_subject(&app, &user).await;

    // An interactive-labelled search now records coverage for each indexer that fired.
    let _ = app
        .search_and_evaluate_subject(
            &title,
            &subject,
            "interactive_search",
            SearchMode::Interactive,
            tokio_util::sync::CancellationToken::new(),
        )
        .await;
    let mut indexers: Vec<String> = coverage
        .recorded()
        .await
        .iter()
        .map(|row| row.2.clone())
        .collect();
    indexers.sort();
    indexers.dedup();
    assert_eq!(
        indexers,
        vec!["indexer-a".to_string(), "indexer-b".to_string()],
        "interactive search records coverage for each indexer that fired"
    );
}

#[tokio::test]
async fn empty_response_from_fired_indexer_counts_as_coverage() {
    // An indexer whose query executed and returned an EMPTY
    // response is still covered — a long-tail release genuinely absent from an
    // indexer must converge, or the cursor re-searches that empty indexer every
    // cycle forever. The determination comes from the multi-indexer fanout's
    // per-indexer outcomes (`Fired { empty: true }`), never from the merged
    // result list being empty.
    let settings = Arc::new(StoredSettingsRepo::default());
    let configs = vec![
        synthetic_direct_nab_indexer_config("indexer-a", "newznab"),
        synthetic_direct_nab_indexer_config("indexer-b", "newznab"),
    ];
    let indexer_client = Arc::new(
        FixedReleaseIndexerClient::new("Ember.Saga.Iron.Rail.2020.1080p.WEB-DL")
            .with_fired_indexers(["indexer-a", "indexer-b"])
            .with_empty_response(),
    );
    let (app, user) =
        bootstrap_with_search_settings_indexer_and_configs(settings, indexer_client, configs);
    let coverage = Arc::new(RecordingScopeIndexerCoverageRepo::new());
    let app = app
        .with_test_overrides(|builder| builder.with_scope_indexer_coverage_store(coverage.clone()));

    let (title, subject) = convergence_test_title_and_subject(&app, &user).await;

    let results = app
        .search_and_evaluate_subject(
            &title,
            &subject,
            "background_acquisition",
            SearchMode::Auto,
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .expect("empty search succeeds");
    assert!(results.is_empty(), "the response genuinely had no results");

    let mut indexers: Vec<String> = coverage
        .recorded()
        .await
        .iter()
        .map(|row| row.2.clone())
        .collect();
    indexers.sort();
    indexers.dedup();
    assert_eq!(
        indexers,
        vec!["indexer-a".to_string(), "indexer-b".to_string()],
        "a zero-result response from a fired indexer records coverage"
    );
    assert!(
        scope_is_converged(&app, &title, &subject).await,
        "empty responses across every routed indexer converge the scope"
    );
}

#[tokio::test]
async fn stale_fingerprint_coverage_reopens_convergence() {
    // A profile/criteria edit changes the fingerprint, so prior coverage
    // (recorded under the old fingerprint) no longer counts and the scope re-opens.
    let settings = Arc::new(StoredSettingsRepo::default());
    let configs = vec![
        synthetic_direct_nab_indexer_config("indexer-a", "newznab"),
        synthetic_direct_nab_indexer_config("indexer-b", "newznab"),
    ];
    let (app, user) = bootstrap_with_search_settings_indexer_and_configs(
        settings,
        Arc::new(MockIndexerClient),
        configs,
    );
    let coverage = Arc::new(RecordingScopeIndexerCoverageRepo::new());
    let app = app
        .with_test_overrides(|builder| builder.with_scope_indexer_coverage_store(coverage.clone()));

    let (title, subject) = convergence_test_title_and_subject(&app, &user).await;
    let convergence = app
        .resolve_scope_convergence(&title, &subject)
        .await
        .expect("routed convergence coordinates");

    // Full coverage, but recorded under a since-superseded fingerprint.
    for indexer_id in ["indexer-a", "indexer-b"] {
        coverage
            .record_coverage(
                &convergence.scope_key,
                &convergence.facet,
                indexer_id,
                "superseded-fingerprint",
            )
            .await
            .unwrap();
    }
    assert!(
        !scope_is_converged(&app, &title, &subject).await,
        "coverage under a stale fingerprint does not count; the scope re-opens"
    );

    // Re-searching under the current fingerprint converges it again.
    app.record_search_coverage(&title, &subject, &convergence.routed_indexer_ids)
        .await;
    assert!(
        scope_is_converged(&app, &title, &subject).await,
        "coverage under the current fingerprint converges the scope"
    );
}

#[tokio::test]
async fn coverage_excludes_disabled_indexers() {
    // A disabled indexer is never queried, so it must not be recorded as covered
    // (otherwise enabling it later would wrongly present as already-searched).
    let settings = Arc::new(StoredSettingsRepo::default());
    let mut disabled_b = synthetic_direct_nab_indexer_config("indexer-b", "newznab");
    disabled_b.is_enabled = false;
    let configs = vec![
        synthetic_direct_nab_indexer_config("indexer-a", "newznab"),
        disabled_b,
    ];
    let (app, user) = bootstrap_with_search_settings_indexer_and_configs(
        settings,
        Arc::new(MockIndexerClient),
        configs,
    );
    let coverage = Arc::new(RecordingScopeIndexerCoverageRepo::new());
    let app = app
        .with_test_overrides(|builder| builder.with_scope_indexer_coverage_store(coverage.clone()));

    let (title, subject) = convergence_test_title_and_subject(&app, &user).await;
    // Both indexers "fired", but the disabled one is not in the routed set, so the
    // routed∩fired intersection drops it — only the enabled indexer is recorded.
    app.record_search_coverage(
        &title,
        &subject,
        &["indexer-a".to_string(), "indexer-b".to_string()],
    )
    .await;

    let indexers: Vec<String> = coverage
        .recorded()
        .await
        .iter()
        .map(|row| row.2.clone())
        .collect();
    assert_eq!(
        indexers,
        vec!["indexer-a".to_string()],
        "only enabled routed indexers are recorded as covered"
    );
    // With the disabled indexer excluded from the routed set, the one enabled
    // indexer's coverage converges the scope.
    assert!(
        scope_is_converged(&app, &title, &subject).await,
        "scope converges over the enabled routed indexers only"
    );
}

#[tokio::test]
async fn coverage_records_only_indexers_that_fired() {
    // A routed indexer that did NOT fire (deferred/skipped/errored) is
    // not recorded as covered, so the scope stays a target for the cursor to retry.
    let settings = Arc::new(StoredSettingsRepo::default());
    let configs = vec![
        synthetic_direct_nab_indexer_config("indexer-a", "newznab"),
        synthetic_direct_nab_indexer_config("indexer-b", "newznab"),
    ];
    let (app, user) = bootstrap_with_search_settings_indexer_and_configs(
        settings,
        Arc::new(MockIndexerClient),
        configs,
    );
    let coverage = Arc::new(RecordingScopeIndexerCoverageRepo::new());
    let app = app
        .with_test_overrides(|builder| builder.with_scope_indexer_coverage_store(coverage.clone()));

    let (title, subject) = convergence_test_title_and_subject(&app, &user).await;

    // Only indexer-a fired; indexer-b was routed but deferred/skipped/errored.
    app.record_search_coverage(&title, &subject, &["indexer-a".to_string()])
        .await;

    let indexers: Vec<String> = coverage
        .recorded()
        .await
        .iter()
        .map(|row| row.2.clone())
        .collect();
    assert_eq!(
        indexers,
        vec!["indexer-a".to_string()],
        "only the indexer that fired is recorded as covered"
    );
    // indexer-b was routed but did not fire, so it stays uncovered and the scope
    // has not converged.
    assert!(
        !scope_is_converged(&app, &title, &subject).await,
        "a routed indexer that did not fire leaves the scope unconverged"
    );
}

// ── TYPE-001: catalog queueing decides retry-later by error type only ────────

async fn assert_queue_existing_title_submit_decision(
    submit_error: StubSubmitError,
    expect_deferred: bool,
) {
    let download_client = Arc::new(StubDownloadClient::default());
    download_client.set_submit_error(Some(submit_error)).await;
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let (app, user, release_attempts) =
        bootstrap_with_acquisition_tracking_and_indexer_and_release_attempts(
            download_client,
            download_submissions.clone(),
            pending_releases,
            wanted_items,
            Arc::new(MockIndexerClient),
        );
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Typed Failover Queue".into(),
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
    let source_title = "Typed.Failover.Queue.2026.1080p.WEB-DL";

    let error = app
        .queue_existing_title_download(
            &user,
            &title.id,
            QueuedReleaseSelection {
                indexer_id: None,
                source_hint: Some(
                    "https://example.invalid/releases/typed-failover.nzb".to_string(),
                ),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some(source_title.to_string()),
                source_password: None,
                info_hash_hint: None,
                size_bytes: None,
                seeders: None,
            },
            SubmissionScope::Title,
            SubmissionConflictPolicy::Abort,
        )
        .await
        .expect_err("the submit failure surfaces to the caller");
    assert_eq!(
        error.is_retryable_download_submit_failure(),
        expect_deferred
    );
    assert!(download_submissions.store.lock().await.is_empty());

    let attempts = release_attempts.attempts.lock().await.clone();
    let outcomes = attempts
        .iter()
        .filter(|attempt| attempt.source_title.as_deref() == Some(source_title))
        .map(|attempt| attempt.outcome.clone())
        .collect::<Vec<_>>();
    let failed = release_attempts
        .list_failed_release_signatures_for_title(&title.id, 10)
        .await
        .expect("list failed signatures");
    let blocklist = app
        .services
        .workflow
        .blocklist_repo
        .list_for_title(&title.id, 10)
        .await
        .expect("list blocklist");
    if expect_deferred {
        assert!(
            !outcomes.is_empty()
                && outcomes
                    .iter()
                    .all(|outcome| *outcome == ReleaseDownloadAttemptOutcome::Pending),
            "typed failover exhaustion must record Pending only: {outcomes:?}"
        );
        assert!(failed.is_empty(), "{failed:?}");
        assert!(blocklist.is_empty(), "{blocklist:?}");
    } else {
        assert!(
            outcomes.contains(&ReleaseDownloadAttemptOutcome::Failed),
            "{outcomes:?}"
        );
        assert!(
            !failed.is_empty(),
            "the legacy failover text is a definitive failure"
        );
        assert!(
            blocklist
                .iter()
                .any(|entry| entry.release_name == source_title),
            "{blocklist:?}"
        );
    }
}

#[tokio::test]
async fn queue_existing_title_download_defers_typed_failover_exhaustion() {
    assert_queue_existing_title_submit_decision(
        StubSubmitError::FailoverExhausted(
            "all prioritized download clients failed to enqueue this release; last client error: client submit unavailable"
                .to_string(),
        ),
        true,
    )
    .await;
}

#[tokio::test]
async fn queue_existing_title_download_treats_legacy_failover_text_as_definitive() {
    assert_queue_existing_title_submit_decision(
        StubSubmitError::Repository(LEGACY_FAILOVER_REPOSITORY_MESSAGE.to_string()),
        false,
    )
    .await;
}

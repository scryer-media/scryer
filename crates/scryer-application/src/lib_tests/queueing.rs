use super::*;

#[tokio::test]
async fn location_move_drains_an_already_admitted_download_submission() {
    let client = Arc::new(StubDownloadClient::default());
    let submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        client.clone(),
        submissions.clone(),
        Arc::new(TrackingPendingReleaseRepo::default()),
    );
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Draining Download".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let gate = Arc::new(tokio::sync::Notify::new());
    *client.submit_gate.lock().await = Some(gate.clone());
    let submit_app = app.clone();
    let title_id = title.id.clone();
    let submit = tokio::spawn(async move {
        submit_app
            .queue_existing_title_download(
                &user,
                &title_id,
                QueuedReleaseSelection {
                    source_hint: Some("https://example.invalid/draining.nzb".into()),
                    source_kind: Some(DownloadSourceKind::NzbUrl),
                    source_title: Some("Draining.Download.2026.1080p.WEB-DL".into()),
                    ..Default::default()
                },
                SubmissionScope::Title,
                SubmissionConflictPolicy::Abort,
            )
            .await
    });
    within_deadline(
        "the download submission to reach the client",
        client.submit_started.notified(),
    )
    .await;
    let entities = [crate::location::ownership_guard::OwnedEntity::Title(
        title.id.clone(),
    )];
    // One unconstrained poll runs the drain to the title's admission lock:
    // Pending means it is parked behind the admitted submission's lease.
    let mut drain = Box::pin(tokio::task::unconstrained(
        app.runtime
            .library
            .location_ownership
            .drain_title_mutations(&entities),
    ));
    assert!(
        futures_util::poll!(drain.as_mut()).is_pending(),
        "the drain must wait for the admitted submission"
    );
    assert!(submissions.store.lock().await.is_empty());
    gate.notify_one();
    let exclusive = within_deadline("the drain after the submission lands", drain).await;
    assert_eq!(
        submissions.store.lock().await.len(),
        1,
        "submission must be durable before the move can claim ownership"
    );
    app.runtime
        .library
        .location_ownership
        .claim_all("move", &entities);
    drop(exclusive);
    submit.await.unwrap().unwrap();
    assert!(
        app.acquire_location_title_mutation(
            &crate::location::ownership_guard::TITLE_DOWNLOAD_ENTRY,
            &title.id
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn location_locked_title_cannot_download_or_upgrade_and_does_not_blocklist_release() {
    let client = Arc::new(StubDownloadClient::default());
    let submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        client.clone(),
        submissions.clone(),
        Arc::new(TrackingPendingReleaseRepo::default()),
    );
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Locked Download".into(),
                facet: MediaFacet::Series,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let collection = app
        .create_collection(
            &user,
            title.id.clone(),
            "season".into(),
            "1".into(),
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    let episode = app
        .create_episode(
            &user,
            title.id.clone(),
            Some(collection.id.clone()),
            "standard".into(),
            Some("1".into()),
            Some("1".into()),
            None,
            None,
            None,
            None,
            false,
            false,
        )
        .await
        .unwrap();
    let scope = SubmissionScope::Episode {
        episode_id: episode.id,
    };
    let release = QueuedReleaseSelection {
        source_hint: Some("https://example.invalid/locked.nzb".into()),
        source_kind: Some(DownloadSourceKind::NzbUrl),
        source_title: Some("Locked.Download.S01E01.1080p.WEB-DL".into()),
        ..Default::default()
    };
    app.runtime.library.location_ownership.claim_all(
        "move",
        &[crate::location::ownership_guard::OwnedEntity::Title(
            title.id.clone(),
        )],
    );
    for purpose in [
        DownloadSubmissionPurpose::Standard,
        DownloadSubmissionPurpose::OperatorQueued,
        DownloadSubmissionPurpose::ManualReplacement,
        DownloadSubmissionPurpose::AdditionalFile,
    ] {
        let error = app
            .queue_existing_title_download_with_purpose(
                &user,
                &title.id,
                release.clone(),
                scope.clone(),
                SubmissionConflictPolicy::ReplaceEarly,
                purpose,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(error, AppError::LocationOperationBusy(_)),
            "{error}"
        );
    }
    assert!(client.submitted_title_ids.lock().await.is_empty());
    assert!(client.deleted_items.lock().await.is_empty());
    assert!(submissions.store.lock().await.is_empty());
    assert!(title_blocklist_entries(&app, &title.id).await.is_empty());
    assert!(
        app.derive_acquisition_targets_for_title(&Utc::now(), Some(&title.id))
            .await
            .unwrap()
            .is_empty()
    );
    app.runtime
        .library
        .location_ownership
        .release_operation("move");
    app.queue_existing_title_download(
        &user,
        &title.id,
        release,
        scope,
        SubmissionConflictPolicy::Abort,
    )
    .await
    .unwrap();
    assert_eq!(client.submitted_title_ids.lock().await.len(), 1);
}

#[tokio::test]
async fn canonical_submission_holds_artifact_lease_until_client_accepts() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    struct Lease {
        active: Arc<AtomicBool>,
        staged: crate::StagedNzbRef,
    }
    impl Drop for Lease {
        fn drop(&mut self) {
            self.active.store(false, Ordering::SeqCst);
        }
    }
    impl crate::IndexerArtifactLease for Lease {
        fn staged_nzb(&self) -> &crate::StagedNzbRef {
            &self.staged
        }
    }
    struct Resolver {
        active: Arc<AtomicBool>,
        calls: Arc<AtomicUsize>,
    }
    #[async_trait::async_trait]
    impl crate::IndexerArtifactResolver for Resolver {
        async fn resolve_artifact(
            &self,
            request: &crate::IndexerArtifactResolutionRequest,
        ) -> AppResult<crate::PreparedIndexerArtifact> {
            assert_eq!(request.indexer_id.as_deref(), Some("lease-indexer"));
            assert_eq!(request.search_facet, Some(MediaFacet::Series));
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.active.store(true, Ordering::SeqCst);
            Ok(crate::PreparedIndexerArtifact::StagedNzb(Box::new(Lease {
                active: self.active.clone(),
                staged: crate::StagedNzbRef {
                    id: "lease-test".into(),
                    compressed_path: "fixture.nzb.gz".into(),
                    raw_size_bytes: 10,
                },
            })))
        }
    }
    struct Client {
        inner: Arc<dyn DownloadClient>,
        active: Arc<AtomicBool>,
    }
    #[async_trait::async_trait]
    impl DownloadClient for Client {
        async fn submit_download(
            &self,
            request: &DownloadClientAddRequest,
        ) -> AppResult<DownloadGrabResult> {
            assert!(
                self.active.load(Ordering::SeqCst),
                "artifact lease ended before submission"
            );
            assert_eq!(
                request.staged_nzb.as_ref().map(|value| value.id.as_str()),
                Some("lease-test")
            );
            assert!(
                request.source_hint.is_none(),
                "download client must receive the artifact"
            );
            tokio::task::yield_now().await;
            assert!(self.active.load(Ordering::SeqCst));
            self.inner.submit_download(request).await
        }
    }
    let (mut app, user) = bootstrap();
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    app.services.workflow.download_submissions = download_submissions.clone();
    let active = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    app.services.integrations.indexer_artifact_resolver = Some(Arc::new(Resolver {
        active: active.clone(),
        calls: calls.clone(),
    }));
    app.services.integrations.download_client = Arc::new(Client {
        inner: app.services.integrations.download_client.clone(),
        active: active.clone(),
    });
    let (_title, _) = app
        .add_title_and_queue_download(
            &user,
            NewTitle {
                name: "Artifact Lease Show".into(),
                facet: MediaFacet::Series,
                monitored: true,
                ..Default::default()
            },
            QueuedReleaseSelection {
                indexer_id: Some("lease-indexer".into()),
                source_hint: Some("https://indexer.test/release.nzb".into()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Artifact.Lease.Show.S01E01.1080p".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(
        !active.load(Ordering::SeqCst),
        "completed submission should release artifact"
    );
    let submissions = download_submissions.store.lock().await;
    assert_eq!(submissions.len(), 1);
    assert_eq!(
        submissions[0].source_hint.as_deref(),
        Some("https://indexer.test/release.nzb")
    );
    assert_eq!(submissions[0].source_kind, Some(DownloadSourceKind::NzbUrl));
}

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
        external_ids: vec![ExternalId::new("tvdb".to_string(), "123456".to_string())],
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
        external_ids: vec![ExternalId::new("tvdb".to_string(), "654321".to_string())],
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
        external_ids: vec![ExternalId::new("tmdb".to_string(), "987654".to_string())],
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
                external_ids: vec![ExternalId::new("tvdb".to_string(), "7654321".to_string())],
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
    within_deadline(
        "the first submission to reach the downloader gate",
        first_started,
    )
    .await;
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
    // The first submission holds the title's submission lock at the gate, so
    // a second participant on that lock is the second queue parked behind it.
    wait_until(
        "the second submission to park on the title lock",
        || async {
            app.runtime
                .acquisition
                .download_submission_guards
                .title_lock_participants(&title.id)
                .await
                >= 2
        },
    )
    .await;
    *download_client.submit_gate.lock().await = None;
    gate.notify_one();

    let first = within_deadline("the first queue task", first)
        .await
        .expect("first task")
        .expect("first queue");
    let second = within_deadline("the second queue task", second)
        .await
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
    within_deadline(
        "the first submission to reach the downloader gate",
        first_started,
    )
    .await;
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
    // The first submission holds the title's submission lock at the gate, so
    // a second participant on that lock is the second queue parked behind it.
    wait_until(
        "the second submission to park on the title lock",
        || async {
            app.runtime
                .acquisition
                .download_submission_guards
                .title_lock_participants(&title.id)
                .await
                >= 2
        },
    )
    .await;
    *download_client.submit_gate.lock().await = None;
    gate.notify_one();

    let first = within_deadline("the first queue task", first)
        .await
        .expect("first task")
        .expect("first queue");
    let second = within_deadline("the second queue task", second)
        .await
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
async fn a_deleted_download_stops_conflicting_new_submissions_for_its_scope() {
    // The user-delete path settles a download just as decisively as a terminal
    // transition does, so it owes the guard caches the same invalidation. Until
    // it did, a search inside the 30s window still saw the deleted submission
    // as accepted and refused the replacement grab as a non-replaceable
    // conflict.
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let download_queue_commands = Arc::new(TrackingDownloadQueueCommandRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking_and_queue_commands(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
        download_queue_commands.clone(),
    );
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Deleted Regrab".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let queued = app
        .queue_existing_title_download(
            &user,
            &title.id,
            QueuedReleaseSelection {
                source_hint: Some("https://example.invalid/releases/original.nzb".to_string()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Deleted.Regrab.2026.720p.WEB-DL".to_string()),
                ..Default::default()
            },
            SubmissionScope::Title,
            SubmissionConflictPolicy::Abort,
        )
        .await
        .expect("first queue");
    assert!(matches!(queued, QueueDownloadOutcome::Queued(_)));

    // Inside the cache window the accepted set still blocks the scope.
    let blocked = app
        .queue_existing_title_download(
            &user,
            &title.id,
            QueuedReleaseSelection {
                source_hint: Some("https://example.invalid/releases/replacement.nzb".to_string()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Deleted.Regrab.2026.1080p.WEB-DL".to_string()),
                ..Default::default()
            },
            SubmissionScope::Title,
            SubmissionConflictPolicy::Abort,
        )
        .await
        .expect("blocked queue outcome");
    let QueueDownloadOutcome::Conflict(conflict) = blocked else {
        panic!("an accepted in-flight download should conflict its scope");
    };
    assert!(!conflict.replaceable);

    let submission = download_submissions
        .store
        .lock()
        .await
        .first()
        .cloned()
        .expect("the accepted grab should have a submission row");
    let command_id = download_queue_commands
        .seed_pending(
            submission.download_client_id.as_deref(),
            &submission.download_client_type,
            &submission.download_client_item_id,
            false,
        )
        .await;

    let token = tokio_util::sync::CancellationToken::new();
    let poller = tokio::spawn(start_background_download_delete_poller(
        app.clone(),
        token.child_token(),
    ));
    within_deadline(
        "the queued delete",
        download_queue_commands.wait_completed(&command_id),
    )
    .await;
    token.cancel();
    poller.await.expect("delete poller should stop cleanly");

    let regrabbed = app
        .queue_existing_title_download(
            &user,
            &title.id,
            QueuedReleaseSelection {
                source_hint: Some("https://example.invalid/releases/replacement.nzb".to_string()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Deleted.Regrab.2026.1080p.WEB-DL".to_string()),
                ..Default::default()
            },
            SubmissionScope::Title,
            SubmissionConflictPolicy::Abort,
        )
        .await
        .expect("regrab queue");
    assert!(
        matches!(regrabbed, QueueDownloadOutcome::Queued(_)),
        "a deleted download must not block the replacement grab for its scope"
    );
    assert_eq!(
        download_client.submitted_release_titles.lock().await.len(),
        2
    );
}

#[tokio::test]
async fn an_operator_delete_stops_conflicting_new_submissions_for_its_scope() {
    // The remove-and-reacquire gate: the operator deletes a queued download,
    // the delete worker completes it locally, and the very next queue for the
    // same scope was refused as a conflict on a download the client no longer
    // had. The delete worker never takes the tracked terminal transition that
    // calls `forget_settled_download`, so the guard's 30s accepted-submission
    // state and its client snapshots kept describing the deleted item.
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
                name: "Reacquired Lantern".into(),
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
                source_title: Some("Reacquired.Lantern.2026.720p.WEB-DL".to_string()),
                ..Default::default()
            },
            SubmissionScope::Title,
            SubmissionConflictPolicy::Abort,
        )
        .await
        .expect("first queue");
    assert!(matches!(first, QueueDownloadOutcome::Queued(_)));

    // The control: an accepted submission that nothing has ended still holds
    // its scope for the rest of the cache window.
    let blocked = app
        .queue_existing_title_download(
            &user,
            &title.id,
            QueuedReleaseSelection {
                source_hint: Some("https://example.invalid/releases/second.nzb".to_string()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Reacquired.Lantern.2026.1080p.WEB-DL".to_string()),
                ..Default::default()
            },
            SubmissionScope::Title,
            SubmissionConflictPolicy::Abort,
        )
        .await
        .expect("second queue outcome");
    let QueueDownloadOutcome::Conflict(conflict) = blocked else {
        panic!("an accepted in-flight download should still conflict its scope");
    };
    assert_eq!(conflict.state, Some(DownloadQueueState::Queued));
    assert!(!conflict.replaceable);

    // The operator delete, as the delete worker performs it: the submission is
    // finalized as ignored for its client item. The client no longer lists it.
    let submission = download_submissions
        .store
        .lock()
        .await
        .first()
        .cloned()
        .expect("the accepted submission should exist");
    download_client.queue_items.lock().await.clear();
    let outcome = crate::integration::workflow::finalize_scryer_download_ignored(
        &app,
        crate::domain_events::DomainEventActor::system(),
        ClientJobLocator::from_submission(&submission),
    )
    .await
    .expect("the delete should finalize the submission");
    assert!(matches!(
        outcome,
        crate::integration::workflow::FinalizeIgnoredOutcome::Finalized
    ));

    let reacquired = app
        .queue_existing_title_download(
            &user,
            &title.id,
            QueuedReleaseSelection {
                source_hint: Some("https://example.invalid/releases/second.nzb".to_string()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Reacquired.Lantern.2026.1080p.WEB-DL".to_string()),
                ..Default::default()
            },
            SubmissionScope::Title,
            SubmissionConflictPolicy::Abort,
        )
        .await
        .expect("reacquire queue");
    assert!(
        matches!(reacquired, QueueDownloadOutcome::Queued(_)),
        "a deleted download must not block the reacquire that follows it"
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
            stream_id: None,
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
            stream_id: None,
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
            release_listing_json: None,
        })
        .await
        .expect("record client-created seed binding");
    download_client
        .set_queue_error(Some("queue unavailable"))
        .await;
    download_client
        .set_recent_activity_error(Some("history unavailable"))
        .await;
    let error = app
        .queue_existing_title_download(
            &user,
            &title.id,
            QueuedReleaseSelection {
                source_hint: Some("https://example.invalid/second.nzb".into()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Second.Release.2026.1080p".into()),
                ..Default::default()
            },
            SubmissionScope::Title,
            SubmissionConflictPolicy::Abort,
        )
        .await
        .expect_err("a cold-cache total outage must remain a retryable deferral");
    assert!(matches!(error, AppError::DownloadSubmitUnavailable(_)));
    assert!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .is_empty()
    );
    download_client.set_queue_error(None).await;
    download_client.set_recent_activity_error(None).await;
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
async fn queue_existing_title_download_adopts_a_foreign_observation_stub_identity() {
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
                name: "Observed First".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .expect("create title");
    // The client already holds the job Scryer is about to grab, and the tracker
    // has persisted its title-less observation stub under the job's foreign
    // canonical identity.
    let foreign_download_id = scryer_domain::download_identity::DownloadId::new();
    let job_id = format!("job-for-{}", title.id);
    let stub = DownloadSubmission {
        download_id: foreign_download_id,
        title_id: String::new(),
        facet: String::new(),
        download_client_id: Some("primary".to_string()),
        download_client_type: "nzbget".to_string(),
        download_client_item_id: job_id.clone(),
        source_hint: None,
        source_provider_id: None,
        source_provider_name: None,
        source_kind: None,
        source_title: None,
        info_hash: None,
        release_size_bytes: None,
        request_signature: None,
        purpose: crate::DownloadSubmissionPurpose::Standard,
        scope: SubmissionScope::Orphan,
        release_listing_json: None,
    };
    assert!(stub.is_observation_stub());
    download_submissions
        .record_submission(stub)
        .await
        .expect("record the tracker's observation stub");

    let outcome = app
        .queue_existing_title_download(
            &user,
            &title.id,
            QueuedReleaseSelection {
                source_hint: Some("https://example.invalid/observed.nzb".to_string()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Observed.First.2026.1080p.WEB-DL".to_string()),
                ..Default::default()
            },
            SubmissionScope::Title,
            SubmissionConflictPolicy::Abort,
        )
        .await
        .expect("a grab of a job Scryer only observed adopts that job's identity");
    let QueueDownloadOutcome::Queued(queued) = outcome else {
        panic!("the adopted grab should be returned as queued");
    };

    // Scryer's grab is what now owns the job, so it is not a reuse of an
    // earlier Scryer submission.
    assert!(!queued.reused_existing);
    assert_eq!(queued.job_id, job_id);
    let submissions = download_submissions.store.lock().await;
    assert_eq!(submissions.len(), 1);
    assert_eq!(submissions[0].download_id, foreign_download_id);
    assert_eq!(submissions[0].title_id, title.id);
    assert_eq!(
        submissions[0].source_title.as_deref(),
        Some("Observed.First.2026.1080p.WEB-DL")
    );
    assert!(submissions[0].request_signature.is_some());
    assert!(!submissions[0].is_observation_stub());
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
            release_listing_json: None,
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
            release_listing_json: None,
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
            release_listing_json: None,
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
            release_listing_json: None,
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
            release_listing_json: None,
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
            release_listing_json: None,
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
                release_listing_json: None,
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
                release_listing_json: None,
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
            release_listing_json: None,
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
            release_listing_json: None,
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
            proxy_config_id: None,
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
            proxy_config_id: None,
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
            proxy_config_id: None,
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
            proxy_config_id: None,
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
            proxy_config_id: None,
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

    // Creation is registry-gated now, so the vocabulary has to exist before a
    // title can be born carrying it.
    app.create_title_tag_definition(&user, "anime-hd", None)
        .await
        .expect("tag should be defined");
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
        .release_search_title_for_wanted_item(&title, &wanted, None, None)
        .await;
    let subject = app
        .resolve_release_search_subject_for_wanted_item(&title, &search_title, &wanted, None)
        .await
        .expect("subject should resolve");

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
            proxy_config_id: None,
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
    // Creation is registry-gated now, so the vocabulary has to exist before a
    // title can be born carrying it.
    app.create_title_tag_definition(user, "anime-hd", None)
        .await
        .expect("tag should be defined");
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
        .release_search_title_for_wanted_item(&title, &wanted, None, None)
        .await;
    let subject = app
        .resolve_release_search_subject_for_wanted_item(&title, &search_title, &wanted, None)
        .await
        .expect("subject should resolve");
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
        &[],
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
    app.record_search_coverage(&title, &subject, &convergence.routed_indexer_ids, &[])
        .await;
    assert!(
        scope_is_converged(&app, &title, &subject).await,
        "scope converges once every routed indexer is covered under the current fingerprint"
    );
}

/// Indexer plugin whose declared capabilities the test can change, standing in
/// for a plugin update.
struct SwitchableCapsPluginProvider {
    capabilities: std::sync::Mutex<scryer_domain::IndexerProviderCapabilities>,
}

impl SwitchableCapsPluginProvider {
    fn set(&self, capabilities: scryer_domain::IndexerProviderCapabilities) {
        *self.capabilities.lock().expect("capabilities") = capabilities;
    }
}

impl IndexerPluginProvider for SwitchableCapsPluginProvider {
    fn client_for_provider(&self, _config: &IndexerConfig) -> Option<Arc<dyn IndexerClient>> {
        None
    }

    fn available_provider_types(&self) -> Vec<String> {
        vec!["newznab".to_string()]
    }

    fn scoring_policies(&self) -> Vec<scryer_rules::UserPolicy> {
        Vec::new()
    }

    fn capabilities_for_provider(
        &self,
        _provider_type: &str,
    ) -> scryer_domain::IndexerProviderCapabilities {
        self.capabilities.lock().expect("capabilities").clone()
    }
}

fn anime_only_capabilities() -> scryer_domain::IndexerProviderCapabilities {
    scryer_domain::IndexerProviderCapabilities {
        supported_ids: HashMap::from([("anime".into(), vec!["anidb_id".into()])]),
        query_param: Some("q".into()),
        supported_query_facets: vec!["anime".into()],
        search: true,
        anidb_search: true,
        ..Default::default()
    }
}

#[tokio::test]
async fn an_indexer_that_cannot_serve_a_scope_stays_covered_until_it_or_its_plugin_changes() {
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
    let plugins = Arc::new(SwitchableCapsPluginProvider {
        capabilities: std::sync::Mutex::new(anime_only_capabilities()),
    });
    let app = app.with_test_overrides(|builder| {
        builder
            .with_scope_indexer_coverage_store(coverage.clone())
            .with_plugin_provider(plugins.clone())
    });
    let (title, subject) = convergence_test_title_and_subject(&app, &user).await;
    let convergence = app
        .resolve_scope_convergence(&title, &subject)
        .await
        .expect("routed convergence coordinates");
    let uncovered = || async {
        app.uncovered_indexers_for_scope(
            &convergence.scope_key,
            &convergence.facet,
            &convergence.fingerprint,
            &convergence.routed_indexer_ids,
        )
        .await
        .unwrap()
    };

    // indexer-a searched; indexer-b could not serve the scope. Both count, so
    // the scope converges instead of re-searching indexer-b every cycle.
    app.record_search_coverage(
        &title,
        &subject,
        &["indexer-a".to_string()],
        &["indexer-b".to_string()],
    )
    .await;
    assert!(
        uncovered().await.is_empty(),
        "an unsupported indexer is covered"
    );

    // A plugin update that changes what the provider declares opens the scope
    // for the indexer recorded as unable to serve it, and only that one.
    let mut with_movies = anime_only_capabilities();
    with_movies
        .supported_ids
        .insert("movie".into(), vec!["imdb_id".into()]);
    with_movies.supported_query_facets.push("movie".into());
    plugins.set(with_movies);
    assert_eq!(uncovered().await, vec!["indexer-b".to_string()]);

    // Back to the recorded capabilities, the receipt matches again.
    plugins.set(anime_only_capabilities());
    assert!(uncovered().await.is_empty());

    // Editing the indexer opens it too.
    app.services
        .integrations
        .indexer_configs
        .update(crate::IndexerConfigUpdate {
            id: "indexer-b".to_string(),
            config_json: Some(r#"{"categories":"2000"}"#.to_string()),
            ..Default::default()
        })
        .await
        .expect("indexer update");
    assert_eq!(uncovered().await, vec!["indexer-b".to_string()]);

    // So does a caps refresh that changes the stored snapshot.
    app.record_search_coverage(&title, &subject, &[], &["indexer-b".to_string()])
        .await;
    assert!(uncovered().await.is_empty());
    app.services
        .integrations
        .indexer_configs
        .update(crate::IndexerConfigUpdate {
            id: "indexer-b".to_string(),
            caps_snapshot_json: Some(Some(r#"{"searching":{}}"#.to_string())),
            ..Default::default()
        })
        .await
        .expect("caps refresh");
    assert_eq!(uncovered().await, vec!["indexer-b".to_string()]);
}

#[tokio::test]
async fn a_walk_coverage_snapshot_skips_covered_scopes_and_rereads_the_rest() {
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
    let covered = app
        .resolve_scope_convergence(&title, &subject)
        .await
        .expect("routed convergence coordinates");
    app.record_convergence_coverage(&covered, &covered.routed_indexer_ids, &[])
        .await;
    let mut later = covered.clone();
    later.scope_key = "episode:snapshot-later".to_string();
    let mut unloaded = covered.clone();
    unloaded.scope_key = "episode:snapshot-unloaded".to_string();
    app.record_convergence_coverage(&unloaded, &unloaded.routed_indexer_ids, &[])
        .await;

    let memo = crate::acquisition::convergence::ConvergenceInputMemo::default();
    app.load_walk_coverage(
        &memo,
        vec![covered.scope_key.clone(), later.scope_key.clone()],
    )
    .await;
    assert_eq!(coverage.list_calls(), 1, "one read loads the walk");

    // Covered in the snapshot: skipped without a read, however often asked.
    for _ in 0..3 {
        assert!(
            app.uncovered_indexers_for_scope_memoized(&covered, &memo)
                .await
                .unwrap()
                .is_empty()
        );
    }
    assert_eq!(coverage.list_calls(), 1, "a covered scope costs no read");

    // Uncovered in the snapshot: the store is asked again, so the answer is
    // the unmemoized one.
    assert_eq!(
        app.uncovered_indexers_for_scope_memoized(&later, &memo)
            .await
            .unwrap(),
        later.routed_indexer_ids
    );
    assert_eq!(coverage.list_calls(), 2);

    // A receipt written after the snapshot, as a search earlier in the walk
    // writes one, is seen.
    app.record_convergence_coverage(&later, &later.routed_indexer_ids, &[])
        .await;
    assert!(
        app.uncovered_indexers_for_scope_memoized(&later, &memo)
            .await
            .unwrap()
            .is_empty()
    );

    // A scope the walk never loaded reads its own rows.
    let reads_before = coverage.list_calls();
    assert!(
        app.uncovered_indexers_for_scope_memoized(&unloaded, &memo)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(coverage.list_calls(), reads_before + 1);
}

#[tokio::test]
async fn a_covered_background_walk_builds_no_title_match_evidence() {
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let indexer_client = Arc::new(
        FixedReleaseIndexerClient::new("Evidence Budget Fixture.2024.1080p.WEB-DL")
            .with_fired_indexers(["indexer-a", "indexer-b"])
            .with_empty_response(),
    );
    let (app, user, _, repos) = bootstrap_with_acquisition_tracking_and_indexer_and_repos(
        Arc::new(StubDownloadClient::default()),
        Arc::new(TrackingDownloadSubmissionRepo::default()),
        Arc::new(TrackingPendingReleaseRepo::default()),
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
    let (title, wanted_id) = seed_movie_wanted_for_acquisition(
        &app,
        &user,
        &wanted_items,
        "Evidence Budget Fixture",
        2024,
    )
    .await;
    let wanted = wanted_items
        .get_acquisition_scope_state_by_id(&wanted_id)
        .await
        .expect("read wanted")
        .expect("seeded wanted row");
    let search_title = app
        .release_search_title_for_wanted_item(&title, &wanted, None, None)
        .await;
    let subject = app
        .resolve_release_search_subject_for_wanted_item(&title, &search_title, &wanted, None)
        .await
        .expect("subject should resolve");
    let convergence = app
        .resolve_scope_convergence(&search_title, &subject)
        .await
        .expect("resolve live convergence coordinates");
    app.record_convergence_coverage(&convergence, &convergence.routed_indexer_ids, &[])
        .await;

    let catalog_loads = || {
        repos
            .titles
            .list_for_matching_calls
            .load(std::sync::atomic::Ordering::SeqCst)
    };
    let loads_before = catalog_loads();
    let coverage_reads_before = coverage.list_calls();
    app.run_background_acquisition_cycle_once().await;

    assert!(
        indexer_client.requested_indexer_id_sets().await.is_empty(),
        "a covered scope spends no indexer query"
    );
    assert_eq!(
        catalog_loads(),
        loads_before,
        "a stage the gate skips builds no collision or spelling evidence"
    );
    assert_eq!(
        coverage.list_calls() - coverage_reads_before,
        1,
        "the walk reads its coverage once"
    );

    // Uncovered, the same stage builds its evidence and searches.
    app.prune_scope_key_coverage(&convergence.scope_key, None)
        .await;
    app.run_background_acquisition_cycle_once().await;
    assert!(
        !indexer_client.requested_indexer_id_sets().await.is_empty(),
        "an uncovered scope searches"
    );
    assert!(
        catalog_loads() > loads_before,
        "a searched stage builds its title-match evidence"
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

    app.record_search_coverage(&title, &subject, &convergence.routed_indexer_ids, &[])
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

    app.record_search_coverage(&title, &subject, &convergence.routed_indexer_ids, &[])
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

    app.record_search_coverage(&title, &subject, &convergence.routed_indexer_ids, &[])
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
        .release_search_title_for_wanted_item(&title, &wanted, None, None)
        .await;
    let subject = app
        .resolve_release_search_subject_for_wanted_item(&title, &search_title, &wanted, None)
        .await
        .expect("subject should resolve");
    let convergence = app
        .resolve_scope_convergence(&title, &subject)
        .await
        .expect("resolve live convergence coordinates");
    app.record_search_coverage(&title, &subject, &convergence.routed_indexer_ids, &[])
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

/// Convergence resolves only the quality profile — not the whole upgrade
/// context — and a title walk memoizes the per-title inputs. Neither may move
/// the fingerprint: coverage rows written under the full-context resolution
/// must stay valid, including under a category profile override and
/// non-default required audio languages.
#[tokio::test]
async fn convergence_fingerprint_matches_the_full_upgrade_context_resolution() {
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_SYSTEM,
            QUALITY_PROFILE_ID_KEY,
            "\"global-profile\"",
        )
        .await;
    for category in ["movie", "series", "anime"] {
        settings
            .set_scoped_value(
                SETTINGS_SCOPE_SYSTEM,
                QUALITY_PROFILE_ID_KEY,
                category,
                "\"category-profile\"",
            )
            .await;
    }
    settings
        .set_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            REQUIRED_AUDIO_LANGUAGES_KEY,
            "anime",
            "[\"ja\"]",
        )
        .await;
    settings
        .set_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            REQUIRED_AUDIO_LANGUAGES_KEY,
            "movie",
            "[\"en\",\"ja\"]",
        )
        .await;
    let mut category_profile = test_quality_profile("category-profile");
    category_profile.criteria.cutoff_tier = Some("1080p".to_string());
    let quality_profiles = Arc::new(StoredQualityProfileRepo::default());
    quality_profiles
        .set_profiles(vec![
            test_quality_profile("global-profile"),
            category_profile,
        ])
        .await;
    let (app, user) = bootstrap_with_settings_repo_and_profiles(
        settings,
        quality_profiles,
        Arc::new(MockIndexerClient),
    );
    for indexer_id in ["indexer-a", "indexer-b"] {
        app.services
            .integrations
            .indexer_configs
            .create(synthetic_direct_nab_indexer_config(indexer_id, "newznab"))
            .await
            .expect("create routed indexer");
    }
    let (title, subject) = convergence_test_title_and_subject(&app, &user).await;
    // A series-movie stage searches under a movie-facet record of the same
    // title, whose required audio languages differ. One memo serves both, as
    // it does within a walk.
    let mut movie_record = title.clone();
    movie_record.facet = MediaFacet::Movie;
    let memo = crate::acquisition::convergence::ConvergenceInputMemo::default();

    let mut fingerprints = Vec::new();
    for record in [&title, &movie_record] {
        let context = app
            .resolve_upgrade_context_for_title_with_category_and_quality(
                record,
                Some(subject.category.as_str()),
                None,
            )
            .await
            .expect("full upgrade context resolves");
        assert_eq!(
            context.profile.id, "category-profile",
            "fixture: the category override governs the scope"
        );
        let audio = app
            .resolve_required_audio_languages_for_title(record)
            .await
            .expect("required audio languages resolve");
        assert!(!audio.is_empty(), "fixture: required audio is non-default");
        let full_context_fingerprint = crate::acquisition::convergence::compute_search_fingerprint(
            &context.profile.id,
            &crate::acquisition::convergence::profile_criteria_version(&context.profile.criteria),
            &audio,
            &crate::acquisition::convergence::scope_match_identity(&subject),
        );

        let unmemoized = app
            .resolve_scope_convergence(record, &subject)
            .await
            .expect("routed convergence coordinates");
        assert_eq!(unmemoized.fingerprint, full_context_fingerprint);
        // Twice: the first call fills the memo, the second answers from it.
        for _ in 0..2 {
            let memoized = app
                .resolve_scope_convergence_memoized(record, &subject, &memo)
                .await
                .expect("routed convergence coordinates");
            assert_eq!(memoized.fingerprint, full_context_fingerprint);
            let mut routed = memoized.routed_indexer_ids.clone();
            routed.sort();
            assert_eq!(
                routed,
                vec!["indexer-a".to_string(), "indexer-b".to_string()]
            );
        }
        fingerprints.push(full_context_fingerprint);
    }
    assert_ne!(
        fingerprints[0], fingerprints[1],
        "the memo must not hand the movie-facet record the series' audio languages"
    );
}

/// A background stage skipped as covered pays only for its convergence
/// coordinates. The scoring persona and the acquisition thresholds belong to
/// candidate evaluation, which a covered stage never reaches.
#[tokio::test]
async fn covered_background_walk_reads_no_persona_or_acquisition_thresholds() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let indexer_client = Arc::new(
        FixedReleaseIndexerClient::new("Covered Walk Fixture.2024.1080p.WEB-DL")
            .with_fired_indexers(["indexer-a", "indexer-b"])
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
    let settings = Arc::new(StoredSettingsRepo::default());
    let coverage = Arc::new(RecordingScopeIndexerCoverageRepo::new());
    let app = app.with_test_overrides(|builder| {
        builder
            .with_scope_indexer_coverage_store(coverage.clone())
            .with_settings(settings.clone())
    });

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Covered Walk Fixture".into(),
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
        .release_search_title_for_wanted_item(&title, &wanted, None, None)
        .await;
    let subject = app
        .resolve_release_search_subject_for_wanted_item(&title, &search_title, &wanted, None)
        .await
        .expect("subject should resolve");
    let convergence = app
        .resolve_scope_convergence(&search_title, &subject)
        .await
        .expect("resolve live convergence coordinates");
    app.record_search_coverage(
        &search_title,
        &subject,
        &convergence.routed_indexer_ids,
        &[],
    )
    .await;
    assert!(
        scope_is_converged(&app, &search_title, &subject).await,
        "fixture: every routed indexer is covered"
    );

    settings.reset_read_log();
    app.run_background_acquisition_cycle_once().await;

    assert!(
        indexer_client.requested_indexer_id_sets().await.is_empty(),
        "a covered scope spends no indexer query"
    );
    // Target derivation reads the persona once per cycle, however many stages
    // follow. The cycle reads its rotation cursor after derivation and before
    // the first title walk, so everything after that read is the walk's.
    let reads = settings.read_log();
    let walk_start = reads
        .iter()
        .position(|key| {
            key == crate::acquisition::convergence::BACKGROUND_ACQUISITION_RESUME_AFTER_KEY
        })
        .expect("the cycle reads its rotation cursor before walking");
    let walk_reads = &reads[walk_start..];
    assert!(
        walk_reads
            .iter()
            .any(|key| key == INDEXER_ROUTING_SETTINGS_KEY),
        "the stage reached the convergence gate: {walk_reads:?}"
    );
    // The acquisition thresholds are the only reader of the last key.
    for key in [SCORING_PERSONA_KEY, "acquisition.same_tier_min_delta"] {
        assert!(
            !walk_reads.iter().any(|read| read == key),
            "a covered stage never resolves the persona or the acquisition thresholds ({key}): {walk_reads:?}"
        );
    }
}

/// The rotation cursors are written only when they move. A cycle that ends
/// where the stored cursor already points leaves the settings row alone; a
/// cycle that moves it writes the new position.
#[tokio::test]
async fn background_cycle_writes_the_rotation_cursor_only_when_it_moves() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let indexer_client = Arc::new(
        FixedReleaseIndexerClient::new("Cursor Write Fixture.2024.1080p.WEB-DL")
            .with_fired_indexers(["indexer-a"])
            .with_empty_response(),
    );
    let (app, user) = bootstrap_with_acquisition_tracking_and_indexer(
        download_client,
        download_submissions,
        pending_releases,
        wanted_items.clone(),
        indexer_client,
    );
    let settings = Arc::new(StoredSettingsRepo::default());
    let coverage = Arc::new(RecordingScopeIndexerCoverageRepo::new());
    let app = app.with_test_overrides(|builder| {
        builder
            .with_scope_indexer_coverage_store(coverage.clone())
            .with_settings(settings.clone())
    });
    seed_movie_wanted_for_acquisition(&app, &user, &wanted_items, "Cursor Write Fixture", 2024)
        .await;

    // A freshly added title is hot, so it rotates through the hot lane.
    let cursor_key = crate::acquisition::convergence::BACKGROUND_ACQUISITION_HOT_RESUME_AFTER_KEY;
    settings
        .set_value(
            SETTINGS_SCOPE_SYSTEM,
            cursor_key,
            "\"scope-that-no-longer-exists\"",
        )
        .await;

    settings.reset_write_log();
    let outcome = app.run_background_acquisition_cycle_once().await;
    assert_eq!(
        outcome.targets_derived, 1,
        "fixture: one missing movie scope"
    );
    assert_eq!(
        settings
            .write_log()
            .iter()
            .filter(|key| key.as_str() == cursor_key)
            .count(),
        1,
        "a cursor that moved is written: {:?}",
        settings.write_log()
    );
    let moved_to = app
        .background_acquisition_hot_resume_position()
        .await
        .expect("the moved cursor reads back");
    assert_ne!(moved_to, "scope-that-no-longer-exists");

    settings.reset_write_log();
    app.run_background_acquisition_cycle_once().await;
    assert!(
        settings.write_log().is_empty(),
        "an unchanged cursor is not written again: {:?}",
        settings.write_log()
    );
    assert_eq!(
        app.background_acquisition_hot_resume_position()
            .await
            .as_deref(),
        Some(moved_to.as_str())
    );

    // The store call compares against the position the cycle read, in the
    // trimmed form reads return.
    settings.reset_write_log();
    app.store_background_acquisition_resume_position(Some(&moved_to), Some(&moved_to))
        .await;
    app.store_background_acquisition_hot_resume_position(None, None)
        .await;
    assert!(settings.write_log().is_empty());
    app.store_background_acquisition_hot_resume_position(None, Some("hot-scope"))
        .await;
    assert_eq!(
        settings.write_log(),
        vec![crate::acquisition::convergence::BACKGROUND_ACQUISITION_HOT_RESUME_AFTER_KEY]
    );
}

/// A library of covered movies and one open movie, with the rotation set so
/// the open movie is the last scope a cycle reaches, and a batch of one.
async fn covered_movies_ahead_of_an_open_movie(
    covered_count: usize,
) -> (AppUseCase, Arc<FixedReleaseIndexerClient>) {
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let indexer_client = Arc::new(
        FixedReleaseIndexerClient::new("Top Up Open Fixture.2024.1080p.WEB-DL")
            .with_fired_indexers(["indexer-a"])
            .with_empty_response(),
    );
    let (app, user) = bootstrap_with_acquisition_tracking_and_indexer(
        Arc::new(StubDownloadClient::default()),
        Arc::new(TrackingDownloadSubmissionRepo::default()),
        Arc::new(TrackingPendingReleaseRepo::default()),
        wanted_items.clone(),
        indexer_client.clone(),
    );
    app.services
        .integrations
        .indexer_configs
        .delete("acquisition-indexer")
        .await
        .expect("remove bootstrap indexer");
    app.services
        .integrations
        .indexer_configs
        .create(synthetic_direct_nab_indexer_config("indexer-a", "newznab"))
        .await
        .expect("create routed indexer");
    let settings = Arc::new(StoredSettingsRepo::default());
    let coverage = Arc::new(RecordingScopeIndexerCoverageRepo::new());
    let app = app.with_test_overrides(|builder| {
        builder
            .with_scope_indexer_coverage_store(coverage.clone())
            .with_settings(settings.clone())
    });

    for number in 0..covered_count {
        let (title, wanted_id) = seed_movie_wanted_for_acquisition(
            &app,
            &user,
            &wanted_items,
            &format!("Top Up Covered Fixture {number}"),
            2024,
        )
        .await;
        let wanted = wanted_items
            .get_acquisition_scope_state_by_id(&wanted_id)
            .await
            .expect("load wanted scope")
            .expect("wanted scope exists");
        let search_title = app
            .release_search_title_for_wanted_item(&title, &wanted, None, None)
            .await;
        let subject = app
            .resolve_release_search_subject_for_wanted_item(&title, &search_title, &wanted, None)
            .await
            .expect("subject should resolve");
        let convergence = app
            .resolve_scope_convergence(&search_title, &subject)
            .await
            .expect("resolve live convergence coordinates");
        app.record_search_coverage(
            &search_title,
            &subject,
            &convergence.routed_indexer_ids,
            &[],
        )
        .await;
        assert!(
            scope_is_converged(&app, &search_title, &subject).await,
            "fixture: the movie is covered"
        );
    }
    let (open_title, _) =
        seed_movie_wanted_for_acquisition(&app, &user, &wanted_items, "Top Up Open Fixture", 2024)
            .await;

    // A rotation resumes after its cursor, so a cursor on the open movie puts
    // it last, in whichever lane it sits.
    let targets = app
        .derive_acquisition_targets(&Utc::now())
        .await
        .expect("derive targets");
    assert_eq!(
        targets.len(),
        covered_count + 1,
        "fixture: every movie is due"
    );
    let open_scope_key = targets
        .iter()
        .find(|target| target.title_id == open_title.id)
        .expect("the open movie is a target")
        .scope_key
        .clone();
    let cursor = serde_json::to_string(&open_scope_key).expect("encode cursor");
    for key in [
        crate::acquisition::convergence::BACKGROUND_ACQUISITION_HOT_RESUME_AFTER_KEY,
        crate::acquisition::convergence::BACKGROUND_ACQUISITION_RESUME_AFTER_KEY,
    ] {
        settings
            .set_value(SETTINGS_SCOPE_SYSTEM, key, &cursor)
            .await;
    }
    settings
        .set_value(
            SETTINGS_SCOPE_SYSTEM,
            crate::acquisition::convergence::ACQUISITION_LONG_TAIL_BACKFILL_MAX_SCOPES_PER_CYCLE_KEY,
            "1",
        )
        .await;
    (app, indexer_client)
}

/// A scope whose walk had nothing to do does not use up the batch: the cycle
/// carries on along the rotation until it has spent the batch on a scope that
/// needed it.
#[tokio::test]
async fn covered_scopes_do_not_use_up_the_batch() {
    let (app, indexer_client) = covered_movies_ahead_of_an_open_movie(2).await;

    let outcome = app.run_background_acquisition_cycle_once().await;

    assert_eq!(
        outcome.titles_walked, 3,
        "two covered movies, then the open one"
    );
    assert_eq!(
        indexer_client.requested_indexer_id_sets().await.len(),
        1,
        "the open movie is searched in the same cycle"
    );
}

/// The cycle follows idle scopes only so far: a run of them longer than the
/// bound ends the cycle, and the rotation carries on from there next time.
#[tokio::test]
async fn a_cycle_stops_following_covered_scopes_at_its_bound() {
    let (app, indexer_client) = covered_movies_ahead_of_an_open_movie(5).await;

    let first = app.run_background_acquisition_cycle_once().await;
    assert_eq!(first.titles_walked, 4, "a batch of one follows four scopes");
    assert!(indexer_client.requested_indexer_id_sets().await.is_empty());

    let second = app.run_background_acquisition_cycle_once().await;
    assert_eq!(
        second.titles_walked, 2,
        "the last covered movie, then the open one"
    );
    assert_eq!(indexer_client.requested_indexer_id_sets().await.len(), 1);
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
        .release_search_title_for_wanted_item(&title, &wanted, None, None)
        .await;
    let subject = app
        .resolve_release_search_subject_for_wanted_item(&title, &search_title, &wanted, None)
        .await
        .expect("subject should resolve");
    let convergence = app
        .resolve_scope_convergence(&title, &subject)
        .await
        .expect("resolve live convergence coordinates");
    app.record_search_coverage(&title, &subject, &convergence.routed_indexer_ids, &[])
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
        release_listing_json: None,
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
            release_listing_json: None,
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
            app.runtime.environment.now(),
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
            app.runtime.environment.now(),
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
    app.record_search_coverage(&title, &subject, &convergence.routed_indexer_ids, &[])
        .await;
    assert!(
        scope_is_converged(&app, &title, &subject).await,
        "coverage under the current fingerprint converges the scope"
    );
}

#[tokio::test]
async fn pre_release_match_identity_differs_only_by_its_phase_suffix() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let (app, user) = bootstrap_with_search_settings_indexer_and_configs(
        settings,
        Arc::new(MockIndexerClient),
        vec![synthetic_direct_nab_indexer_config("indexer-a", "newznab")],
    );
    let (_, released) = convergence_test_title_and_subject(&app, &user).await;
    assert!(!released.pre_release, "fixture: the subject is released");
    let mut pre_release = released.clone();
    pre_release.pre_release = true;

    let released_identity = crate::acquisition::convergence::scope_match_identity(&released);
    assert!(
        !released_identity.contains("phase="),
        "released identities are unchanged"
    );
    assert_eq!(
        crate::acquisition::convergence::scope_match_identity(&pre_release),
        format!("{released_identity};phase=pre")
    );
}

#[tokio::test]
async fn pre_release_coverage_does_not_cover_the_released_scope() {
    // An empty search before air proves nothing about the released catalog;
    // once the scope is released, every routed indexer is uncovered again.
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

    let (title, released) = convergence_test_title_and_subject(&app, &user).await;
    let mut pre_release = released.clone();
    pre_release.pre_release = true;

    app.record_search_coverage(
        &title,
        &pre_release,
        &["indexer-a".to_string(), "indexer-b".to_string()],
        &[],
    )
    .await;
    assert!(
        scope_is_converged(&app, &title, &pre_release).await,
        "fixture: the pre-release search covered the scope"
    );

    let convergence = app
        .resolve_scope_convergence(&title, &released)
        .await
        .expect("routed convergence coordinates");
    let mut uncovered = app
        .uncovered_indexers_for_scope(
            &convergence.scope_key,
            &convergence.facet,
            &convergence.fingerprint,
            &convergence.routed_indexer_ids,
        )
        .await
        .expect("coverage read");
    uncovered.sort();
    assert_eq!(
        uncovered,
        vec!["indexer-a".to_string(), "indexer-b".to_string()],
        "released scope is searched again on every routed indexer"
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
        &[],
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
async fn routed_indexers_exclude_indexers_without_automatic_search() {
    // The background search never queries an indexer with automatic search
    // off, so it can never earn a receipt; routing a scope to it would keep
    // that scope in the walk every cycle.
    let settings = Arc::new(StoredSettingsRepo::default());
    let mut manual_only_b = synthetic_direct_nab_indexer_config("indexer-b", "newznab");
    manual_only_b.enable_auto_search = false;
    // indexer-c is searchable but unrouted, so it proves the plan is in force.
    let configs = vec![
        synthetic_direct_nab_indexer_config("indexer-a", "newznab"),
        manual_only_b,
        synthetic_direct_nab_indexer_config("indexer-c", "newznab"),
    ];
    let (app, user) = bootstrap_with_search_settings_indexer_and_configs(
        settings.clone(),
        Arc::new(MockIndexerClient),
        configs,
    );
    let coverage = Arc::new(RecordingScopeIndexerCoverageRepo::new());
    let app = app
        .with_test_overrides(|builder| builder.with_scope_indexer_coverage_store(coverage.clone()));

    let (title, subject) = convergence_test_title_and_subject(&app, &user).await;
    settings
        .set_scoped_value(
            crate::SETTINGS_SCOPE_SYSTEM,
            crate::INDEXER_ROUTING_SETTINGS_KEY,
            &title.library_id,
            &serde_json::json!({
                "indexer-a": { "enabled": true, "categories": [], "priority": 1 },
                "indexer-b": { "enabled": true, "categories": [], "priority": 2 }
            })
            .to_string(),
        )
        .await;

    let convergence = app
        .resolve_scope_convergence(&title, &subject)
        .await
        .expect("routed convergence coordinates");
    assert_eq!(
        convergence.routed_indexer_ids,
        vec!["indexer-a".to_string()],
        "the plan routes both, but only the auto-search indexer is searchable"
    );

    app.record_search_coverage(&title, &subject, &["indexer-a".to_string()], &[])
        .await;
    assert!(
        scope_is_converged(&app, &title, &subject).await,
        "covering the auto-search indexer converges the scope"
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
    app.record_search_coverage(&title, &subject, &["indexer-a".to_string()], &[])
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

/// The acquisition walk asks the same question RSS does: does this release
/// name the title? A cour's own name lives only in the anime numbering
/// bridge, so a romanized cour-numbered release proved nothing against the
/// walk's subject evidence and every result was discarded in silence.
#[tokio::test]
async fn wanted_item_subject_evidence_carries_the_anime_bridge_cour_names() {
    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Fullmetal Alchemist Brotherhood".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .expect("create anime title");

    let bridge = scryer_domain::AnimeNumberingBridge {
        source: Default::default(),
        generated_on: "2026-01-01".into(),
        corroborating_order: None,
        seasons: vec![scryer_domain::AnimeCommunitySeason {
            index: 4,
            titles: vec![
                "Hagane no Renkinjutsushi Saigo no Gassho o Utau Toki no Hikari to Kage no Uta"
                    .into(),
            ],
            absolute_start: Some(37),
            ..Default::default()
        }],
    };
    app.services
        .catalog
        .shows
        .replace_anime_numbering_bridge(&title.id, Some(&bridge))
        .await
        .expect("store the anime numbering bridge");

    let now = Utc::now().to_rfc3339();
    let wanted = AcquisitionScopeState {
        id: Id::new().0,
        title_id: title.id.clone(),
        title_name: Some(title.name.clone()),
        title_slug: title.slug.clone(),
        title_facet: Some("anime".to_string()),
        library_id: Some(title.library_id.clone()),
        library_name: None,
        library_slug: None,
        episode_id: None,
        collection_id: None,
        series_movie_link_id: None,
        season_number: Some("1".to_string()),
        episode_number: Some("59".to_string()),
        media_type: "episode".to_string(),
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
        .release_search_title_for_wanted_item(&title, &wanted, None, None)
        .await;
    let subject = app
        .resolve_release_search_subject_for_wanted_item(&title, &search_title, &wanted, None)
        .await
        .expect("subject should resolve");

    let release = "Hagane no Renkinjutsushi Saigo no Gasshou wo Utau Toki no Hikari to Kage no Uta - 23.720p.WEB-DL.AV1.AAC2.0-NTb";
    let parsed = crate::release_parser::parse_release_metadata_for_target(
        release,
        &subject.title_evidence.parse_context,
    );

    assert!(
        crate::acquisition_release_search::parsed_release_matches_title_evidence(
            &parsed,
            &subject.title_evidence
        ),
        "the walk must recognise a release named after a bridge cour"
    );
}

/// Sonarr's `QueueSpecification` reads the live queue and nothing else: a
/// submission the client no longer lists cannot block a new grab, and a client
/// that is answering needs no separate authority proof.
#[tokio::test]
async fn a_submission_the_client_no_longer_lists_does_not_block_a_new_grab() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let registry = Arc::new(super::downloads::RecordingDownloadRegistry::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );
    let app =
        base_app.with_test_overrides(|services| services.with_download_registry(registry.clone()));
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Fixture Vanished Claim".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .expect("create title");
    let prior_download_id = scryer_domain::download_identity::DownloadId::new();
    download_submissions
        .record_submission(DownloadSubmission {
            download_id: prior_download_id,
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
            source_title: Some("Fixture.First.2026.1080p.WEB-DL".to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: SubmissionScope::Title,
            release_listing_json: None,
        })
        .await
        .expect("record prior submission");
    registry
        .bind(
            ClientJobLocator::new(Some("primary"), "nzbget", "missing-primary-job"),
            prior_download_id,
        )
        .await;
    // Another client's read succeeded; this one's did not. The binding is still
    // live. Before the convergence that was "unavailable"; now the listing is
    // the only fact, and no client is in backoff.
    download_client
        .set_snapshot_authoritative_client_ids(["secondary".to_string()])
        .await;

    app.queue_existing_title_download(
        &user,
        &title.id,
        QueuedReleaseSelection {
            source_hint: Some("https://example.invalid/second.nzb".to_string()),
            source_kind: Some(DownloadSourceKind::NzbUrl),
            source_title: Some("Fixture.Second.2026.1080p.WEB-DL".to_string()),
            ..Default::default()
        },
        SubmissionScope::Title,
        SubmissionConflictPolicy::Abort,
    )
    .await
    .expect("a job no client lists cannot hold the scope");
    assert_eq!(
        download_client.submitted_release_titles.lock().await.len(),
        1,
        "no binding may block a grab"
    );
}

/// The one case where silence is not evidence: while the client that ran the
/// submission is in failure backoff, its listing proves nothing, so the
/// acquisition fails closed and names the client.
#[tokio::test]
async fn a_submission_on_a_blocked_client_fails_closed_until_the_client_returns() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );
    let disabled_until = Utc::now() + chrono::Duration::minutes(15);
    let status = Arc::new(
        super::downloads::RecordingDownloadClientStatusRepo::with_blocked_client(
            "primary",
            disabled_until,
        ),
    );
    let app = base_app
        .with_test_overrides(|services| services.with_download_client_status(status.clone()));
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Fixture Blocked Client".into(),
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
            download_client_item_id: "unlisted-primary-job".to_string(),
            source_hint: None,
            source_provider_id: None,
            source_provider_name: None,
            source_kind: Some(DownloadSourceKind::NzbUrl),
            source_title: Some("Fixture.Held.2026.1080p.WEB-DL".to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: SubmissionScope::Title,
            release_listing_json: None,
        })
        .await
        .expect("record prior submission");

    let error = app
        .queue_existing_title_download(
            &user,
            &title.id,
            QueuedReleaseSelection {
                source_hint: Some("https://example.invalid/second.nzb".to_string()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Fixture.Replacement.2026.1080p.WEB-DL".to_string()),
                ..Default::default()
            },
            SubmissionScope::Title,
            SubmissionConflictPolicy::Abort,
        )
        .await
        .expect_err("a blocked client's silence is not evidence");
    match error {
        AppError::DownloadSubmitUnavailable(message) => {
            assert!(
                message.contains("primary") && message.contains("failure backoff"),
                "the refusal names the blocked client: {message}"
            );
        }
        other => panic!("unexpected error: {other}"),
    }
    assert!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .is_empty()
    );
}

/// A client-reported terminal row is replaced on sight: the client answered,
/// and only a backoff window could make that answer untrustworthy.
#[tokio::test]
async fn a_failed_queue_row_is_replaced_without_asking_for_snapshot_authority() {
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
                name: "Fixture Terminal Row".into(),
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
            source_title: Some("Fixture.Failed.2026.1080p.WEB-DL".to_string()),
            info_hash: None,
            release_size_bytes: None,
            request_signature: None,
            scope: SubmissionScope::Title,
            release_listing_json: None,
        })
        .await
        .expect("record prior submission");
    let mut failed_item =
        queue_history_fixture_item("failed-primary-job", DownloadQueueState::Failed, 0);
    failed_item.client_id = "primary".to_string();
    download_client.history_items.lock().await.push(failed_item);
    download_client
        .set_snapshot_authoritative_client_ids(["secondary".to_string()])
        .await;

    app.queue_existing_title_download(
        &user,
        &title.id,
        QueuedReleaseSelection {
            source_hint: Some("https://example.invalid/replacement.nzb".to_string()),
            source_kind: Some(DownloadSourceKind::NzbUrl),
            source_title: Some("Fixture.Replacement.2026.1080p.WEB-DL".to_string()),
            ..Default::default()
        },
        SubmissionScope::Title,
        SubmissionConflictPolicy::Abort,
    )
    .await
    .expect("a terminal row never blocks a replacement");
    assert_eq!(
        download_client.submitted_release_titles.lock().await.len(),
        1
    );
}

struct ListingTokenFixture {
    app: AppUseCase,
    operator: User,
    title: scryer_domain::Title,
    submissions: Arc<TrackingDownloadSubmissionRepo>,
}

async fn listing_token_fixture() -> ListingTokenFixture {
    let download_client = Arc::new(StubDownloadClient::default());
    let submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let (app, admin) = bootstrap_with_cleanup_tracking(
        download_client,
        submissions.clone(),
        Arc::new(TrackingPendingReleaseRepo::default()),
    );
    app.create_download_client_config(
        &admin,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
            proxy_config_id: None,
        },
    )
    .await
    .expect("create download client config");
    let title = app
        .add_title(
            &admin,
            NewTitle {
                name: "Listing Ticket".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .expect("create title");
    let (_created, operator) = create_authenticated_user(
        &app,
        &admin,
        "listing_ticket_user",
        "password123",
        vec![
            TestPermissionPreset::CatalogView,
            TestPermissionPreset::TitleManagement,
        ],
    )
    .await;
    ListingTokenFixture {
        app,
        operator,
        title,
        submissions,
    }
}

fn fixed_instant(raw: &str) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339(raw)
        .expect("fixed instant")
        .with_timezone(&chrono::Utc)
}

/// Offers one listing-rich result through the search's token step at
/// `offered_at`, returning the result as offered (token attached).
async fn offer_listing_with_token(
    fixture: &ListingTokenFixture,
    offered_at: chrono::DateTime<chrono::Utc>,
) -> IndexerSearchResult {
    fixture
        .app
        .runtime
        .environment
        .set_fixed_now_for_tests(Some(offered_at));
    let subject = fixture
        .app
        .resolve_release_search_subject_for_title(&fixture.title)
        .await
        .expect("resolve search subject");
    let mut results = vec![
        FixedReleaseIndexerClient::new("Listing.Ticket.2026.1080p.WEB-DL-GRP")
            .with_listing_facts()
            .release(),
    ];
    fixture
        .app
        .attach_candidate_tokens(
            &fixture.operator,
            &fixture.title,
            &subject,
            &mut results,
            false,
        )
        .await;
    let offered = results.remove(0);
    assert!(offered.candidate_token.is_some(), "{offered:?}");
    offered
}

async fn persisted_listing(fixture: &ListingTokenFixture) -> Option<String> {
    let submissions = fixture.submissions.store.lock().await.clone();
    assert_eq!(submissions.len(), 1, "{submissions:?}");
    submissions[0].release_listing_json.clone()
}

#[tokio::test]
async fn a_token_grab_persists_the_offered_listing_anchored_at_the_grab() {
    let offered_at = fixed_instant("2026-05-01T10:00:00Z");
    let grabbed_at = fixed_instant("2026-05-01T10:07:30Z");

    for replacement in [false, true] {
        let fixture = listing_token_fixture().await;
        let offered = offer_listing_with_token(&fixture, offered_at).await;
        let token = offered.candidate_token.clone().expect("token");
        fixture
            .app
            .runtime
            .environment
            .set_fixed_now_for_tests(Some(grabbed_at));

        let outcome = if replacement {
            fixture
                .app
                .queue_replacement_release_from_candidate_token(
                    &fixture.operator,
                    &fixture.title.id,
                    &token,
                    SubmissionConflictPolicy::Abort,
                    None,
                )
                .await
        } else {
            fixture
                .app
                .queue_existing_title_download_from_candidate_token(
                    &fixture.operator,
                    &fixture.title.id,
                    &token,
                    SubmissionScope::Title,
                    SubmissionConflictPolicy::Abort,
                )
                .await
        }
        .expect("token grab");
        assert!(matches!(outcome, QueueDownloadOutcome::Queued(_)));

        let expected =
            crate::quality::release_listing::ReleaseListingSnapshot::capture_from_search_result(
                &offered, grabbed_at,
            );
        assert_eq!(expected.thumbs_up, Some(7));
        assert_eq!(
            persisted_listing(&fixture).await,
            Some(expected.to_json_string()),
            "the offered facts persist, anchored at the grab (replacement: {replacement})"
        );
    }
}

#[tokio::test]
async fn a_token_grab_whose_listing_ticket_is_gone_persists_no_listing() {
    let fixture = listing_token_fixture().await;
    let offered = offer_listing_with_token(&fixture, fixed_instant("2026-05-01T10:00:00Z")).await;
    // What a restart or eviction leaves behind: a valid token, no ticket.
    fixture
        .app
        .runtime
        .acquisition
        .release_candidate_listings
        .lock()
        .expect("listing tickets")
        .clear();

    let outcome = fixture
        .app
        .queue_existing_title_download_from_candidate_token(
            &fixture.operator,
            &fixture.title.id,
            offered.candidate_token.as_deref().expect("token"),
            SubmissionScope::Title,
            SubmissionConflictPolicy::Abort,
        )
        .await
        .expect("a lost ticket never fails the grab");
    assert!(matches!(outcome, QueueDownloadOutcome::Queued(_)));
    assert_eq!(persisted_listing(&fixture).await, None);
}

#[tokio::test]
async fn a_token_minted_without_a_listing_ticket_persists_no_listing() {
    let fixture = listing_token_fixture().await;
    fixture
        .app
        .runtime
        .environment
        .set_fixed_now_for_tests(Some(fixed_instant("2026-05-01T10:00:00Z")));
    let token = fixture
        .app
        .issue_release_candidate_token(
            &fixture.operator,
            &fixture.title.id,
            &SubmissionScope::Title,
            &QueuedReleaseSelection {
                source_hint: Some("https://example.invalid/no-ticket.nzb".into()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Listing.Ticket.2026.720p.WEB-DL-GRP".into()),
                ..Default::default()
            },
        )
        .await
        .expect("issue token");

    fixture
        .app
        .queue_existing_title_download_from_candidate_token(
            &fixture.operator,
            &fixture.title.id,
            &token,
            SubmissionScope::Title,
            SubmissionConflictPolicy::Abort,
        )
        .await
        .expect("token grab");
    assert_eq!(persisted_listing(&fixture).await, None);
}

#[test]
fn listing_tickets_drop_expired_then_oldest_at_the_cap() {
    use crate::services::{ReleaseCandidateListingTicket, ReleaseCandidateListingTickets};
    let now = fixed_instant("2026-05-01T10:00:00Z");
    let ticket = |expires_at| ReleaseCandidateListingTicket {
        actor_id: "actor".into(),
        title_id: "title".into(),
        scope_kind: "title".into(),
        scope_id: None,
        source_hint: "https://example.invalid/ticket.nzb".into(),
        source_title: "Listing.Ticket.2026.1080p.WEB-DL-GRP".into(),
        listing:
            crate::quality::release_listing::ReleaseListingSnapshot::capture_from_search_result(
                &FixedReleaseIndexerClient::new("Listing.Ticket.2026.1080p.WEB-DL-GRP").release(),
                now,
            ),
        expires_at,
    };
    let live = now + chrono::Duration::minutes(30);
    let mut tickets = ReleaseCandidateListingTickets::default();

    tickets.insert("first".into(), ticket(live), now, 2);
    tickets.insert("second".into(), ticket(live), now, 2);
    tickets.insert("third".into(), ticket(live), now, 2);
    assert_eq!(tickets.len(), 2);
    assert!(!tickets.contains("first"), "the oldest ticket is evicted");
    assert!(tickets.contains("second") && tickets.contains("third"));

    let mut tickets = ReleaseCandidateListingTickets::default();
    tickets.insert(
        "expired".into(),
        ticket(now - chrono::Duration::seconds(1)),
        now,
        2,
    );
    tickets.insert("older-live".into(), ticket(live), now, 2);
    tickets.insert("newer-live".into(), ticket(live), now, 2);
    assert!(!tickets.contains("expired"), "expired tickets go first");
    assert!(tickets.contains("older-live") && tickets.contains("newer-live"));
    assert!(tickets.get("older-live", now).is_some());
    assert!(
        tickets.get("older-live", live).is_none(),
        "a ticket is unreadable once it expires"
    );

    // Far below the cap, an expired ticket is still purged by the next insert.
    let mut tickets = ReleaseCandidateListingTickets::default();
    let soon = now + chrono::Duration::minutes(1);
    tickets.insert("short-lived".into(), ticket(soon), now, 4096);
    tickets.insert("long-lived".into(), ticket(live), now, 4096);
    assert_eq!(tickets.len(), 2, "nothing has expired yet");
    let later = soon + chrono::Duration::seconds(1);
    tickets.insert(
        "fresh".into(),
        ticket(later + chrono::Duration::minutes(30)),
        later,
        4096,
    );
    assert!(
        !tickets.contains("short-lived"),
        "an expired ticket is gone after the next insert"
    );
    assert!(tickets.contains("long-lived") && tickets.contains("fresh"));
    assert_eq!(tickets.len(), 2);
    assert_eq!(
        tickets.order_len(),
        2,
        "its eviction-order slot went with it"
    );
}

#[tokio::test]
async fn a_token_whose_listing_ticket_fails_its_binding_persists_no_listing() {
    let fixture = listing_token_fixture().await;
    let offered = offer_listing_with_token(&fixture, fixed_instant("2026-05-01T10:00:00Z")).await;
    let offered_token = offered.candidate_token.as_deref().expect("token");
    // A validly signed token that presents the offered ticket's reference
    // for a different source: the ticket exists but is bound elsewhere.
    let mut claims = jsonwebtoken::dangerous::insecure_decode::<
        crate::types::ReleaseCandidateTokenClaims,
    >(offered_token)
    .expect("decode offered token")
    .claims;
    assert!(claims.listing_ref.is_some(), "the offer holds a ticket");
    claims.source_hint = "https://example.invalid/another-source.nzb".to_string();
    let signing_key = fixture
        .app
        .release_candidate_signing_key_for_actor(&fixture.operator)
        .await
        .expect("signing key");
    let rebound_token = jsonwebtoken::encode(
        &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(&signing_key),
    )
    .expect("sign rebound token");

    let outcome = fixture
        .app
        .queue_existing_title_download_from_candidate_token(
            &fixture.operator,
            &fixture.title.id,
            &rebound_token,
            SubmissionScope::Title,
            SubmissionConflictPolicy::Abort,
        )
        .await
        .expect("a mismatched ticket never fails the grab");
    assert!(matches!(outcome, QueueDownloadOutcome::Queued(_)));
    assert_eq!(persisted_listing(&fixture).await, None);
}

/// A TVDB season that two AniDB entries split between them: the first cour is
/// entry 7001, which the title carries, and the second is entry 7002.
fn split_season_bridge() -> scryer_domain::AnimeNumberingBridge {
    let cour = |index: i32, anidb_id: i64, name: &str, tvdb_start: i32| {
        scryer_domain::AnimeCommunitySeason {
            index,
            anidb_id: Some(anidb_id),
            titles: vec![name.into()],
            ranges: vec![scryer_domain::AnimeCommunitySeasonRange {
                community_episode_start: 1,
                community_episode_end: Some(12),
                tvdb_season: 1,
                tvdb_episode_start: tvdb_start,
                tvdb_episode_end: Some(tvdb_start + 11),
            }],
            episode_count: Some(12),
            ..Default::default()
        }
    };
    scryer_domain::AnimeNumberingBridge {
        source: Default::default(),
        generated_on: "2026-01-01".into(),
        corroborating_order: None,
        seasons: vec![
            cour(1, 7001, "Harbor Lantern Saga", 1),
            cour(2, 7002, "Harbor Lantern Saga Second Tide", 13),
        ],
    }
}

struct AnidbSelectionFixture {
    app: AppUseCase,
    shows: std::sync::Arc<super::support_library_show::MockShowRepo>,
    title: Title,
}

impl AnidbSelectionFixture {
    async fn new(facet: MediaFacet, bridge: Option<scryer_domain::AnimeNumberingBridge>) -> Self {
        let shows = std::sync::Arc::new(super::support_library_show::MockShowRepo::default());
        let (app, user) = bootstrap();
        let app = app.with_test_overrides({
            let shows = shows.clone();
            move |services| services.with_shows(shows)
        });
        let title = app
            .add_title(
                &user,
                NewTitle {
                    name: "Harbor Lantern Saga".into(),
                    facet,
                    monitored: true,
                    external_ids: vec![scryer_domain::ExternalId::new("anidb", "7001")],
                    ..Default::default()
                },
            )
            .await
            .expect("create title");
        if let Some(bridge) = bridge {
            app.services
                .catalog
                .shows
                .replace_anime_numbering_bridge(&title.id, Some(&bridge))
                .await
                .expect("store the numbering bridge");
        }
        Self { app, shows, title }
    }

    async fn episode(&self, episode_number: u32) -> Episode {
        let episode = Episode {
            id: Id::new().0,
            title_id: self.title.id.clone(),
            collection_id: Some("season-1".to_string()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some(episode_number.to_string()),
            season_number: Some("1".to_string()),
            episode_label: None,
            title: None,
            air_date: None,
            duration_seconds: Some(1_440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            contiguous_absolute_number: None,
            overview: None,
            tvdb_id: None,
            tmdb_id: None,
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        };
        self.shows.episodes.lock().await.push(episode.clone());
        episode
    }

    fn wanted(&self, episode: &Episode) -> AcquisitionScopeState {
        let now = Utc::now().to_rfc3339();
        AcquisitionScopeState {
            id: Id::new().0,
            title_id: self.title.id.clone(),
            title_name: Some(self.title.name.clone()),
            title_slug: self.title.slug.clone(),
            title_facet: Some(self.title.facet.as_str().to_string()),
            library_id: Some(self.title.library_id.clone()),
            library_name: None,
            library_slug: None,
            episode_id: Some(episode.id.clone()),
            collection_id: episode.collection_id.clone(),
            series_movie_link_id: None,
            season_number: episode.season_number.clone(),
            episode_number: episode.episode_number.clone(),
            media_type: "episode".to_string(),
            last_search_at: None,
            status: AcquisitionScopeStatus::Wanted,
            grabbed_release: None,
            landed_bar: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: now.clone(),
            updated_at: now,
        }
    }

    /// The subject a title walk resolves for one episode, with or without the
    /// walk's catalog memo.
    async fn walk_subject(
        &self,
        episode: &Episode,
        reads: Option<&crate::acquisition::title_reads::TitleCatalogReads>,
    ) -> crate::acquisition_release_search::ResolvedReleaseSearchSubject {
        let wanted = self.wanted(episode);
        let search_title = self
            .app
            .release_search_title_for_wanted_item(&self.title, &wanted, Some(episode), reads)
            .await;
        self.app
            .resolve_pending_release_search_subject_for_wanted_item(
                &self.title,
                &search_title,
                &wanted,
                Some(episode),
                reads,
            )
            .await
            .expect("subject resolves")
            .for_convergence()
            .clone()
    }

    /// The AniDB id the automatic and the interactive lane each send for one
    /// episode.
    async fn anidb_ids(&self, episode: &Episode) -> (Option<String>, Option<String>) {
        let wanted = self.wanted(episode);
        let search_title = self
            .app
            .release_search_title_for_wanted_item(&self.title, &wanted, Some(episode), None)
            .await;
        let automatic = self
            .app
            .resolve_release_search_subject_for_wanted_item(
                &self.title,
                &search_title,
                &wanted,
                Some(episode),
            )
            .await
            .expect("automatic subject")
            .anidb_id;
        let interactive = self
            .app
            .resolve_release_search_subject_for_episode(
                &self.title,
                episode.season_number.as_deref().unwrap(),
                episode.episode_number.as_deref().unwrap(),
            )
            .await
            .expect("interactive subject")
            .anidb_id;
        (automatic, interactive)
    }
}

/// Both the walk's subject and the interactive subject for an episode are
/// pre-release until a day after it airs, and released after that.
#[tokio::test]
async fn episode_subjects_are_pre_release_until_a_day_after_air() {
    let fixture = AnidbSelectionFixture::new(MediaFacet::Series, None).await;
    let now = Utc::now();
    let cases = [
        (
            (now + chrono::Duration::days(1)).date_naive().to_string(),
            true,
            "airs tomorrow",
        ),
        (
            (now - chrono::Duration::hours(12)).to_rfc3339(),
            true,
            "aired twelve hours ago",
        ),
        (
            (now - chrono::Duration::days(2)).date_naive().to_string(),
            false,
            "aired two days ago",
        ),
    ];
    for (number, (air_date, expected, case)) in (1_u32..).zip(cases) {
        let mut episode = fixture.episode(number).await;
        episode.air_date = Some(air_date);
        for stored in fixture.shows.episodes.lock().await.iter_mut() {
            if stored.id == episode.id {
                stored.air_date = episode.air_date.clone();
            }
        }

        let walk = fixture.walk_subject(&episode, None).await;
        assert_eq!(walk.pre_release, expected, "walk subject: {case}");
        let interactive = fixture
            .app
            .resolve_release_search_subject_for_episode(&fixture.title, "1", &number.to_string())
            .await
            .expect("interactive subject");
        assert_eq!(
            interactive.pre_release, expected,
            "interactive subject: {case}"
        );
    }
}

fn scoped_anidb_id(scope_id: &str, anidb_id: &str, source_scope: Option<&str>) -> ScopedExternalId {
    ScopedExternalId {
        scope_id: scope_id.to_string(),
        source: "anidb".to_string(),
        external_id: anidb_id.to_string(),
        provenance: "anibridge".to_string(),
        source_scope: source_scope.map(str::to_string),
    }
}

/// A title walk resolves every anime episode subject from what it already
/// holds: the title's episode ids are read once for the whole walk, and the
/// absolute scale comes from the walk's episode list rather than a catalog
/// query per stage. Each subject is the one the unshared path resolves.
#[tokio::test]
async fn a_title_walk_resolves_anime_subjects_from_its_own_catalog_reads() {
    use std::sync::atomic::Ordering;

    for contiguous_title in [false, true] {
        let fixture = AnidbSelectionFixture::new(MediaFacet::Anime, None).await;
        let mut episodes = Vec::new();
        for number in 1..=3_u32 {
            let mut episode = fixture.episode(number).await;
            episode.absolute_number = Some(number.to_string());
            // On the contiguous title only the last episode carries a
            // contiguous number, so the first two need the title's scale.
            if contiguous_title && number == 3 {
                episode.contiguous_absolute_number = Some(3);
            }
            episodes.push(episode);
        }
        *fixture.shows.episodes.lock().await = episodes.clone();
        *fixture.shows.episode_external_ids.lock().await =
            vec![scoped_anidb_id(&episodes[0].id, "7101", None)];
        *fixture.shows.collection_external_ids.lock().await =
            vec![scoped_anidb_id("season-1", "7100", None)];

        let mut unshared = Vec::new();
        for episode in &episodes {
            unshared.push(fixture.walk_subject(episode, None).await);
        }

        let counts = || {
            [
                fixture
                    .shows
                    .episode_external_id_reads
                    .load(Ordering::SeqCst),
                fixture
                    .shows
                    .title_episode_external_id_reads
                    .load(Ordering::SeqCst),
                fixture.shows.absolute_scale_reads.load(Ordering::SeqCst),
            ]
        };
        let before = counts();
        let reads = crate::acquisition::title_reads::TitleCatalogReads::with_episodes(
            &fixture.title.id,
            episodes.clone(),
        );
        let mut shared = Vec::new();
        for episode in &episodes {
            shared.push(fixture.walk_subject(episode, Some(&reads)).await);
        }
        let after = counts();

        assert_eq!(
            format!("{shared:?}"),
            format!("{unshared:?}"),
            "the walk resolves exactly what the unshared path does (contiguous title: {contiguous_title})"
        );
        assert_eq!(after[0] - before[0], 0, "no per-episode external id read");
        assert_eq!(
            after[1] - before[1],
            1,
            "the title's episode ids are read once per walk"
        );
        assert_eq!(
            after[2] - before[2],
            0,
            "the absolute scale needs no catalog query"
        );

        assert_eq!(shared[0].anidb_id.as_deref(), Some("7101"));
        assert_eq!(shared[1].anidb_id.as_deref(), Some("7100"));
        let expected_absolute = if contiguous_title {
            [None, None, Some(3)]
        } else {
            [Some(1), Some(2), Some(3)]
        };
        assert_eq!(
            shared
                .iter()
                .map(|subject| subject.absolute_episode)
                .collect::<Vec<_>>(),
            expected_absolute,
            "the scale is the title's (contiguous title: {contiguous_title})"
        );
    }
}

#[tokio::test]
async fn episode_search_sends_the_anidb_id_of_the_cour_the_episode_sits_in() {
    let fixture = AnidbSelectionFixture::new(MediaFacet::Anime, Some(split_season_bridge())).await;
    let first_cour = fixture.episode(5).await;
    let second_cour = fixture.episode(20).await;

    let expected_first = Some("7001".to_string());
    assert_eq!(
        fixture.anidb_ids(&first_cour).await,
        (expected_first.clone(), expected_first)
    );
    let expected_second = Some("7002".to_string());
    assert_eq!(
        fixture.anidb_ids(&second_cour).await,
        (expected_second.clone(), expected_second)
    );
}

#[tokio::test]
async fn episode_scoped_anidb_id_wins_over_the_bridge_and_prefers_the_r_scope() {
    let fixture = AnidbSelectionFixture::new(MediaFacet::Anime, Some(split_season_bridge())).await;
    let episode = fixture.episode(20).await;
    *fixture.shows.episode_external_ids.lock().await = vec![
        scoped_anidb_id(&episode.id, "7100", None),
        scoped_anidb_id(&episode.id, "7200", Some("R")),
    ];
    *fixture.shows.collection_external_ids.lock().await =
        vec![scoped_anidb_id("season-1", "7300", Some("R"))];

    let expected = Some("7200".to_string());
    assert_eq!(
        fixture.anidb_ids(&episode).await,
        (expected.clone(), expected)
    );
}

#[tokio::test]
async fn season_scoped_anidb_id_still_applies_without_a_cour_or_episode_id() {
    let fixture = AnidbSelectionFixture::new(MediaFacet::Anime, None).await;
    let episode = fixture.episode(20).await;
    *fixture.shows.collection_external_ids.lock().await =
        vec![scoped_anidb_id("season-1", "7300", Some("R"))];

    let expected = Some("7300".to_string());
    assert_eq!(
        fixture.anidb_ids(&episode).await,
        (expected.clone(), expected)
    );
}

#[tokio::test]
async fn episode_search_without_scoped_ids_or_bridge_keeps_the_title_anidb_id() {
    let fixture = AnidbSelectionFixture::new(MediaFacet::Anime, None).await;
    let episode = fixture.episode(20).await;

    let expected = Some("7001".to_string());
    assert_eq!(
        fixture.anidb_ids(&episode).await,
        (expected.clone(), expected)
    );
}

#[tokio::test]
async fn non_anime_episode_search_ignores_episode_scoped_and_cour_anidb_ids() {
    let fixture = AnidbSelectionFixture::new(MediaFacet::Series, Some(split_season_bridge())).await;
    let episode = fixture.episode(20).await;
    *fixture.shows.episode_external_ids.lock().await =
        vec![scoped_anidb_id(&episode.id, "7200", Some("R"))];

    let expected = Some("7001".to_string());
    assert_eq!(
        fixture.anidb_ids(&episode).await,
        (expected.clone(), expected)
    );
}

#[test]
fn scan_blocked_facet_streaks_count_consecutive_cycles_and_reset_when_free() {
    let mut streaks = crate::acquisition::workflow::ScanBlockedFacetStreaks::default();

    assert_eq!(
        streaks.observe(&[MediaFacet::Series]),
        vec![(MediaFacet::Series, 1)]
    );
    assert_eq!(
        streaks.observe(&[MediaFacet::Series, MediaFacet::Movie, MediaFacet::Series]),
        vec![(MediaFacet::Series, 2), (MediaFacet::Movie, 1)],
        "a facet listed twice in one cycle still counts once"
    );
    assert_eq!(
        streaks.observe(&[MediaFacet::Series]),
        vec![(MediaFacet::Series, 3)],
        "a facet that was free this cycle drops out"
    );
    assert_eq!(
        streaks.observe(&[MediaFacet::Movie, MediaFacet::Series]),
        vec![(MediaFacet::Movie, 1), (MediaFacet::Series, 4)],
        "a freed facet starts counting again from one"
    );
    assert!(streaks.observe(&[]).is_empty());
    assert_eq!(
        streaks.observe(&[MediaFacet::Series]),
        vec![(MediaFacet::Series, 1)],
        "a cycle with no active scan resets every streak"
    );
}

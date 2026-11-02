use super::*;

struct Fixture {
    app: AppUseCase,
    actor: User,
    title: Title,
    candidate: PendingRelease,
    client: Arc<StubDownloadClient>,
    pending: Arc<TrackingPendingReleaseRepo>,
}

impl Fixture {
    async fn new(remove_on_delete: bool) -> Self {
        let client = Arc::new(StubDownloadClient {
            remove_queue_item_on_delete: remove_on_delete,
            unique_job_ids: true,
            ..Default::default()
        });
        let pending = Arc::new(TrackingPendingReleaseRepo::default());
        let wanted = Arc::new(TrackingAcquisitionScopeStateRepo::default());
        let (app, actor) = bootstrap_with_acquisition_tracking(
            client.clone(),
            Arc::new(TrackingDownloadSubmissionRepo::default()),
            pending.clone(),
            wanted.clone(),
        );
        let (title, wanted_id) =
            seed_movie_wanted_for_acquisition(&app, &actor, &wanted, "Standby Queue Fixture", 2024)
                .await;
        let candidate = pending_movie_release(
            &wanted_id,
            &title,
            "Standby.Queue.Fixture.2024.1080p.WEB-DL-GRP",
            PendingReleaseStatus::Standby,
        );
        pending.insert_pending_release(&candidate).await.unwrap();
        pending.reset_status_writes();
        Self {
            app,
            actor,
            title,
            candidate,
            client,
            pending,
        }
    }

    async fn conflict(&self, id: &str, title_id: &str) {
        self.app
            .services
            .workflow
            .download_submissions
            .record_submission(DownloadSubmission {
                download_id: scryer_domain::download_identity::DownloadId::new(),
                title_id: title_id.to_string(),
                purpose: DownloadSubmissionPurpose::Standard,
                facet: "movie".into(),
                download_client_id: Some("primary".into()),
                download_client_type: "nzbget".into(),
                download_client_item_id: id.into(),
                source_hint: None,
                source_provider_id: None,
                source_provider_name: None,
                source_kind: None,
                source_title: Some(format!("Older {id}")),
                info_hash: None,
                release_size_bytes: None,
                request_signature: None,
                scope: SubmissionScope::Title,
                release_listing_json: None,
            })
            .await
            .unwrap();
        let mut item = queue_history_fixture_item(id, DownloadQueueState::Downloading, 0);
        item.title_id = Some(title_id.into());
        self.client.queue_items.lock().await.push(item);
    }

    async fn queue(
        &self,
        choice: StandbyQueueChoice,
        conflicts: &[SubmissionScopeConflict],
    ) -> AppResult<StandbyQueueOutcome> {
        let identities = conflicts
            .iter()
            .map(|item| {
                ClientJobLocator::new(
                    item.download_client_id.as_deref(),
                    &item.download_client_type,
                    &item.download_client_item_id,
                )
            })
            .collect::<Vec<_>>();
        self.app
            .queue_standby_release(&self.actor, &self.candidate.id, choice, &identities)
            .await
    }

    async fn status(&self) -> PendingReleaseStatus {
        self.pending
            .get_pending_release(&self.candidate.id)
            .await
            .unwrap()
            .unwrap()
            .status
    }
}

#[tokio::test]
async fn standby_queue_direct_and_repeated_request_submit_once() {
    let f = Fixture::new(true).await;
    assert!(
        f.queue(StandbyQueueChoice::Check, &[])
            .await
            .unwrap()
            .grabbed
    );
    assert_eq!(f.status().await, PendingReleaseStatus::Grabbed);
    assert!(
        f.queue(StandbyQueueChoice::Check, &[])
            .await
            .unwrap()
            .grabbed
    );
    assert_eq!(f.client.submitted_release_titles.lock().await.len(), 1);
    assert_eq!(
        f.pending.status_writes(),
        vec![(f.candidate.id.clone(), PendingReleaseStatus::Grabbed)]
    );
    let submissions = f
        .app
        .services
        .workflow
        .download_submissions
        .list_for_title(&f.title.id)
        .await
        .unwrap();
    assert_eq!(
        submissions[0].purpose,
        DownloadSubmissionPurpose::OperatorQueued
    );
    let events = release_grabbed_actors(&f.app, &f.title.id).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].1.as_deref(), Some(f.actor.id.as_str()));
}

#[tokio::test]
async fn standby_queue_confirms_multiple_and_queues_alongside() {
    let f = Fixture::new(true).await;
    f.conflict("first", &f.title.id).await;
    f.conflict("second", &f.title.id).await;
    let result = f.queue(StandbyQueueChoice::Check, &[]).await.unwrap();
    assert_eq!(result.conflicts.len(), 2);
    assert!(!result.grabbed);
    assert!(f.pending.status_writes().is_empty());
    assert!(f.client.submitted_release_titles.lock().await.is_empty());
    assert!(
        f.queue(StandbyQueueChoice::Queue, &result.conflicts)
            .await
            .unwrap()
            .grabbed
    );
    assert!(f.client.delete_attempts.lock().await.is_empty());
    let queue = f.client.queue_items.lock().await;
    assert_eq!(queue.len(), 3);
    for id in ["first", "second"] {
        assert!(queue.iter().any(|item| item.download_client_item_id == id));
    }
}

#[tokio::test]
async fn standby_queue_changed_conflicts_require_confirmation_for_both_choices() {
    for choice in [
        StandbyQueueChoice::Queue,
        StandbyQueueChoice::CancelAndQueue,
    ] {
        let f = Fixture::new(true).await;
        f.conflict("first", &f.title.id).await;
        let original = f.queue(StandbyQueueChoice::Check, &[]).await.unwrap();
        f.conflict("new", &f.title.id).await;
        let changed = f.queue(choice, &original.conflicts).await.unwrap();
        assert!(!changed.grabbed);
        assert_eq!(changed.conflicts.len(), 2);
        assert!(f.client.delete_attempts.lock().await.is_empty());
        assert!(f.client.submitted_release_titles.lock().await.is_empty());
        assert_eq!(f.status().await, PendingReleaseStatus::Standby);
    }
}

#[tokio::test]
async fn standby_queue_cancel_preserves_unrelated_downloads_and_files() {
    let f = Fixture::new(true).await;
    let files = tempfile::tempdir().unwrap();
    let unrelated = files.path().join("unrelated.mkv");
    std::fs::write(&unrelated, b"preserve").unwrap();
    f.conflict("first", &f.title.id).await;
    f.conflict("second", &f.title.id).await;
    f.conflict("unrelated", "other-title").await;
    let result = f.queue(StandbyQueueChoice::Check, &[]).await.unwrap();
    assert_eq!(result.conflicts.len(), 2);
    assert!(
        f.queue(StandbyQueueChoice::CancelAndQueue, &result.conflicts)
            .await
            .unwrap()
            .grabbed
    );
    let remaining = f.client.queue_items.lock().await;
    assert_eq!(remaining.len(), 2);
    assert!(
        remaining
            .iter()
            .any(|item| item.download_client_item_id == "unrelated")
    );
    assert!(
        remaining
            .iter()
            .all(|item| item.download_client_item_id != "first"
                && item.download_client_item_id != "second")
    );
    let requests = f.client.deleted_requests.lock().await;
    assert_eq!(requests.len(), 2);
    assert!(
        requests
            .iter()
            .all(|(_, _, id, history, remove_data)| id != "unrelated" && !history && !remove_data)
    );
    assert_eq!(std::fs::read(&unrelated).unwrap(), b"preserve");
}

#[tokio::test]
async fn standby_queue_failed_or_unresolved_cancel_never_submits() {
    for fail in [false, true] {
        let f = Fixture::new(false).await;
        f.conflict("first", &f.title.id).await;
        let result = f.queue(StandbyQueueChoice::Check, &[]).await.unwrap();
        if fail {
            f.client.set_delete_error(Some("synthetic refusal")).await;
        }
        assert!(
            f.queue(StandbyQueueChoice::CancelAndQueue, &result.conflicts)
                .await
                .is_err()
        );
        assert!(f.client.submitted_release_titles.lock().await.is_empty());
        assert_eq!(f.status().await, PendingReleaseStatus::Standby);
        assert_eq!(
            f.app
                .services
                .workflow
                .download_submissions
                .list_for_title(&f.title.id)
                .await
                .unwrap()
                .len(),
            1
        );
    }
}

#[tokio::test]
async fn standby_queue_submission_failure_is_retryable() {
    let f = Fixture::new(true).await;
    f.client
        .set_submit_error(Some(StubSubmitError::Rejected("synthetic refusal".into())))
        .await;
    assert!(f.queue(StandbyQueueChoice::Check, &[]).await.is_err());
    assert_eq!(f.status().await, PendingReleaseStatus::Standby);
    f.client.set_submit_error(None).await;
    assert!(
        f.queue(StandbyQueueChoice::Check, &[])
            .await
            .unwrap()
            .grabbed
    );
}

#[tokio::test]
async fn standby_queue_requires_library_manage_permission() {
    let f = Fixture::new(true).await;
    let viewer = test_user_with_app_permissions("viewer", AppPermissionMask::default());
    assert!(
        f.app
            .queue_standby_release(&viewer, &f.candidate.id, StandbyQueueChoice::Check, &[])
            .await
            .is_err()
    );
    assert!(f.client.submitted_release_titles.lock().await.is_empty());
    assert!(f.pending.status_writes().is_empty());
}

#[tokio::test]
async fn standby_queue_deduplicates_the_same_release_saved_twice() {
    let f = Fixture::new(true).await;
    assert!(
        f.queue(StandbyQueueChoice::Check, &[])
            .await
            .unwrap()
            .grabbed
    );
    let mut duplicate = f.candidate.clone();
    duplicate.id = Id::new().0;
    f.pending.insert_pending_release(&duplicate).await.unwrap();
    let result = f
        .app
        .queue_standby_release(&f.actor, &duplicate.id, StandbyQueueChoice::Check, &[])
        .await
        .unwrap();
    if !result.grabbed {
        let ids = result
            .conflicts
            .iter()
            .map(|item| {
                ClientJobLocator::new(
                    item.download_client_id.as_deref(),
                    &item.download_client_type,
                    &item.download_client_item_id,
                )
            })
            .collect::<Vec<_>>();
        assert!(
            f.app
                .queue_standby_release(&f.actor, &duplicate.id, StandbyQueueChoice::Queue, &ids)
                .await
                .unwrap()
                .grabbed
        );
    }
    assert_eq!(f.client.submitted_release_titles.lock().await.len(), 1);
}

#[tokio::test]
async fn standby_queue_retries_explicit_failure_despite_recent_acceptance() {
    let f = Fixture::new(true).await;
    assert!(
        f.queue(StandbyQueueChoice::Check, &[])
            .await
            .unwrap()
            .grabbed
    );
    let mut duplicate = f.candidate.clone();
    duplicate.id = Id::new().0;
    f.pending.insert_pending_release(&duplicate).await.unwrap();
    // Retain the accepted-id cache and seed an older successful snapshot.
    let guards = &f.app.runtime.acquisition.download_submission_guards;
    let snapshot = f
        .client
        .list_snapshot_outcome_excluding_client_types(100, &[])
        .await
        .unwrap();
    guards.store_client_snapshot(guards.client_snapshot_generation(), snapshot);
    for item in f.client.queue_items.lock().await.iter_mut() {
        item.state = DownloadQueueState::Failed;
    }
    let result = f
        .app
        .queue_standby_release(&f.actor, &duplicate.id, StandbyQueueChoice::Check, &[])
        .await
        .unwrap();
    assert!(result.grabbed);
    assert!(result.conflicts.is_empty());
    assert_eq!(f.client.submitted_release_titles.lock().await.len(), 2);
}

#[tokio::test]
async fn standby_queue_interrupted_request_keeps_candidate_retryable() {
    let f = Fixture::new(true).await;
    *f.client.submit_gate.lock().await = Some(Arc::new(tokio::sync::Notify::new()));
    let mut request = Box::pin(f.queue(StandbyQueueChoice::Check, &[]));
    crate::test_wait::within_deadline("operator reaches gated client submission", async {
        tokio::select! {
            result = &mut request => panic!("submission finished before client gate: {result:?}"),
            _ = f.client.submit_started.notified() => {},
        }
    })
    .await;
    assert_eq!(f.status().await, PendingReleaseStatus::Standby);
    drop(request);
    assert_eq!(f.status().await, PendingReleaseStatus::Standby);
    *f.client.submit_gate.lock().await = None;
    let result = crate::test_wait::within_deadline(
        "retry interrupted standby queue",
        f.queue(StandbyQueueChoice::Check, &[]),
    )
    .await
    .unwrap();
    assert!(result.grabbed);
    assert_eq!(f.client.submitted_release_titles.lock().await.len(), 1);
}

#[tokio::test]
async fn standby_queue_concurrent_clicks_submit_once_without_a_durable_claim() {
    let f = Fixture::new(true).await;
    let (first, second) =
        crate::test_wait::within_deadline("concurrent standby queue requests", async {
            tokio::join!(
                f.queue(StandbyQueueChoice::Check, &[]),
                f.queue(StandbyQueueChoice::Check, &[])
            )
        })
        .await;
    assert!(first.unwrap().grabbed);
    assert!(second.unwrap().grabbed);
    assert_eq!(f.client.submitted_release_titles.lock().await.len(), 1);
    assert_eq!(f.status().await, PendingReleaseStatus::Grabbed);
}

#[tokio::test]
async fn standby_queue_preserves_episode_and_pack_coverage() {
    for (release, pack) in [
        ("Recent.Failed.Season.Pack.S07.1080p.WEB-DL-GroupB", true),
        (
            "Recent.Failed.Season.Pack.S07E23.1080p.WEB-DL-GroupB",
            false,
        ),
    ] {
        let original = "Recent.Failed.Season.Pack.S07.1080p.WEB-DL-GroupA";
        let (app, title, _, client) = seed_recent_failed_season_pack_fixture_with_client(
            Arc::new(
                TrackingIndexerClient::default().with_season_pack_titles(vec![original.into()]),
            ),
            Arc::new(TrackingAcquisitionScopeStateRepo::default()),
            Arc::new(TrackingPendingReleaseRepo::default()),
            Arc::new(StubDownloadClient::default().with_unique_job_ids()),
        )
        .await;
        app.run_background_acquisition_cycle_once().await;
        set_queued_pack_state(
            &app,
            &title,
            &client,
            original,
            DownloadQueueState::Downloading,
            TrackedDownloadState::Downloading,
        )
        .await;
        let id = save_season_standby(&app, &title, release, 0).await;
        let actor = test_admin_user();
        let conflicts = app
            .queue_standby_release(&actor, &id, StandbyQueueChoice::Check, &[])
            .await
            .unwrap();
        assert_eq!(conflicts.conflicts.len(), 1);
        let identities = conflicts
            .conflicts
            .iter()
            .map(|item| {
                ClientJobLocator::new(
                    item.download_client_id.as_deref(),
                    &item.download_client_type,
                    &item.download_client_item_id,
                )
            })
            .collect::<Vec<_>>();
        assert!(
            app.queue_standby_release(&actor, &id, StandbyQueueChoice::Queue, &identities)
                .await
                .unwrap()
                .grabbed
        );
        let submissions = app
            .services
            .workflow
            .download_submissions
            .list_for_title(&title.id)
            .await
            .unwrap();
        let submitted = submissions
            .iter()
            .find(|item| item.source_title.as_deref() == Some(release))
            .unwrap();
        if pack {
            assert!(!matches!(
                submitted.scope,
                SubmissionScope::Episode { .. } | SubmissionScope::Title
            ));
        } else {
            assert!(matches!(submitted.scope, SubmissionScope::Episode { .. }));
        }
        assert_eq!(
            pending_status(&app, &id).await,
            PendingReleaseStatus::Grabbed
        );
        assert_eq!(client.submitted_release_titles.lock().await.len(), 2);
    }
}

#[tokio::test]
async fn standby_queue_deduplicates_an_automatic_grab() {
    let release = "Recent.Failed.Season.Pack.S07.1080p.WEB-DL-GroupA";
    let (app, title, client) = season_pack_downloading_fixture(vec![release.into()]).await;
    let submissions = app
        .services
        .workflow
        .download_submissions
        .list_for_title(&title.id)
        .await
        .unwrap();
    let anchor = season_scope_states(&app, &title, "7").await.remove(0);
    let mut saved =
        pending_movie_release(&anchor.id, &title, release, PendingReleaseStatus::Standby);
    saved.release_url = submissions[0].source_hint.clone();
    saved.source_kind = submissions[0].source_kind;
    saved.release_size_bytes = None;
    app.services
        .workflow
        .pending_releases
        .insert_pending_release(&saved)
        .await
        .unwrap();
    let id = saved.id;
    let actor = test_admin_user();
    let result = app
        .queue_standby_release(&actor, &id, StandbyQueueChoice::Check, &[])
        .await
        .unwrap();
    assert_eq!(result.conflicts.len(), 1);
    let identities = result
        .conflicts
        .iter()
        .map(|item| {
            ClientJobLocator::new(
                item.download_client_id.as_deref(),
                &item.download_client_type,
                &item.download_client_item_id,
            )
        })
        .collect::<Vec<_>>();
    assert!(
        app.queue_standby_release(&actor, &id, StandbyQueueChoice::Queue, &identities)
            .await
            .unwrap()
            .grabbed
    );
    assert_eq!(client.submitted_release_titles.lock().await.len(), 1);
}

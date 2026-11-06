use super::*;
use crate::error_reasons::standby as reason;

#[track_caller]
fn assert_refused<T: std::fmt::Debug>(result: AppResult<T>, expected: &str) {
    let error = result.expect_err("expected a refusal");
    assert_eq!(error.reason(), Some(expected), "{error:?}");
}

fn identities(conflicts: &[SubmissionScopeConflict]) -> Vec<ClientJobLocator> {
    conflicts
        .iter()
        .map(|item| {
            ClientJobLocator::new(
                item.download_client_id.as_deref(),
                &item.download_client_type,
                &item.download_client_item_id,
            )
        })
        .collect()
}

fn item_ids(conflicts: &[SubmissionScopeConflict]) -> Vec<&str> {
    conflicts
        .iter()
        .map(|item| item.download_client_item_id.as_str())
        .collect()
}

async fn row_ids(f: &Fixture) -> Vec<String> {
    f.app
        .services
        .workflow
        .download_submissions
        .list_for_title(&f.title.id)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.download_client_item_id)
        .collect()
}

struct Fixture {
    app: AppUseCase,
    actor: User,
    title: Title,
    candidate: PendingRelease,
    client: Arc<StubDownloadClient>,
    pending: Arc<TrackingPendingReleaseRepo>,
    wanted_id: String,
}

impl Fixture {
    async fn new(remove_on_delete: bool) -> Self {
        Self::with_client(StubDownloadClient {
            remove_queue_item_on_delete: remove_on_delete,
            unique_job_ids: true,
            ..Default::default()
        })
        .await
    }

    async fn with_client(client: StubDownloadClient) -> Self {
        let client = Arc::new(client);
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
            wanted_id,
        }
    }

    async fn conflict(&self, id: &str, title_id: &str) {
        self.conflict_in_state(id, title_id, DownloadQueueState::Downloading)
            .await;
    }

    async fn conflict_in_state(&self, id: &str, title_id: &str, state: DownloadQueueState) {
        self.record_row(id, title_id, "primary").await;
        let mut item = queue_history_fixture_item(id, state, 0);
        item.title_id = Some(title_id.into());
        self.client.queue_items.lock().await.push(item);
    }

    async fn record_row(&self, id: &str, title_id: &str, client_id: &str) {
        self.app
            .services
            .workflow
            .download_submissions
            .record_submission(DownloadSubmission {
                download_id: scryer_domain::download_identity::DownloadId::new(),
                title_id: title_id.to_string(),
                purpose: DownloadSubmissionPurpose::Standard,
                facet: "movie".into(),
                download_client_id: Some(client_id.into()),
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
    }

    async fn queue(
        &self,
        choice: StandbyQueueChoice,
        conflicts: &[SubmissionScopeConflict],
    ) -> AppResult<StandbyQueueOutcome> {
        self.app
            .queue_standby_release(
                &self.actor,
                &self.candidate.id,
                choice,
                &identities(conflicts),
            )
            .await
    }

    async fn wanted_status(&self) -> AcquisitionScopeStatus {
        self.app
            .services
            .workflow
            .acquisition_scope_states
            .get_acquisition_scope_state_by_id(&self.wanted_id)
            .await
            .unwrap()
            .unwrap()
            .status
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
async fn standby_queue_cancel_is_best_effort_and_always_queues() {
    for fail in [false, true] {
        // The client keeps the job listed after a cancel, the way most
        // clients do while post-processing, or refuses the cancel outright.
        let f = Fixture::new(false).await;
        f.conflict("first", &f.title.id).await;
        let result = f.queue(StandbyQueueChoice::Check, &[]).await.unwrap();
        if fail {
            f.client.set_delete_error(Some("synthetic refusal")).await;
        }
        let outcome = f
            .queue(StandbyQueueChoice::CancelAndQueue, &result.conflicts)
            .await
            .unwrap();
        assert!(outcome.grabbed);
        assert!(outcome.conflicts.is_empty());
        assert_eq!(f.client.delete_attempts.lock().await.len(), 1);
        assert_eq!(f.client.submitted_release_titles.lock().await.len(), 1);
        assert_eq!(f.status().await, PendingReleaseStatus::Grabbed);
        let (cancelled, failed) = if fail {
            (vec![], vec!["first"])
        } else {
            (vec!["first"], vec![])
        };
        assert_eq!(item_ids(&outcome.cancelled), cancelled);
        assert_eq!(item_ids(&outcome.cancel_failed), failed);
        // A cancel the client took is settled at once without waiting to see
        // the job go; one it refused keeps its row and stays tracked.
        let rows = row_ids(&f).await;
        assert!(rows.iter().any(|id| id != "first"));
        assert_eq!(rows.iter().any(|id| id == "first"), fail);
    }
}

#[tokio::test]
async fn standby_queue_submission_failure_is_retryable() {
    let f = Fixture::new(true).await;
    f.client
        .set_submit_error(Some(StubSubmitError::Rejected("synthetic refusal".into())))
        .await;
    assert!(matches!(
        f.queue(StandbyQueueChoice::Check, &[]).await,
        Err(AppError::DownloadSubmitRejected(_))
    ));
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
async fn standby_queue_failed_grab_leaves_conflicts_running() {
    for error in [
        StubSubmitError::Rejected("synthetic refusal".into()),
        StubSubmitError::SubmitUnavailable("synthetic outage".into()),
    ] {
        let f = Fixture::new(true).await;
        f.conflict("first", &f.title.id).await;
        let result = f.queue(StandbyQueueChoice::Check, &[]).await.unwrap();
        assert_eq!(item_ids(&result.conflicts), vec!["first"]);
        f.client.set_submit_error(Some(error)).await;
        assert!(
            f.queue(StandbyQueueChoice::CancelAndQueue, &result.conflicts)
                .await
                .is_err()
        );
        assert!(f.client.delete_attempts.lock().await.is_empty());
        assert!(
            f.client
                .queue_items
                .lock()
                .await
                .iter()
                .any(|item| item.download_client_item_id == "first")
        );
        assert_eq!(row_ids(&f).await, vec!["first".to_string()]);
        assert_eq!(f.status().await, PendingReleaseStatus::Standby);
        // The conflict is still there to confirm on the retry.
        f.client.set_submit_error(None).await;
        let outcome = f
            .queue(StandbyQueueChoice::CancelAndQueue, &result.conflicts)
            .await
            .unwrap();
        assert!(outcome.grabbed);
        assert_eq!(item_ids(&outcome.cancelled), vec!["first"]);
    }
}

#[tokio::test]
async fn standby_queue_cancel_keeps_the_wanted_rows_the_new_grab_holds() {
    let queued = Fixture::new(true).await;
    queued.conflict("first", &queued.title.id).await;
    let result = queued.queue(StandbyQueueChoice::Check, &[]).await.unwrap();
    assert!(
        queued
            .queue(StandbyQueueChoice::Queue, &result.conflicts)
            .await
            .unwrap()
            .grabbed
    );
    let cancelled = Fixture::new(true).await;
    cancelled.conflict("first", &cancelled.title.id).await;
    let result = cancelled
        .queue(StandbyQueueChoice::Check, &[])
        .await
        .unwrap();
    let outcome = cancelled
        .queue(StandbyQueueChoice::CancelAndQueue, &result.conflicts)
        .await
        .unwrap();
    assert_eq!(item_ids(&outcome.cancelled), vec!["first"]);
    // Cancelling the replaced download after the grab must not reopen the
    // row the new release now holds.
    assert_eq!(
        cancelled.wanted_status().await,
        queued.wanted_status().await
    );
    assert_ne!(
        cancelled.wanted_status().await,
        AcquisitionScopeStatus::Wanted
    );
}

#[tokio::test]
async fn standby_queue_cancel_never_cancels_the_job_the_grab_reused() {
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
    assert_eq!(result.conflicts.len(), 1);
    let outcome = f
        .app
        .queue_standby_release(
            &f.actor,
            &duplicate.id,
            StandbyQueueChoice::CancelAndQueue,
            &identities(&result.conflicts),
        )
        .await
        .unwrap();
    assert!(outcome.grabbed);
    assert!(outcome.cancelled.is_empty());
    assert!(outcome.cancel_failed.is_empty());
    assert!(f.client.delete_attempts.lock().await.is_empty());
    assert_eq!(f.client.submitted_release_titles.lock().await.len(), 1);
    assert_eq!(f.client.queue_items.lock().await.len(), 1);
}

#[tokio::test]
async fn standby_queue_requires_library_manage_permission() {
    let f = Fixture::new(true).await;
    let viewer = test_user_with_app_permissions("viewer", AppPermissionMask::default());
    assert_refused(
        f.app
            .queue_standby_release(&viewer, &f.candidate.id, StandbyQueueChoice::Check, &[])
            .await,
        crate::error_reasons::authorization::LIBRARY_ACCESS_DENIED,
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
    // The first copy is running, so the duplicate first lists it as a
    // conflict; queueing alongside it reuses that same client job.
    let result = f
        .app
        .queue_standby_release(&f.actor, &duplicate.id, StandbyQueueChoice::Check, &[])
        .await
        .unwrap();
    assert!(!result.grabbed);
    assert_eq!(result.conflicts.len(), 1);
    assert!(
        f.app
            .queue_standby_release(
                &f.actor,
                &duplicate.id,
                StandbyQueueChoice::Queue,
                &identities(&result.conflicts),
            )
            .await
            .unwrap()
            .grabbed
    );
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
async fn standby_queue_interrupted_request_still_submits_exactly_once() {
    let f = Fixture::new(true).await;
    let gate = Arc::new(tokio::sync::Notify::new());
    *f.client.submit_gate.lock().await = Some(gate.clone());
    let mut request = Box::pin(f.queue(StandbyQueueChoice::Check, &[]));
    crate::test_wait::within_deadline("operator reaches gated client submission", async {
        tokio::select! {
            result = &mut request => panic!("submission finished before client gate: {result:?}"),
            _ = f.client.submit_started.notified() => {},
        }
    })
    .await;
    // The caller gives up while the client holds the job.
    drop(request);
    // The request still owns the title, so a retry is refused, not doubled.
    assert_refused(
        f.queue(StandbyQueueChoice::Check, &[]).await,
        reason::TITLE_BUSY,
    );
    gate.notify_one();
    crate::test_wait::wait_until("abandoned standby request finishes", || async {
        f.status().await == PendingReleaseStatus::Grabbed
    })
    .await;
    let retry = crate::test_wait::within_deadline(
        "retry after the abandoned request",
        f.queue(StandbyQueueChoice::Check, &[]),
    )
    .await
    .unwrap();
    assert!(retry.grabbed);
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
    // Whichever click arrives second either sees the grab or is told the
    // first still holds the title; neither submits again.
    for result in [first, second] {
        match result {
            Ok(outcome) => assert!(outcome.grabbed),
            Err(error) => assert_eq!(error.reason(), Some(reason::TITLE_BUSY)),
        }
    }
    assert_eq!(f.client.submitted_release_titles.lock().await.len(), 1);
    assert_eq!(f.status().await, PendingReleaseStatus::Grabbed);
}

#[tokio::test]
async fn standby_queue_cancels_a_download_being_processed_best_effort() {
    for state in [
        DownloadQueueState::Verifying,
        DownloadQueueState::Extracting,
        DownloadQueueState::ImportPending,
    ] {
        let f = Fixture::new(true).await;
        f.conflict("first", &f.title.id).await;
        f.conflict_in_state("importing", &f.title.id, state).await;
        let result = f.queue(StandbyQueueChoice::Check, &[]).await.unwrap();
        assert_eq!(result.conflicts.len(), 2);
        let outcome = f
            .queue(StandbyQueueChoice::CancelAndQueue, &result.conflicts)
            .await
            .unwrap();
        assert!(outcome.grabbed, "{state:?}");
        assert_eq!(f.client.delete_attempts.lock().await.len(), 2);
        assert_eq!(outcome.cancelled.len(), 2);
        assert!(outcome.cancel_failed.is_empty());
        assert_eq!(f.client.submitted_release_titles.lock().await.len(), 1);
        assert_eq!(f.status().await, PendingReleaseStatus::Grabbed);
    }
}

#[tokio::test]
async fn standby_queue_queues_after_a_cancel_the_client_keeps_in_history() {
    let f = Fixture::with_client(StubDownloadClient {
        remove_queue_item_on_delete: true,
        keep_cancelled_in_history: true,
        unique_job_ids: true,
        ..Default::default()
    })
    .await;
    f.conflict("first", &f.title.id).await;
    let result = f.queue(StandbyQueueChoice::Check, &[]).await.unwrap();
    assert!(
        f.queue(StandbyQueueChoice::CancelAndQueue, &result.conflicts)
            .await
            .unwrap()
            .grabbed
    );
    // The cancelled job is still listed, as a failed history row.
    assert!(f.client.queue_items.lock().await.iter().any(|item| {
        item.download_client_item_id == "first" && item.state == DownloadQueueState::Failed
    }));
    assert_eq!(f.client.submitted_release_titles.lock().await.len(), 1);
    assert_eq!(f.status().await, PendingReleaseStatus::Grabbed);
}

#[tokio::test]
async fn standby_queue_ignores_rows_on_clients_that_cannot_be_reached() {
    let f = Fixture::new(true).await;
    // A row left on a client that is disabled, removed or not answering.
    f.record_row("stale", &f.title.id, "retired-client").await;
    let result = f.queue(StandbyQueueChoice::Check, &[]).await.unwrap();
    assert!(result.grabbed);
    assert!(result.conflicts.is_empty());
    assert!(f.client.delete_attempts.lock().await.is_empty());
    assert_eq!(f.client.submitted_release_titles.lock().await.len(), 1);
}

#[tokio::test]
async fn standby_queue_partial_cancel_still_queues_and_reports_both_sides() {
    let f = Fixture::new(true).await;
    f.conflict("first", &f.title.id).await;
    f.conflict("second", &f.title.id).await;
    let result = f.queue(StandbyQueueChoice::Check, &[]).await.unwrap();
    assert_eq!(result.conflicts.len(), 2);
    f.client
        .delete_refused_ids
        .lock()
        .await
        .insert("first".into());
    let outcome = f
        .queue(StandbyQueueChoice::CancelAndQueue, &result.conflicts)
        .await
        .unwrap();
    assert!(outcome.grabbed);
    assert!(outcome.conflicts.is_empty());
    // A failed cancel does not stop the later ones.
    assert_eq!(item_ids(&outcome.cancelled), vec!["second"]);
    assert_eq!(item_ids(&outcome.cancel_failed), vec!["first"]);
    let rows = row_ids(&f).await;
    assert!(rows.iter().any(|id| id == "first"));
    assert!(rows.iter().all(|id| id != "second"));
    assert_eq!(f.client.submitted_release_titles.lock().await.len(), 1);
    assert_eq!(f.status().await, PendingReleaseStatus::Grabbed);
}

#[tokio::test]
async fn standby_queue_proceeds_when_no_client_can_be_read() {
    let f = Fixture::new(true).await;
    f.conflict("first", &f.title.id).await;
    f.client.set_queue_error(Some("synthetic outage")).await;
    for choice in [
        StandbyQueueChoice::Check,
        StandbyQueueChoice::CancelAndQueue,
    ] {
        let outcome = f.queue(choice, &[]).await.unwrap();
        assert!(outcome.grabbed, "{choice:?}");
        assert!(outcome.conflicts.is_empty());
    }
    assert!(f.client.delete_attempts.lock().await.is_empty());
    assert_eq!(f.client.submitted_release_titles.lock().await.len(), 1);
    assert_eq!(f.status().await, PendingReleaseStatus::Grabbed);
}

#[tokio::test]
async fn standby_queue_honours_the_blocklist() {
    let f = Fixture::new(true).await;
    f.app
        .services
        .workflow
        .blocklist_repo
        .block(&NewBlocklistEntry {
            title_id: f.title.id.clone(),
            release_name: f.candidate.release_title.clone(),
            indexer_id: String::new(),
            info_hash: None,
            reason: Some("synthetic".into()),
        })
        .await
        .unwrap();
    assert_refused(
        f.queue(StandbyQueueChoice::Check, &[]).await,
        reason::RELEASE_BLOCKLISTED,
    );
    assert!(f.client.submitted_release_titles.lock().await.is_empty());
    assert_eq!(f.status().await, PendingReleaseStatus::Standby);
}

#[tokio::test]
async fn standby_queue_refuses_a_release_already_in_the_client() {
    let f = Fixture::new(true).await;
    let mut item = queue_history_fixture_item("same", DownloadQueueState::Downloading, 0);
    item.title_name = f.candidate.release_title.clone();
    f.client.queue_items.lock().await.push(item);
    assert_refused(
        f.queue(StandbyQueueChoice::Check, &[]).await,
        reason::ALREADY_ACTIVE,
    );
    assert!(f.client.submitted_release_titles.lock().await.is_empty());
    assert_eq!(f.status().await, PendingReleaseStatus::Standby);
}

#[tokio::test]
async fn standby_queue_refuses_while_an_operator_walk_holds_the_title() {
    let f = Fixture::new(true).await;
    let walk = f
        .app
        .runtime
        .acquisition
        .title_walk_locks
        .try_acquire(&f.title.id)
        .await
        .expect("walk lock is free");
    assert_refused(
        crate::test_wait::within_deadline(
            "standby request behind an operator walk",
            f.queue(StandbyQueueChoice::Check, &[]),
        )
        .await,
        reason::TITLE_BUSY,
    );
    drop(walk);
    assert!(
        f.queue(StandbyQueueChoice::Check, &[])
            .await
            .unwrap()
            .grabbed
    );
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
        assert!(
            app.queue_standby_release(
                &actor,
                &id,
                StandbyQueueChoice::Queue,
                &identities(&conflicts.conflicts)
            )
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
    // The automatic grab already runs this exact release in the client, so
    // the saved copy is refused rather than submitted a second time.
    for choice in [
        StandbyQueueChoice::Check,
        StandbyQueueChoice::Queue,
        StandbyQueueChoice::CancelAndQueue,
    ] {
        assert_refused(
            app.queue_standby_release(&actor, &id, choice, &[]).await,
            reason::ALREADY_ACTIVE,
        );
    }
    assert_eq!(client.submitted_release_titles.lock().await.len(), 1);
    assert!(client.deleted_requests.lock().await.is_empty());
}

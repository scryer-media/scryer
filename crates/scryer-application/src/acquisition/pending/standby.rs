use super::*;
use crate::error_reasons::standby as refusal;

fn locator(conflict: &SubmissionScopeConflict) -> ClientJobLocator {
    ClientJobLocator::new(
        conflict.download_client_id.as_deref(),
        &conflict.download_client_type,
        &conflict.download_client_item_id,
    )
}

fn needs_confirmation(
    conflicts: &[SubmissionScopeConflict],
    choice: StandbyQueueChoice,
    confirmed: &[ClientJobLocator],
) -> bool {
    !conflicts.is_empty()
        && (choice == StandbyQueueChoice::Check
            || conflicts
                .iter()
                .any(|item| !confirmed.contains(&locator(item))))
}

/// Whether a fresh client read proves the conflict no longer runs.
///
/// Only the live queue counts. A client may keep the cancelled job in its
/// history under the same id (Weaver archives it as a failed row), and
/// Sonarr's `QueueSpecification` likewise reads only the live queue. A row in
/// a non-blocking state (completed, failed) is therefore not a running job.
fn cancellation_confirmed(
    snapshot: &DownloadClientSnapshotOutcome,
    conflict: &SubmissionScopeConflict,
) -> bool {
    let identity = locator(conflict);
    let authoritative = identity.client_id.as_ref().map_or(
        snapshot.any_client_read_succeeded && snapshot.failed_client_ids.is_empty(),
        |id| snapshot.authoritative_client_ids.contains(id),
    );
    authoritative
        && !snapshot.items.iter().any(|item| {
            item.download_client_item_id == identity.item_id
                && item.client_type == identity.client_type
                && identity
                    .client_id
                    .as_ref()
                    .is_none_or(|id| id == &item.client_id)
                && crate::catalog::workflow::queue_state_blocks_submission(item.state)
        })
}

impl AppUseCase {
    /// Queue a saved release without searching or trusting browser-supplied release facts.
    ///
    /// The work runs to completion even when the caller stops waiting: once a
    /// client may hold the job, abandoning the request half-way would leave
    /// the candidate retryable and a retry would submit it a second time.
    pub async fn queue_standby_release(
        &self,
        actor: &User,
        id: &str,
        choice: StandbyQueueChoice,
        confirmed: &[ClientJobLocator],
    ) -> AppResult<StandbyQueueOutcome> {
        let app = self.clone();
        let actor = actor.clone();
        let id = id.to_string();
        let confirmed = confirmed.to_vec();
        match tokio::spawn(async move {
            app.queue_standby_release_to_completion(&actor, &id, choice, &confirmed)
                .await
        })
        .await
        {
            Ok(result) => result,
            Err(error) if error.is_panic() => std::panic::resume_unwind(error.into_panic()),
            Err(_) => Err(AppError::canceled("standby queue request was cancelled")),
        }
    }

    async fn queue_standby_release_to_completion(
        &self,
        actor: &User,
        id: &str,
        choice: StandbyQueueChoice,
        confirmed: &[ClientJobLocator],
    ) -> AppResult<StandbyQueueOutcome> {
        let repo = &self.services.workflow.pending_releases;
        let initial = repo.get_pending_release(id).await?.ok_or_else(|| {
            AppError::not_found_reason(refusal::NOT_FOUND, "Saved candidate not found")
        })?;
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(&initial.title_id)
            .await?
            .ok_or_else(|| {
                AppError::not_found_reason(refusal::NOT_FOUND, "Candidate title not found")
            })?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        // Saved-candidate walkers hold this through their status cleanup.
        // Use the same lock order as every walk: walk, then submission. A
        // background walk yields; an operator's walk refuses at once rather
        // than holding this request for the length of its search.
        let Some(_walk_guard) = self
            .runtime
            .acquisition
            .title_walk_locks
            .acquire_unless_interactive(&initial.title_id)
            .await
        else {
            return Err(AppError::validation_refused(
                refusal::TITLE_BUSY,
                "Another search or queue action on this title is running; try again when it finishes",
            ));
        };
        let _guard = self
            .runtime
            .acquisition
            .download_submission_guards
            .acquire_title(&initial.title_id)
            .await;
        let pr = repo.get_pending_release(id).await?.ok_or_else(|| {
            AppError::not_found_reason(refusal::NOT_FOUND, "Saved candidate not found")
        })?;
        if pr.status == PendingReleaseStatus::Grabbed {
            return Ok(StandbyQueueOutcome {
                grabbed: true,
                ..Default::default()
            });
        }
        if pr.status != PendingReleaseStatus::Standby || pr.title_id != initial.title_id {
            return Err(AppError::validation_refused(
                refusal::NOT_AVAILABLE,
                "This saved candidate is no longer available",
            ));
        }
        let wanted = self
            .services
            .workflow
            .acquisition_scope_states
            .get_acquisition_scope_state_by_id(&pr.wanted_item_id)
            .await?
            .filter(|row| row.title_id == pr.title_id)
            .ok_or_else(|| {
                AppError::validation_refused(
                    refusal::NOT_QUEUEABLE,
                    "The saved candidate no longer has a valid target",
                )
            })?;
        let now = self.runtime.environment.now();
        let trigger = PendingGrabTrigger::StandbyOperator;
        let PendingJudgement::Admitted(admitted) = self
            .judge_pending_release(&wanted, &pr, &now, trigger)
            .await?
        else {
            return Err(AppError::validation_refused(
                refusal::NOT_QUEUEABLE,
                "The saved candidate cannot currently be queued",
            ));
        };
        let conflicts = self.live_standby_conflicts(&admitted.title).await?;
        if needs_confirmation(&conflicts, choice, confirmed) {
            return Ok(StandbyQueueOutcome {
                grabbed: false,
                conflicts,
                ..Default::default()
            });
        }
        if choice == StandbyQueueChoice::CancelAndQueue && !conflicts.is_empty() {
            if let Some(partial) = self
                .cancel_standby_conflicts(&pr.title_id, &conflicts)
                .await?
            {
                return Ok(partial);
            }
        }
        match self
            .grab_admitted_pending_release(
                &wanted,
                &pr,
                &now,
                trigger,
                *admitted,
                &DomainEventActor::from(actor),
            )
            .await?
        {
            PendingGrabOutcome::Grabbed { .. } => Ok(StandbyQueueOutcome {
                grabbed: true,
                ..Default::default()
            }),
            _ => Err(AppError::validation_refused(
                refusal::NOT_QUEUEABLE,
                "The saved release was not accepted; it remains available to retry",
            )),
        }
    }

    /// Title-wide downloads the client is still running.
    ///
    /// A submission row whose client is disabled, removed or unreachable is
    /// not a live conflict: nothing can be confirmed or cancelled there, and
    /// the canonical submission path treats an answering client's silence the
    /// same way.
    async fn live_standby_conflicts(
        &self,
        title: &Title,
    ) -> AppResult<Vec<SubmissionScopeConflict>> {
        let submissions = self
            .services
            .workflow
            .download_submissions
            .list_for_title(&title.id)
            .await?;
        if submissions.is_empty() {
            return Ok(vec![]);
        }
        let guards = &self.runtime.acquisition.download_submission_guards;
        let generation = guards.client_snapshot_generation();
        let snapshot = self
            .services
            .integrations
            .download_client
            .list_snapshot_outcome_excluding_client_types(100, &[])
            .await?;
        if !snapshot.any_client_read_succeeded {
            return Err(AppError::validation_refused(
                refusal::CANCELLATION_UNCONFIRMED,
                "The current download queue could not be verified",
            ));
        }
        // Canonical deduplication must see the same fresh client failures
        // as this confirmation pass, rather than an older cached queue.
        guards.store_client_snapshot(generation, snapshot.clone());
        let mut accepted = guards
            .cached_title_state(&title.id)
            .map(|state| state.accepted_download_ids)
            .unwrap_or_default();
        for submission in &submissions {
            let on_live_client = submission
                .download_client_id
                .as_ref()
                .is_none_or(|id| snapshot.authoritative_client_ids.contains(id));
            if !on_live_client
                || snapshot.items.iter().any(|item| {
                    item.state == DownloadQueueState::Failed
                        && crate::catalog::workflow::queue_item_matches_submission(item, submission)
                })
            {
                accepted.remove(&submission.download_id);
            }
        }
        let episodes = self
            .services
            .catalog
            .shows
            .list_episodes_for_title(&title.id)
            .await?;
        Self::find_blocking_download_submissions_in_state(
            title,
            &SubmissionScope::Title,
            &submissions,
            &snapshot,
            &episodes,
            &accepted,
            &HashMap::new(),
        )
    }

    /// Cancel every conflict, all-or-nothing as far as the clients allow.
    ///
    /// Nothing is cancelled unless every conflict is still replaceable: a job
    /// that is verifying, extracting or waiting for import is never displaced
    /// (Sonarr's queue never lets a grab displace an importing item). Local
    /// rows are dropped only for jobs a fresh read of the live queue confirms
    /// gone. Returns `Some` outcome when cancellation stopped part-way, naming
    /// what was cancelled and what still runs; the release is then not queued.
    async fn cancel_standby_conflicts(
        &self,
        title_id: &str,
        conflicts: &[SubmissionScopeConflict],
    ) -> AppResult<Option<StandbyQueueOutcome>> {
        if conflicts.iter().any(|conflict| !conflict.replaceable) {
            return Err(AppError::validation_refused(
                refusal::CONFLICT_NOT_REPLACEABLE,
                "A conflicting download is already being processed and cannot be cancelled",
            ));
        }
        let _location = self
            .acquire_location_title_mutation(
                &crate::location::ownership_guard::TITLE_DOWNLOAD_ENTRY,
                title_id,
            )
            .await?;
        let mut requested = Vec::new();
        for conflict in conflicts {
            match self.cancel_conflict_in_client(conflict).await {
                Ok(()) => requested.push(conflict),
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        client_type = conflict.download_client_type.as_str(),
                        item_id = conflict.download_client_item_id.as_str(),
                        "standby: conflicting download could not be cancelled"
                    );
                    break;
                }
            }
        }
        let cancelled: Vec<&SubmissionScopeConflict> = if requested.is_empty() {
            Vec::new()
        } else {
            // Do not discard a binding or submit a replacement on an ambiguous delete.
            match self
                .services
                .integrations
                .download_client
                .list_snapshot_outcome_excluding_client_types(100, &[])
                .await
            {
                Ok(snapshot) => requested
                    .into_iter()
                    .filter(|conflict| cancellation_confirmed(&snapshot, conflict))
                    .collect(),
                Err(error) => {
                    tracing::warn!(error = %error, "standby: cancellation could not be confirmed");
                    Vec::new()
                }
            }
        };
        for conflict in &cancelled {
            self.settle_cancelled_conflict(conflict).await?;
        }
        if cancelled.len() == conflicts.len() {
            return Ok(None);
        }
        if cancelled.is_empty() {
            return Err(AppError::validation_refused(
                refusal::CANCELLATION_UNCONFIRMED,
                "Cancellation has not been confirmed; the saved release was not queued",
            ));
        }
        let remaining = conflicts
            .iter()
            .filter(|conflict| {
                !cancelled
                    .iter()
                    .any(|done| locator(done) == locator(conflict))
            })
            .cloned()
            .collect();
        Ok(Some(StandbyQueueOutcome {
            grabbed: false,
            conflicts: remaining,
            cancelled: cancelled.into_iter().cloned().collect(),
            refusal: Some(refusal::CANCELLATION_INCOMPLETE),
        }))
    }
}

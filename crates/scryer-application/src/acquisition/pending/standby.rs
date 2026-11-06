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
        // The grab runs first: a release the client never accepts must leave
        // the downloads it would have replaced running. The standby submission
        // queues alongside live conflicts, so they do not block it.
        let (grabbed_scope, grabbed_job) = match self
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
            PendingGrabOutcome::Grabbed { scope, job } => (scope, job),
            _ => {
                return Err(AppError::validation_refused(
                    refusal::NOT_QUEUEABLE,
                    "The saved release was not accepted; it remains available to retry",
                ));
            }
        };
        // A submission that reused one of the listed jobs queued the release
        // there; that job is the release now, never something to cancel.
        let conflicts: Vec<_> = conflicts
            .into_iter()
            .filter(|conflict| locator(conflict) != grabbed_job)
            .collect();
        // Best effort, as Sonarr's queue removal: ask each client to cancel,
        // settle locally, and report without waiting for the clients to act.
        let (cancelled, cancel_failed) =
            if choice == StandbyQueueChoice::CancelAndQueue && !conflicts.is_empty() {
                self.cancel_standby_conflicts(&pr.title_id, conflicts, &grabbed_scope)
                    .await
            } else {
                (Vec::new(), Vec::new())
            };
        Ok(StandbyQueueOutcome {
            grabbed: true,
            conflicts: Vec::new(),
            cancelled,
            cancel_failed,
        })
    }

    /// Title-wide downloads the client is still running.
    ///
    /// A submission row whose client is disabled, removed or unreachable is
    /// not a live conflict: nothing can be confirmed or cancelled there, and
    /// the canonical submission path treats an answering client's silence the
    /// same way. When no client can be read at all there is nothing to
    /// cancel, so the release queues as if nothing else were running.
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
        let snapshot = match self
            .services
            .integrations
            .download_client
            .list_snapshot_outcome_excluding_client_types(100, &[])
            .await
        {
            Ok(snapshot) if snapshot.any_client_read_succeeded => snapshot,
            Ok(_) => return Ok(vec![]),
            Err(error) => {
                tracing::warn!(error = %error, "standby: download queue could not be read");
                return Ok(vec![]);
            }
        };
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

    /// Ask each conflict's client to cancel it, without waiting to see it go.
    ///
    /// Cancellation is best effort, whatever the conflict's state: most
    /// clients ignore a cancel during post-processing, and Sonarr's queue
    /// removal likewise issues the client delete and moves on. A conflict
    /// whose client took the request is settled locally at once; one whose
    /// request failed keeps running and stays tracked. Returns the cancelled
    /// and the failed conflicts, in that order.
    /// Runs after the release is queued, so a failure here never undoes that
    /// grab: a conflict that could not be cancelled is reported as such.
    async fn cancel_standby_conflicts(
        &self,
        title_id: &str,
        conflicts: Vec<SubmissionScopeConflict>,
        grabbed_scope: &SubmissionScope,
    ) -> (Vec<SubmissionScopeConflict>, Vec<SubmissionScopeConflict>) {
        let _location = match self
            .acquire_location_title_mutation(
                &crate::location::ownership_guard::TITLE_DOWNLOAD_ENTRY,
                title_id,
            )
            .await
        {
            Ok(location) => location,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    title_id,
                    "standby: conflicting downloads could not be cancelled"
                );
                return (Vec::new(), conflicts);
            }
        };
        let mut cancelled = Vec::new();
        let mut failed = Vec::new();
        for conflict in conflicts {
            match self.cancel_conflict_in_client(&conflict).await {
                Ok(()) => {
                    // The client took the cancel, so the download is gone
                    // whatever happens locally. A failed local settle leaves
                    // the submission row for the tracker to reconcile; it must
                    // not abort the remaining cancels or the grab.
                    if let Err(error) = self
                        .settle_cancelled_conflict(&conflict, Some(grabbed_scope))
                        .await
                    {
                        tracing::warn!(
                            error = %error,
                            client_type = conflict.download_client_type.as_str(),
                            item_id = conflict.download_client_item_id.as_str(),
                            "standby: cancelled download could not be settled locally"
                        );
                    }
                    cancelled.push(conflict);
                }
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        client_type = conflict.download_client_type.as_str(),
                        item_id = conflict.download_client_item_id.as_str(),
                        "standby: conflicting download could not be cancelled"
                    );
                    failed.push(conflict);
                }
            }
        }
        (cancelled, failed)
    }
}

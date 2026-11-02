use super::*;

mod refusal {
    /// The saved candidate no longer exists.
    pub const NOT_FOUND: &str = "STANDBY_NOT_FOUND";
    /// Another workflow owns or has retired this candidate.
    pub const NOT_AVAILABLE: &str = "STANDBY_NOT_AVAILABLE";
    /// Current coverage cannot safely identify what to queue.
    pub const NOT_QUEUEABLE: &str = "STANDBY_NOT_QUEUEABLE";
    /// The client has not confirmed cancellation or queue visibility.
    pub const CANCELLATION_UNCONFIRMED: &str = "STANDBY_CANCELLATION_UNCONFIRMED";
    #[cfg(test)]
    pub const ALL: &[&str] = &[
        NOT_FOUND,
        NOT_AVAILABLE,
        NOT_QUEUEABLE,
        CANCELLATION_UNCONFIRMED,
    ];
}

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
    pub async fn queue_standby_release(
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
        // Use the same lock order: walk, then submission. Keeping the row
        // Standby until acceptance makes an interrupted operator request retryable.
        let _walk_guard = self
            .runtime
            .acquisition
            .title_walk_locks
            .acquire(&initial.title_id)
            .await;
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
                conflicts: vec![],
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
        let submissions = self
            .services
            .workflow
            .download_submissions
            .list_for_title(&pr.title_id)
            .await?;
        let conflicts = if submissions.is_empty() {
            vec![]
        } else {
            let guards = &self.runtime.acquisition.download_submission_guards;
            let generation = guards.client_snapshot_generation();
            let snapshot = self
                .services
                .integrations
                .download_client
                .list_snapshot_outcome_excluding_client_types(100, &[])
                .await?;
            for submission in &submissions {
                if submission
                    .download_client_id
                    .as_ref()
                    .is_some_and(|id| !snapshot.authoritative_client_ids.contains(id))
                    || (submission.download_client_id.is_none()
                        && !snapshot.failed_client_ids.is_empty())
                    || !snapshot.any_client_read_succeeded
                {
                    return Err(AppError::validation_refused(
                        refusal::CANCELLATION_UNCONFIRMED,
                        "The current download queue could not be verified",
                    ));
                }
            }
            // Canonical deduplication must see the same fresh client failures
            // as this confirmation pass, rather than an older cached queue.
            guards.store_client_snapshot(generation, snapshot.clone());
            let mut accepted = self
                .runtime
                .acquisition
                .download_submission_guards
                .cached_title_state(&pr.title_id)
                .map(|state| state.accepted_download_ids)
                .unwrap_or_default();
            for submission in &submissions {
                if snapshot.items.iter().any(|item| {
                    item.state == DownloadQueueState::Failed
                        && crate::catalog::workflow::queue_item_matches_submission(item, submission)
                }) {
                    accepted.remove(&submission.download_id);
                }
            }
            let episodes = self
                .services
                .catalog
                .shows
                .list_episodes_for_title(&pr.title_id)
                .await?;
            Self::find_blocking_download_submissions_in_state(
                &admitted.title,
                &SubmissionScope::Title,
                &submissions,
                &snapshot,
                &episodes,
                &accepted,
                &HashMap::new(),
            )?
        };
        if needs_confirmation(&conflicts, choice, confirmed) {
            return Ok(StandbyQueueOutcome {
                grabbed: false,
                conflicts,
            });
        }
        {
            if choice == StandbyQueueChoice::CancelAndQueue {
                let _location = self
                    .acquire_location_title_mutation(
                        &crate::location::ownership_guard::TITLE_DOWNLOAD_ENTRY,
                        &pr.title_id,
                    )
                    .await?;
                for conflict in &conflicts {
                    self.cancel_standby_conflict(conflict).await?;
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
                    conflicts: vec![],
                }),
                _ => Err(AppError::validation_refused(
                    refusal::NOT_QUEUEABLE,
                    "The saved release was not accepted; it remains available to retry",
                )),
            }
        }
    }

    async fn cancel_standby_conflict(&self, conflict: &SubmissionScopeConflict) -> AppResult<()> {
        let client = &self.services.integrations.download_client;
        if let Some(id) = conflict.download_client_id.as_deref() {
            client
                .delete_queue_item_for_client_id(
                    id,
                    &conflict.download_client_item_id,
                    false,
                    false,
                )
                .await?;
        } else {
            client
                .delete_queue_item_for_client(
                    &conflict.download_client_type,
                    &conflict.download_client_item_id,
                    false,
                    false,
                )
                .await?;
        }
        // Do not discard the binding or submit a replacement on an ambiguous delete.
        let snapshot = client
            .list_snapshot_outcome_excluding_client_types(100, &[])
            .await?;
        let identity = locator(conflict);
        let authoritative = identity.client_id.as_ref().map_or(
            snapshot.any_client_read_succeeded && snapshot.failed_client_ids.is_empty(),
            |id| snapshot.authoritative_client_ids.contains(id),
        );
        if !authoritative
            || snapshot.items.iter().any(|item| {
                item.download_client_item_id == identity.item_id
                    && item.client_type == identity.client_type
                    && identity
                        .client_id
                        .as_ref()
                        .is_none_or(|id| id == &item.client_id)
            })
        {
            return Err(AppError::validation_refused(
                refusal::CANCELLATION_UNCONFIRMED,
                "Cancellation has not been confirmed; the saved release was not queued",
            ));
        }
        self.services
            .workflow
            .download_submissions
            .delete_by_client_item_id(&identity)
            .await?;
        self.runtime
            .acquisition
            .invalidate_download_registry_observations();
        self.runtime
            .acquisition
            .download_submission_guards
            .forget_settled_download(&conflict.title_id);
        self.reset_wanted_items_for_submission_scope(&conflict.title_id, &conflict.scope)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reason_codes_are_unique_and_prefixed() {
        let unique: HashSet<_> = refusal::ALL.iter().copied().collect();
        assert_eq!(unique.len(), refusal::ALL.len());
        assert!(unique.iter().all(|code| code.starts_with("STANDBY_")
            && code.chars().all(|c| c.is_ascii_uppercase() || c == '_')));
    }
}

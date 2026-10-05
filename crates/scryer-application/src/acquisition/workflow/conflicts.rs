impl AppUseCase {
    pub async fn trigger_title_wanted_search(
        &self,
        actor: &User,
        title_id: &str,
        conflict_policy: SubmissionConflictPolicy,
    ) -> AppResult<WantedSearchOutcome> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound("title not found".to_string()))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        self.trigger_title_wanted_search_unchecked(title_id, conflict_policy)
            .await
    }

    /// Queues the title's wanted search without an actor permission check.
    /// Only for callers whose own policy already authorized the add, such as
    /// a media request approved by request policy.
    pub(crate) async fn trigger_title_wanted_search_unchecked(
        &self,
        title_id: &str,
        conflict_policy: SubmissionConflictPolicy,
    ) -> AppResult<WantedSearchOutcome> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound("title not found".to_string()))?;

        let now = Utc::now();
        let outcome = if let Some(handler) = self.facet_registry.get(&title.facet) {
            if handler.has_episodes() {
                self.queue_monitored_series_items_for_search(&title, &now)
                    .await?
            } else if title.monitored {
                self.queue_monitored_movie_for_search(&title, conflict_policy)
                    .await?
            } else {
                WantedSearchOutcome::default()
            }
        } else {
            WantedSearchOutcome::default()
        };

        if outcome.queued_count > 0 {
            self.runtime.acquisition.acquisition_wake.notify_one();
        }

        Ok(outcome)
    }
}
impl AppUseCase {
    async fn wanted_item_blocking_submissions(
        &self,
        title: &Title,
        item: &AcquisitionScopeState,
    ) -> AppResult<Vec<SubmissionScopeConflict>> {
        let scope = self.wanted_item_submission_scope(item).await?;
        self.find_blocking_download_submissions(title, &scope).await
    }
}
impl AppUseCase {
    async fn handle_wanted_item_conflict(
        &self,
        title: &Title,
        item: &AcquisitionScopeState,
        conflict_policy: SubmissionConflictPolicy,
    ) -> AppResult<Option<WantedSearchOutcome>> {
        let conflicts = self.wanted_item_blocking_submissions(title, item).await?;
        if conflicts.is_empty() {
            return Ok(None);
        }

        match conflict_policy {
            SubmissionConflictPolicy::ReplaceEarly
                if conflicts.iter().all(|conflict| conflict.replaceable) =>
            {
                self.replace_blocking_download_submissions(&conflicts)
                    .await?;
                Ok(None)
            }
            SubmissionConflictPolicy::ReplaceEarly | SubmissionConflictPolicy::Abort => {
                let conflict = conflicts
                    .iter()
                    .find(|conflict| !conflict.replaceable)
                    .cloned()
                    .unwrap_or_else(|| conflicts[0].clone());
                Ok(Some(WantedSearchOutcome {
                    queued_count: 0,
                    skipped_in_progress_count: 0,
                    conflict: Some(conflict),
                }))
            }
            SubmissionConflictPolicy::Skip => Ok(Some(WantedSearchOutcome {
                queued_count: 0,
                skipped_in_progress_count: 1,
                conflict: Some(conflicts[0].clone()),
            })),
        }
    }
}
impl AppUseCase {
    /// A user-initiated "search this scope again": after the submission-conflict
    /// gate, re-open the scope's convergence (all coverage pruned, state reset,
    /// acquisition woken) so the cursor searches it on the next cycle even if it
    /// had converged.
    pub(crate) async fn reopen_wanted_scope_with_policy(
        &self,
        title: &Title,
        item: &AcquisitionScopeState,
        conflict_policy: SubmissionConflictPolicy,
    ) -> AppResult<WantedSearchOutcome> {
        if let Some(outcome) = self
            .handle_wanted_item_conflict(title, item, conflict_policy)
            .await?
        {
            return Ok(outcome);
        }

        self.reopen_wanted_scope_for_acquisition(item, CoverageReopen::All)
            .await;

        Ok(WantedSearchOutcome {
            queued_count: 1,
            skipped_in_progress_count: 0,
            conflict: None,
        })
    }
}
impl AppUseCase {
    async fn queue_monitored_movie_for_search(
        &self,
        title: &Title,
        conflict_policy: SubmissionConflictPolicy,
    ) -> AppResult<WantedSearchOutcome> {
        let has_file = self
            .services
            .library
            .media_files
            .list_media_files_for_title(&title.id)
            .await
            .map(|files| files.iter().any(|file| file.role.is_primary()))
            .unwrap_or(false);

        if has_file {
            return Ok(WantedSearchOutcome::default());
        }

        if let Some(item) = self
            .services
            .workflow
            .acquisition_scope_states
            .get_acquisition_scope_state_for_title(&title.id, None)
            .await?
        {
            if item.status == AcquisitionScopeStatus::Grabbed {
                return Ok(WantedSearchOutcome::default());
            }

            return self
                .reopen_wanted_scope_with_policy(title, &item, conflict_policy)
                .await;
        }

        // No state row yet — the scope is simply an untouched derived target.
        // The conflict gate still applies; a clean trigger just wakes the loop
        // (the target derivation already includes this fileless monitored movie).
        let item = self.new_wanted_state_view(title, "movie", None, None, None, None);
        if let Some(outcome) = self
            .handle_wanted_item_conflict(title, &item, conflict_policy)
            .await?
        {
            return Ok(outcome);
        }

        Ok(WantedSearchOutcome {
            queued_count: 1,
            skipped_in_progress_count: 0,
            conflict: None,
        })
    }
}
impl AppUseCase {
    /// An unpersisted state view for a scope with no ledger row yet — used for
    /// conflict checks and as the insert template when a state row is needed.
    pub(crate) fn new_wanted_state_view(
        &self,
        title: &Title,
        media_type: &str,
        episode_id: Option<String>,
        collection_id: Option<String>,
        series_movie_link_id: Option<String>,
        season_number: Option<String>,
    ) -> AcquisitionScopeState {
        let now = Utc::now().to_rfc3339();
        AcquisitionScopeState {
            landed_bar: None,
            id: Id::new().0,
            title_id: title.id.clone(),
            title_name: None,
            title_slug: None,
            title_facet: None,
            library_id: Some(title.library_id.clone()),
            library_name: None,
            library_slug: None,
            episode_id,
            collection_id,
            series_movie_link_id,
            season_number,
            episode_number: None,
            media_type: media_type.to_string(),
            last_search_at: None,
            status: AcquisitionScopeStatus::Wanted,
            grabbed_release: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: now.clone(),
            updated_at: now,
        }
    }
}

use super::*;
use crate::error_reasons::subtitles as reasons;
use crate::{DomainEventActor, JobRun, JobRunRecord, JobRunStatus};
use scryer_domain::{DomainEventPayload, Id, JobRunStartedEventData, LibraryPermission};

const MAX_EPISODES: usize = 1000;

fn normalize_episode_selection(mut ids: Vec<String>) -> AppResult<Vec<String>> {
    if ids.is_empty() || ids.len() > MAX_EPISODES || ids.iter().any(|id| id.trim().is_empty()) {
        return Err(AppError::validation_refused(
            reasons::BATCH_SELECTION_INVALID,
            "Select between one and 1000 episodes from the same series",
        ));
    }
    ids.sort();
    ids.dedup();
    Ok(ids)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum ItemStatus {
    Downloaded,
    AlreadySatisfied,
    MissingMedia,
    NoMatch,
    Deferred,
    Failed,
    Disabled,
}

#[derive(serde::Serialize)]
struct ItemResult {
    id: String,
    status: ItemStatus,
    #[serde(flatten)]
    outcome: AutomaticSubtitleOutcome,
    reason: Option<String>,
}

impl ItemResult {
    fn simple(id: String, status: ItemStatus) -> Self {
        Self {
            id,
            status,
            outcome: AutomaticSubtitleOutcome::default(),
            reason: None,
        }
    }
    fn failed(id: String, error: AppError) -> Self {
        // Subtitle-only users can read the result, but repository and provider diagnostics
        // can contain paths or configuration they are not authorized to inspect.
        warn!(%error, media_file_id = %id, "subtitle batch item failed");
        let status = if error.validation_reason() == Some(reasons::MEDIA_FILE_NOT_FOUND) {
            ItemStatus::MissingMedia
        } else if error.validation_reason() == Some(reasons::BATCH_DISABLED) {
            ItemStatus::Disabled
        } else if matches!(error, AppError::TemporaryUnavailable { .. }) {
            ItemStatus::Deferred
        } else {
            ItemStatus::Failed
        };
        Self {
            id,
            status,
            outcome: AutomaticSubtitleOutcome::default(),
            reason: error.validation_reason().map(str::to_owned),
        }
    }
    fn warning(&self) -> bool {
        matches!(
            self.status,
            ItemStatus::Failed | ItemStatus::Deferred | ItemStatus::Disabled
        ) || self.outcome.failed > 0
            || self.outcome.deferred > 0
            || self.outcome.sync_warnings > 0
    }
}

fn batch_completion_status(items: &[ItemResult]) -> JobRunStatus {
    if !items.is_empty()
        && items
            .iter()
            .all(|item| matches!(item.status, ItemStatus::Failed))
        && !items.iter().any(|item| item.outcome.downloaded > 0)
    {
        JobRunStatus::Failed
    } else if items.iter().any(ItemResult::warning) {
        JobRunStatus::Warning
    } else {
        JobRunStatus::Completed
    }
}

impl AppUseCase {
    pub async fn start_episode_subtitle_search(
        &self,
        actor: &User,
        title_id: &str,
        episode_ids: Vec<String>,
    ) -> AppResult<JobRun> {
        let episode_ids = normalize_episode_selection(episode_ids)?;
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| {
                AppError::not_found_reason(reasons::TITLE_NOT_FOUND, "Title not found")
            })?;
        self.require_library_permission(
            actor,
            &title.library_id,
            LibraryPermission::ManageSubtitles,
        )
        .await?;
        let episodes = self
            .services
            .catalog
            .shows
            .get_episodes_by_ids(&episode_ids)
            .await?;
        if !is_series_title(&title)
            || episodes.len() != episode_ids.len()
            || episodes.iter().any(|episode| episode.title_id != title.id)
        {
            return Err(AppError::validation_refused(
                reasons::BATCH_SELECTION_INVALID,
                "Select between one and 1000 episodes from the same series",
            ));
        }
        let settings = self.subtitle_settings().await?;
        if !settings.enabled {
            return Err(AppError::validation_refused(
                reasons::BATCH_DISABLED,
                "Subtitle handling is disabled",
            ));
        }
        if settings.languages.is_empty() {
            return Err(AppError::validation_refused(
                reasons::BATCH_LANGUAGES_REQUIRED,
                "No wanted subtitle languages are configured",
            ));
        }
        let guard = self
            .runtime
            .jobs
            .interactive_operation_guards
            .try_acquire("episode-subtitle-batch")
            .await
            .ok_or_else(|| {
                AppError::validation_refused(
                    reasons::BATCH_ALREADY_RUNNING,
                    "An episode subtitle batch is already running",
                )
            })?;
        let now = Utc::now();
        let run = JobRunRecord {
            id: Id::new().0,
            job_key: JobKey::SubtitleSearch,
            operation_type: format!("episode_subtitle_search:{}:{}", title.library_id, title.id),
            status: JobRunStatus::Running,
            trigger_source: JobTriggerSource::Manual,
            actor_user_id: Some(actor.id.clone()),
            progress_json: Some(
                serde_json::json!({
                    "kind": "episode_subtitle_search", "status": "running", "phase": "queued",
                    "completed": 0, "total": episode_ids.len(), "episodeIds": episode_ids,
                })
                .to_string(),
            ),
            summary_json: Some(serde_json::json!({"kind": "episode_subtitle_search"}).to_string()),
            summary_text: None,
            error_text: None,
            started_at: now,
            completed_at: None,
            created_at: now,
            updated_at: now,
        };
        let run = self.services.events.job_runs.create_job_run(&run).await?;
        let accepted = JobRun::from_record(&run, None);
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(accepted.clone())
            .await;
        let event_actor = DomainEventActor::from(actor);
        let _ = self
            .append_domain_event(crate::domain_events::new_job_run_domain_event(
                event_actor.clone(),
                run.id.clone(),
                DomainEventPayload::JobRunStarted(JobRunStartedEventData {
                    run_id: run.id.clone(),
                    job_key: run.job_key.as_str().to_owned(),
                    operation_type: run.operation_type.clone(),
                    trigger_source: run.trigger_source.as_str().to_owned(),
                }),
            ))
            .await;
        let app = self.clone();
        let actor = actor.clone();
        self.restore_coordinator().spawn_task(async move {
            let mut run = run;
            let result = app.run_episode_subtitle_batch(&actor, &title.id, &episode_ids, &mut run).await;
            drop(guard);
            match result {
                Ok(items) => {
                    let warnings = items.iter().filter(|item| item.warning()).count();
                    let failed = items.iter().filter(|item| matches!(item.status, ItemStatus::Failed)).count();
                    let status = batch_completion_status(&items);
                    let downloaded: u32 = items.iter().map(|item| item.outcome.downloaded).sum();
                    let summary = serde_json::json!({
                        "kind": "episode_subtitle_search", "total": items.len(), "completed": items.len(),
                        "downloaded": downloaded, "warnings": warnings, "failed": failed, "items": items,
                    });
                    let text = format!("Subtitle batch completed: {downloaded} downloaded, {warnings} item(s) with warnings");
                    if let Err(error) = app.finish_job_run(
                        run, event_actor, Some(text), Some(summary.to_string()), None, Some(status),
                    ).await {
                        warn!(%error, "failed to finish subtitle batch");
                    }
                }
                Err(error) => {
                    run.summary_json = run.progress_json.clone();
                    warn!(%error, run_id = %run.id, "subtitle batch failed");
                    if let Err(error) = app.fail_job_run(run, event_actor,
                        "Subtitle batch could not finish. Retry the selected episodes.".to_string()).await {
                        warn!(%error, "failed to record subtitle batch failure");
                    }
                }
            }
        });
        Ok(accepted)
    }

    async fn run_episode_subtitle_batch(
        &self,
        actor: &User,
        title_id: &str,
        episode_ids: &[String],
        run: &mut JobRunRecord,
    ) -> AppResult<Vec<ItemResult>> {
        let files = self
            .services
            .library
            .media_files
            .list_live_media_files_for_episode_ids(title_id, episode_ids)
            .await?;
        let covered: HashSet<_> = files
            .iter()
            .flat_map(|file| file.episode_ids.iter().cloned())
            .collect();
        let mut results: Vec<_> = episode_ids
            .iter()
            .filter(|id| !covered.contains(*id))
            .map(|id| ItemResult::simple(id.clone(), ItemStatus::MissingMedia))
            .collect();
        let ids: std::collections::BTreeSet<_> =
            files.into_iter().map(|file| file.media_file.id).collect();
        let total = results.len() + ids.len();
        // Keep both the persisted progress and work bounded even with many editions per episode.
        if total > MAX_EPISODES {
            return Err(AppError::validation_refused(
                reasons::BATCH_SELECTION_INVALID,
                "The selection contains more than 1000 media files",
            ));
        }
        let mut search = AutomaticSubtitleSearch::default();
        for id in ids {
            let shutdown = self.restore_coordinator().shutdown_workers.clone();
            if shutdown.is_cancelled() {
                return Err(AppError::temporary_unavailable(
                    "Subtitle batch interrupted by shutdown",
                    None,
                ));
            }
            // Do not cancel a file write halfway through. Existing provider deadlines bound each file;
            // shutdown stops the next file and the provider pacing wait below is cancellation-aware.
            let item = self
                .search_selected_episode_file(actor, title_id, &id, &mut search)
                .await;
            results.push(match item {
                Ok(item) => item,
                Err(error) => ItemResult::failed(id, error),
            });
            run.progress_json = Some(serde_json::json!({"kind": "episode_subtitle_search", "status": "running", "phase": "running",
                "completed": results.len(), "total": total, "items": results}).to_string());
            run.updated_at = Utc::now();
            *run = self.services.events.job_runs.update_job_run(run).await?;
            self.runtime
                .jobs
                .job_run_tracker
                .upsert_active_run(JobRun::from_record(run, None))
                .await;
        }
        if self.restore_coordinator().shutdown_workers.is_cancelled() {
            return Err(AppError::temporary_unavailable(
                "Subtitle batch interrupted by shutdown",
                None,
            ));
        }
        Ok(results)
    }

    async fn search_selected_episode_file(
        &self,
        actor: &User,
        title_id: &str,
        id: &str,
        search: &mut AutomaticSubtitleSearch,
    ) -> AppResult<ItemResult> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| {
                AppError::not_found_reason(reasons::TITLE_NOT_FOUND, "Title not found")
            })?;
        let actor = self.attach_user_authorization(actor.clone()).await?;
        self.require_library_permission(
            &actor,
            &title.library_id,
            LibraryPermission::ManageSubtitles,
        )
        .await?;
        let Some(mf) = self
            .services
            .library
            .media_files
            .get_media_file_by_id(id)
            .await?
        else {
            return Ok(ItemResult::simple(id.to_owned(), ItemStatus::MissingMedia));
        };
        if mf.title_id != title.id
            || !tokio::fs::try_exists(stored_path_to_path_buf(&mf.file_path))
                .await
                .map_err(|error| AppError::Repository(error.to_string()))?
        {
            return Ok(ItemResult::simple(id.to_owned(), ItemStatus::MissingMedia));
        }
        let settings = self.subtitle_settings().await?;
        if !settings.enabled || settings.languages.is_empty() {
            return Ok(ItemResult::simple(id.to_owned(), ItemStatus::Disabled));
        }
        let outcome =
            search_automatic_subtitles_for_file(self, &title, &mf, Some(&actor), search).await?;
        let status = if outcome.failed > 0 {
            ItemStatus::Failed
        } else if outcome.deferred > 0 {
            ItemStatus::Deferred
        } else if outcome.downloaded > 0 {
            ItemStatus::Downloaded
        } else if outcome.searched > 0 {
            ItemStatus::NoMatch
        } else {
            ItemStatus::AlreadySatisfied
        };
        Ok(ItemResult {
            id: id.to_owned(),
            status,
            outcome,
            reason: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subtitle_batch_diagnostics_do_not_expose_repository_details() {
        let result = ItemResult::failed(
            "file-1".into(),
            AppError::Repository("private diagnostic".into()),
        );
        let value = serde_json::to_value(result).unwrap();
        assert_eq!(value["status"], "failed");
        assert!(value.get("error").is_none());
        assert!(!value.to_string().contains("private diagnostic"));
    }

    #[test]
    fn subtitle_batch_preserves_partial_language_success() {
        let mut item = ItemResult::simple("file-1".into(), ItemStatus::Failed);
        item.outcome.failed = 1;
        assert_eq!(
            batch_completion_status(std::slice::from_ref(&item)),
            JobRunStatus::Failed
        );
        item.outcome.downloaded = 1;
        assert_eq!(batch_completion_status(&[item]), JobRunStatus::Warning);
    }

    #[test]
    fn subtitle_batch_deferrals_are_distinct_from_failures() {
        let mut outcome = AutomaticSubtitleOutcome::default();
        outcome.record_error(&AppError::temporary_unavailable(
            "deferred",
            Some(Duration::from_secs(60)),
        ));
        assert_eq!(outcome.deferred, 1);
        assert_eq!(outcome.failed, 0);
        let result = ItemResult::failed(
            "file-1".into(),
            AppError::temporary_unavailable("deferred", None),
        );
        assert!(matches!(result.status, ItemStatus::Deferred));
        assert!(result.warning());
    }
}

use super::*;
use crate::{DomainEventActor, JobKey, JobRun, JobRunRecord, JobRunStatus, JobTriggerSource};
use scryer_domain::{DomainEventPayload, Id, JobRunCompletedEventData, JobRunStartedEventData};
use std::collections::BTreeSet;

impl AppUseCase {
    pub async fn refresh_selected_file_scores(
        &self,
        actor: &User,
        selection: ScoreRefreshSelection,
    ) -> AppResult<JobRun> {
        if selection.ids().is_empty()
            || selection.ids().len() > 1000
            || selection.ids().iter().any(|id| id.trim().is_empty())
        {
            return Err(AppError::validation_refused(
                crate::error_reasons::quality::SELECT_BETWEEN_1_1000_FILES_SEASONS_TITLES,
                "select between 1 and 1000 files, seasons, or titles",
            ));
        }
        let ids: Vec<_> = selection
            .ids()
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let selection = match selection {
            ScoreRefreshSelection::Files(_) => ScoreRefreshSelection::Files(ids),
            ScoreRefreshSelection::Seasons(_) => ScoreRefreshSelection::Seasons(ids),
            ScoreRefreshSelection::Titles(_) => ScoreRefreshSelection::Titles(ids),
        };
        let mut libraries = BTreeSet::new();
        for id in selection.ids() {
            let title_id = match &selection {
                ScoreRefreshSelection::Files(_) => {
                    self.services
                        .library
                        .media_files
                        .get_media_file_by_id(id)
                        .await?
                        .ok_or_else(|| {
                            AppError::not_found_reason(
                                crate::error_reasons::quality::FILE_NOT_FOUND,
                                "selected file",
                            )
                        })?
                        .title_id
                }
                ScoreRefreshSelection::Seasons(_) => {
                    self.services
                        .catalog
                        .shows
                        .get_collection_by_id(id)
                        .await?
                        .ok_or_else(|| {
                            AppError::not_found_reason(
                                crate::error_reasons::quality::SEASON_NOT_FOUND,
                                "selected season",
                            )
                        })?
                        .title_id
                }
                ScoreRefreshSelection::Titles(_) => id.clone(),
            };
            let title = self.get_title(actor, &title_id).await?.ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::quality::TITLE_NOT_FOUND,
                    "selected title",
                )
            })?;
            self.require_library_permission(
                actor,
                &title.library_id,
                LibraryPermission::ManageTitles,
            )
            .await?;
            libraries.insert(title.library_id);
        }
        let runtime = &self.runtime.jobs.score_refresh;
        let mut active = runtime.cancellations.lock().await;
        if active.len() >= 16 {
            return Err(AppError::validation_refused(
                crate::error_reasons::quality::SCORE_REFRESH_QUEUE_FULL,
                "score refresh queue is full",
            ));
        }
        let now = self.runtime.environment.now();
        let run = JobRunRecord {
            id: Id::new().0,
            job_key: JobKey::MediaFileRescore,
            operation_type: format!(
                "media_file_rescore:{}:{}",
                libraries.into_iter().collect::<Vec<_>>().join(","),
                selection.ids().len()
            ),
            status: JobRunStatus::Running,
            trigger_source: JobTriggerSource::Manual,
            actor_user_id: Some(actor.id.clone()),
            progress_json: Some(serde_json::json!({"phase":"queued", "completed":0}).to_string()),
            summary_json: None,
            summary_text: None,
            error_text: None,
            started_at: now,
            completed_at: None,
            created_at: now,
            updated_at: now,
        };
        let run = self.services.events.job_runs.create_job_run(&run).await?;
        let token = tokio_util::sync::CancellationToken::new();
        active.insert(run.id.clone(), (actor.id.clone(), token.clone()));
        drop(active);
        let public_run = JobRun::from_record(&run, None);
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(public_run.clone())
            .await;
        let _ = self
            .append_domain_event(crate::domain_events::new_job_run_domain_event(
                DomainEventActor::from(actor),
                run.id.clone(),
                DomainEventPayload::JobRunStarted(JobRunStartedEventData {
                    run_id: run.id.clone(),
                    job_key: run.job_key.as_str().into(),
                    operation_type: run.operation_type.clone(),
                    trigger_source: run.trigger_source.as_str().into(),
                }),
            ))
            .await;
        let app = self.clone();
        let actor = actor.clone();
        self.restore_coordinator().spawn_task(async move {
            let id = run.id.clone();
            if let Err(error) = app
                .run_score_refresh(&actor, run.clone(), selection, token)
                .await
            {
                tracing::error!(run_id = %id, error = %error, "score refresh job could not finish");
                let mut failed = run;
                failed.status = JobRunStatus::Failed;
                failed.completed_at = Some(app.runtime.environment.now());
                failed.updated_at = failed.completed_at.unwrap();
                failed.error_text =
                    Some("Score refresh interrupted; completed evaluations were retained".into());
                let _ = app.services.events.job_runs.update_job_run(&failed).await;
                app.runtime
                    .jobs
                    .job_run_tracker
                    .upsert_active_run(JobRun::from_record(&failed, None))
                    .await;
            }
            app.runtime
                .jobs
                .score_refresh
                .cancellations
                .lock()
                .await
                .remove(&id);
        });
        Ok(public_run)
    }

    pub async fn cancel_file_score_refresh(&self, actor: &User, run_id: &str) -> AppResult<bool> {
        let active = self.runtime.jobs.score_refresh.cancellations.lock().await;
        let Some((owner, token)) = active.get(run_id) else {
            return Ok(false);
        };
        if owner != &actor.id {
            return Err(AppError::unauthorized_reason(
                crate::error_reasons::quality::SCORE_REFRESH_BELONGS_ANOTHER_USER,
                "score refresh belongs to another user",
            ));
        }
        token.cancel();
        Ok(true)
    }

    async fn run_score_refresh(
        &self,
        actor: &User,
        mut run: JobRunRecord,
        selection: ScoreRefreshSelection,
        cancel: tokio_util::sync::CancellationToken,
    ) -> AppResult<()> {
        let mut completed = 0usize;
        let mut failed = 0usize;
        let mut errors = Vec::new();
        let requested_at = run.started_at.to_rfc3339();
        let runtime = &self.runtime.jobs.score_refresh;
        let worker = tokio::select! {
            guard = runtime.worker.lock() => Some(guard),
            _ = cancel.cancelled() => None,
        };
        let mut after = String::new();
        while worker.is_some() && !cancel.is_cancelled() {
            let page_result = if let ScoreRefreshSelection::Files(ids) = &selection {
                // Keep explicitly selected missing files in the result instead of silently dropping them.
                Ok(ids
                    .iter()
                    .filter(|id| id.as_str() > after.as_str())
                    .cloned()
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .take(100)
                    .collect())
            } else {
                self.services
                    .library
                    .media_files
                    .score_refresh_page(&selection, &after)
                    .await
            };
            let page: Vec<String> = match page_result {
                Ok(page) => page,
                Err(_) => {
                    failed += 1;
                    errors.push(serde_json::json!({"reason":"selection_unavailable"}));
                    break;
                }
            };
            if page.is_empty() {
                break;
            }
            for id in &page {
                if cancel.is_cancelled() {
                    break;
                }
                if self
                    .refresh_file_score(actor, id, &requested_at)
                    .await
                    .is_err()
                {
                    failed += 1;
                    if errors.len() < 100 {
                        errors.push(serde_json::json!({"fileId": id, "reason":"evaluation_failed_or_inputs_changed"}));
                    }
                }
                completed += 1;
                after.clone_from(id);
            }
            run.progress_json = Some(
                serde_json::json!({"phase":"scoring", "completed":completed,"failed":failed})
                    .to_string(),
            );
            run.updated_at = self.runtime.environment.now();
            run = self.services.events.job_runs.update_job_run(&run).await?;
            self.runtime
                .jobs
                .job_run_tracker
                .upsert_active_run(JobRun::from_record(&run, None))
                .await;
        }
        drop(worker);
        let canceled = cancel.is_cancelled();
        run.status = if canceled || failed > 0 {
            JobRunStatus::Warning
        } else {
            JobRunStatus::Completed
        };
        run.completed_at = Some(self.runtime.environment.now());
        run.updated_at = run.completed_at.unwrap();
        run.summary_text = Some(format!(
            "File scores: {} evaluated, {failed} failed{}",
            completed.saturating_sub(failed),
            if canceled { "; canceled" } else { "" }
        ));
        run.summary_json = Some(serde_json::json!({"completed":completed,"failed":failed,"canceled":canceled,"errors":errors}).to_string());
        run.progress_json = run.summary_json.clone();
        let run = self.services.events.job_runs.update_job_run(&run).await?;
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(JobRun::from_record(&run, None))
            .await;
        self.append_domain_event(crate::domain_events::new_job_run_domain_event(
            DomainEventActor::from(actor),
            run.id.clone(),
            DomainEventPayload::JobRunCompleted(JobRunCompletedEventData {
                run_id: run.id.clone(),
                job_key: run.job_key.as_str().into(),
                summary_text: run.summary_text.clone(),
            }),
        ))
        .await?;
        Ok(())
    }
}

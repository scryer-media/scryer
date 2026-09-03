#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TitleLogicalDeleteOptions {
    pub(crate) purge_recycle_bin_entries: bool,
    pub(crate) append_title_deleted_event: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeleteTitlesJobItem {
    pub title_id: String,
    pub preview_fingerprint: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeleteTitlesJobRequest {
    pub items: Vec<DeleteTitlesJobItem>,
    pub delete_files_on_disk: bool,
    pub typed_confirmation: Option<String>,
}

#[derive(Clone, Debug)]
pub struct DeleteTitlesJobAccepted {
    pub job_run: JobRun,
    pub accepted_title_ids: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct DeleteMediaFileJobAccepted {
    pub job_run: JobRun,
}

/// How the on-disk half of a media-file delete is authorized.
#[derive(Clone, Debug)]
pub(crate) enum MediaFileDiskDeletion {
    /// Leave the files on disk; only catalog rows are removed.
    Keep,
    /// Delete from disk after validating this file's own preview fingerprint.
    DeleteConfirmed(DeleteExecutionConfirmation),
    /// Delete from disk; an aggregate preview covering this file was already
    /// validated by the caller.
    DeletePreapproved,
}

/// One media file that could not be deleted during a batch episode-file delete.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeleteEpisodeFileFailure {
    file_id: String,
    error: String,
}

/// Outcome of the batch episode-file deletion run loop. Internal to the job;
/// it is persisted as the run's `summary_json` rather than returned to callers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DeleteEpisodeFilesOutcome {
    deleted_file_ids: Vec<String>,
    failed: Vec<DeleteEpisodeFileFailure>,
}

/// Accepted batch episode-file deletion: the background run plus the media
/// files it will work through.
#[derive(Clone, Debug)]
pub struct DeleteEpisodeFilesJobAccepted {
    pub job_run: JobRun,
    pub accepted_file_ids: Vec<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct EpisodeFileDeletionProgress {
    status: String,
    phase: String,
    title_id: String,
    delete_from_disk: bool,
    total: usize,
    processed: usize,
    deleted: usize,
    failed: usize,
    current_file_id: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct EpisodeFileDeletionSummary {
    title_id: String,
    delete_from_disk: bool,
    deleted_file_ids: Vec<String>,
    failed: Vec<DeleteEpisodeFileFailure>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct TitleDeletionProgress {
    status: String,
    phase: String,
    total: usize,
    processed: usize,
    succeeded: usize,
    failed: usize,
    current_title: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct TitleDeletionFailure {
    title_id: String,
    error: String,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct TitleDeletionSummary {
    total: usize,
    succeeded: usize,
    failed: usize,
    succeeded_title_ids: Vec<String>,
    failures: Vec<TitleDeletionFailure>,
}
impl AppUseCase {
    pub(crate) async fn should_remove_completed_download(
        &self,
        library_id: Option<&str>,
        facet: &MediaFacet,
        client_id: &str,
    ) -> bool {
        match self
            .read_download_client_routing_entry(library_id, facet, client_id)
            .await
            .ok()
            .flatten()
        {
            Some(entry) => entry.remove_completed,
            None => default_download_client_routing_entry().remove_completed,
        }
    }
}
impl AppUseCase {
    pub(crate) async fn should_remove_failed_download(
        &self,
        library_id: Option<&str>,
        facet: &MediaFacet,
        client_id: &str,
    ) -> bool {
        match self
            .read_download_client_routing_entry(library_id, facet, client_id)
            .await
            .ok()
            .flatten()
        {
            Some(entry) => entry.remove_failed,
            None => default_download_client_routing_entry().remove_failed,
        }
    }
}
impl AppUseCase {
    pub async fn delete_title(
        &self,
        actor: &User,
        id: &str,
        delete_files_on_disk: bool,
        delete_confirmation: Option<DeleteExecutionConfirmation>,
    ) -> AppResult<()> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", id)))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        if delete_files_on_disk {
            let delete_confirmation = delete_confirmation.ok_or_else(|| {
                AppError::Validation(
                    "delete preview confirmation is required before deleting files on disk".into(),
                )
            })?;
            let DeleteExecutionConfirmation {
                preview_fingerprint,
                typed_confirmation,
            } = delete_confirmation;
            self.execute_delete_title_files(
                id,
                &preview_fingerprint,
                typed_confirmation.as_deref(),
            )
            .await?;
        }

        self.delete_title_logical_cleanup(
            &title,
            actor,
            TitleLogicalDeleteOptions {
                purge_recycle_bin_entries: true,
                append_title_deleted_event: true,
            },
        )
        .await?;

        Ok(())
    }

    pub async fn start_delete_titles_job(
        &self,
        actor: &User,
        request: DeleteTitlesJobRequest,
    ) -> AppResult<DeleteTitlesJobAccepted> {
        if request.items.is_empty() {
            return Err(AppError::Validation(
                "at least one title is required for deletion".into(),
            ));
        }
        let deletion_guard = self
            .runtime
            .jobs
            .title_deletion_lock
            .clone()
            .try_lock_owned()
            .map_err(|_| AppError::Validation("a title deletion job is already running".into()))?;
        if self
            .runtime
            .jobs
            .job_run_tracker
            .has_active_job(JobKey::TitleDeletion)
            .await
        {
            return Err(AppError::Validation(
                "a title deletion job is already running".into(),
            ));
        }

        let mut seen = HashSet::new();
        for item in &request.items {
            if item.title_id.trim().is_empty() {
                return Err(AppError::Validation("title id is required".into()));
            }
            if !seen.insert(item.title_id.clone()) {
                return Err(AppError::Validation(format!(
                    "duplicate title id in delete request: {}",
                    item.title_id
                )));
            }
            if request.delete_files_on_disk
                && item
                    .preview_fingerprint
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .is_none()
            {
                return Err(AppError::Validation(format!(
                    "delete preview confirmation is required before deleting files on disk for title {}",
                    item.title_id
                )));
            }
            let title = self
                .services
                .catalog
                .titles
                .get_by_id(&item.title_id)
                .await?
                .ok_or_else(|| AppError::NotFound(format!("title {}", item.title_id)))?;
            self.require_library_permission(
                actor,
                &title.library_id,
                scryer_domain::LibraryPermission::ManageTitles,
            )
            .await?;
        }

        let accepted_title_ids = request
            .items
            .iter()
            .map(|item| item.title_id.clone())
            .collect::<Vec<_>>();
        let now = chrono::Utc::now();
        let mut run = JobRunRecord {
            id: Id::new().0,
            job_key: JobKey::TitleDeletion,
            operation_type: format!("title_deletion:{}", accepted_title_ids.len()),
            status: JobRunStatus::Running,
            trigger_source: JobTriggerSource::Manual,
            actor_user_id: Some(actor.id.clone()),
            progress_json: serde_json::to_string(&TitleDeletionProgress {
                status: JobRunStatus::Running.as_str().to_string(),
                phase: "queued".to_string(),
                total: accepted_title_ids.len(),
                processed: 0,
                succeeded: 0,
                failed: 0,
                current_title: None,
            })
            .ok(),
            summary_json: None,
            summary_text: None,
            error_text: None,
            started_at: now,
            completed_at: None,
            created_at: now,
            updated_at: now,
        };
        run = self.services.events.job_runs.create_job_run(&run).await?;
        let run_payload = JobRun::from_record(&run, None);
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(run_payload.clone())
            .await;
        let actor_event = DomainEventActor::from(actor);
        let _ = self
            .append_domain_event(crate::domain_events::new_job_run_domain_event(
                actor_event.clone(),
                run.id.clone(),
                DomainEventPayload::JobRunStarted(JobRunStartedEventData {
                    run_id: run.id.clone(),
                    job_key: run.job_key.as_str().to_string(),
                    operation_type: run.operation_type.clone(),
                    trigger_source: run.trigger_source.as_str().to_string(),
                }),
            ))
            .await;

        let app = self.clone();
        tokio::spawn(async move {
            app.run_delete_titles_job(run, actor_event, request, deletion_guard)
                .await;
        });

        Ok(DeleteTitlesJobAccepted {
            job_run: run_payload,
            accepted_title_ids,
        })
    }
}
impl AppUseCase {
    async fn run_delete_titles_job(
        &self,
        mut run: JobRunRecord,
        actor: DomainEventActor,
        request: DeleteTitlesJobRequest,
        _deletion_guard: tokio::sync::OwnedMutexGuard<()>,
    ) {
        let total = request.items.len();
        let mut succeeded_title_ids = Vec::new();
        let mut failures = Vec::new();

        for item in request.items {
            let phase = if request.delete_files_on_disk {
                "deleting_files"
            } else {
                "cleaning_catalog"
            };
            let _ = self
                .update_title_deletion_progress(
                    &mut run,
                    TitleDeletionProgress {
                        status: JobRunStatus::Running.as_str().to_string(),
                        phase: phase.to_string(),
                        total,
                        processed: succeeded_title_ids.len() + failures.len(),
                        succeeded: succeeded_title_ids.len(),
                        failed: failures.len(),
                        current_title: Some(item.title_id.clone()),
                    },
                )
                .await;

            let result = self
                .delete_title_job_item(
                    actor.clone(),
                    &item,
                    request.delete_files_on_disk,
                    request.typed_confirmation.as_deref(),
                )
                .await;
            match result {
                Ok(()) => succeeded_title_ids.push(item.title_id),
                Err(error) => failures.push(TitleDeletionFailure {
                    title_id: item.title_id,
                    error: error.to_string(),
                }),
            }

            let _ = self
                .update_title_deletion_progress(
                    &mut run,
                    TitleDeletionProgress {
                        status: JobRunStatus::Running.as_str().to_string(),
                        phase: "running".to_string(),
                        total,
                        processed: succeeded_title_ids.len() + failures.len(),
                        succeeded: succeeded_title_ids.len(),
                        failed: failures.len(),
                        current_title: None,
                    },
                )
                .await;
        }

        let summary = TitleDeletionSummary {
            total,
            succeeded: succeeded_title_ids.len(),
            failed: failures.len(),
            succeeded_title_ids,
            failures,
        };
        let status = if summary.failed == 0 {
            JobRunStatus::Completed
        } else if summary.succeeded == 0 {
            JobRunStatus::Failed
        } else {
            JobRunStatus::Warning
        };
        let summary_text = match status {
            JobRunStatus::Completed => format!("Deleted {} title(s)", summary.succeeded),
            JobRunStatus::Warning => format!(
                "Deleted {} title(s); {} failed",
                summary.succeeded, summary.failed
            ),
            JobRunStatus::Failed => format!("Failed to delete {} title(s)", summary.failed),
            _ => "Title deletion finished".to_string(),
        };
        if let Err(error) = self
            .finish_title_deletion_job(run, actor, status, summary_text, summary)
            .await
        {
            warn!(error = %error, "failed to finish title deletion job");
        }
    }

    async fn delete_title_job_item(
        &self,
        actor: DomainEventActor,
        item: &DeleteTitlesJobItem,
        delete_files_on_disk: bool,
        typed_confirmation: Option<&str>,
    ) -> AppResult<()> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(&item.title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", item.title_id)))?;

        if delete_files_on_disk {
            let preview_fingerprint = item
                .preview_fingerprint
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    AppError::Validation(
                        "delete preview confirmation is required before deleting files on disk"
                            .into(),
                    )
                })?;
            self.execute_delete_title_files(
                &item.title_id,
                preview_fingerprint,
                typed_confirmation,
            )
            .await?;
        }

        self.delete_title_logical_cleanup(
            &title,
            actor,
            TitleLogicalDeleteOptions {
                purge_recycle_bin_entries: true,
                append_title_deleted_event: true,
            },
        )
        .await
    }

    async fn update_title_deletion_progress(
        &self,
        run: &mut JobRunRecord,
        progress: TitleDeletionProgress,
    ) -> AppResult<()> {
        run.progress_json = serde_json::to_string(&progress).ok();
        run.updated_at = chrono::Utc::now();
        let updated = self.services.events.job_runs.update_job_run(run).await?;
        *run = updated.clone();
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(JobRun::from_record(&updated, None))
            .await;
        Ok(())
    }

    async fn finish_title_deletion_job(
        &self,
        mut run: JobRunRecord,
        actor: DomainEventActor,
        status: JobRunStatus,
        summary_text: String,
        summary: TitleDeletionSummary,
    ) -> AppResult<()> {
        let completed_at = chrono::Utc::now();
        run.status = status;
        run.progress_json = Some(
            serde_json::json!({
                "status": status.as_str(),
                "phase": "completed",
                "total": summary.total,
                "processed": summary.total,
                "succeeded": summary.succeeded,
                "failed": summary.failed,
                "currentTitle": null,
            })
            .to_string(),
        );
        run.summary_text = Some(summary_text);
        run.summary_json = serde_json::to_string(&summary).ok();
        run.error_text = (status == JobRunStatus::Failed)
            .then(|| "all title deletions failed".to_string());
        run.completed_at = Some(completed_at);
        run.updated_at = completed_at;
        let updated = self.services.events.job_runs.update_job_run(&run).await?;
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(JobRun::from_record(&updated, None))
            .await;
        let payload = if status == JobRunStatus::Failed {
            DomainEventPayload::JobRunFailed(JobRunFailedEventData {
                run_id: updated.id.clone(),
                job_key: updated.job_key.as_str().to_string(),
                error_text: updated.error_text.clone(),
            })
        } else {
            DomainEventPayload::JobRunCompleted(JobRunCompletedEventData {
                run_id: updated.id.clone(),
                job_key: updated.job_key.as_str().to_string(),
                summary_text: updated.summary_text.clone(),
            })
        };
        let _ = self
            .append_domain_event(crate::domain_events::new_job_run_domain_event(
                actor,
                updated.id.clone(),
                payload,
            ))
            .await;
        Ok(())
    }

    pub(crate) async fn delete_title_logical_cleanup(
        &self,
        title: &scryer_domain::Title,
        actor: impl Into<DomainEventActor>,
        options: TitleLogicalDeleteOptions,
    ) -> AppResult<()> {
        let actor = actor.into();
        self.purge_title_logical_dependents(
            title,
            options.purge_recycle_bin_entries,
            actor.clone(),
        )
        .await?;
        self.delete_title_row(title, actor, options.append_title_deleted_event)
            .await
    }
}
impl AppUseCase {
    pub(crate) async fn delete_title_row(
        &self,
        title: &scryer_domain::Title,
        actor: DomainEventActor,
        append_title_deleted_event: bool,
    ) -> AppResult<()> {
        let title_id = title.id.as_str();

        self.services.catalog.titles.delete(title_id).await?;

        if append_title_deleted_event {
            let _ = self
                .append_domain_event(new_title_domain_event(
                    actor,
                    title,
                    DomainEventPayload::TitleDeleted(TitleDeletedEventData {
                        title: title_context_snapshot(title),
                    }),
                ))
                .await;
        }

        Ok(())
    }
}
impl AppUseCase {
    pub async fn start_delete_media_file_job(
        &self,
        actor: &User,
        file_id: &str,
        delete_from_disk: bool,
        delete_confirmation: Option<DeleteExecutionConfirmation>,
    ) -> AppResult<DeleteMediaFileJobAccepted> {
        let guard_key = format!("media-file:{file_id}");
        let deletion_guard = self
            .runtime
            .jobs
            .interactive_operation_guards
            .try_acquire(&guard_key)
            .await
            .ok_or_else(|| {
                AppError::Validation(format!(
                    "a media file deletion is already running for {file_id}"
                ))
            })?;
        let library_id = self
            .validate_delete_media_file_job_request(
                actor,
                file_id,
                delete_from_disk,
                delete_confirmation.as_ref(),
            )
            .await?;

        let now = chrono::Utc::now();
        let mut run = JobRunRecord {
            id: Id::new().0,
            job_key: JobKey::MediaFileDeletion,
            operation_type: format!("media_file_deletion:{library_id}:{file_id}"),
            status: JobRunStatus::Running,
            trigger_source: JobTriggerSource::Manual,
            actor_user_id: Some(actor.id.clone()),
            progress_json: serde_json::json!({
                "status": JobRunStatus::Running.as_str(),
                "phase": "queued",
                "fileId": file_id,
                "deleteFromDisk": delete_from_disk,
            })
            .to_string()
            .into(),
            summary_json: None,
            summary_text: None,
            error_text: None,
            started_at: now,
            completed_at: None,
            created_at: now,
            updated_at: now,
        };
        run = self.services.events.job_runs.create_job_run(&run).await?;
        let job_run = JobRun::from_record(&run, None);
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(job_run.clone())
            .await;
        let actor_event = DomainEventActor::from(actor);
        let _ = self
            .append_domain_event(crate::domain_events::new_job_run_domain_event(
                actor_event.clone(),
                run.id.clone(),
                DomainEventPayload::JobRunStarted(JobRunStartedEventData {
                    run_id: run.id.clone(),
                    job_key: run.job_key.as_str().to_string(),
                    operation_type: run.operation_type.clone(),
                    trigger_source: run.trigger_source.as_str().to_string(),
                }),
            ))
            .await;

        let app = self.clone();
        let actor = actor.clone();
        let file_id = file_id.to_string();
        tokio::spawn(async move {
            app.run_delete_media_file_job(
                run,
                actor_event,
                actor,
                file_id,
                delete_from_disk,
                delete_confirmation,
                deletion_guard,
            )
            .await;
        });

        Ok(DeleteMediaFileJobAccepted { job_run })
    }

    async fn validate_delete_media_file_job_request(
        &self,
        actor: &User,
        file_id: &str,
        delete_from_disk: bool,
        delete_confirmation: Option<&DeleteExecutionConfirmation>,
    ) -> AppResult<String> {
        let media_file = self
            .services
            .library
            .media_files
            .get_media_file_by_id(file_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("media file {file_id}")))?;
        let library_id = self
            .services
            .catalog
            .libraries
            .title_library_id(&media_file.title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", media_file.title_id)))?;
        self.require_library_permission(
            actor,
            &library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        if let Some(DeleteExecutionConfirmation {
            preview_fingerprint,
            typed_confirmation,
        }) = delete_confirmation.filter(|_| delete_from_disk)
        {
            self.validate_delete_media_file(
                file_id,
                preview_fingerprint,
                typed_confirmation.as_deref(),
            )
            .await?;
        } else if delete_from_disk {
            return Err(AppError::Validation(
                "delete preview confirmation is required before deleting files on disk".into(),
            ));
        }

        Ok(library_id)
    }

    #[allow(clippy::too_many_arguments)] // The spawned job boundary owns its inputs explicitly.
    async fn run_delete_media_file_job(
        &self,
        run: JobRunRecord,
        actor_event: DomainEventActor,
        actor: User,
        file_id: String,
        delete_from_disk: bool,
        delete_confirmation: Option<DeleteExecutionConfirmation>,
        _deletion_guard: tokio::sync::OwnedMutexGuard<()>,
    ) {
        let result = self
            .delete_media_file(&actor, &file_id, delete_from_disk, delete_confirmation)
            .await;
        let (status, summary_text, error_text) = match result {
            Ok(()) => (
                JobRunStatus::Completed,
                "Deleted media file".to_string(),
                None,
            ),
            Err(error) => (
                JobRunStatus::Failed,
                "Failed to delete media file".to_string(),
                Some(error.to_string()),
            ),
        };
        if let Err(error) = self
            .finish_media_file_deletion_job(
                run,
                actor_event,
                status,
                &file_id,
                delete_from_disk,
                summary_text,
                error_text,
            )
            .await
        {
            warn!(error = %error, file_id = %file_id, "failed to finish media file deletion job");
        }
    }

    #[allow(clippy::too_many_arguments)] // Completion fields are persisted as one job outcome.
    async fn finish_media_file_deletion_job(
        &self,
        mut run: JobRunRecord,
        actor: DomainEventActor,
        status: JobRunStatus,
        file_id: &str,
        delete_from_disk: bool,
        summary_text: String,
        error_text: Option<String>,
    ) -> AppResult<()> {
        let completed_at = chrono::Utc::now();
        run.status = status;
        run.progress_json = Some(
            serde_json::json!({
                "status": status.as_str(),
                "phase": "completed",
                "fileId": file_id,
                "deleteFromDisk": delete_from_disk,
            })
            .to_string(),
        );
        run.summary_text = Some(summary_text);
        run.summary_json = Some(
            serde_json::json!({
                "fileId": file_id,
                "deleteFromDisk": delete_from_disk,
            })
            .to_string(),
        );
        run.error_text = error_text;
        run.completed_at = Some(completed_at);
        run.updated_at = completed_at;
        let updated = self.services.events.job_runs.update_job_run(&run).await?;
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(JobRun::from_record(&updated, None))
            .await;
        let payload = if status == JobRunStatus::Failed {
            DomainEventPayload::JobRunFailed(JobRunFailedEventData {
                run_id: updated.id.clone(),
                job_key: updated.job_key.as_str().to_string(),
                error_text: updated.error_text.clone(),
            })
        } else {
            DomainEventPayload::JobRunCompleted(JobRunCompletedEventData {
                run_id: updated.id.clone(),
                job_key: updated.job_key.as_str().to_string(),
                summary_text: updated.summary_text.clone(),
            })
        };
        let _ = self
            .append_domain_event(crate::domain_events::new_job_run_domain_event(
                actor,
                updated.id.clone(),
                payload,
            ))
            .await;
        Ok(())
    }

    pub async fn delete_media_file(
        &self,
        actor: &User,
        file_id: &str,
        delete_from_disk: bool,
        delete_confirmation: Option<DeleteExecutionConfirmation>,
    ) -> AppResult<()> {
        let media_file = self
            .services
            .library
            .media_files
            .get_media_file_by_id(file_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("media file {}", file_id)))?;
        let library_id = self
            .services
            .catalog
            .libraries
            .title_library_id(&media_file.title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", media_file.title_id)))?;
        self.require_library_permission(
            actor,
            &library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        let disk_deletion = if delete_from_disk {
            MediaFileDiskDeletion::DeleteConfirmed(delete_confirmation.ok_or_else(|| {
                AppError::Validation(
                    "delete preview confirmation is required before deleting files on disk".into(),
                )
            })?)
        } else {
            MediaFileDiskDeletion::Keep
        };
        self.delete_media_file_authorized(actor, media_file, disk_deletion)
            .await
    }

    /// Delete one media file's disk paths (per `disk_deletion`), catalog row,
    /// dependents, and any movie collection that pointed at it. The caller is
    /// responsible for the `ManageTitles` permission check.
    pub(crate) async fn delete_media_file_authorized(
        &self,
        actor: &User,
        media_file: TitleMediaFile,
        disk_deletion: MediaFileDiskDeletion,
    ) -> AppResult<()> {
        let owned_file_id = media_file.id.clone();
        let file_id = owned_file_id.as_str();
        let delete_from_disk = !matches!(disk_deletion, MediaFileDiskDeletion::Keep);
        let matching_movie_collection_ids = self
            .services
            .catalog
            .shows
            .list_collections_for_title(&media_file.title_id)
            .await?
            .into_iter()
            .filter(|collection| {
                collection.ordered_path.as_deref() == Some(media_file.file_path.as_str())
            })
            .filter_map(|collection| {
                if collection.collection_type == scryer_domain::CollectionType::Movie {
                    Some(collection.id)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        match disk_deletion {
            MediaFileDiskDeletion::Keep => {}
            MediaFileDiskDeletion::DeleteConfirmed(DeleteExecutionConfirmation {
                preview_fingerprint,
                typed_confirmation,
            }) => {
                self.execute_delete_media_file(
                    file_id,
                    &preview_fingerprint,
                    typed_confirmation.as_deref(),
                )
                .await?;
            }
            MediaFileDiskDeletion::DeletePreapproved => {
                self.execute_delete_media_file_preapproved(file_id).await?;
            }
        }

        self.delete_media_file_record_with_dependents(file_id).await?;
        for collection_id in matching_movie_collection_ids {
            if let Err(error) = self
                .services
                .catalog
                .shows
                .delete_collection(&collection_id)
                .await
            {
                tracing::warn!(
                    error = %error,
                    file_id = %file_id,
                    collection_id = %collection_id,
                    file_path = %media_file.file_path,
                    "failed to delete matching movie collection after media file delete"
                );
            }
        }
        info!(
            file_id = %file_id,
            file_path = %media_file.file_path,
            delete_from_disk = %delete_from_disk,
            "media file deleted"
        );

        if delete_from_disk
            && let Ok(Some(title)) = self
                .services
                .catalog
                .titles
                .get_by_id(&media_file.title_id)
                .await
        {
            let _ = self
                .append_domain_event(new_title_domain_event(
                    actor,
                    &title,
                    DomainEventPayload::MediaFileDeleted(MediaFileDeletedEventData {
                        title: title_context_snapshot(&title),
                        media_updates: vec![deleted_media_update(media_file.file_path.clone())],
                        file_id: Some(media_file.id.clone()),
                        reason: MediaFileDeletedReason::Deleted,
                        episode_ids: media_file.episode_id.iter().cloned().collect(),
                    }),
                ))
                .await;
        }

        Ok(())
    }

    /// Start a background job deleting every media file linked to `episode_ids`
    /// on `title_id`.
    ///
    /// Everything that can fail as a whole is validated here, before the run
    /// exists, so the caller sees it as a plain error: the targets are resolved
    /// (which re-checks `ManageTitles` on the title's library and rejects an
    /// empty selection), the aggregate preview is recomputed, and when
    /// `delete_from_disk` is set its fingerprint must still match what the
    /// client confirmed. The per-file interactive guards are also taken up front
    /// for every accepted file and held for the lifetime of the run, so a batch
    /// cannot race a single-file deletion job over the same media file.
    ///
    /// Files whose preview could not be built are still accepted and recorded as
    /// failures while the run works through the rest.
    pub async fn start_delete_episode_files_job(
        &self,
        actor: &User,
        title_id: &str,
        episode_ids: &[String],
        delete_from_disk: bool,
        delete_confirmation: Option<DeleteExecutionConfirmation>,
    ) -> AppResult<DeleteEpisodeFilesJobAccepted> {
        let preview = self
            .preview_delete_episode_files(actor, title_id, episode_ids)
            .await?;

        if delete_from_disk {
            let DeleteExecutionConfirmation {
                preview_fingerprint,
                typed_confirmation,
            } = delete_confirmation.ok_or_else(|| {
                AppError::Validation(
                    "delete preview confirmation is required before deleting files on disk".into(),
                )
            })?;
            crate::library::user_delete::ensure_aggregate_delete_confirmation(
                &preview.preview,
                &preview_fingerprint,
                typed_confirmation.as_deref(),
            )?;
        }

        let library_id = self
            .services
            .catalog
            .libraries
            .title_library_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;

        // Take every per-file guard before the run is created. A busy file aborts
        // the whole request and the guards taken so far are dropped here, so a
        // rejected batch leaves nothing locked.
        let mut deletion_guards = Vec::with_capacity(preview.items.len());
        for item in &preview.items {
            let guard = self
                .runtime
                .jobs
                .interactive_operation_guards
                .try_acquire(&format!("media-file:{}", item.file_id))
                .await
                .ok_or_else(|| {
                    AppError::Validation(format!(
                        "a media file deletion is already running for {}",
                        item.file_id
                    ))
                })?;
            deletion_guards.push(guard);
        }

        let accepted_file_ids = preview
            .items
            .iter()
            .map(|item| item.file_id.clone())
            .collect::<Vec<_>>();
        let total = accepted_file_ids.len();
        let now = chrono::Utc::now();
        let mut run = JobRunRecord {
            id: Id::new().0,
            job_key: JobKey::MediaFileDeletion,
            operation_type: format!("media_file_deletion:{library_id}:episodes:{total}"),
            status: JobRunStatus::Running,
            trigger_source: JobTriggerSource::Manual,
            actor_user_id: Some(actor.id.clone()),
            progress_json: serde_json::to_string(&EpisodeFileDeletionProgress {
                status: JobRunStatus::Running.as_str().to_string(),
                phase: "queued".to_string(),
                title_id: title_id.to_string(),
                delete_from_disk,
                total,
                processed: 0,
                deleted: 0,
                failed: 0,
                current_file_id: None,
            })
            .ok(),
            summary_json: None,
            summary_text: None,
            error_text: None,
            started_at: now,
            completed_at: None,
            created_at: now,
            updated_at: now,
        };
        run = self.services.events.job_runs.create_job_run(&run).await?;
        let job_run = JobRun::from_record(&run, None);
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(job_run.clone())
            .await;
        let actor_event = DomainEventActor::from(actor);
        let _ = self
            .append_domain_event(crate::domain_events::new_job_run_domain_event(
                actor_event.clone(),
                run.id.clone(),
                DomainEventPayload::JobRunStarted(JobRunStartedEventData {
                    run_id: run.id.clone(),
                    job_key: run.job_key.as_str().to_string(),
                    operation_type: run.operation_type.clone(),
                    trigger_source: run.trigger_source.as_str().to_string(),
                }),
            ))
            .await;

        let app = self.clone();
        let actor = actor.clone();
        let title_id = title_id.to_string();
        let items = preview.items;
        tokio::spawn(async move {
            app.run_delete_episode_files_job(
                run,
                actor_event,
                actor,
                title_id,
                items,
                delete_from_disk,
                deletion_guards,
            )
            .await;
        });

        Ok(DeleteEpisodeFilesJobAccepted {
            job_run,
            accepted_file_ids,
        })
    }

    #[allow(clippy::too_many_arguments)] // The spawned job boundary owns its inputs explicitly.
    async fn run_delete_episode_files_job(
        &self,
        mut run: JobRunRecord,
        actor_event: DomainEventActor,
        actor: User,
        title_id: String,
        items: Vec<crate::library::user_delete::DeleteEpisodeFilePreviewResult>,
        delete_from_disk: bool,
        _deletion_guards: Vec<tokio::sync::OwnedMutexGuard<()>>,
    ) {
        let total = items.len();
        let mut outcome = DeleteEpisodeFilesOutcome::default();

        for item in &items {
            let _ = self
                .update_episode_file_deletion_progress(
                    &mut run,
                    EpisodeFileDeletionProgress {
                        status: JobRunStatus::Running.as_str().to_string(),
                        phase: if delete_from_disk {
                            "deleting_files".to_string()
                        } else {
                            "cleaning_catalog".to_string()
                        },
                        title_id: title_id.clone(),
                        delete_from_disk,
                        total,
                        processed: outcome.deleted_file_ids.len() + outcome.failed.len(),
                        deleted: outcome.deleted_file_ids.len(),
                        failed: outcome.failed.len(),
                        current_file_id: Some(item.file_id.clone()),
                    },
                )
                .await;

            if let Some(error) = item.error.as_deref() {
                outcome.failed.push(DeleteEpisodeFileFailure {
                    file_id: item.file_id.clone(),
                    error: error.to_string(),
                });
                continue;
            }
            match self
                .delete_preapproved_media_file(&actor, &item.file_id, delete_from_disk)
                .await
            {
                Ok(()) => outcome.deleted_file_ids.push(item.file_id.clone()),
                Err(error) => {
                    warn!(
                        error = %error,
                        title_id = %title_id,
                        file_id = %item.file_id,
                        episode_id = %item.episode_id,
                        "failed to delete episode media file in batch"
                    );
                    outcome.failed.push(DeleteEpisodeFileFailure {
                        file_id: item.file_id.clone(),
                        error: error.to_string(),
                    });
                }
            }
        }

        info!(
            title_id = %title_id,
            deleted = outcome.deleted_file_ids.len(),
            failed = outcome.failed.len(),
            delete_from_disk = %delete_from_disk,
            "episode media files deleted"
        );

        let summary = EpisodeFileDeletionSummary {
            title_id: title_id.clone(),
            delete_from_disk,
            deleted_file_ids: outcome.deleted_file_ids,
            failed: outcome.failed,
        };
        // Same ladder as the title-deletion job: a run that deleted nothing is
        // a failure, a partial batch is a warning.
        let status = if summary.failed.is_empty() {
            JobRunStatus::Completed
        } else if summary.deleted_file_ids.is_empty() {
            JobRunStatus::Failed
        } else {
            JobRunStatus::Warning
        };
        let summary_text = format!(
            "Deleted {} of {total} episode files",
            summary.deleted_file_ids.len()
        );
        let error_text = (!summary.failed.is_empty()).then(|| {
            summary
                .failed
                .iter()
                .map(|failure| format!("{}: {}", failure.file_id, failure.error))
                .collect::<Vec<_>>()
                .join("; ")
        });
        if let Err(error) = self
            .finish_episode_file_deletion_job(
                run,
                actor_event,
                status,
                total,
                summary_text,
                error_text,
                summary,
            )
            .await
        {
            warn!(error = %error, title_id = %title_id, "failed to finish episode file deletion job");
        }
    }

    async fn update_episode_file_deletion_progress(
        &self,
        run: &mut JobRunRecord,
        progress: EpisodeFileDeletionProgress,
    ) -> AppResult<()> {
        run.progress_json = serde_json::to_string(&progress).ok();
        run.updated_at = chrono::Utc::now();
        let updated = self.services.events.job_runs.update_job_run(run).await?;
        *run = updated.clone();
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(JobRun::from_record(&updated, None))
            .await;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)] // Completion fields are persisted as one job outcome.
    async fn finish_episode_file_deletion_job(
        &self,
        mut run: JobRunRecord,
        actor: DomainEventActor,
        status: JobRunStatus,
        total: usize,
        summary_text: String,
        error_text: Option<String>,
        summary: EpisodeFileDeletionSummary,
    ) -> AppResult<()> {
        let completed_at = chrono::Utc::now();
        run.status = status;
        run.progress_json = serde_json::to_string(&EpisodeFileDeletionProgress {
            status: status.as_str().to_string(),
            phase: "completed".to_string(),
            title_id: summary.title_id.clone(),
            delete_from_disk: summary.delete_from_disk,
            total,
            processed: total,
            deleted: summary.deleted_file_ids.len(),
            failed: summary.failed.len(),
            current_file_id: None,
        })
        .ok();
        run.summary_text = Some(summary_text);
        run.summary_json = serde_json::to_string(&summary).ok();
        run.error_text = error_text;
        run.completed_at = Some(completed_at);
        run.updated_at = completed_at;
        let updated = self.services.events.job_runs.update_job_run(&run).await?;
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(JobRun::from_record(&updated, None))
            .await;
        let payload = if status == JobRunStatus::Failed {
            DomainEventPayload::JobRunFailed(JobRunFailedEventData {
                run_id: updated.id.clone(),
                job_key: updated.job_key.as_str().to_string(),
                error_text: updated.error_text.clone(),
            })
        } else {
            DomainEventPayload::JobRunCompleted(JobRunCompletedEventData {
                run_id: updated.id.clone(),
                job_key: updated.job_key.as_str().to_string(),
                summary_text: updated.summary_text.clone(),
            })
        };
        let _ = self
            .append_domain_event(crate::domain_events::new_job_run_domain_event(
                actor,
                updated.id.clone(),
                payload,
            ))
            .await;
        Ok(())
    }

    /// Delete one media file whose disk removal was already authorized by an
    /// aggregate preview. The caller holds this file's interactive guard for the
    /// lifetime of the batch job, so no guard is taken here.
    async fn delete_preapproved_media_file(
        &self,
        actor: &User,
        file_id: &str,
        delete_from_disk: bool,
    ) -> AppResult<()> {
        let media_file = self
            .services
            .library
            .media_files
            .get_media_file_by_id(file_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("media file {file_id}")))?;
        let disk_deletion = if delete_from_disk {
            MediaFileDiskDeletion::DeletePreapproved
        } else {
            MediaFileDiskDeletion::Keep
        };
        self.delete_media_file_authorized(actor, media_file, disk_deletion)
            .await
    }

    pub(crate) async fn cleanup_media_file_subtitle_state(&self, file_id: &str) -> AppResult<()> {
        let downloads = self
            .services
            .workflow
            .subtitle_downloads
            .list_for_media_file(file_id)
            .await?;
        for download in downloads {
            self.services
                .workflow
                .subtitle_downloads
                .delete(&download.id)
                .await?;
            self.services
                .workflow
                .subtitle_downloads
                .delete_probe_cache_entry(&download.media_file_id, &download.file_path)
                .await?;
        }

        let probe_cache_entries = self
            .services
            .workflow
            .subtitle_downloads
            .list_probe_cache_for_media_file(file_id)
            .await?;
        for entry in probe_cache_entries {
            self.services
                .workflow
                .subtitle_downloads
                .delete_probe_cache_entry(&entry.media_file_id, &entry.file_path)
                .await?;
        }

        Ok(())
    }

    pub(crate) async fn delete_media_file_record_with_dependents(
        &self,
        file_id: &str,
    ) -> AppResult<()> {
        self.cleanup_media_file_subtitle_state(file_id).await?;
        self.services
            .library
            .media_files
            .delete_media_file(file_id)
            .await
    }
}
impl AppUseCase {
    pub async fn delete_collection(&self, actor: &User, collection_id: &str) -> AppResult<()> {
        self.require_collection_permission(
            actor,
            collection_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        self.services
            .catalog
            .shows
            .delete_collection(collection_id)
            .await?;
        Ok(())
    }
}
impl AppUseCase {
    pub async fn delete_episode(&self, actor: &User, episode_id: &str) -> AppResult<()> {
        self.require_episode_permission(
            actor,
            episode_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        self.services
            .catalog
            .shows
            .delete_episode(episode_id)
            .await?;
        Ok(())
    }
}

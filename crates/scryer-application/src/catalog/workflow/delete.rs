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
        // Deleting a title an operation is mid-move would strip the catalog
        // record out from under it (FR-084).
        self.ensure_location_ownership_allows_title(
            &crate::location::ownership_guard::TITLE_DELETE_ENTRY,
            &title.id,
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

    /// Maintenance-policy variant of [`Self::delete_title`] with files on disk
    /// (RFC 137 §9.6). The same fresh-manifest fingerprint check applies; the
    /// human typed confirmation is replaced by the typed policy authorization,
    /// and the actor recorded on the logical cleanup is the system actor the
    /// action handler runs as. Only the maintenance action executor calls this.
    pub(crate) async fn delete_title_by_policy(
        &self,
        actor: &User,
        id: &str,
        preview_fingerprint: &str,
        authorization: &crate::PolicyDeleteAuthorization,
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

        self.execute_delete_title_files_by_policy(id, preview_fingerprint, authorization)
            .await?;

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
        // The bulk job does not route through `delete_title`, so it carries its
        // own check (FR-084). One item's refusal leaves the rest of the batch
        // alone.
        self.ensure_location_ownership_allows_title(
            &crate::location::ownership_guard::TITLE_DELETE_JOB_ENTRY,
            &title.id,
        )
        .await?;

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
        // Removing a file (or its disk copy) under an in-flight move would
        // invalidate the plan the operation is executing (FR-084). The bulk
        // deletion job routes through here too.
        self.ensure_location_ownership_allows_title(
            &crate::location::ownership_guard::MEDIA_FILE_DELETE_ENTRY,
            &media_file.title_id,
        )
        .await?;
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

        if delete_from_disk {
            let delete_confirmation = delete_confirmation.ok_or_else(|| {
                AppError::Validation(
                    "delete preview confirmation is required before deleting files on disk".into(),
                )
            })?;
            let DeleteExecutionConfirmation {
                preview_fingerprint,
                typed_confirmation,
            } = delete_confirmation;
            self.execute_delete_media_file(
                file_id,
                &preview_fingerprint,
                typed_confirmation.as_deref(),
            )
            .await?;
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

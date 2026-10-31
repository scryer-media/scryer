impl AppUseCase {
    pub async fn retry_download_password(
        &self,
        actor: &User,
        download_id: scryer_domain::download_identity::DownloadId,
        source: crate::ClientJobLocator,
        password: &str,
    ) -> AppResult<crate::DownloadClientRetryOutcome> {
        let repository = &self.services.workflow.download_submissions;
        let submission = repository
            .find_by_canonical_download_id(&download_id)
            .await?
            .ok_or_else(|| AppError::not_found_reason(crate::error_reasons::integration::DOWNLOAD_SUBMISSION_NOT_FOUND, "download submission not found"))?;
        if submission.title_id.is_empty() {
            self.require_any_library_permission(
                actor,
                scryer_domain::LibraryPermission::ResolveImports,
            )
            .await?;
        } else {
            self.require_title_library_permission(
                actor,
                &submission.title_id,
                scryer_domain::LibraryPermission::ResolveImports,
            )
            .await?;
        }
        if password.is_empty() {
            return Err(AppError::ArchivePasswordRequired {
                message: "Enter a new archive password to retry this job.".into(),
            });
        }
        if !matches!(source.client_type.as_str(), "sabnzbd" | "nzbget" | "weaver") {
            return Err(AppError::validation_refused(crate::error_reasons::integration::DOWNLOAD_CLIENT_DOES_NOT_SUPPORT_PASSWORD_RETRIES, "this download client does not support password retries"));
        }
        let client_id = source
            .client_id
            .as_deref()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                AppError::validation_refused(crate::error_reasons::integration::PASSWORD_RETRY_REQUIRES_EXACT_DOWNLOAD_CLIENT, "password retry requires an exact download client")
            })?
            .to_owned();
        self.runtime
            .acquisition
            .tracked_download_handle
            .as_ref()
            .ok_or_else(|| AppError::Repository("tracked download service unavailable".into()))?;
        let claim = crate::DownloadPasswordRetryClaim {
            download_id,
            authorized_title_id: submission.title_id,
            source,
            attempt_id: scryer_domain::Id::new().0,
        };
        // Serialize claim admission with deletion preflight, but never hold a
        // title guard across the remote client's retry request.
        let title_guard = if claim.authorized_title_id.is_empty() {
            None
        } else {
            Some(
                self.runtime
                    .jobs
                    .interactive_operation_guards
                    .try_acquire(&format!("title-delete:{}", claim.authorized_title_id))
                    .await
                    .ok_or_else(|| {
                        AppError::validation_refused(
                            crate::title_delete::IN_PROGRESS,
                            "Title deletion is in progress; retry after it finishes.",
                        )
                    })?,
            )
        };
        let claimed = repository.claim_password_retry(&claim, password).await;
        self.invalidate_download_queue_read_model().await;
        if claimed? != crate::DownloadPasswordRetryClaimOutcome::Claimed {
            return Err(AppError::validation_refused(crate::error_reasons::integration::DOWNLOAD_UNAVAILABLE, "download is not eligible for a password retry or is awaiting reconciliation"
                    ));
        }
        drop(title_guard);
        // The durable claim and encrypted replacement precede the single remote
        // mutation. A dropped request or process retains that claim for recovery.
        // The request is bounded, so the claim's store can tell a dispatch
        // still in flight from one that ended without a known outcome.
        let outcome = match tokio::time::timeout(
            crate::DOWNLOAD_PASSWORD_RETRY_DISPATCH_TIMEOUT,
            self.services
                .integrations
                .download_client
                .retry_failed_job_for_client(&client_id, &claim.source.item_id, password),
        )
        .await
        {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(_)) | Err(_) => crate::DownloadClientRetryOutcome::Uncertain,
        };
        let finished = repository.finish_password_retry(&claim, &outcome).await;
        // An error may follow a committed state change; never retain the old
        // recovery projection on an ambiguous completion.
        self.invalidate_download_queue_read_model().await;
        if finished.is_err() {
            return Ok(crate::DownloadClientRetryOutcome::Uncertain);
        }
        if matches!(outcome, crate::DownloadClientRetryOutcome::Accepted { .. }) {
            self.runtime
                .acquisition
                .invalidate_download_registry_observations();
        }
        Ok(outcome)
    }

    pub async fn pause_download_queue_item(
        &self,
        actor: &User,
        client_id: Option<&str>,
        download_client_item_id: &str,
    ) -> AppResult<()> {
        self.require_download_item_permission(
            actor,
            client_id,
            None,
            download_client_item_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        if let Some(client_id) = client_id.filter(|value| !value.trim().is_empty()) {
            self.services
                .integrations
                .download_client
                .pause_queue_item_for_client(client_id, download_client_item_id)
                .await?;
        } else {
            self.services
                .integrations
                .download_client
                .pause_queue_item(download_client_item_id)
                .await?;
        }
        self.emit_download_queue_item_command_issued_event(
            actor,
            download_client_item_id.to_string(),
            scryer_domain::DownloadQueueCommandAction::Pause,
        )
        .await;
        Ok(())
    }
}
impl AppUseCase {
    pub async fn resume_download_queue_item(
        &self,
        actor: &User,
        client_id: Option<&str>,
        download_client_item_id: &str,
    ) -> AppResult<()> {
        self.require_download_item_permission(
            actor,
            client_id,
            None,
            download_client_item_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        if let Some(client_id) = client_id.filter(|value| !value.trim().is_empty()) {
            self.services
                .integrations
                .download_client
                .resume_queue_item_for_client(client_id, download_client_item_id)
                .await?;
        } else {
            self.services
                .integrations
                .download_client
                .resume_queue_item(download_client_item_id)
                .await?;
        }
        self.emit_download_queue_item_command_issued_event(
            actor,
            download_client_item_id.to_string(),
            scryer_domain::DownloadQueueCommandAction::Resume,
        )
        .await;
        Ok(())
    }
}
impl AppUseCase {
    pub async fn delete_download_queue_item(
        &self,
        actor: &User,
        client_id: Option<&str>,
        client_type: &str,
        download_client_item_id: &str,
        is_history: bool,
    ) -> AppResult<crate::DownloadQueueCommandRecord> {
        self.require_download_item_permission(
            actor,
            client_id,
            Some(client_type),
            download_client_item_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        let client_type = self.normalize_download_client_type(client_type)?;
        let locator =
            crate::ClientJobLocator::new(client_id, &client_type, download_client_item_id);
        let canonical_download_id = match self
            .services
            .workflow
            .download_registry
            .find_active_binding_by_locator(&locator)
            .await
        {
            Ok(Some(binding)) => Some(binding.download_id),
            Ok(None) => {
                tracing::warn!(
                    target: "download_identity_resolver",
                    client_config_id = ?locator.client_id,
                    client_type = %locator.client_type,
                    native_item_id = %locator.item_id,
                    "queue delete has no active download binding; using legacy command identity"
                );
                None
            }
            Err(error) => {
                tracing::warn!(
                    target: "download_identity_resolver",
                    client_config_id = ?locator.client_id,
                    client_type = %locator.client_type,
                    native_item_id = %locator.item_id,
                    error = %error,
                    "failed to resolve queue delete download binding; using legacy command identity"
                );
                None
            }
        };
        let command_repository = &self.services.workflow.download_queue_commands;
        let command = if let Some(canonical_download_id) = canonical_download_id.as_ref() {
            command_repository
                .queue_delete_command_for_download(
                    Some(canonical_download_id),
                    client_id,
                    &client_type,
                    download_client_item_id,
                    is_history,
                    Some(actor.id.as_str()),
                )
                .await?
        } else {
            command_repository
                .queue_delete_command(
                    client_id,
                    &client_type,
                    download_client_item_id,
                    is_history,
                    Some(actor.id.as_str()),
                )
                .await?
        };
        self.emit_download_queue_item_command_issued_event(
            actor,
            download_client_item_id.to_string(),
            scryer_domain::DownloadQueueCommandAction::Delete,
        )
        .await;
        Ok(command)
    }
}

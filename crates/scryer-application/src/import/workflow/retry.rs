async fn validate_import_retry_source(
    app: &AppUseCase,
    record: &ImportRecord,
    completed: &CompletedDownload,
) -> AppResult<scryer_domain::download_identity::DownloadId> {
    let locator = completed_download_identity(completed);
    if locator
        != ClientJobLocator::new(
            record.source_client_id.as_deref(),
            &record.source_system,
            &record.source_ref,
        )
    {
        return Err(AppError::Validation(
            "the import record conflicts with its completed source identity; use manual import"
                .into(),
        ));
    }
    let registry = &app.services.workflow.download_registry;
    let binding = registry.find_active_binding_by_locator(&locator).await?;
    let canonical = app
        .services
        .workflow
        .imports
        .canonical_download_id_for_import(&record.id)
        .await?
        .ok_or_else(|| {
            AppError::Validation(
                "the completed source has no trustworthy download identity; use manual import"
                    .into(),
            )
        })?;
    if binding
        .as_ref()
        .is_some_and(|binding| binding.download_id != canonical)
    {
        return Err(AppError::Validation("the client job ID now belongs to a different download; use manual import for the retained files".into()));
    }
    let submissions = &app.services.workflow.download_submissions;
    let identity = DownloadSubmissionIdentity::default();
    let state = submissions
        .get_identity_tracked_state_for_download(Some(&canonical), &identity, Some(&locator))
        .await?;
    let reason = submissions
        .get_identity_tracked_state_reason_for_download(Some(&canonical), &identity, Some(&locator))
        .await?;
    if reason.as_deref() == Some(crate::IMPORT_RETRY_TRACKED_STATE_REASON) {
        return Err(AppError::Validation(
            "this import retry is already running or awaiting reconciliation".into(),
        ));
    }
    if matches!(state.as_deref(), Some("failed" | "failed_pending"))
        && reason.as_deref()
            != Some(crate::tracked_downloads::IMPORT_GATE_REJECTED_TRACKED_STATE_REASON)
    {
        return Err(AppError::Validation(
            "the download failed; retry the download before importing".into(),
        ));
    }
    if matches!(
        state.as_deref(),
        Some("imported" | "imported_seeding" | "ignored")
    ) {
        return Err(AppError::Validation(
            "the download is already settled; refresh its history".into(),
        ));
    }
    if !completed.client_id.trim().is_empty()
        && let Some(config) = app
            .services
            .integrations
            .download_client_configs
            .get_by_id(&completed.client_id)
            .await?
    {
        if !config
            .client_type
            .eq_ignore_ascii_case(&completed.client_type)
        {
            return Err(AppError::Validation(
                "the download client configuration no longer matches this source; use manual import".into(),
            ));
        }
        let mut offset = 0;
        let mut established = false;
        for _ in 0..10 {
            match app
                .services
                .integrations
                .download_client
                .observe_download(&locator, offset)
                .await?
            {
                crate::DownloadClientObservation::Present(item) => {
                    if ClientJobLocator::new(
                        Some(&item.client_id),
                        &item.client_type,
                        &item.download_client_item_id,
                    ) != locator
                    {
                        return Err(AppError::Validation(
                            "the client returned a different download identity; refresh and retry"
                                .into(),
                        ));
                    }
                    if !matches!(
                        item.state,
                        scryer_domain::DownloadQueueState::Completed
                            | scryer_domain::DownloadQueueState::ImportPending
                    ) {
                        return Err(AppError::Validation("the client does not report a completed download; import retry cannot restart a failed or active download".into()));
                    }
                    established = true;
                    break;
                }
                crate::DownloadClientObservation::Absent => {
                    established = true;
                    break;
                }
                crate::DownloadClientObservation::Unknown {
                    next_history_offset,
                    ..
                } => {
                    if next_history_offset <= offset {
                        break;
                    }
                    offset = next_history_offset;
                }
            }
        }
        if !established {
            return Err(AppError::Validation("the client's current download state could not be established; try again when its history is available".into()));
        }
    }
    if !Path::new(&completed.dest_dir).exists() {
        return Err(AppError::Validation("the import source is no longer available; restore the download or correct its path before retrying".into()));
    }
    Ok(canonical)
}

pub(crate) async fn retry_tracked_import(
    app: &AppUseCase,
    actor: &User,
    tracked: &crate::tracked_downloads::TrackedDownload,
) -> AppResult<ImportResult> {
    let source = ClientJobLocator::new(
        Some(&tracked.client_id),
        &tracked.client_type,
        &tracked.client_item.download_client_item_id,
    );
    let mut records = app
        .services
        .workflow
        .imports
        .list_imports_for_identities(&[source])
        .await?;
    records.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    for record in records {
        if app
            .services
            .workflow
            .imports
            .canonical_download_id_for_import(&record.id)
            .await?
            == Some(tracked.download_id)
        {
            return retry_failed_import(app, actor, &record.id, None).await;
        }
    }
    Err(AppError::Validation(
        "no completed import attempt is available to retry; use manual import".into(),
    ))
}

async fn reconcile_claimed_history_retry(
    app: &AppUseCase,
    td: &mut crate::tracked_downloads::TrackedDownload,
    completed: &CompletedDownload,
    evidence: &ReleaseEvidence,
    result: &ImportResult,
    claim: &crate::ImportRetryClaim,
) -> AppResult<()> {
    compute_history_retry_state(app, td, completed, evidence, result).await?;
    let reason = if td.state == TrackedDownloadState::ImportBlocked {
        Some(crate::tracked_downloads::ImportBlockedReason::AfterImport.as_str())
    } else if td.burned_by_import_gate {
        Some(crate::tracked_downloads::IMPORT_GATE_REJECTED_TRACKED_STATE_REASON)
    } else {
        None
    };
    if !app
        .services
        .workflow
        .imports
        .finish_import_retry(
            claim,
            td.state,
            reason,
            td.status_messages.first().map(String::as_str),
        )
        .await?
        .is_finalized()
    {
        return Err(AppError::Validation(
            "a newer import attempt owns this download; refresh its history".into(),
        ));
    }
    if td.state.counts_as_imported() {
        crate::completed_download_handler::schedule_non_destructive_import_mark(
            app,
            td,
            result,
            Some(completed),
        );
    }
    app.refresh_import_record_queue_snapshot(&claim.import_id)
        .await;
    Ok(())
}

pub(crate) fn schedule_import_retry_recovery(app: &AppUseCase) {
    let Ok(cursor) = app
        .runtime
        .imports
        .execution_coordinator
        .retry_recovery_cursor
        .clone()
        .try_lock_owned()
    else {
        return;
    };
    let app = app.clone();
    tokio::spawn(async move {
        let mut cursor = cursor;
        match app
            .services
            .workflow
            .imports
            .list_import_retry_recovery(cursor.as_ref(), 25)
            .await
        {
            Ok(claims) => {
                *cursor = if claims.len() == 25 {
                    claims.last().map(|claim| claim.download_id)
                } else {
                    None
                };
                for claim in claims {
                    if let Err(error) = recover_import_retry(&app, &claim).await {
                        tracing::warn!(import_id = %claim.import_id, error = %error, "import retry remains pending reconciliation");
                    }
                }
            }
            Err(error) => {
                tracing::warn!(error = %error, "could not read import retry recovery work")
            }
        }
    });
}

pub(crate) async fn recover_import_retry(
    app: &AppUseCase,
    claim: &crate::ImportRetryClaim,
) -> AppResult<()> {
    let record = app
        .services
        .workflow
        .imports
        .get_import_by_id(&claim.import_id)
        .await?
        .ok_or_else(|| AppError::NotFound("retry import record".into()))?;
    let payload: StoredCompletedImportRequestPayload =
        serde_json::from_str(&record.payload_json)
            .map_err(|e| AppError::Repository(format!("invalid completed import payload: {e}")))?;
    let (completed, evidence, target_title_id) = match payload {
        StoredCompletedImportRequestPayload::Current(payload) => (
            payload.completed,
            payload.release_evidence,
            payload.target_title_id,
        ),
        StoredCompletedImportRequestPayload::Legacy(completed) => {
            let provenance = resolve_import_provenance(
                app,
                completed,
                ImportProvenanceRequest {
                    identity_policy: CompletedImportIdentityPolicy::RequireSubmission,
                    queue_item: None,
                    requested_target_title_id: None,
                    release_evidence_override: None,
                    persisted: None,
                    tolerate_lookup_failure: false,
                },
            )
            .await?;
            (
                provenance.completed,
                provenance.release_evidence,
                provenance.target_title_id,
            )
        }
    };
    let Some(_permit) = app
        .runtime
        .imports
        .execution_coordinator
        .try_acquire_source(&completed)
        .await
    else {
        return Ok(());
    };
    // The recovery page can race a live attempt finishing while we acquire its permit.
    let current_claim = app
        .services
        .workflow
        .imports
        .get_import_retry_claim(&claim.download_id)
        .await?;
    if !current_claim.is_some_and(|current| {
        current.attempt_id == claim.attempt_id && current.import_id == claim.import_id
    }) {
        return Ok(());
    }
    let record = app
        .services
        .workflow
        .imports
        .get_import_by_id(&claim.import_id)
        .await?
        .ok_or_else(|| AppError::NotFound("retry import record".into()))?;
    let id = crate::tracked_downloads::tracked_download_id(
        Some(&completed.client_id),
        &completed.client_type,
        &completed.download_client_item_id,
    );
    let handle = app.runtime.acquisition.tracked_download_handle.clone();
    let mut tracked = if let Some(handle) = handle.as_ref() {
        handle
            .begin_history_retry(id.clone(), claim.download_id)
            .await?
    } else {
        None
    };
    let mut finished = None;
    let outcome = async {
        if tracked.is_none() { tracked = retry_tracked_snapshot(app, &claim.import_id, &completed).await?; }
        let td = tracked.as_mut().ok_or_else(|| AppError::Repository("retry snapshot unavailable".into()))?;
        // Claiming cleared the old result, so any recorded result belongs to this execution.
        let result = record.result_json.as_deref().and_then(|json| serde_json::from_str::<ImportResult>(json).ok());
        let result = match result {
            Some(result) => result,
            None => {
                let artifacts = app.services.workflow.import_artifacts.list_by_source_identity_for_download(Some(&claim.download_id), &claim.source).await?;
                let verified_candidate = artifacts.iter().any(|artifact| matches!(artifact.result.as_str(), "imported" | "already_present"));
                let result = ImportResult {
                    decision: if verified_candidate { ImportDecision::Skipped } else { ImportDecision::Failed },
                    skip_reason: Some(if verified_candidate { scryer_domain::ImportSkipReason::AlreadyImported } else { scryer_domain::ImportSkipReason::PolicyMismatch }),
                    error_message: Some("Import retry was interrupted; review retained files and retry import if needed".into()),
                    title_id: evidence.title_id().map(str::to_string).or_else(|| target_title_id.clone()),
                    ..base_completed_import_result(&claim.import_id, &completed, &evidence, claim.started_at)
                };
                app.update_import_status_and_notify(&claim.import_id, ImportStatus::Failed, serde_json::to_string(&result).ok()).await?;
                result
            }
        };
        reconcile_claimed_history_retry(app, td, &completed, &evidence, &result, claim).await?;
        finished = tracked.take().map(Box::new);
        Ok(())
    }.await;
    if let Some(handle) = handle {
        handle
            .finish_history_retry(id, claim.download_id, finished)
            .await?;
    }
    outcome
}

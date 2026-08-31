use super::verification::{
    verify_import_inner_with_release_evidence, verify_skipped_import_with_release_evidence,
};
use super::*;

const IMPORT_MARK_RETRY_INITIAL_SECONDS: u64 = 15;
const IMPORT_MARK_RETRY_MAX_SECONDS: u64 = 300;
const IMPORT_MARK_RETRY_MAX_ATTEMPTS: usize = 5;

pub(crate) fn schedule_non_destructive_import_mark(
    app: &AppUseCase,
    td: &TrackedDownload,
    result: &ImportResult,
    completed: Option<&CompletedDownload>,
) {
    let client_id = completed
        .map(|item| item.client_id.trim())
        .filter(|client_id| !client_id.is_empty())
        .unwrap_or_else(|| td.client_id.trim());
    if client_id.is_empty() {
        return;
    }

    let request = if let Some(completed) = completed {
        download_client_mark_imported_request(td, completed, result)
    } else {
        crate::DownloadClientMarkImportedRequest {
            client_item_id: td.client_item.download_client_item_id.clone(),
            info_hash: crate::normalize_torrent_info_hash(td.client_item.download_id.as_deref())
                .or_else(|| {
                    crate::normalize_torrent_info_hash(Some(
                        &td.client_item.download_client_item_id,
                    ))
                }),
            title_id: td.title_id.clone(),
            title_name: (!td.client_item.title_name.trim().is_empty())
                .then(|| td.client_item.title_name.clone()),
            category: td.client_item.category.clone(),
            imported_path: result.dest_path.clone(),
            download_path: None,
        }
    };
    let client_id = client_id.to_string();
    let download_client = app.services.integrations.download_client.clone();

    tokio::spawn(async move {
        let mut retry_seconds = IMPORT_MARK_RETRY_INITIAL_SECONDS;
        for attempt in 1..=IMPORT_MARK_RETRY_MAX_ATTEMPTS {
            match download_client
                .mark_imported_non_destructive_for_client_id(&client_id, &request)
                .await
            {
                Ok(()) => break,
                Err(error) => {
                    if attempt == IMPORT_MARK_RETRY_MAX_ATTEMPTS {
                        tracing::warn!(
                            client_id,
                            client_item_id = %request.client_item_id,
                            attempts = attempt,
                            error = %error,
                            "giving up marking imported download in client after bounded retries"
                        );
                        break;
                    }
                    if attempt == 1 {
                        tracing::warn!(
                            client_id,
                            client_item_id = %request.client_item_id,
                            attempt,
                            retry_seconds,
                            error = %error,
                            "failed to mark imported download in client; retrying"
                        );
                    } else {
                        tracing::debug!(
                            client_id,
                            client_item_id = %request.client_item_id,
                            attempt,
                            retry_seconds,
                            error = %error,
                            "failed to mark imported download in client; retrying"
                        );
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(retry_seconds)).await;
                    retry_seconds = retry_seconds
                        .saturating_mul(2)
                        .min(IMPORT_MARK_RETRY_MAX_SECONDS);
                }
            }
        }
    });
}

async fn apply_no_video_import_backoff(
    app: &AppUseCase,
    td: &mut TrackedDownload,
    result: &ImportResult,
) -> bool {
    if result.skip_reason != Some(ImportSkipReason::NoVideoFiles) {
        return false;
    }

    let signature = no_video_import_source_signature(&result.source_path);
    let attempts = td
        .no_video_import_retry
        .as_ref()
        .filter(|retry| retry.signature == signature)
        .map(|retry| retry.attempts.saturating_add(1))
        .unwrap_or(1);

    if attempts >= NO_VIDEO_BLOCK_AFTER_UNCHANGED_ATTEMPTS {
        let result_json = serde_json::to_string(result).ok();
        if let Err(err) = app
            .update_import_status_and_notify(&result.import_id, ImportStatus::Skipped, result_json)
            .await
        {
            tracing::warn!(
                import_id = result.import_id.as_str(),
                error = %err,
                "failed to mark exhausted no-video import attempt as skipped"
            );
        }
        td.block_no_video_import_after_retries(format!(
            "{} No video files were found after {attempts} unchanged checks. Manual review required.",
            import_result_message(result, ImportStatus::Skipped)
        ));
        return true;
    }

    let delay = if attempts == 1 {
        Duration::seconds(NO_VIDEO_FIRST_RETRY_DELAY_SECS)
    } else {
        Duration::seconds(NO_VIDEO_SECOND_RETRY_DELAY_SECS)
    };
    let next_retry_at = Utc::now() + delay;

    let result_json = serde_json::to_string(result).ok();
    if let Err(err) = app
        .update_import_status_and_notify(&result.import_id, ImportStatus::Pending, result_json)
        .await
    {
        tracing::warn!(
            import_id = result.import_id.as_str(),
            error = %err,
            "failed to restore no-video import attempt to pending status"
        );
    }

    td.schedule_no_video_import_retry(
        signature,
        attempts,
        next_retry_at,
        format!(
            "{} Retrying automatically at {}.",
            import_result_message(result, ImportStatus::Skipped),
            next_retry_at.to_rfc3339()
        ),
    );
    true
}

fn no_video_import_source_signature(source_path: &str) -> NoVideoImportSourceSignature {
    let path = Path::new(source_path);
    let mut signature = NoVideoImportSourceSignature {
        source_path: source_path.to_string(),
        file_count: 0,
        total_bytes: 0,
        latest_mtime: None,
    };
    accumulate_no_video_source_signature(path, &mut signature);
    signature
}

fn accumulate_no_video_source_signature(path: &Path, signature: &mut NoVideoImportSourceSignature) {
    let Ok(metadata) = std::fs::metadata(path) else {
        return;
    };
    update_no_video_signature_mtime(signature, metadata.modified().ok());

    if metadata.is_file() {
        signature.file_count = signature.file_count.saturating_add(1);
        signature.total_bytes = signature.total_bytes.saturating_add(metadata.len());
        return;
    }

    if metadata.is_dir()
        && let Ok(entries) = std::fs::read_dir(path)
    {
        for entry in entries.flatten() {
            accumulate_no_video_source_signature(&entry.path(), signature);
        }
    }
}

fn update_no_video_signature_mtime(
    signature: &mut NoVideoImportSourceSignature,
    modified: Option<std::time::SystemTime>,
) {
    let Some(modified) = modified else {
        return;
    };
    let modified = DateTime::<Utc>::from(modified);
    if signature
        .latest_mtime
        .is_none_or(|latest| modified > latest)
    {
        signature.latest_mtime = Some(modified);
    }
}

fn schedule_import_verification_retry(td: &mut TrackedDownload) {
    td.schedule_import_execution_retry(Utc::now(), |_, next_retry_at| {
        format!(
            "Import verification is temporarily unavailable. Retrying at {}.",
            next_retry_at.to_rfc3339()
        )
    });
}

#[cfg(test)]
pub(super) async fn apply_import_result(
    app: &AppUseCase,
    td: &mut TrackedDownload,
    result: ImportResult,
    files_imported_this_pass: usize,
) -> bool {
    apply_import_result_with_completed(app, td, result, files_imported_this_pass, None, None).await
}

pub(super) async fn apply_import_result_with_completed(
    app: &AppUseCase,
    td: &mut TrackedDownload,
    result: ImportResult,
    files_imported_this_pass: usize,
    completed: Option<&CompletedDownload>,
    release_evidence: Option<&crate::import_workflow::ReleaseEvidence>,
) -> bool {
    let already_imported = result.decision == ImportDecision::Skipped
        && result.skip_reason == Some(ImportSkipReason::AlreadyImported);
    let intentionally_ignored_aggregate =
        result.decision == ImportDecision::Skipped && result.skip_reason.is_none();
    if result.decision == ImportDecision::Imported
        || already_imported
        || intentionally_ignored_aggregate
    {
        if result.decision == ImportDecision::Imported || already_imported {
            td.clear_no_video_import_retry();
            td.clear_import_execution_retry();
        }
        let verification = if intentionally_ignored_aggregate {
            verify_skipped_import_with_release_evidence(
                app,
                td,
                files_imported_this_pass,
                completed,
                release_evidence,
            )
            .await
        } else {
            verify_import_inner_with_release_evidence(
                app,
                td,
                files_imported_this_pass,
                completed,
                release_evidence,
            )
            .await
        };
        let verified = match verification {
            Ok(verified) => verified,
            Err(error) => {
                tracing::warn!(tracked_id = %td.id, error = %error, "import verification evidence is unavailable");
                schedule_import_verification_retry(td);
                return false;
            }
        };
        if verified {
            td.clear_no_video_import_retry();
            td.clear_import_execution_retry();
            td.state = TrackedDownloadState::Imported;
            td.status = TrackedDownloadStatus::Ok;
            td.status_messages.clear();
            schedule_non_destructive_import_mark(app, td, &result, completed);
            return true;
        }

        if result.decision == ImportDecision::Imported {
            td.state = TrackedDownloadState::ImportPending;
            td.status = TrackedDownloadStatus::Warning;
            td.status_messages = vec![
                "Import partially completed; waiting for remaining files or verification."
                    .to_string(),
            ];
            return false;
        }
    }

    if result.decision == ImportDecision::Rejected && result.release_burned {
        // The series aggregate stores the completed download's job directory
        // in `source_path`; equality is safely inside that directory, so a
        // dedicated job directory is removed whole.
        td.clear_no_video_import_retry();
        td.clear_import_execution_retry();
        if !crate::seeding_gate::client_type_is_torrent(app, &td.client_type) {
            let (library_id, resolved_facet) =
                crate::import_workflow::cleanup_routing_scope_for_title_id(
                    app,
                    td.title_id.as_deref(),
                )
                .await;
            let facet = resolved_facet
                .or_else(|| crate::import_workflow::facet_from_tracked_label(td.facet.as_deref()));
            let routing_key = if td.client_id.trim().is_empty() {
                td.client_type.as_str()
            } else {
                td.client_id.as_str()
            };
            let should_remove = match facet.as_ref() {
                Some(facet) => {
                    app.should_remove_failed_download(library_id.as_deref(), facet, routing_key)
                        .await
                }
                None => false,
            };

            if should_remove {
                if let Some(completed) = completed {
                    let rejected_sources = vec![crate::stored_paths::stored_path_to_path_buf(
                        &result.source_path,
                    )];
                    match crate::import_workflow::delete_burned_download_data(
                        app,
                        completed,
                        &rejected_sources,
                    )
                    .await
                    {
                        crate::import_workflow::BurnedDataCleanupOutcome::DeletedDirectory(
                            path,
                        ) => {
                            tracing::info!(
                                path = %path.display(),
                                "import: deleted burned Usenet download directory"
                            );
                        }
                        crate::import_workflow::BurnedDataCleanupOutcome::DeletedFiles(paths) => {
                            tracing::info!(
                                paths = ?paths,
                                "import: deleted burned Usenet download source files"
                            );
                        }
                        crate::import_workflow::BurnedDataCleanupOutcome::Skipped(reason) => {
                            tracing::warn!(
                                import_id = result.import_id.as_str(),
                                job_dir = %completed.dest_dir,
                                reason,
                                "import: skipped burned Usenet download data cleanup"
                            );
                        }
                        crate::import_workflow::BurnedDataCleanupOutcome::Failed(error) => {
                            tracing::warn!(
                                import_id = result.import_id.as_str(),
                                job_dir = %completed.dest_dir,
                                error = %error,
                                "import: failed to clean up burned Usenet download data"
                            );
                        }
                    }
                } else {
                    tracing::warn!(
                        import_id = result.import_id.as_str(),
                        "import: no completed download source available for burned data cleanup"
                    );
                }
            } else {
                tracing::info!(
                    client_id = routing_key,
                    "Remove failed is off for this client; keeping the client's copy of the burned download"
                );
            }
        }

        td.state = TrackedDownloadState::Failed;
        td.status = TrackedDownloadStatus::Error;
        td.status_messages = vec![import_result_message(&result, ImportStatus::Failed)];
        td.burned_by_import_gate = true;
        return false;
    }

    if result.skip_reason == Some(ImportSkipReason::NoVideoFiles) {
        match import_artifacts_for_completed_download(app, td, completed).await {
            Err(error) => {
                tracing::warn!(
                    tracked_id = %td.id,
                    error = %error,
                    "no-video retry cannot read artifact evidence; preserving retryable state"
                );
                schedule_import_verification_retry(td);
                return false;
            }
            Ok(artifacts) => {
                let successful_artifacts = artifacts
                    .iter()
                    .filter(|artifact| {
                        matches!(artifact.result.as_str(), "imported" | "already_present")
                    })
                    .count();
                if successful_artifacts > 0 {
                    let verified = match verify_import_inner_with_release_evidence(
                        app,
                        td,
                        files_imported_this_pass,
                        completed,
                        release_evidence,
                    )
                    .await
                    {
                        Ok(verified) => verified,
                        Err(error) => {
                            tracing::warn!(tracked_id = %td.id, error = %error, "import verification evidence is unavailable");
                            schedule_import_verification_retry(td);
                            return false;
                        }
                    };
                    if verified {
                        td.clear_no_video_import_retry();
                        td.clear_import_execution_retry();
                        td.state = TrackedDownloadState::Imported;
                        td.status = TrackedDownloadStatus::Ok;
                        td.status_messages.clear();
                        schedule_non_destructive_import_mark(app, td, &result, completed);
                        return true;
                    }
                    td.state = TrackedDownloadState::ImportPending;
                    td.status = TrackedDownloadStatus::Warning;
                    td.status_messages = vec![
                        "Import partially completed; waiting for remaining files or verification."
                            .to_string(),
                    ];
                    return false;
                }
            }
        }

        if apply_no_video_import_backoff(app, td, &result).await {
            return false;
        }
    }

    td.clear_no_video_import_retry();

    if completed_import_result_is_retryable(&result) {
        let result_json = serde_json::to_string(&result).ok();
        if let Err(err) = app
            .update_import_status_and_notify(&result.import_id, ImportStatus::Pending, result_json)
            .await
        {
            tracing::warn!(
                import_id = result.import_id.as_str(),
                error = %err,
                "failed to restore retryable import attempt to pending status"
            );
        }
        // Sonarr re-attempts an approved-but-failed import on every refresh
        // and never gives up; Scryer does the same behind a capped backoff so
        // a stuck share or a slow unpack does not hammer the pipeline.
        let attempts = td.schedule_import_execution_retry(Utc::now(), |attempts, next_retry_at| {
            retryable_import_result_message(&result, attempts, next_retry_at)
        });
        tracing::info!(
            id = %td.id,
            import_id = result.import_id.as_str(),
            decision = ?result.decision,
            skip_reason = ?result.skip_reason,
            attempts,
            "import: execution failed; scheduled automatic retry"
        );
        return false;
    }

    td.clear_import_execution_retry();

    match result.decision {
        ImportDecision::Failed => {
            td.state = TrackedDownloadState::ImportBlocked;
            td.status = TrackedDownloadStatus::Error;
            td.status_messages = vec![import_result_message(&result, ImportStatus::Failed)];
            false
        }
        _ => {
            td.state = TrackedDownloadState::ImportBlocked;
            td.status = TrackedDownloadStatus::Warning;
            td.status_messages = vec![import_result_message(&result, ImportStatus::Skipped)];
            false
        }
    }
}

fn download_client_mark_imported_request(
    td: &TrackedDownload,
    completed: &CompletedDownload,
    result: &ImportResult,
) -> crate::DownloadClientMarkImportedRequest {
    crate::DownloadClientMarkImportedRequest {
        client_item_id: completed.download_client_item_id.clone(),
        info_hash: crate::normalize_torrent_info_hash(completed.download_id.as_deref()).or_else(
            || crate::normalize_torrent_info_hash(Some(&completed.download_client_item_id)),
        ),
        title_id: td.title_id.clone(),
        title_name: (!td.client_item.title_name.trim().is_empty())
            .then(|| td.client_item.title_name.clone()),
        category: completed
            .category
            .clone()
            .or_else(|| td.client_item.category.clone()),
        imported_path: result
            .dest_path
            .as_deref()
            .filter(|path| !path.trim().is_empty())
            .map(str::to_string),
        download_path: (!completed.dest_dir.trim().is_empty()).then(|| completed.dest_dir.clone()),
    }
}

fn retryable_import_result_message(
    result: &ImportResult,
    attempts: u32,
    next_retry_at: DateTime<Utc>,
) -> String {
    let detail = import_result_message(result, ImportStatus::Skipped);
    format!(
        "{detail} Retrying automatically (attempt {attempts}) at {}.",
        next_retry_at.to_rfc3339()
    )
}

fn import_result_message(result: &ImportResult, fallback_status: ImportStatus) -> String {
    if let Some(message) = result
        .error_message
        .as_ref()
        .filter(|message| !message.trim().is_empty())
    {
        return message.clone();
    }

    if let Some(skip_reason) = result.skip_reason.as_ref() {
        return format!("Import blocked: {}", skip_reason.as_str());
    }

    format!("Import ended with status {}", fallback_status.as_str())
}

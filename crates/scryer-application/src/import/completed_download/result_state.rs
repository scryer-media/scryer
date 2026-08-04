use super::verification::verify_import_inner;
use super::*;

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

#[cfg(test)]
pub(super) async fn apply_import_result(
    app: &AppUseCase,
    td: &mut TrackedDownload,
    result: ImportResult,
    files_imported_this_pass: usize,
) -> bool {
    apply_import_result_with_completed(app, td, result, files_imported_this_pass, None).await
}

pub(super) async fn apply_import_result_with_completed(
    app: &AppUseCase,
    td: &mut TrackedDownload,
    result: ImportResult,
    files_imported_this_pass: usize,
    completed: Option<&CompletedDownload>,
) -> bool {
    let already_imported = result.skip_reason == Some(ImportSkipReason::AlreadyImported);
    if result.decision == ImportDecision::Imported || already_imported {
        td.clear_no_video_import_retry();
        if verify_import_inner(app, td, files_imported_this_pass, completed).await {
            td.state = TrackedDownloadState::Imported;
            td.status = TrackedDownloadStatus::Ok;
            td.status_messages.clear();
            return true;
        }

        if already_imported {
            td.state = TrackedDownloadState::Imported;
            td.status = TrackedDownloadStatus::Ok;
            td.status_messages.clear();
            return true;
        }

        td.state = TrackedDownloadState::ImportPending;
        td.status = TrackedDownloadStatus::Warning;
        td.status_messages = vec![
            "Import partially completed; waiting for remaining files or verification.".to_string(),
        ];
        return false;
    }

    if apply_no_video_import_backoff(app, td, &result).await {
        return false;
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
        td.state = TrackedDownloadState::ImportPending;
        td.status = TrackedDownloadStatus::Warning;
        td.status_messages = vec![retryable_import_result_message(&result)];
        return false;
    }

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

fn retryable_import_result_message(result: &ImportResult) -> String {
    let detail = import_result_message(result, ImportStatus::Skipped);
    format!("{detail} Retrying automatically.")
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

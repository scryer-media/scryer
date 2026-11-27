//! FailedDownloadHandler — failure detection and processing.
//!
//! check(): detects downloads that failed in the client or are encrypted.
//! process_failed(): records the failure, emits events, and optionally reacquires.

use scryer_domain::{DownloadQueueState, TrackedDownloadState, TrackedDownloadStatus};

use crate::acquisition_workflow::{DownloadFailureContext, FailureHandlingOutcome};
use crate::tracked_downloads::TrackedDownload;
use crate::{AppUseCase, ClientJobLocator};

/// Detect failed downloads during the poll cycle.
///
/// Called for downloads that have not reached a terminal tracked state. If the
/// client reports failure, transitions to FailedPending before import can run.
pub fn check(td: &mut TrackedDownload) {
    check_at(td, chrono::Utc::now());
}

pub(crate) fn check_at(td: &mut TrackedDownload, now: chrono::DateTime<chrono::Utc>) {
    if td.blocklist_confirmation_failed {
        return;
    }
    // Only process if in a check-eligible state.
    if !matches!(
        td.state,
        TrackedDownloadState::Downloading
            | TrackedDownloadState::ImportPending
            | TrackedDownloadState::ImportBlocked
    ) {
        return;
    }

    if td.client_item.state != DownloadQueueState::Failed {
        // A job that left the failed state (retried in the client) gets a
        // fresh answer if it fails again.
        td.failed_without_grab_recheck = None;
        return;
    }

    if !tracked_download_has_scryer_failure_origin(td) {
        warn_download_not_grabbed(td);
        return;
    }

    if td
        .failed_without_grab_recheck
        .as_ref()
        .is_some_and(|recheck| recheck.next_retry_at > now)
    {
        warn_download_not_grabbed(td);
        return;
    }

    td.state = TrackedDownloadState::FailedPending;
    td.status = TrackedDownloadStatus::Error;
    td.status_messages.clear();
}

/// Process a download in FailedPending state.
///
/// Records the failure, emits activity events, and optionally triggers
/// a re-search for the same title.
pub async fn process_failed(app: &AppUseCase, td: &mut TrackedDownload) {
    if td.state != TrackedDownloadState::FailedPending {
        return;
    }

    match tracked_download_grabbed_submission_lookup(app, td).await {
        Ok(true) => td.failed_without_grab_recheck = None,
        Ok(false) => {
            // The origin marker and the missing row disagree. Remember the
            // answer so `check` stops re-queueing the job every poll, and ask
            // again on a backoff: a fresh grab writes its row only after the
            // client accepted the job, so the row can still arrive.
            let attempts = td
                .failed_without_grab_recheck
                .as_ref()
                .map(|recheck| recheck.attempts.saturating_add(1))
                .unwrap_or(1);
            td.failed_without_grab_recheck =
                Some(crate::tracked_downloads::ImportExecutionRetryState {
                    attempts,
                    next_retry_at: chrono::Utc::now()
                        + crate::tracked_downloads::import_execution_retry_delay(attempts),
                });
            warn_download_not_grabbed(td);
            td.state = TrackedDownloadState::Downloading;
            td.skip_reacquire_on_failure = false;
            td.manual_failure_cleanup = false;
            return;
        }
        Err(error) => {
            // A failed read is not "no grab on record": the download stays
            // where it was and the next poll asks again.
            tracing::warn!(
                %error,
                reason_code = crate::error_reasons::download_origin::LOOKUP_FAILED,
                id = %td.id,
                "could not read whether Scryer grabbed this failed download; keeping its state"
            );
            return;
        }
    }

    if !td.manual_failure_cleanup
        && let Some(failure) = td.client_item.password_failure()
    {
        if crate::acquisition_workflow::record_password_retry_failure(app, td, failure)
            .await
            .is_err()
        {
            td.status_messages = vec![crate::tracked_downloads::TrackedStatusMessage::from(
                "Could not record password failure; awaiting retry",
            )];
            return;
        }
        // Retain the existing release for an operator password retry. A
        // credential failure is not evidence to blocklist or replace it.
        td.state = TrackedDownloadState::Failed;
        td.status = TrackedDownloadStatus::Error;
        td.status_messages = vec![crate::tracked_downloads::TrackedStatusMessage::from(
            failure.message(),
        )];
        td.skip_reacquire_on_failure = true;
        return;
    }

    // Import's verdict lives in the status messages: the client item is the
    // client's own report and is refreshed on every poll.
    let import_verdict = td
        .status_messages
        .iter()
        .find_map(|message| match message.kind {
            crate::tracked_downloads::TrackedStatusMessageKind::UnwantedExecutables => Some((
                message,
                crate::error_reasons::download_failure::UNWANTED_EXECUTABLES,
            )),
            crate::tracked_downloads::TrackedStatusMessageKind::ImportRefused => Some((
                message,
                crate::error_reasons::download_failure::IMPORT_REFUSED,
            )),
            _ => None,
        });
    let failure_code = import_verdict.map(|(_, code)| code);
    let failure_reason = import_verdict
        .map(|(message, _)| message)
        .map(|message| message.text.as_str())
        .or(td.client_item.attention_reason.as_deref())
        .unwrap_or("Failed download detected")
        .to_owned();
    let failure_reason = failure_reason.as_str();

    tracing::warn!(
        id = %td.id,
        title_id = ?td.title_id,
        reason = failure_reason,
        "download failed - processing failure"
    );

    let outcome = crate::acquisition_workflow::process_download_failure_with_cleanup_intent(
        app,
        td.canonical_download_id(),
        DownloadFailureContext {
            wanted_item: None,
            title_id: td.title_id.clone(),
            client_id: td.client_id.clone(),
            client_type: td.client_type.clone(),
            client_name: Some(td.client_item.client_name.clone()),
            client_item_id: td.client_item.download_client_item_id.clone(),
            release_title: td
                .source_title
                .clone()
                .unwrap_or_else(|| td.client_item.title_name.clone()),
            reason: failure_reason.to_string(),
            // The import gate and the warning timeout route a download here
            // with this flag set; the blocklist then names Scryer's verdict,
            // not the client.
            blocklist_reason: td
                .burned_by_import_gate
                .then(|| format!("import rejected: {failure_reason}")),
            failure_code,
            skip_reacquire: td.skip_reacquire_on_failure,
        },
        td.manual_failure_cleanup,
    )
    .await;
    if outcome == FailureHandlingOutcome::RecordingFailed {
        td.status_messages = vec![crate::tracked_downloads::TrackedStatusMessage::from(
            "Could not confirm blocklist entry; download preserved for retry",
        )];
        return;
    }
    if outcome == FailureHandlingOutcome::MissingBlocklistKey {
        let message = "Cannot blocklist this download because its title or release name is missing. Automatic retries stopped; download and files are preserved. Resolve the missing identity before retrying the blocklist action.";
        crate::tracked_downloads::persist_import_blocked_state_marker(
            app,
            td,
            crate::tracked_downloads::ImportBlockedReason::BlocklistConfirmationFailed,
            Some(message),
        )
        .await;
        td.state = TrackedDownloadState::ImportBlocked;
        td.status = TrackedDownloadStatus::Error;
        td.status_messages = vec![crate::tracked_downloads::TrackedStatusMessage::from(
            message,
        )];
        td.blocklist_confirmation_failed = true;
        td.manual_failure_cleanup = false;
        td.import_attempted = true;
        return;
    }
    if td.skip_reacquire_on_failure
        && !matches!(outcome, FailureHandlingOutcome::RecordedNoReacquire)
    {
        td.status_messages
            .push(crate::tracked_downloads::TrackedStatusMessage::from(
            "Failure was recorded, but Scryer could not confirm that reacquisition was disabled."
                .to_string(),
        ));
    }
    crate::fail_active_manual_import_for_source(app, td, failure_reason).await;

    td.state = TrackedDownloadState::Failed;
}

pub(crate) fn tracked_download_failure_submission_identity(
    td: &TrackedDownload,
) -> Option<ClientJobLocator> {
    if !tracked_download_has_scryer_failure_origin(td) {
        return None;
    }

    Some(ClientJobLocator::new(
        Some(td.client_id.as_str()),
        &td.client_type,
        &td.client_item.download_client_item_id,
    ))
}

fn tracked_download_has_scryer_failure_origin(td: &TrackedDownload) -> bool {
    td.client_item.is_scryer_origin
}

#[cfg(test)]
pub(crate) async fn tracked_download_has_grabbed_submission(
    app: &AppUseCase,
    td: &TrackedDownload,
) -> bool {
    tracked_download_grabbed_submission_lookup(app, td)
        .await
        .unwrap_or(false)
}

/// Whether Scryer's own grab is on record for this download, keeping a failed
/// read apart from "no grab on record" so a caller that reports the answer to
/// an operator does not state the second when only the first is known.
pub(crate) async fn tracked_download_grabbed_submission_lookup(
    app: &AppUseCase,
    td: &TrackedDownload,
) -> crate::AppResult<bool> {
    let Some(identity) = tracked_download_failure_submission_identity(td) else {
        return Ok(false);
    };

    Ok(app
        .services
        .workflow
        .download_submissions
        .find_by_client_item_id_for_download(td.canonical_download_id(), &identity)
        .await?
        .is_some())
}

pub(crate) fn warn_download_not_grabbed(td: &mut TrackedDownload) {
    td.status_messages.clear();
    td.warn(
        "Download has failed but wasn't grabbed by Scryer. Skipping automatic failure handling.",
    );
}

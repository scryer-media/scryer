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
        return;
    }

    if !tracked_download_has_scryer_failure_origin(td) {
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

    if !tracked_download_has_grabbed_submission(app, td).await {
        warn_download_not_grabbed(td);
        td.state = TrackedDownloadState::Downloading;
        td.skip_reacquire_on_failure = false;
        return;
    }

    let failure_reason = td
        .client_item
        .attention_reason
        .as_deref()
        .unwrap_or("Failed download detected");

    tracing::warn!(
        id = %td.id,
        title_id = ?td.title_id,
        reason = failure_reason,
        "download failed - processing failure"
    );

    let outcome = crate::acquisition_workflow::process_download_failure_for_download(
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
            remove_from_client_if_configured: false,
            skip_reacquire: td.skip_reacquire_on_failure,
        },
    )
    .await;
    if td.skip_reacquire_on_failure
        && !matches!(outcome, FailureHandlingOutcome::RecordedNoReacquire)
    {
        td.status_messages.push(
            "Failure was recorded, but Scryer could not confirm that reacquisition was disabled."
                .to_string(),
        );
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

pub(crate) async fn tracked_download_has_grabbed_submission(
    app: &AppUseCase,
    td: &TrackedDownload,
) -> bool {
    let Some(identity) = tracked_download_failure_submission_identity(td) else {
        return false;
    };

    download_submission_exists_for_download(app, td.canonical_download_id(), &identity).await
}

pub(crate) async fn download_submission_exists_for_download(
    app: &AppUseCase,
    canonical_download_id: Option<&scryer_domain::download_identity::DownloadId>,
    identity: &ClientJobLocator,
) -> bool {
    app.services
        .workflow
        .download_submissions
        .find_by_client_item_id_for_download(canonical_download_id, identity)
        .await
        .ok()
        .flatten()
        .is_some()
}

pub(crate) fn warn_download_not_grabbed(td: &mut TrackedDownload) {
    td.status_messages.clear();
    td.warn(
        "Download has failed but wasn't grabbed by Scryer. Skipping automatic failure handling.",
    );
}

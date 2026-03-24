//! FailedDownloadHandler — failure detection and processing (plan 055).
//!
//! check(): detects downloads that failed in the client or are encrypted.
//! process_failed(): records the failure, emits events, triggers redownload.

use scryer_domain::{DownloadQueueState, TrackedDownloadState, TrackedDownloadStatus};

use crate::tracked_downloads::TrackedDownload;
use crate::{ActivityChannel, ActivityKind, ActivitySeverity, AppUseCase};

/// Detect failed downloads during the poll cycle.
///
/// Called for downloads in Downloading or ImportBlocked state. If the client
/// reports failure (or the download is encrypted), transitions to FailedPending.
pub fn check(td: &mut TrackedDownload) {
    // Only process if in a check-eligible state.
    if td.state != TrackedDownloadState::Downloading
        && td.state != TrackedDownloadState::ImportBlocked
    {
        return;
    }

    if td.client_item.state != DownloadQueueState::Failed {
        return;
    }

    // Check if scryer has context to handle this failure.
    if td.title_id.is_none() || td.title_id.as_deref() == Some("") {
        td.status = TrackedDownloadStatus::Warning;
        td.status_messages.clear();
        td.warn("Download failed but isn't linked to a scryer title. Skipping automatic failure handling.");
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

    td.state = TrackedDownloadState::Failed;

    let failure_reason = td
        .client_item
        .attention_reason
        .as_deref()
        .unwrap_or("Failed download detected");

    tracing::warn!(
        id = %td.id,
        title_id = ?td.title_id,
        reason = failure_reason,
        "download failed — processing failure"
    );

    // Emit activity event.
    let _ = app
        .services
        .record_activity_event(
            None,
            td.title_id.clone(),
            td.facet.clone(),
            ActivityKind::AcquisitionDownloadFailed,
            format!(
                "Download failed: {} — {}",
                td.client_item.title_name, failure_reason
            ),
            ActivitySeverity::Error,
            vec![ActivityChannel::WebUi, ActivityChannel::Toast],
        )
        .await;

    // TODO: trigger re-search if auto-redownload is enabled and title is known.
    // This would call the acquisition search pipeline for the same title/facet/collection.
}

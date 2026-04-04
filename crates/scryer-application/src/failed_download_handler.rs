//! FailedDownloadHandler — failure detection and processing (plan 055).
//!
//! check(): detects downloads that failed in the client or are encrypted.
//! process_failed(): records the failure, emits events, triggers redownload.

use scryer_domain::{
    DomainEventPayload, DownloadFailedEventData, DownloadQueueState, TrackedDownloadState,
    TrackedDownloadStatus,
};

use crate::AppUseCase;
use crate::app_usecase_acquisition::DownloadFailureContext;
use crate::domain_events::{
    new_global_domain_event, new_title_domain_event, title_context_snapshot,
};
use crate::tracked_downloads::TrackedDownload;

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

    let _ = crate::app_usecase_acquisition::process_download_failure(
        app,
        DownloadFailureContext {
            wanted_item: None,
            title_id: td.title_id.clone(),
            client_id: td.client_id.clone(),
            client_item_id: td.client_item.download_client_item_id.clone(),
            release_title: td.client_item.title_name.clone(),
            reason: failure_reason.to_string(),
            remove_from_client_if_configured: false,
        },
        None,
    )
    .await;

    td.state = TrackedDownloadState::Failed;

    let message = format!(
        "Download failed: {} — {}",
        td.client_item.title_name, failure_reason
    );
    if let Some(title_id) = td.title_id.as_deref()
        && let Ok(Some(title)) = app.services.titles.get_by_id(title_id).await
    {
        let _ = app
            .services
            .append_domain_event(new_title_domain_event(
                None,
                &title,
                DomainEventPayload::DownloadFailed(DownloadFailedEventData {
                    title: Some(title_context_snapshot(&title)),
                    source_title: Some(td.client_item.title_name.clone()),
                    source_hint: Some(td.client_item.download_client_item_id.clone()),
                    error_message: Some(message),
                }),
            ))
            .await;
    } else {
        let _ = app
            .services
            .append_domain_event(new_global_domain_event(
                None,
                DomainEventPayload::DownloadFailed(DownloadFailedEventData {
                    title: None,
                    source_title: Some(td.client_item.title_name.clone()),
                    source_hint: Some(td.client_item.download_client_item_id.clone()),
                    error_message: Some(message),
                }),
            ))
            .await;
    }
}

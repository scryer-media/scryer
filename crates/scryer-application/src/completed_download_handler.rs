//! CompletedDownloadHandler — two-phase import bridge (plan 055).
//!
//! Phase 1 (check): validate completed downloads, resolve title, gate auto-import.
//! Phase 2 (import): run the import pipeline, verify completion across passes.

use scryer_domain::{
    DownloadQueueState, TitleMatchType, TrackedDownloadState, TrackedDownloadStatus,
};

use crate::tracked_downloads::TrackedDownload;
use crate::AppUseCase;

/// Phase 1: evaluate a tracked download whose client reports completion.
///
/// Called every poll cycle for downloads in Downloading or ImportBlocked state.
/// Transitions to ImportPending if all validations pass, or ImportBlocked with
/// warnings if auto-import is not safe.
pub fn check(_app: &AppUseCase, td: &mut TrackedDownload) {
    // Only process if client reports completed.
    if td.client_item.state != DownloadQueueState::Completed {
        return;
    }

    // Only process if still in a check-eligible state.
    if td.state != TrackedDownloadState::Downloading
        && td.state != TrackedDownloadState::ImportBlocked
    {
        return;
    }

    // Validate output path.
    // (For now, we trust the client's dest_dir will be available at import time.
    //  A full ValidatePath check would need the CompletedDownload which requires
    //  an async call — handled in the import phase.)

    // Auto-import safety gating.
    match td.match_type {
        TitleMatchType::Unmatched => {
            td.state = TrackedDownloadState::ImportBlocked;
            td.status = TrackedDownloadStatus::Warning;
            if !td.status_messages.iter().any(|m| m.contains("couldn't be matched")) {
                td.status_messages.clear();
                td.warn("Download couldn't be matched to a library title. Assign a title manually or check the download name.");
            }
            return;
        }
        TitleMatchType::IdOnly => {
            // ID-only matches from automated grabs are too risky for auto-import.
            // Interactive searches (user confirmed) are trusted.
            if !td.client_item.is_scryer_origin {
                td.state = TrackedDownloadState::ImportBlocked;
                td.status = TrackedDownloadStatus::Warning;
                if !td.status_messages.iter().any(|m| m.contains("matched by ID only")) {
                    td.status_messages.clear();
                    td.warn("Download was matched to a title by ID only. Manual confirmation required to import.");
                }
                return;
            }
        }
        TitleMatchType::Submission | TitleMatchType::ClientParameter | TitleMatchType::TitleParse => {
            // High-confidence matches — proceed.
        }
    }

    // Check that the resolved title still exists.
    // (This is a sync check against cached data; the actual title lookup
    //  was done during resolve_title. If the title was deleted since then,
    //  title_id will still be set but import will fail gracefully.)

    if td.title_id.is_none() || td.title_id.as_deref() == Some("") {
        td.state = TrackedDownloadState::ImportBlocked;
        td.warn("No title linked to this download.");
        return;
    }

    // All checks passed — queue for import.
    td.state = TrackedDownloadState::ImportPending;
    td.status = TrackedDownloadStatus::Ok;
    td.status_messages.clear();
}

/// Phase 2: run the actual import for a download in ImportPending state.
///
/// This is async because it calls the import pipeline. Returns true if the
/// download transitioned to a terminal state (Imported or ImportBlocked).
pub async fn import(
    _app: &AppUseCase,
    td: &mut TrackedDownload,
) -> bool {
    if td.state != TrackedDownloadState::ImportPending {
        return false;
    }

    td.state = TrackedDownloadState::Importing;
    td.status = TrackedDownloadStatus::Ok;
    td.status_messages.clear();

    // Delegate to the existing import pipeline via try_import_completed_downloads.
    // The actual file import logic lives in app_usecase_import.rs — we call it
    // through the existing path and inspect the result.
    //
    // For now, we set ImportPending and let the existing poller logic handle it.
    // The full integration (calling run_import directly, writing artifacts,
    // calling verify_import) will be wired in the poller rewrite.
    //
    // TODO: wire direct import call + artifact recording + verify_import
    td.state = TrackedDownloadState::ImportPending;
    false
}

/// Verify whether a download's import is complete by checking cumulative
/// artifact history across all passes.
///
/// Returns true if all expected files are accounted for (imported or already_present).
pub async fn verify_import(
    app: &AppUseCase,
    td: &TrackedDownload,
    files_imported_this_pass: usize,
) -> bool {
    let source_ref = &td.client_item.download_client_item_id;

    // Count total successful artifacts across all passes.
    let imported_count = app
        .services
        .import_artifacts
        .count_by_result(&td.client_type, source_ref, "imported")
        .await
        .unwrap_or(0);

    let already_present_count = app
        .services
        .import_artifacts
        .count_by_result(&td.client_type, source_ref, "already_present")
        .await
        .unwrap_or(0);

    let total_success = imported_count + already_present_count;

    // For movies: 1 file expected.
    // For episodes: we don't yet know how many episodes are expected from the
    // download metadata alone. Use the heuristic: if at least 1 file was
    // imported in this pass AND the union count > 0, consider it done.
    //
    // A more precise check would compare against the expected episode count
    // from the download's parsed metadata, but that requires deeper integration
    // with the release parser. For now, this covers the common cases.
    if files_imported_this_pass > 0 && total_success > 0 {
        return true;
    }

    // If nothing was imported this pass but prior passes succeeded, still complete.
    if files_imported_this_pass == 0 && total_success > 0 {
        // Cross-check: were there any rejected files this pass that should block?
        let rejected_count = app
            .services
            .import_artifacts
            .count_by_result(&td.client_type, source_ref, "rejected")
            .await
            .unwrap_or(0);

        // If we have some successes and no new rejections, consider complete.
        if rejected_count == 0 {
            return true;
        }
    }

    false
}

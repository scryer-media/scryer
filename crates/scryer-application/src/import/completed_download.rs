//! CompletedDownloadHandler — two-phase import bridge.
//!
//! Phase 1 (check): validate completed downloads, resolve title, gate auto-import.
//! Phase 2 (import): run the import pipeline, verify completion across passes.

use chrono::{DateTime, Duration, Utc};
use scryer_domain::{
    CompletedDownload, DownloadQueueItem, DownloadQueueState, ImportDecision,
    ImportRejectedEventData, ImportResult, ImportSkipReason, ImportStatus, TitleMatchType,
    TrackedDownloadState, TrackedDownloadStatus,
};
use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::domain_events::{
    new_global_domain_event, new_title_domain_event, title_context_snapshot,
};
use crate::import_workflow::{
    ResolvedCompletedDownloadOriginForImport,
    block_completed_download_origin_conflict_for_manual_review,
    completed_import_result_is_retryable, import_completed_download,
    resolve_completed_download_origin_for_import,
};
use crate::stored_paths::path_to_stored_string;
use crate::tracked_downloads::{NoVideoImportSourceSignature, TrackedDownload};
use crate::{AppResult, AppUseCase, DownloadSourceIdentity, DownloadSubmissionActorSnapshot, User};
use crate::{
    apply_remote_path_mappings_to_completed_download, parse_download_client_remote_path_mappings,
};

mod check;
mod execute;
mod lookup;
mod path_state;
mod result_state;
mod verification;

pub use check::check;
pub(crate) use check::check_with_lookup;
pub use execute::import;
pub(crate) use execute::{import_with_lookup, mark_importing};
#[cfg(test)]
pub(crate) use lookup::load_completed_download_lookup_for_items;
pub(crate) use lookup::{
    CompletedDownloadLookup, load_completed_download_lookup_for_items_excluding_client_types,
    load_completed_download_lookup_for_tracked_client_items_excluding_client_types,
};
pub use verification::verify_import;

#[cfg(test)]
mod tests;

const PATH_WAITING_MESSAGE: &str =
    "Completed download path is not available yet. Retrying for up to 10 minutes.";
const PATH_BLOCKED_MESSAGE: &str = "Completed download path is still unavailable. Check remote path mappings, volume mounts, or download paths, then retry manually.";
const PATH_BLOCKED_NZBDAV_SYMLINK_MESSAGE: &str = "Completed download path is still unavailable. Check remote path mappings, confirm the NZBDAV completed-symlinks mount is visible to Scryer, and make sure the rclone mount was started with --links before retrying manually.";
const PATH_URL_UNSUPPORTED_MESSAGE: &str = "Completed download path is a URL, not a local filesystem path. Mount it locally or use remote path mappings before retrying.";
const ID_ONLY_CONFLICT_MESSAGE: &str = "Download name conflicts with the current ID-only title match. Manual confirmation required before import.";
const UNMANAGED_CATEGORY_BLOCKED_MESSAGE: &str = "Download wasn't grabbed by Scryer and is not in a Scryer download category. Manual confirmation required before import.";
const NO_VIDEO_FIRST_RETRY_DELAY_SECS: i64 = 30;
const NO_VIDEO_SECOND_RETRY_DELAY_SECS: i64 = 120;
const NO_VIDEO_BLOCK_AFTER_UNCHANGED_ATTEMPTS: u8 = 3;
const IMPORT_RUNNING_MESSAGE: &str = "Moving files to library.";
const COMPLETED_PATH_GRACE_PERIOD_MINUTES: i64 = 10;

pub(crate) async fn load_completed_download_lookup(
    app: &AppUseCase,
) -> AppResult<CompletedDownloadLookup> {
    lookup::load_completed_download_lookup(app).await
}

/// Durable tracked-state marker recorded for a queue item's download
/// identity, if any. Reconciliation sweeps use this to skip items whose
/// identity already reached a terminal or operator-blocked outcome.
pub(crate) async fn queue_item_identity_tracked_state(
    app: &AppUseCase,
    item: &DownloadQueueItem,
) -> Option<TrackedDownloadState> {
    let identity = lookup::observed_queue_item_identity(item);
    let source_identity = lookup::queue_item_source_identity(item);
    lookup::download_id_tracked_state(app, &identity, Some(&source_identity)).await
}

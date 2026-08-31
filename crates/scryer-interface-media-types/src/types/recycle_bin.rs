use super::{JobRunPayload, Long};
use async_graphql::{Enum, InputObject, SimpleObject};
use chrono::{DateTime, Utc};

// ── Recycle Bin ────────────────────────────────────────────────────────────

#[derive(SimpleObject, Clone)]
/// File moved to the recycle bin with its original library context.
pub struct RecycledItemPayload {
    /// Recycle-bin entry ID.
    pub id: async_graphql::ID,
    /// Original absolute or library-relative file path.
    pub original_path: String,
    /// Original file name.
    pub file_name: String,
    /// File size in bytes.
    pub size_bytes: Long,
    /// Associated title ID, or null when no title was matched.
    pub title_id: Option<async_graphql::ID>,
    /// Associated title name, or null when no title was matched.
    pub title_name: Option<String>,
    /// Reason the file entered the recycle bin.
    pub reason: String,
    /// Time the file entered the recycle bin in UTC.
    pub recycled_at: DateTime<Utc>,
    /// Time the retention policy schedules this file for permanent deletion in UTC.
    pub scheduled_deletion_at: DateTime<Utc>,
    /// Media root containing the original file.
    pub media_root: String,
    /// Library ID containing the original file.
    pub library_id: async_graphql::ID,
    /// Library name containing the original file.
    pub library_name: String,
}

#[derive(SimpleObject, Clone)]
/// A page of recycle-bin entries and its total matching count.
pub struct RecycledItemsPayload {
    /// Recycle-bin entries in the requested page.
    pub items: Vec<RecycledItemPayload>,
    /// Total matching entries across all pages.
    pub total_count: i32,
}

#[derive(SimpleObject, Clone)]
/// Accepted request to restore one recycle-bin entry.
pub struct RestoreRecycledItemPayload {
    /// Recycle-bin entry ID.
    pub id: async_graphql::ID,
    /// Background job accepted for the restore operation.
    pub job_run: JobRunPayload,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Behavior when a restored file already occupies its destination path.
pub enum RecycleRestoreConflictPolicyValue {
    /// Preserve both files using a distinct destination name.
    KeepBoth,
    /// Replace the existing destination file.
    ReplaceExisting,
}

#[derive(InputObject)]
/// Recycle-bin entries and the preview version to restore.
pub struct RestoreRecycledItemsInput {
    /// Recycle-bin entry IDs to restore.
    pub ids: Vec<async_graphql::ID>,
    /// Conflict behavior for occupied destination paths.
    pub conflict_policy: RecycleRestoreConflictPolicyValue,
    /// Fingerprint returned by the current restore preview.
    pub preview_fingerprint: String,
}

#[derive(InputObject)]
/// Recycle-bin entries to delete permanently.
pub struct DeleteRecycledItemsInput {
    /// Recycle-bin entry IDs to delete.
    pub ids: Vec<async_graphql::ID>,
}

#[derive(SimpleObject, Clone)]
/// One destination collision found during restore preview.
pub struct RecycleRestorePreviewItemPayload {
    /// Recycle-bin entry ID.
    pub id: async_graphql::ID,
    /// Original path that would be restored.
    pub original_path: String,
    /// Whether the destination is currently occupied.
    pub destination_occupied: bool,
}

#[derive(SimpleObject, Clone)]
/// Restore preview fingerprint and destination collision details.
pub struct RecycleRestorePreviewPayload {
    /// Fingerprint required to confirm the current preview.
    pub fingerprint: String,
    /// Preview entries for the selected recycle-bin records.
    pub items: Vec<RecycleRestorePreviewItemPayload>,
}

#[derive(SimpleObject, Clone)]
/// Accepted background restore for multiple recycle-bin entries.
pub struct RestoreRecycledItemsPayload {
    /// Recycle-bin entry IDs accepted for restore.
    pub ids: Vec<async_graphql::ID>,
    /// Background job accepted for the restore operation.
    pub job_run: JobRunPayload,
}

#[derive(SimpleObject, Clone)]
/// Accepted background deletion for multiple recycle-bin entries.
pub struct DeleteRecycledItemsPayload {
    /// Recycle-bin entry IDs accepted for deletion.
    pub ids: Vec<async_graphql::ID>,
    /// Background job accepted for the deletion operation.
    pub job_run: JobRunPayload,
}

#[derive(SimpleObject, Clone)]
/// Result of deleting one recycle-bin entry.
pub struct DeleteRecycledItemPayload {
    /// Recycle-bin entry ID.
    pub id: async_graphql::ID,
    /// Whether the recycle-bin record and file were deleted.
    pub deleted: bool,
}

#[derive(SimpleObject, Clone)]
/// Count of files permanently purged from the recycle bin.
pub struct EmptyRecycleBinPayload {
    /// Number of recycle-bin entries purged.
    pub purged_count: i32,
}

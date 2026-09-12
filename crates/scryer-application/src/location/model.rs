//! Core types shared by every location operation.
//!
//! These are the persisted shapes behind `location_operations` (operation row,
//! per-title checkpoints, per-file verification records) plus the small value
//! enums the preview, executor, and GraphQL surfaces all speak.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Which location workflow an operation belongs to.
///
/// Spec "Key Entities" — Location operation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum LocationOperationType {
    /// Correct which folder a title owns; never touches file content (US1).
    FolderReassignment,
    /// Move selected titles to another root inside the same library (US2).
    RootMove,
    /// Replace one root's path with a new, unconfigured path (US4).
    RootChange,
    /// Fold one root's managed contents into another root in the same library (US5).
    RootConsolidation,
    /// Move titles into a different library, with or without a merge (US6/US7).
    CrossLibraryTransfer,
    /// Adopt content the user already moved outside Scryer (US3).
    Adoption,
}

impl LocationOperationType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::FolderReassignment => "folder_reassignment",
            Self::RootMove => "root_move",
            Self::RootChange => "root_change",
            Self::RootConsolidation => "root_consolidation",
            Self::CrossLibraryTransfer => "cross_library_transfer",
            Self::Adoption => "adoption",
        }
    }

    /// Parse a persisted value. Unknown values are rejected: an operation whose
    /// type cannot be read must not be run as some other type.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "folder_reassignment" => Some(Self::FolderReassignment),
            "root_move" => Some(Self::RootMove),
            "root_change" => Some(Self::RootChange),
            "root_consolidation" => Some(Self::RootConsolidation),
            "cross_library_transfer" => Some(Self::CrossLibraryTransfer),
            "adoption" => Some(Self::Adoption),
            _ => None,
        }
    }

    /// Whether this operation type is root-wide and therefore requires the
    /// stronger typed confirmation (FR-029).
    pub fn requires_typed_confirmation(&self) -> bool {
        matches!(self, Self::RootChange | Self::RootConsolidation)
    }
}

/// How the filesystem side of an operation is performed.
///
/// Spec "Product Language": **Move with Scryer** vs **Files are already there**.
/// Catalog-only reassignments (FR-076) never present a mode choice.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum LocationExecutionMode {
    /// Scryer performs and verifies the filesystem operation.
    MoveWithScryer,
    /// The user already moved the files; Scryer verifies and adopts them.
    FilesAlreadyThere,
    /// Trust the user's external move and update catalog mappings only.
    UserMovedFiles,
    /// No filesystem work at all: fileless titles (FR-076) and folder-match
    /// correction (FR-014).
    CatalogOnly,
}

impl LocationExecutionMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::MoveWithScryer => "move_with_scryer",
            Self::FilesAlreadyThere => "files_already_there",
            Self::UserMovedFiles => "user_moved_files",
            Self::CatalogOnly => "catalog_only",
        }
    }

    /// Parse a persisted value. Unknown values are rejected rather than falling
    /// back to a mode that would touch files.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "move_with_scryer" => Some(Self::MoveWithScryer),
            "files_already_there" => Some(Self::FilesAlreadyThere),
            "user_moved_files" => Some(Self::UserMovedFiles),
            "catalog_only" => Some(Self::CatalogOnly),
            _ => None,
        }
    }

    /// Whether this mode can copy bytes and therefore needs verification.
    pub fn moves_files(&self) -> bool {
        matches!(self, Self::MoveWithScryer)
    }
}

/// Operation lifecycle states surfaced in Activity (FR-091, US8 scenario 1).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum LocationOperationState {
    /// Accepted and persisted; not yet started.
    Queued,
    /// Validating paths, ownership, permissions, and free space.
    Preparing,
    /// Renaming or copying title content.
    Moving,
    /// Verifying destination content at the applicable depth.
    Verifying,
    /// Applying catalog changes: ownership flips, merges, role resolution.
    Reconciling,
    /// Recycling redundant sources and removing empty source directories.
    CleaningUp,
    /// Finished with every item as previewed.
    Completed,
    /// Finished, but with warnings the user must see (FR-073 preserve paths,
    /// unmanaged content, hardlink notes).
    CompletedWithWarnings,
    /// Stopped at a safe title checkpoint on user request (FR-092).
    Canceled,
    /// Stopped on an error; completed titles remain consistent.
    Failed,
}

impl LocationOperationState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Preparing => "preparing",
            Self::Moving => "moving",
            Self::Verifying => "verifying",
            Self::Reconciling => "reconciling",
            Self::CleaningUp => "cleaning_up",
            Self::Completed => "completed",
            Self::CompletedWithWarnings => "completed_with_warnings",
            Self::Canceled => "canceled",
            Self::Failed => "failed",
        }
    }

    /// Parse a persisted value. Unknown values are rejected: a state that cannot
    /// be read must not be resumed as if it were `queued`.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "queued" => Some(Self::Queued),
            "preparing" => Some(Self::Preparing),
            "moving" => Some(Self::Moving),
            "verifying" => Some(Self::Verifying),
            "reconciling" => Some(Self::Reconciling),
            "cleaning_up" => Some(Self::CleaningUp),
            "completed" => Some(Self::Completed),
            "completed_with_warnings" => Some(Self::CompletedWithWarnings),
            "canceled" => Some(Self::Canceled),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }

    /// Terminal states hold no operation ownership and never resume.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::CompletedWithWarnings | Self::Canceled | Self::Failed
        )
    }

    /// Non-terminal states keep their (title, root) ownership claims alive and
    /// are eligible for restart resume (FR-033).
    pub fn is_active(&self) -> bool {
        !self.is_terminal()
    }
}

/// Machine-readable reason an operation, or one title inside it, stopped
/// short of its plan.
///
/// `detail` keeps the raw cause (a database error verbatim, the path that
/// failed verification); the code is what the Activity panel keys its guidance
/// on, so the user is told which way out applies — retry the same operation,
/// plan again, or fix the storage first — without anyone parsing English.
///
/// A code is written wherever a `Failed` or `Canceled` state is, on the
/// operation row and on the title checkpoint. Persisted as its snake_case
/// string; a stored value this build does not know reads back as `None`
/// rather than refusing the row, so a downgrade still lists the operation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum LocationReasonCode {
    /// Another operation still owns some of the titles or roots (FR-084).
    /// Retry once it has finished.
    OwnershipConflict,
    /// A file could not be read or written; the source is intact. Fix the
    /// storage, then retry.
    StorageError,
    /// A copied file did not match its source; the copy is not trusted and
    /// the source is untouched. Retry.
    VerificationMismatch,
    /// Something changed under the plan after the preview was taken (FR-089).
    /// Plan again.
    StalePlan,
    /// The title can no longer merge into its destination. Fix the destination
    /// title, then plan again.
    MergeUnavailable,
    /// The files were copied and verified, but the catalog update failed. Retry
    /// the same operation; never plan again for this title.
    CatalogWriteFailed,
    /// The move finished but the old copy could not be removed. Retry re-runs
    /// the cleanup step.
    SourceCleanupFailed,
    /// An accepted operation had no runner and was failed so the queue moved.
    Stranded,
    /// The user canceled at a safe checkpoint (FR-092). Retry moves the rest.
    Canceled,
    /// The user abandoned a stalled operation. Retry once the storage is back.
    Abandoned,
    /// A root the plan needs is not mounted; the operation is left interrupted.
    /// Resume once it is back, or abandon.
    RootUnavailable,
}

impl LocationReasonCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OwnershipConflict => "ownership_conflict",
            Self::StorageError => "storage_error",
            Self::VerificationMismatch => "verification_mismatch",
            Self::StalePlan => "stale_plan",
            Self::MergeUnavailable => "merge_unavailable",
            Self::CatalogWriteFailed => "catalog_write_failed",
            Self::SourceCleanupFailed => "source_cleanup_failed",
            Self::Stranded => "stranded",
            Self::Canceled => "canceled",
            Self::Abandoned => "abandoned",
            Self::RootUnavailable => "root_unavailable",
        }
    }

    /// Parse a persisted value; unknown values read back as `None`.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "ownership_conflict" => Some(Self::OwnershipConflict),
            "storage_error" => Some(Self::StorageError),
            "verification_mismatch" => Some(Self::VerificationMismatch),
            "stale_plan" => Some(Self::StalePlan),
            "merge_unavailable" => Some(Self::MergeUnavailable),
            "catalog_write_failed" => Some(Self::CatalogWriteFailed),
            "source_cleanup_failed" => Some(Self::SourceCleanupFailed),
            "stranded" => Some(Self::Stranded),
            "canceled" => Some(Self::Canceled),
            "abandoned" => Some(Self::Abandoned),
            "root_unavailable" => Some(Self::RootUnavailable),
            _ => None,
        }
    }

    /// Which of several failed titles' codes the operation row should carry.
    ///
    /// Guidance is per operation, so the code that changes what the user must
    /// do wins: a catalog failure forbids planning again, a stale plan demands
    /// it, and everything else says retry.
    pub fn most_consequential(codes: impl IntoIterator<Item = Self>) -> Option<Self> {
        codes.into_iter().max_by_key(|code| code.consequence_rank())
    }

    fn consequence_rank(self) -> u8 {
        match self {
            Self::CatalogWriteFailed => 7,
            Self::MergeUnavailable => 6,
            Self::StalePlan => 5,
            Self::SourceCleanupFailed => 4,
            Self::VerificationMismatch => 3,
            Self::StorageError => 2,
            Self::OwnershipConflict
            | Self::Stranded
            | Self::Canceled
            | Self::Abandoned
            | Self::RootUnavailable => 1,
        }
    }
}

// The verification value types live in [`scryer_domain`]: the import worker
// serializes them across a process boundary, so they cannot depend on anything
// here. Re-exported under their established paths — this module is still where
// the operation model names them.
pub use scryer_domain::{
    AppliedVerificationDepth, FileVerificationOutcome, MoveCrcAlgorithm, StreamedContentHashes,
    VerificationDepth,
};

/// The 0205 columns as they read back off a media file row.
///
/// `None` on a media file means the row has never been hashed end to end, or a
/// scan saw its sampled proof change and invalidated it (FR-046). The CRC is
/// only interpretable next to its algorithm tag, so both travel together and a
/// row missing the tag reads back without a CRC rather than with an
/// unattributable one.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersistedContentHashes {
    /// Full-file BLAKE3 (hex).
    pub full_blake3: String,
    /// Streaming CRC and its algorithm tag, when one was persisted. Absent for
    /// a hash the backfill job produced before a CRC existed for that row, or
    /// for a row whose stored tag is not one this build understands.
    pub move_crc: Option<u64>,
    pub crc_algorithm: Option<MoveCrcAlgorithm>,
    /// When the hash was computed. Its absence is what separates a hash whose
    /// vintage can be attested from one that cannot (see
    /// [`crate::location::collisions::FullHash`]).
    pub hash_computed_at: Option<DateTime<Utc>>,
}

impl PersistedContentHashes {
    /// The write side: what one streaming pass produced, stamped with when.
    pub fn from_streamed(hashes: &StreamedContentHashes, computed_at: DateTime<Utc>) -> Self {
        Self {
            full_blake3: hashes.full_blake3.clone(),
            move_crc: Some(hashes.move_crc),
            crc_algorithm: Some(hashes.crc_algorithm),
            hash_computed_at: Some(computed_at),
        }
    }
}

/// What the catalog already knows about a source file's bytes, carried on the
/// plan so the transfer can prove an occupied destination without reading the
/// source a second time.
///
/// The stored BLAKE3 was streamed the last time the file was written or hashed
/// end to end (FR-041/046). It is trusted only while the file's signature —
/// its size and the platform's mtime scheme, the same facts a scan uses to
/// invalidate stored hashes — still matches on disk at comparison time. A file
/// touched since is compared the slow way, both sides in full.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KnownSourceContent {
    /// Full-file BLAKE3 (hex) of the source as last recorded.
    pub full_blake3: String,
    pub size_bytes: u64,
    /// The source signature the hash was recorded against (see
    /// [`crate::file_source_signature`]).
    pub signature_scheme: String,
    pub signature_value: String,
}

impl KnownSourceContent {
    /// Read the stored facts off a media file row. `None` unless the row holds
    /// an attested full hash and a signature in this platform's scheme.
    pub fn from_media_file(media_file: &crate::TitleMediaFile) -> Option<Self> {
        let full_blake3 = crate::location::collisions::FullHash::from_persisted(
            media_file.content_hashes.as_ref(),
        )
        .as_known()?
        .to_string();
        let scheme = media_file.source_signature_scheme.as_deref()?;
        let value = media_file.source_signature_value.as_deref()?;
        if scheme != crate::file_source_signature::MEDIA_FILE_SOURCE_SIGNATURE_SCHEME
            || value.trim().is_empty()
        {
            return None;
        }
        Some(Self {
            full_blake3,
            size_bytes: u64::try_from(media_file.size_bytes).ok()?,
            signature_scheme: scheme.to_string(),
            signature_value: value.to_string(),
        })
    }

    /// Whether the file on disk is still the one the stored hash describes.
    pub fn matches(&self, metadata: &std::fs::Metadata) -> bool {
        metadata.len() == self.size_bytes
            && crate::file_source_signature::file_source_signature_from_metadata(metadata)
                .ok()
                .is_some_and(|signature| {
                    signature.scheme == self.signature_scheme
                        && signature.value == self.signature_value
                })
    }
}

/// Per-file verification record persisted for the operation (D5) and surfaced in
/// Activity as "verified (full)" / "verified (quick)" (FR-043).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileVerificationRecord {
    /// Owning operation.
    pub operation_id: String,
    /// Title whose checkpoint this file belongs to.
    pub title_id: String,
    /// Media file record, when the file is tracked media rather than a companion
    /// asset.
    pub media_file_id: Option<String>,
    /// Stored-path string of the source file.
    pub source_path: String,
    /// Stored-path string of the destination file.
    pub destination_path: String,
    /// Hashes computed during the copy; absent for same-filesystem renames,
    /// which need no verification pass (FR-032).
    pub hashes: Option<StreamedContentHashes>,
    /// Requested vs applied depth and whether the quick floor was a fallback.
    pub depth: AppliedVerificationDepth,
    /// Verification result.
    pub outcome: FileVerificationOutcome,
    /// Human-readable explanation for a non-verified outcome or a fallback.
    pub detail: Option<String>,
    pub verified_at: DateTime<Utc>,
}

/// Progress of one title inside an operation. Titles are the safe-cancel and
/// resume granularity (FR-092, FR-089).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TitleCheckpointState {
    /// Planned but not started; still eligible for a staleness check.
    Pending,
    /// Content is being renamed or copied.
    Moving,
    /// Destination content is being proven at the applicable depth.
    Verifying,
    /// Destination verified; catalog ownership flip and merge unions are running.
    Reconciling,
    /// Sources recycled or removed and empty source directories cleaned up.
    CleaningUp,
    /// Title finished exactly as previewed.
    Completed,
    /// Title finished, but the user must see something (dedup, collision rename,
    /// preserve-instead-of-recycle).
    CompletedWithWarnings,
    /// Deliberately not processed: no-op titles, and titles the user removed from
    /// the selection.
    Skipped,
    /// Could not enter the operation: active download/import (FR-086), unresolved
    /// classification (FR-016), or unmapped merge records (FR-066).
    Blocked,
    /// Processing failed; the source is intact.
    Failed,
}

impl TitleCheckpointState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Moving => "moving",
            Self::Verifying => "verifying",
            Self::Reconciling => "reconciling",
            Self::CleaningUp => "cleaning_up",
            Self::Completed => "completed",
            Self::CompletedWithWarnings => "completed_with_warnings",
            Self::Skipped => "skipped",
            Self::Blocked => "blocked",
            Self::Failed => "failed",
        }
    }

    /// Parse a persisted value. Unknown values are rejected so a checkpoint that
    /// cannot be read is never mistaken for unstarted work.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "pending" => Some(Self::Pending),
            "moving" => Some(Self::Moving),
            "verifying" => Some(Self::Verifying),
            "reconciling" => Some(Self::Reconciling),
            "cleaning_up" => Some(Self::CleaningUp),
            "completed" => Some(Self::Completed),
            "completed_with_warnings" => Some(Self::CompletedWithWarnings),
            "skipped" => Some(Self::Skipped),
            "blocked" => Some(Self::Blocked),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }

    /// A finished title is never reprocessed on resume (FR-092: "retry/resume
    /// never repeats verified work").
    pub fn is_settled(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::CompletedWithWarnings | Self::Skipped | Self::Blocked
        )
    }
}

/// Where one title's content sits on each side of the operation, as previewed.
///
/// Carried on the checkpoint so a resumed operation can explain what it was
/// doing without re-deriving the plan, and so Activity's per-title expansion has
/// the source and destination it needs (FR-091).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TitleCheckpointPlacement {
    pub source_library_id: Option<String>,
    pub source_root_id: Option<String>,
    pub source_folder_path: Option<String>,
    pub destination_library_id: Option<String>,
    pub destination_root_id: Option<String>,
    pub destination_folder_path: Option<String>,
    /// Set when this title merges into an existing destination title (D8, US7).
    pub merged_into_title_id: Option<String>,
    /// Name of that surviving title, resolved from the catalog when the
    /// checkpoint is read — **not** a persisted column.
    ///
    /// 0217 has no column for it and this campaign's migrations are immutable,
    /// so the store resolves it in the same `SELECT` that reads the checkpoint
    /// (one `LEFT JOIN titles`, no extra round-trip). The writer never sets it:
    /// [`crate::location::root_move::RootMoveTitleExecution::placement`] leaves
    /// it `None` and the upsert has no parameter for it.
    ///
    /// Consequences of resolving live rather than freezing the name: a
    /// destination renamed after the merge reads out under its *current* name,
    /// and one deleted afterwards reads out as `None` so the row falls back to
    /// the id it already showed. Neither case shows less than the id-only row
    /// did, and the current name is what a user chasing "where did my title
    /// go?" actually needs. Freezing the name at execution time would take a
    /// new column, so it is a follow-up, not a workaround.
    #[serde(default)]
    pub merged_into_title_name: Option<String>,
}

/// Per-title checkpoint row: the unit resume restarts from.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TitleCheckpoint {
    pub operation_id: String,
    pub title_id: String,
    /// Position of this title in the confirmed plan; resume walks in this order.
    pub sequence: i64,
    pub state: TitleCheckpointState,
    /// The class this title was previewed as (FR-015); `None` for workflows that
    /// do not classify per title.
    pub classification: Option<crate::location::classify::TitleLocationClass>,
    /// Source and destination placement as previewed.
    pub placement: TitleCheckpointPlacement,
    /// Files planned for this title.
    pub files_total: i64,
    /// Files whose destination is verified.
    pub files_verified: i64,
    /// Bytes planned for this title.
    pub bytes_total: i64,
    /// Bytes whose destination is verified.
    pub bytes_verified: i64,
    /// Warning or failure explanation shown in the per-title Activity expansion.
    pub detail: Option<String>,
    /// Why the title failed, when it did; keys the panel's guidance.
    pub reason_code: Option<LocationReasonCode>,
    /// When this title entered the operation. Checkpoint rows are written when a
    /// title starts, so this is the row's `created_at` in 0206.
    pub started_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    /// When the title settled; 0206's `checkpointed_at`.
    pub completed_at: Option<DateTime<Utc>>,
}

/// Aggregate counters shown in Activity (US8 scenario 1).
///
/// # Persistence coverage
///
/// Every field here has a 0206 column: the volume counters (`title_total`,
/// `title_completed_count`, `title_blocked_count`, `file_total`,
/// `file_completed_count`, `bytes_total`, `bytes_completed`) and the outcome
/// counters (`merge_count`, `dedup_count`, `rename_count`, `no_op_count`,
/// `unresolved_count`). The whole struct round-trips, so Activity reads the
/// counters off the operation row rather than re-deriving them from checkpoints.
///
/// The runner recomputes all of them from the confirmed plan and the settled
/// checkpoints on every progress write (see
/// [`crate::location::executor::OperationWorkPlan`]), so they are a snapshot of
/// durable facts rather than a running total a resume could double-count.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct LocationOperationCounters {
    pub titles_total: i64,
    pub titles_processed: i64,
    /// Titles that could not enter the operation (FR-086, FR-016).
    pub titles_blocked: i64,
    pub files_total: i64,
    pub files_processed: i64,
    pub bytes_total: i64,
    pub bytes_processed: i64,
    /// Titles merged into an existing destination title (US7).
    pub merges: i64,
    /// Files or companion assets recycled as proven duplicates (FR-073).
    pub dedups: i64,
    /// Files or companion assets renamed to avoid a collision (FR-074/075).
    pub renames: i64,
    /// Titles that needed no change.
    pub no_ops: i64,
    /// Items still needing a user decision.
    pub unresolved: i64,
}

/// The persisted operation row (D5).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocationOperation {
    pub id: String,
    pub operation_type: LocationOperationType,
    pub mode: LocationExecutionMode,
    pub state: LocationOperationState,
    /// User who confirmed the operation; shown in Activity (FR-091). `None` once
    /// that user is deleted (0206's `ON DELETE SET NULL`), because the operation
    /// record outlives the actor.
    pub initiated_by_user_id: Option<String>,
    /// Source library, when the operation is scoped to one.
    pub source_library_id: Option<String>,
    /// Destination library; differs from the source only for transfers.
    pub destination_library_id: Option<String>,
    /// Source root synthetic id (FR-078), for root-scoped operations.
    pub source_root_id: Option<String>,
    /// Destination root synthetic id.
    pub destination_root_id: Option<String>,
    /// Fingerprint of the full confirmed plan; a mismatch voids the confirmation
    /// (FR-081, FR-089).
    pub plan_fingerprint: String,
    /// Depth the user's preference asked for at confirmation time; the preview
    /// stated it and per-file records stamp what was actually achieved (FR-043).
    pub verification_depth: VerificationDepth,
    /// Files that could only be proven at the quick floor, so the weaker
    /// guarantee is visible on the operation itself (FR-042/043).
    pub verification_fallback_count: i64,
    pub counters: LocationOperationCounters,
    /// Concise failure or warning explanation for Activity.
    pub detail: Option<String>,
    /// Why the operation stopped short, when it did; keys the panel's guidance
    /// (retry, plan again, or fix the storage first).
    pub reason_code: Option<LocationReasonCode>,
    /// The Activity job run this operation reports through, when it has one.
    pub job_run_id: Option<String>,
    /// The workflow-operation row this operation reports through, when it has
    /// one.
    pub workflow_operation_id: Option<String>,
    /// A cancel was requested; the runner stops at the next title checkpoint
    /// (FR-092). Persisted so a cancel survives a restart.
    pub cancel_requested: bool,
    pub cancel_requested_at: Option<DateTime<Utc>>,
    /// When the user confirmed the fingerprinted plan (FR-081).
    pub confirmed_at: Option<DateTime<Utc>>,
    /// When the runner first left `queued`.
    pub started_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verification_depth_defaults_to_full_and_round_trips() {
        assert_eq!(VerificationDepth::default(), VerificationDepth::Full);
        for depth in [VerificationDepth::Full, VerificationDepth::Quick] {
            assert_eq!(VerificationDepth::from_setting(depth.as_str()), Ok(depth));
        }
        assert!(VerificationDepth::from_setting("none").is_err());
    }

    #[test]
    fn only_finished_operation_states_are_terminal() {
        for state in [
            LocationOperationState::Completed,
            LocationOperationState::CompletedWithWarnings,
            LocationOperationState::Canceled,
            LocationOperationState::Failed,
        ] {
            assert!(state.is_terminal(), "{} should be terminal", state.as_str());
        }
        for state in [
            LocationOperationState::Queued,
            LocationOperationState::Preparing,
            LocationOperationState::Moving,
            LocationOperationState::Verifying,
            LocationOperationState::Reconciling,
            LocationOperationState::CleaningUp,
        ] {
            assert!(state.is_active(), "{} should be active", state.as_str());
        }
    }

    #[test]
    fn resume_never_reprocesses_a_settled_title() {
        for state in [
            TitleCheckpointState::Completed,
            TitleCheckpointState::CompletedWithWarnings,
            TitleCheckpointState::Skipped,
            TitleCheckpointState::Blocked,
        ] {
            assert!(state.is_settled(), "{} should be settled", state.as_str());
        }
        for state in [
            TitleCheckpointState::Pending,
            TitleCheckpointState::Moving,
            TitleCheckpointState::Verifying,
            TitleCheckpointState::Reconciling,
            TitleCheckpointState::CleaningUp,
            TitleCheckpointState::Failed,
        ] {
            assert!(
                !state.is_settled(),
                "{} should be resumable",
                state.as_str()
            );
        }
    }

    #[test]
    fn only_verified_files_unblock_source_removal() {
        assert!(FileVerificationOutcome::Verified.permits_source_removal());
        assert!(!FileVerificationOutcome::Mismatch.permits_source_removal());
        assert!(!FileVerificationOutcome::Unavailable.permits_source_removal());
    }

    #[test]
    fn quick_fallback_records_the_reduced_guarantee() {
        let fallback = AppliedVerificationDepth::quick_fallback();
        assert_eq!(fallback.requested, VerificationDepth::Full);
        assert_eq!(fallback.applied, VerificationDepth::Quick);
        assert!(fallback.fell_back);

        let exact = AppliedVerificationDepth::exact(VerificationDepth::Quick);
        assert!(!exact.fell_back);
    }

    #[test]
    fn only_root_wide_operations_require_typed_confirmation() {
        assert!(LocationOperationType::RootChange.requires_typed_confirmation());
        assert!(LocationOperationType::RootConsolidation.requires_typed_confirmation());
        assert!(!LocationOperationType::RootMove.requires_typed_confirmation());
        assert!(!LocationOperationType::FolderReassignment.requires_typed_confirmation());
        assert!(!LocationOperationType::CrossLibraryTransfer.requires_typed_confirmation());
        assert!(!LocationOperationType::Adoption.requires_typed_confirmation());
    }
}

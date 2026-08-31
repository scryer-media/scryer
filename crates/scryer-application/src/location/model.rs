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
    /// No filesystem work at all: fileless titles (FR-076) and folder-match
    /// correction (FR-014).
    CatalogOnly,
}

impl LocationExecutionMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::MoveWithScryer => "move_with_scryer",
            Self::FilesAlreadyThere => "files_already_there",
            Self::CatalogOnly => "catalog_only",
        }
    }

    /// Parse a persisted value. Unknown values are rejected rather than falling
    /// back to a mode that would touch files.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "move_with_scryer" => Some(Self::MoveWithScryer),
            "files_already_there" => Some(Self::FilesAlreadyThere),
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

/// How thoroughly a copied file is proven before its source may be touched.
///
/// FR-042: **full** (default) reads the destination back and compares it against
/// the CRC streamed during the copy; **quick** uses the sampled head+tail proof
/// plus size and is the universal floor — full falls back to it, and verification
/// never drops below it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum VerificationDepth {
    /// Full destination read-back compared against the streamed CRC.
    #[default]
    Full,
    /// Sampled head+tail content proof plus size.
    Quick,
}

impl VerificationDepth {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Quick => "quick",
        }
    }

    /// Parse a persisted setting value. Unknown values are an error rather than a
    /// silent downgrade so a corrupt setting cannot quietly weaken verification.
    pub fn from_setting(value: &str) -> Result<Self, String> {
        match value.trim() {
            "full" => Ok(Self::Full),
            "quick" => Ok(Self::Quick),
            other => Err(format!("unsupported verification depth: {other}")),
        }
    }
}

/// The depth actually applied to one file, including the fallback case.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub struct AppliedVerificationDepth {
    /// The depth the operation was configured to apply.
    pub requested: VerificationDepth,
    /// The depth actually achieved for this file.
    pub applied: VerificationDepth,
    /// True when `requested` was `Full` but only the quick floor could run
    /// (FR-042 fallback); recorded so the reduced guarantee is auditable.
    pub fell_back: bool,
}

impl AppliedVerificationDepth {
    /// The depth was applied exactly as requested.
    pub fn exact(depth: VerificationDepth) -> Self {
        Self {
            requested: depth,
            applied: depth,
            fell_back: false,
        }
    }

    /// Full verification was requested but only the quick floor could run.
    pub fn quick_fallback() -> Self {
        Self {
            requested: VerificationDepth::Full,
            applied: VerificationDepth::Quick,
            fell_back: true,
        }
    }
}

/// Outcome of verifying one destination file.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum FileVerificationOutcome {
    /// Destination content matched at the applied depth; source removal is
    /// unblocked for this file (FR-044).
    Verified,
    /// Destination content did not match; the source is never touched.
    Mismatch,
    /// Verification could not run at all (unreadable destination, vanished
    /// mount); treated as a failure, never as a pass.
    Unavailable,
}

impl FileVerificationOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::Mismatch => "mismatch",
            Self::Unavailable => "unavailable",
        }
    }

    /// Parse a persisted value. An unreadable outcome is never treated as a
    /// pass (FR-044).
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "verified" => Some(Self::Verified),
            "mismatch" => Some(Self::Mismatch),
            "unavailable" => Some(Self::Unavailable),
            _ => None,
        }
    }

    /// Only a verified file may have its source recycled or removed (FR-044).
    pub fn permits_source_removal(&self) -> bool {
        matches!(self, Self::Verified)
    }
}

/// CRC algorithm identifier persisted alongside a stored CRC so a future default
/// change cannot be mistaken for a corruption (FR-041 "algorithm-tagged").
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum MoveCrcAlgorithm {
    /// CRC-64/NVME — the default; see [`crate::location::verify`].
    #[default]
    Crc64Nvme,
}

impl MoveCrcAlgorithm {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Crc64Nvme => "crc64_nvme",
        }
    }

    pub fn from_setting(value: &str) -> Result<Self, String> {
        match value.trim() {
            "crc64_nvme" => Ok(Self::Crc64Nvme),
            other => Err(format!("unsupported move CRC algorithm: {other}")),
        }
    }
}

/// Hashes produced by one streaming copy pass (D2): read once at the source,
/// both values persisted with the destination media file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StreamedContentHashes {
    /// Bytes streamed through the hashers.
    pub size_bytes: u64,
    /// Algorithm tag for `move_crc`.
    pub crc_algorithm: MoveCrcAlgorithm,
    /// Streaming CRC over the copied bytes; the move-corruption check.
    pub move_crc: u64,
    /// Full-file BLAKE3 (hex); the dedup identity (D4, FR-073).
    pub full_blake3: String,
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
/// Migration 0206 has columns for the volume counters only (`title_total`,
/// `title_completed_count`, `title_blocked_count`, `file_total`,
/// `file_completed_count`, `bytes_total`, `bytes_completed`). The outcome
/// counters below it — `merges`, `dedups`, `renames`, `no_ops`, `unresolved` —
/// have no columns yet, so a store round-trip returns them as zero and callers
/// derive them from the checkpoint rows (merged/skipped/blocked titles) and the
/// collision engine's per-file results until a follow-up migration adds them.
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

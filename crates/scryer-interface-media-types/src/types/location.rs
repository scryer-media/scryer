//! GraphQL surface for location operations: the shared move preview, the
//! confirmed operation row, and the inputs that start, cancel, or resume one
//! (US2, FR-010 to FR-017, FR-030, FR-080 to FR-083).
//!
//! Every payload here is a faithful projection of an application type. Where the
//! application does not know something the preview leaves it null rather than
//! guessing: an unprobed free-space estimate reports unknown, and a plan that
//! moves nothing states a verification depth that applies to no files.

use super::{Long, VerificationDepthValue};
use async_graphql::{Enum, ID, InputObject, SimpleObject};
use chrono::{DateTime, Utc};

/// Which location workflow an operation belongs to.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum LocationOperationTypeValue {
    /// Correct which folder a title owns; file content is never touched.
    FolderReassignment,
    /// Move selected titles to another root inside the same library.
    RootMove,
    /// Replace one root's path with a new, unconfigured path.
    RootChange,
    /// Fold one root's managed contents into another root in the same library.
    RootConsolidation,
    /// Move titles into a different library, with or without a merge.
    CrossLibraryTransfer,
    /// Adopt content the user already moved outside Scryer.
    Adoption,
}

/// How the filesystem side of an operation is performed.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum LocationExecutionModeValue {
    /// Scryer performs and verifies the filesystem operation.
    MoveWithScryer,
    /// The user already moved the files; Scryer verifies and adopts them.
    FilesAlreadyThere,
    /// No filesystem work at all: fileless titles and folder-match correction.
    CatalogOnly,
}

/// Lifecycle state of a confirmed location operation, as shown in Activity.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum LocationOperationStateValue {
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
    /// Finished, but with warnings the user must see.
    CompletedWithWarnings,
    /// Stopped at a safe title checkpoint on user request.
    Canceled,
    /// Stopped on an error; completed titles remain consistent.
    Failed,
}

/// The single class a selected title falls into for a requested destination.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum TitleLocationClassValue {
    /// Destination is in another library and the transfer is supported.
    CrossLibraryTransfer,
    /// Destination is another root inside the title's current library.
    RootMove,
    /// The title already lives at the requested destination; nothing to do.
    NoOp,
    /// Monitored title with no tracked files: catalog reassignment only.
    CatalogOnly,
    /// The destination can never accept this title.
    Incompatible,
    /// The title could go, but a user decision is still outstanding.
    NeedsResolution,
}

/// Every kind of change a location plan can contain.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum LocationPlanItemKindValue {
    /// Content moves from a source path to a destination path.
    Move,
    /// Content keeps its directory but changes name.
    Rename,
    /// A source title folds into an existing destination title.
    Merge,
    /// A proven-duplicate file is recycled rather than moved.
    Dedup,
    /// Catalog-only change; no bytes move.
    CatalogChange,
    /// A media file's role for its logical slot changes.
    RoleChange,
    /// The title already satisfies the request; nothing happens.
    NoOp,
    /// The title cannot enter the operation until the user resolves something.
    Blocked,
    /// Content at the source that Scryer does not manage.
    UnmanagedContent,
    /// Something the user must see before confirming.
    Warning,
}

/// Progress of one title inside an operation.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum LocationTitleCheckpointStateValue {
    /// Planned but not started.
    Pending,
    /// Content is being renamed or copied.
    Moving,
    /// Destination content is being proven at the applicable depth.
    Verifying,
    /// Destination verified; catalog ownership and merge unions are running.
    Reconciling,
    /// Sources recycled and empty source directories cleaned up.
    CleaningUp,
    /// Title finished exactly as previewed.
    Completed,
    /// Title finished, but the user must see something about it.
    CompletedWithWarnings,
    /// Deliberately not processed.
    Skipped,
    /// Could not enter the operation.
    Blocked,
    /// Processing failed; the source is intact.
    Failed,
}

/// How much consent an operation demands before it may start.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum LocationConfirmationRequirementValue {
    /// Confirming the fingerprinted plan is enough.
    Simple,
    /// A root-wide operation that also requires the typed phrase.
    Typed,
}

#[derive(SimpleObject, Clone)]
/// One previewed change, in the vocabulary every location workflow shares.
pub struct LocationPlanItemPayload {
    /// What kind of change this item describes.
    pub kind: LocationPlanItemKindValue,
    /// Title this item belongs to, or null when it belongs to none.
    pub title_id: Option<ID>,
    /// Media file this item acts on, or null for untracked content.
    pub media_file_id: Option<ID>,
    /// Source path, or null when the item has no source.
    pub source_path: Option<String>,
    /// Destination path, or null when the item has no destination.
    pub destination_path: Option<String>,
    /// Bytes this item accounts for; zero for catalog-only items.
    pub size_bytes: Long,
    /// Whether source and destination share a volume, or null when undetermined.
    pub same_volume: Option<bool>,
    /// Machine-readable reason, for grouping and translation.
    pub reason_code: Option<String>,
    /// Human-readable explanation, always present for blocking and warning items.
    pub detail: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// One section of a plan: the complete count for a kind plus a sample of its items.
pub struct LocationPlanSectionPayload {
    /// Kind every item in this section shares.
    pub kind: LocationPlanItemKindValue,
    /// Complete count for this section across the whole plan.
    pub items_total: Long,
    /// Complete byte total for this section, not just the sampled items.
    pub bytes_total: Long,
    /// Whether the sampled items are the complete section.
    pub complete: bool,
    /// The sampled items the client renders.
    pub items: Vec<LocationPlanItemPayload>,
}

#[derive(SimpleObject, Clone)]
/// Complete item count for one plan-item kind.
pub struct LocationPlanKindCountPayload {
    /// The kind being counted.
    pub kind: LocationPlanItemKindValue,
    /// How many items of that kind the whole plan contains.
    pub count: Long,
}

#[derive(SimpleObject, Clone)]
/// Complete counts across the whole plan, independent of sampling.
pub struct LocationPlanCountsPayload {
    /// Every plan item, of every kind.
    pub items_total: Long,
    /// Titles the plan covers.
    pub titles_total: Long,
    /// Files the plan covers.
    pub files_total: Long,
    /// Bytes the plan covers.
    pub bytes_total: Long,
    /// Per-kind complete counts, including kinds with no sampled items.
    pub by_kind: Vec<LocationPlanKindCountPayload>,
}

#[derive(SimpleObject, Clone)]
/// One selected title and the class it was previewed as.
pub struct LocationClassifiedTitlePayload {
    /// Selected title identity.
    pub title_id: ID,
    /// Class this title falls into for the requested destination.
    pub class: TitleLocationClassValue,
    /// Library this title would end up in.
    pub destination_library_id: ID,
    /// Root this title would end up on.
    pub destination_root_id: ID,
    /// Machine-readable reason for the class, for grouping and translation.
    pub reason_code: Option<String>,
    /// Human-readable explanation, always present for blocking classes.
    pub reason: Option<String>,
    /// Whether this title stops the operation from starting.
    pub blocks_start: bool,
}

#[derive(SimpleObject, Clone)]
/// One classification group: a class, its count, and the titles in it.
pub struct LocationClassificationGroupPayload {
    /// Class shared by every title in this group.
    pub class: TitleLocationClassValue,
    /// How many selected titles fall into this class.
    pub count: Long,
    /// The titles in this class, in selection order.
    pub titles: Vec<LocationClassifiedTitlePayload>,
}

#[derive(SimpleObject, Clone)]
/// Every selected title grouped by class, with no title omitted.
pub struct LocationSelectionClassificationPayload {
    /// One group per class, always all six, including empty ones.
    pub groups: Vec<LocationClassificationGroupPayload>,
    /// Selected titles across every group.
    pub titles_total: Long,
    /// Whether any title in the selection blocks the start.
    pub blocks_start: bool,
}

#[derive(SimpleObject, Clone)]
/// Estimated free space the operation needs, including recycle-copy cost.
pub struct LocationFreeSpaceEstimatePayload {
    /// Bytes that must be free on the destination volume.
    pub destination_required_bytes: Long,
    /// Destination requirement including the recycle cost when the bin shares that volume.
    pub destination_total_required_bytes: Long,
    /// Bytes free on the destination volume, or null when it was not probed.
    pub destination_available_bytes: Option<Long>,
    /// Bytes that must be free wherever the recycle bin lives.
    pub recycle_required_bytes: Long,
    /// Bytes free on the recycle volume, or null when it was not probed.
    pub recycle_available_bytes: Option<Long>,
    /// Whether the move is a same-volume rename that needs no destination space.
    pub same_volume_move: bool,
    /// Whether recycling copies bytes because the bin is on another volume.
    pub recycle_on_other_volume: bool,
    /// Whether the recycle bin shares the destination volume.
    pub recycle_shares_destination_volume: bool,
    /// Whether recycling is configured and available for this operation.
    pub recycling_available: bool,
    /// Whether the volumes behind this estimate were actually probed.
    pub probed: bool,
    /// Whether the space suffices, or null when it could not be determined.
    pub sufficient: Option<bool>,
}

#[derive(SimpleObject, Clone)]
/// The verification depth this plan will apply, stated before anything moves.
pub struct LocationVerificationStatementPayload {
    /// Depth resolved from the user preference at preview time.
    pub depth: VerificationDepthValue,
    /// Files this depth will be applied to.
    pub files: Long,
    /// Bytes this depth will be applied to.
    pub bytes: Long,
    /// Whether any file in this plan will actually be verified.
    pub applies: bool,
}

#[derive(SimpleObject, Clone)]
/// The confirmation this plan demands before it may start.
pub struct LocationPlanConfirmationPayload {
    /// How much consent the operation demands.
    pub requirement: LocationConfirmationRequirementValue,
    /// The phrase the user must type, when typed confirmation applies.
    pub typed_phrase: Option<String>,
    /// The prompt shown beside the typed-confirmation field.
    pub typed_prompt: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// A read-only preview of a location operation; nothing is changed.
pub struct LocationOperationPreviewPayload {
    /// Fingerprint over the full plan, echoed back to confirm it.
    pub plan_fingerprint: String,
    /// Which location workflow this plan belongs to.
    pub operation_type: LocationOperationTypeValue,
    /// How the filesystem side would be performed.
    pub mode: LocationExecutionModeValue,
    /// Source library, when the whole selection shares one.
    pub source_library_id: Option<ID>,
    /// Destination library for the selection.
    pub destination_library_id: Option<ID>,
    /// Source root, when the whole selection shares one.
    pub source_root_id: Option<ID>,
    /// Destination root, when the request named one.
    pub destination_root_id: Option<ID>,
    /// The selection this plan was built for, in a stable order.
    pub selection: Vec<ID>,
    /// Complete counts across the whole plan.
    pub counts: LocationPlanCountsPayload,
    /// Plan sections in a stable kind order, each a complete count plus a sample.
    pub sections: Vec<LocationPlanSectionPayload>,
    /// Every selected title grouped by class.
    pub classification: LocationSelectionClassificationPayload,
    /// Estimated free space the operation needs.
    pub free_space: LocationFreeSpaceEstimatePayload,
    /// The verification depth that will apply.
    pub verification: LocationVerificationStatementPayload,
    /// The confirmation this plan demands.
    pub confirmation: LocationPlanConfirmationPayload,
    /// Warnings the user must see before confirming.
    pub warnings: Vec<String>,
    /// Whether blocking items or classes stop this plan from starting.
    pub blocks_start: bool,
}

#[derive(SimpleObject, Clone)]
/// Aggregate counters for a running or finished operation.
pub struct LocationOperationCountersPayload {
    /// Titles the confirmed plan covers.
    pub titles_total: Long,
    /// Titles the operation has finished processing.
    pub titles_processed: Long,
    /// Titles that could not enter the operation.
    pub titles_blocked: Long,
    /// Files the confirmed plan covers.
    pub files_total: Long,
    /// Files the operation has finished processing.
    pub files_processed: Long,
    /// Bytes the confirmed plan covers.
    pub bytes_total: Long,
    /// Bytes the operation has finished processing.
    pub bytes_processed: Long,
    /// Titles merged into an existing destination title.
    pub merges: Long,
    /// Files recycled as proven duplicates.
    pub dedups: Long,
    /// Files renamed to avoid a collision.
    pub renames: Long,
    /// Titles that needed no change.
    pub no_ops: Long,
    /// Items still needing a user decision.
    pub unresolved: Long,
}

#[derive(SimpleObject, Clone)]
/// Per-title progress inside an operation; the unit a resume restarts from.
pub struct LocationTitleCheckpointPayload {
    /// Title this checkpoint tracks.
    pub title_id: ID,
    /// Position of this title in the confirmed plan.
    pub sequence: Long,
    /// How far this title has progressed.
    pub state: LocationTitleCheckpointStateValue,
    /// Class this title was previewed as, or null for workflows that do not classify.
    pub classification: Option<TitleLocationClassValue>,
    /// Library the title started in.
    pub source_library_id: Option<ID>,
    /// Root the title started on.
    pub source_root_id: Option<ID>,
    /// Folder the title started in.
    pub source_folder_path: Option<String>,
    /// Library the title ends up in.
    pub destination_library_id: Option<ID>,
    /// Root the title ends up on.
    pub destination_root_id: Option<ID>,
    /// Folder the title ends up in.
    pub destination_folder_path: Option<String>,
    /// Destination title this one merges into, when it merges.
    pub merged_into_title_id: Option<ID>,
    /// Files planned for this title.
    pub files_total: Long,
    /// Files whose destination is verified.
    pub files_verified: Long,
    /// Bytes planned for this title.
    pub bytes_total: Long,
    /// Bytes whose destination is verified.
    pub bytes_verified: Long,
    /// Warning or failure explanation for this title.
    pub detail: Option<String>,
    /// When this title entered the operation.
    pub started_at: Option<DateTime<Utc>>,
    /// When this checkpoint was last written.
    pub updated_at: DateTime<Utc>,
    /// When this title settled.
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(SimpleObject, Clone)]
/// A confirmed location operation and everything Activity shows about it.
pub struct LocationOperationPayload {
    /// Operation identity.
    pub id: ID,
    /// Which location workflow this operation belongs to.
    pub operation_type: LocationOperationTypeValue,
    /// How the filesystem side is performed.
    pub mode: LocationExecutionModeValue,
    /// Lifecycle state.
    pub state: LocationOperationStateValue,
    /// User who confirmed the operation, or null once that user is deleted.
    pub initiated_by_user_id: Option<ID>,
    /// Source library, when the operation is scoped to one.
    pub source_library_id: Option<ID>,
    /// Destination library; differs from the source only for transfers.
    pub destination_library_id: Option<ID>,
    /// Source root, for root-scoped operations.
    pub source_root_id: Option<ID>,
    /// Destination root.
    pub destination_root_id: Option<ID>,
    /// Fingerprint of the full confirmed plan.
    pub plan_fingerprint: String,
    /// Depth the user's preference asked for at confirmation time.
    pub verification_depth: VerificationDepthValue,
    /// Files that could only be proven at the quick floor.
    pub verification_fallback_count: Long,
    /// Aggregate counters shown in Activity.
    pub counters: LocationOperationCountersPayload,
    /// Concise failure or warning explanation.
    pub detail: Option<String>,
    /// The Activity job run this operation reports through, when it has one.
    pub job_run_id: Option<ID>,
    /// The workflow-operation row this operation reports through, when it has one.
    pub workflow_operation_id: Option<ID>,
    /// Whether a cancel was requested.
    pub cancel_requested: bool,
    /// When the cancel was requested.
    pub cancel_requested_at: Option<DateTime<Utc>>,
    /// When the user confirmed the fingerprinted plan.
    pub confirmed_at: Option<DateTime<Utc>>,
    /// When the runner first left the queued state.
    pub started_at: Option<DateTime<Utc>>,
    /// When the operation row was created.
    pub created_at: DateTime<Utc>,
    /// When the operation row was last written.
    pub updated_at: DateTime<Utc>,
    /// When the operation reached a terminal state.
    pub completed_at: Option<DateTime<Utc>>,
    /// Per-title checkpoints in plan order.
    pub title_checkpoints: Vec<LocationTitleCheckpointPayload>,
}

#[derive(SimpleObject, Clone)]
/// Acceptance of a confirmed location operation; the work runs in the background.
pub struct StartLocationOperationPayload {
    /// The accepted operation, as persisted.
    pub operation: LocationOperationPayload,
    /// Fingerprint of the plan the server rebuilt and accepted.
    pub plan_fingerprint: String,
}

#[derive(SimpleObject, Clone)]
/// Result of requesting cancellation; the runner stops at the next title checkpoint.
pub struct CancelLocationOperationPayload {
    /// Operation the cancel was requested for.
    pub id: ID,
    /// Whether the request was recorded; false when the operation already finished.
    pub cancel_requested: bool,
}

#[derive(SimpleObject, Clone)]
/// Result of asking an interrupted operation to pick up from its checkpoints.
pub struct ResumeLocationOperationPayload {
    /// Operation the resume was requested for.
    pub id: ID,
    /// Whether the operation was restarted.
    pub resumed: bool,
    /// Why it was not restarted, when it was not.
    pub detail: Option<String>,
}

#[derive(InputObject, Clone)]
/// Destination for a location operation. Both fields are optional: naming only a
/// root keeps each title in its own library, and naming only a library lets that
/// library's root selection decide.
pub struct LocationDestinationInput {
    /// Destination library, or null to keep each title in its own library.
    pub library_id: Option<ID>,
    /// Destination root, or null to keep each title on its own root.
    pub root_id: Option<ID>,
}

#[derive(InputObject, Clone)]
/// Selection and destination to preview; nothing is changed.
pub struct LocationOperationPreviewInput {
    /// Titles to move, in the order the client submitted them.
    pub title_ids: Vec<ID>,
    /// Where the selection would go.
    pub destination: LocationDestinationInput,
}

#[derive(InputObject, Clone)]
/// Confirmation of a previewed location operation.
pub struct StartLocationOperationInput {
    /// Titles to move, matching the previewed selection.
    pub title_ids: Vec<ID>,
    /// Where the selection goes, matching the previewed destination.
    pub destination: LocationDestinationInput,
    /// Fingerprint of the previewed plan; a stale fingerprint is refused.
    pub plan_fingerprint: String,
    /// Phrase the user typed, for operations that require typed confirmation.
    pub typed_confirmation: Option<String>,
}

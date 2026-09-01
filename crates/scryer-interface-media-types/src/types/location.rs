//! GraphQL surface for location operations: the shared move preview, the
//! confirmed operation row, and the inputs that start, cancel, or resume one
//! (US2, FR-010 to FR-017, FR-030, FR-080 to FR-083).
//!
//! Every payload here is a faithful projection of an application type. Where the
//! application does not know something the preview leaves it null rather than
//! guessing: an unprobed free-space estimate reports unknown, and a plan that
//! moves nothing states a verification depth that applies to no files.

use super::{Long, MediaFacetValue, VerificationDepthValue};
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

/// What destination-title detection concluded for a title crossing into another
/// library. Matching is by stable metadata identity, never by title text.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum LocationDestinationIdentityMatchValue {
    /// Exactly one destination title shares the identity, so this is a merge.
    Unique,
    /// No destination title shares the identity, so this is a plain transfer.
    None,
    /// Several destination titles share an identity and the user must choose.
    Ambiguous,
    /// A destination title has the same name but shares no identity. It is never
    /// merged into automatically.
    SameNameNoIdentity,
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
    /// Library this title lives in today.
    pub source_library_id: ID,
    /// Root this title lives on today.
    pub source_root_id: ID,
    /// Folder this title owns today, or null when it owns none.
    pub source_folder_path: Option<String>,
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
    /// What destination-title detection concluded, or null when the title stays
    /// in its own library and no detection was run.
    pub destination_identity_match: Option<LocationDestinationIdentityMatchValue>,
    /// The existing destination title this title merges into, or null for a
    /// transfer into a title the destination library does not have yet.
    pub merge_target_title_id: Option<ID>,
    /// A destination title carrying the same name but no shared identity, or
    /// null when there is none. It is never merged into automatically.
    pub same_named_destination_title_id: Option<ID>,
    /// Name of that same-named destination title, when there is one.
    pub same_named_destination_title_name: Option<String>,
    /// The destination titles the user is choosing between, for an ambiguous
    /// identity. Empty for every other outcome.
    pub ambiguous_destination_title_ids: Vec<ID>,
    /// The same titles with the names and shared identities the user needs to
    /// tell them apart. Empty for every outcome that is not ambiguous.
    pub ambiguous_destination_candidates: Vec<LocationAmbiguousDestinationCandidatePayload>,
    /// The series↔anime facet conversion this destination performs, or null when
    /// the destination library's facet is the title's own (FR-057).
    pub facet_conversion: Option<LocationFacetConversionPayload>,
}

#[derive(SimpleObject, Clone)]
/// One destination title an ambiguous identity points at, with what it shares.
///
/// The ids alone cannot be chosen between; the name is what the user reads and
/// the shared identities are why the candidate is on the list at all (FR-055).
pub struct LocationAmbiguousDestinationCandidatePayload {
    /// The candidate destination title.
    pub title_id: ID,
    /// Its name in the destination library.
    pub title_name: String,
    /// The identities both titles carry, as `source:external_id`, sorted.
    pub shared_identities: Vec<String>,
}

/// What a facet conversion does to one title-level setting (FR-057).
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum LocationFacetSettingDispositionValue {
    /// The value stays on the title, but nothing reads it under the new facet.
    BecomesInvalid,
    /// The value does not survive the conversion.
    Resets,
    /// The value is still read, and decides something different than it did.
    ChangesMeaning,
}

#[derive(SimpleObject, Clone)]
/// One title-level setting the facet conversion affects, named individually so
/// the client lists them rather than showing a blanket sentence (FR-057).
pub struct LocationFacetConvertedSettingPayload {
    /// Stable machine key for the setting, for grouping and translation.
    pub setting: String,
    /// Human-readable name of the setting.
    pub label: String,
    /// The value the title carries today, or null when it carries none
    /// explicitly and the conversion changes which default applies.
    pub value: Option<String>,
    /// Whether the setting becomes invalid, resets, or changes meaning.
    pub disposition: LocationFacetSettingDispositionValue,
    /// The sentence explaining the consequence.
    pub detail: String,
}

#[derive(SimpleObject, Clone)]
/// The series↔anime conversion a cross-library transfer performs, with every
/// setting it affects (FR-057) and the folder-only scope of the rename
/// (FR-058).
pub struct LocationFacetConversionPayload {
    /// The facet the title carries today.
    pub from_facet: MediaFacetValue,
    /// The facet it carries after the transfer.
    pub to_facet: MediaFacetValue,
    /// Every affected setting, in a stable order. Empty when the conversion
    /// touches nothing the title has set.
    pub settings: Vec<LocationFacetConvertedSettingPayload>,
    /// FR-058, as the sentence the preview shows: the conversion recalculates
    /// the folder name only.
    pub files_keep_their_names: String,
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
    /// One summary per title that merges into an existing destination title.
    /// Empty for every plan with no merge in it.
    pub merges: Vec<LocationMergePreviewPayload>,
}

/// How one table's rows are treated when a title merges into another (FR-064).
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum LocationMergeDispositionValue {
    /// Source rows are re-pointed and kept beside the destination's own.
    Union,
    /// Source rows are rewritten through the source-to-destination identity map.
    Map,
    /// The destination's value stands and the source's is discarded.
    DestinationWins,
    /// Source rows are intentionally not carried over.
    Drop,
}

/// Why one record stops a merge from running (FR-066).
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum LocationMergeBlockReasonValue {
    /// No destination episode carries the source episode's identity.
    UnmappedEpisode,
    /// More than one destination episode carries it.
    AmbiguousDestinationEpisode,
    /// Two source episodes carry the same identity.
    AmbiguousSourceEpisode,
    /// The source episode has no season/episode pair and no absolute number.
    UnidentifiableEpisode,
    /// A record references a source episode that is not in the catalog.
    UnknownEpisodeReference,
    /// No destination collection carries the source collection's identity.
    UnmappedCollection,
    /// More than one destination collection carries it.
    AmbiguousDestinationCollection,
    /// Two source collections carry the same identity.
    AmbiguousSourceCollection,
    /// No destination series-movie link carries the source link's identity.
    UnmappedSeriesMovieLink,
    /// More than one destination link carries it.
    AmbiguousDestinationSeriesMovieLink,
    /// Two source links carry the same identity.
    AmbiguousSourceSeriesMovieLink,
    /// Another resumable location operation still holds the source title.
    ResumableOperationHoldsSource,
    /// An unconsumed manual-import selection is an active import on the source.
    ActiveManualImportSelection,
}

/// The role a media file holds for one logical slot after a merge (FR-068).
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum LocationMergeMediaRoleValue {
    /// The file that represents the slot.
    Primary,
    /// A further file kept alongside the primary.
    Additional,
}

/// Why a media file's role changed in a merge (FR-070).
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum LocationMergeRoleChangeReasonValue {
    /// The destination already had a primary for the slot, and a move never
    /// demotes one.
    DestinationPrimaryRetained,
    /// Another source file already claimed primary for that destination episode.
    SourcePrimaryAlreadyClaimed,
    /// Two source episodes collapsed onto one destination episode.
    CollapsedSourceEpisodes,
}

/// Derived-cache work a completed merge leaves behind for Scryer to redo.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum LocationMergePostMergeWorkValue {
    /// Rebuild the merged title's search projection.
    ReindexTitleSearchTerms,
    /// Refresh the merged title's recommendations.
    RegenerateRecommendations,
    /// Recompute title and library statistics.
    RecomputeStatistics,
    /// Drop the retired title's indexer coverage rows.
    DropSourceIndexerCoverage,
}

#[derive(SimpleObject, Clone)]
/// One record the merge engine could not map, so the merge cannot run (FR-066).
pub struct LocationMergeBlockedRecordPayload {
    /// The table whose source rows cannot be carried.
    pub table: String,
    /// Why the record blocks the merge.
    pub reason: LocationMergeBlockReasonValue,
    /// The source identity that could not be mapped.
    pub source_id: ID,
    /// The sentence explaining what would otherwise be guessed at.
    pub detail: String,
}

#[derive(SimpleObject, Clone)]
/// One setting the destination title keeps and the merging title loses (FR-063).
pub struct LocationMergeDestinationWinsPayload {
    /// The setting, in the words the rule states it.
    pub setting: String,
    /// The value the destination keeps, when it is a value worth naming.
    pub destination_value: Option<String>,
    /// The value the merging title loses, when it is a value worth naming.
    pub source_value: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// One table's contribution to the merge, with the rows it applies to (FR-064).
pub struct LocationMergeTableDispositionPayload {
    /// The table.
    pub table: String,
    /// How its rows are treated.
    pub disposition: LocationMergeDispositionValue,
    /// Source rows the disposition applies to.
    pub source_row_count: Long,
    /// Why, in one line.
    pub note: String,
}

#[derive(SimpleObject, Clone)]
/// One media-file role the merge resolves, never silently (FR-068 to FR-070).
pub struct LocationMergeRoleChangePayload {
    /// The media file whose role changes.
    pub file_id: ID,
    /// The source episode the file was attached to.
    pub source_episode_id: ID,
    /// The destination episode it is attached to after the merge.
    pub destination_episode_id: ID,
    /// The role it held.
    pub previous_role: LocationMergeMediaRoleValue,
    /// The role it holds after the merge.
    pub new_role: LocationMergeMediaRoleValue,
    /// Why the role changed.
    pub reason: LocationMergeRoleChangeReasonValue,
    /// The sentence the preview shows for this change.
    pub detail: String,
}

#[derive(SimpleObject, Clone)]
/// One reserved setting whose two sides disagreed; the destination's wins.
pub struct LocationMergeReservedTagConflictPayload {
    /// The reserved tag prefix, for grouping and translation.
    pub prefix: String,
    /// Human-readable name of the setting, when the prefix is a known one.
    pub setting: Option<String>,
    /// The value the destination keeps.
    pub destination_value: Option<String>,
    /// The value the merging title loses.
    pub source_value: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// One media request whose library follows the content into the destination.
pub struct LocationMergeMediaRequestRepointPayload {
    /// The request being repointed.
    pub request_id: ID,
    /// The library it belonged to.
    pub previous_library_id: ID,
    /// The library it belongs to after the merge.
    pub destination_library_id: ID,
}

#[derive(SimpleObject, Clone)]
/// One category of data the merge deliberately does not carry (FR-071).
pub struct LocationMergeDroppedCategoryPayload {
    /// The table the rows live in.
    pub table: String,
    /// How many source rows are dropped.
    pub source_row_count: Long,
    /// The adjudication that decided it.
    pub decision: String,
    /// Why dropping is the right answer.
    pub reason: String,
}

#[derive(SimpleObject, Clone)]
/// What merging one title into an existing destination title would do (FR-071).
///
/// The same decision the merge itself is built from, so the preview can never
/// describe a merge the engine would not perform.
pub struct LocationMergePreviewPayload {
    /// The title that merges away.
    pub source_title_id: ID,
    /// The title that survives.
    pub destination_title_id: ID,
    /// Library the merging title comes from.
    pub source_library_id: Option<ID>,
    /// Library the surviving title lives in.
    pub destination_library_id: Option<ID>,
    /// Whether unmappable records stop this merge from running.
    pub blocked: bool,
    /// The records that block it, one per line the user can act on.
    pub blocked_records: Vec<LocationMergeBlockedRecordPayload>,
    /// What the destination keeps.
    pub destination_wins: Vec<LocationMergeDestinationWinsPayload>,
    /// What carries forward, per table, with counts.
    pub dispositions: Vec<LocationMergeTableDispositionPayload>,
    /// Every media-file role the merge changes.
    pub role_changes: Vec<LocationMergeRoleChangePayload>,
    /// Reserved settings whose values disagree.
    pub reserved_tag_conflicts: Vec<LocationMergeReservedTagConflictPayload>,
    /// Free-form tags the merging title contributes.
    pub free_form_tags_added: Vec<String>,
    /// Requests whose library follows the content.
    pub media_request_repoints: Vec<LocationMergeMediaRequestRepointPayload>,
    /// What is not carried over, and why.
    pub dropped: Vec<LocationMergeDroppedCategoryPayload>,
    /// Derived caches the merge leaves for Scryer to rebuild afterwards.
    pub post_merge_work: Vec<LocationMergePostMergeWorkValue>,
    /// Anything else the operator should read before confirming, including the
    /// notes explaining where this schema differs from the merge inventory.
    pub notes: Vec<String>,
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

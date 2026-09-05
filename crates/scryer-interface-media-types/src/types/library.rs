use super::{
    DownloadClientRoutingEntryInput, DownloadClientRoutingEntryPayload, DownloadSourceKindValue,
    FillerPolicyValue, ImportDecisionValue, ImportModeValue, ImportSkipReasonValue,
    ImportStatusValue, ImportTypeValue, IndexerRoutingEntryInput, IndexerRoutingEntryPayload,
    JobRunPayload, Long, ManualImportCandidateMappingInput, MediaFacetValue, MonitorSelectionInput,
    MonitorTypeValue, PendingImportStatusValue, RecapPolicyValue, RenameCollisionPolicyValue,
    RenameMissingMetadataPolicyValue, ScoringPersonaValue,
};
use async_graphql::{Enum, ID, InputObject, MaybeUndefined, OneofObject, SimpleObject};
use chrono::{DateTime, Utc};

#[derive(SimpleObject, Clone)]
/// Root-folder path and default marker.
pub struct RootFolderPayload {
    /// Filesystem path.
    pub path: String,
    /// Whether this is the default root folder.
    pub is_default: bool,
}

#[derive(SimpleObject, Clone)]
/// Library root-folder identity, path, and default marker.
pub struct LibraryRootPayload {
    /// Root-folder ID.
    pub id: ID,
    /// Filesystem path.
    pub path: String,
    /// Whether this is the default root folder.
    pub is_default: bool,
}

#[derive(SimpleObject, Clone)]
/// Effective library settings together with nullable per-library overrides.
pub struct LibrarySettingsPayload {
    /// Library override for required audio language codes; `original` resolves per title.
    pub required_audio_languages_override: Option<Vec<String>>,
    /// Effective configured requirements after inheritance; `original` remains unchanged.
    pub required_audio_languages: Vec<String>,
    /// Library metadata-language override; null means inherit the global default.
    pub metadata_language_override: Option<String>,
    /// Effective metadata language after inheritance.
    pub metadata_language: String,
    /// Library override for season folders; null means inherit the facet setting.
    pub use_season_folders_override: Option<bool>,
    /// Effective season-folder setting after inheritance.
    pub use_season_folders: bool,
    /// Library override quality-profile ID; null means inherit.
    pub quality_profile_id_override: Option<ID>,
    /// Effective quality-profile ID.
    pub quality_profile_id: ID,
    /// Library override request quality-profile IDs; null means inherit.
    pub request_quality_profile_ids_override: Option<Vec<ID>>,
    /// Effective request quality-profile IDs; empty means no additional profiles.
    pub request_quality_profile_ids: Vec<ID>,
    /// Effective default request quality-profile ID.
    pub request_quality_profile_default_id: ID,
    /// Library override scoring persona; null means inherit.
    pub scoring_persona_override: Option<ScoringPersonaValue>,
    /// Effective scoring persona.
    pub scoring_persona: ScoringPersonaValue,
    /// Library override filler policy; null means inherit.
    pub filler_policy_override: Option<FillerPolicyValue>,
    /// Effective filler policy, or null when unset.
    pub filler_policy: Option<FillerPolicyValue>,
    /// Library override recap policy; null means inherit.
    pub recap_policy_override: Option<RecapPolicyValue>,
    /// Effective recap policy, or null when unset.
    pub recap_policy: Option<RecapPolicyValue>,
    /// Library override for monitoring specials; null means inherit.
    pub monitor_specials_override: Option<bool>,
    /// Effective specials monitoring setting, or null when unset.
    pub monitor_specials: Option<bool>,
    /// Library override for inter-season movies; null means inherit.
    pub inter_season_movies_override: Option<bool>,
    /// Effective inter-season movie setting, or null when unset.
    pub inter_season_movies: Option<bool>,
    /// Library override for filler movies; null means inherit.
    pub monitor_filler_movies_override: Option<bool>,
    /// Effective filler-movie monitoring setting, or null when unset.
    pub monitor_filler_movies: Option<bool>,
    /// Library override for NFO writing; null means inherit.
    pub nfo_write_on_import_override: Option<bool>,
    /// Effective NFO-on-import setting.
    pub nfo_write_on_import: bool,
    /// Library override for Plex match writing; null means inherit.
    pub plexmatch_write_on_import_override: Option<bool>,
    /// Effective Plex match-on-import setting, or null when unset.
    pub plexmatch_write_on_import: Option<bool>,
    /// Library override import mode; null means inherit.
    pub import_mode_override: Option<ImportModeValue>,
    /// Effective import mode.
    pub import_mode: ImportModeValue,
    /// Library override for Linux permission updates; null means inherit.
    pub set_permissions_linux_override: Option<bool>,
    /// Effective Linux permission-update setting.
    pub set_permissions_linux: bool,
    /// Library override file chmod mode; null means inherit.
    pub file_chmod_override: Option<String>,
    /// Effective file chmod mode, or null when unset.
    pub file_chmod: Option<String>,
    /// Library override folder chmod mode; null means inherit.
    pub folder_chmod_override: Option<String>,
    /// Effective folder chmod mode, or null when unset.
    pub folder_chmod: Option<String>,
    /// Library override chown group; null means inherit.
    pub chown_group_override: Option<String>,
    /// Effective chown group, or null when unset.
    pub chown_group: Option<String>,
    /// Library override indexer routing entries; null means inherit.
    pub indexer_routing_override: Option<Vec<IndexerRoutingEntryPayload>>,
    /// Library override download-client routing entries; null means inherit.
    pub download_client_routing_override: Option<Vec<DownloadClientRoutingEntryPayload>>,
}

#[derive(SimpleObject, Clone)]
/// Download-client option used by paged queue and history filters.
pub struct DownloadClientFilterOptionPayload {
    /// Download-client ID.
    pub client_id: ID,
    /// Download-client name.
    pub client_name: String,
    /// Download-client provider type.
    pub client_type: String,
}

#[derive(SimpleObject, Clone)]
/// Result of one import operation, including decision and source/destination paths.
pub struct ImportResultPayload {
    /// Import record ID.
    pub import_id: ID,
    /// Final import decision.
    pub decision: ImportDecisionValue,
    /// Skip reason when the decision skipped the import, or null otherwise.
    pub skip_reason: Option<ImportSkipReasonValue>,
    /// Imported title ID, or null when no title was bound.
    pub title_id: Option<ID>,
    /// Source path examined by the import.
    pub source_path: String,
    /// Destination path written by the import, or null when no destination was written.
    pub dest_path: Option<String>,
    /// Error message, or null when the operation completed without an error.
    pub error_message: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// Persisted import record with lifecycle status, decision, IDs, and UTC timestamps.
pub struct ImportRecordPayload {
    /// Import record ID.
    pub id: ID,
    /// External system that supplied the import.
    pub source_system: String,
    /// Source-system reference identifying the download or item.
    pub source_ref: String,
    /// Source title, or null when unavailable.
    pub source_title: Option<String>,
    /// Media facet, or null when not yet classified.
    pub facet: Option<MediaFacetValue>,
    /// Import operation type.
    pub import_type: ImportTypeValue,
    /// Current import lifecycle status.
    pub status: ImportStatusValue,
    /// Error message, or null when no error is recorded.
    pub error_message: Option<String>,
    /// Import decision, or null before a decision is made.
    pub decision: Option<ImportDecisionValue>,
    /// Skip reason, or null when not skipped.
    pub skip_reason: Option<ImportSkipReasonValue>,
    /// Bound title ID, or null when no title was selected.
    pub title_id: Option<ID>,
    /// Source path, or null when not recorded.
    pub source_path: Option<String>,
    /// Destination path, or null when no destination was produced.
    pub dest_path: Option<String>,
    /// UTC start time, or null before processing begins.
    pub started_at: Option<DateTime<Utc>>,
    /// UTC completion time, or null while incomplete.
    pub finished_at: Option<DateTime<Utc>>,
    /// UTC record creation time.
    pub created_at: DateTime<Utc>,
}

#[derive(InputObject)]
/// Requests retrying one import record, optionally supplying an archive password.
pub struct RetryImportInput {
    /// Import record ID to retry.
    pub import_id: ID,
    /// Optional password for an encrypted source archive.
    pub password: Option<String>,
}

#[derive(InputObject)]
/// Identifies a tracked download to ignore without deleting it from the download client.
pub struct IgnoreTrackedDownloadInput {
    /// Download-client ID, or null when the provider identity is sufficient.
    pub client_id: Option<ID>,
    /// Download-client provider type.
    pub client_type: String,
    /// Provider-issued download item ID.
    pub download_client_item_id: String,
}

#[derive(InputObject)]
/// Marks a tracked download failed and optionally prevents reacquisition.
pub struct MarkTrackedDownloadFailedInput {
    /// Download-client ID, or null when the provider identity is sufficient.
    pub client_id: Option<ID>,
    /// Download-client provider type.
    pub client_type: String,
    /// Provider-issued download item ID.
    pub download_client_item_id: String,
    /// When true, suppress reacquisition after marking failure.
    pub skip_reacquire: Option<bool>,
}

#[derive(OneofObject, Clone)]
/// Union input selecting the acquisition scope of a tracked download.
pub enum QueueDownloadScopeInput {
    /// One target episode ID.
    Episode(ID),
    /// Multiple target episode IDs.
    EpisodeSet(Vec<ID>),
    /// One series-movie link ID.
    SeriesMovie(ID),
    /// One collection ID.
    Collection(ID),
    /// Whole-title marker; the boolean must indicate the title scope.
    Title(bool),
}

#[derive(InputObject)]
/// Assigns a tracked download to a title and an explicit acquisition scope.
pub struct AssignTrackedDownloadTitleInput {
    /// Download-client ID, or null when the provider identity is sufficient.
    pub client_id: Option<ID>,
    /// Download-client provider type.
    pub client_type: String,
    /// Provider-issued download item ID.
    pub download_client_item_id: String,
    /// Target title ID.
    pub title_id: ID,
    /// Target scope within the title.
    pub scope: QueueDownloadScopeInput,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Lifecycle of title hydration while a tracked download is being associated.
pub enum AddTitleHydrationStateValue {
    /// Hydration has been requested but is not complete.
    Pending,
    /// Hydration completed successfully.
    Complete,
    /// No hydration was needed.
    NotRequired,
}

#[derive(InputObject)]
/// Requests scanning one library and optionally supplies an import warmup session.
pub struct ScanLibraryInput {
    /// Target library ID.
    pub library_id: ID,
    /// Optional import warmup session ID to use during the scan.
    pub import_warmup_session_id: Option<ID>,
}

#[derive(SimpleObject, Clone)]
/// Counts produced by a library scan.
pub struct LibraryScanSummaryPayload {
    /// Number of files or records scanned.
    pub scanned: i32,
    /// Number matched to known titles.
    pub matched: i32,
    /// Number imported successfully.
    pub imported: i32,
    /// Number skipped by scan or import policy.
    pub skipped: i32,
    /// Number left unmatched.
    pub unmatched: i32,
}

#[derive(SimpleObject, Clone)]
/// Pending-import counts grouped by media facet.
pub struct PendingImportCountsPayload {
    /// Pending movie count.
    pub movie: i32,
    /// Pending series count.
    pub series: i32,
    /// Pending anime count.
    pub anime: i32,
}

#[derive(SimpleObject, Clone)]
/// Title-history event counts for one dashboard activity window.
pub struct ActivityWindowCountsPayload {
    /// Releases grabbed during the window.
    pub grabbed: i32,
    /// Existing files replaced by a better release during the window.
    pub upgraded: i32,
    /// Imports that completed during the window.
    pub imported: i32,
    /// Imports rejected as failed during the window; skipped imports are excluded.
    pub import_failed: i32,
    /// Downloads that failed during the window.
    pub download_failed: i32,
}

#[derive(SimpleObject, Clone)]
/// A trailing activity window together with the window immediately before it,
/// so callers can render each count with its period-over-period delta.
pub struct DashboardActivityStatsPayload {
    /// Counts for the trailing window ending at the time of the request.
    pub current: ActivityWindowCountsPayload,
    /// Counts for the equally long window immediately before the current one.
    pub previous: ActivityWindowCountsPayload,
}

#[derive(SimpleObject, Clone)]
/// Filesystem usage of the volume backing one library root folder.
pub struct StorageRootUsagePayload {
    /// Configured root-folder path as stored on the library.
    pub path: String,
    /// ID of the library that owns this root folder.
    pub library_id: ID,
    /// Name of the library that owns this root folder.
    pub library_name: String,
    /// Media facet of the library that owns this root folder.
    pub facet: MediaFacetValue,
    /// Bytes in use on the backing filesystem; null when it cannot be inspected.
    pub used_bytes: Option<Long>,
    /// Total bytes on the backing filesystem; null when it cannot be inspected.
    pub total_bytes: Option<Long>,
}

#[derive(SimpleObject, Clone)]
/// Media-request counts grouped by media facet.
pub struct MediaRequestCountsPayload {
    /// Movie request count.
    pub movie: i32,
    /// Series request count.
    pub series: i32,
    /// Anime request count.
    pub anime: i32,
}

#[derive(SimpleObject, Clone)]
/// Authorization-filtered counts used for application navigation indicators.
pub struct NavigationBadgeCountsPayload {
    /// Pending imports visible to the caller.
    pub pending_import_counts: PendingImportCountsPayload,
    /// Pending media requests visible to the caller.
    pub pending_media_request_counts: MediaRequestCountsPayload,
    /// Count of imports awaiting operator attention and visible to the caller.
    pub activity_import_count: i32,
    /// Count of available plugin updates visible to the caller.
    pub plugin_update_count: i32,
}

#[derive(SimpleObject, Clone)]
/// One metadata search attempt made while resolving a pending import.
pub struct PendingImportSearchAttemptPayload {
    /// Search query submitted.
    pub query: String,
    /// Number of metadata results returned.
    pub result_count: i32,
    /// Top result titles retained for diagnostics.
    pub top_results: Vec<String>,
    /// Human-readable attempt summary.
    pub summary: String,
}

/// Coarse bucket for why a pending import is awaiting resolution.
///
/// The free-text `reason` field remains the authoritative scanner code; this
/// enum is the stable grouping the dashboard filters on, so a new scanner code
/// surfaces as `OTHER` instead of breaking clients.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum PendingImportReasonClassValue {
    /// Metadata lookup returned no candidates to choose from.
    Unmatched,
    /// Metadata lookup returned candidates but none could be accepted automatically.
    Ambiguous,
    /// The file's media metadata could not be read, so its quality is unknown.
    QualityUnknown,
    /// Any other scanner reason, including parse and folder-ownership problems.
    Other,
}

#[derive(SimpleObject, Clone)]
/// One unmatched library item awaiting import resolution.
pub struct PendingImportItemPayload {
    /// Pending-import item ID.
    pub id: ID,
    /// Library ID containing the item.
    pub library_id: ID,
    /// Media facet inferred for the item.
    pub facet: MediaFacetValue,
    /// Current pending-import status.
    pub status: PendingImportStatusValue,
    /// Bound title ID, or null before resolution.
    pub title_id: Option<ID>,
    /// Bound title name, or null before resolution.
    pub title_name: Option<String>,
    /// Bound title slug, or null before resolution.
    pub title_slug: Option<String>,
    /// Display name derived from the unmatched item.
    pub display_name: String,
    /// Full source path.
    pub path: String,
    /// Containing folder path, or null when not applicable.
    pub folder_path: Option<String>,
    /// Metadata query suggested for resolution.
    pub query: String,
    /// Parsed year hint, or null when unavailable.
    pub year_hint: Option<i32>,
    /// Explanation for the pending state.
    pub reason: String,
    /// Coarse bucket for `reason`, stable across scanner reason-code changes.
    pub reason_class: PendingImportReasonClassValue,
    /// Metadata search attempts made for this item.
    pub search_attempts: Vec<PendingImportSearchAttemptPayload>,
    /// Size of the pending file; null for folder items and unreadable files.
    pub size_bytes: Option<Long>,
    /// When the scanner first recorded this item.
    pub created_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
/// Offset-paginated pending-import connection.
pub struct PendingImportConnectionPayload {
    /// Items in the requested page; empty means no items matched.
    pub items: Vec<PendingImportItemPayload>,
    /// Total matching items before pagination.
    pub total_count: i32,
    /// Whether more matching items exist after this page.
    pub has_more: bool,
}

#[derive(InputObject)]
/// Resolves one pending import to a title and requested title metadata.
pub struct ResolvePendingImportInput {
    /// Pending-import item ID.
    pub pending_import_id: ID,
    /// Title metadata to associate with the item.
    pub title: AddTitleInput,
}

#[derive(SimpleObject, Clone)]
/// Parsed file details and suggested episode bindings for a pending import.
pub struct PendingImportBindingFilePreviewPayload {
    /// Full file path.
    pub file_path: String,
    /// File name component.
    pub file_name: String,
    /// File size in bytes.
    pub size_bytes: Long,
    /// Parsed season number, or null when unavailable.
    pub parsed_season: Option<i32>,
    /// Parsed episode numbers; empty means none were detected.
    pub parsed_episodes: Vec<i32>,
    /// Parsed absolute episode numbers; empty means none were detected.
    pub parsed_absolute_numbers: Vec<i32>,
    /// Suggested episode IDs; empty means no suggestions were found.
    pub suggested_episode_ids: Vec<ID>,
}

#[derive(InputObject)]
/// Binds one pending import to an optional collection and episode IDs.
pub struct BindPendingImportInput {
    /// Pending-import item ID.
    pub pending_import_id: ID,
    /// Collection ID, or null when binding episodes directly.
    pub collection_id: Option<ID>,
    /// Episode IDs to bind; empty means no explicit episode list.
    pub episode_ids: Vec<ID>,
}

#[derive(SimpleObject, Clone)]
/// Result of ignoring one pending import.
pub struct IgnorePendingImportPayload {
    /// Pending-import item ID.
    pub id: ID,
    /// Status after the ignore operation.
    pub status: PendingImportStatusValue,
}

#[derive(SimpleObject, Clone)]
/// Result of requesting cancellation for an acquisition search.
pub struct CancelAcquisitionSearchPayload {
    /// Acquisition-search job ID.
    pub id: ID,
    /// True when cancellation was accepted; false when the search was already terminal.
    pub accepted: bool,
}

#[derive(SimpleObject, Clone)]
/// Result of requesting cancellation for a library scan.
pub struct CancelLibraryScanPayload {
    /// Library-scan session ID.
    pub session_id: ID,
    /// True when cancellation was accepted; false when the scan was already terminal.
    pub accepted: bool,
}

#[derive(SimpleObject, Clone)]
/// Read-only media deletion preview with confirmation and sample-path details.
pub struct DeletePreviewPayload {
    /// Stable preview fingerprint required to apply the same plan.
    pub fingerprint: String,
    /// Total files selected by the preview.
    pub total_file_count: i32,
    /// Selected media-file count.
    pub media_count: i32,
    /// Selected subtitle-file count.
    pub subtitle_count: i32,
    /// Selected image-file count.
    pub image_count: i32,
    /// Selected other-file count.
    pub other_count: i32,
    /// Selected directory count.
    pub directory_count: i32,
    /// Whether typed confirmation is required before applying the plan.
    pub requires_typed_confirmation: bool,
    /// Required confirmation text, or null when typed confirmation is not required.
    pub typed_confirmation_prompt: Option<String>,
    /// Human-readable preview target label.
    pub target_label: String,
    /// Sample paths from the selected files and directories.
    pub sample_paths: Vec<String>,
}

#[derive(SimpleObject, Clone)]
/// Per-title result within a multi-title deletion preview.
pub struct DeleteTitlePreviewResultPayload {
    /// Target title ID.
    pub title_id: ID,
    /// Deletion preview, or null when preview generation failed.
    pub preview: Option<DeletePreviewPayload>,
    /// Error message, or null when preview generation succeeded.
    pub error: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// Combined deletion preview for multiple titles.
pub struct DeleteTitlesPreviewPayload {
    /// Aggregate preview across all successful title targets.
    pub preview: DeletePreviewPayload,
    /// Per-title results.
    pub items: Vec<DeleteTitlePreviewResultPayload>,
    /// Number of title targets that failed preview generation.
    pub failed_count: i32,
}

#[derive(SimpleObject, Clone)]
/// Per-file result within a multi-episode media-file deletion preview.
pub struct DeleteEpisodeFilePreviewResultPayload {
    /// Media-file identity that would be deleted.
    pub file_id: ID,
    /// Episode identity the media file is linked to.
    pub episode_id: ID,
    /// Deletion preview for this file, or null when preview generation failed.
    pub preview: Option<DeletePreviewPayload>,
    /// Error message, or null when preview generation succeeded.
    pub error: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// Combined deletion preview for the media files of several episodes of one title.
pub struct DeleteEpisodeFilesPreviewPayload {
    /// Aggregate preview across all resolved media files.
    pub preview: DeletePreviewPayload,
    /// Per-file results, ordered by media-file identity.
    pub items: Vec<DeleteEpisodeFilePreviewResultPayload>,
    /// Number of media files resolved from the requested episodes.
    pub file_count: i32,
    /// Number of media files that failed preview generation.
    pub failed_count: i32,
}

#[derive(SimpleObject, Clone)]
/// Accepted media-file IDs and background job information for a multi-episode
/// media-file deletion request.
pub struct DeleteEpisodeFilesPayload {
    /// Background job run tracking the deletion work.
    pub job_run: JobRunPayload,
    /// Media-file identities accepted for processing.
    pub accepted_file_ids: Vec<ID>,
}

#[derive(SimpleObject, Clone)]
/// Accepted title IDs and background job information for a deletion request.
pub struct DeleteTitlesPayload {
    /// Background job run tracking the deletion work.
    pub job_run: JobRunPayload,
    /// Title IDs accepted for processing.
    pub accepted_title_ids: Vec<ID>,
}

#[derive(SimpleObject, Clone)]
/// One media rename plan item with current and proposed paths and decision details.
pub struct MediaRenamePlanItemPayload {
    /// Collection ID, or null when not part of a collection.
    pub collection_id: Option<ID>,
    /// Series-movie link IDs associated with the item.
    pub series_movie_link_ids: Vec<ID>,
    /// Current filesystem path.
    pub current_path: String,
    /// Proposed filesystem path, or null when no rename is possible.
    pub proposed_path: Option<String>,
    /// Normalized filename, or null when metadata is insufficient.
    pub normalized_filename: Option<String>,
    /// Whether the proposed path collides with another path.
    pub collision: bool,
    /// Machine-readable reason for the plan decision.
    pub reason_code: String,
    /// Planned write action.
    pub write_action: String,
    /// Source file size in bytes, or null when unavailable.
    pub source_size_bytes: Option<Long>,
    /// Source modification time as Unix milliseconds, or null when unavailable.
    pub source_mtime_unix_ms: Option<Long>,
}

#[derive(SimpleObject, Clone)]
/// Read-only media rename plan and stable fingerprint for later application.
pub struct MediaRenamePlanPayload {
    /// Media facet covered by the plan.
    pub facet: MediaFacetValue,
    /// Target title ID, or null when the plan covers the whole facet.
    pub title_id: Option<ID>,
    /// Rename template used to generate proposed paths.
    pub template: String,
    /// Collision policy used by the plan.
    pub collision_policy: RenameCollisionPolicyValue,
    /// Missing-metadata policy used by the plan.
    pub missing_metadata_policy: RenameMissingMetadataPolicyValue,
    /// Stable fingerprint required to apply this exact plan.
    pub fingerprint: String,
    /// Total plan-item count.
    pub total: i32,
    /// Number of items eligible for rename.
    pub renamable: i32,
    /// Number of items already matching the desired path.
    pub noop: i32,
    /// Number of collision items.
    pub conflicts: i32,
    /// Number of items with planning errors.
    pub errors: i32,
    /// Plan items in deterministic service order.
    pub items: Vec<MediaRenamePlanItemPayload>,
}

#[derive(SimpleObject, Clone)]
/// Result for one applied media rename plan item.
pub struct MediaRenameApplyItemPayload {
    /// Collection ID, or null when not part of a collection.
    pub collection_id: Option<ID>,
    /// Series-movie link IDs associated with the item.
    pub series_movie_link_ids: Vec<ID>,
    /// Path before the apply operation.
    pub current_path: String,
    /// Planned path, or null when no rename was proposed.
    pub proposed_path: Option<String>,
    /// Final path after application, or null when not moved.
    pub final_path: Option<String>,
    /// Write action attempted.
    pub write_action: String,
    /// Apply status.
    pub status: String,
    /// Machine-readable reason for the outcome.
    pub reason_code: String,
    /// Error message, or null when the item succeeded or was skipped without error.
    pub error_message: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// Aggregate result of applying a previously generated rename plan.
pub struct MediaRenameApplyPayload {
    /// Fingerprint of the plan that was applied.
    pub plan_fingerprint: String,
    /// Total plan-item count.
    pub total: i32,
    /// Number of items applied.
    pub applied: i32,
    /// Number of items skipped.
    pub skipped: i32,
    /// Number of items that failed.
    pub failed: i32,
    /// Per-item apply results.
    pub items: Vec<MediaRenameApplyItemPayload>,
}

#[derive(InputObject, Clone)]
/// An external identifier supplied with a title or media request.
pub struct ExternalIdInput {
    /// Provider namespace for the identifier, such as TVDB or IMDb.
    pub source: String,
    /// Provider-issued identifier value.
    pub value: String,
}

#[derive(InputObject, Clone)]
/// Optional title settings used when creating or updating a title.
pub struct TitleOptionsInput {
    /// Quality profile identity; omission preserves the current value, null clears it, and a value replaces it.
    pub quality_profile_id: MaybeUndefined<ID>,
    /// Root-folder identity used when a title is created; omission preserves the current value, null clears it, and a value replaces it. Changing the root of an existing title that has tracked files is refused: preview the change with locationOperationPreview and run it with startLocationOperation.
    #[graphql(
        deprecation = "Retired for existing titles with tracked files: use locationOperationPreview and startLocationOperation to move a title. Still accepted at title creation and for titles with no tracked files."
    )]
    pub root_folder_id: MaybeUndefined<ID>,
    /// Monitoring policy; omission preserves the current value, null clears it, and a value replaces it.
    pub monitor_type: MaybeUndefined<MonitorTypeValue>,
    /// Whether season folders are used for Series or Anime; omission preserves the current value, null clears it, and a value replaces it. Movies reject this field.
    pub use_season_folders: MaybeUndefined<bool>,
    /// Metadata language; omission preserves the current value, null clears it, and a value replaces it.
    pub metadata_language: MaybeUndefined<String>,
    /// Whether specials are monitored; omission preserves the current value, null clears it, and a value replaces it.
    pub monitor_specials: MaybeUndefined<bool>,
    /// Whether inter-season movies are monitored; omission preserves the current value, null clears it, and a value replaces it.
    pub inter_season_movies: MaybeUndefined<bool>,
    /// Filler policy; omission preserves the current value, null clears it, and a value replaces it.
    pub filler_policy: MaybeUndefined<FillerPolicyValue>,
    /// Recap policy; omission preserves the current value, null clears it, and a value replaces it.
    pub recap_policy: MaybeUndefined<RecapPolicyValue>,
    /// Seasons and series movies to monitor under `ADVANCED`; omission preserves the current selection, null clears it, and a value replaces it. Required with `ADVANCED`, and rejected for movies.
    pub monitor_selection: MaybeUndefined<MonitorSelectionInput>,
}

#[derive(InputObject, Clone)]
/// Metadata and acquisition settings for creating a title.
pub struct AddTitleInput {
    /// Display name of the title.
    pub name: String,
    /// Media facet, such as movie, series, or anime.
    pub facet: MediaFacetValue,
    /// Library identity receiving the title; null lets the server resolve the default behavior.
    pub library_id: Option<ID>,
    /// Whether the title starts monitored.
    pub monitored: bool,
    /// Tag values attached to the title.
    pub tags: Vec<String>,
    /// Optional title settings to apply at creation.
    pub options: Option<TitleOptionsInput>,
    /// External provider identifiers for the title.
    pub external_ids: Option<Vec<ExternalIdInput>>,
    /// SMG canonical movie title ID, when supplied by metadata search.
    pub smg_id: Option<i64>,
    /// TVDB title ID, when supplied by metadata search.
    pub tvdb_id: Option<String>,
    /// TMDB title ID, when supplied by metadata search.
    pub tmdb_id: Option<i64>,
    /// IMDb title ID, when supplied by metadata search.
    pub imdb_id: Option<String>,
    /// Download source locator, such as an NZB URL or magnet URI, used when queuing the title.
    pub source_hint: Option<String>,
    /// Optional source category for the title.
    pub source_kind: Option<DownloadSourceKindValue>,
    /// Optional source release title.
    pub source_title: Option<String>,
    /// Optional minimum availability value used by acquisition logic.
    pub min_availability: Option<String>,
    // Non-artwork metadata fields supplied from the search result.
    // Poster and fanart URLs are sourced from server-side SMG metadata.
    /// Release year when known.
    pub year: Option<i32>,
    /// Plot summary when known.
    pub overview: Option<String>,
    /// Sort key for title ordering.
    pub sort_title: Option<String>,
    /// URL-safe title slug.
    pub slug: Option<String>,
    /// Runtime in minutes.
    pub runtime_minutes: Option<i32>,
    /// Metadata language code.
    pub language: Option<String>,
    /// Provider content-status label.
    pub content_status: Option<String>,
}

#[derive(InputObject)]
/// Candidate mappings selected from a manual-import preview.
pub struct QueueManualImportInput {
    /// Manual-import selection identity.
    pub selection_id: ID,
    /// File mappings to enqueue for import.
    pub files: Vec<ManualImportCandidateMappingInput>,
}

#[derive(InputObject)]
/// Download identity used to build a manual-import preview.
pub struct BeginManualImportSelectionInput {
    /// Configured download-client identity.
    pub client_id: ID,
    /// Download-client provider type.
    pub client_type: String,
    /// Provider-specific download item identity.
    pub download_client_item_id: String,
    /// Title identity used to suggest import targets.
    pub title_id: ID,
    /// Explicitly extract an archive-only download before building the preview.
    pub extract_archives: Option<bool>,
}

#[derive(InputObject, Clone)]
/// Scope and mode for previewing media file renames.
pub struct MediaRenamePreviewInput {
    /// Facet whose paths are previewed.
    pub facet: MediaFacetValue,
    /// Optional title identity; null previews the full facet scope.
    pub title_id: Option<ID>,
    /// Whether to calculate changes without applying them; omission uses the resolver default.
    pub dry_run: Option<bool>,
    /// Whether to return only the items counted by `renamable`; counts and fingerprint still describe the whole plan.
    pub renamable_only: Option<bool>,
    /// Maximum number of `items` returned; counts and fingerprint still describe the whole plan.
    pub max_items: Option<i32>,
}

#[derive(InputObject, Clone)]
/// Titles whose files should be renamed in the background.
pub struct RenameTitlesInput {
    /// Facet shared by every requested title.
    pub facet: MediaFacetValue,
    /// Titles to rename; each is locked until the job finishes with it.
    pub title_ids: Vec<ID>,
}

#[derive(SimpleObject, Clone)]
/// Accepted rename work and the job run tracking it.
pub struct RenameTitlesPayload {
    /// Background job run tracking the rename work.
    pub job_run: JobRunPayload,
    /// Title IDs accepted for processing.
    pub accepted_title_ids: Vec<ID>,
}

#[derive(InputObject, Clone)]
/// Scope and mode for previewing renames across several titles at once.
pub struct MediaRenamePreviewBulkInput {
    /// Facet shared by every requested title.
    pub facet: MediaFacetValue,
    /// Titles whose rename plans are returned, in the order supplied.
    pub title_ids: Vec<ID>,
    /// Whether to return only the items counted by `renamable`; counts and fingerprint still describe the whole plan.
    pub renamable_only: Option<bool>,
    /// Maximum number of `items` returned across all plans; counts and fingerprints still describe the whole plan.
    pub max_items: Option<i32>,
}

#[derive(InputObject, Clone)]
/// Idempotent request to apply a title's rename plan.
pub struct MediaRenameApplyInput {
    /// Facet containing the title.
    pub facet: MediaFacetValue,
    /// Title identity whose rename plan is applied.
    pub title_id: ID,
    /// Preview fingerprint required to ensure the plan is current.
    pub fingerprint: String,
    /// Optional caller key preventing duplicate application of the same request.
    pub idempotency_key: Option<String>,
}

#[derive(InputObject, Clone)]
/// Idempotent request to apply a bulk rename plan.
pub struct MediaRenameBulkApplyInput {
    /// Facet containing the titles.
    pub facet: MediaFacetValue,
    /// Preview fingerprint required to ensure the plan is current.
    pub fingerprint: String,
    /// Optional caller key preventing duplicate application of the same request.
    pub idempotency_key: Option<String>,
}

#[derive(InputObject)]
/// Destructive deletion request for one title.
pub struct DeleteTitleInput {
    /// Title identity to delete.
    pub title_id: ID,
    /// Whether associated media files are removed from disk.
    pub delete_files_on_disk: Option<bool>,
    /// Preview fingerprint required to confirm the current deletion target.
    pub preview_fingerprint: Option<String>,
    /// Required typed confirmation for destructive deletion.
    pub typed_confirmation: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// Identity returned after deleting a title.
pub struct DeleteTitlePayload {
    /// Deleted title identity.
    pub id: async_graphql::ID,
}

#[derive(InputObject)]
/// Destructive deletion request for multiple titles.
pub struct DeleteTitlesInput {
    /// Title deletion items with per-title preview fingerprints.
    pub items: Vec<DeleteTitlesItemInput>,
    /// Whether associated media files are removed from disk.
    pub delete_files_on_disk: Option<bool>,
    /// Required typed confirmation for destructive deletion.
    pub typed_confirmation: Option<String>,
}

#[derive(InputObject)]
/// One title identity and its preview fingerprint for bulk deletion.
pub struct DeleteTitlesItemInput {
    /// Title identity to delete.
    pub title_id: ID,
    /// Preview fingerprint required to confirm this title's current deletion target.
    pub preview_fingerprint: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// Identity returned after clearing a title release blocklist entry.
pub struct ClearTitleReleaseBlocklistEntryPayload {
    /// Cleared release-blocklist entry identity.
    pub id: async_graphql::ID,
}

#[derive(InputObject)]
/// Input for generating deletion previews for selected titles.
pub struct DeleteTitlesPreviewInput {
    /// Title identities to include in the preview.
    pub title_ids: Vec<ID>,
}

#[derive(InputObject)]
/// Input for previewing deletion of the media files of selected episodes.
pub struct DeleteEpisodeFilesPreviewInput {
    /// Title identity owning the episodes.
    pub title_id: ID,
    /// Episode identities whose media files would be deleted.
    pub episode_ids: Vec<ID>,
}

#[derive(InputObject)]
/// Destructive deletion request for the media files of selected episodes.
pub struct DeleteEpisodeFilesInput {
    /// Title identity owning the episodes.
    pub title_id: ID,
    /// Episode identities whose media files are deleted.
    pub episode_ids: Vec<ID>,
    /// Whether the files are removed from disk.
    pub delete_from_disk: Option<bool>,
    /// Aggregate preview fingerprint required to confirm the current deletion target.
    pub preview_fingerprint: Option<String>,
    /// Required typed confirmation for large destructive deletions.
    pub typed_confirmation: Option<String>,
}

#[derive(InputObject)]
/// Monitoring state change for a title.
pub struct SetTitleMonitoredInput {
    /// Title identity to update.
    pub title_id: ID,
    /// Whether the title is monitored.
    pub monitored: bool,
}

#[derive(InputObject)]
/// Patch for title metadata and settings.
pub struct UpdateTitleInput {
    /// Title identity to patch.
    pub title_id: ID,
    /// Replacement title name; omission preserves it.
    pub name: Option<String>,
    /// Replacement facet; omission preserves it.
    pub facet: Option<MediaFacetValue>,
    /// Replacement tag list; omission preserves it and an empty list clears tags.
    pub tags: Option<Vec<String>>,
    /// Optional title settings patch.
    pub options: Option<TitleOptionsInput>,
}

#[derive(InputObject)]
/// User-tag additions and removals applied across a set of titles.
pub struct UpdateTitleTagsInput {
    /// Titles to patch. Every title's library is checked before the first write, so a set containing one title the caller cannot manage changes nothing.
    pub title_ids: Vec<ID>,
    /// Labels to add. Each must already be defined in the title-tag registry, and reserved `scryer:` entries are rejected.
    pub add: Option<Vec<String>>,
    /// Labels to remove. A label the registry no longer defines may still be removed, so a deleted tag can always be cleared off a title.
    pub remove: Option<Vec<String>>,
}

#[derive(InputObject)]
/// User-tag additions and removals applied across a set of series movies.
pub struct UpdateSeriesMovieTagsInput {
    /// Series-movie links to patch. Each link's series is checked for title-management rights before the first write, so a set spanning a library the caller cannot manage changes nothing.
    pub series_movie_link_ids: Vec<ID>,
    /// Labels to add. Each must already be defined in the title-tag registry, and reserved `scryer:` entries are rejected.
    pub add: Option<Vec<String>>,
    /// Labels to remove. A label the registry no longer defines may still be removed, so a deleted tag can always be cleared off a series movie.
    pub remove: Option<Vec<String>>,
}

#[derive(InputObject)]
/// Primary-file assignment for a movie title.
pub struct SetPrimaryMovieFileInput {
    /// Movie title identity.
    pub title_id: ID,
    /// Media-file identity to make primary.
    pub file_id: ID,
}

#[derive(InputObject)]
/// External metadata identity used to repair a title match.
pub struct FixTitleMatchInput {
    /// Title identity to rematch.
    pub title_id: ID,
    /// TVDB identity to associate with the title. Required for Series and Anime.
    pub tvdb_id: Option<String>,
    /// SMG canonical movie title ID to associate with a Movie title.
    pub smg_id: Option<i64>,
}

#[derive(InputObject, Clone)]
/// Monitoring state change for a collection.
pub struct SetCollectionMonitoredInput {
    /// Collection identity to update.
    pub collection_id: ID,
    /// Whether the collection is monitored.
    pub monitored: bool,
}

#[derive(InputObject, Clone)]
/// Monitoring state change for an episode.
pub struct SetEpisodeMonitoredInput {
    /// Episode identity to update.
    pub episode_id: ID,
    /// Whether the episode is monitored.
    pub monitored: bool,
}

#[derive(InputObject, Clone)]
/// Monitoring state change for a series-movie link.
pub struct SetSeriesMovieMonitoredInput {
    /// Series-movie link identity to update.
    pub series_movie_link_id: ID,
    /// Whether the linked movie is monitored.
    pub monitored: bool,
}

#[derive(InputObject, Clone)]
/// New library root path and default marker.
pub struct CreateLibraryRootInput {
    /// Absolute filesystem path for the root.
    pub path: String,
    /// Whether this root is the library default.
    pub is_default: bool,
}

#[derive(InputObject, Clone)]
/// Replacement library root path and default marker.
pub struct UpdateLibraryRootInput {
    /// Absolute filesystem path for the root.
    pub path: String,
    /// Whether this root is the library default.
    pub is_default: bool,
}

#[derive(InputObject, Clone)]
/// New media library definition.
pub struct CreateLibraryInput {
    /// Media facet stored in the library.
    pub facet: MediaFacetValue,
    /// Library display name.
    pub name: String,
    /// Root paths owned by the library.
    pub roots: Vec<CreateLibraryRootInput>,
    /// Optional library acquisition and import settings.
    pub settings: Option<LibrarySettingsInput>,
}

#[derive(InputObject, Clone)]
/// Patch for an existing media library.
pub struct UpdateLibraryInput {
    /// Library identity to patch.
    pub library_id: ID,
    /// Replacement display name; omission preserves it.
    pub name: Option<String>,
    /// Replacement root list; omission preserves roots and an empty list clears them when valid.
    pub roots: Option<Vec<UpdateLibraryRootInput>>,
    /// Optional replacement library settings.
    pub settings: Option<LibrarySettingsInput>,
}

#[derive(InputObject, Clone)]
/// Acquisition, import, routing, and filesystem settings for a library.
pub struct LibrarySettingsInput {
    /// Required audio-language codes; use `original` to resolve per title.
    pub required_audio_languages: Option<Vec<String>>,
    /// Metadata language override; null inherits the global default.
    pub metadata_language: Option<String>,
    /// Whether episodic titles use season folders; null inherits the facet setting.
    pub use_season_folders: Option<bool>,
    /// Default quality profile identity.
    pub quality_profile_id: Option<ID>,
    /// Quality profile identities allowed for requests.
    pub request_quality_profile_ids: Option<Vec<ID>>,
    /// Scoring persona applied by default.
    pub scoring_persona: Option<ScoringPersonaValue>,
    /// Filler monitoring policy.
    pub filler_policy: Option<FillerPolicyValue>,
    /// Recap monitoring policy.
    pub recap_policy: Option<RecapPolicyValue>,
    /// Whether specials are monitored.
    pub monitor_specials: Option<bool>,
    /// Whether inter-season movies are monitored.
    pub inter_season_movies: Option<bool>,
    /// Whether filler movies are monitored.
    pub monitor_filler_movies: Option<bool>,
    /// Whether NFO metadata is written during import.
    pub nfo_write_on_import: Option<bool>,
    /// Whether Plex match metadata is written during import.
    pub plexmatch_write_on_import: Option<bool>,
    /// Import mode used for library files.
    pub import_mode: Option<ImportModeValue>,
    /// Whether Linux ownership and mode changes are applied.
    pub set_permissions_linux: Option<bool>,
    /// File chmod mode in numeric or accepted symbolic notation.
    pub file_chmod: Option<String>,
    /// Folder chmod mode in numeric or accepted symbolic notation.
    pub folder_chmod: Option<String>,
    /// Unix group name applied when permissions are set.
    pub chown_group: Option<String>,
    /// Indexer routing rules for this library.
    pub indexer_routing: Option<Vec<IndexerRoutingEntryInput>>,
    /// Download-client routing rules for this library.
    pub download_client_routing: Option<Vec<DownloadClientRoutingEntryInput>>,
}

#[derive(SimpleObject, Clone)]
/// Identity returned after deleting a library.
pub struct DeleteLibraryPayload {
    /// Deleted library identity.
    pub id: async_graphql::ID,
}

#[derive(InputObject)]
/// Destructive deletion request for one media file.
pub struct DeleteMediaFileInput {
    /// Media-file identity to delete.
    pub file_id: ID,
    /// Whether the file is removed from disk.
    pub delete_from_disk: Option<bool>,
    /// Preview fingerprint required to confirm the current deletion target.
    pub preview_fingerprint: Option<String>,
    /// Required typed confirmation for destructive deletion.
    pub typed_confirmation: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// Result of a media-file deletion request.
pub struct DeleteMediaFilePayload {
    /// Deleted media-file identity.
    pub id: async_graphql::ID,
    /// Background job accepted to complete deletion and related cleanup.
    pub job_run: JobRunPayload,
}

/// How a candidate folder relates to the title being edited.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum FolderMatchOwnershipValue {
    /// No title in the library claims the folder.
    Unowned,
    /// The title being edited already owns it; selecting it is a no-op.
    OwnedByThisTitle,
    /// Another title owns it; it is never taken silently.
    OwnedByAnotherTitle,
}

/// How the user chose to settle a candidate folder's ownership.
#[derive(Enum, Copy, Clone, Eq, PartialEq, Default)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum FolderMatchResolutionValue {
    /// Claim an unowned folder. The default, and rejected against an owned
    /// folder so a conflict never resolves itself.
    #[default]
    Assign,
    /// Trade folders with the current owner.
    Swap,
    /// Take the folder; the former owner becomes unmatched and needs repair.
    TakeOver,
}

/// What a folder-match correction actually did.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum FolderMatchOutcomeValue {
    /// The title already owned the folder; nothing was submitted.
    AlreadyOwned,
    /// An unowned folder became the title's folder.
    Assigned,
    /// Two titles traded folders.
    Swapped,
    /// The folder changed hands and the former owner is now unmatched.
    TakenOver,
}

#[derive(SimpleObject, Clone)]
/// A title taking part in a folder-match correction.
pub struct FolderMatchTitleRefPayload {
    /// Title identity.
    pub id: ID,
    /// Title display name.
    pub name: String,
    /// Folder this title owns, or null when it owns none.
    pub folder_path: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// A title left without a folder by a takeover, and how it surfaces for repair.
pub struct DisplacedTitleRepairPayload {
    /// Displaced title identity.
    pub id: ID,
    /// Displaced title display name.
    pub name: String,
    /// Folder the displaced title no longer owns.
    pub previous_folder_path: String,
    /// Reason recorded on its unmatched-discovery item.
    pub repair_reason_code: String,
}

#[derive(SimpleObject, Clone)]
/// Read-only description of a proposed folder-match correction.
pub struct ChangeTitleFolderPreviewPayload {
    /// The title being edited.
    pub title: FolderMatchTitleRefPayload,
    /// Media facet of the title being edited.
    pub facet: MediaFacetValue,
    /// Library that owns the title and every candidate folder.
    pub library_id: ID,
    /// Library display name.
    pub library_name: String,
    /// Root containing the title's current folder, or null when it owns none.
    pub current_root_id: Option<ID>,
    /// Path of the root containing the title's current folder.
    pub current_root_path: Option<String>,
    /// Candidate folder, normalized to its stored form.
    pub selected_folder_path: String,
    /// Root containing the candidate folder; always a root of the title's library.
    pub selected_root_id: ID,
    /// Path of the root containing the candidate folder.
    pub selected_root_path: String,
    /// How the candidate folder relates to the title being edited.
    pub ownership: FolderMatchOwnershipValue,
    /// The other title holding the candidate folder, or null when unowned.
    pub current_owner: Option<FolderMatchTitleRefPayload>,
    /// Tracked media rows the title currently has inside its existing folder.
    pub current_folder_tracked_media_count: i32,
    /// Tracked media rows inside the candidate folder, counted across the title
    /// being edited and the candidate folder's owner.
    pub selected_folder_tracked_media_count: i32,
    /// Always false: correcting a folder match never moves file content.
    pub files_will_move: bool,
    /// Whether the title already owns the candidate folder, making this a no-op.
    pub no_op: bool,
    /// Resolutions this exact selection admits.
    pub available_resolutions: Vec<FolderMatchResolutionValue>,
}

#[derive(SimpleObject, Clone)]
/// Result of applying a folder-match correction.
pub struct ChangeTitleFolderPayload {
    /// What the correction actually did.
    pub outcome: FolderMatchOutcomeValue,
    /// The edited title after the change.
    pub title: FolderMatchTitleRefPayload,
    /// Folder the edited title owned before, or null when it owned none.
    pub previous_folder_path: Option<String>,
    /// Media associations detached because a title gave up a folder.
    pub detached_media_file_count: i32,
    /// Rescan of the edited title's new folder, or null for a no-op.
    pub scan: Option<LibraryScanSummaryPayload>,
    /// The other title after a swap, with the folder it received.
    pub swapped_title: Option<FolderMatchTitleRefPayload>,
    /// Rescan of the swapped title's new folder.
    pub swapped_title_scan: Option<LibraryScanSummaryPayload>,
    /// The title left unmatched by a takeover, or null when none was displaced.
    pub displaced_title: Option<DisplacedTitleRepairPayload>,
}

#[derive(InputObject, Clone)]
/// Request describing a proposed folder-match correction; changes nothing.
pub struct ChangeTitleFolderPreviewInput {
    /// Title whose folder match would change.
    pub title_id: ID,
    /// Candidate folder, which must be inside one of the title's library roots.
    pub folder_path: String,
}

#[derive(InputObject, Clone)]
/// Request applying a folder-match correction.
pub struct ApplyTitleFolderChangeInput {
    /// Title whose folder match changes.
    pub title_id: ID,
    /// Chosen folder, which must be inside one of the title's library roots.
    pub folder_path: String,
    /// How to settle ownership. Defaults to ASSIGN, which is refused when
    /// another title already owns the folder.
    pub resolution: Option<FolderMatchResolutionValue>,
}

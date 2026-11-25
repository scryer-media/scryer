use super::{JobRunStatusValue, LibraryScanPhaseProgressPayload, MediaFacetValue};
use async_graphql::{Enum, ID, InputObject, Json, SimpleObject};
use chrono::{DateTime, Utc};

// ── External Import (Sonarr/Radarr) ────────────────────────────────────────

#[derive(InputObject, Clone)]
/// Credentials for connecting to one external service instance.
pub struct ExternalImportConnectionInput {
    /// Base URL of the external service.
    pub base_url: String,
    /// API key used only for the connection operation and secret storage.
    pub api_key: String,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Supported external title-management service source.
pub enum ExternalArrSourceKind {
    /// Sonarr source.
    Sonarr,
    /// Radarr source.
    Radarr,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// External service kind accepted by connection validation and warmup.
pub enum ExternalImportConnectionKind {
    /// Sonarr service.
    Sonarr,
    /// Radarr service.
    Radarr,
    /// Prowlarr service.
    Prowlarr,
}

#[derive(InputObject)]
/// External service kind and credentials to validate.
pub struct ValidateExternalImportConnectionInput {
    /// External service kind.
    pub kind: ExternalImportConnectionKind,
    /// Base URL and API key to validate.
    pub connection: ExternalImportConnectionInput,
}

#[derive(InputObject)]
/// API key for one external service instance in an import setup draft.
pub struct ExternalImportSetupInstanceApiKeyInput {
    /// External instance ID used to identify the draft entry.
    pub instance_id: ID,
    /// External service kind.
    pub kind: ExternalImportConnectionKind,
    /// API key stored in the draft.
    pub api_key: String,
}

#[derive(InputObject)]
/// Secret values collected for an external import setup.
pub struct SaveExternalImportSetupSecretDraftInput {
    /// External instance API keys keyed by instance ID.
    pub instance_api_keys: Vec<ExternalImportSetupInstanceApiKeyInput>,
    /// Replacement API keys keyed by download-client deduplication key.
    pub download_client_api_key_overrides: Vec<DownloadClientApiKeyOverrideInput>,
    /// Replacement passwords keyed by download-client deduplication key.
    pub download_client_password_overrides: Vec<DownloadClientPasswordOverrideInput>,
    /// Replacement API keys keyed by indexer deduplication key.
    pub indexer_api_key_overrides: Vec<IndexerApiKeyOverrideInput>,
}

#[derive(SimpleObject, Clone)]
/// API key entry returned from an external import secret draft.
pub struct ExternalImportSetupInstanceApiKeyPayload {
    /// External instance ID.
    pub instance_id: ID,
    /// External service kind.
    pub kind: ExternalImportConnectionKind,
    /// Stored API key value for the draft.
    pub api_key: String,
}

#[derive(SimpleObject, Clone)]
/// Download-client or indexer API key stored in an import draft.
pub struct ExternalImportSetupApiKeyOverridePayload {
    /// Stable candidate identity used for deduplication.
    pub dedup_key: String,
    /// Stored API key value for the draft.
    pub api_key: String,
}

#[derive(SimpleObject, Clone)]
/// Download-client password stored in an import draft.
pub struct ExternalImportSetupPasswordOverridePayload {
    /// Stable candidate identity used for deduplication.
    pub dedup_key: String,
    /// Stored password value for the draft.
    pub password: String,
}

#[derive(SimpleObject, Clone)]
/// Secret values held in an external import setup draft.
pub struct ExternalImportSetupSecretDraftPayload {
    /// External instance API keys in the draft.
    pub instance_api_keys: Vec<ExternalImportSetupInstanceApiKeyPayload>,
    /// Download-client API key overrides in the draft.
    pub download_client_api_key_overrides: Vec<ExternalImportSetupApiKeyOverridePayload>,
    /// Download-client password overrides in the draft.
    pub download_client_password_overrides: Vec<ExternalImportSetupPasswordOverridePayload>,
    /// Indexer API key overrides in the draft.
    pub indexer_api_key_overrides: Vec<ExternalImportSetupApiKeyOverridePayload>,
    /// Last update time in UTC.
    pub updated_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
/// Ownership and existence status for the current user's import secret draft.
pub struct ExternalImportSetupSecretDraftStatusPayload {
    /// Whether a secret draft exists.
    pub has_draft: bool,
    /// Whether the draft belongs to the current user.
    pub owned_by_current_user: bool,
    /// Draft update time in UTC, or null when no draft exists.
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(SimpleObject, Clone)]
/// Result of saving an external import secret draft.
pub struct SaveExternalImportSetupSecretDraftPayload {
    /// Whether another user's existing draft was replaced.
    pub overwrote_another_user_draft: bool,
    /// Save time in UTC.
    pub updated_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
/// Result of clearing the current user's external import secret draft.
pub struct ClearExternalImportSetupSecretDraftPayload {
    /// False when there was no draft owned by the caller to clear.
    pub cleared: bool,
}

#[derive(InputObject)]
/// External source and credentials for an asynchronous warmup session.
pub struct StartExternalImportArrSourceWarmupInput {
    /// External source kind.
    pub kind: ExternalArrSourceKind,
    /// Connection URL and API key.
    pub connection: ExternalImportConnectionInput,
}

#[derive(InputObject)]
/// Prowlarr credentials for an asynchronous warmup session.
pub struct StartExternalImportProwlarrWarmupInput {
    /// Connection URL and API key.
    pub connection: ExternalImportConnectionInput,
}

#[derive(InputObject)]
/// Warmup session IDs whose aggregate progress should be calculated.
pub struct ExternalImportAggregateWarmupProgressInput {
    /// Warmup session IDs whose progress should be aggregated.
    pub source_warmup_session_ids: Vec<ID>,
}

#[derive(InputObject)]
/// Warmup sessions and optional Prowlarr connection used to build an import preview.
pub struct PreviewExternalImportInput {
    /// Source warmup session IDs to include.
    pub source_warmup_session_ids: Vec<ID>,
    /// Prowlarr warmup session ID, when one was started.
    pub prowlarr_warmup_session_id: Option<ID>,
    #[graphql(deprecation = "use prowlarrWarmupSessionId")]
    /// Deprecated direct Prowlarr connection retained for compatibility.
    pub prowlarr: Option<ExternalImportConnectionInput>,
}

#[derive(InputObject)]
/// API key override for a masked download-client candidate.
pub struct DownloadClientApiKeyOverrideInput {
    /// Stable candidate identity used to apply the override.
    pub dedup_key: String,
    /// Replacement API key.
    pub api_key: String,
}

#[derive(InputObject)]
/// Password override for a masked download-client candidate.
pub struct DownloadClientPasswordOverrideInput {
    /// Stable candidate identity used to apply the override.
    pub dedup_key: String,
    /// Replacement password.
    pub password: String,
}

#[derive(InputObject)]
/// API key override for a masked or conflicting indexer candidate.
pub struct IndexerApiKeyOverrideInput {
    /// Stable candidate identity used to apply the override.
    pub dedup_key: String,
    /// Replacement API key.
    pub api_key: String,
}

#[derive(InputObject)]
/// Candidate selections, mappings, and secret overrides for an external import.
pub struct ExecuteExternalImportInput {
    /// Source warmup session IDs whose data is being imported.
    pub source_warmup_session_ids: Vec<ID>,
    /// Optional Prowlarr connection used when no Prowlarr warmup session is available.
    pub prowlarr: Option<ExternalImportConnectionInput>,
    /// Deduplication keys of download clients to create.
    pub selected_download_client_dedup_keys: Vec<String>,
    /// Deduplication keys of indexers to create.
    pub selected_indexer_dedup_keys: Vec<String>,
    /// API key overrides keyed by download-client deduplication key.
    pub download_client_api_key_overrides: Vec<DownloadClientApiKeyOverrideInput>,
    /// Password overrides keyed by download-client deduplication key.
    pub download_client_password_overrides: Vec<DownloadClientPasswordOverrideInput>,
    /// API key overrides keyed by indexer deduplication key.
    pub indexer_api_key_overrides: Vec<IndexerApiKeyOverrideInput>,
}

#[derive(InputObject)]
/// Mapping from an external root folder to a Scryer library facet.
pub struct ExternalImportSourceLibraryMappingInput {
    /// Warmup session that surfaced this root; null identifies a manually added root without a monitored-status snapshot.
    pub source_warmup_session_id: Option<ID>,
    /// External source identity, required when a warmup session is supplied.
    pub source_key: Option<String>,
    /// External source kind, required when a warmup session is supplied.
    pub kind: Option<ExternalArrSourceKind>,
    /// Root path reported by the external source.
    pub arr_root_path: String,
    /// Destination root path in Scryer.
    pub scryer_root_path: String,
    /// Scryer library ID receiving the imported root.
    pub library_id: ID,
    /// Media facet receiving the imported root.
    pub facet: MediaFacetValue,
}

#[derive(InputObject)]
/// Warmup sessions and root mappings to finalize as an external import.
pub struct FinalizeExternalImportInput {
    /// Accepted source import to require resolved before applying library mappings.
    pub source_import_run_id: Option<ID>,
    /// Source warmup session IDs to finalize.
    pub source_warmup_session_ids: Vec<ID>,
    /// Root-to-library mappings selected for import.
    pub mappings: Vec<ExternalImportSourceLibraryMappingInput>,
}

#[derive(SimpleObject, Clone)]
/// Result of canceling an external import monitor warmup.
pub struct CancelExternalImportMonitorWarmupPayload {
    /// Warmup session ID.
    pub session_id: ID,
    /// Whether cancellation changed the session state.
    pub canceled: bool,
}

#[derive(SimpleObject, Clone)]
/// Connection probe result for an external service.
pub struct ExternalImportConnectionValidationPayload {
    /// Service kind that was validated.
    pub kind: ExternalImportConnectionKind,
    /// Normalized service base URL.
    pub base_url: String,
    /// Whether the service accepted the connection credentials.
    pub connected: bool,
    /// Detected service version, or null when unavailable.
    pub version: Option<String>,
    /// Connection failure detail, or null when connected.
    pub error: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// Aggregate title-fetch progress for external source warmups.
pub struct ExternalImportAggregateWarmupProgressPayload {
    /// Aggregate warmup status.
    pub status: ExternalImportMonitorWarmupStatusValue,
    /// Whether the total title count is known.
    pub titles_total_known: bool,
    /// Number of titles fetched.
    pub titles_fetched: i32,
    /// Total titles expected when known.
    pub titles_total: i32,
    /// Failure detail, or null when no error occurred.
    pub error_message: Option<String>,
}

#[derive(Enum, Copy, Clone, Debug, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// External setting inferred or applied during library import.
pub enum ExternalImportLibrarySettingKey {
    /// Whether title and file names are renamed.
    RenameEnabled,
    /// Whether NFO metadata is written during import.
    NfoWriteOnImport,
    /// Whether Plex match metadata is written during import.
    PlexmatchWriteOnImport,
    /// Whether Linux file permissions are set.
    SetPermissionsLinux,
    /// Folder permission mode.
    FolderChmod,
    /// Group assigned to created folders.
    ChownGroup,
    /// Quality profile ID.
    QualityProfileId,
    /// Quality profile IDs used for requests.
    RequestQualityProfileIds,
    /// Whether special episodes are monitored.
    MonitorSpecials,
    /// Title rename template.
    RenameTemplate,
    /// Folder naming template.
    FolderTemplate,
    /// Required audio language codes.
    RequiredAudioLanguages,
}

#[derive(Enum, Copy, Clone, Debug, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Confidence assigned to an inferred external library setting.
pub enum ExternalImportLibrarySettingConfidence {
    /// Strong evidence supports the setting.
    High,
    /// Some evidence supports the setting.
    Medium,
    /// Weak evidence supports the setting.
    Low,
}

#[derive(Enum, Copy, Clone, Debug, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// How an inferred external setting was handled.
pub enum ExternalImportLibrarySettingDisposition {
    /// Applied automatically.
    AutoApplied,
    /// Returned as a suggestion without automatic application.
    Suggested,
    /// Not applied because evidence or compatibility was insufficient.
    Skipped,
}

#[derive(SimpleObject, Clone)]
/// Typed value emitted for an inferred library setting.
pub struct ExternalImportLibrarySettingValuePayload {
    /// Boolean value, or null when the setting is not boolean.
    pub bool_value: Option<bool>,
    /// String value, or null when the setting is not scalar text.
    pub string_value: Option<String>,
    /// String-list value, or null when the setting is not a list.
    pub string_list_value: Option<Vec<String>>,
}

#[derive(SimpleObject, Clone)]
/// Evidence supporting one inferred external library setting.
pub struct ExternalImportLibrarySettingEvidencePayload {
    /// External source identity supplying the evidence.
    pub source_key: String,
    /// External source kind supplying the evidence.
    pub source_kind: ExternalArrSourceKind,
    /// Number of matching source records.
    pub matching_count: i32,
    /// Number of source records considered.
    pub total_count: i32,
    /// Human-readable evidence detail, when available.
    pub detail: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// One setting application decision for a Scryer library facet.
pub struct ExternalImportLibrarySettingApplicationPayload {
    /// Scryer library ID receiving the setting.
    pub library_id: ID,
    /// Media facet receiving the setting.
    pub facet: MediaFacetValue,
    /// Setting being evaluated.
    pub setting: ExternalImportLibrarySettingKey,
    /// Candidate value for the setting.
    pub value: ExternalImportLibrarySettingValuePayload,
    /// Confidence in the candidate value.
    pub confidence: ExternalImportLibrarySettingConfidence,
    /// Whether the value was applied, suggested, or skipped.
    pub disposition: ExternalImportLibrarySettingDisposition,
    /// Source evidence used for the decision.
    pub evidence: Vec<ExternalImportLibrarySettingEvidencePayload>,
    /// Additional reason, or null when no explanation was recorded.
    pub reason: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// Accepted external import finalization and its monitor warmup session.
pub struct FinalizeExternalImportPayload {
    /// Monitor warmup session ID tracking accepted background work.
    pub monitor_warmup_session_id: ID,
    /// Session ID of the background apply; poll `externalImportWarmupStatus`
    /// with it until the apply reaches a terminal status.
    pub finalize_session_id: ID,
    /// Status of the background apply at the moment finalize was accepted.
    pub progress: ExternalImportMonitorWarmupProgressPayload,
}

#[derive(InputObject)]
/// Language used when clearing metadata for a full rehydration.
pub struct RehydrateAllMetadataInput {
    /// Metadata language code, such as `en` or `en-US`.
    pub language: String,
}

#[derive(SimpleObject, Clone)]
/// Result of clearing metadata before background rehydration.
pub struct RehydrateAllMetadataPayload {
    /// Language whose metadata was cleared.
    pub language: String,
    /// Number of titles cleared.
    pub titles_cleared: i64,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Lifecycle state of an external monitor warmup session.
pub enum ExternalImportMonitorWarmupStatusValue {
    /// Session accepted but not started.
    Queued,
    /// Session is actively fetching or building data.
    Running,
    /// Session completed successfully.
    Completed,
    /// Session was canceled before completion.
    Canceled,
    /// Session failed before completion.
    Failed,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Current work phase of an external monitor warmup session.
pub enum ExternalImportMonitorWarmupPhaseValue {
    /// Loading indexer definitions.
    LoadingIndexers,
    /// Loading movie records.
    LoadingMovies,
    /// Loading series records.
    LoadingSeries,
    /// Loading episode records.
    LoadingEpisodes,
    /// Building the imported monitor snapshot.
    BuildingSnapshot,
    /// Warmup data is ready for use.
    Ready,
}

#[derive(SimpleObject, Clone)]
/// Progress and match counts for one external monitor warmup session.
pub struct ExternalImportMonitorWarmupProgressPayload {
    /// Warmup session ID.
    pub session_id: ID,
    /// Warmup lifecycle status.
    pub status: ExternalImportMonitorWarmupStatusValue,
    /// Current warmup phase.
    pub phase: ExternalImportMonitorWarmupPhaseValue,
    /// Session start time in UTC.
    pub started_at: DateTime<Utc>,
    /// Last progress update time in UTC.
    pub updated_at: DateTime<Utc>,
    /// Whether the overall total is known.
    pub overall_total_known: bool,
    /// Overall progress counters.
    pub overall_progress: LibraryScanPhaseProgressPayload,
    /// Whether the movie total is known.
    pub movies_total_known: bool,
    /// Movie loading progress counters.
    pub movies_progress: LibraryScanPhaseProgressPayload,
    /// Whether the series total is known.
    pub series_total_known: bool,
    /// Series loading progress counters.
    pub series_progress: LibraryScanPhaseProgressPayload,
    /// Whether the episode fetch total is known.
    pub episode_fetch_total_known: bool,
    /// Expected episode count when known.
    pub episode_fetch_expected_total: Option<i32>,
    /// Expected monitored episode count when known.
    pub episode_fetch_expected_monitored_total: Option<i32>,
    /// Episode loading progress counters.
    pub episode_fetch_progress: LibraryScanPhaseProgressPayload,
    /// Whether snapshot-build totals are known.
    pub snapshot_build_total_known: bool,
    /// Snapshot-build progress counters.
    pub snapshot_build_progress: LibraryScanPhaseProgressPayload,
    /// Number of matched movies.
    pub matched_movie_count: i32,
    /// Number of matched series.
    pub matched_series_count: i32,
    /// Number of unmatched movies.
    pub unmatched_movie_count: i32,
    /// Number of unmatched series.
    pub unmatched_series_count: i32,
    /// Number of ambiguous movies.
    pub ambiguous_movie_count: i32,
    /// Number of ambiguous series.
    pub ambiguous_series_count: i32,
    /// Failure detail, or null when no error occurred.
    pub error_message: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// External import candidates and source warmup results.
pub struct ExternalImportPreviewPayload {
    /// Whether Prowlarr connected during preview construction.
    pub prowlarr_connected: bool,
    /// Prowlarr version, or null when unavailable.
    pub prowlarr_version: Option<String>,
    /// Prowlarr connection error, or null when connected.
    pub prowlarr_error: Option<String>,
    /// External Arr source sessions included in the preview.
    pub arr_sources: Vec<ExternalImportArrSourcePayload>,
    /// Root folders reported by the sources.
    pub root_folders: Vec<ExternalImportRootFolderPayload>,
    /// Download-client candidates grouped by deduplication identity.
    pub download_clients: Vec<ExternalImportDownloadClientPayload>,
    /// Indexer candidates grouped by deduplication identity.
    pub indexers: Vec<ExternalImportIndexerPayload>,
}

#[derive(SimpleObject, Clone)]
/// Connection and warmup result for one external Arr source.
pub struct ExternalImportArrSourcePayload {
    /// Warmup session ID.
    pub session_id: ID,
    /// Stable source identity.
    pub source_key: String,
    /// External source kind.
    pub kind: ExternalArrSourceKind,
    /// Source base URL.
    pub base_url: String,
    /// Whether the source connection succeeded.
    pub connected: bool,
    /// Source version, or null when unavailable.
    pub version: Option<String>,
    /// Warmup lifecycle status.
    pub status: ExternalImportMonitorWarmupStatusValue,
    /// Connection or warmup error, or null on success.
    pub error: Option<String>,
}

#[derive(SimpleObject, Clone)]
/// Root folder reported by an external Arr source.
pub struct ExternalImportRootFolderPayload {
    /// Warmup session that reported the root.
    pub source_warmup_session_id: ID,
    /// Stable source identity.
    pub source_key: String,
    /// External source kind.
    pub kind: ExternalArrSourceKind,
    /// Root path reported by the source.
    pub arr_root_path: String,
}

#[derive(SimpleObject, Clone)]
/// Download-client candidate discovered during external import.
pub struct ExternalImportDownloadClientPayload {
    /// Source identities that reported this candidate.
    pub source_keys: Vec<String>,
    /// Download-client name.
    pub name: String,
    /// Source implementation name.
    pub implementation: String,
    /// Scryer client type, or null when unsupported.
    pub scryer_client_type: Option<String>,
    /// Hostname or address, when reported.
    pub host: Option<String>,
    /// Port as reported by the external source, which may not be numeric.
    pub port: Option<String>,
    /// Whether the source uses TLS.
    pub use_ssl: bool,
    /// URL base path, when reported.
    pub url_base: Option<String>,
    /// Username, when reported.
    pub username: Option<String>,
    /// Whether an API key was returned by the source.
    pub api_key_present: bool,
    /// Stable identity used to deduplicate this candidate.
    pub dedup_key: String,
    /// Whether Scryer can create this client type.
    pub supported: bool,
    /// Whether an explicit password override is required.
    pub requires_password_override: bool,
}

#[derive(SimpleObject, Clone)]
/// Indexer candidate discovered during external import.
pub struct ExternalImportIndexerPayload {
    /// Source identities that reported this candidate.
    pub source_keys: Vec<String>,
    /// Indexer name.
    pub name: String,
    /// Source implementation name.
    pub implementation: String,
    /// Scryer provider type, or null when unsupported.
    pub scryer_provider_type: Option<String>,
    /// Indexer base URL, when reported.
    pub base_url: Option<String>,
    /// Whether an API key was returned by the source.
    pub api_key_present: bool,
    /// Stable identity used to deduplicate this candidate.
    pub dedup_key: String,
    /// Whether Scryer can create this provider type.
    pub supported: bool,
    /// Number of grouped child indexers.
    pub child_count: i32,
    /// Names of grouped child indexers.
    pub child_names: Vec<String>,
    /// Whether an explicit API key override is required.
    pub requires_api_key_override: bool,
    /// Provider help URL for obtaining an API key, when available.
    pub api_key_help_url: Option<String>,
}

#[derive(SimpleObject)]
/// Durable receipt for one selected source configuration.
pub struct ExternalImportSourceItemPayload {
    /// Stable source identity used for retries and explicit skips.
    pub key: String,
    /// Source display name.
    pub name: String,
    /// Reserved configuration identity, retained across interruption and retry.
    pub config_id: ID,
    /// Configuration kind: indexer or download_client.
    pub kind: String,
    /// Current phase: queued, installing, checking, saving, succeeded, failed, interrupted, or skipped.
    pub state: String,
    /// Stable failure reason code, or null for an item without a failure.
    pub reason: Option<String>,
    /// The server's own failure sentence with URL credentials masked: the fallback for an unrecognized reason and the detail behind SOURCE_IMPORT_ITEM_FAILED.
    pub message: Option<String>,
}

#[derive(SimpleObject)]
/// Authorized progress of an accepted background source import.
pub struct ExternalImportSourceJobPayload {
    /// Durable workflow run identity.
    pub run_id: ID,
    /// Last persisted update time, used to reject stale progress responses.
    pub updated_at: String,
    /// Whether the current attempt still has work scheduled.
    pub active: bool,
    /// Whether every selected source succeeded or was explicitly skipped.
    pub ready: bool,
    /// Receipts for the fixed submitted selections, without credentials.
    pub items: Vec<ExternalImportSourceItemPayload>,
}

// ── External rule import (Sonarr/Radarr custom formats, Maintainerr rule groups) ──

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Application an external rule import reads from.
pub enum ExternalRuleImportSource {
    /// Sonarr custom formats, imported as disabled scoring rule sets.
    Sonarr,
    /// Radarr custom formats, imported as disabled scoring rule sets.
    Radarr,
    /// Maintainerr rule groups, imported as disabled lifecycle rules.
    Maintainerr,
}

#[derive(InputObject, Clone)]
/// Where an external rule import reads from.
pub struct ExternalRuleImportInput {
    /// Application the URL serves.
    pub source: ExternalRuleImportSource,
    /// Base URL of the application, including any URL base or BASE_PATH prefix.
    pub base_url: String,
    /// API key; required for Sonarr and Radarr. Maintainerr has no API authentication, so a key sent with a Maintainerr import is ignored.
    pub api_key: Option<String>,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Step an external rule import has reached.
pub enum ExternalRuleImportPhase {
    /// Accepted and not yet started.
    Queued,
    /// Reading the source.
    Fetching,
    /// Translating what was read.
    Planning,
    /// Saving translated rules.
    Saving,
    /// Done; the report is final.
    Finished,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Outcome of one imported source object.
pub enum ExternalRuleImportItemState {
    /// Planned and not yet saved.
    Pending,
    /// Saved as an exact translation.
    Saved,
    /// Saved, but the translation needs review.
    SavedNeedsReview,
    /// Saved as a placeholder because the source rule could not be translated.
    SavedFailedTranslation,
    /// A rule with this name already exists; nothing was written.
    SkippedExists,
    /// Nothing was saved.
    Failed,
}

#[derive(SimpleObject)]
/// A coded observation about an import run or one of its items.
pub struct ExternalRuleImportFactPayload {
    /// Stable reason code.
    pub code: String,
    /// The server's sentence: the fallback for an unrecognized code.
    pub message: String,
    /// Values the code refers to, such as a settings id or a limit, or null.
    pub details: Option<Json<serde_json::Value>>,
}

#[derive(SimpleObject)]
/// Report line for one source object.
pub struct ExternalRuleImportItemPayload {
    /// Stable identity within the source, such as customformat:12.
    pub key: String,
    /// Name in the source application.
    pub source_name: String,
    /// Name the saved rule received, or null when nothing was saved.
    pub saved_name: Option<String>,
    /// Saved rule set identity, or null when nothing was saved.
    pub rule_set_id: Option<ID>,
    /// Outcome.
    pub state: ExternalRuleImportItemState,
    /// Stable reason code for a skipped, failed or partial outcome, or null.
    pub reason: Option<String>,
    /// The server's failure sentence: the fallback for an unrecognized reason.
    pub message: Option<String>,
    /// Translation observations about this item.
    pub facts: Vec<ExternalRuleImportFactPayload>,
}

#[derive(SimpleObject)]
/// Number of report lines in each state.
pub struct ExternalRuleImportCountsPayload {
    /// Every report line.
    pub total: i32,
    /// Lines not yet saved.
    pub pending: i32,
    /// Lines saved as exact translations.
    pub saved: i32,
    /// Lines saved but needing review.
    pub saved_needs_review: i32,
    /// Lines saved as untranslated placeholders.
    pub saved_failed_translation: i32,
    /// Lines skipped because the rule already exists.
    pub skipped_exists: i32,
    /// Lines where nothing was saved.
    pub failed: i32,
}

#[derive(SimpleObject)]
/// An external rule import run and its per-item report. Credentials are never returned.
pub struct ExternalRuleImportStatusPayload {
    /// Job run identity.
    pub run_id: ID,
    /// Job run status.
    pub status: JobRunStatusValue,
    /// Whether the run is still working.
    pub active: bool,
    /// Report revision; a higher revision is newer.
    pub revision: i64,
    /// Last persisted update time.
    pub updated_at: DateTime<Utc>,
    /// Application read.
    pub source: ExternalRuleImportSource,
    /// Base URL read.
    pub base_url: String,
    /// Step reached.
    pub phase: ExternalRuleImportPhase,
    /// Stable reason the run stopped short, or null.
    pub reason: Option<String>,
    /// The server's sentence for that reason: the fallback for an unrecognized code.
    pub message: Option<String>,
    /// Per-state counts.
    pub counts: ExternalRuleImportCountsPayload,
    /// One line per source object, at most 1000.
    pub items: Vec<ExternalRuleImportItemPayload>,
    /// Observations about the fetch that belong to no single item.
    pub facts: Vec<ExternalRuleImportFactPayload>,
}

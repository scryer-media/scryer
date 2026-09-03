mod acquisition;
mod api_keys;
pub mod application_upgrade;
mod authorization;
mod catalog;
pub mod challenge_solver;
mod contracts;
mod discovery;
mod download_client_config;
mod download_client_path_mappings;
mod download_identity;

pub(crate) const DOWNLOAD_QUEUE_RECENT_COMPLETED_LIMIT: usize = 300;

/// Widened completed-history bound used when a stuck download misses the recent
/// window.
///
/// Deliberately larger than [`DOWNLOAD_QUEUE_RECENT_COMPLETED_LIMIT`]: a
/// download stranded for a while sinks below the recent cut-off, and this is
/// the retry path whose whole job is to find it again. Clients that bound the
/// fetch server-side (SABnzbd pages by limit) pay proportionally; clients that
/// cannot (nzbget returns its whole history regardless) pay nothing extra,
/// since the limit only decides how much of an already-fetched response is
/// retained.
pub(crate) const DOWNLOAD_QUEUE_STUCK_COMPLETED_LOOKUP_LIMIT: usize = 1_000;
pub use download_identity::{
    AcceptedDownloadIdentityInput, DOWNLOAD_ID_PARAMETER, ObservedDownloadIdentityInput,
    accepted_download_submission_identity, download_id_from_info_hash,
    download_submission_identity_is_empty, normalize_torrent_info_hash, observed_download_identity,
};
mod events;
pub mod external_import;
pub mod file_source_signature;
mod folder_ownership;
pub mod fs_integrity;
pub mod fs_safety;
mod health;
mod helpers;
mod image_proxy;
mod import;
mod indexer_category;
mod indexer_errors;
pub use indexer_category::{
    CATEGORY_MISMATCH_CODE, IndexerCategoryFamily, NZB_HEAD_PROBE_BYTES, enforce_nzb_category_gate,
    indexer_category_contradicts_facet, indexer_category_family, nzb_head_category,
};
mod integration;
mod jobs;
mod library;
#[path = "library/scan/scanner.rs"]
mod library_scan;
#[path = "library/scan/coordinator.rs"]
mod library_scan_coordinator;
#[path = "library/scan/helpers.rs"]
mod library_scan_helpers;
#[path = "library/scan/metadata.rs"]
mod library_scan_metadata;
#[path = "library/scan/progress.rs"]
mod library_scan_progress;
#[path = "library/scan/titles.rs"]
mod library_scan_titles;
#[path = "library/scan/unmatched.rs"]
mod library_scan_unmatched;
mod media;
mod media_requests;
mod media_servers;
mod notifications;
mod null_repositories;
mod oauth;
pub mod persisted_records;
mod plugins;
mod polling_worker;
mod ports;
pub use ports::{CatalogOwnedExternalIdRecord, CatalogOwnedTitleRecord, TitleOptionsPatch};
mod quality;
mod rate_limit_signal;
mod rules;
mod scheduler;
mod security;
mod services;
mod settings;
pub mod stored_paths;
pub mod subtitles;
pub mod testing;
mod types;
pub mod upstream_scheduler;

pub(crate) use acquisition::acquisition as acquisition_workflow;
pub(crate) use acquisition::admission;
pub(crate) use acquisition::coverage as acquisition_coverage;
pub(crate) use acquisition::decision_helpers as acquisition_decision_helpers;
pub(crate) use acquisition::delay_profile;
pub(crate) use acquisition::policy as acquisition_policy;
pub(crate) use acquisition::release_search as acquisition_release_search;
pub(crate) use acquisition::rss as app_usecase_rss;
pub(crate) use acquisition::search_queries as acquisition_search_queries;
pub(crate) use catalog::catalog as catalog_workflow;
pub(crate) use catalog::facets::handler as facet_handler;
pub(crate) use catalog::helpers as catalog_helpers;
pub(crate) use catalog::release_search as app_usecase_discovery;
pub(crate) use events::activity;
pub(crate) use events::domain_events;
pub(crate) use events::event_views;
pub(crate) use import::archive_extractor;
pub(crate) use import::checks as import_checks;
pub(crate) use import::decide as import_decide;
pub(crate) use import::import as import_workflow;
pub(crate) use import::parameters as import_parameters;
pub(crate) use import::post_download_gate;
pub(crate) use import::seeding_gate;
pub(crate) use import::title_resolution as import_title_resolution;
pub(crate) use integration::integration as app_usecase_integration;
pub(crate) use library::discovery as library_discovery;
pub(crate) use library::filename_parser as library_filename_parser;
pub(crate) use library::nfo;
pub(crate) use library::rename as library_rename;
pub(crate) use library::title_matching;
pub(crate) use media::audio_requirements;
pub(crate) use media::language_data as media_language_data;
pub(crate) use quality::canonical as canonical_scoring;
pub(crate) use quality::profile as quality_profile;
pub(crate) use quality::release_group_db;
pub(crate) use quality::release_parser;
pub(crate) use quality::scoring_weights;
pub(crate) use quality::trash_scores;
pub(crate) use rules::user_rule_input;

pub use download_client_config::resolve_download_client_base_url_from_config_json;
pub use import::completed_download as completed_download_handler;
pub use ports::{
    CatalogDiscoveryCandidatesRecord, CatalogDiscoveryGroup, CatalogDiscoveryGroupKind,
    CatalogDiscoveryQuery, CatalogDiscoveryResult, CatalogDiscoverySectionCandidatesRecord,
    CatalogDiscoverySurface, DISCOVERY_DEFAULT_SCOPE_KEY, DiscoveryCanonicalTagFilterOption,
    DiscoveryContextIncrementalCommit, DiscoveryContextSnapshotCommit, DiscoveryExternalIdRecord,
    DiscoveryFacetRecord, DiscoveryHomeCandidate, DiscoveryHomeFilterOptions, DiscoveryHomeFilters,
    DiscoveryHomeQuery, DiscoveryHomeResult, DiscoveryHomeSectionCandidatesRecord,
    DiscoveryItemDetailQuery, DiscoveryItemLibraryProvenanceRecord, DiscoveryItemRecord,
    DiscoveryItemsPageRecord, DiscoveryItemsQuery, DiscoveryItemsResult,
    DiscoveryItemsStorageQuery, DiscoveryPendingContextChangeRecord, DiscoveryPruneReport,
    DiscoveryPublicFeedCommit, DiscoveryRankComponentRecord, DiscoveryRepository,
    DiscoverySectionItemsRecord, DiscoverySectionRecord, DiscoverySectionResult,
    DiscoverySourceTagRecord, DiscoverySubmittedSubjectRecord, DiscoverySyncRunRecord,
    DiscoverySyncStateRecord, DiscoverySyncStatus, EpisodeImageUrlUpdate,
    MediaRequestQualityProfileReferenceCounts, MediaRequestResolution,
    MediaRequestResolutionResult, MediaRequestSubmissionResult, MediaRequestUpdateResult,
    SeriesMovieExternalIdLookupMatch, SubtitleSyncClient, SubtitleSyncJob, TitleArtworkUrlUpdate,
    TitleDeletePreviewInfo, TitleExternalIdLookup, TitleExternalIdLookupMatch,
    UserUiSettingsRepository,
};
pub(crate) mod normalize;
pub use api_keys::{
    API_KEY_PREFIX, ApiKeyAuthentication, ApiKeyExpiryPreset, ApiKeySummary, CreateApiKey,
    CreatedApiKey, DevelopmentApiKeySeed, parse_api_key,
};
pub use events::retention::user_facing_domain_event_types;
pub use import::failed_download as failed_download_handler;
pub use import::post_processing as app_usecase_post_processing;
pub use import::upgrade;
pub use integration::tracked_downloads;
pub use library::filesystem_walk;
pub use library::recycle_bin;
pub use notifications::runtime::{
    NotificationSubscriptionTargetCreate, NotificationSubscriptionTargetUpdate,
};
pub use oauth::{
    CreateOAuthClientRegistration, OAUTH_E2E_CLIENT_ENV, OAUTH_E2E_CLIENT_ID,
    OAUTH_GENERIC_NATIVE_CLIENT_ID, OAUTH_JELLYFIN_LINK_SCOPE, OAUTH_LIBRARY_SCOPE,
    OAuthClientInfo, OAuthClientSource, OAuthConnectedAppSummary, OAuthIssuedCode, OAuthTokenPair,
    UpdateOAuthClientRegistration,
};
pub use plugins::catalog::blake3_digest as plugin_wasm_blake3_digest;
pub use plugins::catalog::decompress_zstd as plugin_wasm_decompress_zstd;
pub use plugins::catalog::verify_split_digest as plugin_wasm_verify_split_digest;
pub use plugins::managed_rules;
pub use plugins::plugins::RUNTIME_PLUGIN_LOAD_CONCURRENCY;
pub use plugins::plugins::decode_persisted_plugin_wasm_payload;
pub use plugins::plugins::load_runtime_plugin_from_persisted_installation_payload;
pub use quality::release_dedup;
pub use services::{
    ActiveImportStream, ActiveImportStreamHandle, ActiveImportStreamPhase, ActiveImportStreamSync,
    DownloadClientCategoryAdmissionSnapshot, DownloadClientCategorySnapshotStore,
    DownloadQueueSync, ImportCancellation, PluginInstallInProgressError,
    PluginInstallOperationKind, PluginInstallProgressSnapshot, PluginInstallState, RuntimeFeature,
    RuntimePerformanceClass, RuntimePerformanceSnapshot,
};
pub use types::canonicalize_jellyfin_user_id;
pub use upstream_scheduler::{
    AccountQuotaKey, AdmissionReason, DeferralReason, EstimatedCost, ExpectedValueHint,
    OutboundDestinationCooldownSnapshotEntry, OutboundHostRpsSnapshotEntry,
    OutboundRateLimitSnapshot, RateLimitCooldownAction, RssFreshnessContext, SchedulerAdmission,
    SchedulerBatchDecision, SchedulerBatchRequest, SchedulerCandidate, SchedulerCandidateId,
    SchedulerFeedback, SchedulerFeedbackOutcome, SchedulerIntent, SchedulerLease,
    SchedulerOperation, SchedulerPluginKind, SchedulerSnapshot, SchedulerSnapshotEntry,
    SchedulerSnapshotFilter, SearchLearningContext, SkipReason, UpstreamScheduler,
};
pub const SCRYER_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const LIBRARY_SCAN_MAX_RECURSIVE_DEPTH: usize =
    library::discovery::LIBRARY_SCAN_MAX_RECURSIVE_DEPTH;

use chrono::{DateTime, Duration, Utc};
use scryer_domain::{
    AppPermission, AppPermissionMask, BlocklistEntry, CalendarEpisode, Collection, CollectionType,
    CompletedDownload, DomainEvent, DomainEventFilter, DomainEventType, DownloadClientConfig,
    DownloadQueueItem, DownloadQueueState, Episode, ExternalId, HistoryEvent, Id, ImportFileResult,
    ImportMode, ImportRecord, ImportResult, ImportStatus, IndexerConfig, Library, LibraryGrant,
    MediaFacet, MediaRequest, MediaServerConnection, MediaServerDefaultLibraryGrant,
    MediaServerPathMapping, MediaServerProvider, NewDomainEvent, NewDownloadClientConfig,
    NewIndexerConfig, NewTitle, PluginCatalogSource, PluginCatalogStatusRecord, PluginInstallation,
    PolicyInput, PolicyOutput, RuleSet, SubtitleProviderConfig, TaggedAlias, Title,
    TitleHistoryEventType, TitleHistoryRecord, User,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{Mutex, OnceCell, RwLock, Semaphore, broadcast};

pub type AppResult<T> = Result<T, AppError>;

use crate::quality_profile::resolve_profile_id_for_title;
pub use acquisition::delay_profile::{
    DELAY_PROFILE_CATALOG_KEY, DelayDecision, DelayProfile, PreferredProtocol, is_usenet_source,
    parse_delay_profile_catalog, resolve_delay_decision, resolve_delay_profile,
    validate_delay_profile_catalog,
};
pub use acquisition::policy::AcquisitionThresholds;
pub use acquisition::seed_goals::{
    ReleaseSeedMinimums, ResolvedSeedGoals, SeedGoalRequest, SeedGoalResolutionSource,
    SeedGoalResolver, prowlarr_managed_goal_profile, prowlarr_managed_minimum_seeders,
    release_extra_f64, release_extra_i64,
};
pub use acquisition::wanted_views::{
    AcquisitionSearchJobView, AcquisitionSearchProgress, AcquisitionSearchRequest,
    WantedConvergenceState, WantedScopeView, WantedViewConvergence,
};
pub use acquisition_workflow::start_background_acquisition_poller;
pub use app_usecase_integration::derive_download_queue_display_state;
pub use app_usecase_integration::enrich_download_queue_items_from_submissions;
pub use app_usecase_integration::matches_download_activity_filter;
pub use app_usecase_integration::matches_download_queue_filter;
pub use app_usecase_integration::{
    DownloadQueuePollerOptions, start_download_queue_poller,
    start_download_queue_poller_with_options,
};
pub use app_usecase_integration::{DownloadSeedingState, derive_download_seeding_state};
pub use app_usecase_post_processing::{PostProcessingContext, run_post_processing};
pub use app_usecase_rss::RssSyncReport;
#[cfg(test)]
pub(crate) use audio_requirements::missing_required_audio_languages;
#[cfg(feature = "runtime-media-analysis")]
pub(crate) use audio_requirements::{RequiredAudioVerdict, classify_required_audio};
pub(crate) use audio_requirements::{
    normalize_required_audio_requirements, release_audio_language_hints_for_title,
    required_audio_languages_match, resolve_required_audio_requirements,
    title_audio_language_context,
};
pub use catalog::facets::handler::{
    FacetHandler, HydrationResult, movie_to_hydration_result, series_to_hydration_result,
};
pub use catalog::facets::movie::MovieFacetHandler;
pub use catalog::facets::registry::FacetRegistry;
pub use catalog::facets::series::SeriesFacetHandler;
pub use catalog::interactive_release_search::{
    InteractiveReleaseSearchIndexerStatus, InteractiveReleaseSearchIndexerView,
    InteractiveReleaseSearchRequest, InteractiveReleaseSearchSnapshot,
    InteractiveReleaseSearchState,
};
pub use catalog::release_search::release_candidate_fingerprint;
pub use catalog::title_hydration::start_background_title_hydration_loop;
pub use catalog::title_images::start_background_title_image_loop;
pub use catalog::workflow::{
    DeleteEpisodeFilesJobAccepted, DeleteTitlesJobAccepted, DeleteTitlesJobItem,
    DeleteTitlesJobRequest,
};
pub use contracts::{
    AcquisitionScopeStatesQuery, ActivityWindowCounts, AudioStreamDetail,
    CanonicalDownloadIdentityDisposition, ClaimedMediaFile, ClientJobLocator, CollectionUpdate,
    DashboardActivityStats, DeleteExecutionConfirmation, DownloadClientAddRequest,
    DownloadClientBindingRecord, DownloadClientConfigUpdate, DownloadClientMarkImportedRequest,
    DownloadClientStatus, DownloadOrigin, DownloadRecord, DownloadSubmission,
    DownloadSubmissionActorSnapshot, DownloadSubmissionIdentity, DownloadSubmissionPurpose,
    EpisodeUpdate, ImportArtifact, IndexerConfigSyncResult, IndexerConfigUpdate,
    IndexerDownloadClientMappingCatalog, IndexerDownloadClientMappingClient,
    IndexerDownloadClientMappingIndexer, IndexerDownloadClientProviderCompatibility,
    IndexerProxyConfigUpdate, IndexerProxyTestResult, IndexerRoutingEntry, IndexerRoutingPlan,
    IndexerSearchEligibility, IndexerSyncPlan, IndexerValidationResult, InsertMediaFileInput,
    ManagedIndexerChildPlan, ManagedIndexerRoutingScope, MediaAnalysisOutcome, MediaFileAnalysis,
    MediaFileCatalogDisposition, MediaFileRole, NewBlocklistEntry, NewIndexerProxyConfig,
    NewSeedingProfile, NotificationScopeIdUpdate, ObservationResolution, ObservedClientJob,
    PendingReleasePageSort, PendingReleasesPageQuery, PendingStagedNzb, PersistedSeedGoals,
    QueueDownloadOutcome, QueuedDownloadResult, QueuedManualImport, QueuedReleaseSelection,
    ReleaseDecisionsQuery, ResolvedDownloadArtifact, SearchMode, SeedingProfileUpdate,
    StagedNzbRef, StorageRootUsage, SubmissionConflictPolicy, SubmissionScope,
    SubmissionScopeConflict, SubtitleGenerationInput, SubtitleProviderConfigUpdate,
    SubtitleProviderValidationResult, SubtitleStreamDetail, SuccessfulGrabCommit,
    TerminalDownloadHistoryRow, TitleHistoryFilter, TitleHistoryPage, WantedSearchOutcome,
    indexer_search_eligibility,
};
pub use domain_events::DomainEventActor;
pub use download_client_path_mappings::{
    DownloadClientRemotePathMapping, apply_remote_path_mappings_to_completed_download,
    apply_remote_path_mappings_to_status, has_download_client_remote_path_mappings,
    parse_download_client_remote_path_mappings, remap_remote_path,
};
pub use event_views::{
    apply_download_queue_projection_event, apply_job_next_run_projection_event,
    apply_job_run_projection_event, apply_library_scan_projection_event, replay_active_job_runs,
    replay_download_queue_state, replay_job_next_runs, replay_library_scan_state,
    sorted_download_queue_items,
};
pub use events::activity::{ActivityChannel, ActivityEvent, ActivityKind, ActivitySeverity};
pub use events::activity_api::{
    is_supported_title_history_event_type, supported_title_history_event_types,
};
pub(crate) use import_workflow::fail_active_manual_import_for_source;
pub use import_workflow::{
    ManualImportCandidateMapping, ManualImportExecutionResult, ManualImportFileMapping,
    ManualImportFileResult, ManualImportRequestPayload, begin_manual_import_selection,
    execute_manual_import, execute_queued_manual_import, import_completed_download,
    retry_failed_import, start_background_manual_import_poller,
};
pub use integration::download_queue_commands::start_background_download_delete_poller;
pub(crate) use integration::integration::ManualImportSourceResolution;
pub use jobs::jobs::start_background_library_refresh_loop;
pub use library::rename::{
    LibraryRenamer, NullLibraryRenamer, RenameApplyItemResult, RenameApplyResult,
    RenameApplyStatus, RenameCollisionPolicy, RenameMissingMetadataPolicy, RenamePlan,
    RenamePlanItem, RenameWriteAction, build_rename_plan_fingerprint, render_rename_template,
    sanitize_filesystem_component,
};
pub(crate) use library::rename::{
    effective_title_folder_path, normalize_season_folder_template_or_default,
    normalize_specials_folder_template_or_default, normalize_title_folder_template_or_default,
    render_episode_folder_name, validate_rename_template_for_facet,
    validate_season_folder_template, validate_specials_folder_template,
    validate_title_folder_template,
};
pub use media::language::{
    normalize_detected_audio_language_code, normalize_detected_audio_languages,
    normalize_detected_subtitle_language_code, normalize_detected_subtitle_languages,
    normalize_known_audio_language_code, normalize_metadata_language_code,
};
pub use media_requests::{
    ListMediaRequestsInput, SubmitMediaRequestInput, SubmitMediaRequestOutcome,
    UpdateMediaRequestInput,
};
pub use media_servers::{
    EmbyConnectionMode, EmbyLocalSetupMethod, MediaServerConnectionDraft,
    MediaServerConnectionPatch, MediaServerPlaybackLink,
    start_background_media_server_playback_reconciliation_loop,
};
pub use plugins::plugins::{
    ManualPluginPreview, PluginCatalogStatus, RegistryPlugin, RulePackRegistryEntry,
    RulePackTemplate,
};
pub use ports::{MediaServerCatalogItem, MediaServerCatalogItemKind};
pub use security::backup::{AutoBackupRunOutcome, start_background_auto_backup_scheduler};
pub use security::backup_bundle::{
    BACKUP_TABLE_CATALOG, BLOB_MARKER_BASE64, BLOB_MARKER_TYPE, BackupBundleExportRequest,
    BackupBundleInspectSummary, BackupBundleRestorePayload, BackupBundleStaging,
    BackupExportOutcome, BackupExportSecrets, BackupInstanceSecrets, BackupRestorePreparedBundle,
    BackupTableCatalogEntry, BackupTableClassification, EXPORT_BATCH_SIZE,
    PreparedBackupBundleDirectory, backup_table_part_filename, inspect_backup_bundle,
    prepare_backup_restore_payload,
};
pub use security::external_accounts::{ExternalAuthRuntimeConnection, ExternalAuthRuntimeSettings};
pub use settings::settings::{
    AcquisitionSettings, AutoBackupSettings, BackupSettings, DownloadClientRoutingSettingsEntry,
    ExternalImportLibraryPathsSelection, ExternalImportLibrarySettingsAutoApplyDraft,
    ExternalImportLibrarySettingsAutoApplyResult, ExternalImportSettingsAutoApplySkip,
    FacetScoringPersonaSelection, GeneralSettings, IndexerRoutingSettingsEntry,
    LibraryPathsSettings, LibrarySettings, LibrarySettingsOverrideDraft, MediaSettings,
    PluginAutoUpdateSettings, QualityProfileSelection, QualityProfileSettings,
    RequestQualityProfileSettings, SaveQualityProfileSettings, SecuritySettings, ServiceSettings,
    SubtitleSettings, UpdateAutoBackupSettings, UpdateBackupSettings,
    UpdateFacetScoringPersonaSelection, UpdateGeneralSettings, UpdateLibraryPaths,
    UpdateMediaSettings, UpdatePluginAutoUpdateSettings, UpdateQualityProfileSelection,
    UpdateSecuritySettings, UpdateServiceSettings, UpdateSubtitleSettings,
};
pub use subtitles::orchestration::{
    DownloadSubtitleForMediaFileRequest, spawn_subtitle_search_for_file,
    start_background_subtitle_poller,
};

pub(crate) const LIBRARY_SCAN_GLOBAL_TITLE_WALK_CONCURRENCY: usize = 4;
pub(crate) const LIBRARY_SCAN_MOVIE_TITLE_ANALYSIS_GROUP_CONCURRENCY: usize = 24;
pub(crate) const LIBRARY_SCAN_EPISODIC_TITLE_ANALYSIS_GROUP_CONCURRENCY: usize = 4;
pub(crate) const LIBRARY_SCAN_MOVIE_FILE_ANALYSIS_CONCURRENCY_PER_WALK: usize = 1;
pub(crate) const LIBRARY_SCAN_EPISODIC_FILE_ANALYSIS_CONCURRENCY_PER_WALK: usize = 6;
pub(crate) const GLOBAL_LIBRARY_SCAN_ANALYSIS_CONCURRENCY: usize = 24;
pub use acquisition::release_search::release_strategy_kind_for_label;
pub use helpers::{
    HashDomain, accepted_inputs_for_client, blake3_identity_hex, nice_thread,
    normalize_release_name, normalize_release_password,
};
pub(crate) use helpers::{
    INHERIT_QUALITY_PROFILE_VALUE, NATIVE_DOWNLOAD_CLIENT_TYPES, await_cancellable,
    await_cancellable_app_result, normalize_release_attempt_hint,
    normalize_release_selection_signature, normalize_show_text_opt, normalize_tags,
    parsed_episode_lookup_season, sanitize_ids, to_hex,
};
pub(crate) use helpers::{filesystem_space, filesystem_space_raw};
pub use image_proxy::image_proxy_source_token;
pub use indexer_errors::{
    CONNECTION_TEST_INDEXER_ID, ClassifiedIndexerError, IndexerErrorRecorder,
    IndexerErrorRepository, NullIndexerErrorRecorder, UNKNOWN_INDEXER_ERROR_MESSAGE,
    classify_indexer_http_response, classify_newznab_error_message,
    indexer_error_history_is_persistable, indexer_response_content_type,
    redact_indexer_response_headers, unknown_indexer_error,
};
pub use jobs::definitions::{
    JobCategory, JobDefinition, JobKey, JobRun, JobRunRecord, JobRunStatus, JobRunTracker,
    JobScheduleInfo, JobScheduleKind, JobSection, JobTriggerSource, LibraryProbeSignature,
};
pub use library::user_delete::{
    DeleteEpisodeFilePreviewResult, DeleteEpisodeFilesPreview, DeletePreview, DeleteTitlesPreview,
};
pub use library_scan::{
    AnimeEpisodeMapping, AnimeMapping, AnimeMovie, BulkArtworkUrlResult, BulkMetadataResult,
    DiscoveryCollectionCompletionInput, DiscoveryCollectionCompletionResult,
    DiscoveryContentCertification, DiscoveryContentRating, DiscoveryContextChangeType,
    DiscoveryContextChangedSubjectInput, DiscoveryContextChangesInput,
    DiscoveryContextChangesResult, DiscoveryContextSnapshotAckResult,
    DiscoveryContextSnapshotPageResult, DiscoveryContextSnapshotStatusResult,
    DiscoveryContextSnapshotSubmitInput, DiscoveryContextSnapshotSubmitResult,
    DiscoveryDashboardResult, DiscoveryDashboardSection, DiscoveryExternalIdInput, DiscoveryFacet,
    DiscoveryPublicFeedInput, DiscoveryRatingProvenance, DiscoveryRelatedResult,
    DiscoverySnapshotFacetGroup, DiscoverySnapshotFacetValue, DiscoverySubjectInput,
    DiscoveryTitle, EpisodeArtworkUrls, EpisodeMetadata, LibraryDirectoryScanResult, LibraryFile,
    LibraryFileBatch, LibraryFileBatchReceiver, LibraryScanSummary, LibraryScanner,
    MetadataGateway, MetadataSearchItem, MetadataSearchQuery, MovieMetadata, MovieTitleBulkResult,
    MovieTitleRef, MultiMetadataSearchResult, RichMetadataSearchItem, SeasonMetadata,
    SeriesArtworkUrls, SeriesMetadata, TitleArtworkUrls, TitleRecommendationsInput,
    TitleResolution,
};
pub use library_scan_progress::{
    LibraryScanMode, LibraryScanPhaseProgress, LibraryScanSession, LibraryScanStatus,
    LibraryScanTracker,
};
pub use media::analyzer::NativeMediaAnalyzer;
pub use notifications::dispatcher::start_notification_dispatcher;
pub use null_repositories::NullIndexerErrorRepository;
pub use null_repositories::{
    NullAcquisitionScopeStateRepository, NullAcquisitionStateRepository, NullBlocklistRepository,
    NullDomainEventRepository, NullDownloadQueueCommandRepository, NullDownloadRegistryRepository,
    NullDownloadSubmissionRepository, NullExternalImportMonitorSnapshotRepository,
    NullExternalImportSetupSecretDraftRepository, NullFileImporter, NullHousekeepingRepository,
    NullImportArtifactRepository, NullImportRepository, NullIndexerProxyConfigRepository,
    NullIndexerSearchLearningRepository, NullIndexerStatsTracker, NullJobRunRepository,
    NullLibraryProbeRepository, NullLibraryRepository, NullLibraryScanUnmatchedItemRepository,
    NullLogicalBackupExporter, NullMediaFileRepository, NullMediaRequestRepository,
    NullMediaServerConnectionRepository, NullNotificationChannelRepository,
    NullNotificationSubscriptionRepository, NullOAuthRepository, NullPendingReleaseRepository,
    NullPluginDescriptorLoader, NullPluginHttpTrustConfigRuntime, NullPluginInstallationRepository,
    NullPostProcessingScriptRepository, NullRuleSetRepository, NullScopeIndexerCoverageRepository,
    NullSettingsRepository, NullStagedNzbStore, NullSubtitleDownloadRepository,
    NullSystemInfoProvider, NullTitleImageProcessor, NullTitleImageRepository,
    NullUpstreamScheduler, NullWorkflowOperationRepository,
};
pub use ports::{
    AcquisitionScopeStateRepository, AcquisitionStateRepository, ArchiveExtractorClient,
    ArchiveExtractorPluginProvider, BlocklistRepository, BuiltinDownloadClientConnectionTester,
    DatastoreInfo, DomainEventRepository, DownloadClient, DownloadClientConfigRepository,
    DownloadClientFeedbackScope, DownloadClientPluginProvider, DownloadClientSnapshotOutcome,
    DownloadQueueCommandRepository, DownloadRegistryRepository, DownloadSubmissionRepository,
    EmbyApiKeyExchange, EmbyApiKeyExchangeCleanup, EmbyAvatar, EmbyConnectAddressStatus,
    EmbyConnectIdentityVerification, EmbyConnectServer, EmbyConnectUserType, EmbyServerIdentity,
    EmbyServerUser, ExternalIdentityVerifier, ExternalImportMonitorSnapshotRepository,
    ExternalImportSetupInstanceApiKeyDraft, ExternalImportSetupSecretDraft,
    ExternalImportSetupSecretDraftInput, ExternalImportSetupSecretDraftRepository,
    ExternalImportSetupSecretDraftSaveResult, ExternalImportSetupSecretDraftStatus,
    ExternalImportSetupSecretInstanceKind, ExternalImportSetupSecretOverrideDraft,
    ExternalPluginWasm, FileImporter, HousekeepingMediaFileRootRow, HousekeepingRepository,
    IdentityTrackedStateTarget, ImageProxyCacheControl, ImageProxyCacheEntryRecord, ImageProxyKind,
    ImageProxyRegistration, ImageProxyRepository, ImageProxySourceRecord, ImportArtifactRepository,
    ImportFileExecutionContext, ImportFilePermissions, ImportFileTransferProgress,
    ImportFileTransferProgressSender, ImportRepository, IndexerCapsSnapshotRefresher,
    IndexerClient, IndexerConfigRepository, IndexerManagementClient, IndexerPluginProvider,
    IndexerProxyConfigRepository, IndexerSearchCandidateWrite, IndexerSearchLearningContext,
    IndexerSearchLearningKey, IndexerSearchLearningRecord, IndexerSearchLearningRepository,
    IndexerSearchRunWrite, IndexerStatsTracker, IndexerSystemBackoff, JellyfinServerUser,
    JobRunRepository, LibraryProbeRepository, LibraryRepository,
    LibraryScanUnmatchedItemRepository, LogicalBackupExporter, MediaAnalyzer, MediaFileRepository,
    MediaRequestQuery, MediaRequestRepository, MediaServerConnectionRepository, MediaServerUser,
    MediaServerUserGroup, MediaServerUserGroupStatus, NOTIFICATION_REQUEST_SCHEMA_VERSION,
    NewMediaRequest, NormalizedIndexerSearchCandidate, NotificationActorPayload,
    NotificationAppPayload, NotificationApplicationUpdatePayload, NotificationChannelRepository,
    NotificationClient, NotificationDownloadPayload, NotificationEpisodePayload,
    NotificationExternalIdsPayload, NotificationFilePayload, NotificationHealthPayload,
    NotificationImportPayload, NotificationManualInteractionPayload, NotificationMediaFilePayload,
    NotificationMediaUpdatePayload, NotificationMediaUpdateTypePayload, NotificationPayload,
    NotificationPluginProvider, NotificationReleasePayload, NotificationSeverityPayload,
    NotificationSubscriptionRepository, NotificationTitlePayload, OAuthRepository,
    PendingReleaseRepository, PlexServerDiscovery, PlexServerUser, PluginDescriptorLoader,
    PluginHttpTrustConfigRuntime, PluginInstallationRepository, PostProcessingScriptRepository,
    QualityProfileRepository, ReleaseAttemptRepository, ReusableIndexerSearchCandidate,
    ReusableIndexerSearchStrategy, RuleSetRepository, RuntimePluginLoad, ScopeCoverageRow,
    ScopeIndexerCoverageRepository, SeedingProfileRepository, SettingsRepository, ShowRepository,
    StagedNzbStore, SubtitleDownloadRepository, SubtitlePluginProvider, SubtitleProviderClient,
    SubtitleProviderConfigRepository, SystemInfoProvider, TitleImageProcessor,
    TitleImageRepository, TitleRepository, TotpRepository, UserExternalAccountRepository,
    UserRepository, VerifiedExternalIdentity, WebauthnRepository, WorkflowOperationInfo,
    WorkflowOperationRepository,
};
pub use quality::release_parser::{
    AudioCodec, ExternalIdSource, ParsedEpisodeMetadata, ParsedEpisodeReleaseType,
    ParsedReleaseMetadata, ParsedSpecialKind, ReleaseParseAnalysis, ReleaseParseContext,
    ReleaseSource, StreamingService, TargetedReleaseParseAnalysis, VideoCodec,
    analyze_release_against_targets, analyze_release_for_target, best_parse_for_target,
    build_candidate_bank_contexts, build_release_parse_context,
    build_release_parse_context_for_title, parse_release_metadata,
    parse_release_metadata_for_target,
};
pub use quality::scoring_weights::{
    ScoringOverrides, ScoringPersona, ScoringWeights, build_weights, build_weights_for_category,
};
pub use quality_profile::{
    BLOCK_SCORE, BUILTIN_DEFAULT_QUALITY_PROFILE_ID, QUALITY_PROFILE_CATALOG_KEY,
    QUALITY_PROFILE_ID_KEY, QUALITY_PROFILE_INHERIT_VALUE, QualityProfile, QualityProfileCriteria,
    QualityProfileDecision, REQUEST_QUALITY_PROFILE_IDS_KEY, ScoringConfig, ScoringEntry,
    ScoringSource, apply_age_scoring, apply_size_scoring_for_category, builtin_4k_profile,
    builtin_8k_profile, builtin_1080p_profile, builtin_anime_profile,
    builtin_default_quality_profile, evaluate_against_profile, parse_profile_catalog_from_json,
};
pub use rate_limit_signal::{RateLimitSignal, RateLimitSignalSource};
pub use services::{
    AppServices, AppServicesBuilder, AppUseCase, ExternalImportArrSourceKind,
    ExternalImportArrSourceSeriesEntry, ExternalImportArrSourceWarmupResult,
    ExternalImportMonitorWarmupBeginResult, ExternalImportMonitorWarmupPhase,
    ExternalImportMonitorWarmupPhaseProgress, ExternalImportMonitorWarmupProgressSnapshot,
    ExternalImportMonitorWarmupStatus, ExternalImportProwlarrWarmupResult, ProviderCatalogFamily,
};
pub use settings::keys::{
    ANIME_FILLER_POLICY_KEY, ANIME_INTER_SEASON_MOVIES_KEY, ANIME_MONITOR_FILLER_MOVIES_KEY,
    ANIME_MONITOR_SPECIALS_KEY, ANIME_PATH_KEY, ANIME_RECAP_POLICY_KEY, ANIME_ROOT_FOLDERS_KEY,
    API_KEYS_RESTRICT_TO_SYSTEM_SETTINGS_USERS_KEY, AUDIO_PERSONA_MIGRATION_SENTINEL_KEY,
    AUTO_BACKUP_DAILY_TIME_LOCAL_KEY, AUTO_BACKUP_DISABLED_MISSING_KEY_NOTICE_KEY,
    AUTO_BACKUP_ENABLED_KEY, AUTO_BACKUP_KEY_KEY, AUTO_BACKUP_POST_UPGRADE_PENDING_VERSION_KEY,
    BACKUP_PATH_KEY, CHOWN_GROUP_KEY, DEFAULT_ANIME_LIBRARY_PATH,
    DEFAULT_AUTO_BACKUP_DAILY_TIME_LOCAL, DEFAULT_FILLER_POLICY, DEFAULT_FOLDER_TEMPLATE_ANIME,
    DEFAULT_FOLDER_TEMPLATE_MOVIE, DEFAULT_FOLDER_TEMPLATE_SERIES, DEFAULT_IMAGE_CACHE_MAX_SIZE_MB,
    DEFAULT_MOVIE_LIBRARY_PATH, DEFAULT_RECAP_POLICY, DEFAULT_RENAME_COLLISION_POLICY,
    DEFAULT_RENAME_MISSING_METADATA_POLICY, DEFAULT_RENAME_TEMPLATE_ANIME,
    DEFAULT_RENAME_TEMPLATE_MOVIE, DEFAULT_RENAME_TEMPLATE_SERIES, DEFAULT_SEASON_FOLDER_TEMPLATE,
    DEFAULT_SEEDING_PROFILE_SETTING_KEY, DEFAULT_SERIES_LIBRARY_PATH,
    DEFAULT_SPECIALS_FOLDER_TEMPLATE, DISCOVERY_REGION_KEY,
    DOWNLOAD_CLIENT_DEFAULT_CATEGORY_SETTING_KEY, DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
    FILE_CHMOD_KEY, FOLDER_CHMOD_KEY, FOLDER_TEMPLATE_KEY, FORM_LOGIN_ENABLED_KEY,
    HISTORY_KEEP_FOREVER_KEY, HISTORY_RETENTION_DAYS_KEY, IMAGE_CACHE_MAX_BYTES_ENV,
    IMAGE_CACHE_MAX_SIZE_MB_KEY, IMPORT_MODE_KEY, INDEXER_ROUTING_SETTINGS_KEY,
    LEGACY_NZBGET_CATEGORY_SETTING_KEY, LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY,
    METADATA_LANGUAGE_KEY, MFA_REQUIRE_CONFIG_STEP_UP_KEY, MFA_REQUIRE_PASSWORD_LOGIN_KEY,
    MINIMUM_SEEDERS_FLOOR_DEFAULT, MINIMUM_SEEDERS_FLOOR_DEFAULT_JSON,
    MINIMUM_SEEDERS_FLOOR_SETTING_KEY, MOVIES_PATH_KEY, MOVIES_ROOT_FOLDERS_KEY,
    NFO_WRITE_ON_IMPORT_ANIME_KEY, NFO_WRITE_ON_IMPORT_MOVIE_KEY, NFO_WRITE_ON_IMPORT_SERIES_KEY,
    NZBGET_OLDER_PRIORITY_SETTING_KEY, NZBGET_RECENT_PRIORITY_SETTING_KEY, PASSWORD_MIN_LENGTH_KEY,
    PASSWORD_MIN_LENGTH_MIN, PLEXMATCH_WRITE_ON_IMPORT_ANIME_KEY,
    PLEXMATCH_WRITE_ON_IMPORT_SERIES_KEY, PLUGIN_AUTO_UPDATE_ENABLED_KEY,
    PLUGIN_HTTP_CA_BUNDLE_PEM_KEY, POST_PROCESSING_SCRIPT_ANIME_KEY,
    POST_PROCESSING_SCRIPT_MOVIE_KEY, POST_PROCESSING_SCRIPT_SERIES_KEY,
    POST_PROCESSING_TIMEOUT_KEY, RECYCLE_BIN_ENABLED_KEY, RECYCLE_BIN_PATH_KEY,
    RECYCLE_BIN_RETENTION_DAYS_KEY, RENAME_COLLISION_POLICY_ANIME_GLOBAL_KEY,
    RENAME_COLLISION_POLICY_GLOBAL_KEY, RENAME_COLLISION_POLICY_KEY,
    RENAME_COLLISION_POLICY_MOVIE_GLOBAL_KEY, RENAME_COLLISION_POLICY_SERIES_GLOBAL_KEY,
    RENAME_ENABLED_KEY, RENAME_MISSING_METADATA_POLICY_ANIME_GLOBAL_KEY,
    RENAME_MISSING_METADATA_POLICY_GLOBAL_KEY, RENAME_MISSING_METADATA_POLICY_KEY,
    RENAME_MISSING_METADATA_POLICY_MOVIE_GLOBAL_KEY,
    RENAME_MISSING_METADATA_POLICY_SERIES_GLOBAL_KEY, RENAME_TEMPLATE_ANIME_GLOBAL_KEY,
    RENAME_TEMPLATE_KEY, RENAME_TEMPLATE_MOVIE_GLOBAL_KEY, RENAME_TEMPLATE_SERIES_GLOBAL_KEY,
    REQUIRED_AUDIO_LANGUAGES_KEY, SCORING_PERSONA_KEY, SEASON_FOLDER_TEMPLATE_KEY, SERIES_PATH_KEY,
    SERIES_ROOT_FOLDERS_KEY, SET_PERMISSIONS_LINUX_KEY, SETTINGS_SCOPE_MEDIA,
    SETTINGS_SCOPE_SYSTEM, SETTINGS_SOURCE_TYPED_GRAPHQL, SETUP_COMPLETE_KEY,
    SKIP_LOGIN_FOR_LOCAL_IPS_KEY, SPECIALS_FOLDER_TEMPLATE_KEY,
    TITLE_METADATA_LANGUAGE_OVERRIDE_KEY, TITLE_REQUIRED_AUDIO_OVERRIDE_KEY, TLS_CERT_PATH_KEY,
    TLS_KEY_PATH_KEY, TOTP_REQUIRE_EMBY_LOGIN_KEY, TOTP_REQUIRE_JELLYFIN_LOGIN_KEY,
    USE_SEASON_FOLDERS_KEY,
};
pub use settings::runtime::is_bootstrap_default_library_root_set;
pub(crate) use types::JwtClaims;
pub use types::MetadataFieldUpdate;
#[cfg(test)]
pub(crate) use types::ReleaseCandidateTokenClaims;
pub use types::{
    AcquisitionScopeCompleteTransition, AcquisitionScopeGrabTransition,
    AcquisitionScopePauseTransition, AcquisitionScopeState, AcquisitionScopeStatus,
    AddTitleAndQueueDownloadOutcome, AddTitleHydrationState, AddTitleOutcome,
    ApiKeyProvisioningSource, ApiKeyRecord, AuthenticatedTokenClaims, BackupDownloadTicket,
    BackupInfo, BackupStatus, BackupTrigger, CancelLibraryScanResult,
    CollectionEpisodeProgressSummary, CreateTitleOutcome, CutoffUnmetItem, CutoffUnmetPage,
    CutoffUnmetQualitySummary, DecisionCodeCount, DiskSpaceInfo, DownloadActivityFilter,
    DownloadDisplayState, DownloadGrabResult, DownloadHistoryFilter, DownloadHistoryPage,
    DownloadHistorySort, DownloadHistorySortKey, DownloadImportFilter, DownloadImportPage,
    DownloadQueueCommandRecord, DownloadQueuePage, DownloadSourceKind, EpisodeMediaAvailability,
    EpisodeMediaAvailabilityState, EpisodeScopedMediaFile, FixTitleMatchResult, HealthCheckResult,
    HealthCheckStatus, HousekeepingReport, IgnorePendingImportResult, IndexerQueryStats,
    JwtAuthConfig, JwtSessionScope, LibraryRootDraft, LibraryScanUnmatchedItem,
    LibraryScanUnmatchedSearchAttempt, LoginFailureTimingClass, LoginVerificationChallengeRecord,
    LoginVerificationMethod, LoginVerificationRequirement, LoginVerificationSatisfied,
    ManualImportSelection, ManualImportSelectionCandidate, MediaFileAssociations,
    MediaRequestCounts, MissingEpisodeCandidate, MissingScopeCandidates,
    MissingSeriesMovieLinkCandidate, MissingTitleCandidate, OAuthAuthorizationCodeRecord,
    OAuthAuthorizationSource, OAuthClientRegistrationRecord, OAuthConnectedAppRecord,
    OAuthRefreshGrantRecord, OAuthRefreshRotation, OAuthRefreshRotationOutcome,
    OAuthRefreshTokenRecord, PasskeySummary, PendingImportBindingFilePreview,
    PendingImportBindingPreview, PendingImportConnection, PendingImportCounts, PendingImportItem,
    PendingImportReasonClass, PendingImportSearchAttempt, PendingImportStatus, PendingRelease,
    PendingReleaseObservation, PendingReleaseRole, PendingReleaseStatus, PendingReleaseStatusCount,
    PendingTitleHydration, PrimaryCollectionSummary, RecycleBinBatchJobAccepted,
    RecycleBinSettings, RecycleRestoreConflictPolicy, RecycleRestorePreview,
    RecycleRestorePreviewItem, RecycledItem, ReleaseDecision, ReleaseDownloadAttemptOutcome,
    ReleaseDownloadFailureRecord, ReleaseDownloadFailureSignature, ResolvePendingImportResult,
    RuntimePathStyle, ScopedExternalId, SortDirection, SystemHealth, TitleAcquisitionDiagnostics,
    TitleCatalogContentStatus, TitleCatalogFilter, TitleCatalogFilterCounts,
    TitleCatalogFilterOptions, TitleCatalogResult, TitleCatalogSort, TitleCatalogSortKey,
    TitleCatalogTagFilterOption, TitleCredit, TitleEpisodeProgressSummary, TitleExternalRating,
    TitleImageBlob, TitleImageKind, TitleImageSourceResult, TitleImageSyncTask,
    TitleImageVariantRecord, TitleImageVariantSpec, TitleMediaFile, TitleMediaSizeSummary,
    TitleMetadataUpdate, TitleMovieMediaSummary, TitleQualitySummary, TitleRatingSummary,
    TitleReleaseBlocklistEntry, TotpCredentialRecord, TotpEnrollmentChallengeRecord,
    TotpEnrollmentComplete, TotpEnrollmentStart, TotpFailedAttemptRecord, TotpRecoveryCodeRecord,
    TotpStatus, UiDateTimeFormat, UiDefaultLandingView, UiDensity, UiSettings, UiSettingsFacet,
    UiSettingsUpdate, UiSidebarMode, UiTableColumnSetting, UiTableViewMode, UiTheme,
    UpdateRecycleBinSettings, UserAuthFactorStatus, UserLoginSnapshot, VerifiedLocalCredentials,
    WantedKind, WantedStatusCount, WebauthnChallengePurpose, WebauthnChallengeRecord,
    WebauthnChallengeStart, WebauthnChallengeType, WebauthnCredentialRecord,
};
pub use types::{
    CapturedIndexerHttpHeader, CapturedIndexerHttpResponse, INDEXER_CAPS_REFRESH_ERROR_PREFIX,
    IndexerErrorClassification, IndexerErrorDetail, IndexerErrorOperation, IndexerErrorPage,
    IndexerErrorSummary, IndexerQueryOutcome, IndexerResponseAttributes, IndexerSearchCompletion,
    IndexerSearchIncompleteReason, IndexerSearchOutcome, IndexerSearchPage,
    IndexerSearchPageReservation, IndexerSearchPageSink, IndexerSearchPlanCapability,
    IndexerSearchPlanRequest, IndexerSearchPlanSummary, IndexerSearchResponse, IndexerSearchResult,
    IndexerSearchStrategyEvent, IndexerSearchStrategyEventSink, IndexerSearchStrategyRequest,
    NewIndexerError, ReleaseCandidateProvenance, ReleaseSearchSubjectKind, ReleaseStrategyKind,
    extract_magnet_info_hash, indexer_search_identity, is_valid_magnet_uri,
    search_relevant_indexer_caps, search_relevant_managed_indexer_metadata,
};
pub use types::{
    EXTERNAL_IMPORT_MONITOR_APPLY_SESSION_ID, EXTERNAL_IMPORT_MONITOR_APPLY_SESSION_PREFIX,
    ExternalIdHint, ExternalIdProvider, ExternalImportMonitorEpisodeEntry,
    ExternalImportMonitorMovieEntry, ExternalImportMonitorSeasonEntry,
    ExternalImportMonitorSeriesEntry, ExternalImportMonitorSnapshotChunk,
    ExternalImportMonitorSnapshotEntryKind, LibraryScanHint, LibraryScanHintFacet,
    LibraryScanHintSet, LibraryScanHintSource, derive_primary_quality_label,
    external_import_monitor_apply_session_id_for_library,
    is_external_import_monitor_apply_session_id, library_scan_file_full_path_key,
    library_scan_file_leaf_key, library_scan_folder_full_path_key, library_scan_folder_leaf_key,
};
pub use types::{SmgScryerUpdateNotice, SmgVersionCompatibilityNotice};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AutoEligibilityReason {
    pub code: String,
    pub summary: String,
    pub count: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("unauthorized: {0}")]
    Unauthorized(String),

    #[error("validation: {0}")]
    Validation(String),

    #[error("no auto-eligible release found")]
    NoAutoEligibleRelease {
        candidate_count: usize,
        reasons: Vec<AutoEligibilityReason>,
    },

    #[error("plugin install already in progress for '{0}'")]
    PluginInstallInProgress(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("{0}")]
    DownloadFeedbackTimeout(String),

    #[error("{0}")]
    DownloadSubmitAmbiguous(String),

    /// An ambiguous client mutation annotated by the router with the selected
    /// configured client. The display text intentionally remains identical to
    /// the underlying client error.
    #[error("{message}")]
    DownloadSubmitAmbiguousWithClient {
        message: String,
        client_id: Option<String>,
        client_type: String,
    },

    #[error("{0}")]
    DownloadSubmitRejected(String),

    #[error("{0}")]
    DownloadSubmitUnavailable(String),

    /// The indexer accepted the search result but no longer serves its download
    /// artifact. This is neither retryable client unavailability nor a release
    /// failure worth blocklisting.
    #[error("{0}")]
    DownloadSourceGone(String),

    /// Every eligible download client in the routing order was tried and none
    /// enqueued the release. A retryable submission failure like
    /// `DownloadSubmitUnavailable`, kept distinct for diagnostics; the payload
    /// is display-only context (the final client error when one was captured)
    /// and no consumer may inspect it to make an operational decision.
    #[error("{0}")]
    DownloadSubmitFailoverExhausted(String),

    #[error("{message}")]
    ArchiveExtractionPluginRequired {
        message: String,
        source_path: Option<String>,
    },

    #[error("{message}")]
    ArchiveExtractionTimedOut { message: String },

    #[error("{message}")]
    TemporaryUnavailable {
        message: String,
        retry_after: Option<std::time::Duration>,
        rate_limit_cooldown: RateLimitCooldownAction,
    },

    #[error("{0}")]
    MfaStepUpRequired(String),

    #[error("{0}")]
    ReauthenticationRequired(String),

    #[error("{0}")]
    TotpEnrollmentRequired(String),

    #[error("{0}")]
    MfaEnrollmentRequired(String),

    #[error("{0}")]
    PasswordChangeRequired(String),

    #[error("{0}")]
    TotpInvalidCode(String),

    #[error("{0}")]
    TotpRecoveryCodeUsed(String),

    #[error("canceled: {0}")]
    Canceled(String),

    #[error("manual reconciliation required: {0}")]
    ManualReconciliationRequired(String),

    #[error("import evidence unavailable: {0}")]
    ImportEvidenceUnavailable(String),

    #[error("failed to inspect import source {path}: {message}")]
    ImportSourceInspection { path: String, message: String },

    #[error("unsupported import source type: {path}")]
    UnsupportedImportSource { path: String },

    #[error("import source changed while being inspected {path}: {message}")]
    ImportSourceChanged { path: String, message: String },

    #[error("repository: {0}")]
    Repository(String),
}

impl AppError {
    pub fn canceled(message: impl Into<String>) -> Self {
        Self::Canceled(message.into())
    }

    pub fn download_submit_unavailable(message: impl Into<String>) -> Self {
        Self::DownloadSubmitUnavailable(message.into())
    }

    pub fn download_submit_failover_exhausted(message: impl Into<String>) -> Self {
        Self::DownloadSubmitFailoverExhausted(message.into())
    }

    pub fn with_ambiguous_download_submission_client(
        self,
        client_id: Option<String>,
        client_type: String,
    ) -> Self {
        match self {
            Self::DownloadSubmitAmbiguous(message) => Self::DownloadSubmitAmbiguousWithClient {
                message,
                client_id,
                client_type,
            },
            other => other,
        }
    }

    pub fn ambiguous_download_submission_client(&self) -> Option<(Option<&str>, &str)> {
        match self {
            Self::DownloadSubmitAmbiguousWithClient {
                client_id,
                client_type,
                ..
            } => Some((client_id.as_deref(), client_type.as_str())),
            _ => None,
        }
    }

    pub fn archive_extraction_plugin_required(source_path: Option<String>) -> Self {
        Self::ArchiveExtractionPluginRequired {
            message: "This import is blocked because the download contains archive files. Install, update, or enable the Archive Extraction plugin, then re-import.".to_string(),
            source_path,
        }
    }

    pub fn archive_extraction_timed_out(message: impl Into<String>) -> Self {
        Self::ArchiveExtractionTimedOut {
            message: message.into(),
        }
    }

    pub fn temporary_unavailable(
        message: impl Into<String>,
        retry_after: Option<std::time::Duration>,
    ) -> Self {
        Self::TemporaryUnavailable {
            message: message.into(),
            retry_after,
            rate_limit_cooldown: RateLimitCooldownAction::None,
        }
    }

    pub fn rate_limited_temporary_unavailable(
        message: impl Into<String>,
        retry_after: Option<std::time::Duration>,
        rate_limit_cooldown: RateLimitCooldownAction,
    ) -> Self {
        Self::TemporaryUnavailable {
            message: message.into(),
            retry_after,
            rate_limit_cooldown,
        }
    }

    pub fn into_download_submit_unavailable(self) -> Self {
        match self {
            Self::DownloadSubmitUnavailable(_)
            | Self::DownloadSubmitFailoverExhausted(_)
            | Self::DownloadSubmitAmbiguous(_)
            | Self::DownloadSubmitAmbiguousWithClient { .. }
            | Self::DownloadSubmitRejected(_)
            | Self::DownloadSourceGone(_) => self,
            _ => Self::DownloadSubmitUnavailable(self.to_string()),
        }
    }

    pub fn is_download_submit_unavailable(&self) -> bool {
        matches!(self, Self::DownloadSubmitUnavailable(_))
    }

    pub fn is_download_source_gone(&self) -> bool {
        matches!(self, Self::DownloadSourceGone(_))
    }

    /// The typed retryable download-submission failures: the submitter was
    /// unavailable, or every prioritized client was tried and failed. Text is
    /// never consulted — renaming, prefixing, or wrapping a message cannot
    /// change scheduling.
    pub fn is_retryable_download_submit_failure(&self) -> bool {
        matches!(
            self,
            Self::DownloadSubmitUnavailable(_) | Self::DownloadSubmitFailoverExhausted(_)
        )
    }

    pub fn is_download_submit_ambiguous(&self) -> bool {
        matches!(
            self,
            Self::DownloadSubmitAmbiguous(_) | Self::DownloadSubmitAmbiguousWithClient { .. }
        )
    }

    pub fn is_canceled(&self) -> bool {
        matches!(self, Self::Canceled(_))
    }
}

#[cfg(test)]
mod lib_tests;

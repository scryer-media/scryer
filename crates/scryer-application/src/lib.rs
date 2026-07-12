mod acquisition;
mod authorization;
mod catalog;
mod contracts;
mod download_client_config;
mod download_client_path_mappings;
mod download_identity;

pub(crate) const DOWNLOAD_QUEUE_RECENT_COMPLETED_LIMIT: usize = 100;
pub use download_identity::{
    AcceptedDownloadIdentityInput, DOWNLOAD_ID_PARAMETER, ObservedDownloadIdentityInput,
    accepted_download_submission_identity, download_id_from_info_hash,
    download_submission_identity_is_empty, normalize_torrent_info_hash, observed_download_identity,
};
mod events;
pub mod external_import;
pub mod fs_integrity;
mod fs_safety;
mod health;
mod helpers;
mod import;
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
mod quality;
mod rules;
mod security;
mod services;
mod settings;
pub mod stored_paths;
pub mod subtitles;
pub mod testing;
mod types;

pub(crate) use acquisition::acquisition as acquisition_workflow;
pub(crate) use acquisition::coverage as acquisition_coverage;
pub(crate) use acquisition::decision_helpers as acquisition_decision_helpers;
pub(crate) use acquisition::delay_profile;
pub(crate) use acquisition::policy as acquisition_policy;
pub(crate) use acquisition::release_search as acquisition_release_search;
pub(crate) use acquisition::rss as app_usecase_rss;
pub(crate) use acquisition::search_queries as acquisition_search_queries;
pub(crate) use catalog::catalog as catalog_workflow;
pub(crate) use catalog::discovery as app_usecase_discovery;
pub(crate) use catalog::facets::handler as facet_handler;
pub(crate) use catalog::helpers as catalog_helpers;
pub(crate) use events::activity;
pub(crate) use events::domain_events;
pub(crate) use events::event_views;
pub(crate) use import::archive_extractor;
pub(crate) use import::checks as import_checks;
pub(crate) use import::import as import_workflow;
pub(crate) use import::parameters as import_parameters;
pub(crate) use import::post_download_gate;
pub(crate) use import::title_resolution as import_title_resolution;
pub(crate) use integration::integration as app_usecase_integration;
pub(crate) use library::discovery as library_discovery;
pub(crate) use library::filename_parser as library_filename_parser;
pub(crate) use library::nfo;
pub(crate) use library::rename as library_rename;
pub(crate) use library::title_matching;
pub(crate) use media::audio_requirements;
pub(crate) use media::language_data as media_language_data;
pub(crate) use quality::profile as quality_profile;
pub(crate) use quality::release_group_db;
pub(crate) use quality::release_parser;
pub(crate) use quality::scoring_weights;
pub(crate) use rules::user_rule_input;

pub use download_client_config::resolve_download_client_base_url_from_config_json;
pub use import::completed_download as completed_download_handler;
pub use ports::{
    EpisodeImageUrlUpdate, MediaRequestResolution, MediaRequestResolutionResult,
    MediaRequestSubmissionResult, MediaRequestUpdateResult, SubtitleSyncClient, SubtitleSyncJob,
    TitleArtworkUrlUpdate, TitleDeletePreviewInfo,
};
pub(crate) mod normalize;
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
    OAUTH_E2E_CLIENT_ENV, OAUTH_E2E_CLIENT_ID, OAUTH_GENERIC_NATIVE_CLIENT_ID, OAUTH_LIBRARY_SCOPE,
    OAuthClientInfo, OAuthConnectedAppSummary, OAuthIssuedCode, OAuthTokenPair,
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
    PluginInstallInProgressError, PluginInstallOperationKind, PluginInstallProgressSnapshot,
    PluginInstallState, RuntimeFeature, RuntimePerformanceClass, RuntimePerformanceSnapshot,
};
pub const SCRYER_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const LIBRARY_SCAN_MAX_RECURSIVE_DEPTH: usize =
    library::discovery::LIBRARY_SCAN_MAX_RECURSIVE_DEPTH;

use aws_lc_rs::digest as aws_lc_digest;
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
pub use acquisition_workflow::start_background_acquisition_poller;
pub use app_usecase_integration::derive_download_queue_display_state;
pub use app_usecase_integration::enrich_download_queue_items_from_submissions;
pub use app_usecase_integration::matches_download_activity_filter;
pub use app_usecase_integration::matches_download_queue_filter;
pub use app_usecase_integration::{
    DownloadQueuePollerOptions, start_download_queue_poller,
    start_download_queue_poller_with_options,
};
pub use app_usecase_post_processing::{PostProcessingContext, run_post_processing};
pub use app_usecase_rss::RssSyncReport;
#[cfg(any(test, feature = "runtime-media-analysis"))]
pub(crate) use audio_requirements::missing_required_audio_languages;
pub(crate) use audio_requirements::{
    normalize_required_audio_languages, release_audio_language_hints_for_title,
    required_audio_languages_match, title_audio_language_context,
};
pub use catalog::facets::handler::{
    FacetHandler, HydrationResult, movie_to_hydration_result, series_to_hydration_result,
};
pub use catalog::facets::movie::MovieFacetHandler;
pub use catalog::facets::registry::FacetRegistry;
pub use catalog::facets::series::SeriesFacetHandler;
pub use catalog::title_hydration::start_background_title_hydration_loop;
pub use catalog::title_images::start_background_title_image_loop;
pub use catalog::workflow::{DeleteTitlesJobAccepted, DeleteTitlesJobItem, DeleteTitlesJobRequest};
pub use contracts::{
    AudioStreamDetail, CollectionUpdate, DeleteExecutionConfirmation, DownloadClientAddRequest,
    DownloadClientConfigUpdate, DownloadClientMarkImportedRequest, DownloadClientStatus,
    DownloadSourceIdentity, DownloadSubmission, DownloadSubmissionActorSnapshot,
    DownloadSubmissionIdentity, DownloadSubmissionPurpose, EpisodeUpdate, ImportArtifact,
    IndexerConfigSyncResult, IndexerConfigUpdate, IndexerRoutingEntry, IndexerRoutingPlan,
    IndexerSyncPlan, IndexerValidationResult, InsertMediaFileInput, ManagedIndexerChildPlan,
    ManagedIndexerRoutingScope, MediaAnalysisOutcome, MediaFileAnalysis, MediaFileRole,
    NewBlocklistEntry, NotificationScopeIdUpdate, PendingStagedNzb, QueueDownloadOutcome,
    QueuedDownloadResult, QueuedReleaseSelection, ReleaseDecisionsQuery, SearchMode, StagedNzbRef,
    SubmissionConflictPolicy, SubmissionScope, SubmissionScopeConflict, SubtitleGenerationInput,
    SubtitleProviderConfigUpdate, SubtitleProviderValidationResult, SubtitleStreamDetail,
    SuccessfulGrabCommit, TitleHistoryFilter, TitleHistoryPage, WantedItemsQuery,
    WantedSearchOutcome,
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
pub(crate) use import::import::{resolve_import_paths, use_season_folders};
pub(crate) use import_workflow::fail_active_manual_import_for_source;
pub use import_workflow::{
    ManualImportExecutionResult, ManualImportFileMapping, ManualImportFilePreview,
    ManualImportFileResult, ManualImportPreview, ManualImportRequestPayload, execute_manual_import,
    execute_queued_manual_import, import_completed_download, preview_manual_import,
    preview_manual_import_path, retry_failed_import, start_background_manual_import_poller,
    try_import_completed_downloads, try_import_provided_completed_downloads,
    try_import_recent_completed_downloads,
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
    normalize_title_folder_template_or_default, render_title_folder_template,
    validate_season_folder_template, validate_title_folder_template,
};
pub use media::language::{
    normalize_detected_audio_language_code, normalize_detected_audio_languages,
    normalize_detected_subtitle_language_code, normalize_detected_subtitle_languages,
};
pub use media_requests::{
    ListMediaRequestsInput, SubmitMediaRequestInput, SubmitMediaRequestOutcome,
    UpdateMediaRequestInput,
};
pub use media_servers::{MediaServerConnectionDraft, MediaServerConnectionPatch};
pub use plugins::plugins::{
    ManualPluginPreview, PluginCatalogStatus, RegistryPlugin, RulePackRegistryEntry,
    RulePackTemplate,
};
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
    ExternalImportLibraryPathsSelection, FacetScoringPersonaSelection, GeneralSettings,
    IndexerRoutingSettingsEntry, LibraryPathsSettings, LibrarySettings,
    LibrarySettingsOverrideDraft, MediaSettings, QualityProfileSelection, QualityProfileSettings,
    RequestQualityProfileSettings, SaveQualityProfileSettings, SecuritySettings, ServiceSettings,
    SubtitleSettings, UpdateAutoBackupSettings, UpdateBackupSettings,
    UpdateFacetScoringPersonaSelection, UpdateGeneralSettings, UpdateLibraryPaths,
    UpdateMediaSettings, UpdateQualityProfileSelection, UpdateSecuritySettings,
    UpdateServiceSettings, UpdateSubtitleSettings,
};
pub use subtitles::orchestration::{
    DownloadSubtitleForMediaFileRequest, spawn_subtitle_search_for_file,
    start_background_subtitle_poller,
};

pub const DOWNLOAD_FEEDBACK_TIMEOUT_MESSAGE: &str =
    "download feedback timed out after 10s; queue status is temporarily unavailable";

pub(crate) const GLOBAL_LIBRARY_SCAN_ANALYSIS_CONCURRENCY: usize = 4;
pub use acquisition::release_search::release_strategy_kind_for_label;
pub use app_usecase_integration::publish_download_queue_snapshot_events;
#[cfg(unix)]
pub(crate) use helpers::statvfs_path;
pub(crate) use helpers::{
    INHERIT_QUALITY_PROFILE_VALUE, NATIVE_DOWNLOAD_CLIENT_TYPES, await_cancellable,
    await_cancellable_app_result, normalize_release_attempt_hint, normalize_release_attempt_title,
    normalize_release_selection_signature, normalize_show_text_opt, normalize_tags,
    parsed_episode_lookup_season, release_password_protection_hint, sanitize_ids, sha256_hex,
    to_hex,
};
pub use helpers::{accepted_inputs_for_client, nice_thread, normalize_release_password};
pub use jobs::definitions::{
    JobCategory, JobDefinition, JobKey, JobRun, JobRunRecord, JobRunStatus, JobRunTracker,
    JobScheduleInfo, JobScheduleKind, JobSection, JobTriggerSource, LibraryProbeSignature,
};
pub use library::user_delete::{DeletePreview, DeleteTitlesPreview};
pub use library_scan::{
    AnimeEpisodeMapping, AnimeMapping, AnimeMovie, BulkArtworkUrlResult, BulkMetadataResult,
    EpisodeArtworkUrls, EpisodeMetadata, LibraryDirectoryScanResult, LibraryFile, LibraryFileBatch,
    LibraryFileBatchReceiver, LibraryScanSummary, LibraryScanner, MetadataGateway,
    MetadataSearchItem, MetadataSearchQuery, MovieMetadata, MultiMetadataSearchResult,
    RichMetadataSearchItem, SeasonMetadata, SeriesArtworkUrls, SeriesMetadata, TitleArtworkUrls,
    source_signature_from_std_metadata,
};
pub use library_scan_progress::{
    LibraryScanMode, LibraryScanPhaseProgress, LibraryScanSession, LibraryScanStatus,
    LibraryScanTracker,
};
pub use media::analyzer::NativeMediaAnalyzer;
pub use notifications::dispatcher::start_notification_dispatcher;
pub use null_repositories::{
    NullAcquisitionStateRepository, NullBlocklistRepository, NullDomainEventRepository,
    NullDownloadQueueCommandRepository, NullDownloadSubmissionRepository,
    NullExternalImportMonitorSnapshotRepository, NullFileImporter, NullHousekeepingRepository,
    NullImportArtifactRepository, NullImportRepository, NullIndexerStatsTracker,
    NullJobRunRepository, NullLibraryProbeRepository, NullLibraryRepository,
    NullLibraryScanUnmatchedItemRepository, NullLogicalBackupExporter, NullMediaFileRepository,
    NullMediaRequestRepository, NullMediaServerConnectionRepository,
    NullNotificationChannelRepository, NullNotificationSubscriptionRepository, NullOAuthRepository,
    NullPendingReleaseRepository, NullPluginDescriptorLoader, NullPluginHttpTrustConfigRuntime,
    NullPluginInstallationRepository, NullPostProcessingScriptRepository, NullRuleSetRepository,
    NullSettingsRepository, NullStagedNzbStore, NullSubtitleDownloadRepository,
    NullSystemInfoProvider, NullTitleImageProcessor, NullTitleImageRepository,
    NullWantedItemRepository, NullWorkflowOperationRepository,
};
pub use ports::{
    AcquisitionStateRepository, BlocklistRepository, BuiltinDownloadClientConnectionTester,
    DatastoreInfo, DomainEventRepository, DownloadClient, DownloadClientConfigRepository,
    DownloadClientPluginProvider, DownloadQueueCommandRepository, DownloadSubmissionRepository,
    ExternalIdentityVerifier, ExternalImportMonitorSnapshotRepository, ExternalPluginWasm,
    FileImporter, HousekeepingMediaFileRootRow, HousekeepingRepository, ImportArtifactRepository,
    ImportFileTransferProgress, ImportFileTransferProgressSender, ImportRepository,
    IndexerCapsSnapshotRefresher, IndexerClient, IndexerConfigRepository, IndexerManagementClient,
    IndexerPluginProvider, IndexerStatsTracker, IndexerSystemBackoff, JellyfinServerUser,
    JobRunRepository, LibraryProbeRepository, LibraryRepository,
    LibraryScanUnmatchedItemRepository, LogicalBackupExporter, MediaAnalyzer, MediaFileRepository,
    MediaRequestQuery, MediaRequestRepository, MediaServerConnectionRepository, MediaServerUser,
    MediaServerUserGroup, MediaServerUserGroupStatus, NOTIFICATION_REQUEST_SCHEMA_VERSION,
    NewMediaRequest, NotificationActorPayload, NotificationAppPayload,
    NotificationApplicationUpdatePayload, NotificationChannelRepository, NotificationClient,
    NotificationDownloadPayload, NotificationEpisodePayload, NotificationExternalIdsPayload,
    NotificationFilePayload, NotificationHealthPayload, NotificationImportPayload,
    NotificationManualInteractionPayload, NotificationMediaFilePayload,
    NotificationMediaUpdatePayload, NotificationMediaUpdateTypePayload, NotificationPayload,
    NotificationPluginProvider, NotificationReleasePayload, NotificationSeverityPayload,
    NotificationSubscriptionRepository, NotificationTitlePayload, OAuthRepository,
    PendingReleaseRepository, PlexServerDiscovery, PlexServerUser, PluginDescriptorLoader,
    PluginHttpTrustConfigRuntime, PluginInstallationRepository, PostProcessingScriptRepository,
    QualityProfileRepository, ReleaseAttemptRepository, RuleSetRepository, RuntimePluginLoad,
    SettingsRepository, ShowRepository, StagedNzbStore, SubtitleDownloadRepository,
    SubtitlePluginProvider, SubtitleProviderClient, SubtitleProviderConfigRepository,
    SystemInfoProvider, TitleImageProcessor, TitleImageRepository, TitleRepository, TotpRepository,
    UserExternalAccountRepository, UserRepository, VerifiedExternalIdentity, WantedItemRepository,
    WebauthnRepository, WorkflowOperationInfo, WorkflowOperationRepository,
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
    BLOCK_SCORE, QUALITY_PROFILE_CATALOG_KEY, QUALITY_PROFILE_ID_KEY,
    QUALITY_PROFILE_INHERIT_VALUE, QualityProfile, QualityProfileCriteria, QualityProfileDecision,
    REQUEST_QUALITY_PROFILE_IDS_KEY, ScoringConfig, ScoringEntry, ScoringSource, apply_age_scoring,
    apply_size_scoring_for_category, default_quality_profile_8k_for_search,
    default_quality_profile_1080p_for_search, default_quality_profile_for_search,
    evaluate_against_profile, parse_profile_catalog_from_json,
};
pub use services::{
    AppServices, AppServicesBuilder, AppUseCase, ExternalImportMonitorWarmupBeginResult,
    ExternalImportMonitorWarmupPhase, ExternalImportMonitorWarmupPhaseProgress,
    ExternalImportMonitorWarmupProgressSnapshot, ExternalImportMonitorWarmupStatus,
    ProviderCatalogFamily,
};
pub use settings::keys::{
    ANIME_FILLER_POLICY_KEY, ANIME_INTER_SEASON_MOVIES_KEY, ANIME_MONITOR_FILLER_MOVIES_KEY,
    ANIME_MONITOR_SPECIALS_KEY, ANIME_PATH_KEY, ANIME_RECAP_POLICY_KEY, ANIME_ROOT_FOLDERS_KEY,
    AUDIO_PERSONA_MIGRATION_SENTINEL_KEY, AUTO_BACKUP_DAILY_TIME_LOCAL_KEY,
    AUTO_BACKUP_DISABLED_MISSING_KEY_NOTICE_KEY, AUTO_BACKUP_ENABLED_KEY, AUTO_BACKUP_KEY_KEY,
    AUTO_BACKUP_POST_UPGRADE_PENDING_VERSION_KEY, BACKUP_PATH_KEY, DEFAULT_ANIME_LIBRARY_PATH,
    DEFAULT_AUTO_BACKUP_DAILY_TIME_LOCAL, DEFAULT_FILLER_POLICY, DEFAULT_FOLDER_TEMPLATE_ANIME,
    DEFAULT_FOLDER_TEMPLATE_MOVIE, DEFAULT_FOLDER_TEMPLATE_SERIES, DEFAULT_MOVIE_LIBRARY_PATH,
    DEFAULT_RECAP_POLICY, DEFAULT_RENAME_COLLISION_POLICY, DEFAULT_RENAME_MISSING_METADATA_POLICY,
    DEFAULT_RENAME_TEMPLATE_ANIME, DEFAULT_RENAME_TEMPLATE_MOVIE, DEFAULT_RENAME_TEMPLATE_SERIES,
    DEFAULT_SEASON_FOLDER_TEMPLATE_ANIME, DEFAULT_SEASON_FOLDER_TEMPLATE_SERIES,
    DEFAULT_SERIES_LIBRARY_PATH, DOWNLOAD_CLIENT_DEFAULT_CATEGORY_SETTING_KEY,
    DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY, FOLDER_TEMPLATE_KEY, FORM_LOGIN_ENABLED_KEY,
    FRUITLESS_WANTED_RESET_LAST_RUN_KEY, HISTORY_KEEP_FOREVER_KEY, HISTORY_RETENTION_DAYS_KEY,
    IMPORT_MODE_KEY, INDEXER_ROUTING_SETTINGS_KEY, LEGACY_NZBGET_CATEGORY_SETTING_KEY,
    LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY, METADATA_LANGUAGE_KEY,
    MFA_REQUIRE_CONFIG_STEP_UP_KEY, MFA_REQUIRE_PASSWORD_LOGIN_KEY, MOVIES_PATH_KEY,
    MOVIES_ROOT_FOLDERS_KEY, NFO_WRITE_ON_IMPORT_ANIME_KEY, NFO_WRITE_ON_IMPORT_MOVIE_KEY,
    NFO_WRITE_ON_IMPORT_SERIES_KEY, NZBGET_OLDER_PRIORITY_SETTING_KEY,
    NZBGET_RECENT_PRIORITY_SETTING_KEY, PASSWORD_MIN_LENGTH_KEY, PASSWORD_MIN_LENGTH_MIN,
    PLEXMATCH_WRITE_ON_IMPORT_ANIME_KEY, PLEXMATCH_WRITE_ON_IMPORT_SERIES_KEY,
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
    SERIES_ROOT_FOLDERS_KEY, SETTINGS_SCOPE_MEDIA, SETTINGS_SCOPE_SYSTEM,
    SETTINGS_SOURCE_TYPED_GRAPHQL, SETUP_COMPLETE_KEY,
    SKIP_LOGIN_FOR_LOCAL_IPS_KEY, TITLE_REQUIRED_AUDIO_OVERRIDE_KEY, TLS_CERT_PATH_KEY,
    TLS_KEY_PATH_KEY, TOTP_REQUIRE_JELLYFIN_LOGIN_KEY,
};
pub(crate) use types::JwtClaims;
#[cfg(test)]
pub(crate) use types::ReleaseCandidateTokenClaims;
pub use types::{
    AddTitleAndQueueDownloadOutcome, AddTitleHydrationState, AddTitleOutcome,
    AuthenticatedTokenClaims, BackupDownloadTicket, BackupInfo, BackupStatus, BackupTrigger,
    CancelLibraryScanResult, CreateTitleOutcome, CutoffUnmetItem, CutoffUnmetQualitySummary,
    DecisionCodeCount, DiskSpaceInfo, DownloadActivityFilter, DownloadDisplayState,
    DownloadGrabResult, DownloadHistoryFilter, DownloadHistoryPage, DownloadHistorySort,
    DownloadHistorySortKey, DownloadImportFilter, DownloadImportPage, DownloadQueueCommandRecord,
    DownloadSourceKind, EpisodeScopedMediaFile, FixTitleMatchResult, HealthCheckResult,
    HealthCheckStatus, HousekeepingReport, IgnorePendingImportResult, IndexerQueryStats,
    JwtAuthConfig, JwtSessionScope, LibraryRootDraft, LibraryScanUnmatchedItem,
    LibraryScanUnmatchedSearchAttempt, LoginFailureTimingClass, MediaRequestCounts,
    OAuthAuthorizationCodeRecord, OAuthAuthorizationSource, OAuthConnectedAppRecord,
    OAuthRefreshGrantRecord, OAuthRefreshRotation, OAuthRefreshRotationOutcome,
    OAuthRefreshTokenRecord, PasskeySummary, PendingImportBindingFilePreview,
    PendingImportBindingPreview, PendingImportConnection, PendingImportCounts, PendingImportItem,
    PendingImportSearchAttempt, PendingImportStatus, PendingRelease, PendingReleaseStatus,
    PendingReleaseStatusCount, PendingTitleHydration, PrimaryCollectionSummary, RecycleBinSettings,
    RecycledItem, ReleaseDecision, ReleaseDownloadAttemptOutcome, ReleaseDownloadFailureSignature,
    ResolvePendingImportResult, RuntimePathStyle, ScopedExternalId, SortDirection, SystemHealth,
    TitleAcquisitionDiagnostics, TitleCatalogContentStatus, TitleCatalogFilter, TitleCatalogResult,
    TitleCatalogSort, TitleCatalogSortKey, TitleEpisodeProgressSummary, TitleImageBlob,
    TitleImageKind, TitleImageSourceResult, TitleImageSyncTask, TitleImageVariantRecord,
    TitleImageVariantSpec, TitleMediaFile, TitleMediaSizeSummary, TitleMetadataUpdate,
    TitleQualitySummary, TitleReleaseBlocklistEntry, TotpCredentialRecord,
    TotpEnrollmentChallengeRecord, TotpEnrollmentComplete, TotpEnrollmentStart,
    TotpFailedAttemptRecord, TotpRecoveryCodeRecord, TotpStatus, UpdateRecycleBinSettings,
    UserAuthFactorStatus, WantedCompleteTransition, WantedGrabTransition, WantedItem,
    WantedPauseTransition, WantedSearchTransition, WantedStatus, WantedStatusCount,
    WebauthnChallengeRecord, WebauthnChallengeStart, WebauthnChallengeType,
    WebauthnCredentialRecord,
};
pub use types::{
    ExternalIdHint, ExternalIdProvider, ExternalImportMonitorEpisodeEntry,
    ExternalImportMonitorMovieEntry, ExternalImportMonitorSeasonEntry,
    ExternalImportMonitorSeriesEntry, ExternalImportMonitorSnapshotChunk,
    ExternalImportMonitorSnapshotEntryKind, LibraryScanHint, LibraryScanHintFacet,
    LibraryScanHintSet, LibraryScanHintSource, library_scan_file_leaf_key,
    library_scan_folder_leaf_key,
};
pub use types::{
    IndexerSearchResponse, IndexerSearchResult, ReleaseCandidateProvenance,
    ReleaseSearchSubjectKind, ReleaseStrategyKind,
};
pub use types::{SmgScryerUpdateNotice, SmgVersionCompatibilityNotice};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("unauthorized: {0}")]
    Unauthorized(String),

    #[error("validation: {0}")]
    Validation(String),

    #[error("plugin install already in progress for '{0}'")]
    PluginInstallInProgress(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("{0}")]
    DownloadFeedbackTimeout(String),

    #[error("{0}")]
    DownloadSubmitAmbiguous(String),

    #[error("{0}")]
    DownloadSubmitUnavailable(String),

    #[error("{0}")]
    MfaStepUpRequired(String),

    #[error("{0}")]
    TotpEnrollmentRequired(String),

    #[error("{0}")]
    MfaEnrollmentRequired(String),

    #[error("{0}")]
    TotpInvalidCode(String),

    #[error("{0}")]
    TotpRecoveryCodeUsed(String),

    #[error("repository: {0}")]
    Repository(String),
}

impl AppError {
    pub fn download_submit_unavailable(message: impl Into<String>) -> Self {
        Self::DownloadSubmitUnavailable(message.into())
    }

    pub fn into_download_submit_unavailable(self) -> Self {
        match self {
            Self::DownloadSubmitUnavailable(_) | Self::DownloadSubmitAmbiguous(_) => self,
            _ => Self::DownloadSubmitUnavailable(self.to_string()),
        }
    }

    pub fn is_download_submit_unavailable(&self) -> bool {
        matches!(self, Self::DownloadSubmitUnavailable(_))
    }
}

#[cfg(test)]
mod lib_tests;

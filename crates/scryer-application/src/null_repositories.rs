use std::path::Path;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scryer_domain::ImportFileResult;
use scryer_domain::{
    AppPermissionMask, DomainEvent, DomainEventFilter, DomainEventType, ImportRecord, ImportStatus,
    ImportType, Library, LibraryGrant, MediaFacet, MediaRequest, NewDomainEvent,
    TitleHistoryEventType, User,
};

use scryer_domain::RuleSet;
use scryer_domain::{MaintenanceRuleRevision, MaintenanceRuleSet};

use crate::contracts::{
    ClientJobLocator, DownloadClientBindingRecord, DownloadRecord, ObservationResolution,
    ObservedClientJob,
};
use crate::ports::DownloadRegistryRepository;
use crate::ports::{
    DiscoveryHomeCandidate, DiscoveryHomeFilterOptions, DiscoveryHomeFilters,
    DiscoveryHomeSectionCandidatesRecord,
};
use crate::types::{
    ApiKeyRecord, OAuthClientRegistrationRecord, PendingImportStatus, PendingReleaseObservation,
    PendingReleaseRole, PendingReleaseStatus,
};
use crate::{
    AcquisitionScopeStatesQuery, AcquisitionStateRepository, IndexerErrorDetail, IndexerErrorPage,
    IndexerErrorRepository, InsertMediaFileInput, JellyfinServerUser, MediaRequestResolutionResult,
    MediaRequestSubmissionResult, MediaRequestUpdateResult, MediaServerConnectionRepository,
    NewIndexerError, PlexServerDiscovery, PlexServerUser, SuccessfulGrabCommit,
};
use scryer_domain::{PersistedPluginWasmPayload, PluginInstallation};

use scryer_domain::BlocklistEntry;

use crate::{
    AcquisitionScopeState, AcquisitionScopeStateRepository, AppError, AppResult,
    BlocklistRepository, BuiltinDownloadClientConnectionTester, CollectionEpisodeProgressSummary,
    CutoffUnmetQualitySummary, DiscoveryContextIncrementalCommit, DiscoveryContextSnapshotCommit,
    DiscoveryFacetRecord, DiscoveryItemRecord, DiscoveryItemsPageRecord,
    DiscoveryItemsStorageQuery, DiscoveryPendingContextChangeRecord, DiscoveryPruneReport,
    DiscoveryPublicFeedCommit, DiscoveryRepository, DiscoverySectionRecord,
    DiscoverySubmittedSubjectRecord, DiscoverySyncRunRecord, DiscoverySyncStateRecord,
    DomainEventRepository, DownloadQueueCommandRecord, DownloadQueueCommandRepository,
    DownloadSubmission, DownloadSubmissionRepository, ExternalIdentityVerifier,
    ExternalImportMonitorSnapshotRepository, ExternalImportSetupSecretDraft,
    ExternalImportSetupSecretDraftInput, ExternalImportSetupSecretDraftRepository,
    ExternalImportSetupSecretDraftSaveResult, ExternalImportSetupSecretDraftStatus, FileImporter,
    HousekeepingRepository, ImageProxyCacheControl, ImageProxyCacheEntryRecord,
    ImageProxyRegistration, ImageProxyRepository, ImageProxySourceRecord, ImportArtifact,
    ImportArtifactRepository, ImportRepository, IndexerQueryStats, IndexerSearchLearningKey,
    IndexerSearchLearningRecord, IndexerSearchLearningRepository, IndexerStatsTracker, JobKey,
    JobRunRecord, JobRunRepository, LibraryProbeRepository, LibraryProbeSignature,
    LibraryRepository, LibraryRootDraft, LibraryScanUnmatchedItem,
    LibraryScanUnmatchedItemRepository, MaintenanceRuleSetRepository, MediaFileRepository,
    MediaRequestCounts, MediaRequestQuery, MediaRequestRepository, MediaRequestResolution,
    NewBlocklistEntry, NewMediaRequest, NotificationChannelRepository,
    NotificationSubscriptionRepository, OAuthAuthorizationCodeRecord, OAuthConnectedAppRecord,
    OAuthRefreshGrantRecord, OAuthRefreshRotationOutcome, OAuthRefreshTokenRecord, OAuthRepository,
    PendingRelease, PendingReleaseRepository, PendingReleasesPageQuery, PendingStagedNzb,
    PluginDescriptorLoader, PluginInstallationRepository, PostProcessingScriptRepository,
    ProxyConfigRepository, ReleaseDecision, RuleSetRepository, SchedulerAdmission,
    SchedulerBatchDecision, SchedulerBatchRequest, SchedulerFeedback, SchedulerLease,
    SchedulerSnapshot, SchedulerSnapshotFilter, ScopeIndexerCoverageRepository,
    SeedingProfileRepository, SettingsRepository, StagedNzbRef, StagedNzbStore, SystemInfoProvider,
    TitleEpisodeProgressSummary, TitleImageBlob, TitleImageKind, TitleImageProcessor,
    TitleImageRepository, TitleImageSourceResult, TitleImageSyncTask, TitleImageVariantSpec,
    TitleMediaFile, TitleMediaSizeSummary, TitleMovieMediaSummary, TitleQualitySummary, UiSettings,
    UiSettingsUpdate, UpstreamScheduler, UserExternalAccountRepository, UserUiSettingsRepository,
    VerifiedExternalIdentity, WebauthnChallengeRecord, WebauthnCredentialRecord,
    WebauthnRepository, WorkflowOperationInfo, WorkflowOperationRepository,
    ports::CatalogDiscoveryCandidatesRecord, ports::DatastoreInfo, ports::LogicalBackupExporter,
    ports::TotpRepository, types::TotpCredentialRecord, types::TotpEnrollmentChallengeRecord,
    types::TotpFailedAttemptRecord, types::TotpRecoveryCodeRecord,
};

#[derive(Default)]
pub struct NullSeedingProfileRepository;

#[async_trait]
impl SeedingProfileRepository for NullSeedingProfileRepository {
    async fn list(&self) -> AppResult<Vec<scryer_domain::SeedingProfile>> {
        Ok(Vec::new())
    }

    async fn get_by_id(&self, _id: &str) -> AppResult<Option<scryer_domain::SeedingProfile>> {
        Ok(None)
    }

    async fn create(
        &self,
        profile: scryer_domain::SeedingProfile,
    ) -> AppResult<scryer_domain::SeedingProfile> {
        Ok(profile)
    }

    async fn update(
        &self,
        profile: scryer_domain::SeedingProfile,
    ) -> AppResult<scryer_domain::SeedingProfile> {
        Ok(profile)
    }

    async fn delete(&self, _id: &str) -> AppResult<()> {
        Ok(())
    }
}

#[derive(Default)]
pub struct NullProxyConfigRepository;

#[derive(Default)]
pub struct NullIndexerErrorRepository;

#[async_trait]
impl IndexerErrorRepository for NullIndexerErrorRepository {
    async fn record(&self, _error: NewIndexerError) -> AppResult<()> {
        Ok(())
    }

    async fn list(
        &self,
        _indexer_id: Option<&str>,
        _first: usize,
        _after: Option<&str>,
    ) -> AppResult<IndexerErrorPage> {
        Ok(IndexerErrorPage {
            items: Vec::new(),
            next_cursor: None,
        })
    }

    async fn get_detail(&self, _id: &str) -> AppResult<Option<IndexerErrorDetail>> {
        Ok(None)
    }

    async fn delete_older_than(&self, _cutoff: chrono::DateTime<chrono::Utc>) -> AppResult<u32> {
        Ok(0)
    }
}

#[async_trait]
impl ProxyConfigRepository for NullProxyConfigRepository {
    async fn list(
        &self,
        _provider_type: Option<scryer_domain::ProxyProviderType>,
    ) -> AppResult<Vec<scryer_domain::ProxyConfig>> {
        Ok(Vec::new())
    }

    async fn get_by_id(&self, _id: &str) -> AppResult<Option<scryer_domain::ProxyConfig>> {
        Ok(None)
    }

    async fn create(
        &self,
        config: scryer_domain::ProxyConfig,
    ) -> AppResult<scryer_domain::ProxyConfig> {
        Ok(config)
    }

    async fn update(
        &self,
        config: scryer_domain::ProxyConfig,
    ) -> AppResult<scryer_domain::ProxyConfig> {
        Ok(config)
    }

    async fn delete(&self, _id: &str) -> AppResult<()> {
        Ok(())
    }

    async fn record_health(
        &self,
        _id: &str,
        _status: scryer_domain::ProxyHealthStatus,
        _error_message: Option<String>,
        _error_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn pin_host_key(
        &self,
        _id: &str,
        _fingerprint: &str,
        _pinned_at: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn clear_host_key(&self, _id: &str) -> AppResult<()> {
        Ok(())
    }
}

#[derive(Default)]
pub struct NullScopeIndexerCoverageRepository;

#[async_trait]
impl ScopeIndexerCoverageRepository for NullScopeIndexerCoverageRepository {
    async fn record_coverage(
        &self,
        _scope_key: &str,
        _facet: &str,
        _indexer_id: &str,
        _fingerprint: &str,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn covered_indexers(
        &self,
        _scope_key: &str,
        _facet: &str,
        _fingerprint: &str,
        _stale_before: Option<chrono::DateTime<chrono::Utc>>,
    ) -> AppResult<Vec<String>> {
        Ok(Vec::new())
    }

    async fn prune_scope(&self, _scope_key: &str) -> AppResult<()> {
        Ok(())
    }

    async fn prune_scope_indexer(&self, _scope_key: &str, _indexer_id: &str) -> AppResult<()> {
        Ok(())
    }

    async fn list_coverage_for_scope_keys(
        &self,
        _scope_keys: &[String],
    ) -> AppResult<Vec<crate::ScopeCoverageRow>> {
        Ok(Vec::new())
    }
}

#[derive(Default)]
pub struct NullDiscoveryRepository;

#[async_trait]
impl DiscoveryRepository for NullDiscoveryRepository {
    async fn get_discovery_sync_state(
        &self,
        _scope_key: &str,
    ) -> AppResult<Option<DiscoverySyncStateRecord>> {
        Ok(None)
    }

    async fn upsert_discovery_sync_state(
        &self,
        _state: &DiscoverySyncStateRecord,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn try_acquire_discovery_sync_lease(
        &self,
        _scope_key: &str,
        _owner_id: &str,
        _lease_expires_at: DateTime<Utc>,
        _now: DateTime<Utc>,
    ) -> AppResult<bool> {
        Ok(true)
    }

    async fn renew_discovery_sync_lease(
        &self,
        _scope_key: &str,
        _owner_id: &str,
        _lease_expires_at: DateTime<Utc>,
        _now: DateTime<Utc>,
    ) -> AppResult<bool> {
        Ok(true)
    }

    async fn release_discovery_sync_lease(
        &self,
        _scope_key: &str,
        _owner_id: &str,
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn get_discovery_sync_run(&self, _id: &str) -> AppResult<Option<DiscoverySyncRunRecord>> {
        Ok(None)
    }

    async fn list_recent_discovery_sync_runs(
        &self,
        _limit: i64,
    ) -> AppResult<Vec<DiscoverySyncRunRecord>> {
        Ok(Vec::new())
    }

    async fn list_unacked_discovery_context_snapshot_runs(
        &self,
        _limit: i64,
    ) -> AppResult<Vec<DiscoverySyncRunRecord>> {
        Ok(Vec::new())
    }

    async fn upsert_discovery_sync_run(&self, _run: &DiscoverySyncRunRecord) -> AppResult<()> {
        Ok(())
    }

    async fn commit_discovery_context_snapshot(
        &self,
        _commit: &DiscoveryContextSnapshotCommit,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn commit_discovery_context_incremental(
        &self,
        _commit: &DiscoveryContextIncrementalCommit,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn commit_discovery_public_feed(
        &self,
        _commit: &DiscoveryPublicFeedCommit,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn replace_discovery_submitted_subjects(
        &self,
        _run_id: &str,
        _subjects: &[DiscoverySubmittedSubjectRecord],
    ) -> AppResult<()> {
        Ok(())
    }

    async fn list_discovery_submitted_subjects(
        &self,
        _run_id: &str,
    ) -> AppResult<Vec<DiscoverySubmittedSubjectRecord>> {
        Ok(Vec::new())
    }

    async fn upsert_pending_discovery_context_change(
        &self,
        _change: &DiscoveryPendingContextChangeRecord,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn get_pending_discovery_context_change(
        &self,
        _id: &str,
    ) -> AppResult<Option<DiscoveryPendingContextChangeRecord>> {
        Ok(None)
    }

    async fn delete_pending_discovery_context_change(&self, _id: &str) -> AppResult<u64> {
        Ok(0)
    }

    async fn list_all_pending_discovery_context_changes(
        &self,
        _scope_key: &str,
    ) -> AppResult<Vec<DiscoveryPendingContextChangeRecord>> {
        Ok(Vec::new())
    }

    async fn list_pending_discovery_context_changes(
        &self,
        _scope_key: &str,
        _limit: i64,
    ) -> AppResult<Vec<DiscoveryPendingContextChangeRecord>> {
        Ok(Vec::new())
    }

    async fn count_pending_discovery_context_changes(&self, _scope_key: &str) -> AppResult<i64> {
        Ok(0)
    }

    async fn clear_pending_discovery_context_changes_through_sequence(
        &self,
        _scope_key: &str,
        _last_seen_sequence: i64,
    ) -> AppResult<u64> {
        Ok(0)
    }

    async fn replace_discovery_sections(
        &self,
        _run_id: &str,
        _sections: &[DiscoverySectionRecord],
    ) -> AppResult<()> {
        Ok(())
    }

    async fn replace_discovery_items(
        &self,
        _run_id: &str,
        _items: &[DiscoveryItemRecord],
    ) -> AppResult<()> {
        Ok(())
    }

    async fn replace_discovery_facets(
        &self,
        _run_id: &str,
        _facets: &[DiscoveryFacetRecord],
    ) -> AppResult<()> {
        Ok(())
    }

    async fn list_discovery_sections(
        &self,
        _run_id: &str,
        _surface: Option<&str>,
    ) -> AppResult<Vec<DiscoverySectionRecord>> {
        Ok(Vec::new())
    }

    async fn list_public_discovery_section_items(
        &self,
        _run_id: &str,
        _allowed_media_kinds: &[String],
        _include_unresolved: bool,
        _filters: &DiscoveryHomeFilters,
        _limit_per_section: i64,
    ) -> AppResult<Vec<DiscoveryHomeSectionCandidatesRecord>> {
        Ok(Vec::new())
    }

    async fn list_personalized_discovery_home_items(
        &self,
        _run_id: &str,
        _readable_library_ids: &[String],
        _allowed_media_kinds: &[String],
        _include_unresolved: bool,
        _filters: &DiscoveryHomeFilters,
        _limit: i64,
    ) -> AppResult<Vec<DiscoveryHomeCandidate>> {
        Ok(Vec::new())
    }

    async fn list_personalized_complete_collection_items(
        &self,
        _run_id: &str,
        _readable_library_ids: &[String],
        _allowed_media_kinds: &[String],
        _include_unresolved: bool,
        _filters: &DiscoveryHomeFilters,
        _limit: i64,
    ) -> AppResult<Vec<DiscoveryHomeCandidate>> {
        Ok(Vec::new())
    }

    async fn list_personalized_discovery_facets(
        &self,
        _run_id: &str,
        _readable_library_ids: &[String],
        _allowed_media_kinds: &[String],
        _include_unresolved: bool,
    ) -> AppResult<Vec<DiscoveryFacetRecord>> {
        Ok(Vec::new())
    }

    async fn list_discovery_home_top_rated_items(
        &self,
        _public_run_id: Option<&str>,
        _context_run_id: Option<&str>,
        _readable_library_ids: &[String],
        _allowed_media_kinds: &[String],
        _owned_library_ids: &[String],
        _excluded_identity_keys: &[String],
        _include_unresolved: bool,
        _filters: &DiscoveryHomeFilters,
        _limit: i64,
    ) -> AppResult<Vec<DiscoveryHomeCandidate>> {
        Ok(Vec::new())
    }

    async fn hydrate_discovery_home_candidates(
        &self,
        _candidates: &mut [DiscoveryHomeCandidate],
    ) -> AppResult<()> {
        Ok(())
    }

    async fn hydrate_discovery_home_hero(
        &self,
        _candidate: &mut DiscoveryHomeCandidate,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn list_discovery_home_filter_options(
        &self,
        _public_run_id: Option<&str>,
        _context_run_id: Option<&str>,
        _readable_library_ids: &[String],
        _allowed_media_kinds: &[String],
        _include_unresolved: bool,
    ) -> AppResult<DiscoveryHomeFilterOptions> {
        Ok(DiscoveryHomeFilterOptions::default())
    }

    async fn list_catalog_public_discovery_items(
        &self,
        _run_id: &str,
        _owned_library_ids: &[String],
        _excluded_identity_keys: &[String],
        _media_kind: &str,
        _include_unresolved: bool,
        _limit: i64,
    ) -> AppResult<CatalogDiscoveryCandidatesRecord> {
        Ok(CatalogDiscoveryCandidatesRecord::default())
    }

    async fn list_catalog_public_discovery_sections(
        &self,
        _run_id: &str,
        _owned_library_ids: &[String],
        _excluded_identity_keys: &[String],
        _media_kind: &str,
        _include_unresolved: bool,
        _limit_per_section: i64,
    ) -> AppResult<Vec<crate::ports::CatalogDiscoverySectionCandidatesRecord>> {
        Ok(Vec::new())
    }

    async fn list_catalog_personalized_discovery_items(
        &self,
        _run_id: &str,
        _readable_library_ids: &[String],
        _media_kind: &str,
        _include_unresolved: bool,
        _limit: i64,
    ) -> AppResult<CatalogDiscoveryCandidatesRecord> {
        Ok(CatalogDiscoveryCandidatesRecord::default())
    }

    async fn query_discovery_items(
        &self,
        _query: &DiscoveryItemsStorageQuery,
    ) -> AppResult<DiscoveryItemsPageRecord> {
        Ok(DiscoveryItemsPageRecord {
            items: Vec::new(),
            total_count: 0,
        })
    }

    async fn replace_title_more_like_this_items(
        &self,
        _title_id: &str,
        _language: &str,
        _items: &[DiscoveryItemRecord],
    ) -> AppResult<()> {
        Ok(())
    }

    async fn list_title_more_like_this_items(
        &self,
        _title_id: &str,
        _limit: i64,
    ) -> AppResult<Vec<DiscoveryItemRecord>> {
        Ok(Vec::new())
    }

    async fn list_discovery_items_for_generation(
        &self,
        _base_generation_id: &str,
    ) -> AppResult<Vec<DiscoveryItemRecord>> {
        Ok(Vec::new())
    }

    async fn list_discovery_facets(&self, _run_id: &str) -> AppResult<Vec<DiscoveryFacetRecord>> {
        Ok(Vec::new())
    }

    async fn prune_discovery_history(
        &self,
        _scope_key: &str,
        _retain_successful_per_kind: usize,
        _diagnostic_cutoff: DateTime<Utc>,
    ) -> AppResult<DiscoveryPruneReport> {
        Ok(DiscoveryPruneReport::default())
    }
}

#[derive(Default)]
pub struct NullImportRepository;

#[async_trait]
impl ImportRepository for NullImportRepository {
    async fn queue_import_request(
        &self,
        _source_identity: ClientJobLocator,
        _import_type: String,
        _payload_json: String,
    ) -> AppResult<String> {
        Err(AppError::Repository(
            "import repository is not configured".to_string(),
        ))
    }
    async fn get_import_by_id(&self, _: &str) -> AppResult<Option<ImportRecord>> {
        Ok(None)
    }
    async fn update_import_status(
        &self,
        _: &str,
        _: ImportStatus,
        _: Option<String>,
    ) -> AppResult<()> {
        Ok(())
    }
    async fn update_import_transfer_progress(
        &self,
        _: &str,
        _: scryer_domain::ImportTransferPhase,
        _: i64,
        _: i64,
    ) -> AppResult<()> {
        Ok(())
    }
    async fn recover_stale_processing_imports(&self, _stale_seconds: i64) -> AppResult<u64> {
        Ok(0)
    }
    async fn recover_stale_processing_imports_for_type(
        &self,
        _: ImportType,
        _: i64,
    ) -> AppResult<u64> {
        Ok(0)
    }
    async fn list_pending_imports(&self) -> AppResult<Vec<ImportRecord>> {
        Ok(vec![])
    }
    async fn list_pending_imports_for_type(&self, _: ImportType) -> AppResult<Vec<ImportRecord>> {
        Ok(vec![])
    }
    async fn list_imports_for_identities(
        &self,
        _: &[ClientJobLocator],
    ) -> AppResult<Vec<ImportRecord>> {
        Ok(vec![])
    }
    async fn list_imports(&self, _limit: usize) -> AppResult<Vec<ImportRecord>> {
        Ok(vec![])
    }
}

#[derive(Default)]
pub struct NullExternalImportMonitorSnapshotRepository;

#[async_trait]
impl ExternalImportMonitorSnapshotRepository for NullExternalImportMonitorSnapshotRepository {
    async fn append_external_import_monitor_snapshot_chunk(
        &self,
        _: &crate::ExternalImportMonitorSnapshotChunk,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn list_external_import_monitor_snapshot_chunk_batch(
        &self,
        _: &str,
        _: crate::MediaFacet,
        _: crate::ExternalImportMonitorSnapshotEntryKind,
        _: Option<i32>,
        _: i32,
    ) -> AppResult<Vec<crate::ExternalImportMonitorSnapshotChunk>> {
        Ok(vec![])
    }

    async fn delete_external_import_monitor_snapshot_chunks(
        &self,
        _: &str,
        _: crate::MediaFacet,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn delete_external_import_monitor_snapshot_chunks_for_session_prefix(
        &self,
        _: &str,
        _: MediaFacet,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn delete_external_import_monitor_snapshot_chunks_except_session_prefix(
        &self,
        _: &str,
    ) -> AppResult<()> {
        Ok(())
    }
}

#[derive(Default)]
pub struct NullExternalImportSetupSecretDraftRepository;

#[async_trait]
impl ExternalImportSetupSecretDraftRepository for NullExternalImportSetupSecretDraftRepository {
    async fn get_for_owner(&self, _: &str) -> AppResult<Option<ExternalImportSetupSecretDraft>> {
        Ok(None)
    }

    async fn status_for_actor(&self, _: &str) -> AppResult<ExternalImportSetupSecretDraftStatus> {
        Ok(ExternalImportSetupSecretDraftStatus {
            has_draft: false,
            owned_by_current_user: false,
            updated_at: None,
        })
    }

    async fn save_for_owner(
        &self,
        _: &str,
        _: ExternalImportSetupSecretDraftInput,
    ) -> AppResult<ExternalImportSetupSecretDraftSaveResult> {
        Err(AppError::Repository(
            "external import setup secret draft repository is not configured".to_string(),
        ))
    }

    async fn clear_for_owner(&self, _: &str) -> AppResult<bool> {
        Ok(false)
    }
}

#[derive(Default)]
pub struct NullDownloadQueueCommandRepository;

#[async_trait]
impl DownloadQueueCommandRepository for NullDownloadQueueCommandRepository {
    async fn queue_delete_command(
        &self,
        _: Option<&str>,
        _: &str,
        _: &str,
        _: bool,
        _: Option<&str>,
    ) -> AppResult<DownloadQueueCommandRecord> {
        Err(AppError::Repository(
            "download queue command repository is not configured".to_string(),
        ))
    }

    async fn recover_stale_running_delete_commands(&self, _: i64) -> AppResult<u64> {
        Ok(0)
    }

    async fn list_pending_delete_commands(&self) -> AppResult<Vec<DownloadQueueCommandRecord>> {
        Ok(vec![])
    }

    async fn mark_delete_command_running(&self, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn mark_delete_command_completed(&self, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn mark_delete_command_failed(&self, _: &str, _: Option<&str>) -> AppResult<()> {
        Ok(())
    }

    async fn list_latest_delete_commands_for_sources(
        &self,
        _: &[(Option<String>, String, String, bool)],
        _: bool,
    ) -> AppResult<Vec<DownloadQueueCommandRecord>> {
        Ok(vec![])
    }

    async fn prune_terminal_delete_commands_older_than(&self, _: i64) -> AppResult<u32> {
        Ok(0)
    }
}

#[derive(Default)]
pub struct NullWorkflowOperationRepository;

#[async_trait]
impl WorkflowOperationRepository for NullWorkflowOperationRepository {
    async fn create_workflow_operation(
        &self,
        _operation_type: String,
        _status: String,
        _actor_user_id: Option<String>,
        _progress_json: Option<String>,
        _started_at: Option<String>,
        _completed_at: Option<String>,
    ) -> AppResult<WorkflowOperationInfo> {
        Err(AppError::Repository(
            "workflow operation repository is not configured".to_string(),
        ))
    }
}

#[derive(Default)]
pub struct NullLocationOperationRepository;

#[async_trait]
impl crate::ports::LocationOperationRepository for NullLocationOperationRepository {
    async fn create_location_operation(
        &self,
        _operation: &crate::location::model::LocationOperation,
        _plan_json: Option<&str>,
    ) -> AppResult<()> {
        Err(location_operation_repository_missing())
    }

    async fn get_location_operation(
        &self,
        _operation_id: &str,
    ) -> AppResult<Option<crate::location::model::LocationOperation>> {
        Err(location_operation_repository_missing())
    }

    async fn get_location_operation_plan_json(
        &self,
        _operation_id: &str,
    ) -> AppResult<Option<String>> {
        Err(location_operation_repository_missing())
    }

    async fn list_active_location_operations(
        &self,
    ) -> AppResult<Vec<crate::location::model::LocationOperation>> {
        Ok(Vec::new())
    }

    async fn update_location_operation_progress(
        &self,
        _progress: &crate::ports::LocationOperationProgress,
    ) -> AppResult<()> {
        Err(location_operation_repository_missing())
    }

    async fn set_location_operation_job_run(
        &self,
        _operation_id: &str,
        _job_run_id: &str,
    ) -> AppResult<()> {
        Err(location_operation_repository_missing())
    }

    async fn request_location_operation_cancel(&self, _operation_id: &str) -> AppResult<bool> {
        Err(location_operation_repository_missing())
    }

    async fn location_operation_cancel_requested(&self, _operation_id: &str) -> AppResult<bool> {
        Ok(false)
    }

    async fn upsert_location_title_checkpoint(
        &self,
        _checkpoint: &crate::location::model::TitleCheckpoint,
    ) -> AppResult<()> {
        Err(location_operation_repository_missing())
    }

    async fn list_location_title_checkpoints(
        &self,
        _operation_id: &str,
    ) -> AppResult<Vec<crate::location::model::TitleCheckpoint>> {
        Ok(Vec::new())
    }

    async fn record_location_file_verification(
        &self,
        _record: &crate::location::model::FileVerificationRecord,
    ) -> AppResult<()> {
        Err(location_operation_repository_missing())
    }

    async fn list_location_file_verifications(
        &self,
        _operation_id: &str,
        _title_id: Option<&str>,
    ) -> AppResult<Vec<crate::location::model::FileVerificationRecord>> {
        Ok(Vec::new())
    }

    async fn verified_destination_paths(
        &self,
        _operation_id: &str,
        _title_id: &str,
    ) -> AppResult<std::collections::BTreeSet<String>> {
        Ok(std::collections::BTreeSet::new())
    }

    async fn claim_location_operation_ownership(
        &self,
        _operation_id: &str,
        _entities: &[crate::location::ownership_guard::OwnedEntity],
    ) -> AppResult<crate::ports::LocationOwnershipOutcome> {
        Err(location_operation_repository_missing())
    }

    async fn release_location_operation_ownership(&self, _operation_id: &str) -> AppResult<u64> {
        Ok(0)
    }

    async fn location_ownership_holder(
        &self,
        _entity: &crate::location::ownership_guard::OwnedEntity,
    ) -> AppResult<Option<String>> {
        Ok(None)
    }

    async fn list_location_ownership_claims(
        &self,
    ) -> AppResult<Vec<crate::ports::LocationOwnershipClaim>> {
        Ok(Vec::new())
    }
}

/// Reads answer "nothing is happening" so a guard consulting an unconfigured
/// datastore never invents a conflict; writes fail loudly rather than pretending
/// an operation was persisted.
fn location_operation_repository_missing() -> AppError {
    AppError::Repository("location operation repository is not configured".to_string())
}

/// The US7 merge engine with no datastore behind it.
///
/// Unlike the location-operation null repository, **both** halves fail here.
/// There is no safe "nothing is happening" answer for a merge: an empty
/// snapshot would plan a merge over a destination the engine cannot see, which
/// is precisely the guess FR-066 exists to prevent. A deployment with no merge
/// store configured must refuse the merge, not perform an unexamined one.
#[derive(Default)]
pub struct NullTitleMergeRepository;

#[async_trait]
impl crate::location::merge::engine::TitleMergeRepository for NullTitleMergeRepository {
    async fn load_merge_snapshot(
        &self,
        _source_title_id: &str,
        _destination_title_id: &str,
        _current_operation_id: Option<&str>,
    ) -> AppResult<crate::location::merge::engine::MergeCatalogSnapshot> {
        Err(title_merge_repository_missing())
    }

    async fn execute_title_merge(
        &self,
        _plan: &crate::location::merge::engine::MergePlan,
    ) -> AppResult<crate::location::merge::engine::MergeOutcome> {
        Err(title_merge_repository_missing())
    }
}

fn title_merge_repository_missing() -> AppError {
    AppError::Repository("title merge repository is not configured".to_string())
}

#[derive(Default)]
pub struct NullMediaFileRepository;

#[async_trait]
impl MediaFileRepository for NullMediaFileRepository {
    async fn insert_media_file(&self, _input: &InsertMediaFileInput) -> AppResult<String> {
        Err(AppError::Repository(
            "media file repository is not configured".to_string(),
        ))
    }

    async fn claim_import_destination(
        &self,
        _input: &InsertMediaFileInput,
        _associations: &crate::MediaFileAssociations,
    ) -> AppResult<crate::ClaimedMediaFile> {
        Err(AppError::Repository(
            "media file repository is not configured".to_string(),
        ))
    }

    async fn link_file_to_episode(&self, _file_id: &str, _episode_id: &str) -> AppResult<()> {
        Err(AppError::Repository(
            "media file repository is not configured".to_string(),
        ))
    }

    async fn link_file_to_series_movie(
        &self,
        _file_id: &str,
        _series_movie_link_id: &str,
    ) -> AppResult<()> {
        Err(AppError::Repository(
            "media file repository is not configured".to_string(),
        ))
    }

    async fn list_media_files_for_title(&self, _title_id: &str) -> AppResult<Vec<TitleMediaFile>> {
        Err(AppError::Repository(
            "media file repository is not configured".to_string(),
        ))
    }

    async fn list_live_media_files_for_episode_ids(
        &self,
        _title_id: &str,
        _episode_ids: &[String],
    ) -> AppResult<Vec<crate::EpisodeScopedMediaFile>> {
        Err(AppError::Repository(
            "media file repository is not configured".to_string(),
        ))
    }

    async fn list_series_movie_link_ids_with_files_for_title(
        &self,
        _title_id: &str,
    ) -> AppResult<Vec<String>> {
        Ok(Vec::new())
    }

    async fn list_title_media_size_summaries(
        &self,
        _title_ids: &[String],
    ) -> AppResult<Vec<TitleMediaSizeSummary>> {
        Ok(Vec::new())
    }

    async fn collection_media_size_bytes(
        &self,
        _title_id: &str,
        _ordered_path: &str,
    ) -> AppResult<Option<i64>> {
        Ok(None)
    }

    async fn list_title_quality_summaries(
        &self,
        _title_ids: &[String],
    ) -> AppResult<Vec<TitleQualitySummary>> {
        Ok(Vec::new())
    }

    async fn list_title_movie_media_summaries(
        &self,
        _title_ids: &[String],
    ) -> AppResult<Vec<TitleMovieMediaSummary>> {
        Ok(Vec::new())
    }

    async fn list_cutoff_unmet_quality_summaries(
        &self,
        _title_ids: &[String],
    ) -> AppResult<Vec<CutoffUnmetQualitySummary>> {
        Ok(Vec::new())
    }

    async fn list_title_episode_progress_summaries(
        &self,
        _title_ids: &[String],
    ) -> AppResult<Vec<TitleEpisodeProgressSummary>> {
        Ok(Vec::new())
    }

    async fn list_collection_episode_progress_summaries(
        &self,
        _title_ids: &[String],
    ) -> AppResult<Vec<CollectionEpisodeProgressSummary>> {
        Ok(Vec::new())
    }

    async fn update_media_file_analysis(
        &self,
        _file_id: &str,
        _analysis: crate::MediaFileAnalysis,
    ) -> AppResult<()> {
        Err(AppError::Repository(
            "media file repository is not configured".to_string(),
        ))
    }

    async fn update_media_file_source_signature(
        &self,
        _file_id: &str,
        _size_bytes: i64,
        _source_signature_scheme: Option<String>,
        _source_signature_value: Option<String>,
    ) -> AppResult<()> {
        Err(AppError::Repository(
            "media file repository is not configured".to_string(),
        ))
    }

    async fn update_media_file_path(&self, _file_id: &str, _file_path: &str) -> AppResult<()> {
        Err(AppError::Repository(
            "media file repository is not configured".to_string(),
        ))
    }

    async fn set_media_file_roles_for_title(
        &self,
        _title_id: &str,
        _primary_file_id: &str,
        _additional_file_ids: &[String],
    ) -> AppResult<()> {
        Err(AppError::Repository(
            "media file repository is not configured".to_string(),
        ))
    }

    async fn set_media_file_roles_for_episode(
        &self,
        _title_id: &str,
        _episode_id: &str,
        _primary_file_id: &str,
        _additional_file_ids: &[String],
    ) -> AppResult<()> {
        Err(AppError::Repository(
            "media file repository is not configured".to_string(),
        ))
    }

    async fn mark_scan_failed(&self, _file_id: &str, _error: &str) -> AppResult<()> {
        Err(AppError::Repository(
            "media file repository is not configured".to_string(),
        ))
    }

    async fn delete_media_file(&self, _file_id: &str) -> AppResult<()> {
        Err(AppError::Repository(
            "media file repository is not configured".to_string(),
        ))
    }

    async fn get_media_file_by_id(&self, _file_id: &str) -> AppResult<Option<TitleMediaFile>> {
        Ok(None)
    }

    async fn get_media_file_by_path(&self, _file_path: &str) -> AppResult<Option<TitleMediaFile>> {
        Ok(None)
    }
}

#[derive(Default)]
pub struct NullFileImporter;

#[async_trait]
impl FileImporter for NullFileImporter {
    async fn snapshot_import_source(
        &self,
        _source: &Path,
    ) -> AppResult<scryer_domain::ImportSourceSnapshot> {
        Err(AppError::Repository(
            "file importer is not configured".to_string(),
        ))
    }

    async fn import_file(
        &self,
        _source: &Path,
        _dest: &Path,
        _mode: scryer_domain::ImportMode,
        _expected_source: Option<&scryer_domain::ImportSourceSnapshot>,
    ) -> AppResult<ImportFileResult> {
        Err(AppError::Repository(
            "file importer is not configured".to_string(),
        ))
    }

    async fn remove_import_source_after_verified_import(
        &self,
        _guard: scryer_domain::ImportSourceCleanupGuard,
        _final_dest_path: &Path,
    ) -> AppResult<()> {
        Err(AppError::Repository(
            "file importer is not configured".to_string(),
        ))
    }
}

#[derive(Default)]
pub struct NullTitleImageRepository;

#[async_trait]
impl TitleImageRepository for NullTitleImageRepository {
    async fn list_title_image_refresh_work(
        &self,
        _limit: usize,
        _skipped: &[TitleImageSyncTask],
    ) -> AppResult<Vec<TitleImageSyncTask>> {
        Ok(vec![])
    }

    async fn clear_title_image_cache(&self) -> AppResult<()> {
        Ok(())
    }

    async fn upsert_title_image_source_result(
        &self,
        _title_id: &str,
        _result: TitleImageSourceResult,
        _event: Option<NewDomainEvent>,
    ) -> AppResult<Option<DomainEvent>> {
        Err(AppError::Repository(
            "title image repository is not configured".to_string(),
        ))
    }

    async fn get_title_image_blob(
        &self,
        _title_id: &str,
        _kind: TitleImageKind,
        _variant_key: &str,
    ) -> AppResult<Option<TitleImageBlob>> {
        Ok(None)
    }
}

#[derive(Default)]
pub struct NullImageProxyRepository;

#[async_trait]
impl ImageProxyRepository for NullImageProxyRepository {
    fn register_image_source(&self, registration: ImageProxyRegistration) -> String {
        let token = crate::image_proxy_source_token(
            registration.upstream_url.as_deref(),
            registration.owner_type.as_deref(),
            registration.owner_id.as_deref(),
            registration.image_kind,
        );
        format!("/images/media/{token}/{}", registration.default_variant)
    }

    async fn flush_image_proxy_sources(&self) -> AppResult<()> {
        Ok(())
    }

    fn clear_image_proxy_memory(&self) {}

    async fn get_image_proxy_source(
        &self,
        _token: &str,
    ) -> AppResult<Option<ImageProxySourceRecord>> {
        Ok(None)
    }

    async fn get_image_proxy_cache_entry(
        &self,
        _token: &str,
        _variant: &str,
    ) -> AppResult<Option<ImageProxyCacheEntryRecord>> {
        Ok(None)
    }

    async fn upsert_image_proxy_cache_entry(
        &self,
        _entry: &ImageProxyCacheEntryRecord,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn touch_image_proxy_cache_entry(
        &self,
        _token: &str,
        _variant: &str,
        _observed_fetched_at: chrono::DateTime<chrono::Utc>,
        _last_accessed_at: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn delete_image_proxy_cache_entry(&self, _token: &str, _variant: &str) -> AppResult<()> {
        Ok(())
    }

    async fn list_image_proxy_cache_entries_lru(
        &self,
    ) -> AppResult<Vec<ImageProxyCacheEntryRecord>> {
        Ok(Vec::new())
    }

    async fn clear_image_proxy_cache_entries(&self) -> AppResult<()> {
        Ok(())
    }

    async fn prune_image_proxy_sources_before(
        &self,
        _cutoff: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<u64> {
        Ok(0)
    }
}

#[derive(Default)]
pub struct NullImageProxyCacheControl;

#[async_trait]
impl ImageProxyCacheControl for NullImageProxyCacheControl {
    async fn clear_cache(&self) -> AppResult<()> {
        Ok(())
    }

    async fn set_configured_max_bytes(&self, _value: u64) -> AppResult<()> {
        Ok(())
    }
}

#[derive(Default)]
pub struct NullTitleImageProcessor;

#[async_trait]
impl TitleImageProcessor for NullTitleImageProcessor {
    async fn fetch_and_process_image(
        &self,
        _kind: TitleImageKind,
        _source_url: &str,
        _variants: Vec<TitleImageVariantSpec>,
    ) -> AppResult<TitleImageSourceResult> {
        Err(AppError::Repository(
            "title image processor is not configured".to_string(),
        ))
    }
}

#[derive(Default)]
pub struct NullAcquisitionScopeStateRepository;

#[async_trait]
impl AcquisitionScopeStateRepository for NullAcquisitionScopeStateRepository {
    async fn upsert_acquisition_scope_state(
        &self,
        _item: &AcquisitionScopeState,
    ) -> AppResult<String> {
        Err(AppError::Repository(
            "wanted item repository is not configured".to_string(),
        ))
    }
    async fn update_acquisition_scope_status(
        &self,
        _id: &str,
        _status: &str,
        _last_search_at: Option<&str>,
        _grabbed_release: Option<&str>,
    ) -> AppResult<()> {
        Err(AppError::Repository(
            "wanted item repository is not configured".to_string(),
        ))
    }
    async fn record_acquisition_scope_search_attempt(
        &self,
        _id: &str,
        _last_search_at: &str,
    ) -> AppResult<()> {
        Ok(())
    }
    async fn get_acquisition_scope_state_for_title(
        &self,
        _title_id: &str,
        _episode_id: Option<&str>,
    ) -> AppResult<Option<AcquisitionScopeState>> {
        Ok(None)
    }
    async fn delete_acquisition_scope_states_for_title(&self, _title_id: &str) -> AppResult<()> {
        Ok(())
    }
    async fn delete_acquisition_scope_states_for_collection(
        &self,
        _collection_id: &str,
    ) -> AppResult<()> {
        Ok(())
    }
    async fn delete_acquisition_scope_states_for_series_movie_link(
        &self,
        _series_movie_link_id: &str,
    ) -> AppResult<()> {
        Ok(())
    }
    async fn delete_acquisition_scope_states_for_episode(
        &self,
        _episode_id: &str,
    ) -> AppResult<()> {
        Ok(())
    }
    async fn insert_release_decision(&self, _decision: &ReleaseDecision) -> AppResult<String> {
        Err(AppError::Repository(
            "wanted item repository is not configured".to_string(),
        ))
    }
    async fn get_acquisition_scope_state_by_id(
        &self,
        _id: &str,
    ) -> AppResult<Option<AcquisitionScopeState>> {
        Ok(None)
    }
    async fn list_acquisition_scope_states(
        &self,
        _query: AcquisitionScopeStatesQuery,
    ) -> AppResult<Vec<AcquisitionScopeState>> {
        Ok(vec![])
    }
    async fn count_acquisition_scope_states(
        &self,
        _query: AcquisitionScopeStatesQuery,
    ) -> AppResult<i64> {
        Ok(0)
    }
    async fn list_release_decisions_for_title(
        &self,
        _title_id: &str,
        _limit: i64,
        _offset: i64,
    ) -> AppResult<Vec<ReleaseDecision>> {
        Ok(vec![])
    }
    async fn list_release_decisions_for_acquisition_scope_state(
        &self,
        _wanted_item_id: &str,
        _limit: i64,
        _offset: i64,
    ) -> AppResult<Vec<ReleaseDecision>> {
        Ok(vec![])
    }
    async fn count_release_decisions_for_title(&self, _title_id: &str) -> AppResult<i64> {
        Ok(0)
    }
    async fn count_release_decisions_for_acquisition_scope_state(
        &self,
        _wanted_item_id: &str,
    ) -> AppResult<i64> {
        Ok(0)
    }
}

#[derive(Default)]
pub struct NullRuleSetRepository;

#[async_trait]
impl RuleSetRepository for NullRuleSetRepository {
    async fn list_rule_sets(&self) -> AppResult<Vec<RuleSet>> {
        Ok(vec![])
    }
    async fn list_enabled_rule_sets(&self) -> AppResult<Vec<RuleSet>> {
        Ok(vec![])
    }
    async fn get_rule_set(&self, _id: &str) -> AppResult<Option<RuleSet>> {
        Ok(None)
    }
    async fn create_rule_set(&self, _rule_set: &RuleSet) -> AppResult<()> {
        Err(AppError::Repository(
            "rule set repository is not configured".to_string(),
        ))
    }
    async fn update_rule_set(&self, _rule_set: &RuleSet) -> AppResult<()> {
        Err(AppError::Repository(
            "rule set repository is not configured".to_string(),
        ))
    }
    async fn delete_rule_set(&self, _id: &str) -> AppResult<()> {
        Err(AppError::Repository(
            "rule set repository is not configured".to_string(),
        ))
    }
    async fn record_rule_set_history(
        &self,
        _rule_set_id: &str,
        _action: &str,
        _rego_source: Option<&str>,
        _actor_id: Option<&str>,
    ) -> AppResult<()> {
        Ok(())
    }
    async fn get_rule_set_by_managed_key(&self, _key: &str) -> AppResult<Option<RuleSet>> {
        Ok(None)
    }
    async fn delete_rule_set_by_managed_key(&self, _key: &str) -> AppResult<()> {
        Ok(())
    }
    async fn list_rule_sets_by_managed_key_prefix(&self, _prefix: &str) -> AppResult<Vec<RuleSet>> {
        Ok(vec![])
    }
}

#[derive(Default)]
pub struct NullMaintenanceRuleSetRepository;

#[async_trait]
impl MaintenanceRuleSetRepository for NullMaintenanceRuleSetRepository {
    async fn list_rule_sets(&self) -> AppResult<Vec<MaintenanceRuleSet>> {
        Ok(vec![])
    }
    async fn get_rule_set(&self, _id: &str) -> AppResult<Option<MaintenanceRuleSet>> {
        Ok(None)
    }
    async fn create_rule_set(
        &self,
        _rule_set: &MaintenanceRuleSet,
        _revision: &MaintenanceRuleRevision,
    ) -> AppResult<()> {
        Err(AppError::Repository(
            "maintenance rule repository is not configured".to_string(),
        ))
    }
    async fn add_revision(
        &self,
        _revision: &MaintenanceRuleRevision,
        _updated_at: DateTime<Utc>,
    ) -> AppResult<()> {
        Err(AppError::Repository(
            "maintenance rule repository is not configured".to_string(),
        ))
    }
    async fn get_revision(
        &self,
        _rule_set_id: &str,
        _revision_number: i64,
    ) -> AppResult<Option<MaintenanceRuleRevision>> {
        Ok(None)
    }
    async fn list_revisions(&self, _rule_set_id: &str) -> AppResult<Vec<MaintenanceRuleRevision>> {
        Ok(vec![])
    }
    async fn update_rule_set_metadata(
        &self,
        _id: &str,
        _name: &str,
        _description: &str,
        _library_ids: &[String],
        _disarm: bool,
        _updated_at: DateTime<Utc>,
    ) -> AppResult<()> {
        Err(AppError::Repository(
            "maintenance rule repository is not configured".to_string(),
        ))
    }
    async fn delete_rule_set(&self, _id: &str) -> AppResult<()> {
        Err(AppError::Repository(
            "maintenance rule repository is not configured".to_string(),
        ))
    }
    async fn update_rule_set_evaluation_mode(
        &self,
        _id: &str,
        _mode: scryer_domain::MaintenanceEvaluationMode,
        _enabled: bool,
        _updated_at: DateTime<Utc>,
    ) -> AppResult<()> {
        Err(AppError::Repository(
            "maintenance rule repository is not configured".to_string(),
        ))
    }
    async fn update_rule_set_arming(
        &self,
        _id: &str,
        _arming: scryer_domain::MaintenanceEffectArming,
        _updated_at: DateTime<Utc>,
    ) -> AppResult<()> {
        Err(AppError::Repository(
            "maintenance rule repository is not configured".to_string(),
        ))
    }
}

/// Reads answer empty and writes refuse: an assembly with no maintenance
/// evaluation store has no candidates, so the evaluator finds nothing to
/// reconcile rather than silently dropping rows it believed it wrote.
#[derive(Default)]
pub struct NullMaintenanceEvaluationRepository;

const MAINTENANCE_EVALUATION_NOT_CONFIGURED: &str =
    "maintenance evaluation repository is not configured";

#[async_trait]
impl crate::ports::MaintenanceCandidateRepository for NullMaintenanceEvaluationRepository {
    async fn get_active_candidate(
        &self,
        _rule_set_id: &str,
        _title_id: &str,
    ) -> AppResult<Option<scryer_domain::LifecycleCandidate>> {
        Ok(None)
    }
    async fn list_candidates(
        &self,
        _query: &crate::ports::MaintenanceCandidateQuery,
    ) -> AppResult<Vec<scryer_domain::LifecycleCandidate>> {
        Ok(vec![])
    }
    async fn max_match_generation(&self, _rule_set_id: &str, _title_id: &str) -> AppResult<i64> {
        Ok(0)
    }
    async fn create_candidate(
        &self,
        _candidate: &scryer_domain::LifecycleCandidate,
    ) -> AppResult<()> {
        Err(AppError::Repository(
            MAINTENANCE_EVALUATION_NOT_CONFIGURED.to_string(),
        ))
    }
    async fn record_candidate_match(
        &self,
        _id: &str,
        _last_matched_at: DateTime<Utc>,
        _reason_codes: &[String],
        _updated_at: DateTime<Utc>,
    ) -> AppResult<()> {
        Err(AppError::Repository(
            MAINTENANCE_EVALUATION_NOT_CONFIGURED.to_string(),
        ))
    }
    async fn hold_candidate(
        &self,
        _id: &str,
        _held_since: DateTime<Utc>,
        _updated_at: DateTime<Utc>,
    ) -> AppResult<()> {
        Err(AppError::Repository(
            MAINTENANCE_EVALUATION_NOT_CONFIGURED.to_string(),
        ))
    }
    async fn transition_candidate_state(
        &self,
        _id: &str,
        _state: scryer_domain::MaintenanceCandidateState,
        _state_reason: &str,
        _expected_states: &[scryer_domain::MaintenanceCandidateState],
        _updated_at: DateTime<Utc>,
    ) -> AppResult<bool> {
        // A refusal, not `Ok(false)`: an unconfigured store never wrote the row,
        // which is a different thing from losing a race for it.
        Err(AppError::Repository(
            MAINTENANCE_EVALUATION_NOT_CONFIGURED.to_string(),
        ))
    }
    async fn cancel_active_candidates_for_rule(
        &self,
        _rule_set_id: &str,
        _state_reason: &str,
        _updated_at: DateTime<Utc>,
    ) -> AppResult<u64> {
        Ok(0)
    }
    async fn count_candidates_by_state(
        &self,
        _rule_set_id: &str,
    ) -> AppResult<Vec<(scryer_domain::MaintenanceCandidateState, i64)>> {
        Ok(vec![])
    }
    async fn list_due_candidates(
        &self,
        _rule_set_id: &str,
        _due_before: DateTime<Utc>,
        _stale_before: DateTime<Utc>,
        _limit: usize,
    ) -> AppResult<Vec<scryer_domain::LifecycleCandidate>> {
        Ok(vec![])
    }
    async fn lease_candidate_for_execution(
        &self,
        _id: &str,
        _stale_before: DateTime<Utc>,
        _updated_at: DateTime<Utc>,
    ) -> AppResult<bool> {
        Ok(false)
    }
    async fn record_candidate_attempts(
        &self,
        _id: &str,
        _action_attempts: i64,
        _updated_at: DateTime<Utc>,
    ) -> AppResult<()> {
        Err(AppError::Repository(
            MAINTENANCE_EVALUATION_NOT_CONFIGURED.to_string(),
        ))
    }
}

#[async_trait]
impl crate::ports::LifecycleActionRunRepository for NullMaintenanceEvaluationRepository {
    async fn start_action_run(&self, _run: &scryer_domain::LifecycleActionRun) -> AppResult<()> {
        Err(AppError::Repository(
            MAINTENANCE_EVALUATION_NOT_CONFIGURED.to_string(),
        ))
    }
    async fn finish_action_run(&self, _run: &scryer_domain::LifecycleActionRun) -> AppResult<()> {
        Err(AppError::Repository(
            MAINTENANCE_EVALUATION_NOT_CONFIGURED.to_string(),
        ))
    }
    async fn list_action_runs(
        &self,
        _rule_set_id: Option<&str>,
        _candidate_id: Option<&str>,
        _limit: Option<usize>,
    ) -> AppResult<Vec<scryer_domain::LifecycleActionRun>> {
        Ok(vec![])
    }
}

#[async_trait]
impl crate::ports::MaintenanceExclusionRepository for NullMaintenanceEvaluationRepository {
    async fn list_exclusions(
        &self,
        _rule_set_id: Option<&str>,
    ) -> AppResult<Vec<scryer_domain::MaintenanceRuleExclusion>> {
        Ok(vec![])
    }
    async fn get_exclusion(
        &self,
        _id: &str,
    ) -> AppResult<Option<scryer_domain::MaintenanceRuleExclusion>> {
        Ok(None)
    }
    async fn create_exclusion(
        &self,
        _exclusion: &scryer_domain::MaintenanceRuleExclusion,
    ) -> AppResult<()> {
        Err(AppError::Repository(
            MAINTENANCE_EVALUATION_NOT_CONFIGURED.to_string(),
        ))
    }
    async fn delete_exclusion(&self, _id: &str) -> AppResult<()> {
        Err(AppError::Repository(
            MAINTENANCE_EVALUATION_NOT_CONFIGURED.to_string(),
        ))
    }
}

#[async_trait]
impl crate::ports::MaintenanceEvaluationRunRepository for NullMaintenanceEvaluationRepository {
    async fn start_evaluation_run(
        &self,
        _run: &scryer_domain::MaintenanceEvaluationRun,
    ) -> AppResult<()> {
        Err(AppError::Repository(
            MAINTENANCE_EVALUATION_NOT_CONFIGURED.to_string(),
        ))
    }
    async fn finish_evaluation_run(
        &self,
        _run: &scryer_domain::MaintenanceEvaluationRun,
    ) -> AppResult<()> {
        Err(AppError::Repository(
            MAINTENANCE_EVALUATION_NOT_CONFIGURED.to_string(),
        ))
    }
    async fn list_evaluation_runs(
        &self,
        _rule_set_id: Option<&str>,
        _limit: Option<usize>,
    ) -> AppResult<Vec<scryer_domain::MaintenanceEvaluationRun>> {
        Ok(vec![])
    }
}

#[derive(Default)]
pub struct NullPostProcessingScriptRepository;

#[async_trait]
impl PostProcessingScriptRepository for NullPostProcessingScriptRepository {
    async fn list_scripts(&self) -> AppResult<Vec<scryer_domain::PostProcessingScript>> {
        Ok(vec![])
    }
    async fn get_script(
        &self,
        _id: &str,
    ) -> AppResult<Option<scryer_domain::PostProcessingScript>> {
        Ok(None)
    }
    async fn create_script(
        &self,
        _script: scryer_domain::PostProcessingScript,
    ) -> AppResult<scryer_domain::PostProcessingScript> {
        Err(AppError::Repository(
            "post-processing script repository is not configured".to_string(),
        ))
    }
    async fn update_script(
        &self,
        _script: scryer_domain::PostProcessingScript,
    ) -> AppResult<scryer_domain::PostProcessingScript> {
        Err(AppError::Repository(
            "post-processing script repository is not configured".to_string(),
        ))
    }
    async fn delete_script(&self, _id: &str) -> AppResult<()> {
        Err(AppError::Repository(
            "post-processing script repository is not configured".to_string(),
        ))
    }
    async fn list_enabled_for_facet(
        &self,
        _facet: &str,
    ) -> AppResult<Vec<scryer_domain::PostProcessingScript>> {
        Ok(vec![])
    }
    async fn record_run(&self, _run: scryer_domain::PostProcessingScriptRun) -> AppResult<()> {
        Ok(())
    }
    async fn list_runs_for_script(
        &self,
        _script_id: &str,
        _limit: usize,
    ) -> AppResult<Vec<scryer_domain::PostProcessingScriptRun>> {
        Ok(vec![])
    }
    async fn list_runs_for_title(
        &self,
        _title_id: &str,
        _limit: usize,
    ) -> AppResult<Vec<scryer_domain::PostProcessingScriptRun>> {
        Ok(vec![])
    }
}

#[derive(Default)]
pub struct NullPluginInstallationRepository;

pub struct NullBuiltinDownloadClientConnectionTester;

#[async_trait]
impl BuiltinDownloadClientConnectionTester for NullBuiltinDownloadClientConnectionTester {
    async fn test_connection(
        &self,
        _client_type: &str,
        _config_json: &str,
        _proxy_config: Option<&scryer_domain::ProxyConfig>,
    ) -> AppResult<()> {
        Err(AppError::Repository(
            "download client connection tester is not configured".to_string(),
        ))
    }
}

pub struct NullPluginDescriptorLoader;

impl PluginDescriptorLoader for NullPluginDescriptorLoader {
    fn load_descriptor_from_wasm_bytes(
        &self,
        _wasm_bytes: &[u8],
    ) -> AppResult<scryer_plugin_sdk::PluginDescriptor> {
        Err(AppError::Repository(
            "plugin descriptor loader is not configured".to_string(),
        ))
    }
}

#[async_trait]
impl PluginInstallationRepository for NullPluginInstallationRepository {
    async fn list_plugin_installations(&self) -> AppResult<Vec<PluginInstallation>> {
        Ok(vec![])
    }
    async fn get_plugin_installation(
        &self,
        _plugin_id: &str,
    ) -> AppResult<Option<PluginInstallation>> {
        Ok(None)
    }
    async fn create_plugin_installation(
        &self,
        _installation: &PluginInstallation,
        _wasm_bytes: Option<&[u8]>,
    ) -> AppResult<PluginInstallation> {
        Err(AppError::Repository(
            "plugin installation repository is not configured".to_string(),
        ))
    }
    async fn update_plugin_installation(
        &self,
        _installation: &PluginInstallation,
        _wasm_bytes: Option<&[u8]>,
    ) -> AppResult<PluginInstallation> {
        Err(AppError::Repository(
            "plugin installation repository is not configured".to_string(),
        ))
    }
    async fn delete_plugin_installation(&self, _plugin_id: &str) -> AppResult<()> {
        Err(AppError::Repository(
            "plugin installation repository is not configured".to_string(),
        ))
    }
    async fn get_enabled_plugin_wasm_bytes(
        &self,
    ) -> AppResult<Vec<(PluginInstallation, Option<PersistedPluginWasmPayload>)>> {
        Ok(vec![])
    }
    async fn get_plugin_installation_wasm_payload(
        &self,
        _plugin_id: &str,
    ) -> AppResult<Option<PersistedPluginWasmPayload>> {
        Ok(None)
    }
    async fn seed_builtin(
        &self,
        _plugin_id: &str,
        _name: &str,
        _description: &str,
        _version: &str,
        _sdk_version: &str,
        _sdk_constraint: &str,
        _plugin_type: &str,
        _provider_type: &str,
    ) -> AppResult<()> {
        Ok(())
    }
    async fn upsert_plugin_catalog_source(
        &self,
        _source: &scryer_domain::PluginCatalogSource,
    ) -> AppResult<()> {
        Ok(())
    }
    async fn delete_plugin_catalog_source(&self, _source_key: &str) -> AppResult<()> {
        Ok(())
    }
    async fn list_plugin_catalog_sources(
        &self,
    ) -> AppResult<Vec<scryer_domain::PluginCatalogSource>> {
        Ok(vec![])
    }
    async fn get_plugin_catalog_source(
        &self,
        _source_key: &str,
    ) -> AppResult<Option<scryer_domain::PluginCatalogSource>> {
        Ok(None)
    }
    async fn upsert_plugin_catalog_status(
        &self,
        _status: &scryer_domain::PluginCatalogStatusRecord,
    ) -> AppResult<()> {
        Ok(())
    }
    async fn get_plugin_catalog_status(
        &self,
        _status_key: &str,
    ) -> AppResult<Option<scryer_domain::PluginCatalogStatusRecord>> {
        Ok(None)
    }
}

#[derive(Default)]
pub struct NullSystemInfoProvider;

#[async_trait]
impl SystemInfoProvider for NullSystemInfoProvider {
    async fn datastore_info(&self) -> AppResult<DatastoreInfo> {
        Ok(DatastoreInfo {
            engine: "unknown".to_string(),
            current_migration_key: None,
        })
    }

    async fn current_migration_version(&self) -> AppResult<Option<String>> {
        Ok(None)
    }
    async fn current_encryption_key_base64(&self) -> AppResult<Option<String>> {
        Ok(None)
    }
}

#[derive(Default)]
pub struct NullPluginHttpTrustConfigRuntime;

impl crate::PluginHttpTrustConfigRuntime for NullPluginHttpTrustConfigRuntime {
    fn set_plugin_http_ca_bundle_pem(&self, _bundle_pem: String) -> AppResult<()> {
        Ok(())
    }
}

#[derive(Default)]
pub struct NullLogicalBackupExporter;

#[async_trait]
impl LogicalBackupExporter for NullLogicalBackupExporter {
    async fn export_backup_bundle(
        &self,
        _request: crate::BackupBundleExportRequest,
    ) -> AppResult<crate::BackupExportOutcome> {
        Err(AppError::Repository(
            "logical backup exporter is not configured".to_string(),
        ))
    }
}

#[derive(Default)]
pub struct NullIndexerStatsTracker;

impl IndexerStatsTracker for NullIndexerStatsTracker {
    fn record_query(&self, _indexer_id: &str, _indexer_name: &str, _success: bool) {}
    fn record_grab(&self, _indexer_id: &str, _indexer_name: &str) {}
    fn record_api_limits(
        &self,
        _indexer_id: &str,
        _api_current: Option<u32>,
        _api_max: Option<u32>,
        _grab_current: Option<u32>,
        _grab_max: Option<u32>,
    ) {
    }
    fn all_stats(&self) -> Vec<IndexerQueryStats> {
        vec![]
    }
}

#[derive(Default)]
pub struct NullUpstreamScheduler;

#[async_trait]
impl UpstreamScheduler for NullUpstreamScheduler {
    async fn admit_batch(
        &self,
        request: SchedulerBatchRequest,
    ) -> AppResult<SchedulerBatchDecision> {
        let decisions = request
            .candidates
            .into_iter()
            .map(|candidate| SchedulerAdmission::Admit {
                candidate_id: candidate.candidate_id.clone(),
                lease: SchedulerLease {
                    lease_id: candidate.candidate_id.to_string(),
                    candidate_id: candidate.candidate_id,
                    host_key: candidate.host_key,
                    destination_key: candidate.destination_key,
                    account_quota_key: candidate.account_quota_key,
                    rss_request_key: candidate.rss_request_key,
                    operation: candidate.operation,
                    intent: candidate.intent,
                    issued_at: request.now,
                },
                reason: crate::AdmissionReason::BackgroundValue,
            })
            .collect();
        Ok(SchedulerBatchDecision {
            batch_id: request.batch_id,
            decisions,
        })
    }

    async fn record_feedback(&self, _feedback: SchedulerFeedback) -> AppResult<()> {
        Ok(())
    }

    async fn snapshot(&self, _filter: SchedulerSnapshotFilter) -> AppResult<SchedulerSnapshot> {
        Ok(SchedulerSnapshot::default())
    }
}

#[derive(Default)]
pub struct NullIndexerSearchLearningRepository;

#[async_trait]
impl IndexerSearchLearningRepository for NullIndexerSearchLearningRepository {
    async fn list_for_title(
        &self,
        _indexer_id: &str,
        _title_id: &str,
        _facet: &str,
    ) -> AppResult<Vec<IndexerSearchLearningRecord>> {
        Ok(vec![])
    }

    async fn record_outcome(
        &self,
        key: &IndexerSearchLearningKey,
        usable_hits: u32,
    ) -> AppResult<IndexerSearchLearningRecord> {
        Ok(IndexerSearchLearningRecord {
            key: key.clone(),
            attempts: 1,
            empty_successes: u32::from(usable_hits == 0),
            usable_successes: u32::from(usable_hits > 0),
            last_attempt_at: None,
            last_usable_at: None,
            suppressed: false,
            updated_at: None,
        })
    }

    async fn set_suppressed(
        &self,
        _key: &IndexerSearchLearningKey,
        _suppressed: bool,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn try_claim_suppressed_reprobe(
        &self,
        _key: &IndexerSearchLearningKey,
        _stale_before: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<bool> {
        Ok(false)
    }
}

#[derive(Default)]
pub struct NullNotificationChannelRepository;

#[async_trait]
impl NotificationChannelRepository for NullNotificationChannelRepository {
    async fn list_channels(&self) -> AppResult<Vec<scryer_domain::NotificationChannelConfig>> {
        Ok(vec![])
    }
    async fn get_channel(
        &self,
        _id: &str,
    ) -> AppResult<Option<scryer_domain::NotificationChannelConfig>> {
        Ok(None)
    }
    async fn create_channel(
        &self,
        _config: scryer_domain::NotificationChannelConfig,
    ) -> AppResult<scryer_domain::NotificationChannelConfig> {
        Err(AppError::Repository(
            "notification channel repository is not configured".to_string(),
        ))
    }
    async fn update_channel(
        &self,
        _config: scryer_domain::NotificationChannelConfig,
    ) -> AppResult<scryer_domain::NotificationChannelConfig> {
        Err(AppError::Repository(
            "notification channel repository is not configured".to_string(),
        ))
    }
    async fn delete_channel(&self, _id: &str) -> AppResult<()> {
        Err(AppError::Repository(
            "notification channel repository is not configured".to_string(),
        ))
    }
}

#[derive(Default)]
pub struct NullNotificationSubscriptionRepository;

#[async_trait]
impl NotificationSubscriptionRepository for NullNotificationSubscriptionRepository {
    async fn list_subscriptions(&self) -> AppResult<Vec<scryer_domain::NotificationSubscription>> {
        Ok(vec![])
    }
    async fn list_subscriptions_for_channel(
        &self,
        _channel_id: &str,
    ) -> AppResult<Vec<scryer_domain::NotificationSubscription>> {
        Ok(vec![])
    }
    async fn list_subscriptions_for_target(
        &self,
        _target_kind: scryer_domain::NotificationTargetKind,
        _target_id: &str,
    ) -> AppResult<Vec<scryer_domain::NotificationSubscription>> {
        Ok(vec![])
    }
    async fn list_subscriptions_for_event(
        &self,
        _event_type: scryer_domain::NotificationEventType,
    ) -> AppResult<Vec<scryer_domain::NotificationSubscription>> {
        Ok(vec![])
    }
    async fn create_subscription(
        &self,
        _sub: scryer_domain::NotificationSubscription,
    ) -> AppResult<scryer_domain::NotificationSubscription> {
        Err(AppError::Repository(
            "notification subscription repository is not configured".to_string(),
        ))
    }
    async fn update_subscription(
        &self,
        _sub: scryer_domain::NotificationSubscription,
    ) -> AppResult<scryer_domain::NotificationSubscription> {
        Err(AppError::Repository(
            "notification subscription repository is not configured".to_string(),
        ))
    }
    async fn delete_subscription(&self, _id: &str) -> AppResult<()> {
        Err(AppError::Repository(
            "notification subscription repository is not configured".to_string(),
        ))
    }
}

#[derive(Default)]
pub struct NullDomainEventRepository;

#[async_trait]
impl DomainEventRepository for NullDomainEventRepository {
    async fn append(&self, _: NewDomainEvent) -> AppResult<DomainEvent> {
        Err(AppError::Repository(
            "domain event repository is not configured".to_string(),
        ))
    }

    async fn append_many(&self, _: Vec<NewDomainEvent>) -> AppResult<Vec<DomainEvent>> {
        Err(AppError::Repository(
            "domain event repository is not configured".to_string(),
        ))
    }

    async fn list(&self, _: &DomainEventFilter) -> AppResult<Vec<DomainEvent>> {
        Ok(vec![])
    }

    async fn count_title_history_page_events(
        &self,
        _: Option<&[TitleHistoryEventType]>,
        _: Option<&[String]>,
        _: Option<&str>,
    ) -> AppResult<i64> {
        Ok(0)
    }

    async fn count_dashboard_activity_events(
        &self,
        _: &[String],
        _: chrono::DateTime<chrono::Utc>,
        _: chrono::DateTime<chrono::Utc>,
        _: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<crate::DashboardActivityStats> {
        Ok(crate::DashboardActivityStats::default())
    }

    async fn list_title_history_page_events(
        &self,
        _: Option<&[TitleHistoryEventType]>,
        _: Option<&[String]>,
        _: Option<&str>,
        _: usize,
        _: usize,
    ) -> AppResult<Vec<DomainEvent>> {
        Ok(vec![])
    }

    async fn list_after_sequence(&self, _: i64, _: usize) -> AppResult<Vec<DomainEvent>> {
        Ok(vec![])
    }

    async fn delete_for_title_ids(&self, _: &[String]) -> AppResult<u32> {
        Ok(0)
    }

    async fn get_subscriber_offset(&self, _: &str) -> AppResult<i64> {
        Ok(0)
    }

    async fn set_subscriber_offset(&self, _: &str, _: i64) -> AppResult<()> {
        Ok(())
    }
}

#[derive(Default)]
pub struct NullHousekeepingRepository;

#[async_trait]
impl HousekeepingRepository for NullHousekeepingRepository {
    async fn delete_stale_workflow_operations(
        &self,
        _completed_days: i64,
        _warning_failed_days: i64,
    ) -> AppResult<u32> {
        Ok(0)
    }
    async fn delete_release_decisions_older_than(&self, _days: i64) -> AppResult<u32> {
        Ok(0)
    }
    async fn delete_release_attempts_older_than(&self, _days: i64) -> AppResult<u32> {
        Ok(0)
    }
    async fn delete_history_events_older_than(&self, _days: i64) -> AppResult<u32> {
        Ok(0)
    }
    async fn delete_domain_events_older_than_for_types(
        &self,
        _days: i64,
        _event_types: &[DomainEventType],
    ) -> AppResult<u32> {
        Ok(0)
    }
    async fn delete_title_history_older_than(&self, _days: i64) -> AppResult<u32> {
        Ok(0)
    }
    async fn delete_download_import_artifacts_older_than(&self, _days: i64) -> AppResult<u32> {
        Ok(0)
    }
    async fn delete_terminal_imports_older_than(&self, _days: i64) -> AppResult<u32> {
        Ok(0)
    }
    async fn delete_terminal_download_queue_commands_older_than(
        &self,
        _days: i64,
    ) -> AppResult<u32> {
        Ok(0)
    }
    async fn delete_rule_set_history_older_than(&self, _days: i64) -> AppResult<u32> {
        Ok(0)
    }
    async fn delete_history_events_for_title_ids(&self, _title_ids: &[String]) -> AppResult<u32> {
        Ok(0)
    }
    async fn delete_download_import_artifacts_for_title_ids(
        &self,
        _title_ids: &[String],
    ) -> AppResult<u32> {
        Ok(0)
    }
    async fn delete_release_attempts_for_title_ids(&self, _title_ids: &[String]) -> AppResult<u32> {
        Ok(0)
    }
    async fn list_all_media_file_paths(&self) -> AppResult<Vec<(String, String)>> {
        Ok(vec![])
    }

    async fn list_media_files_with_roots(
        &self,
    ) -> AppResult<Vec<crate::HousekeepingMediaFileRootRow>> {
        Ok(vec![])
    }

    async fn delete_media_files_by_ids(&self, _ids: &[String]) -> AppResult<u32> {
        Ok(0)
    }
    async fn prune_unreferenced_title_image_blobs(&self, _limit: u32) -> AppResult<u32> {
        Ok(0)
    }
}

#[derive(Default)]
pub struct NullDownloadSubmissionRepository;

#[derive(Default)]
pub struct NullDownloadRegistryRepository;

#[derive(Default)]
pub struct NullAcquisitionStateRepository;

#[async_trait]
impl DownloadRegistryRepository for NullDownloadRegistryRepository {
    async fn resolve_observation(
        &self,
        observation: &ObservedClientJob,
    ) -> AppResult<ObservationResolution> {
        Err(AppError::Repository(format!(
            "download registry is unavailable for observation {}:{}",
            observation.locator.client_type, observation.locator.item_id
        )))
    }

    async fn load_download(
        &self,
        _: &scryer_domain::download_identity::DownloadId,
    ) -> AppResult<Option<DownloadRecord>> {
        Ok(None)
    }

    async fn load_binding(
        &self,
        _: &scryer_domain::download_identity::DownloadId,
    ) -> AppResult<Option<DownloadClientBindingRecord>> {
        Ok(None)
    }

    async fn find_active_binding_by_locator(
        &self,
        _: &ClientJobLocator,
    ) -> AppResult<Option<DownloadClientBindingRecord>> {
        Ok(None)
    }

    async fn end_binding(&self, _: &scryer_domain::download_identity::DownloadId) -> AppResult<()> {
        Ok(())
    }
}

#[async_trait]
impl DownloadSubmissionRepository for NullDownloadSubmissionRepository {
    async fn record_submission(&self, _: DownloadSubmission) -> AppResult<()> {
        Ok(())
    }
    async fn record_ambiguous_submission(&self, _: DownloadSubmission) -> AppResult<()> {
        Ok(())
    }
    async fn record_submission_with_identity(
        &self,
        _: DownloadSubmission,
        _: crate::DownloadSubmissionIdentity,
        _: Option<crate::PersistedSeedGoals>,
    ) -> AppResult<crate::CanonicalDownloadIdentityDisposition> {
        Ok(crate::CanonicalDownloadIdentityDisposition::Requested)
    }
    async fn find_by_client_item_id(
        &self,
        _: &ClientJobLocator,
    ) -> AppResult<Option<DownloadSubmission>> {
        Ok(None)
    }
    async fn list_for_client_items(
        &self,
        _: &[ClientJobLocator],
    ) -> AppResult<Vec<DownloadSubmission>> {
        Ok(vec![])
    }
    async fn list_for_title(&self, _: &str) -> AppResult<Vec<DownloadSubmission>> {
        Ok(vec![])
    }
    async fn find_by_title_and_request_signature(
        &self,
        _: &str,
        _: &str,
        _: crate::DownloadSubmissionPurpose,
        _: &crate::SubmissionScope,
    ) -> AppResult<Option<DownloadSubmission>> {
        Ok(None)
    }
    async fn delete_for_title(&self, _: &str) -> AppResult<()> {
        Ok(())
    }
    async fn delete_by_client_item_id(&self, _: &ClientJobLocator) -> AppResult<()> {
        Ok(())
    }
    async fn update_tracked_state(&self, _: &ClientJobLocator, _: &str) -> AppResult<()> {
        Ok(())
    }
    async fn get_tracked_state(&self, _: &ClientJobLocator) -> AppResult<Option<String>> {
        Ok(None)
    }
}

#[async_trait]
impl AcquisitionStateRepository for NullAcquisitionStateRepository {
    async fn commit_successful_grab(&self, _: &SuccessfulGrabCommit) -> AppResult<()> {
        Ok(())
    }
}

pub struct NullImportArtifactRepository;

#[async_trait]
impl ImportArtifactRepository for NullImportArtifactRepository {
    async fn insert_artifact(&self, _: ImportArtifact) -> AppResult<()> {
        Ok(())
    }
    async fn list_by_source_identity(
        &self,
        _: &ClientJobLocator,
    ) -> AppResult<Vec<ImportArtifact>> {
        Ok(vec![])
    }
    async fn count_by_result_for_source_identity(
        &self,
        _: &ClientJobLocator,
        _: &str,
    ) -> AppResult<u64> {
        Ok(0)
    }
}

#[derive(Default)]
pub struct NullStagedNzbStore;

#[async_trait]
impl StagedNzbStore for NullStagedNzbStore {
    async fn create_pending_staged_nzb(
        &self,
        _source_url: &str,
        _title_id: Option<&str>,
    ) -> AppResult<PendingStagedNzb> {
        Err(AppError::Repository(
            "staged nzb store is not configured".to_string(),
        ))
    }

    async fn finalize_pending_staged_nzb(
        &self,
        _pending: PendingStagedNzb,
        _raw_size_bytes: u64,
    ) -> AppResult<StagedNzbRef> {
        Err(AppError::Repository(
            "staged nzb store is not configured".to_string(),
        ))
    }

    async fn delete_staged_nzb(&self, _: &StagedNzbRef) -> AppResult<bool> {
        Ok(false)
    }

    async fn prune_staged_nzbs_older_than(
        &self,
        _older_than: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<u32> {
        Ok(0)
    }

    fn mark_artifact_active(&self, _path: &std::path::Path) -> AppResult<()> {
        Ok(())
    }

    fn mark_artifact_inactive(&self, _path: &std::path::Path) -> AppResult<()> {
        Ok(())
    }
}

pub struct NullPendingReleaseRepository;

#[async_trait]
impl PendingReleaseRepository for NullPendingReleaseRepository {
    async fn insert_pending_release(&self, _: &PendingRelease) -> AppResult<String> {
        Ok(String::new())
    }
    async fn insert_pending_release_with_role(
        &self,
        _: &PendingRelease,
        _: PendingReleaseRole,
    ) -> AppResult<String> {
        Ok(String::new())
    }
    async fn insert_pending_release_observation(
        &self,
        _: &PendingRelease,
        _: &PendingReleaseObservation,
    ) -> AppResult<String> {
        Ok(String::new())
    }
    async fn list_expired_pending_releases(&self, _: &str) -> AppResult<Vec<PendingRelease>> {
        Ok(vec![])
    }
    async fn list_waiting_pending_releases(&self) -> AppResult<Vec<PendingRelease>> {
        Ok(vec![])
    }
    async fn list_active_release_age_unknown_pending_releases(
        &self,
    ) -> AppResult<Vec<PendingRelease>> {
        Ok(vec![])
    }
    async fn get_pending_release(&self, _: &str) -> AppResult<Option<PendingRelease>> {
        Ok(None)
    }
    async fn list_pending_releases_for_wanted_item(
        &self,
        _: &str,
    ) -> AppResult<Vec<PendingRelease>> {
        Ok(vec![])
    }
    async fn list_pending_releases_for_title(&self, _: &str) -> AppResult<Vec<PendingRelease>> {
        Ok(vec![])
    }
    async fn list_pending_releases_page(
        &self,
        _: PendingReleasesPageQuery,
    ) -> AppResult<(Vec<PendingRelease>, i64)> {
        Ok((vec![], 0))
    }
    async fn update_pending_release_status(
        &self,
        _: &str,
        _: PendingReleaseStatus,
        _: Option<&str>,
    ) -> AppResult<()> {
        Ok(())
    }
    async fn expire_pending_release(&self, _: &str, _: &str) -> AppResult<()> {
        Ok(())
    }
    async fn mark_release_age_unknown_pending_release_needs_review(
        &self,
        _: &str,
        _: &str,
    ) -> AppResult<()> {
        Ok(())
    }
    async fn update_pending_release_delay_until(&self, _: &str, _: &str) -> AppResult<()> {
        Ok(())
    }
    async fn list_standby_pending_releases_for_wanted_item(
        &self,
        _: &str,
    ) -> AppResult<Vec<PendingRelease>> {
        Ok(vec![])
    }
    async fn list_standby_pending_releases_for_title(
        &self,
        _: &str,
    ) -> AppResult<Vec<PendingRelease>> {
        Ok(vec![])
    }
    async fn count_standby_pending_releases_for_wanted_items(
        &self,
        _: &[String],
    ) -> AppResult<std::collections::HashMap<String, i64>> {
        Ok(std::collections::HashMap::new())
    }
    async fn delete_standby_pending_releases_for_wanted_item(&self, _: &str) -> AppResult<()> {
        Ok(())
    }
    async fn list_all_standby_pending_releases(&self) -> AppResult<Vec<PendingRelease>> {
        Ok(vec![])
    }
    async fn compare_and_set_pending_release_status(
        &self,
        _: &str,
        _: PendingReleaseStatus,
        _: PendingReleaseStatus,
        _: Option<&str>,
    ) -> AppResult<bool> {
        Ok(false)
    }
    async fn retire_lower_or_equal_overlapping_pending_releases(
        &self,
        _: &[String],
    ) -> AppResult<()> {
        Ok(())
    }
    async fn delete_pending_releases_for_title(&self, _: &str) -> AppResult<()> {
        Ok(())
    }
}

#[derive(Default)]
pub struct NullSettingsRepository;

#[async_trait]
impl SettingsRepository for NullSettingsRepository {
    async fn get_setting_json(
        &self,
        _: &str,
        _: &str,
        _: Option<String>,
    ) -> AppResult<Option<String>> {
        Ok(None)
    }

    async fn upsert_setting_json(
        &self,
        _: &str,
        _: &str,
        _: Option<String>,
        _: String,
        _: &str,
        _: Option<String>,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn delete_setting_value(&self, _: &str, _: &str, _: Option<String>) -> AppResult<()> {
        Ok(())
    }

    async fn delete_values_for_scope_id(&self, _: &str) -> AppResult<u32> {
        Ok(0)
    }
}

#[derive(Default)]
pub struct NullBlocklistRepository;

#[async_trait]
impl BlocklistRepository for NullBlocklistRepository {
    async fn block(&self, _: &NewBlocklistEntry) -> AppResult<bool> {
        Ok(false)
    }
    async fn list_for_title(&self, _: &str, _: usize) -> AppResult<Vec<BlocklistEntry>> {
        Ok(vec![])
    }
    async fn list_all(&self, _: usize, _: usize) -> AppResult<(Vec<BlocklistEntry>, i64)> {
        Ok((vec![], 0))
    }
    async fn get(&self, _: &str) -> AppResult<Option<BlocklistEntry>> {
        Ok(None)
    }
    async fn is_blocked(&self, _: &str, _: &str, _: &str, _: Option<&str>) -> AppResult<bool> {
        Ok(false)
    }
    async fn remove(&self, _: &str) -> AppResult<()> {
        Ok(())
    }
    async fn delete_for_title(&self, _: &str) -> AppResult<()> {
        Ok(())
    }
    async fn delete_for_indexer(&self, _: &str) -> AppResult<()> {
        Ok(())
    }
}

// ── Additional null impls for test bootstrapping ─────────────────────────────

pub struct NullSubtitleDownloadRepository;

#[async_trait]
impl crate::SubtitleDownloadRepository for NullSubtitleDownloadRepository {
    async fn list_for_title(
        &self,
        _title_id: &str,
    ) -> AppResult<Vec<scryer_domain::SubtitleDownload>> {
        Ok(Vec::new())
    }
    async fn get(&self, _id: &str) -> AppResult<Option<scryer_domain::SubtitleDownload>> {
        Ok(None)
    }
    async fn list_for_media_file(
        &self,
        _media_file_id: &str,
    ) -> AppResult<Vec<scryer_domain::SubtitleDownload>> {
        Ok(Vec::new())
    }
    async fn list_probe_cache_for_media_file(
        &self,
        _media_file_id: &str,
    ) -> AppResult<Vec<crate::subtitles::ExternalSubtitleProbeCacheEntry>> {
        Ok(Vec::new())
    }
    async fn list_blocklist_for_media_file(
        &self,
        _media_file_id: &str,
    ) -> AppResult<Vec<scryer_domain::SubtitleBlocklistEntry>> {
        Ok(Vec::new())
    }
    async fn insert(&self, _download: &scryer_domain::SubtitleDownload) -> AppResult<()> {
        Ok(())
    }
    async fn upsert_probe_cache_entry(
        &self,
        _entry: &crate::subtitles::ExternalSubtitleProbeCacheEntry,
    ) -> AppResult<()> {
        Ok(())
    }
    async fn set_synced(&self, _id: &str, _synced: bool) -> AppResult<()> {
        Ok(())
    }
    async fn delete(&self, _id: &str) -> AppResult<Option<scryer_domain::SubtitleDownload>> {
        Ok(None)
    }
    async fn delete_probe_cache_entry(&self, _: &str, _: &str) -> AppResult<()> {
        Ok(())
    }
    async fn is_blocklisted(
        &self,
        _media_file_id: &str,
        _provider: &str,
        _provider_file_id: &str,
    ) -> AppResult<bool> {
        Ok(false)
    }
    async fn blocklist(
        &self,
        _media_file_id: &str,
        _provider: &str,
        _provider_file_id: &str,
        _language: &str,
        _reason: Option<&str>,
    ) -> AppResult<()> {
        Ok(())
    }
}

#[derive(Default)]
pub struct NullJobRunRepository;

#[async_trait]
impl JobRunRepository for NullJobRunRepository {
    async fn create_job_run(&self, run: &JobRunRecord) -> AppResult<JobRunRecord> {
        Ok(run.clone())
    }

    async fn update_job_run(&self, run: &JobRunRecord) -> AppResult<JobRunRecord> {
        Ok(run.clone())
    }

    async fn get_job_run(&self, _run_id: &str) -> AppResult<Option<JobRunRecord>> {
        Ok(None)
    }

    async fn list_job_runs(
        &self,
        _job_key: Option<JobKey>,
        _limit: usize,
    ) -> AppResult<Vec<JobRunRecord>> {
        Ok(Vec::new())
    }

    async fn list_job_runs_for_actor(
        &self,
        _job_key: Option<JobKey>,
        _actor_user_id: &str,
        _limit: usize,
    ) -> AppResult<Vec<JobRunRecord>> {
        Ok(Vec::new())
    }

    async fn list_active_job_runs(&self) -> AppResult<Vec<JobRunRecord>> {
        Ok(Vec::new())
    }

    async fn reconcile_interrupted_job_runs(&self, _excluded_run_ids: &[String]) -> AppResult<u64> {
        Ok(0)
    }
}

#[derive(Default)]
pub struct NullLibraryProbeRepository;

#[async_trait]
impl LibraryProbeRepository for NullLibraryProbeRepository {
    async fn get_probe_signature(
        &self,
        _title_id: &str,
    ) -> AppResult<Option<LibraryProbeSignature>> {
        Ok(None)
    }

    async fn upsert_probe_signature(&self, _probe: &LibraryProbeSignature) -> AppResult<()> {
        Ok(())
    }

    async fn delete_probe_signatures_for_title_ids(&self, _: &[String]) -> AppResult<u32> {
        Ok(0)
    }
}

#[derive(Default)]
pub struct NullLibraryScanUnmatchedItemRepository;

#[async_trait]
impl LibraryScanUnmatchedItemRepository for NullLibraryScanUnmatchedItemRepository {
    async fn upsert_library_scan_unmatched_item(
        &self,
        _item: &LibraryScanUnmatchedItem,
    ) -> AppResult<String> {
        Err(AppError::Repository(
            "library scan unmatched item repository is not configured".to_string(),
        ))
    }

    async fn get_library_scan_unmatched_item(
        &self,
        _id: &str,
    ) -> AppResult<Option<LibraryScanUnmatchedItem>> {
        Ok(None)
    }

    async fn delete_library_scan_unmatched_item(
        &self,
        _library_id: &str,
        _facet: scryer_domain::MediaFacet,
        _item_path: &str,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn delete_for_library(&self, _library_id: &str) -> AppResult<u32> {
        Ok(0)
    }

    async fn list_library_scan_unmatched_items(
        &self,
        _facet: Option<scryer_domain::MediaFacet>,
        _scan_root: Option<&str>,
        _status: Option<PendingImportStatus>,
        _limit: i64,
        _offset: i64,
    ) -> AppResult<Vec<LibraryScanUnmatchedItem>> {
        Ok(Vec::new())
    }

    async fn count_library_scan_unmatched_items(
        &self,
        _facet: Option<scryer_domain::MediaFacet>,
        _scan_root: Option<&str>,
        _status: Option<PendingImportStatus>,
    ) -> AppResult<i64> {
        Ok(0)
    }
}

#[derive(Default)]
pub struct NullLibraryRepository;

fn null_default_library(facet: MediaFacet) -> Library {
    let now = Utc::now();
    Library {
        id: scryer_domain::default_library_id_for_facet(&facet),
        facet: facet.clone(),
        name: format!("Default {}", facet.as_str()),
        slug: scryer_domain::default_library_slug_for_facet(&facet).to_string(),
        is_default: true,
        roots: Vec::new(),
        created_at: now,
        updated_at: now,
    }
}

#[async_trait]
impl LibraryRepository for NullLibraryRepository {
    async fn list(&self, facet: Option<MediaFacet>) -> AppResult<Vec<Library>> {
        Ok(match facet {
            Some(facet) => vec![null_default_library(facet)],
            None => [MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime]
                .into_iter()
                .map(null_default_library)
                .collect(),
        })
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<Library>> {
        Ok([MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime]
            .into_iter()
            .map(null_default_library)
            .find(|library| library.id == id))
    }

    async fn default_for_facet(&self, facet: MediaFacet) -> AppResult<Option<Library>> {
        Ok(Some(null_default_library(facet)))
    }

    async fn create(&self, library: Library, _roots: Vec<LibraryRootDraft>) -> AppResult<Library> {
        Ok(library)
    }

    async fn update(
        &self,
        _library_id: &str,
        _name: String,
        _slug: String,
        _roots: Vec<LibraryRootDraft>,
    ) -> AppResult<Library> {
        Err(AppError::Repository(
            "library repository not configured".into(),
        ))
    }

    async fn set_root_path(&self, _root_id: &str, _path: &str) -> AppResult<Library> {
        Err(AppError::Repository(
            "library repository not configured".into(),
        ))
    }

    async fn delete_library(&self, _library_id: &str) -> AppResult<bool> {
        Ok(false)
    }

    async fn app_permission_mask_for_user(&self, _user_id: &str) -> AppResult<AppPermissionMask> {
        Ok(AppPermissionMask::NONE)
    }

    async fn set_app_permission_mask_for_user(
        &self,
        _user_id: &str,
        _permissions: AppPermissionMask,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn permission_masks_for_user(&self, _user_id: &str) -> AppResult<Vec<LibraryGrant>> {
        Ok(vec![])
    }

    async fn set_grants_for_user(
        &self,
        _user_id: &str,
        _grants: Vec<LibraryGrant>,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn title_library_id(&self, _title_id: &str) -> AppResult<Option<String>> {
        Ok(None)
    }
}

#[derive(Default)]
pub struct NullMediaRequestRepository;

#[async_trait]
impl MediaRequestRepository for NullMediaRequestRepository {
    async fn submit(
        &self,
        _request: NewMediaRequest,
        _requester: &User,
        _submitted_event: NewDomainEvent,
    ) -> AppResult<MediaRequestSubmissionResult> {
        Err(AppError::Repository(
            "media request repository not configured".into(),
        ))
    }

    async fn list(&self, _query: MediaRequestQuery) -> AppResult<Vec<MediaRequest>> {
        Ok(Vec::new())
    }

    async fn get(&self, _request_id: &str) -> AppResult<Option<MediaRequest>> {
        Ok(None)
    }

    async fn resolve_pending_overlapping(
        &self,
        _request: &MediaRequest,
        _resolution: MediaRequestResolution,
    ) -> AppResult<MediaRequestResolutionResult> {
        Ok(MediaRequestResolutionResult {
            updated: 0,
            event: None,
        })
    }

    async fn resolve_pending(
        &self,
        _request_id: &str,
        _resolution: MediaRequestResolution,
    ) -> AppResult<MediaRequestResolutionResult> {
        Ok(MediaRequestResolutionResult {
            updated: 0,
            event: None,
        })
    }

    async fn update_pending_request_preferences(
        &self,
        _request_id: &str,
        _requested_quality_profile_id: String,
        _requested_quality_profile_name: String,
        _requested_monitor_type: Option<String>,
        _requested_monitor_selection: Option<scryer_domain::MonitorSelection>,
        _requested_lease_days: Option<i64>,
        _updated_event: NewDomainEvent,
    ) -> AppResult<MediaRequestUpdateResult> {
        Err(AppError::Repository(
            "media request repository not configured".into(),
        ))
    }

    async fn count_pending_by_facet(
        &self,
        _library_ids: &[String],
    ) -> AppResult<MediaRequestCounts> {
        Ok(MediaRequestCounts::default())
    }

    async fn requester_user_ids_by_title_ids(
        &self,
        _title_ids: &[String],
    ) -> AppResult<std::collections::HashMap<String, Vec<String>>> {
        Ok(std::collections::HashMap::new())
    }

    async fn count_for_requester(
        &self,
        _user_id: &str,
        _status: Option<scryer_domain::MediaRequestStatus>,
        _since: Option<DateTime<Utc>>,
    ) -> AppResult<u64> {
        Ok(0)
    }

    async fn history_for_fingerprint(
        &self,
        _identity_fingerprint: &str,
    ) -> AppResult<Vec<MediaRequest>> {
        Ok(Vec::new())
    }

    async fn latest_request_at_for_user(&self, _user_id: &str) -> AppResult<Option<DateTime<Utc>>> {
        Ok(None)
    }

    /// A no-op rather than a refusal: the caller has already submitted the
    /// request and is only stamping provenance onto it, and a null repository
    /// has no row to stamp. Failing here would turn "no store configured" into
    /// a warning on every submission.
    async fn record_decision_on_request(
        &self,
        _request_id: &str,
        _decision_id: Option<&str>,
        _rule_set_ids: &[String],
        _tags: &[String],
    ) -> AppResult<()> {
        Ok(())
    }
}

/// Reads answer empty and writes refuse: an assembly with no request-rule store
/// has no rules, so the evaluator finds nothing to apply rather than silently
/// dropping a rule it believed it wrote.
#[derive(Default)]
pub struct NullRequestRuleSetRepository;

const REQUEST_RULE_NOT_CONFIGURED: &str = "request rule repository is not configured";

#[async_trait]
impl crate::ports::RequestRuleSetRepository for NullRequestRuleSetRepository {
    async fn list_rule_sets(&self) -> AppResult<Vec<scryer_domain::RequestRuleSet>> {
        Ok(Vec::new())
    }
    async fn get_rule_set(&self, _id: &str) -> AppResult<Option<scryer_domain::RequestRuleSet>> {
        Ok(None)
    }
    async fn create_rule_set(
        &self,
        _rule_set: &scryer_domain::RequestRuleSet,
        _revision: &scryer_domain::RequestRuleRevision,
    ) -> AppResult<()> {
        Err(AppError::Repository(
            REQUEST_RULE_NOT_CONFIGURED.to_string(),
        ))
    }
    async fn add_revision(
        &self,
        _revision: &scryer_domain::RequestRuleRevision,
        _updated_at: DateTime<Utc>,
    ) -> AppResult<()> {
        Err(AppError::Repository(
            REQUEST_RULE_NOT_CONFIGURED.to_string(),
        ))
    }
    async fn get_revision(
        &self,
        _rule_set_id: &str,
        _revision_number: i64,
    ) -> AppResult<Option<scryer_domain::RequestRuleRevision>> {
        Ok(None)
    }
    async fn list_revisions(
        &self,
        _rule_set_id: &str,
    ) -> AppResult<Vec<scryer_domain::RequestRuleRevision>> {
        Ok(Vec::new())
    }
    async fn update_rule_set_metadata(
        &self,
        _id: &str,
        _name: &str,
        _description: &str,
        _library_ids: &[String],
        _updated_at: DateTime<Utc>,
    ) -> AppResult<()> {
        Err(AppError::Repository(
            REQUEST_RULE_NOT_CONFIGURED.to_string(),
        ))
    }
    async fn update_rule_set_evaluation_mode(
        &self,
        _id: &str,
        _mode: scryer_domain::RequestRuleEvaluationMode,
        _enabled: bool,
        _updated_at: DateTime<Utc>,
    ) -> AppResult<()> {
        Err(AppError::Repository(
            REQUEST_RULE_NOT_CONFIGURED.to_string(),
        ))
    }
    async fn delete_rule_set(&self, _id: &str) -> AppResult<()> {
        Err(AppError::Repository(
            REQUEST_RULE_NOT_CONFIGURED.to_string(),
        ))
    }
}

/// A trace that cannot be written must not be silently discarded: recording is
/// the one thing FR-016 requires of every evaluation, so the write refuses
/// rather than pretending.
#[derive(Default)]
pub struct NullRequestRuleDecisionRepository;

#[async_trait]
impl crate::ports::RequestRuleDecisionRepository for NullRequestRuleDecisionRepository {
    async fn record(&self, _decision: &scryer_domain::RequestRuleDecisionRecord) -> AppResult<()> {
        Err(AppError::Repository(
            "request rule decision repository is not configured".to_string(),
        ))
    }
    async fn latest_for_request(
        &self,
        _request_id: &str,
    ) -> AppResult<Option<scryer_domain::RequestRuleDecisionRecord>> {
        Ok(None)
    }
    async fn list_recent(
        &self,
        _limit: usize,
        _outcome: Option<scryer_domain::RequestDecisionOutcome>,
    ) -> AppResult<Vec<scryer_domain::RequestRuleDecisionRecord>> {
        Ok(Vec::new())
    }
    async fn count_for_rule_set(&self, _rule_set_id: &str) -> AppResult<u64> {
        Ok(0)
    }
}

/// No claim store means no lease can be created, so writes refuse. Reads answer
/// empty, which is the honest shape: an instance without the table has no
/// holds — the executor's own unreadable-store hold covers the case where the
/// store exists but cannot answer.
#[derive(Default)]
pub struct NullLifecycleClaimRepository;

const LIFECYCLE_CLAIM_NOT_CONFIGURED: &str = "lifecycle claim repository is not configured";

#[async_trait]
impl crate::ports::LifecycleClaimRepository for NullLifecycleClaimRepository {
    async fn create(&self, _claim: &scryer_domain::LifecycleClaim) -> AppResult<()> {
        Err(AppError::Repository(
            LIFECYCLE_CLAIM_NOT_CONFIGURED.to_string(),
        ))
    }
    async fn get(&self, _id: &str) -> AppResult<Option<scryer_domain::LifecycleClaim>> {
        Ok(None)
    }
    async fn list_for_title(
        &self,
        _title_id: &str,
    ) -> AppResult<Vec<scryer_domain::LifecycleClaim>> {
        Ok(Vec::new())
    }
    async fn list_live_for_titles(
        &self,
        _title_ids: &[String],
    ) -> AppResult<std::collections::HashMap<String, Vec<scryer_domain::LifecycleClaim>>> {
        Ok(std::collections::HashMap::new())
    }
    async fn list_retention_history_for_titles(
        &self,
        _title_ids: &[String],
    ) -> AppResult<std::collections::HashMap<String, Vec<scryer_domain::LifecycleClaim>>> {
        Ok(std::collections::HashMap::new())
    }
    async fn list_dormant(&self, _limit: usize) -> AppResult<Vec<scryer_domain::LifecycleClaim>> {
        Ok(Vec::new())
    }
    async fn activate(
        &self,
        _id: &str,
        _starts_at: DateTime<Utc>,
        _expires_at: Option<DateTime<Utc>>,
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        Err(AppError::Repository(
            LIFECYCLE_CLAIM_NOT_CONFIGURED.to_string(),
        ))
    }
    async fn expire_due(&self, _now: DateTime<Utc>) -> AppResult<u64> {
        Ok(0)
    }
    async fn release_for_producer_ref(
        &self,
        _producer: scryer_domain::LifecycleClaimProducer,
        _producer_ref: &str,
        _reason: &str,
        _now: DateTime<Utc>,
    ) -> AppResult<u64> {
        Ok(0)
    }
    async fn release_claim(&self, _id: &str, _reason: &str, _now: DateTime<Utc>) -> AppResult<u64> {
        Ok(0)
    }
    async fn release_for_title(
        &self,
        _title_id: &str,
        _reason: &str,
        _now: DateTime<Utc>,
    ) -> AppResult<u64> {
        Ok(0)
    }
    async fn extend(
        &self,
        _id: &str,
        _expires_at: DateTime<Utc>,
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        Err(AppError::Repository(
            LIFECYCLE_CLAIM_NOT_CONFIGURED.to_string(),
        ))
    }
    async fn convert_to_permanent(
        &self,
        _id: &str,
        _replacement: &scryer_domain::LifecycleClaim,
        _now: DateTime<Utc>,
    ) -> AppResult<()> {
        Err(AppError::Repository(
            LIFECYCLE_CLAIM_NOT_CONFIGURED.to_string(),
        ))
    }
    async fn count_live_for_user(&self, _user_id: &str) -> AppResult<u64> {
        Ok(0)
    }
}

#[derive(Default)]
pub struct NullWebauthnRepository;

#[async_trait]
impl WebauthnRepository for NullWebauthnRepository {
    async fn list_credentials_for_user(&self, _: &str) -> AppResult<Vec<WebauthnCredentialRecord>> {
        Ok(vec![])
    }

    async fn get_credential_by_id_for_user(
        &self,
        _: &str,
        _: &str,
    ) -> AppResult<Option<WebauthnCredentialRecord>> {
        Ok(None)
    }

    async fn get_credential_by_credential_id(
        &self,
        _: &str,
    ) -> AppResult<Option<WebauthnCredentialRecord>> {
        Ok(None)
    }

    async fn create_credential(
        &self,
        _: WebauthnCredentialRecord,
    ) -> AppResult<WebauthnCredentialRecord> {
        Err(AppError::Repository("not configured".into()))
    }

    async fn update_credential(
        &self,
        _: WebauthnCredentialRecord,
    ) -> AppResult<WebauthnCredentialRecord> {
        Err(AppError::Repository("not configured".into()))
    }

    async fn update_credential_if_current(
        &self,
        _: WebauthnCredentialRecord,
        _: &str,
    ) -> AppResult<Option<WebauthnCredentialRecord>> {
        Err(AppError::Repository("not configured".into()))
    }

    async fn delete_credential_for_user(&self, _: &str, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn create_challenge(
        &self,
        _: WebauthnChallengeRecord,
    ) -> AppResult<WebauthnChallengeRecord> {
        Err(AppError::Repository("not configured".into()))
    }

    async fn get_challenge(&self, _: &str) -> AppResult<Option<WebauthnChallengeRecord>> {
        Ok(None)
    }

    async fn take_challenge(&self, _: &str) -> AppResult<Option<WebauthnChallengeRecord>> {
        Ok(None)
    }

    async fn delete_challenge(&self, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn delete_expired_challenges(&self, _: &str) -> AppResult<u64> {
        Ok(0)
    }
}

#[derive(Default)]
pub struct NullTotpRepository;

#[async_trait]
impl TotpRepository for NullTotpRepository {
    async fn get_credential_for_user(&self, _: &str) -> AppResult<Option<TotpCredentialRecord>> {
        Ok(None)
    }

    async fn upsert_credential(&self, _: TotpCredentialRecord) -> AppResult<TotpCredentialRecord> {
        Err(AppError::Repository("not configured".into()))
    }

    async fn delete_credential_for_user(&self, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn create_enrollment_challenge(
        &self,
        _: TotpEnrollmentChallengeRecord,
    ) -> AppResult<TotpEnrollmentChallengeRecord> {
        Err(AppError::Repository("not configured".into()))
    }

    async fn get_enrollment_challenge(
        &self,
        _: &str,
        _: &str,
    ) -> AppResult<Option<TotpEnrollmentChallengeRecord>> {
        Ok(None)
    }

    async fn delete_enrollment_challenge(&self, _: &str, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn delete_enrollment_challenges_for_user(&self, _: &str) -> AppResult<u64> {
        Ok(0)
    }

    async fn delete_expired_enrollment_challenges(&self, _: &str) -> AppResult<u64> {
        Ok(0)
    }

    async fn reset_user_mfa_and_invalidate_sessions(&self, _: &str, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn replace_recovery_codes(
        &self,
        _: &str,
        codes: Vec<TotpRecoveryCodeRecord>,
    ) -> AppResult<()> {
        if codes.is_empty() {
            Ok(())
        } else {
            Err(AppError::Repository("not configured".into()))
        }
    }

    async fn list_recovery_codes_for_user(
        &self,
        _: &str,
    ) -> AppResult<Vec<TotpRecoveryCodeRecord>> {
        Ok(Vec::new())
    }

    async fn mark_recovery_code_used(&self, _: &str, _: &str, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn record_failed_attempt(&self, _: TotpFailedAttemptRecord) -> AppResult<()> {
        Ok(())
    }

    async fn count_failed_attempts_since(&self, _: &str, _: &str) -> AppResult<i64> {
        Ok(0)
    }

    async fn clear_failed_attempts(&self, _: &str) -> AppResult<u64> {
        Ok(0)
    }
}

#[derive(Default)]
pub struct NullOAuthRepository;

#[async_trait]
impl OAuthRepository for NullOAuthRepository {
    async fn create_api_key(&self, _: ApiKeyRecord) -> AppResult<ApiKeyRecord> {
        Err(AppError::Repository("not configured".into()))
    }

    async fn get_api_key_by_lookup_id(&self, _: &str) -> AppResult<Option<ApiKeyRecord>> {
        Ok(None)
    }

    async fn list_api_keys(&self, _: &str) -> AppResult<Vec<ApiKeyRecord>> {
        Ok(Vec::new())
    }

    async fn list_environment_api_keys(&self) -> AppResult<Vec<ApiKeyRecord>> {
        Ok(Vec::new())
    }

    async fn upsert_environment_api_key(&self, _: ApiKeyRecord) -> AppResult<ApiKeyRecord> {
        Err(AppError::Repository("not configured".into()))
    }

    async fn revoke_api_key(
        &self,
        _: &str,
        _: &str,
        _: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<bool> {
        Ok(false)
    }

    async fn touch_api_key_last_used(
        &self,
        _: &str,
        _: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<bool> {
        Ok(false)
    }

    async fn create_client_registration(
        &self,
        _: OAuthClientRegistrationRecord,
    ) -> AppResult<OAuthClientRegistrationRecord> {
        Err(AppError::Repository("not configured".into()))
    }

    async fn get_client_registration(
        &self,
        _: &str,
    ) -> AppResult<Option<OAuthClientRegistrationRecord>> {
        Ok(None)
    }

    async fn list_client_registrations(&self) -> AppResult<Vec<OAuthClientRegistrationRecord>> {
        Ok(Vec::new())
    }

    async fn update_client_registration(
        &self,
        _: OAuthClientRegistrationRecord,
        _: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<Option<OAuthClientRegistrationRecord>> {
        Ok(None)
    }

    async fn delete_client_registration(
        &self,
        _: &str,
        _: chrono::DateTime<chrono::Utc>,
        _: &str,
    ) -> AppResult<bool> {
        Ok(false)
    }

    async fn is_refresh_grant_active(&self, _: &str, _: &str) -> AppResult<bool> {
        Ok(false)
    }

    async fn create_authorization_code(
        &self,
        _: OAuthAuthorizationCodeRecord,
    ) -> AppResult<OAuthAuthorizationCodeRecord> {
        Err(AppError::Repository("not configured".into()))
    }

    async fn get_authorization_code(
        &self,
        _: &str,
    ) -> AppResult<Option<OAuthAuthorizationCodeRecord>> {
        Ok(None)
    }

    async fn consume_authorization_code(
        &self,
        _: &str,
        _: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<bool> {
        Ok(false)
    }

    async fn consume_authorization_code_and_create_refresh_grant(
        &self,
        _: OAuthAuthorizationCodeRecord,
        _: chrono::DateTime<chrono::Utc>,
        _: OAuthRefreshGrantRecord,
        _: OAuthRefreshTokenRecord,
        _: bool,
    ) -> AppResult<Option<OAuthRefreshGrantRecord>> {
        Ok(None)
    }

    async fn create_refresh_grant(
        &self,
        _: OAuthRefreshGrantRecord,
        _: OAuthRefreshTokenRecord,
        _: bool,
    ) -> AppResult<OAuthRefreshGrantRecord> {
        Err(AppError::Repository("not configured".into()))
    }

    async fn get_refresh_token(
        &self,
        _: &str,
    ) -> AppResult<Option<(OAuthRefreshTokenRecord, OAuthRefreshGrantRecord)>> {
        Ok(None)
    }

    async fn get_refresh_grant(&self, _: &str) -> AppResult<Option<OAuthRefreshGrantRecord>> {
        Ok(None)
    }

    async fn rotate_refresh_token(
        &self,
        _: &str,
        _: chrono::DateTime<chrono::Utc>,
        _: OAuthRefreshTokenRecord,
    ) -> AppResult<OAuthRefreshRotationOutcome> {
        Ok(OAuthRefreshRotationOutcome::Unavailable)
    }

    async fn revoke_refresh_grant(
        &self,
        _: &str,
        _: &str,
        _: chrono::DateTime<chrono::Utc>,
        _: &str,
    ) -> AppResult<bool> {
        Ok(false)
    }

    async fn revoke_refresh_family(
        &self,
        _: &str,
        _: chrono::DateTime<chrono::Utc>,
        _: &str,
    ) -> AppResult<u64> {
        Ok(0)
    }

    async fn revoke_user_refresh_grants(
        &self,
        _: &str,
        _: chrono::DateTime<chrono::Utc>,
        _: &str,
    ) -> AppResult<u64> {
        Ok(0)
    }

    async fn revoke_authless_refresh_grants(
        &self,
        _: chrono::DateTime<chrono::Utc>,
        _: &str,
    ) -> AppResult<u64> {
        Ok(0)
    }

    async fn touch_refresh_grant_last_used(
        &self,
        _: &str,
        _: &str,
        _: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<bool> {
        Ok(false)
    }

    async fn list_connected_apps(&self, _: &str) -> AppResult<Vec<OAuthConnectedAppRecord>> {
        Ok(Vec::new())
    }
}

#[derive(Default)]
pub struct NullUserExternalAccountRepository;

#[async_trait]
impl UserExternalAccountRepository for NullUserExternalAccountRepository {
    async fn create(
        &self,
        _: scryer_domain::UserExternalAccount,
    ) -> AppResult<scryer_domain::UserExternalAccount> {
        Err(AppError::Repository("not configured".into()))
    }

    async fn create_or_get_by_provider_identity(
        &self,
        _: scryer_domain::UserExternalAccount,
    ) -> AppResult<scryer_domain::UserExternalAccount> {
        Err(AppError::Repository("not configured".into()))
    }

    async fn list_by_user_id(&self, _: &str) -> AppResult<Vec<scryer_domain::UserExternalAccount>> {
        Ok(Vec::new())
    }

    async fn get_by_id(&self, _: &str) -> AppResult<Option<scryer_domain::UserExternalAccount>> {
        Ok(None)
    }

    async fn get_by_provider_identity(
        &self,
        _: scryer_domain::ExternalAccountProvider,
        _: &str,
        _: &str,
    ) -> AppResult<Option<scryer_domain::UserExternalAccount>> {
        Ok(None)
    }

    async fn get_pending_claim_by_provider_username(
        &self,
        _: scryer_domain::ExternalAccountProvider,
        _: &str,
        _: &str,
    ) -> AppResult<Option<scryer_domain::UserExternalAccount>> {
        Ok(None)
    }

    async fn list_verified_by_connection(
        &self,
        _: scryer_domain::ExternalAccountProvider,
        _: &str,
    ) -> AppResult<Vec<scryer_domain::UserExternalAccount>> {
        Ok(Vec::new())
    }

    async fn update(
        &self,
        _: scryer_domain::UserExternalAccount,
    ) -> AppResult<scryer_domain::UserExternalAccount> {
        Err(AppError::Repository("not configured".into()))
    }

    async fn create_auto_added_user_with_account(
        &self,
        _: scryer_domain::User,
        _: scryer_domain::AppPermissionMask,
        _: Vec<scryer_domain::LibraryGrant>,
        _: scryer_domain::UserExternalAccount,
    ) -> AppResult<(scryer_domain::User, scryer_domain::UserExternalAccount)> {
        Err(AppError::Repository("not configured".into()))
    }

    async fn delete(&self, _: &str) -> AppResult<()> {
        Ok(())
    }
}

#[derive(Default)]
pub struct NullUserUiSettingsRepository;

#[async_trait]
impl UserUiSettingsRepository for NullUserUiSettingsRepository {
    async fn get_by_user_id(&self, user_id: &str) -> AppResult<Option<UiSettings>> {
        Ok(Some(UiSettings::defaults_for_user(user_id.to_string())))
    }

    async fn upsert(&self, user_id: &str, settings: UiSettingsUpdate) -> AppResult<UiSettings> {
        let mut current = UiSettings::defaults_for_user(user_id.to_string());
        current.theme = settings.theme;
        current.date_time_format = settings.date_time_format;
        current.highlight_color = settings.highlight_color;
        current.secondary_color = settings.secondary_color;
        current.high_contrast_mode = settings.high_contrast_mode;
        current.reduce_motion = settings.reduce_motion;
        current.hide_sponsor_button = settings.hide_sponsor_button;
        current.density = settings.density;
        current.sidebar_mode = settings.sidebar_mode;
        current.default_landing_view = settings.default_landing_view;
        current.table_columns = settings.table_columns;
        Ok(current)
    }
}

#[derive(Default)]
pub struct NullMediaServerConnectionRepository;

#[async_trait]
impl MediaServerConnectionRepository for NullMediaServerConnectionRepository {
    async fn list(
        &self,
        _: Option<scryer_domain::MediaServerProvider>,
    ) -> AppResult<Vec<scryer_domain::MediaServerConnection>> {
        Ok(Vec::new())
    }

    async fn get_by_id(&self, _: &str) -> AppResult<Option<scryer_domain::MediaServerConnection>> {
        Ok(None)
    }

    async fn create(
        &self,
        _: scryer_domain::MediaServerConnection,
    ) -> AppResult<scryer_domain::MediaServerConnection> {
        Err(AppError::Repository("not configured".into()))
    }

    async fn update(
        &self,
        _: scryer_domain::MediaServerConnection,
    ) -> AppResult<scryer_domain::MediaServerConnection> {
        Err(AppError::Repository("not configured".into()))
    }

    async fn list_playback_items_for_entity(
        &self,
        _: scryer_domain::MediaServerPlaybackEntityKind,
        _: &str,
    ) -> AppResult<Vec<scryer_domain::MediaServerPlaybackItem>> {
        Ok(Vec::new())
    }

    async fn replace_playback_items_for_connection(
        &self,
        _: &str,
        _: Vec<scryer_domain::MediaServerPlaybackItem>,
    ) -> AppResult<()> {
        Err(AppError::Repository("not configured".into()))
    }

    async fn delete(&self, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn has_external_accounts(&self, _: &str) -> AppResult<bool> {
        Ok(false)
    }

    async fn has_notification_channels(&self, _: &str) -> AppResult<bool> {
        Ok(false)
    }
}

#[derive(Default)]
pub struct NullExternalIdentityVerifier;

#[async_trait]
impl ExternalIdentityVerifier for NullExternalIdentityVerifier {
    async fn verify_plex(
        &self,
        _: &str,
        _: Option<&str>,
        _: &str,
    ) -> AppResult<VerifiedExternalIdentity> {
        Err(AppError::Repository(
            "external identity verification is not configured".into(),
        ))
    }

    async fn discover_plex_servers(&self, _: &str) -> AppResult<Vec<PlexServerDiscovery>> {
        Err(AppError::Repository(
            "external identity verification is not configured".into(),
        ))
    }

    async fn verify_jellyfin(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: &str,
    ) -> AppResult<VerifiedExternalIdentity> {
        Err(AppError::Repository(
            "external identity verification is not configured".into(),
        ))
    }

    async fn test_jellyfin_connection(&self, _: &str) -> AppResult<()> {
        Err(AppError::Repository(
            "external identity verification is not configured".into(),
        ))
    }

    async fn test_jellyfin_api_key(&self, _: &str, _: &str) -> AppResult<()> {
        Err(AppError::Repository(
            "external identity verification is not configured".into(),
        ))
    }

    async fn exchange_jellyfin_admin_api_key(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: &str,
    ) -> AppResult<String> {
        Err(AppError::Repository(
            "external identity verification is not configured".into(),
        ))
    }

    async fn list_jellyfin_users(
        &self,
        _: &str,
        _: &str,
        _: Option<&str>,
    ) -> AppResult<Vec<JellyfinServerUser>> {
        Err(AppError::Repository(
            "external identity verification is not configured".into(),
        ))
    }

    async fn list_plex_users(&self, _: &str, _: Option<&str>) -> AppResult<Vec<PlexServerUser>> {
        Err(AppError::Repository(
            "external identity verification is not configured".into(),
        ))
    }
}

// ── Maintenance safety probes (RFC 137 §9.10, WP-G) ─────────────────────────

/// Playback probe for an assembly with no media-server integration.
///
/// Returns an empty snapshot, which the fold reads as `Clear`: nothing can be
/// playing on servers Scryer does not know about. This is deliberately *not*
/// `Unreachable` — "no connection configured" is a known answer, while "a
/// configured connection did not respond" is not.
#[derive(Default)]
pub struct NullMediaServerPlaybackProbe;

#[async_trait]
impl crate::ports::MediaServerPlaybackProbe for NullMediaServerPlaybackProbe {
    async fn active_playback(&self) -> AppResult<crate::ports::PlaybackActivitySnapshot> {
        Ok(crate::ports::PlaybackActivitySnapshot::empty(Utc::now()))
    }
}

/// No signal store configured: reads are empty and writes are refused.
///
/// Writes fail loudly rather than silently succeeding, because a sync sweep
/// that "worked" against a store that kept nothing would leave the sync state
/// claiming a success that produced no observations.
#[derive(Default)]
pub struct NullMediaServerSignalRepository;

#[async_trait]
impl crate::ports::MediaServerSignalRepository for NullMediaServerSignalRepository {
    async fn replace_participant_signals(
        &self,
        _: &str,
        _: &str,
        _: &[scryer_domain::NewUserMediaSignal],
    ) -> AppResult<u64> {
        Err(AppError::Repository("not configured".into()))
    }

    async fn movie_signals_for_titles(
        &self,
        _: &[String],
    ) -> AppResult<std::collections::HashMap<String, Vec<scryer_domain::UserMediaSignal>>> {
        Ok(std::collections::HashMap::new())
    }

    async fn episode_signals_for_titles(
        &self,
        _: &[String],
    ) -> AppResult<std::collections::HashMap<String, Vec<scryer_domain::UserMediaSignal>>> {
        Ok(std::collections::HashMap::new())
    }

    async fn signal_sync_states(
        &self,
    ) -> AppResult<Vec<scryer_domain::MediaServerSignalSyncState>> {
        Ok(Vec::new())
    }

    async fn upsert_signal_sync_state(
        &self,
        _: &scryer_domain::MediaServerSignalSyncState,
    ) -> AppResult<()> {
        Err(AppError::Repository("not configured".into()))
    }
}

/// No signal adapter configured. Every fetch is an error rather than an empty
/// list: "this participant has watched nothing" and "nobody asked the server"
/// are different facts, and the second one must not be recorded as the first.
#[derive(Default)]
pub struct NullMediaServerSignalSource;

#[async_trait]
impl crate::ports::MediaServerSignalSource for NullMediaServerSignalSource {
    async fn fetch_played_items(
        &self,
        _: &scryer_domain::MediaServerConnection,
        _: &str,
    ) -> AppResult<Vec<crate::ports::ProviderPlayedItem>> {
        Err(AppError::Repository(
            "media-server signal source is not configured".into(),
        ))
    }
}

#[cfg(test)]
pub mod test_nulls {
    use crate::{
        AppError, AppResult, BuiltinDownloadClientConnectionTester, CollectionUpdate,
        CreateTitleOutcome, DownloadClient, DownloadClientAddRequest,
        DownloadClientConfigRepository, DownloadClientConfigUpdate, DownloadGrabResult,
        EpisodeUpdate, IndexerClient, IndexerRoutingPlan, IndexerSearchResponse,
        PendingTitleHydration, PrimaryCollectionSummary, QualityProfile, QualityProfileRepository,
        ReleaseAttemptRepository, ReleaseDownloadAttemptOutcome, ReleaseDownloadFailureSignature,
        ScopedExternalId, SearchMode, ShowRepository, TitleMetadataUpdate, TitleRepository,
        UserRepository,
    };
    use async_trait::async_trait;
    use scryer_domain::{
        CalendarEpisode, Collection, DownloadClientConfig, Episode, MediaFacet, Title, User,
    };

    #[derive(Default)]
    pub struct NullTitleRepository;

    #[async_trait]
    impl TitleRepository for NullTitleRepository {
        async fn list(&self, _: Option<MediaFacet>, _: Option<String>) -> AppResult<Vec<Title>> {
            Ok(vec![])
        }
        async fn list_for_libraries(
            &self,
            _: Option<MediaFacet>,
            _: &[String],
            _: Option<String>,
        ) -> AppResult<Vec<Title>> {
            Ok(vec![])
        }
        async fn list_by_external_ids(&self, _: &str, _: &[String]) -> AppResult<Vec<Title>> {
            Ok(vec![])
        }
        async fn list_for_matching(
            &self,
            _: Option<MediaFacet>,
            _: Option<String>,
        ) -> AppResult<Vec<Title>> {
            Ok(vec![])
        }
        async fn get_by_id(&self, _: &str) -> AppResult<Option<Title>> {
            Ok(None)
        }
        async fn get_by_facet_and_slug(&self, _: MediaFacet, _: &str) -> AppResult<Option<Title>> {
            Ok(None)
        }
        async fn get_by_facet_libraries_and_slug(
            &self,
            _: MediaFacet,
            _: &[String],
            _: &str,
        ) -> AppResult<Option<Title>> {
            Ok(None)
        }
        async fn find_by_external_id(&self, _: &str, _: &str) -> AppResult<Option<Title>> {
            Ok(None)
        }
        async fn find_by_external_id_in_facet(
            &self,
            _: MediaFacet,
            _: &str,
            _: &str,
        ) -> AppResult<Option<Title>> {
            Ok(None)
        }
        async fn create_or_get_existing(&self, _: Title) -> AppResult<CreateTitleOutcome> {
            Err(AppError::Repository("not configured".into()))
        }
        async fn create(&self, _: Title) -> AppResult<Title> {
            Err(AppError::Repository("not configured".into()))
        }
        async fn list_titles_due_for_hydration(
            &self,
            _: usize,
            _: &[MediaFacet],
        ) -> AppResult<Vec<PendingTitleHydration>> {
            Ok(vec![])
        }
        async fn mark_title_metadata_hydration_due_now(&self, _: &str) -> AppResult<()> {
            Ok(())
        }
        async fn schedule_title_metadata_hydration_retry(
            &self,
            _: &str,
            _: &str,
            _: i64,
        ) -> AppResult<()> {
            Ok(())
        }
        async fn clear_title_metadata_hydration_retry_state(&self, _: &str) -> AppResult<()> {
            Ok(())
        }
        async fn update_monitored(&self, _: &str, _: bool) -> AppResult<Title> {
            Err(AppError::Repository("not configured".into()))
        }
        async fn update_metadata(
            &self,
            _: &str,
            _: Option<String>,
            _: Option<MediaFacet>,
            _: Option<Vec<String>>,
            _: Option<String>,
        ) -> AppResult<Title> {
            Err(AppError::Repository("not configured".into()))
        }
        async fn update_title_hydrated_metadata(
            &self,
            _: &str,
            _: TitleMetadataUpdate,
        ) -> AppResult<Title> {
            Err(AppError::Repository("not configured".into()))
        }
        async fn replace_match_state(
            &self,
            _: &str,
            _: Vec<scryer_domain::ExternalId>,
            _: Vec<String>,
        ) -> AppResult<Title> {
            Err(AppError::Repository("not configured".into()))
        }
        async fn delete(&self, _: &str) -> AppResult<()> {
            Ok(())
        }
        async fn set_folder_path(&self, _: &str, _: &str) -> AppResult<()> {
            Ok(())
        }
        async fn clear_folder_path(&self, _: &str) -> AppResult<()> {
            Ok(())
        }
        async fn clear_metadata_language_for_all(&self) -> AppResult<u64> {
            Ok(0)
        }
    }

    #[derive(Default)]
    pub struct NullShowRepository;

    #[async_trait]
    impl ShowRepository for NullShowRepository {
        async fn list_series_movie_links_for_title(
            &self,
            _: &str,
        ) -> AppResult<Vec<scryer_domain::SeriesMovieLink>> {
            Ok(vec![])
        }
        async fn list_series_movie_external_id_lookup_matches(
            &self,
            _: &[String],
            _: &[crate::TitleExternalIdLookup],
        ) -> AppResult<Vec<crate::SeriesMovieExternalIdLookupMatch>> {
            Ok(vec![])
        }
        async fn get_series_movie_link_by_id(
            &self,
            _: &str,
        ) -> AppResult<Option<scryer_domain::SeriesMovieLink>> {
            Ok(None)
        }
        async fn find_series_movie_link_by_legacy_collection_id(
            &self,
            _: &str,
        ) -> AppResult<Option<scryer_domain::SeriesMovieLink>> {
            Ok(None)
        }
        async fn upsert_series_movie_link(
            &self,
            link: scryer_domain::SeriesMovieLink,
        ) -> AppResult<scryer_domain::SeriesMovieLink> {
            Ok(link)
        }
        async fn delete_stale_series_movie_links(&self, _: &str, _: &[String]) -> AppResult<()> {
            Ok(())
        }
        async fn list_collections_for_title(&self, _: &str) -> AppResult<Vec<Collection>> {
            Ok(vec![])
        }
        async fn list_collection_external_ids(&self, _: &str) -> AppResult<Vec<ScopedExternalId>> {
            Ok(vec![])
        }
        async fn list_collections_for_titles(
            &self,
            _: &[String],
        ) -> AppResult<std::collections::HashMap<String, Vec<Collection>>> {
            Ok(std::collections::HashMap::new())
        }
        async fn get_collection_by_id(&self, _: &str) -> AppResult<Option<Collection>> {
            Ok(None)
        }
        async fn get_collection_by_ordered_path(&self, _: &str) -> AppResult<Option<Collection>> {
            Ok(None)
        }
        async fn create_collection(&self, _: Collection) -> AppResult<Collection> {
            Err(AppError::Repository("not configured".into()))
        }
        async fn update_collection(&self, _: &str, _: CollectionUpdate) -> AppResult<Collection> {
            Err(AppError::Repository("not configured".into()))
        }
        async fn set_collection_episodes_monitored(&self, _: &str, _: bool) -> AppResult<()> {
            Ok(())
        }
        async fn set_collections_monitored(&self, _: &[String], _: bool) -> AppResult<()> {
            Ok(())
        }
        async fn delete_collection(&self, _: &str) -> AppResult<()> {
            Ok(())
        }
        async fn delete_collections_for_title(&self, _: &str) -> AppResult<()> {
            Ok(())
        }
        async fn list_episodes_for_collection(&self, _: &str) -> AppResult<Vec<Episode>> {
            Ok(vec![])
        }
        async fn list_episodes_for_title(&self, _: &str) -> AppResult<Vec<Episode>> {
            Ok(vec![])
        }
        async fn list_episode_external_ids(&self, _: &str) -> AppResult<Vec<ScopedExternalId>> {
            Ok(vec![])
        }
        async fn get_episode_by_id(&self, _: &str) -> AppResult<Option<Episode>> {
            Ok(None)
        }
        async fn create_episode(&self, _: Episode) -> AppResult<Episode> {
            Err(AppError::Repository("not configured".into()))
        }
        async fn update_episode(&self, _: &str, _: EpisodeUpdate) -> AppResult<Episode> {
            Err(AppError::Repository("not configured".into()))
        }
        async fn set_episodes_monitored(&self, _: &[String], _: bool) -> AppResult<()> {
            Ok(())
        }
        async fn delete_episode(&self, _: &str) -> AppResult<()> {
            Ok(())
        }
        async fn delete_episodes_for_title(&self, _: &str) -> AppResult<()> {
            Ok(())
        }
        async fn find_episode_by_title_and_numbers(
            &self,
            _: &str,
            _: &str,
            _: &str,
        ) -> AppResult<Option<Episode>> {
            Ok(None)
        }
        async fn find_episode_by_title_and_absolute_number(
            &self,
            _: &str,
            _: &str,
        ) -> AppResult<Option<Episode>> {
            Ok(None)
        }
        async fn list_primary_collection_summaries(
            &self,
            _: &[String],
        ) -> AppResult<Vec<PrimaryCollectionSummary>> {
            Ok(vec![])
        }
        async fn list_episodes_in_date_range(
            &self,
            _: &str,
            _: &str,
        ) -> AppResult<Vec<CalendarEpisode>> {
            Ok(vec![])
        }
        async fn replace_anibridge_scoped_external_ids_for_title(
            &self,
            _: &str,
            _: Vec<ScopedExternalId>,
            _: Vec<ScopedExternalId>,
        ) -> AppResult<()> {
            Ok(())
        }
    }

    #[derive(Default)]
    pub struct NullUserRepository;

    #[async_trait]
    impl UserRepository for NullUserRepository {
        async fn get_by_username(&self, _: &str) -> AppResult<Option<User>> {
            Ok(None)
        }
        async fn create(&self, _: User) -> AppResult<User> {
            Err(AppError::Repository("not configured".into()))
        }
        async fn list_all(&self) -> AppResult<Vec<User>> {
            Ok(vec![])
        }
        async fn get_by_id(&self, _: &str) -> AppResult<Option<User>> {
            Ok(None)
        }
        async fn auth_session_version(&self, _: &str) -> AppResult<Option<String>> {
            Ok(None)
        }
        async fn update_password_and_invalidate_sessions(
            &self,
            _: &str,
            _: String,
            _: bool,
            _: &str,
        ) -> AppResult<User> {
            Err(AppError::Repository("not configured".into()))
        }
        async fn update_own_password_and_invalidate_sessions(
            &self,
            _: &str,
            _: String,
            _: bool,
            _: &str,
            _: Option<&str>,
        ) -> AppResult<User> {
            Err(AppError::Repository("not configured".into()))
        }
        async fn update_login_status_and_rotate_session(
            &self,
            _: &str,
            _: scryer_domain::UserLoginStatus,
            _: &str,
        ) -> AppResult<User> {
            Err(AppError::Repository("not configured".into()))
        }
        async fn delete(&self, _: &str) -> AppResult<()> {
            Ok(())
        }
    }

    #[derive(Default)]
    pub struct NullIndexerClient;

    #[async_trait]
    impl IndexerClient for NullIndexerClient {
        async fn search(
            &self,
            _: String,
            _: std::collections::HashMap<String, String>,
            _: Option<String>,
            _: Option<String>,
            _: Option<String>,
            _: Option<Vec<String>>,
            _: Option<IndexerRoutingPlan>,
            _: SearchMode,
            _: crate::IndexerErrorOperation,
            _: Option<u32>,
            _: Option<u32>,
            _: Option<u32>,
            _: Option<i32>,
            _: Vec<scryer_domain::TaggedAlias>,
            _: Option<crate::IndexerSearchLearningContext>,
            _: tokio_util::sync::CancellationToken,
        ) -> AppResult<IndexerSearchResponse> {
            Ok(IndexerSearchResponse {
                completion: crate::IndexerSearchCompletion::Complete,

                indexer_outcomes: Vec::new(),
                results: vec![],
                api_current: None,
                api_max: None,
                grab_current: None,
                grab_max: None,
            })
        }
    }

    #[derive(Default)]
    pub struct NullDownloadClient;

    #[async_trait]
    impl BuiltinDownloadClientConnectionTester for NullDownloadClient {
        async fn test_connection(
            &self,
            _: &str,
            _: &str,
            _: Option<&scryer_domain::ProxyConfig>,
        ) -> AppResult<()> {
            Err(AppError::Repository("not configured".into()))
        }
    }

    #[async_trait]
    impl DownloadClient for NullDownloadClient {
        async fn submit_download(
            &self,
            _: &DownloadClientAddRequest,
        ) -> AppResult<DownloadGrabResult> {
            Err(AppError::Repository("not configured".into()))
        }
    }

    #[derive(Default)]
    pub struct NullDownloadClientConfigRepository;

    #[async_trait]
    impl DownloadClientConfigRepository for NullDownloadClientConfigRepository {
        async fn list(&self, _: Option<String>) -> AppResult<Vec<DownloadClientConfig>> {
            Ok(vec![])
        }
        async fn get_by_id(&self, _: &str) -> AppResult<Option<DownloadClientConfig>> {
            Ok(None)
        }
        async fn create(&self, _: DownloadClientConfig) -> AppResult<DownloadClientConfig> {
            Err(AppError::Repository("not configured".into()))
        }
        async fn update(&self, _: DownloadClientConfigUpdate) -> AppResult<DownloadClientConfig> {
            Err(AppError::Repository("not configured".into()))
        }
        async fn delete(&self, _: &str) -> AppResult<()> {
            Ok(())
        }
        async fn reorder(&self, _: Vec<String>) -> AppResult<()> {
            Ok(())
        }
    }

    #[derive(Default)]
    pub struct NullReleaseAttemptRepository;

    #[async_trait]
    impl ReleaseAttemptRepository for NullReleaseAttemptRepository {
        async fn record_release_attempt(
            &self,
            _: Option<String>,
            _: Option<String>,
            _: Option<String>,
            _: ReleaseDownloadAttemptOutcome,
            _: Option<String>,
            _: Option<String>,
        ) -> AppResult<()> {
            Ok(())
        }
        async fn list_failed_release_signatures(
            &self,
            _: usize,
        ) -> AppResult<Vec<ReleaseDownloadFailureSignature>> {
            Ok(vec![])
        }
        async fn list_failed_release_signatures_for_title(
            &self,
            _: &str,
            _: usize,
        ) -> AppResult<Vec<crate::ReleaseDownloadFailureRecord>> {
            Ok(vec![])
        }
        async fn get_latest_source_password(
            &self,
            _: Option<&str>,
            _: Option<&str>,
            _: Option<&str>,
        ) -> AppResult<Option<String>> {
            Ok(None)
        }
    }

    #[derive(Default)]
    pub struct NullQualityProfileRepository;

    #[async_trait]
    impl QualityProfileRepository for NullQualityProfileRepository {
        async fn list_quality_profiles(
            &self,
            _: &str,
            _: Option<String>,
        ) -> AppResult<Vec<QualityProfile>> {
            Ok(vec![])
        }

        async fn replace_quality_profiles(
            &self,
            _: &str,
            _: Option<String>,
            _: Vec<QualityProfile>,
        ) -> AppResult<()> {
            Ok(())
        }
    }
}

#![allow(dead_code)]

use std::{
    collections::HashMap,
    sync::{Arc, Once},
};

use async_graphql_axum::GraphQLRequest;
use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::State;
use axum::http::{HeaderMap, Request as HttpRequest, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tower::ServiceExt as _;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use scryer_application::{
    AcquisitionScopeStateRepository, AppResult, AppServices, AppUseCase, AuthenticatedTokenClaims,
    BlocklistRepository, ExternalIdentityVerifier, FacetRegistry, HousekeepingRepository,
    IndexerPluginProvider, JwtAuthConfig, MediaFileRepository, MovieFacetHandler,
    OAuthAuthorizationSource, PendingReleaseRepository, SeriesFacetHandler,
    SubtitleDownloadRepository,
};
use scryer_infrastructure_acquisition::{
    downloads::{
        clients::NzbgetDownloadClient, config_store::DownloadClientConfigStore,
        staged_nzb_store::FileSystemStagedNzbStore,
    },
    indexers::{
        config_store::IndexerConfigStore, search_client::MultiIndexerSearchClient,
        stats::InMemoryIndexerStatsTracker,
    },
};
use scryer_infrastructure_configuration::{
    customization::{
        maintenance_evaluation_store::MaintenanceEvaluationStore,
        maintenance_rule_set_store::MaintenanceRuleSetStore, plugin_store::PluginStore,
        post_processing_script_store::PostProcessingScriptStore, rule_set_store::RuleSetStore,
    },
    settings::{quality_profile_store::QualityProfileStore, settings_store::SettingsStore},
};
use scryer_infrastructure_crypto::EncryptionKey;
use scryer_infrastructure_datastore::{SqliteServices, keystore};
use scryer_infrastructure_identity::{
    external_identity::HttpExternalIdentityVerifier,
    oauth::store::OAuthStore,
    users::{store::UserStore, totp_store::TotpStore, webauthn_store::WebauthnStore},
};
use scryer_infrastructure_library::media::{
    images::{image_proxy_store::ImageProxyStore, title_image_store::TitleImageStore},
    libraries::{
        scan_unmatched_store::LibraryScanUnmatchedStore,
        scanner::FileSystemLibraryScanner,
        state_store::{
            BlocklistStore, HousekeepingStore, LibraryProbeStore, PendingReleaseStore,
            SubtitleDownloadStore, WantedStore,
        },
        store::LibraryStore,
    },
    search::media_file_store::MediaFileStore,
    servers::MediaServerConnectionStore,
    shows::store::ShowStore,
    titles::store::TitleStore,
};
use scryer_infrastructure_metadata::metadata::gateway::client::{
    MetadataGatewayClient, SmgEnrollmentConfig,
};
use scryer_infrastructure_workflow::workflow::{
    release_store::ReleaseStore,
    stores::{
        AcquisitionStore, DomainEventStore, DownloadQueueCommandStore, DownloadRegistryStore,
        DownloadSubmissionStore, ExternalImportMonitorStore, ExternalImportSetupSecretDraftStore,
        ImportStore, WorkflowOperationStore,
    },
};
use scryer_interface::context::{
    AuthRuntimeStateHandle, AuthRuntimeStateSnapshot, MfaVerification,
};
use scryer_interface::{ApiSchema, build_schema};

pub fn disable_platform_keystore_for_tests() {
    keystore::disable_platform_keystore_for_tests();
}

static TEST_WASMTIME_RUNTIME: Once = Once::new();

pub fn initialize_wasm_runtime_for_tests() {
    TEST_WASMTIME_RUNTIME.call_once(|| {
        // Nextest gives each test a process, so this test-only cache is shared
        // across the suite instead of recompiling the same modules per test.
        let cache_dir = std::env::temp_dir().join("scryer-wasmtime-integration-cache");
        scryer_plugins::initialize_wasm_runtime_at(cache_dir)
            .expect("test Wasmtime cache must initialize");
    });
}

/// Shared integration-test context.
///
/// Boots wiremock servers for external APIs, in-memory SQLite, real
/// infrastructure clients pointed at wiremock, a full `AppUseCase`,
/// GraphQL schema, and an axum server on a random port.
pub struct TestContext {
    pub nzbget_server: MockServer,
    pub nzbgeek_server: MockServer,
    pub smg_server: MockServer,
    /// Base URL of the test axum server (e.g. `http://127.0.0.1:12345`).
    pub app_url: String,
    pub schema: ApiSchema,
    pub auth_runtime: AuthRuntimeStateHandle,
    pub app: AppUseCase,
    pub titles: TitleStore,
    pub shows: ShowStore,
    pub libraries: LibraryStore,
    pub users: UserStore,
    pub customization: PluginStore,
    pub library_probe: LibraryProbeStore,
    pub library_state: TestLibraryStateStore,
    pub library_scan_unmatched: LibraryScanUnmatchedStore,
    /// The same tracker the app writes grabs and queries into, so tests can
    /// seed stats and read them back through the API.
    pub indexer_stats: Arc<dyn scryer_application::IndexerStatsTracker>,
    pub media_files: MediaFileStore,
    pub db: SqliteServices,
    pub settings_store: Arc<SettingsStore>,
    pub app_data_dir: tempfile::TempDir,
    pub staged_nzb_store: Arc<FileSystemStagedNzbStore>,
    pub staged_nzb_dir: tempfile::TempDir,
}

#[derive(Clone)]
pub struct TestLibraryStateStore {
    pub wanted: WantedStore,
    pub pending_releases: PendingReleaseStore,
    pub blocklist: BlocklistStore,
    pub housekeeping: HousekeepingStore,
    pub subtitle_downloads: SubtitleDownloadStore,
}

#[async_trait]
impl AcquisitionScopeStateRepository for TestLibraryStateStore {
    async fn upsert_acquisition_scope_state(
        &self,
        item: &scryer_application::AcquisitionScopeState,
    ) -> AppResult<String> {
        self.wanted.upsert_acquisition_scope_state(item).await
    }

    async fn update_acquisition_scope_status(
        &self,
        id: &str,
        status: &str,
        last_search_at: Option<&str>,
        grabbed_release: Option<&str>,
    ) -> AppResult<()> {
        self.wanted
            .update_acquisition_scope_status(id, status, last_search_at, grabbed_release)
            .await
    }

    async fn record_acquisition_scope_search_attempt(
        &self,
        id: &str,
        last_search_at: &str,
    ) -> AppResult<()> {
        self.wanted
            .record_acquisition_scope_search_attempt(id, last_search_at)
            .await
    }

    async fn get_acquisition_scope_state_for_title(
        &self,
        title_id: &str,
        episode_id: Option<&str>,
    ) -> AppResult<Option<scryer_application::AcquisitionScopeState>> {
        self.wanted
            .get_acquisition_scope_state_for_title(title_id, episode_id)
            .await
    }

    async fn delete_acquisition_scope_states_for_title(&self, title_id: &str) -> AppResult<()> {
        self.wanted
            .delete_acquisition_scope_states_for_title(title_id)
            .await
    }

    async fn delete_acquisition_scope_states_for_collection(
        &self,
        collection_id: &str,
    ) -> AppResult<()> {
        self.wanted
            .delete_acquisition_scope_states_for_collection(collection_id)
            .await
    }

    async fn delete_acquisition_scope_states_for_series_movie_link(
        &self,
        series_movie_link_id: &str,
    ) -> AppResult<()> {
        self.wanted
            .delete_acquisition_scope_states_for_series_movie_link(series_movie_link_id)
            .await
    }

    async fn delete_acquisition_scope_states_for_episode(&self, episode_id: &str) -> AppResult<()> {
        self.wanted
            .delete_acquisition_scope_states_for_episode(episode_id)
            .await
    }

    async fn insert_release_decision(
        &self,
        decision: &scryer_application::ReleaseDecision,
    ) -> AppResult<String> {
        self.wanted.insert_release_decision(decision).await
    }

    async fn get_acquisition_scope_state_by_id(
        &self,
        id: &str,
    ) -> AppResult<Option<scryer_application::AcquisitionScopeState>> {
        self.wanted.get_acquisition_scope_state_by_id(id).await
    }

    async fn list_acquisition_scope_states(
        &self,
        query: scryer_application::AcquisitionScopeStatesQuery,
    ) -> AppResult<Vec<scryer_application::AcquisitionScopeState>> {
        self.wanted.list_acquisition_scope_states(query).await
    }

    async fn count_acquisition_scope_states(
        &self,
        query: scryer_application::AcquisitionScopeStatesQuery,
    ) -> AppResult<i64> {
        self.wanted.count_acquisition_scope_states(query).await
    }

    async fn list_release_decisions_for_title(
        &self,
        title_id: &str,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<scryer_application::ReleaseDecision>> {
        self.wanted
            .list_release_decisions_for_title(title_id, limit, offset)
            .await
    }

    async fn list_release_decisions_for_acquisition_scope_state(
        &self,
        wanted_item_id: &str,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<scryer_application::ReleaseDecision>> {
        self.wanted
            .list_release_decisions_for_acquisition_scope_state(wanted_item_id, limit, offset)
            .await
    }

    async fn count_release_decisions_for_title(&self, title_id: &str) -> AppResult<i64> {
        self.wanted
            .count_release_decisions_for_title(title_id)
            .await
    }

    async fn count_release_decisions_for_acquisition_scope_state(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<i64> {
        self.wanted
            .count_release_decisions_for_acquisition_scope_state(wanted_item_id)
            .await
    }
}

#[async_trait]
impl PendingReleaseRepository for TestLibraryStateStore {
    async fn insert_pending_release(
        &self,
        release: &scryer_application::PendingRelease,
    ) -> AppResult<String> {
        self.pending_releases.insert_pending_release(release).await
    }

    async fn insert_pending_release_with_role(
        &self,
        release: &scryer_application::PendingRelease,
        role: scryer_application::PendingReleaseRole,
    ) -> AppResult<String> {
        self.pending_releases
            .insert_pending_release_with_role(release, role)
            .await
    }

    async fn insert_pending_release_observation(
        &self,
        release: &scryer_application::PendingRelease,
        observation: &scryer_application::PendingReleaseObservation,
    ) -> AppResult<String> {
        self.pending_releases
            .insert_pending_release_observation(release, observation)
            .await
    }

    async fn list_expired_pending_releases(
        &self,
        now: &str,
    ) -> AppResult<Vec<scryer_application::PendingRelease>> {
        self.pending_releases
            .list_expired_pending_releases(now)
            .await
    }

    async fn list_waiting_pending_releases(
        &self,
    ) -> AppResult<Vec<scryer_application::PendingRelease>> {
        self.pending_releases.list_waiting_pending_releases().await
    }

    async fn list_active_release_age_unknown_pending_releases(
        &self,
    ) -> AppResult<Vec<scryer_application::PendingRelease>> {
        self.pending_releases
            .list_active_release_age_unknown_pending_releases()
            .await
    }

    async fn get_pending_release(
        &self,
        id: &str,
    ) -> AppResult<Option<scryer_application::PendingRelease>> {
        self.pending_releases.get_pending_release(id).await
    }

    async fn list_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<Vec<scryer_application::PendingRelease>> {
        self.pending_releases
            .list_pending_releases_for_wanted_item(wanted_item_id)
            .await
    }

    async fn list_pending_releases_for_title(
        &self,
        title_id: &str,
    ) -> AppResult<Vec<scryer_application::PendingRelease>> {
        self.pending_releases
            .list_pending_releases_for_title(title_id)
            .await
    }

    async fn list_pending_releases_page(
        &self,
        query: scryer_application::PendingReleasesPageQuery,
    ) -> AppResult<(Vec<scryer_application::PendingRelease>, i64)> {
        self.pending_releases
            .list_pending_releases_page(query)
            .await
    }

    async fn update_pending_release_status(
        &self,
        id: &str,
        status: scryer_application::PendingReleaseStatus,
        grabbed_at: Option<&str>,
    ) -> AppResult<()> {
        self.pending_releases
            .update_pending_release_status(id, status, grabbed_at)
            .await
    }

    async fn expire_pending_release(&self, id: &str, decision_code: &str) -> AppResult<()> {
        self.pending_releases
            .expire_pending_release(id, decision_code)
            .await
    }

    async fn mark_release_age_unknown_pending_release_needs_review(
        &self,
        id: &str,
        decision_code: &str,
    ) -> AppResult<()> {
        self.pending_releases
            .mark_release_age_unknown_pending_release_needs_review(id, decision_code)
            .await
    }

    async fn update_pending_release_delay_until(
        &self,
        id: &str,
        delay_until: &str,
    ) -> AppResult<()> {
        self.pending_releases
            .update_pending_release_delay_until(id, delay_until)
            .await
    }

    async fn list_standby_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<Vec<scryer_application::PendingRelease>> {
        self.pending_releases
            .list_standby_pending_releases_for_wanted_item(wanted_item_id)
            .await
    }

    async fn list_standby_pending_releases_for_title(
        &self,
        title_id: &str,
    ) -> AppResult<Vec<scryer_application::PendingRelease>> {
        self.pending_releases
            .list_standby_pending_releases_for_title(title_id)
            .await
    }

    async fn count_standby_pending_releases_for_wanted_items(
        &self,
        wanted_item_ids: &[String],
    ) -> AppResult<std::collections::HashMap<String, i64>> {
        self.pending_releases
            .count_standby_pending_releases_for_wanted_items(wanted_item_ids)
            .await
    }

    async fn delete_standby_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<()> {
        self.pending_releases
            .delete_standby_pending_releases_for_wanted_item(wanted_item_id)
            .await
    }

    async fn list_all_standby_pending_releases(
        &self,
    ) -> AppResult<Vec<scryer_application::PendingRelease>> {
        self.pending_releases
            .list_all_standby_pending_releases()
            .await
    }

    async fn compare_and_set_pending_release_status(
        &self,
        id: &str,
        current_status: scryer_application::PendingReleaseStatus,
        next_status: scryer_application::PendingReleaseStatus,
        grabbed_at: Option<&str>,
    ) -> AppResult<bool> {
        self.pending_releases
            .compare_and_set_pending_release_status(id, current_status, next_status, grabbed_at)
            .await
    }

    async fn retire_lower_or_equal_overlapping_pending_releases(
        &self,
        lower_or_equal_ids: &[String],
    ) -> AppResult<()> {
        self.pending_releases
            .retire_lower_or_equal_overlapping_pending_releases(lower_or_equal_ids)
            .await
    }

    async fn delete_pending_releases_for_title(&self, title_id: &str) -> AppResult<()> {
        self.pending_releases
            .delete_pending_releases_for_title(title_id)
            .await
    }
}

#[async_trait]
impl BlocklistRepository for TestLibraryStateStore {
    async fn block(&self, entry: &scryer_application::NewBlocklistEntry) -> AppResult<bool> {
        self.blocklist.block(entry).await
    }

    async fn list_for_title(
        &self,
        title_id: &str,
        limit: usize,
    ) -> AppResult<Vec<scryer_domain::BlocklistEntry>> {
        self.blocklist.list_for_title(title_id, limit).await
    }

    async fn list_all(
        &self,
        limit: usize,
        offset: usize,
    ) -> AppResult<(Vec<scryer_domain::BlocklistEntry>, i64)> {
        self.blocklist.list_all(limit, offset).await
    }

    async fn is_blocked(
        &self,
        title_id: &str,
        indexer_id: &str,
        release_name: &str,
        info_hash: Option<&str>,
    ) -> AppResult<bool> {
        self.blocklist
            .is_blocked(title_id, indexer_id, release_name, info_hash)
            .await
    }

    async fn get(&self, id: &str) -> AppResult<Option<scryer_domain::BlocklistEntry>> {
        self.blocklist.get(id).await
    }

    async fn remove(&self, id: &str) -> AppResult<()> {
        self.blocklist.remove(id).await
    }

    async fn delete_for_title(&self, title_id: &str) -> AppResult<()> {
        self.blocklist.delete_for_title(title_id).await
    }

    async fn delete_for_indexer(&self, indexer_id: &str) -> AppResult<()> {
        self.blocklist.delete_for_indexer(indexer_id).await
    }
}

#[async_trait]
impl HousekeepingRepository for TestLibraryStateStore {
    async fn delete_stale_workflow_operations(
        &self,
        completed_days: i64,
        warning_failed_days: i64,
    ) -> AppResult<u32> {
        self.housekeeping
            .delete_stale_workflow_operations(completed_days, warning_failed_days)
            .await
    }

    async fn delete_release_decisions_older_than(&self, days: i64) -> AppResult<u32> {
        self.housekeeping
            .delete_release_decisions_older_than(days)
            .await
    }

    async fn delete_release_attempts_older_than(&self, days: i64) -> AppResult<u32> {
        self.housekeeping
            .delete_release_attempts_older_than(days)
            .await
    }

    async fn delete_history_events_older_than(&self, days: i64) -> AppResult<u32> {
        self.housekeeping
            .delete_history_events_older_than(days)
            .await
    }

    async fn delete_domain_events_older_than_for_types(
        &self,
        days: i64,
        event_types: &[scryer_domain::DomainEventType],
    ) -> AppResult<u32> {
        self.housekeeping
            .delete_domain_events_older_than_for_types(days, event_types)
            .await
    }

    async fn delete_title_history_older_than(&self, days: i64) -> AppResult<u32> {
        self.housekeeping
            .delete_title_history_older_than(days)
            .await
    }

    async fn delete_download_import_artifacts_older_than(&self, days: i64) -> AppResult<u32> {
        self.housekeeping
            .delete_download_import_artifacts_older_than(days)
            .await
    }

    async fn delete_terminal_imports_older_than(&self, days: i64) -> AppResult<u32> {
        self.housekeeping
            .delete_terminal_imports_older_than(days)
            .await
    }

    async fn delete_terminal_download_queue_commands_older_than(
        &self,
        days: i64,
    ) -> AppResult<u32> {
        self.housekeeping
            .delete_terminal_download_queue_commands_older_than(days)
            .await
    }

    async fn delete_rule_set_history_older_than(&self, days: i64) -> AppResult<u32> {
        self.housekeeping
            .delete_rule_set_history_older_than(days)
            .await
    }

    async fn delete_history_events_for_title_ids(&self, title_ids: &[String]) -> AppResult<u32> {
        self.housekeeping
            .delete_history_events_for_title_ids(title_ids)
            .await
    }

    async fn delete_download_import_artifacts_for_title_ids(
        &self,
        title_ids: &[String],
    ) -> AppResult<u32> {
        self.housekeeping
            .delete_download_import_artifacts_for_title_ids(title_ids)
            .await
    }

    async fn delete_release_attempts_for_title_ids(&self, title_ids: &[String]) -> AppResult<u32> {
        self.housekeeping
            .delete_release_attempts_for_title_ids(title_ids)
            .await
    }

    async fn list_all_media_file_paths(&self) -> AppResult<Vec<(String, String)>> {
        self.housekeeping.list_all_media_file_paths().await
    }

    async fn list_media_files_with_roots(
        &self,
    ) -> AppResult<Vec<scryer_application::HousekeepingMediaFileRootRow>> {
        self.housekeeping.list_media_files_with_roots().await
    }

    async fn delete_media_files_by_ids(&self, ids: &[String]) -> AppResult<u32> {
        self.housekeeping.delete_media_files_by_ids(ids).await
    }

    async fn prune_unreferenced_title_image_blobs(&self, limit: u32) -> AppResult<u32> {
        self.housekeeping
            .prune_unreferenced_title_image_blobs(limit)
            .await
    }

    async fn run_database_maintenance(&self) -> AppResult<()> {
        self.housekeeping.run_database_maintenance().await
    }
}

#[async_trait]
impl SubtitleDownloadRepository for TestLibraryStateStore {
    async fn list_for_title(
        &self,
        title_id: &str,
    ) -> AppResult<Vec<scryer_domain::SubtitleDownload>> {
        self.subtitle_downloads.list_for_title(title_id).await
    }

    async fn get(&self, id: &str) -> AppResult<Option<scryer_domain::SubtitleDownload>> {
        self.subtitle_downloads.get(id).await
    }

    async fn list_for_media_file(
        &self,
        media_file_id: &str,
    ) -> AppResult<Vec<scryer_domain::SubtitleDownload>> {
        self.subtitle_downloads
            .list_for_media_file(media_file_id)
            .await
    }

    async fn list_probe_cache_for_media_file(
        &self,
        media_file_id: &str,
    ) -> AppResult<Vec<scryer_application::subtitles::ExternalSubtitleProbeCacheEntry>> {
        self.subtitle_downloads
            .list_probe_cache_for_media_file(media_file_id)
            .await
    }

    async fn list_blocklist_for_media_file(
        &self,
        media_file_id: &str,
    ) -> AppResult<Vec<scryer_domain::SubtitleBlocklistEntry>> {
        self.subtitle_downloads
            .list_blocklist_for_media_file(media_file_id)
            .await
    }

    async fn insert(&self, download: &scryer_domain::SubtitleDownload) -> AppResult<()> {
        self.subtitle_downloads.insert(download).await
    }

    async fn upsert_probe_cache_entry(
        &self,
        entry: &scryer_application::subtitles::ExternalSubtitleProbeCacheEntry,
    ) -> AppResult<()> {
        self.subtitle_downloads
            .upsert_probe_cache_entry(entry)
            .await
    }

    async fn set_synced(&self, id: &str, synced: bool) -> AppResult<()> {
        self.subtitle_downloads.set_synced(id, synced).await
    }

    async fn delete(&self, id: &str) -> AppResult<Option<scryer_domain::SubtitleDownload>> {
        self.subtitle_downloads.delete(id).await
    }

    async fn delete_probe_cache_entry(
        &self,
        media_file_id: &str,
        file_path: &str,
    ) -> AppResult<()> {
        self.subtitle_downloads
            .delete_probe_cache_entry(media_file_id, file_path)
            .await
    }

    async fn is_blocklisted(
        &self,
        media_file_id: &str,
        provider: &str,
        provider_file_id: &str,
    ) -> AppResult<bool> {
        self.subtitle_downloads
            .is_blocklisted(media_file_id, provider, provider_file_id)
            .await
    }

    async fn blocklist(
        &self,
        media_file_id: &str,
        provider: &str,
        provider_file_id: &str,
        language: &str,
        reason: Option<&str>,
    ) -> AppResult<()> {
        self.subtitle_downloads
            .blocklist(media_file_id, provider, provider_file_id, language, reason)
            .await
    }
}

pub fn disabled_auth_runtime_handle() -> AuthRuntimeStateHandle {
    AuthRuntimeStateHandle::new(AuthRuntimeStateSnapshot {
        form_login_enabled: false,
        skip_login_for_local_ips: false,
        effective_form_login_enabled: false,
        webauthn_configured: false,
        passkey_enabled: false,
        env_override_active: false,
        env_override_description: None,
        epoch: 0,
    })
}

impl TestContext {
    pub async fn new() -> Self {
        Self::new_with_external_identity_verifier(Arc::new(HttpExternalIdentityVerifier::new()))
            .await
    }

    pub async fn new_with_external_identity_verifier(
        external_identity_verifier: Arc<dyn ExternalIdentityVerifier>,
    ) -> Self {
        disable_platform_keystore_for_tests();
        initialize_wasm_runtime_for_tests();

        // Start wiremock mock servers for each external API
        let nzbget_server = MockServer::start().await;
        let nzbgeek_server = MockServer::start().await;
        let smg_server = MockServer::start().await;
        mount_default_smg_metadata_mocks(&smg_server).await;

        // In-memory SQLite with migrations applied
        let db = SqliteServices::new(":memory:")
            .await
            .expect("failed to create in-memory SQLite");
        db.set_encryption_key(EncryptionKey::generate())
            .await
            .expect("failed to configure test encryption key");
        let app_data_dir = tempfile::Builder::new()
            .prefix("scryer-test-data-")
            .tempdir_in("/tmp")
            .expect("failed to create app data tempdir");
        let staged_nzb_dir = tempfile::TempDir::new().expect("failed to create staged nzb tempdir");
        let staged_nzb_store = Arc::new(
            FileSystemStagedNzbStore::new(staged_nzb_dir.path())
                .await
                .expect("failed to create staged nzb store"),
        );
        let staged_nzb_pipeline_limit = Arc::new(tokio::sync::Semaphore::new(4));
        let datastore = db.datastore();
        let release_store = Arc::new(ReleaseStore::new(
            datastore.clone(),
            db.encryption_key_state(),
        ));
        let settings_store = Arc::new(SettingsStore::new(
            datastore.clone(),
            db.encryption_key_state(),
        ));
        let quality_profile_store = Arc::new(QualityProfileStore::new(datastore.clone()));

        // Real clients pointed at wiremock URLs
        let nzbget = NzbgetDownloadClient::with_staged_nzb_store(
            nzbget_server.uri(),
            Some("test-user".to_string()),
            Some("test-pass".to_string()),
            "SCORE".to_string(),
            staged_nzb_store.clone(),
            staged_nzb_pipeline_limit.clone(),
        );

        let indexer_config_store = Arc::new(IndexerConfigStore::new(
            datastore.clone(),
            db.encryption_key_state(),
        ));
        let download_client_config_store = Arc::new(DownloadClientConfigStore::new(
            datastore.clone(),
            db.encryption_key_state(),
        ));

        // Build indexer client backed by built-in WASM plugins (using DynamicPluginProvider
        // so reload_plugins works in integration tests)
        let plugin_provider: Arc<dyn IndexerPluginProvider> =
            Arc::new(scryer_plugins::DynamicPluginProvider::new(
                scryer_plugins::build_indexer_plugin_provider(&[], &[]),
            ));
        let indexer_stats: Arc<dyn scryer_application::IndexerStatsTracker> =
            Arc::new(InMemoryIndexerStatsTracker::new(None));
        let indexer_client = MultiIndexerSearchClient::new(
            indexer_config_store.clone(),
            indexer_stats.clone(),
            plugin_provider.clone(),
        );

        let metadata_gateway = MetadataGatewayClient::new_with_enrollment_store(
            format!("{}/graphql", smg_server.uri()),
            settings_store.clone(),
            SmgEnrollmentConfig {
                registration_secret: None,
            },
        );

        // Build repository implementations from the shared DB runtime.
        let title_store = TitleStore::new(datastore.clone());
        let show_store = ShowStore::new(datastore.clone());
        let library_store = LibraryStore::new(datastore.clone());
        let user_store = UserStore::new(datastore.clone());
        let totp_store = TotpStore::new(datastore.clone(), db.encryption_key_state());
        let titles: Arc<dyn scryer_application::TitleRepository> = Arc::new(title_store.clone());
        let shows: Arc<dyn scryer_application::ShowRepository> = Arc::new(show_store.clone());
        let users: Arc<dyn scryer_application::UserRepository> = Arc::new(user_store.clone());
        let ui_settings: Arc<dyn scryer_application::UserUiSettingsRepository> =
            Arc::new(user_store.clone());
        let indexer_configs: Arc<dyn scryer_application::IndexerConfigRepository> =
            indexer_config_store;
        let download_client_configs: Arc<dyn scryer_application::DownloadClientConfigRepository> =
            download_client_config_store;
        let release_attempts: Arc<dyn scryer_application::ReleaseAttemptRepository> = release_store;
        let settings: Arc<dyn scryer_application::SettingsRepository> = settings_store.clone();
        let quality_profiles: Arc<dyn scryer_application::QualityProfileRepository> =
            quality_profile_store.clone();

        let library_probe_store = LibraryProbeStore::new(datastore.clone());
        let wanted_store = WantedStore::new(datastore.clone());
        let pending_release_store =
            PendingReleaseStore::new(datastore.clone(), db.encryption_key_state());
        let blocklist_store = BlocklistStore::new(datastore.clone());
        let housekeeping_store = HousekeepingStore::new(datastore.clone());
        let subtitle_download_store = SubtitleDownloadStore::new(datastore.clone());
        let library_state_store = TestLibraryStateStore {
            wanted: wanted_store.clone(),
            pending_releases: pending_release_store.clone(),
            blocklist: blocklist_store.clone(),
            housekeeping: housekeeping_store.clone(),
            subtitle_downloads: subtitle_download_store.clone(),
        };
        let library_scan_unmatched_store = LibraryScanUnmatchedStore::new(datastore.clone());
        let media_file_store = MediaFileStore::new(datastore.clone());
        let title_image_store = TitleImageStore::new(datastore.clone());
        let image_proxy_store = ImageProxyStore::new(datastore.clone());
        let rule_set_store = RuleSetStore::new(datastore.clone());
        let maintenance_rule_set_store = MaintenanceRuleSetStore::new(datastore.clone());
        let maintenance_evaluation_store = MaintenanceEvaluationStore::new(datastore.clone());
        let post_processing_script_store = PostProcessingScriptStore::new(datastore.clone());
        let plugin_store = PluginStore::new(datastore.clone());
        let oauth_store = OAuthStore::new(datastore.clone());
        let domain_event_store = Arc::new(DomainEventStore::new(datastore.clone()));
        let acquisition_store = Arc::new(AcquisitionStore::new(datastore.clone()));
        let download_submission_store = Arc::new(DownloadSubmissionStore::new(datastore.clone()));
        let download_registry_store = Arc::new(DownloadRegistryStore::new(datastore.clone()));
        let import_store = Arc::new(ImportStore::new(datastore.clone()));
        let external_import_monitor_store =
            Arc::new(ExternalImportMonitorStore::new(datastore.clone()));
        let external_import_setup_secret_draft_store = Arc::new(
            ExternalImportSetupSecretDraftStore::new(datastore.clone(), db.encryption_key_state()),
        );
        let download_queue_command_store =
            Arc::new(DownloadQueueCommandStore::new(datastore.clone()));
        let workflow_operation_store = Arc::new(WorkflowOperationStore::new(datastore.clone()));
        let services = AppServices::builder(
            titles,
            shows,
            users,
            indexer_configs,
            Arc::new(indexer_client),
            Arc::new(nzbget),
            download_client_configs,
            release_attempts,
            settings,
            quality_profiles,
            app_data_dir.path().display().to_string(),
        )
        .with_media_files(Arc::new(media_file_store.clone()))
        .with_acquisition_scope_states(Arc::new(wanted_store))
        .with_pending_releases(Arc::new(pending_release_store))
        .with_blocklist_repo(Arc::new(blocklist_store))
        .with_library_probe_signatures(Arc::new(library_probe_store.clone()))
        .with_library_scan_unmatched_items(Arc::new(library_scan_unmatched_store.clone()))
        .with_title_images(Arc::new(title_image_store))
        .with_image_proxy(Arc::new(image_proxy_store))
        .with_housekeeping(Arc::new(housekeeping_store))
        .with_subtitle_downloads(Arc::new(subtitle_download_store))
        .with_libraries(Arc::new(library_store.clone()))
        .with_external_account_store(Arc::new(user_store.clone()))
        .with_user_ui_settings_store(ui_settings)
        .with_external_identity_verifier(external_identity_verifier)
        .with_media_server_connection_store(Arc::new(MediaServerConnectionStore::new(
            datastore.clone(),
            db.encryption_key_state(),
        )))
        .with_webauthn_store(Arc::new(WebauthnStore::new(datastore.clone())))
        .with_totp_store(Arc::new(totp_store))
        .with_oauth_store(Arc::new(oauth_store))
        .with_rule_set_store(Arc::new(rule_set_store))
        .with_maintenance_rule_set_store(Arc::new(maintenance_rule_set_store))
        .with_maintenance_evaluation_store(Arc::new(maintenance_evaluation_store))
        .with_post_processing_script_store(Arc::new(post_processing_script_store))
        .with_plugin_installation_store(Arc::new(plugin_store.clone()))
        .with_acquisition_state(acquisition_store)
        .with_domain_events(domain_event_store)
        .with_download_queue_commands(download_queue_command_store)
        .with_download_registry(download_registry_store)
        .with_download_submissions(download_submission_store)
        .with_external_import_monitor_snapshots(external_import_monitor_store)
        .with_external_import_setup_secret_drafts(external_import_setup_secret_draft_store)
        .with_import_artifacts(import_store.clone())
        .with_imports(import_store)
        .with_job_runs(workflow_operation_store.clone())
        .with_system_info(settings_store.clone())
        .with_metadata_gateway(Arc::new(metadata_gateway))
        .with_library_scanner(Arc::new(FileSystemLibraryScanner::new()))
        .with_indexer_stats(indexer_stats.clone())
        .with_plugin_provider(plugin_provider)
        .with_staged_nzb_store(staged_nzb_store.clone())
        .with_staged_nzb_pipeline_limit(staged_nzb_pipeline_limit)
        .with_workflow_operations(workflow_operation_store)
        // The plugin catalog client picks artifacts by matching their declared
        // requirements against this capability-token set — WASI targets and
        // wasm features in one namespace. Left empty, no catalog artifact is
        // runnable and every downloadable plugin disappears from the listing,
        // so the test host declares what the real one does.
        .with_supported_plugin_required_features(
            scryer_plugins::detect_plugin_runtime_capabilities(),
        )
        .build();

        // Facet registry with all built-in facets
        let mut registry = FacetRegistry::new();
        registry.register(Arc::new(MovieFacetHandler));
        registry.register(Arc::new(SeriesFacetHandler::new(
            scryer_domain::MediaFacet::Series,
        )));
        registry.register(Arc::new(SeriesFacetHandler::new(
            scryer_domain::MediaFacet::Anime,
        )));
        let facet_registry = Arc::new(registry);

        let app = AppUseCase::new(
            services,
            JwtAuthConfig {
                issuer: "scryer-test".to_string(),
                access_ttl_seconds: 3600,
                jwt_signing_salt: "test-salt".to_string(),
            },
            facet_registry,
        );

        // Build the GraphQL schema with authentication disabled.
        let auth_runtime = disabled_auth_runtime_handle();
        let schema = build_schema(app.clone(), auth_runtime.clone());

        // Start axum server on a random port
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("failed to bind test server");
        let addr = listener.local_addr().expect("failed to get local addr");
        let app_url = format!("http://{addr}");

        let router = build_test_router(app.clone(), schema.clone(), auth_runtime.clone());
        tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("test server failed");
        });

        Self {
            nzbget_server,
            nzbgeek_server,
            smg_server,
            app_url,
            schema,
            auth_runtime,
            app,
            titles: title_store,
            shows: show_store,
            libraries: library_store,
            users: user_store,
            customization: plugin_store,
            library_probe: library_probe_store,
            library_state: library_state_store,
            library_scan_unmatched: library_scan_unmatched_store,
            indexer_stats,
            media_files: media_file_store,
            db,
            settings_store,
            app_data_dir,
            staged_nzb_store,
            staged_nzb_dir,
        }
    }

    pub async fn link_primary_file_to_episode(
        &self,
        title_id: &str,
        file_id: &str,
        episode_id: &str,
    ) -> AppResult<()> {
        self.media_files
            .link_file_to_episode(file_id, episode_id)
            .await?;
        self.media_files
            .set_media_file_roles_for_episode(title_id, episode_id, file_id, &[])
            .await
    }

    /// URL for the GraphQL endpoint.
    pub fn graphql_url(&self) -> String {
        format!("{}/graphql", self.app_url)
    }

    /// Build a reqwest client suitable for hitting the test server.
    pub fn http_client(&self) -> reqwest::Client {
        scryer_outbound_http::generic_reqwest_client()
    }

    pub async fn graphql_json(&self, query: &str, variables: Value, token: Option<&str>) -> Value {
        let body = serde_json::to_vec(&json!({ "query": query, "variables": variables }))
            .expect("serialize graphql request");
        let mut request = HttpRequest::builder()
            .method("POST")
            .uri("/graphql")
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(token) = token {
            request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }

        let app = self.app.clone();
        let schema = self.schema.clone();
        let auth_runtime = self.auth_runtime.clone();
        let request = request
            .body(Body::from(body))
            .expect("build graphql request");
        let response = tokio::spawn(async move {
            build_test_router(app, schema, auth_runtime)
                .oneshot(request)
                .await
        })
        .await
        .expect("graphql request task should finish")
        .expect("graphql request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read graphql response body");
        serde_json::from_slice(&bytes).expect("should be valid JSON")
    }
}

async fn mount_default_smg_metadata_mocks(server: &MockServer) {
    let fixture = json!({
        "data": {
            "metadataBulk": {
                "movies": [{
                    "tvdb_id": 123456,
                    "name": "Test Movie Title",
                    "slug": "test-movie-title",
                    "year": 2024,
                    "status": "Released",
                    "overview": "A gripping tale of testing integration.",
                    "poster_url": "https://artworks.thetvdb.com/banners/movies/123456/posters/test.jpg",
                    "language": "eng",
                    "runtime_minutes": 142,
                    "sort_title": "Test Movie Title",
                    "imdb_id": "tt1234567",
                    "canonical_tags": [
                        {
                            "key": "canonical:genre:action",
                            "category": "genre",
                            "name": "Action",
                            "confidence": 1.0
                        },
                        {
                            "key": "canonical:genre:thriller",
                            "category": "genre",
                            "name": "Thriller",
                            "confidence": 1.0
                        }
                    ],
                    "studio": "Test Studios",
                    "tmdb_release_date": "2024-06-15"
                }],
                "series": [{
                    "tvdb_id": 345678,
                    "name": "Test Show Name",
                    "sort_name": "Test Show Name",
                    "slug": "test-show-name",
                    "status": "Continuing",
                    "year": 2023,
                    "first_aired": "2023-09-15",
                    "overview": "A compelling drama about software testing.",
                    "network": "Test Network",
                    "runtime_minutes": 45,
                    "poster_url": "https://artworks.thetvdb.com/banners/series/345678/posters/test.jpg",
                    "country": "usa",
                    "canonical_tags": [
                        {
                            "key": "canonical:genre:drama",
                            "category": "genre",
                            "name": "Drama",
                            "confidence": 1.0
                        },
                        {
                            "key": "canonical:genre:thriller",
                            "category": "genre",
                            "name": "Thriller",
                            "confidence": 1.0
                        }
                    ],
                    "aliases": ["Testing Show", "QA Chronicles"],
                    "tagged_aliases": [],
                    "seasons": [
                        {
                            "tvdb_id": 1000001,
                            "number": 1,
                            "label": "Season 1",
                            "episode_type": "default"
                        }
                    ],
                    "episodes": [
                        {
                            "tvdb_id": 2000001,
                            "episode_number": 1,
                            "season_number": 1,
                            "name": "Pilot",
                            "aired": "2023-09-15",
                            "runtime_minutes": 60,
                            "is_filler": false,
                            "is_recap": false,
                            "language": "eng",
                            "overview": "The team assembles.",
                            "absolute_number": "1"
                        }
                    ],
                    "anime_mappings": [],
                    "anime_movies": []
                }]
            }
        }
    })
    .to_string();

    Mock::given(method("GET"))
        .and(path("/graphql"))
        .and(is_search_titles_batch_request)
        .respond_with(search_titles_batch_response)
        .with_priority(2)
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(is_search_titles_batch_request)
        .respond_with(search_titles_batch_response)
        .with_priority(2)
        .mount(server)
        .await;

    Mock::given(method("GET"))
        .and(path("/graphql"))
        .and(is_search_tvdb_batch_request)
        .respond_with(search_tvdb_batch_response)
        .with_priority(2)
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(is_search_tvdb_batch_request)
        .respond_with(search_tvdb_batch_response)
        .with_priority(2)
        .mount(server)
        .await;

    Mock::given(method("GET"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture.clone()))
        .with_priority(100)
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
        .with_priority(100)
        .mount(server)
        .await;
}

fn is_search_tvdb_batch_request(request: &Request) -> bool {
    is_batch_search_request(request, "SearchTvdbBatch", "searchTvdbBatch")
        && search_tvdb_batch_inputs(request).is_some()
}

fn is_search_titles_batch_request(request: &Request) -> bool {
    is_batch_search_request(request, "SearchTitlesBatch", "searchTitlesBatch")
        && search_tvdb_batch_inputs(request).is_some()
}

fn is_batch_search_request(request: &Request, operation_name: &str, field_name: &str) -> bool {
    let body_matches = request.body_json::<Value>().ok().is_some_and(|body| {
        body.get("operationName")
            .and_then(Value::as_str)
            .is_some_and(|operation| operation == operation_name)
            || body
                .get("query")
                .and_then(Value::as_str)
                .is_some_and(|query| query.contains(operation_name) || query.contains(field_name))
    });
    let query_matches = request.url.query_pairs().any(|(key, value)| {
        (key == "operationName" && value == operation_name)
            || (key == "query" && (value.contains(operation_name) || value.contains(field_name)))
    });

    body_matches || query_matches
}

fn search_tvdb_batch_response(request: &Request) -> ResponseTemplate {
    let inputs = search_tvdb_batch_inputs(request).unwrap_or_default();
    let mut query_counts = HashMap::new();
    for input in &inputs {
        if let (Some(query), Some(type_hint)) = (
            input.get("query").and_then(Value::as_str),
            input.get("type").and_then(Value::as_str),
        ) {
            let year = input.get("year").and_then(Value::as_i64);
            *query_counts
                .entry(search_tvdb_query_key(type_hint, query, year))
                .or_insert(0) += 1;
        }
    }

    let batch = inputs
        .iter()
        .map(|input| {
            let query = input
                .get("query")
                .and_then(Value::as_str)
                .unwrap_or("Test Title");
            let type_hint = input
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("series");
            let year = input.get("year").and_then(Value::as_i64);
            let tvdb_id = input
                .get("tvdbId")
                .and_then(json_i64)
                .unwrap_or_else(|| stable_search_tvdb_id(type_hint, query, year));
            let key = search_tvdb_query_key(type_hint, query, year);
            let name = if query_counts.get(&key).copied().unwrap_or_default() > 1 {
                format!("{query} {tvdb_id}")
            } else {
                query.to_string()
            };
            let mut signals = vec!["exact_title".to_string()];
            if year.is_some() {
                signals.push("exact_year".to_string());
            }
            if input.get("tvdbId").and_then(json_i64).is_some() {
                signals.push("external_id:tvdb".to_string());
            }
            if input.get("imdbId").and_then(Value::as_str).is_some() {
                signals.push("external_id:imdb".to_string());
            }
            if input.get("tmdbId").and_then(Value::as_str).is_some() {
                signals.push("external_id:tmdb".to_string());
            }

            json!({
                "query": query,
                "type": type_hint,
                "year": year,
                "results": [{
                    "tvdb_id": tvdb_id,
                    "name": name,
                    "year": year,
                    "auto_match_safe": true,
                    "auto_match_signals": signals
                }]
            })
        })
        .collect::<Vec<_>>();

    ResponseTemplate::new(200).set_body_json(json!({
        "data": {
            "searchTvdbBatch": batch
        }
    }))
}

fn search_titles_batch_response(request: &Request) -> ResponseTemplate {
    let inputs = search_tvdb_batch_inputs(request).unwrap_or_default();
    let mut query_counts = HashMap::new();
    for input in &inputs {
        if let (Some(query), Some(type_hint)) = (
            input.get("query").and_then(Value::as_str),
            input.get("type").and_then(Value::as_str),
        ) {
            let year = input.get("year").and_then(Value::as_i64);
            *query_counts
                .entry(search_tvdb_query_key(type_hint, query, year))
                .or_insert(0) += 1;
        }
    }

    let batch = inputs
        .iter()
        .map(|input| {
            let query = input
                .get("query")
                .and_then(Value::as_str)
                .unwrap_or("Test Title");
            let type_hint = input.get("type").and_then(Value::as_str).unwrap_or("movie");
            let year = input.get("year").and_then(Value::as_i64);
            let tvdb_id = input
                .get("tvdbId")
                .and_then(json_i64)
                .unwrap_or_else(|| stable_search_tvdb_id(type_hint, query, year));
            let title_id = stable_search_title_id(tvdb_id);
            let key = search_tvdb_query_key(type_hint, query, year);
            let name = if query_counts.get(&key).copied().unwrap_or_default() > 1 {
                format!("{query} {tvdb_id}")
            } else {
                query.to_string()
            };
            let mut signals = vec!["exact_title".to_string()];
            if year.is_some() {
                signals.push("exact_year".to_string());
            }
            if input.get("tvdbId").and_then(json_i64).is_some() {
                signals.push("external_id:tvdb".to_string());
            }
            if input.get("imdbId").and_then(Value::as_str).is_some() {
                signals.push("external_id:imdb".to_string());
            }
            if input.get("tmdbId").and_then(Value::as_str).is_some() {
                signals.push("external_id:tmdb".to_string());
            }

            json!({
                "query": query,
                "type": type_hint,
                "year": year,
                "limit": 10,
                "total_results": 1,
                "results": [{
                    "title_id": title_id,
                    "kind": "movie",
                    "primary_source": "tvdb",
                    "tvdb_id": tvdb_id,
                    "tmdb_id": tvdb_id,
                    "imdb_id": format!("tt{tvdb_id:07}"),
                    "name": name,
                    "year": year,
                    "external_ids": [
                        {
                            "source": "smg",
                            "kind": "title",
                            "id": title_id.to_string(),
                            "key": format!("smg:title:{title_id}")
                        },
                        {
                            "source": "tvdb",
                            "kind": "movie",
                            "id": tvdb_id.to_string(),
                            "key": format!("tvdb:movie:{tvdb_id}")
                        }
                    ],
                    "auto_match_safe": true,
                    "auto_match_signals": signals,
                    "created": false
                }]
            })
        })
        .collect::<Vec<_>>();

    ResponseTemplate::new(200).set_body_json(json!({
        "data": {
            "searchTitlesBatch": batch
        }
    }))
}

fn search_tvdb_batch_inputs(request: &Request) -> Option<Vec<Value>> {
    let variables = if let Ok(payload) = request.body_json::<Value>() {
        payload.get("variables").cloned()
    } else {
        request.url.query_pairs().find_map(|(key, value)| {
            (key == "variables")
                .then(|| serde_json::from_str::<Value>(&value).ok())
                .flatten()
        })
    }?;

    let language = variables.get("language").and_then(Value::as_str)?;
    if language.trim().is_empty() {
        return None;
    }
    let requests = variables.get("requests")?.as_array()?;
    if requests.is_empty()
        || !requests.iter().all(|input| {
            input.get("query").and_then(Value::as_str).is_some()
                && input.get("type").and_then(Value::as_str).is_some()
        })
    {
        return None;
    }

    Some(requests.clone())
}

fn search_tvdb_query_key(type_hint: &str, query: &str, year: Option<i64>) -> String {
    format!("{type_hint}\0{query}\0{}", year.unwrap_or_default())
}

fn stable_search_tvdb_id(type_hint: &str, query: &str, year: Option<i64>) -> i64 {
    let key = format!("{type_hint}\0{query}\0{}", year.unwrap_or_default());
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in key.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    let base = if type_hint.eq_ignore_ascii_case("movie") {
        800_000_000_i64
    } else {
        900_000_000_i64
    };
    base + i64::try_from(hash % 100_000_000).unwrap_or(0)
}

fn stable_search_title_id(tvdb_id: i64) -> i64 {
    // The shared movie metadata fixture identifies TVDB 123456 as SMG title 101.
    // Other echo results remain stable and distinct by their TVDB identifier.
    if tvdb_id == 123_456 { 101 } else { tvdb_id }
}

fn json_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(|value| value.parse::<i64>().ok()))
}

/// Load a fixture file relative to the workspace `tests/fixtures/` directory.
pub fn load_fixture(path: &str) -> String {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let fixture_path = std::path::Path::new(manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests")
        .join("fixtures")
        .join(path);
    std::fs::read_to_string(&fixture_path)
        .unwrap_or_else(|e| panic!("failed to load fixture {}: {e}", fixture_path.display()))
}

/// Build a minimal axum router with a GraphQL endpoint and authentication disabled.
fn build_test_router(
    app: AppUseCase,
    schema: ApiSchema,
    auth_runtime: AuthRuntimeStateHandle,
) -> Router {
    Router::new().route(
        "/graphql",
        post(test_graphql_handler).with_state((app, schema, auth_runtime)),
    )
}

/// Minimal GraphQL handler that replicates default-user auth injection.
async fn test_graphql_handler(
    State((app, schema, auth_runtime)): State<(AppUseCase, ApiSchema, AuthRuntimeStateHandle)>,
    headers: HeaderMap,
    req: GraphQLRequest,
) -> Response {
    let snapshot = auth_runtime.snapshot();
    let actor = if let Some(token) = authorization_token_from_headers(&headers) {
        match app.authenticate_token_with_claims(token).await {
            Ok((_user, claims))
                if snapshot.effective_form_login_enabled
                    && claims.oauth_authorization_source == OAuthAuthorizationSource::Authless =>
            {
                None
            }
            Ok((user, claims)) => Some((user, claims, true)),
            Err(_) if !snapshot.effective_form_login_enabled => app
                .find_or_create_default_user()
                .await
                .ok()
                .map(|user| (user, AuthenticatedTokenClaims::default(), false)),
            Err(_) if snapshot.skip_login_for_local_ips => {
                app.find_or_create_default_user().await.ok().map(|user| {
                    (
                        user,
                        AuthenticatedTokenClaims {
                            mfa_verified_until: Some(i64::MAX),
                            mfa_step_up_verified_until: Some(i64::MAX),
                            ..AuthenticatedTokenClaims::default()
                        },
                        false,
                    )
                })
            }
            Err(_) => None,
        }
    } else if snapshot.effective_form_login_enabled {
        if snapshot.skip_login_for_local_ips {
            app.find_or_create_default_user().await.ok().map(|user| {
                (
                    user,
                    AuthenticatedTokenClaims {
                        mfa_verified_until: Some(i64::MAX),
                        mfa_step_up_verified_until: Some(i64::MAX),
                        ..AuthenticatedTokenClaims::default()
                    },
                    false,
                )
            })
        } else {
            None
        }
    } else {
        app.find_or_create_default_user()
            .await
            .ok()
            .map(|user| (user, AuthenticatedTokenClaims::default(), false))
    };
    let mut request = req.into_inner();
    let response_status = graphql_response_status(&mut request);
    if let Some((user, claims, authenticated_token)) = actor {
        request = request.data(MfaVerification {
            verified_until: claims.mfa_verified_until,
            step_up_verified_until: claims.mfa_step_up_verified_until,
            security_action_verified_until: claims.security_action_verified_until,
            session_scope: claims.session_scope,
            persist_session: claims.persist_session,
            auth_session_version: claims.auth_session_version.clone(),
            password_change_required_after_enrollment: claims
                .password_change_required_after_enrollment,
            oauth_authorization_source: claims.oauth_authorization_source,
        });
        let mut user = app
            .attach_user_authorization(user.clone())
            .await
            .unwrap_or(user);
        user.authorization.actor_capabilities = if authenticated_token {
            claims.actor_capabilities
        } else {
            scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT
        };
        if claims.is_oauth_access_token() {
            if claims.oauth_authorization_source == OAuthAuthorizationSource::Authless {
                user.username = "Anonymous".to_string();
            }
            user.authorization.app = scryer_domain::AppPermissionMask::NONE;
            user.authorization.actor_capabilities = scryer_domain::ActorCapabilityMask::NONE;
        }
        request = request.data(user);
    }
    let graphql_response = schema.execute(request).await;
    if app
        .image_proxy_repository()
        .flush_image_proxy_sources()
        .await
        .is_err()
    {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    let mut response = async_graphql_axum::GraphQLResponse::from(graphql_response).into_response();
    *response.status_mut() = response_status;
    response
}

fn authorization_token_from_headers(headers: &HeaderMap) -> Option<&str> {
    let raw = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let mut parts = raw.split_whitespace();
    let scheme = parts.next()?;
    let token = parts.next()?;
    if parts.next().is_some() || !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    Some(token)
}

fn graphql_response_status(request: &mut async_graphql::Request) -> StatusCode {
    let _ = request;
    StatusCode::OK
}

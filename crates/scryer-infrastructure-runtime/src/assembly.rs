use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
#[cfg(test)]
use scryer_application::DiscoveryRepository;
use scryer_application::{
    AppError, AppResult, AppServices, AppServicesBuilder, DownloadClient,
    DownloadClientConfigRepository, ImageProxyRepository, IndexerClient, IndexerConfigRepository,
    IndexerErrorRepository, IndexerSearchLearningRepository, IndexerStatsTracker,
    LibraryRepository, LogicalBackupExporter, MediaRequestRepository,
    MediaServerConnectionRepository, OAuthRepository, PluginInstallationRepository,
    PostProcessingScriptRepository, QualityProfileRepository, RuleSetRepository,
    ScopeIndexerCoverageRepository, SettingsRepository, ShowRepository,
    SubtitleProviderConfigRepository, TitleImageProcessor, TitleImageRepository, TitleRepository,
    TotpRepository, UpstreamScheduler, UserExternalAccountRepository, UserRepository,
    UserUiSettingsRepository, WebauthnRepository,
};

#[cfg(feature = "image-processing")]
use crate::HttpTitleImageProcessor;
use crate::discovery::store::DiscoveryStore;
use crate::external_identity::HttpExternalIdentityVerifier;
use crate::indexers::scope_indexer_coverage_store::ScopeIndexerCoverageStore;
use crate::media::images::image_proxy_store::ImageProxyStore;
use crate::media_server_playback::HttpMediaServerPlaybackProbe;
use crate::media_server_signals::HttpMediaServerSignalSource;
use crate::postgres::{
    PostgresLogicalBackupExporter, PostgresServices, restore_backup_bundle_into_postgres_pool,
    restore_prepared_backup_directory_into_postgres_pool,
};
use crate::queries::sql_runtime::StoreDatastore;
use crate::{
    AcquisitionStore, BlocklistStore, DomainEventStore, DownloadClientConfigStore,
    DownloadQueueCommandStore, DownloadRegistryStore, DownloadSubmissionStore,
    ExternalImportMonitorStore, ExternalImportSetupSecretDraftStore, FileSystemStagedNzbStore,
    HousekeepingStore, ImportStore, InMemoryIndexerStatsTracker, IndexerConfigStore,
    IndexerErrorStore, IndexerProxyConfigStore, IndexerSearchLearningStore, LibraryProbeStore,
    LibraryScanUnmatchedStore, LocationOperationStore, MaintenanceEvaluationStore,
    MaintenanceRuleSetStore, MediaFileStore, MediaRequestStore, MediaServerConnectionStore,
    MediaServerSignalStore, MetadataGatewayClient, MigrationMode, NotificationStore, OAuthStore,
    PendingReleaseStore, PluginStore, PostProcessingScriptStore, QualityProfileStore, ReleaseStore,
    RuleSetStore,
    SeedingProfileStore, SettingsStore, ShowStore, SmgEnrollmentConfig,
    SqliteLogicalBackupExporter, SqliteServices, SubtitleDownloadStore,
    SubtitleProviderConfigStore, TitleImageStore, TitleMergeStore, TitleStore, TotpStore,
    WantedStore, WebauthnStore, WorkflowOperationStore,
};
use crate::{LibraryStore, UserStore};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DatastoreEngine {
    Sqlite,
    Postgres,
}

#[cfg(feature = "image-processing")]
fn title_image_processor() -> Arc<dyn TitleImageProcessor> {
    Arc::new(HttpTitleImageProcessor::new())
}

#[cfg(not(feature = "image-processing"))]
fn title_image_processor() -> Arc<dyn TitleImageProcessor> {
    Arc::new(scryer_application::NullTitleImageProcessor)
}

impl DatastoreEngine {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sqlite => "sqlite",
            Self::Postgres => "postgres",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DatastoreConfigSource {
    EnvDbUrl,
    EnvDbPath,
    DefaultSqlite,
}

impl DatastoreConfigSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::EnvDbUrl => "SCRYER_DB_URL",
            Self::EnvDbPath => "SCRYER_DB_PATH",
            Self::DefaultSqlite => "default_sqlite",
        }
    }
}

#[derive(Clone, Debug)]
pub struct DatastoreConfig {
    pub engine: DatastoreEngine,
    pub database_url: String,
    pub redacted_database_url: String,
    pub source: DatastoreConfigSource,
    pub data_dir: PathBuf,
    pub migration_mode: MigrationMode,
}

impl DatastoreConfig {
    pub fn sqlite(
        database_url: impl Into<String>,
        data_dir: impl Into<PathBuf>,
        migration_mode: MigrationMode,
    ) -> Self {
        Self::sqlite_with_source(
            database_url,
            DatastoreConfigSource::EnvDbPath,
            data_dir,
            migration_mode,
        )
    }

    pub fn sqlite_with_source(
        database_url: impl Into<String>,
        source: DatastoreConfigSource,
        data_dir: impl Into<PathBuf>,
        migration_mode: MigrationMode,
    ) -> Self {
        let database_url = database_url.into();
        Self {
            engine: DatastoreEngine::Sqlite,
            redacted_database_url: database_url.clone(),
            database_url,
            source,
            data_dir: data_dir.into(),
            migration_mode,
        }
    }

    pub fn postgres(
        database_url: impl Into<String>,
        redacted_database_url: impl Into<String>,
        source: DatastoreConfigSource,
        data_dir: impl Into<PathBuf>,
        migration_mode: MigrationMode,
    ) -> Self {
        Self {
            engine: DatastoreEngine::Postgres,
            database_url: database_url.into(),
            redacted_database_url: redacted_database_url.into(),
            source,
            data_dir: data_dir.into(),
            migration_mode,
        }
    }

    pub fn backup_dir(&self) -> PathBuf {
        self.data_dir.join("backups")
    }

    pub fn safe_database_url(&self) -> &str {
        if self.redacted_database_url.is_empty() {
            &self.database_url
        } else {
            &self.redacted_database_url
        }
    }
}

pub fn resolve_datastore_config_from_env(
    data_dir: impl Into<PathBuf>,
    migration_mode: MigrationMode,
) -> AppResult<DatastoreConfig> {
    let data_dir = data_dir.into();
    if let Some(raw_url) = env_string("SCRYER_DB_URL") {
        return datastore_config_from_url(
            raw_url,
            DatastoreConfigSource::EnvDbUrl,
            data_dir,
            migration_mode,
        );
    }

    if let Some(db_path) = env_string("SCRYER_DB_PATH") {
        return Ok(DatastoreConfig::sqlite_with_source(
            db_path,
            DatastoreConfigSource::EnvDbPath,
            data_dir,
            migration_mode,
        ));
    }

    Ok(DatastoreConfig::sqlite_with_source(
        format!("sqlite://{}", data_dir.join("scryer.db").display()),
        DatastoreConfigSource::DefaultSqlite,
        data_dir,
        migration_mode,
    ))
}

fn datastore_config_from_url(
    raw_url: String,
    source: DatastoreConfigSource,
    data_dir: PathBuf,
    migration_mode: MigrationMode,
) -> AppResult<DatastoreConfig> {
    let parsed = url::Url::parse(&raw_url)
        .map_err(|error| AppError::Validation(format!("invalid SCRYER_DB_URL: {error}")))?;
    match parsed.scheme() {
        "sqlite" => Ok(DatastoreConfig::sqlite_with_source(
            raw_url,
            source,
            data_dir,
            migration_mode,
        )),
        "postgres" | "postgresql" => {
            let (database_url, redacted_url) = resolve_postgres_url(parsed)?;
            Ok(DatastoreConfig::postgres(
                database_url,
                redacted_url,
                source,
                data_dir,
                migration_mode,
            ))
        }
        scheme => Err(AppError::Validation(format!(
            "unsupported datastore URL scheme '{scheme}'; expected sqlite, postgres, or postgresql"
        ))),
    }
}

fn resolve_postgres_url(mut url: url::Url) -> AppResult<(String, String)> {
    if url.host_str().is_none_or(|host| host.trim().is_empty()) {
        return Err(AppError::Validation(
            "PostgreSQL datastore URL must include a host".to_string(),
        ));
    }

    let database_name = url.path().trim_start_matches('/').trim();
    if database_name.is_empty() {
        return Err(AppError::Validation(
            "PostgreSQL datastore URL must include a database name".to_string(),
        ));
    }

    let sslmode = url
        .query_pairs()
        .find(|(key, _)| key == "sslmode")
        .map(|(_, value)| value.to_string());
    let Some(sslmode) = sslmode else {
        return Err(AppError::Validation(
            "PostgreSQL datastore URL must include an explicit sslmode".to_string(),
        ));
    };
    if !matches!(
        sslmode.as_str(),
        "disable" | "prefer" | "require" | "verify-ca" | "verify-full"
    ) {
        return Err(AppError::Validation(format!(
            "unsupported PostgreSQL sslmode '{sslmode}'; expected disable, prefer, require, verify-ca, or verify-full"
        )));
    }

    let username = env_string("SCRYER_DB_USER")
        .or_else(|| {
            let username = url.username().trim();
            if username.is_empty() {
                None
            } else {
                Some(username.to_string())
            }
        })
        .ok_or_else(|| {
            AppError::Validation(
                "PostgreSQL datastore requires SCRYER_DB_USER or a URL username".to_string(),
            )
        })?;

    let password = postgres_password(&url)?;

    url.set_username(&username).map_err(|_| {
        AppError::Validation("failed to set PostgreSQL username on datastore URL".to_string())
    })?;
    url.set_password(Some(&password)).map_err(|_| {
        AppError::Validation("failed to set PostgreSQL password on datastore URL".to_string())
    })?;

    let mut redacted = url.clone();
    let _ = redacted.set_username("<redacted>");
    let _ = redacted.set_password(Some("<redacted>"));

    Ok((url.to_string(), redacted.to_string()))
}

fn postgres_password(url: &url::Url) -> AppResult<String> {
    if let Some(password_file) = env_string("SCRYER_DB_PASSWORD_FILE") {
        let password = std::fs::read_to_string(&password_file).map_err(|error| {
            AppError::Validation(format!(
                "failed to read SCRYER_DB_PASSWORD_FILE {}: {error}",
                password_file
            ))
        })?;
        let password = password.trim_end().to_string();
        if password.is_empty() {
            return Err(AppError::Validation(
                "SCRYER_DB_PASSWORD_FILE did not contain a password".to_string(),
            ));
        }
        return Ok(password);
    }

    if let Some(password) = env_string_raw("SCRYER_DB_PASSWORD") {
        return Ok(password);
    }

    url.password()
        .map(str::to_string)
        .filter(|password| !password.trim().is_empty())
        .ok_or_else(|| {
            AppError::Validation(
                "PostgreSQL datastore requires SCRYER_DB_PASSWORD, SCRYER_DB_PASSWORD_FILE, or a URL password"
                    .to_string(),
            )
        })
}

fn env_string(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_string_raw(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

#[derive(Clone)]
pub struct DatastoreCustomizationStore {
    rule_sets: Arc<RuleSetStore>,
    post_processing_scripts: Arc<PostProcessingScriptStore>,
    plugins: Arc<PluginStore>,
}

impl DatastoreCustomizationStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self::from_stores(
            Arc::new(RuleSetStore::new(datastore.clone())),
            Arc::new(PostProcessingScriptStore::new(datastore.clone())),
            Arc::new(PluginStore::new(datastore)),
        )
    }

    fn from_stores(
        rule_sets: Arc<RuleSetStore>,
        post_processing_scripts: Arc<PostProcessingScriptStore>,
        plugins: Arc<PluginStore>,
    ) -> Self {
        Self {
            rule_sets,
            post_processing_scripts,
            plugins,
        }
    }

    pub async fn delete_incompatible_external_plugin_installations(
        &self,
        preserve_restored_recovery_targets: bool,
    ) -> AppResult<Vec<String>> {
        self.plugins
            .delete_incompatible_external_plugin_installations(preserve_restored_recovery_targets)
            .await
    }
}

#[async_trait]
impl RuleSetRepository for DatastoreCustomizationStore {
    async fn list_rule_sets(&self) -> AppResult<Vec<scryer_domain::RuleSet>> {
        self.rule_sets.list_rule_sets().await
    }

    async fn list_enabled_rule_sets(&self) -> AppResult<Vec<scryer_domain::RuleSet>> {
        self.rule_sets.list_enabled_rule_sets().await
    }

    async fn get_rule_set(&self, id: &str) -> AppResult<Option<scryer_domain::RuleSet>> {
        self.rule_sets.get_rule_set(id).await
    }

    async fn create_rule_set(&self, rule_set: &scryer_domain::RuleSet) -> AppResult<()> {
        self.rule_sets.create_rule_set(rule_set).await
    }

    async fn update_rule_set(&self, rule_set: &scryer_domain::RuleSet) -> AppResult<()> {
        self.rule_sets.update_rule_set(rule_set).await
    }

    async fn delete_rule_set(&self, id: &str) -> AppResult<()> {
        self.rule_sets.delete_rule_set(id).await
    }

    async fn record_rule_set_history(
        &self,
        rule_set_id: &str,
        action: &str,
        rego_source: Option<&str>,
        actor_id: Option<&str>,
    ) -> AppResult<()> {
        self.rule_sets
            .record_rule_set_history(rule_set_id, action, rego_source, actor_id)
            .await
    }

    async fn get_rule_set_by_managed_key(
        &self,
        key: &str,
    ) -> AppResult<Option<scryer_domain::RuleSet>> {
        self.rule_sets.get_rule_set_by_managed_key(key).await
    }

    async fn delete_rule_set_by_managed_key(&self, key: &str) -> AppResult<()> {
        self.rule_sets.delete_rule_set_by_managed_key(key).await
    }

    async fn list_rule_sets_by_managed_key_prefix(
        &self,
        prefix: &str,
    ) -> AppResult<Vec<scryer_domain::RuleSet>> {
        self.rule_sets
            .list_rule_sets_by_managed_key_prefix(prefix)
            .await
    }
}

#[async_trait]
impl PostProcessingScriptRepository for DatastoreCustomizationStore {
    async fn list_scripts(&self) -> AppResult<Vec<scryer_domain::PostProcessingScript>> {
        self.post_processing_scripts.list_scripts().await
    }

    async fn get_script(&self, id: &str) -> AppResult<Option<scryer_domain::PostProcessingScript>> {
        self.post_processing_scripts.get_script(id).await
    }

    async fn create_script(
        &self,
        script: scryer_domain::PostProcessingScript,
    ) -> AppResult<scryer_domain::PostProcessingScript> {
        self.post_processing_scripts.create_script(script).await
    }

    async fn update_script(
        &self,
        script: scryer_domain::PostProcessingScript,
    ) -> AppResult<scryer_domain::PostProcessingScript> {
        self.post_processing_scripts.update_script(script).await
    }

    async fn delete_script(&self, id: &str) -> AppResult<()> {
        self.post_processing_scripts.delete_script(id).await
    }

    async fn list_enabled_for_facet(
        &self,
        facet: &str,
    ) -> AppResult<Vec<scryer_domain::PostProcessingScript>> {
        self.post_processing_scripts
            .list_enabled_for_facet(facet)
            .await
    }

    async fn record_run(&self, run: scryer_domain::PostProcessingScriptRun) -> AppResult<()> {
        self.post_processing_scripts.record_run(run).await
    }

    async fn list_runs_for_script(
        &self,
        script_id: &str,
        limit: usize,
    ) -> AppResult<Vec<scryer_domain::PostProcessingScriptRun>> {
        self.post_processing_scripts
            .list_runs_for_script(script_id, limit)
            .await
    }

    async fn list_runs_for_title(
        &self,
        title_id: &str,
        limit: usize,
    ) -> AppResult<Vec<scryer_domain::PostProcessingScriptRun>> {
        self.post_processing_scripts
            .list_runs_for_title(title_id, limit)
            .await
    }
}

#[async_trait]
impl PluginInstallationRepository for DatastoreCustomizationStore {
    async fn list_plugin_installations(&self) -> AppResult<Vec<scryer_domain::PluginInstallation>> {
        self.plugins.list_plugin_installations().await
    }

    async fn get_plugin_installation(
        &self,
        plugin_id: &str,
    ) -> AppResult<Option<scryer_domain::PluginInstallation>> {
        self.plugins.get_plugin_installation(plugin_id).await
    }

    async fn create_plugin_installation(
        &self,
        installation: &scryer_domain::PluginInstallation,
        wasm_bytes: Option<&[u8]>,
    ) -> AppResult<scryer_domain::PluginInstallation> {
        self.plugins
            .create_plugin_installation(installation, wasm_bytes)
            .await
    }

    async fn update_plugin_installation(
        &self,
        installation: &scryer_domain::PluginInstallation,
        wasm_bytes: Option<&[u8]>,
    ) -> AppResult<scryer_domain::PluginInstallation> {
        self.plugins
            .update_plugin_installation(installation, wasm_bytes)
            .await
    }

    async fn delete_plugin_installation(&self, plugin_id: &str) -> AppResult<()> {
        self.plugins.delete_plugin_installation(plugin_id).await
    }

    async fn get_enabled_plugin_wasm_bytes(
        &self,
    ) -> AppResult<
        Vec<(
            scryer_domain::PluginInstallation,
            Option<scryer_domain::PersistedPluginWasmPayload>,
        )>,
    > {
        self.plugins.get_enabled_plugin_wasm_bytes().await
    }

    async fn get_plugin_installation_wasm_payload(
        &self,
        plugin_id: &str,
    ) -> AppResult<Option<scryer_domain::PersistedPluginWasmPayload>> {
        self.plugins
            .get_plugin_installation_wasm_payload(plugin_id)
            .await
    }

    async fn seed_builtin(
        &self,
        plugin_id: &str,
        name: &str,
        description: &str,
        version: &str,
        sdk_version: &str,
        sdk_constraint: &str,
        plugin_type: &str,
        provider_type: &str,
    ) -> AppResult<()> {
        self.plugins
            .seed_builtin(
                plugin_id,
                name,
                description,
                version,
                sdk_version,
                sdk_constraint,
                plugin_type,
                provider_type,
            )
            .await
    }

    async fn upsert_plugin_catalog_source(
        &self,
        source: &scryer_domain::PluginCatalogSource,
    ) -> AppResult<()> {
        self.plugins.upsert_plugin_catalog_source(source).await
    }

    async fn delete_plugin_catalog_source(&self, source_key: &str) -> AppResult<()> {
        self.plugins.delete_plugin_catalog_source(source_key).await
    }

    async fn list_plugin_catalog_sources(
        &self,
    ) -> AppResult<Vec<scryer_domain::PluginCatalogSource>> {
        self.plugins.list_plugin_catalog_sources().await
    }

    async fn get_plugin_catalog_source(
        &self,
        source_key: &str,
    ) -> AppResult<Option<scryer_domain::PluginCatalogSource>> {
        self.plugins.get_plugin_catalog_source(source_key).await
    }

    async fn upsert_plugin_catalog_status(
        &self,
        status: &scryer_domain::PluginCatalogStatusRecord,
    ) -> AppResult<()> {
        self.plugins.upsert_plugin_catalog_status(status).await
    }

    async fn get_plugin_catalog_status(
        &self,
        status_key: &str,
    ) -> AppResult<Option<scryer_domain::PluginCatalogStatusRecord>> {
        self.plugins.get_plugin_catalog_status(status_key).await
    }
}

#[derive(Clone)]
pub struct DatastoreAssembly {
    config: DatastoreConfig,
    stores: DatastoreStores,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DatastoreEncryptionBootstrapReport {
    pub migrated_indexer_configs: u64,
    pub encrypted_release_attempt_source_passwords: u64,
    pub encrypted_pending_release_source_passwords: u64,
}

#[derive(Clone)]
enum DatastoreStores {
    Sqlite {
        db: SqliteServices,
        title_store: Arc<TitleStore>,
        show_store: Arc<ShowStore>,
        library_store: Arc<LibraryStore>,
        media_request_store: Arc<MediaRequestStore>,
        media_server_connection_store: Arc<MediaServerConnectionStore>,
        user_store: Arc<UserStore>,
        webauthn_store: Arc<WebauthnStore>,
        totp_store: Arc<TotpStore>,
        oauth_store: Arc<OAuthStore>,
        indexer_config_store: Arc<IndexerConfigStore>,
        indexer_error_store: Arc<IndexerErrorStore>,
        indexer_proxy_config_store: Arc<IndexerProxyConfigStore>,
        download_client_config_store: Arc<DownloadClientConfigStore>,
        seeding_profile_store: Arc<SeedingProfileStore>,
        subtitle_provider_config_store: Arc<SubtitleProviderConfigStore>,
        rule_set_store: Arc<RuleSetStore>,
        post_processing_script_store: Arc<PostProcessingScriptStore>,
        plugin_store: Arc<PluginStore>,
        library_probe_store: Arc<LibraryProbeStore>,
        library_scan_unmatched_store: Arc<LibraryScanUnmatchedStore>,
        location_operation_store: Arc<LocationOperationStore>,
        media_file_store: Arc<MediaFileStore>,
        wanted_store: Arc<WantedStore>,
        pending_release_store: Arc<PendingReleaseStore>,
        blocklist_store: Arc<BlocklistStore>,
        subtitle_download_store: Arc<SubtitleDownloadStore>,
        housekeeping_store: Arc<HousekeepingStore>,
        title_image_store: Arc<TitleImageStore>,
        image_proxy_store: Arc<ImageProxyStore>,
        notification_store: Arc<NotificationStore>,
        release_store: Arc<ReleaseStore>,
        settings_store: Arc<SettingsStore>,
        quality_profile_store: Arc<QualityProfileStore>,
        domain_event_store: Arc<DomainEventStore>,
        acquisition_store: Arc<AcquisitionStore>,
        download_registry_store: Arc<DownloadRegistryStore>,
        download_submission_store: Arc<DownloadSubmissionStore>,
        import_store: Arc<ImportStore>,
        external_import_monitor_store: Arc<ExternalImportMonitorStore>,
        external_import_setup_secret_draft_store: Arc<ExternalImportSetupSecretDraftStore>,
        download_queue_command_store: Arc<DownloadQueueCommandStore>,
        workflow_operation_store: Arc<WorkflowOperationStore>,
        discovery_store: Arc<DiscoveryStore>,
        backup_exporter: Arc<SqliteLogicalBackupExporter>,
    },
    Postgres {
        db: PostgresServices,
        title_store: Arc<TitleStore>,
        show_store: Arc<ShowStore>,
        library_store: Arc<LibraryStore>,
        media_request_store: Arc<MediaRequestStore>,
        media_server_connection_store: Arc<MediaServerConnectionStore>,
        user_store: Arc<UserStore>,
        webauthn_store: Arc<WebauthnStore>,
        totp_store: Arc<TotpStore>,
        oauth_store: Arc<OAuthStore>,
        indexer_config_store: Arc<IndexerConfigStore>,
        indexer_error_store: Arc<IndexerErrorStore>,
        indexer_proxy_config_store: Arc<IndexerProxyConfigStore>,
        download_client_config_store: Arc<DownloadClientConfigStore>,
        seeding_profile_store: Arc<SeedingProfileStore>,
        subtitle_provider_config_store: Arc<SubtitleProviderConfigStore>,
        rule_set_store: Arc<RuleSetStore>,
        post_processing_script_store: Arc<PostProcessingScriptStore>,
        plugin_store: Arc<PluginStore>,
        library_probe_store: Arc<LibraryProbeStore>,
        library_scan_unmatched_store: Arc<LibraryScanUnmatchedStore>,
        location_operation_store: Arc<LocationOperationStore>,
        media_file_store: Arc<MediaFileStore>,
        wanted_store: Arc<WantedStore>,
        pending_release_store: Arc<PendingReleaseStore>,
        blocklist_store: Arc<BlocklistStore>,
        subtitle_download_store: Arc<SubtitleDownloadStore>,
        housekeeping_store: Arc<HousekeepingStore>,
        title_image_store: Arc<TitleImageStore>,
        image_proxy_store: Arc<ImageProxyStore>,
        notification_store: Arc<NotificationStore>,
        release_store: Arc<ReleaseStore>,
        settings_store: Arc<SettingsStore>,
        quality_profile_store: Arc<QualityProfileStore>,
        domain_event_store: Arc<DomainEventStore>,
        acquisition_store: Arc<AcquisitionStore>,
        download_registry_store: Arc<DownloadRegistryStore>,
        download_submission_store: Arc<DownloadSubmissionStore>,
        import_store: Arc<ImportStore>,
        external_import_monitor_store: Arc<ExternalImportMonitorStore>,
        external_import_setup_secret_draft_store: Arc<ExternalImportSetupSecretDraftStore>,
        download_queue_command_store: Arc<DownloadQueueCommandStore>,
        workflow_operation_store: Arc<WorkflowOperationStore>,
        discovery_store: Arc<DiscoveryStore>,
        backup_exporter: Arc<PostgresLogicalBackupExporter>,
    },
}

impl DatastoreAssembly {
    pub async fn connect(config: DatastoreConfig) -> Result<Self, AppError> {
        match config.engine {
            DatastoreEngine::Sqlite => Self::connect_sqlite(config).await,
            DatastoreEngine::Postgres => Self::connect_postgres(config).await,
        }
    }

    async fn connect_sqlite(config: DatastoreConfig) -> Result<Self, AppError> {
        let db = SqliteServices::new_with_mode_and_data_dir(
            config.database_url.clone(),
            config.migration_mode,
            Some(config.data_dir.clone()),
        )
        .await?;
        let datastore = db.datastore();
        let title_store = Arc::new(TitleStore::new(datastore.clone()));
        let show_store = Arc::new(ShowStore::new(datastore.clone()));
        let library_store = Arc::new(LibraryStore::new(datastore.clone()));
        let media_request_store = Arc::new(MediaRequestStore::new(datastore.clone()));
        let media_server_connection_store = Arc::new(MediaServerConnectionStore::new(
            datastore.clone(),
            db.encryption_key_state(),
        ));
        let user_store = Arc::new(UserStore::new(datastore.clone()));
        let webauthn_store = Arc::new(WebauthnStore::new(datastore.clone()));
        let totp_store = Arc::new(TotpStore::new(datastore.clone(), db.encryption_key_state()));
        let oauth_store = Arc::new(OAuthStore::new(datastore.clone()));
        let indexer_config_store = Arc::new(IndexerConfigStore::new(
            datastore.clone(),
            db.encryption_key_state(),
        ));
        let indexer_error_store = Arc::new(IndexerErrorStore::new(datastore.clone()));
        let indexer_proxy_config_store = Arc::new(IndexerProxyConfigStore::new(datastore.clone()));
        let download_client_config_store = Arc::new(DownloadClientConfigStore::new(
            datastore.clone(),
            db.encryption_key_state(),
        ));
        let seeding_profile_store = Arc::new(SeedingProfileStore::new(datastore.clone()));
        let subtitle_provider_config_store = Arc::new(SubtitleProviderConfigStore::new(
            datastore.clone(),
            db.encryption_key_state(),
        ));
        let rule_set_store = Arc::new(RuleSetStore::new(datastore.clone()));
        let post_processing_script_store =
            Arc::new(PostProcessingScriptStore::new(datastore.clone()));
        let notification_store = Arc::new(NotificationStore::new(
            datastore.clone(),
            db.encryption_key_state(),
        ));
        let plugin_store = Arc::new(PluginStore::new(datastore.clone()));
        let library_probe_store = Arc::new(LibraryProbeStore::new(datastore.clone()));
        let library_scan_unmatched_store =
            Arc::new(LibraryScanUnmatchedStore::new(datastore.clone()));
        let location_operation_store = Arc::new(LocationOperationStore::new(datastore.clone()));
        let media_file_store = Arc::new(MediaFileStore::new(datastore.clone()));
        let wanted_store = Arc::new(WantedStore::new(datastore.clone()));
        let pending_release_store = Arc::new(PendingReleaseStore::new(
            datastore.clone(),
            db.encryption_key_state(),
        ));
        let blocklist_store = Arc::new(BlocklistStore::new(datastore.clone()));
        let subtitle_download_store = Arc::new(SubtitleDownloadStore::new(datastore.clone()));
        let housekeeping_store = Arc::new(HousekeepingStore::new(datastore.clone()));
        let title_image_store = Arc::new(TitleImageStore::new(datastore.clone()));
        let image_proxy_store = Arc::new(ImageProxyStore::new(datastore.clone()));
        let release_store = Arc::new(ReleaseStore::new(
            datastore.clone(),
            db.encryption_key_state(),
        ));
        let settings_store = Arc::new(SettingsStore::new(
            datastore.clone(),
            db.encryption_key_state(),
        ));
        let quality_profile_store = Arc::new(QualityProfileStore::new(datastore.clone()));
        let domain_event_store = Arc::new(DomainEventStore::new(datastore.clone()));
        let acquisition_store = Arc::new(AcquisitionStore::new(datastore.clone()));
        let download_registry_store = Arc::new(DownloadRegistryStore::new(datastore.clone()));
        let download_submission_store = Arc::new(DownloadSubmissionStore::new(datastore.clone()));
        let import_store = Arc::new(ImportStore::new(datastore.clone()));
        let external_import_monitor_store =
            Arc::new(ExternalImportMonitorStore::new(datastore.clone()));
        let external_import_setup_secret_draft_store = Arc::new(
            ExternalImportSetupSecretDraftStore::new(datastore.clone(), db.encryption_key_state()),
        );
        let download_queue_command_store =
            Arc::new(DownloadQueueCommandStore::new(datastore.clone()));
        let workflow_operation_store = Arc::new(WorkflowOperationStore::new(datastore.clone()));
        let discovery_store = Arc::new(DiscoveryStore::new(datastore.clone()));
        let backup_exporter = Arc::new(SqliteLogicalBackupExporter::new(
            config.database_url.clone(),
        ));

        let stores = DatastoreStores::Sqlite {
            db,
            title_store,
            show_store,
            library_store,
            media_request_store,
            media_server_connection_store,
            user_store,
            webauthn_store,
            totp_store,
            oauth_store,
            indexer_config_store,
            indexer_error_store,
            indexer_proxy_config_store,
            download_client_config_store,
            seeding_profile_store,
            subtitle_provider_config_store,
            rule_set_store,
            post_processing_script_store,
            plugin_store,
            library_probe_store,
            library_scan_unmatched_store,
            location_operation_store,
            media_file_store,
            wanted_store,
            pending_release_store,
            blocklist_store,
            subtitle_download_store,
            housekeeping_store,
            title_image_store,
            image_proxy_store,
            notification_store,
            release_store,
            settings_store,
            quality_profile_store,
            domain_event_store,
            acquisition_store,
            download_registry_store,
            download_submission_store,
            import_store,
            external_import_monitor_store,
            external_import_setup_secret_draft_store,
            download_queue_command_store,
            workflow_operation_store,
            discovery_store,
            backup_exporter,
        };

        Ok(Self { config, stores })
    }

    async fn connect_postgres(config: DatastoreConfig) -> Result<Self, AppError> {
        let db = PostgresServices::new_with_mode_and_data_dir(
            config.database_url.clone(),
            config.migration_mode,
            Some(config.data_dir.clone()),
        )
        .await?;
        let datastore = db.datastore();
        let title_store = Arc::new(TitleStore::new(datastore.clone()));
        let show_store = Arc::new(ShowStore::new(datastore.clone()));
        let library_store = Arc::new(LibraryStore::new(datastore.clone()));
        let media_request_store = Arc::new(MediaRequestStore::new(datastore.clone()));
        let media_server_connection_store = Arc::new(MediaServerConnectionStore::new(
            datastore.clone(),
            db.encryption_key_state(),
        ));
        let user_store = Arc::new(UserStore::new(datastore.clone()));
        let webauthn_store = Arc::new(WebauthnStore::new(datastore.clone()));
        let totp_store = Arc::new(TotpStore::new(datastore.clone(), db.encryption_key_state()));
        let oauth_store = Arc::new(OAuthStore::new(datastore.clone()));
        let indexer_config_store = Arc::new(IndexerConfigStore::new(
            datastore.clone(),
            db.encryption_key_state(),
        ));
        let indexer_error_store = Arc::new(IndexerErrorStore::new(datastore.clone()));
        let indexer_proxy_config_store = Arc::new(IndexerProxyConfigStore::new(datastore.clone()));
        let download_client_config_store = Arc::new(DownloadClientConfigStore::new(
            datastore.clone(),
            db.encryption_key_state(),
        ));
        let seeding_profile_store = Arc::new(SeedingProfileStore::new(datastore.clone()));
        let subtitle_provider_config_store = Arc::new(SubtitleProviderConfigStore::new(
            datastore.clone(),
            db.encryption_key_state(),
        ));
        let rule_set_store = Arc::new(RuleSetStore::new(datastore.clone()));
        let post_processing_script_store =
            Arc::new(PostProcessingScriptStore::new(datastore.clone()));
        let notification_store = Arc::new(NotificationStore::new(
            datastore.clone(),
            db.encryption_key_state(),
        ));
        let plugin_store = Arc::new(PluginStore::new(datastore.clone()));
        let library_probe_store = Arc::new(LibraryProbeStore::new(datastore.clone()));
        let library_scan_unmatched_store =
            Arc::new(LibraryScanUnmatchedStore::new(datastore.clone()));
        let location_operation_store = Arc::new(LocationOperationStore::new(datastore.clone()));
        let media_file_store = Arc::new(MediaFileStore::new(datastore.clone()));
        let wanted_store = Arc::new(WantedStore::new(datastore.clone()));
        let pending_release_store = Arc::new(PendingReleaseStore::new(
            datastore.clone(),
            db.encryption_key_state(),
        ));
        let blocklist_store = Arc::new(BlocklistStore::new(datastore.clone()));
        let subtitle_download_store = Arc::new(SubtitleDownloadStore::new(datastore.clone()));
        let housekeeping_store = Arc::new(HousekeepingStore::new(datastore.clone()));
        let title_image_store = Arc::new(TitleImageStore::new(datastore.clone()));
        let image_proxy_store = Arc::new(ImageProxyStore::new(datastore.clone()));
        let release_store = Arc::new(ReleaseStore::new(
            datastore.clone(),
            db.encryption_key_state(),
        ));
        let settings_store = Arc::new(SettingsStore::new(
            datastore.clone(),
            db.encryption_key_state(),
        ));
        let quality_profile_store = Arc::new(QualityProfileStore::new(datastore.clone()));
        let domain_event_store = Arc::new(DomainEventStore::new(datastore.clone()));
        let acquisition_store = Arc::new(AcquisitionStore::new(datastore.clone()));
        let download_registry_store = Arc::new(DownloadRegistryStore::new(datastore.clone()));
        let download_submission_store = Arc::new(DownloadSubmissionStore::new(datastore.clone()));
        let import_store = Arc::new(ImportStore::new(datastore.clone()));
        let external_import_monitor_store =
            Arc::new(ExternalImportMonitorStore::new(datastore.clone()));
        let external_import_setup_secret_draft_store = Arc::new(
            ExternalImportSetupSecretDraftStore::new(datastore.clone(), db.encryption_key_state()),
        );
        let download_queue_command_store =
            Arc::new(DownloadQueueCommandStore::new(datastore.clone()));
        let workflow_operation_store = Arc::new(WorkflowOperationStore::new(datastore.clone()));
        let discovery_store = Arc::new(DiscoveryStore::new(datastore.clone()));
        let backup_exporter = Arc::new(PostgresLogicalBackupExporter::new(&db));

        let stores = DatastoreStores::Postgres {
            db,
            title_store,
            show_store,
            library_store,
            media_request_store,
            media_server_connection_store,
            user_store,
            webauthn_store,
            totp_store,
            oauth_store,
            indexer_config_store,
            indexer_error_store,
            indexer_proxy_config_store,
            download_client_config_store,
            seeding_profile_store,
            subtitle_provider_config_store,
            rule_set_store,
            post_processing_script_store,
            plugin_store,
            library_probe_store,
            library_scan_unmatched_store,
            location_operation_store,
            media_file_store,
            wanted_store,
            pending_release_store,
            blocklist_store,
            subtitle_download_store,
            housekeeping_store,
            title_image_store,
            image_proxy_store,
            notification_store,
            release_store,
            settings_store,
            quality_profile_store,
            domain_event_store,
            acquisition_store,
            download_registry_store,
            download_submission_store,
            import_store,
            external_import_monitor_store,
            external_import_setup_secret_draft_store,
            download_queue_command_store,
            workflow_operation_store,
            discovery_store,
            backup_exporter,
        };

        Ok(Self { config, stores })
    }

    pub fn engine(&self) -> DatastoreEngine {
        self.config.engine
    }

    pub fn backup_dir(&self) -> PathBuf {
        self.config.backup_dir()
    }

    pub fn staged_nzb_path(&self) -> PathBuf {
        match self.config.engine {
            DatastoreEngine::Sqlite => {
                FileSystemStagedNzbStore::path_for_main_db(&self.config.database_url)
            }
            DatastoreEngine::Postgres => self.config.data_dir.join("staged-nzbs"),
        }
    }

    pub fn datastore(&self) -> StoreDatastore {
        match &self.stores {
            DatastoreStores::Sqlite { db, .. } => db.datastore(),
            DatastoreStores::Postgres { db, .. } => db.datastore(),
        }
    }

    /// Built on demand: maintenance rules ship dark, so the store is not part
    /// of the per-engine store set every assembly path constructs eagerly.
    pub fn maintenance_rule_set_store(&self) -> Arc<MaintenanceRuleSetStore> {
        Arc::new(MaintenanceRuleSetStore::new(self.datastore()))
    }

    /// Built on demand for the same reason: the evaluator ships behind an
    /// instance gate that defaults off, so its store is not part of the eager
    /// per-engine store set either.
    pub fn maintenance_evaluation_store(&self) -> Arc<MaintenanceEvaluationStore> {
        Arc::new(MaintenanceEvaluationStore::new(self.datastore()))
    }

    /// Media-server watch signals (RFC 137 §7.3, WP-M). Built on demand: the
    /// store is written only by the signal sync job, which does nothing at all
    /// until a media-server connection with verified linked accounts exists.
    pub fn media_server_signal_store(&self) -> Arc<MediaServerSignalStore> {
        Arc::new(MediaServerSignalStore::new(self.datastore()))
    }

    pub fn settings_store(&self) -> Arc<SettingsStore> {
        match &self.stores {
            DatastoreStores::Sqlite { settings_store, .. } => settings_store.clone(),
            DatastoreStores::Postgres { settings_store, .. } => settings_store.clone(),
        }
    }

    pub fn quality_profile_store(&self) -> Arc<QualityProfileStore> {
        match &self.stores {
            DatastoreStores::Sqlite {
                quality_profile_store,
                ..
            } => quality_profile_store.clone(),
            DatastoreStores::Postgres {
                quality_profile_store,
                ..
            } => quality_profile_store.clone(),
        }
    }

    pub fn customization_store(&self) -> DatastoreCustomizationStore {
        match &self.stores {
            DatastoreStores::Sqlite {
                rule_set_store,
                post_processing_script_store,
                plugin_store,
                ..
            } => DatastoreCustomizationStore::from_stores(
                rule_set_store.clone(),
                post_processing_script_store.clone(),
                plugin_store.clone(),
            ),
            DatastoreStores::Postgres {
                rule_set_store,
                post_processing_script_store,
                plugin_store,
                ..
            } => DatastoreCustomizationStore::from_stores(
                rule_set_store.clone(),
                post_processing_script_store.clone(),
                plugin_store.clone(),
            ),
        }
    }

    pub async fn bootstrap_encryption(&self) -> Result<DatastoreEncryptionBootstrapReport, String> {
        match &self.stores {
            DatastoreStores::Sqlite {
                db,
                indexer_config_store,
                pending_release_store,
                release_store,
                ..
            } => {
                let encryption_key = crate::encryption::ensure_encryption_key(
                    db,
                    Some(self.config.data_dir.clone()),
                )
                .await?;
                db.set_encryption_key(encryption_key)
                    .await
                    .map_err(|error| error.to_string())?;
                let migrated_indexer_configs = indexer_config_store
                    .migrate_legacy_indexer_config_sources()
                    .await
                    .map_err(|error| error.to_string())?;
                let encrypted_release_attempt_source_passwords = release_store
                    .backfill_source_passwords()
                    .await
                    .map_err(|error| error.to_string())?;
                let encrypted_pending_release_source_passwords = pending_release_store
                    .backfill_source_passwords()
                    .await
                    .map_err(|error| error.to_string())?;
                Ok(DatastoreEncryptionBootstrapReport {
                    migrated_indexer_configs,
                    encrypted_release_attempt_source_passwords,
                    encrypted_pending_release_source_passwords,
                })
            }
            DatastoreStores::Postgres {
                db,
                pending_release_store,
                release_store,
                ..
            } => {
                let encryption_key = crate::encryption::ensure_encryption_key_without_legacy(Some(
                    self.config.data_dir.clone(),
                ))
                .await?;
                db.set_encryption_key(encryption_key)
                    .await
                    .map_err(|error| error.to_string())?;
                let encrypted_release_attempt_source_passwords = release_store
                    .backfill_source_passwords()
                    .await
                    .map_err(|error| error.to_string())?;
                let encrypted_pending_release_source_passwords = pending_release_store
                    .backfill_source_passwords()
                    .await
                    .map_err(|error| error.to_string())?;
                Ok(DatastoreEncryptionBootstrapReport {
                    migrated_indexer_configs: 0,
                    encrypted_release_attempt_source_passwords,
                    encrypted_pending_release_source_passwords,
                })
            }
        }
    }

    pub fn indexer_configs(&self) -> Arc<dyn IndexerConfigRepository> {
        match &self.stores {
            DatastoreStores::Sqlite {
                indexer_config_store,
                ..
            } => indexer_config_store.clone(),
            DatastoreStores::Postgres {
                indexer_config_store,
                ..
            } => indexer_config_store.clone(),
        }
    }

    pub fn indexer_proxy_configs(
        &self,
    ) -> Arc<dyn scryer_application::IndexerProxyConfigRepository> {
        match &self.stores {
            DatastoreStores::Sqlite {
                indexer_proxy_config_store,
                ..
            } => indexer_proxy_config_store.clone(),
            DatastoreStores::Postgres {
                indexer_proxy_config_store,
                ..
            } => indexer_proxy_config_store.clone(),
        }
    }

    pub fn download_submissions(
        &self,
    ) -> Arc<dyn scryer_application::DownloadSubmissionRepository> {
        match &self.stores {
            DatastoreStores::Sqlite {
                download_submission_store,
                ..
            } => download_submission_store.clone(),
            DatastoreStores::Postgres {
                download_submission_store,
                ..
            } => download_submission_store.clone(),
        }
    }

    pub fn download_registry(&self) -> Arc<dyn scryer_application::DownloadRegistryRepository> {
        match &self.stores {
            DatastoreStores::Sqlite {
                download_registry_store,
                ..
            } => download_registry_store.clone(),
            DatastoreStores::Postgres {
                download_registry_store,
                ..
            } => download_registry_store.clone(),
        }
    }

    pub fn seeding_profiles(&self) -> Arc<dyn scryer_application::SeedingProfileRepository> {
        match &self.stores {
            DatastoreStores::Sqlite {
                seeding_profile_store,
                ..
            } => seeding_profile_store.clone(),
            DatastoreStores::Postgres {
                seeding_profile_store,
                ..
            } => seeding_profile_store.clone(),
        }
    }

    pub fn download_client_configs(&self) -> Arc<dyn DownloadClientConfigRepository> {
        match &self.stores {
            DatastoreStores::Sqlite {
                download_client_config_store,
                ..
            } => download_client_config_store.clone(),
            DatastoreStores::Postgres {
                download_client_config_store,
                ..
            } => download_client_config_store.clone(),
        }
    }

    pub fn media_server_connections(&self) -> Arc<dyn MediaServerConnectionRepository> {
        match &self.stores {
            DatastoreStores::Sqlite {
                media_server_connection_store,
                ..
            } => media_server_connection_store.clone(),
            DatastoreStores::Postgres {
                media_server_connection_store,
                ..
            } => media_server_connection_store.clone(),
        }
    }

    pub fn subtitle_provider_configs(&self) -> Arc<dyn SubtitleProviderConfigRepository> {
        match &self.stores {
            DatastoreStores::Sqlite {
                subtitle_provider_config_store,
                ..
            } => subtitle_provider_config_store.clone(),
            DatastoreStores::Postgres {
                subtitle_provider_config_store,
                ..
            } => subtitle_provider_config_store.clone(),
        }
    }

    pub fn settings(&self) -> Arc<dyn SettingsRepository> {
        match &self.stores {
            DatastoreStores::Sqlite { settings_store, .. } => settings_store.clone(),
            DatastoreStores::Postgres { settings_store, .. } => settings_store.clone(),
        }
    }

    pub fn quality_profiles(&self) -> Arc<dyn QualityProfileRepository> {
        match &self.stores {
            DatastoreStores::Sqlite {
                quality_profile_store,
                ..
            } => quality_profile_store.clone(),
            DatastoreStores::Postgres {
                quality_profile_store,
                ..
            } => quality_profile_store.clone(),
        }
    }

    pub fn title_images(&self) -> Arc<dyn TitleImageRepository> {
        match &self.stores {
            DatastoreStores::Sqlite {
                title_image_store, ..
            } => title_image_store.clone(),
            DatastoreStores::Postgres {
                title_image_store, ..
            } => title_image_store.clone(),
        }
    }

    pub fn image_proxy(&self) -> Arc<dyn ImageProxyRepository> {
        match &self.stores {
            DatastoreStores::Sqlite {
                image_proxy_store, ..
            } => image_proxy_store.clone(),
            DatastoreStores::Postgres {
                image_proxy_store, ..
            } => image_proxy_store.clone(),
        }
    }

    pub fn logical_backup_exporter(&self) -> Arc<dyn LogicalBackupExporter> {
        match &self.stores {
            DatastoreStores::Sqlite {
                backup_exporter, ..
            } => backup_exporter.clone(),
            DatastoreStores::Postgres {
                backup_exporter, ..
            } => backup_exporter.clone(),
        }
    }

    pub fn indexer_stats_tracker(&self) -> Arc<dyn IndexerStatsTracker> {
        match &self.stores {
            DatastoreStores::Sqlite { db, .. } => {
                Arc::new(InMemoryIndexerStatsTracker::new(Some(db.datastore())))
            }
            DatastoreStores::Postgres { .. } => Arc::new(InMemoryIndexerStatsTracker::new(None)),
        }
    }

    pub fn indexer_search_learning_repository(&self) -> Arc<dyn IndexerSearchLearningRepository> {
        match &self.stores {
            DatastoreStores::Sqlite { db, .. } => Arc::new(IndexerSearchLearningStore::new(
                db.datastore(),
                db.encryption_key_state(),
            )),
            DatastoreStores::Postgres { db, .. } => Arc::new(IndexerSearchLearningStore::new(
                db.datastore(),
                db.encryption_key_state(),
            )),
        }
    }

    pub fn indexer_errors(&self) -> Arc<dyn IndexerErrorRepository> {
        match &self.stores {
            DatastoreStores::Sqlite {
                indexer_error_store,
                ..
            } => indexer_error_store.clone(),
            DatastoreStores::Postgres {
                indexer_error_store,
                ..
            } => indexer_error_store.clone(),
        }
    }

    fn scope_indexer_coverage_repository(&self) -> Arc<dyn ScopeIndexerCoverageRepository> {
        match &self.stores {
            DatastoreStores::Sqlite { db, .. } => {
                Arc::new(ScopeIndexerCoverageStore::new(db.datastore()))
            }
            DatastoreStores::Postgres { db, .. } => {
                Arc::new(ScopeIndexerCoverageStore::new(db.datastore()))
            }
        }
    }

    pub async fn upstream_scheduler(&self) -> AppResult<Arc<dyn UpstreamScheduler>> {
        let scheduler = match &self.stores {
            DatastoreStores::Sqlite { db, .. } => {
                crate::upstream_scheduler::InMemoryUpstreamScheduler::new_persistent(db.datastore())
                    .await?
            }
            DatastoreStores::Postgres { db, .. } => {
                crate::upstream_scheduler::InMemoryUpstreamScheduler::new_persistent(db.datastore())
                    .await?
            }
        };
        Ok(Arc::new(scheduler))
    }

    pub fn metadata_gateway_client(
        &self,
        endpoint: String,
        enrollment_config: SmgEnrollmentConfig,
    ) -> MetadataGatewayClient {
        match &self.stores {
            DatastoreStores::Sqlite { settings_store, .. } => {
                MetadataGatewayClient::new_with_enrollment_store(
                    endpoint,
                    settings_store.clone(),
                    enrollment_config,
                )
            }
            DatastoreStores::Postgres { settings_store, .. } => {
                MetadataGatewayClient::new_with_enrollment_store(
                    endpoint,
                    settings_store.clone(),
                    enrollment_config,
                )
            }
        }
    }

    pub fn app_services_builder(
        &self,
        indexer_client: Arc<dyn IndexerClient>,
        download_client: Arc<dyn DownloadClient>,
    ) -> AppServicesBuilder {
        match &self.stores {
            DatastoreStores::Sqlite {
                title_store,
                show_store,
                library_store,
                media_request_store,
                media_server_connection_store,
                user_store,
                webauthn_store,
                totp_store,
                oauth_store,
                release_store,
                library_probe_store,
                library_scan_unmatched_store,
            location_operation_store,
                media_file_store,
                wanted_store,
                pending_release_store,
                blocklist_store,
                subtitle_download_store,
                housekeeping_store,
                title_image_store,
                image_proxy_store,
                rule_set_store,
                post_processing_script_store,
                plugin_store,
                domain_event_store,
                acquisition_store,
                download_registry_store,
                download_submission_store,
                import_store,
                external_import_monitor_store,
                external_import_setup_secret_draft_store,
                download_queue_command_store,
                workflow_operation_store,
                discovery_store,
                notification_store,
                settings_store,
                ..
            } => {
                let titles: Arc<dyn TitleRepository> = title_store.clone();
                let shows: Arc<dyn ShowRepository> = show_store.clone();
                let users: Arc<dyn UserRepository> = user_store.clone();
                let ui_settings: Arc<dyn UserUiSettingsRepository> = user_store.clone();
                let external_accounts: Arc<dyn UserExternalAccountRepository> = user_store.clone();
                let webauthn: Arc<dyn WebauthnRepository> = webauthn_store.clone();
                let totp: Arc<dyn TotpRepository> = totp_store.clone();
                let oauth: Arc<dyn OAuthRepository> = oauth_store.clone();
                let libraries: Arc<dyn LibraryRepository> = library_store.clone();
                let media_requests: Arc<dyn MediaRequestRepository> = media_request_store.clone();

                AppServices::builder(
                    titles,
                    shows,
                    users,
                    self.indexer_configs(),
                    indexer_client,
                    download_client,
                    self.download_client_configs(),
                    release_store.clone(),
                    self.settings(),
                    self.quality_profiles(),
                    self.backup_dir(),
                )
                .with_indexer_error_repository(self.indexer_errors())
                .with_libraries(libraries)
                .with_media_requests(media_requests)
                .with_user_ui_settings_store(ui_settings)
                .with_external_account_store(external_accounts)
                .with_oauth_store(oauth)
                .with_indexer_proxy_config_store(self.indexer_proxy_configs())
                .with_seeding_profiles(self.seeding_profiles())
                .with_external_identity_verifier(Arc::new(HttpExternalIdentityVerifier::new()))
                .with_media_server_connection_store(media_server_connection_store.clone())
                // Maintenance safety: live playback observation (RFC 137 §9.10, WP-G).
                .with_media_server_playback_probe(Arc::new(HttpMediaServerPlaybackProbe::new(
                    media_server_connection_store.clone(),
                )))
                // Media-server watch signals (RFC 137 §7.3, WP-M).
                .with_media_server_signal_source(Arc::new(HttpMediaServerSignalSource::new()))
                .with_media_server_signal_store(self.media_server_signal_store())
                .with_webauthn_store(webauthn)
                .with_totp_store(totp)
                .with_media_files(media_file_store.clone())
                .with_acquisition_scope_states(wanted_store.clone())
                .with_scope_indexer_coverage_store(self.scope_indexer_coverage_repository())
                .with_pending_releases(pending_release_store.clone())
                .with_blocklist_repo(blocklist_store.clone())
                .with_library_probe_signatures(library_probe_store.clone())
                .with_library_scan_unmatched_items(library_scan_unmatched_store.clone())
                .with_location_operation_repository(location_operation_store.clone())
                // The US7 merge store needs only the datastore, so it is
                // built here rather than threaded through both store
                // variants: nothing else in the assembly holds it.
                .with_title_merge_repository(Arc::new(TitleMergeStore::new(
                    self.datastore(),
                )))
                .with_title_images(title_image_store.clone())
                .with_image_proxy(image_proxy_store.clone())
                .with_housekeeping(housekeeping_store.clone())
                .with_subtitle_downloads(subtitle_download_store.clone())
                .with_rule_set_store(rule_set_store.clone())
                .with_maintenance_rule_set_store(self.maintenance_rule_set_store())
                .with_maintenance_evaluation_store(self.maintenance_evaluation_store())
                .with_post_processing_script_store(post_processing_script_store.clone())
                .with_plugin_installation_store(plugin_store.clone())
                .with_acquisition_state(acquisition_store.clone())
                .with_domain_events(domain_event_store.clone())
                .with_download_registry(download_registry_store.clone())
                .with_download_submissions(download_submission_store.clone())
                .with_download_queue_commands(download_queue_command_store.clone())
                .with_external_import_monitor_snapshots(external_import_monitor_store.clone())
                .with_external_import_setup_secret_drafts(
                    external_import_setup_secret_draft_store.clone(),
                )
                .with_import_artifacts(import_store.clone())
                .with_imports(import_store.clone())
                .with_job_runs(workflow_operation_store.clone())
                .with_discovery_store(discovery_store.clone())
                .with_notification_store(notification_store.clone())
                .with_system_info(settings_store.clone())
                .with_logical_backup_exporter(self.logical_backup_exporter())
                .with_title_image_processor(title_image_processor())
                .with_workflow_operations(workflow_operation_store.clone())
            }
            DatastoreStores::Postgres {
                title_store,
                show_store,
                library_store,
                media_request_store,
                media_server_connection_store,
                user_store,
                webauthn_store,
                totp_store,
                oauth_store,
                rule_set_store,
                post_processing_script_store,
                plugin_store,
                library_probe_store,
                library_scan_unmatched_store,
            location_operation_store,
                media_file_store,
                wanted_store,
                pending_release_store,
                blocklist_store,
                subtitle_download_store,
                housekeeping_store,
                title_image_store,
                image_proxy_store,
                notification_store,
                release_store,
                settings_store,
                domain_event_store,
                acquisition_store,
                download_registry_store,
                download_submission_store,
                import_store,
                external_import_monitor_store,
                external_import_setup_secret_draft_store,
                download_queue_command_store,
                workflow_operation_store,
                discovery_store,
                ..
            } => {
                let titles: Arc<dyn TitleRepository> = title_store.clone();
                let shows: Arc<dyn ShowRepository> = show_store.clone();
                let users: Arc<dyn UserRepository> = user_store.clone();
                let ui_settings: Arc<dyn UserUiSettingsRepository> = user_store.clone();
                let external_accounts: Arc<dyn UserExternalAccountRepository> = user_store.clone();
                let webauthn: Arc<dyn WebauthnRepository> = webauthn_store.clone();
                let totp: Arc<dyn TotpRepository> = totp_store.clone();
                let oauth: Arc<dyn OAuthRepository> = oauth_store.clone();
                let libraries: Arc<dyn LibraryRepository> = library_store.clone();
                let media_requests: Arc<dyn MediaRequestRepository> = media_request_store.clone();

                AppServices::builder(
                    titles,
                    shows,
                    users,
                    self.indexer_configs(),
                    indexer_client,
                    download_client,
                    self.download_client_configs(),
                    release_store.clone(),
                    self.settings(),
                    self.quality_profiles(),
                    self.backup_dir(),
                )
                .with_indexer_error_repository(self.indexer_errors())
                .with_libraries(libraries)
                .with_media_requests(media_requests)
                .with_user_ui_settings_store(ui_settings)
                .with_external_account_store(external_accounts)
                .with_oauth_store(oauth)
                .with_indexer_proxy_config_store(self.indexer_proxy_configs())
                .with_seeding_profiles(self.seeding_profiles())
                .with_external_identity_verifier(Arc::new(HttpExternalIdentityVerifier::new()))
                .with_media_server_connection_store(media_server_connection_store.clone())
                // Maintenance safety: live playback observation (RFC 137 §9.10, WP-G).
                .with_media_server_playback_probe(Arc::new(HttpMediaServerPlaybackProbe::new(
                    media_server_connection_store.clone(),
                )))
                // Media-server watch signals (RFC 137 §7.3, WP-M).
                .with_media_server_signal_source(Arc::new(HttpMediaServerSignalSource::new()))
                .with_media_server_signal_store(self.media_server_signal_store())
                .with_webauthn_store(webauthn)
                .with_totp_store(totp)
                .with_media_files(media_file_store.clone())
                .with_acquisition_scope_states(wanted_store.clone())
                .with_scope_indexer_coverage_store(self.scope_indexer_coverage_repository())
                .with_pending_releases(pending_release_store.clone())
                .with_blocklist_repo(blocklist_store.clone())
                .with_library_probe_signatures(library_probe_store.clone())
                .with_library_scan_unmatched_items(library_scan_unmatched_store.clone())
                .with_location_operation_repository(location_operation_store.clone())
                // The US7 merge store needs only the datastore, so it is
                // built here rather than threaded through both store
                // variants: nothing else in the assembly holds it.
                .with_title_merge_repository(Arc::new(TitleMergeStore::new(
                    self.datastore(),
                )))
                .with_title_images(title_image_store.clone())
                .with_image_proxy(image_proxy_store.clone())
                .with_housekeeping(housekeeping_store.clone())
                .with_subtitle_downloads(subtitle_download_store.clone())
                .with_rule_set_store(rule_set_store.clone())
                .with_maintenance_rule_set_store(self.maintenance_rule_set_store())
                .with_maintenance_evaluation_store(self.maintenance_evaluation_store())
                .with_post_processing_script_store(post_processing_script_store.clone())
                .with_plugin_installation_store(plugin_store.clone())
                .with_acquisition_state(acquisition_store.clone())
                .with_domain_events(domain_event_store.clone())
                .with_download_registry(download_registry_store.clone())
                .with_download_submissions(download_submission_store.clone())
                .with_download_queue_commands(download_queue_command_store.clone())
                .with_external_import_monitor_snapshots(external_import_monitor_store.clone())
                .with_external_import_setup_secret_drafts(
                    external_import_setup_secret_draft_store.clone(),
                )
                .with_import_artifacts(import_store.clone())
                .with_imports(import_store.clone())
                .with_job_runs(workflow_operation_store.clone())
                .with_discovery_store(discovery_store.clone())
                .with_notification_store(notification_store.clone())
                .with_system_info(settings_store.clone())
                .with_logical_backup_exporter(self.logical_backup_exporter())
                .with_title_image_processor(title_image_processor())
                .with_workflow_operations(workflow_operation_store.clone())
            }
        }
    }
}

pub async fn validate_datastore(config: DatastoreConfig) -> Result<(), AppError> {
    match config.engine {
        DatastoreEngine::Sqlite => {
            SqliteServices::new_with_mode(config.database_url, config.migration_mode).await?;
            Ok(())
        }
        DatastoreEngine::Postgres => {
            PostgresServices::new_with_mode(config.database_url, config.migration_mode).await?;
            Ok(())
        }
    }
}

pub async fn restore_backup_bundle_to_datastore(
    config: DatastoreConfig,
    bundle_path: &Path,
    passphrase: Option<&str>,
) -> AppResult<scryer_application::BackupRestorePreparedBundle> {
    match config.engine {
        DatastoreEngine::Sqlite => {
            let target_db_path = datastore_file_path(&config.database_url);
            restore_backup_bundle_to_datastore_path(
                &target_db_path,
                config.migration_mode,
                bundle_path,
                passphrase,
            )
            .await
        }
        DatastoreEngine::Postgres => {
            let services =
                PostgresServices::new_with_mode(config.database_url, config.migration_mode).await?;
            let restore_result =
                restore_backup_bundle_into_postgres_pool(services.pool(), bundle_path, passphrase)
                    .await;
            services.pool().close().await;
            restore_result
        }
    }
}

pub async fn restore_prepared_backup_directory_to_datastore(
    config: DatastoreConfig,
    prepared_root: &Path,
) -> AppResult<scryer_application::BackupRestorePreparedBundle> {
    match config.engine {
        DatastoreEngine::Sqlite => Err(AppError::Validation(
            "prepared backup directory restore is only supported for PostgreSQL".into(),
        )),
        DatastoreEngine::Postgres => {
            let services =
                PostgresServices::new_with_mode(config.database_url, config.migration_mode).await?;
            let restore_result = restore_prepared_backup_directory_into_postgres_pool(
                services.pool(),
                prepared_root,
            )
            .await;
            services.pool().close().await;
            restore_result
        }
    }
}

pub async fn restore_backup_bundle_to_datastore_path(
    target_db_path: &Path,
    migration_mode: MigrationMode,
    bundle_path: &Path,
    passphrase: Option<&str>,
) -> AppResult<scryer_application::BackupRestorePreparedBundle> {
    let services =
        SqliteServices::new_with_mode(target_db_path.to_string_lossy(), migration_mode).await?;
    let restore_result = crate::sqlite_backup::restore_backup_bundle_into_sqlite_pool(
        services.pool(),
        bundle_path,
        passphrase,
    )
    .await;

    let checkpoint_result = if restore_result.is_ok() {
        sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(services.pool())
            .await
            .map(|_| ())
            .map_err(|error| {
                AppError::Repository(format!("failed to checkpoint restored database: {error}"))
            })
    } else {
        Ok(())
    };

    services.pool().close().await;
    drop(services);
    let prepared = restore_result?;
    checkpoint_result?;
    Ok(prepared)
}

pub fn datastore_file_path(database_url: &str) -> PathBuf {
    let raw = database_url
        .strip_prefix("sqlite://")
        .unwrap_or(database_url);
    let raw = raw.split('?').next().unwrap_or(raw);
    PathBuf::from(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use scryer_application::{
        AcquisitionScopeStateRepository, BACKUP_TABLE_CATALOG, BackupBundleExportRequest,
        BackupExportSecrets, BackupTableClassification, DiscoverySyncStateRecord,
        LogicalBackupExporter, ReleaseDecision, SettingsRepository, SystemInfoProvider,
        TitleImageKind, TitleImageRepository, TitleImageSourceResult, TitleImageVariantRecord,
        TitleRepository, UserRepository, inspect_backup_bundle,
    };
    use scryer_domain::{ExternalId, MediaFacet, Title, User};
    use sqlx::Row;
    use std::sync::{Mutex, MutexGuard};
    use tempfile::TempDir;

    use crate::SettingDefinitionSeed;
    use crate::queries::sql_runtime::{SqlArg, SqlRuntime};

    static ENV_LOCK: Mutex<()> = Mutex::new(());
    const DATASTORE_ENV_KEYS: &[&str] = &[
        "SCRYER_DB_URL",
        "SCRYER_DB_PATH",
        "SCRYER_DB_USER",
        "SCRYER_DB_PASSWORD",
        "SCRYER_DB_PASSWORD_FILE",
    ];

    struct EnvSnapshot {
        _guard: MutexGuard<'static, ()>,
        values: Vec<(&'static str, Option<String>)>,
    }

    impl EnvSnapshot {
        fn new() -> Self {
            let guard = ENV_LOCK.lock().expect("env lock");
            let values = DATASTORE_ENV_KEYS
                .iter()
                .map(|key| (*key, std::env::var(key).ok()))
                .collect::<Vec<_>>();
            for key in DATASTORE_ENV_KEYS {
                clear_env(key);
            }
            Self {
                _guard: guard,
                values,
            }
        }
    }

    impl Drop for EnvSnapshot {
        fn drop(&mut self) {
            for (key, value) in &self.values {
                match value {
                    Some(value) => set_env(key, value),
                    None => clear_env(key),
                }
            }
        }
    }

    fn set_env(key: &str, value: &str) {
        // Tests serialize env mutation with ENV_LOCK.
        unsafe { std::env::set_var(key, value) };
    }

    fn clear_env(key: &str) {
        // Tests serialize env mutation with ENV_LOCK.
        unsafe { std::env::remove_var(key) };
    }

    fn data_dir() -> PathBuf {
        std::env::temp_dir().join("scryer-datastore-config-tests")
    }

    #[tokio::test]
    async fn sqlite_scope_indexer_coverage_repository_persists_rows() {
        let data_dir = TempDir::new().expect("data dir");
        let database_path = data_dir.path().join("scryer.db");
        let assembly = DatastoreAssembly::connect(DatastoreConfig::sqlite(
            format!("sqlite://{}", database_path.display()),
            data_dir.path(),
            MigrationMode::Apply,
        ))
        .await
        .expect("sqlite datastore assembly");

        let coverage = assembly.scope_indexer_coverage_repository();
        coverage
            .record_coverage("title:title-1", "movie", "indexer-1", "fingerprint-1")
            .await
            .expect("record coverage");

        assert_eq!(
            coverage
                .covered_indexers("title:title-1", "movie", "fingerprint-1", None)
                .await
                .expect("read coverage"),
            vec!["indexer-1".to_string()]
        );
    }

    fn validation_message(result: AppResult<DatastoreConfig>) -> String {
        match result {
            Err(AppError::Validation(message)) => message,
            other => panic!("expected validation error, got {other:?}"),
        }
    }

    #[test]
    fn resolves_sqlite_default_and_db_path_fallback() {
        let _env = EnvSnapshot::new();
        let config = resolve_datastore_config_from_env(data_dir(), MigrationMode::Apply)
            .expect("default sqlite config");
        assert_eq!(config.engine, DatastoreEngine::Sqlite);
        assert_eq!(config.source, DatastoreConfigSource::DefaultSqlite);
        assert!(config.database_url.ends_with("/scryer.db"));
        assert_eq!(config.database_url, config.safe_database_url());

        set_env("SCRYER_DB_PATH", "sqlite:///custom/scryer.db");
        let config = resolve_datastore_config_from_env(data_dir(), MigrationMode::Apply)
            .expect("db path config");
        assert_eq!(config.engine, DatastoreEngine::Sqlite);
        assert_eq!(config.source, DatastoreConfigSource::EnvDbPath);
        assert_eq!(config.database_url, "sqlite:///custom/scryer.db");
    }

    #[test]
    fn db_url_precedes_db_path_and_redacts_postgres_credentials() {
        let _env = EnvSnapshot::new();
        set_env("SCRYER_DB_PATH", "sqlite:///ignored.db");
        set_env(
            "SCRYER_DB_URL",
            "postgres://url_user:url_pass@db:5432/scryer?sslmode=require",
        );
        set_env("SCRYER_DB_USER", "env_user");
        set_env("SCRYER_DB_PASSWORD", "env_pass");

        let config = resolve_datastore_config_from_env(data_dir(), MigrationMode::Apply)
            .expect("postgres config");
        assert_eq!(config.engine, DatastoreEngine::Postgres);
        assert_eq!(config.source, DatastoreConfigSource::EnvDbUrl);
        assert!(
            config
                .database_url
                .starts_with("postgres://env_user:env_pass@")
        );
        assert!(!config.safe_database_url().contains("env_user"));
        assert!(!config.safe_database_url().contains("env_pass"));
        assert!(config.safe_database_url().contains("%3Credacted%3E"));
    }

    #[test]
    fn password_file_overrides_password_env_and_url_password() {
        let _env = EnvSnapshot::new();
        let password_path = data_dir().join(format!("password-{}.txt", std::process::id()));
        std::fs::create_dir_all(password_path.parent().expect("password parent"))
            .expect("password dir");
        std::fs::write(&password_path, "file_pass\n").expect("password file");

        set_env(
            "SCRYER_DB_URL",
            "postgres://url_user:url_pass@db:5432/scryer?sslmode=require",
        );
        set_env("SCRYER_DB_PASSWORD", "env_pass");
        set_env(
            "SCRYER_DB_PASSWORD_FILE",
            password_path.to_str().expect("utf-8 password path"),
        );

        let config = resolve_datastore_config_from_env(data_dir(), MigrationMode::Apply)
            .expect("postgres config");
        assert!(config.database_url.contains("file_pass"));
        assert!(!config.database_url.contains("env_pass"));
        assert!(!config.database_url.contains("url_pass"));

        let _ = std::fs::remove_file(password_path);
    }

    #[test]
    fn password_env_preserves_operator_secret_bytes() {
        let _env = EnvSnapshot::new();
        set_env(
            "SCRYER_DB_URL",
            "postgres://url_user:url_pass@db:5432/scryer?sslmode=require",
        );
        set_env("SCRYER_DB_PASSWORD", "  env pass  ");

        let config = resolve_datastore_config_from_env(data_dir(), MigrationMode::Apply)
            .expect("postgres config");
        let parsed = url::Url::parse(&config.database_url).expect("valid postgres url");
        assert_eq!(parsed.password(), Some("%20%20env%20pass%20%20"));
    }

    #[test]
    fn postgres_url_requires_database_credentials_and_sslmode() {
        let _env = EnvSnapshot::new();

        set_env(
            "SCRYER_DB_URL",
            "postgres://user:pass@/scryer?sslmode=require",
        );
        assert!(
            validation_message(resolve_datastore_config_from_env(
                data_dir(),
                MigrationMode::Apply
            ))
            .contains("host")
        );

        set_env("SCRYER_DB_URL", "postgres://user:pass@db:5432/scryer");
        assert!(
            validation_message(resolve_datastore_config_from_env(
                data_dir(),
                MigrationMode::Apply
            ))
            .contains("explicit sslmode")
        );

        set_env(
            "SCRYER_DB_URL",
            "postgres://user:pass@db:5432/?sslmode=require",
        );
        assert!(
            validation_message(resolve_datastore_config_from_env(
                data_dir(),
                MigrationMode::Apply
            ))
            .contains("database name")
        );

        set_env("SCRYER_DB_URL", "postgres://db:5432/scryer?sslmode=require");
        assert!(
            validation_message(resolve_datastore_config_from_env(
                data_dir(),
                MigrationMode::Apply
            ))
            .contains("SCRYER_DB_USER")
        );

        set_env("SCRYER_DB_USER", "user");
        assert!(
            validation_message(resolve_datastore_config_from_env(
                data_dir(),
                MigrationMode::Apply
            ))
            .contains("SCRYER_DB_PASSWORD")
        );
    }

    #[test]
    fn rejects_unsupported_datastore_url_scheme_and_sslmode() {
        let _env = EnvSnapshot::new();

        set_env("SCRYER_DB_URL", "mysql://db/scryer");
        assert!(
            validation_message(resolve_datastore_config_from_env(
                data_dir(),
                MigrationMode::Apply
            ))
            .contains("unsupported datastore URL scheme")
        );

        set_env(
            "SCRYER_DB_URL",
            "postgres://user:pass@db:5432/scryer?sslmode=allow",
        );
        assert!(
            validation_message(resolve_datastore_config_from_env(
                data_dir(),
                MigrationMode::Apply
            ))
            .contains("unsupported PostgreSQL sslmode")
        );
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum TestBackupEngine {
        Sqlite,
    }

    struct TestBackupSource {
        engine: TestBackupEngine,
        database_url: String,
        sqlite: Option<SqliteServices>,
        postgres: Option<PostgresServices>,
        admin_pool: Option<sqlx::PgPool>,
        schema: Option<String>,
        _temp: TempDir,
    }

    struct TestBackupTarget {
        engine: TestBackupEngine,
        config: DatastoreConfig,
        admin_pool: Option<sqlx::PgPool>,
        schema: Option<String>,
        _temp: TempDir,
    }

    #[cfg(not(feature = "runtime-backups"))]
    const BACKUP_PAYLOAD_SUPPORT_UNAVAILABLE: &str =
        "backup bundle payload support is not compiled into this target";

    #[tokio::test]
    async fn sqlite_logical_backup_restore_round_trip_preserves_setup_data() -> AppResult<()> {
        run_backup_restore_round_trip_or_skip(TestBackupEngine::Sqlite, TestBackupEngine::Sqlite)
            .await
    }

    async fn run_backup_restore_round_trip_or_skip(
        source_engine: TestBackupEngine,
        target_engine: TestBackupEngine,
    ) -> AppResult<()> {
        let result = run_backup_restore_round_trip(source_engine, target_engine).await;
        #[cfg(feature = "runtime-backups")]
        {
            result
        }
        #[cfg(not(feature = "runtime-backups"))]
        match result {
            Err(AppError::Repository(message)) if message == BACKUP_PAYLOAD_SUPPORT_UNAVAILABLE => {
                eprintln!(
                    "skipping backup/restore round trip; {BACKUP_PAYLOAD_SUPPORT_UNAVAILABLE}"
                );
                Ok(())
            }
            result => result,
        }
    }

    async fn run_backup_restore_round_trip(
        source_engine: TestBackupEngine,
        target_engine: TestBackupEngine,
    ) -> AppResult<()> {
        let source = TestBackupSource::new(source_engine).await?;
        let target = TestBackupTarget::new(target_engine).await?;
        let bundle_dir = tempfile::tempdir().map_err(|error| {
            AppError::Repository(format!("failed to create backup bundle tempdir: {error}"))
        })?;
        let bundle_path = bundle_dir.path().join("matrix.scryer-backup.enc");
        let passphrase = "scryer-backup-matrix-passphrase";

        let result = async {
            source.seed().await?;
            target.seed_stale_ephemeral_rows().await?;
            let outcome = source.export_backup(&bundle_path, passphrase).await?;
            let inspected = inspect_backup_bundle(&bundle_path, Some(passphrase))?;
            let expected_bundle_tables = BACKUP_TABLE_CATALOG
                .iter()
                .filter(|entry| entry.classification == BackupTableClassification::Export)
                .map(|entry| entry.table.to_string())
                .collect::<std::collections::BTreeSet<_>>();
            let inspected_tables = inspected
                .row_counts
                .keys()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>();
            assert!(outcome.summary.encrypted);
            assert_eq!(outcome.summary.source_engine, source.engine_name());
            assert_eq!(
                outcome
                    .summary
                    .row_counts
                    .get("settings_definitions")
                    .copied(),
                Some(4),
                "backup should include the seeded setting definition rows"
            );
            assert_eq!(
                inspected.row_counts.get("settings_definitions").copied(),
                Some(4),
                "inspected bundle should persist the setting definition row count"
            );
            assert_eq!(
                inspected.row_counts.get("settings_values").copied(),
                Some(4),
                "inspected bundle should persist the seeded setting value row count"
            );
            assert_eq!(
                inspected_tables, expected_bundle_tables,
                "inspected bundle table set should match the export catalog"
            );
            for table in [
                "discovery_sync_state",
                "title_more_like_this_items",
                "title_images",
                "title_image_variants",
                "title_image_blobs",
            ] {
                assert!(
                    !outcome.summary.row_counts.contains_key(table),
                    "backup should omit reset-only table {table}"
                );
                assert!(
                    !inspected.row_counts.contains_key(table),
                    "inspected bundle should omit reset-only table {table}"
                );
            }
            assert_eq!(
                inspected.row_counts.get("scope_indexer_coverage").copied(),
                Some(1),
                "inspected bundle should persist convergence coverage"
            );
            assert!(
                outcome.summary.row_counts.contains_key("settings_values"),
                "backup should include settings JSON rows"
            );
            assert!(
                outcome.summary.row_counts.contains_key("titles"),
                "backup should include title rows"
            );

            let prepared = restore_backup_bundle_to_datastore(
                target.config.clone(),
                &bundle_path,
                Some(passphrase),
            )
            .await?;
            assert_eq!(prepared.summary().source_engine, source.engine_name());
            assert!(prepared.summary().encrypted);
            assert!(
                prepared
                    .instance_secrets_env()
                    .contains("SCRYER_ENCRYPTION_KEY"),
                "restore should return restored instance secrets"
            );

            target.verify_restored().await
        }
        .await;

        let source_cleanup = source.cleanup().await;
        let target_cleanup = target.cleanup().await;
        result?;
        source_cleanup?;
        target_cleanup?;
        Ok(())
    }

    impl TestBackupSource {
        async fn new(engine: TestBackupEngine) -> AppResult<Self> {
            let temp = tempfile::tempdir().map_err(|error| {
                AppError::Repository(format!("failed to create source tempdir: {error}"))
            })?;
            let data_dir = temp.path().join("data");
            std::fs::create_dir_all(&data_dir).map_err(|error| {
                AppError::Repository(format!("failed to create source data dir: {error}"))
            })?;

            let db_path = data_dir.join("scryer.db");
            let database_url = format!("sqlite://{}", db_path.display());
            let sqlite =
                SqliteServices::new_with_mode(database_url.clone(), MigrationMode::Apply).await?;
            Ok(Self {
                engine,
                database_url,
                sqlite: Some(sqlite),
                postgres: None,
                admin_pool: None,
                schema: None,
                _temp: temp,
            })
        }

        fn engine_name(&self) -> &'static str {
            let TestBackupEngine::Sqlite = self.engine;
            "sqlite"
        }

        async fn seed(&self) -> AppResult<()> {
            let services = self.sqlite.as_ref().expect("sqlite source");
            let datastore = services.datastore();
            let settings = SettingsStore::new(datastore.clone(), services.encryption_key_state());
            let titles = TitleStore::new(datastore.clone());
            let images = TitleImageStore::new(datastore.clone());
            let users = UserStore::new(datastore.clone());
            settings
                .batch_ensure_setting_definitions(backup_matrix_setting_definitions())
                .await?;
            seed_backup_matrix_data(&settings, &titles, &users).await?;
            seed_backup_matrix_title_image(&images).await?;
            seed_backup_matrix_runtime_state(
                &datastore,
                "source-fingerprint",
                "2026-07-17T12:34:56Z",
                "source-discovery-generation",
            )
            .await
        }

        async fn export_backup(
            &self,
            output_path: &Path,
            passphrase: &str,
        ) -> AppResult<scryer_application::BackupExportOutcome> {
            let request = BackupBundleExportRequest {
                output_path: output_path.to_path_buf(),
                passphrase: passphrase.to_string(),
                source_migration_key: self.current_migration_key().await?,
                source_scryer_version: "backup-matrix-test".to_string(),
                source_engine: self.engine_name().to_string(),
                secrets: BackupExportSecrets {
                    encryption_master_key: "test-master-key".to_string(),
                    jwt_signing_secret: "test-jwt-secret".to_string(),
                    smg_registration_secret: Some("test-smg-secret".to_string()),
                    smg_gateway_url: Some("https://smg.example.invalid/graphql".to_string()),
                },
            };

            SqliteLogicalBackupExporter::new(self.database_url.clone())
                .export_backup_bundle(request)
                .await
        }

        async fn current_migration_key(&self) -> AppResult<Option<String>> {
            let services = self.sqlite.as_ref().expect("sqlite source");
            let settings =
                SettingsStore::new(services.datastore(), services.encryption_key_state());
            settings
                .datastore_info()
                .await
                .map(|info| info.current_migration_key)
        }

        async fn cleanup(self) -> AppResult<()> {
            if let Some(sqlite) = self.sqlite {
                sqlite.pool().close().await;
            }
            if let Some(postgres) = self.postgres {
                postgres.pool().close().await;
            }
            cleanup_postgres_schema(self.admin_pool, self.schema).await
        }
    }

    impl TestBackupTarget {
        async fn new(engine: TestBackupEngine) -> AppResult<Self> {
            let temp = tempfile::tempdir().map_err(|error| {
                AppError::Repository(format!("failed to create target tempdir: {error}"))
            })?;
            let data_dir = temp.path().join("data");
            std::fs::create_dir_all(&data_dir).map_err(|error| {
                AppError::Repository(format!("failed to create target data dir: {error}"))
            })?;

            let db_path = data_dir.join("scryer.db");
            let database_url = format!("sqlite://{}", db_path.display());
            Ok(Self {
                engine,
                config: DatastoreConfig::sqlite(database_url, data_dir, MigrationMode::Apply),
                admin_pool: None,
                schema: None,
                _temp: temp,
            })
        }
        async fn seed_stale_ephemeral_rows(&self) -> AppResult<()> {
            let TestBackupEngine::Sqlite = self.engine;
            {
                let services = SqliteServices::new_with_mode(
                    self.config.database_url.clone(),
                    MigrationMode::Apply,
                )
                .await?;
                let datastore = services.datastore();
                let titles = TitleStore::new(datastore.clone());
                let images = TitleImageStore::new(datastore.clone());
                TitleRepository::create(&titles, backup_matrix_title()).await?;
                seed_backup_matrix_title_image(&images).await?;
                seed_backup_matrix_runtime_state(
                    &datastore,
                    "target-fingerprint",
                    "2025-01-02T03:04:05Z",
                    "target-discovery-generation",
                )
                .await?;
                services.pool().close().await;
            }
            Ok(())
        }

        async fn verify_restored(&self) -> AppResult<()> {
            let services = SqliteServices::new_with_mode(
                self.config.database_url.clone(),
                MigrationMode::Apply,
            )
            .await?;
            let datastore = services.datastore();
            let settings = SettingsStore::new(datastore.clone(), services.encryption_key_state());
            let titles = TitleStore::new(datastore.clone());
            let images = TitleImageStore::new(datastore.clone());
            let users = UserStore::new(datastore.clone());
            verify_backup_matrix_data(&settings, &titles, &users).await?;
            verify_backup_matrix_runtime_state(&datastore).await?;
            verify_backup_matrix_title_image_restore(&images).await?;
            verify_sqlite_title_image_restore_tables(services.pool()).await?;
            services.pool().close().await;
            Ok(())
        }

        async fn cleanup(self) -> AppResult<()> {
            cleanup_postgres_schema(self.admin_pool, self.schema).await
        }
    }

    async fn seed_backup_matrix_data<S, T, U>(settings: &S, titles: &T, users: &U) -> AppResult<()>
    where
        S: SettingsRepository,
        T: TitleRepository,
        U: UserRepository,
    {
        settings
            .upsert_setting_json(
                "backup_matrix",
                "json_payload",
                None,
                serde_json::json!({
                    "encrypted_config": {
                        "secret_ref": "matrix-secret",
                        "enabled": true
                    },
                    "plugin_descriptor": {
                        "id": "matrix.plugin",
                        "version": "1.0.0"
                    },
                    "search_state": ["alpha", "beta"]
                })
                .to_string(),
                "backup_matrix_test",
                None,
            )
            .await?;

        settings
            .upsert_setting_json(
                "system",
                "acquisition.convergence_resume_after",
                None,
                serde_json::json!("title:backup-lattice-title").to_string(),
                "backup_matrix_test",
                None,
            )
            .await?;
        settings
            .upsert_setting_json(
                "system",
                "acquisition.convergence_seeded_at",
                None,
                serde_json::json!("2026-07-17T00:00:00Z").to_string(),
                "backup_matrix_test",
                None,
            )
            .await?;
        settings
            .upsert_setting_json(
                "system",
                "auth.totp.require_emby_login",
                None,
                "true".to_string(),
                "backup_matrix_test",
                None,
            )
            .await?;

        UserRepository::create(users, User::new_admin("backup-matrix-admin")).await?;
        TitleRepository::create(titles, backup_matrix_title()).await?;
        Ok(())
    }

    async fn seed_backup_matrix_runtime_state(
        datastore: &StoreDatastore,
        coverage_fingerprint: &str,
        coverage_searched_at: &str,
        discovery_marker: &str,
    ) -> AppResult<()> {
        ScopeIndexerCoverageStore::new(datastore.clone())
            .record_coverage(
                "backup-lattice-title",
                "movie",
                "backup-indexer",
                coverage_fingerprint,
            )
            .await?;
        let now = chrono::Utc::now();
        SqlRuntime::execute(
            datastore.read_exec(),
            "INSERT INTO wanted_items (
                 id, title_id, media_type, status, created_at, updated_at
             ) VALUES ({}, {}, {}, {}, {}, {})",
            &[
                SqlArg::Text("backup-matrix-wanted".to_string()),
                SqlArg::Text("backup-lattice-title".to_string()),
                SqlArg::Text("movie".to_string()),
                SqlArg::Text("wanted".to_string()),
                SqlArg::Timestamp(now),
                SqlArg::Timestamp(now),
            ],
        )
        .await?;
        WantedStore::new(datastore.clone())
            .insert_release_decision(&ReleaseDecision {
                id: "backup-matrix-release-decision".to_string(),
                wanted_item_id: "backup-matrix-wanted".to_string(),
                title_id: "backup-lattice-title".to_string(),
                release_title: "Synthetic Backup Release".to_string(),
                release_url: None,
                release_size_bytes: Some(1_000_000),
                decision_code: "eligible".to_string(),
                candidate_score: 100,
                current_score: None,
                score_delta: None,
                explanation_json: Some(
                    serde_json::json!({
                        "quality_profile_decision": {
                            "scoring_log": [{"code": "backup_matrix", "delta": 100}],
                        },
                        "fingerprint": coverage_fingerprint,
                    })
                    .to_string(),
                ),
                created_at: now.to_rfc3339(),
            })
            .await?;
        let coverage_searched_at = chrono::DateTime::parse_from_rfc3339(coverage_searched_at)
            .map_err(|error| {
                AppError::Repository(format!("invalid backup matrix coverage timestamp: {error}"))
            })?
            .with_timezone(&chrono::Utc);
        SqlRuntime::execute(
            datastore.read_exec(),
            "UPDATE scope_indexer_coverage
                SET searched_at = {}
              WHERE scope_key = {} AND facet = {} AND indexer_id = {}",
            &[
                SqlArg::Timestamp(coverage_searched_at),
                SqlArg::Text("backup-lattice-title".to_string()),
                SqlArg::Text("movie".to_string()),
                SqlArg::Text("backup-indexer".to_string()),
            ],
        )
        .await?;
        seed_backup_matrix_emby_connection(datastore, coverage_fingerprint).await?;

        let discovery_state = DiscoverySyncStateRecord {
            last_subject_fingerprint: Some(discovery_marker.to_string()),
            ..DiscoverySyncStateRecord::default()
        };
        DiscoveryStore::new(datastore.clone())
            .upsert_discovery_sync_state(&discovery_state)
            .await
    }

    async fn seed_backup_matrix_emby_connection(
        datastore: &StoreDatastore,
        fingerprint: &str,
    ) -> AppResult<()> {
        let now = chrono::Utc::now();
        SqlRuntime::execute(
            datastore.read_exec(),
            "INSERT INTO media_server_connections (
                 id, provider, display_name, base_url, enabled, login_enabled,
                 linking_enabled, auto_add_enabled, default_app_permissions, created_at, updated_at
             ) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
            &[
                SqlArg::Text("backup-matrix-emby".to_string()),
                SqlArg::Text("emby".to_string()),
                SqlArg::Text("Backup Matrix Emby".to_string()),
                SqlArg::Text(format!("https://{fingerprint}.emby.invalid/emby")),
                SqlArg::Bool(true),
                SqlArg::Bool(true),
                SqlArg::Bool(true),
                SqlArg::Bool(false),
                SqlArg::I64(0),
                SqlArg::Timestamp(now),
                SqlArg::Timestamp(now),
            ],
        )
        .await?;
        SqlRuntime::execute(
            datastore.read_exec(),
            "INSERT INTO emby_media_server_details (
                 connection_id, api_key, server_id, connect_enabled, created_at, updated_at
             ) VALUES ({}, {}, {}, {}, {}, {})",
            &[
                SqlArg::Text("backup-matrix-emby".to_string()),
                SqlArg::Text(format!("{fingerprint}-emby-api-key")),
                SqlArg::Text(format!("{fingerprint}-emby-server-id")),
                SqlArg::Bool(fingerprint == "source-fingerprint"),
                SqlArg::Timestamp(now),
                SqlArg::Timestamp(now),
            ],
        )
        .await?;
        Ok(())
    }

    async fn verify_backup_matrix_runtime_state(datastore: &StoreDatastore) -> AppResult<()> {
        let emby = SqlRuntime::fetch_optional(
            datastore.read_exec(),
            "SELECT connection.base_url, detail.api_key, detail.server_id, detail.connect_enabled
               FROM media_server_connections connection
               JOIN emby_media_server_details detail
                 ON detail.connection_id = connection.id
              WHERE connection.id = {}",
            &[SqlArg::Text("backup-matrix-emby".to_string())],
        )
        .await?
        .expect("restored Emby connection should exist");
        assert_eq!(
            emby.text("base_url")?,
            "https://source-fingerprint.emby.invalid/emby"
        );
        assert_eq!(emby.text("api_key")?, "source-fingerprint-emby-api-key");
        assert_eq!(emby.text("server_id")?, "source-fingerprint-emby-server-id");
        assert!(emby.bool("connect_enabled")?);

        let release_decisions = WantedStore::new(datastore.clone())
            .list_release_decisions_for_title("backup-lattice-title", 10, 0)
            .await?;
        assert_eq!(release_decisions.len(), 1);
        let explanation = release_decisions[0]
            .explanation_json
            .as_deref()
            .expect("restored release decision should retain its explanation");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(explanation)
                .map_err(|error| AppError::Repository(error.to_string()))?["fingerprint"],
            "source-fingerprint",
            "compressed release-decision explanations should survive logical backup restore"
        );

        let coverage = ScopeIndexerCoverageStore::new(datastore.clone());
        assert_eq!(
            coverage
                .covered_indexers("backup-lattice-title", "movie", "source-fingerprint", None,)
                .await?,
            vec!["backup-indexer".to_string()],
            "source convergence coverage should round-trip"
        );
        assert!(
            coverage
                .covered_indexers("backup-lattice-title", "movie", "target-fingerprint", None,)
                .await?
                .is_empty(),
            "target convergence coverage should be replaced"
        );
        let coverage_rows = coverage
            .list_coverage_for_scope_keys(&["backup-lattice-title".to_string()])
            .await?;
        assert_eq!(coverage_rows.len(), 1);
        assert_eq!(
            chrono::DateTime::parse_from_rfc3339(&coverage_rows[0].searched_at)
                .map_err(|error| AppError::Repository(error.to_string()))?
                .to_utc(),
            chrono::DateTime::parse_from_rfc3339("2026-07-17T12:34:56Z")
                .expect("fixed source coverage timestamp")
                .to_utc(),
            "source convergence timestamp should replace stale target state"
        );

        let default_scope = DiscoverySyncStateRecord::default().scope_key;
        assert!(
            DiscoveryStore::new(datastore.clone())
                .get_discovery_sync_state(&default_scope)
                .await?
                .is_none(),
            "Discovery state should be empty after restore"
        );
        Ok(())
    }

    async fn seed_backup_matrix_title_image<I>(images: &I) -> AppResult<()>
    where
        I: TitleImageRepository,
    {
        let variant_bytes = vec![1, 2, 3, 4, 5];
        let variant_digest = format!("blake3:{}", blake3::hash(&variant_bytes).to_hex());
        images
            .upsert_title_image_source_result(
                "backup-lattice-title",
                TitleImageSourceResult {
                    kind: TitleImageKind::Poster,
                    requested_source_url: "https://image.tmdb.org/t/p/original/poster.jpg"
                        .to_string(),
                    source_url: "https://image.tmdb.org/t/p/original/poster.jpg".to_string(),
                    source_etag: Some("matrix-etag".to_string()),
                    source_last_modified: Some("Wed, 12 Jun 2026 03:00:00 GMT".to_string()),
                    source_format: "jpeg".to_string(),
                    source_width: 1200,
                    source_height: 1800,
                    variants: vec![TitleImageVariantRecord {
                        variant_key: "w250".to_string(),
                        format: "avif".to_string(),
                        width: 250,
                        height: 375,
                        bytes: variant_bytes,
                        digest: variant_digest,
                    }],
                },
                None,
            )
            .await?;
        Ok(())
    }

    fn backup_matrix_setting_definition() -> SettingDefinitionSeed {
        SettingDefinitionSeed {
            category: "backup_matrix".to_string(),
            scope: "backup_matrix".to_string(),
            key_name: "json_payload".to_string(),
            data_type: "json".to_string(),
            default_value_json: "{\"default\":true}".to_string(),
            is_sensitive: false,
            validation_json: Some("{\"type\":\"object\"}".to_string()),
        }
    }

    fn backup_matrix_setting_definitions() -> Vec<SettingDefinitionSeed> {
        vec![
            backup_matrix_setting_definition(),
            backup_matrix_convergence_setting_definition("acquisition.convergence_resume_after"),
            backup_matrix_convergence_setting_definition("acquisition.convergence_seeded_at"),
            SettingDefinitionSeed {
                category: "authentication".to_string(),
                scope: "system".to_string(),
                key_name: "auth.totp.require_emby_login".to_string(),
                data_type: "boolean".to_string(),
                default_value_json: "false".to_string(),
                is_sensitive: false,
                validation_json: None,
            },
        ]
    }

    fn backup_matrix_convergence_setting_definition(key_name: &str) -> SettingDefinitionSeed {
        SettingDefinitionSeed {
            category: "acquisition".to_string(),
            scope: "system".to_string(),
            key_name: key_name.to_string(),
            data_type: "json".to_string(),
            default_value_json: "null".to_string(),
            is_sensitive: false,
            validation_json: None,
        }
    }

    async fn verify_backup_matrix_data<S, T, U>(
        settings: &S,
        titles: &T,
        users: &U,
    ) -> AppResult<()>
    where
        S: SettingsRepository,
        T: TitleRepository,
        U: UserRepository,
    {
        let value = settings
            .get_setting_json("backup_matrix", "json_payload", None)
            .await?
            .ok_or_else(|| AppError::Repository("restored setting missing".into()))?;
        let decoded: serde_json::Value = serde_json::from_str(&value)
            .map_err(|error| AppError::Repository(format!("invalid restored JSON: {error}")))?;
        assert_eq!(decoded["plugin_descriptor"]["id"], "matrix.plugin");
        assert_eq!(decoded["encrypted_config"]["enabled"], true);
        assert_eq!(
            settings
                .get_setting_json("system", "acquisition.convergence_resume_after", None)
                .await?
                .as_deref(),
            Some("\"title:backup-lattice-title\"")
        );
        assert_eq!(
            settings
                .get_setting_json("system", "acquisition.convergence_seeded_at", None)
                .await?
                .as_deref(),
            Some("\"2026-07-17T00:00:00Z\"")
        );
        assert_eq!(
            settings
                .get_setting_json("system", "auth.totp.require_emby_login", None)
                .await?
                .as_deref(),
            Some("true")
        );

        let user = UserRepository::get_by_username(users, "backup-matrix-admin").await?;
        assert!(user.is_some(), "restored admin identity should exist");

        let title = TitleRepository::get_by_id(titles, "backup-lattice-title").await?;
        let title = title.expect("restored title should exist");
        assert_eq!(title.external_ids[0].source, "tmdb");
        assert_eq!(title.external_ids[0].value, "424242");
        assert_eq!(
            title.poster_url.as_deref(),
            Some("https://image.tmdb.org/t/p/original/poster.jpg"),
            "durable remote artwork URL should survive restore"
        );
        Ok(())
    }

    async fn verify_backup_matrix_title_image_restore<I>(images: &I) -> AppResult<()>
    where
        I: TitleImageRepository,
    {
        let blob = images
            .get_title_image_blob("backup-lattice-title", TitleImageKind::Poster, "w250")
            .await?;
        assert!(
            blob.is_none(),
            "restored backup should not include generated title image variant bytes"
        );

        let tasks = images.list_title_image_refresh_work(10, &[]).await?;
        let task = tasks
            .iter()
            .find(|task| {
                task.title_id == "backup-lattice-title" && task.kind == TitleImageKind::Poster
            })
            .expect("restored title image metadata should queue refresh work");
        assert_eq!(
            task.source_url,
            "https://image.tmdb.org/t/p/original/poster.jpg"
        );
        assert!(
            task.variants
                .iter()
                .any(|variant| variant.variant_key == "w250"),
            "poster refresh work should include the missing preferred variant"
        );
        Ok(())
    }

    async fn verify_sqlite_title_image_restore_tables(pool: &sqlx::SqlitePool) -> AppResult<()> {
        let row = sqlx::query(
            "SELECT poster_local_path, background_local_path
               FROM titles
              WHERE id = ?",
        )
        .bind("backup-lattice-title")
        .fetch_one(pool)
        .await
        .map_err(|error| AppError::Repository(format!("failed to load restored title: {error}")))?;
        let poster_local_path: Option<String> =
            row.try_get("poster_local_path").map_err(|error| {
                AppError::Repository(format!("failed to decode poster local path: {error}"))
            })?;
        let background_local_path: Option<String> =
            row.try_get("background_local_path").map_err(|error| {
                AppError::Repository(format!("failed to decode background local path: {error}"))
            })?;
        assert!(poster_local_path.is_none());
        assert!(background_local_path.is_none());

        let image_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM title_images")
            .fetch_one(pool)
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to count restored title images: {error}"))
            })?;
        assert_eq!(image_count, 0);

        let variant_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM title_image_variants")
            .fetch_one(pool)
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to count restored variants: {error}"))
            })?;
        assert_eq!(variant_count, 0);
        let blob_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM title_image_blobs")
            .fetch_one(pool)
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to count restored image blobs: {error}"))
            })?;
        assert_eq!(blob_count, 0);
        Ok(())
    }

    fn backup_matrix_title() -> Title {
        Title {
            id: "backup-lattice-title".to_string(),
            library_id: "movie_default_library".to_string(),
            name: "Backup Lattice Movie".to_string(),
            facet: MediaFacet::Movie,
            monitored: true,
            tags: vec!["backup".to_string(), "lattice".to_string()],
            canonical_tags: vec![],
            external_ids: vec![ExternalId {
                source: "tmdb".to_string(),
                value: "424242".to_string(),
            }],
            root_folder_id: scryer_domain::root_folder_id_for_path("/data/movies"),
            created_by: None,
            created_at: chrono::Utc::now(),
            year: Some(2026),
            overview: Some("Logical backup lattice fixture".to_string()),
            poster_url: Some("https://image.tmdb.org/t/p/original/poster.jpg".to_string()),
            poster_source_url: None,
            background_url: None,
            background_source_url: None,
            sort_title: Some("Backup Lattice Movie".to_string()),
            catalog_sort_key: String::new(),
            slug: Some("backup-lattice-movie".to_string()),
            imdb_id: Some("tt4242420".to_string()),
            runtime_minutes: Some(101),
            popularity: None,
            content_status: Some("released".to_string()),
            language: Some("eng".to_string()),
            first_aired: Some("2026-01-01".to_string()),
            network: None,
            studio: Some("Scryer Tests".to_string()),
            country: Some("US".to_string()),
            aliases: vec!["Lattice Fixture".to_string()],
            tagged_aliases: Vec::new(),
            metadata_language: Some("eng".to_string()),
            metadata_fetched_at: Some(chrono::Utc::now()),
            min_availability: None,
            digital_release_date: Some("2026-01-02".to_string()),
            folder_path: Some("/data/movies/Backup Lattice Movie (2026)".to_string()),
        }
    }

    async fn cleanup_postgres_schema(
        admin_pool: Option<sqlx::PgPool>,
        schema: Option<String>,
    ) -> AppResult<()> {
        if let (Some(admin_pool), Some(schema)) = (admin_pool, schema) {
            let cleanup = sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
                .execute(&admin_pool)
                .await
                .map(|_| ())
                .map_err(|error| {
                    AppError::Repository(format!("failed to drop test schema {schema}: {error}"))
                });
            admin_pool.close().await;
            cleanup?;
        }
        Ok(())
    }
}

#![allow(clippy::too_many_arguments)]

mod activity;
mod quality_profile;
mod release_parser;
mod library_scan;
mod library_rename;
pub(crate) mod nfo;
mod null_repositories;
mod types;
pub(crate) mod facet_handler;
mod facet_movie;
mod facet_series;
mod facet_registry;
mod app_usecase_activity;
mod app_usecase_admin;
mod app_usecase_rules;
mod app_usecase_catalog;
pub(crate) mod app_usecase_library;
mod app_usecase_discovery;
mod app_usecase_integration;
mod app_usecase_import;
mod app_usecase_security;
mod acquisition_policy;
mod app_usecase_acquisition;

use crate::activity::ActivityStream;
use async_trait::async_trait;
use chrono::{Duration, Utc};
use scryer_domain::{
    Collection, CompletedDownload, DownloadClientConfig, DownloadQueueItem, DownloadQueueState,
    Entitlement, Episode, EventType, ExternalId, HistoryEvent, Id, ImportFileResult, ImportRecord,
    IndexerConfig, MediaFacet, NewDownloadClientConfig, NewIndexerConfig, NewTitle, PolicyInput,
    PolicyOutput, RuleSet, Title, User,
};
use std::path::Path;
use rand_core::OsRng;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::broadcast;

pub type AppResult<T> = Result<T, AppError>;

use crate::quality_profile::resolve_profile_id_for_title;
pub use activity::{ActivityChannel, ActivityEvent, ActivityKind, ActivitySeverity};
pub use acquisition_policy::AcquisitionThresholds;
pub use app_usecase_acquisition::start_background_acquisition_poller;
pub use app_usecase_integration::start_download_queue_poller;
pub use library_scan::{
    AnimeEpisodeMapping, AnimeMapping, EpisodeMetadata, LibraryFile, LibraryScanSummary,
    LibraryScanner, MetadataGateway, MetadataSearchItem, MovieMetadata, SeasonMetadata,
    SeriesMetadata,
};
pub use app_usecase_import::{
    execute_manual_import, preview_manual_import, try_import_completed_downloads,
    ManualImportFileMapping, ManualImportFilePreview, ManualImportFileResult,
    ManualImportPreview,
};
pub use library_rename::{
    build_rename_plan_fingerprint, render_rename_template, LibraryRenamer,
    NullLibraryRenamer, RenameApplyItemResult, RenameApplyResult, RenameApplyStatus,
    RenameCollisionPolicy, RenameMissingMetadataPolicy, RenamePlan, RenamePlanItem,
    RenameWriteAction,
};
pub use quality_profile::{
    apply_age_scoring, default_quality_profile_1080p_for_search,
    default_quality_profile_for_search, evaluate_against_profile,
    parse_profile_catalog_from_json, QualityProfile,
    QualityProfileCriteria, QualityProfileDecision, ScoringEntry, ScoringSource, BLOCK_SCORE,
    QUALITY_PROFILE_CATALOG_KEY, QUALITY_PROFILE_ID_KEY,
};
pub use release_parser::{parse_release_metadata, ParsedEpisodeMetadata, ParsedReleaseMetadata};
pub use null_repositories::{
    NullFileImporter, NullImportRepository, NullIndexerStatsTracker, NullMediaFileRepository,
    NullRuleSetRepository, NullSystemInfoProvider, NullWantedItemRepository,
};
pub use types::{
    IndexerQueryStats, IndexerSearchResult, JwtAuthConfig, PrimaryCollectionSummary,
    ReleaseDecision, ReleaseDownloadAttemptOutcome, ReleaseDownloadFailureSignature, SystemHealth,
    TitleMediaFile, TitleMetadataUpdate, TitleReleaseBlocklistEntry, WantedItem,
};
pub(crate) use types::JwtClaims;
pub use facet_handler::{FacetHandler, HydrationResult};
pub use facet_movie::MovieFacetHandler;
pub use facet_series::SeriesFacetHandler;
pub use facet_registry::FacetRegistry;

const SETTINGS_SCOPE_SYSTEM: &str = "system";
const SETTINGS_SCOPE_MEDIA: &str = "media";
const INHERIT_QUALITY_PROFILE_VALUE: &str = "__inherit__";
const ALLOWED_DOWNLOAD_CLIENT_TYPES: [&str; 3] = ["nzbget", "sabnzbd", "qbittorrent"];
const INDEXER_PROVIDER_NZBGEEK: &str = "nzbgeek";

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("unauthorized: {0}")]
    Unauthorized(String),

    #[error("validation: {0}")]
    Validation(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("repository: {0}")]
    Repository(String),
}

#[derive(Clone)]
pub struct AppServices {
    pub titles: Arc<dyn TitleRepository>,
    pub shows: Arc<dyn ShowRepository>,
    pub users: Arc<dyn UserRepository>,
    pub events: Arc<dyn EventRepository>,
    pub indexer_configs: Arc<dyn IndexerConfigRepository>,
    pub indexer_client: Arc<dyn IndexerClient>,
    pub download_client: Arc<dyn DownloadClient>,
    pub metadata_gateway: Arc<dyn MetadataGateway>,
    pub library_scanner: Arc<dyn LibraryScanner>,
    pub library_renamer: Arc<dyn LibraryRenamer>,
    pub imports: Arc<dyn ImportRepository>,
    pub file_importer: Arc<dyn FileImporter>,
    pub media_files: Arc<dyn MediaFileRepository>,
    pub download_client_configs: Arc<dyn DownloadClientConfigRepository>,
    pub release_attempts: Arc<dyn ReleaseAttemptRepository>,
    pub settings: Arc<dyn SettingsRepository>,
    pub quality_profiles: Arc<dyn QualityProfileRepository>,
    pub wanted_items: Arc<dyn WantedItemRepository>,
    pub rule_sets: Arc<dyn RuleSetRepository>,
    pub system_info: Arc<dyn SystemInfoProvider>,
    pub indexer_stats: Arc<dyn IndexerStatsTracker>,
    pub user_rules: Arc<std::sync::RwLock<scryer_rules::UserRulesEngine>>,
    pub db_path: String,
    pub activity_stream: ActivityStream,
    pub event_broadcast: broadcast::Sender<HistoryEvent>,
    pub activity_event_broadcast: broadcast::Sender<ActivityEvent>,
    pub download_queue_broadcast: broadcast::Sender<Vec<DownloadQueueItem>>,
    pub acquisition_wake: Arc<tokio::sync::Notify>,
}

impl AppServices {
    pub fn with_default_channels(
        titles: Arc<dyn TitleRepository>,
        shows: Arc<dyn ShowRepository>,
        users: Arc<dyn UserRepository>,
        events: Arc<dyn EventRepository>,
        indexer_configs: Arc<dyn IndexerConfigRepository>,
        indexer_client: Arc<dyn IndexerClient>,
        download_client: Arc<dyn DownloadClient>,
        download_client_configs: Arc<dyn DownloadClientConfigRepository>,
        release_attempts: Arc<dyn ReleaseAttemptRepository>,
        settings: Arc<dyn SettingsRepository>,
        quality_profiles: Arc<dyn QualityProfileRepository>,
        db_path: String,
    ) -> Self {
        let (tx, _rx) = broadcast::channel(256);
        let (activity_tx, _activity_rx) = broadcast::channel(256);
        let (queue_tx, _queue_rx) = broadcast::channel(16);
        Self {
            titles,
            shows,
            users,
            events,
            indexer_configs,
            indexer_client,
            download_client,
            metadata_gateway: Arc::new(crate::library_scan::NullMetadataGateway),
            library_scanner: Arc::new(crate::library_scan::NullLibraryScanner),
            library_renamer: Arc::new(crate::library_rename::NullLibraryRenamer),
            imports: Arc::new(NullImportRepository),
            file_importer: Arc::new(NullFileImporter),
            media_files: Arc::new(NullMediaFileRepository),
            download_client_configs,
            release_attempts,
            settings,
            quality_profiles,
            wanted_items: Arc::new(NullWantedItemRepository),
            rule_sets: Arc::new(NullRuleSetRepository),
            system_info: Arc::new(NullSystemInfoProvider),
            indexer_stats: Arc::new(NullIndexerStatsTracker),
            user_rules: Arc::new(std::sync::RwLock::new(scryer_rules::UserRulesEngine::empty())),
            db_path,
            activity_stream: ActivityStream::new(),
            event_broadcast: tx,
            activity_event_broadcast: activity_tx,
            download_queue_broadcast: queue_tx,
            acquisition_wake: Arc::new(tokio::sync::Notify::new()),
        }
    }

    async fn record_event(
        &self,
        actor_user_id: Option<String>,
        title_id: Option<String>,
        event_type: EventType,
        message: String,
    ) -> AppResult<()> {
        let event = HistoryEvent {
            id: Id::new().0,
            event_type,
            actor_user_id,
            title_id,
            message,
            occurred_at: Utc::now(),
        };

        self.events
            .append(event.clone())
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let activity = ActivityEvent::new(
            ActivityKind::SystemNotice,
            event.actor_user_id.clone(),
            event.title_id.clone(),
            event.message.clone(),
            ActivitySeverity::Info,
            vec![ActivityChannel::WebUi],
        );
        self.activity_stream.push(activity.clone()).await;
        let _ = self.activity_event_broadcast.send(activity);
        let _ = self.event_broadcast.send(event);
        Ok(())
    }

    pub async fn record_activity_event(
        &self,
        actor_user_id: Option<String>,
        title_id: Option<String>,
        kind: ActivityKind,
        message: String,
        severity: ActivitySeverity,
        channels: Vec<ActivityChannel>,
    ) -> AppResult<()> {
        let event = ActivityEvent::new(kind, actor_user_id, title_id, message, severity, channels);
        self.activity_stream.push(event.clone()).await;
        let _ = self.activity_event_broadcast.send(event);
        Ok(())
    }
}

#[async_trait]
pub trait TitleRepository: Send + Sync {
    async fn list(&self, facet: Option<MediaFacet>, query: Option<String>)
        -> AppResult<Vec<Title>>;
    async fn get_by_id(&self, id: &str) -> AppResult<Option<Title>>;
    async fn create(&self, title: Title) -> AppResult<Title>;
    async fn update_monitored(&self, id: &str, monitored: bool) -> AppResult<Title>;
    async fn update_metadata(
        &self,
        id: &str,
        name: Option<String>,
        facet: Option<MediaFacet>,
        tags: Option<Vec<String>>,
    ) -> AppResult<Title>;
    async fn update_title_hydrated_metadata(
        &self,
        id: &str,
        metadata: TitleMetadataUpdate,
    ) -> AppResult<Title>;
    async fn delete(&self, id: &str) -> AppResult<()>;
}

#[async_trait]
pub trait ShowRepository: Send + Sync {
    async fn list_collections_for_title(&self, title_id: &str) -> AppResult<Vec<Collection>>;
    async fn get_collection_by_id(&self, collection_id: &str) -> AppResult<Option<Collection>>;
    async fn create_collection(&self, collection: Collection) -> AppResult<Collection>;
    async fn update_collection(
        &self,
        collection_id: &str,
        collection_type: Option<String>,
        collection_index: Option<String>,
        label: Option<String>,
        ordered_path: Option<String>,
        first_episode_number: Option<String>,
        last_episode_number: Option<String>,
        monitored: Option<bool>,
    ) -> AppResult<Collection>;
    async fn set_collection_episodes_monitored(
        &self,
        collection_id: &str,
        monitored: bool,
    ) -> AppResult<()>;
    async fn delete_collection(&self, collection_id: &str) -> AppResult<()>;
    async fn list_episodes_for_collection(&self, collection_id: &str) -> AppResult<Vec<Episode>>;
    async fn get_episode_by_id(&self, episode_id: &str) -> AppResult<Option<Episode>>;
    async fn create_episode(&self, episode: Episode) -> AppResult<Episode>;
    async fn update_episode(
        &self,
        episode_id: &str,
        episode_type: Option<String>,
        episode_number: Option<String>,
        season_number: Option<String>,
        episode_label: Option<String>,
        title: Option<String>,
        air_date: Option<String>,
        duration_seconds: Option<i64>,
        has_multi_audio: Option<bool>,
        has_subtitle: Option<bool>,
        monitored: Option<bool>,
        collection_id: Option<String>,
    ) -> AppResult<Episode>;
    async fn delete_episode(&self, episode_id: &str) -> AppResult<()>;
    async fn find_episode_by_title_and_numbers(
        &self,
        title_id: &str,
        season_number: &str,
        episode_number: &str,
    ) -> AppResult<Option<Episode>>;
    async fn find_episode_by_title_and_absolute_number(
        &self,
        title_id: &str,
        absolute_number: &str,
    ) -> AppResult<Option<Episode>>;
    /// Fetch the primary (index=0) collection summary for a batch of title IDs.
    async fn list_primary_collection_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<PrimaryCollectionSummary>>;
}

#[async_trait]
pub trait UserRepository: Send + Sync {
    async fn get_by_username(&self, username: &str) -> AppResult<Option<User>>;
    async fn create(&self, user: User) -> AppResult<User>;
    async fn list_all(&self) -> AppResult<Vec<User>>;
    async fn get_by_id(&self, id: &str) -> AppResult<Option<User>>;
    async fn update_entitlements(
        &self,
        id: &str,
        entitlements: Vec<Entitlement>,
    ) -> AppResult<User>;
    async fn update_password_hash(&self, id: &str, password_hash: String) -> AppResult<User>;
    async fn delete(&self, id: &str) -> AppResult<()>;
}

#[async_trait]
pub trait EventRepository: Send + Sync {
    async fn list(
        &self,
        title_id: Option<String>,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<HistoryEvent>>;
    async fn append(&self, event: HistoryEvent) -> AppResult<()>;
}

#[async_trait]
pub trait IndexerConfigRepository: Send + Sync {
    async fn list(&self, provider_type: Option<String>) -> AppResult<Vec<IndexerConfig>>;
    async fn get_by_id(&self, id: &str) -> AppResult<Option<IndexerConfig>>;
    async fn create(&self, config: IndexerConfig) -> AppResult<IndexerConfig>;
    async fn touch_last_error(&self, provider_type: &str) -> AppResult<()>;
    async fn update(
        &self,
        id: &str,
        name: Option<String>,
        provider_type: Option<String>,
        base_url: Option<String>,
        api_key_encrypted: Option<String>,
        rate_limit_seconds: Option<i64>,
        rate_limit_burst: Option<i64>,
        is_enabled: Option<bool>,
        enable_interactive_search: Option<bool>,
        enable_auto_search: Option<bool>,
    ) -> AppResult<IndexerConfig>;
    async fn delete(&self, id: &str) -> AppResult<()>;
}

#[async_trait]
pub trait DownloadClientConfigRepository: Send + Sync {
    async fn list(&self, client_type: Option<String>) -> AppResult<Vec<DownloadClientConfig>>;
    async fn get_by_id(&self, id: &str) -> AppResult<Option<DownloadClientConfig>>;
    async fn create(&self, config: DownloadClientConfig) -> AppResult<DownloadClientConfig>;
    async fn update(
        &self,
        id: &str,
        name: Option<String>,
        client_type: Option<String>,
        base_url: Option<String>,
        config_json: Option<String>,
        is_enabled: Option<bool>,
    ) -> AppResult<DownloadClientConfig>;
    async fn delete(&self, id: &str) -> AppResult<()>;
}

#[async_trait]
pub trait SettingsRepository: Send + Sync {
    async fn get_setting_json(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
    ) -> AppResult<Option<String>>;
}

#[async_trait]
pub trait SystemInfoProvider: Send + Sync {
    async fn current_migration_version(&self) -> AppResult<Option<String>>;
    async fn pending_migration_count(&self) -> AppResult<usize>;
    async fn smg_cert_expires_at(&self) -> AppResult<Option<String>>;
}

/// Tracks per-indexer query counts and API quota information in memory.
pub trait IndexerStatsTracker: Send + Sync {
    fn record_query(&self, indexer_id: &str, indexer_name: &str, success: bool);
    fn record_api_limits(
        &self,
        indexer_id: &str,
        api_current: Option<u32>,
        api_max: Option<u32>,
        grab_current: Option<u32>,
        grab_max: Option<u32>,
    );
    fn all_stats(&self) -> Vec<IndexerQueryStats>;
}

#[async_trait]
pub trait QualityProfileRepository: Send + Sync {
    async fn list_quality_profiles(
        &self,
        scope: &str,
        scope_id: Option<String>,
    ) -> AppResult<Vec<QualityProfile>>;
}

#[async_trait]
pub trait ReleaseAttemptRepository: Send + Sync {
    async fn record_release_attempt(
        &self,
        title_id: Option<String>,
        source_hint: Option<String>,
        source_title: Option<String>,
        outcome: ReleaseDownloadAttemptOutcome,
        error_message: Option<String>,
        source_password: Option<String>,
    ) -> AppResult<()>;

    async fn list_failed_release_signatures(
        &self,
        limit: usize,
    ) -> AppResult<Vec<ReleaseDownloadFailureSignature>>;

    async fn list_failed_release_signatures_for_title(
        &self,
        title_id: &str,
        limit: usize,
    ) -> AppResult<Vec<TitleReleaseBlocklistEntry>>;

    async fn get_latest_source_password(
        &self,
        title_id: Option<&str>,
        source_hint: Option<&str>,
        source_title: Option<&str>,
    ) -> AppResult<Option<String>>;
}

#[async_trait]
pub trait ImportRepository: Send + Sync {
    async fn queue_import_request(
        &self,
        source_system: String,
        source_ref: String,
        import_type: String,
        payload_json: String,
    ) -> AppResult<String>;

    async fn get_import_by_source_ref(
        &self,
        source_system: &str,
        source_ref: &str,
    ) -> AppResult<Option<ImportRecord>>;

    async fn update_import_status(
        &self,
        import_id: &str,
        status: &str,
        result_json: Option<String>,
    ) -> AppResult<()>;

    async fn recover_stale_processing_imports(&self, stale_seconds: i64) -> AppResult<u64>;

    async fn list_pending_imports(&self) -> AppResult<Vec<ImportRecord>>;

    async fn is_already_imported(
        &self,
        source_system: &str,
        source_ref: &str,
    ) -> AppResult<bool>;

    async fn list_imports(&self, limit: usize) -> AppResult<Vec<ImportRecord>>;
}

#[async_trait]
pub trait FileImporter: Send + Sync {
    async fn import_file(
        &self,
        source: &Path,
        dest: &Path,
    ) -> AppResult<ImportFileResult>;
}

#[async_trait]
pub trait MediaFileRepository: Send + Sync {
    async fn insert_media_file(
        &self,
        title_id: &str,
        file_path: &str,
        size_bytes: i64,
        quality_label: Option<String>,
    ) -> AppResult<String>;

    async fn link_file_to_episode(
        &self,
        file_id: &str,
        episode_id: &str,
    ) -> AppResult<()>;

    async fn list_media_files_for_title(
        &self,
        title_id: &str,
    ) -> AppResult<Vec<TitleMediaFile>>;
}

#[async_trait]
pub trait WantedItemRepository: Send + Sync {
    async fn upsert_wanted_item(&self, item: &WantedItem) -> AppResult<String>;
    async fn list_due_wanted_items(&self, now: &str, batch_limit: i64) -> AppResult<Vec<WantedItem>>;
    async fn update_wanted_item_status(
        &self,
        id: &str,
        status: &str,
        next_search_at: Option<&str>,
        last_search_at: Option<&str>,
        search_count: i64,
        current_score: Option<i32>,
        grabbed_release: Option<&str>,
    ) -> AppResult<()>;
    async fn get_wanted_item_for_title(
        &self,
        title_id: &str,
        episode_id: Option<&str>,
    ) -> AppResult<Option<WantedItem>>;
    async fn delete_wanted_items_for_title(&self, title_id: &str) -> AppResult<()>;
    async fn insert_release_decision(&self, decision: &ReleaseDecision) -> AppResult<String>;
    async fn get_wanted_item_by_id(&self, id: &str) -> AppResult<Option<WantedItem>>;
    async fn list_wanted_items(
        &self,
        status: Option<&str>,
        media_type: Option<&str>,
        title_id: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<WantedItem>>;
    async fn count_wanted_items(
        &self,
        status: Option<&str>,
        media_type: Option<&str>,
        title_id: Option<&str>,
    ) -> AppResult<i64>;
    async fn list_release_decisions_for_title(
        &self,
        title_id: &str,
        limit: i64,
    ) -> AppResult<Vec<ReleaseDecision>>;
    async fn list_release_decisions_for_wanted_item(
        &self,
        wanted_item_id: &str,
        limit: i64,
    ) -> AppResult<Vec<ReleaseDecision>>;
}

#[async_trait]
pub trait RuleSetRepository: Send + Sync {
    async fn list_rule_sets(&self) -> AppResult<Vec<RuleSet>>;
    async fn list_enabled_rule_sets(&self) -> AppResult<Vec<RuleSet>>;
    async fn get_rule_set(&self, id: &str) -> AppResult<Option<RuleSet>>;
    async fn create_rule_set(&self, rule_set: &RuleSet) -> AppResult<()>;
    async fn update_rule_set(&self, rule_set: &RuleSet) -> AppResult<()>;
    async fn delete_rule_set(&self, id: &str) -> AppResult<()>;
    async fn record_rule_set_history(
        &self,
        rule_set_id: &str,
        action: &str,
        rego_source: Option<&str>,
        actor_id: Option<&str>,
    ) -> AppResult<()>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchMode {
    Interactive,
    Auto,
}

/// Per-indexer routing entry resolved from the `indexer.routing:<scope>` setting.
#[derive(Clone, Debug)]
pub struct IndexerRoutingEntry {
    pub enabled: bool,
    pub categories: Vec<String>,
}

/// Per-indexer routing plan for a given facet scope.
/// When `Some`, indexers not in the map use default behavior; indexers
/// with `enabled: false` are skipped entirely for this scope.
#[derive(Clone, Debug)]
pub struct IndexerRoutingPlan {
    pub entries: std::collections::HashMap<String, IndexerRoutingEntry>,
}

#[async_trait]
pub trait IndexerClient: Send + Sync {
    async fn search(
        &self,
        query: String,
        imdb_id: Option<String>,
        tvdb_id: Option<String>,
        category: Option<String>,
        newznab_categories: Option<Vec<String>>,
        indexer_routing: Option<IndexerRoutingPlan>,
        limit: usize,
        mode: SearchMode,
    ) -> AppResult<Vec<IndexerSearchResult>>;
}

#[async_trait]
pub trait DownloadClient: Send + Sync {
    async fn submit_to_download_queue(
        &self,
        title: &Title,
        source_hint: Option<String>,
        source_title: Option<String>,
        source_password: Option<String>,
        category: Option<String>,
    ) -> AppResult<String>;

    async fn list_queue(&self) -> AppResult<Vec<DownloadQueueItem>> {
        Err(AppError::Repository(
            "download queue listing is not supported for this client".to_string(),
        ))
    }

    async fn list_history(&self) -> AppResult<Vec<DownloadQueueItem>> {
        Err(AppError::Repository(
            "download history listing is not supported for this client".to_string(),
        ))
    }

    async fn list_completed_downloads(&self) -> AppResult<Vec<CompletedDownload>> {
        Err(AppError::Repository(
            "completed download listing is not supported for this client".to_string(),
        ))
    }

    async fn pause_queue_item(&self, _id: &str) -> AppResult<()> {
        Err(AppError::Repository(
            "pause is not supported for this download client".to_string(),
        ))
    }

    async fn resume_queue_item(&self, _id: &str) -> AppResult<()> {
        Err(AppError::Repository(
            "resume is not supported for this download client".to_string(),
        ))
    }

    async fn delete_queue_item(&self, _id: &str, _is_history: bool) -> AppResult<()> {
        Err(AppError::Repository(
            "delete is not supported for this download client".to_string(),
        ))
    }
}

#[derive(Clone)]
pub struct AppUseCase {
    pub services: AppServices,
    pub auth: JwtAuthConfig,
    pub facet_registry: Arc<FacetRegistry>,
}


#[allow(dead_code)]
fn is_release_blocklisted(
    result: &IndexerSearchResult,
    failed_source_hints: &std::collections::HashSet<String>,
    failed_source_titles: &std::collections::HashSet<String>,
) -> bool {
    if let Some(download_url) = normalize_release_attempt_hint(result.download_url.as_deref()) {
        if failed_source_hints.contains(&download_url) {
            return true;
        }
    }

    if let Some(link) = normalize_release_attempt_hint(result.link.as_deref()) {
        if failed_source_hints.contains(&link) {
            return true;
        }
    }

    if let Some(title) = normalize_release_attempt_title(Some(result.title.as_str())) {
        if failed_source_titles.contains(&title) {
            return true;
        }
    }

    false
}

#[allow(dead_code)]
fn normalize_release_attempt_hint(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[allow(dead_code)]
fn normalize_release_attempt_title(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
}

#[allow(dead_code)]
fn normalize_release_password(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty() && *value != "0")
        .map(str::to_string)
}

#[allow(dead_code)]
async fn lookup_latest_source_password(
    release_attempts: &dyn ReleaseAttemptRepository,
    title_id: Option<&str>,
    source_hint: Option<&str>,
    source_title: Option<&str>,
) -> Option<String> {
    if let Some(title_id) = title_id {
        if let Ok(Some(password)) = release_attempts
            .get_latest_source_password(Some(title_id), source_hint, source_title)
            .await
        {
            return normalize_release_password(Some(password.as_str()));
        }
    }

    let password = release_attempts
        .get_latest_source_password(None, source_hint, source_title)
        .await
        .ok()
        .flatten();
    normalize_release_password(password.as_deref())
}

pub(crate) fn require(actor: &User, entitlement: &Entitlement) -> AppResult<()> {
    if actor.has_entitlement(entitlement) {
        Ok(())
    } else {
        Err(AppError::Unauthorized(format!(
            "user {} lacks {:?}",
            actor.username, entitlement
        )))
    }
}

fn sha256_hex(input: impl AsRef<str>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_ref().as_bytes());
    let digest = hasher.finalize();
    to_hex(&digest)
}

fn to_hex(value: &[u8]) -> String {
    let mut output = String::with_capacity(value.len() * 2);
    for byte in value {
        output.push_str(&format!("{byte:02x}"));
    }
    output
}

fn normalize_tag(raw: String) -> String {
    raw.trim().to_lowercase()
}

fn normalize_show_text(raw: String) -> Option<String> {
    let value = raw.trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn normalize_show_text_opt(raw: Option<String>) -> Option<String> {
    raw.and_then(normalize_show_text)
}

fn normalize_tags(raw: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for value in raw {
        let normalized = normalize_tag(value.clone());
        if normalized.is_empty() {
            continue;
        }
        if seen.insert(normalized.clone()) {
            out.push(normalized);
        }
    }
    out
}

#[allow(dead_code)]
fn normalize_release_attempt_value(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn sanitize_ids(ids: Vec<ExternalId>) -> Vec<ExternalId> {
    ids.into_iter()
        .filter_map(|id| {
            let source = id.source.trim().to_lowercase();
            let value = id.value.trim().to_string();
            if source.is_empty() || value.is_empty() {
                None
            } else {
                Some(ExternalId { source, value })
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use p256::pkcs8::{EncodePrivateKey, EncodePublicKey};
    use tokio::sync::Mutex;

    #[derive(Default)]
    struct MockTitleRepo {
        store: Arc<Mutex<Vec<Title>>>,
    }

    #[async_trait]
    impl TitleRepository for MockTitleRepo {
        async fn list(
            &self,
            facet: Option<MediaFacet>,
            query: Option<String>,
        ) -> AppResult<Vec<Title>> {
            let list = self.store.lock().await.clone();
            let normalized_query = query.map(|value| value.to_lowercase());
            Ok(list
                .into_iter()
                .filter(|title| {
                    let facet_match = facet
                        .as_ref()
                        .is_none_or(|expected| &title.facet == expected);
                    let query_match = normalized_query
                        .as_ref()
                        .is_none_or(|term| title.name.to_lowercase().contains(term));
                    facet_match && query_match
                })
                .collect())
        }

        async fn get_by_id(&self, id: &str) -> AppResult<Option<Title>> {
            let list = self.store.lock().await;
            Ok(list.iter().find(|title| title.id == id).cloned())
        }

        async fn create(&self, title: Title) -> AppResult<Title> {
            self.store.lock().await.push(title.clone());
            Ok(title)
        }

        async fn update_metadata(
            &self,
            id: &str,
            name: Option<String>,
            facet: Option<MediaFacet>,
            tags: Option<Vec<String>>,
        ) -> AppResult<Title> {
            let mut list = self.store.lock().await;
            let title = list
                .iter_mut()
                .find(|entry| entry.id == id)
                .ok_or_else(|| AppError::NotFound(format!("title {}", id)))?;

            if let Some(name) = name {
                let normalized = name.trim();
                if normalized.is_empty() {
                    return Err(AppError::Validation("title name cannot be empty".into()));
                }
                title.name = normalized.to_string();
            }

            if let Some(facet) = facet {
                title.facet = facet;
            }

            if let Some(tags) = tags {
                title.tags = normalize_tags(&tags);
            }

            Ok(title.clone())
        }

        async fn update_monitored(&self, id: &str, monitored: bool) -> AppResult<Title> {
            let mut list = self.store.lock().await;
            let title = list
                .iter_mut()
                .find(|entry| entry.id == id)
                .ok_or_else(|| AppError::NotFound(format!("title {}", id)))?;
            title.monitored = monitored;
            Ok(title.clone())
        }

        async fn update_title_hydrated_metadata(
            &self,
            id: &str,
            metadata: TitleMetadataUpdate,
        ) -> AppResult<Title> {
            let mut list = self.store.lock().await;
            let title = list
                .iter_mut()
                .find(|entry| entry.id == id)
                .ok_or_else(|| AppError::NotFound(format!("title {}", id)))?;
            title.year = metadata.year;
            title.overview = metadata.overview;
            title.poster_url = metadata.poster_url;
            title.sort_title = metadata.sort_title;
            title.slug = metadata.slug;
            title.imdb_id = metadata.imdb_id;
            title.runtime_minutes = metadata.runtime_minutes;
            title.genres = metadata.genres;
            title.content_status = metadata.content_status;
            title.language = metadata.language;
            title.first_aired = metadata.first_aired;
            title.network = metadata.network;
            title.studio = metadata.studio;
            title.country = metadata.country;
            title.aliases = metadata.aliases;
            title.metadata_language = metadata.metadata_language;
            Ok(title.clone())
        }

        async fn delete(&self, id: &str) -> AppResult<()> {
            let mut list = self.store.lock().await;
            let position = list
                .iter()
                .position(|entry| entry.id == id)
                .ok_or_else(|| AppError::NotFound(format!("title {}", id)))?;
            list.remove(position);
            Ok(())
        }
    }

    #[derive(Default)]
    struct MockUserRepo {
        store: Arc<Mutex<Vec<User>>>,
    }

    #[async_trait]
    impl UserRepository for MockUserRepo {
        async fn get_by_username(&self, username: &str) -> AppResult<Option<User>> {
            let users = self.store.lock().await;
            Ok(users.iter().find(|user| user.username == username).cloned())
        }

        async fn get_by_id(&self, id: &str) -> AppResult<Option<User>> {
            let users = self.store.lock().await;
            Ok(users.iter().find(|user| user.id == id).cloned())
        }

        async fn create(&self, user: User) -> AppResult<User> {
            self.store.lock().await.push(user.clone());
            Ok(user)
        }

        async fn list_all(&self) -> AppResult<Vec<User>> {
            Ok(self.store.lock().await.clone())
        }

        async fn update_entitlements(
            &self,
            id: &str,
            entitlements: Vec<Entitlement>,
        ) -> AppResult<User> {
            let mut users = self.store.lock().await;
            let user = users
                .iter_mut()
                .find(|entry| entry.id == id)
                .ok_or_else(|| AppError::NotFound(format!("user {}", id)))?;
            user.entitlements = entitlements;
            Ok(user.clone())
        }

        async fn update_password_hash(&self, id: &str, password_hash: String) -> AppResult<User> {
            let mut users = self.store.lock().await;
            let user = users
                .iter_mut()
                .find(|entry| entry.id == id)
                .ok_or_else(|| AppError::NotFound(format!("user {}", id)))?;
            user.password_hash = Some(password_hash);
            Ok(user.clone())
        }

        async fn delete(&self, id: &str) -> AppResult<()> {
            let mut users = self.store.lock().await;
            let index = users
                .iter()
                .position(|entry| entry.id == id)
                .ok_or_else(|| AppError::NotFound(format!("user {}", id)))?;
            users.remove(index);
            Ok(())
        }
    }

    #[derive(Default)]
    struct MockEventRepo {
        store: Arc<Mutex<Vec<HistoryEvent>>>,
    }

    #[derive(Default)]
    struct MockShowRepo {
        collections: Arc<Mutex<Vec<Collection>>>,
        episodes: Arc<Mutex<Vec<Episode>>>,
    }

    #[async_trait]
    impl ShowRepository for MockShowRepo {
        async fn list_collections_for_title(&self, title_id: &str) -> AppResult<Vec<Collection>> {
            let collections = self.collections.lock().await;
            Ok(collections
                .iter()
                .filter(|item| item.title_id == title_id)
                .cloned()
                .collect())
        }

        async fn get_collection_by_id(&self, collection_id: &str) -> AppResult<Option<Collection>> {
            let collections = self.collections.lock().await;
            Ok(collections
                .iter()
                .find(|item| item.id == collection_id)
                .cloned())
        }

        async fn create_collection(&self, collection: Collection) -> AppResult<Collection> {
            self.collections.lock().await.push(collection.clone());
            Ok(collection)
        }

        async fn update_collection(
            &self,
            collection_id: &str,
            collection_type: Option<String>,
            collection_index: Option<String>,
            label: Option<String>,
            ordered_path: Option<String>,
            first_episode_number: Option<String>,
            last_episode_number: Option<String>,
            monitored: Option<bool>,
        ) -> AppResult<Collection> {
            let mut collections = self.collections.lock().await;
            let item = collections
                .iter_mut()
                .find(|entry| entry.id == collection_id)
                .ok_or_else(|| AppError::NotFound(format!("collection {}", collection_id)))?;

            if let Some(value) = collection_type {
                item.collection_type = value;
            }
            if let Some(value) = collection_index {
                item.collection_index = value;
            }
            if let Some(value) = label {
                item.label = Some(value);
            }
            if let Some(value) = ordered_path {
                item.ordered_path = Some(value);
            }
            if let Some(value) = first_episode_number {
                item.first_episode_number = Some(value);
            }
            if let Some(value) = last_episode_number {
                item.last_episode_number = Some(value);
            }
            if let Some(value) = monitored {
                item.monitored = value;
            }

            Ok(item.clone())
        }

        async fn set_collection_episodes_monitored(
            &self,
            collection_id: &str,
            monitored: bool,
        ) -> AppResult<()> {
            let mut episodes = self.episodes.lock().await;
            for episode in episodes.iter_mut() {
                if episode.collection_id.as_deref() == Some(collection_id) {
                    episode.monitored = monitored;
                }
            }
            Ok(())
        }

        async fn delete_collection(&self, collection_id: &str) -> AppResult<()> {
            let mut collections = self.collections.lock().await;
            let index = collections
                .iter()
                .position(|item| item.id == collection_id)
                .ok_or_else(|| AppError::NotFound(format!("collection {}", collection_id)))?;
            collections.remove(index);

            let mut episodes = self.episodes.lock().await;
            for episode in episodes.iter_mut() {
                if episode.collection_id.as_deref() == Some(collection_id) {
                    episode.collection_id = None;
                }
            }
            Ok(())
        }

        async fn list_episodes_for_collection(
            &self,
            collection_id: &str,
        ) -> AppResult<Vec<Episode>> {
            let episodes = self.episodes.lock().await;
            Ok(episodes
                .iter()
                .filter(|item| item.collection_id.as_deref() == Some(collection_id))
                .cloned()
                .collect())
        }

        async fn get_episode_by_id(&self, episode_id: &str) -> AppResult<Option<Episode>> {
            let episodes = self.episodes.lock().await;
            Ok(episodes.iter().find(|item| item.id == episode_id).cloned())
        }

        async fn create_episode(&self, episode: Episode) -> AppResult<Episode> {
            self.episodes.lock().await.push(episode.clone());
            Ok(episode)
        }

        async fn update_episode(
            &self,
            episode_id: &str,
            episode_type: Option<String>,
            episode_number: Option<String>,
            season_number: Option<String>,
            episode_label: Option<String>,
            title: Option<String>,
            air_date: Option<String>,
            duration_seconds: Option<i64>,
            has_multi_audio: Option<bool>,
            has_subtitle: Option<bool>,
            monitored: Option<bool>,
            collection_id: Option<String>,
        ) -> AppResult<Episode> {
            let mut episodes = self.episodes.lock().await;
            let item = episodes
                .iter_mut()
                .find(|entry| entry.id == episode_id)
                .ok_or_else(|| AppError::NotFound(format!("episode {}", episode_id)))?;

            if let Some(value) = episode_type {
                item.episode_type = value;
            }
            if let Some(value) = episode_number {
                item.episode_number = Some(value);
            }
            if let Some(value) = season_number {
                item.season_number = Some(value);
            }
            if let Some(value) = episode_label {
                item.episode_label = Some(value);
            }
            if let Some(value) = title {
                item.title = Some(value);
            }
            if let Some(value) = air_date {
                item.air_date = Some(value);
            }
            if let Some(value) = duration_seconds {
                item.duration_seconds = Some(value);
            }
            if let Some(value) = has_multi_audio {
                item.has_multi_audio = value;
            }
            if let Some(value) = has_subtitle {
                item.has_subtitle = value;
            }
            if let Some(value) = monitored {
                item.monitored = value;
            }
            if let Some(value) = collection_id {
                item.collection_id = Some(value);
            }

            Ok(item.clone())
        }

        async fn delete_episode(&self, episode_id: &str) -> AppResult<()> {
            let mut episodes = self.episodes.lock().await;
            let index = episodes
                .iter()
                .position(|item| item.id == episode_id)
                .ok_or_else(|| AppError::NotFound(format!("episode {}", episode_id)))?;
            episodes.remove(index);
            Ok(())
        }

        async fn find_episode_by_title_and_numbers(
            &self,
            title_id: &str,
            _season_number: &str,
            episode_number: &str,
        ) -> AppResult<Option<Episode>> {
            let episodes = self.episodes.lock().await;
            Ok(episodes
                .iter()
                .find(|ep| {
                    ep.title_id == title_id
                        && ep.episode_number.as_deref() == Some(episode_number)
                })
                .cloned())
        }

        async fn find_episode_by_title_and_absolute_number(
            &self,
            title_id: &str,
            absolute_number: &str,
        ) -> AppResult<Option<Episode>> {
            let episodes = self.episodes.lock().await;
            Ok(episodes
                .iter()
                .find(|ep| {
                    ep.title_id == title_id
                        && ep.absolute_number.as_deref() == Some(absolute_number)
                })
                .cloned())
        }

        async fn list_primary_collection_summaries(
            &self,
            title_ids: &[String],
        ) -> AppResult<Vec<PrimaryCollectionSummary>> {
            let collections = self.collections.lock().await;
            let mut out = Vec::new();
            for tid in title_ids {
                if let Some(c) = collections.iter().find(|c| c.title_id == *tid && c.collection_index == "0") {
                    out.push(PrimaryCollectionSummary {
                        title_id: tid.clone(),
                        label: c.label.clone(),
                        ordered_path: c.ordered_path.clone(),
                    });
                }
            }
            Ok(out)
        }
    }

    #[derive(Default)]
    struct MockIndexerClient;

    #[async_trait]
    impl IndexerClient for MockIndexerClient {
        async fn search(
            &self,
            query: String,
            imdb_id: Option<String>,
            tvdb_id: Option<String>,
            category: Option<String>,
            _newznab_categories: Option<Vec<String>>,
            _indexer_routing: Option<IndexerRoutingPlan>,
            _limit: usize,
            _mode: SearchMode,
        ) -> AppResult<Vec<IndexerSearchResult>> {
            if let Some(tvdb) = tvdb_id {
                tracing::info!(tvdb_id = %tvdb, category = ?category, "mock nzbgeek search");
            }
            if let Some(imdb) = imdb_id {
                tracing::info!(imdb_id = %imdb, category = ?category, "mock nzbgeek search");
            }
            Ok(vec![IndexerSearchResult {
                source: "nzbgeek".into(),
                title: format!("match for {query}"),
                link: None,
                download_url: None,
                size_bytes: None,
                published_at: Some("1970-01-01T00:00:00Z".into()),
                thumbs_up: None,
                thumbs_down: None,
                nzbgeek_languages: None,
                nzbgeek_subtitles: None,
                nzbgeek_grabs: None,
                nzbgeek_password_protected: None,
                parsed_release_metadata: None,
                quality_profile_decision: None,
            }])
        }
    }

    #[derive(Default)]
    struct MockIndexerConfigRepo {
        store: Arc<Mutex<Vec<IndexerConfig>>>,
    }

    #[derive(Default)]
    struct MockSettingsRepo;

    #[async_trait]
    impl SettingsRepository for MockSettingsRepo {
        async fn get_setting_json(
            &self,
            _scope: &str,
            _key_name: &str,
            _scope_id: Option<String>,
        ) -> AppResult<Option<String>> {
            Ok(None)
        }
    }

    #[derive(Default)]
    struct MockQualityProfileRepo;

    #[async_trait]
    impl QualityProfileRepository for MockQualityProfileRepo {
        async fn list_quality_profiles(
            &self,
            _scope: &str,
            _scope_id: Option<String>,
        ) -> AppResult<Vec<QualityProfile>> {
            Ok(vec![])
        }
    }

    #[async_trait]
    impl IndexerConfigRepository for MockIndexerConfigRepo {
        async fn list(&self, provider_filter: Option<String>) -> AppResult<Vec<IndexerConfig>> {
            let entries = self.store.lock().await;
            Ok(entries
                .iter()
                .filter(|entry| {
                    provider_filter
                        .as_ref()
                        .is_none_or(|provider| provider == &entry.provider_type)
                })
                .cloned()
                .collect())
        }

        async fn get_by_id(&self, id: &str) -> AppResult<Option<IndexerConfig>> {
            let entries = self.store.lock().await;
            Ok(entries.iter().find(|entry| entry.id == id).cloned())
        }

        async fn touch_last_error(&self, provider_type: &str) -> AppResult<()> {
            let mut entries = self.store.lock().await;
            let now = Utc::now();
            for entry in entries.iter_mut() {
                if entry.provider_type == provider_type {
                    entry.last_error_at = Some(now);
                    entry.updated_at = now;
                }
            }
            Ok(())
        }

        async fn create(&self, config: IndexerConfig) -> AppResult<IndexerConfig> {
            let mut entries = self.store.lock().await;
            entries.push(config.clone());
            Ok(config)
        }

        async fn update(
            &self,
            id: &str,
            name: Option<String>,
            provider_type: Option<String>,
            base_url: Option<String>,
            api_key_encrypted: Option<String>,
            rate_limit_seconds: Option<i64>,
            rate_limit_burst: Option<i64>,
            is_enabled: Option<bool>,
            enable_interactive_search: Option<bool>,
            enable_auto_search: Option<bool>,
        ) -> AppResult<IndexerConfig> {
            let mut entries = self.store.lock().await;
            let item = entries
                .iter_mut()
                .find(|entry| entry.id == id)
                .ok_or_else(|| AppError::NotFound(format!("indexer config {}", id)))?;

            if let Some(name) = name {
                item.name = name;
            }
            if let Some(provider_type) = provider_type {
                item.provider_type = provider_type;
            }
            if let Some(base_url) = base_url {
                item.base_url = base_url;
            }
            if let Some(api_key_encrypted) = api_key_encrypted {
                item.api_key_encrypted = Some(api_key_encrypted);
            }
            if let Some(rate_limit_seconds) = rate_limit_seconds {
                item.rate_limit_seconds = Some(rate_limit_seconds);
            }
            if let Some(rate_limit_burst) = rate_limit_burst {
                item.rate_limit_burst = Some(rate_limit_burst);
            }
            if let Some(is_enabled) = is_enabled {
                item.is_enabled = is_enabled;
            }
            if let Some(enable_interactive_search) = enable_interactive_search {
                item.enable_interactive_search = enable_interactive_search;
            }
            if let Some(enable_auto_search) = enable_auto_search {
                item.enable_auto_search = enable_auto_search;
            }
            item.updated_at = Utc::now();

            Ok(item.clone())
        }

        async fn delete(&self, id: &str) -> AppResult<()> {
            let mut entries = self.store.lock().await;
            let position = entries
                .iter()
                .position(|entry| entry.id == id)
                .ok_or_else(|| AppError::NotFound(format!("indexer config {}", id)))?;
            entries.remove(position);
            Ok(())
        }
    }

    #[derive(Default)]
    struct MockDownloadClientConfigRepo {
        store: Arc<Mutex<Vec<DownloadClientConfig>>>,
    }

    #[async_trait]
    impl DownloadClientConfigRepository for MockDownloadClientConfigRepo {
        async fn list(&self, client_type: Option<String>) -> AppResult<Vec<DownloadClientConfig>> {
            let entries = self.store.lock().await;
            Ok(entries
                .iter()
                .filter(|entry| {
                    client_type
                        .as_ref()
                        .is_none_or(|client_type| client_type == &entry.client_type)
                })
                .cloned()
                .collect())
        }

        async fn get_by_id(&self, id: &str) -> AppResult<Option<DownloadClientConfig>> {
            let entries = self.store.lock().await;
            Ok(entries.iter().find(|entry| entry.id == id).cloned())
        }

        async fn create(&self, config: DownloadClientConfig) -> AppResult<DownloadClientConfig> {
            let mut entries = self.store.lock().await;
            entries.push(config.clone());
            Ok(config)
        }

        async fn update(
            &self,
            id: &str,
            name: Option<String>,
            client_type: Option<String>,
            base_url: Option<String>,
            config_json: Option<String>,
            is_enabled: Option<bool>,
        ) -> AppResult<DownloadClientConfig> {
            let mut entries = self.store.lock().await;
            let item = entries
                .iter_mut()
                .find(|entry| entry.id == id)
                .ok_or_else(|| AppError::NotFound(format!("download client config {id}")))?;

            if let Some(name) = name {
                item.name = name;
            }
            if let Some(client_type) = client_type {
                item.client_type = client_type;
            }
            if let Some(base_url) = base_url {
                item.base_url = Some(base_url);
            }
            if let Some(config_json) = config_json {
                item.config_json = config_json;
            }
            if let Some(is_enabled) = is_enabled {
                item.is_enabled = is_enabled;
            }
            item.updated_at = Utc::now();

            Ok(item.clone())
        }

        async fn delete(&self, id: &str) -> AppResult<()> {
            let mut entries = self.store.lock().await;
            let position = entries
                .iter()
                .position(|entry| entry.id == id)
                .ok_or_else(|| AppError::NotFound(format!("download client config {id}")))?;
            entries.remove(position);
            Ok(())
        }
    }

    #[derive(Default)]
    struct MockReleaseAttemptRepo;

    #[async_trait]
    impl ReleaseAttemptRepository for MockReleaseAttemptRepo {
        async fn record_release_attempt(
            &self,
            _title_id: Option<String>,
            _source_hint: Option<String>,
            _source_title: Option<String>,
            _outcome: ReleaseDownloadAttemptOutcome,
            _error_message: Option<String>,
            _source_password: Option<String>,
        ) -> AppResult<()> {
            Ok(())
        }

        async fn list_failed_release_signatures(
            &self,
            _limit: usize,
        ) -> AppResult<Vec<ReleaseDownloadFailureSignature>> {
            Ok(vec![])
        }

        async fn list_failed_release_signatures_for_title(
            &self,
            _title_id: &str,
            _limit: usize,
        ) -> AppResult<Vec<TitleReleaseBlocklistEntry>> {
            Ok(vec![])
        }

        async fn get_latest_source_password(
            &self,
            _title_id: Option<&str>,
            _source_hint: Option<&str>,
            _source_title: Option<&str>,
        ) -> AppResult<Option<String>> {
            Ok(None)
        }
    }

    #[async_trait]
    impl EventRepository for MockEventRepo {
        async fn list(
            &self,
            title_id: Option<String>,
            limit: i64,
            offset: i64,
        ) -> AppResult<Vec<HistoryEvent>> {
            let mut events = self.store.lock().await.clone();
            if let Some(id) = title_id {
                events = events
                    .into_iter()
                    .filter(|event| event.title_id.as_ref() == Some(&id))
                    .collect();
            }
            let start = usize::try_from(offset.max(0)).unwrap_or(0);
            let end = start.saturating_add(usize::try_from(limit.max(0)).unwrap_or(0));
            Ok(events
                .into_iter()
                .skip(start)
                .take(end.saturating_sub(start))
                .collect())
        }

        async fn append(&self, event: HistoryEvent) -> AppResult<()> {
            self.store.lock().await.push(event);
            Ok(())
        }
    }

    struct StubDownloadClient;

    #[async_trait]
    impl DownloadClient for StubDownloadClient {
    async fn submit_to_download_queue(
        &self,
        title: &Title,
        _source_hint: Option<String>,
        _source_title: Option<String>,
        _source_password: Option<String>,
        _category: Option<String>,
    ) -> AppResult<String> {
            Ok(format!("job-for-{}", title.id))
        }
    }

    fn bootstrap() -> (AppUseCase, User) {
        let titles = Arc::new(MockTitleRepo::default());
        let shows = Arc::new(MockShowRepo::default());
        let users = Arc::new(MockUserRepo::default());
        let events = Arc::new(MockEventRepo::default());
        let indexer_configs = Arc::new(MockIndexerConfigRepo::default());
        let download_client_configs = Arc::new(MockDownloadClientConfigRepo::default());
        let release_attempts = Arc::new(MockReleaseAttemptRepo);
        let settings = Arc::new(MockSettingsRepo);
        let quality_profiles = Arc::new(MockQualityProfileRepo);
        let download_client = Arc::new(StubDownloadClient);
        let indexer_client = Arc::new(MockIndexerClient::default());

        let services = AppServices::with_default_channels(
            titles,
            shows,
            users,
            events,
            indexer_configs,
            indexer_client,
            download_client,
            download_client_configs,
            release_attempts,
            settings,
            quality_profiles,
            String::new(),
        );
        let signing_key = p256::ecdsa::SigningKey::random(&mut rand_core::OsRng);
        let jwt_ec_private_pem = signing_key
            .to_pkcs8_pem(p256::pkcs8::LineEnding::LF)
            .expect("test EC private key")
            .to_string();
        let jwt_ec_public_pem = signing_key
            .verifying_key()
            .to_public_key_pem(p256::pkcs8::LineEnding::LF)
            .expect("test EC public key");
        let mut registry = FacetRegistry::new();
        registry.register(Arc::new(MovieFacetHandler));
        registry.register(Arc::new(SeriesFacetHandler::new(scryer_domain::MediaFacet::Tv)));
        registry.register(Arc::new(SeriesFacetHandler::new(scryer_domain::MediaFacet::Anime)));
        let app = AppUseCase::new(
            services,
            JwtAuthConfig {
                issuer: "scryer-test".to_string(),
                access_ttl_seconds: 3600,
                jwt_ec_private_pem,
                jwt_ec_public_pem,
            },
            Arc::new(registry),
        );

        (app, User::new_admin("admin"))
    }

    #[tokio::test]
    async fn add_title_and_queue_sends_download_job() {
        let (app, user) = bootstrap();
        let (title, job_id) = app
            .add_title_and_queue_download(
                &user,
                NewTitle {
                    name: "Show One".into(),
                    facet: MediaFacet::Tv,
                    monitored: true,
                    tags: vec![],
                    external_ids: vec![],
                    min_availability: None,
                },
                None,
                None,
            )
            .await
            .expect("title + queue should succeed");

        assert_eq!(job_id, format!("job-for-{}", title.id));
    }

    #[tokio::test]
    async fn search_titles_supports_facet_filter() {
        let (app, user) = bootstrap();

        app.add_title(
            &user,
            NewTitle {
                name: "Movie A".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
            },
        )
        .await
        .expect("create movie");

        app.add_title(
            &user,
            NewTitle {
                name: "Show B".into(),
                facet: MediaFacet::Tv,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
            },
        )
        .await
        .expect("create tv");

        let tvs = app
            .list_titles(&user, Some(MediaFacet::Tv), None)
            .await
            .expect("list titles");

        assert!(tvs.iter().all(|item| item.facet == MediaFacet::Tv));
    }

    #[tokio::test]
    async fn search_indexer_requires_query() {
        let (app, user) = bootstrap();

        let result = app
            .search_indexers(&user, "   ".into(), None, None, None, 10)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn create_user_and_list_users() {
        let (app, user) = bootstrap();

        let created = app
            .create_user(
                &user,
                "editor".into(),
                "password123".to_string(),
                vec![Entitlement::ViewCatalog, Entitlement::ManageTitle],
            )
            .await
            .expect("create user");

        let users = app.list_users(&user).await.expect("list users");
        assert!(users.iter().any(|entry| entry.username == created.username));
        assert_eq!(users.len(), 1);
    }

    #[tokio::test]
    async fn get_user_by_id_returns_created_user() {
        let (app, user) = bootstrap();

        let created = app
            .create_user(
                &user,
                "viewer".into(),
                "password123".to_string(),
                vec![Entitlement::ViewCatalog],
            )
            .await
            .expect("create user");

        let found = app.get_user(&user, &created.id).await.expect("get user");

        assert!(found.is_some());
        let found = found.expect("user should exist");
        assert_eq!(found.id, created.id);
        assert_eq!(found.username, "viewer");
    }

    #[tokio::test]
    async fn create_user_rejects_duplicate_username() {
        let (app, user) = bootstrap();

        let _created = app
            .create_user(
                &user,
                "editor".into(),
                "password123".to_string(),
                vec![Entitlement::ViewCatalog],
            )
            .await
            .expect("first create");

        let second = app
            .create_user(
                &user,
                "editor".into(),
                "password123".to_string(),
                vec![Entitlement::ViewCatalog],
            )
            .await;

        assert!(second.is_err());
    }

    #[tokio::test]
    async fn delete_title_removes_title_from_catalog() {
        let (app, user) = bootstrap();

        let created = app
            .add_title(
                &user,
                NewTitle {
                    name: "Delete Me".into(),
                    facet: MediaFacet::Movie,
                    monitored: false,
                    tags: vec![],
                    external_ids: vec![],
                    min_availability: None,
                },
            )
            .await
            .expect("create title");

        app.delete_title(&user, &created.id, false)
            .await
            .expect("delete title");

        let titles = app
            .list_titles(&user, Some(MediaFacet::Movie), None)
            .await
            .expect("list titles");
        assert!(titles.is_empty());
    }

    #[tokio::test]
    async fn update_user_entitlements_changes_permissions() {
        let (app, user) = bootstrap();

        let created = app
            .create_user(
                &user,
                "editor".into(),
                "password123".to_string(),
                vec![Entitlement::ViewCatalog],
            )
            .await
            .expect("create user");

        let updated = app
            .set_user_entitlements(
                &user,
                &created.id,
                vec![Entitlement::ViewCatalog, Entitlement::ManageTitle],
            )
            .await
            .expect("update entitlements");

        assert!(updated.entitlements.contains(&Entitlement::ManageTitle));
    }

    #[tokio::test]
    async fn update_user_password_is_hashed() {
        let (app, user) = bootstrap();

        let created = app
            .create_user(
                &user,
                "password-user".into(),
                "before-pass".to_string(),
                vec![Entitlement::ViewCatalog],
            )
            .await
            .expect("create user");

        let updated = app
            .set_user_password(&user, &created.id, "after-pass".to_string(), None)
            .await
            .expect("update password");

        assert!(updated.password_hash.is_some());
        assert_ne!(
            updated.password_hash, created.password_hash,
            "password hash should change when password is updated"
        );
        assert_ne!(updated.password_hash, Some("after-pass".to_string()));
    }

    #[tokio::test]
    async fn delete_other_user_removes_user() {
        let (app, user) = bootstrap();

        let created = app
            .create_user(
                &user,
                "removable".into(),
                "password123".to_string(),
                vec![Entitlement::ViewCatalog],
            )
            .await
            .expect("create user");

        app.delete_user(&user, &created.id)
            .await
            .expect("delete user");

        let users = app.list_users(&user).await.expect("list users");
        assert!(!users.iter().any(|entry| entry.id == created.id));
    }

    #[tokio::test]
    async fn update_title_metadata_changes_name_and_tags() {
        let (app, user) = bootstrap();
        let created = app
            .add_title(
                &user,
                NewTitle {
                    name: "Original".into(),
                    facet: MediaFacet::Movie,
                    monitored: true,
                    tags: vec!["SciFi".into()],
                    external_ids: vec![],
                    min_availability: None,
                },
            )
            .await
            .expect("create title");

        let updated = app
            .update_title_metadata(
                &user,
                &created.id,
                Some("Updated Name".into()),
                None,
                Some(vec!["Action".into(), "Drama".into(), "Action".into()]),
            )
            .await
            .expect("update title metadata");

        assert_eq!(updated.name, "Updated Name");
        assert_eq!(
            updated.tags,
            vec!["action".to_string(), "drama".to_string()]
        );
    }

    #[tokio::test]
    async fn create_collection_and_episode() {
        let (app, user) = bootstrap();
        let title = app
            .add_title(
                &user,
                NewTitle {
                    name: "The Odes".into(),
                    facet: MediaFacet::Tv,
                    monitored: true,
                    tags: vec![],
                    external_ids: vec![],
                    min_availability: None,
                },
            )
            .await
            .expect("create title");

        let collection = app
            .create_collection(
                &user,
                title.id.clone(),
                "season".into(),
                "1".into(),
                Some("Season One".into()),
                None,
                Some("1".into()),
                Some("12".into()),
            )
            .await
            .expect("create collection");

        let episode = app
            .create_episode(
                &user,
                title.id.clone(),
                Some(collection.id.clone()),
                "episode".into(),
                Some("1".into()),
                Some("1".into()),
                Some("Pilot".into()),
                Some("Pilot".into()),
                None,
                Some(1_200),
                false,
                false,
            )
            .await
            .expect("create episode");

        let collections = app
            .list_collections(&user, &title.id)
            .await
            .expect("list collections");
        let episodes = app
            .list_episodes(&user, &collection.id)
            .await
            .expect("list episodes");

        assert_eq!(collections.len(), 1);
        assert_eq!(collections[0].id, collection.id);
        assert_eq!(episodes.len(), 1);
        assert_eq!(episodes[0].id, episode.id);
    }

    #[tokio::test]
    async fn read_collection_by_id_returns_item() {
        let (app, user) = bootstrap();
        let title = app
            .add_title(
                &user,
                NewTitle {
                    name: "Read Collection".into(),
                    facet: MediaFacet::Tv,
                    monitored: true,
                    tags: vec![],
                    external_ids: vec![],
                    min_availability: None,
                },
            )
            .await
            .expect("create title");

        let collection = app
            .create_collection(
                &user,
                title.id.clone(),
                "season".into(),
                "1".into(),
                Some("Season One".into()),
                None,
                Some("1".into()),
                Some("12".into()),
            )
            .await
            .expect("create collection");

        let found = app
            .get_collection(&user, &collection.id)
            .await
            .expect("get collection")
            .expect("found collection");

        assert_eq!(found.id, collection.id);
        assert_eq!(found.collection_index, collection.collection_index);
    }

    #[tokio::test]
    async fn read_episode_by_id_returns_item() {
        let (app, user) = bootstrap();
        let title = app
            .add_title(
                &user,
                NewTitle {
                    name: "Read Episode".into(),
                    facet: MediaFacet::Tv,
                    monitored: true,
                    tags: vec![],
                    external_ids: vec![],
                    min_availability: None,
                },
            )
            .await
            .expect("create title");

        let collection = app
            .create_collection(
                &user,
                title.id.clone(),
                "season".into(),
                "1".into(),
                Some("Season One".into()),
                None,
                Some("1".into()),
                Some("12".into()),
            )
            .await
            .expect("create collection");

        let episode = app
            .create_episode(
                &user,
                title.id.clone(),
                Some(collection.id.clone()),
                "episode".into(),
                Some("1".into()),
                Some("1".into()),
                Some("Pilot".into()),
                Some("Pilot".into()),
                None,
                Some(1_200),
                false,
                false,
            )
            .await
            .expect("create episode");

        let found = app
            .get_episode(&user, &episode.id)
            .await
            .expect("get episode")
            .expect("found episode");

        assert_eq!(found.id, episode.id);
        assert_eq!(found.episode_number, episode.episode_number);
    }

    #[tokio::test]
    async fn delete_collection_removes_collection_entry() {
        let (app, user) = bootstrap();
        let title = app
            .add_title(
                &user,
                NewTitle {
                    name: "Collection Delete".into(),
                    facet: MediaFacet::Tv,
                    monitored: true,
                    tags: vec![],
                    external_ids: vec![],
                    min_availability: None,
                },
            )
            .await
            .expect("create title");

        let collection = app
            .create_collection(
                &user,
                title.id.clone(),
                "season".into(),
                "1".into(),
                Some("Season One".into()),
                None,
                Some("1".into()),
                Some("12".into()),
            )
            .await
            .expect("create collection");

        app.delete_collection(&user, &collection.id)
            .await
            .expect("delete collection");

        let collections = app
            .list_collections(&user, &title.id)
            .await
            .expect("list collections");
        assert!(collections.is_empty());
    }

    #[tokio::test]
    async fn delete_episode_removes_episode_entry() {
        let (app, user) = bootstrap();
        let title = app
            .add_title(
                &user,
                NewTitle {
                    name: "Episode Delete".into(),
                    facet: MediaFacet::Tv,
                    monitored: true,
                    tags: vec![],
                    external_ids: vec![],
                    min_availability: None,
                },
            )
            .await
            .expect("create title");

        let collection = app
            .create_collection(
                &user,
                title.id.clone(),
                "season".into(),
                "1".into(),
                Some("Season One".into()),
                None,
                Some("1".into()),
                Some("12".into()),
            )
            .await
            .expect("create collection");

        let episode = app
            .create_episode(
                &user,
                title.id.clone(),
                Some(collection.id.clone()),
                "episode".into(),
                Some("1".into()),
                Some("1".into()),
                Some("Pilot".into()),
                Some("Pilot".into()),
                None,
                Some(1_200),
                false,
                false,
            )
            .await
            .expect("create episode");

        app.delete_episode(&user, &episode.id)
            .await
            .expect("delete episode");

        let episodes = app
            .list_episodes(&user, &collection.id)
            .await
            .expect("list episodes");
        assert!(episodes.is_empty(), "expected episode to be deleted");
    }

    #[tokio::test]
    async fn update_collection_changes_fields() {
        let (app, user) = bootstrap();
        let title = app
            .add_title(
                &user,
                NewTitle {
                    name: "Update Collection".into(),
                    facet: MediaFacet::Tv,
                    monitored: true,
                    tags: vec![],
                    external_ids: vec![],
                    min_availability: None,
                },
            )
            .await
            .expect("create title");

        let collection = app
            .create_collection(
                &user,
                title.id.clone(),
                "season".into(),
                "1".into(),
                Some("Season One".into()),
                Some("s1".into()),
                Some("1".into()),
                Some("12".into()),
            )
            .await
            .expect("create collection");

        let updated = app
            .update_collection(
                &user,
                collection.id.clone(),
                Some("arc".into()),
                None,
                Some("Arc One".into()),
                Some("arc-one".into()),
                None,
                Some("13".into()),
                None,
            )
            .await
            .expect("update collection");

        assert_eq!(updated.collection_type, "arc");
        assert_eq!(updated.label, Some("Arc One".into()));
        assert_eq!(updated.ordered_path, Some("arc-one".into()));
        assert_eq!(updated.last_episode_number, Some("13".into()));
        assert_eq!(updated.collection_index, "1");
    }

    #[tokio::test]
    async fn update_episode_changes_fields() {
        let (app, user) = bootstrap();
        let title = app
            .add_title(
                &user,
                NewTitle {
                    name: "Update Episode".into(),
                    facet: MediaFacet::Tv,
                    monitored: true,
                    tags: vec![],
                    external_ids: vec![],
                    min_availability: None,
                },
            )
            .await
            .expect("create title");

        let collection = app
            .create_collection(
                &user,
                title.id.clone(),
                "season".into(),
                "1".into(),
                Some("Season One".into()),
                None,
                Some("1".into()),
                Some("12".into()),
            )
            .await
            .expect("create collection");

        let episode = app
            .create_episode(
                &user,
                title.id.clone(),
                Some(collection.id.clone()),
                "episode".into(),
                Some("1".into()),
                Some("1".into()),
                Some("Pilot".into()),
                Some("Pilot".into()),
                None,
                Some(1_200),
                false,
                false,
            )
            .await
            .expect("create episode");

        let updated = app
            .update_episode(
                &user,
                episode.id.clone(),
                Some("special".into()),
                Some("E01".into()),
                None,
                None,
                Some("Pilot Updated".into()),
                Some("2026-01-01".into()),
                Some(1_800),
                Some(true),
                None,
                None,
                Some(collection.id.clone()),
            )
            .await
            .expect("update episode");

        assert_eq!(updated.episode_type, "special");
        assert_eq!(updated.episode_number, Some("E01".into()));
        assert_eq!(updated.title, Some("Pilot Updated".into()));
        assert_eq!(updated.air_date, Some("2026-01-01".into()));
        assert_eq!(updated.duration_seconds, Some(1_800));
        assert!(updated.has_multi_audio);
        assert!(!updated.has_subtitle);
    }

    #[test]
    fn hash_and_validate_password_round_trip() {
        let (app, _user) = bootstrap();
        let hashed = app
            .hash_password("P@ssw0rd")
            .expect("hash should be generated");
        assert!(app
            .validate_password("P@ssw0rd", &hashed)
            .expect("hash should be valid"));
        assert!(!app
            .validate_password("wrong", &hashed)
            .expect("hash should validate"));
    }

    #[test]
    fn hash_version_is_explicit() {
        let (app, _user) = bootstrap();

        assert!(app.hash_password("abc").expect("hash").starts_with("v2$"));
    }

    #[test]
    fn v1_password_still_validates() {
        let (app, _user) = bootstrap();
        // Simulate a legacy v1 hash
        let salt = "abcdef0123456789abcdef0123456789";
        let digest = sha256_hex(format!("{salt}legacy-pass"));
        let v1_hash = format!("v1${salt}${digest}");
        assert!(app
            .validate_password("legacy-pass", &v1_hash)
            .expect("v1 should validate"));
        assert!(!app
            .validate_password("wrong", &v1_hash)
            .expect("v1 should reject wrong password"));
    }
}

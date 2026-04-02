#![allow(clippy::too_many_arguments)]

mod acquisition_policy;
mod activity;
mod app_usecase_acquisition;
mod app_usecase_activity;
mod app_usecase_admin;
mod app_usecase_backup;
mod app_usecase_catalog;
mod app_usecase_discovery;
mod app_usecase_health;
mod app_usecase_housekeeping;
mod app_usecase_import;
mod app_usecase_indexer_test;
mod app_usecase_integration;
pub(crate) mod app_usecase_library;
mod app_usecase_notifications;
mod app_usecase_pending;
mod app_usecase_plugins;
pub mod app_usecase_post_processing;
pub(crate) mod app_usecase_rss;
mod app_usecase_rules;
mod app_usecase_security;
mod app_usecase_settings;
mod app_usecase_subtitles;
mod app_usecase_title_images;
pub(crate) mod archive_extractor;
pub mod completed_download_handler;
mod delay_profile;
pub(crate) mod facet_handler;
mod facet_movie;
mod facet_registry;
mod facet_series;
pub mod failed_download_handler;
pub(crate) mod import_checks;
mod library_rename;
mod library_scan;
pub mod managed_rules;
mod media_analyzer;
pub(crate) mod nfo;
pub(crate) mod normalize;
mod notification_dispatcher;
mod null_repositories;
mod post_download_gate;
mod quality_profile;
pub mod recycle_bin;
pub mod release_dedup;
mod release_group_db;
mod release_parser;
mod scoring_weights;
pub mod subtitles;
pub mod tracked_downloads;
mod types;
pub mod upgrade;
mod user_rule_input;

use crate::activity::ActivityStream;
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use rand_core::OsRng;
use ring::digest as ring_digest;
use scryer_domain::{
    BlocklistEntry, CalendarEpisode, Collection, CollectionType, CompletedDownload,
    DownloadClientConfig, DownloadQueueItem, DownloadQueueState, Entitlement, Episode, EventType,
    ExternalId, HistoryEvent, Id, ImportFileResult, ImportRecord, ImportResult, ImportStatus,
    IndexerConfig, MediaFacet, NewDownloadClientConfig, NewIndexerConfig, NewTitle,
    PluginInstallation, PolicyInput, PolicyOutput, RuleSet, TaggedAlias, Title,
    TitleHistoryEventType, TitleHistoryRecord, User,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{Mutex, OnceCell, RwLock, Semaphore, broadcast};

pub type AppResult<T> = Result<T, AppError>;

use crate::quality_profile::resolve_profile_id_for_title;
pub use acquisition_policy::AcquisitionThresholds;
pub use activity::{
    ActivityChannel, ActivityEvent, ActivityKind, ActivitySeverity, NotificationEnvelope,
};
pub use app_usecase_acquisition::start_background_acquisition_poller;
pub use app_usecase_backup::BackupService;
pub use app_usecase_catalog::{
    DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY, LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY,
    start_background_hydration_loop,
};
pub use app_usecase_import::{
    ManualImportFileMapping, ManualImportFilePreview, ManualImportFileResult, ManualImportPreview,
    execute_manual_import, import_completed_download, preview_manual_import, retry_failed_import,
    try_import_completed_downloads,
};
pub use app_usecase_integration::start_download_queue_poller;
pub use app_usecase_plugins::{RegistryPlugin, RulePackRegistryEntry, RulePackTemplate};
pub use app_usecase_post_processing::{PostProcessingContext, run_post_processing};
pub use app_usecase_rss::RssSyncReport;
pub use app_usecase_rules::{ConvenienceAudioSetting, ConvenienceSettings};
pub use app_usecase_settings::{AcquisitionSettings, SubtitleSettings, UpdateSubtitleSettings};
pub use app_usecase_subtitles::{spawn_subtitle_search_for_file, start_background_subtitle_poller};
pub use app_usecase_title_images::start_background_banner_loop;
pub use app_usecase_title_images::start_background_fanart_loop;
pub use app_usecase_title_images::start_background_poster_loop;
pub use delay_profile::{
    DELAY_PROFILE_CATALOG_KEY, DelayDecision, DelayProfile, PreferredProtocol, is_usenet_source,
    parse_delay_profile_catalog, resolve_delay_decision, resolve_delay_profile,
    validate_delay_profile_catalog,
};
pub use facet_handler::{
    FacetHandler, HydrationResult, movie_to_hydration_result, series_to_hydration_result,
};
pub use facet_movie::MovieFacetHandler;
pub use facet_registry::FacetRegistry;
pub use facet_series::SeriesFacetHandler;
pub use library_rename::{
    LibraryRenamer, NullLibraryRenamer, RenameApplyItemResult, RenameApplyResult,
    RenameApplyStatus, RenameCollisionPolicy, RenameMissingMetadataPolicy, RenamePlan,
    RenamePlanItem, RenameWriteAction, build_rename_plan_fingerprint, render_rename_template,
};
pub use library_scan::{
    AnibridgeSourceMapping, AnimeEpisodeMapping, AnimeMapping, AnimeMovie, BulkMetadataResult,
    EpisodeMetadata, LibraryFile, LibraryFileBatch, LibraryFileBatchReceiver, LibraryScanSummary,
    LibraryScanner, MetadataGateway, MetadataSearchItem, MovieMetadata, MultiMetadataSearchResult,
    RichMetadataSearchItem, SeasonMetadata, SeriesMetadata,
};
pub use media_analyzer::NativeMediaAnalyzer;
pub use notification_dispatcher::start_notification_dispatcher;
pub use null_repositories::{
    NullAcquisitionStateRepository, NullBlocklistRepository, NullDownloadSubmissionRepository,
    NullFileImporter, NullHousekeepingRepository, NullImportRepository, NullIndexerStatsTracker,
    NullMediaFileRepository, NullNotificationChannelRepository,
    NullNotificationSubscriptionRepository, NullPendingReleaseRepository,
    NullPluginInstallationRepository, NullPostProcessingScriptRepository, NullRuleSetRepository,
    NullSettingsRepository, NullStagedNzbStore, NullSystemInfoProvider, NullTitleHistoryRepository,
    NullTitleImageProcessor, NullTitleImageRepository, NullWantedItemRepository,
};
pub use quality_profile::{
    BLOCK_SCORE, QUALITY_PROFILE_CATALOG_KEY, QUALITY_PROFILE_ID_KEY,
    QUALITY_PROFILE_INHERIT_VALUE, QualityProfile, QualityProfileCriteria, QualityProfileDecision,
    ScoringConfig, ScoringEntry, ScoringSource, apply_age_scoring, apply_size_scoring_for_category,
    default_quality_profile_1080p_for_search, default_quality_profile_for_search,
    evaluate_against_profile, parse_profile_catalog_from_json,
};
pub use release_parser::{
    ParsedEpisodeMetadata, ParsedEpisodeReleaseType, ParsedReleaseMetadata, ParsedSpecialKind,
    parse_release_metadata,
};
pub use scoring_weights::{
    ScoringOverrides, ScoringPersona, ScoringWeights, build_weights, build_weights_for_category,
};
pub(crate) use types::JwtClaims;
pub use types::{
    BackupInfo, DiskSpaceInfo, DownloadGrabResult, DownloadHistoryPage, DownloadSourceKind,
    FixTitleMatchResult, HealthCheckResult, HealthCheckStatus, HousekeepingReport,
    IndexerQueryStats, IndexerSearchResponse, IndexerSearchResult, JwtAuthConfig, PendingRelease,
    PendingReleaseStatus, PrimaryCollectionSummary, ReleaseDecision, ReleaseDownloadAttemptOutcome,
    ReleaseDownloadFailureSignature, SystemHealth, TitleImageBlob, TitleImageKind,
    TitleImageReplacement, TitleImageStorageMode, TitleImageSyncTask, TitleImageVariantRecord,
    TitleMediaFile, TitleMediaSizeSummary, TitleMetadataUpdate, TitleReleaseBlocklistEntry,
    WantedCompleteTransition, WantedGrabTransition, WantedItem, WantedPauseTransition,
    WantedSearchTransition, WantedStatus,
};

const SETTINGS_SCOPE_SYSTEM: &str = "system";
const SETTINGS_SCOPE_MEDIA: &str = "media";
const INHERIT_QUALITY_PROFILE_VALUE: &str = "__inherit__";
const NATIVE_DOWNLOAD_CLIENT_TYPES: [&str; 4] = ["nzbget", "sabnzbd", "qbittorrent", "weaver"];

/// Return the accepted input kinds for a download client type, checking
/// the plugin provider first (WASM plugins), then falling back to known
/// native client capabilities.
///
/// An empty vec means the client has not declared any capabilities and
/// will not receive any downloads.
pub fn accepted_inputs_for_client(
    client_type: &str,
    plugin_provider: Option<&Arc<dyn DownloadClientPluginProvider>>,
) -> Vec<DownloadSourceKind> {
    if let Some(provider) = plugin_provider {
        let inputs = provider.accepted_inputs_for_provider(client_type);
        if !inputs.is_empty() {
            return inputs
                .iter()
                .filter_map(|s| DownloadSourceKind::parse(s))
                .collect();
        }
    }
    native_accepted_inputs(client_type)
}

/// Native client capabilities. Returns the accepted input kinds for
/// built-in download client types.
fn native_accepted_inputs(client_type: &str) -> Vec<DownloadSourceKind> {
    match client_type {
        "nzbget" | "sabnzbd" | "weaver" => vec![DownloadSourceKind::NzbFile],
        "qbittorrent" => vec![
            DownloadSourceKind::TorrentFile,
            DownloadSourceKind::MagnetUri,
        ],
        _ => vec![],
    }
}
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

/// Lower the calling thread's scheduling priority via `nice(10)`.
///
/// Call this at the top of CPU-heavy `spawn_blocking` closures (AVIF encoding,
/// alass alignment, audio decoding) so they don't starve the async runtime.
/// Safe to call on any Unix platform; silently ignored on Windows.
#[cfg(unix)]
pub fn nice_thread() {
    // SAFETY: nice() is always safe to call; worst case it returns -1 with EPERM
    // which we intentionally ignore — the work still proceeds at normal priority.
    unsafe {
        libc::nice(10);
    }
}

#[cfg(not(unix))]
pub fn nice_thread() {}

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
    pub media_analyzer: Arc<dyn MediaAnalyzer>,
    pub download_client_configs: Arc<dyn DownloadClientConfigRepository>,
    pub release_attempts: Arc<dyn ReleaseAttemptRepository>,
    pub acquisition_state: Arc<dyn AcquisitionStateRepository>,
    pub download_submissions: Arc<dyn DownloadSubmissionRepository>,
    pub settings: Arc<dyn SettingsRepository>,
    pub quality_profiles: Arc<dyn QualityProfileRepository>,
    pub wanted_items: Arc<dyn WantedItemRepository>,
    pub rule_sets: Arc<dyn RuleSetRepository>,
    pub pp_scripts: Arc<dyn PostProcessingScriptRepository>,
    pub plugin_installations: Arc<dyn PluginInstallationRepository>,
    pub system_info: Arc<dyn SystemInfoProvider>,
    pub title_images: Arc<dyn TitleImageRepository>,
    pub title_image_processor: Arc<dyn TitleImageProcessor>,
    pub indexer_stats: Arc<dyn IndexerStatsTracker>,
    pub user_rules: Arc<std::sync::RwLock<scryer_rules::UserRulesEngine>>,
    pub plugin_provider: Option<Arc<dyn IndexerPluginProvider>>,
    pub download_client_plugin_provider: Option<Arc<dyn DownloadClientPluginProvider>>,
    pub notification_channels: Option<Arc<dyn NotificationChannelRepository>>,
    pub notification_subscriptions: Option<Arc<dyn NotificationSubscriptionRepository>>,
    pub notification_provider: Option<Arc<dyn NotificationPluginProvider>>,
    pub db_path: String,
    pub activity_stream: ActivityStream,
    pub event_broadcast: broadcast::Sender<HistoryEvent>,
    pub activity_event_broadcast: broadcast::Sender<ActivityEvent>,
    pub download_queue_broadcast: broadcast::Sender<Vec<DownloadQueueItem>>,
    pub import_history_broadcast: broadcast::Sender<()>,
    pub settings_changed_broadcast: broadcast::Sender<Vec<String>>,
    pub acquisition_wake: Arc<tokio::sync::Notify>,
    pub hydration_wake: Arc<tokio::sync::Notify>,
    pub poster_wake: Arc<tokio::sync::Notify>,
    pub banner_wake: Arc<tokio::sync::Notify>,
    pub fanart_wake: Arc<tokio::sync::Notify>,
    pub housekeeping: Arc<dyn HousekeepingRepository>,
    pub health_check_results: Arc<tokio::sync::RwLock<Vec<HealthCheckResult>>>,
    pub pending_releases: Arc<dyn PendingReleaseRepository>,
    pub title_history: Arc<dyn TitleHistoryRepository>,
    pub blocklist_repo: Arc<dyn BlocklistRepository>,
    pub rss_seen_guids: Arc<tokio::sync::RwLock<HashSet<String>>>,
    pub subtitle_downloads: Arc<dyn SubtitleDownloadRepository>,
    pub import_artifacts: Arc<dyn ImportArtifactRepository>,
    pub staged_nzb_store: Arc<dyn StagedNzbStore>,
    pub staged_nzb_pipeline_limit: Arc<Semaphore>,
    pub tracked_download_handle: Option<tracked_downloads::TrackedDownloadHandle>,
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
        let (import_history_tx, _) = broadcast::channel::<()>(16);
        let (settings_changed_tx, _) = broadcast::channel::<Vec<String>>(16);
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
            media_analyzer: Arc::new(NativeMediaAnalyzer),
            download_client_configs,
            release_attempts,
            acquisition_state: Arc::new(NullAcquisitionStateRepository),
            download_submissions: Arc::new(NullDownloadSubmissionRepository),
            settings,
            quality_profiles,
            wanted_items: Arc::new(NullWantedItemRepository),
            rule_sets: Arc::new(NullRuleSetRepository),
            pp_scripts: Arc::new(NullPostProcessingScriptRepository),
            plugin_installations: Arc::new(NullPluginInstallationRepository),
            system_info: Arc::new(NullSystemInfoProvider),
            title_images: Arc::new(NullTitleImageRepository),
            title_image_processor: Arc::new(NullTitleImageProcessor),
            indexer_stats: Arc::new(NullIndexerStatsTracker),
            user_rules: Arc::new(std::sync::RwLock::new(
                scryer_rules::UserRulesEngine::empty(),
            )),
            plugin_provider: None,
            download_client_plugin_provider: None,
            notification_channels: None,
            notification_subscriptions: None,
            notification_provider: None,
            db_path,
            activity_stream: ActivityStream::new(),
            event_broadcast: tx,
            activity_event_broadcast: activity_tx,
            download_queue_broadcast: queue_tx,
            import_history_broadcast: import_history_tx,
            settings_changed_broadcast: settings_changed_tx,
            acquisition_wake: Arc::new(tokio::sync::Notify::new()),
            hydration_wake: Arc::new(tokio::sync::Notify::new()),
            poster_wake: Arc::new(tokio::sync::Notify::new()),
            banner_wake: Arc::new(tokio::sync::Notify::new()),
            fanart_wake: Arc::new(tokio::sync::Notify::new()),
            housekeeping: Arc::new(NullHousekeepingRepository),
            pending_releases: Arc::new(NullPendingReleaseRepository),
            title_history: Arc::new(NullTitleHistoryRepository),
            blocklist_repo: Arc::new(NullBlocklistRepository),
            subtitle_downloads: Arc::new(null_repositories::NullSubtitleDownloadRepository),
            health_check_results: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            rss_seen_guids: Arc::new(tokio::sync::RwLock::new(HashSet::new())),
            import_artifacts: Arc::new(null_repositories::NullImportArtifactRepository),
            staged_nzb_store: Arc::new(null_repositories::NullStagedNzbStore),
            staged_nzb_pipeline_limit: Arc::new(Semaphore::new(4)),
            tracked_download_handle: None,
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
        facet: Option<String>,
        kind: ActivityKind,
        message: String,
        severity: ActivitySeverity,
        channels: Vec<ActivityChannel>,
    ) -> AppResult<()> {
        let mut event =
            ActivityEvent::new(kind, actor_user_id, title_id, message, severity, channels);
        if let Some(f) = facet {
            event = event.with_facet(f);
        }
        self.activity_stream.push(event.clone()).await;
        let _ = self.activity_event_broadcast.send(event);
        Ok(())
    }

    pub async fn record_activity_event_with_notification(
        &self,
        actor_user_id: Option<String>,
        title_id: Option<String>,
        facet: Option<String>,
        kind: ActivityKind,
        message: String,
        severity: ActivitySeverity,
        channels: Vec<ActivityChannel>,
        envelope: crate::activity::NotificationEnvelope,
    ) -> AppResult<()> {
        let mut event =
            ActivityEvent::new(kind, actor_user_id, title_id, message, severity, channels)
                .with_notification(envelope);
        if let Some(f) = facet {
            event = event.with_facet(f);
        }
        self.activity_stream.push(event.clone()).await;
        let _ = self.activity_event_broadcast.send(event);
        Ok(())
    }

    pub async fn update_import_status_and_notify(
        &self,
        import_id: &str,
        status: ImportStatus,
        result_json: Option<String>,
    ) -> AppResult<()> {
        self.imports
            .update_import_status(import_id, status, result_json.clone())
            .await?;
        if matches!(status, ImportStatus::Completed | ImportStatus::Failed) {
            let _ = self.import_history_broadcast.send(());
        }

        // Dual-write: emit title history event from import result
        if let Some(ref json) = result_json
            && let Ok(result) = serde_json::from_str::<ImportResult>(json)
            && let Some(ref title_id) = result.title_id
        {
            let event_type = match status {
                ImportStatus::Completed => TitleHistoryEventType::Imported,
                ImportStatus::Failed => TitleHistoryEventType::ImportFailed,
                ImportStatus::Skipped => TitleHistoryEventType::ImportSkipped,
                _ => return Ok(()),
            };
            let mut data = std::collections::HashMap::new();
            data.insert("import_id".into(), serde_json::json!(import_id));
            data.insert("source_path".into(), serde_json::json!(result.source_path));
            if let Some(ref dp) = result.dest_path {
                data.insert("dest_path".into(), serde_json::json!(dp));
            }
            if let Some(ref msg) = result.error_message {
                data.insert("message".into(), serde_json::json!(msg));
            }
            if let Some(ref sr) = result.skip_reason {
                data.insert("skip_reason".into(), serde_json::json!(sr.as_str()));
            }
            data.insert(
                "decision".into(),
                serde_json::json!(result.decision.as_str()),
            );
            if let Some(sz) = result.file_size_bytes {
                data.insert("size_bytes".into(), serde_json::json!(sz));
            }
            let _ = self
                .title_history
                .record_event(&NewTitleHistoryEvent {
                    title_id: title_id.clone(),
                    episode_id: None,
                    collection_id: None,
                    event_type,
                    source_title: Some(result.source_path.clone()),
                    quality: None,
                    download_id: None,
                    data,
                })
                .await;
        }
        Ok(())
    }

    pub async fn record_title_history(&self, event: NewTitleHistoryEvent) -> AppResult<String> {
        let id = self.title_history.record_event(&event).await?;
        let _ = self.import_history_broadcast.send(());
        Ok(id)
    }
}

#[async_trait]
pub trait TitleRepository: Send + Sync {
    async fn list(&self, facet: Option<MediaFacet>, query: Option<String>)
    -> AppResult<Vec<Title>>;
    async fn get_by_id(&self, id: &str) -> AppResult<Option<Title>>;
    async fn find_by_external_id(&self, source: &str, value: &str) -> AppResult<Option<Title>>;
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
    async fn replace_match_state(
        &self,
        id: &str,
        external_ids: Vec<ExternalId>,
        tags: Vec<String>,
    ) -> AppResult<Title>;
    async fn delete(&self, id: &str) -> AppResult<()>;
    async fn set_folder_path(&self, id: &str, folder_path: &str) -> AppResult<()>;
    /// Return titles that need hydration: either never hydrated
    /// (`metadata_fetched_at IS NULL`) or hydrated in a different language
    /// (`metadata_language IS NULL OR metadata_language != language`).
    /// Ordered by creation time, up to `limit`.
    async fn list_unhydrated(&self, limit: usize, language: &str) -> AppResult<Vec<Title>>;
    /// Clear `metadata_language` on all titles that have one set, marking them
    /// for re-hydration in the next background loop pass.
    async fn clear_metadata_language_for_all(&self) -> AppResult<u64>;
}

#[async_trait]
pub trait TitleImageRepository: Send + Sync {
    async fn list_titles_requiring_image_refresh(
        &self,
        kind: TitleImageKind,
        limit: usize,
    ) -> AppResult<Vec<TitleImageSyncTask>>;

    async fn replace_title_image(
        &self,
        title_id: &str,
        replacement: TitleImageReplacement,
    ) -> AppResult<()>;

    async fn get_title_image_blob(
        &self,
        title_id: &str,
        kind: TitleImageKind,
        variant_key: &str,
    ) -> AppResult<Option<TitleImageBlob>>;
}

#[async_trait]
pub trait TitleImageProcessor: Send + Sync {
    async fn fetch_and_process_image(
        &self,
        kind: TitleImageKind,
        source_url: &str,
    ) -> AppResult<TitleImageReplacement>;
}

#[async_trait]
pub trait ShowRepository: Send + Sync {
    async fn list_collections_for_title(&self, title_id: &str) -> AppResult<Vec<Collection>>;
    async fn get_collection_by_id(&self, collection_id: &str) -> AppResult<Option<Collection>>;
    async fn create_collection(&self, collection: Collection) -> AppResult<Collection>;
    async fn update_collection(
        &self,
        collection_id: &str,
        collection_type: Option<CollectionType>,
        collection_index: Option<String>,
        label: Option<String>,
        ordered_path: Option<String>,
        first_episode_number: Option<String>,
        last_episode_number: Option<String>,
        monitored: Option<bool>,
    ) -> AppResult<Collection>;
    async fn update_interstitial_season_episode(
        &self,
        collection_id: &str,
        season_episode: Option<String>,
    ) -> AppResult<()>;
    async fn set_collection_episodes_monitored(
        &self,
        collection_id: &str,
        monitored: bool,
    ) -> AppResult<()>;
    async fn delete_collection(&self, collection_id: &str) -> AppResult<()>;
    async fn delete_collections_for_title(&self, title_id: &str) -> AppResult<()>;
    async fn list_episodes_for_collection(&self, collection_id: &str) -> AppResult<Vec<Episode>>;
    async fn list_episodes_for_title(&self, title_id: &str) -> AppResult<Vec<Episode>>;
    async fn get_episode_by_id(&self, episode_id: &str) -> AppResult<Option<Episode>>;
    async fn create_episode(&self, episode: Episode) -> AppResult<Episode>;
    async fn update_episode(
        &self,
        episode_id: &str,
        episode_type: Option<scryer_domain::EpisodeType>,
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
        overview: Option<String>,
        tvdb_id: Option<String>,
    ) -> AppResult<Episode>;
    async fn delete_episode(&self, episode_id: &str) -> AppResult<()>;
    async fn delete_episodes_for_title(&self, title_id: &str) -> AppResult<()>;
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
    async fn list_episodes_in_date_range(
        &self,
        start_date: &str,
        end_date: &str,
    ) -> AppResult<Vec<CalendarEpisode>>;
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
        config_json: Option<String>,
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
    async fn reorder(&self, ordered_ids: Vec<String>) -> AppResult<()>;
}

#[async_trait]
pub trait SettingsRepository: Send + Sync {
    async fn get_setting_json(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
    ) -> AppResult<Option<String>>;

    async fn upsert_setting_json(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
        value_json: String,
        source: &str,
        updated_by_user_id: Option<String>,
    ) -> AppResult<()>;
}

#[async_trait]
pub trait SystemInfoProvider: Send + Sync {
    async fn current_migration_version(&self) -> AppResult<Option<String>>;
    async fn pending_migration_count(&self) -> AppResult<usize>;
    async fn smg_cert_expires_at(&self) -> AppResult<Option<String>>;
    async fn vacuum_into(&self, dest_path: &str) -> AppResult<()>;
}

#[async_trait]
pub trait HousekeepingRepository: Send + Sync {
    async fn delete_release_decisions_older_than(&self, days: i64) -> AppResult<u32>;
    async fn delete_release_attempts_older_than(&self, days: i64) -> AppResult<u32>;
    async fn delete_dispatched_event_outboxes_older_than(&self, days: i64) -> AppResult<u32>;
    async fn delete_history_events_older_than(&self, days: i64) -> AppResult<u32>;
    async fn list_all_media_file_paths(&self) -> AppResult<Vec<(String, String)>>;
    async fn delete_media_files_by_ids(&self, ids: &[String]) -> AppResult<u32>;
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

    /// Returns true if the indexer is at or near its API quota (>= 95%).
    fn is_at_quota(&self, indexer_id: &str) -> bool {
        self.all_stats()
            .iter()
            .find(|s| s.indexer_id == indexer_id)
            .map(|s| match (s.api_current, s.api_max) {
                (Some(c), Some(m)) if m > 0 => c >= m * 95 / 100,
                _ => false,
            })
            .unwrap_or(false)
    }
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

#[derive(Clone, Debug)]
pub struct DownloadSubmission {
    pub title_id: String,
    pub facet: String,
    pub download_client_type: String,
    pub download_client_item_id: String,
    pub source_title: Option<String>,
    pub collection_id: Option<String>,
}

#[derive(Clone, Debug)]
pub struct SuccessfulGrabCommit {
    pub wanted_item_id: String,
    pub search_count: i64,
    pub current_score: Option<i32>,
    pub grabbed_release: String,
    pub last_search_at: Option<String>,
    pub download_submission: DownloadSubmission,
    pub grabbed_pending_release_id: Option<String>,
    pub grabbed_at: Option<String>,
}

#[async_trait]
pub trait AcquisitionStateRepository: Send + Sync {
    async fn commit_successful_grab(&self, commit: &SuccessfulGrabCommit) -> AppResult<()>;
}

#[async_trait]
pub trait DownloadSubmissionRepository: Send + Sync {
    async fn record_submission(&self, submission: DownloadSubmission) -> AppResult<()>;

    async fn find_by_client_item_id(
        &self,
        download_client_type: &str,
        download_client_item_id: &str,
    ) -> AppResult<Option<DownloadSubmission>>;

    async fn list_for_title(&self, title_id: &str) -> AppResult<Vec<DownloadSubmission>>;

    async fn delete_for_title(&self, title_id: &str) -> AppResult<()>;

    async fn delete_by_client_item_id(&self, download_client_item_id: &str) -> AppResult<()>;

    /// Update the tracked_state column for restart reconstruction.
    async fn update_tracked_state(
        &self,
        download_client_type: &str,
        download_client_item_id: &str,
        tracked_state: &str,
    ) -> AppResult<()>;

    /// Read the tracked_state column for a download.
    async fn get_tracked_state(
        &self,
        download_client_type: &str,
        download_client_item_id: &str,
    ) -> AppResult<Option<String>>;
}

/// Per-file import outcome history for completion verification across passes.
#[derive(Clone, Debug)]
pub struct ImportArtifact {
    pub id: String,
    pub source_system: String,
    pub source_ref: String,
    pub import_id: Option<String>,
    pub relative_path: Option<String>,
    pub normalized_file_name: String,
    pub media_kind: String,
    pub title_id: Option<String>,
    pub episode_id: Option<String>,
    pub season_number: Option<i32>,
    pub episode_number: Option<i32>,
    pub result: String,
    pub reason_code: Option<String>,
    pub imported_media_file_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StagedNzbRef {
    pub id: String,
    pub compressed_path: PathBuf,
    pub raw_size_bytes: u64,
}

#[derive(Clone, Debug)]
pub struct PendingStagedNzb {
    pub id: String,
    pub compressed_path: PathBuf,
    pub partial_path: PathBuf,
}

#[async_trait]
pub trait ImportArtifactRepository: Send + Sync {
    async fn insert_artifact(&self, artifact: ImportArtifact) -> AppResult<()>;

    /// List all artifacts for a download, ordered by created_at.
    async fn list_by_source_ref(
        &self,
        source_system: &str,
        source_ref: &str,
    ) -> AppResult<Vec<ImportArtifact>>;

    /// Count artifacts with a given result for a download.
    async fn count_by_result(
        &self,
        source_system: &str,
        source_ref: &str,
        result: &str,
    ) -> AppResult<u64>;
}

#[async_trait]
pub trait StagedNzbStore: Send + Sync {
    async fn create_pending_staged_nzb(
        &self,
        source_url: &str,
        title_id: Option<&str>,
    ) -> AppResult<PendingStagedNzb>;

    async fn finalize_pending_staged_nzb(
        &self,
        pending: PendingStagedNzb,
        raw_size_bytes: u64,
    ) -> AppResult<StagedNzbRef>;

    async fn delete_staged_nzb(&self, staged_nzb: &StagedNzbRef) -> AppResult<bool>;

    async fn prune_staged_nzbs_older_than(&self, older_than: DateTime<Utc>) -> AppResult<u32>;

    fn mark_artifact_active(&self, path: &Path) -> AppResult<()>;

    fn mark_artifact_inactive(&self, path: &Path) -> AppResult<()>;
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

    async fn get_import_by_id(&self, id: &str) -> AppResult<Option<ImportRecord>>;

    async fn get_import_by_source_ref(
        &self,
        source_system: &str,
        source_ref: &str,
    ) -> AppResult<Option<ImportRecord>>;

    async fn update_import_status(
        &self,
        import_id: &str,
        status: ImportStatus,
        result_json: Option<String>,
    ) -> AppResult<()>;

    async fn recover_stale_processing_imports(&self, stale_seconds: i64) -> AppResult<u64>;

    async fn list_pending_imports(&self) -> AppResult<Vec<ImportRecord>>;

    async fn is_already_imported(&self, source_system: &str, source_ref: &str) -> AppResult<bool>;

    async fn list_imports(&self, limit: usize) -> AppResult<Vec<ImportRecord>>;
}

#[async_trait]
pub trait FileImporter: Send + Sync {
    async fn import_file(&self, source: &Path, dest: &Path) -> AppResult<ImportFileResult>;
}

/// Parsed media properties from media analysis — application-layer DTO.
/// A single audio stream, mirroring `scryer_mediainfo::AudioStreamDetail`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AudioStreamDetail {
    pub codec: Option<String>,
    pub channels: Option<i32>,
    pub language: Option<String>,
    pub bitrate_kbps: Option<i32>,
}

/// A single subtitle stream, mirroring `scryer_mediainfo::SubtitleStreamDetail`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SubtitleStreamDetail {
    pub codec: Option<String>,
    pub language: Option<String>,
    pub name: Option<String>,
    pub forced: bool,
    pub default: bool,
}

/// Mirrors `scryer_mediainfo::MediaAnalysis` without depending on that crate.
#[derive(Clone, Debug)]
pub struct MediaFileAnalysis {
    pub video_codec: Option<String>,
    pub video_width: Option<i32>,
    pub video_height: Option<i32>,
    pub video_bitrate_kbps: Option<i32>,
    pub video_bit_depth: Option<i32>,
    pub video_hdr_format: Option<String>,
    pub video_frame_rate: Option<String>,
    pub video_profile: Option<String>,
    pub audio_codec: Option<String>,
    pub audio_channels: Option<i32>,
    pub audio_bitrate_kbps: Option<i32>,
    pub audio_languages: Vec<String>,
    pub audio_streams: Vec<AudioStreamDetail>,
    pub subtitle_languages: Vec<String>,
    pub subtitle_codecs: Vec<String>,
    pub subtitle_streams: Vec<SubtitleStreamDetail>,
    pub has_multiaudio: bool,
    pub duration_seconds: Option<i32>,
    pub num_chapters: Option<i32>,
    pub container_format: Option<String>,
    pub raw_json: String,
}

#[derive(Clone, Debug)]
pub enum MediaAnalysisOutcome {
    Valid(Box<MediaFileAnalysis>),
    Invalid(String),
}

#[async_trait]
pub trait MediaAnalyzer: Send + Sync {
    async fn analyze_file(&self, path: PathBuf) -> AppResult<MediaAnalysisOutcome>;
}

/// Input for inserting a media file record with rich metadata.
#[derive(Clone, Debug, Default)]
pub struct InsertMediaFileInput {
    pub title_id: String,
    pub file_path: String,
    pub size_bytes: i64,
    pub source_signature_scheme: Option<String>,
    pub source_signature_value: Option<String>,
    pub quality_label: Option<String>,
    pub scene_name: Option<String>,
    pub release_group: Option<String>,
    pub source_type: Option<String>,
    pub resolution: Option<String>,
    pub video_codec_parsed: Option<String>,
    pub audio_codec_parsed: Option<String>,
    pub acquisition_score: Option<i32>,
    pub scoring_log: Option<String>,
    pub indexer_source: Option<String>,
    pub grabbed_release_title: Option<String>,
    pub grabbed_at: Option<String>,
    pub edition: Option<String>,
    pub original_file_path: Option<String>,
    pub release_hash: Option<String>,
}

#[async_trait]
pub trait MediaFileRepository: Send + Sync {
    async fn insert_media_file(&self, input: &InsertMediaFileInput) -> AppResult<String>;

    async fn link_file_to_episode(&self, file_id: &str, episode_id: &str) -> AppResult<()>;

    async fn list_media_files_for_title(&self, title_id: &str) -> AppResult<Vec<TitleMediaFile>>;

    async fn list_title_media_size_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleMediaSizeSummary>>;

    async fn update_media_file_analysis(
        &self,
        file_id: &str,
        analysis: MediaFileAnalysis,
    ) -> AppResult<()>;

    async fn update_media_file_source_signature(
        &self,
        file_id: &str,
        size_bytes: i64,
        source_signature_scheme: Option<String>,
        source_signature_value: Option<String>,
    ) -> AppResult<()>;

    async fn mark_scan_failed(&self, file_id: &str, error: &str) -> AppResult<()>;

    async fn get_media_file_by_id(&self, file_id: &str) -> AppResult<Option<TitleMediaFile>>;

    async fn delete_media_file(&self, file_id: &str) -> AppResult<()>;
}

#[async_trait]
pub trait WantedItemRepository: Send + Sync {
    async fn upsert_wanted_item(&self, item: &WantedItem) -> AppResult<String>;
    async fn ensure_wanted_item_seeded(&self, item: &WantedItem) -> AppResult<String> {
        let existing = find_existing_wanted_item_seed(self, item).await?;
        let mut seeded = item.clone();

        if let Some(existing) = existing.as_ref() {
            seeded.id = existing.id.clone();
            if existing.search_count > 0 {
                seeded.next_search_at = existing.next_search_at.clone();
            }
            if item.status == WantedStatus::Wanted && existing.status != WantedStatus::Wanted {
                seeded.status = existing.status;
            }
        }

        self.upsert_wanted_item(&seeded).await?;
        Ok(existing.map_or(item.id.clone(), |item| item.id))
    }
    async fn list_due_wanted_items(
        &self,
        now: &str,
        batch_limit: i64,
    ) -> AppResult<Vec<WantedItem>>;
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
    async fn schedule_wanted_item_search(
        &self,
        transition: &WantedSearchTransition,
    ) -> AppResult<()> {
        self.update_wanted_item_status(
            &transition.id,
            WantedStatus::Wanted.as_str(),
            transition.next_search_at.as_deref(),
            transition.last_search_at.as_deref(),
            transition.search_count,
            transition.current_score,
            transition.grabbed_release.as_deref(),
        )
        .await
    }
    async fn transition_wanted_to_grabbed(
        &self,
        transition: &WantedGrabTransition,
    ) -> AppResult<()> {
        self.update_wanted_item_status(
            &transition.id,
            WantedStatus::Grabbed.as_str(),
            None,
            transition.last_search_at.as_deref(),
            transition.search_count,
            transition.current_score,
            Some(&transition.grabbed_release),
        )
        .await
    }
    async fn transition_wanted_to_completed(
        &self,
        transition: &WantedCompleteTransition,
    ) -> AppResult<()> {
        self.update_wanted_item_status(
            &transition.id,
            WantedStatus::Completed.as_str(),
            None,
            transition.last_search_at.as_deref(),
            transition.search_count,
            transition.current_score,
            transition.grabbed_release.as_deref(),
        )
        .await
    }
    async fn transition_wanted_to_paused(
        &self,
        transition: &WantedPauseTransition,
    ) -> AppResult<()> {
        self.update_wanted_item_status(
            &transition.id,
            WantedStatus::Paused.as_str(),
            None,
            transition.last_search_at.as_deref(),
            transition.search_count,
            transition.current_score,
            transition.grabbed_release.as_deref(),
        )
        .await
    }
    async fn get_wanted_item_for_title(
        &self,
        title_id: &str,
        episode_id: Option<&str>,
    ) -> AppResult<Option<WantedItem>>;
    async fn delete_wanted_items_for_title(&self, title_id: &str) -> AppResult<()>;
    /// Reset `next_search_at` to now for wanted items that have been searched
    /// but never found a viable candidate (`current_score IS NULL`).
    async fn reset_fruitless_wanted_items(&self, now: &str) -> AppResult<u64>;
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

async fn find_existing_wanted_item_seed<R: WantedItemRepository + ?Sized>(
    repo: &R,
    item: &WantedItem,
) -> AppResult<Option<WantedItem>> {
    if let Some(collection_id) = item.collection_id.as_deref() {
        return Ok(repo
            .list_wanted_items(None, None, Some(&item.title_id), 500, 0)
            .await?
            .into_iter()
            .find(|existing| existing.collection_id.as_deref() == Some(collection_id)));
    }

    if let Some(episode_id) = item.episode_id.as_deref() {
        return repo
            .get_wanted_item_for_title(&item.title_id, Some(episode_id))
            .await;
    }

    Ok(repo
        .list_wanted_items(None, None, Some(&item.title_id), 500, 0)
        .await?
        .into_iter()
        .find(|existing| existing.episode_id.is_none() && existing.collection_id.is_none()))
}

#[async_trait]
pub trait PendingReleaseRepository: Send + Sync {
    async fn insert_pending_release(&self, release: &PendingRelease) -> AppResult<String>;
    async fn list_expired_pending_releases(&self, now: &str) -> AppResult<Vec<PendingRelease>>;
    async fn list_waiting_pending_releases(&self) -> AppResult<Vec<PendingRelease>>;
    async fn get_pending_release(&self, id: &str) -> AppResult<Option<PendingRelease>>;
    async fn list_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<Vec<PendingRelease>>;
    async fn update_pending_release_status(
        &self,
        id: &str,
        status: PendingReleaseStatus,
        grabbed_at: Option<&str>,
    ) -> AppResult<()>;
    async fn list_standby_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<Vec<PendingRelease>>;
    async fn delete_standby_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<()>;
    async fn list_all_standby_pending_releases(&self) -> AppResult<Vec<PendingRelease>>;
    async fn compare_and_set_pending_release_status(
        &self,
        id: &str,
        current_status: PendingReleaseStatus,
        next_status: PendingReleaseStatus,
        grabbed_at: Option<&str>,
    ) -> AppResult<bool>;
    async fn supersede_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
        except_id: &str,
    ) -> AppResult<()>;
    async fn delete_pending_releases_for_title(&self, title_id: &str) -> AppResult<()>;
}

// ── Title history ────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct NewTitleHistoryEvent {
    pub title_id: String,
    pub episode_id: Option<String>,
    pub collection_id: Option<String>,
    pub event_type: TitleHistoryEventType,
    pub source_title: Option<String>,
    pub quality: Option<String>,
    pub download_id: Option<String>,
    pub data: std::collections::HashMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default)]
pub struct TitleHistoryFilter {
    pub event_types: Option<Vec<TitleHistoryEventType>>,
    pub title_ids: Option<Vec<String>>,
    pub download_id: Option<String>,
    pub limit: usize,
    pub offset: usize,
}

#[derive(Clone, Debug)]
pub struct TitleHistoryPage {
    pub records: Vec<TitleHistoryRecord>,
    pub total_count: i64,
}

#[async_trait]
pub trait TitleHistoryRepository: Send + Sync {
    async fn record_event(&self, event: &NewTitleHistoryEvent) -> AppResult<String>;

    async fn list_history(&self, filter: &TitleHistoryFilter) -> AppResult<TitleHistoryPage>;

    async fn list_for_title(
        &self,
        title_id: &str,
        event_types: Option<&[TitleHistoryEventType]>,
        limit: usize,
        offset: usize,
    ) -> AppResult<TitleHistoryPage>;

    async fn list_for_episode(
        &self,
        episode_id: &str,
        limit: usize,
    ) -> AppResult<Vec<TitleHistoryRecord>>;

    async fn find_by_download_id(&self, download_id: &str) -> AppResult<Vec<TitleHistoryRecord>>;

    async fn delete_for_title(&self, title_id: &str) -> AppResult<()>;
}

#[derive(Clone, Debug)]
pub struct NewBlocklistEntry {
    pub title_id: String,
    pub source_title: Option<String>,
    pub source_hint: Option<String>,
    pub quality: Option<String>,
    pub download_id: Option<String>,
    pub reason: Option<String>,
    pub data: std::collections::HashMap<String, serde_json::Value>,
}

#[async_trait]
pub trait BlocklistRepository: Send + Sync {
    async fn add(&self, entry: &NewBlocklistEntry) -> AppResult<String>;

    async fn list_for_title(&self, title_id: &str, limit: usize) -> AppResult<Vec<BlocklistEntry>>;

    async fn list_all(&self, limit: usize, offset: usize) -> AppResult<(Vec<BlocklistEntry>, i64)>;

    async fn remove(&self, id: &str) -> AppResult<()>;

    async fn is_blocklisted(&self, title_id: &str, source_title: &str) -> AppResult<bool>;

    async fn delete_for_title(&self, title_id: &str) -> AppResult<()>;
}

// ── Rule sets ────────────────────────────────────────────────────────────────

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
    async fn get_rule_set_by_managed_key(&self, key: &str) -> AppResult<Option<RuleSet>>;
    async fn delete_rule_set_by_managed_key(&self, key: &str) -> AppResult<()>;
    async fn list_rule_sets_by_managed_key_prefix(&self, prefix: &str) -> AppResult<Vec<RuleSet>>;
}

#[async_trait]
pub trait PostProcessingScriptRepository: Send + Sync {
    async fn list_scripts(&self) -> AppResult<Vec<scryer_domain::PostProcessingScript>>;
    async fn get_script(&self, id: &str) -> AppResult<Option<scryer_domain::PostProcessingScript>>;
    async fn create_script(
        &self,
        script: scryer_domain::PostProcessingScript,
    ) -> AppResult<scryer_domain::PostProcessingScript>;
    async fn update_script(
        &self,
        script: scryer_domain::PostProcessingScript,
    ) -> AppResult<scryer_domain::PostProcessingScript>;
    async fn delete_script(&self, id: &str) -> AppResult<()>;
    async fn list_enabled_for_facet(
        &self,
        facet: &str,
    ) -> AppResult<Vec<scryer_domain::PostProcessingScript>>;
    async fn record_run(&self, run: scryer_domain::PostProcessingScriptRun) -> AppResult<()>;
    async fn list_runs_for_script(
        &self,
        script_id: &str,
        limit: usize,
    ) -> AppResult<Vec<scryer_domain::PostProcessingScriptRun>>;
    async fn list_runs_for_title(
        &self,
        title_id: &str,
        limit: usize,
    ) -> AppResult<Vec<scryer_domain::PostProcessingScriptRun>>;
}

#[async_trait]
pub trait PluginInstallationRepository: Send + Sync {
    async fn list_plugin_installations(&self) -> AppResult<Vec<PluginInstallation>>;
    async fn get_plugin_installation(
        &self,
        plugin_id: &str,
    ) -> AppResult<Option<PluginInstallation>>;
    async fn create_plugin_installation(
        &self,
        installation: &PluginInstallation,
        wasm_bytes: Option<&[u8]>,
    ) -> AppResult<PluginInstallation>;
    async fn update_plugin_installation(
        &self,
        installation: &PluginInstallation,
        wasm_bytes: Option<&[u8]>,
    ) -> AppResult<PluginInstallation>;
    async fn delete_plugin_installation(&self, plugin_id: &str) -> AppResult<()>;
    async fn get_enabled_plugin_wasm_bytes(
        &self,
    ) -> AppResult<Vec<(PluginInstallation, Option<Vec<u8>>)>>;
    async fn seed_builtin(
        &self,
        plugin_id: &str,
        name: &str,
        description: &str,
        version: &str,
        provider_type: &str,
    ) -> AppResult<()>;
    /// Store the JSON registry cache.
    async fn store_registry_cache(&self, json: &str) -> AppResult<()>;
    /// Retrieve the JSON registry cache. Returns None if never fetched.
    async fn get_registry_cache(&self) -> AppResult<Option<String>>;
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
    pub priority: i64,
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
        ids: std::collections::HashMap<String, String>,
        category: Option<String>,
        facet: Option<String>,
        newznab_categories: Option<Vec<String>>,
        indexer_routing: Option<IndexerRoutingPlan>,
        mode: SearchMode,
        season: Option<u32>,
        episode: Option<u32>,
        absolute_episode: Option<u32>,
        tagged_aliases: Vec<TaggedAlias>,
    ) -> AppResult<IndexerSearchResponse>;
}

/// Provides WASM-backed indexer clients for provider types not handled natively.
/// Implemented by scryer-plugins; consumed by MultiIndexerSearchClient.
pub trait IndexerPluginProvider: Send + Sync {
    fn client_for_provider(&self, config: &IndexerConfig) -> Option<Arc<dyn IndexerClient>>;
    fn available_provider_types(&self) -> Vec<String>;
    /// Scoring policies bundled with loaded plugins. Included alongside user
    /// rules when the rules engine is rebuilt.
    fn scoring_policies(&self) -> Vec<scryer_rules::UserPolicy>;
    /// Rebuild the loaded plugin set. `external_wasm_bytes` are user-installed
    /// WASM plugins that take priority over builtins. `disabled_builtins` is a
    /// list of provider_type strings for builtins the user has disabled.
    /// Returns Err if the provider does not support dynamic reload.
    fn reload_plugins(
        &self,
        external_wasm_bytes: &[&[u8]],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        let _ = (external_wasm_bytes, disabled_builtins);
        Err("this provider does not support dynamic reload".to_string())
    }
    /// Returns the config field schema declared by the plugin for this provider type.
    fn config_fields_for_provider(
        &self,
        _provider_type: &str,
    ) -> Vec<scryer_domain::ConfigFieldDef> {
        vec![]
    }
    /// Returns the human-readable plugin name for a given provider type.
    fn plugin_name_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
    /// Returns the plugin's default base URL for a provider type, if set.
    /// When present, the plugin has a fixed public endpoint and doesn't need
    /// a user-supplied base_url. Some providers may still use the standard
    /// api_key field.
    fn default_base_url_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
    /// Returns the plugin-declared rate limit (seconds between requests) for a provider type.
    /// Used when auto-creating IndexerConfig entries so the config inherits the plugin's preference.
    fn rate_limit_seconds_for_provider(&self, _provider_type: &str) -> Option<i64> {
        None
    }
    /// Returns the search capabilities declared by the plugin for a provider type.
    /// Defaults to a generic newznab-like profile for backward compat with unknown providers.
    fn capabilities_for_provider(
        &self,
        _provider_type: &str,
    ) -> scryer_domain::IndexerProviderCapabilities {
        scryer_domain::IndexerProviderCapabilities {
            rss: true,
            supported_ids: std::collections::HashMap::from([
                ("movie".into(), vec!["imdb_id".into()]),
                ("series".into(), vec!["tvdb_id".into()]),
            ]),
            deduplicates_aliases: false,
            season_param: Some("season".into()),
            episode_param: Some("ep".into()),
            query_param: Some("q".into()),
            // Legacy fields
            search: true,
            imdb_search: true,
            tvdb_search: true,
            anidb_search: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct DownloadClientAddRequest {
    pub title: Title,
    pub source_hint: Option<String>,
    pub staged_nzb: Option<StagedNzbRef>,
    pub source_kind: Option<DownloadSourceKind>,
    pub source_title: Option<String>,
    pub source_password: Option<String>,
    pub category: Option<String>,
    pub queue_priority: Option<String>,
    pub download_directory: Option<String>,
    pub release_title: Option<String>,
    pub indexer_name: Option<String>,
    pub info_hash_hint: Option<String>,
    pub seed_goal_ratio: Option<f64>,
    pub seed_goal_seconds: Option<i64>,
    pub is_recent: Option<bool>,
    pub season_pack: Option<bool>,
}

impl DownloadClientAddRequest {
    pub fn from_legacy(
        title: &Title,
        source_hint: Option<String>,
        source_kind: Option<DownloadSourceKind>,
        source_title: Option<String>,
        source_password: Option<String>,
        category: Option<String>,
    ) -> Self {
        Self {
            title: title.clone(),
            source_hint,
            staged_nzb: None,
            source_kind,
            source_title,
            source_password,
            category,
            queue_priority: None,
            download_directory: None,
            release_title: None,
            indexer_name: None,
            info_hash_hint: None,
            seed_goal_ratio: None,
            seed_goal_seconds: None,
            is_recent: None,
            season_pack: None,
        }
    }
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct DownloadClientStatus {
    pub version: Option<String>,
    pub is_localhost: Option<bool>,
    pub remote_output_roots: Vec<String>,
    pub removes_completed_downloads: Option<bool>,
    pub sorting_mode: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct DownloadClientMarkImportedRequest {
    pub client_item_id: String,
    pub info_hash: Option<String>,
    pub title_id: Option<String>,
    pub title_name: Option<String>,
    pub category: Option<String>,
    pub imported_path: Option<String>,
    pub download_path: Option<String>,
}

pub trait DownloadClientPluginProvider: Send + Sync {
    fn client_for_config(&self, config: &DownloadClientConfig) -> Option<Arc<dyn DownloadClient>>;
    fn available_provider_types(&self) -> Vec<String>;
    fn config_fields_for_provider(
        &self,
        _provider_type: &str,
    ) -> Vec<scryer_domain::ConfigFieldDef> {
        vec![]
    }
    fn plugin_name_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
    fn default_base_url_for_provider(&self, _provider_type: &str) -> Option<String> {
        None
    }
    fn accepted_inputs_for_provider(&self, _provider_type: &str) -> Vec<String> {
        vec![]
    }
    fn reload_plugins(
        &self,
        external_wasm_bytes: &[&[u8]],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        let _ = (external_wasm_bytes, disabled_builtins);
        Err("this provider does not support dynamic reload".to_string())
    }
}

// ── Notification traits ────────────────────────────────────────────────

#[async_trait]
pub trait NotificationClient: Send + Sync {
    async fn send_notification(
        &self,
        event_type: &str,
        title: &str,
        message: &str,
        metadata: &std::collections::HashMap<String, serde_json::Value>,
    ) -> AppResult<()>;
}

pub trait NotificationPluginProvider: Send + Sync {
    fn client_for_channel(
        &self,
        config: &scryer_domain::NotificationChannelConfig,
    ) -> Option<Arc<dyn NotificationClient>>;
    fn available_provider_types(&self) -> Vec<String>;
    fn config_fields_for_provider(&self, provider_type: &str)
    -> Vec<scryer_domain::ConfigFieldDef>;
    fn plugin_name_for_provider(&self, provider_type: &str) -> Option<String>;
    fn reload_plugins(
        &self,
        external_wasm_bytes: &[&[u8]],
        disabled_builtins: &[String],
    ) -> Result<(), String> {
        let _ = (external_wasm_bytes, disabled_builtins);
        Err("this provider does not support dynamic reload".to_string())
    }
}

#[async_trait]
pub trait NotificationChannelRepository: Send + Sync {
    async fn list_channels(&self) -> AppResult<Vec<scryer_domain::NotificationChannelConfig>>;
    async fn get_channel(
        &self,
        id: &str,
    ) -> AppResult<Option<scryer_domain::NotificationChannelConfig>>;
    async fn create_channel(
        &self,
        config: scryer_domain::NotificationChannelConfig,
    ) -> AppResult<scryer_domain::NotificationChannelConfig>;
    async fn update_channel(
        &self,
        config: scryer_domain::NotificationChannelConfig,
    ) -> AppResult<scryer_domain::NotificationChannelConfig>;
    async fn delete_channel(&self, id: &str) -> AppResult<()>;
}

#[async_trait]
pub trait NotificationSubscriptionRepository: Send + Sync {
    async fn list_subscriptions(&self) -> AppResult<Vec<scryer_domain::NotificationSubscription>>;
    async fn list_subscriptions_for_channel(
        &self,
        channel_id: &str,
    ) -> AppResult<Vec<scryer_domain::NotificationSubscription>>;
    async fn list_subscriptions_for_event(
        &self,
        event_type: &str,
    ) -> AppResult<Vec<scryer_domain::NotificationSubscription>>;
    async fn create_subscription(
        &self,
        sub: scryer_domain::NotificationSubscription,
    ) -> AppResult<scryer_domain::NotificationSubscription>;
    async fn update_subscription(
        &self,
        sub: scryer_domain::NotificationSubscription,
    ) -> AppResult<scryer_domain::NotificationSubscription>;
    async fn delete_subscription(&self, id: &str) -> AppResult<()>;
}

#[async_trait]
pub trait DownloadClient: Send + Sync {
    async fn submit_download(
        &self,
        request: &DownloadClientAddRequest,
    ) -> AppResult<DownloadGrabResult>;

    async fn submit_to_download_queue(
        &self,
        title: &Title,
        source_hint: Option<String>,
        source_kind: Option<DownloadSourceKind>,
        source_title: Option<String>,
        source_password: Option<String>,
        category: Option<String>,
    ) -> AppResult<DownloadGrabResult> {
        let request = DownloadClientAddRequest::from_legacy(
            title,
            source_hint,
            source_kind,
            source_title,
            source_password,
            category,
        );
        self.submit_download(&request).await
    }

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

    async fn list_history_page(
        &self,
        offset: usize,
        limit: usize,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let items = self.list_history().await?;
        Ok(items.into_iter().skip(offset).take(limit).collect())
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

    async fn mark_imported(&self, _request: &DownloadClientMarkImportedRequest) -> AppResult<()> {
        Err(AppError::Repository(
            "mark_imported is not supported for this download client".to_string(),
        ))
    }

    async fn get_client_status(&self) -> AppResult<DownloadClientStatus> {
        Err(AppError::Repository(
            "client status is not supported for this download client".to_string(),
        ))
    }

    async fn test_connection(&self) -> AppResult<String> {
        Err(AppError::Repository(
            "test connection is not supported for this download client".to_string(),
        ))
    }
}

#[derive(Clone)]
pub struct AppUseCase {
    pub services: AppServices,
    pub auth: JwtAuthConfig,
    pub facet_registry: Arc<FacetRegistry>,
    pub(crate) jwt_signing_keys: Arc<RwLock<HashMap<String, Vec<u8>>>>,
    pub(crate) jwt_signing_keys_loaded: Arc<OnceCell<()>>,
    pub(crate) jwt_signing_keys_seed_lock: Arc<Mutex<()>>,
}

pub(crate) fn normalize_release_attempt_hint(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(crate) fn normalize_release_attempt_title(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
}

pub(crate) fn normalize_release_password(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty() && *value != "0")
        .map(str::to_string)
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
    let hash = ring_digest::digest(&ring_digest::SHA256, input.as_ref().as_bytes());
    to_hex(hash.as_ref())
}

pub(crate) fn to_hex(value: &[u8]) -> String {
    let mut output = String::with_capacity(value.len() * 2);
    for byte in value {
        output.push_str(&format!("{byte:02x}"));
    }
    output
}

#[cfg(unix)]
fn statvfs_path(path: &str) -> Option<libc::statvfs> {
    use std::ffi::CString;
    let c_path = CString::new(path).ok()?;
    unsafe {
        let mut buf: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c_path.as_ptr(), &mut buf) == 0 {
            Some(buf)
        } else {
            None
        }
    }
}

fn normalize_tag(raw: String) -> String {
    raw.trim().to_lowercase()
}

fn normalize_show_text(raw: String) -> Option<String> {
    let value = raw.trim().to_string();
    if value.is_empty() { None } else { Some(value) }
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

// ── Subtitle download repository ─────────────────────────────────────────

#[async_trait]
pub trait SubtitleDownloadRepository: Send + Sync {
    async fn list_for_title(
        &self,
        title_id: &str,
    ) -> AppResult<Vec<scryer_domain::SubtitleDownload>>;
    async fn list_for_media_file(
        &self,
        media_file_id: &str,
    ) -> AppResult<Vec<scryer_domain::SubtitleDownload>>;
    async fn insert(&self, download: &scryer_domain::SubtitleDownload) -> AppResult<()>;
    async fn set_synced(&self, id: &str, synced: bool) -> AppResult<()>;
    async fn delete(&self, id: &str) -> AppResult<Option<scryer_domain::SubtitleDownload>>;
    async fn is_blacklisted(
        &self,
        media_file_id: &str,
        provider: &str,
        provider_file_id: &str,
    ) -> AppResult<bool>;
    async fn blacklist(
        &self,
        media_file_id: &str,
        provider: &str,
        provider_file_id: &str,
        language: &str,
        reason: Option<&str>,
    ) -> AppResult<()>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
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

        async fn find_by_external_id(&self, source: &str, value: &str) -> AppResult<Option<Title>> {
            let list = self.store.lock().await;
            Ok(list
                .iter()
                .find(|title| {
                    title.external_ids.iter().any(|external_id| {
                        external_id.source.eq_ignore_ascii_case(source)
                            && external_id.value == value
                    })
                })
                .cloned())
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

        async fn replace_match_state(
            &self,
            id: &str,
            external_ids: Vec<ExternalId>,
            tags: Vec<String>,
        ) -> AppResult<Title> {
            let mut list = self.store.lock().await;
            let title = list
                .iter_mut()
                .find(|entry| entry.id == id)
                .ok_or_else(|| AppError::NotFound(format!("title {}", id)))?;
            title.external_ids = external_ids;
            title.tags = tags;
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

        async fn set_folder_path(&self, id: &str, folder_path: &str) -> AppResult<()> {
            let mut list = self.store.lock().await;
            let title = list
                .iter_mut()
                .find(|entry| entry.id == id)
                .ok_or_else(|| AppError::NotFound(format!("title {}", id)))?;
            title.folder_path = Some(folder_path.to_string());
            Ok(())
        }

        async fn list_unhydrated(&self, limit: usize, language: &str) -> AppResult<Vec<Title>> {
            let list = self.store.lock().await;
            Ok(list
                .iter()
                .filter(|t| {
                    t.metadata_fetched_at.is_none()
                        || t.metadata_language.as_deref() != Some(language)
                })
                .take(limit)
                .cloned()
                .collect())
        }

        async fn clear_metadata_language_for_all(&self) -> AppResult<u64> {
            let mut list = self.store.lock().await;
            let mut count = 0u64;
            for title in list.iter_mut() {
                if title.metadata_language.is_some() {
                    title.metadata_language = None;
                    count += 1;
                }
            }
            Ok(count)
        }
    }

    #[derive(Default)]
    struct MockUserRepo {
        store: Arc<Mutex<Vec<User>>>,
        get_by_id_calls: Arc<AtomicUsize>,
        list_all_calls: Arc<AtomicUsize>,
    }

    impl MockUserRepo {
        fn get_by_id_call_count(&self) -> usize {
            self.get_by_id_calls.load(Ordering::SeqCst)
        }

        fn list_all_call_count(&self) -> usize {
            self.list_all_calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl UserRepository for MockUserRepo {
        async fn get_by_username(&self, username: &str) -> AppResult<Option<User>> {
            let users = self.store.lock().await;
            Ok(users.iter().find(|user| user.username == username).cloned())
        }

        async fn get_by_id(&self, id: &str) -> AppResult<Option<User>> {
            self.get_by_id_calls.fetch_add(1, Ordering::SeqCst);
            let users = self.store.lock().await;
            Ok(users.iter().find(|user| user.id == id).cloned())
        }

        async fn create(&self, user: User) -> AppResult<User> {
            self.store.lock().await.push(user.clone());
            Ok(user)
        }

        async fn list_all(&self) -> AppResult<Vec<User>> {
            self.list_all_calls.fetch_add(1, Ordering::SeqCst);
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
            collection_type: Option<CollectionType>,
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

        async fn update_interstitial_season_episode(
            &self,
            _collection_id: &str,
            _season_episode: Option<String>,
        ) -> AppResult<()> {
            Ok(())
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

        async fn delete_collections_for_title(&self, title_id: &str) -> AppResult<()> {
            let mut collections = self.collections.lock().await;
            collections.retain(|item| item.title_id != title_id);

            let mut episodes = self.episodes.lock().await;
            for episode in episodes.iter_mut() {
                if episode.title_id == title_id {
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

        async fn list_episodes_for_title(&self, title_id: &str) -> AppResult<Vec<Episode>> {
            let episodes = self.episodes.lock().await;
            Ok(episodes
                .iter()
                .filter(|item| item.title_id == title_id)
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
            episode_type: Option<scryer_domain::EpisodeType>,
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
            overview: Option<String>,
            tvdb_id: Option<String>,
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
            if let Some(value) = overview {
                item.overview = Some(value);
            }
            if let Some(value) = tvdb_id {
                item.tvdb_id = Some(value);
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

        async fn delete_episodes_for_title(&self, title_id: &str) -> AppResult<()> {
            let mut episodes = self.episodes.lock().await;
            episodes.retain(|item| item.title_id != title_id);
            Ok(())
        }

        async fn find_episode_by_title_and_numbers(
            &self,
            title_id: &str,
            season_number: &str,
            episode_number: &str,
        ) -> AppResult<Option<Episode>> {
            let episodes = self.episodes.lock().await;
            Ok(episodes
                .iter()
                .find(|ep| {
                    ep.title_id == title_id
                        && ep.season_number.as_deref() == Some(season_number)
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
                if let Some(c) = collections
                    .iter()
                    .filter(|c| c.title_id == *tid)
                    .filter(|c| {
                        c.collection_type == CollectionType::Movie || c.collection_index == "0"
                    })
                    .min_by(|left, right| {
                        let left_key = (
                            left.collection_type != CollectionType::Movie,
                            left.ordered_path
                                .as_deref()
                                .is_none_or(|path| path.trim().is_empty()),
                            left.collection_index.parse::<u32>().unwrap_or(u32::MAX),
                            left.collection_index.clone(),
                        );
                        let right_key = (
                            right.collection_type != CollectionType::Movie,
                            right
                                .ordered_path
                                .as_deref()
                                .is_none_or(|path| path.trim().is_empty()),
                            right.collection_index.parse::<u32>().unwrap_or(u32::MAX),
                            right.collection_index.clone(),
                        );
                        left_key.cmp(&right_key)
                    })
                {
                    out.push(PrimaryCollectionSummary {
                        title_id: tid.clone(),
                        label: c.label.clone(),
                        ordered_path: c.ordered_path.clone(),
                    });
                }
            }
            Ok(out)
        }

        async fn list_episodes_in_date_range(
            &self,
            _start_date: &str,
            _end_date: &str,
        ) -> AppResult<Vec<CalendarEpisode>> {
            Ok(vec![])
        }
    }

    #[derive(Default)]
    struct MockIndexerClient;

    #[async_trait]
    impl IndexerClient for MockIndexerClient {
        async fn search(
            &self,
            query: String,
            ids: std::collections::HashMap<String, String>,
            category: Option<String>,
            _facet: Option<String>,
            _newznab_categories: Option<Vec<String>>,
            _indexer_routing: Option<IndexerRoutingPlan>,
            _mode: SearchMode,
            _season: Option<u32>,
            _episode: Option<u32>,
            _absolute_episode: Option<u32>,
            _tagged_aliases: Vec<TaggedAlias>,
        ) -> AppResult<IndexerSearchResponse> {
            if let Some(tvdb) = ids.get("tvdb_id") {
                tracing::info!(tvdb_id = %tvdb, category = ?category, "mock nzbgeek search");
            }
            if let Some(imdb) = ids.get("imdb_id") {
                tracing::info!(imdb_id = %imdb, category = ?category, "mock nzbgeek search");
            }
            Ok(IndexerSearchResponse {
                results: vec![IndexerSearchResult {
                    source: "nzbgeek".into(),
                    title: format!("match for {query}"),
                    link: None,
                    download_url: None,
                    source_kind: Some(DownloadSourceKind::NzbUrl),
                    size_bytes: None,
                    published_at: Some("1970-01-01T00:00:00Z".into()),
                    thumbs_up: None,
                    thumbs_down: None,
                    indexer_languages: None,
                    indexer_subtitles: None,
                    indexer_grabs: None,
                    password_hint: None,
                    parsed_release_metadata: None,
                    quality_profile_decision: None,
                    extra: Default::default(),
                    guid: None,
                    info_url: None,
                }],
                api_current: None,
                api_max: None,
                grab_current: None,
                grab_max: None,
            })
        }
    }

    struct MockMetadataGateway {
        movies: HashMap<i64, MovieMetadata>,
    }

    #[async_trait]
    impl MetadataGateway for MockMetadataGateway {
        async fn search_tvdb(
            &self,
            _query: &str,
            _type_hint: &str,
        ) -> AppResult<Vec<MetadataSearchItem>> {
            Err(AppError::Repository("not implemented in tests".into()))
        }

        async fn search_tvdb_rich(
            &self,
            _query: &str,
            _type_hint: &str,
            _limit: i32,
            _language: &str,
        ) -> AppResult<Vec<RichMetadataSearchItem>> {
            Err(AppError::Repository("not implemented in tests".into()))
        }

        async fn search_tvdb_multi(
            &self,
            _query: &str,
            _limit: i32,
            _language: &str,
        ) -> AppResult<MultiMetadataSearchResult> {
            Err(AppError::Repository("not implemented in tests".into()))
        }

        async fn get_movie(&self, tvdb_id: i64, _language: &str) -> AppResult<MovieMetadata> {
            self.movies
                .get(&tvdb_id)
                .cloned()
                .ok_or_else(|| AppError::NotFound(format!("movie {tvdb_id}")))
        }

        async fn get_series(&self, _tvdb_id: i64, _language: &str) -> AppResult<SeriesMetadata> {
            Err(AppError::Repository("not implemented in tests".into()))
        }

        async fn get_metadata_bulk(
            &self,
            movie_tvdb_ids: &[i64],
            _series_tvdb_ids: &[i64],
            _language: &str,
        ) -> AppResult<BulkMetadataResult> {
            let movies = movie_tvdb_ids
                .iter()
                .filter_map(|tvdb_id| {
                    self.movies
                        .get(tvdb_id)
                        .cloned()
                        .map(|movie| (*tvdb_id, movie))
                })
                .collect();
            Ok(BulkMetadataResult {
                movies,
                series: HashMap::new(),
            })
        }

        async fn anibridge_mappings_for_episode(
            &self,
            _tvdb_id: i64,
            _season: i32,
            _episode: i32,
        ) -> AppResult<Vec<crate::library_scan::AnibridgeSourceMapping>> {
            Ok(vec![])
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

        async fn upsert_setting_json(
            &self,
            _scope: &str,
            _key_name: &str,
            _scope_id: Option<String>,
            _value_json: String,
            _source: &str,
            _updated_by_user_id: Option<String>,
        ) -> AppResult<()> {
            Ok(())
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
            config_json: Option<String>,
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
            if let Some(config_json) = config_json {
                item.config_json = Some(config_json);
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
            _base_url: Option<String>,
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

        async fn reorder(&self, ordered_ids: Vec<String>) -> AppResult<()> {
            let mut entries = self.store.lock().await;
            for (index, id) in ordered_ids.iter().enumerate() {
                if let Some(entry) = entries.iter_mut().find(|e| &e.id == id) {
                    entry.client_priority = index as i64;
                }
            }
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

    #[derive(Default, Clone)]
    struct TrackingDownloadSubmissionRepo {
        store: Arc<Mutex<Vec<DownloadSubmission>>>,
        deleted_title_ids: Arc<Mutex<Vec<String>>>,
    }

    #[derive(Default, Clone)]
    struct TrackingWantedItemRepo {
        store: Arc<Mutex<Vec<WantedItem>>>,
        release_decisions: Arc<Mutex<Vec<ReleaseDecision>>>,
    }

    #[derive(Clone)]
    struct TrackingAcquisitionStateRepo {
        download_submissions: Arc<TrackingDownloadSubmissionRepo>,
        pending_releases: Arc<TrackingPendingReleaseRepo>,
        wanted_items: Arc<TrackingWantedItemRepo>,
    }

    #[async_trait]
    impl WantedItemRepository for TrackingWantedItemRepo {
        async fn upsert_wanted_item(&self, item: &WantedItem) -> AppResult<String> {
            let mut store = self.store.lock().await;
            if let Some(existing) = store.iter_mut().find(|existing| existing.id == item.id) {
                *existing = item.clone();
            } else {
                store.push(item.clone());
            }
            Ok(item.id.clone())
        }

        async fn list_due_wanted_items(
            &self,
            now: &str,
            batch_limit: i64,
        ) -> AppResult<Vec<WantedItem>> {
            let now = chrono::DateTime::parse_from_rfc3339(now)
                .map(|value| value.with_timezone(&Utc))
                .map_err(|err| AppError::Repository(err.to_string()))?;
            let mut items: Vec<WantedItem> = self
                .store
                .lock()
                .await
                .iter()
                .filter(|item| {
                    item.status == WantedStatus::Wanted
                        && item
                            .next_search_at
                            .as_deref()
                            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                            .map(|value| value.with_timezone(&Utc) <= now)
                            .unwrap_or(true)
                })
                .cloned()
                .collect();
            items.sort_by(|left, right| left.created_at.cmp(&right.created_at));
            items.truncate(batch_limit.max(0) as usize);
            Ok(items)
        }

        async fn update_wanted_item_status(
            &self,
            id: &str,
            status: &str,
            next_search_at: Option<&str>,
            last_search_at: Option<&str>,
            search_count: i64,
            current_score: Option<i32>,
            grabbed_release: Option<&str>,
        ) -> AppResult<()> {
            let mut store = self.store.lock().await;
            let item = store
                .iter_mut()
                .find(|item| item.id == id)
                .ok_or_else(|| AppError::NotFound(format!("wanted item {id}")))?;
            item.status = WantedStatus::parse(status)
                .ok_or_else(|| AppError::Repository(format!("invalid wanted status {status}")))?;
            item.next_search_at = next_search_at.map(str::to_string);
            item.last_search_at = last_search_at.map(str::to_string);
            item.search_count = search_count;
            item.current_score = current_score;
            item.grabbed_release = grabbed_release.map(str::to_string);
            item.updated_at = Utc::now().to_rfc3339();
            Ok(())
        }

        async fn get_wanted_item_for_title(
            &self,
            title_id: &str,
            episode_id: Option<&str>,
        ) -> AppResult<Option<WantedItem>> {
            Ok(self
                .store
                .lock()
                .await
                .iter()
                .find(|item| item.title_id == title_id && item.episode_id.as_deref() == episode_id)
                .cloned())
        }

        async fn delete_wanted_items_for_title(&self, title_id: &str) -> AppResult<()> {
            self.store
                .lock()
                .await
                .retain(|item| item.title_id != title_id);
            Ok(())
        }

        async fn reset_fruitless_wanted_items(&self, _now: &str) -> AppResult<u64> {
            Ok(0)
        }

        async fn insert_release_decision(&self, decision: &ReleaseDecision) -> AppResult<String> {
            self.release_decisions.lock().await.push(decision.clone());
            Ok(decision.id.clone())
        }

        async fn get_wanted_item_by_id(&self, id: &str) -> AppResult<Option<WantedItem>> {
            Ok(self
                .store
                .lock()
                .await
                .iter()
                .find(|item| item.id == id)
                .cloned())
        }

        async fn list_wanted_items(
            &self,
            status: Option<&str>,
            media_type: Option<&str>,
            title_id: Option<&str>,
            limit: i64,
            offset: i64,
        ) -> AppResult<Vec<WantedItem>> {
            let items: Vec<WantedItem> = self
                .store
                .lock()
                .await
                .iter()
                .filter(|item| {
                    status.is_none_or(|status| item.status.as_str() == status)
                        && media_type.is_none_or(|media_type| item.media_type == media_type)
                        && title_id.is_none_or(|title_id| item.title_id == title_id)
                })
                .skip(offset.max(0) as usize)
                .take(limit.max(0) as usize)
                .cloned()
                .collect();
            Ok(items)
        }

        async fn count_wanted_items(
            &self,
            status: Option<&str>,
            media_type: Option<&str>,
            title_id: Option<&str>,
        ) -> AppResult<i64> {
            Ok(self
                .store
                .lock()
                .await
                .iter()
                .filter(|item| {
                    status.is_none_or(|status| item.status.as_str() == status)
                        && media_type.is_none_or(|media_type| item.media_type == media_type)
                        && title_id.is_none_or(|title_id| item.title_id == title_id)
                })
                .count() as i64)
        }

        async fn list_release_decisions_for_title(
            &self,
            title_id: &str,
            limit: i64,
        ) -> AppResult<Vec<ReleaseDecision>> {
            Ok(self
                .release_decisions
                .lock()
                .await
                .iter()
                .filter(|decision| decision.title_id == title_id)
                .take(limit.max(0) as usize)
                .cloned()
                .collect())
        }

        async fn list_release_decisions_for_wanted_item(
            &self,
            wanted_item_id: &str,
            limit: i64,
        ) -> AppResult<Vec<ReleaseDecision>> {
            Ok(self
                .release_decisions
                .lock()
                .await
                .iter()
                .filter(|decision| decision.wanted_item_id == wanted_item_id)
                .take(limit.max(0) as usize)
                .cloned()
                .collect())
        }
    }

    #[async_trait]
    impl AcquisitionStateRepository for TrackingAcquisitionStateRepo {
        async fn commit_successful_grab(&self, commit: &SuccessfulGrabCommit) -> AppResult<()> {
            self.download_submissions
                .record_submission(commit.download_submission.clone())
                .await?;

            self.wanted_items
                .update_wanted_item_status(
                    &commit.wanted_item_id,
                    WantedStatus::Grabbed.as_str(),
                    None,
                    commit.last_search_at.as_deref(),
                    commit.search_count,
                    commit.current_score,
                    Some(&commit.grabbed_release),
                )
                .await?;

            if let Some(pending_release_id) = commit.grabbed_pending_release_id.as_deref() {
                self.pending_releases
                    .update_pending_release_status(
                        pending_release_id,
                        PendingReleaseStatus::Grabbed,
                        commit.grabbed_at.as_deref(),
                    )
                    .await?;
            }

            let mut store = self.pending_releases.store.lock().await;
            for release in store.iter_mut() {
                let is_sibling = release.wanted_item_id == commit.wanted_item_id
                    && commit
                        .grabbed_pending_release_id
                        .as_deref()
                        .is_none_or(|pending_release_id| release.id != pending_release_id);
                let should_supersede = matches!(
                    release.status,
                    PendingReleaseStatus::Waiting | PendingReleaseStatus::Standby
                );
                if is_sibling && should_supersede {
                    release.status = PendingReleaseStatus::Superseded;
                }
            }

            Ok(())
        }
    }

    #[async_trait]
    impl DownloadSubmissionRepository for TrackingDownloadSubmissionRepo {
        async fn record_submission(&self, submission: DownloadSubmission) -> AppResult<()> {
            let mut entries = self.store.lock().await;
            if let Some(existing) = entries.iter_mut().find(|entry| {
                entry.download_client_type == submission.download_client_type
                    && entry.download_client_item_id == submission.download_client_item_id
            }) {
                *existing = submission;
            } else {
                entries.push(submission);
            }
            Ok(())
        }

        async fn find_by_client_item_id(
            &self,
            download_client_type: &str,
            download_client_item_id: &str,
        ) -> AppResult<Option<DownloadSubmission>> {
            let entries = self.store.lock().await;
            Ok(entries
                .iter()
                .find(|entry| {
                    entry.download_client_type == download_client_type
                        && entry.download_client_item_id == download_client_item_id
                })
                .cloned())
        }

        async fn list_for_title(&self, title_id: &str) -> AppResult<Vec<DownloadSubmission>> {
            let entries = self.store.lock().await;
            Ok(entries
                .iter()
                .filter(|entry| entry.title_id == title_id)
                .cloned()
                .collect())
        }

        async fn delete_for_title(&self, title_id: &str) -> AppResult<()> {
            self.deleted_title_ids
                .lock()
                .await
                .push(title_id.to_string());
            self.store
                .lock()
                .await
                .retain(|entry| entry.title_id != title_id);
            Ok(())
        }

        async fn delete_by_client_item_id(&self, download_client_item_id: &str) -> AppResult<()> {
            self.store
                .lock()
                .await
                .retain(|entry| entry.download_client_item_id != download_client_item_id);
            Ok(())
        }

        async fn update_tracked_state(&self, _: &str, _: &str, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn get_tracked_state(&self, _: &str, _: &str) -> AppResult<Option<String>> {
            Ok(None)
        }
    }

    #[derive(Default, Clone)]
    struct TrackingPendingReleaseRepo {
        store: Arc<Mutex<Vec<PendingRelease>>>,
        deleted_title_ids: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl PendingReleaseRepository for TrackingPendingReleaseRepo {
        async fn insert_pending_release(&self, release: &PendingRelease) -> AppResult<String> {
            self.store.lock().await.push(release.clone());
            Ok(release.id.clone())
        }

        async fn list_expired_pending_releases(&self, _: &str) -> AppResult<Vec<PendingRelease>> {
            Ok(vec![])
        }

        async fn list_waiting_pending_releases(&self) -> AppResult<Vec<PendingRelease>> {
            Ok(vec![])
        }

        async fn get_pending_release(&self, id: &str) -> AppResult<Option<PendingRelease>> {
            Ok(self
                .store
                .lock()
                .await
                .iter()
                .find(|release| release.id == id)
                .cloned())
        }

        async fn list_pending_releases_for_wanted_item(
            &self,
            wanted_item_id: &str,
        ) -> AppResult<Vec<PendingRelease>> {
            Ok(self
                .store
                .lock()
                .await
                .iter()
                .filter(|release| {
                    release.wanted_item_id == wanted_item_id
                        && release.status == PendingReleaseStatus::Waiting
                })
                .cloned()
                .collect())
        }

        async fn update_pending_release_status(
            &self,
            id: &str,
            status: PendingReleaseStatus,
            grabbed_at: Option<&str>,
        ) -> AppResult<()> {
            if let Some(release) = self
                .store
                .lock()
                .await
                .iter_mut()
                .find(|release| release.id == id)
            {
                release.status = status;
                release.grabbed_at = grabbed_at.map(str::to_string);
            }
            Ok(())
        }

        async fn list_standby_pending_releases_for_wanted_item(
            &self,
            wanted_item_id: &str,
        ) -> AppResult<Vec<PendingRelease>> {
            Ok(self
                .store
                .lock()
                .await
                .iter()
                .filter(|release| {
                    release.wanted_item_id == wanted_item_id
                        && release.status == PendingReleaseStatus::Standby
                })
                .cloned()
                .collect())
        }

        async fn delete_standby_pending_releases_for_wanted_item(
            &self,
            wanted_item_id: &str,
        ) -> AppResult<()> {
            self.store.lock().await.retain(|release| {
                !(release.wanted_item_id == wanted_item_id
                    && release.status == PendingReleaseStatus::Standby)
            });
            Ok(())
        }

        async fn list_all_standby_pending_releases(&self) -> AppResult<Vec<PendingRelease>> {
            Ok(self
                .store
                .lock()
                .await
                .iter()
                .filter(|release| release.status == PendingReleaseStatus::Standby)
                .cloned()
                .collect())
        }

        async fn compare_and_set_pending_release_status(
            &self,
            id: &str,
            current_status: PendingReleaseStatus,
            next_status: PendingReleaseStatus,
            grabbed_at: Option<&str>,
        ) -> AppResult<bool> {
            let mut store = self.store.lock().await;
            let Some(release) = store.iter_mut().find(|release| release.id == id) else {
                return Ok(false);
            };
            if release.status != current_status {
                return Ok(false);
            }
            release.status = next_status;
            release.grabbed_at = grabbed_at.map(str::to_string);
            Ok(true)
        }

        async fn supersede_pending_releases_for_wanted_item(
            &self,
            wanted_item_id: &str,
            except_id: &str,
        ) -> AppResult<()> {
            for release in self.store.lock().await.iter_mut() {
                if release.wanted_item_id == wanted_item_id
                    && release.id != except_id
                    && release.status == PendingReleaseStatus::Waiting
                {
                    release.status = PendingReleaseStatus::Superseded;
                }
            }
            Ok(())
        }

        async fn delete_pending_releases_for_title(&self, title_id: &str) -> AppResult<()> {
            self.deleted_title_ids
                .lock()
                .await
                .push(title_id.to_string());
            Ok(())
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
                events.retain(|event| event.title_id.as_ref() == Some(&id));
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

    #[derive(Default, Clone)]
    struct StubDownloadClient {
        queue_items: Arc<Mutex<Vec<DownloadQueueItem>>>,
        history_items: Arc<Mutex<Vec<DownloadQueueItem>>>,
        deleted_items: Arc<Mutex<Vec<(String, bool)>>>,
        submitted_release_titles: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl DownloadClient for StubDownloadClient {
        async fn submit_download(
            &self,
            request: &DownloadClientAddRequest,
        ) -> AppResult<DownloadGrabResult> {
            self.submitted_release_titles.lock().await.push(
                request
                    .release_title
                    .clone()
                    .unwrap_or_else(|| request.title.name.clone()),
            );
            Ok(DownloadGrabResult {
                job_id: format!("job-for-{}", request.title.id),
                client_type: "nzbget".to_string(),
            })
        }

        async fn list_queue(&self) -> AppResult<Vec<DownloadQueueItem>> {
            Ok(self.queue_items.lock().await.clone())
        }

        async fn list_history(&self) -> AppResult<Vec<DownloadQueueItem>> {
            Ok(self.history_items.lock().await.clone())
        }

        async fn delete_queue_item(&self, id: &str, is_history: bool) -> AppResult<()> {
            self.deleted_items
                .lock()
                .await
                .push((id.to_string(), is_history));
            Ok(())
        }
    }

    fn bootstrap() -> (AppUseCase, User) {
        bootstrap_with_user_repo(Arc::new(MockUserRepo::default()))
    }

    fn bootstrap_with_user_repo(users: Arc<MockUserRepo>) -> (AppUseCase, User) {
        let titles = Arc::new(MockTitleRepo::default());
        let shows = Arc::new(MockShowRepo::default());
        let events = Arc::new(MockEventRepo::default());
        let indexer_configs = Arc::new(MockIndexerConfigRepo::default());
        let download_client_configs = Arc::new(MockDownloadClientConfigRepo::default());
        let release_attempts = Arc::new(MockReleaseAttemptRepo);
        let settings = Arc::new(MockSettingsRepo);
        let quality_profiles = Arc::new(MockQualityProfileRepo);
        let download_client = Arc::new(StubDownloadClient::default());
        let indexer_client = Arc::new(MockIndexerClient);

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
        let mut registry = FacetRegistry::new();
        registry.register(Arc::new(MovieFacetHandler));
        registry.register(Arc::new(SeriesFacetHandler::new(
            scryer_domain::MediaFacet::Series,
        )));
        registry.register(Arc::new(SeriesFacetHandler::new(
            scryer_domain::MediaFacet::Anime,
        )));
        let app = AppUseCase::new(
            services,
            JwtAuthConfig {
                issuer: "scryer-test".to_string(),
                access_ttl_seconds: 3600,
                jwt_signing_salt: "test-salt".to_string(),
            },
            Arc::new(registry),
        );

        (app, User::new_admin("admin"))
    }

    fn bootstrap_with_cleanup_tracking(
        download_client: Arc<StubDownloadClient>,
        download_submissions: Arc<TrackingDownloadSubmissionRepo>,
        pending_releases: Arc<TrackingPendingReleaseRepo>,
    ) -> (AppUseCase, User) {
        let titles = Arc::new(MockTitleRepo::default());
        let shows = Arc::new(MockShowRepo::default());
        let users = Arc::new(MockUserRepo::default());
        let events = Arc::new(MockEventRepo::default());
        let indexer_configs = Arc::new(MockIndexerConfigRepo::default());
        let download_client_configs = Arc::new(MockDownloadClientConfigRepo::default());
        let release_attempts = Arc::new(MockReleaseAttemptRepo);
        let settings = Arc::new(MockSettingsRepo);
        let quality_profiles = Arc::new(MockQualityProfileRepo);
        let indexer_client = Arc::new(MockIndexerClient);

        let mut services = AppServices::with_default_channels(
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
        services.download_submissions = download_submissions;
        services.pending_releases = pending_releases;

        let mut registry = FacetRegistry::new();
        registry.register(Arc::new(MovieFacetHandler));
        registry.register(Arc::new(SeriesFacetHandler::new(
            scryer_domain::MediaFacet::Series,
        )));
        registry.register(Arc::new(SeriesFacetHandler::new(
            scryer_domain::MediaFacet::Anime,
        )));
        let app = AppUseCase::new(
            services,
            JwtAuthConfig {
                issuer: "scryer-test".to_string(),
                access_ttl_seconds: 3600,
                jwt_signing_salt: "test-salt".to_string(),
            },
            Arc::new(registry),
        );

        (app, User::new_admin("admin"))
    }

    fn bootstrap_with_acquisition_tracking(
        download_client: Arc<StubDownloadClient>,
        download_submissions: Arc<TrackingDownloadSubmissionRepo>,
        pending_releases: Arc<TrackingPendingReleaseRepo>,
        wanted_items: Arc<TrackingWantedItemRepo>,
    ) -> (AppUseCase, User) {
        let (mut app, user) = bootstrap_with_cleanup_tracking(
            download_client,
            download_submissions.clone(),
            pending_releases.clone(),
        );
        app.services.acquisition_state = Arc::new(TrackingAcquisitionStateRepo {
            download_submissions,
            pending_releases,
            wanted_items: wanted_items.clone(),
        });
        app.services.wanted_items = wanted_items;
        (app, user)
    }

    #[tokio::test]
    async fn add_title_and_queue_sends_download_job() {
        let (app, user) = bootstrap();
        let (title, job_id) = app
            .add_title_and_queue_download(
                &user,
                NewTitle {
                    name: "Show One".into(),
                    facet: MediaFacet::Series,
                    monitored: true,
                    tags: vec![],
                    external_ids: vec![],
                    min_availability: None,

                    ..Default::default()
                },
                None,
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

                ..Default::default()
            },
        )
        .await
        .expect("create movie");

        app.add_title(
            &user,
            NewTitle {
                name: "Show B".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,

                ..Default::default()
            },
        )
        .await
        .expect("create tv");

        let tvs = app
            .list_titles(&user, Some(MediaFacet::Series), None)
            .await
            .expect("list titles");

        assert!(tvs.iter().all(|item| item.facet == MediaFacet::Series));
    }

    #[tokio::test]
    async fn search_indexer_requires_query() {
        let (app, user) = bootstrap();

        let result = app
            .search_indexers(&user, "   ".into(), None, None, None, None)
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

                    ..Default::default()
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
    async fn delete_title_cancels_queue_items_linked_via_submission_metadata() {
        let download_client = Arc::new(StubDownloadClient::default());
        let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
        let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
        let (app, user) = bootstrap_with_cleanup_tracking(
            download_client.clone(),
            download_submissions.clone(),
            pending_releases.clone(),
        );

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

                    ..Default::default()
                },
            )
            .await
            .expect("create title");

        download_submissions
            .record_submission(DownloadSubmission {
                title_id: created.id.clone(),
                facet: "movie".to_string(),
                download_client_type: "sabnzbd".to_string(),
                download_client_item_id: "queue-fallback".to_string(),
                source_title: Some(created.name.clone()),
                collection_id: None,
            })
            .await
            .expect("record submission");

        *download_client.queue_items.lock().await = vec![
            DownloadQueueItem {
                id: "queue-direct".to_string(),
                title_id: Some(created.id.clone()),
                title_name: created.name.clone(),
                facet: Some("movie".to_string()),
                client_id: "primary".to_string(),
                client_name: "Primary".to_string(),
                client_type: "nzbget".to_string(),
                state: DownloadQueueState::Queued,
                progress_percent: 0,
                size_bytes: None,
                remaining_seconds: None,
                queued_at: None,
                last_updated_at: None,
                attention_required: false,
                attention_reason: None,
                download_client_item_id: "queue-direct".to_string(),
                import_status: None,
                import_error_message: None,
                imported_at: None,
                is_scryer_origin: true,
                tracked_state: None,
                tracked_status: None,
                tracked_status_messages: Vec::new(),
                tracked_match_type: None,
            },
            DownloadQueueItem {
                id: "queue-fallback".to_string(),
                title_id: None,
                title_name: created.name.clone(),
                facet: None,
                client_id: "primary".to_string(),
                client_name: "Primary".to_string(),
                client_type: "sabnzbd".to_string(),
                state: DownloadQueueState::Queued,
                progress_percent: 0,
                size_bytes: None,
                remaining_seconds: None,
                queued_at: None,
                last_updated_at: None,
                attention_required: false,
                attention_reason: None,
                download_client_item_id: "queue-fallback".to_string(),
                import_status: None,
                import_error_message: None,
                imported_at: None,
                is_scryer_origin: false,
                tracked_state: None,
                tracked_status: None,
                tracked_status_messages: Vec::new(),
                tracked_match_type: None,
            },
            DownloadQueueItem {
                id: "queue-unrelated".to_string(),
                title_id: None,
                title_name: "Other".to_string(),
                facet: None,
                client_id: "primary".to_string(),
                client_name: "Primary".to_string(),
                client_type: "sabnzbd".to_string(),
                state: DownloadQueueState::Queued,
                progress_percent: 0,
                size_bytes: None,
                remaining_seconds: None,
                queued_at: None,
                last_updated_at: None,
                attention_required: false,
                attention_reason: None,
                download_client_item_id: "queue-unrelated".to_string(),
                import_status: None,
                import_error_message: None,
                imported_at: None,
                is_scryer_origin: false,
                tracked_state: None,
                tracked_status: None,
                tracked_status_messages: Vec::new(),
                tracked_match_type: None,
            },
        ];

        app.delete_title(&user, &created.id, false)
            .await
            .expect("delete title");

        let deleted_items = download_client.deleted_items.lock().await.clone();
        assert_eq!(
            deleted_items,
            vec![
                ("queue-direct".to_string(), false),
                ("queue-fallback".to_string(), false),
            ]
        );
        assert_eq!(
            pending_releases.deleted_title_ids.lock().await.clone(),
            vec![created.id.clone()]
        );
        assert_eq!(
            download_submissions.deleted_title_ids.lock().await.clone(),
            vec![created.id.clone()]
        );
        assert!(
            download_submissions
                .store
                .lock()
                .await
                .iter()
                .all(|entry| entry.title_id != created.id)
        );
    }

    #[tokio::test]
    async fn list_download_queue_does_not_treat_stub_submission_as_origin() {
        let download_client = Arc::new(StubDownloadClient::default());
        let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
        let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
        let (app, user) = bootstrap_with_cleanup_tracking(
            download_client.clone(),
            download_submissions.clone(),
            pending_releases,
        );

        app.create_download_client_config(
            &user,
            NewDownloadClientConfig {
                name: "SABnzbd".to_string(),
                client_type: "sabnzbd".to_string(),
                config_json: "{}".to_string(),
                client_priority: 1,
                is_enabled: true,
            },
        )
        .await
        .expect("create download client config");

        download_submissions
            .record_submission(DownloadSubmission {
                title_id: String::new(),
                facet: String::new(),
                download_client_type: "sabnzbd".to_string(),
                download_client_item_id: "foreign-stub".to_string(),
                source_title: Some("Foreign Download".to_string()),
                collection_id: None,
            })
            .await
            .expect("record stub submission");

        *download_client.queue_items.lock().await = vec![DownloadQueueItem {
            id: "foreign-stub".to_string(),
            title_id: None,
            title_name: "Foreign Download".to_string(),
            facet: None,
            client_id: "primary".to_string(),
            client_name: "Primary".to_string(),
            client_type: "sabnzbd".to_string(),
            state: DownloadQueueState::Queued,
            progress_percent: 0,
            size_bytes: None,
            remaining_seconds: None,
            queued_at: None,
            last_updated_at: None,
            attention_required: false,
            attention_reason: None,
            download_client_item_id: "foreign-stub".to_string(),
            import_status: None,
            import_error_message: None,
            imported_at: None,
            is_scryer_origin: false,
            tracked_state: None,
            tracked_status: None,
            tracked_status_messages: Vec::new(),
            tracked_match_type: None,
        }];

        let items = app
            .list_download_queue(&user, true, false)
            .await
            .expect("list queue");

        assert_eq!(items.len(), 1);
        assert!(!items[0].is_scryer_origin);
        assert!(items[0].title_id.is_none());
        assert!(items[0].facet.is_none());
    }

    fn failed_history_item(download_client_item_id: &str, title_name: &str) -> DownloadQueueItem {
        DownloadQueueItem {
            id: download_client_item_id.to_string(),
            title_id: None,
            title_name: title_name.to_string(),
            facet: Some("movie".to_string()),
            client_id: "primary".to_string(),
            client_name: "Primary".to_string(),
            client_type: "nzbget".to_string(),
            state: DownloadQueueState::Failed,
            progress_percent: 100,
            size_bytes: None,
            remaining_seconds: None,
            queued_at: None,
            last_updated_at: None,
            attention_required: true,
            attention_reason: Some("corrupt archive".to_string()),
            download_client_item_id: download_client_item_id.to_string(),
            import_status: None,
            import_error_message: None,
            imported_at: None,
            is_scryer_origin: true,
            tracked_state: None,
            tracked_status: None,
            tracked_status_messages: Vec::new(),
            tracked_match_type: None,
        }
    }

    #[tokio::test]
    async fn acquisition_cycle_retries_standby_candidate_after_failed_grab() {
        let download_client = Arc::new(StubDownloadClient::default());
        let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
        let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
        let wanted_items = Arc::new(TrackingWantedItemRepo::default());
        let (app, user) = bootstrap_with_acquisition_tracking(
            download_client.clone(),
            download_submissions.clone(),
            pending_releases.clone(),
            wanted_items.clone(),
        );

        let title = app
            .add_title(
                &user,
                NewTitle {
                    name: "Failure Recovery".into(),
                    facet: MediaFacet::Movie,
                    monitored: true,
                    tags: vec![],
                    external_ids: vec![],
                    min_availability: None,
                    ..Default::default()
                },
            )
            .await
            .expect("create title");

        let wanted = WantedItem {
            id: Id::new().0,
            title_id: title.id.clone(),
            title_name: Some(title.name.clone()),
            episode_id: None,
            collection_id: None,
            season_number: None,
            media_type: "movie".to_string(),
            search_phase: "initial".to_string(),
            next_search_at: None,
            last_search_at: Some((Utc::now() - chrono::Duration::minutes(5)).to_rfc3339()),
            search_count: 1,
            baseline_date: Some(
                (Utc::now() - chrono::Duration::days(30))
                    .format("%Y-%m-%d")
                    .to_string(),
            ),
            status: WantedStatus::Grabbed,
            grabbed_release: Some(
                serde_json::json!({
                    "title": "Failed.Release.1080p.WEB-DL",
                    "score": 100,
                    "grabbed_at": Utc::now().to_rfc3339(),
                })
                .to_string(),
            ),
            current_score: None,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        };
        wanted_items
            .upsert_wanted_item(&wanted)
            .await
            .expect("seed wanted item");

        pending_releases
            .insert_pending_release(&PendingRelease {
                id: Id::new().0,
                wanted_item_id: wanted.id.clone(),
                title_id: title.id.clone(),
                release_title: "Standby.Release.1080p.WEB-DL".to_string(),
                release_url: Some("https://example.com/standby.nzb".to_string()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                release_size_bytes: Some(1_000),
                release_score: 150,
                scoring_log_json: None,
                indexer_source: Some("nzbgeek".to_string()),
                release_guid: Some("guid-standby".to_string()),
                added_at: Utc::now().to_rfc3339(),
                delay_until: Utc::now().to_rfc3339(),
                status: PendingReleaseStatus::Standby,
                grabbed_at: None,
                source_password: None,
                published_at: Some(Utc::now().to_rfc3339()),
                info_hash: None,
            })
            .await
            .expect("seed standby");

        download_submissions
            .record_submission(DownloadSubmission {
                title_id: title.id.clone(),
                facet: "movie".to_string(),
                download_client_type: "nzbget".to_string(),
                download_client_item_id: "failed-job".to_string(),
                source_title: Some("Failed.Release.1080p.WEB-DL".to_string()),
                collection_id: None,
            })
            .await
            .expect("record failed submission");

        *download_client.history_items.lock().await = vec![failed_history_item(
            "failed-job",
            "Failed.Release.1080p.WEB-DL",
        )];

        app.run_acquisition_cycle_once().await;

        let updated = wanted_items
            .get_wanted_item_by_id(&wanted.id)
            .await
            .expect("get wanted")
            .expect("wanted exists");
        assert_eq!(updated.status, WantedStatus::Grabbed);
        assert_eq!(updated.current_score, None);
        assert!(
            updated
                .grabbed_release
                .as_deref()
                .unwrap_or_default()
                .contains("Standby.Release.1080p.WEB-DL")
        );

        assert!(
            pending_releases
                .list_all_standby_pending_releases()
                .await
                .expect("list standby")
                .is_empty()
        );
        assert!(pending_releases.store.lock().await.iter().any(|release| {
            release.release_title == "Standby.Release.1080p.WEB-DL"
                && release.status == PendingReleaseStatus::Grabbed
        }));

        let submissions = download_submissions.store.lock().await.clone();
        assert!(
            !submissions
                .iter()
                .any(|submission| submission.download_client_item_id == "failed-job")
        );
        assert!(submissions.iter().any(|submission| {
            submission.download_client_item_id == format!("job-for-{}", title.id)
                && submission.source_title.as_deref() == Some("Standby.Release.1080p.WEB-DL")
        }));

        assert_eq!(
            download_client
                .submitted_release_titles
                .lock()
                .await
                .clone(),
            vec!["Standby.Release.1080p.WEB-DL".to_string()]
        );
    }

    #[tokio::test]
    async fn tracked_download_failure_reuses_standby_recovery_policy() {
        let download_client = Arc::new(StubDownloadClient::default());
        let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
        let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
        let wanted_items = Arc::new(TrackingWantedItemRepo::default());
        let (app, user) = bootstrap_with_acquisition_tracking(
            download_client.clone(),
            download_submissions.clone(),
            pending_releases.clone(),
            wanted_items.clone(),
        );

        let title = app
            .add_title(
                &user,
                NewTitle {
                    name: "Tracked Failure Recovery".into(),
                    facet: MediaFacet::Movie,
                    monitored: true,
                    tags: vec![],
                    external_ids: vec![],
                    min_availability: None,
                    ..Default::default()
                },
            )
            .await
            .expect("create title");

        let wanted = WantedItem {
            id: Id::new().0,
            title_id: title.id.clone(),
            title_name: Some(title.name.clone()),
            episode_id: None,
            collection_id: None,
            season_number: None,
            media_type: "movie".to_string(),
            search_phase: "initial".to_string(),
            next_search_at: None,
            last_search_at: Some((Utc::now() - chrono::Duration::minutes(5)).to_rfc3339()),
            search_count: 1,
            baseline_date: Some(
                (Utc::now() - chrono::Duration::days(30))
                    .format("%Y-%m-%d")
                    .to_string(),
            ),
            status: WantedStatus::Grabbed,
            grabbed_release: Some(
                serde_json::json!({
                    "title": "Failed.Release.1080p.WEB-DL",
                    "score": 100,
                    "grabbed_at": Utc::now().to_rfc3339(),
                })
                .to_string(),
            ),
            current_score: None,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        };
        wanted_items
            .upsert_wanted_item(&wanted)
            .await
            .expect("seed wanted item");

        pending_releases
            .insert_pending_release(&PendingRelease {
                id: Id::new().0,
                wanted_item_id: wanted.id.clone(),
                title_id: title.id.clone(),
                release_title: "Standby.Release.1080p.WEB-DL".to_string(),
                release_url: Some("https://example.com/standby.nzb".to_string()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                release_size_bytes: Some(1_000),
                release_score: 150,
                scoring_log_json: None,
                indexer_source: Some("nzbgeek".to_string()),
                release_guid: Some("guid-standby".to_string()),
                added_at: Utc::now().to_rfc3339(),
                delay_until: Utc::now().to_rfc3339(),
                status: PendingReleaseStatus::Standby,
                grabbed_at: None,
                source_password: None,
                published_at: Some(Utc::now().to_rfc3339()),
                info_hash: None,
            })
            .await
            .expect("seed standby");

        download_submissions
            .record_submission(DownloadSubmission {
                title_id: title.id.clone(),
                facet: "movie".to_string(),
                download_client_type: "nzbget".to_string(),
                download_client_item_id: "failed-job".to_string(),
                source_title: Some("Failed.Release.1080p.WEB-DL".to_string()),
                collection_id: None,
            })
            .await
            .expect("record failed submission");

        let mut tracked_download = crate::tracked_downloads::TrackedDownload {
            id: "nzbget:failed-job".to_string(),
            client_id: "primary".to_string(),
            client_type: "nzbget".to_string(),
            client_item: failed_history_item("failed-job", "Failed.Release.1080p.WEB-DL"),
            state: scryer_domain::TrackedDownloadState::FailedPending,
            status: scryer_domain::TrackedDownloadStatus::Error,
            status_messages: Vec::new(),
            title_id: Some(title.id.clone()),
            facet: Some("movie".to_string()),
            source_title: Some("Failed.Release.1080p.WEB-DL".to_string()),
            indexer: None,
            added_at: None,
            notified_manual_interaction: false,
            match_type: scryer_domain::TitleMatchType::Submission,
            is_trackable: true,
            import_attempted: false,
        };

        crate::failed_download_handler::process_failed(&app, &mut tracked_download).await;

        assert_eq!(
            tracked_download.state,
            scryer_domain::TrackedDownloadState::Failed
        );

        let updated = wanted_items
            .get_wanted_item_by_id(&wanted.id)
            .await
            .expect("get wanted")
            .expect("wanted exists");
        assert_eq!(updated.status, WantedStatus::Grabbed);
        assert!(
            updated
                .grabbed_release
                .as_deref()
                .unwrap_or_default()
                .contains("Standby.Release.1080p.WEB-DL")
        );

        assert!(
            pending_releases
                .list_all_standby_pending_releases()
                .await
                .expect("list standby")
                .is_empty()
        );
        assert!(pending_releases.store.lock().await.iter().any(|release| {
            release.release_title == "Standby.Release.1080p.WEB-DL"
                && release.status == PendingReleaseStatus::Grabbed
        }));

        let submissions = download_submissions.store.lock().await.clone();
        assert!(
            !submissions
                .iter()
                .any(|submission| submission.download_client_item_id == "failed-job")
        );
        assert!(submissions.iter().any(|submission| {
            submission.download_client_item_id == format!("job-for-{}", title.id)
                && submission.source_title.as_deref() == Some("Standby.Release.1080p.WEB-DL")
        }));

        assert_eq!(
            download_client
                .submitted_release_titles
                .lock()
                .await
                .clone(),
            vec!["Standby.Release.1080p.WEB-DL".to_string()]
        );
    }

    #[tokio::test]
    async fn acquisition_cycle_prunes_stale_standby_rows_for_non_grabbed_items() {
        let download_client = Arc::new(StubDownloadClient::default());
        let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
        let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
        let wanted_items = Arc::new(TrackingWantedItemRepo::default());
        let (app, user) = bootstrap_with_acquisition_tracking(
            download_client,
            download_submissions,
            pending_releases.clone(),
            wanted_items.clone(),
        );

        let title = app
            .add_title(
                &user,
                NewTitle {
                    name: "Prune Me".into(),
                    facet: MediaFacet::Movie,
                    monitored: true,
                    tags: vec![],
                    external_ids: vec![],
                    min_availability: None,
                    ..Default::default()
                },
            )
            .await
            .expect("create title");

        let wanted = WantedItem {
            id: Id::new().0,
            title_id: title.id.clone(),
            title_name: Some(title.name.clone()),
            episode_id: None,
            collection_id: None,
            season_number: None,
            media_type: "movie".to_string(),
            search_phase: "initial".to_string(),
            next_search_at: None,
            last_search_at: None,
            search_count: 0,
            baseline_date: None,
            status: WantedStatus::Wanted,
            grabbed_release: None,
            current_score: None,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        };
        wanted_items
            .upsert_wanted_item(&wanted)
            .await
            .expect("seed wanted item");

        pending_releases
            .insert_pending_release(&PendingRelease {
                id: Id::new().0,
                wanted_item_id: wanted.id.clone(),
                title_id: title.id.clone(),
                release_title: "Stale.Standby.Release".to_string(),
                release_url: Some("https://example.com/stale.nzb".to_string()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                release_size_bytes: None,
                release_score: 100,
                scoring_log_json: None,
                indexer_source: Some("nzbgeek".to_string()),
                release_guid: Some("guid-stale".to_string()),
                added_at: (Utc::now() - chrono::Duration::hours(30)).to_rfc3339(),
                delay_until: Utc::now().to_rfc3339(),
                status: PendingReleaseStatus::Standby,
                grabbed_at: None,
                source_password: None,
                published_at: None,
                info_hash: None,
            })
            .await
            .expect("seed stale standby");

        app.run_acquisition_cycle_once().await;

        assert!(
            pending_releases
                .list_all_standby_pending_releases()
                .await
                .expect("list standby")
                .is_empty()
        );
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

                    ..Default::default()
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
                    facet: MediaFacet::Series,
                    monitored: true,
                    tags: vec![],
                    external_ids: vec![],
                    min_availability: None,

                    ..Default::default()
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
                "standard".into(),
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
    async fn anime_hybrid_movie_mapping_creates_interstitial_collection() {
        let (mut app, user) = bootstrap();
        app.services.metadata_gateway = Arc::new(MockMetadataGateway {
            movies: HashMap::from([(
                131_963,
                MovieMetadata {
                    tvdb_id: 131_963,
                    name: "Mugen Train".into(),
                    slug: "mugen-train".into(),
                    year: Some(2020),
                    content_status: "Released".into(),
                    overview: "A train mission.".into(),
                    poster_url: "https://example.com/mugen-train.jpg".into(),
                    banner_url: None,
                    background_url: None,
                    language: "eng".into(),
                    runtime_minutes: 117,
                    sort_title: "Mugen Train".into(),
                    imdb_id: "tt11032374".into(),
                    anidb_id: None,
                    genres: vec!["Action".into(), "Anime".into()],
                    studio: "ufotable".into(),
                    tmdb_release_date: Some("2020-10-16".into()),
                },
            )]),
        });
        let title = app
            .add_title(
                &user,
                NewTitle {
                    name: "Demon Slayer".into(),
                    facet: MediaFacet::Anime,
                    monitored: true,
                    tags: vec![],
                    external_ids: vec![ExternalId {
                        source: "tvdb_id".into(),
                        value: "348545".into(),
                    }],
                    min_availability: None,
                    ..Default::default()
                },
            )
            .await
            .expect("create title");

        let seasons = vec![
            SeasonMetadata {
                tvdb_id: 10,
                number: 0,
                label: "Specials".into(),
                episode_type: "special".into(),
            },
            SeasonMetadata {
                tvdb_id: 11,
                number: 1,
                label: "Season 1".into(),
                episode_type: "official".into(),
            },
        ];
        let episodes = vec![
            EpisodeMetadata {
                tvdb_id: 1001,
                episode_number: 1,
                name: "Cruelty".into(),
                aired: "2019-04-06".into(),
                runtime_minutes: 24,
                is_filler: false,
                is_recap: false,
                overview: "Episode 1".into(),
                absolute_number: "1".into(),
                season_number: 1,
            },
            EpisodeMetadata {
                tvdb_id: 1002,
                episode_number: 26,
                name: "New Mission".into(),
                aired: "2019-09-28".into(),
                runtime_minutes: 24,
                is_filler: false,
                is_recap: false,
                overview: "Episode 26".into(),
                absolute_number: "26".into(),
                season_number: 1,
            },
            EpisodeMetadata {
                tvdb_id: 2001,
                episode_number: 1,
                name: "Mugen Train".into(),
                aired: "2020-10-10".into(),
                runtime_minutes: 117,
                is_filler: false,
                is_recap: false,
                overview: "Special cut".into(),
                absolute_number: String::new(),
                season_number: 0,
            },
        ];
        let anime_mappings = vec![AnimeMapping {
            mal_id: Some(40456),
            anilist_id: None,
            anidb_id: None,
            kitsu_id: None,
            thetvdb_id: Some(348545),
            themoviedb_id: Some(438759),
            alt_tvdb_id: Some(131_963),
            thetvdb_season: Some(0),
            score: None,
            anime_media_type: "TV".into(),
            global_media_type: "series".into(),
            status: "finished".into(),
            mapping_type: String::new(),
            episode_mappings: vec![AnimeEpisodeMapping {
                tvdb_season: 0,
                episode_start: 1,
                episode_end: 1,
            }],
        }];
        let anime_movies = vec![AnimeMovie {
            movie_tvdb_id: Some(131_963),
            movie_tmdb_id: Some(438759),
            movie_imdb_id: Some("tt11032374".into()),
            movie_mal_id: Some(40456),
            movie_anidb_id: None,
            name: "Mugen Train".into(),
            slug: "mugen-train".into(),
            year: Some(2020),
            content_status: "released".into(),
            overview: "Demon Slayer: Mugen Train".into(),
            poster_url: "poster".into(),
            language: "eng".into(),
            runtime_minutes: 117,
            sort_title: "Mugen Train".into(),
            imdb_id: "tt11032374".into(),
            genres: vec!["Action".into()],
            studio: "ufotable".into(),
            digital_release_date: Some("2020-10-16".into()),
            association_confidence: "high".into(),
            continuity_status: "canon".into(),
            movie_form: "movie".into(),
            placement: "ordered".into(),
            confidence: "high".into(),
            signal_summary: "TVDB marked special as critical to story".into(),
        }];

        app.create_series_seasons_and_episodes(
            &title,
            &seasons,
            &episodes,
            &anime_mappings,
            &anime_movies,
        )
        .await;

        let collections = app
            .list_collections(&user, &title.id)
            .await
            .expect("list collections");
        let interstitial = collections
            .iter()
            .find(|collection| collection.collection_type == CollectionType::Interstitial)
            .expect("interstitial collection should exist");
        assert_eq!(interstitial.collection_index, "1.1");
        assert_eq!(
            interstitial
                .interstitial_movie
                .as_ref()
                .map(|movie| movie.tvdb_id.as_str()),
            Some("131963")
        );
        assert_eq!(interstitial.label.as_deref(), Some("Mugen Train"));

        let interstitial_episodes = app
            .list_episodes(&user, &interstitial.id)
            .await
            .expect("list interstitial episodes");
        assert_eq!(interstitial_episodes.len(), 1);
        assert_eq!(
            interstitial_episodes[0].title.as_deref(),
            Some("Mugen Train")
        );
    }

    #[tokio::test]
    async fn anime_mapping_without_movie_link_does_not_create_interstitial_collection() {
        let (app, user) = bootstrap();
        let title = app
            .add_title(
                &user,
                NewTitle {
                    name: "Given".into(),
                    facet: MediaFacet::Anime,
                    monitored: true,
                    tags: vec![],
                    external_ids: vec![ExternalId {
                        source: "tvdb_id".into(),
                        value: "361218".into(),
                    }],
                    min_availability: None,
                    ..Default::default()
                },
            )
            .await
            .expect("create title");

        let seasons = vec![
            SeasonMetadata {
                tvdb_id: 20,
                number: 0,
                label: "Specials".into(),
                episode_type: "special".into(),
            },
            SeasonMetadata {
                tvdb_id: 21,
                number: 1,
                label: "Season 1".into(),
                episode_type: "official".into(),
            },
        ];
        let episodes = vec![
            EpisodeMetadata {
                tvdb_id: 3001,
                episode_number: 1,
                name: "Boys in the Band".into(),
                aired: "2019-07-12".into(),
                runtime_minutes: 23,
                is_filler: false,
                is_recap: false,
                overview: "Episode 1".into(),
                absolute_number: "1".into(),
                season_number: 1,
            },
            EpisodeMetadata {
                tvdb_id: 3002,
                episode_number: 1,
                name: "OVA".into(),
                aired: "2020-02-01".into(),
                runtime_minutes: 23,
                is_filler: false,
                is_recap: false,
                overview: "Special".into(),
                absolute_number: String::new(),
                season_number: 0,
            },
        ];
        let anime_mappings = vec![AnimeMapping {
            mal_id: Some(40421),
            anilist_id: None,
            anidb_id: None,
            kitsu_id: None,
            thetvdb_id: Some(361218),
            themoviedb_id: None,
            alt_tvdb_id: None,
            thetvdb_season: Some(0),
            score: None,
            anime_media_type: "TV".into(),
            global_media_type: "series".into(),
            status: "finished".into(),
            mapping_type: String::new(),
            episode_mappings: vec![AnimeEpisodeMapping {
                tvdb_season: 0,
                episode_start: 1,
                episode_end: 1,
            }],
        }];

        app.create_series_seasons_and_episodes(&title, &seasons, &episodes, &anime_mappings, &[])
            .await;

        let collections = app
            .list_collections(&user, &title.id)
            .await
            .expect("list collections");
        assert!(
            collections
                .iter()
                .all(|collection| collection.collection_type != CollectionType::Interstitial),
            "unexpected interstitial collection created"
        );
    }

    #[tokio::test]
    async fn anime_specials_movies_attach_to_specials_collection_and_keep_ordered_movies_separate()
    {
        let (app, user) = bootstrap();
        let title = app
            .add_title(
                &user,
                NewTitle {
                    name: "Attack on Titan".into(),
                    facet: MediaFacet::Anime,
                    monitored: true,
                    tags: vec!["scryer:monitor-specials:false".into()],
                    external_ids: vec![ExternalId {
                        source: "tvdb_id".into(),
                        value: "267440".into(),
                    }],
                    min_availability: None,
                    ..Default::default()
                },
            )
            .await
            .expect("create title");

        let seasons = vec![
            SeasonMetadata {
                tvdb_id: 50,
                number: 0,
                label: "Specials".into(),
                episode_type: "special".into(),
            },
            SeasonMetadata {
                tvdb_id: 51,
                number: 1,
                label: "Season 1".into(),
                episode_type: "official".into(),
            },
            SeasonMetadata {
                tvdb_id: 52,
                number: 2,
                label: "Season 2".into(),
                episode_type: "official".into(),
            },
        ];
        let episodes = vec![
            EpisodeMetadata {
                tvdb_id: 5001,
                episode_number: 1,
                name: "To You, in 2000 Years".into(),
                aired: "2013-04-07".into(),
                runtime_minutes: 24,
                is_filler: false,
                is_recap: false,
                overview: "Episode 1".into(),
                absolute_number: "1".into(),
                season_number: 1,
            },
            EpisodeMetadata {
                tvdb_id: 6001,
                episode_number: 1,
                name: "Beast Titan".into(),
                aired: "2017-04-01".into(),
                runtime_minutes: 24,
                is_filler: false,
                is_recap: false,
                overview: "Episode 1".into(),
                absolute_number: "26".into(),
                season_number: 2,
            },
        ];

        let anime_movies = vec![
            AnimeMovie {
                movie_tvdb_id: Some(379088),
                movie_tmdb_id: Some(379088),
                movie_imdb_id: Some("tt3865768".into()),
                movie_mal_id: Some(23775),
                movie_anidb_id: None,
                name: "Attack on Titan: Crimson Bow and Arrow".into(),
                slug: "crimson-bow-and-arrow".into(),
                year: Some(2014),
                content_status: "released".into(),
                overview: "Recap of episodes 1-13.".into(),
                poster_url: "poster-aot".into(),
                language: "eng".into(),
                runtime_minutes: 120,
                sort_title: "Crimson Bow and Arrow".into(),
                imdb_id: "tt3865768".into(),
                genres: vec!["Action".into()],
                studio: "WIT Studio".into(),
                digital_release_date: Some("2014-11-22".into()),
                association_confidence: "high".into(),
                continuity_status: "unknown".into(),
                movie_form: "recap".into(),
                placement: "specials".into(),
                confidence: "high".into(),
                signal_summary: "TVDB special category marks this as a recap".into(),
            },
            AnimeMovie {
                movie_tvdb_id: Some(131963),
                movie_tmdb_id: Some(438759),
                movie_imdb_id: Some("tt11032374".into()),
                movie_mal_id: Some(40456),
                movie_anidb_id: None,
                name: "Mugen Train".into(),
                slug: "mugen-train".into(),
                year: Some(2020),
                content_status: "released".into(),
                overview: "Canon bridge movie".into(),
                poster_url: "poster-ds".into(),
                language: "eng".into(),
                runtime_minutes: 117,
                sort_title: "Mugen Train".into(),
                imdb_id: "tt11032374".into(),
                genres: vec!["Action".into()],
                studio: "ufotable".into(),
                digital_release_date: Some("2020-10-16".into()),
                association_confidence: "high".into(),
                continuity_status: "canon".into(),
                movie_form: "movie".into(),
                placement: "ordered".into(),
                confidence: "high".into(),
                signal_summary: "TVDB marked special as critical to story".into(),
            },
        ];

        app.create_series_seasons_and_episodes(&title, &seasons, &episodes, &[], &anime_movies)
            .await;

        let collections = app
            .list_collections(&user, &title.id)
            .await
            .expect("list collections");
        let specials = collections
            .iter()
            .find(|collection| collection.collection_type == CollectionType::Specials)
            .expect("specials collection should exist");
        assert!(!specials.monitored);
        assert_eq!(specials.specials_movies.len(), 1);
        assert_eq!(
            specials.specials_movies[0].movie_form.as_deref(),
            Some("recap")
        );

        let interstitial = collections
            .iter()
            .find(|collection| collection.collection_type == CollectionType::Interstitial)
            .expect("ordered movie collection should exist");
        assert!(interstitial.monitored);
        assert_eq!(
            interstitial
                .interstitial_movie
                .as_ref()
                .and_then(|movie| movie.continuity_status.as_deref()),
            Some("canon")
        );
    }

    #[tokio::test]
    async fn read_collection_by_id_returns_item() {
        let (app, user) = bootstrap();
        let title = app
            .add_title(
                &user,
                NewTitle {
                    name: "Read Collection".into(),
                    facet: MediaFacet::Series,
                    monitored: true,
                    tags: vec![],
                    external_ids: vec![],
                    min_availability: None,

                    ..Default::default()
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
                    facet: MediaFacet::Series,
                    monitored: true,
                    tags: vec![],
                    external_ids: vec![],
                    min_availability: None,

                    ..Default::default()
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
                "standard".into(),
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
                    facet: MediaFacet::Series,
                    monitored: true,
                    tags: vec![],
                    external_ids: vec![],
                    min_availability: None,

                    ..Default::default()
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
                    facet: MediaFacet::Series,
                    monitored: true,
                    tags: vec![],
                    external_ids: vec![],
                    min_availability: None,

                    ..Default::default()
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
                "standard".into(),
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
                    facet: MediaFacet::Series,
                    monitored: true,
                    tags: vec![],
                    external_ids: vec![],
                    min_availability: None,

                    ..Default::default()
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

        assert_eq!(updated.collection_type, CollectionType::Arc);
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
                    facet: MediaFacet::Series,
                    monitored: true,
                    tags: vec![],
                    external_ids: vec![],
                    min_availability: None,

                    ..Default::default()
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
                "standard".into(),
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
                Some("Updated overview".into()),
            )
            .await
            .expect("update episode");

        assert_eq!(updated.episode_type, scryer_domain::EpisodeType::Special);
        assert_eq!(updated.episode_number, Some("E01".into()));
        assert_eq!(updated.title, Some("Pilot Updated".into()));
        assert_eq!(updated.air_date, Some("2026-01-01".into()));
        assert_eq!(updated.overview, Some("Updated overview".into()));
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
        assert!(
            app.validate_password("P@ssw0rd", &hashed)
                .expect("hash should be valid")
        );
        assert!(
            !app.validate_password("wrong", &hashed)
                .expect("hash should validate")
        );
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
        assert!(
            app.validate_password("legacy-pass", &v1_hash)
                .expect("v1 should validate")
        );
        assert!(
            !app.validate_password("wrong", &v1_hash)
                .expect("v1 should reject wrong password")
        );
    }

    // ── password edge cases ───────────────────────────────────────────────────

    #[test]
    fn hash_password_empty_returns_error() {
        let (app, _) = bootstrap();
        assert!(app.hash_password("").is_err());
        assert!(app.hash_password("   ").is_err());
    }

    #[test]
    fn validate_password_v1_malformed_no_salt_separator() {
        let (app, _) = bootstrap();
        // Only "v1" prefix, no $ after it
        let bad_hash = "v1nope";
        let result = app.validate_password("anything", bad_hash);
        assert!(
            result.is_err(),
            "malformed v1 hash (no $) should return Err"
        );
    }

    #[test]
    fn validate_password_v1_malformed_no_hash_component() {
        let (app, _) = bootstrap();
        // Has v1$salt but no third segment
        let bad_hash = "v1$somesalt";
        let result = app.validate_password("anything", bad_hash);
        assert!(
            result.is_err(),
            "malformed v1 hash (no hash segment) should return Err"
        );
    }

    #[test]
    fn validate_password_unknown_version_returns_error() {
        let (app, _) = bootstrap();
        let result = app.validate_password("pass", "v99$somehash");
        assert!(result.is_err(), "unknown hash version should return Err");
    }

    // ── JWT round-trip ────────────────────────────────────────────────────────

    /// Derive a per-user JWT signing key (mirrors `AppUseCase::derive_jwt_key`).
    fn test_derive_jwt_key(
        salt: &str,
        password_hash: &str,
        entitlements: &[Entitlement],
    ) -> Vec<u8> {
        use ring::hmac;
        let mut entitlement_claims = entitlements
            .iter()
            .map(AppUseCase::entitlement_claim_string)
            .map(str::to_string)
            .collect::<Vec<_>>();
        entitlement_claims.sort();
        entitlement_claims.dedup();
        let entitlement_fingerprint = sha256_hex(entitlement_claims.join("\n"));
        let signing_material = format!("{password_hash}\n{entitlement_fingerprint}");
        let hmac_key = hmac::Key::new(hmac::HMAC_SHA256, salt.as_bytes());
        hmac::sign(&hmac_key, signing_material.as_bytes())
            .as_ref()
            .to_vec()
    }

    const TEST_PASSWORD_HASH: &str = "v2$argon2id$v=19$m=19456,t=2,p=1$dGVzdHNhbHQ$dGVzdGhhc2g";

    #[tokio::test]
    async fn issue_and_authenticate_token_round_trips() {
        let (app, _) = bootstrap();
        let user = User {
            id: "user-jwt-1".to_string(),
            username: "jwt_user".to_string(),
            password_hash: Some(TEST_PASSWORD_HASH.to_string()),
            entitlements: vec![Entitlement::ViewCatalog],
        };
        app.services.users.create(user.clone()).await.unwrap();
        let token = app.issue_access_token(&user).expect("issue token");
        let decoded = app
            .authenticate_token(&token)
            .await
            .expect("authenticate token");
        assert_eq!(decoded.id, user.id);
        assert_eq!(decoded.username, user.username);
    }

    #[tokio::test]
    async fn entitlements_survive_token_round_trip() {
        let (app, _) = bootstrap();
        let user = User {
            id: "user-jwt-2".to_string(),
            username: "ent_user".to_string(),
            password_hash: Some(TEST_PASSWORD_HASH.to_string()),
            entitlements: vec![Entitlement::ViewCatalog, Entitlement::ManageTitle],
        };
        app.services.users.create(user.clone()).await.unwrap();
        let token = app.issue_access_token(&user).expect("issue token");
        let decoded = app
            .authenticate_token(&token)
            .await
            .expect("authenticate token");
        assert!(decoded.entitlements.contains(&Entitlement::ViewCatalog));
        assert!(decoded.entitlements.contains(&Entitlement::ManageTitle));
    }

    #[tokio::test]
    async fn expired_token_returns_unauthorized() {
        let (app, _) = bootstrap();
        let user = User {
            id: "user-jwt-3".to_string(),
            username: "exp_user".to_string(),
            password_hash: Some(TEST_PASSWORD_HASH.to_string()),
            entitlements: vec![],
        };
        app.services.users.create(user.clone()).await.unwrap();
        // Encode a token with an exp 100 seconds in the past
        let claims = JwtClaims {
            sub: user.id.clone(),
            exp: Utc::now().timestamp() - 100,
            iat: Utc::now().timestamp() - 200,
            iss: app.auth.issuer.clone(),
            username: user.username.clone(),
            entitlements: vec![],
        };
        let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
        let signing_key = test_derive_jwt_key(&app.auth.jwt_signing_salt, TEST_PASSWORD_HASH, &[]);
        let key = jsonwebtoken::EncodingKey::from_secret(&signing_key);
        let expired_token = jsonwebtoken::encode(&header, &claims, &key).expect("encode");
        let result = app.authenticate_token(&expired_token).await;
        assert!(result.is_err(), "expired token should be rejected");
    }

    #[tokio::test]
    async fn wrong_issuer_token_returns_unauthorized() {
        let (app, _) = bootstrap();
        let user = User {
            id: "user-jwt-4".to_string(),
            username: "iss_user".to_string(),
            password_hash: Some(TEST_PASSWORD_HASH.to_string()),
            entitlements: vec![Entitlement::ViewCatalog],
        };
        app.services.users.create(user.clone()).await.unwrap();
        let claims = JwtClaims {
            sub: user.id.clone(),
            exp: Utc::now().timestamp() + 3600,
            iat: Utc::now().timestamp(),
            iss: "wrong-issuer".to_string(),
            username: user.username.clone(),
            entitlements: vec!["view_catalog".to_string()],
        };
        let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
        let signing_key = test_derive_jwt_key(
            &app.auth.jwt_signing_salt,
            TEST_PASSWORD_HASH,
            &user.entitlements,
        );
        let key = jsonwebtoken::EncodingKey::from_secret(&signing_key);
        let bad_token = jsonwebtoken::encode(&header, &claims, &key).expect("encode");
        let result = app.authenticate_token(&bad_token).await;
        assert!(
            result.is_err(),
            "token with wrong issuer should be rejected"
        );
    }

    #[tokio::test]
    async fn authenticate_token_warms_cache_without_get_by_id_round_trip() {
        let users = Arc::new(MockUserRepo::default());
        let (app, _) = bootstrap_with_user_repo(users.clone());
        let user = User {
            id: "user-jwt-cache-1".to_string(),
            username: "cache_user".to_string(),
            password_hash: Some(TEST_PASSWORD_HASH.to_string()),
            entitlements: vec![Entitlement::ViewCatalog],
        };
        app.services.users.create(user.clone()).await.unwrap();

        let token = app.issue_access_token(&user).expect("issue token");
        app.authenticate_token(&token)
            .await
            .expect("authenticate token");
        app.authenticate_token(&token)
            .await
            .expect("authenticate token from warm cache");

        assert_eq!(users.get_by_id_call_count(), 0);
        assert_eq!(users.list_all_call_count(), 1);
    }

    #[tokio::test]
    async fn password_change_invalidates_existing_token_immediately() {
        let (app, admin) = bootstrap();
        let created = app
            .create_user(
                &admin,
                "pw_rotate".to_string(),
                "before-pass".to_string(),
                vec![Entitlement::ViewCatalog],
            )
            .await
            .expect("create user");
        let token = app.issue_access_token(&created).expect("issue token");

        app.set_user_password(&admin, &created.id, "after-pass".to_string(), None)
            .await
            .expect("rotate password");

        let result = app.authenticate_token(&token).await;
        assert!(
            result.is_err(),
            "old token should be rejected after password change"
        );
    }

    #[tokio::test]
    async fn entitlement_change_invalidates_existing_token_and_relogin_works() {
        let (app, admin) = bootstrap();
        let created = app
            .create_user(
                &admin,
                "ent_rotate".to_string(),
                "same-pass".to_string(),
                vec![Entitlement::ViewCatalog],
            )
            .await
            .expect("create user");
        let old_token = app.issue_access_token(&created).expect("issue token");

        let updated = app
            .set_user_entitlements(
                &admin,
                &created.id,
                vec![Entitlement::ViewCatalog, Entitlement::ManageTitle],
            )
            .await
            .expect("update entitlements");

        let old_result = app.authenticate_token(&old_token).await;
        assert!(
            old_result.is_err(),
            "old token should be rejected after entitlement change"
        );

        let relogged = app
            .authenticate_credentials("ent_rotate", "same-pass")
            .await
            .expect("re-login after entitlement change");
        let new_token = app
            .issue_access_token(&relogged)
            .expect("issue refreshed token");
        let decoded = app
            .authenticate_token(&new_token)
            .await
            .expect("authenticate refreshed token");

        assert_eq!(decoded.id, updated.id);
        assert!(decoded.entitlements.contains(&Entitlement::ManageTitle));
    }

    #[tokio::test]
    async fn deleting_user_invalidates_existing_token_immediately() {
        let (app, admin) = bootstrap();
        let created = app
            .create_user(
                &admin,
                "gone_user".to_string(),
                "password123".to_string(),
                vec![Entitlement::ViewCatalog],
            )
            .await
            .expect("create user");
        let token = app.issue_access_token(&created).expect("issue token");

        app.delete_user(&admin, &created.id)
            .await
            .expect("delete user");

        let result = app.authenticate_token(&token).await;
        assert!(result.is_err(), "deleted user token should be rejected");
    }

    #[test]
    fn jwt_key_derivation_is_stable_across_entitlement_order() {
        let (app, _) = bootstrap();
        let key_a = app.derive_jwt_key(
            TEST_PASSWORD_HASH,
            &[Entitlement::ManageTitle, Entitlement::ViewCatalog],
        );
        let key_b = app.derive_jwt_key(
            TEST_PASSWORD_HASH,
            &[Entitlement::ViewCatalog, Entitlement::ManageTitle],
        );

        assert_eq!(key_a, key_b);
    }

    #[tokio::test]
    async fn malformed_entitlement_claims_are_rejected() {
        let (app, _) = bootstrap();
        let user = User {
            id: "user-jwt-malformed".to_string(),
            username: "jwt_claims".to_string(),
            password_hash: Some(TEST_PASSWORD_HASH.to_string()),
            entitlements: vec![Entitlement::ViewCatalog],
        };
        app.services.users.create(user.clone()).await.unwrap();
        app.ensure_jwt_signing_keys_loaded()
            .await
            .expect("seed signing key cache");

        let claims = JwtClaims {
            sub: user.id.clone(),
            exp: Utc::now().timestamp() + 3600,
            iat: Utc::now().timestamp(),
            iss: app.auth.issuer.clone(),
            username: user.username.clone(),
            entitlements: vec!["definitely_not_real".to_string()],
        };
        let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
        let signing_key = test_derive_jwt_key(
            &app.auth.jwt_signing_salt,
            TEST_PASSWORD_HASH,
            &user.entitlements,
        );
        let key = jsonwebtoken::EncodingKey::from_secret(&signing_key);
        let token = jsonwebtoken::encode(&header, &claims, &key).expect("encode");

        let result = app.authenticate_token(&token).await;
        assert!(
            result.is_err(),
            "malformed entitlement claims should be rejected"
        );
    }
}

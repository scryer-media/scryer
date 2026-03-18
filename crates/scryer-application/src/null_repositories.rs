use std::path::Path;

use async_trait::async_trait;
use scryer_domain::ImportFileResult;
use scryer_domain::ImportRecord;

use scryer_domain::RuleSet;

use crate::InsertMediaFileInput;
use scryer_domain::PluginInstallation;

use scryer_domain::{BlocklistEntry, TitleHistoryEventType, TitleHistoryRecord};

use crate::{
    AppError, AppResult, BlocklistRepository, DownloadSubmission, DownloadSubmissionRepository,
    FileImporter, HousekeepingRepository, ImportRepository, IndexerQueryStats, IndexerStatsTracker,
    MediaFileRepository, NewBlocklistEntry, NewTitleHistoryEvent, NotificationChannelRepository,
    NotificationSubscriptionRepository, PendingRelease, PendingReleaseRepository,
    PluginInstallationRepository, PostProcessingScriptRepository, ReleaseDecision,
    RuleSetRepository, SettingsRepository, SystemInfoProvider, TitleHistoryFilter,
    TitleHistoryPage, TitleHistoryRepository, TitleImageBlob, TitleImageKind, TitleImageProcessor,
    TitleImageReplacement, TitleImageRepository, TitleImageSyncTask, TitleMediaFile,
    TitleMediaSizeSummary, WantedItem, WantedItemRepository,
};

#[derive(Default)]
pub struct NullImportRepository;

#[async_trait]
impl ImportRepository for NullImportRepository {
    async fn queue_import_request(
        &self,
        _source_system: String,
        _source_ref: String,
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
    async fn get_import_by_source_ref(&self, _: &str, _: &str) -> AppResult<Option<ImportRecord>> {
        Ok(None)
    }
    async fn update_import_status(&self, _: &str, _: &str, _: Option<String>) -> AppResult<()> {
        Ok(())
    }
    async fn recover_stale_processing_imports(&self, _stale_seconds: i64) -> AppResult<u64> {
        Ok(0)
    }
    async fn list_pending_imports(&self) -> AppResult<Vec<ImportRecord>> {
        Ok(vec![])
    }
    async fn is_already_imported(&self, _: &str, _: &str) -> AppResult<bool> {
        Ok(false)
    }
    async fn list_imports(&self, _limit: usize) -> AppResult<Vec<ImportRecord>> {
        Ok(vec![])
    }
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

    async fn link_file_to_episode(&self, _file_id: &str, _episode_id: &str) -> AppResult<()> {
        Err(AppError::Repository(
            "media file repository is not configured".to_string(),
        ))
    }

    async fn list_media_files_for_title(&self, _title_id: &str) -> AppResult<Vec<TitleMediaFile>> {
        Err(AppError::Repository(
            "media file repository is not configured".to_string(),
        ))
    }

    async fn list_title_media_size_summaries(
        &self,
        _title_ids: &[String],
    ) -> AppResult<Vec<TitleMediaSizeSummary>> {
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
}

#[derive(Default)]
pub struct NullFileImporter;

#[async_trait]
impl FileImporter for NullFileImporter {
    async fn import_file(&self, _source: &Path, _dest: &Path) -> AppResult<ImportFileResult> {
        Err(AppError::Repository(
            "file importer is not configured".to_string(),
        ))
    }
}

#[derive(Default)]
pub struct NullTitleImageRepository;

#[async_trait]
impl TitleImageRepository for NullTitleImageRepository {
    async fn list_titles_requiring_image_refresh(
        &self,
        _kind: TitleImageKind,
        _limit: usize,
    ) -> AppResult<Vec<TitleImageSyncTask>> {
        Ok(vec![])
    }

    async fn replace_title_image(
        &self,
        _title_id: &str,
        _replacement: TitleImageReplacement,
    ) -> AppResult<()> {
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
pub struct NullTitleImageProcessor;

#[async_trait]
impl TitleImageProcessor for NullTitleImageProcessor {
    async fn fetch_and_process_image(
        &self,
        _kind: TitleImageKind,
        _source_url: &str,
    ) -> AppResult<TitleImageReplacement> {
        Err(AppError::Repository(
            "title image processor is not configured".to_string(),
        ))
    }
}

#[derive(Default)]
pub struct NullWantedItemRepository;

#[async_trait]
impl WantedItemRepository for NullWantedItemRepository {
    async fn upsert_wanted_item(&self, _item: &WantedItem) -> AppResult<String> {
        Err(AppError::Repository(
            "wanted item repository is not configured".to_string(),
        ))
    }
    async fn list_due_wanted_items(
        &self,
        _now: &str,
        _batch_limit: i64,
    ) -> AppResult<Vec<WantedItem>> {
        Ok(vec![])
    }
    async fn update_wanted_item_status(
        &self,
        _id: &str,
        _status: &str,
        _next_search_at: Option<&str>,
        _last_search_at: Option<&str>,
        _search_count: i64,
        _current_score: Option<i32>,
        _grabbed_release: Option<&str>,
    ) -> AppResult<()> {
        Err(AppError::Repository(
            "wanted item repository is not configured".to_string(),
        ))
    }
    async fn get_wanted_item_for_title(
        &self,
        _title_id: &str,
        _episode_id: Option<&str>,
    ) -> AppResult<Option<WantedItem>> {
        Ok(None)
    }
    async fn delete_wanted_items_for_title(&self, _title_id: &str) -> AppResult<()> {
        Ok(())
    }
    async fn insert_release_decision(&self, _decision: &ReleaseDecision) -> AppResult<String> {
        Err(AppError::Repository(
            "wanted item repository is not configured".to_string(),
        ))
    }
    async fn get_wanted_item_by_id(&self, _id: &str) -> AppResult<Option<WantedItem>> {
        Ok(None)
    }
    async fn list_wanted_items(
        &self,
        _status: Option<&str>,
        _media_type: Option<&str>,
        _title_id: Option<&str>,
        _limit: i64,
        _offset: i64,
    ) -> AppResult<Vec<WantedItem>> {
        Ok(vec![])
    }
    async fn count_wanted_items(
        &self,
        _status: Option<&str>,
        _media_type: Option<&str>,
        _title_id: Option<&str>,
    ) -> AppResult<i64> {
        Ok(0)
    }
    async fn list_release_decisions_for_title(
        &self,
        _title_id: &str,
        _limit: i64,
    ) -> AppResult<Vec<ReleaseDecision>> {
        Ok(vec![])
    }
    async fn list_release_decisions_for_wanted_item(
        &self,
        _wanted_item_id: &str,
        _limit: i64,
    ) -> AppResult<Vec<ReleaseDecision>> {
        Ok(vec![])
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
    ) -> AppResult<Vec<(PluginInstallation, Option<Vec<u8>>)>> {
        Ok(vec![])
    }
    async fn seed_builtin(
        &self,
        _plugin_id: &str,
        _name: &str,
        _description: &str,
        _version: &str,
        _provider_type: &str,
    ) -> AppResult<()> {
        Ok(())
    }
    async fn store_registry_cache(&self, _json: &str) -> AppResult<()> {
        Ok(())
    }
    async fn get_registry_cache(&self) -> AppResult<Option<String>> {
        Ok(None)
    }
}

#[derive(Default)]
pub struct NullSystemInfoProvider;

#[async_trait]
impl SystemInfoProvider for NullSystemInfoProvider {
    async fn current_migration_version(&self) -> AppResult<Option<String>> {
        Ok(None)
    }
    async fn pending_migration_count(&self) -> AppResult<usize> {
        Ok(0)
    }
    async fn smg_cert_expires_at(&self) -> AppResult<Option<String>> {
        Ok(None)
    }
    async fn vacuum_into(&self, _dest_path: &str) -> AppResult<()> {
        Ok(())
    }
}

#[derive(Default)]
pub struct NullIndexerStatsTracker;

impl IndexerStatsTracker for NullIndexerStatsTracker {
    fn record_query(&self, _indexer_id: &str, _indexer_name: &str, _success: bool) {}
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
    async fn list_subscriptions_for_event(
        &self,
        _event_type: &str,
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
pub struct NullHousekeepingRepository;

#[async_trait]
impl HousekeepingRepository for NullHousekeepingRepository {
    async fn delete_release_decisions_older_than(&self, _days: i64) -> AppResult<u32> {
        Ok(0)
    }
    async fn delete_release_attempts_older_than(&self, _days: i64) -> AppResult<u32> {
        Ok(0)
    }
    async fn delete_dispatched_event_outboxes_older_than(&self, _days: i64) -> AppResult<u32> {
        Ok(0)
    }
    async fn delete_history_events_older_than(&self, _days: i64) -> AppResult<u32> {
        Ok(0)
    }
    async fn list_all_media_file_paths(&self) -> AppResult<Vec<(String, String)>> {
        Ok(vec![])
    }
    async fn delete_media_files_by_ids(&self, _ids: &[String]) -> AppResult<u32> {
        Ok(0)
    }
}

#[derive(Default)]
pub struct NullDownloadSubmissionRepository;

#[async_trait]
impl DownloadSubmissionRepository for NullDownloadSubmissionRepository {
    async fn record_submission(&self, _: DownloadSubmission) -> AppResult<()> {
        Ok(())
    }
    async fn find_by_client_item_id(
        &self,
        _: &str,
        _: &str,
    ) -> AppResult<Option<DownloadSubmission>> {
        Ok(None)
    }
    async fn list_for_title(&self, _: &str) -> AppResult<Vec<DownloadSubmission>> {
        Ok(vec![])
    }
    async fn delete_for_title(&self, _: &str) -> AppResult<()> {
        Ok(())
    }
}

pub struct NullPendingReleaseRepository;

#[async_trait]
impl PendingReleaseRepository for NullPendingReleaseRepository {
    async fn insert_pending_release(&self, _: &PendingRelease) -> AppResult<String> {
        Ok(String::new())
    }
    async fn list_expired_pending_releases(&self, _: &str) -> AppResult<Vec<PendingRelease>> {
        Ok(vec![])
    }
    async fn list_waiting_pending_releases(&self) -> AppResult<Vec<PendingRelease>> {
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
    async fn update_pending_release_status(
        &self,
        _: &str,
        _: &str,
        _: Option<&str>,
    ) -> AppResult<()> {
        Ok(())
    }
    async fn supersede_pending_releases_for_wanted_item(&self, _: &str, _: &str) -> AppResult<()> {
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
}

#[derive(Default)]
pub struct NullTitleHistoryRepository;

#[async_trait]
impl TitleHistoryRepository for NullTitleHistoryRepository {
    async fn record_event(&self, _: &NewTitleHistoryEvent) -> AppResult<String> {
        Ok(String::new())
    }
    async fn list_history(&self, _: &TitleHistoryFilter) -> AppResult<TitleHistoryPage> {
        Ok(TitleHistoryPage {
            records: vec![],
            total_count: 0,
        })
    }
    async fn list_for_title(
        &self,
        _: &str,
        _: Option<&[TitleHistoryEventType]>,
        _: usize,
        _: usize,
    ) -> AppResult<TitleHistoryPage> {
        Ok(TitleHistoryPage {
            records: vec![],
            total_count: 0,
        })
    }
    async fn list_for_episode(&self, _: &str, _: usize) -> AppResult<Vec<TitleHistoryRecord>> {
        Ok(vec![])
    }
    async fn find_by_download_id(&self, _: &str) -> AppResult<Vec<TitleHistoryRecord>> {
        Ok(vec![])
    }
    async fn delete_for_title(&self, _: &str) -> AppResult<()> {
        Ok(())
    }
}

#[derive(Default)]
pub struct NullBlocklistRepository;

#[async_trait]
impl BlocklistRepository for NullBlocklistRepository {
    async fn add(&self, _: &NewBlocklistEntry) -> AppResult<String> {
        Ok(String::new())
    }
    async fn list_for_title(&self, _: &str, _: usize) -> AppResult<Vec<BlocklistEntry>> {
        Ok(vec![])
    }
    async fn list_all(&self, _: usize, _: usize) -> AppResult<(Vec<BlocklistEntry>, i64)> {
        Ok((vec![], 0))
    }
    async fn remove(&self, _: &str) -> AppResult<()> {
        Ok(())
    }
    async fn is_blocklisted(&self, _: &str, _: &str) -> AppResult<bool> {
        Ok(false)
    }
    async fn delete_for_title(&self, _: &str) -> AppResult<()> {
        Ok(())
    }
}

// ── Additional null impls for test bootstrapping ─────────────────────────────

#[cfg(test)]
pub mod test_nulls {
    use crate::{
        AppError, AppResult, DownloadClient, DownloadClientAddRequest,
        DownloadClientConfigRepository, DownloadGrabResult, EventRepository, IndexerClient,
        IndexerRoutingPlan, IndexerSearchResponse, PrimaryCollectionSummary, QualityProfile,
        QualityProfileRepository, ReleaseAttemptRepository, ReleaseDownloadAttemptOutcome,
        ReleaseDownloadFailureSignature, SearchMode, ShowRepository, TitleMetadataUpdate,
        TitleReleaseBlocklistEntry, TitleRepository, UserRepository,
    };
    use async_trait::async_trait;
    use scryer_domain::{
        CalendarEpisode, Collection, DownloadClientConfig, Entitlement, Episode, HistoryEvent,
        MediaFacet, Title, User,
    };

    #[derive(Default)]
    pub struct NullTitleRepository;

    #[async_trait]
    impl TitleRepository for NullTitleRepository {
        async fn list(&self, _: Option<MediaFacet>, _: Option<String>) -> AppResult<Vec<Title>> {
            Ok(vec![])
        }
        async fn get_by_id(&self, _: &str) -> AppResult<Option<Title>> {
            Ok(None)
        }
        async fn create(&self, _: Title) -> AppResult<Title> {
            Err(AppError::Repository("not configured".into()))
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
        async fn delete(&self, _: &str) -> AppResult<()> {
            Ok(())
        }
        async fn set_folder_path(&self, _: &str, _: &str) -> AppResult<()> {
            Ok(())
        }
        async fn list_unhydrated(&self, _: usize, _: &str) -> AppResult<Vec<Title>> {
            Ok(vec![])
        }
        async fn clear_metadata_language_for_all(&self) -> AppResult<u64> {
            Ok(0)
        }
    }

    #[derive(Default)]
    pub struct NullShowRepository;

    #[async_trait]
    impl ShowRepository for NullShowRepository {
        async fn list_collections_for_title(&self, _: &str) -> AppResult<Vec<Collection>> {
            Ok(vec![])
        }
        async fn get_collection_by_id(&self, _: &str) -> AppResult<Option<Collection>> {
            Ok(None)
        }
        async fn create_collection(&self, _: Collection) -> AppResult<Collection> {
            Err(AppError::Repository("not configured".into()))
        }
        async fn update_collection(
            &self,
            _: &str,
            _: Option<String>,
            _: Option<String>,
            _: Option<String>,
            _: Option<String>,
            _: Option<String>,
            _: Option<String>,
            _: Option<bool>,
        ) -> AppResult<Collection> {
            Err(AppError::Repository("not configured".into()))
        }
        async fn update_interstitial_season_episode(
            &self,
            _: &str,
            _: Option<String>,
        ) -> AppResult<()> {
            Ok(())
        }
        async fn set_collection_episodes_monitored(&self, _: &str, _: bool) -> AppResult<()> {
            Ok(())
        }
        async fn delete_collection(&self, _: &str) -> AppResult<()> {
            Ok(())
        }
        async fn list_episodes_for_collection(&self, _: &str) -> AppResult<Vec<Episode>> {
            Ok(vec![])
        }
        async fn get_episode_by_id(&self, _: &str) -> AppResult<Option<Episode>> {
            Ok(None)
        }
        async fn create_episode(&self, _: Episode) -> AppResult<Episode> {
            Err(AppError::Repository("not configured".into()))
        }
        async fn update_episode(
            &self,
            _: &str,
            _: Option<String>,
            _: Option<String>,
            _: Option<String>,
            _: Option<String>,
            _: Option<String>,
            _: Option<String>,
            _: Option<i64>,
            _: Option<bool>,
            _: Option<bool>,
            _: Option<bool>,
            _: Option<String>,
        ) -> AppResult<Episode> {
            Err(AppError::Repository("not configured".into()))
        }
        async fn delete_episode(&self, _: &str) -> AppResult<()> {
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
        async fn update_entitlements(&self, _: &str, _: Vec<Entitlement>) -> AppResult<User> {
            Err(AppError::Repository("not configured".into()))
        }
        async fn update_password_hash(&self, _: &str, _: String) -> AppResult<User> {
            Err(AppError::Repository("not configured".into()))
        }
        async fn delete(&self, _: &str) -> AppResult<()> {
            Ok(())
        }
    }

    #[derive(Default)]
    pub struct NullEventRepository;

    #[async_trait]
    impl EventRepository for NullEventRepository {
        async fn list(&self, _: Option<String>, _: i64, _: i64) -> AppResult<Vec<HistoryEvent>> {
            Ok(vec![])
        }
        async fn append(&self, _: HistoryEvent) -> AppResult<()> {
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
            _: Option<String>,
            _: Option<String>,
            _: Option<String>,
            _: Option<String>,
            _: Option<Vec<String>>,
            _: Option<IndexerRoutingPlan>,
            _: SearchMode,
            _: Option<u32>,
            _: Option<u32>,
            _: Option<u32>,
        ) -> AppResult<IndexerSearchResponse> {
            Ok(IndexerSearchResponse {
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
        async fn update(
            &self,
            _: &str,
            _: Option<String>,
            _: Option<String>,
            _: Option<String>,
            _: Option<String>,
            _: Option<bool>,
        ) -> AppResult<DownloadClientConfig> {
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
        ) -> AppResult<Vec<TitleReleaseBlocklistEntry>> {
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
    }
}

pub struct NullSubtitleDownloadRepository;

#[async_trait]
impl crate::SubtitleDownloadRepository for NullSubtitleDownloadRepository {
    async fn list_for_title(
        &self,
        _title_id: &str,
    ) -> AppResult<Vec<scryer_domain::SubtitleDownload>> {
        Ok(Vec::new())
    }
    async fn list_for_media_file(
        &self,
        _media_file_id: &str,
    ) -> AppResult<Vec<scryer_domain::SubtitleDownload>> {
        Ok(Vec::new())
    }
    async fn insert(&self, _download: &scryer_domain::SubtitleDownload) -> AppResult<()> {
        Ok(())
    }
    async fn delete(&self, _id: &str) -> AppResult<Option<scryer_domain::SubtitleDownload>> {
        Ok(None)
    }
    async fn is_blacklisted(
        &self,
        _media_file_id: &str,
        _provider: &str,
        _provider_file_id: &str,
    ) -> AppResult<bool> {
        Ok(false)
    }
    async fn blacklist(
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

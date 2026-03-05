use std::path::Path;

use async_trait::async_trait;
use scryer_domain::ImportFileResult;
use scryer_domain::ImportRecord;

use scryer_domain::RuleSet;

use scryer_domain::PluginInstallation;

use crate::{
    AppError, AppResult, FileImporter, ImportRepository, IndexerQueryStats,
    IndexerStatsTracker, MediaFileRepository, PluginInstallationRepository, ReleaseDecision,
    RuleSetRepository, SystemInfoProvider, TitleMediaFile, WantedItem, WantedItemRepository,
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
        Err(AppError::Repository("import repository is not configured".to_string()))
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
    async fn insert_media_file(
        &self,
        _title_id: &str,
        _file_path: &str,
        _size_bytes: i64,
        _quality_label: Option<String>,
    ) -> AppResult<String> {
        Err(AppError::Repository("media file repository is not configured".to_string()))
    }

    async fn link_file_to_episode(
        &self,
        _file_id: &str,
        _episode_id: &str,
    ) -> AppResult<()> {
        Err(AppError::Repository("media file repository is not configured".to_string()))
    }

    async fn list_media_files_for_title(
        &self,
        _title_id: &str,
    ) -> AppResult<Vec<TitleMediaFile>> {
        Err(AppError::Repository("media file repository is not configured".to_string()))
    }
}

#[derive(Default)]
pub struct NullFileImporter;

#[async_trait]
impl FileImporter for NullFileImporter {
    async fn import_file(&self, _source: &Path, _dest: &Path) -> AppResult<ImportFileResult> {
        Err(AppError::Repository("file importer is not configured".to_string()))
    }
}

#[derive(Default)]
pub struct NullWantedItemRepository;

#[async_trait]
impl WantedItemRepository for NullWantedItemRepository {
    async fn upsert_wanted_item(&self, _item: &WantedItem) -> AppResult<String> {
        Err(AppError::Repository("wanted item repository is not configured".to_string()))
    }
    async fn list_due_wanted_items(&self, _now: &str, _batch_limit: i64) -> AppResult<Vec<WantedItem>> {
        Ok(vec![])
    }
    async fn update_wanted_item_status(
        &self, _id: &str, _status: &str, _next_search_at: Option<&str>,
        _last_search_at: Option<&str>, _search_count: i64,
        _current_score: Option<i32>, _grabbed_release: Option<&str>,
    ) -> AppResult<()> {
        Err(AppError::Repository("wanted item repository is not configured".to_string()))
    }
    async fn get_wanted_item_for_title(
        &self, _title_id: &str, _episode_id: Option<&str>,
    ) -> AppResult<Option<WantedItem>> {
        Ok(None)
    }
    async fn delete_wanted_items_for_title(&self, _title_id: &str) -> AppResult<()> {
        Ok(())
    }
    async fn insert_release_decision(&self, _decision: &ReleaseDecision) -> AppResult<String> {
        Err(AppError::Repository("wanted item repository is not configured".to_string()))
    }
    async fn get_wanted_item_by_id(&self, _id: &str) -> AppResult<Option<WantedItem>> {
        Ok(None)
    }
    async fn list_wanted_items(
        &self, _status: Option<&str>, _media_type: Option<&str>,
        _title_id: Option<&str>, _limit: i64, _offset: i64,
    ) -> AppResult<Vec<WantedItem>> {
        Ok(vec![])
    }
    async fn count_wanted_items(
        &self, _status: Option<&str>, _media_type: Option<&str>,
        _title_id: Option<&str>,
    ) -> AppResult<i64> {
        Ok(0)
    }
    async fn list_release_decisions_for_title(
        &self, _title_id: &str, _limit: i64,
    ) -> AppResult<Vec<ReleaseDecision>> {
        Ok(vec![])
    }
    async fn list_release_decisions_for_wanted_item(
        &self, _wanted_item_id: &str, _limit: i64,
    ) -> AppResult<Vec<ReleaseDecision>> {
        Ok(vec![])
    }
}

#[derive(Default)]
pub struct NullRuleSetRepository;

#[async_trait]
impl RuleSetRepository for NullRuleSetRepository {
    async fn list_rule_sets(&self) -> AppResult<Vec<RuleSet>> { Ok(vec![]) }
    async fn list_enabled_rule_sets(&self) -> AppResult<Vec<RuleSet>> { Ok(vec![]) }
    async fn get_rule_set(&self, _id: &str) -> AppResult<Option<RuleSet>> { Ok(None) }
    async fn create_rule_set(&self, _rule_set: &RuleSet) -> AppResult<()> {
        Err(AppError::Repository("rule set repository is not configured".to_string()))
    }
    async fn update_rule_set(&self, _rule_set: &RuleSet) -> AppResult<()> {
        Err(AppError::Repository("rule set repository is not configured".to_string()))
    }
    async fn delete_rule_set(&self, _id: &str) -> AppResult<()> {
        Err(AppError::Repository("rule set repository is not configured".to_string()))
    }
    async fn record_rule_set_history(
        &self, _rule_set_id: &str, _action: &str,
        _rego_source: Option<&str>, _actor_id: Option<&str>,
    ) -> AppResult<()> { Ok(()) }
}

#[derive(Default)]
pub struct NullPluginInstallationRepository;

#[async_trait]
impl PluginInstallationRepository for NullPluginInstallationRepository {
    async fn list_plugin_installations(&self) -> AppResult<Vec<PluginInstallation>> { Ok(vec![]) }
    async fn get_plugin_installation(&self, _plugin_id: &str) -> AppResult<Option<PluginInstallation>> { Ok(None) }
    async fn create_plugin_installation(&self, _installation: &PluginInstallation, _wasm_bytes: Option<&[u8]>) -> AppResult<PluginInstallation> {
        Err(AppError::Repository("plugin installation repository is not configured".to_string()))
    }
    async fn update_plugin_installation(&self, _installation: &PluginInstallation, _wasm_bytes: Option<&[u8]>) -> AppResult<PluginInstallation> {
        Err(AppError::Repository("plugin installation repository is not configured".to_string()))
    }
    async fn delete_plugin_installation(&self, _plugin_id: &str) -> AppResult<()> {
        Err(AppError::Repository("plugin installation repository is not configured".to_string()))
    }
    async fn get_enabled_plugin_wasm_bytes(&self) -> AppResult<Vec<(PluginInstallation, Option<Vec<u8>>)>> { Ok(vec![]) }
    async fn seed_builtin(&self, _plugin_id: &str, _name: &str, _description: &str, _version: &str, _provider_type: &str) -> AppResult<()> { Ok(()) }
    async fn store_registry_cache(&self, _json: &str) -> AppResult<()> { Ok(()) }
    async fn get_registry_cache(&self) -> AppResult<Option<String>> { Ok(None) }
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
}

#[derive(Default)]
pub struct NullIndexerStatsTracker;

impl IndexerStatsTracker for NullIndexerStatsTracker {
    fn record_query(&self, _indexer_id: &str, _indexer_name: &str, _success: bool) {}
    fn record_api_limits(
        &self, _indexer_id: &str, _api_current: Option<u32>, _api_max: Option<u32>,
        _grab_current: Option<u32>, _grab_max: Option<u32>,
    ) {}
    fn all_stats(&self) -> Vec<IndexerQueryStats> { vec![] }
}

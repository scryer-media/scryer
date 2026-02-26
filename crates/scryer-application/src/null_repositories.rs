use std::path::Path;

use async_trait::async_trait;
use scryer_domain::ImportFileResult;
use scryer_domain::ImportRecord;

use crate::{
    AppError, AppResult, FileImporter, ImportRepository, MediaFileRepository,
    ReleaseDecision, TitleMediaFile,
    WantedItem, WantedItemRepository,
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

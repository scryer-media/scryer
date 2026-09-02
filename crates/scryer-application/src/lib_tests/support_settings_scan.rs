use super::*;

#[derive(Default)]
pub(super) struct MockIndexerConfigRepo {
    pub(super) store: Arc<Mutex<Vec<IndexerConfig>>>,
}

#[derive(Default)]
pub(super) struct MockSettingsRepo;

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

    async fn delete_setting_value(
        &self,
        _scope: &str,
        _key_name: &str,
        _scope_id: Option<String>,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn delete_values_for_scope_id(&self, _scope_id: &str) -> AppResult<u32> {
        Ok(0)
    }
}

#[derive(Default, Clone)]
pub(super) struct StoredSettingsRepo {
    pub(super) values: StoredSettingValues,
}

pub(super) type StoredSettingValues = Arc<Mutex<HashMap<(String, String, Option<String>), String>>>;

impl StoredSettingsRepo {
    pub(super) async fn set_value(&self, scope: &str, key_name: &str, value: &str) {
        self.values.lock().await.insert(
            (scope.to_string(), key_name.to_string(), None),
            value.to_string(),
        );
    }

    pub(super) async fn get_value(&self, scope: &str, key_name: &str) -> Option<String> {
        self.values
            .lock()
            .await
            .get(&(scope.to_string(), key_name.to_string(), None))
            .cloned()
    }

    pub(super) async fn set_scoped_value(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: &str,
        value: &str,
    ) {
        self.values.lock().await.insert(
            (
                scope.to_string(),
                key_name.to_string(),
                Some(scope_id.to_string()),
            ),
            value.to_string(),
        );
    }

    pub(super) async fn get_scoped_value(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: &str,
    ) -> Option<String> {
        self.values
            .lock()
            .await
            .get(&(
                scope.to_string(),
                key_name.to_string(),
                Some(scope_id.to_string()),
            ))
            .cloned()
    }
}

#[async_trait]
impl SettingsRepository for StoredSettingsRepo {
    async fn get_setting_json(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
    ) -> AppResult<Option<String>> {
        Ok(self
            .values
            .lock()
            .await
            .get(&(scope.to_string(), key_name.to_string(), scope_id))
            .cloned())
    }

    async fn upsert_setting_json(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
        value_json: String,
        _source: &str,
        _updated_by_user_id: Option<String>,
    ) -> AppResult<()> {
        self.values.lock().await.insert(
            (scope.to_string(), key_name.to_string(), scope_id),
            value_json,
        );
        Ok(())
    }

    async fn delete_setting_value(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
    ) -> AppResult<()> {
        self.values
            .lock()
            .await
            .remove(&(scope.to_string(), key_name.to_string(), scope_id));
        Ok(())
    }

    async fn delete_values_for_scope_id(&self, scope_id: &str) -> AppResult<u32> {
        let mut values = self.values.lock().await;
        let before = values.len();
        values.retain(|(_, _, stored_scope_id), _| stored_scope_id.as_deref() != Some(scope_id));
        Ok((before - values.len()) as u32)
    }
}

#[derive(Default, Clone)]
pub(super) struct CoalescingSettingsRepo {
    pub(super) values: StoredSettingValues,
}

impl CoalescingSettingsRepo {
    pub(super) async fn set_value(&self, scope: &str, key_name: &str, value: &str) {
        self.values.lock().await.insert(
            (scope.to_string(), key_name.to_string(), None),
            value.to_string(),
        );
    }

    pub(super) async fn set_scoped_value(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: &str,
        value: &str,
    ) {
        self.values.lock().await.insert(
            (
                scope.to_string(),
                key_name.to_string(),
                Some(scope_id.to_string()),
            ),
            value.to_string(),
        );
    }

    pub(super) fn implicit_default(key_name: &str) -> Option<&'static str> {
        match key_name {
            QUALITY_PROFILE_ID_KEY => Some("\"4k\""),
            SCORING_PERSONA_KEY => Some("\"Balanced\""),
            DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY | INDEXER_ROUTING_SETTINGS_KEY => Some("{}"),
            _ => None,
        }
    }
}

#[async_trait]
impl SettingsRepository for CoalescingSettingsRepo {
    async fn get_setting_json(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
    ) -> AppResult<Option<String>> {
        if let Some(value) = self
            .values
            .lock()
            .await
            .get(&(scope.to_string(), key_name.to_string(), scope_id.clone()))
            .cloned()
        {
            return Ok(Some(value));
        }

        Ok(Self::implicit_default(key_name).map(str::to_string))
    }

    async fn get_setting_json_explicit(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
    ) -> AppResult<Option<String>> {
        Ok(self
            .values
            .lock()
            .await
            .get(&(scope.to_string(), key_name.to_string(), scope_id))
            .cloned())
    }

    async fn upsert_setting_json(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
        value_json: String,
        _source: &str,
        _updated_by_user_id: Option<String>,
    ) -> AppResult<()> {
        self.values.lock().await.insert(
            (scope.to_string(), key_name.to_string(), scope_id),
            value_json,
        );
        Ok(())
    }

    async fn delete_setting_value(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
    ) -> AppResult<()> {
        self.values
            .lock()
            .await
            .remove(&(scope.to_string(), key_name.to_string(), scope_id));
        Ok(())
    }

    async fn delete_values_for_scope_id(&self, scope_id: &str) -> AppResult<u32> {
        let mut values = self.values.lock().await;
        let before = values.len();
        values.retain(|(_, _, stored_scope_id), _| stored_scope_id.as_deref() != Some(scope_id));
        Ok((before - values.len()) as u32)
    }
}

#[derive(Default, Clone)]
pub(super) struct MutableLibraryScanner {
    pub(super) library_files: Arc<Mutex<Vec<LibraryFile>>>,
}

impl MutableLibraryScanner {
    pub(super) async fn set_library_files(&self, files: Vec<LibraryFile>) {
        *self.library_files.lock().await = files;
    }
}

#[async_trait]
impl LibraryScanner for MutableLibraryScanner {
    async fn scan_library(&self, _root: &str) -> AppResult<Vec<LibraryFile>> {
        Ok(self.library_files.lock().await.clone())
    }

    async fn scan_library_batched(
        &self,
        _root: &str,
        _batch_size: usize,
    ) -> AppResult<LibraryFileBatchReceiver> {
        let files = self.library_files.lock().await.clone();
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        tx.send(Ok(files))
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        Ok(rx)
    }

    async fn scan_directory_batched(
        &self,
        _root: &str,
        _batch_size: usize,
    ) -> AppResult<LibraryFileBatchReceiver> {
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        drop(tx);
        Ok(rx)
    }
}

#[derive(Default, Clone)]
pub(super) struct EmptySearchMetadataGateway;

#[async_trait]
impl MetadataGateway for EmptySearchMetadataGateway {
    async fn search_tvdb(
        &self,
        _query: &str,
        _type_hint: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<MetadataSearchItem>> {
        Ok(Vec::new())
    }

    async fn search_tvdb_batch(
        &self,
        queries: &[MetadataSearchQuery],
        _language: &str,
    ) -> AppResult<HashMap<MetadataSearchQuery, Vec<MetadataSearchItem>>> {
        Ok(queries
            .iter()
            .cloned()
            .map(|query| (query, Vec::new()))
            .collect())
    }

    async fn search_tvdb_rich(
        &self,
        _query: &str,
        _type_hint: &str,
        _limit: i32,
        _language: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<RichMetadataSearchItem>> {
        Ok(Vec::new())
    }

    async fn search_tvdb_multi(
        &self,
        _query: &str,
        _limit: i32,
        _language: &str,
    ) -> AppResult<MultiMetadataSearchResult> {
        Ok(MultiMetadataSearchResult::default())
    }

    async fn get_movie(&self, _tvdb_id: i64, _language: &str) -> AppResult<MovieMetadata> {
        Err(AppError::NotFound(
            "movie metadata unavailable in test".into(),
        ))
    }

    async fn get_series(&self, _tvdb_id: i64, _language: &str) -> AppResult<SeriesMetadata> {
        Err(AppError::NotFound(
            "series metadata unavailable in test".into(),
        ))
    }

    async fn get_metadata_bulk(
        &self,
        _movie_tvdb_ids: &[i64],
        _series_tvdb_ids: &[i64],
        _language: &str,
    ) -> AppResult<BulkMetadataResult> {
        Ok(BulkMetadataResult::default())
    }
}

pub(super) struct BlockingBatchMetadataGateway {
    pub(super) batch_search_calls: Arc<AtomicUsize>,
    pub(super) batch_search_started: Arc<Notify>,
    pub(super) blocked_calls: Arc<Vec<usize>>,
    pub(super) released_through: Arc<AtomicUsize>,
    pub(super) release_notify: Arc<Notify>,
}

impl BlockingBatchMetadataGateway {
    pub(super) fn blocking_calls(blocked_calls: &[usize]) -> Self {
        Self {
            batch_search_calls: Arc::new(AtomicUsize::new(0)),
            batch_search_started: Arc::new(Notify::new()),
            blocked_calls: Arc::new(blocked_calls.to_vec()),
            released_through: Arc::new(AtomicUsize::new(0)),
            release_notify: Arc::new(Notify::new()),
        }
    }

    pub(super) async fn wait_for_batch_search(&self) {
        self.wait_for_batch_search_calls(1).await;
    }

    pub(super) async fn wait_for_batch_search_calls(&self, expected_calls: usize) {
        if self.batch_search_calls.load(Ordering::SeqCst) >= expected_calls {
            return;
        }

        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if self.batch_search_calls.load(Ordering::SeqCst) >= expected_calls {
                    break;
                }
                self.batch_search_started.notified().await;
            }
        })
        .await
        .expect("timed out waiting for metadata search to start");
    }

    pub(super) fn release_through(&self, call_number: usize) {
        self.released_through.store(call_number, Ordering::SeqCst);
        self.release_notify.notify_waiters();
    }

    pub(super) fn release(&self) {
        self.release_through(usize::MAX);
    }
}

impl Default for BlockingBatchMetadataGateway {
    fn default() -> Self {
        Self::blocking_calls(&[1])
    }
}

#[async_trait]
impl MetadataGateway for BlockingBatchMetadataGateway {
    async fn search_tvdb(
        &self,
        _query: &str,
        _type_hint: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<MetadataSearchItem>> {
        Ok(Vec::new())
    }

    async fn search_tvdb_batch(
        &self,
        queries: &[MetadataSearchQuery],
        _language: &str,
    ) -> AppResult<HashMap<MetadataSearchQuery, Vec<MetadataSearchItem>>> {
        let call_number = self.batch_search_calls.fetch_add(1, Ordering::SeqCst) + 1;
        self.batch_search_started.notify_waiters();

        if self.blocked_calls.contains(&call_number) {
            while self.released_through.load(Ordering::SeqCst) < call_number {
                self.release_notify.notified().await;
            }
        }

        Ok(queries
            .iter()
            .cloned()
            .map(|query| (query, Vec::new()))
            .collect())
    }

    async fn search_tvdb_rich(
        &self,
        _query: &str,
        _type_hint: &str,
        _limit: i32,
        _language: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<RichMetadataSearchItem>> {
        Ok(Vec::new())
    }

    async fn search_tvdb_multi(
        &self,
        _query: &str,
        _limit: i32,
        _language: &str,
    ) -> AppResult<MultiMetadataSearchResult> {
        Ok(MultiMetadataSearchResult::default())
    }

    async fn get_movie(&self, _tvdb_id: i64, _language: &str) -> AppResult<MovieMetadata> {
        Err(AppError::NotFound(
            "movie metadata unavailable in test".into(),
        ))
    }

    async fn get_series(&self, _tvdb_id: i64, _language: &str) -> AppResult<SeriesMetadata> {
        Err(AppError::NotFound(
            "series metadata unavailable in test".into(),
        ))
    }

    async fn get_metadata_bulk(
        &self,
        _movie_tvdb_ids: &[i64],
        _series_tvdb_ids: &[i64],
        _language: &str,
    ) -> AppResult<BulkMetadataResult> {
        Ok(BulkMetadataResult::default())
    }
}

#[derive(Default, Clone)]
pub(super) struct TrackingLibraryScanUnmatchedItemRepo {
    pub(super) items: Arc<Mutex<Vec<LibraryScanUnmatchedItem>>>,
}

impl TrackingLibraryScanUnmatchedItemRepo {
    pub(super) async fn items(&self) -> Vec<LibraryScanUnmatchedItem> {
        self.items.lock().await.clone()
    }
}

#[async_trait]
impl LibraryScanUnmatchedItemRepository for TrackingLibraryScanUnmatchedItemRepo {
    async fn upsert_library_scan_unmatched_item(
        &self,
        item: &LibraryScanUnmatchedItem,
    ) -> AppResult<String> {
        let mut items = self.items.lock().await;
        if let Some(existing) = items.iter_mut().find(|existing| existing.id == item.id) {
            let mut updated = item.clone();
            updated.created_at = existing.created_at.clone();
            if existing.status == PendingImportStatus::Ignored
                && updated.status == PendingImportStatus::Pending
            {
                updated.status = PendingImportStatus::Ignored;
            }
            *existing = updated;
        } else {
            items.push(item.clone());
        }

        Ok(item.id.clone())
    }

    async fn get_library_scan_unmatched_item(
        &self,
        id: &str,
    ) -> AppResult<Option<LibraryScanUnmatchedItem>> {
        Ok(self
            .items
            .lock()
            .await
            .iter()
            .find(|item| item.id == id)
            .cloned())
    }

    async fn delete_library_scan_unmatched_item(
        &self,
        library_id: &str,
        facet: MediaFacet,
        item_path: &str,
    ) -> AppResult<()> {
        self.items.lock().await.retain(|item| {
            !(item.library_id == library_id && item.facet == facet && item.item_path == item_path)
        });
        Ok(())
    }

    async fn delete_for_library(&self, library_id: &str) -> AppResult<u32> {
        let mut items = self.items.lock().await;
        let before = items.len();
        items.retain(|item| item.library_id != library_id);
        Ok((before - items.len()) as u32)
    }

    async fn list_library_scan_unmatched_items(
        &self,
        facet: Option<MediaFacet>,
        scan_root: Option<&str>,
        status: Option<PendingImportStatus>,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<LibraryScanUnmatchedItem>> {
        let offset = offset.max(0) as usize;
        let limit = limit.max(0) as usize;
        let mut items: Vec<_> = self
            .items
            .lock()
            .await
            .iter()
            .filter(|item| {
                facet
                    .as_ref()
                    .is_none_or(|expected| &item.facet == expected)
            })
            .filter(|item| {
                scan_root
                    .as_ref()
                    .is_none_or(|expected| item.scan_root == *expected)
            })
            .filter(|item| status.is_none_or(|expected| item.status == expected))
            .cloned()
            .collect();
        items.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));

        Ok(items.into_iter().skip(offset).take(limit).collect())
    }

    async fn count_library_scan_unmatched_items(
        &self,
        facet: Option<MediaFacet>,
        scan_root: Option<&str>,
        status: Option<PendingImportStatus>,
    ) -> AppResult<i64> {
        Ok(self
            .items
            .lock()
            .await
            .iter()
            .filter(|item| {
                facet
                    .as_ref()
                    .is_none_or(|expected| &item.facet == expected)
            })
            .filter(|item| {
                scan_root
                    .as_ref()
                    .is_none_or(|expected| item.scan_root == *expected)
            })
            .filter(|item| status.is_none_or(|expected| item.status == expected))
            .count() as i64)
    }
}

#[derive(Default)]
pub(super) struct MockQualityProfileRepo;

#[async_trait]
impl QualityProfileRepository for MockQualityProfileRepo {
    async fn list_quality_profiles(
        &self,
        _scope: &str,
        _scope_id: Option<String>,
    ) -> AppResult<Vec<QualityProfile>> {
        Ok(vec![])
    }

    async fn replace_quality_profiles(
        &self,
        _scope: &str,
        _scope_id: Option<String>,
        _profiles: Vec<QualityProfile>,
    ) -> AppResult<()> {
        Ok(())
    }
}

#[derive(Default, Clone)]
pub(super) struct StoredQualityProfileRepo {
    pub(super) profiles: Arc<Mutex<Vec<QualityProfile>>>,
}

impl StoredQualityProfileRepo {
    pub(super) async fn set_profiles(&self, profiles: Vec<QualityProfile>) {
        *self.profiles.lock().await = profiles;
    }
}

#[async_trait]
impl QualityProfileRepository for StoredQualityProfileRepo {
    async fn list_quality_profiles(
        &self,
        _scope: &str,
        _scope_id: Option<String>,
    ) -> AppResult<Vec<QualityProfile>> {
        Ok(self.profiles.lock().await.clone())
    }

    async fn replace_quality_profiles(
        &self,
        _scope: &str,
        _scope_id: Option<String>,
        profiles: Vec<QualityProfile>,
    ) -> AppResult<()> {
        *self.profiles.lock().await = profiles;
        Ok(())
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

    async fn set_system_backoff(&self, id: &str, backoff: IndexerSystemBackoff) -> AppResult<()> {
        let mut entries = self.store.lock().await;
        for entry in entries.iter_mut().filter(|entry| entry.id == id) {
            entry.disabled_until = Some(backoff.disabled_until);
        }
        Ok(())
    }

    async fn clear_system_backoff(&self, id: &str) -> AppResult<()> {
        let mut entries = self.store.lock().await;
        for entry in entries.iter_mut().filter(|entry| entry.id == id) {
            entry.disabled_until = None;
        }
        Ok(())
    }

    async fn touch_last_error(&self, id: &str) -> AppResult<()> {
        let mut entries = self.store.lock().await;
        let now = Utc::now();
        for entry in entries.iter_mut() {
            if entry.id == id {
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

    async fn update(&self, update: crate::IndexerConfigUpdate) -> AppResult<IndexerConfig> {
        let crate::IndexerConfigUpdate {
            id,
            name,
            provider_type,
            derived_base_url,
            rate_limit_seconds,
            rate_limit_burst,
            is_enabled,
            enable_interactive_search,
            enable_auto_search,
            proxy_config_id,
            download_client_id,
            seeding_profile_id,
            managed_parent_config_id,
            managed_child_key,
            managed_metadata_json,
            caps_snapshot_json,
            config_json,
        } = update;
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
        if let Some(base_url) = derived_base_url {
            item.base_url = base_url;
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
        if let Some(proxy_config_id) = proxy_config_id {
            item.proxy_config_id = proxy_config_id;
        }
        if let Some(download_client_id) = download_client_id {
            item.download_client_id = download_client_id;
        }
        if let Some(seeding_profile_id) = seeding_profile_id {
            item.seeding_profile_id = seeding_profile_id;
        }
        if let Some(managed_parent_config_id) = managed_parent_config_id {
            item.managed_parent_config_id = managed_parent_config_id;
        }
        if let Some(managed_child_key) = managed_child_key {
            item.managed_child_key = managed_child_key;
        }
        if let Some(managed_metadata_json) = managed_metadata_json {
            item.managed_metadata_json = managed_metadata_json;
        }
        if let Some(caps_snapshot_json) = caps_snapshot_json {
            item.caps_snapshot_json = caps_snapshot_json;
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
pub(super) struct MockSeedingProfileRepo {
    pub(super) store: Arc<Mutex<Vec<scryer_domain::SeedingProfile>>>,
}

#[async_trait]
impl crate::SeedingProfileRepository for MockSeedingProfileRepo {
    async fn list(&self) -> AppResult<Vec<scryer_domain::SeedingProfile>> {
        Ok(self.store.lock().await.clone())
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<scryer_domain::SeedingProfile>> {
        Ok(self
            .store
            .lock()
            .await
            .iter()
            .find(|profile| profile.id == id)
            .cloned())
    }

    async fn create(
        &self,
        profile: scryer_domain::SeedingProfile,
    ) -> AppResult<scryer_domain::SeedingProfile> {
        let mut entries = self.store.lock().await;
        if entries
            .iter()
            .any(|entry| entry.name.eq_ignore_ascii_case(&profile.name))
        {
            return Err(AppError::Validation(format!(
                "seeding profile name '{}' is already in use",
                profile.name
            )));
        }
        entries.push(profile.clone());
        Ok(profile)
    }

    async fn update(
        &self,
        profile: scryer_domain::SeedingProfile,
    ) -> AppResult<scryer_domain::SeedingProfile> {
        let mut entries = self.store.lock().await;
        let item = entries
            .iter_mut()
            .find(|entry| entry.id == profile.id)
            .ok_or_else(|| AppError::NotFound(format!("seeding profile {}", profile.id)))?;
        *item = profile.clone();
        Ok(profile)
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        let mut entries = self.store.lock().await;
        let before = entries.len();
        entries.retain(|entry| entry.id != id);
        if entries.len() == before {
            return Err(AppError::NotFound(format!("seeding profile {id}")));
        }
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct MockDownloadClientConfigRepo {
    pub(super) store: Arc<Mutex<Vec<DownloadClientConfig>>>,
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
        update: crate::DownloadClientConfigUpdate,
    ) -> AppResult<DownloadClientConfig> {
        let crate::DownloadClientConfigUpdate {
            id,
            name,
            client_type,
            config_json,
            is_enabled,
            proxy_config_id,
        } = update;
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
        if let Some(proxy_config_id) = proxy_config_id {
            item.proxy_config_id = proxy_config_id;
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

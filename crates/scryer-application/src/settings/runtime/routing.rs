#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadClientRoutingSettingsEntry {
    pub client_id: String,
    pub enabled: bool,
    pub category: Option<String>,
    pub recent_queue_priority: Option<String>,
    pub older_queue_priority: Option<String>,
    pub remove_completed: bool,
    pub remove_failed: bool,
    pub seeding_profile_id: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexerRoutingSettingsEntry {
    pub indexer_id: String,
    pub enabled: bool,
    pub categories: Vec<String>,
    pub priority: i32,
}
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LibrarySettingsOverrideDraft {
    pub required_audio_languages: Option<Vec<String>>,
    pub metadata_language: Option<String>,
    pub use_season_folders: Option<bool>,
    pub quality_profile_id: Option<String>,
    pub request_quality_profile_ids: Option<Vec<String>>,
    pub scoring_persona: Option<ScoringPersona>,
    pub filler_policy: Option<String>,
    pub recap_policy: Option<String>,
    pub monitor_specials: Option<bool>,
    pub inter_season_movies: Option<bool>,
    pub monitor_filler_movies: Option<bool>,
    pub nfo_write_on_import: Option<bool>,
    pub plexmatch_write_on_import: Option<bool>,
    pub import_mode: Option<ImportMode>,
    pub set_permissions_linux: Option<bool>,
    pub file_chmod: Option<String>,
    pub folder_chmod: Option<String>,
    pub chown_group: Option<String>,
    pub indexer_routing: Option<Vec<IndexerRoutingSettingsEntry>>,
    pub download_client_routing: Option<Vec<DownloadClientRoutingSettingsEntry>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExternalImportLibrarySettingsAutoApplyDraft {
    pub quality_profile_id: Option<String>,
    pub request_quality_profile_ids: Option<Vec<String>>,
    pub monitor_specials: Option<bool>,
    pub nfo_write_on_import: Option<bool>,
    pub plexmatch_write_on_import: Option<bool>,
    pub set_permissions_linux: Option<bool>,
    pub folder_chmod: Option<String>,
    pub chown_group: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExternalImportLibrarySettingsAutoApplyResult {
    pub changed_keys: Vec<String>,
    pub skipped_keys: Vec<ExternalImportSettingsAutoApplySkip>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalImportSettingsAutoApplySkip {
    pub key_name: String,
    pub reason: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibrarySettings {
    pub required_audio_languages_override: Option<Vec<String>>,
    pub required_audio_languages: Vec<String>,
    pub metadata_language_override: Option<String>,
    pub metadata_language: String,
    pub use_season_folders_override: Option<bool>,
    pub use_season_folders: bool,
    pub quality_profile_id_override: Option<String>,
    pub quality_profile_id: String,
    pub request_quality_profile_ids_override: Option<Vec<String>>,
    pub request_quality_profile_ids: Vec<String>,
    pub request_quality_profile_default_id: String,
    pub scoring_persona_override: Option<ScoringPersona>,
    pub scoring_persona: ScoringPersona,
    pub filler_policy_override: Option<String>,
    pub filler_policy: Option<String>,
    pub recap_policy_override: Option<String>,
    pub recap_policy: Option<String>,
    pub monitor_specials_override: Option<bool>,
    pub monitor_specials: Option<bool>,
    pub inter_season_movies_override: Option<bool>,
    pub inter_season_movies: Option<bool>,
    pub monitor_filler_movies_override: Option<bool>,
    pub monitor_filler_movies: Option<bool>,
    pub nfo_write_on_import_override: Option<bool>,
    pub nfo_write_on_import: bool,
    pub plexmatch_write_on_import_override: Option<bool>,
    pub plexmatch_write_on_import: Option<bool>,
    pub import_mode_override: Option<ImportMode>,
    pub import_mode: ImportMode,
    pub set_permissions_linux_override: Option<bool>,
    pub set_permissions_linux: bool,
    pub file_chmod_override: Option<String>,
    pub file_chmod: Option<String>,
    pub folder_chmod_override: Option<String>,
    pub folder_chmod: Option<String>,
    pub chown_group_override: Option<String>,
    pub chown_group: Option<String>,
    pub indexer_routing_override: Option<Vec<IndexerRoutingSettingsEntry>>,
    pub download_client_routing_override: Option<Vec<DownloadClientRoutingSettingsEntry>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestQualityProfileSettings {
    pub profile_ids: Vec<String>,
    pub default_profile_id: String,
}

fn download_client_routing_payload(
    entries: Vec<DownloadClientRoutingSettingsEntry>,
) -> AppResult<serde_json::Map<String, serde_json::Value>> {
    let mut payload = serde_json::Map::new();
    for entry in entries {
        let client_id = entry.client_id.trim();
        if client_id.is_empty() {
            return Err(AppError::Validation(
                "download client routing entry requires client_id".to_string(),
            ));
        }

        payload.insert(
            client_id.to_string(),
            serde_json::json!({
                "enabled": entry.enabled,
                "category": normalize_optional_string(entry.category),
                "recentQueuePriority": normalize_optional_string(entry.recent_queue_priority),
                "olderQueuePriority": normalize_optional_string(entry.older_queue_priority),
                "removeCompleted": entry.remove_completed,
                "removeFailed": entry.remove_failed,
                "seedingProfileId": normalize_optional_string(entry.seeding_profile_id),
            }),
        );
    }
    Ok(payload)
}
fn download_client_routing_settings_entry_from_domain(
    client_id: String,
    entry: crate::catalog_helpers::DownloadClientRoutingEntry,
) -> DownloadClientRoutingSettingsEntry {
    DownloadClientRoutingSettingsEntry {
        client_id,
        enabled: entry.enabled,
        category: entry.category,
        recent_queue_priority: entry.recent_queue_priority,
        older_queue_priority: entry.older_queue_priority,
        remove_completed: entry.remove_completed,
        remove_failed: entry.remove_failed,
        seeding_profile_id: entry.seeding_profile_id,
    }
}
fn disabled_download_client_routing_settings_entry(
    client_id: String,
) -> DownloadClientRoutingSettingsEntry {
    let mut entry = crate::catalog_helpers::default_download_client_routing_entry();
    entry.enabled = false;
    download_client_routing_settings_entry_from_domain(client_id, entry)
}
fn normalize_download_client_routing_settings_entry(
    entry: DownloadClientRoutingSettingsEntry,
) -> AppResult<DownloadClientRoutingSettingsEntry> {
    let client_id = entry.client_id.trim().to_string();
    if client_id.is_empty() {
        return Err(AppError::Validation(
            "download client routing entry requires client_id".to_string(),
        ));
    }

    Ok(DownloadClientRoutingSettingsEntry {
        client_id,
        enabled: entry.enabled,
        category: normalize_optional_string(entry.category),
        recent_queue_priority: normalize_optional_string(entry.recent_queue_priority),
        older_queue_priority: normalize_optional_string(entry.older_queue_priority),
        remove_completed: entry.remove_completed,
        remove_failed: entry.remove_failed,
        seeding_profile_id: normalize_optional_string(entry.seeding_profile_id),
    })
}
fn indexer_routing_payload(
    entries: Vec<IndexerRoutingSettingsEntry>,
) -> AppResult<serde_json::Map<String, serde_json::Value>> {
    let mut payload = serde_json::Map::new();
    for entry in entries {
        let indexer_id = entry.indexer_id.trim();
        if indexer_id.is_empty() {
            return Err(AppError::Validation(
                "indexer routing entry requires indexer_id".to_string(),
            ));
        }

        let categories = entry
            .categories
            .into_iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();

        payload.insert(
            indexer_id.to_string(),
            serde_json::json!({
                "enabled": entry.enabled,
                "categories": categories,
                "priority": entry.priority,
            }),
        );
    }
    Ok(payload)
}
fn parse_routing_priority(raw_priority: &serde_json::Value) -> Option<i64> {
    match raw_priority {
        serde_json::Value::Number(number) => number.as_i64(),
        serde_json::Value::String(value) => value.parse::<i64>().ok(),
        _ => None,
    }
}
fn next_routing_priority(routing_by_id: &serde_json::Map<String, serde_json::Value>) -> i64 {
    let max_explicit_priority = routing_by_id
        .values()
        .filter_map(|value| value.get("priority"))
        .filter_map(parse_routing_priority)
        .max();

    match max_explicit_priority {
        Some(max_priority) => max_priority + 1,
        None => routing_by_id.len() as i64 + 1,
    }
}
fn default_download_client_routing_entry_json(priority: i64) -> serde_json::Value {
    serde_json::json!({
        "enabled": true,
        "category": "",
        "recentQueuePriority": "",
        "olderQueuePriority": "",
        "removeCompleted": true,
        "removeFailed": true,
        "priority": priority,
    })
}
fn default_indexer_routing_entry_json(scope_id: &str, priority: i64) -> serde_json::Value {
    serde_json::json!({
        "enabled": true,
        "categories": default_indexer_routing_categories_for_scope(scope_id),
        "priority": priority,
    })
}
/// Fill in any fields missing from a stored download-client routing entry with
/// canonical defaults. Returns `true` if the entry was modified. This is the
/// single source of truth for what a "complete" entry looks like at rest, used
/// by both the per-client ensure path and the startup normalization migration.
fn normalize_download_client_routing_entry_in_place(
    entry: &mut serde_json::Map<String, serde_json::Value>,
    fallback_priority: i64,
) -> bool {
    let mut changed = false;
    if !entry.contains_key("enabled") {
        entry.insert("enabled".to_string(), serde_json::Value::Bool(true));
        changed = true;
    }
    if !entry.contains_key("category") {
        entry.insert(
            "category".to_string(),
            serde_json::Value::String(String::new()),
        );
        changed = true;
    }
    if !entry.contains_key("recentQueuePriority") {
        entry.insert(
            "recentQueuePriority".to_string(),
            serde_json::Value::String(String::new()),
        );
        changed = true;
    }
    if !entry.contains_key("olderQueuePriority") {
        entry.insert(
            "olderQueuePriority".to_string(),
            serde_json::Value::String(String::new()),
        );
        changed = true;
    }
    if !entry.contains_key("removeCompleted") {
        entry.insert("removeCompleted".to_string(), serde_json::Value::Bool(true));
        changed = true;
    }
    if !entry.contains_key("removeFailed") {
        entry.insert("removeFailed".to_string(), serde_json::Value::Bool(true));
        changed = true;
    }
    if !entry.contains_key("priority") {
        entry.insert(
            "priority".to_string(),
            serde_json::Value::Number(fallback_priority.into()),
        );
        changed = true;
    }
    changed
}
/// Fill in any fields missing from a stored indexer routing entry with
/// canonical defaults. Returns `true` if the entry was modified.
fn normalize_indexer_routing_entry_in_place(
    scope_id: &str,
    entry: &mut serde_json::Map<String, serde_json::Value>,
    fallback_priority: i64,
) -> bool {
    let mut changed = false;
    if !entry.contains_key("enabled") {
        entry.insert("enabled".to_string(), serde_json::Value::Bool(true));
        changed = true;
    }
    if !entry.contains_key("categories") {
        entry.insert(
            "categories".to_string(),
            serde_json::json!(default_indexer_routing_categories_for_scope(scope_id)),
        );
        changed = true;
    }
    if !entry.contains_key("priority") {
        entry.insert(
            "priority".to_string(),
            serde_json::Value::Number(fallback_priority.into()),
        );
        changed = true;
    }
    changed
}
impl AppUseCase {
    pub(crate) async fn load_download_client_routing_json(
        &self,
        scope_id: &str,
    ) -> AppResult<Option<String>> {
        if let Some(raw_json) = self
            .read_setting_string_value(DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY, Some(scope_id))
            .await?
        {
            return Ok(Some(raw_json));
        }

        self.read_setting_string_value(LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY, Some(scope_id))
            .await
    }
}
impl AppUseCase {
    async fn load_explicit_download_client_routing_json(
        &self,
        scope_id: &str,
    ) -> AppResult<Option<String>> {
        if let Some(raw_json) = self
            .read_setting_string_value_explicit(
                DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
                Some(scope_id),
            )
            .await?
        {
            return Ok(Some(raw_json));
        }

        self.read_setting_string_value_explicit(
            LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY,
            Some(scope_id),
        )
        .await
    }
}
impl AppUseCase {
    async fn load_download_client_routing_override(
        &self,
        library_id: &str,
    ) -> AppResult<Option<Vec<DownloadClientRoutingSettingsEntry>>> {
        let Some(raw_json) = self
            .load_explicit_download_client_routing_json(library_id)
            .await?
        else {
            return Ok(None);
        };
        let Some(entries) = crate::catalog_helpers::parse_download_client_routing_map(&raw_json)
        else {
            warn!(
                library_id,
                "ignoring invalid library-scoped download client routing override in settings"
            );
            return Ok(None);
        };
        let entries = entries
            .into_iter()
            .map(|(client_id, config)| {
                let entry = crate::catalog_helpers::parse_download_client_routing_entry(&config);
                download_client_routing_settings_entry_from_domain(client_id, entry)
            })
            .collect::<Vec<_>>();
        let routing = self
            .complete_library_download_client_routing_entries(entries)
            .await?;
        Ok(Some(routing))
    }
}
impl AppUseCase {
    async fn complete_library_download_client_routing_entries(
        &self,
        entries: Vec<DownloadClientRoutingSettingsEntry>,
    ) -> AppResult<Vec<DownloadClientRoutingSettingsEntry>> {
        let mut completed = Vec::new();
        let mut seen = HashSet::new();

        for entry in entries {
            let entry = normalize_download_client_routing_settings_entry(entry)?;
            if seen.insert(entry.client_id.clone()) {
                completed.push(entry);
            }
        }

        for config in self
            .services
            .integrations
            .download_client_configs
            .list(None)
            .await?
        {
            if seen.insert(config.id.clone()) {
                completed.push(disabled_download_client_routing_settings_entry(config.id));
            }
        }

        Ok(completed)
    }
}
impl AppUseCase {
    async fn load_indexer_routing_override(
        &self,
        library_id: &str,
    ) -> AppResult<Option<Vec<IndexerRoutingSettingsEntry>>> {
        let Some(raw_json) = self
            .read_setting_string_value_explicit(INDEXER_ROUTING_SETTINGS_KEY, Some(library_id))
            .await?
        else {
            return Ok(None);
        };
        let Some(plan) = self.parse_indexer_routing_plan(library_id, &raw_json) else {
            return Ok(Some(Vec::new()));
        };
        let mut routing = plan
            .entries
            .into_iter()
            .map(|(indexer_id, entry)| IndexerRoutingSettingsEntry {
                indexer_id,
                enabled: entry.enabled,
                categories: entry.categories,
                priority: entry.priority as i32,
            })
            .collect::<Vec<_>>();
        routing.sort_by_key(|entry| (entry.priority, entry.indexer_id.clone()));
        Ok(Some(routing))
    }
}
impl AppUseCase {
    pub async fn get_download_client_routing(
        &self,
        actor: &User,
        scope_id: &str,
    ) -> AppResult<Vec<DownloadClientRoutingSettingsEntry>> {
        self.require_library_settings_read_permission(actor).await?;

        let raw_json = self.load_download_client_routing_json(scope_id).await?;
        let Some(raw_json) = raw_json else {
            return Ok(Vec::new());
        };
        let Some(entries) = crate::catalog_helpers::parse_download_client_routing_map(&raw_json)
        else {
            return Ok(Vec::new());
        };

        let mut routing = entries
            .into_iter()
            .map(|(client_id, config)| {
                let entry = crate::catalog_helpers::parse_download_client_routing_entry(&config);
                DownloadClientRoutingSettingsEntry {
                    client_id,
                    enabled: entry.enabled,
                    category: entry.category,
                    recent_queue_priority: entry.recent_queue_priority,
                    older_queue_priority: entry.older_queue_priority,
                    remove_completed: entry.remove_completed,
                    remove_failed: entry.remove_failed,
                    seeding_profile_id: entry.seeding_profile_id,
                }
            })
            .collect::<Vec<_>>();
        routing.sort_by(|left, right| left.client_id.cmp(&right.client_id));
        Ok(routing)
    }
}
impl AppUseCase {
    pub async fn update_download_client_routing(
        &self,
        actor: &User,
        scope_id: &str,
        entries: Vec<DownloadClientRoutingSettingsEntry>,
    ) -> AppResult<Vec<DownloadClientRoutingSettingsEntry>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        let mut payload = serde_json::Map::new();
        for entry in entries {
            let client_id = entry.client_id.trim();
            if client_id.is_empty() {
                return Err(AppError::Validation(
                    "download client routing entry requires client_id".to_string(),
                ));
            }

            payload.insert(
                client_id.to_string(),
                serde_json::json!({
                    "enabled": entry.enabled,
                    "category": normalize_optional_string(entry.category),
                    "recentQueuePriority": normalize_optional_string(entry.recent_queue_priority),
                    "olderQueuePriority": normalize_optional_string(entry.older_queue_priority),
                    "removeCompleted": entry.remove_completed,
                    "removeFailed": entry.remove_failed,
                    "seedingProfileId": normalize_optional_string(entry.seeding_profile_id),
                }),
            );
        }

        self.services
            .config
            .settings
            .upsert_setting_json(
                SETTINGS_SCOPE_SYSTEM,
                DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
                Some(scope_id.to_string()),
                serde_json::Value::Object(payload).to_string(),
                SETTINGS_SOURCE_TYPED_GRAPHQL,
                (!actor.is_system_execution_actor()).then(|| actor.id.clone()),
            )
            .await?;

        self.refresh_download_client_category_admission_best_effort()
            .await;

        self.emit_settings_saved(
            actor,
            "download_client_routing",
            Some(scope_id.to_string()),
            vec![DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY.to_string()],
        )
        .await;

        self.get_download_client_routing(actor, scope_id).await
    }
}
impl AppUseCase {
    pub async fn ensure_download_client_routing_entry_for_client(
        &self,
        actor: &User,
        client_id: &str,
    ) -> AppResult<()> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        let mut changed = false;
        for scope_id in ["movie", "series", "anime"] {
            let current = self.load_download_client_routing_json(scope_id).await?;
            let mut payload = current
                .as_deref()
                .and_then(parse_json_object)
                .unwrap_or_default();

            if payload.contains_key(client_id) {
                continue;
            }

            let next_priority = next_routing_priority(&payload);
            payload.insert(
                client_id.to_string(),
                default_download_client_routing_entry_json(next_priority),
            );

            self.services
                .config
                .settings
                .upsert_setting_json(
                    SETTINGS_SCOPE_SYSTEM,
                    DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
                    Some(scope_id.to_string()),
                    serde_json::Value::Object(payload).to_string(),
                    "admin_graphql",
                    (!actor.is_system_execution_actor()).then(|| actor.id.clone()),
                )
                .await?;
            changed = true;
        }

        if changed {
            self.refresh_download_client_category_admission_best_effort()
                .await;
        }

        Ok(())
    }
}
impl AppUseCase {
    pub async fn ensure_indexer_routing_entry_for_indexer(
        &self,
        actor: &User,
        indexer_id: &str,
    ) -> AppResult<()> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        self.ensure_indexer_routing_entry_for_indexer_internal(
            indexer_id,
            "admin_graphql",
            (!actor.is_system_execution_actor()).then(|| actor.id.clone()),
        )
        .await
    }
}
impl AppUseCase {
    async fn ensure_indexer_routing_entry_for_indexer_internal(
        &self,
        indexer_id: &str,
        source: &str,
        updated_by_user_id: Option<String>,
    ) -> AppResult<()> {
        let indexer_id = indexer_id.trim();
        if indexer_id.is_empty() {
            return Err(AppError::Validation(
                "indexer routing entry requires indexer_id".to_string(),
            ));
        }

        for scope_id in ["movie", "series", "anime"] {
            let current = self
                .read_setting_string_value(INDEXER_ROUTING_SETTINGS_KEY, Some(scope_id))
                .await?;
            let mut payload = current
                .as_deref()
                .and_then(parse_json_object)
                .unwrap_or_default();

            if payload.contains_key(indexer_id) {
                continue;
            }

            let next_priority = next_routing_priority(&payload);
            payload.insert(
                indexer_id.to_string(),
                default_indexer_routing_entry_json(scope_id, next_priority),
            );

            self.services
                .config
                .settings
                .upsert_setting_json(
                    SETTINGS_SCOPE_SYSTEM,
                    INDEXER_ROUTING_SETTINGS_KEY,
                    Some(scope_id.to_string()),
                    serde_json::Value::Object(payload).to_string(),
                    source,
                    updated_by_user_id.clone(),
                )
                .await?;
        }

        Ok(())
    }
}
impl AppUseCase {
    pub async fn ensure_indexer_routing_entries_for_existing_indexers(&self) -> AppResult<()> {
        let configs = self
            .services
            .integrations
            .indexer_configs
            .list(None)
            .await?;
        for config in configs {
            self.ensure_indexer_routing_entry_for_indexer_internal(
                &config.id,
                "startup_reconcile",
                None,
            )
            .await?;
        }
        Ok(())
    }
}
impl AppUseCase {
    pub(crate) async fn remove_indexer_routing_entries_internal(
        &self,
        indexer_ids: &[String],
        source: &str,
        updated_by_user_id: Option<String>,
    ) -> AppResult<()> {
        if indexer_ids.is_empty() {
            return Ok(());
        }

        for scope_id in ["movie", "series", "anime"] {
            let Some(raw_json) = self
                .read_setting_string_value(INDEXER_ROUTING_SETTINGS_KEY, Some(scope_id))
                .await?
            else {
                continue;
            };
            let mut payload = parse_json_object(&raw_json).ok_or_else(|| {
                AppError::Repository(format!(
                    "indexer routing settings for scope '{scope_id}' are not a JSON object"
                ))
            })?;
            let mut changed = false;
            for indexer_id in indexer_ids {
                changed |= payload.remove(indexer_id).is_some();
            }
            if !changed {
                continue;
            }

            self.services
                .config
                .settings
                .upsert_setting_json(
                    SETTINGS_SCOPE_SYSTEM,
                    INDEXER_ROUTING_SETTINGS_KEY,
                    Some(scope_id.to_string()),
                    serde_json::Value::Object(payload).to_string(),
                    source,
                    updated_by_user_id.clone(),
                )
                .await?;
        }
        Ok(())
    }
}
impl AppUseCase {
    pub async fn get_indexer_routing(
        &self,
        actor: &User,
        scope_id: &str,
    ) -> AppResult<Vec<IndexerRoutingSettingsEntry>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        let Some(plan) = self.resolve_indexer_routing(None, Some(scope_id)).await else {
            return Ok(Vec::new());
        };

        let mut routing = plan
            .entries
            .into_iter()
            .map(|(indexer_id, entry)| IndexerRoutingSettingsEntry {
                indexer_id,
                enabled: entry.enabled,
                categories: entry.categories,
                priority: entry.priority as i32,
            })
            .collect::<Vec<_>>();
        routing.sort_by_key(|entry| (entry.priority, entry.indexer_id.clone()));
        Ok(routing)
    }
}
impl AppUseCase {
    pub async fn update_indexer_routing(
        &self,
        actor: &User,
        scope_id: &str,
        entries: Vec<IndexerRoutingSettingsEntry>,
    ) -> AppResult<Vec<IndexerRoutingSettingsEntry>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        let _sync_guard = self
            .runtime
            .integrations
            .managed_indexer_sync_lock
            .clone()
            .lock_owned()
            .await;
        self.update_indexer_routing_without_sync_lock(actor, scope_id, entries)
            .await
    }

    pub(crate) async fn update_indexer_routing_without_sync_lock(
        &self,
        actor: &User,
        scope_id: &str,
        entries: Vec<IndexerRoutingSettingsEntry>,
    ) -> AppResult<Vec<IndexerRoutingSettingsEntry>> {
        let previous = self.get_indexer_routing(actor, scope_id).await?;

        let mut payload = serde_json::Map::new();
        for entry in entries {
            let indexer_id = entry.indexer_id.trim();
            if indexer_id.is_empty() {
                return Err(AppError::Validation(
                    "indexer routing entry requires indexer_id".to_string(),
                ));
            }

            let categories = entry
                .categories
                .into_iter()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>();

            payload.insert(
                indexer_id.to_string(),
                serde_json::json!({
                    "enabled": entry.enabled,
                    "categories": categories,
                    "priority": entry.priority,
                }),
            );
        }

        self.services
            .config
            .settings
            .upsert_setting_json(
                SETTINGS_SCOPE_SYSTEM,
                INDEXER_ROUTING_SETTINGS_KEY,
                Some(scope_id.to_string()),
                serde_json::Value::Object(payload).to_string(),
                SETTINGS_SOURCE_TYPED_GRAPHQL,
                (!actor.is_system_execution_actor()).then(|| actor.id.clone()),
            )
            .await?;

        self.emit_settings_saved(
            actor,
            "indexer_routing",
            Some(scope_id.to_string()),
            vec![INDEXER_ROUTING_SETTINGS_KEY.to_string()],
        )
        .await;

        let updated = self.get_indexer_routing(actor, scope_id).await?;
        let canonical = |entries: &[IndexerRoutingSettingsEntry]| {
            entries
                .iter()
                .map(|entry| {
                    let mut categories = entry.categories.clone();
                    categories.sort();
                    categories.dedup();
                    (
                        entry.indexer_id.clone(),
                        (entry.enabled, categories, entry.priority),
                    )
                })
                .collect::<std::collections::HashMap<_, _>>()
        };
        let previous_by_id = canonical(&previous);
        let updated_by_id = canonical(&updated);
        let changed_indexers = previous_by_id
            .keys()
            .chain(updated_by_id.keys())
            .filter(|indexer_id| previous_by_id.get(*indexer_id) != updated_by_id.get(*indexer_id))
            .cloned()
            .collect::<std::collections::HashSet<_>>();
        for indexer_id in changed_indexers {
            self.prune_indexer_search_learning_best_effort(
                &indexer_id,
                "indexer_routing_change",
            )
            .await;
        }
        Ok(updated)
    }
}
impl AppUseCase {
    /// Idempotent backfill: walks all persisted routing settings and rewrites
    /// any entry that is missing canonical fields with explicit defaults.
    /// Intended to run once per startup so legacy installs converge on the
    /// fully-materialized JSON shape that the typed write paths now produce.
    /// Reads stay read-only — this is the single explicit write boundary for
    /// the migration.
    pub async fn normalize_routing_settings(&self) -> AppResult<()> {
        const NORMALIZE_SOURCE: &str = "startup_normalize_routing";

        for scope_id in ["movie", "series", "anime"] {
            if let Some(raw_json) = self.load_download_client_routing_json(scope_id).await?
                && let Some(mut payload) = parse_json_object(&raw_json)
            {
                let mut changed = false;
                let mut next_priority = next_routing_priority(&payload);
                for (_, value) in payload.iter_mut() {
                    if let Some(entry) = value.as_object_mut() {
                        let missing_priority = !entry.contains_key("priority");
                        if normalize_download_client_routing_entry_in_place(entry, next_priority) {
                            changed = true;
                            if missing_priority {
                                next_priority += 1;
                            }
                        }
                    }
                }
                if changed {
                    self.services
                        .config
                        .settings
                        .upsert_setting_json(
                            SETTINGS_SCOPE_SYSTEM,
                            DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
                            Some(scope_id.to_string()),
                            serde_json::Value::Object(payload).to_string(),
                            NORMALIZE_SOURCE,
                            None,
                        )
                        .await?;
                }
            }
        }

        for scope_id in ["movie", "series", "anime"] {
            if let Some(raw_json) = self
                .read_setting_string_value(INDEXER_ROUTING_SETTINGS_KEY, Some(scope_id))
                .await?
                && let Some(mut payload) = parse_json_object(&raw_json)
            {
                let mut changed = false;
                let mut next_priority = next_routing_priority(&payload);
                for (_, value) in payload.iter_mut() {
                    if let Some(entry) = value.as_object_mut() {
                        let missing_priority = !entry.contains_key("priority");
                        if normalize_indexer_routing_entry_in_place(scope_id, entry, next_priority)
                        {
                            changed = true;
                            if missing_priority {
                                next_priority += 1;
                            }
                        }
                    }
                }
                if changed {
                    self.services
                        .config
                        .settings
                        .upsert_setting_json(
                            SETTINGS_SCOPE_SYSTEM,
                            INDEXER_ROUTING_SETTINGS_KEY,
                            Some(scope_id.to_string()),
                            serde_json::Value::Object(payload).to_string(),
                            NORMALIZE_SOURCE,
                            None,
                        )
                        .await?;
                }
            }
        }

        Ok(())
    }
}

/// Change only explicit `removeFailed: false` values, preserving every other
/// routing field and leaving missing values to their existing default path.
fn flip_explicit_remove_failed_defaults_in_place(
    payload: &mut serde_json::Map<String, serde_json::Value>,
) -> usize {
    let mut flipped = 0;
    for value in payload.values_mut() {
        let Some(entry) = value.as_object_mut() else {
            continue;
        };
        if entry.get("removeFailed") == Some(&serde_json::Value::Bool(false)) {
            entry.insert("removeFailed".to_string(), serde_json::Value::Bool(true));
            flipped += 1;
        }
    }
    flipped
}

impl AppUseCase {
    /// One-shot migration helper for the historic, normalization-written
    /// `removeFailed: false` default. Both facet routes and library overrides
    /// are explicit routing scopes and must be visited.
    pub async fn flip_explicit_remove_failed_defaults(&self) -> AppResult<usize> {
        const MIGRATION_SOURCE: &str = "startup_remove_failed_default_flip";

        let mut scope_ids = ["movie", "series", "anime"]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        scope_ids.extend(
            self.services
                .catalog
                .libraries
                .list(None)
                .await?
                .into_iter()
                .map(|library| library.id),
        );

        let routing_values = self
            .services
            .config
            .settings
            .list_setting_json_explicit_for_scope_ids(
                SETTINGS_SCOPE_SYSTEM,
                DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
                &scope_ids,
            )
            .await?;
        let mut flipped = 0;
        for (scope_id, raw_json) in routing_values {
            let Some(mut payload) = parse_json_object(&raw_json) else {
                continue;
            };
            let changed = flip_explicit_remove_failed_defaults_in_place(&mut payload);
            if changed == 0 {
                continue;
            }
            self.services
                .config
                .settings
                .upsert_setting_json(
                    SETTINGS_SCOPE_SYSTEM,
                    DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
                    Some(scope_id),
                    serde_json::Value::Object(payload).to_string(),
                    MIGRATION_SOURCE,
                    None,
                )
                .await?;
            flipped += changed;
        }
        Ok(flipped)
    }
}

#[cfg(test)]
mod remove_failed_default_tests {
    use super::*;

    #[test]
    fn flips_only_explicit_false_remove_failed_values() {
        let mut payload = serde_json::json!({
            "flip": {
                "enabled": false,
                "category": "movies",
                "removeCompleted": false,
                "removeFailed": false,
                "priority": 7
            },
            "keep_true": { "removeFailed": true, "priority": 3 },
            "keep_missing": { "enabled": true, "priority": 4 },
            "not_an_entry": "ignored"
        })
        .as_object()
        .expect("routing payload object")
        .clone();

        assert_eq!(flip_explicit_remove_failed_defaults_in_place(&mut payload), 1);
        assert_eq!(
            payload["flip"]["removeFailed"],
            serde_json::Value::Bool(true)
        );
        assert_eq!(payload["flip"]["enabled"], serde_json::Value::Bool(false));
        assert_eq!(payload["flip"]["category"], serde_json::Value::String("movies".to_string()));
        assert_eq!(payload["flip"]["removeCompleted"], serde_json::Value::Bool(false));
        assert_eq!(payload["flip"]["priority"], serde_json::Value::from(7));
        assert_eq!(
            payload["keep_true"]["removeFailed"],
            serde_json::Value::Bool(true)
        );
        assert!(payload["keep_missing"].get("removeFailed").is_none());
        assert_eq!(payload["not_an_entry"], serde_json::Value::String("ignored".to_string()));
    }
}

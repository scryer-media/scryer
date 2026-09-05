#[derive(Clone, Debug)]
struct PreparedManagedIndexerChild {
    child_key: String,
    name: String,
    provider_type: String,
    base_url: String,
    config_json: String,
    is_enabled: bool,
    enable_interactive_search: bool,
    enable_auto_search: bool,
    managed_metadata_json: Option<String>,
    caps_snapshot_json: Option<String>,
    routing_by_scope: HashMap<String, Vec<String>>,
}

pub(crate) struct CapsSnapshotRefreshOutcome {
    snapshot_json: Option<String>,
    error_message: Option<String>,
}

const PROWLARR_MANAGED_CHILD_RATE_LIMIT_SECONDS: i64 = 2;
const MANAGED_CHILD_LOCAL_DISABLES_KEY: &str = "locally_disabled_children";

fn managed_child_is_locally_disabled(metadata: Option<&str>, child_key: &str) -> bool {
    metadata
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .and_then(|value| {
            value
                .get(MANAGED_CHILD_LOCAL_DISABLES_KEY)?
                .as_array()
                .cloned()
        })
        .is_some_and(|keys| {
            keys.iter()
                .filter_map(serde_json::Value::as_str)
                .any(|key| key == child_key)
        })
}

fn with_managed_child_local_disable(
    metadata: Option<&str>,
    child_key: &str,
    locally_disabled: bool,
) -> AppResult<Option<String>> {
    let mut value = match metadata.map(str::trim).filter(|raw| !raw.is_empty()) {
        Some(raw) => serde_json::from_str::<serde_json::Value>(raw).map_err(|error| {
            AppError::Repository(format!("invalid managed child metadata: {error}"))
        })?,
        None => serde_json::json!({}),
    };
    let object = value.as_object_mut().ok_or_else(|| {
        AppError::Repository("managed indexer metadata must be a JSON object".into())
    })?;
    let mut disabled_children = object
        .get(MANAGED_CHILD_LOCAL_DISABLES_KEY)
        .and_then(serde_json::Value::as_array)
        .map(|keys| {
            keys.iter()
                .filter_map(serde_json::Value::as_str)
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if locally_disabled {
        if !disabled_children.iter().any(|key| key == child_key) {
            disabled_children.push(child_key.to_string());
        }
    } else {
        disabled_children.retain(|key| key != child_key);
    }
    if disabled_children.is_empty() {
        object.remove(MANAGED_CHILD_LOCAL_DISABLES_KEY);
    } else {
        object.insert(
            MANAGED_CHILD_LOCAL_DISABLES_KEY.to_string(),
            serde_json::Value::Array(
                disabled_children
                    .into_iter()
                    .map(serde_json::Value::String)
                    .collect(),
            ),
        );
    }
    if object.is_empty() {
        Ok(None)
    } else {
        serde_json::to_string(&value)
            .map(Some)
            .map_err(|error| AppError::Repository(error.to_string()))
    }
}

fn merge_managed_child_metadata(existing: Option<&str>, desired: Option<&str>) -> Option<String> {
    let desired = desired?.trim();
    if desired.is_empty() {
        return None;
    }

    let mut desired_value = serde_json::from_str::<serde_json::Value>(desired).ok()?;
    let desired_object = desired_value.as_object_mut()?;
    if desired_object
        .get("caps_snapshot")
        .is_some_and(|value| !value.is_null())
    {
        return Some(desired.to_string());
    }

    let existing_snapshot = existing
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .and_then(|value| value.as_object().cloned())
        .and_then(|object| object.get("caps_snapshot").cloned())
        .filter(|value| !value.is_null())?;

    desired_object.insert("caps_snapshot".to_string(), existing_snapshot);
    serde_json::to_string(&desired_value).ok()
}
fn next_indexer_routing_priority(entries: &[IndexerRoutingSettingsEntry]) -> i32 {
    entries
        .iter()
        .map(|entry| entry.priority)
        .max()
        .unwrap_or(0)
        + 1
}
fn upsert_indexer_routing_entry(
    entries: &mut Vec<IndexerRoutingSettingsEntry>,
    indexer_id: &str,
    categories: Vec<String>,
) {
    if let Some(entry) = entries
        .iter_mut()
        .find(|entry| entry.indexer_id == indexer_id)
    {
        entry.categories = categories;
        return;
    }

    entries.push(IndexerRoutingSettingsEntry {
        indexer_id: indexer_id.to_string(),
        enabled: true,
        categories,
        priority: next_indexer_routing_priority(entries),
    });
}
fn parse_indexer_config_json(
    config_json: Option<&str>,
) -> AppResult<serde_json::Map<String, serde_json::Value>> {
    let raw = config_json.unwrap_or_default().trim();
    if raw.is_empty() {
        return Ok(serde_json::Map::new());
    }

    let parsed: serde_json::Value =
        serde_json::from_str(raw).map_err(|error| AppError::Validation(error.to_string()))?;
    parsed
        .as_object()
        .cloned()
        .ok_or_else(|| AppError::Validation("indexer config_json must be a JSON object".into()))
}
fn indexer_connection_url_field(
    fields: &[scryer_domain::ConfigFieldDef],
) -> AppResult<Option<&scryer_domain::ConfigFieldDef>> {
    let mut connection_fields = fields
        .iter()
        .filter(|field| field.role == Some(scryer_domain::ConfigFieldRole::ConnectionUrl));
    let Some(field) = connection_fields.next() else {
        return Ok(None);
    };
    if connection_fields.next().is_some() {
        return Err(AppError::Validation(
            "indexer provider declares multiple connection_url config fields".into(),
        ));
    }
    Ok(Some(field))
}
pub(crate) fn derive_indexer_base_url_from_config_fields(
    fields: &[scryer_domain::ConfigFieldDef],
    config_json: Option<&str>,
) -> AppResult<String> {
    let Some(field) = indexer_connection_url_field(fields)? else {
        return Ok(String::new());
    };
    let object = parse_indexer_config_json(config_json)?;
    let raw = object
        .get(&field.key)
        .and_then(config_value_to_string)
        .or_else(|| {
            field
                .default_value
                .as_deref()
                .map(str::trim)
                .map(str::to_string)
        })
        .filter(|value| !value.is_empty())
        .ok_or_else(|| AppError::Validation("indexer connection URL is required".into()))?;

    if (field.key.contains("feed") || field.key.contains("rss"))
        && let Some(origin) = extract_base_url_origin(&raw)
    {
        return Ok(origin);
    }

    Ok(raw)
}
/// Scheme-preserving origin for base-URL derivation (`https://host[:port]`).
/// Distinct from `extract_url_origin`, which was repurposed for display
/// labels and returns the bare host — a base URL must keep its scheme or it
/// fails downstream URL validation.
fn extract_base_url_origin(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let (scheme, remainder) = trimmed.split_once("://")?;
    if scheme.is_empty() {
        return None;
    }

    let authority = remainder.split(['/', '?', '#']).next()?.trim();
    if authority.is_empty() {
        return None;
    }

    Some(format!("{scheme}://{authority}"))
}
pub(crate) fn normalize_indexer_config_json(
    fields: &[scryer_domain::ConfigFieldDef],
    config_json: Option<&str>,
    persisted_config_json: Option<&str>,
) -> AppResult<String> {
    let connection_url_field = indexer_connection_url_field(fields)?;
    let option_supplies_connection_url = connection_url_field.is_some_and(|connection_field| {
        fields.iter().any(|field| {
            field
                .options
                .iter()
                .any(|option| option.config_overrides.contains_key(&connection_field.key))
        })
    });

    let mut object = parse_indexer_config_json(config_json)?;
    let persisted = parse_indexer_config_json(persisted_config_json)?;

    for field in fields {
        if !object.contains_key(&field.key)
            && let Some(stored) = persisted.get(&field.key)
            && !config_value_is_empty(Some(stored))
        {
            object.insert(field.key.clone(), stored.clone());
        }
    }

    for field in fields {
        let Some(selected_value) = object.get(&field.key).and_then(serde_json::Value::as_str)
        else {
            continue;
        };
        let Some(selected_option) = field
            .options
            .iter()
            .find(|option| option.value == selected_value)
        else {
            continue;
        };
        for (key, value) in &selected_option.config_overrides {
            if fields.iter().any(|candidate| candidate.key == *key)
                && config_value_is_empty(object.get(key))
            {
                object.insert(key.clone(), serde_json::Value::String(value.clone()));
            }
        }
    }

    for field in fields {
        if config_value_is_empty(object.get(&field.key))
            && let Some(default_value) = field
                .default_value
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
        {
            object.insert(
                field.key.clone(),
                serde_json::Value::String(default_value.to_string()),
            );
        }

        // Requiredness is read through the shared evaluator rather than off
        // `required` alone: a field the form is hiding must not be demanded
        // here, or the operator is left with an error about a field they
        // cannot see.
        let value_of = |key: &str| {
            object
                .get(key)
                .and_then(serde_json::Value::as_str)
        };
        if scryer_domain::config_field_is_required(field, value_of)
            && config_value_is_empty(object.get(&field.key))
        {
            return Err(AppError::Validation(format!(
                "{} is required",
                field.label.trim()
            )));
        }
    }

    if option_supplies_connection_url
        && let Some(connection_field) = connection_url_field
        && config_value_is_empty(object.get(&connection_field.key))
    {
        return Err(AppError::Validation(format!(
            "{} is required",
            connection_field.label.trim()
        )));
    }

    serde_json::to_string(&serde_json::Value::Object(object))
        .map_err(|error| AppError::Repository(error.to_string()))
}
impl AppUseCase {
    pub fn indexer_config_fields_for_provider_type(
        &self,
        provider_type: &str,
    ) -> AppResult<Vec<scryer_domain::ConfigFieldDef>> {
        let normalized = provider_type.trim().to_lowercase();
        let Some(provider) = self.services.integrations.plugin_provider.available() else {
            return Err(AppError::Validation(
                "indexer provider is unavailable".into(),
            ));
        };
        if !provider
            .available_provider_types()
            .into_iter()
            .any(|value| value == normalized)
        {
            return Err(AppError::Validation(format!(
                "unsupported indexer provider type '{provider_type}'"
            )));
        }

        let fields = provider.config_fields_for_provider(&normalized);
        indexer_connection_url_field(&fields)?;
        Ok(fields)
    }
}
impl AppUseCase {
    fn indexer_management_capabilities_for_provider_type(
        &self,
        provider_type: &str,
    ) -> scryer_domain::IndexerManagementCapabilities {
        self.services
            .integrations
            .plugin_provider
            .available()
            .map(|provider| provider.management_capabilities_for_provider(provider_type))
            .unwrap_or_default()
    }
}
impl AppUseCase {
    pub(crate) async fn fetch_caps_snapshot_json_for_config(
        &self,
        config: &IndexerConfig,
    ) -> AppResult<Option<String>> {
        let Some(refresher) = self
            .services
            .integrations
            .indexer_caps_refresher
            .available()
        else {
            return Ok(None);
        };
        let Some(snapshot) = refresher.fetch_for_config(config).await? else {
            if config.is_direct_nab() {
                return Err(AppError::Repository(
                    "caps refresh returned no Newznab caps snapshot".into(),
                ));
            }
            return Ok(None);
        };
        serde_json::to_string(&snapshot)
            .map(Some)
            .map_err(|error| AppError::Repository(error.to_string()))
    }
}
impl AppUseCase {
    pub(crate) async fn prune_indexer_search_learning_best_effort(
        &self,
        indexer_id: &str,
        reason: &'static str,
    ) {
        if let Err(error) = self
            .services
            .integrations
            .indexer_client
            .prune_search_learning(indexer_id)
            .await
        {
            tracing::warn!(
                config_id = indexer_id,
                reason,
                error = %error,
                "failed to invalidate indexer search learning"
            );
        }
    }

    async fn record_caps_refresh_failure(&self, config: &IndexerConfig, error: &AppError) {
        let message = format!("{} {error}", crate::INDEXER_CAPS_REFRESH_ERROR_PREFIX);
        if let Err(record_error) = self
            .services
            .integrations
            .indexer_configs
            .record_last_error(&config.id, Some(message))
            .await
        {
            tracing::warn!(config_id = %config.id, error = %record_error, "failed to persist indexer caps health error");
        }
        if let Err(prune_error) = self
            .services
            .integrations
            .scope_indexer_coverage
            .prune_indexer(&config.id)
            .await
        {
            tracing::warn!(config_id = %config.id, error = %prune_error, "failed to invalidate coverage after caps refresh failure");
        }
        self.prune_indexer_search_learning_best_effort(&config.id, "caps_refresh_failure")
            .await;
    }

    pub(crate) async fn refresh_caps_snapshot_json_best_effort(
        &self,
        config: &IndexerConfig,
        fallback: Option<&str>,
    ) -> CapsSnapshotRefreshOutcome {
        match self.fetch_caps_snapshot_json_for_config(config).await {
            Ok(Some(snapshot_json)) => {
                if config.last_error_message.as_deref().is_some_and(|message| {
                    message.starts_with(crate::INDEXER_CAPS_REFRESH_ERROR_PREFIX)
                }) && let Err(error) = self
                    .services
                    .integrations
                    .indexer_configs
                    .clear_last_error(&config.id)
                    .await
                {
                    tracing::warn!(config_id = %config.id, error = %error, "failed to clear recovered indexer caps error");
                }
                CapsSnapshotRefreshOutcome {
                    snapshot_json: Some(snapshot_json),
                    error_message: None,
                }
            }
            Ok(None) => CapsSnapshotRefreshOutcome {
                snapshot_json: fallback.map(ToOwned::to_owned),
                error_message: None,
            },
            Err(error) => {
                let error_message = format!("{} {error}", crate::INDEXER_CAPS_REFRESH_ERROR_PREFIX);
                self.record_caps_refresh_failure(config, &error).await;
                tracing::warn!(
                    config_id = %config.id,
                    provider_type = %config.provider_type,
                    error = %error,
                    "failed to refresh indexer caps snapshot; keeping the last known snapshot"
                );
                CapsSnapshotRefreshOutcome {
                    snapshot_json: fallback.map(ToOwned::to_owned),
                    error_message: Some(error_message),
                }
            }
        }
    }
}
impl AppUseCase {
    pub async fn list_indexer_configs(
        &self,
        actor: &User,
        provider_filter: Option<String>,
    ) -> AppResult<Vec<IndexerConfig>> {
        let settings_permissions = scryer_domain::AppPermissionMask::from_permissions([
            scryer_domain::AppPermission::ManageSystemSettings,
            scryer_domain::AppPermission::ManageCatalogSettings,
        ]);
        if !self
            .has_any_app_permission(actor, settings_permissions)
            .await?
        {
            return Err(AppError::Unauthorized(
                "You do not have permission to perform this action".to_string(),
            ));
        }
        self.services
            .integrations
            .indexer_configs
            .list(provider_filter.map(|provider| provider.trim().to_lowercase()))
            .await
    }
}
impl AppUseCase {
    pub async fn refresh_enabled_direct_nab_caps_snapshots(
        &self,
        actor: &User,
    ) -> AppResult<(u32, Vec<String>)> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let configs = self
            .services
            .integrations
            .indexer_configs
            .list(None)
            .await?;
        let mut refreshed = 0_u32;
        let mut failures = Vec::new();

        for config in configs {
            if !config.is_enabled || !config.is_direct_nab() {
                continue;
            }

            match self.fetch_caps_snapshot_json_for_config(&config).await {
                Ok(Some(snapshot_json)) => {
                    let updated =
                        if config.caps_snapshot_json.as_deref() != Some(snapshot_json.as_str()) {
                            self.services
                                .integrations
                                .indexer_configs
                                .update(IndexerConfigUpdate {
                                    id: config.id.clone(),
                                    caps_snapshot_json: Some(Some(snapshot_json)),
                                    ..Default::default()
                                })
                                .await?
                        } else {
                            config.clone()
                        };
                    if crate::indexer_search_identity(&config, None)
                        != crate::indexer_search_identity(&updated, None)
                    {
                        self.prune_indexer_search_learning_best_effort(
                            &config.id,
                            "caps_snapshot_change",
                        )
                        .await;
                    }
                    if config.last_error_message.as_deref().is_some_and(|message| {
                        message.starts_with(crate::INDEXER_CAPS_REFRESH_ERROR_PREFIX)
                    }) && let Err(error) = self
                        .services
                        .integrations
                        .indexer_configs
                        .clear_last_error(&config.id)
                        .await
                    {
                        tracing::warn!(config_id = %config.id, error = %error, "failed to clear recovered indexer caps error");
                    }
                    refreshed += 1;
                }
                Ok(None) => {}
                Err(error) => {
                    self.record_caps_refresh_failure(&config, &error).await;
                    tracing::warn!(
                        config_id = %config.id,
                        provider_type = %config.provider_type,
                        error = %error,
                        "failed to refresh direct indexer caps snapshot"
                    );
                    failures.push(format!("{}: {}", config.name, error));
                }
            }
        }

        Ok((refreshed, failures))
    }
}
impl AppUseCase {
    pub async fn sync_enabled_prowlarr_indexers(
        &self,
        actor: &User,
    ) -> AppResult<(u32, Vec<String>)> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let parents = self
            .services
            .integrations
            .indexer_configs
            .list(Some("prowlarr".to_string()))
            .await?
            .into_iter()
            .filter(|config| config.managed_parent_config_id.is_none() && config.is_enabled)
            .collect::<Vec<_>>();

        let mut synced_count = 0;
        let mut failures = Vec::new();
        for parent in parents {
            match self.sync_indexer_config(actor, &parent.id).await {
                Ok(_) => synced_count += 1,
                Err(error) => failures.push(format!("{}: {error}", parent.name)),
            }
        }

        Ok((synced_count, failures))
    }
}
impl AppUseCase {
    async fn validate_enabled_proxy_config_id(&self, raw_id: &str) -> AppResult<String> {
        let id = raw_id.trim();
        if id.is_empty() {
            return Err(AppError::Validation(
                "proxy config id cannot be empty".into(),
            ));
        }
        let config = self
            .services
            .integrations
            .proxy_configs
            .get_by_id(id)
            .await?
            .ok_or_else(|| AppError::Validation("Proxy configuration was not found.".into()))?;
        if !config.is_enabled {
            return Err(AppError::Validation(
                "Proxy is disabled for this indexer.".into(),
            ));
        }
        Ok(config.id)
    }
}
impl AppUseCase {
    pub async fn get_indexer_config(
        &self,
        actor: &User,
        config_id: &str,
    ) -> AppResult<Option<IndexerConfig>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        self.services
            .integrations
            .indexer_configs
            .get_by_id(config_id)
            .await
    }
}
impl AppUseCase {
    pub async fn create_indexer_config(
        &self,
        actor: &User,
        input: NewIndexerConfig,
    ) -> AppResult<IndexerConfig> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let name = input.name.trim().to_string();
        if name.is_empty() {
            return Err(AppError::Validation("indexer name is required".into()));
        }

        let provider_type = input.provider_type.trim().to_lowercase();
        if provider_type.is_empty() {
            return Err(AppError::Validation("provider type is required".into()));
        }

        let fields = self.indexer_config_fields_for_provider_type(&provider_type)?;
        let management_capabilities =
            self.indexer_management_capabilities_for_provider_type(&provider_type);
        let normalized_config_json =
            normalize_indexer_config_json(&fields, input.config_json.as_deref(), None)?;
        let base_url =
            derive_indexer_base_url_from_config_fields(&fields, Some(&normalized_config_json))?;
        let proxy_config_id = match input.proxy_config_id {
            Some(id) => Some(self.validate_enabled_proxy_config_id(&id).await?),
            None => None,
        };
        let download_client_id = input
            .download_client_id
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty());
        self.test_indexer_connection(
            actor,
            &provider_type,
            Some(&normalized_config_json),
            None,
            Some(proxy_config_id.as_deref()),
        )
        .await?;

        let mut config = IndexerConfig {
            id: Id::new().0,
            name,
            provider_type,
            base_url,
            api_key_encrypted: None,
            rate_limit_seconds: input.rate_limit_seconds,
            rate_limit_burst: input.rate_limit_burst,
            disabled_until: None,
            is_enabled: input.is_enabled,
            enable_interactive_search: if management_capabilities.supports_managed_children_sync {
                false
            } else {
                input.enable_interactive_search
            },
            enable_auto_search: if management_capabilities.supports_managed_children_sync {
                false
            } else {
                input.enable_auto_search
            },
            proxy_config_id,
            download_client_id,
            seeding_profile_id: None,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            config_json: Some(normalized_config_json),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        if let Some(client_id) = config.download_client_id.as_deref() {
            let client = self
                .services
                .integrations
                .download_client_configs
                .get_by_id(client_id)
                .await?
                .ok_or_else(|| {
                    AppError::NotFound(format!("download client config '{client_id}' not found"))
                })?;
            self.validate_indexer_download_client_mapping(&config, &client)?;
        }
        let caps_refresh = self
            .refresh_caps_snapshot_json_best_effort(&config, None)
            .await;
        config.caps_snapshot_json = caps_refresh.snapshot_json;
        if let Some(error_message) = caps_refresh.error_message {
            config.last_error_message = Some(error_message);
            config.last_error_at = Some(Utc::now());
        }

        let created = self
            .services
            .integrations
            .indexer_configs
            .create(config)
            .await?;
        self.ensure_indexer_routing_entry_for_indexer(actor, &created.id)
            .await?;
        if management_capabilities.supports_managed_children_sync && created.is_enabled {
            self.queue_managed_indexer_sync(actor, &created.id);
        }
        self.publish_indexers_changed();
        Ok(created)
    }
}
impl AppUseCase {
    pub async fn update_indexer_config(
        &self,
        actor: &User,
        update: IndexerConfigUpdate,
    ) -> AppResult<IndexerConfig> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let config_id = update.id.trim();
        if config_id.is_empty() {
            return Err(AppError::Validation("indexer config id is required".into()));
        }
        if !update.has_changes() {
            return Err(AppError::Validation(
                "at least one indexer field must be provided".into(),
            ));
        }
        let mut changes_other_than_enabled = update.clone();
        changes_other_than_enabled.is_enabled = None;
        let managed_enabled_only =
            update.is_enabled.is_some() && !changes_other_than_enabled.has_changes();

        let normalized_name = update.name.map(|value| value.trim().to_string());
        if normalized_name.as_ref().is_some_and(String::is_empty) {
            return Err(AppError::Validation("indexer name cannot be empty".into()));
        }

        let normalized_provider = update
            .provider_type
            .map(|value| value.trim().to_lowercase());
        if normalized_provider.as_ref().is_some_and(String::is_empty) {
            return Err(AppError::Validation("provider type cannot be empty".into()));
        }

        let existing = self
            .services
            .integrations
            .indexer_configs
            .get_by_id(config_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("indexer config '{config_id}' not found")))?;
        if existing.managed_parent_config_id.is_some()
            && managed_enabled_only
            && let Some(requested_enabled) = update.is_enabled
        {
            return self
                .set_managed_child_indexer_enabled(actor, &existing, requested_enabled)
                .await;
        }
        if existing.managed_parent_config_id.is_some() {
            return Err(AppError::Validation(
                "managed child indexers are controlled by their parent sync and cannot be edited directly"
                    .into(),
            ));
        }
        let effective_provider = normalized_provider
            .as_deref()
            .unwrap_or(existing.provider_type.as_str())
            .to_string();
        let normalized_download_client_id = update.download_client_id.clone().map(|value| {
            value
                .map(|id| id.trim().to_string())
                .filter(|id| !id.is_empty())
        });
        let fields = self.indexer_config_fields_for_provider_type(&effective_provider)?;
        let normalized_config_json = update
            .config_json
            .as_deref()
            .map(|raw| {
                normalize_indexer_config_json(&fields, Some(raw), existing.config_json.as_deref())
            })
            .transpose()?;
        let normalized_base_url =
            if normalized_config_json.is_some() || normalized_provider.is_some() {
                let config_source = normalized_config_json
                    .as_deref()
                    .or(existing.config_json.as_deref());
                Some(derive_indexer_base_url_from_config_fields(
                    &fields,
                    config_source,
                )?)
            } else {
                None
            };
        let management_capabilities =
            self.indexer_management_capabilities_for_provider_type(&effective_provider);
        let normalized_proxy_config_id = match update.proxy_config_id.clone() {
            Some(Some(id)) => {
                if existing.managed_parent_config_id.is_some() {
                    return Err(AppError::Validation(
                        "managed indexers cannot use a proxy; the managing application owns challenge solving".into(),
                    ));
                }
                Some(Some(self.validate_enabled_proxy_config_id(&id).await?))
            }
            Some(None) => Some(None),
            None => None,
        };
        let should_validate_connection = normalized_provider.is_some()
            || normalized_config_json.is_some()
            || normalized_proxy_config_id.is_some()
            || matches!(update.is_enabled, Some(true)) && !existing.is_enabled;
        let should_sync_managed_children = management_capabilities.supports_managed_children_sync
            && updated_managed_parent_requires_sync(
                &existing,
                update.is_enabled,
                normalized_provider.is_some(),
                normalized_config_json.is_some(),
                normalized_proxy_config_id.is_some(),
            );

        if should_validate_connection {
            let validation_config_json = normalized_config_json
                .as_deref()
                .or(existing.config_json.as_deref());
            let proxy_override = normalized_proxy_config_id
                .as_ref()
                .map(|value| value.as_deref());
            self.test_indexer_connection(
                actor,
                &effective_provider,
                validation_config_json,
                None,
                proxy_override,
            )
            .await?;
        }

        let preview_config = IndexerConfig {
            id: existing.id.clone(),
            name: normalized_name
                .clone()
                .unwrap_or_else(|| existing.name.clone()),
            provider_type: normalized_provider
                .clone()
                .unwrap_or_else(|| existing.provider_type.clone()),
            base_url: normalized_base_url
                .clone()
                .unwrap_or_else(|| existing.base_url.clone()),
            api_key_encrypted: existing.api_key_encrypted.clone(),
            rate_limit_seconds: update.rate_limit_seconds.or(existing.rate_limit_seconds),
            rate_limit_burst: update.rate_limit_burst.or(existing.rate_limit_burst),
            disabled_until: existing.disabled_until,
            is_enabled: update.is_enabled.unwrap_or(existing.is_enabled),
            enable_interactive_search: if management_capabilities.supports_managed_children_sync {
                false
            } else {
                update
                    .enable_interactive_search
                    .unwrap_or(existing.enable_interactive_search)
            },
            enable_auto_search: if management_capabilities.supports_managed_children_sync {
                false
            } else {
                update
                    .enable_auto_search
                    .unwrap_or(existing.enable_auto_search)
            },
            proxy_config_id: normalized_proxy_config_id
                .clone()
                .unwrap_or_else(|| existing.proxy_config_id.clone()),
            download_client_id: normalized_download_client_id
                .clone()
                .unwrap_or_else(|| existing.download_client_id.clone()),
            seeding_profile_id: existing.seeding_profile_id.clone(),
            managed_parent_config_id: update
                .managed_parent_config_id
                .clone()
                .unwrap_or_else(|| existing.managed_parent_config_id.clone()),
            managed_child_key: update
                .managed_child_key
                .clone()
                .unwrap_or_else(|| existing.managed_child_key.clone()),
            managed_metadata_json: update
                .managed_metadata_json
                .clone()
                .unwrap_or_else(|| existing.managed_metadata_json.clone()),
            caps_snapshot_json: existing.caps_snapshot_json.clone(),
            last_health_status: existing.last_health_status.clone(),
            last_error_message: existing.last_error_message.clone(),
            last_error_at: existing.last_error_at,
            config_json: normalized_config_json
                .clone()
                .or_else(|| existing.config_json.clone()),
            created_at: existing.created_at,
            updated_at: existing.updated_at,
        };
        if (normalized_provider.is_some() || normalized_download_client_id.is_some())
            && let Some(client_id) = preview_config.download_client_id.as_deref()
        {
            let client = self
                .services
                .integrations
                .download_client_configs
                .get_by_id(client_id)
                .await?
                .ok_or_else(|| {
                    AppError::Validation(format!(
                        "mapped download client '{client_id}' no longer exists"
                    ))
                })?;
            self.validate_indexer_download_client_mapping(&preview_config, &client)?;
        }
        let caps_refresh = self
            .refresh_caps_snapshot_json_best_effort(
                &preview_config,
                existing.caps_snapshot_json.as_deref(),
            )
            .await;

        let mut updated = self
            .services
            .integrations
            .indexer_configs
            .update(IndexerConfigUpdate {
                id: config_id.to_string(),
                name: normalized_name,
                provider_type: normalized_provider,
                derived_base_url: normalized_base_url,
                rate_limit_seconds: update.rate_limit_seconds,
                rate_limit_burst: update.rate_limit_burst,
                is_enabled: update.is_enabled,
                enable_interactive_search: if management_capabilities.supports_managed_children_sync
                {
                    Some(false)
                } else {
                    update.enable_interactive_search
                },
                enable_auto_search: if management_capabilities.supports_managed_children_sync {
                    Some(false)
                } else {
                    update.enable_auto_search
                },
                proxy_config_id: normalized_proxy_config_id,
                download_client_id: normalized_download_client_id,
                seeding_profile_id: None,
                managed_parent_config_id: update.managed_parent_config_id,
                managed_child_key: update.managed_child_key,
                managed_metadata_json: update.managed_metadata_json,
                caps_snapshot_json: Some(caps_refresh.snapshot_json),
                config_json: normalized_config_json,
            })
            .await?;
        if caps_refresh.error_message.is_none()
            && crate::indexer_search_identity(&existing, None)
                != crate::indexer_search_identity(&updated, None)
        {
            self.prune_indexer_search_learning_best_effort(
                &updated.id,
                "search_relevant_config_change",
            )
            .await;
        }
        if should_validate_connection && caps_refresh.error_message.is_none() {
            let indexer_configs = &self.services.integrations.indexer_configs;
            indexer_configs.clear_last_error(&updated.id).await?;
            // A save that just passed validation is the operator's "try again":
            // drop the persisted system backoff and its in-memory mirror so the
            // next search dispatches to this indexer instead of skipping it.
            indexer_configs.clear_system_backoff(&updated.id).await?;
            updated.disabled_until = None;
            self.services
                .integrations
                .indexer_client
                .reset_indexer_backoff(&updated.id)
                .await;
        }
        if should_sync_managed_children {
            if updated.is_enabled {
                self.queue_managed_indexer_sync(actor, &updated.id);
            } else if existing.is_enabled != updated.is_enabled
                && let Err(error) = self
                    .set_managed_child_indexers_enabled_state(&updated.id, false)
                    .await
            {
                self.publish_indexers_changed();
                return Err(error);
            }
        }
        self.publish_indexers_changed();
        Ok(updated)
    }
}
impl AppUseCase {
    pub async fn delete_indexer_config(&self, actor: &User, config_id: &str) -> AppResult<()> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        self.delete_indexer_config_tree(config_id, false, "admin_graphql", Some(actor.id.clone()))
            .await?;
        Ok(())
    }

    pub(crate) async fn delete_indexer_config_tree(
        &self,
        config_id: &str,
        allow_managed_child: bool,
        source: &str,
        updated_by_user_id: Option<String>,
    ) -> AppResult<Vec<String>> {
        let config_id = config_id.trim();
        let config = self
            .services
            .integrations
            .indexer_configs
            .get_by_id(config_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("indexer config '{config_id}' not found")))?;
        if config.managed_parent_config_id.is_some() && !allow_managed_child {
            return Err(AppError::Validation(
                "managed child indexers are controlled by their parent sync".into(),
            ));
        }

        let configs = self
            .services
            .integrations
            .indexer_configs
            .list(None)
            .await?;
        let mut deletion_layers = vec![vec![config.id.clone()]];
        loop {
            let parent_ids = deletion_layers.last().expect("deletion tree has a root");
            let next_layer = configs
                .iter()
                .filter(|candidate| {
                    candidate
                        .managed_parent_config_id
                        .as_ref()
                        .is_some_and(|parent_id| parent_ids.contains(parent_id))
                })
                .map(|candidate| candidate.id.clone())
                .collect::<Vec<_>>();
            if next_layer.is_empty() {
                break;
            }
            deletion_layers.push(next_layer);
        }

        let deleted_ids = deletion_layers
            .iter()
            .rev()
            .flatten()
            .cloned()
            .collect::<Vec<_>>();
        let (root_id, descendant_ids) = deleted_ids
            .split_last()
            .expect("deletion tree always contains its root");
        for id in descendant_ids {
            self.services
                .integrations
                .indexer_configs
                .delete(id)
                .await?;
        }
        self.remove_indexer_routing_entries_internal(&deleted_ids, source, updated_by_user_id)
            .await?;
        self.services
            .integrations
            .indexer_configs
            .delete(root_id)
            .await?;
        // A deleted indexer's blocklist rows can never match again -- the key
        // carries its id -- but they would still render in the UI as blocks the
        // operator cannot reason about. Managed children churn ids on re-sync,
        // so this is the path that keeps that from accumulating.
        for id in &deleted_ids {
            if let Err(error) = self
                .services
                .workflow
                .blocklist_repo
                .delete_for_indexer(id)
                .await
            {
                tracing::warn!(
                    config_id = %id,
                    error = %error,
                    "failed to drop blocklist entries for deleted indexer"
                );
            }
        }
        self.publish_indexers_changed();
        Ok(deleted_ids)
    }

    pub async fn reconcile_orphaned_managed_indexer_configs(&self) -> AppResult<u32> {
        let configs = self
            .services
            .integrations
            .indexer_configs
            .list(None)
            .await?;
        let known_ids = configs
            .iter()
            .map(|config| config.id.as_str())
            .collect::<Vec<_>>();
        let mut orphan_ids = configs
            .iter()
            .filter_map(|config| {
                let parent_id = config.managed_parent_config_id.as_deref()?;
                (!known_ids.contains(&parent_id)).then(|| config.id.clone())
            })
            .collect::<Vec<_>>();

        loop {
            let mut added = false;
            for config in &configs {
                if orphan_ids.contains(&config.id) {
                    continue;
                }
                if config
                    .managed_parent_config_id
                    .as_ref()
                    .is_some_and(|parent_id| orphan_ids.contains(parent_id))
                {
                    orphan_ids.push(config.id.clone());
                    added = true;
                }
            }
            if !added {
                break;
            }
        }

        self.remove_indexer_routing_entries_internal(&orphan_ids, "startup_orphan_reconcile", None)
            .await?;
        for orphan_id in orphan_ids.iter().rev() {
            self.services
                .integrations
                .indexer_configs
                .delete(orphan_id)
                .await?;
        }
        if !orphan_ids.is_empty() {
            tracing::info!(
                deleted = orphan_ids.len(),
                "removed orphaned managed indexer configs"
            );
            self.publish_indexers_changed();
        }
        Ok(orphan_ids.len() as u32)
    }
}
impl AppUseCase {
    pub async fn sync_indexer_config(
        &self,
        actor: &User,
        config_id: &str,
    ) -> AppResult<IndexerConfigSyncResult> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let config_id = config_id.trim();
        if config_id.is_empty() {
            return Err(AppError::Validation("indexer config id is required".into()));
        }

        let _sync_guard = self
            .runtime
            .integrations
            .managed_indexer_sync_lock
            .clone()
            .lock_owned()
            .await;
        let mut indexers_changed = false;
        macro_rules! try_sync_step {
            ($expr:expr) => {
                match $expr {
                    Ok(value) => value,
                    Err(error) => {
                        if indexers_changed {
                            self.publish_indexers_changed();
                        }
                        return Err(error);
                    }
                }
            };
        }

        let parent = self
            .services
            .integrations
            .indexer_configs
            .get_by_id(config_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("indexer config '{config_id}' not found")))?;
        if parent.managed_parent_config_id.is_some() {
            return Err(AppError::Validation(
                "managed child indexers cannot be synced directly".into(),
            ));
        }

        let provider = self
            .services
            .integrations
            .plugin_provider
            .available()
            .ok_or_else(|| AppError::Repository("indexer provider not available".into()))?;
        let management_capabilities =
            provider.management_capabilities_for_provider(&parent.provider_type);
        if !management_capabilities.supports_managed_children_sync {
            return Err(AppError::Validation(format!(
                "provider type '{}' does not support managed child sync",
                parent.provider_type
            )));
        }

        let parent = if parent.enable_interactive_search || parent.enable_auto_search {
            let updated = self
                .services
                .integrations
                .indexer_configs
                .update(IndexerConfigUpdate {
                    id: parent.id.clone(),
                    enable_interactive_search: Some(false),
                    enable_auto_search: Some(false),
                    ..Default::default()
                })
                .await?;
            indexers_changed = true;
            updated
        } else {
            parent
        };

        let client = try_sync_step!(provider.management_client_for_provider(&parent).ok_or_else(
            || {
                AppError::Validation(format!(
                    "no indexer management client available for provider type '{}'",
                    parent.provider_type
                ))
            }
        ));

        let plan = try_sync_step!(client.plan_sync(&parent.id).await);
        let desired_children =
            try_sync_step!(self.prepare_managed_indexer_sync_plan(&parent, plan).await);
        let existing_children =
            try_sync_step!(self.services.integrations.indexer_configs.list(None).await)
                .into_iter()
                .filter(|candidate| {
                    candidate.managed_parent_config_id.as_deref() == Some(parent.id.as_str())
                })
                .collect::<Vec<_>>();
        let mut existing_by_key = existing_children
            .into_iter()
            .filter_map(|candidate| {
                candidate
                    .managed_child_key
                    .clone()
                    .map(|child_key| (child_key, candidate))
            })
            .collect::<HashMap<_, _>>();
        let mut routing_by_scope = try_sync_step!(self.load_indexer_routing_by_scope(actor).await);
        let mut result = IndexerConfigSyncResult {
            parent_config_id: parent.id.clone(),
            ..Default::default()
        };
        let managed_rate_limit_seconds = parent
            .provider_type
            .eq_ignore_ascii_case("prowlarr")
            .then_some(PROWLARR_MANAGED_CHILD_RATE_LIMIT_SECONDS);

        for desired in desired_children {
            if let Some(existing) = existing_by_key.remove(&desired.child_key) {
                let locally_disabled = managed_child_is_locally_disabled(
                    parent.managed_metadata_json.as_deref(),
                    &desired.child_key,
                );
                let managed_metadata_json = merge_managed_child_metadata(
                    existing.managed_metadata_json.as_deref(),
                    desired.managed_metadata_json.as_deref(),
                )
                .or_else(|| desired.managed_metadata_json.clone());
                let updated = try_sync_step!(
                    self.services
                        .integrations
                        .indexer_configs
                        .update(IndexerConfigUpdate {
                            id: existing.id.clone(),
                            name: Some(desired.name.clone()),
                            provider_type: Some(desired.provider_type.clone()),
                            derived_base_url: Some(desired.base_url.clone()),
                            rate_limit_seconds: if existing.rate_limit_seconds.is_none() {
                                managed_rate_limit_seconds
                            } else {
                                None
                            },
                            rate_limit_burst: None,
                            is_enabled: Some(desired.is_enabled && !locally_disabled),
                            enable_interactive_search: Some(desired.enable_interactive_search),
                            enable_auto_search: Some(desired.enable_auto_search),
                            proxy_config_id: Some(parent.proxy_config_id.clone()),
                            download_client_id: None,
                            seeding_profile_id: None,
                            managed_parent_config_id: Some(Some(parent.id.clone())),
                            managed_child_key: Some(Some(desired.child_key.clone())),
                            managed_metadata_json: Some(managed_metadata_json),
                            // Sync plans no longer fetch caps, so a plan without a
                            // snapshot must not clear the stored one — the background
                            // enrichment pass owns caps_snapshot_json refreshes.
                            caps_snapshot_json: desired.caps_snapshot_json.clone().map(Some),
                            config_json: Some(desired.config_json.clone()),
                        })
                        .await
                );
                if crate::indexer_search_identity(&existing, None)
                    != crate::indexer_search_identity(&updated, None)
                {
                    self.prune_indexer_search_learning_best_effort(
                        &updated.id,
                        "managed_indexer_search_change",
                    )
                    .await;
                }
                indexers_changed = true;
                apply_managed_child_routing(
                    &mut routing_by_scope,
                    &updated.id,
                    &desired.routing_by_scope,
                );
                result.updated_ids.push(updated.id);
            } else {
                let created = try_sync_step!(
                    self.services
                        .integrations
                        .indexer_configs
                        .create(IndexerConfig {
                            id: Id::new().0,
                            name: desired.name.clone(),
                            provider_type: desired.provider_type.clone(),
                            base_url: desired.base_url.clone(),
                            api_key_encrypted: None,
                            rate_limit_seconds: managed_rate_limit_seconds,
                            rate_limit_burst: None,
                            disabled_until: None,
                            is_enabled: desired.is_enabled
                                && !managed_child_is_locally_disabled(
                                    parent.managed_metadata_json.as_deref(),
                                    &desired.child_key,
                                ),
                            enable_interactive_search: desired.enable_interactive_search,
                            enable_auto_search: desired.enable_auto_search,
                            proxy_config_id: parent.proxy_config_id.clone(),
                            download_client_id: None,
                            seeding_profile_id: None,
                            managed_parent_config_id: Some(parent.id.clone()),
                            managed_child_key: Some(desired.child_key.clone()),
                            managed_metadata_json: desired.managed_metadata_json.clone(),
                            caps_snapshot_json: desired.caps_snapshot_json.clone(),
                            last_health_status: None,
                            last_error_message: None,
                            last_error_at: None,
                            config_json: Some(desired.config_json.clone()),
                            created_at: Utc::now(),
                            updated_at: Utc::now(),
                        })
                        .await
                );
                indexers_changed = true;
                apply_managed_child_routing(
                    &mut routing_by_scope,
                    &created.id,
                    &desired.routing_by_scope,
                );
                result.created_ids.push(created.id);
            }
        }

        for (_, obsolete) in existing_by_key {
            try_sync_step!(
                self.services
                    .integrations
                    .indexer_configs
                    .delete(&obsolete.id)
                    .await
            );
            indexers_changed = true;
            remove_indexer_routing_entries(&mut routing_by_scope, &obsolete.id);
            result.deleted_ids.push(obsolete.id);
        }

        try_sync_step!(
            self.save_indexer_routing_by_scope(actor, routing_by_scope)
                .await
        );
        if indexers_changed {
            self.publish_indexers_changed();
        }
        self.queue_managed_indexer_enrichment(actor, &parent.id);
        Ok(result)
    }
}
impl AppUseCase {
    async fn set_managed_child_indexer_enabled(
        &self,
        actor: &User,
        child: &IndexerConfig,
        requested_enabled: bool,
    ) -> AppResult<IndexerConfig> {
        let child_id = child.id.clone();
        let (updated, parent_id) = self
            .set_managed_child_local_disable(&child_id, !requested_enabled)
            .await?;

        if !requested_enabled {
            self.publish_indexers_changed();
            return Ok(updated);
        }

        if let Err(error) = self.sync_indexer_config(actor, &parent_id).await {
            if let Err(restore_error) = self.set_managed_child_local_disable(&child_id, true).await
            {
                tracing::error!(
                    child_id = %child_id,
                    error = %restore_error,
                    "failed to restore managed child local disable after sync failure"
                );
            } else {
                self.publish_indexers_changed();
            }
            return Err(error);
        }
        self.services
            .integrations
            .indexer_configs
            .get_by_id(&child_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("indexer config '{child_id}' not found")))
    }

    async fn set_managed_child_local_disable(
        &self,
        child_id: &str,
        locally_disabled: bool,
    ) -> AppResult<(IndexerConfig, String)> {
        let child_id = child_id.to_string();
        let (updated, parent_id) = {
            let _sync_guard = self
                .runtime
                .integrations
                .managed_indexer_sync_lock
                .clone()
                .lock_owned()
                .await;
            let current = self
                .services
                .integrations
                .indexer_configs
                .get_by_id(&child_id)
                .await?
                .ok_or_else(|| {
                    AppError::NotFound(format!("indexer config '{child_id}' not found"))
                })?;
            let parent_id = current.managed_parent_config_id.clone().ok_or_else(|| {
                AppError::Validation("managed child indexer requires a parent".into())
            })?;
            let child_key = current.managed_child_key.as_deref().ok_or_else(|| {
                AppError::Validation("managed child indexer requires a child key".into())
            })?;
            let parent = self
                .services
                .integrations
                .indexer_configs
                .get_by_id(&parent_id)
                .await?
                .ok_or_else(|| {
                    AppError::NotFound(format!("indexer config '{parent_id}' not found"))
                })?;
            let metadata = with_managed_child_local_disable(
                parent.managed_metadata_json.as_deref(),
                child_key,
                locally_disabled,
            )?;
            self.services
                .integrations
                .indexer_configs
                .update(IndexerConfigUpdate {
                    id: parent_id.clone(),
                    managed_metadata_json: Some(metadata),
                    ..Default::default()
                })
                .await?;
            let updated = self
                .services
                .integrations
                .indexer_configs
                .update(IndexerConfigUpdate {
                    id: child_id.clone(),
                    is_enabled: Some(false),
                    ..Default::default()
                })
                .await?;
            (updated, parent_id)
        };
        Ok((updated, parent_id))
    }

    async fn set_managed_child_indexers_enabled_state(
        &self,
        parent_config_id: &str,
        is_enabled: bool,
    ) -> AppResult<()> {
        let children = self
            .services
            .integrations
            .indexer_configs
            .list(None)
            .await?
            .into_iter()
            .filter(|candidate| {
                candidate.managed_parent_config_id.as_deref() == Some(parent_config_id)
                    && candidate.is_enabled != is_enabled
            })
            .collect::<Vec<_>>();

        for child in children {
            self.services
                .integrations
                .indexer_configs
                .update(IndexerConfigUpdate {
                    id: child.id,
                    is_enabled: Some(is_enabled),
                    ..Default::default()
                })
                .await?;
        }

        Ok(())
    }
}
impl AppUseCase {
    async fn load_indexer_routing_by_scope(
        &self,
        actor: &User,
    ) -> AppResult<HashMap<String, Vec<IndexerRoutingSettingsEntry>>> {
        let mut routing_by_scope = HashMap::new();
        for scope_id in MANAGED_INDEXER_SCOPE_IDS {
            routing_by_scope.insert(
                scope_id.to_string(),
                self.get_indexer_routing(actor, scope_id).await?,
            );
        }
        Ok(routing_by_scope)
    }
}
impl AppUseCase {
    async fn save_indexer_routing_by_scope(
        &self,
        actor: &User,
        mut routing_by_scope: HashMap<String, Vec<IndexerRoutingSettingsEntry>>,
    ) -> AppResult<()> {
        for scope_id in MANAGED_INDEXER_SCOPE_IDS {
            let entries = routing_by_scope.remove(*scope_id).unwrap_or_default();
            self.update_indexer_routing_without_sync_lock(actor, scope_id, entries)
                .await?;
        }
        Ok(())
    }
}
fn updated_managed_parent_requires_sync(
    existing: &IndexerConfig,
    updated_enabled_state: Option<bool>,
    provider_changed: bool,
    config_changed: bool,
    proxy_changed: bool,
) -> bool {
    if !existing.is_enabled && !matches!(updated_enabled_state, Some(true)) {
        return false;
    }

    provider_changed
        || config_changed
        || proxy_changed
        || (matches!(updated_enabled_state, Some(true)) && !existing.is_enabled)
        || (matches!(updated_enabled_state, Some(false)) && existing.is_enabled)
}
fn merge_tracked_download_background_work_state(
    tracked: &mut crate::tracked_downloads::TrackedDownload,
    finished: crate::tracked_downloads::TrackedDownload,
) {
    tracked.merge_background_work_state_from(finished);
}

fn protocol_families_from_capabilities(
    capabilities: &scryer_domain::IndexerProviderCapabilities,
) -> Option<Vec<&'static str>> {
    let mut families = Vec::new();
    for protocol in &capabilities.protocols {
        match protocol {
            scryer_domain::IndexerProtocolCapability::Usenet => families.push("usenet"),
            scryer_domain::IndexerProtocolCapability::Torrent => families.push("torrent"),
            scryer_domain::IndexerProtocolCapability::Mixed => {
                families.push("usenet");
                families.push("torrent");
            }
            scryer_domain::IndexerProtocolCapability::Unknown => return None,
        }
    }
    families.sort_unstable();
    families.dedup();
    (!families.is_empty()).then_some(families)
}

fn download_client_supports_protocol_families(
    client_type: &str,
    plugin_provider: Option<&Arc<dyn DownloadClientPluginProvider>>,
    required_families: &[&str],
) -> bool {
    let accepted_inputs = crate::accepted_inputs_for_client(client_type, plugin_provider);
    let supports_usenet = accepted_inputs.iter().any(|input| {
        matches!(
            input,
            DownloadSourceKind::NzbFile | DownloadSourceKind::NzbUrl
        )
    });
    let supports_torrent = accepted_inputs.iter().any(|input| {
        matches!(
            input,
            DownloadSourceKind::TorrentFile | DownloadSourceKind::MagnetUri
        )
    });
    required_families.iter().all(|family| match *family {
        "usenet" => supports_usenet,
        "torrent" => supports_torrent,
        _ => false,
    })
}

impl AppUseCase {
    fn indexer_download_mapping_families(&self, provider_type: &str) -> Option<Vec<&'static str>> {
        let capabilities = self
            .services
            .integrations
            .plugin_provider
            .available()?
            .capabilities_for_provider(provider_type);
        protocol_families_from_capabilities(&capabilities)
    }

    fn indexer_download_client_provider_compatibility(
        &self,
        provider_type: &str,
        is_managed_child: bool,
        clients: &[scryer_domain::DownloadClientConfig],
        plugin_provider: Option<&Arc<dyn DownloadClientPluginProvider>>,
    ) -> IndexerDownloadClientProviderCompatibility {
        let families = if provider_type.eq_ignore_ascii_case("prowlarr") && !is_managed_child {
            None
        } else {
            self.indexer_download_mapping_families(provider_type)
        };
        let compatible_client_ids = families
            .as_ref()
            .map(|required_families| {
                clients
                    .iter()
                    .filter(|client| {
                        download_client_supports_protocol_families(
                            &client.client_type,
                            plugin_provider,
                            required_families,
                        )
                    })
                    .map(|client| client.id.clone())
                    .collect()
            })
            .unwrap_or_default();
        IndexerDownloadClientProviderCompatibility {
            provider_type: provider_type.to_string(),
            protocol_families: families
                .clone()
                .unwrap_or_default()
                .into_iter()
                .map(str::to_string)
                .collect(),
            supports_mapping: families.is_some(),
            compatible_client_ids,
        }
    }

    pub(crate) fn validate_indexer_download_client_mapping(
        &self,
        indexer: &IndexerConfig,
        client: &scryer_domain::DownloadClientConfig,
    ) -> AppResult<()> {
        if indexer.provider_type.eq_ignore_ascii_case("prowlarr")
            && indexer.managed_parent_config_id.is_none()
        {
            return Err(AppError::Validation(
                "Prowlarr management parents cannot be mapped to a download client".into(),
            ));
        }

        let required_families = self
            .indexer_download_mapping_families(&indexer.provider_type)
            .ok_or_else(|| {
                AppError::Validation(format!(
                    "indexer provider '{}' does not declare a supported protocol family",
                    indexer.provider_type
                ))
            })?;
        let plugin_provider = self
            .services
            .integrations
            .download_client_plugin_provider
            .available();
        if !download_client_supports_protocol_families(
            &client.client_type,
            plugin_provider,
            &required_families,
        ) {
            return Err(AppError::Validation(format!(
                "download client '{}' does not support all protocol families required by indexer '{}'",
                client.name, indexer.name
            )));
        }
        Ok(())
    }

    pub async fn set_indexer_download_client_mapping(
        &self,
        actor: &User,
        indexer_id: &str,
        download_client_id: Option<&str>,
    ) -> AppResult<IndexerConfig> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let indexer_id = indexer_id.trim();
        if indexer_id.is_empty() {
            return Err(AppError::Validation("indexer id is required".into()));
        }
        let normalized_client_id = download_client_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let existing = self
            .services
            .integrations
            .indexer_configs
            .get_by_id(indexer_id)
            .await?
            .ok_or_else(|| {
                AppError::NotFound(format!("indexer config '{indexer_id}' not found"))
            })?;
        if let Some(client_id) = normalized_client_id.as_deref() {
            let client = self
                .services
                .integrations
                .download_client_configs
                .get_by_id(client_id)
                .await?
                .ok_or_else(|| {
                    AppError::NotFound(format!("download client config '{client_id}' not found"))
                })?;
            self.validate_indexer_download_client_mapping(&existing, &client)?;
        }
        if existing.download_client_id == normalized_client_id {
            return Ok(existing);
        }

        let updated = self
            .services
            .integrations
            .indexer_configs
            .set_download_client_mapping(indexer_id, normalized_client_id)
            .await?;
        self.publish_indexers_changed();
        self.emit_configuration_changed_event(
            actor,
            "indexer",
            Some(updated.id.clone()),
            scryer_domain::ConfigurationChangeAction::Updated,
        )
        .await;
        Ok(updated)
    }

    pub async fn get_indexer_download_client_mapping_catalog(
        &self,
        actor: &User,
    ) -> AppResult<IndexerDownloadClientMappingCatalog> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let indexers = self
            .services
            .integrations
            .indexer_configs
            .list(None)
            .await?;
        let clients = self
            .services
            .integrations
            .download_client_configs
            .list(None)
            .await?;
        let plugin_provider = self
            .services
            .integrations
            .download_client_plugin_provider
            .available();
        let client_payloads = clients
            .iter()
            .map(|client| IndexerDownloadClientMappingClient {
                id: client.id.clone(),
                name: client.name.clone(),
                client_type: client.client_type.clone(),
                is_enabled: client.is_enabled,
                health_status: client.status.as_str().to_string(),
            })
            .collect();
        let indexer_payloads = indexers
            .iter()
            .map(|indexer| {
                let compatibility = self.indexer_download_client_provider_compatibility(
                    &indexer.provider_type,
                    indexer.managed_parent_config_id.is_some(),
                    &clients,
                    plugin_provider,
                );
                IndexerDownloadClientMappingIndexer {
                    id: indexer.id.clone(),
                    name: indexer.name.clone(),
                    download_client_id: indexer.download_client_id.clone(),
                    supports_mapping: compatibility.supports_mapping,
                    protocol_families: compatibility.protocol_families,
                    compatible_client_ids: compatibility.compatible_client_ids,
                }
            })
            .collect();
        let mut provider_types = self
            .available_indexer_provider_types()
            .into_iter()
            .map(|(provider_type, _, _, _)| provider_type)
            .chain(indexers.iter().map(|indexer| indexer.provider_type.clone()))
            .collect::<Vec<_>>();
        provider_types.sort_unstable();
        provider_types.dedup();
        let provider_compatibility = provider_types
            .into_iter()
            .map(|provider_type| {
                self.indexer_download_client_provider_compatibility(
                    &provider_type,
                    false,
                    &clients,
                    plugin_provider,
                )
            })
            .collect();
        Ok(IndexerDownloadClientMappingCatalog {
            clients: client_payloads,
            indexers: indexer_payloads,
            provider_compatibility,
        })
    }
}

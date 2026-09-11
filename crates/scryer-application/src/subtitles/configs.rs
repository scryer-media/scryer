use chrono::Utc;
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::{
    AppError, AppResult, AppUseCase, Id, SubtitleProviderClient, SubtitleProviderConfig,
    SubtitleProviderConfigUpdate, SubtitleProviderValidationResult, User,
};
use scryer_domain::PluginHostBindingId;

const OPENSUBTITLES_PROVIDER_TYPE: &str = "opensubtitles";
const OPENSUBTITLES_PROVIDER_NAME: &str = "OpenSubtitles";
const LEGACY_OPENSUBTITLES_API_KEY_KEY: &str = "subtitles.opensubtitles_api_key";
const LEGACY_OPENSUBTITLES_USERNAME_KEY: &str = "subtitles.opensubtitles_username";
const LEGACY_OPENSUBTITLES_PASSWORD_KEY: &str = "subtitles.opensubtitles_password";

impl AppUseCase {
    pub async fn list_subtitle_provider_configs(
        &self,
        actor: &User,
    ) -> AppResult<Vec<SubtitleProviderConfig>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        self.subtitle_provider_configs_repo()?.list(None).await
    }

    pub async fn get_subtitle_provider_config(
        &self,
        actor: &User,
        id: &str,
    ) -> AppResult<Option<SubtitleProviderConfig>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        self.subtitle_provider_configs_repo()?.get_by_id(id).await
    }

    pub async fn create_subtitle_provider_config(
        &self,
        actor: &User,
        name: String,
        provider_type: String,
        config_json: String,
        enabled_facets: Option<Vec<String>>,
        is_enabled: bool,
    ) -> AppResult<SubtitleProviderConfig> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        let name = name.trim().to_string();
        if name.is_empty() {
            return Err(AppError::Validation(
                "subtitle provider config name must not be empty".to_string(),
            ));
        }

        let provider_type = provider_type.trim().to_ascii_lowercase();
        if provider_type.is_empty() {
            return Err(AppError::Validation(
                "subtitle provider type must not be empty".to_string(),
            ));
        }

        let now = Utc::now();
        let enabled_facets = match enabled_facets {
            Some(facets) => normalize_subtitle_provider_facets(facets),
            None => self.subtitle_provider_recommended_facets(&provider_type),
        };
        self.subtitle_provider_configs_repo()?
            .create(SubtitleProviderConfig {
                id: Id::new().0,
                name,
                provider_type,
                config_json,
                enabled_facets,
                is_enabled,
                last_health_status: None,
                last_error: None,
                last_error_at: None,
                disabled_until: None,
                created_at: now,
                updated_at: now,
            })
            .await
    }

    pub async fn update_subtitle_provider_config(
        &self,
        actor: &User,
        mut update: SubtitleProviderConfigUpdate,
    ) -> AppResult<SubtitleProviderConfig> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        if !update.has_changes() {
            return Err(AppError::Validation(
                "at least one subtitle provider config field must be provided".to_string(),
            ));
        }
        if update.config_json.is_some() || update.provider_type.is_some() {
            let existing = self
                .subtitle_provider_configs_repo()?
                .get_by_id(&update.id)
                .await?
                .ok_or_else(|| {
                    AppError::NotFound(format!("subtitle provider config {}", update.id))
                })?;
            let provider_type = update
                .provider_type
                .as_deref()
                .unwrap_or(existing.provider_type.as_str());
            if let Some(config_json) = update.config_json.as_ref() {
                update.config_json = Some(self.merge_subtitle_provider_config_values(
                    provider_type,
                    config_json,
                    Some(existing.config_json.as_str()),
                )?);
            }
        }
        if let Some(enabled_facets) = update.enabled_facets.take() {
            update.enabled_facets = Some(normalize_subtitle_provider_facets(enabled_facets));
        }
        self.subtitle_provider_configs_repo()?.update(update).await
    }

    pub async fn delete_subtitle_provider_config(&self, actor: &User, id: &str) -> AppResult<()> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        self.subtitle_provider_configs_repo()?.delete(id).await
    }

    pub fn available_subtitle_provider_types(&self) -> Vec<String> {
        self.services
            .integrations
            .subtitle_plugin_provider
            .available()
            .map(|provider| {
                provider
                    .available_provider_types()
                    .into_iter()
                    .filter(|provider_type| {
                        provider.supports_catalog_search_for_provider(provider_type)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn subtitle_provider_config_fields(
        &self,
        provider_type: &str,
    ) -> Vec<scryer_domain::ConfigFieldDef> {
        self.services
            .integrations
            .subtitle_plugin_provider
            .available()
            .map(|provider| provider.config_fields_for_provider(provider_type))
            .unwrap_or_default()
    }

    pub fn subtitle_provider_name(&self, provider_type: &str) -> Option<String> {
        self.services
            .integrations
            .subtitle_plugin_provider
            .available()
            .and_then(|provider| provider.plugin_name_for_provider(provider_type))
    }

    pub fn subtitle_provider_recommended_facets(&self, provider_type: &str) -> Vec<String> {
        self.services
            .integrations
            .subtitle_plugin_provider
            .available()
            .map(|provider| {
                normalize_subtitle_provider_facets(
                    provider.recommended_facets_for_provider(provider_type),
                )
            })
            .unwrap_or_default()
    }

    pub async fn test_subtitle_provider_connection(
        &self,
        actor: &User,
        id: Option<&str>,
        provider_type: String,
        config_json: String,
    ) -> AppResult<SubtitleProviderValidationResult> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        let normalized_provider_type = provider_type.trim().to_ascii_lowercase();
        self.ensure_provider_plugin_not_blocked(&normalized_provider_type)
            .await?;

        let config = if let Some(id) = id {
            let mut config = self
                .subtitle_provider_configs_repo()?
                .get_by_id(id)
                .await?
                .ok_or_else(|| AppError::NotFound(format!("subtitle provider config {id}")))?;
            if !normalized_provider_type.is_empty() {
                config.provider_type = normalized_provider_type.clone();
            }
            config.config_json = self.merge_subtitle_provider_config_values(
                config.provider_type.as_str(),
                &config_json,
                Some(config.config_json.as_str()),
            )?;
            config
        } else {
            SubtitleProviderConfig {
                id: "transient".to_string(),
                name: "transient".to_string(),
                provider_type: normalized_provider_type,
                config_json,
                enabled_facets: Vec::new(),
                is_enabled: true,
                last_health_status: None,
                last_error: None,
                last_error_at: None,
                disabled_until: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            }
        };

        let client = self.subtitle_provider_client_for_config(&config).await?;
        let result = client.validate_connection().await?;
        if id.is_some() {
            self.record_subtitle_provider_validation_result(&config.id, &result)
                .await;
        }
        Ok(result)
    }

    async fn record_subtitle_provider_validation_result(
        &self,
        config_id: &str,
        result: &SubtitleProviderValidationResult,
    ) {
        let status = format!("Connection test: {}", result.status);
        let is_valid = result.status.eq_ignore_ascii_case("valid");
        let update = SubtitleProviderConfigUpdate {
            id: config_id.to_string(),
            last_health_status: Some(status),
            last_error: Some(if is_valid {
                None
            } else {
                result
                    .message
                    .clone()
                    .or_else(|| Some(result.status.clone()))
            }),
            last_error_at: Some(if is_valid { None } else { Some(Utc::now()) }),
            ..Default::default()
        };
        let Ok(repo) = self.subtitle_provider_configs_repo() else {
            return;
        };
        if let Err(error) = repo.update(update).await {
            tracing::warn!(
                subtitle_provider_config_id = config_id,
                error = %error,
                "failed to update subtitle provider validation status"
            );
        }
    }

    pub async fn subtitle_provider_client_for_config(
        &self,
        config: &SubtitleProviderConfig,
    ) -> AppResult<Arc<dyn SubtitleProviderClient>> {
        let bindings = self.subtitle_host_bindings().await?;
        self.services
            .integrations
            .subtitle_plugin_provider
            .available()
            .and_then(|provider| provider.client_for_config(config, &bindings))
            .ok_or_else(|| {
                AppError::NotFound(format!(
                    "no subtitle plugin for provider type '{}'",
                    config.provider_type
                ))
            })
    }

    fn merge_subtitle_provider_config_values(
        &self,
        provider_type: &str,
        config_json: &str,
        existing_config_json: Option<&str>,
    ) -> AppResult<String> {
        let mut secret_keys = self
            .subtitle_provider_config_fields(provider_type)
            .into_iter()
            .filter(|field| matches!(field.field_type, scryer_domain::ConfigFieldType::Password))
            .map(|field| field.key)
            .collect::<HashSet<_>>();

        let mut incoming = parse_config_object(config_json)?;
        let Some(existing_config_json) = existing_config_json else {
            return serde_json::to_string(&incoming).map_err(|error| {
                AppError::Validation(format!("invalid subtitle provider config_json: {error}"))
            });
        };
        let existing = parse_config_object(existing_config_json)?;

        // Read-side GraphQL intentionally does not expose config_json. Preserve
        // omitted existing keys on updates so callers can edit one field without
        // needing the full decrypted config payload.
        for (key, value) in &existing {
            incoming.entry(key.clone()).or_insert_with(|| value.clone());
        }

        for key in existing.keys().chain(incoming.keys()) {
            if looks_like_secret_config_key(key) {
                secret_keys.insert(key.clone());
            }
        }

        for key in secret_keys {
            let incoming_is_missing = !incoming.contains_key(&key);
            if incoming_is_missing
                && let Some(existing_value) = existing.get(&key).filter(|value| !value.is_null())
            {
                incoming.insert(key, existing_value.clone());
            }
        }

        serde_json::to_string(&incoming).map_err(|error| {
            AppError::Validation(format!("invalid subtitle provider config_json: {error}"))
        })
    }

    pub async fn subtitle_host_bindings(&self) -> AppResult<HashMap<PluginHostBindingId, String>> {
        let mut bindings = HashMap::new();

        if let Some(api_key) = self
            .read_setting_string_value(LEGACY_OPENSUBTITLES_API_KEY_KEY, None)
            .await?
            .and_then(normalize_non_empty)
        {
            bindings.insert(PluginHostBindingId::SmgOpenSubtitlesApiKey, api_key);
        }

        Ok(bindings)
    }

    fn subtitle_provider_configs_repo(
        &self,
    ) -> AppResult<&Arc<dyn crate::SubtitleProviderConfigRepository>> {
        self.services
            .integrations
            .subtitle_provider_configs
            .available()
            .ok_or_else(|| {
                AppError::Repository(
                    "subtitle provider config repository is not configured".to_string(),
                )
            })
    }

    pub async fn migrate_legacy_opensubtitles_provider_config(
        &self,
    ) -> AppResult<Option<SubtitleProviderConfig>> {
        let Some(_) = self
            .services
            .integrations
            .subtitle_provider_configs
            .available()
        else {
            return Ok(None);
        };

        let settings = self.subtitle_settings().await?;
        let Some(config_json) = self.legacy_opensubtitles_config_json(true).await? else {
            return Ok(None);
        };

        let repo = self.subtitle_provider_configs_repo()?;
        let existing = repo
            .list(Some(OPENSUBTITLES_PROVIDER_TYPE.to_string()))
            .await?;
        if !existing.is_empty() {
            return Ok(None);
        }

        repo.create(SubtitleProviderConfig {
            id: Id::new().0,
            name: OPENSUBTITLES_PROVIDER_NAME.to_string(),
            provider_type: OPENSUBTITLES_PROVIDER_TYPE.to_string(),
            config_json,
            enabled_facets: vec!["movie".to_string(), "series".to_string()],
            is_enabled: settings.enabled,
            last_health_status: None,
            last_error: None,
            last_error_at: None,
            disabled_until: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .map(Some)
    }

    async fn legacy_opensubtitles_config_json(
        &self,
        require_username: bool,
    ) -> AppResult<Option<String>> {
        let username = self
            .read_setting_string_value(LEGACY_OPENSUBTITLES_USERNAME_KEY, None)
            .await?
            .and_then(normalize_non_empty)
            .unwrap_or_default();
        if require_username && username.is_empty() {
            return Ok(None);
        }

        let password = self
            .read_setting_string_value(LEGACY_OPENSUBTITLES_PASSWORD_KEY, None)
            .await?
            .and_then(normalize_non_empty)
            .unwrap_or_default();

        serde_json::to_string(&json!({
            "username": username,
            "password": password,
            "enable_hash_lookup": true,
        }))
        .map(Some)
        .map_err(|error| AppError::Repository(error.to_string()))
    }
}

fn normalize_subtitle_provider_facets(raw: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    raw.into_iter()
        .filter_map(|facet| {
            let normalized = facet.trim().to_ascii_lowercase();
            if matches!(normalized.as_str(), "movie" | "series" | "anime")
                && seen.insert(normalized.clone())
            {
                Some(normalized)
            } else {
                None
            }
        })
        .collect()
}

fn looks_like_secret_config_key(key: &str) -> bool {
    let normalized = key.trim().to_ascii_lowercase();
    normalized.contains("password")
        || normalized.contains("secret")
        || normalized.contains("token")
        || normalized == "api_key"
        || normalized == "apikey"
        || normalized.contains("api_key")
}

fn parse_config_object(config_json: &str) -> AppResult<Map<String, Value>> {
    let trimmed = config_json.trim();
    if trimmed.is_empty() {
        return Ok(Map::new());
    }

    match serde_json::from_str::<Value>(trimmed) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err(AppError::Validation(
            "subtitle provider config_json must be a JSON object".to_string(),
        )),
        Err(error) => Err(AppError::Validation(format!(
            "invalid subtitle provider config_json: {error}"
        ))),
    }
}

fn normalize_non_empty(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

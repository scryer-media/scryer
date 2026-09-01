use super::*;
use chrono::Utc;

impl AppUseCase {
    /// Test an indexer connection by performing a minimal search through the plugin system.
    /// This validates: plugin availability, HTTP connectivity, API key, response parsing.
    pub async fn test_indexer_connection(
        &self,
        actor: &User,
        provider_type: &str,
        config_json: Option<&str>,
        indexer_id: Option<&str>,
        indexer_proxy_config_id_override: Option<Option<&str>>,
    ) -> AppResult<()> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let fields = self.indexer_config_fields_for_provider_type(provider_type)?;
        let persisted_config = if let Some(indexer_id) = indexer_id {
            self.services
                .integrations
                .indexer_configs
                .get_by_id(indexer_id)
                .await?
        } else {
            None
        };
        let persisted_config_json = persisted_config
            .as_ref()
            .filter(|config| {
                persisted_indexer_config_can_restore_secrets(
                    &fields,
                    provider_type,
                    config.provider_type.as_str(),
                    config_json,
                    config.config_json.as_deref(),
                )
            })
            .and_then(|config| config.config_json.as_deref());
        let normalized_config_json = crate::app_usecase_integration::normalize_indexer_config_json(
            &fields,
            config_json,
            persisted_config_json,
        )?;
        let provider = self
            .services
            .integrations
            .plugin_provider
            .available()
            .ok_or_else(|| AppError::Repository("indexer provider not available".into()))?;
        provider.validate_config_for_provider(provider_type, &normalized_config_json)?;
        let base_url = crate::app_usecase_integration::derive_indexer_base_url_from_config_fields(
            &fields,
            Some(&normalized_config_json),
        )?;
        let validated_base_url = validate_test_flight_url(&base_url)?;
        preflight_test_flight_url(&validated_base_url).await?;

        let now = Utc::now();

        // Build a temporary IndexerConfig to get a client from the provider.
        // Reject obviously invalid API keys (e.g. masked placeholders from
        // Sonarr/Radarr import that were stored before the masking fix).
        let parsed_config: serde_json::Value = serde_json::from_str(&normalized_config_json)
            .map_err(|error| AppError::Validation(error.to_string()))?;
        for field in fields
            .iter()
            .filter(|field| field.field_type == scryer_domain::ConfigFieldType::Password)
        {
            if let Some(trimmed) = parsed_config
                .get(&field.key)
                .and_then(|value| value.as_str())
                .map(str::trim)
                && trimmed.chars().all(|c| c == '*')
                && !trimmed.is_empty()
            {
                return Err(AppError::Validation(
                    "API key appears to be a masked placeholder — enter the real key".into(),
                ));
            }
        }

        let indexer_proxy_config_id = match indexer_proxy_config_id_override {
            Some(Some(id)) => Some(id.to_string()),
            Some(None) => None,
            None => persisted_config
                .as_ref()
                .and_then(|config| config.indexer_proxy_config_id.clone()),
        };
        let indexer_proxy_config = if let Some(indexer_proxy_config_id) =
            indexer_proxy_config_id.as_deref()
        {
            let proxy_config = self
                .services
                .integrations
                .indexer_proxy_configs
                .get_by_id(indexer_proxy_config_id)
                .await?
                .ok_or_else(|| {
                    AppError::Validation("Indexer proxy configuration was not found.".to_string())
                })?;
            if !proxy_config.is_enabled {
                return Err(AppError::Validation(
                    "Indexer proxy is disabled for this indexer.".to_string(),
                ));
            }
            Some(proxy_config)
        } else {
            None
        };

        // The probe deliberately runs under a synthetic id: it must exercise the
        // submitted configuration, not whatever is stored. Nothing may persist
        // against that id — `indexer_error_history_is_persistable` is what the
        // capture paths ask before writing error history, so a failed probe
        // reports its own error instead of a foreign-key failure behind it.
        let temp_config = IndexerConfig {
            id: crate::CONNECTION_TEST_INDEXER_ID.to_string(),
            name: "Test Connection".to_string(),
            provider_type: provider_type.to_string(),
            base_url,
            api_key_encrypted: None,
            rate_limit_seconds: None,
            rate_limit_burst: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            disabled_until: None,
            indexer_proxy_config_id,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            config_json: Some(normalized_config_json),
            created_at: now,
            updated_at: now,
        };

        let management_capabilities = provider.management_capabilities_for_provider(provider_type);
        if management_capabilities.supports_validate_config
            || management_capabilities.supports_managed_children_sync
        {
            let client = provider
                .management_client_for_provider(&temp_config)
                .ok_or_else(|| {
                    AppError::Validation(format!(
                        "no indexer management client available for provider type '{provider_type}'"
                    ))
                })?;
            if management_capabilities.supports_validate_config {
                let result = client.validate_connection().await?;
                validate_indexer_connection_result(result)?;
            }
            if let Some(config) = persisted_config.as_ref() {
                self.services
                    .integrations
                    .indexer_configs
                    .clear_last_error(&config.id)
                    .await?;
                self.publish_indexers_changed();
            }
            return Ok(());
        }

        let client = provider
            .client_for_provider_with_proxy(&temp_config, indexer_proxy_config.as_ref())
            .ok_or_else(|| {
                AppError::Validation(format!(
                    "no indexer provider available for provider type '{provider_type}'"
                ))
            })?;
        let capabilities = provider.capabilities_for_provider(provider_type);
        let (query, ids, facet) = build_connection_test_search_request(&capabilities);

        // Perform a real search request to validate the full pipeline.
        client
            .search(
                query,
                ids,
                None,
                facet,
                None,
                None,
                None,
                SearchMode::Interactive,
                IndexerErrorOperation::ConnectionTest,
                None,
                None,
                None,
                vec![],
                None,
                tokio_util::sync::CancellationToken::new(),
            )
            .await
            .map_err(map_indexer_connection_test_error)?;

        let caps_refresh_available = self
            .services
            .integrations
            .indexer_caps_refresher
            .available()
            .is_some();
        let caps_snapshot = self
            .fetch_caps_snapshot_json_for_config(&temp_config)
            .await
            .map_err(map_indexer_connection_test_error)?;
        if caps_refresh_available && temp_config.is_direct_nab() && caps_snapshot.is_none() {
            return Err(AppError::Validation(
                "indexer connection test did not return a valid Newznab caps document".to_string(),
            ));
        }

        if let Some(config) = persisted_config.as_ref() {
            self.services
                .integrations
                .indexer_configs
                .clear_last_error(&config.id)
                .await?;
            self.publish_indexers_changed();
        }

        Ok(())
    }

    pub async fn preview_managed_indexer_children(
        &self,
        actor: &User,
        provider_type: &str,
        config_json: Option<&str>,
    ) -> AppResult<(crate::IndexerValidationResult, crate::IndexerSyncPlan)> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let fields = self.indexer_config_fields_for_provider_type(provider_type)?;
        let normalized_config_json = crate::app_usecase_integration::normalize_indexer_config_json(
            &fields,
            config_json,
            None,
        )?;
        let base_url = crate::app_usecase_integration::derive_indexer_base_url_from_config_fields(
            &fields,
            Some(&normalized_config_json),
        )?;
        let validated_base_url = validate_test_flight_url(&base_url)?;
        preflight_test_flight_url(&validated_base_url).await?;

        let provider = self
            .services
            .integrations
            .plugin_provider
            .available()
            .ok_or_else(|| AppError::Repository("indexer provider not available".into()))?;
        let management_capabilities = provider.management_capabilities_for_provider(provider_type);
        if !management_capabilities.supports_managed_children_sync {
            return Err(AppError::Validation(format!(
                "provider type '{provider_type}' does not support managed child sync"
            )));
        }

        let parsed_config: serde_json::Value = serde_json::from_str(&normalized_config_json)
            .map_err(|error| AppError::Validation(error.to_string()))?;
        for field in fields
            .iter()
            .filter(|field| field.field_type == scryer_domain::ConfigFieldType::Password)
        {
            if let Some(trimmed) = parsed_config
                .get(&field.key)
                .and_then(|value| value.as_str())
                .map(str::trim)
                && trimmed.chars().all(|c| c == '*')
                && !trimmed.is_empty()
            {
                return Err(AppError::Validation(
                    "API key appears to be a masked placeholder — enter the real key".into(),
                ));
            }
        }

        let now = Utc::now();
        let temp_config = IndexerConfig {
            id: "preview-managed-sync".to_string(),
            name: "Preview Managed Sync".to_string(),
            provider_type: provider_type.to_string(),
            base_url,
            api_key_encrypted: None,
            rate_limit_seconds: None,
            rate_limit_burst: None,
            is_enabled: true,
            enable_interactive_search: false,
            enable_auto_search: false,
            disabled_until: None,
            indexer_proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            config_json: Some(normalized_config_json),
            created_at: now,
            updated_at: now,
        };

        let client = provider
            .management_client_for_provider(&temp_config)
            .ok_or_else(|| {
                AppError::Validation(format!(
                    "no indexer management client available for provider type '{provider_type}'"
                ))
            })?;
        let validation = if management_capabilities.supports_validate_config {
            client.validate_connection().await?
        } else {
            crate::IndexerValidationResult {
                status: "valid".to_string(),
                message: None,
                retry_after_seconds: None,
            }
        };
        validate_indexer_connection_result(validation.clone())?;
        let plan = client.preview_sync_plan("preview-managed-sync").await?;

        Ok((validation, plan))
    }
}

fn validate_indexer_connection_result(result: crate::IndexerValidationResult) -> AppResult<()> {
    if result.status.eq_ignore_ascii_case("valid") {
        return Ok(());
    }

    let message = result.message.unwrap_or(result.status.clone());
    if matches!(
        result.status.as_str(),
        "invalid_config" | "auth_failed" | "missing_host_binding"
    ) {
        return Err(AppError::Validation(message));
    }

    Err(AppError::Repository(format!(
        "indexer connection test failed: {message}"
    )))
}

fn map_indexer_connection_test_error(error: AppError) -> AppError {
    match error {
        AppError::Validation(_) => error,
        error => {
            let message = error.to_string();
            if let Some(user_message) = known_newznab_error_message(&message) {
                AppError::Validation(user_message)
            } else {
                AppError::Repository(format!("indexer connection test failed: {message}"))
            }
        }
    }
}

#[derive(Clone, Copy)]
struct NewznabApiError {
    code: u16,
    user_message: &'static str,
}

const KNOWN_NEWZNAB_API_ERRORS: &[NewznabApiError] = &[
    NewznabApiError {
        code: 100,
        user_message: "Invalid API Key",
    },
    NewznabApiError {
        code: 101,
        user_message: "Account suspended",
    },
    NewznabApiError {
        code: 102,
        user_message: "Insufficient privileges",
    },
    NewznabApiError {
        code: 103,
        user_message: "Registration denied",
    },
    NewznabApiError {
        code: 104,
        user_message: "Registrations are closed",
    },
    NewznabApiError {
        code: 105,
        user_message: "Invalid registration",
    },
    NewznabApiError {
        code: 106,
        user_message: "Invalid registration email address",
    },
    NewznabApiError {
        code: 107,
        user_message: "Registration failed",
    },
    NewznabApiError {
        code: 200,
        user_message: "Missing parameter",
    },
    NewznabApiError {
        code: 201,
        user_message: "Incorrect parameter",
    },
    NewznabApiError {
        code: 202,
        user_message: "No such function",
    },
    NewznabApiError {
        code: 203,
        user_message: "Function not available",
    },
    NewznabApiError {
        code: 300,
        user_message: "No such item",
    },
    NewznabApiError {
        code: 500,
        user_message: "Request limit reached",
    },
    NewznabApiError {
        code: 501,
        user_message: "Download limit reached",
    },
    NewznabApiError {
        code: 900,
        user_message: "Unknown Newznab error",
    },
    NewznabApiError {
        code: 910,
        user_message: "Newznab API disabled",
    },
];

fn known_newznab_error_message(message: &str) -> Option<String> {
    if let Some(classified) = crate::classify_newznab_error_message(message) {
        return Some(classified.message.to_string());
    }

    let code = extract_newznab_error_code(message)?;
    let known_error = KNOWN_NEWZNAB_API_ERRORS
        .iter()
        .find(|error| error.code == code)?;
    let provider_message = extract_newznab_provider_message(message, code);

    Some(match (known_error.code, provider_message) {
        (100, Some(provider_message)) if mentions_api_key(&provider_message) => provider_message,
        (_, Some(provider_message)) if !provider_message.is_empty() => provider_message,
        _ => known_error.user_message.to_string(),
    })
}

fn extract_newznab_error_code(message: &str) -> Option<u16> {
    let lower = message.to_ascii_lowercase();
    let marker = lower.find("newznab")?;
    let after_marker = &message[marker..];
    let lower_after_marker = &lower[marker..];
    let error_marker = lower_after_marker.find("error")?;
    let after_error = &after_marker[error_marker + "error".len()..];

    after_error
        .split(|c: char| !c.is_ascii_digit())
        .find(|part| !part.is_empty())
        .and_then(|part| part.parse::<u16>().ok())
}

fn extract_newznab_provider_message(message: &str, code: u16) -> Option<String> {
    let code_text = code.to_string();
    let code_index = message.find(&code_text)?;
    message[code_index + code_text.len()..]
        .trim_start_matches(|c: char| c.is_whitespace() || matches!(c, ':' | '-' | '.' | ')'))
        .split(':')
        .next()
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(ToOwned::to_owned)
}

fn mentions_api_key(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    normalized.contains("api key") || normalized.contains("apikey")
}

fn validate_test_flight_url(raw: &str) -> AppResult<url::Url> {
    let url = url::Url::parse(raw)
        .map_err(|error| AppError::Validation(format!("invalid base URL: {error}")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(AppError::Validation(
            "base URL must use http or https".into(),
        ));
    }
    if url.host_str().is_none() {
        return Err(AppError::Validation("base URL must include a host".into()));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(AppError::Validation(
            "base URL must not include embedded credentials".into(),
        ));
    }
    Ok(url)
}

fn persisted_indexer_config_can_restore_secrets(
    fields: &[scryer_domain::ConfigFieldDef],
    requested_provider_type: &str,
    persisted_provider_type: &str,
    requested_config_json: Option<&str>,
    persisted_config_json: Option<&str>,
) -> bool {
    if !requested_provider_type
        .trim()
        .eq_ignore_ascii_case(persisted_provider_type.trim())
    {
        return false;
    }

    let Some(requested_origin) =
        indexer_connection_origin_from_config(fields, requested_config_json)
    else {
        return true;
    };
    let Some(persisted_origin) =
        indexer_connection_origin_from_config(fields, persisted_config_json)
    else {
        return false;
    };

    requested_origin == persisted_origin
}

fn indexer_connection_origin_from_config(
    fields: &[scryer_domain::ConfigFieldDef],
    config_json: Option<&str>,
) -> Option<String> {
    let raw = config_connection_url_value(fields, config_json)?;
    let base_url = if raw.field_key_contains_feed_or_rss {
        url::Url::parse(&raw.value)
            .map(|url| url.origin().ascii_serialization())
            .unwrap_or(raw.value)
    } else {
        raw.value
    };
    let url = validate_test_flight_url(&base_url).ok()?;
    Some(url.origin().ascii_serialization())
}

struct IndexerConnectionUrlValue {
    value: String,
    field_key_contains_feed_or_rss: bool,
}

fn config_connection_url_value(
    fields: &[scryer_domain::ConfigFieldDef],
    config_json: Option<&str>,
) -> Option<IndexerConnectionUrlValue> {
    let raw = config_json?;
    let object = serde_json::from_str::<serde_json::Value>(raw)
        .ok()?
        .as_object()
        .cloned()?;
    let field = fields
        .iter()
        .find(|field| field.role == Some(scryer_domain::ConfigFieldRole::ConnectionUrl))?;
    let value = object
        .get(&field.key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_string();

    Some(IndexerConnectionUrlValue {
        value,
        field_key_contains_feed_or_rss: field.key.contains("feed") || field.key.contains("rss"),
    })
}

fn format_preflight_transport_error(url: &url::Url, origin: &str, error: &str) -> String {
    let mut message =
        format!("base URL preflight failed before sending credentials to {origin}: {error}");
    if url.scheme().eq_ignore_ascii_case("https") {
        message.push_str(
            ". If this service is not configured for TLS, try http:// instead of https://.",
        );
    }
    message
}

#[cfg(not(test))]
async fn preflight_test_flight_url(url: &url::Url) -> AppResult<()> {
    let origin = url.origin().ascii_serialization();
    let client = scryer_outbound_http::indexer_reqwest_client();
    let outbound_http = scryer_outbound_http::OutboundHttpClient::new(
        client,
        scryer_outbound_http::RateLimitRegistry::new(),
    );

    outbound_http
        .send(
            scryer_outbound_http::RequestPolicy::no_retry("indexer_preflight", "head_origin"),
            || outbound_http.client().head(&origin),
        )
        .await
        .map_err(|error| {
            AppError::Validation(format_preflight_transport_error(
                url,
                &origin,
                &error.to_string(),
            ))
        })?;

    Ok(())
}

#[cfg(test)]
async fn preflight_test_flight_url(_url: &url::Url) -> AppResult<()> {
    Ok(())
}

fn build_connection_test_search_request(
    capabilities: &scryer_domain::IndexerProviderCapabilities,
) -> (
    String,
    std::collections::HashMap<String, String>,
    Option<String>,
) {
    if capabilities.query_param.is_some() || capabilities.search {
        return (
            "scryer connection test".to_string(),
            std::collections::HashMap::new(),
            None,
        );
    }

    let mut supported_facets: Vec<_> = capabilities.supported_ids.iter().collect();
    supported_facets.sort_by_key(|(left, _)| *left);

    for (facet, id_types) in supported_facets {
        if let Some(id_type) = id_types.iter().find(|id_type| !id_type.is_empty()) {
            return (
                String::new(),
                std::collections::HashMap::from([(
                    id_type.clone(),
                    connection_test_id_value(id_type),
                )]),
                Some(facet.clone()),
            );
        }
    }

    (String::new(), std::collections::HashMap::new(), None)
}

fn connection_test_id_value(id_type: &str) -> String {
    match id_type {
        "imdb_id" => "tt0000001".to_string(),
        _ => "1".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NullSettingsRepository;
    use crate::null_repositories::test_nulls::{
        NullDownloadClient, NullDownloadClientConfigRepository, NullIndexerClient,
        NullQualityProfileRepository, NullReleaseAttemptRepository, NullShowRepository,
        NullTitleRepository, NullUserRepository,
    };
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    use std::time::Duration;
    use tokio::sync::{Mutex, Semaphore};

    #[test]
    fn known_newznab_errors_use_catalog_messages() {
        assert_eq!(
            known_newznab_error_message(
                "plugin scryer_indexer_search() failed: Newznab API error 101: Account suspended",
            )
            .as_deref(),
            Some("Account suspended")
        );
        assert_eq!(
            known_newznab_error_message("Newznab API error 203").as_deref(),
            Some("Function not available")
        );
    }

    #[test]
    fn unknown_newznab_errors_are_not_user_facing() {
        assert_eq!(
            known_newznab_error_message("Newznab API error 777: Provider-specific oddity"),
            None
        );
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct RecordedSearchCall {
        query: String,
        ids: HashMap<String, String>,
        facet: Option<String>,
    }

    struct RecordingIndexerClient {
        calls: Arc<std::sync::Mutex<Vec<RecordedSearchCall>>>,
        pruned_indexers: Arc<std::sync::Mutex<Vec<String>>>,
        search_error: Option<String>,
    }

    impl RecordingIndexerClient {
        fn new(fail_search: bool) -> Self {
            let search_error = fail_search.then(|| "forced failure".to_string());
            Self::with_search_error(search_error)
        }

        fn with_search_error(search_error: Option<String>) -> Self {
            Self {
                calls: Arc::new(std::sync::Mutex::new(Vec::new())),
                pruned_indexers: Arc::new(std::sync::Mutex::new(Vec::new())),
                search_error,
            }
        }

        fn pruned_indexers(&self) -> Vec<String> {
            self.pruned_indexers.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl IndexerClient for RecordingIndexerClient {
        async fn search(
            &self,
            query: String,
            ids: HashMap<String, String>,
            _category: Option<String>,
            facet: Option<String>,
            _id_search_facet: Option<String>,
            _newznab_categories: Option<Vec<String>>,
            _indexer_routing: Option<IndexerRoutingPlan>,
            _mode: SearchMode,
            _operation: IndexerErrorOperation,
            _season: Option<u32>,
            _episode: Option<u32>,
            _absolute_episode: Option<u32>,
            _tagged_aliases: Vec<scryer_domain::TaggedAlias>,
            _learning_context: Option<crate::IndexerSearchLearningContext>,
            _cancel_token: tokio_util::sync::CancellationToken,
        ) -> AppResult<IndexerSearchResponse> {
            self.calls
                .lock()
                .unwrap()
                .push(RecordedSearchCall { query, ids, facet });

            if let Some(error) = &self.search_error {
                return Err(AppError::Repository(error.clone()));
            }

            Ok(IndexerSearchResponse {
                completion: crate::IndexerSearchCompletion::Complete,

                indexer_outcomes: Vec::new(),
                results: vec![],
                api_current: None,
                api_max: None,
                grab_current: None,
                grab_max: None,
            })
        }

        async fn prune_search_learning(&self, indexer_id: &str) -> AppResult<()> {
            self.pruned_indexers
                .lock()
                .unwrap()
                .push(indexer_id.to_string());
            Ok(())
        }
    }

    #[derive(Default)]
    struct RecordingScopeCoverageRepository {
        pruned_indexers: Arc<std::sync::Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl ScopeIndexerCoverageRepository for RecordingScopeCoverageRepository {
        async fn record_coverage(
            &self,
            _scope_key: &str,
            _facet: &str,
            _indexer_id: &str,
            _fingerprint: &str,
        ) -> AppResult<()> {
            Ok(())
        }

        async fn covered_indexers(
            &self,
            _scope_key: &str,
            _facet: &str,
            _fingerprint: &str,
            _stale_before: Option<chrono::DateTime<Utc>>,
        ) -> AppResult<Vec<String>> {
            Ok(Vec::new())
        }

        async fn prune_scope(&self, _scope_key: &str) -> AppResult<()> {
            Ok(())
        }

        async fn prune_scope_indexer(&self, _scope_key: &str, _indexer_id: &str) -> AppResult<()> {
            Ok(())
        }

        async fn prune_indexer(&self, indexer_id: &str) -> AppResult<()> {
            self.pruned_indexers
                .lock()
                .unwrap()
                .push(indexer_id.to_string());
            Ok(())
        }

        async fn list_coverage_for_scope_keys(
            &self,
            _scope_keys: &[String],
        ) -> AppResult<Vec<ScopeCoverageRow>> {
            Ok(Vec::new())
        }
    }

    struct EmptyCapsSnapshotRefresher;

    #[async_trait]
    impl IndexerCapsSnapshotRefresher for EmptyCapsSnapshotRefresher {
        async fn fetch_for_config(
            &self,
            _config: &IndexerConfig,
        ) -> AppResult<Option<scryer_domain::IndexerCapsSnapshot>> {
            Ok(None)
        }
    }

    struct SuccessfulCapsSnapshotRefresher;

    #[async_trait]
    impl IndexerCapsSnapshotRefresher for SuccessfulCapsSnapshotRefresher {
        async fn fetch_for_config(
            &self,
            _config: &IndexerConfig,
        ) -> AppResult<Option<scryer_domain::IndexerCapsSnapshot>> {
            Ok(Some(scryer_domain::IndexerCapsSnapshot::default()))
        }
    }

    struct SuccessfulValidationThenFailingCapsSnapshotRefresher {
        calls: AtomicUsize,
    }

    impl SuccessfulValidationThenFailingCapsSnapshotRefresher {
        fn new() -> Self {
            Self {
                calls: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl IndexerCapsSnapshotRefresher for SuccessfulValidationThenFailingCapsSnapshotRefresher {
        async fn fetch_for_config(
            &self,
            _config: &IndexerConfig,
        ) -> AppResult<Option<scryer_domain::IndexerCapsSnapshot>> {
            if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                Ok(Some(scryer_domain::IndexerCapsSnapshot::default()))
            } else {
                Err(AppError::Repository("synthetic caps failure".into()))
            }
        }
    }

    type RecordedIndexerErrors = Arc<Mutex<Vec<(String, Option<String>)>>>;

    struct RecordingIndexerConfigRepo {
        created: Arc<Mutex<Vec<IndexerConfig>>>,
        cleared_ids: Arc<Mutex<Vec<String>>>,
        recorded_errors: RecordedIndexerErrors,
    }

    impl RecordingIndexerConfigRepo {
        fn new() -> Self {
            Self {
                created: Arc::new(Mutex::new(Vec::new())),
                cleared_ids: Arc::new(Mutex::new(Vec::new())),
                recorded_errors: Arc::new(Mutex::new(Vec::new())),
            }
        }

        async fn cleared_ids(&self) -> Vec<String> {
            self.cleared_ids.lock().await.clone()
        }
    }

    #[async_trait]
    impl IndexerConfigRepository for RecordingIndexerConfigRepo {
        async fn list(&self, _provider_filter: Option<String>) -> AppResult<Vec<IndexerConfig>> {
            Ok(self.created.lock().await.clone())
        }

        async fn get_by_id(&self, id: &str) -> AppResult<Option<IndexerConfig>> {
            let created = self.created.lock().await;
            Ok(created.iter().find(|config| config.id == id).cloned())
        }

        async fn create(&self, config: IndexerConfig) -> AppResult<IndexerConfig> {
            self.created.lock().await.push(config.clone());
            Ok(config)
        }

        async fn update(&self, update: crate::IndexerConfigUpdate) -> AppResult<IndexerConfig> {
            let mut created = self.created.lock().await;
            let config = created
                .iter_mut()
                .find(|config| config.id == update.id)
                .ok_or_else(|| {
                    AppError::NotFound(format!("indexer config '{}' not found", update.id))
                })?;

            if let Some(name) = update.name {
                config.name = name;
            }
            if let Some(provider_type) = update.provider_type {
                config.provider_type = provider_type;
            }
            if let Some(base_url) = update.derived_base_url {
                config.base_url = base_url;
            }
            if let Some(rate_limit_seconds) = update.rate_limit_seconds {
                config.rate_limit_seconds = Some(rate_limit_seconds);
            }
            if let Some(rate_limit_burst) = update.rate_limit_burst {
                config.rate_limit_burst = Some(rate_limit_burst);
            }
            if let Some(is_enabled) = update.is_enabled {
                config.is_enabled = is_enabled;
            }
            if let Some(enable_interactive_search) = update.enable_interactive_search {
                config.enable_interactive_search = enable_interactive_search;
            }
            if let Some(enable_auto_search) = update.enable_auto_search {
                config.enable_auto_search = enable_auto_search;
            }
            if let Some(managed_parent_config_id) = update.managed_parent_config_id {
                config.managed_parent_config_id = managed_parent_config_id;
            }
            if let Some(managed_child_key) = update.managed_child_key {
                config.managed_child_key = managed_child_key;
            }
            if let Some(managed_metadata_json) = update.managed_metadata_json {
                config.managed_metadata_json = managed_metadata_json;
            }
            if let Some(caps_snapshot_json) = update.caps_snapshot_json {
                config.caps_snapshot_json = caps_snapshot_json;
            }
            if let Some(config_json) = update.config_json {
                config.config_json = Some(config_json);
            }
            config.updated_at = Utc::now();

            Ok(config.clone())
        }

        async fn delete(&self, id: &str) -> AppResult<()> {
            self.created.lock().await.retain(|config| config.id != id);
            Ok(())
        }

        async fn touch_last_error(&self, _provider_type: &str) -> AppResult<()> {
            Ok(())
        }

        async fn clear_last_error(&self, id: &str) -> AppResult<()> {
            self.cleared_ids.lock().await.push(id.to_string());
            Ok(())
        }

        async fn record_last_error(&self, id: &str, message: Option<String>) -> AppResult<()> {
            self.recorded_errors
                .lock()
                .await
                .push((id.to_string(), message));
            Ok(())
        }
    }

    #[derive(Default)]
    struct RecordingSettingsRepository {
        values: RecordingSettingsValues,
        fail_upsert: bool,
        system_upsert_barrier: Option<Arc<SettingsUpsertBarrier>>,
    }

    struct SettingsUpsertBarrier {
        block_once: AtomicBool,
        entered: Semaphore,
        release: Semaphore,
    }

    impl SettingsUpsertBarrier {
        fn new() -> Self {
            Self {
                block_once: AtomicBool::new(true),
                entered: Semaphore::new(0),
                release: Semaphore::new(0),
            }
        }
    }

    impl RecordingSettingsRepository {
        fn with_upsert_failure() -> Self {
            Self {
                values: Default::default(),
                fail_upsert: true,
                system_upsert_barrier: None,
            }
        }

        fn with_system_upsert_barrier() -> (Self, Arc<SettingsUpsertBarrier>) {
            let barrier = Arc::new(SettingsUpsertBarrier::new());
            (
                Self {
                    values: Default::default(),
                    fail_upsert: false,
                    system_upsert_barrier: Some(barrier.clone()),
                },
                barrier,
            )
        }
    }

    type RecordingSettingsKey = (String, String, Option<String>);
    type RecordingSettingsValues = Arc<Mutex<HashMap<RecordingSettingsKey, String>>>;

    #[async_trait]
    impl SettingsRepository for RecordingSettingsRepository {
        async fn get_setting_json(
            &self,
            scope_kind: &str,
            key: &str,
            scope_id: Option<String>,
        ) -> AppResult<Option<String>> {
            Ok(self
                .values
                .lock()
                .await
                .get(&(scope_kind.to_string(), key.to_string(), scope_id))
                .cloned())
        }

        async fn upsert_setting_json(
            &self,
            scope_kind: &str,
            key: &str,
            scope_id: Option<String>,
            value: String,
            _source: &str,
            updated_by_user_id: Option<String>,
        ) -> AppResult<()> {
            if self.fail_upsert {
                return Err(AppError::Repository("forced settings write failure".into()));
            }
            if updated_by_user_id.is_none()
                && let Some(barrier) = &self.system_upsert_barrier
                && barrier.block_once.swap(false, Ordering::SeqCst)
            {
                barrier.entered.add_permits(1);
                barrier
                    .release
                    .acquire()
                    .await
                    .expect("settings upsert barrier should remain open")
                    .forget();
            }
            self.values
                .lock()
                .await
                .insert((scope_kind.to_string(), key.to_string(), scope_id), value);
            Ok(())
        }

        async fn delete_setting_value(
            &self,
            scope_kind: &str,
            key: &str,
            scope_id: Option<String>,
        ) -> AppResult<()> {
            self.values
                .lock()
                .await
                .remove(&(scope_kind.to_string(), key.to_string(), scope_id));
            Ok(())
        }

        async fn delete_values_for_scope_id(&self, scope_id: &str) -> AppResult<u32> {
            let mut values = self.values.lock().await;
            let before = values.len();
            values.retain(|(_, _, entry_scope_id), _| entry_scope_id.as_deref() != Some(scope_id));
            Ok((before - values.len()) as u32)
        }
    }

    struct RecordingPluginProvider {
        seen_configs: Arc<std::sync::Mutex<Vec<IndexerConfig>>>,
        seen_management_configs: Arc<std::sync::Mutex<Vec<IndexerConfig>>>,
        client: Arc<RecordingIndexerClient>,
        provider_type: String,
        fields: Vec<scryer_domain::ConfigFieldDef>,
        capabilities: scryer_domain::IndexerProviderCapabilities,
        supports_validate_config: bool,
        supports_managed_children_sync: bool,
        validate_result: crate::IndexerValidationResult,
        sync_plan: crate::IndexerSyncPlan,
        plan_sync_error: Option<String>,
        plan_sync_fail_on_call: Option<usize>,
        plan_sync_calls: Arc<std::sync::Mutex<usize>>,
        preview_sync_plan_calls: Arc<std::sync::Mutex<usize>>,
        plan_sync_delay: Option<Duration>,
        active_plan_sync_calls: Arc<AtomicUsize>,
        max_concurrent_plan_sync_calls: Arc<AtomicUsize>,
    }

    impl RecordingPluginProvider {
        fn new(
            provider_type: &str,
            fields: Vec<scryer_domain::ConfigFieldDef>,
            capabilities: scryer_domain::IndexerProviderCapabilities,
            client: Arc<RecordingIndexerClient>,
        ) -> Self {
            Self {
                seen_configs: Arc::new(std::sync::Mutex::new(Vec::new())),
                seen_management_configs: Arc::new(std::sync::Mutex::new(Vec::new())),
                client,
                provider_type: provider_type.to_string(),
                fields,
                capabilities,
                supports_validate_config: false,
                supports_managed_children_sync: false,
                validate_result: crate::IndexerValidationResult {
                    status: "valid".to_string(),
                    message: None,
                    retry_after_seconds: None,
                },
                sync_plan: crate::IndexerSyncPlan::default(),
                plan_sync_error: None,
                plan_sync_fail_on_call: None,
                plan_sync_calls: Arc::new(std::sync::Mutex::new(0)),
                preview_sync_plan_calls: Arc::new(std::sync::Mutex::new(0)),
                plan_sync_delay: None,
                active_plan_sync_calls: Arc::new(AtomicUsize::new(0)),
                max_concurrent_plan_sync_calls: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn with_validate_config_support() -> Self {
            let mut provider = Self::new(
                "torrent_rss",
                vec![string_field(
                    "feed_url",
                    "Feed URL",
                    Some(scryer_domain::ConfigFieldRole::ConnectionUrl),
                )],
                rss_only_capabilities(),
                Arc::new(RecordingIndexerClient::new(false)),
            );
            provider.supports_validate_config = true;
            provider
        }

        fn with_sync_plan(sync_plan: crate::IndexerSyncPlan) -> Self {
            Self::with_sync_plan_for_provider("manager", sync_plan)
        }

        fn with_sync_plan_for_provider(
            provider_type: &str,
            sync_plan: crate::IndexerSyncPlan,
        ) -> Self {
            let mut provider = Self::new(
                provider_type,
                vec![string_field(
                    "base_url",
                    "Base URL",
                    Some(scryer_domain::ConfigFieldRole::ConnectionUrl),
                )],
                Default::default(),
                Arc::new(RecordingIndexerClient::new(false)),
            );
            provider.supports_validate_config = true;
            provider.supports_managed_children_sync = true;
            provider.sync_plan = sync_plan;
            provider
        }

        fn with_delayed_sync_plan(sync_plan: crate::IndexerSyncPlan, delay: Duration) -> Self {
            let mut provider = Self::with_sync_plan(sync_plan);
            provider.plan_sync_delay = Some(delay);
            provider
        }

        fn with_plan_sync_error(message: &str) -> Self {
            let mut provider = Self::with_sync_plan(crate::IndexerSyncPlan::default());
            provider.plan_sync_error = Some(message.to_string());
            provider.plan_sync_fail_on_call = Some(1);
            provider
        }
    }

    struct RecordingIndexerManagementClient {
        validate_result: crate::IndexerValidationResult,
        sync_plan: crate::IndexerSyncPlan,
        plan_sync_error: Option<String>,
        plan_sync_fail_on_call: Option<usize>,
        plan_sync_calls: Arc<std::sync::Mutex<usize>>,
        preview_sync_plan_calls: Arc<std::sync::Mutex<usize>>,
        plan_sync_delay: Option<Duration>,
        active_plan_sync_calls: Arc<AtomicUsize>,
        max_concurrent_plan_sync_calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl IndexerManagementClient for RecordingIndexerManagementClient {
        async fn validate_connection(&self) -> AppResult<crate::IndexerValidationResult> {
            Ok(self.validate_result.clone())
        }

        async fn preview_sync_plan(
            &self,
            _parent_config_id: &str,
        ) -> AppResult<crate::IndexerSyncPlan> {
            let mut calls = self.preview_sync_plan_calls.lock().unwrap();
            *calls += 1;
            Ok(self.sync_plan.clone())
        }

        async fn plan_sync(&self, _parent_config_id: &str) -> AppResult<crate::IndexerSyncPlan> {
            let active = self.active_plan_sync_calls.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_concurrent_plan_sync_calls
                .fetch_max(active, Ordering::SeqCst);
            if let Some(delay) = self.plan_sync_delay {
                tokio::time::sleep(delay).await;
            }
            self.active_plan_sync_calls.fetch_sub(1, Ordering::SeqCst);

            let mut calls = self.plan_sync_calls.lock().unwrap();
            *calls += 1;
            if let Some(message) = &self.plan_sync_error
                && self
                    .plan_sync_fail_on_call
                    .is_none_or(|call| *calls == call)
            {
                return Err(AppError::Validation(message.clone()));
            }
            Ok(self.sync_plan.clone())
        }

        fn name(&self) -> &str {
            "torrent_rss"
        }
    }

    impl IndexerPluginProvider for RecordingPluginProvider {
        fn client_for_provider(&self, config: &IndexerConfig) -> Option<Arc<dyn IndexerClient>> {
            self.seen_configs.lock().unwrap().push(config.clone());
            Some(self.client.clone())
        }

        fn management_client_for_provider(
            &self,
            config: &IndexerConfig,
        ) -> Option<Arc<dyn IndexerManagementClient>> {
            self.seen_management_configs
                .lock()
                .unwrap()
                .push(config.clone());
            Some(Arc::new(RecordingIndexerManagementClient {
                validate_result: self.validate_result.clone(),
                sync_plan: self.sync_plan.clone(),
                plan_sync_error: self.plan_sync_error.clone(),
                plan_sync_fail_on_call: self.plan_sync_fail_on_call,
                plan_sync_calls: self.plan_sync_calls.clone(),
                preview_sync_plan_calls: self.preview_sync_plan_calls.clone(),
                plan_sync_delay: self.plan_sync_delay,
                active_plan_sync_calls: self.active_plan_sync_calls.clone(),
                max_concurrent_plan_sync_calls: self.max_concurrent_plan_sync_calls.clone(),
            }))
        }

        fn available_provider_types(&self) -> Vec<String> {
            let mut provider_types = vec![self.provider_type.clone()];
            if self.provider_type == "manager" {
                provider_types.push("torrent_rss".to_string());
            }
            provider_types
        }

        fn management_capabilities_for_provider(
            &self,
            provider_type: &str,
        ) -> scryer_domain::IndexerManagementCapabilities {
            if provider_type != self.provider_type {
                return scryer_domain::IndexerManagementCapabilities::default();
            }

            scryer_domain::IndexerManagementCapabilities {
                supports_validate_config: self.supports_validate_config,
                supports_managed_children_sync: self.supports_managed_children_sync,
            }
        }

        fn config_fields_for_provider(
            &self,
            provider_type: &str,
        ) -> Vec<scryer_domain::ConfigFieldDef> {
            if provider_type != self.provider_type {
                if self.provider_type == "manager" && provider_type == "torrent_rss" {
                    return vec![string_field(
                        "feed_url",
                        "Feed URL",
                        Some(scryer_domain::ConfigFieldRole::ConnectionUrl),
                    )];
                }
                return vec![];
            }
            self.fields.clone()
        }

        fn capabilities_for_provider(
            &self,
            provider_type: &str,
        ) -> scryer_domain::IndexerProviderCapabilities {
            if provider_type == self.provider_type {
                return self.capabilities.clone();
            }

            Default::default()
        }

        fn scoring_policies(&self) -> Vec<scryer_rules::UserPolicy> {
            vec![]
        }
    }

    fn string_field(
        key: &str,
        label: &str,
        role: Option<scryer_domain::ConfigFieldRole>,
    ) -> scryer_domain::ConfigFieldDef {
        scryer_domain::ConfigFieldDef {
            key: key.to_string(),
            label: label.to_string(),
            field_type: scryer_domain::ConfigFieldType::String,
            required: true,
            default_value: None,
            value_source: scryer_domain::ConfigFieldValueSource::User,
            role,
            host_binding: None,
            options: vec![],
            help_text: None,
        }
    }

    fn password_field(key: &str, label: &str) -> scryer_domain::ConfigFieldDef {
        scryer_domain::ConfigFieldDef {
            key: key.to_string(),
            label: label.to_string(),
            field_type: scryer_domain::ConfigFieldType::Password,
            required: true,
            default_value: None,
            value_source: scryer_domain::ConfigFieldValueSource::User,
            role: None,
            host_binding: None,
            options: vec![],
            help_text: None,
        }
    }

    fn searchable_capabilities() -> scryer_domain::IndexerProviderCapabilities {
        scryer_domain::IndexerProviderCapabilities {
            query_param: Some("q".into()),
            search: true,
            ..Default::default()
        }
    }

    fn rss_only_capabilities() -> scryer_domain::IndexerProviderCapabilities {
        scryer_domain::IndexerProviderCapabilities {
            rss: true,
            query_param: None,
            search: false,
            supported_ids: HashMap::new(),
            ..Default::default()
        }
    }

    fn test_app(
        indexer_configs: Arc<dyn IndexerConfigRepository>,
        plugin_provider: Option<Arc<dyn IndexerPluginProvider>>,
        settings: Arc<dyn SettingsRepository>,
    ) -> AppUseCase {
        test_app_with_indexer_client(
            indexer_configs,
            plugin_provider,
            settings,
            Arc::new(NullIndexerClient),
        )
    }

    fn test_app_with_indexer_client(
        indexer_configs: Arc<dyn IndexerConfigRepository>,
        plugin_provider: Option<Arc<dyn IndexerPluginProvider>>,
        settings: Arc<dyn SettingsRepository>,
        indexer_client: Arc<dyn IndexerClient>,
    ) -> AppUseCase {
        let services = AppServices::builder(
            Arc::new(NullTitleRepository),
            Arc::new(NullShowRepository),
            Arc::new(NullUserRepository),
            indexer_configs,
            indexer_client,
            Arc::new(NullDownloadClient),
            Arc::new(NullDownloadClientConfigRepository),
            Arc::new(NullReleaseAttemptRepository),
            settings,
            Arc::new(NullQualityProfileRepository),
            String::new(),
        );
        let services = if let Some(plugin_provider) = plugin_provider {
            services
                .with_plugin_provider(plugin_provider)
                .build_partial_for_tests()
        } else {
            services.build_partial_for_tests()
        };

        AppUseCase::new(
            services,
            JwtAuthConfig {
                issuer: "test".to_string(),
                access_ttl_seconds: 3600,
                jwt_signing_salt: "test-salt".to_string(),
            },
            Arc::new(FacetRegistry::new()),
        )
    }

    async fn wait_for_plan_sync_calls(provider: &RecordingPluginProvider, expected_calls: usize) {
        for _ in 0..50 {
            if *provider.plan_sync_calls.lock().unwrap() == expected_calls {
                tokio::task::yield_now().await;
                tokio::time::sleep(Duration::from_millis(10)).await;
                assert_eq!(*provider.plan_sync_calls.lock().unwrap(), expected_calls);
                return;
            }
            tokio::task::yield_now().await;
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        assert_eq!(*provider.plan_sync_calls.lock().unwrap(), expected_calls);
    }

    async fn expect_indexers_changed(
        receiver: &mut tokio::sync::broadcast::Receiver<()>,
        context: &str,
    ) {
        tokio::time::timeout(Duration::from_secs(1), receiver.recv())
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for indexersChanged: {context}"))
            .unwrap_or_else(|error| {
                panic!("indexersChanged receiver failed for {context}: {error}")
            });
    }

    fn test_admin() -> User {
        let mut user = User::new_admin("admin");
        user.authorization = scryer_domain::UserAuthorization {
            app: scryer_domain::AppPermissionMask::from_permissions([
                scryer_domain::AppPermission::ManageSystemSettings,
                scryer_domain::AppPermission::ManageCatalogSettings,
            ]),
            actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
            loaded: true,
            ..Default::default()
        };
        user
    }

    #[test]
    fn validate_test_flight_url_uses_origin_only_for_preflight() {
        let url = validate_test_flight_url("https://api.nzbgeek.info/api?t=search&apikey=secret")
            .expect("valid test-flight URL");

        assert_eq!(
            url.origin().ascii_serialization(),
            "https://api.nzbgeek.info"
        );
    }

    #[test]
    fn validate_test_flight_url_rejects_embedded_credentials() {
        let err = validate_test_flight_url("https://user:secret@api.nzbgeek.info")
            .expect_err("embedded credentials should be rejected");

        assert!(
            err.to_string()
                .contains("base URL must not include embedded credentials")
        );
    }

    #[test]
    fn validate_test_flight_url_allows_operator_homelab_addresses() {
        for raw in [
            "http://localhost:9696",
            "http://127.0.0.1:9696",
            "http://192.168.1.10:9696",
            "http://10.42.0.20:9696",
            "http://prowlarr:9696",
        ] {
            validate_test_flight_url(raw)
                .unwrap_or_else(|error| panic!("{raw} should be valid: {error}"));
        }
    }

    #[test]
    fn preflight_transport_error_hints_when_https_service_lacks_tls() {
        let url = validate_test_flight_url("https://localhost:9696").expect("valid URL");
        let message = format_preflight_transport_error(
            &url,
            "https://localhost:9696/",
            "error sending request for url (https://localhost:9696/)",
        );

        assert!(message.contains("try http:// instead of https://"));
        assert!(message.contains("https://localhost:9696/"));
    }

    #[tokio::test]
    async fn create_indexer_config_derives_base_url_from_feed_url() {
        let indexer_repo = Arc::new(RecordingIndexerConfigRepo::new());
        let client = Arc::new(RecordingIndexerClient::new(false));
        let app = test_app(
            indexer_repo.clone(),
            Some(Arc::new(RecordingPluginProvider::new(
                "torrent_rss",
                vec![string_field(
                    "feed_url",
                    "Feed URL",
                    Some(scryer_domain::ConfigFieldRole::ConnectionUrl),
                )],
                rss_only_capabilities(),
                client,
            ))),
            Arc::new(NullSettingsRepository),
        );
        let created = app
            .create_indexer_config(
                &test_admin(),
                NewIndexerConfig {
                    name: "RSS".to_string(),
                    provider_type: "torrent_rss".to_string(),
                    rate_limit_seconds: None,
                    rate_limit_burst: None,
                    is_enabled: true,
                    enable_interactive_search: true,
                    enable_auto_search: true,
                    indexer_proxy_config_id: None,
                    download_client_id: None,
                    config_json: Some(
                        r#"{"feed_url":"https://ipt.beelyrics.net/t.rss?u=2203846"}"#.to_string(),
                    ),
                },
            )
            .await
            .unwrap();

        assert_eq!(created.base_url, "https://ipt.beelyrics.net");
    }

    #[tokio::test]
    async fn create_indexer_config_publishes_indexers_changed() {
        let indexer_repo = Arc::new(RecordingIndexerConfigRepo::new());
        let client = Arc::new(RecordingIndexerClient::new(false));
        let app = test_app(
            indexer_repo.clone(),
            Some(Arc::new(RecordingPluginProvider::new(
                "torrent_rss",
                vec![string_field(
                    "feed_url",
                    "Feed URL",
                    Some(scryer_domain::ConfigFieldRole::ConnectionUrl),
                )],
                rss_only_capabilities(),
                client,
            ))),
            Arc::new(NullSettingsRepository),
        );
        let mut receiver = app.runtime.events.indexers_changed_broadcast.subscribe();

        app.create_indexer_config(
            &test_admin(),
            NewIndexerConfig {
                name: "RSS".to_string(),
                provider_type: "torrent_rss".to_string(),
                rate_limit_seconds: None,
                rate_limit_burst: None,
                is_enabled: true,
                enable_interactive_search: true,
                enable_auto_search: true,
                indexer_proxy_config_id: None,
                download_client_id: None,
                config_json: Some(
                    r#"{"feed_url":"https://ipt.beelyrics.net/t.rss?u=2203846"}"#.to_string(),
                ),
            },
        )
        .await
        .unwrap();

        expect_indexers_changed(&mut receiver, "create_indexer_config").await;
    }

    #[tokio::test]
    async fn create_indexer_config_rejects_invalid_connection_details() {
        let indexer_repo = Arc::new(RecordingIndexerConfigRepo::new());
        let client = Arc::new(RecordingIndexerClient::new(true));
        let app = test_app(
            indexer_repo.clone(),
            Some(Arc::new(RecordingPluginProvider::new(
                "nzbgeek",
                vec![
                    string_field(
                        "base_url",
                        "Base URL",
                        Some(scryer_domain::ConfigFieldRole::ConnectionUrl),
                    ),
                    password_field("api_key", "API Key"),
                ],
                searchable_capabilities(),
                client,
            ))),
            Arc::new(NullSettingsRepository),
        );

        let error = app
            .create_indexer_config(
                &test_admin(),
                NewIndexerConfig {
                    name: "NZBGeek".to_string(),
                    provider_type: "nzbgeek".to_string(),
                    rate_limit_seconds: None,
                    rate_limit_burst: None,
                    is_enabled: true,
                    enable_interactive_search: true,
                    enable_auto_search: true,
                    indexer_proxy_config_id: None,
                    download_client_id: None,
                    config_json: Some(
                        r#"{"base_url":"https://api.nzbgeek.info/","api_key":"bad-key"}"#
                            .to_string(),
                    ),
                },
            )
            .await
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "repository: indexer connection test failed: repository: forced failure"
        );
        assert!(indexer_repo.created.lock().await.is_empty());
    }

    #[tokio::test]
    async fn update_indexer_config_rejects_invalid_connection_details() {
        let indexer_repo = Arc::new(RecordingIndexerConfigRepo::new());
        indexer_repo.created.lock().await.push(IndexerConfig {
            id: "cfg-1".to_string(),
            name: "NZBGeek".to_string(),
            provider_type: "nzbgeek".to_string(),
            base_url: "https://api.nzbgeek.info/".to_string(),
            api_key_encrypted: None,
            rate_limit_seconds: None,
            rate_limit_burst: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            indexer_proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            config_json: Some(
                r#"{"base_url":"https://api.nzbgeek.info/","api_key":"good-key"}"#.to_string(),
            ),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        });
        let client = Arc::new(RecordingIndexerClient::new(true));
        let app = test_app(
            indexer_repo.clone(),
            Some(Arc::new(RecordingPluginProvider::new(
                "nzbgeek",
                vec![
                    string_field(
                        "base_url",
                        "Base URL",
                        Some(scryer_domain::ConfigFieldRole::ConnectionUrl),
                    ),
                    password_field("api_key", "API Key"),
                ],
                searchable_capabilities(),
                client,
            ))),
            Arc::new(NullSettingsRepository),
        );

        let error = app
            .update_indexer_config(
                &test_admin(),
                crate::IndexerConfigUpdate {
                    id: "cfg-1".to_string(),
                    name: None,
                    provider_type: None,
                    derived_base_url: None,
                    rate_limit_seconds: None,
                    rate_limit_burst: None,
                    is_enabled: None,
                    enable_interactive_search: None,
                    enable_auto_search: None,
                    indexer_proxy_config_id: None,
                    download_client_id: None,
                    seeding_profile_id: None,
                    managed_parent_config_id: None,
                    managed_child_key: None,
                    managed_metadata_json: None,
                    caps_snapshot_json: None,
                    config_json: Some(
                        r#"{"base_url":"https://api.nzbgeek.info/","api_key":"bad-key"}"#
                            .to_string(),
                    ),
                },
            )
            .await
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "repository: indexer connection test failed: repository: forced failure"
        );
        let stored = indexer_repo
            .get_by_id("cfg-1")
            .await
            .unwrap()
            .expect("existing config");
        assert_eq!(
            stored.config_json.as_deref(),
            Some(r#"{"base_url":"https://api.nzbgeek.info/","api_key":"good-key"}"#)
        );
    }

    #[tokio::test]
    async fn update_indexer_config_skips_connection_test_for_rename_only() {
        let indexer_repo = Arc::new(RecordingIndexerConfigRepo::new());
        indexer_repo.created.lock().await.push(IndexerConfig {
            id: "cfg-1".to_string(),
            name: "NZBGeek".to_string(),
            provider_type: "nzbgeek".to_string(),
            base_url: "https://api.nzbgeek.info/".to_string(),
            api_key_encrypted: None,
            rate_limit_seconds: None,
            rate_limit_burst: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            indexer_proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            config_json: Some(
                r#"{"base_url":"https://api.nzbgeek.info/","api_key":"good-key"}"#.to_string(),
            ),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        });
        let client = Arc::new(RecordingIndexerClient::new(true));
        let app = test_app_with_indexer_client(
            indexer_repo.clone(),
            Some(Arc::new(RecordingPluginProvider::new(
                "nzbgeek",
                vec![
                    string_field(
                        "base_url",
                        "Base URL",
                        Some(scryer_domain::ConfigFieldRole::ConnectionUrl),
                    ),
                    password_field("api_key", "API Key"),
                ],
                searchable_capabilities(),
                client.clone(),
            ))),
            Arc::new(NullSettingsRepository),
            client.clone(),
        );

        let updated = app
            .update_indexer_config(
                &test_admin(),
                crate::IndexerConfigUpdate {
                    id: "cfg-1".to_string(),
                    name: Some("NZBGeek Mirror".to_string()),
                    provider_type: None,
                    derived_base_url: None,
                    rate_limit_seconds: None,
                    rate_limit_burst: None,
                    is_enabled: None,
                    enable_interactive_search: None,
                    enable_auto_search: None,
                    indexer_proxy_config_id: None,
                    download_client_id: None,
                    seeding_profile_id: None,
                    managed_parent_config_id: None,
                    managed_child_key: None,
                    managed_metadata_json: None,
                    caps_snapshot_json: None,
                    config_json: None,
                },
            )
            .await
            .unwrap();

        assert_eq!(updated.name, "NZBGeek Mirror");
        assert!(client.calls.lock().unwrap().is_empty());
        assert!(client.pruned_indexers().is_empty());
        assert!(indexer_repo.cleared_ids().await.is_empty());
    }

    #[tokio::test]
    async fn update_indexer_config_clears_last_error_after_successful_validation() {
        let indexer_repo = Arc::new(RecordingIndexerConfigRepo::new());
        indexer_repo.created.lock().await.push(IndexerConfig {
            id: "cfg-1".to_string(),
            name: "NZBGeek".to_string(),
            provider_type: "nzbgeek".to_string(),
            base_url: "https://api.nzbgeek.info/".to_string(),
            api_key_encrypted: None,
            rate_limit_seconds: None,
            rate_limit_burst: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            indexer_proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: Some("Last search failed".to_string()),
            last_error_message: Some("Last search failed".to_string()),
            last_error_at: Some(Utc::now()),
            config_json: Some(
                r#"{"base_url":"https://api.nzbgeek.info/","api_key":"good-key"}"#.to_string(),
            ),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        });
        let client = Arc::new(RecordingIndexerClient::new(false));
        let app = test_app_with_indexer_client(
            indexer_repo.clone(),
            Some(Arc::new(RecordingPluginProvider::new(
                "nzbgeek",
                vec![
                    string_field(
                        "base_url",
                        "Base URL",
                        Some(scryer_domain::ConfigFieldRole::ConnectionUrl),
                    ),
                    password_field("api_key", "API Key"),
                ],
                searchable_capabilities(),
                client.clone(),
            ))),
            Arc::new(NullSettingsRepository),
            client.clone(),
        );

        app.update_indexer_config(
            &test_admin(),
            crate::IndexerConfigUpdate {
                id: "cfg-1".to_string(),
                name: None,
                provider_type: None,
                derived_base_url: None,
                rate_limit_seconds: None,
                rate_limit_burst: None,
                is_enabled: None,
                enable_interactive_search: None,
                enable_auto_search: None,
                indexer_proxy_config_id: None,
                download_client_id: None,
                seeding_profile_id: None,
                managed_parent_config_id: None,
                managed_child_key: None,
                managed_metadata_json: None,
                caps_snapshot_json: None,
                config_json: Some(
                    r#"{"base_url":"https://api.nzbgeek.info/","api_key":"new-good-key"}"#
                        .to_string(),
                ),
            },
        )
        .await
        .expect("validated update should succeed");

        assert_eq!(indexer_repo.cleared_ids().await, vec!["cfg-1".to_string()]);
        assert_eq!(client.pruned_indexers(), vec!["cfg-1".to_string()]);
    }

    #[tokio::test]
    async fn missing_caps_snapshot_prunes_coverage_and_learning() {
        let indexer_repo = Arc::new(RecordingIndexerConfigRepo::new());
        indexer_repo.created.lock().await.push(IndexerConfig {
            id: "cfg-caps".into(),
            name: "Synthetic Indexer".into(),
            provider_type: "newznab".into(),
            base_url: "https://indexer.example.test".into(),
            api_key_encrypted: None,
            rate_limit_seconds: None,
            rate_limit_burst: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            indexer_proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            config_json: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        });
        let indexer_client = Arc::new(RecordingIndexerClient::new(false));
        let coverage = Arc::new(RecordingScopeCoverageRepository::default());
        let services = AppServices::builder(
            Arc::new(NullTitleRepository),
            Arc::new(NullShowRepository),
            Arc::new(NullUserRepository),
            indexer_repo.clone(),
            indexer_client.clone(),
            Arc::new(NullDownloadClient),
            Arc::new(NullDownloadClientConfigRepository),
            Arc::new(NullReleaseAttemptRepository),
            Arc::new(NullSettingsRepository),
            Arc::new(NullQualityProfileRepository),
            String::new(),
        )
        .with_scope_indexer_coverage_store(coverage.clone())
        .with_indexer_caps_refresher(Arc::new(EmptyCapsSnapshotRefresher))
        .build_partial_for_tests();
        let app = AppUseCase::new(
            services,
            JwtAuthConfig {
                issuer: "test".into(),
                access_ttl_seconds: 3_600,
                jwt_signing_salt: "test-salt".into(),
            },
            Arc::new(FacetRegistry::new()),
        );

        let (refreshed, failures) = app
            .refresh_enabled_direct_nab_caps_snapshots(&test_admin())
            .await
            .expect("caps refresh pass should report provider failures");

        assert_eq!(refreshed, 0);
        assert_eq!(failures.len(), 1);
        assert_eq!(indexer_client.pruned_indexers(), vec!["cfg-caps"]);
        assert_eq!(
            coverage.pruned_indexers.lock().unwrap().as_slice(),
            &["cfg-caps"]
        );
        let errors = indexer_repo.recorded_errors.lock().await;
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].0, "cfg-caps");
        assert!(
            errors[0]
                .1
                .as_deref()
                .is_some_and(|message| message.starts_with("caps refresh failed:"))
        );
    }

    #[tokio::test]
    async fn successful_caps_refresh_clears_only_caps_health_errors() {
        let indexer_repo = Arc::new(RecordingIndexerConfigRepo::new());
        let now = Utc::now();
        let caps_error = IndexerConfig {
            id: "cfg-caps-error".into(),
            name: "Synthetic Indexer A".into(),
            provider_type: "newznab".into(),
            base_url: "https://indexer-a.example.test".into(),
            api_key_encrypted: None,
            rate_limit_seconds: None,
            rate_limit_burst: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            indexer_proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_message: Some("caps refresh failed: synthetic failure".into()),
            last_error_at: Some(now),
            config_json: None,
            created_at: now,
            updated_at: now,
        };
        let mut unrelated_error = caps_error.clone();
        unrelated_error.id = "cfg-other-error".into();
        unrelated_error.name = "Synthetic Indexer B".into();
        unrelated_error.base_url = "https://indexer-b.example.test".into();
        unrelated_error.last_error_message = Some("authentication failed".into());
        indexer_repo
            .created
            .lock()
            .await
            .extend([caps_error, unrelated_error]);
        let services = AppServices::builder(
            Arc::new(NullTitleRepository),
            Arc::new(NullShowRepository),
            Arc::new(NullUserRepository),
            indexer_repo.clone(),
            Arc::new(RecordingIndexerClient::new(false)),
            Arc::new(NullDownloadClient),
            Arc::new(NullDownloadClientConfigRepository),
            Arc::new(NullReleaseAttemptRepository),
            Arc::new(NullSettingsRepository),
            Arc::new(NullQualityProfileRepository),
            String::new(),
        )
        .with_indexer_caps_refresher(Arc::new(SuccessfulCapsSnapshotRefresher))
        .build_partial_for_tests();
        let app = AppUseCase::new(
            services,
            JwtAuthConfig {
                issuer: "test".into(),
                access_ttl_seconds: 3_600,
                jwt_signing_salt: "test-salt".into(),
            },
            Arc::new(FacetRegistry::new()),
        );

        let (refreshed, failures) = app
            .refresh_enabled_direct_nab_caps_snapshots(&test_admin())
            .await
            .expect("valid caps should refresh both indexers");

        assert_eq!(refreshed, 2);
        assert!(failures.is_empty());
        assert_eq!(
            indexer_repo.cleared_ids().await,
            vec!["cfg-caps-error".to_string()]
        );
    }

    #[tokio::test]
    async fn validated_update_does_not_clear_a_caps_refresh_failure() {
        let indexer_repo = Arc::new(RecordingIndexerConfigRepo::new());
        indexer_repo.created.lock().await.push(IndexerConfig {
            id: "cfg-update-caps".into(),
            name: "Synthetic Indexer".into(),
            provider_type: "newznab".into(),
            base_url: "https://indexer.example.test".into(),
            api_key_encrypted: None,
            rate_limit_seconds: None,
            rate_limit_burst: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            indexer_proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            config_json: Some(
                serde_json::json!({
                    "base_url": "https://indexer.example.test",
                    "api_key": "old-key",
                })
                .to_string(),
            ),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        });
        let indexer_client = Arc::new(RecordingIndexerClient::new(false));
        let coverage = Arc::new(RecordingScopeCoverageRepository::default());
        let plugin_provider = Arc::new(RecordingPluginProvider::new(
            "newznab",
            vec![
                string_field(
                    "base_url",
                    "Base URL",
                    Some(scryer_domain::ConfigFieldRole::ConnectionUrl),
                ),
                password_field("api_key", "API Key"),
            ],
            searchable_capabilities(),
            indexer_client.clone(),
        ));
        let services = AppServices::builder(
            Arc::new(NullTitleRepository),
            Arc::new(NullShowRepository),
            Arc::new(NullUserRepository),
            indexer_repo.clone(),
            indexer_client.clone(),
            Arc::new(NullDownloadClient),
            Arc::new(NullDownloadClientConfigRepository),
            Arc::new(NullReleaseAttemptRepository),
            Arc::new(NullSettingsRepository),
            Arc::new(NullQualityProfileRepository),
            String::new(),
        )
        .with_scope_indexer_coverage_store(coverage)
        .with_indexer_caps_refresher(Arc::new(
            SuccessfulValidationThenFailingCapsSnapshotRefresher::new(),
        ))
        .with_plugin_provider(plugin_provider)
        .build_partial_for_tests();
        let app = AppUseCase::new(
            services,
            JwtAuthConfig {
                issuer: "test".into(),
                access_ttl_seconds: 3_600,
                jwt_signing_salt: "test-salt".into(),
            },
            Arc::new(FacetRegistry::new()),
        );

        app.update_indexer_config(
            &test_admin(),
            IndexerConfigUpdate {
                id: "cfg-update-caps".into(),
                config_json: Some(
                    serde_json::json!({
                        "base_url": "https://indexer.example.test",
                        "api_key": "new-key",
                    })
                    .to_string(),
                ),
                ..Default::default()
            },
        )
        .await
        .expect("caps failure remains non-blocking after connection validation");

        assert!(indexer_repo.cleared_ids().await.is_empty());
        assert_eq!(indexer_client.pruned_indexers(), vec!["cfg-update-caps"]);
        let errors = indexer_repo.recorded_errors.lock().await;
        assert_eq!(errors.len(), 1);
        assert!(
            errors[0]
                .1
                .as_deref()
                .is_some_and(|message| message.starts_with("caps refresh failed:"))
        );
    }

    #[tokio::test]
    async fn test_indexer_connection_with_indexer_id_uses_persisted_base_url_and_api_key() {
        let indexer_repo = Arc::new(RecordingIndexerConfigRepo::new());
        indexer_repo.created.lock().await.push(IndexerConfig {
            id: "cfg-1".to_string(),
            name: "NZBGeek".to_string(),
            provider_type: "nzbgeek".to_string(),
            base_url: "https://api.nzbgeek.info".to_string(),
            api_key_encrypted: None,
            rate_limit_seconds: None,
            rate_limit_burst: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            indexer_proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            config_json: Some(
                r#"{"base_url":"https://api.nzbgeek.info","api_key":"good-key"}"#.to_string(),
            ),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        });
        let client = Arc::new(RecordingIndexerClient::new(false));
        let provider = Arc::new(RecordingPluginProvider::new(
            "nzbgeek",
            vec![
                string_field(
                    "base_url",
                    "Base URL",
                    Some(scryer_domain::ConfigFieldRole::ConnectionUrl),
                ),
                password_field("api_key", "API Key"),
            ],
            searchable_capabilities(),
            client,
        ));
        let app = test_app(
            indexer_repo.clone(),
            Some(provider.clone()),
            Arc::new(NullSettingsRepository),
        );
        let mut receiver = app.runtime.events.indexers_changed_broadcast.subscribe();

        app.test_indexer_connection(&test_admin(), "nzbgeek", None, Some("cfg-1"), None)
            .await
            .expect("persisted config should be reused");

        let seen = provider.seen_configs.lock().unwrap().clone();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].base_url, "https://api.nzbgeek.info");
        assert_eq!(
            seen[0].config_json.as_deref(),
            Some(r#"{"base_url":"https://api.nzbgeek.info","api_key":"good-key"}"#)
        );
        assert_eq!(indexer_repo.cleared_ids().await, vec!["cfg-1".to_string()]);
        expect_indexers_changed(&mut receiver, "test_indexer_connection").await;
    }

    #[tokio::test]
    async fn test_indexer_connection_requires_secret_when_origin_changes() {
        let indexer_repo = Arc::new(RecordingIndexerConfigRepo::new());
        indexer_repo.created.lock().await.push(IndexerConfig {
            id: "cfg-1".to_string(),
            name: "NZBGeek".to_string(),
            provider_type: "nzbgeek".to_string(),
            base_url: "https://api.nzbgeek.info".to_string(),
            api_key_encrypted: None,
            rate_limit_seconds: None,
            rate_limit_burst: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            indexer_proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            config_json: Some(
                r#"{"base_url":"https://api.nzbgeek.info","api_key":"good-key"}"#.to_string(),
            ),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        });
        let provider = Arc::new(RecordingPluginProvider::new(
            "nzbgeek",
            vec![
                string_field(
                    "base_url",
                    "Base URL",
                    Some(scryer_domain::ConfigFieldRole::ConnectionUrl),
                ),
                password_field("api_key", "API Key"),
            ],
            searchable_capabilities(),
            Arc::new(RecordingIndexerClient::new(false)),
        ));
        let app = test_app(
            indexer_repo,
            Some(provider.clone()),
            Arc::new(NullSettingsRepository),
        );

        let error = app
            .test_indexer_connection(
                &test_admin(),
                "nzbgeek",
                Some(r#"{"base_url":"https://mirror.nzbgeek.info","api_key":""}"#),
                Some("cfg-1"),
                None,
            )
            .await
            .expect_err("changed origin should require an explicit API key");

        assert_eq!(error.to_string(), "validation: API Key is required");
        assert!(provider.seen_configs.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn create_indexer_config_saves_managed_parent_before_background_sync_failure() {
        let indexer_repo = Arc::new(RecordingIndexerConfigRepo::new());
        let provider = Arc::new(RecordingPluginProvider::with_plan_sync_error(
            "managed sync preflight failed",
        ));
        let app = test_app(
            indexer_repo.clone(),
            Some(provider.clone()),
            Arc::new(NullSettingsRepository),
        );

        let created = app
            .create_indexer_config(
                &test_admin(),
                NewIndexerConfig {
                    name: "Manager".to_string(),
                    provider_type: "manager".to_string(),
                    rate_limit_seconds: None,
                    rate_limit_burst: None,
                    is_enabled: true,
                    enable_interactive_search: true,
                    enable_auto_search: true,
                    indexer_proxy_config_id: None,
                    download_client_id: None,
                    config_json: Some(r#"{"base_url":"https://manager.example"}"#.to_string()),
                },
            )
            .await
            .expect("managed parent should save before background sync failure");

        wait_for_plan_sync_calls(&provider, 1).await;
        let stored = indexer_repo
            .get_by_id(&created.id)
            .await
            .unwrap()
            .expect("saved managed parent");
        assert_eq!(stored.name, "Manager");
        assert!(
            indexer_repo
                .list(None)
                .await
                .unwrap()
                .iter()
                .all(|config| config.managed_parent_config_id.is_none())
        );
        assert_eq!(*provider.plan_sync_calls.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn update_indexer_config_enables_managed_parent_even_if_background_sync_fails() {
        let indexer_repo = Arc::new(RecordingIndexerConfigRepo::new());
        indexer_repo.created.lock().await.push(IndexerConfig {
            id: "cfg-1".to_string(),
            name: "Manager".to_string(),
            provider_type: "manager".to_string(),
            base_url: "https://manager.example".to_string(),
            api_key_encrypted: None,
            rate_limit_seconds: None,
            rate_limit_burst: None,
            disabled_until: None,
            is_enabled: false,
            enable_interactive_search: false,
            enable_auto_search: false,
            indexer_proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            config_json: Some(r#"{"base_url":"https://manager.example"}"#.to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        });
        let provider = Arc::new(RecordingPluginProvider::with_plan_sync_error(
            "managed sync preflight failed",
        ));
        let app = test_app(
            indexer_repo.clone(),
            Some(provider.clone()),
            Arc::new(NullSettingsRepository),
        );

        let updated = app
            .update_indexer_config(
                &test_admin(),
                crate::IndexerConfigUpdate {
                    id: "cfg-1".to_string(),
                    name: None,
                    provider_type: None,
                    derived_base_url: None,
                    rate_limit_seconds: None,
                    rate_limit_burst: None,
                    is_enabled: Some(true),
                    enable_interactive_search: None,
                    enable_auto_search: None,
                    indexer_proxy_config_id: None,
                    download_client_id: None,
                    seeding_profile_id: None,
                    managed_parent_config_id: None,
                    managed_child_key: None,
                    managed_metadata_json: None,
                    caps_snapshot_json: None,
                    config_json: None,
                },
            )
            .await
            .expect("managed parent should enable before background sync failure");

        assert!(updated.is_enabled);
        wait_for_plan_sync_calls(&provider, 1).await;
        let stored = indexer_repo
            .get_by_id("cfg-1")
            .await
            .unwrap()
            .expect("existing config");
        assert!(stored.is_enabled);
        assert_eq!(*provider.plan_sync_calls.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn test_indexer_connection_derives_base_url_from_feed_url() {
        let client = Arc::new(RecordingIndexerClient::new(false));
        let provider = Arc::new(RecordingPluginProvider::new(
            "torrent_rss",
            vec![string_field(
                "feed_url",
                "Feed URL",
                Some(scryer_domain::ConfigFieldRole::ConnectionUrl),
            )],
            rss_only_capabilities(),
            client.clone(),
        ));
        let app = test_app(
            Arc::new(RecordingIndexerConfigRepo::new()),
            Some(provider.clone()),
            Arc::new(NullSettingsRepository),
        );

        app.test_indexer_connection(
            &test_admin(),
            "torrent_rss",
            Some(r#"{"feed_url":"https://ipt.beelyrics.net/t.rss?u=2203846"}"#),
            None,
            None,
        )
        .await
        .unwrap();

        let seen = provider.seen_configs.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].base_url, "https://ipt.beelyrics.net");

        let calls = client.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].query.is_empty());
        assert!(calls[0].ids.is_empty());
        assert_eq!(calls[0].facet, None);
    }

    #[tokio::test]
    async fn test_indexer_connection_accepts_operator_private_lan_url() {
        let client = Arc::new(RecordingIndexerClient::new(false));
        let provider = Arc::new(RecordingPluginProvider::new(
            "newznab",
            vec![string_field(
                "base_url",
                "Base URL",
                Some(scryer_domain::ConfigFieldRole::ConnectionUrl),
            )],
            searchable_capabilities(),
            client,
        ));
        let app = test_app(
            Arc::new(RecordingIndexerConfigRepo::new()),
            Some(provider.clone()),
            Arc::new(NullSettingsRepository),
        );

        app.test_indexer_connection(
            &test_admin(),
            "newznab",
            Some(r#"{"base_url":"http://192.168.1.10:9696"}"#),
            None,
            None,
        )
        .await
        .expect("operator LAN indexer URL should test successfully");

        let seen = provider.seen_configs.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].base_url, "http://192.168.1.10:9696");
    }

    #[tokio::test]
    async fn test_indexer_connection_trims_connection_url_in_config_json() {
        let client = Arc::new(RecordingIndexerClient::new(false));
        let provider = Arc::new(RecordingPluginProvider::new(
            "newznab",
            vec![string_field(
                "base_url",
                "Base URL",
                Some(scryer_domain::ConfigFieldRole::ConnectionUrl),
            )],
            searchable_capabilities(),
            client,
        ));
        let app = test_app(
            Arc::new(RecordingIndexerConfigRepo::new()),
            Some(provider.clone()),
            Arc::new(NullSettingsRepository),
        );

        app.test_indexer_connection(
            &test_admin(),
            "newznab",
            Some(r#"{"base_url":"  https://api.nzbgeek.info/  \n"}"#),
            None,
            None,
        )
        .await
        .unwrap();

        let seen = provider.seen_configs.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].base_url, "https://api.nzbgeek.info/");
    }

    #[tokio::test]
    async fn test_indexer_connection_uses_non_empty_query_for_searchable_provider() {
        let client = Arc::new(RecordingIndexerClient::new(false));
        let provider = Arc::new(RecordingPluginProvider::new(
            "newznab",
            vec![string_field(
                "base_url",
                "Base URL",
                Some(scryer_domain::ConfigFieldRole::ConnectionUrl),
            )],
            searchable_capabilities(),
            client.clone(),
        ));
        let app = test_app(
            Arc::new(RecordingIndexerConfigRepo::new()),
            Some(provider),
            Arc::new(NullSettingsRepository),
        );

        app.test_indexer_connection(
            &test_admin(),
            "newznab",
            Some(r#"{"base_url":"https://api.nzbgeek.info/"}"#),
            None,
            None,
        )
        .await
        .unwrap();

        let calls = client.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].query, "scryer connection test");
        assert!(calls[0].ids.is_empty());
        assert_eq!(calls[0].facet, None);
    }

    #[tokio::test]
    async fn test_indexer_connection_uses_id_search_for_id_only_provider() {
        let client = Arc::new(RecordingIndexerClient::new(false));
        let provider = Arc::new(RecordingPluginProvider::new(
            "id_only",
            vec![string_field(
                "base_url",
                "Base URL",
                Some(scryer_domain::ConfigFieldRole::ConnectionUrl),
            )],
            scryer_domain::IndexerProviderCapabilities {
                rss: true,
                supported_ids: HashMap::from([("movie".into(), vec!["imdb_id".into()])]),
                query_param: None,
                search: false,
                ..Default::default()
            },
            client.clone(),
        ));
        let app = test_app(
            Arc::new(RecordingIndexerConfigRepo::new()),
            Some(provider),
            Arc::new(NullSettingsRepository),
        );

        app.test_indexer_connection(
            &test_admin(),
            "id_only",
            Some(r#"{"base_url":"https://example.invalid"}"#),
            None,
            None,
        )
        .await
        .unwrap();

        let calls = client.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].query.is_empty());
        assert_eq!(
            calls[0].ids,
            HashMap::from([("imdb_id".to_string(), "tt0000001".to_string())])
        );
        assert_eq!(calls[0].facet.as_deref(), Some("movie"));
    }

    #[tokio::test]
    async fn test_indexer_connection_propagates_search_failures() {
        let client = Arc::new(RecordingIndexerClient::new(true));
        let provider = Arc::new(RecordingPluginProvider::new(
            "newznab",
            vec![string_field(
                "base_url",
                "Base URL",
                Some(scryer_domain::ConfigFieldRole::ConnectionUrl),
            )],
            searchable_capabilities(),
            client,
        ));
        let app = test_app(
            Arc::new(RecordingIndexerConfigRepo::new()),
            Some(provider),
            Arc::new(NullSettingsRepository),
        );

        let error = app
            .test_indexer_connection(
                &test_admin(),
                "newznab",
                Some(r#"{"base_url":"https://api.nzbgeek.info/"}"#),
                None,
                None,
            )
            .await
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "repository: indexer connection test failed: repository: forced failure"
        );
    }

    /// The probe runs under a synthetic indexer id with no `indexers` row, so
    /// error history can never be written for it. The capture paths ask
    /// `indexer_error_history_is_persistable` before writing, which is what
    /// keeps a foreign-key failure from standing in for the probe's own error.
    #[tokio::test]
    async fn a_failing_connection_test_reports_the_probe_error_not_a_storage_error() {
        assert!(
            !crate::indexer_error_history_is_persistable(crate::CONNECTION_TEST_INDEXER_ID),
            "the connection-test id must be excluded from error history"
        );
        assert!(
            crate::indexer_error_history_is_persistable("indexer-1"),
            "a stored indexer id still records history"
        );

        let client = Arc::new(RecordingIndexerClient::with_search_error(Some(
            "plugin scryer_indexer_search() failed: connection refused".to_string(),
        )));
        let provider = Arc::new(RecordingPluginProvider::new(
            "newznab",
            vec![string_field(
                "base_url",
                "Base URL",
                Some(scryer_domain::ConfigFieldRole::ConnectionUrl),
            )],
            searchable_capabilities(),
            client,
        ));
        let app = test_app(
            Arc::new(RecordingIndexerConfigRepo::new()),
            Some(provider),
            Arc::new(NullSettingsRepository),
        );

        let error = app
            .test_indexer_connection(
                &test_admin(),
                "newznab",
                Some(r#"{"base_url":"https://api.nzbgeek.info/"}"#),
                None,
                None,
            )
            .await
            .unwrap_err()
            .to_string();

        assert!(
            error.contains("connection refused"),
            "the probe's own error must survive: {error}"
        );
        assert!(
            !error.to_ascii_lowercase().contains("foreign key"),
            "a storage failure must never replace the probe error: {error}"
        );
    }

    #[tokio::test]
    async fn test_indexer_connection_surfaces_invalid_api_key_search_failures() {
        let client = Arc::new(RecordingIndexerClient::with_search_error(Some(
            "plugin scryer_indexer_search() failed: Newznab API key error 100: Invalid API Key"
                .to_string(),
        )));
        let provider = Arc::new(RecordingPluginProvider::new(
            "newznab",
            vec![string_field(
                "base_url",
                "Base URL",
                Some(scryer_domain::ConfigFieldRole::ConnectionUrl),
            )],
            searchable_capabilities(),
            client,
        ));
        let app = test_app(
            Arc::new(RecordingIndexerConfigRepo::new()),
            Some(provider),
            Arc::new(NullSettingsRepository),
        );

        let error = app
            .test_indexer_connection(
                &test_admin(),
                "newznab",
                Some(r#"{"base_url":"https://api.nzbgeek.info/"}"#),
                None,
                None,
            )
            .await
            .unwrap_err();

        assert_eq!(error.to_string(), "validation: Invalid API Key");
    }

    #[tokio::test]
    async fn test_indexer_connection_uses_validate_config_when_supported() {
        let provider = Arc::new(RecordingPluginProvider::with_validate_config_support());
        let app = test_app(
            Arc::new(RecordingIndexerConfigRepo::new()),
            Some(provider.clone()),
            Arc::new(NullSettingsRepository),
        );

        app.test_indexer_connection(
            &test_admin(),
            "torrent_rss",
            Some(r#"{"feed_url":"https://ipt.beelyrics.net/t.rss?u=2203846"}"#),
            None,
            None,
        )
        .await
        .unwrap();

        let seen_search = provider.seen_configs.lock().unwrap();
        assert!(seen_search.is_empty());

        let seen_management = provider.seen_management_configs.lock().unwrap();
        assert_eq!(seen_management.len(), 1);
        assert_eq!(seen_management[0].base_url, "https://ipt.beelyrics.net");
        assert_eq!(*provider.plan_sync_calls.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn preview_managed_indexer_children_uses_preview_plan_without_full_sync() {
        let sync_plan = crate::IndexerSyncPlan {
            children: vec![crate::ManagedIndexerChildPlan {
                child_key: "preview-child".to_string(),
                name: "Preview Child".to_string(),
                provider_type: "torrent_rss".to_string(),
                config_json: r#"{"feed_url":"https://preview.example/rss"}"#.to_string(),
                is_enabled: true,
                enable_interactive_search: true,
                enable_auto_search: true,
                managed_metadata_json: None,
                caps_snapshot_json: None,
                routing_scopes: Vec::new(),
            }],
        };
        let provider = Arc::new(RecordingPluginProvider::with_sync_plan(sync_plan));
        let app = test_app(
            Arc::new(RecordingIndexerConfigRepo::new()),
            Some(provider.clone()),
            Arc::new(NullSettingsRepository),
        );

        let (_validation, plan) = app
            .preview_managed_indexer_children(
                &test_admin(),
                "manager",
                Some(r#"{"base_url":"https://manager.example"}"#),
            )
            .await
            .expect("preview plan");

        assert_eq!(plan.children.len(), 1);
        assert_eq!(*provider.preview_sync_plan_calls.lock().unwrap(), 1);
        assert_eq!(*provider.plan_sync_calls.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn create_indexer_config_queues_background_sync_for_managed_children() {
        let indexer_repo = Arc::new(RecordingIndexerConfigRepo::new());
        let sync_plan = crate::IndexerSyncPlan {
            children: vec![crate::ManagedIndexerChildPlan {
                child_key: "new".to_string(),
                name: "Managed New".to_string(),
                provider_type: "torrent_rss".to_string(),
                config_json: r#"{"feed_url":"https://new.example/rss"}"#.to_string(),
                is_enabled: true,
                enable_interactive_search: false,
                enable_auto_search: true,
                managed_metadata_json: Some("{\"source\":\"new\"}".to_string()),
                caps_snapshot_json: None,
                routing_scopes: vec![crate::ManagedIndexerRoutingScope {
                    scope_id: "anime".to_string(),
                    categories: vec!["5070".to_string()],
                }],
            }],
        };
        let provider = Arc::new(RecordingPluginProvider::with_sync_plan(sync_plan));
        let app = test_app(
            indexer_repo.clone(),
            Some(provider.clone()),
            Arc::new(RecordingSettingsRepository::default()),
        );

        let created = app
            .create_indexer_config(
                &test_admin(),
                NewIndexerConfig {
                    name: "Parent Manager".to_string(),
                    provider_type: "manager".to_string(),
                    rate_limit_seconds: None,
                    rate_limit_burst: None,
                    is_enabled: true,
                    enable_interactive_search: true,
                    enable_auto_search: true,
                    indexer_proxy_config_id: None,
                    download_client_id: None,
                    config_json: Some(r#"{"base_url":"https://manager.example"}"#.to_string()),
                },
            )
            .await
            .expect("managed parent should queue background sync");

        wait_for_plan_sync_calls(&provider, 1).await;
        let configs = indexer_repo.list(None).await.unwrap();
        assert_eq!(configs.len(), 2);
        assert!(configs.iter().any(|config| config.id == created.id));
        let child = configs
            .iter()
            .find(|config| config.managed_child_key.as_deref() == Some("new"))
            .expect("managed child created");
        assert_eq!(
            child.managed_parent_config_id.as_deref(),
            Some(created.id.as_str())
        );
        assert_eq!(*provider.plan_sync_calls.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn create_indexer_config_keeps_managed_parent_when_background_sync_fails() {
        let indexer_repo = Arc::new(RecordingIndexerConfigRepo::new());
        let provider = Arc::new(RecordingPluginProvider::with_plan_sync_error(
            "managed sync failed after create",
        ));
        let app = test_app(
            indexer_repo.clone(),
            Some(provider.clone()),
            Arc::new(RecordingSettingsRepository::default()),
        );

        let created = app
            .create_indexer_config(
                &test_admin(),
                NewIndexerConfig {
                    name: "Parent Manager".to_string(),
                    provider_type: "manager".to_string(),
                    rate_limit_seconds: None,
                    rate_limit_burst: None,
                    is_enabled: true,
                    enable_interactive_search: true,
                    enable_auto_search: true,
                    indexer_proxy_config_id: None,
                    download_client_id: None,
                    config_json: Some(r#"{"base_url":"https://manager.example"}"#.to_string()),
                },
            )
            .await
            .expect("create should keep parent when background sync fails");

        wait_for_plan_sync_calls(&provider, 1).await;
        let configs = indexer_repo.list(None).await.unwrap();
        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].id, created.id);
        assert_eq!(*provider.plan_sync_calls.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn managed_parent_providers_disable_search_modes_on_create_and_update() {
        let indexer_repo = Arc::new(RecordingIndexerConfigRepo::new());
        let app = test_app(
            indexer_repo,
            Some(Arc::new(RecordingPluginProvider::with_sync_plan(
                crate::IndexerSyncPlan::default(),
            ))),
            Arc::new(NullSettingsRepository),
        );

        let created = app
            .create_indexer_config(
                &test_admin(),
                NewIndexerConfig {
                    name: "Parent Manager".to_string(),
                    provider_type: "manager".to_string(),
                    rate_limit_seconds: None,
                    rate_limit_burst: None,
                    is_enabled: true,
                    enable_interactive_search: true,
                    enable_auto_search: true,
                    indexer_proxy_config_id: None,
                    download_client_id: None,
                    config_json: Some(r#"{"base_url":"https://manager.example"}"#.to_string()),
                },
            )
            .await
            .unwrap();

        assert!(!created.enable_interactive_search);
        assert!(!created.enable_auto_search);

        let updated = app
            .update_indexer_config(
                &test_admin(),
                IndexerConfigUpdate {
                    id: created.id.clone(),
                    name: Some("Renamed Parent Manager".to_string()),
                    enable_interactive_search: Some(true),
                    enable_auto_search: Some(true),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(updated.name, "Renamed Parent Manager");
        assert!(!updated.enable_interactive_search);
        assert!(!updated.enable_auto_search);
    }

    #[tokio::test]
    async fn disabling_managed_parent_disables_existing_children_without_syncing() {
        let indexer_repo = Arc::new(RecordingIndexerConfigRepo::new());
        let now = Utc::now();
        indexer_repo
            .create(IndexerConfig {
                id: "parent".to_string(),
                name: "Parent Manager".to_string(),
                provider_type: "manager".to_string(),
                base_url: "https://manager.example".to_string(),
                api_key_encrypted: None,
                rate_limit_seconds: None,
                rate_limit_burst: None,
                disabled_until: None,
                is_enabled: true,
                enable_interactive_search: false,
                enable_auto_search: false,
                indexer_proxy_config_id: None,
                download_client_id: None,
                seeding_profile_id: None,
                managed_parent_config_id: None,
                managed_child_key: None,
                managed_metadata_json: None,
                caps_snapshot_json: None,
                last_health_status: None,
                last_error_message: None,
                last_error_at: None,
                config_json: Some(r#"{"base_url":"https://manager.example"}"#.to_string()),
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        indexer_repo
            .create(IndexerConfig {
                id: "child".to_string(),
                name: "Managed Child".to_string(),
                provider_type: "torrent_rss".to_string(),
                base_url: "https://child.example".to_string(),
                api_key_encrypted: None,
                rate_limit_seconds: None,
                rate_limit_burst: None,
                disabled_until: None,
                is_enabled: true,
                enable_interactive_search: true,
                enable_auto_search: true,
                indexer_proxy_config_id: None,
                download_client_id: None,
                seeding_profile_id: None,
                managed_parent_config_id: Some("parent".to_string()),
                managed_child_key: Some("child".to_string()),
                managed_metadata_json: None,
                caps_snapshot_json: None,
                last_health_status: None,
                last_error_message: None,
                last_error_at: None,
                config_json: Some(r#"{"feed_url":"https://child.example/rss"}"#.to_string()),
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();

        let provider = Arc::new(RecordingPluginProvider::with_sync_plan(
            crate::IndexerSyncPlan::default(),
        ));
        let app = test_app(
            indexer_repo.clone(),
            Some(provider.clone()),
            Arc::new(NullSettingsRepository),
        );

        let updated = app
            .update_indexer_config(
                &test_admin(),
                IndexerConfigUpdate {
                    id: "parent".to_string(),
                    is_enabled: Some(false),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert!(!updated.is_enabled);
        let child = indexer_repo.get_by_id("child").await.unwrap().unwrap();
        assert!(!child.is_enabled);
        assert_eq!(*provider.plan_sync_calls.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn managed_child_local_disable_survives_sync_until_reenabled() {
        let indexer_repo = Arc::new(RecordingIndexerConfigRepo::new());
        let now = Utc::now();
        indexer_repo
            .create(IndexerConfig {
                id: "parent".to_string(),
                name: "Parent Manager".to_string(),
                provider_type: "manager".to_string(),
                base_url: "https://manager.example".to_string(),
                api_key_encrypted: None,
                rate_limit_seconds: None,
                rate_limit_burst: None,
                disabled_until: None,
                is_enabled: true,
                enable_interactive_search: false,
                enable_auto_search: false,
                indexer_proxy_config_id: None,
                download_client_id: None,
                seeding_profile_id: None,
                managed_parent_config_id: None,
                managed_child_key: None,
                managed_metadata_json: Some(
                    r#"{"locally_disabled_children":["keep","upstream-disabled"]}"#.to_string(),
                ),
                caps_snapshot_json: None,
                last_health_status: None,
                last_error_message: None,
                last_error_at: None,
                config_json: Some(r#"{"base_url":"https://manager.example"}"#.to_string()),
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        indexer_repo
            .create(IndexerConfig {
                id: "child".to_string(),
                name: "Managed Child".to_string(),
                provider_type: "torrent_rss".to_string(),
                base_url: "https://child.example".to_string(),
                api_key_encrypted: None,
                rate_limit_seconds: None,
                rate_limit_burst: None,
                disabled_until: None,
                is_enabled: false,
                enable_interactive_search: false,
                enable_auto_search: false,
                indexer_proxy_config_id: None,
                download_client_id: None,
                seeding_profile_id: None,
                managed_parent_config_id: Some("parent".to_string()),
                managed_child_key: Some("keep".to_string()),
                managed_metadata_json: None,
                caps_snapshot_json: None,
                last_health_status: None,
                last_error_message: None,
                last_error_at: None,
                config_json: Some(r#"{"feed_url":"https://child.example/rss"}"#.to_string()),
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();

        let mut upstream_disabled_child = indexer_repo.get_by_id("child").await.unwrap().unwrap();
        upstream_disabled_child.id = "upstream-disabled-child".to_string();
        upstream_disabled_child.name = "Upstream Disabled Child".to_string();
        upstream_disabled_child.managed_child_key = Some("upstream-disabled".to_string());
        indexer_repo.create(upstream_disabled_child).await.unwrap();

        let removal_app = test_app(
            indexer_repo.clone(),
            Some(Arc::new(RecordingPluginProvider::with_sync_plan(
                crate::IndexerSyncPlan::default(),
            ))),
            Arc::new(NullSettingsRepository),
        );
        removal_app
            .sync_indexer_config(&test_admin(), "parent")
            .await
            .unwrap();
        assert!(indexer_repo.get_by_id("child").await.unwrap().is_none());
        assert!(
            indexer_repo
                .get_by_id("upstream-disabled-child")
                .await
                .unwrap()
                .is_none()
        );

        let enabled_child_plan = crate::ManagedIndexerChildPlan {
            child_key: "keep".to_string(),
            name: "Managed Child".to_string(),
            provider_type: "torrent_rss".to_string(),
            config_json: r#"{"feed_url":"https://child.example/rss"}"#.to_string(),
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: false,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            routing_scopes: vec![],
        };
        let mut upstream_disabled_plan = enabled_child_plan.clone();
        upstream_disabled_plan.child_key = "upstream-disabled".to_string();
        upstream_disabled_plan.name = "Upstream Disabled Child".to_string();
        upstream_disabled_plan.is_enabled = false;
        let sync_plan = crate::IndexerSyncPlan {
            children: vec![enabled_child_plan, upstream_disabled_plan],
        };
        let provider = Arc::new(RecordingPluginProvider::with_sync_plan(sync_plan));
        let app = test_app(
            indexer_repo.clone(),
            Some(provider.clone()),
            Arc::new(NullSettingsRepository),
        );

        app.sync_indexer_config(&test_admin(), "parent")
            .await
            .unwrap();
        let child = indexer_repo
            .list(None)
            .await
            .unwrap()
            .into_iter()
            .find(|config| config.managed_child_key.as_deref() == Some("keep"))
            .unwrap();
        let child_id = child.id.clone();
        assert!(!child.is_enabled);
        assert!(child.enable_interactive_search);
        assert!(!child.enable_auto_search);
        let upstream_disabled_child_id = indexer_repo
            .list(None)
            .await
            .unwrap()
            .into_iter()
            .find(|config| config.managed_child_key.as_deref() == Some("upstream-disabled"))
            .unwrap()
            .id;

        let reenabled = app
            .update_indexer_config(
                &test_admin(),
                IndexerConfigUpdate {
                    id: child_id,
                    is_enabled: Some(true),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(reenabled.is_enabled);
        assert_eq!(reenabled.managed_metadata_json, None);
        assert_eq!(
            indexer_repo
                .get_by_id("parent")
                .await
                .unwrap()
                .unwrap()
                .managed_metadata_json
                .as_deref(),
            Some(r#"{"locally_disabled_children":["upstream-disabled"]}"#)
        );

        let still_disabled = app
            .update_indexer_config(
                &test_admin(),
                IndexerConfigUpdate {
                    id: upstream_disabled_child_id,
                    is_enabled: Some(true),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(!still_disabled.is_enabled);
        assert_eq!(still_disabled.managed_metadata_json, None);
        assert_eq!(
            indexer_repo
                .get_by_id("parent")
                .await
                .unwrap()
                .unwrap()
                .managed_metadata_json,
            None
        );
        assert_eq!(*provider.plan_sync_calls.lock().unwrap(), 3);
    }

    #[tokio::test]
    async fn updating_enabled_managed_parent_config_queues_background_sync() {
        let indexer_repo = Arc::new(RecordingIndexerConfigRepo::new());
        let now = Utc::now();
        indexer_repo
            .create(IndexerConfig {
                id: "parent".to_string(),
                name: "Parent Manager".to_string(),
                provider_type: "manager".to_string(),
                base_url: "https://manager.example".to_string(),
                api_key_encrypted: None,
                rate_limit_seconds: None,
                rate_limit_burst: None,
                disabled_until: None,
                is_enabled: true,
                enable_interactive_search: false,
                enable_auto_search: false,
                indexer_proxy_config_id: None,
                download_client_id: None,
                seeding_profile_id: None,
                managed_parent_config_id: None,
                managed_child_key: None,
                managed_metadata_json: None,
                caps_snapshot_json: None,
                last_health_status: None,
                last_error_message: None,
                last_error_at: None,
                config_json: Some(r#"{"base_url":"https://manager.example"}"#.to_string()),
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();

        let provider = Arc::new(RecordingPluginProvider::with_sync_plan(
            crate::IndexerSyncPlan::default(),
        ));
        let app = test_app(
            indexer_repo,
            Some(provider.clone()),
            Arc::new(NullSettingsRepository),
        );

        let updated = app
            .update_indexer_config(
                &test_admin(),
                IndexerConfigUpdate {
                    id: "parent".to_string(),
                    config_json: Some(
                        r#"{"base_url":"https://manager.changed.example"}"#.to_string(),
                    ),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(updated.base_url, "https://manager.changed.example");
        wait_for_plan_sync_calls(&provider, 1).await;
        assert_eq!(*provider.plan_sync_calls.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn sync_enabled_prowlarr_indexers_skips_disabled_parents() {
        let indexer_repo = Arc::new(RecordingIndexerConfigRepo::new());
        let now = Utc::now();
        indexer_repo
            .create(IndexerConfig {
                id: "enabled-parent".to_string(),
                name: "Enabled Prowlarr".to_string(),
                provider_type: "prowlarr".to_string(),
                base_url: "http://prowlarr.local".to_string(),
                api_key_encrypted: None,
                rate_limit_seconds: None,
                rate_limit_burst: None,
                disabled_until: None,
                is_enabled: true,
                enable_interactive_search: false,
                enable_auto_search: false,
                indexer_proxy_config_id: None,
                download_client_id: None,
                seeding_profile_id: None,
                managed_parent_config_id: None,
                managed_child_key: None,
                managed_metadata_json: None,
                caps_snapshot_json: None,
                last_health_status: None,
                last_error_message: None,
                last_error_at: None,
                config_json: Some(
                    r#"{"base_url":"http://prowlarr.local","api_key":"secret"}"#.to_string(),
                ),
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        indexer_repo
            .create(IndexerConfig {
                id: "disabled-parent".to_string(),
                name: "Disabled Prowlarr".to_string(),
                provider_type: "prowlarr".to_string(),
                base_url: "http://prowlarr.disabled".to_string(),
                api_key_encrypted: None,
                rate_limit_seconds: None,
                rate_limit_burst: None,
                disabled_until: None,
                is_enabled: false,
                enable_interactive_search: false,
                enable_auto_search: false,
                indexer_proxy_config_id: None,
                download_client_id: None,
                seeding_profile_id: None,
                managed_parent_config_id: None,
                managed_child_key: None,
                managed_metadata_json: None,
                caps_snapshot_json: None,
                last_health_status: None,
                last_error_message: None,
                last_error_at: None,
                config_json: Some(
                    r#"{"base_url":"http://prowlarr.disabled","api_key":"secret"}"#.to_string(),
                ),
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();

        let provider = Arc::new(RecordingPluginProvider::with_sync_plan_for_provider(
            "prowlarr",
            crate::IndexerSyncPlan::default(),
        ));
        let app = test_app(
            indexer_repo,
            Some(provider.clone()),
            Arc::new(RecordingSettingsRepository::default()),
        );

        let (synced_count, failures) = app
            .sync_enabled_prowlarr_indexers(&test_admin())
            .await
            .unwrap();

        assert_eq!(synced_count, 1);
        assert!(failures.is_empty());
        assert_eq!(*provider.plan_sync_calls.lock().unwrap(), 1);
        let seen = provider.seen_management_configs.lock().unwrap().clone();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].id, "enabled-parent");
    }

    #[tokio::test]
    async fn sync_indexer_config_serializes_concurrent_runs_globally() {
        let indexer_repo = Arc::new(RecordingIndexerConfigRepo::new());
        let now = Utc::now();
        indexer_repo
            .create(IndexerConfig {
                id: "parent-a".to_string(),
                name: "Parent Manager A".to_string(),
                provider_type: "manager".to_string(),
                base_url: "https://manager.example".to_string(),
                api_key_encrypted: None,
                rate_limit_seconds: None,
                rate_limit_burst: None,
                disabled_until: None,
                is_enabled: true,
                enable_interactive_search: false,
                enable_auto_search: false,
                indexer_proxy_config_id: None,
                download_client_id: None,
                seeding_profile_id: None,
                managed_parent_config_id: None,
                managed_child_key: None,
                managed_metadata_json: None,
                caps_snapshot_json: None,
                last_health_status: None,
                last_error_message: None,
                last_error_at: None,
                config_json: Some(r#"{"base_url":"https://manager.example"}"#.to_string()),
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        indexer_repo
            .create(IndexerConfig {
                id: "parent-b".to_string(),
                name: "Parent Manager B".to_string(),
                provider_type: "manager".to_string(),
                base_url: "https://manager.example".to_string(),
                api_key_encrypted: None,
                rate_limit_seconds: None,
                rate_limit_burst: None,
                disabled_until: None,
                is_enabled: true,
                enable_interactive_search: false,
                enable_auto_search: false,
                indexer_proxy_config_id: None,
                download_client_id: None,
                seeding_profile_id: None,
                managed_parent_config_id: None,
                managed_child_key: None,
                managed_metadata_json: None,
                caps_snapshot_json: None,
                last_health_status: None,
                last_error_message: None,
                last_error_at: None,
                config_json: Some(r#"{"base_url":"https://manager.example"}"#.to_string()),
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();

        let provider = Arc::new(RecordingPluginProvider::with_delayed_sync_plan(
            crate::IndexerSyncPlan::default(),
            Duration::from_millis(50),
        ));
        let app = test_app(
            indexer_repo,
            Some(provider.clone()),
            Arc::new(RecordingSettingsRepository::default()),
        );
        let actor = test_admin();

        let (first, second) = tokio::join!(
            app.sync_indexer_config(&actor, "parent-a"),
            app.sync_indexer_config(&actor, "parent-b"),
        );

        first.unwrap();
        second.unwrap();
        assert_eq!(*provider.plan_sync_calls.lock().unwrap(), 2);
        assert_eq!(
            provider
                .max_concurrent_plan_sync_calls
                .load(Ordering::SeqCst),
            1
        );
    }

    #[tokio::test]
    async fn routing_update_waits_for_managed_sync_and_wins() {
        let indexer_repo = Arc::new(RecordingIndexerConfigRepo::new());
        let now = Utc::now();
        indexer_repo
            .create(IndexerConfig {
                id: "parent".to_string(),
                name: "Parent Manager".to_string(),
                provider_type: "manager".to_string(),
                base_url: "https://manager.example".to_string(),
                api_key_encrypted: None,
                rate_limit_seconds: None,
                rate_limit_burst: None,
                disabled_until: None,
                is_enabled: true,
                enable_interactive_search: false,
                enable_auto_search: false,
                indexer_proxy_config_id: None,
                download_client_id: None,
                seeding_profile_id: None,
                managed_parent_config_id: None,
                managed_child_key: None,
                managed_metadata_json: None,
                caps_snapshot_json: None,
                last_health_status: None,
                last_error_message: None,
                last_error_at: None,
                config_json: Some(r#"{"base_url":"https://manager.example"}"#.to_string()),
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();

        let provider = Arc::new(RecordingPluginProvider::with_sync_plan(
            crate::IndexerSyncPlan::default(),
        ));
        let (settings_repo, barrier) = RecordingSettingsRepository::with_system_upsert_barrier();
        let app = test_app(indexer_repo, Some(provider), Arc::new(settings_repo));
        app.update_indexer_routing(
            &test_admin(),
            "movie",
            vec![IndexerRoutingSettingsEntry {
                indexer_id: "managed-child".to_string(),
                enabled: true,
                categories: vec!["2000".to_string()],
                priority: 1,
            }],
        )
        .await
        .unwrap();

        let sync_app = app.clone();
        let sync_task = tokio::spawn(async move {
            sync_app
                .sync_indexer_config(&scryer_domain::User::system_execution_actor(), "parent")
                .await
        });
        barrier
            .entered
            .acquire()
            .await
            .expect("settings upsert barrier should remain open")
            .forget();

        let admin = test_admin();
        let update = app.update_indexer_routing(
            &admin,
            "movie",
            vec![IndexerRoutingSettingsEntry {
                indexer_id: "managed-child".to_string(),
                enabled: false,
                categories: vec!["2000".to_string()],
                priority: 1,
            }],
        );
        tokio::pin!(update);
        let first_poll = std::future::poll_fn(|cx| {
            std::task::Poll::Ready(std::future::Future::poll(update.as_mut(), cx))
        });
        assert!(first_poll.await.is_pending());

        barrier.release.add_permits(1);
        sync_task.await.unwrap().unwrap();
        update.await.unwrap();

        let routing = app
            .get_indexer_routing(&test_admin(), "movie")
            .await
            .unwrap();
        let managed_child = routing
            .iter()
            .find(|entry| entry.indexer_id == "managed-child")
            .expect("managed child routing entry");
        assert!(!managed_child.enabled);
    }

    #[tokio::test]
    async fn sync_indexer_config_publishes_indexers_changed_after_partial_failure() {
        let indexer_repo = Arc::new(RecordingIndexerConfigRepo::new());
        let now = Utc::now();
        let parent = IndexerConfig {
            id: "parent".to_string(),
            name: "Parent Manager".to_string(),
            provider_type: "manager".to_string(),
            base_url: "https://manager.example".to_string(),
            api_key_encrypted: None,
            rate_limit_seconds: None,
            rate_limit_burst: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: false,
            enable_auto_search: false,
            indexer_proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            config_json: Some(r#"{"base_url":"https://manager.example"}"#.to_string()),
            created_at: now,
            updated_at: now,
        };
        indexer_repo.create(parent.clone()).await.unwrap();

        let sync_plan = crate::IndexerSyncPlan {
            children: vec![crate::ManagedIndexerChildPlan {
                child_key: "new".to_string(),
                name: "Managed New".to_string(),
                provider_type: "torrent_rss".to_string(),
                config_json: r#"{"feed_url":"https://new.example/rss"}"#.to_string(),
                is_enabled: true,
                enable_interactive_search: false,
                enable_auto_search: true,
                managed_metadata_json: Some("{\"source\":\"new\"}".to_string()),
                caps_snapshot_json: None,
                routing_scopes: vec![crate::ManagedIndexerRoutingScope {
                    scope_id: "movie".to_string(),
                    categories: vec!["2000".to_string()],
                }],
            }],
        };
        let provider = Arc::new(RecordingPluginProvider::with_sync_plan(sync_plan));
        let app = test_app(
            indexer_repo.clone(),
            Some(provider),
            Arc::new(RecordingSettingsRepository::with_upsert_failure()),
        );
        let mut receiver = app.runtime.events.indexers_changed_broadcast.subscribe();

        let error = app
            .sync_indexer_config(&test_admin(), &parent.id)
            .await
            .expect_err("routing save should fail after child reconciliation");

        assert_eq!(
            error.to_string(),
            "repository: forced settings write failure"
        );
        expect_indexers_changed(&mut receiver, "partial sync failure").await;
        assert!(
            indexer_repo
                .list(None)
                .await
                .unwrap()
                .iter()
                .any(|config| config.managed_child_key.as_deref() == Some("new"))
        );
    }

    #[tokio::test]
    async fn sync_indexer_config_reconciles_managed_children_and_routing() {
        let indexer_repo = Arc::new(RecordingIndexerConfigRepo::new());
        let now = Utc::now();
        let parent = IndexerConfig {
            id: "parent".to_string(),
            name: "Parent Manager".to_string(),
            provider_type: "manager".to_string(),
            base_url: "https://manager.example".to_string(),
            api_key_encrypted: None,
            rate_limit_seconds: None,
            rate_limit_burst: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            indexer_proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: Some(r#"{"locally_disabled_children":["keep"]}"#.to_string()),
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            config_json: Some("{}".to_string()),
            created_at: now,
            updated_at: now,
        };
        let existing_keep = IndexerConfig {
            id: "managed-keep".to_string(),
            name: "Old Keep".to_string(),
            provider_type: "torrent_rss".to_string(),
            base_url: "https://old.example".to_string(),
            api_key_encrypted: None,
            rate_limit_seconds: None,
            rate_limit_burst: None,
            disabled_until: None,
            is_enabled: false,
            enable_interactive_search: false,
            enable_auto_search: false,
            indexer_proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: Some(parent.id.clone()),
            managed_child_key: Some("keep".to_string()),
            managed_metadata_json: Some(
                serde_json::json!({
                    "old": true,
                    "caps_snapshot": {
                        "search": {
                            "available": true,
                            "supported_params": ["q"]
                        }
                    }
                })
                .to_string(),
            ),
            caps_snapshot_json: Some(r#"{"search":{"available":true}}"#.to_string()),
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            config_json: Some(r#"{"feed_url":"https://old.example/rss"}"#.to_string()),
            created_at: now,
            updated_at: now,
        };
        let existing_delete = IndexerConfig {
            id: "managed-delete".to_string(),
            name: "Delete Me".to_string(),
            provider_type: "torrent_rss".to_string(),
            base_url: "https://delete.example".to_string(),
            api_key_encrypted: None,
            rate_limit_seconds: None,
            rate_limit_burst: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            indexer_proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: Some(parent.id.clone()),
            managed_child_key: Some("delete".to_string()),
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            config_json: Some(r#"{"feed_url":"https://delete.example/rss"}"#.to_string()),
            created_at: now,
            updated_at: now,
        };
        indexer_repo.create(parent.clone()).await.unwrap();
        indexer_repo.create(existing_keep).await.unwrap();
        indexer_repo.create(existing_delete).await.unwrap();

        let sync_plan = crate::IndexerSyncPlan {
            children: vec![
                crate::ManagedIndexerChildPlan {
                    child_key: "keep".to_string(),
                    name: "Managed Keep".to_string(),
                    provider_type: "torrent_rss".to_string(),
                    config_json: r#"{"feed_url":"https://keep.example/rss"}"#.to_string(),
                    is_enabled: true,
                    enable_interactive_search: true,
                    enable_auto_search: false,
                    managed_metadata_json: Some("{\"source\":\"keep\"}".to_string()),
                    caps_snapshot_json: None,
                    routing_scopes: vec![
                        crate::ManagedIndexerRoutingScope {
                            scope_id: "movie".to_string(),
                            categories: vec!["2000".to_string()],
                        },
                        crate::ManagedIndexerRoutingScope {
                            scope_id: "series".to_string(),
                            categories: vec!["5000".to_string()],
                        },
                    ],
                },
                crate::ManagedIndexerChildPlan {
                    child_key: "new".to_string(),
                    name: "Managed New".to_string(),
                    provider_type: "torrent_rss".to_string(),
                    config_json: r#"{"feed_url":"https://new.example/rss"}"#.to_string(),
                    is_enabled: true,
                    enable_interactive_search: false,
                    enable_auto_search: true,
                    managed_metadata_json: Some("{\"source\":\"new\"}".to_string()),
                    caps_snapshot_json: None,
                    routing_scopes: vec![crate::ManagedIndexerRoutingScope {
                        scope_id: "anime".to_string(),
                        categories: vec!["5070".to_string(), "5070".to_string()],
                    }],
                },
            ],
        };
        let provider = Arc::new(RecordingPluginProvider::with_sync_plan(sync_plan));
        let app = test_app(
            indexer_repo.clone(),
            Some(provider.clone()),
            Arc::new(RecordingSettingsRepository::default()),
        );

        app.update_indexer_routing(
            &test_admin(),
            "movie",
            vec![IndexerRoutingSettingsEntry {
                indexer_id: "managed-keep".to_string(),
                enabled: false,
                categories: vec!["1111".to_string()],
                priority: 17,
            }],
        )
        .await
        .unwrap();
        app.update_indexer_routing(
            &test_admin(),
            "anime",
            vec![
                IndexerRoutingSettingsEntry {
                    indexer_id: "managed-keep".to_string(),
                    enabled: true,
                    categories: vec!["9999".to_string()],
                    priority: 3,
                },
                IndexerRoutingSettingsEntry {
                    indexer_id: "managed-delete".to_string(),
                    enabled: true,
                    categories: vec!["8888".to_string()],
                    priority: 4,
                },
            ],
        )
        .await
        .unwrap();

        let mut receiver = app.runtime.events.indexers_changed_broadcast.subscribe();
        let result = app
            .sync_indexer_config(&test_admin(), &parent.id)
            .await
            .unwrap();

        expect_indexers_changed(&mut receiver, "sync_indexer_config").await;
        assert_eq!(result.parent_config_id, parent.id);
        assert_eq!(result.created_ids.len(), 1);
        assert_eq!(result.updated_ids, vec!["managed-keep".to_string()]);
        assert_eq!(result.deleted_ids, vec!["managed-delete".to_string()]);

        let configs = indexer_repo.list(None).await.unwrap();
        assert_eq!(configs.len(), 3);
        let synced_parent = configs
            .iter()
            .find(|config| config.id == parent.id)
            .unwrap();
        assert!(!synced_parent.enable_interactive_search);
        assert!(!synced_parent.enable_auto_search);
        assert_eq!(
            synced_parent.managed_metadata_json.as_deref(),
            Some(r#"{"locally_disabled_children":["keep"]}"#)
        );
        let keep = configs
            .iter()
            .find(|config| config.id == "managed-keep")
            .unwrap();
        assert_eq!(keep.name, "Managed Keep");
        assert!(!keep.is_enabled);
        assert_eq!(keep.base_url, "https://keep.example");
        assert_eq!(keep.managed_child_key.as_deref(), Some("keep"));
        let keep_metadata: serde_json::Value = serde_json::from_str(
            keep.managed_metadata_json
                .as_deref()
                .expect("managed keep metadata"),
        )
        .unwrap();
        assert_eq!(keep_metadata["source"], "keep");
        assert!(keep_metadata["locally_disabled"].is_null());
        assert_eq!(keep_metadata["caps_snapshot"]["search"]["available"], true);
        assert_eq!(
            keep_metadata["caps_snapshot"]["search"]["supported_params"],
            serde_json::json!(["q"])
        );
        assert_eq!(
            keep.caps_snapshot_json.as_deref(),
            Some(r#"{"search":{"available":true}}"#),
            "a sync plan without caps must not clear the stored snapshot column"
        );
        assert!(
            configs
                .iter()
                .any(|config| config.managed_child_key.as_deref() == Some("new"))
        );
        assert!(!configs.iter().any(|config| config.id == "managed-delete"));

        let movie = app
            .get_indexer_routing(&test_admin(), "movie")
            .await
            .unwrap();
        let keep_movie = movie
            .iter()
            .find(|entry| entry.indexer_id == "managed-keep")
            .unwrap();
        assert!(!keep_movie.enabled);
        assert_eq!(keep_movie.categories, vec!["2000"]);
        assert_eq!(keep_movie.priority, 17);

        let series = app
            .get_indexer_routing(&test_admin(), "series")
            .await
            .unwrap();
        assert!(series.iter().any(|entry| {
            entry.indexer_id == "managed-keep" && entry.enabled && entry.categories == vec!["5000"]
        }));

        let anime = app
            .get_indexer_routing(&test_admin(), "anime")
            .await
            .unwrap();
        assert!(!anime.iter().any(|entry| entry.indexer_id == "managed-keep"));
        assert!(
            !anime
                .iter()
                .any(|entry| entry.indexer_id == "managed-delete")
        );
        assert!(anime.iter().any(|entry| {
            entry.indexer_id == result.created_ids[0]
                && entry.enabled
                && entry.categories == vec!["5070"]
        }));
    }

    #[tokio::test]
    async fn prowlarr_sync_backfills_only_missing_child_pacing() {
        let indexer_repo = Arc::new(RecordingIndexerConfigRepo::new());
        let now = Utc::now();
        let parent = IndexerConfig {
            id: "prowlarr-parent".to_string(),
            name: "Prowlarr".to_string(),
            provider_type: "prowlarr".to_string(),
            base_url: "https://prowlarr.example".to_string(),
            api_key_encrypted: None,
            rate_limit_seconds: None,
            rate_limit_burst: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: false,
            enable_auto_search: false,
            indexer_proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            config_json: Some(r#"{"base_url":"https://prowlarr.example"}"#.to_string()),
            created_at: now,
            updated_at: now,
        };
        indexer_repo.create(parent.clone()).await.unwrap();

        let existing_template = IndexerConfig {
            id: String::new(),
            name: String::new(),
            provider_type: "prowlarr".to_string(),
            base_url: "https://child.example".to_string(),
            api_key_encrypted: None,
            rate_limit_seconds: None,
            rate_limit_burst: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            indexer_proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: Some(parent.id.clone()),
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            config_json: Some(r#"{"base_url":"https://child.example"}"#.to_string()),
            created_at: now,
            updated_at: now,
        };
        for (child_key, rate_limit_seconds) in
            [("missing", None), ("zero", Some(0)), ("custom", Some(7))]
        {
            let mut child = existing_template.clone();
            child.id = format!("child-{child_key}");
            child.name = child_key.to_string();
            child.managed_child_key = Some(child_key.to_string());
            child.rate_limit_seconds = rate_limit_seconds;
            indexer_repo.create(child).await.unwrap();
        }

        let children = ["missing", "zero", "custom", "new"]
            .into_iter()
            .map(|child_key| crate::ManagedIndexerChildPlan {
                child_key: child_key.to_string(),
                name: child_key.to_string(),
                provider_type: "prowlarr".to_string(),
                config_json: format!(r#"{{"base_url":"https://{child_key}.example"}}"#),
                is_enabled: true,
                enable_interactive_search: true,
                enable_auto_search: true,
                managed_metadata_json: None,
                caps_snapshot_json: None,
                routing_scopes: Vec::new(),
            })
            .collect();
        let provider = Arc::new(RecordingPluginProvider::with_sync_plan_for_provider(
            "prowlarr",
            crate::IndexerSyncPlan { children },
        ));
        let app = test_app(
            indexer_repo.clone(),
            Some(provider),
            Arc::new(RecordingSettingsRepository::default()),
        );

        app.sync_indexer_config(&test_admin(), &parent.id)
            .await
            .unwrap();

        let configs = indexer_repo.list(None).await.unwrap();
        let child_rate = |child_key: &str| {
            configs
                .iter()
                .find(|config| config.managed_child_key.as_deref() == Some(child_key))
                .expect("managed child should exist")
                .rate_limit_seconds
        };
        assert_eq!(child_rate("missing"), Some(2));
        assert_eq!(child_rate("zero"), Some(0));
        assert_eq!(child_rate("custom"), Some(7));
        assert_eq!(child_rate("new"), Some(2));
    }

    #[tokio::test]
    async fn sync_indexer_config_publishes_indexers_changed() {
        let indexer_repo = Arc::new(RecordingIndexerConfigRepo::new());
        let now = Utc::now();
        indexer_repo
            .create(IndexerConfig {
                id: "parent".to_string(),
                name: "Parent Manager".to_string(),
                provider_type: "manager".to_string(),
                base_url: "https://manager.example".to_string(),
                api_key_encrypted: None,
                rate_limit_seconds: None,
                rate_limit_burst: None,
                disabled_until: None,
                is_enabled: true,
                enable_interactive_search: false,
                enable_auto_search: false,
                indexer_proxy_config_id: None,
                download_client_id: None,
                seeding_profile_id: None,
                managed_parent_config_id: None,
                managed_child_key: None,
                managed_metadata_json: None,
                caps_snapshot_json: None,
                last_health_status: None,
                last_error_message: None,
                last_error_at: None,
                config_json: Some("{}".to_string()),
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();

        let provider = Arc::new(RecordingPluginProvider::with_sync_plan(
            crate::IndexerSyncPlan {
                children: vec![crate::ManagedIndexerChildPlan {
                    child_key: "new".to_string(),
                    name: "Managed New".to_string(),
                    provider_type: "torrent_rss".to_string(),
                    config_json: r#"{"feed_url":"https://new.example/rss"}"#.to_string(),
                    is_enabled: true,
                    enable_interactive_search: true,
                    enable_auto_search: false,
                    managed_metadata_json: None,
                    caps_snapshot_json: None,
                    routing_scopes: Vec::new(),
                }],
            },
        ));
        let app = test_app(
            indexer_repo,
            Some(provider),
            Arc::new(RecordingSettingsRepository::default()),
        );
        let mut receiver = app.runtime.events.indexers_changed_broadcast.subscribe();

        app.sync_indexer_config(&test_admin(), "parent")
            .await
            .unwrap();

        expect_indexers_changed(&mut receiver, "sync_indexer_config").await;
    }

    #[tokio::test]
    async fn managed_child_indexers_allow_only_global_enable_updates() {
        let indexer_repo = Arc::new(RecordingIndexerConfigRepo::new());
        let now = Utc::now();
        indexer_repo
            .create(IndexerConfig {
                id: "parent".to_string(),
                name: "Parent Manager".to_string(),
                provider_type: "manager".to_string(),
                base_url: "https://manager.example".to_string(),
                api_key_encrypted: None,
                rate_limit_seconds: None,
                rate_limit_burst: None,
                disabled_until: None,
                is_enabled: true,
                enable_interactive_search: false,
                enable_auto_search: false,
                indexer_proxy_config_id: None,
                download_client_id: None,
                seeding_profile_id: None,
                managed_parent_config_id: None,
                managed_child_key: None,
                managed_metadata_json: None,
                caps_snapshot_json: None,
                last_health_status: None,
                last_error_message: None,
                last_error_at: None,
                config_json: Some(r#"{"base_url":"https://manager.example"}"#.to_string()),
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        indexer_repo
            .create(IndexerConfig {
                id: "child".to_string(),
                name: "Managed Child".to_string(),
                provider_type: "torrent_rss".to_string(),
                base_url: "https://managed.example".to_string(),
                api_key_encrypted: None,
                rate_limit_seconds: None,
                rate_limit_burst: None,
                disabled_until: None,
                is_enabled: true,
                enable_interactive_search: true,
                enable_auto_search: true,
                indexer_proxy_config_id: None,
                download_client_id: None,
                seeding_profile_id: None,
                managed_parent_config_id: Some("parent".to_string()),
                managed_child_key: Some("child".to_string()),
                managed_metadata_json: None,
                caps_snapshot_json: None,
                last_health_status: None,
                last_error_message: None,
                last_error_at: None,
                config_json: Some(r#"{"feed_url":"https://managed.example/rss"}"#.to_string()),
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        let app = test_app(
            indexer_repo.clone(),
            Some(Arc::new(RecordingPluginProvider::new(
                "torrent_rss",
                vec![string_field(
                    "feed_url",
                    "Feed URL",
                    Some(scryer_domain::ConfigFieldRole::ConnectionUrl),
                )],
                rss_only_capabilities(),
                Arc::new(RecordingIndexerClient::new(false)),
            ))),
            Arc::new(NullSettingsRepository),
        );

        let disabled = app
            .update_indexer_config(
                &test_admin(),
                IndexerConfigUpdate {
                    id: "child".to_string(),
                    is_enabled: Some(false),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(!disabled.is_enabled);
        assert_eq!(disabled.managed_metadata_json, None);
        let parent = indexer_repo.get_by_id("parent").await.unwrap().unwrap();
        assert_eq!(
            parent.managed_metadata_json.as_deref(),
            Some(r#"{"locally_disabled_children":["child"]}"#)
        );

        let enable_error = app
            .update_indexer_config(
                &test_admin(),
                IndexerConfigUpdate {
                    id: "child".to_string(),
                    is_enabled: Some(true),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert!(
            enable_error
                .to_string()
                .contains("does not support managed child sync")
        );
        assert!(
            !indexer_repo
                .get_by_id("child")
                .await
                .unwrap()
                .unwrap()
                .is_enabled
        );
        let parent = indexer_repo.get_by_id("parent").await.unwrap().unwrap();
        assert_eq!(
            parent.managed_metadata_json.as_deref(),
            Some(r#"{"locally_disabled_children":["child"]}"#)
        );

        let rate_limit_error = app
            .update_indexer_config(
                &test_admin(),
                IndexerConfigUpdate {
                    id: "child".to_string(),
                    rate_limit_seconds: Some(15),
                    rate_limit_burst: Some(3),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert!(
            rate_limit_error
                .to_string()
                .contains("managed child indexers are controlled by their parent sync")
        );

        let update_error = app
            .update_indexer_config(
                &test_admin(),
                IndexerConfigUpdate {
                    id: "child".to_string(),
                    name: Some("Renamed".to_string()),
                    is_enabled: Some(true),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert!(
            update_error
                .to_string()
                .contains("managed child indexers are controlled by their parent sync")
        );

        let delete_error = app
            .delete_indexer_config(&test_admin(), "child")
            .await
            .unwrap_err();
        assert!(
            delete_error
                .to_string()
                .contains("managed child indexers are controlled by their parent sync")
        );
    }

    #[tokio::test]
    async fn indexer_routing_changes_prune_learning_but_category_order_does_not() {
        let client = Arc::new(RecordingIndexerClient::new(false));
        let app = test_app_with_indexer_client(
            Arc::new(RecordingIndexerConfigRepo::new()),
            None,
            Arc::new(RecordingSettingsRepository::default()),
            client.clone(),
        );
        let entry = |enabled: bool, categories: Vec<&str>| IndexerRoutingSettingsEntry {
            indexer_id: "idx-routing".into(),
            enabled,
            categories: categories.into_iter().map(str::to_string).collect(),
            priority: 4,
        };

        app.update_indexer_routing(
            &test_admin(),
            "series",
            vec![entry(true, vec!["5000", "5070"])],
        )
        .await
        .expect("initial routing should persist");
        client.pruned_indexers.lock().unwrap().clear();

        app.update_indexer_routing(
            &test_admin(),
            "series",
            vec![entry(true, vec!["5070", "5000", "5000"])],
        )
        .await
        .expect("equivalent routing should persist");
        assert!(client.pruned_indexers().is_empty());

        app.update_indexer_routing(
            &test_admin(),
            "series",
            vec![entry(false, vec!["5000", "5070"])],
        )
        .await
        .expect("changed routing should persist");
        assert_eq!(client.pruned_indexers(), vec!["idx-routing"]);
    }
}

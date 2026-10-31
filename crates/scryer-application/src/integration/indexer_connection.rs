use super::*;
use chrono::Utc;

pub(crate) struct ConnectionValidation {
    pub(crate) caps_snapshot_json: Option<String>,
    persisted_config: Option<IndexerConfig>,
}

/// A draft indexer configuration made ready to run under the probe's
/// synthetic id.
struct PreparedIndexerProbe {
    provider: Arc<dyn IndexerPluginProvider>,
    fields: Vec<scryer_domain::ConfigFieldDef>,
    persisted_config: Option<IndexerConfig>,
    validated_base_url: Option<url::Url>,
    accounting: Option<crate::IndexerAccountingContext>,
    proxy_config: Option<scryer_domain::ProxyConfig>,
    temp_config: IndexerConfig,
}

impl AppUseCase {
    /// Test an indexer connection by performing a minimal search through the plugin system.
    /// This validates: plugin availability, HTTP connectivity, API key, response parsing.
    pub async fn test_indexer_connection(
        &self,
        actor: &User,
        provider_type: &str,
        config_json: Option<&str>,
        indexer_id: Option<&str>,
        proxy_config_id_override: Option<Option<&str>>,
    ) -> AppResult<()> {
        let result = self
            .probe_indexer_connection(
                actor,
                provider_type,
                config_json,
                indexer_id,
                proxy_config_id_override,
            )
            .await?;
        if let Some(config) = result.persisted_config {
            if let Some(snapshot) = result.caps_snapshot_json {
                if !self
                    .services
                    .integrations
                    .indexer_configs
                    .save_caps_if_unchanged(&config, &snapshot)
                    .await?
                {
                    return Err(AppError::validation_refused(crate::error_reasons::integration::INDEXER_SETTINGS_CHANGED_DURING_VALIDATION, "Indexer settings changed during validation; reload and test again"));
                }
                let mut refreshed = config.clone();
                refreshed.caps_snapshot_json = Some(snapshot);
                if crate::indexer_search_identity(&config, None)
                    != crate::indexer_search_identity(&refreshed, None)
                {
                    self.prune_indexer_search_learning_best_effort(
                        &config.id,
                        "caps_snapshot_change",
                    )
                    .await;
                }
            }
            self.services
                .integrations
                .indexer_configs
                .set_last_error_if_unchanged(&config, None, None)
                .await?;
            self.publish_indexers_changed();
        }
        Ok(())
    }

    /// Validate the proposed connection without clearing the saved config's
    /// health. A save clears health only after its remaining checks succeed.
    pub(crate) async fn probe_indexer_connection(
        &self,
        actor: &User,
        provider_type: &str,
        config_json: Option<&str>,
        indexer_id: Option<&str>,
        proxy_config_id_override: Option<Option<&str>>,
    ) -> AppResult<ConnectionValidation> {
        let PreparedIndexerProbe {
            provider,
            fields,
            persisted_config,
            validated_base_url,
            accounting,
            proxy_config,
            temp_config,
        } = self
            .prepare_indexer_probe(
                actor,
                provider_type,
                config_json,
                indexer_id,
                proxy_config_id_override,
            )
            .await?;

        if let Some(validated_base_url) = validated_base_url.as_ref()
            && !temp_config.is_direct_nab()
        {
            preflight_test_flight_url(
                validated_base_url,
                proxy_config.as_ref(),
                accounting.as_ref(),
                self.services.integrations.indexer_stats.clone(),
            )
            .await?;
        }
        let management_capabilities = provider.management_capabilities_for_provider(provider_type);
        if management_capabilities.supports_validate_config
            || management_capabilities.supports_managed_children_sync
        {
            let client = provider
                .management_client_for_provider_with_accounting(&temp_config, accounting.as_ref())
                .ok_or_else(|| {
                    AppError::validation_refused(crate::error_reasons::integration::NO_INDEXER_MANAGEMENT_CLIENT_AVAILABLE_PROVIDER_TYPE, format!(
                        "no indexer management client available for provider type '{provider_type}'"
                    ))
                })?;
            if management_capabilities.supports_validate_config {
                let result = client.validate_connection().await?;
                validate_indexer_connection_result(result)?;
            }
            return Ok(ConnectionValidation {
                caps_snapshot_json: None,
                persisted_config,
            });
        }

        let client = provider
            .client_for_provider_with_accounting(
                &temp_config,
                proxy_config.as_ref(),
                accounting.as_ref(),
            )
            .ok_or_else(|| {
                AppError::validation_refused(
                    crate::error_reasons::integration::NO_INDEXER_PROVIDER_AVAILABLE_PROVIDER_TYPE,
                    format!("no indexer provider available for provider type '{provider_type}'"),
                )
            })?;
        if !client
            .probe_connection()
            .await
            .map_err(map_indexer_connection_test_error)?
        {
            let capabilities = provider.capabilities_for_provider(provider_type);
            let (query, ids, facet) = build_connection_test_search_request(&capabilities);

            // Older components and providers without a dedicated probe retain
            // the established minimal-search validation path.
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
                    None, // a connection probe has no subject, so no year
                    vec![],
                    None,
                    tokio_util::sync::CancellationToken::new(),
                )
                .await
                .map_err(map_indexer_connection_test_error)?;
        }

        let mut caps_config = temp_config.clone();
        // Sharing and reuse are keyed by the actual settings, never just the saved id.
        let matching_config = persisted_config.filter(|saved| {
            let mut normalized = saved.clone();
            if let Ok(config) = crate::app_usecase_integration::normalize_indexer_config_json(
                &fields,
                saved.config_json.as_deref(),
                None,
            ) {
                normalized.config_json = Some(config);
            }
            super::indexer_caps_cache::connection_identity(&normalized)
                == super::indexer_caps_cache::connection_identity(&temp_config)
        });
        if let Some(saved) = matching_config.as_ref() {
            caps_config.id = saved.id.clone();
            caps_config.caps_snapshot_json = saved.caps_snapshot_json.clone();
        } else if let Some(id) = indexer_id {
            caps_config.id = id.to_string();
        }
        let caps_refresh_available = self
            .services
            .integrations
            .indexer_caps_refresher
            .available()
            .is_some();
        let caps_snapshot = self
            .fetch_caps_snapshot_json_for_config_with_accounting(
                &caps_config,
                proxy_config.as_ref(),
                accounting.as_ref(),
            )
            .await
            .map_err(map_indexer_connection_test_error)?;
        if caps_refresh_available && temp_config.is_direct_nab() && caps_snapshot.is_none() {
            return Err(AppError::validation_refused(crate::error_reasons::integration::INDEXER_CONNECTION_TEST_DID_NOT_RETURN_VALID_NEWZNAB_CAPS_DOCUMENT, "indexer connection test did not return a valid Newznab caps document".to_string()));
        }

        Ok(ConnectionValidation {
            caps_snapshot_json: caps_snapshot,
            persisted_config: matching_config,
        })
    }

    /// The image challenge the indexer's sign-in asks for under the operator's
    /// draft settings, fetched before they are saved. The draft is prepared
    /// exactly as a connection test prepares it: stored secrets are restored
    /// for an existing indexer and the proxy the draft names is used. `None`
    /// when the provider declares no field for the answer, or its sign-in
    /// asks for nothing right now. The image is returned once and never
    /// stored.
    pub async fn indexer_login_challenge(
        &self,
        actor: &User,
        provider_type: &str,
        config_json: Option<&str>,
        indexer_id: Option<&str>,
        proxy_config_id_override: Option<Option<&str>>,
    ) -> AppResult<Option<crate::IndexerLoginChallenge>> {
        let prepared = self
            .prepare_indexer_probe(
                actor,
                provider_type,
                config_json,
                indexer_id,
                proxy_config_id_override,
            )
            .await?;
        if !prepared
            .fields
            .iter()
            .any(|field| field.role == Some(scryer_domain::ConfigFieldRole::LoginChallengeAnswer))
        {
            return Ok(None);
        }
        let management = prepared
            .provider
            .management_capabilities_for_provider(provider_type);
        if management.supports_validate_config || management.supports_managed_children_sync {
            return Ok(None);
        }
        let Some(client) = prepared.provider.client_for_provider_with_accounting(
            &prepared.temp_config,
            prepared.proxy_config.as_ref(),
            prepared.accounting.as_ref(),
        ) else {
            return Ok(None);
        };
        client
            .login_challenge()
            .await
            .map_err(map_indexer_connection_test_error)
    }

    /// The submitted draft as a connection probe runs it: the provider is
    /// usable, the config is normalized against the effective fields with
    /// stored secrets restored, and the proxy it names is resolved.
    async fn prepare_indexer_probe(
        &self,
        actor: &User,
        provider_type: &str,
        config_json: Option<&str>,
        indexer_id: Option<&str>,
        proxy_config_id_override: Option<Option<&str>>,
    ) -> AppResult<PreparedIndexerProbe> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        self.ensure_provider_plugin_not_blocked(provider_type)
            .await?;

        // Fails early for an unknown provider, before any stored config is read.
        self.indexer_config_fields_for_provider_type(provider_type)?;
        let persisted_config = if let Some(indexer_id) = indexer_id {
            self.services
                .integrations
                .indexer_configs
                .get_by_id(indexer_id)
                .await?
        } else {
            None
        };
        let same_provider_persisted_json = persisted_config
            .as_ref()
            .filter(|config| {
                config
                    .provider_type
                    .trim()
                    .eq_ignore_ascii_case(provider_type.trim())
            })
            .and_then(|config| config.config_json.as_deref());
        let fields = self
            .effective_indexer_config_fields(
                provider_type,
                &[config_json, same_provider_persisted_json],
            )
            .await?;
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
            .cloned()
            .ok_or_else(|| AppError::Repository("indexer provider not available".into()))?;
        provider.validate_config_for_provider(provider_type, &normalized_config_json)?;
        let base_url = crate::app_usecase_integration::derive_indexer_base_url_from_config_fields(
            &fields,
            Some(&normalized_config_json),
        )?;
        // An empty base URL means the provider declares no connection-URL
        // field (its endpoint is fixed by the plugin's allowed hosts), so
        // there is no operator URL to validate or preflight.
        let validated_base_url = if base_url.is_empty() {
            None
        } else {
            Some(validate_test_flight_url(&base_url)?)
        };
        let accounting = persisted_config
            .as_ref()
            .and_then(crate::IndexerAccountingContext::for_config);

        let now = Utc::now();

        // Build a temporary IndexerConfig to get a client from the provider.
        // Reject obviously invalid API keys (e.g. masked placeholders from
        // Sonarr/Radarr import that were stored before the masking fix).
        let parsed_config: serde_json::Value = serde_json::from_str(&normalized_config_json)
            .map_err(|error| {
                AppError::validation_refused(
                    crate::error_reasons::integration::INDEXER_CONFIG_INVALID,
                    error.to_string(),
                )
            })?;
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
                return Err(AppError::validation_refused(crate::error_reasons::integration::API_KEY_APPEARS_MASKED_PLACEHOLDER_ENTER_REAL_KEY, "API key appears to be a masked placeholder — enter the real key"));
            }
        }

        let proxy_config_id = match proxy_config_id_override {
            Some(Some(id)) => Some(id.to_string()),
            Some(None) => None,
            None => persisted_config
                .as_ref()
                .and_then(|config| config.proxy_config_id.clone()),
        };
        let proxy_config = if let Some(proxy_config_id) = proxy_config_id.as_deref() {
            let proxy_config = self
                .services
                .integrations
                .proxy_configs
                .get_by_id(proxy_config_id)
                .await?
                .ok_or_else(|| {
                    AppError::validation_refused(
                        crate::error_reasons::integration::PROXY_CONFIGURATION_NOT_FOUND,
                        "Proxy configuration was not found.".to_string(),
                    )
                })?;
            if !proxy_config.is_enabled {
                return Err(AppError::validation_refused(
                    crate::error_reasons::integration::PROXY_DISABLED_INDEXER,
                    "Proxy is disabled for this indexer.".to_string(),
                ));
            }
            // Prowlarr owns challenge handling for the indexers it fronts, so
            // a challenge solver can never sit in front of it. Transport and
            // tunnel proxies only carry bytes and remain allowed.
            if proxy_config.is_challenge_solver()
                && (provider_type.trim().eq_ignore_ascii_case("prowlarr")
                    || persisted_config
                        .as_ref()
                        .is_some_and(IndexerConfig::is_prowlarr_nab_proxy))
            {
                return Err(AppError::validation_refused(crate::error_reasons::integration::PROWLARR_INDEXERS_CANNOT_USE_CHALLENGE_SOLVERS, "Prowlarr indexers cannot use challenge solvers; Prowlarr owns challenge handling."
                        .to_string()));
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
            max_queries_per_minute: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            disabled_until: None,
            proxy_config_id,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_code: None,
            last_error_message: None,
            last_error_at: None,
            config_json: Some(normalized_config_json),
            created_at: now,
            updated_at: now,
        };

        Ok(PreparedIndexerProbe {
            provider,
            fields,
            persisted_config,
            validated_base_url,
            accounting,
            proxy_config,
            temp_config,
        })
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
        // The preview API takes no proxy identity, so this Prowlarr preview
        // preflight has none to honor.
        preflight_test_flight_url(
            &validated_base_url,
            None,
            None,
            self.services.integrations.indexer_stats.clone(),
        )
        .await?;

        let provider = self
            .services
            .integrations
            .plugin_provider
            .available()
            .ok_or_else(|| AppError::Repository("indexer provider not available".into()))?;
        let management_capabilities = provider.management_capabilities_for_provider(provider_type);
        if !management_capabilities.supports_managed_children_sync {
            return Err(AppError::validation_refused(crate::error_reasons::integration::PROVIDER_TYPE_DOES_NOT_SUPPORT_MANAGED_CHILD_SYNC, format!(
                "provider type '{provider_type}' does not support managed child sync"
            )));
        }

        let parsed_config: serde_json::Value = serde_json::from_str(&normalized_config_json)
            .map_err(|error| {
                AppError::validation_refused(
                    crate::error_reasons::integration::INDEXER_CONFIG_INVALID,
                    error.to_string(),
                )
            })?;
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
                return Err(AppError::validation_refused(crate::error_reasons::integration::API_KEY_APPEARS_MASKED_PLACEHOLDER_ENTER_REAL_KEY, "API key appears to be a masked placeholder — enter the real key"));
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
            max_queries_per_minute: None,
            is_enabled: true,
            enable_interactive_search: false,
            enable_auto_search: false,
            disabled_until: None,
            proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_code: None,
            last_error_message: None,
            last_error_at: None,
            config_json: Some(normalized_config_json),
            created_at: now,
            updated_at: now,
        };

        let client = provider
            .management_client_for_provider(&temp_config)
            .ok_or_else(|| {
                AppError::validation_refused(crate::error_reasons::integration::NO_INDEXER_MANAGEMENT_CLIENT_AVAILABLE_PROVIDER_TYPE, format!(
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

    // Whatever the indexer said about itself is the only thing that tells an
    // operator what to fix, so it is surfaced whichever status carried it. The
    // old allowlist surfaced three statuses and masked every other one as
    // "Internal server error", which named neither the indexer nor the problem.
    let message = result
        .message
        .map(|message| message.trim().to_string())
        .filter(|message| !message.is_empty())
        .unwrap_or_else(|| format!("indexer reported status '{}'", result.status));

    Err(AppError::Validation(message))
}

/// Both call sites wrap the indexer interaction itself — the probe search and
/// the caps fetch — so everything arriving here describes the remote indexer,
/// not Scryer. `Repository` masks it as "Internal server error" behind a
/// reference id, which is the one thing an operator saving an indexer cannot
/// act on; hand back what the indexer actually said instead.
fn map_indexer_connection_test_error(error: AppError) -> AppError {
    match error {
        // Kinds that already carry a user-facing message and their own GraphQL
        // code. Re-wrapping them would only bury what they say.
        AppError::Validation(_)
        | AppError::ValidationRefused { .. }
        | AppError::ValidationRefusedWithCount { .. }
        | AppError::Unauthorized(_)
        | AppError::UnauthorizedReason { .. }
        | AppError::NotFound(_)
        | AppError::NotFoundReason { .. }
        | AppError::Canceled(_)
        | AppError::TemporaryUnavailable { .. } => error,
        // Keeps the reason the plugin named, so a client can still say why
        // in its own language; only the sentence says it was the test.
        AppError::PluginFailed { message, failure } => AppError::PluginFailed {
            message: format!("indexer connection test failed: {message}"),
            failure,
        },
        error => {
            // `Repository`'s Display prepends "repository: ", which is noise in
            // a toast, so unwrap it to the message the provider wrote.
            let message = match error {
                AppError::Repository(message)
                | AppError::RepositoryFailure { message, .. }
                | AppError::RepositoryFailed { message, .. } => message,
                other => other.to_string(),
            };
            match known_newznab_error_message(&message) {
                Some(user_message) => AppError::validation_refused(
                    crate::error_reasons::integration::INDEXER_REQUEST_REJECTED,
                    user_message,
                ),
                None => AppError::validation_refused(
                    crate::error_reasons::integration::INDEXER_CONNECTION_TEST_FAILED,
                    format!("indexer connection test failed: {message}"),
                ),
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
    let url = url::Url::parse(raw).map_err(|error| {
        AppError::validation_refused(
            crate::error_reasons::integration::BASE_URL_INVALID,
            format!("invalid base URL: {error}"),
        )
    })?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(AppError::validation_refused(
            crate::error_reasons::integration::BASE_URL_INVALID,
            "base URL must use http or https",
        ));
    }
    if url.host_str().is_none() {
        return Err(AppError::validation_refused(
            crate::error_reasons::integration::BASE_URL_HOST_REQUIRED,
            "base URL must include a host",
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(AppError::validation_refused(
            crate::error_reasons::integration::BASE_URL_CREDENTIALS_FORBIDDEN,
            "base URL must not include embedded credentials",
        ));
    }
    Ok(url)
}

pub(super) fn persisted_indexer_config_can_restore_secrets(
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
async fn preflight_test_flight_url(
    url: &url::Url,
    proxy: Option<&scryer_domain::ProxyConfig>,
    accounting: Option<&crate::IndexerAccountingContext>,
    stats: Arc<dyn crate::IndexerStatsTracker>,
) -> AppResult<()> {
    observed_preflight_test_flight_url(url, proxy, accounting, stats).await
}

async fn observed_preflight_test_flight_url(
    url: &url::Url,
    proxy: Option<&scryer_domain::ProxyConfig>,
    accounting: Option<&crate::IndexerAccountingContext>,
    stats: Arc<dyn crate::IndexerStatsTracker>,
) -> AppResult<()> {
    let origin = url.origin().ascii_serialization();
    // The preflight is the first request a save sends the indexer, so it goes
    // through the indexer's proxy like every search does, or not at all.
    let client = crate::transport_proxy::indexer_host_request_client(
        proxy,
        scryer_outbound_http::indexer_reqwest_client,
    )
    // The transport builder supplies its own diagnostic; no provider condition
    // can be inferred here. Keep this internal transport failure as a relay.
    .map_err(AppError::Validation)?;
    let outbound_http = scryer_outbound_http::OutboundHttpClient::new(
        client,
        scryer_outbound_http::RateLimitRegistry::indexers(),
    );

    let tally = crate::IndexerRequestTally::new(
        accounting
            .map(|context| context.indexer_id.as_str())
            .unwrap_or(crate::CONNECTION_TEST_INDEXER_ID)
            .to_string(),
        accounting
            .map(|context| context.indexer_name.as_str())
            .unwrap_or("Test Connection")
            .to_string(),
        crate::IndexerErrorOperation::IndexerAction,
        stats,
    )
    .with_indexer_origin(url.as_str());
    let response = outbound_http
        .send_with_dispatch_observer(
            scryer_outbound_http::RequestPolicy::no_retry("indexer_preflight", "head_origin"),
            || outbound_http.client().head(&origin),
            |url| {
                if tally.try_note_sent(url.as_str()) {
                    Ok(())
                } else {
                    Err(scryer_outbound_http::OutboundHttpError::DispatchRejected)
                }
            },
        )
        .await
        .map_err(|error| {
            tally.note_result(crate::IndexerRequestResult::NoResponse);
            AppError::validation_refused(
                crate::error_reasons::integration::INDEXER_CONNECTION_UNREACHABLE,
                format_preflight_transport_error(url, &origin, &error.to_string()),
            )
        })?;
    tally.note_response(&crate::CapturedIndexerHttpResponse {
        status: response.status().as_u16(),
        headers: Vec::new(),
        body: Vec::new(),
    });

    Ok(())
}

#[cfg(test)]
thread_local! {
    /// The proxy id each unit-test preflight was handed, in call order. Unit
    /// tests never send the preflight; they only prove which egress it was
    /// given. `observed_preflight_test_flight_url` is tested for the send.
    static PREFLIGHT_PROXY_IDS: std::cell::RefCell<Vec<Option<String>>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
fn take_preflight_proxy_ids() -> Vec<Option<String>> {
    PREFLIGHT_PROXY_IDS.with(|ids| std::mem::take(&mut *ids.borrow_mut()))
}

#[cfg(test)]
async fn preflight_test_flight_url(
    _url: &url::Url,
    proxy: Option<&scryer_domain::ProxyConfig>,
    _accounting: Option<&crate::IndexerAccountingContext>,
    _stats: Arc<dyn crate::IndexerStatsTracker>,
) -> AppResult<()> {
    PREFLIGHT_PROXY_IDS.with(|ids| ids.borrow_mut().push(proxy.map(|proxy| proxy.id.clone())));
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
    #[tokio::test]
    async fn connection_preflight_counts_saved_identity_and_respects_shutdown() {
        #[derive(Default)]
        struct Stats {
            ids: std::sync::Mutex<Vec<String>>,
            gate: crate::IndexerDispatchGate,
        }
        impl crate::IndexerStatsTracker for Stats {
            fn dispatch_gate(&self) -> Option<crate::IndexerDispatchGate> {
                Some(self.gate.clone())
            }
            fn record_query(&self, _: &str, _: &str, _: bool) {}
            fn record_grab(&self, _: &str, _: &str) {}
            fn record_api_limits(
                &self,
                _: &str,
                _: Option<u32>,
                _: Option<u32>,
                _: Option<u32>,
                _: Option<u32>,
            ) {
            }
            fn record_api_request_sent(&self, id: &str, _: &str) {
                self.ids.lock().unwrap().push(id.to_string());
            }
            fn all_stats(&self) -> Vec<crate::IndexerQueryStats> {
                vec![]
            }
        }
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("HEAD"))
            .respond_with(wiremock::ResponseTemplate::new(200))
            .expect(2)
            .mount(&server)
            .await;
        let url = url::Url::parse(&server.uri()).unwrap();
        let stats = Arc::new(Stats::default());
        let accounting = crate::IndexerAccountingContext {
            indexer_id: "saved-preflight".into(),
            indexer_name: "Saved".into(),
        };
        observed_preflight_test_flight_url(&url, None, Some(&accounting), stats.clone())
            .await
            .unwrap();
        observed_preflight_test_flight_url(&url, None, None, stats.clone())
            .await
            .unwrap();
        stats.gate.close();
        assert!(
            observed_preflight_test_flight_url(&url, None, Some(&accounting), stats.clone())
                .await
                .is_err()
        );
        assert_eq!(*stats.ids.lock().unwrap(), vec!["saved-preflight"]);
    }

    /// Minimal HTTP forward proxy: records each absolute-form request line and
    /// answers it itself, without contacting the origin.
    async fn spawn_recording_http_proxy() -> (String, Arc<std::sync::Mutex<Vec<String>>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("proxy double should bind");
        let address = listener.local_addr().expect("proxy double address");
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorder = Arc::clone(&seen);
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let recorder = Arc::clone(&recorder);
                tokio::spawn(async move {
                    let mut buffer = vec![0u8; 4096];
                    let mut received = Vec::new();
                    while !received.windows(4).any(|window| window == b"\r\n\r\n") {
                        match stream.read(&mut buffer).await {
                            Ok(0) | Err(_) => return,
                            Ok(read) => received.extend_from_slice(&buffer[..read]),
                        }
                    }
                    if let Some(line) = String::from_utf8_lossy(&received).lines().next() {
                        recorder.lock().unwrap().push(line.to_string());
                    }
                    let _ = stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        )
                        .await;
                    let _ = stream.flush().await;
                });
            }
        });
        (format!("http://{address}"), seen)
    }

    fn transport_proxy_config(id: &str, base_url: &str) -> scryer_domain::ProxyConfig {
        scryer_domain::ProxyConfig {
            id: id.to_string(),
            name: "House Proxy".into(),
            provider_type: scryer_domain::ProxyProviderType::Http,
            protocol: None,
            base_url: base_url.to_string(),
            ..solver_proxy_config(id)
        }
    }

    /// The e2e proxy matrix caught the save-time preflight `HEAD /` reaching a
    /// proxied indexer straight from Scryer's own address. The preflight is
    /// indexer traffic like the search that follows it, so the indexer's proxy
    /// carries it.
    #[tokio::test]
    async fn preflight_for_a_proxied_indexer_travels_through_its_transport_proxy() {
        let origin = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(wiremock::ResponseTemplate::new(200))
            .mount(&origin)
            .await;
        let (proxy_url, proxy_requests) = spawn_recording_http_proxy().await;
        let proxy = transport_proxy_config("house-proxy", &proxy_url);
        let url = url::Url::parse(&format!("{}/api", origin.uri())).unwrap();

        observed_preflight_test_flight_url(
            &url,
            Some(&proxy),
            None,
            Arc::new(crate::NullIndexerStatsTracker),
        )
        .await
        .expect("the proxy answers the preflight");

        let proxy_requests = proxy_requests.lock().unwrap().clone();
        assert_eq!(
            proxy_requests,
            vec![format!("HEAD {}/ HTTP/1.1", origin.uri())],
            "the proxy must carry the preflight"
        );
        assert!(
            origin
                .received_requests()
                .await
                .unwrap_or_default()
                .is_empty(),
            "the indexer must never be reached directly"
        );
    }

    /// Fail closed: when the assigned proxy cannot carry the preflight, the
    /// preflight fails and names the proxy. It never falls back to a direct
    /// request. A challenge solver is not a hop, so it keeps the direct
    /// preflight exactly as the plugin hosts keep an indexer's first request
    /// direct.
    #[tokio::test]
    async fn preflight_fails_closed_when_the_assigned_proxy_cannot_carry_it() {
        let origin = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(wiremock::ResponseTemplate::new(200))
            .mount(&origin)
            .await;
        let url = url::Url::parse(&origin.uri()).unwrap();

        let unbuildable = transport_proxy_config("house-proxy", "not a proxy url");
        let mut disabled = transport_proxy_config("house-proxy", "http://127.0.0.1:1");
        disabled.is_enabled = false;
        let mut unusable_tunnel = crate::tunnel_proxy::tests::tunnel_config();
        unusable_tunnel.id = "preflight-unusable-tunnel".into();
        unusable_tunnel.username_encrypted = None;

        for proxy in [unbuildable, disabled, unusable_tunnel] {
            let error = observed_preflight_test_flight_url(
                &url,
                Some(&proxy),
                None,
                Arc::new(crate::NullIndexerStatsTracker),
            )
            .await
            .expect_err("an unusable proxy must fail the preflight");
            assert!(
                error.to_string().contains(proxy.name.trim()),
                "the failure must name the proxy: {error}"
            );
        }
        assert!(
            origin
                .received_requests()
                .await
                .unwrap_or_default()
                .is_empty(),
            "nothing may reach the indexer directly when its proxy is unusable"
        );

        observed_preflight_test_flight_url(
            &url,
            Some(&solver_proxy_config("solver")),
            None,
            Arc::new(crate::NullIndexerStatsTracker),
        )
        .await
        .expect("a solver-assigned preflight is direct");
        assert_eq!(
            origin.received_requests().await.unwrap_or_default().len(),
            1
        );
    }
    use crate::NullSettingsRepository;
    use crate::null_repositories::test_nulls::{
        NullDownloadClient, NullDownloadClientConfigRepository, NullIndexerClient,
        NullQualityProfileRepository, NullReleaseAttemptRepository, NullShowRepository,
        NullTitleRepository, NullUserRepository,
    };
    use async_trait::async_trait;
    use std::collections::{HashMap, HashSet};
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
    fn unknown_newznab_errors_have_no_curated_message() {
        assert_eq!(
            known_newznab_error_message("Newznab API error 777: Provider-specific oddity"),
            None
        );
    }

    #[test]
    fn a_plugin_failure_survives_the_connection_test_with_its_reason() {
        let failure = crate::PluginFailure {
            reason: crate::PluginFailureReason::AuthFailed,
            detail: Some("Tracker login failed: wrong password".to_string()),
        };
        let mapped = map_indexer_connection_test_error(AppError::PluginFailed {
            message: "indexer indexer_search component: plugin error AuthFailed: x".to_string(),
            failure: failure.clone(),
        });

        assert_eq!(
            mapped.to_string(),
            "indexer connection test failed: indexer indexer_search component: \
             plugin error AuthFailed: x"
        );
        assert_eq!(mapped.plugin_failure(), Some(&failure));
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
        probe_result: bool,
        dynamic_fields: Option<crate::PluginDynamicConfigFields>,
        dynamic_calls: Arc<std::sync::Mutex<Vec<std::collections::BTreeMap<String, String>>>>,
        /// `Err` names the refusal reason the challenge fails with.
        login_challenge: Option<Result<crate::IndexerLoginChallenge, &'static str>>,
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
                probe_result: false,
                dynamic_fields: None,
                dynamic_calls: Arc::new(std::sync::Mutex::new(Vec::new())),
                login_challenge: None,
            }
        }

        fn with_login_challenge(
            mut self,
            challenge: Result<crate::IndexerLoginChallenge, &'static str>,
        ) -> Self {
            self.login_challenge = Some(challenge);
            self
        }

        fn with_dynamic_fields(mut self, fields: crate::PluginDynamicConfigFields) -> Self {
            self.dynamic_fields = Some(fields);
            self
        }

        fn dynamic_calls(&self) -> Vec<std::collections::BTreeMap<String, String>> {
            self.dynamic_calls.lock().unwrap().clone()
        }

        fn with_connection_probe(mut self) -> Self {
            self.probe_result = true;
            self
        }

        fn pruned_indexers(&self) -> Vec<String> {
            self.pruned_indexers.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl IndexerClient for RecordingIndexerClient {
        async fn probe_connection(&self) -> AppResult<bool> {
            Ok(self.probe_result)
        }

        async fn dynamic_config_fields(
            &self,
            values: &std::collections::BTreeMap<String, String>,
        ) -> AppResult<Option<crate::PluginDynamicConfigFields>> {
            self.dynamic_calls.lock().unwrap().push(values.clone());
            Ok(self.dynamic_fields.clone())
        }

        async fn login_challenge(&self) -> AppResult<Option<crate::IndexerLoginChallenge>> {
            match &self.login_challenge {
                None => Ok(None),
                Some(Ok(challenge)) => Ok(Some(challenge.clone())),
                Some(Err(reason)) => Err(AppError::validation_refused(
                    reason,
                    "synthetic challenge refusal",
                )),
            }
        }

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
            _year: Option<i32>,
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
                next_cursor: None,
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
            _proxy: Option<&scryer_domain::ProxyConfig>,
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
            _proxy: Option<&scryer_domain::ProxyConfig>,
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
            _proxy: Option<&scryer_domain::ProxyConfig>,
        ) -> AppResult<Option<scryer_domain::IndexerCapsSnapshot>> {
            if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                Ok(Some(scryer_domain::IndexerCapsSnapshot::default()))
            } else {
                Err(AppError::Repository("synthetic caps failure".into()))
            }
        }
    }

    /// Stands in for the outbound `t=caps` request: every call is one send.
    #[derive(Default)]
    struct RecordingCapsSnapshotRefresher {
        requested_ids: std::sync::Mutex<Vec<String>>,
        requested_proxy_ids: std::sync::Mutex<Vec<Option<String>>>,
    }

    impl RecordingCapsSnapshotRefresher {
        fn requested_ids(&self) -> Vec<String> {
            self.requested_ids.lock().unwrap().clone()
        }

        /// The proxy each send was handed, in call order.
        fn requested_proxy_ids(&self) -> Vec<Option<String>> {
            self.requested_proxy_ids.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl IndexerCapsSnapshotRefresher for RecordingCapsSnapshotRefresher {
        async fn fetch_for_config(
            &self,
            config: &IndexerConfig,
            proxy: Option<&scryer_domain::ProxyConfig>,
        ) -> AppResult<Option<scryer_domain::IndexerCapsSnapshot>> {
            self.requested_ids.lock().unwrap().push(config.id.clone());
            self.requested_proxy_ids
                .lock()
                .unwrap()
                .push(proxy.map(|proxy| proxy.id.clone()));
            Ok(Some(scryer_domain::IndexerCapsSnapshot::default()))
        }
    }

    type RecordedIndexerErrors = Arc<Mutex<Vec<(String, Option<String>)>>>;

    struct RecordingIndexerConfigRepo {
        created: Arc<Mutex<Vec<IndexerConfig>>>,
        cleared_ids: Arc<Mutex<Vec<String>>>,
        recorded_errors: RecordedIndexerErrors,
        recorded_error_codes: Arc<Mutex<Vec<Option<String>>>>,
        system_backoffs: Arc<Mutex<HashMap<String, crate::IndexerSystemBackoff>>>,
    }

    impl RecordingIndexerConfigRepo {
        fn new() -> Self {
            Self {
                created: Arc::new(Mutex::new(Vec::new())),
                cleared_ids: Arc::new(Mutex::new(Vec::new())),
                recorded_errors: Arc::new(Mutex::new(Vec::new())),
                recorded_error_codes: Arc::new(Mutex::new(Vec::new())),
                system_backoffs: Arc::new(Mutex::new(HashMap::new())),
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

        async fn save_caps_if_unchanged(
            &self,
            expected: &IndexerConfig,
            snapshot: &str,
        ) -> AppResult<bool> {
            let mut configs = self.created.lock().await;
            let Some(current) = configs.iter_mut().find(|config| config.id == expected.id) else {
                return Ok(false);
            };
            if current.updated_at != expected.updated_at
                || (current.caps_snapshot_json != expected.caps_snapshot_json
                    && current.caps_snapshot_json.as_deref() != Some(snapshot))
            {
                return Ok(false);
            }
            current.caps_snapshot_json = Some(snapshot.into());
            Ok(true)
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

        async fn record_last_error(
            &self,
            id: &str,
            message: Option<String>,
            code: Option<String>,
        ) -> AppResult<()> {
            self.recorded_error_codes.lock().await.push(code);
            self.recorded_errors
                .lock()
                .await
                .push((id.to_string(), message));
            Ok(())
        }

        async fn list_system_backoffs(
            &self,
        ) -> AppResult<HashMap<String, crate::IndexerSystemBackoff>> {
            Ok(self.system_backoffs.lock().await.clone())
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
        seen_client_accounting: Arc<std::sync::Mutex<Vec<Option<crate::IndexerAccountingContext>>>>,
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
        dynamic_trigger_keys: Vec<String>,
        enrichment_caps_json: Option<String>,
        enrichment_requests: Arc<std::sync::Mutex<Vec<HashSet<String>>>>,
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
                seen_client_accounting: Arc::new(std::sync::Mutex::new(Vec::new())),
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
                dynamic_trigger_keys: Vec::new(),
                enrichment_caps_json: None,
                enrichment_requests: Arc::new(std::sync::Mutex::new(Vec::new())),
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

        fn with_validation_result(status: &str, message: Option<&str>) -> Self {
            let mut provider = Self::with_validate_config_support();
            provider.validate_result = crate::IndexerValidationResult {
                status: status.to_string(),
                message: message.map(|message| message.to_string()),
                retry_after_seconds: None,
            };
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
        enrichment_caps_json: Option<String>,
        enrichment_requests: Arc<std::sync::Mutex<Vec<HashSet<String>>>>,
    }

    #[async_trait]
    impl IndexerManagementClient for RecordingIndexerManagementClient {
        async fn enrichment_sync_plan(
            &self,
            _parent_config_id: &str,
            caps_due: &HashSet<String>,
        ) -> AppResult<Option<crate::IndexerSyncPlan>> {
            let Some(caps_json) = self.enrichment_caps_json.clone() else {
                return Ok(None);
            };
            self.enrichment_requests
                .lock()
                .unwrap()
                .push(caps_due.clone());
            let mut plan = self.sync_plan.clone();
            for child in &mut plan.children {
                child.caps_snapshot_json = caps_due
                    .contains(&child.child_key)
                    .then(|| caps_json.clone());
            }
            Ok(Some(plan))
        }

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

        fn client_for_provider_with_accounting(
            &self,
            config: &IndexerConfig,
            _proxy_config: Option<&scryer_domain::ProxyConfig>,
            accounting: Option<&crate::IndexerAccountingContext>,
        ) -> Option<Arc<dyn IndexerClient>> {
            self.seen_client_accounting
                .lock()
                .unwrap()
                .push(accounting.cloned());
            self.client_for_provider(config)
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
                enrichment_caps_json: self.enrichment_caps_json.clone(),
                enrichment_requests: self.enrichment_requests.clone(),
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

        fn dynamic_config_trigger_keys_for_provider(&self, provider_type: &str) -> Vec<String> {
            if provider_type == self.provider_type {
                self.dynamic_trigger_keys.clone()
            } else {
                Vec::new()
            }
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
            ..Default::default()
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
            ..Default::default()
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
                jwt_signing_salt: "test-salt".to_string(),
            },
            Arc::new(FacetRegistry::new()),
        )
    }

    fn test_app_with_proxy_configs(
        indexer_configs: Arc<dyn IndexerConfigRepository>,
        plugin_provider: Option<Arc<dyn IndexerPluginProvider>>,
        settings: Arc<dyn SettingsRepository>,
        proxy_configs: Arc<dyn crate::ProxyConfigRepository>,
    ) -> AppUseCase {
        let services = AppServices::builder(
            Arc::new(NullTitleRepository),
            Arc::new(NullShowRepository),
            Arc::new(NullUserRepository),
            indexer_configs,
            Arc::new(NullIndexerClient),
            Arc::new(NullDownloadClient),
            Arc::new(NullDownloadClientConfigRepository),
            Arc::new(NullReleaseAttemptRepository),
            settings,
            Arc::new(NullQualityProfileRepository),
            String::new(),
        )
        .with_proxy_config_store(proxy_configs);
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
                jwt_signing_salt: "test-salt".to_string(),
            },
            Arc::new(FacetRegistry::new()),
        )
    }

    /// A proxy repository holding exactly one proxy.
    struct SingleProxyRepository(scryer_domain::ProxyConfig);

    fn solver_proxy_config(id: &str) -> scryer_domain::ProxyConfig {
        let now = Utc::now();
        scryer_domain::ProxyConfig {
            id: id.to_string(),
            name: "Solver".into(),
            provider_type: scryer_domain::ProxyProviderType::Byparr,
            protocol: Some(scryer_domain::ChallengeSolverProtocol::RequestSolutionV1),
            base_url: "http://solver.internal:8191".into(),
            request_timeout_seconds: 5,
            is_enabled: true,
            username_encrypted: None,
            password_encrypted: None,
            remote_dns: false,
            private_key_encrypted: None,
            private_key_passphrase_encrypted: None,
            peer_public_key: None,
            preshared_key_encrypted: None,
            tunnel_public_key: None,
            tunnel_addresses: Vec::new(),
            tunnel_dns_servers: Vec::new(),
            tunnel_mtu: None,
            tunnel_keepalive_seconds: None,
            host_key_fingerprint: None,
            host_key_pinned_at: None,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[async_trait]
    impl crate::ProxyConfigRepository for SingleProxyRepository {
        async fn list(
            &self,
            _: Option<scryer_domain::ProxyProviderType>,
        ) -> AppResult<Vec<scryer_domain::ProxyConfig>> {
            Ok(vec![self.0.clone()])
        }
        async fn get_by_id(&self, id: &str) -> AppResult<Option<scryer_domain::ProxyConfig>> {
            Ok((id == self.0.id).then(|| self.0.clone()))
        }
        async fn create(
            &self,
            config: scryer_domain::ProxyConfig,
        ) -> AppResult<scryer_domain::ProxyConfig> {
            Ok(config)
        }
        async fn update(
            &self,
            config: scryer_domain::ProxyConfig,
        ) -> AppResult<scryer_domain::ProxyConfig> {
            Ok(config)
        }
        async fn delete(&self, _: &str) -> AppResult<()> {
            Ok(())
        }
        async fn record_health(
            &self,
            _: &str,
            _: scryer_domain::ProxyHealthStatus,
            _: Option<String>,
            _: Option<chrono::DateTime<Utc>>,
        ) -> AppResult<()> {
            Ok(())
        }
        async fn pin_host_key(
            &self,
            _: &str,
            _: &str,
            _: chrono::DateTime<Utc>,
            _: chrono::DateTime<Utc>,
        ) -> AppResult<bool> {
            Ok(true)
        }
        async fn clear_host_key(&self, _: &str) -> AppResult<()> {
            Ok(())
        }
    }

    /// Waits for the queued background sync to call `plan_sync` and then to
    /// finish applying its plan. Every sync holds `managed_indexer_sync_lock`
    /// across both, and tokio's mutex is FIFO, so taking the lock here waits
    /// out that sync and any other sync already queued behind it; the exact
    /// count afterwards is therefore a settled observation.
    async fn wait_for_plan_sync_calls(
        app: &AppUseCase,
        provider: &RecordingPluginProvider,
        expected_calls: usize,
    ) {
        crate::test_wait::wait_until(
            "the background sync to call plan_sync",
            move || async move { *provider.plan_sync_calls.lock().unwrap() >= expected_calls },
        )
        .await;
        drop(
            crate::test_wait::within_deadline(
                "the background sync to release the managed indexer sync lock",
                app.runtime.integrations.managed_indexer_sync_lock.lock(),
            )
            .await,
        );
        assert_eq!(*provider.plan_sync_calls.lock().unwrap(), expected_calls);
    }

    async fn expect_indexers_changed(
        receiver: &mut tokio::sync::broadcast::Receiver<()>,
        context: &str,
    ) {
        tokio::time::timeout(crate::test_wait::TEST_WAIT_DEADLINE, receiver.recv())
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
    async fn prowlarr_parent_rejects_solver_on_create_update_and_test() {
        let provider = Arc::new(RecordingPluginProvider::new(
            "prowlarr",
            vec![string_field(
                "base_url",
                "Base URL",
                Some(scryer_domain::ConfigFieldRole::ConnectionUrl),
            )],
            searchable_capabilities(),
            Arc::new(RecordingIndexerClient::new(false)),
        ));
        let app = test_app_with_proxy_configs(
            Arc::new(RecordingIndexerConfigRepo::new()),
            Some(provider),
            Arc::new(NullSettingsRepository),
            Arc::new(SingleProxyRepository(solver_proxy_config("solver"))),
        );
        let input = NewIndexerConfig {
            name: "Prowlarr".into(),
            provider_type: "prowlarr".into(),
            rate_limit_seconds: None,
            rate_limit_burst: None,
            max_queries_per_minute: None,
            is_enabled: true,
            enable_interactive_search: false,
            enable_auto_search: false,
            proxy_config_id: Some("solver".into()),
            download_client_id: None,
            config_json: Some(r#"{"base_url":"http://localhost:9696"}"#.into()),
        };
        let error = app
            .create_indexer_config(&test_admin(), input.clone())
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Prowlarr indexers cannot use challenge solvers")
        );
        let error = app
            .test_indexer_connection(
                &test_admin(),
                "prowlarr",
                input.config_json.as_deref(),
                None,
                Some(Some("solver")),
            )
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Prowlarr indexers cannot use challenge solvers")
        );
        let created = app
            .create_indexer_config(
                &test_admin(),
                NewIndexerConfig {
                    proxy_config_id: None,
                    ..input
                },
            )
            .await
            .unwrap();
        let error = app
            .update_indexer_config(
                &test_admin(),
                IndexerConfigUpdate {
                    id: created.id,
                    proxy_config_id: Some(Some("solver".into())),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Prowlarr indexers cannot use challenge solvers")
        );
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
                    max_queries_per_minute: None,
                    is_enabled: true,
                    enable_interactive_search: true,
                    enable_auto_search: true,
                    proxy_config_id: None,
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
                max_queries_per_minute: None,
                is_enabled: true,
                enable_interactive_search: true,
                enable_auto_search: true,
                proxy_config_id: None,
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
                    max_queries_per_minute: None,
                    is_enabled: true,
                    enable_interactive_search: true,
                    enable_auto_search: true,
                    proxy_config_id: None,
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
            "validation: indexer connection test failed: forced failure"
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
            max_queries_per_minute: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_code: None,
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
                    max_queries_per_minute: None,
                    is_enabled: None,
                    enable_interactive_search: None,
                    enable_auto_search: None,
                    proxy_config_id: None,
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
            "validation: indexer connection test failed: forced failure"
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
            max_queries_per_minute: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_code: None,
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
                    max_queries_per_minute: None,
                    is_enabled: None,
                    enable_interactive_search: None,
                    enable_auto_search: None,
                    proxy_config_id: None,
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
            max_queries_per_minute: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: Some("Last search failed".to_string()),
            last_error_code: None,
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
                max_queries_per_minute: None,
                is_enabled: None,
                enable_interactive_search: None,
                enable_auto_search: None,
                proxy_config_id: None,
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
    async fn deferred_caps_snapshot_preserves_coverage_and_learning() {
        let indexer_repo = Arc::new(RecordingIndexerConfigRepo::new());
        indexer_repo.created.lock().await.push(IndexerConfig {
            id: "cfg-caps".into(),
            name: "Synthetic Indexer".into(),
            provider_type: "newznab".into(),
            base_url: "https://indexer.example.test".into(),
            api_key_encrypted: None,
            rate_limit_seconds: None,
            rate_limit_burst: None,
            max_queries_per_minute: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_code: None,
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
                jwt_signing_salt: "test-salt".into(),
            },
            Arc::new(FacetRegistry::new()),
        );

        let (refreshed, failures) = app
            .refresh_enabled_direct_nab_caps_snapshots(&test_admin())
            .await
            .expect("caps refresh pass should tolerate deferred discovery");

        assert_eq!(refreshed, 0);
        assert!(failures.is_empty());
        assert!(indexer_client.pruned_indexers().is_empty());
        assert!(coverage.pruned_indexers.lock().unwrap().is_empty());
        assert!(indexer_repo.recorded_errors.lock().await.is_empty());
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
            max_queries_per_minute: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_code: Some(crate::error_reasons::indexer_caps::REFRESH_FAILED.into()),
            last_error_message: Some("Synthetic capability failure".into()),
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
        unrelated_error.last_error_code = None;
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

    /// The startup and daily caps pass is unattended traffic. A caps request is
    /// a counted API hit, so an indexer that search dispatch is holding off —
    /// a persisted system backoff (e.g. a Newznab quota wall) or a config-level
    /// `disabled_until` — must not be sent one either. Restarting inside the
    /// window must not buy a send; an expired window refreshes normally.
    #[tokio::test]
    async fn background_caps_refresh_sends_nothing_to_an_indexer_in_active_backoff() {
        let indexer_repo = Arc::new(RecordingIndexerConfigRepo::new());
        let now = Utc::now();
        let base = IndexerConfig {
            id: String::new(),
            name: String::new(),
            provider_type: "newznab".into(),
            base_url: String::new(),
            api_key_encrypted: None,
            rate_limit_seconds: None,
            rate_limit_burst: None,
            max_queries_per_minute: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: Some(r#"{"known":"snapshot"}"#.into()),
            last_health_status: None,
            last_error_code: None,
            last_error_message: None,
            last_error_at: None,
            config_json: None,
            created_at: now,
            updated_at: now,
        };
        let indexer = |id: &str, disabled_until: Option<chrono::DateTime<Utc>>| IndexerConfig {
            id: id.into(),
            name: format!("Synthetic {id}"),
            base_url: format!("https://{id}.example.test"),
            disabled_until,
            ..base.clone()
        };
        indexer_repo.created.lock().await.extend([
            indexer("cfg-quota-backoff", None),
            indexer("cfg-backoff-expired", None),
            indexer(
                "cfg-config-disabled",
                Some(now + chrono::Duration::minutes(10)),
            ),
            indexer("cfg-healthy", None),
        ]);
        indexer_repo.system_backoffs.lock().await.extend([
            (
                "cfg-quota-backoff".to_string(),
                crate::IndexerSystemBackoff {
                    disabled_until: now + chrono::Duration::minutes(10),
                    escalation_level: 2,
                },
            ),
            (
                "cfg-backoff-expired".to_string(),
                crate::IndexerSystemBackoff {
                    disabled_until: now - chrono::Duration::minutes(1),
                    escalation_level: 1,
                },
            ),
        ]);
        let refresher = Arc::new(RecordingCapsSnapshotRefresher::default());
        let indexer_client = Arc::new(RecordingIndexerClient::new(false));
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
        .with_indexer_caps_refresher(refresher.clone())
        .build_partial_for_tests();
        let app = AppUseCase::new(
            services,
            JwtAuthConfig {
                issuer: "test".into(),
                jwt_signing_salt: "test-salt".into(),
            },
            Arc::new(FacetRegistry::new()),
        );

        let (refreshed, failures) = app
            .refresh_enabled_direct_nab_caps_snapshots(&User::new_admin("system-indexer-caps"))
            .await
            .expect("a caps pass that holds off backed-off indexers is not a failure");

        assert_eq!(
            refresher.requested_ids(),
            vec!["cfg-backoff-expired".to_string(), "cfg-healthy".to_string()],
            "only indexers outside an active backoff may be sent a caps request"
        );
        assert_eq!(refreshed, 2);
        assert!(failures.is_empty());
        // Holding off is not a caps failure: the known snapshot, health and
        // learning of a held-off indexer stay exactly as they were. (The two
        // refreshed indexers do prune learning, because their snapshot changed.)
        assert!(indexer_repo.recorded_errors.lock().await.is_empty());
        let pruned = indexer_client.pruned_indexers();
        let stored = indexer_repo.created.lock().await;
        for id in ["cfg-quota-backoff", "cfg-config-disabled"] {
            assert!(!pruned.iter().any(|pruned_id| pruned_id == id));
            let config = stored.iter().find(|config| config.id == id).unwrap();
            assert_eq!(
                config.caps_snapshot_json.as_deref(),
                Some(r#"{"known":"snapshot"}"#)
            );
        }
        assert_eq!(
            indexer_repo.system_backoffs.lock().await["cfg-quota-backoff"].escalation_level,
            2
        );
    }

    #[tokio::test]
    async fn validated_update_persists_the_first_caps_fetch_without_fetching_again() {
        let indexer_repo = Arc::new(RecordingIndexerConfigRepo::new());
        indexer_repo.created.lock().await.push(IndexerConfig {
            id: "cfg-update-caps".into(),
            name: "Synthetic Indexer".into(),
            provider_type: "newznab".into(),
            base_url: "https://indexer.example.test".into(),
            api_key_encrypted: None,
            rate_limit_seconds: None,
            rate_limit_burst: None,
            max_queries_per_minute: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_code: None,
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
        .expect("validated caps should persist without another fetch");

        assert_eq!(indexer_repo.cleared_ids().await, vec!["cfg-update-caps"]);
        assert_eq!(indexer_client.pruned_indexers(), vec!["cfg-update-caps"]);
        assert!(indexer_repo.recorded_errors.lock().await.is_empty());
        let stored = indexer_repo
            .get_by_id("cfg-update-caps")
            .await
            .unwrap()
            .unwrap();
        assert!(super::super::indexer_caps_cache::fresh_snapshot(&stored, Utc::now()).is_some());
    }

    fn proxied_newznab_app(
        indexer_repo: Arc<RecordingIndexerConfigRepo>,
        refresher: Arc<RecordingCapsSnapshotRefresher>,
        proxy: scryer_domain::ProxyConfig,
    ) -> AppUseCase {
        let indexer_client = Arc::new(RecordingIndexerClient::new(false));
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
            indexer_repo,
            indexer_client,
            Arc::new(NullDownloadClient),
            Arc::new(NullDownloadClientConfigRepository),
            Arc::new(NullReleaseAttemptRepository),
            Arc::new(NullSettingsRepository),
            Arc::new(NullQualityProfileRepository),
            String::new(),
        )
        .with_proxy_config_store(Arc::new(SingleProxyRepository(proxy)))
        .with_indexer_caps_refresher(refresher)
        .with_plugin_provider(plugin_provider)
        .build_partial_for_tests();
        AppUseCase::new(
            services,
            JwtAuthConfig {
                issuer: "test".into(),
                jwt_signing_salt: "test-salt".into(),
            },
            Arc::new(FacetRegistry::new()),
        )
    }

    /// The e2e proxy matrix saved a Newznab indexer behind an HTTP proxy and
    /// found three requests in the indexer's log from Scryer's own address:
    /// the save's preflight `HEAD /` and a `t=caps` from each of the save's
    /// connection probe and caps refresh. Only the plugin search went through
    /// the proxy. Every request Scryer issues to that indexer itself — on
    /// create, on update, and in the unattended caps pass — must be handed the
    /// indexer's proxy.
    #[tokio::test]
    async fn every_host_request_for_a_proxied_newznab_indexer_is_given_its_proxy() {
        let indexer_repo = Arc::new(RecordingIndexerConfigRepo::new());
        let refresher = Arc::new(RecordingCapsSnapshotRefresher::default());
        let app = proxied_newznab_app(
            indexer_repo.clone(),
            refresher.clone(),
            transport_proxy_config("house-proxy", "http://proxy.internal:3128"),
        );
        take_preflight_proxy_ids();

        let created = app
            .create_indexer_config(
                &test_admin(),
                NewIndexerConfig {
                    name: "Proxied Newznab".into(),
                    provider_type: "newznab".into(),
                    rate_limit_seconds: None,
                    rate_limit_burst: None,
                    max_queries_per_minute: None,
                    is_enabled: true,
                    enable_interactive_search: true,
                    enable_auto_search: true,
                    proxy_config_id: Some("house-proxy".into()),
                    download_client_id: None,
                    config_json: Some(
                        serde_json::json!({
                            "base_url": "http://newznab.internal:8088",
                            "api_key": "secret",
                        })
                        .to_string(),
                    ),
                },
            )
            .await
            .expect("a proxied indexer saves");
        assert_eq!(created.proxy_config_id.as_deref(), Some("house-proxy"));

        let house_proxy = Some("house-proxy".to_string());
        assert!(
            take_preflight_proxy_ids().is_empty(),
            "Newznab validation has no HEAD preflight"
        );
        assert_eq!(
            refresher.requested_proxy_ids(),
            vec![house_proxy.clone()],
            "the single caps request must retain the assigned proxy"
        );

        app.update_indexer_config(
            &test_admin(),
            IndexerConfigUpdate {
                id: created.id.clone(),
                name: Some("Renamed Proxied Newznab".into()),
                ..Default::default()
            },
        )
        .await
        .expect("rename saves");
        app.refresh_enabled_direct_nab_caps_snapshots(&test_admin())
            .await
            .expect("unattended caps pass");

        assert_eq!(
            refresher.requested_proxy_ids(),
            vec![house_proxy],
            "rename and a fresh unattended pass must not request caps"
        );
    }

    /// Fail closed: a direct Newznab indexer whose assigned proxy is disabled
    /// or gone is not sent a caps request at all — not directly, not through
    /// anything else — and the refresh is recorded as failed.
    #[tokio::test]
    async fn caps_pass_sends_nothing_for_an_indexer_whose_proxy_is_unusable() {
        let now = Utc::now();
        let indexer = |id: &str, proxy_config_id: &str| IndexerConfig {
            id: id.into(),
            name: format!("Synthetic {id}"),
            provider_type: "newznab".into(),
            base_url: format!("http://{id}.internal:8088"),
            api_key_encrypted: None,
            rate_limit_seconds: None,
            rate_limit_burst: None,
            max_queries_per_minute: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            proxy_config_id: Some(proxy_config_id.into()),
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_code: None,
            last_error_message: None,
            last_error_at: None,
            config_json: None,
            created_at: now,
            updated_at: now,
        };
        let indexer_repo = Arc::new(RecordingIndexerConfigRepo::new());
        indexer_repo.created.lock().await.extend([
            indexer("cfg-disabled-proxy", "house-proxy"),
            indexer("cfg-missing-proxy", "deleted-proxy"),
        ]);
        let refresher = Arc::new(RecordingCapsSnapshotRefresher::default());
        let mut disabled = transport_proxy_config("house-proxy", "http://proxy.internal:3128");
        disabled.is_enabled = false;
        let app = proxied_newznab_app(indexer_repo.clone(), refresher.clone(), disabled);

        let (refreshed, failures) = app
            .refresh_enabled_direct_nab_caps_snapshots(&test_admin())
            .await
            .expect("an unusable proxy is a per-indexer failure");

        assert!(
            refresher.requested_ids().is_empty(),
            "no caps request may be sent for an indexer whose proxy is unusable"
        );
        assert_eq!(refreshed, 0);
        assert_eq!(failures.len(), 2, "{failures:?}");
        let errors = indexer_repo.recorded_errors.lock().await;
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(indexer_repo.recorded_error_codes.lock().await.iter().all(
            |code| code.as_deref() == Some(crate::error_reasons::indexer_caps::REFRESH_FAILED)
        ));
    }

    #[tokio::test]
    async fn test_indexer_connection_uses_submitted_config_with_saved_accounting_context() {
        let indexer_repo = Arc::new(RecordingIndexerConfigRepo::new());
        indexer_repo.created.lock().await.push(IndexerConfig {
            id: "cfg-1".to_string(),
            name: "NZBGeek".to_string(),
            provider_type: "nzbgeek".to_string(),
            base_url: "https://api.nzbgeek.info".to_string(),
            api_key_encrypted: None,
            rate_limit_seconds: None,
            rate_limit_burst: None,
            max_queries_per_minute: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_code: None,
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

        app.test_indexer_connection(
            &test_admin(),
            "nzbgeek",
            Some(r#"{"base_url":"https://api.nzbgeek.info","api_key":"submitted-key"}"#),
            Some("cfg-1"),
            None,
        )
        .await
        .expect("submitted config should execute under the saved indexer's accounting context");

        let seen = provider.seen_configs.lock().unwrap().clone();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].id, crate::CONNECTION_TEST_INDEXER_ID);
        assert_eq!(seen[0].base_url, "https://api.nzbgeek.info");
        assert_eq!(
            seen[0].config_json.as_deref(),
            Some(r#"{"base_url":"https://api.nzbgeek.info","api_key":"submitted-key"}"#)
        );
        assert!(
            seen[0].id != "cfg-1",
            "execution must not reuse the saved id"
        );
        let accounting = provider.seen_client_accounting.lock().unwrap().clone();
        assert_eq!(accounting.len(), 1);
        let accounting = accounting[0]
            .as_ref()
            .expect("saved connection tests must retain the saved accounting identity");
        assert_eq!(accounting.indexer_id, "cfg-1");
        assert_eq!(accounting.indexer_name, "NZBGeek");
        assert!(
            indexer_repo.cleared_ids().await.is_empty(),
            "testing an unsaved credential must not clear health of the stored credential"
        );
        assert!(receiver.try_recv().is_err());
    }

    fn challenge_answer_field() -> scryer_domain::ConfigFieldDef {
        scryer_domain::ConfigFieldDef {
            required: false,
            ..string_field(
                "captcha_answer",
                "Challenge Answer",
                Some(scryer_domain::ConfigFieldRole::LoginChallengeAnswer),
            )
        }
    }

    fn saved_challenge_indexer() -> IndexerConfig {
        IndexerConfig {
            id: "cfg-1".to_string(),
            name: "Synthetic Tracker".to_string(),
            provider_type: "synthetic-engine".to_string(),
            base_url: "https://tracker.example".to_string(),
            api_key_encrypted: None,
            rate_limit_seconds: None,
            rate_limit_burst: None,
            max_queries_per_minute: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_code: None,
            last_error_message: None,
            last_error_at: None,
            config_json: Some(
                r#"{"base_url":"https://tracker.example","password":"stored-secret"}"#.to_string(),
            ),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    /// The challenge is fetched with the draft settings, under the probe's
    /// synthetic id, with the stored secret the draft left blank restored, and
    /// nothing about it is saved.
    #[tokio::test]
    async fn login_challenge_runs_the_draft_with_restored_secrets_and_saves_nothing() {
        let indexer_repo = Arc::new(RecordingIndexerConfigRepo::new());
        indexer_repo
            .created
            .lock()
            .await
            .push(saved_challenge_indexer());
        let challenge = crate::IndexerLoginChallenge {
            answer_field_key: "captcha_answer".to_string(),
            content_type: "image/png".to_string(),
            image_base64: "iVBORw==".to_string(),
        };
        let client = Arc::new(
            RecordingIndexerClient::new(false).with_login_challenge(Ok(challenge.clone())),
        );
        let provider = Arc::new(RecordingPluginProvider::new(
            "synthetic-engine",
            vec![
                string_field(
                    "base_url",
                    "Base URL",
                    Some(scryer_domain::ConfigFieldRole::ConnectionUrl),
                ),
                password_field("password", "Password"),
                challenge_answer_field(),
            ],
            searchable_capabilities(),
            client.clone(),
        ));
        let app = test_app(
            indexer_repo.clone(),
            Some(provider.clone()),
            Arc::new(NullSettingsRepository),
        );

        let shown = app
            .indexer_login_challenge(
                &test_admin(),
                "synthetic-engine",
                Some(r#"{"base_url":"https://tracker.example","captcha_answer":""}"#),
                Some("cfg-1"),
                None,
            )
            .await
            .unwrap();
        assert_eq!(shown, Some(challenge));

        let seen = provider.seen_configs.lock().unwrap().clone();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].id, crate::CONNECTION_TEST_INDEXER_ID);
        let config: serde_json::Value =
            serde_json::from_str(seen[0].config_json.as_deref().unwrap()).unwrap();
        assert_eq!(config["password"], "stored-secret");
        assert_eq!(config["base_url"], "https://tracker.example");
        assert!(client.calls.lock().unwrap().is_empty(), "no search runs");
        assert!(indexer_repo.cleared_ids().await.is_empty());
        assert_eq!(
            indexer_repo.created.lock().await.clone()[0].config_json,
            saved_challenge_indexer().config_json,
            "the stored settings are untouched"
        );
    }

    /// A provider with no answer field is never asked, and a refusal keeps the
    /// reason it was raised with.
    #[tokio::test]
    async fn login_challenge_skips_providers_without_an_answer_field_and_keeps_refusal_reasons() {
        let fields = vec![string_field(
            "base_url",
            "Base URL",
            Some(scryer_domain::ConfigFieldRole::ConnectionUrl),
        )];
        let client = Arc::new(RecordingIndexerClient::new(false).with_login_challenge(Ok(
            crate::IndexerLoginChallenge {
                answer_field_key: "captcha_answer".to_string(),
                content_type: "image/png".to_string(),
                image_base64: "iVBORw==".to_string(),
            },
        )));
        let provider = Arc::new(RecordingPluginProvider::new(
            "synthetic-engine",
            fields.clone(),
            searchable_capabilities(),
            client,
        ));
        let app = test_app(
            Arc::new(RecordingIndexerConfigRepo::new()),
            Some(provider.clone()),
            Arc::new(NullSettingsRepository),
        );
        let draft = Some(r#"{"base_url":"https://tracker.example"}"#);
        assert_eq!(
            app.indexer_login_challenge(&test_admin(), "synthetic-engine", draft, None, None)
                .await
                .unwrap(),
            None
        );
        assert!(provider.seen_configs.lock().unwrap().is_empty());

        let refusing =
            Arc::new(RecordingPluginProvider::new(
                "synthetic-engine",
                [fields, vec![challenge_answer_field()]].concat(),
                searchable_capabilities(),
                Arc::new(RecordingIndexerClient::new(false).with_login_challenge(Err(
                    crate::login_challenge_refusal::IMAGE_TYPE_UNSUPPORTED,
                ))),
            ));
        let app = test_app(
            Arc::new(RecordingIndexerConfigRepo::new()),
            Some(refusing),
            Arc::new(NullSettingsRepository),
        );
        let error = app
            .indexer_login_challenge(&test_admin(), "synthetic-engine", draft, None, None)
            .await
            .unwrap_err();
        assert_eq!(
            error.validation_reason(),
            Some(crate::login_challenge_refusal::IMAGE_TYPE_UNSUPPORTED)
        );
    }

    #[tokio::test]
    async fn test_indexer_connection_without_saved_config_has_no_accounting_context() {
        let provider = Arc::new(RecordingPluginProvider::new(
            "newznab",
            vec![string_field(
                "base_url",
                "Base URL",
                Some(scryer_domain::ConfigFieldRole::ConnectionUrl),
            )],
            searchable_capabilities(),
            Arc::new(RecordingIndexerClient::new(false)),
        ));
        let app = test_app(
            Arc::new(RecordingIndexerConfigRepo::new()),
            Some(provider.clone()),
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
        .expect("an unsaved probe should still execute");

        assert_eq!(
            provider.seen_configs.lock().unwrap()[0].id,
            crate::CONNECTION_TEST_INDEXER_ID
        );
        let accounting = provider.seen_client_accounting.lock().unwrap().clone();
        assert_eq!(accounting.len(), 1);
        assert!(accounting[0].is_none());
    }

    #[tokio::test]
    async fn probe_without_connection_url_field_skips_url_validation() {
        let indexer_repo = Arc::new(RecordingIndexerConfigRepo::new());
        let provider = Arc::new(RecordingPluginProvider::new(
            "fixed_endpoint",
            vec![password_field("api_key", "API Key")],
            searchable_capabilities(),
            Arc::new(RecordingIndexerClient::new(false)),
        ));
        let app = test_app(
            indexer_repo.clone(),
            Some(provider.clone()),
            Arc::new(NullSettingsRepository),
        );

        app.test_indexer_connection(
            &test_admin(),
            "fixed_endpoint",
            Some(r#"{"api_key":"synthetic-key"}"#),
            None,
            None,
        )
        .await
        .expect("a provider without a connection URL field should still probe");
        {
            let seen = provider.seen_configs.lock().unwrap();
            assert_eq!(seen.len(), 1);
            assert_eq!(seen[0].base_url, "");
        }

        let created = app
            .create_indexer_config(
                &test_admin(),
                NewIndexerConfig {
                    name: "Fixed Endpoint".to_string(),
                    provider_type: "fixed_endpoint".to_string(),
                    rate_limit_seconds: None,
                    rate_limit_burst: None,
                    max_queries_per_minute: None,
                    is_enabled: true,
                    enable_interactive_search: true,
                    enable_auto_search: true,
                    proxy_config_id: None,
                    download_client_id: None,
                    config_json: Some(r#"{"api_key":"synthetic-key"}"#.to_string()),
                },
            )
            .await
            .expect("an enabled provider without a connection URL field should save");
        assert_eq!(created.base_url, "");
        assert!(created.is_enabled);
    }

    #[tokio::test]
    async fn probe_with_connection_url_field_still_requires_valid_url() {
        let provider = Arc::new(RecordingPluginProvider::new(
            "url_endpoint",
            vec![string_field(
                "base_url",
                "Base URL",
                Some(scryer_domain::ConfigFieldRole::ConnectionUrl),
            )],
            searchable_capabilities(),
            Arc::new(RecordingIndexerClient::new(false)),
        ));
        let app = test_app(
            Arc::new(RecordingIndexerConfigRepo::new()),
            Some(provider.clone()),
            Arc::new(NullSettingsRepository),
        );

        let missing = app
            .test_indexer_connection(&test_admin(), "url_endpoint", Some("{}"), None, None)
            .await
            .unwrap_err();
        assert!(missing.to_string().contains("Base URL is required"));
        let invalid = app
            .test_indexer_connection(
                &test_admin(),
                "url_endpoint",
                Some(r#"{"base_url":"not a url"}"#),
                None,
                None,
            )
            .await
            .unwrap_err();
        assert!(invalid.to_string().contains("invalid base URL"));
        assert!(provider.seen_configs.lock().unwrap().is_empty());
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
            max_queries_per_minute: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_code: None,
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
                    max_queries_per_minute: None,
                    is_enabled: true,
                    enable_interactive_search: true,
                    enable_auto_search: true,
                    proxy_config_id: None,
                    download_client_id: None,
                    config_json: Some(r#"{"base_url":"https://manager.example"}"#.to_string()),
                },
            )
            .await
            .expect("managed parent should save before background sync failure");

        wait_for_plan_sync_calls(&app, &provider, 1).await;
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
            max_queries_per_minute: None,
            disabled_until: None,
            is_enabled: false,
            enable_interactive_search: false,
            enable_auto_search: false,
            proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_code: None,
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
                    max_queries_per_minute: None,
                    is_enabled: Some(true),
                    enable_interactive_search: None,
                    enable_auto_search: None,
                    proxy_config_id: None,
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
        wait_for_plan_sync_calls(&app, &provider, 1).await;
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
    async fn test_indexer_connection_skips_generic_search_after_provider_probe() {
        let client = Arc::new(RecordingIndexerClient::new(false).with_connection_probe());
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

        assert!(client.calls.lock().unwrap().is_empty());
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
            "validation: indexer connection test failed: forced failure"
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
            !crate::indexer_error_history_is_persistable(crate::PREVIEW_MANAGED_SYNC_INDEXER_ID),
            "the managed-sync preview id must be excluded from error history"
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

    /// A status outside the old three-name allowlist used to be masked as
    /// "Internal server error" behind a reference id, leaving the operator
    /// saving the indexer nothing to act on.
    #[tokio::test]
    async fn test_indexer_connection_surfaces_unlisted_validation_status_message() {
        let provider = Arc::new(RecordingPluginProvider::with_validation_result(
            "rate_limited",
            Some("Request limit reached, try again in an hour"),
        ));
        let app = test_app(
            Arc::new(RecordingIndexerConfigRepo::new()),
            Some(provider),
            Arc::new(NullSettingsRepository),
        );

        let error = app
            .test_indexer_connection(
                &test_admin(),
                "torrent_rss",
                Some(r#"{"feed_url":"https://ipt.beelyrics.net/t.rss?u=2203846"}"#),
                None,
                None,
            )
            .await
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "validation: Request limit reached, try again in an hour"
        );
    }

    #[tokio::test]
    async fn test_indexer_connection_names_the_status_when_no_message_is_sent() {
        let provider = Arc::new(RecordingPluginProvider::with_validation_result(
            "quota_exceeded",
            None,
        ));
        let app = test_app(
            Arc::new(RecordingIndexerConfigRepo::new()),
            Some(provider),
            Arc::new(NullSettingsRepository),
        );

        let error = app
            .test_indexer_connection(
                &test_admin(),
                "torrent_rss",
                Some(r#"{"feed_url":"https://ipt.beelyrics.net/t.rss?u=2203846"}"#),
                None,
                None,
            )
            .await
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "validation: indexer reported status 'quota_exceeded'"
        );
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
                    max_queries_per_minute: None,
                    is_enabled: true,
                    enable_interactive_search: true,
                    enable_auto_search: true,
                    proxy_config_id: None,
                    download_client_id: None,
                    config_json: Some(r#"{"base_url":"https://manager.example"}"#.to_string()),
                },
            )
            .await
            .expect("managed parent should queue background sync");

        wait_for_plan_sync_calls(&app, &provider, 1).await;
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
                    max_queries_per_minute: None,
                    is_enabled: true,
                    enable_interactive_search: true,
                    enable_auto_search: true,
                    proxy_config_id: None,
                    download_client_id: None,
                    config_json: Some(r#"{"base_url":"https://manager.example"}"#.to_string()),
                },
            )
            .await
            .expect("create should keep parent when background sync fails");

        wait_for_plan_sync_calls(&app, &provider, 1).await;
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
                    max_queries_per_minute: None,
                    is_enabled: true,
                    enable_interactive_search: true,
                    enable_auto_search: true,
                    proxy_config_id: None,
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
                max_queries_per_minute: None,
                disabled_until: None,
                is_enabled: true,
                enable_interactive_search: false,
                enable_auto_search: false,
                proxy_config_id: None,
                download_client_id: None,
                seeding_profile_id: None,
                managed_parent_config_id: None,
                managed_child_key: None,
                managed_metadata_json: None,
                caps_snapshot_json: None,
                last_health_status: None,
                last_error_code: None,
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
                max_queries_per_minute: None,
                disabled_until: None,
                is_enabled: true,
                enable_interactive_search: true,
                enable_auto_search: true,
                proxy_config_id: None,
                download_client_id: None,
                seeding_profile_id: None,
                managed_parent_config_id: Some("parent".to_string()),
                managed_child_key: Some("child".to_string()),
                managed_metadata_json: None,
                caps_snapshot_json: None,
                last_health_status: None,
                last_error_code: None,
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
                max_queries_per_minute: None,
                disabled_until: None,
                is_enabled: true,
                enable_interactive_search: false,
                enable_auto_search: false,
                proxy_config_id: None,
                download_client_id: None,
                seeding_profile_id: None,
                managed_parent_config_id: None,
                managed_child_key: None,
                managed_metadata_json: Some(
                    r#"{"locally_disabled_children":["keep","upstream-disabled"]}"#.to_string(),
                ),
                caps_snapshot_json: None,
                last_health_status: None,
                last_error_code: None,
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
                max_queries_per_minute: None,
                disabled_until: None,
                is_enabled: false,
                enable_interactive_search: false,
                enable_auto_search: false,
                proxy_config_id: None,
                download_client_id: None,
                seeding_profile_id: None,
                managed_parent_config_id: Some("parent".to_string()),
                managed_child_key: Some("keep".to_string()),
                managed_metadata_json: None,
                caps_snapshot_json: None,
                last_health_status: None,
                last_error_code: None,
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
                max_queries_per_minute: None,
                disabled_until: None,
                is_enabled: true,
                enable_interactive_search: false,
                enable_auto_search: false,
                proxy_config_id: None,
                download_client_id: None,
                seeding_profile_id: None,
                managed_parent_config_id: None,
                managed_child_key: None,
                managed_metadata_json: None,
                caps_snapshot_json: None,
                last_health_status: None,
                last_error_code: None,
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
        wait_for_plan_sync_calls(&app, &provider, 1).await;
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
                max_queries_per_minute: None,
                disabled_until: None,
                is_enabled: true,
                enable_interactive_search: false,
                enable_auto_search: false,
                proxy_config_id: None,
                download_client_id: None,
                seeding_profile_id: None,
                managed_parent_config_id: None,
                managed_child_key: None,
                managed_metadata_json: None,
                caps_snapshot_json: None,
                last_health_status: None,
                last_error_code: None,
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
                max_queries_per_minute: None,
                disabled_until: None,
                is_enabled: false,
                enable_interactive_search: false,
                enable_auto_search: false,
                proxy_config_id: None,
                download_client_id: None,
                seeding_profile_id: None,
                managed_parent_config_id: None,
                managed_child_key: None,
                managed_metadata_json: None,
                caps_snapshot_json: None,
                last_health_status: None,
                last_error_code: None,
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
                max_queries_per_minute: None,
                disabled_until: None,
                is_enabled: true,
                enable_interactive_search: false,
                enable_auto_search: false,
                proxy_config_id: None,
                download_client_id: None,
                seeding_profile_id: None,
                managed_parent_config_id: None,
                managed_child_key: None,
                managed_metadata_json: None,
                caps_snapshot_json: None,
                last_health_status: None,
                last_error_code: None,
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
                max_queries_per_minute: None,
                disabled_until: None,
                is_enabled: true,
                enable_interactive_search: false,
                enable_auto_search: false,
                proxy_config_id: None,
                download_client_id: None,
                seeding_profile_id: None,
                managed_parent_config_id: None,
                managed_child_key: None,
                managed_metadata_json: None,
                caps_snapshot_json: None,
                last_health_status: None,
                last_error_code: None,
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
                max_queries_per_minute: None,
                disabled_until: None,
                is_enabled: true,
                enable_interactive_search: false,
                enable_auto_search: false,
                proxy_config_id: None,
                download_client_id: None,
                seeding_profile_id: None,
                managed_parent_config_id: None,
                managed_child_key: None,
                managed_metadata_json: None,
                caps_snapshot_json: None,
                last_health_status: None,
                last_error_code: None,
                last_error_message: None,
                last_error_at: None,
                config_json: Some(r#"{"base_url":"https://manager.example"}"#.to_string()),
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();

        // Sync only persists routing scopes it changes, so the plan seeds a
        // new scope to guarantee a system routing write for the barrier.
        let provider = Arc::new(RecordingPluginProvider::with_sync_plan(
            crate::IndexerSyncPlan {
                children: vec![crate::ManagedIndexerChildPlan {
                    child_key: "seeded-child".to_string(),
                    name: "Seeded Child".to_string(),
                    provider_type: "torrent_rss".to_string(),
                    config_json: r#"{"feed_url":"https://seeded.example/rss"}"#.to_string(),
                    is_enabled: true,
                    enable_interactive_search: true,
                    enable_auto_search: true,
                    managed_metadata_json: None,
                    caps_snapshot_json: None,
                    routing_scopes: vec![crate::ManagedIndexerRoutingScope {
                        scope_id: "anime".to_string(),
                        categories: vec!["5070".to_string()],
                    }],
                }],
            },
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
            max_queries_per_minute: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: false,
            enable_auto_search: false,
            proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_code: None,
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
            max_queries_per_minute: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: Some(r#"{"locally_disabled_children":["keep"]}"#.to_string()),
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_code: None,
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
            max_queries_per_minute: None,
            disabled_until: None,
            is_enabled: false,
            enable_interactive_search: false,
            enable_auto_search: false,
            proxy_config_id: None,
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
            last_error_code: None,
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
            max_queries_per_minute: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: Some(parent.id.clone()),
            managed_child_key: Some("delete".to_string()),
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_code: None,
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
        // Existing categories belong to the operator; the plan only seeds new routes.
        assert_eq!(keep_movie.categories, vec!["1111"]);
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
            max_queries_per_minute: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: false,
            enable_auto_search: false,
            proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_code: None,
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
            max_queries_per_minute: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: Some(parent.id.clone()),
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_code: None,
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

    fn managed_test_child(
        id: &str,
        caps_snapshot_json: Option<String>,
        is_enabled: bool,
        disabled_until: Option<chrono::DateTime<Utc>>,
    ) -> IndexerConfig {
        let now = Utc::now();
        IndexerConfig {
            id: id.to_string(),
            name: format!("Synthetic {id}"),
            provider_type: "torrent_rss".to_string(),
            base_url: format!("https://{id}.example/rss"),
            api_key_encrypted: None,
            rate_limit_seconds: None,
            rate_limit_burst: None,
            max_queries_per_minute: None,
            disabled_until,
            is_enabled,
            enable_interactive_search: true,
            enable_auto_search: true,
            proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: Some("parent".to_string()),
            managed_child_key: Some(id.to_string()),
            managed_metadata_json: None,
            caps_snapshot_json,
            last_health_status: None,
            last_error_code: None,
            last_error_message: None,
            last_error_at: None,
            config_json: Some(format!(r#"{{"feed_url":"https://{id}.example/rss"}}"#)),
            created_at: now,
            updated_at: now,
        }
    }

    #[tokio::test]
    async fn managed_caps_pass_fetches_only_due_children_and_then_nothing() {
        let indexer_repo = Arc::new(RecordingIndexerConfigRepo::new());
        let now = Utc::now();
        let mut parent = managed_test_child("parent", None, true, None);
        parent.provider_type = "manager".to_string();
        parent.managed_parent_config_id = None;
        parent.managed_child_key = None;
        parent.enable_interactive_search = false;
        parent.enable_auto_search = false;
        parent.config_json = Some(r#"{"base_url":"https://manager.example"}"#.to_string());
        indexer_repo.create(parent).await.unwrap();
        let snapshot_at = |at| {
            Some(
                crate::integration::indexer_caps_cache::serialize_snapshot(
                    &scryer_domain::IndexerCapsSnapshot::default(),
                    at,
                )
                .unwrap(),
            )
        };
        for child in [
            managed_test_child("fresh", snapshot_at(now), true, None),
            managed_test_child(
                "expired",
                snapshot_at(now - chrono::Duration::days(8)),
                true,
                None,
            ),
            managed_test_child("new", None, true, None),
            managed_test_child(
                "cooling",
                None,
                true,
                Some(now + chrono::Duration::hours(1)),
            ),
            managed_test_child("off", None, false, None),
        ] {
            indexer_repo.create(child).await.unwrap();
        }
        let mut provider = RecordingPluginProvider::with_sync_plan(crate::IndexerSyncPlan {
            children: ["fresh", "expired", "new", "cooling", "off"]
                .into_iter()
                .map(|key| crate::ManagedIndexerChildPlan {
                    child_key: key.to_string(),
                    name: format!("Synthetic {key}"),
                    provider_type: "torrent_rss".to_string(),
                    config_json: format!(r#"{{"feed_url":"https://{key}.example/rss"}}"#),
                    is_enabled: true,
                    enable_interactive_search: true,
                    enable_auto_search: true,
                    managed_metadata_json: None,
                    caps_snapshot_json: None,
                    routing_scopes: Vec::new(),
                })
                .collect(),
        });
        provider.enrichment_caps_json =
            Some(serde_json::to_string(&scryer_domain::IndexerCapsSnapshot::default()).unwrap());
        let provider = Arc::new(provider);
        let app = test_app(
            indexer_repo.clone(),
            Some(provider.clone()),
            Arc::new(RecordingSettingsRepository::default()),
        );

        app.enrich_managed_indexer_children(&test_admin(), "parent")
            .await
            .unwrap();
        assert_eq!(
            provider.enrichment_requests.lock().unwrap().clone(),
            vec![HashSet::from(["expired".to_string(), "new".to_string()])],
            "only children with a missing or expired snapshot that may be contacted are due"
        );
        for id in ["expired", "new"] {
            let stored = indexer_repo.get_by_id(id).await.unwrap().unwrap();
            assert!(
                crate::integration::indexer_caps_cache::fresh_snapshot(&stored, Utc::now())
                    .is_some(),
                "{id} stores a dated snapshot"
            );
        }

        app.enrich_managed_indexer_children(&test_admin(), "parent")
            .await
            .unwrap();
        assert_eq!(
            provider.enrichment_requests.lock().unwrap().len(),
            1,
            "a pass with every snapshot fresh must not ask for caps"
        );
    }

    #[tokio::test]
    async fn a_managed_child_that_gets_no_caps_is_not_asked_again_on_the_next_pass() {
        let indexer_repo = Arc::new(RecordingIndexerConfigRepo::new());
        let mut parent = managed_test_child("parent", None, true, None);
        parent.provider_type = "manager".to_string();
        parent.managed_parent_config_id = None;
        parent.managed_child_key = None;
        parent.enable_interactive_search = false;
        parent.enable_auto_search = false;
        parent.config_json = Some(r#"{"base_url":"https://manager.example"}"#.to_string());
        indexer_repo.create(parent).await.unwrap();
        indexer_repo
            .create(managed_test_child("silent", None, true, None))
            .await
            .unwrap();
        // The manager's plan never carries this child, so no caps come back.
        let mut provider = RecordingPluginProvider::with_sync_plan(crate::IndexerSyncPlan {
            children: Vec::new(),
        });
        provider.enrichment_caps_json =
            Some(serde_json::to_string(&scryer_domain::IndexerCapsSnapshot::default()).unwrap());
        let provider = Arc::new(provider);
        let app = test_app(
            indexer_repo.clone(),
            Some(provider.clone()),
            Arc::new(RecordingSettingsRepository::default()),
        );

        app.enrich_managed_indexer_children(&test_admin(), "parent")
            .await
            .unwrap();
        app.enrich_managed_indexer_children(&test_admin(), "parent")
            .await
            .unwrap();

        assert_eq!(
            provider.enrichment_requests.lock().unwrap().clone(),
            vec![HashSet::from(["silent".to_string()])],
            "an unanswered child waits out its retry instead of being asked on every pass"
        );

        indexer_repo
            .update(crate::IndexerConfigUpdate {
                id: "silent".to_string(),
                derived_base_url: Some("https://silent-moved.example".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        app.enrich_managed_indexer_children(&test_admin(), "parent")
            .await
            .unwrap();
        assert_eq!(
            provider.enrichment_requests.lock().unwrap().len(),
            2,
            "a connection change makes the child due again at once"
        );
    }

    #[tokio::test]
    async fn managed_child_connection_change_makes_its_caps_due() {
        let indexer_repo = Arc::new(RecordingIndexerConfigRepo::new());
        let mut parent = managed_test_child("parent", None, true, None);
        parent.provider_type = "manager".to_string();
        parent.managed_parent_config_id = None;
        parent.managed_child_key = None;
        parent.enable_interactive_search = false;
        parent.enable_auto_search = false;
        parent.config_json = Some(r#"{"base_url":"https://manager.example"}"#.to_string());
        indexer_repo.create(parent).await.unwrap();
        let fresh = crate::integration::indexer_caps_cache::serialize_snapshot(
            &scryer_domain::IndexerCapsSnapshot::default(),
            Utc::now(),
        )
        .unwrap();
        for id in ["moved", "same"] {
            indexer_repo
                .create(managed_test_child(id, Some(fresh.clone()), true, None))
                .await
                .unwrap();
        }
        let plan_child = |key: &str, feed: &str| crate::ManagedIndexerChildPlan {
            child_key: key.to_string(),
            name: format!("Synthetic {key}"),
            provider_type: "torrent_rss".to_string(),
            config_json: format!(r#"{{"feed_url":"{feed}"}}"#),
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            routing_scopes: Vec::new(),
        };
        let sync_with = |moved_feed: &'static str| {
            let provider = Arc::new(RecordingPluginProvider::with_sync_plan(
                crate::IndexerSyncPlan {
                    children: vec![
                        plan_child("moved", moved_feed),
                        plan_child("same", "https://same.example/rss"),
                    ],
                },
            ));
            test_app(
                indexer_repo.clone(),
                Some(provider),
                Arc::new(RecordingSettingsRepository::default()),
            )
        };
        // The first sync settles each child on its synced form; only a later
        // change to that form is a connection change.
        sync_with("https://moved.example/rss")
            .sync_indexer_config(&test_admin(), "parent")
            .await
            .unwrap();
        for id in ["moved", "same"] {
            indexer_repo
                .update(crate::IndexerConfigUpdate {
                    id: id.to_string(),
                    caps_snapshot_json: Some(Some(fresh.clone())),
                    ..Default::default()
                })
                .await
                .unwrap();
        }

        sync_with("https://elsewhere.example/rss")
            .sync_indexer_config(&test_admin(), "parent")
            .await
            .unwrap();

        let moved = indexer_repo.get_by_id("moved").await.unwrap().unwrap();
        assert!(
            moved.caps_snapshot_json.is_some(),
            "stale caps stay searchable"
        );
        assert!(
            crate::integration::indexer_caps_cache::fresh_snapshot(&moved, Utc::now()).is_none()
        );
        let same = indexer_repo.get_by_id("same").await.unwrap().unwrap();
        assert_eq!(same.caps_snapshot_json.as_deref(), Some(fresh.as_str()));
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
                max_queries_per_minute: None,
                disabled_until: None,
                is_enabled: true,
                enable_interactive_search: false,
                enable_auto_search: false,
                proxy_config_id: None,
                download_client_id: None,
                seeding_profile_id: None,
                managed_parent_config_id: None,
                managed_child_key: None,
                managed_metadata_json: None,
                caps_snapshot_json: None,
                last_health_status: None,
                last_error_code: None,
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
    async fn managed_child_indexers_allow_only_local_field_updates() {
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
                max_queries_per_minute: None,
                disabled_until: None,
                is_enabled: true,
                enable_interactive_search: false,
                enable_auto_search: false,
                proxy_config_id: None,
                download_client_id: None,
                seeding_profile_id: None,
                managed_parent_config_id: None,
                managed_child_key: None,
                managed_metadata_json: None,
                caps_snapshot_json: None,
                last_health_status: None,
                last_error_code: None,
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
                max_queries_per_minute: None,
                disabled_until: None,
                is_enabled: true,
                enable_interactive_search: true,
                enable_auto_search: true,
                proxy_config_id: None,
                download_client_id: None,
                seeding_profile_id: None,
                managed_parent_config_id: Some("parent".to_string()),
                managed_child_key: Some("child".to_string()),
                managed_metadata_json: None,
                caps_snapshot_json: None,
                last_health_status: None,
                last_error_code: None,
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

        // Rate limits and search flags are the child's own, alongside enabled.
        let rate_limited = app
            .update_indexer_config(
                &test_admin(),
                IndexerConfigUpdate {
                    id: "child".to_string(),
                    rate_limit_seconds: Some(15),
                    rate_limit_burst: Some(3),
                    enable_auto_search: Some(false),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(rate_limited.rate_limit_seconds, Some(15));
        assert_eq!(rate_limited.rate_limit_burst, Some(3));
        assert!(!rate_limited.enable_auto_search);
        assert!(
            !rate_limited.is_enabled,
            "a local edit leaves enabled alone"
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
    fn cache_config() -> IndexerConfig {
        serde_json::from_value(serde_json::json!({
            "id": "cache-config", "name": "Synthetic indexer", "provider_type": "newznab",
            "base_url": "https://indexer.example.test",
            "config_json": r#"{"base_url":"https://indexer.example.test","api_key":"synthetic-key"}"#,
            "is_enabled": true, "enable_interactive_search": true, "enable_auto_search": true,
            "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z"
        })).unwrap()
    }

    fn cache_app(
        repo: Arc<RecordingIndexerConfigRepo>,
        refresher: Arc<dyn IndexerCapsSnapshotRefresher>,
        client: Arc<RecordingIndexerClient>,
    ) -> AppUseCase {
        let provider = Arc::new(RecordingPluginProvider::new(
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
            client.clone(),
        ));
        let services = AppServices::builder(
            Arc::new(NullTitleRepository),
            Arc::new(NullShowRepository),
            Arc::new(NullUserRepository),
            repo,
            client,
            Arc::new(NullDownloadClient),
            Arc::new(NullDownloadClientConfigRepository),
            Arc::new(NullReleaseAttemptRepository),
            Arc::new(NullSettingsRepository),
            Arc::new(NullQualityProfileRepository),
            String::new(),
        )
        .with_plugin_provider(provider)
        .with_indexer_caps_refresher(refresher)
        .build_partial_for_tests();
        AppUseCase::new(
            services,
            JwtAuthConfig {
                issuer: "test".into(),
                jwt_signing_salt: "test-salt".into(),
            },
            Arc::new(FacetRegistry::new()),
        )
    }

    #[tokio::test]
    async fn local_edits_and_unchanged_masked_form_preserve_caps_health_and_backoff_without_requests()
     {
        let repo = Arc::new(RecordingIndexerConfigRepo::new());
        let mut config = cache_config();
        config.caps_snapshot_json = Some("legacy snapshot".into());
        config.last_error_message = Some("prior failure".into());
        config.disabled_until = Some(Utc::now() + chrono::Duration::days(1));
        repo.created.lock().await.push(config.clone());
        let refresher = Arc::new(RecordingCapsSnapshotRefresher::default());
        let client = Arc::new(RecordingIndexerClient::new(true));
        let app = cache_app(repo.clone(), refresher.clone(), client.clone());
        let changes = [
            IndexerConfigUpdate {
                name: Some("Renamed".into()),
                ..Default::default()
            },
            // Already enabled, so this is not a re-enable: re-enabling a
            // disabled indexer probes it on purpose.
            IndexerConfigUpdate {
                is_enabled: Some(true),
                ..Default::default()
            },
            IndexerConfigUpdate {
                rate_limit_seconds: Some(10),
                rate_limit_burst: Some(5),
                max_queries_per_minute: None,
                ..Default::default()
            },
            IndexerConfigUpdate {
                enable_auto_search: Some(false),
                enable_interactive_search: Some(false),
                ..Default::default()
            },
            IndexerConfigUpdate {
                download_client_id: Some(None),
                ..Default::default()
            },
            IndexerConfigUpdate {
                provider_type: Some(" NEWZNAB ".into()),
                config_json: Some(
                    r#"{"api_key":"********","base_url":" https://indexer.example.test "}"#.into(),
                ),
                proxy_config_id: Some(None),
                ..Default::default()
            },
            IndexerConfigUpdate {
                is_enabled: Some(false),
                ..Default::default()
            },
        ];
        for mut update in changes {
            update.id = config.id.clone();
            let saved = app
                .update_indexer_config(&test_admin(), update)
                .await
                .unwrap();
            assert_eq!(saved.caps_snapshot_json, config.caps_snapshot_json);
            assert_eq!(saved.last_error_message, config.last_error_message);
            assert_eq!(saved.disabled_until, config.disabled_until);
        }
        assert!(refresher.requested_ids().is_empty());
        assert!(client.calls.lock().unwrap().is_empty());
        assert!(repo.cleared_ids().await.is_empty());
    }

    #[tokio::test]
    async fn cold_test_fetches_once_warm_test_and_restart_only_test_feed() {
        let repo = Arc::new(RecordingIndexerConfigRepo::new());
        let config = cache_config();
        repo.created.lock().await.push(config.clone());
        let refresher = Arc::new(RecordingCapsSnapshotRefresher::default());
        let client = Arc::new(RecordingIndexerClient::new(false));
        for _ in 0..2 {
            let app = cache_app(repo.clone(), refresher.clone(), client.clone());
            app.test_indexer_connection(
                &test_admin(),
                "newznab",
                config.config_json.as_deref(),
                Some(&config.id),
                None,
            )
            .await
            .unwrap();
            assert_eq!(
                app.refresh_enabled_direct_nab_caps_snapshots(&test_admin())
                    .await
                    .unwrap(),
                (0, vec![])
            );
        }
        assert_eq!(refresher.requested_ids(), vec![config.id]);
        assert_eq!(client.calls.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn disabled_connection_edit_invalidates_caps_without_testing_then_enable_probes() {
        let repo = Arc::new(RecordingIndexerConfigRepo::new());
        let mut config = cache_config();
        config.is_enabled = false;
        config.caps_snapshot_json = Some(
            super::super::indexer_caps_cache::serialize_snapshot(
                &scryer_domain::IndexerCapsSnapshot::default(),
                Utc::now(),
            )
            .unwrap(),
        );
        repo.created.lock().await.push(config.clone());
        let refresher = Arc::new(RecordingCapsSnapshotRefresher::default());
        let client = Arc::new(RecordingIndexerClient::new(true));
        let app = cache_app(repo.clone(), refresher.clone(), client.clone());
        let saved = app
            .update_indexer_config(
                &test_admin(),
                IndexerConfigUpdate {
                    id: config.id.clone(),
                    config_json: Some(
                        r#"{"base_url":"https://other.example.test","api_key":"new-key"}"#.into(),
                    ),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(saved.caps_snapshot_json.is_none());
        assert!(client.calls.lock().unwrap().is_empty());

        // Re-enabling is a "try again": it probes the saved connection, and a
        // failed probe refuses the enable instead of saving it.
        let error = app
            .update_indexer_config(
                &test_admin(),
                IndexerConfigUpdate {
                    id: config.id.clone(),
                    is_enabled: Some(true),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "validation: indexer connection test failed: forced failure"
        );
        assert_eq!(client.calls.lock().unwrap().len(), 1);
        assert!(
            !repo
                .get_by_id(&config.id)
                .await
                .unwrap()
                .unwrap()
                .is_enabled
        );
    }

    #[tokio::test]
    async fn enabled_connection_change_never_reuses_old_caps_and_failed_validation_preserves_saved_config()
     {
        let repo = Arc::new(RecordingIndexerConfigRepo::new());
        let config = cache_config();
        repo.created.lock().await.push(config.clone());
        let refresher = Arc::new(SuccessfulValidationThenFailingCapsSnapshotRefresher::new());
        let client = Arc::new(RecordingIndexerClient::new(false));
        let app = cache_app(repo.clone(), refresher.clone(), client);
        app.test_indexer_connection(
            &test_admin(),
            "newznab",
            config.config_json.as_deref(),
            Some(&config.id),
            None,
        )
        .await
        .unwrap();
        let saved = repo.get_by_id(&config.id).await.unwrap().unwrap();
        let error = app
            .update_indexer_config(
                &test_admin(),
                IndexerConfigUpdate {
                    id: config.id.clone(),
                    config_json: Some(
                        r#"{"base_url":"https://other.example.test","api_key":"new-key"}"#.into(),
                    ),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("synthetic caps failure"));
        assert_eq!(refresher.calls.load(Ordering::SeqCst), 2);
        assert_eq!(repo.get_by_id(&config.id).await.unwrap().unwrap(), saved);
        assert_eq!(repo.cleared_ids().await, vec![config.id]);
    }
    #[tokio::test]
    async fn create_enabled_requests_feed_and_caps_once_disabled_create_requests_neither() {
        for enabled in [false, true] {
            let repo = Arc::new(RecordingIndexerConfigRepo::new());
            let refresher = Arc::new(RecordingCapsSnapshotRefresher::default());
            let client = Arc::new(RecordingIndexerClient::new(false));
            let app = cache_app(repo, refresher.clone(), client.clone());
            let saved = app
                .create_indexer_config(
                    &test_admin(),
                    NewIndexerConfig {
                        name: "Synthetic indexer".into(),
                        provider_type: "newznab".into(),
                        rate_limit_seconds: None,
                        rate_limit_burst: None,
                        max_queries_per_minute: None,
                        is_enabled: enabled,
                        enable_interactive_search: true,
                        enable_auto_search: true,
                        proxy_config_id: None,
                        download_client_id: None,
                        config_json: cache_config().config_json,
                    },
                )
                .await
                .unwrap();
            assert_eq!(client.calls.lock().unwrap().len(), usize::from(enabled));
            assert_eq!(refresher.requested_ids().len(), usize::from(enabled));
            assert_eq!(saved.caps_snapshot_json.is_some(), enabled);
        }
    }

    #[tokio::test]
    async fn failed_background_refresh_preserves_expired_snapshot_and_its_timestamp() {
        let repo = Arc::new(RecordingIndexerConfigRepo::new());
        let mut config = cache_config();
        config.caps_snapshot_json = Some(
            super::super::indexer_caps_cache::serialize_snapshot(
                &scryer_domain::IndexerCapsSnapshot::default(),
                Utc::now() - chrono::Duration::days(8),
            )
            .unwrap(),
        );
        repo.created.lock().await.push(config.clone());
        let refresher = Arc::new(SuccessfulValidationThenFailingCapsSnapshotRefresher::new());
        refresher.calls.store(1, Ordering::SeqCst);
        let app = cache_app(
            repo.clone(),
            refresher,
            Arc::new(RecordingIndexerClient::new(false)),
        );
        let (count, failures) = app
            .refresh_enabled_direct_nab_caps_snapshots(&test_admin())
            .await
            .unwrap();
        assert_eq!(count, 0);
        assert_eq!(failures.len(), 1);
        let stored = repo.get_by_id(&config.id).await.unwrap().unwrap();
        assert_eq!(stored.caps_snapshot_json, config.caps_snapshot_json);
        assert!(super::super::indexer_caps_cache::fresh_snapshot(&stored, Utc::now()).is_none());
    }

    struct HeldCapsRefresher {
        calls: AtomicUsize,
        release: Semaphore,
    }

    impl Default for HeldCapsRefresher {
        fn default() -> Self {
            Self {
                calls: AtomicUsize::new(0),
                release: Semaphore::new(0),
            }
        }
    }

    #[async_trait]
    impl IndexerCapsSnapshotRefresher for HeldCapsRefresher {
        async fn fetch_for_config(
            &self,
            _config: &IndexerConfig,
            _proxy: Option<&scryer_domain::ProxyConfig>,
        ) -> AppResult<Option<scryer_domain::IndexerCapsSnapshot>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.release.acquire().await.unwrap().forget();
            Ok(Some(scryer_domain::IndexerCapsSnapshot::default()))
        }
    }

    fn assert_pending(future: std::pin::Pin<&mut impl std::future::Future>) {
        assert!(
            future
                .poll(&mut std::task::Context::from_waker(std::task::Waker::noop()))
                .is_pending()
        );
    }

    #[tokio::test]
    async fn test_save_and_background_caps_calls_share_only_matching_connections() {
        let repo = Arc::new(RecordingIndexerConfigRepo::new());
        let config = cache_config();
        repo.created.lock().await.push(config.clone());
        let refresher = Arc::new(HeldCapsRefresher::default());
        let app = cache_app(
            repo.clone(),
            refresher.clone(),
            Arc::new(RecordingIndexerClient::new(false)),
        );
        let actor = test_admin();
        let test = app.test_indexer_connection(
            &actor,
            "newznab",
            config.config_json.as_deref(),
            Some(&config.id),
            None,
        );
        let background = app.refresh_enabled_direct_nab_caps_snapshots(&actor);
        tokio::pin!(test, background);
        assert_pending(test.as_mut());
        assert_eq!(refresher.calls.load(Ordering::SeqCst), 1);
        assert_pending(background.as_mut());
        refresher.release.add_permits(1);
        let (test, background) = tokio::time::timeout(Duration::from_secs(10), async {
            tokio::join!(test, background)
        })
        .await
        .unwrap();
        test.unwrap();
        assert_eq!(background.unwrap(), (1, vec![]));
        assert_eq!(refresher.calls.load(Ordering::SeqCst), 1);

        let changed = r#"{"base_url":"https://indexer.example.test","api_key":"replacement"}"#;
        let save = app.update_indexer_config(
            &actor,
            IndexerConfigUpdate {
                id: config.id.clone(),
                config_json: Some(changed.into()),
                ..Default::default()
            },
        );
        let test =
            app.test_indexer_connection(&actor, "newznab", Some(changed), Some(&config.id), None);
        tokio::pin!(save, test);
        assert_pending(save.as_mut());
        assert_eq!(refresher.calls.load(Ordering::SeqCst), 2);
        assert_pending(test.as_mut());
        refresher.release.add_permits(1);
        let (save, test) =
            tokio::time::timeout(Duration::from_secs(10), async { tokio::join!(save, test) })
                .await
                .unwrap();
        assert!(save.unwrap().caps_snapshot_json.is_some());
        test.unwrap();
        assert_eq!(refresher.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn delayed_test_does_not_clear_health_recorded_while_it_was_running() {
        let repo = Arc::new(RecordingIndexerConfigRepo::new());
        let config = cache_config();
        repo.created.lock().await.push(config.clone());
        let refresher = Arc::new(HeldCapsRefresher::default());
        let app = cache_app(
            repo.clone(),
            refresher.clone(),
            Arc::new(RecordingIndexerClient::new(false)),
        );
        let actor = test_admin();
        let test = app.test_indexer_connection(
            &actor,
            "newznab",
            config.config_json.as_deref(),
            Some(&config.id),
            None,
        );
        tokio::pin!(test);
        assert_pending(test.as_mut());
        {
            let mut configs = repo.created.lock().await;
            configs[0].last_error_at = Some(Utc::now());
            configs[0].last_error_message = Some("New search failure".into());
        }
        refresher.release.add_permits(1);
        tokio::time::timeout(Duration::from_secs(10), test)
            .await
            .unwrap()
            .unwrap();
        assert!(repo.cleared_ids().await.is_empty());
        assert_eq!(
            repo.get_by_id(&config.id)
                .await
                .unwrap()
                .unwrap()
                .last_error_message
                .as_deref(),
            Some("New search failure"),
        );
    }

    #[tokio::test]
    async fn delayed_background_caps_cannot_restore_an_invalidated_snapshot() {
        let repo = Arc::new(RecordingIndexerConfigRepo::new());
        let config = cache_config();
        repo.created.lock().await.push(config.clone());
        let refresher = Arc::new(HeldCapsRefresher::default());
        let app = cache_app(
            repo.clone(),
            refresher.clone(),
            Arc::new(RecordingIndexerClient::new(false)),
        );
        let actor = test_admin();
        let background = app.refresh_enabled_direct_nab_caps_snapshots(&actor);
        tokio::pin!(background);
        assert_pending(background.as_mut());
        assert_eq!(refresher.calls.load(Ordering::SeqCst), 1);
        let saved = app
            .update_indexer_config(
                &actor,
                IndexerConfigUpdate {
                    id: config.id.clone(),
                    is_enabled: Some(false),
                    config_json: Some(
                        r#"{"base_url":"https://indexer.example.test","api_key":"replacement"}"#
                            .into(),
                    ),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        refresher.release.add_permits(1);
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(10), background)
                .await
                .unwrap()
                .unwrap(),
            (0, vec![])
        );
        assert_eq!(repo.get_by_id(&config.id).await.unwrap().unwrap(), saved);
    }

    mod dynamic_config_fields {
        use super::*;
        use std::collections::BTreeMap;

        fn dynamic_answer() -> crate::PluginDynamicConfigFields {
            crate::PluginDynamicConfigFields {
                fields: vec![
                    scryer_domain::ConfigFieldDef {
                        key: "cookie".into(),
                        label: "Cookie".into(),
                        field_type: scryer_domain::ConfigFieldType::Password,
                        required: true,
                        ..Default::default()
                    },
                    scryer_domain::ConfigFieldDef {
                        key: "sort".into(),
                        label: "Sort".into(),
                        field_type: scryer_domain::ConfigFieldType::Select,
                        options: vec![scryer_domain::ConfigFieldOption {
                            value: "new".into(),
                            label: "Newest".into(),
                            config_overrides: Default::default(),
                        }],
                        ..Default::default()
                    },
                ],
                hidden_field_keys: vec!["username".into()],
            }
        }

        fn dynamic_app(
            trigger_keys: &[&str],
            client: RecordingIndexerClient,
        ) -> (
            AppUseCase,
            Arc<RecordingIndexerConfigRepo>,
            Arc<RecordingIndexerClient>,
        ) {
            let client = Arc::new(client);
            let mut provider = RecordingPluginProvider::new(
                "cardigann",
                vec![
                    string_field("definition", "Definition", None),
                    string_field("username", "Username", None),
                    password_field("api_key", "API key"),
                ],
                searchable_capabilities(),
                client.clone(),
            );
            provider.fields[2].required = false;
            provider.dynamic_trigger_keys =
                trigger_keys.iter().map(|key| key.to_string()).collect();
            let repo = Arc::new(RecordingIndexerConfigRepo::new());
            let app = test_app(
                repo.clone(),
                Some(Arc::new(provider)),
                Arc::new(NullSettingsRepository),
            );
            (app, repo, client)
        }

        fn new_config(config_json: &str) -> NewIndexerConfig {
            NewIndexerConfig {
                name: "Tracker".into(),
                provider_type: "cardigann".into(),
                rate_limit_seconds: None,
                rate_limit_burst: None,
                max_queries_per_minute: None,
                is_enabled: false,
                enable_interactive_search: true,
                enable_auto_search: true,
                proxy_config_id: None,
                download_client_id: None,
                config_json: Some(config_json.into()),
            }
        }

        fn stored_value(config: &IndexerConfig, key: &str) -> Option<String> {
            serde_json::from_str::<serde_json::Value>(config.config_json.as_deref()?)
                .ok()?
                .get(key)?
                .as_str()
                .map(str::to_string)
        }

        #[tokio::test]
        async fn query_sends_only_trigger_values_and_caches_per_value() {
            let (app, _, client) = dynamic_app(
                &["definition", "api_key"],
                RecordingIndexerClient::new(false).with_dynamic_fields(dynamic_answer()),
            );
            let values = BTreeMap::from([
                ("definition".to_string(), "alpha".to_string()),
                ("username".to_string(), "someone".to_string()),
                ("api_key".to_string(), "hunter2".to_string()),
            ]);

            let first = app
                .indexer_dynamic_config_fields(&test_admin(), "cardigann", &values)
                .await
                .unwrap();
            let second = app
                .indexer_dynamic_config_fields(&test_admin(), "cardigann", &values)
                .await
                .unwrap();

            assert_eq!(first, dynamic_answer());
            assert_eq!(second, first);
            // A password field is never a trigger, so the key never reaches
            // the plugin, and the repeat is answered from the cache.
            assert_eq!(
                client.dynamic_calls(),
                vec![BTreeMap::from([(
                    "definition".to_string(),
                    "alpha".to_string()
                )])]
            );

            let beta = BTreeMap::from([("definition".to_string(), "beta".to_string())]);
            app.indexer_dynamic_config_fields(&test_admin(), "cardigann", &beta)
                .await
                .unwrap();
            assert_eq!(client.dynamic_calls().len(), 2);
        }

        #[tokio::test]
        async fn a_blocked_plugin_answers_the_form_empty_but_keeps_stored_secrets_redacted() {
            let (app, _, client) = dynamic_app(
                &["definition"],
                RecordingIndexerClient::new(false).with_dynamic_fields(dynamic_answer()),
            );
            app.runtime
                .plugins
                .compatibility_blockers
                .write()
                .await
                .insert("cardigann".into(), "synthetic incompatibility".into());

            let values = BTreeMap::from([("definition".to_string(), "alpha".to_string())]);
            let answer = app
                .indexer_dynamic_config_fields(&test_admin(), "cardigann", &values)
                .await
                .unwrap();
            assert_eq!(answer, crate::PluginDynamicConfigFields::default());

            // A stored config still carries the dynamic secret the plugin added
            // before it was blocked; without the plugin to ask, that key must be
            // treated as an optional secret, not dropped from redaction.
            let fields = app
                .effective_indexer_config_fields(
                    "cardigann",
                    &[Some(r#"{"definition":"alpha","cookie":"c1"}"#)],
                )
                .await
                .unwrap();
            let cookie = fields
                .iter()
                .find(|field| field.key == "cookie")
                .expect("the unknown stored key is listed");
            assert_eq!(cookie.field_type, scryer_domain::ConfigFieldType::Password);
            assert!(!cookie.required);
            assert!(
                client.dynamic_calls().is_empty(),
                "a blocked plugin is never asked"
            );
        }

        #[tokio::test]
        async fn an_unblocked_plugin_names_its_stored_dynamic_fields() {
            let (app, _, _) = dynamic_app(
                &["definition"],
                RecordingIndexerClient::new(false).with_dynamic_fields(dynamic_answer()),
            );
            let fields = app
                .effective_indexer_config_fields(
                    "cardigann",
                    &[Some(r#"{"definition":"alpha","cookie":"c1"}"#)],
                )
                .await
                .unwrap();
            let expected = dynamic_answer()
                .fields
                .into_iter()
                .find(|field| field.key == "cookie")
                .expect("the answer declares the cookie");
            assert_eq!(
                fields.iter().find(|field| field.key == "cookie"),
                Some(&expected)
            );
        }

        #[tokio::test]
        async fn provider_without_trigger_keys_answers_empty_without_starting_plugin() {
            let (app, _, client) = dynamic_app(
                &[],
                RecordingIndexerClient::new(false).with_dynamic_fields(dynamic_answer()),
            );
            let values = BTreeMap::from([("definition".to_string(), "alpha".to_string())]);

            let answer = app
                .indexer_dynamic_config_fields(&test_admin(), "cardigann", &values)
                .await
                .unwrap();

            assert_eq!(answer, crate::PluginDynamicConfigFields::default());
            assert!(client.dynamic_calls().is_empty());
        }

        #[tokio::test]
        async fn unsupported_answer_is_empty_not_an_error() {
            let (app, _, client) = dynamic_app(&["definition"], RecordingIndexerClient::new(false));
            let values = BTreeMap::from([("definition".to_string(), "alpha".to_string())]);

            let answer = app
                .indexer_dynamic_config_fields(&test_admin(), "cardigann", &values)
                .await
                .unwrap();

            assert_eq!(answer, crate::PluginDynamicConfigFields::default());
            assert_eq!(client.dynamic_calls().len(), 1);
        }

        #[tokio::test]
        async fn answer_reusing_a_static_key_is_refused_whole() {
            let mut answer = dynamic_answer();
            answer.fields[1].key = "username".into();
            let (app, _, _) = dynamic_app(
                &["definition"],
                RecordingIndexerClient::new(false).with_dynamic_fields(answer),
            );
            let values = BTreeMap::from([("definition".to_string(), "alpha".to_string())]);

            let error = app
                .indexer_dynamic_config_fields(&test_admin(), "cardigann", &values)
                .await
                .unwrap_err();

            assert!(
                error.to_string().contains("reuses a static field key"),
                "{error}"
            );
        }

        #[tokio::test]
        async fn save_demands_dynamic_required_fields_but_not_hidden_static_ones() {
            let (app, _, _) = dynamic_app(
                &["definition"],
                RecordingIndexerClient::new(false).with_dynamic_fields(dynamic_answer()),
            );

            let error = app
                .create_indexer_config(&test_admin(), new_config(r#"{"definition":"alpha"}"#))
                .await
                .unwrap_err();
            assert!(error.to_string().contains("Cookie is required"), "{error}");

            // `username` is required statically but hidden for this
            // definition, so the save must not ask for it.
            let created = app
                .create_indexer_config(
                    &test_admin(),
                    new_config(r#"{"definition":"alpha","cookie":"c1","sort":"new"}"#),
                )
                .await
                .unwrap();
            assert_eq!(stored_value(&created, "cookie").as_deref(), Some("c1"));
            assert_eq!(stored_value(&created, "sort").as_deref(), Some("new"));
        }

        #[tokio::test]
        async fn edit_restores_stored_dynamic_secret_and_reuses_stored_trigger_value() {
            let (app, _, client) = dynamic_app(
                &["definition"],
                RecordingIndexerClient::new(false).with_dynamic_fields(dynamic_answer()),
            );
            let created = app
                .create_indexer_config(
                    &test_admin(),
                    new_config(r#"{"definition":"alpha","cookie":"c1"}"#),
                )
                .await
                .unwrap();

            // The edit form omits a stored secret it did not change, and may
            // omit the trigger too; both come back from the stored config.
            let updated = app
                .update_indexer_config(
                    &test_admin(),
                    IndexerConfigUpdate {
                        id: created.id.clone(),
                        config_json: Some(r#"{"sort":"new"}"#.into()),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            assert_eq!(stored_value(&updated, "cookie").as_deref(), Some("c1"));
            assert_eq!(
                stored_value(&updated, "definition").as_deref(),
                Some("alpha")
            );
            assert!(
                client.dynamic_calls().iter().all(|values| {
                    values.get("definition").map(String::as_str) == Some("alpha")
                })
            );

            // Payloads redact the dynamic secret exactly like a static one.
            let fields = app.indexer_config_fields_for_config(&updated).await;
            let cookie = fields.iter().find(|field| field.key == "cookie").unwrap();
            assert_eq!(cookie.field_type, scryer_domain::ConfigFieldType::Password);
            let username = fields.iter().find(|field| field.key == "username").unwrap();
            assert!(!username.required);
        }

        #[tokio::test]
        async fn unreachable_plugin_treats_unknown_stored_keys_as_secrets() {
            let (app, _, _) = dynamic_app(
                &["definition"],
                RecordingIndexerClient::new(false).with_dynamic_fields({
                    let mut broken = dynamic_answer();
                    broken.hidden_field_keys = vec!["definition".into()];
                    broken
                }),
            );

            let fields = app
                .effective_indexer_config_fields(
                    "cardigann",
                    &[Some(r#"{"definition":"alpha","cookie":"c1"}"#)],
                )
                .await
                .unwrap();

            let cookie = fields.iter().find(|field| field.key == "cookie").unwrap();
            assert_eq!(cookie.field_type, scryer_domain::ConfigFieldType::Password);
            assert!(!cookie.required);
        }
    }
}

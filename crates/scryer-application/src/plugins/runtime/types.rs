/// A single rule template within a community rule pack.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RulePackTemplate {
    pub id: String,
    pub title: String,
    pub description: String,
    pub category: String,
    pub rego_source: String,
    #[serde(default)]
    pub applied_facets: Vec<String>,
}
#[derive(Clone, Debug, Deserialize)]
struct RulePackRule {
    id: String,
    title: String,
    description: String,
    category: String,
    #[serde(alias = "regoSource")]
    rego_source: String,
    #[serde(default, alias = "appliedFacets")]
    applied_facets: Vec<String>,
}
fn normalized_constraint(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|constraint| !constraint.is_empty())
        .map(str::to_string)
}
fn current_sdk_version() -> &'static semver::Version {
    static VERSION: LazyLock<semver::Version> = LazyLock::new(|| {
        semver::Version::parse(SDK_VERSION).expect("SDK_VERSION must be valid semver")
    });
    &VERSION
}
const SQLITE_PLUGIN_WASM_ZSTD_LEVEL: i32 = 3;
const LEGACY_INDEXER_PLUGIN_TYPE: &str = "indexer";
const USENET_INDEXER_PLUGIN_TYPE: &str = "usenet_indexer";
const TORRENT_INDEXER_PLUGIN_TYPE: &str = "torrent_indexer";
const CURRENT_SCRYER_VERSION: &str = env!("CARGO_PKG_VERSION");
fn current_scryer_version() -> &'static semver::Version {
    static VERSION: LazyLock<semver::Version> = LazyLock::new(|| {
        semver::Version::parse(CURRENT_SCRYER_VERSION)
            .expect("CARGO_PKG_VERSION must be a valid semver version")
    });
    &VERSION
}
fn is_indexer_plugin_type(plugin_type: &str) -> bool {
    matches!(
        plugin_type,
        LEGACY_INDEXER_PLUGIN_TYPE | USENET_INDEXER_PLUGIN_TYPE | TORRENT_INDEXER_PLUGIN_TYPE
    )
}
fn normalize_provider_key(provider_type: &str) -> String {
    provider_type.trim().to_ascii_lowercase()
}
fn is_reserved_first_party_provider(provider_type: &str) -> bool {
    provider_type.trim().eq_ignore_ascii_case("prowlarr")
}
fn lifecycle_status_label(status: PluginLifecycleStatus) -> String {
    match status {
        PluginLifecycleStatus::Beta => "beta".to_string(),
        PluginLifecycleStatus::Active => "active".to_string(),
        PluginLifecycleStatus::Deprecated => "deprecated".to_string(),
    }
}
fn plugin_type_belongs_to_indexer_family(plugin_type: &str) -> bool {
    matches!(
        plugin_type,
        "indexer" | "usenet_indexer" | "torrent_indexer"
    )
}
fn plugin_request_policy(
    scope: impl Into<String>,
    request_label: impl Into<String>,
) -> RequestPolicy {
    RequestPolicy::safe_read(scope.into(), request_label.into())
        .with_max_retries(2)
        .with_backoff(Duration::from_secs(1), Duration::from_secs(30))
        .without_redirects()
}
fn primary_and_mirrors(primary_url: &str, mirror_urls: &[String]) -> Vec<String> {
    std::iter::once(primary_url.to_string())
        .chain(mirror_urls.iter().cloned())
        .collect()
}
impl AppUseCase {
    fn default_base_url_for_plugin(
        &self,
        plugin_type: &str,
        provider_type: &str,
    ) -> Option<String> {
        match plugin_type {
            "download_client" => self
                .services
                .integrations
                .download_client_plugin_provider
                .available()
                .and_then(|provider| provider.default_base_url_for_provider(provider_type)),
            _ if is_indexer_plugin_type(plugin_type) => self
                .services
                .integrations
                .plugin_provider
                .available()
                .and_then(|provider| provider.default_base_url_for_provider(provider_type)),
            _ => None,
        }
    }
}
impl AppUseCase {
    /// Reload runtime plugin providers from database state + builtins.
    pub async fn reload_plugin_providers(&self) -> AppResult<()> {
        let enabled = self
            .services
            .customization
            .plugin_installations
            .get_enabled_plugin_wasm_bytes()
            .await?;

        let mut runtime_plugins = Vec::new();
        let mut pending_plugins = enabled.into_iter().filter_map(|(installation, payload)| {
            if !matches!(
                installation.source_kind,
                PluginSourceKind::Downloaded
                    | PluginSourceKind::Community
                    | PluginSourceKind::Manual
            ) {
                return None;
            }
            if !installation_sdk_contract_is_host_compatible(&installation) {
                return None;
            }
            if installation_is_host_blocked(&installation) {
                return None;
            }

            payload.map(|payload| (installation, payload))
        });
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..RUNTIME_PLUGIN_LOAD_CONCURRENCY {
            let Some((installation, payload)) = pending_plugins.next() else {
                break;
            };
            tasks.spawn(async move {
                let plugin_id = installation.plugin_id.clone();
                let version = installation.version.clone();
                let loaded = load_runtime_plugin_from_persisted_installation_payload(
                    &installation,
                    &payload,
                )
                .await;
                (plugin_id, version, loaded)
            });
        }
        while let Some(result) = tasks.join_next().await {
            let (plugin_id, version, loaded) = result.map_err(|error| {
                AppError::Repository(format!("runtime plugin load task panicked: {error}"))
            })?;
            match loaded {
                Ok(runtime_plugin) => runtime_plugins.push(runtime_plugin),
                Err(error) => {
                    warn!(
                        plugin_id = plugin_id.as_str(),
                        version = version.as_str(),
                        error = %error,
                        "skipping installed plugin after persisted payload validation failed"
                    );
                }
            }

            if let Some((installation, payload)) = pending_plugins.next() {
                tasks.spawn(async move {
                    let plugin_id = installation.plugin_id.clone();
                    let version = installation.version.clone();
                    let loaded = load_runtime_plugin_from_persisted_installation_payload(
                        &installation,
                        &payload,
                    )
                    .await;
                    (plugin_id, version, loaded)
                });
            }
        }
        let indexer_plugins = runtime_plugins
            .iter()
            .filter(|plugin| plugin_type_belongs_to_indexer_family(plugin.descriptor.plugin_type()))
            .cloned()
            .collect::<Vec<_>>();
        let download_client_plugins = runtime_plugins
            .iter()
            .filter(|plugin| plugin.descriptor.plugin_type() == "download_client")
            .cloned()
            .collect::<Vec<_>>();
        let subtitle_plugins = runtime_plugins
            .iter()
            .filter(|plugin| plugin.descriptor.plugin_type() == "subtitle_provider")
            .cloned()
            .collect::<Vec<_>>();
        let archive_extractor_plugins = runtime_plugins
            .iter()
            .filter(|plugin| plugin.descriptor.plugin_type() == "archive_extractor")
            .cloned()
            .collect::<Vec<_>>();
        let notification_plugins = runtime_plugins
            .iter()
            .filter(|plugin| plugin.descriptor.plugin_type() == "notification")
            .cloned()
            .collect::<Vec<_>>();

        // Collect provider_types of builtins the user has disabled
        // (must query all installations, not just enabled ones)
        let all_installations = self
            .services
            .customization
            .plugin_installations
            .list_plugin_installations()
            .await?;
        let disabled_builtins: Vec<String> = all_installations
            .iter()
            .filter(|inst| {
                inst.is_builtin
                    && !inst.is_enabled
                    && !is_reserved_first_party_provider(&inst.provider_type)
            })
            .map(|inst| inst.provider_type.clone())
            .collect();

        if let Some(provider) = self.services.integrations.plugin_provider.available() {
            provider
                .reload_runtime_plugins(&indexer_plugins, &disabled_builtins)
                .map_err(|e| {
                    AppError::Repository(format!("failed to reload plugin provider: {e}"))
                })?;
        }

        if let Some(provider) = self
            .services
            .integrations
            .download_client_plugin_provider
            .available()
        {
            provider
                .reload_runtime_plugins(&download_client_plugins, &disabled_builtins)
                .map_err(|e| {
                    AppError::Repository(format!(
                        "failed to reload download client plugin provider: {e}"
                    ))
                })?;
        }

        if let Some(provider) = self
            .services
            .integrations
            .subtitle_plugin_provider
            .available()
        {
            provider
                .reload_runtime_plugins(&subtitle_plugins, &disabled_builtins)
                .map_err(|e| {
                    AppError::Repository(format!("failed to reload subtitle plugin provider: {e}"))
                })?;
        }

        if let Some(provider) = self
            .services
            .integrations
            .archive_extractor_plugin_provider
            .available()
        {
            provider
                .reload_runtime_plugins(&archive_extractor_plugins, &disabled_builtins)
                .map_err(|e| {
                    AppError::Repository(format!(
                        "failed to reload archive extractor plugin provider: {e}"
                    ))
                })?;
        }

        // Also rebuild notification plugin provider
        if let Some(notif_provider) = self.services.notifications.notification_provider() {
            notif_provider
                .reload_runtime_plugins(&notification_plugins, &disabled_builtins)
                .map_err(|e| {
                    AppError::Repository(format!(
                        "failed to reload notification plugin provider: {e}"
                    ))
                })?;
        }

        Ok(())
    }
}
impl AppUseCase {
    /// Rebuild the runtime plugin providers and rules engine from the latest plugin state.
    pub async fn rebuild_plugin_provider(&self) -> AppResult<()> {
        self.reload_plugin_providers().await?;
        self.seed_builtin_plugins().await?;
        self.rebuild_user_rules_engine().await?;
        Ok(())
    }
}
impl AppUseCase {
    /// Ensure every auto-provisionable indexer plugin with a default connection URL
    /// has at least one IndexerConfig. This covers the case where a plugin was
    /// installed before the auto-create logic existed, or when the registry was
    /// stale at install time.
    pub async fn reconcile_indexer_configs(&self) -> AppResult<()> {
        self.reconcile_orphaned_managed_indexer_configs().await?;
        let Some(provider) = self.services.integrations.plugin_provider.available() else {
            return Ok(());
        };

        let now = Utc::now();
        for pt in provider.available_provider_types() {
            let fields = provider.config_fields_for_provider(&pt);
            let Some(connection_field) = fields
                .iter()
                .find(|field| field.role == Some(scryer_domain::ConfigFieldRole::ConnectionUrl))
            else {
                continue;
            };
            let Some(default_url) = provider.default_base_url_for_provider(&pt) else {
                continue;
            };
            if !indexer_config_can_be_auto_created(&fields) {
                continue;
            }
            let existing = self
                .services
                .integrations
                .indexer_configs
                .list(Some(pt.clone()))
                .await
                .unwrap_or_default();
            if existing.is_empty() {
                let name = provider
                    .plugin_name_for_provider(&pt)
                    .unwrap_or_else(|| pt.clone());
                let config = IndexerConfig {
                    id: Id::new().0,
                    name,
                    provider_type: pt.clone(),
                    base_url: default_url.clone(),
                    api_key_encrypted: None,
                    is_enabled: true,
                    enable_interactive_search: true,
                    enable_auto_search: true,
                    rate_limit_seconds: provider.rate_limit_seconds_for_provider(&pt),
                    rate_limit_burst: None,
                    disabled_until: None,
                    proxy_config_id: None,
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
                            connection_field.key.clone(): default_url,
                        })
                        .to_string(),
                    ),
                    created_at: now,
                    updated_at: now,
                };
                if let Err(e) = self
                    .services
                    .integrations
                    .indexer_configs
                    .create(config)
                    .await
                {
                    tracing::warn!(
                        error = %e,
                        provider_type = pt.as_str(),
                        "failed to auto-create indexer config during reconciliation"
                    );
                } else {
                    tracing::info!(
                        provider_type = pt.as_str(),
                        "auto-created indexer config for plugin"
                    );
                }
            }
        }
        Ok(())
    }
}
impl AppUseCase {
    pub async fn plugin_update_count(&self, actor: &User) -> AppResult<i64> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        Ok(self
            .build_available_plugins()
            .await?
            .into_iter()
            .filter(|plugin| plugin.update_available)
            .count() as i64)
    }
}
impl AppUseCase {
    pub async fn prime_plugin_trust_roots_internal(&self) -> AppResult<()> {
        let _ = self;
        super::catalog::prime_sigstore_trust_roots().await
    }
}

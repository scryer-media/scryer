use super::*;
use chrono::Utc;
use ring::digest as ring_digest;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use tracing::warn;

/// Registry plugin entry merged with local installation state.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegistryPlugin {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub plugin_type: String,
    pub provider_type: String,
    pub author: String,
    pub official: bool,
    pub builtin: bool,
    pub source_url: Option<String>,
    pub wasm_url: Option<String>,
    pub wasm_sha256: Option<String>,
    pub min_scryer_version: Option<String>,
    /// Merged from local installation state.
    pub is_installed: bool,
    pub is_enabled: bool,
    pub installed_version: Option<String>,
    /// True when the registry version is newer than the installed version.
    pub update_available: bool,
    /// When set, installing this plugin auto-creates an IndexerConfig with this URL.
    pub default_base_url: Option<String>,
}

/// Raw registry JSON format (matches scryer-plugins/registry.json).
#[derive(Clone, Debug, Deserialize)]
struct RegistryManifest {
    #[allow(dead_code)]
    schema_version: u32,
    plugins: Vec<RegistryEntry>,
}

#[derive(Clone, Debug, Deserialize)]
struct RegistryEntry {
    id: String,
    name: String,
    description: String,
    plugin_type: String,
    provider_type: String,
    version: String,
    #[serde(default)]
    author: String,
    #[serde(default)]
    official: bool,
    #[serde(default)]
    builtin: bool,
    #[serde(default)]
    source_url: Option<String>,
    #[serde(default)]
    wasm_url: Option<String>,
    #[serde(default)]
    wasm_sha256: Option<String>,
    #[serde(default)]
    min_scryer_version: Option<String>,
}

const DEFAULT_REGISTRY_URL: &str =
    "https://raw.githubusercontent.com/scryer-media/scryer-plugins/main/registry.json";

const LEGACY_INDEXER_PLUGIN_TYPE: &str = "indexer";
const USENET_INDEXER_PLUGIN_TYPE: &str = "usenet_indexer";
const TORRENT_INDEXER_PLUGIN_TYPE: &str = "torrent_indexer";
const CURRENT_SCRYER_VERSION: &str = env!("CARGO_PKG_VERSION");

fn current_scryer_version() -> &'static semver::Version {
    static VERSION: OnceLock<semver::Version> = OnceLock::new();
    VERSION.get_or_init(|| {
        semver::Version::parse(CURRENT_SCRYER_VERSION)
            .expect("CARGO_PKG_VERSION must be a valid semver version")
    })
}

fn is_indexer_plugin_type(plugin_type: &str) -> bool {
    matches!(
        plugin_type,
        LEGACY_INDEXER_PLUGIN_TYPE | USENET_INDEXER_PLUGIN_TYPE | TORRENT_INDEXER_PLUGIN_TYPE
    )
}

fn merged_plugin_type(registry_type: &str, installed_type: Option<&str>) -> String {
    match installed_type {
        Some(installed)
            if is_indexer_plugin_type(registry_type) && is_indexer_plugin_type(installed) =>
        {
            if registry_type == LEGACY_INDEXER_PLUGIN_TYPE
                && installed != LEGACY_INDEXER_PLUGIN_TYPE
            {
                installed.to_string()
            } else {
                registry_type.to_string()
            }
        }
        _ => registry_type.to_string(),
    }
}

fn parse_min_scryer_version(entry: &RegistryEntry) -> Option<semver::Version> {
    let min_version = entry.min_scryer_version.as_deref()?.trim();
    if min_version.is_empty() {
        return None;
    }

    match semver::Version::parse(min_version) {
        Ok(version) => Some(version),
        Err(err) => {
            warn!(
                plugin_id = entry.id.as_str(),
                required_version = min_version,
                error = %err,
                "skipping plugin with invalid min_scryer_version"
            );
            None
        }
    }
}

fn registry_entry_is_host_compatible(entry: &RegistryEntry) -> bool {
    match entry.min_scryer_version.as_deref().map(str::trim) {
        None | Some("") => true,
        Some(_) => parse_min_scryer_version(entry)
            .is_some_and(|required| current_scryer_version() >= &required),
    }
}

fn ensure_registry_entry_is_host_compatible(entry: &RegistryEntry) -> AppResult<()> {
    let Some(required_raw) = entry.min_scryer_version.as_deref().map(str::trim) else {
        return Ok(());
    };
    if required_raw.is_empty() {
        return Ok(());
    }

    let required = semver::Version::parse(required_raw).map_err(|err| {
        AppError::Validation(format!(
            "plugin '{}' requires an invalid min_scryer_version '{}': {err}",
            entry.id, required_raw
        ))
    })?;

    let current = current_scryer_version();
    if current < &required {
        return Err(AppError::Validation(format!(
            "plugin '{}' requires Scryer {} but current Scryer is {}",
            entry.id, required, current
        )));
    }

    Ok(())
}

impl AppUseCase {
    /// Seed database rows for built-in plugins. Uses INSERT OR IGNORE so
    /// existing user toggles are preserved across restarts.
    pub async fn seed_builtin_plugins(&self) -> AppResult<()> {
        let repo = &self.services.plugin_installations;
        repo.seed_builtin(
            "nzbgeek",
            "NZBGeek Indexer",
            "NZBGeek-specific Newznab indexer with metadata extraction (thumbs, subtitles, password detection)",
            "0.1.0",
            "nzbgeek",
        )
        .await?;
        repo.seed_builtin(
            "newznab",
            "Newznab Indexer",
            "Generic Newznab protocol indexer for DogNZB and other compatible services",
            "0.1.0",
            "newznab",
        )
        .await?;
        Ok(())
    }

    /// Rebuild the plugin provider from database state + builtins.
    pub async fn rebuild_plugin_provider(&self) -> AppResult<()> {
        let enabled = self
            .services
            .plugin_installations
            .get_enabled_plugin_wasm_bytes()
            .await?;

        // Collect WASM bytes for user-installed (non-builtin) enabled plugins
        let external_bytes: Vec<Vec<u8>> = enabled
            .iter()
            .filter(|(inst, _)| !inst.is_builtin)
            .filter_map(|(_, wasm)| wasm.clone())
            .collect();
        let external_refs: Vec<&[u8]> = external_bytes.iter().map(|b| b.as_slice()).collect();

        // Collect provider_types of builtins the user has disabled
        // (must query all installations, not just enabled ones)
        let all_installations = self
            .services
            .plugin_installations
            .list_plugin_installations()
            .await?;
        let disabled_builtins: Vec<String> = all_installations
            .iter()
            .filter(|inst| inst.is_builtin && !inst.is_enabled)
            .map(|inst| inst.provider_type.clone())
            .collect();

        if let Some(ref provider) = self.services.plugin_provider {
            provider
                .reload_plugins(&external_refs, &disabled_builtins)
                .map_err(|e| {
                    AppError::Repository(format!("failed to reload plugin provider: {e}"))
                })?;
        }

        if let Some(ref provider) = self.services.download_client_plugin_provider {
            provider
                .reload_plugins(&external_refs, &disabled_builtins)
                .map_err(|e| {
                    AppError::Repository(format!(
                        "failed to reload download client plugin provider: {e}"
                    ))
                })?;
        }

        // Also rebuild notification plugin provider
        if let Some(ref notif_provider) = self.services.notification_provider {
            notif_provider
                .reload_plugins(&external_refs, &disabled_builtins)
                .map_err(|e| {
                    AppError::Repository(format!(
                        "failed to reload notification plugin provider: {e}"
                    ))
                })?;
        }

        // Rebuild rules engine to pick up new/removed scoring policies
        self.rebuild_user_rules_engine().await?;
        Ok(())
    }

    /// Ensure every auto-provisionable indexer plugin with a `default_base_url`
    /// has at least one IndexerConfig. This covers the case where a plugin was
    /// installed before the auto-create logic existed, or when the registry was
    /// stale at install time.
    pub async fn reconcile_indexer_configs(&self) -> AppResult<()> {
        let Some(ref provider) = self.services.plugin_provider else {
            return Ok(());
        };

        let now = Utc::now();
        for pt in provider.available_provider_types() {
            let Some(default_url) = provider.default_base_url_for_provider(&pt) else {
                continue;
            };
            if should_skip_auto_created_indexer_config(&pt) {
                continue;
            }
            let existing = self
                .services
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
                    base_url: default_url,
                    api_key_encrypted: None,
                    is_enabled: true,
                    enable_interactive_search: true,
                    enable_auto_search: true,
                    rate_limit_seconds: provider.rate_limit_seconds_for_provider(&pt),
                    rate_limit_burst: None,
                    disabled_until: None,
                    last_health_status: None,
                    last_error_at: None,
                    config_json: None,
                    created_at: now,
                    updated_at: now,
                };
                if let Err(e) = self.services.indexer_configs.create(config).await {
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

    /// Returns all available indexer provider types with their config field schemas.
    /// Tuple: (provider_type, name, config_fields, default_base_url)
    pub fn available_indexer_provider_types(
        &self,
    ) -> Vec<(
        String,
        String,
        Vec<scryer_domain::ConfigFieldDef>,
        Option<String>,
    )> {
        let Some(ref provider) = self.services.plugin_provider else {
            return vec![];
        };
        let mut seen = std::collections::HashSet::new();
        provider
            .available_provider_types()
            .into_iter()
            .filter(|pt| seen.insert(pt.clone()))
            .map(|pt| {
                let name = provider
                    .plugin_name_for_provider(&pt)
                    .unwrap_or_else(|| pt.clone());
                let fields = provider.config_fields_for_provider(&pt);
                let default_base_url = provider.default_base_url_for_provider(&pt);
                (pt, name, fields, default_base_url)
            })
            .collect()
    }

    pub fn available_download_client_provider_types(
        &self,
    ) -> Vec<(
        String,
        String,
        Vec<scryer_domain::ConfigFieldDef>,
        Option<String>,
    )> {
        let Some(ref provider) = self.services.download_client_plugin_provider else {
            return vec![];
        };
        let mut seen = std::collections::HashSet::new();
        provider
            .available_provider_types()
            .into_iter()
            .filter(|pt| seen.insert(pt.clone()))
            .map(|pt| {
                let name = provider
                    .plugin_name_for_provider(&pt)
                    .unwrap_or_else(|| pt.clone());
                let fields = provider.config_fields_for_provider(&pt);
                let default_base_url = provider.default_base_url_for_provider(&pt);
                (pt, name, fields, default_base_url)
            })
            .collect()
    }

    /// List available plugins by merging cached registry with local installations.
    pub async fn list_available_plugins(&self, actor: &User) -> AppResult<Vec<RegistryPlugin>> {
        require(actor, &Entitlement::ManageConfig)?;

        let installations = self
            .services
            .plugin_installations
            .list_plugin_installations()
            .await?;

        // Try to parse cached registry
        let registry_json = self
            .services
            .plugin_installations
            .get_registry_cache()
            .await?;

        let registry_entries: Vec<RegistryEntry> = match registry_json {
            Some(json) => serde_json::from_str::<RegistryManifest>(&json)
                .map(|m| m.plugins)
                .unwrap_or_default(),
            None => vec![],
        };

        // Build merged list
        let mut result = Vec::new();

        // Start with registry entries, annotated with installation state
        for entry in &registry_entries {
            let inst = installations.iter().find(|i| i.plugin_id == entry.id);
            let is_compatible = registry_entry_is_host_compatible(entry);
            if inst.is_none() && !is_compatible {
                continue;
            }
            let plugin_type =
                merged_plugin_type(&entry.plugin_type, inst.map(|i| i.plugin_type.as_str()));
            result.push(RegistryPlugin {
                id: entry.id.clone(),
                name: entry.name.clone(),
                description: entry.description.clone(),
                version: entry.version.clone(),
                plugin_type,
                provider_type: entry.provider_type.clone(),
                author: entry.author.clone(),
                official: entry.official,
                builtin: entry.builtin,
                source_url: entry.source_url.clone(),
                wasm_url: entry.wasm_url.clone(),
                wasm_sha256: entry.wasm_sha256.clone(),
                min_scryer_version: entry.min_scryer_version.clone(),
                default_base_url: match entry.plugin_type.as_str() {
                    "download_client" => self
                        .services
                        .download_client_plugin_provider
                        .as_ref()
                        .and_then(|p| p.default_base_url_for_provider(&entry.provider_type)),
                    _ => self
                        .services
                        .plugin_provider
                        .as_ref()
                        .and_then(|p| p.default_base_url_for_provider(&entry.provider_type)),
                },
                is_installed: inst.is_some(),
                is_enabled: inst.map(|i| i.is_enabled).unwrap_or(false),
                installed_version: inst.map(|i| i.version.clone()),
                update_available: inst
                    .map(|i| {
                        if !is_compatible {
                            return false;
                        }
                        semver::Version::parse(&entry.version)
                            .ok()
                            .zip(semver::Version::parse(&i.version).ok())
                            .map(|(reg, inst)| reg > inst)
                            .unwrap_or(false)
                    })
                    .unwrap_or(false),
            });
        }

        // Add any installed plugins not in the registry (e.g. manually installed)
        for inst in &installations {
            if !result.iter().any(|r| r.id == inst.plugin_id) {
                result.push(RegistryPlugin {
                    id: inst.plugin_id.clone(),
                    name: inst.name.clone(),
                    description: inst.description.clone(),
                    version: inst.version.clone(),
                    plugin_type: inst.plugin_type.clone(),
                    provider_type: inst.provider_type.clone(),
                    author: String::new(),
                    official: false,
                    builtin: inst.is_builtin,
                    source_url: inst.source_url.clone(),
                    wasm_url: None,
                    wasm_sha256: inst.wasm_sha256.clone(),
                    min_scryer_version: None,
                    default_base_url: match inst.plugin_type.as_str() {
                        "download_client" => self
                            .services
                            .download_client_plugin_provider
                            .as_ref()
                            .and_then(|p| p.default_base_url_for_provider(&inst.provider_type)),
                        _ => self
                            .services
                            .plugin_provider
                            .as_ref()
                            .and_then(|p| p.default_base_url_for_provider(&inst.provider_type)),
                    },
                    is_installed: true,
                    is_enabled: inst.is_enabled,
                    installed_version: Some(inst.version.clone()),
                    update_available: false,
                });
            }
        }

        Ok(result)
    }

    /// Refresh the plugin registry from the remote URL.
    pub async fn refresh_plugin_registry(&self, actor: &User) -> AppResult<Vec<RegistryPlugin>> {
        require(actor, &Entitlement::ManageConfig)?;
        self.refresh_plugin_registry_internal().await?;
        self.list_available_plugins(actor).await
    }

    /// Internal registry refresh (no auth check) for use by startup and background tasks.
    pub async fn refresh_plugin_registry_internal(&self) -> AppResult<()> {
        let body = reqwest::get(DEFAULT_REGISTRY_URL)
            .await
            .map_err(|e| AppError::Repository(format!("failed to fetch plugin registry: {e}")))?
            .text()
            .await
            .map_err(|e| {
                AppError::Repository(format!("failed to read plugin registry body: {e}"))
            })?;

        let _manifest: RegistryManifest = serde_json::from_str(&body)
            .map_err(|e| AppError::Validation(format!("invalid plugin registry JSON: {e}")))?;

        self.services
            .plugin_installations
            .store_registry_cache(&body)
            .await?;

        Ok(())
    }

    /// Install a plugin from the registry.
    pub async fn install_plugin(
        &self,
        actor: &User,
        plugin_id: &str,
    ) -> AppResult<PluginInstallation> {
        require(actor, &Entitlement::ManageConfig)?;

        // Look up plugin in cached registry
        let registry_json = self
            .services
            .plugin_installations
            .get_registry_cache()
            .await?
            .ok_or_else(|| {
                AppError::Validation("plugin registry not loaded; refresh first".to_string())
            })?;

        let manifest: RegistryManifest = serde_json::from_str(&registry_json)
            .map_err(|e| AppError::Repository(format!("invalid cached registry: {e}")))?;

        let entry = manifest
            .plugins
            .iter()
            .find(|p| p.id == plugin_id)
            .ok_or_else(|| AppError::NotFound(format!("plugin '{plugin_id}' not in registry")))?;

        ensure_registry_entry_is_host_compatible(entry)?;

        // Can't install built-in plugins (they're already installed)
        if entry.builtin {
            return Err(AppError::Validation(
                "built-in plugins are always installed".to_string(),
            ));
        }

        let wasm_url = entry.wasm_url.as_ref().ok_or_else(|| {
            AppError::Validation(format!("plugin '{plugin_id}' has no wasm_url in registry"))
        })?;

        // Download WASM
        let wasm_bytes = reqwest::get(wasm_url)
            .await
            .map_err(|e| AppError::Repository(format!("failed to download plugin WASM: {e}")))?
            .bytes()
            .await
            .map_err(|e| AppError::Repository(format!("failed to read plugin WASM: {e}")))?;

        // Verify SHA256 if provided
        if let Some(ref expected_sha) = entry.wasm_sha256 {
            let actual_sha =
                crate::to_hex(ring_digest::digest(&ring_digest::SHA256, &wasm_bytes).as_ref());
            if actual_sha != *expected_sha {
                return Err(AppError::Validation(format!(
                    "WASM SHA256 mismatch: expected {expected_sha}, got {actual_sha}"
                )));
            }
        }

        let now = Utc::now();
        let installation = PluginInstallation {
            id: Id::new().0,
            plugin_id: plugin_id.to_string(),
            name: entry.name.clone(),
            description: entry.description.clone(),
            version: entry.version.clone(),
            plugin_type: entry.plugin_type.clone(),
            provider_type: entry.provider_type.clone(),
            is_enabled: true,
            is_builtin: false,
            wasm_sha256: entry.wasm_sha256.clone(),
            source_url: entry.source_url.clone(),
            installed_at: now,
            updated_at: now,
        };

        let result = self
            .services
            .plugin_installations
            .create_plugin_installation(&installation, Some(&wasm_bytes))
            .await?;

        self.rebuild_plugin_provider().await?;

        // Auto-create an IndexerConfig for single-endpoint indexer plugins.
        // Read default_base_url from the loaded plugin descriptor (not the
        // registry cache) — the WASM itself is the source of truth.
        if is_indexer_plugin_type(&entry.plugin_type) {
            let default_url = self
                .services
                .plugin_provider
                .as_ref()
                .and_then(|p| p.default_base_url_for_provider(&entry.provider_type));
            if let Some(ref default_url) = default_url {
                let existing = self
                    .services
                    .indexer_configs
                    .list(Some(entry.provider_type.clone()))
                    .await
                    .unwrap_or_default();
                if existing.is_empty() {
                    let plugin_rate_limit = self
                        .services
                        .plugin_provider
                        .as_ref()
                        .and_then(|p| p.rate_limit_seconds_for_provider(&entry.provider_type));
                    let config = IndexerConfig {
                        id: Id::new().0,
                        name: entry.name.clone(),
                        provider_type: entry.provider_type.clone(),
                        base_url: default_url.clone(),
                        api_key_encrypted: None,
                        is_enabled: true,
                        enable_interactive_search: true,
                        enable_auto_search: true,
                        rate_limit_seconds: plugin_rate_limit,
                        rate_limit_burst: None,
                        disabled_until: None,
                        last_health_status: None,
                        last_error_at: None,
                        config_json: None,
                        created_at: now,
                        updated_at: now,
                    };
                    if let Err(e) = self.services.indexer_configs.create(config).await {
                        tracing::warn!(error = %e, "failed to auto-create indexer config for plugin");
                    }
                }
            }
        }

        Ok(result)
    }

    /// Uninstall a non-builtin plugin.
    pub async fn uninstall_plugin(&self, actor: &User, plugin_id: &str) -> AppResult<()> {
        require(actor, &Entitlement::ManageConfig)?;

        let installation = self
            .services
            .plugin_installations
            .get_plugin_installation(plugin_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("plugin '{plugin_id}' not installed")))?;

        if installation.is_builtin {
            return Err(AppError::Validation(
                "cannot uninstall built-in plugins; disable them instead".to_string(),
            ));
        }

        // Delete all associated IndexerConfigs for this plugin's provider type.
        if is_indexer_plugin_type(&installation.plugin_type) {
            let configs = self
                .services
                .indexer_configs
                .list(Some(installation.provider_type.clone()))
                .await
                .unwrap_or_default();
            for config in configs {
                if let Err(e) = self.services.indexer_configs.delete(&config.id).await {
                    tracing::warn!(error = %e, indexer = config.name, "failed to delete indexer config during plugin uninstall");
                }
            }
        }

        self.services
            .plugin_installations
            .delete_plugin_installation(plugin_id)
            .await?;

        self.rebuild_plugin_provider().await?;
        Ok(())
    }

    /// Toggle a plugin's enabled/disabled state.
    pub async fn toggle_plugin(
        &self,
        actor: &User,
        plugin_id: &str,
        enabled: bool,
    ) -> AppResult<PluginInstallation> {
        require(actor, &Entitlement::ManageConfig)?;

        let mut installation = self
            .services
            .plugin_installations
            .get_plugin_installation(plugin_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("plugin '{plugin_id}' not installed")))?;

        installation.is_enabled = enabled;
        installation.updated_at = Utc::now();

        let result = self
            .services
            .plugin_installations
            .update_plugin_installation(&installation, None)
            .await?;

        self.rebuild_plugin_provider().await?;
        Ok(result)
    }

    /// Upgrade a non-builtin plugin to the latest registry version.
    pub async fn upgrade_plugin(
        &self,
        actor: &User,
        plugin_id: &str,
    ) -> AppResult<PluginInstallation> {
        require(actor, &Entitlement::ManageConfig)?;

        let installation = self
            .services
            .plugin_installations
            .get_plugin_installation(plugin_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("plugin '{plugin_id}' not installed")))?;

        if installation.is_builtin {
            return Err(AppError::Validation(
                "cannot upgrade built-in plugins".to_string(),
            ));
        }

        // Look up in cached registry
        let registry_json = self
            .services
            .plugin_installations
            .get_registry_cache()
            .await?
            .ok_or_else(|| {
                AppError::Validation("plugin registry not loaded; refresh first".to_string())
            })?;

        let manifest: RegistryManifest = serde_json::from_str(&registry_json)
            .map_err(|e| AppError::Repository(format!("invalid cached registry: {e}")))?;

        let entry = manifest
            .plugins
            .iter()
            .find(|p| p.id == plugin_id)
            .ok_or_else(|| AppError::NotFound(format!("plugin '{plugin_id}' not in registry")))?;

        ensure_registry_entry_is_host_compatible(entry)?;

        // Verify a newer version exists
        let reg_ver = semver::Version::parse(&entry.version).map_err(|e| {
            AppError::Validation(format!("invalid registry version '{}': {e}", entry.version))
        })?;
        let inst_ver = semver::Version::parse(&installation.version).map_err(|e| {
            AppError::Validation(format!(
                "invalid installed version '{}': {e}",
                installation.version
            ))
        })?;
        if reg_ver <= inst_ver {
            return Err(AppError::Validation(format!(
                "plugin '{plugin_id}' is already at version {} (registry has {})",
                installation.version, entry.version
            )));
        }

        let wasm_url = entry.wasm_url.as_ref().ok_or_else(|| {
            AppError::Validation(format!("plugin '{plugin_id}' has no wasm_url in registry"))
        })?;

        // Download WASM
        let wasm_bytes = reqwest::get(wasm_url)
            .await
            .map_err(|e| AppError::Repository(format!("failed to download plugin WASM: {e}")))?
            .bytes()
            .await
            .map_err(|e| AppError::Repository(format!("failed to read plugin WASM: {e}")))?;

        // Verify SHA256 if provided
        if let Some(ref expected_sha) = entry.wasm_sha256 {
            let actual_sha =
                crate::to_hex(ring_digest::digest(&ring_digest::SHA256, &wasm_bytes).as_ref());
            if actual_sha != *expected_sha {
                return Err(AppError::Validation(format!(
                    "WASM SHA256 mismatch: expected {expected_sha}, got {actual_sha}"
                )));
            }
        }

        let mut updated = installation;
        updated.version = entry.version.clone();
        updated.name = entry.name.clone();
        updated.description = entry.description.clone();
        updated.wasm_sha256 = entry.wasm_sha256.clone();
        updated.updated_at = Utc::now();

        let result = self
            .services
            .plugin_installations
            .update_plugin_installation(&updated, Some(&wasm_bytes))
            .await?;

        self.rebuild_plugin_provider().await?;
        Ok(result)
    }
}

// NZBGeek has a fixed endpoint, but it still needs a user-supplied API key.
fn should_skip_auto_created_indexer_config(provider_type: &str) -> bool {
    provider_type.eq_ignore_ascii_case("nzbgeek")
}

#[cfg(test)]
#[path = "app_usecase_plugins_tests.rs"]
mod app_usecase_plugins_tests;

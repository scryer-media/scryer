use super::*;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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
        let installations = self
            .services
            .plugin_installations
            .get_enabled_plugin_wasm_bytes()
            .await?;

        // Collect WASM bytes for user-installed (non-builtin) enabled plugins
        let external_bytes: Vec<Vec<u8>> = installations
            .iter()
            .filter(|(inst, _)| !inst.is_builtin)
            .filter_map(|(_, wasm)| wasm.clone())
            .collect();
        let external_refs: Vec<&[u8]> = external_bytes.iter().map(|b| b.as_slice()).collect();

        // Collect provider_types of builtins the user has disabled
        let disabled_builtins: Vec<String> = installations
            .iter()
            .filter(|(inst, _)| inst.is_builtin && !inst.is_enabled)
            .map(|(inst, _)| inst.provider_type.clone())
            .collect();

        if let Some(ref provider) = self.services.plugin_provider {
            if let Err(e) = provider.reload_plugins(&external_refs, &disabled_builtins) {
                tracing::warn!(error = %e, "failed to reload plugin provider");
            }
        }

        // Rebuild rules engine to pick up new/removed scoring policies
        self.rebuild_user_rules_engine().await?;
        Ok(())
    }

    /// Returns all available indexer provider types with their config field schemas.
    pub fn available_indexer_provider_types(
        &self,
    ) -> Vec<(String, String, Vec<scryer_domain::ConfigFieldDef>)> {
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
                (pt, name, fields)
            })
            .collect()
    }

    /// List available plugins by merging cached registry with local installations.
    pub async fn list_available_plugins(
        &self,
        actor: &User,
    ) -> AppResult<Vec<RegistryPlugin>> {
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
            result.push(RegistryPlugin {
                id: entry.id.clone(),
                name: entry.name.clone(),
                description: entry.description.clone(),
                version: entry.version.clone(),
                plugin_type: entry.plugin_type.clone(),
                provider_type: entry.provider_type.clone(),
                author: entry.author.clone(),
                official: entry.official,
                builtin: entry.builtin,
                source_url: entry.source_url.clone(),
                wasm_url: entry.wasm_url.clone(),
                wasm_sha256: entry.wasm_sha256.clone(),
                min_scryer_version: entry.min_scryer_version.clone(),
                is_installed: inst.is_some(),
                is_enabled: inst.map(|i| i.is_enabled).unwrap_or(false),
                installed_version: inst.map(|i| i.version.clone()),
                update_available: inst
                    .map(|i| {
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
    pub async fn refresh_plugin_registry(
        &self,
        actor: &User,
    ) -> AppResult<Vec<RegistryPlugin>> {
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

        let _manifest: RegistryManifest = serde_json::from_str(&body).map_err(|e| {
            AppError::Validation(format!("invalid plugin registry JSON: {e}"))
        })?;

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
                AppError::Validation(
                    "plugin registry not loaded; refresh first".to_string(),
                )
            })?;

        let manifest: RegistryManifest = serde_json::from_str(&registry_json)
            .map_err(|e| AppError::Repository(format!("invalid cached registry: {e}")))?;

        let entry = manifest
            .plugins
            .iter()
            .find(|p| p.id == plugin_id)
            .ok_or_else(|| {
                AppError::NotFound(format!("plugin '{plugin_id}' not in registry"))
            })?;

        // Can't install built-in plugins (they're already installed)
        if entry.builtin {
            return Err(AppError::Validation(
                "built-in plugins are always installed".to_string(),
            ));
        }

        let wasm_url = entry.wasm_url.as_ref().ok_or_else(|| {
            AppError::Validation(format!(
                "plugin '{plugin_id}' has no wasm_url in registry"
            ))
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
            let actual_sha = format!("{:x}", Sha256::digest(&wasm_bytes));
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
        Ok(result)
    }

    /// Uninstall a non-builtin plugin.
    pub async fn uninstall_plugin(
        &self,
        actor: &User,
        plugin_id: &str,
    ) -> AppResult<()> {
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
                AppError::Validation(
                    "plugin registry not loaded; refresh first".to_string(),
                )
            })?;

        let manifest: RegistryManifest = serde_json::from_str(&registry_json)
            .map_err(|e| AppError::Repository(format!("invalid cached registry: {e}")))?;

        let entry = manifest
            .plugins
            .iter()
            .find(|p| p.id == plugin_id)
            .ok_or_else(|| {
                AppError::NotFound(format!("plugin '{plugin_id}' not in registry"))
            })?;

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
            AppError::Validation(format!(
                "plugin '{plugin_id}' has no wasm_url in registry"
            ))
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
            let actual_sha = format!("{:x}", Sha256::digest(&wasm_bytes));
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

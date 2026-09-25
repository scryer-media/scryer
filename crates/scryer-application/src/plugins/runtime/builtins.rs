/// Registry plugin entry merged with local installation state.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegistryPlugin {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub latest_version: Option<String>,
    pub plugin_type: String,
    pub provider_type: String,
    pub author: String,
    pub official: bool,
    pub publisher: Option<String>,
    pub support_tier: PluginSupportTier,
    pub status: Option<String>,
    pub docs_url: Option<String>,
    pub source_repo: Option<String>,
    pub builtin: bool,
    pub source_url: Option<String>,
    pub source_kind: Option<String>,
    pub blocked_reason: Option<String>,
    pub wasm_url: Option<String>,
    pub wasm_sha256: Option<String>,
    pub min_scryer_version: Option<String>,
    pub bytes: Option<u64>,
    /// Merged from local installation state.
    pub is_installed: bool,
    pub is_enabled: bool,
    pub installed_version: Option<String>,
    /// True when the catalog has a newer version or a compatible optimized artifact.
    pub update_available: bool,
    pub install_in_progress: bool,
    /// When set, installing this plugin auto-creates an IndexerConfig with this URL.
    pub default_base_url: Option<String>,
}
impl ValidatedCatalogPluginInstall {
    fn into_new_installation(
        self,
        plugin_id: String,
    ) -> AppResult<(PluginInstallation, RuntimePluginLoad)> {
        let now = Utc::now();
        let runtime_plugin = runtime_plugin_load_from_validated(
            self.descriptor.clone(),
            self.runtime_wasm_bytes,
            self.runtime_first_party,
        );
        Ok((
            PluginInstallation {
                id: Id::new().0,
                plugin_id,
                name: self.descriptor.name.clone(),
                description: self.description,
                version: self.descriptor.version.clone(),
                sdk_version: self.descriptor.sdk_version.clone(),
                sdk_constraint: self.sdk_constraint,
                scryer_constraint: self.scryer_constraint,
                plugin_type: self.descriptor.plugin_type().to_string(),
                provider_type: normalize_provider_key(self.descriptor.provider_type()),
                source_kind: self.source_kind,
                is_enabled: true,
                is_builtin: false,
                wasm_encoding: self.wasm_encoding,
                wasm_digest_algo: Some(self.wasm_digest_algo),
                source_url: Some(self.source_url.clone()),
                support_tier: self.support_tier,
                publisher: Some(self.publisher),
                docs_url: Some(self.docs_url),
                source_repo: Some(self.source_repo),
                manifest_url: Some(self.manifest_url),
                wasm_digest: Some(self.wasm_digest),
                artifact_digest: Some(self.artifact_digest),
                descriptor_json: Some(persisted_plugin_descriptor_json(&self.descriptor)?),
                installed_at: now,
                updated_at: now,
            },
            runtime_plugin,
        ))
    }

    fn into_updated_installation(
        self,
        mut installation: PluginInstallation,
    ) -> AppResult<(PluginInstallation, RuntimePluginLoad)> {
        let runtime_plugin = runtime_plugin_load_from_validated(
            self.descriptor.clone(),
            self.runtime_wasm_bytes,
            self.runtime_first_party,
        );
        installation.name = self.descriptor.name.clone();
        installation.description = self.description;
        installation.version = self.descriptor.version.clone();
        installation.sdk_version = self.descriptor.sdk_version.clone();
        installation.sdk_constraint = self.sdk_constraint;
        installation.scryer_constraint = self.scryer_constraint;
        installation.plugin_type = self.descriptor.plugin_type().to_string();
        installation.provider_type = normalize_provider_key(self.descriptor.provider_type());
        installation.source_kind = self.source_kind;
        installation.is_builtin = false;
        installation.wasm_encoding = self.wasm_encoding;
        installation.wasm_digest_algo = Some(self.wasm_digest_algo);
        installation.source_url = Some(self.source_url.clone());
        installation.support_tier = self.support_tier;
        installation.publisher = Some(self.publisher);
        installation.docs_url = Some(self.docs_url);
        installation.source_repo = Some(self.source_repo);
        installation.manifest_url = Some(self.manifest_url);
        installation.wasm_digest = Some(self.wasm_digest);
        installation.artifact_digest = Some(self.artifact_digest);
        installation.descriptor_json = Some(persisted_plugin_descriptor_json(&self.descriptor)?);
        installation.updated_at = Utc::now();
        Ok((installation, runtime_plugin))
    }
}
fn preserves_legacy_nzbgeek_builtin_for_catalog_migration(
    installation: &PluginInstallation,
) -> bool {
    installation.plugin_id == LEGACY_NZBGEEK_PLUGIN_ID
        && installation.is_builtin
        && installation.source_kind == PluginSourceKind::Bundled
}
const LEGACY_NZBGEEK_PLUGIN_ID: &str = "nzbgeek";
struct BuiltinPluginSeed {
    name: String,
    version: String,
    sdk_version: String,
    sdk_constraint: String,
    plugin_type: String,
    provider_type: String,
}
fn builtin_lookup_key(plugin_type: &str, provider_type: &str) -> String {
    let family = if is_indexer_plugin_type(plugin_type) {
        "indexer"
    } else {
        plugin_type
    };
    format!("{family}::{}", normalize_provider_key(provider_type))
}
impl AppUseCase {
    fn builtin_plugin_inventory(&self) -> Vec<BuiltinPluginSeed> {
        let mut builtins = Vec::new();

        if let Some(provider) = self.services.integrations.plugin_provider.available() {
            for provider_type in provider.builtin_provider_types() {
                let provider_key = normalize_provider_key(&provider_type);
                let Some(name) = provider.plugin_name_for_provider(&provider_key) else {
                    continue;
                };
                let Some(version) = provider.plugin_version_for_provider(&provider_key) else {
                    continue;
                };
                let Some(sdk_version) = provider.plugin_sdk_version_for_provider(&provider_key)
                else {
                    continue;
                };
                let Some(sdk_constraint) =
                    provider.plugin_sdk_constraint_for_provider(&provider_key)
                else {
                    continue;
                };
                let plugin_type = provider
                    .plugin_type_for_provider(&provider_key)
                    .unwrap_or_else(|| LEGACY_INDEXER_PLUGIN_TYPE.to_string());
                builtins.push(BuiltinPluginSeed {
                    name,
                    version,
                    sdk_version,
                    sdk_constraint,
                    plugin_type,
                    provider_type: provider_key,
                });
            }
        }

        if let Some(provider) = self
            .services
            .integrations
            .subtitle_plugin_provider
            .available()
        {
            for provider_type in provider.builtin_provider_types() {
                let provider_key = normalize_provider_key(&provider_type);
                let Some(name) = provider.plugin_name_for_provider(&provider_key) else {
                    continue;
                };
                let Some(version) = provider.plugin_version_for_provider(&provider_key) else {
                    continue;
                };
                let Some(sdk_version) = provider.plugin_sdk_version_for_provider(&provider_key)
                else {
                    continue;
                };
                let Some(sdk_constraint) =
                    provider.plugin_sdk_constraint_for_provider(&provider_key)
                else {
                    continue;
                };
                builtins.push(BuiltinPluginSeed {
                    name,
                    version,
                    sdk_version,
                    sdk_constraint,
                    plugin_type: "subtitle_provider".to_string(),
                    provider_type: provider_key,
                });
            }
        }

        if let Some(provider) = self
            .services
            .integrations
            .download_client_plugin_provider
            .available()
        {
            for provider_type in provider.builtin_provider_types() {
                let provider_key = normalize_provider_key(&provider_type);
                let Some(name) = provider.plugin_name_for_provider(&provider_key) else {
                    continue;
                };
                let Some(version) = provider.plugin_version_for_provider(&provider_key) else {
                    continue;
                };
                let Some(sdk_version) = provider.plugin_sdk_version_for_provider(&provider_key)
                else {
                    continue;
                };
                let Some(sdk_constraint) =
                    provider.plugin_sdk_constraint_for_provider(&provider_key)
                else {
                    continue;
                };
                builtins.push(BuiltinPluginSeed {
                    name,
                    version,
                    sdk_version,
                    sdk_constraint,
                    plugin_type: "download_client".to_string(),
                    provider_type: provider_key,
                });
            }
        }

        if let Some(provider) = self.services.notifications.notification_provider() {
            for provider_type in provider.builtin_provider_types() {
                let provider_key = normalize_provider_key(&provider_type);
                let Some(name) = provider.plugin_name_for_provider(&provider_key) else {
                    continue;
                };
                let Some(version) = provider.plugin_version_for_provider(&provider_key) else {
                    continue;
                };
                let Some(sdk_version) = provider.plugin_sdk_version_for_provider(&provider_key)
                else {
                    continue;
                };
                let Some(sdk_constraint) =
                    provider.plugin_sdk_constraint_for_provider(&provider_key)
                else {
                    continue;
                };
                builtins.push(BuiltinPluginSeed {
                    name,
                    version,
                    sdk_version,
                    sdk_constraint,
                    plugin_type: "notification".to_string(),
                    provider_type: provider_key,
                });
            }
        }

        builtins
    }
}
impl AppUseCase {
    fn builtin_seed_by_key(&self) -> std::collections::HashMap<String, BuiltinPluginSeed> {
        self.builtin_plugin_inventory()
            .into_iter()
            .map(|seed| {
                (
                    builtin_lookup_key(&seed.plugin_type, &seed.provider_type),
                    seed,
                )
            })
            .collect()
    }
}
impl AppUseCase {
    async fn prepare_runtime_builtin(&self, installation: &PluginInstallation) -> AppResult<()> {
        let provider_type = installation.provider_type.clone();
        if is_indexer_plugin_type(&installation.plugin_type) {
            let provider = self
                .services
                .integrations
                .plugin_provider
                .available()
                .cloned()
                .ok_or_else(|| {
                    AppError::Repository("indexer plugin provider unavailable".to_string())
                })?;
            return tokio::task::spawn_blocking(move || {
                provider.prepare_builtin_plugin(&provider_type)
            })
            .await
            .map_err(|error| {
                AppError::Repository(format!(
                    "built-in indexer plugin preparation task failed: {error}"
                ))
            })?
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to prepare built-in indexer plugin: {error}"
                ))
            });
        }

        match installation.plugin_type.as_str() {
            "subtitle_provider" => {
                let provider = self
                    .services
                    .integrations
                    .subtitle_plugin_provider
                    .available()
                    .cloned()
                    .ok_or_else(|| {
                        AppError::Repository("subtitle plugin provider unavailable".to_string())
                    })?;
                tokio::task::spawn_blocking(move || provider.prepare_builtin_plugin(&provider_type))
                    .await
                    .map_err(|error| {
                        AppError::Repository(format!(
                            "built-in subtitle plugin preparation task failed: {error}"
                        ))
                    })?
                    .map_err(|error| {
                        AppError::Repository(format!(
                            "failed to prepare built-in subtitle plugin: {error}"
                        ))
                    })
            }
            other => Err(AppError::Validation(format!(
                "unsupported plugin_type '{}' for built-in preparation",
                other
            ))),
        }
    }

    fn apply_runtime_builtin_restore(&self, installation: &PluginInstallation) -> AppResult<()> {
        let provider_type = installation.provider_type.as_str();
        if is_indexer_plugin_type(&installation.plugin_type) {
            let provider = self
                .services
                .integrations
                .plugin_provider
                .available()
                .ok_or_else(|| {
                    AppError::Repository("indexer plugin provider unavailable".to_string())
                })?;
            provider
                .restore_builtin_plugin(provider_type)
                .map_err(|e| {
                    AppError::Repository(format!("failed to restore built-in indexer plugin: {e}"))
                })?;
            return Ok(());
        }

        match installation.plugin_type.as_str() {
            "subtitle_provider" => {
                let provider = self
                    .services
                    .integrations
                    .subtitle_plugin_provider
                    .available()
                    .ok_or_else(|| {
                        AppError::Repository("subtitle plugin provider unavailable".to_string())
                    })?;
                provider
                    .restore_builtin_plugin(provider_type)
                    .map_err(|e| {
                        AppError::Repository(format!(
                            "failed to restore built-in subtitle plugin: {e}"
                        ))
                    })?;
            }
            other => {
                return Err(AppError::Validation(format!(
                    "unsupported plugin_type '{}' for built-in restore",
                    other
                )));
            }
        }

        Ok(())
    }
}
impl AppUseCase {
    /// Seed database rows for built-in plugins. Uses INSERT OR IGNORE so
    /// existing user toggles are preserved across restarts.
    pub async fn seed_builtin_plugins(&self) -> AppResult<()> {
        let repo = &self.services.customization.plugin_installations;
        let builtins = self.builtin_plugin_inventory();

        let builtin_keys = builtins
            .iter()
            .map(|builtin| builtin_lookup_key(&builtin.plugin_type, &builtin.provider_type))
            .collect::<std::collections::HashSet<_>>();

        for builtin in builtins {
            repo.seed_builtin(
                &builtin.provider_type,
                &builtin.name,
                "",
                &builtin.version,
                &builtin.sdk_version,
                &builtin.sdk_constraint,
                &builtin.plugin_type,
                &builtin.provider_type,
            )
            .await?;
        }

        let stale_builtin_plugin_ids = repo
            .list_plugin_installations()
            .await?
            .into_iter()
            .filter(|installation| {
                installation.is_builtin
                    && !preserves_legacy_nzbgeek_builtin_for_catalog_migration(installation)
                    && !builtin_keys.contains(&builtin_lookup_key(
                        &installation.plugin_type,
                        &installation.provider_type,
                    ))
            })
            .map(|installation| installation.plugin_id)
            .collect::<Vec<_>>();

        for plugin_id in stale_builtin_plugin_ids {
            repo.delete_plugin_installation(&plugin_id).await?;
        }

        Ok(())
    }
}

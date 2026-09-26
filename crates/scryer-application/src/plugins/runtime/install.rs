fn installation_is_host_blocked(installation: &PluginInstallation) -> bool {
    normalized_constraint(installation.scryer_constraint.as_deref()).is_some_and(|constraint| {
        host_version_matches_constraint(CURRENT_SCRYER_VERSION, &constraint)
            .map(|matches| !matches)
            .unwrap_or(true)
    })
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
impl AppUseCase {
    async fn load_runtime_plugin_for_installation(
        &self,
        installation: &PluginInstallation,
    ) -> AppResult<RuntimePluginLoad> {
        let payload = self
            .services
            .customization
            .plugin_installations
            .get_plugin_installation_wasm_payload(&installation.plugin_id)
            .await?
            .ok_or_else(|| {
                AppError::Validation(format!(
                    "plugin '{}' is missing persisted WASM payload",
                    installation.plugin_id
                ))
            })?;
        load_runtime_plugin_from_persisted_installation_payload(installation, &payload).await
    }
}
impl AppUseCase {
    fn apply_runtime_plugin_upsert(
        &self,
        installation: &PluginInstallation,
        runtime_plugin: RuntimePluginLoad,
    ) -> AppResult<()> {
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
                .upsert_runtime_plugin(runtime_plugin)
                .map_err(|e| {
                    AppError::Repository(format!("failed to upsert indexer plugin: {e}"))
                })?;
            return Ok(());
        }

        match installation.plugin_type.as_str() {
            "download_client" => {
                let provider = self
                    .services
                    .integrations
                    .download_client_plugin_provider
                    .available()
                    .ok_or_else(|| {
                        AppError::Repository(
                            "download client plugin provider unavailable".to_string(),
                        )
                    })?;
                provider
                    .upsert_runtime_plugin(runtime_plugin)
                    .map_err(|e| {
                        AppError::Repository(format!(
                            "failed to upsert download client plugin: {e}"
                        ))
                    })?;
            }
            "notification" => {
                let provider = self
                    .services
                    .notifications
                    .notification_provider()
                    .ok_or_else(|| {
                        AppError::Repository("notification plugin provider unavailable".to_string())
                    })?;
                provider
                    .upsert_runtime_plugin(runtime_plugin)
                    .map_err(|e| {
                        AppError::Repository(format!("failed to upsert notification plugin: {e}"))
                    })?;
            }
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
                    .upsert_runtime_plugin(runtime_plugin)
                    .map_err(|e| {
                        AppError::Repository(format!("failed to upsert subtitle plugin: {e}"))
                    })?;
            }
            "archive_extractor" => {
                let provider = self
                    .services
                    .integrations
                    .archive_extractor_plugin_provider
                    .available()
                    .ok_or_else(|| {
                        AppError::Repository(
                            "archive extractor plugin provider unavailable".to_string(),
                        )
                    })?;
                provider
                    .upsert_runtime_plugin(runtime_plugin)
                    .map_err(|e| {
                        AppError::Repository(format!(
                            "failed to upsert archive extractor plugin: {e}"
                        ))
                    })?;
            }
            "list_provider" => {
                self.services
                    .lists
                    .plugins
                    .upsert_runtime_plugin(runtime_plugin)
                    .map_err(|e| {
                        AppError::Repository(format!("failed to upsert list provider plugin: {e}"))
                    })?;
            }
            other => {
                return Err(AppError::Validation(format!(
                    "unsupported plugin_type '{}' for runtime upsert",
                    other
                )));
            }
        }

        Ok(())
    }
}
impl AppUseCase {
    fn apply_runtime_plugin_replace(
        &self,
        previous_installation: &PluginInstallation,
        next_installation: &PluginInstallation,
        runtime_plugin: RuntimePluginLoad,
    ) -> AppResult<()> {
        if previous_installation.plugin_type != next_installation.plugin_type
            || previous_installation.provider_type != next_installation.provider_type
        {
            self.apply_runtime_plugin_removal_for_values(
                previous_installation.plugin_type.as_str(),
                previous_installation.provider_type.as_str(),
            )?;
        }

        self.apply_runtime_plugin_upsert(next_installation, runtime_plugin)
    }
}
impl AppUseCase {
    fn apply_runtime_plugin_removal(&self, installation: &PluginInstallation) -> AppResult<()> {
        self.apply_runtime_plugin_removal_for_values(
            installation.plugin_type.as_str(),
            installation.provider_type.as_str(),
        )
    }
}
impl AppUseCase {
    fn plugin_install_in_progress_error(plugin_id: &str) -> AppError {
        AppError::PluginInstallInProgress(plugin_id.trim().to_ascii_lowercase())
    }
}
impl AppUseCase {
    /// Uninstall a non-builtin plugin or revert a downloaded builtin override.
    pub async fn uninstall_plugin(&self, actor: &User, plugin_id: &str) -> AppResult<()> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let installation = self
            .services
            .customization
            .plugin_installations
            .get_plugin_installation(plugin_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("plugin '{plugin_id}' not installed")))?;

        if installation.is_builtin && installation.source_kind == PluginSourceKind::Bundled {
            return Err(AppError::Validation(
                "cannot uninstall built-in plugins; disable them instead".to_string(),
            ));
        }

        if installation.is_builtin && installation.source_kind == PluginSourceKind::Downloaded {
            let mut builtin_by_key = self.builtin_seed_by_key();
            let builtin_seed = builtin_by_key
                .remove(&builtin_lookup_key(
                    &installation.plugin_type,
                    &installation.provider_type,
                ))
                .ok_or_else(|| {
                    AppError::Validation(format!(
                        "cannot revert built-in plugin '{}' because no bundled definition is available",
                        plugin_id
                    ))
                })?;
            let mut reverted = installation.clone();
            reverted.name = builtin_seed.name;
            reverted.version = builtin_seed.version;
            reverted.sdk_version = builtin_seed.sdk_version;
            reverted.sdk_constraint = builtin_seed.sdk_constraint;
            reverted.scryer_constraint = None;
            reverted.plugin_type = builtin_seed.plugin_type;
            reverted.provider_type = builtin_seed.provider_type;
            reverted.source_kind = PluginSourceKind::Bundled;
            reverted.wasm_encoding = PluginWasmEncoding::Identity;
            reverted.wasm_digest_algo = None;
            reverted.source_url = None;
            reverted.manifest_url = None;
            reverted.wasm_digest = None;
            reverted.artifact_digest = None;
            reverted.updated_at = Utc::now();

            self.services
                .customization
                .plugin_installations
                .update_plugin_installation(&reverted, None)
                .await?;

            let runtime_touched = reverted.is_enabled;
            if runtime_touched {
                self.apply_runtime_builtin_restore(&reverted)?;
            } else {
                self.apply_runtime_plugin_removal(&reverted)?;
            }
            self.finalize_runtime_plugin_mutation(&reverted.plugin_type, runtime_touched)
                .await?;
            return Ok(());
        }

        // Delete all associated IndexerConfigs for this plugin's provider type.
        if is_indexer_plugin_type(&installation.plugin_type) {
            let configs = self
                .services
                .integrations
                .indexer_configs
                .list(Some(installation.provider_type.clone()))
                .await?;
            for config in configs {
                if self
                    .services
                    .integrations
                    .indexer_configs
                    .get_by_id(&config.id)
                    .await?
                    .is_some()
                {
                    self.delete_indexer_config_tree(
                        &config.id,
                        true,
                        "plugin_uninstall",
                        Some(actor.id.clone()),
                    )
                    .await?;
                }
            }
        }

        self.services
            .customization
            .plugin_installations
            .delete_plugin_installation(plugin_id)
            .await?;

        self.apply_runtime_plugin_removal(&installation)?;
        self.finalize_runtime_plugin_mutation(&installation.plugin_type, installation.is_enabled)
            .await?;
        Ok(())
    }
}
impl AppUseCase {
    /// Toggle a plugin's enabled/disabled state.
    pub async fn get_plugin_installation(
        &self,
        actor: &User,
        plugin_id: &str,
    ) -> AppResult<Option<PluginInstallation>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        self.services
            .customization
            .plugin_installations
            .get_plugin_installation(plugin_id)
            .await
    }

    pub async fn plugin_declares_provider_alias(
        &self,
        actor: &User,
        plugin_id: &str,
        provider_alias: &str,
    ) -> AppResult<bool> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let installation = self
            .services
            .customization
            .plugin_installations
            .get_plugin_installation(plugin_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("plugin '{plugin_id}' not installed")))?;
        let runtime_plugin = self
            .load_runtime_plugin_for_installation(&installation)
            .await?;
        let descriptor_loader = self.services.customization.plugin_descriptor_loader.clone();
        let wasm_bytes = runtime_plugin.wasm_bytes;
        let descriptor = tokio::task::spawn_blocking(move || {
            descriptor_loader.load_descriptor_from_wasm_bytes(&wasm_bytes)
        })
        .await
        .map_err(|error| {
            AppError::Repository(format!("plugin descriptor loading task failed: {error}"))
        })??;
        let provider_alias = provider_alias.trim().to_ascii_lowercase();
        Ok(descriptor
            .provider_aliases()
            .iter()
            .any(|alias| alias.trim().to_ascii_lowercase() == provider_alias))
    }

    pub async fn toggle_plugin(
        &self,
        actor: &User,
        plugin_id: &str,
        enabled: bool,
    ) -> AppResult<PluginInstallation> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let mut installation = self
            .services
            .customization
            .plugin_installations
            .get_plugin_installation(plugin_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("plugin '{plugin_id}' not installed")))?;

        let prepared_runtime_plugin = if enabled {
            if installation.is_builtin && installation.source_kind == PluginSourceKind::Bundled {
                self.prepare_runtime_builtin(&installation).await?;
                None
            } else {
                let runtime_plugin = self
                    .load_runtime_plugin_for_installation(&installation)
                    .await?;
                let descriptor_loader =
                    self.services.customization.plugin_descriptor_loader.clone();
                let wasm_bytes = runtime_plugin.wasm_bytes.clone();
                tokio::task::spawn_blocking(move || {
                    descriptor_loader.load_descriptor_from_wasm_bytes(&wasm_bytes)
                })
                .await
                .map_err(|error| {
                    AppError::Repository(format!("plugin descriptor loading task failed: {error}"))
                })??;
                Some(runtime_plugin)
            }
        } else {
            None
        };

        installation.is_enabled = enabled;
        installation.updated_at = Utc::now();

        let result = self
            .services
            .customization
            .plugin_installations
            .update_plugin_installation(&installation, None)
            .await?;

        if enabled {
            if result.is_builtin && result.source_kind == PluginSourceKind::Bundled {
                self.apply_runtime_builtin_restore(&result)?;
            } else {
                let runtime_plugin = prepared_runtime_plugin.ok_or_else(|| {
                    AppError::Repository(
                        "enabled external plugin was not prepared before activation".to_string(),
                    )
                })?;
                self.apply_runtime_plugin_upsert(&result, runtime_plugin)?;
            }
        } else {
            self.apply_runtime_plugin_removal(&result)?;
        }
        self.finalize_runtime_plugin_mutation(&installation.plugin_type, true)
            .await?;
        Ok(result)
    }
}

fn same_plugin_identity(installation: &PluginInstallation, descriptor: &PluginDescriptor) -> bool {
    installation.plugin_id == descriptor.id
        && normalize_provider_key(&installation.provider_type)
            == normalize_provider_key(descriptor.provider_type())
        && (installation.plugin_type == descriptor.plugin_type()
            || (is_indexer_plugin_type(&installation.plugin_type)
                && is_indexer_plugin_type(descriptor.plugin_type())))
}

fn component_replacement_has_same_origin(
    installation: &PluginInstallation,
    resolved: &CatalogPluginResolution,
) -> bool {
    installation.plugin_id == resolved.catalog_entry.id
        && normalize_provider_key(&installation.provider_type)
            == normalize_provider_key(&resolved.catalog_entry.provider_type)
        && (installation.plugin_type == resolved.catalog_entry.plugin_type
            || (is_indexer_plugin_type(&installation.plugin_type)
                && is_indexer_plugin_type(&resolved.catalog_entry.plugin_type)))
        && installation.publisher.as_deref() == Some(resolved.catalog_entry.publisher.as_str())
        && installation.source_repo.as_deref() == Some(resolved.catalog_entry.source_repo.as_str())
        && installation.support_tier == resolved.effective_support_tier
        && installation.source_kind == resolved.source_kind
}

impl AppUseCase {
    /// Repair one startup compatibility target, independently of routine
    /// auto-update policy. The caller journals the original row and digest.
    /// Validation completes before the store atomically replaces row + bytes.
    pub async fn repair_plugin_component_installation(
        &self,
        installation: PluginInstallation,
        bundled: Option<RuntimePluginLoad>,
    ) -> AppResult<bool> {
        if installation.source_kind != PluginSourceKind::Bundled
            && !installation_is_host_blocked(&installation)
            && let Ok(current) = self
                .load_runtime_plugin_for_installation(&installation)
                .await
            && self
                .validate_component_upgrade_runtime(&installation, &current)
                .await
                .is_ok()
        {
            return Ok(false);
        }

        if let Some(bundled) = bundled
            .filter(|plugin| same_plugin_identity(&installation, &plugin.descriptor))
            .filter(|_| {
                installation.source_kind == PluginSourceKind::Bundled
                    || installation_is_catalog_official(&installation)
            })
        {
            self.validate_component_upgrade_runtime(&installation, &bundled)
                .await?;
            if installation.source_kind == PluginSourceKind::Bundled {
                return Ok(false);
            }
            let mut updated = installation.clone();
            updated.version = bundled.descriptor.version.clone();
            updated.sdk_version = bundled.descriptor.sdk_version.clone();
            updated.sdk_constraint = plugin_descriptor_sdk_constraint(&bundled.descriptor);
            updated.scryer_constraint = None;
            updated.plugin_type = bundled.descriptor.plugin_type().to_string();
            updated.descriptor_json = Some(persisted_plugin_descriptor_json(&bundled.descriptor)?);
            updated.wasm_encoding = PluginWasmEncoding::Identity;
            updated.wasm_digest_algo = Some("blake3".into());
            let digest = blake3::hash(&bundled.wasm_bytes).to_hex().to_string();
            updated.wasm_digest = Some(digest.clone());
            updated.artifact_digest = Some(format!("blake3:{digest}"));
            updated.updated_at = Utc::now();
            // Keep installation identity, enabled preference and source
            // provenance; the journal also retains the original metadata.
            self.services
                .customization
                .plugin_installations
                .update_plugin_installation(&updated, Some(&bundled.wasm_bytes))
                .await?;
            return Ok(true);
        }

        let resolved = self.resolved_catalog_plugins().await?.into_iter()
            .find(|resolved| component_replacement_has_same_origin(&installation, resolved))
            .ok_or_else(|| AppError::Validation(
                "No compatible component from the installed plugin's original publisher and source; install a compatible build. The original installation and configuration have been retained.".into()
            ))?;
        let reporter = PluginInstallProgressReporter::new(self, "system", &installation.plugin_id);
        let prepared = self
            .prepare_catalog_plugin_install(&resolved, &reporter)
            .await?;
        let validated = self
            .validate_prepared_catalog_plugin_install(prepared)
            .await?;
        let bytes = validated.persisted_wasm_bytes.clone();
        let (updated, runtime) = validated.into_updated_installation(installation.clone())?;
        self.validate_component_upgrade_runtime(&installation, &runtime)
            .await?;
        self.services
            .customization
            .plugin_installations
            .update_plugin_installation(&updated, Some(&bytes))
            .await?;
        Ok(true)
    }

    async fn validate_component_upgrade_runtime(
        &self,
        installation: &PluginInstallation,
        runtime: &RuntimePluginLoad,
    ) -> AppResult<()> {
        if !same_plugin_identity(installation, &runtime.descriptor) {
            return Err(AppError::Validation(
                "Component replacement changed plugin identity".into(),
            ));
        }
        let loader = self.services.customization.plugin_descriptor_loader.clone();
        let bytes = runtime.wasm_bytes.clone();
        let descriptor =
            tokio::task::spawn_blocking(move || loader.validate_startup_component(&bytes))
                .await
                .map_err(|error| {
                    AppError::Repository(format!("Component validation task failed: {error}"))
                })??;
        if serde_json::to_value(&descriptor).ok() != serde_json::to_value(&runtime.descriptor).ok()
        {
            return Err(AppError::Validation(
                "Component descriptor differs from its persisted metadata".into(),
            ));
        }
        ensure_host_process_capability_allowed(&descriptor, installation.support_tier)?;
        validate_sdk_contract(
            &descriptor.id,
            &descriptor.sdk_version,
            &plugin_descriptor_sdk_constraint(&descriptor),
            SDK_VERSION,
        )
        .map_err(AppError::Validation)?;
        Ok(())
    }

    pub async fn set_plugin_component_blocker(
        &self,
        installation: &PluginInstallation,
        reason: Option<String>,
    ) -> AppResult<()> {
        let mut blockers = self.runtime.plugins.compatibility_blockers.write().await;
        if let Some(reason) = reason {
            blockers.insert(installation.plugin_id.clone(), reason);
        } else {
            blockers.remove(&installation.plugin_id);
        }
        Ok(())
    }
}

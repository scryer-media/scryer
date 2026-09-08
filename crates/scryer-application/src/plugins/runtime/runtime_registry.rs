pub const RUNTIME_PLUGIN_LOAD_CONCURRENCY: usize = 4;
impl AppUseCase {
    fn apply_runtime_plugin_removal_for_values(
        &self,
        plugin_type: &str,
        provider_type: &str,
    ) -> AppResult<()> {
        if is_indexer_plugin_type(plugin_type) {
            let provider = self
                .services
                .integrations
                .plugin_provider
                .available()
                .ok_or_else(|| {
                    AppError::Repository("indexer plugin provider unavailable".to_string())
                })?;
            provider.remove_runtime_plugin(provider_type).map_err(|e| {
                AppError::Repository(format!("failed to remove indexer plugin: {e}"))
            })?;
            return Ok(());
        }

        match plugin_type {
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
                provider.remove_runtime_plugin(provider_type).map_err(|e| {
                    AppError::Repository(format!("failed to remove download client plugin: {e}"))
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
                provider.remove_runtime_plugin(provider_type).map_err(|e| {
                    AppError::Repository(format!("failed to remove notification plugin: {e}"))
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
                provider.remove_runtime_plugin(provider_type).map_err(|e| {
                    AppError::Repository(format!("failed to remove subtitle plugin: {e}"))
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
                provider.remove_runtime_plugin(provider_type).map_err(|e| {
                    AppError::Repository(format!("failed to remove archive extractor plugin: {e}"))
                })?;
            }
            other => {
                return Err(AppError::Validation(format!(
                    "unsupported plugin_type '{}' for runtime removal",
                    other
                )));
            }
        }

        Ok(())
    }
}
impl AppUseCase {
    async fn finalize_runtime_plugin_mutation(
        &self,
        plugin_type: &str,
        runtime_touched: bool,
    ) -> AppResult<()> {
        self.finalize_runtime_plugin_mutation_for_types([plugin_type], runtime_touched)
            .await
    }
}
impl AppUseCase {
    async fn finalize_runtime_plugin_mutation_for_types<'a>(
        &self,
        plugin_types: impl IntoIterator<Item = &'a str>,
        runtime_touched: bool,
    ) -> AppResult<()> {
        let plugin_types = plugin_types.into_iter().collect::<Vec<_>>();
        // A successful manual repair must clear its startup diagnostic without
        // requiring a restart. Recheck only blocked installations; unrelated
        // mutations must not clear another provider's blocker.
        let blocked = self
            .runtime
            .plugins
            .compatibility_blockers
            .read()
            .await
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        for id in blocked {
            let installation = self
                .services
                .customization
                .plugin_installations
                .get_plugin_installation(&id)
                .await?;
            let recovered = match installation {
                None => true,
                Some(ref installation)
                    if installation_is_host_blocked(installation)
                        || !installation_sdk_contract_is_host_compatible(installation) =>
                {
                    false
                }
                Some(ref installation) => {
                    match self
                        .load_runtime_plugin_for_installation(installation)
                        .await
                    {
                        Ok(runtime) => self
                            .validate_component_upgrade_runtime(installation, &runtime)
                            .await
                            .is_ok(),
                        Err(_) => false,
                    }
                }
            };
            if recovered {
                self.runtime
                    .plugins
                    .compatibility_blockers
                    .write()
                    .await
                    .remove(&id);
            }
        }
        if runtime_touched
            && plugin_types
                .iter()
                .any(|plugin_type| is_indexer_plugin_type(plugin_type))
        {
            self.rebuild_user_rules_engine().await?;
        }

        let mut families = plugin_types
            .into_iter()
            .flat_map(provider_catalog_families_for_plugin_type)
            .collect::<Vec<_>>();
        families.sort_by_key(|family| family.as_str());
        families.dedup();
        self.publish_provider_catalog_changed(families);
        Ok(())
    }
}
impl AppUseCase {
    async fn build_available_plugins(&self) -> AppResult<Vec<RegistryPlugin>> {
        let installations = self
            .services
            .customization
            .plugin_installations
            .list_plugin_installations()
            .await?;
        // Any holder of the slot counts as busy, including the system actor
        // running a scheduled automatic update.
        let install_in_progress_ids = self
            .runtime
            .plugins
            .plugin_install_orchestrator
            .active_plugin_ids()
            .await;

        let sources = self
            .services
            .customization
            .plugin_installations
            .list_plugin_catalog_sources()
            .await?;
        let catalog_resolution = self
            .resolve_catalog_plugins_from_sources(&sources)
            .await
            .unwrap_or_default();
        let central = catalog_resolution.central;
        let resolved = catalog_resolution.resolved;
        let resolved_by_id = resolved
            .iter()
            .map(|resolved| (resolved.catalog_entry.id.clone(), resolved.clone()))
            .collect::<std::collections::HashMap<_, _>>();
        let builtin_by_key = self.builtin_seed_by_key();
        let effective_installations = installations.iter().collect::<Vec<_>>();

        let mut result = Vec::new();

        if let Some(central) = central {
            for entry in central.plugins {
                let inst = effective_installations
                    .iter()
                    .copied()
                    .find(|installation| installation.plugin_id == entry.id);
                let plugin_type =
                    merged_plugin_type(&entry.plugin_type, inst.map(|i| i.plugin_type.as_str()));
                if is_reserved_first_party_provider(&entry.provider_type) {
                    continue;
                }
                let builtin = inst
                    .map(|installation| installation.is_builtin)
                    .unwrap_or_else(|| {
                        builtin_by_key
                            .contains_key(&builtin_lookup_key(&plugin_type, &entry.provider_type))
                    });
                let selected = resolved_by_id.get(&entry.id);
                let selected_release = selected.map(|value| value.release.clone());
                let active_release =
                    inst.and_then(|installation| installed_catalog_release(&entry, installation));
                let display_release = selected_release.clone().or_else(|| active_release.clone());

                if inst.is_none() && display_release.is_none() && !builtin {
                    continue;
                }

                let version = display_release
                    .as_ref()
                    .map(|release| release.version.clone())
                    .or_else(|| inst.map(|installation| installation.version.clone()))
                    .unwrap_or_default();
                let update_available =
                    inst.zip(selected).is_some_and(|(installation, resolved)| {
                        catalog_plugin_update_available(installation, resolved)
                    });

                result.push(RegistryPlugin {
                    id: entry.id.clone(),
                    name: entry.name.clone(),
                    description: entry.description.clone(),
                    version,
                    latest_version: None,
                    plugin_type: plugin_type.clone(),
                    provider_type: entry.provider_type.clone(),
                    author: entry.publisher.clone(),
                    official: entry.support_tier == PluginSupportTier::Official,
                    publisher: Some(entry.publisher.clone()),
                    support_tier: entry.support_tier,
                    status: Some(lifecycle_status_label(entry.status)),
                    docs_url: Some(entry.docs_url.clone()),
                    source_repo: Some(entry.source_repo.clone()),
                    builtin,
                    source_url: inst
                        .and_then(|installation| installation.source_url.clone())
                        .or_else(|| selected.map(|value| value.artifact.url.clone())),
                    source_kind: inst
                        .map(|installation| source_kind_label(installation.source_kind))
                        .or_else(|| Some(source_kind_label(PluginSourceKind::Downloaded))),
                    blocked_reason: None,
                    wasm_url: selected.map(|value| value.artifact.url.clone()),
                    wasm_sha256: None,
                    min_scryer_version: selected_release
                        .as_ref()
                        .and_then(|release| release.min_scryer_version.clone()),
                    bytes: selected.map(|value| value.artifact.bytes),
                    default_base_url: self
                        .default_base_url_for_plugin(&plugin_type, &entry.provider_type),
                    is_installed: inst.is_some(),
                    is_enabled: inst.map(|i| i.is_enabled).unwrap_or(false),
                    installed_version: inst.map(|i| i.version.clone()),
                    update_available,
                    install_in_progress: install_in_progress_ids.contains(&entry.id),
                });
            }
        }

        for resolved in resolved.into_iter().filter(|resolved| {
            matches!(
                resolved.source_kind,
                PluginSourceKind::Community | PluginSourceKind::Manual
            )
        }) {
            let inst = effective_installations
                .iter()
                .copied()
                .find(|installation| installation.plugin_id == resolved.catalog_entry.id);
            if is_reserved_first_party_provider(&resolved.catalog_entry.provider_type) {
                continue;
            }
            let plugin_type = merged_plugin_type(
                &resolved.catalog_entry.plugin_type,
                inst.map(|i| i.plugin_type.as_str()),
            );
            let builtin = inst
                .map(|installation| installation.is_builtin)
                .unwrap_or_else(|| {
                    builtin_by_key.contains_key(&builtin_lookup_key(
                        &plugin_type,
                        &resolved.catalog_entry.provider_type,
                    ))
                });
            let update_available = inst.is_some_and(|installation| {
                catalog_plugin_update_available(installation, &resolved)
            });

            result.push(RegistryPlugin {
                id: resolved.catalog_entry.id.clone(),
                name: resolved.catalog_entry.name.clone(),
                description: resolved.catalog_entry.description.clone(),
                version: resolved.release.version.clone(),
                latest_version: None,
                plugin_type: plugin_type.clone(),
                provider_type: resolved.catalog_entry.provider_type.clone(),
                author: resolved.catalog_entry.publisher.clone(),
                official: resolved.effective_support_tier == PluginSupportTier::Official,
                publisher: Some(resolved.catalog_entry.publisher.clone()),
                support_tier: resolved.effective_support_tier,
                status: Some(lifecycle_status_label(resolved.catalog_entry.status)),
                docs_url: Some(resolved.catalog_entry.docs_url.clone()),
                source_repo: Some(resolved.catalog_entry.source_repo.clone()),
                builtin,
                source_url: inst
                    .and_then(|installation| installation.source_url.clone())
                    .or_else(|| Some(resolved.artifact.url.clone())),
                source_kind: inst
                    .map(|installation| source_kind_label(installation.source_kind))
                    .or_else(|| Some(source_kind_label(resolved.source_kind))),
                blocked_reason: None,
                wasm_url: Some(resolved.artifact.url.clone()),
                wasm_sha256: None,
                min_scryer_version: resolved.release.min_scryer_version.clone(),
                bytes: Some(resolved.artifact.bytes),
                default_base_url: self.default_base_url_for_plugin(
                    &plugin_type,
                    &resolved.catalog_entry.provider_type,
                ),
                is_installed: inst.is_some(),
                is_enabled: inst.map(|i| i.is_enabled).unwrap_or(false),
                installed_version: inst.map(|i| i.version.clone()),
                update_available,
                install_in_progress: install_in_progress_ids.contains(&resolved.catalog_entry.id),
            });
        }

        for inst in effective_installations {
            if is_reserved_first_party_provider(&inst.provider_type) {
                continue;
            }
            if !result.iter().any(|r| r.id == inst.plugin_id) {
                let builtin = builtin_by_key
                    .contains_key(&builtin_lookup_key(&inst.plugin_type, &inst.provider_type))
                    || inst.is_builtin;
                result.push(RegistryPlugin {
                    id: inst.plugin_id.clone(),
                    name: inst.name.clone(),
                    description: inst.description.clone(),
                    version: inst.version.clone(),
                    latest_version: None,
                    plugin_type: inst.plugin_type.clone(),
                    provider_type: inst.provider_type.clone(),
                    author: String::new(),
                    official: false,
                    publisher: inst.publisher.clone(),
                    support_tier: inst.support_tier,
                    status: None,
                    docs_url: inst.docs_url.clone(),
                    source_repo: inst.source_repo.clone(),
                    builtin,
                    source_url: inst.source_url.clone(),
                    source_kind: Some(source_kind_label(inst.source_kind)),
                    blocked_reason: None,
                    wasm_url: None,
                    wasm_sha256: None,
                    min_scryer_version: None,
                    bytes: None,
                    default_base_url: self
                        .default_base_url_for_plugin(&inst.plugin_type, &inst.provider_type),
                    is_installed: true,
                    is_enabled: inst.is_enabled,
                    installed_version: Some(inst.version.clone()),
                    update_available: false,
                    install_in_progress: install_in_progress_ids.contains(&inst.plugin_id),
                });
            }
        }

        let blockers = self.runtime.plugins.compatibility_blockers.read().await;
        for plugin in &mut result {
            plugin.blocked_reason = blockers.get(&plugin.id).cloned();
        }
        Ok(result)
    }
}
impl AppUseCase {
    /// List available plugins by merging cached registry with local installations.
    pub async fn list_available_plugins(&self, actor: &User) -> AppResult<Vec<RegistryPlugin>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        self.build_available_plugins().await
    }
}
impl AppUseCase {
    /// Refresh the plugin registry from the remote URL.
    pub async fn refresh_plugin_registry(&self, actor: &User) -> AppResult<Vec<RegistryPlugin>> {
        self.refresh_plugin_catalog(actor).await
    }
}
impl AppUseCase {
    /// Internal registry refresh (no auth check) for use by startup and background tasks.
    pub async fn refresh_plugin_registry_internal(&self) -> AppResult<()> {
        self.refresh_plugin_catalog_internal().await
    }
}

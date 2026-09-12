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

const COMPONENT_BLOCKER_RETAINED: &str =
    "The original installation and its configuration have been retained.";

/// Whether a catalog entry serves the same provider as an installed plugin.
///
/// Deliberately looser than `component_replacement_has_same_origin`: it ignores
/// the provenance an in-place upgrade needs, so it can find the build an
/// operator could install by hand.
fn catalog_entry_serves_same_provider(
    installation: &PluginInstallation,
    entry: &CatalogV3PluginEntry,
) -> bool {
    normalize_provider_key(&installation.provider_type)
        == normalize_provider_key(&entry.provider_type)
        && (installation.plugin_type == entry.plugin_type
            || (is_indexer_plugin_type(&installation.plugin_type)
                && is_indexer_plugin_type(&entry.plugin_type)))
}

/// The catalog entry that replaces a deprecated one, or `None`.
///
/// The catalog schema carries no successor field, so the replacement is only
/// claimed when exactly one live entry publishes the same product name for the
/// same plugin type — which is how `xbmc` resolves to `kodi`. Anything
/// ambiguous falls back to the terminal message rather than naming a guess.
fn catalog_successor_for<'a>(
    catalog: &'a CatalogV3,
    deprecated: &CatalogV3PluginEntry,
) -> Option<&'a CatalogV3PluginEntry> {
    let mut candidates = catalog.plugins.iter().filter(|entry| {
        entry.id != deprecated.id
            && entry.status != PluginLifecycleStatus::Deprecated
            && entry.plugin_type == deprecated.plugin_type
            && entry
                .name
                .trim()
                .eq_ignore_ascii_case(deprecated.name.trim())
    });
    let successor = candidates.next()?;
    candidates.next().is_none().then_some(successor)
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

        let resolutions = self.resolved_catalog_plugins().await?;
        let Some(resolved) = resolutions
            .iter()
            .find(|resolved| component_replacement_has_same_origin(&installation, resolved))
            .cloned()
        else {
            // One generic sentence covered three different operator actions.
            // Say which one this is.
            return Err(AppError::Validation(
                self.component_blocker_reason(&installation, &resolutions)
                    .await,
            ));
        };
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

    /// Why this installation has no component replacement, phrased as the step
    /// the operator can actually take.
    async fn component_blocker_reason(
        &self,
        installation: &PluginInstallation,
        resolutions: &[CatalogPluginResolution],
    ) -> String {
        // A build for this provider does resolve, just not from this
        // installation's origin, so it cannot be swapped in place. This is the
        // manual-upload case: the operator installs the catalog build instead.
        if let Some(available) = resolutions.iter().find(|resolved| {
            catalog_entry_serves_same_provider(installation, &resolved.catalog_entry)
        }) {
            return format!(
                "This build was not installed from the plugin catalog, so it cannot be upgraded in place. A compatible build of {} ({}) is available in the catalog; install it to replace this one. {COMPONENT_BLOCKER_RETAINED}",
                available.catalog_entry.name, available.release.version,
            );
        }

        // A deprecated entry publishes no runnable release, so it never reaches
        // `resolutions`. The raw catalog is the only place left that can say
        // what became of the plugin.
        if let Ok(Some(catalog)) = self.cached_central_catalog().await
            && let Some(entry) = catalog.plugins.iter().find(|entry| {
                entry.id == installation.plugin_id
                    || catalog_entry_serves_same_provider(installation, entry)
            })
            && entry.status == PluginLifecycleStatus::Deprecated
        {
            return match catalog_successor_for(&catalog, entry) {
                Some(successor) => format!(
                    "{} is deprecated and has no build for this version of Scryer. It is replaced by {} ({}); install that plugin and move this configuration to it. {COMPONENT_BLOCKER_RETAINED}",
                    entry.name, successor.name, successor.id,
                ),
                None => format!(
                    "{} is deprecated, has no build for this version of Scryer, and has no replacement in the catalog, so it cannot run on this version. {COMPONENT_BLOCKER_RETAINED}",
                    entry.name,
                ),
            };
        }

        format!(
            "No compatible component is published for {} by its original publisher and source repository. {COMPONENT_BLOCKER_RETAINED}",
            installation.name,
        )
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

    /// The compatibility blocker standing in the way of a provider, if any.
    ///
    /// Blockers are keyed by plugin id, but the provider-test and health paths
    /// only know the provider type the operator configured, so fall back to
    /// resolving through the installation list.
    pub async fn plugin_component_blocker_for_provider(
        &self,
        provider_type: &str,
    ) -> Option<String> {
        let blockers = self
            .runtime
            .plugins
            .compatibility_blockers
            .read()
            .await
            .clone();
        if blockers.is_empty() {
            return None;
        }
        let key = normalize_provider_key(provider_type);
        if let Some(reason) = blockers.get(&key) {
            return Some(reason.clone());
        }
        self.services
            .customization
            .plugin_installations
            .list_plugin_installations()
            .await
            .ok()?
            .iter()
            .find(|installation| normalize_provider_key(&installation.provider_type) == key)
            .and_then(|installation| blockers.get(&installation.plugin_id).cloned())
    }

    /// Refuse a provider connection or notification test when the plugin behind
    /// it is blocked.
    ///
    /// Without this the test paths fall through to their generic "provider
    /// unavailable" or "does not support test notifications" messages, which
    /// send the operator hunting for a configuration mistake that is not there.
    pub(crate) async fn ensure_provider_plugin_not_blocked(
        &self,
        provider_type: &str,
    ) -> AppResult<()> {
        match self
            .plugin_component_blocker_for_provider(provider_type)
            .await
        {
            Some(reason) => Err(AppError::Validation(format!(
                "The '{provider_type}' plugin is blocked on this version of Scryer, so it cannot be tested. {reason}"
            ))),
            None => Ok(()),
        }
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

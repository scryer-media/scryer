/// Result of one scheduled automatic plugin-update cycle.
#[derive(Clone, Debug, Default)]
pub(crate) struct PluginAutoUpdateReport {
    pub(crate) updated: Vec<PluginAutoUpdateUpgrade>,
    pub(crate) failed: Vec<PluginAutoUpdateFailure>,
    /// Candidate selection itself failed (nothing per-plugin ran).
    pub(crate) error: Option<String>,
}
impl PluginAutoUpdateReport {
    pub(crate) fn did_work(&self) -> bool {
        !self.updated.is_empty() || !self.failed.is_empty() || self.error.is_some()
    }

    pub(crate) fn has_failures(&self) -> bool {
        !self.failed.is_empty() || self.error.is_some()
    }
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PluginAutoUpdateUpgrade {
    pub(crate) plugin_id: String,
    pub(crate) from_version: String,
    pub(crate) to_version: String,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PluginAutoUpdateFailure {
    pub(crate) plugin_id: String,
    pub(crate) error: String,
    pub(crate) rolled_back: bool,
    pub(crate) rollback_error: Option<String>,
}
/// A candidate that did not survive its upgrade, plus what compensating it did.
struct PluginAutoUpdateFailed {
    error: AppError,
    rolled_back: bool,
    rollback_error: Option<String>,
}
/// Eligibility of the release the catalog already selected for this plugin.
///
/// Automation takes exactly the candidate the update badge and a manual upgrade
/// would take, restricted to stable releases on the installed major.minor.
fn plugin_auto_update_candidate(
    installation: &PluginInstallation,
    resolved: &CatalogPluginResolution,
) -> bool {
    if !catalog_plugin_update_available(installation, resolved) {
        return false;
    }
    let Some(selected) =
        parse_catalog_release_version(&resolved.catalog_entry.id, &resolved.release)
    else {
        return false;
    };
    let Ok(installed) = semver::Version::parse(installation.version.as_str()) else {
        return false;
    };
    selected.pre.is_empty()
        && selected.major == installed.major
        && selected.minor == installed.minor
}
/// The prior row as the upgrade left it: unchanged means nothing to compensate.
fn plugin_installation_matches_snapshot(
    current: &PluginInstallation,
    prior: &PluginInstallation,
) -> bool {
    current.version == prior.version
        && current.wasm_digest == prior.wasm_digest
        && current.artifact_digest == prior.artifact_digest
        && current.plugin_type == prior.plugin_type
        && current.provider_type == prior.provider_type
        && current.updated_at == prior.updated_at
}
impl AppUseCase {
    async fn collect_plugin_auto_update_candidates(
        &self,
    ) -> AppResult<Vec<(PluginInstallation, CatalogPluginResolution)>> {
        let mut installations = self
            .services
            .customization
            .plugin_installations
            .list_plugin_installations()
            .await?
            .into_iter()
            .filter(|installation| {
                installation_is_catalog_official(installation)
                    || (installation.is_builtin
                        && installation.source_kind == PluginSourceKind::Bundled)
            })
            .collect::<Vec<_>>();
        if installations.is_empty() {
            return Ok(Vec::new());
        }
        installations.sort_by(|left, right| left.plugin_id.cmp(&right.plugin_id));

        let resolved_by_id = self
            .resolved_catalog_plugins()
            .await?
            .into_iter()
            .map(|resolved| (resolved.catalog_entry.id.clone(), resolved))
            .collect::<HashMap<_, _>>();

        Ok(installations
            .into_iter()
            .filter_map(|installation| {
                let resolved = resolved_by_id.get(&installation.plugin_id)?;
                if !catalog_resolution_is_first_party(resolved) {
                    return None;
                }
                if !plugin_auto_update_candidate(&installation, resolved) {
                    return None;
                }
                Some((installation, resolved.clone()))
            })
            .collect())
    }
}
impl AppUseCase {
    /// Applies eligible official-plugin updates after a scheduled catalog
    /// refresh. Infallible by construction: automation must never abort the
    /// catalog-refresh job, so every internal error becomes a report entry.
    pub(crate) async fn run_scheduled_plugin_auto_update(&self) -> PluginAutoUpdateReport {
        let mut report = PluginAutoUpdateReport::default();
        match self.load_plugin_auto_update_settings().await {
            Ok(settings) => {
                if !settings.enabled {
                    return report;
                }
            }
            Err(error) => {
                warn!(
                    error = %error,
                    "skipping automatic plugin updates: settings are unavailable"
                );
                return report;
            }
        }

        let candidates = match self.collect_plugin_auto_update_candidates().await {
            Ok(candidates) => candidates,
            Err(error) => {
                warn!(error = %error, "automatic plugin update candidate selection failed");
                report.error = Some(error.to_string());
                return report;
            }
        };

        let actor = User::system_execution_actor();
        for (installation, resolved) in candidates {
            let plugin_id = installation.plugin_id.clone();
            if self
                .runtime
                .plugins
                .plugin_install_orchestrator
                .begin(&actor.id, &plugin_id, PluginInstallOperationKind::Upgrade)
                .await
                .is_err()
            {
                // An interactive operation owns the slot; the next scheduled
                // cycle retries this plugin.
                report.failed.push(PluginAutoUpdateFailure {
                    plugin_id,
                    error: "an install or upgrade is already in progress".to_string(),
                    rolled_back: false,
                    rollback_error: None,
                });
                continue;
            }

            let reporter = PluginInstallProgressReporter::new(self, &actor.id, &plugin_id);
            let from_version = installation.version.clone();
            match self
                .upgrade_catalog_plugin_with_rollback(resolved, installation, &reporter)
                .await
            {
                Ok(updated) => {
                    reporter.succeeded().await;
                    tracing::info!(
                        plugin_id = plugin_id.as_str(),
                        from_version = from_version.as_str(),
                        to_version = updated.version.as_str(),
                        "automatically updated official plugin"
                    );
                    report.updated.push(PluginAutoUpdateUpgrade {
                        plugin_id,
                        from_version,
                        to_version: updated.version,
                    });
                }
                Err(failure) => {
                    reporter.failed(&failure.error).await;
                    warn!(
                        plugin_id = plugin_id.as_str(),
                        error = %failure.error,
                        "automatic plugin update failed"
                    );
                    report.failed.push(PluginAutoUpdateFailure {
                        plugin_id,
                        error: failure.error.to_string(),
                        rolled_back: failure.rolled_back,
                        rollback_error: failure.rollback_error,
                    });
                }
            }
        }

        report
    }
}
impl AppUseCase {
    async fn upgrade_catalog_plugin_with_rollback(
        &self,
        resolved: CatalogPluginResolution,
        installation: PluginInstallation,
        reporter: &PluginInstallProgressReporter,
    ) -> Result<PluginInstallation, PluginAutoUpdateFailed> {
        let prior_record = installation.clone();
        // Automation only touches a plugin it can put back. Downloaded plugins
        // need their persisted artifact; bundled plugins can be restored from
        // the host's built-in runtime.
        let prior_wasm = match self
            .services
            .customization
            .plugin_installations
            .get_plugin_installation_wasm_payload(&prior_record.plugin_id)
            .await
        {
            Ok(Some(payload)) => Some(payload),
            Ok(None)
                if prior_record.is_builtin
                    && prior_record.source_kind == PluginSourceKind::Bundled =>
            {
                None
            }
            Ok(None) => {
                return Err(PluginAutoUpdateFailed {
                    error: AppError::Validation(format!(
                        "plugin '{}' is missing persisted WASM payload",
                        prior_record.plugin_id
                    )),
                    rolled_back: false,
                    rollback_error: None,
                });
            }
            Err(error) => {
                return Err(PluginAutoUpdateFailed {
                    error,
                    rolled_back: false,
                    rollback_error: None,
                });
            }
        };

        let error = match self
            .upgrade_catalog_plugin(resolved, installation, reporter)
            .await
        {
            Ok(updated) => return Ok(updated),
            Err(error) => error,
        };

        // The upgrade persists the installation row before it touches the
        // runtime, so a row that still matches the snapshot proves the runtime
        // was never re-registered either: there is nothing to compensate.
        let current = match self
            .services
            .customization
            .plugin_installations
            .get_plugin_installation(&prior_record.plugin_id)
            .await
        {
            Ok(current) => current,
            Err(read_error) => {
                warn!(
                    plugin_id = prior_record.plugin_id.as_str(),
                    error = %read_error,
                    "failed to read back a plugin installation after a failed automatic update"
                );
                return Err(PluginAutoUpdateFailed {
                    error,
                    rolled_back: false,
                    rollback_error: Some(read_error.to_string()),
                });
            }
        };
        let Some(current) =
            current.filter(|current| !plugin_installation_matches_snapshot(current, &prior_record))
        else {
            return Err(PluginAutoUpdateFailed {
                error,
                rolled_back: false,
                rollback_error: None,
            });
        };

        let rollback = match prior_wasm.as_ref() {
            Some(prior_wasm) => {
                self.restore_plugin_installation_snapshot(&prior_record, prior_wasm, &current)
                    .await
            }
            None => {
                self.restore_bundled_plugin_installation_snapshot(&prior_record, &current)
                    .await
            }
        };

        match rollback {
            Ok(()) => Err(PluginAutoUpdateFailed {
                error,
                rolled_back: true,
                rollback_error: None,
            }),
            Err(rollback_error) => {
                warn!(
                    plugin_id = prior_record.plugin_id.as_str(),
                    error = %rollback_error,
                    "failed to roll back automatic plugin update"
                );
                Err(PluginAutoUpdateFailed {
                    error,
                    rolled_back: false,
                    rollback_error: Some(rollback_error.to_string()),
                })
            }
        }
    }
}
impl AppUseCase {
    /// Puts the prior record, its artifact, and its runtime registration back
    /// after an upgrade that already persisted `current`. A disabled plugin was
    /// never handed to the runtime by the upgrade (`runtime_touched =
    /// result.is_enabled`), so only its row needs restoring.
    async fn restore_plugin_installation_snapshot(
        &self,
        prior_record: &PluginInstallation,
        prior_wasm: &PersistedPluginWasmPayload,
        current: &PluginInstallation,
    ) -> AppResult<()> {
        let restored = self
            .services
            .customization
            .plugin_installations
            .update_plugin_installation(prior_record, Some(prior_wasm.bytes.as_slice()))
            .await?;

        if restored.is_enabled {
            let runtime_plugin = self.load_runtime_plugin_for_installation(&restored).await?;
            self.publish_runtime_plugin(Some(current), &restored, runtime_plugin)
                .await?;
        }

        self.finalize_runtime_plugin_mutation_for_types(
            [current.plugin_type.as_str(), restored.plugin_type.as_str()],
            restored.is_enabled,
        )
        .await
    }

    async fn restore_bundled_plugin_installation_snapshot(
        &self,
        prior_record: &PluginInstallation,
        current: &PluginInstallation,
    ) -> AppResult<()> {
        let restored = self
            .services
            .customization
            .plugin_installations
            .update_plugin_installation(prior_record, None)
            .await?;

        if restored.is_enabled {
            if current.plugin_type != restored.plugin_type
                || current.provider_type != restored.provider_type
            {
                self.apply_runtime_plugin_removal(current)?;
            }
            self.apply_runtime_builtin_restore(&restored).await?;
        }

        self.finalize_runtime_plugin_mutation_for_types(
            [current.plugin_type.as_str(), restored.plugin_type.as_str()],
            restored.is_enabled,
        )
        .await
    }
}

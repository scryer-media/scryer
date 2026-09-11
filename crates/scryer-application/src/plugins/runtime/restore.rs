#[derive(Clone, Debug)]
pub struct PluginCatalogStatus {
    pub refresh_state: String,
    pub github_available: bool,
    pub last_checked_at: Option<String>,
    pub outage_message: Option<String>,
    pub blocked_actions: Vec<String>,
    pub restore_warnings: Vec<String>,
    pub last_error: Option<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredPluginCatalogStatusPayload {
    #[serde(default)]
    github_available: bool,
    #[serde(default)]
    blocked_actions: Vec<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    restore_warnings: Vec<String>,
    #[serde(default)]
    last_error: Option<String>,
}
#[derive(Clone, Debug)]
struct RestoredPluginRecoveryTarget {
    installation: PluginInstallation,
    source_repo: Option<String>,
}
struct PreparedRestoredPluginRecovery {
    updated_installation: PluginInstallation,
    persisted_wasm_bytes: Vec<u8>,
}
const RESTORE_PLUGIN_RECOVERY_ACTOR_ID: &str = "system:restore-plugin-recovery";
fn restore_warning_label(installation: &PluginInstallation) -> &str {
    if installation.name.trim().is_empty() {
        installation.plugin_id.as_str()
    } else {
        installation.name.as_str()
    }
}
/// The warning for a plugin that came back at a version other than the one the
/// backup recorded, or `None` when the versions match.
///
/// Recovery deliberately installs whatever the catalog publishes now rather
/// than pinning the backup's version; this only makes that visible.
fn restore_version_change_warning(
    installation: &PluginInstallation,
    bundle_version: &str,
) -> Option<String> {
    (installation.version != bundle_version).then(|| {
        format!(
            "Plugin '{}' was restored at version {} rather than version {} from the backup, because that is the version the plugin catalog publishes now.",
            restore_warning_label(installation),
            installation.version,
            bundle_version,
        )
    })
}

impl AppUseCase {
    pub async fn recover_restored_plugins_after_backup_restore(&self) -> AppResult<()> {
        let installations = self
            .services
            .customization
            .plugin_installations
            .list_plugin_installations()
            .await?;

        let mut recoverable = Vec::new();
        let mut skipped_local_uploads = Vec::new();
        for installation in installations {
            match installation.source_kind {
                PluginSourceKind::Downloaded | PluginSourceKind::Community => {
                    recoverable.push(RestoredPluginRecoveryTarget {
                        installation,
                        source_repo: None,
                    });
                }
                PluginSourceKind::Manual => {
                    let source_repo = installation
                        .source_repo
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(str::to_string);
                    if source_repo.is_some() {
                        recoverable.push(RestoredPluginRecoveryTarget {
                            installation,
                            source_repo,
                        });
                    } else {
                        skipped_local_uploads.push(installation);
                    }
                }
                PluginSourceKind::Bundled => {}
            }
        }

        // Captured before the recovery tasks consume `recoverable`, so the
        // version the bundle carried can be compared with what the catalog
        // actually published back.
        let bundle_versions = recoverable
            .iter()
            .map(|target| {
                (
                    target.installation.plugin_id.clone(),
                    target.installation.version.clone(),
                )
            })
            .collect::<std::collections::HashMap<_, _>>();

        // Every consequence of the skip in one line: the upload is gone, the
        // configuration that used it is not, and it is still switched on.
        // Saying only that the restore was skipped left the operator with a
        // provider that looks configured and enabled but cannot run.
        let mut restore_warnings = skipped_local_uploads
            .iter()
            .map(|installation| {
                format!(
                    "Skipped restoring plugin '{}' because it was uploaded locally and cannot be re-downloaded from a remote catalog source. Its installation has been removed, but its saved configuration remains and is still enabled, so it will not work until you upload the plugin again.",
                    restore_warning_label(installation)
                )
            })
            .collect::<Vec<_>>();

        let prepared_updates = if recoverable.is_empty() {
            Vec::new()
        } else {
            for target in &recoverable {
                if let Some(source_repo) = target.source_repo.as_deref() {
                    self.ensure_manual_plugin_catalog_source_for_restore(source_repo)
                        .await?;
                }
            }

            self.refresh_plugin_catalog_internal().await?;
            let resolved_plugins = self.resolved_catalog_plugins().await?;
            let mut recovery_tasks = tokio::task::JoinSet::new();
            for target in recoverable {
                let app = self.clone();
                let resolved = resolved_plugins
                    .iter()
                    .find(|candidate| {
                        app.catalog_resolution_matches_restored_installation(candidate, &target)
                    })
                    .cloned()
                    .ok_or_else(|| {
                        AppError::NotFound(format!(
                            "plugin '{}' is not available from the plugin catalog",
                            target.installation.plugin_id
                        ))
                    })?;
                recovery_tasks.spawn(async move {
                    app.prepare_restored_plugin_recovery(target, resolved).await
                });
            }

            let mut prepared = Vec::new();
            while let Some(joined) = recovery_tasks.join_next().await {
                let prepared_update = joined.map_err(|error| {
                    AppError::Repository(format!(
                        "restored plugin recovery task failed to complete: {error}"
                    ))
                })??;
                prepared.push(prepared_update);
            }
            prepared
        };
        let rebuild_required = !prepared_updates.is_empty() || !skipped_local_uploads.is_empty();

        for installation in &skipped_local_uploads {
            self.services
                .customization
                .plugin_installations
                .delete_plugin_installation(&installation.plugin_id)
                .await?;
        }

        for prepared in prepared_updates {
            // Recovery installs whatever the catalog publishes now, which need
            // not be the version the backup recorded. Restoring a different
            // version is the intended behaviour, but a silent one is a change
            // to the operator's fleet that they never agreed to.
            if let Some(bundle_version) =
                bundle_versions.get(&prepared.updated_installation.plugin_id)
                && let Some(warning) =
                    restore_version_change_warning(&prepared.updated_installation, bundle_version)
            {
                restore_warnings.push(warning);
            }
            self.services
                .customization
                .plugin_installations
                .update_plugin_installation(
                    &prepared.updated_installation,
                    Some(prepared.persisted_wasm_bytes.as_slice()),
                )
                .await?;
        }

        self.set_plugin_restore_warnings(restore_warnings).await?;

        if rebuild_required {
            self.rebuild_plugin_provider().await?;
        }

        Ok(())
    }
}
impl AppUseCase {
    async fn set_plugin_restore_warnings(&self, restore_warnings: Vec<String>) -> AppResult<()> {
        let mut payload = self.load_stored_plugin_catalog_status_payload().await?;
        payload.restore_warnings = restore_warnings;
        self.persist_plugin_catalog_status_payload(payload, Utc::now())
            .await
    }
}
impl AppUseCase {
    fn catalog_resolution_matches_restored_installation(
        &self,
        resolved: &CatalogPluginResolution,
        target: &RestoredPluginRecoveryTarget,
    ) -> bool {
        if resolved.catalog_entry.id != target.installation.plugin_id {
            return false;
        }

        match target.installation.source_kind {
            PluginSourceKind::Downloaded => resolved.source_kind == PluginSourceKind::Downloaded,
            PluginSourceKind::Community => resolved.source_kind == PluginSourceKind::Community,
            PluginSourceKind::Manual => target
                .source_repo
                .as_deref()
                .and_then(|source_repo| GitHubRepo::parse(source_repo).ok())
                .is_some_and(|repo| {
                    resolved.source_kind == PluginSourceKind::Manual
                        && resolved.github_repo.slug() == repo.slug()
                }),
            PluginSourceKind::Bundled => false,
        }
    }
}

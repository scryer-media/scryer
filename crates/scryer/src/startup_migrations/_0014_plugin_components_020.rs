use scryer_application::{AppUseCase, PluginInstallationRepository, RuntimePluginLoad};
use scryer_infrastructure_runtime::DatastoreCustomizationStore;
use scryer_infrastructure_sql::runtime::StoreDatastore;

use super::compatibility_journal as journal;
pub(crate) const ID: &str = "0014_plugin_components_020";

#[cfg(test)]
#[path = "plugin_compatibility_tests.rs"]
mod tests;

/// What one compatibility pass did, so the completion log and the tests read
/// the same numbers.
pub(crate) struct ComponentPassCounts {
    pub(crate) replaced: usize,
    pub(crate) blocked: usize,
    pub(crate) unchanged: usize,
}

pub(crate) async fn migrate(
    app: &AppUseCase,
    datastore: &StoreDatastore,
    store: &DatastoreCustomizationStore,
) -> Result<bool, String> {
    let counts = run_pass(app, datastore, store).await?;
    // Emitted whatever the verdict. The runner records a ledger row, and logs
    // its own completion line, only when this returns true, so with any plugin
    // blocked an operator otherwise saw no evidence the pass had run at all.
    tracing::info!(
        migration_id = ID,
        replaced = counts.replaced,
        blocked = counts.blocked,
        unchanged = counts.unchanged,
        "plugin component compatibility pass completed"
    );
    Ok(counts.blocked == 0)
}

pub(crate) async fn run_pass(
    app: &AppUseCase,
    datastore: &StoreDatastore,
    store: &DatastoreCustomizationStore,
) -> Result<ComponentPassCounts, String> {
    journal::ensure(datastore).await?;
    let mut bundled = Vec::new();
    for asset in scryer_plugins::builtins::INDEXER_BUILTINS
        .iter()
        .chain(scryer_plugins::builtins::SUBTITLE_BUILTINS)
        .chain(scryer_plugins::builtins::DOWNLOAD_CLIENT_BUILTINS)
        .chain(scryer_plugins::builtins::NOTIFICATION_BUILTINS)
    {
        bundled.push(RuntimePluginLoad {
            descriptor: serde_json::from_str(asset.descriptor_json)
                .map_err(|error| error.to_string())?,
            wasm_bytes: scryer_plugins::builtins::decode_builtin_wasm(*asset)?,
            first_party: true,
        });
    }
    let installations = store
        .list_plugin_installations()
        .await
        .map_err(|error| error.to_string())?;
    // Read before the `pending` writes below: `record` overwrites the status
    // for a key, so a prior boot's verdict is only visible up here.
    let previously_blocked = journal::blocked_subjects(datastore, ID).await?;
    let mut outcomes = Vec::new();
    let mut refreshed = false;
    for installation in installations {
        let payload = store
            .get_plugin_installation_wasm_payload(&installation.plugin_id)
            .await
            .map_err(|error| error.to_string())?;
        let original = serde_json::to_string(&installation).map_err(|error| error.to_string())?;
        // Builtin seeding refreshes updated_at on every boot. Retain that
        // timestamp in the first journal snapshot without treating it as a
        // change to the artifact or its compatibility contract.
        let mut compatibility_metadata =
            serde_json::to_value(&installation).map_err(|error| error.to_string())?;
        compatibility_metadata
            .as_object_mut()
            .ok_or("plugin installation metadata must be an object")?
            .remove("updated_at");
        let builtin = bundled
            .iter()
            .find(|plugin| plugin.descriptor.id == installation.plugin_id)
            .cloned();
        let evidence = serde_json::to_vec(&(
            env!("CARGO_PKG_VERSION"),
            &compatibility_metadata,
            payload
                .as_ref()
                .map(|payload| scryer_application::plugin_wasm_blake3_digest(&payload.bytes)),
            builtin
                .as_ref()
                .map(|builtin| scryer_application::plugin_wasm_blake3_digest(&builtin.wasm_bytes)),
        ))
        .map_err(|error| error.to_string())?;
        let digest = scryer_application::plugin_wasm_blake3_digest(&evidence);
        // Revalidate initialization on every boot: two builds can share a
        // version while carrying different host capabilities. Valid artifacts
        // are read-only no-ops; the journal prevents losing original evidence.
        journal::record(
            datastore,
            ID,
            &installation.id,
            &digest,
            &original,
            "pending",
            None,
        )
        .await?;
        let mut outcome = app
            .repair_plugin_component_installation(installation.clone(), builtin.clone())
            .await;
        // A stale catalog must not strand a recoverable upgrade. This refresh
        // is independent of scheduled auto-updates and attempted once per boot.
        //
        // Skip the network call when this exact evidence digest was already
        // recorded blocked: the catalog was refreshed on the boot that produced
        // that verdict and still did not resolve the plugin, so repeating it
        // every boot only costs a request. The digest covers the app version,
        // the installation metadata and both artifact digests, so a new build
        // or any change to the installation retries the refresh. Re-inspection
        // itself still runs every boot; only the fetch is suppressed.
        let evidence_already_blocked =
            previously_blocked.contains(&(installation.id.clone(), digest.clone()));
        if outcome.is_err()
            && !evidence_already_blocked
            && installation.publisher.is_some()
            && installation.source_repo.is_some()
        {
            if !refreshed {
                refreshed = true;
                if let Err(error) = app.refresh_plugin_catalog_internal().await {
                    tracing::warn!(%error, "component compatibility catalog refresh unavailable");
                }
            }
            outcome = app
                .repair_plugin_component_installation(installation.clone(), builtin)
                .await;
        }
        let replaced = matches!(outcome, Ok(true));
        let reason = outcome.err().map(|error| error.to_string());
        app.set_plugin_component_blocker(&installation, reason.clone())
            .await
            .map_err(|error| error.to_string())?;
        journal::record(
            datastore,
            ID,
            &installation.id,
            &digest,
            &original,
            if reason.is_some() {
                "blocked"
            } else {
                "validated"
            },
            reason.clone(),
        )
        .await?;
        outcomes.push((installation, reason, replaced));
    }
    // Rebuild once after durable repairs, before consumers can start. Reload
    // excludes blockers and suppresses their bundled fallbacks.
    app.reload_plugin_providers()
        .await
        .map_err(|error| error.to_string())?;
    let mut counts = ComponentPassCounts {
        replaced: 0,
        blocked: 0,
        unchanged: 0,
    };
    for (installation, reason, replaced) in outcomes {
        if let Some(reason) = &reason {
            counts.blocked += 1;
            tracing::warn!(plugin_id = installation.plugin_id, %reason,
                "plugin component upgrade blocked; original installation retained");
        } else if replaced {
            counts.replaced += 1;
        } else {
            counts.unchanged += 1;
        }
    }
    Ok(counts)
}

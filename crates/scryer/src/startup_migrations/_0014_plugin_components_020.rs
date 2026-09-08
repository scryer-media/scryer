use scryer_application::{AppUseCase, PluginInstallationRepository, RuntimePluginLoad};
use scryer_infrastructure_runtime::DatastoreCustomizationStore;
use scryer_infrastructure_sql::runtime::StoreDatastore;

use super::compatibility_journal as journal;
pub(crate) const ID: &str = "0014_plugin_components_020";

#[cfg(test)]
#[path = "plugin_compatibility_tests.rs"]
mod tests;

pub(crate) async fn migrate(
    app: &AppUseCase,
    datastore: &StoreDatastore,
    store: &DatastoreCustomizationStore,
) -> Result<bool, String> {
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
        if outcome.is_err()
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
        outcomes.push((installation, reason));
    }
    // Rebuild once after durable repairs, before consumers can start. Reload
    // excludes blockers and suppresses their bundled fallbacks.
    app.reload_plugin_providers()
        .await
        .map_err(|error| error.to_string())?;
    let mut blocked = 0usize;
    for (installation, reason) in outcomes {
        if let Some(reason) = &reason {
            blocked += 1;
            tracing::warn!(plugin_id = installation.plugin_id, %reason,
                "plugin component upgrade blocked; original installation retained");
        }
    }
    Ok(blocked == 0)
}

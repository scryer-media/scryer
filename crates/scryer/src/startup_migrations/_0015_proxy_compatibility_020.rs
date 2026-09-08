use super::compatibility_journal as journal;
use scryer_application::ProxyConfigRepository;
use scryer_infrastructure_sql::runtime::{SqlRuntime, StoreDatastore};
use std::collections::HashMap;
use std::sync::Arc;

pub(crate) const ID: &str = "0015_proxy_compatibility_020";

#[cfg(test)]
#[path = "proxy_compatibility_tests.rs"]
mod tests;

/// Runs after 0218/0219 and key bootstrap, before any outbound clients exist.
/// Existing SQL migrations remain immutable. Invalid configurations and
/// assignments are retained, with one diagnostic per affected provider.
pub(crate) async fn migrate(
    datastore: &StoreDatastore,
    proxies: Arc<dyn ProxyConfigRepository>,
) -> Result<bool, String> {
    journal::ensure(datastore).await?;
    let rows = SqlRuntime::fetch_all(datastore.read_exec(),
        "SELECT id, provider_type, protocol, base_url, request_timeout_seconds, is_enabled, updated_at
         FROM proxy_configs ORDER BY id", &[]).await.map_err(|error| error.to_string())?;
    let mut states = HashMap::new();
    let mut blocked = 0usize;
    for row in rows {
        let id = row.text("id").map_err(|error| error.to_string())?;
        let original = serde_json::to_string(&(
            &id,
            row.text("provider_type")
                .map_err(|error| error.to_string())?,
            row.opt_text("protocol")
                .map_err(|error| error.to_string())?,
            row.text("base_url").map_err(|error| error.to_string())?,
            row.i64("request_timeout_seconds")
                .map_err(|error| error.to_string())?,
            row.bool("is_enabled").map_err(|error| error.to_string())?,
        ))
        .map_err(|error| error.to_string())?;
        let digest = scryer_application::plugin_wasm_blake3_digest(original.as_bytes());
        // Always revalidate decryption against this boot's key state. A prior
        // successful startup is not proof that today's key can read the row.
        let outcome = proxies.get_by_id(&id).await;
        let (enabled, reason) = match outcome {
            Ok(Some(proxy)) => (proxy.is_enabled, None),
            Ok(None) => (
                false,
                Some("Proxy configuration disappeared during startup".into()),
            ),
            Err(error) => (false, Some(error.to_string())),
        };
        if reason.is_some() {
            blocked += 1;
        }
        journal::record(
            datastore,
            ID,
            &format!("proxy:{id}"),
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
        states.insert(id, (enabled, reason));
    }
    // Fixed identifiers, never operator-provided SQL.
    for (table, family) in [
        ("indexers", "indexer"),
        ("download_clients", "download_client"),
    ] {
        let sql = format!(
            "SELECT id, proxy_config_id, is_enabled FROM {table} WHERE proxy_config_id IS NOT NULL ORDER BY id"
        );
        for row in SqlRuntime::fetch_all(datastore.read_exec(), &sql, &[])
            .await
            .map_err(|error| error.to_string())?
        {
            let id = row.text("id").map_err(|error| error.to_string())?;
            let proxy = row
                .text("proxy_config_id")
                .map_err(|error| error.to_string())?;
            let enabled = row.bool("is_enabled").map_err(|error| error.to_string())?;
            let reason = match states.get(&proxy) {
                None => Some(
                    "Assigned proxy does not exist; restore it or explicitly change the assignment"
                        .to_string(),
                ),
                Some((_, Some(reason))) => Some(format!("Assigned proxy is blocked: {reason}")),
                Some((false, None)) if enabled => Some("Assigned proxy is disabled".to_string()),
                _ => None,
            };
            if reason.is_some() {
                blocked += 1;
            }
            let original = serde_json::to_string(&(&id, &proxy, enabled))
                .map_err(|error| error.to_string())?;
            let digest = scryer_application::plugin_wasm_blake3_digest(original.as_bytes());
            journal::record(
                datastore,
                ID,
                &format!("{family}:{id}"),
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
            if let Some(reason) = reason {
                tracing::warn!(provider_id = id, proxy_id = proxy, %reason, "proxy upgrade assignment blocked");
            }
        }
    }
    Ok(blocked == 0)
}

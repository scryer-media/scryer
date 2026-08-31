use std::sync::Arc;

use scryer_application::{AppUseCase, SETTINGS_SCOPE_SYSTEM};
use scryer_infrastructure_configuration::settings::settings_store::SettingsStore;

pub(crate) const DOWNLOAD_CLIENT_REMOVE_FAILED_DEFAULT_FLIPPED_0018_STATE_KEY: &str =
    "download_client.remove_failed_default_flipped_0018";
const STATE_NONE: &str = "none";
const STATE_PENDING: &str = "pending";
const STATE_COMPLETED: &str = "completed";

async fn read_state(settings_store: Arc<SettingsStore>) -> String {
    settings_store
        .get_setting_with_defaults(
            SETTINGS_SCOPE_SYSTEM,
            DOWNLOAD_CLIENT_REMOVE_FAILED_DEFAULT_FLIPPED_0018_STATE_KEY,
            None,
        )
        .await
        .ok()
        .flatten()
        .and_then(|record| serde_json::from_str::<String>(&record.effective_value_json).ok())
        .unwrap_or_else(|| STATE_NONE.to_string())
}

async fn set_state(settings_store: Arc<SettingsStore>, state: &str) -> Result<(), String> {
    let value_json = serde_json::to_string(state).map_err(|error| error.to_string())?;
    settings_store
        .upsert_setting_value(
            SETTINGS_SCOPE_SYSTEM,
            DOWNLOAD_CLIENT_REMOVE_FAILED_DEFAULT_FLIPPED_0018_STATE_KEY,
            None,
            value_json,
            "system",
            None,
        )
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// One-time correction for `removeFailed: false` values written by the old
/// startup normalization default, not by an operator choice.
pub(crate) async fn flip_download_client_remove_failed_default(
    app_use_case: &AppUseCase,
    settings_store: Arc<SettingsStore>,
) -> Result<(), String> {
    if read_state(settings_store.clone()).await == STATE_COMPLETED {
        return Ok(());
    }

    set_state(settings_store.clone(), STATE_PENDING).await?;

    let flipped = app_use_case
        .flip_explicit_remove_failed_defaults()
        .await
        .map_err(|error| error.to_string())?;
    set_state(settings_store, STATE_COMPLETED).await?;
    tracing::info!(
        flipped,
        "flipped persisted download-client remove-failed defaults"
    );
    Ok(())
}

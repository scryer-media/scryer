use std::sync::Arc;

use scryer_application::{AppUseCase, SETTINGS_SCOPE_SYSTEM};
use scryer_infrastructure_configuration::settings::settings_store::SettingsStore;

use super::versioning::{MajorMinor, parse_major_minor};

pub(crate) const TITLE_METADATA_REHYDRATION_017_STATE_KEY: &str =
    "catalog.title_metadata_rehydration_017_state";
const STATE_NONE: &str = "none";
const STATE_PENDING: &str = "pending";
const STATE_COMPLETED: &str = "completed";

fn is_017_or_later(version: &str) -> bool {
    parse_major_minor(version).is_some_and(|version| version >= MajorMinor::new(0, 17))
}

fn should_attempt_title_metadata_rehydration(current_version: &str, migration_state: &str) -> bool {
    migration_state != STATE_COMPLETED && is_017_or_later(current_version)
}

async fn read_state(settings_store: Arc<SettingsStore>) -> String {
    settings_store
        .get_setting_with_defaults(
            SETTINGS_SCOPE_SYSTEM,
            TITLE_METADATA_REHYDRATION_017_STATE_KEY,
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
            TITLE_METADATA_REHYDRATION_017_STATE_KEY,
            None,
            value_json,
            "system",
            None,
        )
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}

pub(crate) async fn rehydrate_title_metadata_for_017_upgrade(
    app_use_case: &AppUseCase,
    settings_store: Arc<SettingsStore>,
    current_version: &str,
) -> Result<(), String> {
    let state = read_state(settings_store.clone()).await;
    if !should_attempt_title_metadata_rehydration(current_version, &state) {
        return Ok(());
    }

    set_state(settings_store.clone(), STATE_PENDING).await?;

    let titles_rehydrated = app_use_case
        .hydrate_all_titles_for_current_language()
        .await
        .map_err(|error| error.to_string())?;
    set_state(settings_store, STATE_COMPLETED).await?;
    tracing::info!(
        titles_rehydrated,
        "0.17 title metadata rehydration migration completed"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_metadata_rehydration_runs_once_on_any_017_or_later_host() {
        assert!(should_attempt_title_metadata_rehydration(
            "0.17.0", STATE_NONE,
        ));
        assert!(should_attempt_title_metadata_rehydration(
            "0.17.0-rc.1",
            STATE_NONE,
        ));
        assert!(should_attempt_title_metadata_rehydration(
            "0.18.0", STATE_NONE,
        ));
        assert!(should_attempt_title_metadata_rehydration(
            "0.17.0",
            STATE_PENDING,
        ));
        assert!(!should_attempt_title_metadata_rehydration(
            "0.16.9",
            STATE_PENDING,
        ));
        assert!(!should_attempt_title_metadata_rehydration(
            "0.17.0",
            STATE_COMPLETED,
        ));
    }
}

use std::sync::Arc;

use scryer_application::{
    HISTORY_KEEP_FOREVER_KEY, HISTORY_RETENTION_DAYS_KEY, SETTINGS_SCOPE_SYSTEM,
};
use scryer_infrastructure_configuration::settings::settings_store::SettingsStore;

pub(crate) async fn clear_legacy_history_retention_forever_override(
    settings_store: Arc<SettingsStore>,
) -> Result<(), String> {
    let keep_forever = settings_store
        .get_setting_with_defaults(SETTINGS_SCOPE_SYSTEM, HISTORY_KEEP_FOREVER_KEY, None)
        .await
        .map_err(|error| error.to_string())?;
    let retention_days = settings_store
        .get_setting_with_defaults(SETTINGS_SCOPE_SYSTEM, HISTORY_RETENTION_DAYS_KEY, None)
        .await
        .map_err(|error| error.to_string())?;

    let should_clear = keep_forever.as_ref().is_some_and(|record| {
        record.source.as_deref() == Some("migration")
            && record.value_json.as_deref() == Some("true")
            && !retention_days
                .as_ref()
                .is_some_and(scryer_infrastructure_sql::types::SettingsValueRecord::has_override)
    });

    if !should_clear {
        return Ok(());
    }

    settings_store
        .delete_setting_value(SETTINGS_SCOPE_SYSTEM, HISTORY_KEEP_FOREVER_KEY, None)
        .await
        .map_err(|error| error.to_string())?;
    tracing::info!("cleared legacy history retention forever override");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings_bootstrap::seed_service_setting_definitions;
    use scryer_infrastructure_datastore::{MigrationMode, SqliteServices};

    async fn bootstrap_settings_store() -> (tempfile::TempDir, Arc<SettingsStore>) {
        let temp = tempfile::tempdir().expect("tempdir");
        let db_path = temp.path().join("scryer.db");
        let services = SqliteServices::new_with_mode(
            db_path.to_string_lossy().to_string(),
            MigrationMode::Apply,
        )
        .await
        .expect("sqlite services");
        let store = Arc::new(SettingsStore::new(
            services.datastore(),
            services.encryption_key_state(),
        ));
        seed_service_setting_definitions(store.clone())
            .await
            .expect("seed setting definitions");
        (temp, store)
    }

    #[tokio::test]
    async fn legacy_migration_history_override_is_cleared_back_to_default() {
        let (_temp, store) = bootstrap_settings_store().await;
        store
            .upsert_setting_value(
                SETTINGS_SCOPE_SYSTEM,
                HISTORY_KEEP_FOREVER_KEY,
                None,
                "true",
                "migration",
                None,
            )
            .await
            .expect("seed legacy migration override");

        clear_legacy_history_retention_forever_override(store.clone())
            .await
            .expect("migration should succeed");

        let keep_forever = store
            .get_setting_with_defaults(SETTINGS_SCOPE_SYSTEM, HISTORY_KEEP_FOREVER_KEY, None)
            .await
            .expect("load keep forever")
            .expect("setting exists");
        assert_eq!(keep_forever.effective_value_json, "false");
        assert_eq!(keep_forever.value_json, None);
        assert_eq!(keep_forever.source, None);
    }

    #[tokio::test]
    async fn legacy_migration_history_override_is_preserved_when_user_has_retention_override() {
        let (_temp, store) = bootstrap_settings_store().await;
        store
            .upsert_setting_value(
                SETTINGS_SCOPE_SYSTEM,
                HISTORY_RETENTION_DAYS_KEY,
                None,
                "30",
                "ui",
                Some("user-1".to_string()),
            )
            .await
            .expect("seed explicit retention override");
        store
            .upsert_setting_value(
                SETTINGS_SCOPE_SYSTEM,
                HISTORY_KEEP_FOREVER_KEY,
                None,
                "true",
                "migration",
                None,
            )
            .await
            .expect("seed legacy migration override");

        clear_legacy_history_retention_forever_override(store.clone())
            .await
            .expect("migration should succeed");

        let keep_forever = store
            .get_setting_with_defaults(SETTINGS_SCOPE_SYSTEM, HISTORY_KEEP_FOREVER_KEY, None)
            .await
            .expect("load keep forever")
            .expect("setting exists");
        assert_eq!(keep_forever.effective_value_json, "true");
        assert_eq!(keep_forever.value_json.as_deref(), Some("true"));
        assert_eq!(keep_forever.source.as_deref(), Some("migration"));
    }
}

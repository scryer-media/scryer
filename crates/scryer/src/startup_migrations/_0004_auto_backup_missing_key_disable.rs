use std::sync::Arc;

use scryer_application::{
    AUTO_BACKUP_DISABLED_MISSING_KEY_NOTICE_KEY, AUTO_BACKUP_ENABLED_KEY, AUTO_BACKUP_KEY_KEY,
    SETTINGS_SCOPE_SYSTEM,
};
use scryer_infrastructure_configuration::settings::settings_store::SettingsStore;

fn parse_bootstrap_bool_token(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "y" | "on" => Some(true),
        "0" | "false" | "no" | "n" | "off" => Some(false),
        _ => None,
    }
}

fn parse_bootstrap_bool(value_json: &str) -> Option<bool> {
    serde_json::from_str::<bool>(value_json)
        .ok()
        .or_else(|| {
            serde_json::from_str::<String>(value_json)
                .ok()
                .as_deref()
                .and_then(parse_bootstrap_bool_token)
        })
        .or_else(|| parse_bootstrap_bool_token(value_json))
}

fn parse_bootstrap_string(value_json: &str) -> Option<String> {
    serde_json::from_str::<Option<String>>(value_json)
        .ok()
        .flatten()
        .or_else(|| serde_json::from_str::<String>(value_json).ok())
}

async fn read_bootstrap_system_bool(
    settings_store: Arc<SettingsStore>,
    key: &str,
    default: bool,
) -> Result<bool, String> {
    let Some(record) = settings_store
        .get_setting_with_defaults(SETTINGS_SCOPE_SYSTEM, key, None)
        .await
        .map_err(|error| error.to_string())?
    else {
        return Ok(default);
    };
    Ok(parse_bootstrap_bool(&record.effective_value_json).unwrap_or(default))
}

async fn read_bootstrap_system_string(
    settings_store: Arc<SettingsStore>,
    key: &str,
) -> Result<Option<String>, String> {
    let Some(record) = settings_store
        .get_setting_with_defaults(SETTINGS_SCOPE_SYSTEM, key, None)
        .await
        .map_err(|error| error.to_string())?
    else {
        return Ok(None);
    };
    Ok(parse_bootstrap_string(&record.effective_value_json))
}

async fn upsert_bootstrap_system_bool(
    settings_store: Arc<SettingsStore>,
    key: &str,
    value: bool,
) -> Result<(), String> {
    settings_store
        .upsert_setting_value(
            SETTINGS_SCOPE_SYSTEM,
            key,
            None,
            value.to_string(),
            "system",
            None,
        )
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}

pub(crate) async fn disable_auto_backups_without_key(
    settings_store: Arc<SettingsStore>,
) -> Result<(), String> {
    let enabled =
        read_bootstrap_system_bool(settings_store.clone(), AUTO_BACKUP_ENABLED_KEY, false).await?;
    if !enabled {
        return Ok(());
    }

    let key_present = read_bootstrap_system_string(settings_store.clone(), AUTO_BACKUP_KEY_KEY)
        .await?
        .is_some_and(|value| !value.trim().is_empty());
    if key_present {
        return Ok(());
    }

    upsert_bootstrap_system_bool(settings_store.clone(), AUTO_BACKUP_ENABLED_KEY, false).await?;
    upsert_bootstrap_system_bool(
        settings_store,
        AUTO_BACKUP_DISABLED_MISSING_KEY_NOTICE_KEY,
        true,
    )
    .await?;

    tracing::warn!(
        "disabled automatic backups because no automatic-backup encryption key is configured"
    );
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
    async fn disables_enabled_auto_backups_without_key_and_sets_notice() {
        let (_temp, store) = bootstrap_settings_store().await;
        store
            .upsert_setting_value(
                SETTINGS_SCOPE_SYSTEM,
                AUTO_BACKUP_ENABLED_KEY,
                None,
                "true",
                "ui",
                Some("user-1".to_string()),
            )
            .await
            .expect("enable automatic backups");

        disable_auto_backups_without_key(store.clone())
            .await
            .expect("migration should succeed");

        let enabled = read_bootstrap_system_bool(store.clone(), AUTO_BACKUP_ENABLED_KEY, true)
            .await
            .expect("read enabled");
        let notice =
            read_bootstrap_system_bool(store, AUTO_BACKUP_DISABLED_MISSING_KEY_NOTICE_KEY, false)
                .await
                .expect("read notice");
        assert!(!enabled);
        assert!(notice);
    }

    #[tokio::test]
    async fn preserves_enabled_auto_backups_with_key() {
        let (_temp, store) = bootstrap_settings_store().await;
        store
            .upsert_setting_value(
                SETTINGS_SCOPE_SYSTEM,
                AUTO_BACKUP_ENABLED_KEY,
                None,
                "true",
                "ui",
                Some("user-1".to_string()),
            )
            .await
            .expect("enable automatic backups");
        store
            .upsert_setting_value(
                SETTINGS_SCOPE_SYSTEM,
                AUTO_BACKUP_KEY_KEY,
                None,
                serde_json::to_string("backup-key").expect("serialize key"),
                "ui",
                Some("user-1".to_string()),
            )
            .await
            .expect("set automatic backup key");

        disable_auto_backups_without_key(store.clone())
            .await
            .expect("migration should succeed");

        let enabled = read_bootstrap_system_bool(store.clone(), AUTO_BACKUP_ENABLED_KEY, false)
            .await
            .expect("read enabled");
        let notice =
            read_bootstrap_system_bool(store, AUTO_BACKUP_DISABLED_MISSING_KEY_NOTICE_KEY, false)
                .await
                .expect("read notice");
        assert!(enabled);
        assert!(!notice);
    }
}

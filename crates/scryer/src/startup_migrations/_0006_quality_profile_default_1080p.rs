use std::sync::Arc;

use scryer_application::{
    BUILTIN_DEFAULT_QUALITY_PROFILE_ID, QUALITY_PROFILE_ID_KEY, QualityProfileRepository,
    SETTINGS_SCOPE_SYSTEM,
};
use scryer_infrastructure_configuration::settings::{
    quality_profile_store::QualityProfileStore, settings_store::SettingsStore,
};

use crate::settings_bootstrap::parse_quality_profile_id;

pub(crate) const QUALITY_PROFILE_DEFAULT_1080P_MIGRATION_STATE_KEY: &str =
    "quality.profile_default_1080p_migration_state";
const STATE_COMPLETED: &str = "completed";

/// The implicit global default before the built-in default became 1080p.
const LEGACY_DEFAULT_PROFILE_ID: &str = "4k";

/// True when the explicit global profile row pins the legacy `4k` default and
/// was written by Scryer itself (bootstrap normalization or the legacy settings
/// migration) rather than by a user. Such rows only exist because older
/// releases materialized the implicit default; clearing them lets the new
/// built-in default apply. A user-attributed row is a deliberate choice and is
/// preserved even when it selects `4k`.
fn is_system_written_legacy_default(
    value_json: Option<&str>,
    updated_by_user_id: Option<&str>,
) -> bool {
    let Some(value_json) = value_json else {
        return false;
    };
    if updated_by_user_id.is_some_and(|user_id| !user_id.trim().is_empty()) {
        return false;
    }
    parse_quality_profile_id(value_json).as_deref() == Some(LEGACY_DEFAULT_PROFILE_ID)
}

async fn read_state(settings_store: Arc<SettingsStore>) -> String {
    settings_store
        .get_setting_with_defaults(
            SETTINGS_SCOPE_SYSTEM,
            QUALITY_PROFILE_DEFAULT_1080P_MIGRATION_STATE_KEY,
            None,
        )
        .await
        .ok()
        .flatten()
        .and_then(|record| serde_json::from_str::<String>(&record.effective_value_json).ok())
        .unwrap_or_default()
}

async fn mark_completed(settings_store: Arc<SettingsStore>) -> Result<(), String> {
    let value_json = serde_json::to_string(STATE_COMPLETED).map_err(|error| error.to_string())?;
    settings_store
        .upsert_setting_value(
            SETTINGS_SCOPE_SYSTEM,
            QUALITY_PROFILE_DEFAULT_1080P_MIGRATION_STATE_KEY,
            None,
            value_json,
            "system",
            None,
        )
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// One-shot migration for the 4k→1080p built-in default flip: a global
/// `quality.profile_id` row that Scryer itself wrote as `4k` is deleted so the
/// install returns to a true fallback state governed by the new definition
/// default. Deletion only happens when the catalog actually carries the
/// built-in default profile — a catalog that dropped it (legacy normalization
/// materialized `4k` exactly in that state) keeps its explicit row, since
/// deleting it would leave the effective global pointing at a profile that
/// does not exist. Runs after the settings bootstrap has re-seeded the
/// definition and merged/normalized the catalog.
pub(crate) async fn clear_system_written_legacy_default_global_profile(
    settings_store: Arc<SettingsStore>,
    quality_profiles: Arc<QualityProfileStore>,
) -> Result<(), String> {
    if read_state(settings_store.clone()).await == STATE_COMPLETED {
        return Ok(());
    }

    let record = settings_store
        .get_setting_with_defaults(SETTINGS_SCOPE_SYSTEM, QUALITY_PROFILE_ID_KEY, None)
        .await
        .map_err(|error| error.to_string())?;

    if let Some(record) = record
        && is_system_written_legacy_default(
            record.value_json.as_deref(),
            record.updated_by_user_id.as_deref(),
        )
    {
        let builtin_default_available = quality_profiles
            .list_quality_profiles(SETTINGS_SCOPE_SYSTEM, None)
            .await
            .map_err(|error| error.to_string())?
            .iter()
            .any(|profile| profile.id == BUILTIN_DEFAULT_QUALITY_PROFILE_ID);
        if builtin_default_available {
            settings_store
                .delete_setting_value(SETTINGS_SCOPE_SYSTEM, QUALITY_PROFILE_ID_KEY, None)
                .await
                .map_err(|error| error.to_string())?;
            tracing::info!(
                builtin_default = BUILTIN_DEFAULT_QUALITY_PROFILE_ID,
                "cleared system-written legacy 4k global quality profile; the built-in default now applies"
            );
        } else {
            tracing::info!(
                "keeping the system-written 4k global quality profile: the catalog does not carry the built-in default"
            );
        }
    }

    mark_completed(settings_store).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_written_legacy_default_rows_are_cleared() {
        assert!(is_system_written_legacy_default(Some("\"4k\""), None));
        assert!(is_system_written_legacy_default(Some("4k"), None));
        assert!(is_system_written_legacy_default(
            Some(" \"4k\" "),
            Some("  ")
        ));
    }

    #[test]
    fn user_choices_and_other_profiles_are_preserved() {
        assert!(!is_system_written_legacy_default(
            Some("\"4k\""),
            Some("user-1")
        ));
        assert!(!is_system_written_legacy_default(Some("\"1080p\""), None));
        assert!(!is_system_written_legacy_default(Some("\"custom\""), None));
        assert!(!is_system_written_legacy_default(None, None));
    }

    async fn migration_settings_store() -> (
        tempfile::TempDir,
        Arc<SettingsStore>,
        Arc<QualityProfileStore>,
    ) {
        let temp = tempfile::tempdir().expect("tempdir");
        let db_path = temp.path().join("scryer.db");
        let services = scryer_infrastructure_datastore::SqliteServices::new_with_mode(
            db_path.to_string_lossy().to_string(),
            scryer_infrastructure_datastore::MigrationMode::Apply,
        )
        .await
        .expect("sqlite services");
        let store = Arc::new(SettingsStore::new(
            services.datastore(),
            services.encryption_key_state(),
        ));
        crate::settings_bootstrap::seed_service_setting_definitions(store.clone())
            .await
            .expect("seed setting definitions");
        let quality_profiles = Arc::new(QualityProfileStore::new(services.datastore()));
        quality_profiles
            .replace_quality_profiles(
                SETTINGS_SCOPE_SYSTEM,
                None,
                vec![
                    scryer_application::builtin_default_quality_profile(),
                    scryer_application::builtin_4k_profile(),
                ],
            )
            .await
            .expect("seed profile catalog");
        (temp, store, quality_profiles)
    }

    async fn explicit_global_value(store: &SettingsStore) -> Option<String> {
        store
            .get_setting_with_defaults(SETTINGS_SCOPE_SYSTEM, QUALITY_PROFILE_ID_KEY, None)
            .await
            .expect("read global profile")
            .expect("definition exists")
            .value_json
    }

    #[tokio::test]
    async fn migration_clears_a_system_written_4k_global_and_completes_once() {
        let (_temp, store, quality_profiles) = migration_settings_store().await;
        store
            .upsert_setting_value(
                SETTINGS_SCOPE_SYSTEM,
                QUALITY_PROFILE_ID_KEY,
                None,
                "\"4k\"".to_string(),
                "bootstrap-normalization",
                None,
            )
            .await
            .expect("seed system-written global");

        clear_system_written_legacy_default_global_profile(store.clone(), quality_profiles.clone())
            .await
            .expect("migration should succeed");

        assert_eq!(explicit_global_value(store.as_ref()).await, None);
        assert_eq!(read_state(store.clone()).await, STATE_COMPLETED);

        // A later explicit 4k choice survives re-running the completed migration.
        store
            .upsert_setting_value(
                SETTINGS_SCOPE_SYSTEM,
                QUALITY_PROFILE_ID_KEY,
                None,
                "\"4k\"".to_string(),
                "bootstrap-normalization",
                None,
            )
            .await
            .expect("re-seed global");
        clear_system_written_legacy_default_global_profile(store.clone(), quality_profiles.clone())
            .await
            .expect("migration should succeed");
        assert!(explicit_global_value(store.as_ref()).await.is_some());
    }

    #[tokio::test]
    async fn migration_leaves_scoped_rows_untouched() {
        let (_temp, store, quality_profiles) = migration_settings_store().await;
        store
            .upsert_setting_value(
                SETTINGS_SCOPE_SYSTEM,
                QUALITY_PROFILE_ID_KEY,
                Some("library-1".to_string()),
                "\"4k\"".to_string(),
                "bootstrap-normalization",
                None,
            )
            .await
            .expect("seed scoped profile row");

        clear_system_written_legacy_default_global_profile(store.clone(), quality_profiles.clone())
            .await
            .expect("migration should succeed");

        let scoped = store
            .get_setting_with_defaults(
                SETTINGS_SCOPE_SYSTEM,
                QUALITY_PROFILE_ID_KEY,
                Some("library-1".to_string()),
            )
            .await
            .expect("read scoped profile")
            .expect("definition exists");
        assert_eq!(
            scoped.value_json.as_deref(),
            Some("\"4k\""),
            "scoped rows are explicit configuration and stay untouched"
        );
        assert_eq!(read_state(store.clone()).await, STATE_COMPLETED);
    }

    #[tokio::test]
    async fn migration_keeps_the_row_when_the_catalog_lacks_the_builtin_default() {
        let (_temp, store, quality_profiles) = migration_settings_store().await;
        quality_profiles
            .replace_quality_profiles(
                SETTINGS_SCOPE_SYSTEM,
                None,
                vec![scryer_application::builtin_4k_profile()],
            )
            .await
            .expect("replace catalog without the built-in default");
        store
            .upsert_setting_value(
                SETTINGS_SCOPE_SYSTEM,
                QUALITY_PROFILE_ID_KEY,
                None,
                "\"4k\"".to_string(),
                "bootstrap-normalization",
                None,
            )
            .await
            .expect("seed system-written global");

        clear_system_written_legacy_default_global_profile(store.clone(), quality_profiles.clone())
            .await
            .expect("migration should succeed");

        assert!(
            explicit_global_value(store.as_ref()).await.is_some(),
            "deleting the row would leave the effective global dangling"
        );
        assert_eq!(read_state(store.clone()).await, STATE_COMPLETED);
    }

    #[tokio::test]
    async fn migration_preserves_a_user_attributed_4k_global() {
        let (_temp, store, quality_profiles) = migration_settings_store().await;
        store
            .upsert_setting_value(
                SETTINGS_SCOPE_SYSTEM,
                QUALITY_PROFILE_ID_KEY,
                None,
                "\"4k\"".to_string(),
                "user",
                Some("user-1".to_string()),
            )
            .await
            .expect("seed user-chosen global");

        clear_system_written_legacy_default_global_profile(store.clone(), quality_profiles.clone())
            .await
            .expect("migration should succeed");

        assert!(
            explicit_global_value(store.as_ref()).await.is_some(),
            "a deliberate 4k choice is preserved"
        );
        assert_eq!(read_state(store.clone()).await, STATE_COMPLETED);
    }
}

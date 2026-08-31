use std::future::Future;
use std::sync::Arc;

use scryer_application::{AppResult, AppUseCase, SETTINGS_SCOPE_SYSTEM};
use scryer_infrastructure_configuration::settings::settings_store::SettingsStore;

use super::versioning::{MajorMinor, parse_major_minor};

pub(crate) const TITLE_CREDITS_REHYDRATION_018_STATE_KEY: &str =
    "catalog.title_credits_rehydration_018_state";
const STATE_NONE: &str = "none";
const STATE_PENDING: &str = "pending";
const STATE_COMPLETED: &str = "completed";

fn is_018_or_later(version: &str) -> bool {
    parse_major_minor(version).is_some_and(|version| version >= MajorMinor::new(0, 18))
}

fn should_attempt_title_credits_rehydration(current_version: &str, migration_state: &str) -> bool {
    migration_state != STATE_COMPLETED && is_018_or_later(current_version)
}

async fn read_state(settings_store: Arc<SettingsStore>) -> String {
    settings_store
        .get_setting_with_defaults(
            SETTINGS_SCOPE_SYSTEM,
            TITLE_CREDITS_REHYDRATION_018_STATE_KEY,
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
            TITLE_CREDITS_REHYDRATION_018_STATE_KEY,
            None,
            value_json,
            "system",
            None,
        )
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}

pub(crate) async fn rehydrate_title_credits_for_018_upgrade(
    app_use_case: &AppUseCase,
    settings_store: Arc<SettingsStore>,
    current_version: &str,
) -> Result<(), String> {
    start_title_credits_rehydration(settings_store, current_version, move || async move {
        app_use_case.hydrate_all_titles_for_current_language().await
    })
    .await
}

async fn start_title_credits_rehydration<F, Fut>(
    settings_store: Arc<SettingsStore>,
    current_version: &str,
    rehydrate: F,
) -> Result<(), String>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = AppResult<u32>>,
{
    let state = read_state(settings_store.clone()).await;
    if !should_attempt_title_credits_rehydration(current_version, &state) {
        return Ok(());
    }

    set_state(settings_store.clone(), STATE_PENDING).await?;

    let titles_rehydrated = rehydrate().await.map_err(|error| error.to_string())?;
    set_state(settings_store, STATE_COMPLETED).await?;
    tracing::info!(
        titles_rehydrated,
        "0.18 title credits rehydration migration completed"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use scryer_application::AppError;
    use scryer_infrastructure_datastore::{MigrationMode, SqliteServices};

    use super::*;
    use crate::settings_bootstrap::seed_service_setting_definitions;

    #[test]
    fn title_credits_rehydration_runs_once_on_any_018_or_later_host() {
        assert!(should_attempt_title_credits_rehydration(
            "0.18.0", STATE_NONE,
        ));
        assert!(should_attempt_title_credits_rehydration(
            "0.18.16-rc.1",
            STATE_NONE,
        ));
        assert!(should_attempt_title_credits_rehydration(
            "0.19.0", STATE_NONE,
        ));
        assert!(should_attempt_title_credits_rehydration(
            "0.18.0",
            STATE_PENDING,
        ));
        assert!(!should_attempt_title_credits_rehydration(
            "0.17.9",
            STATE_PENDING,
        ));
        assert!(!should_attempt_title_credits_rehydration(
            "0.17.9", STATE_NONE,
        ));
        assert!(!should_attempt_title_credits_rehydration(
            "0.18.0",
            STATE_COMPLETED,
        ));
    }

    async fn migration_settings_store() -> (tempfile::TempDir, Arc<SettingsStore>) {
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
    async fn successful_rehydration_marks_the_migration_completed_once() {
        let (_temp, store) = migration_settings_store().await;
        assert_eq!(read_state(store.clone()).await, STATE_NONE);

        static RUNS: AtomicUsize = AtomicUsize::new(0);
        start_title_credits_rehydration(store.clone(), "0.18.16", || async {
            RUNS.fetch_add(1, Ordering::SeqCst);
            Ok(7)
        })
        .await
        .expect("migration should succeed on a 0.18 host with no recorded state");

        assert_eq!(read_state(store.clone()).await, STATE_COMPLETED);
        assert_eq!(RUNS.load(Ordering::SeqCst), 1);

        start_title_credits_rehydration(store.clone(), "0.18.16", || async {
            RUNS.fetch_add(1, Ordering::SeqCst);
            Ok(7)
        })
        .await
        .expect("completed migration should be a no-op");
        assert_eq!(RUNS.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn failed_rehydration_leaves_the_migration_pending_for_the_next_startup() {
        let (_temp, store) = migration_settings_store().await;

        let result = start_title_credits_rehydration(store.clone(), "0.18.16", || async {
            Err(AppError::Repository("smg unavailable".to_string()))
        })
        .await;
        assert!(result.is_err());

        assert_eq!(read_state(store.clone()).await, STATE_PENDING);

        start_title_credits_rehydration(store.clone(), "0.18.16", || async { Ok(3) })
            .await
            .expect("a pending migration should retry successfully");

        assert_eq!(read_state(store).await, STATE_COMPLETED);
    }

    #[tokio::test]
    async fn pre_018_hosts_never_start_the_migration() {
        let (_temp, store) = migration_settings_store().await;

        start_title_credits_rehydration(store.clone(), "0.17.9", || async { Ok(0) })
            .await
            .expect("inapplicable migration should be a no-op");
        assert_eq!(read_state(store).await, STATE_NONE);
    }
}

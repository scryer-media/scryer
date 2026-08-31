use std::collections::HashSet;
use std::future::Future;
use std::sync::Arc;
use std::time::Instant;

use chrono::Utc;
use scryer_application::{AppUseCase, IndexerConfigRepository, SETTINGS_SCOPE_SYSTEM};
use scryer_infrastructure_configuration::settings::{
    quality_profile_store::QualityProfileStore, settings_store::SettingsStore,
};
use scryer_infrastructure_runtime::DatastoreCustomizationStore;
use scryer_infrastructure_sql::runtime::{SqlArg, SqlRuntime, StoreDatastore};

use super::{
    _0001_legacy_history_retention_forever_override as migration_0001,
    _0002_enhanced_subsync_plugin_016 as migration_0002,
    _0003_title_image_artwork_url_refresh as migration_0003,
    _0004_auto_backup_missing_key_disable as migration_0004,
    _0005_title_metadata_rehydration_017 as migration_0005,
    _0006_quality_profile_default_1080p as migration_0006,
    _0007_emby_plugin_compatibility as migration_0007,
    _0008_title_credits_rehydration_018 as migration_0008,
    _0010_download_client_remove_failed_default as migration_0010,
    _0011_long_tail_reconverge_default as migration_0011,
    _0012_legacy_newznab_wrappers_01822 as migration_0012,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MigrationPhase {
    Early,
    ApplicationReady,
}

#[derive(Clone, Copy, Debug)]
struct MigrationSpec {
    id: &'static str,
    description: &'static str,
    phase: MigrationPhase,
    legacy_state_key: Option<&'static str>,
}

const MIGRATIONS: &[MigrationSpec] = &[
    MigrationSpec {
        id: "0001_legacy_history_retention_forever_override",
        description: "clear the legacy history retention forever override",
        phase: MigrationPhase::Early,
        legacy_state_key: None,
    },
    MigrationSpec {
        id: "0002_enhanced_subsync_plugin_016",
        description: "migrate enhanced subtitle sync for the 0.16 upgrade",
        phase: MigrationPhase::ApplicationReady,
        legacy_state_key: Some(migration_0002::ENHANCED_SUBSYNC_016_MIGRATION_STATE_KEY),
    },
    MigrationSpec {
        id: "0003_title_image_artwork_url_refresh",
        description: "refresh title image artwork URLs for the 0.16 upgrade",
        phase: MigrationPhase::ApplicationReady,
        legacy_state_key: Some(migration_0003::TITLE_IMAGE_ARTWORK_URL_REFRESH_STATE_KEY),
    },
    MigrationSpec {
        id: "0004_auto_backup_missing_key_disable",
        description: "disable automatic backups missing an encryption key",
        phase: MigrationPhase::ApplicationReady,
        legacy_state_key: None,
    },
    MigrationSpec {
        id: "0005_title_metadata_rehydration_017",
        description: "rehydrate title metadata for the 0.17 upgrade",
        phase: MigrationPhase::ApplicationReady,
        legacy_state_key: Some(migration_0005::TITLE_METADATA_REHYDRATION_017_STATE_KEY),
    },
    MigrationSpec {
        id: "0006_quality_profile_default_1080p",
        description: "clear the system-written legacy quality profile default",
        phase: MigrationPhase::ApplicationReady,
        legacy_state_key: Some(migration_0006::QUALITY_PROFILE_DEFAULT_1080P_MIGRATION_STATE_KEY),
    },
    MigrationSpec {
        id: "0007_emby_plugin_compatibility",
        description: "migrate the legacy Emby plugin installation",
        phase: MigrationPhase::ApplicationReady,
        legacy_state_key: Some(migration_0007::MIGRATION_STATE_KEY),
    },
    MigrationSpec {
        id: "0008_title_credits_rehydration_018",
        description: "rehydrate title credits for the 0.18 upgrade",
        phase: MigrationPhase::ApplicationReady,
        legacy_state_key: Some(migration_0008::TITLE_CREDITS_REHYDRATION_018_STATE_KEY),
    },
    MigrationSpec {
        id: "0010_download_client_remove_failed_default",
        description: "flip the legacy download-client remove-failed default",
        phase: MigrationPhase::ApplicationReady,
        legacy_state_key: Some(
            migration_0010::DOWNLOAD_CLIENT_REMOVE_FAILED_DEFAULT_FLIPPED_0018_STATE_KEY,
        ),
    },
    MigrationSpec {
        id: "0011_long_tail_reconverge_default",
        description: "clear the legacy long-tail reconvergence default",
        phase: MigrationPhase::ApplicationReady,
        legacy_state_key: Some(migration_0011::MIGRATION_STATE_KEY),
    },
    MigrationSpec {
        id: "0012_legacy_newznab_wrappers_01822",
        description: "migrate legacy Newznab wrapper plugins to provider profiles",
        phase: MigrationPhase::Early,
        legacy_state_key: None,
    },
];

#[derive(Clone)]
struct MigrationLedger {
    datastore: StoreDatastore,
}

impl MigrationLedger {
    async fn applied_ids(&self) -> Result<HashSet<String>, String> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT migration_id FROM application_migrations ORDER BY migration_id",
            &[],
        )
        .await
        .map_err(|error| error.to_string())?;
        rows.into_iter()
            .map(|row| row.text("migration_id").map_err(|error| error.to_string()))
            .collect()
    }

    async fn record(&self, spec: MigrationSpec, elapsed_ms: i64) -> Result<(), String> {
        SqlRuntime::execute_write(
            &self.datastore,
            "record_application_migration",
            "INSERT INTO application_migrations
             (migration_id, description, applied_at, execution_time_ms)
             VALUES ({}, {}, {}, {})
             ON CONFLICT (migration_id) DO NOTHING",
            vec![
                SqlArg::Text(spec.id.to_string()),
                SqlArg::Text(spec.description.to_string()),
                SqlArg::Timestamp(Utc::now()),
                SqlArg::I64(elapsed_ms),
            ],
        )
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
    }
}

pub(crate) struct ApplicationMigrator {
    ledger: MigrationLedger,
    applied: HashSet<String>,
    settings: Arc<SettingsStore>,
}

impl ApplicationMigrator {
    pub(crate) async fn load(
        datastore: StoreDatastore,
        settings: Arc<SettingsStore>,
    ) -> Result<Self, String> {
        validate_registry(MIGRATIONS)?;
        let ledger = MigrationLedger { datastore };
        let applied = ledger.applied_ids().await?;
        let mut migrator = Self {
            ledger,
            applied,
            settings,
        };
        migrator.adopt_legacy_completion_markers().await?;
        Ok(migrator)
    }

    async fn adopt_legacy_completion_markers(&mut self) -> Result<(), String> {
        for spec in MIGRATIONS {
            if self.applied.contains(spec.id) {
                continue;
            }
            let Some(key) = spec.legacy_state_key else {
                continue;
            };
            let state = self
                .settings
                .get_setting_with_defaults(SETTINGS_SCOPE_SYSTEM, key, None)
                .await
                .map_err(|error| {
                    format!(
                        "failed to read legacy state for application migration {}: {error}",
                        spec.id
                    )
                })?
                .and_then(|record| {
                    serde_json::from_str::<String>(&record.effective_value_json).ok()
                });
            if state.as_deref() == Some("completed") {
                self.ledger.record(*spec, 0).await?;
                self.applied.insert(spec.id.to_string());
                tracing::info!(
                    migration_id = spec.id,
                    "adopted completed legacy application migration"
                );
            }
        }
        Ok(())
    }

    pub(crate) async fn run_early(
        &mut self,
        indexer_configs: Arc<dyn IndexerConfigRepository>,
        plugin_installations: &DatastoreCustomizationStore,
    ) -> Result<(), String> {
        for spec in MIGRATIONS
            .iter()
            .copied()
            .filter(|spec| spec.phase == MigrationPhase::Early)
        {
            if self.applied.contains(spec.id) {
                continue;
            }
            match spec.id {
                "0001_legacy_history_retention_forever_override" => {
                    let settings = self.settings.clone();
                    self.run_retryable(spec, async move {
                        migration_0001::clear_legacy_history_retention_forever_override(settings)
                            .await
                    })
                    .await;
                }
                "0012_legacy_newznab_wrappers_01822" => {
                    let started = Instant::now();
                    let report =
                        migration_0012::migrate(indexer_configs.clone(), plugin_installations)
                            .await?;
                    self.record_success(spec, started).await?;
                    if report.indexer_configs > 0 || report.plugin_installations > 0 {
                        tracing::info!(
                            indexer_configs = report.indexer_configs,
                            plugin_installations = report.plugin_installations,
                            "migrated legacy NZBGeek and DogNZB plugins to generic Newznab profiles"
                        );
                    }
                    if report.skipped_indexer_configs > 0 {
                        tracing::warn!(
                            skipped_indexer_configs = report.skipped_indexer_configs,
                            "legacy Newznab wrapper configurations were left unconverted; those indexers need operator attention"
                        );
                    }
                }
                _ => unreachable!("early migration registry and dispatcher must agree"),
            }
        }
        Ok(())
    }

    pub(crate) async fn run_application_ready(
        &mut self,
        app: &AppUseCase,
        quality_profiles: Arc<QualityProfileStore>,
        previous_version: Option<&str>,
        current_version: &str,
    ) {
        for spec in MIGRATIONS
            .iter()
            .copied()
            .filter(|spec| spec.phase == MigrationPhase::ApplicationReady)
        {
            if self.applied.contains(spec.id) {
                continue;
            }
            match spec.id {
                "0002_enhanced_subsync_plugin_016" => {
                    self.run_retryable(
                        spec,
                        migration_0002::migrate_enhanced_subsync_plugin_for_016_upgrade(
                            app,
                            self.settings.clone(),
                            previous_version,
                            current_version,
                        ),
                    )
                    .await;
                }
                "0003_title_image_artwork_url_refresh" => {
                    self.run_retryable(
                        spec,
                        migration_0003::refresh_title_image_artwork_urls_for_upgrade(
                            app,
                            self.settings.clone(),
                            previous_version,
                            current_version,
                        ),
                    )
                    .await;
                }
                "0004_auto_backup_missing_key_disable" => {
                    self.run_retryable(
                        spec,
                        migration_0004::disable_auto_backups_without_key(self.settings.clone()),
                    )
                    .await;
                }
                "0005_title_metadata_rehydration_017" => {
                    let app = app.clone();
                    let settings = self.settings.clone();
                    let current_version = current_version.to_string();
                    std::mem::drop(self.spawn_retryable(spec, async move {
                        migration_0005::rehydrate_title_metadata_for_017_upgrade(
                            &app,
                            settings,
                            &current_version,
                        )
                        .await
                    }));
                }
                "0006_quality_profile_default_1080p" => {
                    self.run_retryable(
                        spec,
                        migration_0006::clear_system_written_legacy_default_global_profile(
                            self.settings.clone(),
                            quality_profiles.clone(),
                        ),
                    )
                    .await;
                }
                "0007_emby_plugin_compatibility" => {
                    self.run_retryable(
                        spec,
                        migration_0007::migrate_emby_plugin_compatibility(
                            app,
                            self.settings.clone(),
                        ),
                    )
                    .await;
                }
                "0008_title_credits_rehydration_018" => {
                    let app = app.clone();
                    let settings = self.settings.clone();
                    let current_version = current_version.to_string();
                    std::mem::drop(self.spawn_retryable(spec, async move {
                        migration_0008::rehydrate_title_credits_for_018_upgrade(
                            &app,
                            settings,
                            &current_version,
                        )
                        .await
                    }));
                }
                "0010_download_client_remove_failed_default" => {
                    self.run_retryable(
                        spec,
                        migration_0010::flip_download_client_remove_failed_default(
                            app,
                            self.settings.clone(),
                        ),
                    )
                    .await;
                }
                "0011_long_tail_reconverge_default" => {
                    self.run_retryable(spec, migration_0011::migrate(self.settings.clone()))
                        .await;
                }
                _ => unreachable!("application-ready migration registry and dispatcher must agree"),
            }
        }
    }

    async fn run_retryable<F>(&mut self, spec: MigrationSpec, migration: F)
    where
        F: Future<Output = Result<(), String>>,
    {
        let started = Instant::now();
        match migration.await {
            Ok(()) => {
                if let Err(error) = self.record_success(spec, started).await {
                    tracing::warn!(migration_id = spec.id, error = %error, "application migration completed but success could not be recorded");
                }
            }
            Err(error) => {
                tracing::warn!(migration_id = spec.id, error = %error, "application migration deferred until the next startup");
            }
        }
    }

    fn spawn_retryable<F>(&self, spec: MigrationSpec, migration: F) -> tokio::task::JoinHandle<()>
    where
        F: Future<Output = Result<(), String>> + Send + 'static,
    {
        let ledger = self.ledger.clone();
        tokio::spawn(async move {
            let started = Instant::now();
            match migration.await {
                Ok(()) => {
                    let elapsed_ms = elapsed_ms(started);
                    if let Err(error) = ledger.record(spec, elapsed_ms).await {
                        tracing::warn!(migration_id = spec.id, error = %error, "background application migration completed but success could not be recorded");
                    } else {
                        tracing::info!(
                            migration_id = spec.id,
                            "background application migration completed"
                        );
                    }
                }
                Err(error) => {
                    tracing::warn!(migration_id = spec.id, error = %error, "background application migration deferred until the next startup");
                }
            }
        })
    }

    async fn record_success(
        &mut self,
        spec: MigrationSpec,
        started: Instant,
    ) -> Result<(), String> {
        self.ledger.record(spec, elapsed_ms(started)).await?;
        self.applied.insert(spec.id.to_string());
        tracing::info!(migration_id = spec.id, "application migration completed");
        Ok(())
    }
}

fn elapsed_ms(started: Instant) -> i64 {
    started.elapsed().as_millis().min(i64::MAX as u128) as i64
}

fn validate_registry(registry: &[MigrationSpec]) -> Result<(), String> {
    let mut ids = HashSet::new();
    for spec in registry {
        if !ids.insert(spec.id) {
            return Err(format!("duplicate application migration id `{}`", spec.id));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use scryer_infrastructure_configuration::settings::settings_store::SettingsStore;
    use scryer_infrastructure_datastore::{MigrationMode, SqliteServices};

    use super::*;
    use crate::settings_bootstrap::seed_service_setting_definitions;

    async fn test_migrator() -> (tempfile::TempDir, Arc<SettingsStore>, ApplicationMigrator) {
        let temp = tempfile::tempdir().expect("tempdir");
        let services = SqliteServices::new_with_mode(
            temp.path().join("scryer.db").to_string_lossy().to_string(),
            MigrationMode::Apply,
        )
        .await
        .expect("sqlite services");
        let settings = Arc::new(SettingsStore::new(
            services.datastore(),
            services.encryption_key_state(),
        ));
        seed_service_setting_definitions(settings.clone())
            .await
            .expect("seed setting definitions");
        let migrator = ApplicationMigrator::load(services.datastore(), settings.clone())
            .await
            .expect("load application migrator");
        (temp, settings, migrator)
    }

    #[test]
    fn migration_registry_is_ordered_and_unique() {
        validate_registry(MIGRATIONS).expect("registry should be valid");
        assert!(MIGRATIONS.windows(2).all(|pair| pair[0].id < pair[1].id));
    }

    #[test]
    fn duplicate_migration_ids_are_rejected() {
        let duplicate = [MIGRATIONS[0], MIGRATIONS[0]];
        assert!(validate_registry(&duplicate).is_err());
    }

    #[tokio::test]
    async fn only_successful_migrations_are_recorded_and_reloaded() {
        let (_temp, settings, mut migrator) = test_migrator().await;
        let spec = MIGRATIONS[0];

        migrator
            .run_retryable(spec, async { Err("synthetic failure".to_string()) })
            .await;
        assert!(
            !migrator
                .ledger
                .applied_ids()
                .await
                .unwrap()
                .contains(spec.id)
        );

        migrator.run_retryable(spec, async { Ok(()) }).await;
        assert!(
            migrator
                .ledger
                .applied_ids()
                .await
                .unwrap()
                .contains(spec.id)
        );

        let restarted = ApplicationMigrator::load(migrator.ledger.datastore.clone(), settings)
            .await
            .expect("reload application migrator");
        assert!(restarted.applied.contains(spec.id));
    }

    #[tokio::test]
    async fn completed_legacy_marker_is_adopted_without_dispatch() {
        let (temp, settings, migrator) = test_migrator().await;
        drop(migrator);
        settings
            .upsert_setting_value(
                SETTINGS_SCOPE_SYSTEM,
                migration_0011::MIGRATION_STATE_KEY,
                None,
                serde_json::to_string("completed").unwrap(),
                "system",
                None,
            )
            .await
            .expect("write legacy completion marker");

        let services = SqliteServices::new_with_mode(
            temp.path().join("scryer.db").to_string_lossy().to_string(),
            MigrationMode::Apply,
        )
        .await
        .expect("reopen sqlite services");
        let restarted = ApplicationMigrator::load(services.datastore(), settings)
            .await
            .expect("load application migrator");
        assert!(
            restarted
                .applied
                .contains("0011_long_tail_reconverge_default")
        );
    }

    #[tokio::test]
    async fn background_migration_is_recorded_only_after_completion() {
        let (_temp, _settings, migrator) = test_migrator().await;
        let spec = MIGRATIONS[0];
        let (release, wait) = tokio::sync::oneshot::channel();
        let handle = migrator.spawn_retryable(spec, async move {
            wait.await.map_err(|error| error.to_string())?;
            Ok(())
        });

        tokio::task::yield_now().await;
        assert!(
            !migrator
                .ledger
                .applied_ids()
                .await
                .unwrap()
                .contains(spec.id)
        );
        release.send(()).expect("release migration");
        handle.await.expect("background migration task");
        assert!(
            migrator
                .ledger
                .applied_ids()
                .await
                .unwrap()
                .contains(spec.id)
        );
    }
}

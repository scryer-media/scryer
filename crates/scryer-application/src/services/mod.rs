use super::*;
use crate::ports::IndexerCapsSnapshotRefresher;
use scryer_runtime_info::{BinaryClass, BinaryLane};
use std::io::{Read, Write};

mod app_use_case;
mod builder;
mod bundles;
mod external_import_warmup;
mod guards;
mod plugin_install;
mod runtime;

pub use app_use_case::*;
pub use builder::*;
pub use bundles::*;
pub use external_import_warmup::*;
pub use guards::*;
pub use plugin_install::*;
pub use runtime::*;

#[cfg(test)]
use app_use_case::{
    RuntimePerformanceProbe, classify_config_io_elapsed, classify_cpu_elapsed,
    initialize_runtime_performance_snapshot, probe_config_io_performance,
};
use builder::AppServicesBuildConfiguration;
use runtime::normalize_supported_plugin_required_features;

pub(crate) use runtime::{
    CachedWantedProjection, CompletedDownloadAdmission, DownloadQueueReadModel,
    ReleaseCandidatePasswordTicket, download_observation_is_admitted,
    normalize_download_client_category,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::null_repositories::test_nulls::{
        NullDownloadClient, NullDownloadClientConfigRepository, NullIndexerClient,
        NullQualityProfileRepository, NullReleaseAttemptRepository, NullShowRepository,
        NullTitleRepository, NullUserRepository,
    };
    use async_trait::async_trait;
    use scryer_domain::IndexerConfig;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::tempdir;

    struct TestIndexerConfigRepository;

    #[async_trait]
    impl IndexerConfigRepository for TestIndexerConfigRepository {
        async fn list(&self, _provider_type: Option<String>) -> AppResult<Vec<IndexerConfig>> {
            Ok(Vec::new())
        }

        async fn get_by_id(&self, _id: &str) -> AppResult<Option<IndexerConfig>> {
            Ok(None)
        }

        async fn create(&self, config: IndexerConfig) -> AppResult<IndexerConfig> {
            Ok(config)
        }

        async fn touch_last_error(&self, _provider_type: &str) -> AppResult<()> {
            Ok(())
        }

        async fn update(&self, _update: crate::IndexerConfigUpdate) -> AppResult<IndexerConfig> {
            Err(AppError::Repository("not configured".into()))
        }

        async fn delete(&self, _id: &str) -> AppResult<()> {
            Ok(())
        }
    }

    fn test_builder() -> AppServicesBuilder {
        AppServices::builder(
            Arc::new(NullTitleRepository),
            Arc::new(NullShowRepository),
            Arc::new(NullUserRepository),
            Arc::new(TestIndexerConfigRepository),
            Arc::new(NullIndexerClient),
            Arc::new(NullDownloadClient),
            Arc::new(NullDownloadClientConfigRepository),
            Arc::new(NullReleaseAttemptRepository),
            Arc::new(NullSettingsRepository),
            Arc::new(NullQualityProfileRepository),
            String::new(),
        )
    }

    #[test]
    #[should_panic(expected = "AppServicesBuilder missing required runtime services")]
    fn build_requires_explicit_runtime_dependencies() {
        let _ = test_builder().build();
    }

    #[test]
    fn build_partial_for_tests_allows_partial_test_assemblies() {
        let _ = test_builder().build_partial_for_tests();
    }

    #[test]
    fn runtime_build_identity_defaults_to_portable() {
        let runtime = AppRuntimeState::default();
        assert_eq!(runtime.environment.build_lane, BinaryLane::Portable);
        assert_eq!(runtime.environment.build_class, BinaryClass::Portable);
        assert!(
            runtime
                .environment
                .supported_plugin_required_features
                .is_empty()
        );
    }

    #[test]
    fn runtime_environment_builder_sets_build_identity() {
        let assembly = test_builder()
            .with_runtime_environment(
                BinaryLane::Haswell,
                "/tmp/scryer-config",
                Vec::<String>::new(),
            )
            .build_partial_for_tests();
        assert_eq!(assembly.runtime.environment.build_lane, BinaryLane::Haswell);
        assert_eq!(
            assembly.runtime.environment.build_class,
            BinaryClass::Optimized
        );
        assert_eq!(
            assembly.runtime.environment.config_dir.as_ref(),
            &PathBuf::from("/tmp/scryer-config")
        );
    }

    #[test]
    fn runtime_environment_builder_sets_supported_plugin_required_features() {
        let assembly = test_builder()
            .with_runtime_environment(
                BinaryLane::Portable,
                "/tmp/scryer-config",
                ["simd128", " relaxed-simd ", ""],
            )
            .build_partial_for_tests();
        assert_eq!(
            assembly
                .runtime
                .environment
                .supported_plugin_required_features
                .as_ref(),
            &HashSet::from(["simd128".to_string(), "relaxed-simd".to_string()])
        );
    }

    #[tokio::test]
    async fn runtime_performance_initializer_shares_one_probe_run() {
        let cell = Arc::new(OnceCell::new());
        let config_dir = Arc::new(PathBuf::from("."));
        let probe_runs = Arc::new(AtomicUsize::new(0));
        let probe: RuntimePerformanceProbe = Arc::new({
            let probe_runs = probe_runs.clone();
            move |_path: PathBuf| {
                probe_runs.fetch_add(1, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(50));
                RuntimePerformanceSnapshot {
                    cpu_class: RuntimePerformanceClass::Fast,
                    config_io_class: RuntimePerformanceClass::Slow,
                    cpu_probe_elapsed_ms: Some(50),
                    config_io_probe_elapsed_ms: Some(5),
                }
            }
        });

        let left = {
            let cell = cell.clone();
            let config_dir = config_dir.clone();
            let probe = probe.clone();
            tokio::spawn(async move {
                initialize_runtime_performance_snapshot(cell.as_ref(), config_dir, probe).await
            })
        };
        let right = {
            let cell = cell.clone();
            let config_dir = config_dir.clone();
            let probe = probe.clone();
            tokio::spawn(async move {
                initialize_runtime_performance_snapshot(cell.as_ref(), config_dir, probe).await
            })
        };

        let first = left.await.expect("left probe");
        let second = right.await.expect("right probe");
        assert_eq!(probe_runs.load(Ordering::SeqCst), 1);
        assert_eq!(first, second);

        let start = std::time::Instant::now();
        let cached =
            initialize_runtime_performance_snapshot(cell.as_ref(), config_dir, probe).await;
        assert_eq!(cached, first);
        assert!(start.elapsed() < std::time::Duration::from_millis(20));
    }

    #[test]
    fn cpu_elapsed_threshold_classification_is_stable() {
        assert_eq!(
            classify_cpu_elapsed(std::time::Duration::from_millis(125)),
            RuntimePerformanceClass::Fast
        );
        assert_eq!(
            classify_cpu_elapsed(std::time::Duration::from_millis(126)),
            RuntimePerformanceClass::Slow
        );
    }

    #[test]
    fn config_io_elapsed_threshold_classification_is_stable() {
        assert_eq!(
            classify_config_io_elapsed(std::time::Duration::from_millis(200)),
            RuntimePerformanceClass::Fast
        );
        assert_eq!(
            classify_config_io_elapsed(std::time::Duration::from_millis(201)),
            RuntimePerformanceClass::Slow
        );
    }

    #[test]
    fn config_io_probe_creates_missing_directory_before_measuring() {
        let temp = tempdir().expect("tempdir");
        let missing = temp.path().join("missing");
        let (class, elapsed_ms) = probe_config_io_performance(&missing);
        assert!(matches!(
            class,
            RuntimePerformanceClass::Slow | RuntimePerformanceClass::Fast
        ));
        assert!(missing.is_dir());
        assert!(elapsed_ms.is_some());
    }

    #[tokio::test]
    async fn external_import_warmup_begin_creates_new_session_after_completion() {
        let orchestrator = ExternalImportMonitorWarmupOrchestrator::default();
        let first = orchestrator
            .begin(
                "user-1",
                "fingerprint-1",
                ExternalImportMonitorWarmupProgressSnapshot::new("session-1".into()),
            )
            .await;
        assert!(first.created);
        let _subscription = orchestrator
            .subscribe("user-1", &first.snapshot.session_id)
            .await
            .expect("subscribe to first session");

        let mut completed = first.snapshot.clone();
        completed.status = ExternalImportMonitorWarmupStatus::Completed;
        assert!(
            orchestrator
                .update(&completed.session_id, completed.clone())
                .await
        );

        let second = orchestrator
            .begin(
                "user-1",
                "fingerprint-1",
                ExternalImportMonitorWarmupProgressSnapshot::new("session-2".into()),
            )
            .await;

        assert!(second.created);
        assert_ne!(second.snapshot.session_id, first.snapshot.session_id);
    }

    #[tokio::test]
    async fn external_import_prowlarr_warmup_deduplicates_per_actor_and_isolates_results() {
        let orchestrator = ExternalImportMonitorWarmupOrchestrator::default();
        let first = orchestrator
            .begin(
                "user-1",
                "prowlarr-source=http://prowlarr|key",
                ExternalImportMonitorWarmupProgressSnapshot::new("session-1".into()),
            )
            .await;
        let reused = orchestrator
            .begin(
                "user-1",
                "prowlarr-source=http://prowlarr|key",
                ExternalImportMonitorWarmupProgressSnapshot::new("session-2".into()),
            )
            .await;
        let other_actor = orchestrator
            .begin(
                "user-2",
                "prowlarr-source=http://prowlarr|key",
                ExternalImportMonitorWarmupProgressSnapshot::new("session-3".into()),
            )
            .await;

        assert!(first.created);
        assert!(!reused.created);
        assert_eq!(reused.snapshot.session_id, first.snapshot.session_id);
        assert!(other_actor.created);
        assert_ne!(other_actor.snapshot.session_id, first.snapshot.session_id);

        assert!(
            orchestrator
                .set_prowlarr_result(
                    &first.snapshot.session_id,
                    ExternalImportProwlarrWarmupResult {
                        base_url: "http://prowlarr".into(),
                        api_key: "key".into(),
                        version: Some("2.0.0".into()),
                        plan: crate::IndexerSyncPlan {
                            children: Vec::new(),
                        },
                    },
                )
                .await
        );
        assert!(
            orchestrator
                .prowlarr_result("user-1", &first.snapshot.session_id)
                .await
                .is_some()
        );
        assert!(
            orchestrator
                .prowlarr_result("user-2", &first.snapshot.session_id)
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn external_import_warmup_prune_only_removes_import_source_sessions() {
        let orchestrator = ExternalImportMonitorWarmupOrchestrator::default();
        let source = orchestrator
            .begin(
                "user-1",
                "arr-source=sonarr|http://sonarr|key",
                ExternalImportMonitorWarmupProgressSnapshot::new("source-session".into()),
            )
            .await;
        let prowlarr_source = orchestrator
            .begin(
                "user-1",
                "prowlarr-source=http://prowlarr|key",
                ExternalImportMonitorWarmupProgressSnapshot::new("prowlarr-session".into()),
            )
            .await;
        let apply = orchestrator
            .begin(
                "user-1",
                crate::EXTERNAL_IMPORT_MONITOR_APPLY_SESSION_ID,
                ExternalImportMonitorWarmupProgressSnapshot::new(
                    crate::EXTERNAL_IMPORT_MONITOR_APPLY_SESSION_ID.to_string(),
                ),
            )
            .await;
        let old_updated_at = (Utc::now() - chrono::Duration::hours(3)).to_rfc3339();

        for snapshot in [&source.snapshot, &prowlarr_source.snapshot, &apply.snapshot] {
            let mut completed = snapshot.clone();
            completed.status = ExternalImportMonitorWarmupStatus::Completed;
            completed.updated_at = old_updated_at.clone();
            let session_id = completed.session_id.clone();
            assert!(orchestrator.update(&session_id, completed).await);
        }

        let removed = orchestrator
            .prune_terminal_older_than(chrono::Duration::hours(2))
            .await;

        assert_eq!(removed.len(), 2);
        assert!(removed.contains(&"source-session".to_string()));
        assert!(removed.contains(&"prowlarr-session".to_string()));
        assert!(
            orchestrator
                .snapshot("user-1", crate::EXTERNAL_IMPORT_MONITOR_APPLY_SESSION_ID)
                .await
                .is_some(),
            "apply session should not be pruned as a source session"
        );
    }

    #[tokio::test]
    async fn external_import_warmup_update_persists_without_active_subscribers() {
        let orchestrator = ExternalImportMonitorWarmupOrchestrator::default();
        let begin = orchestrator
            .begin(
                "user-1",
                "fingerprint-1",
                ExternalImportMonitorWarmupProgressSnapshot::new("session-1".into()),
            )
            .await;
        assert!(begin.created);

        let mut running = begin.snapshot.clone();
        running.status = ExternalImportMonitorWarmupStatus::Running;
        running.phase = ExternalImportMonitorWarmupPhase::LoadingSeries;
        running.series_total_known = true;
        running.series_progress.total = 42;
        running.series_progress.completed = 17;

        assert!(
            orchestrator
                .update(&running.session_id, running.clone())
                .await
        );

        let snapshot = orchestrator
            .snapshot("user-1", &running.session_id)
            .await
            .expect("stored snapshot");

        assert_eq!(snapshot.status, ExternalImportMonitorWarmupStatus::Running);
        assert_eq!(
            snapshot.phase,
            ExternalImportMonitorWarmupPhase::LoadingSeries
        );
        assert!(snapshot.series_total_known);
        assert_eq!(snapshot.series_progress.total, 42);
        assert_eq!(snapshot.series_progress.completed, 17);
    }

    #[tokio::test]
    async fn backup_execution_guards_allow_cross_trigger_overlap_but_block_same_trigger() {
        let guards = BackupExecutionGuardTable::default();

        let manual_guard = guards
            .try_acquire("manual")
            .await
            .expect("manual guard should acquire");
        let auto_guard = guards
            .try_acquire("auto")
            .await
            .expect("auto guard should acquire while manual is running");

        assert!(
            guards.try_acquire("manual").await.is_none(),
            "same-trigger manual backup should be blocked",
        );
        assert!(
            guards.try_acquire("auto").await.is_none(),
            "same-trigger automatic backup should be blocked",
        );

        drop(manual_guard);
        assert!(
            guards.try_acquire("manual").await.is_some(),
            "manual guard should be released after completion",
        );

        drop(auto_guard);
        assert!(
            guards.try_acquire("auto").await.is_some(),
            "automatic guard should be released after completion",
        );
    }

    #[tokio::test]
    async fn interactive_operation_guards_allow_distinct_resources_but_block_duplicates() {
        let guards = InteractiveOperationGuardTable::default();

        let media_file_guard = guards
            .try_acquire("media-file:file-1")
            .await
            .expect("media file guard should acquire");
        let recycle_entry_guard = guards
            .try_acquire("recycle-entry:entry-1")
            .await
            .expect("recycle entry guard should acquire independently");

        assert!(
            guards.try_acquire("media-file:file-1").await.is_none(),
            "the same media file must not queue a duplicate operation",
        );
        assert!(
            guards.try_acquire("recycle-entry:entry-1").await.is_none(),
            "the same recycle entry must not queue a duplicate operation",
        );

        drop(media_file_guard);
        drop(recycle_entry_guard);

        assert!(guards.try_acquire("media-file:file-1").await.is_some());
        assert!(guards.try_acquire("recycle-entry:entry-1").await.is_some());
    }

    #[tokio::test]
    async fn acquire_title_serializes_submissions_for_same_title() {
        let guards = DownloadSubmissionGuardTable::default();
        let title_guard = guards.acquire_title("title-1").await;

        let guards_clone = guards.clone();
        let waiting_guard =
            tokio::spawn(async move { guards_clone.acquire_title("title-1").await });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(!waiting_guard.is_finished());

        drop(title_guard);

        tokio::time::timeout(std::time::Duration::from_secs(1), waiting_guard)
            .await
            .expect("overlapping scope guard should acquire after release")
            .expect("scope task should complete");
    }

    #[tokio::test]
    async fn plugin_install_orchestrator_rejects_second_begin_for_same_plugin_until_terminal() {
        let orchestrator = PluginInstallOrchestrator::default();
        let snapshot = orchestrator
            .begin("admin", "email", PluginInstallOperationKind::Install)
            .await
            .expect("first install should start");
        assert_eq!(snapshot.state, PluginInstallState::Downloading);

        let err = orchestrator
            .begin("viewer", "email", PluginInstallOperationKind::Upgrade)
            .await
            .expect_err("same plugin should be globally locked");
        assert_eq!(
            err,
            PluginInstallInProgressError {
                plugin_id: "email".to_string(),
            }
        );

        orchestrator
            .transition("admin", "email", PluginInstallState::Succeeded, None, None)
            .await;

        let upgrade = orchestrator
            .begin("viewer", "email", PluginInstallOperationKind::Upgrade)
            .await
            .expect("terminal state should release plugin lock");
        assert_eq!(upgrade.state, PluginInstallState::Downloading);
        assert_eq!(upgrade.operation_kind, PluginInstallOperationKind::Upgrade);
    }

    #[tokio::test]
    async fn plugin_install_orchestrator_scopes_progress_to_initiating_actor() {
        let orchestrator = PluginInstallOrchestrator::default();
        orchestrator
            .begin("admin", "email", PluginInstallOperationKind::Install)
            .await
            .expect("install should start");

        // Busy-ness is global; only the progress snapshot is actor-scoped.
        let active = orchestrator.active_plugin_ids().await;
        assert!(active.contains("email"));

        let admin_rx = orchestrator
            .subscribe("admin", "email")
            .await
            .expect("initiating actor should see snapshot");
        assert_eq!(admin_rx.borrow().state, PluginInstallState::Downloading);

        assert!(
            orchestrator.subscribe("viewer", "email").await.is_none(),
            "other actors should not see the snapshot"
        );

        orchestrator
            .transition(
                "admin",
                "email",
                PluginInstallState::Verifying,
                Some("verifying manifest".to_string()),
                None,
            )
            .await;

        let admin_rx = orchestrator
            .subscribe("admin", "email")
            .await
            .expect("snapshot should remain visible to initiator");
        let snapshot = admin_rx.borrow().clone();
        assert_eq!(snapshot.state, PluginInstallState::Verifying);
        assert_eq!(snapshot.message.as_deref(), Some("verifying manifest"));
    }
}

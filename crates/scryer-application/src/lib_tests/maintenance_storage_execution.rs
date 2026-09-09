//! Storage-pressure execution acceptance tests.
//!
//! The scripted probe is intentionally used only here: it proves that an
//! already-armed deletion refreshes capacity before every unlink without
//! depending on a workstation's real free space.

use super::*;

use crate::lib_tests::maintenance_title_completion::{TitleFixture, title_fixture};
use crate::maintenance_rules::storage::{
    TestCapacityProbe, install_maintenance_storage_capacity_probe_for_test,
};
use crate::maintenance_rules::{
    MaintenanceActionKind, MaintenanceActionSpec, MaintenanceCandidateFilter,
    MaintenanceGatesUpdate, MaintenanceRuleDraft,
};
use crate::ports::SubtitleDownloadRepository;
use scryer_domain::{
    ExternalSubtitleSourceKind, LifecycleActionRunStatus, MaintenanceEffectArming,
    MaintenanceEvaluationMode, MaintenanceRuleSubjectKind, SubtitleBlocklistEntry,
    SubtitleDownload,
};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Mutex as StdMutex;

#[derive(Default)]
struct TrackedSubtitleRepo {
    downloads: Mutex<Vec<SubtitleDownload>>,
}

#[async_trait]
impl SubtitleDownloadRepository for TrackedSubtitleRepo {
    async fn list_for_title(&self, title_id: &str) -> AppResult<Vec<SubtitleDownload>> {
        Ok(self
            .downloads
            .lock()
            .await
            .iter()
            .filter(|download| download.title_id == title_id)
            .cloned()
            .collect())
    }

    async fn get(&self, id: &str) -> AppResult<Option<SubtitleDownload>> {
        Ok(self
            .downloads
            .lock()
            .await
            .iter()
            .find(|download| download.id == id)
            .cloned())
    }

    async fn list_for_media_file(&self, media_file_id: &str) -> AppResult<Vec<SubtitleDownload>> {
        Ok(self
            .downloads
            .lock()
            .await
            .iter()
            .filter(|download| download.media_file_id == media_file_id)
            .cloned()
            .collect())
    }

    async fn list_probe_cache_for_media_file(
        &self,
        _: &str,
    ) -> AppResult<Vec<crate::subtitles::ExternalSubtitleProbeCacheEntry>> {
        Ok(Vec::new())
    }

    async fn list_blocklist_for_media_file(
        &self,
        _: &str,
    ) -> AppResult<Vec<SubtitleBlocklistEntry>> {
        Ok(Vec::new())
    }

    async fn insert(&self, download: &SubtitleDownload) -> AppResult<()> {
        self.downloads.lock().await.push(download.clone());
        Ok(())
    }

    async fn upsert_probe_cache_entry(
        &self,
        _: &crate::subtitles::ExternalSubtitleProbeCacheEntry,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn set_synced(&self, id: &str, synced: bool) -> AppResult<()> {
        if let Some(download) = self
            .downloads
            .lock()
            .await
            .iter_mut()
            .find(|download| download.id == id)
        {
            download.synced = synced;
        }
        Ok(())
    }

    async fn delete(&self, id: &str) -> AppResult<Option<SubtitleDownload>> {
        let mut downloads = self.downloads.lock().await;
        let Some(index) = downloads.iter().position(|download| download.id == id) else {
            return Ok(None);
        };
        Ok(Some(downloads.remove(index)))
    }

    async fn delete_probe_cache_entry(&self, _: &str, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn is_blocklisted(&self, _: &str, _: &str, _: &str) -> AppResult<bool> {
        Ok(false)
    }

    async fn blocklist(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: &str,
        _: Option<&str>,
    ) -> AppResult<()> {
        Ok(())
    }
}

fn capacity(available_bytes: u64) -> crate::helpers::FilesystemSpace {
    crate::helpers::FilesystemSpace {
        total_bytes: 1_000,
        available_bytes,
    }
}

fn scripted_capacity_probe(
    readings: impl IntoIterator<Item = Option<crate::helpers::FilesystemSpace>>,
) -> TestCapacityProbe {
    let readings = Arc::new(StdMutex::new(VecDeque::from_iter(readings)));
    Arc::new(move |_| {
        readings
            .lock()
            .expect("capacity sequence lock")
            .pop_front()
            .expect("scripted capacity reading")
    })
}

async fn selected_root_id(fixture: &TitleFixture) -> String {
    selected_root_id_at(fixture, &fixture.root).await
}

async fn selected_root_id_at(fixture: &TitleFixture, path: &Path) -> String {
    let root_path = path.to_string_lossy().to_string();
    fixture
        .execution
        .app
        .services
        .catalog
        .libraries
        .list(None)
        .await
        .expect("list configured libraries")
        .into_iter()
        .flat_map(|library| library.roots)
        .find(|root| root.path == root_path)
        .expect("fixture root is configured")
        .id
}

async fn add_movie_file(fixture: &TitleFixture, relative_path: &str) -> (PathBuf, String) {
    let path = fixture.root.join(relative_path);
    let file_id = add_movie_file_at(fixture, &path).await;
    (path, file_id)
}

async fn add_movie_file_at(fixture: &TitleFixture, path: &Path) -> String {
    std::fs::create_dir_all(path.parent().expect("media parent")).expect("create media parent");
    std::fs::write(&path, b"video").expect("write media file");
    fixture
        .execution
        .app
        .services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: fixture.title.id.clone(),
            file_path: path.to_string_lossy().to_string(),
            size_bytes: 5,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .expect("insert movie file")
}

async fn arm_storage_title_deletion(fixture: &TitleFixture, storage_root_id: String) -> String {
    let matcher = format!(
        "match if {{ input.subject.title_id == {:?}; input.facts.storage_available_bytes < 200 }}",
        fixture.title.id
    );
    arm_storage_title_deletion_with_matcher(fixture, storage_root_id, matcher).await
}

async fn arm_storage_title_deletion_with_matcher(
    fixture: &TitleFixture,
    storage_root_id: String,
    matcher: String,
) -> String {
    let created = fixture
        .execution
        .app
        .create_maintenance_rule_set(
            &fixture.execution.user,
            MaintenanceRuleDraft {
                subject_kind: MaintenanceRuleSubjectKind::Title,
                name: "Storage pressure".into(),
                description: String::new(),
                rego_source: matcher,
                action_definition: crate::maintenance_rules::MaintenanceActionDefinition::Legacy(
                    MaintenanceActionSpec::new(MaintenanceActionKind::UnmonitorTitleDeleteAllFiles),
                ),
                grace_days: 0,
                storage_root_id: Some(storage_root_id),
                library_ids: vec![],
                evaluation_mode: None,
            },
        )
        .await
        .expect("create storage rule");
    fixture
        .execution
        .app
        .set_maintenance_rule_evaluation_mode(
            &fixture.execution.user,
            &created.rule_set.id,
            MaintenanceEvaluationMode::Observe,
        )
        .await
        .expect("enable observation");
    fixture
        .execution
        .app
        .set_maintenance_instance_gates(
            &fixture.execution.user,
            MaintenanceGatesUpdate {
                evaluation_enabled: Some(true),
                result_display_enabled: Some(true),
                destructive_effects_enabled: Some(true),
                ..Default::default()
            },
        )
        .await
        .expect("open maintenance gates");
    fixture
        .execution
        .app
        .run_maintenance_rule_evaluation_job()
        .await
        .expect("evaluate storage rule");
    let active_candidates = fixture
        .execution
        .evaluation
        .all_candidates()
        .await
        .into_iter()
        .filter(|candidate| {
            candidate.rule_set_id == created.rule_set.id && !candidate.state.is_terminal()
        })
        .count();
    assert_eq!(active_candidates, 1, "one selected movie candidate");
    fixture
        .execution
        .app
        .set_maintenance_rule_arming(
            &fixture.execution.user,
            &created.rule_set.id,
            MaintenanceEffectArming::Destructive,
            Some(active_candidates as i64),
        )
        .await
        .expect("arm storage rule");
    created.rule_set.id
}

#[tokio::test]
async fn candidate_view_reports_exact_selected_root_file_totals() {
    let _probe =
        install_maintenance_storage_capacity_probe_for_test(scripted_capacity_probe([Some(
            capacity(100),
        )]))
        .await;
    let fixture = title_fixture("Candidate Root Summary", MediaFacet::Movie).await;
    add_movie_file(&fixture, "Selected/movie.mkv").await;
    let outside_root = fixture
        .root
        .parent()
        .expect("fixture root parent")
        .join("unselected/movie.mkv");
    add_movie_file_at(&fixture, &outside_root).await;

    let rule_set_id = arm_storage_title_deletion(&fixture, selected_root_id(&fixture).await).await;
    let candidate = fixture
        .execution
        .app
        .list_maintenance_candidates(
            &fixture.execution.user,
            MaintenanceCandidateFilter {
                rule_set_id: Some(rule_set_id),
                ..Default::default()
            },
        )
        .await
        .expect("list storage candidate")
        .into_iter()
        .next()
        .expect("storage candidate");

    assert_eq!(candidate.file_count, Some(2));
    assert_eq!(candidate.total_size_bytes, Some(10));
    assert_eq!(candidate.storage_root_file_count, Some(1));
    assert_eq!(candidate.storage_root_total_size_bytes, Some(5));
}

#[tokio::test]
async fn storage_recovery_stops_before_the_next_file() {
    // Evaluation, action safety, first file, then a recovered capacity before
    // the second file. The recovery must cancel the remaining manifest.
    let _probe = install_maintenance_storage_capacity_probe_for_test(scripted_capacity_probe([
        Some(capacity(100)),
        Some(capacity(100)),
        Some(capacity(100)),
        Some(capacity(700)),
    ]))
    .await;
    let fixture = title_fixture("Pressure Movie", MediaFacet::Movie).await;
    let (first, _) = add_movie_file(&fixture, "Pressure/first.mkv").await;
    let (second, _) = add_movie_file(&fixture, "Pressure/second.mkv").await;

    arm_storage_title_deletion(&fixture, selected_root_id(&fixture).await).await;
    let report = fixture
        .execution
        .app
        .run_lifecycle_action_handling_job()
        .await
        .expect("run storage deletion");
    assert_eq!(
        report.canceled, 1,
        "recovered capacity cancels remaining work"
    );
    assert_eq!(report.executed, 0);
    assert_ne!(
        first.exists(),
        second.exists(),
        "exactly one file was deleted"
    );
    let action_runs = fixture.execution.evaluation.all_action_runs().await;
    assert_eq!(action_runs.len(), 1);
    assert_eq!(action_runs[0].status, LifecycleActionRunStatus::Held);
}

#[tokio::test]
async fn unknown_storage_capacity_holds_without_consuming_an_attempt_then_resumes() {
    // Evaluation, the initial action check, and the first file are low. The
    // second per-file refresh is unavailable, then scheduled evaluation and a
    // retry can reuse the incomplete manifest without repeating file one.
    let _probe = install_maintenance_storage_capacity_probe_for_test(scripted_capacity_probe([
        Some(capacity(100)),
        Some(capacity(100)),
        Some(capacity(100)),
        None,
        Some(capacity(100)),
        Some(capacity(100)),
        Some(capacity(100)),
    ]))
    .await;
    let fixture = title_fixture("Recoverable Unknown", MediaFacet::Movie).await;
    let (first_path, first_id) = add_movie_file(&fixture, "Recoverable/first.mkv").await;
    let (second_path, second_id) = add_movie_file(&fixture, "Recoverable/second.mkv").await;
    let rule_set_id = arm_storage_title_deletion(&fixture, selected_root_id(&fixture).await).await;

    let held = fixture
        .execution
        .app
        .run_lifecycle_action_handling_job()
        .await
        .expect("hold while capacity is unavailable");
    assert_eq!(held.held, 1);
    let action_runs = fixture.execution.evaluation.all_action_runs().await;
    assert_eq!(action_runs.len(), 1);
    assert_eq!(action_runs[0].status, LifecycleActionRunStatus::Held);
    let detail: serde_json::Value =
        serde_json::from_str(&action_runs[0].detail).expect("parse partial deletion journal");
    let completed_ids = detail["files"]
        .as_array()
        .expect("journal files")
        .iter()
        .filter(|file| file["completed"] == true)
        .filter_map(|file| file["file_id"].as_str())
        .collect::<HashSet<_>>();
    assert_eq!(completed_ids.len(), 1, "one file completed before the hold");
    let (completed_path, remaining_path) = if completed_ids.contains(first_id.as_str()) {
        (&first_path, &second_path)
    } else {
        assert!(completed_ids.contains(second_id.as_str()));
        (&second_path, &first_path)
    };
    assert!(
        !completed_path.exists(),
        "journaled completed file stays deleted"
    );
    assert!(
        remaining_path.exists(),
        "unknown capacity preserves the next file"
    );
    let held_candidate = fixture
        .execution
        .evaluation
        .all_candidates()
        .await
        .into_iter()
        .find(|candidate| candidate.rule_set_id == rule_set_id)
        .expect("candidate remains for recovery");
    assert_eq!(
        held_candidate.action_attempts, 0,
        "unknown capacity refunds the attempt"
    );
    let match_generation = held_candidate.match_generation;
    let due_at = held_candidate.due_at;

    fixture
        .execution
        .app
        .run_maintenance_rule_evaluation_job()
        .await
        .expect("re-evaluate recoverable candidate");
    let after_evaluation = fixture
        .execution
        .evaluation
        .all_candidates()
        .await
        .into_iter()
        .find(|candidate| candidate.rule_set_id == rule_set_id)
        .expect("partial candidate stays active after evaluation");
    assert_eq!(after_evaluation.match_generation, match_generation);
    assert_eq!(after_evaluation.due_at, due_at);
    assert_eq!(after_evaluation.action_attempts, 0);

    let resumed = fixture
        .execution
        .app
        .run_lifecycle_action_handling_job()
        .await
        .expect("resume after fresh capacity returns");
    assert_eq!(
        resumed.executed, 1,
        "recovered low capacity resumes deletion"
    );
    assert!(
        !completed_path.exists(),
        "retry resumes at the journaled remaining file"
    );
    assert!(!remaining_path.exists());
    let candidates = fixture.execution.evaluation.all_candidates().await;
    assert_eq!(candidates[0].action_attempts, 1);
    let action_runs = fixture.execution.evaluation.all_action_runs().await;
    assert_eq!(action_runs.len(), 2);
    assert_eq!(action_runs[0].status, LifecycleActionRunStatus::Held);
    assert_eq!(action_runs[1].status, LifecycleActionRunStatus::Succeeded);
}

#[tokio::test]
async fn storage_retry_reconciles_a_journaled_path_missing_before_catalog_cleanup() {
    // The first action creates and persists the manifest, then capacity becomes
    // unavailable before unlink. Removing the path now models a crash after an
    // authorized unlink but before the catalog row and journal were reconciled.
    let _probe = install_maintenance_storage_capacity_probe_for_test(scripted_capacity_probe([
        Some(capacity(100)),
        Some(capacity(100)),
        None,
        Some(capacity(100)),
        Some(capacity(100)),
        Some(capacity(100)),
    ]))
    .await;
    let fixture = title_fixture("Interrupted Cleanup", MediaFacet::Movie).await;
    let (media_path, media_file_id) = add_movie_file(&fixture, "Interrupted/movie.mkv").await;
    arm_storage_title_deletion(&fixture, selected_root_id(&fixture).await).await;

    let held = fixture
        .execution
        .app
        .run_lifecycle_action_handling_job()
        .await
        .expect("persist manifest before the interrupted unlink");
    assert_eq!(held.held, 1);
    let held_runs = fixture.execution.evaluation.all_action_runs().await;
    assert_eq!(held_runs.len(), 1);
    let held_detail: serde_json::Value =
        serde_json::from_str(&held_runs[0].detail).expect("parse held journal");
    assert_eq!(
        held_detail["files"]
            .as_array()
            .and_then(|files| files.first())
            .and_then(|file| file["file_id"].as_str()),
        Some(media_file_id.as_str()),
        "the retry may clean up only this persisted journal file"
    );
    assert!(media_path.exists());

    std::fs::remove_file(&media_path).expect("simulate unlink before catalog cleanup");
    fixture
        .execution
        .app
        .run_maintenance_rule_evaluation_job()
        .await
        .expect("re-evaluate exact missing journal path");
    let resumed = fixture
        .execution
        .app
        .run_lifecycle_action_handling_job()
        .await
        .expect("reconcile missing journal path");
    assert_eq!(resumed.executed, 1, "root summary does not cancel recovery");
    assert_eq!(resumed.canceled, 0);
    assert!(!media_path.exists());
    assert!(
        fixture
            .execution
            .app
            .services
            .library
            .media_files
            .get_media_file_by_id(&media_file_id)
            .await
            .expect("read cleaned catalog row")
            .is_none(),
        "the exact missing path's catalog row is reconciled"
    );
    let action_runs = fixture.execution.evaluation.all_action_runs().await;
    assert_eq!(action_runs.len(), 2);
    assert_eq!(action_runs[0].status, LifecycleActionRunStatus::Held);
    assert_eq!(action_runs[1].status, LifecycleActionRunStatus::Succeeded);
    let completed_detail: serde_json::Value =
        serde_json::from_str(&action_runs[1].detail).expect("parse completed journal");
    assert_eq!(
        completed_detail["files"]
            .as_array()
            .and_then(|files| files.first())
            .and_then(|file| file["file_id"].as_str()),
        Some(media_file_id.as_str())
    );
    assert_eq!(
        completed_detail["files"]
            .as_array()
            .and_then(|files| files.first())
            .and_then(|file| file["completed"].as_bool()),
        Some(true)
    );
}

#[tokio::test]
async fn selected_root_journal_resumes_while_retaining_owned_files_on_other_roots() {
    let _probe = install_maintenance_storage_capacity_probe_for_test(scripted_capacity_probe([
        Some(capacity(100)),
        Some(capacity(100)),
        None,
        Some(capacity(100)),
        Some(capacity(100)),
        Some(capacity(100)),
    ]))
    .await;
    let fixture = title_fixture("Two Roots", MediaFacet::Movie).await;
    let root_b = fixture
        .root
        .parent()
        .expect("fixture root parent")
        .join("root-b");
    std::fs::create_dir_all(&root_b).expect("create second configured root");
    fixture
        .execution
        .app
        .update_media_settings(
            &fixture.execution.user,
            MediaFacet::Movie,
            empty_update_media_settings_with_roots(vec![
                build_root_folder_entry(&fixture.root, true),
                build_root_folder_entry(&root_b, false),
            ]),
        )
        .await
        .expect("configure both movie roots");
    let (root_a_path, root_a_file_id) = add_movie_file(&fixture, "Two Roots/root-a.mkv").await;
    let root_b_path = root_b.join("Two Roots/root-b.mkv");
    let root_b_file_id = add_movie_file_at(&fixture, &root_b_path).await;
    arm_storage_title_deletion(&fixture, selected_root_id(&fixture).await).await;

    let held = fixture
        .execution
        .app
        .run_lifecycle_action_handling_job()
        .await
        .expect("hold selected-root journal before unlink");
    assert_eq!(held.held, 1);
    let held_runs = fixture.execution.evaluation.all_action_runs().await;
    let held_detail: serde_json::Value =
        serde_json::from_str(&held_runs[0].detail).expect("parse selected-root journal");
    let journal_file_ids = held_detail["files"]
        .as_array()
        .expect("journal files")
        .iter()
        .filter_map(|file| file["file_id"].as_str())
        .collect::<HashSet<_>>();
    assert_eq!(journal_file_ids, HashSet::from([root_a_file_id.as_str()]));
    assert!(root_a_path.exists());
    assert!(root_b_path.exists());

    fixture
        .execution
        .app
        .run_maintenance_rule_evaluation_job()
        .await
        .expect("re-evaluate selected-root journal");
    let resumed = fixture
        .execution
        .app
        .run_lifecycle_action_handling_job()
        .await
        .expect("resume selected-root journal");
    assert_eq!(resumed.executed, 1);
    assert!(!root_a_path.exists());
    assert!(root_b_path.exists(), "owned root-b file stays retained");
    assert!(
        fixture
            .execution
            .app
            .services
            .library
            .media_files
            .get_media_file_by_id(&root_a_file_id)
            .await
            .expect("read root-a row")
            .is_none()
    );
    assert!(
        fixture
            .execution
            .app
            .services
            .library
            .media_files
            .get_media_file_by_id(&root_b_file_id)
            .await
            .expect("read retained root-b row")
            .is_some(),
        "retained root-b ownership does not block root-a recovery"
    );
}

#[tokio::test]
async fn storage_guard_hold_before_mutation_rechecks_live_monitoring_on_retry() {
    let _probe = install_maintenance_storage_capacity_probe_for_test(scripted_capacity_probe([
        Some(capacity(100)),
        Some(capacity(100)),
        Some(capacity(100)),
        Some(capacity(100)),
    ]))
    .await;
    let fixture = title_fixture("Guarded Retry", MediaFacet::Movie).await;
    let (media_path, media_file_id) = add_movie_file(&fixture, "Guarded/movie.mkv").await;
    let matcher = format!(
        "match if {{ input.subject.title_id == {:?}; input.facts.monitored; input.facts.storage_available_bytes < 200 }}",
        fixture.title.id
    );
    let rule_set_id = arm_storage_title_deletion_with_matcher(
        &fixture,
        selected_root_id(&fixture).await,
        matcher,
    )
    .await;
    let guard = fixture
        .execution
        .app
        .runtime
        .jobs
        .interactive_operation_guards
        .try_acquire("maintenance-storage-deletion")
        .await
        .expect("hold storage deletion guard");

    let held = fixture
        .execution
        .app
        .run_lifecycle_action_handling_job()
        .await
        .expect("hold before mutation starts");
    assert_eq!(held.held, 1);
    assert!(media_path.exists());
    let held_runs = fixture.execution.evaluation.all_action_runs().await;
    let held_detail: serde_json::Value =
        serde_json::from_str(&held_runs[0].detail).expect("parse pre-mutation journal");
    assert_eq!(held_detail["mutation_started"], false);
    let held_candidate = fixture
        .execution
        .evaluation
        .all_candidates()
        .await
        .into_iter()
        .find(|candidate| candidate.rule_set_id == rule_set_id)
        .expect("candidate remains after guard hold");
    assert_eq!(
        held_candidate.action_attempts, 0,
        "guard hold refunds attempt"
    );

    fixture
        .execution
        .app
        .set_title_monitored(&fixture.execution.user, &fixture.title.id, false)
        .await
        .expect("change monitoring while mutation has not started");
    drop(guard);
    let retried = fixture
        .execution
        .app
        .run_lifecycle_action_handling_job()
        .await
        .expect("retry with live monitoring state");
    assert_eq!(retried.canceled, 1);
    assert_eq!(retried.executed, 0);
    assert!(
        media_path.exists(),
        "an unstarted checkpoint never restores its stale monitored fact"
    );
    assert!(
        fixture
            .execution
            .app
            .services
            .library
            .media_files
            .get_media_file_by_id(&media_file_id)
            .await
            .expect("read retained media row")
            .is_some()
    );
}

#[tokio::test]
async fn tracked_off_root_subtitle_holds_the_selected_root_manifest_before_video_deletion() {
    let _probe = install_maintenance_storage_capacity_probe_for_test(scripted_capacity_probe([
        Some(capacity(100)),
        Some(capacity(100)),
    ]))
    .await;
    let mut fixture = title_fixture("Bounded Subtitle", MediaFacet::Movie).await;
    let subtitles = Arc::new(TrackedSubtitleRepo::default());
    fixture.execution.app = fixture
        .execution
        .app
        .with_test_overrides(|services| services.with_subtitle_downloads(subtitles.clone()));
    let (video_path, media_file_id) = add_movie_file(&fixture, "Bounded/video.mkv").await;
    let subtitle_path = fixture
        .root
        .parent()
        .expect("fixture root parent")
        .join("outside/Bounded.en.srt");
    std::fs::create_dir_all(subtitle_path.parent().expect("subtitle parent"))
        .expect("create subtitle parent");
    std::fs::write(&subtitle_path, b"subtitle").expect("write tracked subtitle");
    subtitles
        .insert(&SubtitleDownload {
            id: Id::new().0,
            media_file_id,
            title_id: fixture.title.id.clone(),
            episode_id: None,
            source_kind: ExternalSubtitleSourceKind::Downloaded,
            language: "eng".to_string(),
            provider: Some("fixture".to_string()),
            provider_file_id: Some("fixture-subtitle".to_string()),
            file_path: subtitle_path.to_string_lossy().to_string(),
            score: Some(100),
            hearing_impaired: false,
            forced: false,
            ai_translated: false,
            machine_translated: false,
            uploader: None,
            release_info: None,
            synced: false,
            downloaded_at: Utc::now().to_rfc3339(),
        })
        .await
        .expect("track off-root subtitle");

    arm_storage_title_deletion(&fixture, selected_root_id(&fixture).await).await;
    let report = fixture
        .execution
        .app
        .run_lifecycle_action_handling_job()
        .await
        .expect("hold cross-root manifest");
    assert_eq!(report.held, 1);
    assert_eq!(report.executed, 0);
    assert!(
        video_path.exists(),
        "cross-root manifest never unlinks video"
    );
    assert!(
        subtitle_path.exists(),
        "tracked subtitle remains on its own root"
    );
    let candidate = fixture
        .execution
        .evaluation
        .all_candidates()
        .await
        .into_iter()
        .next()
        .expect("candidate remains held");
    assert_eq!(candidate.action_attempts, 0);
}

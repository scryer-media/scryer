//! Sequence-only deletion regressions over the real scoped-deletion journal.

use super::*;

use crate::lib_tests::maintenance_execution::{ExecutionFixture, execution_app, seed_title};
use crate::lib_tests::maintenance_sequence_execution::{arm_and_evaluate, sequence_draft};
use crate::lib_tests::maintenance_title_completion::{
    TitleFixture, add_episode_file, season_and_episode, title_fixture,
};
use crate::maintenance_rules::storage::{
    TestCapacityProbe, install_maintenance_storage_capacity_probe_for_test,
};
use crate::maintenance_rules::{
    MaintenanceActionSequence, MaintenanceActionStep, MaintenanceActionStepKind,
    MaintenanceActionStepParameters,
};
use crate::ports::{
    LifecycleActionRunRepository, MaintenanceActionStepRepository, MaintenanceCandidateRepository,
    MaintenanceSequenceCompletionRepository,
};
use chrono::{Duration, Utc};
use scryer_domain::{
    LifecycleActionRunStatus, MaintenanceActionStepKey, MaintenanceActionStepRun,
    MaintenanceActionStepState, MaintenanceEffectArming, MaintenanceSequenceTerminalMembership,
    MaintenanceSequenceTerminalOutcome,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) fn deletion_steps() -> Vec<MaintenanceActionStep> {
    vec![
        MaintenanceActionStep {
            id: "unmonitor".to_string(),
            kind: MaintenanceActionStepKind::Unmonitor,
            parameters: MaintenanceActionStepParameters::Unmonitor {
                include_descendants: true,
            },
        },
        MaintenanceActionStep {
            id: "delete-files".to_string(),
            kind: MaintenanceActionStepKind::DeleteFiles,
            parameters: MaintenanceActionStepParameters::None,
        },
    ]
}

fn write_media_file(path: &Path) {
    std::fs::create_dir_all(path.parent().expect("media path parent"))
        .expect("create media parent");
    std::fs::write(path, b"video").expect("write media file");
}

async fn add_movie_file(app: &AppUseCase, title_id: &str, path: &Path) -> String {
    write_media_file(path);
    app.services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title_id.to_string(),
            file_path: path.to_string_lossy().to_string(),
            size_bytes: 5,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .expect("insert media file")
}

/// Seed a catalog target that fails the real policy manifest path because its
/// file was never created. Executor regressions use this to exercise bounded
/// high-risk sequence failures without a synthetic deletion adapter.
pub(super) async fn add_catalog_movie_file_without_physical_path(
    app: &AppUseCase,
    title_id: &str,
    path: &Path,
) -> String {
    app.services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title_id.to_string(),
            file_path: path.to_string_lossy().to_string(),
            size_bytes: 5,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .expect("insert catalog-only media file")
}

async fn selected_root_id(fixture: &TitleFixture) -> String {
    let root_path = fixture.root.to_string_lossy();
    fixture
        .execution
        .app
        .services
        .catalog
        .libraries
        .list(None)
        .await
        .expect("list configured roots")
        .into_iter()
        .flat_map(|library| library.roots)
        .find(|root| root.path == root_path)
        .expect("fixture root is configured")
        .id
}

pub(super) async fn selected_root_sequence_deletion_draft(
    fixture: &TitleFixture,
) -> crate::maintenance_rules::MaintenanceRuleDraft {
    let mut draft = sequence_draft(deletion_steps());
    draft.storage_root_id = Some(selected_root_id(fixture).await);
    draft
}

fn capacity(available_bytes: u64) -> crate::helpers::FilesystemSpace {
    crate::helpers::FilesystemSpace {
        total_bytes: 1_000,
        available_bytes,
    }
}

fn capacity_holds_after_file_removal(path: PathBuf) -> (TestCapacityProbe, Arc<AtomicBool>) {
    let recovered = Arc::new(AtomicBool::new(false));
    let probe_recovered = recovered.clone();
    (
        Arc::new(move |_| {
            (path.exists() || probe_recovered.load(Ordering::SeqCst)).then_some(capacity(100))
        }),
        recovered,
    )
}

fn stable_capacity_probe(reading: crate::helpers::FilesystemSpace) -> TestCapacityProbe {
    Arc::new(move |_| Some(reading))
}

async fn persist_completed_unmonitor(
    fixture: &ExecutionFixture,
    candidate: &scryer_domain::LifecycleCandidate,
    sequence: &MaintenanceActionSequence,
) {
    let step = sequence
        .steps
        .iter()
        .find(|step| step.kind == MaintenanceActionStepKind::Unmonitor)
        .expect("unmonitor step");
    let now = Utc::now();
    let run = MaintenanceActionStepRun {
        key: MaintenanceActionStepKey {
            candidate_id: candidate.id.clone(),
            match_generation: candidate.match_generation,
            revision_number: candidate.revision_number,
            step_id: step.id.clone(),
        },
        rule_set_id: candidate.rule_set_id.clone(),
        title_id: candidate.title_id.clone(),
        subject_kind: candidate.subject_kind.clone(),
        subject_id: candidate.subject_id.clone(),
        sequence_content_hash: sequence.content_hash().expect("sequence hash"),
        step_kind: step.kind.as_wire_str().to_string(),
        intent_json: serde_json::json!({"step": step, "search_request": null}).to_string(),
        before_state_json: serde_json::json!({
            "schema_version": 1,
            "facts_monitored": true,
            "monitoring_changed": true,
        })
        .to_string(),
        target_identity_json: serde_json::json!({
            "schema_version": 1,
            "rule_set_id": candidate.rule_set_id,
            "candidate_id": candidate.id,
            "match_generation": candidate.match_generation,
            "revision_number": candidate.revision_number,
            "title_id": candidate.title_id,
            "subject_kind": candidate.subject_kind,
            "subject_id": candidate.subject_id,
            "step_id": step.id,
            "step_kind": step.kind.as_wire_str(),
            "sequence_content_hash": sequence.content_hash().expect("sequence hash"),
        })
        .to_string(),
        provenance_json: serde_json::json!({
            "schema_version": 1,
            "rule_set_id": candidate.rule_set_id,
            "candidate_id": candidate.id,
            "match_generation": candidate.match_generation,
            "revision_number": candidate.revision_number,
            "title_id": candidate.title_id,
            "subject_kind": candidate.subject_kind,
            "subject_id": candidate.subject_id,
            "step_id": step.id,
            "step_kind": step.kind.as_wire_str(),
            "monitoring_changed": true,
            "include_descendants": true,
            "expected_postcondition": "unmonitored",
        })
        .to_string(),
        state: MaintenanceActionStepState::Running,
        attempt: 1,
        lease_id: None,
        lease_expires_at: Some(now + Duration::minutes(5)),
        hold_reason: None,
        error: None,
        created_at: now,
        updated_at: now,
        finished_at: None,
    };
    let lease_id = "persisted-unmonitor-lease";
    let mut claimed = match fixture
        .evaluation
        .claim_action_step(&run, now - Duration::minutes(1), lease_id, now)
        .await
        .expect("persist completed prerequisite")
    {
        crate::MaintenanceActionStepClaim::Claimed(run) => run,
        other => panic!("unexpected unmonitor claim: {other:?}"),
    };
    claimed.state = MaintenanceActionStepState::Succeeded;
    claimed.updated_at = now;
    claimed.finished_at = Some(now);
    assert!(
        fixture
            .evaluation
            .finish_action_step(&claimed, lease_id)
            .await
            .expect("finish persisted prerequisite")
    );
}

fn terminal_membership(
    id: impl Into<String>,
    candidate: &scryer_domain::LifecycleCandidate,
    sequence: &MaintenanceActionSequence,
    outcome: MaintenanceSequenceTerminalOutcome,
) -> MaintenanceSequenceTerminalMembership {
    let expected_step_ids: Vec<_> = sequence.steps.iter().map(|step| step.id.clone()).collect();
    let terminal_step_id = match outcome {
        MaintenanceSequenceTerminalOutcome::Succeeded => None,
        MaintenanceSequenceTerminalOutcome::Failed => expected_step_ids.last().cloned(),
    };
    MaintenanceSequenceTerminalMembership {
        id: id.into(),
        candidate_id: candidate.id.clone(),
        rule_set_id: candidate.rule_set_id.clone(),
        revision_number: candidate.revision_number,
        matcher_content_hash: candidate.matcher_content_hash.clone(),
        title_id: candidate.title_id.clone(),
        subject_kind: candidate.subject_kind.clone(),
        subject_id: candidate.subject_id.clone(),
        match_generation: candidate.match_generation,
        sequence_content_hash: sequence.content_hash().expect("sequence hash"),
        outcome,
        expected_step_ids: expected_step_ids.clone(),
        completed_step_ids: expected_step_ids,
        terminal_step_id,
        created_at: Utc::now(),
        released_at: None,
    }
}

#[tokio::test]
async fn sequence_unmonitor_then_delete_files_removes_only_the_matched_title_files() {
    let fixture = title_fixture("Sequence target", MediaFacet::Movie).await;
    let target_path = fixture.root.join("target/movie.mkv");
    let target_file_id =
        add_movie_file(&fixture.execution.app, &fixture.title.id, &target_path).await;

    let (rule_set_id, candidate) =
        arm_and_evaluate(&fixture.execution, sequence_draft(deletion_steps())).await;
    let sibling = seed_title(
        &fixture.execution.app,
        &fixture.execution.user,
        "Sequence sibling",
        true,
    )
    .await;
    let sibling_path = fixture.root.join("sibling/movie.mkv");
    let sibling_file_id = add_movie_file(&fixture.execution.app, &sibling.id, &sibling_path).await;
    fixture
        .execution
        .app
        .set_maintenance_rule_arming(
            &fixture.execution.user,
            &rule_set_id,
            MaintenanceEffectArming::Destructive,
            Some(1),
        )
        .await
        .expect("arm destructive sequence");

    let report = fixture
        .execution
        .app
        .run_lifecycle_action_handling_job()
        .await
        .expect("execute sequence deletion");
    assert_eq!(report.executed, 1, "{report:?}");
    assert!(!target_path.exists(), "target file is deleted");
    assert!(sibling_path.exists(), "sibling file is preserved");
    assert!(
        fixture
            .execution
            .app
            .services
            .library
            .media_files
            .get_media_file_by_id(&target_file_id)
            .await
            .expect("read deleted target row")
            .is_none()
    );
    assert!(
        fixture
            .execution
            .app
            .services
            .library
            .media_files
            .get_media_file_by_id(&sibling_file_id)
            .await
            .expect("read preserved sibling row")
            .is_some()
    );
    let history = fixture
        .execution
        .app
        .list_maintenance_action_runs(
            &fixture.execution.user,
            Some(&rule_set_id),
            Some(&candidate.id),
            None,
        )
        .await
        .expect("read operator-visible sequence history");
    assert_eq!(
        history.len(),
        1,
        "the child journal is not a second history row"
    );
    let summary: serde_json::Value =
        serde_json::from_str(&history[0].run.detail).expect("parse sequence summary");
    let deletion_outcomes = summary["deletion_outcomes"]
        .as_array()
        .expect("summary embeds its deletion evidence");
    assert_eq!(deletion_outcomes.len(), 1);
    let checkpoint: serde_json::Value = serde_json::from_str(
        deletion_outcomes[0]["detail"]
            .as_str()
            .expect("embedded scoped deletion checkpoint"),
    )
    .expect("parse embedded checkpoint");
    let files = checkpoint["files"]
        .as_array()
        .expect("checkpoint has file outcomes");
    assert_eq!(
        files.len(),
        1,
        "history retains the exact per-file outcome count"
    );
    assert_eq!(files[0]["file_id"].as_str(), Some(target_file_id.as_str()));
    assert_eq!(files[0]["completed"].as_bool(), Some(true));
}

#[tokio::test]
async fn episode_sequence_unmonitor_then_delete_files_preserves_other_season_records_and_files() {
    let fixture = title_fixture("Sequence episode scope", MediaFacet::Series).await;
    let (_, selected) = season_and_episode(&fixture, 1, 2, "2020-01-02").await;
    let (_, other_season) = season_and_episode(&fixture, 2, 2, "2020-02-02").await;
    let selected_path = fixture.root.join("Sequence/Season 01/S01E02.mkv");
    let other_path = fixture.root.join("Sequence/Season 02/S02E02.mkv");
    let selected_file_id =
        add_episode_file(&fixture, &selected, "Sequence/Season 01/S01E02.mkv").await;
    let other_file_id =
        add_episode_file(&fixture, &other_season, "Sequence/Season 02/S02E02.mkv").await;
    let mut draft = sequence_draft(deletion_steps());
    draft.subject_kind = scryer_domain::MaintenanceRuleSubjectKind::Episode;
    draft.rego_source = format!(
        "package selected_episode\nimport rego.v1\n\nmatch if {{\n  input.subject.subject_id == {:?}\n}}\n",
        selected.id
    );
    let (rule_set_id, candidate) = arm_and_evaluate(&fixture.execution, draft).await;
    assert_eq!(candidate.subject_id, selected.id);
    fixture
        .execution
        .app
        .set_maintenance_rule_arming(
            &fixture.execution.user,
            &rule_set_id,
            MaintenanceEffectArming::Destructive,
            Some(1),
        )
        .await
        .expect("arm destructive episode sequence");

    let report = fixture
        .execution
        .app
        .run_lifecycle_action_handling_job()
        .await
        .expect("execute selected episode sequence");
    assert_eq!(report.executed, 1, "{report:?}");
    assert!(
        !selected_path.exists(),
        "S01E02 is the exact deletion target"
    );
    assert!(other_path.exists(), "S02E02 stays on disk");
    assert!(
        fixture
            .execution
            .app
            .services
            .library
            .media_files
            .get_media_file_by_id(&selected_file_id)
            .await
            .expect("read deleted S01E02 row")
            .is_none()
    );
    assert!(
        fixture
            .execution
            .app
            .services
            .library
            .media_files
            .get_media_file_by_id(&other_file_id)
            .await
            .expect("read preserved S02E02 row")
            .is_some()
    );
    assert!(
        fixture
            .execution
            .app
            .services
            .catalog
            .shows
            .get_episode_by_id(&selected.id)
            .await
            .expect("read unmonitored S01E02")
            .is_some_and(|episode| !episode.monitored)
    );
    assert!(
        fixture
            .execution
            .app
            .services
            .catalog
            .shows
            .get_episode_by_id(&other_season.id)
            .await
            .expect("read preserved S02E02")
            .is_some_and(|episode| episode.monitored),
        "the sequence cannot unmonitor or delete a same-title sibling episode"
    );
}

#[tokio::test]
async fn a_fresh_re_monitor_cancels_the_sequence_delete_files_step_and_preserves_files() {
    let fixture = execution_app(None);
    let target = seed_title(&fixture.app, &fixture.user, "Re-monitored target", true).await;
    let tempdir = tempfile::tempdir().expect("tempdir");
    let target_path = tempdir.path().join("target/movie.mkv");
    let target_file_id = add_movie_file(&fixture.app, &target.id, &target_path).await;
    let (_rule_set_id, candidate) =
        arm_and_evaluate(&fixture, sequence_draft(deletion_steps())).await;
    let sequence = MaintenanceActionSequence::new(deletion_steps());

    fixture
        .app
        .set_title_monitored(&fixture.user, &target.id, false)
        .await
        .expect("model the completed unmonitor step");
    persist_completed_unmonitor(&fixture, &candidate, &sequence).await;
    fixture
        .app
        .set_title_monitored(&fixture.user, &target.id, true)
        .await
        .expect("external re-monitor before deletion");
    fixture
        .app
        .set_maintenance_rule_arming(
            &fixture.user,
            &_rule_set_id,
            MaintenanceEffectArming::Destructive,
            Some(1),
        )
        .await
        .expect("arm destructive sequence");

    let report = fixture
        .app
        .run_lifecycle_action_handling_job()
        .await
        .expect("refuse stale destructive prerequisite");
    assert_eq!(report.canceled, 1, "{report:?}");
    assert!(target_path.exists(), "re-monitor preserves the file");
    assert!(
        fixture
            .app
            .services
            .library
            .media_files
            .get_media_file_by_id(&target_file_id)
            .await
            .expect("read preserved target row")
            .is_some()
    );
}

#[tokio::test]
async fn storage_sequence_holds_its_journal_when_target_or_root_identity_changes() {
    let fixture = title_fixture("Sequence storage drift", MediaFacet::Movie).await;
    let first_path = fixture.root.join("Sequence/first.mkv");
    let first_file_id =
        add_movie_file(&fixture.execution.app, &fixture.title.id, &first_path).await;
    let second_path = fixture.root.join("Sequence/second.mkv");
    let second_file_id =
        add_movie_file(&fixture.execution.app, &fixture.title.id, &second_path).await;
    let mut ordered_targets = [(first_file_id, first_path), (second_file_id, second_path)];
    ordered_targets.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    let [(first_file_id, first_path), (second_file_id, second_path)] = ordered_targets;
    let (probe, capacity_recovered) = capacity_holds_after_file_removal(first_path.clone());
    let _probe = install_maintenance_storage_capacity_probe_for_test(probe).await;
    let root_id = selected_root_id(&fixture).await;
    let mut draft = sequence_draft(deletion_steps());
    draft.storage_root_id = Some(root_id);
    let (rule_set_id, candidate) = arm_and_evaluate(&fixture.execution, draft).await;
    fixture
        .execution
        .app
        .set_maintenance_rule_arming(
            &fixture.execution.user,
            &rule_set_id,
            MaintenanceEffectArming::Destructive,
            Some(1),
        )
        .await
        .expect("arm destructive storage sequence");

    let first = fixture
        .execution
        .app
        .run_lifecycle_action_handling_job()
        .await
        .expect("persist storage sequence journal before capacity hold");
    assert_eq!(first.held, 1, "{first:?}");
    let deletion_run = fixture
        .execution
        .evaluation
        .latest_scoped_deletion_action_run(
            &candidate.id,
            candidate.match_generation,
            &candidate.action_kind,
        )
        .await
        .expect("read exact scoped deletion journal")
        .expect("DeleteFiles hold persists its scoped journal");
    assert_eq!(deletion_run.status, LifecycleActionRunStatus::Held);
    let journal: serde_json::Value =
        serde_json::from_str(&deletion_run.detail).expect("parse storage journal");
    assert_eq!(journal["sequence_step_id"].as_str(), Some("delete-files"));
    let journal_files = journal["files"].as_array().expect("journal files");
    assert_eq!(
        journal_files
            .iter()
            .find(|file| file["file_id"].as_str() == Some(first_file_id.as_str()))
            .and_then(|file| file["completed"].as_bool()),
        Some(true),
        "the first policy target completed before capacity became unknown"
    );
    assert_eq!(
        journal_files
            .iter()
            .find(|file| file["file_id"].as_str() == Some(second_file_id.as_str()))
            .and_then(|file| file["completed"].as_bool()),
        Some(false),
        "the second target remains journaled for the held recovery"
    );

    let added_target = fixture.root.join("Sequence/external-after-journal.mkv");
    let added_target_id =
        add_movie_file(&fixture.execution.app, &fixture.title.id, &added_target).await;
    let replacement_root = fixture
        .root
        .parent()
        .expect("fixture root parent")
        .join("replacement-root");
    std::fs::create_dir_all(&replacement_root).expect("create replacement root");
    fixture
        .execution
        .app
        .update_media_settings(
            &fixture.execution.user,
            MediaFacet::Movie,
            empty_update_media_settings_with_roots(vec![build_root_folder_entry(
                &replacement_root,
                true,
            )]),
        )
        .await
        .expect("replace selected storage root");
    capacity_recovered.store(true, Ordering::SeqCst);
    fixture
        .execution
        .app
        .run_maintenance_rule_evaluation_job()
        .await
        .expect("refresh matching candidate");

    let resumed = fixture
        .execution
        .app
        .run_lifecycle_action_handling_job()
        .await
        .expect("hold an invalidated storage journal");
    assert_eq!(resumed.held, 1, "{resumed:?}");
    assert!(
        !first_path.exists(),
        "the completed policy target stays deleted after drift"
    );
    assert!(second_path.exists(), "journaled target remains preserved");
    assert!(added_target.exists(), "new target remains preserved");
    assert!(
        fixture
            .execution
            .app
            .services
            .library
            .media_files
            .get_media_file_by_id(&first_file_id)
            .await
            .expect("read completed storage row")
            .is_none()
    );
    for file_id in [&second_file_id, &added_target_id] {
        assert!(
            fixture
                .execution
                .app
                .services
                .library
                .media_files
                .get_media_file_by_id(file_id)
                .await
                .expect("read preserved storage row")
                .is_some()
        );
    }
    let steps = fixture
        .execution
        .evaluation
        .list_action_steps(
            &candidate.id,
            candidate.match_generation,
            candidate.revision_number,
        )
        .await
        .expect("read sequence steps");
    assert_eq!(
        steps
            .iter()
            .find(|step| step.key.step_id == "unmonitor")
            .expect("persisted Unmonitor step")
            .state,
        MaintenanceActionStepState::Succeeded
    );
    assert_eq!(
        steps
            .iter()
            .find(|step| step.key.step_id == "delete-files")
            .expect("persisted DeleteFiles step")
            .state,
        MaintenanceActionStepState::Held
    );
}

#[tokio::test]
async fn completed_storage_sequence_releases_its_exact_marker_after_root_filter_omits_it() {
    let _probe =
        install_maintenance_storage_capacity_probe_for_test(stable_capacity_probe(capacity(100)))
            .await;
    let fixture = title_fixture("Sequence re-import", MediaFacet::Movie).await;
    let first_path = fixture.root.join("Sequence/first.mkv");
    add_movie_file(&fixture.execution.app, &fixture.title.id, &first_path).await;
    let root_id = selected_root_id(&fixture).await;
    let mut draft = sequence_draft(deletion_steps());
    draft.storage_root_id = Some(root_id);
    let (rule_set_id, candidate) = arm_and_evaluate(&fixture.execution, draft).await;
    fixture
        .execution
        .app
        .set_maintenance_rule_arming(
            &fixture.execution.user,
            &rule_set_id,
            MaintenanceEffectArming::Destructive,
            Some(1),
        )
        .await
        .expect("arm destructive storage sequence");

    let executed = fixture
        .execution
        .app
        .run_lifecycle_action_handling_job()
        .await
        .expect("complete selected-root deletion");
    assert_eq!(executed.executed, 1, "{executed:?}");
    assert!(
        fixture
            .execution
            .evaluation
            .get_active_sequence_completion(
                &candidate.rule_set_id,
                candidate.revision_number,
                &candidate.subject_kind,
                &candidate.subject_id,
            )
            .await
            .expect("read terminal membership")
            .is_some(),
        "the completed sequence must latch continuous membership before reconciliation"
    );

    fixture
        .execution
        .app
        .run_maintenance_rule_evaluation_job()
        .await
        .expect("reconcile the root-omitted subject");
    assert!(
        fixture
            .execution
            .evaluation
            .get_active_sequence_completion(
                &candidate.rule_set_id,
                candidate.revision_number,
                &candidate.subject_kind,
                &candidate.subject_id,
            )
            .await
            .expect("read released terminal membership")
            .is_none(),
        "confirmed absence from the selected root releases only this completed deletion marker"
    );

    let reimported = fixture.root.join("Sequence/reimported.mkv");
    add_movie_file(&fixture.execution.app, &fixture.title.id, &reimported).await;
    fixture
        .execution
        .app
        .run_maintenance_rule_evaluation_job()
        .await
        .expect("evaluate the exact re-import");
    let reopened = fixture
        .execution
        .evaluation
        .get_active_subject_candidate(
            &candidate.rule_set_id,
            &candidate.subject_kind,
            &candidate.subject_id,
        )
        .await
        .expect("read re-import candidate")
        .expect("a re-import opens a new grace generation");
    assert_eq!(reopened.match_generation, candidate.match_generation + 1);
}

#[tokio::test]
async fn unmonitor_only_storage_sequence_releases_after_its_catalog_membership_disappears() {
    let _probe =
        install_maintenance_storage_capacity_probe_for_test(stable_capacity_probe(capacity(100)))
            .await;
    let fixture = title_fixture("Sequence unmonitor omission", MediaFacet::Movie).await;
    let file_path = fixture.root.join("Sequence/unmonitor-only.mkv");
    let file_id = add_movie_file(&fixture.execution.app, &fixture.title.id, &file_path).await;
    let mut steps = deletion_steps();
    steps.pop();
    let sequence = MaintenanceActionSequence {
        schema_version: 2,
        steps: steps.clone(),
    };
    let mut draft = sequence_draft(steps);
    draft.storage_root_id = Some(selected_root_id(&fixture).await);
    let (_rule_set_id, candidate) = arm_and_evaluate(&fixture.execution, draft).await;
    fixture
        .execution
        .evaluation
        .seed_sequence_terminal_membership(terminal_membership(
            "unmonitor-only-terminal",
            &candidate,
            &sequence,
            MaintenanceSequenceTerminalOutcome::Succeeded,
        ))
        .await;
    fixture
        .execution
        .app
        .services
        .library
        .media_files
        .delete_media_file(&file_id)
        .await
        .expect("remove externally absent catalog membership");

    fixture
        .execution
        .app
        .run_maintenance_rule_evaluation_job()
        .await
        .expect("reconcile unmonitor-only storage omission");

    assert!(
        fixture
            .execution
            .evaluation
            .get_active_sequence_completion(
                &candidate.rule_set_id,
                candidate.revision_number,
                &candidate.subject_kind,
                &candidate.subject_id,
            )
            .await
            .expect("read unmonitor-only terminal membership")
            .is_none(),
        "a storage-scoped sequence without DeleteFiles still releases after its exact subject vanishes"
    );
}

#[tokio::test]
async fn failed_storage_sequence_releases_after_its_exact_subject_is_omitted() {
    let _probe =
        install_maintenance_storage_capacity_probe_for_test(stable_capacity_probe(capacity(100)))
            .await;
    let fixture = title_fixture("Sequence failed omission", MediaFacet::Movie).await;
    let file_path = fixture.root.join("Sequence/failed.mkv");
    let file_id = add_movie_file(&fixture.execution.app, &fixture.title.id, &file_path).await;
    let steps = deletion_steps();
    let sequence = MaintenanceActionSequence {
        schema_version: 2,
        steps: steps.clone(),
    };
    let mut draft = sequence_draft(steps);
    draft.storage_root_id = Some(selected_root_id(&fixture).await);
    let (_rule_set_id, candidate) = arm_and_evaluate(&fixture.execution, draft).await;
    fixture
        .execution
        .evaluation
        .seed_sequence_terminal_membership(terminal_membership(
            "failed-terminal",
            &candidate,
            &sequence,
            MaintenanceSequenceTerminalOutcome::Failed,
        ))
        .await;
    fixture
        .execution
        .app
        .services
        .library
        .media_files
        .delete_media_file(&file_id)
        .await
        .expect("remove externally absent catalog membership");

    fixture
        .execution
        .app
        .run_maintenance_rule_evaluation_job()
        .await
        .expect("reconcile failed storage omission");

    assert!(
        fixture
            .execution
            .evaluation
            .get_active_sequence_completion(
                &candidate.rule_set_id,
                candidate.revision_number,
                &candidate.subject_kind,
                &candidate.subject_id,
            )
            .await
            .expect("read failed terminal membership")
            .is_none(),
        "a failed terminal membership does not keep an absent storage subject latched forever"
    );
}

#[tokio::test]
async fn storage_omission_sweep_reaches_a_terminal_marker_after_one_hundred_retained_rows() {
    let _probe =
        install_maintenance_storage_capacity_probe_for_test(stable_capacity_probe(capacity(100)))
            .await;
    let fixture = title_fixture("Sequence keyset omission", MediaFacet::Movie).await;
    let file_path = fixture.root.join("Sequence/keyset.mkv");
    let file_id = add_movie_file(&fixture.execution.app, &fixture.title.id, &file_path).await;
    let mut steps = deletion_steps();
    steps.pop();
    let sequence = MaintenanceActionSequence {
        schema_version: 2,
        steps: steps.clone(),
    };
    let mut draft = sequence_draft(steps);
    draft.storage_root_id = Some(selected_root_id(&fixture).await);
    let (_rule_set_id, candidate) = arm_and_evaluate(&fixture.execution, draft).await;

    for index in 0..100 {
        let mut retained = terminal_membership(
            format!("000-retained-{index:03}"),
            &candidate,
            &sequence,
            MaintenanceSequenceTerminalOutcome::Succeeded,
        );
        retained.sequence_content_hash = "obsolete-sequence-hash".to_string();
        retained.subject_id = format!("retained-subject-{index:03}");
        fixture
            .execution
            .evaluation
            .seed_sequence_terminal_membership(retained)
            .await;
    }
    fixture
        .execution
        .evaluation
        .seed_sequence_terminal_membership(terminal_membership(
            "999-releasable-target",
            &candidate,
            &sequence,
            MaintenanceSequenceTerminalOutcome::Succeeded,
        ))
        .await;
    fixture
        .execution
        .app
        .services
        .library
        .media_files
        .delete_media_file(&file_id)
        .await
        .expect("remove root membership before paged sweep");

    fixture
        .execution
        .app
        .run_maintenance_rule_evaluation_job()
        .await
        .expect("sweep every active terminal-marker page");

    assert!(
        fixture
            .execution
            .evaluation
            .get_active_sequence_completion(
                &candidate.rule_set_id,
                candidate.revision_number,
                &candidate.subject_kind,
                &candidate.subject_id,
            )
            .await
            .expect("read terminal marker after keyset sweep")
            .is_none(),
        "the marker after a full retained page must not starve behind it"
    );
}

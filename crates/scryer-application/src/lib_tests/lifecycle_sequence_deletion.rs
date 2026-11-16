//! Sequence-only deletion regressions over the real scoped-deletion journal.

use super::*;

use crate::lib_tests::lifecycle_execution::{ExecutionFixture, execution_app, seed_title};
use crate::lib_tests::lifecycle_sequence_execution::{
    arm_and_evaluate, arm_and_evaluate_all, sequence_draft, with_matcher,
};
use crate::lib_tests::lifecycle_title_completion::{
    TitleFixture, add_episode_file, season_and_episode, title_fixture,
};
use crate::lifecycle_rules::storage::{
    TestCapacityProbe, install_lifecycle_storage_capacity_probe_for_test,
};
use crate::lifecycle_rules::{
    LifecycleActionSequence, LifecycleActionStep, LifecycleActionStepKind,
    LifecycleActionStepParameters,
};
use crate::ports::{
    LifecycleActionRunRepository, LifecycleActionStepRepository, LifecycleCandidateRepository,
    LifecycleSequenceCompletionRepository,
};
use chrono::{Duration, Utc};
use scryer_domain::{
    LifecycleActionRunStatus, LifecycleActionStepKey, LifecycleActionStepRun,
    LifecycleActionStepState, LifecycleEffectArming, LifecycleSequenceTerminalMembership,
    LifecycleSequenceTerminalOutcome,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) fn deletion_steps() -> Vec<LifecycleActionStep> {
    vec![
        LifecycleActionStep {
            id: "unmonitor".to_string(),
            kind: LifecycleActionStepKind::Unmonitor,
            parameters: LifecycleActionStepParameters::Unmonitor {
                include_descendants: true,
            },
        },
        LifecycleActionStep {
            id: "delete_files".to_string(),
            kind: LifecycleActionStepKind::DeleteFiles,
            parameters: LifecycleActionStepParameters::None,
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
    super::lifecycle_title_completion::record_fixture_title_folder(
        app,
        title_id,
        path.parent().unwrap(),
    )
    .await;
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

/// A matcher that reads the subject's storage facts and holds every title
/// whose files sit on a configured root, so deletion rechecks capacity per
/// file.
pub(super) const STORAGE_MATCHER: &str =
    "match if {\n\tsome root in input.facts.storage_roots\n\troot.available_percent >= 0\n}\n";

pub(super) fn storage_sequence_deletion_draft() -> crate::lifecycle_rules::LifecycleRuleDraft {
    let mut draft = sequence_draft(deletion_steps());
    with_matcher(&mut draft, STORAGE_MATCHER);
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
    sequence: &LifecycleActionSequence,
) {
    let step = sequence
        .steps
        .iter()
        .find(|step| step.kind == LifecycleActionStepKind::Unmonitor)
        .expect("unmonitor step");
    let now = Utc::now();
    let run = LifecycleActionStepRun {
        key: LifecycleActionStepKey {
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
        state: LifecycleActionStepState::Running,
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
        crate::LifecycleActionStepClaim::Claimed(run) => run,
        other => panic!("unexpected unmonitor claim: {other:?}"),
    };
    claimed.state = LifecycleActionStepState::Succeeded;
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
    sequence: &LifecycleActionSequence,
    outcome: LifecycleSequenceTerminalOutcome,
) -> LifecycleSequenceTerminalMembership {
    let expected_step_ids: Vec<_> = sequence.steps.iter().map(|step| step.id.clone()).collect();
    let terminal_step_id = match outcome {
        LifecycleSequenceTerminalOutcome::Succeeded => None,
        LifecycleSequenceTerminalOutcome::Failed => expected_step_ids.last().cloned(),
    };
    LifecycleSequenceTerminalMembership {
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
    super::lifecycle_rules::arm_rule(
        &fixture.execution.app,
        &fixture.execution.user,
        &rule_set_id,
        LifecycleEffectArming::Destructive,
        Some(1),
    )
    .await;

    let report = fixture
        .execution
        .app
        .run_every_rule_action_pass_for_tests()
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
        .list_lifecycle_action_runs(
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
    super::lifecycle_sequence_execution::with_matcher(
        &mut draft,
        &format!(
            "package selected_episode\nimport rego.v1\n\nmatch if {{\n  input.episode.id == {:?}\n}}\n",
            selected.id
        ),
    );
    draft.rego_source = crate::lib_tests::episode_heads(&draft.rego_source);
    let (rule_set_id, candidate) = arm_and_evaluate(&fixture.execution, draft).await;
    assert_eq!(candidate.subject_id, selected.id);
    super::lifecycle_rules::arm_rule(
        &fixture.execution.app,
        &fixture.execution.user,
        &rule_set_id,
        LifecycleEffectArming::Destructive,
        Some(1),
    )
    .await;

    let report = fixture
        .execution
        .app
        .run_every_rule_action_pass_for_tests()
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
    let tempdir = tempfile::tempdir().expect("tempdir");
    update_default_library_roots(
        &fixture.app,
        &fixture.user,
        MediaFacet::Movie,
        vec![build_root_folder_entry(tempdir.path(), true)],
    )
    .await
    .expect("configure isolated movie root");
    let target = seed_title(&fixture.app, &fixture.user, "Re-monitored target", true).await;
    let target_path = tempdir.path().join("target/movie.mkv");
    let target_file_id = add_movie_file(&fixture.app, &target.id, &target_path).await;
    let (_rule_set_id, candidate) =
        arm_and_evaluate(&fixture, sequence_draft(deletion_steps())).await;
    let sequence = LifecycleActionSequence::new(deletion_steps());

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
    super::lifecycle_rules::arm_rule(
        &fixture.app,
        &fixture.user,
        &_rule_set_id,
        LifecycleEffectArming::Destructive,
        Some(1),
    )
    .await;

    let report = fixture
        .app
        .run_every_rule_action_pass_for_tests()
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

/// A journal written while a rule could select one configured root never
/// resumes: deletion no longer narrows to a root, so resuming it would act
/// over files the journal never covered. It holds with the journal kept.
#[tokio::test]
async fn a_journal_written_for_one_root_holds_and_never_resumes() {
    let fixture = title_fixture("Sequence root journal", MediaFacet::Movie).await;
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
    let _probe = install_lifecycle_storage_capacity_probe_for_test(probe).await;
    let (rule_set_id, candidate) =
        arm_and_evaluate(&fixture.execution, storage_sequence_deletion_draft()).await;
    super::lifecycle_rules::arm_rule(
        &fixture.execution.app,
        &fixture.execution.user,
        &rule_set_id,
        LifecycleEffectArming::Destructive,
        Some(1),
    )
    .await;

    let first = fixture
        .execution
        .app
        .run_every_rule_action_pass_for_tests()
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
    assert!(!first_path.exists(), "the first target completed");
    assert!(second_path.exists(), "the second target is journaled");

    // Rewrite the journal as one written for a single selected root.
    let mut journal: serde_json::Value =
        serde_json::from_str(&deletion_run.detail).expect("parse storage journal");
    journal["storage_root_id"] = serde_json::json!("synthetic-selected-root");
    journal["storage_root_identity"] = serde_json::json!("synthetic-root-identity");
    let mut rooted = deletion_run.clone();
    rooted.detail = journal.to_string();
    fixture
        .execution
        .evaluation
        .finish_action_run(&rooted)
        .await
        .expect("store the root journal");

    capacity_recovered.store(true, Ordering::SeqCst);
    for pass in ["first pass", "second pass"] {
        fixture.execution.evaluation.begin_next_evaluation().await;
        let report = fixture
            .execution
            .app
            .run_every_rule_action_pass_for_tests()
            .await
            .expect("hold a root journal");
        assert_eq!(report.held, 1, "{pass}: {report:?}");
        assert_eq!(report.executed, 0, "{pass}: {report:?}");
        let held_run = fixture
            .execution
            .evaluation
            .latest_scoped_deletion_action_run(
                &candidate.id,
                candidate.match_generation,
                &candidate.action_kind,
            )
            .await
            .expect("read held scoped deletion run")
            .expect("held DeleteFiles run persists");
        assert_eq!(held_run.id, deletion_run.id, "{pass}");
        let held_journal: serde_json::Value =
            serde_json::from_str(&held_run.detail).expect("parse held journal");
        assert_eq!(
            held_journal["storage_root_id"], journal["storage_root_id"],
            "{pass}"
        );
        assert_eq!(held_journal["files"], journal["files"], "{pass}");
        assert!(
            second_path.exists(),
            "{pass}: the journaled target is preserved"
        );
        assert!(
            fixture
                .execution
                .app
                .services
                .library
                .media_files
                .get_media_file_by_id(&second_file_id)
                .await
                .expect("read preserved storage row")
                .is_some(),
            "{pass}"
        );
    }
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
}

#[tokio::test]
async fn completed_storage_sequence_releases_its_exact_marker_after_root_filter_omits_it() {
    let _probe =
        install_lifecycle_storage_capacity_probe_for_test(stable_capacity_probe(capacity(100)))
            .await;
    let fixture = title_fixture("Sequence re-import", MediaFacet::Movie).await;
    let first_path = fixture.root.join("Sequence/first.mkv");
    add_movie_file(&fixture.execution.app, &fixture.title.id, &first_path).await;
    let draft = storage_sequence_deletion_draft();
    let (rule_set_id, candidate) = arm_and_evaluate(&fixture.execution, draft).await;
    super::lifecycle_rules::arm_rule(
        &fixture.execution.app,
        &fixture.execution.user,
        &rule_set_id,
        LifecycleEffectArming::Destructive,
        Some(1),
    )
    .await;

    let executed = fixture
        .execution
        .app
        .run_every_rule_action_pass_for_tests()
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
        .run_lifecycle_rule_evaluation_job()
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
        .run_lifecycle_rule_evaluation_job()
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
        install_lifecycle_storage_capacity_probe_for_test(stable_capacity_probe(capacity(100)))
            .await;
    let fixture = title_fixture("Sequence unmonitor omission", MediaFacet::Movie).await;
    let file_path = fixture.root.join("Sequence/unmonitor-only.mkv");
    let file_id = add_movie_file(&fixture.execution.app, &fixture.title.id, &file_path).await;
    let mut steps = deletion_steps();
    steps.pop();
    let sequence = LifecycleActionSequence {
        schema_version: 2,
        steps: steps.clone(),
    };
    let mut draft = sequence_draft(steps);
    with_matcher(&mut draft, STORAGE_MATCHER);
    let (_rule_set_id, candidate) = arm_and_evaluate(&fixture.execution, draft).await;
    fixture
        .execution
        .evaluation
        .seed_sequence_terminal_membership(terminal_membership(
            "unmonitor-only-terminal",
            &candidate,
            &sequence,
            LifecycleSequenceTerminalOutcome::Succeeded,
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
        .run_lifecycle_rule_evaluation_job()
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
        install_lifecycle_storage_capacity_probe_for_test(stable_capacity_probe(capacity(100)))
            .await;
    let fixture = title_fixture("Sequence failed omission", MediaFacet::Movie).await;
    let file_path = fixture.root.join("Sequence/failed.mkv");
    let file_id = add_movie_file(&fixture.execution.app, &fixture.title.id, &file_path).await;
    let steps = deletion_steps();
    let sequence = LifecycleActionSequence {
        schema_version: 2,
        steps: steps.clone(),
    };
    let mut draft = sequence_draft(steps);
    with_matcher(&mut draft, STORAGE_MATCHER);
    let (_rule_set_id, candidate) = arm_and_evaluate(&fixture.execution, draft).await;
    fixture
        .execution
        .evaluation
        .seed_sequence_terminal_membership(terminal_membership(
            "failed-terminal",
            &candidate,
            &sequence,
            LifecycleSequenceTerminalOutcome::Failed,
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
        .run_lifecycle_rule_evaluation_job()
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
        install_lifecycle_storage_capacity_probe_for_test(stable_capacity_probe(capacity(100)))
            .await;
    let fixture = title_fixture("Sequence keyset omission", MediaFacet::Movie).await;
    let file_path = fixture.root.join("Sequence/keyset.mkv");
    let file_id = add_movie_file(&fixture.execution.app, &fixture.title.id, &file_path).await;
    let mut steps = deletion_steps();
    steps.pop();
    let sequence = LifecycleActionSequence {
        schema_version: 2,
        steps: steps.clone(),
    };
    let mut draft = sequence_draft(steps);
    with_matcher(&mut draft, STORAGE_MATCHER);
    let (_rule_set_id, candidate) = arm_and_evaluate(&fixture.execution, draft).await;

    for index in 0..100 {
        let mut retained = terminal_membership(
            format!("000-retained-{index:03}"),
            &candidate,
            &sequence,
            LifecycleSequenceTerminalOutcome::Succeeded,
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
            LifecycleSequenceTerminalOutcome::Succeeded,
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
        .run_lifecycle_rule_evaluation_job()
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

// ── A deleting rule acts on at most the titles its operator chose per run ───

/// Five synthetic movies, each with one file on the fixture root, under one
/// armed deleting rule with the given maximum. A stray file beside them that
/// no title owns must outlive every run.
async fn deleting_rule_over_five(
    max_titles_per_run: u32,
) -> (TitleFixture, String, Vec<PathBuf>, PathBuf) {
    let fixture = title_fixture("Synthetic retire 0", MediaFacet::Movie).await;
    let mut paths = vec![fixture.root.join("retire-0/movie.mkv")];
    add_movie_file(&fixture.execution.app, &fixture.title.id, &paths[0]).await;
    for index in 1..5 {
        let title = seed_title(
            &fixture.execution.app,
            &fixture.execution.user,
            &format!("Synthetic retire {index}"),
            true,
        )
        .await;
        let path = fixture.root.join(format!("retire-{index}/movie.mkv"));
        add_movie_file(&fixture.execution.app, &title.id, &path).await;
        paths.push(path);
    }
    let stray = fixture.root.join("unowned/stray.mkv");
    write_media_file(&stray);
    let (rule_set_id, candidates) = arm_and_evaluate_all(
        &fixture.execution,
        crate::lifecycle_rules::LifecycleRuleDraft {
            max_titles_per_run: Some(max_titles_per_run),
            ..sequence_draft(deletion_steps())
        },
    )
    .await;
    assert_eq!(candidates.len(), 5, "{candidates:?}");
    super::lifecycle_rules::arm_rule(
        &fixture.execution.app,
        &fixture.execution.user,
        &rule_set_id,
        LifecycleEffectArming::Destructive,
        Some(5),
    )
    .await;
    (fixture, rule_set_id, paths, stray)
}

fn remaining(paths: &[PathBuf]) -> usize {
    paths.iter().filter(|path| path.exists()).count()
}

#[tokio::test]
async fn a_deleting_rule_stops_at_its_maximum_and_the_next_fire_continues() {
    let (fixture, _rule_set_id, paths, stray) = deleting_rule_over_five(2).await;
    let before = fixture.execution.evaluation.all_candidates().await;

    let first = fixture
        .execution
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("first fire");
    assert_eq!(first.executed, 2, "{first:?}");
    assert_eq!(first.held, 0, "past the maximum is not a hold: {first:?}");
    assert_eq!(first.rules_stopped_at_maximum, 1, "{first:?}");
    assert_eq!(remaining(&paths), 3, "files past the maximum are untouched");
    assert!(stray.exists(), "a file no title owns is never touched");
    // The three left over are exactly as evaluation left them: no hold, no
    // reason, no stamp, no history.
    let after = fixture.execution.evaluation.all_candidates().await;
    let untouched = after
        .iter()
        .filter(|candidate| {
            before.iter().any(|seen| {
                seen.id == candidate.id
                    && seen.state == candidate.state
                    && seen.state_reason == candidate.state_reason
                    && seen.updated_at == candidate.updated_at
            })
        })
        .count();
    assert_eq!(untouched, 3, "{after:?}");
    let acted_on: std::collections::HashSet<String> = fixture
        .execution
        .evaluation
        .all_action_runs()
        .await
        .into_iter()
        .map(|run| run.candidate_id)
        .collect();
    assert_eq!(acted_on.len(), 2, "history names only the titles acted on");

    let second = fixture
        .execution
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("second fire");
    assert_eq!(second.executed, 2, "{second:?}");
    assert_eq!(remaining(&paths), 1);

    let third = fixture
        .execution
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("third fire");
    assert_eq!(third.executed, 1, "{third:?}");
    assert_eq!(third.rules_stopped_at_maximum, 0, "{third:?}");
    assert_eq!(remaining(&paths), 0);
    assert!(stray.exists());
}

/// The instance-wide job, scheduled or run by hand, never acts for a rule: a
/// rule's maximum bounds everything done between two of its own fires.
#[tokio::test]
async fn the_instance_wide_job_never_acts_and_one_fire_stops_at_the_maximum() {
    let (fixture, rule_set_id, paths, stray) = deleting_rule_over_five(2).await;
    let app = &fixture.execution.app;

    for _ in 0..3 {
        let report = app
            .run_lifecycle_action_handling_job()
            .await
            .expect("instance-wide pass");
        assert_eq!(report.executed, 0, "{report:?}");
        app.run_scheduled_job_now(JobKey::LifecycleActionHandling, JobTriggerSource::Manual)
            .await
            .expect("instance-wide job run by hand");
    }
    assert_eq!(
        remaining(&paths),
        5,
        "no instance-wide pass deletes anything"
    );

    let fire = app
        .run_lifecycle_action_handling(Some(&rule_set_id))
        .await
        .expect("the rule's own fire");
    assert_eq!(fire.executed, 2, "{fire:?}");
    assert_eq!(remaining(&paths), 3);

    for _ in 0..3 {
        app.run_lifecycle_action_handling_job()
            .await
            .expect("instance-wide pass after the fire");
        app.run_scheduled_job_now(JobKey::LifecycleActionHandling, JobTriggerSource::Manual)
            .await
            .expect("instance-wide job run by hand after the fire");
    }
    assert_eq!(
        remaining(&paths),
        3,
        "one fire plus any number of instance-wide passes acts on at most the maximum"
    );
    assert!(stray.exists(), "a file no title owns is never touched");
}

/// A rule on a manual schedule acts only when that rule is run by hand. A
/// request to act now that names no rule is refused with its code.
#[tokio::test]
async fn a_manual_schedule_rule_acts_only_through_its_own_trigger() {
    let (fixture, rule_set_id, paths, _stray) = deleting_rule_over_five(5).await;
    let app = &fixture.execution.app;
    let user = &fixture.execution.user;
    let rule = app
        .require_lifecycle_rule_set(&rule_set_id)
        .await
        .expect("rule");
    app.update_lifecycle_rule_metadata_with_schedule(
        user,
        &rule_set_id,
        rule.name.clone(),
        rule.description.clone(),
        rule.library_ids.clone(),
        Some(scryer_domain::ScriptSchedule::Manual),
        None,
    )
    .await
    .expect("manual schedule");

    app.run_lifecycle_action_handling_job()
        .await
        .expect("instance-wide pass");
    app.run_scheduled_job_now(JobKey::LifecycleActionHandling, JobTriggerSource::Manual)
        .await
        .expect("instance-wide job run by hand");
    assert_eq!(remaining(&paths), 5, "the instance-wide job ran no rule");

    let refused = app
        .run_lifecycle_action_handler_now(user, None)
        .await
        .expect_err("a run that names no rule is refused");
    assert_eq!(
        refused.validation_reason(),
        Some(crate::error_reasons::lifecycle::ACTION_RUN_RULE_REQUIRED)
    );
    assert_eq!(remaining(&paths), 5);

    // The rule's own trigger acts. It runs under the rule's claim, so a
    // second trigger while it runs is turned away; the run is awaited here
    // through the same pass the trigger spawns.
    let _claim = app
        .claim_lifecycle_rule_run(&rule_set_id)
        .expect("the rule is idle");
    let busy = app
        .run_lifecycle_action_handler_now(user, Some(rule_set_id.clone()))
        .await
        .expect("trigger while the rule runs");
    assert!(!busy.started, "{busy:?}");
    let fired = app
        .run_lifecycle_action_handling(Some(&rule_set_id))
        .await
        .expect("the rule's manual run");
    assert_eq!(fired.executed, 5, "{fired:?}");
    assert_eq!(remaining(&paths), 0);
}

#[tokio::test]
async fn a_very_large_maximum_acts_on_every_due_title() {
    let (fixture, _rule_set_id, paths, stray) = deleting_rule_over_five(u32::MAX).await;

    let report = fixture
        .execution
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("one fire");
    assert_eq!(report.executed, 5, "{report:?}");
    assert_eq!(report.rules_stopped_at_maximum, 0, "{report:?}");
    assert_eq!(remaining(&paths), 0);
    assert!(stray.exists(), "a file no title owns is never touched");
}

/// A deleting rule that is armed but somehow stored without a maximum shows
/// the refusal by code, still evaluates and previews, and acts on nothing.
#[tokio::test]
async fn an_armed_deleting_rule_without_a_maximum_shows_its_refusal_and_acts_on_nothing() {
    use crate::error_reasons::lifecycle::RULE_ARMED_WITHOUT_MAX_TITLES_PER_RUN;
    use crate::lifecycle_rules::{
        LifecyclePreviewMatcher, LifecyclePreviewRequest, LifecyclePreviewSelection,
    };
    use crate::ports::LifecycleRuleSetRepository;

    let (fixture, rule_set_id, paths, stray) = deleting_rule_over_five(2).await;
    let app = &fixture.execution.app;
    let user = &fixture.execution.user;
    let rule = app
        .require_lifecycle_rule_set(&rule_set_id)
        .await
        .expect("rule");
    let armed = app
        .load_lifecycle_rule_detail(rule.clone())
        .await
        .expect("detail");
    assert_eq!(armed.refusal(), None, "a rule with a maximum is usable");

    // Saving refuses this shape, so the row is written beneath the service.
    fixture
        .execution
        .rules
        .update_rule_set_metadata(
            &rule_set_id,
            &rule.name,
            &rule.description,
            &rule.library_ids,
            None,
            Some(None),
            false,
            Utc::now(),
        )
        .await
        .expect("clear the maximum in the store");
    let rule = app
        .require_lifecycle_rule_set(&rule_set_id)
        .await
        .expect("rule");
    assert_eq!(rule.effect_arming, LifecycleEffectArming::Destructive);
    let detail = app.load_lifecycle_rule_detail(rule).await.expect("detail");
    assert_eq!(detail.load_refusal, None, "evaluation is not refused");
    assert_eq!(
        detail.action_refusal,
        Some(RULE_ARMED_WITHOUT_MAX_TITLES_PER_RUN)
    );
    assert_eq!(
        detail.refusal(),
        Some(RULE_ARMED_WITHOUT_MAX_TITLES_PER_RUN)
    );

    app.run_lifecycle_rule_evaluation_job()
        .await
        .expect("the rule still evaluates");
    let preview = app
        .preview_lifecycle_rule(
            user,
            LifecyclePreviewRequest {
                matcher: LifecyclePreviewMatcher::Stored {
                    rule_set_id: rule_set_id.clone(),
                },
                selection: LifecyclePreviewSelection::Titles(vec![fixture.title.id.clone()]),
            },
        )
        .await
        .expect("the rule still previews");
    assert_eq!(preview.titles.len(), 1);

    let report = app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("fire");
    assert_eq!(report.executed, 0, "{report:?}");
    let refused = app
        .run_lifecycle_action_handler_now(user, Some(rule_set_id.clone()))
        .await
        .expect_err("acting now is refused");
    assert_eq!(
        refused.validation_reason(),
        Some(RULE_ARMED_WITHOUT_MAX_TITLES_PER_RUN)
    );
    assert_eq!(remaining(&paths), 5, "nothing is deleted");
    assert!(stray.exists());
}

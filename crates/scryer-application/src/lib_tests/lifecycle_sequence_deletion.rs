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
    persist_succeeded_step(
        fixture,
        candidate,
        sequence,
        LifecycleActionStepKind::Unmonitor,
    )
    .await;
}

async fn persist_succeeded_step(
    fixture: &ExecutionFixture,
    candidate: &scryer_domain::LifecycleCandidate,
    sequence: &LifecycleActionSequence,
    kind: LifecycleActionStepKind,
) {
    persist_succeeded_step_with_before(
        fixture,
        candidate,
        sequence,
        kind,
        serde_json::json!({
            "schema_version": 1,
            "facts_monitored": true,
            "monitoring_changed": true,
        }),
    )
    .await;
}

async fn persist_succeeded_step_with_before(
    fixture: &ExecutionFixture,
    candidate: &scryer_domain::LifecycleCandidate,
    sequence: &LifecycleActionSequence,
    kind: LifecycleActionStepKind,
    before_state: serde_json::Value,
) {
    let step = sequence
        .steps
        .iter()
        .find(|step| step.kind == kind)
        .expect("sequence step");
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
        before_state_json: before_state.to_string(),
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

/// A stored journal carrying a key this version does not know still reads,
/// and the held sequence resumes from it.
#[tokio::test]
async fn a_journal_with_an_unknown_key_still_reads_and_resumes() {
    let fixture = title_fixture("Sequence unknown key journal", MediaFacet::Movie).await;
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

    let mut journal: serde_json::Value =
        serde_json::from_str(&deletion_run.detail).expect("parse storage journal");
    journal["synthetic_unknown_key"] = serde_json::json!({"note": "written by another version"});
    let mut extended = deletion_run.clone();
    extended.detail = journal.to_string();
    fixture
        .execution
        .evaluation
        .finish_action_run(&extended)
        .await
        .expect("store the extended journal");

    capacity_recovered.store(true, Ordering::SeqCst);
    fixture.execution.evaluation.begin_next_evaluation().await;
    let report = fixture
        .execution
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("resume from the extended journal");
    assert_eq!(report.executed, 1, "{report:?}");
    assert!(!second_path.exists(), "the journaled target is deleted");
    for file_id in [&first_file_id, &second_file_id] {
        assert!(
            fixture
                .execution
                .app
                .services
                .library
                .media_files
                .get_media_file_by_id(file_id)
                .await
                .expect("read deleted storage row")
                .is_none()
        );
    }
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
    let (_, fired) = app
        .run_scheduled_lifecycle_rule(&rule_set_id)
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

// ── Conflicts across levels ─────────────────────────────────────────────────

fn cross_level_draft(rego_source: &str) -> crate::lifecycle_rules::LifecycleRuleDraft {
    crate::lifecycle_rules::LifecycleRuleDraft {
        name: "Synthetic cross-level rule".to_string(),
        description: String::new(),
        rego_source: rego_source.to_string(),
        grace_days: 0,
        library_ids: Vec::new(),
        max_titles_per_run: Some(100),
    }
}

/// Arm the rule for deletion, acknowledging the candidates it opened.
async fn arm_destructive(fixture: &TitleFixture, rule_set_id: &str) {
    let live = fixture
        .execution
        .evaluation
        .all_candidates()
        .await
        .iter()
        .filter(|candidate| !candidate.state.is_terminal())
        .count();
    super::lifecycle_rules::arm_rule(
        &fixture.execution.app,
        &fixture.execution.user,
        rule_set_id,
        LifecycleEffectArming::Destructive,
        Some(i64::try_from(live).expect("candidate count")),
    )
    .await;
}

async fn assert_no_steps(fixture: &TitleFixture, candidate: &scryer_domain::LifecycleCandidate) {
    assert!(
        fixture
            .execution
            .evaluation
            .list_action_steps(
                &candidate.id,
                candidate.match_generation,
                candidate.revision_number,
            )
            .await
            .expect("read steps")
            .is_empty(),
        "{candidate:?}"
    );
}

/// A title the rule monitors while it deletes the files of one of its
/// seasons: both candidates are held with `actions_conflict`, nothing runs,
/// and the season's file stays on disk.
#[tokio::test]
async fn a_title_monitor_and_a_season_delete_hold_both_and_delete_nothing() {
    let fixture = title_fixture("Synthetic Cross Hold", MediaFacet::Series).await;
    let (_, episode) = season_and_episode(&fixture, 1, 1, "2020-01-01").await;
    let path = fixture.root.join("Cross Hold/Season 01/S01E01.mkv");
    add_episode_file(&fixture, &episode, "Cross Hold/Season 01/S01E01.mkv").await;
    let (rule_set_id, candidates) = arm_and_evaluate_all(
        &fixture.execution,
        cross_level_draft(
            "package rules\nimport rego.v1\n\n\
             monitor if input.facts.monitored\n\n\
             delete_season_files if input.subject.season_number == 1\n",
        ),
    )
    .await;
    assert_eq!(candidates.len(), 2, "{candidates:?}");
    arm_destructive(&fixture, &rule_set_id).await;

    let report = fixture
        .execution
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("action pass");
    assert_eq!(report.executed, 0, "{report:?}");
    assert_eq!(report.held, 2, "{report:?}");
    for held in fixture.execution.evaluation.all_candidates().await {
        assert_eq!(
            held.state,
            scryer_domain::LifecycleCandidateState::Blocked,
            "{held:?}"
        );
        assert_eq!(
            held.state_reason,
            crate::lifecycle_rules::execution_reason::ACTIONS_CONFLICT
        );
        assert!(held.actions_conflict.is_some(), "{held:?}");
        assert_no_steps(&fixture, &held).await;
    }
    assert!(path.exists(), "the season's file stays on disk");
}

/// A held candidate still shows the sequence it would run, and a fire that
/// finds it already held for the conflict writes nothing to it.
#[tokio::test]
async fn a_marked_candidate_keeps_its_sequence_view_and_a_second_hold_writes_nothing() {
    let fixture = title_fixture("Synthetic Cross View", MediaFacet::Series).await;
    let (season, episode) = season_and_episode(&fixture, 1, 1, "2020-01-01").await;
    let path = fixture.root.join("Cross View/Season 01/S01E01.mkv");
    add_episode_file(&fixture, &episode, "Cross View/Season 01/S01E01.mkv").await;
    let (rule_set_id, candidates) = arm_and_evaluate_all(
        &fixture.execution,
        cross_level_draft(
            "package rules\nimport rego.v1\n\n\
             monitor if input.facts.monitored\n\n\
             delete_season_files if input.subject.season_number == 1\n",
        ),
    )
    .await;
    assert_eq!(candidates.len(), 2, "{candidates:?}");
    arm_destructive(&fixture, &rule_set_id).await;
    let report = fixture
        .execution
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("first action pass");
    assert_eq!(report.held, 2, "{report:?}");

    let views = fixture
        .execution
        .app
        .list_lifecycle_candidates(
            &fixture.execution.user,
            crate::lifecycle_rules::LifecycleCandidateFilter {
                rule_set_id: Some(rule_set_id.clone()),
                ..Default::default()
            },
        )
        .await
        .expect("list candidates");
    let season_view = views
        .iter()
        .find(|view| view.candidate.subject_id == season.id)
        .expect("season candidate view");
    assert!(season_view.candidate.actions_conflict.is_some());
    assert!(
        season_view.action_sequence.is_some(),
        "a marked candidate still shows its sequence: {season_view:?}"
    );

    let before = fixture.execution.evaluation.all_candidates().await;
    fixture.execution.evaluation.begin_next_evaluation().await;
    let report = fixture
        .execution
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("second action pass");
    assert_eq!(report.executed, 0, "{report:?}");
    assert_eq!(report.held, 2, "both were examined again: {report:?}");
    let after = fixture.execution.evaluation.all_candidates().await;
    for held in &after {
        let earlier = before
            .iter()
            .find(|candidate| candidate.id == held.id)
            .expect("same candidate");
        assert_eq!(
            held.state_reason,
            crate::lifecycle_rules::execution_reason::ACTIONS_CONFLICT
        );
        assert_eq!(
            held.updated_at, earlier.updated_at,
            "already held for the conflict: nothing is written {held:?}"
        );
        assert_no_steps(&fixture, held).await;
    }
    assert!(path.exists(), "the season's file stays on disk");
}

/// A pass in which the season's write fails cannot judge the title free of
/// the season's decision: the title keeps its marker and nothing runs.
#[tokio::test]
async fn a_failed_season_write_keeps_the_title_held() {
    let fixture = title_fixture("Synthetic Cross Failed", MediaFacet::Series).await;
    let (season, episode) = season_and_episode(&fixture, 1, 1, "2020-01-01").await;
    let path = fixture.root.join("Cross Failed/Season 01/S01E01.mkv");
    add_episode_file(&fixture, &episode, "Cross Failed/Season 01/S01E01.mkv").await;
    let (rule_set_id, candidates) = arm_and_evaluate_all(
        &fixture.execution,
        cross_level_draft(
            "package rules\nimport rego.v1\n\n\
             monitor if input.facts.monitored\n\n\
             delete_season_files if input.subject.season_number == 1\n",
        ),
    )
    .await;
    assert_eq!(candidates.len(), 2, "{candidates:?}");
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.actions_conflict.is_some()),
        "{candidates:?}"
    );
    arm_destructive(&fixture, &rule_set_id).await;

    *fixture
        .execution
        .evaluation
        .fail_record_match_for_subject
        .lock()
        .unwrap() = Some(season.id.clone());
    fixture
        .execution
        .app
        .run_lifecycle_rule_evaluation_job()
        .await
        .expect("evaluate with the season's write failing");
    let title = fixture
        .execution
        .evaluation
        .all_candidates()
        .await
        .into_iter()
        .find(|candidate| candidate.subject_id == fixture.title.id)
        .expect("title candidate");
    assert!(
        title.actions_conflict.is_some(),
        "the title keeps its marker: {title:?}"
    );

    let report = fixture
        .execution
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("action pass");
    assert_eq!(report.executed, 0, "{report:?}");
    for held in fixture.execution.evaluation.all_candidates().await {
        assert_no_steps(&fixture, &held).await;
    }
    assert!(path.exists(), "the season's file stays on disk");
}

/// Once an evaluation no longer finds the conflict, the held candidates are
/// examined again and each runs what it decided.
#[tokio::test]
async fn a_cleared_cross_level_conflict_lets_both_sides_proceed() {
    let fixture = title_fixture("Synthetic Cross Clear", MediaFacet::Series).await;
    let (_, episode) = season_and_episode(&fixture, 1, 1, "2020-01-01").await;
    let path = fixture.root.join("Cross Clear/Season 01/S01E01.mkv");
    add_episode_file(&fixture, &episode, "Cross Clear/Season 01/S01E01.mkv").await;
    fixture
        .execution
        .app
        .set_title_monitored(&fixture.execution.user, &fixture.title.id, false)
        .await
        .expect("unmonitor title");
    let (rule_set_id, candidates) = arm_and_evaluate_all(
        &fixture.execution,
        cross_level_draft(
            "package rules\nimport rego.v1\n\n\
             monitor if not input.facts.monitored\n\n\
             notify := \"Synthetic review\" if input.subject.title_id != \"\"\n\n\
             delete_season_files if input.subject.season_number == 1\n",
        ),
    )
    .await;
    assert_eq!(candidates.len(), 2, "{candidates:?}");
    arm_destructive(&fixture, &rule_set_id).await;
    let report = fixture
        .execution
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("held action pass");
    assert_eq!(report.executed, 0, "{report:?}");
    assert!(path.exists());

    // The title is monitored now, so the rule decides only `notify` for it.
    fixture
        .execution
        .app
        .set_title_monitored(&fixture.execution.user, &fixture.title.id, true)
        .await
        .expect("monitor title");
    fixture
        .execution
        .app
        .run_lifecycle_rule_evaluation_job()
        .await
        .expect("evaluate again");
    let live: Vec<_> = fixture
        .execution
        .evaluation
        .all_candidates()
        .await
        .into_iter()
        .filter(|candidate| !candidate.state.is_terminal())
        .collect();
    assert_eq!(live.len(), 2, "{live:?}");
    assert!(
        live.iter()
            .all(|candidate| candidate.actions_conflict.is_none()),
        "{live:?}"
    );

    let report = fixture
        .execution
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("action pass after the conflict cleared");
    assert_eq!(report.executed, 2, "{report:?}");
    assert!(
        !path.exists(),
        "the season's delete ran once nothing contradicted it"
    );
}

/// A season candidate opened while its title decided nothing holds at its
/// execution recheck when the title now decides a conflicting action, even
/// with no evaluation in between, and its file stays on disk.
#[tokio::test]
async fn a_title_conflict_that_appears_after_evaluation_holds_at_execution() {
    let fixture = title_fixture("Synthetic Cross Late", MediaFacet::Series).await;
    let (season, episode) = season_and_episode(&fixture, 1, 1, "2020-01-01").await;
    let path = fixture.root.join("Cross Late/Season 01/S01E01.mkv");
    add_episode_file(&fixture, &episode, "Cross Late/Season 01/S01E01.mkv").await;
    fixture
        .execution
        .app
        .set_title_monitored(&fixture.execution.user, &fixture.title.id, false)
        .await
        .expect("unmonitor title");
    let (rule_set_id, candidates) = arm_and_evaluate_all(
        &fixture.execution,
        cross_level_draft(
            "package rules\nimport rego.v1\n\n\
             monitor if input.facts.monitored\n\n\
             delete_season_files if input.subject.season_number == 1\n",
        ),
    )
    .await;
    assert_eq!(
        candidates.len(),
        1,
        "only the season is decided: {candidates:?}"
    );
    assert_eq!(candidates[0].subject_id, season.id);
    assert_eq!(candidates[0].actions_conflict, None);
    arm_destructive(&fixture, &rule_set_id).await;

    fixture
        .execution
        .app
        .set_title_monitored(&fixture.execution.user, &fixture.title.id, true)
        .await
        .expect("monitor title");
    let report = fixture
        .execution
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("action pass");
    assert_eq!(report.executed, 0, "{report:?}");
    let held = fixture
        .execution
        .evaluation
        .all_candidates()
        .await
        .remove(0);
    assert_eq!(
        held.state_reason,
        crate::lifecycle_rules::execution_reason::ACTIONS_CONFLICT,
        "{held:?}"
    );
    assert!(!held.state.is_terminal(), "{held:?}");
    assert!(path.exists(), "the season's file stays on disk");
}

// ── Manual runs evaluate first ──────────────────────────────────────────────

/// Trigger one rule's manual run and wait for the run it spawns, which holds
/// the rule's claim until it finishes.
async fn manual_run(fixture: &TitleFixture, rule_set_id: &str) {
    let app = &fixture.execution.app;
    let trigger = app
        .run_lifecycle_action_handler_now(&fixture.execution.user, Some(rule_set_id.to_string()))
        .await
        .expect("manual run");
    assert!(trigger.started, "{trigger:?}");
    tokio::time::timeout(std::time::Duration::from_secs(60), async {
        while app.claim_lifecycle_rule_run(rule_set_id).is_none() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the manual run finishes");
}

/// A title whose delete was decided cleanly, and a season of it that starts
/// deciding a monitor after that evaluation: the manual run evaluates first,
/// sees the conflict, holds both and deletes nothing.
#[tokio::test]
async fn a_manual_run_evaluates_first_and_holds_a_new_cross_level_conflict() {
    let fixture = title_fixture("Synthetic Manual Conflict", MediaFacet::Series).await;
    let (season, episode) = season_and_episode(&fixture, 1, 1, "2020-01-01").await;
    let path = fixture.root.join("Manual Conflict/Season 01/S01E01.mkv");
    add_episode_file(&fixture, &episode, "Manual Conflict/Season 01/S01E01.mkv").await;
    let (rule_set_id, candidates) = arm_and_evaluate_all(
        &fixture.execution,
        cross_level_draft(
            "package rules\nimport rego.v1\n\n\
             delete_files if input.subject.title_id != \"\"\n\n\
             monitor_season if {\n\
             \tinput.subject.season_number == 1\n\
             \tnot input.facts.monitored\n\
             }\n",
        ),
    )
    .await;
    assert_eq!(
        candidates.len(),
        1,
        "only the title is decided: {candidates:?}"
    );
    assert_eq!(candidates[0].subject_id, fixture.title.id);
    assert_eq!(candidates[0].actions_conflict, None);
    arm_destructive(&fixture, &rule_set_id).await;

    // The title is unmonitored now, so the season decides a monitor.
    fixture
        .execution
        .app
        .set_title_monitored(&fixture.execution.user, &fixture.title.id, false)
        .await
        .expect("unmonitor title");
    manual_run(&fixture, &rule_set_id).await;

    let live: Vec<_> = fixture
        .execution
        .evaluation
        .all_candidates()
        .await
        .into_iter()
        .filter(|candidate| !candidate.state.is_terminal())
        .collect();
    assert_eq!(live.len(), 2, "{live:?}");
    assert!(
        live.iter()
            .any(|candidate| candidate.subject_id == season.id)
    );
    for held in &live {
        assert!(held.actions_conflict.is_some(), "{held:?}");
        assert_eq!(held.state, scryer_domain::LifecycleCandidateState::Blocked);
        assert_eq!(
            held.state_reason,
            crate::lifecycle_rules::execution_reason::ACTIONS_CONFLICT
        );
        assert_no_steps(&fixture, held).await;
    }
    assert!(path.exists(), "nothing is deleted");
}

/// A dry-run rule's manual run evaluates it and acts on nothing.
#[tokio::test]
async fn a_manual_run_of_a_dry_run_rule_acts_on_nothing() {
    let fixture = title_fixture("Synthetic Manual Dry", MediaFacet::Series).await;
    let (_, episode) = season_and_episode(&fixture, 1, 1, "2020-01-01").await;
    let path = fixture.root.join("Manual Dry/Season 01/S01E01.mkv");
    add_episode_file(&fixture, &episode, "Manual Dry/Season 01/S01E01.mkv").await;
    let (rule_set_id, _) = arm_and_evaluate_all(
        &fixture.execution,
        cross_level_draft(
            "package rules\nimport rego.v1\n\n\
             delete_files if not input.facts.monitored\n",
        ),
    )
    .await;
    super::lifecycle_rules::arm_rule(
        &fixture.execution.app,
        &fixture.execution.user,
        &rule_set_id,
        LifecycleEffectArming::None,
        None,
    )
    .await;
    fixture
        .execution
        .app
        .set_title_monitored(&fixture.execution.user, &fixture.title.id, false)
        .await
        .expect("unmonitor title");

    manual_run(&fixture, &rule_set_id).await;

    let candidates = fixture.execution.evaluation.all_candidates().await;
    assert_eq!(candidates.len(), 1, "the evaluation ran: {candidates:?}");
    assert_no_steps(&fixture, &candidates[0]).await;
    assert!(path.exists(), "a dry-run rule deletes nothing");
}

/// A manual run whose evaluation opens a candidate that is already due acts
/// on it in the same run.
#[tokio::test]
async fn a_manual_run_acts_on_a_candidate_its_evaluation_opens_already_due() {
    let fixture = title_fixture("Synthetic Manual Due", MediaFacet::Series).await;
    let (_, episode) = season_and_episode(&fixture, 1, 1, "2020-01-01").await;
    let path = fixture.root.join("Manual Due/Season 01/S01E01.mkv");
    add_episode_file(&fixture, &episode, "Manual Due/Season 01/S01E01.mkv").await;
    let (rule_set_id, candidates) = arm_and_evaluate_all(
        &fixture.execution,
        cross_level_draft(
            "package rules\nimport rego.v1\n\n\
             delete_files if not input.facts.monitored\n",
        ),
    )
    .await;
    assert!(candidates.is_empty(), "{candidates:?}");
    fixture
        .execution
        .app
        .set_title_monitored(&fixture.execution.user, &fixture.title.id, false)
        .await
        .expect("unmonitor title");

    manual_run(&fixture, &rule_set_id).await;

    let candidates = fixture.execution.evaluation.all_candidates().await;
    assert_eq!(candidates.len(), 1, "{candidates:?}");
    assert_eq!(
        candidates[0].state,
        scryer_domain::LifecycleCandidateState::Succeeded,
        "{:?}",
        candidates[0]
    );
    assert!(!path.exists(), "the due delete ran in the same manual run");
}

// ── Monitoring conflicts across rules and levels ────────────────────────────

/// Create, arm and evaluate a rule, acknowledging the candidates it opened
/// when it deletes, then run one action pass over every rule.
async fn add_rule_and_act(
    fixture: &ExecutionFixture,
    name: &str,
    rego_source: &str,
) -> (
    String,
    crate::lifecycle_rules::LifecycleActionHandlingReport,
) {
    let (rule_set_id, _) = arm_and_evaluate_all(
        fixture,
        crate::lifecycle_rules::LifecycleRuleDraft {
            name: name.to_string(),
            ..cross_level_draft(rego_source)
        },
    )
    .await;
    let rule = fixture
        .app
        .require_lifecycle_rule_set(&rule_set_id)
        .await
        .expect("rule");
    if rule.effect_arming == LifecycleEffectArming::Destructive {
        let live = fixture
            .evaluation
            .all_candidates()
            .await
            .iter()
            .filter(|candidate| {
                candidate.rule_set_id == rule_set_id && !candidate.state.is_terminal()
            })
            .count();
        super::lifecycle_rules::arm_rule(
            &fixture.app,
            &fixture.user,
            &rule_set_id,
            LifecycleEffectArming::Destructive,
            Some(i64::try_from(live).expect("candidate count")),
        )
        .await;
    }
    let report = fixture
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("action pass");
    (rule_set_id, report)
}

async fn rule_candidates(
    fixture: &ExecutionFixture,
    rule_set_id: &str,
) -> Vec<scryer_domain::LifecycleCandidate> {
    fixture
        .evaluation
        .all_candidates()
        .await
        .into_iter()
        .filter(|candidate| candidate.rule_set_id == rule_set_id)
        .collect()
}

/// One rule monitored a show; another rule's season delete on the same show
/// would undo that, so it holds with `monitoring_conflict` and deletes
/// nothing.
#[tokio::test]
async fn a_season_delete_holds_after_another_rule_monitored_its_title() {
    let fixture = title_fixture("Synthetic Season Tug", MediaFacet::Series).await;
    let (season, episode) = season_and_episode(&fixture, 1, 1, "2020-01-01").await;
    let path = fixture.root.join("Season Tug/Season 01/S01E01.mkv");
    add_episode_file(&fixture, &episode, "Season Tug/Season 01/S01E01.mkv").await;
    fixture
        .execution
        .app
        .set_title_monitored(&fixture.execution.user, &fixture.title.id, false)
        .await
        .expect("unmonitor title");

    let (_, report) = add_rule_and_act(
        &fixture.execution,
        "Synthetic title monitor",
        "package rules\nimport rego.v1\n\nmonitor if not input.facts.monitored\n",
    )
    .await;
    assert_eq!(report.executed, 1, "{report:?}");

    let (delete_rule, report) = add_rule_and_act(
        &fixture.execution,
        "Synthetic season delete",
        "package rules\nimport rego.v1\n\n\
         delete_season_files if input.subject.season_number == 1\n",
    )
    .await;
    assert_eq!(report.executed, 0, "{report:?}");
    let candidates = rule_candidates(&fixture.execution, &delete_rule).await;
    let held = candidates
        .iter()
        .find(|candidate| candidate.subject_id == season.id)
        .unwrap_or_else(|| panic!("the season has a candidate: {candidates:?}"));
    assert_eq!(held.state, scryer_domain::LifecycleCandidateState::Blocked);
    assert_eq!(
        held.state_reason,
        crate::lifecycle_rules::execution_reason::MONITORING_CONFLICT
    );
    assert_no_steps(&fixture, held).await;
    assert!(path.exists(), "the season's file stays on disk");
}

/// One rule monitored an episode; another rule unmonitoring the episode's
/// show would undo that, so it holds with `monitoring_conflict`.
#[tokio::test]
async fn a_title_unmonitor_holds_after_another_rule_monitored_one_of_its_episodes() {
    let fixture = title_fixture("Synthetic Episode Tug", MediaFacet::Series).await;
    let (_, episode) = season_and_episode(&fixture, 1, 1, "2020-01-01").await;
    fixture
        .execution
        .app
        .set_episode_monitored(&fixture.execution.user, &episode.id, false)
        .await
        .expect("unmonitor episode");

    let (_, report) = add_rule_and_act(
        &fixture.execution,
        "Synthetic episode monitor",
        &format!(
            "package rules\nimport rego.v1\n\n\
             monitor_episode if input.subject.episode_id == {:?}\n",
            episode.id
        ),
    )
    .await;
    assert_eq!(report.executed, 1, "{report:?}");

    let (unmonitor_rule, report) = add_rule_and_act(
        &fixture.execution,
        "Synthetic title unmonitor",
        &format!(
            "package rules\nimport rego.v1\n\n\
             unmonitor if input.subject.title_id == {:?}\n",
            fixture.title.id
        ),
    )
    .await;
    assert_eq!(report.executed, 0, "{report:?}");
    let candidates = rule_candidates(&fixture.execution, &unmonitor_rule).await;
    assert_eq!(candidates.len(), 1, "{candidates:?}");
    assert_eq!(
        candidates[0].state,
        scryer_domain::LifecycleCandidateState::Blocked
    );
    assert_eq!(
        candidates[0].state_reason,
        crate::lifecycle_rules::execution_reason::MONITORING_CONFLICT
    );
    assert!(
        fixture
            .execution
            .app
            .get_title(&fixture.execution.user, &fixture.title.id)
            .await
            .expect("read title")
            .expect("title exists")
            .monitored,
        "the show stays monitored"
    );
}

/// A monitoring change on one show never holds an opposite change on an
/// unrelated title.
#[tokio::test]
async fn an_opposite_monitoring_change_on_an_unrelated_title_does_not_hold() {
    let fixture = title_fixture("Synthetic Episode Side", MediaFacet::Series).await;
    let (_, episode) = season_and_episode(&fixture, 1, 1, "2020-01-01").await;
    fixture
        .execution
        .app
        .set_episode_monitored(&fixture.execution.user, &episode.id, false)
        .await
        .expect("unmonitor episode");
    let other = seed_title(
        &fixture.execution.app,
        &fixture.execution.user,
        "Synthetic Unrelated Title",
        true,
    )
    .await;

    let (_, report) = add_rule_and_act(
        &fixture.execution,
        "Synthetic episode monitor",
        &format!(
            "package rules\nimport rego.v1\n\n\
             monitor_episode if input.subject.episode_id == {:?}\n",
            episode.id
        ),
    )
    .await;
    assert_eq!(report.executed, 1, "{report:?}");

    let (unmonitor_rule, report) = add_rule_and_act(
        &fixture.execution,
        "Synthetic unrelated unmonitor",
        &format!(
            "package rules\nimport rego.v1\n\n\
             unmonitor if input.subject.title_id == {:?}\n",
            other.id
        ),
    )
    .await;
    assert_eq!(report.executed, 1, "{report:?}");
    let candidates = rule_candidates(&fixture.execution, &unmonitor_rule).await;
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.state != scryer_domain::LifecycleCandidateState::Blocked),
        "{candidates:?}"
    );
    assert!(
        !fixture
            .execution
            .app
            .get_title(&fixture.execution.user, &other.id)
            .await
            .expect("read title")
            .expect("title exists")
            .monitored
    );
}

// ── Evaluation of a delete that has started ─────────────────────────────────

/// A storage matcher with one more condition, so deletion rechecks capacity
/// per file and the probe can hold the sequence between its files.
fn storage_matcher_with(condition: &str) -> String {
    format!(
        "match if {{\n\tsome root in input.facts.storage_roots\n\troot.available_percent >= 0\n\t{condition}\n}}\n"
    )
}

struct HeldDelete {
    fixture: TitleFixture,
    rule_set_id: String,
    candidate: scryer_domain::LifecycleCandidate,
    remaining_path: PathBuf,
    capacity_recovered: Arc<AtomicBool>,
    _probe: crate::lifecycle_rules::storage::LifecycleStorageCapacityProbeGuard,
}

impl HeldDelete {
    /// Run a full evaluation pass and read this candidate back.
    async fn reevaluate(&self) -> scryer_domain::LifecycleCandidate {
        self.fixture
            .execution
            .app
            .run_lifecycle_rule_evaluation_job()
            .await
            .expect("re-evaluate the held delete");
        let candidates = rule_candidates(&self.fixture.execution, &self.rule_set_id).await;
        assert_eq!(candidates.len(), 1, "{candidates:?}");
        let candidate = candidates.into_iter().next().expect("candidate");
        assert_eq!(candidate.id, self.candidate.id);
        candidate
    }

    async fn act(&self) -> crate::lifecycle_rules::LifecycleActionHandlingReport {
        self.fixture
            .execution
            .app
            .run_every_rule_action_pass_for_tests()
            .await
            .expect("action pass")
    }
}

/// Two five-byte movie files; the sequence unmonitors the title, deletes the
/// lower-id file, then holds on capacity before the other. Capacity reads
/// again once the hold returns.
async fn hold_movie_delete_after_one_file(name: &str, condition: &str) -> HeldDelete {
    let fixture = title_fixture(name, MediaFacet::Movie).await;
    let first_path = fixture.root.join("Held/first.mkv");
    let first_file_id =
        add_movie_file(&fixture.execution.app, &fixture.title.id, &first_path).await;
    let second_path = fixture.root.join("Held/second.mkv");
    let second_file_id =
        add_movie_file(&fixture.execution.app, &fixture.title.id, &second_path).await;
    let mut ordered = [(first_file_id, first_path), (second_file_id, second_path)];
    ordered.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    let [(_, deleted_path), (_, remaining_path)] = ordered;
    let (probe, capacity_recovered) = capacity_holds_after_file_removal(deleted_path.clone());
    let probe = install_lifecycle_storage_capacity_probe_for_test(probe).await;
    let mut draft = sequence_draft(deletion_steps());
    with_matcher(&mut draft, &storage_matcher_with(condition));
    let (rule_set_id, candidate) = arm_and_evaluate(&fixture.execution, draft).await;
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
        .expect("delete the first file, then hold");
    assert_eq!(report.held, 1, "{report:?}");
    assert!(!deleted_path.exists(), "the first file is deleted");
    assert!(remaining_path.exists(), "the second file waits");
    capacity_recovered.store(true, Ordering::SeqCst);
    HeldDelete {
        fixture,
        rule_set_id,
        candidate,
        remaining_path,
        capacity_recovered,
        _probe: probe,
    }
}

/// A rule that matches ten bytes or more still matches once its own delete
/// removed half of them, and the held sequence resumes to the end.
#[tokio::test]
async fn a_size_rule_still_matches_after_its_own_delete_removed_half_the_files() {
    let held = hold_movie_delete_after_one_file(
        "Synthetic Size Hold",
        "input.facts.total_file_size_bytes >= 10",
    )
    .await;

    let candidate = held.reevaluate().await;
    assert!(!candidate.state.is_terminal(), "{candidate:?}");

    let report = held.act().await;
    assert_eq!(report.executed, 1, "{report:?}");
    assert!(!held.remaining_path.exists(), "the sequence resumes");
}

/// A rule conditioned on `monitored` still matches after its own unmonitor
/// step ran.
#[tokio::test]
async fn a_monitored_rule_still_matches_after_its_own_unmonitor_step() {
    let held =
        hold_movie_delete_after_one_file("Synthetic Monitored Hold", "input.facts.monitored").await;
    assert!(
        !held
            .fixture
            .execution
            .app
            .services
            .catalog
            .titles
            .get_by_id(&held.fixture.title.id)
            .await
            .expect("read title")
            .expect("title exists")
            .monitored,
        "the sequence's own unmonitor step ran"
    );

    let candidate = held.reevaluate().await;
    assert!(!candidate.state.is_terminal(), "{candidate:?}");

    let report = held.act().await;
    assert_eq!(report.executed, 1, "{report:?}");
    assert!(!held.remaining_path.exists(), "the sequence resumes");
}

/// A season head conditioned on the season's and the show's file counts
/// still matches after its own delete removed one of the season's two files.
#[tokio::test]
async fn a_season_file_count_rule_still_matches_after_its_own_partial_delete() {
    let fixture = title_fixture("Synthetic Season Count", MediaFacet::Series).await;
    let (season, first_episode) = season_and_episode(&fixture, 1, 1, "2020-01-01").await;
    let second_episode = fixture
        .execution
        .app
        .create_episode(
            &fixture.execution.user,
            fixture.title.id.clone(),
            Some(season.id.clone()),
            "standard".into(),
            Some("2".to_string()),
            Some("1".to_string()),
            None,
            Some("Episode 1-2".to_string()),
            Some("2020-01-02".to_string()),
            None,
            false,
            false,
        )
        .await
        .expect("create the season's second episode");
    let first_rel = "Season Count/Season 01/S01E01.mkv";
    let second_rel = "Season Count/Season 01/S01E02.mkv";
    let first_file_id = add_episode_file(&fixture, &first_episode, first_rel).await;
    let second_file_id = add_episode_file(&fixture, &second_episode, second_rel).await;
    let mut ordered = [
        (first_file_id, fixture.root.join(first_rel)),
        (second_file_id, fixture.root.join(second_rel)),
    ];
    ordered.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    let [(_, deleted_path), (_, remaining_path)] = ordered;
    let (probe, capacity_recovered) = capacity_holds_after_file_removal(deleted_path.clone());
    let _probe = install_lifecycle_storage_capacity_probe_for_test(probe).await;

    let (rule_set_id, candidates) = arm_and_evaluate_all(
        &fixture.execution,
        cross_level_draft(
            "package rules\nimport rego.v1\n\n\
             delete_season_files if {\n\
             \tinput.subject.season_number == 1\n\
             \tinput.season.facts.file_count >= 2\n\
             \tinput.facts.file_count >= 2\n\
             \tsome root in input.facts.storage_roots\n\
             \troot.available_percent >= 0\n\
             }\n",
        ),
    )
    .await;
    assert_eq!(candidates.len(), 1, "{candidates:?}");
    let candidate = candidates.into_iter().next().expect("season candidate");
    assert_eq!(candidate.subject_id, season.id);
    arm_destructive(&fixture, &rule_set_id).await;
    let report = fixture
        .execution
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("delete one season file, then hold");
    assert_eq!(report.held, 1, "{report:?}");
    assert!(!deleted_path.exists(), "one season file is deleted");
    assert!(remaining_path.exists(), "the other season file waits");

    capacity_recovered.store(true, Ordering::SeqCst);
    let held = HeldDelete {
        fixture,
        rule_set_id,
        candidate,
        remaining_path,
        capacity_recovered,
        _probe,
    };
    let candidate = held.reevaluate().await;
    assert!(!candidate.state.is_terminal(), "{candidate:?}");

    let report = held.act().await;
    assert_eq!(report.executed, 1, "{report:?}");
    assert!(!held.remaining_path.exists(), "the sequence resumes");
}

/// A file imported from outside while the delete is held is not the
/// sequence's own change: the facts read as they are, the size rule no longer
/// matches, the candidate ends, and the waiting file stays.
#[tokio::test]
async fn an_outside_file_change_mid_delete_ends_the_candidate_and_keeps_the_file() {
    let held = hold_movie_delete_after_one_file(
        "Synthetic Outside Import",
        "input.facts.total_file_size_bytes >= 10",
    )
    .await;
    let imported_path = held.fixture.root.join("Held/imported.mkv");
    write_media_file(&imported_path);
    held.fixture
        .execution
        .app
        .services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: held.fixture.title.id.clone(),
            file_path: imported_path.to_string_lossy().to_string(),
            size_bytes: 1,
            role: MediaFileRole::Primary,
            ..Default::default()
        })
        .await
        .expect("import a file from outside the sequence");

    let candidate = held.reevaluate().await;
    assert!(candidate.state.is_terminal(), "{candidate:?}");

    let report = held.act().await;
    assert_eq!(report.executed, 0, "{report:?}");
    assert!(held.remaining_path.exists(), "the waiting file stays");
    assert!(imported_path.exists(), "the imported file stays");
    assert!(held.capacity_recovered.load(Ordering::SeqCst));
}

/// A candidate whose sequence never started is judged on the facts as they
/// are: an outside unmonitor ends a rule conditioned on `monitored`.
#[tokio::test]
async fn a_candidate_without_a_started_delete_is_judged_on_live_facts() {
    let fixture = title_fixture("Synthetic Unstarted", MediaFacet::Movie).await;
    let path = fixture.root.join("Unstarted/movie.mkv");
    add_movie_file(&fixture.execution.app, &fixture.title.id, &path).await;
    let _probe =
        install_lifecycle_storage_capacity_probe_for_test(stable_capacity_probe(capacity(100)))
            .await;
    let mut draft = sequence_draft(deletion_steps());
    with_matcher(&mut draft, &storage_matcher_with("input.facts.monitored"));
    let (rule_set_id, candidate) = arm_and_evaluate(&fixture.execution, draft).await;
    assert_no_steps(&fixture, &candidate).await;
    fixture
        .execution
        .app
        .set_title_monitored(&fixture.execution.user, &fixture.title.id, false)
        .await
        .expect("unmonitor from outside");

    fixture
        .execution
        .app
        .run_lifecycle_rule_evaluation_job()
        .await
        .expect("re-evaluate");
    let candidates = rule_candidates(&fixture.execution, &rule_set_id).await;
    let reread = candidates
        .iter()
        .find(|reread| reread.id == candidate.id)
        .expect("candidate");
    assert!(reread.state.is_terminal(), "{reread:?}");
    assert!(path.exists());
}

/// The pass's batched step read gives every candidate the same started
/// delete, or none, that its own read gives: no steps, a completed unmonitor,
/// a started delete, and step rows that belong to another generation.
#[tokio::test]
async fn the_batched_started_delete_read_matches_per_candidate_reads() {
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
    let target = seed_title(&fixture.app, &fixture.user, "Synthetic Batched Read", true).await;
    add_movie_file(
        &fixture.app,
        &target.id,
        &tempdir.path().join("Batched/movie.mkv"),
    )
    .await;
    let (_rule_set_id, mut base) =
        arm_and_evaluate(&fixture, sequence_draft(deletion_steps())).await;
    base.state = scryer_domain::LifecycleCandidateState::Due;
    let sequence = LifecycleActionSequence::new(deletion_steps());
    let variant = |id: &str| {
        let mut candidate = base.clone();
        candidate.id = id.to_string();
        candidate
    };

    let no_steps = variant("synthetic-no-steps");
    let unmonitored = variant("synthetic-unmonitored");
    persist_completed_unmonitor(&fixture, &unmonitored, &sequence).await;
    let delete_started = variant("synthetic-delete-started");
    persist_completed_unmonitor(&fixture, &delete_started, &sequence).await;
    persist_succeeded_step(
        &fixture,
        &delete_started,
        &sequence,
        LifecycleActionStepKind::DeleteFiles,
    )
    .await;
    let other_generation = variant("synthetic-other-generation");
    let mut earlier_generation = other_generation.clone();
    earlier_generation.match_generation -= 1;
    persist_completed_unmonitor(&fixture, &earlier_generation, &sequence).await;
    let candidates = vec![
        no_steps.clone(),
        unmonitored.clone(),
        delete_started.clone(),
        other_generation.clone(),
    ];

    let mut batched = crate::lifecycle_rules::action_execution::StartedDeleteReads::default();
    fixture
        .app
        .lifecycle_read_ahead_started_delete_steps(candidates.clone(), &mut batched)
        .await;
    assert_eq!(batched.read_ahead_len(), candidates.len());
    let mut started = Vec::new();
    for candidate in &candidates {
        let batched_read = fixture
            .app
            .lifecycle_started_sequence_delete(candidate, &mut batched)
            .await
            .expect("batched read")
            .map(|started| started.comparable());
        let own_read = fixture
            .app
            .lifecycle_started_sequence_delete(candidate, &mut Default::default())
            .await
            .expect("per-candidate read")
            .map(|started| started.comparable());
        assert_eq!(batched_read, own_read, "{}", candidate.id);
        started.push(batched_read.map(|(_, steps, journal)| (steps.len(), journal)));
    }
    assert_eq!(
        started,
        vec![None, Some((1, false)), Some((2, false)), None],
        "no steps, unmonitor done, delete started, other generation"
    );
}

/// An evaluation pass lands between the sequence's completed Unmonitor and
/// its DeleteFiles, after one of the show's episodes was re-monitored from
/// outside. The restore no longer puts the show's monitored episode count
/// back, so the rule reads the show as it is, the candidate ends, and no file
/// goes.
#[tokio::test]
async fn an_outside_re_monitor_between_unmonitor_and_delete_is_not_hidden() {
    let fixture = title_fixture("Synthetic Gap Re-monitor", MediaFacet::Series).await;
    let (first_season, first_episode) = season_and_episode(&fixture, 1, 1, "2020-01-01").await;
    let (second_season, second_episode) = season_and_episode(&fixture, 2, 1, "2021-01-01").await;
    let relative = "Gap Re-monitor/Season 01/S01E01.mkv";
    add_episode_file(&fixture, &first_episode, relative).await;
    let path = fixture.root.join(relative);
    let app = &fixture.execution.app;
    let user = &fixture.execution.user;
    let title_id = &fixture.title.id;
    let seasons = [&first_season.id, &second_season.id];
    let episodes = [&first_episode.id, &second_episode.id];
    let set_monitoring = |monitored: bool| async move {
        app.set_title_monitored(user, title_id, monitored)
            .await
            .expect("set title monitoring");
        for season_id in seasons {
            app.set_collection_monitored(user, season_id, monitored)
                .await
                .expect("set season monitoring");
        }
        for episode_id in episodes {
            app.set_episode_monitored(user, episode_id, monitored)
                .await
                .expect("set episode monitoring");
        }
    };
    set_monitoring(true).await;
    let _probe =
        install_lifecycle_storage_capacity_probe_for_test(stable_capacity_probe(capacity(100)))
            .await;
    let mut draft = sequence_draft(deletion_steps());
    with_matcher(
        &mut draft,
        &storage_matcher_with("input.facts.monitored_episode_count >= 2"),
    );
    let (rule_set_id, candidate) = arm_and_evaluate(&fixture.execution, draft).await;
    assert_eq!(candidate.subject_id, fixture.title.id);

    // The sequence's own Unmonitor completed: the show and everything under
    // it is unmonitored, and DeleteFiles has not begun. The executor left the
    // candidate blocked between the two steps.
    set_monitoring(false).await;
    assert!(
        fixture
            .execution
            .evaluation
            .transition_candidate_state(
                &candidate.id,
                scryer_domain::LifecycleCandidateState::Blocked,
                "synthetic_hold_between_steps",
                &[candidate.state],
                Utc::now(),
            )
            .await
            .expect("block the candidate between its steps")
    );
    persist_succeeded_step_with_before(
        &fixture.execution,
        &candidate,
        &crate::lifecycle_rules::action_execution::declared_candidate_sequence(
            &candidate,
            &Default::default(),
        )
        .expect("the candidate's own sequence"),
        LifecycleActionStepKind::Unmonitor,
        serde_json::json!({
            "schema_version": 1,
            "facts_monitored": true,
            "facts_monitored_episode_count": 2,
            "monitoring_changed": true,
        }),
    )
    .await;
    let reread = |label: &'static str| {
        let fixture = &fixture;
        let rule_set_id = &rule_set_id;
        let candidate_id = candidate.id.clone();
        async move {
            fixture
                .execution
                .app
                .run_lifecycle_rule_evaluation_job()
                .await
                .expect(label);
            rule_candidates(&fixture.execution, rule_set_id)
                .await
                .into_iter()
                .find(|reread| reread.id == candidate_id)
                .expect("candidate")
        }
    };
    let intact = reread("evaluate with the own unmonitor intact").await;
    assert!(
        !intact.state.is_terminal(),
        "the own unmonitor is restored while it holds: {intact:?}"
    );

    // Monitoring one episode also monitors its season and the show.
    app.set_episode_monitored(user, &first_episode.id, true)
        .await
        .expect("re-monitor an episode from outside");
    let after = reread("evaluate after the outside re-monitor").await;
    assert!(after.state.is_terminal(), "{after:?}");

    arm_destructive(&fixture, &rule_set_id).await;
    let report = app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("action pass");
    assert_eq!(report.executed, 0, "{report:?}");
    assert!(path.exists(), "no file is deleted");
}

/// A started delete whose journal no longer passes its identity check holds
/// its candidate at evaluation, and the next pass holds it again.
#[tokio::test]
async fn a_journal_failing_its_identity_check_holds_the_candidate_across_passes() {
    let held = hold_movie_delete_after_one_file(
        "Synthetic Mismatched Journal",
        "input.facts.file_count >= 0",
    )
    .await;
    let intact = held.reevaluate().await;
    assert!(!intact.state.is_terminal(), "{intact:?}");
    assert!(
        intact.held_since.is_none(),
        "an intact journal holds nothing: {intact:?}"
    );

    let evaluation = &held.fixture.execution.evaluation;
    let run = evaluation
        .latest_scoped_deletion_action_run(
            &held.candidate.id,
            held.candidate.match_generation,
            &held.candidate.action_kind,
        )
        .await
        .expect("read the journal")
        .expect("the held delete kept its journal");
    let mut journal: serde_json::Value =
        serde_json::from_str(&run.detail).expect("parse the journal");
    journal["subject_id"] = serde_json::json!("synthetic-other-subject");
    let mut mismatched = run.clone();
    mismatched.detail = journal.to_string();
    evaluation
        .finish_action_run(&mismatched)
        .await
        .expect("store the mismatched journal");

    for pass in ["first", "second"] {
        let candidate = held.reevaluate().await;
        assert!(!candidate.state.is_terminal(), "{pass} pass: {candidate:?}");
        assert!(
            candidate.held_since.is_some(),
            "{pass} pass holds: {candidate:?}"
        );
        assert!(held.remaining_path.exists(), "{pass} pass deletes nothing");
    }
}

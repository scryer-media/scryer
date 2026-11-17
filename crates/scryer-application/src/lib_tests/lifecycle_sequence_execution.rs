//! End-to-end v2 sequence execution regressions.
//!
//! These use the normal candidate scheduler and durable in-memory repositories.
//! The completed-job adapter is private to the executor and simulates an
//! externally accepted job that has not finished yet; it is never an action a
//! user can place in a sequence.

use crate::lib_tests::lifecycle_execution::{execution_app, seed_title};
use crate::lib_tests::lifecycle_sequence_deletion::{
    add_catalog_movie_file_without_physical_path, storage_sequence_deletion_draft,
};
use crate::lib_tests::lifecycle_title_completion::{TitleFixture, title_fixture};
use crate::lifecycle_rules::sequence_execution::{
    TestCompletedSequenceJobMap, TestCompletedSequenceJobState,
    install_test_completed_sequence_job_adapter, set_test_completed_sequence_job_state,
};
use crate::lifecycle_rules::{
    LifecycleActionSequence, LifecycleActionStep, LifecycleActionStepKind,
    LifecycleActionStepParameters, LifecycleGatesUpdate, LifecycleRuleDraft,
    LifecycleSearchCondition,
};
use crate::ports::{
    LifecycleActionRunRepository, LifecycleActionStepRepository, LifecycleCandidateRepository,
    LifecycleRuleSetRepository,
};
use chrono::{Duration, Utc};
use scryer_domain::{
    LifecycleActionStepKey, LifecycleActionStepRun, LifecycleActionStepState,
    LifecycleCandidateState, LifecycleRuleState, MediaFacet,
};
use std::collections::HashMap;

pub(super) const ALWAYS_SOURCE: &str = "match := true\n";

/// Swap a sequence draft's always-true matcher for another, keeping the
/// action heads its steps added.
pub(super) fn with_source(draft: &mut LifecycleRuleDraft, matcher: &str) {
    draft.rego_source = draft.rego_source.replacen(ALWAYS_SOURCE, matcher, 1);
}

pub(super) fn sequence_draft(steps: Vec<LifecycleActionStep>) -> LifecycleRuleDraft {
    LifecycleRuleDraft {
        max_titles_per_run: Some(100),
        name: "Sequence executor regression".to_string(),
        description: String::new(),
        rego_source: crate::lib_tests::steps_source(ALWAYS_SOURCE, &steps),
        grace_days: 0,
        library_ids: Vec::new(),
    }
}

pub(super) async fn arm_and_evaluate(
    fixture: &crate::lib_tests::lifecycle_execution::ExecutionFixture,
    draft: LifecycleRuleDraft,
) -> (String, scryer_domain::LifecycleCandidate) {
    let (rule_set_id, candidates) = arm_and_evaluate_all(fixture, draft).await;
    assert_eq!(candidates.len(), 1, "{candidates:?}");
    (
        rule_set_id,
        candidates.into_iter().next().expect("candidate"),
    )
}

pub(super) async fn arm_and_evaluate_all(
    fixture: &crate::lib_tests::lifecycle_execution::ExecutionFixture,
    draft: LifecycleRuleDraft,
) -> (String, Vec<scryer_domain::LifecycleCandidate>) {
    let created = fixture
        .app
        .create_lifecycle_rule_set(&fixture.user, draft)
        .await
        .expect("create sequence rule");
    let rule_set_id = created.rule_set.id.clone();
    super::lifecycle_rules::set_rule_state(
        &fixture.app,
        &fixture.user,
        &rule_set_id,
        scryer_domain::LifecycleRuleState::DryRun,
        None,
    )
    .await;
    fixture
        .app
        .set_lifecycle_instance_gates(
            &fixture.user,
            LifecycleGatesUpdate {
                enabled: Some(true),
                actions_paused: Some(false),
            },
        )
        .await
        .expect("enable lifecycle and unpause actions");
    // Arm before the first evaluation: a delete rule confirms the empty
    // candidate list it has at that point.
    let state = enabled_state_for(&created.declared_actions);
    super::lifecycle_rules::arm_rule(
        &fixture.app,
        &fixture.user,
        &rule_set_id,
        state,
        created.declared_actions.destructive.then_some(0),
    )
    .await;
    fixture
        .app
        .run_lifecycle_rule_evaluation_job()
        .await
        .expect("evaluate sequence rule");
    let candidates = fixture.evaluation.all_candidates().await;
    (rule_set_id, candidates)
}

async fn define_title_tags(
    fixture: &crate::lib_tests::lifecycle_execution::ExecutionFixture,
    labels: &[&str],
) {
    for label in labels {
        fixture
            .app
            .create_title_tag_definition(&fixture.user, label, None)
            .await
            .expect("define title tag");
    }
}

async fn seed_stale_running_add_tag_step(
    fixture: &crate::lib_tests::lifecycle_execution::ExecutionFixture,
    candidate: &scryer_domain::LifecycleCandidate,
    sequence: &LifecycleActionSequence,
    step: &LifecycleActionStep,
) -> LifecycleActionStepKey {
    seed_stale_running_step(
        fixture,
        candidate,
        sequence,
        step,
        serde_json::json!({
            "changed_tags": ["crash-tag"],
            "expected_postcondition": "tags_present",
        }),
    )
    .await
}

/// A step a crashed worker left running, its checkpoint carrying `detail`.
async fn seed_stale_running_step(
    fixture: &crate::lib_tests::lifecycle_execution::ExecutionFixture,
    candidate: &scryer_domain::LifecycleCandidate,
    sequence: &LifecycleActionSequence,
    step: &LifecycleActionStep,
    detail: serde_json::Value,
) -> LifecycleActionStepKey {
    let abandoned_at = Utc::now() - Duration::minutes(61);
    let mut provenance = serde_json::json!({
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
    });
    if let (Some(provenance), Some(detail)) = (provenance.as_object_mut(), detail.as_object()) {
        provenance.extend(detail.clone());
    }
    let key = LifecycleActionStepKey {
        candidate_id: candidate.id.clone(),
        match_generation: candidate.match_generation,
        revision_number: candidate.revision_number,
        step_id: step.id.clone(),
    };
    let run = LifecycleActionStepRun {
        key: key.clone(),
        rule_set_id: candidate.rule_set_id.clone(),
        title_id: candidate.title_id.clone(),
        subject_kind: candidate.subject_kind.clone(),
        subject_id: candidate.subject_id.clone(),
        sequence_content_hash: sequence.content_hash().expect("sequence hash"),
        step_kind: step.kind.as_wire_str().to_string(),
        intent_json: serde_json::json!({"step": step, "search_request": null}).to_string(),
        before_state_json: serde_json::json!({"schema_version": 1, "title_tags": []}).to_string(),
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
        provenance_json: provenance.to_string(),
        state: LifecycleActionStepState::Running,
        attempt: 1,
        lease_id: Some("crashed-sequence-worker".to_string()),
        lease_expires_at: Some(abandoned_at),
        hold_reason: None,
        error: None,
        created_at: abandoned_at,
        updated_at: abandoned_at,
        finished_at: None,
    };
    assert!(matches!(
        fixture
            .evaluation
            .claim_action_step(&run, Utc::now(), "crashed-sequence-worker", abandoned_at)
            .await
            .expect("persist stale intent"),
        crate::ports::LifecycleActionStepClaim::Claimed(_)
    ));
    assert!(
        fixture
            .evaluation
            .transition_candidate_state(
                &candidate.id,
                LifecycleCandidateState::Due,
                "seed_stale_sequence_lease",
                std::slice::from_ref(&candidate.state),
                abandoned_at,
            )
            .await
            .expect("make candidate leasable for stale recovery")
    );
    assert!(
        fixture
            .evaluation
            .lease_candidate_for_execution(&candidate.id, abandoned_at, abandoned_at)
            .await
            .expect("persist stale candidate lease")
    );
    key
}

async fn finish_seeded_step_as_failed(
    fixture: &crate::lib_tests::lifecycle_execution::ExecutionFixture,
    candidate: &scryer_domain::LifecycleCandidate,
    key: &LifecycleActionStepKey,
    attempt: i64,
) {
    let now = Utc::now();
    for ordinal in 1..=attempt {
        let mut claim = fixture
            .evaluation
            .list_action_steps(&key.candidate_id, key.match_generation, key.revision_number)
            .await
            .expect("read seeded step")
            .into_iter()
            .next()
            .expect("seeded step");
        claim.state = LifecycleActionStepState::Running;
        claim.attempt = ordinal;
        let lease_id = format!("persisted-sequence-failure-{ordinal}");
        let mut failed = match fixture
            .evaluation
            .claim_action_step(&claim, now, &lease_id, now)
            .await
            .expect("claim seeded step for failure")
        {
            crate::ports::LifecycleActionStepClaim::Claimed(run) => run,
            other => panic!("expected seeded step claim, got {other:?}"),
        };
        failed.state = LifecycleActionStepState::Failed;
        failed.error = Some("simulated persistence failure after mutation".to_string());
        failed.finished_at = Some(now);
        failed.updated_at = now;
        assert!(
            fixture
                .evaluation
                .finish_action_step(&failed, &lease_id)
                .await
                .expect("persist failed step")
        );
    }
    assert!(
        fixture
            .evaluation
            .transition_candidate_state(
                &candidate.id,
                LifecycleCandidateState::Due,
                "seed_failed_sequence_step",
                &[LifecycleCandidateState::Executing],
                now,
            )
            .await
            .expect("return failed seed to due")
    );
}

fn fresh_runtime_app(app: &crate::AppUseCase) -> crate::AppUseCase {
    crate::AppUseCase {
        runtime: crate::services::AppRuntimeState::default(),
        ..app.clone()
    }
}

#[tokio::test]
async fn sequence_unmonitor_executes_and_records_its_stable_step() {
    let fixture = execution_app(None);
    let title = seed_title(&fixture.app, &fixture.user, "V2 unmonitor", true).await;
    let (_rule_set_id, candidate) = arm_and_evaluate(
        &fixture,
        sequence_draft(vec![LifecycleActionStep {
            id: "unmonitor".to_string(),
            kind: LifecycleActionStepKind::Unmonitor,
            parameters: LifecycleActionStepParameters::Unmonitor {
                include_descendants: false,
            },
        }]),
    )
    .await;

    let report = fixture
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("execute sequence");
    assert_eq!(report.executed, 1, "{report:?}");
    assert!(
        !fixture
            .app
            .get_title(&fixture.user, &title.id)
            .await
            .expect("read title")
            .expect("title exists")
            .monitored
    );
    let candidates = fixture.evaluation.all_candidates().await;
    assert_eq!(candidates[0].state, LifecycleCandidateState::Succeeded);
    let steps = fixture
        .evaluation
        .list_action_steps(
            &candidate.id,
            candidate.match_generation,
            candidate.revision_number,
        )
        .await
        .expect("read durable steps");
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].key.step_id, "unmonitor");
    assert!(steps[0].state.is_completion_success());
}

#[tokio::test]
async fn empty_sequence_remains_membership_only_when_armed() {
    let fixture = execution_app(None);
    seed_title(&fixture.app, &fixture.user, "V2 empty membership", false).await;
    let (rule_set_id, candidate) = arm_and_evaluate(
        &fixture,
        sequence_draft(vec![LifecycleActionStep {
            id: "unmonitor".to_string(),
            kind: LifecycleActionStepKind::Unmonitor,
            parameters: LifecycleActionStepParameters::Unmonitor {
                include_descendants: true,
            },
        }]),
    )
    .await;
    // A save can no longer write a rule that declares no action, but history
    // stored before then can still carry an empty step sequence.
    let stored = fixture
        .rules
        .get_revision(&rule_set_id, candidate.revision_number)
        .await
        .expect("read revision")
        .expect("revision exists");
    fixture
        .rules
        .replace_revision_in_place(scryer_domain::LifecycleRuleRevision {
            action_spec_json: serde_json::to_string(
                &crate::lifecycle_rules::LifecycleActionSequence::new(Vec::new()),
            )
            .expect("encode empty sequence"),
            ..stored
        })
        .await;

    let report = fixture
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("skip empty sequence execution");
    assert_eq!(report.rules_eligible, 0, "{report:?}");
    assert_eq!(report.candidates_considered, 0, "{report:?}");
    assert_eq!(
        fixture.evaluation.all_candidates().await[0].state,
        candidate.state.clone()
    );
    assert!(
        fixture
            .evaluation
            .list_action_steps(
                &candidate.id,
                candidate.match_generation,
                candidate.revision_number,
            )
            .await
            .expect("empty sequence step history")
            .is_empty()
    );
    assert!(
        fixture
            .evaluation
            .list_action_runs(None, Some(&candidate.id), None)
            .await
            .expect("empty sequence action history")
            .is_empty()
    );
}

#[tokio::test]
async fn stale_intent_before_a_tag_mutation_retries_the_proven_idempotent_write() {
    let fixture = execution_app(None);
    let title = seed_title(&fixture.app, &fixture.user, "V2 crash before tag", false).await;
    define_title_tags(&fixture, &["crash-tag"]).await;
    let sequence = LifecycleActionSequence::new(vec![LifecycleActionStep {
        id: "add_tags".to_string(),
        kind: LifecycleActionStepKind::AddTags,
        parameters: LifecycleActionStepParameters::Tags {
            tags: vec!["crash-tag".to_string()],
        },
    }]);
    let (_rule_set_id, candidate) =
        arm_and_evaluate(&fixture, sequence_draft(sequence.steps.clone())).await;
    let key =
        seed_stale_running_add_tag_step(&fixture, &candidate, &sequence, &sequence.steps[0]).await;

    let report = fresh_runtime_app(&fixture.app)
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("reclaim stale intent");
    assert_eq!(report.executed, 1, "{report:?}");
    assert!(
        fixture
            .app
            .get_title(&fixture.user, &title.id)
            .await
            .expect("read title")
            .expect("title exists")
            .tags
            .iter()
            .any(|tag| tag == "crash-tag")
    );
    let step = fixture
        .evaluation
        .list_action_steps(&key.candidate_id, key.match_generation, key.revision_number)
        .await
        .expect("retried step")
        .into_iter()
        .next()
        .expect("tag step");
    assert_eq!(step.state, LifecycleActionStepState::Succeeded);
}

/// Reclaim a fetch-subtitles step a crashed worker left running with the
/// given checkpoint, and return the state the step finishes in. Subtitles
/// are off in the fixture, so a step that really runs starts no search and
/// finishes as already satisfied.
async fn reclaim_stale_fetch_subtitles(
    name: &str,
    checkpoint: serde_json::Value,
) -> (LifecycleActionStepState, serde_json::Value) {
    let fixture = execution_app(None);
    seed_title(&fixture.app, &fixture.user, name, false).await;
    let sequence = LifecycleActionSequence::new(vec![LifecycleActionStep {
        id: "fetch_subtitles".to_string(),
        kind: LifecycleActionStepKind::FetchSubtitles,
        parameters: LifecycleActionStepParameters::None,
    }]);
    let (_rule_set_id, candidate) =
        arm_and_evaluate(&fixture, sequence_draft(sequence.steps.clone())).await;
    let key = seed_stale_running_step(
        &fixture,
        &candidate,
        &sequence,
        &sequence.steps[0],
        checkpoint,
    )
    .await;
    fresh_runtime_app(&fixture.app)
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("reclaim stale fetch-subtitles step");
    let step = fixture
        .evaluation
        .list_action_steps(&key.candidate_id, key.match_generation, key.revision_number)
        .await
        .expect("read step")
        .into_iter()
        .next()
        .expect("fetch-subtitles step");
    let provenance = serde_json::from_str(&step.provenance_json).expect("provenance");
    (step.state, provenance)
}

#[tokio::test]
async fn a_fetch_subtitles_step_that_recorded_its_start_finishes_without_starting_again() {
    let (state, provenance) = reclaim_stale_fetch_subtitles(
        "Subtitles already started",
        serde_json::json!({
            "expected_postcondition": "subtitle_search_started",
            "searches_started": 2,
        }),
    )
    .await;
    assert_eq!(state, LifecycleActionStepState::Succeeded, "{provenance}");
    assert_eq!(provenance["searches_started"], 2);
}

#[tokio::test]
async fn a_fetch_subtitles_step_that_crashed_before_its_start_runs_again() {
    let (state, provenance) = reclaim_stale_fetch_subtitles(
        "Subtitles never started",
        serde_json::json!({"expected_postcondition": "subtitle_search_started"}),
    )
    .await;
    assert_eq!(
        state,
        LifecycleActionStepState::AlreadySatisfied,
        "{provenance}"
    );
    assert_eq!(provenance["searches_started"], 0);
}

#[tokio::test]
async fn stale_intent_with_a_changed_prestate_holds_without_overwriting_live_tags() {
    let fixture = execution_app(None);
    let title = seed_title(&fixture.app, &fixture.user, "V2 crash changed tags", false).await;
    define_title_tags(&fixture, &["crash-tag", "external-tag"]).await;
    let sequence = LifecycleActionSequence::new(vec![LifecycleActionStep {
        id: "add_tags".to_string(),
        kind: LifecycleActionStepKind::AddTags,
        parameters: LifecycleActionStepParameters::Tags {
            tags: vec!["crash-tag".to_string()],
        },
    }]);
    let (_rule_set_id, candidate) =
        arm_and_evaluate(&fixture, sequence_draft(sequence.steps.clone())).await;
    let key =
        seed_stale_running_add_tag_step(&fixture, &candidate, &sequence, &sequence.steps[0]).await;
    fixture
        .app
        .update_title_tags(
            &fixture.user,
            std::slice::from_ref(&title.id),
            &["external-tag".to_string()],
            &[],
        )
        .await
        .expect("apply external tag mutation");

    let report = fresh_runtime_app(&fixture.app)
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("hold changed stale intent");
    assert_eq!(report.held, 1, "{report:?}");
    let stored = fixture
        .app
        .get_title(&fixture.user, &title.id)
        .await
        .expect("read title")
        .expect("title exists");
    assert!(stored.tags.iter().any(|tag| tag == "external-tag"));
    assert!(!stored.tags.iter().any(|tag| tag == "crash-tag"));
    let step = fixture
        .evaluation
        .list_action_steps(&key.candidate_id, key.match_generation, key.revision_number)
        .await
        .expect("held step")
        .into_iter()
        .next()
        .expect("tag step");
    assert_eq!(step.state, LifecycleActionStepState::Running);
}

#[tokio::test]
async fn stale_intent_after_a_tag_mutation_recovers_as_already_satisfied() {
    let fixture = execution_app(None);
    let title = seed_title(&fixture.app, &fixture.user, "V2 crash after tag", false).await;
    define_title_tags(&fixture, &["crash-tag"]).await;
    let sequence = LifecycleActionSequence::new(vec![LifecycleActionStep {
        id: "add_tags".to_string(),
        kind: LifecycleActionStepKind::AddTags,
        parameters: LifecycleActionStepParameters::Tags {
            tags: vec!["crash-tag".to_string()],
        },
    }]);
    let (_rule_set_id, candidate) =
        arm_and_evaluate(&fixture, sequence_draft(sequence.steps.clone())).await;
    let key =
        seed_stale_running_add_tag_step(&fixture, &candidate, &sequence, &sequence.steps[0]).await;
    fixture
        .app
        .update_title_tags(
            &fixture.user,
            std::slice::from_ref(&title.id),
            &["crash-tag".to_string()],
            &[],
        )
        .await
        .expect("apply interrupted mutation");

    let report = fresh_runtime_app(&fixture.app)
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("recover stale postcondition");
    assert_eq!(report.executed, 1, "{report:?}");
    let step = fixture
        .evaluation
        .list_action_steps(&key.candidate_id, key.match_generation, key.revision_number)
        .await
        .expect("recovered step")
        .into_iter()
        .next()
        .expect("tag step");
    assert_eq!(step.state, LifecycleActionStepState::AlreadySatisfied);
    assert_eq!(
        fixture.evaluation.all_candidates().await[0].state,
        LifecycleCandidateState::Succeeded
    );
}

#[tokio::test]
async fn failed_checkpoint_after_tag_mutation_recovers_without_losing_its_attribution() {
    let fixture = execution_app(None);
    let title = seed_title(&fixture.app, &fixture.user, "V2 failed after tag", false).await;
    define_title_tags(&fixture, &["crash-tag"]).await;
    let sequence = LifecycleActionSequence::new(vec![LifecycleActionStep {
        id: "add_tags".to_string(),
        kind: LifecycleActionStepKind::AddTags,
        parameters: LifecycleActionStepParameters::Tags {
            tags: vec!["crash-tag".to_string()],
        },
    }]);
    let (_rule_set_id, candidate) =
        arm_and_evaluate(&fixture, sequence_draft(sequence.steps.clone())).await;
    let key =
        seed_stale_running_add_tag_step(&fixture, &candidate, &sequence, &sequence.steps[0]).await;
    fixture
        .app
        .update_title_tags(
            &fixture.user,
            std::slice::from_ref(&title.id),
            &["crash-tag".to_string()],
            &[],
        )
        .await
        .expect("apply mutation before persistence failure");
    finish_seeded_step_as_failed(&fixture, &candidate, &key, 1).await;

    let report = fresh_runtime_app(&fixture.app)
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("recover failed postcondition");
    assert_eq!(report.executed, 1, "{report:?}");
    let step = fixture
        .evaluation
        .list_action_steps(&key.candidate_id, key.match_generation, key.revision_number)
        .await
        .expect("recovered failed step")
        .into_iter()
        .next()
        .expect("tag step");
    assert_eq!(step.state, LifecycleActionStepState::AlreadySatisfied);
    let provenance: serde_json::Value =
        serde_json::from_str(&step.provenance_json).expect("saved provenance");
    assert_eq!(
        provenance
            .get("changed_tags")
            .and_then(serde_json::Value::as_array)
            .expect("owned changed tags"),
        &vec![serde_json::Value::String("crash-tag".to_string())],
        "recovery must retain the durable action attribution"
    );
}

#[tokio::test]
async fn exhausted_failed_step_becomes_terminal_without_a_fourth_dispatch() {
    let fixture = execution_app(None);
    let title = seed_title(&fixture.app, &fixture.user, "V2 exhausted crash", false).await;
    define_title_tags(&fixture, &["crash-tag"]).await;
    let sequence = LifecycleActionSequence::new(vec![LifecycleActionStep {
        id: "add_tags".to_string(),
        kind: LifecycleActionStepKind::AddTags,
        parameters: LifecycleActionStepParameters::Tags {
            tags: vec!["crash-tag".to_string()],
        },
    }]);
    let (_rule_set_id, candidate) =
        arm_and_evaluate(&fixture, sequence_draft(sequence.steps.clone())).await;
    let key =
        seed_stale_running_add_tag_step(&fixture, &candidate, &sequence, &sequence.steps[0]).await;
    finish_seeded_step_as_failed(&fixture, &candidate, &key, 3).await;

    let report = fresh_runtime_app(&fixture.app)
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("terminalize exhausted step");
    assert_eq!(report.failed, 1, "{report:?}");
    assert_eq!(
        fixture.evaluation.all_candidates().await[0].state,
        LifecycleCandidateState::Failed
    );
    assert!(
        !fixture
            .app
            .get_title(&fixture.user, &title.id)
            .await
            .expect("read title")
            .expect("title exists")
            .tags
            .iter()
            .any(|tag| tag == "crash-tag")
    );
    let step = fixture
        .evaluation
        .list_action_steps(&key.candidate_id, key.match_generation, key.revision_number)
        .await
        .expect("exhausted step")
        .into_iter()
        .next()
        .expect("tag step");
    assert_eq!(step.state, LifecycleActionStepState::Failed);
    assert_eq!(step.attempt, 3, "the executor must not claim attempt four");
}

#[tokio::test]
async fn real_search_acceptance_is_durable_for_unconditional_and_changed_profile_conditions() {
    let unconditional = execution_app(None);
    seed_title(
        &unconditional.app,
        &unconditional.user,
        "V2 unconditional accepted search",
        false,
    )
    .await;
    let (_rule_set_id, candidate) = arm_and_evaluate(
        &unconditional,
        sequence_draft(vec![LifecycleActionStep {
            id: "search".to_string(),
            kind: LifecycleActionStepKind::Search,
            parameters: LifecycleActionStepParameters::Search {
                condition: LifecycleSearchCondition::Unconditional,
            },
        }]),
    )
    .await;
    assert_eq!(
        unconditional
            .app
            .run_every_rule_action_pass_for_tests()
            .await
            .expect("accept unconditional lifecycle search")
            .executed,
        1
    );
    let unconditional_key = LifecycleActionStepKey {
        candidate_id: candidate.id.clone(),
        match_generation: candidate.match_generation,
        revision_number: candidate.revision_number,
        step_id: "search".to_string(),
    };
    let receipts = unconditional
        .evaluation
        .list_action_job_receipts(&unconditional_key)
        .await
        .expect("unconditional accepted receipt");
    assert_eq!(receipts.len(), 1);
    assert_eq!(
        receipts[0].state,
        scryer_domain::LifecycleActionJobReceiptState::Accepted
    );
    assert_eq!(
        fresh_runtime_app(&unconditional.app)
            .run_every_rule_action_pass_for_tests()
            .await
            .expect("restart after accepted unconditional search")
            .executed,
        0,
        "terminal membership prevents a restart from dispatching a duplicate search"
    );
    assert_eq!(
        unconditional
            .evaluation
            .list_action_job_receipts(&unconditional_key)
            .await
            .expect("unconditional receipt after restart")
            .len(),
        1
    );

    let conditional = execution_app(None);
    seed_title(
        &conditional.app,
        &conditional.user,
        "V2 profile changed accepted search",
        false,
    )
    .await;
    let (_rule_set_id, candidate) = arm_and_evaluate(
        &conditional,
        sequence_draft(vec![
            LifecycleActionStep {
                id: "change_quality_profile".to_string(),
                kind: LifecycleActionStepKind::ChangeQualityProfile,
                parameters: LifecycleActionStepParameters::ChangeQualityProfile {
                    target_quality_profile_id: "4k".to_string(),
                },
            },
            LifecycleActionStep {
                id: "search".to_string(),
                kind: LifecycleActionStepKind::Search,
                parameters: LifecycleActionStepParameters::Search {
                    condition: LifecycleSearchCondition::PreviousProfileChanged,
                },
            },
        ]),
    )
    .await;
    assert_eq!(
        conditional
            .app
            .run_every_rule_action_pass_for_tests()
            .await
            .expect("accept profile-changed lifecycle search")
            .executed,
        1
    );
    let conditional_key = LifecycleActionStepKey {
        candidate_id: candidate.id.clone(),
        match_generation: candidate.match_generation,
        revision_number: candidate.revision_number,
        step_id: "search".to_string(),
    };
    let receipts = conditional
        .evaluation
        .list_action_job_receipts(&conditional_key)
        .await
        .expect("conditional accepted receipt");
    assert_eq!(receipts.len(), 1);
    assert_eq!(
        receipts[0].state,
        scryer_domain::LifecycleActionJobReceiptState::Accepted
    );
    let profile = conditional
        .evaluation
        .list_action_steps(
            &candidate.id,
            candidate.match_generation,
            candidate.revision_number,
        )
        .await
        .expect("profile progress")
        .into_iter()
        .find(|step| step.key.step_id == "change_quality_profile")
        .expect("profile step");
    let provenance: serde_json::Value =
        serde_json::from_str(&profile.provenance_json).expect("profile provenance");
    assert_eq!(
        provenance
            .get("profile_changed")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        fresh_runtime_app(&conditional.app)
            .run_every_rule_action_pass_for_tests()
            .await
            .expect("restart after conditional accepted search")
            .executed,
        0
    );
    assert_eq!(
        conditional
            .evaluation
            .list_action_job_receipts(&conditional_key)
            .await
            .expect("conditional receipt after restart")
            .len(),
        1
    );
}

#[tokio::test]
async fn completed_job_adapter_holds_without_leasing_then_reconciles_unknown_once() {
    let fixture = execution_app(None);
    let title = seed_title(&fixture.app, &fixture.user, "V2 completed job", true).await;
    // Declared heads always search before they unmonitor, so the unmonitor
    // step waits behind the accepted search job.
    let steps = vec![
        LifecycleActionStep {
            id: "search".to_string(),
            kind: LifecycleActionStepKind::Search,
            parameters: LifecycleActionStepParameters::Search {
                condition: LifecycleSearchCondition::Unconditional,
            },
        },
        LifecycleActionStep {
            id: "unmonitor".to_string(),
            kind: LifecycleActionStepKind::Unmonitor,
            parameters: LifecycleActionStepParameters::Unmonitor {
                include_descendants: true,
            },
        },
    ];
    let (_rule_set_id, candidate) = arm_and_evaluate(&fixture, sequence_draft(steps)).await;
    let key = LifecycleActionStepKey {
        candidate_id: candidate.id.clone(),
        match_generation: candidate.match_generation,
        revision_number: candidate.revision_number,
        step_id: "search".to_string(),
    };
    let _adapter = install_test_completed_sequence_job_adapter(TestCompletedSequenceJobMap::from(
        [(key.clone(), TestCompletedSequenceJobState::Waiting)],
    ))
    .await;

    let waiting = fixture
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("accepted job holds");
    assert_eq!(waiting.held, 1, "{waiting:?}");
    assert_eq!(
        fixture.evaluation.all_candidates().await[0].state,
        LifecycleCandidateState::Blocked
    );
    assert_eq!(
        fixture
            .evaluation
            .list_action_job_receipts(&key)
            .await
            .expect("accepted receipt")
            .len(),
        1
    );

    // Simulate a worker dying after durable dispatch acceptance and before it
    // can finish the Running step. Both leases are older than the executor's
    // 60-minute stale boundary; no in-memory worker state is carried forward.
    let abandoned_at = Utc::now() - Duration::minutes(61);
    assert!(
        fixture
            .evaluation
            .transition_candidate_state(
                &candidate.id,
                LifecycleCandidateState::Due,
                "test_restart_due",
                &[LifecycleCandidateState::Blocked],
                abandoned_at,
            )
            .await
            .expect("make waiting candidate due")
    );
    assert!(
        fixture
            .evaluation
            .lease_candidate_for_execution(&candidate.id, Utc::now(), abandoned_at)
            .await
            .expect("seed stale execution lease")
    );
    let mut stale_step = fixture
        .evaluation
        .list_action_steps(
            &candidate.id,
            candidate.match_generation,
            candidate.revision_number,
        )
        .await
        .expect("read accepted step")
        .into_iter()
        .next()
        .expect("search step");
    stale_step.state = LifecycleActionStepState::Running;
    stale_step.lease_id = Some("crashed-sequence-worker".to_string());
    stale_step.lease_expires_at = Some(abandoned_at);
    stale_step.updated_at = abandoned_at;
    stale_step.finished_at = None;
    stale_step.hold_reason = None;
    assert!(matches!(
        fixture
            .evaluation
            .claim_action_step(
                &stale_step,
                Utc::now(),
                "crashed-sequence-worker",
                abandoned_at,
            )
            .await
            .expect("seed stale running step"),
        crate::ports::LifecycleActionStepClaim::Claimed(_)
    ));

    // A process restart gets a new runtime and coordination locks while
    // retaining only the durable repositories. It must reconcile the accepted
    // receipt instead of submitting a second job.
    let restarted = crate::AppUseCase {
        runtime: crate::services::AppRuntimeState::default(),
        ..fixture.app.clone()
    };
    set_test_completed_sequence_job_state(&key, TestCompletedSequenceJobState::Unknown);
    let unknown = restarted
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("unknown job holds");
    assert_eq!(unknown.held, 1, "{unknown:?}");
    let receipts = fixture
        .evaluation
        .list_action_job_receipts(&key)
        .await
        .expect("unknown receipt");
    assert_eq!(receipts.len(), 1, "unknown must not submit a second job");

    set_test_completed_sequence_job_state(&key, TestCompletedSequenceJobState::Completed);
    fixture.evaluation.begin_next_evaluation().await;
    let completed = restarted
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("reconcile completed job");
    assert_eq!(completed.executed, 1, "{completed:?}");
    assert!(
        !fixture
            .app
            .get_title(&fixture.user, &title.id)
            .await
            .expect("read title")
            .expect("title exists")
            .monitored,
        "the step after the reconciled search ran"
    );
    assert_eq!(
        fixture.evaluation.all_candidates().await[0].state,
        LifecycleCandidateState::Succeeded
    );
    assert_eq!(
        fixture
            .evaluation
            .list_action_job_receipts(&key)
            .await
            .expect("completed receipt")
            .len(),
        1
    );
}

#[tokio::test]
async fn completed_job_adapter_retries_confirmed_failure_with_a_new_dispatch_receipt() {
    let fixture = execution_app(None);
    let title = seed_title(&fixture.app, &fixture.user, "V2 failed job", false).await;
    define_title_tags(&fixture, &["after_retry"]).await;
    let (_rule_set_id, candidate) = arm_and_evaluate(
        &fixture,
        sequence_draft(vec![
            LifecycleActionStep {
                id: "search".to_string(),
                kind: LifecycleActionStepKind::Search,
                parameters: LifecycleActionStepParameters::Search {
                    condition: LifecycleSearchCondition::Unconditional,
                },
            },
            LifecycleActionStep {
                id: "add_tags".to_string(),
                kind: LifecycleActionStepKind::AddTags,
                parameters: LifecycleActionStepParameters::Tags {
                    tags: vec!["after_retry".to_string()],
                },
            },
        ]),
    )
    .await;
    let key = LifecycleActionStepKey {
        candidate_id: candidate.id.clone(),
        match_generation: candidate.match_generation,
        revision_number: candidate.revision_number,
        step_id: "search".to_string(),
    };
    let _adapter = install_test_completed_sequence_job_adapter(HashMap::from([(
        key.clone(),
        TestCompletedSequenceJobState::Failed,
    )]))
    .await;

    let failed = fixture
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("record confirmed job failure");
    assert_eq!(failed.failed, 1, "{failed:?}");
    assert_eq!(
        fixture.evaluation.all_candidates().await[0].state,
        LifecycleCandidateState::Blocked
    );

    set_test_completed_sequence_job_state(&key, TestCompletedSequenceJobState::Completed);
    fixture.evaluation.begin_next_evaluation().await;
    let retried = fixture
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("retry confirmed job failure");
    assert_eq!(retried.executed, 1, "{retried:?}");
    assert_eq!(
        fixture
            .evaluation
            .list_action_job_receipts(&key)
            .await
            .expect("retry receipts")
            .len(),
        2,
        "a confirmed failed receipt earns one new dispatch attempt"
    );
    assert!(
        fixture
            .app
            .get_title(&fixture.user, &title.id)
            .await
            .expect("read title")
            .expect("title exists")
            .tags
            .iter()
            .any(|tag| tag == "after_retry")
    );
}

#[tokio::test]
async fn completed_job_adapter_enforces_the_durable_step_attempt_budget() {
    let fixture = execution_app(None);
    seed_title(&fixture.app, &fixture.user, "V2 bounded failed job", false).await;
    let (_rule_set_id, candidate) = arm_and_evaluate(
        &fixture,
        sequence_draft(vec![LifecycleActionStep {
            id: "search".to_string(),
            kind: LifecycleActionStepKind::Search,
            parameters: LifecycleActionStepParameters::Search {
                condition: LifecycleSearchCondition::Unconditional,
            },
        }]),
    )
    .await;
    let key = LifecycleActionStepKey {
        candidate_id: candidate.id.clone(),
        match_generation: candidate.match_generation,
        revision_number: candidate.revision_number,
        step_id: "search".to_string(),
    };
    let _adapter = install_test_completed_sequence_job_adapter(HashMap::from([(
        key.clone(),
        TestCompletedSequenceJobState::Failed,
    )]))
    .await;

    for retry in 1..=3 {
        fixture.evaluation.begin_next_evaluation().await;
        let report = fixture
            .app
            .run_every_rule_action_pass_for_tests()
            .await
            .expect("record bounded failed step attempt");
        assert_eq!(report.failed, 1, "retry {retry}: {report:?}");
    }
    assert_eq!(
        fixture.evaluation.all_candidates().await[0].state,
        LifecycleCandidateState::Failed
    );
    let steps = fixture
        .evaluation
        .list_action_steps(
            &candidate.id,
            candidate.match_generation,
            candidate.revision_number,
        )
        .await
        .expect("failed durable step");
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].attempt, 3);
    assert_eq!(
        fixture
            .evaluation
            .list_action_job_receipts(&key)
            .await
            .expect("bounded dispatch receipts")
            .len(),
        3
    );
}

#[tokio::test]
async fn unmonitor_after_a_waiting_search_reads_monitoring_when_it_runs() {
    let fixture = execution_app(None);
    let title = seed_title(&fixture.app, &fixture.user, "V2 monitoring baseline", true).await;
    let steps = vec![
        LifecycleActionStep {
            id: "unmonitor".to_string(),
            kind: LifecycleActionStepKind::Unmonitor,
            parameters: LifecycleActionStepParameters::Unmonitor {
                include_descendants: false,
            },
        },
        LifecycleActionStep {
            id: "search".to_string(),
            kind: LifecycleActionStepKind::Search,
            parameters: LifecycleActionStepParameters::Search {
                condition: LifecycleSearchCondition::Unconditional,
            },
        },
    ];
    let mut draft = sequence_draft(steps);
    with_source(&mut draft, "match if { input.facts.monitored }\n");
    let (_rule_set_id, candidate) = arm_and_evaluate(&fixture, draft).await;
    // Declared heads always search before they unmonitor, so the unmonitor
    // waits behind the accepted search and acts on monitoring as it is when
    // the search completes.
    let key = LifecycleActionStepKey {
        candidate_id: candidate.id,
        match_generation: candidate.match_generation,
        revision_number: candidate.revision_number,
        step_id: "search".to_string(),
    };
    let _adapter = install_test_completed_sequence_job_adapter(HashMap::from([(
        key.clone(),
        TestCompletedSequenceJobState::Waiting,
    )]))
    .await;

    let waiting = fixture
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("search waits ahead of the unmonitor");
    assert_eq!(waiting.held, 1, "{waiting:?}");
    assert!(
        fixture
            .app
            .get_title(&fixture.user, &title.id)
            .await
            .expect("read title")
            .expect("title exists")
            .monitored,
        "nothing is unmonitored while the search waits"
    );
    set_test_completed_sequence_job_state(&key, TestCompletedSequenceJobState::Completed);
    fixture.evaluation.begin_next_evaluation().await;
    let completed = fixture
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("the unmonitor runs once the search completes");
    assert_eq!(completed.executed, 1, "{completed:?}");
    assert!(
        !fixture
            .app
            .get_title(&fixture.user, &title.id)
            .await
            .expect("read title")
            .expect("title exists")
            .monitored
    );
}

#[tokio::test]
async fn tag_delta_baseline_preserves_external_tags_across_a_waiting_step() {
    let fixture = execution_app(None);
    let title = seed_title(&fixture.app, &fixture.user, "V2 tag baseline", false).await;
    define_title_tags(&fixture, &["owned-add", "owned-remove", "external-c"]).await;
    fixture
        .app
        .update_title_tags(
            &fixture.user,
            std::slice::from_ref(&title.id),
            &["owned-remove".to_string()],
            &[],
        )
        .await
        .expect("seed removable tag");
    let mut draft = sequence_draft(vec![
        LifecycleActionStep {
            id: "add_tags".to_string(),
            kind: LifecycleActionStepKind::AddTags,
            parameters: LifecycleActionStepParameters::Tags {
                tags: vec!["owned-add".to_string()],
            },
        },
        LifecycleActionStep {
            id: "remove_tags".to_string(),
            kind: LifecycleActionStepKind::RemoveTags,
            parameters: LifecycleActionStepParameters::Tags {
                tags: vec!["owned-remove".to_string()],
            },
        },
        LifecycleActionStep {
            id: "search".to_string(),
            kind: LifecycleActionStepKind::Search,
            parameters: LifecycleActionStepParameters::Search {
                condition: LifecycleSearchCondition::Unconditional,
            },
        },
    ]);
    with_source(
        &mut draft,
        "match if { input.facts.tags[_] == \"owned-remove\" }\n",
    );
    let (_rule_set_id, candidate) = arm_and_evaluate(&fixture, draft).await;
    let key = LifecycleActionStepKey {
        candidate_id: candidate.id,
        match_generation: candidate.match_generation,
        revision_number: candidate.revision_number,
        step_id: "search".to_string(),
    };
    let _adapter = install_test_completed_sequence_job_adapter(HashMap::from([(
        key.clone(),
        TestCompletedSequenceJobState::Waiting,
    )]))
    .await;

    assert_eq!(
        fixture
            .app
            .run_every_rule_action_pass_for_tests()
            .await
            .expect("own tag deltas keep matcher eligible")
            .held,
        1
    );
    fixture
        .app
        .update_title_tags(
            &fixture.user,
            std::slice::from_ref(&title.id),
            &["external-c".to_string()],
            &[],
        )
        .await
        .expect("external tag update");
    set_test_completed_sequence_job_state(&key, TestCompletedSequenceJobState::Completed);
    fixture.evaluation.begin_next_evaluation().await;
    assert_eq!(
        fixture
            .app
            .run_every_rule_action_pass_for_tests()
            .await
            .expect("live external tag does not cancel the sequence")
            .executed,
        1
    );
    let stored = fixture
        .app
        .get_title(&fixture.user, &title.id)
        .await
        .expect("read title")
        .expect("title exists");
    assert!(stored.tags.iter().any(|tag| tag == "owned-add"));
    assert!(!stored.tags.iter().any(|tag| tag == "owned-remove"));
    assert!(stored.tags.iter().any(|tag| tag == "external-c"));
}

/// A title counts once toward the rule's maximum however many of its steps
/// run, and the due titles past the maximum wait, untouched, for the next
/// fire.
#[tokio::test]
async fn the_rule_maximum_counts_titles_and_leaves_the_rest_for_the_next_fire() {
    let fixture = execution_app(None);
    for index in 1..=4 {
        seed_title(
            &fixture.app,
            &fixture.user,
            &format!("Synthetic maximum {index}"),
            false,
        )
        .await;
    }
    define_title_tags(&fixture, &["step-added", "step-removed"]).await;
    let (_rule_set_id, candidates) = arm_and_evaluate_all(
        &fixture,
        LifecycleRuleDraft {
            max_titles_per_run: Some(3),
            ..sequence_draft(vec![
                LifecycleActionStep {
                    id: "add_tags".to_string(),
                    kind: LifecycleActionStepKind::AddTags,
                    parameters: LifecycleActionStepParameters::Tags {
                        tags: vec!["step-added".to_string()],
                    },
                },
                LifecycleActionStep {
                    id: "remove_tags".to_string(),
                    kind: LifecycleActionStepKind::RemoveTags,
                    parameters: LifecycleActionStepParameters::Tags {
                        tags: vec!["step-removed".to_string()],
                    },
                },
                LifecycleActionStep {
                    id: "unmonitor".to_string(),
                    kind: LifecycleActionStepKind::Unmonitor,
                    parameters: LifecycleActionStepParameters::Unmonitor {
                        include_descendants: false,
                    },
                },
            ])
        },
    )
    .await;
    assert_eq!(candidates.len(), 4, "{candidates:?}");

    let first_pass = fixture
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("run the rule up to its maximum");
    assert_eq!(first_pass.executed, 3, "{first_pass:?}");
    assert_eq!(first_pass.held, 0, "{first_pass:?}");
    assert_eq!(first_pass.rules_stopped_at_maximum, 1, "{first_pass:?}");
    let after_first = fixture.evaluation.all_candidates().await;
    let waiting: Vec<_> = after_first
        .iter()
        .filter(|candidate| candidate.state != LifecycleCandidateState::Succeeded)
        .collect();
    assert_eq!(waiting.len(), 1, "{after_first:?}");
    let waiting = waiting[0].clone();
    let untouched = candidates
        .iter()
        .find(|candidate| candidate.id == waiting.id)
        .expect("the waiting candidate was opened by evaluation");
    assert_eq!(
        (&waiting.state, &waiting.state_reason, waiting.updated_at),
        (
            &untouched.state,
            &untouched.state_reason,
            untouched.updated_at
        ),
        "a title past the maximum is not held, stamped or explained"
    );
    assert!(
        fixture
            .evaluation
            .list_action_steps(
                &waiting.id,
                waiting.match_generation,
                waiting.revision_number
            )
            .await
            .expect("read waiting steps")
            .is_empty()
    );
    for done in after_first
        .iter()
        .filter(|candidate| candidate.state == LifecycleCandidateState::Succeeded)
    {
        let steps = fixture
            .evaluation
            .list_action_steps(&done.id, done.match_generation, done.revision_number)
            .await
            .expect("read finished steps");
        assert_eq!(
            steps.len(),
            3,
            "every step of a counted title runs: {steps:?}"
        );
    }

    let next_fire = fixture
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("the next fire continues");
    assert_eq!(next_fire.executed, 1, "{next_fire:?}");
    assert_eq!(next_fire.rules_stopped_at_maximum, 0, "{next_fire:?}");
    assert!(
        fixture
            .evaluation
            .all_candidates()
            .await
            .iter()
            .all(|candidate| candidate.state == LifecycleCandidateState::Succeeded)
    );
}

#[tokio::test]
async fn high_risk_sequence_failures_trip_the_per_pass_breaker_before_a_fourth_delete() {
    let fixture = title_fixture("Sequence breaker first", MediaFacet::Movie).await;
    let mut titles = vec![fixture.title.clone()];
    for index in 2..=4 {
        titles.push(
            seed_title(
                &fixture.execution.app,
                &fixture.execution.user,
                &format!("Sequence breaker {index}"),
                true,
            )
            .await,
        );
    }
    for (index, title) in titles.iter().enumerate() {
        let folder = fixture.root.join(format!("title-{index}"));
        std::fs::create_dir(&folder).expect("create title folder");
        let path = folder.join(format!("missing-{index}.mkv"));
        add_catalog_movie_file_without_physical_path(&fixture.execution.app, &title.id, &path)
            .await;
        std::fs::write(&path, b"admit the configured-root title")
            .expect("create file for storage-scoped evaluation");
        super::lifecycle_title_completion::record_fixture_title_folder(
            &fixture.execution.app,
            &title.id,
            &folder,
        )
        .await;
    }
    let draft = storage_sequence_deletion_draft();
    sample_fixture_root(&fixture);
    let (rule_set_id, candidates) = arm_and_evaluate_all(&fixture.execution, draft).await;
    assert_eq!(candidates.len(), 4, "{candidates:?}");
    super::lifecycle_rules::arm_rule(
        &fixture.execution.app,
        &fixture.execution.user,
        &rule_set_id,
        LifecycleRuleState::Enabled,
        Some(4),
    )
    .await;
    fixture
        .execution
        .media_files
        .fail_delete_media_file("post-preview catalog cleanup failure")
        .await;

    let report = fixture
        .execution
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("handle high-risk failing sequence pass");
    let candidates = fixture.execution.evaluation.all_candidates().await;
    assert_eq!(report.failed, 3, "{report:?}; candidates={candidates:#?}");
    assert_eq!(
        report.candidates_considered, 3,
        "{report:?}; candidates={candidates:#?}"
    );
    assert_eq!(
        candidates
            .iter()
            .filter(|candidate| candidate.state == LifecycleCandidateState::Blocked)
            .count(),
        3,
        "only three destructive failures may be attempted in one pass"
    );
    let mut untouched = Vec::new();
    for candidate in &candidates {
        let steps = fixture
            .execution
            .evaluation
            .list_action_steps(
                &candidate.id,
                candidate.match_generation,
                candidate.revision_number,
            )
            .await
            .expect("read breaker candidate progress");
        if steps.is_empty() {
            untouched.push(candidate);
        }
    }
    assert_eq!(
        untouched.len(),
        1,
        "the fourth candidate must have no dispatched step evidence"
    );
    let untouched = untouched[0];
    assert!(
        fixture
            .execution
            .app
            .services
            .library
            .media_files
            .list_media_files_for_title(&untouched.title_id)
            .await
            .expect("read untouched candidate files")
            .iter()
            .any(|file| file.role == crate::MediaFileRole::Primary),
        "the circuit breaker must preserve the untouched candidate's file"
    );
    assert!(
        !fixture
            .execution
            .evaluation
            .all_action_runs()
            .await
            .iter()
            .any(|run| run.candidate_id == untouched.id),
        "the circuit breaker must not begin a deletion or sequence history journal for the fourth candidate"
    );
}

#[tokio::test]
async fn transient_tag_conflict_scan_failure_holds_tags_but_not_unrelated_actions() {
    let fixture = execution_app(None);
    let title = seed_title(&fixture.app, &fixture.user, "Conflict scan failure", true).await;
    define_title_tags(&fixture, &["keep"]).await;
    for kind in [
        LifecycleActionStepKind::AddTags,
        LifecycleActionStepKind::RemoveTags,
    ] {
        arm_and_evaluate_all(
            &fixture,
            sequence_draft(vec![LifecycleActionStep {
                id: "tags".into(),
                kind,
                parameters: LifecycleActionStepParameters::Tags {
                    tags: vec!["keep".into()],
                },
            }]),
        )
        .await;
    }
    arm_and_evaluate_all(
        &fixture,
        sequence_draft(vec![LifecycleActionStep {
            id: "unmonitor".into(),
            kind: LifecycleActionStepKind::Unmonitor,
            parameters: LifecycleActionStepParameters::Unmonitor {
                include_descendants: false,
            },
        }]),
    )
    .await;
    fixture
        .evaluation
        .fail_next_due_read
        .store(true, std::sync::atomic::Ordering::SeqCst);
    fixture
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("handler");
    let candidates = fixture.evaluation.all_candidates().await;
    let held: Vec<_> = candidates
        .iter()
        .filter(|row| row.state == LifecycleCandidateState::Blocked)
        .collect();
    assert_eq!(held.len(), 2, "{candidates:?}");
    assert!(held.iter().all(|row| row.action_attempts == 0));
    assert!(
        held.iter().all(|row| row.state_reason
            == crate::lifecycle_rules::execution_reason::UNKNOWN_AT_EXECUTION)
    );
    assert_eq!(
        candidates
            .iter()
            .filter(|row| row.state == LifecycleCandidateState::Succeeded)
            .count(),
        1
    );
    let title = fixture
        .app
        .services
        .catalog
        .titles
        .get_by_id(&title.id)
        .await
        .unwrap()
        .unwrap();
    assert!(!title.monitored);
    assert!(!title.tags.iter().any(|tag| tag == "keep"));
}

/// A revision stored before rules declared their actions is refused at load,
/// whatever its source defines. Its source here defines `delete_title`, which
/// a step-sequence revision only ever read as a helper, and its stored steps
/// delete the title: nothing is evaluated, nothing runs, and the rule cannot
/// be enabled until its source is saved again.
#[tokio::test]
async fn a_stored_step_sequence_revision_is_refused_and_deletes_nothing() {
    use crate::scripts::scheduler::CustomJobSource;
    let fixture = execution_app(None);
    let title = seed_title(&fixture.app, &fixture.user, "Stored step order", false).await;
    let delete_title = LifecycleActionStep {
        id: "delete_title".to_string(),
        kind: LifecycleActionStepKind::DeleteTitleAndFiles,
        parameters: LifecycleActionStepParameters::None,
    };
    let (rule_set_id, candidate) =
        arm_and_evaluate(&fixture, sequence_draft(vec![delete_title.clone()])).await;
    let stored = fixture
        .rules
        .get_revision(&rule_set_id, candidate.revision_number)
        .await
        .expect("read revision")
        .expect("revision exists");
    fixture
        .rules
        .replace_revision_in_place(scryer_domain::LifecycleRuleRevision {
            action_spec_json: serde_json::to_string(&LifecycleActionSequence::new(vec![
                delete_title,
            ]))
            .expect("encode step order"),
            ..stored
        })
        .await;

    let detail = fixture
        .app
        .get_lifecycle_rule_set(&fixture.user, &rule_set_id)
        .await
        .expect("a refused rule still reads")
        .expect("rule exists");
    assert_eq!(
        detail.load_refusal,
        Some(crate::error_reasons::lifecycle::RULE_ACTIONS_UNREADABLE)
    );
    assert!(detail.declared_actions.heads.is_empty());
    assert!(
        fixture
            .app
            .load_scheduled_lifecycle_rules()
            .await
            .expect("load")
            .is_empty(),
        "a rule that would only refuse is not scheduled"
    );

    let evaluation = fixture
        .app
        .run_lifecycle_rule_evaluation_job()
        .await
        .expect("evaluation pass");
    assert_eq!(evaluation.rules_failed, 1, "{evaluation:?}");
    assert_eq!(evaluation.candidates_created, 0, "{evaluation:?}");
    assert_eq!(evaluation.candidates_matched, 0, "{evaluation:?}");

    fixture.evaluation.end_every_grace().await;
    let report = fixture
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("action pass");
    assert_eq!(report.executed, 0, "{report:?}");
    assert!(
        fixture
            .app
            .get_title(&fixture.user, &title.id)
            .await
            .expect("read title")
            .is_some(),
        "the stored delete step never runs"
    );
    assert!(fixture.evaluation.all_action_runs().await.is_empty());
    assert_eq!(
        fixture.evaluation.all_candidates().await[0].state,
        candidate.state,
        "the open candidate is left as it was"
    );

    let refused = fixture
        .app
        .set_lifecycle_rule_state(
            &fixture.user,
            &rule_set_id,
            scryer_domain::LifecycleRuleState::Enabled,
            None,
            Some(0),
        )
        .await
        .expect_err("a refused rule cannot be enabled");
    assert_eq!(
        refused.validation_reason(),
        Some(crate::error_reasons::lifecycle::RULE_ACTIONS_UNREADABLE),
        "{refused:?}"
    );
}

/// A rule is scheduled from its latest evaluation, and a rule holding an
/// interrupted (`executing`) candidate is marked so it resumes at once.
#[tokio::test]
async fn a_scheduled_rule_loads_its_last_run_and_interrupted_sequences() {
    use crate::scripts::scheduler::CustomJobSource;
    let fixture = execution_app(None);
    seed_title(&fixture.app, &fixture.user, "Schedule anchor", true).await;
    let (rule_set_id, candidate) =
        arm_and_evaluate(&fixture, sequence_draft(vec![monitor_step()])).await;
    let latest_run = fixture
        .app
        .list_lifecycle_evaluation_runs(&fixture.user, Some(&rule_set_id), Some(1))
        .await
        .expect("runs")
        .into_iter()
        .next()
        .expect("the evaluation recorded a run");

    let loaded = fixture
        .app
        .load_scheduled_lifecycle_rules()
        .await
        .expect("load");
    assert_eq!(loaded.len(), 1, "{loaded:?}");
    assert_eq!(loaded[0].last_run_at, Some(latest_run.started_at));
    assert!(!loaded[0].in_flight);

    fixture
        .evaluation
        .strand_as_executing(&candidate.id, Utc::now())
        .await;
    let loaded = fixture
        .app
        .load_scheduled_lifecycle_rules()
        .await
        .expect("load");
    assert!(loaded[0].in_flight, "{loaded:?}");
}

/// A scheduled rule fire is tracked work: a restore drain waits for it to
/// finish, and once draining begins no new fire starts.
#[tokio::test]
async fn a_restore_drain_waits_for_an_in_flight_rule_fire() {
    use crate::scripts::scheduler::{CustomJobDispatcher, CustomJobTarget, DueCustomJob};
    let fixture = execution_app(None);
    seed_title(&fixture.app, &fixture.user, "Drained fire", true).await;
    let (rule_set_id, _candidate) =
        arm_and_evaluate(&fixture, sequence_draft(vec![monitor_step()])).await;
    let due = DueCustomJob {
        target: CustomJobTarget::LifecycleRule,
        script_id: rule_set_id.clone(),
        trigger_source: crate::JobTriggerSource::ScheduledInterval,
        execution_mode: scryer_domain::ExecutionMode::FireAndForget,
    };
    let runs_before = fixture.evaluation.all_runs().await.len();

    // The fire is spawned and has not run yet on this single-threaded
    // runtime; the drain must be what lets it finish.
    fixture.app.dispatch(due.clone()).await;
    fixture
        .app
        .restore_coordinator()
        .drain(&crate::RestoreDeadline::new())
        .await
        .expect("drain");
    let runs = fixture.evaluation.all_runs().await;
    assert_eq!(runs.len(), runs_before + 1, "the drain waited for the fire");
    assert!(
        runs.iter().all(|run| run.finished_at.is_some()),
        "no evaluation is left half-written: {runs:?}"
    );
    assert!(
        fixture
            .app
            .runtime
            .jobs
            .custom_job_scheduler
            .begin_lifecycle_rule_fire(&rule_set_id)
            .is_some(),
        "the fire released its rule"
    );

    fixture.app.dispatch(due).await;
    tokio::task::yield_now().await;
    assert_eq!(
        fixture.evaluation.all_runs().await.len(),
        runs_before + 1,
        "no fire starts after a drain began"
    );
}

/// A manual run of one rule takes the same claim as a scheduled fire, so it
/// never evaluates beside a fire of the same rule.
#[tokio::test]
async fn a_manual_rule_run_refuses_while_that_rule_is_running() {
    let fixture = execution_app(None);
    seed_title(&fixture.app, &fixture.user, "Busy rule", true).await;
    let (rule_set_id, _candidate) =
        arm_and_evaluate(&fixture, sequence_draft(vec![monitor_step()])).await;
    let runs_before = fixture.evaluation.all_runs().await.len();
    let fire = fixture
        .app
        .runtime
        .jobs
        .custom_job_scheduler
        .begin_lifecycle_rule_fire(&rule_set_id)
        .expect("a fire holds the rule");

    let refused = fixture
        .app
        .run_lifecycle_evaluation_now(&fixture.user, Some(rule_set_id.clone()))
        .await
        .expect("trigger");
    assert!(!refused.started, "{refused:?}");
    assert_eq!(fixture.evaluation.all_runs().await.len(), runs_before);

    drop(fire);
    let started = fixture
        .app
        .run_lifecycle_evaluation_now(&fixture.user, Some(rule_set_id))
        .await
        .expect("trigger");
    assert!(started.started, "{started:?}");
    assert_eq!(fixture.evaluation.all_runs().await.len(), runs_before + 1);
}

/// A candidate with no recorded decision has nothing it may run: it is held
/// with a stable reason, and its title is untouched.
#[tokio::test]
async fn a_candidate_without_decided_actions_cannot_execute() {
    let fixture = execution_app(None);
    let title = seed_title(&fixture.app, &fixture.user, "No decision", false).await;
    let (_rule_set_id, candidate) = arm_and_evaluate(
        &fixture,
        sequence_draft(vec![LifecycleActionStep {
            id: "delete_title".to_string(),
            kind: LifecycleActionStepKind::DeleteTitleAndFiles,
            parameters: LifecycleActionStepParameters::None,
        }]),
    )
    .await;
    fixture
        .evaluation
        .forget_decided_actions(&candidate.id)
        .await;
    fixture.evaluation.end_every_grace().await;

    let report = fixture
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("action pass");
    assert_eq!(report.executed, 0, "{report:?}");
    let held = &fixture.evaluation.all_candidates().await[0];
    assert_eq!(held.state, LifecycleCandidateState::Blocked, "{held:?}");
    assert_eq!(
        held.state_reason,
        crate::lifecycle_rules::execution_reason::ACTIONS_NOT_RUNNABLE
    );
    assert!(
        fixture
            .app
            .get_title(&fixture.user, &title.id)
            .await
            .expect("read title")
            .is_some()
    );
}

/// A recorded decision naming a head the rule's revision does not declare is
/// never run: the candidate is held and the title is untouched.
#[tokio::test]
async fn a_decision_naming_an_undeclared_head_is_held() {
    let fixture = execution_app(None);
    let title = seed_title(&fixture.app, &fixture.user, "Undeclared head", false).await;
    let (_rule_set_id, candidate) = arm_and_evaluate(
        &fixture,
        sequence_draft(vec![LifecycleActionStep {
            id: "delete_files".to_string(),
            kind: LifecycleActionStepKind::DeleteFiles,
            parameters: LifecycleActionStepParameters::None,
        }]),
    )
    .await;
    fixture
        .evaluation
        .force_decided_actions(
            &candidate.id,
            &scryer_rules::lifecycle_actions::LifecycleActions {
                delete_files: true,
                delete_title: true,
                ..Default::default()
            },
        )
        .await;
    fixture.evaluation.end_every_grace().await;

    let report = fixture
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("action pass");
    assert_eq!(report.executed, 0, "{report:?}");
    let held = &fixture.evaluation.all_candidates().await[0];
    assert_eq!(held.state, LifecycleCandidateState::Blocked, "{held:?}");
    assert_eq!(
        held.state_reason,
        crate::lifecycle_rules::execution_reason::ACTION_NOT_DECLARED
    );
    assert!(
        fixture
            .app
            .get_title(&fixture.user, &title.id)
            .await
            .expect("read title")
            .is_some()
    );
}

fn decision_change_draft(rego_source: &str, grace_days: i64) -> LifecycleRuleDraft {
    LifecycleRuleDraft {
        max_titles_per_run: Some(100),
        name: "Decision change regression".to_string(),
        description: String::new(),
        rego_source: rego_source.to_string(),
        grace_days,
        library_ids: Vec::new(),
    }
}

/// Flip the title's monitoring and evaluate again, returning the candidate
/// that is open afterwards.
async fn reevaluate_after_monitoring(
    fixture: &crate::lib_tests::lifecycle_execution::ExecutionFixture,
    title_id: &str,
    monitored: bool,
) -> scryer_domain::LifecycleCandidate {
    fixture
        .app
        .set_title_monitored(&fixture.user, title_id, monitored)
        .await
        .expect("flip monitoring");
    fixture
        .app
        .run_lifecycle_rule_evaluation_job()
        .await
        .expect("evaluate again");
    let candidates = fixture.evaluation.all_candidates().await;
    let open: Vec<_> = candidates
        .iter()
        .filter(|row| row.state != LifecycleCandidateState::Canceled)
        .cloned()
        .collect();
    assert_eq!(open.len(), 1, "{candidates:?}");
    assert!(
        candidates
            .iter()
            .any(|row| row.state == LifecycleCandidateState::Canceled
                && row.state_reason == "actions_changed"),
        "{candidates:?}"
    );
    open.into_iter().next().expect("open candidate")
}

/// A changed decision that deletes nothing new keeps the clock and the
/// attempts already spent, so flipping decisions cannot reset the bound on
/// retries of a candidate that is already due.
#[tokio::test]
async fn a_changed_decision_keeps_the_clock_and_attempts_when_it_deletes_nothing_new() {
    let fixture = execution_app(None);
    let title = seed_title(&fixture.app, &fixture.user, "Decision change", false).await;
    let (_rule_set_id, first) = arm_and_evaluate(
        &fixture,
        decision_change_draft(
            "match := true\n\nsearch if {\n\tmatch\n}\n\nunmonitor if {\n\tinput.facts.monitored\n}\n",
            3,
        ),
    )
    .await;
    fixture.evaluation.set_action_attempts(&first.id, 2).await;
    fixture.evaluation.end_every_grace().await;
    let due_at = fixture.evaluation.all_candidates().await[0].due_at;

    let next = reevaluate_after_monitoring(&fixture, &title.id, true).await;
    assert_ne!(next.id, first.id);
    assert_eq!(next.action_attempts, 2, "{next:?}");
    assert_eq!(next.due_at, due_at, "the past deadline carries over");
    assert_eq!(next.first_matched_at, first.first_matched_at);
}

/// Deleting the title where the stored decision deleted only files is a new
/// delete: the grace and the attempts start over.
#[tokio::test]
async fn a_changed_decision_that_deletes_something_new_restarts_grace() {
    let fixture = execution_app(None);
    let title = seed_title(&fixture.app, &fixture.user, "Delete widening", false).await;
    let (_rule_set_id, first) = arm_and_evaluate(
        &fixture,
        decision_change_draft(
            "match := true\n\ndelete_files if {\n\tnot input.facts.monitored\n}\n\n\
             delete_title if {\n\tinput.facts.monitored\n}\n",
            7,
        ),
    )
    .await;
    fixture.evaluation.set_action_attempts(&first.id, 2).await;
    // The stored candidate's grace has run out; a carried clock would leave
    // the new delete due at once.
    fixture.evaluation.end_every_grace().await;
    let before = Utc::now();

    let next = reevaluate_after_monitoring(&fixture, &title.id, true).await;
    assert_eq!(next.action_attempts, 0, "{next:?}");
    assert!(next.first_matched_at >= before, "{next:?}");
    assert_eq!(next.due_at, next.first_matched_at + Duration::days(7));
}

/// The delete budget follows the subject across a recreate. The first pass
/// unmonitors and its delete fails; evaluation does not count that unmonitor
/// against the rule, so the recreate comes from outside: a tag added since
/// means the rule no longer decides `unmonitor`, and the next evaluation
/// replaces the candidate. The new candidate's delete continues at attempt
/// two, and the delete is attempted three times in all, never more.
#[tokio::test]
async fn a_recreated_candidate_keeps_the_failed_delete_attempts_it_carries() {
    let fixture = title_fixture("Carried delete budget", MediaFacet::Movie).await;
    let folder = fixture.root.join("carried");
    std::fs::create_dir(&folder).expect("create title folder");
    let path = folder.join("carried.mkv");
    add_catalog_movie_file_without_physical_path(&fixture.execution.app, &fixture.title.id, &path)
        .await;
    std::fs::write(&path, b"admit the configured-root title").expect("create file");
    super::lifecycle_title_completion::record_fixture_title_folder(
        &fixture.execution.app,
        &fixture.title.id,
        &folder,
    )
    .await;
    let mut draft = storage_sequence_deletion_draft();
    draft.rego_source = "match := true\n\nunmonitor if {\n\tinput.facts.monitored\n\
                         \tnot \"synthetic-settled\" in input.facts.tags\n}\n\n\
                         delete_files if {\n\tmatch\n}\n"
        .to_string();
    define_title_tags(&fixture.execution, &["synthetic-settled"]).await;
    sample_fixture_root(&fixture);
    let (rule_set_id, candidates) = arm_and_evaluate_all(&fixture.execution, draft).await;
    assert_eq!(candidates.len(), 1, "{candidates:?}");
    super::lifecycle_rules::arm_rule(
        &fixture.execution.app,
        &fixture.execution.user,
        &rule_set_id,
        LifecycleRuleState::Enabled,
        Some(1),
    )
    .await;
    fixture
        .execution
        .media_files
        .fail_delete_media_file("fixture delete failure")
        .await;
    let delete_attempts = async || {
        fixture
            .execution
            .evaluation
            .all_step_attempts()
            .await
            .into_iter()
            .filter(|attempt| attempt.key.step_id == "delete_files")
            .map(|attempt| (attempt.key.candidate_id, attempt.attempt, attempt.state))
            .collect::<Vec<_>>()
    };

    fixture
        .execution
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("first pass");
    let first = delete_attempts().await;
    assert_eq!(first.len(), 1, "{first:?}");
    assert_eq!(first[0].1, 1);

    // The injected fault is in the catalog cleanup after the file left the
    // disk; put the file back so the root still holds the title.
    std::fs::write(&path, b"admit the configured-root title").expect("restore file");
    // Tagged from outside: the decision changes without deleting more.
    fixture
        .execution
        .app
        .update_title_tags(
            &fixture.execution.user,
            std::slice::from_ref(&fixture.title.id),
            &["synthetic-settled".to_string()],
            &[],
        )
        .await
        .expect("tag the title from outside");
    fixture
        .execution
        .app
        .run_lifecycle_rule_evaluation_job()
        .await
        .expect("re-evaluate");
    let open: Vec<_> = fixture
        .execution
        .evaluation
        .all_candidates()
        .await
        .into_iter()
        .filter(|candidate| candidate.state != LifecycleCandidateState::Canceled)
        .collect();
    assert_eq!(open.len(), 1, "{open:?}");
    assert_ne!(
        open[0].id,
        first[0].0,
        "the candidate was replaced: {open:#?} {:#?}",
        fixture.execution.evaluation.all_step_attempts().await
    );

    for _ in 0..4 {
        std::fs::write(&path, b"admit the configured-root title").expect("restore file");
        fixture.execution.evaluation.end_every_grace().await;
        fixture.execution.evaluation.begin_next_evaluation().await;
        fixture
            .execution
            .app
            .run_every_rule_action_pass_for_tests()
            .await
            .expect("later pass");
    }
    let all = delete_attempts().await;
    assert_eq!(
        all.iter()
            .map(|(_, attempt, _)| *attempt)
            .collect::<Vec<_>>(),
        vec![1, 2, 3],
        "the replacement continues the budget: {all:?}"
    );
    assert!(
        all.iter()
            .all(|(_, _, state)| *state == LifecycleActionStepState::Failed)
    );
}

fn monitor_step() -> LifecycleActionStep {
    LifecycleActionStep {
        id: "monitor".to_string(),
        kind: LifecycleActionStepKind::Monitor,
        parameters: LifecycleActionStepParameters::None,
    }
}

fn with_notify(mut draft: LifecycleRuleDraft, message: &str) -> LifecycleRuleDraft {
    draft.rego_source.push_str(&format!(
        "\nnotify := {} if {{\n\tmatch\n}}\n",
        serde_json::to_string(message).expect("a string always serializes")
    ));
    draft
}

/// The `lifecycle_rule_notified` events stored so far.
async fn notified_events(
    events: &crate::lib_tests::MockDomainEventRepo,
) -> Vec<scryer_domain::LifecycleRuleNotifiedEventData> {
    events
        .events
        .lock()
        .await
        .iter()
        .filter_map(|event| match &event.payload {
            scryer_domain::DomainEventPayload::LifecycleRuleNotified(data) => Some(data.clone()),
            _ => None,
        })
        .collect()
}

fn execution_app_with_events() -> (
    crate::lib_tests::lifecycle_execution::ExecutionFixture,
    std::sync::Arc<crate::lib_tests::MockDomainEventRepo>,
) {
    let mut fixture = execution_app(None);
    let events = std::sync::Arc::new(crate::lib_tests::MockDomainEventRepo::default());
    fixture.app = fixture.app.with_test_overrides({
        let events = events.clone();
        move |services| services.with_domain_events(events)
    });
    (fixture, events)
}

#[tokio::test]
async fn a_monitor_rule_monitors_the_title_and_notifies_once() {
    let (fixture, events) = execution_app_with_events();
    let title = seed_title(&fixture.app, &fixture.user, "Synthetic Monitor", false).await;
    let (rule_set_id, candidate) = arm_and_evaluate(
        &fixture,
        with_notify(sequence_draft(vec![monitor_step()]), "Watch this again"),
    )
    .await;

    let report = fixture
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("execute sequence");
    assert_eq!(report.executed, 1, "{report:?}");
    assert!(
        fixture
            .app
            .get_title(&fixture.user, &title.id)
            .await
            .expect("read title")
            .expect("title exists")
            .monitored
    );
    let steps = fixture
        .evaluation
        .list_action_steps(
            &candidate.id,
            candidate.match_generation,
            candidate.revision_number,
        )
        .await
        .expect("read durable steps");
    assert_eq!(steps.len(), 1, "notify is not a step: {steps:?}");
    assert_eq!(steps[0].key.step_id, "monitor");
    assert!(steps[0].state.is_completion_success());

    let notified = notified_events(&events).await;
    assert_eq!(notified.len(), 1, "{notified:?}");
    assert_eq!(notified[0].rule_set_id, rule_set_id);
    assert_eq!(notified[0].rule_name, "Sequence executor regression");
    assert_eq!(notified[0].message, "Watch this again");
    assert_eq!(notified[0].subject_label, "Synthetic Monitor");
    assert_eq!(notified[0].title.title_name, "Synthetic Monitor");
    assert_eq!(notified[0].subject_id, title.id);

    // Later passes find the candidate finished and send nothing more.
    for _ in 0..2 {
        fixture
            .app
            .run_lifecycle_rule_evaluation_job()
            .await
            .expect("re-evaluate");
        fixture
            .app
            .run_every_rule_action_pass_for_tests()
            .await
            .expect("later action pass");
    }
    assert_eq!(notified_events(&events).await.len(), 1);
}

/// A rule whose only action stops applying once it has run, while `notify`
/// keeps matching: the decision drops the step, which recreates the
/// candidate with nothing left to do.
fn self_retiring_draft(action_head: &str, message: &str) -> LifecycleRuleDraft {
    LifecycleRuleDraft {
        max_titles_per_run: Some(100),
        name: "Sequence executor regression".to_string(),
        description: String::new(),
        rego_source: format!(
            "{ALWAYS_SOURCE}\n{action_head}\n\nnotify := {} if {{\n\tmatch\n}}\n",
            serde_json::to_string(message).expect("a string always serializes")
        ),
        grace_days: 0,
        library_ids: Vec::new(),
    }
}

async fn run_several_passes(fixture: &crate::lib_tests::lifecycle_execution::ExecutionFixture) {
    for _ in 0..3 {
        fixture
            .app
            .run_lifecycle_rule_evaluation_job()
            .await
            .expect("re-evaluate");
        fixture
            .app
            .run_every_rule_action_pass_for_tests()
            .await
            .expect("later action pass");
    }
}

#[tokio::test]
async fn an_unmonitor_if_monitored_rule_notifies_once_for_its_one_act() {
    let (fixture, events) = execution_app_with_events();
    let title = seed_title(
        &fixture.app,
        &fixture.user,
        "Synthetic Unmonitor Notify",
        true,
    )
    .await;
    arm_and_evaluate(
        &fixture,
        self_retiring_draft(
            "unmonitor if {\n\tmatch\n\tinput.facts.monitored\n}",
            "Unmonitored",
        ),
    )
    .await;
    let report = fixture
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("execute sequence");
    assert_eq!(report.executed, 1, "{report:?}");
    assert!(
        !fixture
            .app
            .get_title(&fixture.user, &title.id)
            .await
            .expect("read title")
            .expect("title exists")
            .monitored
    );
    assert_eq!(notified_events(&events).await.len(), 1);

    // The rule still matches through `notify`, without the step it ran.
    run_several_passes(&fixture).await;
    assert_eq!(
        notified_events(&events).await.len(),
        1,
        "a recreated candidate with nothing left to do must not notify again"
    );
}

#[tokio::test]
async fn an_add_tags_rule_notifies_once_for_its_one_act() {
    let (fixture, events) = execution_app_with_events();
    define_title_tags(&fixture, &["synthetic-kept"]).await;
    let title = seed_title(&fixture.app, &fixture.user, "Synthetic Tag Notify", true).await;
    arm_and_evaluate(
        &fixture,
        self_retiring_draft(
            "add_tags := [\"synthetic-kept\"] if {\n\tmatch\n\tnot \"synthetic-kept\" in input.facts.tags\n}",
            "Tagged",
        ),
    )
    .await;
    let report = fixture
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("execute sequence");
    assert_eq!(report.executed, 1, "{report:?}");
    assert!(
        fixture
            .app
            .get_title(&fixture.user, &title.id)
            .await
            .expect("read title")
            .expect("title exists")
            .tags
            .iter()
            .any(|tag| tag == "synthetic-kept")
    );
    assert_eq!(notified_events(&events).await.len(), 1);

    run_several_passes(&fixture).await;
    assert_eq!(
        notified_events(&events).await.len(),
        1,
        "a recreated candidate with nothing left to do must not notify again"
    );
}

#[tokio::test]
async fn a_notify_only_rule_notifies_once_per_membership() {
    let (fixture, events) = execution_app_with_events();
    seed_title(&fixture.app, &fixture.user, "Synthetic Notify Only", true).await;
    arm_and_evaluate(&fixture, self_retiring_draft("", "Just so you know")).await;
    fixture
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("execute sequence");
    assert_eq!(notified_events(&events).await.len(), 1);
    run_several_passes(&fixture).await;
    assert_eq!(notified_events(&events).await.len(), 1);
}

#[tokio::test]
async fn a_rule_that_would_undo_another_rules_monitoring_holds_instead_of_flipping_it() {
    let fixture = execution_app(None);
    let title = seed_title(&fixture.app, &fixture.user, "Synthetic Tug Of War", false).await;
    let monitored = || async {
        fixture
            .app
            .get_title(&fixture.user, &title.id)
            .await
            .expect("read title")
            .expect("title exists")
            .monitored
    };
    let draft = |name: &str, head: &str| LifecycleRuleDraft {
        max_titles_per_run: Some(100),
        name: name.to_string(),
        rego_source: format!("{ALWAYS_SOURCE}\n{head}\n"),
        ..sequence_draft(Vec::new())
    };
    let (monitor_rule, _) = arm_and_evaluate_all(
        &fixture,
        draft(
            "Synthetic monitor side",
            "monitor if {\n\tmatch\n\tnot input.facts.monitored\n}",
        ),
    )
    .await;
    let (unmonitor_rule, _) = arm_and_evaluate_all(
        &fixture,
        draft(
            "Synthetic unmonitor side",
            "unmonitor if {\n\tmatch\n\tinput.facts.monitored\n}",
        ),
    )
    .await;

    // The title starts unmonitored: only the monitor side matches, and acts.
    fixture
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("first action pass");
    assert!(monitored().await);

    // Now the unmonitor side matches, but undoing the other rule's change
    // would flip the title every cycle: it holds, however often it is asked.
    for _ in 0..3 {
        fixture
            .app
            .run_lifecycle_rule_evaluation_job()
            .await
            .expect("re-evaluate");
        fixture
            .app
            .run_every_rule_action_pass_for_tests()
            .await
            .expect("later action pass");
        assert!(
            monitored().await,
            "the unmonitor side must not undo the monitor"
        );
    }
    let candidates = fixture.evaluation.all_candidates().await;
    let held = candidates
        .iter()
        .find(|candidate| {
            candidate.rule_set_id == unmonitor_rule
                && candidate.state == LifecycleCandidateState::Blocked
        })
        .unwrap_or_else(|| panic!("the unmonitor side holds: {candidates:?}"));
    assert_eq!(
        held.state_reason,
        crate::lifecycle_rules::action_execution::execution_reason::MONITORING_CONFLICT
    );
    assert!(
        candidates
            .iter()
            .filter(|candidate| candidate.rule_set_id == monitor_rule)
            .all(|candidate| candidate.state != LifecycleCandidateState::Blocked),
        "{candidates:?}"
    );

    // Holding again writes nothing.
    let before = fixture.evaluation.all_candidates().await;
    fixture
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("idle action pass");
    assert_eq!(
        fixture
            .evaluation
            .all_candidates()
            .await
            .iter()
            .find(|candidate| candidate.id == held.id)
            .map(|candidate| candidate.updated_at),
        before
            .iter()
            .find(|candidate| candidate.id == held.id)
            .map(|candidate| candidate.updated_at)
    );

    // Once the other rule's change ages out of the window, the hold lifts.
    fixture
        .evaluation
        .age_step_attempts(chrono::Duration::seconds(
            crate::lifecycle_rules::action_execution::LIFECYCLE_MONITORING_CONFLICT_WINDOW_SECONDS
                + 60,
        ))
        .await;
    fixture
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("action pass after the window");
    assert!(!monitored().await);
}

#[tokio::test]
async fn a_dry_run_rule_never_notifies() {
    let (fixture, events) = execution_app_with_events();
    let title = seed_title(&fixture.app, &fixture.user, "Synthetic Dry Notify", false).await;
    let created = fixture
        .app
        .create_lifecycle_rule_set(
            &fixture.user,
            with_notify(sequence_draft(vec![monitor_step()]), "Dry run only"),
        )
        .await
        .expect("create rule");
    super::lifecycle_rules::set_rule_state(
        &fixture.app,
        &fixture.user,
        &created.rule_set.id,
        scryer_domain::LifecycleRuleState::DryRun,
        None,
    )
    .await;
    fixture
        .app
        .set_lifecycle_instance_gates(
            &fixture.user,
            LifecycleGatesUpdate {
                enabled: Some(true),
                actions_paused: Some(false),
            },
        )
        .await
        .expect("enable lifecycle");
    for _ in 0..2 {
        fixture
            .app
            .run_lifecycle_rule_evaluation_job()
            .await
            .expect("evaluate");
        let report = fixture
            .app
            .run_every_rule_action_pass_for_tests()
            .await
            .expect("action pass");
        assert_eq!(report.executed, 0, "{report:?}");
    }
    assert!(notified_events(&events).await.is_empty());
    assert!(
        !fixture
            .app
            .get_title(&fixture.user, &title.id)
            .await
            .expect("read title")
            .expect("title exists")
            .monitored
    );
}

#[tokio::test]
async fn conflicting_decided_actions_hold_with_a_code_and_change_nothing() {
    let (fixture, events) = execution_app_with_events();
    let title = seed_title(&fixture.app, &fixture.user, "Synthetic Conflict", false).await;
    let (_rule_set_id, candidate) = arm_and_evaluate(
        &fixture,
        with_notify(
            sequence_draft(vec![
                monitor_step(),
                LifecycleActionStep {
                    id: "unmonitor".to_string(),
                    kind: LifecycleActionStepKind::Unmonitor,
                    parameters: LifecycleActionStepParameters::Unmonitor {
                        include_descendants: false,
                    },
                },
            ]),
            "Never sent",
        ),
    )
    .await;
    fixture.evaluation.end_every_grace().await;

    let report = fixture
        .app
        .run_every_rule_action_pass_for_tests()
        .await
        .expect("action pass");
    assert_eq!(report.executed, 0, "{report:?}");
    let held = &fixture.evaluation.all_candidates().await[0];
    assert_eq!(held.state, LifecycleCandidateState::Blocked, "{held:?}");
    assert_eq!(
        held.state_reason,
        crate::lifecycle_rules::execution_reason::ACTIONS_CONFLICT
    );
    assert!(
        fixture
            .evaluation
            .list_action_steps(
                &candidate.id,
                candidate.match_generation,
                candidate.revision_number,
            )
            .await
            .expect("read steps")
            .is_empty()
    );
    assert!(
        !fixture
            .app
            .get_title(&fixture.user, &title.id)
            .await
            .expect("read title")
            .expect("title exists")
            .monitored
    );
    assert!(notified_events(&events).await.is_empty());
}

/// Creates a rule, opens the gates and arms it without evaluating anything.
async fn create_and_arm(
    fixture: &crate::lib_tests::lifecycle_execution::ExecutionFixture,
    draft: LifecycleRuleDraft,
) -> String {
    let created = fixture
        .app
        .create_lifecycle_rule_set(&fixture.user, draft)
        .await
        .expect("create rule");
    let rule_set_id = created.rule_set.id.clone();
    super::lifecycle_rules::set_rule_state(
        &fixture.app,
        &fixture.user,
        &rule_set_id,
        scryer_domain::LifecycleRuleState::DryRun,
        None,
    )
    .await;
    fixture
        .app
        .set_lifecycle_instance_gates(
            &fixture.user,
            LifecycleGatesUpdate {
                enabled: Some(true),
                actions_paused: Some(false),
            },
        )
        .await
        .expect("enable lifecycle and unpause actions");
    let state = enabled_state_for(&created.declared_actions);
    super::lifecycle_rules::arm_rule(&fixture.app, &fixture.user, &rule_set_id, state, None).await;
    rule_set_id
}

/// The most a rule declaring these actions can be set to: a rule that
/// declares none only runs as a dry run.
fn enabled_state_for(
    declared: &crate::lifecycle_rules::LifecycleDeclaredActions,
) -> LifecycleRuleState {
    if declared.heads.is_empty() {
        LifecycleRuleState::DryRun
    } else {
        LifecycleRuleState::Enabled
    }
}

#[tokio::test]
async fn a_scheduled_fire_evaluates_then_acts_for_that_rule_only() {
    let fixture = execution_app(None);
    let title = seed_title(&fixture.app, &fixture.user, "Synthetic Scheduled", false).await;
    let fired = create_and_arm(&fixture, sequence_draft(vec![monitor_step()])).await;
    let other = create_and_arm(&fixture, sequence_draft(vec![monitor_step()])).await;

    let (evaluation, actions) = fixture
        .app
        .run_scheduled_lifecycle_rule(&fired)
        .await
        .expect("scheduled fire");
    assert_eq!(evaluation.rules_evaluated, 1, "{evaluation:?}");
    assert_eq!(actions.rules_eligible, 1, "{actions:?}");
    assert_eq!(actions.executed, 1, "{actions:?}");

    let candidates = fixture.evaluation.all_candidates().await;
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.rule_set_id == fired),
        "the other rule was neither evaluated nor acted on: {candidates:?}"
    );
    assert_eq!(candidates.len(), 1);
    assert!(
        fixture
            .app
            .get_title(&fixture.user, &title.id)
            .await
            .expect("read title")
            .expect("title exists")
            .monitored
    );

    // The other rule's own fire then runs it, and only it.
    let (evaluation, _) = fixture
        .app
        .run_scheduled_lifecycle_rule(&other)
        .await
        .expect("other fire");
    assert_eq!(evaluation.rules_evaluated, 1);
    assert_eq!(
        fixture
            .evaluation
            .all_candidates()
            .await
            .iter()
            .filter(|candidate| candidate.rule_set_id == other)
            .count(),
        1
    );
}

#[tokio::test]
async fn a_rule_runs_on_the_default_schedule_until_one_is_chosen() {
    use crate::scripts::scheduler::CustomJobSource;
    use scryer_domain::ScriptSchedule;

    let fixture = execution_app(None);
    let created = fixture
        .app
        .create_lifecycle_rule_set(&fixture.user, sequence_draft(vec![monitor_step()]))
        .await
        .expect("create rule");
    assert_eq!(
        created.rule_set.schedule,
        scryer_domain::default_lifecycle_rule_schedule()
    );
    assert_eq!(
        created.rule_set.schedule,
        ScriptSchedule::Interval {
            every_seconds: 8 * 60 * 60
        }
    );
    let id = created.rule_set.id.clone();

    // A disabled rule is not scheduled.
    assert!(
        fixture
            .app
            .load_scheduled_lifecycle_rules()
            .await
            .expect("load")
            .is_empty()
    );

    let refused = fixture
        .app
        .update_lifecycle_rule_metadata_with_schedule(
            &fixture.user,
            &id,
            created.rule_set.name.clone(),
            String::new(),
            Vec::new(),
            Some(ScriptSchedule::Interval { every_seconds: 5 }),
            None,
        )
        .await
        .expect_err("an interval below the floor is refused");
    assert_eq!(
        refused.validation_reason(),
        Some(crate::error_reasons::scripts::SCHEDULE_INVALID)
    );
    let refused = fixture
        .app
        .create_lifecycle_rule_set_with_schedule(
            &fixture.user,
            sequence_draft(vec![monitor_step()]),
            crate::rules::builder_model::BuilderModelChange::Keep,
            Some(ScriptSchedule::Daily {
                time_local: "25:00".to_string(),
            }),
        )
        .await
        .expect_err("an impossible time is refused");
    assert_eq!(
        refused.validation_reason(),
        Some(crate::error_reasons::scripts::SCHEDULE_INVALID)
    );

    super::lifecycle_rules::set_rule_state(
        &fixture.app,
        &fixture.user,
        &id,
        scryer_domain::LifecycleRuleState::DryRun,
        None,
    )
    .await;
    let daily = ScriptSchedule::Daily {
        time_local: "03:30".to_string(),
    };
    let updated = fixture
        .app
        .update_lifecycle_rule_metadata_with_schedule(
            &fixture.user,
            &id,
            created.rule_set.name.clone(),
            String::new(),
            Vec::new(),
            Some(daily.clone()),
            None,
        )
        .await
        .expect("reschedule");
    assert_eq!(updated.schedule, daily);
    let stored = fixture
        .app
        .get_lifecycle_rule_set(&fixture.user, &id)
        .await
        .expect("read rule")
        .expect("rule exists");
    assert_eq!(stored.rule_set.schedule, daily);
    assert_eq!(
        fixture
            .app
            .load_scheduled_lifecycle_rules()
            .await
            .expect("load")
            .into_iter()
            .map(|rule| (rule.id, rule.schedule))
            .collect::<Vec<_>>(),
        vec![(id.clone(), daily.clone())]
    );
    assert!(
        fixture
            .app
            .runtime
            .jobs
            .custom_job_scheduler
            .lifecycle_rule_next_run_at(&id)
            .is_some(),
        "the reschedule reloaded the scheduler"
    );

    // A metadata save without a schedule keeps the stored one.
    let kept = fixture
        .app
        .update_lifecycle_rule_metadata(
            &fixture.user,
            &id,
            "Renamed".to_string(),
            String::new(),
            Vec::new(),
        )
        .await
        .expect("rename");
    assert_eq!(kept.schedule, daily);
}

/// A scheduled evaluation reads free space from the capacity owner's cache,
/// never the disk: record a fresh sample of the fixture root, as the sampler
/// would have.
fn sample_fixture_root(fixture: &TitleFixture) {
    fixture
        .execution
        .app
        .runtime
        .health
        .storage_capacity
        .record_for_test(
            &fixture.root.to_string_lossy(),
            Utc::now(),
            Some(crate::helpers::FilesystemSpace {
                total_bytes: 1_000,
                available_bytes: 100,
            }),
        );
}

//! End-to-end v2 sequence execution regressions.
//!
//! These use the normal candidate scheduler and durable in-memory repositories.
//! The completed-job adapter is private to the executor and simulates an
//! externally accepted job that has not finished yet; it is never an action a
//! user can place in a sequence.

use crate::lib_tests::maintenance_execution::{execution_app, seed_title};
use crate::lib_tests::maintenance_sequence_deletion::{
    add_catalog_movie_file_without_physical_path, selected_root_sequence_deletion_draft,
};
use crate::lib_tests::maintenance_title_completion::title_fixture;
use crate::maintenance_rules::sequence_execution::{
    TestCompletedSequenceJobMap, TestCompletedSequenceJobState,
    install_test_completed_sequence_job_adapter, set_test_completed_sequence_job_state,
};
use crate::maintenance_rules::{
    MaintenanceActionDefinition, MaintenanceActionSequence, MaintenanceActionStep,
    MaintenanceActionStepKind, MaintenanceActionStepParameters, MaintenanceGatesUpdate,
    MaintenanceRuleDraft, MaintenanceSearchCondition,
};
use crate::ports::{
    LifecycleActionRunRepository, MaintenanceActionStepRepository, MaintenanceCandidateRepository,
};
use chrono::{Duration, Utc};
use scryer_domain::{
    MaintenanceActionStepKey, MaintenanceActionStepRun, MaintenanceActionStepState,
    MaintenanceCandidateState, MaintenanceEffectArming, MaintenanceEvaluationMode,
    MaintenanceRuleSubjectKind, MediaFacet,
};
use std::collections::HashMap;

const ALWAYS_MATCHER: &str = "match := true\n";

pub(super) fn sequence_draft(steps: Vec<MaintenanceActionStep>) -> MaintenanceRuleDraft {
    MaintenanceRuleDraft {
        subject_kind: MaintenanceRuleSubjectKind::Title,
        name: "Sequence executor regression".to_string(),
        description: String::new(),
        rego_source: ALWAYS_MATCHER.to_string(),
        action_definition: MaintenanceActionDefinition::Sequence(MaintenanceActionSequence::new(
            steps,
        )),
        grace_days: 0,
        storage_root_id: None,
        library_ids: Vec::new(),
        evaluation_mode: None,
    }
}

pub(super) async fn arm_and_evaluate(
    fixture: &crate::lib_tests::maintenance_execution::ExecutionFixture,
    draft: MaintenanceRuleDraft,
) -> (String, scryer_domain::LifecycleCandidate) {
    let (rule_set_id, candidates) = arm_and_evaluate_all(fixture, draft).await;
    assert_eq!(candidates.len(), 1, "{candidates:?}");
    (
        rule_set_id,
        candidates.into_iter().next().expect("candidate"),
    )
}

async fn arm_and_evaluate_all(
    fixture: &crate::lib_tests::maintenance_execution::ExecutionFixture,
    draft: MaintenanceRuleDraft,
) -> (String, Vec<scryer_domain::LifecycleCandidate>) {
    let created = fixture
        .app
        .create_maintenance_rule_set(&fixture.user, draft)
        .await
        .expect("create sequence rule");
    let rule_set_id = created.rule_set.id;
    fixture
        .app
        .set_maintenance_rule_evaluation_mode(
            &fixture.user,
            &rule_set_id,
            MaintenanceEvaluationMode::Observe,
        )
        .await
        .expect("observe sequence rule");
    fixture
        .app
        .set_maintenance_instance_gates(
            &fixture.user,
            MaintenanceGatesUpdate {
                evaluation_enabled: Some(true),
                result_display_enabled: Some(true),
                reversible_effects_enabled: Some(true),
                destructive_effects_enabled: Some(true),
                ..Default::default()
            },
        )
        .await
        .expect("open maintenance gates");
    fixture
        .app
        .set_maintenance_rule_arming(
            &fixture.user,
            &rule_set_id,
            MaintenanceEffectArming::Reversible,
            None,
        )
        .await
        .expect("arm sequence rule");
    fixture
        .app
        .run_maintenance_rule_evaluation_job()
        .await
        .expect("evaluate sequence rule");
    let candidates = fixture.evaluation.all_candidates().await;
    (rule_set_id, candidates)
}

async fn define_title_tags(
    fixture: &crate::lib_tests::maintenance_execution::ExecutionFixture,
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
    fixture: &crate::lib_tests::maintenance_execution::ExecutionFixture,
    candidate: &scryer_domain::LifecycleCandidate,
    sequence: &MaintenanceActionSequence,
    step: &MaintenanceActionStep,
) -> MaintenanceActionStepKey {
    let abandoned_at = Utc::now() - Duration::minutes(61);
    let key = MaintenanceActionStepKey {
        candidate_id: candidate.id.clone(),
        match_generation: candidate.match_generation,
        revision_number: candidate.revision_number,
        step_id: step.id.clone(),
    };
    let run = MaintenanceActionStepRun {
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
            "changed_tags": ["crash-tag"],
            "expected_postcondition": "tags_present",
        })
        .to_string(),
        state: MaintenanceActionStepState::Running,
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
        crate::ports::MaintenanceActionStepClaim::Claimed(_)
    ));
    assert!(
        fixture
            .evaluation
            .transition_candidate_state(
                &candidate.id,
                MaintenanceCandidateState::Due,
                "seed_stale_sequence_lease",
                &[candidate.state.clone()],
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
    fixture: &crate::lib_tests::maintenance_execution::ExecutionFixture,
    candidate: &scryer_domain::LifecycleCandidate,
    key: &MaintenanceActionStepKey,
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
        claim.state = MaintenanceActionStepState::Running;
        claim.attempt = ordinal;
        let lease_id = format!("persisted-sequence-failure-{ordinal}");
        let mut failed = match fixture
            .evaluation
            .claim_action_step(&claim, now, &lease_id, now)
            .await
            .expect("claim seeded step for failure")
        {
            crate::ports::MaintenanceActionStepClaim::Claimed(run) => run,
            other => panic!("expected seeded step claim, got {other:?}"),
        };
        failed.state = MaintenanceActionStepState::Failed;
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
                MaintenanceCandidateState::Due,
                "seed_failed_sequence_step",
                &[MaintenanceCandidateState::Executing],
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
        sequence_draft(vec![MaintenanceActionStep {
            id: "unmonitor".to_string(),
            kind: MaintenanceActionStepKind::Unmonitor,
            parameters: MaintenanceActionStepParameters::Unmonitor {
                include_descendants: false,
            },
        }]),
    )
    .await;

    let report = fixture
        .app
        .run_lifecycle_action_handling_job()
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
    assert_eq!(candidates[0].state, MaintenanceCandidateState::Succeeded);
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
    let (_rule_set_id, candidate) = arm_and_evaluate(&fixture, sequence_draft(Vec::new())).await;

    let report = fixture
        .app
        .run_lifecycle_action_handling_job()
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
    let sequence = MaintenanceActionSequence::new(vec![MaintenanceActionStep {
        id: "tag".to_string(),
        kind: MaintenanceActionStepKind::AddTags,
        parameters: MaintenanceActionStepParameters::Tags {
            tags: vec!["crash-tag".to_string()],
        },
    }]);
    let (_rule_set_id, candidate) =
        arm_and_evaluate(&fixture, sequence_draft(sequence.steps.clone())).await;
    let key =
        seed_stale_running_add_tag_step(&fixture, &candidate, &sequence, &sequence.steps[0]).await;

    let report = fresh_runtime_app(&fixture.app)
        .run_lifecycle_action_handling_job()
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
    assert_eq!(step.state, MaintenanceActionStepState::Succeeded);
}

#[tokio::test]
async fn stale_intent_with_a_changed_prestate_holds_without_overwriting_live_tags() {
    let fixture = execution_app(None);
    let title = seed_title(&fixture.app, &fixture.user, "V2 crash changed tags", false).await;
    define_title_tags(&fixture, &["crash-tag", "external-tag"]).await;
    let sequence = MaintenanceActionSequence::new(vec![MaintenanceActionStep {
        id: "tag".to_string(),
        kind: MaintenanceActionStepKind::AddTags,
        parameters: MaintenanceActionStepParameters::Tags {
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
        .run_lifecycle_action_handling_job()
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
    assert_eq!(step.state, MaintenanceActionStepState::Running);
}

#[tokio::test]
async fn stale_intent_after_a_tag_mutation_recovers_as_already_satisfied() {
    let fixture = execution_app(None);
    let title = seed_title(&fixture.app, &fixture.user, "V2 crash after tag", false).await;
    define_title_tags(&fixture, &["crash-tag"]).await;
    let sequence = MaintenanceActionSequence::new(vec![MaintenanceActionStep {
        id: "tag".to_string(),
        kind: MaintenanceActionStepKind::AddTags,
        parameters: MaintenanceActionStepParameters::Tags {
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
        .run_lifecycle_action_handling_job()
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
    assert_eq!(step.state, MaintenanceActionStepState::AlreadySatisfied);
    assert_eq!(
        fixture.evaluation.all_candidates().await[0].state,
        MaintenanceCandidateState::Succeeded
    );
}

#[tokio::test]
async fn failed_checkpoint_after_tag_mutation_recovers_without_losing_its_attribution() {
    let fixture = execution_app(None);
    let title = seed_title(&fixture.app, &fixture.user, "V2 failed after tag", false).await;
    define_title_tags(&fixture, &["crash-tag"]).await;
    let sequence = MaintenanceActionSequence::new(vec![MaintenanceActionStep {
        id: "tag".to_string(),
        kind: MaintenanceActionStepKind::AddTags,
        parameters: MaintenanceActionStepParameters::Tags {
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
        .run_lifecycle_action_handling_job()
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
    assert_eq!(step.state, MaintenanceActionStepState::AlreadySatisfied);
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
    let sequence = MaintenanceActionSequence::new(vec![MaintenanceActionStep {
        id: "tag".to_string(),
        kind: MaintenanceActionStepKind::AddTags,
        parameters: MaintenanceActionStepParameters::Tags {
            tags: vec!["crash-tag".to_string()],
        },
    }]);
    let (_rule_set_id, candidate) =
        arm_and_evaluate(&fixture, sequence_draft(sequence.steps.clone())).await;
    let key =
        seed_stale_running_add_tag_step(&fixture, &candidate, &sequence, &sequence.steps[0]).await;
    finish_seeded_step_as_failed(&fixture, &candidate, &key, 3).await;

    let report = fresh_runtime_app(&fixture.app)
        .run_lifecycle_action_handling_job()
        .await
        .expect("terminalize exhausted step");
    assert_eq!(report.failed, 1, "{report:?}");
    assert_eq!(
        fixture.evaluation.all_candidates().await[0].state,
        MaintenanceCandidateState::Failed
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
    assert_eq!(step.state, MaintenanceActionStepState::Failed);
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
        sequence_draft(vec![MaintenanceActionStep {
            id: "search".to_string(),
            kind: MaintenanceActionStepKind::Search,
            parameters: MaintenanceActionStepParameters::Search {
                condition: MaintenanceSearchCondition::Unconditional,
            },
        }]),
    )
    .await;
    assert_eq!(
        unconditional
            .app
            .run_lifecycle_action_handling_job()
            .await
            .expect("accept unconditional maintenance search")
            .executed,
        1
    );
    let unconditional_key = MaintenanceActionStepKey {
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
        scryer_domain::MaintenanceActionJobReceiptState::Accepted
    );
    assert_eq!(
        fresh_runtime_app(&unconditional.app)
            .run_lifecycle_action_handling_job()
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
            MaintenanceActionStep {
                id: "profile".to_string(),
                kind: MaintenanceActionStepKind::ChangeQualityProfile,
                parameters: MaintenanceActionStepParameters::ChangeQualityProfile {
                    target_quality_profile_id: "4k".to_string(),
                },
            },
            MaintenanceActionStep {
                id: "search".to_string(),
                kind: MaintenanceActionStepKind::Search,
                parameters: MaintenanceActionStepParameters::Search {
                    condition: MaintenanceSearchCondition::PreviousProfileChanged,
                },
            },
        ]),
    )
    .await;
    assert_eq!(
        conditional
            .app
            .run_lifecycle_action_handling_job()
            .await
            .expect("accept profile-changed maintenance search")
            .executed,
        1
    );
    let conditional_key = MaintenanceActionStepKey {
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
        scryer_domain::MaintenanceActionJobReceiptState::Accepted
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
        .find(|step| step.key.step_id == "profile")
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
            .run_lifecycle_action_handling_job()
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
    let title = seed_title(&fixture.app, &fixture.user, "V2 completed job", false).await;
    define_title_tags(&fixture, &["after_completed_job"]).await;
    let (_rule_set_id, candidate) = arm_and_evaluate(
        &fixture,
        sequence_draft(vec![
            MaintenanceActionStep {
                id: "search".to_string(),
                kind: MaintenanceActionStepKind::Search,
                parameters: MaintenanceActionStepParameters::Search {
                    condition: MaintenanceSearchCondition::Unconditional,
                },
            },
            MaintenanceActionStep {
                id: "tag".to_string(),
                kind: MaintenanceActionStepKind::AddTags,
                parameters: MaintenanceActionStepParameters::Tags {
                    tags: vec!["after_completed_job".to_string()],
                },
            },
        ]),
    )
    .await;
    let key = MaintenanceActionStepKey {
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
        .run_lifecycle_action_handling_job()
        .await
        .expect("accepted job holds");
    assert_eq!(waiting.held, 1, "{waiting:?}");
    assert_eq!(
        fixture.evaluation.all_candidates().await[0].state,
        MaintenanceCandidateState::Blocked
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
                MaintenanceCandidateState::Due,
                "test_restart_due",
                &[MaintenanceCandidateState::Blocked],
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
    stale_step.state = MaintenanceActionStepState::Running;
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
        crate::ports::MaintenanceActionStepClaim::Claimed(_)
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
        .run_lifecycle_action_handling_job()
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
    let completed = restarted
        .run_lifecycle_action_handling_job()
        .await
        .expect("reconcile completed job");
    assert_eq!(completed.executed, 1, "{completed:?}");
    assert!(
        fixture
            .app
            .get_title(&fixture.user, &title.id)
            .await
            .expect("read title")
            .expect("title exists")
            .tags
            .iter()
            .any(|tag| tag == "after_completed_job")
    );
    assert_eq!(
        fixture.evaluation.all_candidates().await[0].state,
        MaintenanceCandidateState::Succeeded
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
            MaintenanceActionStep {
                id: "search".to_string(),
                kind: MaintenanceActionStepKind::Search,
                parameters: MaintenanceActionStepParameters::Search {
                    condition: MaintenanceSearchCondition::Unconditional,
                },
            },
            MaintenanceActionStep {
                id: "tag".to_string(),
                kind: MaintenanceActionStepKind::AddTags,
                parameters: MaintenanceActionStepParameters::Tags {
                    tags: vec!["after_retry".to_string()],
                },
            },
        ]),
    )
    .await;
    let key = MaintenanceActionStepKey {
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
        .run_lifecycle_action_handling_job()
        .await
        .expect("record confirmed job failure");
    assert_eq!(failed.failed, 1, "{failed:?}");
    assert_eq!(
        fixture.evaluation.all_candidates().await[0].state,
        MaintenanceCandidateState::Blocked
    );

    set_test_completed_sequence_job_state(&key, TestCompletedSequenceJobState::Completed);
    let retried = fixture
        .app
        .run_lifecycle_action_handling_job()
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
        sequence_draft(vec![MaintenanceActionStep {
            id: "search".to_string(),
            kind: MaintenanceActionStepKind::Search,
            parameters: MaintenanceActionStepParameters::Search {
                condition: MaintenanceSearchCondition::Unconditional,
            },
        }]),
    )
    .await;
    let key = MaintenanceActionStepKey {
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
        let report = fixture
            .app
            .run_lifecycle_action_handling_job()
            .await
            .expect("record bounded failed step attempt");
        assert_eq!(report.failed, 1, "retry {retry}: {report:?}");
    }
    assert_eq!(
        fixture.evaluation.all_candidates().await[0].state,
        MaintenanceCandidateState::Failed
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
async fn unmonitor_baseline_keeps_own_change_visible_but_preserves_external_remonitor() {
    let fixture = execution_app(None);
    let title = seed_title(&fixture.app, &fixture.user, "V2 monitoring baseline", true).await;
    let mut draft = sequence_draft(vec![
        MaintenanceActionStep {
            id: "unmonitor".to_string(),
            kind: MaintenanceActionStepKind::Unmonitor,
            parameters: MaintenanceActionStepParameters::Unmonitor {
                include_descendants: false,
            },
        },
        MaintenanceActionStep {
            id: "search".to_string(),
            kind: MaintenanceActionStepKind::Search,
            parameters: MaintenanceActionStepParameters::Search {
                condition: MaintenanceSearchCondition::Unconditional,
            },
        },
    ]);
    draft.rego_source = "match if { input.facts.monitored }\n".to_string();
    let (_rule_set_id, candidate) = arm_and_evaluate(&fixture, draft).await;
    let key = MaintenanceActionStepKey {
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
        .run_lifecycle_action_handling_job()
        .await
        .expect("own unmonitor may reach waiting search");
    assert_eq!(waiting.held, 1, "{waiting:?}");
    fixture
        .app
        .set_title_monitored(&fixture.user, &title.id, true)
        .await
        .expect("external remonitor");
    set_test_completed_sequence_job_state(&key, TestCompletedSequenceJobState::Completed);
    let completed = fixture
        .app
        .run_lifecycle_action_handling_job()
        .await
        .expect("external remonitor remains live matcher input");
    assert_eq!(completed.executed, 1, "{completed:?}");
    assert!(
        fixture
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
        MaintenanceActionStep {
            id: "add".to_string(),
            kind: MaintenanceActionStepKind::AddTags,
            parameters: MaintenanceActionStepParameters::Tags {
                tags: vec!["owned-add".to_string()],
            },
        },
        MaintenanceActionStep {
            id: "remove".to_string(),
            kind: MaintenanceActionStepKind::RemoveTags,
            parameters: MaintenanceActionStepParameters::Tags {
                tags: vec!["owned-remove".to_string()],
            },
        },
        MaintenanceActionStep {
            id: "search".to_string(),
            kind: MaintenanceActionStepKind::Search,
            parameters: MaintenanceActionStepParameters::Search {
                condition: MaintenanceSearchCondition::Unconditional,
            },
        },
    ]);
    draft.rego_source = "match if { input.facts.tags[_] == \"owned-remove\" }\n".to_string();
    let (_rule_set_id, candidate) = arm_and_evaluate(&fixture, draft).await;
    let key = MaintenanceActionStepKey {
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
            .run_lifecycle_action_handling_job()
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
    assert_eq!(
        fixture
            .app
            .run_lifecycle_action_handling_job()
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

#[tokio::test]
async fn sequence_step_budget_stops_between_steps_then_resumes_without_replay() {
    let fixture = execution_app(None);
    for index in 1..=4 {
        seed_title(
            &fixture.app,
            &fixture.user,
            &format!("V2 pass step budget {index}"),
            false,
        )
        .await;
    }
    define_title_tags(&fixture, &["step-added", "step-removed"]).await;
    let (_rule_set_id, candidates) = arm_and_evaluate_all(
        &fixture,
        sequence_draft(vec![
            MaintenanceActionStep {
                id: "add".to_string(),
                kind: MaintenanceActionStepKind::AddTags,
                parameters: MaintenanceActionStepParameters::Tags {
                    tags: vec!["step-added".to_string()],
                },
            },
            MaintenanceActionStep {
                id: "remove".to_string(),
                kind: MaintenanceActionStepKind::RemoveTags,
                parameters: MaintenanceActionStepParameters::Tags {
                    tags: vec!["step-removed".to_string()],
                },
            },
            MaintenanceActionStep {
                id: "unmonitor".to_string(),
                kind: MaintenanceActionStepKind::Unmonitor,
                parameters: MaintenanceActionStepParameters::Unmonitor {
                    include_descendants: false,
                },
            },
        ]),
    )
    .await;
    assert_eq!(candidates.len(), 4, "{candidates:?}");

    let first_pass = fixture
        .app
        .run_lifecycle_action_handling_job()
        .await
        .expect("run sequence steps through the pass allowance");
    assert_eq!(first_pass.executed, 3, "{first_pass:?}");
    assert_eq!(first_pass.held, 1, "{first_pass:?}");
    let paused = fixture
        .evaluation
        .all_candidates()
        .await
        .into_iter()
        .find(|candidate| candidate.state == MaintenanceCandidateState::Blocked)
        .expect("candidate paused after its tenth dispatched step");
    let paused_steps = fixture
        .evaluation
        .list_action_steps(&paused.id, paused.match_generation, paused.revision_number)
        .await
        .expect("read paused sequence steps");
    assert_eq!(paused_steps.len(), 1, "{paused_steps:?}");
    assert_eq!(paused_steps[0].key.step_id, "add");
    assert_eq!(paused_steps[0].attempt, 1);
    assert_eq!(paused_steps[0].state, MaintenanceActionStepState::Succeeded);

    let resumed = fixture
        .app
        .run_lifecycle_action_handling_job()
        .await
        .expect("resume the remaining sequence steps next pass");
    assert_eq!(resumed.executed, 1, "{resumed:?}");
    let resumed_steps = fixture
        .evaluation
        .list_action_steps(&paused.id, paused.match_generation, paused.revision_number)
        .await
        .expect("read completed sequence steps");
    assert_eq!(resumed_steps.len(), 3, "{resumed_steps:?}");
    let add = resumed_steps
        .iter()
        .find(|step| step.key.step_id == "add")
        .expect("persisted first step");
    assert_eq!(add.state, MaintenanceActionStepState::Succeeded);
    assert_eq!(add.attempt, 1, "resuming must not replay the first step");
    assert!(
        resumed_steps
            .iter()
            .all(|step| step.state.is_completion_success())
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
        let path = fixture.root.join(format!("missing-{index}.mkv"));
        add_catalog_movie_file_without_physical_path(&fixture.execution.app, &title.id, &path)
            .await;
        std::fs::write(&path, b"admit the configured-root title")
            .expect("create file for storage-scoped evaluation");
    }
    let draft = selected_root_sequence_deletion_draft(&fixture).await;
    let (rule_set_id, candidates) = arm_and_evaluate_all(&fixture.execution, draft).await;
    assert_eq!(candidates.len(), 4, "{candidates:?}");
    fixture
        .execution
        .app
        .set_maintenance_rule_arming(
            &fixture.execution.user,
            &rule_set_id,
            MaintenanceEffectArming::Destructive,
            Some(4),
        )
        .await
        .expect("destructively arm the four candidates");
    fixture
        .execution
        .media_files
        .fail_delete_media_file("post-preview catalog cleanup failure")
        .await;

    let report = fixture
        .execution
        .app
        .run_lifecycle_action_handling_job()
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
            .filter(|candidate| candidate.state == MaintenanceCandidateState::Blocked)
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

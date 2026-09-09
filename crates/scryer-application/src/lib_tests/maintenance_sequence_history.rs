//! Operator-visible action history for real schema-2 sequence executions.
//!
//! This deliberately drives the scheduler and action handler instead of
//! seeding lifecycle-action rows: every summary must originate from the same
//! durable path that production uses.

use crate::lib_tests::maintenance_execution::{execution_app, seed_title};
use crate::lib_tests::maintenance_sequence_execution::{arm_and_evaluate, sequence_draft};
use crate::maintenance_rules::sequence_execution::{
    TestCompletedSequenceJobMap, TestCompletedSequenceJobState,
    install_test_completed_sequence_job_adapter, set_test_completed_sequence_job_state,
};
use crate::maintenance_rules::{
    MaintenanceActionSequence, MaintenanceActionStep, MaintenanceActionStepKind,
    MaintenanceActionStepParameters, MaintenanceSearchCondition,
};
use crate::ports::MaintenanceCandidateRepository;
use chrono::{Duration, Utc};
use scryer_domain::{
    MaintenanceActionStepKey, MaintenanceActionStepState, MaintenanceCandidateState,
};

#[tokio::test]
async fn real_sequence_history_tracks_waiting_search_then_the_following_step() {
    let fixture = execution_app(None);
    let title = seed_title(&fixture.app, &fixture.user, "Sequence History", false).await;
    fixture
        .app
        .create_title_tag_definition(&fixture.user, "after-search", None)
        .await
        .expect("define follow-up tag");
    let (rule_set_id, candidate) = arm_and_evaluate(
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
                    tags: vec!["after-search".to_string()],
                },
            },
        ]),
    )
    .await;
    let search_key = MaintenanceActionStepKey {
        candidate_id: candidate.id.clone(),
        match_generation: candidate.match_generation,
        revision_number: candidate.revision_number,
        step_id: "search".to_string(),
    };
    let _adapter = install_test_completed_sequence_job_adapter(TestCompletedSequenceJobMap::from(
        [(search_key.clone(), TestCompletedSequenceJobState::Waiting)],
    ))
    .await;

    let waiting = fixture
        .app
        .run_lifecycle_action_handling_job()
        .await
        .expect("execute accepted search");
    assert_eq!(waiting.held, 1, "the external job remains pending");

    let history = fixture
        .app
        .list_maintenance_action_runs(&fixture.user, Some(&rule_set_id), Some(&candidate.id), None)
        .await
        .expect("list sequence history after held search");
    assert_eq!(
        history.len(),
        1,
        "one sequence execution, no step journal row"
    );
    let held = &history[0];
    assert_eq!(held.run.status.as_storage_str(), "held");
    assert_eq!(held.sequence_steps.len(), 2);
    assert_eq!(
        held.sequence_steps[0].run.as_ref().map(|run| run.state),
        Some(MaintenanceActionStepState::Held),
    );
    assert!(
        held.sequence_steps[0]
            .receipts
            .iter()
            .any(|receipt| receipt.state.as_storage_str() == "accepted")
    );
    assert!(
        held.sequence_steps[1].run.is_none(),
        "a later step remains visibly pending while search waits"
    );

    set_test_completed_sequence_job_state(&search_key, TestCompletedSequenceJobState::Completed);
    let completed = fixture
        .app
        .run_lifecycle_action_handling_job()
        .await
        .expect("resume completed search and apply tag");
    assert_eq!(completed.executed, 1);

    let history = fixture
        .app
        .list_maintenance_action_runs(&fixture.user, Some(&rule_set_id), Some(&candidate.id), None)
        .await
        .expect("list completed sequence history");
    assert_eq!(
        history.len(),
        2,
        "one held and one completed sequence attempt"
    );
    let completed = history
        .iter()
        .find(|run| run.run.status.as_storage_str() == "succeeded")
        .expect("completed sequence summary");
    let held = history
        .iter()
        .find(|run| run.run.status.as_storage_str() == "held")
        .expect("held sequence summary remains in history");
    assert_eq!(
        held.sequence_steps[0].run.as_ref().map(|run| run.state),
        Some(MaintenanceActionStepState::Held),
        "the held attempt retains its own immutable step snapshot",
    );
    assert!(
        held.sequence_steps[1].run.is_none(),
        "a later retry must not retroactively fill a held attempt's pending step",
    );
    assert_eq!(completed.sequence_steps.len(), 2);
    assert_eq!(
        completed.sequence_steps[1]
            .run
            .as_ref()
            .map(|run| run.state),
        Some(MaintenanceActionStepState::Succeeded),
        "the later step is rendered from its durable sequence record",
    );
    assert!(
        fixture
            .app
            .get_title(&fixture.user, &title.id)
            .await
            .expect("read title")
            .expect("title survives sequence")
            .tags
            .iter()
            .any(|tag| tag == "after-search"),
        "the real follow-up step ran after the accepted job completed"
    );
}

#[tokio::test]
async fn fresh_lease_closes_an_abandoned_sequence_history_before_recording_its_retry() {
    let fixture = execution_app(None);
    let title = seed_title(
        &fixture.app,
        &fixture.user,
        "Sequence history restart",
        false,
    )
    .await;
    fixture
        .app
        .create_title_tag_definition(&fixture.user, "recovered", None)
        .await
        .expect("define recovery tag");
    let sequence = MaintenanceActionSequence::new(vec![MaintenanceActionStep {
        id: "tag".to_string(),
        kind: MaintenanceActionStepKind::AddTags,
        parameters: MaintenanceActionStepParameters::Tags {
            tags: vec!["recovered".to_string()],
        },
    }]);
    let (rule_set_id, candidate) =
        arm_and_evaluate(&fixture, sequence_draft(sequence.steps.clone())).await;
    let abandoned_at = Utc::now() - Duration::minutes(61);
    assert!(
        fixture
            .evaluation
            .transition_candidate_state(
                &candidate.id,
                MaintenanceCandidateState::Due,
                "seed_abandoned_sequence_history",
                &[candidate.state.clone()],
                abandoned_at,
            )
            .await
            .expect("make candidate leasable for an abandoned lease")
    );
    assert!(
        fixture
            .evaluation
            .lease_candidate_for_execution(&candidate.id, Utc::now(), abandoned_at)
            .await
            .expect("persist abandoned candidate lease")
    );
    let abandoned_candidate = fixture
        .evaluation
        .all_candidates()
        .await
        .into_iter()
        .next()
        .expect("candidate after abandoned lease");
    let abandoned = fixture
        .app
        .begin_maintenance_sequence_history(&abandoned_candidate, &sequence, abandoned_at)
        .await
        .expect("persist abandoned sequence history");
    assert_eq!(abandoned.status.as_storage_str(), "running");

    let report = fixture
        .app
        .run_lifecycle_action_handling_job()
        .await
        .expect("reclaim stale lease and execute sequence");
    assert_eq!(report.executed, 1, "the retry executes exactly once");

    let history = fixture
        .app
        .list_maintenance_action_runs(&fixture.user, Some(&rule_set_id), Some(&candidate.id), None)
        .await
        .expect("list restart history");
    assert_eq!(
        history.len(),
        2,
        "abandoned and retried executions are distinct"
    );
    let interrupted = history
        .iter()
        .find(|entry| entry.run.id == abandoned.id)
        .expect("abandoned summary stays visible");
    assert_eq!(interrupted.run.status.as_storage_str(), "held");
    assert_eq!(interrupted.run.hold_reason.as_deref(), Some("interrupted"));
    assert!(
        history
            .iter()
            .any(|entry| entry.run.status.as_storage_str() == "succeeded"),
        "the new lease writes its own terminal summary"
    );
    let after = fixture
        .evaluation
        .all_candidates()
        .await
        .into_iter()
        .next()
        .expect("candidate after recovery");
    assert_eq!(
        after.action_attempts, 0,
        "closing the abandoned summary does not consume an action attempt"
    );
    assert!(
        fixture
            .app
            .get_title(&fixture.user, &title.id)
            .await
            .expect("read recovered title")
            .expect("title survives tag sequence")
            .tags
            .iter()
            .any(|tag| tag == "recovered")
    );
}

//! Store round-trips for the maintenance evaluator's three tables, including
//! the two invariants that live in the schema rather than in Rust: one active
//! candidate per (rule, title), and one global exclusion per title.

use super::*;
use crate::workflow::stores::WorkflowOperationStore;
use scryer_application::{
    JobKey, JobRunRecord, JobRunRepository, JobRunStatus, JobTriggerSource,
    LifecycleActionRunRepository, MaintenanceActionStepRepository, MaintenanceCandidateQuery,
    MaintenanceCandidateRepository, MaintenanceEvaluationRunRepository,
    MaintenanceExclusionRepository, MaintenanceRuleSetRepository, MaintenanceSearchJobRunCreation,
    MaintenanceSequenceCompletionRepository,
};
use scryer_domain::{
    LifecycleCandidate, MaintenanceActionJobReceipt, MaintenanceActionJobReceiptState,
    MaintenanceActionStepKey, MaintenanceActionStepRun, MaintenanceActionStepState,
    MaintenanceCandidateState, MaintenanceEvaluationMode, MaintenanceEvaluationRun,
    MaintenanceEvaluationRunStatus, MaintenanceRuleExclusion, MaintenanceRuleRevision,
    MaintenanceRuleSet, MaintenanceRuleSubjectKind, MaintenanceSequenceTerminalMembership,
    MaintenanceSequenceTerminalOutcome,
};

fn evaluation_store(services: &SqliteServices) -> crate::MaintenanceEvaluationStore {
    crate::MaintenanceEvaluationStore::new(services.datastore())
}

/// Candidates, runs, and exclusions all cascade from a rule set, so every test
/// here needs one to exist first.
async fn seed_rule_set(services: &SqliteServices, id: &str) {
    let now = Utc::now();
    crate::MaintenanceRuleSetStore::new(services.datastore())
        .create_rule_set(
            &MaintenanceRuleSet {
                id: id.to_string(),
                name: format!("rule {id}"),
                description: String::new(),
                enabled: false,
                evaluation_mode: MaintenanceEvaluationMode::Disabled,
                effect_arming: scryer_domain::MaintenanceEffectArming::None,
                destructive_rearm_required: false,
                library_ids: Vec::new(),
                subject_kind: MaintenanceRuleSubjectKind::Title,
                current_revision_number: 1,
                created_at: now,
                updated_at: now,
            },
            &MaintenanceRuleRevision {
                id: format!("{id}-rev-1"),
                rule_set_id: id.to_string(),
                revision_number: 1,
                rego_source: format!("package scryer.maintenance.user.{id}\nmatch := true\n"),
                action_spec_json: r#"{"kind":"unmonitor_scope_keep_files","schema_version":1}"#
                    .to_string(),
                grace_days: 7,
                storage_root_id: None,
                matcher_content_hash: "hash-1".to_string(),
                created_by: None,
                created_at: now,
            },
        )
        .await
        .expect("seed rule set");
}

fn candidate(id: &str, rule_set_id: &str, title_id: &str, generation: i64) -> LifecycleCandidate {
    let now = Utc::now();
    LifecycleCandidate {
        id: id.to_string(),
        rule_set_id: rule_set_id.to_string(),
        revision_number: 1,
        matcher_content_hash: "hash-1".to_string(),
        title_id: title_id.to_string(),
        subject_id: title_id.to_string(),
        library_id: "library-1".to_string(),
        facet: "movie".to_string(),
        subject_kind: "title".to_string(),
        match_generation: generation,
        state: MaintenanceCandidateState::Observing,
        state_reason: "first_match".to_string(),
        reason_codes: vec!["stale".to_string(), "unwatched".to_string()],
        action_kind: "unmonitor_scope_keep_files".to_string(),
        grace_days: 7,
        first_matched_at: now,
        last_matched_at: now,
        due_at: now + chrono::Duration::days(7),
        last_evaluated_at: now,
        held_since: None,
        action_attempts: 0,
        created_at: now,
        updated_at: now,
    }
}

fn sequence_step_run(candidate: &LifecycleCandidate, attempt: i64) -> MaintenanceActionStepRun {
    let now = Utc::now();
    MaintenanceActionStepRun {
        key: MaintenanceActionStepKey {
            candidate_id: candidate.id.clone(),
            match_generation: candidate.match_generation,
            revision_number: candidate.revision_number,
            step_id: "search".to_string(),
        },
        rule_set_id: candidate.rule_set_id.clone(),
        title_id: candidate.title_id.clone(),
        subject_kind: candidate.subject_kind.clone(),
        subject_id: candidate.subject_id.clone(),
        sequence_content_hash: "sequence-hash".to_string(),
        step_kind: "search".to_string(),
        intent_json: r#"{"schema_version":2,"effective_request":"missing:title-1"}"#.to_string(),
        before_state_json: r#"{"schema_version":1,"monitored":true}"#.to_string(),
        target_identity_json: serde_json::json!({
            "schema_version": 1,
            "rule_set_id": candidate.rule_set_id,
            "candidate_id": candidate.id,
            "match_generation": candidate.match_generation,
            "revision_number": candidate.revision_number,
            "title_id": candidate.title_id,
            "subject_kind": candidate.subject_kind,
            "subject_id": candidate.subject_id,
            "step_id": "search",
            "step_kind": "search",
            "sequence_content_hash": "sequence-hash",
        })
        .to_string(),
        provenance_json: r#"{"schema_version":1,"request":"saved"}"#.to_string(),
        state: MaintenanceActionStepState::Running,
        attempt,
        lease_id: None,
        lease_expires_at: Some(now + chrono::Duration::minutes(5)),
        hold_reason: None,
        error: None,
        created_at: now,
        updated_at: now,
        finished_at: None,
    }
}

#[tokio::test]
async fn candidates_round_trip_with_their_reason_codes_and_timestamps() {
    let (services, db) = temp_services("scryer_maintenance_candidates").await;
    seed_rule_set(&services, "rule-a").await;
    let store = evaluation_store(&services);

    let created = candidate("cand-1", "rule-a", "title-1", 1);
    store
        .create_candidate(&created)
        .await
        .expect("create candidate");

    let loaded = store
        .get_active_candidate("rule-a", "title-1")
        .await
        .expect("read candidate")
        .expect("candidate exists");
    assert_eq!(loaded.id, created.id);
    assert_eq!(loaded.state, MaintenanceCandidateState::Observing);
    assert_eq!(loaded.reason_codes, created.reason_codes);
    assert_eq!(loaded.action_kind, created.action_kind);
    assert_eq!(loaded.match_generation, 1);
    assert_eq!(loaded.grace_days, 7);
    assert_eq!(loaded.held_since, None);
    assert_eq!(
        loaded.first_matched_at.timestamp(),
        created.first_matched_at.timestamp()
    );
    assert_eq!(loaded.due_at.timestamp(), created.due_at.timestamp());

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn a_rule_set_may_hold_only_one_active_candidate_per_title() {
    let (services, db) = temp_services("scryer_maintenance_candidate_invariant").await;
    seed_rule_set(&services, "rule-a").await;
    let store = evaluation_store(&services);

    store
        .create_candidate(&candidate("cand-1", "rule-a", "title-1", 1))
        .await
        .expect("create the first candidate");

    let rejected = store
        .create_candidate(&candidate("cand-2", "rule-a", "title-1", 2))
        .await
        .expect_err("a second active candidate for the same subject must be refused");
    assert!(
        rejected.to_string().contains("active candidate"),
        "{rejected}"
    );

    // Closing the first one frees the slot: a cancel-then-rematch is exactly
    // how a new generation is supposed to begin.
    store
        .transition_candidate_state(
            "cand-1",
            MaintenanceCandidateState::Canceled,
            "no_match",
            &[MaintenanceCandidateState::Observing],
            Utc::now(),
        )
        .await
        .expect("cancel");
    store
        .create_candidate(&candidate("cand-2", "rule-a", "title-1", 2))
        .await
        .expect("a fresh candidate may open once the previous one is terminal");

    assert_eq!(
        store
            .max_match_generation("rule-a", "title-1")
            .await
            .expect("max generation"),
        2,
        "generations count terminal rows too"
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn sequence_step_journal_and_terminal_marker_block_readmission_until_confirmed_no_match() {
    let (services, db) = temp_services("scryer_maintenance_sequence_latch").await;
    seed_rule_set(&services, "rule-a").await;
    let store = evaluation_store(&services);
    let mut first = candidate("sequence-candidate-1", "rule-a", "title-1", 1);
    first.action_kind = "action_sequence".to_string();
    store
        .create_candidate(&first)
        .await
        .expect("create sequence candidate");

    let leased_at = Utc::now();
    assert!(
        store
            .transition_candidate_state(
                &first.id,
                MaintenanceCandidateState::Executing,
                "execution_leased",
                &[MaintenanceCandidateState::Observing],
                leased_at,
            )
            .await
            .expect("lease candidate")
    );
    let original = sequence_step_run(&first, 0);
    let claimed = match store
        .claim_action_step(
            &original,
            leased_at - chrono::Duration::minutes(1),
            "step-lease-1",
            leased_at,
        )
        .await
        .expect("claim step")
    {
        scryer_application::MaintenanceActionStepClaim::Claimed(step) => step,
        other => panic!("unexpected step claim: {other:?}"),
    };
    let finished_at = leased_at + chrono::Duration::seconds(1);
    let mut completed = claimed.clone();
    completed.state = MaintenanceActionStepState::Succeeded;
    completed.updated_at = finished_at;
    completed.finished_at = Some(finished_at);
    assert!(
        store
            .finish_action_step(&completed, "step-lease-1")
            .await
            .expect("finish step")
    );
    let membership = MaintenanceSequenceTerminalMembership {
        id: "sequence-membership-1".to_string(),
        candidate_id: first.id.clone(),
        rule_set_id: first.rule_set_id.clone(),
        revision_number: first.revision_number,
        matcher_content_hash: first.matcher_content_hash.clone(),
        title_id: first.title_id.clone(),
        subject_kind: first.subject_kind.clone(),
        subject_id: first.subject_id.clone(),
        match_generation: first.match_generation,
        sequence_content_hash: "sequence-hash".to_string(),
        outcome: MaintenanceSequenceTerminalOutcome::Succeeded,
        expected_step_ids: vec!["search".to_string()],
        completed_step_ids: vec!["search".to_string()],
        terminal_step_id: None,
        created_at: finished_at,
        released_at: None,
    };
    assert!(
        store
            .finish_sequence_terminal_membership_and_candidate(
                &membership,
                MaintenanceCandidateState::Executing,
                leased_at,
                MaintenanceCandidateState::Succeeded,
                "action_succeeded",
                finished_at,
            )
            .await
            .expect("atomically finish sequence")
    );

    let mut second = candidate("sequence-candidate-2", "rule-a", "title-1", 2);
    second.action_kind = "action_sequence".to_string();
    assert!(store.create_candidate(&second).await.is_err());
    assert!(
        store
            .release_sequence_completion_on_confirmed_non_match(
                "rule-a",
                1,
                "title",
                "title-1",
                finished_at + chrono::Duration::seconds(1),
            )
            .await
            .expect("confirmed no-match releases exact latch")
    );
    store
        .create_candidate(&second)
        .await
        .expect("new generation after confirmed no-match");

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn failed_terminal_sequence_membership_preserves_the_exhausted_generation() {
    let (services, db) = temp_services("scryer_maintenance_sequence_failed_latch").await;
    seed_rule_set(&services, "rule-a").await;
    let store = evaluation_store(&services);
    let mut first = candidate("sequence-failed-1", "rule-a", "title-failed", 1);
    first.action_kind = "action_sequence".to_string();
    store
        .create_candidate(&first)
        .await
        .expect("create sequence candidate");
    let leased_at = Utc::now();
    store
        .transition_candidate_state(
            &first.id,
            MaintenanceCandidateState::Executing,
            "execution_leased",
            &[MaintenanceCandidateState::Observing],
            leased_at,
        )
        .await
        .expect("lease candidate");
    let claimed = match store
        .claim_action_step(
            &sequence_step_run(&first, 2),
            leased_at - chrono::Duration::minutes(1),
            "failed-step-lease",
            leased_at,
        )
        .await
        .expect("claim exhausted step")
    {
        scryer_application::MaintenanceActionStepClaim::Claimed(step) => step,
        other => panic!("unexpected claim: {other:?}"),
    };
    let finished_at = leased_at + chrono::Duration::seconds(1);
    let mut failed = claimed;
    failed.state = MaintenanceActionStepState::Failed;
    failed.error = Some("retry budget exhausted".to_string());
    failed.updated_at = finished_at;
    failed.finished_at = Some(finished_at);
    assert!(
        store
            .finish_action_step(&failed, "failed-step-lease")
            .await
            .expect("persist terminal failure")
    );
    let membership = MaintenanceSequenceTerminalMembership {
        id: "sequence-failed-membership".to_string(),
        candidate_id: first.id.clone(),
        rule_set_id: first.rule_set_id.clone(),
        revision_number: first.revision_number,
        matcher_content_hash: first.matcher_content_hash.clone(),
        title_id: first.title_id.clone(),
        subject_kind: first.subject_kind.clone(),
        subject_id: first.subject_id.clone(),
        match_generation: first.match_generation,
        sequence_content_hash: "sequence-hash".to_string(),
        outcome: MaintenanceSequenceTerminalOutcome::Failed,
        expected_step_ids: vec!["search".to_string()],
        completed_step_ids: vec!["search".to_string()],
        terminal_step_id: Some("search".to_string()),
        created_at: finished_at,
        released_at: None,
    };
    assert!(
        store
            .finish_sequence_terminal_membership_and_candidate(
                &membership,
                MaintenanceCandidateState::Executing,
                leased_at,
                MaintenanceCandidateState::Failed,
                "retry_budget_exhausted",
                finished_at,
            )
            .await
            .expect("finish failed sequence")
    );
    let mut next = candidate("sequence-failed-2", "rule-a", "title-failed", 2);
    next.action_kind = "action_sequence".to_string();
    assert!(store.create_candidate(&next).await.is_err());

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn stale_candidate_lease_cannot_commit_a_terminal_sequence_membership() {
    let (services, db) = temp_services("scryer_maintenance_sequence_stale_terminal").await;
    seed_rule_set(&services, "rule-a").await;
    let store = evaluation_store(&services);
    let mut candidate = candidate("sequence-stale-1", "rule-a", "title-stale", 1);
    candidate.action_kind = "action_sequence".to_string();
    store
        .create_candidate(&candidate)
        .await
        .expect("create candidate");
    let leased_at = Utc::now();
    store
        .transition_candidate_state(
            &candidate.id,
            MaintenanceCandidateState::Executing,
            "execution_leased",
            &[MaintenanceCandidateState::Observing],
            leased_at,
        )
        .await
        .expect("lease candidate");
    let claimed = match store
        .claim_action_step(
            &sequence_step_run(&candidate, 0),
            leased_at - chrono::Duration::minutes(1),
            "stale-step-lease",
            leased_at,
        )
        .await
        .expect("claim step")
    {
        scryer_application::MaintenanceActionStepClaim::Claimed(step) => step,
        other => panic!("unexpected claim: {other:?}"),
    };
    let finished_at = leased_at + chrono::Duration::seconds(1);
    let mut completed = claimed;
    completed.state = MaintenanceActionStepState::Succeeded;
    completed.updated_at = finished_at;
    completed.finished_at = Some(finished_at);
    store
        .finish_action_step(&completed, "stale-step-lease")
        .await
        .expect("finish step");
    let reclaimed_at = finished_at + chrono::Duration::seconds(1);
    store
        .record_candidate_match(&candidate.id, reclaimed_at, &[], reclaimed_at)
        .await
        .expect("simulate a newer candidate lease write");
    let membership = MaintenanceSequenceTerminalMembership {
        id: "stale-membership".to_string(),
        candidate_id: candidate.id.clone(),
        rule_set_id: candidate.rule_set_id.clone(),
        revision_number: candidate.revision_number,
        matcher_content_hash: candidate.matcher_content_hash.clone(),
        title_id: candidate.title_id.clone(),
        subject_kind: candidate.subject_kind.clone(),
        subject_id: candidate.subject_id.clone(),
        match_generation: candidate.match_generation,
        sequence_content_hash: "sequence-hash".to_string(),
        outcome: MaintenanceSequenceTerminalOutcome::Succeeded,
        expected_step_ids: vec!["search".to_string()],
        completed_step_ids: vec!["search".to_string()],
        terminal_step_id: None,
        created_at: finished_at,
        released_at: None,
    };
    assert!(
        store
            .finish_sequence_terminal_membership_and_candidate(
                &membership,
                MaintenanceCandidateState::Executing,
                leased_at,
                MaintenanceCandidateState::Succeeded,
                "action_succeeded",
                finished_at,
            )
            .await
            .is_err()
    );
    assert!(
        store
            .get_active_sequence_completion("rule-a", 1, "title", "title-stale")
            .await
            .expect("membership reload")
            .is_none()
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn held_and_failed_step_retries_keep_the_first_intent_and_before_state() {
    let (services, db) = temp_services("scryer_maintenance_sequence_retry_journal").await;
    seed_rule_set(&services, "rule-a").await;
    let store = evaluation_store(&services);
    let candidate = candidate("sequence-retry-1", "rule-a", "title-2", 1);
    store
        .create_candidate(&candidate)
        .await
        .expect("create candidate");
    let original = sequence_step_run(&candidate, 0);
    let leased_at = Utc::now();
    let claimed = match store
        .claim_action_step(
            &original,
            leased_at - chrono::Duration::minutes(1),
            "step-lease-1",
            leased_at,
        )
        .await
        .expect("claim first attempt")
    {
        scryer_application::MaintenanceActionStepClaim::Claimed(step) => step,
        other => panic!("unexpected step claim: {other:?}"),
    };
    let mut held = claimed.clone();
    held.state = MaintenanceActionStepState::Held;
    held.hold_reason = Some("search provider unavailable".to_string());
    held.updated_at = leased_at + chrono::Duration::seconds(1);
    held.finished_at = Some(held.updated_at);
    assert!(
        store
            .finish_action_step(&held, "step-lease-1")
            .await
            .expect("hold first attempt")
    );

    let mut reconstructed = sequence_step_run(&candidate, 0);
    reconstructed.intent_json = r#"{"different":"must-not-replace-saved-request"}"#.to_string();
    reconstructed.before_state_json = r#"{"different":"must-not-replace-before"}"#.to_string();
    reconstructed.provenance_json = r#"{"different":"must-not-replace-provenance"}"#.to_string();
    let resumed = match store
        .claim_action_step(
            &reconstructed,
            held.updated_at,
            "step-lease-2",
            held.updated_at + chrono::Duration::seconds(1),
        )
        .await
        .expect("resume held attempt")
    {
        scryer_application::MaintenanceActionStepClaim::Claimed(step) => step,
        other => panic!("unexpected held resume: {other:?}"),
    };
    assert_eq!(resumed.attempt, 0, "a hold does not spend failure budget");
    assert_eq!(resumed.intent_json, original.intent_json);
    assert_eq!(resumed.before_state_json, original.before_state_json);
    assert_eq!(resumed.provenance_json, original.provenance_json);

    let mut failed = resumed.clone();
    failed.state = MaintenanceActionStepState::Failed;
    failed.error = Some("transient failure".to_string());
    failed.updated_at += chrono::Duration::seconds(1);
    failed.finished_at = Some(failed.updated_at);
    assert!(
        store
            .finish_action_step(&failed, "step-lease-2")
            .await
            .expect("finish failed attempt")
    );
    reconstructed.attempt = 1;
    let retried = match store
        .claim_action_step(
            &reconstructed,
            failed.updated_at,
            "step-lease-3",
            failed.updated_at + chrono::Duration::seconds(1),
        )
        .await
        .expect("retry failed attempt")
    {
        scryer_application::MaintenanceActionStepClaim::Claimed(step) => step,
        other => panic!("unexpected failed retry: {other:?}"),
    };
    assert_eq!(retried.attempt, 1);
    assert_eq!(retried.intent_json, original.intent_json);
    assert_eq!(retried.before_state_json, original.before_state_json);
    assert_eq!(retried.provenance_json, original.provenance_json);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn independent_stores_race_a_step_claim_without_creating_two_attempts() {
    let (services, db) = temp_services("scryer_maintenance_sequence_claim_race").await;
    seed_rule_set(&services, "rule-a").await;
    let setup = evaluation_store(&services);
    let candidate = candidate("sequence-race-1", "rule-a", "title-5", 1);
    setup
        .create_candidate(&candidate)
        .await
        .expect("create candidate");
    let step = sequence_step_run(&candidate, 0);
    let mut corrupt = step.clone();
    corrupt.target_identity_json = r#"{"schema_version":99}"#.to_string();
    assert!(
        setup
            .claim_action_step(
                &corrupt,
                Utc::now() - chrono::Duration::minutes(1),
                "corrupt-lease",
                Utc::now(),
            )
            .await
            .is_err()
    );
    let first = crate::MaintenanceEvaluationStore::new(services.datastore());
    let second = crate::MaintenanceEvaluationStore::new(services.datastore());
    let now = Utc::now();
    let (left, right) = tokio::join!(
        first.claim_action_step(
            &step,
            now - chrono::Duration::minutes(1),
            "race-lease-a",
            now,
        ),
        second.claim_action_step(
            &step,
            now - chrono::Duration::minutes(1),
            "race-lease-b",
            now,
        )
    );
    let claims = [left.expect("left claim"), right.expect("right claim")];
    assert_eq!(
        claims
            .iter()
            .filter(|claim| matches!(
                claim,
                scryer_application::MaintenanceActionStepClaim::Claimed(_)
            ))
            .count(),
        1
    );
    assert_eq!(
        claims
            .iter()
            .filter(|claim| matches!(claim, scryer_application::MaintenanceActionStepClaim::Busy))
            .count(),
        1
    );
    assert_eq!(
        setup
            .list_action_step_attempts(&step.key)
            .await
            .expect("read immutable attempts")
            .len(),
        1
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn accepted_search_receipt_is_atomic_and_reused_across_a_later_dispatch_attempt() {
    let (services, db) = temp_services("scryer_maintenance_search_receipt_atomic").await;
    seed_rule_set(&services, "rule-a").await;
    let evaluation = evaluation_store(&services);
    let sequence_candidate = candidate("sequence-search-1", "rule-a", "title-3", 1);
    evaluation
        .create_candidate(&sequence_candidate)
        .await
        .expect("create candidate");
    let step = sequence_step_run(&sequence_candidate, 0);
    let claimed = match evaluation
        .claim_action_step(
            &step,
            Utc::now() - chrono::Duration::minutes(1),
            "search-step-lease",
            Utc::now(),
        )
        .await
        .expect("persist Search step")
    {
        scryer_application::MaintenanceActionStepClaim::Claimed(step) => step,
        other => panic!("unexpected claim: {other:?}"),
    };
    let jobs = WorkflowOperationStore::new(services.datastore());
    let now = Utc::now();
    let initial_receipt = MaintenanceActionJobReceipt {
        schema_version: 1,
        key: claimed.key.clone(),
        dispatch_attempt: 1,
        logical_request_key: claimed.key.logical_request_key(),
        request_hash: "stable-effective-request-hash".to_string(),
        job_run_id: Some("search-job-1".to_string()),
        state: MaintenanceActionJobReceiptState::Accepted,
        reconciliation_evidence_json: r#"{"accepted":true}"#.to_string(),
        created_at: now,
        updated_at: now,
    };
    let run = JobRunRecord {
        id: "search-job-1".to_string(),
        job_key: JobKey::AcquisitionSearch,
        operation_type: "maintenance_acquisition_search:missing:1".to_string(),
        status: JobRunStatus::Running,
        trigger_source: JobTriggerSource::SystemInternal,
        actor_user_id: None,
        progress_json: Some(r#"{"state":"running"}"#.to_string()),
        summary_json: None,
        summary_text: None,
        error_text: None,
        started_at: now,
        completed_at: None,
        created_at: now,
        updated_at: now,
    };
    let mut unsupported_receipt = initial_receipt.clone();
    unsupported_receipt.schema_version = 99;
    assert!(
        jobs.create_maintenance_search_job_run(&run, &unsupported_receipt)
            .await
            .is_err()
    );
    assert!(
        evaluation
            .list_action_job_receipts(&claimed.key)
            .await
            .expect("unsupported receipt leaves no durable state")
            .is_empty()
    );

    let competing_jobs = WorkflowOperationStore::new(services.datastore());
    let (left, right) = tokio::join!(
        jobs.create_maintenance_search_job_run(&run, &initial_receipt),
        competing_jobs.create_maintenance_search_job_run(&run, &initial_receipt)
    );
    let creations = [
        left.expect("first dispatch resolves"),
        right.expect("second dispatch resolves"),
    ];
    assert_eq!(
        creations
            .iter()
            .filter(|creation| matches!(creation, MaintenanceSearchJobRunCreation::Created { .. }))
            .count(),
        1,
        "only one concurrent transaction creates the durable job"
    );
    assert_eq!(
        creations
            .iter()
            .filter(|creation| matches!(creation, MaintenanceSearchJobRunCreation::Existing { .. }))
            .count(),
        1,
        "the loser receives the accepted receipt rather than a duplicate job"
    );

    let retry_receipt = MaintenanceActionJobReceipt {
        dispatch_attempt: 2,
        job_run_id: Some("search-job-2".to_string()),
        updated_at: now + chrono::Duration::seconds(1),
        ..initial_receipt.clone()
    };
    let retry_run = JobRunRecord {
        id: "search-job-2".to_string(),
        updated_at: now + chrono::Duration::seconds(1),
        ..run.clone()
    };
    match jobs
        .create_maintenance_search_job_run(&retry_run, &retry_receipt)
        .await
        .expect("return accepted logical receipt")
    {
        MaintenanceSearchJobRunCreation::Existing { receipt } => {
            assert_eq!(receipt.job_run_id.as_deref(), Some("search-job-1"));
        }
        other => panic!("later dispatch attempt created duplicate job: {other:?}"),
    }
    assert!(
        jobs.get_job_run("search-job-2")
            .await
            .expect("read job history")
            .is_none()
    );
    assert_eq!(
        evaluation
            .list_action_job_receipts(&claimed.key)
            .await
            .expect("read receipt")
            .len(),
        1
    );
    sqlx::query("DELETE FROM workflow_operations WHERE id = ?")
        .bind("search-job-1")
        .execute(services.pool())
        .await
        .expect("prune job history without deleting accepted receipt");
    let pruned_retry = MaintenanceActionJobReceipt {
        dispatch_attempt: 3,
        job_run_id: Some("search-job-3".to_string()),
        updated_at: now + chrono::Duration::seconds(2),
        ..initial_receipt.clone()
    };
    let pruned_run = JobRunRecord {
        id: "search-job-3".to_string(),
        updated_at: now + chrono::Duration::seconds(2),
        ..run.clone()
    };
    assert!(matches!(
        jobs.create_maintenance_search_job_run(&pruned_run, &pruned_retry)
            .await
            .expect("accepted receipt survives pruned job history"),
        MaintenanceSearchJobRunCreation::Existing { .. }
    ));
    assert!(
        jobs.get_job_run("search-job-3")
            .await
            .expect("read job history")
            .is_none()
    );

    let rollback_candidate = candidate("sequence-search-rollback", "rule-a", "title-4", 1);
    evaluation
        .create_candidate(&rollback_candidate)
        .await
        .expect("create independent candidate");
    let rollback_claim = match evaluation
        .claim_action_step(
            &sequence_step_run(&rollback_candidate, 0),
            Utc::now() - chrono::Duration::minutes(1),
            "rollback-step-lease",
            Utc::now(),
        )
        .await
        .expect("persist independent step")
    {
        scryer_application::MaintenanceActionStepClaim::Claimed(step) => step,
        other => panic!("unexpected claim: {other:?}"),
    };
    let rollback_receipt = MaintenanceActionJobReceipt {
        key: rollback_claim.key.clone(),
        request_hash: "different-request-hash".to_string(),
        ..initial_receipt.clone()
    };
    assert!(
        jobs.create_maintenance_search_job_run(&run, &rollback_receipt)
            .await
            .is_err()
    );
    assert!(
        evaluation
            .list_action_job_receipts(&rollback_claim.key)
            .await
            .expect("receipt rollback check")
            .is_empty()
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn recording_a_match_advances_the_last_match_and_clears_a_hold() {
    let (services, db) = temp_services("scryer_maintenance_candidate_match").await;
    seed_rule_set(&services, "rule-a").await;
    let store = evaluation_store(&services);
    let created = candidate("cand-1", "rule-a", "title-1", 1);
    store.create_candidate(&created).await.expect("create");

    let held_at = Utc::now() + chrono::Duration::minutes(1);
    store
        .hold_candidate("cand-1", held_at, held_at)
        .await
        .expect("hold");
    let held = store
        .get_active_candidate("rule-a", "title-1")
        .await
        .expect("read")
        .expect("exists");
    assert_eq!(
        held.held_since.map(|value| value.timestamp()),
        Some(held_at.timestamp())
    );
    assert_eq!(
        held.due_at.timestamp(),
        created.due_at.timestamp(),
        "a hold must never move the grace clock"
    );

    // A second hold keeps the first hold's timestamp: how long it has been held
    // is the number that matters.
    let held_again_at = held_at + chrono::Duration::minutes(5);
    store
        .hold_candidate("cand-1", held_again_at, held_again_at)
        .await
        .expect("hold again");
    let still_held = store
        .get_active_candidate("rule-a", "title-1")
        .await
        .expect("read")
        .expect("exists");
    assert_eq!(
        still_held.held_since.map(|value| value.timestamp()),
        Some(held_at.timestamp())
    );

    let matched_at = held_again_at + chrono::Duration::minutes(1);
    store
        .record_candidate_match(
            "cand-1",
            matched_at,
            &["still_stale".to_string()],
            matched_at,
        )
        .await
        .expect("record match");
    let matched = store
        .get_active_candidate("rule-a", "title-1")
        .await
        .expect("read")
        .expect("exists");
    assert_eq!(
        matched.held_since, None,
        "a confirmed match clears the hold"
    );
    assert_eq!(matched.reason_codes, vec!["still_stale".to_string()]);
    assert_eq!(matched.last_matched_at.timestamp(), matched_at.timestamp());
    assert_eq!(
        matched.first_matched_at.timestamp(),
        created.first_matched_at.timestamp(),
        "a repeat match never restarts the clock"
    );
    assert_eq!(matched.due_at.timestamp(), created.due_at.timestamp());

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn listing_filters_by_rule_state_and_library_and_counts_by_state() {
    let (services, db) = temp_services("scryer_maintenance_candidate_listing").await;
    seed_rule_set(&services, "rule-a").await;
    seed_rule_set(&services, "rule-b").await;
    let store = evaluation_store(&services);

    store
        .create_candidate(&candidate("cand-1", "rule-a", "title-1", 1))
        .await
        .expect("create");
    let mut other_library = candidate("cand-2", "rule-a", "title-2", 1);
    other_library.library_id = "library-2".to_string();
    store
        .create_candidate(&other_library)
        .await
        .expect("create");
    store
        .create_candidate(&candidate("cand-3", "rule-b", "title-1", 1))
        .await
        .expect("create");
    store
        .transition_candidate_state(
            "cand-3",
            MaintenanceCandidateState::Canceled,
            "no_match",
            &[MaintenanceCandidateState::Observing],
            Utc::now(),
        )
        .await
        .expect("cancel");

    let by_rule = store
        .list_candidates(&MaintenanceCandidateQuery {
            rule_set_id: Some("rule-a".to_string()),
            ..Default::default()
        })
        .await
        .expect("list");
    assert_eq!(by_rule.len(), 2);

    let by_library = store
        .list_candidates(&MaintenanceCandidateQuery {
            library_id: Some("library-2".to_string()),
            ..Default::default()
        })
        .await
        .expect("list");
    assert_eq!(by_library.len(), 1);
    assert_eq!(by_library[0].id, "cand-2");

    let by_state = store
        .list_candidates(&MaintenanceCandidateQuery {
            states: vec![MaintenanceCandidateState::Canceled],
            ..Default::default()
        })
        .await
        .expect("list");
    assert_eq!(by_state.len(), 1);
    assert_eq!(by_state[0].id, "cand-3");

    let limited = store
        .list_candidates(&MaintenanceCandidateQuery {
            limit: Some(1),
            ..Default::default()
        })
        .await
        .expect("list");
    assert_eq!(limited.len(), 1);

    let counts = store
        .count_candidates_by_state("rule-a")
        .await
        .expect("counts");
    assert_eq!(counts, vec![(MaintenanceCandidateState::Observing, 2)]);

    let canceled = store
        .cancel_active_candidates_for_rule("rule-a", "revision_superseded", Utc::now())
        .await
        .expect("bulk cancel");
    assert_eq!(canceled, 2);
    assert!(
        store
            .get_active_candidate("rule-a", "title-1")
            .await
            .expect("read")
            .is_none()
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn exclusions_allow_one_global_row_and_one_row_per_rule_and_title() {
    let (services, db) = temp_services("scryer_maintenance_exclusions").await;
    seed_rule_set(&services, "rule-a").await;
    seed_rule_set(&services, "rule-b").await;
    let store = evaluation_store(&services);

    let global = MaintenanceRuleExclusion {
        id: "excl-global".to_string(),
        rule_set_id: None,
        title_id: "title-1".to_string(),
        subject_id: "title-1".to_string(),
        subject_kind: scryer_domain::MaintenanceRuleSubjectKind::Title,
        reason: "operator pinned".to_string(),
        created_by: Some("user-1".to_string()),
        created_at: Utc::now(),
    };
    store
        .create_exclusion(&global)
        .await
        .expect("create global exclusion");

    // NULL rule_set_id is distinct from itself inside a plain UNIQUE
    // constraint, so this is exactly the duplicate the partial index exists to
    // stop.
    let duplicate_global = MaintenanceRuleExclusion {
        id: "excl-global-2".to_string(),
        ..global.clone()
    };
    assert!(
        store.create_exclusion(&duplicate_global).await.is_err(),
        "a title may carry at most one global exclusion"
    );

    let per_rule = MaintenanceRuleExclusion {
        id: "excl-rule-a".to_string(),
        rule_set_id: Some("rule-a".to_string()),
        title_id: "title-1".to_string(),
        subject_id: "title-1".to_string(),
        subject_kind: scryer_domain::MaintenanceRuleSubjectKind::Title,
        reason: String::new(),
        created_by: None,
        created_at: Utc::now(),
    };
    store
        .create_exclusion(&per_rule)
        .await
        .expect("a per-rule exclusion coexists with the global one");
    assert!(
        store
            .create_exclusion(&MaintenanceRuleExclusion {
                id: "excl-rule-a-2".to_string(),
                ..per_rule.clone()
            })
            .await
            .is_err(),
        "a rule may carry at most one exclusion per title"
    );

    // Narrowing to a rule returns that rule's rows plus every global row,
    // because both are what actually stop it acting.
    let for_rule_a = store
        .list_exclusions(Some("rule-a"))
        .await
        .expect("list for rule");
    assert_eq!(for_rule_a.len(), 2);
    let for_rule_b = store
        .list_exclusions(Some("rule-b"))
        .await
        .expect("list for rule");
    assert_eq!(for_rule_b.len(), 1);
    assert_eq!(for_rule_b[0].id, "excl-global");
    assert_eq!(for_rule_b[0].rule_set_id, None);
    assert_eq!(for_rule_b[0].reason, "operator pinned");
    assert_eq!(for_rule_b[0].created_by.as_deref(), Some("user-1"));

    assert_eq!(
        store.list_exclusions(None).await.expect("list all").len(),
        2
    );

    store
        .delete_exclusion("excl-global")
        .await
        .expect("delete exclusion");
    assert!(
        store
            .get_exclusion("excl-global")
            .await
            .expect("read")
            .is_none()
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn evaluation_runs_start_as_running_and_finish_with_their_counts() {
    let (services, db) = temp_services("scryer_maintenance_evaluation_runs").await;
    seed_rule_set(&services, "rule-a").await;
    let store = evaluation_store(&services);

    let mut run = MaintenanceEvaluationRun {
        id: "run-1".to_string(),
        rule_set_id: "rule-a".to_string(),
        revision_number: 1,
        matcher_content_hash: "hash-1".to_string(),
        started_at: Utc::now(),
        finished_at: None,
        status: MaintenanceEvaluationRunStatus::Running,
        evaluated_count: 0,
        matched_count: 0,
        no_match_count: 0,
        unknown_count: 0,
        error_count: 0,
        canceled_candidates: 0,
        superseded_candidates: 0,
        duration_ms: None,
        error: None,
    };
    store.start_evaluation_run(&run).await.expect("start run");

    let started = store
        .list_evaluation_runs(Some("rule-a"), None)
        .await
        .expect("list runs");
    assert_eq!(started.len(), 1);
    assert_eq!(started[0].status, MaintenanceEvaluationRunStatus::Running);
    assert_eq!(started[0].finished_at, None);

    run.finished_at = Some(Utc::now());
    run.status = MaintenanceEvaluationRunStatus::Succeeded;
    run.evaluated_count = 12;
    run.matched_count = 3;
    run.no_match_count = 7;
    run.unknown_count = 1;
    run.error_count = 1;
    run.canceled_candidates = 2;
    run.superseded_candidates = 1;
    run.duration_ms = Some(48);
    store.finish_evaluation_run(&run).await.expect("finish run");

    let finished = store
        .list_evaluation_runs(None, Some(5))
        .await
        .expect("list runs");
    assert_eq!(finished.len(), 1);
    let stored = &finished[0];
    assert_eq!(stored.status, MaintenanceEvaluationRunStatus::Succeeded);
    assert_eq!(stored.evaluated_count, 12);
    assert_eq!(stored.matched_count, 3);
    assert_eq!(stored.no_match_count, 7);
    assert_eq!(stored.unknown_count, 1);
    assert_eq!(stored.error_count, 1);
    assert_eq!(stored.canceled_candidates, 2);
    assert_eq!(stored.superseded_candidates, 1);
    assert_eq!(stored.duration_ms, Some(48));
    assert!(stored.finished_at.is_some());
    assert_eq!(stored.error, None);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn deleting_a_rule_set_takes_its_candidates_runs_and_exclusions_with_it() {
    let (services, db) = temp_services("scryer_maintenance_cascade").await;
    seed_rule_set(&services, "rule-a").await;
    let store = evaluation_store(&services);
    let rules = crate::MaintenanceRuleSetStore::new(services.datastore());

    store
        .create_candidate(&candidate("cand-1", "rule-a", "title-1", 1))
        .await
        .expect("create candidate");
    store
        .create_exclusion(&MaintenanceRuleExclusion {
            id: "excl-1".to_string(),
            rule_set_id: Some("rule-a".to_string()),
            title_id: "title-1".to_string(),
            subject_id: "title-1".to_string(),
            subject_kind: scryer_domain::MaintenanceRuleSubjectKind::Title,
            reason: String::new(),
            created_by: None,
            created_at: Utc::now(),
        })
        .await
        .expect("create exclusion");
    store
        .start_evaluation_run(&MaintenanceEvaluationRun {
            id: "run-1".to_string(),
            rule_set_id: "rule-a".to_string(),
            revision_number: 1,
            matcher_content_hash: "hash-1".to_string(),
            started_at: Utc::now(),
            finished_at: None,
            status: MaintenanceEvaluationRunStatus::Running,
            evaluated_count: 0,
            matched_count: 0,
            no_match_count: 0,
            unknown_count: 0,
            error_count: 0,
            canceled_candidates: 0,
            superseded_candidates: 0,
            duration_ms: None,
            error: None,
        })
        .await
        .expect("start run");

    rules
        .delete_rule_set("rule-a")
        .await
        .expect("delete rule set");

    assert!(
        store
            .list_candidates(&MaintenanceCandidateQuery::default())
            .await
            .expect("list")
            .is_empty()
    );
    assert!(store.list_exclusions(None).await.expect("list").is_empty());
    assert!(
        store
            .list_evaluation_runs(None, None)
            .await
            .expect("list")
            .is_empty()
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn evaluation_mode_and_enabled_move_together() {
    let (services, db) = temp_services("scryer_maintenance_rule_mode").await;
    seed_rule_set(&services, "rule-a").await;
    let rules = crate::MaintenanceRuleSetStore::new(services.datastore());

    rules
        .update_rule_set_evaluation_mode(
            "rule-a",
            MaintenanceEvaluationMode::Shadow,
            true,
            Utc::now(),
        )
        .await
        .expect("arm rule");
    let armed = rules
        .get_rule_set("rule-a")
        .await
        .expect("read")
        .expect("exists");
    assert_eq!(armed.evaluation_mode, MaintenanceEvaluationMode::Shadow);
    assert!(armed.enabled);
    assert_eq!(
        armed.current_revision_number, 1,
        "a mode change never appends a revision"
    );

    rules
        .update_rule_set_evaluation_mode(
            "rule-a",
            MaintenanceEvaluationMode::Disabled,
            false,
            Utc::now(),
        )
        .await
        .expect("disarm rule");
    let disarmed = rules
        .get_rule_set("rule-a")
        .await
        .expect("read")
        .expect("exists");
    assert_eq!(
        disarmed.evaluation_mode,
        MaintenanceEvaluationMode::Disabled
    );
    assert!(!disarmed.enabled);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn the_execution_lease_is_exclusive_and_reclaimable_when_stale() {
    let (services, db) = temp_services("scryer_maintenance_lease").await;
    seed_rule_set(&services, "rule-a").await;
    let store = evaluation_store(&services);
    let now = Utc::now();

    let mut row = candidate("cand-1", "rule-a", "title-1", 1);
    row.state = MaintenanceCandidateState::Due;
    store.create_candidate(&row).await.expect("create");

    let stale_before = now - chrono::Duration::hours(1);
    assert!(
        store
            .lease_candidate_for_execution("cand-1", stale_before, now)
            .await
            .expect("first lease"),
        "a due candidate must be leasable"
    );
    assert!(
        !store
            .lease_candidate_for_execution("cand-1", stale_before, now)
            .await
            .expect("second lease"),
        "a fresh executing lease must be exclusive"
    );

    // A lease whose worker crashed goes stale and may be reclaimed.
    let later = now + chrono::Duration::hours(2);
    assert!(
        store
            .lease_candidate_for_execution("cand-1", later - chrono::Duration::hours(1), later)
            .await
            .expect("stale re-lease"),
        "a stale executing lease must be reclaimable"
    );

    // Terminal candidates are never leasable.
    store
        .transition_candidate_state(
            "cand-1",
            MaintenanceCandidateState::Succeeded,
            "action_succeeded",
            &[MaintenanceCandidateState::Executing],
            later,
        )
        .await
        .expect("finish");
    assert!(
        !store
            .lease_candidate_for_execution("cand-1", stale_before, later)
            .await
            .expect("terminal lease"),
        "a terminal candidate must not be leasable"
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn due_selection_returns_only_actionable_states_past_their_due_time() {
    let (services, db) = temp_services("scryer_maintenance_due").await;
    seed_rule_set(&services, "rule-a").await;
    let store = evaluation_store(&services);
    let now = Utc::now();

    let mut due_row = candidate("cand-due", "rule-a", "title-1", 1);
    due_row.due_at = now - chrono::Duration::hours(1);
    store.create_candidate(&due_row).await.expect("create due");

    let mut future_row = candidate("cand-future", "rule-a", "title-2", 1);
    future_row.due_at = now + chrono::Duration::days(3);
    store
        .create_candidate(&future_row)
        .await
        .expect("create future");

    let mut blocked_row = candidate("cand-blocked", "rule-a", "title-3", 1);
    blocked_row.state = MaintenanceCandidateState::Blocked;
    blocked_row.due_at = now - chrono::Duration::hours(2);
    store
        .create_candidate(&blocked_row)
        .await
        .expect("create blocked");

    let due = store
        .list_due_candidates("rule-a", now, now - chrono::Duration::hours(1), 10)
        .await
        .expect("list due");
    let ids: Vec<&str> = due.iter().map(|row| row.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["cand-blocked", "cand-due"],
        "oldest due first; blocked re-checks, future stays out"
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn due_selection_returns_an_abandoned_lease_but_not_a_live_one() {
    let (services, db) = temp_services("scryer_maintenance_due_stale_lease").await;
    seed_rule_set(&services, "rule-a").await;
    let store = evaluation_store(&services);
    let now = Utc::now();

    let mut row = candidate("cand-leased", "rule-a", "title-1", 1);
    row.state = MaintenanceCandidateState::Due;
    row.due_at = now - chrono::Duration::hours(1);
    store.create_candidate(&row).await.expect("create");
    assert!(
        store
            .lease_candidate_for_execution("cand-leased", now - chrono::Duration::hours(1), now)
            .await
            .expect("lease"),
        "the candidate must lease before it can be stranded"
    );

    // A worker holding a fresh lease is doing its job; selecting the row again
    // would put two workers on one candidate.
    let live = store
        .list_due_candidates("rule-a", now, now - chrono::Duration::hours(1), 10)
        .await
        .expect("list with a live lease");
    assert!(
        live.is_empty(),
        "a fresh executing lease must stay out of the selection: {live:?}"
    );

    // Once the lease has gone stale nobody is driving the row. Before this arm
    // existed it was never selected again, so the lease's own reclaim branch
    // could not fire and the row stayed `executing` forever — permanently
    // inflating the count destructive arming makes an operator acknowledge.
    let later = now + chrono::Duration::hours(2);
    let stranded = store
        .list_due_candidates("rule-a", later, later - chrono::Duration::hours(1), 10)
        .await
        .expect("list with a stale lease");
    let ids: Vec<&str> = stranded.iter().map(|row| row.id.as_str()).collect();
    assert_eq!(ids, vec!["cand-leased"]);
    assert_eq!(stranded[0].state, MaintenanceCandidateState::Executing);

    assert!(
        store
            .lease_candidate_for_execution("cand-leased", later - chrono::Duration::hours(1), later)
            .await
            .expect("reclaim"),
        "the selected stale row must be re-leasable, or the reclaim is still dead code"
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn a_candidate_transition_lands_only_from_a_state_the_caller_expected() {
    let (services, db) = temp_services("scryer_maintenance_transition_cas").await;
    seed_rule_set(&services, "rule-a").await;
    let store = evaluation_store(&services);
    let now = Utc::now();

    let mut row = candidate("cand-1", "rule-a", "title-1", 1);
    row.state = MaintenanceCandidateState::Due;
    store.create_candidate(&row).await.expect("create");
    assert!(
        store
            .lease_candidate_for_execution("cand-1", now - chrono::Duration::hours(1), now)
            .await
            .expect("lease")
    );

    // The evaluator's expectation set never includes `executing`, so its write
    // finds no row and says so rather than cancelling a leased candidate out
    // from under the worker that owns it.
    assert!(
        !store
            .transition_candidate_state(
                "cand-1",
                MaintenanceCandidateState::Canceled,
                "no_match",
                &[
                    MaintenanceCandidateState::Observing,
                    MaintenanceCandidateState::Blocked,
                ],
                now,
            )
            .await
            .expect("compare-and-set"),
        "a transition whose expectation does not hold must not write"
    );
    let unchanged = store
        .get_active_candidate("rule-a", "title-1")
        .await
        .expect("read")
        .expect("exists");
    assert_eq!(unchanged.state, MaintenanceCandidateState::Executing);
    assert_eq!(unchanged.state_reason, "execution_leased");

    // The lease holder's own write names `executing`, and lands.
    assert!(
        store
            .transition_candidate_state(
                "cand-1",
                MaintenanceCandidateState::Succeeded,
                "action_succeeded",
                &[MaintenanceCandidateState::Executing],
                now,
            )
            .await
            .expect("compare-and-set")
    );

    let refused = store
        .transition_candidate_state(
            "cand-1",
            MaintenanceCandidateState::Canceled,
            "no_match",
            &[],
            now,
        )
        .await
        .expect_err("an empty expectation is the unconditional write this replaced");
    assert!(refused.to_string().contains("expects"), "{refused}");

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn stale_sequence_worker_cannot_finish_a_newer_execution_lease() {
    let (services, db) = temp_services("scryer_maintenance_sequence_finish_lease_cas").await;
    seed_rule_set(&services, "rule-a").await;
    let store = evaluation_store(&services);
    let mut row = candidate("sequence-finish-lease", "rule-a", "title-lease", 1);
    row.action_kind = "action_sequence".to_string();
    row.state = MaintenanceCandidateState::Due;
    store
        .create_candidate(&row)
        .await
        .expect("create candidate");

    let first_lease_at = Utc::now();
    assert!(
        store
            .lease_candidate_for_execution(
                &row.id,
                first_lease_at - chrono::Duration::minutes(1),
                first_lease_at,
            )
            .await
            .expect("take first lease")
    );
    let replacement_lease_at = first_lease_at + chrono::Duration::seconds(1);
    assert!(
        store
            .lease_candidate_for_execution(&row.id, replacement_lease_at, replacement_lease_at,)
            .await
            .expect("reclaim stale execution lease")
    );

    assert!(
        !store
            .finish_leased_candidate(
                &row.id,
                first_lease_at,
                MaintenanceCandidateState::Blocked,
                "unknown_at_execution",
                replacement_lease_at + chrono::Duration::seconds(1),
            )
            .await
            .expect("stale sequence finisher is a compare-and-set miss"),
        "the first worker must not overwrite the newer executing lease"
    );
    let current = store
        .get_active_candidate("rule-a", "title-lease")
        .await
        .expect("reload candidate")
        .expect("candidate remains active");
    assert_eq!(current.state, MaintenanceCandidateState::Executing);
    assert_eq!(current.updated_at, replacement_lease_at);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn action_runs_append_finish_and_enforce_attempt_uniqueness() {
    let (services, db) = temp_services("scryer_maintenance_action_runs").await;
    seed_rule_set(&services, "rule-a").await;
    let store = evaluation_store(&services);
    let now = Utc::now();

    let row = candidate("cand-1", "rule-a", "title-1", 1);
    store.create_candidate(&row).await.expect("create");

    let mut run = scryer_domain::LifecycleActionRun {
        id: "run-1".to_string(),
        candidate_id: "cand-1".to_string(),
        rule_set_id: "rule-a".to_string(),
        revision_number: 1,
        title_id: "title-1".to_string(),
        subject_id: "title-1".to_string(),
        subject_kind: "title".to_string(),
        action_kind: "unmonitor_scope_keep_files".to_string(),
        match_generation: 1,
        idempotency_key: "cand-1:1:unmonitor:abcd".to_string(),
        attempt: 1,
        status: scryer_domain::LifecycleActionRunStatus::Running,
        hold_reason: None,
        error: None,
        detail: "{}".to_string(),
        started_at: now,
        finished_at: None,
        created_at: now,
    };
    store.start_action_run(&run).await.expect("start");

    // The same (key, attempt) must be refused: that is the duplicate-execution
    // detector.
    let mut duplicate = run.clone();
    duplicate.id = "run-dup".to_string();
    assert!(
        store.start_action_run(&duplicate).await.is_err(),
        "duplicate (idempotency_key, attempt) must be refused"
    );

    // A retry is the same key with the next attempt.
    let mut retry = run.clone();
    retry.id = "run-2".to_string();
    retry.attempt = 2;
    store.start_action_run(&retry).await.expect("retry row");

    run.status = scryer_domain::LifecycleActionRunStatus::Succeeded;
    run.detail = r#"{"unmonitored":true}"#.to_string();
    run.finished_at = Some(now + chrono::Duration::seconds(1));
    store.finish_action_run(&run).await.expect("finish");

    let listed = store
        .list_action_runs(Some("rule-a"), None, Some(10))
        .await
        .expect("list");
    assert_eq!(listed.len(), 2);
    let finished = listed
        .iter()
        .find(|stored| stored.id == "run-1")
        .expect("finished row");
    assert_eq!(
        finished.status,
        scryer_domain::LifecycleActionRunStatus::Succeeded
    );
    assert_eq!(finished.detail, r#"{"unmonitored":true}"#);
    assert!(finished.finished_at.is_some());

    let by_candidate = store
        .list_action_runs(None, Some("cand-1"), None)
        .await
        .expect("list by candidate");
    assert_eq!(by_candidate.len(), 2);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn storage_held_action_refund_rekeys_the_run_and_is_atomic() {
    let (services, db) = temp_services("scryer_maintenance_held_action_refund").await;
    seed_rule_set(&services, "rule-a").await;
    let store = evaluation_store(&services);
    let now = Utc::now();

    let mut row = candidate("cand-1", "rule-a", "title-1", 1);
    row.action_kind = "unmonitor_scope_delete_files".to_string();
    store.create_candidate(&row).await.expect("create");
    assert!(
        store
            .transition_candidate_state(
                &row.id,
                MaintenanceCandidateState::Executing,
                "execution_leased",
                &[MaintenanceCandidateState::Observing],
                now,
            )
            .await
            .expect("lease")
    );
    store
        .record_candidate_attempts(&row.id, 1, now)
        .await
        .expect("reserve attempt");

    let running = scryer_domain::LifecycleActionRun {
        id: "run-1".to_string(),
        candidate_id: row.id.clone(),
        rule_set_id: row.rule_set_id.clone(),
        revision_number: row.revision_number,
        title_id: row.title_id.clone(),
        subject_id: row.subject_id.clone(),
        subject_kind: row.subject_kind.clone(),
        action_kind: row.action_kind.clone(),
        match_generation: row.match_generation,
        idempotency_key: "cand-1:1:delete".to_string(),
        attempt: 1,
        status: scryer_domain::LifecycleActionRunStatus::Running,
        hold_reason: None,
        error: None,
        detail: r#"{"scoped_deletion":1,"files":[]}"#.to_string(),
        started_at: now,
        finished_at: None,
        created_at: now,
    };
    store.start_action_run(&running).await.expect("start");

    let mut held = running.clone();
    held.status = scryer_domain::LifecycleActionRunStatus::Held;
    held.idempotency_key = format!("hold:{}", held.id);
    held.hold_reason = Some("storage_capacity_unavailable".to_string());
    held.finished_at = Some(now + chrono::Duration::seconds(1));
    assert!(
        store
            .finish_held_action_run_and_release_attempt(&held, 1)
            .await
            .expect("finish held and refund")
    );

    let refunded = store
        .get_active_candidate("rule-a", "title-1")
        .await
        .expect("read")
        .expect("candidate remains active");
    assert_eq!(refunded.state, MaintenanceCandidateState::Blocked);
    assert_eq!(refunded.action_attempts, 0);
    assert_eq!(
        store
            .latest_scoped_deletion_action_run(&row.id, 1, &row.action_kind)
            .await
            .expect("checkpoint")
            .expect("held checkpoint")
            .id,
        held.id,
        "the rekeyed hold retains durable checkpoint evidence"
    );
    assert!(
        !store
            .finish_held_action_run_and_release_attempt(&held, 1)
            .await
            .expect("second finish is a compare-and-set miss"),
        "a repeated hold must not refund twice"
    );

    // Releasing the original execution key makes its same attempt reusable.
    assert!(
        store
            .transition_candidate_state(
                &row.id,
                MaintenanceCandidateState::Executing,
                "execution_leased",
                &[MaintenanceCandidateState::Blocked],
                now + chrono::Duration::seconds(2),
            )
            .await
            .expect("re-lease")
    );
    store
        .record_candidate_attempts(&row.id, 1, now + chrono::Duration::seconds(2))
        .await
        .expect("reserve same attempt again");
    let mut retry = running.clone();
    retry.id = "run-2".to_string();
    retry.started_at = now + chrono::Duration::seconds(2);
    retry.created_at = retry.started_at;
    store
        .start_action_run(&retry)
        .await
        .expect("same attempt and original execution key are reusable");

    let mut wrong_generation = retry.clone();
    wrong_generation.status = scryer_domain::LifecycleActionRunStatus::Held;
    wrong_generation.match_generation = 2;
    wrong_generation.idempotency_key = format!("hold:{}", wrong_generation.id);
    wrong_generation.hold_reason = Some("storage_capacity_unavailable".to_string());
    wrong_generation.finished_at = Some(now + chrono::Duration::seconds(3));
    assert!(
        !store
            .finish_held_action_run_and_release_attempt(&wrong_generation, 1)
            .await
            .expect("wrong generation is a conditional miss")
    );
    let mut wrong_attempt = retry.clone();
    wrong_attempt.status = scryer_domain::LifecycleActionRunStatus::Held;
    wrong_attempt.attempt = 2;
    wrong_attempt.idempotency_key = format!("hold:{}", wrong_attempt.id);
    wrong_attempt.hold_reason = Some("storage_capacity_unavailable".to_string());
    wrong_attempt.finished_at = Some(now + chrono::Duration::seconds(3));
    assert!(
        !store
            .finish_held_action_run_and_release_attempt(&wrong_attempt, 2)
            .await
            .expect("wrong attempt is a conditional miss")
    );
    let unchanged = store
        .get_active_candidate("rule-a", "title-1")
        .await
        .expect("read")
        .expect("candidate remains active");
    assert_eq!(unchanged.state, MaintenanceCandidateState::Executing);
    assert_eq!(unchanged.action_attempts, 1);

    let mut no_longer_running = retry.clone();
    no_longer_running.status = scryer_domain::LifecycleActionRunStatus::Succeeded;
    no_longer_running.finished_at = Some(now + chrono::Duration::seconds(4));
    store
        .finish_action_run(&no_longer_running)
        .await
        .expect("finish retry before synthetic hold");
    no_longer_running.status = scryer_domain::LifecycleActionRunStatus::Held;
    no_longer_running.idempotency_key = format!("hold:{}", no_longer_running.id);
    no_longer_running.hold_reason = Some("storage_capacity_unavailable".to_string());
    assert!(
        store
            .finish_held_action_run_and_release_attempt(&no_longer_running, 1)
            .await
            .is_err(),
        "a non-running action row rolls back the candidate refund"
    );
    let rolled_back = store
        .get_active_candidate("rule-a", "title-1")
        .await
        .expect("read")
        .expect("candidate remains active");
    assert_eq!(rolled_back.state, MaintenanceCandidateState::Executing);
    assert_eq!(rolled_back.action_attempts, 1);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn maintenance_scoped_deletion_checkpoint_survives_unbounded_safety_holds() {
    let (services, db) = temp_services("scryer_maintenance_checkpoint_holds").await;
    seed_rule_set(&services, "rule-a").await;
    let store = evaluation_store(&services);
    store
        .create_candidate(&candidate("cand-1", "rule-a", "title-1", 1))
        .await
        .unwrap();
    let now = Utc::now();
    let checkpoint = scryer_domain::LifecycleActionRun {
        id: "checkpoint".into(),
        candidate_id: "cand-1".into(),
        rule_set_id: "rule-a".into(),
        revision_number: 1,
        title_id: "title-1".into(),
        subject_kind: "episode".into(),
        subject_id: "episode-1".into(),
        action_kind: "unmonitor_scope_delete_files".into(),
        match_generation: 1,
        idempotency_key: "checkpoint:1".into(),
        attempt: 1,
        status: scryer_domain::LifecycleActionRunStatus::Failed,
        hold_reason: None,
        error: Some("interrupted deletion".into()),
        detail: r#"{"scoped_deletion":1,"files":[]}"#.into(),
        started_at: now,
        finished_at: Some(now),
        created_at: now,
    };
    store.start_action_run(&checkpoint).await.unwrap();
    for index in 0..102 {
        let mut hold = checkpoint.clone();
        hold.id = format!("hold-{index}");
        hold.idempotency_key = hold.id.clone();
        hold.status = scryer_domain::LifecycleActionRunStatus::Held;
        hold.detail = if index == 101 {
            // The underscore in the checkpoint key is literal, not a LIKE wildcard.
            r#"{"scopedXdeletion":1}"#.into()
        } else {
            "{}".into()
        };
        hold.started_at = now + chrono::Duration::seconds(index + 1);
        store.start_action_run(&hold).await.unwrap();
    }
    assert!(
        store
            .list_action_runs(None, Some("cand-1"), Some(100))
            .await
            .unwrap()
            .iter()
            .all(|run| run.id != checkpoint.id)
    );
    assert_eq!(
        store
            .latest_scoped_deletion_action_run("cand-1", 1, "unmonitor_scope_delete_files")
            .await
            .unwrap()
            .unwrap()
            .id,
        checkpoint.id
    );
    assert!(
        store
            .latest_scoped_deletion_action_run(
                "other-candidate",
                1,
                "unmonitor_scope_delete_files",
            )
            .await
            .unwrap()
            .is_none()
    );
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn effect_arming_and_attempts_round_trip() {
    let (services, db) = temp_services("scryer_maintenance_arming").await;
    seed_rule_set(&services, "rule-a").await;
    let rules = crate::MaintenanceRuleSetStore::new(services.datastore());
    let store = evaluation_store(&services);
    let now = Utc::now();

    rules
        .update_rule_set_arming(
            "rule-a",
            scryer_domain::MaintenanceEffectArming::Destructive,
            now,
        )
        .await
        .expect("arm");
    let armed = rules
        .get_rule_set("rule-a")
        .await
        .expect("read")
        .expect("exists");
    assert_eq!(
        armed.effect_arming,
        scryer_domain::MaintenanceEffectArming::Destructive
    );
    assert_eq!(
        armed.current_revision_number, 1,
        "arming never appends a revision"
    );

    let row = candidate("cand-1", "rule-a", "title-1", 1);
    store.create_candidate(&row).await.expect("create");
    store
        .record_candidate_attempts("cand-1", 2, now)
        .await
        .expect("attempts");
    let stored = store
        .get_active_candidate("rule-a", "title-1")
        .await
        .expect("read")
        .expect("exists");
    assert_eq!(stored.action_attempts, 2);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn maintenance_scope_candidates_and_exclusions_have_independent_subject_identity() {
    let (services, db) = temp_services("maintenance_scope_identity").await;
    seed_rule_set(&services, "rule-a").await;
    let store = evaluation_store(&services);
    for (index, (kind, subject)) in [
        ("title", "title-1"),
        ("season", "season-1"),
        ("episode", "episode-1"),
        ("episode", "episode-2"),
    ]
    .into_iter()
    .enumerate()
    {
        let mut row = candidate(&format!("candidate-{index}"), "rule-a", "title-1", 1);
        row.subject_kind = kind.into();
        row.subject_id = subject.into();
        store.create_candidate(&row).await.unwrap();
        let loaded = store
            .get_active_subject_candidate("rule-a", kind, subject)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded.id, row.id);
        assert_eq!(loaded.due_at, row.due_at);
        assert_eq!(
            store
                .max_subject_match_generation("rule-a", kind, subject)
                .await
                .unwrap(),
            1
        );
        let mut duplicate = row.clone();
        duplicate.id.push_str("-duplicate");
        assert!(store.create_candidate(&duplicate).await.is_err());
        let exclusion = MaintenanceRuleExclusion {
            id: format!("exclusion-{index}"),
            rule_set_id: None,
            title_id: "title-1".into(),
            subject_kind: MaintenanceRuleSubjectKind::parse_storage(kind).unwrap(),
            subject_id: subject.into(),
            reason: "scope".into(),
            created_by: None,
            created_at: Utc::now(),
        };
        store.create_exclusion(&exclusion).await.unwrap();
    }
    assert_eq!(store.list_exclusions(None).await.unwrap().len(), 4);
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn action_sequence_migration_preserves_pre_0231_in_flight_legacy_rows() {
    crate::spellfix::register_spellfix_auto_extension().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let db = directory.path().join("legacy.db");
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .unwrap();
    crate::migrations::replay_source_catalog_for_fresh_install(&pool, Some(230), true)
        .await
        .unwrap();
    run_embedded_migration(&pool, r#"
        INSERT INTO maintenance_rule_sets (id, name, enabled, evaluation_mode, effect_arming) VALUES ('rule', 'legacy', 1, 'observe', 'destructive');
        INSERT INTO lifecycle_candidates (id, rule_set_id, revision_number, title_id, subject_kind, subject_id, match_generation, first_matched_at, due_at, action_attempts) VALUES ('candidate', 'rule', 3, 'title', 'title', 'title', 7, '2026-01-01T00:00:00Z', '2026-02-01T00:00:00Z', 2);
        INSERT INTO maintenance_rule_exclusions (id, title_id, subject_kind, subject_id, reason) VALUES ('exclusion', 'title', 'title', 'title', 'keep');
        INSERT INTO lifecycle_action_runs (id, candidate_id, rule_set_id, revision_number, title_id, subject_kind, subject_id, action_kind, match_generation, idempotency_key, attempt, status, detail, started_at) VALUES ('run', 'candidate', 'rule', 3, 'title', 'title', 'title', 'unmonitor_scope_keep_files', 7, 'legacy-key', 2, 'failed', '{"evidence":true}', '2026-01-02T00:00:00Z');
    "#).await;
    crate::migrations::run_migrations(&pool, crate::types::MigrationMode::Apply)
        .await
        .unwrap();
    let row = sqlx::query("SELECT subject_kind, subject_id, first_matched_at, due_at, match_generation, action_attempts FROM lifecycle_candidates WHERE id = 'candidate'").fetch_one(&pool).await.unwrap();
    assert_eq!(row.get::<String, _>("subject_kind"), "title");
    assert_eq!(row.get::<String, _>("subject_id"), "title");
    assert_eq!(
        row.get::<String, _>("first_matched_at"),
        "2026-01-01T00:00:00Z"
    );
    assert_eq!(row.get::<String, _>("due_at"), "2026-02-01T00:00:00Z");
    assert_eq!(row.get::<i64, _>("match_generation"), 7);
    assert_eq!(row.get::<i64, _>("action_attempts"), 2);
    let row = sqlx::query("SELECT subject_kind, subject_id, detail, idempotency_key FROM lifecycle_action_runs WHERE id = 'run'").fetch_one(&pool).await.unwrap();
    assert_eq!(row.get::<String, _>("subject_kind"), "title");
    assert_eq!(row.get::<String, _>("subject_id"), "title");
    assert_eq!(row.get::<String, _>("detail"), r#"{"evidence":true}"#);
    assert_eq!(row.get::<String, _>("idempotency_key"), "legacy-key");
    let arming: String =
        sqlx::query_scalar("SELECT effect_arming FROM maintenance_rule_sets WHERE id = 'rule'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(arming, "destructive");
    let exclusion: String = sqlx::query_scalar(
        "SELECT subject_id FROM maintenance_rule_exclusions WHERE id = 'exclusion'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(exclusion, "title");
    crate::migrations::run_migrations(&pool, crate::types::MigrationMode::ValidateOnly)
        .await
        .unwrap();
}

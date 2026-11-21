//! Store round-trips for the lifecycle evaluator's three tables, including
//! the two invariants that live in the schema rather than in Rust: one active
//! candidate per (rule, title), and one global exclusion per title.

use super::*;
use crate::workflow::stores::WorkflowOperationStore;
use scryer_application::{
    JobKey, JobRunRecord, JobRunRepository, JobRunStatus, JobTriggerSource,
    LifecycleActionRunRepository, LifecycleActionStepRepository, LifecycleCandidateQuery,
    LifecycleCandidateRepository, LifecycleEvaluationRunRepository, LifecycleExclusionRepository,
    LifecycleRuleSetRepository, LifecycleSearchJobRunCreation,
    LifecycleSequenceCompletionRepository,
};
use scryer_domain::{
    LifecycleActionJobReceipt, LifecycleActionJobReceiptState, LifecycleActionStepKey,
    LifecycleActionStepRun, LifecycleActionStepState, LifecycleCandidate, LifecycleCandidateState,
    LifecycleEvaluationRun, LifecycleEvaluationRunStatus, LifecycleRuleExclusion,
    LifecycleRuleRevision, LifecycleRuleSet, LifecycleRuleState, LifecycleRuleSubjectKind,
    LifecycleSequenceTerminalMembership, LifecycleSequenceTerminalOutcome,
};

fn evaluation_store(services: &SqliteServices) -> crate::LifecycleEvaluationStore {
    crate::LifecycleEvaluationStore::new(services.datastore())
}

/// Candidates, runs, and exclusions all cascade from a rule set, so every test
/// here needs one to exist first.
async fn seed_rule_set(services: &SqliteServices, id: &str) {
    let now = Utc::now();
    crate::LifecycleRuleSetStore::new(services.datastore())
        .create_rule_set(
            &LifecycleRuleSet {
                id: id.to_string(),
                name: format!("rule {id}"),
                description: String::new(),
                state: LifecycleRuleState::Disabled,
                edited_since_enabled: false,
                destructive_confirmed_revision: None,
                library_ids: Vec::new(),
                system_owned: false,
                current_revision_number: 1,
                schedule: scryer_domain::default_lifecycle_rule_schedule(),
                max_titles_per_run: None,
                created_at: now,
                updated_at: now,
            },
            &LifecycleRuleRevision {
                id: format!("{id}-rev-1"),
                rule_set_id: id.to_string(),
                revision_number: 1,
                rego_source: format!("package scryer.lifecycle.user.{id}\nmatch := true\n"),
                declared_actions_json:
                    r#"{"kind":"unmonitor_scope_keep_files","schema_version":1}"#.to_string(),
                grace_days: 7,
                source_content_hash: "hash-1".to_string(),
                created_by: None,
                created_at: now,
                builder_model_json: None,
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
        source_content_hash: "hash-1".to_string(),
        title_id: title_id.to_string(),
        subject_id: title_id.to_string(),
        library_id: "library-1".to_string(),
        facet: "movie".to_string(),
        subject_kind: "title".to_string(),
        match_generation: generation,
        state: LifecycleCandidateState::Observing,
        state_reason: "first_match".to_string(),
        state_cause_code: None,
        reason_codes: vec!["stale".to_string(), "unwatched".to_string()],
        action_kind: "unmonitor_scope_keep_files".to_string(),
        grace_days: 7,
        first_matched_at: now,
        last_matched_at: now,
        due_at: now + chrono::Duration::days(7),
        last_evaluated_at: now,
        held_since: None,
        action_attempts: 0,
        decided_actions_json: None,
        actions_conflict: None,
        created_at: now,
        updated_at: now,
    }
}

fn sequence_step_run(candidate: &LifecycleCandidate, attempt: i64) -> LifecycleActionStepRun {
    let now = Utc::now();
    LifecycleActionStepRun {
        key: LifecycleActionStepKey {
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
        state: LifecycleActionStepState::Running,
        attempt,
        lease_id: None,
        lease_expires_at: Some(now + chrono::Duration::minutes(5)),
        hold_reason: None,
        error: None,
        cause_code: None,
        created_at: now,
        updated_at: now,
        finished_at: None,
    }
}

#[tokio::test]
async fn candidates_round_trip_with_their_reason_codes_and_timestamps() {
    let (services, db) = temp_services("scryer_lifecycle_candidates").await;
    seed_rule_set(&services, "rule-a").await;
    let store = evaluation_store(&services);

    let created = candidate("cand-1", "rule-a", "title-1", 1);
    store
        .create_candidate(&created)
        .await
        .expect("create candidate");

    let loaded = store
        .get_active_subject_candidate("rule-a", "title", "title-1")
        .await
        .expect("read candidate")
        .expect("candidate exists");
    assert_eq!(loaded.id, created.id);
    assert_eq!(loaded.state, LifecycleCandidateState::Observing);
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

/// One title's cross-level markers land in one batch: each row is set or
/// cleared to its own value, a row outside the expected states is left
/// alone, and the count is the rows written.
#[tokio::test]
async fn conflict_markers_are_written_in_one_batch_with_a_per_row_state_check() {
    let (services, db) = temp_services("scryer_lifecycle_candidate_conflict_batch").await;
    seed_rule_set(&services, "rule-a").await;
    let store = evaluation_store(&services);
    for (id, title) in [
        ("cand-1", "title-1"),
        ("cand-2", "title-2"),
        ("cand-3", "title-3"),
    ] {
        store
            .create_candidate(&candidate(id, "rule-a", title, 1))
            .await
            .expect("create candidate");
    }
    store
        .transition_candidate_state(
            "cand-3",
            LifecycleCandidateState::Canceled,
            "no_match",
            &[LifecycleCandidateState::Observing],
            Utc::now(),
        )
        .await
        .expect("cancel");
    let marker = |other: &str| scryer_domain::LifecycleActionsConflict {
        head: "monitor".into(),
        other_subject_kind: "season".into(),
        other_subject_id: other.into(),
        other_head: "delete_season_files".into(),
    };
    let expected = [
        LifecycleCandidateState::Observing,
        LifecycleCandidateState::Blocked,
    ];

    let written = store
        .set_candidate_actions_conflicts(
            &[
                ("cand-1".to_string(), Some(marker("season-1"))),
                ("cand-2".to_string(), Some(marker("season-2"))),
                ("cand-3".to_string(), Some(marker("season-3"))),
            ],
            &expected,
            Utc::now(),
        )
        .await
        .expect("set markers");
    assert_eq!(
        written, 2,
        "the canceled row is outside the expected states"
    );
    let markers = |rows: Vec<LifecycleCandidate>| {
        let mut markers: Vec<_> = rows
            .into_iter()
            .map(|row| (row.id, row.actions_conflict))
            .collect();
        markers.sort_by(|a, b| a.0.cmp(&b.0));
        markers
    };
    let ids = ["cand-1", "cand-2", "cand-3"].map(String::from).to_vec();
    assert_eq!(
        markers(store.get_candidates_by_ids(&ids).await.expect("read")),
        vec![
            ("cand-1".to_string(), Some(marker("season-1"))),
            ("cand-2".to_string(), Some(marker("season-2"))),
            ("cand-3".to_string(), None),
        ]
    );

    // A clear and a change in the same batch.
    let written = store
        .set_candidate_actions_conflicts(
            &[
                ("cand-1".to_string(), None),
                ("cand-2".to_string(), Some(marker("season-9"))),
            ],
            &expected,
            Utc::now(),
        )
        .await
        .expect("change markers");
    assert_eq!(written, 2);
    assert_eq!(
        markers(store.get_candidates_by_ids(&ids).await.expect("read")),
        vec![
            ("cand-1".to_string(), None),
            ("cand-2".to_string(), Some(marker("season-9"))),
            ("cand-3".to_string(), None),
        ]
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn a_rule_set_may_hold_only_one_active_candidate_per_title() {
    let (services, db) = temp_services("scryer_lifecycle_candidate_invariant").await;
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
            LifecycleCandidateState::Canceled,
            "no_match",
            &[LifecycleCandidateState::Observing],
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
            .max_subject_match_generation("rule-a", "title", "title-1")
            .await
            .expect("max generation"),
        2,
        "generations count terminal rows too"
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn sequence_step_journal_and_terminal_marker_block_readmission_until_confirmed_no_match() {
    let (services, db) = temp_services("scryer_lifecycle_sequence_latch").await;
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
                LifecycleCandidateState::Executing,
                "execution_leased",
                &[LifecycleCandidateState::Observing],
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
        scryer_application::LifecycleActionStepClaim::Claimed(step) => step,
        other => panic!("unexpected step claim: {other:?}"),
    };
    let finished_at = leased_at + chrono::Duration::seconds(1);
    let mut completed = claimed.clone();
    completed.state = LifecycleActionStepState::Succeeded;
    completed.updated_at = finished_at;
    completed.finished_at = Some(finished_at);
    assert!(
        store
            .finish_action_step(&completed, "step-lease-1")
            .await
            .expect("finish step")
    );
    let membership = LifecycleSequenceTerminalMembership {
        id: "sequence-membership-1".to_string(),
        candidate_id: first.id.clone(),
        rule_set_id: first.rule_set_id.clone(),
        revision_number: first.revision_number,
        source_content_hash: first.source_content_hash.clone(),
        title_id: first.title_id.clone(),
        subject_kind: first.subject_kind.clone(),
        subject_id: first.subject_id.clone(),
        match_generation: first.match_generation,
        sequence_content_hash: "sequence-hash".to_string(),
        outcome: LifecycleSequenceTerminalOutcome::Succeeded,
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
                LifecycleCandidateState::Executing,
                leased_at,
                LifecycleCandidateState::Succeeded,
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
    let (services, db) = temp_services("scryer_lifecycle_sequence_failed_latch").await;
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
            LifecycleCandidateState::Executing,
            "execution_leased",
            &[LifecycleCandidateState::Observing],
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
        scryer_application::LifecycleActionStepClaim::Claimed(step) => step,
        other => panic!("unexpected claim: {other:?}"),
    };
    let finished_at = leased_at + chrono::Duration::seconds(1);
    let mut failed = claimed;
    failed.state = LifecycleActionStepState::Failed;
    failed.error = Some("retry budget exhausted".to_string());
    failed.updated_at = finished_at;
    failed.finished_at = Some(finished_at);
    assert!(
        store
            .finish_action_step(&failed, "failed-step-lease")
            .await
            .expect("persist terminal failure")
    );
    let membership = LifecycleSequenceTerminalMembership {
        id: "sequence-failed-membership".to_string(),
        candidate_id: first.id.clone(),
        rule_set_id: first.rule_set_id.clone(),
        revision_number: first.revision_number,
        source_content_hash: first.source_content_hash.clone(),
        title_id: first.title_id.clone(),
        subject_kind: first.subject_kind.clone(),
        subject_id: first.subject_id.clone(),
        match_generation: first.match_generation,
        sequence_content_hash: "sequence-hash".to_string(),
        outcome: LifecycleSequenceTerminalOutcome::Failed,
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
                LifecycleCandidateState::Executing,
                leased_at,
                LifecycleCandidateState::Failed,
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
    let (services, db) = temp_services("scryer_lifecycle_sequence_stale_terminal").await;
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
            LifecycleCandidateState::Executing,
            "execution_leased",
            &[LifecycleCandidateState::Observing],
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
        scryer_application::LifecycleActionStepClaim::Claimed(step) => step,
        other => panic!("unexpected claim: {other:?}"),
    };
    let finished_at = leased_at + chrono::Duration::seconds(1);
    let mut completed = claimed;
    completed.state = LifecycleActionStepState::Succeeded;
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
    let membership = LifecycleSequenceTerminalMembership {
        id: "stale-membership".to_string(),
        candidate_id: candidate.id.clone(),
        rule_set_id: candidate.rule_set_id.clone(),
        revision_number: candidate.revision_number,
        source_content_hash: candidate.source_content_hash.clone(),
        title_id: candidate.title_id.clone(),
        subject_kind: candidate.subject_kind.clone(),
        subject_id: candidate.subject_id.clone(),
        match_generation: candidate.match_generation,
        sequence_content_hash: "sequence-hash".to_string(),
        outcome: LifecycleSequenceTerminalOutcome::Succeeded,
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
                LifecycleCandidateState::Executing,
                leased_at,
                LifecycleCandidateState::Succeeded,
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
    let (services, db) = temp_services("scryer_lifecycle_sequence_retry_journal").await;
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
        scryer_application::LifecycleActionStepClaim::Claimed(step) => step,
        other => panic!("unexpected step claim: {other:?}"),
    };
    let mut held = claimed.clone();
    held.state = LifecycleActionStepState::Held;
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
        scryer_application::LifecycleActionStepClaim::Claimed(step) => step,
        other => panic!("unexpected held resume: {other:?}"),
    };
    assert_eq!(resumed.attempt, 0, "a hold does not spend failure budget");
    assert_eq!(resumed.intent_json, original.intent_json);
    assert_eq!(resumed.before_state_json, original.before_state_json);
    assert_eq!(resumed.provenance_json, original.provenance_json);

    let mut failed = resumed.clone();
    failed.state = LifecycleActionStepState::Failed;
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
        scryer_application::LifecycleActionStepClaim::Claimed(step) => step,
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
    let (services, db) = temp_services("scryer_lifecycle_sequence_claim_race").await;
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
    let first = crate::LifecycleEvaluationStore::new(services.datastore());
    let second = crate::LifecycleEvaluationStore::new(services.datastore());
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
                scryer_application::LifecycleActionStepClaim::Claimed(_)
            ))
            .count(),
        1
    );
    assert_eq!(
        claims
            .iter()
            .filter(|claim| matches!(claim, scryer_application::LifecycleActionStepClaim::Busy))
            .count(),
        1
    );
    let attempts: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM lifecycle_action_step_attempts
          WHERE candidate_id = ? AND match_generation = ? AND revision_number = ? AND step_id = ?",
    )
    .bind(&step.key.candidate_id)
    .bind(step.key.match_generation)
    .bind(step.key.revision_number)
    .bind(&step.key.step_id)
    .fetch_one(&services.pool)
    .await
    .expect("count immutable attempts");
    assert_eq!(attempts, 1);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn accepted_search_receipt_is_atomic_and_reused_across_a_later_dispatch_attempt() {
    let (services, db) = temp_services("scryer_lifecycle_search_receipt_atomic").await;
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
        scryer_application::LifecycleActionStepClaim::Claimed(step) => step,
        other => panic!("unexpected claim: {other:?}"),
    };
    let jobs = WorkflowOperationStore::new(services.datastore());
    let now = Utc::now();
    let initial_receipt = LifecycleActionJobReceipt {
        schema_version: 1,
        key: claimed.key.clone(),
        dispatch_attempt: 1,
        logical_request_key: claimed.key.logical_request_key(),
        request_hash: "stable-effective-request-hash".to_string(),
        job_run_id: Some("search-job-1".to_string()),
        state: LifecycleActionJobReceiptState::Accepted,
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
        jobs.create_lifecycle_search_job_run(&run, &unsupported_receipt)
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
        jobs.create_lifecycle_search_job_run(&run, &initial_receipt),
        competing_jobs.create_lifecycle_search_job_run(&run, &initial_receipt)
    );
    let creations = [
        left.expect("first dispatch resolves"),
        right.expect("second dispatch resolves"),
    ];
    assert_eq!(
        creations
            .iter()
            .filter(|creation| matches!(creation, LifecycleSearchJobRunCreation::Created { .. }))
            .count(),
        1,
        "only one concurrent transaction creates the durable job"
    );
    assert_eq!(
        creations
            .iter()
            .filter(|creation| matches!(creation, LifecycleSearchJobRunCreation::Existing { .. }))
            .count(),
        1,
        "the loser receives the accepted receipt rather than a duplicate job"
    );

    let retry_receipt = LifecycleActionJobReceipt {
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
        .create_lifecycle_search_job_run(&retry_run, &retry_receipt)
        .await
        .expect("return accepted logical receipt")
    {
        LifecycleSearchJobRunCreation::Existing { receipt } => {
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
    let pruned_retry = LifecycleActionJobReceipt {
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
        jobs.create_lifecycle_search_job_run(&pruned_run, &pruned_retry)
            .await
            .expect("accepted receipt survives pruned job history"),
        LifecycleSearchJobRunCreation::Existing { .. }
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
        scryer_application::LifecycleActionStepClaim::Claimed(step) => step,
        other => panic!("unexpected claim: {other:?}"),
    };
    let rollback_receipt = LifecycleActionJobReceipt {
        key: rollback_claim.key.clone(),
        request_hash: "different-request-hash".to_string(),
        ..initial_receipt.clone()
    };
    assert!(
        jobs.create_lifecycle_search_job_run(&run, &rollback_receipt)
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
    let (services, db) = temp_services("scryer_lifecycle_candidate_match").await;
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
        .get_active_subject_candidate("rule-a", "title", "title-1")
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
        .get_active_subject_candidate("rule-a", "title", "title-1")
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
        .get_active_subject_candidate("rule-a", "title", "title-1")
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

/// A page of repeat matches recorded together leaves every row exactly as
/// recording them one at a time does: held and unheld rows, changed and
/// unchanged reason codes, a row left out of the page, and an id with no row.
#[tokio::test]
async fn a_page_of_matches_leaves_the_rows_one_at_a_time_recording_leaves() {
    let (one_at_a_time, one_db) = temp_services("scryer_lifecycle_match_rows_single").await;
    let (together, together_db) = temp_services("scryer_lifecycle_match_rows_batch").await;
    let seeded: Vec<LifecycleCandidate> = (0..5)
        .map(|index| {
            candidate(
                &format!("cand-{index}"),
                "rule-a",
                &format!("title-{index}"),
                1,
            )
        })
        .collect();
    let held_at = Utc::now() + chrono::Duration::minutes(1);
    for services in [&one_at_a_time, &together] {
        seed_rule_set(services, "rule-a").await;
        let store = evaluation_store(services);
        for row in &seeded {
            store.create_candidate(row).await.expect("create");
        }
        for held in ["cand-1", "cand-3"] {
            store
                .hold_candidate(held, held_at, held_at)
                .await
                .expect("hold");
        }
    }

    let matched_at = held_at + chrono::Duration::minutes(1);
    let matches: Vec<scryer_application::LifecycleCandidateMatch> = [
        ("cand-0", vec!["stale".to_string(), "unwatched".to_string()]),
        ("cand-1", vec!["still_stale".to_string()]),
        ("cand-3", Vec::new()),
        ("cand-4", vec!["unwatched".to_string()]),
        ("cand-missing", vec!["stale".to_string()]),
    ]
    .into_iter()
    .map(
        |(id, reason_codes)| scryer_application::LifecycleCandidateMatch {
            id: id.to_string(),
            last_matched_at: matched_at,
            reason_codes,
            updated_at: matched_at,
        },
    )
    .collect();

    let single_store = evaluation_store(&one_at_a_time);
    for candidate_match in &matches {
        single_store
            .record_candidate_match(
                &candidate_match.id,
                candidate_match.last_matched_at,
                &candidate_match.reason_codes,
                candidate_match.updated_at,
            )
            .await
            .expect("record one match");
    }
    evaluation_store(&together)
        .record_candidate_matches(&matches)
        .await
        .expect("record the page");

    let ids: Vec<String> = seeded.iter().map(|row| row.id.clone()).collect();
    let mut expected = single_store
        .get_candidates_by_ids(&ids)
        .await
        .expect("read one-at-a-time rows");
    let mut actual = evaluation_store(&together)
        .get_candidates_by_ids(&ids)
        .await
        .expect("read batched rows");
    expected.sort_by(|left, right| left.id.cmp(&right.id));
    actual.sort_by(|left, right| left.id.cmp(&right.id));
    assert_eq!(expected.len(), seeded.len());
    assert_eq!(actual, expected);
    let untouched = actual
        .iter()
        .find(|row| row.id == "cand-2")
        .expect("row left out of the page");
    assert_eq!(untouched.reason_codes, seeded[2].reason_codes);

    let _ = std::fs::remove_file(one_db);
    let _ = std::fs::remove_file(together_db);
}

#[tokio::test]
async fn listing_filters_by_rule_state_and_library_and_counts_by_state() {
    let (services, db) = temp_services("scryer_lifecycle_candidate_listing").await;
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
            LifecycleCandidateState::Canceled,
            "no_match",
            &[LifecycleCandidateState::Observing],
            Utc::now(),
        )
        .await
        .expect("cancel");

    let by_rule = store
        .list_candidates(&LifecycleCandidateQuery {
            rule_set_id: Some("rule-a".to_string()),
            ..Default::default()
        })
        .await
        .expect("list");
    assert_eq!(by_rule.len(), 2);

    let by_library = store
        .list_candidates(&LifecycleCandidateQuery {
            library_id: Some("library-2".to_string()),
            ..Default::default()
        })
        .await
        .expect("list");
    assert_eq!(by_library.len(), 1);
    assert_eq!(by_library[0].id, "cand-2");

    let by_state = store
        .list_candidates(&LifecycleCandidateQuery {
            states: vec![LifecycleCandidateState::Canceled],
            ..Default::default()
        })
        .await
        .expect("list");
    assert_eq!(by_state.len(), 1);
    assert_eq!(by_state[0].id, "cand-3");

    let limited = store
        .list_candidates(&LifecycleCandidateQuery {
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
    assert_eq!(counts, vec![(LifecycleCandidateState::Observing, 2)]);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn exclusions_allow_one_global_row_and_one_row_per_rule_and_title() {
    let (services, db) = temp_services("scryer_lifecycle_exclusions").await;
    seed_rule_set(&services, "rule-a").await;
    seed_rule_set(&services, "rule-b").await;
    let store = evaluation_store(&services);

    let global = LifecycleRuleExclusion {
        id: "excl-global".to_string(),
        rule_set_id: None,
        title_id: "title-1".to_string(),
        subject_id: "title-1".to_string(),
        subject_kind: scryer_domain::LifecycleRuleSubjectKind::Title,
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
    let duplicate_global = LifecycleRuleExclusion {
        id: "excl-global-2".to_string(),
        ..global.clone()
    };
    assert!(
        store.create_exclusion(&duplicate_global).await.is_err(),
        "a title may carry at most one global exclusion"
    );

    let per_rule = LifecycleRuleExclusion {
        id: "excl-rule-a".to_string(),
        rule_set_id: Some("rule-a".to_string()),
        title_id: "title-1".to_string(),
        subject_id: "title-1".to_string(),
        subject_kind: scryer_domain::LifecycleRuleSubjectKind::Title,
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
            .create_exclusion(&LifecycleRuleExclusion {
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
    let (services, db) = temp_services("scryer_lifecycle_evaluation_runs").await;
    seed_rule_set(&services, "rule-a").await;
    let store = evaluation_store(&services);

    let mut run = LifecycleEvaluationRun {
        id: "run-1".to_string(),
        rule_set_id: "rule-a".to_string(),
        revision_number: 1,
        source_content_hash: "hash-1".to_string(),
        started_at: Utc::now(),
        finished_at: None,
        status: LifecycleEvaluationRunStatus::Running,
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
    assert_eq!(started[0].status, LifecycleEvaluationRunStatus::Running);
    assert_eq!(started[0].finished_at, None);

    run.finished_at = Some(Utc::now());
    run.status = LifecycleEvaluationRunStatus::Succeeded;
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
    assert_eq!(stored.status, LifecycleEvaluationRunStatus::Succeeded);
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
async fn housekeeping_prunes_only_evaluation_runs_older_than_retention() {
    use scryer_application::HousekeepingRepository;

    let (services, db) = temp_services("scryer_lifecycle_evaluation_run_retention").await;
    seed_rule_set(&services, "rule-a").await;
    let store = evaluation_store(&services);
    let now = Utc::now();
    let run = |id: &str, age_days: i64| LifecycleEvaluationRun {
        id: id.to_string(),
        rule_set_id: "rule-a".to_string(),
        revision_number: 1,
        source_content_hash: "hash-1".to_string(),
        started_at: now - chrono::Duration::days(age_days),
        finished_at: None,
        status: LifecycleEvaluationRunStatus::Succeeded,
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
    for (id, age_days) in [("run-old", 400), ("run-edge", 181), ("run-recent", 179)] {
        store
            .start_evaluation_run(&run(id, age_days))
            .await
            .expect("start run");
    }
    store
        .create_candidate(&candidate("cand-1", "rule-a", "title-1", 1))
        .await
        .expect("create candidate");
    sqlx::query(
        "INSERT INTO lifecycle_action_runs (id, candidate_id, rule_set_id, revision_number, title_id,
                                           action_kind, match_generation, idempotency_key, attempt, status,
                                           started_at)
         VALUES ('action-run-old', 'cand-1', 'rule-a', 1, 'title-1', 'delete', 1, 'action-run-key', 1,
                 'succeeded', '2020-01-01T00:00:00Z')",
    )
    .execute(&services.pool)
    .await
    .expect("seed old action run");

    let deleted = housekeeping_store(&services)
        .delete_lifecycle_evaluation_runs_older_than(180)
        .await
        .expect("prune evaluation runs");
    assert_eq!(deleted, 2);

    let remaining = store
        .list_evaluation_runs(None, None)
        .await
        .expect("list runs")
        .into_iter()
        .map(|run| run.id)
        .collect::<Vec<_>>();
    assert_eq!(remaining, vec!["run-recent".to_string()]);

    let action_runs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM lifecycle_action_runs")
        .fetch_one(&services.pool)
        .await
        .expect("count action runs");
    assert_eq!(action_runs, 1);
    let candidates: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM lifecycle_candidates")
        .fetch_one(&services.pool)
        .await
        .expect("count candidates");
    assert_eq!(candidates, 1);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn deleting_a_rule_set_takes_its_candidates_runs_and_exclusions_with_it() {
    let (services, db) = temp_services("scryer_lifecycle_cascade").await;
    seed_rule_set(&services, "rule-a").await;
    let store = evaluation_store(&services);
    let rules = crate::LifecycleRuleSetStore::new(services.datastore());

    store
        .create_candidate(&candidate("cand-1", "rule-a", "title-1", 1))
        .await
        .expect("create candidate");
    store
        .create_exclusion(&LifecycleRuleExclusion {
            id: "excl-1".to_string(),
            rule_set_id: Some("rule-a".to_string()),
            title_id: "title-1".to_string(),
            subject_id: "title-1".to_string(),
            subject_kind: scryer_domain::LifecycleRuleSubjectKind::Title,
            reason: String::new(),
            created_by: None,
            created_at: Utc::now(),
        })
        .await
        .expect("create exclusion");
    store
        .start_evaluation_run(&LifecycleEvaluationRun {
            id: "run-1".to_string(),
            rule_set_id: "rule-a".to_string(),
            revision_number: 1,
            source_content_hash: "hash-1".to_string(),
            started_at: Utc::now(),
            finished_at: None,
            status: LifecycleEvaluationRunStatus::Running,
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
            .list_candidates(&LifecycleCandidateQuery::default())
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
async fn a_state_write_moves_state_and_confirmation_together() {
    let (services, db) = temp_services("scryer_lifecycle_rule_mode").await;
    seed_rule_set(&services, "rule-a").await;
    let rules = crate::LifecycleRuleSetStore::new(services.datastore());

    assert!(
        rules
            .update_rule_set_state("rule-a", LifecycleRuleState::Enabled, None, Utc::now())
            .await
            .expect("enable rule")
    );
    let armed = rules
        .get_rule_set("rule-a")
        .await
        .expect("read")
        .expect("exists");
    assert_eq!(armed.state, LifecycleRuleState::Enabled);
    assert_eq!(armed.destructive_confirmed_revision, None);
    assert!(!armed.edited_since_enabled);
    assert_eq!(
        armed.current_revision_number, 1,
        "a state change never appends a revision"
    );

    assert!(
        rules
            .update_rule_set_state("rule-a", LifecycleRuleState::Disabled, None, Utc::now())
            .await
            .expect("disable rule")
    );
    let disarmed = rules
        .get_rule_set("rule-a")
        .await
        .expect("read")
        .expect("exists");
    assert_eq!(disarmed.state, LifecycleRuleState::Disabled);
    assert_eq!(disarmed.destructive_confirmed_revision, None);
    assert!(
        !rules
            .update_rule_set_state("absent", LifecycleRuleState::DryRun, None, Utc::now())
            .await
            .expect("missing rule"),
        "a missing rule reports no write"
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn the_execution_lease_is_exclusive_and_reclaimable_when_stale() {
    let (services, db) = temp_services("scryer_lifecycle_lease").await;
    seed_rule_set(&services, "rule-a").await;
    let store = evaluation_store(&services);
    let now = Utc::now();

    let mut row = candidate("cand-1", "rule-a", "title-1", 1);
    row.state = LifecycleCandidateState::Due;
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
            LifecycleCandidateState::Succeeded,
            "action_succeeded",
            &[LifecycleCandidateState::Executing],
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
    let (services, db) = temp_services("scryer_lifecycle_due").await;
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
    blocked_row.state = LifecycleCandidateState::Blocked;
    blocked_row.due_at = now - chrono::Duration::hours(2);
    store
        .create_candidate(&blocked_row)
        .await
        .expect("create blocked");

    let due = store
        .list_due_candidates(
            "rule-a",
            now,
            now - chrono::Duration::hours(1),
            None,
            None,
            10,
        )
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
async fn due_selection_leaves_out_a_hold_written_after_the_cutoff_until_it_passes() {
    let (services, db) = temp_services("scryer_lifecycle_due_held_after").await;
    seed_rule_set(&services, "rule-a").await;
    let store = evaluation_store(&services);
    let now = Utc::now();
    let evaluated_at = now - chrono::Duration::minutes(30);

    // Held by an action pass after the evaluation began.
    let mut held_since_evaluation = candidate("cand-held-later", "rule-a", "title-1", 1);
    held_since_evaluation.state = LifecycleCandidateState::Blocked;
    held_since_evaluation.due_at = now - chrono::Duration::hours(2);
    held_since_evaluation.updated_at = evaluated_at + chrono::Duration::minutes(1);
    // Held by the evaluation itself, stamped with its start.
    let mut held_by_evaluation = candidate("cand-held-by-eval", "rule-a", "title-2", 1);
    held_by_evaluation.state = LifecycleCandidateState::Blocked;
    held_by_evaluation.due_at = now - chrono::Duration::hours(2);
    held_by_evaluation.updated_at = evaluated_at;
    // Due work written after the cutoff is never left out.
    let mut due_row = candidate("cand-due", "rule-a", "title-3", 1);
    due_row.state = LifecycleCandidateState::Due;
    due_row.due_at = now - chrono::Duration::hours(1);
    due_row.updated_at = evaluated_at + chrono::Duration::minutes(2);
    for row in [&held_since_evaluation, &held_by_evaluation, &due_row] {
        store.create_candidate(row).await.expect("create");
    }

    let stale_before = now - chrono::Duration::hours(1);
    let ids = |rows: Vec<LifecycleCandidate>| -> Vec<String> {
        rows.into_iter().map(|row| row.id).collect()
    };
    let skipped = store
        .list_due_candidates("rule-a", now, stale_before, Some(evaluated_at), None, 10)
        .await
        .expect("list with a cutoff");
    assert_eq!(ids(skipped), vec!["cand-held-by-eval", "cand-due"]);

    // The next evaluation moves the cutoff past the hold, which returns it.
    let next_evaluation = evaluated_at + chrono::Duration::minutes(5);
    let released = store
        .list_due_candidates("rule-a", now, stale_before, Some(next_evaluation), None, 10)
        .await
        .expect("list after the next evaluation");
    assert_eq!(
        ids(released),
        vec!["cand-held-by-eval", "cand-held-later", "cand-due"]
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn due_selection_pages_past_a_cursor_in_due_then_id_order() {
    let (services, db) = temp_services("scryer_lifecycle_due_cursor").await;
    seed_rule_set(&services, "rule-a").await;
    let store = evaluation_store(&services);
    let now = Utc::now();
    let tied = now - chrono::Duration::hours(2);
    // Two rows share a due time, so the id breaks the tie across a page.
    for (id, title, due_at) in [
        ("cand-c", "title-3", now - chrono::Duration::hours(1)),
        ("cand-b", "title-2", tied),
        ("cand-a", "title-1", tied),
        ("cand-d", "title-4", now - chrono::Duration::minutes(5)),
    ] {
        let mut row = candidate(id, "rule-a", title, 1);
        row.due_at = due_at;
        store.create_candidate(&row).await.expect("create due");
    }

    let stale_before = now - chrono::Duration::hours(1);
    let mut walked = Vec::new();
    let mut cursor: Option<scryer_application::LifecycleDueCursor> = None;
    loop {
        let page = store
            .list_due_candidates("rule-a", now, stale_before, None, cursor.as_ref(), 2)
            .await
            .expect("list a page");
        let Some(last) = page.last() else { break };
        cursor = Some(scryer_application::LifecycleDueCursor::after(last));
        walked.extend(page.iter().map(|row| row.id.clone()));
        if page.len() < 2 {
            break;
        }
    }
    assert_eq!(walked, vec!["cand-a", "cand-b", "cand-c", "cand-d"]);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn due_selection_returns_an_abandoned_lease_but_not_a_live_one() {
    let (services, db) = temp_services("scryer_lifecycle_due_stale_lease").await;
    seed_rule_set(&services, "rule-a").await;
    let store = evaluation_store(&services);
    let now = Utc::now();

    let mut row = candidate("cand-leased", "rule-a", "title-1", 1);
    row.state = LifecycleCandidateState::Due;
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
        .list_due_candidates(
            "rule-a",
            now,
            now - chrono::Duration::hours(1),
            None,
            None,
            10,
        )
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
        .list_due_candidates(
            "rule-a",
            later,
            later - chrono::Duration::hours(1),
            None,
            None,
            10,
        )
        .await
        .expect("list with a stale lease");
    let ids: Vec<&str> = stranded.iter().map(|row| row.id.as_str()).collect();
    assert_eq!(ids, vec!["cand-leased"]);
    assert_eq!(stranded[0].state, LifecycleCandidateState::Executing);

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
    let (services, db) = temp_services("scryer_lifecycle_transition_cas").await;
    seed_rule_set(&services, "rule-a").await;
    let store = evaluation_store(&services);
    let now = Utc::now();

    let mut row = candidate("cand-1", "rule-a", "title-1", 1);
    row.state = LifecycleCandidateState::Due;
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
                LifecycleCandidateState::Canceled,
                "no_match",
                &[
                    LifecycleCandidateState::Observing,
                    LifecycleCandidateState::Blocked,
                ],
                now,
            )
            .await
            .expect("compare-and-set"),
        "a transition whose expectation does not hold must not write"
    );
    let unchanged = store
        .get_active_subject_candidate("rule-a", "title", "title-1")
        .await
        .expect("read")
        .expect("exists");
    assert_eq!(unchanged.state, LifecycleCandidateState::Executing);
    assert_eq!(unchanged.state_reason, "execution_leased");

    // The lease holder's own write names `executing`, and lands.
    assert!(
        store
            .transition_candidate_state(
                "cand-1",
                LifecycleCandidateState::Succeeded,
                "action_succeeded",
                &[LifecycleCandidateState::Executing],
                now,
            )
            .await
            .expect("compare-and-set")
    );

    let refused = store
        .transition_candidate_state(
            "cand-1",
            LifecycleCandidateState::Canceled,
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
    let (services, db) = temp_services("scryer_lifecycle_sequence_finish_lease_cas").await;
    seed_rule_set(&services, "rule-a").await;
    let store = evaluation_store(&services);
    let mut row = candidate("sequence-finish-lease", "rule-a", "title-lease", 1);
    row.action_kind = "action_sequence".to_string();
    row.state = LifecycleCandidateState::Due;
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
                LifecycleCandidateState::Blocked,
                "unknown_at_execution",
                replacement_lease_at + chrono::Duration::seconds(1),
            )
            .await
            .expect("stale sequence finisher is a compare-and-set miss"),
        "the first worker must not overwrite the newer executing lease"
    );
    let current = store
        .get_active_subject_candidate("rule-a", "title", "title-lease")
        .await
        .expect("reload candidate")
        .expect("candidate remains active");
    assert_eq!(current.state, LifecycleCandidateState::Executing);
    assert_eq!(current.updated_at, replacement_lease_at);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn action_runs_append_finish_and_enforce_attempt_uniqueness() {
    let (services, db) = temp_services("scryer_lifecycle_action_runs").await;
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
        cause_code: None,
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
async fn lifecycle_scoped_deletion_checkpoint_survives_unbounded_safety_holds() {
    let (services, db) = temp_services("scryer_lifecycle_checkpoint_holds").await;
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
        cause_code: None,
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

/// Retention removes only held runs past the window that are neither the
/// newest of their candidate generation nor a resume journal, and a resume
/// after the prune still finds the journal it would have found before.
#[tokio::test]
async fn housekeeping_prunes_old_held_action_runs_but_keeps_journals_and_the_newest() {
    use scryer_application::HousekeepingRepository;
    use scryer_domain::LifecycleActionRunStatus as Status;

    let (services, db) = temp_services("scryer_lifecycle_held_run_retention").await;
    seed_rule_set(&services, "rule-a").await;
    let store = evaluation_store(&services);
    for (id, title) in [
        ("cand-emberfall", "title-emberfall"),
        ("cand-cinderwake", "title-cinderwake"),
    ] {
        store
            .create_candidate(&candidate(id, "rule-a", title, 1))
            .await
            .expect("create candidate");
    }
    let kind = "unmonitor_scope_delete_files";
    let journal = r#"{"scoped_deletion":1,"files":[]}"#;
    let now = Utc::now();
    // (id, candidate, generation, status, detail, age in days, finished)
    let rows: &[(&str, &str, i64, Status, &str, i64, bool)] = &[
        // Emberfall generation 1: an old held journal row, not the newest.
        (
            "ember-journal-held",
            "cand-emberfall",
            1,
            Status::Held,
            journal,
            200,
            true,
        ),
        (
            "ember-held-old",
            "cand-emberfall",
            1,
            Status::Held,
            "{}",
            190,
            true,
        ),
        // The underscore of the journal key is literal: this is no journal.
        (
            "ember-held-lookalike",
            "cand-emberfall",
            1,
            Status::Held,
            r#"{"scopedXdeletion":1}"#,
            180,
            true,
        ),
        (
            "ember-failed-old",
            "cand-emberfall",
            1,
            Status::Failed,
            "{}",
            170,
            true,
        ),
        (
            "ember-held-unfinished",
            "cand-emberfall",
            1,
            Status::Held,
            "{}",
            160,
            false,
        ),
        (
            "ember-held-recent",
            "cand-emberfall",
            1,
            Status::Held,
            "{}",
            10,
            true,
        ),
        // Emberfall generation 2: every row is old and held; the newest stays,
        // and a tie on start time is broken by id as the resume read does.
        (
            "ember-gen2-held-a",
            "cand-emberfall",
            2,
            Status::Held,
            "{}",
            150,
            true,
        ),
        (
            "ember-gen2-tie-a",
            "cand-emberfall",
            2,
            Status::Held,
            "{}",
            100,
            true,
        ),
        (
            "ember-gen2-tie-b",
            "cand-emberfall",
            2,
            Status::Held,
            "{}",
            100,
            true,
        ),
        // Cinderwake: the journal row is the newest of its generation.
        (
            "cinder-held-old",
            "cand-cinderwake",
            1,
            Status::Held,
            "{}",
            140,
            true,
        ),
        (
            "cinder-journal-held",
            "cand-cinderwake",
            1,
            Status::Held,
            journal,
            130,
            true,
        ),
    ];
    for (id, candidate_id, generation, status, detail, age_days, finished) in rows {
        let at = now - chrono::Duration::days(*age_days);
        store
            .start_action_run(&scryer_domain::LifecycleActionRun {
                id: (*id).into(),
                candidate_id: (*candidate_id).into(),
                rule_set_id: "rule-a".into(),
                revision_number: 1,
                title_id: "title".into(),
                subject_kind: "episode".into(),
                subject_id: "episode-1".into(),
                action_kind: kind.into(),
                match_generation: *generation,
                idempotency_key: format!("hold:{id}"),
                attempt: 1,
                status: *status,
                hold_reason: (*status == Status::Held).then(|| "tag_conflict".into()),
                error: None,
                cause_code: None,
                detail: (*detail).into(),
                started_at: at,
                finished_at: finished.then_some(at),
                created_at: at,
            })
            .await
            .expect("seed action run");
    }
    let resume = |candidate_id: &'static str| {
        let store = &store;
        async move {
            store
                .latest_scoped_deletion_action_run(candidate_id, 1, kind)
                .await
                .expect("read journal")
                .map(|run| run.id)
        }
    };
    let ember_before = resume("cand-emberfall").await;
    let cinder_before = resume("cand-cinderwake").await;
    assert_eq!(ember_before.as_deref(), Some("ember-journal-held"));
    assert_eq!(cinder_before.as_deref(), Some("cinder-journal-held"));

    let housekeeping = housekeeping_store(&services);
    let deleted = housekeeping
        .delete_held_lifecycle_action_runs_older_than(90)
        .await
        .expect("prune held runs");
    assert_eq!(deleted, 5);

    let mut remaining: Vec<String> = sqlx::query_scalar("SELECT id FROM lifecycle_action_runs")
        .fetch_all(&services.pool)
        .await
        .expect("list remaining runs");
    remaining.sort();
    assert_eq!(
        remaining,
        vec![
            "cinder-journal-held",
            "ember-failed-old",
            "ember-gen2-tie-b",
            "ember-held-recent",
            "ember-held-unfinished",
            "ember-journal-held",
        ]
    );
    assert_eq!(resume("cand-emberfall").await, ember_before);
    assert_eq!(resume("cand-cinderwake").await, cinder_before);

    assert_eq!(
        housekeeping
            .delete_held_lifecycle_action_runs_older_than(90)
            .await
            .expect("second prune"),
        0,
        "a second pass finds nothing"
    );

    let _ = std::fs::remove_file(db);
}

/// The retention pass reads held runs through the finish-time index and
/// finds a newer run of the same generation through the generation index.
#[tokio::test]
async fn held_action_run_retention_is_index_served_on_sqlite() {
    let (services, db) = temp_services("scryer_lifecycle_held_run_retention_plan").await;
    let plan: Vec<(i64, i64, i64, String)> = sqlx::query_as(
        "EXPLAIN QUERY PLAN
         SELECT 1 AS present FROM lifecycle_action_runs
          WHERE status = ?1
            AND finished_at IS NOT NULL
            AND finished_at < ?2
            AND detail NOT LIKE ?3 ESCAPE '!'
            AND EXISTS (
                SELECT 1
                  FROM lifecycle_action_runs newer
                 WHERE newer.candidate_id = lifecycle_action_runs.candidate_id
                   AND newer.match_generation = lifecycle_action_runs.match_generation
                   AND (
                       newer.started_at > lifecycle_action_runs.started_at
                       OR (
                           newer.started_at = lifecycle_action_runs.started_at
                           AND newer.id > lifecycle_action_runs.id
                       )
                   )
            )
          LIMIT 1",
    )
    .bind("held")
    .bind("2026-01-01T00:00:00Z")
    .bind("%\"scoped!_deletion\":%")
    .fetch_all(&services.pool)
    .await
    .expect("explain retention probe");
    let details = plan
        .into_iter()
        .map(|(_, _, _, detail)| detail)
        .collect::<Vec<_>>()
        .join("\n");
    println!("{details}");
    assert!(
        details.contains("idx_lifecycle_action_runs_status_finished"),
        "{details}"
    );
    assert!(
        details.contains("idx_lifecycle_action_runs_generation_started"),
        "{details}"
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn deletion_confirmation_and_attempts_round_trip() {
    let (services, db) = temp_services("scryer_lifecycle_arming").await;
    seed_rule_set(&services, "rule-a").await;
    let rules = crate::LifecycleRuleSetStore::new(services.datastore());
    let store = evaluation_store(&services);
    let now = Utc::now();

    rules
        .update_rule_set_state(
            "rule-a",
            LifecycleRuleState::Enabled,
            Some(&scryer_domain::LifecycleRuleArmingConfirmation::from(
                &rules.get_rule_set("rule-a").await.unwrap().unwrap(),
            )),
            now,
        )
        .await
        .expect("arm");
    let armed = rules
        .get_rule_set("rule-a")
        .await
        .expect("read")
        .expect("exists");
    assert_eq!(armed.state, LifecycleRuleState::Enabled);
    assert_eq!(armed.destructive_confirmed_revision, Some(1));
    assert!(armed.destructive_confirmed());
    assert_eq!(
        armed.current_revision_number, 1,
        "arming never appends a revision"
    );

    let mut row = candidate("cand-1", "rule-a", "title-1", 1);
    row.action_attempts = 2;
    store.create_candidate(&row).await.expect("create");
    let stored = store
        .get_active_subject_candidate("rule-a", "title", "title-1")
        .await
        .expect("read")
        .expect("exists");
    assert_eq!(stored.action_attempts, 2);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn lifecycle_scope_candidates_and_exclusions_have_independent_subject_identity() {
    let (services, db) = temp_services("lifecycle_scope_identity").await;
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
        let exclusion = LifecycleRuleExclusion {
            id: format!("exclusion-{index}"),
            rule_set_id: None,
            title_id: "title-1".into(),
            subject_kind: LifecycleRuleSubjectKind::parse_storage(kind).unwrap(),
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
async fn action_sequence_migration_preserves_pre_0232_in_flight_legacy_rows() {
    let directory = tempfile::tempdir().unwrap();
    let db = directory.path().join("legacy.db");
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .unwrap();
    crate::migrations::replay_source_catalog_for_fresh_install(&pool, Some(231), true)
        .await
        .unwrap();
    run_embedded_migration(&pool, r#"
        INSERT INTO maintenance_rule_sets (id, name, enabled, evaluation_mode, effect_arming) VALUES ('rule', 'legacy', 1, 'observe', 'destructive');
        INSERT INTO lifecycle_candidates (id, rule_set_id, revision_number, title_id, subject_kind, subject_id, match_generation, first_matched_at, due_at, action_attempts) VALUES ('candidate', 'rule', 3, 'title', 'title', 'title', 7, '2026-01-01T00:00:00Z', '2026-02-01T00:00:00Z', 2);
        INSERT INTO maintenance_rule_exclusions (id, title_id, subject_kind, subject_id, reason) VALUES ('exclusion', 'title', 'title', 'title', 'keep');
        INSERT INTO lifecycle_action_runs (id, candidate_id, rule_set_id, revision_number, title_id, subject_kind, subject_id, action_kind, match_generation, idempotency_key, attempt, status, detail, started_at) VALUES ('run', 'candidate', 'rule', 3, 'title', 'title', 'title', 'unmonitor_scope_keep_files', 7, 'legacy-key', 2, 'failed', '{"evidence":true}', '2026-01-02T00:00:00Z');
    "#).await;
    // Every step up to the one that removes rules written before rules
    // declared their own actions; that step is checked at the end.
    crate::migrations::run_upgrade_migrations_through(&pool, 347)
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
    let rule_state: (bool, String, String) = sqlx::query_as(
        "SELECT enabled, evaluation_mode, effect_arming FROM maintenance_rule_sets WHERE id = 'rule'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(rule_state, (false, "disabled".into(), "none".into()));
    let exclusion: String = sqlx::query_scalar(
        "SELECT subject_id FROM maintenance_rule_exclusions WHERE id = 'exclusion'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(exclusion, "title");

    // The rest of the upgrade removes this user rule and its history; the
    // exclusion that names no rule is kept.
    crate::migrations::run_migrations(&pool, crate::types::MigrationMode::Apply)
        .await
        .unwrap();
    for (table, count) in [
        (
            "lifecycle_rule_sets",
            "SELECT COUNT(*) FROM lifecycle_rule_sets",
        ),
        (
            "lifecycle_candidates",
            "SELECT COUNT(*) FROM lifecycle_candidates",
        ),
        (
            "lifecycle_action_runs",
            "SELECT COUNT(*) FROM lifecycle_action_runs",
        ),
    ] {
        let remaining: i64 = sqlx::query_scalar(count).fetch_one(&pool).await.unwrap();
        assert_eq!(remaining, 0, "{table}");
    }
    let exclusion: String = sqlx::query_scalar(
        "SELECT subject_id FROM lifecycle_rule_exclusions WHERE id = 'exclusion'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(exclusion, "title");
    crate::migrations::run_migrations(&pool, crate::types::MigrationMode::ValidateOnly)
        .await
        .unwrap();
}

#[tokio::test]
async fn guarded_rule_deletion_cannot_erase_a_concurrently_created_candidate() {
    let (services, db) = temp_services("guarded_lifecycle_delete").await;
    let rules = crate::LifecycleRuleSetStore::new(services.datastore());
    let evaluation = evaluation_store(&services);
    for n in 0..8 {
        let rule_id = format!("race-{n}");
        seed_rule_set(&services, &rule_id).await;
        let row = candidate(&format!("candidate-{n}"), &rule_id, "title", 1);
        let (deleted, inserted) = tokio::join!(
            rules.delete_rule_set_if_unused(&rule_id),
            evaluation.create_candidate(&row)
        );
        assert_ne!(
            deleted.is_ok(),
            inserted.is_ok(),
            "one operation must win: {deleted:?} {inserted:?}"
        );
        if inserted.is_ok() {
            assert!(rules.get_rule_set(&rule_id).await.unwrap().is_some());
            assert_eq!(
                evaluation
                    .get_active_subject_candidate(&rule_id, "title", "title")
                    .await
                    .unwrap()
                    .unwrap()
                    .id,
                row.id
            );
            assert!(rules.delete_rule_set_if_unused(&rule_id).await.is_err());
        } else {
            assert!(rules.get_rule_set(&rule_id).await.unwrap().is_none());
        }
    }
    services.pool().close().await;
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn guarded_rule_deletion_preserves_action_history_even_for_terminal_candidates() {
    let (services, db) = temp_services("guarded_lifecycle_history").await;
    seed_rule_set(&services, "history-rule").await;
    let store = evaluation_store(&services);
    let mut row = candidate("history-candidate", "history-rule", "title", 1);
    row.state = LifecycleCandidateState::Succeeded;
    store.create_candidate(&row).await.unwrap();
    let now = Utc::now();
    let run = scryer_domain::LifecycleActionRun {
        id: "history-run".into(),
        candidate_id: row.id.clone(),
        rule_set_id: row.rule_set_id.clone(),
        revision_number: 1,
        title_id: row.title_id.clone(),
        subject_id: row.subject_id.clone(),
        subject_kind: row.subject_kind.clone(),
        action_kind: row.action_kind.clone(),
        match_generation: 1,
        idempotency_key: "history-key".into(),
        attempt: 1,
        status: scryer_domain::LifecycleActionRunStatus::Succeeded,
        hold_reason: None,
        error: None,
        cause_code: None,
        detail: "{}".into(),
        started_at: now,
        finished_at: Some(now),
        created_at: now,
    };
    store.start_action_run(&run).await.unwrap();
    let rules = crate::LifecycleRuleSetStore::new(services.datastore());
    assert!(
        rules
            .delete_rule_set_if_unused(&row.rule_set_id)
            .await
            .is_err()
    );
    assert_eq!(
        store
            .list_action_runs(Some(&row.rule_set_id), None, Some(10))
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        rules
            .get_rule_set(&row.rule_set_id)
            .await
            .unwrap()
            .is_some()
    );
    services.pool().close().await;
    let _ = std::fs::remove_file(db);
}

async fn seed_sequence_memberships(
    services: &SqliteServices,
    at: chrono::DateTime<Utc>,
) -> crate::LifecycleEvaluationStore {
    seed_rule_set(services, "rule-a").await;
    let store = evaluation_store(services);
    // (subject, created offset in seconds, already released)
    let rows = [
        ("subject-active", 0, false),
        ("subject-released", 0, true),
        ("subject-created-later", 60, false),
    ];
    for (index, (subject, created_offset, released)) in rows.into_iter().enumerate() {
        let candidate = candidate(&format!("candidate-{index}"), "rule-a", subject, 1);
        store.create_candidate(&candidate).await.expect("candidate");
        let created_at = at + chrono::Duration::seconds(created_offset);
        sqlx::query(
            "INSERT INTO lifecycle_sequence_terminal_memberships
             (id, candidate_id, rule_set_id, revision_number, source_content_hash, title_id,
              subject_kind, subject_id, match_generation, sequence_content_hash, outcome,
              created_at, released_at)
             VALUES (?, ?, 'rule-a', 1, 'hash-1', ?, 'title', ?, 1, 'sequence-hash',
                     'succeeded', ?, ?)",
        )
        .bind(format!("membership-{index}"))
        .bind(&candidate.id)
        .bind(subject)
        .bind(subject)
        .bind(created_at.to_rfc3339())
        .bind(released.then(|| created_at.to_rfc3339()))
        .execute(services.pool())
        .await
        .expect("membership");
    }
    store
}

async fn membership_rows(services: &SqliteServices) -> Vec<(String, Option<String>)> {
    sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT id, released_at FROM lifecycle_sequence_terminal_memberships ORDER BY id",
    )
    .fetch_all(services.pool())
    .await
    .expect("memberships")
}

/// Releasing on a confirmed non-match answers and changes exactly what the
/// unconditional update did, and a subject with nothing to release never
/// waits on the writer.
#[tokio::test]
async fn confirmed_non_match_release_matches_the_unconditional_update() {
    let at = Utc::now();
    let released_at = at + chrono::Duration::seconds(30);
    let (old_path, old_db) = temp_services("scryer_sequence_release_unconditional").await;
    let (new_path, new_db) = temp_services("scryer_sequence_release_probed").await;
    seed_sequence_memberships(&old_path, at).await;
    let store = seed_sequence_memberships(&new_path, at).await;

    for subject in [
        "subject-active",
        "subject-released",
        "subject-created-later",
        "subject-without-membership",
    ] {
        let unconditional = sqlx::query(
            "UPDATE lifecycle_sequence_terminal_memberships
                SET released_at = ?
              WHERE rule_set_id = 'rule-a' AND revision_number = 1 AND subject_kind = 'title'
                AND subject_id = ? AND released_at IS NULL AND created_at <= ?",
        )
        .bind(released_at.to_rfc3339())
        .bind(subject)
        .bind(released_at.to_rfc3339())
        .execute(old_path.pool())
        .await
        .expect("unconditional release")
        .rows_affected()
            == 1;
        let probed = store
            .release_sequence_completion_on_confirmed_non_match(
                "rule-a",
                1,
                "title",
                subject,
                released_at,
            )
            .await
            .expect("probed release");
        assert_eq!(probed, unconditional, "{subject}");
        assert_eq!(probed, subject == "subject-active", "{subject}");
    }
    assert_eq!(
        membership_rows(&new_path).await,
        membership_rows(&old_path).await
    );

    let writer_gate = new_path.writer_gate();
    let _held = writer_gate.lock().await;
    let idle = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        store.release_sequence_completion_on_confirmed_non_match(
            "rule-a",
            1,
            "title",
            "subject-without-membership",
            released_at,
        ),
    )
    .await
    .expect("a subject with nothing to release does not wait on the writer")
    .expect("probed release");
    assert!(!idle);
    drop(_held);

    old_path.pool().close().await;
    new_path.pool().close().await;
    let _ = std::fs::remove_file(old_db);
    let _ = std::fs::remove_file(new_db);
}

fn evaluation_run(
    id: &str,
    rule_set_id: &str,
    started_at: chrono::DateTime<Utc>,
) -> LifecycleEvaluationRun {
    LifecycleEvaluationRun {
        id: id.to_string(),
        rule_set_id: rule_set_id.to_string(),
        revision_number: 1,
        source_content_hash: "hash-1".to_string(),
        started_at,
        finished_at: None,
        status: LifecycleEvaluationRunStatus::Running,
        evaluated_count: 0,
        matched_count: 0,
        no_match_count: 0,
        unknown_count: 0,
        error_count: 0,
        canceled_candidates: 0,
        superseded_candidates: 0,
        duration_ms: None,
        error: None,
    }
}

#[tokio::test]
async fn schedule_anchors_carry_each_rules_latest_run_and_interrupted_sequences() {
    let (services, db) = temp_services("scryer_lifecycle_schedule_anchors").await;
    for id in ["rule-a", "rule-b", "rule-c"] {
        seed_rule_set(&services, id).await;
    }
    let store = evaluation_store(&services);
    let now = Utc::now();
    let latest = now - chrono::Duration::minutes(5);
    for (id, rule, at) in [
        ("run-1", "rule-a", now - chrono::Duration::hours(3)),
        ("run-2", "rule-a", latest),
        ("run-3", "rule-c", now - chrono::Duration::hours(1)),
    ] {
        store
            .start_evaluation_run(&evaluation_run(id, rule, at))
            .await
            .expect("start run");
    }
    let mut row = candidate("cand-b", "rule-b", "title-1", 1);
    row.state = LifecycleCandidateState::Due;
    store.create_candidate(&row).await.expect("create");
    assert!(
        store
            .lease_candidate_for_execution("cand-b", now - chrono::Duration::hours(1), now)
            .await
            .expect("lease")
    );

    let anchors = store
        .rule_schedule_anchors(&["rule-a".to_string(), "rule-b".to_string()])
        .await
        .expect("anchors");
    assert_eq!(anchors.len(), 2, "{anchors:?}");
    let a = anchors["rule-a"];
    assert_eq!(
        a.last_run_at.map(|at| at.timestamp_millis()),
        Some(latest.timestamp_millis())
    );
    assert!(!a.in_flight);
    let b = anchors["rule-b"];
    assert_eq!(b.last_run_at, None);
    assert!(b.in_flight);
    assert!(
        !anchors.contains_key("rule-c"),
        "a rule not asked for is not read"
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn step_acts_list_succeeded_attempts_of_those_kinds_within_the_window_only() {
    let (services, db) = temp_services("scryer_lifecycle_step_acts_window").await;
    seed_rule_set(&services, "rule-a").await;
    let store = evaluation_store(&services);
    let leased_at = Utc::now();
    let window_start = leased_at - chrono::Duration::minutes(1);

    let claim = async |candidate_id: &str, step_kind: &str| {
        let row = candidate(candidate_id, "rule-a", candidate_id, 1);
        store
            .create_candidate(&row)
            .await
            .expect("create candidate");
        let mut step = sequence_step_run(&row, 0);
        step.step_kind = step_kind.to_string();
        let mut identity: serde_json::Value =
            serde_json::from_str(&step.target_identity_json).expect("identity");
        identity["step_kind"] = serde_json::Value::from(step_kind);
        step.target_identity_json = identity.to_string();
        let lease = format!("lease-{candidate_id}");
        match store
            .claim_action_step(&step, window_start, &lease, leased_at)
            .await
            .expect("claim")
        {
            scryer_application::LifecycleActionStepClaim::Claimed(step) => (step, lease),
            other => panic!("unexpected step claim: {other:?}"),
        }
    };
    let finish = async |step: &scryer_domain::LifecycleActionStepRun,
                        lease: &str,
                        state: LifecycleActionStepState| {
        let mut finished = step.clone();
        finished.state = state;
        if state == LifecycleActionStepState::Failed {
            finished.error = Some("fixture failure".to_string());
        }
        finished.updated_at = leased_at + chrono::Duration::seconds(1);
        finished.finished_at = Some(finished.updated_at);
        assert!(
            store
                .finish_action_step(&finished, lease)
                .await
                .expect("finish")
        );
    };
    let (unmonitored, unmonitor_lease) = claim("title-unmonitor", "unmonitor").await;
    finish(
        &unmonitored,
        &unmonitor_lease,
        LifecycleActionStepState::Succeeded,
    )
    .await;
    let (monitor_failed, monitor_lease) = claim("title-monitor-failed", "monitor").await;
    finish(
        &monitor_failed,
        &monitor_lease,
        LifecycleActionStepState::Failed,
    )
    .await;
    let (_running, _) = claim("title-monitor-running", "monitor").await;
    let (searched, search_lease) = claim("title-search", "search").await;
    finish(
        &searched,
        &search_lease,
        LifecycleActionStepState::Succeeded,
    )
    .await;

    let kinds = ["monitor", "unmonitor"];
    let acts = store
        .list_step_acts_since(&kinds, window_start, 100)
        .await
        .expect("list acts");
    assert_eq!(
        acts,
        vec![scryer_application::LifecycleStepAct {
            rule_set_id: "rule-a".to_string(),
            title_id: "title-unmonitor".to_string(),
            step_kind: "unmonitor".to_string(),
        }],
        "only succeeded attempts of the named kinds are acts"
    );
    let later = store
        .list_step_acts_since(&kinds, leased_at + chrono::Duration::minutes(1), 100)
        .await
        .expect("list acts");
    assert!(
        later.is_empty(),
        "attempts before the window are not listed"
    );
    assert!(
        store
            .list_step_acts_since(&kinds, window_start, 0)
            .await
            .expect("list acts")
            .is_empty()
    );

    let _ = std::fs::remove_file(db);
}

//! Store round-trips for the maintenance evaluator's three tables, including
//! the two invariants that live in the schema rather than in Rust: one active
//! candidate per (rule, title), and one global exclusion per title.

use super::*;
use scryer_application::{
    MaintenanceCandidateQuery, MaintenanceCandidateRepository, MaintenanceEvaluationRunRepository,
    MaintenanceExclusionRepository, MaintenanceRuleSetRepository,
};
use scryer_domain::{
    LifecycleCandidate, MaintenanceCandidateState, MaintenanceEvaluationMode,
    MaintenanceEvaluationRun, MaintenanceEvaluationRunStatus, MaintenanceRuleExclusion,
    MaintenanceRuleRevision, MaintenanceRuleSet, MaintenanceRuleSubjectKind,
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
        created_at: now,
        updated_at: now,
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

//! The action executor's gate matrix, arming contract, safety rechecks, and
//! postconditions (RFC 137 sections 8, 9.8, 9.10; tracks D2/D3).
//!
//! Every test that makes an action execute also asserts the evidence trail:
//! the action-run row, the candidate's terminal state, and its reason. The
//! deletion journey itself lives in the integration suite, where the real
//! delete path and stores exist; here the mock harness proves the decisions
//! around execution.

use super::*;

use crate::lib_tests::maintenance_evaluation::InMemoryMaintenanceEvaluationRepo;
use crate::location::ownership_guard::OwnedEntity;
use crate::location::test_support::InMemoryLocationOperationStore;
use crate::lib_tests::maintenance_rules::{InMemoryMaintenanceRuleRepo, MaintenanceRuleReadFault};
use crate::maintenance_rules::{
    MAINTENANCE_MAX_ACTION_ATTEMPTS, MaintenanceActionKind, MaintenanceActionSpec,
    MaintenanceGatesUpdate, MaintenanceMatcherDraft, MaintenanceRuleDraft, execution_reason,
};
use crate::ports::{
    ConnectionPlaybackActivity, MediaServerPlaybackProbe, PlaybackActivitySnapshot,
    PlaybackProbeStatus,
};
use scryer_domain::{
    MaintenanceCandidateState, MaintenanceEffectArming, MaintenanceEvaluationMode,
    MediaServerProvider,
};

/// A playback probe whose answer the test chooses.
struct FixedPlaybackProbe {
    status: PlaybackProbeStatus,
}

#[async_trait]
impl MediaServerPlaybackProbe for FixedPlaybackProbe {
    async fn active_playback(&self) -> AppResult<PlaybackActivitySnapshot> {
        Ok(PlaybackActivitySnapshot {
            connections: vec![ConnectionPlaybackActivity {
                connection_id: "conn-1".to_string(),
                provider: MediaServerProvider::Jellyfin,
                status: self.status.clone(),
            }],
            observed_at: Utc::now(),
        })
    }
}

struct ExecutionFixture {
    app: AppUseCase,
    user: User,
    rules: Arc<InMemoryMaintenanceRuleRepo>,
    evaluation: Arc<InMemoryMaintenanceEvaluationRepo>,
}

fn execution_app(playback: Option<PlaybackProbeStatus>) -> ExecutionFixture {
    let (app, user) = bootstrap();
    let rules = Arc::new(InMemoryMaintenanceRuleRepo::default());
    let evaluation = Arc::new(InMemoryMaintenanceEvaluationRepo::default());
    let media_files = Arc::new(MockMediaFileRepo::default());
    let app = app.with_test_overrides(|services| {
        let services = services
            .with_maintenance_rule_set_store(rules.clone())
            .with_maintenance_evaluation_store(evaluation.clone())
            .with_media_files(media_files);
        match playback {
            Some(status) => {
                services.with_media_server_playback_probe(Arc::new(FixedPlaybackProbe { status }))
            }
            None => services,
        }
    });
    ExecutionFixture {
        app,
        user,
        rules,
        evaluation,
    }
}

const MONITORED_MATCHER: &str = "match if {\n\
     \tinput.facts.monitored\n\
     }\n";

const ALWAYS_MATCHER: &str = "match := true\n";

fn unmonitor_draft(rego_source: &str) -> MaintenanceRuleDraft {
    MaintenanceRuleDraft {
        name: "Unmonitor stale".to_string(),
        description: String::new(),
        rego_source: rego_source.to_string(),
        action_spec: MaintenanceActionSpec::new(MaintenanceActionKind::UnmonitorScopeKeepFiles),
        grace_days: 0,
        library_ids: Vec::new(),
        evaluation_mode: None,
    }
}

fn delete_draft() -> MaintenanceRuleDraft {
    MaintenanceRuleDraft {
        name: "Retire".to_string(),
        description: String::new(),
        rego_source: ALWAYS_MATCHER.to_string(),
        action_spec: MaintenanceActionSpec::new(MaintenanceActionKind::DeleteTitleAndFiles),
        grace_days: 0,
        library_ids: Vec::new(),
        evaluation_mode: None,
    }
}

async fn seed_title(app: &AppUseCase, user: &User, name: &str, monitored: bool) -> Title {
    app.add_title(
        user,
        NewTitle {
            name: name.to_string(),
            facet: MediaFacet::Movie,
            monitored,
            tags: vec![],
            external_ids: vec![],
            ..Default::default()
        },
    )
    .await
    .expect("create title")
}

impl ExecutionFixture {
    async fn observed_rule(&self, draft: MaintenanceRuleDraft) -> String {
        let created = self
            .app
            .create_maintenance_rule_set(&self.user, draft)
            .await
            .expect("create rule set");
        self.app
            .set_maintenance_rule_evaluation_mode(
                &self.user,
                &created.rule_set.id,
                MaintenanceEvaluationMode::Observe,
            )
            .await
            .expect("observe rule");
        created.rule_set.id
    }

    async fn open_gates(&self, reversible: bool, destructive: bool) {
        self.app
            .set_maintenance_instance_gates(
                &self.user,
                MaintenanceGatesUpdate {
                    evaluation_enabled: Some(true),
                    result_display_enabled: Some(true),
                    reversible_effects_enabled: Some(reversible),
                    destructive_effects_enabled: Some(destructive),
                    ..Default::default()
                },
            )
            .await
            .expect("arm gates");
    }

    async fn arm(&self, rule_set_id: &str, arming: MaintenanceEffectArming, ack: Option<i64>) {
        self.app
            .set_maintenance_rule_arming(&self.user, rule_set_id, arming, ack)
            .await
            .expect("arm rule");
    }

    async fn evaluate(&self) {
        self.app
            .run_maintenance_rule_evaluation_job()
            .await
            .expect("evaluation pass");
    }

    async fn handle(&self) -> crate::maintenance_rules::MaintenanceActionHandlingReport {
        self.app
            .run_lifecycle_action_handling_job()
            .await
            .expect("handler pass")
    }

    async fn only_candidate(&self) -> scryer_domain::LifecycleCandidate {
        let candidates = self.evaluation.all_candidates().await;
        assert_eq!(candidates.len(), 1, "{candidates:?}");
        candidates[0].clone()
    }
}

// ── Arming survives nothing it did not authorize (Fix 1) ────────────────────

#[tokio::test]
async fn replacing_the_matcher_disarms_the_rule() {
    let fixture = execution_app(None);
    let rule_id = fixture.observed_rule(delete_draft()).await;
    fixture.open_gates(true, true).await;
    seed_title(&fixture.app, &fixture.user, "Doomed", true).await;
    fixture.evaluate().await;
    fixture
        .arm(&rule_id, MaintenanceEffectArming::Destructive, Some(1))
        .await;

    // The operator armed *this* matcher after acknowledging what it would
    // delete. Swapping the matcher out needs only catalog-settings authority,
    // so if the arming survived, a different rule would run destructively under
    // an acknowledgement nobody gave for it.
    let replaced = fixture
        .app
        .update_maintenance_rule_matcher(
            &fixture.user,
            &rule_id,
            MaintenanceMatcherDraft {
                rego_source: ALWAYS_MATCHER.to_string(),
                action_spec: MaintenanceActionSpec::new(MaintenanceActionKind::DeleteTitleAndFiles),
                grace_days: 0,
            },
        )
        .await
        .expect("replace the matcher");
    assert_eq!(
        replaced.rule_set.effect_arming,
        MaintenanceEffectArming::None,
        "the returned detail must report the disarm the store just wrote"
    );

    let stored = fixture
        .app
        .get_maintenance_rule_set(&fixture.user, &rule_id)
        .await
        .expect("read rule")
        .expect("rule exists");
    assert_eq!(
        stored.rule_set.effect_arming,
        MaintenanceEffectArming::None,
        "the disarm must be persisted, not just reported"
    );

    // And it is a real refusal, not just a field: the next pass finds no
    // eligible rule at all.
    fixture.evaluate().await;
    let report = fixture.handle().await;
    assert_eq!(report.rules_eligible, 0, "{report:?}");
    assert!(
        fixture.evaluation.all_action_runs().await.is_empty(),
        "a disarmed rule must not reach an action run"
    );

    // Re-arming against the new matcher is what puts it back in play.
    let pending = fixture
        .app
        .count_active_maintenance_candidates(&rule_id)
        .await
        .expect("count candidates");
    fixture
        .arm(
            &rule_id,
            MaintenanceEffectArming::Destructive,
            Some(pending),
        )
        .await;
    let rearmed = fixture.handle().await;
    assert_eq!(rearmed.rules_eligible, 1, "{rearmed:?}");
}

#[tokio::test]
async fn renaming_a_rule_leaves_its_arming_alone() {
    let fixture = execution_app(None);
    let rule_id = fixture.observed_rule(delete_draft()).await;
    fixture.open_gates(true, true).await;
    seed_title(&fixture.app, &fixture.user, "Doomed", true).await;
    fixture.evaluate().await;
    fixture
        .arm(&rule_id, MaintenanceEffectArming::Destructive, Some(1))
        .await;

    // A rename moves neither the matcher nor the action, so the operator's
    // acknowledgement still describes exactly the same blast radius.
    fixture
        .app
        .update_maintenance_rule_metadata(
            &fixture.user,
            &rule_id,
            "Retire, renamed".to_string(),
            "Same matcher".to_string(),
            Vec::new(),
        )
        .await
        .expect("rename");

    let stored = fixture
        .app
        .get_maintenance_rule_set(&fixture.user, &rule_id)
        .await
        .expect("read rule")
        .expect("rule exists");
    assert_eq!(stored.rule_set.name, "Retire, renamed");
    assert_eq!(
        stored.rule_set.effect_arming,
        MaintenanceEffectArming::Destructive,
        "a rename must not disarm a rule"
    );
    assert_eq!(
        stored.rule_set.current_revision_number, 1,
        "a rename appends no revision, which is why it does not disarm"
    );
}

// ── Stranded leases are recoverable (Fix 3) ─────────────────────────────────

#[tokio::test]
async fn a_stranded_execution_lease_is_reclaimed_and_retried_at_the_next_attempt() {
    let fixture = execution_app(None);
    let rule_id = fixture
        .observed_rule(unmonitor_draft(MONITORED_MATCHER))
        .await;
    fixture.open_gates(true, false).await;
    fixture
        .arm(&rule_id, MaintenanceEffectArming::Reversible, None)
        .await;
    let title = seed_title(&fixture.app, &fixture.user, "Interrupted", true).await;
    fixture.evaluate().await;

    // Attempt one reserved its number and inserted its `running` row, then its
    // worker died: the candidate is `executing` with nobody driving it and the
    // run row never finished.
    let candidate = fixture.only_candidate().await;
    fixture
        .evaluation
        .record_candidate_attempts(&candidate.id, 1, Utc::now())
        .await
        .expect("reserve the interrupted attempt");
    fixture
        .evaluation
        .start_action_run(&scryer_domain::LifecycleActionRun {
            id: "run-interrupted".to_string(),
            candidate_id: candidate.id.clone(),
            rule_set_id: rule_id.clone(),
            revision_number: candidate.revision_number,
            title_id: title.id.clone(),
            action_kind: candidate.action_kind.clone(),
            match_generation: candidate.match_generation,
            idempotency_key: "orphan-key".to_string(),
            attempt: 1,
            status: scryer_domain::LifecycleActionRunStatus::Running,
            hold_reason: None,
            error: None,
            detail: "{}".to_string(),
            started_at: Utc::now(),
            finished_at: None,
            created_at: Utc::now(),
        })
        .await
        .expect("the interrupted attempt's run row");
    fixture
        .evaluation
        .strand_as_executing(&candidate.id, Utc::now() - chrono::Duration::hours(4))
        .await;

    let report = fixture.handle().await;
    assert_eq!(
        report.candidates_considered, 1,
        "a stranded lease must be selectable again: {report:?}"
    );
    assert_eq!(report.executed, 1, "{report:?}");

    let stored = fixture
        .app
        .get_title(&fixture.user, &title.id)
        .await
        .expect("read title")
        .expect("title exists");
    assert!(!stored.monitored, "the reclaimed attempt must do the work");

    let runs = fixture.evaluation.all_action_runs().await;
    let orphan = runs
        .iter()
        .find(|run| run.id == "run-interrupted")
        .expect("the orphaned run row is still there");
    assert_eq!(
        orphan.status,
        scryer_domain::LifecycleActionRunStatus::Failed,
        "an abandoned attempt must not keep reading as live work"
    );
    assert!(
        orphan
            .error
            .as_deref()
            .is_some_and(|error| error.contains(execution_reason::LEASE_RECLAIMED)),
        "{orphan:?}"
    );

    let fresh = runs
        .iter()
        .find(|run| run.id != "run-interrupted")
        .expect("the reclaiming attempt wrote its own row");
    assert_eq!(
        fresh.attempt, 2,
        "the reclaim must take the next attempt number, not collide with the orphan's"
    );
    assert_eq!(
        fresh.status,
        scryer_domain::LifecycleActionRunStatus::Succeeded
    );
    assert_eq!(
        fixture.only_candidate().await.state,
        MaintenanceCandidateState::Succeeded
    );
}

#[tokio::test]
async fn a_reclaimed_candidate_whose_title_is_gone_cancels_instead_of_acting() {
    let fixture = execution_app(None);
    let rule_id = fixture
        .observed_rule(unmonitor_draft(MONITORED_MATCHER))
        .await;
    fixture.open_gates(true, false).await;
    fixture
        .arm(&rule_id, MaintenanceEffectArming::Reversible, None)
        .await;
    let title = seed_title(&fixture.app, &fixture.user, "Vanished", true).await;
    fixture.evaluate().await;

    let candidate = fixture.only_candidate().await;
    fixture
        .evaluation
        .strand_as_executing(&candidate.id, Utc::now() - chrono::Duration::hours(4))
        .await;
    // The subject was deleted while the lease was stranded. Reclaiming does not
    // resume a decision made hours ago: the ordinary safety chain re-runs first,
    // and a missing subject cancels there.
    fixture
        .app
        .delete_title(&fixture.user, &title.id, false, None)
        .await
        .expect("delete the title");

    let report = fixture.handle().await;
    assert_eq!(report.canceled, 1, "{report:?}");
    assert_eq!(report.executed, 0);

    let candidate = fixture.only_candidate().await;
    assert_eq!(candidate.state, MaintenanceCandidateState::Canceled);
    assert_eq!(candidate.state_reason, execution_reason::TITLE_MISSING);
}

#[tokio::test]
async fn a_candidate_that_crash_looped_through_its_attempts_terminates() {
    let fixture = execution_app(None);
    let rule_id = fixture
        .observed_rule(unmonitor_draft(MONITORED_MATCHER))
        .await;
    fixture.open_gates(true, false).await;
    fixture
        .arm(&rule_id, MaintenanceEffectArming::Reversible, None)
        .await;
    let title = seed_title(&fixture.app, &fixture.user, "Doomed Loop", true).await;
    fixture.evaluate().await;

    // Every attempt in the budget was reserved and none of them reported back —
    // the shape a crash loop leaves. Counting reservations is what lets the cap
    // bite here at all; a counter only advanced on a handled failure would retry
    // this row forever.
    let candidate = fixture.only_candidate().await;
    fixture
        .evaluation
        .record_candidate_attempts(&candidate.id, MAINTENANCE_MAX_ACTION_ATTEMPTS, Utc::now())
        .await
        .expect("reserve the whole budget");
    fixture
        .evaluation
        .strand_as_executing(&candidate.id, Utc::now() - chrono::Duration::hours(4))
        .await;

    let report = fixture.handle().await;
    assert_eq!(report.failed, 1, "{report:?}");
    assert_eq!(report.executed, 0);

    let candidate = fixture.only_candidate().await;
    assert_eq!(candidate.state, MaintenanceCandidateState::Failed);
    assert_eq!(candidate.state_reason, execution_reason::ACTION_FAILED);

    let stored = fixture
        .app
        .get_title(&fixture.user, &title.id)
        .await
        .expect("read title")
        .expect("title exists");
    assert!(
        stored.monitored,
        "an exhausted candidate must not get one more go at the action"
    );
}

// ── Concurrent writers cannot clobber each other (Fix 4) ────────────────────

#[tokio::test]
async fn the_evaluator_leaves_a_candidate_under_an_execution_lease_alone() {
    let fixture = execution_app(None);
    let rule_id = fixture
        .observed_rule(unmonitor_draft(MONITORED_MATCHER))
        .await;
    fixture.open_gates(true, false).await;
    fixture
        .arm(&rule_id, MaintenanceEffectArming::Reversible, None)
        .await;
    let title = seed_title(&fixture.app, &fixture.user, "Leased", true).await;
    fixture.evaluate().await;

    let candidate = fixture.only_candidate().await;
    let leased_at = Utc::now();
    fixture
        .evaluation
        .strand_as_executing(&candidate.id, leased_at)
        .await;

    // The subject stops matching while the executor holds the lease. The
    // evaluator runs on its own schedule and would ordinarily cancel here —
    // straight over a row the executor is about to write a result to.
    fixture
        .app
        .set_title_monitored(&fixture.user, &title.id, false)
        .await
        .expect("manual unmonitor");
    let report = fixture
        .app
        .run_maintenance_rule_evaluation_job()
        .await
        .expect("evaluation pass");

    assert_eq!(
        report.candidates_canceled, 0,
        "an executing candidate is the executor's to decide: {report:?}"
    );
    assert_eq!(
        report.candidates_held, 0,
        "not held either — the pass skips the subject whole"
    );

    let after = fixture.only_candidate().await;
    assert_eq!(after.state, MaintenanceCandidateState::Executing);
    assert_eq!(after.updated_at, leased_at, "the lease was not touched");
}

#[tokio::test]
async fn a_terminal_write_after_the_lease_moved_reports_lease_lost() {
    let fixture = execution_app(None);
    let rule_id = fixture
        .observed_rule(unmonitor_draft(MONITORED_MATCHER))
        .await;
    fixture.open_gates(true, false).await;
    fixture
        .arm(&rule_id, MaintenanceEffectArming::Reversible, None)
        .await;
    let title = seed_title(&fixture.app, &fixture.user, "Contested", true).await;
    fixture.evaluate().await;

    // Another pass declares this lease stale and takes the candidate in the
    // window between the action finishing and its result being recorded.
    let candidate = fixture.only_candidate().await;
    fixture
        .evaluation
        .steal_lease_when_the_next_run_finishes(&candidate.id)
        .await;

    let report = fixture.handle().await;
    assert_eq!(report.lease_lost, 1, "{report:?}");
    assert_eq!(
        report.executed, 0,
        "a worker that lost its lease must not claim the outcome"
    );

    let after = fixture.only_candidate().await;
    assert_eq!(
        after.state,
        MaintenanceCandidateState::Due,
        "the new owner's state stands; the losing worker wrote nothing over it"
    );
    assert_ne!(after.state_reason, execution_reason::ACTION_SUCCEEDED);

    // The action itself did happen, and its run row still says so: that is
    // honest evidence of what this worker did, whoever owns the candidate now.
    let stored = fixture
        .app
        .get_title(&fixture.user, &title.id)
        .await
        .expect("read title")
        .expect("title exists");
    assert!(!stored.monitored);
    let runs = fixture.evaluation.all_action_runs().await;
    assert_eq!(runs.len(), 1);
    assert_eq!(
        runs[0].status,
        scryer_domain::LifecycleActionRunStatus::Succeeded
    );
}

// ── A transient rule read holds; only a deleted rule cancels (Fix 5) ────────

#[tokio::test]
async fn a_rule_that_is_actually_gone_cancels_its_candidate() {
    let fixture = execution_app(None);
    let rule_id = fixture
        .observed_rule(unmonitor_draft(MONITORED_MATCHER))
        .await;
    fixture.open_gates(true, false).await;
    fixture
        .arm(&rule_id, MaintenanceEffectArming::Reversible, None)
        .await;
    seed_title(&fixture.app, &fixture.user, "Orphaned", true).await;
    fixture.evaluate().await;

    fixture
        .rules
        .fail_rule_set_reads(MaintenanceRuleReadFault::Missing)
        .await;

    let report = fixture.handle().await;
    assert_eq!(report.canceled, 1, "{report:?}");

    let candidate = fixture.only_candidate().await;
    assert_eq!(candidate.state, MaintenanceCandidateState::Canceled);
    assert_eq!(candidate.state_reason, execution_reason::RULE_NOT_ELIGIBLE);
}

#[tokio::test]
async fn an_unreadable_rule_store_holds_the_candidate_instead_of_cancelling_it() {
    let fixture = execution_app(None);
    let rule_id = fixture
        .observed_rule(unmonitor_draft(MONITORED_MATCHER))
        .await;
    fixture.open_gates(true, false).await;
    fixture
        .arm(&rule_id, MaintenanceEffectArming::Reversible, None)
        .await;
    seed_title(&fixture.app, &fixture.user, "Unreadable", true).await;
    fixture.evaluate().await;

    // A store that cannot answer says nothing about whether the rule exists.
    // Cancelling here would retire a live candidate on the strength of a network
    // blip — the one failure on this path that used to do exactly that, while
    // every sibling check held.
    fixture
        .rules
        .fail_rule_set_reads(MaintenanceRuleReadFault::Unreachable)
        .await;

    let report = fixture.handle().await;
    assert_eq!(report.held, 1, "{report:?}");
    assert_eq!(report.canceled, 0);

    let candidate = fixture.only_candidate().await;
    assert_eq!(candidate.state, MaintenanceCandidateState::Blocked);
    assert_eq!(candidate.state_reason, execution_reason::RULE_NOT_ELIGIBLE);
}

#[tokio::test]
async fn both_effect_gates_off_executes_nothing() {
    let fixture = execution_app(None);
    let rule_id = fixture
        .observed_rule(unmonitor_draft(MONITORED_MATCHER))
        .await;
    fixture.open_gates(false, false).await;
    fixture
        .arm(&rule_id, MaintenanceEffectArming::Reversible, None)
        .await;
    seed_title(&fixture.app, &fixture.user, "Watched Movie", true).await;
    // The evaluation gate is off too here; open only it for the evaluation.
    fixture
        .app
        .set_maintenance_instance_gates(
            &fixture.user,
            MaintenanceGatesUpdate {
                evaluation_enabled: Some(true),
                ..Default::default()
            },
        )
        .await
        .expect("evaluation gate");
    fixture.evaluate().await;

    let report = fixture.handle().await;
    assert!(!report.gates_enabled);
    assert_eq!(report.candidates_considered, 0);
    assert!(fixture.evaluation.all_action_runs().await.is_empty());
}

#[tokio::test]
async fn reversible_unmonitor_executes_and_records_the_evidence() {
    let fixture = execution_app(None);
    let rule_id = fixture
        .observed_rule(unmonitor_draft(MONITORED_MATCHER))
        .await;
    fixture.open_gates(true, false).await;
    fixture
        .arm(&rule_id, MaintenanceEffectArming::Reversible, None)
        .await;
    let title = seed_title(&fixture.app, &fixture.user, "Watched Movie", true).await;
    fixture.evaluate().await;

    let report = fixture.handle().await;
    assert!(report.gates_enabled);
    assert_eq!(report.rules_eligible, 1);
    assert_eq!(report.executed, 1, "{report:?}");

    let stored = fixture
        .app
        .get_title(&fixture.user, &title.id)
        .await
        .expect("read title")
        .expect("title exists");
    assert!(!stored.monitored, "the action must unmonitor the title");

    let candidates = fixture.evaluation.all_candidates().await;
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].state, MaintenanceCandidateState::Succeeded);
    assert_eq!(
        candidates[0].state_reason,
        execution_reason::ACTION_SUCCEEDED
    );

    let runs = fixture.evaluation.all_action_runs().await;
    assert_eq!(runs.len(), 1);
    assert_eq!(
        runs[0].status,
        scryer_domain::LifecycleActionRunStatus::Succeeded
    );
    assert_eq!(runs[0].attempt, 1);
    assert!(runs[0].finished_at.is_some());
}

#[tokio::test]
async fn an_already_met_postcondition_reports_already_satisfied() {
    let fixture = execution_app(None);
    let rule_id = fixture.observed_rule(unmonitor_draft(ALWAYS_MATCHER)).await;
    fixture.open_gates(true, false).await;
    fixture
        .arm(&rule_id, MaintenanceEffectArming::Reversible, None)
        .await;
    seed_title(&fixture.app, &fixture.user, "Already Done", false).await;
    fixture.evaluate().await;

    let report = fixture.handle().await;
    assert_eq!(report.already_satisfied, 1, "{report:?}");
    assert_eq!(report.executed, 0);

    let candidates = fixture.evaluation.all_candidates().await;
    assert_eq!(candidates[0].state, MaintenanceCandidateState::Succeeded);
    assert_eq!(
        candidates[0].state_reason,
        execution_reason::ALREADY_SATISFIED
    );
}

#[tokio::test]
async fn a_subject_that_stopped_matching_cancels_instead_of_acting() {
    let fixture = execution_app(None);
    let rule_id = fixture
        .observed_rule(unmonitor_draft(MONITORED_MATCHER))
        .await;
    fixture.open_gates(true, false).await;
    fixture
        .arm(&rule_id, MaintenanceEffectArming::Reversible, None)
        .await;
    let title = seed_title(&fixture.app, &fixture.user, "Changed Mind", true).await;
    fixture.evaluate().await;

    // The operator unmonitors by hand between evaluation and handling; the
    // fresh re-evaluation must see it and cancel rather than "act" on a
    // decision that is no longer true.
    fixture
        .app
        .set_title_monitored(&fixture.user, &title.id, false)
        .await
        .expect("manual unmonitor");

    let report = fixture.handle().await;
    assert_eq!(report.canceled, 1, "{report:?}");
    assert_eq!(report.executed, 0);
    assert_eq!(report.already_satisfied, 0);

    let candidates = fixture.evaluation.all_candidates().await;
    assert_eq!(candidates[0].state, MaintenanceCandidateState::Canceled);
    assert_eq!(
        candidates[0].state_reason,
        execution_reason::NO_MATCH_AT_EXECUTION
    );
}

/// Scope is part of what an operator acknowledged when they armed the rule, so
/// the handler re-reads it with everything else it re-reads. Without this check
/// a rule narrowed after its candidates were opened still acts on the subjects
/// its scope no longer covers, and the narrowing is invisible to the executor.
#[tokio::test]
async fn a_candidate_outside_the_fresh_rule_scope_cancels_instead_of_acting() {
    let fixture = execution_app(None);
    let rule_id = fixture
        .observed_rule(unmonitor_draft(MONITORED_MATCHER))
        .await;
    fixture.open_gates(true, false).await;
    fixture
        .arm(&rule_id, MaintenanceEffectArming::Reversible, None)
        .await;
    seed_title(&fixture.app, &fixture.user, "Out Of Scope", true).await;
    fixture.evaluate().await;

    // Narrowed at the store, with the arming deliberately left in place: going
    // through the service would disarm the rule, and this test would then prove
    // nothing about the executor's own scope check.
    fixture
        .rules
        .re_scope_without_disarming(&rule_id, vec!["library-elsewhere".to_string()])
        .await;

    let report = fixture.handle().await;
    assert_eq!(report.canceled, 1, "{report:?}");
    assert_eq!(report.executed, 0, "{report:?}");

    let candidate = fixture.only_candidate().await;
    assert_eq!(candidate.state, MaintenanceCandidateState::Canceled);
    assert_eq!(
        candidate.state_reason,
        crate::maintenance_rules::candidate_reason::OUT_OF_SCOPE
    );
}

/// An empty scope means instance-wide, which covers every library, so the same
/// check must not turn a rule with no scope into a rule that reaches nothing.
#[tokio::test]
async fn an_empty_rule_scope_still_reaches_every_library() {
    let fixture = execution_app(None);
    let rule_id = fixture
        .observed_rule(unmonitor_draft(MONITORED_MATCHER))
        .await;
    fixture.open_gates(true, false).await;
    fixture
        .arm(&rule_id, MaintenanceEffectArming::Reversible, None)
        .await;
    seed_title(&fixture.app, &fixture.user, "In Scope", true).await;
    fixture.evaluate().await;

    let report = fixture.handle().await;
    assert_eq!(report.executed, 1, "{report:?}");
    assert_eq!(report.canceled, 0, "{report:?}");
}

#[tokio::test]
async fn a_destructive_rule_needs_both_the_gate_and_destructive_arming() {
    let fixture = execution_app(None);
    let rule_id = fixture.observed_rule(delete_draft()).await;
    // Reversible gate on, destructive off: the high-risk rule is not eligible.
    fixture.open_gates(true, false).await;
    seed_title(&fixture.app, &fixture.user, "Doomed", true).await;
    fixture.evaluate().await;
    fixture
        .arm(&rule_id, MaintenanceEffectArming::Destructive, Some(1))
        .await;

    let report = fixture.handle().await;
    assert_eq!(report.rules_eligible, 0, "{report:?}");
    assert!(fixture.evaluation.all_action_runs().await.is_empty());
}

#[tokio::test]
async fn destructive_arming_demands_a_high_risk_action_and_the_current_count() {
    let fixture = execution_app(None);
    let unmonitor_rule = fixture.observed_rule(unmonitor_draft(ALWAYS_MATCHER)).await;

    // A medium-risk action cannot be armed destructive at all.
    let refused = fixture
        .app
        .set_maintenance_rule_arming(
            &fixture.user,
            &unmonitor_rule,
            MaintenanceEffectArming::Destructive,
            Some(0),
        )
        .await
        .expect_err("destructive arming of a medium-risk action must be refused");
    assert!(
        refused.to_string().contains("high-risk")
            || refused.to_string().contains("can delete files"),
        "{refused}"
    );

    let delete_rule = fixture.observed_rule(delete_draft()).await;
    fixture.open_gates(true, false).await;
    fixture
        .app
        .set_maintenance_instance_gates(
            &fixture.user,
            MaintenanceGatesUpdate {
                evaluation_enabled: Some(true),
                ..Default::default()
            },
        )
        .await
        .expect("evaluation gate");
    seed_title(&fixture.app, &fixture.user, "A", true).await;
    seed_title(&fixture.app, &fixture.user, "B", true).await;
    fixture.evaluate().await;

    // The acknowledgement must equal the current non-terminal count, and the
    // refusal names that count so a client can re-present it.
    let mismatch = fixture
        .app
        .set_maintenance_rule_arming(
            &fixture.user,
            &delete_rule,
            MaintenanceEffectArming::Destructive,
            Some(1),
        )
        .await
        .expect_err("a stale acknowledged count must be refused");
    assert!(
        mismatch
            .to_string()
            .contains("acknowledging the current candidate count (2)"),
        "{mismatch}"
    );

    let armed = fixture
        .app
        .set_maintenance_rule_arming(
            &fixture.user,
            &delete_rule,
            MaintenanceEffectArming::Destructive,
            Some(2),
        )
        .await
        .expect("correct acknowledgement arms");
    assert_eq!(
        armed.rule_set.effect_arming,
        MaintenanceEffectArming::Destructive
    );

    // Disarming never needs an acknowledgement.
    fixture
        .arm(&delete_rule, MaintenanceEffectArming::None, None)
        .await;
}

#[tokio::test]
async fn destructive_arming_requires_system_settings_authority() {
    let fixture = execution_app(None);
    let delete_rule = fixture.observed_rule(delete_draft()).await;
    let mut outsider = User::new_admin("outsider");
    outsider.authorization = scryer_domain::UserAuthorization {
        app: AppPermissionMask::from_permissions([
            scryer_domain::AppPermission::ManageCatalogSettings,
        ]),
        loaded: true,
        ..Default::default()
    };

    // Catalog authority is enough for reversible…
    fixture
        .app
        .set_maintenance_rule_arming(
            &outsider,
            &delete_rule,
            MaintenanceEffectArming::Reversible,
            None,
        )
        .await
        .expect("reversible arming under catalog authority");

    // …but destructive arming is elevated (RFC 15).
    let refused = fixture
        .app
        .set_maintenance_rule_arming(
            &outsider,
            &delete_rule,
            MaintenanceEffectArming::Destructive,
            Some(0),
        )
        .await
        .expect_err("destructive arming must demand system-settings authority");
    assert!(matches!(refused, AppError::Unauthorized(_)), "{refused:?}");
}

#[tokio::test]
async fn active_playback_blocks_every_due_action() {
    let fixture = execution_app(Some(PlaybackProbeStatus::ActiveSessions(2)));
    let rule_id = fixture
        .observed_rule(unmonitor_draft(MONITORED_MATCHER))
        .await;
    fixture.open_gates(true, false).await;
    fixture
        .arm(&rule_id, MaintenanceEffectArming::Reversible, None)
        .await;
    let title = seed_title(&fixture.app, &fixture.user, "Being Watched", true).await;
    fixture.evaluate().await;

    let report = fixture.handle().await;
    assert_eq!(report.held, 1, "{report:?}");
    assert_eq!(report.executed, 0);

    let stored = fixture
        .app
        .get_title(&fixture.user, &title.id)
        .await
        .expect("read title")
        .expect("title exists");
    assert!(stored.monitored, "a held action must not mutate the title");

    let candidates = fixture.evaluation.all_candidates().await;
    assert_eq!(candidates[0].state, MaintenanceCandidateState::Blocked);
    assert_eq!(candidates[0].state_reason, execution_reason::PLAYBACK_HOLD);

    let runs = fixture.evaluation.all_action_runs().await;
    assert_eq!(runs.len(), 1);
    assert_eq!(
        runs[0].status,
        scryer_domain::LifecycleActionRunStatus::Held
    );
    assert_eq!(
        runs[0].hold_reason.as_deref(),
        Some(execution_reason::PLAYBACK_HOLD)
    );
}

#[tokio::test]
async fn an_unreachable_media_server_fails_closed() {
    let fixture = execution_app(Some(PlaybackProbeStatus::Unreachable(
        "status 401".to_string(),
    )));
    let rule_id = fixture
        .observed_rule(unmonitor_draft(MONITORED_MATCHER))
        .await;
    fixture.open_gates(true, false).await;
    fixture
        .arm(&rule_id, MaintenanceEffectArming::Reversible, None)
        .await;
    seed_title(&fixture.app, &fixture.user, "Unknown Playback", true).await;
    fixture.evaluate().await;

    let report = fixture.handle().await;
    assert_eq!(report.held, 1, "{report:?}");
    let candidates = fixture.evaluation.all_candidates().await;
    assert_eq!(candidates[0].state, MaintenanceCandidateState::Blocked);
    assert_eq!(
        candidates[0].state_reason,
        execution_reason::PLAYBACK_UNKNOWN
    );
}

#[tokio::test]
async fn a_shadow_rule_never_acts_even_when_armed() {
    let fixture = execution_app(None);
    let created = fixture
        .app
        .create_maintenance_rule_set(&fixture.user, unmonitor_draft(MONITORED_MATCHER))
        .await
        .expect("create rule");
    let rule_id = created.rule_set.id;
    fixture
        .app
        .set_maintenance_rule_evaluation_mode(
            &fixture.user,
            &rule_id,
            MaintenanceEvaluationMode::Shadow,
        )
        .await
        .expect("shadow mode");
    fixture.open_gates(true, true).await;
    fixture
        .arm(&rule_id, MaintenanceEffectArming::Reversible, None)
        .await;
    seed_title(&fixture.app, &fixture.user, "Shadowed", true).await;
    fixture.evaluate().await;

    let report = fixture.handle().await;
    assert_eq!(report.rules_eligible, 0, "{report:?}");
    assert!(fixture.evaluation.all_action_runs().await.is_empty());
}

#[tokio::test]
async fn a_candidate_inside_its_grace_window_is_not_touched() {
    let fixture = execution_app(None);
    let mut draft = unmonitor_draft(MONITORED_MATCHER);
    draft.grace_days = 30;
    let rule_id = fixture.observed_rule(draft).await;
    fixture.open_gates(true, false).await;
    fixture
        .arm(&rule_id, MaintenanceEffectArming::Reversible, None)
        .await;
    seed_title(&fixture.app, &fixture.user, "Still Grace", true).await;
    fixture.evaluate().await;

    let report = fixture.handle().await;
    assert_eq!(report.candidates_considered, 0, "{report:?}");
    let candidates = fixture.evaluation.all_candidates().await;
    assert_eq!(candidates[0].state, MaintenanceCandidateState::Observing);
}

#[tokio::test]
async fn an_undecidable_subject_blocks_at_execution() {
    let fixture = execution_app(None);
    let rule_id = fixture
        .observed_rule(unmonitor_draft(MONITORED_MATCHER))
        .await;
    fixture.open_gates(true, false).await;
    fixture
        .arm(&rule_id, MaintenanceEffectArming::Reversible, None)
        .await;
    seed_title(&fixture.app, &fixture.user, "Undecidable", true).await;
    fixture.evaluate().await;

    // Rewrite the revision in place (test-only) so the same revision now
    // returns unknown: the executor's fresh re-evaluation must hold, exactly
    // as it would when a fact source degrades between passes.
    let detail = fixture
        .app
        .get_maintenance_rule_set(&fixture.user, &rule_id)
        .await
        .expect("read rule")
        .expect("rule exists");
    let mut revision = detail.revision;
    revision.rego_source = scryer_rules::maintenance::rewrite_package_declaration(
        "match := true\n\nunknown := true\n",
        &rule_id,
    );
    fixture.rules.replace_revision_in_place(revision).await;

    let report = fixture.handle().await;
    assert_eq!(report.held, 1, "{report:?}");
    let candidates = fixture.evaluation.all_candidates().await;
    assert_eq!(candidates[0].state, MaintenanceCandidateState::Blocked);
    assert_eq!(
        candidates[0].state_reason,
        execution_reason::UNKNOWN_AT_EXECUTION
    );
}

#[tokio::test]
async fn a_blocked_candidate_is_retried_on_the_next_pass() {
    let fixture = execution_app(Some(PlaybackProbeStatus::ActiveSessions(1)));
    let rule_id = fixture
        .observed_rule(unmonitor_draft(MONITORED_MATCHER))
        .await;
    fixture.open_gates(true, false).await;
    fixture
        .arm(&rule_id, MaintenanceEffectArming::Reversible, None)
        .await;
    let title = seed_title(&fixture.app, &fixture.user, "Later", true).await;
    fixture.evaluate().await;

    let first = fixture.handle().await;
    assert_eq!(first.held, 1);

    // Playback ends: swap the probe by rebuilding the app? The probe is fixed,
    // so instead assert the Blocked candidate is selected again on the next
    // pass and held again, which is the re-check contract.
    let second = fixture.handle().await;
    assert_eq!(second.candidates_considered, 1, "{second:?}");
    assert_eq!(second.held, 1);
    let stored = fixture
        .app
        .get_title(&fixture.user, &title.id)
        .await
        .expect("read title")
        .expect("title exists");
    assert!(stored.monitored);
}

// ── A title an operation owns is out of reach (FR-084) ──────────────────────

fn execution_app_with_operations(
    operations: Arc<InMemoryLocationOperationStore>,
) -> ExecutionFixture {
    let (app, user) = bootstrap();
    let rules = Arc::new(InMemoryMaintenanceRuleRepo::default());
    let evaluation = Arc::new(InMemoryMaintenanceEvaluationRepo::default());
    let media_files = Arc::new(MockMediaFileRepo::default());
    let app = app.with_test_overrides(|services| {
        services
            .with_maintenance_rule_set_store(rules.clone())
            .with_maintenance_evaluation_store(evaluation.clone())
            .with_media_files(media_files)
            .with_location_operation_repository(operations)
    });
    ExecutionFixture {
        app,
        user,
        rules,
        evaluation,
    }
}

#[tokio::test]
async fn a_destructive_action_holds_while_a_location_operation_owns_the_title() {
    let operations = Arc::new(InMemoryLocationOperationStore::new());
    let fixture = execution_app_with_operations(operations.clone());
    let rule_id = fixture.observed_rule(delete_draft()).await;
    fixture.open_gates(true, true).await;
    let title = seed_title(&fixture.app, &fixture.user, "Mid-move", true).await;
    fixture.evaluate().await;
    fixture
        .arm(&rule_id, MaintenanceEffectArming::Destructive, Some(1))
        .await;
    operations
        .claim_location_operation_ownership(
            "operation-1",
            &[OwnedEntity::Title(title.id.clone())],
        )
        .await
        .expect("claim the title");

    let report = fixture.handle().await;
    assert_eq!(report.held, 1, "{report:?}");
    assert_eq!(report.executed, 0);
    assert!(
        fixture
            .app
            .get_title(&fixture.user, &title.id)
            .await
            .expect("read title")
            .is_some(),
        "a held delete must not remove the title"
    );
    let candidate = fixture.only_candidate().await;
    assert_eq!(candidate.state, MaintenanceCandidateState::Blocked);
    assert_eq!(
        candidate.state_reason,
        execution_reason::LOCATION_OPERATION_HOLD
    );
    let runs = fixture.evaluation.all_action_runs().await;
    assert_eq!(runs.len(), 1);
    assert_eq!(
        runs[0].hold_reason.as_deref(),
        Some(execution_reason::LOCATION_OPERATION_HOLD)
    );

    // The operation releasing its claim is what lifts the hold: the next pass
    // no longer holds for this reason.
    operations
        .release_location_operation_ownership("operation-1")
        .await
        .expect("release the claim");
    fixture.evaluate().await;
    let second = fixture.handle().await;
    assert_eq!(second.held, 0, "{second:?}");
    let candidate = fixture.only_candidate().await;
    assert_ne!(
        candidate.state_reason,
        execution_reason::LOCATION_OPERATION_HOLD
    );
}

#[tokio::test]
async fn the_policy_delete_itself_refuses_a_title_a_location_operation_owns() {
    let operations = Arc::new(InMemoryLocationOperationStore::new());
    let fixture = execution_app_with_operations(operations.clone());
    let title = seed_title(&fixture.app, &fixture.user, "Mid-move", true).await;
    operations
        .claim_location_operation_ownership(
            "operation-1",
            &[OwnedEntity::Title(title.id.clone())],
        )
        .await
        .expect("claim the title");

    let authorization = crate::PolicyDeleteAuthorization {
        rule_set_id: "rule-1".to_string(),
        candidate_id: "candidate-1".to_string(),
        revision_number: 1,
    };
    let error = fixture
        .app
        .delete_title_by_policy(&fixture.user, &title.id, "unused", &authorization)
        .await
        .expect_err("an owned title must be refused before any fingerprint check");
    assert!(
        matches!(&error, AppError::Validation(message) if message.contains("location operation owns")),
        "{error:?}"
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

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
use crate::lib_tests::maintenance_rules::InMemoryMaintenanceRuleRepo;
use crate::maintenance_rules::{
    MaintenanceActionKind, MaintenanceActionSpec, MaintenanceGatesUpdate, MaintenanceRuleDraft,
    execution_reason,
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
     \tinput.facts.monitored.status == \"known\"\n\
     \tinput.facts.monitored.value\n\
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

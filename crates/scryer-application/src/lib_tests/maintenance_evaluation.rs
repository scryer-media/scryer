//! The candidate state machine (RFC 137 sections 7.5 and 8).
//!
//! These tests are the specification of what the dark evaluator is allowed to
//! do to a candidate. Every branch of RFC 7.5 has a test here, and every one of
//! them also asserts what did *not* happen: the grace clock is the thing an
//! action will eventually fire on, so a test that only checks the happy
//! transition would not notice the clock being silently restarted.

use super::*;

use crate::lib_tests::maintenance_rules::InMemoryMaintenanceRuleRepo;
use crate::maintenance_rules::{
    MaintenanceActionKind, MaintenanceActionSpec, MaintenanceCandidateFilter,
    MaintenanceGatesUpdate, MaintenanceMatcherDraft, MaintenanceRuleDraft, candidate_reason,
};
use crate::ports::MaintenanceCandidateQuery;
use scryer_domain::{
    LifecycleActionRun, LifecycleCandidate, MaintenanceCandidateState, MaintenanceEvaluationMode,
    MaintenanceEvaluationRun, MaintenanceEvaluationRunStatus, MaintenanceRuleExclusion,
    MaintenanceRuleRevision,
};

/// Matches every monitored title.
const MONITORED_MATCHER: &str = "package whatever\n\
     import rego.v1\n\n\
     match if {\n\
     \tinput.facts.monitored\n\
     }\n";

/// Matches nothing: the facet never equals this sentinel.
const NEVER_MATCHER: &str = "package whatever\n\
     import rego.v1\n\n\
     match if {\n\
     \tinput.subject.facet == \"not-a-facet\"\n\
     }\n";

/// Would match, but declares its own hold on the envelope surface.
///
/// Deliberately written against `input.observations` rather than
/// `input.facts`: this is the manual, opted-out path, so the hold here is the
/// rule's own `unknown` rather than the engine's, and the reason code is one
/// the author chose. The candidate must be held either way.
const UNKNOWN_MATCHER: &str = "package whatever\n\
     import rego.v1\n\n\
     match := true\n\n\
     unknown if {\n\
     \tinput.observations.last_upgraded_at.status != \"known\"\n\
     }\n\n\
     reasons contains \"upgrade_history_missing\" if {\n\
     \tinput.observations.last_upgraded_at.status != \"known\"\n\
     }\n";

// ── In-memory evaluation store ──────────────────────────────────────────────

/// Mirrors the SQL store's contract, including the one-active-candidate-per
/// (rule, title) invariant, so a service bug cannot hide behind a permissive
/// double.
#[derive(Default)]
pub(super) struct InMemoryMaintenanceEvaluationRepo {
    candidates: Mutex<Vec<LifecycleCandidate>>,
    exclusions: Mutex<Vec<MaintenanceRuleExclusion>>,
    runs: Mutex<Vec<MaintenanceEvaluationRun>>,
    action_runs: Mutex<Vec<LifecycleActionRun>>,
    /// When set to a candidate id, the next [`Self::finish_action_run`] also
    /// moves that candidate out of `executing`, exactly as a concurrent pass
    /// reclaiming a stale lease would.
    ///
    /// Production has no such hook — only the lease writes that state — but the
    /// window between a worker finishing its action and recording its result is
    /// precisely where a lost update would land, so a test needs a way to write
    /// into it.
    steal_lease_on_finish: Mutex<Option<String>>,
}

impl InMemoryMaintenanceEvaluationRepo {
    pub(super) async fn all_candidates(&self) -> Vec<LifecycleCandidate> {
        self.candidates.lock().await.clone()
    }

    async fn all_runs(&self) -> Vec<MaintenanceEvaluationRun> {
        self.runs.lock().await.clone()
    }

    pub(super) async fn all_action_runs(&self) -> Vec<LifecycleActionRun> {
        self.action_runs.lock().await.clone()
    }

    /// Leave a candidate looking like a crashed worker left it: `executing`,
    /// with an `updated_at` old enough for the lease to call it abandoned.
    /// Nothing in production can produce this row deliberately, which is exactly
    /// why it used to be unrecoverable.
    pub(super) async fn strand_as_executing(&self, id: &str, updated_at: DateTime<Utc>) {
        let mut rows = self.candidates.lock().await;
        let candidate = rows
            .iter_mut()
            .find(|candidate| candidate.id == id)
            .expect("candidate exists");
        candidate.state = MaintenanceCandidateState::Executing;
        candidate.state_reason = "execution_leased".to_string();
        candidate.updated_at = updated_at;
    }

    /// Arm the lease theft described on [`Self::steal_lease_on_finish`].
    pub(super) async fn steal_lease_when_the_next_run_finishes(&self, candidate_id: &str) {
        *self.steal_lease_on_finish.lock().await = Some(candidate_id.to_string());
    }
}

#[async_trait]
impl MaintenanceCandidateRepository for InMemoryMaintenanceEvaluationRepo {
    async fn get_active_candidate(
        &self,
        rule_set_id: &str,
        title_id: &str,
    ) -> AppResult<Option<LifecycleCandidate>> {
        Ok(self
            .candidates
            .lock()
            .await
            .iter()
            .find(|candidate| {
                candidate.rule_set_id == rule_set_id
                    && candidate.title_id == title_id
                    && !candidate.state.is_terminal()
            })
            .cloned())
    }

    async fn list_candidates(
        &self,
        query: &MaintenanceCandidateQuery,
    ) -> AppResult<Vec<LifecycleCandidate>> {
        let mut rows: Vec<LifecycleCandidate> = self
            .candidates
            .lock()
            .await
            .iter()
            .filter(|candidate| {
                query
                    .rule_set_id
                    .as_ref()
                    .is_none_or(|id| &candidate.rule_set_id == id)
                    && query
                        .library_id
                        .as_ref()
                        .is_none_or(|id| &candidate.library_id == id)
                    && (query.states.is_empty() || query.states.contains(&candidate.state))
            })
            .cloned()
            .collect();
        rows.sort_by(|left, right| {
            left.due_at
                .cmp(&right.due_at)
                .then_with(|| left.id.cmp(&right.id))
        });
        if let Some(limit) = query.limit {
            rows.truncate(limit);
        }
        Ok(rows)
    }

    async fn max_match_generation(&self, rule_set_id: &str, title_id: &str) -> AppResult<i64> {
        Ok(self
            .candidates
            .lock()
            .await
            .iter()
            .filter(|candidate| {
                candidate.rule_set_id == rule_set_id && candidate.title_id == title_id
            })
            .map(|candidate| candidate.match_generation)
            .max()
            .unwrap_or(0))
    }

    async fn create_candidate(&self, candidate: &LifecycleCandidate) -> AppResult<()> {
        let mut rows = self.candidates.lock().await;
        if rows.iter().any(|row| {
            row.rule_set_id == candidate.rule_set_id
                && row.title_id == candidate.title_id
                && !row.state.is_terminal()
        }) {
            return Err(AppError::Validation(format!(
                "maintenance rule {} already has an active candidate for title {}",
                candidate.rule_set_id, candidate.title_id
            )));
        }
        rows.push(candidate.clone());
        Ok(())
    }

    async fn record_candidate_match(
        &self,
        id: &str,
        last_matched_at: DateTime<Utc>,
        reason_codes: &[String],
        updated_at: DateTime<Utc>,
    ) -> AppResult<()> {
        let mut rows = self.candidates.lock().await;
        let candidate = rows
            .iter_mut()
            .find(|candidate| candidate.id == id)
            .ok_or_else(|| AppError::NotFound(id.to_string()))?;
        candidate.last_matched_at = last_matched_at;
        candidate.last_evaluated_at = last_matched_at;
        candidate.reason_codes = reason_codes.to_vec();
        candidate.held_since = None;
        candidate.updated_at = updated_at;
        Ok(())
    }

    async fn hold_candidate(
        &self,
        id: &str,
        held_since: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    ) -> AppResult<()> {
        let mut rows = self.candidates.lock().await;
        let candidate = rows
            .iter_mut()
            .find(|candidate| candidate.id == id)
            .ok_or_else(|| AppError::NotFound(id.to_string()))?;
        candidate.last_evaluated_at = held_since;
        candidate.held_since = candidate.held_since.or(Some(held_since));
        candidate.updated_at = updated_at;
        Ok(())
    }

    async fn transition_candidate_state(
        &self,
        id: &str,
        state: MaintenanceCandidateState,
        state_reason: &str,
        expected_states: &[MaintenanceCandidateState],
        updated_at: DateTime<Utc>,
    ) -> AppResult<bool> {
        if expected_states.is_empty() {
            return Err(AppError::Validation(
                "a candidate transition must name the states it expects".to_string(),
            ));
        }
        let mut rows = self.candidates.lock().await;
        let candidate = rows
            .iter_mut()
            .find(|candidate| candidate.id == id)
            .ok_or_else(|| AppError::NotFound(id.to_string()))?;
        // The compare-and-set is the whole point of the method, so the double
        // enforces it: a permissive fake would let a lost update pass here and
        // only fail against the real store.
        if !expected_states.contains(&candidate.state) {
            return Ok(false);
        }
        candidate.state = state;
        candidate.state_reason = state_reason.to_string();
        candidate.last_evaluated_at = updated_at;
        candidate.updated_at = updated_at;
        Ok(true)
    }

    async fn cancel_active_candidates_for_rule(
        &self,
        rule_set_id: &str,
        state_reason: &str,
        updated_at: DateTime<Utc>,
    ) -> AppResult<u64> {
        let mut canceled = 0;
        for candidate in self.candidates.lock().await.iter_mut() {
            if candidate.rule_set_id == rule_set_id && !candidate.state.is_terminal() {
                candidate.state = MaintenanceCandidateState::Canceled;
                candidate.state_reason = state_reason.to_string();
                candidate.updated_at = updated_at;
                canceled += 1;
            }
        }
        Ok(canceled)
    }

    async fn count_candidates_by_state(
        &self,
        rule_set_id: &str,
    ) -> AppResult<Vec<(MaintenanceCandidateState, i64)>> {
        let mut counts: HashMap<String, (MaintenanceCandidateState, i64)> = HashMap::new();
        for candidate in self.candidates.lock().await.iter() {
            if candidate.rule_set_id != rule_set_id {
                continue;
            }
            let entry = counts
                .entry(candidate.state.as_storage_str().to_string())
                .or_insert((candidate.state, 0));
            entry.1 += 1;
        }
        let mut rows: Vec<(MaintenanceCandidateState, i64)> = counts.into_values().collect();
        rows.sort_by_key(|(state, _)| state.as_storage_str());
        Ok(rows)
    }

    async fn list_due_candidates(
        &self,
        rule_set_id: &str,
        due_before: DateTime<Utc>,
        stale_before: DateTime<Utc>,
        limit: usize,
    ) -> AppResult<Vec<LifecycleCandidate>> {
        let mut rows: Vec<LifecycleCandidate> = self
            .candidates
            .lock()
            .await
            .iter()
            .filter(|candidate| {
                candidate.rule_set_id == rule_set_id
                    && candidate.due_at <= due_before
                    && (matches!(
                        candidate.state,
                        MaintenanceCandidateState::Observing
                            | MaintenanceCandidateState::PendingAction
                            | MaintenanceCandidateState::Due
                            | MaintenanceCandidateState::Blocked
                    ) || (candidate.state == MaintenanceCandidateState::Executing
                        && candidate.updated_at < stale_before))
            })
            .cloned()
            .collect();
        rows.sort_by_key(|candidate| candidate.due_at);
        rows.truncate(limit);
        Ok(rows)
    }

    async fn lease_candidate_for_execution(
        &self,
        id: &str,
        stale_before: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    ) -> AppResult<bool> {
        let mut rows = self.candidates.lock().await;
        let Some(candidate) = rows.iter_mut().find(|candidate| candidate.id == id) else {
            return Ok(false);
        };
        let leasable = candidate.state == MaintenanceCandidateState::Due
            || (candidate.state == MaintenanceCandidateState::Executing
                && candidate.updated_at < stale_before);
        if !leasable {
            return Ok(false);
        }
        candidate.state = MaintenanceCandidateState::Executing;
        candidate.state_reason = "execution_leased".to_string();
        candidate.updated_at = updated_at;
        Ok(true)
    }

    async fn record_candidate_attempts(
        &self,
        id: &str,
        action_attempts: i64,
        updated_at: DateTime<Utc>,
    ) -> AppResult<()> {
        let mut rows = self.candidates.lock().await;
        let candidate = rows
            .iter_mut()
            .find(|candidate| candidate.id == id)
            .ok_or_else(|| AppError::NotFound(id.to_string()))?;
        candidate.action_attempts = action_attempts;
        candidate.updated_at = updated_at;
        Ok(())
    }
}

#[async_trait]
impl LifecycleActionRunRepository for InMemoryMaintenanceEvaluationRepo {
    async fn start_action_run(&self, run: &LifecycleActionRun) -> AppResult<()> {
        let mut rows = self.action_runs.lock().await;
        if rows.iter().any(|stored| {
            stored.idempotency_key == run.idempotency_key && stored.attempt == run.attempt
        }) {
            return Err(AppError::Repository(format!(
                "duplicate action attempt {}#{}",
                run.idempotency_key, run.attempt
            )));
        }
        rows.push(run.clone());
        Ok(())
    }

    async fn finish_action_run(&self, run: &LifecycleActionRun) -> AppResult<()> {
        {
            let mut rows = self.action_runs.lock().await;
            let stored = rows
                .iter_mut()
                .find(|stored| stored.id == run.id)
                .ok_or_else(|| AppError::NotFound(run.id.clone()))?;
            *stored = run.clone();
        }

        // The armed lease theft, if any: a concurrent pass decided this lease
        // was stale and took it while the action was still running.
        if let Some(candidate_id) = self.steal_lease_on_finish.lock().await.take() {
            let mut rows = self.candidates.lock().await;
            if let Some(candidate) = rows
                .iter_mut()
                .find(|candidate| candidate.id == candidate_id)
            {
                candidate.state = MaintenanceCandidateState::Due;
                candidate.state_reason = "execution_lease_reclaimed".to_string();
            }
        }
        Ok(())
    }

    async fn list_action_runs(
        &self,
        rule_set_id: Option<&str>,
        candidate_id: Option<&str>,
        limit: Option<usize>,
    ) -> AppResult<Vec<LifecycleActionRun>> {
        let mut rows: Vec<LifecycleActionRun> = self
            .action_runs
            .lock()
            .await
            .iter()
            .filter(|run| {
                rule_set_id.is_none_or(|rule_set_id| run.rule_set_id == rule_set_id)
                    && candidate_id.is_none_or(|candidate_id| run.candidate_id == candidate_id)
            })
            .cloned()
            .collect();
        rows.sort_by_key(|run| std::cmp::Reverse(run.started_at));
        if let Some(limit) = limit {
            rows.truncate(limit);
        }
        Ok(rows)
    }
}

#[async_trait]
impl MaintenanceExclusionRepository for InMemoryMaintenanceEvaluationRepo {
    async fn list_exclusions(
        &self,
        rule_set_id: Option<&str>,
    ) -> AppResult<Vec<MaintenanceRuleExclusion>> {
        Ok(self
            .exclusions
            .lock()
            .await
            .iter()
            .filter(|exclusion| match rule_set_id {
                Some(rule_set_id) => exclusion
                    .rule_set_id
                    .as_deref()
                    .is_none_or(|id| id == rule_set_id),
                None => true,
            })
            .cloned()
            .collect())
    }

    async fn get_exclusion(&self, id: &str) -> AppResult<Option<MaintenanceRuleExclusion>> {
        Ok(self
            .exclusions
            .lock()
            .await
            .iter()
            .find(|exclusion| exclusion.id == id)
            .cloned())
    }

    async fn create_exclusion(&self, exclusion: &MaintenanceRuleExclusion) -> AppResult<()> {
        self.exclusions.lock().await.push(exclusion.clone());
        Ok(())
    }

    async fn delete_exclusion(&self, id: &str) -> AppResult<()> {
        self.exclusions
            .lock()
            .await
            .retain(|exclusion| exclusion.id != id);
        Ok(())
    }
}

#[async_trait]
impl MaintenanceEvaluationRunRepository for InMemoryMaintenanceEvaluationRepo {
    async fn start_evaluation_run(&self, run: &MaintenanceEvaluationRun) -> AppResult<()> {
        self.runs.lock().await.push(run.clone());
        Ok(())
    }

    async fn finish_evaluation_run(&self, run: &MaintenanceEvaluationRun) -> AppResult<()> {
        let mut rows = self.runs.lock().await;
        let stored = rows
            .iter_mut()
            .find(|stored| stored.id == run.id)
            .ok_or_else(|| AppError::NotFound(run.id.clone()))?;
        *stored = run.clone();
        Ok(())
    }

    async fn list_evaluation_runs(
        &self,
        rule_set_id: Option<&str>,
        limit: Option<usize>,
    ) -> AppResult<Vec<MaintenanceEvaluationRun>> {
        let mut rows: Vec<MaintenanceEvaluationRun> = self
            .runs
            .lock()
            .await
            .iter()
            .filter(|run| rule_set_id.is_none_or(|id| run.rule_set_id == id))
            .cloned()
            .collect();
        rows.sort_by_key(|run| std::cmp::Reverse(run.started_at));
        if let Some(limit) = limit {
            rows.truncate(limit);
        }
        Ok(rows)
    }
}

// ── Fixture ─────────────────────────────────────────────────────────────────

struct EvaluationFixture {
    app: AppUseCase,
    user: User,
    rules: Arc<InMemoryMaintenanceRuleRepo>,
    evaluation: Arc<InMemoryMaintenanceEvaluationRepo>,
}

fn evaluation_app() -> EvaluationFixture {
    let (app, user) = bootstrap();
    let rules = Arc::new(InMemoryMaintenanceRuleRepo::default());
    let evaluation = Arc::new(InMemoryMaintenanceEvaluationRepo::default());
    let media_files = Arc::new(MockMediaFileRepo::default());
    let app = app.with_test_overrides(|services| {
        services
            .with_maintenance_rule_set_store(rules.clone())
            .with_maintenance_evaluation_store(evaluation.clone())
            .with_media_files(media_files)
    });
    EvaluationFixture {
        app,
        user,
        rules,
        evaluation,
    }
}

fn draft(rego_source: &str, grace_days: i64) -> MaintenanceRuleDraft {
    MaintenanceRuleDraft {
        name: "Stale movies".to_string(),
        description: String::new(),
        rego_source: rego_source.to_string(),
        action_spec: MaintenanceActionSpec::new(MaintenanceActionKind::UnmonitorScopeKeepFiles),
        grace_days,
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

impl EvaluationFixture {
    /// Create a rule and arm it, which is always two deliberate steps.
    async fn armed_rule(&self, rego_source: &str, grace_days: i64) -> String {
        let created = self
            .app
            .create_maintenance_rule_set(&self.user, draft(rego_source, grace_days))
            .await
            .expect("create rule set");
        self.app
            .set_maintenance_rule_evaluation_mode(
                &self.user,
                &created.rule_set.id,
                MaintenanceEvaluationMode::Shadow,
            )
            .await
            .expect("arm rule");
        created.rule_set.id
    }

    async fn open_evaluation_gate(&self) {
        self.app
            .set_maintenance_instance_gates(
                &self.user,
                MaintenanceGatesUpdate {
                    evaluation_enabled: Some(true),
                    ..Default::default()
                },
            )
            .await
            .expect("arm the evaluation gate");
    }

    async fn evaluate(&self) -> crate::maintenance_rules::MaintenanceEvaluationReport {
        self.app
            .run_maintenance_rule_evaluation_job()
            .await
            .expect("evaluation pass")
    }

    async fn only_candidate(&self) -> LifecycleCandidate {
        let candidates = self.evaluation.all_candidates().await;
        assert_eq!(candidates.len(), 1, "{candidates:?}");
        candidates[0].clone()
    }
}

// ── Gates ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn every_gate_defaults_off_and_updates_are_independent() {
    let fixture = evaluation_app();

    let initial = fixture
        .app
        .maintenance_instance_gates(&fixture.user)
        .await
        .expect("read gates");
    assert_eq!(
        initial,
        crate::maintenance_rules::MaintenanceGates::default(),
        "an unconfigured instance must evaluate and act on nothing"
    );

    let updated = fixture
        .app
        .set_maintenance_instance_gates(
            &fixture.user,
            MaintenanceGatesUpdate {
                evaluation_enabled: Some(true),
                destructive_effects_enabled: Some(true),
                ..Default::default()
            },
        )
        .await
        .expect("set gates");
    assert!(updated.evaluation_enabled);
    assert!(updated.destructive_effects_enabled);
    assert!(!updated.result_display_enabled);

    // Omitting a field must leave it exactly as stored, or arming one gate
    // would silently disarm the others.
    let partial = fixture
        .app
        .set_maintenance_instance_gates(
            &fixture.user,
            MaintenanceGatesUpdate {
                result_display_enabled: Some(true),
                ..Default::default()
            },
        )
        .await
        .expect("set gates");
    assert!(partial.evaluation_enabled);
    assert!(partial.destructive_effects_enabled);
    assert!(partial.result_display_enabled);
}

#[tokio::test]
async fn gates_require_system_settings_management() {
    let fixture = evaluation_app();
    let mut outsider = User::new_admin("outsider");
    outsider.authorization = scryer_domain::UserAuthorization {
        app: AppPermissionMask::default(),
        loaded: true,
        ..Default::default()
    };

    let read = fixture
        .app
        .maintenance_instance_gates(&outsider)
        .await
        .expect_err("reading gates must be permission gated");
    assert!(matches!(read, AppError::Unauthorized(_)), "{read:?}");

    let write = fixture
        .app
        .set_maintenance_instance_gates(&outsider, MaintenanceGatesUpdate::default())
        .await
        .expect_err("arming gates must be permission gated");
    assert!(matches!(write, AppError::Unauthorized(_)), "{write:?}");
}

#[tokio::test]
async fn the_evaluation_gate_makes_a_run_a_no_op() {
    let fixture = evaluation_app();
    seed_title(&fixture.app, &fixture.user, "Monitored", true).await;
    fixture.armed_rule(MONITORED_MATCHER, 7).await;

    let report = fixture.evaluate().await;

    assert!(!report.gate_enabled);
    assert_eq!(report.rules_considered, 0);
    assert!(
        fixture.evaluation.all_candidates().await.is_empty(),
        "the gate must stop evaluation before anything is recorded"
    );
    assert!(
        fixture.evaluation.all_runs().await.is_empty(),
        "a gated-off pass must not even record a run"
    );
}

// ── Rule mode ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn setting_a_mode_derives_enabled_and_creation_still_ships_dark() {
    let fixture = evaluation_app();
    let created = fixture
        .app
        .create_maintenance_rule_set(&fixture.user, draft(MONITORED_MATCHER, 7))
        .await
        .expect("create rule set");
    assert!(!created.rule_set.enabled, "rules are created disabled");

    for (mode, expected_enabled) in [
        (MaintenanceEvaluationMode::Shadow, true),
        (MaintenanceEvaluationMode::Observe, true),
        (MaintenanceEvaluationMode::Disabled, false),
    ] {
        let detail = fixture
            .app
            .set_maintenance_rule_evaluation_mode(&fixture.user, &created.rule_set.id, mode)
            .await
            .expect("set mode");
        assert_eq!(detail.rule_set.evaluation_mode, mode);
        assert_eq!(detail.rule_set.enabled, expected_enabled);
        assert_eq!(
            detail.rule_set.current_revision_number, 1,
            "a mode change is not a matcher change, so no revision is appended"
        );
    }
}

// ── The state machine ───────────────────────────────────────────────────────

#[tokio::test]
async fn a_first_match_opens_a_candidate_and_starts_the_clock() {
    let fixture = evaluation_app();
    let title = seed_title(&fixture.app, &fixture.user, "Monitored", true).await;
    let rule_set_id = fixture.armed_rule(MONITORED_MATCHER, 7).await;
    fixture.open_evaluation_gate().await;

    let report = fixture.evaluate().await;
    assert_eq!(report.candidates_created, 1);
    assert_eq!(report.titles_evaluated, 1);

    let candidate = fixture.only_candidate().await;
    assert_eq!(candidate.rule_set_id, rule_set_id);
    assert_eq!(candidate.title_id, title.id);
    assert_eq!(candidate.state, MaintenanceCandidateState::Observing);
    assert_eq!(candidate.state_reason, candidate_reason::FIRST_MATCH);
    assert_eq!(candidate.match_generation, 1);
    assert_eq!(candidate.revision_number, 1);
    assert_eq!(candidate.grace_days, 7);
    assert_eq!(candidate.first_matched_at, candidate.last_matched_at);
    assert_eq!(
        candidate.due_at,
        candidate.first_matched_at + chrono::Duration::days(7),
        "the grace period is materialized onto the row"
    );
    assert_eq!(candidate.held_since, None);
    assert_eq!(
        candidate.action_kind,
        MaintenanceActionKind::UnmonitorScopeKeepFiles.as_wire_str(),
        "the candidate records what the revision authorizes, even though nothing runs it"
    );
}

#[tokio::test]
async fn a_zero_day_grace_is_due_the_moment_it_matches() {
    let fixture = evaluation_app();
    seed_title(&fixture.app, &fixture.user, "Monitored", true).await;
    fixture.armed_rule(MONITORED_MATCHER, 0).await;
    fixture.open_evaluation_gate().await;

    fixture.evaluate().await;

    let candidate = fixture.only_candidate().await;
    assert_eq!(candidate.due_at, candidate.first_matched_at);
}

#[tokio::test]
async fn a_repeat_match_advances_only_the_last_match_and_never_the_clock() {
    let fixture = evaluation_app();
    seed_title(&fixture.app, &fixture.user, "Monitored", true).await;
    fixture.armed_rule(MONITORED_MATCHER, 7).await;
    fixture.open_evaluation_gate().await;

    fixture.evaluate().await;
    let first = fixture.only_candidate().await;

    let report = fixture.evaluate().await;
    assert_eq!(
        report.candidates_created, 0,
        "a second pass must reuse the candidate, not open a rival one"
    );
    assert_eq!(report.candidates_matched, 1);

    let second = fixture.only_candidate().await;
    assert_eq!(second.id, first.id);
    assert_eq!(second.match_generation, first.match_generation);
    assert_eq!(
        second.first_matched_at, first.first_matched_at,
        "continuous membership must not restart the grace clock"
    );
    assert_eq!(second.due_at, first.due_at);
    assert!(second.last_matched_at >= first.last_matched_at);
    assert_eq!(second.state, MaintenanceCandidateState::Observing);
}

#[tokio::test]
async fn an_unknown_decision_holds_the_candidate_instead_of_cancelling_it() {
    let fixture = evaluation_app();
    seed_title(&fixture.app, &fixture.user, "Monitored", true).await;
    let rule_set_id = fixture.armed_rule(MONITORED_MATCHER, 7).await;
    fixture.open_evaluation_gate().await;
    fixture.evaluate().await;
    let before = fixture.only_candidate().await;

    // The matcher is swapped in place rather than through a revision bump, so
    // what this test observes is the unknown branch and not the supersede one.
    swap_matcher_in_place(&fixture, &rule_set_id, UNKNOWN_MATCHER, &before).await;

    let report = fixture.evaluate().await;
    assert_eq!(report.candidates_held, 1);
    assert_eq!(report.candidates_canceled, 0, "unknown must never cancel");
    assert_eq!(
        report.candidates_created, 0,
        "an unknown decision must not open a candidate"
    );

    let after = fixture.only_candidate().await;
    assert_eq!(after.id, before.id);
    assert_eq!(after.state, MaintenanceCandidateState::Observing);
    assert_eq!(
        after.due_at, before.due_at,
        "a hold must not advance the clock"
    );
    assert_eq!(after.last_matched_at, before.last_matched_at);
    assert!(after.held_since.is_some(), "a held candidate records when");
}

/// An unknown decision on a subject with no candidate is simply nothing: it
/// must not open one on evidence the rule admits it does not have.
#[tokio::test]
async fn an_unknown_decision_never_opens_a_candidate() {
    let fixture = evaluation_app();
    seed_title(&fixture.app, &fixture.user, "Monitored", true).await;
    fixture.armed_rule(UNKNOWN_MATCHER, 7).await;
    fixture.open_evaluation_gate().await;

    let report = fixture.evaluate().await;

    assert_eq!(report.candidates_created, 0);
    assert!(fixture.evaluation.all_candidates().await.is_empty());
    let runs = fixture.evaluation.all_runs().await;
    assert_eq!(runs[0].unknown_count, 1);
    assert_eq!(runs[0].matched_count, 0);
}

#[tokio::test]
async fn a_no_match_cancels_the_candidate() {
    let fixture = evaluation_app();
    seed_title(&fixture.app, &fixture.user, "Monitored", true).await;
    let rule_set_id = fixture.armed_rule(MONITORED_MATCHER, 7).await;
    fixture.open_evaluation_gate().await;
    fixture.evaluate().await;
    let opened = fixture.only_candidate().await;

    // Replace revision 1 in place so the cancel is attributable to the decision
    // and not to a supersede.
    swap_matcher_in_place(&fixture, &rule_set_id, NEVER_MATCHER, &opened).await;

    let report = fixture.evaluate().await;
    assert_eq!(report.candidates_canceled, 1);
    assert_eq!(report.candidates_superseded, 0);

    let canceled = fixture.only_candidate().await;
    assert_eq!(canceled.id, opened.id);
    assert_eq!(canceled.state, MaintenanceCandidateState::Canceled);
    assert_eq!(canceled.state_reason, candidate_reason::NO_MATCH);
}

#[tokio::test]
async fn a_rematch_after_a_cancel_opens_a_new_generation_with_a_fresh_clock() {
    let fixture = evaluation_app();
    seed_title(&fixture.app, &fixture.user, "Monitored", true).await;
    let rule_set_id = fixture.armed_rule(MONITORED_MATCHER, 7).await;
    fixture.open_evaluation_gate().await;
    fixture.evaluate().await;
    let first = fixture.only_candidate().await;

    swap_matcher_in_place(&fixture, &rule_set_id, NEVER_MATCHER, &first).await;
    fixture.evaluate().await;
    swap_matcher_in_place(&fixture, &rule_set_id, MONITORED_MATCHER, &first).await;
    let report = fixture.evaluate().await;

    assert_eq!(report.candidates_created, 1);
    let candidates = fixture.evaluation.all_candidates().await;
    assert_eq!(candidates.len(), 2, "the canceled row is kept as history");
    let fresh = candidates
        .iter()
        .find(|candidate| !candidate.state.is_terminal())
        .expect("a live candidate");
    assert_eq!(fresh.match_generation, 2);
    assert!(
        fresh.first_matched_at >= first.first_matched_at,
        "a new membership starts a new clock"
    );
    assert_ne!(fresh.id, first.id);
}

#[tokio::test]
async fn a_matcher_edit_supersedes_the_candidate_and_restarts_the_clock() {
    let fixture = evaluation_app();
    seed_title(&fixture.app, &fixture.user, "Monitored", true).await;
    let rule_set_id = fixture.armed_rule(MONITORED_MATCHER, 7).await;
    fixture.open_evaluation_gate().await;
    fixture.evaluate().await;
    let original = fixture.only_candidate().await;

    fixture
        .app
        .update_maintenance_rule_matcher(
            &fixture.user,
            &rule_set_id,
            MaintenanceMatcherDraft {
                rego_source: MONITORED_MATCHER.to_string(),
                action_spec: MaintenanceActionSpec::new(MaintenanceActionKind::DeleteTitleAndFiles),
                grace_days: 30,
            },
        )
        .await
        .expect("update matcher");

    let report = fixture.evaluate().await;
    assert_eq!(report.candidates_superseded, 1);
    assert_eq!(
        report.candidates_created, 1,
        "the same match under the new revision opens a new candidate"
    );

    let candidates = fixture.evaluation.all_candidates().await;
    let superseded = candidates
        .iter()
        .find(|candidate| candidate.id == original.id)
        .expect("the original row survives as history");
    assert_eq!(superseded.state, MaintenanceCandidateState::Canceled);
    assert_eq!(
        superseded.state_reason,
        candidate_reason::REVISION_SUPERSEDED
    );

    let fresh = candidates
        .iter()
        .find(|candidate| !candidate.state.is_terminal())
        .expect("a live candidate");
    assert_eq!(fresh.revision_number, 2);
    assert_eq!(fresh.match_generation, 2);
    assert_eq!(
        fresh.grace_days, 30,
        "the new revision's grace period applies"
    );
    assert_eq!(
        fresh.due_at,
        fresh.first_matched_at + chrono::Duration::days(30)
    );
    assert_eq!(
        fresh.action_kind,
        MaintenanceActionKind::DeleteTitleAndFiles.as_wire_str()
    );
}

#[tokio::test]
async fn an_exclusion_blocks_creation_and_closes_an_existing_candidate() {
    let fixture = evaluation_app();
    let excluded = seed_title(&fixture.app, &fixture.user, "Hands Off", true).await;
    let kept = seed_title(&fixture.app, &fixture.user, "Fair Game", true).await;
    fixture.armed_rule(MONITORED_MATCHER, 7).await;
    fixture.open_evaluation_gate().await;

    fixture.evaluate().await;
    assert_eq!(fixture.evaluation.all_candidates().await.len(), 2);

    fixture
        .app
        .exclude_maintenance_subject(
            &fixture.user,
            &excluded.id,
            None,
            Some("operator pinned".to_string()),
        )
        .await
        .expect("exclude subject");

    let report = fixture.evaluate().await;
    assert_eq!(report.candidates_excluded, 1);
    assert_eq!(
        report.titles_evaluated, 1,
        "an excluded subject is never evaluated at all"
    );

    let candidates = fixture.evaluation.all_candidates().await;
    let excluded_row = candidates
        .iter()
        .find(|candidate| candidate.title_id == excluded.id)
        .expect("excluded candidate");
    assert_eq!(excluded_row.state, MaintenanceCandidateState::Excluded);
    assert_eq!(excluded_row.state_reason, candidate_reason::EXCLUDED);

    let kept_row = candidates
        .iter()
        .find(|candidate| candidate.title_id == kept.id)
        .expect("kept candidate");
    assert_eq!(kept_row.state, MaintenanceCandidateState::Observing);

    // A third pass must not reopen the excluded subject.
    fixture.evaluate().await;
    assert_eq!(
        fixture
            .evaluation
            .all_candidates()
            .await
            .iter()
            .filter(|candidate| candidate.title_id == excluded.id && !candidate.state.is_terminal())
            .count(),
        0
    );
}

/// Reconciliation only ever visits titles the rule is *currently* scoped to, so
/// without a sweep a candidate the scope moved away from is never looked at
/// again: it stays live forever and keeps counting toward the number
/// destructive arming makes an operator acknowledge.
#[tokio::test]
async fn a_candidate_the_scope_no_longer_covers_is_canceled_by_the_next_pass() {
    let fixture = evaluation_app();
    let title = seed_title(&fixture.app, &fixture.user, "Monitored", true).await;
    let rule_set_id = fixture.armed_rule(MONITORED_MATCHER, 7).await;
    fixture.open_evaluation_gate().await;
    fixture.evaluate().await;
    let opened = fixture.only_candidate().await;
    assert_eq!(opened.state, MaintenanceCandidateState::Observing);
    assert_eq!(opened.title_id, title.id);

    // Narrow the rule onto a library the subject is not in.
    fixture
        .app
        .update_maintenance_rule_metadata(
            &fixture.user,
            &rule_set_id,
            "Stale movies".to_string(),
            String::new(),
            vec!["library-elsewhere".to_string()],
        )
        .await
        .expect("re-scope the rule");

    let report = fixture.evaluate().await;
    assert_eq!(
        report.titles_evaluated, 0,
        "the narrowed scope selects no subjects at all, which is the whole problem"
    );
    assert_eq!(
        report.candidates_canceled, 1,
        "an out-of-scope cancel is counted with the rule's other cancels: {report:?}"
    );

    let closed = fixture.only_candidate().await;
    assert_eq!(closed.state, MaintenanceCandidateState::Canceled);
    assert_eq!(closed.state_reason, candidate_reason::OUT_OF_SCOPE);
}

/// The sweep writes through the same compare-and-set every other evaluator
/// write uses, so a candidate the action handler holds a lease on is left
/// entirely alone and picked up on a later pass.
#[tokio::test]
async fn an_out_of_scope_candidate_under_an_execution_lease_is_left_alone() {
    let fixture = evaluation_app();
    seed_title(&fixture.app, &fixture.user, "Monitored", true).await;
    let rule_set_id = fixture.armed_rule(MONITORED_MATCHER, 7).await;
    fixture.open_evaluation_gate().await;
    fixture.evaluate().await;
    let opened = fixture.only_candidate().await;

    fixture
        .evaluation
        .strand_as_executing(&opened.id, Utc::now())
        .await;
    fixture
        .app
        .update_maintenance_rule_metadata(
            &fixture.user,
            &rule_set_id,
            "Stale movies".to_string(),
            String::new(),
            vec!["library-elsewhere".to_string()],
        )
        .await
        .expect("re-scope the rule");

    let report = fixture.evaluate().await;
    assert_eq!(
        report.candidates_canceled, 0,
        "a leased candidate belongs to the handler for the length of its lease: {report:?}"
    );

    let after = fixture.only_candidate().await;
    assert_eq!(
        after.state,
        MaintenanceCandidateState::Executing,
        "the evaluator must not cancel a row out from under the executor"
    );
}

#[tokio::test]
async fn a_disabled_rule_is_skipped_and_its_candidates_are_left_alone() {
    let fixture = evaluation_app();
    seed_title(&fixture.app, &fixture.user, "Monitored", true).await;
    let rule_set_id = fixture.armed_rule(MONITORED_MATCHER, 7).await;
    fixture.open_evaluation_gate().await;
    fixture.evaluate().await;
    let before = fixture.only_candidate().await;

    fixture
        .app
        .set_maintenance_rule_evaluation_mode(
            &fixture.user,
            &rule_set_id,
            MaintenanceEvaluationMode::Disabled,
        )
        .await
        .expect("disable rule");

    let report = fixture.evaluate().await;
    assert_eq!(report.rules_considered, 1);
    assert_eq!(report.rules_evaluated, 0);

    let after = fixture.only_candidate().await;
    assert_eq!(
        after, before,
        "turning a rule off must not destroy the membership it established"
    );
}

#[tokio::test]
async fn a_pass_records_a_run_with_the_counts_it_produced() {
    let fixture = evaluation_app();
    seed_title(&fixture.app, &fixture.user, "Monitored", true).await;
    seed_title(&fixture.app, &fixture.user, "Unmonitored", false).await;
    let rule_set_id = fixture.armed_rule(MONITORED_MATCHER, 7).await;
    fixture.open_evaluation_gate().await;

    fixture.evaluate().await;

    let runs = fixture.evaluation.all_runs().await;
    assert_eq!(runs.len(), 1);
    let run = &runs[0];
    assert_eq!(run.rule_set_id, rule_set_id);
    assert_eq!(run.revision_number, 1);
    assert_eq!(run.status, MaintenanceEvaluationRunStatus::Succeeded);
    assert_eq!(run.evaluated_count, 2);
    assert_eq!(run.matched_count, 1);
    assert_eq!(run.no_match_count, 1);
    assert_eq!(run.unknown_count, 0);
    assert_eq!(run.error_count, 0);
    assert!(run.finished_at.is_some());
    assert!(run.duration_ms.is_some());
    assert_eq!(run.error, None);
}

#[tokio::test]
async fn candidates_are_only_visible_through_the_gate_or_an_explicit_shadow_request() {
    let fixture = evaluation_app();
    seed_title(&fixture.app, &fixture.user, "Monitored", true).await;
    fixture.armed_rule(MONITORED_MATCHER, 7).await;
    fixture.open_evaluation_gate().await;
    fixture.evaluate().await;

    let hidden = fixture
        .app
        .list_maintenance_candidates(&fixture.user, MaintenanceCandidateFilter::default())
        .await
        .expect("list candidates");
    assert!(
        hidden.is_empty(),
        "shadow results stay dark until the operator asks for them"
    );

    let shown = fixture
        .app
        .list_maintenance_candidates(
            &fixture.user,
            MaintenanceCandidateFilter {
                include_shadow: true,
                ..Default::default()
            },
        )
        .await
        .expect("list candidates");
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].title_name, "Monitored");
    assert_eq!(shown[0].rule_name, "Stale movies");

    // Arming result display still hides shadow, because shadow is about the
    // rule, not about the instance.
    fixture
        .app
        .set_maintenance_instance_gates(
            &fixture.user,
            MaintenanceGatesUpdate {
                result_display_enabled: Some(true),
                ..Default::default()
            },
        )
        .await
        .expect("arm result display");
    let still_hidden = fixture
        .app
        .list_maintenance_candidates(&fixture.user, MaintenanceCandidateFilter::default())
        .await
        .expect("list candidates");
    assert!(still_hidden.is_empty());
}

// ── Provenance facts ────────────────────────────────────────────────────────

/// Matches a title a media request created, on behalf of two people, that an
/// operator named `admin` added. Reads every provenance fact at once so the
/// batched request lookup, the username resolution, and the contract entries
/// that let those paths validate at all are all proven by one evaluation.
const REQUESTED_MATCHER: &str = "package whatever\n\
     import rego.v1\n\n\
     match if {\n\
     \tinput.facts.requested\n\
     \tcount(input.facts.requested_by_user_ids) == 2\n\
     \tinput.facts.added_by_username == \"admin\"\n\
     }\n";

/// [`evaluation_app`] with the media-request repository swapped in and the
/// acting user present in the user store, so the provenance facts resolve
/// against something real instead of an empty roster.
async fn provenance_app() -> (EvaluationFixture, Arc<MockMediaRequestRepo>) {
    let users = Arc::new(MockUserRepo::default());
    let (app, user) = bootstrap_with_user_repo(users.clone());
    UserRepository::create(users.as_ref(), user.clone())
        .await
        .expect("seed the acting user");

    let rules = Arc::new(InMemoryMaintenanceRuleRepo::default());
    let evaluation = Arc::new(InMemoryMaintenanceEvaluationRepo::default());
    let media_requests = Arc::new(MockMediaRequestRepo::default());
    let app = app.with_test_overrides(|services| {
        services
            .with_maintenance_rule_set_store(rules.clone())
            .with_maintenance_evaluation_store(evaluation.clone())
            .with_media_files(Arc::new(MockMediaFileRepo::default()))
            .with_media_requests(media_requests.clone())
    });

    (
        EvaluationFixture {
            app,
            user,
            rules,
            evaluation,
        },
        media_requests,
    )
}

/// An approved request that created `title_id`, submitted by `submitter` and
/// seconded by `seconder`.
fn approved_request(title_id: &str, submitter: &str, seconder: &str) -> MediaRequest {
    let now = Utc::now();
    MediaRequest {
        background_url: None,
        requested_monitor_selection: None,
        requested_lease_days: None,
        approved_lease_days: None,
        decision_id: None,
        decided_by_rule_set_ids: Vec::new(),
        policy_tags: Vec::new(),
        metadata_snapshot_json: "{}".to_string(),
        rating_summary: scryer_domain::TitleRatingSummary::default(),
        id: Id::new().0,
        library_id: "library-1".to_string(),
        facet: MediaFacet::Movie,
        status: scryer_domain::MediaRequestStatus::Approved,
        identity_fingerprint: format!("fingerprint-{title_id}"),
        title: "Requested Movie".to_string(),
        sort_title: None,
        slug: None,
        poster_url: None,
        year: None,
        overview: None,
        runtime_minutes: None,
        language: None,
        content_status: None,
        requested_quality_profile_id: None,
        requested_quality_profile_name: None,
        requested_monitor_type: None,
        resolved_by_user_id: None,
        resolved_at: Some(now),
        created_title_id: Some(title_id.to_string()),
        approved_quality_profile_id: None,
        approved_quality_profile_name: None,
        external_ids: Vec::new(),
        requesters: vec![
            MediaRequestRequester {
                user_id: submitter.to_string(),
                username: "admin".to_string(),
                avatar_url: None,
                requested_at: now,
            },
            MediaRequestRequester {
                user_id: seconder.to_string(),
                username: "casey".to_string(),
                avatar_url: None,
                requested_at: now + chrono::Duration::seconds(30),
            },
        ],
        created_by_user_id: submitter.to_string(),
        created_at: now,
        updated_at: now,
    }
}

#[tokio::test]
async fn a_rule_can_match_on_who_requested_a_title() {
    let (fixture, media_requests) = provenance_app().await;
    let requested = seed_title(&fixture.app, &fixture.user, "Requested Movie", true).await;
    let scanned = seed_title(&fixture.app, &fixture.user, "Scanned Movie", true).await;

    media_requests.requests.lock().await.push(approved_request(
        &requested.id,
        &fixture.user.id,
        "user-casey",
    ));

    fixture.armed_rule(REQUESTED_MATCHER, 7).await;
    fixture.open_evaluation_gate().await;
    let report = fixture.evaluate().await;

    assert_eq!(report.titles_evaluated, 2);
    assert_eq!(report.candidates_created, 1);
    let candidate = fixture.only_candidate().await;
    assert_eq!(
        candidate.title_id, requested.id,
        "only the requested title should match; {} must not",
        scanned.id
    );
}

/// Replace the revision currently in force without bumping the revision number,
/// so a test can change what a rule decides without also triggering the
/// supersede path.
async fn swap_matcher_in_place(
    fixture: &EvaluationFixture,
    rule_set_id: &str,
    rego_source: &str,
    reference: &LifecycleCandidate,
) {
    fixture
        .rules
        .replace_revision_in_place(MaintenanceRuleRevision {
            id: Id::new().0,
            rule_set_id: rule_set_id.to_string(),
            revision_number: reference.revision_number,
            rego_source: scryer_rules::maintenance::rewrite_package_declaration(
                rego_source,
                rule_set_id,
            ),
            action_spec_json: r#"{"kind":"unmonitor_scope_keep_files","schema_version":1}"#
                .to_string(),
            grace_days: reference.grace_days,
            matcher_content_hash: reference.matcher_content_hash.clone(),
            created_by: None,
            created_at: Utc::now(),
        })
        .await;
}

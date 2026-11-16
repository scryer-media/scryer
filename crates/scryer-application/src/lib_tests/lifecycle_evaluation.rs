//! The candidate state machine.
//!
//! These tests are the specification of what the evaluator is allowed to
//! do to a candidate. Every candidate transition has a test here, and every one of
//! them also asserts what did *not* happen: the grace clock is the thing an
//! action will eventually fire on, so a test that only checks the happy
//! transition would not notice the clock being silently restarted.

use super::*;
use std::sync::atomic::AtomicBool;

use crate::lib_tests::lifecycle_rules::InMemoryLifecycleRuleRepo;
use crate::lifecycle_rules::{
    ACTION_SEQUENCE_KIND, LifecycleCandidateFilter, LifecycleGates, LifecycleGatesUpdate,
    LifecycleRuleDraft, LifecycleSourceDraft, candidate_reason,
};
use crate::ports::{LifecycleActionStepCandidateKey, LifecycleCandidateQuery};
use scryer_domain::{
    Id, LifecycleActionJobReceipt, LifecycleActionJobReceiptState, LifecycleActionRun,
    LifecycleActionStepAttempt, LifecycleActionStepKey, LifecycleActionStepRun,
    LifecycleActionStepState, LifecycleCandidate, LifecycleCandidateState, LifecycleEffectArming,
    LifecycleEvaluationMode, LifecycleEvaluationRun, LifecycleEvaluationRunStatus,
    LifecycleRuleExclusion, LifecycleRuleRevision, LifecycleRuleState,
    LifecycleSequenceTerminalMembership, LifecycleSequenceTerminalOutcome,
};

/// Matches every monitored title.
const MONITORED_SOURCE: &str = "package whatever\n\
     import rego.v1\n\n\
     match if {\n\
     \tinput.facts.monitored\n\
     }\n";

/// Matches nothing: the facet never equals this sentinel.
const NEVER_SOURCE: &str = "package whatever\n\
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
const UNKNOWN_SOURCE: &str = "package whatever\n\
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
pub(super) type LifecycleActionJobReceipts = Arc<Mutex<Vec<LifecycleActionJobReceipt>>>;

#[derive(Default)]
pub(super) struct InMemoryLifecycleEvaluationRepo {
    pub(super) fail_action_completion: AtomicBool,
    pub(super) fail_next_due_read: AtomicBool,
    /// How many cross-level conflict markers were written.
    pub(super) actions_conflict_writes: std::sync::atomic::AtomicUsize,
    /// How many batches those markers were written in.
    pub(super) actions_conflict_batches: std::sync::atomic::AtomicUsize,
    /// When set to a subject id, recording a continuing match for that
    /// subject's candidate fails, as a lost database write would.
    pub(super) fail_record_match_for_subject: std::sync::Mutex<Option<String>>,
    candidates: Mutex<Vec<LifecycleCandidate>>,
    exclusions: Mutex<Vec<LifecycleRuleExclusion>>,
    runs: Mutex<Vec<LifecycleEvaluationRun>>,
    action_runs: Mutex<Vec<LifecycleActionRun>>,
    action_steps: Mutex<Vec<LifecycleActionStepRun>>,
    action_step_attempts: Mutex<Vec<LifecycleActionStepAttempt>>,
    pub(super) action_job_receipts: LifecycleActionJobReceipts,
    sequence_terminal_memberships: Mutex<Vec<LifecycleSequenceTerminalMembership>>,
    /// When set to a candidate id, the next finished action run or step also
    /// moves that candidate out of `executing`, exactly as a concurrent pass
    /// reclaiming a stale lease would.
    ///
    /// Production has no such hook — only the lease writes that state — but the
    /// window between a worker finishing its action and recording its result is
    /// precisely where a lost update would land, so a test needs a way to write
    /// into it.
    steal_lease_on_finish: Mutex<Option<String>>,
}

impl InMemoryLifecycleEvaluationRepo {
    pub(super) async fn all_candidates(&self) -> Vec<LifecycleCandidate> {
        self.candidates.lock().await.clone()
    }

    /// Every recorded step attempt, oldest first.
    pub(super) async fn all_step_attempts(&self) -> Vec<LifecycleActionStepAttempt> {
        self.action_step_attempts.lock().await.clone()
    }

    /// Move every recorded step attempt `by` into the past, as time passing
    /// would.
    pub(super) async fn age_step_attempts(&self, by: chrono::Duration) {
        for attempt in self.action_step_attempts.lock().await.iter_mut() {
            attempt.started_at -= by;
        }
    }

    pub(super) async fn all_runs(&self) -> Vec<LifecycleEvaluationRun> {
        self.runs.lock().await.clone()
    }

    /// Move every recorded evaluation start `by` into the past, so a hold an
    /// action pass writes afterwards is certainly later than it.
    pub(super) async fn age_evaluation_runs(&self, by: chrono::Duration) {
        for run in self.runs.lock().await.iter_mut() {
            run.started_at -= by;
        }
    }

    /// Record that each evaluated rule's next evaluation began now, without
    /// running it, as the evaluation that opens a scheduled fire would. For a
    /// test that pins only the action half of consecutive fires: a candidate
    /// held by one fire's action pass is examined again by the next.
    pub(super) async fn begin_next_evaluation(&self) {
        let mut runs = self.runs.lock().await;
        let mut latest: HashMap<String, LifecycleEvaluationRun> = HashMap::new();
        for run in runs.iter() {
            let newer = latest
                .get(&run.rule_set_id)
                .is_none_or(|seen| seen.started_at < run.started_at);
            if newer {
                latest.insert(run.rule_set_id.clone(), run.clone());
            }
        }
        let now = Utc::now();
        for (_, mut run) in latest {
            run.id = scryer_domain::Id::new().0;
            run.started_at = now;
            runs.push(run);
        }
    }

    pub(super) async fn all_action_runs(&self) -> Vec<LifecycleActionRun> {
        self.action_runs.lock().await.clone()
    }

    /// Move every candidate's grace deadline into the past, as the clock
    /// passing the deadline would.
    pub(super) async fn end_every_grace(&self) {
        for candidate in self.candidates.lock().await.iter_mut() {
            candidate.due_at = Utc::now() - chrono::Duration::seconds(1);
        }
    }

    /// Durable step rows for one step id, across every candidate.
    pub(super) async fn action_steps_with_id(&self, step_id: &str) -> Vec<LifecycleActionStepRun> {
        self.action_steps
            .lock()
            .await
            .iter()
            .filter(|step| step.key.step_id == step_id)
            .cloned()
            .collect()
    }

    /// Seed a terminal marker for evaluator-only reconciliation tests. The
    /// production finish path validates durable step evidence before it writes
    /// a marker; these tests isolate pagination and raw no-match handling.
    pub(super) async fn seed_sequence_terminal_membership(
        &self,
        membership: LifecycleSequenceTerminalMembership,
    ) {
        self.sequence_terminal_memberships
            .lock()
            .await
            .push(membership);
    }

    /// Drop a candidate's recorded decision, the shape a candidate written
    /// before rules declared their actions carries.
    pub(super) async fn forget_decided_actions(&self, id: &str) {
        let mut rows = self.candidates.lock().await;
        let candidate = rows
            .iter_mut()
            .find(|candidate| candidate.id == id)
            .expect("candidate exists");
        candidate.decided_actions_json = None;
    }

    /// Overwrite the decision recorded on a candidate, as a row written by
    /// some other path would carry it.
    pub(super) async fn force_decided_actions(
        &self,
        id: &str,
        actions: &scryer_rules::lifecycle_actions::LifecycleActions,
    ) {
        let mut rows = self.candidates.lock().await;
        let candidate = rows
            .iter_mut()
            .find(|candidate| candidate.id == id)
            .expect("candidate exists");
        candidate.decided_actions_json =
            Some(crate::lifecycle_rules::declared_actions::decided_actions_to_json(actions));
    }

    /// Record execution attempts already spent on a candidate.
    pub(super) async fn set_action_attempts(&self, id: &str, attempts: i64) {
        let mut rows = self.candidates.lock().await;
        let candidate = rows
            .iter_mut()
            .find(|candidate| candidate.id == id)
            .expect("candidate exists");
        candidate.action_attempts = attempts;
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
        candidate.state = LifecycleCandidateState::Executing;
        candidate.state_reason = "execution_leased".to_string();
        candidate.updated_at = updated_at;
    }

    /// The armed lease theft, if any: a concurrent pass decided this lease
    /// was stale and took it while the action was still running.
    async fn steal_armed_lease(&self) {
        if let Some(candidate_id) = self.steal_lease_on_finish.lock().await.take() {
            let mut rows = self.candidates.lock().await;
            if let Some(candidate) = rows
                .iter_mut()
                .find(|candidate| candidate.id == candidate_id)
            {
                candidate.state = LifecycleCandidateState::Due;
                candidate.state_reason = "execution_lease_reclaimed".to_string();
            }
        }
    }

    /// Arm the lease theft described on [`Self::steal_lease_on_finish`].
    pub(super) async fn steal_lease_when_the_next_run_finishes(&self, candidate_id: &str) {
        *self.steal_lease_on_finish.lock().await = Some(candidate_id.to_string());
    }
}

#[async_trait]
impl LifecycleCandidateRepository for InMemoryLifecycleEvaluationRepo {
    async fn get_active_subject_candidate(
        &self,
        rule_set_id: &str,
        subject_kind: &str,
        subject_id: &str,
    ) -> AppResult<Option<LifecycleCandidate>> {
        Ok(self
            .candidates
            .lock()
            .await
            .iter()
            .find(|candidate| {
                candidate.rule_set_id == rule_set_id
                    && candidate.subject_kind == subject_kind
                    && candidate.subject_id == subject_id
                    && !candidate.state.is_terminal()
            })
            .cloned())
    }

    async fn list_active_subject_candidates(
        &self,
        rule_set_id: &str,
        subject_kind: &str,
        subject_ids: &[String],
    ) -> AppResult<Vec<LifecycleCandidate>> {
        let wanted: std::collections::HashSet<&str> =
            subject_ids.iter().map(String::as_str).collect();
        Ok(self
            .candidates
            .lock()
            .await
            .iter()
            .filter(|candidate| {
                candidate.rule_set_id == rule_set_id
                    && candidate.subject_kind == subject_kind
                    && wanted.contains(candidate.subject_id.as_str())
                    && !candidate.state.is_terminal()
            })
            .cloned()
            .collect())
    }

    async fn get_candidates_by_ids(&self, ids: &[String]) -> AppResult<Vec<LifecycleCandidate>> {
        let wanted: std::collections::HashSet<&str> = ids.iter().map(String::as_str).collect();
        Ok(self
            .candidates
            .lock()
            .await
            .iter()
            .filter(|candidate| wanted.contains(candidate.id.as_str()))
            .cloned()
            .collect())
    }

    async fn list_candidates(
        &self,
        query: &LifecycleCandidateQuery,
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

    async fn max_subject_match_generation(
        &self,
        rule_set_id: &str,
        subject_kind: &str,
        subject_id: &str,
    ) -> AppResult<i64> {
        Ok(self
            .candidates
            .lock()
            .await
            .iter()
            .filter(|candidate| {
                candidate.rule_set_id == rule_set_id
                    && candidate.subject_kind == subject_kind
                    && candidate.subject_id == subject_id
            })
            .map(|candidate| candidate.match_generation)
            .max()
            .unwrap_or(0))
    }

    async fn create_candidate(&self, candidate: &LifecycleCandidate) -> AppResult<()> {
        if candidate.action_kind == ACTION_SEQUENCE_KIND
            && self
                .sequence_terminal_memberships
                .lock()
                .await
                .iter()
                .any(|membership| {
                    membership.rule_set_id == candidate.rule_set_id
                        && membership.revision_number == candidate.revision_number
                        && membership.subject_kind == candidate.subject_kind
                        && membership.subject_id == candidate.subject_id
                        && membership.released_at.is_none()
                })
        {
            return Err(AppError::Validation(
                "lifecycle sequence membership is terminal until a confirmed no-match".into(),
            ));
        }
        let mut rows = self.candidates.lock().await;
        if rows.iter().any(|row| {
            row.rule_set_id == candidate.rule_set_id
                && row.subject_kind == candidate.subject_kind
                && row.subject_id == candidate.subject_id
                && !row.state.is_terminal()
        }) {
            return Err(AppError::Validation(format!(
                "lifecycle rule {} already has an active candidate for title {}",
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
        if self
            .fail_record_match_for_subject
            .lock()
            .unwrap()
            .as_deref()
            == Some(candidate.subject_id.as_str())
        {
            return Err(AppError::Repository(
                "injected candidate match write failure".into(),
            ));
        }
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
        state: LifecycleCandidateState,
        state_reason: &str,
        expected_states: &[LifecycleCandidateState],
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

    async fn set_candidate_actions_conflicts(
        &self,
        changes: &[(String, Option<scryer_domain::LifecycleActionsConflict>)],
        expected_states: &[LifecycleCandidateState],
        updated_at: DateTime<Utc>,
    ) -> AppResult<usize> {
        self.actions_conflict_writes
            .fetch_add(changes.len(), std::sync::atomic::Ordering::SeqCst);
        self.actions_conflict_batches
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut rows = self.candidates.lock().await;
        let mut written = 0;
        for (id, conflict) in changes {
            if let Some(candidate) = rows.iter_mut().find(|candidate| &candidate.id == id)
                && expected_states.contains(&candidate.state)
            {
                candidate.actions_conflict = conflict.clone();
                candidate.updated_at = updated_at;
                written += 1;
            }
        }
        Ok(written)
    }

    async fn finish_leased_candidate(
        &self,
        id: &str,
        expected_lease_updated_at: DateTime<Utc>,
        state: LifecycleCandidateState,
        state_reason: &str,
        finished_at: DateTime<Utc>,
    ) -> AppResult<bool> {
        let mut rows = self.candidates.lock().await;
        let candidate = rows
            .iter_mut()
            .find(|candidate| candidate.id == id)
            .ok_or_else(|| AppError::NotFound(id.to_string()))?;
        if candidate.state != LifecycleCandidateState::Executing
            || candidate.updated_at != expected_lease_updated_at
        {
            return Ok(false);
        }
        candidate.state = state;
        candidate.state_reason = state_reason.to_string();
        candidate.last_evaluated_at = finished_at;
        candidate.updated_at = finished_at;
        Ok(true)
    }

    async fn count_candidates_by_state(
        &self,
        rule_set_id: &str,
    ) -> AppResult<Vec<(LifecycleCandidateState, i64)>> {
        let mut counts: HashMap<String, (LifecycleCandidateState, i64)> = HashMap::new();
        for candidate in self.candidates.lock().await.iter() {
            if candidate.rule_set_id != rule_set_id {
                continue;
            }
            let entry = counts
                .entry(candidate.state.as_storage_str().to_string())
                .or_insert((candidate.state, 0));
            entry.1 += 1;
        }
        let mut rows: Vec<(LifecycleCandidateState, i64)> = counts.into_values().collect();
        rows.sort_by_key(|(state, _)| state.as_storage_str());
        Ok(rows)
    }

    async fn list_due_candidates(
        &self,
        rule_set_id: &str,
        due_before: DateTime<Utc>,
        stale_before: DateTime<Utc>,
        held_after: Option<DateTime<Utc>>,
        after: Option<&crate::LifecycleDueCursor>,
        limit: usize,
    ) -> AppResult<Vec<LifecycleCandidate>> {
        if self.fail_next_due_read.swap(false, Ordering::SeqCst) {
            return Err(AppError::Repository(
                "transient due-candidate read failure".into(),
            ));
        }
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
                        LifecycleCandidateState::Observing
                            | LifecycleCandidateState::PendingAction
                            | LifecycleCandidateState::Due
                            | LifecycleCandidateState::Blocked
                    ) || (candidate.state == LifecycleCandidateState::Executing
                        && candidate.updated_at < stale_before))
                    && !held_after.is_some_and(|held_after| {
                        candidate.state == LifecycleCandidateState::Blocked
                            && candidate.updated_at > held_after
                    })
                    && after.is_none_or(|after| {
                        (candidate.due_at, candidate.id.as_str())
                            > (after.due_at, after.id.as_str())
                    })
            })
            .cloned()
            .collect();
        rows.sort_by(|left, right| (left.due_at, &left.id).cmp(&(right.due_at, &right.id)));
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
        let leasable = candidate.state == LifecycleCandidateState::Due
            || (candidate.state == LifecycleCandidateState::Executing
                && candidate.updated_at < stale_before);
        if !leasable {
            return Ok(false);
        }
        candidate.state = LifecycleCandidateState::Executing;
        candidate.state_reason = "execution_leased".to_string();
        candidate.updated_at = updated_at;
        Ok(true)
    }
}

#[async_trait]
impl LifecycleActionRunRepository for InMemoryLifecycleEvaluationRepo {
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
        if self.fail_action_completion.load(Ordering::SeqCst) {
            return Err(AppError::Repository(
                "synthetic action result persistence failure".into(),
            ));
        }
        {
            let mut rows = self.action_runs.lock().await;
            let stored = rows
                .iter_mut()
                .find(|stored| stored.id == run.id)
                .ok_or_else(|| AppError::NotFound(run.id.clone()))?;
            *stored = run.clone();
        }

        self.steal_armed_lease().await;
        Ok(())
    }

    async fn latest_scoped_deletion_action_run(
        &self,
        candidate_id: &str,
        match_generation: i64,
        action_kind: &str,
    ) -> AppResult<Option<LifecycleActionRun>> {
        Ok(self
            .action_runs
            .lock()
            .await
            .iter()
            .filter(|run| {
                run.candidate_id == candidate_id
                    && run.match_generation == match_generation
                    && run.action_kind == action_kind
                    && run.detail.contains("\"scoped_deletion\":")
            })
            .max_by(|a, b| (a.started_at, &a.id).cmp(&(b.started_at, &b.id)))
            .cloned())
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

    async fn list_sequence_history_action_runs(
        &self,
        rule_set_id: Option<&str>,
        candidate_id: Option<&str>,
        limit: usize,
    ) -> AppResult<Vec<LifecycleActionRun>> {
        let mut rows: Vec<LifecycleActionRun> = self
            .action_runs
            .lock()
            .await
            .iter()
            .filter(|run| {
                rule_set_id.is_none_or(|rule_set_id| run.rule_set_id == rule_set_id)
                    && candidate_id.is_none_or(|candidate_id| run.candidate_id == candidate_id)
                    && run.action_kind == ACTION_SEQUENCE_KIND
                    && run.detail.contains("\"maintenance_sequence_history\":true")
            })
            .cloned()
            .collect();
        rows.sort_by_key(|run| std::cmp::Reverse(run.started_at));
        rows.truncate(limit.min(200));
        Ok(rows)
    }

    async fn list_non_sequence_action_runs(
        &self,
        rule_set_id: Option<&str>,
        candidate_id: Option<&str>,
        limit: usize,
    ) -> AppResult<Vec<LifecycleActionRun>> {
        let mut rows: Vec<LifecycleActionRun> = self
            .action_runs
            .lock()
            .await
            .iter()
            .filter(|run| {
                rule_set_id.is_none_or(|rule_set_id| run.rule_set_id == rule_set_id)
                    && candidate_id.is_none_or(|candidate_id| run.candidate_id == candidate_id)
                    && run.action_kind != ACTION_SEQUENCE_KIND
            })
            .cloned()
            .collect();
        rows.sort_by_key(|run| std::cmp::Reverse(run.started_at));
        rows.truncate(limit.min(200));
        Ok(rows)
    }
}

#[async_trait]
impl LifecycleActionStepRepository for InMemoryLifecycleEvaluationRepo {
    async fn get_action_step(
        &self,
        key: &LifecycleActionStepKey,
    ) -> AppResult<Option<LifecycleActionStepRun>> {
        Ok(self
            .action_steps
            .lock()
            .await
            .iter()
            .find(|step| &step.key == key)
            .cloned())
    }

    async fn claim_action_step(
        &self,
        step: &LifecycleActionStepRun,
        stale_before: DateTime<Utc>,
        lease_id: &str,
        leased_at: DateTime<Utc>,
    ) -> AppResult<LifecycleActionStepClaim> {
        if step.state != LifecycleActionStepState::Running || step.attempt < 0 {
            return Err(AppError::Validation(
                "a lifecycle action step claim must start a non-negative running attempt".into(),
            ));
        }
        if !in_memory_action_step_identity_is_consistent(step) {
            return Err(AppError::Validation(
                "a lifecycle action step claim has an invalid target identity".into(),
            ));
        }
        let mut claimed = step.clone();
        claimed.lease_id = Some(lease_id.to_string());
        claimed.updated_at = leased_at;
        claimed.finished_at = None;
        claimed.hold_reason = None;
        claimed.error = None;
        let (result, started_attempt) = {
            let mut rows = self.action_steps.lock().await;
            if let Some(index) = rows.iter().position(|stored| stored.key == claimed.key) {
                let existing = rows[index].clone();
                if !in_memory_action_step_identity_is_consistent(&existing)
                    || existing.sequence_content_hash != claimed.sequence_content_hash
                    || existing.step_kind != claimed.step_kind
                {
                    return Err(AppError::Validation(
                        "lifecycle action step does not match its persisted sequence identity"
                            .into(),
                    ));
                }
                if existing.state.is_completion_success() {
                    (LifecycleActionStepClaim::Completed(existing), false)
                } else if existing.state == LifecycleActionStepState::Running
                    && existing.updated_at >= stale_before
                {
                    (LifecycleActionStepClaim::Busy, false)
                } else if existing.state == LifecycleActionStepState::Running {
                    let mut reclaimed = existing;
                    reclaimed.lease_id = claimed.lease_id;
                    reclaimed.lease_expires_at = claimed.lease_expires_at;
                    reclaimed.updated_at = claimed.updated_at;
                    rows[index] = reclaimed.clone();
                    (LifecycleActionStepClaim::Claimed(reclaimed), false)
                } else if existing.state == LifecycleActionStepState::Held {
                    let mut resumed = existing;
                    resumed.state = LifecycleActionStepState::Running;
                    resumed.lease_id = claimed.lease_id;
                    resumed.lease_expires_at = claimed.lease_expires_at;
                    resumed.hold_reason = None;
                    resumed.error = None;
                    resumed.updated_at = claimed.updated_at;
                    resumed.finished_at = None;
                    rows[index] = resumed.clone();
                    (LifecycleActionStepClaim::Claimed(resumed), true)
                } else {
                    if claimed.attempt <= existing.attempt {
                        return Err(AppError::Validation(
                            "a retried lifecycle action step must advance its attempt".into(),
                        ));
                    }
                    let mut retried = existing;
                    retried.state = LifecycleActionStepState::Running;
                    retried.attempt = claimed.attempt;
                    retried.lease_id = claimed.lease_id;
                    retried.lease_expires_at = claimed.lease_expires_at;
                    retried.hold_reason = None;
                    retried.error = None;
                    retried.updated_at = claimed.updated_at;
                    retried.finished_at = None;
                    rows[index] = retried.clone();
                    (LifecycleActionStepClaim::Claimed(retried), true)
                }
            } else {
                rows.push(claimed.clone());
                (LifecycleActionStepClaim::Claimed(claimed), true)
            }
        };
        if started_attempt && let LifecycleActionStepClaim::Claimed(step) = &result {
            self.action_step_attempts
                .lock()
                .await
                .push(in_memory_action_step_attempt(step));
        }
        Ok(result)
    }

    async fn checkpoint_action_step(
        &self,
        step: &LifecycleActionStepRun,
        expected_lease_id: &str,
    ) -> AppResult<bool> {
        update_in_memory_action_step(self, step, expected_lease_id, false).await
    }

    async fn finish_action_step(
        &self,
        step: &LifecycleActionStepRun,
        expected_lease_id: &str,
    ) -> AppResult<bool> {
        if !step.state.is_attempt_finished() || step.state == LifecycleActionStepState::Running {
            return Err(AppError::Validation(
                "a lifecycle action step finish must carry an attempt result".into(),
            ));
        }
        let finished = update_in_memory_action_step(self, step, expected_lease_id, true).await?;
        if finished {
            self.steal_armed_lease().await;
        }
        Ok(finished)
    }

    async fn list_action_steps(
        &self,
        candidate_id: &str,
        match_generation: i64,
        revision_number: i64,
    ) -> AppResult<Vec<LifecycleActionStepRun>> {
        self.list_action_steps_for_candidates(&[LifecycleActionStepCandidateKey {
            candidate_id: candidate_id.to_string(),
            match_generation,
            revision_number,
        }])
        .await
    }

    async fn list_action_steps_for_candidates(
        &self,
        candidates: &[LifecycleActionStepCandidateKey],
    ) -> AppResult<Vec<LifecycleActionStepRun>> {
        if candidates.len() > 100 {
            return Err(AppError::Validation(
                "lifecycle sequence progress accepts at most 100 candidates per page".into(),
            ));
        }
        let mut steps: Vec<_> = self
            .action_steps
            .lock()
            .await
            .iter()
            .filter(|step| {
                candidates.iter().any(|candidate| {
                    candidate.candidate_id == step.key.candidate_id
                        && candidate.match_generation == step.key.match_generation
                        && candidate.revision_number == step.key.revision_number
                })
            })
            .cloned()
            .collect();
        steps.sort_by(|left, right| {
            left.key
                .candidate_id
                .cmp(&right.key.candidate_id)
                .then_with(|| left.key.step_id.cmp(&right.key.step_id))
        });
        Ok(steps)
    }

    async fn list_step_acts_since(
        &self,
        step_kinds: &[&str],
        since: chrono::DateTime<chrono::Utc>,
        limit: usize,
    ) -> AppResult<Vec<crate::LifecycleStepAct>> {
        let steps = self.action_steps.lock().await;
        let mut acts: Vec<crate::LifecycleStepAct> = Vec::new();
        for attempt in self.action_step_attempts.lock().await.iter() {
            if attempt.started_at < since
                || attempt.state != scryer_domain::LifecycleActionStepState::Succeeded
            {
                continue;
            }
            for step in steps.iter().filter(|step| {
                step.key == attempt.key && step_kinds.contains(&step.step_kind.as_str())
            }) {
                let act = crate::LifecycleStepAct {
                    rule_set_id: step.rule_set_id.clone(),
                    title_id: step.title_id.clone(),
                    step_kind: step.step_kind.clone(),
                };
                if !acts.contains(&act) && acts.len() < limit {
                    acts.push(act);
                }
            }
        }
        Ok(acts)
    }

    async fn list_action_job_receipts(
        &self,
        key: &LifecycleActionStepKey,
    ) -> AppResult<Vec<LifecycleActionJobReceipt>> {
        let mut receipts: Vec<_> = self
            .action_job_receipts
            .lock()
            .await
            .iter()
            .filter(|receipt| &receipt.key == key)
            .cloned()
            .collect();
        receipts.sort_by_key(|receipt| receipt.dispatch_attempt);
        Ok(receipts)
    }

    async fn list_action_job_receipts_for_steps(
        &self,
        steps: &[LifecycleActionStepKey],
    ) -> AppResult<Vec<LifecycleActionJobReceipt>> {
        if steps.len() > 700 {
            return Err(AppError::Validation(
                "lifecycle sequence receipt projection accepts at most 700 steps".into(),
            ));
        }
        let mut receipts: Vec<_> = self
            .action_job_receipts
            .lock()
            .await
            .iter()
            .filter(|receipt| steps.contains(&receipt.key))
            .cloned()
            .collect();
        receipts.sort_by(|left, right| {
            left.key
                .candidate_id
                .cmp(&right.key.candidate_id)
                .then_with(|| left.key.step_id.cmp(&right.key.step_id))
                .then_with(|| left.dispatch_attempt.cmp(&right.dispatch_attempt))
        });
        Ok(receipts)
    }

    async fn claim_action_job_dispatch(
        &self,
        receipt: &LifecycleActionJobReceipt,
    ) -> AppResult<LifecycleActionJobReceiptClaim> {
        receipt
            .validate_schema()
            .map_err(|error| AppError::Validation(error.to_string()))?;
        if receipt.state != LifecycleActionJobReceiptState::Dispatching {
            return Err(AppError::Validation(
                "a generic lifecycle job dispatch must begin dispatching".into(),
            ));
        }
        let mut receipts = self.action_job_receipts.lock().await;
        if let Some(existing) = receipts.iter().find(|stored| {
            stored.key == receipt.key && stored.dispatch_attempt == receipt.dispatch_attempt
        }) {
            return Ok(LifecycleActionJobReceiptClaim::Existing(existing.clone()));
        }
        receipts.push(receipt.clone());
        Ok(LifecycleActionJobReceiptClaim::Claimed(receipt.clone()))
    }

    async fn transition_action_job_receipt(
        &self,
        transition: &LifecycleActionJobReceiptTransition,
    ) -> AppResult<bool> {
        if transition.expected_states.is_empty() {
            return Err(AppError::Validation(
                "a lifecycle job receipt transition must name expected states".into(),
            ));
        }
        let mut receipts = self.action_job_receipts.lock().await;
        let Some(receipt) = receipts.iter_mut().find(|receipt| {
            receipt.key == transition.key && receipt.dispatch_attempt == transition.dispatch_attempt
        }) else {
            return Ok(false);
        };
        if !transition.expected_states.contains(&receipt.state) {
            return Ok(false);
        }
        receipt.state = transition.next_state;
        receipt.job_run_id = transition.job_run_id.clone();
        receipt.reconciliation_evidence_json = transition.reconciliation_evidence_json.clone();
        receipt.updated_at = transition.updated_at;
        receipt
            .validate_schema()
            .map_err(|error| AppError::Validation(error.to_string()))?;
        Ok(true)
    }
}

fn in_memory_action_step_identity_is_consistent(step: &LifecycleActionStepRun) -> bool {
    let Ok(target) = serde_json::from_str::<serde_json::Value>(&step.target_identity_json) else {
        return false;
    };
    let text = |name: &str, expected: &str| {
        target.get(name).and_then(serde_json::Value::as_str) == Some(expected)
    };
    target
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        == Some(1)
        && text("rule_set_id", &step.rule_set_id)
        && text("candidate_id", &step.key.candidate_id)
        && target
            .get("match_generation")
            .and_then(serde_json::Value::as_i64)
            == Some(step.key.match_generation)
        && target
            .get("revision_number")
            .and_then(serde_json::Value::as_i64)
            == Some(step.key.revision_number)
        && text("title_id", &step.title_id)
        && text("subject_kind", &step.subject_kind)
        && text("subject_id", &step.subject_id)
        && text("step_id", &step.key.step_id)
        && text("step_kind", &step.step_kind)
        && text("sequence_content_hash", &step.sequence_content_hash)
}

fn in_memory_action_step_attempt(step: &LifecycleActionStepRun) -> LifecycleActionStepAttempt {
    LifecycleActionStepAttempt {
        id: Id::new().0,
        key: step.key.clone(),
        attempt: step.attempt,
        state: LifecycleActionStepState::Running,
        intent_json: step.intent_json.clone(),
        before_state_json: step.before_state_json.clone(),
        target_identity_json: step.target_identity_json.clone(),
        provenance_json: step.provenance_json.clone(),
        hold_reason: None,
        error: None,
        started_at: step.updated_at,
        finished_at: None,
        created_at: step.updated_at,
        updated_at: step.updated_at,
    }
}

async fn update_in_memory_action_step(
    repository: &InMemoryLifecycleEvaluationRepo,
    step: &LifecycleActionStepRun,
    expected_lease_id: &str,
    finish: bool,
) -> AppResult<bool> {
    let mut steps = repository.action_steps.lock().await;
    let Some(stored) = steps.iter_mut().find(|stored| {
        stored.key == step.key
            && stored.state == LifecycleActionStepState::Running
            && stored.lease_id.as_deref() == Some(expected_lease_id)
    }) else {
        return Ok(false);
    };
    if finish {
        if !in_memory_action_step_identity_is_consistent(stored) {
            return Err(AppError::Repository(
                "lifecycle action step has inconsistent target identity".into(),
            ));
        }
        stored.before_state_json = step.before_state_json.clone();
        stored.target_identity_json = step.target_identity_json.clone();
        stored.provenance_json = step.provenance_json.clone();
        stored.state = step.state;
        stored.lease_id = None;
        stored.lease_expires_at = None;
        stored.hold_reason = step.hold_reason.clone();
        stored.error = step.error.clone();
        stored.updated_at = step.updated_at;
        stored.finished_at = step.finished_at;
    } else {
        stored.before_state_json = step.before_state_json.clone();
        stored.target_identity_json = step.target_identity_json.clone();
        stored.provenance_json = step.provenance_json.clone();
        stored.updated_at = step.updated_at;
    }
    let persisted = stored.clone();
    drop(steps);
    let mut attempts = repository.action_step_attempts.lock().await;
    let Some(attempt) = attempts
        .iter_mut()
        .rev()
        .find(|attempt| attempt.key == persisted.key && attempt.finished_at.is_none())
    else {
        return Err(AppError::Repository(
            "lifecycle action step lost its current immutable attempt".into(),
        ));
    };
    attempt.before_state_json = persisted.before_state_json.clone();
    attempt.target_identity_json = persisted.target_identity_json.clone();
    attempt.provenance_json = persisted.provenance_json.clone();
    attempt.updated_at = persisted.updated_at;
    if finish {
        attempt.state = persisted.state;
        attempt.hold_reason = persisted.hold_reason.clone();
        attempt.error = persisted.error.clone();
        attempt.finished_at = persisted.finished_at;
    }
    Ok(true)
}

#[async_trait]
impl LifecycleSequenceCompletionRepository for InMemoryLifecycleEvaluationRepo {
    async fn get_active_sequence_completion(
        &self,
        rule_set_id: &str,
        revision_number: i64,
        subject_kind: &str,
        subject_id: &str,
    ) -> AppResult<Option<LifecycleSequenceTerminalMembership>> {
        Ok(self
            .sequence_terminal_memberships
            .lock()
            .await
            .iter()
            .find(|membership| {
                membership.rule_set_id == rule_set_id
                    && membership.revision_number == revision_number
                    && membership.subject_kind == subject_kind
                    && membership.subject_id == subject_id
                    && membership.released_at.is_none()
            })
            .cloned())
    }

    async fn finish_sequence_terminal_membership_and_candidate(
        &self,
        membership: &LifecycleSequenceTerminalMembership,
        expected_candidate_state: LifecycleCandidateState,
        expected_candidate_updated_at: DateTime<Utc>,
        terminal_candidate_state: LifecycleCandidateState,
        state_reason: &str,
        finished_at: DateTime<Utc>,
    ) -> AppResult<bool> {
        if !matches!(
            (membership.outcome, terminal_candidate_state),
            (
                LifecycleSequenceTerminalOutcome::Succeeded,
                LifecycleCandidateState::Succeeded
            ) | (
                LifecycleSequenceTerminalOutcome::Failed,
                LifecycleCandidateState::Failed
            )
        ) {
            return Err(AppError::Validation(
                "lifecycle sequence terminal outcome does not match candidate state".into(),
            ));
        }
        if membership.expected_step_ids.len() > 7
            || membership.expected_step_ids.iter().any(|id| id.is_empty())
            || membership
                .expected_step_ids
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
                != membership.expected_step_ids.len()
        {
            return Err(AppError::Validation(
                "lifecycle sequence terminal marker has invalid expected step ids".into(),
            ));
        }
        let steps = self.action_steps.lock().await.clone();
        let step_state = |step_id: &str| {
            steps
                .iter()
                .find(|step| {
                    step.key.candidate_id == membership.candidate_id
                        && step.key.match_generation == membership.match_generation
                        && step.key.revision_number == membership.revision_number
                        && step.key.step_id == step_id
                })
                .map(|step| step.state)
        };
        let terminal_index = membership.terminal_step_id.as_deref().and_then(|id| {
            membership
                .expected_step_ids
                .iter()
                .position(|expected| expected == id)
        });
        let expected_completed = match membership.outcome {
            LifecycleSequenceTerminalOutcome::Succeeded => {
                if membership.terminal_step_id.is_some()
                    || membership.completed_step_ids != membership.expected_step_ids
                {
                    return Err(AppError::Validation(
                        "successful lifecycle sequence marker lacks complete step evidence".into(),
                    ));
                }
                membership.expected_step_ids.as_slice()
            }
            LifecycleSequenceTerminalOutcome::Failed => {
                let Some(index) = terminal_index else {
                    return Err(AppError::Validation(
                        "failed lifecycle sequence marker lacks its failed step id".into(),
                    ));
                };
                let expected = &membership.expected_step_ids[..=index];
                if membership.completed_step_ids != expected {
                    return Err(AppError::Validation(
                        "failed lifecycle sequence marker does not preserve its step prefix".into(),
                    ));
                }
                expected
            }
        };
        for (index, step_id) in expected_completed.iter().enumerate() {
            let Some(state) = step_state(step_id) else {
                return Err(AppError::Validation(
                    "lifecycle terminal marker references a missing step".into(),
                ));
            };
            let expected_failure = membership.outcome == LifecycleSequenceTerminalOutcome::Failed
                && index + 1 == expected_completed.len();
            if (expected_failure && state != LifecycleActionStepState::Failed)
                || (!expected_failure && !state.is_completion_success())
            {
                return Err(AppError::Validation(
                    "lifecycle terminal marker does not match durable step evidence".into(),
                ));
            }
        }
        let mut memberships = self.sequence_terminal_memberships.lock().await;
        if let Some(existing) = memberships
            .iter()
            .find(|existing| existing.candidate_id == membership.candidate_id)
        {
            return Ok(same_terminal_membership(existing, membership));
        }
        if memberships.iter().any(|existing| {
            existing.rule_set_id == membership.rule_set_id
                && existing.revision_number == membership.revision_number
                && existing.subject_kind == membership.subject_kind
                && existing.subject_id == membership.subject_id
                && existing.released_at.is_none()
        }) {
            return Ok(false);
        }
        let mut candidates = self.candidates.lock().await;
        let Some(candidate) = candidates.iter_mut().find(|candidate| {
            candidate.id == membership.candidate_id
                && candidate.revision_number == membership.revision_number
                && candidate.match_generation == membership.match_generation
        }) else {
            return Ok(false);
        };
        if candidate.state != expected_candidate_state
            || candidate.updated_at != expected_candidate_updated_at
        {
            return Ok(false);
        }
        memberships.push(membership.clone());
        candidate.state = terminal_candidate_state;
        candidate.state_reason = state_reason.to_string();
        candidate.updated_at = finished_at;
        Ok(true)
    }

    async fn release_sequence_completion_on_confirmed_non_match(
        &self,
        rule_set_id: &str,
        revision_number: i64,
        subject_kind: &str,
        subject_id: &str,
        released_at: DateTime<Utc>,
    ) -> AppResult<bool> {
        let mut memberships = self.sequence_terminal_memberships.lock().await;
        let Some(membership) = memberships.iter_mut().find(|membership| {
            membership.rule_set_id == rule_set_id
                && membership.revision_number == revision_number
                && membership.subject_kind == subject_kind
                && membership.subject_id == subject_id
                && membership.released_at.is_none()
                && membership.created_at <= released_at
        }) else {
            return Ok(false);
        };
        membership.released_at = Some(released_at);
        Ok(true)
    }
}

fn same_terminal_membership(
    left: &LifecycleSequenceTerminalMembership,
    right: &LifecycleSequenceTerminalMembership,
) -> bool {
    left.candidate_id == right.candidate_id
        && left.rule_set_id == right.rule_set_id
        && left.revision_number == right.revision_number
        && left.source_content_hash == right.source_content_hash
        && left.title_id == right.title_id
        && left.subject_kind == right.subject_kind
        && left.subject_id == right.subject_id
        && left.match_generation == right.match_generation
        && left.sequence_content_hash == right.sequence_content_hash
        && left.outcome == right.outcome
        && left.expected_step_ids == right.expected_step_ids
        && left.completed_step_ids == right.completed_step_ids
        && left.terminal_step_id == right.terminal_step_id
}

#[async_trait]
impl LifecycleExclusionRepository for InMemoryLifecycleEvaluationRepo {
    async fn list_exclusions(
        &self,
        rule_set_id: Option<&str>,
    ) -> AppResult<Vec<LifecycleRuleExclusion>> {
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

    async fn get_exclusion(&self, id: &str) -> AppResult<Option<LifecycleRuleExclusion>> {
        Ok(self
            .exclusions
            .lock()
            .await
            .iter()
            .find(|exclusion| exclusion.id == id)
            .cloned())
    }

    async fn create_exclusion(&self, exclusion: &LifecycleRuleExclusion) -> AppResult<()> {
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
impl LifecycleEvaluationRunRepository for InMemoryLifecycleEvaluationRepo {
    async fn start_evaluation_run(&self, run: &LifecycleEvaluationRun) -> AppResult<()> {
        self.runs.lock().await.push(run.clone());
        Ok(())
    }

    async fn finish_evaluation_run(&self, run: &LifecycleEvaluationRun) -> AppResult<()> {
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
    ) -> AppResult<Vec<LifecycleEvaluationRun>> {
        let mut rows: Vec<LifecycleEvaluationRun> = self
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

    async fn rule_schedule_anchors(
        &self,
        rule_set_ids: &[String],
    ) -> AppResult<std::collections::HashMap<String, crate::LifecycleRuleScheduleAnchor>> {
        let wanted: std::collections::HashSet<&str> =
            rule_set_ids.iter().map(String::as_str).collect();
        let mut anchors =
            std::collections::HashMap::<String, crate::LifecycleRuleScheduleAnchor>::new();
        for run in self.runs.lock().await.iter() {
            if !wanted.contains(run.rule_set_id.as_str()) {
                continue;
            }
            let anchor = anchors.entry(run.rule_set_id.clone()).or_default();
            anchor.last_run_at = anchor.last_run_at.max(Some(run.started_at));
        }
        for candidate in self.candidates.lock().await.iter() {
            if wanted.contains(candidate.rule_set_id.as_str())
                && candidate.state == LifecycleCandidateState::Executing
            {
                anchors
                    .entry(candidate.rule_set_id.clone())
                    .or_default()
                    .in_flight = true;
            }
        }
        Ok(anchors)
    }
}

// ── Fixture ─────────────────────────────────────────────────────────────────

struct EvaluationFixture {
    app: AppUseCase,
    user: User,
    rules: Arc<InMemoryLifecycleRuleRepo>,
    evaluation: Arc<InMemoryLifecycleEvaluationRepo>,
}

fn evaluation_app() -> EvaluationFixture {
    let (app, user) = bootstrap();
    let rules = Arc::new(InMemoryLifecycleRuleRepo::default());
    let evaluation = Arc::new(InMemoryLifecycleEvaluationRepo::default());
    let media_files = Arc::new(MockMediaFileRepo::default());
    let app = app.with_test_overrides(|services| {
        services
            .with_lifecycle_rule_set_store(rules.clone())
            .with_lifecycle_evaluation_store(evaluation.clone())
            .with_media_files(media_files)
    });
    EvaluationFixture {
        app,
        user,
        rules,
        evaluation,
    }
}

fn draft(rego_source: &str, grace_days: i64) -> LifecycleRuleDraft {
    LifecycleRuleDraft {
        max_titles_per_run: Some(100),
        name: "Stale movies".to_string(),
        description: String::new(),
        rego_source: crate::lib_tests::heads_source(
            rego_source,
            &[scryer_rules::lifecycle_actions::LifecycleActionHead::Unmonitor],
        ),
        grace_days,
        library_ids: Vec::new(),
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
    /// Create a rule and move it to dry run, which is always two deliberate
    /// steps.
    async fn armed_rule(&self, rego_source: &str, grace_days: i64) -> String {
        let created = self
            .app
            .create_lifecycle_rule_set(&self.user, draft(rego_source, grace_days))
            .await
            .expect("create rule set");
        super::lifecycle_rules::set_rule_state(
            &self.app,
            &self.user,
            &created.rule_set.id,
            LifecycleRuleState::DryRun,
            None,
        )
        .await;
        created.rule_set.id
    }

    /// Evaluation alone, with candidate display left off.
    async fn open_evaluation_gate(&self) {
        super::lifecycle_rules::store_lifecycle_gates(
            &self.app,
            LifecycleGates {
                evaluation_enabled: true,
                ..Default::default()
            },
        )
        .await;
    }

    async fn evaluate(&self) -> crate::lifecycle_rules::LifecycleEvaluationReport {
        self.app
            .run_lifecycle_rule_evaluation_job()
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
async fn the_switches_default_off_and_write_their_gates_in_pairs() {
    let fixture = evaluation_app();

    let initial = fixture
        .app
        .lifecycle_instance_gates(&fixture.user)
        .await
        .expect("read gates");
    assert_eq!(
        initial,
        LifecycleGates::default(),
        "an unconfigured instance must evaluate and act on nothing"
    );
    assert!(!initial.enabled());
    assert!(initial.actions_paused());

    let enabled = fixture
        .app
        .set_lifecycle_instance_gates(
            &fixture.user,
            LifecycleGatesUpdate {
                enabled: Some(true),
                ..Default::default()
            },
        )
        .await
        .expect("set gates");
    assert!(enabled.evaluation_enabled && enabled.result_display_enabled);
    assert!(
        !enabled.reversible_effects_enabled && !enabled.destructive_effects_enabled,
        "enabling lifecycle must not unpause actions"
    );

    // Omitting a switch must leave its gates exactly as stored.
    let unpaused = fixture
        .app
        .set_lifecycle_instance_gates(
            &fixture.user,
            LifecycleGatesUpdate {
                actions_paused: Some(false),
                ..Default::default()
            },
        )
        .await
        .expect("set gates");
    assert!(unpaused.enabled());
    assert!(unpaused.reversible_effects_enabled && unpaused.destructive_effects_enabled);
    assert!(!unpaused.actions_paused());

    let paused = fixture
        .app
        .set_lifecycle_instance_gates(
            &fixture.user,
            LifecycleGatesUpdate {
                enabled: Some(false),
                actions_paused: Some(true),
            },
        )
        .await
        .expect("set gates");
    assert_eq!(paused, LifecycleGates::default());
}

#[test]
fn actions_read_as_paused_only_when_no_effect_can_run() {
    let reversible_only = LifecycleGates {
        reversible_effects_enabled: true,
        ..Default::default()
    };
    assert!(!reversible_only.actions_paused());
    let display_only = LifecycleGates {
        result_display_enabled: true,
        ..Default::default()
    };
    assert!(!display_only.enabled());
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
        .lifecycle_instance_gates(&outsider)
        .await
        .expect_err("reading gates must be permission gated");
    assert!(
        matches!(
            read,
            AppError::Unauthorized(_) | AppError::UnauthorizedReason { .. }
        ),
        "{read:?}"
    );

    let write = fixture
        .app
        .set_lifecycle_instance_gates(&outsider, LifecycleGatesUpdate::default())
        .await
        .expect_err("arming gates must be permission gated");
    assert!(
        matches!(
            write,
            AppError::Unauthorized(_) | AppError::UnauthorizedReason { .. }
        ),
        "{write:?}"
    );
}

#[tokio::test]
async fn the_evaluation_gate_makes_a_run_a_no_op() {
    let fixture = evaluation_app();
    seed_title(&fixture.app, &fixture.user, "Monitored", true).await;
    fixture.armed_rule(MONITORED_SOURCE, 7).await;

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

// ── Rule state ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn each_state_writes_its_mode_and_arming_and_creation_still_ships_dark() {
    let fixture = evaluation_app();
    let created = fixture
        .app
        .create_lifecycle_rule_set(&fixture.user, draft(MONITORED_SOURCE, 7))
        .await
        .expect("create rule set");
    assert!(!created.rule_set.enabled, "rules are created disabled");
    assert_eq!(
        crate::lifecycle_rules::lifecycle_rule_state(&created),
        LifecycleRuleState::Disabled
    );

    for (state, mode, arming) in [
        (
            LifecycleRuleState::DryRun,
            LifecycleEvaluationMode::Observe,
            LifecycleEffectArming::None,
        ),
        (
            LifecycleRuleState::Enabled,
            LifecycleEvaluationMode::Observe,
            LifecycleEffectArming::Reversible,
        ),
        (
            LifecycleRuleState::DryRun,
            LifecycleEvaluationMode::Observe,
            LifecycleEffectArming::None,
        ),
        (
            LifecycleRuleState::Enabled,
            LifecycleEvaluationMode::Observe,
            LifecycleEffectArming::Reversible,
        ),
        (
            LifecycleRuleState::Disabled,
            LifecycleEvaluationMode::Disabled,
            LifecycleEffectArming::None,
        ),
    ] {
        let detail = fixture
            .app
            .set_lifecycle_rule_state(&fixture.user, &created.rule_set.id, state, None, None)
            .await
            .expect("set state");
        assert_eq!(detail.rule_set.evaluation_mode, mode, "{state:?}");
        assert_eq!(detail.rule_set.effect_arming, arming, "{state:?}");
        assert_eq!(
            detail.rule_set.enabled,
            mode != LifecycleEvaluationMode::Disabled
        );
        assert_eq!(crate::lifecycle_rules::lifecycle_rule_state(&detail), state);
        let stored = fixture
            .app
            .services
            .customization
            .lifecycle_rule_sets
            .get_rule_set(&created.rule_set.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            stored, detail.rule_set,
            "the detail mirrors the store write"
        );
        assert_eq!(
            detail.rule_set.current_revision_number, 1,
            "a state change is not a matcher change, so no revision is appended"
        );
    }
}

#[tokio::test]
async fn an_edit_drops_an_enabled_rule_to_dry_run_until_it_is_enabled_again() {
    let fixture = evaluation_app();
    let created = fixture
        .app
        .create_lifecycle_rule_set(&fixture.user, draft(MONITORED_SOURCE, 7))
        .await
        .expect("create rule set");
    let id = created.rule_set.id;
    super::lifecycle_rules::set_rule_state(
        &fixture.app,
        &fixture.user,
        &id,
        LifecycleRuleState::Enabled,
        None,
    )
    .await;

    let edited = fixture
        .app
        .update_lifecycle_rule_source(
            &fixture.user,
            &id,
            LifecycleSourceDraft {
                rego_source: crate::lib_tests::heads_source(
                    MONITORED_SOURCE,
                    &[scryer_rules::lifecycle_actions::LifecycleActionHead::DeleteTitle],
                ),
                grace_days: 3,
                max_titles_per_run: None,
            },
        )
        .await
        .expect("edit matcher");
    assert_eq!(edited.rule_set.effect_arming, LifecycleEffectArming::None);
    assert!(edited.rule_set.destructive_rearm_required);
    assert_eq!(
        crate::lifecycle_rules::lifecycle_rule_state(&edited),
        LifecycleRuleState::DryRun
    );
    let stored = fixture
        .app
        .services
        .customization
        .lifecycle_rule_sets
        .get_rule_set(&id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        stored, edited.rule_set,
        "the detail mirrors the store write"
    );

    // A second edit keeps the flag; it only reflects that the rule was enabled.
    let again = fixture
        .app
        .update_lifecycle_rule_source(
            &fixture.user,
            &id,
            LifecycleSourceDraft {
                rego_source: crate::lib_tests::heads_source(
                    MONITORED_SOURCE,
                    &[scryer_rules::lifecycle_actions::LifecycleActionHead::DeleteTitle],
                ),
                grace_days: 4,
                max_titles_per_run: None,
            },
        )
        .await
        .expect("edit matcher");
    assert!(again.rule_set.destructive_rearm_required);

    let reenabled = super::lifecycle_rules::set_rule_state(
        &fixture.app,
        &fixture.user,
        &id,
        LifecycleRuleState::Enabled,
        Some(0),
    )
    .await;
    assert!(!reenabled.rule_set.destructive_rearm_required);
    assert_eq!(
        crate::lifecycle_rules::lifecycle_rule_state(&reenabled),
        LifecycleRuleState::Enabled
    );

    // A rule that was never enabled is not flagged by an edit.
    let dry = super::lifecycle_rules::set_rule_state(
        &fixture.app,
        &fixture.user,
        &id,
        LifecycleRuleState::DryRun,
        None,
    )
    .await;
    assert!(!dry.rule_set.destructive_rearm_required);
    let edited_dry = fixture
        .app
        .update_lifecycle_rule_source(
            &fixture.user,
            &id,
            LifecycleSourceDraft {
                rego_source: crate::lib_tests::heads_source(
                    MONITORED_SOURCE,
                    &[scryer_rules::lifecycle_actions::LifecycleActionHead::DeleteTitle],
                ),
                grace_days: 5,
                max_titles_per_run: None,
            },
        )
        .await
        .expect("edit matcher");
    assert!(!edited_dry.rule_set.destructive_rearm_required);
}

// ── The state machine ───────────────────────────────────────────────────────

#[tokio::test]
async fn a_first_match_opens_a_candidate_and_starts_the_clock() {
    let fixture = evaluation_app();
    let title = seed_title(&fixture.app, &fixture.user, "Monitored", true).await;
    let rule_set_id = fixture.armed_rule(MONITORED_SOURCE, 7).await;
    fixture.open_evaluation_gate().await;

    let report = fixture.evaluate().await;
    assert_eq!(report.candidates_created, 1);
    assert_eq!(report.titles_evaluated, 1);

    let candidate = fixture.only_candidate().await;
    assert_eq!(candidate.rule_set_id, rule_set_id);
    assert_eq!(candidate.title_id, title.id);
    assert_eq!(candidate.state, LifecycleCandidateState::Observing);
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
        crate::lifecycle_rules::ACTION_SEQUENCE_KIND,
        "a rule that declares its actions records the sequence kind"
    );
}

#[tokio::test]
async fn a_zero_day_grace_is_due_the_moment_it_matches() {
    let fixture = evaluation_app();
    seed_title(&fixture.app, &fixture.user, "Monitored", true).await;
    fixture.armed_rule(MONITORED_SOURCE, 0).await;
    fixture.open_evaluation_gate().await;

    fixture.evaluate().await;

    let candidate = fixture.only_candidate().await;
    assert_eq!(candidate.due_at, candidate.first_matched_at);
}

#[tokio::test]
async fn a_repeat_match_advances_only_the_last_match_and_never_the_clock() {
    let fixture = evaluation_app();
    seed_title(&fixture.app, &fixture.user, "Monitored", true).await;
    fixture.armed_rule(MONITORED_SOURCE, 7).await;
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
    assert_eq!(second.state, LifecycleCandidateState::Observing);
}

#[tokio::test]
async fn an_unknown_decision_holds_the_candidate_instead_of_cancelling_it() {
    let fixture = evaluation_app();
    seed_title(&fixture.app, &fixture.user, "Monitored", true).await;
    let rule_set_id = fixture.armed_rule(MONITORED_SOURCE, 7).await;
    fixture.open_evaluation_gate().await;
    fixture.evaluate().await;
    let before = fixture.only_candidate().await;

    // The matcher is swapped in place rather than through a revision bump, so
    // what this test observes is the unknown branch and not the supersede one.
    swap_source_in_place(&fixture, &rule_set_id, UNKNOWN_SOURCE, &before).await;

    let report = fixture.evaluate().await;
    assert_eq!(report.candidates_held, 1);
    assert_eq!(report.candidates_canceled, 0, "unknown must never cancel");
    assert_eq!(
        report.candidates_created, 0,
        "an unknown decision must not open a candidate"
    );

    let after = fixture.only_candidate().await;
    assert_eq!(after.id, before.id);
    assert_eq!(after.state, LifecycleCandidateState::Observing);
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
    fixture.armed_rule(UNKNOWN_SOURCE, 7).await;
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
    let rule_set_id = fixture.armed_rule(MONITORED_SOURCE, 7).await;
    fixture.open_evaluation_gate().await;
    fixture.evaluate().await;
    let opened = fixture.only_candidate().await;

    // Replace revision 1 in place so the cancel is attributable to the decision
    // and not to a supersede.
    swap_source_in_place(&fixture, &rule_set_id, NEVER_SOURCE, &opened).await;

    let report = fixture.evaluate().await;
    assert_eq!(report.candidates_canceled, 1);
    assert_eq!(report.candidates_superseded, 0);

    let canceled = fixture.only_candidate().await;
    assert_eq!(canceled.id, opened.id);
    assert_eq!(canceled.state, LifecycleCandidateState::Canceled);
    assert_eq!(canceled.state_reason, candidate_reason::NO_MATCH);
}

#[tokio::test]
async fn a_rematch_after_a_cancel_opens_a_new_generation_with_a_fresh_clock() {
    let fixture = evaluation_app();
    seed_title(&fixture.app, &fixture.user, "Monitored", true).await;
    let rule_set_id = fixture.armed_rule(MONITORED_SOURCE, 7).await;
    fixture.open_evaluation_gate().await;
    fixture.evaluate().await;
    let first = fixture.only_candidate().await;

    swap_source_in_place(&fixture, &rule_set_id, NEVER_SOURCE, &first).await;
    fixture.evaluate().await;
    swap_source_in_place(&fixture, &rule_set_id, MONITORED_SOURCE, &first).await;
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
async fn a_source_edit_supersedes_the_candidate_and_restarts_the_clock() {
    let fixture = evaluation_app();
    seed_title(&fixture.app, &fixture.user, "Monitored", true).await;
    let rule_set_id = fixture.armed_rule(MONITORED_SOURCE, 7).await;
    fixture.open_evaluation_gate().await;
    fixture.evaluate().await;
    let original = fixture.only_candidate().await;

    fixture
        .app
        .update_lifecycle_rule_source(
            &fixture.user,
            &rule_set_id,
            LifecycleSourceDraft {
                rego_source: crate::lib_tests::heads_source(
                    MONITORED_SOURCE,
                    &[scryer_rules::lifecycle_actions::LifecycleActionHead::DeleteTitle],
                ),
                grace_days: 30,
                max_titles_per_run: None,
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
    assert_eq!(superseded.state, LifecycleCandidateState::Canceled);
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
        crate::lifecycle_rules::ACTION_SEQUENCE_KIND
    );
}

#[tokio::test]
async fn an_exclusion_blocks_creation_and_closes_an_existing_candidate() {
    let fixture = evaluation_app();
    let excluded = seed_title(&fixture.app, &fixture.user, "Hands Off", true).await;
    let kept = seed_title(&fixture.app, &fixture.user, "Fair Game", true).await;
    fixture.armed_rule(MONITORED_SOURCE, 7).await;
    fixture.open_evaluation_gate().await;

    fixture.evaluate().await;
    assert_eq!(fixture.evaluation.all_candidates().await.len(), 2);

    fixture
        .app
        .exclude_lifecycle_subject(
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
    assert_eq!(excluded_row.state, LifecycleCandidateState::Excluded);
    assert_eq!(excluded_row.state_reason, candidate_reason::EXCLUDED);

    let kept_row = candidates
        .iter()
        .find(|candidate| candidate.title_id == kept.id)
        .expect("kept candidate");
    assert_eq!(kept_row.state, LifecycleCandidateState::Observing);

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
    let rule_set_id = fixture.armed_rule(MONITORED_SOURCE, 7).await;
    fixture.open_evaluation_gate().await;
    fixture.evaluate().await;
    let opened = fixture.only_candidate().await;
    assert_eq!(opened.state, LifecycleCandidateState::Observing);
    assert_eq!(opened.title_id, title.id);

    // Narrow the rule onto a library the subject is not in.
    fixture
        .app
        .update_lifecycle_rule_metadata(
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
    assert_eq!(closed.state, LifecycleCandidateState::Canceled);
    assert_eq!(closed.state_reason, candidate_reason::OUT_OF_SCOPE);
}

/// The sweep writes through the same compare-and-set every other evaluator
/// write uses, so a candidate the action handler holds a lease on is left
/// entirely alone and picked up on a later pass.
#[tokio::test]
async fn an_out_of_scope_candidate_under_an_execution_lease_is_left_alone() {
    let fixture = evaluation_app();
    seed_title(&fixture.app, &fixture.user, "Monitored", true).await;
    let rule_set_id = fixture.armed_rule(MONITORED_SOURCE, 7).await;
    fixture.open_evaluation_gate().await;
    fixture.evaluate().await;
    let opened = fixture.only_candidate().await;

    fixture
        .evaluation
        .strand_as_executing(&opened.id, Utc::now())
        .await;
    fixture
        .app
        .update_lifecycle_rule_metadata(
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
        LifecycleCandidateState::Executing,
        "the evaluator must not cancel a row out from under the executor"
    );
}

#[tokio::test]
async fn a_disabled_rule_is_skipped_and_its_candidates_are_left_alone() {
    let fixture = evaluation_app();
    seed_title(&fixture.app, &fixture.user, "Monitored", true).await;
    let rule_set_id = fixture.armed_rule(MONITORED_SOURCE, 7).await;
    fixture.open_evaluation_gate().await;
    fixture.evaluate().await;
    let before = fixture.only_candidate().await;

    super::lifecycle_rules::set_rule_state(
        &fixture.app,
        &fixture.user,
        &rule_set_id,
        LifecycleRuleState::Disabled,
        None,
    )
    .await;

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
    let rule_set_id = fixture.armed_rule(MONITORED_SOURCE, 7).await;
    fixture.open_evaluation_gate().await;

    fixture.evaluate().await;

    let runs = fixture.evaluation.all_runs().await;
    assert_eq!(runs.len(), 1);
    let run = &runs[0];
    assert_eq!(run.rule_set_id, rule_set_id);
    assert_eq!(run.revision_number, 1);
    assert_eq!(run.status, LifecycleEvaluationRunStatus::Succeeded);
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
async fn candidates_are_only_visible_while_lifecycle_is_enabled() {
    let fixture = evaluation_app();
    seed_title(&fixture.app, &fixture.user, "Monitored", true).await;
    fixture.armed_rule(MONITORED_SOURCE, 7).await;
    fixture.open_evaluation_gate().await;
    fixture.evaluate().await;

    let hidden = fixture
        .app
        .list_lifecycle_candidates(&fixture.user, LifecycleCandidateFilter::default())
        .await
        .expect("list candidates");
    assert!(
        hidden.is_empty(),
        "results stay dark while result display is off"
    );

    fixture
        .app
        .set_lifecycle_instance_gates(
            &fixture.user,
            LifecycleGatesUpdate {
                enabled: Some(true),
                ..Default::default()
            },
        )
        .await
        .expect("enable lifecycle");
    let shown = fixture
        .app
        .list_lifecycle_candidates(&fixture.user, LifecycleCandidateFilter::default())
        .await
        .expect("list candidates");
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].title_name, "Monitored");
    assert_eq!(shown[0].rule_name, "Stale movies");
}

// ── Provenance facts ────────────────────────────────────────────────────────

/// Matches a title a media request created, on behalf of two people, that an
/// operator named `admin` added. Reads every provenance fact at once so the
/// batched request lookup, the username resolution, and the contract entries
/// that let those paths validate at all are all proven by one evaluation.
const REQUESTED_SOURCE: &str = "package whatever\n\
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

    let rules = Arc::new(InMemoryLifecycleRuleRepo::default());
    let evaluation = Arc::new(InMemoryLifecycleEvaluationRepo::default());
    let media_requests = Arc::new(MockMediaRequestRepo::default());
    let app = app.with_test_overrides(|services| {
        services
            .with_lifecycle_rule_set_store(rules.clone())
            .with_lifecycle_evaluation_store(evaluation.clone())
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
        origin: Default::default(),
        created_at: now,
        updated_at: now,
        requested_required_audio_languages: None,
        requested_preferred_audio_languages: None,
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

    fixture.armed_rule(REQUESTED_SOURCE, 7).await;
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
async fn swap_source_in_place(
    fixture: &EvaluationFixture,
    rule_set_id: &str,
    rego_source: &str,
    reference: &LifecycleCandidate,
) {
    fixture
        .rules
        .replace_revision_in_place(LifecycleRuleRevision {
            id: Id::new().0,
            rule_set_id: rule_set_id.to_string(),
            revision_number: reference.revision_number,
            rego_source: scryer_rules::lifecycle::rewrite_package_declaration(
                &crate::lib_tests::heads_source(
                    rego_source,
                    &[scryer_rules::lifecycle_actions::LifecycleActionHead::Unmonitor],
                ),
                rule_set_id,
            ),
            action_spec_json: r#"{"schema_version":3,"heads":["unmonitor"],"destructive":false,"quality_profiles":[],"add_tags":[],"remove_tags":[]}"#
                .to_string(),
            grace_days: reference.grace_days,
            source_content_hash: reference.source_content_hash.clone(),
            created_by: None,
            created_at: Utc::now(),
            builder_model_json: None,
        })
        .await;
}

// ── Levels ──────────────────────────────────────────────────────────────────

/// A monitored series with one season of two episodes.
async fn seed_series_with_season(fixture: &EvaluationFixture) -> Title {
    let title = fixture
        .app
        .add_title(
            &fixture.user,
            NewTitle {
                name: "Synthetic Levels".to_string(),
                facet: MediaFacet::Series,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .expect("create title");
    let season = fixture
        .app
        .create_collection(
            &fixture.user,
            title.id.clone(),
            "season".into(),
            "1".into(),
            None,
            None,
            None,
            None,
        )
        .await
        .expect("season");
    for number in 1..=2 {
        fixture
            .app
            .create_episode(
                &fixture.user,
                title.id.clone(),
                Some(season.id.clone()),
                "standard".into(),
                Some(number.to_string()),
                Some("1".into()),
                None,
                Some(format!("Episode {number}")),
                Some("2020-01-01".into()),
                None,
                false,
                false,
            )
            .await
            .expect("episode");
    }
    title
}

async fn armed_source(fixture: &EvaluationFixture, rego_source: &str) -> String {
    let created = fixture
        .app
        .create_lifecycle_rule_set(
            &fixture.user,
            LifecycleRuleDraft {
                rego_source: rego_source.to_string(),
                ..draft(MONITORED_SOURCE, 0)
            },
        )
        .await
        .expect("create rule set");
    super::lifecycle_rules::set_rule_state(
        &fixture.app,
        &fixture.user,
        &created.rule_set.id,
        LifecycleRuleState::DryRun,
        None,
    )
    .await;
    created.rule_set.id
}

fn candidate_levels(candidates: &[LifecycleCandidate]) -> Vec<String> {
    let mut levels: Vec<String> = candidates
        .iter()
        .map(|candidate| candidate.subject_kind.clone())
        .collect();
    levels.sort();
    levels
}

/// A rule whose heads all act on the title decides only the title: its
/// seasons and episodes are never decided, so none of them opens a
/// candidate even though the title's facts match.
#[tokio::test]
async fn a_title_only_rule_decides_no_season_or_episode() {
    let fixture = evaluation_app();
    let title = seed_series_with_season(&fixture).await;
    armed_source(
        &fixture,
        "package rules\nimport rego.v1\n\nunmonitor if input.facts.monitored\n",
    )
    .await;
    fixture.open_evaluation_gate().await;
    fixture.evaluate().await;

    let candidates = fixture.evaluation.all_candidates().await;
    assert_eq!(
        candidate_levels(&candidates),
        vec!["title"],
        "{candidates:?}"
    );
    assert_eq!(candidates[0].subject_id, title.id);
}

/// A rule with a title head and a season head decides the title through
/// the title head and the season through the season head, in one pass; no
/// episode is decided because no head acts on one.
#[tokio::test]
async fn a_mixed_level_rule_opens_a_candidate_at_each_level_it_acts_on() {
    let fixture = evaluation_app();
    let title = seed_series_with_season(&fixture).await;
    armed_source(
        &fixture,
        "package rules\nimport rego.v1\n\n\
         notify := \"Fixture review\" if input.facts.monitored\n\n\
         unmonitor_season if input.facts.monitored\n",
    )
    .await;
    fixture.open_evaluation_gate().await;
    fixture.evaluate().await;

    let candidates = fixture.evaluation.all_candidates().await;
    assert_eq!(
        candidate_levels(&candidates),
        vec!["season", "title"],
        "{candidates:?}"
    );
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.title_id == title.id)
    );
}

// ── Conflicts across levels ─────────────────────────────────────────────────

/// A show with two seasons: season 1 holds two episodes, season 2 one.
struct TwoSeasons {
    title: Title,
    season_one: String,
    season_one_episode: String,
    season_two: String,
    season_two_episode: String,
}

async fn seed_series_with_two_seasons(fixture: &EvaluationFixture) -> TwoSeasons {
    let title = fixture
        .app
        .add_title(
            &fixture.user,
            NewTitle {
                name: "Synthetic Cross Levels".to_string(),
                facet: MediaFacet::Series,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .expect("create title");
    let mut seasons = Vec::new();
    for (season_number, episode_count) in [(1, 2), (2, 1)] {
        let season = fixture
            .app
            .create_collection(
                &fixture.user,
                title.id.clone(),
                "season".into(),
                season_number.to_string(),
                None,
                None,
                None,
                None,
            )
            .await
            .expect("season");
        let mut episodes = Vec::new();
        for number in 1..=episode_count {
            let episode = fixture
                .app
                .create_episode(
                    &fixture.user,
                    title.id.clone(),
                    Some(season.id.clone()),
                    "standard".into(),
                    Some(number.to_string()),
                    Some(season_number.to_string()),
                    None,
                    Some(format!("Episode {season_number}-{number}")),
                    Some("2020-01-01".into()),
                    None,
                    false,
                    false,
                )
                .await
                .expect("episode");
            episodes.push(episode.id);
        }
        seasons.push((season.id, episodes));
    }
    let (season_two, season_two_episodes) = seasons.pop().expect("season 2");
    let (season_one, season_one_episodes) = seasons.pop().expect("season 1");
    TwoSeasons {
        title,
        season_one,
        season_one_episode: season_one_episodes[0].clone(),
        season_two,
        season_two_episode: season_two_episodes[0].clone(),
    }
}

fn live_candidate<'a>(
    candidates: &'a [LifecycleCandidate],
    subject_id: &str,
) -> &'a LifecycleCandidate {
    candidates
        .iter()
        .find(|candidate| candidate.subject_id == subject_id && !candidate.state.is_terminal())
        .unwrap_or_else(|| panic!("a live candidate for {subject_id}: {candidates:?}"))
}

fn conflict_writes(fixture: &EvaluationFixture) -> usize {
    fixture
        .evaluation
        .actions_conflict_writes
        .load(std::sync::atomic::Ordering::SeqCst)
}

/// A title the rule monitors while it deletes one of its seasons' files:
/// both candidates carry a marker that names the other side, and the
/// season the rule does not decide carries none.
#[tokio::test]
async fn a_title_head_and_a_season_head_that_conflict_mark_both_candidates() {
    let fixture = evaluation_app();
    let show = seed_series_with_two_seasons(&fixture).await;
    armed_source(
        &fixture,
        "package rules\nimport rego.v1\n\n\
         monitor if input.facts.monitored\n\n\
         delete_season_files if input.subject.season_number == 1\n",
    )
    .await;
    fixture.open_evaluation_gate().await;
    fixture.evaluate().await;

    let candidates = fixture.evaluation.all_candidates().await;
    assert_eq!(candidates.len(), 2, "{candidates:?}");
    let title = live_candidate(&candidates, &show.title.id);
    assert_eq!(
        title.actions_conflict,
        Some(scryer_domain::LifecycleActionsConflict {
            head: "monitor".into(),
            other_subject_kind: "season".into(),
            other_subject_id: show.season_one.clone(),
            other_head: "delete_season_files".into(),
        })
    );
    let season = live_candidate(&candidates, &show.season_one);
    assert_eq!(
        season.actions_conflict,
        Some(scryer_domain::LifecycleActionsConflict {
            head: "delete_season_files".into(),
            other_subject_kind: "title".into(),
            other_subject_id: show.title.id.clone(),
            other_head: "monitor".into(),
        })
    );
    // The marker is the only thing that changed: the decision, the state and
    // the clock are the ones a candidate without a conflict gets.
    assert_eq!(season.state, LifecycleCandidateState::Observing);
    assert_eq!(season.state_reason, candidate_reason::FIRST_MATCH);
    assert_eq!(conflict_writes(&fixture), 2);

    // A second pass that decides the same writes nothing.
    fixture.evaluate().await;
    assert_eq!(conflict_writes(&fixture), 2);
}

/// A season pairs with its own episodes only: `monitor_season` on season 1
/// conflicts with `delete_episode_files` on an episode of season 1, and not
/// with the same head on an episode of season 2.
#[tokio::test]
async fn a_season_head_conflicts_only_with_its_own_episodes() {
    for same_season in [true, false] {
        let fixture = evaluation_app();
        let show = seed_series_with_two_seasons(&fixture).await;
        let episode = if same_season {
            show.season_one_episode.clone()
        } else {
            show.season_two_episode.clone()
        };
        armed_source(
            &fixture,
            &format!(
                "package rules\nimport rego.v1\n\n\
                 monitor_season if input.subject.season_number == 1\n\n\
                 delete_episode_files if input.subject.episode_id == {episode:?}\n"
            ),
        )
        .await;
        fixture.open_evaluation_gate().await;
        fixture.evaluate().await;

        let candidates = fixture.evaluation.all_candidates().await;
        assert_eq!(candidates.len(), 2, "{candidates:?}");
        let season = live_candidate(&candidates, &show.season_one);
        let episode_candidate = live_candidate(&candidates, &episode);
        if same_season {
            assert_eq!(
                season
                    .actions_conflict
                    .as_ref()
                    .map(|conflict| conflict.other_subject_id.as_str()),
                Some(episode.as_str())
            );
            assert_eq!(
                episode_candidate.actions_conflict.as_ref().map(|conflict| (
                    conflict.other_head.as_str(),
                    conflict.other_subject_id.as_str()
                )),
                Some(("monitor_season", show.season_one.as_str()))
            );
        } else {
            assert_eq!(season.actions_conflict, None, "{season:?}");
            assert_eq!(episode_candidate.actions_conflict, None);
            assert_eq!(conflict_writes(&fixture), 0);
        }
        assert!(
            candidates
                .iter()
                .all(|candidate| candidate.subject_id != show.season_two)
        );
    }
}

/// The marker goes once the rule stops deciding one side of the pair: the
/// title no longer matches, and the season's marker is cleared on that pass.
#[tokio::test]
async fn a_cleared_conflict_clears_the_marker_on_the_next_evaluation() {
    let fixture = evaluation_app();
    let show = seed_series_with_two_seasons(&fixture).await;
    armed_source(
        &fixture,
        "package rules\nimport rego.v1\n\n\
         monitor if not input.facts.monitored\n\n\
         notify := \"Synthetic review\" if input.subject.title_id != \"\"\n\n\
         delete_season_files if input.subject.season_number == 1\n",
    )
    .await;
    fixture
        .app
        .set_title_monitored(&fixture.user, &show.title.id, false)
        .await
        .expect("unmonitor title");
    fixture.open_evaluation_gate().await;
    fixture.evaluate().await;
    let candidates = fixture.evaluation.all_candidates().await;
    assert!(
        live_candidate(&candidates, &show.title.id)
            .actions_conflict
            .is_some()
    );
    assert!(
        live_candidate(&candidates, &show.season_one)
            .actions_conflict
            .is_some()
    );

    fixture
        .app
        .set_title_monitored(&fixture.user, &show.title.id, true)
        .await
        .expect("monitor title");
    fixture.evaluate().await;
    let candidates = fixture.evaluation.all_candidates().await;
    // The title now decides only `notify`, a different intent, so its old
    // candidate closed and a new one opened without a marker.
    let title = live_candidate(&candidates, &show.title.id);
    assert_eq!(title.actions_conflict, None, "{title:?}");
    let season = live_candidate(&candidates, &show.season_one);
    assert_eq!(season.actions_conflict, None, "{season:?}");
}

/// Mixes that do not contradict each other are never marked: deleting a
/// title's files and a season's, or unmonitoring a title while deleting an
/// episode's files.
#[tokio::test]
async fn allowed_mixes_across_levels_are_not_marked() {
    for source in [
        "package rules\nimport rego.v1\n\n\
         delete_files if input.facts.monitored\n\n\
         delete_season_files if input.subject.season_number == 1\n",
        "package rules\nimport rego.v1\n\n\
         unmonitor if input.facts.monitored\n\n\
         delete_episode_files if input.subject.season_number == 1\n",
    ] {
        let fixture = evaluation_app();
        seed_series_with_two_seasons(&fixture).await;
        armed_source(&fixture, source).await;
        fixture.open_evaluation_gate().await;
        fixture.evaluate().await;
        let candidates = fixture.evaluation.all_candidates().await;
        assert!(candidates.len() >= 2, "{candidates:?}");
        assert!(
            candidates
                .iter()
                .all(|candidate| candidate.actions_conflict.is_none()),
            "{candidates:?}"
        );
        assert_eq!(conflict_writes(&fixture), 0, "{source}");
    }
}

/// A rule whose declared heads cannot pair across levels keeps no decisions
/// and writes no marker, even with a candidate at every level it acts on.
#[tokio::test]
async fn a_rule_that_cannot_conflict_across_levels_writes_no_marker() {
    let fixture = evaluation_app();
    seed_series_with_two_seasons(&fixture).await;
    armed_source(
        &fixture,
        "package rules\nimport rego.v1\n\n\
         monitor if input.facts.monitored\n\n\
         monitor_season if input.subject.season_number == 1\n\n\
         notify_episode := \"Synthetic\" if input.subject.season_number == 1\n",
    )
    .await;
    fixture.open_evaluation_gate().await;
    fixture.evaluate().await;
    fixture.evaluate().await;
    let candidates = fixture.evaluation.all_candidates().await;
    assert_eq!(
        candidate_levels(&candidates),
        vec!["episode", "episode", "season", "title"]
    );
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.actions_conflict.is_none())
    );
    assert_eq!(conflict_writes(&fixture), 0);
}

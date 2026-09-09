//! Dual-dialect storage for the maintenance evaluator's three tables
//! (RFC 137 section 11): lifecycle candidates, rule exclusions, and per-rule
//! evaluation runs.
//!
//! The store owns one invariant the callers must not have to remember: a rule
//! set holds at most one non-terminal candidate per title. The migration states
//! it as a partial unique index in both dialects; [`MaintenanceEvaluationStore`]
//! states it again as a checked read inside the insert's transaction, so the
//! failure is a named application error rather than a raw constraint violation.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scryer_application::maintenance_rules::ACTION_SEQUENCE_KIND;
use scryer_application::{
    AppError, AppResult, LifecycleActionRunRepository, MaintenanceActionJobReceiptClaim,
    MaintenanceActionJobReceiptTransition, MaintenanceActionStepCandidateKey,
    MaintenanceActionStepClaim, MaintenanceActionStepRepository, MaintenanceCandidateQuery,
    MaintenanceCandidateRepository, MaintenanceEvaluationRunRepository,
    MaintenanceExclusionRepository, MaintenanceSequenceCompletionRepository,
};
use scryer_domain::{
    LifecycleActionRun, LifecycleActionRunStatus, LifecycleCandidate,
    MAINTENANCE_TERMINAL_CANDIDATE_STATES, MaintenanceActionJobReceipt,
    MaintenanceActionJobReceiptState, MaintenanceActionStepAttempt, MaintenanceActionStepKey,
    MaintenanceActionStepRun, MaintenanceActionStepState, MaintenanceCandidateState,
    MaintenanceEvaluationRun, MaintenanceEvaluationRunStatus, MaintenanceRuleExclusion,
    MaintenanceSequenceTerminalMembership, MaintenanceSequenceTerminalOutcome,
};

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, SqlTx, StoreDatastore};
use crate::storage::sql::json::{canonical_json_text, json_text_or};

#[derive(Clone)]
pub struct MaintenanceEvaluationStore {
    datastore: StoreDatastore,
}

impl MaintenanceEvaluationStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

// ── Column lists and shared SQL ─────────────────────────────────────────────

const CANDIDATE_COLUMNS: &str = "id, rule_set_id, revision_number, matcher_content_hash, title_id,
    library_id, facet, subject_kind, subject_id, match_generation, state, state_reason, reason_codes,
    action_kind, grace_days, first_matched_at, last_matched_at, due_at, last_evaluated_at,
    held_since, action_attempts, created_at, updated_at";

const INSERT_CANDIDATE_SQL: &str = "INSERT INTO lifecycle_candidates
        (id, rule_set_id, revision_number, matcher_content_hash, title_id, library_id, facet,
         subject_kind, subject_id, match_generation, state, state_reason, reason_codes, action_kind,
         grace_days, first_matched_at, last_matched_at, due_at, last_evaluated_at, held_since,
         action_attempts, created_at, updated_at)
     VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})";

const ACTION_RUN_COLUMNS: &str =
    "id, candidate_id, rule_set_id, revision_number, title_id, subject_kind, subject_id,
    action_kind, match_generation, idempotency_key, attempt, status, hold_reason, error,
    detail, started_at, finished_at, created_at";

const INSERT_ACTION_RUN_SQL: &str = "INSERT INTO lifecycle_action_runs
        (id, candidate_id, rule_set_id, revision_number, title_id, subject_kind, subject_id, action_kind,
         match_generation, idempotency_key, attempt, status, hold_reason, error, detail,
         started_at, finished_at, created_at)
     VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})";

const ACTION_STEP_COLUMNS: &str = "candidate_id, match_generation, revision_number, step_id,
    rule_set_id, title_id, subject_kind, subject_id, sequence_content_hash, step_kind,
    intent_json, before_state_json, target_identity_json, provenance_json, state, attempt,
    lease_id, lease_expires_at, hold_reason, error, created_at, updated_at, finished_at";

const ACTION_STEP_ATTEMPT_COLUMNS: &str = "id, candidate_id, match_generation, revision_number,
    step_id, attempt, state, intent_json, before_state_json, target_identity_json,
    provenance_json, hold_reason, error, started_at, finished_at, created_at, updated_at";

const ACTION_JOB_RECEIPT_COLUMNS: &str = "candidate_id, match_generation, revision_number,
    step_id, dispatch_attempt, schema_version, logical_request_key, request_hash, job_run_id,
    state, reconciliation_evidence_json, created_at, updated_at";

const SEQUENCE_TERMINAL_MEMBERSHIP_COLUMNS: &str = "id, candidate_id, rule_set_id,
    revision_number, matcher_content_hash, title_id, subject_kind, subject_id, match_generation,
    sequence_content_hash, outcome, expected_step_ids_json, completed_step_ids_json,
    terminal_step_id, created_at, released_at";

const EXCLUSION_COLUMNS: &str =
    "id, rule_set_id, title_id, subject_kind, subject_id, reason, created_by, created_at";

const EVALUATION_RUN_COLUMNS: &str = "id, rule_set_id, revision_number, matcher_content_hash,
    started_at, finished_at, status, evaluated_count, matched_count, no_match_count,
    unknown_count, error_count, canceled_candidates, superseded_candidates, duration_ms, error";

/// `state NOT IN (…)` over the terminal set, as a placeholder list plus its
/// bound arguments. Built from the domain constant so the predicate here and
/// the migration's partial unique index can never drift apart.
fn active_state_predicate() -> (String, Vec<SqlArg>) {
    let placeholders = MAINTENANCE_TERMINAL_CANDIDATE_STATES
        .iter()
        .map(|_| "{}")
        .collect::<Vec<_>>()
        .join(", ");
    let args = MAINTENANCE_TERMINAL_CANDIDATE_STATES
        .iter()
        .map(|state| SqlArg::Text((*state).to_string()))
        .collect();
    (format!("state NOT IN ({placeholders})"), args)
}

// ── Candidates ──────────────────────────────────────────────────────────────

#[async_trait]
impl MaintenanceCandidateRepository for MaintenanceEvaluationStore {
    async fn get_active_subject_candidate(
        &self,
        rule_set_id: &str,
        subject_kind: &str,
        subject_id: &str,
    ) -> AppResult<Option<LifecycleCandidate>> {
        let (predicate, mut args) = active_state_predicate();
        let sql = format!(
            "SELECT {CANDIDATE_COLUMNS}
               FROM lifecycle_candidates
              WHERE rule_set_id = {{}} AND subject_kind = {{}} AND subject_id = {{}} AND {predicate}
              ORDER BY match_generation DESC
              LIMIT 1"
        );
        let mut bound = vec![
            SqlArg::Text(rule_set_id.to_string()),
            SqlArg::Text(subject_kind.to_string()),
            SqlArg::Text(subject_id.to_string()),
        ];
        bound.append(&mut args);

        SqlRuntime::fetch_optional(self.datastore.read_exec(), &sql, &bound)
            .await?
            .as_ref()
            .map(row_to_candidate)
            .transpose()
    }

    async fn list_candidates(
        &self,
        query: &MaintenanceCandidateQuery,
    ) -> AppResult<Vec<LifecycleCandidate>> {
        let mut clauses: Vec<String> = Vec::new();
        let mut args: Vec<SqlArg> = Vec::new();

        if let Some(rule_set_id) = query.rule_set_id.as_deref() {
            clauses.push("rule_set_id = {}".to_string());
            args.push(SqlArg::Text(rule_set_id.to_string()));
        }
        if let Some(library_id) = query.library_id.as_deref() {
            clauses.push("library_id = {}".to_string());
            args.push(SqlArg::Text(library_id.to_string()));
        }
        if !query.states.is_empty() {
            let placeholders = query
                .states
                .iter()
                .map(|_| "{}")
                .collect::<Vec<_>>()
                .join(", ");
            clauses.push(format!("state IN ({placeholders})"));
            for state in &query.states {
                args.push(SqlArg::Text(state.as_storage_str().to_string()));
            }
        }

        let where_clause = if clauses.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", clauses.join(" AND "))
        };
        // Bound in SQL, not after the fact: an unbounded listing of a large
        // library's candidates is exactly the payload this query must not
        // build before discarding most of it.
        let limit_clause = match query.limit {
            Some(limit) => {
                args.push(SqlArg::I64(limit as i64));
                " LIMIT {}"
            }
            None => "",
        };

        let sql = format!(
            "SELECT {CANDIDATE_COLUMNS}
               FROM lifecycle_candidates{where_clause}
              ORDER BY due_at ASC, id ASC{limit_clause}"
        );
        SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args)
            .await?
            .iter()
            .map(row_to_candidate)
            .collect()
    }

    async fn max_subject_match_generation(
        &self,
        rule_set_id: &str,
        subject_kind: &str,
        subject_id: &str,
    ) -> AppResult<i64> {
        // Terminal rows count: generations are monotonic per subject, so a
        // cancel-then-rematch is always distinguishable from a continuation.
        let sql = "SELECT match_generation
                     FROM lifecycle_candidates
                    WHERE rule_set_id = {} AND subject_kind = {} AND subject_id = {}
                    ORDER BY match_generation DESC
                    LIMIT 1";
        Ok(SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            sql,
            &[
                SqlArg::Text(rule_set_id.to_string()),
                SqlArg::Text(subject_kind.to_string()),
                SqlArg::Text(subject_id.to_string()),
            ],
        )
        .await?
        .as_ref()
        .map(|row| row.i64("match_generation"))
        .transpose()?
        .unwrap_or(0))
    }

    async fn create_candidate(&self, candidate: &LifecycleCandidate) -> AppResult<()> {
        let (predicate, mut predicate_args) = active_state_predicate();
        let exists_sql = format!(
            "SELECT id FROM lifecycle_candidates
              WHERE rule_set_id = {{}} AND subject_kind = {{}} AND subject_id = {{}} AND {predicate}
              LIMIT 1"
        );
        let mut exists_args = vec![
            SqlArg::Text(candidate.rule_set_id.clone()),
            SqlArg::Text(candidate.subject_kind.clone()),
            SqlArg::Text(candidate.subject_id.clone()),
        ];
        exists_args.append(&mut predicate_args);
        let insert_args = candidate_args(candidate)?;
        let rule_set_id = candidate.rule_set_id.clone();
        let title_id = candidate.title_id.clone();
        let revision_number = candidate.revision_number;
        let subject_kind = candidate.subject_kind.clone();
        let subject_id = candidate.subject_id.clone();
        let action_kind = candidate.action_kind.clone();

        SqlRuntime::run_in_transaction(&self.datastore, "create_lifecycle_candidate", move |tx| {
            let exists_sql = exists_sql.clone();
            let exists_args = exists_args.clone();
            let insert_args = insert_args.clone();
            let rule_set_id = rule_set_id.clone();
            let title_id = title_id.clone();
            let subject_kind = subject_kind.clone();
            let subject_id = subject_id.clone();
            let action_kind = action_kind.clone();
            Box::pin(async move {
                if SqlRuntime::fetch_optional(SqlExec::Tx(tx), &exists_sql, &exists_args)
                    .await?
                    .is_some()
                {
                    return Err(AppError::Validation(format!(
                        "maintenance rule {rule_set_id} already has an active candidate for title {title_id}"
                    )));
                }
                if action_kind == ACTION_SEQUENCE_KIND
                    && SqlRuntime::fetch_optional(
                        SqlExec::Tx(tx),
                        "SELECT id FROM maintenance_sequence_terminal_memberships
                          WHERE rule_set_id = {} AND revision_number = {}
                            AND subject_kind = {} AND subject_id = {} AND released_at IS NULL
                          LIMIT 1",
                        &[
                            SqlArg::Text(rule_set_id.clone()),
                            SqlArg::I64(revision_number),
                            SqlArg::Text(subject_kind),
                            SqlArg::Text(subject_id),
                        ],
                    )
                    .await?
                    .is_some()
                {
                    return Err(AppError::Validation(
                        "maintenance sequence membership is terminal until a confirmed no-match".to_string(),
                    ));
                }
                let insert_sql = format!("{INSERT_CANDIDATE_SQL} ON CONFLICT DO NOTHING");
                if SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    &insert_sql,
                    &insert_args,
                )
                .await?
                    != 1
                {
                    return Err(AppError::Validation(format!(
                        "maintenance rule {rule_set_id} already has an active candidate for title {title_id}"
                    )));
                }
                Ok(())
            })
        })
        .await
    }

    async fn record_candidate_match(
        &self,
        id: &str,
        last_matched_at: DateTime<Utc>,
        reason_codes: &[String],
        updated_at: DateTime<Utc>,
    ) -> AppResult<()> {
        // `first_matched_at` and `due_at` are absent on purpose: a continuing
        // membership never restarts its own grace clock (RFC 7.5).
        let args = vec![
            SqlArg::Timestamp(last_matched_at),
            SqlArg::Timestamp(last_matched_at),
            SqlArg::Text(canonical_json_text(&reason_codes)?),
            SqlArg::Timestamp(updated_at),
            SqlArg::Text(id.to_string()),
        ];
        execute_write(
            &self.datastore,
            "record_lifecycle_candidate_match",
            "UPDATE lifecycle_candidates
                SET last_matched_at = {}, last_evaluated_at = {}, reason_codes = {},
                    held_since = NULL, updated_at = {}
              WHERE id = {}",
            args,
        )
        .await
    }

    async fn hold_candidate(
        &self,
        id: &str,
        held_since: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    ) -> AppResult<()> {
        // COALESCE keeps the first hold's timestamp: how long a candidate has
        // been held is the interesting number, not when it was last re-held.
        let args = vec![
            SqlArg::Timestamp(held_since),
            SqlArg::Timestamp(held_since),
            SqlArg::Timestamp(updated_at),
            SqlArg::Text(id.to_string()),
        ];
        execute_write(
            &self.datastore,
            "hold_lifecycle_candidate",
            "UPDATE lifecycle_candidates
                SET last_evaluated_at = {}, held_since = COALESCE(held_since, {}), updated_at = {}
              WHERE id = {}",
            args,
        )
        .await
    }

    async fn transition_candidate_state(
        &self,
        id: &str,
        state: MaintenanceCandidateState,
        state_reason: &str,
        expected_states: &[MaintenanceCandidateState],
        updated_at: DateTime<Utc>,
    ) -> AppResult<bool> {
        // An empty expectation would degrade this back into the unconditional
        // UPDATE the compare-and-set exists to remove, so it is refused rather
        // than silently widened.
        if expected_states.is_empty() {
            return Err(AppError::Validation(
                "a candidate transition must name the states it expects".to_string(),
            ));
        }
        let placeholders = expected_states
            .iter()
            .map(|_| "{}")
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "UPDATE lifecycle_candidates
                SET state = {{}}, state_reason = {{}}, last_evaluated_at = {{}}, updated_at = {{}}
              WHERE id = {{}} AND state IN ({placeholders})"
        );
        let mut args = vec![
            SqlArg::Text(state.as_storage_str().to_string()),
            SqlArg::Text(state_reason.to_string()),
            SqlArg::Timestamp(updated_at),
            SqlArg::Timestamp(updated_at),
            SqlArg::Text(id.to_string()),
        ];
        args.extend(
            expected_states
                .iter()
                .map(|state| SqlArg::Text(state.as_storage_str().to_string())),
        );

        let affected = SqlRuntime::run_in_transaction(
            &self.datastore,
            "transition_lifecycle_candidate_state",
            move |tx| {
                let sql = sql.clone();
                let args = args.clone();
                Box::pin(async move { SqlRuntime::execute(SqlExec::Tx(tx), &sql, &args).await })
            },
        )
        .await?;
        Ok(affected == 1)
    }

    async fn finish_leased_candidate(
        &self,
        id: &str,
        expected_lease_updated_at: DateTime<Utc>,
        state: MaintenanceCandidateState,
        state_reason: &str,
        finished_at: DateTime<Utc>,
    ) -> AppResult<bool> {
        let args = vec![
            SqlArg::Text(state.as_storage_str().to_string()),
            SqlArg::Text(state_reason.to_string()),
            SqlArg::Timestamp(finished_at),
            SqlArg::Timestamp(finished_at),
            SqlArg::Text(id.to_string()),
            SqlArg::Text(
                MaintenanceCandidateState::Executing
                    .as_storage_str()
                    .to_string(),
            ),
            SqlArg::Timestamp(expected_lease_updated_at),
        ];
        let affected = SqlRuntime::run_in_transaction(
            &self.datastore,
            "finish_leased_lifecycle_candidate",
            move |tx| {
                let args = args.clone();
                Box::pin(async move {
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "UPDATE lifecycle_candidates
                SET state = {}, state_reason = {}, last_evaluated_at = {}, updated_at = {}
              WHERE id = {} AND state = {} AND updated_at = {}",
                        &args,
                    )
                    .await
                })
            },
        )
        .await?;
        Ok(affected == 1)
    }

    async fn cancel_active_candidates_for_rule(
        &self,
        rule_set_id: &str,
        state_reason: &str,
        updated_at: DateTime<Utc>,
    ) -> AppResult<u64> {
        let (predicate, mut predicate_args) = active_state_predicate();
        let sql = format!(
            "UPDATE lifecycle_candidates
                SET state = {{}}, state_reason = {{}}, last_evaluated_at = {{}}, updated_at = {{}}
              WHERE rule_set_id = {{}} AND {predicate}"
        );
        let mut args = vec![
            SqlArg::Text(
                MaintenanceCandidateState::Canceled
                    .as_storage_str()
                    .to_string(),
            ),
            SqlArg::Text(state_reason.to_string()),
            SqlArg::Timestamp(updated_at),
            SqlArg::Timestamp(updated_at),
            SqlArg::Text(rule_set_id.to_string()),
        ];
        args.append(&mut predicate_args);

        SqlRuntime::run_in_transaction(
            &self.datastore,
            "cancel_active_lifecycle_candidates",
            move |tx| {
                let sql = sql.clone();
                let args = args.clone();
                Box::pin(async move { SqlRuntime::execute(SqlExec::Tx(tx), &sql, &args).await })
            },
        )
        .await
    }

    async fn count_candidates_by_state(
        &self,
        rule_set_id: &str,
    ) -> AppResult<Vec<(MaintenanceCandidateState, i64)>> {
        let sql = "SELECT state, COUNT(*) AS candidate_count
                     FROM lifecycle_candidates
                    WHERE rule_set_id = {}
                    GROUP BY state
                    ORDER BY state ASC";
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            sql,
            &[SqlArg::Text(rule_set_id.to_string())],
        )
        .await?;

        let mut counts = Vec::with_capacity(rows.len());
        for row in &rows {
            // A state this build does not recognize was written by a newer one.
            // Dropping it from the tally is safer than folding it into a state
            // whose meaning it does not share.
            if let Some(state) = MaintenanceCandidateState::parse_storage(&row.text("state")?) {
                counts.push((state, row.i64("candidate_count")?));
            }
        }
        Ok(counts)
    }

    async fn list_due_candidates(
        &self,
        rule_set_id: &str,
        due_before: DateTime<Utc>,
        stale_before: DateTime<Utc>,
        limit: usize,
    ) -> AppResult<Vec<LifecycleCandidate>> {
        // `due_at <= due_before` still bounds both arms: a row that reached
        // `executing` was due when it was leased, so an abandoned lease is
        // always past its due time too. The second arm is what returns that
        // abandoned row — without it, the lease's own reclaim branch is
        // unreachable and the row is stranded for good.
        let sql = format!(
            "SELECT {CANDIDATE_COLUMNS}
               FROM lifecycle_candidates
              WHERE rule_set_id = {{}} AND due_at <= {{}}
                AND (state IN ({{}}, {{}}, {{}}, {{}})
                     OR (state = {{}} AND updated_at < {{}}))
              ORDER BY due_at ASC, id ASC
              LIMIT {{}}"
        );
        let args = vec![
            SqlArg::Text(rule_set_id.to_string()),
            SqlArg::Timestamp(due_before),
            SqlArg::Text(
                MaintenanceCandidateState::Observing
                    .as_storage_str()
                    .to_string(),
            ),
            SqlArg::Text(
                MaintenanceCandidateState::PendingAction
                    .as_storage_str()
                    .to_string(),
            ),
            SqlArg::Text(MaintenanceCandidateState::Due.as_storage_str().to_string()),
            SqlArg::Text(
                MaintenanceCandidateState::Blocked
                    .as_storage_str()
                    .to_string(),
            ),
            SqlArg::Text(
                MaintenanceCandidateState::Executing
                    .as_storage_str()
                    .to_string(),
            ),
            SqlArg::Timestamp(stale_before),
            SqlArg::I64(limit as i64),
        ];
        SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args)
            .await?
            .iter()
            .map(row_to_candidate)
            .collect()
    }

    async fn lease_candidate_for_execution(
        &self,
        id: &str,
        stale_before: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    ) -> AppResult<bool> {
        // The lease is the row count of one conditional write: exactly one
        // caller can move the row into `executing`, and a crashed lease is
        // reclaimable only once its `updated_at` has gone stale.
        let sql = "UPDATE lifecycle_candidates
                      SET state = {}, state_reason = {}, updated_at = {}
                    WHERE id = {}
                      AND (state = {} OR (state = {} AND updated_at < {}))";
        let args = vec![
            SqlArg::Text(
                MaintenanceCandidateState::Executing
                    .as_storage_str()
                    .to_string(),
            ),
            SqlArg::Text("execution_leased".to_string()),
            SqlArg::Timestamp(updated_at),
            SqlArg::Text(id.to_string()),
            SqlArg::Text(MaintenanceCandidateState::Due.as_storage_str().to_string()),
            SqlArg::Text(
                MaintenanceCandidateState::Executing
                    .as_storage_str()
                    .to_string(),
            ),
            SqlArg::Timestamp(stale_before),
        ];
        let affected = SqlRuntime::run_in_transaction(
            &self.datastore,
            "lease_lifecycle_candidate",
            move |tx| {
                let args = args.clone();
                Box::pin(async move { SqlRuntime::execute(SqlExec::Tx(tx), sql, &args).await })
            },
        )
        .await?;
        Ok(affected == 1)
    }

    async fn record_candidate_attempts(
        &self,
        id: &str,
        action_attempts: i64,
        updated_at: DateTime<Utc>,
    ) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "record_lifecycle_candidate_attempts",
            "UPDATE lifecycle_candidates
                SET action_attempts = {}, updated_at = {}
              WHERE id = {}",
            vec![
                SqlArg::I64(action_attempts),
                SqlArg::Timestamp(updated_at),
                SqlArg::Text(id.to_string()),
            ],
        )
        .await
    }
}

// ── Action runs ─────────────────────────────────────────────────────────────

#[async_trait]
impl LifecycleActionRunRepository for MaintenanceEvaluationStore {
    async fn start_action_run(&self, run: &LifecycleActionRun) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "start_lifecycle_action_run",
            INSERT_ACTION_RUN_SQL,
            action_run_args(run),
        )
        .await
    }

    async fn finish_action_run(&self, run: &LifecycleActionRun) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "finish_lifecycle_action_run",
            "UPDATE lifecycle_action_runs
                SET status = {}, hold_reason = {}, error = {}, detail = {}, finished_at = {}
              WHERE id = {}",
            vec![
                SqlArg::Text(run.status.as_storage_str().to_string()),
                SqlArg::OptText(run.hold_reason.clone()),
                SqlArg::OptText(run.error.clone()),
                SqlArg::Text(run.detail.clone()),
                SqlArg::OptTimestamp(run.finished_at),
                SqlArg::Text(run.id.clone()),
            ],
        )
        .await
    }

    async fn finish_held_action_run_and_release_attempt(
        &self,
        run: &LifecycleActionRun,
        expected_attempt: i64,
    ) -> AppResult<bool> {
        if expected_attempt <= 0
            || run.attempt != expected_attempt
            || run.status != LifecycleActionRunStatus::Held
            || run.finished_at.is_none()
            || run.idempotency_key != format!("hold:{}", run.id)
        {
            return Err(AppError::Validation(
                "invalid maintenance held-attempt release".to_string(),
            ));
        }
        let hold_reason = run
            .hold_reason
            .as_deref()
            .filter(|reason| !reason.is_empty())
            .ok_or_else(|| {
                AppError::Validation("maintenance held-attempt release lacks a reason".to_string())
            })?;
        let finished_at = run.finished_at.expect("checked above");
        let candidate_sql = "UPDATE lifecycle_candidates
                SET action_attempts = {}, state = {}, state_reason = {},
                    last_evaluated_at = {}, held_since = COALESCE(held_since, {}), updated_at = {}
              WHERE id = {} AND match_generation = {} AND action_kind = {}
                AND action_attempts = {} AND state = {}";
        let candidate_args = vec![
            SqlArg::I64(expected_attempt - 1),
            SqlArg::Text(
                MaintenanceCandidateState::Blocked
                    .as_storage_str()
                    .to_string(),
            ),
            SqlArg::Text(hold_reason.to_string()),
            SqlArg::Timestamp(finished_at),
            SqlArg::Timestamp(finished_at),
            SqlArg::Timestamp(finished_at),
            SqlArg::Text(run.candidate_id.clone()),
            SqlArg::I64(run.match_generation),
            SqlArg::Text(run.action_kind.clone()),
            SqlArg::I64(expected_attempt),
            SqlArg::Text(
                MaintenanceCandidateState::Executing
                    .as_storage_str()
                    .to_string(),
            ),
        ];
        let action_sql = "UPDATE lifecycle_action_runs
                SET idempotency_key = {}, status = {}, hold_reason = {}, error = {},
                    detail = {}, finished_at = {}
              WHERE id = {} AND candidate_id = {} AND match_generation = {}
                AND action_kind = {} AND attempt = {} AND status = {}";
        let action_args = vec![
            SqlArg::Text(run.idempotency_key.clone()),
            SqlArg::Text(run.status.as_storage_str().to_string()),
            SqlArg::OptText(run.hold_reason.clone()),
            SqlArg::OptText(run.error.clone()),
            SqlArg::Text(run.detail.clone()),
            SqlArg::OptTimestamp(run.finished_at),
            SqlArg::Text(run.id.clone()),
            SqlArg::Text(run.candidate_id.clone()),
            SqlArg::I64(run.match_generation),
            SqlArg::Text(run.action_kind.clone()),
            SqlArg::I64(expected_attempt),
            SqlArg::Text(
                LifecycleActionRunStatus::Running
                    .as_storage_str()
                    .to_string(),
            ),
        ];
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "finish_held_lifecycle_action_run_and_release_attempt",
            move |tx| {
                let candidate_args = candidate_args.clone();
                let action_args = action_args.clone();
                Box::pin(async move {
                    let affected =
                        SqlRuntime::execute(SqlExec::Tx(tx), candidate_sql, &candidate_args)
                            .await?;
                    if affected != 1 {
                        return Ok(false);
                    }
                    let action_affected =
                        SqlRuntime::execute(SqlExec::Tx(tx), action_sql, &action_args).await?;
                    if action_affected != 1 {
                        return Err(AppError::Repository(
                            "maintenance held action run was not running".to_string(),
                        ));
                    }
                    Ok(true)
                })
            },
        )
        .await
    }

    async fn latest_scoped_deletion_action_run(
        &self,
        candidate_id: &str,
        match_generation: i64,
        action_kind: &str,
    ) -> AppResult<Option<LifecycleActionRun>> {
        // Checkpoint details are serialized by the application. Filter before
        // limiting so an arbitrary number of later hold rows cannot hide them.
        let sql = format!(
            "SELECT {ACTION_RUN_COLUMNS}
               FROM lifecycle_action_runs
              WHERE candidate_id = {{}} AND match_generation = {{}} AND action_kind = {{}} AND detail LIKE {{}} ESCAPE '!'
              ORDER BY started_at DESC, id DESC LIMIT 1"
        );
        SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &sql,
            &[
                SqlArg::Text(candidate_id.to_string()),
                SqlArg::I64(match_generation),
                SqlArg::Text(action_kind.to_string()),
                SqlArg::Text("%\"scoped!_deletion\":%".to_string()),
            ],
        )
        .await?
        .as_ref()
        .map(row_to_action_run)
        .transpose()
    }

    async fn list_action_runs(
        &self,
        rule_set_id: Option<&str>,
        candidate_id: Option<&str>,
        limit: Option<usize>,
    ) -> AppResult<Vec<LifecycleActionRun>> {
        let mut clauses: Vec<String> = Vec::new();
        let mut args: Vec<SqlArg> = Vec::new();
        if let Some(rule_set_id) = rule_set_id {
            clauses.push("rule_set_id = {}".to_string());
            args.push(SqlArg::Text(rule_set_id.to_string()));
        }
        if let Some(candidate_id) = candidate_id {
            clauses.push("candidate_id = {}".to_string());
            args.push(SqlArg::Text(candidate_id.to_string()));
        }
        let where_clause = if clauses.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", clauses.join(" AND "))
        };
        let limit_clause = match limit {
            Some(limit) => {
                args.push(SqlArg::I64(limit as i64));
                " LIMIT {}"
            }
            None => "",
        };
        let sql = format!(
            "SELECT {ACTION_RUN_COLUMNS}
               FROM lifecycle_action_runs{where_clause}
              ORDER BY started_at DESC, id DESC{limit_clause}"
        );
        SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args)
            .await?
            .iter()
            .map(row_to_action_run)
            .collect()
    }

    async fn list_sequence_history_action_runs(
        &self,
        rule_set_id: Option<&str>,
        candidate_id: Option<&str>,
        limit: usize,
    ) -> AppResult<Vec<LifecycleActionRun>> {
        let mut clauses = vec![
            "action_kind = {}".to_string(),
            "detail LIKE {} ESCAPE '!'".to_string(),
        ];
        let mut args = vec![
            SqlArg::Text(ACTION_SEQUENCE_KIND.to_string()),
            SqlArg::Text("%\"maintenance!_sequence!_history\":true%".to_string()),
        ];
        if let Some(rule_set_id) = rule_set_id {
            clauses.push("rule_set_id = {}".to_string());
            args.push(SqlArg::Text(rule_set_id.to_string()));
        }
        if let Some(candidate_id) = candidate_id {
            clauses.push("candidate_id = {}".to_string());
            args.push(SqlArg::Text(candidate_id.to_string()));
        }
        args.push(SqlArg::I64(limit.min(200) as i64));
        let sql = format!(
            "SELECT {ACTION_RUN_COLUMNS}
               FROM lifecycle_action_runs
              WHERE {}
              ORDER BY started_at DESC, id DESC LIMIT {{}}",
            clauses.join(" AND ")
        );
        SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args)
            .await?
            .iter()
            .map(row_to_action_run)
            .collect()
    }

    async fn list_non_sequence_action_runs(
        &self,
        rule_set_id: Option<&str>,
        candidate_id: Option<&str>,
        limit: usize,
    ) -> AppResult<Vec<LifecycleActionRun>> {
        let mut clauses = vec!["action_kind <> {}".to_string()];
        let mut args = vec![SqlArg::Text(ACTION_SEQUENCE_KIND.to_string())];
        if let Some(rule_set_id) = rule_set_id {
            clauses.push("rule_set_id = {}".to_string());
            args.push(SqlArg::Text(rule_set_id.to_string()));
        }
        if let Some(candidate_id) = candidate_id {
            clauses.push("candidate_id = {}".to_string());
            args.push(SqlArg::Text(candidate_id.to_string()));
        }
        args.push(SqlArg::I64(limit.min(200) as i64));
        let sql = format!(
            "SELECT {ACTION_RUN_COLUMNS}
               FROM lifecycle_action_runs
              WHERE {}
              ORDER BY started_at DESC, id DESC LIMIT {{}}",
            clauses.join(" AND ")
        );
        SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args)
            .await?
            .iter()
            .map(row_to_action_run)
            .collect()
    }
}

// ── V2 sequence steps, receipts, and membership latches ────────────────────

#[async_trait]
impl MaintenanceActionStepRepository for MaintenanceEvaluationStore {
    async fn get_action_step(
        &self,
        key: &MaintenanceActionStepKey,
    ) -> AppResult<Option<MaintenanceActionStepRun>> {
        SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &format!(
                "SELECT {ACTION_STEP_COLUMNS} FROM maintenance_action_steps
                  WHERE candidate_id = {{}} AND match_generation = {{}}
                    AND revision_number = {{}} AND step_id = {{}}"
            ),
            &action_step_key_args(key),
        )
        .await?
        .as_ref()
        .map(row_to_action_step)
        .transpose()
    }

    async fn claim_action_step(
        &self,
        step: &MaintenanceActionStepRun,
        stale_before: DateTime<Utc>,
        lease_id: &str,
        leased_at: DateTime<Utc>,
    ) -> AppResult<MaintenanceActionStepClaim> {
        if step.state != MaintenanceActionStepState::Running || step.attempt < 0 {
            return Err(AppError::Validation(
                "a maintenance action step claim must start a non-negative running attempt".into(),
            ));
        }
        if !action_step_target_identity_is_consistent(step) {
            return Err(AppError::Validation(
                "a maintenance action step claim has an invalid target identity".into(),
            ));
        }
        let mut claimed = step.clone();
        claimed.lease_id = Some(lease_id.to_string());
        claimed.updated_at = leased_at;
        claimed.finished_at = None;
        claimed.hold_reason = None;
        claimed.error = None;
        let key = claimed.key.clone();
        let lease_id = lease_id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "claim_maintenance_action_step", move |tx| {
            let claimed = claimed.clone();
            let key = key.clone();
            let lease_id = lease_id.clone();
            Box::pin(async move {
                let existing = SqlRuntime::fetch_optional(
                    SqlExec::Tx(tx),
                    &format!(
                        "SELECT {ACTION_STEP_COLUMNS} FROM maintenance_action_steps
                          WHERE candidate_id = {{}} AND match_generation = {{}}
                            AND revision_number = {{}} AND step_id = {{}}"
                    ),
                    &action_step_key_args(&key),
                )
                .await?
                .as_ref()
                .map(row_to_action_step)
                .transpose()?;
                let existing = if let Some(existing) = existing {
                    existing
                } else {
                    let inserted = SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "INSERT INTO maintenance_action_steps
                         (candidate_id, match_generation, revision_number, step_id, rule_set_id,
                          title_id, subject_kind, subject_id, sequence_content_hash, step_kind,
                          intent_json, before_state_json, target_identity_json, provenance_json,
                          state, attempt, lease_id, lease_expires_at, hold_reason, error,
                          created_at, updated_at, finished_at)
                         VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})
                         ON CONFLICT(candidate_id, match_generation, revision_number, step_id) DO NOTHING",
                        &action_step_args(&claimed),
                    )
                    .await?;
                    if inserted == 1 {
                        insert_action_step_attempt(tx, &claimed).await?;
                        return Ok(MaintenanceActionStepClaim::Claimed(claimed));
                    }
                    SqlRuntime::fetch_optional(
                        SqlExec::Tx(tx),
                        &format!(
                            "SELECT {ACTION_STEP_COLUMNS} FROM maintenance_action_steps
                              WHERE candidate_id = {{}} AND match_generation = {{}}
                                AND revision_number = {{}} AND step_id = {{}}"
                        ),
                        &action_step_key_args(&key),
                    )
                    .await?
                    .as_ref()
                    .map(row_to_action_step)
                    .transpose()?
                    .ok_or_else(|| {
                        AppError::Repository(
                            "maintenance action step insert raced without a durable winner".into(),
                        )
                    })?
                };
                if existing.sequence_content_hash != claimed.sequence_content_hash
                    || existing.step_kind != claimed.step_kind
                    || !action_step_target_identity_is_consistent(&existing)
                {
                    return Err(AppError::Validation(
                        "maintenance action step does not match its persisted sequence identity".into(),
                    ));
                }
                if existing.state.is_completion_success() {
                    return Ok(MaintenanceActionStepClaim::Completed(existing));
                }
                if existing.state == MaintenanceActionStepState::Running
                    && existing.updated_at >= stale_before
                {
                    return Ok(MaintenanceActionStepClaim::Busy);
                }
                if existing.state == MaintenanceActionStepState::Running {
                    let affected = SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "UPDATE maintenance_action_steps
                            SET lease_id = {}, lease_expires_at = {}, updated_at = {}
                          WHERE candidate_id = {} AND match_generation = {}
                            AND revision_number = {} AND step_id = {}
                            AND state = {} AND updated_at = {}",
                        &[
                            SqlArg::Text(lease_id),
                            SqlArg::OptTimestamp(claimed.lease_expires_at),
                            SqlArg::Timestamp(claimed.updated_at),
                            SqlArg::Text(key.candidate_id.clone()),
                            SqlArg::I64(key.match_generation),
                            SqlArg::I64(key.revision_number),
                            SqlArg::Text(key.step_id.clone()),
                            SqlArg::Text(MaintenanceActionStepState::Running.as_storage_str().into()),
                            SqlArg::Timestamp(existing.updated_at),
                        ],
                    )
                    .await?;
                    if affected != 1 {
                        return Ok(MaintenanceActionStepClaim::Busy);
                    }
                    let mut reclaimed = existing;
                    reclaimed.lease_id = claimed.lease_id;
                    reclaimed.lease_expires_at = claimed.lease_expires_at;
                    reclaimed.updated_at = claimed.updated_at;
                    return Ok(MaintenanceActionStepClaim::Claimed(reclaimed));
                }
                if existing.state == MaintenanceActionStepState::Held {
                    let mut resumed = existing.clone();
                    resumed.state = MaintenanceActionStepState::Running;
                    resumed.lease_id = claimed.lease_id;
                    resumed.lease_expires_at = claimed.lease_expires_at;
                    resumed.hold_reason = None;
                    resumed.error = None;
                    resumed.updated_at = claimed.updated_at;
                    resumed.finished_at = None;
                    let affected = SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "UPDATE maintenance_action_steps
                            SET state = {}, lease_id = {}, lease_expires_at = {}, hold_reason = NULL,
                                error = NULL, updated_at = {}, finished_at = NULL
                          WHERE candidate_id = {} AND match_generation = {} AND revision_number = {}
                            AND step_id = {} AND state = {} AND updated_at = {}",
                        &[
                            SqlArg::Text(MaintenanceActionStepState::Running.as_storage_str().into()),
                            SqlArg::OptText(resumed.lease_id.clone()),
                            SqlArg::OptTimestamp(resumed.lease_expires_at),
                            SqlArg::Timestamp(resumed.updated_at),
                            SqlArg::Text(key.candidate_id.clone()),
                            SqlArg::I64(key.match_generation),
                            SqlArg::I64(key.revision_number),
                            SqlArg::Text(key.step_id.clone()),
                            SqlArg::Text(MaintenanceActionStepState::Held.as_storage_str().into()),
                            SqlArg::Timestamp(existing.updated_at),
                        ],
                    )
                    .await?;
                    if affected != 1 {
                        return Ok(MaintenanceActionStepClaim::Busy);
                    }
                    insert_action_step_attempt(tx, &resumed).await?;
                    return Ok(MaintenanceActionStepClaim::Claimed(resumed));
                }
                if claimed.attempt <= existing.attempt {
                    return Err(AppError::Validation(
                        "a retried maintenance action step must advance its attempt".into(),
                    ));
                }
                let mut retried = existing.clone();
                retried.state = MaintenanceActionStepState::Running;
                retried.attempt = claimed.attempt;
                retried.lease_id = claimed.lease_id;
                retried.lease_expires_at = claimed.lease_expires_at;
                retried.hold_reason = None;
                retried.error = None;
                retried.updated_at = claimed.updated_at;
                retried.finished_at = None;
                let affected = SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "UPDATE maintenance_action_steps
                        SET intent_json = {}, before_state_json = {}, target_identity_json = {},
                            provenance_json = {}, state = {}, attempt = {}, lease_id = {},
                            lease_expires_at = {}, hold_reason = NULL, error = NULL,
                            updated_at = {}, finished_at = NULL
                      WHERE candidate_id = {} AND match_generation = {} AND revision_number = {}
                        AND step_id = {} AND state = {} AND updated_at = {}",
                    &action_step_retry_args(&retried, existing.state, existing.updated_at),
                )
                .await?;
                if affected != 1 {
                    return Ok(MaintenanceActionStepClaim::Busy);
                }
                insert_action_step_attempt(tx, &retried).await?;
                Ok(MaintenanceActionStepClaim::Claimed(retried))
            })
        })
        .await
    }

    async fn checkpoint_action_step(
        &self,
        step: &MaintenanceActionStepRun,
        expected_lease_id: &str,
    ) -> AppResult<bool> {
        update_running_action_step(&self.datastore, step, expected_lease_id, false).await
    }

    async fn finish_action_step(
        &self,
        step: &MaintenanceActionStepRun,
        expected_lease_id: &str,
    ) -> AppResult<bool> {
        if !step.state.is_attempt_finished() || step.state == MaintenanceActionStepState::Running {
            return Err(AppError::Validation(
                "a maintenance action step finish must carry an attempt result".into(),
            ));
        }
        update_running_action_step(&self.datastore, step, expected_lease_id, true).await
    }

    async fn list_action_steps(
        &self,
        candidate_id: &str,
        match_generation: i64,
        revision_number: i64,
    ) -> AppResult<Vec<MaintenanceActionStepRun>> {
        SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &format!(
                "SELECT {ACTION_STEP_COLUMNS} FROM maintenance_action_steps
                  WHERE candidate_id = {{}} AND match_generation = {{}} AND revision_number = {{}}
                  ORDER BY step_id ASC"
            ),
            &[
                SqlArg::Text(candidate_id.to_string()),
                SqlArg::I64(match_generation),
                SqlArg::I64(revision_number),
            ],
        )
        .await?
        .iter()
        .map(row_to_action_step)
        .collect()
    }

    async fn list_action_steps_for_candidates(
        &self,
        candidates: &[MaintenanceActionStepCandidateKey],
    ) -> AppResult<Vec<MaintenanceActionStepRun>> {
        if candidates.is_empty() {
            return Ok(Vec::new());
        }
        if candidates.len() > 100 {
            return Err(AppError::Validation(
                "maintenance sequence progress accepts at most 100 candidates per page".into(),
            ));
        }
        let mut args = Vec::with_capacity(candidates.len() * 3);
        let predicates = candidates
            .iter()
            .map(|candidate| {
                args.push(SqlArg::Text(candidate.candidate_id.clone()));
                args.push(SqlArg::I64(candidate.match_generation));
                args.push(SqlArg::I64(candidate.revision_number));
                "(candidate_id = {} AND match_generation = {} AND revision_number = {})"
            })
            .collect::<Vec<_>>()
            .join(" OR ");
        SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &format!(
                "SELECT {ACTION_STEP_COLUMNS} FROM maintenance_action_steps
                  WHERE {predicates}
                  ORDER BY candidate_id ASC, match_generation ASC, revision_number ASC, step_id ASC"
            ),
            &args,
        )
        .await?
        .iter()
        .map(row_to_action_step)
        .collect()
    }

    async fn list_action_step_attempts(
        &self,
        key: &MaintenanceActionStepKey,
    ) -> AppResult<Vec<MaintenanceActionStepAttempt>> {
        SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &format!(
                "SELECT {ACTION_STEP_ATTEMPT_COLUMNS} FROM maintenance_action_step_attempts
                  WHERE candidate_id = {{}} AND match_generation = {{}}
                    AND revision_number = {{}} AND step_id = {{}}
                  ORDER BY started_at ASC, id ASC"
            ),
            &action_step_key_args(key),
        )
        .await?
        .iter()
        .map(row_to_action_step_attempt)
        .collect()
    }

    async fn get_action_job_receipt(
        &self,
        key: &MaintenanceActionStepKey,
        dispatch_attempt: i64,
    ) -> AppResult<Option<MaintenanceActionJobReceipt>> {
        let mut args = action_step_key_args(key);
        args.push(SqlArg::I64(dispatch_attempt));
        SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &format!(
                "SELECT {ACTION_JOB_RECEIPT_COLUMNS} FROM maintenance_action_job_receipts
                  WHERE candidate_id = {{}} AND match_generation = {{}}
                    AND revision_number = {{}} AND step_id = {{}} AND dispatch_attempt = {{}}"
            ),
            &args,
        )
        .await?
        .as_ref()
        .map(row_to_action_job_receipt)
        .transpose()
    }

    async fn list_action_job_receipts(
        &self,
        key: &MaintenanceActionStepKey,
    ) -> AppResult<Vec<MaintenanceActionJobReceipt>> {
        SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &format!(
                "SELECT {ACTION_JOB_RECEIPT_COLUMNS} FROM maintenance_action_job_receipts
                  WHERE candidate_id = {{}} AND match_generation = {{}}
                    AND revision_number = {{}} AND step_id = {{}}
                  ORDER BY dispatch_attempt ASC"
            ),
            &action_step_key_args(key),
        )
        .await?
        .iter()
        .map(row_to_action_job_receipt)
        .collect()
    }

    async fn list_action_job_receipts_for_steps(
        &self,
        steps: &[MaintenanceActionStepKey],
    ) -> AppResult<Vec<MaintenanceActionJobReceipt>> {
        if steps.is_empty() {
            return Ok(Vec::new());
        }
        if steps.len() > 700 {
            return Err(AppError::Validation(
                "maintenance sequence receipt projection accepts at most 700 steps".into(),
            ));
        }
        let mut args = Vec::with_capacity(steps.len() * 4);
        let predicates = steps
            .iter()
            .map(|step| {
                args.extend(action_step_key_args(step));
                "(candidate_id = {} AND match_generation = {} AND revision_number = {} AND step_id = {})"
            })
            .collect::<Vec<_>>()
            .join(" OR ");
        SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &format!(
                "SELECT {ACTION_JOB_RECEIPT_COLUMNS} FROM maintenance_action_job_receipts
                  WHERE {predicates}
                  ORDER BY candidate_id ASC, match_generation ASC, revision_number ASC,
                           step_id ASC, dispatch_attempt ASC"
            ),
            &args,
        )
        .await?
        .iter()
        .map(row_to_action_job_receipt)
        .collect()
    }

    async fn claim_action_job_dispatch(
        &self,
        receipt: &MaintenanceActionJobReceipt,
    ) -> AppResult<MaintenanceActionJobReceiptClaim> {
        receipt
            .validate_schema()
            .map_err(|error| AppError::Validation(error.to_string()))?;
        if receipt.state != MaintenanceActionJobReceiptState::Dispatching {
            return Err(AppError::Validation(
                "a generic maintenance job dispatch must begin in dispatching state".into(),
            ));
        }
        let receipt = receipt.clone();
        SqlRuntime::run_in_transaction(&self.datastore, "claim_maintenance_action_job_dispatch", move |tx| {
            let receipt = receipt.clone();
            Box::pin(async move {
                let mut key_args = action_step_key_args(&receipt.key);
                key_args.push(SqlArg::I64(receipt.dispatch_attempt));
                if let Some(existing) = SqlRuntime::fetch_optional(
                    SqlExec::Tx(tx),
                    &format!(
                        "SELECT {ACTION_JOB_RECEIPT_COLUMNS} FROM maintenance_action_job_receipts
                          WHERE candidate_id = {{}} AND match_generation = {{}}
                            AND revision_number = {{}} AND step_id = {{}} AND dispatch_attempt = {{}}"
                    ),
                    &key_args,
                )
                .await?
                .as_ref()
                .map(row_to_action_job_receipt)
                .transpose()?
                {
                    return Ok(MaintenanceActionJobReceiptClaim::Existing(existing));
                }
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "INSERT INTO maintenance_action_job_receipts
                     (candidate_id, match_generation, revision_number, step_id, dispatch_attempt,
                      schema_version, logical_request_key, request_hash, job_run_id, state,
                      reconciliation_evidence_json, created_at, updated_at)
                     VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
                    &action_job_receipt_args(&receipt),
                )
                .await?;
                Ok(MaintenanceActionJobReceiptClaim::Claimed(receipt))
            })
        })
        .await
    }

    async fn transition_action_job_receipt(
        &self,
        transition: &MaintenanceActionJobReceiptTransition,
    ) -> AppResult<bool> {
        transition_action_job_receipt(&self.datastore, transition).await
    }
}

#[async_trait]
impl MaintenanceSequenceCompletionRepository for MaintenanceEvaluationStore {
    async fn get_active_sequence_completion(
        &self,
        rule_set_id: &str,
        revision_number: i64,
        subject_kind: &str,
        subject_id: &str,
    ) -> AppResult<Option<MaintenanceSequenceTerminalMembership>> {
        SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &format!(
                "SELECT {SEQUENCE_TERMINAL_MEMBERSHIP_COLUMNS}
                   FROM maintenance_sequence_terminal_memberships
                  WHERE rule_set_id = {{}} AND revision_number = {{}}
                    AND subject_kind = {{}} AND subject_id = {{}} AND released_at IS NULL
                  LIMIT 1"
            ),
            &[
                SqlArg::Text(rule_set_id.to_string()),
                SqlArg::I64(revision_number),
                SqlArg::Text(subject_kind.to_string()),
                SqlArg::Text(subject_id.to_string()),
            ],
        )
        .await?
        .as_ref()
        .map(row_to_sequence_terminal_membership)
        .transpose()
    }

    async fn list_active_sequence_completions(
        &self,
        rule_set_id: &str,
        revision_number: i64,
        after_id: Option<&str>,
        limit: usize,
    ) -> AppResult<Vec<MaintenanceSequenceTerminalMembership>> {
        let limit = i64::try_from(limit.min(100)).map_err(|error| {
            AppError::Validation(format!("invalid maintenance completion limit: {error}"))
        })?;
        SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &format!(
                "SELECT {SEQUENCE_TERMINAL_MEMBERSHIP_COLUMNS}
                   FROM maintenance_sequence_terminal_memberships
                  WHERE rule_set_id = {{}} AND revision_number = {{}} AND released_at IS NULL
                    AND ({{}} IS NULL OR id > {{}})
                  ORDER BY id ASC LIMIT {{}}"
            ),
            &[
                SqlArg::Text(rule_set_id.to_string()),
                SqlArg::I64(revision_number),
                SqlArg::OptText(after_id.map(str::to_string)),
                SqlArg::OptText(after_id.map(str::to_string)),
                SqlArg::I64(limit),
            ],
        )
        .await?
        .iter()
        .map(row_to_sequence_terminal_membership)
        .collect()
    }

    async fn finish_sequence_terminal_membership_and_candidate(
        &self,
        membership: &MaintenanceSequenceTerminalMembership,
        expected_candidate_state: MaintenanceCandidateState,
        expected_candidate_updated_at: DateTime<Utc>,
        terminal_candidate_state: MaintenanceCandidateState,
        state_reason: &str,
        finished_at: DateTime<Utc>,
    ) -> AppResult<bool> {
        validate_terminal_membership(membership, terminal_candidate_state)?;
        let membership = membership.clone();
        let state_reason = state_reason.to_string();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "finish_maintenance_sequence_terminal_membership_and_candidate",
            move |tx| {
                let membership = membership.clone();
                let state_reason = state_reason.clone();
                Box::pin(async move {
                    if let Some(existing) = SqlRuntime::fetch_optional(
                        SqlExec::Tx(tx),
                        &format!(
                            "SELECT {SEQUENCE_TERMINAL_MEMBERSHIP_COLUMNS}
                               FROM maintenance_sequence_terminal_memberships WHERE candidate_id = {{}}"
                        ),
                        &[SqlArg::Text(membership.candidate_id.clone())],
                    )
                    .await?
                    .as_ref()
                    .map(row_to_sequence_terminal_membership)
                    .transpose()?
                    {
                        return Ok(same_terminal_membership(&existing, &membership));
                    }
                    verify_terminal_step_evidence(tx, &membership).await?;
                    let inserted = SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "INSERT INTO maintenance_sequence_terminal_memberships
                         (id, candidate_id, rule_set_id, revision_number, matcher_content_hash, title_id,
                          subject_kind, subject_id, match_generation, sequence_content_hash, outcome,
                          expected_step_ids_json, completed_step_ids_json, terminal_step_id, created_at, released_at)
                         VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})
                         ON CONFLICT DO NOTHING",
                        &terminal_membership_args(&membership)?,
                    )
                    .await?;
                    if inserted != 1 {
                        let existing = SqlRuntime::fetch_optional(
                            SqlExec::Tx(tx),
                            &format!(
                                "SELECT {SEQUENCE_TERMINAL_MEMBERSHIP_COLUMNS}
                                   FROM maintenance_sequence_terminal_memberships
                                  WHERE candidate_id = {{}}"
                            ),
                            &[SqlArg::Text(membership.candidate_id.clone())],
                        )
                        .await?
                        .as_ref()
                        .map(row_to_sequence_terminal_membership)
                        .transpose()?;
                        return Ok(existing.is_some_and(|existing| {
                            same_terminal_membership(&existing, &membership)
                        }));
                    }
                    let affected = SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "UPDATE lifecycle_candidates
                            SET state = {}, state_reason = {}, updated_at = {}
                          WHERE id = {} AND revision_number = {} AND match_generation = {}
                            AND state = {} AND updated_at = {}",
                        &[
                            SqlArg::Text(terminal_candidate_state.as_storage_str().into()),
                            SqlArg::Text(state_reason),
                            SqlArg::Timestamp(finished_at),
                            SqlArg::Text(membership.candidate_id.clone()),
                            SqlArg::I64(membership.revision_number),
                            SqlArg::I64(membership.match_generation),
                            SqlArg::Text(expected_candidate_state.as_storage_str().into()),
                            SqlArg::Timestamp(expected_candidate_updated_at),
                        ],
                    )
                    .await?;
                    if affected != 1 {
                        return Err(AppError::Repository(
                            "maintenance candidate lease was lost before its terminal membership could commit".into(),
                        ));
                    }
                    Ok(true)
                })
            },
        )
        .await
    }

    async fn release_sequence_completion_on_confirmed_non_match(
        &self,
        rule_set_id: &str,
        revision_number: i64,
        subject_kind: &str,
        subject_id: &str,
        released_at: DateTime<Utc>,
    ) -> AppResult<bool> {
        Ok(SqlRuntime::execute_write(
            &self.datastore,
            "release_maintenance_sequence_completion_on_confirmed_non_match",
            "UPDATE maintenance_sequence_terminal_memberships
                SET released_at = {}
              WHERE rule_set_id = {} AND revision_number = {} AND subject_kind = {}
                AND subject_id = {} AND released_at IS NULL AND created_at <= {}",
            vec![
                SqlArg::Timestamp(released_at),
                SqlArg::Text(rule_set_id.to_string()),
                SqlArg::I64(revision_number),
                SqlArg::Text(subject_kind.to_string()),
                SqlArg::Text(subject_id.to_string()),
                SqlArg::Timestamp(released_at),
            ],
        )
        .await?
            == 1)
    }
}

// ── Exclusions ──────────────────────────────────────────────────────────────

#[async_trait]
impl MaintenanceExclusionRepository for MaintenanceEvaluationStore {
    async fn list_exclusions(
        &self,
        rule_set_id: Option<&str>,
    ) -> AppResult<Vec<MaintenanceRuleExclusion>> {
        let (sql, args) = match rule_set_id {
            // What actually stops one rule acting is its own rows plus every
            // global row, so both are returned together.
            Some(rule_set_id) => (
                format!(
                    "SELECT {EXCLUSION_COLUMNS}
                       FROM maintenance_rule_exclusions
                      WHERE rule_set_id = {{}} OR rule_set_id IS NULL
                      ORDER BY created_at DESC, id ASC"
                ),
                vec![SqlArg::Text(rule_set_id.to_string())],
            ),
            None => (
                format!(
                    "SELECT {EXCLUSION_COLUMNS}
                       FROM maintenance_rule_exclusions
                      ORDER BY created_at DESC, id ASC"
                ),
                Vec::new(),
            ),
        };

        SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args)
            .await?
            .iter()
            .map(row_to_exclusion)
            .collect()
    }

    async fn get_exclusion(&self, id: &str) -> AppResult<Option<MaintenanceRuleExclusion>> {
        let sql =
            format!("SELECT {EXCLUSION_COLUMNS} FROM maintenance_rule_exclusions WHERE id = {{}}");
        SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(id.to_string())],
        )
        .await?
        .as_ref()
        .map(row_to_exclusion)
        .transpose()
    }

    async fn create_exclusion(&self, exclusion: &MaintenanceRuleExclusion) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "create_maintenance_rule_exclusion",
            "INSERT INTO maintenance_rule_exclusions
                (id, rule_set_id, title_id, subject_kind, subject_id, reason, created_by, created_at)
             VALUES ({}, {}, {}, {}, {}, {}, {}, {})",
            vec![
                SqlArg::Text(exclusion.id.clone()),
                SqlArg::OptText(exclusion.rule_set_id.clone()),
                SqlArg::Text(exclusion.title_id.clone()),
                SqlArg::Text(exclusion.subject_kind.as_storage_str().to_string()),
                SqlArg::Text(exclusion.subject_id.clone()),
                SqlArg::Text(exclusion.reason.clone()),
                SqlArg::OptText(exclusion.created_by.clone()),
                SqlArg::Timestamp(exclusion.created_at),
            ],
        )
        .await
    }

    async fn delete_exclusion(&self, id: &str) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "delete_maintenance_rule_exclusion",
            "DELETE FROM maintenance_rule_exclusions WHERE id = {}",
            vec![SqlArg::Text(id.to_string())],
        )
        .await
    }
}

// ── Evaluation runs ─────────────────────────────────────────────────────────

#[async_trait]
impl MaintenanceEvaluationRunRepository for MaintenanceEvaluationStore {
    async fn start_evaluation_run(&self, run: &MaintenanceEvaluationRun) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "start_maintenance_evaluation_run",
            "INSERT INTO maintenance_evaluation_runs
                (id, rule_set_id, revision_number, matcher_content_hash, started_at, status)
             VALUES ({}, {}, {}, {}, {}, {})",
            vec![
                SqlArg::Text(run.id.clone()),
                SqlArg::Text(run.rule_set_id.clone()),
                SqlArg::I64(run.revision_number),
                SqlArg::Text(run.matcher_content_hash.clone()),
                SqlArg::Timestamp(run.started_at),
                SqlArg::Text(run.status.as_storage_str().to_string()),
            ],
        )
        .await
    }

    async fn finish_evaluation_run(&self, run: &MaintenanceEvaluationRun) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "finish_maintenance_evaluation_run",
            "UPDATE maintenance_evaluation_runs
                SET finished_at = {}, status = {}, evaluated_count = {}, matched_count = {},
                    no_match_count = {}, unknown_count = {}, error_count = {},
                    canceled_candidates = {}, superseded_candidates = {}, duration_ms = {},
                    error = {}
              WHERE id = {}",
            vec![
                SqlArg::OptTimestamp(run.finished_at),
                SqlArg::Text(run.status.as_storage_str().to_string()),
                SqlArg::I64(run.evaluated_count),
                SqlArg::I64(run.matched_count),
                SqlArg::I64(run.no_match_count),
                SqlArg::I64(run.unknown_count),
                SqlArg::I64(run.error_count),
                SqlArg::I64(run.canceled_candidates),
                SqlArg::I64(run.superseded_candidates),
                SqlArg::OptI64(run.duration_ms),
                SqlArg::OptText(run.error.clone()),
                SqlArg::Text(run.id.clone()),
            ],
        )
        .await
    }

    async fn list_evaluation_runs(
        &self,
        rule_set_id: Option<&str>,
        limit: Option<usize>,
    ) -> AppResult<Vec<MaintenanceEvaluationRun>> {
        let mut args: Vec<SqlArg> = Vec::new();
        let where_clause = match rule_set_id {
            Some(rule_set_id) => {
                args.push(SqlArg::Text(rule_set_id.to_string()));
                " WHERE rule_set_id = {}"
            }
            None => "",
        };
        let limit_clause = match limit {
            Some(limit) => {
                args.push(SqlArg::I64(limit as i64));
                " LIMIT {}"
            }
            None => "",
        };

        let sql = format!(
            "SELECT {EVALUATION_RUN_COLUMNS}
               FROM maintenance_evaluation_runs{where_clause}
              ORDER BY started_at DESC, id ASC{limit_clause}"
        );
        SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args)
            .await?
            .iter()
            .map(row_to_evaluation_run)
            .collect()
    }
}

// ── Shared helpers ──────────────────────────────────────────────────────────

async fn execute_write(
    datastore: &StoreDatastore,
    op_name: &'static str,
    sql: &'static str,
    args: Vec<SqlArg>,
) -> AppResult<()> {
    SqlRuntime::run_in_transaction(datastore, op_name, move |tx| {
        let args = args.clone();
        Box::pin(async move {
            SqlRuntime::execute(SqlExec::Tx(tx), sql, &args).await?;
            Ok(())
        })
    })
    .await
}

fn candidate_args(candidate: &LifecycleCandidate) -> AppResult<Vec<SqlArg>> {
    Ok(vec![
        SqlArg::Text(candidate.id.clone()),
        SqlArg::Text(candidate.rule_set_id.clone()),
        SqlArg::I64(candidate.revision_number),
        SqlArg::Text(candidate.matcher_content_hash.clone()),
        SqlArg::Text(candidate.title_id.clone()),
        SqlArg::Text(candidate.library_id.clone()),
        SqlArg::Text(candidate.facet.clone()),
        SqlArg::Text(candidate.subject_kind.clone()),
        SqlArg::Text(candidate.subject_id.clone()),
        SqlArg::I64(candidate.match_generation),
        SqlArg::Text(candidate.state.as_storage_str().to_string()),
        SqlArg::Text(candidate.state_reason.clone()),
        SqlArg::Text(canonical_json_text(&candidate.reason_codes)?),
        SqlArg::Text(candidate.action_kind.clone()),
        SqlArg::I64(candidate.grace_days),
        SqlArg::Timestamp(candidate.first_matched_at),
        SqlArg::Timestamp(candidate.last_matched_at),
        SqlArg::Timestamp(candidate.due_at),
        SqlArg::Timestamp(candidate.last_evaluated_at),
        SqlArg::OptTimestamp(candidate.held_since),
        SqlArg::I64(candidate.action_attempts),
        SqlArg::Timestamp(candidate.created_at),
        SqlArg::Timestamp(candidate.updated_at),
    ])
}

fn action_run_args(run: &LifecycleActionRun) -> Vec<SqlArg> {
    vec![
        SqlArg::Text(run.id.clone()),
        SqlArg::Text(run.candidate_id.clone()),
        SqlArg::Text(run.rule_set_id.clone()),
        SqlArg::I64(run.revision_number),
        SqlArg::Text(run.title_id.clone()),
        SqlArg::Text(run.subject_kind.clone()),
        SqlArg::Text(run.subject_id.clone()),
        SqlArg::Text(run.action_kind.clone()),
        SqlArg::I64(run.match_generation),
        SqlArg::Text(run.idempotency_key.clone()),
        SqlArg::I64(run.attempt),
        SqlArg::Text(run.status.as_storage_str().to_string()),
        SqlArg::OptText(run.hold_reason.clone()),
        SqlArg::OptText(run.error.clone()),
        SqlArg::Text(run.detail.clone()),
        SqlArg::Timestamp(run.started_at),
        SqlArg::OptTimestamp(run.finished_at),
        SqlArg::Timestamp(run.created_at),
    ]
}

fn action_step_key_args(key: &MaintenanceActionStepKey) -> Vec<SqlArg> {
    vec![
        SqlArg::Text(key.candidate_id.clone()),
        SqlArg::I64(key.match_generation),
        SqlArg::I64(key.revision_number),
        SqlArg::Text(key.step_id.clone()),
    ]
}

fn action_step_args(step: &MaintenanceActionStepRun) -> Vec<SqlArg> {
    vec![
        SqlArg::Text(step.key.candidate_id.clone()),
        SqlArg::I64(step.key.match_generation),
        SqlArg::I64(step.key.revision_number),
        SqlArg::Text(step.key.step_id.clone()),
        SqlArg::Text(step.rule_set_id.clone()),
        SqlArg::Text(step.title_id.clone()),
        SqlArg::Text(step.subject_kind.clone()),
        SqlArg::Text(step.subject_id.clone()),
        SqlArg::Text(step.sequence_content_hash.clone()),
        SqlArg::Text(step.step_kind.clone()),
        SqlArg::Text(step.intent_json.clone()),
        SqlArg::Text(step.before_state_json.clone()),
        SqlArg::Text(step.target_identity_json.clone()),
        SqlArg::Text(step.provenance_json.clone()),
        SqlArg::Text(step.state.as_storage_str().to_string()),
        SqlArg::I64(step.attempt),
        SqlArg::OptText(step.lease_id.clone()),
        SqlArg::OptTimestamp(step.lease_expires_at),
        SqlArg::OptText(step.hold_reason.clone()),
        SqlArg::OptText(step.error.clone()),
        SqlArg::Timestamp(step.created_at),
        SqlArg::Timestamp(step.updated_at),
        SqlArg::OptTimestamp(step.finished_at),
    ]
}

fn action_step_retry_args(
    step: &MaintenanceActionStepRun,
    expected_state: MaintenanceActionStepState,
    expected_updated_at: DateTime<Utc>,
) -> Vec<SqlArg> {
    let mut args = vec![
        SqlArg::Text(step.intent_json.clone()),
        SqlArg::Text(step.before_state_json.clone()),
        SqlArg::Text(step.target_identity_json.clone()),
        SqlArg::Text(step.provenance_json.clone()),
        SqlArg::Text(
            MaintenanceActionStepState::Running
                .as_storage_str()
                .to_string(),
        ),
        SqlArg::I64(step.attempt),
        SqlArg::OptText(step.lease_id.clone()),
        SqlArg::OptTimestamp(step.lease_expires_at),
        SqlArg::Timestamp(step.updated_at),
    ];
    args.extend(action_step_key_args(&step.key));
    args.push(SqlArg::Text(expected_state.as_storage_str().to_string()));
    args.push(SqlArg::Timestamp(expected_updated_at));
    args
}

fn action_step_target_identity_is_consistent(step: &MaintenanceActionStepRun) -> bool {
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

async fn insert_action_step_attempt(
    tx: &mut SqlTx<'_>,
    step: &MaintenanceActionStepRun,
) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "INSERT INTO maintenance_action_step_attempts
         (id, candidate_id, match_generation, revision_number, step_id, attempt, state,
          intent_json, before_state_json, target_identity_json, provenance_json, hold_reason,
          error, started_at, finished_at, created_at, updated_at)
         VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
        &[
            SqlArg::Text(scryer_domain::Id::new().0),
            SqlArg::Text(step.key.candidate_id.clone()),
            SqlArg::I64(step.key.match_generation),
            SqlArg::I64(step.key.revision_number),
            SqlArg::Text(step.key.step_id.clone()),
            SqlArg::I64(step.attempt),
            SqlArg::Text(
                MaintenanceActionStepState::Running
                    .as_storage_str()
                    .to_string(),
            ),
            SqlArg::Text(step.intent_json.clone()),
            SqlArg::Text(step.before_state_json.clone()),
            SqlArg::Text(step.target_identity_json.clone()),
            SqlArg::Text(step.provenance_json.clone()),
            SqlArg::OptText(None),
            SqlArg::OptText(None),
            SqlArg::Timestamp(step.updated_at),
            SqlArg::OptTimestamp(None),
            SqlArg::Timestamp(step.updated_at),
            SqlArg::Timestamp(step.updated_at),
        ],
    )
    .await?;
    Ok(())
}

async fn update_running_action_step(
    datastore: &StoreDatastore,
    step: &MaintenanceActionStepRun,
    expected_lease_id: &str,
    finish: bool,
) -> AppResult<bool> {
    let step = step.clone();
    let expected_lease_id = expected_lease_id.to_string();
    SqlRuntime::run_in_transaction(
        datastore,
        "update_running_maintenance_action_step",
        move |tx| {
            let step = step.clone();
            let expected_lease_id = expected_lease_id.clone();
            Box::pin(async move {
                let (sql, args) = if finish {
                    (
                        "UPDATE maintenance_action_steps
                        SET before_state_json = {}, target_identity_json = {}, provenance_json = {},
                            state = {}, lease_id = NULL, lease_expires_at = NULL, hold_reason = {},
                            error = {}, updated_at = {}, finished_at = {}
                      WHERE candidate_id = {} AND match_generation = {} AND revision_number = {}
                        AND step_id = {} AND state = {} AND lease_id = {}",
                        vec![
                            SqlArg::Text(step.before_state_json.clone()),
                            SqlArg::Text(step.target_identity_json.clone()),
                            SqlArg::Text(step.provenance_json.clone()),
                            SqlArg::Text(step.state.as_storage_str().to_string()),
                            SqlArg::OptText(step.hold_reason.clone()),
                            SqlArg::OptText(step.error.clone()),
                            SqlArg::Timestamp(step.updated_at),
                            SqlArg::OptTimestamp(step.finished_at),
                            SqlArg::Text(step.key.candidate_id.clone()),
                            SqlArg::I64(step.key.match_generation),
                            SqlArg::I64(step.key.revision_number),
                            SqlArg::Text(step.key.step_id.clone()),
                            SqlArg::Text(
                                MaintenanceActionStepState::Running
                                    .as_storage_str()
                                    .to_string(),
                            ),
                            SqlArg::Text(expected_lease_id),
                        ],
                    )
                } else {
                    (
                        "UPDATE maintenance_action_steps
                        SET before_state_json = {}, target_identity_json = {}, provenance_json = {},
                            updated_at = {}
                      WHERE candidate_id = {} AND match_generation = {} AND revision_number = {}
                        AND step_id = {} AND state = {} AND lease_id = {}",
                        vec![
                            SqlArg::Text(step.before_state_json.clone()),
                            SqlArg::Text(step.target_identity_json.clone()),
                            SqlArg::Text(step.provenance_json.clone()),
                            SqlArg::Timestamp(step.updated_at),
                            SqlArg::Text(step.key.candidate_id.clone()),
                            SqlArg::I64(step.key.match_generation),
                            SqlArg::I64(step.key.revision_number),
                            SqlArg::Text(step.key.step_id.clone()),
                            SqlArg::Text(
                                MaintenanceActionStepState::Running
                                    .as_storage_str()
                                    .to_string(),
                            ),
                            SqlArg::Text(expected_lease_id),
                        ],
                    )
                };
                if SqlRuntime::execute(SqlExec::Tx(tx), sql, &args).await? != 1 {
                    return Ok(false);
                }
                update_open_action_step_attempt(tx, &step, finish).await?;
                Ok(true)
            })
        },
    )
    .await
}

async fn update_open_action_step_attempt(
    tx: &mut SqlTx<'_>,
    step: &MaintenanceActionStepRun,
    finish: bool,
) -> AppResult<()> {
    let (sql, args) = if finish {
        (
            "UPDATE maintenance_action_step_attempts
                SET before_state_json = {}, target_identity_json = {}, provenance_json = {}, state = {},
                    hold_reason = {}, error = {}, finished_at = {}, updated_at = {}
              WHERE id = (
                  SELECT id FROM maintenance_action_step_attempts
                   WHERE candidate_id = {} AND match_generation = {} AND revision_number = {}
                     AND step_id = {} AND finished_at IS NULL
                   ORDER BY started_at DESC, id DESC LIMIT 1
              )",
            vec![
                SqlArg::Text(step.before_state_json.clone()),
                SqlArg::Text(step.target_identity_json.clone()),
                SqlArg::Text(step.provenance_json.clone()),
                SqlArg::Text(step.state.as_storage_str().to_string()),
                SqlArg::OptText(step.hold_reason.clone()),
                SqlArg::OptText(step.error.clone()),
                SqlArg::OptTimestamp(step.finished_at),
                SqlArg::Timestamp(step.updated_at),
                SqlArg::Text(step.key.candidate_id.clone()),
                SqlArg::I64(step.key.match_generation),
                SqlArg::I64(step.key.revision_number),
                SqlArg::Text(step.key.step_id.clone()),
            ],
        )
    } else {
        (
            "UPDATE maintenance_action_step_attempts
                SET before_state_json = {}, target_identity_json = {}, provenance_json = {}, updated_at = {}
              WHERE id = (
                  SELECT id FROM maintenance_action_step_attempts
                   WHERE candidate_id = {} AND match_generation = {} AND revision_number = {}
                     AND step_id = {} AND finished_at IS NULL
                   ORDER BY started_at DESC, id DESC LIMIT 1
              )",
            vec![
                SqlArg::Text(step.before_state_json.clone()),
                SqlArg::Text(step.target_identity_json.clone()),
                SqlArg::Text(step.provenance_json.clone()),
                SqlArg::Timestamp(step.updated_at),
                SqlArg::Text(step.key.candidate_id.clone()),
                SqlArg::I64(step.key.match_generation),
                SqlArg::I64(step.key.revision_number),
                SqlArg::Text(step.key.step_id.clone()),
            ],
        )
    };
    if SqlRuntime::execute(SqlExec::Tx(tx), sql, &args).await? != 1 {
        return Err(AppError::Repository(
            "maintenance action step lost its current immutable attempt".into(),
        ));
    }
    Ok(())
}

fn action_job_receipt_args(receipt: &MaintenanceActionJobReceipt) -> Vec<SqlArg> {
    vec![
        SqlArg::Text(receipt.key.candidate_id.clone()),
        SqlArg::I64(receipt.key.match_generation),
        SqlArg::I64(receipt.key.revision_number),
        SqlArg::Text(receipt.key.step_id.clone()),
        SqlArg::I64(receipt.dispatch_attempt),
        SqlArg::I32(receipt.schema_version as i32),
        SqlArg::Text(receipt.logical_request_key.clone()),
        SqlArg::Text(receipt.request_hash.clone()),
        SqlArg::OptText(receipt.job_run_id.clone()),
        SqlArg::Text(receipt.state.as_storage_str().to_string()),
        SqlArg::Text(receipt.reconciliation_evidence_json.clone()),
        SqlArg::Timestamp(receipt.created_at),
        SqlArg::Timestamp(receipt.updated_at),
    ]
}

async fn transition_action_job_receipt(
    datastore: &StoreDatastore,
    transition: &MaintenanceActionJobReceiptTransition,
) -> AppResult<bool> {
    if transition.expected_states.is_empty() {
        return Err(AppError::Validation(
            "a maintenance job receipt transition must name expected states".into(),
        ));
    }
    if matches!(
        transition.next_state,
        MaintenanceActionJobReceiptState::Accepted | MaintenanceActionJobReceiptState::Completed
    ) && transition
        .job_run_id
        .as_deref()
        .is_none_or(|job_run_id| job_run_id.trim().is_empty())
    {
        return Err(AppError::Validation(
            "accepted maintenance job receipts require a real job run id".into(),
        ));
    }
    let mut args = vec![
        SqlArg::Text(transition.next_state.as_storage_str().to_string()),
        SqlArg::OptText(transition.job_run_id.clone()),
        SqlArg::Text(transition.reconciliation_evidence_json.clone()),
        SqlArg::Timestamp(transition.updated_at),
    ];
    args.extend(action_step_key_args(&transition.key));
    args.push(SqlArg::I64(transition.dispatch_attempt));
    let states = transition
        .expected_states
        .iter()
        .map(|_| "{}")
        .collect::<Vec<_>>()
        .join(", ");
    args.extend(
        transition
            .expected_states
            .iter()
            .map(|state| SqlArg::Text(state.as_storage_str().to_string())),
    );
    let sql = format!(
        "UPDATE maintenance_action_job_receipts
            SET state = {{}}, job_run_id = {{}}, reconciliation_evidence_json = {{}}, updated_at = {{}}
          WHERE candidate_id = {{}} AND match_generation = {{}} AND revision_number = {{}}
            AND step_id = {{}} AND dispatch_attempt = {{}} AND state IN ({states})"
    );
    Ok(SqlRuntime::execute_write(
        datastore,
        "transition_maintenance_action_job_receipt",
        &sql,
        args,
    )
    .await?
        == 1)
}

fn validate_terminal_membership(
    membership: &MaintenanceSequenceTerminalMembership,
    terminal_candidate_state: MaintenanceCandidateState,
) -> AppResult<()> {
    let expected_state = match membership.outcome {
        MaintenanceSequenceTerminalOutcome::Succeeded => MaintenanceCandidateState::Succeeded,
        MaintenanceSequenceTerminalOutcome::Failed => MaintenanceCandidateState::Failed,
    };
    if terminal_candidate_state != expected_state {
        return Err(AppError::Validation(
            "maintenance sequence terminal marker and candidate terminal state disagree".into(),
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
            "maintenance sequence terminal marker has invalid expected step ids".into(),
        ));
    }
    Ok(())
}

fn same_terminal_membership(
    left: &MaintenanceSequenceTerminalMembership,
    right: &MaintenanceSequenceTerminalMembership,
) -> bool {
    left.candidate_id == right.candidate_id
        && left.rule_set_id == right.rule_set_id
        && left.revision_number == right.revision_number
        && left.matcher_content_hash == right.matcher_content_hash
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

async fn verify_terminal_step_evidence(
    tx: &mut SqlTx<'_>,
    membership: &MaintenanceSequenceTerminalMembership,
) -> AppResult<()> {
    let terminal_index = membership.terminal_step_id.as_deref().and_then(|id| {
        membership
            .expected_step_ids
            .iter()
            .position(|expected| expected == id)
    });
    let expected_completed = match membership.outcome {
        MaintenanceSequenceTerminalOutcome::Succeeded => {
            if membership.terminal_step_id.is_some()
                || membership.completed_step_ids != membership.expected_step_ids
            {
                return Err(AppError::Validation(
                    "successful maintenance sequence marker lacks complete step evidence".into(),
                ));
            }
            membership.expected_step_ids.as_slice()
        }
        MaintenanceSequenceTerminalOutcome::Failed => {
            let Some(index) = terminal_index else {
                return Err(AppError::Validation(
                    "failed maintenance sequence marker lacks its failed step id".into(),
                ));
            };
            let expected = &membership.expected_step_ids[..=index];
            if membership.completed_step_ids != expected {
                return Err(AppError::Validation(
                    "failed maintenance sequence marker does not preserve its step prefix".into(),
                ));
            }
            expected
        }
    };
    for (index, step_id) in expected_completed.iter().enumerate() {
        let row = SqlRuntime::fetch_optional(
            SqlExec::Tx(tx),
            "SELECT state FROM maintenance_action_steps
              WHERE candidate_id = {} AND match_generation = {} AND revision_number = {} AND step_id = {}",
            &[
                SqlArg::Text(membership.candidate_id.clone()),
                SqlArg::I64(membership.match_generation),
                SqlArg::I64(membership.revision_number),
                SqlArg::Text(step_id.clone()),
            ],
        )
        .await?
        .ok_or_else(|| AppError::Repository("maintenance terminal marker references a missing step".into()))?;
        let state =
            MaintenanceActionStepState::parse_storage(&row.text("state")?).ok_or_else(|| {
                AppError::Repository(
                    "maintenance terminal marker references an unknown step state".into(),
                )
            })?;
        let is_failed_terminal = membership.outcome == MaintenanceSequenceTerminalOutcome::Failed
            && index + 1 == expected_completed.len();
        if (is_failed_terminal && state != MaintenanceActionStepState::Failed)
            || (!is_failed_terminal && !state.is_completion_success())
        {
            return Err(AppError::Validation(
                "maintenance terminal marker does not match persisted step evidence".into(),
            ));
        }
    }
    Ok(())
}

fn terminal_membership_args(
    membership: &MaintenanceSequenceTerminalMembership,
) -> AppResult<Vec<SqlArg>> {
    Ok(vec![
        SqlArg::Text(membership.id.clone()),
        SqlArg::Text(membership.candidate_id.clone()),
        SqlArg::Text(membership.rule_set_id.clone()),
        SqlArg::I64(membership.revision_number),
        SqlArg::Text(membership.matcher_content_hash.clone()),
        SqlArg::Text(membership.title_id.clone()),
        SqlArg::Text(membership.subject_kind.clone()),
        SqlArg::Text(membership.subject_id.clone()),
        SqlArg::I64(membership.match_generation),
        SqlArg::Text(membership.sequence_content_hash.clone()),
        SqlArg::Text(membership.outcome.as_storage_str().to_string()),
        SqlArg::Text(canonical_json_text(&membership.expected_step_ids)?),
        SqlArg::Text(canonical_json_text(&membership.completed_step_ids)?),
        SqlArg::OptText(membership.terminal_step_id.clone()),
        SqlArg::Timestamp(membership.created_at),
        SqlArg::OptTimestamp(membership.released_at),
    ])
}

fn row_to_action_step(row: &SqlRow) -> AppResult<MaintenanceActionStepRun> {
    let state_raw = row.text("state")?;
    Ok(MaintenanceActionStepRun {
        key: MaintenanceActionStepKey {
            candidate_id: row.text("candidate_id")?,
            match_generation: row.i64("match_generation")?,
            revision_number: row.i64("revision_number")?,
            step_id: row.text("step_id")?,
        },
        rule_set_id: row.text("rule_set_id")?,
        title_id: row.text("title_id")?,
        subject_kind: row.text("subject_kind")?,
        subject_id: row.text("subject_id")?,
        sequence_content_hash: row.text("sequence_content_hash")?,
        step_kind: row.text("step_kind")?,
        intent_json: row.text("intent_json")?,
        before_state_json: row.text("before_state_json")?,
        target_identity_json: row.text("target_identity_json")?,
        provenance_json: row.text("provenance_json")?,
        state: MaintenanceActionStepState::parse_storage(&state_raw).ok_or_else(|| {
            AppError::Repository(format!(
                "unknown maintenance action step state: {state_raw}"
            ))
        })?,
        attempt: row.i64("attempt")?,
        lease_id: row.opt_text("lease_id")?,
        lease_expires_at: row.opt_timestamp("lease_expires_at")?,
        hold_reason: row.opt_text("hold_reason")?,
        error: row.opt_text("error")?,
        created_at: row.timestamp("created_at")?,
        updated_at: row.timestamp("updated_at")?,
        finished_at: row.opt_timestamp("finished_at")?,
    })
}

fn row_to_action_step_attempt(row: &SqlRow) -> AppResult<MaintenanceActionStepAttempt> {
    let state_raw = row.text("state")?;
    Ok(MaintenanceActionStepAttempt {
        id: row.text("id")?,
        key: MaintenanceActionStepKey {
            candidate_id: row.text("candidate_id")?,
            match_generation: row.i64("match_generation")?,
            revision_number: row.i64("revision_number")?,
            step_id: row.text("step_id")?,
        },
        attempt: row.i64("attempt")?,
        state: MaintenanceActionStepState::parse_storage(&state_raw).ok_or_else(|| {
            AppError::Repository(format!(
                "unknown maintenance action attempt state: {state_raw}"
            ))
        })?,
        intent_json: row.text("intent_json")?,
        before_state_json: row.text("before_state_json")?,
        target_identity_json: row.text("target_identity_json")?,
        provenance_json: row.text("provenance_json")?,
        hold_reason: row.opt_text("hold_reason")?,
        error: row.opt_text("error")?,
        started_at: row.timestamp("started_at")?,
        finished_at: row.opt_timestamp("finished_at")?,
        created_at: row.timestamp("created_at")?,
        updated_at: row.timestamp("updated_at")?,
    })
}

fn row_to_action_job_receipt(row: &SqlRow) -> AppResult<MaintenanceActionJobReceipt> {
    let state_raw = row.text("state")?;
    let receipt = MaintenanceActionJobReceipt {
        schema_version: row.i32("schema_version")? as u32,
        key: MaintenanceActionStepKey {
            candidate_id: row.text("candidate_id")?,
            match_generation: row.i64("match_generation")?,
            revision_number: row.i64("revision_number")?,
            step_id: row.text("step_id")?,
        },
        dispatch_attempt: row.i64("dispatch_attempt")?,
        logical_request_key: row.text("logical_request_key")?,
        request_hash: row.text("request_hash")?,
        job_run_id: row.opt_text("job_run_id")?,
        state: MaintenanceActionJobReceiptState::parse_storage(&state_raw).ok_or_else(|| {
            AppError::Repository(format!(
                "unknown maintenance action job receipt state: {state_raw}"
            ))
        })?,
        reconciliation_evidence_json: row.text("reconciliation_evidence_json")?,
        created_at: row.timestamp("created_at")?,
        updated_at: row.timestamp("updated_at")?,
    };
    receipt
        .validate_schema()
        .map_err(|error| AppError::Repository(error.to_string()))?;
    Ok(receipt)
}

fn row_to_sequence_terminal_membership(
    row: &SqlRow,
) -> AppResult<MaintenanceSequenceTerminalMembership> {
    let outcome_raw = row.text("outcome")?;
    let expected_raw = row.text("expected_step_ids_json")?;
    let completed_raw = row.text("completed_step_ids_json")?;
    Ok(MaintenanceSequenceTerminalMembership {
        id: row.text("id")?,
        candidate_id: row.text("candidate_id")?,
        rule_set_id: row.text("rule_set_id")?,
        revision_number: row.i64("revision_number")?,
        matcher_content_hash: row.text("matcher_content_hash")?,
        title_id: row.text("title_id")?,
        subject_kind: row.text("subject_kind")?,
        subject_id: row.text("subject_id")?,
        match_generation: row.i64("match_generation")?,
        sequence_content_hash: row.text("sequence_content_hash")?,
        outcome: MaintenanceSequenceTerminalOutcome::parse_storage(&outcome_raw).ok_or_else(
            || {
                AppError::Repository(format!(
                    "unknown maintenance sequence terminal outcome: {outcome_raw}"
                ))
            },
        )?,
        expected_step_ids: serde_json::from_str(&expected_raw).map_err(|error| {
            AppError::Repository(format!("invalid maintenance expected step ids: {error}"))
        })?,
        completed_step_ids: serde_json::from_str(&completed_raw).map_err(|error| {
            AppError::Repository(format!("invalid maintenance completed step ids: {error}"))
        })?,
        terminal_step_id: row.opt_text("terminal_step_id")?,
        created_at: row.timestamp("created_at")?,
        released_at: row.opt_timestamp("released_at")?,
    })
}

fn row_to_action_run(row: &SqlRow) -> AppResult<LifecycleActionRun> {
    Ok(LifecycleActionRun {
        id: row.text("id")?,
        candidate_id: row.text("candidate_id")?,
        rule_set_id: row.text("rule_set_id")?,
        revision_number: row.i64("revision_number")?,
        title_id: row.text("title_id")?,
        subject_kind: row.text("subject_kind")?,
        subject_id: row.text("subject_id")?,
        action_kind: row.text("action_kind")?,
        match_generation: row.i64("match_generation")?,
        idempotency_key: row.text("idempotency_key")?,
        attempt: row.i64("attempt")?,
        status: LifecycleActionRunStatus::parse_storage(&row.text("status")?).unwrap_or_default(),
        hold_reason: row.opt_text("hold_reason")?,
        error: row.opt_text("error")?,
        detail: row.text("detail")?,
        started_at: row.timestamp("started_at")?,
        finished_at: row.opt_timestamp("finished_at")?,
        created_at: row.timestamp("created_at")?,
    })
}

fn row_to_candidate(row: &SqlRow) -> AppResult<LifecycleCandidate> {
    Ok(LifecycleCandidate {
        id: row.text("id")?,
        rule_set_id: row.text("rule_set_id")?,
        revision_number: row.i64("revision_number")?,
        matcher_content_hash: row.text("matcher_content_hash")?,
        title_id: row.text("title_id")?,
        library_id: row.text("library_id")?,
        facet: row.text("facet")?,
        subject_kind: row.text("subject_kind")?,
        subject_id: row.text("subject_id")?,
        match_generation: row.i64("match_generation")?,
        // An unrecognized stored state was written by a newer build. Reading it
        // back as `Blocked` would be a lie about what happened; `Observing` is
        // the state that keeps the row visible and acts on nothing, which is
        // the only safe reading a dark build can give it.
        state: MaintenanceCandidateState::parse_storage(&row.text("state")?).unwrap_or_default(),
        state_reason: row.text("state_reason")?,
        reason_codes: reason_codes(row)?,
        action_kind: row.text("action_kind")?,
        grace_days: row.i64("grace_days")?,
        first_matched_at: row.timestamp("first_matched_at")?,
        last_matched_at: row.timestamp("last_matched_at")?,
        due_at: row.timestamp("due_at")?,
        last_evaluated_at: row.timestamp("last_evaluated_at")?,
        held_since: row.opt_timestamp("held_since")?,
        action_attempts: row.i64("action_attempts")?,
        created_at: row.timestamp("created_at")?,
        updated_at: row.timestamp("updated_at")?,
    })
}

/// Stored as a JSON text array; anything unreadable degrades to no codes rather
/// than failing the whole listing.
fn reason_codes(row: &SqlRow) -> AppResult<Vec<String>> {
    let raw = json_text_or(row, "reason_codes", "[]")?;
    Ok(serde_json::from_str(&raw).unwrap_or_default())
}

fn row_to_exclusion(row: &SqlRow) -> AppResult<MaintenanceRuleExclusion> {
    Ok(MaintenanceRuleExclusion {
        id: row.text("id")?,
        rule_set_id: row.opt_text("rule_set_id")?,
        title_id: row.text("title_id")?,
        subject_kind: scryer_domain::MaintenanceRuleSubjectKind::parse_storage(
            &row.text("subject_kind")?,
        )
        .ok_or_else(|| AppError::Repository("invalid maintenance exclusion scope".into()))?,
        subject_id: row.text("subject_id")?,
        reason: row.text("reason")?,
        created_by: row.opt_text("created_by")?,
        created_at: row.timestamp("created_at")?,
    })
}

fn row_to_evaluation_run(row: &SqlRow) -> AppResult<MaintenanceEvaluationRun> {
    Ok(MaintenanceEvaluationRun {
        id: row.text("id")?,
        rule_set_id: row.text("rule_set_id")?,
        revision_number: row.i64("revision_number")?,
        matcher_content_hash: row.text("matcher_content_hash")?,
        started_at: row.timestamp("started_at")?,
        finished_at: row.opt_timestamp("finished_at")?,
        status: MaintenanceEvaluationRunStatus::parse_storage(&row.text("status")?)
            .unwrap_or_default(),
        evaluated_count: row.i64("evaluated_count")?,
        matched_count: row.i64("matched_count")?,
        no_match_count: row.i64("no_match_count")?,
        unknown_count: row.i64("unknown_count")?,
        error_count: row.i64("error_count")?,
        canceled_candidates: row.i64("canceled_candidates")?,
        superseded_candidates: row.i64("superseded_candidates")?,
        duration_ms: row.opt_i64("duration_ms")?,
        error: row.opt_text("error")?,
    })
}

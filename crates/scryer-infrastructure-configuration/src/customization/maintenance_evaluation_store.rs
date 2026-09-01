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
use scryer_application::{
    AppError, AppResult, LifecycleActionRunRepository, MaintenanceCandidateQuery,
    MaintenanceCandidateRepository, MaintenanceEvaluationRunRepository,
    MaintenanceExclusionRepository,
};
use scryer_domain::{
    LifecycleActionRun, LifecycleActionRunStatus, LifecycleCandidate,
    MAINTENANCE_TERMINAL_CANDIDATE_STATES, MaintenanceCandidateState, MaintenanceEvaluationRun,
    MaintenanceEvaluationRunStatus, MaintenanceRuleExclusion,
};

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, StoreDatastore};
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
    library_id, facet, subject_kind, match_generation, state, state_reason, reason_codes,
    action_kind, grace_days, first_matched_at, last_matched_at, due_at, last_evaluated_at,
    held_since, action_attempts, created_at, updated_at";

const INSERT_CANDIDATE_SQL: &str = "INSERT INTO lifecycle_candidates
        (id, rule_set_id, revision_number, matcher_content_hash, title_id, library_id, facet,
         subject_kind, match_generation, state, state_reason, reason_codes, action_kind,
         grace_days, first_matched_at, last_matched_at, due_at, last_evaluated_at, held_since,
         action_attempts, created_at, updated_at)
     VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})";

const ACTION_RUN_COLUMNS: &str = "id, candidate_id, rule_set_id, revision_number, title_id,
    action_kind, match_generation, idempotency_key, attempt, status, hold_reason, error,
    detail, started_at, finished_at, created_at";

const INSERT_ACTION_RUN_SQL: &str = "INSERT INTO lifecycle_action_runs
        (id, candidate_id, rule_set_id, revision_number, title_id, action_kind,
         match_generation, idempotency_key, attempt, status, hold_reason, error, detail,
         started_at, finished_at, created_at)
     VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})";

const EXCLUSION_COLUMNS: &str = "id, rule_set_id, title_id, reason, created_by, created_at";

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
    async fn get_active_candidate(
        &self,
        rule_set_id: &str,
        title_id: &str,
    ) -> AppResult<Option<LifecycleCandidate>> {
        let (predicate, mut args) = active_state_predicate();
        let sql = format!(
            "SELECT {CANDIDATE_COLUMNS}
               FROM lifecycle_candidates
              WHERE rule_set_id = {{}} AND title_id = {{}} AND {predicate}
              ORDER BY match_generation DESC
              LIMIT 1"
        );
        let mut bound = vec![
            SqlArg::Text(rule_set_id.to_string()),
            SqlArg::Text(title_id.to_string()),
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

    async fn max_match_generation(&self, rule_set_id: &str, title_id: &str) -> AppResult<i64> {
        // Terminal rows count: generations are monotonic per subject, so a
        // cancel-then-rematch is always distinguishable from a continuation.
        let sql = "SELECT match_generation
                     FROM lifecycle_candidates
                    WHERE rule_set_id = {} AND title_id = {}
                    ORDER BY match_generation DESC
                    LIMIT 1";
        Ok(SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            sql,
            &[
                SqlArg::Text(rule_set_id.to_string()),
                SqlArg::Text(title_id.to_string()),
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
              WHERE rule_set_id = {{}} AND title_id = {{}} AND {predicate}
              LIMIT 1"
        );
        let mut exists_args = vec![
            SqlArg::Text(candidate.rule_set_id.clone()),
            SqlArg::Text(candidate.title_id.clone()),
        ];
        exists_args.append(&mut predicate_args);
        let insert_args = candidate_args(candidate)?;
        let rule_set_id = candidate.rule_set_id.clone();
        let title_id = candidate.title_id.clone();

        SqlRuntime::run_in_transaction(&self.datastore, "create_lifecycle_candidate", move |tx| {
            let exists_sql = exists_sql.clone();
            let exists_args = exists_args.clone();
            let insert_args = insert_args.clone();
            let rule_set_id = rule_set_id.clone();
            let title_id = title_id.clone();
            Box::pin(async move {
                if SqlRuntime::fetch_optional(SqlExec::Tx(tx), &exists_sql, &exists_args)
                    .await?
                    .is_some()
                {
                    return Err(AppError::Validation(format!(
                        "maintenance rule {rule_set_id} already has an active candidate for title {title_id}"
                    )));
                }
                SqlRuntime::execute(SqlExec::Tx(tx), INSERT_CANDIDATE_SQL, &insert_args).await?;
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
        updated_at: DateTime<Utc>,
    ) -> AppResult<()> {
        let args = vec![
            SqlArg::Text(state.as_storage_str().to_string()),
            SqlArg::Text(state_reason.to_string()),
            SqlArg::Timestamp(updated_at),
            SqlArg::Timestamp(updated_at),
            SqlArg::Text(id.to_string()),
        ];
        execute_write(
            &self.datastore,
            "transition_lifecycle_candidate_state",
            "UPDATE lifecycle_candidates
                SET state = {}, state_reason = {}, last_evaluated_at = {}, updated_at = {}
              WHERE id = {}",
            args,
        )
        .await
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
        limit: usize,
    ) -> AppResult<Vec<LifecycleCandidate>> {
        let sql = format!(
            "SELECT {CANDIDATE_COLUMNS}
               FROM lifecycle_candidates
              WHERE rule_set_id = {{}} AND due_at <= {{}}
                AND state IN ({{}}, {{}}, {{}}, {{}})
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
                (id, rule_set_id, title_id, reason, created_by, created_at)
             VALUES ({}, {}, {}, {}, {}, {})",
            vec![
                SqlArg::Text(exclusion.id.clone()),
                SqlArg::OptText(exclusion.rule_set_id.clone()),
                SqlArg::Text(exclusion.title_id.clone()),
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

fn row_to_action_run(row: &SqlRow) -> AppResult<LifecycleActionRun> {
    Ok(LifecycleActionRun {
        id: row.text("id")?,
        candidate_id: row.text("candidate_id")?,
        rule_set_id: row.text("rule_set_id")?,
        revision_number: row.i64("revision_number")?,
        title_id: row.text("title_id")?,
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

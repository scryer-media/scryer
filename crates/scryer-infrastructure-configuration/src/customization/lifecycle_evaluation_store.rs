//! Dual-dialect storage for the lifecycle evaluator's three tables:
//! lifecycle candidates, rule exclusions, and per-rule
//! evaluation runs.
//!
//! The store owns one invariant the callers must not have to remember: a rule
//! set holds at most one non-terminal candidate per title. The migration states
//! it as a partial unique index in both dialects; [`LifecycleEvaluationStore`]
//! states it again as a checked read inside the insert's transaction, so the
//! failure is a named application error rather than a raw constraint violation.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scryer_application::lifecycle_rules::ACTION_SEQUENCE_KIND;
use scryer_application::{
    AppError, AppResult, LifecycleActionJobReceiptClaim, LifecycleActionJobReceiptTransition,
    LifecycleActionRunRepository, LifecycleActionStepCandidateKey, LifecycleActionStepClaim,
    LifecycleActionStepRepository, LifecycleCandidateMatch, LifecycleCandidateQuery,
    LifecycleCandidateRepository, LifecycleEvaluationRunRepository, LifecycleExclusionRepository,
    LifecycleRuleScheduleAnchor, LifecycleSequenceCompletionRepository,
};
use scryer_domain::{
    LIFECYCLE_TERMINAL_CANDIDATE_STATES, LifecycleActionJobReceipt, LifecycleActionJobReceiptState,
    LifecycleActionRun, LifecycleActionRunStatus, LifecycleActionStepKey, LifecycleActionStepRun,
    LifecycleActionStepState, LifecycleCandidate, LifecycleCandidateState, LifecycleEvaluationRun,
    LifecycleEvaluationRunStatus, LifecycleRuleExclusion, LifecycleSequenceTerminalMembership,
    LifecycleSequenceTerminalOutcome,
};

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, SqlTx, StoreDatastore};
use crate::storage::sql::json::{canonical_json_text, json_text_or};
use scryer_rules::lifecycle_actions::LifecycleActionHead;

#[derive(Clone)]
pub struct LifecycleEvaluationStore {
    datastore: StoreDatastore,
}

impl LifecycleEvaluationStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

// ── Column lists and shared SQL ─────────────────────────────────────────────

const CANDIDATE_COLUMNS: &str = "id, rule_set_id, revision_number, source_content_hash, title_id,
    library_id, facet, subject_kind, subject_id, match_generation, state, state_reason, reason_codes,
    action_kind, grace_days, first_matched_at, last_matched_at, due_at, last_evaluated_at,
    held_since, action_attempts, created_at, updated_at, decided_actions_json, actions_conflict_json,
    state_cause_code";

const INSERT_CANDIDATE_SQL: &str = "INSERT INTO lifecycle_candidates
        (id, rule_set_id, revision_number, source_content_hash, title_id, library_id, facet,
         subject_kind, subject_id, match_generation, state, state_reason, reason_codes, action_kind,
         grace_days, first_matched_at, last_matched_at, due_at, last_evaluated_at, held_since,
         action_attempts, created_at, updated_at, decided_actions_json, actions_conflict_json,
         state_cause_code)
     VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})";

const ACTION_RUN_COLUMNS: &str =
    "id, candidate_id, rule_set_id, revision_number, title_id, subject_kind, subject_id,
    action_kind, match_generation, idempotency_key, attempt, status, hold_reason, error,
    detail, started_at, finished_at, created_at, cause_code";

const INSERT_ACTION_RUN_SQL: &str = "INSERT INTO lifecycle_action_runs
        (id, candidate_id, rule_set_id, revision_number, title_id, subject_kind, subject_id, action_kind,
         match_generation, idempotency_key, attempt, status, hold_reason, error, detail,
         started_at, finished_at, created_at, cause_code)
     VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})";

const ACTION_STEP_COLUMNS: &str = "candidate_id, match_generation, revision_number, step_id,
    rule_set_id, title_id, subject_kind, subject_id, sequence_content_hash, step_kind,
    intent_json, before_state_json, target_identity_json, provenance_json, state, attempt,
    lease_id, lease_expires_at, hold_reason, error, created_at, updated_at, finished_at, cause_code";

const ACTION_JOB_RECEIPT_COLUMNS: &str = "candidate_id, match_generation, revision_number,
    step_id, dispatch_attempt, schema_version, logical_request_key, request_hash, job_run_id,
    state, reconciliation_evidence_json, created_at, updated_at";

const SEQUENCE_TERMINAL_MEMBERSHIP_COLUMNS: &str = "id, candidate_id, rule_set_id,
    revision_number, source_content_hash, title_id, subject_kind, subject_id, match_generation,
    sequence_content_hash, outcome, expected_step_ids_json, completed_step_ids_json,
    terminal_step_id, created_at, released_at";

const EXCLUSION_COLUMNS: &str =
    "id, rule_set_id, title_id, subject_kind, subject_id, reason, created_by, created_at";

const EVALUATION_RUN_COLUMNS: &str = "id, rule_set_id, revision_number, source_content_hash,
    started_at, finished_at, status, evaluated_count, matched_count, no_match_count,
    unknown_count, error_count, canceled_candidates, superseded_candidates, duration_ms, error";

/// Ids bound per statement by the id-list candidate reads. Far below both
/// dialects' bind limits, so a caller-sized list always fits by chunking; a
/// list that fits in one chunk stays one statement.
const CANDIDATE_ID_LIST_CHUNK: usize = 500;

fn id_placeholders(count: usize) -> String {
    vec!["{}"; count].join(", ")
}

/// `state NOT IN (…)` over the terminal set, as a placeholder list plus its
/// bound arguments. Built from the domain constant so the predicate here and
/// the migration's partial unique index can never drift apart.
fn active_state_predicate() -> (String, Vec<SqlArg>) {
    let placeholders = LIFECYCLE_TERMINAL_CANDIDATE_STATES
        .iter()
        .map(|_| "{}")
        .collect::<Vec<_>>()
        .join(", ");
    let args = LIFECYCLE_TERMINAL_CANDIDATE_STATES
        .iter()
        .map(|state| SqlArg::Text((*state).to_string()))
        .collect();
    (format!("state NOT IN ({placeholders})"), args)
}

/// A subquery yielding each string of a JSON array bound as one text
/// parameter.
fn json_text_array_values_sql(datastore: &StoreDatastore) -> &'static str {
    match datastore {
        StoreDatastore::Sqlite { .. } => "SELECT value FROM json_each({})",
        StoreDatastore::Postgres { .. } => {
            "SELECT value FROM jsonb_array_elements_text(CAST({} AS JSONB)) AS rule_set_ids(value)"
        }
    }
}

/// Reads of the stored decided-actions document, `decided_actions_json`, in
/// the datastore's dialect. Each yields SQL NULL or false for a candidate
/// without a document; the callers guard on the document being present.
struct DecidedActionsJson {
    postgres: bool,
}

impl DecidedActionsJson {
    fn new(datastore: &StoreDatastore) -> Self {
        Self {
            postgres: matches!(datastore, StoreDatastore::Postgres { .. }),
        }
    }

    /// The boolean `key` is true.
    fn is_true(&self, key: &str) -> String {
        if self.postgres {
            format!("(CAST(decided_actions_json AS JSONB) ->> '{key}') = 'true'")
        } else {
            format!("json_extract(decided_actions_json, '$.{key}') = 1")
        }
    }

    /// The boolean `key` is false or absent.
    fn is_not_true(&self, key: &str) -> String {
        if self.postgres {
            format!("COALESCE(CAST(decided_actions_json AS JSONB) ->> '{key}', 'false') <> 'true'")
        } else {
            format!("COALESCE(json_extract(decided_actions_json, '$.{key}'), 0) <> 1")
        }
    }

    /// The optional `key` is present and not null.
    fn is_set(&self, key: &str) -> String {
        if self.postgres {
            format!("(CAST(decided_actions_json AS JSONB) ->> '{key}') IS NOT NULL")
        } else {
            format!("json_extract(decided_actions_json, '$.{key}') IS NOT NULL")
        }
    }

    /// The list `key` holds at least one value.
    fn is_non_empty_list(&self, key: &str) -> String {
        if self.postgres {
            format!(
                "(jsonb_typeof(CAST(decided_actions_json AS JSONB) -> '{key}') = 'array'
                  AND CAST(decided_actions_json AS JSONB) -> '{key}' <> CAST('[]' AS JSONB))"
            )
        } else {
            format!("COALESCE(json_array_length(decided_actions_json, '$.{key}'), 0) > 0")
        }
    }
}

/// SQL that is true when a candidate's stored decided actions include `head`.
/// It states the rule of
/// [`scryer_application::lifecycle_rules::decided_actions_include_head`] over
/// the stored document: a title head matches the heads a candidate reports,
/// whatever its subject; a season or episode head also requires a subject of
/// that kind. Deleting files reports an unmonitor first, and deleting the
/// title subsumes deleting its files.
fn decided_action_head_sql(json: &DecidedActionsJson, head: LifecycleActionHead) -> String {
    use LifecycleActionHead as Head;
    let at_level = |level: &str, base: String| format!("(subject_kind = '{level}' AND {base})");
    let deletes_files = || {
        format!(
            "({} AND {})",
            json.is_true("delete_files"),
            json.is_not_true("delete_title")
        )
    };
    let unmonitors = || format!("({} OR {})", json.is_true("unmonitor"), deletes_files());
    match head {
        Head::Unmonitor => unmonitors(),
        Head::DeleteFiles => deletes_files(),
        Head::DeleteTitle => json.is_true("delete_title"),
        Head::Search => json.is_true("search"),
        Head::SetQualityProfile => json.is_set("set_quality_profile"),
        Head::AddTags => json.is_non_empty_list("add_tags"),
        Head::RemoveTags => json.is_non_empty_list("remove_tags"),
        Head::Monitor => json.is_true("monitor"),
        Head::FetchSubtitles => json.is_true("fetch_subtitles"),
        Head::MoveToLibrary => json.is_set("move_to_library"),
        Head::MoveToRoot => json.is_set("move_to_root"),
        Head::Notify => json.is_set("notify"),
        Head::MonitorSeason => at_level("season", json.is_true("monitor")),
        Head::UnmonitorSeason => at_level("season", unmonitors()),
        Head::DeleteSeasonFiles => at_level("season", deletes_files()),
        Head::FetchSeasonSubtitles => at_level("season", json.is_true("fetch_subtitles")),
        Head::NotifySeason => at_level("season", json.is_set("notify")),
        Head::MonitorEpisode => at_level("episode", json.is_true("monitor")),
        Head::UnmonitorEpisode => at_level("episode", unmonitors()),
        Head::DeleteEpisodeFiles => at_level("episode", deletes_files()),
        Head::FetchEpisodeSubtitles => at_level("episode", json.is_true("fetch_subtitles")),
        Head::NotifyEpisode => at_level("episode", json.is_set("notify")),
    }
}

/// SQL that is true when a candidate's decided actions include any of
/// `heads`. A candidate without a decided-actions document matches none.
fn decided_action_heads_predicate(
    datastore: &StoreDatastore,
    heads: &[LifecycleActionHead],
) -> String {
    let json = DecidedActionsJson::new(datastore);
    let mut seen = Vec::with_capacity(heads.len());
    let alternatives = heads
        .iter()
        .filter(|head| {
            let first = !seen.contains(*head);
            seen.push(**head);
            first
        })
        .map(|head| decided_action_head_sql(&json, *head))
        .collect::<Vec<_>>()
        .join(" OR ");
    if json.postgres {
        format!("(decided_actions_json IS NOT NULL AND ({alternatives}))")
    } else {
        // SQLite's JSON functions fail the whole statement on malformed text;
        // the CASE keeps them from reading a document that does not parse,
        // which, like a missing one, decides nothing.
        format!(
            "(decided_actions_json IS NOT NULL
              AND CASE WHEN json_valid(decided_actions_json) THEN ({alternatives}) ELSE 0 END)"
        )
    }
}

// ── Candidates ──────────────────────────────────────────────────────────────

/// A repeat match: the grace clock (`first_matched_at`, `due_at`) is left as
/// it is.
const RECORD_CANDIDATE_MATCH_SQL: &str = "UPDATE lifecycle_candidates
                SET last_matched_at = {}, last_evaluated_at = {}, reason_codes = {},
                    held_since = NULL, updated_at = {}
              WHERE id = {}";

#[async_trait]
impl LifecycleCandidateRepository for LifecycleEvaluationStore {
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

    async fn list_active_subject_candidates(
        &self,
        rule_set_id: &str,
        subject_kind: &str,
        subject_ids: &[String],
    ) -> AppResult<Vec<LifecycleCandidate>> {
        let mut candidates = Vec::new();
        for chunk in subject_ids.chunks(CANDIDATE_ID_LIST_CHUNK) {
            let (predicate, mut state_args) = active_state_predicate();
            let mut args = vec![
                SqlArg::Text(rule_set_id.to_string()),
                SqlArg::Text(subject_kind.to_string()),
            ];
            args.extend(chunk.iter().cloned().map(SqlArg::Text));
            args.append(&mut state_args);
            let sql = format!(
                "SELECT {CANDIDATE_COLUMNS}
                   FROM lifecycle_candidates
                  WHERE rule_set_id = {{}} AND subject_kind = {{}} AND subject_id IN ({})
                    AND {predicate}",
                id_placeholders(chunk.len())
            );
            for row in SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args).await? {
                candidates.push(row_to_candidate(&row)?);
            }
        }
        Ok(candidates)
    }

    async fn get_candidates_by_ids(&self, ids: &[String]) -> AppResult<Vec<LifecycleCandidate>> {
        let mut candidates = Vec::new();
        for chunk in ids.chunks(CANDIDATE_ID_LIST_CHUNK) {
            let args: Vec<SqlArg> = chunk.iter().cloned().map(SqlArg::Text).collect();
            let sql = format!(
                "SELECT {CANDIDATE_COLUMNS} FROM lifecycle_candidates WHERE id IN ({})",
                id_placeholders(chunk.len())
            );
            for row in SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args).await? {
                candidates.push(row_to_candidate(&row)?);
            }
        }
        Ok(candidates)
    }

    async fn list_candidates(
        &self,
        query: &LifecycleCandidateQuery,
    ) -> AppResult<Vec<LifecycleCandidate>> {
        let mut clauses: Vec<String> = Vec::new();
        let mut args: Vec<SqlArg> = Vec::new();

        if let Some(rule_set_id) = query.rule_set_id.as_deref() {
            clauses.push("rule_set_id = {}".to_string());
            args.push(SqlArg::Text(rule_set_id.to_string()));
        }
        if let Some(rule_set_ids) = query.rule_set_ids.as_ref() {
            if rule_set_ids.is_empty() {
                return Ok(Vec::new());
            }
            // One array parameter, so any number of rule sets fits one statement.
            clauses.push(format!(
                "rule_set_id IN ({})",
                json_text_array_values_sql(&self.datastore)
            ));
            args.push(SqlArg::Text(
                serde_json::to_string(rule_set_ids)
                    .map_err(|error| AppError::Repository(error.to_string()))?,
            ));
        }
        if !query.action_heads.is_empty() {
            clauses.push(decided_action_heads_predicate(
                &self.datastore,
                &query.action_heads,
            ));
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
                    return Err(AppError::validation_refused(crate::error_reasons::CONFIGURATION_LIFECYCLE_CANDIDATE_ALREADY_ACTIVE, format!(
                        "lifecycle rule {rule_set_id} already has an active candidate for title {title_id}"
                    )));
                }
                if action_kind == ACTION_SEQUENCE_KIND
                    && SqlRuntime::fetch_optional(
                        SqlExec::Tx(tx),
                        "SELECT id FROM lifecycle_sequence_terminal_memberships
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
                    return Err(AppError::validation_refused(crate::error_reasons::CONFIGURATION_LIFECYCLE_MEMBERSHIP_TERMINAL, "lifecycle sequence membership is terminal until a confirmed no-match".to_string()));
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
                    return Err(AppError::validation_refused(crate::error_reasons::CONFIGURATION_LIFECYCLE_CANDIDATE_ALREADY_ACTIVE, format!(
                        "lifecycle rule {rule_set_id} already has an active candidate for title {title_id}"
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
        // membership never restarts its own grace clock.
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
            RECORD_CANDIDATE_MATCH_SQL,
            args,
        )
        .await
    }

    async fn record_candidate_matches(&self, matches: &[LifecycleCandidateMatch]) -> AppResult<()> {
        if matches.is_empty() {
            return Ok(());
        }
        // The same statement `record_candidate_match` runs, once per row, in
        // one transaction: five binds a statement, so no id list to chunk.
        let mut statements = Vec::with_capacity(matches.len());
        for candidate_match in matches {
            statements.push(vec![
                SqlArg::Timestamp(candidate_match.last_matched_at),
                SqlArg::Timestamp(candidate_match.last_matched_at),
                SqlArg::Text(canonical_json_text(&candidate_match.reason_codes)?),
                SqlArg::Timestamp(candidate_match.updated_at),
                SqlArg::Text(candidate_match.id.clone()),
            ]);
        }
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "record_lifecycle_candidate_matches",
            move |tx| {
                let statements = statements.clone();
                Box::pin(async move {
                    for args in &statements {
                        SqlRuntime::execute(
                            SqlExec::Tx(&mut *tx),
                            RECORD_CANDIDATE_MATCH_SQL,
                            args,
                        )
                        .await?;
                    }
                    Ok(())
                })
            },
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
        state: LifecycleCandidateState,
        state_reason: &str,
        expected_states: &[LifecycleCandidateState],
        updated_at: DateTime<Utc>,
    ) -> AppResult<bool> {
        // An empty expectation would degrade this back into the unconditional
        // UPDATE the compare-and-set exists to remove, so it is refused rather
        // than silently widened.
        if expected_states.is_empty() {
            return Err(AppError::validation_refused(
                crate::error_reasons::CONFIGURATION_CANDIDATE_EXPECTED_STATES_REQUIRED,
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
                SET state = {{}}, state_reason = {{}}, state_cause_code = NULL,
                    last_evaluated_at = {{}}, updated_at = {{}}
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

    async fn set_candidate_actions_conflicts(
        &self,
        changes: &[(String, Option<scryer_domain::LifecycleActionsConflict>)],
        expected_states: &[LifecycleCandidateState],
        updated_at: DateTime<Utc>,
    ) -> AppResult<usize> {
        if expected_states.is_empty() {
            return Err(AppError::validation_refused(
                crate::error_reasons::CONFIGURATION_CANDIDATE_EXPECTED_STATES_REQUIRED,
                "a candidate transition must name the states it expects".to_string(),
            ));
        }
        if changes.is_empty() {
            return Ok(0);
        }
        let state_placeholders = id_placeholders(expected_states.len());
        // One statement per chunk: each row's marker is chosen by a CASE on
        // its id, and the state predicate keeps the compare-and-set per row.
        // A chunk binds three values per row, far below either bind limit.
        let mut statements: Vec<(String, Vec<SqlArg>)> = Vec::new();
        for chunk in changes.chunks(CANDIDATE_ID_LIST_CHUNK) {
            let mut args = Vec::with_capacity(chunk.len() * 3 + expected_states.len() + 1);
            for (id, conflict) in chunk {
                args.push(SqlArg::Text(id.clone()));
                args.push(SqlArg::OptText(actions_conflict_text(conflict.as_ref())?));
            }
            args.push(SqlArg::Timestamp(updated_at));
            args.extend(chunk.iter().map(|(id, _)| SqlArg::Text(id.clone())));
            args.extend(
                expected_states
                    .iter()
                    .map(|state| SqlArg::Text(state.as_storage_str().to_string())),
            );
            let sql = format!(
                "UPDATE lifecycle_candidates
                    SET actions_conflict_json = CASE id {} ELSE actions_conflict_json END,
                        updated_at = {{}}
                  WHERE id IN ({}) AND state IN ({state_placeholders})",
                vec!["WHEN {} THEN {}"; chunk.len()].join(" "),
                id_placeholders(chunk.len()),
            );
            statements.push((sql, args));
        }
        let affected = SqlRuntime::run_in_transaction(
            &self.datastore,
            "set_lifecycle_candidate_actions_conflicts",
            move |tx| {
                let statements = statements.clone();
                Box::pin(async move {
                    let mut affected = 0;
                    for (sql, args) in &statements {
                        affected += SqlRuntime::execute(SqlExec::Tx(&mut *tx), sql, args).await?;
                    }
                    Ok(affected)
                })
            },
        )
        .await?;
        Ok(usize::try_from(affected).unwrap_or(usize::MAX))
    }

    async fn finish_leased_candidate(
        &self,
        id: &str,
        expected_lease_updated_at: DateTime<Utc>,
        state: LifecycleCandidateState,
        state_reason: &str,
        state_cause_code: Option<&str>,
        finished_at: DateTime<Utc>,
    ) -> AppResult<bool> {
        let args = vec![
            SqlArg::Text(state.as_storage_str().to_string()),
            SqlArg::Text(state_reason.to_string()),
            SqlArg::OptText(state_cause_code.map(str::to_string)),
            SqlArg::Timestamp(finished_at),
            SqlArg::Timestamp(finished_at),
            SqlArg::Text(id.to_string()),
            SqlArg::Text(
                LifecycleCandidateState::Executing
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
                SET state = {}, state_reason = {}, state_cause_code = {}, last_evaluated_at = {},
                    updated_at = {}
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

    async fn count_candidates_by_state(
        &self,
        rule_set_id: &str,
    ) -> AppResult<Vec<(LifecycleCandidateState, i64)>> {
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
            if let Some(state) = LifecycleCandidateState::parse_storage(&row.text("state")?) {
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
        held_after: Option<DateTime<Utc>>,
        after: Option<&scryer_application::LifecycleDueCursor>,
        limit: usize,
    ) -> AppResult<Vec<LifecycleCandidate>> {
        // `due_at <= due_before` still bounds both arms: a row that reached
        // `executing` was due when it was leased, so an abandoned lease is
        // always past its due time too. The second arm is what returns that
        // abandoned row — without it, the lease's own reclaim branch is
        // unreachable and the row is stranded for good.
        //
        // The walk is keyset paged on `(due_at, id)`, served by
        // idx_lifecycle_candidates_rule_due.
        let cursor_clause = if after.is_some() {
            " AND (due_at > {} OR (due_at = {} AND id > {}))"
        } else {
            ""
        };
        // A row an action pass held after the rule's latest evaluation began
        // waits for the next one; see the port.
        let held_clause = if held_after.is_some() {
            " AND NOT (state = {} AND updated_at > {})"
        } else {
            ""
        };
        let sql = format!(
            "SELECT {CANDIDATE_COLUMNS}
               FROM lifecycle_candidates
              WHERE rule_set_id = {{}} AND due_at <= {{}}{cursor_clause}
                AND (state IN ({{}}, {{}}, {{}}, {{}})
                     OR (state = {{}} AND updated_at < {{}})){held_clause}
              ORDER BY due_at ASC, id ASC
              LIMIT {{}}"
        );
        let mut args = vec![
            SqlArg::Text(rule_set_id.to_string()),
            SqlArg::Timestamp(due_before),
        ];
        if let Some(after) = after {
            args.push(SqlArg::Timestamp(after.due_at));
            args.push(SqlArg::Timestamp(after.due_at));
            args.push(SqlArg::Text(after.id.clone()));
        }
        args.extend([
            SqlArg::Text(
                LifecycleCandidateState::Observing
                    .as_storage_str()
                    .to_string(),
            ),
            SqlArg::Text(
                LifecycleCandidateState::PendingAction
                    .as_storage_str()
                    .to_string(),
            ),
            SqlArg::Text(LifecycleCandidateState::Due.as_storage_str().to_string()),
            SqlArg::Text(
                LifecycleCandidateState::Blocked
                    .as_storage_str()
                    .to_string(),
            ),
            SqlArg::Text(
                LifecycleCandidateState::Executing
                    .as_storage_str()
                    .to_string(),
            ),
            SqlArg::Timestamp(stale_before),
        ]);
        if let Some(held_after) = held_after {
            args.push(SqlArg::Text(
                LifecycleCandidateState::Blocked
                    .as_storage_str()
                    .to_string(),
            ));
            args.push(SqlArg::Timestamp(held_after));
        }
        args.push(SqlArg::I64(limit as i64));
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
                      SET state = {}, state_reason = {}, state_cause_code = NULL, updated_at = {}
                    WHERE id = {}
                      AND (state = {} OR (state = {} AND updated_at < {}))";
        let args = vec![
            SqlArg::Text(
                LifecycleCandidateState::Executing
                    .as_storage_str()
                    .to_string(),
            ),
            SqlArg::Text("execution_leased".to_string()),
            SqlArg::Timestamp(updated_at),
            SqlArg::Text(id.to_string()),
            SqlArg::Text(LifecycleCandidateState::Due.as_storage_str().to_string()),
            SqlArg::Text(
                LifecycleCandidateState::Executing
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
}

// ── Action runs ─────────────────────────────────────────────────────────────

#[async_trait]
impl LifecycleActionRunRepository for LifecycleEvaluationStore {
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
                SET status = {}, hold_reason = {}, error = {}, cause_code = {}, detail = {},
                    finished_at = {}
              WHERE id = {}",
            vec![
                SqlArg::Text(run.status.as_storage_str().to_string()),
                SqlArg::OptText(run.hold_reason.clone()),
                SqlArg::OptText(run.error.clone()),
                SqlArg::OptText(run.cause_code.clone()),
                SqlArg::Text(run.detail.clone()),
                SqlArg::OptTimestamp(run.finished_at),
                SqlArg::Text(run.id.clone()),
            ],
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
impl LifecycleActionStepRepository for LifecycleEvaluationStore {
    async fn get_action_step(
        &self,
        key: &LifecycleActionStepKey,
    ) -> AppResult<Option<LifecycleActionStepRun>> {
        SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &format!(
                "SELECT {ACTION_STEP_COLUMNS} FROM lifecycle_action_steps
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
        step: &LifecycleActionStepRun,
        stale_before: DateTime<Utc>,
        lease_id: &str,
        leased_at: DateTime<Utc>,
    ) -> AppResult<LifecycleActionStepClaim> {
        if step.state != LifecycleActionStepState::Running || step.attempt < 0 {
            return Err(AppError::validation_refused(
                crate::error_reasons::CONFIGURATION_LIFECYCLE_STEP_ATTEMPT_INVALID,
                "a lifecycle action step claim must start a non-negative running attempt",
            ));
        }
        if !action_step_target_identity_is_consistent(step) {
            return Err(AppError::validation_refused(
                crate::error_reasons::CONFIGURATION_LIFECYCLE_STEP_TARGET_INVALID,
                "a lifecycle action step claim has an invalid target identity",
            ));
        }
        let mut claimed = step.clone();
        claimed.lease_id = Some(lease_id.to_string());
        claimed.updated_at = leased_at;
        claimed.finished_at = None;
        claimed.hold_reason = None;
        claimed.error = None;
        claimed.cause_code = None;
        let key = claimed.key.clone();
        let lease_id = lease_id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "claim_lifecycle_action_step", move |tx| {
            let claimed = claimed.clone();
            let key = key.clone();
            let lease_id = lease_id.clone();
            Box::pin(async move {
                let existing = SqlRuntime::fetch_optional(
                    SqlExec::Tx(tx),
                    &format!(
                        "SELECT {ACTION_STEP_COLUMNS} FROM lifecycle_action_steps
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
                        "INSERT INTO lifecycle_action_steps
                         (candidate_id, match_generation, revision_number, step_id, rule_set_id,
                          title_id, subject_kind, subject_id, sequence_content_hash, step_kind,
                          intent_json, before_state_json, target_identity_json, provenance_json,
                          state, attempt, lease_id, lease_expires_at, hold_reason, error,
                          created_at, updated_at, finished_at, cause_code)
                         VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})
                         ON CONFLICT(candidate_id, match_generation, revision_number, step_id) DO NOTHING",
                        &action_step_args(&claimed),
                    )
                    .await?;
                    if inserted == 1 {
                        insert_action_step_attempt(tx, &claimed).await?;
                        return Ok(LifecycleActionStepClaim::Claimed(claimed));
                    }
                    SqlRuntime::fetch_optional(
                        SqlExec::Tx(tx),
                        &format!(
                            "SELECT {ACTION_STEP_COLUMNS} FROM lifecycle_action_steps
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
                            "lifecycle action step insert raced without a durable winner".into(),
                        )
                    })?
                };
                if existing.sequence_content_hash != claimed.sequence_content_hash
                    || existing.step_kind != claimed.step_kind
                    || !action_step_target_identity_is_consistent(&existing)
                {
                    return Err(AppError::validation_refused(crate::error_reasons::CONFIGURATION_LIFECYCLE_SEQUENCE_IDENTITY_MISMATCH, "lifecycle action step does not match its persisted sequence identity"));
                }
                if existing.state.is_completion_success() {
                    return Ok(LifecycleActionStepClaim::Completed(existing));
                }
                if existing.state == LifecycleActionStepState::Running
                    && existing.updated_at >= stale_before
                {
                    return Ok(LifecycleActionStepClaim::Busy);
                }
                if existing.state == LifecycleActionStepState::Running {
                    let affected = SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "UPDATE lifecycle_action_steps
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
                            SqlArg::Text(LifecycleActionStepState::Running.as_storage_str().into()),
                            SqlArg::Timestamp(existing.updated_at),
                        ],
                    )
                    .await?;
                    if affected != 1 {
                        return Ok(LifecycleActionStepClaim::Busy);
                    }
                    let mut reclaimed = existing;
                    reclaimed.lease_id = claimed.lease_id;
                    reclaimed.lease_expires_at = claimed.lease_expires_at;
                    reclaimed.updated_at = claimed.updated_at;
                    return Ok(LifecycleActionStepClaim::Claimed(reclaimed));
                }
                if existing.state == LifecycleActionStepState::Held {
                    let mut resumed = existing.clone();
                    resumed.state = LifecycleActionStepState::Running;
                    resumed.lease_id = claimed.lease_id;
                    resumed.lease_expires_at = claimed.lease_expires_at;
                    resumed.hold_reason = None;
                    resumed.error = None;
                    resumed.cause_code = None;
                    resumed.updated_at = claimed.updated_at;
                    resumed.finished_at = None;
                    let affected = SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "UPDATE lifecycle_action_steps
                            SET state = {}, lease_id = {}, lease_expires_at = {}, hold_reason = NULL,
                                error = NULL, cause_code = NULL, updated_at = {}, finished_at = NULL
                          WHERE candidate_id = {} AND match_generation = {} AND revision_number = {}
                            AND step_id = {} AND state = {} AND updated_at = {}",
                        &[
                            SqlArg::Text(LifecycleActionStepState::Running.as_storage_str().into()),
                            SqlArg::OptText(resumed.lease_id.clone()),
                            SqlArg::OptTimestamp(resumed.lease_expires_at),
                            SqlArg::Timestamp(resumed.updated_at),
                            SqlArg::Text(key.candidate_id.clone()),
                            SqlArg::I64(key.match_generation),
                            SqlArg::I64(key.revision_number),
                            SqlArg::Text(key.step_id.clone()),
                            SqlArg::Text(LifecycleActionStepState::Held.as_storage_str().into()),
                            SqlArg::Timestamp(existing.updated_at),
                        ],
                    )
                    .await?;
                    if affected != 1 {
                        return Ok(LifecycleActionStepClaim::Busy);
                    }
                    insert_action_step_attempt(tx, &resumed).await?;
                    return Ok(LifecycleActionStepClaim::Claimed(resumed));
                }
                if claimed.attempt <= existing.attempt {
                    return Err(AppError::validation_refused(crate::error_reasons::CONFIGURATION_LIFECYCLE_RETRY_ATTEMPT_NOT_ADVANCED, "a retried lifecycle action step must advance its attempt"));
                }
                let mut retried = existing.clone();
                retried.state = LifecycleActionStepState::Running;
                retried.attempt = claimed.attempt;
                retried.lease_id = claimed.lease_id;
                retried.lease_expires_at = claimed.lease_expires_at;
                retried.hold_reason = None;
                retried.error = None;
                retried.cause_code = None;
                retried.updated_at = claimed.updated_at;
                retried.finished_at = None;
                let affected = SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "UPDATE lifecycle_action_steps
                        SET intent_json = {}, before_state_json = {}, target_identity_json = {},
                            provenance_json = {}, state = {}, attempt = {}, lease_id = {},
                            lease_expires_at = {}, hold_reason = NULL, error = NULL, cause_code = NULL,
                            updated_at = {}, finished_at = NULL
                      WHERE candidate_id = {} AND match_generation = {} AND revision_number = {}
                        AND step_id = {} AND state = {} AND updated_at = {}",
                    &action_step_retry_args(&retried, existing.state, existing.updated_at),
                )
                .await?;
                if affected != 1 {
                    return Ok(LifecycleActionStepClaim::Busy);
                }
                insert_action_step_attempt(tx, &retried).await?;
                Ok(LifecycleActionStepClaim::Claimed(retried))
            })
        })
        .await
    }

    async fn checkpoint_action_step(
        &self,
        step: &LifecycleActionStepRun,
        expected_lease_id: &str,
    ) -> AppResult<bool> {
        update_running_action_step(&self.datastore, step, expected_lease_id, false).await
    }

    async fn finish_action_step(
        &self,
        step: &LifecycleActionStepRun,
        expected_lease_id: &str,
    ) -> AppResult<bool> {
        if !step.state.is_attempt_finished() || step.state == LifecycleActionStepState::Running {
            return Err(AppError::validation_refused(
                crate::error_reasons::CONFIGURATION_LIFECYCLE_ATTEMPT_RESULT_REQUIRED,
                "a lifecycle action step finish must carry an attempt result",
            ));
        }
        update_running_action_step(&self.datastore, step, expected_lease_id, true).await
    }

    async fn list_action_steps(
        &self,
        candidate_id: &str,
        match_generation: i64,
        revision_number: i64,
    ) -> AppResult<Vec<LifecycleActionStepRun>> {
        SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &format!(
                "SELECT {ACTION_STEP_COLUMNS} FROM lifecycle_action_steps
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
        candidates: &[LifecycleActionStepCandidateKey],
    ) -> AppResult<Vec<LifecycleActionStepRun>> {
        if candidates.is_empty() {
            return Ok(Vec::new());
        }
        if candidates.len() > 100 {
            return Err(AppError::validation_refused(
                crate::error_reasons::CONFIGURATION_LIFECYCLE_CANDIDATE_PAGE_TOO_LARGE,
                "lifecycle sequence progress accepts at most 100 candidates per page",
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
                "SELECT {ACTION_STEP_COLUMNS} FROM lifecycle_action_steps
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

    async fn list_step_acts_since(
        &self,
        step_kinds: &[&str],
        since: DateTime<Utc>,
        limit: usize,
    ) -> AppResult<Vec<scryer_application::LifecycleStepAct>> {
        if step_kinds.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        // Served by idx_lifecycle_action_step_attempts_started; each
        // attempt reaches its step's kind through the step key.
        let kinds = vec!["{}"; step_kinds.len()].join(", ");
        let sql = format!(
            "SELECT DISTINCT s.rule_set_id, s.title_id, s.step_kind
               FROM lifecycle_action_step_attempts a
               JOIN lifecycle_action_steps s
                 ON s.candidate_id = a.candidate_id
                AND s.match_generation = a.match_generation
                AND s.revision_number = a.revision_number
                AND s.step_id = a.step_id
              WHERE a.started_at >= {{}}
                AND a.state = {{}}
                AND s.step_kind IN ({kinds})
              LIMIT {{}}"
        );
        let mut args = vec![
            SqlArg::Timestamp(since),
            SqlArg::Text(
                LifecycleActionStepState::Succeeded
                    .as_storage_str()
                    .to_string(),
            ),
        ];
        args.extend(
            step_kinds
                .iter()
                .map(|kind| SqlArg::Text((*kind).to_string())),
        );
        args.push(SqlArg::I64(i64::try_from(limit).unwrap_or(i64::MAX)));
        let rows = SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args).await?;
        rows.into_iter()
            .map(|row| {
                Ok(scryer_application::LifecycleStepAct {
                    rule_set_id: row.text("rule_set_id")?,
                    title_id: row.text("title_id")?,
                    step_kind: row.text("step_kind")?,
                })
            })
            .collect()
    }

    async fn list_action_job_receipts(
        &self,
        key: &LifecycleActionStepKey,
    ) -> AppResult<Vec<LifecycleActionJobReceipt>> {
        SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &format!(
                "SELECT {ACTION_JOB_RECEIPT_COLUMNS} FROM lifecycle_action_job_receipts
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
        steps: &[LifecycleActionStepKey],
    ) -> AppResult<Vec<LifecycleActionJobReceipt>> {
        if steps.is_empty() {
            return Ok(Vec::new());
        }
        if steps.len() > 700 {
            return Err(AppError::validation_refused(
                crate::error_reasons::CONFIGURATION_LIFECYCLE_RECEIPT_PAGE_TOO_LARGE,
                "lifecycle sequence receipt projection accepts at most 700 steps",
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
                "SELECT {ACTION_JOB_RECEIPT_COLUMNS} FROM lifecycle_action_job_receipts
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
        receipt: &LifecycleActionJobReceipt,
    ) -> AppResult<LifecycleActionJobReceiptClaim> {
        receipt
            .validate_schema()
            .map_err(|error| AppError::Validation(error.to_string()))?;
        if receipt.state != LifecycleActionJobReceiptState::Dispatching {
            return Err(AppError::validation_refused(
                crate::error_reasons::CONFIGURATION_LIFECYCLE_DISPATCH_STATE_INVALID,
                "a generic lifecycle job dispatch must begin in dispatching state",
            ));
        }
        let receipt = receipt.clone();
        SqlRuntime::run_in_transaction(&self.datastore, "claim_lifecycle_action_job_dispatch", move |tx| {
            let receipt = receipt.clone();
            Box::pin(async move {
                let mut key_args = action_step_key_args(&receipt.key);
                key_args.push(SqlArg::I64(receipt.dispatch_attempt));
                if let Some(existing) = SqlRuntime::fetch_optional(
                    SqlExec::Tx(tx),
                    &format!(
                        "SELECT {ACTION_JOB_RECEIPT_COLUMNS} FROM lifecycle_action_job_receipts
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
                    return Ok(LifecycleActionJobReceiptClaim::Existing(existing));
                }
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "INSERT INTO lifecycle_action_job_receipts
                     (candidate_id, match_generation, revision_number, step_id, dispatch_attempt,
                      schema_version, logical_request_key, request_hash, job_run_id, state,
                      reconciliation_evidence_json, created_at, updated_at)
                     VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
                    &action_job_receipt_args(&receipt),
                )
                .await?;
                Ok(LifecycleActionJobReceiptClaim::Claimed(receipt))
            })
        })
        .await
    }

    async fn transition_action_job_receipt(
        &self,
        transition: &LifecycleActionJobReceiptTransition,
    ) -> AppResult<bool> {
        transition_action_job_receipt(&self.datastore, transition).await
    }
}

#[async_trait]
impl LifecycleSequenceCompletionRepository for LifecycleEvaluationStore {
    async fn get_active_sequence_completion(
        &self,
        rule_set_id: &str,
        revision_number: i64,
        subject_kind: &str,
        subject_id: &str,
    ) -> AppResult<Option<LifecycleSequenceTerminalMembership>> {
        SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &format!(
                "SELECT {SEQUENCE_TERMINAL_MEMBERSHIP_COLUMNS}
                   FROM lifecycle_sequence_terminal_memberships
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

    async fn finish_sequence_terminal_membership_and_candidate(
        &self,
        membership: &LifecycleSequenceTerminalMembership,
        expected_candidate_state: LifecycleCandidateState,
        expected_candidate_updated_at: DateTime<Utc>,
        terminal_candidate_state: LifecycleCandidateState,
        state_reason: &str,
        finished_at: DateTime<Utc>,
    ) -> AppResult<bool> {
        validate_terminal_membership(membership, terminal_candidate_state)?;
        let membership = membership.clone();
        let state_reason = state_reason.to_string();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "finish_lifecycle_sequence_terminal_membership_and_candidate",
            move |tx| {
                let membership = membership.clone();
                let state_reason = state_reason.clone();
                Box::pin(async move {
                    if let Some(existing) = SqlRuntime::fetch_optional(
                        SqlExec::Tx(tx),
                        &format!(
                            "SELECT {SEQUENCE_TERMINAL_MEMBERSHIP_COLUMNS}
                               FROM lifecycle_sequence_terminal_memberships WHERE candidate_id = {{}}"
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
                        "INSERT INTO lifecycle_sequence_terminal_memberships
                         (id, candidate_id, rule_set_id, revision_number, source_content_hash, title_id,
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
                                   FROM lifecycle_sequence_terminal_memberships
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
                            SET state = {}, state_reason = {}, state_cause_code = NULL,
                                updated_at = {}
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
                            "lifecycle candidate lease was lost before its terminal membership could commit".into(),
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
        // Evaluation calls this for every confirmed non-match of a sequence
        // rule, and almost none has a membership to release. Asking first on
        // the read side keeps those subjects off the single writer; a
        // membership that qualifies was created before `released_at`, so it
        // is already visible to the read.
        let releasable = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT 1 AS releasable
               FROM lifecycle_sequence_terminal_memberships
              WHERE rule_set_id = {} AND revision_number = {} AND subject_kind = {}
                AND subject_id = {} AND released_at IS NULL AND created_at <= {}
              LIMIT 1",
            &[
                SqlArg::Text(rule_set_id.to_string()),
                SqlArg::I64(revision_number),
                SqlArg::Text(subject_kind.to_string()),
                SqlArg::Text(subject_id.to_string()),
                SqlArg::Timestamp(released_at),
            ],
        )
        .await?
        .is_some();
        if !releasable {
            return Ok(false);
        }
        Ok(SqlRuntime::execute_write(
            &self.datastore,
            "release_lifecycle_sequence_completion_on_confirmed_non_match",
            "UPDATE lifecycle_sequence_terminal_memberships
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
impl LifecycleExclusionRepository for LifecycleEvaluationStore {
    async fn list_exclusions(
        &self,
        rule_set_id: Option<&str>,
    ) -> AppResult<Vec<LifecycleRuleExclusion>> {
        let (sql, args) = match rule_set_id {
            // What actually stops one rule acting is its own rows plus every
            // global row, so both are returned together.
            Some(rule_set_id) => (
                format!(
                    "SELECT {EXCLUSION_COLUMNS}
                       FROM lifecycle_rule_exclusions
                      WHERE rule_set_id = {{}} OR rule_set_id IS NULL
                      ORDER BY created_at DESC, id ASC"
                ),
                vec![SqlArg::Text(rule_set_id.to_string())],
            ),
            None => (
                format!(
                    "SELECT {EXCLUSION_COLUMNS}
                       FROM lifecycle_rule_exclusions
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

    async fn get_exclusion(&self, id: &str) -> AppResult<Option<LifecycleRuleExclusion>> {
        let sql =
            format!("SELECT {EXCLUSION_COLUMNS} FROM lifecycle_rule_exclusions WHERE id = {{}}");
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

    async fn create_exclusion(&self, exclusion: &LifecycleRuleExclusion) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "create_lifecycle_rule_exclusion",
            "INSERT INTO lifecycle_rule_exclusions
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
            "delete_lifecycle_rule_exclusion",
            "DELETE FROM lifecycle_rule_exclusions WHERE id = {}",
            vec![SqlArg::Text(id.to_string())],
        )
        .await
    }
}

// ── Evaluation runs ─────────────────────────────────────────────────────────

#[async_trait]
impl LifecycleEvaluationRunRepository for LifecycleEvaluationStore {
    async fn start_evaluation_run(&self, run: &LifecycleEvaluationRun) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "start_lifecycle_evaluation_run",
            "INSERT INTO lifecycle_evaluation_runs
                (id, rule_set_id, revision_number, source_content_hash, started_at, status)
             VALUES ({}, {}, {}, {}, {}, {})",
            vec![
                SqlArg::Text(run.id.clone()),
                SqlArg::Text(run.rule_set_id.clone()),
                SqlArg::I64(run.revision_number),
                SqlArg::Text(run.source_content_hash.clone()),
                SqlArg::Timestamp(run.started_at),
                SqlArg::Text(run.status.as_storage_str().to_string()),
            ],
        )
        .await
    }

    async fn finish_evaluation_run(&self, run: &LifecycleEvaluationRun) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "finish_lifecycle_evaluation_run",
            "UPDATE lifecycle_evaluation_runs
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
    ) -> AppResult<Vec<LifecycleEvaluationRun>> {
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
               FROM lifecycle_evaluation_runs{where_clause}
              ORDER BY started_at DESC, id ASC{limit_clause}"
        );
        SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args)
            .await?
            .iter()
            .map(row_to_evaluation_run)
            .collect()
    }

    async fn rule_schedule_anchors(
        &self,
        rule_set_ids: &[String],
    ) -> AppResult<std::collections::HashMap<String, LifecycleRuleScheduleAnchor>> {
        let mut anchors = std::collections::HashMap::<String, LifecycleRuleScheduleAnchor>::new();
        for chunk in rule_set_ids.chunks(CANDIDATE_ID_LIST_CHUNK) {
            let args: Vec<SqlArg> = chunk.iter().cloned().map(SqlArg::Text).collect();
            let placeholders = id_placeholders(chunk.len());
            let runs_sql = format!(
                "SELECT rule_set_id, MAX(started_at) AS started_at
                   FROM lifecycle_evaluation_runs
                  WHERE rule_set_id IN ({placeholders})
                  GROUP BY rule_set_id"
            );
            for row in SqlRuntime::fetch_all(self.datastore.read_exec(), &runs_sql, &args).await? {
                anchors
                    .entry(row.text("rule_set_id")?)
                    .or_default()
                    .last_run_at = row.opt_timestamp("started_at")?;
            }
            let executing_sql = format!(
                "SELECT DISTINCT rule_set_id
                   FROM lifecycle_candidates
                  WHERE rule_set_id IN ({placeholders}) AND state = 'executing'"
            );
            for row in
                SqlRuntime::fetch_all(self.datastore.read_exec(), &executing_sql, &args).await?
            {
                anchors
                    .entry(row.text("rule_set_id")?)
                    .or_default()
                    .in_flight = true;
            }
        }
        Ok(anchors)
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
        SqlArg::Text(candidate.source_content_hash.clone()),
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
        SqlArg::OptText(candidate.decided_actions_json.clone()),
        SqlArg::OptText(actions_conflict_text(candidate.actions_conflict.as_ref())?),
        SqlArg::OptText(candidate.state_cause_code.clone()),
    ])
}

fn actions_conflict_text(
    conflict: Option<&scryer_domain::LifecycleActionsConflict>,
) -> AppResult<Option<String>> {
    conflict.map(canonical_json_text).transpose()
}

/// A stored conflict this build cannot read still holds the candidate: it
/// reads back as a conflict naming nothing rather than as none.
fn actions_conflict_from_text(text: &str) -> scryer_domain::LifecycleActionsConflict {
    serde_json::from_str(text).unwrap_or_else(|_| scryer_domain::LifecycleActionsConflict {
        head: String::new(),
        other_subject_kind: String::new(),
        other_subject_id: String::new(),
        other_head: String::new(),
    })
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
        SqlArg::OptText(run.cause_code.clone()),
    ]
}

fn action_step_key_args(key: &LifecycleActionStepKey) -> Vec<SqlArg> {
    vec![
        SqlArg::Text(key.candidate_id.clone()),
        SqlArg::I64(key.match_generation),
        SqlArg::I64(key.revision_number),
        SqlArg::Text(key.step_id.clone()),
    ]
}

fn action_step_args(step: &LifecycleActionStepRun) -> Vec<SqlArg> {
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
        SqlArg::OptText(step.cause_code.clone()),
    ]
}

fn action_step_retry_args(
    step: &LifecycleActionStepRun,
    expected_state: LifecycleActionStepState,
    expected_updated_at: DateTime<Utc>,
) -> Vec<SqlArg> {
    let mut args = vec![
        SqlArg::Text(step.intent_json.clone()),
        SqlArg::Text(step.before_state_json.clone()),
        SqlArg::Text(step.target_identity_json.clone()),
        SqlArg::Text(step.provenance_json.clone()),
        SqlArg::Text(
            LifecycleActionStepState::Running
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

fn action_step_target_identity_is_consistent(step: &LifecycleActionStepRun) -> bool {
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
    step: &LifecycleActionStepRun,
) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "INSERT INTO lifecycle_action_step_attempts
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
                LifecycleActionStepState::Running
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
    step: &LifecycleActionStepRun,
    expected_lease_id: &str,
    finish: bool,
) -> AppResult<bool> {
    let step = step.clone();
    let expected_lease_id = expected_lease_id.to_string();
    SqlRuntime::run_in_transaction(
        datastore,
        "update_running_lifecycle_action_step",
        move |tx| {
            let step = step.clone();
            let expected_lease_id = expected_lease_id.clone();
            Box::pin(async move {
                let (sql, args) = if finish {
                    (
                        "UPDATE lifecycle_action_steps
                        SET before_state_json = {}, target_identity_json = {}, provenance_json = {},
                            state = {}, lease_id = NULL, lease_expires_at = NULL, hold_reason = {},
                            error = {}, cause_code = {}, updated_at = {}, finished_at = {}
                      WHERE candidate_id = {} AND match_generation = {} AND revision_number = {}
                        AND step_id = {} AND state = {} AND lease_id = {}",
                        vec![
                            SqlArg::Text(step.before_state_json.clone()),
                            SqlArg::Text(step.target_identity_json.clone()),
                            SqlArg::Text(step.provenance_json.clone()),
                            SqlArg::Text(step.state.as_storage_str().to_string()),
                            SqlArg::OptText(step.hold_reason.clone()),
                            SqlArg::OptText(step.error.clone()),
                            SqlArg::OptText(step.cause_code.clone()),
                            SqlArg::Timestamp(step.updated_at),
                            SqlArg::OptTimestamp(step.finished_at),
                            SqlArg::Text(step.key.candidate_id.clone()),
                            SqlArg::I64(step.key.match_generation),
                            SqlArg::I64(step.key.revision_number),
                            SqlArg::Text(step.key.step_id.clone()),
                            SqlArg::Text(
                                LifecycleActionStepState::Running
                                    .as_storage_str()
                                    .to_string(),
                            ),
                            SqlArg::Text(expected_lease_id),
                        ],
                    )
                } else {
                    (
                        "UPDATE lifecycle_action_steps
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
                                LifecycleActionStepState::Running
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
    step: &LifecycleActionStepRun,
    finish: bool,
) -> AppResult<()> {
    let (sql, args) = if finish {
        (
            "UPDATE lifecycle_action_step_attempts
                SET before_state_json = {}, target_identity_json = {}, provenance_json = {}, state = {},
                    hold_reason = {}, error = {}, cause_code = {}, finished_at = {}, updated_at = {}
              WHERE id = (
                  SELECT id FROM lifecycle_action_step_attempts
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
                SqlArg::OptText(step.cause_code.clone()),
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
            "UPDATE lifecycle_action_step_attempts
                SET before_state_json = {}, target_identity_json = {}, provenance_json = {}, updated_at = {}
              WHERE id = (
                  SELECT id FROM lifecycle_action_step_attempts
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
            "lifecycle action step lost its current immutable attempt".into(),
        ));
    }
    Ok(())
}

fn action_job_receipt_args(receipt: &LifecycleActionJobReceipt) -> Vec<SqlArg> {
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
    transition: &LifecycleActionJobReceiptTransition,
) -> AppResult<bool> {
    if transition.expected_states.is_empty() {
        return Err(AppError::validation_refused(
            crate::error_reasons::CONFIGURATION_LIFECYCLE_RECEIPT_EXPECTED_STATES_REQUIRED,
            "a lifecycle job receipt transition must name expected states",
        ));
    }
    if matches!(
        transition.next_state,
        LifecycleActionJobReceiptState::Accepted | LifecycleActionJobReceiptState::Completed
    ) && transition
        .job_run_id
        .as_deref()
        .is_none_or(|job_run_id| job_run_id.trim().is_empty())
    {
        return Err(AppError::validation_refused(
            crate::error_reasons::CONFIGURATION_LIFECYCLE_JOB_RUN_ID_REQUIRED,
            "accepted lifecycle job receipts require a real job run id",
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
        "UPDATE lifecycle_action_job_receipts
            SET state = {{}}, job_run_id = {{}}, reconciliation_evidence_json = {{}}, updated_at = {{}}
          WHERE candidate_id = {{}} AND match_generation = {{}} AND revision_number = {{}}
            AND step_id = {{}} AND dispatch_attempt = {{}} AND state IN ({states})"
    );
    Ok(SqlRuntime::execute_write(
        datastore,
        "transition_lifecycle_action_job_receipt",
        &sql,
        args,
    )
    .await?
        == 1)
}

fn validate_terminal_membership(
    membership: &LifecycleSequenceTerminalMembership,
    terminal_candidate_state: LifecycleCandidateState,
) -> AppResult<()> {
    let expected_state = match membership.outcome {
        LifecycleSequenceTerminalOutcome::Succeeded => LifecycleCandidateState::Succeeded,
        LifecycleSequenceTerminalOutcome::Failed => LifecycleCandidateState::Failed,
    };
    if terminal_candidate_state != expected_state {
        return Err(AppError::validation_refused(
            crate::error_reasons::CONFIGURATION_LIFECYCLE_TERMINAL_STATE_MISMATCH,
            "lifecycle sequence terminal marker and candidate terminal state disagree",
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
        return Err(AppError::validation_refused(
            crate::error_reasons::CONFIGURATION_LIFECYCLE_TERMINAL_STEP_IDS_INVALID,
            "lifecycle sequence terminal marker has invalid expected step ids",
        ));
    }
    Ok(())
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

async fn verify_terminal_step_evidence(
    tx: &mut SqlTx<'_>,
    membership: &LifecycleSequenceTerminalMembership,
) -> AppResult<()> {
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
                return Err(AppError::validation_refused(
                    crate::error_reasons::CONFIGURATION_LIFECYCLE_SUCCESS_EVIDENCE_INCOMPLETE,
                    "successful lifecycle sequence marker lacks complete step evidence",
                ));
            }
            membership.expected_step_ids.as_slice()
        }
        LifecycleSequenceTerminalOutcome::Failed => {
            let Some(index) = terminal_index else {
                return Err(AppError::validation_refused(
                    crate::error_reasons::CONFIGURATION_LIFECYCLE_FAILED_STEP_ID_REQUIRED,
                    "failed lifecycle sequence marker lacks its failed step id",
                ));
            };
            let expected = &membership.expected_step_ids[..=index];
            if membership.completed_step_ids != expected {
                return Err(AppError::validation_refused(
                    crate::error_reasons::CONFIGURATION_LIFECYCLE_STEP_PREFIX_MISMATCH,
                    "failed lifecycle sequence marker does not preserve its step prefix",
                ));
            }
            expected
        }
    };
    for (index, step_id) in expected_completed.iter().enumerate() {
        let row = SqlRuntime::fetch_optional(
            SqlExec::Tx(tx),
            "SELECT state FROM lifecycle_action_steps
              WHERE candidate_id = {} AND match_generation = {} AND revision_number = {} AND step_id = {}",
            &[
                SqlArg::Text(membership.candidate_id.clone()),
                SqlArg::I64(membership.match_generation),
                SqlArg::I64(membership.revision_number),
                SqlArg::Text(step_id.clone()),
            ],
        )
        .await?
        .ok_or_else(|| AppError::Repository("lifecycle terminal marker references a missing step".into()))?;
        let state =
            LifecycleActionStepState::parse_storage(&row.text("state")?).ok_or_else(|| {
                AppError::Repository(
                    "lifecycle terminal marker references an unknown step state".into(),
                )
            })?;
        let is_failed_terminal = membership.outcome == LifecycleSequenceTerminalOutcome::Failed
            && index + 1 == expected_completed.len();
        if (is_failed_terminal && state != LifecycleActionStepState::Failed)
            || (!is_failed_terminal && !state.is_completion_success())
        {
            return Err(AppError::validation_refused(
                crate::error_reasons::CONFIGURATION_LIFECYCLE_TERMINAL_EVIDENCE_MISMATCH,
                "lifecycle terminal marker does not match persisted step evidence",
            ));
        }
    }
    Ok(())
}

fn terminal_membership_args(
    membership: &LifecycleSequenceTerminalMembership,
) -> AppResult<Vec<SqlArg>> {
    Ok(vec![
        SqlArg::Text(membership.id.clone()),
        SqlArg::Text(membership.candidate_id.clone()),
        SqlArg::Text(membership.rule_set_id.clone()),
        SqlArg::I64(membership.revision_number),
        SqlArg::Text(membership.source_content_hash.clone()),
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

fn row_to_action_step(row: &SqlRow) -> AppResult<LifecycleActionStepRun> {
    let state_raw = row.text("state")?;
    Ok(LifecycleActionStepRun {
        key: LifecycleActionStepKey {
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
        state: LifecycleActionStepState::parse_storage(&state_raw).ok_or_else(|| {
            AppError::Repository(format!("unknown lifecycle action step state: {state_raw}"))
        })?,
        attempt: row.i64("attempt")?,
        lease_id: row.opt_text("lease_id")?,
        lease_expires_at: row.opt_timestamp("lease_expires_at")?,
        hold_reason: row.opt_text("hold_reason")?,
        error: row.opt_text("error")?,
        cause_code: row.opt_text("cause_code")?,
        created_at: row.timestamp("created_at")?,
        updated_at: row.timestamp("updated_at")?,
        finished_at: row.opt_timestamp("finished_at")?,
    })
}

fn row_to_action_job_receipt(row: &SqlRow) -> AppResult<LifecycleActionJobReceipt> {
    let state_raw = row.text("state")?;
    let receipt = LifecycleActionJobReceipt {
        schema_version: row.i32("schema_version")? as u32,
        key: LifecycleActionStepKey {
            candidate_id: row.text("candidate_id")?,
            match_generation: row.i64("match_generation")?,
            revision_number: row.i64("revision_number")?,
            step_id: row.text("step_id")?,
        },
        dispatch_attempt: row.i64("dispatch_attempt")?,
        logical_request_key: row.text("logical_request_key")?,
        request_hash: row.text("request_hash")?,
        job_run_id: row.opt_text("job_run_id")?,
        state: LifecycleActionJobReceiptState::parse_storage(&state_raw).ok_or_else(|| {
            AppError::Repository(format!(
                "unknown lifecycle action job receipt state: {state_raw}"
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
) -> AppResult<LifecycleSequenceTerminalMembership> {
    let outcome_raw = row.text("outcome")?;
    let expected_raw = row.text("expected_step_ids_json")?;
    let completed_raw = row.text("completed_step_ids_json")?;
    Ok(LifecycleSequenceTerminalMembership {
        id: row.text("id")?,
        candidate_id: row.text("candidate_id")?,
        rule_set_id: row.text("rule_set_id")?,
        revision_number: row.i64("revision_number")?,
        source_content_hash: row.text("source_content_hash")?,
        title_id: row.text("title_id")?,
        subject_kind: row.text("subject_kind")?,
        subject_id: row.text("subject_id")?,
        match_generation: row.i64("match_generation")?,
        sequence_content_hash: row.text("sequence_content_hash")?,
        outcome: LifecycleSequenceTerminalOutcome::parse_storage(&outcome_raw).ok_or_else(
            || {
                AppError::Repository(format!(
                    "unknown lifecycle sequence terminal outcome: {outcome_raw}"
                ))
            },
        )?,
        expected_step_ids: serde_json::from_str(&expected_raw).map_err(|error| {
            AppError::Repository(format!("invalid lifecycle expected step ids: {error}"))
        })?,
        completed_step_ids: serde_json::from_str(&completed_raw).map_err(|error| {
            AppError::Repository(format!("invalid lifecycle completed step ids: {error}"))
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
        cause_code: row.opt_text("cause_code")?,
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
        source_content_hash: row.text("source_content_hash")?,
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
        state: LifecycleCandidateState::parse_storage(&row.text("state")?).unwrap_or_default(),
        state_reason: row.text("state_reason")?,
        state_cause_code: row.opt_text("state_cause_code")?,
        reason_codes: reason_codes(row)?,
        action_kind: row.text("action_kind")?,
        decided_actions_json: row.opt_text("decided_actions_json")?,
        actions_conflict: row
            .opt_text("actions_conflict_json")?
            .map(|text| actions_conflict_from_text(&text)),
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

fn row_to_exclusion(row: &SqlRow) -> AppResult<LifecycleRuleExclusion> {
    Ok(LifecycleRuleExclusion {
        id: row.text("id")?,
        rule_set_id: row.opt_text("rule_set_id")?,
        title_id: row.text("title_id")?,
        subject_kind: scryer_domain::LifecycleRuleSubjectKind::parse_storage(
            &row.text("subject_kind")?,
        )
        .ok_or_else(|| AppError::Repository("invalid lifecycle exclusion scope".into()))?,
        subject_id: row.text("subject_id")?,
        reason: row.text("reason")?,
        created_by: row.opt_text("created_by")?,
        created_at: row.timestamp("created_at")?,
    })
}

fn row_to_evaluation_run(row: &SqlRow) -> AppResult<LifecycleEvaluationRun> {
    Ok(LifecycleEvaluationRun {
        id: row.text("id")?,
        rule_set_id: row.text("rule_set_id")?,
        revision_number: row.i64("revision_number")?,
        source_content_hash: row.text("source_content_hash")?,
        started_at: row.timestamp("started_at")?,
        finished_at: row.opt_timestamp("finished_at")?,
        status: LifecycleEvaluationRunStatus::parse_storage(&row.text("status")?)
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

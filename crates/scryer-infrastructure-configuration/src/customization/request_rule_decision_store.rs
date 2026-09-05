use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scryer_application::{AppResult, RequestRuleDecisionRepository};
use scryer_domain::{RequestDecisionOutcome, RequestRuleDecisionRecord, RequestRuleEvaluationMode};
use sqlx::Row;

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, StoreDatastore, repo_err};
use crate::storage::sql::json::{canonical_json_text, json_text_or};

/// Append-only traces of request evaluations (spec 0003 FR-016).
///
/// There is no update and no delete: a decision explains what the instance did
/// at one instant, and rewriting it would make the explanation a fiction. Rule
/// deletion does not cascade here either — the trace outlives the rule.
#[derive(Clone)]
pub struct RequestRuleDecisionStore {
    datastore: StoreDatastore,
}

impl RequestRuleDecisionStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

#[async_trait]
impl RequestRuleDecisionRepository for RequestRuleDecisionStore {
    async fn record(&self, decision: &RequestRuleDecisionRecord) -> AppResult<()> {
        let args = decision_args(decision)?;
        SqlRuntime::run_in_transaction(&self.datastore, "record_request_rule_decision", move |tx| {
            let args = args.clone();
            Box::pin(async move {
                SqlRuntime::execute(SqlExec::Tx(tx), INSERT_DECISION_SQL, &args).await?;
                Ok(())
            })
        })
        .await
    }

    async fn latest_for_request(
        &self,
        request_id: &str,
    ) -> AppResult<Option<RequestRuleDecisionRecord>> {
        // `id` breaks the tie so two decisions recorded inside the same second
        // — which SQLite's second-resolution timestamps make reachable — still
        // read back in a stable order.
        let sql = format!(
            "SELECT {DECISION_COLUMNS}
               FROM request_rule_decisions
              WHERE request_id = {{}}
              ORDER BY evaluated_at DESC, id DESC
              LIMIT 1"
        );
        SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(request_id.to_string())],
        )
        .await?
        .as_ref()
        .map(row_to_decision)
        .transpose()
    }

    async fn list_recent(
        &self,
        limit: usize,
        outcome: Option<RequestDecisionOutcome>,
    ) -> AppResult<Vec<RequestRuleDecisionRecord>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let mut args = Vec::new();
        let filter = if let Some(outcome) = outcome {
            args.push(SqlArg::Text(outcome.as_storage_str().to_string()));
            " WHERE effective_outcome = {}"
        } else {
            ""
        };
        args.push(SqlArg::I64(limit.min(i64::MAX as usize) as i64));
        let sql = format!(
            "SELECT {DECISION_COLUMNS}
               FROM request_rule_decisions{filter}
              ORDER BY evaluated_at DESC, id DESC
              LIMIT {{}}"
        );
        SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args)
            .await?
            .iter()
            .map(row_to_decision)
            .collect()
    }

    /// The rule ids live inside `votes_json` rather than in a join table, so
    /// this is a substring match over that column rather than a join. It is
    /// deliberately loose: the number drives a "this rule decided N requests"
    /// affordance in the authoring UI and nothing safety-bearing, and rule ids
    /// are generated opaque ids, so a false positive would require one id to
    /// contain another. `%` and `_` in the id would widen the pattern; ids are
    /// UUIDs, which contain neither.
    async fn count_for_rule_set(&self, rule_set_id: &str) -> AppResult<u64> {
        let rule_set_id = rule_set_id.trim();
        if rule_set_id.is_empty() {
            return Ok(0);
        }
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT COUNT(*) AS decision_count
               FROM request_rule_decisions
              WHERE votes_json LIKE {}",
            &[SqlArg::Text(format!("%{rule_set_id}%"))],
        )
        .await?;
        let Some(row) = row else {
            return Ok(0);
        };
        Ok(row.i64("decision_count")?.max(0) as u64)
    }
}

const DECISION_COLUMNS: &str = "id, request_id, evaluated_at, mode, effective_outcome,
    policy_outcome, fallback_reason, votes_json, tags_json, input_hash,
    input_schema_version, created_at";

const INSERT_DECISION_SQL: &str = "INSERT INTO request_rule_decisions
        (id, request_id, evaluated_at, mode, effective_outcome, policy_outcome,
         fallback_reason, votes_json, tags_json, input_hash, input_schema_version, created_at)
     VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})";

fn decision_args(decision: &RequestRuleDecisionRecord) -> AppResult<Vec<SqlArg>> {
    Ok(vec![
        SqlArg::Text(decision.id.clone()),
        SqlArg::Text(decision.request_id.clone()),
        SqlArg::Timestamp(decision.evaluated_at),
        SqlArg::Text(decision.mode.as_storage_str().to_string()),
        SqlArg::Text(decision.effective_outcome.as_storage_str().to_string()),
        SqlArg::Text(decision.policy_outcome.as_storage_str().to_string()),
        SqlArg::OptText(decision.fallback_reason.clone()),
        SqlArg::Text(decision.votes_json.clone()),
        SqlArg::Text(canonical_json_text(&decision.tags)?),
        SqlArg::Text(decision.input_hash.clone()),
        SqlArg::I64(decision.input_schema_version),
        SqlArg::Timestamp(decision.created_at),
    ])
}

fn row_to_decision(row: &SqlRow) -> AppResult<RequestRuleDecisionRecord> {
    Ok(RequestRuleDecisionRecord {
        id: row.text("id")?,
        request_id: row.text("request_id")?,
        evaluated_at: timestamp_or_now(row, "evaluated_at")?,
        // An unrecognized mode or outcome was written by a newer build. Both
        // read back at their safe default — disabled, manual review — so this
        // build never presents a verdict it cannot interpret as an approval.
        mode: RequestRuleEvaluationMode::parse_storage(&row.text("mode")?).unwrap_or_default(),
        effective_outcome: RequestDecisionOutcome::parse_storage(&row.text("effective_outcome")?)
            .unwrap_or_default(),
        policy_outcome: RequestDecisionOutcome::parse_storage(&row.text("policy_outcome")?)
            .unwrap_or_default(),
        fallback_reason: row.opt_text("fallback_reason")?,
        votes_json: json_text_or(row, "votes_json", "[]")?,
        tags: serde_json::from_str(&json_text_or(row, "tags_json", "[]")?).unwrap_or_default(),
        input_hash: row.text("input_hash")?,
        input_schema_version: row.i64("input_schema_version")?,
        created_at: timestamp_or_now(row, "created_at")?,
    })
}

fn timestamp_or_now(row: &SqlRow, column: &str) -> AppResult<DateTime<Utc>> {
    match row {
        SqlRow::Sqlite(row) => {
            let raw: String = row.try_get(column).map_err(repo_err)?;
            Ok(DateTime::parse_from_rfc3339(&raw)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()))
        }
        SqlRow::Postgres(_) => row.timestamp(column),
    }
}

//! Title leases and keep claims (spec 0003 FR-041..FR-044).
//!
//! Every state change here is a conditional UPDATE rather than a
//! read-modify-write: two maintenance passes and the import hook can all touch
//! the same claim, and the predicate is what keeps a replayed import from
//! restarting a window or a release from resurrecting a lapsed hold.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scryer_application::{AppError, AppResult, LifecycleClaimRepository};
use scryer_domain::{
    LIFECYCLE_CLAIM_LIVE_STATES, LifecycleClaim, LifecycleClaimKind, LifecycleClaimProducer,
    LifecycleClaimState,
};
use sqlx::Row;
use std::collections::HashMap;

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRuntime, StoreDatastore};
use crate::queries::sql_runtime::{SqlRow, repo_err};

#[derive(Clone)]
pub struct LifecycleClaimStore {
    datastore: StoreDatastore,
}

impl LifecycleClaimStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

#[async_trait]
impl LifecycleClaimRepository for LifecycleClaimStore {
    async fn create(&self, claim: &LifecycleClaim) -> AppResult<()> {
        let args = claim_args(claim);
        SqlRuntime::run_in_transaction(&self.datastore, "create_lifecycle_claim", move |tx| {
            let args = args.clone();
            Box::pin(async move {
                SqlRuntime::execute(SqlExec::Tx(tx), INSERT_CLAIM_SQL, &args).await?;
                Ok(())
            })
        })
        .await
    }

    async fn get(&self, id: &str) -> AppResult<Option<LifecycleClaim>> {
        let sql = format!("SELECT {CLAIM_COLUMNS} FROM lifecycle_claims WHERE id = {{}}");
        SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(id.to_string())],
        )
        .await?
        .as_ref()
        .map(row_to_claim)
        .transpose()
    }

    async fn list_for_title(&self, title_id: &str) -> AppResult<Vec<LifecycleClaim>> {
        let sql = format!(
            "SELECT {CLAIM_COLUMNS}
               FROM lifecycle_claims
              WHERE title_id = {{}}
              ORDER BY created_at DESC, id DESC"
        );
        SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(title_id.to_string())],
        )
        .await?
        .iter()
        .map(row_to_claim)
        .collect()
    }

    async fn list_live_for_titles(
        &self,
        title_ids: &[String],
    ) -> AppResult<HashMap<String, Vec<LifecycleClaim>>> {
        let mut by_title: HashMap<String, Vec<LifecycleClaim>> = HashMap::new();
        if title_ids.is_empty() {
            return Ok(by_title);
        }
        let title_placeholders = std::iter::repeat_n("{}", title_ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        let state_placeholders = std::iter::repeat_n("{}", LIFECYCLE_CLAIM_LIVE_STATES.len())
            .collect::<Vec<_>>()
            .join(", ");
        let mut args: Vec<SqlArg> = title_ids.iter().cloned().map(SqlArg::Text).collect();
        args.extend(live_state_args());
        let sql = format!(
            "SELECT {CLAIM_COLUMNS}
               FROM lifecycle_claims
              WHERE title_id IN ({title_placeholders})
                AND state IN ({state_placeholders})
              ORDER BY created_at ASC, id ASC"
        );
        for row in SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args).await? {
            let claim = row_to_claim(&row)?;
            by_title
                .entry(claim.title_id.clone())
                .or_default()
                .push(claim);
        }
        Ok(by_title)
    }

    /// Live and expired retention claims, so the maintenance facts can tell a
    /// lease that lapsed apart from a title that never had one. Released and
    /// converted rows stay out: they were withdrawn, not spent.
    async fn list_retention_history_for_titles(
        &self,
        title_ids: &[String],
    ) -> AppResult<HashMap<String, Vec<LifecycleClaim>>> {
        let mut by_title: HashMap<String, Vec<LifecycleClaim>> = HashMap::new();
        if title_ids.is_empty() {
            return Ok(by_title);
        }
        let title_placeholders = std::iter::repeat_n("{}", title_ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        let states = retention_history_state_args();
        let state_placeholders = std::iter::repeat_n("{}", states.len())
            .collect::<Vec<_>>()
            .join(", ");
        let mut args: Vec<SqlArg> = title_ids.iter().cloned().map(SqlArg::Text).collect();
        args.extend(states);
        args.push(SqlArg::Text(
            LifecycleClaimKind::RetainUntil.as_storage_str().to_string(),
        ));
        let sql = format!(
            "SELECT {CLAIM_COLUMNS}
               FROM lifecycle_claims
              WHERE title_id IN ({title_placeholders})
                AND state IN ({state_placeholders})
                AND kind = {{}}
              ORDER BY created_at ASC, id ASC"
        );
        for row in SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args).await? {
            let claim = row_to_claim(&row)?;
            by_title
                .entry(claim.title_id.clone())
                .or_default()
                .push(claim);
        }
        Ok(by_title)
    }

    async fn list_dormant(&self, limit: usize) -> AppResult<Vec<LifecycleClaim>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let sql = format!(
            "SELECT {CLAIM_COLUMNS}
               FROM lifecycle_claims
              WHERE state = {{}}
              ORDER BY created_at ASC, id ASC
              LIMIT {{}}"
        );
        SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &sql,
            &[
                SqlArg::Text(LifecycleClaimState::Dormant.as_storage_str().to_string()),
                SqlArg::I64(limit.min(i64::MAX as usize) as i64),
            ],
        )
        .await?
        .iter()
        .map(row_to_claim)
        .collect()
    }

    /// Conditional on the claim still being dormant. A second import of the
    /// same title must not restart a window the requester already spent.
    async fn activate(
        &self,
        id: &str,
        starts_at: DateTime<Utc>,
        expires_at: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        execute_write(
            &self.datastore,
            "activate_lifecycle_claim",
            "UPDATE lifecycle_claims
                SET state = {}, starts_at = {}, expires_at = {}, updated_at = {}
              WHERE id = {} AND state = {}",
            vec![
                SqlArg::Text(LifecycleClaimState::Active.as_storage_str().to_string()),
                SqlArg::Timestamp(starts_at),
                SqlArg::OptTimestamp(expires_at),
                SqlArg::Timestamp(now),
                SqlArg::Text(id.to_string()),
                SqlArg::Text(LifecycleClaimState::Dormant.as_storage_str().to_string()),
            ],
        )
        .await
        .map(|_| ())
    }

    async fn expire_due(&self, now: DateTime<Utc>) -> AppResult<u64> {
        execute_write(
            &self.datastore,
            "expire_due_lifecycle_claims",
            "UPDATE lifecycle_claims
                SET state = {}, updated_at = {}
              WHERE state = {}
                AND expires_at IS NOT NULL
                AND expires_at <= {}",
            vec![
                SqlArg::Text(LifecycleClaimState::Expired.as_storage_str().to_string()),
                SqlArg::Timestamp(now),
                SqlArg::Text(LifecycleClaimState::Active.as_storage_str().to_string()),
                SqlArg::Timestamp(now),
            ],
        )
        .await
    }

    async fn release_for_producer_ref(
        &self,
        producer: LifecycleClaimProducer,
        producer_ref: &str,
        reason: &str,
        now: DateTime<Utc>,
    ) -> AppResult<u64> {
        let state_placeholders = std::iter::repeat_n("{}", LIFECYCLE_CLAIM_LIVE_STATES.len())
            .collect::<Vec<_>>()
            .join(", ");
        let mut args = vec![
            SqlArg::Text(LifecycleClaimState::Released.as_storage_str().to_string()),
            SqlArg::Text(reason.to_string()),
            SqlArg::Timestamp(now),
            SqlArg::Text(producer.as_storage_str().to_string()),
            SqlArg::Text(producer_ref.to_string()),
        ];
        args.extend(live_state_args());
        execute_write_sql(
            &self.datastore,
            "release_lifecycle_claims_for_producer_ref",
            format!(
                "UPDATE lifecycle_claims
                    SET state = {{}}, released_reason = {{}}, updated_at = {{}}
                  WHERE producer = {{}} AND producer_ref = {{}}
                    AND state IN ({state_placeholders})"
            ),
            args,
        )
        .await
    }

    /// Conditional on the claim still being live, like every other transition
    /// here: releasing a claim that already expired would rewrite history.
    async fn release_claim(&self, id: &str, reason: &str, now: DateTime<Utc>) -> AppResult<u64> {
        let state_placeholders = std::iter::repeat_n("{}", LIFECYCLE_CLAIM_LIVE_STATES.len())
            .collect::<Vec<_>>()
            .join(", ");
        let mut args = vec![
            SqlArg::Text(LifecycleClaimState::Released.as_storage_str().to_string()),
            SqlArg::Text(reason.to_string()),
            SqlArg::Timestamp(now),
            SqlArg::Text(id.to_string()),
        ];
        args.extend(live_state_args());
        execute_write_sql(
            &self.datastore,
            "release_lifecycle_claim",
            format!(
                "UPDATE lifecycle_claims
                    SET state = {{}}, released_reason = {{}}, updated_at = {{}}
                  WHERE id = {{}}
                    AND state IN ({state_placeholders})"
            ),
            args,
        )
        .await
    }

    async fn release_for_title(
        &self,
        title_id: &str,
        reason: &str,
        now: DateTime<Utc>,
    ) -> AppResult<u64> {
        let state_placeholders = std::iter::repeat_n("{}", LIFECYCLE_CLAIM_LIVE_STATES.len())
            .collect::<Vec<_>>()
            .join(", ");
        let mut args = vec![
            SqlArg::Text(LifecycleClaimState::Released.as_storage_str().to_string()),
            SqlArg::Text(reason.to_string()),
            SqlArg::Timestamp(now),
            SqlArg::Text(title_id.to_string()),
        ];
        args.extend(live_state_args());
        execute_write_sql(
            &self.datastore,
            "release_lifecycle_claims_for_title",
            format!(
                "UPDATE lifecycle_claims
                    SET state = {{}}, released_reason = {{}}, updated_at = {{}}
                  WHERE title_id = {{}}
                    AND state IN ({state_placeholders})"
            ),
            args,
        )
        .await
    }

    /// Only a live claim can be extended: pushing an expired lease's date out
    /// would resurrect a hold the operator already watched lapse, which is a
    /// new claim, not an extension.
    async fn extend(
        &self,
        id: &str,
        expires_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        let state_placeholders = std::iter::repeat_n("{}", LIFECYCLE_CLAIM_LIVE_STATES.len())
            .collect::<Vec<_>>()
            .join(", ");
        let mut args = vec![
            SqlArg::Timestamp(expires_at),
            SqlArg::Timestamp(now),
            SqlArg::Text(id.to_string()),
        ];
        args.extend(live_state_args());
        let updated = execute_write_sql(
            &self.datastore,
            "extend_lifecycle_claim",
            format!(
                "UPDATE lifecycle_claims
                    SET expires_at = {{}}, updated_at = {{}}
                  WHERE id = {{}} AND state IN ({state_placeholders})"
            ),
            args,
        )
        .await?;
        if updated == 0 {
            return Err(AppError::Validation(
                "lifecycle claim is no longer live".to_string(),
            ));
        }
        Ok(())
    }

    /// One transaction: there must be no instant in which the title carries
    /// neither the lease being converted nor the keep replacing it.
    async fn convert_to_permanent(
        &self,
        id: &str,
        replacement: &LifecycleClaim,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        let state_placeholders = std::iter::repeat_n("{}", LIFECYCLE_CLAIM_LIVE_STATES.len())
            .collect::<Vec<_>>()
            .join(", ");
        let mut convert_args = vec![
            SqlArg::Text(LifecycleClaimState::Converted.as_storage_str().to_string()),
            SqlArg::Text("converted_to_permanent".to_string()),
            SqlArg::Timestamp(now),
            SqlArg::Text(id.to_string()),
        ];
        convert_args.extend(live_state_args());
        let convert_sql = format!(
            "UPDATE lifecycle_claims
                SET state = {{}}, released_reason = {{}}, updated_at = {{}}
              WHERE id = {{}} AND state IN ({state_placeholders})"
        );
        let insert_args = claim_args(replacement);

        SqlRuntime::run_in_transaction(
            &self.datastore,
            "convert_lifecycle_claim_to_permanent",
            move |tx| {
                let convert_sql = convert_sql.clone();
                let convert_args = convert_args.clone();
                let insert_args = insert_args.clone();
                Box::pin(async move {
                    let updated =
                        SqlRuntime::execute(SqlExec::Tx(tx), &convert_sql, &convert_args).await?;
                    if updated == 0 {
                        return Err(AppError::Validation(
                            "lifecycle claim is no longer live".to_string(),
                        ));
                    }
                    SqlRuntime::execute(SqlExec::Tx(tx), INSERT_CLAIM_SQL, &insert_args).await?;
                    Ok(())
                })
            },
        )
        .await
    }

    async fn count_live_for_user(&self, user_id: &str) -> AppResult<u64> {
        let state_placeholders = std::iter::repeat_n("{}", LIFECYCLE_CLAIM_LIVE_STATES.len())
            .collect::<Vec<_>>()
            .join(", ");
        let mut args = vec![SqlArg::Text(user_id.to_string())];
        args.extend(live_state_args());
        // Joined through the producing request rather than taking a caller
        // supplied id list: the alternative asks the caller to first read every
        // request the user ever made, which is the more expensive of the two by
        // exactly that read.
        let sql = format!(
            "SELECT COUNT(*) AS claim_count
               FROM lifecycle_claims claims
               JOIN media_requests requests ON requests.id = claims.producer_ref
              WHERE requests.created_by_user_id = {{}}
                AND claims.state IN ({state_placeholders})"
        );
        let row = SqlRuntime::fetch_optional(self.datastore.read_exec(), &sql, &args).await?;
        let Some(row) = row else {
            return Ok(0);
        };
        Ok(row.i64("claim_count")?.max(0) as u64)
    }
}

const CLAIM_COLUMNS: &str = "id, title_id, library_id, producer, producer_ref, kind, state,
    duration_days, starts_at, expires_at, created_by, created_at, updated_at, released_reason";

const INSERT_CLAIM_SQL: &str = "INSERT INTO lifecycle_claims
        (id, title_id, library_id, producer, producer_ref, kind, state, duration_days,
         starts_at, expires_at, created_by, created_at, updated_at, released_reason)
     VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})";

fn live_state_args() -> Vec<SqlArg> {
    LIFECYCLE_CLAIM_LIVE_STATES
        .iter()
        .map(|state| SqlArg::Text((*state).to_string()))
        .collect()
}

/// The live states plus `expired`: what a lease's history looks like from the
/// fact builder's side, where "it ran out" and "it was withdrawn" are different
/// answers.
fn retention_history_state_args() -> Vec<SqlArg> {
    let mut args = live_state_args();
    args.push(SqlArg::Text(
        LifecycleClaimState::Expired.as_storage_str().to_string(),
    ));
    args
}

fn claim_args(claim: &LifecycleClaim) -> Vec<SqlArg> {
    vec![
        SqlArg::Text(claim.id.clone()),
        SqlArg::Text(claim.title_id.clone()),
        SqlArg::Text(claim.library_id.clone()),
        SqlArg::Text(claim.producer.as_storage_str().to_string()),
        SqlArg::OptText(claim.producer_ref.clone()),
        SqlArg::Text(claim.kind.as_storage_str().to_string()),
        SqlArg::Text(claim.state.as_storage_str().to_string()),
        SqlArg::OptI64(claim.duration_days),
        SqlArg::OptTimestamp(claim.starts_at),
        SqlArg::OptTimestamp(claim.expires_at),
        SqlArg::OptText(claim.created_by.clone()),
        SqlArg::Timestamp(claim.created_at),
        SqlArg::Timestamp(claim.updated_at),
        SqlArg::OptText(claim.released_reason.clone()),
    ]
}

async fn execute_write(
    datastore: &StoreDatastore,
    op_name: &'static str,
    sql: &'static str,
    args: Vec<SqlArg>,
) -> AppResult<u64> {
    execute_write_sql(datastore, op_name, sql.to_string(), args).await
}

async fn execute_write_sql(
    datastore: &StoreDatastore,
    op_name: &'static str,
    sql: String,
    args: Vec<SqlArg>,
) -> AppResult<u64> {
    SqlRuntime::run_in_transaction(datastore, op_name, move |tx| {
        let sql = sql.clone();
        let args = args.clone();
        Box::pin(async move { SqlRuntime::execute(SqlExec::Tx(tx), &sql, &args).await })
    })
    .await
}

fn row_to_claim(row: &SqlRow) -> AppResult<LifecycleClaim> {
    let producer_raw = row.text("producer")?;
    let producer = LifecycleClaimProducer::parse_storage(&producer_raw).ok_or_else(|| {
        AppError::Repository(format!("unknown lifecycle claim producer {producer_raw}"))
    })?;
    let kind_raw = row.text("kind")?;
    let kind = LifecycleClaimKind::parse_storage(&kind_raw)
        .ok_or_else(|| AppError::Repository(format!("unknown lifecycle claim kind {kind_raw}")))?;
    // A state this build does not know cannot be assumed inert: reading it as
    // released would drop a hold. Refusing the row makes the maintenance pass
    // fail loudly, which is what its unreadable-store hold is for.
    let state_raw = row.text("state")?;
    let state = LifecycleClaimState::parse_storage(&state_raw).ok_or_else(|| {
        AppError::Repository(format!("unknown lifecycle claim state {state_raw}"))
    })?;

    Ok(LifecycleClaim {
        id: row.text("id")?,
        title_id: row.text("title_id")?,
        library_id: row.text("library_id")?,
        producer,
        producer_ref: row.opt_text("producer_ref")?,
        kind,
        state,
        duration_days: row.opt_i64("duration_days")?,
        starts_at: row.opt_timestamp("starts_at")?,
        expires_at: row.opt_timestamp("expires_at")?,
        created_by: row.opt_text("created_by")?,
        created_at: timestamp_or_now(row, "created_at")?,
        updated_at: timestamp_or_now(row, "updated_at")?,
        released_reason: row.opt_text("released_reason")?,
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

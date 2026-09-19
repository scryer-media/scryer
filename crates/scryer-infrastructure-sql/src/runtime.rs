use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use scryer_application::{AppError, AppResult};
use serde_json::Value as JsonValue;
use sqlx::postgres::{PgArguments, PgPool, PgRow};
use sqlx::query::Query;
use sqlx::sqlite::{SqliteArguments, SqlitePool, SqliteRow};
use sqlx::types::Json;
use sqlx::{Postgres, Row, Sqlite, Transaction};
use tokio::sync::Mutex;

use super::common::parse_utc_datetime;
use crate::writer_gate::WriterGateHold;

const SQLITE_BUSY_RETRY_DELAYS: [Duration; 5] = [
    Duration::from_millis(50),
    Duration::from_millis(100),
    Duration::from_millis(250),
    Duration::from_millis(500),
    Duration::from_millis(1000),
];
const SQLITE_BUSY_RETRY_HARD_CAP: Duration = Duration::from_secs(120);

#[derive(Clone)]
pub enum StoreDatastore {
    Sqlite {
        pool: SqlitePool,
        writer_gate: Arc<Mutex<()>>,
    },
    Postgres {
        pool: PgPool,
    },
}

impl StoreDatastore {
    pub fn sqlite(pool: SqlitePool, writer_gate: Arc<Mutex<()>>) -> Self {
        Self::Sqlite { pool, writer_gate }
    }

    /// Read-only executor. On sqlite it bypasses the writer gate and busy
    /// retries, so writes must go through `SqlRuntime::execute_write`,
    /// `run_in_transaction`, or `run_serialized_sqlite` instead.
    pub fn read_exec(&self) -> SqlExec<'_, '_> {
        match self {
            Self::Sqlite { pool, .. } => SqlExec::Target(SqlTarget::Sqlite(pool)),
            Self::Postgres { pool } => SqlExec::Target(SqlTarget::Postgres(pool)),
        }
    }

    /// A point-in-time sample for diagnostics. It takes no connection and
    /// never waits, so sampling cannot add to the pressure it measures.
    pub fn pool_usage(&self) -> PoolUsage {
        match self {
            Self::Sqlite { pool, writer_gate } => PoolUsage {
                max_connections: pool.options().get_max_connections(),
                open_connections: pool.size(),
                idle_connections: pool.num_idle(),
                writer_gate_held: writer_gate.try_lock().is_err(),
            },
            Self::Postgres { pool } => PoolUsage {
                max_connections: pool.options().get_max_connections(),
                open_connections: pool.size(),
                idle_connections: pool.num_idle(),
                writer_gate_held: false,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PoolUsage {
    pub max_connections: u32,
    pub open_connections: u32,
    pub idle_connections: usize,
    /// Sqlite only: a write currently holds the single-writer gate.
    pub writer_gate_held: bool,
}

impl PoolUsage {
    /// Every connection the pool may open is checked out, so the next read
    /// waits for one to come back.
    pub fn is_saturated(&self) -> bool {
        self.open_connections >= self.max_connections && self.idle_connections == 0
    }
}

#[derive(Clone, Copy)]
pub enum SqlTarget<'a> {
    Sqlite(&'a SqlitePool),
    Postgres(&'a PgPool),
}

pub enum SqlExec<'tx, 'db> {
    Target(SqlTarget<'db>),
    Tx(&'tx mut SqlTx<'db>),
}

pub type TxFuture<'a, T> = Pin<Box<dyn Future<Output = AppResult<T>> + Send + 'a>>;

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub enum SqlArg {
    Text(String),
    OptText(Option<String>),
    I32(i32),
    I64(i64),
    OptI32(Option<i32>),
    OptI64(Option<i64>),
    F64(f64),
    OptF64(Option<f64>),
    Bool(bool),
    OptBool(Option<bool>),
    Timestamp(DateTime<Utc>),
    OptTimestamp(Option<DateTime<Utc>>),
    Json(JsonValue),
    OptJson(Option<JsonValue>),
    OptBytes(Option<Vec<u8>>),
}

#[derive(Clone, Copy, Debug)]
enum PlaceholderDialect {
    Sqlite,
    Postgres,
}

pub enum SqlRow {
    Sqlite(SqliteRow),
    Postgres(PgRow),
}

pub enum SqlTx<'db> {
    Sqlite(Transaction<'db, Sqlite>),
    Postgres(Transaction<'db, Postgres>),
}

pub struct SqlRuntime;

impl SqlRuntime {
    /// Holds the sqlite writer gate for the whole op; the gate is not
    /// reentrant, so `op` must not call back into `run_in_transaction`,
    /// `run_serialized_sqlite`, or `execute_write` on the same datastore.
    pub async fn run_serialized_sqlite<T, Op, Fut>(
        datastore: &StoreDatastore,
        op_name: &'static str,
        mut op: Op,
    ) -> AppResult<T>
    where
        T: Send,
        Op: FnMut(SqlitePool) -> Fut + Send,
        Fut: Future<Output = AppResult<T>> + Send,
    {
        let StoreDatastore::Sqlite { pool, writer_gate } = datastore else {
            return Err(AppError::Repository(format!(
                "operation `{op_name}` requires sqlite datastore"
            )));
        };

        let mut hold = WriterGateHold::acquire(writer_gate, op_name).await;
        let result = run_with_sqlite_busy_retries(op_name, || op(pool.clone())).await;
        if result.is_ok() {
            hold.committed();
        }
        result
    }

    pub async fn execute(exec: SqlExec<'_, '_>, template: &str, args: &[SqlArg]) -> AppResult<u64> {
        match exec {
            SqlExec::Target(SqlTarget::Sqlite(pool)) => {
                let sql = render_sql(template, PlaceholderDialect::Sqlite, args.len())?;
                let query = bind_sqlite(sqlx::query(sqlx::AssertSqlSafe(&*sql)), args);
                let result = query.execute(pool).await.map_err(repo_err)?;
                Ok(result.rows_affected())
            }
            SqlExec::Target(SqlTarget::Postgres(pool)) => {
                let sql = render_sql(template, PlaceholderDialect::Postgres, args.len())?;
                let query = bind_postgres(sqlx::query(sqlx::AssertSqlSafe(&*sql)), args);
                let result = query.execute(pool).await.map_err(repo_err)?;
                Ok(result.rows_affected())
            }
            SqlExec::Tx(tx) => tx.execute(template, args).await,
        }
    }

    /// Single-statement write through the canonical transaction machinery:
    /// on sqlite this takes the writer gate and busy retries, unlike
    /// `execute(datastore.read_exec(), ..)` which must stay read-only.
    pub async fn execute_write(
        datastore: &StoreDatastore,
        op_name: &'static str,
        template: &str,
        args: Vec<SqlArg>,
    ) -> AppResult<u64> {
        let template: Arc<str> = Arc::from(template);
        let args: Arc<[SqlArg]> = Arc::from(args);
        Self::run_in_transaction(datastore, op_name, move |tx| {
            let template = Arc::clone(&template);
            let args = Arc::clone(&args);
            Box::pin(async move {
                Self::execute(SqlExec::Tx(tx), template.as_ref(), args.as_ref()).await
            })
        })
        .await
    }

    /// Chunked multi-row `INSERT ... VALUES (..),(..)` over an OPEN transaction.
    ///
    /// Rows are inserted `max(1, BATCH_INSERT_MAX_BINDS / row_width)` at a time
    /// — one statement per chunk — and `rows_affected` is summed across chunks.
    /// The per-statement bind cap keeps any single statement under postgres's
    /// 65535-parameter wire limit (u16) and sqlite's compile-time variable
    /// ceiling (`SQLITE_MAX_VARIABLE_NUMBER`); see [`BATCH_INSERT_MAX_BINDS`].
    ///
    /// Chunk boundaries are invisible to callers ONLY because the caller passes
    /// an already-open `tx`: atomicity comes from that transaction, not from
    /// this primitive. If any chunk fails, the caller's transaction rolls back
    /// every chunk together; there is no per-chunk commit here.
    ///
    /// `suffix` is appended verbatim after the VALUES list (`""` or, e.g.,
    /// `ON CONFLICT DO NOTHING`). An `ON CONFLICT DO UPDATE` suffix is only safe
    /// when no two rows in the SAME statement share the conflict key: postgres
    /// rejects a statement that would affect a row "a second time"
    /// (`ON CONFLICT DO UPDATE command cannot affect row a second time`), so
    /// callers that can emit duplicate conflict keys must dedupe up front or
    /// stay on a per-row loop. `ON CONFLICT DO NOTHING` and suffix-less inserts
    /// are always safe to batch.
    ///
    /// `row_width == 0`, or any row whose bind count differs from `row_width`,
    /// is rejected (naming the offending row index and both counts) BEFORE any
    /// SQL executes, so a validation failure never leaves a partial batch
    /// behind. Empty `rows` is a no-op that never touches `tx`.
    pub async fn execute_batch_insert(
        tx: &mut SqlTx<'_>,
        insert_prefix: &str,
        row_width: usize,
        rows: Vec<Vec<SqlArg>>,
        suffix: &str,
    ) -> AppResult<u64> {
        if row_width == 0 {
            return Err(AppError::Repository(
                "execute_batch_insert requires row_width >= 1, received 0".to_string(),
            ));
        }

        // Validate every row up front: nothing touches `tx` until all rows are
        // known good, so a ragged row can never leave a partial batch behind.
        for (index, row) in rows.iter().enumerate() {
            if row.len() != row_width {
                return Err(AppError::Repository(format!(
                    "execute_batch_insert row {index} has {} bind(s), expected row_width {row_width}",
                    row.len()
                )));
            }
        }

        if rows.is_empty() {
            return Ok(0);
        }

        let rows_per_chunk = batch_rows_per_chunk(row_width);
        let mut total: u64 = 0;
        let mut row_iter = rows.into_iter();
        loop {
            // Move (never clone) each chunk's binds into one flat argument Vec.
            let mut args: Vec<SqlArg> = Vec::with_capacity(rows_per_chunk * row_width);
            let mut chunk_rows = 0usize;
            for row in row_iter.by_ref().take(rows_per_chunk) {
                args.extend(row);
                chunk_rows += 1;
            }
            if chunk_rows == 0 {
                break;
            }
            let sql = build_batch_insert_sql(insert_prefix, row_width, chunk_rows, suffix);
            total = total.saturating_add(tx.execute(&sql, &args).await?);
        }
        Ok(total)
    }

    pub async fn fetch_optional(
        exec: SqlExec<'_, '_>,
        template: &str,
        args: &[SqlArg],
    ) -> AppResult<Option<SqlRow>> {
        match exec {
            SqlExec::Target(SqlTarget::Sqlite(pool)) => {
                let sql = render_sql(template, PlaceholderDialect::Sqlite, args.len())?;
                let query = bind_sqlite(sqlx::query(sqlx::AssertSqlSafe(&*sql)), args);
                query
                    .fetch_optional(pool)
                    .await
                    .map(|row| row.map(SqlRow::Sqlite))
                    .map_err(repo_err)
            }
            SqlExec::Target(SqlTarget::Postgres(pool)) => {
                let sql = render_sql(template, PlaceholderDialect::Postgres, args.len())?;
                let query = bind_postgres(sqlx::query(sqlx::AssertSqlSafe(&*sql)), args);
                query
                    .fetch_optional(pool)
                    .await
                    .map(|row| row.map(SqlRow::Postgres))
                    .map_err(repo_err)
            }
            SqlExec::Tx(tx) => match tx {
                SqlTx::Sqlite(tx) => {
                    let sql = render_sql(template, PlaceholderDialect::Sqlite, args.len())?;
                    let query = bind_sqlite(sqlx::query(sqlx::AssertSqlSafe(&*sql)), args);
                    query
                        .fetch_optional(&mut **tx)
                        .await
                        .map(|row| row.map(SqlRow::Sqlite))
                        .map_err(repo_err)
                }
                SqlTx::Postgres(tx) => {
                    let sql = render_sql(template, PlaceholderDialect::Postgres, args.len())?;
                    let query = bind_postgres(sqlx::query(sqlx::AssertSqlSafe(&*sql)), args);
                    query
                        .fetch_optional(&mut **tx)
                        .await
                        .map(|row| row.map(SqlRow::Postgres))
                        .map_err(repo_err)
                }
            },
        }
    }

    pub async fn fetch_all(
        exec: SqlExec<'_, '_>,
        template: &str,
        args: &[SqlArg],
    ) -> AppResult<Vec<SqlRow>> {
        match exec {
            SqlExec::Target(SqlTarget::Sqlite(pool)) => {
                let sql = render_sql(template, PlaceholderDialect::Sqlite, args.len())?;
                let query = bind_sqlite(sqlx::query(sqlx::AssertSqlSafe(&*sql)), args);
                query
                    .fetch_all(pool)
                    .await
                    .map(|rows| rows.into_iter().map(SqlRow::Sqlite).collect())
                    .map_err(repo_err)
            }
            SqlExec::Target(SqlTarget::Postgres(pool)) => {
                let sql = render_sql(template, PlaceholderDialect::Postgres, args.len())?;
                let query = bind_postgres(sqlx::query(sqlx::AssertSqlSafe(&*sql)), args);
                query
                    .fetch_all(pool)
                    .await
                    .map(|rows| rows.into_iter().map(SqlRow::Postgres).collect())
                    .map_err(repo_err)
            }
            SqlExec::Tx(tx) => match tx {
                SqlTx::Sqlite(tx) => {
                    let sql = render_sql(template, PlaceholderDialect::Sqlite, args.len())?;
                    let query = bind_sqlite(sqlx::query(sqlx::AssertSqlSafe(&*sql)), args);
                    query
                        .fetch_all(&mut **tx)
                        .await
                        .map(|rows| rows.into_iter().map(SqlRow::Sqlite).collect())
                        .map_err(repo_err)
                }
                SqlTx::Postgres(tx) => {
                    let sql = render_sql(template, PlaceholderDialect::Postgres, args.len())?;
                    let query = bind_postgres(sqlx::query(sqlx::AssertSqlSafe(&*sql)), args);
                    query
                        .fetch_all(&mut **tx)
                        .await
                        .map(|rows| rows.into_iter().map(SqlRow::Postgres).collect())
                        .map_err(repo_err)
                }
            },
        }
    }

    /// `op` may run more than once on sqlite busy retries, so it must only
    /// mutate state through the transaction. The writer gate is held for the
    /// whole call and is not reentrant: `op` must not call back into
    /// `run_in_transaction`, `run_serialized_sqlite`, or `execute_write` on
    /// the same datastore.
    pub async fn run_in_transaction<T, F>(
        datastore: &StoreDatastore,
        op_name: &'static str,
        op: F,
    ) -> AppResult<T>
    where
        T: Send,
        F: for<'tx, 'db> Fn(&'tx mut SqlTx<'db>) -> TxFuture<'tx, T> + Send + Sync,
    {
        match datastore {
            StoreDatastore::Sqlite { pool, writer_gate } => {
                let mut hold = WriterGateHold::acquire(writer_gate, op_name).await;
                let outcome = run_with_sqlite_busy_retries(op_name, || {
                    let pool = pool.clone();
                    let op = &op;
                    async move {
                        let mut tx = SqlTx::Sqlite(pool.begin().await.map_err(repo_err)?);
                        let result = {
                            let future = op(&mut tx);
                            future.await?
                        };
                        tx.commit().await?;
                        Ok(result)
                    }
                })
                .await;
                if outcome.is_ok() {
                    hold.committed();
                }
                outcome
            }
            StoreDatastore::Postgres { pool } => {
                let mut tx = SqlTx::Postgres(pool.begin().await.map_err(repo_err)?);
                let result = {
                    let future = op(&mut tx);
                    future.await?
                };
                tx.commit().await?;
                Ok(result)
            }
        }
    }
}

impl<'db> SqlTx<'db> {
    pub async fn execute(&mut self, template: &str, args: &[SqlArg]) -> AppResult<u64> {
        match self {
            SqlTx::Sqlite(tx) => {
                let sql = render_sql(template, PlaceholderDialect::Sqlite, args.len())?;
                let query = bind_sqlite(sqlx::query(sqlx::AssertSqlSafe(&*sql)), args);
                let result = query.execute(&mut **tx).await.map_err(repo_err)?;
                Ok(result.rows_affected())
            }
            SqlTx::Postgres(tx) => {
                let sql = render_sql(template, PlaceholderDialect::Postgres, args.len())?;
                let query = bind_postgres(sqlx::query(sqlx::AssertSqlSafe(&*sql)), args);
                let result = query.execute(&mut **tx).await.map_err(repo_err)?;
                Ok(result.rows_affected())
            }
        }
    }

    pub fn sqlite<'tx>(&'tx mut self) -> Option<&'tx mut Transaction<'db, Sqlite>> {
        match self {
            Self::Sqlite(tx) => Some(tx),
            Self::Postgres(_) => None,
        }
    }

    pub fn postgres<'tx>(&'tx mut self) -> Option<&'tx mut Transaction<'db, Postgres>> {
        match self {
            Self::Sqlite(_) => None,
            Self::Postgres(tx) => Some(tx),
        }
    }

    pub async fn commit(self) -> AppResult<()> {
        match self {
            SqlTx::Sqlite(tx) => tx.commit().await.map_err(repo_err),
            SqlTx::Postgres(tx) => tx.commit().await.map_err(repo_err),
        }
    }
}

#[allow(dead_code)]
impl SqlRow {
    pub fn text(&self, column: &str) -> AppResult<String> {
        match self {
            SqlRow::Sqlite(row) => row.try_get(column).map_err(repo_err),
            SqlRow::Postgres(row) => row.try_get(column).map_err(repo_err),
        }
    }

    pub fn opt_text(&self, column: &str) -> AppResult<Option<String>> {
        match self {
            SqlRow::Sqlite(row) => opt_text_from_sqlite_row(row, column),
            SqlRow::Postgres(row) => opt_text_from_pg_row(row, column),
        }
    }

    pub fn i64(&self, column: &str) -> AppResult<i64> {
        match self {
            SqlRow::Sqlite(row) => row.try_get(column).map_err(repo_err),
            SqlRow::Postgres(row) => i64_from_pg_row(row, column),
        }
    }

    pub fn i32(&self, column: &str) -> AppResult<i32> {
        match self {
            SqlRow::Sqlite(row) => i32_from_sqlite_row(row, column),
            SqlRow::Postgres(row) => i32_from_pg_row(row, column),
        }
    }

    pub fn opt_i64(&self, column: &str) -> AppResult<Option<i64>> {
        match self {
            SqlRow::Sqlite(row) => opt_i64_from_sqlite_row(row, column),
            SqlRow::Postgres(row) => opt_i64_from_pg_row(row, column),
        }
    }

    pub fn opt_i32(&self, column: &str) -> AppResult<Option<i32>> {
        match self {
            SqlRow::Sqlite(row) => opt_i32_from_sqlite_row(row, column),
            SqlRow::Postgres(row) => opt_i32_from_pg_row(row, column),
        }
    }

    pub fn opt_f64(&self, column: &str) -> AppResult<Option<f64>> {
        match self {
            SqlRow::Sqlite(row) => row.try_get(column).map_err(repo_err),
            SqlRow::Postgres(row) => row.try_get(column).map_err(repo_err),
        }
    }

    pub fn bool(&self, column: &str) -> AppResult<bool> {
        match self {
            SqlRow::Sqlite(row) => bool_from_sqlite_row(row, column),
            SqlRow::Postgres(row) => bool_from_pg_row(row, column),
        }
    }

    pub fn opt_bool(&self, column: &str) -> AppResult<Option<bool>> {
        match self {
            SqlRow::Sqlite(row) => opt_bool_from_sqlite_row(row, column),
            SqlRow::Postgres(row) => opt_bool_from_pg_row(row, column),
        }
    }

    pub fn timestamp(&self, column: &str) -> AppResult<DateTime<Utc>> {
        match self {
            SqlRow::Sqlite(row) => {
                let raw: String = row.try_get(column).map_err(repo_err)?;
                parse_utc_datetime(&raw)
            }
            SqlRow::Postgres(row) => row.try_get(column).map_err(repo_err),
        }
    }

    pub fn opt_timestamp(&self, column: &str) -> AppResult<Option<DateTime<Utc>>> {
        match self {
            SqlRow::Sqlite(row) => {
                let raw: Option<String> = row.try_get(column).map_err(repo_err)?;
                match raw {
                    Some(raw) if !raw.trim().is_empty() => parse_utc_datetime(&raw).map(Some),
                    Some(_) | None => Ok(None),
                }
            }
            SqlRow::Postgres(row) => row.try_get(column).map_err(repo_err),
        }
    }

    pub fn opt_json(&self, column: &str) -> AppResult<Option<JsonValue>> {
        match self {
            SqlRow::Sqlite(row) => {
                let raw: Option<String> = row.try_get(column).map_err(repo_err)?;
                match raw {
                    Some(raw) if !raw.trim().is_empty() => {
                        serde_json::from_str(&raw).map(Some).map_err(repo_err)
                    }
                    Some(_) | None => Ok(None),
                }
            }
            SqlRow::Postgres(row) => {
                if let Ok(raw) = row.try_get::<Option<Json<JsonValue>>, _>(column) {
                    return Ok(raw.map(|value| value.0));
                }
                let raw: Option<String> = row.try_get(column).map_err(repo_err)?;
                raw.filter(|value| !value.trim().is_empty())
                    .map(|value| serde_json::from_str(&value).map_err(repo_err))
                    .transpose()
            }
        }
    }

    pub fn opt_bytes(&self, column: &str) -> AppResult<Option<Vec<u8>>> {
        match self {
            SqlRow::Sqlite(row) => row.try_get(column).map_err(repo_err),
            SqlRow::Postgres(row) => row.try_get(column).map_err(repo_err),
        }
    }
}

type SqliteQuery<'q> = Query<'q, Sqlite, SqliteArguments>;
type PostgresQuery<'q> = Query<'q, Postgres, PgArguments>;

fn bind_sqlite<'q>(mut query: SqliteQuery<'q>, values: &'q [SqlArg]) -> SqliteQuery<'q> {
    for value in values {
        query = match value {
            SqlArg::Text(value) => query.bind(value),
            SqlArg::OptText(value) => query.bind(value),
            SqlArg::I32(value) => query.bind(i64::from(*value)),
            SqlArg::I64(value) => query.bind(*value),
            SqlArg::OptI32(value) => query.bind(value.map(i64::from)),
            SqlArg::OptI64(value) => query.bind(*value),
            SqlArg::F64(value) => query.bind(*value),
            SqlArg::OptF64(value) => query.bind(*value),
            SqlArg::Bool(value) => query.bind(if *value { 1_i64 } else { 0_i64 }),
            SqlArg::OptBool(value) => {
                query.bind(value.map(|value| if value { 1_i64 } else { 0_i64 }))
            }
            SqlArg::Timestamp(value) => query.bind(value.to_rfc3339()),
            SqlArg::OptTimestamp(value) => query.bind(value.map(|value| value.to_rfc3339())),
            SqlArg::Json(value) => query.bind(value.to_string()),
            SqlArg::OptJson(value) => query.bind(value.as_ref().map(JsonValue::to_string)),
            SqlArg::OptBytes(value) => query.bind(value),
        };
    }
    query
}

fn bind_postgres<'q>(mut query: PostgresQuery<'q>, values: &'q [SqlArg]) -> PostgresQuery<'q> {
    for value in values {
        query = match value {
            SqlArg::Text(value) => query.bind(value),
            SqlArg::OptText(value) => query.bind(value),
            SqlArg::I32(value) => query.bind(*value),
            SqlArg::I64(value) => query.bind(*value),
            SqlArg::OptI32(value) => query.bind(*value),
            SqlArg::OptI64(value) => query.bind(*value),
            SqlArg::F64(value) => query.bind(*value),
            SqlArg::OptF64(value) => query.bind(*value),
            SqlArg::Bool(value) => query.bind(*value),
            SqlArg::OptBool(value) => query.bind(*value),
            SqlArg::Timestamp(value) => query.bind(*value),
            SqlArg::OptTimestamp(value) => query.bind(*value),
            // Bind JSON by reference (Json<&JsonValue>): sqlx 0.9's
            // `impl<T: Serialize> Encode for Json<T>` covers &JsonValue, so the
            // args slice is serialized in place with no deep clone.
            SqlArg::Json(value) => query.bind(Json(value)),
            SqlArg::OptJson(value) => query.bind(value.as_ref().map(Json)),
            SqlArg::OptBytes(value) => query.bind(value),
        };
    }
    query
}

/// Renders `{}` placeholders into the dialect's positional form (`?` for
/// sqlite, `$1..$n` for postgres). Every `{}` in `template` is a placeholder
/// and there is NO escape: SQL that needs a literal `{}` (e.g. a JSON
/// empty-object default such as `'{}'`) must bind it as an argument instead of
/// inlining it in the template.
fn render_sql(template: &str, dialect: PlaceholderDialect, bind_count: usize) -> AppResult<String> {
    use std::fmt::Write;

    let placeholder_count = template.matches("{}").count();
    if placeholder_count != bind_count {
        return Err(AppError::Repository(format!(
            "sql placeholder mismatch: expected {placeholder_count} bind(s), received {bind_count}"
        )));
    }

    let mut next_index = 1usize;
    let mut rendered = String::with_capacity(template.len() + bind_count * 2);
    let mut parts = template.split("{}").peekable();
    while let Some(part) = parts.next() {
        rendered.push_str(part);
        if parts.peek().is_some() {
            match dialect {
                PlaceholderDialect::Sqlite => rendered.push('?'),
                PlaceholderDialect::Postgres => {
                    rendered.push('$');
                    // write! onto a String is infallible; avoids the
                    // per-placeholder next_index.to_string() allocation.
                    let _ = write!(rendered, "{next_index}");
                    next_index += 1;
                }
            }
        }
    }
    Ok(rendered)
}

/// Per-statement bind cap for [`SqlRuntime::execute_batch_insert`]. Postgres
/// refuses more than 65535 bind parameters in one statement (the u16 protocol
/// wire limit) and sqlite caps variables at `SQLITE_MAX_VARIABLE_NUMBER`
/// (historically 999, 32766 in newer builds); 999 stays at or below both
/// while still collapsing hundreds of single-row inserts into one round trip.
const BATCH_INSERT_MAX_BINDS: usize = 999;

/// Rows per batched statement: as many `row_width`-wide rows as fit under
/// [`BATCH_INSERT_MAX_BINDS`], but never fewer than one — a single row wider
/// than the cap still goes out one row per statement rather than zero. Callers
/// guarantee `row_width >= 1`; the `.max(1)` on the divisor is only a
/// belt-and-braces guard against a future zero slipping past that contract.
fn batch_rows_per_chunk(row_width: usize) -> usize {
    (BATCH_INSERT_MAX_BINDS / row_width.max(1)).max(1)
}

/// Builds the `{insert_prefix} VALUES ({}, ..), ({}, ..) {suffix}` template for
/// one chunk of `chunk_rows` rows, each carrying `row_width` `{}` placeholders.
/// The `{}` tuples are left for [`render_sql`] to lower into `?` / `$n`, so the
/// batch primitive stays dialect-agnostic. An empty (or whitespace-only)
/// `suffix` is dropped entirely so the statement never carries a dangling
/// trailing space. Callers guarantee `row_width >= 1` and `chunk_rows >= 1`.
fn build_batch_insert_sql(
    insert_prefix: &str,
    row_width: usize,
    chunk_rows: usize,
    suffix: &str,
) -> String {
    // One "({}, {}, ...)" placeholder tuple, reused across the chunk's rows.
    let mut tuple = String::with_capacity(row_width * 4 + 2);
    tuple.push('(');
    for index in 0..row_width {
        if index > 0 {
            tuple.push_str(", ");
        }
        tuple.push_str("{}");
    }
    tuple.push(')');

    let mut values =
        String::with_capacity(tuple.len() * chunk_rows + chunk_rows.saturating_sub(1) * 2);
    for row in 0..chunk_rows {
        if row > 0 {
            values.push_str(", ");
        }
        values.push_str(&tuple);
    }

    let suffix = suffix.trim();
    if suffix.is_empty() {
        format!("{insert_prefix} VALUES {values}")
    } else {
        format!("{insert_prefix} VALUES {values} {suffix}")
    }
}

fn opt_text_from_sqlite_row(row: &SqliteRow, column: &str) -> AppResult<Option<String>> {
    match row.try_get::<Option<String>, _>(column) {
        Ok(value) => Ok(value),
        // Deliberate int->text coercion: sqlite's dynamic typing may hand back an integer here.
        Err(string_error) => match row.try_get::<Option<i64>, _>(column) {
            Ok(value) => Ok(value.map(|value| value.to_string())),
            Err(integer_error) => Err(AppError::Repository(format!(
                "failed decode {column} as optional text: {string_error}; {integer_error}"
            ))),
        },
    }
}

fn opt_text_from_pg_row(row: &PgRow, column: &str) -> AppResult<Option<String>> {
    match row.try_get::<Option<String>, _>(column) {
        Ok(value) => Ok(value),
        // Deliberate int->text coercion: tolerates cross-engine schema drift (int vs text columns).
        Err(string_error) => match row.try_get::<Option<i64>, _>(column) {
            Ok(value) => Ok(value.map(|value| value.to_string())),
            Err(integer_error) => Err(AppError::Repository(format!(
                "failed decode {column} as optional text: {string_error}; {integer_error}"
            ))),
        },
    }
}

fn opt_i64_from_sqlite_row(row: &SqliteRow, column: &str) -> AppResult<Option<i64>> {
    row.try_get(column).map_err(repo_err)
}

fn opt_i64_from_pg_row(row: &PgRow, column: &str) -> AppResult<Option<i64>> {
    row.try_get::<Option<i64>, _>(column).or_else(|_| {
        row.try_get::<Option<i32>, _>(column)
            .map(|value| value.map(i64::from))
            .or_else(|_| {
                row.try_get::<Option<i16>, _>(column)
                    .map(|value| value.map(i64::from))
            })
            .map_err(repo_err)
    })
}

fn i64_from_pg_row(row: &PgRow, column: &str) -> AppResult<i64> {
    row.try_get::<i64, _>(column).or_else(|_| {
        row.try_get::<i32, _>(column)
            .map(i64::from)
            .or_else(|_| row.try_get::<i16, _>(column).map(i64::from))
            .map_err(repo_err)
    })
}

fn i32_from_sqlite_row(row: &SqliteRow, column: &str) -> AppResult<i32> {
    let value: i64 = row.try_get(column).map_err(repo_err)?;
    i32_from_i64(column, value)
}

fn i32_from_pg_row(row: &PgRow, column: &str) -> AppResult<i32> {
    // i32 -> i16 -> i64: the widening decode is tried last so a genuinely
    // out-of-range i64 still surfaces the range error from i32_from_i64.
    row.try_get::<i32, _>(column).or_else(|_| {
        row.try_get::<i16, _>(column).map(i32::from).or_else(|_| {
            row.try_get::<i64, _>(column)
                .map_err(repo_err)
                .and_then(|value| i32_from_i64(column, value))
        })
    })
}

fn opt_i32_from_sqlite_row(row: &SqliteRow, column: &str) -> AppResult<Option<i32>> {
    row.try_get::<Option<i64>, _>(column)
        .map_err(repo_err)?
        .map(|value| {
            i32::try_from(value).map_err(|_| {
                AppError::Repository(format!(
                    "value out of range for i32 column {column}: {value}"
                ))
            })
        })
        .transpose()
}

fn opt_i32_from_pg_row(row: &PgRow, column: &str) -> AppResult<Option<i32>> {
    // i32 -> i16 -> i64: the widening decode is tried last so a genuinely
    // out-of-range i64 still surfaces the range error from i32_from_i64.
    row.try_get::<Option<i32>, _>(column).or_else(|_| {
        row.try_get::<Option<i16>, _>(column)
            .map(|value| value.map(i32::from))
            .or_else(|_| {
                row.try_get::<Option<i64>, _>(column)
                    .map_err(repo_err)
                    .and_then(|value| value.map(|value| i32_from_i64(column, value)).transpose())
            })
    })
}

fn i32_from_i64(column: &str, value: i64) -> AppResult<i32> {
    i32::try_from(value).map_err(|_| {
        AppError::Repository(format!(
            "value out of range for i32 column {column}: {value}"
        ))
    })
}

fn bool_from_sqlite_row(row: &SqliteRow, column: &str) -> AppResult<bool> {
    let value: i64 = row.try_get(column).map_err(repo_err)?;
    Ok(value != 0)
}

fn bool_from_pg_row(row: &PgRow, column: &str) -> AppResult<bool> {
    row.try_get(column).map_err(repo_err)
}

fn opt_bool_from_sqlite_row(row: &SqliteRow, column: &str) -> AppResult<Option<bool>> {
    let value: Option<i64> = row.try_get(column).map_err(repo_err)?;
    Ok(value.map(|value| value != 0))
}

fn opt_bool_from_pg_row(row: &PgRow, column: &str) -> AppResult<Option<bool>> {
    row.try_get(column).map_err(repo_err)
}

// SQLITE_BUSY (5) and SQLITE_LOCKED (6) families only; anything else (IOERR,
// CONSTRAINT, ABORT, ...) must fail immediately instead of burning the retry
// deadline while holding the writer gate.
const TRANSIENT_SQLITE_ERROR_CODES: [u64; 6] = [5, 261, 517, 773, 6, 262];

pub fn is_transient_sqlite_busy(error: &AppError) -> bool {
    let AppError::Repository(message) = error else {
        return false;
    };

    let normalized = message.to_ascii_lowercase();
    normalized.contains("database is locked")
        || normalized.contains("database table is locked")
        || normalized.contains("database schema is locked")
        || normalized.contains("sqlite_busy")
        || normalized.contains("busy_snapshot")
        || sqlite_error_codes(&normalized).any(|code| TRANSIENT_SQLITE_ERROR_CODES.contains(&code))
}

// sqlx renders sqlite errors as "(code: N) message"; match N exactly so codes
// that merely start with a transient digit (516, 522, 526, ...) never retry.
fn sqlite_error_codes(message: &str) -> impl Iterator<Item = u64> + '_ {
    const CODE_PREFIX: &str = "code: ";
    message.match_indices(CODE_PREFIX).filter_map(|(index, _)| {
        let digits: String = message[index + CODE_PREFIX.len()..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        digits.parse().ok()
    })
}

pub async fn run_with_sqlite_busy_retries<T, Op, Fut>(
    operation_name: &str,
    mut operation: Op,
) -> AppResult<T>
where
    Op: FnMut() -> Fut,
    Fut: Future<Output = AppResult<T>>,
{
    run_with_sqlite_busy_retries_with_deadline(
        operation_name,
        SQLITE_BUSY_RETRY_HARD_CAP,
        &mut operation,
    )
    .await
}

pub async fn run_with_sqlite_busy_retries_with_deadline<T, Op, Fut>(
    operation_name: &str,
    hard_cap: Duration,
    operation: &mut Op,
) -> AppResult<T>
where
    Op: FnMut() -> Fut,
    Fut: Future<Output = AppResult<T>>,
{
    let started_at = tokio::time::Instant::now();
    let mut attempt = 0usize;

    loop {
        match operation().await {
            Ok(value) => return Ok(value),
            Err(error) if is_transient_sqlite_busy(&error) => {
                let elapsed = started_at.elapsed();
                if elapsed >= hard_cap {
                    tracing::warn!(
                        attempts = attempt,
                        elapsed_ms = elapsed.as_millis(),
                        error = %error,
                        operation = operation_name,
                        "serialized sqlite writer: retry deadline exhausted"
                    );
                    return Err(AppError::Repository(format!(
                        "serialized sqlite writer: retry deadline exceeded for operation `{operation_name}` after {attempt} attempts over {}ms: {error}",
                        elapsed.as_millis()
                    )));
                }

                let scheduled_delay = SQLITE_BUSY_RETRY_DELAYS
                    [attempt.min(SQLITE_BUSY_RETRY_DELAYS.len().saturating_sub(1))];
                let remaining = hard_cap.saturating_sub(elapsed);
                let delay = scheduled_delay.min(remaining);
                tracing::debug!(
                    attempt = attempt + 1,
                    retry_after_ms = delay.as_millis(),
                    elapsed_ms = elapsed.as_millis(),
                    error = %error,
                    operation = operation_name,
                    "serialized sqlite writer: retrying transient sqlite busy"
                );
                attempt = attempt.saturating_add(1);
                tokio::time::sleep(delay).await;
            }
            Err(error) => return Err(error),
        }
    }
}

pub fn repo_err(error: impl std::fmt::Display) -> AppError {
    AppError::Repository(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_error(message: &str) -> AppError {
        AppError::Repository(message.to_string())
    }

    #[test]
    fn busy_matcher_accepts_busy_and_locked_codes() {
        for message in [
            "error returned from database: (code: 5) database is locked",
            "error returned from database: (code: 261) recovery in progress",
            "error returned from database: (code: 517) busy snapshot",
            "error returned from database: (code: 773) busy timeout",
            "error returned from database: (code: 6) database table is locked",
            "error returned from database: (code: 262) shared cache locked",
        ] {
            assert!(
                is_transient_sqlite_busy(&repo_error(message)),
                "expected transient: {message}"
            );
        }
    }

    #[test]
    fn busy_matcher_accepts_stable_message_fallbacks() {
        assert!(is_transient_sqlite_busy(&repo_error(
            "SQLITE_BUSY: unable to acquire lock"
        )));
        assert!(is_transient_sqlite_busy(&repo_error(
            "transaction failed: BUSY_SNAPSHOT"
        )));
    }

    #[test]
    fn busy_matcher_rejects_non_transient_codes() {
        for message in [
            "error returned from database: (code: 516) abort due to rollback",
            "error returned from database: (code: 522) disk I/O error (short read)",
            "error returned from database: (code: 526) unable to open database file",
            "error returned from database: (code: 531) constraint failed in commit hook",
            "error returned from database: (code: 1555) UNIQUE constraint failed: titles.id",
            "error returned from database: (code: 1) SQL logic error",
        ] {
            assert!(
                !is_transient_sqlite_busy(&repo_error(message)),
                "expected non-transient: {message}"
            );
        }
    }

    #[test]
    fn busy_matcher_ignores_non_repository_errors() {
        assert!(!is_transient_sqlite_busy(&AppError::Validation(
            "(code: 5) database is locked".to_string()
        )));
    }

    #[test]
    fn render_sql_sqlite_uses_question_marks() {
        let sql = render_sql(
            "INSERT INTO t (a, b) VALUES ({}, {})",
            PlaceholderDialect::Sqlite,
            2,
        )
        .expect("render sqlite");
        assert_eq!(sql, "INSERT INTO t (a, b) VALUES (?, ?)");
    }

    #[test]
    fn render_sql_postgres_numbers_placeholders() {
        let sql = render_sql(
            "INSERT INTO t (a, b, c) VALUES ({}, {}, {})",
            PlaceholderDialect::Postgres,
            3,
        )
        .expect("render postgres");
        assert_eq!(sql, "INSERT INTO t (a, b, c) VALUES ($1, $2, $3)");
    }

    #[test]
    fn render_sql_postgres_handles_multi_digit_indices() {
        // >9 placeholders exercises the multi-digit write! path ($10, $11, ...).
        let placeholder_count = 12usize;
        let template = vec!["{}"; placeholder_count].join(" ");
        let sql = render_sql(&template, PlaceholderDialect::Postgres, placeholder_count)
            .expect("render postgres multi-digit");
        let expected = (1..=placeholder_count)
            .map(|index| format!("${index}"))
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(sql, expected);
        assert!(sql.contains("$10"), "multi-digit index missing: {sql}");
        assert!(sql.contains("$12"), "multi-digit index missing: {sql}");
    }

    #[test]
    fn render_sql_rejects_arity_mismatch() {
        let err = render_sql("SELECT {} {}", PlaceholderDialect::Postgres, 1)
            .expect_err("expected arity mismatch error");
        let AppError::Repository(message) = err else {
            panic!("expected Repository error for arity mismatch");
        };
        assert!(
            message.contains("expected 2 bind(s), received 1"),
            "unexpected arity mismatch message: {message}"
        );
    }

    // --- execute_batch_insert -------------------------------------------------

    #[test]
    fn batch_rows_per_chunk_divides_by_width_and_clamps() {
        assert_eq!(batch_rows_per_chunk(1), BATCH_INSERT_MAX_BINDS);
        assert_eq!(batch_rows_per_chunk(2), 499);
        assert_eq!(batch_rows_per_chunk(3), 333);
        // A row exactly as wide as the cap still yields one row per statement.
        assert_eq!(batch_rows_per_chunk(BATCH_INSERT_MAX_BINDS), 1);
        // A row wider than the cap clamps to one row per statement, never zero.
        assert_eq!(batch_rows_per_chunk(BATCH_INSERT_MAX_BINDS + 1), 1);
    }

    #[test]
    fn batch_chunk_count_boundaries() {
        let per = batch_rows_per_chunk(2);
        assert_eq!(per, 499);
        // rows * width exactly at the cap => a single chunk.
        assert_eq!(499usize.div_ceil(per), 1);
        // one row over the cap => two chunks.
        assert_eq!(500usize.div_ceil(per), 2);
        // the round-trip test's 700-row / 2-col shape => two chunks.
        assert_eq!(700usize.div_ceil(per), 2);
        // row_width > cap clamps to one row per chunk => one chunk per row.
        let wide = batch_rows_per_chunk(BATCH_INSERT_MAX_BINDS + 1);
        assert_eq!(wide, 1);
        assert_eq!(3usize.div_ceil(wide), 3);
    }

    #[test]
    fn build_batch_insert_sql_emits_tuples_and_suffix() {
        let sql = build_batch_insert_sql("INSERT INTO t (a, b, c)", 3, 2, "ON CONFLICT DO NOTHING");
        assert_eq!(
            sql,
            "INSERT INTO t (a, b, c) VALUES ({}, {}, {}), ({}, {}, {}) ON CONFLICT DO NOTHING"
        );
        // one `{}` per column per row.
        assert_eq!(sql.matches("{}").count(), 3 * 2);
        // one placeholder tuple opener per row.
        assert_eq!(sql.matches("({}").count(), 2);
    }

    #[test]
    fn build_batch_insert_sql_omits_empty_or_blank_suffix() {
        let bare = build_batch_insert_sql("INSERT INTO t (a, b)", 2, 1, "");
        assert_eq!(bare, "INSERT INTO t (a, b) VALUES ({}, {})");
        assert!(
            !bare.ends_with(' '),
            "empty suffix must not leave a trailing space"
        );
        // whitespace-only suffix is trimmed away too.
        let blank = build_batch_insert_sql("INSERT INTO t (a)", 1, 3, "   ");
        assert_eq!(blank, "INSERT INTO t (a) VALUES ({}), ({}), ({})");
    }

    #[test]
    fn build_batch_insert_sql_renders_sequential_placeholders() {
        let sql = build_batch_insert_sql("INSERT INTO t (a, b)", 2, 3, "");
        let binds = 2 * 3;
        let postgres = render_sql(&sql, PlaceholderDialect::Postgres, binds).expect("render pg");
        assert_eq!(
            postgres,
            "INSERT INTO t (a, b) VALUES ($1, $2), ($3, $4), ($5, $6)"
        );
        let sqlite = render_sql(&sql, PlaceholderDialect::Sqlite, binds).expect("render sqlite");
        assert_eq!(sqlite, "INSERT INTO t (a, b) VALUES (?, ?), (?, ?), (?, ?)");
    }

    async fn memory_datastore(db_name: &str) -> StoreDatastore {
        use sqlx::sqlite::SqlitePoolOptions;
        // Unique shared-cache in-memory DB per test: `cache=shared` keeps the
        // schema alive across the pool's connections, and the distinct name
        // isolates parallel tests in this binary from one another. Pinning
        // min/idle/lifetime stops pool recycling from dropping the only
        // connection (which would discard the in-memory database).
        let url = format!("sqlite://file:{db_name}?mode=memory&cache=shared");
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .min_connections(1)
            .idle_timeout(None)
            .max_lifetime(None)
            .connect(&url)
            .await
            .expect("open shared-cache in-memory sqlite");
        StoreDatastore::Sqlite {
            pool,
            writer_gate: Arc::new(Mutex::new(())),
        }
    }

    fn sqlite_pool(datastore: &StoreDatastore) -> &SqlitePool {
        match datastore {
            StoreDatastore::Sqlite { pool, .. } => pool,
            StoreDatastore::Postgres { .. } => panic!("expected sqlite datastore"),
        }
    }

    #[tokio::test]
    async fn execute_batch_insert_round_trips_across_chunks() {
        let datastore = memory_datastore("batch_insert_roundtrip").await;
        let pool = sqlite_pool(&datastore).clone();
        sqlx::query("CREATE TABLE batch_probe (n INTEGER NOT NULL, label TEXT NOT NULL)")
            .execute(&pool)
            .await
            .expect("create scratch table");

        // 700 rows x 2 cols, cap 999 => 499 rows/chunk => exactly two chunks.
        let row_count = 700usize;
        assert_eq!(
            row_count.div_ceil(batch_rows_per_chunk(2)),
            2,
            "round-trip fixture must span two chunks"
        );
        let rows: Vec<Vec<SqlArg>> = (0..row_count)
            .map(|index| {
                vec![
                    SqlArg::I64(index as i64),
                    SqlArg::Text(format!("row-{index}")),
                ]
            })
            .collect();

        let affected =
            SqlRuntime::run_in_transaction(&datastore, "batch_probe_insert", move |tx| {
                let rows = rows.clone();
                Box::pin(async move {
                    SqlRuntime::execute_batch_insert(
                        tx,
                        "INSERT INTO batch_probe (n, label)",
                        2,
                        rows,
                        "",
                    )
                    .await
                })
            })
            .await
            .expect("batch insert should commit");
        assert_eq!(
            affected, row_count as u64,
            "rows_affected must sum across both chunks"
        );

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM batch_probe")
            .fetch_one(&pool)
            .await
            .expect("count rows");
        assert_eq!(count, row_count as i64);

        let first: String = sqlx::query_scalar("SELECT label FROM batch_probe WHERE n = 0")
            .fetch_one(&pool)
            .await
            .expect("first row present");
        assert_eq!(first, "row-0");
        let last: String = sqlx::query_scalar("SELECT label FROM batch_probe WHERE n = ?")
            .bind((row_count - 1) as i64)
            .fetch_one(&pool)
            .await
            .expect("last row present");
        assert_eq!(last, format!("row-{}", row_count - 1));
    }

    #[tokio::test]
    async fn execute_batch_insert_empty_rows_is_noop() {
        let datastore = memory_datastore("batch_insert_empty").await;
        let pool = sqlite_pool(&datastore).clone();
        sqlx::query("CREATE TABLE empty_probe (a TEXT NOT NULL)")
            .execute(&pool)
            .await
            .expect("create scratch table");

        let affected =
            SqlRuntime::run_in_transaction(&datastore, "empty_probe_insert", move |tx| {
                Box::pin(async move {
                    SqlRuntime::execute_batch_insert(
                        tx,
                        "INSERT INTO empty_probe (a)",
                        1,
                        Vec::new(),
                        "",
                    )
                    .await
                })
            })
            .await
            .expect("empty batch is a successful no-op");
        assert_eq!(affected, 0);

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM empty_probe")
            .fetch_one(&pool)
            .await
            .expect("count rows");
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn execute_batch_insert_rejects_zero_row_width() {
        let datastore = memory_datastore("batch_insert_zero_width").await;
        let outcome = SqlRuntime::run_in_transaction(&datastore, "zero_width_probe", move |tx| {
            Box::pin(async move {
                SqlRuntime::execute_batch_insert(
                    tx,
                    "INSERT INTO whatever (a)",
                    0,
                    vec![vec![SqlArg::Text("x".to_string())]],
                    "",
                )
                .await
            })
        })
        .await;
        match outcome {
            Err(AppError::Repository(message)) => assert!(
                message.contains("row_width"),
                "zero-width error must mention row_width: {message}"
            ),
            other => panic!("expected a row_width Repository error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn execute_batch_insert_rejects_ragged_rows_without_partial_write() {
        let datastore = memory_datastore("batch_insert_ragged").await;
        let pool = sqlite_pool(&datastore).clone();
        sqlx::query("CREATE TABLE ragged_probe (a TEXT NOT NULL, b TEXT NOT NULL)")
            .execute(&pool)
            .await
            .expect("create scratch table");

        let rows = vec![
            vec![
                SqlArg::Text("a0".to_string()),
                SqlArg::Text("b0".to_string()),
            ],
            vec![SqlArg::Text("only-one".to_string())], // ragged row at index 1
            vec![
                SqlArg::Text("a2".to_string()),
                SqlArg::Text("b2".to_string()),
            ],
        ];

        let outcome =
            SqlRuntime::run_in_transaction(&datastore, "ragged_probe_insert", move |tx| {
                let rows = rows.clone();
                Box::pin(async move {
                    SqlRuntime::execute_batch_insert(
                        tx,
                        "INSERT INTO ragged_probe (a, b)",
                        2,
                        rows,
                        "",
                    )
                    .await
                })
            })
            .await;

        match outcome {
            Err(AppError::Repository(message)) => {
                assert!(
                    message.contains("row 1"),
                    "ragged error must name the offending row index: {message}"
                );
                assert!(
                    message.contains("1 bind(s)") && message.contains("row_width 2"),
                    "ragged error must name both counts: {message}"
                );
            }
            other => panic!("expected a ragged-row Repository error, got {other:?}"),
        }

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ragged_probe")
            .fetch_one(&pool)
            .await
            .expect("count rows");
        assert_eq!(
            count, 0,
            "ragged rejection must not perform a partial insert"
        );
    }

    /// The writer gate is the sqlite serialization point, so its wait, hold
    /// and outcome must be observable per transaction name. The recorder is a
    /// thread-local, so the whole async body runs inside `block_on` on this
    /// same thread.
    #[test]
    fn run_in_transaction_records_writer_gate_metrics() {
        use std::collections::BTreeMap;

        use metrics_util::debugging::{DebugValue, DebuggingRecorder};

        use crate::writer_gate::{
            WRITER_GATE_HOLD_SECONDS, WRITER_GATE_WAIT_SECONDS, WRITER_TRANSACTIONS_TOTAL,
        };

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build current-thread runtime");
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();

        metrics::with_local_recorder(&recorder, || {
            runtime.block_on(async {
                let datastore = memory_datastore("writer_gate_metrics").await;
                let pool = sqlite_pool(&datastore).clone();
                sqlx::query("CREATE TABLE gate_probe (n INTEGER NOT NULL)")
                    .execute(&pool)
                    .await
                    .expect("create scratch table");

                SqlRuntime::run_in_transaction(&datastore, "gate_probe_insert", move |tx| {
                    Box::pin(async move {
                        SqlRuntime::execute(
                            SqlExec::Tx(tx),
                            "INSERT INTO gate_probe (n) VALUES ({})",
                            &[SqlArg::I64(1)],
                        )
                        .await
                    })
                })
                .await
                .expect("named write must commit");
            });
        });

        let series = snapshotter
            .snapshot()
            .into_vec()
            .into_iter()
            .map(|(key, _, _, value)| {
                let labels = key
                    .key()
                    .labels()
                    .map(|label| (label.key().to_string(), label.value().to_string()))
                    .collect::<BTreeMap<_, _>>();
                (key.key().name().to_string(), labels, value)
            })
            .collect::<Vec<_>>();

        let find = |name: &str| {
            series
                .iter()
                .find(|(metric, labels, _)| {
                    metric == name
                        && labels.get("transaction").map(String::as_str)
                            == Some("gate_probe_insert")
                })
                .unwrap_or_else(|| panic!("missing `{name}` series: {series:?}"))
        };

        for name in [WRITER_GATE_WAIT_SECONDS, WRITER_GATE_HOLD_SECONDS] {
            match &find(name).2 {
                DebugValue::Histogram(samples) => {
                    assert_eq!(samples.len(), 1, "`{name}` must record exactly one sample");
                    assert!(
                        samples[0].into_inner() >= 0.0,
                        "`{name}` sample must be a non-negative duration"
                    );
                }
                other => panic!("`{name}` must be a histogram, got {other:?}"),
            }
        }

        let (_, labels, value) = find(WRITER_TRANSACTIONS_TOTAL);
        assert_eq!(
            labels.get("outcome").map(String::as_str),
            Some("committed"),
            "a successful transaction must count as committed: {labels:?}"
        );
        assert_eq!(value, &DebugValue::Counter(1));
    }
}

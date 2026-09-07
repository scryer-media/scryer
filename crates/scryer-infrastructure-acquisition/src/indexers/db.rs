use chrono::{DateTime, Utc};
use scryer_application::AppResult;

use crate::queries::sql_runtime::{SqlArg, SqlRuntime, StoreDatastore};

/// One persisted indexer quota row, as read back at boot.
#[derive(Clone, Debug)]
pub struct PersistedIndexerQuota {
    pub indexer_id: String,
    pub api_requests_today: u32,
    /// UTC date the current window opened on, `YYYY-MM-DD`.
    pub window_started_on: Option<String>,
    pub api_current: Option<u32>,
    pub api_max: Option<u32>,
    pub grab_current: Option<u32>,
    pub grab_max: Option<u32>,
}

/// Bind the start of `now`'s UTC day in the shape the datastore stores
/// `last_reset_at` in, so the rollover comparison is a plain `<` on both.
///
/// SQLite keeps the column as text, so the bound value has to sort
/// lexicographically against what `CURRENT_TIMESTAMP` writes — `YYYY-MM-DD
/// HH:MM:SS`, *not* RFC3339. `SqlArg::Timestamp` would bind
/// `2026-09-06T00:00:00+00:00`, and `'T'` sorts after `' '`, so every flush
/// would read as a fresh window and reset the count. Postgres stores a real
/// `timestamptz` and takes the value natively.
fn window_start_arg(datastore: &StoreDatastore, now: DateTime<Utc>) -> SqlArg {
    let start_of_day = now
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .expect("midnight is a valid time")
        .and_utc();
    match datastore {
        StoreDatastore::Sqlite { .. } => {
            SqlArg::Text(start_of_day.format("%Y-%m-%d %H:%M:%S").to_string())
        }
        StoreDatastore::Postgres { .. } => SqlArg::Timestamp(start_of_day),
    }
}

/// Upsert quota snapshot for an indexer after a search response.
///
/// `query_delta` counts HTTP requests *sent* to the indexer since the last
/// flush, so it advances on retries, pagination, RSS sweeps, caps refreshes and
/// failures alike. The window resets on a UTC calendar-day boundary rather than
/// a rolling 24 hours from whenever the row happened to be created, because
/// that is what indexers themselves reset on; a rolling window drifted a little
/// further from the provider's own number every day.
///
/// The rollover is expressed as `last_reset_at < excluded.last_reset_at`
/// against a bound start-of-day rather than SQLite's `date('now')`, and the
/// clock reads use `CURRENT_TIMESTAMP` rather than `datetime('now')`, so the
/// statement runs identically on SQLite and Postgres. `last_reset_at` is also
/// written explicitly on insert instead of leaning on the column default, which
/// the two baselines spell differently.
pub async fn upsert_indexer_quota(
    datastore: &StoreDatastore,
    indexer_id: &str,
    api_current: Option<u32>,
    api_max: Option<u32>,
    grab_current: Option<u32>,
    grab_max: Option<u32>,
    query_delta: u32,
) -> AppResult<()> {
    SqlRuntime::execute_write(
        datastore,
        "upsert_indexer_quota",
        "INSERT INTO indexer_api_quotas (indexer_id, api_current, api_max, grab_current, grab_max, queries_today, last_reset_at, last_query_at, updated_at)
         VALUES ({}, {}, {}, {}, {}, {}, {}, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
         ON CONFLICT(indexer_id) DO UPDATE SET
           api_current = COALESCE(excluded.api_current, indexer_api_quotas.api_current),
           api_max = COALESCE(excluded.api_max, indexer_api_quotas.api_max),
           grab_current = COALESCE(excluded.grab_current, indexer_api_quotas.grab_current),
           grab_max = COALESCE(excluded.grab_max, indexer_api_quotas.grab_max),
           queries_today = CASE
             WHEN indexer_api_quotas.last_reset_at < excluded.last_reset_at
             THEN excluded.queries_today
             ELSE indexer_api_quotas.queries_today + excluded.queries_today
           END,
           last_reset_at = CASE
             WHEN indexer_api_quotas.last_reset_at < excluded.last_reset_at
             THEN excluded.last_reset_at
             ELSE indexer_api_quotas.last_reset_at
           END,
           last_query_at = CURRENT_TIMESTAMP,
           updated_at = CURRENT_TIMESTAMP",
        vec![
            SqlArg::Text(indexer_id.to_string()),
            SqlArg::OptI64(api_current.map(i64::from)),
            SqlArg::OptI64(api_max.map(i64::from)),
            SqlArg::OptI64(grab_current.map(i64::from)),
            SqlArg::OptI64(grab_max.map(i64::from)),
            SqlArg::I64(i64::from(query_delta)),
            window_start_arg(datastore, Utc::now()),
        ],
    )
    .await?;
    Ok(())
}

/// Read every persisted quota row.
///
/// The in-memory tracker starts empty on each restart while the persisted
/// counter keeps accumulating for the rest of the UTC day, so without this the
/// dashboard would under-report the account's real spend after every restart.
/// Rows whose window has already rolled over are returned as-is; the caller
/// drops them when it compares the stored date against today.
///
/// `last_reset_at` is read raw and reduced to a date in Rust rather than by
/// SQLite's `date()`, which Postgres does not have.
pub async fn load_indexer_quotas(
    datastore: &StoreDatastore,
) -> AppResult<Vec<PersistedIndexerQuota>> {
    let rows = SqlRuntime::fetch_all(
        datastore.read_exec(),
        "SELECT indexer_id,
                queries_today,
                last_reset_at,
                api_current,
                api_max,
                grab_current,
                grab_max
         FROM indexer_api_quotas",
        &[],
    )
    .await?;

    rows.into_iter()
        .map(|row| {
            Ok(PersistedIndexerQuota {
                indexer_id: row.text("indexer_id")?,
                api_requests_today: u32::try_from(row.i64("queries_today")?).unwrap_or(0),
                window_started_on: row
                    .opt_timestamp("last_reset_at")?
                    .map(|opened_at| opened_at.date_naive().to_string()),
                api_current: optional_u32(row.opt_i64("api_current")?),
                api_max: optional_u32(row.opt_i64("api_max")?),
                grab_current: optional_u32(row.opt_i64("grab_current")?),
                grab_max: optional_u32(row.opt_i64("grab_max")?),
            })
        })
        .collect()
}

fn optional_u32(value: Option<i64>) -> Option<u32> {
    value.and_then(|value| u32::try_from(value).ok())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sqlx::{SqlitePool, postgres::PgPoolOptions, sqlite::SqlitePoolOptions};

    use super::*;

    fn midnight_today() -> DateTime<Utc> {
        Utc::now()
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .expect("midnight is a valid time")
            .and_utc()
    }

    async fn sqlite_datastore() -> (SqlitePool, StoreDatastore) {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");
        // Mirrors the shipped SQLite baseline for `indexer_api_quotas`.
        sqlx::query(
            "CREATE TABLE indexer_api_quotas (
                indexer_id TEXT PRIMARY KEY NOT NULL,
                api_current INTEGER,
                api_max INTEGER,
                grab_current INTEGER,
                grab_max INTEGER,
                queries_today INTEGER NOT NULL DEFAULT 0,
                last_query_at TEXT,
                last_reset_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            )",
        )
        .execute(&pool)
        .await
        .expect("quota table should be created");
        let datastore = StoreDatastore::sqlite(pool.clone(), Arc::new(tokio::sync::Mutex::new(())));
        (pool, datastore)
    }

    #[tokio::test]
    async fn the_sqlite_window_bound_sorts_against_stored_timestamps() {
        let (_pool, datastore) = sqlite_datastore().await;
        let SqlArg::Text(bound) = window_start_arg(&datastore, Utc::now()) else {
            panic!("sqlite stores timestamps as text and must bind text");
        };
        assert!(
            !bound.contains('T'),
            "RFC3339 would sort after every `CURRENT_TIMESTAMP` value and roll the \
             window on every flush: {bound}"
        );
        assert!(bound.ends_with(" 00:00:00"), "expected midnight: {bound}");
        assert_eq!(
            bound,
            format!("{} 00:00:00", Utc::now().date_naive()),
            "the bound must be the start of today's UTC day"
        );
        // The comparison the statement relies on, spelled out.
        assert!(
            format!(
                "{} 23:59:59",
                Utc::now().date_naive() - chrono::Duration::days(1)
            ) < bound.clone(),
            "yesterday must sort before the window start"
        );
        assert!(
            bound < format!("{} 00:00:01", Utc::now().date_naive()),
            "a reading from today must not sort before the window start"
        );
    }

    #[tokio::test]
    async fn the_postgres_window_bound_stays_a_native_timestamp() {
        // `connect_lazy` never dials, so this exercises the dialect branch
        // without needing a server.
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy("postgres://scryer:scryer@127.0.0.1:5432/scryer")
            .expect("a lazy pool should build from a well-formed url");
        let datastore = StoreDatastore::Postgres { pool };
        let SqlArg::Timestamp(bound) = window_start_arg(&datastore, Utc::now()) else {
            panic!("postgres stores a timestamptz and must bind one natively");
        };
        assert_eq!(bound, midnight_today());
    }

    #[tokio::test]
    async fn sqlite_accumulates_within_a_day_and_resets_across_one() {
        let (pool, datastore) = sqlite_datastore().await;

        upsert_indexer_quota(&datastore, "idx-1", Some(7), Some(700), None, None, 3)
            .await
            .expect("first flush inserts");
        upsert_indexer_quota(&datastore, "idx-1", None, None, Some(2), Some(20), 4)
            .await
            .expect("second flush accumulates");

        let loaded = load_indexer_quotas(&datastore).await.expect("load");
        let row = loaded
            .iter()
            .find(|row| row.indexer_id == "idx-1")
            .expect("row");
        assert_eq!(row.api_requests_today, 7, "two flushes in one day add up");
        assert_eq!(
            row.api_current,
            Some(7),
            "a null reading must not clear a known one"
        );
        assert_eq!(row.api_max, Some(700));
        assert_eq!(row.grab_current, Some(2));
        assert_eq!(row.grab_max, Some(20));
        assert_eq!(
            row.window_started_on.as_deref(),
            Some(Utc::now().date_naive().to_string().as_str()),
            "the window opens at the start of today's UTC day"
        );

        // Backdate the window to yesterday; the next flush starts the count over.
        sqlx::query("UPDATE indexer_api_quotas SET last_reset_at = ? WHERE indexer_id = ?")
            .bind(format!(
                "{} 20:00:00",
                Utc::now().date_naive() - chrono::Duration::days(1)
            ))
            .bind("idx-1")
            .execute(&pool)
            .await
            .expect("backdate the window");

        upsert_indexer_quota(&datastore, "idx-1", None, None, None, None, 5)
            .await
            .expect("flush after the day boundary");
        let loaded = load_indexer_quotas(&datastore).await.expect("load");
        let row = loaded
            .iter()
            .find(|row| row.indexer_id == "idx-1")
            .expect("row");
        assert_eq!(
            row.api_requests_today, 5,
            "a new UTC day starts the count over instead of adding to yesterday's"
        );
        assert_eq!(
            row.window_started_on.as_deref(),
            Some(Utc::now().date_naive().to_string().as_str()),
            "the rollover moves the window to today"
        );
    }

    /// The Postgres twin of `sqlite_accumulates_within_a_day_and_resets_across_one`.
    ///
    /// Quota persistence has to behave identically on both datastores, and the
    /// statement uses `CURRENT_TIMESTAMP`, `excluded` and a bound window start
    /// precisely so one statement can run on either. Skipped unless
    /// `SCRYER_TEST_POSTGRES_URL` points at a database.
    #[tokio::test]
    async fn postgres_accumulates_within_a_day_and_resets_across_one_from_env() {
        let Some(database_url) = std::env::var("SCRYER_TEST_POSTGRES_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
        else {
            eprintln!(
                "skipping PostgreSQL indexer-quota parity test; SCRYER_TEST_POSTGRES_URL is not set"
            );
            return;
        };
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect(&database_url)
            .await
            .expect("postgres should connect");
        // Mirrors the shipped Postgres baseline for `indexer_api_quotas`.
        sqlx::query(
            "CREATE TEMP TABLE indexer_api_quotas (
                indexer_id text PRIMARY KEY NOT NULL,
                api_current bigint,
                api_max bigint,
                grab_current bigint,
                grab_max bigint,
                queries_today bigint DEFAULT 0 NOT NULL,
                last_query_at timestamp with time zone,
                last_reset_at timestamp with time zone DEFAULT now() NOT NULL,
                updated_at timestamp with time zone DEFAULT now() NOT NULL
            ) ON COMMIT PRESERVE ROWS",
        )
        .execute(&pool)
        .await
        .expect("temp quota table should be created");
        let datastore = StoreDatastore::Postgres { pool: pool.clone() };

        upsert_indexer_quota(&datastore, "idx-1", Some(7), Some(700), None, None, 3)
            .await
            .expect("first flush inserts");
        upsert_indexer_quota(&datastore, "idx-1", None, None, Some(2), Some(20), 4)
            .await
            .expect("second flush accumulates");

        let loaded = load_indexer_quotas(&datastore).await.expect("load");
        let row = loaded
            .iter()
            .find(|row| row.indexer_id == "idx-1")
            .expect("row");
        assert_eq!(row.api_requests_today, 7, "two flushes in one day add up");
        assert_eq!(
            row.api_current,
            Some(7),
            "a null reading must not clear a known one"
        );
        assert_eq!(row.api_max, Some(700));
        assert_eq!(row.grab_current, Some(2));
        assert_eq!(row.grab_max, Some(20));
        assert_eq!(
            row.window_started_on.as_deref(),
            Some(Utc::now().date_naive().to_string().as_str()),
            "the window opens at the start of today's UTC day"
        );

        sqlx::query("UPDATE indexer_api_quotas SET last_reset_at = $1 WHERE indexer_id = $2")
            .bind(
                (Utc::now().date_naive() - chrono::Duration::days(1))
                    .and_hms_opt(20, 0, 0)
                    .expect("20:00 is a valid time")
                    .and_utc(),
            )
            .bind("idx-1")
            .execute(&pool)
            .await
            .expect("backdate the window");

        upsert_indexer_quota(&datastore, "idx-1", None, None, None, None, 5)
            .await
            .expect("flush after the day boundary");
        let loaded = load_indexer_quotas(&datastore).await.expect("load");
        let row = loaded
            .iter()
            .find(|row| row.indexer_id == "idx-1")
            .expect("row");
        assert_eq!(
            row.api_requests_today, 5,
            "a new UTC day starts the count over instead of adding to yesterday's"
        );
        assert_eq!(
            row.window_started_on.as_deref(),
            Some(Utc::now().date_naive().to_string().as_str()),
            "the rollover moves the window to today"
        );

        sqlx::query("DROP TABLE indexer_api_quotas")
            .execute(&pool)
            .await
            .expect("temp table should drop");
    }
}

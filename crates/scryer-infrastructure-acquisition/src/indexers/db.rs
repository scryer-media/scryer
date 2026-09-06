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

/// Upsert quota snapshot for an indexer after a search response.
///
/// `query_delta` counts HTTP requests *sent* to the indexer since the last
/// flush, so it advances on retries, pagination, RSS sweeps, caps refreshes and
/// failures alike. The window resets on a UTC calendar-day boundary rather than
/// a rolling 24 hours from whenever the row happened to be created, because
/// that is what indexers themselves reset on; a rolling window drifted a little
/// further from the provider's own number every day.
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
        "INSERT INTO indexer_api_quotas (indexer_id, api_current, api_max, grab_current, grab_max, queries_today, last_query_at, updated_at)
         VALUES ({}, {}, {}, {}, {}, {}, datetime('now'), datetime('now'))
         ON CONFLICT(indexer_id) DO UPDATE SET
           api_current = COALESCE(excluded.api_current, indexer_api_quotas.api_current),
           api_max = COALESCE(excluded.api_max, indexer_api_quotas.api_max),
           grab_current = COALESCE(excluded.grab_current, indexer_api_quotas.grab_current),
           grab_max = COALESCE(excluded.grab_max, indexer_api_quotas.grab_max),
           queries_today = CASE
             WHEN date('now') <> date(indexer_api_quotas.last_reset_at)
             THEN excluded.queries_today
             ELSE indexer_api_quotas.queries_today + excluded.queries_today
           END,
           last_reset_at = CASE
             WHEN date('now') <> date(indexer_api_quotas.last_reset_at)
             THEN datetime('now', 'start of day')
             ELSE indexer_api_quotas.last_reset_at
           END,
           last_query_at = datetime('now'),
           updated_at = datetime('now')",
        vec![
            SqlArg::Text(indexer_id.to_string()),
            SqlArg::OptI64(api_current.map(i64::from)),
            SqlArg::OptI64(api_max.map(i64::from)),
            SqlArg::OptI64(grab_current.map(i64::from)),
            SqlArg::OptI64(grab_max.map(i64::from)),
            SqlArg::I64(i64::from(query_delta)),
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
pub async fn load_indexer_quotas(
    datastore: &StoreDatastore,
) -> AppResult<Vec<PersistedIndexerQuota>> {
    let rows = SqlRuntime::fetch_all(
        datastore.read_exec(),
        "SELECT indexer_id,
                queries_today,
                date(last_reset_at) AS window_started_on,
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
                window_started_on: row.opt_text("window_started_on")?,
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

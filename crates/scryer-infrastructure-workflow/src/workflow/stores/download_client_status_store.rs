use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scryer_application::{
    AppResult, DownloadClientStatusRepository, escalation_backoff::DownloadClientStatus,
};

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, SqlTx, StoreDatastore};

const STATUS_COLUMNS: &str = "client_config_id, initial_failure_at, most_recent_failure_at, \
                              escalation_level, disabled_until";

/// `download_client_status` (migration 0241), the download-client twin of the
/// indexer `indexer_system_backoffs` store.
///
/// Only failing clients have rows, so a success is a delete and `list` is the
/// whole table. The escalation policy itself lives in
/// [`DownloadClientStatus::after_failure`]; this store only reads the previous
/// row, applies it, and writes the result back inside one transaction so two
/// concurrent refresh ticks cannot both escalate from the same rung.
#[derive(Clone)]
pub struct DownloadClientStatusStore {
    datastore: StoreDatastore,
}

impl DownloadClientStatusStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

fn status_from_row(row: &SqlRow) -> AppResult<DownloadClientStatus> {
    Ok(DownloadClientStatus {
        initial_failure_at: row.opt_timestamp("initial_failure_at")?,
        most_recent_failure_at: row.opt_timestamp("most_recent_failure_at")?,
        escalation_level: row.i64("escalation_level")?.max(0) as usize,
        disabled_until: row.opt_timestamp("disabled_until")?,
    })
}

async fn load_status_tx(tx: &mut SqlTx<'_>, id: &str) -> AppResult<DownloadClientStatus> {
    let row = SqlRuntime::fetch_optional(
        SqlExec::Tx(tx),
        &format!(
            "SELECT {STATUS_COLUMNS} FROM download_client_status WHERE client_config_id = {{}}"
        ),
        &[SqlArg::Text(id.to_string())],
    )
    .await?;
    row.as_ref()
        .map(status_from_row)
        .transpose()
        .map(Option::unwrap_or_default)
}

#[async_trait]
impl DownloadClientStatusRepository for DownloadClientStatusStore {
    async fn list(&self) -> AppResult<HashMap<String, DownloadClientStatus>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &format!("SELECT {STATUS_COLUMNS} FROM download_client_status"),
            &[],
        )
        .await?;

        let mut statuses = HashMap::with_capacity(rows.len());
        for row in rows {
            statuses.insert(row.text("client_config_id")?, status_from_row(&row)?);
        }
        Ok(statuses)
    }

    async fn record_failure(
        &self,
        client_config_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<DownloadClientStatus> {
        let client_config_id = client_config_id.to_string();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "record_download_client_failure",
            move |tx| {
                let client_config_id = client_config_id.clone();
                Box::pin(async move {
                    let next = load_status_tx(tx, &client_config_id)
                        .await?
                        .after_failure(now);
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "INSERT INTO download_client_status (
                            client_config_id, initial_failure_at, most_recent_failure_at,
                            escalation_level, disabled_until
                         ) VALUES ({}, {}, {}, {}, {})
                         ON CONFLICT(client_config_id) DO UPDATE SET
                            initial_failure_at = excluded.initial_failure_at,
                            most_recent_failure_at = excluded.most_recent_failure_at,
                            escalation_level = excluded.escalation_level,
                            disabled_until = excluded.disabled_until",
                        &[
                            SqlArg::Text(client_config_id),
                            SqlArg::OptTimestamp(next.initial_failure_at),
                            SqlArg::OptTimestamp(next.most_recent_failure_at),
                            SqlArg::I64(next.escalation_level as i64),
                            SqlArg::OptTimestamp(next.disabled_until),
                        ],
                    )
                    .await?;
                    Ok(next)
                })
            },
        )
        .await
    }

    async fn record_success(&self, client_config_id: &str) -> AppResult<()> {
        self.clear(client_config_id).await
    }

    async fn clear(&self, client_config_id: &str) -> AppResult<()> {
        let client_config_id = client_config_id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "clear_download_client_status", move |tx| {
            let client_config_id = client_config_id.clone();
            Box::pin(async move {
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "DELETE FROM download_client_status WHERE client_config_id = {}",
                    &[SqlArg::Text(client_config_id)],
                )
                .await?;
                Ok(())
            })
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use sqlx::sqlite::SqlitePoolOptions;
    use std::sync::Arc;

    async fn store() -> DownloadClientStatusStore {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");
        sqlx::query(
            "CREATE TABLE download_client_status (
                 client_config_id TEXT PRIMARY KEY NOT NULL,
                 initial_failure_at TEXT,
                 most_recent_failure_at TEXT,
                 escalation_level INTEGER NOT NULL DEFAULT 0,
                 disabled_until TEXT
             )",
        )
        .execute(&pool)
        .await
        .expect("download client status fixture table should be created");
        DownloadClientStatusStore::new(StoreDatastore::Sqlite {
            pool,
            writer_gate: Arc::new(tokio::sync::Mutex::new(())),
        })
    }

    fn at(minutes: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + minutes * 60, 0).expect("fixture instant is valid")
    }

    #[tokio::test]
    async fn the_first_failure_persists_rung_one() {
        let store = store().await;

        let status = store
            .record_failure("client-alpha", at(0))
            .await
            .expect("record failure");

        assert_eq!(status.escalation_level, 1);
        assert_eq!(status.initial_failure_at, Some(at(0)));
        assert_eq!(status.disabled_until, Some(at(0) + Duration::seconds(60)));
        assert!(status.is_blocked(at(0)));
        assert_eq!(
            store.list().await.expect("list")["client-alpha"],
            status,
            "the persisted row round-trips"
        );
    }

    #[tokio::test]
    async fn failures_inside_the_five_minute_grace_do_not_escalate() {
        let store = store().await;

        store
            .record_failure("client-alpha", at(0))
            .await
            .expect("first failure");
        let second = store
            .record_failure("client-alpha", at(2))
            .await
            .expect("second failure");

        assert_eq!(second.escalation_level, 1);
        assert_eq!(second.initial_failure_at, Some(at(0)));
        assert_eq!(second.most_recent_failure_at, Some(at(2)));
    }

    #[tokio::test]
    async fn the_ladder_escalates_once_per_post_grace_failure_and_caps_at_level_five() {
        let store = store().await;
        store
            .record_failure("client-alpha", at(0))
            .await
            .expect("first failure");

        let mut last = None;
        for step in 0..8 {
            last = Some(
                store
                    .record_failure("client-alpha", at(10 + step * 10))
                    .await
                    .expect("later failure"),
            );
        }

        let last = last.expect("at least one later failure");
        assert_eq!(last.escalation_level, 5);
        assert_eq!(last.disabled_until, Some(at(80) + Duration::minutes(60)));
    }

    #[tokio::test]
    async fn success_clears_the_row_so_the_next_failure_starts_over() {
        let store = store().await;
        store
            .record_failure("client-alpha", at(0))
            .await
            .expect("first failure");
        store
            .record_failure("client-alpha", at(10))
            .await
            .expect("escalating failure");

        store
            .record_success("client-alpha")
            .await
            .expect("record success");

        assert!(
            store.list().await.expect("list").is_empty(),
            "a healthy client keeps no row"
        );
        let restarted = store
            .record_failure("client-alpha", at(20))
            .await
            .expect("failure after recovery");
        assert_eq!(restarted.escalation_level, 1);
        assert_eq!(restarted.initial_failure_at, Some(at(20)));
    }

    #[tokio::test]
    async fn clear_forgets_only_the_named_client() {
        let store = store().await;
        store
            .record_failure("client-alpha", at(0))
            .await
            .expect("alpha failure");
        store
            .record_failure("client-beta", at(0))
            .await
            .expect("beta failure");

        store.clear("client-alpha").await.expect("clear alpha");

        let listed = store.list().await.expect("list");
        assert_eq!(listed.len(), 1);
        assert!(listed.contains_key("client-beta"));
    }

    #[tokio::test]
    async fn clearing_an_unknown_client_is_a_no_op() {
        let store = store().await;

        store.clear("client-missing").await.expect("clear unknown");

        assert!(store.list().await.expect("list").is_empty());
    }
}

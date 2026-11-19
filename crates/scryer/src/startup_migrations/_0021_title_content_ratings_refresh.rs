use chrono::Utc;
use scryer_infrastructure_sql::runtime::{SqlArg, SqlExec, SqlRuntime, StoreDatastore};

pub(crate) const ID: &str = "0021_title_content_ratings_refresh";
pub(crate) const DESCRIPTION: &str =
    "queue metadata refresh so hydrated titles store their content ratings";

/// Queues every title hydrated before content ratings were stored. The normal
/// hydration worker makes the requests, paces them and retries failures; this
/// pass only sets the due time. Titles already waiting on a retry keep their
/// deadline, and titles never hydrated are already queued.
pub(crate) async fn migrate(datastore: &StoreDatastore) -> Result<(), String> {
    queue_hydrated_titles_once(datastore, ID, DESCRIPTION, "").await
}

/// Claims `id` and queues every hydrated title that matches `also` (an extra
/// `AND ...` clause over `titles`, or empty) and is not already waiting on a
/// retry, in one transaction, so the pass runs at most once.
pub(crate) async fn queue_hydrated_titles_once(
    datastore: &StoreDatastore,
    id: &'static str,
    description: &'static str,
    also: &'static str,
) -> Result<(), String> {
    SqlRuntime::run_in_transaction(datastore, id, move |tx| {
        Box::pin(async move {
            let now = Utc::now();
            // Claim and enqueue atomically, including when the outer runner's
            // completion write fails or another startup races this one.
            let claimed = SqlRuntime::execute(
                SqlExec::Tx(tx),
                "INSERT INTO application_migrations
                 (migration_id, description, applied_at, execution_time_ms)
                 VALUES ({}, {}, {}, 0) ON CONFLICT (migration_id) DO NOTHING",
                &[
                    SqlArg::Text(id.into()),
                    SqlArg::Text(description.into()),
                    SqlArg::Timestamp(now),
                ],
            )
            .await?;
            if claimed == 0 {
                return Ok(());
            }
            let sql = format!(
                "UPDATE titles SET metadata_hydration_next_attempt_at = {{}}
                 WHERE metadata_fetched_at IS NOT NULL
                   AND metadata_hydration_next_attempt_at IS NULL {also}"
            );
            SqlRuntime::execute(SqlExec::Tx(tx), &sql, &[SqlArg::Timestamp(now)]).await?;
            Ok(())
        })
    })
    .await
    .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    async fn fixture() -> (sqlx::SqlitePool, StoreDatastore) {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::raw_sql(
            "CREATE TABLE application_migrations (
            migration_id TEXT PRIMARY KEY, description TEXT, applied_at TEXT, execution_time_ms INTEGER);
            CREATE TABLE titles (id TEXT PRIMARY KEY, metadata_fetched_at TEXT,
            metadata_hydration_next_attempt_at TEXT,
            metadata_hydration_attempt_count INTEGER DEFAULT 0);",
        )
        .execute(&pool)
        .await
        .unwrap();
        let datastore = StoreDatastore::sqlite(pool.clone(), Arc::new(tokio::sync::Mutex::new(())));
        (pool, datastore)
    }

    async fn queued(pool: &sqlx::SqlitePool) -> Vec<String> {
        sqlx::query_scalar(
            "SELECT id FROM titles WHERE metadata_hydration_next_attempt_at IS NOT NULL ORDER BY id",
        )
        .fetch_all(pool)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn queues_hydrated_titles_once_and_keeps_retry_deadlines() {
        let (pool, datastore) = fixture().await;
        sqlx::raw_sql(
            "INSERT INTO titles (id, metadata_fetched_at, metadata_hydration_next_attempt_at,
                metadata_hydration_attempt_count) VALUES
             ('hydrated-a', '2026-01-01', NULL, 0),
             ('hydrated-b', '2026-02-01', NULL, 0),
             ('never-hydrated', NULL, NULL, 0),
             ('retrying', '2026-01-01', '2099-01-01', 3);",
        )
        .execute(&pool)
        .await
        .unwrap();

        migrate(&datastore).await.unwrap();
        assert_eq!(
            queued(&pool).await,
            vec!["hydrated-a", "hydrated-b", "retrying"]
        );
        let retry: (String, i64) = sqlx::query_as(
            "SELECT metadata_hydration_next_attempt_at, metadata_hydration_attempt_count
             FROM titles WHERE id = 'retrying'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(retry, ("2099-01-01".to_string(), 3));

        // Hydration finishes, then a later startup must not queue anything again.
        sqlx::query("UPDATE titles SET metadata_hydration_next_attempt_at = NULL")
            .execute(&pool)
            .await
            .unwrap();
        migrate(&datastore).await.unwrap();
        assert!(queued(&pool).await.is_empty());
        let markers: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM application_migrations WHERE migration_id = ?",
        )
        .bind(ID)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(markers, 1);
    }

    #[tokio::test]
    async fn failed_enqueue_rolls_back_claim_and_retries_successfully() {
        let (pool, datastore) = fixture().await;
        sqlx::raw_sql(
            "INSERT INTO titles (id, metadata_fetched_at) VALUES ('a', '2026-01-01');
             CREATE TRIGGER reject_enqueue BEFORE UPDATE ON titles
             BEGIN SELECT RAISE(ABORT, 'synthetic write failure'); END;",
        )
        .execute(&pool)
        .await
        .unwrap();
        assert!(migrate(&datastore).await.is_err());
        let markers: i64 = sqlx::query_scalar("SELECT count(*) FROM application_migrations")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!((markers, queued(&pool).await.len()), (0, 0));

        sqlx::raw_sql("DROP TRIGGER reject_enqueue;")
            .execute(&pool)
            .await
            .unwrap();
        migrate(&datastore).await.unwrap();
        assert_eq!(queued(&pool).await, vec!["a"]);
    }
}

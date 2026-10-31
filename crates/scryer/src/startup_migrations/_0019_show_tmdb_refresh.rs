use chrono::Utc;
use scryer_application::AppError;
use scryer_domain::ExternalId;
use scryer_infrastructure_sql::runtime::{SqlArg, SqlExec, SqlRuntime, StoreDatastore};

pub(crate) const ID: &str = "0019_show_tmdb_refresh";
pub(crate) const DESCRIPTION: &str = "queue metadata refresh for shows missing TMDB identities";

pub(crate) async fn migrate(datastore: &StoreDatastore) -> Result<(), String> {
    SqlRuntime::run_in_transaction(datastore, ID, |tx| {
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
                    SqlArg::Text(ID.into()),
                    SqlArg::Text(DESCRIPTION.into()),
                    SqlArg::Timestamp(now),
                ],
            )
            .await?;
            if claimed == 0 {
                return Ok(());
            }
            let mut cursor = String::new();
            loop {
                let rows = SqlRuntime::fetch_all(
                    SqlExec::Tx(tx),
                    "SELECT id, external_ids FROM titles
                     WHERE facet IN ('series', 'anime') AND id > {}
                       AND metadata_hydration_next_attempt_at IS NULL
                     ORDER BY id LIMIT 250",
                    &[SqlArg::Text(cursor.clone())],
                )
                .await?;
                if rows.is_empty() {
                    break;
                }
                for row in rows {
                    cursor = row.text("id")?;
                    let ids: Vec<ExternalId> = serde_json::from_str(&row.text("external_ids")?)
                        .map_err(|error| AppError::Repository(error.to_string()))?;
                    let has_tmdb = ids.iter().any(|id| {
                        id.source.trim().eq_ignore_ascii_case("tmdb")
                            && !id.value.trim().is_empty()
                            && id.kind.as_deref().is_none_or(|kind| {
                                kind.trim().is_empty() || kind.trim().eq_ignore_ascii_case("series")
                            })
                    });
                    if !has_tmdb {
                        // Preserve metadata and existing retry deadlines. The
                        // normal hydration worker performs requests and retries.
                        SqlRuntime::execute(
                            SqlExec::Tx(tx),
                            "UPDATE titles SET metadata_hydration_next_attempt_at = {}
                             WHERE id = {} AND metadata_hydration_next_attempt_at IS NULL",
                            &[SqlArg::Timestamp(now), SqlArg::Text(cursor.clone())],
                        )
                        .await?;
                    }
                }
            }
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
        sqlx::raw_sql("CREATE TABLE application_migrations (
            migration_id TEXT PRIMARY KEY, description TEXT, applied_at TEXT, execution_time_ms INTEGER);
            CREATE TABLE titles (id TEXT PRIMARY KEY, facet TEXT, external_ids TEXT,
            metadata_hydration_next_attempt_at TEXT, metadata_fetched_at TEXT DEFAULT '2026-01-01',
            metadata_hydration_attempt_count INTEGER DEFAULT 0);")
            .execute(&pool).await.unwrap();
        let datastore = StoreDatastore::sqlite(pool.clone(), Arc::new(tokio::sync::Mutex::new(())));
        (pool, datastore)
    }

    async fn insert(pool: &sqlx::SqlitePool, id: &str, facet: &str, ids: &str) {
        sqlx::query("INSERT INTO titles (id, facet, external_ids) VALUES (?, ?, ?)")
            .bind(id)
            .bind(facet)
            .bind(ids)
            .execute(pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn queues_missing_show_tmdb_once_across_pages_and_preserves_existing_state() {
        let (pool, datastore) = fixture().await;
        for i in 0..251 {
            insert(
                &pool,
                &format!("missing-{i:03}"),
                if i % 2 == 0 { "series" } else { "anime" },
                "[]",
            )
            .await;
        }
        for (id, facet, ids) in [
            ("movie", "movie", "[]"),
            (
                "complete",
                "series",
                r#"[{"source":"tmdb","kind":"series","value":"42"}]"#,
            ),
            ("unkinded", "series", r#"[{"source":"tmdb","value":"42"}]"#),
            (
                "normalized",
                "anime",
                r#"[{"source":" TMDB ","kind":" SERIES ","value":"42"}]"#,
            ),
            (
                "mapped-movie",
                "anime",
                r#"[{"source":"tmdb","kind":"movie","value":"42"}]"#,
            ),
            ("blank", "series", r#"[{"source":"tmdb","value":" "}]"#),
            ("retry", "series", "[]"),
        ] {
            insert(&pool, id, facet, ids).await;
        }
        sqlx::query("UPDATE titles SET metadata_hydration_next_attempt_at = '2099-01-01', metadata_hydration_attempt_count = 3 WHERE id = 'retry'")
            .execute(&pool).await.unwrap();
        let originals: Vec<(String, String, String, i64)> = sqlx::query_as("SELECT id, external_ids, metadata_fetched_at, metadata_hydration_attempt_count FROM titles ORDER BY id")
            .fetch_all(&pool).await.unwrap();
        migrate(&datastore).await.unwrap();
        let queued: Vec<String> = sqlx::query_scalar("SELECT id FROM titles WHERE metadata_hydration_next_attempt_at IS NOT NULL ORDER BY id")
            .fetch_all(&pool).await.unwrap();
        assert_eq!(queued.len(), 254);
        assert!(queued.contains(&"mapped-movie".into()));
        assert!(queued.contains(&"blank".into()));
        let after: Vec<(String, String, String, i64)> = sqlx::query_as("SELECT id, external_ids, metadata_fetched_at, metadata_hydration_attempt_count FROM titles ORDER BY id")
            .fetch_all(&pool).await.unwrap();
        assert_eq!(originals, after);
        let retry: String = sqlx::query_scalar(
            "SELECT metadata_hydration_next_attempt_at FROM titles WHERE id = 'retry'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(retry, "2099-01-01");
        // Simulate successful hydration that still has no upstream TMDB ID.
        sqlx::query(
            "UPDATE titles SET metadata_hydration_next_attempt_at = NULL WHERE id = 'missing-000'",
        )
        .execute(&pool)
        .await
        .unwrap();
        insert(&pool, "later-show", "series", "[]").await;
        migrate(&datastore).await.unwrap();
        let requeued: i64 = sqlx::query_scalar("SELECT count(*) FROM titles WHERE id IN ('missing-000', 'later-show') AND metadata_hydration_next_attempt_at IS NOT NULL")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(requeued, 0);
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
        insert(&pool, "a", "series", "[]").await;
        insert(&pool, "b", "anime", "[]").await;
        sqlx::raw_sql("CREATE TRIGGER reject_enqueue BEFORE UPDATE ON titles WHEN NEW.id = 'b' BEGIN SELECT RAISE(ABORT, 'synthetic write failure'); END;")
            .execute(&pool).await.unwrap();
        assert!(migrate(&datastore).await.is_err());
        let markers: i64 = sqlx::query_scalar("SELECT count(*) FROM application_migrations")
            .fetch_one(&pool)
            .await
            .unwrap();
        let queued: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM titles WHERE metadata_hydration_next_attempt_at IS NOT NULL",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!((markers, queued), (0, 0));
        sqlx::raw_sql("DROP TRIGGER reject_enqueue;")
            .execute(&pool)
            .await
            .unwrap();
        migrate(&datastore).await.unwrap();
        let queued: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM titles WHERE metadata_hydration_next_attempt_at IS NOT NULL",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(queued, 2);
    }
}

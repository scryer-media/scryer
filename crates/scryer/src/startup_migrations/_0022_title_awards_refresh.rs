use scryer_infrastructure_sql::runtime::StoreDatastore;

use super::_0021_title_content_ratings_refresh::queue_hydrated_titles_once;

pub(crate) const ID: &str = "0022_title_awards_refresh";
pub(crate) const DESCRIPTION: &str =
    "queue metadata refresh so hydrated titles store their awards and content ratings";

/// Queues every hydrated title whose awards or content ratings are not stored,
/// so lifecycle rules can read them. Rules read a set that is not stored as
/// unknown, never as empty, so nothing acts on a title before this refresh
/// reaches it. The hydration worker makes the requests and paces them.
pub(crate) async fn migrate(datastore: &StoreDatastore) -> Result<(), String> {
    queue_hydrated_titles_once(
        datastore,
        ID,
        DESCRIPTION,
        "AND (awards_stored_at IS NULL OR content_ratings_stored_at IS NULL)",
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn queues_hydrated_titles_missing_either_set_once() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::raw_sql(
            "CREATE TABLE application_migrations (
            migration_id TEXT PRIMARY KEY, description TEXT, applied_at TEXT, execution_time_ms INTEGER);
            CREATE TABLE titles (id TEXT PRIMARY KEY, metadata_fetched_at TEXT,
            metadata_hydration_next_attempt_at TEXT, content_ratings_stored_at TEXT,
            awards_stored_at TEXT);
            INSERT INTO titles VALUES
             ('both-stored', '2026-01-01', NULL, '2026-01-01', '2026-01-01'),
             ('no-awards', '2026-01-01', NULL, '2026-01-01', NULL),
             ('no-ratings', '2026-01-01', NULL, NULL, '2026-01-01'),
             ('never-hydrated', NULL, NULL, NULL, NULL),
             ('retrying', '2026-01-01', '2099-01-01', NULL, NULL);",
        )
        .execute(&pool)
        .await
        .unwrap();
        let datastore = StoreDatastore::sqlite(pool.clone(), Arc::new(tokio::sync::Mutex::new(())));
        let queued = || async {
            sqlx::query_scalar::<_, String>(
                "SELECT id FROM titles WHERE metadata_hydration_next_attempt_at IS NOT NULL
                  ORDER BY id",
            )
            .fetch_all(&pool)
            .await
            .unwrap()
        };

        migrate(&datastore).await.unwrap();
        assert_eq!(queued().await, vec!["no-awards", "no-ratings", "retrying"]);
        let retry: String = sqlx::query_scalar(
            "SELECT metadata_hydration_next_attempt_at FROM titles WHERE id = 'retrying'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(retry, "2099-01-01", "a retry keeps its deadline");

        sqlx::query("UPDATE titles SET metadata_hydration_next_attempt_at = NULL")
            .execute(&pool)
            .await
            .unwrap();
        migrate(&datastore).await.unwrap();
        assert!(queued().await.is_empty(), "the pass runs once");
    }
}

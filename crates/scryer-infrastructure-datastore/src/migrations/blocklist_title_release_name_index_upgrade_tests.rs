use sqlx::sqlite::SqlitePoolOptions;

const SQLITE: &str =
    include_str!("../../../scryer/src/db/migrations/0360_blocklist_title_release_name_index.sql");
const POSTGRES: &str = include_str!(
    "../../../scryer/src/db/postgres/migrations/0360_blocklist_title_release_name_index.sql"
);
const ADDED: &str = "idx_blocklist_title_release_name";
const DROPPED: &str = "idx_blocklist_title_id";

/// The search-time exclusion reads, as the blocklist store binds them.
const BY_NAME: &str = "EXPLAIN QUERY PLAN
    SELECT indexer_id, normalized_release_name, info_hash FROM blocklist
     WHERE title_id = ? AND normalized_release_name IN (?, ?)";
const BY_HASH: &str = "EXPLAIN QUERY PLAN
    SELECT indexer_id, normalized_release_name, info_hash FROM blocklist
     WHERE title_id = ? AND info_hash IS NOT NULL AND info_hash IN (?, ?)";
const BY_TITLE: &str = "EXPLAIN QUERY PLAN
    SELECT id FROM blocklist WHERE title_id = ? ORDER BY created_at DESC LIMIT 100";

/// Rows with the skew a real blocklist has: many rows per title, a share of
/// them torrents carrying an infohash.
const SEED: &str = "PRAGMA foreign_keys = OFF;
     WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 4000)
     INSERT INTO blocklist (id, title_id, release_name, reason, created_at,
                            normalized_release_name, indexer_id, info_hash)
     SELECT 'blocked-' || i, 'title-' || (1 + i % 40), 'Lantern.Cove.' || i, 'failed',
            printf('2026-%02d-%02dT00:00:00Z', 1 + i % 12, 1 + i % 28),
            'lantern.cove.' || i, 'indexer-' || (i % 4),
            CASE WHEN i % 3 = 0 THEN printf('%040d', i) ELSE NULL END
       FROM n;
     PRAGMA foreign_keys = ON;
     ANALYZE;";

async fn plan(pool: &sqlx::SqlitePool, sql: &'static str, binds: &[&str]) -> Vec<String> {
    let mut query = sqlx::query_as::<_, (i64, i64, i64, String)>(sql);
    for bind in binds {
        query = query.bind(*bind);
    }
    query
        .fetch_all(pool)
        .await
        .unwrap_or_else(|error| panic!("query plan should load for {sql}: {error}"))
        .into_iter()
        .map(|row| row.3)
        .collect()
}

async fn index_count(pool: &sqlx::SqlitePool, name: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = ?")
        .bind(name)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn assert_sqlite_plans(pool: &sqlx::SqlitePool, state: &str) {
    assert_eq!(index_count(pool, ADDED).await, 1, "{state}: {ADDED} exists");
    assert_eq!(
        index_count(pool, DROPPED).await,
        0,
        "{state}: {DROPPED} is gone"
    );
    let by_name = plan(
        pool,
        BY_NAME,
        &["title-7", "lantern.cove.7", "lantern.cove.47"],
    )
    .await;
    assert!(
        by_name.iter().any(|detail| detail.contains(ADDED)),
        "{state}: a name lookup reads {ADDED}: {by_name:?}"
    );
    let by_hash = plan(
        pool,
        BY_HASH,
        &[
            "title-7",
            "0000000000000000000000000000000000000087",
            "0000000000000000000000000000000000000127",
        ],
    )
    .await;
    assert!(
        by_hash
            .iter()
            .any(|detail| detail.contains("idx_blocklist_info_hash_unique")),
        "{state}: a hash lookup reads the infohash index: {by_hash:?}"
    );
    let by_title = plan(pool, BY_TITLE, &["title-7"]).await;
    assert!(
        by_title
            .iter()
            .any(|detail| detail.contains(&format!("SEARCH blocklist USING INDEX {ADDED}"))),
        "{state}: a title read stays index-served after {DROPPED} is dropped: {by_title:?}"
    );
}

#[tokio::test]
async fn blocklist_title_release_name_index_applies_on_fresh_install() {
    assert_eq!(
        SQLITE, POSTGRES,
        "both engines use the same index definitions"
    );
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    super::run_migrations(&pool, crate::MigrationMode::Apply)
        .await
        .expect("fresh migration catalog should apply");
    sqlx::raw_sql(SEED).execute(&pool).await.unwrap();
    assert_sqlite_plans(&pool, "fresh").await;
}

#[tokio::test]
async fn blocklist_title_release_name_index_upgrades_and_keeps_rows() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    super::replay_source_catalog_for_fresh_install(&pool, Some(359), true)
        .await
        .expect("pre-upgrade catalog should apply");
    assert_eq!(index_count(&pool, ADDED).await, 0);
    assert_eq!(index_count(&pool, DROPPED).await, 1);
    sqlx::raw_sql(SEED).execute(&pool).await.unwrap();
    super::run_migrations(&pool, crate::MigrationMode::Apply)
        .await
        .expect("upgrade must retain the published migration checksums");
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM blocklist")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 4000, "an index change keeps every row");
    sqlx::raw_sql("ANALYZE").execute(&pool).await.unwrap();
    assert_sqlite_plans(&pool, "upgraded").await;
    super::run_migrations(&pool, crate::MigrationMode::Apply)
        .await
        .expect("later startup must be idempotent");
    assert_sqlite_plans(&pool, "restarted").await;
}

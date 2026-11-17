use sqlx::Row;
use sqlx::sqlite::SqlitePoolOptions;

/// The migration that adds the audience favourites tables.
const AUDIENCE_FAVORITES_VERSION: i64 = 351;

const SQLITE_MIGRATION: &str =
    include_str!("../../../scryer/src/db/migrations/0351_media_server_audience_favorites.sql");
const POSTGRES_MIGRATION: &str = include_str!(
    "../../../scryer/src/db/postgres/migrations/0351_media_server_audience_favorites.sql"
);

async fn pool_before_upgrade() -> sqlx::SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    super::replay_source_catalog_for_fresh_install(
        &pool,
        Some(AUDIENCE_FAVORITES_VERSION - 1),
        true,
    )
    .await
    .expect("pre-upgrade catalog should apply");
    pool
}

async fn execute(pool: &sqlx::SqlitePool, sql: &str) {
    sqlx::query(sqlx::AssertSqlSafe(sql.to_string()))
        .execute(pool)
        .await
        .unwrap();
}

async fn count(pool: &sqlx::SqlitePool, sql: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_string()))
        .fetch_one(pool)
        .await
        .unwrap()
}

/// A synthetic connection with one participant favourite and one audience
/// play from before the upgrade.
async fn seed(pool: &sqlx::SqlitePool) {
    execute(
        pool,
        "INSERT INTO media_server_connections
            (id, provider, display_name, base_url, created_at, updated_at)
         VALUES ('connection-1', 'jellyfin', 'Fixture Server', 'http://server.invalid',
             '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .await;
    execute(
        pool,
        "INSERT INTO media_server_audience_plays
            (connection_id, external_account_id, provider_item_id, kind, scryer_title_id)
         VALUES ('connection-1', 'account-1', 'item-1', 'movie', 'title-1')",
    )
    .await;
}

async fn kept_rows(pool: &sqlx::SqlitePool) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT 'connection|' || id || '|' || display_name FROM media_server_connections
         UNION ALL
         SELECT 'play|' || connection_id || '|' || external_account_id || '|' || provider_item_id
           FROM media_server_audience_plays
         ORDER BY 1",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn the_upgrade_adds_empty_favourite_tables_and_keeps_everything_else() {
    let pool = pool_before_upgrade().await;
    seed(&pool).await;
    let kept = kept_rows(&pool).await;

    super::run_migrations(&pool, crate::MigrationMode::Apply)
        .await
        .expect("upgrade should apply");

    assert_eq!(kept_rows(&pool).await, kept, "nothing else changes");
    // Nothing is carried over: the favourite facts stay unknown until the
    // first sweep after the upgrade reads every account.
    assert_eq!(
        count(
            &pool,
            "SELECT COUNT(*) FROM media_server_audience_favorites"
        )
        .await,
        0
    );
    assert_eq!(
        count(
            &pool,
            "SELECT COUNT(*) FROM media_server_audience_favorite_reads"
        )
        .await,
        0
    );
    assert_eq!(
        count(
            &pool,
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'index' AND name = 'idx_media_server_audience_favorites_title'"
        )
        .await,
        1
    );

    execute(
        &pool,
        "INSERT INTO media_server_audience_favorites
            (connection_id, external_account_id, provider_item_id, kind, scryer_title_id)
         VALUES ('connection-1', 'account-1', 'series-1', 'series', 'title-1')",
    )
    .await;
    execute(
        &pool,
        "INSERT INTO media_server_audience_favorite_reads
            (connection_id, external_account_id, sweep_started_at, read_at)
         VALUES ('connection-1', 'account-1', '2026-01-02T00:00:00Z', '2026-01-02T00:00:01Z')",
    )
    .await;
    let favorite = sqlx::query(
        "SELECT kind, scryer_title_id FROM media_server_audience_favorites
         WHERE provider_item_id = 'series-1'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(favorite.get::<String, _>("kind"), "series");

    // Rows go with their connection.
    execute(&pool, "PRAGMA foreign_keys = ON").await;
    execute(
        &pool,
        "DELETE FROM media_server_connections WHERE id = 'connection-1'",
    )
    .await;
    assert_eq!(
        count(
            &pool,
            "SELECT COUNT(*) FROM media_server_audience_favorites"
        )
        .await,
        0
    );
    assert_eq!(
        count(
            &pool,
            "SELECT COUNT(*) FROM media_server_audience_favorite_reads"
        )
        .await,
        0
    );
}

#[tokio::test]
async fn a_second_run_is_a_no_op() {
    let pool = pool_before_upgrade().await;
    seed(&pool).await;
    super::run_migrations(&pool, crate::MigrationMode::Apply)
        .await
        .expect("upgrade should apply");
    execute(
        &pool,
        "INSERT INTO media_server_audience_favorite_reads
            (connection_id, external_account_id, sweep_started_at, failure_reason)
         VALUES ('connection-1', 'account-1', '2026-01-02T00:00:00Z', 'WATCH_SYNC_ITEMS_UNREADABLE')",
    )
    .await;
    super::run_migrations(&pool, crate::MigrationMode::Apply)
        .await
        .expect("a second run applies nothing");
    assert_eq!(
        count(
            &pool,
            "SELECT COUNT(*) FROM media_server_audience_favorite_reads"
        )
        .await,
        1
    );
}

#[test]
fn the_kinds_and_keys_match_on_both_engines() {
    for sql in [SQLITE_MIGRATION, POSTGRES_MIGRATION] {
        assert!(sql.contains("CHECK (kind IN ('movie', 'series', 'season', 'episode'))"));
        assert!(sql.contains("PRIMARY KEY (connection_id, external_account_id, provider_item_id)"));
        assert!(sql.contains("PRIMARY KEY (connection_id, external_account_id)"));
        assert!(sql.contains("ON media_server_audience_favorites(scryer_title_id)"));
        assert_eq!(sql.matches("ON DELETE CASCADE").count(), 2);
    }
    assert!(POSTGRES_MIGRATION.contains("read_at timestamp with time zone,"));
}

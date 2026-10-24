//! The runner applies a pending sequence on one connection.
//!
//! A pool connection caches the schema, and `RENAME COLUMN` / `DROP COLUMN`
//! resolve their column against that cache while parsing, without SQLite's
//! usual reload-and-retry. If 0309 (which adds `user_ui_settings.palette`) ran
//! on one pool connection and 0310 (which renames it) on another that had
//! cached the table before 0309, 0310 failed with `no such column`. That is
//! what every file-backed database with more than one connection hit.

use super::{embedded_catalog, embedded_payload_bytes, replay_catalog_into_fresh_db};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use std::str::FromStr;

const POOL_SIZE: u32 = 4;

async fn file_backed_pool(dir: &tempfile::TempDir) -> sqlx::SqlitePool {
    let path = dir.path().join("scryer.db");
    let options = SqliteConnectOptions::from_str(&format!("sqlite://{}?mode=rwc", path.display()))
        .unwrap()
        .journal_mode(SqliteJournalMode::Wal);
    SqlitePoolOptions::new()
        .max_connections(POOL_SIZE)
        .min_connections(0)
        .connect_with(options)
        .await
        .unwrap()
}

/// Opens every connection the pool allows and has each of them cache the
/// current `user_ui_settings` schema, so whichever one the pool hands out
/// next is holding the pre-0309 table.
async fn cache_schema_on_every_connection(pool: &sqlx::SqlitePool) {
    let mut held = Vec::new();
    for _ in 0..POOL_SIZE {
        held.push(pool.acquire().await.unwrap());
    }
    for conn in &mut held {
        sqlx::query("SELECT * FROM user_ui_settings LIMIT 0")
            .execute(&mut **conn)
            .await
            .unwrap();
    }
    assert_eq!(pool.size(), POOL_SIZE);
}

async fn column_names(pool: &sqlx::SqlitePool, table: &str) -> Vec<String> {
    sqlx::query_scalar("SELECT name FROM pragma_table_info(?1) ORDER BY cid")
        .bind(table)
        .fetch_all(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn palette_migrations_apply_on_a_multi_connection_file_pool() {
    let dir = tempfile::tempdir().unwrap();
    let pool = file_backed_pool(&dir).await;
    let catalog = embedded_catalog().unwrap();
    let payload = embedded_payload_bytes().unwrap();

    replay_catalog_into_fresh_db(&pool, &catalog, &payload, Some(308), true)
        .await
        .expect("catalog through 0308 should apply");
    cache_schema_on_every_connection(&pool).await;

    // 0309 adds the column and 0310 renames it, in one upgrade run.
    super::run_migrations(&pool, crate::MigrationMode::Apply)
        .await
        .expect("0309 and 0310 should apply on a multi-connection pool");

    let columns = column_names(&pool, "user_ui_settings").await;
    assert!(columns.iter().any(|name| name == "palette"), "{columns:?}");
    assert!(
        !columns.iter().any(|name| name == "previous_palette"),
        "{columns:?}"
    );
    let ceiling: i64 = sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(ceiling, catalog.max_version());
}

#[tokio::test]
async fn fresh_replay_applies_the_whole_catalog_on_a_multi_connection_file_pool() {
    let dir = tempfile::tempdir().unwrap();
    let pool = file_backed_pool(&dir).await;
    cache_schema_on_every_connection_before_any_table(&pool).await;

    super::run_migrations(&pool, crate::MigrationMode::Apply)
        .await
        .expect("a fresh install should apply on a multi-connection pool");

    let ceiling: i64 = sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(ceiling, embedded_catalog().unwrap().max_version());
}

/// A fresh database has no application tables yet; loading the empty schema
/// on every connection is enough for each to hold a cache that predates
/// everything the replay creates.
async fn cache_schema_on_every_connection_before_any_table(pool: &sqlx::SqlitePool) {
    let mut held = Vec::new();
    for _ in 0..POOL_SIZE {
        held.push(pool.acquire().await.unwrap());
    }
    for conn in &mut held {
        sqlx::query("SELECT COUNT(*) FROM sqlite_master")
            .execute(&mut **conn)
            .await
            .unwrap();
    }
    assert_eq!(pool.size(), POOL_SIZE);
}

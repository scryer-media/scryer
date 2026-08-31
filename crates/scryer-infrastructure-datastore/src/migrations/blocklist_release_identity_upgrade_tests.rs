//! End-to-end upgrade coverage for migration 0194.
//!
//! 0194 is deliberately lossy, so what it keeps and what it drops is behaviour
//! worth pinning: the release names survive (an upgrade must not release a burst
//! of re-grabs), the URLs do not, duplicates the missing constraint allowed are
//! collapsed once, and the two unique indexes stop them coming back.
//!
//! Each scenario drives the real migration runner over a database built at the
//! pre-0194 state by the real catalog.

use sqlx::{Row, SqlitePool};

/// Version the catalog is replayed to before 0194 is applied.
const PRE_UPGRADE_VERSION: i64 = 193;

async fn pre_upgrade_pool() -> SqlitePool {
    crate::spellfix::register_spellfix_auto_extension()
        .expect("spellfix extension should register before the migration fixture");
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory SQLite should open");
    crate::migrations::replay_source_catalog_for_fresh_install(
        &pool,
        Some(PRE_UPGRADE_VERSION),
        true,
    )
    .await
    .expect("pre-0194 migration fixture should apply");
    pool
}

async fn apply_upgrade(pool: &SqlitePool) {
    crate::migrations::run_migrations(pool, crate::MigrationMode::Apply)
        .await
        .expect("0194 upgrade should apply");
    let applied: i64 =
        sqlx::query_scalar("SELECT success FROM _sqlx_migrations WHERE version = 194")
            .fetch_one(pool)
            .await
            .expect("0194 ledger entry should exist");
    assert_eq!(applied, 1, "0194 must be recorded as successfully applied");
}

/// The blocklist has a foreign key onto titles, so a row needs a title.
///
/// `root_folder_id` is required by a trigger (0136) and is only checked for
/// being non-empty, so any placeholder satisfies it.
async fn seed_title(pool: &SqlitePool, id: &str) {
    sqlx::query(
        "INSERT INTO titles (id, name, facet, monitored, root_folder_id, created_at, updated_at)
         VALUES (?1, ?1, 'movie', 1, 'fixture-root', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .bind(id)
    .execute(pool)
    .await
    .expect("title should insert");
}

/// A pre-0194 blocklist row, in the shape the old writers produced.
async fn seed_legacy_entry(
    pool: &SqlitePool,
    id: &str,
    title_id: &str,
    source_title: Option<&str>,
    source_hint: Option<&str>,
    created_at: &str,
) {
    sqlx::query(
        "INSERT INTO blocklist
         (id, title_id, source_title, source_hint, quality, download_id, reason, data_json, created_at)
         VALUES (?1, ?2, ?3, ?4, '1080p', 'job-1', 'grab failed', '{}', ?5)",
    )
    .bind(id)
    .bind(title_id)
    .bind(source_title)
    .bind(source_hint)
    .bind(created_at)
    .execute(pool)
    .await
    .expect("legacy blocklist row should insert");
}

async fn entries(pool: &SqlitePool, title_id: &str) -> Vec<(String, String, String, String)> {
    sqlx::query(
        "SELECT id, release_name, normalized_release_name, indexer_id
           FROM blocklist WHERE title_id = ?1 ORDER BY normalized_release_name",
    )
    .bind(title_id)
    .fetch_all(pool)
    .await
    .expect("blocklist rows should load")
    .iter()
    .map(|row| {
        (
            row.get::<String, _>("id"),
            row.get::<String, _>("release_name"),
            row.get::<String, _>("normalized_release_name"),
            row.get::<String, _>("indexer_id"),
        )
    })
    .collect()
}

#[tokio::test]
async fn the_upgrade_keeps_release_names_and_drops_the_url_columns() {
    let pool = pre_upgrade_pool().await;
    seed_title(&pool, "title-1").await;
    seed_legacy_entry(
        &pool,
        "entry-1",
        "title-1",
        Some("  Signal.Run.S01E12.1080p.WEB-DL.x265-NTb "),
        Some("https://example.invalid/dl?apikey=SECRET"),
        "2026-01-02T00:00:00Z",
    )
    .await;

    apply_upgrade(&pool).await;

    let rows = entries(&pool, "title-1").await;
    assert_eq!(rows.len(), 1, "the legacy block survives the upgrade");
    let (_, release_name, normalized, indexer_id) = &rows[0];
    assert_eq!(
        release_name, "  Signal.Run.S01E12.1080p.WEB-DL.x265-NTb ",
        "the raw name is preserved verbatim for display"
    );
    assert_eq!(
        normalized, "signal.run.s01e12.1080p.web-dl.x265-ntb",
        "the matcher key is trimmed and lowercased"
    );
    assert_eq!(indexer_id, "", "a legacy row blocks on every indexer");

    // The API key left with the column, not scrubbed in place.
    let leftover: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('blocklist')
          WHERE name IN ('source_hint', 'quality', 'download_id', 'data_json')",
    )
    .fetch_one(&pool)
    .await
    .expect("column list should load");
    assert_eq!(leftover, 0, "every dropped column is gone from the schema");
}

#[tokio::test]
async fn the_upgrade_collapses_duplicates_newest_first_and_drops_nameless_rows() {
    let pool = pre_upgrade_pool().await;
    seed_title(&pool, "title-1").await;
    // The same failure recorded twice, differing only in casing -- exactly what
    // the missing constraint allowed and what the sibling sweep used to clean up.
    seed_legacy_entry(
        &pool,
        "older",
        "title-1",
        Some("Duplicated.Movie.2024.1080p.WEB-DL-GRP"),
        Some("https://example.invalid/a"),
        "2026-01-01T00:00:00Z",
    )
    .await;
    seed_legacy_entry(
        &pool,
        "newer",
        "title-1",
        Some("duplicated.movie.2024.1080p.web-dl-grp"),
        None,
        "2026-01-05T00:00:00Z",
    )
    .await;
    // A row that names nothing can never match, so it is dropped outright.
    seed_legacy_entry(
        &pool,
        "nameless",
        "title-1",
        None,
        Some("https://x/y"),
        "2026-01-03T00:00:00Z",
    )
    .await;

    apply_upgrade(&pool).await;

    let rows = entries(&pool, "title-1").await;
    assert_eq!(
        rows.len(),
        1,
        "the duplicate pair collapses to one row: {rows:?}"
    );
    assert_eq!(rows[0].0, "newer", "the newest row wins");
}

#[tokio::test]
async fn the_upgrade_leaves_a_constraint_that_refuses_a_duplicate_block() {
    let pool = pre_upgrade_pool().await;
    seed_title(&pool, "title-1").await;
    seed_legacy_entry(
        &pool,
        "entry-1",
        "title-1",
        Some("Constrained.Movie.2024.1080p.WEB-DL-GRP"),
        None,
        "2026-01-01T00:00:00Z",
    )
    .await;

    apply_upgrade(&pool).await;

    // Same title, same indexer, same normalized name: the unique index refuses
    // it, so idempotence no longer depends on a read-then-write.
    let duplicate = sqlx::query(
        "INSERT INTO blocklist
         (id, title_id, release_name, normalized_release_name, indexer_id, info_hash, reason, created_at)
         VALUES ('entry-2', 'title-1', 'Constrained.Movie.2024.1080p.WEB-DL-GRP',
                 'constrained.movie.2024.1080p.web-dl-grp', '', NULL, 'again', '2026-01-06T00:00:00Z')",
    )
    .execute(&pool)
    .await;
    assert!(
        duplicate.is_err(),
        "a second identical block must be refused"
    );

    // A different indexer is a different block, and is allowed.
    sqlx::query(
        "INSERT INTO blocklist
         (id, title_id, release_name, normalized_release_name, indexer_id, info_hash, reason, created_at)
         VALUES ('entry-3', 'title-1', 'Constrained.Movie.2024.1080p.WEB-DL-GRP',
                 'constrained.movie.2024.1080p.web-dl-grp', 'indexer-b', NULL, 'other', '2026-01-06T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .expect("the same release on another indexer is a distinct block");

    assert_eq!(entries(&pool, "title-1").await.len(), 2);
}

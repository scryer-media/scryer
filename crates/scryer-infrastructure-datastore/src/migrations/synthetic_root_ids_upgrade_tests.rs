//! End-to-end upgrade coverage for migrations 0210, 0211 and 0212 (numbered
//! 0204-0206 on the feature branch, 0208-0210 on release-NEXT before main's
//! shipped 0204/0205 were merged in; renumbered each time).
//!
//! Each scenario drives the real migration runner over a database the real
//! catalog built at the pre-upgrade state, because the behaviour worth pinning is
//! what a *user's* existing catalog looks like afterwards: a root that keeps its
//! titles across a re-key, a legacy id that is still resolvable, and the new
//! tables actually existing on both engines.

use sqlx::{Row, SqlitePool};

use scryer_domain::{normalize_library_root_path, root_folder_id_for_normalized_path};

use super::synthetic_root_ids::synthetic_root_id_from_legacy_id;

/// Version the catalog is replayed to before the new migrations are applied.
const PRE_UPGRADE_VERSION: i64 = 203;

fn path_derived_id(path: &str) -> String {
    root_folder_id_for_normalized_path(&normalize_library_root_path(path))
}

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
    .expect("pre-upgrade migration fixture should apply");
    pool
}

async fn apply_upgrade(pool: &SqlitePool) {
    crate::migrations::run_migrations(pool, crate::MigrationMode::Apply)
        .await
        .expect("synthetic-root-id upgrade should apply");
    for version in [210, 211, 212] {
        let applied: i64 =
            sqlx::query_scalar("SELECT success FROM _sqlx_migrations WHERE version = ?1")
                .bind(version)
                .fetch_one(pool)
                .await
                .unwrap_or_else(|error| panic!("{version} ledger entry should exist: {error}"));
        assert_eq!(
            applied, 1,
            "{version} must be recorded as successfully applied"
        );
    }
}

/// A root in the shape the pre-0210 writer produced: id == hash(normalized path).
async fn seed_path_derived_root(pool: &SqlitePool, library_id: &str, path: &str) -> String {
    let id = path_derived_id(path);
    sqlx::query(
        "INSERT INTO library_roots
             (id, library_id, path, normalized_path, is_default, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, 0, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .bind(&id)
    .bind(library_id)
    .bind(path)
    .bind(normalize_library_root_path(path))
    .execute(pool)
    .await
    .expect("path-derived root should insert");
    id
}

async fn seed_title(pool: &SqlitePool, id: &str, library_id: &str, root_folder_id: &str) {
    sqlx::query(
        "INSERT INTO titles
             (id, library_id, name, facet, monitored, root_folder_id, created_at, updated_at)
         VALUES (?1, ?2, ?1, 'movie', 1, ?3, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .bind(id)
    .bind(library_id)
    .bind(root_folder_id)
    .execute(pool)
    .await
    .expect("title should insert");
}

async fn title_root(pool: &SqlitePool, title_id: &str) -> String {
    sqlx::query_scalar("SELECT root_folder_id FROM titles WHERE id = ?1")
        .bind(title_id)
        .fetch_one(pool)
        .await
        .expect("title should load")
}

async fn root_exists(pool: &SqlitePool, root_id: &str) -> bool {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM library_roots WHERE id = ?1")
        .bind(root_id)
        .fetch_one(pool)
        .await
        .expect("root count should load");
    count == 1
}

// ── 0210 ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn the_upgrade_rekeys_path_derived_roots_and_carries_their_titles_along() {
    let pool = pre_upgrade_pool().await;
    let legacy_id =
        seed_path_derived_root(&pool, "movie_default_library", "/mnt/archive/movies").await;
    seed_title(&pool, "title-1", "movie_default_library", &legacy_id).await;
    seed_title(&pool, "title-2", "movie_default_library", &legacy_id).await;

    apply_upgrade(&pool).await;

    let expected_id = synthetic_root_id_from_legacy_id(&legacy_id);
    assert!(
        root_exists(&pool, &expected_id).await,
        "the root must exist under its synthetic id"
    );
    assert!(
        !root_exists(&pool, &legacy_id).await,
        "the path-derived id must be gone"
    );
    assert_eq!(title_root(&pool, "title-1").await, expected_id);
    assert_eq!(title_root(&pool, "title-2").await, expected_id);
}

#[tokio::test]
async fn the_upgrade_retains_the_legacy_id_for_diagnostics_and_lookup() {
    let pool = pre_upgrade_pool().await;
    let legacy_id =
        seed_path_derived_root(&pool, "movie_default_library", "/mnt/archive/movies").await;

    apply_upgrade(&pool).await;

    let new_id = synthetic_root_id_from_legacy_id(&legacy_id);
    let retained: String =
        sqlx::query_scalar("SELECT legacy_path_derived_id FROM library_roots WHERE id = ?1")
            .bind(&new_id)
            .fetch_one(&pool)
            .await
            .expect("legacy id column should load");
    assert_eq!(retained, legacy_id);

    let row = sqlx::query(
        "SELECT root_id, normalized_path, remapped
           FROM library_root_id_remaps WHERE legacy_root_id = ?1",
    )
    .bind(&legacy_id)
    .fetch_one(&pool)
    .await
    .expect("remap row should exist");
    assert_eq!(row.get::<String, _>("root_id"), new_id);
    assert_eq!(
        row.get::<String, _>("normalized_path"),
        normalize_library_root_path("/mnt/archive/movies")
    );
    assert_eq!(row.get::<i64, _>("remapped"), 1);
}

#[tokio::test]
async fn the_upgrade_leaves_roots_that_were_never_path_derived_in_place() {
    let pool = pre_upgrade_pool().await;
    // The baseline's seeded default roots already carry non-path-derived ids;
    // re-keying them would break stable references for no gain.
    seed_title(
        &pool,
        "seeded-title",
        "movie_default_library",
        "canonical_root_for_movie_default_library",
    )
    .await;

    apply_upgrade(&pool).await;

    assert!(
        root_exists(&pool, "canonical_root_for_movie_default_library").await,
        "the seeded canonical root must keep its id"
    );
    assert_eq!(
        title_root(&pool, "seeded-title").await,
        "canonical_root_for_movie_default_library"
    );
    // The alias is still recorded, so a caller holding the path-derived id can
    // still find the root.
    let mapped: String =
        sqlx::query_scalar("SELECT root_id FROM library_root_id_remaps WHERE legacy_root_id = ?1")
            .bind(path_derived_id("/data/movies"))
            .fetch_one(&pool)
            .await
            .expect("alias row should exist for the seeded root");
    assert_eq!(mapped, "canonical_root_for_movie_default_library");
}

#[tokio::test]
async fn a_root_path_change_after_the_upgrade_no_longer_changes_identity() {
    let pool = pre_upgrade_pool().await;
    let legacy_id = seed_path_derived_root(&pool, "movie_default_library", "/mnt/old").await;
    seed_title(&pool, "moving-title", "movie_default_library", &legacy_id).await;

    apply_upgrade(&pool).await;

    let root_id = synthetic_root_id_from_legacy_id(&legacy_id);
    sqlx::query(
        "UPDATE library_roots SET path = '/mnt/new', normalized_path = '/mnt/new' WHERE id = ?1",
    )
    .bind(&root_id)
    .execute(&pool)
    .await
    .expect("root path should update");

    assert_eq!(title_root(&pool, "moving-title").await, root_id);
    assert!(root_exists(&pool, &root_id).await);
}

// ── 0211 ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn the_upgrade_adds_nullable_full_hash_columns_to_media_files() {
    let pool = pre_upgrade_pool().await;
    seed_title(
        &pool,
        "hash-title",
        "movie_default_library",
        "canonical_root_for_movie_default_library",
    )
    .await;
    sqlx::query(
        "INSERT INTO media_files (id, title_id, file_path, size_bytes, created_at)
         VALUES ('file-1', 'hash-title', '/data/movies/a.mkv', 10, '2026-01-01T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .expect("pre-upgrade media file should insert");

    apply_upgrade(&pool).await;

    let row = sqlx::query(
        "SELECT full_blake3, move_crc, move_crc_algorithm, hash_computed_at
           FROM media_files WHERE id = 'file-1'",
    )
    .fetch_one(&pool)
    .await
    .expect("media file should load");
    // Existing rows keep no full hash: only a read-once pass may set one.
    assert_eq!(row.get::<Option<String>, _>("full_blake3"), None);
    assert_eq!(row.get::<Option<String>, _>("move_crc"), None);
    assert_eq!(row.get::<Option<String>, _>("move_crc_algorithm"), None);
    assert_eq!(row.get::<Option<String>, _>("hash_computed_at"), None);

    sqlx::query(
        "UPDATE media_files
            SET full_blake3 = 'b3hash',
                move_crc = 'deadbeef',
                move_crc_algorithm = 'crc64/nvme',
                hash_computed_at = '2026-02-01T00:00:00Z'
          WHERE id = 'file-1'",
    )
    .execute(&pool)
    .await
    .expect("full hashes should persist");
    let algorithm: String =
        sqlx::query_scalar("SELECT move_crc_algorithm FROM media_files WHERE id = 'file-1'")
            .fetch_one(&pool)
            .await
            .expect("algorithm should load");
    assert_eq!(algorithm, "crc64/nvme");
}

// ── 0212 ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn the_upgrade_creates_the_location_operation_tables() {
    let pool = pre_upgrade_pool().await;
    apply_upgrade(&pool).await;

    sqlx::query(
        "INSERT INTO location_operations
             (id, operation_type, execution_mode, state, verification_depth)
         VALUES ('op-1', 'root_change', 'move', 'running', 'full')",
    )
    .execute(&pool)
    .await
    .expect("operation row should insert");
    sqlx::query(
        "INSERT INTO location_operation_title_checkpoints
             (operation_id, title_id, sequence, state, classification, note)
         VALUES ('op-1', 'title-1', 0, 'verified', 'move',
                 'one companion asset was renamed')",
    )
    .execute(&pool)
    .await
    .expect("checkpoint row should insert");
    sqlx::query(
        "INSERT INTO location_operation_verifications
             (id, operation_id, title_id, source_path, destination_path,
              requested_depth, applied_depth, fell_back, fallback_reason, outcome)
         VALUES ('ver-1', 'op-1', 'title-1', '/src/a.mkv', '/dst/a.mkv',
                 'full', 'quick', 1, 'read_back_unavailable', 'passed')",
    )
    .execute(&pool)
    .await
    .expect("verification row should insert");
    sqlx::query(
        "INSERT INTO location_operation_verifications
             (id, operation_id, title_id, source_path, destination_path,
              requested_depth, applied_depth, outcome, detail)
         VALUES ('ver-2', 'op-1', 'title-1', '/src/b.mkv', '/dst/b.mkv',
                 'full', 'full', 'passed', 'verified (full)')",
    )
    .execute(&pool)
    .await
    .expect("verification row with a plain detail should insert");
    sqlx::query(
        "INSERT INTO location_operation_owned_entities
             (operation_id, entity_type, entity_id)
         VALUES ('op-1', 'title', 'title-1')",
    )
    .execute(&pool)
    .await
    .expect("ownership row should insert");

    // The applied depth and its fallback survive, so the weaker guarantee stays
    // auditable after the fact (FR-043).
    let row = sqlx::query(
        "SELECT applied_depth, fell_back, fallback_reason
           FROM location_operation_verifications WHERE id = 'ver-1'",
    )
    .fetch_one(&pool)
    .await
    .expect("verification should load");
    assert_eq!(row.get::<String, _>("applied_depth"), "quick");
    assert_eq!(row.get::<i64, _>("fell_back"), 1);
    assert_eq!(
        row.get::<String, _>("fallback_reason"),
        "read_back_unavailable"
    );

    // A verification that neither fell back nor failed still has somewhere to
    // put its note, separate from the fallback and failure columns (FR-043).
    let plain = sqlx::query(
        "SELECT fallback_reason, failure_reason, detail
           FROM location_operation_verifications WHERE id = 'ver-2'",
    )
    .fetch_one(&pool)
    .await
    .expect("verification should load");
    assert_eq!(plain.get::<Option<String>, _>("fallback_reason"), None);
    assert_eq!(plain.get::<Option<String>, _>("failure_reason"), None);
    assert_eq!(plain.get::<String, _>("detail"), "verified (full)");

    // Likewise a checkpoint's warning note, separate from blocked/failed.
    let checkpoint = sqlx::query(
        "SELECT blocked_reason, failure_reason, note
           FROM location_operation_title_checkpoints WHERE title_id = 'title-1'",
    )
    .fetch_one(&pool)
    .await
    .expect("checkpoint should load");
    assert_eq!(checkpoint.get::<Option<String>, _>("blocked_reason"), None);
    assert_eq!(checkpoint.get::<Option<String>, _>("failure_reason"), None);
    assert_eq!(
        checkpoint.get::<String, _>("note"),
        "one companion asset was renamed"
    );

    // The FR-091 outcome counters default to zero and take real values.
    let counters = sqlx::query(
        "SELECT merge_count, dedup_count, rename_count, no_op_count, unresolved_count
           FROM location_operations WHERE id = 'op-1'",
    )
    .fetch_one(&pool)
    .await
    .expect("operation should load");
    for column in [
        "merge_count",
        "dedup_count",
        "rename_count",
        "no_op_count",
        "unresolved_count",
    ] {
        assert_eq!(
            counters.get::<i64, _>(column),
            0,
            "{column} should default to zero"
        );
    }
    sqlx::query(
        "UPDATE location_operations
            SET merge_count = 1, dedup_count = 2, rename_count = 3,
                no_op_count = 4, unresolved_count = 5
          WHERE id = 'op-1'",
    )
    .execute(&pool)
    .await
    .expect("counters should update");
    let updated: i64 = sqlx::query_scalar(
        "SELECT merge_count + dedup_count + rename_count + no_op_count + unresolved_count
           FROM location_operations WHERE id = 'op-1'",
    )
    .fetch_one(&pool)
    .await
    .expect("counters should load");
    assert_eq!(updated, 15);
}

#[tokio::test]
async fn two_operations_cannot_own_the_same_entity_at_once() {
    let pool = pre_upgrade_pool().await;
    apply_upgrade(&pool).await;

    for id in ["op-1", "op-2"] {
        sqlx::query(
            "INSERT INTO location_operations (id, operation_type, execution_mode, state)
             VALUES (?1, 'root_move', 'move', 'running')",
        )
        .bind(id)
        .execute(&pool)
        .await
        .expect("operation row should insert");
    }

    sqlx::query(
        "INSERT INTO location_operation_owned_entities (operation_id, entity_type, entity_id)
         VALUES ('op-1', 'title', 'contested-title')",
    )
    .execute(&pool)
    .await
    .expect("first claim should succeed");

    let contested = sqlx::query(
        "INSERT INTO location_operation_owned_entities (operation_id, entity_type, entity_id)
         VALUES ('op-2', 'title', 'contested-title')",
    )
    .execute(&pool)
    .await;
    assert!(
        contested.is_err(),
        "a second live claim on the same title must be rejected by the registry"
    );

    // Releasing the first claim frees the entity for the next operation.
    sqlx::query(
        "UPDATE location_operation_owned_entities
            SET released_at = '2026-01-01T00:00:00Z'
          WHERE operation_id = 'op-1'",
    )
    .execute(&pool)
    .await
    .expect("release should apply");
    sqlx::query(
        "INSERT INTO location_operation_owned_entities (operation_id, entity_type, entity_id)
         VALUES ('op-2', 'title', 'contested-title')",
    )
    .execute(&pool)
    .await
    .expect("claim after release should succeed");
}

#[tokio::test]
async fn deleting_an_operation_takes_its_checkpoints_and_records_with_it() {
    let pool = pre_upgrade_pool().await;
    apply_upgrade(&pool).await;

    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("foreign keys should enable");
    sqlx::query(
        "INSERT INTO location_operations (id, operation_type, execution_mode, state)
         VALUES ('op-1', 'adoption', 'adopt', 'completed')",
    )
    .execute(&pool)
    .await
    .expect("operation row should insert");
    sqlx::query(
        "INSERT INTO location_operation_title_checkpoints (operation_id, title_id, state)
         VALUES ('op-1', 'title-1', 'committed')",
    )
    .execute(&pool)
    .await
    .expect("checkpoint row should insert");

    sqlx::query("DELETE FROM location_operations WHERE id = 'op-1'")
        .execute(&pool)
        .await
        .expect("operation should delete");
    let remaining: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM location_operation_title_checkpoints")
            .fetch_one(&pool)
            .await
            .expect("checkpoint count should load");
    assert_eq!(remaining, 0);
}

// ── PostgreSQL ──────────────────────────────────────────────────────────────
//
// The same scenarios against the other engine, because 0210-0212 ship separate
// PostgreSQL SQL files (timestamptz/boolean rather than TEXT/INTEGER) and the
// 0210 hook has a separate `$1`-placeholder implementation. Skips when
// `SCRYER_TEST_POSTGRES_URL` is unset, matching the convention in
// `postgres::services`. Each run works inside its own schema and drops it.

fn postgres_test_url() -> Option<String> {
    std::env::var("SCRYER_TEST_POSTGRES_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[tokio::test]
async fn postgres_upgrade_rekeys_roots_and_creates_the_location_tables() {
    let Some(url) = postgres_test_url() else {
        eprintln!(
            "skipping PostgreSQL synthetic-root-id upgrade test; SCRYER_TEST_POSTGRES_URL is not set"
        );
        return;
    };
    let admin = sqlx::PgPool::connect(&url)
        .await
        .expect("postgres should connect");
    let schema = format!("synthetic_root_ids_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin)
        .await
        .expect("test schema should create");

    let scoped_url = if url.contains('?') {
        format!("{url}&options=-c%20search_path%3D{schema}")
    } else {
        format!("{url}?options=-c%20search_path%3D{schema}")
    };
    let pool = sqlx::PgPool::connect(&scoped_url)
        .await
        .expect("scoped postgres pool should connect");

    let outcome = async {
        crate::postgres::replay_source_catalog_for_fresh_install(&pool, Some(PRE_UPGRADE_VERSION))
            .await
            .expect("pre-upgrade postgres fixture should apply");

        let legacy_id = path_derived_id("/mnt/archive/movies");
        sqlx::query(
            "INSERT INTO library_roots
                 (id, library_id, path, normalized_path, is_default, created_at, updated_at)
             VALUES ($1, 'movie_default_library', '/mnt/archive/movies', $2, false,
                     '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(&legacy_id)
        .bind(normalize_library_root_path("/mnt/archive/movies"))
        .execute(&pool)
        .await
        .expect("path-derived root should insert");
        sqlx::query(
            "INSERT INTO titles
                 (id, library_id, name, facet, monitored, root_folder_id, created_at, updated_at)
             VALUES ('pg-title-1', 'movie_default_library', 'PG Title', 'movie', true, $1,
                     '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(&legacy_id)
        .execute(&pool)
        .await
        .expect("title should insert");

        // Upgrade through the real service bootstrap, the same path production
        // takes on boot, rather than reaching into the migration runner.
        let services = crate::postgres::PostgresServices::new_with_mode(
            &scoped_url,
            crate::MigrationMode::Apply,
        )
        .await
        .expect("synthetic-root-id postgres upgrade should apply");
        drop(services);

        let expected_id = synthetic_root_id_from_legacy_id(&legacy_id);
        let moved: String =
            sqlx::query_scalar("SELECT root_folder_id FROM titles WHERE id = 'pg-title-1'")
                .fetch_one(&pool)
                .await
                .expect("title should load");
        assert_eq!(moved, expected_id);

        let retained: String =
            sqlx::query_scalar("SELECT legacy_path_derived_id FROM library_roots WHERE id = $1")
                .bind(&expected_id)
                .fetch_one(&pool)
                .await
                .expect("legacy id column should load");
        assert_eq!(retained, legacy_id);

        let mapped: String = sqlx::query_scalar(
            "SELECT root_id FROM library_root_id_remaps WHERE legacy_root_id = $1",
        )
        .bind(&legacy_id)
        .fetch_one(&pool)
        .await
        .expect("remap row should exist");
        assert_eq!(mapped, expected_id);

        // The seeded canonical root keeps its id on this engine too.
        let seeded: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM library_roots
              WHERE id = 'canonical_root_for_movie_default_library'",
        )
        .fetch_one(&pool)
        .await
        .expect("seeded root count should load");
        assert_eq!(seeded, 1);

        // 0211 columns exist and are nullable.
        let hashed: Option<String> =
            sqlx::query_scalar("SELECT full_blake3 FROM media_files LIMIT 1")
                .fetch_optional(&pool)
                .await
                .expect("full_blake3 column should exist")
                .flatten();
        assert_eq!(hashed, None);

        // 0212 tables exist, including the single-owner registry index.
        sqlx::query(
            "INSERT INTO location_operations (id, operation_type, execution_mode, state)
             VALUES ('pg-op-1', 'root_change', 'move', 'running'),
                    ('pg-op-2', 'root_change', 'move', 'running')",
        )
        .execute(&pool)
        .await
        .expect("operation rows should insert");
        sqlx::query(
            "INSERT INTO location_operation_owned_entities (operation_id, entity_type, entity_id)
             VALUES ('pg-op-1', 'root', $1)",
        )
        .bind(&expected_id)
        .execute(&pool)
        .await
        .expect("first claim should succeed");
        let contested = sqlx::query(
            "INSERT INTO location_operation_owned_entities (operation_id, entity_type, entity_id)
             VALUES ('pg-op-2', 'root', $1)",
        )
        .bind(&expected_id)
        .execute(&pool)
        .await;
        assert!(
            contested.is_err(),
            "a second live claim on the same root must be rejected by the registry"
        );

        // The columns the amended 0212 added are on this engine too, with the
        // same names — the store issues one statement for both engines.
        sqlx::query(
            "INSERT INTO location_operation_title_checkpoints
                 (operation_id, title_id, state, note)
             VALUES ('pg-op-1', 'pg-title-1', 'completed_with_warnings',
                     'one companion asset was renamed')",
        )
        .execute(&pool)
        .await
        .expect("checkpoint row with a note should insert");
        sqlx::query(
            "INSERT INTO location_operation_verifications
                 (id, operation_id, title_id, source_path, destination_path,
                  requested_depth, applied_depth, outcome, detail)
             VALUES ('pg-ver-1', 'pg-op-1', 'pg-title-1', '/src/a.mkv', '/dst/a.mkv',
                     'full', 'full', 'verified', 'verified (full)')",
        )
        .execute(&pool)
        .await
        .expect("verification row with a detail should insert");
        // `integer` columns on this engine, so the sum comes back as an i32.
        let counters: i32 = sqlx::query_scalar(
            "SELECT merge_count + dedup_count + rename_count + no_op_count + unresolved_count
               FROM location_operations WHERE id = 'pg-op-1'",
        )
        .fetch_one(&pool)
        .await
        .expect("counters should load");
        assert_eq!(counters, 0, "the outcome counters default to zero");
    }
    .await;

    pool.close().await;
    sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&admin)
        .await
        .expect("test schema should drop");
    outcome
}

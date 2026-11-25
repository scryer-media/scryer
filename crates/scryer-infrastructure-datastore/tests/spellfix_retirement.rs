//! A database that still carries the retired `spellfix1` virtual table must
//! survive an upgrade, on a build that has no `spellfix1` module at all.
//!
//! The fixture here is the real hazard, not an approximation of it: a
//! `sqlite_master` row describing a virtual table whose module is gone. Every
//! statement that touches such an object fails with `no such module`,
//! including `DROP TABLE`, and the already-checksummed migrations 0092 and
//! 0236 still name the table. The retirement step rewrites the schema under
//! `PRAGMA writable_schema` and leaves a plain stand-in behind, and this
//! asserts both halves plus the integrity of the resulting schema.

// disk-guard: test, example and build-script code may call the filesystem directly.
#![allow(clippy::disallowed_methods)]

use scryer_infrastructure_datastore::migrations::{
    embedded_catalog, replay_source_catalog_for_fresh_install, run_migrations,
};
use scryer_infrastructure_datastore::{MigrationMode, retire_spellfix_virtual_table};
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};

/// The migration ceiling of a 0.21.7 install: the last version that shipped
/// before `spellfix.c` was removed.
const LEGACY_MIGRATION_CEILING: i64 = 249;

async fn pool() -> SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory sqlite should open")
}

/// A file-backed pool, opened the way the application opens one. In-memory
/// SQLite gives every pooled connection its *own* database, so anything about
/// more than one connection, or about a schema surviving a reopen, has to run
/// against a file.
async fn file_pool(path: &std::path::Path, max_connections: u32) -> SqlitePool {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(std::time::Duration::from_secs(10));
    SqlitePoolOptions::new()
        .max_connections(max_connections)
        .connect_with(options)
        .await
        .expect("file-backed sqlite should open")
}

/// Write the schema row by hand, which is the only way to produce a virtual
/// table whose module this binary cannot load.
async fn install_orphaned_virtual_table(pool: &SqlitePool) {
    sqlx::raw_sql(
        "PRAGMA writable_schema = ON;
         INSERT INTO sqlite_master (type, name, tbl_name, rootpage, sql)
         VALUES ('table', 'title_search_spellfix', 'title_search_spellfix', 0,
                 'CREATE VIRTUAL TABLE title_search_spellfix USING spellfix1');
         PRAGMA writable_schema = RESET;",
    )
    .execute(pool)
    .await
    .expect("the schema row should be writable");

    // The shadow table the module used to own. It is an ordinary table, so it
    // outlives the module and has to be cleaned up too.
    sqlx::query("CREATE TABLE title_search_spellfix_vocab (id INTEGER PRIMARY KEY NOT NULL)")
        .execute(pool)
        .await
        .expect("the shadow table should be created");
}

async fn object_sql(pool: &SqlitePool, name: &str) -> Option<String> {
    sqlx::query_scalar::<_, String>("SELECT COALESCE(sql, '') FROM sqlite_master WHERE name = ?1")
        .bind(name)
        .fetch_optional(pool)
        .await
        .expect("sqlite_master should be readable")
}

#[tokio::test]
async fn retirement_removes_the_module_less_virtual_table_and_lets_old_migrations_replay() {
    let pool = pool().await;
    install_orphaned_virtual_table(&pool).await;

    // The fixture is only worth anything if it really is unusable first.
    let error = sqlx::query("SELECT COUNT(*) FROM title_search_spellfix")
        .fetch_one(&pool)
        .await
        .expect_err("a virtual table without its module must not be queryable");
    assert!(
        error.to_string().contains("no such module"),
        "unexpected failure: {error}"
    );

    retire_spellfix_virtual_table(&pool)
        .await
        .expect("retirement must succeed");

    let sql = object_sql(&pool, "title_search_spellfix")
        .await
        .expect("the stand-in table should exist");
    assert!(
        !sql.to_ascii_lowercase().contains("spellfix1"),
        "the virtual table must be gone, found: {sql}"
    );
    assert!(
        object_sql(&pool, "title_search_spellfix_vocab")
            .await
            .is_none(),
        "the shadow table must be gone"
    );

    let integrity = sqlx::query_scalar::<_, String>("PRAGMA integrity_check")
        .fetch_one(&pool)
        .await
        .expect("integrity_check should run");
    assert_eq!(
        integrity, "ok",
        "rewriting the schema must leave the database intact"
    );

    // What migrations 0092 and 0236 do, verbatim in shape: they are already
    // applied and checksummed, so they still have to run against the stand-in.
    sqlx::raw_sql(
        "CREATE VIRTUAL TABLE IF NOT EXISTS title_search_spellfix USING spellfix1;
         DELETE FROM title_search_spellfix;
         INSERT INTO title_search_spellfix (rowid, word, rank, langid)
         VALUES (1, 'example', 1, 0);
         DELETE FROM title_search_spellfix WHERE rowid = 1;",
    )
    .execute(&pool)
    .await
    .expect("the pre-removal migrations must replay against the stand-in");

    // Idempotent: a second pass changes nothing and still succeeds.
    retire_spellfix_virtual_table(&pool)
        .await
        .expect("retirement must be repeatable");
    assert!(
        object_sql(&pool, "title_search_spellfix").await.is_some(),
        "the stand-in stays until migration 0252 drops it"
    );
}

#[tokio::test]
async fn retirement_is_a_no_op_on_a_database_that_never_had_the_table() {
    let pool = pool().await;
    sqlx::raw_sql(
        "CREATE TABLE _sqlx_migrations (version BIGINT PRIMARY KEY NOT NULL);
         INSERT INTO _sqlx_migrations (version) VALUES (252);",
    )
    .execute(&pool)
    .await
    .unwrap();

    retire_spellfix_virtual_table(&pool)
        .await
        .expect("retirement must succeed");

    assert!(
        object_sql(&pool, "title_search_spellfix").await.is_none(),
        "a database already past 0252 must not get the stand-in back"
    );
}

#[tokio::test]
async fn a_fresh_install_carries_no_spellfix_objects_and_has_the_queue() {
    let pool = pool().await;
    scryer_infrastructure_datastore::migrations::replay_source_catalog_for_fresh_install(
        &pool, None, true,
    )
    .await
    .expect("fresh migrations should apply");

    let leftovers: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE name LIKE 'title_search_spellfix%'
         OR name = 'title_search_terms_delete_spellfix' ORDER BY name",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(
        leftovers.is_empty(),
        "migration 0252 must leave nothing behind: {leftovers:?}"
    );

    let queue: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM title_search_index_queue")
        .fetch_one(&pool)
        .await
        .expect("the fuzzy index queue must exist");
    assert_eq!(queue, 0);
}

// ---------------------------------------------------------------------------
// The real upgrade, over a database shaped like a 0.21.7 install.
// ---------------------------------------------------------------------------

/// Replay the real catalog to the 0.21.7 ceiling and then put the legacy
/// spellfix shape back by hand.
///
/// The replay cannot produce that shape itself: this build has no `spellfix1`
/// module, so the retirement step that runs ahead of the replay leaves the
/// plain stand-in where migration 0092 would have created a virtual table.
/// Swapping the stand-in for a `sqlite_master` row naming the missing module,
/// plus the ordinary shadow table the module used to own, is the only way to
/// reproduce what is actually on disk in an existing install.
async fn legacy_0_21_7_database(pool: &SqlitePool) {
    replay_source_catalog_for_fresh_install(pool, Some(LEGACY_MIGRATION_CEILING), true)
        .await
        .expect("the pre-upgrade catalog should replay");

    sqlx::query("DROP TABLE IF EXISTS title_search_spellfix")
        .execute(pool)
        .await
        .expect("the stand-in should be droppable");

    sqlx::raw_sql(
        "PRAGMA writable_schema = ON;
         INSERT INTO sqlite_master (type, name, tbl_name, rootpage, sql)
         VALUES ('table', 'title_search_spellfix', 'title_search_spellfix', 0,
                 'CREATE VIRTUAL TABLE title_search_spellfix USING spellfix1');
         PRAGMA writable_schema = RESET;",
    )
    .execute(pool)
    .await
    .expect("the schema row should be writable");

    // The shadow table spellfix1 owned, with the index the module created on
    // it. Both are ordinary objects, so both outlive the module.
    sqlx::raw_sql(
        "CREATE TABLE title_search_spellfix_vocab (
             id INTEGER PRIMARY KEY,
             rank INT,
             langid INT,
             word TEXT,
             k1 TEXT,
             k2 TEXT
         );
         CREATE INDEX title_search_spellfix_vocab_index_1
             ON title_search_spellfix_vocab(langid, k2);",
    )
    .execute(pool)
    .await
    .expect("the shadow table should be created");

    assert!(
        object_sql(pool, "title_search_terms_delete_spellfix")
            .await
            .is_some(),
        "migration 0236 should have left its trigger behind"
    );

    seed_title_search_terms(pool).await;
}

/// A few hundred projection rows across several titles, so migration 0250's
/// `DELETE FROM title_search_terms` fires the 0236 trigger once per row.
async fn seed_title_search_terms(pool: &SqlitePool) {
    // The baseline seeds the default libraries and their canonical roots, and
    // `titles` has a trigger demanding a root_folder_id, so the parents come
    // from the catalog rather than from hand-written rows.
    let root_folder_id: String = sqlx::query_scalar(
        "SELECT id FROM library_roots
          WHERE library_id = 'movie_default_library'
          ORDER BY is_default DESC, id
          LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("the baseline should have seeded a canonical movie root");

    sqlx::query(
        "WITH RECURSIVE counter(n) AS (
             SELECT 0 UNION ALL SELECT n + 1 FROM counter WHERE n < 5
         )
         INSERT INTO titles
             (id, name, name_normalized, facet, created_at, library_id, root_folder_id)
         SELECT 'synthetic-title-' || n,
                'Synthetic Title ' || n,
                'synthetic title ' || n,
                'movie',
                '2020-01-01T00:00:00Z',
                'movie_default_library',
                ?1
         FROM counter",
    )
    .bind(&root_folder_id)
    .execute(pool)
    .await
    .expect("the legacy title fixture should insert");

    sqlx::raw_sql(
        "WITH RECURSIVE counter(n) AS (
             SELECT 1 UNION ALL SELECT n + 1 FROM counter WHERE n < 600
         )
         INSERT INTO title_search_terms
             (term_id, title_id, facet, term_kind, raw_term, normalized_term, weight)
         SELECT n,
                'synthetic-title-' || (n % 6),
                'movie',
                'primary',
                'Synthetic Term ' || n,
                'synthetic term ' || n,
                1
         FROM counter;",
    )
    .execute(pool)
    .await
    .expect("the legacy projection fixture should insert");

    let terms: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM title_search_terms")
        .fetch_one(pool)
        .await
        .expect("the projection should be readable");
    assert_eq!(terms, 600, "the fixture must have rows for 0250 to delete");
}

async fn spellfix_leftovers(pool: &SqlitePool) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT name FROM sqlite_master
          WHERE name LIKE 'title_search_spellfix%'
             OR name = 'title_search_terms_delete_spellfix'
          ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .expect("sqlite_master should be readable")
}

/// Everything the upgrade is supposed to be true of afterwards, asserted on a
/// database that has just been through it.
async fn assert_upgraded_end_state(pool: &SqlitePool) {
    let catalog = embedded_catalog().expect("the embedded catalog should decode");
    let max_version = catalog.max_version();
    assert!(
        max_version >= 254,
        "this test assumes the catalog reaches at least 0254, found {max_version}"
    );

    for migration in &catalog.migrations {
        let success: Option<i64> =
            sqlx::query_scalar("SELECT success FROM _sqlx_migrations WHERE version = ?1")
                .bind(migration.version)
                .fetch_optional(pool)
                .await
                .expect("the ledger should be readable");
        assert_eq!(
            success,
            Some(1),
            "migration {} must be recorded as successfully applied",
            migration.key
        );
    }

    let leftovers = spellfix_leftovers(pool).await;
    assert!(
        leftovers.is_empty(),
        "no spellfix object or trigger may survive the upgrade: {leftovers:?}"
    );

    assert!(
        object_sql(pool, "titles_delete_enqueue_fuzzy_index")
            .await
            .is_some(),
        "the fuzzy-index enqueue trigger must replace the spellfix trigger"
    );
    let queued: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM title_search_index_queue")
        .fetch_one(pool)
        .await
        .expect("the fuzzy index queue must exist");
    assert_eq!(queued, 0);

    // 0250 empties the projection because its new columns cannot be computed
    // in SQL; the application reseeds it on the next start.
    let terms: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM title_search_terms")
        .fetch_one(pool)
        .await
        .expect("the projection table must survive");
    assert_eq!(terms, 0, "migration 0250 must have emptied the projection");

    // The titles themselves are untouched, and the per-row trigger fired
    // without the module being present.
    let titles: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM titles")
        .fetch_one(pool)
        .await
        .expect("titles must survive");
    assert_eq!(titles, 6, "the upgrade must not touch the catalog itself");

    let integrity = sqlx::query_scalar::<_, String>("PRAGMA integrity_check")
        .fetch_one(pool)
        .await
        .expect("integrity_check should run");
    assert_eq!(integrity, "ok", "the rewritten schema must be intact");

    let foreign_key_violations: Vec<(String,)> = sqlx::query_as("PRAGMA foreign_key_check")
        .fetch_all(pool)
        .await
        .expect("foreign_key_check should run");
    assert!(
        foreign_key_violations.is_empty(),
        "the upgrade must leave no dangling references: {:?}",
        foreign_key_violations
            .iter()
            .map(|row| row.0.clone())
            .collect::<Vec<_>>()
    );

    // Both of these fail outright while an orphaned virtual table is still in
    // the schema, which is why they are the end-state proof.
    sqlx::query("VACUUM")
        .execute(pool)
        .await
        .expect("VACUUM must succeed once the orphaned virtual table is gone");
}

#[tokio::test]
async fn a_0_21_7_database_upgrades_with_the_spellfix1_module_missing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pool = file_pool(&dir.path().join("scryer.db"), 1).await;
    legacy_0_21_7_database(&pool).await;

    // The fixture is only worth anything if it really is the hazard first.
    let error = sqlx::query("SELECT COUNT(*) FROM title_search_spellfix")
        .fetch_one(&pool)
        .await
        .expect_err("a virtual table without its module must not be queryable");
    assert!(
        error.to_string().contains("no such module"),
        "unexpected failure: {error}"
    );

    run_migrations(&pool, MigrationMode::Apply)
        .await
        .expect("the upgrade must apply");
    assert_upgraded_end_state(&pool).await;

    // Idempotent: the second pass has nothing pending and changes nothing.
    let ledger_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .expect("the ledger should be readable");
    run_migrations(&pool, MigrationMode::Apply)
        .await
        .expect("a second upgrade pass must be a no-op");
    let ledger_after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .expect("the ledger should be readable");
    assert_eq!(ledger_before, ledger_after);
    assert!(spellfix_leftovers(&pool).await.is_empty());
}

#[tokio::test]
async fn an_upgrade_interrupted_between_the_schema_delete_and_the_stand_in_recovers() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pool = file_pool(&dir.path().join("scryer.db"), 1).await;
    legacy_0_21_7_database(&pool).await;

    // The crash window: the schema row is gone, but the process died before
    // the shadow table was dropped and before the stand-in was installed.
    sqlx::raw_sql(
        "PRAGMA writable_schema = ON;
         DELETE FROM sqlite_master
          WHERE name = 'title_search_spellfix' OR tbl_name = 'title_search_spellfix';
         PRAGMA writable_schema = RESET;",
    )
    .execute(&pool)
    .await
    .expect("the schema row should be deletable");
    assert!(
        object_sql(&pool, "title_search_spellfix").await.is_none(),
        "the fixture must start from a deleted schema row"
    );
    assert!(
        object_sql(&pool, "title_search_spellfix_vocab")
            .await
            .is_some(),
        "the fixture must start with the shadow table still present"
    );

    run_migrations(&pool, MigrationMode::Apply)
        .await
        .expect("the resumed upgrade must apply");
    assert_upgraded_end_state(&pool).await;
}

// ---------------------------------------------------------------------------
// Other pooled connections must not be left on a stale schema.
// ---------------------------------------------------------------------------

/// The full retirement, with a second connection held across it.
///
/// This path happens to be safe even without an explicit cookie bump, because
/// the shadow-table `DROP` and the stand-in `CREATE` that follow the schema
/// rewrite are ordinary DDL and bump the cookie themselves. It is pinned so
/// that a future reordering cannot quietly remove that accident.
#[tokio::test]
async fn a_held_pooled_connection_sees_the_retirement() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pool = file_pool(&dir.path().join("scryer.db"), 4).await;

    sqlx::raw_sql(
        "CREATE TABLE title_probe (id INTEGER PRIMARY KEY NOT NULL, name TEXT NOT NULL);
         INSERT INTO title_probe (id, name) VALUES (1, 'synthetic probe');",
    )
    .execute(&pool)
    .await
    .expect("the probe table should be created");
    install_orphaned_virtual_table(&pool).await;

    // A second connection, checked out and kept, with the pre-retirement
    // schema loaded into its cache.
    let mut observer = pool
        .acquire()
        .await
        .expect("a second connection should be available");
    let probed: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM title_probe")
        .fetch_one(&mut *observer)
        .await
        .expect("the observer should read its own schema");
    assert_eq!(probed, 1);
    let stale = sqlx::query("SELECT COUNT(*) FROM title_search_spellfix")
        .fetch_one(&mut *observer)
        .await
        .expect_err("the observer must start out unable to use the virtual table");
    assert!(
        stale.to_string().contains("no such module"),
        "unexpected failure: {stale}"
    );

    retire_spellfix_virtual_table(&pool)
        .await
        .expect("retirement must succeed");

    // `sqlite_master` is read live, so the observer always sees the new row.
    let names: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE name LIKE 'title_search_spellfix%' ORDER BY name",
    )
    .fetch_all(&mut *observer)
    .await
    .expect("sqlite_master should be readable from the observer");
    assert_eq!(names, vec!["title_search_spellfix".to_string()]);

    // The cached schema is what matters: this is migration 0236's statement,
    // and it resolves against whatever definition the observer holds.
    sqlx::query("DELETE FROM title_search_spellfix")
        .execute(&mut *observer)
        .await
        .expect("the held connection must resolve the stand-in, not the retired module");
    sqlx::query("INSERT INTO title_search_spellfix (rowid, word, rank, langid) VALUES (1, 'synthetic', 1, 0)")
        .execute(&mut *observer)
        .await
        .expect("the held connection must be able to write the stand-in");

    // A statement that never mentioned spellfix keeps working either way.
    let probed: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM title_probe")
        .fetch_one(&mut *observer)
        .await
        .expect("unrelated queries must keep working");
    assert_eq!(probed, 1);
}

/// The schema rewrite on its own, with nothing after it to bump the cookie.
///
/// A `DELETE FROM sqlite_master` under `writable_schema` does not change the
/// schema cookie, so every other connection in the pool keeps the schema it
/// cached and goes on resolving `title_search_spellfix` to a definition that
/// is no longer in the file. SQLite's own recipe for writable_schema edits
/// sets `PRAGMA schema_version` for exactly this reason.
///
/// A database already past 0252 that still carries the orphaned row reaches
/// this window with nothing behind it: the shadow table is gone, so the `DROP`
/// is a no-op, and the stand-in is not installed. Without the cookie bump the
/// held connection is stranded, which is what this asserts against.
#[tokio::test]
async fn the_schema_rewrite_alone_does_not_strand_another_pooled_connection() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pool = file_pool(&dir.path().join("scryer.db"), 4).await;

    sqlx::raw_sql(
        "CREATE TABLE title_probe (id INTEGER PRIMARY KEY NOT NULL, name TEXT NOT NULL);
         INSERT INTO title_probe (id, name) VALUES (1, 'synthetic probe');
         CREATE TABLE _sqlx_migrations (version BIGINT PRIMARY KEY NOT NULL);
         INSERT INTO _sqlx_migrations (version) VALUES (252);
         PRAGMA writable_schema = ON;
         INSERT INTO sqlite_master (type, name, tbl_name, rootpage, sql)
         VALUES ('table', 'title_search_spellfix', 'title_search_spellfix', 0,
                 'CREATE VIRTUAL TABLE title_search_spellfix USING spellfix1');
         PRAGMA writable_schema = RESET;",
    )
    .execute(&pool)
    .await
    .expect("the fixture should be writable");

    // Held across the retirement, with the pre-retirement schema cached.
    let mut observer = pool
        .acquire()
        .await
        .expect("a second connection should be available");
    let stale = sqlx::query("SELECT COUNT(*) FROM title_search_spellfix")
        .fetch_one(&mut *observer)
        .await
        .expect_err("the observer must start out unable to use the virtual table");
    assert!(
        stale.to_string().contains("no such module"),
        "unexpected failure: {stale}"
    );

    retire_spellfix_virtual_table(&pool)
        .await
        .expect("retirement must succeed");
    assert!(
        object_sql(&pool, "title_search_spellfix").await.is_none(),
        "a database past 0252 gets no stand-in back"
    );

    // The observer reads `sqlite_master` live, so it agrees the row is gone …
    let names: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE name LIKE 'title_search_spellfix%' ORDER BY name",
    )
    .fetch_all(&mut *observer)
    .await
    .expect("sqlite_master should be readable from the observer");
    assert!(
        names.is_empty(),
        "the observer should see an empty schema table: {names:?}"
    );

    // … but name resolution runs against its cached schema, which is the part
    // the cookie bump fixes. Without it this fails with "table
    // title_search_spellfix already exists".
    sqlx::query(
        "CREATE TABLE title_search_spellfix (
             rowid INTEGER PRIMARY KEY NOT NULL,
             word TEXT,
             rank INTEGER,
             langid INTEGER
         )",
    )
    .execute(&mut *observer)
    .await
    .expect("the held connection must not still believe the retired table exists");
    sqlx::query("DELETE FROM title_search_spellfix")
        .execute(&mut *observer)
        .await
        .expect("the held connection must resolve the table it just created");

    let probed: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM title_probe")
        .fetch_one(&mut *observer)
        .await
        .expect("unrelated queries must keep working");
    assert_eq!(probed, 1);
}

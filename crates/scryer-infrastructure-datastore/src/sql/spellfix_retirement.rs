//! Retiring the `spellfix1` virtual table, and letting the migrations that
//! predate its removal still replay.
//!
//! The fuzzy title lane is a tantivy index now, so the vendored `spellfix.c`
//! is gone and no SQLite connection this process opens can instantiate a
//! `spellfix1` module. Two consequences have to be handled here rather than
//! in a migration file, because they happen *before* the migration that
//! removes the table could run:
//!
//! 1. **An existing database still holds the virtual table.** Its definition
//!    lives in `sqlite_master`, and any statement that touches it — including
//!    the `DELETE FROM title_search_spellfix` in migration 0236, a `VACUUM`,
//!    or a backup's `integrity_check` — fails with `no such module: spellfix1`.
//!    `DROP TABLE` fails for the same reason, so the row is removed by
//!    rewriting the schema under `PRAGMA writable_schema`, which is the one
//!    supported way to delete an object whose module is unavailable.
//!
//! 2. **Migrations 0092 and 0236 are already applied and checksummed.**
//!    Editing them would fail every existing installation's checksum
//!    verification, so they still contain their `title_search_spellfix`
//!    statements: 0092's `CREATE VIRTUAL TABLE`, and 0236's repair `DELETE`
//!    plus the `title_search_terms_delete_spellfix` trigger whose body names
//!    the table. A plain table of that name satisfies all of them —
//!    `CREATE VIRTUAL TABLE IF NOT EXISTS` becomes a no-op when the name is
//!    taken, and the `DELETE`s and the trigger work against an ordinary table
//!    — so this installs one as a stand-in for exactly as long as the replay
//!    needs it. Migration 0252 drops it together with the 0236 trigger.
//!
//!    The baselines are a different case: they are snapshots the ledger does
//!    not checksum, only the per-version migrations are, so the
//!    `CREATE VIRTUAL TABLE title_search_spellfix USING spellfix1;` line was
//!    simply removed from `0140_baseline.sql` and `0198_baseline.sql` in this
//!    release. A fresh install that starts from a baseline still needs the
//!    stand-in anyway, because the migrations above the baseline replay on top
//!    of it and 0236 is one of them.
//!
//! Both steps are idempotent and both are no-ops on a database that has
//! already passed 0252.

use scryer_application::{AppError, AppResult};
use sqlx::{Row, SqliteConnection, SqlitePool};

/// The migration that drops the stand-in table and the 0236 trigger. A
/// database at or past this version needs neither step below.
const SPELLFIX_REMOVAL_VERSION: i64 = 252;

const SPELLFIX_TABLE: &str = "title_search_spellfix";

fn repo_err(error: impl std::fmt::Display) -> AppError {
    AppError::Repository(error.to_string())
}

/// Remove any `spellfix1` virtual table, then install the plain stand-in the
/// pre-removal migrations need, if they still have to run.
pub async fn retire_spellfix_virtual_table(pool: &SqlitePool) -> AppResult<()> {
    let mut connection = pool.acquire().await.map_err(repo_err)?;
    retire_spellfix_virtual_table_on(&mut connection).await
}

/// [`retire_spellfix_virtual_table`] on a connection the caller already holds.
/// The migration runner applies every pending migration on one connection,
/// and this has to run on that same connection so the schema it leaves behind
/// is the schema the migrations after it parse against.
pub(crate) async fn retire_spellfix_virtual_table_on(
    connection: &mut SqliteConnection,
) -> AppResult<()> {
    if let Some(sql) = spellfix_object_sql(connection).await?
        && sql.to_ascii_lowercase().contains("using spellfix1")
    {
        drop_virtual_table_via_writable_schema(connection).await?;
    }

    if applied_migration_ceiling(connection).await? < SPELLFIX_REMOVAL_VERSION
        && spellfix_object_sql(connection).await?.is_none()
    {
        install_stand_in_table(connection).await?;
    }
    Ok(())
}

async fn spellfix_object_sql(connection: &mut SqliteConnection) -> AppResult<Option<String>> {
    let row = sqlx::query("SELECT COALESCE(sql, '') AS sql FROM sqlite_master WHERE name = ?1")
        .bind(SPELLFIX_TABLE)
        .fetch_optional(&mut *connection)
        .await
        .map_err(repo_err)?;
    row.map(|row| row.try_get::<String, _>("sql").map_err(repo_err))
        .transpose()
}

/// The highest migration version recorded as applied, or 0 when the ledger
/// does not exist yet (a fresh database, which replays everything above the
/// baseline it starts from).
async fn applied_migration_ceiling(connection: &mut SqliteConnection) -> AppResult<i64> {
    let ledger_exists = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations'",
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(repo_err)?;
    if ledger_exists == 0 {
        return Ok(0);
    }
    sqlx::query_scalar::<_, Option<i64>>("SELECT MAX(version) FROM _sqlx_migrations")
        .fetch_one(&mut *connection)
        .await
        .map(|value| value.unwrap_or(0))
        .map_err(repo_err)
}

/// The only way to delete an object whose module is missing.
///
/// `writable_schema` is left on for the shortest possible window and is
/// always turned off again, including on the error path: a connection that
/// keeps it on would let any later statement corrupt the schema. `RESET`
/// rather than `OFF` so the schema cache is reloaded immediately instead of
/// at the next connection.
///
/// `RESET` only reloads *this* connection. Deleting a row from `sqlite_master`
/// is not DDL, so SQLite does not bump the schema cookie for it, and every
/// other connection in the pool would keep the schema it had already cached
/// and go on resolving `title_search_spellfix` to a virtual table whose module
/// is gone. SQLite's own recipe for `writable_schema` edits sets
/// `PRAGMA schema_version` for exactly this reason, and that is done here
/// inside the same write transaction as the delete, so a crash leaves either
/// both or neither.
async fn drop_virtual_table_via_writable_schema(
    connection: &mut SqliteConnection,
) -> AppResult<()> {
    let result = async {
        sqlx::query("PRAGMA writable_schema = ON")
            .execute(&mut *connection)
            .await
            .map_err(repo_err)?;
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut *connection)
            .await
            .map_err(repo_err)?;

        // Everything between BEGIN and COMMIT, the COMMIT included. A COMMIT
        // that fails — SQLITE_BUSY, an I/O error — leaves the transaction open
        // exactly as an earlier failure does, so it has to take the same exit.
        let transaction = async {
            let schema_version = sqlx::query_scalar::<_, i64>("PRAGMA schema_version")
                .fetch_one(&mut *connection)
                .await
                .map_err(repo_err)?;
            sqlx::query("DELETE FROM sqlite_master WHERE name = ?1 OR tbl_name = ?1")
                .bind(SPELLFIX_TABLE)
                .execute(&mut *connection)
                .await
                .map_err(repo_err)?;
            // PRAGMA arguments cannot be bound, and this one is an integer
            // this function computed, so there is nothing to inject.
            let next = next_schema_version(schema_version);
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "PRAGMA schema_version = {next}"
            )))
            .execute(&mut *connection)
            .await
            .map_err(repo_err)?;
            sqlx::query("COMMIT")
                .execute(&mut *connection)
                .await
                .map_err(repo_err)?;
            Ok::<(), AppError>(())
        }
        .await;

        // One exit for every failure above. Without it the connection could
        // go back to the pool still holding the IMMEDIATE write lock, and the
        // next statement to land on it would run inside that abandoned
        // transaction while every other writer blocked behind the lock. A
        // rollback that fails in turn — because SQLite already unwound the
        // transaction itself — adds nothing to the original error.
        if let Err(error) = transaction {
            let _ = sqlx::query("ROLLBACK").execute(&mut *connection).await;
            return Err(error);
        }
        Ok(())
    }
    .await;

    // RESET runs whatever happened above, and its own failure must not mask
    // the original one. `result?` also returns before the vocab DROP below,
    // so a failed rewrite never gets to drop the shadow table.
    let reset = sqlx::query("PRAGMA writable_schema = RESET")
        .execute(&mut *connection)
        .await
        .map_err(repo_err);
    result?;
    reset?;

    sqlx::query("DROP TABLE IF EXISTS title_search_spellfix_vocab")
        .execute(&mut *connection)
        .await
        .map_err(repo_err)?;

    tracing::info!("removed the retired spellfix1 virtual table from the schema");
    Ok(())
}

/// Any value other than the current one makes every other connection reload
/// its schema, so the only thing that matters is that this differs. The header
/// field is a 32-bit counter, so it is allowed to wrap; outside the ordinary
/// range it restarts at 1 rather than being pushed past what the field holds.
fn next_schema_version(current: i64) -> i64 {
    if current <= 0 || current >= u32::MAX as i64 {
        1
    } else {
        current + 1
    }
}

/// The columns the pre-removal migrations reference: `rowid` for the 0236
/// trigger and its repair `DELETE`, and `word` because the old typo lane
/// selected it. Nothing reads this table any more; it exists only so the
/// replay of already-applied SQL does not need a module that is gone.
async fn install_stand_in_table(connection: &mut SqliteConnection) -> AppResult<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS title_search_spellfix (
             rowid INTEGER PRIMARY KEY NOT NULL,
             word TEXT,
             rank INTEGER,
             langid INTEGER
         )",
    )
    .execute(&mut *connection)
    .await
    .map_err(repo_err)?;
    Ok(())
}

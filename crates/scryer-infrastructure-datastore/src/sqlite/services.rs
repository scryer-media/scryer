use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use scryer_application::{AppError, AppResult};
use sqlx::ConnectOptions;

use crate::encryption::{EncryptionKey, load_existing_sqlite_migration_encryption_key};
use crate::migrations::{MigrationHookContext, MigrationProgress};
use crate::storage::sql::runtime::{SqlRuntime, StoreDatastore, repo_err};
use crate::storage::sqlite::writer::{SqliteWriterGate, new_writer_gate};
use crate::types::MigrationMode;

const DEFAULT_SQLITE_MAX_CONNECTIONS: u32 = 16;
const MAX_SQLITE_CONNECTIONS_CAP: u32 = 64;
const SQLITE_SLOW_STATEMENT_MS: u64 = 2000;
/// SQLite's built-in default is 1000 pages, which under a library scan's write
/// rate (~1100 WAL pages/s) checkpoints roughly once a second and rewrites the
/// same hot btree pages over and over. 16k pages (~64 MB at a 4 KiB page size)
/// moves that to one checkpoint every ~15 s while keeping the WAL well under
/// 100 MB. Durability is unchanged: a checkpoint only moves already-committed
/// WAL frames into the main database.
const DEFAULT_SQLITE_WAL_AUTOCHECKPOINT_PAGES: u32 = 16_384;
const MAX_SQLITE_WAL_AUTOCHECKPOINT_PAGES: u32 = 262_144;
const SQLITE_CACHE_SIZE_PRAGMA: i64 = -20_000;

fn sqlite_max_connections_from_env() -> u32 {
    std::env::var("SCRYER_SQLITE_MAX_CONNECTIONS")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_SQLITE_MAX_CONNECTIONS)
        .clamp(1, MAX_SQLITE_CONNECTIONS_CAP)
}

/// `0` disables automatic checkpointing entirely, so it is allowed through
/// unclamped for operators who drive checkpoints themselves.
fn sqlite_wal_autocheckpoint_pages_from_env() -> u32 {
    match std::env::var("SCRYER_SQLITE_WAL_AUTOCHECKPOINT_PAGES")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
    {
        Some(0) => 0,
        Some(value) => value.min(MAX_SQLITE_WAL_AUTOCHECKPOINT_PAGES),
        None => DEFAULT_SQLITE_WAL_AUTOCHECKPOINT_PAGES,
    }
}

/// Refreshes planner statistics once at startup so a restart, or a migration
/// that added indexes, does not wait for the daily housekeeping run. A failure
/// is logged and startup continues.
async fn run_startup_optimize(pool: &sqlx::SqlitePool) {
    let result: Result<(), sqlx::Error> = async {
        // Taken out of the pool and closed afterwards, on every path, so the
        // analysis limit below never reaches a pooled connection and the
        // daily maintenance ANALYZE stays unbounded.
        let mut conn = pool.acquire().await?.detach();
        // Rows sampled per index, so the ANALYZE stays bounded on a large
        // database.
        let optimized = match sqlx::query("PRAGMA analysis_limit = 1000")
            .execute(&mut conn)
            .await
        {
            // 0x10002: analyze every table that may need it, not only the
            // ones this connection has queried.
            Ok(_) => sqlx::query("PRAGMA optimize = 0x10002")
                .execute(&mut conn)
                .await
                .map(|_| ()),
            Err(error) => Err(error),
        };
        let closed = sqlx::Connection::close(conn).await;
        optimized.and(closed)
    }
    .await;
    if let Err(error) = result {
        tracing::warn!(error = %error, "startup PRAGMA optimize failed");
    }
}

#[derive(Clone)]
pub struct SqliteServices {
    pub pool: sqlx::SqlitePool,
    pub encryption_key: Arc<RwLock<Option<EncryptionKey>>>,
    pub writer_gate: SqliteWriterGate,
}

impl SqliteServices {
    /// Public pool accessor for cross-crate query access.
    pub fn pool(&self) -> &sqlx::SqlitePool {
        &self.pool
    }

    pub fn datastore(&self) -> StoreDatastore {
        StoreDatastore::sqlite(self.pool.clone(), self.writer_gate.clone())
    }

    pub async fn new(path: impl AsRef<str>) -> Result<Self, AppError> {
        Self::new_with_mode(path, MigrationMode::Apply).await
    }

    pub async fn new_with_mode(
        path: impl AsRef<str>,
        migration_mode: MigrationMode,
    ) -> Result<Self, AppError> {
        Self::new_with_mode_and_data_dir(path, migration_mode, None).await
    }

    pub async fn new_with_mode_and_data_dir(
        path: impl AsRef<str>,
        migration_mode: MigrationMode,
        data_dir: Option<PathBuf>,
    ) -> Result<Self, AppError> {
        Self::new_with_migration_progress(
            path,
            migration_mode,
            data_dir,
            MigrationProgress::default(),
        )
        .await
    }

    /// Opens the database like [`Self::new_with_mode_and_data_dir`], counting
    /// the migrations it applies into `migration_progress`.
    pub async fn new_with_migration_progress(
        path: impl AsRef<str>,
        migration_mode: MigrationMode,
        data_dir: Option<PathBuf>,
        migration_progress: MigrationProgress,
    ) -> Result<Self, AppError> {
        let db_url = crate::sqlite_url_with_create(path.as_ref());
        let is_memory = db_url.contains(":memory:");

        // Ensure the parent directory exists for file-based databases.
        if !is_memory
            && let Some(file_path) = path
                .as_ref()
                .strip_prefix("sqlite://")
                .or(Some(path.as_ref()))
        {
            let file_path = file_path.split('?').next().unwrap_or(file_path);
            let db_file = std::path::Path::new(file_path);
            if let Some(parent) = db_file.parent() {
                if !parent.as_os_str().is_empty() && !parent.exists() {
                    tracing::info!(path = %parent.display(), "creating database directory");
                    std::fs::create_dir_all(parent).map_err(|err| {
                        AppError::Repository(format!(
                            "cannot create database directory {}: {err}",
                            parent.display(),
                        ))
                    })?;
                }
                if parent.exists() {
                    let meta = std::fs::metadata(parent);
                    let probe = parent.join(".scryer-probe");
                    let writable = std::fs::File::create(&probe).is_ok();
                    let _ = std::fs::remove_file(&probe);
                    tracing::debug!(
                        path = %parent.display(),
                        writable,
                        permissions = ?meta.as_ref().map(|m| m.permissions()),
                        "database directory check",
                    );
                }
            }
        }

        let pool_opts = if is_memory {
            // For in-memory databases the data only lives as long as at least one
            // connection is open. Prevent pool recycling from destroying the DB.
            sqlx::sqlite::SqlitePoolOptions::new()
                .max_connections(1)
                .min_connections(1)
                .idle_timeout(None)
                .max_lifetime(None)
        } else {
            sqlx::sqlite::SqlitePoolOptions::new()
                .max_connections(sqlite_max_connections_from_env())
        };

        let mut connect_opts: sqlx::sqlite::SqliteConnectOptions = db_url
            .parse()
            .map_err(|err: sqlx::Error| AppError::Repository(err.to_string()))?;
        connect_opts = connect_opts.log_slow_statements(
            tracing::log::LevelFilter::Debug,
            std::time::Duration::from_millis(SQLITE_SLOW_STATEMENT_MS),
        );
        if !is_memory {
            connect_opts = connect_opts
                .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
                // WAL with synchronous=NORMAL skips one fsync per commit; a
                // power loss (not a process crash) may drop the last commit(s)
                // but never corrupts the database.
                .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
                .busy_timeout(std::time::Duration::from_millis(10_000))
                // Negative means KiB: 20 MB of page cache per connection.
                .pragma("cache_size", SQLITE_CACHE_SIZE_PRAGMA.to_string())
                .pragma(
                    "wal_autocheckpoint",
                    sqlite_wal_autocheckpoint_pages_from_env().to_string(),
                );
        }

        let pool = pool_opts.connect_with(connect_opts).await.map_err(|err| {
            AppError::Repository(format!("cannot open database at {}: {err}", path.as_ref(),))
        })?;

        let migration_encryption_key = if is_memory {
            None
        } else {
            load_existing_sqlite_migration_encryption_key(&pool, data_dir)
                .await
                .map_err(AppError::Repository)?
        };
        crate::migrations::run_migrations_with_hook_context(
            &pool,
            migration_mode,
            MigrationHookContext {
                encryption_key: migration_encryption_key,
                progress: migration_progress,
            },
        )
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
        if matches!(migration_mode, MigrationMode::Apply) {
            crate::queries::title_search::seed_title_search_projection_if_stale(&pool).await?;
            if !is_memory {
                run_startup_optimize(&pool).await;
            }
        }

        Ok(Self {
            pool,
            encryption_key: Arc::new(RwLock::new(None)),
            writer_gate: new_writer_gate(),
        })
    }

    pub fn encryption_key_state(&self) -> Arc<RwLock<Option<EncryptionKey>>> {
        self.encryption_key.clone()
    }

    #[doc(hidden)]
    pub fn writer_gate(&self) -> SqliteWriterGate {
        self.writer_gate.clone()
    }

    pub async fn set_encryption_key(&self, key: crate::encryption::EncryptionKey) -> AppResult<()> {
        *self
            .encryption_key
            .write()
            .map_err(|_| AppError::Repository("encryption key lock poisoned".to_string()))? =
            Some(key);

        Ok(())
    }

    pub async fn vacuum_into(&self, dest_path: &str) -> AppResult<()> {
        let dest_path = dest_path.to_string();

        SqlRuntime::run_serialized_sqlite(&self.datastore(), "vacuum_into", move |pool| {
            let dest_path = dest_path.clone();
            async move {
                sqlx::query("VACUUM INTO ?")
                    .bind(dest_path)
                    .execute(&pool)
                    .await
                    .map_err(repo_err)?;
                Ok(())
            }
        })
        .await
    }
}

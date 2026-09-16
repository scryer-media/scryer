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
const SQLITE_SLOW_STATEMENT_WARN_MS: u64 = 1000;

fn sqlite_max_connections_from_env() -> u32 {
    std::env::var("SCRYER_SQLITE_MAX_CONNECTIONS")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_SQLITE_MAX_CONNECTIONS)
        .clamp(1, MAX_SQLITE_CONNECTIONS_CAP)
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
        crate::spellfix::register_spellfix_auto_extension()?;

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
            tracing::log::LevelFilter::Warn,
            std::time::Duration::from_millis(SQLITE_SLOW_STATEMENT_WARN_MS),
        );
        if !is_memory {
            connect_opts = connect_opts
                .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
                // WAL pairs with synchronous=NORMAL to skip one fsync per commit;
                // power loss may drop the last commit(s), never corrupts; needs
                // product sign-off to enable.
                // .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
                .busy_timeout(std::time::Duration::from_millis(10_000));
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
            crate::queries::title_search::seed_title_search_projection_if_empty(&pool).await?;
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

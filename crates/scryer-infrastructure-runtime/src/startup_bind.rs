//! The saved listening host, read before the listener binds.
//!
//! This runs before migrations and before the datastore is assembled, so it
//! opens one short-lived connection, never creates a database, and treats a
//! missing settings table or row as "nothing saved": that is a fresh install.
use std::time::Duration;

use sqlx::{ConnectOptions, Connection};

use crate::assembly::{DatastoreConfig, DatastoreEngine, datastore_file_path};

/// Bounds the whole read so an unreachable server cannot hold up the listener.
const READ_TIMEOUT: Duration = Duration::from_secs(5);

const SQLITE_QUERY: &str = "SELECT sv.value_json
     FROM settings_values sv
     JOIN settings_definitions d
       ON sv.setting_definition_id = d.id
      AND sv.scope = d.scope
    WHERE d.scope = ?1 AND d.key_name = ?2 AND sv.scope_id IS NULL
    LIMIT 1";

const POSTGRES_QUERY: &str = "SELECT sv.value_json
     FROM settings_values sv
     JOIN settings_definitions d
       ON sv.setting_definition_id = d.id
      AND sv.scope = d.scope
    WHERE d.scope = $1 AND d.key_name = $2 AND sv.scope_id IS NULL
    LIMIT 1";

/// The saved host as stored, or `None` when nothing is saved. An error is a
/// datastore that exists but could not be read.
pub async fn read_saved_bind_host(config: &DatastoreConfig) -> Result<Option<String>, String> {
    match tokio::time::timeout(READ_TIMEOUT, read(config)).await {
        Ok(result) => result,
        Err(_) => Err(format!(
            "reading the saved listening address took longer than {} seconds",
            READ_TIMEOUT.as_secs()
        )),
    }
}

async fn read(config: &DatastoreConfig) -> Result<Option<String>, String> {
    let key = scryer_application::network_bind::BIND_HOST_KEY;
    let scope = scryer_application::SETTINGS_SCOPE_SYSTEM;
    let raw: Option<String> = match config.engine {
        DatastoreEngine::Sqlite => {
            if !datastore_file_path(&config.database_url).exists() {
                return Ok(None);
            }
            let options = config
                .database_url
                .parse::<sqlx::sqlite::SqliteConnectOptions>()
                .map_err(|error| format!("invalid SQLite database URL: {error}"))?
                .create_if_missing(false)
                .disable_statement_logging();
            let mut connection = options
                .connect()
                .await
                .map_err(|error| format!("could not open the database: {error}"))?;
            let tables: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' \
                 AND name IN ('settings_values', 'settings_definitions')",
            )
            .fetch_one(&mut connection)
            .await
            .map_err(|error| format!("could not inspect the database: {error}"))?;
            let raw = if tables == 2 {
                sqlx::query_scalar(SQLITE_QUERY)
                    .bind(scope)
                    .bind(key)
                    .fetch_optional(&mut connection)
                    .await
                    .map_err(|error| format!("could not read the saved address: {error}"))?
            } else {
                None
            };
            let _ = connection.close().await;
            raw
        }
        DatastoreEngine::Postgres => {
            let options = config
                .database_url
                .parse::<sqlx::postgres::PgConnectOptions>()
                .map_err(|error| format!("invalid PostgreSQL database URL: {error}"))?
                .disable_statement_logging();
            let mut connection = options
                .connect()
                .await
                .map_err(|error| format!("could not connect to the database: {error}"))?;
            let present: bool = sqlx::query_scalar(
                "SELECT to_regclass('settings_values') IS NOT NULL \
                 AND to_regclass('settings_definitions') IS NOT NULL",
            )
            .fetch_one(&mut connection)
            .await
            .map_err(|error| format!("could not inspect the database: {error}"))?;
            let raw = if present {
                sqlx::query_scalar(POSTGRES_QUERY)
                    .bind(scope)
                    .bind(key)
                    .fetch_optional(&mut connection)
                    .await
                    .map_err(|error| format!("could not read the saved address: {error}"))?
            } else {
                None
            };
            let _ = connection.close().await;
            raw
        }
    };
    let Some(raw) = raw else {
        return Ok(None);
    };
    serde_json::from_str::<Option<String>>(&raw)
        .map_err(|error| format!("the saved address is not readable JSON: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MigrationMode;

    fn config_for(path: &std::path::Path) -> DatastoreConfig {
        DatastoreConfig::sqlite(
            format!("sqlite://{}", path.display()),
            path.parent().unwrap(),
            MigrationMode::Apply,
        )
    }

    async fn create(path: &std::path::Path, statements: &[&'static str]) {
        let mut connection = format!("sqlite://{}?mode=rwc", path.display())
            .parse::<sqlx::sqlite::SqliteConnectOptions>()
            .unwrap()
            .connect()
            .await
            .unwrap();
        for statement in statements {
            sqlx::query(*statement)
                .execute(&mut connection)
                .await
                .unwrap();
        }
        connection.close().await.unwrap();
    }

    const SCHEMA: &[&str] = &[
        "CREATE TABLE settings_definitions (id TEXT PRIMARY KEY, scope TEXT NOT NULL, key_name TEXT NOT NULL)",
        "CREATE TABLE settings_values (id TEXT PRIMARY KEY, setting_definition_id TEXT NOT NULL, scope TEXT NOT NULL, scope_id TEXT, value_json TEXT NOT NULL)",
        "INSERT INTO settings_definitions VALUES ('def-bind', 'system', 'network.bind_host')",
    ];

    #[tokio::test]
    async fn startup_bind_read_tolerates_a_fresh_install() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("scryer.db");
        assert_eq!(read_saved_bind_host(&config_for(&path)).await, Ok(None));
        assert!(!path.exists(), "the read must never create a database");

        create(&path, &[]).await;
        assert_eq!(read_saved_bind_host(&config_for(&path)).await, Ok(None));

        create(&path, SCHEMA).await;
        assert_eq!(read_saved_bind_host(&config_for(&path)).await, Ok(None));
    }

    #[tokio::test]
    async fn startup_bind_read_returns_the_saved_system_value_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("scryer.db");
        let mut statements = SCHEMA.to_vec();
        statements.push(
            "INSERT INTO settings_values VALUES ('scoped', 'def-bind', 'system', 'other', '\"192.0.2.9\"')",
        );
        create(&path, &statements).await;
        assert_eq!(read_saved_bind_host(&config_for(&path)).await, Ok(None));

        create(
            &path,
            &["INSERT INTO settings_values VALUES ('saved', 'def-bind', 'system', NULL, '\"0.0.0.0\"')"],
        )
        .await;
        assert_eq!(
            read_saved_bind_host(&config_for(&path)).await,
            Ok(Some("0.0.0.0".to_string()))
        );

        create(
            &path,
            &["UPDATE settings_values SET value_json = 'null' WHERE id = 'saved'"],
        )
        .await;
        assert_eq!(read_saved_bind_host(&config_for(&path)).await, Ok(None));

        create(
            &path,
            &["UPDATE settings_values SET value_json = 'not json' WHERE id = 'saved'"],
        )
        .await;
        assert!(read_saved_bind_host(&config_for(&path)).await.is_err());
    }
}

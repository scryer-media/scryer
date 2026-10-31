use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use chrono::Utc;
use scryer_application::{
    AppError, AppResult, DownloadClientConfigRepository, DownloadClientConfigUpdate,
};
use scryer_domain::{DownloadClientConfig, DownloadClientStatus};

use crate::config_store::{current_encryption_key, decrypt_value, maybe_encrypt_value};
use crate::encryption::EncryptionKey;
use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, StoreDatastore};

const DOWNLOAD_CLIENT_COLUMNS: &str = "id, name, client_type, config_json, is_enabled, status,
    client_priority, last_error, last_seen_at, proxy_config_id, created_at, updated_at";

const DOWNLOAD_CLIENT_INSERT_SQL: &str = "INSERT INTO download_clients (
    id, name, client_type, config_json, is_enabled, status,
    client_priority, last_error, last_seen_at, proxy_config_id, created_at, updated_at
) VALUES (
    {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}
)";

#[derive(Clone)]
pub struct DownloadClientConfigStore {
    datastore: StoreDatastore,
    encryption_key: Arc<RwLock<Option<EncryptionKey>>>,
}

impl DownloadClientConfigStore {
    pub fn new(
        datastore: StoreDatastore,
        encryption_key: Arc<RwLock<Option<EncryptionKey>>>,
    ) -> Self {
        Self {
            datastore,
            encryption_key,
        }
    }

    fn encryption_key(&self) -> AppResult<Option<EncryptionKey>> {
        current_encryption_key(&self.encryption_key)
    }
}

#[async_trait]
impl DownloadClientConfigRepository for DownloadClientConfigStore {
    async fn list(&self, client_type: Option<String>) -> AppResult<Vec<DownloadClientConfig>> {
        let encryption_key = self.encryption_key()?;
        let (sql, args) = match client_type {
            Some(client_type) => (
                format!(
                    "SELECT {DOWNLOAD_CLIENT_COLUMNS} FROM download_clients WHERE client_type = {{}} ORDER BY client_priority ASC"
                ),
                vec![SqlArg::Text(client_type)],
            ),
            None => (
                format!(
                    "SELECT {DOWNLOAD_CLIENT_COLUMNS} FROM download_clients ORDER BY client_priority ASC"
                ),
                Vec::new(),
            ),
        };
        fetch_download_clients(
            self.datastore.read_exec(),
            &sql,
            &args,
            encryption_key.as_ref(),
        )
        .await
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<DownloadClientConfig>> {
        let encryption_key = self.encryption_key()?;
        fetch_optional_download_client(
            self.datastore.read_exec(),
            &format!("SELECT {DOWNLOAD_CLIENT_COLUMNS} FROM download_clients WHERE id = {{}}"),
            &[SqlArg::Text(id.to_string())],
            encryption_key.as_ref(),
        )
        .await
    }

    async fn create(&self, config: DownloadClientConfig) -> AppResult<DownloadClientConfig> {
        let encryption_key = self.encryption_key()?;
        let args = download_client_insert_args(&config, encryption_key.as_ref())?;
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "create_download_client_config",
            move |tx| {
                let config = config.clone();
                let args = args.clone();
                Box::pin(async move {
                    SqlRuntime::execute(SqlExec::Tx(tx), DOWNLOAD_CLIENT_INSERT_SQL, &args).await?;
                    Ok(config)
                })
            },
        )
        .await
    }

    async fn update(&self, update: DownloadClientConfigUpdate) -> AppResult<DownloadClientConfig> {
        let encryption_key = self.encryption_key()?;
        let mut assignments = vec!["updated_at = {}".to_string()];
        let mut args = vec![SqlArg::Timestamp(Utc::now())];

        if let Some(name) = update.name.as_ref() {
            assignments.push("name = {}".to_string());
            args.push(SqlArg::Text(name.clone()));
        }
        if let Some(client_type) = update.client_type.as_ref() {
            assignments.push("client_type = {}".to_string());
            args.push(SqlArg::Text(client_type.clone()));
        }
        if let Some(config_json) = update.config_json.as_ref() {
            assignments.push("config_json = {}".to_string());
            args.push(SqlArg::Text(maybe_encrypt_value(
                encryption_key.as_ref(),
                config_json,
            )?));
        }
        if let Some(is_enabled) = update.is_enabled {
            assignments.push("is_enabled = {}".to_string());
            args.push(SqlArg::Bool(is_enabled));
        }
        // Tri-state: an omitted patch leaves the assignment alone, `Some(None)`
        // clears it, `Some(Some(id))` sets it.
        if let Some(proxy_config_id) = update.proxy_config_id.as_ref() {
            assignments.push("proxy_config_id = {}".to_string());
            args.push(SqlArg::OptText(proxy_config_id.clone()));
        }

        if assignments.len() == 1 {
            return Err(AppError::validation_refused(
                crate::error_reasons::ACQUISITION_DOWNLOAD_CLIENT_UPDATE_EMPTY,
                "at least one download client config field must be provided",
            ));
        }

        let id = update.id.clone();
        args.push(SqlArg::Text(id.clone()));
        let sql = format!(
            "UPDATE download_clients SET {} WHERE id = {{}}",
            assignments.join(", ")
        );
        let saved = SqlRuntime::run_in_transaction(
            &self.datastore,
            "update_download_client_config",
            move |tx| {
                let sql = sql.clone();
                let args = args.clone();
                let id = id.clone();
                let encryption_key = encryption_key.clone();
                Box::pin(async move {
                    let rows = SqlRuntime::execute(SqlExec::Tx(tx), &sql, &args).await?;
                    if rows == 0 {
                        return Err(AppError::not_found_reason(
                            crate::error_reasons::ACQUISITION_DOWNLOAD_CLIENT_CONFIG_NOT_FOUND,
                            format!("download client config {id}"),
                        ));
                    }
                    fetch_optional_download_client(
                        SqlExec::Tx(tx),
                        &format!(
                            "SELECT {DOWNLOAD_CLIENT_COLUMNS} FROM download_clients WHERE id = {{}}"
                        ),
                        &[SqlArg::Text(id.clone())],
                        encryption_key.as_ref(),
                    )
                    .await?
                    .ok_or_else(|| {
                        AppError::not_found_reason(
                            crate::error_reasons::ACQUISITION_DOWNLOAD_CLIENT_CONFIG_NOT_FOUND,
                            format!("download client config {id}"),
                        )
                    })
                })
            },
        )
        .await?;
        // A running HTTP downloader follows its saved configuration at once:
        // disabled, it starts nothing new; no longer `http`, it stops.
        crate::downloads::clients::apply_saved_http_config(&saved);
        Ok(saved)
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        self.delete_with_cleared_indexer_mapping_count(id)
            .await
            .map(|_| ())
    }

    async fn delete_with_cleared_indexer_mapping_count(&self, id: &str) -> AppResult<u64> {
        let config_id = id.to_string();
        let id = id.to_string();
        let cleared = SqlRuntime::run_in_transaction(
            &self.datastore,
            "delete_download_client_config",
            move |tx| {
                let id = id.clone();
                Box::pin(async move {
                    let cleared = SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "UPDATE indexers
                            SET download_client_id = NULL,
                                updated_at = {}
                          WHERE download_client_id = {}",
                        &[SqlArg::Timestamp(Utc::now()), SqlArg::Text(id.clone())],
                    )
                    .await?;
                    // A binding is a fact about a client job. Deleting the
                    // client deletes every job it could ever list, so the
                    // bindings end here, in the same transaction: a binding
                    // that outlives its client can never be resolved again and
                    // nothing must be left holding a scope. Sonarr drops the
                    // tracked downloads of a removed client the same way.
                    let ended_at = Utc::now();
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "UPDATE download_client_bindings
                            SET ended_at = {}
                          WHERE client_config_id = {}
                            AND ended_at IS NULL",
                        &[SqlArg::Timestamp(ended_at), SqlArg::Text(id.clone())],
                    )
                    .await?;
                    // Ending the binding is not enough. The observation
                    // resolver rebinds an ended binding whose download is still
                    // live onto whichever client reports the job next, so a
                    // download left non-terminal here would be re-adopted the
                    // moment the same client is re-registered under a new id —
                    // which is exactly how a load-test instance ended up with
                    // 961 canonical downloads conflicting with themselves on
                    // every poll. Deleting the client is Scryer deciding it is
                    // done with those downloads, so they are marked terminal
                    // (see `DownloadRecord::terminal_at`) in the same
                    // transaction.
                    //
                    // Only downloads this client's binding was the last active
                    // one for: a download that some other config still holds
                    // actively is untouched.
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "UPDATE downloads
                            SET terminal_at = {}
                          WHERE terminal_at IS NULL
                            AND id IN (
                                SELECT download_id
                                  FROM download_client_bindings
                                 WHERE client_config_id = {}
                            )
                            AND NOT EXISTS (
                                SELECT 1
                                  FROM download_client_bindings other
                                 WHERE other.download_id = downloads.id
                                   AND other.ended_at IS NULL
                            )",
                        &[SqlArg::Timestamp(ended_at), SqlArg::Text(id.clone())],
                    )
                    .await?;
                    let rows = SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "DELETE FROM download_clients WHERE id = {}",
                        &[SqlArg::Text(id.clone())],
                    )
                    .await?;
                    if rows == 0 {
                        return Err(AppError::not_found_reason(
                            crate::error_reasons::ACQUISITION_DOWNLOAD_CLIENT_CONFIG_NOT_FOUND,
                            format!("download client config {id}"),
                        ));
                    }
                    Ok(cleared)
                })
            },
        )
        .await?;
        // The deleted client's HTTP downloader, if it ran one, stops; its
        // queue and files stay on disk.
        crate::downloads::clients::forget_http_engine(&config_id);
        Ok(cleared)
    }

    async fn reorder(&self, ordered_ids: Vec<String>) -> AppResult<()> {
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "reorder_download_client_configs",
            move |tx| {
                let ordered_ids = ordered_ids.clone();
                Box::pin(async move {
                    for (index, id) in ordered_ids.iter().enumerate() {
                        SqlRuntime::execute(
                            SqlExec::Tx(tx),
                            "UPDATE download_clients
                             SET client_priority = {}, updated_at = {}
                             WHERE id = {}",
                            &[
                                SqlArg::I64(index as i64),
                                SqlArg::Timestamp(Utc::now()),
                                SqlArg::Text(id.clone()),
                            ],
                        )
                        .await?;
                    }
                    Ok(())
                })
            },
        )
        .await
    }
}

fn download_client_insert_args(
    config: &DownloadClientConfig,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<Vec<SqlArg>> {
    Ok(vec![
        SqlArg::Text(config.id.clone()),
        SqlArg::Text(config.name.clone()),
        SqlArg::Text(config.client_type.clone()),
        SqlArg::Text(maybe_encrypt_value(encryption_key, &config.config_json)?),
        SqlArg::Bool(config.is_enabled),
        SqlArg::Text(config.status.as_str().to_string()),
        SqlArg::I64(config.client_priority),
        SqlArg::OptText(config.last_error.clone()),
        SqlArg::OptTimestamp(config.last_seen_at),
        SqlArg::OptText(config.proxy_config_id.clone()),
        SqlArg::Timestamp(config.created_at),
        SqlArg::Timestamp(config.updated_at),
    ])
}

async fn fetch_download_clients(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<Vec<DownloadClientConfig>> {
    SqlRuntime::fetch_all(exec, sql, args)
        .await?
        .into_iter()
        .map(|row| row_to_download_client_config(&row, encryption_key))
        .collect()
}

async fn fetch_optional_download_client(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<Option<DownloadClientConfig>> {
    SqlRuntime::fetch_optional(exec, sql, args)
        .await?
        .map(|row| row_to_download_client_config(&row, encryption_key))
        .transpose()
}

fn row_to_download_client_config(
    row: &SqlRow,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<DownloadClientConfig> {
    let status_raw = row.text("status")?;
    Ok(DownloadClientConfig {
        id: row.text("id")?,
        name: row.text("name")?,
        client_type: row.text("client_type")?,
        config_json: decrypt_value(
            encryption_key,
            row.text("config_json")?,
            "config_json",
            false,
        )?,
        client_priority: row.i64("client_priority")?,
        is_enabled: row.bool("is_enabled")?,
        status: DownloadClientStatus::parse(&status_raw).unwrap_or_default(),
        last_error: row.opt_text("last_error")?,
        last_seen_at: row.opt_timestamp("last_seen_at")?,
        proxy_config_id: row.opt_text("proxy_config_id")?,
        created_at: row.timestamp("created_at")?,
        updated_at: row.timestamp("updated_at")?,
    })
}

#[cfg(test)]
mod tests {
    use scryer_application::DownloadClientConfigRepository;
    use sqlx::Row;
    use sqlx::sqlite::SqlitePoolOptions;

    use super::*;

    #[tokio::test]
    async fn delete_atomically_clears_indexer_mappings_and_returns_count() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");
        sqlx::query("CREATE TABLE download_clients (id TEXT PRIMARY KEY)")
            .execute(&pool)
            .await
            .expect("download_clients table should be created");
        sqlx::query(
            "CREATE TABLE indexers (
                id TEXT PRIMARY KEY,
                download_client_id TEXT,
                updated_at TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .expect("indexers table should be created");
        sqlx::query(
            "CREATE TABLE download_client_bindings (
                download_id TEXT PRIMARY KEY,
                client_config_id TEXT,
                client_type_snapshot TEXT,
                native_item_id TEXT,
                created_at TEXT NOT NULL,
                ended_at TEXT
            )",
        )
        .execute(&pool)
        .await
        .expect("download_client_bindings table should be created");
        sqlx::query(
            "CREATE TABLE downloads (
                id TEXT PRIMARY KEY,
                origin TEXT NOT NULL,
                created_at TEXT NOT NULL,
                terminal_at TEXT
            )",
        )
        .execute(&pool)
        .await
        .expect("downloads table should be created");
        sqlx::query(
            "INSERT INTO downloads (id, origin, created_at) VALUES
                 ('download-1', 'scryer_submission', '2026-01-01T00:00:00Z'),
                 ('download-2', 'scryer_submission', '2026-01-01T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("downloads should insert");
        sqlx::query(
            "INSERT INTO download_client_bindings
                 (download_id, client_config_id, client_type_snapshot, native_item_id, created_at)
             VALUES
                 ('download-1', 'client-1', 'nzbget', 'job-1', '2026-01-01T00:00:00Z'),
                 ('download-2', 'client-2', 'nzbget', 'job-2', '2026-01-01T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("bindings should insert");
        sqlx::query("INSERT INTO download_clients (id) VALUES ('client-1'), ('client-2')")
            .execute(&pool)
            .await
            .expect("clients should insert");
        sqlx::query(
            "INSERT INTO indexers (id, download_client_id, updated_at) VALUES
                ('idx-1', 'client-1', '2026-01-01T00:00:00Z'),
                ('idx-2', 'client-1', '2026-01-01T00:00:00Z'),
                ('idx-3', 'client-2', '2026-01-01T00:00:00Z'),
                ('idx-missing', 'missing-client', '2026-01-01T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("indexers should insert");

        let store = DownloadClientConfigStore::new(
            StoreDatastore::Sqlite {
                pool: pool.clone(),
                writer_gate: Arc::new(tokio::sync::Mutex::new(())),
            },
            Arc::new(RwLock::new(None)),
        );

        let cleared = store
            .delete_with_cleared_indexer_mapping_count("client-1")
            .await
            .expect("client deletion should succeed");
        assert_eq!(cleared, 2);

        let rows =
            sqlx::query("SELECT id, download_client_id, updated_at FROM indexers ORDER BY id")
                .fetch_all(&pool)
                .await
                .expect("indexers should load");
        assert_eq!(rows[0].get::<Option<String>, _>("download_client_id"), None);
        assert_ne!(
            rows[0].get::<String, _>("updated_at"),
            "2026-01-01T00:00:00Z"
        );
        assert_eq!(rows[1].get::<Option<String>, _>("download_client_id"), None);
        assert_eq!(
            rows[2]
                .get::<Option<String>, _>("download_client_id")
                .as_deref(),
            Some("client-2")
        );

        let ended: Vec<(String, Option<String>)> = sqlx::query_as(
            "SELECT download_id, ended_at FROM download_client_bindings ORDER BY download_id",
        )
        .fetch_all(&pool)
        .await
        .expect("bindings should load");
        assert!(
            ended[0].1.is_some(),
            "the deleted client's binding ends in the same transaction"
        );
        assert!(
            ended[1].1.is_none(),
            "another client's binding is untouched"
        );

        store
            .delete_with_cleared_indexer_mapping_count("missing-client")
            .await
            .expect_err("missing client deletion should roll back mapping cleanup");
        let missing_mapping: Option<String> =
            sqlx::query_scalar("SELECT download_client_id FROM indexers WHERE id = 'idx-missing'")
                .fetch_one(&pool)
                .await
                .expect("rolled-back mapping should load");
        assert_eq!(missing_mapping.as_deref(), Some("missing-client"));
        let untouched: Option<String> = sqlx::query_scalar(
            "SELECT ended_at FROM download_client_bindings WHERE download_id = 'download-2'",
        )
        .fetch_one(&pool)
        .await
        .expect("rolled-back binding should load");
        assert!(
            untouched.is_none(),
            "a failed delete rolls its binding cleanup back too"
        );

        let terminal: Vec<(String, Option<String>)> =
            sqlx::query_as("SELECT id, terminal_at FROM downloads ORDER BY id")
                .fetch_all(&pool)
                .await
                .expect("downloads should load");
        assert!(
            terminal[0].1.is_some(),
            "the deleted client's download is finished with its binding"
        );
        assert!(
            terminal[1].1.is_none(),
            "another client's download is untouched"
        );
    }

    /// Ending the binding is not enough on its own: the observation resolver
    /// rebinds an ended binding whose download is still live onto whichever
    /// client reports the job next, so a download left non-terminal here comes
    /// straight back when the same client is re-registered under a new id.
    #[tokio::test]
    async fn delete_leaves_a_download_alone_while_another_config_binds_it_actively() {
        let pool = delete_terminal_schema().await;
        // download-shared has two bindings: client-1's (about to be deleted)
        // and client-2's, which stays active.
        sqlx::query(
            "INSERT INTO downloads (id, origin, created_at) VALUES
                 ('download-own', 'scryer_submission', '2026-01-01T00:00:00Z'),
                 ('download-shared', 'scryer_submission', '2026-01-01T00:00:00Z'),
                 ('download-done', 'scryer_submission', '2026-01-01T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("downloads should insert");
        sqlx::query(
            "INSERT INTO download_client_bindings
                 (download_id, client_config_id, client_type_snapshot, native_item_id, created_at, ended_at)
             VALUES
                 ('download-own', 'client-1', 'nzbget', 'job-1', '2026-01-01T00:00:00Z', NULL),
                 ('download-shared', 'client-2', 'nzbget', 'job-2', '2026-01-01T00:00:00Z', NULL),
                 ('download-done', 'client-1', 'nzbget', 'job-3', '2026-01-01T00:00:00Z', '2026-01-02T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("bindings should insert");
        // The shared download is also reachable from the deleted client, via a
        // binding row that is already ended.
        sqlx::query(
            "UPDATE download_client_bindings SET client_config_id = 'client-1'
              WHERE download_id = 'download-done'",
        )
        .execute(&pool)
        .await
        .expect("binding should update");
        sqlx::query("INSERT INTO download_clients (id) VALUES ('client-1'), ('client-2')")
            .execute(&pool)
            .await
            .expect("clients should insert");

        let store = DownloadClientConfigStore::new(
            StoreDatastore::Sqlite {
                pool: pool.clone(),
                writer_gate: Arc::new(tokio::sync::Mutex::new(())),
            },
            Arc::new(RwLock::new(None)),
        );
        store
            .delete_with_cleared_indexer_mapping_count("client-1")
            .await
            .expect("client deletion should succeed");

        let terminal: Vec<(String, Option<String>)> =
            sqlx::query_as("SELECT id, terminal_at FROM downloads ORDER BY id")
                .fetch_all(&pool)
                .await
                .expect("downloads should load");
        assert_eq!(terminal[0].0, "download-done");
        assert!(
            terminal[0].1.is_some(),
            "a download whose only binding was this client's is finished"
        );
        assert_eq!(terminal[1].0, "download-own");
        assert!(
            terminal[1].1.is_some(),
            "the deleted client's live download is finished"
        );
        assert_eq!(terminal[2].0, "download-shared");
        assert!(
            terminal[2].1.is_none(),
            "a download another config still binds actively must stay live"
        );
    }

    async fn delete_terminal_schema() -> sqlx::SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");
        for statement in [
            "CREATE TABLE download_clients (id TEXT PRIMARY KEY)",
            "CREATE TABLE indexers (
                 id TEXT PRIMARY KEY,
                 download_client_id TEXT,
                 updated_at TEXT NOT NULL
             )",
            "CREATE TABLE downloads (
                 id TEXT PRIMARY KEY,
                 origin TEXT NOT NULL,
                 created_at TEXT NOT NULL,
                 terminal_at TEXT
             )",
            "CREATE TABLE download_client_bindings (
                 download_id TEXT PRIMARY KEY,
                 client_config_id TEXT,
                 client_type_snapshot TEXT,
                 native_item_id TEXT,
                 created_at TEXT NOT NULL,
                 ended_at TEXT
             )",
        ] {
            sqlx::query(statement)
                .execute(&pool)
                .await
                .expect("schema should be created");
        }
        pool
    }

    /// Postgres counterpart: the terminalising UPDATE uses a correlated
    /// `NOT EXISTS` against the table it is updating, which the two dialects
    /// plan differently. Skipped unless `SCRYER_TEST_POSTGRES_URL` is set.
    #[tokio::test]
    async fn postgres_delete_terminalises_only_downloads_it_was_the_last_binding_for() {
        let Some(raw_url) = std::env::var("SCRYER_TEST_POSTGRES_URL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        else {
            eprintln!(
                "skipping PostgreSQL client-delete terminal test; SCRYER_TEST_POSTGRES_URL is not set"
            );
            return;
        };
        let admin_pool = sqlx::PgPool::connect(&raw_url)
            .await
            .expect("postgres should connect");
        let schema = format!(
            "scryer_test_{}_{}",
            std::process::id(),
            scryer_domain::Id::new().0.replace('-', "_")
        );
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin_pool)
            .await
            .expect("test schema should be created");

        let separator = if raw_url.contains('?') { '&' } else { '?' };
        let schema_url = format!("{raw_url}{separator}options=-csearch_path%3D{schema}");
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .connect(&schema_url)
            .await
            .expect("postgres should connect with search_path");
        for statement in [
            "CREATE TABLE download_clients (id TEXT PRIMARY KEY)",
            "CREATE TABLE indexers (
                 id TEXT PRIMARY KEY,
                 download_client_id TEXT,
                 updated_at TIMESTAMP WITH TIME ZONE NOT NULL
             )",
            "CREATE TABLE downloads (
                 id TEXT PRIMARY KEY,
                 origin TEXT NOT NULL,
                 created_at TIMESTAMP WITH TIME ZONE NOT NULL,
                 terminal_at TIMESTAMP WITH TIME ZONE
             )",
            "CREATE TABLE download_client_bindings (
                 download_id TEXT PRIMARY KEY REFERENCES downloads(id),
                 client_config_id TEXT,
                 client_type_snapshot TEXT,
                 native_item_id TEXT,
                 created_at TIMESTAMP WITH TIME ZONE NOT NULL,
                 ended_at TIMESTAMP WITH TIME ZONE
             )",
            "INSERT INTO download_clients (id) VALUES ('client-1'), ('client-2')",
            "INSERT INTO downloads (id, origin, created_at) VALUES
                 ('download-own', 'scryer_submission', NOW()),
                 ('download-shared', 'scryer_submission', NOW())",
            "INSERT INTO download_client_bindings
                 (download_id, client_config_id, client_type_snapshot, native_item_id, created_at)
             VALUES
                 ('download-own', 'client-1', 'nzbget', 'job-1', NOW()),
                 ('download-shared', 'client-2', 'nzbget', 'job-2', NOW())",
        ] {
            sqlx::query(sqlx::AssertSqlSafe(statement))
                .execute(&pool)
                .await
                .expect("schema and fixtures should be created");
        }

        let store = DownloadClientConfigStore::new(
            StoreDatastore::Postgres { pool: pool.clone() },
            Arc::new(RwLock::new(None)),
        );
        store
            .delete_with_cleared_indexer_mapping_count("client-1")
            .await
            .expect("client deletion should succeed");

        let terminal: Vec<(String, Option<chrono::DateTime<Utc>>)> =
            sqlx::query_as("SELECT id, terminal_at FROM downloads ORDER BY id")
                .fetch_all(&pool)
                .await
                .expect("downloads should load");
        assert!(terminal[0].1.is_some(), "download-own must be finished");
        assert!(terminal[1].1.is_none(), "download-shared must stay live");

        sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&admin_pool)
            .await
            .expect("test schema should be dropped");
    }

    #[tokio::test]
    async fn proxy_assignment_round_trips_and_patches_as_a_tri_state() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");
        sqlx::query(
            "CREATE TABLE download_clients (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                client_type TEXT NOT NULL,
                config_json TEXT NOT NULL,
                is_enabled INTEGER NOT NULL DEFAULT 1,
                status TEXT NOT NULL DEFAULT 'idle',
                client_priority INTEGER NOT NULL DEFAULT 0,
                last_error TEXT,
                last_seen_at TEXT,
                proxy_config_id TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .expect("download_clients table should be created");

        let store = DownloadClientConfigStore::new(
            StoreDatastore::Sqlite {
                pool: pool.clone(),
                writer_gate: Arc::new(tokio::sync::Mutex::new(())),
            },
            Arc::new(RwLock::new(None)),
        );

        let now = Utc::now();
        let created = store
            .create(DownloadClientConfig {
                id: "client-1".to_string(),
                name: "Seedbox SAB".to_string(),
                client_type: "sabnzbd".to_string(),
                config_json: "{}".to_string(),
                client_priority: 0,
                is_enabled: true,
                status: DownloadClientStatus::Healthy,
                last_error: None,
                last_seen_at: None,
                proxy_config_id: Some("proxy-1".to_string()),
                created_at: now,
                updated_at: now,
            })
            .await
            .expect("client should insert");
        assert_eq!(created.proxy_config_id.as_deref(), Some("proxy-1"));

        let loaded = store
            .get_by_id("client-1")
            .await
            .expect("client should load")
            .expect("client should exist");
        assert_eq!(loaded.proxy_config_id.as_deref(), Some("proxy-1"));

        // An omitted patch leaves the assignment alone.
        let renamed = store
            .update(DownloadClientConfigUpdate {
                id: "client-1".to_string(),
                name: Some("Renamed".to_string()),
                ..Default::default()
            })
            .await
            .expect("rename should succeed");
        assert_eq!(renamed.proxy_config_id.as_deref(), Some("proxy-1"));

        // An explicit null clears it.
        let cleared = store
            .update(DownloadClientConfigUpdate {
                id: "client-1".to_string(),
                proxy_config_id: Some(None),
                ..Default::default()
            })
            .await
            .expect("clearing should succeed");
        assert_eq!(cleared.proxy_config_id, None);

        // And a value sets it again.
        let reassigned = store
            .update(DownloadClientConfigUpdate {
                id: "client-1".to_string(),
                proxy_config_id: Some(Some("proxy-2".to_string())),
                ..Default::default()
            })
            .await
            .expect("reassignment should succeed");
        assert_eq!(reassigned.proxy_config_id.as_deref(), Some("proxy-2"));
    }
}

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use chrono::Utc;
use scryer_application::{
    AppError, AppResult, IndexerConfigRepository, IndexerConfigUpdate, IndexerSystemBackoff,
};
use scryer_domain::IndexerConfig;
use serde_json::Value as JsonValue;

use crate::config_store::{
    current_encryption_key, decrypt_optional_value, maybe_encrypt_optional, maybe_encrypt_value,
};
use crate::encryption::EncryptionKey;
use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, StoreDatastore};

const INDEXER_COLUMNS: &str =
    "id, name, provider_type, base_url, api_key_encrypted, rate_limit_seconds,
    rate_limit_burst, disabled_until, is_enabled, enable_interactive_search, enable_auto_search,
    proxy_config_id, download_client_id, seeding_profile_id, managed_parent_config_id,
    managed_child_key,
    managed_metadata_json,
    caps_snapshot_json, last_health_status, last_error_message, last_error_at, config_json,
    created_at, updated_at";

const INDEXER_INSERT_SQL: &str = "INSERT INTO indexers (
    id, name, provider_type, base_url, api_key_encrypted, rate_limit_seconds,
    rate_limit_burst, disabled_until, is_enabled, enable_interactive_search,
    enable_auto_search, proxy_config_id, download_client_id, seeding_profile_id,
    managed_parent_config_id, managed_child_key,
    managed_metadata_json, caps_snapshot_json, last_health_status, last_error_message,
    last_error_at, config_json, created_at, updated_at
) VALUES (
    {}, {}, {}, {}, {}, {}, {}, {}, {}, {},
    {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}
)";

#[derive(Clone)]
pub struct IndexerConfigStore {
    datastore: StoreDatastore,
    encryption_key: Arc<RwLock<Option<EncryptionKey>>>,
}

impl IndexerConfigStore {
    pub fn new(
        datastore: StoreDatastore,
        encryption_key: Arc<RwLock<Option<EncryptionKey>>>,
    ) -> Self {
        Self {
            datastore,
            encryption_key,
        }
    }

    pub async fn migrate_legacy_indexer_config_sources(&self) -> AppResult<u64> {
        let encryption_key = self.encryption_key()?;
        let configs = fetch_indexers(
            self.datastore.read_exec(),
            &format!("SELECT {INDEXER_COLUMNS} FROM indexers ORDER BY created_at DESC"),
            &[],
            encryption_key.as_ref(),
        )
        .await?;

        let mut updates = Vec::new();
        for config in configs {
            if config.base_url.trim().is_empty() && config.api_key_encrypted.is_none() {
                continue;
            }

            let mut object = config
                .config_json
                .as_deref()
                .and_then(|raw| serde_json::from_str::<JsonValue>(raw).ok())
                .and_then(|value| value.as_object().cloned())
                .unwrap_or_default();

            let connection_key = legacy_indexer_connection_key(&config.provider_type);
            let mut changed = false;
            if !config.base_url.trim().is_empty()
                && config_value_missing(object.get(connection_key))
            {
                object.insert(
                    connection_key.to_string(),
                    JsonValue::String(config.base_url.trim().to_string()),
                );
                changed = true;
            }

            let legacy_api_key = config
                .api_key_encrypted
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty());
            if let Some(api_key) = legacy_api_key
                && config_value_missing(object.get("api_key"))
            {
                object.insert(
                    "api_key".to_string(),
                    JsonValue::String(api_key.to_string()),
                );
                changed = true;
            }

            let clear_legacy_api_key = config.api_key_encrypted.is_some();
            if !changed && !clear_legacy_api_key {
                continue;
            }

            let stored_config_json = if changed {
                let normalized = JsonValue::Object(object).to_string();
                maybe_encrypt_optional(encryption_key.as_ref(), Some(&normalized))?
            } else {
                None
            };
            updates.push((config.id, stored_config_json));
        }

        if updates.is_empty() {
            return Ok(0);
        }

        let update_count = updates.len() as u64;
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "migrate_legacy_indexer_config_sources",
            move |tx| {
                let updates = updates.clone();
                Box::pin(async move {
                    let now = Utc::now();
                    for (id, stored_config_json) in updates {
                        if let Some(stored_config_json) = stored_config_json {
                            SqlRuntime::execute(
                                SqlExec::Tx(tx),
                                "UPDATE indexers
                                 SET config_json = {}, api_key_encrypted = NULL, updated_at = {}
                                 WHERE id = {}",
                                &[
                                    SqlArg::Text(stored_config_json),
                                    SqlArg::Timestamp(now),
                                    SqlArg::Text(id),
                                ],
                            )
                            .await?;
                        } else {
                            SqlRuntime::execute(
                                SqlExec::Tx(tx),
                                "UPDATE indexers
                                 SET api_key_encrypted = NULL, updated_at = {}
                                 WHERE id = {}",
                                &[SqlArg::Timestamp(now), SqlArg::Text(id)],
                            )
                            .await?;
                        }
                    }
                    Ok(update_count)
                })
            },
        )
        .await
    }

    fn encryption_key(&self) -> AppResult<Option<EncryptionKey>> {
        current_encryption_key(&self.encryption_key)
    }
}

#[async_trait]
impl IndexerConfigRepository for IndexerConfigStore {
    async fn list(&self, provider_type: Option<String>) -> AppResult<Vec<IndexerConfig>> {
        let encryption_key = self.encryption_key()?;
        let (sql, args) = match provider_type {
            Some(provider_type) => (
                format!(
                    "SELECT {INDEXER_COLUMNS} FROM indexers WHERE provider_type = {{}} ORDER BY created_at DESC"
                ),
                vec![SqlArg::Text(provider_type)],
            ),
            None => (
                format!("SELECT {INDEXER_COLUMNS} FROM indexers ORDER BY created_at DESC"),
                Vec::new(),
            ),
        };
        fetch_indexers(
            self.datastore.read_exec(),
            &sql,
            &args,
            encryption_key.as_ref(),
        )
        .await
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<IndexerConfig>> {
        let encryption_key = self.encryption_key()?;
        fetch_optional_indexer(
            self.datastore.read_exec(),
            &format!("SELECT {INDEXER_COLUMNS} FROM indexers WHERE id = {{}}"),
            &[SqlArg::Text(id.to_string())],
            encryption_key.as_ref(),
        )
        .await
    }

    async fn create(&self, config: IndexerConfig) -> AppResult<IndexerConfig> {
        let encryption_key = self.encryption_key()?;
        let args = indexer_insert_args(&config, encryption_key.as_ref())?;
        SqlRuntime::run_in_transaction(&self.datastore, "create_indexer_config", move |tx| {
            let config = config.clone();
            let args = args.clone();
            Box::pin(async move {
                SqlRuntime::execute(SqlExec::Tx(tx), INDEXER_INSERT_SQL, &args).await?;
                Ok(config)
            })
        })
        .await
    }

    async fn touch_last_error(&self, id: &str) -> AppResult<()> {
        let id = id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "touch_indexer_last_error", move |tx| {
            let id = id.clone();
            Box::pin(async move {
                let now = Utc::now();
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "UPDATE indexers
                     SET last_error_at = {}
                     WHERE id = {}",
                    &[SqlArg::Timestamp(now), SqlArg::Text(id)],
                )
                .await?;
                Ok(())
            })
        })
        .await
    }

    async fn record_last_error(&self, id: &str, message: Option<String>) -> AppResult<()> {
        let id = id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "record_indexer_last_error", move |tx| {
            let id = id.clone();
            let message = message.clone();
            Box::pin(async move {
                let now = Utc::now();
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "UPDATE indexers
                     SET last_error_at = {}, last_error_message = {}
                     WHERE id = {}",
                    &[
                        SqlArg::Timestamp(now),
                        SqlArg::OptText(message),
                        SqlArg::Text(id),
                    ],
                )
                .await?;
                Ok(())
            })
        })
        .await
    }

    async fn clear_last_error(&self, id: &str) -> AppResult<()> {
        let id = id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "clear_indexer_last_error", move |tx| {
            let id = id.clone();
            Box::pin(async move {
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "UPDATE indexers
                     SET last_error_at = NULL, last_error_message = NULL, last_health_status = NULL
                     WHERE id = {}",
                    &[SqlArg::Text(id)],
                )
                .await?;
                Ok(())
            })
        })
        .await
    }

    async fn list_system_backoffs(&self) -> AppResult<HashMap<String, IndexerSystemBackoff>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT indexer_id, disabled_until, escalation_level FROM indexer_system_backoffs",
            &[],
        )
        .await?;

        let mut backoffs = HashMap::with_capacity(rows.len());
        for row in rows {
            let escalation_level = row.i64("escalation_level")?.max(0) as usize;
            backoffs.insert(
                row.text("indexer_id")?,
                IndexerSystemBackoff {
                    disabled_until: row.timestamp("disabled_until")?,
                    escalation_level,
                },
            );
        }
        Ok(backoffs)
    }

    async fn set_system_backoff(&self, id: &str, backoff: IndexerSystemBackoff) -> AppResult<()> {
        let id = id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "set_indexer_system_backoff", move |tx| {
            let id = id.clone();
            let backoff = backoff.clone();
            Box::pin(async move {
                let now = Utc::now();
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "INSERT INTO indexer_system_backoffs (
                        indexer_id, disabled_until, escalation_level, created_at, updated_at
                     ) VALUES ({}, {}, {}, {}, {})
                     ON CONFLICT(indexer_id) DO UPDATE SET
                        disabled_until = excluded.disabled_until,
                        escalation_level = excluded.escalation_level,
                        updated_at = excluded.updated_at",
                    &[
                        SqlArg::Text(id),
                        SqlArg::Timestamp(backoff.disabled_until),
                        SqlArg::I64(backoff.escalation_level as i64),
                        SqlArg::Timestamp(now),
                        SqlArg::Timestamp(now),
                    ],
                )
                .await?;
                Ok(())
            })
        })
        .await
    }

    async fn clear_system_backoff(&self, id: &str) -> AppResult<()> {
        let id = id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "clear_indexer_system_backoff", move |tx| {
            let id = id.clone();
            Box::pin(async move {
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "DELETE FROM indexer_system_backoffs WHERE indexer_id = {}",
                    &[SqlArg::Text(id)],
                )
                .await?;
                Ok(())
            })
        })
        .await
    }

    async fn update(&self, update: IndexerConfigUpdate) -> AppResult<IndexerConfig> {
        let encryption_key = self.encryption_key()?;
        let mut assignments = vec!["updated_at = {}".to_string()];
        let mut args = vec![SqlArg::Timestamp(Utc::now())];

        if let Some(name) = update.name.as_ref() {
            assignments.push("name = {}".to_string());
            args.push(SqlArg::Text(name.clone()));
        }
        if let Some(provider_type) = update.provider_type.as_ref() {
            assignments.push("provider_type = {}".to_string());
            args.push(SqlArg::Text(provider_type.clone()));
        }
        if let Some(base_url) = update.derived_base_url.as_ref() {
            assignments.push("base_url = {}".to_string());
            args.push(SqlArg::Text(base_url.clone()));
        }
        if let Some(rate_limit_seconds) = update.rate_limit_seconds {
            assignments.push("rate_limit_seconds = {}".to_string());
            args.push(SqlArg::I64(rate_limit_seconds));
        }
        if let Some(rate_limit_burst) = update.rate_limit_burst {
            assignments.push("rate_limit_burst = {}".to_string());
            args.push(SqlArg::I64(rate_limit_burst));
        }
        if let Some(is_enabled) = update.is_enabled {
            assignments.push("is_enabled = {}".to_string());
            args.push(SqlArg::Bool(is_enabled));
        }
        if let Some(enable_interactive_search) = update.enable_interactive_search {
            assignments.push("enable_interactive_search = {}".to_string());
            args.push(SqlArg::Bool(enable_interactive_search));
        }
        if let Some(enable_auto_search) = update.enable_auto_search {
            assignments.push("enable_auto_search = {}".to_string());
            args.push(SqlArg::Bool(enable_auto_search));
        }
        if let Some(proxy_config_id) = update.proxy_config_id.as_ref() {
            assignments.push("proxy_config_id = {}".to_string());
            args.push(SqlArg::OptText(proxy_config_id.clone()));
        }
        if let Some(download_client_id) = update.download_client_id.as_ref() {
            assignments.push("download_client_id = {}".to_string());
            args.push(SqlArg::OptText(download_client_id.clone()));
        }
        if let Some(seeding_profile_id) = update.seeding_profile_id.as_ref() {
            assignments.push("seeding_profile_id = {}".to_string());
            args.push(SqlArg::OptText(seeding_profile_id.clone()));
        }
        if let Some(managed_parent_config_id) = update.managed_parent_config_id.as_ref() {
            assignments.push("managed_parent_config_id = {}".to_string());
            args.push(SqlArg::OptText(managed_parent_config_id.clone()));
        }
        if let Some(managed_child_key) = update.managed_child_key.as_ref() {
            assignments.push("managed_child_key = {}".to_string());
            args.push(SqlArg::OptText(managed_child_key.clone()));
        }
        if let Some(managed_metadata_json) = update.managed_metadata_json.as_ref() {
            assignments.push("managed_metadata_json = {}".to_string());
            args.push(SqlArg::OptText(managed_metadata_json.clone()));
        }
        if let Some(caps_snapshot_json) = update.caps_snapshot_json.as_ref() {
            assignments.push("caps_snapshot_json = {}".to_string());
            args.push(SqlArg::OptText(caps_snapshot_json.clone()));
        }
        if let Some(config_json) = update.config_json.as_ref() {
            assignments.push("config_json = {}".to_string());
            args.push(SqlArg::Text(maybe_encrypt_value(
                encryption_key.as_ref(),
                config_json,
            )?));
        }

        if assignments.len() == 1 {
            return Err(AppError::Validation(
                "at least one indexer config field must be provided".into(),
            ));
        }

        let id = update.id.clone();
        args.push(SqlArg::Text(id.clone()));
        let sql = format!(
            "UPDATE indexers SET {} WHERE id = {{}}",
            assignments.join(", ")
        );
        SqlRuntime::run_in_transaction(&self.datastore, "update_indexer_config", move |tx| {
            let sql = sql.clone();
            let args = args.clone();
            let id = id.clone();
            let encryption_key = encryption_key.clone();
            Box::pin(async move {
                let rows = SqlRuntime::execute(SqlExec::Tx(tx), &sql, &args).await?;
                if rows == 0 {
                    return Err(AppError::NotFound(format!("indexer config {id}")));
                }
                fetch_optional_indexer(
                    SqlExec::Tx(tx),
                    &format!("SELECT {INDEXER_COLUMNS} FROM indexers WHERE id = {{}}"),
                    &[SqlArg::Text(id.clone())],
                    encryption_key.as_ref(),
                )
                .await?
                .ok_or_else(|| AppError::NotFound(format!("indexer config {id}")))
            })
        })
        .await
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        let id = id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "delete_indexer_config", move |tx| {
            let id = id.clone();
            Box::pin(async move {
                let rows = SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "DELETE FROM indexers WHERE id = {}",
                    &[SqlArg::Text(id.clone())],
                )
                .await?;
                if rows == 0 {
                    return Err(AppError::NotFound(format!("indexer config {id}")));
                }
                Ok(())
            })
        })
        .await
    }
}

fn indexer_insert_args(
    config: &IndexerConfig,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<Vec<SqlArg>> {
    let stored_config_json = maybe_encrypt_optional(encryption_key, config.config_json.as_ref())?;
    Ok(vec![
        SqlArg::Text(config.id.clone()),
        SqlArg::Text(config.name.clone()),
        SqlArg::Text(config.provider_type.clone()),
        SqlArg::Text(config.base_url.clone()),
        SqlArg::OptText(None),
        SqlArg::OptI64(config.rate_limit_seconds),
        SqlArg::OptI64(config.rate_limit_burst),
        SqlArg::OptTimestamp(config.disabled_until),
        SqlArg::Bool(config.is_enabled),
        SqlArg::Bool(config.enable_interactive_search),
        SqlArg::Bool(config.enable_auto_search),
        SqlArg::OptText(config.proxy_config_id.clone()),
        SqlArg::OptText(config.download_client_id.clone()),
        SqlArg::OptText(config.seeding_profile_id.clone()),
        SqlArg::OptText(config.managed_parent_config_id.clone()),
        SqlArg::OptText(config.managed_child_key.clone()),
        SqlArg::OptText(config.managed_metadata_json.clone()),
        SqlArg::OptText(config.caps_snapshot_json.clone()),
        SqlArg::OptText(config.last_health_status.clone()),
        SqlArg::OptText(config.last_error_message.clone()),
        SqlArg::OptTimestamp(config.last_error_at),
        SqlArg::OptText(stored_config_json),
        SqlArg::Timestamp(config.created_at),
        SqlArg::Timestamp(config.updated_at),
    ])
}

async fn fetch_indexers(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<Vec<IndexerConfig>> {
    SqlRuntime::fetch_all(exec, sql, args)
        .await?
        .into_iter()
        .map(|row| row_to_indexer_config(&row, encryption_key))
        .collect()
}

async fn fetch_optional_indexer(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<Option<IndexerConfig>> {
    SqlRuntime::fetch_optional(exec, sql, args)
        .await?
        .map(|row| row_to_indexer_config(&row, encryption_key))
        .transpose()
}

fn row_to_indexer_config(
    row: &SqlRow,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<IndexerConfig> {
    Ok(IndexerConfig {
        id: row.text("id")?,
        name: row.text("name")?,
        provider_type: row.text("provider_type")?,
        base_url: row.text("base_url")?,
        api_key_encrypted: decrypt_optional_value(
            encryption_key,
            row.opt_text("api_key_encrypted")?,
            "API key",
            false,
        )?,
        rate_limit_seconds: row.opt_i64("rate_limit_seconds")?,
        rate_limit_burst: row.opt_i64("rate_limit_burst")?,
        disabled_until: row.opt_timestamp("disabled_until")?,
        is_enabled: row.bool("is_enabled")?,
        enable_interactive_search: row.bool("enable_interactive_search")?,
        enable_auto_search: row.bool("enable_auto_search")?,
        proxy_config_id: row.opt_text("proxy_config_id")?,
        download_client_id: row.opt_text("download_client_id")?,
        seeding_profile_id: row.opt_text("seeding_profile_id")?,
        managed_parent_config_id: row.opt_text("managed_parent_config_id")?,
        managed_child_key: row.opt_text("managed_child_key")?,
        managed_metadata_json: row.opt_text("managed_metadata_json")?,
        caps_snapshot_json: row.opt_text("caps_snapshot_json")?,
        last_health_status: row.opt_text("last_health_status")?,
        last_error_message: row.opt_text("last_error_message")?,
        last_error_at: row.opt_timestamp("last_error_at")?,
        config_json: decrypt_optional_value(
            encryption_key,
            row.opt_text("config_json")?,
            "config_json",
            false,
        )?,
        created_at: row.timestamp("created_at")?,
        updated_at: row.timestamp("updated_at")?,
    })
}

fn legacy_indexer_connection_key(provider_type: &str) -> &'static str {
    if provider_type.eq_ignore_ascii_case("torrent_rss") {
        "feed_url"
    } else {
        "base_url"
    }
}

fn config_value_missing(value: Option<&JsonValue>) -> bool {
    match value {
        None | Some(JsonValue::Null) => true,
        Some(JsonValue::String(value)) => value.trim().is_empty(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use scryer_application::IndexerConfigRepository;
    use sqlx::sqlite::SqlitePoolOptions;

    use super::*;

    async fn create_test_indexers_table(pool: &sqlx::SqlitePool) {
        sqlx::query(
            "CREATE TABLE indexers (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                provider_type TEXT NOT NULL,
                base_url TEXT NOT NULL DEFAULT '',
                api_key_encrypted TEXT,
                rate_limit_seconds INTEGER,
                rate_limit_burst INTEGER,
                disabled_until TEXT,
                is_enabled INTEGER NOT NULL DEFAULT 1,
                enable_interactive_search INTEGER NOT NULL DEFAULT 1,
                enable_auto_search INTEGER NOT NULL DEFAULT 1,
                proxy_config_id TEXT,
                download_client_id TEXT,
                seeding_profile_id TEXT,
                managed_parent_config_id TEXT,
                managed_child_key TEXT,
                managed_metadata_json TEXT,
                caps_snapshot_json TEXT,
                last_health_status TEXT,
                last_error_message TEXT,
                last_error_at TEXT,
                config_json TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
        )
        .execute(pool)
        .await
        .expect("indexers table should be created");
    }

    async fn create_test_indexer_system_backoffs_table(pool: &sqlx::SqlitePool) {
        sqlx::query(
            "CREATE TABLE indexer_system_backoffs (
                indexer_id TEXT PRIMARY KEY NOT NULL,
                disabled_until TEXT NOT NULL,
                escalation_level INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            )",
        )
        .execute(pool)
        .await
        .expect("system backoffs table should be created");
    }

    #[tokio::test]
    async fn record_last_error_persists_message_without_changing_updated_at() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");
        create_test_indexers_table(&pool).await;

        let created_at = chrono::DateTime::parse_from_rfc3339("2026-01-02T03:04:05Z")
            .expect("created timestamp should parse")
            .with_timezone(&Utc);
        let updated_at = chrono::DateTime::parse_from_rfc3339("2026-01-03T03:04:05Z")
            .expect("updated timestamp should parse")
            .with_timezone(&Utc);
        sqlx::query(
            "INSERT INTO indexers (
                id, name, provider_type, base_url, api_key_encrypted, rate_limit_seconds,
                rate_limit_burst, disabled_until, is_enabled, enable_interactive_search,
                enable_auto_search, last_health_status, last_error_at, config_json, created_at,
                updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind("idx-error")
        .bind("Erroring Indexer")
        .bind("newznab")
        .bind("")
        .bind(None::<String>)
        .bind(None::<i64>)
        .bind(None::<i64>)
        .bind(None::<String>)
        .bind(1_i64)
        .bind(1_i64)
        .bind(1_i64)
        .bind(None::<String>)
        .bind(None::<String>)
        .bind(None::<String>)
        .bind(created_at.to_rfc3339())
        .bind(updated_at.to_rfc3339())
        .execute(&pool)
        .await
        .expect("indexer row should insert");

        let store = IndexerConfigStore::new(
            StoreDatastore::Sqlite {
                pool,
                writer_gate: Arc::new(tokio::sync::Mutex::new(())),
            },
            Arc::new(RwLock::new(None)),
        );

        store
            .record_last_error(
                "idx-error",
                Some("HTTP 429: Indexer query limit reached; retry after 321s".to_string()),
            )
            .await
            .expect("record should succeed");

        let config = store
            .get_by_id("idx-error")
            .await
            .expect("query should succeed")
            .expect("config should exist");
        assert!(config.last_error_at.is_some());
        assert_eq!(
            config.last_error_message.as_deref(),
            Some("HTTP 429: Indexer query limit reached; retry after 321s")
        );
        assert_eq!(config.updated_at, updated_at);
    }

    #[tokio::test]
    async fn clear_last_error_clears_error_fields_without_changing_updated_at() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");
        create_test_indexers_table(&pool).await;

        let created_at = chrono::DateTime::parse_from_rfc3339("2026-01-02T03:04:05Z")
            .expect("created timestamp should parse")
            .with_timezone(&Utc);
        let updated_at = chrono::DateTime::parse_from_rfc3339("2026-01-03T03:04:05Z")
            .expect("updated timestamp should parse")
            .with_timezone(&Utc);
        let error_at = chrono::DateTime::parse_from_rfc3339("2026-01-04T03:04:05Z")
            .expect("error timestamp should parse")
            .with_timezone(&Utc);
        sqlx::query(
            "INSERT INTO indexers (
                id, name, provider_type, base_url, api_key_encrypted, rate_limit_seconds,
                rate_limit_burst, disabled_until, is_enabled, enable_interactive_search,
                enable_auto_search, last_health_status, last_error_message, last_error_at,
                config_json, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind("idx-recovered")
        .bind("Recovered Indexer")
        .bind("newznab")
        .bind("")
        .bind(None::<String>)
        .bind(None::<i64>)
        .bind(None::<i64>)
        .bind(None::<String>)
        .bind(1_i64)
        .bind(1_i64)
        .bind(1_i64)
        .bind(Some("Last search failed"))
        .bind(Some("HTTP 429: query limit reached"))
        .bind(Some(error_at.to_rfc3339()))
        .bind(None::<String>)
        .bind(created_at.to_rfc3339())
        .bind(updated_at.to_rfc3339())
        .execute(&pool)
        .await
        .expect("indexer row should insert");

        let store = IndexerConfigStore::new(
            StoreDatastore::Sqlite {
                pool,
                writer_gate: Arc::new(tokio::sync::Mutex::new(())),
            },
            Arc::new(RwLock::new(None)),
        );

        store
            .clear_last_error("idx-recovered")
            .await
            .expect("clear should succeed");

        let config = store
            .get_by_id("idx-recovered")
            .await
            .expect("query should succeed")
            .expect("config should exist");
        assert!(config.last_error_at.is_none());
        assert!(config.last_error_message.is_none());
        assert!(config.last_health_status.is_none());
        assert_eq!(config.updated_at, updated_at);
    }

    #[tokio::test]
    async fn system_backoff_methods_do_not_change_config_disabled_until() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");
        create_test_indexers_table(&pool).await;
        create_test_indexer_system_backoffs_table(&pool).await;

        let now = Utc::now();
        let config_disabled_until = now + chrono::Duration::hours(6);
        let system_disabled_until = now + chrono::Duration::minutes(5);
        sqlx::query(
            "INSERT INTO indexers (
                id, name, provider_type, base_url, api_key_encrypted, rate_limit_seconds,
                rate_limit_burst, disabled_until, is_enabled, enable_interactive_search,
                enable_auto_search, last_health_status, last_error_at, config_json, created_at,
                updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind("idx-system-backoff")
        .bind("System Backoff Indexer")
        .bind("newznab")
        .bind("")
        .bind(None::<String>)
        .bind(None::<i64>)
        .bind(None::<i64>)
        .bind(Some(config_disabled_until.to_rfc3339()))
        .bind(1_i64)
        .bind(1_i64)
        .bind(1_i64)
        .bind(None::<String>)
        .bind(None::<String>)
        .bind(None::<String>)
        .bind(now.to_rfc3339())
        .bind(now.to_rfc3339())
        .execute(&pool)
        .await
        .expect("indexer row should insert");

        let store = IndexerConfigStore::new(
            StoreDatastore::Sqlite {
                pool,
                writer_gate: Arc::new(tokio::sync::Mutex::new(())),
            },
            Arc::new(RwLock::new(None)),
        );

        store
            .set_system_backoff(
                "idx-system-backoff",
                IndexerSystemBackoff {
                    disabled_until: system_disabled_until,
                    escalation_level: 3,
                },
            )
            .await
            .expect("system backoff should be persisted");

        let backoffs = store
            .list_system_backoffs()
            .await
            .expect("system backoffs should load");
        assert_eq!(
            backoffs.get("idx-system-backoff"),
            Some(&IndexerSystemBackoff {
                disabled_until: system_disabled_until,
                escalation_level: 3,
            })
        );

        let config = store
            .get_by_id("idx-system-backoff")
            .await
            .expect("config should load")
            .expect("config should exist");
        assert_eq!(config.disabled_until, Some(config_disabled_until));

        store
            .clear_system_backoff("idx-system-backoff")
            .await
            .expect("system backoff should clear");
        assert!(
            store
                .list_system_backoffs()
                .await
                .expect("system backoffs should load")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn migration_clears_legacy_api_key_when_config_json_already_has_it() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");
        create_test_indexers_table(&pool).await;

        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO indexers (
                id, name, provider_type, base_url, api_key_encrypted, rate_limit_seconds,
                rate_limit_burst, disabled_until, is_enabled, enable_interactive_search,
                enable_auto_search, last_health_status, last_error_at, config_json, created_at,
                updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind("idx-1")
        .bind("Test Indexer")
        .bind("newznab")
        .bind("")
        .bind("legacy-secret")
        .bind(None::<i64>)
        .bind(None::<i64>)
        .bind(None::<String>)
        .bind(1_i64)
        .bind(1_i64)
        .bind(1_i64)
        .bind(None::<String>)
        .bind(None::<String>)
        .bind(r#"{"api_key":"config-secret"}"#)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("legacy row should insert");

        let store = IndexerConfigStore::new(
            StoreDatastore::Sqlite {
                pool,
                writer_gate: Arc::new(tokio::sync::Mutex::new(())),
            },
            Arc::new(RwLock::new(None)),
        );
        let migrated = store
            .migrate_legacy_indexer_config_sources()
            .await
            .expect("migration should succeed");
        assert_eq!(migrated, 1);

        let config = store
            .get_by_id("idx-1")
            .await
            .expect("query should succeed")
            .expect("config should exist");
        assert_eq!(
            config.config_json.as_deref(),
            Some(r#"{"api_key":"config-secret"}"#)
        );
        assert_eq!(config.api_key_encrypted, None);
    }

    #[tokio::test]
    async fn download_client_mapping_update_preserves_and_clears_nested_optional_value() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");
        create_test_indexers_table(&pool).await;

        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO indexers (
                id, name, provider_type, base_url, is_enabled, enable_interactive_search,
                enable_auto_search, download_client_id, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind("idx-mapped")
        .bind("Mapped Indexer")
        .bind("newznab")
        .bind("")
        .bind(1_i64)
        .bind(1_i64)
        .bind(1_i64)
        .bind("client-1")
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("mapped indexer should insert");

        let store = IndexerConfigStore::new(
            StoreDatastore::Sqlite {
                pool,
                writer_gate: Arc::new(tokio::sync::Mutex::new(())),
            },
            Arc::new(RwLock::new(None)),
        );

        let preserved = store
            .update(IndexerConfigUpdate {
                id: "idx-mapped".to_string(),
                name: Some("Renamed Indexer".to_string()),
                download_client_id: None,
                ..Default::default()
            })
            .await
            .expect("unrelated update should succeed");
        assert_eq!(preserved.download_client_id.as_deref(), Some("client-1"));

        let cleared = store
            .update(IndexerConfigUpdate {
                id: "idx-mapped".to_string(),
                download_client_id: Some(None),
                ..Default::default()
            })
            .await
            .expect("mapping clear should succeed");
        assert_eq!(cleared.download_client_id, None);
    }
}

use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use scryer_application::{AppError, AppResult, IndexerProxyConfigRepository};
use scryer_domain::{
    ChallengeSolverProtocol, IndexerProxyConfig, IndexerProxyHealthStatus, IndexerProxyKind,
    IndexerProxyProviderType,
};

use crate::config_store::{current_encryption_key, decrypt_optional_value, encrypt_optional_value};
use crate::encryption::EncryptionKey;
use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, StoreDatastore};

const INDEXER_PROXY_COLUMNS: &str = "id, name, provider_type, protocol, base_url,
    request_timeout_seconds, is_enabled, username_encrypted, password_encrypted, remote_dns,
    last_health_status, last_error_message, last_error_at, created_at, updated_at";

const INDEXER_PROXY_INSERT_SQL: &str = "INSERT INTO indexer_proxy_configs (
    id, name, provider_type, protocol, base_url, request_timeout_seconds, is_enabled,
    username_encrypted, password_encrypted, remote_dns,
    last_health_status, last_error_message, last_error_at, created_at, updated_at
) VALUES (
    {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}
)";

/// Label used by the shared at-rest encryption helpers for proxy credentials.
const PROXY_CREDENTIAL_LABEL: &str = "indexer proxy credential";

#[derive(Clone)]
pub struct IndexerProxyConfigStore {
    datastore: StoreDatastore,
    encryption_key: Arc<RwLock<Option<EncryptionKey>>>,
}

impl IndexerProxyConfigStore {
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
impl IndexerProxyConfigRepository for IndexerProxyConfigStore {
    async fn list(
        &self,
        provider_type: Option<IndexerProxyProviderType>,
    ) -> AppResult<Vec<IndexerProxyConfig>> {
        let (sql, args) = match provider_type {
            Some(provider_type) => (
                format!(
                    "SELECT {INDEXER_PROXY_COLUMNS} FROM indexer_proxy_configs WHERE provider_type = {{}} ORDER BY created_at DESC"
                ),
                vec![SqlArg::Text(provider_type.as_str().to_string())],
            ),
            None => (
                format!(
                    "SELECT {INDEXER_PROXY_COLUMNS} FROM indexer_proxy_configs ORDER BY created_at DESC"
                ),
                Vec::new(),
            ),
        };
        let encryption_key = self.encryption_key()?;
        fetch_proxy_configs(
            self.datastore.read_exec(),
            &sql,
            &args,
            encryption_key.as_ref(),
        )
        .await
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<IndexerProxyConfig>> {
        let encryption_key = self.encryption_key()?;
        fetch_optional_proxy_config(
            self.datastore.read_exec(),
            &format!("SELECT {INDEXER_PROXY_COLUMNS} FROM indexer_proxy_configs WHERE id = {{}}"),
            &[SqlArg::Text(id.to_string())],
            encryption_key.as_ref(),
        )
        .await
    }

    async fn create(&self, config: IndexerProxyConfig) -> AppResult<IndexerProxyConfig> {
        let encryption_key = self.encryption_key()?;
        let args = proxy_insert_args(&config, encryption_key.as_ref())?;
        SqlRuntime::run_in_transaction(&self.datastore, "create_indexer_proxy_config", move |tx| {
            let config = config.clone();
            let args = args.clone();
            Box::pin(async move {
                SqlRuntime::execute(SqlExec::Tx(tx), INDEXER_PROXY_INSERT_SQL, &args).await?;
                Ok(config)
            })
        })
        .await
    }

    async fn update(&self, config: IndexerProxyConfig) -> AppResult<IndexerProxyConfig> {
        let encryption_key = self.encryption_key()?;
        let args = vec![
            SqlArg::Text(config.name.clone()),
            SqlArg::Text(config.provider_type.as_str().to_string()),
            SqlArg::OptText(
                config
                    .protocol
                    .map(|protocol| protocol.as_str().to_string()),
            ),
            SqlArg::Text(config.base_url.clone()),
            SqlArg::I64(i64::from(config.request_timeout_seconds)),
            SqlArg::Bool(config.is_enabled),
            SqlArg::OptText(encrypt_credential(
                encryption_key.as_ref(),
                config.username_encrypted.as_ref(),
            )?),
            SqlArg::OptText(encrypt_credential(
                encryption_key.as_ref(),
                config.password_encrypted.as_ref(),
            )?),
            SqlArg::Bool(config.remote_dns),
            SqlArg::OptText(
                config
                    .last_health_status
                    .map(|status| status.as_str().to_string()),
            ),
            SqlArg::OptText(config.last_error_message.clone()),
            SqlArg::OptTimestamp(config.last_error_at),
            SqlArg::Timestamp(config.updated_at),
            SqlArg::Text(config.id.clone()),
        ];
        SqlRuntime::run_in_transaction(&self.datastore, "update_indexer_proxy_config", move |tx| {
            let config = config.clone();
            let args = args.clone();
            Box::pin(async move {
                let rows = SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "UPDATE indexer_proxy_configs SET
                            name = {}, provider_type = {}, protocol = {}, base_url = {},
                            request_timeout_seconds = {}, is_enabled = {},
                            username_encrypted = {}, password_encrypted = {}, remote_dns = {},
                            last_health_status = {}, last_error_message = {},
                            last_error_at = {}, updated_at = {}
                         WHERE id = {}",
                    &args,
                )
                .await?;
                if rows == 0 {
                    return Err(AppError::NotFound(format!(
                        "indexer proxy config {}",
                        config.id
                    )));
                }
                Ok(config)
            })
        })
        .await
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        let id = id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "delete_indexer_proxy_config", move |tx| {
            let id = id.clone();
            Box::pin(async move {
                let rows = SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "DELETE FROM indexer_proxy_configs WHERE id = {}",
                    &[SqlArg::Text(id.clone())],
                )
                .await?;
                if rows == 0 {
                    return Err(AppError::NotFound(format!("indexer proxy config {id}")));
                }
                Ok(())
            })
        })
        .await
    }

    async fn record_health(
        &self,
        id: &str,
        status: IndexerProxyHealthStatus,
        error_message: Option<String>,
        error_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> AppResult<()> {
        // Health is observational state: deliberately leave `updated_at`
        // untouched so plugin client cache revisions only change on config
        // edits.
        let rows = SqlRuntime::execute_write(
            &self.datastore,
            "record_indexer_proxy_health",
            "UPDATE indexer_proxy_configs SET
                    last_health_status = {}, last_error_message = {}, last_error_at = {}
                 WHERE id = {}",
            vec![
                SqlArg::Text(status.as_str().to_string()),
                SqlArg::OptText(error_message),
                SqlArg::OptTimestamp(error_at),
                SqlArg::Text(id.to_string()),
            ],
        )
        .await?;
        if rows == 0 {
            return Err(AppError::NotFound(format!("indexer proxy config {id}")));
        }
        Ok(())
    }
}

/// Encrypt a proxy credential for storage. Mirrors the indexer API key: the
/// in-memory field holds plaintext and only the persisted column is encrypted.
fn encrypt_credential(
    key: Option<&EncryptionKey>,
    value: Option<&String>,
) -> AppResult<Option<String>> {
    encrypt_optional_value(key, value, PROXY_CREDENTIAL_LABEL, false)
}

fn decrypt_credential(
    key: Option<&EncryptionKey>,
    value: Option<String>,
) -> AppResult<Option<String>> {
    decrypt_optional_value(key, value, PROXY_CREDENTIAL_LABEL, false)
}

fn proxy_insert_args(
    config: &IndexerProxyConfig,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<Vec<SqlArg>> {
    Ok(vec![
        SqlArg::Text(config.id.clone()),
        SqlArg::Text(config.name.clone()),
        SqlArg::Text(config.provider_type.as_str().to_string()),
        SqlArg::OptText(
            config
                .protocol
                .map(|protocol| protocol.as_str().to_string()),
        ),
        SqlArg::Text(config.base_url.clone()),
        SqlArg::I64(i64::from(config.request_timeout_seconds)),
        SqlArg::Bool(config.is_enabled),
        SqlArg::OptText(encrypt_credential(
            encryption_key,
            config.username_encrypted.as_ref(),
        )?),
        SqlArg::OptText(encrypt_credential(
            encryption_key,
            config.password_encrypted.as_ref(),
        )?),
        SqlArg::Bool(config.remote_dns),
        SqlArg::OptText(
            config
                .last_health_status
                .map(|status| status.as_str().to_string()),
        ),
        SqlArg::OptText(config.last_error_message.clone()),
        SqlArg::OptTimestamp(config.last_error_at),
        SqlArg::Timestamp(config.created_at),
        SqlArg::Timestamp(config.updated_at),
    ])
}

async fn fetch_proxy_configs(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<Vec<IndexerProxyConfig>> {
    SqlRuntime::fetch_all(exec, sql, args)
        .await?
        .into_iter()
        .map(|row| row_to_proxy_config(&row, encryption_key))
        .collect()
}

async fn fetch_optional_proxy_config(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<Option<IndexerProxyConfig>> {
    SqlRuntime::fetch_optional(exec, sql, args)
        .await?
        .map(|row| row_to_proxy_config(&row, encryption_key))
        .transpose()
}

/// Resolve the stored `protocol` column.
///
/// A challenge solver has no meaning without one, so a missing value there is
/// a corrupt row. A transport proxy speaks no solver protocol at all, so NULL
/// is the correct reading and a stray value is ignored rather than rejected —
/// downgrading a transport row must not make it unloadable.
fn parse_persisted_protocol(
    provider_type: IndexerProxyProviderType,
    stored: Option<String>,
) -> AppResult<Option<ChallengeSolverProtocol>> {
    match provider_type.kind() {
        IndexerProxyKind::Transport => Ok(None),
        IndexerProxyKind::ChallengeSolver => {
            let raw = stored.ok_or_else(|| {
                AppError::Repository(format!(
                    "indexer proxy provider '{}' requires a solver protocol",
                    provider_type.as_str()
                ))
            })?;
            ChallengeSolverProtocol::parse(&raw)
                .ok_or_else(|| {
                    AppError::Repository(format!("unknown indexer proxy protocol '{raw}'"))
                })
                .map(Some)
        }
    }
}

fn row_to_proxy_config(
    row: &SqlRow,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<IndexerProxyConfig> {
    let provider_type = row.text("provider_type")?;
    let provider_type = IndexerProxyProviderType::parse(&provider_type).ok_or_else(|| {
        AppError::Repository(format!(
            "unknown indexer proxy provider type '{provider_type}'"
        ))
    })?;
    let last_health_status = row
        .opt_text("last_health_status")?
        .as_deref()
        .and_then(IndexerProxyHealthStatus::parse);

    Ok(IndexerProxyConfig {
        id: row.text("id")?,
        name: row.text("name")?,
        provider_type,
        protocol: parse_persisted_protocol(provider_type, row.opt_text("protocol")?)?,
        base_url: row.text("base_url")?,
        request_timeout_seconds: clamp_persisted_proxy_timeout(row.i64("request_timeout_seconds")?),
        is_enabled: row.bool("is_enabled")?,
        username_encrypted: decrypt_credential(
            encryption_key,
            row.opt_text("username_encrypted")?,
        )?,
        password_encrypted: decrypt_credential(
            encryption_key,
            row.opt_text("password_encrypted")?,
        )?,
        remote_dns: row.bool("remote_dns")?,
        last_health_status,
        last_error_message: row.opt_text("last_error_message")?,
        last_error_at: row.opt_timestamp("last_error_at")?,
        created_at: row.timestamp("created_at")?,
        updated_at: row.timestamp("updated_at")?,
    })
}

fn clamp_persisted_proxy_timeout(timeout_seconds: i64) -> u32 {
    timeout_seconds.clamp(
        1,
        i64::from(scryer_outbound_http::MAX_INDEXER_PROXY_TIMEOUT_SECONDS),
    ) as u32
}

#[cfg(test)]
mod tests {
    use super::{clamp_persisted_proxy_timeout, parse_persisted_protocol};
    use scryer_domain::{ChallengeSolverProtocol, IndexerProxyProviderType};

    #[test]
    fn persisted_proxy_timeout_is_clamped_to_supported_range() {
        assert_eq!(clamp_persisted_proxy_timeout(-1), 1);
        assert_eq!(clamp_persisted_proxy_timeout(60), 60);
        assert_eq!(clamp_persisted_proxy_timeout(180), 120);
    }

    #[test]
    fn solver_rows_still_require_a_protocol() {
        assert_eq!(
            parse_persisted_protocol(
                IndexerProxyProviderType::Trawl,
                Some("request_solution_v1".to_string()),
            )
            .expect("existing solver rows must keep loading"),
            Some(ChallengeSolverProtocol::RequestSolutionV1)
        );
        assert!(parse_persisted_protocol(IndexerProxyProviderType::Byparr, None).is_err());
        assert!(
            parse_persisted_protocol(
                IndexerProxyProviderType::Byparr,
                Some("nonsense".to_string()),
            )
            .is_err()
        );
    }

    #[test]
    fn transport_rows_read_a_null_protocol_as_none() {
        assert_eq!(
            parse_persisted_protocol(IndexerProxyProviderType::Socks5, None)
                .expect("transport rows carry no protocol"),
            None
        );
        assert_eq!(
            parse_persisted_protocol(
                IndexerProxyProviderType::Http,
                Some("request_solution_v1".to_string()),
            )
            .expect("a stray protocol must not make a transport row unloadable"),
            None
        );
    }
}

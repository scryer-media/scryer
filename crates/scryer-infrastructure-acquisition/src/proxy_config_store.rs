use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use scryer_application::{AppError, AppResult, ProxyConfigRepository};
use scryer_domain::{
    ChallengeSolverProtocol, ProxyConfig, ProxyHealthStatus, ProxyKind, ProxyProviderType,
};

use crate::config_store::{current_encryption_key, decrypt_optional_value, encrypt_optional_value};
use crate::encryption::EncryptionKey;
use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, StoreDatastore};

const PROXY_COLUMNS: &str = "id, name, provider_type, protocol, base_url,
    request_timeout_seconds, is_enabled, username_encrypted, password_encrypted, remote_dns,
    private_key_encrypted, private_key_passphrase_encrypted,
    peer_public_key, preshared_key_encrypted, tunnel_public_key,
    tunnel_addresses, tunnel_dns_servers, tunnel_mtu, tunnel_keepalive_seconds,
    host_key_fingerprint, host_key_pinned_at,
    last_health_status, last_error_message, last_error_at, created_at, updated_at";

const PROXY_INSERT_SQL: &str = "INSERT INTO proxy_configs (
    id, name, provider_type, protocol, base_url, request_timeout_seconds, is_enabled,
    username_encrypted, password_encrypted, remote_dns,
    private_key_encrypted, private_key_passphrase_encrypted,
    peer_public_key, preshared_key_encrypted, tunnel_public_key,
    tunnel_addresses, tunnel_dns_servers, tunnel_mtu, tunnel_keepalive_seconds,
    host_key_fingerprint, host_key_pinned_at,
    last_health_status, last_error_message, last_error_at, created_at, updated_at
) VALUES (
    {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {},
    {}, {}, {}
)";

/// Label used by the shared at-rest encryption helpers for proxy credentials.
const PROXY_CREDENTIAL_LABEL: &str = "proxy credential";

/// Separate label so a key failure is not reported as a credential failure.
const PROXY_PRIVATE_KEY_LABEL: &str = "proxy private key";

/// A WireGuard preshared key is a symmetric secret and is encrypted at rest
/// like the private key, under its own label so a decrypt failure names the
/// field the operator has to go and fix.
const PROXY_PRESHARED_KEY_LABEL: &str = "proxy preshared key";

#[derive(Clone)]
pub struct ProxyConfigStore {
    datastore: StoreDatastore,
    encryption_key: Arc<RwLock<Option<EncryptionKey>>>,
}

impl ProxyConfigStore {
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
impl ProxyConfigRepository for ProxyConfigStore {
    async fn list(&self, provider_type: Option<ProxyProviderType>) -> AppResult<Vec<ProxyConfig>> {
        let (sql, args) = match provider_type {
            Some(provider_type) => (
                format!(
                    "SELECT {PROXY_COLUMNS} FROM proxy_configs WHERE provider_type = {{}} ORDER BY created_at DESC"
                ),
                vec![SqlArg::Text(provider_type.as_str().to_string())],
            ),
            None => (
                format!("SELECT {PROXY_COLUMNS} FROM proxy_configs ORDER BY created_at DESC"),
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

    async fn get_by_id(&self, id: &str) -> AppResult<Option<ProxyConfig>> {
        let encryption_key = self.encryption_key()?;
        fetch_optional_proxy_config(
            self.datastore.read_exec(),
            &format!("SELECT {PROXY_COLUMNS} FROM proxy_configs WHERE id = {{}}"),
            &[SqlArg::Text(id.to_string())],
            encryption_key.as_ref(),
        )
        .await
    }

    async fn create(&self, config: ProxyConfig) -> AppResult<ProxyConfig> {
        let encryption_key = self.encryption_key()?;
        let args = proxy_insert_args(&config, encryption_key.as_ref())?;
        SqlRuntime::run_in_transaction(&self.datastore, "create_proxy_config", move |tx| {
            let config = config.clone();
            let args = args.clone();
            Box::pin(async move {
                SqlRuntime::execute(SqlExec::Tx(tx), PROXY_INSERT_SQL, &args).await?;
                Ok(config)
            })
        })
        .await
    }

    async fn update(&self, config: ProxyConfig) -> AppResult<ProxyConfig> {
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
            SqlArg::OptText(encrypt_private_key(
                encryption_key.as_ref(),
                config.private_key_encrypted.as_ref(),
            )?),
            SqlArg::OptText(encrypt_private_key(
                encryption_key.as_ref(),
                config.private_key_passphrase_encrypted.as_ref(),
            )?),
            SqlArg::OptText(config.peer_public_key.clone()),
            SqlArg::OptText(encrypt_preshared_key(
                encryption_key.as_ref(),
                config.preshared_key_encrypted.as_ref(),
            )?),
            SqlArg::OptText(config.tunnel_public_key.clone()),
            SqlArg::OptText(join_tunnel_list(&config.tunnel_addresses)),
            SqlArg::OptText(join_tunnel_list(&config.tunnel_dns_servers)),
            SqlArg::OptI64(config.tunnel_mtu.map(i64::from)),
            SqlArg::OptI64(config.tunnel_keepalive_seconds.map(i64::from)),
            SqlArg::OptText(config.host_key_fingerprint.clone()),
            SqlArg::OptTimestamp(config.host_key_pinned_at),
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
        SqlRuntime::run_in_transaction(&self.datastore, "update_proxy_config", move |tx| {
            let config = config.clone();
            let args = args.clone();
            Box::pin(async move {
                let rows = SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "UPDATE proxy_configs SET
                            name = {}, provider_type = {}, protocol = {}, base_url = {},
                            request_timeout_seconds = {}, is_enabled = {},
                            username_encrypted = {}, password_encrypted = {}, remote_dns = {},
                            private_key_encrypted = {},
                            private_key_passphrase_encrypted = {},
                            peer_public_key = {}, preshared_key_encrypted = {},
                            tunnel_public_key = {}, tunnel_addresses = {},
                            tunnel_dns_servers = {}, tunnel_mtu = {},
                            tunnel_keepalive_seconds = {},
                            host_key_fingerprint = {}, host_key_pinned_at = {},
                            last_health_status = {}, last_error_message = {},
                            last_error_at = {}, updated_at = {}
                         WHERE id = {}",
                    &args,
                )
                .await?;
                if rows == 0 {
                    return Err(AppError::NotFound(format!("proxy config {}", config.id)));
                }
                Ok(config)
            })
        })
        .await
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        let id = id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "delete_proxy_config", move |tx| {
            let id = id.clone();
            Box::pin(async move {
                let rows = SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "DELETE FROM proxy_configs WHERE id = {}",
                    &[SqlArg::Text(id.clone())],
                )
                .await?;
                if rows == 0 {
                    return Err(AppError::NotFound(format!("proxy config {id}")));
                }
                Ok(())
            })
        })
        .await
    }

    async fn record_health(
        &self,
        id: &str,
        status: ProxyHealthStatus,
        error_message: Option<String>,
        error_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> AppResult<()> {
        // Health is observational state: deliberately leave `updated_at`
        // untouched so plugin client cache revisions only change on config
        // edits.
        let rows = SqlRuntime::execute_write(
            &self.datastore,
            "record_proxy_health",
            "UPDATE proxy_configs SET
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
            return Err(AppError::NotFound(format!("proxy config {id}")));
        }
        Ok(())
    }

    async fn pin_host_key(
        &self,
        id: &str,
        fingerprint: &str,
        pinned_at: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<()> {
        // A host key is public, so it is stored in the clear. `updated_at` is
        // left alone on purpose: the engine pins right after the connect that
        // succeeded, and bumping the cache revision there would evict that very
        // session on every first connect.
        let rows = SqlRuntime::execute_write(
            &self.datastore,
            "pin_proxy_host_key",
            "UPDATE proxy_configs SET
                    host_key_fingerprint = {}, host_key_pinned_at = {}
                 WHERE id = {}",
            vec![
                SqlArg::Text(fingerprint.to_string()),
                SqlArg::Timestamp(pinned_at),
                SqlArg::Text(id.to_string()),
            ],
        )
        .await?;
        if rows == 0 {
            return Err(AppError::NotFound(format!("proxy config {id}")));
        }
        Ok(())
    }

    async fn clear_host_key(&self, id: &str) -> AppResult<()> {
        // The opposite call: an operator asking for a fresh trust decision
        // after a legitimate rekey. `updated_at` *is* bumped so any cached
        // client or session built on the old pin is evicted and the next
        // connect really is a new one.
        let rows = SqlRuntime::execute_write(
            &self.datastore,
            "clear_proxy_host_key",
            "UPDATE proxy_configs SET
                    host_key_fingerprint = NULL, host_key_pinned_at = NULL, updated_at = {}
                 WHERE id = {}",
            vec![
                SqlArg::Timestamp(chrono::Utc::now()),
                SqlArg::Text(id.to_string()),
            ],
        )
        .await?;
        if rows == 0 {
            return Err(AppError::NotFound(format!("proxy config {id}")));
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

/// Tunnel key material follows the same at-rest convention as the credentials:
/// the in-memory field is plaintext PEM, the column is ciphertext.
fn encrypt_private_key(
    key: Option<&EncryptionKey>,
    value: Option<&String>,
) -> AppResult<Option<String>> {
    encrypt_optional_value(key, value, PROXY_PRIVATE_KEY_LABEL, false)
}

fn decrypt_private_key(
    key: Option<&EncryptionKey>,
    value: Option<String>,
) -> AppResult<Option<String>> {
    decrypt_optional_value(key, value, PROXY_PRIVATE_KEY_LABEL, false)
}

fn encrypt_preshared_key(
    key: Option<&EncryptionKey>,
    value: Option<&String>,
) -> AppResult<Option<String>> {
    encrypt_optional_value(key, value, PROXY_PRESHARED_KEY_LABEL, false)
}

fn decrypt_preshared_key(
    key: Option<&EncryptionKey>,
    value: Option<String>,
) -> AppResult<Option<String>> {
    decrypt_optional_value(key, value, PROXY_PRESHARED_KEY_LABEL, false)
}

/// Store an address or DNS list as the comma-separated text the column holds.
///
/// An empty list is NULL rather than an empty string, so "the operator cleared
/// this" and "this row predates WireGuard" read the same way — which they
/// should, because neither carries a list.
fn join_tunnel_list(values: &[String]) -> Option<String> {
    let joined = values
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(",");
    (!joined.is_empty()).then_some(joined)
}

/// The inverse. Tolerant on read: blank entries are dropped rather than
/// rejected, because a stored list is only ever as good as what was pasted and
/// an unloadable row is the worse failure.
fn split_tunnel_list(stored: Option<String>) -> Vec<String> {
    stored
        .as_deref()
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

/// Read a stored MTU or keepalive back into the `u16` the domain carries.
///
/// Out-of-range values are read as "no opinion" rather than as an error: the
/// engine's own validation owns the range, and a nonsense integer must not make
/// the row unloadable.
fn opt_u16_from_stored(stored: Option<i64>) -> Option<u16> {
    stored.and_then(|value| u16::try_from(value).ok())
}

fn proxy_insert_args(
    config: &ProxyConfig,
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
        SqlArg::OptText(encrypt_private_key(
            encryption_key,
            config.private_key_encrypted.as_ref(),
        )?),
        SqlArg::OptText(encrypt_private_key(
            encryption_key,
            config.private_key_passphrase_encrypted.as_ref(),
        )?),
        SqlArg::OptText(config.peer_public_key.clone()),
        SqlArg::OptText(encrypt_preshared_key(
            encryption_key,
            config.preshared_key_encrypted.as_ref(),
        )?),
        SqlArg::OptText(config.tunnel_public_key.clone()),
        SqlArg::OptText(join_tunnel_list(&config.tunnel_addresses)),
        SqlArg::OptText(join_tunnel_list(&config.tunnel_dns_servers)),
        SqlArg::OptI64(config.tunnel_mtu.map(i64::from)),
        SqlArg::OptI64(config.tunnel_keepalive_seconds.map(i64::from)),
        SqlArg::OptText(config.host_key_fingerprint.clone()),
        SqlArg::OptTimestamp(config.host_key_pinned_at),
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
) -> AppResult<Vec<ProxyConfig>> {
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
) -> AppResult<Option<ProxyConfig>> {
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
    provider_type: ProxyProviderType,
    stored: Option<String>,
) -> AppResult<Option<ChallengeSolverProtocol>> {
    match provider_type.kind() {
        ProxyKind::Transport | ProxyKind::Tunnel => Ok(None),
        ProxyKind::ChallengeSolver => {
            let raw = stored.ok_or_else(|| {
                AppError::Repository(format!(
                    "proxy provider '{}' requires a solver protocol",
                    provider_type.as_str()
                ))
            })?;
            ChallengeSolverProtocol::parse(&raw)
                .ok_or_else(|| AppError::Repository(format!("unknown proxy protocol '{raw}'")))
                .map(Some)
        }
    }
}

fn row_to_proxy_config(
    row: &SqlRow,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<ProxyConfig> {
    let provider_type = row.text("provider_type")?;
    let provider_type = ProxyProviderType::parse(&provider_type).ok_or_else(|| {
        AppError::Repository(format!("unknown proxy provider type '{provider_type}'"))
    })?;
    let last_health_status = row
        .opt_text("last_health_status")?
        .as_deref()
        .and_then(ProxyHealthStatus::parse);

    Ok(ProxyConfig {
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
        private_key_encrypted: decrypt_private_key(
            encryption_key,
            row.opt_text("private_key_encrypted")?,
        )?,
        private_key_passphrase_encrypted: decrypt_private_key(
            encryption_key,
            row.opt_text("private_key_passphrase_encrypted")?,
        )?,
        peer_public_key: row.opt_text("peer_public_key")?,
        preshared_key_encrypted: decrypt_preshared_key(
            encryption_key,
            row.opt_text("preshared_key_encrypted")?,
        )?,
        tunnel_public_key: row.opt_text("tunnel_public_key")?,
        tunnel_addresses: split_tunnel_list(row.opt_text("tunnel_addresses")?),
        tunnel_dns_servers: split_tunnel_list(row.opt_text("tunnel_dns_servers")?),
        tunnel_mtu: opt_u16_from_stored(row.opt_i64("tunnel_mtu")?),
        tunnel_keepalive_seconds: opt_u16_from_stored(row.opt_i64("tunnel_keepalive_seconds")?),
        host_key_fingerprint: row.opt_text("host_key_fingerprint")?,
        host_key_pinned_at: row.opt_timestamp("host_key_pinned_at")?,
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
        i64::from(scryer_outbound_http::MAX_PROXY_TIMEOUT_SECONDS),
    ) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use scryer_domain::{ChallengeSolverProtocol, ProxyProviderType};
    use sqlx::sqlite::SqlitePoolOptions;

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
                ProxyProviderType::Trawl,
                Some("request_solution_v1".to_string()),
            )
            .expect("existing solver rows must keep loading"),
            Some(ChallengeSolverProtocol::RequestSolutionV1)
        );
        assert!(parse_persisted_protocol(ProxyProviderType::Byparr, None).is_err());
        assert!(
            parse_persisted_protocol(ProxyProviderType::Byparr, Some("nonsense".to_string()),)
                .is_err()
        );
    }

    #[test]
    fn transport_rows_read_a_null_protocol_as_none() {
        assert_eq!(
            parse_persisted_protocol(ProxyProviderType::Socks5, None)
                .expect("transport rows carry no protocol"),
            None
        );
        assert_eq!(
            parse_persisted_protocol(
                ProxyProviderType::Http,
                Some("request_solution_v1".to_string()),
            )
            .expect("a stray protocol must not make a transport row unloadable"),
            None
        );
    }

    /// The post-0211 SQLite shape, copied from
    /// `migrations/0216_first_class_proxies.sql`, so the round-trip below
    /// exercises the real columns rather than a hand-rolled subset.
    async fn proxy_store() -> (ProxyConfigStore, sqlx::SqlitePool) {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");
        sqlx::raw_sql(
            "CREATE TABLE proxy_configs (
                 id TEXT PRIMARY KEY NOT NULL,
                 name TEXT NOT NULL,
                 provider_type TEXT NOT NULL,
                 protocol TEXT,
                 base_url TEXT NOT NULL,
                 request_timeout_seconds INTEGER NOT NULL DEFAULT 60,
                 is_enabled INTEGER NOT NULL DEFAULT 1,
                 username_encrypted TEXT,
                 password_encrypted TEXT,
                 remote_dns INTEGER NOT NULL DEFAULT 0,
                 private_key_encrypted TEXT,
                 private_key_passphrase_encrypted TEXT,
                 peer_public_key TEXT,
                 preshared_key_encrypted TEXT,
                 tunnel_public_key TEXT,
                 tunnel_addresses TEXT,
                 tunnel_dns_servers TEXT,
                 tunnel_mtu INTEGER,
                 tunnel_keepalive_seconds INTEGER,
                 host_key_fingerprint TEXT,
                 host_key_pinned_at TEXT,
                 last_health_status TEXT,
                 last_error_message TEXT,
                 last_error_at TEXT,
                 created_at TEXT NOT NULL,
                 updated_at TEXT NOT NULL
             )",
        )
        .execute(&pool)
        .await
        .expect("proxy_configs table should be created");
        let store = ProxyConfigStore::new(
            StoreDatastore::Sqlite {
                pool: pool.clone(),
                writer_gate: Arc::new(tokio::sync::Mutex::new(())),
            },
            Arc::new(RwLock::new(None)),
        );
        (store, pool)
    }

    fn tunnel_config() -> ProxyConfig {
        let now = chrono::Utc::now();
        ProxyConfig {
            id: "tunnel-1".to_string(),
            name: "Seedbox".to_string(),
            provider_type: ProxyProviderType::SshTunnel,
            protocol: None,
            base_url: "ssh://seedbox.test:22".to_string(),
            request_timeout_seconds: 30,
            is_enabled: true,
            username_encrypted: Some("operator".to_string()),
            password_encrypted: None,
            remote_dns: false,
            private_key_encrypted: Some(
                "-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----"
                    .to_string(),
            ),
            private_key_passphrase_encrypted: Some("phrase".to_string()),
            peer_public_key: None,
            preshared_key_encrypted: None,
            tunnel_public_key: None,
            tunnel_addresses: Vec::new(),
            tunnel_dns_servers: Vec::new(),
            tunnel_mtu: None,
            tunnel_keepalive_seconds: None,
            host_key_fingerprint: None,
            host_key_pinned_at: None,
            last_health_status: Some(ProxyHealthStatus::Unknown),
            last_error_message: None,
            last_error_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[tokio::test]
    async fn tunnel_key_material_round_trips_and_the_host_key_pin_is_operator_visible() {
        let (store, _pool) = proxy_store().await;
        let config = tunnel_config();
        store
            .create(config.clone())
            .await
            .expect("tunnel row should insert");

        let loaded = store
            .get_by_id("tunnel-1")
            .await
            .expect("row should load")
            .expect("row should exist");
        assert_eq!(loaded.private_key_encrypted, config.private_key_encrypted);
        assert_eq!(
            loaded.private_key_passphrase_encrypted,
            config.private_key_passphrase_encrypted
        );
        assert_eq!(loaded.host_key_fingerprint, None);
        assert_eq!(loaded.host_key_pinned_at, None);
        // A tunnel speaks no solver protocol, exactly like a transport hop.
        assert_eq!(loaded.protocol, None);

        // Pinning is trust-on-first-use and must not look like a config edit:
        // the cache revision (`updated_at`) has to survive it.
        let pinned_at = chrono::Utc::now();
        store
            .pin_host_key("tunnel-1", "SHA256:abc", pinned_at)
            .await
            .expect("pin should succeed");
        let pinned = store
            .get_by_id("tunnel-1")
            .await
            .expect("row should load")
            .expect("row should exist");
        assert_eq!(pinned.host_key_fingerprint.as_deref(), Some("SHA256:abc"));
        assert!(pinned.host_key_pinned_at.is_some());
        assert_eq!(pinned.updated_at, loaded.updated_at);

        // Clearing is an operator edit whose point is that the next connection
        // is a new one, so it *does* move the revision.
        store
            .clear_host_key("tunnel-1")
            .await
            .expect("clear should succeed");
        let cleared = store
            .get_by_id("tunnel-1")
            .await
            .expect("row should load")
            .expect("row should exist");
        assert_eq!(cleared.host_key_fingerprint, None);
        assert_eq!(cleared.host_key_pinned_at, None);
        assert!(cleared.updated_at > loaded.updated_at);
    }

    fn wireguard_config() -> ProxyConfig {
        let now = chrono::Utc::now();
        ProxyConfig {
            id: "wireguard-1".to_string(),
            name: "VPN".to_string(),
            provider_type: ProxyProviderType::WireGuard,
            protocol: None,
            base_url: "wireguard://vpn.test:51820".to_string(),
            request_timeout_seconds: 30,
            is_enabled: true,
            // WireGuard has no user and no password, and no passphrase format.
            username_encrypted: None,
            password_encrypted: None,
            remote_dns: false,
            private_key_encrypted: Some("cHJpdmF0ZS1rZXktYmFzZTY0LXBsYWNlaG9sZGVy".to_string()),
            private_key_passphrase_encrypted: None,
            peer_public_key: Some("cGVlci1wdWJsaWMta2V5LWJhc2U2NC1wbGFjZWhvbGRlcg==".to_string()),
            preshared_key_encrypted: Some("cHJlc2hhcmVkLWtleQ==".to_string()),
            tunnel_public_key: Some("b3VyLXB1YmxpYy1rZXk=".to_string()),
            tunnel_addresses: vec!["10.6.0.2/32".to_string(), "fd00::2/128".to_string()],
            tunnel_dns_servers: vec!["10.6.0.1".to_string()],
            tunnel_mtu: Some(1420),
            tunnel_keepalive_seconds: Some(0),
            host_key_fingerprint: None,
            host_key_pinned_at: None,
            last_health_status: Some(ProxyHealthStatus::Unknown),
            last_error_message: None,
            last_error_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[tokio::test]
    async fn wireguard_key_material_and_link_settings_round_trip() {
        let (store, pool) = proxy_store().await;
        let config = wireguard_config();
        store
            .create(config.clone())
            .await
            .expect("wireguard row should insert");

        let loaded = store
            .get_by_id("wireguard-1")
            .await
            .expect("row should load")
            .expect("row should exist");
        assert_eq!(loaded.private_key_encrypted, config.private_key_encrypted);
        assert_eq!(
            loaded.preshared_key_encrypted,
            config.preshared_key_encrypted
        );
        assert_eq!(loaded.peer_public_key, config.peer_public_key);
        assert_eq!(loaded.tunnel_public_key, config.tunnel_public_key);
        // The lists survive their comma-separated storage in order.
        assert_eq!(loaded.tunnel_addresses, config.tunnel_addresses);
        assert_eq!(loaded.tunnel_dns_servers, config.tunnel_dns_servers);
        assert_eq!(loaded.tunnel_mtu, Some(1420));
        // Zero is "keepalive off", which must not read back as "unset".
        assert_eq!(loaded.tunnel_keepalive_seconds, Some(0));

        // Both public keys are stored in the clear, because an operator has to
        // read them to compare them against their server. The two secrets are
        // not — and with no encryption key configured this store writes them
        // through, so the check that matters is which *column* holds which
        // value, not the ciphertext.
        let stored: (Option<String>, Option<String>, Option<String>) = sqlx::query_as(
            "SELECT peer_public_key, tunnel_public_key, tunnel_addresses
               FROM proxy_configs WHERE id = 'wireguard-1'",
        )
        .fetch_one(&pool)
        .await
        .expect("read the raw row");
        assert_eq!(
            stored,
            (
                config.peer_public_key.clone(),
                config.tunnel_public_key.clone(),
                Some("10.6.0.2/32,fd00::2/128".to_string()),
            )
        );

        // An update rewrites all seven, including clearing the lists.
        let mut edited = loaded.clone();
        edited.tunnel_dns_servers = Vec::new();
        edited.tunnel_mtu = None;
        edited.tunnel_keepalive_seconds = None;
        edited.preshared_key_encrypted = None;
        edited.updated_at = chrono::Utc::now();
        store.update(edited).await.expect("update should succeed");
        let reloaded = store
            .get_by_id("wireguard-1")
            .await
            .expect("row should load")
            .expect("row should exist");
        assert!(reloaded.tunnel_dns_servers.is_empty());
        assert_eq!(reloaded.tunnel_mtu, None);
        assert_eq!(reloaded.tunnel_keepalive_seconds, None);
        assert_eq!(reloaded.preshared_key_encrypted, None);
        assert_eq!(reloaded.tunnel_addresses, config.tunnel_addresses);
    }

    #[test]
    fn tunnel_lists_survive_the_comma_separated_column() {
        assert_eq!(join_tunnel_list(&[]), None);
        assert_eq!(
            join_tunnel_list(&["10.6.0.2/32".to_string(), " fd00::2/128 ".to_string()]),
            Some("10.6.0.2/32,fd00::2/128".to_string())
        );
        assert_eq!(split_tunnel_list(None), Vec::<String>::new());
        // A blank entry is dropped rather than made into an empty address: a
        // stored list is only ever as good as what was pasted, and an
        // unloadable row is the worse failure.
        assert_eq!(
            split_tunnel_list(Some("10.6.0.2/32, ,fd00::2/128".to_string())),
            vec!["10.6.0.2/32".to_string(), "fd00::2/128".to_string()]
        );
    }

    #[test]
    fn out_of_range_link_settings_read_as_no_opinion() {
        assert_eq!(opt_u16_from_stored(Some(1420)), Some(1420));
        assert_eq!(opt_u16_from_stored(Some(0)), Some(0));
        assert_eq!(opt_u16_from_stored(None), None);
        // The engine owns the real range; a nonsense integer must not make the
        // row unloadable.
        assert_eq!(opt_u16_from_stored(Some(-1)), None);
        assert_eq!(opt_u16_from_stored(Some(100_000)), None);
    }
}

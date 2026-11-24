pub use scryer_infrastructure_crypto::{EncryptionKey, decrypt_value, encrypt_value, is_encrypted};

use crate::keystore::{self, KeyStore};
use std::path::PathBuf;

const ENCRYPTION_KEY_SETTING: &str = "encryption.master_key";
const SETTINGS_SCOPE_SYSTEM: &str = "system";
const ALLOW_EPHEMERAL_ENCRYPTION_KEY_ENV: &str = "SCRYER_ALLOW_EPHEMERAL_ENCRYPTION_KEY";

/// Ensure an encryption master key is available.
///
/// Priority:
/// 1. `SCRYER_ENCRYPTION_KEY` env var (explicit override, always wins)
/// 2. Platform keystores (Docker secret, OS keychain, key file — in priority order)
/// 3. Legacy DB migration (one-time, deprecated — remove at 1.0.0)
/// 4. Auto-generate in memory, store in best available keystore, warn loudly
///
/// The master key is **never** stored in the database. Legacy DB keys are migrated
/// out on first startup after upgrade.
pub async fn ensure_encryption_key(
    db: &crate::SqliteServices,
    data_dir: Option<PathBuf>,
) -> Result<EncryptionKey, String> {
    let stores = keystore::platform_keystores(data_dir);

    // 1. Env var (always wins, all platforms)
    if let Some(key) = from_env_var()? {
        opportunistic_store(&stores, &key);
        tracing::info!("using encryption master key from SCRYER_ENCRYPTION_KEY");
        return Ok(key);
    }

    // 2. Platform keystores (Docker secret, keychain, key file — in priority order)
    for store in &stores {
        match store.get_key() {
            Ok(Some(key_b64)) => {
                let key = EncryptionKey::from_base64(&key_b64)
                    .map_err(|e| format!("invalid key in {}: {e}", store.name()))?;
                tracing::info!("using encryption master key from {}", store.name());
                return Ok(key);
            }
            Ok(None) => continue,
            Err(e) => {
                tracing::warn!("could not read from {}: {e}", store.name());
                continue;
            }
        }
    }

    // 3. Legacy DB migration (deprecated — remove at 1.0.0)
    #[expect(deprecated)]
    if let Some(key) = try_migrate_from_db(db, &stores).await? {
        return Ok(key);
    }

    // 4. Auto-generate, store in best available keystore, warn user
    let key = EncryptionKey::generate();
    let stored_in = try_store_new_key(&stores, &key);
    finish_generated_key_bootstrap(key, stored_in)
}

/// Ensure an encryption key for engines that do not support the legacy
/// SQLite-only plaintext DB-key migration path.
pub async fn ensure_encryption_key_without_legacy(
    data_dir: Option<PathBuf>,
) -> Result<EncryptionKey, String> {
    let stores = keystore::platform_keystores(data_dir);

    if let Some(key) = from_env_var()? {
        opportunistic_store(&stores, &key);
        tracing::info!("using encryption master key from SCRYER_ENCRYPTION_KEY");
        return Ok(key);
    }

    for store in &stores {
        match store.get_key() {
            Ok(Some(key_b64)) => {
                let key = EncryptionKey::from_base64(&key_b64)
                    .map_err(|e| format!("invalid key in {}: {e}", store.name()))?;
                tracing::info!("using encryption master key from {}", store.name());
                return Ok(key);
            }
            Ok(None) => continue,
            Err(e) => {
                tracing::warn!("could not read from {}: {e}", store.name());
                continue;
            }
        }
    }

    let key = EncryptionKey::generate();
    let stored_in = try_store_new_key(&stores, &key);
    finish_generated_key_bootstrap(key, stored_in)
}

/// Load an already configured encryption key without generating or storing one.
///
/// Migration hooks use this before normal bootstrap so encrypted legacy values
/// can be read without changing a fresh install's key lifecycle.
pub fn load_existing_encryption_key_without_generation(
    data_dir: Option<PathBuf>,
) -> Result<Option<EncryptionKey>, String> {
    if let Some(key) = from_env_var()? {
        tracing::info!("using encryption master key from SCRYER_ENCRYPTION_KEY for migrations");
        return Ok(Some(key));
    }

    let stores = keystore::platform_keystores(data_dir);
    for store in &stores {
        match store.get_key() {
            Ok(Some(key_b64)) => {
                let key = EncryptionKey::from_base64(&key_b64)
                    .map_err(|e| format!("invalid key in {}: {e}", store.name()))?;
                tracing::info!(
                    "using encryption master key from {} for migrations",
                    store.name()
                );
                return Ok(Some(key));
            }
            Ok(None) => continue,
            Err(e) => {
                tracing::warn!(
                    "could not read migration encryption key from {}: {e}",
                    store.name()
                );
                continue;
            }
        }
    }

    Ok(None)
}

/// Load an existing key for SQLite migrations, including the deprecated
/// database-stored key so migration hooks can decrypt legacy config before
/// normal bootstrap has a chance to migrate that key out.
pub async fn load_existing_sqlite_migration_encryption_key(
    pool: &sqlx::SqlitePool,
    data_dir: Option<PathBuf>,
) -> Result<Option<EncryptionKey>, String> {
    if let Some(key) = load_existing_encryption_key_without_generation(data_dir)? {
        return Ok(Some(key));
    }

    #[allow(deprecated)]
    read_legacy_db_key_from_pool(pool).await
}

/// Check the `SCRYER_ENCRYPTION_KEY` environment variable.
fn from_env_var() -> Result<Option<EncryptionKey>, String> {
    let Ok(env_key) = std::env::var("SCRYER_ENCRYPTION_KEY") else {
        return Ok(None);
    };
    let trimmed = env_key.trim().to_string();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let key = EncryptionKey::from_base64(&trimmed)
        .map_err(|e| format!("invalid SCRYER_ENCRYPTION_KEY: {e}"))?;
    Ok(Some(key))
}

fn ephemeral_encryption_key_allowed() -> bool {
    std::env::var(ALLOW_EPHEMERAL_ENCRYPTION_KEY_ENV)
        .ok()
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("true"))
}

fn finish_generated_key_bootstrap(
    key: EncryptionKey,
    stored_in: Option<&'static str>,
) -> Result<EncryptionKey, String> {
    match stored_in {
        Some(name) => {
            tracing::warn!(
                "generated new encryption master key and stored in {name} — \
                 all sensitive settings (passwords, API keys) are encrypted with this key"
            );
            Ok(key)
        }
        None if ephemeral_encryption_key_allowed() => {
            tracing::warn!(
                "generated new ephemeral encryption master key in memory because no persistent \
                 keystore accepted it; encrypted settings may be unrecoverable after restart. \
                 This should only be used for throwaway or development instances"
            );
            Ok(key)
        }
        None => Err(format!(
            "generated a new encryption master key but no persistent keystore accepted it. \
             Startup is stopping before using an unpersisted key. Run `scryer init`, mount a \
             Docker secret, set SCRYER_ENCRYPTION_KEY, or make the Scryer data directory \
             writable. For throwaway or development instances only, set \
             {ALLOW_EPHEMERAL_ENCRYPTION_KEY_ENV}=true to use an in-memory key for this process."
        )),
    }
}

/// Try to store the key in the first writable keystore. Returns the store name on success.
fn try_store_new_key(stores: &[Box<dyn KeyStore>], key: &EncryptionKey) -> Option<&'static str> {
    for store in stores {
        match store.set_key(&key.to_base64()) {
            Ok(()) => return Some(store.name()),
            Err(e) => {
                tracing::warn!("could not store encryption key in {}: {e}", store.name());
                continue;
            }
        }
    }
    None
}

/// If the key was loaded from an env var or other source, also store it in the
/// first available keystore so the user can drop the env var later.
/// If the keystore already has a different key, overwrite it to stay in sync.
fn opportunistic_store(stores: &[Box<dyn KeyStore>], key: &EncryptionKey) {
    let key_b64 = key.to_base64();
    for store in stores {
        match store.get_key() {
            Ok(None) => match store.set_key(&key_b64) {
                Ok(()) => {
                    tracing::info!("copied encryption key to {}", store.name());
                    return;
                }
                Err(_) => continue,
            },
            Ok(Some(existing)) if existing == key_b64 => return, // already in sync
            Ok(Some(_)) => {
                // Keystore has a stale key — overwrite with the authoritative one
                match store.set_key(&key_b64) {
                    Ok(()) => {
                        tracing::info!("updated stale encryption key in {}", store.name());
                        return;
                    }
                    Err(e) => {
                        tracing::warn!(
                            "{} has a different encryption key but could not be updated: {e}",
                            store.name()
                        );
                        continue;
                    }
                }
            }
            Err(_) => continue,
        }
    }
}

// ── Legacy DB migration (deprecated) ────────────────────────────────────────

/// One-time migration of the encryption key from plaintext DB storage to a
/// proper keystore. The DB setting is cleared after migration.
#[deprecated(since = "0.10.0", note = "legacy DB key migration — remove at 1.0.0")]
async fn try_migrate_from_db(
    db: &crate::SqliteServices,
    stores: &[Box<dyn KeyStore>],
) -> Result<Option<EncryptionKey>, String> {
    #[allow(deprecated)]
    let db_key = read_legacy_db_key(db).await?;
    let Some(key) = db_key else {
        return Ok(None);
    };

    let migrated_to = try_store_new_key(stores, &key);
    if let Some(name) = migrated_to {
        #[allow(deprecated)]
        clear_legacy_db_key(db).await?;
        tracing::info!(
            "migrated encryption key from database to {name} — \
             plaintext key removed from database"
        );
    } else {
        // No writable keystore — keep the key in the DB rather than risk losing it.
        // Warn with setup guidance, but do NOT clear the DB entry or print the key.
        warn_legacy_db_key_kept_without_store(&key);
    }
    Ok(Some(key))
}

fn warn_legacy_db_key_kept_without_store(_key: &EncryptionKey) {
    tracing::warn!(
        "encryption key is still stored in the database (legacy storage) — \
         no secure keystore is writable, so the database key was left in place. Run `scryer init`, \
         provide a Docker secret, set SCRYER_ENCRYPTION_KEY, or make the Scryer data directory \
         writable to complete the migration"
    );
}

#[deprecated(since = "0.10.0", note = "legacy DB key migration — remove at 1.0.0")]
#[allow(deprecated)]
async fn read_legacy_db_key(db: &crate::SqliteServices) -> Result<Option<EncryptionKey>, String> {
    read_legacy_db_key_from_pool(db.pool()).await
}

#[deprecated(since = "0.10.0", note = "legacy DB key migration — remove at 1.0.0")]
async fn read_legacy_db_key_from_pool(
    pool: &sqlx::SqlitePool,
) -> Result<Option<EncryptionKey>, String> {
    let table_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)
           FROM sqlite_master
          WHERE type = 'table'
            AND name IN ('settings_definitions', 'settings_values')",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| format!("failed to inspect settings tables for encryption key: {e}"))?;
    if table_count < 2 {
        return Ok(None);
    }

    let raw_value = sqlx::query_scalar::<_, String>(
        "SELECT values_table.value_json
           FROM settings_values values_table
           JOIN settings_definitions definitions
             ON definitions.id = values_table.setting_definition_id
          WHERE definitions.scope = ?1
            AND definitions.key_name = ?2
            AND values_table.scope = ?1
            AND COALESCE(values_table.scope_id, '') = ''
          ORDER BY values_table.updated_at DESC
          LIMIT 1",
    )
    .bind(SETTINGS_SCOPE_SYSTEM)
    .bind(ENCRYPTION_KEY_SETTING)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("failed to read legacy encryption key setting: {e}"))?;

    let existing = raw_value.as_deref().and_then(parse_string_json);
    match existing {
        Some(key_b64) if !key_b64.is_empty() && key_b64 != "migrated" => {
            let key = EncryptionKey::from_base64(&key_b64)
                .map_err(|e| format!("invalid encryption key in database: {e}"))?;
            tracing::info!("using legacy database encryption key for migrations");
            Ok(Some(key))
        }
        _ => Ok(None),
    }
}

#[deprecated(since = "0.10.0", note = "legacy DB key migration — remove at 1.0.0")]
async fn clear_legacy_db_key(db: &crate::SqliteServices) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();
    let updated = sqlx::query(
        "UPDATE settings_values
            SET value_json = ?1,
                source = ?2,
                updated_by_user_id = NULL,
                updated_at = ?3
          WHERE id = (
              SELECT values_table.id
                FROM settings_values values_table
                JOIN settings_definitions definitions
                  ON definitions.id = values_table.setting_definition_id
               WHERE definitions.scope = ?4
                 AND definitions.key_name = ?5
                 AND values_table.scope = ?4
                 AND COALESCE(values_table.scope_id, '') = ''
               ORDER BY values_table.updated_at DESC
               LIMIT 1
          )",
    )
    .bind(serde_json::to_string("migrated").expect("static JSON string"))
    .bind("migration")
    .bind(now)
    .bind(SETTINGS_SCOPE_SYSTEM)
    .bind(ENCRYPTION_KEY_SETTING)
    .execute(db.pool())
    .await
    .map_err(|e| format!("failed to clear legacy DB key: {e}"))?;
    if updated.rows_affected() != 1 {
        return Err(
            "failed to clear legacy DB key: setting disappeared during migration".to_string(),
        );
    }
    Ok(())
}

fn parse_string_json(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "null" {
        return None;
    }
    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(serde_json::Value::String(s)) if !s.is_empty() => Some(s),
        _ => None,
    }
}

#[cfg(test)]
// disk-guard: test, example and build-script code may call the filesystem directly.
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use base64::{Engine, engine::general_purpose::STANDARD};
    use scryer_infrastructure_configuration::settings::settings_store::SettingsStore;
    use scryer_infrastructure_sql::types::SettingDefinitionSeed;
    use std::io::{self, Write};
    use std::sync::{Arc, Mutex, OnceLock};
    use tracing_subscriber::fmt::MakeWriter;

    use crate::SqliteServices;

    #[derive(Clone)]
    struct SharedLogWriter {
        buffer: Arc<Mutex<Vec<u8>>>,
    }

    struct SharedLogGuard {
        buffer: Arc<Mutex<Vec<u8>>>,
    }

    impl<'a> MakeWriter<'a> for SharedLogWriter {
        type Writer = SharedLogGuard;

        fn make_writer(&'a self) -> Self::Writer {
            SharedLogGuard {
                buffer: self.buffer.clone(),
            }
        }
    }

    impl Write for SharedLogGuard {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.buffer
                .lock()
                .expect("lock log buffer")
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn capture_logs<R>(f: impl FnOnce() -> R) -> (R, String) {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::fmt()
            .with_writer(SharedLogWriter {
                buffer: buffer.clone(),
            })
            .with_ansi(false)
            .without_time()
            .finish();

        let result = tracing::subscriber::with_default(subscriber, f);
        let logs = String::from_utf8(buffer.lock().expect("lock log buffer").clone())
            .expect("logs should be UTF-8");
        (result, logs)
    }

    fn restore_ephemeral_env(original: Option<String>) {
        match original {
            Some(value) => unsafe { std::env::set_var(ALLOW_EPHEMERAL_ENCRYPTION_KEY_ENV, value) },
            None => unsafe { std::env::remove_var(ALLOW_EPHEMERAL_ENCRYPTION_KEY_ENV) },
        }
    }

    fn known_test_key() -> EncryptionKey {
        EncryptionKey::from_bytes([7u8; 32])
    }

    fn assert_logs_do_not_contain_key_material(logs: &str, key: &EncryptionKey) {
        assert!(!logs.contains(&key.to_base64()));
        assert!(!logs.contains("SCRYER_ENCRYPTION_KEY="));
    }

    fn encryption_key_setting_definition() -> SettingDefinitionSeed {
        SettingDefinitionSeed {
            category: "service".into(),
            scope: SETTINGS_SCOPE_SYSTEM.into(),
            key_name: ENCRYPTION_KEY_SETTING.into(),
            data_type: "string".into(),
            default_value_json: "null".into(),
            is_sensitive: true,
            validation_json: None,
        }
    }

    struct FailingTestKeystore {
        error: &'static str,
    }

    impl KeyStore for FailingTestKeystore {
        fn get_key(&self) -> Result<Option<String>, String> {
            Ok(None)
        }

        fn set_key(&self, _key_base64: &str) -> Result<(), String> {
            Err(self.error.to_string())
        }

        fn delete_key(&self) -> Result<(), String> {
            Ok(())
        }

        fn name(&self) -> &'static str {
            "failing test keystore"
        }
    }

    #[test]
    fn generated_key_without_writable_store_fails_closed_without_logging_key_material() {
        let _guard = env_lock().lock().expect("lock env guard");
        let original = std::env::var(ALLOW_EPHEMERAL_ENCRYPTION_KEY_ENV).ok();
        unsafe { std::env::remove_var(ALLOW_EPHEMERAL_ENCRYPTION_KEY_ENV) };

        let key = known_test_key();
        let key_b64 = key.to_base64();
        let (result, logs) = capture_logs(|| finish_generated_key_bootstrap(key.clone(), None));
        restore_ephemeral_env(original);

        let error = result.expect_err("startup should fail without persistent key storage");
        assert!(error.contains("scryer init"));
        assert!(error.contains(ALLOW_EPHEMERAL_ENCRYPTION_KEY_ENV));
        assert!(!error.contains(&key_b64));
        assert_logs_do_not_contain_key_material(&logs, &key);
    }

    #[test]
    fn generated_key_store_failure_warning_does_not_log_key_material() {
        let _guard = env_lock().lock().expect("lock env guard");
        let original = std::env::var(ALLOW_EPHEMERAL_ENCRYPTION_KEY_ENV).ok();
        unsafe { std::env::remove_var(ALLOW_EPHEMERAL_ENCRYPTION_KEY_ENV) };

        let key = known_test_key();
        let key_b64 = key.to_base64();
        let stores: Vec<Box<dyn KeyStore>> = vec![Box::new(FailingTestKeystore {
            error: "permission denied",
        })];
        let ((stored_in, result), logs) = capture_logs(|| {
            let stored_in = try_store_new_key(&stores, &key);
            let result = finish_generated_key_bootstrap(key.clone(), stored_in);
            (stored_in, result)
        });
        restore_ephemeral_env(original);

        assert!(stored_in.is_none());
        let error = result.expect_err("startup should fail without persistent key storage");
        assert!(logs.contains("could not store encryption key in failing test keystore"));
        assert!(logs.contains("permission denied"));
        assert!(!error.contains(&key_b64));
        assert_logs_do_not_contain_key_material(&logs, &key);
    }

    #[test]
    fn generated_key_with_ephemeral_opt_in_warns_without_logging_key_material() {
        let _guard = env_lock().lock().expect("lock env guard");
        let original = std::env::var(ALLOW_EPHEMERAL_ENCRYPTION_KEY_ENV).ok();
        unsafe { std::env::set_var(ALLOW_EPHEMERAL_ENCRYPTION_KEY_ENV, " TrUe ") };

        let key = known_test_key();
        let (result, logs) = capture_logs(|| finish_generated_key_bootstrap(key.clone(), None));
        restore_ephemeral_env(original);

        let loaded = result.expect("ephemeral opt-in should allow startup");
        assert_eq!(loaded.to_base64(), key.to_base64());
        assert!(logs.contains("ephemeral encryption master key"));
        assert_logs_do_not_contain_key_material(&logs, &key);
    }

    #[test]
    fn ephemeral_opt_in_requires_true_token() {
        let _guard = env_lock().lock().expect("lock env guard");
        let original = std::env::var(ALLOW_EPHEMERAL_ENCRYPTION_KEY_ENV).ok();

        unsafe { std::env::set_var(ALLOW_EPHEMERAL_ENCRYPTION_KEY_ENV, "1") };
        assert!(!ephemeral_encryption_key_allowed());

        unsafe { std::env::set_var(ALLOW_EPHEMERAL_ENCRYPTION_KEY_ENV, "true") };
        assert!(ephemeral_encryption_key_allowed());

        restore_ephemeral_env(original);
    }

    #[test]
    fn legacy_db_key_warning_does_not_log_key_material() {
        let key = known_test_key();
        let (_, logs) = capture_logs(|| warn_legacy_db_key_kept_without_store(&key));

        assert!(logs.contains("legacy storage"));
        assert_logs_do_not_contain_key_material(&logs, &key);
    }

    #[test]
    fn legacy_db_key_without_writable_store_stays_in_db_and_logs_no_material() {
        let _guard = env_lock().lock().expect("lock env guard");
        let original_key = std::env::var("SCRYER_ENCRYPTION_KEY").ok();
        unsafe { std::env::remove_var("SCRYER_ENCRYPTION_KEY") };

        let key = known_test_key();
        let key_b64 = key.to_base64();
        let legacy_value_json = serde_json::to_string(&key_b64).expect("serialize legacy key");

        let ((loaded, stored_value_json), logs) = capture_logs(|| {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build runtime");
            runtime.block_on(async {
                let temp = tempfile::tempdir().expect("tempdir");
                let db_path = temp.path().join("scryer.db");
                let db_url = format!("sqlite://{}", db_path.display());
                let services = SqliteServices::new(db_url).await.expect("sqlite services");
                let settings =
                    SettingsStore::new(services.datastore(), services.encryption_key_state());

                settings
                    .batch_ensure_setting_definitions(vec![encryption_key_setting_definition()])
                    .await
                    .expect("seed encryption key definition");
                settings
                    .upsert_setting_value(
                        SETTINGS_SCOPE_SYSTEM,
                        ENCRYPTION_KEY_SETTING,
                        None,
                        legacy_value_json.clone(),
                        "test",
                        None,
                    )
                    .await
                    .expect("seed legacy encryption key");

                let loaded = ensure_encryption_key(&services, None)
                    .await
                    .expect("legacy key should be retained");
                let stored_value_json = settings
                    .get_setting_with_defaults(SETTINGS_SCOPE_SYSTEM, ENCRYPTION_KEY_SETTING, None)
                    .await
                    .expect("read legacy setting")
                    .expect("legacy setting should exist")
                    .value_json
                    .expect("legacy value should be stored");

                (loaded, stored_value_json)
            })
        });

        match original_key {
            Some(value) => unsafe { std::env::set_var("SCRYER_ENCRYPTION_KEY", value) },
            None => unsafe { std::env::remove_var("SCRYER_ENCRYPTION_KEY") },
        }

        assert_eq!(loaded.to_base64(), key.to_base64());
        assert_eq!(stored_value_json, legacy_value_json);
        assert!(logs.contains("legacy storage"));
        assert_logs_do_not_contain_key_material(&logs, &key);
    }

    #[test]
    fn encrypt_decrypt_round_trip() {
        let key = EncryptionKey::generate();
        let plaintext = "secret-api-key-12345";
        let encrypted = encrypt_value(&key, plaintext).unwrap();

        assert!(is_encrypted(&encrypted));
        assert_ne!(encrypted, plaintext);

        let decrypted = decrypt_value(&key, &encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn plaintext_passthrough() {
        let key = EncryptionKey::generate();
        let plaintext = "not-encrypted-value";
        let result = decrypt_value(&key, plaintext).unwrap();
        assert_eq!(result, plaintext);
    }

    #[test]
    fn wrong_key_fails() {
        let key1 = EncryptionKey::generate();
        let key2 = EncryptionKey::generate();
        let encrypted = encrypt_value(&key1, "secret").unwrap();
        let result = decrypt_value(&key2, &encrypted);
        assert!(result.is_err());
    }

    #[test]
    fn key_base64_round_trip() {
        let key = EncryptionKey::generate();
        let encoded = key.to_base64();
        let decoded = EncryptionKey::from_base64(&encoded).unwrap();
        assert_eq!(key.to_base64(), decoded.to_base64());
    }

    #[test]
    fn empty_string_encrypts() {
        let key = EncryptionKey::generate();
        let encrypted = encrypt_value(&key, "").unwrap();
        let decrypted = decrypt_value(&key, &encrypted).unwrap();
        assert_eq!(decrypted, "");
    }

    #[test]
    fn json_value_encrypts() {
        let key = EncryptionKey::generate();
        let json = r#""my-password-123""#;
        let encrypted = encrypt_value(&key, json).unwrap();
        let decrypted = decrypt_value(&key, &encrypted).unwrap();
        assert_eq!(decrypted, json);
    }

    #[test]
    fn is_encrypted_detection() {
        assert!(is_encrypted("enc:v1:abc123"));
        assert!(!is_encrypted("plain-value"));
        assert!(!is_encrypted(""));
    }

    #[test]
    fn reject_invalid_base64_key() {
        let result = EncryptionKey::from_base64("not-valid-base64!!!");
        assert!(result.is_err());
    }

    #[test]
    fn reject_wrong_length_key() {
        let too_short = STANDARD.encode([0u8; 16]);
        let result = EncryptionKey::from_base64(&too_short);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("32 bytes"));
    }
}

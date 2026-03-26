use base64::{Engine, engine::general_purpose::STANDARD};
use ring::aead::{self, AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use ring::rand::{SecureRandom, SystemRandom};

const ENCRYPTED_PREFIX: &str = "enc:v1:";

/// A 32-byte AES-256-GCM key for encrypting/decrypting sensitive values at rest.
#[derive(Clone)]
pub struct EncryptionKey {
    key_bytes: [u8; 32],
}

impl std::fmt::Debug for EncryptionKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EncryptionKey")
            .field("key_bytes", &"[REDACTED]")
            .finish()
    }
}

impl EncryptionKey {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self { key_bytes: bytes }
    }

    pub fn from_base64(encoded: &str) -> Result<Self, String> {
        let decoded = STANDARD
            .decode(encoded.trim())
            .map_err(|e| format!("invalid base64: {e}"))?;
        if decoded.len() != 32 {
            return Err(format!(
                "encryption key must be exactly 32 bytes, got {}",
                decoded.len()
            ));
        }
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&decoded);
        Ok(Self { key_bytes: bytes })
    }

    pub fn generate() -> Self {
        let rng = SystemRandom::new();
        let mut bytes = [0u8; 32];
        rng.fill(&mut bytes)
            .expect("failed to generate random encryption key");
        Self { key_bytes: bytes }
    }

    pub fn to_base64(&self) -> String {
        STANDARD.encode(self.key_bytes)
    }

    fn make_key(&self) -> LessSafeKey {
        let unbound = UnboundKey::new(&AES_256_GCM, &self.key_bytes)
            .expect("AES-256-GCM key construction should not fail with 32-byte key");
        LessSafeKey::new(unbound)
    }
}

/// Encrypt a plaintext string. Returns `enc:v1:<base64(nonce || ciphertext || tag)>`.
pub fn encrypt_value(key: &EncryptionKey, plaintext: &str) -> Result<String, String> {
    let aead_key = key.make_key();
    let rng = SystemRandom::new();

    let mut nonce_bytes = [0u8; NONCE_LEN];
    rng.fill(&mut nonce_bytes)
        .map_err(|_| "failed to generate nonce".to_string())?;
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);

    // seal_in_place_append_tag needs a buffer with room for the tag
    let mut in_out = plaintext.as_bytes().to_vec();
    aead_key
        .seal_in_place_append_tag(nonce, Aad::empty(), &mut in_out)
        .map_err(|_| "encryption failed".to_string())?;

    // Prepend the nonce: nonce || ciphertext || tag
    let mut combined = Vec::with_capacity(NONCE_LEN + in_out.len());
    combined.extend_from_slice(&nonce_bytes);
    combined.extend_from_slice(&in_out);

    Ok(format!("{ENCRYPTED_PREFIX}{}", STANDARD.encode(&combined)))
}

/// Decrypt a stored value. If it doesn't have the `enc:v1:` prefix, return as-is (plaintext passthrough).
pub fn decrypt_value(key: &EncryptionKey, stored: &str) -> Result<String, String> {
    let Some(encoded) = stored.strip_prefix(ENCRYPTED_PREFIX) else {
        return Ok(stored.to_string());
    };

    let combined = STANDARD
        .decode(encoded.trim())
        .map_err(|e| format!("invalid base64 in encrypted value: {e}"))?;

    if combined.len() < NONCE_LEN + aead::AES_256_GCM.tag_len() {
        return Err("encrypted value too short".to_string());
    }

    let (nonce_bytes, ciphertext_and_tag) = combined.split_at(NONCE_LEN);
    let nonce = Nonce::try_assume_unique_for_key(nonce_bytes)
        .map_err(|_| "invalid nonce length".to_string())?;

    let aead_key = key.make_key();
    let mut in_out = ciphertext_and_tag.to_vec();
    let plaintext = aead_key
        .open_in_place(nonce, Aad::empty(), &mut in_out)
        .map_err(|_| "decryption failed (wrong key or corrupted data)".to_string())?;

    String::from_utf8(plaintext.to_vec())
        .map_err(|e| format!("decrypted value is not valid UTF-8: {e}"))
}

/// Check if a value is encrypted (has the `enc:v1:` prefix).
pub fn is_encrypted(value: &str) -> bool {
    value.starts_with(ENCRYPTED_PREFIX)
}

use crate::keystore::{self, KeyStore};
use std::path::PathBuf;

const ENCRYPTION_KEY_SETTING: &str = "encryption.master_key";
const SETTINGS_SCOPE_SYSTEM: &str = "system";

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
    match stored_in {
        Some(name) => {
            tracing::warn!(
                "generated new encryption master key and stored in {name} — \
                 all sensitive settings (passwords, API keys) are encrypted with this key"
            );
        }
        None => {
            tracing::warn!(
                "generated new encryption master key (in memory only) — \
                 set SCRYER_ENCRYPTION_KEY to persist it across restarts\n\n  \
                 SCRYER_ENCRYPTION_KEY={}\n",
                key.to_base64()
            );
        }
    }
    Ok(key)
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
        // Log the key so the user can capture it, but do NOT clear the DB entry.
        tracing::warn!(
            "encryption key is in the database (legacy storage) — \
             no secure keystore available to migrate it to. Set \
             SCRYER_ENCRYPTION_KEY as an environment variable or Docker \
             secret to complete the migration:\n\n  \
             SCRYER_ENCRYPTION_KEY={}\n",
            key.to_base64()
        );
    }
    Ok(Some(key))
}

#[deprecated(since = "0.10.0", note = "legacy DB key migration — remove at 1.0.0")]
async fn read_legacy_db_key(db: &crate::SqliteServices) -> Result<Option<EncryptionKey>, String> {
    let record = db
        .get_setting_with_defaults(SETTINGS_SCOPE_SYSTEM, ENCRYPTION_KEY_SETTING, None)
        .await
        .map_err(|e| format!("failed to read encryption key setting: {e}"))?;

    let existing = record
        .as_ref()
        .and_then(|r| r.value_json.as_deref())
        .and_then(parse_string_json);

    match existing {
        Some(key_b64) if !key_b64.is_empty() && key_b64 != "migrated" => {
            let key = EncryptionKey::from_base64(&key_b64)
                .map_err(|e| format!("invalid encryption key in database: {e}"))?;
            Ok(Some(key))
        }
        _ => Ok(None),
    }
}

#[deprecated(since = "0.10.0", note = "legacy DB key migration — remove at 1.0.0")]
async fn clear_legacy_db_key(db: &crate::SqliteServices) -> Result<(), String> {
    db.upsert_setting_value(
        SETTINGS_SCOPE_SYSTEM,
        ENCRYPTION_KEY_SETTING,
        None,
        serde_json::to_string("migrated").unwrap(),
        "migration",
        None,
    )
    .await
    .map_err(|e| format!("failed to clear legacy DB key: {e}"))?;
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
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_round_trip() {
        let key = EncryptionKey::generate();
        let plaintext = "secret-api-key-12345";
        let encrypted = encrypt_value(&key, plaintext).unwrap();

        assert!(encrypted.starts_with(ENCRYPTED_PREFIX));
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
        assert_eq!(key.key_bytes, decoded.key_bytes);
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

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

const ENCRYPTION_KEY_SETTING: &str = "encryption.master_key";
const SETTINGS_SCOPE_SYSTEM: &str = "system";

/// Ensure an encryption master key exists in the database.
///
/// Priority: `SCRYER_ENCRYPTION_KEY` env var > DB setting > auto-generate.
///
/// This must run BEFORE `set_encryption_key` is called, so the master key
/// itself is stored unencrypted (it's the one unprotected sensitive value).
pub async fn ensure_encryption_key(db: &crate::SqliteServices) -> Result<EncryptionKey, String> {
    // Check env var first
    if let Ok(env_key) = std::env::var("SCRYER_ENCRYPTION_KEY") {
        let env_key = env_key.trim().to_string();
        if !env_key.is_empty() {
            let key = EncryptionKey::from_base64(&env_key)
                .map_err(|e| format!("invalid SCRYER_ENCRYPTION_KEY: {e}"))?;

            db.upsert_setting_value(
                SETTINGS_SCOPE_SYSTEM,
                ENCRYPTION_KEY_SETTING,
                None,
                serde_json::to_string(&key.to_base64()).unwrap(),
                "env",
                None,
            )
            .await
            .map_err(|e| format!("failed to persist encryption key from env: {e}"))?;

            tracing::info!("using encryption master key from SCRYER_ENCRYPTION_KEY");
            return Ok(key);
        }
    }

    // Check DB
    let record = db
        .get_setting_with_defaults(SETTINGS_SCOPE_SYSTEM, ENCRYPTION_KEY_SETTING, None)
        .await
        .map_err(|e| format!("failed to read encryption key setting: {e}"))?;

    let existing = record
        .as_ref()
        .and_then(|r| r.value_json.as_deref())
        .and_then(parse_string_json);

    if let Some(key_b64) = existing {
        if !key_b64.is_empty() {
            let key = EncryptionKey::from_base64(&key_b64)
                .map_err(|e| format!("invalid encryption key in database: {e}"))?;
            tracing::info!("loaded encryption master key from database");
            return Ok(key);
        }
    }

    // Generate new key
    let key = EncryptionKey::generate();
    tracing::warn!(
        "generated new encryption master key — all sensitive settings (passwords, API keys) \
         are encrypted with this key. To preserve it across upgrades, add this to your \
         docker-compose.yml environment:\n\n  SCRYER_ENCRYPTION_KEY: {}\n",
        key.to_base64()
    );

    db.upsert_setting_value(
        SETTINGS_SCOPE_SYSTEM,
        ENCRYPTION_KEY_SETTING,
        None,
        serde_json::to_string(&key.to_base64()).unwrap(),
        "auto-generated",
        None,
    )
    .await
    .map_err(|e| format!("failed to persist generated encryption key: {e}"))?;

    Ok(key)
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

use aws_lc_rs::aead::{self, AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use aws_lc_rs::rand::{SecureRandom, SystemRandom};
use base64::{Engine, engine::general_purpose::STANDARD};

pub mod artifact_reference;
pub mod config;

const ENCRYPTED_PREFIX: &str = "enc:v1:";

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
            .expect("failed to generate random encryption master key");
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

pub fn encrypt_value(key: &EncryptionKey, plaintext: &str) -> Result<String, String> {
    let aead_key = key.make_key();
    let rng = SystemRandom::new();
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rng.fill(&mut nonce_bytes)
        .map_err(|_| "failed to generate nonce".to_string())?;
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);
    let mut in_out = plaintext.as_bytes().to_vec();
    aead_key
        .seal_in_place_append_tag(nonce, Aad::empty(), &mut in_out)
        .map_err(|_| "encryption failed".to_string())?;
    let mut combined = Vec::with_capacity(NONCE_LEN + in_out.len());
    combined.extend_from_slice(&nonce_bytes);
    combined.extend_from_slice(&in_out);
    Ok(format!("{ENCRYPTED_PREFIX}{}", STANDARD.encode(&combined)))
}

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

pub fn is_encrypted(value: &str) -> bool {
    value.starts_with(ENCRYPTED_PREFIX)
}

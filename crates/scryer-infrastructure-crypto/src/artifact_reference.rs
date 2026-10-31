//! Confidential, deterministic envelopes for cache-independent Nab retrieval.

use crate::EncryptionKey;
use aws_lc_rs::{
    aead::{AES_256_GCM_SIV, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey},
    hmac,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use scryer_application::{AppError, AppResult, error_reasons::indexer_clients as reasons};

const PREFIX: &str = "nab1.";
const CONTEXT: &[u8] = b"scryer-indexer-artifact-reference-v1";

/// Uses the locked AWS-LC AES-GCM-SIV implementation (RFC 8452), never the
/// random-nonce AES-GCM configuration envelope or its plaintext read fallback.
/// Equality for identical authenticated payloads is intentional: consumers
/// persist enclosure URLs and use their equality to stop repeated RSS walks.
pub struct ArtifactReferenceCipher {
    key: LessSafeKey,
    nonce_key: hmac::Key,
}

impl std::fmt::Debug for ArtifactReferenceCipher {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ArtifactReferenceCipher")
            .finish_non_exhaustive()
    }
}

impl ArtifactReferenceCipher {
    pub fn new(master: &EncryptionKey) -> Self {
        let root = hmac::Key::new(hmac::HMAC_SHA256, &master.key_bytes);
        let encryption = hmac::sign(&root, b"scryer-indexer-artifact-encryption-v1");
        let nonce = hmac::sign(&root, b"scryer-indexer-artifact-nonce-v1");
        Self {
            key: LessSafeKey::new(
                UnboundKey::new(&AES_256_GCM_SIV, encryption.as_ref())
                    .expect("SHA-256 produces an AES-256 key"),
            ),
            nonce_key: hmac::Key::new(hmac::HMAC_SHA256, nonce.as_ref()),
        }
    }

    pub fn seal(&self, payload: &[u8]) -> AppResult<String> {
        // Distinct complete payloads get pseudorandom nonces, rather than a
        // fixed nonce per release identity. Changing URL or authorization
        // context changes the nonce input. GCM-SIV additionally tolerates
        // accidental collisions in the truncated 96-bit nonce.
        let digest = hmac::sign(&self.nonce_key, payload);
        let mut nonce_bytes = [0u8; NONCE_LEN];
        nonce_bytes.copy_from_slice(&digest.as_ref()[..NONCE_LEN]);
        let mut encrypted = payload.to_vec();
        self.key
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(nonce_bytes),
                Aad::from(CONTEXT),
                &mut encrypted,
            )
            .map_err(|_| AppError::Repository("Artifact reference encryption failed".into()))?;
        let mut envelope = Vec::with_capacity(NONCE_LEN + encrypted.len());
        envelope.extend_from_slice(&nonce_bytes);
        envelope.extend_from_slice(&encrypted);
        Ok(format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(envelope)))
    }

    pub fn open(&self, reference: &str) -> AppResult<Vec<u8>> {
        let encoded = reference.strip_prefix(PREFIX).ok_or_else(invalid)?;
        let mut envelope = URL_SAFE_NO_PAD.decode(encoded).map_err(|_| invalid())?;
        if envelope.len() < NONCE_LEN + AES_256_GCM_SIV.tag_len() {
            return Err(invalid());
        }
        let (nonce, encrypted) = envelope.split_at_mut(NONCE_LEN);
        let nonce = Nonce::try_assume_unique_for_key(nonce).map_err(|_| invalid())?;
        let plaintext = self
            .key
            .open_in_place(nonce, Aad::from(CONTEXT), encrypted)
            .map_err(|_| invalid())?;
        Ok(plaintext.to_vec())
    }
}

fn invalid() -> AppError {
    AppError::unauthorized_reason(reasons::REFERENCE_INVALID, "Invalid artifact reference")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_are_stable_confidential_and_bound_to_complete_payload_and_instance() {
        let master = EncryptionKey::from_bytes([42; 32]);
        let cipher = ArtifactReferenceCipher::new(&master);
        let payload = br#"{"client":"fixture","generation":1,"url":"https://indexer.example/get?apikey=secret"}"#;
        let reference = cipher.seal(payload).unwrap();
        assert_eq!(
            reference,
            ArtifactReferenceCipher::new(&master).seal(payload).unwrap()
        );
        assert!(!reference.contains("secret") && !reference.contains("indexer.example"));
        assert_eq!(cipher.open(&reference).unwrap(), payload);
        let changed = br#"{"client":"fixture","generation":2,"url":"https://indexer.example/get?apikey=secret"}"#;
        assert_ne!(reference, cipher.seal(changed).unwrap());
        assert_eq!(
            ArtifactReferenceCipher::new(&EncryptionKey::from_bytes([43; 32]))
                .open(&reference)
                .unwrap_err()
                .reason(),
            Some(reasons::REFERENCE_INVALID)
        );
    }

    #[test]
    fn plaintext_wrong_versions_truncation_and_tampering_never_fall_back() {
        let cipher = ArtifactReferenceCipher::new(&EncryptionKey::from_bytes([42; 32]));
        for input in [
            "https://indexer.example/get?apikey=secret",
            "{}",
            "enc:v1:AAAA",
            "nab2.AAAA",
            "nab1.AAAA",
            "nab1.++++",
        ] {
            assert_eq!(
                cipher.open(input).unwrap_err().reason(),
                Some(reasons::REFERENCE_INVALID)
            );
        }
        let reference = cipher.seal(b"fixture payload").unwrap();
        let original = URL_SAFE_NO_PAD
            .decode(reference.strip_prefix(PREFIX).unwrap())
            .unwrap();
        for position in [0, NONCE_LEN, original.len() - 1] {
            let mut corrupted = original.clone();
            corrupted[position] ^= 1;
            assert_eq!(
                cipher
                    .open(&format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(corrupted)))
                    .unwrap_err()
                    .reason(),
                Some(reasons::REFERENCE_INVALID)
            );
        }
    }
}

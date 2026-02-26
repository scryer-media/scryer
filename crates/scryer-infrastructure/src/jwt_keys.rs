use p256::ecdsa::SigningKey;
use p256::pkcs8::{DecodePrivateKey, EncodePrivateKey, EncodePublicKey};

const JWT_PRIVATE_KEY_SETTING: &str = "jwt.private_key";
const SETTINGS_SCOPE_SYSTEM: &str = "system";

/// Ensure a JWT EC P-256 private key exists.
///
/// Priority: env var > DB setting > auto-generate.
/// Returns `(private_pem, public_pem)`.
pub async fn ensure_jwt_keys(
    db: &crate::SqliteServices,
    env_private_pem: Option<String>,
) -> Result<(String, String), String> {
    // 1. Env var provided → validate, persist, derive public
    if let Some(priv_pem) = env_private_pem {
        let signing_key = SigningKey::from_pkcs8_pem(&priv_pem)
            .map_err(|e| format!("SCRYER_JWT_EC_PRIVATE_PEM is not a valid PKCS#8 EC P-256 key: {e}"))?;
        let public_pem = signing_key
            .verifying_key()
            .to_public_key_pem(p256::pkcs8::LineEnding::LF)
            .map_err(|e| format!("failed to derive EC public key: {e}"))?;

        db.upsert_setting_value(
            SETTINGS_SCOPE_SYSTEM,
            JWT_PRIVATE_KEY_SETTING,
            None,
            serde_json::to_string(&priv_pem).unwrap(),
            "env",
            None,
        )
        .await
        .map_err(|e| format!("failed to persist JWT private key from env: {e}"))?;

        tracing::info!("using JWT EC private key from SCRYER_JWT_EC_PRIVATE_PEM");
        return Ok((priv_pem, public_pem));
    }

    // 2. Check DB (tolerate decryption failures — just regenerate)
    match db
        .get_setting_with_defaults(SETTINGS_SCOPE_SYSTEM, JWT_PRIVATE_KEY_SETTING, None)
        .await
    {
        Ok(record) => {
            let existing = record
                .as_ref()
                .and_then(|r| r.value_json.as_deref())
                .and_then(parse_string_json);

            if let Some(priv_pem) = existing {
                if !priv_pem.is_empty() {
                    let signing_key = SigningKey::from_pkcs8_pem(&priv_pem)
                        .map_err(|e| format!("JWT private key in database is invalid: {e}"))?;
                    let public_pem = signing_key
                        .verifying_key()
                        .to_public_key_pem(p256::pkcs8::LineEnding::LF)
                        .map_err(|e| format!("failed to derive EC public key: {e}"))?;

                    tracing::info!("loaded JWT EC private key from database");
                    return Ok((priv_pem, public_pem));
                }
            }
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                "could not read JWT key from database (encryption key changed?), generating new key pair"
            );
        }
    }

    // 3. Generate new key pair
    let signing_key = SigningKey::random(&mut rand_core::OsRng);
    let private_pem = signing_key
        .to_pkcs8_pem(p256::pkcs8::LineEnding::LF)
        .map_err(|e| format!("failed to encode JWT EC private key: {e}"))?
        .to_string();
    let public_pem = signing_key
        .verifying_key()
        .to_public_key_pem(p256::pkcs8::LineEnding::LF)
        .map_err(|e| format!("failed to encode JWT EC public key: {e}"))?;

    tracing::info!("generated new JWT EC key pair (persisted to database)");

    db.upsert_setting_value(
        SETTINGS_SCOPE_SYSTEM,
        JWT_PRIVATE_KEY_SETTING,
        None,
        serde_json::to_string(&private_pem).unwrap(),
        "auto-generated",
        None,
    )
    .await
    .map_err(|e| format!("failed to persist JWT private key: {e}"))?;

    Ok((private_pem, public_pem))
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

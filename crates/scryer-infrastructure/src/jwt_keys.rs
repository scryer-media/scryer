use base64::{engine::general_purpose::STANDARD, Engine};
use ring::rand::{SecureRandom, SystemRandom};

const JWT_HMAC_SECRET_SETTING: &str = "jwt.hmac_secret";
const SETTINGS_SCOPE_SYSTEM: &str = "system";

/// Ensure a JWT HMAC-SHA-512 secret exists.
///
/// Priority: `SCRYER_JWT_HMAC_SECRET` env var > DB setting > auto-generate (64 random bytes).
pub async fn ensure_jwt_hmac_secret(
    db: &crate::SqliteServices,
    env_secret: Option<String>,
) -> Result<String, String> {
    // 1. Env var provided → validate and persist
    if let Some(secret_b64) = env_secret {
        let secret_b64 = secret_b64.trim().to_string();
        if !secret_b64.is_empty() {
            validate_hmac_secret(&secret_b64)?;

            db.upsert_setting_value(
                SETTINGS_SCOPE_SYSTEM,
                JWT_HMAC_SECRET_SETTING,
                None,
                serde_json::to_string(&secret_b64).unwrap(),
                "env",
                None,
            )
            .await
            .map_err(|e| format!("failed to persist JWT HMAC secret from env: {e}"))?;

            tracing::info!("using JWT HMAC secret from SCRYER_JWT_HMAC_SECRET");
            return Ok(secret_b64);
        }
    }

    // 2. Check DB
    match db
        .get_setting_with_defaults(SETTINGS_SCOPE_SYSTEM, JWT_HMAC_SECRET_SETTING, None)
        .await
    {
        Ok(record) => {
            let existing = record
                .as_ref()
                .and_then(|r| r.value_json.as_deref())
                .and_then(parse_string_json);

            if let Some(secret_b64) = existing {
                if !secret_b64.is_empty() {
                    validate_hmac_secret(&secret_b64)
                        .map_err(|e| format!("JWT HMAC secret in database is invalid: {e}"))?;
                    tracing::info!("loaded JWT HMAC secret from database");
                    return Ok(secret_b64);
                }
            }
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                "could not read JWT HMAC secret from database, generating new secret"
            );
        }
    }

    // 3. Generate new 64-byte secret
    let rng = SystemRandom::new();
    let mut bytes = [0u8; 64];
    rng.fill(&mut bytes)
        .map_err(|_| "failed to generate random JWT HMAC secret".to_string())?;
    let secret_b64 = STANDARD.encode(bytes);

    tracing::warn!(
        "generated new JWT HMAC secret — all existing sessions are invalidated. \
         To preserve it across upgrades, set:\n\n  SCRYER_JWT_HMAC_SECRET: {}\n",
        secret_b64
    );

    db.upsert_setting_value(
        SETTINGS_SCOPE_SYSTEM,
        JWT_HMAC_SECRET_SETTING,
        None,
        serde_json::to_string(&secret_b64).unwrap(),
        "auto-generated",
        None,
    )
    .await
    .map_err(|e| format!("failed to persist JWT HMAC secret: {e}"))?;

    Ok(secret_b64)
}

fn validate_hmac_secret(b64: &str) -> Result<(), String> {
    let bytes = STANDARD
        .decode(b64.trim())
        .map_err(|e| format!("JWT HMAC secret is not valid base64: {e}"))?;
    if bytes.len() < 32 {
        return Err(format!(
            "JWT HMAC secret must be at least 32 bytes, got {}",
            bytes.len()
        ));
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

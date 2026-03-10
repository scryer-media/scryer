use chrono::{DateTime, Utc};
use serde::Deserialize;
use tracing::info;

const SETTINGS_SCOPE_SYSTEM: &str = "system";
const RENEWAL_THRESHOLD_DAYS: i64 = 30;

/// Cached enrollment state for the current Scryer instance.
pub struct EnrollmentState {
    pub instance_id: String,
    pub client_key_pem: String,
    pub client_cert_pem: String,
    pub ca_cert_pem: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Deserialize)]
struct RegisterResponse {
    certificate: String,
    expires_at: String,
    ca_certificate: String,
}

/// Load or generate the instance ID (UUIDv4) for this Scryer instance.
pub async fn ensure_instance_id(db: &crate::SqliteServices) -> Result<String, String> {
    let existing = load_setting(db, "smg.instance_id").await?;

    if let Some(id) = existing {
        if !id.is_empty() {
            return Ok(id);
        }
    }

    let instance_id = uuid::Uuid::new_v4().to_string();
    info!(instance_id = %instance_id, "generated new SMG instance ID");

    persist_setting(db, "smg.instance_id", &instance_id).await?;

    Ok(instance_id)
}

/// Load existing enrollment from DB, or enroll with SMG if missing/expired.
///
/// Follows the same ensure pattern as `jwt_keys::ensure_jwt_hmac_secret`.
pub async fn ensure_enrolled(
    db: &crate::SqliteServices,
    registration_url: &str,
    registration_secret: &str,
    ca_cert_override: Option<&str>,
) -> Result<EnrollmentState, String> {
    let key = load_setting(db, "smg.client_key").await?;
    let cert = load_setting(db, "smg.client_cert").await?;
    let expires_str = load_setting(db, "smg.cert_expires_at").await?;
    let ca_cert = load_setting(db, "smg.ca_cert").await?;

    if let (Some(key), Some(cert), Some(expires_str), Some(ca_cert)) =
        (key, cert, expires_str, ca_cert)
    {
        if let Ok(expires_at) = expires_str.parse::<DateTime<Utc>>() {
            let days_remaining = (expires_at - Utc::now()).num_days();
            if days_remaining > RENEWAL_THRESHOLD_DAYS {
                return Ok(EnrollmentState {
                    instance_id: ensure_instance_id(db).await?,
                    client_key_pem: key,
                    client_cert_pem: cert,
                    ca_cert_pem: ca_cert,
                    expires_at,
                });
            }
            info!(days_remaining, "SMG cert expiring soon, re-enrolling");
        }
    }

    let instance_id = ensure_instance_id(db).await?;
    enroll_with_smg(
        db,
        &instance_id,
        registration_url,
        registration_secret,
        ca_cert_override,
    )
    .await
}

async fn enroll_with_smg(
    db: &crate::SqliteServices,
    instance_id: &str,
    registration_url: &str,
    registration_secret: &str,
    ca_cert_override: Option<&str>,
) -> Result<EnrollmentState, String> {
    // Generate EC P-256 keypair
    let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
        .map_err(|e| format!("failed to generate EC P-256 keypair: {e}"))?;
    let private_key_pem = key_pair.serialize_pem();

    // Create CSR with CN=instance_id, O="scryer"
    let mut params = rcgen::CertificateParams::default();
    params.distinguished_name = rcgen::DistinguishedName::new();
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, instance_id);
    params
        .distinguished_name
        .push(rcgen::DnType::OrganizationName, "scryer");

    let csr = params
        .serialize_request(&key_pair)
        .map_err(|e| format!("failed to create CSR: {e}"))?;
    let csr_pem = csr
        .pem()
        .map_err(|e| format!("failed to serialize CSR to PEM: {e}"))?;

    // POST to SMG registration endpoint
    let mut builder = reqwest::Client::builder().timeout(std::time::Duration::from_secs(30));
    if let Some(ca_pem) = ca_cert_override {
        let cert = reqwest::Certificate::from_pem(ca_pem.as_bytes())
            .map_err(|e| format!("failed to parse SCRYER_SMG_CA_CERT: {e}"))?;
        builder = builder.add_root_certificate(cert);
    }
    let http = builder
        .build()
        .map_err(|e| format!("failed to build HTTP client for enrollment: {e}"))?;

    let response = http
        .post(registration_url)
        .json(&serde_json::json!({
            "csr": csr_pem,
            "version": env!("CARGO_PKG_VERSION"),
            "registration_secret": registration_secret,
        }))
        .send()
        .await
        .map_err(|e| format!("SMG registration request failed: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("SMG registration failed (HTTP {status}): {body}"));
    }

    let reg: RegisterResponse = response
        .json()
        .await
        .map_err(|e| format!("failed to parse SMG registration response: {e}"))?;

    let expires_at = reg
        .expires_at
        .parse::<DateTime<Utc>>()
        .map_err(|e| format!("invalid expires_at in registration response: {e}"))?;

    validate_certificate(&reg.certificate, instance_id)?;

    // Persist all enrollment data (smg.client_key is sensitive → auto-encrypted by DB layer)
    persist_setting(db, "smg.client_key", &private_key_pem).await?;
    persist_setting(db, "smg.client_cert", &reg.certificate).await?;
    persist_setting(db, "smg.cert_expires_at", &reg.expires_at).await?;
    persist_setting(db, "smg.ca_cert", &reg.ca_certificate).await?;

    info!(
        instance_id,
        expires_at = %expires_at,
        "enrolled with SMG"
    );

    Ok(EnrollmentState {
        instance_id: instance_id.to_string(),
        client_key_pem: private_key_pem,
        client_cert_pem: reg.certificate,
        ca_cert_pem: reg.ca_certificate,
        expires_at,
    })
}

/// Validate the signed certificate CN matches our instance ID.
fn validate_certificate(cert_pem: &str, expected_cn: &str) -> Result<(), String> {
    let (_, pem) = x509_parser::pem::parse_x509_pem(cert_pem.as_bytes())
        .map_err(|e| format!("failed to parse certificate PEM: {e}"))?;
    let (_, cert) = x509_parser::parse_x509_certificate(&pem.contents)
        .map_err(|e| format!("failed to parse certificate DER: {e}"))?;

    let cn = cert
        .subject()
        .iter_common_name()
        .next()
        .and_then(|attr| attr.as_str().ok())
        .unwrap_or("");
    if cn != expected_cn {
        return Err(format!(
            "certificate CN mismatch: expected '{expected_cn}', got '{cn}'"
        ));
    }

    Ok(())
}

/// Build a `reqwest::Identity` from the enrollment state (key + cert PEM bundle).
pub fn build_mtls_identity(state: &EnrollmentState) -> Result<reqwest::Identity, String> {
    let combined = format!("{}\n{}", state.client_key_pem, state.client_cert_pem);
    reqwest::Identity::from_pem(combined.as_bytes())
        .map_err(|e| format!("failed to build mTLS identity: {e}"))
}

/// Parse the CA certificate PEM into a `reqwest::Certificate` for TLS root store.
pub fn build_ca_certificate(state: &EnrollmentState) -> Result<reqwest::Certificate, String> {
    reqwest::Certificate::from_pem(state.ca_cert_pem.as_bytes())
        .map_err(|e| format!("failed to parse CA certificate: {e}"))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn load_setting(db: &crate::SqliteServices, key: &str) -> Result<Option<String>, String> {
    let record = db
        .get_setting_with_defaults(SETTINGS_SCOPE_SYSTEM, key, None)
        .await
        .map_err(|e| format!("failed to read {key}: {e}"))?;
    Ok(record
        .as_ref()
        .and_then(|r| r.value_json.as_deref())
        .and_then(parse_string_json))
}

async fn persist_setting(db: &crate::SqliteServices, key: &str, value: &str) -> Result<(), String> {
    db.upsert_setting_value(
        SETTINGS_SCOPE_SYSTEM,
        key,
        None,
        serde_json::to_string(value).unwrap(),
        "smg-enrollment",
        None,
    )
    .await
    .map(|_| ())
    .map_err(|e| format!("failed to persist {key}: {e}"))
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

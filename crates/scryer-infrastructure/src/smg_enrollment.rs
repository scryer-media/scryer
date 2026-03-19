use chrono::{DateTime, Utc};
use serde::Deserialize;
use tracing::info;

const SETTINGS_SCOPE_SYSTEM: &str = "system";
const RENEWAL_THRESHOLD_DAYS: i64 = 30;

/// Returned when SMG rejects registration due to version incompatibility.
#[derive(Debug, Clone)]
pub struct VersionIncompatible {
    pub minimum_version: String,
    pub your_version: String,
    pub message: String,
}

/// Errors that can occur during SMG enrollment.
#[derive(Debug)]
pub enum EnrollmentError {
    VersionIncompatible(VersionIncompatible),
    Other(String),
}

impl std::fmt::Display for EnrollmentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::VersionIncompatible(v) => write!(
                f,
                "version incompatible: minimum={}, yours={}, message={}",
                v.minimum_version, v.your_version, v.message
            ),
            Self::Other(s) => f.write_str(s),
        }
    }
}

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
    #[serde(default)]
    opensubtitles_api_key: Option<String>,
}

/// Load or generate the instance ID (UUIDv4) for this Scryer instance.
pub async fn ensure_instance_id(db: &crate::SqliteServices) -> Result<String, String> {
    let existing = load_setting(db, "smg.instance_id").await?;

    if let Some(id) = existing
        && !id.is_empty()
    {
        return Ok(id);
    }

    let instance_id = uuid::Uuid::new_v4().to_string();
    info!(instance_id = %instance_id, "generated new SMG instance ID");

    persist_setting(db, "smg.instance_id", &instance_id).await?;

    Ok(instance_id)
}

/// Clear cached enrollment data from the database so the next call to
/// `ensure_enrolled` performs a fresh registration.
pub async fn clear_enrollment_cache(db: &crate::SqliteServices) -> Result<(), String> {
    for key in &[
        "smg.client_key",
        "smg.client_cert",
        "smg.cert_expires_at",
        "smg.ca_cert",
    ] {
        persist_setting(db, key, "").await?;
    }
    Ok(())
}

/// Load existing enrollment from DB, or enroll with SMG if missing/expired.
pub async fn ensure_enrolled(
    db: &crate::SqliteServices,
    registration_url: &str,
    registration_secret: &str,
    ca_cert_override: Option<&str>,
) -> Result<EnrollmentState, EnrollmentError> {
    let key = load_setting(db, "smg.client_key")
        .await
        .map_err(EnrollmentError::Other)?;
    let cert = load_setting(db, "smg.client_cert")
        .await
        .map_err(EnrollmentError::Other)?;
    let expires_str = load_setting(db, "smg.cert_expires_at")
        .await
        .map_err(EnrollmentError::Other)?;
    let ca_cert = load_setting(db, "smg.ca_cert")
        .await
        .map_err(EnrollmentError::Other)?;

    if let (Some(key), Some(cert), Some(expires_str), Some(ca_cert)) =
        (key, cert, expires_str, ca_cert)
        && let Ok(expires_at) = expires_str.parse::<DateTime<Utc>>()
    {
        let days_remaining = (expires_at - Utc::now()).num_days();
        if days_remaining > RENEWAL_THRESHOLD_DAYS {
            let instance_id = ensure_instance_id(db)
                .await
                .map_err(EnrollmentError::Other)?;
            let ca_cn = extract_pem_cn(&ca_cert).unwrap_or_default();
            let cert_cn = extract_pem_cn(&cert).unwrap_or_default();
            info!(
                %instance_id,
                days_remaining,
                %expires_at,
                cert_cn,
                ca_cn,
                "using cached SMG enrollment (skipping /api/register)"
            );
            return Ok(EnrollmentState {
                instance_id,
                client_key_pem: key,
                client_cert_pem: cert,
                ca_cert_pem: ca_cert,
                expires_at,
            });
        }
        info!(days_remaining, "SMG cert expiring soon, re-enrolling");
    }

    let instance_id = ensure_instance_id(db)
        .await
        .map_err(EnrollmentError::Other)?;
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
) -> Result<EnrollmentState, EnrollmentError> {
    // Generate EC P-256 keypair
    let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
        .map_err(|e| EnrollmentError::Other(format!("failed to generate EC P-256 keypair: {e}")))?;
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
        .map_err(|e| EnrollmentError::Other(format!("failed to create CSR: {e}")))?;
    let csr_pem = csr
        .pem()
        .map_err(|e| EnrollmentError::Other(format!("failed to serialize CSR to PEM: {e}")))?;

    // POST to SMG registration endpoint
    let mut builder = reqwest::Client::builder().timeout(std::time::Duration::from_secs(30));
    if let Some(ca_pem) = ca_cert_override {
        let cert = reqwest::Certificate::from_pem(ca_pem.as_bytes()).map_err(|e| {
            EnrollmentError::Other(format!("failed to parse SCRYER_SMG_CA_CERT: {e}"))
        })?;
        builder = builder.add_root_certificate(cert);
    }
    let http = builder.build().map_err(|e| {
        EnrollmentError::Other(format!("failed to build HTTP client for enrollment: {e}"))
    })?;

    let response = http
        .post(registration_url)
        .json(&serde_json::json!({
            "csr": csr_pem,
            "version": env!("CARGO_PKG_VERSION"),
            "registration_secret": registration_secret,
        }))
        .send()
        .await
        .map_err(|e| EnrollmentError::Other(format!("SMG registration request failed: {e}")))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();

        // Check for structured version incompatibility response (HTTP 422)
        if status.as_u16() == 422
            && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&body)
            && parsed.get("error").and_then(|v| v.as_str()) == Some("version_incompatible")
        {
            return Err(EnrollmentError::VersionIncompatible(VersionIncompatible {
                minimum_version: parsed
                    .get("minimum_version")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string(),
                your_version: parsed
                    .get("your_version")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string(),
                message: parsed
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            }));
        }

        return Err(EnrollmentError::Other(format!(
            "SMG registration failed (HTTP {status}): {body}"
        )));
    }

    let reg: RegisterResponse = response.json().await.map_err(|e| {
        EnrollmentError::Other(format!("failed to parse SMG registration response: {e}"))
    })?;

    let expires_at = reg.expires_at.parse::<DateTime<Utc>>().map_err(|e| {
        EnrollmentError::Other(format!("invalid expires_at in registration response: {e}"))
    })?;

    validate_certificate(&reg.certificate, instance_id).map_err(EnrollmentError::Other)?;

    // Persist all enrollment data (smg.client_key is sensitive → auto-encrypted by DB layer)
    persist_setting(db, "smg.client_key", &private_key_pem)
        .await
        .map_err(EnrollmentError::Other)?;
    persist_setting(db, "smg.client_cert", &reg.certificate)
        .await
        .map_err(EnrollmentError::Other)?;
    persist_setting(db, "smg.cert_expires_at", &reg.expires_at)
        .await
        .map_err(EnrollmentError::Other)?;
    persist_setting(db, "smg.ca_cert", &reg.ca_certificate)
        .await
        .map_err(EnrollmentError::Other)?;

    // Persist OpenSubtitles API key if provided by SMG
    if let Some(os_key) = &reg.opensubtitles_api_key
        && !os_key.is_empty()
    {
        persist_setting(db, "subtitles.opensubtitles_api_key", os_key)
            .await
            .map_err(EnrollmentError::Other)?;
        info!("OpenSubtitles API key received from SMG");
    }

    let ca_cn = extract_pem_cn(&reg.ca_certificate).unwrap_or_default();
    let cert_issuer = extract_pem_issuer_cn(&reg.certificate).unwrap_or_default();
    info!(
        instance_id,
        expires_at = %expires_at,
        ca_cn,
        cert_issuer,
        "enrolled with SMG (fresh registration)"
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

/// Sign a request for application-layer instance authentication.
///
/// Constructs message `"{timestamp}:{body_hash}"` and signs with ECDSA P-256 SHA-256.
/// Returns a base64-encoded ASN.1 DER signature.
///
/// The verifier (SMG) computes SHA-256 of the same message and calls
/// `ecdsa.VerifyASN1(pubKey, sha256(message), signature)`. The p256 `Signer`
/// trait internally hashes with SHA-256 before signing, so both sides agree on
/// the digest: `SHA-256("{timestamp}:{body_hash}")`.
pub fn sign_request(
    private_key_pem: &str,
    timestamp: i64,
    body_hash: &str,
) -> Result<String, String> {
    use base64::Engine as _;
    use p256::ecdsa::{DerSignature, SigningKey, signature::Signer};
    use p256::pkcs8::DecodePrivateKey;

    let signing_key = SigningKey::from_pkcs8_pem(private_key_pem)
        .map_err(|e| format!("failed to parse private key for signing: {e}"))?;

    let message = format!("{timestamp}:{body_hash}");
    let signature: DerSignature = signing_key.sign(message.as_bytes());

    Ok(base64::engine::general_purpose::STANDARD.encode(signature.as_ref()))
}

/// Convert a PEM-encoded certificate to base64-encoded DER for the `X-Scryer-Cert` header.
pub fn cert_pem_to_base64_der(cert_pem: &str) -> Result<String, String> {
    use base64::Engine as _;

    let (_, pem) = x509_parser::pem::parse_x509_pem(cert_pem.as_bytes())
        .map_err(|e| format!("failed to parse certificate PEM: {e}"))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(&pem.contents))
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

/// Extract the Subject CN from a PEM-encoded certificate for logging.
fn extract_pem_cn(pem_str: &str) -> Option<String> {
    let (_, pem) = x509_parser::pem::parse_x509_pem(pem_str.as_bytes()).ok()?;
    let (_, cert) = x509_parser::parse_x509_certificate(&pem.contents).ok()?;

    cert.subject()
        .iter_common_name()
        .next()
        .and_then(|attr| attr.as_str().ok())
        .map(|s| s.to_string())
}

/// Extract the Issuer CN from a PEM-encoded certificate for logging.
fn extract_pem_issuer_cn(pem_str: &str) -> Option<String> {
    let (_, pem) = x509_parser::pem::parse_x509_pem(pem_str.as_bytes()).ok()?;
    let (_, cert) = x509_parser::parse_x509_certificate(&pem.contents).ok()?;

    cert.issuer()
        .iter_common_name()
        .next()
        .and_then(|attr| attr.as_str().ok())
        .map(|s| s.to_string())
}

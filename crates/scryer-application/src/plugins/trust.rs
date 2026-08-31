use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

use base64::Engine;
use const_oid::db::rfc5280::ID_KP_CODE_SIGNING;
use rustls_pki_types::{CertificateDer, TrustAnchor, UnixTime};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sigstore::{
    crypto::{CosignVerificationKey, Signature, SigningScheme},
    trust::{TrustRoot, sigstore::SigstoreTrustRoot},
};
use tokio::sync::Semaphore;
use tracing::debug;
use webpki::{EndEntityCert, KeyUsage};
use x509_cert::{
    Certificate,
    der::{DecodePem, Encode},
    ext::{
        Extension,
        pkix::{SubjectAltName, name::GeneralName},
    },
};

use super::catalog::RequiredSigner;
use crate::{AppError, AppResult};

const SIGSTORE_GITHUB_WORKFLOW_NAME_OID: &str = "1.3.6.1.4.1.57264.1.4";
const SIGSTORE_GITHUB_WORKFLOW_REPOSITORY_OID: &str = "1.3.6.1.4.1.57264.1.5";
const SIGSTORE_GITHUB_WORKFLOW_REF_OID: &str = "1.3.6.1.4.1.57264.1.6";

type RekorVerificationKeys = BTreeMap<String, CosignVerificationKey>;
type FulcioTrustAnchors = Vec<TrustAnchor<'static>>;

struct SigstoreTrustMaterial {
    rekor_keys: Arc<RekorVerificationKeys>,
    fulcio_anchors: Arc<FulcioTrustAnchors>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SignedArtifactBundle {
    pub(super) base64_signature: String,
    pub(super) cert: String,
    pub(super) rekor_bundle: RekorBundle,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(super) struct RekorBundle {
    signed_entry_timestamp: String,
    pub(super) payload: RekorPayload,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RekorPayload {
    pub(super) body: String,
    pub(super) integrated_time: i64,
    pub(super) log_index: i64,
    #[serde(rename = "logID")]
    pub(super) log_id: String,
}

static SIGSTORE_TRUST_MATERIAL: OnceLock<Mutex<Option<Arc<SigstoreTrustMaterial>>>> =
    OnceLock::new();
static VERIFY_LIMIT: OnceLock<Semaphore> = OnceLock::new();

pub async fn verify_signed_blob(
    raw: Vec<u8>,
    bundle_raw: Vec<u8>,
    required_signer: RequiredSigner,
) -> AppResult<()> {
    let permit = VERIFY_LIMIT
        .get_or_init(|| Semaphore::new(2))
        .acquire()
        .await
        .map_err(|_| AppError::Repository("plugin verification worker is closed".to_string()))?;
    let result = tokio::task::spawn_blocking(move || {
        verify_signed_blob_blocking(&raw, &bundle_raw, &required_signer)
    })
    .await
    .map_err(|error| {
        AppError::Repository(format!("plugin signature verification panicked: {error}"))
    })?;
    drop(permit);
    result
}

fn verify_signed_blob_blocking(
    raw: &[u8],
    bundle_raw: &[u8],
    required_signer: &RequiredSigner,
) -> AppResult<()> {
    let bundle_text = std::str::from_utf8(bundle_raw)
        .map_err(|error| AppError::Validation(format!("invalid Sigstore bundle UTF-8: {error}")))?;
    let bundle_text = normalize_sigstore_bundle(bundle_text)?;
    let rekor_keys = cached_rekor_verification_keys()?;
    let bundle = parse_and_verify_bundle(&bundle_text, rekor_keys.as_ref())?;
    let cert_pem = normalize_bundle_cert(&bundle.cert)?;

    verify_rekor_hashedrekord_binding(
        raw,
        &bundle.base64_signature,
        &cert_pem,
        &bundle.rekor_bundle.payload.body,
    )?;
    verify_blob_signature(&cert_pem, &bundle.base64_signature, raw)?;
    verify_fulcio_certificate_chain(&cert_pem, &bundle)?;
    verify_signer_identity(&cert_pem, required_signer)?;
    Ok(())
}

fn parse_and_verify_bundle(
    raw: &str,
    rekor_keys: &RekorVerificationKeys,
) -> AppResult<SignedArtifactBundle> {
    let bundle: SignedArtifactBundle = serde_json::from_str(raw).map_err(|error| {
        AppError::Validation(format!("failed to parse Sigstore bundle: {error}"))
    })?;
    let canonical_payload = serde_json_canonicalizer::to_vec(&bundle.rekor_bundle.payload)
        .map_err(|error| {
            AppError::Validation(format!(
                "failed to canonicalize Sigstore Rekor payload: {error}"
            ))
        })?;
    let rekor_key = rekor_keys
        .get(&bundle.rekor_bundle.payload.log_id)
        .ok_or_else(|| {
            AppError::Validation(format!(
                "Sigstore Rekor public key '{}' is not trusted",
                bundle.rekor_bundle.payload.log_id
            ))
        })?;
    rekor_key
        .verify_signature(
            Signature::Base64Encoded(bundle.rekor_bundle.signed_entry_timestamp.as_bytes()),
            &canonical_payload,
        )
        .map_err(|error| {
            AppError::Validation(format!(
                "Sigstore Rekor bundle verification failed: {error}"
            ))
        })?;
    Ok(bundle)
}

fn verify_blob_signature(cert_pem: &str, base64_signature: &str, raw: &[u8]) -> AppResult<()> {
    let cert = Certificate::from_pem(cert_pem.as_bytes()).map_err(|error| {
        AppError::Validation(format!("failed to parse Sigstore certificate: {error}"))
    })?;
    let subject_public_key_info = cert
        .tbs_certificate()
        .subject_public_key_info()
        .to_der()
        .map_err(|error| {
            AppError::Validation(format!(
                "failed to encode Sigstore certificate public key: {error}"
            ))
        })?;
    let verification_key =
        CosignVerificationKey::try_from_der(&subject_public_key_info).map_err(|error| {
            AppError::Validation(format!(
                "failed to parse Sigstore certificate public key: {error}"
            ))
        })?;
    verification_key
        .verify_signature(Signature::Base64Encoded(base64_signature.as_bytes()), raw)
        .map_err(|error| {
            AppError::Validation(format!(
                "Sigstore blob signature verification failed: {error}"
            ))
        })
}

pub(super) fn verify_rekor_hashedrekord_binding(
    raw: &[u8],
    base64_signature: &str,
    cert_pem: &str,
    base64_rekor_body: &str,
) -> AppResult<()> {
    let body = base64::engine::general_purpose::STANDARD
        .decode(base64_rekor_body.as_bytes())
        .map_err(|error| AppError::Validation(format!("invalid Rekor body encoding: {error}")))?;
    let body: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|error| AppError::Validation(format!("invalid Rekor body JSON: {error}")))?;
    let kind = sigstore_bundle_string_field(&body, &["kind"], "Rekor body kind")?;
    let api_version =
        sigstore_bundle_string_field(&body, &["apiVersion"], "Rekor body apiVersion")?;
    if kind != "hashedrekord" || api_version != "0.0.1" {
        return Err(AppError::Validation(
            "unsupported Rekor body; expected hashedrekord v0.0.1".to_string(),
        ));
    }

    let hash_algorithm = sigstore_bundle_string_field(
        &body,
        &["spec", "data", "hash", "algorithm"],
        "Rekor hashedrekord SHA-256 algorithm",
    )?;
    if !hash_algorithm.eq_ignore_ascii_case("sha256") {
        return Err(AppError::Validation(format!(
            "unsupported Rekor hashedrekord digest algorithm: {hash_algorithm}"
        )));
    }
    let recorded_digest = sigstore_bundle_string_field(
        &body,
        &["spec", "data", "hash", "value"],
        "Rekor hashedrekord digest",
    )?;
    let digest = Sha256::digest(raw);
    let expected_hex = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let expected_base64 = base64::engine::general_purpose::STANDARD.encode(digest);
    if !recorded_digest.eq_ignore_ascii_case(&expected_hex) && recorded_digest != expected_base64 {
        return Err(AppError::Validation(
            "Rekor hashedrekord digest does not match the plugin artifact".to_string(),
        ));
    }

    let recorded_signature = sigstore_bundle_string_field(
        &body,
        &["spec", "signature", "content"],
        "Rekor hashedrekord signature",
    )?;
    let outer_signature = base64::engine::general_purpose::STANDARD
        .decode(base64_signature.as_bytes())
        .map_err(|error| {
            AppError::Validation(format!("invalid bundle signature encoding: {error}"))
        })?;
    let rekor_signature = base64::engine::general_purpose::STANDARD
        .decode(recorded_signature.as_bytes())
        .map_err(|error| {
            AppError::Validation(format!("invalid Rekor signature encoding: {error}"))
        })?;
    if rekor_signature != outer_signature {
        return Err(AppError::Validation(
            "Rekor hashedrekord signature does not match the bundle signature".to_string(),
        ));
    }

    let recorded_certificate = sigstore_bundle_string_field(
        &body,
        &["spec", "signature", "publicKey", "content"],
        "Rekor hashedrekord certificate",
    )?;
    if sigstore_certificate_der(cert_pem)?
        != sigstore_certificate_der(&normalize_bundle_cert(recorded_certificate)?)?
    {
        return Err(AppError::Validation(
            "Rekor hashedrekord certificate does not match the bundle certificate".to_string(),
        ));
    }
    Ok(())
}

fn sigstore_certificate_der(cert_pem: &str) -> AppResult<Vec<u8>> {
    Certificate::from_pem(cert_pem.as_bytes())
        .map_err(|error| {
            AppError::Validation(format!("failed to parse Sigstore certificate: {error}"))
        })?
        .to_der()
        .map_err(|error| {
            AppError::Validation(format!("failed to encode Sigstore certificate: {error}"))
        })
}

fn verify_fulcio_certificate_chain(cert_pem: &str, bundle: &SignedArtifactBundle) -> AppResult<()> {
    let cert = Certificate::from_pem(cert_pem.as_bytes()).map_err(|error| {
        AppError::Validation(format!("failed to parse Sigstore certificate: {error}"))
    })?;
    let cert_der = cert.to_der().map_err(|error| {
        AppError::Validation(format!("failed to encode Sigstore certificate: {error}"))
    })?;
    let cert_der = CertificateDer::from(cert_der.as_slice());
    let end_entity = EndEntityCert::try_from(&cert_der)
        .map_err(|error| AppError::Validation(format!("invalid Sigstore certificate: {error}")))?;
    let verification_time = rekor_integrated_time(bundle.rekor_bundle.payload.integrated_time)?;
    let trust_anchors = cached_fulcio_trust_anchors()?;

    end_entity
        .verify_for_usage(
            webpki::ALL_VERIFICATION_ALGS,
            trust_anchors.as_slice(),
            &[],
            verification_time,
            KeyUsage::required(ID_KP_CODE_SIGNING.as_bytes()),
            None,
            None,
        )
        .map_err(|error| {
            AppError::Validation(format!(
                "Sigstore Fulcio certificate chain verification failed: {error}"
            ))
        })?;
    Ok(())
}

fn rekor_integrated_time(integrated_time: i64) -> AppResult<UnixTime> {
    let integrated_time = u64::try_from(integrated_time)
        .map_err(|_| AppError::Validation("Sigstore Rekor integrated time is negative".into()))?;
    Ok(UnixTime::since_unix_epoch(Duration::from_secs(
        integrated_time,
    )))
}

fn cached_rekor_verification_keys() -> AppResult<Arc<RekorVerificationKeys>> {
    Ok(cached_sigstore_trust_material()?.rekor_keys.clone())
}

fn cached_fulcio_trust_anchors() -> AppResult<Arc<FulcioTrustAnchors>> {
    Ok(cached_sigstore_trust_material()?.fulcio_anchors.clone())
}

fn cached_sigstore_trust_material() -> AppResult<Arc<SigstoreTrustMaterial>> {
    let cache = SIGSTORE_TRUST_MATERIAL.get_or_init(|| Mutex::new(None));
    let mut guard = cache.lock().map_err(|_| {
        AppError::Repository("sigstore trust-root cache lock is poisoned".to_string())
    })?;
    if let Some(cached) = guard.as_ref() {
        return Ok(cached.clone());
    }
    let loaded = Arc::new(load_sigstore_trust_material_blocking()?);
    *guard = Some(loaded.clone());
    Ok(loaded)
}

fn load_sigstore_trust_material_blocking() -> AppResult<SigstoreTrustMaterial> {
    scryer_outbound_http::install_default_rustls_provider();
    let trust_root = tokio::runtime::Handle::current()
        .block_on(SigstoreTrustRoot::new(None))
        .map_err(|error| {
            AppError::Repository(format!("failed to load Sigstore trust root: {error}"))
        })?;
    let rekor_keys = trust_root.rekor_keys().map_err(|error| {
        AppError::Repository(format!(
            "failed to load Sigstore Rekor public keys: {error}"
        ))
    })?;
    let fulcio_certs = trust_root.fulcio_certs().map_err(|error| {
        AppError::Repository(format!(
            "failed to load Sigstore Fulcio certificates: {error}"
        ))
    })?;
    let anchors = fulcio_certs
        .iter()
        .map(|cert| {
            webpki::anchor_from_trusted_cert(cert)
                .map(|anchor| anchor.to_owned())
                .map_err(|error| AppError::Repository(error.to_string()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if anchors.is_empty() {
        return Err(AppError::Repository(
            "Sigstore Fulcio trust root is empty".to_string(),
        ));
    }
    Ok(SigstoreTrustMaterial {
        rekor_keys: Arc::new(parse_rekor_verification_keys(rekor_keys)?),
        fulcio_anchors: Arc::new(anchors),
    })
}

pub async fn prime_sigstore_trust_roots() -> AppResult<()> {
    tokio::task::spawn_blocking(cached_sigstore_trust_material)
        .await
        .map_err(|error| {
            AppError::Repository(format!("sigstore trust-root priming panicked: {error}"))
        })?
        .map(|_| ())
}

pub(super) fn parse_rekor_verification_keys(
    keys: BTreeMap<String, &[u8]>,
) -> AppResult<RekorVerificationKeys> {
    let parsed = keys
        .into_iter()
        .filter_map(|(key_id, key)| {
            match CosignVerificationKey::from_der(key, &SigningScheme::default()) {
                Ok(key) => Some((key_id, key)),
                Err(error) => {
                    debug!(%key_id, %error, "skipping unsupported Rekor public key");
                    None
                }
            }
        })
        .collect::<BTreeMap<_, _>>();
    if parsed.is_empty() {
        return Err(AppError::Repository(
            "failed to parse any Rekor public keys from the Sigstore trust root".to_string(),
        ));
    }
    Ok(parsed)
}

fn verify_signer_identity(cert_pem: &str, required_signer: &RequiredSigner) -> AppResult<()> {
    let cert = Certificate::from_pem(cert_pem.as_bytes()).map_err(|error| {
        AppError::Validation(format!("failed to parse Sigstore certificate: {error}"))
    })?;
    let repository = cert_extension_utf8(&cert, SIGSTORE_GITHUB_WORKFLOW_REPOSITORY_OID)?;
    if repository.as_deref() != Some(required_signer.github_repository.as_str()) {
        return Err(AppError::Validation(format!(
            "Sigstore signer repo mismatch: expected '{}', got '{}'",
            required_signer.github_repository,
            repository.unwrap_or_else(|| "<missing>".to_string())
        )));
    }

    if let Some(expected_workflow) = required_signer.github_workflow.as_deref() {
        let workflow_name = cert_extension_utf8(&cert, SIGSTORE_GITHUB_WORKFLOW_NAME_OID)?;
        let workflow_ref = cert_extension_utf8(&cert, SIGSTORE_GITHUB_WORKFLOW_REF_OID)?;
        let subject_uri = cert_subject_uri(&cert)?;
        let matched = workflow_name.as_deref() == Some(expected_workflow)
            || workflow_ref
                .as_deref()
                .is_some_and(|value| value.contains(expected_workflow))
            || subject_uri
                .as_deref()
                .is_some_and(|value| value.contains(expected_workflow));
        if !matched {
            return Err(AppError::Validation(format!(
                "Sigstore workflow mismatch for '{}'",
                required_signer.github_repository
            )));
        }
    }
    Ok(())
}

pub(super) fn normalize_sigstore_bundle(bundle_text: &str) -> AppResult<String> {
    let Ok(bundle_json) = serde_json::from_str::<serde_json::Value>(bundle_text) else {
        return Ok(bundle_text.to_string());
    };
    if bundle_json.get("base64Signature").is_some() || bundle_json.get("messageSignature").is_none()
    {
        return Ok(bundle_text.to_string());
    }

    let tlog_entry = sigstore_bundle_value(&bundle_json, &["verificationMaterial", "tlogEntries"])
        .and_then(|value| value.as_array())
        .and_then(|entries| entries.first())
        .ok_or_else(|| {
            AppError::Validation(
                "Sigstore bundle missing verificationMaterial.tlogEntries[0]".to_string(),
            )
        })?;
    let cert_pem = normalize_bundle_cert(sigstore_bundle_string_field(
        &bundle_json,
        &["verificationMaterial", "certificate", "rawBytes"],
        "verificationMaterial.certificate.rawBytes",
    )?)?;

    serde_json::to_string(&serde_json::json!({
        "base64Signature": sigstore_bundle_string_field(
            &bundle_json,
            &["messageSignature", "signature"],
            "messageSignature.signature",
        )?,
        "cert": cert_pem,
        "rekorBundle": {
            "SignedEntryTimestamp": sigstore_bundle_string_field(
                tlog_entry,
                &["inclusionPromise", "signedEntryTimestamp"],
                "verificationMaterial.tlogEntries[0].inclusionPromise.signedEntryTimestamp",
            )?,
            "Payload": {
                "body": sigstore_bundle_string_field(
                    tlog_entry,
                    &["canonicalizedBody"],
                    "verificationMaterial.tlogEntries[0].canonicalizedBody",
                )?,
                "integratedTime": sigstore_bundle_i64_field(
                    tlog_entry,
                    &["integratedTime"],
                    "verificationMaterial.tlogEntries[0].integratedTime",
                )?,
                "logIndex": sigstore_bundle_i64_field(
                    tlog_entry,
                    &["logIndex"],
                    "verificationMaterial.tlogEntries[0].logIndex",
                )?,
                "logID": sigstore_bundle_string_field(
                    tlog_entry,
                    &["logId", "keyId"],
                    "verificationMaterial.tlogEntries[0].logId.keyId",
                )
                .map(normalize_rekor_log_id)?,
            }
        }
    }))
    .map_err(|error| AppError::Validation(format!("failed to normalize Sigstore bundle: {error}")))
}

fn sigstore_bundle_value<'a>(
    value: &'a serde_json::Value,
    path: &[&str],
) -> Option<&'a serde_json::Value> {
    path.iter()
        .try_fold(value, |current, segment| current.get(*segment))
}

fn sigstore_bundle_string_field<'a>(
    value: &'a serde_json::Value,
    path: &[&str],
    label: &str,
) -> AppResult<&'a str> {
    sigstore_bundle_value(value, path)
        .and_then(|value| value.as_str())
        .ok_or_else(|| AppError::Validation(format!("Sigstore bundle missing {label}")))
}

fn sigstore_bundle_i64_field(
    value: &serde_json::Value,
    path: &[&str],
    label: &str,
) -> AppResult<i64> {
    let Some(value) = sigstore_bundle_value(value, path) else {
        return Err(AppError::Validation(format!(
            "Sigstore bundle missing {label}"
        )));
    };
    if let Some(number) = value.as_i64() {
        return Ok(number);
    }
    let Some(number) = value.as_str() else {
        return Err(AppError::Validation(format!(
            "Sigstore bundle {label} is not an integer"
        )));
    };
    number.parse::<i64>().map_err(|error| {
        AppError::Validation(format!(
            "Sigstore bundle {label} is not a valid integer: {error}"
        ))
    })
}

fn normalize_rekor_log_id(key_id: &str) -> String {
    if key_id.len().is_multiple_of(2) && key_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return key_id.to_ascii_lowercase();
    }

    match base64::engine::general_purpose::STANDARD.decode(key_id.as_bytes()) {
        Ok(decoded) => {
            use std::fmt::Write as _;

            let mut hex = String::with_capacity(decoded.len() * 2);
            for byte in decoded {
                let _ = write!(&mut hex, "{byte:02x}");
            }
            hex
        }
        Err(_) => key_id.to_string(),
    }
}

pub(super) fn normalize_bundle_cert(cert: &str) -> AppResult<String> {
    if cert.contains("-----BEGIN CERTIFICATE-----") {
        return Ok(cert.to_string());
    }
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(cert.as_bytes())
        .map_err(|error| {
            AppError::Validation(format!("invalid base64 Sigstore certificate: {error}"))
        })?;
    if let Ok(decoded_text) = String::from_utf8(decoded.clone())
        && decoded_text.contains("-----BEGIN CERTIFICATE-----")
    {
        return Ok(decoded_text);
    }
    Ok(pem_encode_certificate(&decoded))
}

pub(super) fn pem_encode_certificate(der: &[u8]) -> String {
    let base64 = base64::engine::general_purpose::STANDARD.encode(der);
    let mut pem = String::from("-----BEGIN CERTIFICATE-----\n");
    for chunk in base64.as_bytes().chunks(64) {
        pem.push_str(&String::from_utf8_lossy(chunk));
        pem.push('\n');
    }
    pem.push_str("-----END CERTIFICATE-----\n");
    pem
}

fn cert_extension_utf8(cert: &Certificate, oid: &str) -> AppResult<Option<String>> {
    let Some(extensions) = cert.tbs_certificate().extensions() else {
        return Ok(None);
    };
    extensions
        .iter()
        .find(|ext: &&Extension| ext.extn_id.to_string() == oid)
        .map(|ext| {
            String::from_utf8(ext.extn_value.clone().into_bytes().into_vec()).map_err(|_| {
                AppError::Validation(format!(
                    "Sigstore certificate extension {oid} is not valid UTF-8"
                ))
            })
        })
        .transpose()
}

fn cert_subject_uri(cert: &Certificate) -> AppResult<Option<String>> {
    let san = cert
        .tbs_certificate()
        .get_extension::<SubjectAltName>()
        .map_err(|error| AppError::Validation(format!("failed to read certificate SAN: {error}")))?
        .map(|(_, san)| san);
    let Some(san) = san else {
        return Ok(None);
    };
    Ok(san.0.iter().find_map(|name| match name {
        GeneralName::UniformResourceIdentifier(uri) => Some(uri.to_string()),
        _ => None,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signed_rekor_bundle() -> (String, RekorVerificationKeys) {
        let signer = SigningScheme::ECDSA_P256_SHA256_ASN1
            .create_signer()
            .expect("create test Rekor signer");
        let payload = RekorPayload {
            body: base64::engine::general_purpose::STANDARD.encode(b"{}"),
            integrated_time: 1_700_000_000,
            log_index: 42,
            log_id: "test-rekor-key".to_string(),
        };
        let canonical_payload =
            serde_json_canonicalizer::to_vec(&payload).expect("canonicalize test Rekor payload");
        let signed_entry_timestamp = base64::engine::general_purpose::STANDARD.encode(
            signer
                .sign(&canonical_payload)
                .expect("sign test Rekor payload"),
        );
        let bundle = serde_json::json!({
            "base64Signature": "c2lnbmF0dXJl",
            "cert": "certificate",
            "rekorBundle": {
                "SignedEntryTimestamp": signed_entry_timestamp,
                "Payload": payload,
            },
        })
        .to_string();
        let keys = BTreeMap::from([(
            "test-rekor-key".to_string(),
            signer
                .to_verification_key()
                .expect("derive test Rekor verification key"),
        )]);
        (bundle, keys)
    }

    #[test]
    fn rekor_bundle_verifier_accepts_a_valid_signed_entry_timestamp() {
        let (bundle, keys) = signed_rekor_bundle();

        parse_and_verify_bundle(&bundle, &keys).expect("verify signed Rekor bundle");
    }

    #[test]
    fn rekor_bundle_verifier_rejects_an_unknown_log_key() {
        let (bundle, _) = signed_rekor_bundle();

        let error = parse_and_verify_bundle(&bundle, &BTreeMap::new())
            .expect_err("unknown Rekor key must fail closed");
        assert!(error.to_string().contains("is not trusted"));
    }

    #[test]
    fn rekor_bundle_verifier_rejects_an_invalid_signed_entry_timestamp() {
        let (bundle, keys) = signed_rekor_bundle();
        let mut bundle: serde_json::Value =
            serde_json::from_str(&bundle).expect("parse test bundle");
        bundle["rekorBundle"]["SignedEntryTimestamp"] = serde_json::Value::String(
            base64::engine::general_purpose::STANDARD.encode(b"invalid signature"),
        );

        let error = parse_and_verify_bundle(&bundle.to_string(), &keys)
            .expect_err("invalid Rekor signature must fail closed");
        assert!(
            error
                .to_string()
                .contains("Rekor bundle verification failed")
        );
    }

    #[tokio::test]
    async fn trust_root_client_loads_without_sigstores_tls_feature() {
        prime_sigstore_trust_roots()
            .await
            .expect("application-configured AWS-LC Rustls client should load Sigstore trust roots");

        assert!(!cached_rekor_verification_keys().unwrap().is_empty());
        assert!(!cached_fulcio_trust_anchors().unwrap().is_empty());
    }
}

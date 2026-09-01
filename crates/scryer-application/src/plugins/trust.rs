use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock, RwLock},
    time::{Duration, Instant},
};

use aws_lc_rs::signature::{
    ECDSA_P256_SHA256_ASN1, ECDSA_P384_SHA256_ASN1, ECDSA_P521_SHA256_ASN1,
    RSA_PKCS1_2048_8192_SHA256, UnparsedPublicKey,
};
use base64::Engine;
use const_oid::db::rfc5280::{ID_KP_CODE_SIGNING, ID_KP_TIME_STAMPING};
use rustls_pki_types::{CertificateDer, TrustAnchor, UnixTime};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
#[cfg(test)]
use sigstore::crypto::SigningScheme;
use sigstore::{
    crypto::{CosignVerificationKey, Signature},
    trust::sigstore::SigstoreTrustRoot,
};
use sigstore_protobuf_specs::dev::sigstore::{
    bundle::v1::{
        Bundle as ProtoBundle, bundle::Content as ProtoBundleContent,
        verification_material::Content as ProtoVerificationMaterial,
    },
    common::v1::HashAlgorithm as ProtoHashAlgorithm,
    rekor::v1::{InclusionProof as ProtoInclusionProof, TransparencyLogEntry},
};
use tokio::sync::{Mutex as AsyncMutex, Semaphore};
use tracing::{info, warn};
use webpki::{EndEntityCert, KeyUsage};
use x509_cert::{
    Certificate,
    der::{Decode, DecodePem, Encode, asn1::Utf8StringRef},
    ext::{
        Extension,
        pkix::{
            SignedCertificateTimestampList, SubjectAltName,
            name::GeneralName,
            sct::{HashAlgorithm, SignatureAlgorithm},
        },
    },
};

use super::catalog::RequiredSigner;
use crate::{AppError, AppResult};

const SIGSTORE_OIDC_ISSUER_OID: &str = "1.3.6.1.4.1.57264.1.1";
const SIGSTORE_GITHUB_WORKFLOW_REPOSITORY_OID: &str = "1.3.6.1.4.1.57264.1.5";
const SIGSTORE_OIDC_ISSUER_V2_OID: &str = "1.3.6.1.4.1.57264.1.8";
const SIGSTORE_BUILD_SIGNER_URI_OID: &str = "1.3.6.1.4.1.57264.1.9";
const SIGSTORE_SOURCE_REPOSITORY_URI_OID: &str = "1.3.6.1.4.1.57264.1.12";
const SIGSTORE_BUILD_CONFIG_URI_OID: &str = "1.3.6.1.4.1.57264.1.18";
const GITHUB_ACTIONS_OIDC_ISSUER: &str = "https://token.actions.githubusercontent.com";
const SIGSTORE_BUNDLE_V03_MEDIA_TYPE: &str = "application/vnd.dev.sigstore.bundle.v0.3+json";
const SIGSTORE_BUNDLE_V03_LEGACY_MEDIA_TYPE: &str =
    "application/vnd.dev.sigstore.bundle+json;version=0.3";
const OID_CMS_SIGNED_DATA: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x07, 0x02];
const OID_TST_INFO: &[u8] = &[
    0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x09, 0x10, 0x01, 0x04,
];
const OID_SHA256: &[u8] = &[0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01];
const OID_CONTENT_TYPE: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x09, 0x03];
const OID_MESSAGE_DIGEST: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x09, 0x04];
const OID_SIGNING_CERTIFICATE_V2: &[u8] = &[
    0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x09, 0x10, 0x02, 0x2f,
];
const OID_ECDSA_WITH_SHA256: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02];
const OID_RSA_WITH_SHA256: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0b];
const SIGSTORE_TRUST_REFRESH_TIMEOUT: Duration = Duration::from_secs(120);
const SIGSTORE_TRUST_SOURCE: &str = "https://tuf-repo-cdn.sigstore.dev/trusted_root.json";
const EMBEDDED_SIGSTORE_TRUST_ROOT: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../scryer-plugins/builtins/sigstore-trusted-root.json"
));
const EMBEDDED_SIGSTORE_TRUST_ROOT_PROVENANCE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../scryer-plugins/builtins/sigstore-trusted-root.provenance.json"
));

struct TimedVerificationKey {
    key: CosignVerificationKey,
    valid_for: TimeWindow,
    key_details: String,
}

type RekorVerificationKeys = BTreeMap<String, TimedVerificationKey>;
type CtfeVerificationKeys = BTreeMap<String, TimedVerificationKey>;

struct CertificateAuthorityTrustChain {
    anchor: TrustAnchor<'static>,
    anchor_der: CertificateDer<'static>,
    intermediates: Vec<CertificateDer<'static>>,
    valid_for: TimeWindow,
    certificate_validity: Vec<TimeWindow>,
}

#[derive(Clone, Copy, Debug, Default)]
struct TimeWindow {
    start: Option<i64>,
    end: Option<i64>,
}

impl TimeWindow {
    fn contains(self, timestamp: i64) -> bool {
        self.start.is_none_or(|start| timestamp >= start)
            && self.end.is_none_or(|end| timestamp <= end)
    }
}

struct SigstoreTrustMaterial {
    rekor_keys: RekorVerificationKeys,
    ctfe_keys: CtfeVerificationKeys,
    fulcio_chains: Vec<CertificateAuthorityTrustChain>,
    tsa_chains: Vec<CertificateAuthorityTrustChain>,
}

struct CachedSigstoreTrustMaterial {
    material: Arc<SigstoreTrustMaterial>,
    digest: String,
    source: String,
    refreshed_at: Option<Instant>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SigstoreTrustRootProvenance {
    schema_version: u32,
    sha256: String,
    source: String,
    target: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TrustedRootDocument {
    media_type: String,
    certificate_authorities: Vec<TrustedCertificateAuthority>,
    timestamp_authorities: Vec<TrustedCertificateAuthority>,
    tlogs: Vec<TrustedLogInstance>,
    ctlogs: Vec<TrustedLogInstance>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TrustedCertificateAuthority {
    cert_chain: TrustedCertificateChain,
    valid_for: TrustedTimeRange,
}

#[derive(Debug, Deserialize)]
struct TrustedCertificateChain {
    certificates: Vec<TrustedCertificate>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TrustedCertificate {
    raw_bytes: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TrustedLogInstance {
    log_id: TrustedLogId,
    public_key: TrustedPublicKey,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TrustedLogId {
    key_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TrustedPublicKey {
    raw_bytes: String,
    key_details: String,
    valid_for: TrustedTimeRange,
}

#[derive(Debug, Deserialize)]
struct TrustedTimeRange {
    start: String,
    #[serde(default)]
    end: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct SignedArtifactBundle {
    pub(super) base64_signature: String,
    pub(super) cert: String,
    pub(super) rekor_bundle: RekorBundle,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "PascalCase")]
pub(super) struct RekorBundle {
    signed_entry_timestamp: String,
    pub(super) payload: RekorPayload,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct RekorPayload {
    pub(super) body: String,
    pub(super) integrated_time: i64,
    pub(super) log_index: i64,
    #[serde(rename = "logID")]
    pub(super) log_id: String,
}

static SIGSTORE_TRUST_MATERIAL: OnceLock<RwLock<CachedSigstoreTrustMaterial>> = OnceLock::new();
static SIGSTORE_TRUST_REFRESH: OnceLock<AsyncMutex<()>> = OnceLock::new();
static VERIFY_LIMIT: OnceLock<Semaphore> = OnceLock::new();

pub async fn verify_signed_blob(
    raw: Vec<u8>,
    bundle_raw: Vec<u8>,
    required_signer: RequiredSigner,
) -> AppResult<()> {
    let trust_material = current_sigstore_trust_material()?;
    let permit = VERIFY_LIMIT
        .get_or_init(|| Semaphore::new(2))
        .acquire()
        .await
        .map_err(|_| AppError::Repository("plugin verification worker is closed".to_string()))?;
    let result = tokio::task::spawn_blocking(move || {
        verify_signed_blob_blocking(&raw, &bundle_raw, &required_signer, &trust_material)
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
    trust_material: &SigstoreTrustMaterial,
) -> AppResult<()> {
    let bundle_text = std::str::from_utf8(bundle_raw)
        .map_err(|error| AppError::Validation(format!("invalid Sigstore bundle UTF-8: {error}")))?;
    let bundle_json: serde_json::Value = serde_json::from_str(bundle_text).map_err(|error| {
        AppError::Validation(format!("failed to parse Sigstore bundle: {error}"))
    })?;
    let is_legacy = bundle_json.get("base64Signature").is_some();
    let is_v03 = bundle_json.get("messageSignature").is_some();
    if is_legacy == is_v03 {
        return Err(AppError::Validation(
            "unsupported Sigstore bundle content; expected a Cosign v2 blob signature or a Sigstore v0.3 message signature"
                .to_string(),
        ));
    }

    if is_legacy {
        verify_legacy_signed_blob(raw, bundle_text, required_signer, trust_material)
    } else {
        verify_v03_signed_blob(raw, bundle_text, required_signer, trust_material)
    }
}

fn verify_legacy_signed_blob(
    raw: &[u8],
    bundle_text: &str,
    required_signer: &RequiredSigner,
    trust_material: &SigstoreTrustMaterial,
) -> AppResult<()> {
    let bundle = parse_and_verify_bundle(bundle_text, &trust_material.rekor_keys)?;
    let cert_pem = normalize_bundle_cert(&bundle.cert)?;

    verify_rekor_hashedrekord_binding(
        raw,
        &bundle.base64_signature,
        &cert_pem,
        &bundle.rekor_bundle.payload.body,
    )?;
    verify_blob_signature(&cert_pem, &bundle.base64_signature, raw)?;
    verify_fulcio_certificate_chain(
        &cert_pem,
        bundle.rekor_bundle.payload.integrated_time,
        trust_material,
    )?;
    verify_signer_identity(&cert_pem, required_signer)?;
    Ok(())
}

fn verify_v03_signed_blob(
    raw: &[u8],
    bundle_text: &str,
    required_signer: &RequiredSigner,
    trust_material: &SigstoreTrustMaterial,
) -> AppResult<()> {
    let bundle: ProtoBundle = serde_json::from_str(bundle_text).map_err(|error| {
        AppError::Validation(format!("failed to parse Sigstore v0.3 bundle: {error}"))
    })?;
    if !matches!(
        bundle.media_type.as_str(),
        SIGSTORE_BUNDLE_V03_MEDIA_TYPE | SIGSTORE_BUNDLE_V03_LEGACY_MEDIA_TYPE
    ) {
        return Err(AppError::Validation(format!(
            "unsupported Sigstore message-signature bundle media type: {}",
            bundle.media_type
        )));
    }

    let verification_material = bundle.verification_material.ok_or_else(|| {
        AppError::Validation("Sigstore v0.3 bundle is missing verification material".to_string())
    })?;
    let rfc3161_timestamps = verification_material
        .timestamp_verification_data
        .as_ref()
        .map(|timestamps| timestamps.rfc3161_timestamps.clone())
        .unwrap_or_default();
    if rfc3161_timestamps.len() > 4 {
        return Err(AppError::Validation(
            "Sigstore v0.3 bundle contains too many RFC3161 timestamps".to_string(),
        ));
    }
    let [tlog_entry] = verification_material.tlog_entries.as_slice() else {
        return Err(AppError::Validation(
            "Sigstore v0.3 bundle must contain exactly one transparency-log entry".to_string(),
        ));
    };
    let certificate = match verification_material.content {
        Some(ProtoVerificationMaterial::Certificate(certificate)) => certificate,
        _ => {
            return Err(AppError::Validation(
                "Sigstore v0.3 keyless bundle must contain exactly one leaf certificate"
                    .to_string(),
            ));
        }
    };
    if certificate.raw_bytes.is_empty() {
        return Err(AppError::Validation(
            "Sigstore v0.3 bundle contains an empty leaf certificate".to_string(),
        ));
    }

    let message_signature = match bundle.content {
        Some(ProtoBundleContent::MessageSignature(signature)) => signature,
        _ => {
            return Err(AppError::Validation(
                "Sigstore v0.3 plugin bundle must contain a message signature".to_string(),
            ));
        }
    };
    if message_signature.signature.is_empty() {
        return Err(AppError::Validation(
            "Sigstore v0.3 bundle contains an empty message signature".to_string(),
        ));
    }
    let message_digest = message_signature.message_digest.as_ref().ok_or_else(|| {
        AppError::Validation("Sigstore v0.3 bundle is missing its artifact digest".to_string())
    })?;
    if message_digest.algorithm != ProtoHashAlgorithm::Sha2256 as i32 {
        return Err(AppError::Validation(
            "Sigstore v0.3 plugin bundle must identify the artifact with SHA-256".to_string(),
        ));
    }
    if message_digest.digest.as_slice() != Sha256::digest(raw).as_slice() {
        return Err(AppError::Validation(
            "Sigstore v0.3 bundle digest does not match the plugin artifact".to_string(),
        ));
    }

    let integrated_time = verify_v03_tlog_entry(tlog_entry, &trust_material.rekor_keys)?;
    let timestamp_times = rfc3161_timestamps
        .iter()
        .map(|timestamp| {
            verify_rfc3161_timestamp(
                &timestamp.signed_timestamp,
                &message_signature.signature,
                trust_material,
            )
        })
        .collect::<AppResult<Vec<_>>>()?;
    let cert_pem = pem_encode_certificate(&certificate.raw_bytes);
    let base64_signature =
        base64::engine::general_purpose::STANDARD.encode(&message_signature.signature);
    let base64_rekor_body =
        base64::engine::general_purpose::STANDARD.encode(&tlog_entry.canonicalized_body);

    verify_rekor_hashedrekord_binding(raw, &base64_signature, &cert_pem, &base64_rekor_body)?;
    verify_blob_signature(&cert_pem, &base64_signature, raw)?;
    verify_fulcio_certificate_chain(&cert_pem, integrated_time, trust_material)?;
    for timestamp_time in timestamp_times {
        verify_fulcio_certificate_chain(&cert_pem, timestamp_time, trust_material)?;
    }
    verify_signer_identity(&cert_pem, required_signer)?;
    Ok(())
}

fn verify_v03_tlog_entry(
    entry: &TransparencyLogEntry,
    rekor_keys: &RekorVerificationKeys,
) -> AppResult<i64> {
    if entry.log_index < 0 || entry.integrated_time < 0 || entry.canonicalized_body.is_empty() {
        return Err(AppError::Validation(
            "Sigstore v0.3 transparency-log entry has invalid required fields".to_string(),
        ));
    }
    let kind_version = entry.kind_version.as_ref().ok_or_else(|| {
        AppError::Validation("Sigstore v0.3 bundle is missing the Rekor kind/version".to_string())
    })?;
    if kind_version.kind != "hashedrekord" || kind_version.version != "0.0.1" {
        return Err(AppError::Validation(
            "unsupported Sigstore v0.3 Rekor entry; expected hashedrekord v0.0.1".to_string(),
        ));
    }
    let log_id = entry.log_id.as_ref().ok_or_else(|| {
        AppError::Validation("Sigstore v0.3 bundle is missing the Rekor log ID".to_string())
    })?;
    if log_id.key_id.len() != 32 {
        return Err(AppError::Validation(
            "Sigstore v0.3 bundle contains an invalid Rekor log ID".to_string(),
        ));
    }
    let log_id_hex = lower_hex(&log_id.key_id);
    let rekor_key = rekor_keys.get(&log_id_hex).ok_or_else(|| {
        AppError::Validation(format!(
            "Sigstore Rekor public key '{log_id_hex}' is not trusted"
        ))
    })?;
    if !rekor_key.valid_for.contains(entry.integrated_time) {
        return Err(AppError::Validation(format!(
            "Sigstore Rekor public key '{log_id_hex}' was not valid at the integrated time"
        )));
    }

    let inclusion_promise = entry.inclusion_promise.as_ref().ok_or_else(|| {
        AppError::Validation(
            "Sigstore v0.3 bundle requires a Rekor inclusion promise for trusted signing time"
                .to_string(),
        )
    })?;
    if inclusion_promise.signed_entry_timestamp.is_empty() {
        return Err(AppError::Validation(
            "Sigstore v0.3 bundle contains an empty Rekor inclusion promise".to_string(),
        ));
    }
    let payload = RekorPayload {
        body: base64::engine::general_purpose::STANDARD.encode(&entry.canonicalized_body),
        integrated_time: entry.integrated_time,
        log_index: entry.log_index,
        log_id: log_id_hex.clone(),
    };
    let canonical_payload = serde_json_canonicalizer::to_vec(&payload).map_err(|error| {
        AppError::Validation(format!(
            "failed to canonicalize Sigstore v0.3 Rekor payload: {error}"
        ))
    })?;
    rekor_key
        .key
        .verify_signature(
            Signature::Raw(&inclusion_promise.signed_entry_timestamp),
            &canonical_payload,
        )
        .map_err(|error| {
            AppError::Validation(format!(
                "Sigstore v0.3 Rekor inclusion-promise verification failed: {error}"
            ))
        })?;

    let proof = entry.inclusion_proof.as_ref().ok_or_else(|| {
        AppError::Validation(
            "Sigstore v0.3 bundle is missing its Rekor inclusion proof".to_string(),
        )
    })?;
    verify_v03_inclusion_proof(
        proof,
        &entry.canonicalized_body,
        &rekor_key.key,
        &log_id.key_id,
    )?;
    Ok(entry.integrated_time)
}

fn verify_v03_inclusion_proof(
    proof: &ProtoInclusionProof,
    canonicalized_body: &[u8],
    rekor_key: &CosignVerificationKey,
    log_key_id: &[u8],
) -> AppResult<()> {
    let leaf_index = u64::try_from(proof.log_index).map_err(|_| {
        AppError::Validation("Sigstore Rekor inclusion proof has a negative index".to_string())
    })?;
    let tree_size = u64::try_from(proof.tree_size).map_err(|_| {
        AppError::Validation("Sigstore Rekor inclusion proof has an invalid tree size".to_string())
    })?;
    if tree_size == 0 || leaf_index >= tree_size {
        return Err(AppError::Validation(
            "Sigstore Rekor inclusion proof index is outside its tree".to_string(),
        ));
    }
    let root_hash = slice_to_sha256(&proof.root_hash, "Rekor inclusion-proof root")?;
    let proof_hashes = proof
        .hashes
        .iter()
        .map(|hash| slice_to_sha256(hash, "Rekor inclusion-proof hash"))
        .collect::<AppResult<Vec<_>>>()?;
    let checkpoint = proof.checkpoint.as_ref().ok_or_else(|| {
        AppError::Validation(
            "Sigstore v0.3 Rekor inclusion proof is missing its signed checkpoint".to_string(),
        )
    })?;
    verify_rekor_checkpoint(
        &checkpoint.envelope,
        tree_size,
        &root_hash,
        rekor_key,
        log_key_id,
    )?;

    let leaf_hash = sha256_prefixed(0, canonicalized_body);
    let computed_root =
        merkle_root_from_inclusion_proof(leaf_index, tree_size, leaf_hash, &proof_hashes)?;
    if computed_root != root_hash {
        return Err(AppError::Validation(
            "Sigstore Rekor inclusion proof does not match its signed tree root".to_string(),
        ));
    }
    Ok(())
}

fn verify_rekor_checkpoint(
    envelope: &str,
    proof_tree_size: u64,
    proof_root_hash: &[u8; 32],
    rekor_key: &CosignVerificationKey,
    log_key_id: &[u8],
) -> AppResult<()> {
    if envelope.contains('\r') {
        return Err(AppError::Validation(
            "Sigstore Rekor checkpoint contains unsupported line endings".to_string(),
        ));
    }
    let (note, signatures) = envelope.split_once("\n\n").ok_or_else(|| {
        AppError::Validation("Sigstore Rekor checkpoint has an invalid envelope".to_string())
    })?;
    let mut note_lines = note.split('\n');
    let origin = note_lines.next().unwrap_or_default();
    let checkpoint_size = note_lines
        .next()
        .ok_or_else(|| {
            AppError::Validation("Sigstore Rekor checkpoint is missing its tree size".to_string())
        })?
        .parse::<u64>()
        .map_err(|_| {
            AppError::Validation("Sigstore Rekor checkpoint has an invalid tree size".to_string())
        })?;
    let checkpoint_root = note_lines.next().ok_or_else(|| {
        AppError::Validation("Sigstore Rekor checkpoint is missing its root hash".to_string())
    })?;
    if origin.is_empty() || checkpoint_size != proof_tree_size {
        return Err(AppError::Validation(
            "Sigstore Rekor checkpoint does not match the inclusion-proof tree size".to_string(),
        ));
    }
    let checkpoint_root = base64::engine::general_purpose::STANDARD
        .decode(checkpoint_root.as_bytes())
        .map_err(|error| {
            AppError::Validation(format!(
                "invalid Sigstore Rekor checkpoint root encoding: {error}"
            ))
        })?;
    if checkpoint_root.as_slice() != proof_root_hash {
        return Err(AppError::Validation(
            "Sigstore Rekor checkpoint does not match the inclusion-proof root".to_string(),
        ));
    }

    let signed_note = format!("{note}\n");
    let expected_hint = log_key_id.get(..4).ok_or_else(|| {
        AppError::Validation("Sigstore Rekor log ID is too short for a checkpoint".to_string())
    })?;
    let mut saw_signature = false;
    for line in signatures.lines().filter(|line| !line.is_empty()) {
        saw_signature = true;
        let line = line.strip_prefix("— ").ok_or_else(|| {
            AppError::Validation("Sigstore Rekor checkpoint signature is malformed".to_string())
        })?;
        let (name, encoded_signature) = line.split_once(' ').ok_or_else(|| {
            AppError::Validation("Sigstore Rekor checkpoint signature is malformed".to_string())
        })?;
        if name.is_empty() || encoded_signature.is_empty() || encoded_signature.contains(' ') {
            return Err(AppError::Validation(
                "Sigstore Rekor checkpoint signature is malformed".to_string(),
            ));
        }
        let signature = base64::engine::general_purpose::STANDARD
            .decode(encoded_signature.as_bytes())
            .map_err(|error| {
                AppError::Validation(format!(
                    "invalid Sigstore Rekor checkpoint signature encoding: {error}"
                ))
            })?;
        let Some((key_hint, raw_signature)) = signature.split_at_checked(4) else {
            return Err(AppError::Validation(
                "Sigstore Rekor checkpoint signature is too short".to_string(),
            ));
        };
        if key_hint == expected_hint
            && rekor_key
                .verify_signature(Signature::Raw(raw_signature), signed_note.as_bytes())
                .is_ok()
        {
            return Ok(());
        }
    }
    if !saw_signature {
        return Err(AppError::Validation(
            "Sigstore Rekor checkpoint is missing its signature".to_string(),
        ));
    }
    Err(AppError::Validation(
        "Sigstore Rekor checkpoint signature verification failed".to_string(),
    ))
}

fn merkle_root_from_inclusion_proof(
    leaf_index: u64,
    tree_size: u64,
    leaf_hash: [u8; 32],
    proof_hashes: &[[u8; 32]],
) -> AppResult<[u8; 32]> {
    let inner = u64::BITS as u64 - (leaf_index ^ (tree_size - 1)).leading_zeros() as u64;
    let border = (leaf_index >> inner).count_ones() as u64;
    let expected_len = inner + border;
    if proof_hashes.len() as u64 != expected_len {
        return Err(AppError::Validation(format!(
            "Sigstore Rekor inclusion proof has {} hashes; expected {expected_len}",
            proof_hashes.len()
        )));
    }

    let mut root = leaf_hash;
    for (level, hash) in proof_hashes.iter().take(inner as usize).enumerate() {
        root = if ((leaf_index >> level) & 1) == 0 {
            sha256_node(&root, hash)
        } else {
            sha256_node(hash, &root)
        };
    }
    for hash in proof_hashes.iter().skip(inner as usize) {
        root = sha256_node(hash, &root);
    }
    Ok(root)
}

fn sha256_prefixed(prefix: u8, value: &[u8]) -> [u8; 32] {
    Sha256::new()
        .chain_update([prefix])
        .chain_update(value)
        .finalize()
        .into()
}

fn sha256_node(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    Sha256::new()
        .chain_update([1])
        .chain_update(left)
        .chain_update(right)
        .finalize()
        .into()
}

fn slice_to_sha256(value: &[u8], label: &str) -> AppResult<[u8; 32]> {
    value.try_into().map_err(|_| {
        AppError::Validation(format!("Sigstore {label} must contain exactly 32 bytes"))
    })
}

#[derive(Clone, Copy)]
struct DerElement<'a> {
    tag: u8,
    encoded: &'a [u8],
    content: &'a [u8],
}

#[derive(Clone, Copy)]
enum CmsSignatureAlgorithm {
    EcdsaSha256,
    RsaPkcs1Sha256,
}

struct ParsedRfc3161Timestamp<'a> {
    tst_info: &'a [u8],
    message_imprint: [u8; 32],
    generated_at: i64,
    signer_issuer: &'a [u8],
    signer_serial: Vec<u8>,
    signed_attributes: Vec<u8>,
    signed_message_digest: [u8; 32],
    signing_certificate_digest: [u8; 32],
    signature_algorithm: CmsSignatureAlgorithm,
    signature: &'a [u8],
}

fn verify_rfc3161_timestamp(
    timestamp: &[u8],
    message_signature: &[u8],
    trust_material: &SigstoreTrustMaterial,
) -> AppResult<i64> {
    let parsed = parse_rfc3161_timestamp(timestamp)?;
    if parsed.message_imprint != Sha256::digest(message_signature).as_slice() {
        return Err(AppError::Validation(
            "Sigstore RFC3161 timestamp does not bind the plugin signature".to_string(),
        ));
    }
    if parsed.signed_message_digest != Sha256::digest(parsed.tst_info).as_slice() {
        return Err(AppError::Validation(
            "Sigstore RFC3161 CMS message digest does not bind the timestamp info".to_string(),
        ));
    }

    for chain in &trust_material.tsa_chains {
        if !chain.valid_for.contains(parsed.generated_at)
            || !chain
                .certificate_validity
                .iter()
                .all(|validity| validity.contains(parsed.generated_at))
        {
            continue;
        }
        let Some(signer_der) = chain.intermediates.first() else {
            continue;
        };
        if Sha256::digest(signer_der.as_ref()).as_slice() != parsed.signing_certificate_digest {
            continue;
        }
        let signer = Certificate::from_der(signer_der.as_ref()).map_err(|error| {
            AppError::Validation(format!(
                "failed to parse Sigstore timestamp signer certificate: {error}"
            ))
        })?;
        let signer_issuer = signer.tbs_certificate.issuer.to_der().map_err(|error| {
            AppError::Validation(format!(
                "failed to encode Sigstore timestamp signer issuer: {error}"
            ))
        })?;
        if signer_issuer != parsed.signer_issuer
            || signer.tbs_certificate.serial_number.as_bytes() != parsed.signer_serial
        {
            continue;
        }

        let end_entity = EndEntityCert::try_from(signer_der).map_err(|error| {
            AppError::Validation(format!(
                "invalid Sigstore timestamp signer certificate: {error}"
            ))
        })?;
        let verification_time = rekor_integrated_time(parsed.generated_at)?;
        if end_entity
            .verify_for_usage(
                webpki::ALL_VERIFICATION_ALGS,
                std::slice::from_ref(&chain.anchor),
                &chain.intermediates[1..],
                verification_time,
                KeyUsage::required(ID_KP_TIME_STAMPING.as_bytes()),
                None,
                None,
            )
            .is_err()
        {
            continue;
        }
        let signer_spki = signer
            .tbs_certificate
            .subject_public_key_info
            .to_der()
            .map_err(|error| {
                AppError::Validation(format!(
                    "failed to encode Sigstore timestamp signer public key: {error}"
                ))
            })?;
        if verify_cms_signature(
            parsed.signature_algorithm,
            &signer_spki,
            &parsed.signed_attributes,
            parsed.signature,
        ) {
            return Ok(parsed.generated_at);
        }
    }

    Err(AppError::Validation(
        "Sigstore RFC3161 timestamp signature or timestamp-authority chain is invalid".to_string(),
    ))
}

fn verify_cms_signature(
    algorithm: CmsSignatureAlgorithm,
    public_key: &[u8],
    signed_attributes: &[u8],
    signature: &[u8],
) -> bool {
    match algorithm {
        CmsSignatureAlgorithm::EcdsaSha256 => {
            UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, public_key)
                .verify(signed_attributes, signature)
                .is_ok()
                || UnparsedPublicKey::new(&ECDSA_P384_SHA256_ASN1, public_key)
                    .verify(signed_attributes, signature)
                    .is_ok()
                || UnparsedPublicKey::new(&ECDSA_P521_SHA256_ASN1, public_key)
                    .verify(signed_attributes, signature)
                    .is_ok()
        }
        CmsSignatureAlgorithm::RsaPkcs1Sha256 => {
            UnparsedPublicKey::new(&RSA_PKCS1_2048_8192_SHA256, public_key)
                .verify(signed_attributes, signature)
                .is_ok()
        }
    }
}

fn parse_rfc3161_timestamp(raw: &[u8]) -> AppResult<ParsedRfc3161Timestamp<'_>> {
    let response = der_exact(raw, 0x30, "RFC3161 timestamp response")?;
    let mut response_fields = response.content;
    let status = der_take_expected(&mut response_fields, 0x30, "RFC3161 status")?;
    let mut status_fields = status.content;
    let status_code = der_positive_integer(
        der_take_expected(&mut status_fields, 0x02, "RFC3161 status code")?,
        "RFC3161 status code",
    )?;
    if status_code != [0] && status_code != [1] {
        return Err(AppError::Validation(
            "Sigstore RFC3161 timestamp authority did not grant the timestamp".to_string(),
        ));
    }
    while !status_fields.is_empty() {
        der_take(&mut status_fields, "RFC3161 status field")?;
    }
    let token = der_take_expected(
        &mut response_fields,
        0x30,
        "RFC3161 timestamp CMS content info",
    )?;
    der_require_empty(response_fields, "RFC3161 timestamp response")?;

    let mut content_info = token.content;
    der_expect_oid(
        &mut content_info,
        OID_CMS_SIGNED_DATA,
        "CMS signed-data content type",
    )?;
    let signed_data_wrapper =
        der_take_expected(&mut content_info, 0xa0, "CMS signed-data wrapper")?;
    der_require_empty(content_info, "CMS content info")?;
    let signed_data = der_exact(
        signed_data_wrapper.content,
        0x30,
        "CMS signed-data sequence",
    )?;
    parse_rfc3161_signed_data(signed_data.content)
}

fn parse_rfc3161_signed_data(mut fields: &[u8]) -> AppResult<ParsedRfc3161Timestamp<'_>> {
    let signed_data_version = der_positive_integer(
        der_take_expected(&mut fields, 0x02, "CMS signed-data version")?,
        "CMS signed-data version",
    )?;
    if signed_data_version != [3] {
        return Err(AppError::Validation(
            "unsupported Sigstore RFC3161 CMS signed-data version".to_string(),
        ));
    }
    let digest_algorithms = der_take_expected(&mut fields, 0x31, "CMS digest algorithms")?;
    let digest_algorithm = der_exact(
        digest_algorithms.content,
        0x30,
        "CMS SHA-256 digest algorithm",
    )?;
    parse_sha256_algorithm_identifier(digest_algorithm)?;

    let encapsulated = der_take_expected(&mut fields, 0x30, "CMS encapsulated content")?;
    let mut encapsulated_fields = encapsulated.content;
    der_expect_oid(
        &mut encapsulated_fields,
        OID_TST_INFO,
        "RFC3161 TSTInfo content type",
    )?;
    let content_wrapper = der_take_expected(
        &mut encapsulated_fields,
        0xa0,
        "CMS timestamp content wrapper",
    )?;
    der_require_empty(encapsulated_fields, "CMS encapsulated content")?;
    let tst_info = der_exact(content_wrapper.content, 0x04, "RFC3161 TSTInfo octets")?.content;

    if fields.first() == Some(&0xa0) {
        der_take(&mut fields, "CMS embedded certificates")?;
    }
    if fields.first() == Some(&0xa1) {
        return Err(AppError::Validation(
            "Sigstore RFC3161 timestamps with CMS revocation data are unsupported".to_string(),
        ));
    }
    let signer_infos = der_take_expected(&mut fields, 0x31, "CMS signer infos")?;
    der_require_empty(fields, "CMS signed data")?;
    let signer_info = der_exact(signer_infos.content, 0x30, "CMS signer info")?;

    let (message_imprint, generated_at) = parse_tst_info(tst_info)?;
    let mut signer_fields = signer_info.content;
    let signer_info_version = der_positive_integer(
        der_take_expected(&mut signer_fields, 0x02, "CMS signer-info version")?,
        "CMS signer-info version",
    )?;
    if signer_info_version != [1] {
        return Err(AppError::Validation(
            "unsupported Sigstore RFC3161 CMS signer-info version".to_string(),
        ));
    }
    let signer_identifier = der_take_expected(&mut signer_fields, 0x30, "CMS signer identifier")?;
    let mut signer_identifier_fields = signer_identifier.content;
    let signer_issuer =
        der_take_expected(&mut signer_identifier_fields, 0x30, "CMS signer issuer")?;
    let signer_serial = der_positive_integer(
        der_take_expected(
            &mut signer_identifier_fields,
            0x02,
            "CMS signer serial number",
        )?,
        "CMS signer serial number",
    )?
    .to_vec();
    der_require_empty(signer_identifier_fields, "CMS signer identifier")?;
    parse_sha256_algorithm_identifier(der_take_expected(
        &mut signer_fields,
        0x30,
        "CMS signer digest algorithm",
    )?)?;
    let signed_attributes = der_take_expected(&mut signer_fields, 0xa0, "CMS signed attributes")?;
    let (signed_message_digest, signing_certificate_digest) =
        parse_signed_attributes(signed_attributes.content)?;
    let signed_attributes = der_wrap(0x31, signed_attributes.content);
    let signature_algorithm = parse_cms_signature_algorithm(der_take_expected(
        &mut signer_fields,
        0x30,
        "CMS signature algorithm",
    )?)?;
    let signature = der_take_expected(&mut signer_fields, 0x04, "CMS signature")?.content;
    if signer_fields.first() == Some(&0xa1) {
        der_take(&mut signer_fields, "CMS unsigned attributes")?;
    }
    der_require_empty(signer_fields, "CMS signer info")?;

    Ok(ParsedRfc3161Timestamp {
        tst_info,
        message_imprint,
        generated_at,
        signer_issuer: signer_issuer.encoded,
        signer_serial,
        signed_attributes,
        signed_message_digest,
        signing_certificate_digest,
        signature_algorithm,
        signature,
    })
}

fn parse_tst_info(raw: &[u8]) -> AppResult<([u8; 32], i64)> {
    let tst_info = der_exact(raw, 0x30, "RFC3161 TSTInfo")?;
    let mut fields = tst_info.content;
    let version = der_positive_integer(
        der_take_expected(&mut fields, 0x02, "RFC3161 TSTInfo version")?,
        "RFC3161 TSTInfo version",
    )?;
    if version != [1] {
        return Err(AppError::Validation(
            "unsupported Sigstore RFC3161 TSTInfo version".to_string(),
        ));
    }
    der_take_expected(&mut fields, 0x06, "RFC3161 timestamp policy")?;
    let message_imprint = der_take_expected(&mut fields, 0x30, "RFC3161 message imprint")?;
    let mut imprint_fields = message_imprint.content;
    parse_sha256_algorithm_identifier(der_take_expected(
        &mut imprint_fields,
        0x30,
        "RFC3161 message-imprint algorithm",
    )?)?;
    let imprint = slice_to_sha256(
        der_take_expected(&mut imprint_fields, 0x04, "RFC3161 message-imprint digest")?.content,
        "RFC3161 message-imprint digest",
    )?;
    der_require_empty(imprint_fields, "RFC3161 message imprint")?;
    der_take_expected(&mut fields, 0x02, "RFC3161 timestamp serial number")?;
    let generated_at = der_take_expected(&mut fields, 0x18, "RFC3161 generation time")?;
    let generated_at = std::str::from_utf8(generated_at.content).map_err(|_| {
        AppError::Validation("Sigstore RFC3161 generation time is not ASCII".to_string())
    })?;
    let generated_at = chrono::NaiveDateTime::parse_from_str(generated_at, "%Y%m%d%H%M%S%.fZ")
        .map_err(|error| {
            AppError::Validation(format!("invalid Sigstore RFC3161 generation time: {error}"))
        })?
        .and_utc()
        .timestamp();
    while !fields.is_empty() {
        der_take(&mut fields, "RFC3161 TSTInfo optional field")?;
    }
    Ok((imprint, generated_at))
}

fn parse_signed_attributes(raw: &[u8]) -> AppResult<([u8; 32], [u8; 32])> {
    let mut fields = raw;
    let mut previous_encoding: Option<&[u8]> = None;
    let mut content_type = false;
    let mut message_digest = None;
    let mut signing_certificate_digest = None;
    while !fields.is_empty() {
        let attribute = der_take_expected(&mut fields, 0x30, "CMS signed attribute")?;
        if previous_encoding.is_some_and(|previous| previous > attribute.encoded) {
            return Err(AppError::Validation(
                "Sigstore RFC3161 CMS signed attributes are not DER-sorted".to_string(),
            ));
        }
        previous_encoding = Some(attribute.encoded);
        let mut attribute_fields = attribute.content;
        let oid = der_take_expected(&mut attribute_fields, 0x06, "CMS attribute OID")?;
        let values = der_take_expected(&mut attribute_fields, 0x31, "CMS attribute values")?;
        der_require_empty(attribute_fields, "CMS signed attribute")?;
        if oid.content == OID_CONTENT_TYPE {
            if content_type {
                return Err(AppError::Validation(
                    "Sigstore RFC3161 CMS content-type attribute is duplicated".to_string(),
                ));
            }
            let value = der_exact(values.content, 0x06, "CMS content-type attribute")?;
            if value.content != OID_TST_INFO {
                return Err(AppError::Validation(
                    "Sigstore RFC3161 CMS content type is not TSTInfo".to_string(),
                ));
            }
            content_type = true;
        } else if oid.content == OID_MESSAGE_DIGEST {
            if message_digest.is_some() {
                return Err(AppError::Validation(
                    "Sigstore RFC3161 CMS message-digest attribute is duplicated".to_string(),
                ));
            }
            message_digest = Some(slice_to_sha256(
                der_exact(values.content, 0x04, "CMS message-digest attribute")?.content,
                "RFC3161 CMS message-digest attribute",
            )?);
        } else if oid.content == OID_SIGNING_CERTIFICATE_V2 {
            if signing_certificate_digest.is_some() {
                return Err(AppError::Validation(
                    "Sigstore RFC3161 signing-certificate attribute is duplicated".to_string(),
                ));
            }
            signing_certificate_digest = Some(parse_signing_certificate_v2(values.content)?);
        }
    }
    if !content_type {
        return Err(AppError::Validation(
            "Sigstore RFC3161 CMS signed attributes are missing content type".to_string(),
        ));
    }
    Ok((
        message_digest.ok_or_else(|| {
            AppError::Validation(
                "Sigstore RFC3161 CMS signed attributes are missing message digest".to_string(),
            )
        })?,
        signing_certificate_digest.ok_or_else(|| {
            AppError::Validation(
                "Sigstore RFC3161 CMS signed attributes are missing signing certificate"
                    .to_string(),
            )
        })?,
    ))
}

fn parse_signing_certificate_v2(raw: &[u8]) -> AppResult<[u8; 32]> {
    let signing_certificate = der_exact(raw, 0x30, "CMS SigningCertificateV2")?;
    let mut signing_certificate_fields = signing_certificate.content;
    let certificates = der_take_expected(
        &mut signing_certificate_fields,
        0x30,
        "CMS SigningCertificateV2 certificates",
    )?;
    let mut certificates_fields = certificates.content;
    let certificate = der_take_expected(
        &mut certificates_fields,
        0x30,
        "CMS ESSCertIDv2 certificate",
    )?;
    der_require_empty(certificates_fields, "CMS SigningCertificateV2 certificates")?;
    while !signing_certificate_fields.is_empty() {
        der_take(
            &mut signing_certificate_fields,
            "CMS SigningCertificateV2 policy",
        )?;
    }

    let mut certificate_fields = certificate.content;
    if certificate_fields.first() == Some(&0x30) {
        parse_sha256_algorithm_identifier(der_take_expected(
            &mut certificate_fields,
            0x30,
            "CMS ESSCertIDv2 hash algorithm",
        )?)?;
    }
    let digest = slice_to_sha256(
        der_take_expected(
            &mut certificate_fields,
            0x04,
            "CMS ESSCertIDv2 certificate digest",
        )?
        .content,
        "RFC3161 signing-certificate digest",
    )?;
    while !certificate_fields.is_empty() {
        der_take(&mut certificate_fields, "CMS ESSCertIDv2 issuer/serial")?;
    }
    Ok(digest)
}

fn parse_sha256_algorithm_identifier(algorithm: DerElement<'_>) -> AppResult<()> {
    let mut fields = algorithm.content;
    der_expect_oid(&mut fields, OID_SHA256, "SHA-256 algorithm")?;
    if !fields.is_empty() {
        let parameters = der_take_expected(&mut fields, 0x05, "SHA-256 algorithm parameters")?;
        if !parameters.content.is_empty() {
            return Err(AppError::Validation(
                "Sigstore SHA-256 algorithm has invalid parameters".to_string(),
            ));
        }
    }
    der_require_empty(fields, "SHA-256 algorithm identifier")
}

fn parse_cms_signature_algorithm(algorithm: DerElement<'_>) -> AppResult<CmsSignatureAlgorithm> {
    let mut fields = algorithm.content;
    let oid = der_take_expected(&mut fields, 0x06, "CMS signature algorithm OID")?;
    let parsed = if oid.content == OID_ECDSA_WITH_SHA256 {
        CmsSignatureAlgorithm::EcdsaSha256
    } else if oid.content == OID_RSA_WITH_SHA256 {
        CmsSignatureAlgorithm::RsaPkcs1Sha256
    } else {
        return Err(AppError::Validation(
            "unsupported Sigstore RFC3161 CMS signature algorithm".to_string(),
        ));
    };
    if !fields.is_empty() {
        let parameters = der_take_expected(&mut fields, 0x05, "CMS signature parameters")?;
        if !parameters.content.is_empty() {
            return Err(AppError::Validation(
                "Sigstore RFC3161 CMS signature algorithm has invalid parameters".to_string(),
            ));
        }
    }
    der_require_empty(fields, "CMS signature algorithm")?;
    Ok(parsed)
}

fn der_expect_oid(input: &mut &[u8], expected: &[u8], label: &str) -> AppResult<()> {
    let oid = der_take_expected(input, 0x06, label)?;
    if oid.content != expected {
        return Err(AppError::Validation(format!(
            "Sigstore {label} is unsupported"
        )));
    }
    Ok(())
}

fn der_positive_integer<'a>(element: DerElement<'a>, label: &str) -> AppResult<&'a [u8]> {
    let value = element.content;
    if value.is_empty() || value[0] & 0x80 != 0 {
        return Err(AppError::Validation(format!(
            "Sigstore {label} is not a positive DER integer"
        )));
    }
    if value.len() > 1 && value[0] == 0 {
        if value[1] & 0x80 == 0 {
            return Err(AppError::Validation(format!(
                "Sigstore {label} is not minimally encoded"
            )));
        }
        Ok(&value[1..])
    } else {
        Ok(value)
    }
}

fn der_exact<'a>(raw: &'a [u8], tag: u8, label: &str) -> AppResult<DerElement<'a>> {
    let mut input = raw;
    let element = der_take_expected(&mut input, tag, label)?;
    der_require_empty(input, label)?;
    Ok(element)
}

fn der_take_expected<'a>(input: &mut &'a [u8], tag: u8, label: &str) -> AppResult<DerElement<'a>> {
    let element = der_take(input, label)?;
    if element.tag != tag {
        return Err(AppError::Validation(format!(
            "Sigstore {label} has an unexpected DER tag"
        )));
    }
    Ok(element)
}

fn der_take<'a>(input: &mut &'a [u8], label: &str) -> AppResult<DerElement<'a>> {
    if input.len() < 2 || input[0] & 0x1f == 0x1f {
        return Err(AppError::Validation(format!(
            "Sigstore {label} has an invalid DER header"
        )));
    }
    let tag = input[0];
    let first_length = input[1];
    let (header_len, content_len) = if first_length & 0x80 == 0 {
        (2, usize::from(first_length))
    } else {
        let length_bytes = usize::from(first_length & 0x7f);
        if length_bytes == 0 || length_bytes > std::mem::size_of::<usize>() {
            return Err(AppError::Validation(format!(
                "Sigstore {label} uses an invalid DER length"
            )));
        }
        let length_end = 2_usize
            .checked_add(length_bytes)
            .ok_or_else(|| AppError::Validation(format!("Sigstore {label} DER length overflow")))?;
        let encoded_length = input.get(2..length_end).ok_or_else(|| {
            AppError::Validation(format!("Sigstore {label} has a truncated DER length"))
        })?;
        if encoded_length[0] == 0 {
            return Err(AppError::Validation(format!(
                "Sigstore {label} DER length is not minimally encoded"
            )));
        }
        let mut content_len = 0_usize;
        for byte in encoded_length {
            content_len = content_len
                .checked_mul(256)
                .and_then(|length| length.checked_add(usize::from(*byte)))
                .ok_or_else(|| {
                    AppError::Validation(format!("Sigstore {label} DER length overflow"))
                })?;
        }
        if content_len < 128 {
            return Err(AppError::Validation(format!(
                "Sigstore {label} DER length is not minimally encoded"
            )));
        }
        (length_end, content_len)
    };
    let encoded_len = header_len.checked_add(content_len).ok_or_else(|| {
        AppError::Validation(format!("Sigstore {label} DER element length overflow"))
    })?;
    let encoded = input
        .get(..encoded_len)
        .ok_or_else(|| AppError::Validation(format!("Sigstore {label} contains truncated DER")))?;
    let content = &encoded[header_len..];
    *input = &input[encoded_len..];
    Ok(DerElement {
        tag,
        encoded,
        content,
    })
}

fn der_wrap(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut encoded = Vec::with_capacity(content.len() + 6);
    encoded.push(tag);
    if content.len() < 128 {
        encoded.push(content.len() as u8);
    } else {
        let bytes = content.len().to_be_bytes();
        let first_nonzero = bytes
            .iter()
            .position(|byte| *byte != 0)
            .unwrap_or(bytes.len() - 1);
        let length = &bytes[first_nonzero..];
        encoded.push(0x80 | length.len() as u8);
        encoded.extend_from_slice(length);
    }
    encoded.extend_from_slice(content);
    encoded
}

fn der_require_empty(input: &[u8], label: &str) -> AppResult<()> {
    if input.is_empty() {
        Ok(())
    } else {
        Err(AppError::Validation(format!(
            "Sigstore {label} contains unexpected trailing DER data"
        )))
    }
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
    if !rekor_key
        .valid_for
        .contains(bundle.rekor_bundle.payload.integrated_time)
    {
        return Err(AppError::Validation(format!(
            "Sigstore Rekor public key '{}' was not valid at the integrated time",
            bundle.rekor_bundle.payload.log_id
        )));
    }
    rekor_key
        .key
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
        .tbs_certificate
        .subject_public_key_info
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

fn verify_fulcio_certificate_chain(
    cert_pem: &str,
    integrated_time: i64,
    trust_material: &SigstoreTrustMaterial,
) -> AppResult<()> {
    let cert = Certificate::from_pem(cert_pem.as_bytes()).map_err(|error| {
        AppError::Validation(format!("failed to parse Sigstore certificate: {error}"))
    })?;
    let cert_der = cert.to_der().map_err(|error| {
        AppError::Validation(format!("failed to encode Sigstore certificate: {error}"))
    })?;
    let cert_der = CertificateDer::from(cert_der.as_slice());
    let end_entity = EndEntityCert::try_from(&cert_der)
        .map_err(|error| AppError::Validation(format!("invalid Sigstore certificate: {error}")))?;
    let verification_time = rekor_integrated_time(integrated_time)?;
    let mut last_error = None;

    for chain in &trust_material.fulcio_chains {
        if !chain.valid_for.contains(integrated_time)
            || !chain
                .certificate_validity
                .iter()
                .all(|validity| validity.contains(integrated_time))
        {
            continue;
        }
        match end_entity.verify_for_usage(
            webpki::ALL_VERIFICATION_ALGS,
            std::slice::from_ref(&chain.anchor),
            &chain.intermediates,
            verification_time,
            KeyUsage::required(ID_KP_CODE_SIGNING.as_bytes()),
            None,
            None,
        ) {
            Ok(_) => {
                verify_embedded_sct(&cert, chain, &trust_material.ctfe_keys, integrated_time)?;
                return Ok(());
            }
            Err(error) => last_error = Some(error),
        }
    }

    Err(AppError::Validation(format!(
        "Sigstore Fulcio certificate chain verification failed at the Rekor integrated time{}",
        last_error
            .map(|error| format!(": {error}"))
            .unwrap_or_default()
    )))
}

fn rekor_integrated_time(integrated_time: i64) -> AppResult<UnixTime> {
    let integrated_time = u64::try_from(integrated_time)
        .map_err(|_| AppError::Validation("Sigstore Rekor integrated time is negative".into()))?;
    Ok(UnixTime::since_unix_epoch(Duration::from_secs(
        integrated_time,
    )))
}

fn verify_embedded_sct(
    cert: &Certificate,
    chain: &CertificateAuthorityTrustChain,
    ctfe_keys: &CtfeVerificationKeys,
    rekor_time: i64,
) -> AppResult<()> {
    let sct_list = cert
        .tbs_certificate
        .get::<SignedCertificateTimestampList>()
        .map_err(|error| {
            AppError::Validation(format!("failed to read Sigstore certificate SCT: {error}"))
        })?
        .map(|(_, sct_list)| sct_list)
        .ok_or_else(|| {
            AppError::Validation("Sigstore certificate is missing its embedded SCT".to_string())
        })?;
    let serialized_scts = sct_list.parse_timestamps().map_err(|error| {
        AppError::Validation(format!(
            "failed to parse Sigstore certificate SCT list: {error:?}"
        ))
    })?;
    let [serialized_sct] = serialized_scts.as_slice() else {
        return Err(AppError::Validation(
            "Sigstore certificate must contain exactly one embedded SCT".to_string(),
        ));
    };
    let sct = serialized_sct.parse_timestamp().map_err(|error| {
        AppError::Validation(format!(
            "failed to parse Sigstore certificate SCT: {error:?}"
        ))
    })?;
    let sct_time = i64::try_from(sct.timestamp / 1_000).map_err(|_| {
        AppError::Validation("Sigstore certificate SCT timestamp is out of range".to_string())
    })?;
    if sct_time > rekor_time {
        return Err(AppError::Validation(
            "Sigstore certificate SCT is newer than the Rekor integrated time".to_string(),
        ));
    }
    if !chain.valid_for.contains(sct_time)
        || !chain
            .certificate_validity
            .iter()
            .all(|validity| validity.contains(sct_time))
    {
        return Err(AppError::Validation(
            "Sigstore Fulcio chain was not valid at the SCT timestamp".to_string(),
        ));
    }

    let log_id = lower_hex(&sct.log_id.key_id);
    let ctfe_key = ctfe_keys.get(&log_id).ok_or_else(|| {
        AppError::Validation(format!(
            "Sigstore CT log public key '{log_id}' is not trusted"
        ))
    })?;
    if !ctfe_key.valid_for.contains(sct_time) {
        return Err(AppError::Validation(format!(
            "Sigstore CT log public key '{log_id}' was not valid at the SCT timestamp"
        )));
    }
    verify_sct_algorithm(&ctfe_key.key_details, &sct.signature.algorithm)?;

    let issuer_der = chain.intermediates.first().unwrap_or(&chain.anchor_der);
    let issuer = Certificate::from_der(issuer_der.as_ref()).map_err(|error| {
        AppError::Validation(format!("failed to parse Sigstore Fulcio issuer: {error}"))
    })?;
    let issuer_spki = issuer
        .tbs_certificate
        .subject_public_key_info
        .to_der()
        .map_err(|error| {
            AppError::Validation(format!(
                "failed to encode Sigstore Fulcio issuer public key: {error}"
            ))
        })?;
    let issuer_key_hash = Sha256::digest(&issuer_spki);

    let mut precert = cert.tbs_certificate.clone();
    precert.extensions = precert.extensions.map(|extensions| {
        extensions
            .into_iter()
            .filter(|extension| extension.extn_id.to_string() != "1.3.6.1.4.1.11129.2.4.2")
            .collect()
    });
    let precert_der = precert.to_der().map_err(|error| {
        AppError::Validation(format!(
            "failed to reconstruct Sigstore precertificate: {error}"
        ))
    })?;

    let mut signed_data = Vec::with_capacity(precert_der.len() + 64);
    signed_data.extend_from_slice(&[0, 0]);
    signed_data.extend_from_slice(&sct.timestamp.to_be_bytes());
    signed_data.extend_from_slice(&1_u16.to_be_bytes());
    signed_data.extend_from_slice(&issuer_key_hash);
    push_tls_u24(&mut signed_data, &precert_der)?;
    let extensions = sct.extensions.as_slice();
    let extension_len = u16::try_from(extensions.len())
        .map_err(|_| AppError::Validation("Sigstore SCT extensions are too large".to_string()))?;
    signed_data.extend_from_slice(&extension_len.to_be_bytes());
    signed_data.extend_from_slice(extensions);

    ctfe_key
        .key
        .verify_signature(
            Signature::Raw(sct.signature.signature.as_slice()),
            &signed_data,
        )
        .map_err(|error| {
            AppError::Validation(format!(
                "Sigstore certificate SCT verification failed: {error}"
            ))
        })
}

fn verify_sct_algorithm(
    key_details: &str,
    algorithm: &x509_cert::ext::pkix::SignatureAndHashAlgorithm,
) -> AppResult<()> {
    let matches = match key_details {
        "PKIX_ECDSA_P256_SHA_256" => matches!(
            (&algorithm.hash, &algorithm.signature),
            (HashAlgorithm::Sha256, SignatureAlgorithm::Ecdsa)
        ),
        "PKIX_ECDSA_P384_SHA_384" => matches!(
            (&algorithm.hash, &algorithm.signature),
            (HashAlgorithm::Sha384, SignatureAlgorithm::Ecdsa)
        ),
        "PKIX_ED25519" => matches!(
            (&algorithm.hash, &algorithm.signature),
            (
                HashAlgorithm::Intrinsic | HashAlgorithm::None,
                SignatureAlgorithm::Ed25519
            )
        ),
        _ => false,
    };
    if matches {
        Ok(())
    } else {
        Err(AppError::Validation(format!(
            "Sigstore SCT algorithm does not match trusted CT key type '{key_details}'"
        )))
    }
}

fn push_tls_u24(output: &mut Vec<u8>, value: &[u8]) -> AppResult<()> {
    let length = u32::try_from(value.len())
        .ok()
        .filter(|length| *length <= 0x00ff_ffff)
        .ok_or_else(|| AppError::Validation("Sigstore precertificate is too large".to_string()))?;
    output.extend_from_slice(&[(length >> 16) as u8, (length >> 8) as u8, length as u8]);
    output.extend_from_slice(value);
    Ok(())
}

fn lower_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(&mut hex, "{byte:02x}");
    }
    hex
}

fn sigstore_trust_material_cache() -> AppResult<&'static RwLock<CachedSigstoreTrustMaterial>> {
    if let Some(cache) = SIGSTORE_TRUST_MATERIAL.get() {
        return Ok(cache);
    }

    let embedded = load_embedded_sigstore_trust_material()?;
    let _ = SIGSTORE_TRUST_MATERIAL.set(RwLock::new(embedded));
    SIGSTORE_TRUST_MATERIAL.get().ok_or_else(|| {
        AppError::Repository("failed to initialize Sigstore trust material".to_string())
    })
}

fn current_sigstore_trust_material() -> AppResult<Arc<SigstoreTrustMaterial>> {
    sigstore_trust_material_cache()?
        .read()
        .map(|cached| cached.material.clone())
        .map_err(|_| AppError::Repository("Sigstore trust-root cache lock is poisoned".to_string()))
}

fn load_embedded_sigstore_trust_material() -> AppResult<CachedSigstoreTrustMaterial> {
    let cached = load_sigstore_trust_material_from_snapshot(
        EMBEDDED_SIGSTORE_TRUST_ROOT,
        EMBEDDED_SIGSTORE_TRUST_ROOT_PROVENANCE,
    )?;
    info!(digest = %cached.digest, source = %cached.source, "loaded embedded Sigstore trust material");
    Ok(cached)
}

fn load_sigstore_trust_material_from_snapshot(
    root: &[u8],
    provenance_json: &[u8],
) -> AppResult<CachedSigstoreTrustMaterial> {
    let provenance: SigstoreTrustRootProvenance =
        serde_json::from_slice(provenance_json).map_err(|error| {
            AppError::Repository(format!(
                "failed to parse embedded Sigstore trust-root provenance: {error}"
            ))
        })?;
    if provenance.schema_version != 1
        || provenance.target != "trusted_root.json"
        || provenance.source.trim().is_empty()
    {
        return Err(AppError::Repository(
            "embedded Sigstore trust-root provenance has an invalid schema, source, or target"
                .to_string(),
        ));
    }

    let digest = lower_hex(&Sha256::digest(root));
    if !digest.eq_ignore_ascii_case(&provenance.sha256) {
        return Err(AppError::Repository(format!(
            "embedded Sigstore trust-root digest mismatch: expected {}, got {digest}",
            provenance.sha256
        )));
    }
    let material = Arc::new(parse_trusted_root_document(root)?);
    let source = format!(
        "{}/{}",
        provenance.source.trim_end_matches('/'),
        provenance.target
    );
    Ok(CachedSigstoreTrustMaterial {
        material,
        digest,
        source,
        refreshed_at: None,
    })
}

async fn retrieve_sigstore_trust_material() -> AppResult<CachedSigstoreTrustMaterial> {
    scryer_outbound_http::install_default_rustls_provider();
    let cache_dir = tempfile::tempdir().map_err(|error| {
        AppError::Repository(format!(
            "failed to create temporary Sigstore trust-root cache: {error}"
        ))
    })?;
    let _trust_root = SigstoreTrustRoot::new(Some(cache_dir.path()))
        .await
        .map_err(|error| {
            AppError::Repository(format!("failed to load Sigstore trust root: {error}"))
        })?;
    let trusted_root_json =
        std::fs::read(cache_dir.path().join("trusted_root.json")).map_err(|error| {
            AppError::Repository(format!(
                "failed to read TUF-verified Sigstore trusted_root.json: {error}"
            ))
        })?;
    let digest = lower_hex(&Sha256::digest(&trusted_root_json));
    let material = Arc::new(parse_trusted_root_document(&trusted_root_json)?);
    Ok(CachedSigstoreTrustMaterial {
        material,
        digest,
        source: SIGSTORE_TRUST_SOURCE.to_string(),
        refreshed_at: Some(Instant::now()),
    })
}

fn parse_trusted_root_document(raw: &[u8]) -> AppResult<SigstoreTrustMaterial> {
    let trusted_root: TrustedRootDocument = serde_json::from_slice(raw).map_err(|error| {
        AppError::Repository(format!(
            "failed to parse Sigstore trusted_root.json: {error}"
        ))
    })?;
    if !trusted_root
        .media_type
        .starts_with("application/vnd.dev.sigstore.trustedroot")
    {
        return Err(AppError::Repository(format!(
            "unsupported Sigstore trusted-root media type '{}'",
            trusted_root.media_type
        )));
    }

    let rekor_keys = parse_trusted_log_keys(trusted_root.tlogs, "Rekor")?;
    let ctfe_keys = parse_trusted_log_keys(trusted_root.ctlogs, "CT")?;
    let fulcio_chains =
        parse_certificate_authority_trust_chains(trusted_root.certificate_authorities, "Fulcio")?;
    let tsa_chains =
        parse_certificate_authority_trust_chains(trusted_root.timestamp_authorities, "timestamp")?;
    if rekor_keys.is_empty() {
        return Err(AppError::Repository(
            "Sigstore Rekor trust root is empty".to_string(),
        ));
    }
    if ctfe_keys.is_empty() {
        return Err(AppError::Repository(
            "Sigstore CT trust root is empty".to_string(),
        ));
    }
    if fulcio_chains.is_empty() {
        return Err(AppError::Repository(
            "Sigstore Fulcio trust root is empty".to_string(),
        ));
    }
    if tsa_chains.is_empty() {
        return Err(AppError::Repository(
            "Sigstore timestamp-authority trust root is empty".to_string(),
        ));
    }
    Ok(SigstoreTrustMaterial {
        rekor_keys,
        ctfe_keys,
        fulcio_chains,
        tsa_chains,
    })
}

fn parse_trusted_log_keys(
    logs: Vec<TrustedLogInstance>,
    label: &str,
) -> AppResult<BTreeMap<String, TimedVerificationKey>> {
    let mut parsed = BTreeMap::new();
    for log in logs {
        let key_id = lower_hex(&decode_trusted_root_base64(
            &log.log_id.key_id,
            &format!("{label} log ID"),
        )?);
        let key_der = decode_trusted_root_base64(
            &log.public_key.raw_bytes,
            &format!("{label} public key '{key_id}'"),
        )?;
        let key = CosignVerificationKey::try_from_der(&key_der).map_err(|error| {
            AppError::Repository(format!(
                "failed to parse Sigstore {label} public key '{key_id}': {error}"
            ))
        })?;
        let value = TimedVerificationKey {
            key,
            valid_for: parse_trusted_time_window(&log.public_key.valid_for)?,
            key_details: log.public_key.key_details,
        };
        if parsed.insert(key_id.clone(), value).is_some() {
            return Err(AppError::Repository(format!(
                "Sigstore trust root contains duplicate {label} log ID '{key_id}'"
            )));
        }
    }
    Ok(parsed)
}

fn parse_certificate_authority_trust_chains(
    authorities: Vec<TrustedCertificateAuthority>,
    label: &str,
) -> AppResult<Vec<CertificateAuthorityTrustChain>> {
    authorities
        .into_iter()
        .enumerate()
        .map(|(authority_index, authority)| {
            let mut certificates = authority
                .cert_chain
                .certificates
                .into_iter()
                .enumerate()
                .map(|(certificate_index, certificate)| {
                    decode_trusted_root_base64(
                        &certificate.raw_bytes,
                        &format!(
                            "{label} authority {authority_index} certificate {certificate_index}"
                        ),
                    )
                    .map(CertificateDer::from)
                })
                .collect::<AppResult<Vec<_>>>()?;
            let certificate_validity = certificates
                .iter()
                .map(x509_certificate_validity)
                .collect::<AppResult<Vec<_>>>()?;
            let anchor_der = certificates.pop().ok_or_else(|| {
                AppError::Repository(format!(
                    "Sigstore Fulcio authority {authority_index} has an empty certificate chain"
                ))
            })?;
            let anchor = webpki::anchor_from_trusted_cert(&anchor_der)
                .map(|anchor| anchor.to_owned())
                .map_err(|error| {
                    AppError::Repository(format!(
                        "failed to parse Sigstore Fulcio authority {authority_index} anchor: {error}"
                    ))
                })?;
            Ok(CertificateAuthorityTrustChain {
                anchor,
                anchor_der,
                intermediates: certificates,
                valid_for: parse_trusted_time_window(&authority.valid_for)?,
                certificate_validity,
            })
        })
        .collect()
}

fn x509_certificate_validity(cert_der: &CertificateDer<'_>) -> AppResult<TimeWindow> {
    let cert = Certificate::from_der(cert_der.as_ref()).map_err(|error| {
        AppError::Repository(format!(
            "failed to parse Sigstore Fulcio certificate validity: {error}"
        ))
    })?;
    let validity = &cert.tbs_certificate.validity;
    let start = i64::try_from(validity.not_before.to_unix_duration().as_secs()).map_err(|_| {
        AppError::Repository("Sigstore Fulcio certificate notBefore is out of range".to_string())
    })?;
    let end = i64::try_from(validity.not_after.to_unix_duration().as_secs()).map_err(|_| {
        AppError::Repository("Sigstore Fulcio certificate notAfter is out of range".to_string())
    })?;
    Ok(TimeWindow {
        start: Some(start),
        end: Some(end),
    })
}

fn parse_trusted_time_window(range: &TrustedTimeRange) -> AppResult<TimeWindow> {
    let parse = |value: &str, label: &str| {
        chrono::DateTime::parse_from_rfc3339(value)
            .map(|value| value.timestamp())
            .map_err(|error| {
                AppError::Repository(format!(
                    "invalid Sigstore trust-root {label} timestamp '{value}': {error}"
                ))
            })
    };
    Ok(TimeWindow {
        start: Some(parse(&range.start, "start")?),
        end: range
            .end
            .as_deref()
            .map(|end| parse(end, "end"))
            .transpose()?,
    })
}

fn decode_trusted_root_base64(value: &str, label: &str) -> AppResult<Vec<u8>> {
    base64::engine::general_purpose::STANDARD
        .decode(value.as_bytes())
        .map_err(|error| {
            AppError::Repository(format!(
                "invalid base64 encoding for Sigstore trust-root {label}: {error}"
            ))
        })
}

pub async fn prime_sigstore_trust_roots() -> AppResult<()> {
    let cache = sigstore_trust_material_cache()?;
    let requested_at = Instant::now();
    let refresh_guard = SIGSTORE_TRUST_REFRESH
        .get_or_init(|| AsyncMutex::new(()))
        .lock()
        .await;

    if cache
        .read()
        .map_err(|_| {
            AppError::Repository("Sigstore trust-root cache lock is poisoned".to_string())
        })?
        .refreshed_at
        .is_some_and(|refreshed_at| refreshed_at >= requested_at)
    {
        drop(refresh_guard);
        return Ok(());
    }

    let started_at = Instant::now();
    let refresh_result = tokio::time::timeout(
        SIGSTORE_TRUST_REFRESH_TIMEOUT,
        retrieve_sigstore_trust_material(),
    )
    .await;
    let duration_ms = started_at.elapsed().as_millis();
    let refreshed = match refresh_result {
        Ok(Ok(refreshed)) => refreshed,
        Ok(Err(error)) => {
            let current = cache.read().map_err(|_| {
                AppError::Repository("Sigstore trust-root cache lock is poisoned".to_string())
            })?;
            warn!(
                error = %error,
                retained_digest = %current.digest,
                retained_source = %current.source,
                duration_ms,
                outcome = "failure",
                "failed to refresh Sigstore trust material; retaining current snapshot"
            );
            return Err(error);
        }
        Err(_) => {
            let error = AppError::Repository(format!(
                "Sigstore trust-root refresh timed out after {} seconds",
                SIGSTORE_TRUST_REFRESH_TIMEOUT.as_secs()
            ));
            let current = cache.read().map_err(|_| {
                AppError::Repository("Sigstore trust-root cache lock is poisoned".to_string())
            })?;
            warn!(
                retained_digest = %current.digest,
                retained_source = %current.source,
                duration_ms,
                outcome = "timeout",
                "Sigstore trust-material refresh timed out; retaining current snapshot"
            );
            return Err(error);
        }
    };
    let digest = refreshed.digest.clone();
    let source = refreshed.source.clone();
    *cache.write().map_err(|_| {
        AppError::Repository("Sigstore trust-root cache lock is poisoned".to_string())
    })? = refreshed;
    drop(refresh_guard);
    info!(%digest, %source, duration_ms, outcome = "success", "refreshed Sigstore trust material");
    Ok(())
}

#[derive(Debug, Default)]
struct CertificateIdentity {
    issuers: Vec<String>,
    repositories: Vec<String>,
    workflow_uris: Vec<String>,
}

fn verify_signer_identity(cert_pem: &str, required_signer: &RequiredSigner) -> AppResult<()> {
    let cert = Certificate::from_pem(cert_pem.as_bytes()).map_err(|error| {
        AppError::Validation(format!("failed to parse Sigstore certificate: {error}"))
    })?;
    let mut identity = CertificateIdentity::default();
    identity
        .issuers
        .extend(cert_extension_raw_utf8(&cert, SIGSTORE_OIDC_ISSUER_OID)?);
    identity
        .issuers
        .extend(cert_extension_der_utf8(&cert, SIGSTORE_OIDC_ISSUER_V2_OID)?);
    identity.repositories.extend(cert_extension_raw_utf8(
        &cert,
        SIGSTORE_GITHUB_WORKFLOW_REPOSITORY_OID,
    )?);
    identity.repositories.extend(cert_extension_der_utf8(
        &cert,
        SIGSTORE_SOURCE_REPOSITORY_URI_OID,
    )?);
    identity.workflow_uris = cert_subject_uris(&cert)?;
    identity.workflow_uris.extend(cert_extension_der_utf8(
        &cert,
        SIGSTORE_BUILD_SIGNER_URI_OID,
    )?);
    identity.workflow_uris.extend(cert_extension_der_utf8(
        &cert,
        SIGSTORE_BUILD_CONFIG_URI_OID,
    )?);
    verify_certificate_identity(&identity, required_signer)
}

fn verify_certificate_identity(
    identity: &CertificateIdentity,
    required_signer: &RequiredSigner,
) -> AppResult<()> {
    if identity.issuers.is_empty()
        || identity
            .issuers
            .iter()
            .any(|issuer| issuer != GITHUB_ACTIONS_OIDC_ISSUER)
    {
        return Err(AppError::Validation(format!(
            "Sigstore OIDC issuer mismatch: expected '{GITHUB_ACTIONS_OIDC_ISSUER}'"
        )));
    }

    let repository_uri = format!("https://github.com/{}", required_signer.github_repository);
    if identity.repositories.is_empty()
        || identity.repositories.iter().any(|repository| {
            repository != &required_signer.github_repository && repository != &repository_uri
        })
    {
        return Err(AppError::Validation(format!(
            "Sigstore signer repo mismatch: expected '{}'",
            required_signer.github_repository
        )));
    }

    if let Some(expected_workflow) = required_signer.github_workflow.as_deref() {
        let matched = identity.workflow_uris.iter().any(|uri| {
            github_workflow_uri_matches(
                uri,
                &required_signer.github_repository,
                expected_workflow,
                required_signer.github_ref.as_deref(),
            )
        });
        if !matched {
            let expected_ref = required_signer
                .github_ref
                .as_deref()
                .unwrap_or("a refs/tags/* release ref");
            return Err(AppError::Validation(format!(
                "Sigstore workflow identity mismatch for '{}'; expected '{}@{}'",
                required_signer.github_repository, expected_workflow, expected_ref
            )));
        }
    } else if required_signer.github_ref.is_some() {
        return Err(AppError::Validation(
            "Sigstore signer policy cannot require a Git ref without a workflow".to_string(),
        ));
    }
    Ok(())
}

fn github_workflow_uri_matches(
    uri: &str,
    repository: &str,
    workflow: &str,
    expected_ref: Option<&str>,
) -> bool {
    let expected_prefix = format!("https://github.com/{repository}/");
    let Some(workflow_and_ref) = uri.strip_prefix(&expected_prefix) else {
        return false;
    };
    let Some((actual_workflow, git_ref)) = workflow_and_ref.rsplit_once('@') else {
        return false;
    };
    let Some(tag) = git_ref.strip_prefix("refs/tags/") else {
        return false;
    };
    let ref_matches = match expected_ref {
        Some(expected_ref) => expected_ref == git_ref,
        None => true,
    };
    actual_workflow == workflow
        && !tag.is_empty()
        && !git_ref.contains(['?', '#'])
        && !workflow.contains(['?', '#', '@'])
        && ref_matches
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

fn cert_extension<'a>(cert: &'a Certificate, oid: &str) -> AppResult<Option<&'a Extension>> {
    let Some(extensions) = cert.tbs_certificate.extensions.as_deref() else {
        return Ok(None);
    };
    let mut matches = extensions
        .iter()
        .filter(|ext: &&Extension| ext.extn_id.to_string() == oid);
    let extension = matches.next();
    if matches.next().is_some() {
        return Err(AppError::Validation(format!(
            "Sigstore certificate contains duplicate extension {oid}"
        )));
    }
    Ok(extension)
}

fn cert_extension_raw_utf8(cert: &Certificate, oid: &str) -> AppResult<Option<String>> {
    cert_extension(cert, oid)?
        .map(|extension| {
            std::str::from_utf8(extension.extn_value.as_bytes())
                .map(str::to_owned)
                .map_err(|_| {
                    AppError::Validation(format!(
                        "Sigstore certificate extension {oid} is not valid UTF-8"
                    ))
                })
        })
        .transpose()
}

fn cert_extension_der_utf8(cert: &Certificate, oid: &str) -> AppResult<Option<String>> {
    cert_extension(cert, oid)?
        .map(|extension| {
            Utf8StringRef::from_der(extension.extn_value.as_bytes())
                .map(|value| value.as_str().to_owned())
                .map_err(|error| {
                    AppError::Validation(format!(
                        "Sigstore certificate extension {oid} is not valid DER UTF8String: {error}"
                    ))
                })
        })
        .transpose()
}

fn cert_subject_uris(cert: &Certificate) -> AppResult<Vec<String>> {
    let san = cert
        .tbs_certificate
        .get::<SubjectAltName>()
        .map_err(|error| AppError::Validation(format!("failed to read certificate SAN: {error}")))?
        .map(|(_, san)| san);
    let Some(san) = san else {
        return Ok(Vec::new());
    };
    Ok(san
        .0
        .iter()
        .filter_map(|name| match name {
            GeneralName::UniformResourceIdentifier(uri) => Some(uri.to_string()),
            _ => None,
        })
        .collect())
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
            TimedVerificationKey {
                key: signer
                    .to_verification_key()
                    .expect("derive test Rekor verification key"),
                valid_for: TimeWindow::default(),
                key_details: "PKIX_ECDSA_P256_SHA_256".to_string(),
            },
        )]);
        (bundle, keys)
    }

    fn signed_v03_tlog_entry(
        canonicalized_body: &[u8],
        integrated_time: i64,
        log_index: i64,
    ) -> (serde_json::Value, RekorVerificationKeys) {
        let signer = SigningScheme::ECDSA_P256_SHA256_ASN1
            .create_signer()
            .expect("create test Rekor signer");
        let log_key_id = [7_u8; 32];
        let log_id_hex = lower_hex(&log_key_id);
        let payload = RekorPayload {
            body: base64::engine::general_purpose::STANDARD.encode(canonicalized_body),
            integrated_time,
            log_index,
            log_id: log_id_hex.clone(),
        };
        let canonical_payload =
            serde_json_canonicalizer::to_vec(&payload).expect("canonicalize v0.3 Rekor payload");
        let signed_entry_timestamp = signer
            .sign(&canonical_payload)
            .expect("sign v0.3 Rekor payload");

        let root_hash = sha256_prefixed(0, canonicalized_body);
        let checkpoint_note = format!(
            "rekor.test\n1\n{}\n",
            base64::engine::general_purpose::STANDARD.encode(root_hash)
        );
        let checkpoint_signature = signer
            .sign(checkpoint_note.as_bytes())
            .expect("sign v0.3 Rekor checkpoint");
        let mut checkpoint_signature_with_hint = log_key_id[..4].to_vec();
        checkpoint_signature_with_hint.extend_from_slice(&checkpoint_signature);
        let checkpoint = format!(
            "{checkpoint_note}\n— rekor.test {}\n",
            base64::engine::general_purpose::STANDARD.encode(checkpoint_signature_with_hint)
        );

        let entry = serde_json::json!({
            "logIndex": log_index.to_string(),
            "logId": {
                "keyId": base64::engine::general_purpose::STANDARD.encode(log_key_id),
            },
            "kindVersion": {
                "kind": "hashedrekord",
                "version": "0.0.1",
            },
            "integratedTime": integrated_time.to_string(),
            "inclusionPromise": {
                "signedEntryTimestamp": base64::engine::general_purpose::STANDARD
                    .encode(signed_entry_timestamp),
            },
            "inclusionProof": {
                "logIndex": "0",
                "rootHash": base64::engine::general_purpose::STANDARD.encode(root_hash),
                "treeSize": "1",
                "hashes": [],
                "checkpoint": {
                    "envelope": checkpoint,
                },
            },
            "canonicalizedBody": base64::engine::general_purpose::STANDARD
                .encode(canonicalized_body),
        });
        let keys = BTreeMap::from([(
            log_id_hex,
            TimedVerificationKey {
                key: signer
                    .to_verification_key()
                    .expect("derive v0.3 Rekor verification key"),
                valid_for: TimeWindow::default(),
                key_details: "PKIX_ECDSA_P256_SHA_256".to_string(),
            },
        )]);
        (entry, keys)
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

    #[test]
    fn rekor_bundle_verifier_enforces_the_key_validity_window() {
        let (bundle, mut keys) = signed_rekor_bundle();
        keys.get_mut("test-rekor-key").unwrap().valid_for = TimeWindow {
            start: Some(1_700_000_001),
            end: None,
        };

        let error = parse_and_verify_bundle(&bundle, &keys)
            .expect_err("Rekor key outside its validity window must fail closed");
        assert!(
            error
                .to_string()
                .contains("not valid at the integrated time")
        );
    }

    #[test]
    fn v03_rekor_verifier_accepts_a_complete_proof_and_checkpoint() {
        let body = br#"{"apiVersion":"0.0.1","kind":"hashedrekord","spec":{}}"#;
        let (entry, keys) = signed_v03_tlog_entry(body, 1_700_000_000, 42);
        let entry: TransparencyLogEntry =
            serde_json::from_value(entry).expect("parse v0.3 test tlog entry");

        verify_v03_tlog_entry(&entry, &keys)
            .expect("verify complete v0.3 Rekor proof and checkpoint");
    }

    #[test]
    fn v03_rekor_verifier_rejects_a_tampered_inclusion_proof() {
        let body = br#"{"apiVersion":"0.0.1","kind":"hashedrekord","spec":{}}"#;
        let (mut entry, keys) = signed_v03_tlog_entry(body, 1_700_000_000, 42);
        entry["inclusionProof"]["rootHash"] =
            serde_json::Value::String(base64::engine::general_purpose::STANDARD.encode([0_u8; 32]));
        let entry: TransparencyLogEntry =
            serde_json::from_value(entry).expect("parse tampered v0.3 test tlog entry");

        let error = verify_v03_tlog_entry(&entry, &keys)
            .expect_err("tampered v0.3 inclusion proof must fail closed");
        assert!(error.to_string().contains("checkpoint"));
    }

    #[test]
    fn cosign_v2_and_v3_bundle_profiles_verify_the_same_release() {
        let raw =
            include_bytes!("../../test-fixtures/sigstore/scryer-upgrade-manifest-v0.19.3.json");
        let legacy_bundle_raw = include_bytes!(
            "../../test-fixtures/sigstore/scryer-upgrade-manifest-v0.19.3.sigstore.json"
        );
        let legacy_bundle_text =
            std::str::from_utf8(legacy_bundle_raw).expect("legacy bundle is UTF-8");
        let legacy_bundle: SignedArtifactBundle =
            serde_json::from_str(legacy_bundle_text).expect("parse legacy bundle fixture");
        let mut trust_material = parse_trusted_root_document(EMBEDDED_SIGSTORE_TRUST_ROOT)
            .expect("load embedded Sigstore trust material");

        verify_legacy_signed_blob(
            raw,
            legacy_bundle_text,
            &required_github_signer(),
            &trust_material,
        )
        .expect("Cosign v2 bundle should verify");

        let canonicalized_body = base64::engine::general_purpose::STANDARD
            .decode(legacy_bundle.rekor_bundle.payload.body.as_bytes())
            .expect("decode legacy Rekor body");
        let (tlog_entry, synthetic_rekor_keys) = signed_v03_tlog_entry(
            &canonicalized_body,
            legacy_bundle.rekor_bundle.payload.integrated_time,
            legacy_bundle.rekor_bundle.payload.log_index,
        );
        trust_material.rekor_keys.extend(synthetic_rekor_keys);
        let cert_pem = normalize_bundle_cert(&legacy_bundle.cert).expect("normalize fixture cert");
        let cert_der = sigstore_certificate_der(&cert_pem).expect("decode fixture cert");
        let v03_bundle = serde_json::json!({
            "mediaType": SIGSTORE_BUNDLE_V03_MEDIA_TYPE,
            "verificationMaterial": {
                "certificate": {
                    "rawBytes": base64::engine::general_purpose::STANDARD.encode(cert_der),
                },
                "tlogEntries": [tlog_entry],
            },
            "messageSignature": {
                "messageDigest": {
                    "algorithm": "SHA2_256",
                    "digest": base64::engine::general_purpose::STANDARD
                        .encode(Sha256::digest(raw)),
                },
                "signature": legacy_bundle.base64_signature,
            },
        })
        .to_string();

        verify_v03_signed_blob(raw, &v03_bundle, &required_github_signer(), &trust_material)
            .expect("Cosign v3 / Sigstore v0.3 bundle should verify");

        let mut altered = raw.to_vec();
        altered[0] ^= 1;
        assert!(
            verify_v03_signed_blob(
                &altered,
                &v03_bundle,
                &required_github_signer(),
                &trust_material,
            )
            .is_err(),
            "v0.3 artifact tampering must fail closed"
        );
    }

    #[tokio::test]
    async fn v03_real_cosign_plugin_bundle_verifies_with_rfc3161_timestamp() {
        let raw = include_bytes!("../../test-fixtures/sigstore/fanzub-catalog-v2.1.0.json");
        let bundle =
            include_bytes!("../../test-fixtures/sigstore/fanzub-catalog-v2.1.0.sigstore.json");
        let required_signer = RequiredSigner {
            github_repository: "scryer-media/scryer-plugins".to_string(),
            github_workflow: Some(".github/workflows/release-plugin-v3.yml".to_string()),
            github_ref: Some("refs/tags/plugins-v3/release/1787972354-109745e51603".to_string()),
        };

        verify_signed_blob(raw.to_vec(), bundle.to_vec(), required_signer.clone())
            .await
            .expect("real Cosign v3 plugin bundle should verify");

        let mut altered = raw.to_vec();
        altered[0] ^= 1;
        assert!(
            verify_signed_blob(altered, bundle.to_vec(), required_signer.clone())
                .await
                .is_err(),
            "real Cosign v3 artifact tampering must fail closed"
        );

        let mut tampered_bundle: serde_json::Value =
            serde_json::from_slice(bundle).expect("parse real Cosign v3 fixture");
        let timestamp = tampered_bundle["verificationMaterial"]["timestampVerificationData"]
            ["rfc3161Timestamps"][0]["signedTimestamp"]
            .as_str()
            .expect("fixture has RFC3161 timestamp");
        let mut timestamp = base64::engine::general_purpose::STANDARD
            .decode(timestamp)
            .expect("decode fixture RFC3161 timestamp");
        *timestamp.last_mut().expect("timestamp is nonempty") ^= 1;
        tampered_bundle["verificationMaterial"]["timestampVerificationData"]["rfc3161Timestamps"]
            [0]["signedTimestamp"] =
            serde_json::Value::String(base64::engine::general_purpose::STANDARD.encode(timestamp));
        assert!(
            verify_signed_blob(
                raw.to_vec(),
                serde_json::to_vec(&tampered_bundle).expect("serialize tampered bundle"),
                required_signer,
            )
            .await
            .is_err(),
            "tampered RFC3161 timestamp must fail closed"
        );
    }

    fn required_github_signer() -> RequiredSigner {
        RequiredSigner {
            github_repository: "scryer-media/scryer".to_string(),
            github_workflow: Some(".github/workflows/scryer.yml".to_string()),
            github_ref: Some("refs/tags/scryer-v0.19.3".to_string()),
        }
    }

    fn github_identity(workflow_uri: &str) -> CertificateIdentity {
        CertificateIdentity {
            issuers: vec![GITHUB_ACTIONS_OIDC_ISSUER.to_string()],
            repositories: vec!["https://github.com/scryer-media/scryer".to_string()],
            workflow_uris: vec![workflow_uri.to_string()],
        }
    }

    #[test]
    fn signer_identity_accepts_an_exact_github_workflow_uri() {
        let identity = github_identity(
            "https://github.com/scryer-media/scryer/.github/workflows/scryer.yml@refs/tags/scryer-v0.19.3",
        );

        verify_certificate_identity(&identity, &required_github_signer())
            .expect("exact GitHub workflow identity should verify");
    }

    #[test]
    fn signer_identity_rejects_workflow_substrings_and_crafted_refs() {
        let suffix = github_identity(
            "https://github.com/scryer-media/scryer/.github/workflows/scryer.yml.evil@refs/heads/main",
        );
        assert!(verify_certificate_identity(&suffix, &required_github_signer()).is_err());

        let crafted_ref = github_identity(
            "https://github.com/scryer-media/scryer/.github/workflows/evil.yml@refs/heads/attack/.github/workflows/scryer.yml",
        );
        assert!(verify_certificate_identity(&crafted_ref, &required_github_signer()).is_err());

        for git_ref in [
            "refs/pull/123/merge",
            "refs/heads/main",
            "refs/tags/scryer-v0.19.2",
        ] {
            let identity = github_identity(&format!(
                "https://github.com/scryer-media/scryer/.github/workflows/scryer.yml@{git_ref}"
            ));
            assert!(
                verify_certificate_identity(&identity, &required_github_signer()).is_err(),
                "unexpected ref {git_ref} must fail closed"
            );
        }
    }

    #[test]
    fn signer_identity_without_an_exact_ref_still_requires_a_release_tag() {
        let mut signer = required_github_signer();
        signer.github_ref = None;
        let tag = github_identity(
            "https://github.com/scryer-media/scryer/.github/workflows/scryer.yml@refs/tags/scryer-v0.19.3",
        );
        verify_certificate_identity(&tag, &signer)
            .expect("legacy signer policy should accept a release tag");

        let pull_request = github_identity(
            "https://github.com/scryer-media/scryer/.github/workflows/scryer.yml@refs/pull/123/merge",
        );
        assert!(verify_certificate_identity(&pull_request, &signer).is_err());
    }

    #[test]
    fn signer_identity_rejects_a_non_github_oidc_issuer() {
        let mut identity = github_identity(
            "https://github.com/scryer-media/scryer/.github/workflows/scryer.yml@refs/heads/main",
        );
        identity.issuers = vec!["https://example.invalid".to_string()];

        let error = verify_certificate_identity(&identity, &required_github_signer())
            .expect_err("wrong issuer must fail closed");
        assert!(error.to_string().contains("OIDC issuer mismatch"));
    }

    #[test]
    fn embedded_trust_root_matches_its_receipt_and_loads_offline() {
        let cached = load_embedded_sigstore_trust_material()
            .expect("embedded Sigstore trust root should match its receipt and parse");
        let material = cached.material;
        assert!(!material.rekor_keys.is_empty());
        assert!(!material.ctfe_keys.is_empty());
        assert!(!material.fulcio_chains.is_empty());
        assert!(!material.tsa_chains.is_empty());
        assert_eq!(cached.source, SIGSTORE_TRUST_SOURCE);
    }

    #[test]
    fn embedded_trust_root_rejects_digest_mismatch_and_malformed_content() {
        let mut tampered = EMBEDDED_SIGSTORE_TRUST_ROOT.to_vec();
        tampered[0] ^= 1;
        let error = load_sigstore_trust_material_from_snapshot(
            &tampered,
            EMBEDDED_SIGSTORE_TRUST_ROOT_PROVENANCE,
        )
        .err()
        .expect("tampered embedded trust root must fail closed");
        assert!(error.to_string().contains("digest mismatch"));

        let malformed = b"not a trusted root";
        let receipt = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1,
            "source": "https://tuf-repo-cdn.sigstore.dev",
            "target": "trusted_root.json",
            "sha256": lower_hex(&Sha256::digest(malformed)),
        }))
        .unwrap();
        let error = load_sigstore_trust_material_from_snapshot(malformed, &receipt)
            .err()
            .expect("malformed embedded trust root must fail closed");
        assert!(
            error
                .to_string()
                .contains("failed to parse Sigstore trusted_root.json")
        );
    }
}

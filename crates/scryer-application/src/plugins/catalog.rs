use std::collections::HashSet;

#[cfg(feature = "runtime-plugin-trust")]
use std::{
    collections::BTreeMap,
    io::Read,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

#[cfg(feature = "runtime-plugin-trust")]
use base64::Engine;
#[cfg(feature = "runtime-plugin-trust")]
use const_oid::db::rfc5280::ID_KP_CODE_SIGNING;
#[cfg(feature = "runtime-plugin-trust")]
use rustls_pki_types::{CertificateDer, TrustAnchor, UnixTime};
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
#[cfg(feature = "runtime-plugin-trust")]
use sha2::{Digest, Sha256};
#[cfg(feature = "runtime-plugin-trust")]
use sigstore::{
    cosign::{CosignCapabilities, bundle::SignedArtifactBundle},
    crypto::{CosignVerificationKey, SigningScheme},
    trust::{TrustRoot, sigstore::SigstoreTrustRoot},
};
#[cfg(feature = "runtime-plugin-trust")]
use tokio::sync::Semaphore;
#[cfg(feature = "runtime-plugin-trust")]
use tracing::debug;
use url::Url;
#[cfg(feature = "runtime-plugin-trust")]
use webpki::{EndEntityCert, KeyUsage};
#[cfg(feature = "runtime-plugin-trust")]
use x509_cert::{
    Certificate,
    der::{DecodePem, Encode},
    ext::{
        Extension,
        pkix::{SubjectAltName, name::GeneralName},
    },
};

use crate::{AppError, AppResult};
use scryer_domain::PluginSupportTier;

#[cfg(test)]
const CHILD_CATALOG_SCHEMA_VERSION: &str = "scryer.plugin.child_catalog.v2";
#[cfg(feature = "runtime-plugin-trust")]
const SIGSTORE_GITHUB_WORKFLOW_NAME_OID: &str = "1.3.6.1.4.1.57264.1.4";
#[cfg(feature = "runtime-plugin-trust")]
const SIGSTORE_GITHUB_WORKFLOW_REPOSITORY_OID: &str = "1.3.6.1.4.1.57264.1.5";
#[cfg(feature = "runtime-plugin-trust")]
const SIGSTORE_GITHUB_WORKFLOW_REF_OID: &str = "1.3.6.1.4.1.57264.1.6";
pub const PLUGIN_CATALOG_JSON_OUTPUT_LIMIT: u64 = 16 * 1024 * 1024;
pub const PLUGIN_CATALOG_REDIRECT_OUTPUT_LIMIT: u64 = 2 * 1024 * 1024;
pub const PLUGIN_SIGNATURE_BUNDLE_OUTPUT_LIMIT: u64 = 2 * 1024 * 1024;
pub const RULE_PACK_MANIFEST_FALLBACK_OUTPUT_LIMIT: u64 = 32 * 1024 * 1024;
pub const MANUAL_PLUGIN_WASM_OUTPUT_LIMIT: u64 = 128 * 1024 * 1024;
#[cfg(feature = "runtime-plugin-trust")]
type RekorVerificationKeys = BTreeMap<String, CosignVerificationKey>;
#[cfg(feature = "runtime-plugin-trust")]
type FulcioTrustAnchors = Vec<TrustAnchor<'static>>;

#[cfg(feature = "runtime-plugin-trust")]
struct SigstoreTrustMaterial {
    rekor_keys: Arc<RekorVerificationKeys>,
    fulcio_anchors: Arc<FulcioTrustAnchors>,
}

#[cfg(feature = "runtime-plugin-trust")]
static SIGSTORE_TRUST_MATERIAL: OnceLock<Mutex<Option<Arc<SigstoreTrustMaterial>>>> =
    OnceLock::new();

#[cfg(feature = "runtime-plugin-trust")]
static VERIFY_LIMIT: OnceLock<Semaphore> = OnceLock::new();

#[cfg(test)]
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
// Deserialized by test catalog fixtures; serde construction is invisible to dead-code analysis.
#[allow(dead_code)]
pub struct RulePackCatalogEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub author: String,
    pub version: String,
    pub url: String,
    #[serde(default)]
    pub min_scryer_version: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RequiredSigner {
    pub github_repository: String,
    #[serde(default)]
    pub github_workflow: Option<String>,
}

pub const CATALOG_V3_SCHEMA_VERSION: &str = "scryer.plugin.catalog.v3";
pub const CATALOG_V3_REDIRECT_SCHEMA_VERSION: &str = "scryer.plugin.catalog.v3.redirect";
pub const CATALOG_V3_REDIRECT_BUNDLE_SUFFIX: &str = ".bundle.json";
pub const CATALOG_V3_RUNTIME_WASIP1: &str = "wasm32-wasip1";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginLifecycleStatus {
    Beta,
    Active,
    Deprecated,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogV3Redirect {
    pub schema_version: String,
    pub catalog_version: u64,
    pub artifacts: Vec<CatalogV3RedirectArtifact>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogV3RedirectArtifact {
    pub url: String,
    #[serde(default)]
    pub mirror_urls: Vec<String>,
    pub signature_url: String,
    #[serde(default)]
    pub signature_mirror_urls: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogV3 {
    pub schema_version: String,
    pub catalog_version: u64,
    pub plugins: Vec<CatalogV3PluginEntry>,
    #[serde(default)]
    pub community_sources: Vec<CatalogV3CommunitySource>,
    #[serde(default)]
    pub rule_packs: Vec<CatalogV3RulePackEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogV3CommunitySource {
    pub id: String,
    pub github_repository: String,
    pub support_tier: PluginSupportTier,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogV3PluginEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub plugin_type: String,
    pub provider_type: String,
    pub publisher: String,
    pub support_tier: PluginSupportTier,
    pub status: PluginLifecycleStatus,
    pub docs_url: String,
    pub source_repo: String,
    pub required_signer: RequiredSigner,
    pub releases: Vec<CatalogV3PluginRelease>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogV3PluginRelease {
    pub version: String,
    pub sdk_constraint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_scryer_version: Option<String>,
    pub artifacts: Vec<CatalogV3PluginArtifact>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogV3PluginArtifact {
    pub runtime: String,
    #[serde(default)]
    pub required_features: Vec<String>,
    pub url: String,
    #[serde(default)]
    pub mirror_urls: Vec<String>,
    pub signature_url: String,
    #[serde(default)]
    pub signature_mirror_urls: Vec<String>,
    pub digests: Vec<String>,
    pub wasm_digests: Vec<String>,
    pub bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogV3RulePackEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub author: String,
    pub releases: Vec<CatalogV3RulePackRelease>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogV3RulePackRelease {
    pub version: String,
    #[serde(default)]
    pub min_scryer_version: Option<String>,
    pub rule_pack_digests: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_pack_bytes: Option<u64>,
    pub artifacts: Vec<CatalogV3DistributionArtifact>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogV3DistributionArtifact {
    pub url: String,
    #[serde(default)]
    pub mirror_urls: Vec<String>,
    pub signature_url: String,
    #[serde(default)]
    pub signature_mirror_urls: Vec<String>,
    pub digests: Vec<String>,
}

#[cfg(test)]
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChildCatalog {
    pub schema_version: String,
    pub id: String,
    pub name: String,
    pub description: String,
    pub plugin_type: String,
    pub provider_type: String,
    pub publisher: String,
    pub support_tier: PluginSupportTier,
    pub docs_url: String,
    pub source_repo: String,
    pub releases: Vec<ChildCatalogRelease>,
}

#[cfg(test)]
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChildCatalogRelease {
    pub version: String,
    pub sdk_constraint: String,
    pub artifact_manifest_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubRepo {
    pub owner: String,
    pub name: String,
}

#[cfg(test)]
#[derive(Clone, Debug)]
pub struct CatalogOutageStatus {
    pub github_available: bool,
    pub blocked_actions: Vec<String>,
}

#[cfg(test)]
#[derive(Debug, Deserialize)]
struct GitHubStatusSummary {
    status: GitHubOverallStatus,
    components: Vec<GitHubStatusComponent>,
}

#[cfg(test)]
#[derive(Debug, Deserialize)]
struct GitHubOverallStatus {
    indicator: String,
}

#[cfg(test)]
#[derive(Debug, Deserialize)]
struct GitHubStatusComponent {
    name: String,
    status: String,
}

impl GitHubRepo {
    pub fn parse(input: &str) -> AppResult<Self> {
        let trimmed = input.trim().trim_end_matches('/');
        if let Some((owner, name)) = trimmed.split_once('/')
            && !trimmed.starts_with("http://")
            && !trimmed.starts_with("https://")
        {
            return Self::from_parts(owner, name);
        }

        let url = Url::parse(trimmed)
            .map_err(|e| AppError::Validation(format!("invalid GitHub repository URL: {e}")))?;
        if url.host_str() != Some("github.com") {
            return Err(AppError::Validation(
                "manual plugin repositories must be hosted on github.com".to_string(),
            ));
        }
        let mut segments = url
            .path_segments()
            .ok_or_else(|| AppError::Validation("invalid GitHub repository URL".to_string()))?;
        let owner = segments
            .next()
            .ok_or_else(|| AppError::Validation("GitHub owner is missing".to_string()))?;
        let name = segments
            .next()
            .ok_or_else(|| AppError::Validation("GitHub repo name is missing".to_string()))?;
        Self::from_parts(owner, name)
    }

    pub fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }

    #[cfg(test)]
    pub fn release_asset_prefix(&self) -> String {
        format!(
            "https://github.com/{}/{}/releases/download/",
            self.owner, self.name
        )
    }

    pub fn catalog_v3_url(&self) -> String {
        format!(
            "https://github.com/{}/{}/releases/latest/download/catalog-v3.json",
            self.owner, self.name
        )
    }

    pub fn delegated_catalog_v3_url(&self) -> String {
        format!(
            "https://github.com/{}/{}/releases/download/catalog%2Fv3/catalog-v3.min.json.zst",
            self.owner, self.name
        )
    }

    fn from_parts(owner: &str, name: &str) -> AppResult<Self> {
        let owner = owner.trim();
        let name = name.trim().trim_end_matches(".git");
        if owner.is_empty() || name.is_empty() {
            return Err(AppError::Validation(
                "GitHub repository must include owner and repo".to_string(),
            ));
        }
        Ok(Self {
            owner: owner.to_string(),
            name: name.to_string(),
        })
    }
}

#[cfg(test)]
pub fn parse_and_validate_child_catalog(
    raw: &[u8],
    manual_repo: Option<&GitHubRepo>,
) -> AppResult<ChildCatalog> {
    let catalog: ChildCatalog = serde_json::from_slice(raw)
        .map_err(|e| AppError::Validation(format!("invalid child plugin catalog JSON: {e}")))?;
    validate_child_catalog(&catalog, manual_repo)?;
    Ok(catalog)
}

pub fn parse_and_validate_catalog_v3(raw: &[u8]) -> AppResult<CatalogV3> {
    let catalog: CatalogV3 = serde_json::from_slice(raw)
        .map_err(|e| AppError::Validation(format!("invalid plugin catalog v3 JSON: {e}")))?;
    validate_catalog_v3(&catalog)?;
    Ok(catalog)
}

pub fn parse_and_validate_catalog_v3_redirect(raw: &[u8]) -> AppResult<CatalogV3Redirect> {
    let redirect: CatalogV3Redirect = serde_json::from_slice(raw)
        .map_err(|e| AppError::Validation(format!("invalid plugin catalog redirect JSON: {e}")))?;
    validate_catalog_v3_redirect(&redirect)?;
    Ok(redirect)
}

#[cfg(feature = "runtime-plugin-trust")]
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
    .map_err(|e| AppError::Repository(format!("plugin signature verification panicked: {e}")))?;
    drop(permit);
    result
}

#[cfg(not(feature = "runtime-plugin-trust"))]
pub async fn verify_signed_blob(
    _raw: Vec<u8>,
    _bundle_raw: Vec<u8>,
    _required_signer: RequiredSigner,
) -> AppResult<()> {
    Err(AppError::Validation(
        "plugin signature verification is not compiled into this target".to_string(),
    ))
}

#[cfg(feature = "runtime-plugin-trust")]
fn read_bounded_decompressed<R: Read>(
    mut reader: R,
    max_output_bytes: u64,
    label: String,
) -> AppResult<Vec<u8>> {
    let max_output_len = usize::try_from(max_output_bytes).map_err(|_| {
        AppError::Validation(format!(
            "{label} decompressed size limit {max_output_bytes} exceeds this platform's maximum buffer size"
        ))
    })?;
    let mut output = Vec::new();
    let mut buffer = [0u8; 8192];

    loop {
        let remaining = max_output_len.saturating_sub(output.len());
        let read_len = if remaining == 0 {
            1
        } else {
            remaining.min(buffer.len())
        };
        let bytes_read = reader.read(&mut buffer[..read_len]).map_err(|e| {
            AppError::Repository(format!("failed to decompress {label} payload: {e}"))
        })?;
        if bytes_read == 0 {
            return Ok(output);
        }
        if bytes_read > remaining {
            return Err(AppError::Validation(format!(
                "{label} decompressed payload exceeds maximum size of {max_output_bytes} bytes"
            )));
        }
        output.extend_from_slice(&buffer[..bytes_read]);
    }
}

pub fn bound_uncompressed_bytes(
    bytes: Vec<u8>,
    max_output_bytes: u64,
    label: &str,
) -> AppResult<Vec<u8>> {
    let actual_bytes = u64::try_from(bytes.len()).map_err(|_| {
        AppError::Validation(format!(
            "{label} payload is too large to validate decompressed size"
        ))
    })?;
    if actual_bytes > max_output_bytes {
        return Err(AppError::Validation(format!(
            "{label} payload exceeds maximum size of {max_output_bytes} bytes"
        )));
    }
    Ok(bytes)
}

#[cfg(feature = "runtime-plugin-trust")]
pub async fn decompress_zstd(
    compressed: Vec<u8>,
    max_output_bytes: u64,
    label: impl Into<String>,
) -> AppResult<Vec<u8>> {
    let label = label.into();
    tokio::task::spawn_blocking(move || {
        let decoder = zstd::Decoder::new(compressed.as_slice()).map_err(|e| {
            AppError::Repository(format!(
                "failed to initialize zstd decoder for {label}: {e}"
            ))
        })?;
        read_bounded_decompressed(decoder, max_output_bytes, label)
    })
    .await
    .map_err(|e| AppError::Repository(format!("zstd decompression panicked: {e}")))?
}

#[cfg(not(feature = "runtime-plugin-trust"))]
pub async fn decompress_zstd(
    _compressed: Vec<u8>,
    _max_output_bytes: u64,
    _label: impl Into<String>,
) -> AppResult<Vec<u8>> {
    Err(AppError::Validation(
        "plugin zstd decompression is not compiled into this target".to_string(),
    ))
}

#[cfg(feature = "runtime-plugin-trust")]
pub async fn decompress_brotli(
    compressed: Vec<u8>,
    max_output_bytes: u64,
    label: impl Into<String>,
) -> AppResult<Vec<u8>> {
    let label = label.into();
    tokio::task::spawn_blocking(move || {
        let decoder = brotli::Decompressor::new(compressed.as_slice(), 4096);
        read_bounded_decompressed(decoder, max_output_bytes, label)
    })
    .await
    .map_err(|e| AppError::Repository(format!("brotli decompression panicked: {e}")))?
}

#[cfg(not(feature = "runtime-plugin-trust"))]
pub async fn decompress_brotli(
    _compressed: Vec<u8>,
    _max_output_bytes: u64,
    _label: impl Into<String>,
) -> AppResult<Vec<u8>> {
    Err(AppError::Validation(
        "plugin brotli decompression is not compiled into this target".to_string(),
    ))
}

#[cfg(feature = "runtime-plugin-trust")]
pub async fn compress_zstd(bytes: Vec<u8>, level: i32) -> AppResult<Vec<u8>> {
    tokio::task::spawn_blocking(move || {
        zstd::encode_all(bytes.as_slice(), level)
            .map_err(|e| AppError::Repository(format!("failed to compress zstd payload: {e}")))
    })
    .await
    .map_err(|e| AppError::Repository(format!("zstd compression panicked: {e}")))?
}

#[cfg(not(feature = "runtime-plugin-trust"))]
pub async fn compress_zstd(_bytes: Vec<u8>, _level: i32) -> AppResult<Vec<u8>> {
    Err(AppError::Validation(
        "plugin zstd compression is not compiled into this target".to_string(),
    ))
}

pub fn blake3_digest(bytes: &[u8]) -> String {
    format!("blake3:{}", blake3::hash(bytes).to_hex())
}

pub fn parse_digest_string(input: &str) -> AppResult<(String, String)> {
    let trimmed = input.trim();
    let (algo, digest) = trimmed
        .split_once(':')
        .ok_or_else(|| AppError::Validation(format!("invalid digest string '{trimmed}'")))?;
    let algo = algo.trim().to_ascii_lowercase();
    let digest = normalize_hex_digest(digest)?;
    if algo.is_empty() {
        return Err(AppError::Validation(
            "digest algorithm is missing".to_string(),
        ));
    }
    Ok((algo, digest))
}

pub fn verify_split_digest(
    label: &str,
    algorithm: &str,
    expected_digest: &str,
    bytes: &[u8],
) -> AppResult<()> {
    let normalized_algorithm = algorithm.trim().to_ascii_lowercase();
    let expected_digest = normalize_hex_digest(expected_digest)?;
    match normalized_algorithm.as_str() {
        "blake3" => {
            let actual_digest = blake3::hash(bytes).to_hex().to_string();
            if actual_digest.eq_ignore_ascii_case(&expected_digest) {
                Ok(())
            } else {
                Err(AppError::Validation(format!(
                    "{label} digest mismatch: expected blake3:{expected_digest}, got blake3:{actual_digest}"
                )))
            }
        }
        _ => Err(AppError::Validation(format!(
            "{label} uses unsupported digest algorithm '{normalized_algorithm}'"
        ))),
    }
}

pub fn verify_digest_set(label: &str, expected_digests: &[String], bytes: &[u8]) -> AppResult<()> {
    for digest in expected_digests {
        let Ok((algorithm, digest_hex)) = parse_digest_string(digest) else {
            continue;
        };
        if algorithm != "blake3" {
            continue;
        }
        return verify_split_digest(label, &algorithm, &digest_hex, bytes);
    }
    if !expected_digests.is_empty() {
        return Err(AppError::Validation(format!(
            "{label} is missing a usable blake3 digest"
        )));
    }
    Err(AppError::Validation(format!(
        "{label} must declare at least one digest"
    )))
}

pub fn redirect_bundle_url_for(url: &str) -> String {
    if let Some(prefix) = url.strip_suffix(".json") {
        return format!("{prefix}{CATALOG_V3_REDIRECT_BUNDLE_SUFFIX}");
    }
    format!("{url}{CATALOG_V3_REDIRECT_BUNDLE_SUFFIX}")
}

#[cfg(test)]
pub fn github_outage_status_from_summary(raw: &[u8]) -> Option<CatalogOutageStatus> {
    let summary: GitHubStatusSummary = serde_json::from_slice(raw).ok()?;
    if summary.status.indicator == "none" {
        return Some(CatalogOutageStatus {
            github_available: true,
            blocked_actions: Vec::new(),
        });
    }

    let relevant_outage = summary.components.iter().any(|component| {
        matches!(component.name.as_str(), "API Requests" | "Git Operations")
            && component.status != "operational"
    });
    if !relevant_outage {
        return Some(CatalogOutageStatus {
            github_available: true,
            blocked_actions: Vec::new(),
        });
    }

    Some(CatalogOutageStatus {
        github_available: false,
        blocked_actions: vec![
            "catalog_refresh".to_string(),
            "install".to_string(),
            "install_manual".to_string(),
            "upgrade".to_string(),
            "manual_repo_inspection".to_string(),
        ],
    })
}

#[cfg(test)]
fn validate_child_catalog(
    catalog: &ChildCatalog,
    manual_repo: Option<&GitHubRepo>,
) -> AppResult<()> {
    if catalog.schema_version != CHILD_CATALOG_SCHEMA_VERSION {
        return Err(AppError::Validation(format!(
            "unsupported child plugin catalog schema '{}'",
            catalog.schema_version
        )));
    }

    require_non_empty("plugin id", &catalog.id)?;
    require_non_empty("plugin name", &catalog.name)?;
    require_non_empty("plugin type", &catalog.plugin_type)?;
    require_non_empty("provider type", &catalog.provider_type)?;
    require_non_empty("publisher", &catalog.publisher)?;
    require_non_empty("docs_url", &catalog.docs_url)?;
    require_non_empty("source_repo", &catalog.source_repo)?;
    let source_repo = GitHubRepo::parse(&catalog.source_repo)?;

    if let Some(manual_repo) = manual_repo
        && &source_repo != manual_repo
    {
        return Err(AppError::Validation(format!(
            "manual child catalog source repo '{}' does not match requested repo '{}'",
            source_repo.slug(),
            manual_repo.slug()
        )));
    }

    let mut versions = HashSet::new();
    for release in &catalog.releases {
        parse_version(&release.version)?;
        VersionReq::parse(&release.sdk_constraint).map_err(|e| {
            AppError::Validation(format!(
                "invalid sdk_constraint '{}' for plugin '{}': {e}",
                release.sdk_constraint, catalog.id
            ))
        })?;
        if !versions.insert(release.version.clone()) {
            return Err(AppError::Validation(format!(
                "duplicate release version '{}' for plugin '{}'",
                release.version, catalog.id
            )));
        }
        require_release_asset_url(
            "release manifest",
            &release.artifact_manifest_url,
            &source_repo,
        )?;
    }

    Ok(())
}

fn validate_catalog_v3_redirect(redirect: &CatalogV3Redirect) -> AppResult<()> {
    if redirect.schema_version != CATALOG_V3_REDIRECT_SCHEMA_VERSION {
        return Err(AppError::Validation(format!(
            "unsupported plugin catalog redirect schema '{}'",
            redirect.schema_version
        )));
    }
    if redirect.catalog_version == 0 {
        return Err(AppError::Validation(
            "plugin catalog redirect catalog_version must be greater than zero".to_string(),
        ));
    }
    if redirect.artifacts.is_empty() {
        return Err(AppError::Validation(
            "plugin catalog redirect must include at least one artifact".to_string(),
        ));
    }
    for artifact in &redirect.artifacts {
        validate_distribution_url("plugin catalog redirect artifact", &artifact.url)?;
        validate_distribution_url(
            "plugin catalog redirect artifact signature",
            &artifact.signature_url,
        )?;
        for mirror in &artifact.mirror_urls {
            validate_distribution_url("plugin catalog redirect artifact mirror", mirror)?;
        }
        for mirror in &artifact.signature_mirror_urls {
            validate_distribution_url("plugin catalog redirect signature mirror", mirror)?;
        }
    }
    Ok(())
}

fn validate_catalog_v3(catalog: &CatalogV3) -> AppResult<()> {
    if catalog.schema_version != CATALOG_V3_SCHEMA_VERSION {
        return Err(AppError::Validation(format!(
            "unsupported plugin catalog schema '{}'",
            catalog.schema_version
        )));
    }
    if catalog.catalog_version == 0 {
        return Err(AppError::Validation(
            "plugin catalog version must be greater than zero".to_string(),
        ));
    }

    let mut plugin_ids = HashSet::new();
    for plugin in &catalog.plugins {
        require_non_empty("plugin id", &plugin.id)?;
        require_non_empty("plugin name", &plugin.name)?;
        require_non_empty("plugin type", &plugin.plugin_type)?;
        require_non_empty("provider type", &plugin.provider_type)?;
        require_non_empty("publisher", &plugin.publisher)?;
        require_non_empty("docs_url", &plugin.docs_url)?;
        require_non_empty("source_repo", &plugin.source_repo)?;
        if !plugin_ids.insert(plugin.id.clone()) {
            return Err(AppError::Validation(format!(
                "duplicate plugin id '{}' in plugin catalog",
                plugin.id
            )));
        }
        let source_repo = GitHubRepo::parse(&plugin.source_repo)?;
        if plugin.required_signer.github_repository != source_repo.slug() {
            return Err(AppError::Validation(format!(
                "plugin '{}' signer repo '{}' does not match source repo '{}'",
                plugin.id,
                plugin.required_signer.github_repository,
                source_repo.slug()
            )));
        }
        validate_plugin_release_set(plugin)?;
    }

    let mut community_source_ids = HashSet::new();
    for source in &catalog.community_sources {
        require_non_empty("community source id", &source.id)?;
        require_non_empty(
            "community source github_repository",
            &source.github_repository,
        )?;
        if source.support_tier != PluginSupportTier::VerifiedCommunity {
            return Err(AppError::Validation(format!(
                "community source '{}' support tier must be verified_community",
                source.id
            )));
        }
        if !community_source_ids.insert(source.id.clone()) {
            return Err(AppError::Validation(format!(
                "duplicate community source id '{}' in plugin catalog",
                source.id
            )));
        }
        GitHubRepo::parse(&source.github_repository)?;
    }

    let mut rule_pack_ids = HashSet::new();
    for pack in &catalog.rule_packs {
        require_non_empty("rule pack id", &pack.id)?;
        require_non_empty("rule pack name", &pack.name)?;
        require_non_empty("rule pack author", &pack.author)?;
        if !rule_pack_ids.insert(pack.id.clone()) {
            return Err(AppError::Validation(format!(
                "duplicate rule pack id '{}' in plugin catalog",
                pack.id
            )));
        }
        let mut versions = HashSet::new();
        for release in &pack.releases {
            parse_version(&release.version)?;
            if !versions.insert(release.version.clone()) {
                return Err(AppError::Validation(format!(
                    "duplicate rule pack release version '{}' for '{}'",
                    release.version, pack.id
                )));
            }
            if let Some(min_scryer_version) = release.min_scryer_version.as_deref() {
                Version::parse(min_scryer_version.trim()).map_err(|error| {
                    AppError::Validation(format!(
                        "rule pack '{}' has invalid min_scryer_version '{}': {error}",
                        pack.id, min_scryer_version
                    ))
                })?;
            }
            if release.rule_pack_digests.is_empty() {
                return Err(AppError::Validation(format!(
                    "rule pack '{}' release '{}' must include rule_pack_digests",
                    pack.id, release.version
                )));
            }
            for digest in &release.rule_pack_digests {
                require_digest("rule_pack_digest", digest)?;
            }
            validate_distribution_artifacts(&pack.id, &release.version, &release.artifacts)?;
        }
    }

    Ok(())
}

fn validate_plugin_release_set(plugin: &CatalogV3PluginEntry) -> AppResult<()> {
    let mut versions = HashSet::new();
    for release in &plugin.releases {
        parse_version(&release.version)?;
        VersionReq::parse(&release.sdk_constraint).map_err(|e| {
            AppError::Validation(format!(
                "invalid sdk_constraint '{}' for plugin '{}': {e}",
                release.sdk_constraint, plugin.id
            ))
        })?;
        if let Some(min_scryer_version) = release.min_scryer_version.as_deref() {
            Version::parse(min_scryer_version.trim()).map_err(|error| {
                AppError::Validation(format!(
                    "plugin '{}' release '{}' has invalid min_scryer_version '{}': {error}",
                    plugin.id, release.version, min_scryer_version
                ))
            })?;
        }
        if !versions.insert(release.version.clone()) {
            return Err(AppError::Validation(format!(
                "duplicate release version '{}' for plugin '{}'",
                release.version, plugin.id
            )));
        }
        if release.artifacts.is_empty() {
            return Err(AppError::Validation(format!(
                "plugin '{}' release '{}' must include at least one artifact",
                plugin.id, release.version
            )));
        }
        let mut artifact_keys = HashSet::new();
        for artifact in &release.artifacts {
            require_non_empty("artifact runtime", &artifact.runtime)?;
            if artifact.runtime != CATALOG_V3_RUNTIME_WASIP1 {
                return Err(AppError::Validation(format!(
                    "plugin '{}' release '{}' has unsupported runtime '{}'",
                    plugin.id, release.version, artifact.runtime
                )));
            }
            let mut features = artifact
                .required_features
                .iter()
                .map(|feature| feature.trim().to_ascii_lowercase())
                .collect::<Vec<_>>();
            features.sort();
            features.dedup();
            for feature in &features {
                match feature.as_str() {
                    "simd128" | "relaxed-simd" => {}
                    _ => {
                        return Err(AppError::Validation(format!(
                            "plugin '{}' release '{}' uses unsupported required feature '{}'",
                            plugin.id, release.version, feature
                        )));
                    }
                }
            }
            if features.iter().any(|feature| feature == "relaxed-simd")
                && !features.iter().any(|feature| feature == "simd128")
            {
                return Err(AppError::Validation(format!(
                    "plugin '{}' release '{}' cannot require relaxed-simd without simd128",
                    plugin.id, release.version
                )));
            }
            let artifact_url = artifact.url.trim();
            let encoding = artifact_encoding_from_url(artifact_url).ok_or_else(|| {
                AppError::Validation(format!(
                    "plugin '{}' release '{}' artifact '{}' has unsupported encoding",
                    plugin.id, release.version, artifact.url
                ))
            })?;
            let artifact_key = format!("{encoding}|{}", features.join(","));
            if !artifact_keys.insert(artifact_key) {
                return Err(AppError::Validation(format!(
                    "plugin '{}' release '{}' has duplicate artifact variant/encoding rows",
                    plugin.id, release.version
                )));
            }
            validate_distribution_artifact(
                &format!("plugin '{}' release '{}'", plugin.id, release.version),
                artifact_url,
                &artifact.mirror_urls,
                &artifact.signature_url,
                &artifact.signature_mirror_urls,
                &artifact.digests,
            )?;
            if artifact.wasm_digests.is_empty() {
                return Err(AppError::Validation(format!(
                    "plugin '{}' release '{}' artifact '{}' must include wasm_digests",
                    plugin.id, release.version, artifact.url
                )));
            }
            for digest in &artifact.wasm_digests {
                require_digest("wasm_digest", digest)?;
            }
            if artifact.bytes == 0 {
                return Err(AppError::Validation(format!(
                    "plugin '{}' release '{}' artifact '{}' bytes must be greater than zero",
                    plugin.id, release.version, artifact.url
                )));
            }
        }
    }
    Ok(())
}

fn validate_distribution_artifacts(
    owner_id: &str,
    release_version: &str,
    artifacts: &[CatalogV3DistributionArtifact],
) -> AppResult<()> {
    if artifacts.is_empty() {
        return Err(AppError::Validation(format!(
            "'{owner_id}' release '{release_version}' must include at least one artifact"
        )));
    }
    for artifact in artifacts {
        validate_distribution_artifact(
            &format!("'{}' release '{}'", owner_id, release_version),
            &artifact.url,
            &artifact.mirror_urls,
            &artifact.signature_url,
            &artifact.signature_mirror_urls,
            &artifact.digests,
        )?;
    }
    Ok(())
}

fn validate_distribution_artifact(
    label: &str,
    url: &str,
    mirror_urls: &[String],
    signature_url: &str,
    signature_mirror_urls: &[String],
    digests: &[String],
) -> AppResult<()> {
    validate_distribution_url(label, url)?;
    validate_distribution_url(&format!("{label} signature"), signature_url)?;
    for mirror in mirror_urls {
        validate_distribution_url(&format!("{label} mirror"), mirror)?;
    }
    for mirror in signature_mirror_urls {
        validate_distribution_url(&format!("{label} signature mirror"), mirror)?;
    }
    if digests.is_empty() {
        return Err(AppError::Validation(format!(
            "{label} must include at least one digest"
        )));
    }
    for digest in digests {
        require_digest("artifact digest", digest)?;
    }
    Ok(())
}

fn validate_distribution_url(label: &str, url: &str) -> AppResult<()> {
    let parsed = Url::parse(url).map_err(|error| {
        AppError::Validation(format!("{label} URL '{url}' is invalid: {error}"))
    })?;
    match parsed.scheme() {
        "http" | "https" => Ok(()),
        scheme => Err(AppError::Validation(format!(
            "{label} URL '{url}' must use http or https, got '{scheme}'"
        ))),
    }
}

pub fn artifact_encoding_from_url(url: &str) -> Option<&'static str> {
    let normalized = url.trim().to_ascii_lowercase();
    if normalized.ends_with(".br") {
        Some("br")
    } else if normalized.ends_with(".zst") {
        Some("zst")
    } else {
        None
    }
}

#[cfg(feature = "runtime-plugin-trust")]
fn verify_signed_blob_blocking(
    raw: &[u8],
    bundle_raw: &[u8],
    required_signer: &RequiredSigner,
) -> AppResult<()> {
    let bundle_text = std::str::from_utf8(bundle_raw)
        .map_err(|e| AppError::Validation(format!("invalid Sigstore bundle UTF-8: {e}")))?;
    let bundle_text = normalize_sigstore_bundle(bundle_text)?;
    let rekor_keys = cached_rekor_verification_keys()?;

    let bundle = SignedArtifactBundle::new_verified(bundle_text.as_str(), rekor_keys.as_ref())
        .map_err(|e| {
            AppError::Validation(format!("Sigstore Rekor bundle verification failed: {e}"))
        })?;
    let cert_pem = normalize_bundle_cert(&bundle.cert)?;
    verify_rekor_hashedrekord_binding(
        raw,
        &bundle.base64_signature,
        &cert_pem,
        &bundle.rekor_bundle.payload.body,
    )?;
    <sigstore::cosign::Client as CosignCapabilities>::verify_blob(
        &cert_pem,
        &bundle.base64_signature,
        raw,
    )
    .map_err(|e| {
        AppError::Validation(format!("Sigstore blob signature verification failed: {e}"))
    })?;
    verify_fulcio_certificate_chain(&cert_pem, &bundle)?;
    verify_signer_identity(&cert_pem, required_signer)?;
    Ok(())
}

#[cfg(feature = "runtime-plugin-trust")]
fn verify_rekor_hashedrekord_binding(
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

#[cfg(feature = "runtime-plugin-trust")]
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

#[cfg(feature = "runtime-plugin-trust")]
fn verify_fulcio_certificate_chain(cert_pem: &str, bundle: &SignedArtifactBundle) -> AppResult<()> {
    let cert = Certificate::from_pem(cert_pem.as_bytes())
        .map_err(|e| AppError::Validation(format!("failed to parse Sigstore certificate: {e}")))?;
    let cert_der = cert
        .to_der()
        .map_err(|e| AppError::Validation(format!("failed to encode Sigstore certificate: {e}")))?;
    let cert_der = CertificateDer::from(cert_der.as_slice());
    let end_entity = EndEntityCert::try_from(&cert_der)
        .map_err(|e| AppError::Validation(format!("invalid Sigstore certificate: {e}")))?;
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
        .map_err(|e| {
            AppError::Validation(format!(
                "Sigstore Fulcio certificate chain verification failed: {e}"
            ))
        })?;

    Ok(())
}

#[cfg(feature = "runtime-plugin-trust")]
fn rekor_integrated_time(integrated_time: i64) -> AppResult<UnixTime> {
    let integrated_time = u64::try_from(integrated_time)
        .map_err(|_| AppError::Validation("Sigstore Rekor integrated time is negative".into()))?;
    Ok(UnixTime::since_unix_epoch(Duration::from_secs(
        integrated_time,
    )))
}

#[cfg(feature = "runtime-plugin-trust")]
fn cached_rekor_verification_keys() -> AppResult<Arc<RekorVerificationKeys>> {
    Ok(cached_sigstore_trust_material()?.rekor_keys.clone())
}

#[cfg(feature = "runtime-plugin-trust")]
fn cached_fulcio_trust_anchors() -> AppResult<Arc<FulcioTrustAnchors>> {
    Ok(cached_sigstore_trust_material()?.fulcio_anchors.clone())
}

#[cfg(feature = "runtime-plugin-trust")]
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

#[cfg(feature = "runtime-plugin-trust")]
fn load_sigstore_trust_material_blocking() -> AppResult<SigstoreTrustMaterial> {
    let trust_root = tokio::runtime::Handle::current()
        .block_on(SigstoreTrustRoot::new(None))
        .map_err(|e| AppError::Repository(format!("failed to load Sigstore trust root: {e}")))?;
    let rekor_keys = trust_root.rekor_keys().map_err(|e| {
        AppError::Repository(format!("failed to load Sigstore Rekor public keys: {e}"))
    })?;
    let fulcio_certs = trust_root.fulcio_certs().map_err(|e| {
        AppError::Repository(format!("failed to load Sigstore Fulcio certificates: {e}"))
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

#[cfg(feature = "runtime-plugin-trust")]
pub async fn prime_sigstore_trust_roots() -> AppResult<()> {
    tokio::task::spawn_blocking(cached_sigstore_trust_material)
        .await
        .map_err(|error| {
            AppError::Repository(format!("sigstore trust-root priming panicked: {error}"))
        })?
        .map(|_| ())
}

#[cfg(not(feature = "runtime-plugin-trust"))]
pub async fn prime_sigstore_trust_roots() -> AppResult<()> {
    Err(AppError::Validation(
        "plugin signature verification is not compiled into this target".to_string(),
    ))
}

#[cfg(feature = "runtime-plugin-trust")]
fn parse_rekor_verification_keys(
    keys: std::collections::BTreeMap<String, &[u8]>,
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

#[cfg(feature = "runtime-plugin-trust")]
fn verify_signer_identity(cert_pem: &str, required_signer: &RequiredSigner) -> AppResult<()> {
    let cert = Certificate::from_pem(cert_pem.as_bytes())
        .map_err(|e| AppError::Validation(format!("failed to parse Sigstore certificate: {e}")))?;
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

#[cfg(feature = "runtime-plugin-trust")]
fn normalize_sigstore_bundle(bundle_text: &str) -> AppResult<String> {
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
    .map_err(|e| AppError::Validation(format!("failed to normalize Sigstore bundle: {e}")))
}

#[cfg(feature = "runtime-plugin-trust")]
fn sigstore_bundle_value<'a>(
    value: &'a serde_json::Value,
    path: &[&str],
) -> Option<&'a serde_json::Value> {
    path.iter()
        .try_fold(value, |current, segment| current.get(*segment))
}

#[cfg(feature = "runtime-plugin-trust")]
fn sigstore_bundle_string_field<'a>(
    value: &'a serde_json::Value,
    path: &[&str],
    label: &str,
) -> AppResult<&'a str> {
    sigstore_bundle_value(value, path)
        .and_then(|value| value.as_str())
        .ok_or_else(|| AppError::Validation(format!("Sigstore bundle missing {label}")))
}

#[cfg(feature = "runtime-plugin-trust")]
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
    number.parse::<i64>().map_err(|e| {
        AppError::Validation(format!(
            "Sigstore bundle {label} is not a valid integer: {e}"
        ))
    })
}

#[cfg(feature = "runtime-plugin-trust")]
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

#[cfg(feature = "runtime-plugin-trust")]
fn normalize_bundle_cert(cert: &str) -> AppResult<String> {
    if cert.contains("-----BEGIN CERTIFICATE-----") {
        return Ok(cert.to_string());
    }
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(cert.as_bytes())
        .map_err(|e| AppError::Validation(format!("invalid base64 Sigstore certificate: {e}")))?;
    if let Ok(decoded_text) = String::from_utf8(decoded.clone())
        && decoded_text.contains("-----BEGIN CERTIFICATE-----")
    {
        return Ok(decoded_text);
    }
    Ok(pem_encode_certificate(&decoded))
}

#[cfg(feature = "runtime-plugin-trust")]
fn pem_encode_certificate(der: &[u8]) -> String {
    let base64 = base64::engine::general_purpose::STANDARD.encode(der);
    let mut pem = String::from("-----BEGIN CERTIFICATE-----\n");
    for chunk in base64.as_bytes().chunks(64) {
        pem.push_str(&String::from_utf8_lossy(chunk));
        pem.push('\n');
    }
    pem.push_str("-----END CERTIFICATE-----\n");
    pem
}

#[cfg(feature = "runtime-plugin-trust")]
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

#[cfg(feature = "runtime-plugin-trust")]
fn cert_subject_uri(cert: &Certificate) -> AppResult<Option<String>> {
    let san = cert
        .tbs_certificate()
        .get_extension::<SubjectAltName>()
        .map_err(|e| AppError::Validation(format!("failed to read certificate SAN: {e}")))?
        .map(|(_, san)| san);
    let Some(san) = san else {
        return Ok(None);
    };
    Ok(san.0.iter().find_map(|name| match name {
        GeneralName::UniformResourceIdentifier(uri) => Some(uri.to_string()),
        _ => None,
    }))
}

fn require_non_empty(label: &str, value: &str) -> AppResult<()> {
    if value.trim().is_empty() {
        return Err(AppError::Validation(format!("{label} is required")));
    }
    Ok(())
}

fn parse_version(version: &str) -> AppResult<Version> {
    Version::parse(version.trim_start_matches('v')).map_err(|e| {
        AppError::Validation(format!("invalid plugin release version '{version}': {e}"))
    })
}

fn require_digest(label: &str, digest: &str) -> AppResult<()> {
    parse_digest_string(digest).map(|_| ()).map_err(|_| {
        AppError::Validation(format!(
            "{label} must use a supported <algorithm>:<hex> digest"
        ))
    })
}

fn normalize_hex_digest(input: &str) -> AppResult<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(AppError::Validation("digest value is missing".to_string()));
    }
    if !trimmed.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(AppError::Validation(format!(
            "digest value '{trimmed}' must be hexadecimal"
        )));
    }
    Ok(trimmed.to_ascii_lowercase())
}

#[cfg(test)]
fn require_release_asset_url(label: &str, url: &str, repo: &GitHubRepo) -> AppResult<()> {
    if !url.starts_with(&repo.release_asset_prefix()) {
        return Err(AppError::Validation(format!(
            "{label} URL must point to GitHub Releases for '{}'",
            repo.slug()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_digest_string_splits_blake3_digest() {
        let (algorithm, digest) =
            parse_digest_string("blake3:0123456789abcdef").expect("digest should parse");
        assert_eq!(algorithm, "blake3");
        assert_eq!(digest, "0123456789abcdef");
    }

    #[test]
    fn redirect_bundle_url_uses_json_companion_contract() {
        assert_eq!(
            redirect_bundle_url_for("https://cdn.scryer.media/catalog/v3/catalog-v3.redirect.json"),
            "https://cdn.scryer.media/catalog/v3/catalog-v3.redirect.bundle.json"
        );
    }

    #[test]
    fn parse_digest_string_rejects_malformed_values() {
        assert!(parse_digest_string("blake3").is_err());
        assert!(parse_digest_string("blake3:not-hex").is_err());
        assert!(parse_digest_string(":abcd").is_err());
    }

    #[test]
    fn verify_split_digest_accepts_matching_blake3_hex() {
        let bytes = b"hello from scryer";
        let digest = blake3::hash(bytes).to_hex().to_string();
        verify_split_digest("plugin wasm", "blake3", &digest, bytes)
            .expect("matching digest should verify");
    }

    #[test]
    fn verify_split_digest_rejects_unknown_algorithms() {
        let err = verify_split_digest("plugin wasm", "sha256", "abcd", b"bytes").unwrap_err();
        assert!(err.to_string().contains("unsupported digest algorithm"));
    }

    #[test]
    fn github_status_fail_open_for_malformed_response() {
        assert!(github_outage_status_from_summary(b"not json").is_none());
    }

    #[test]
    fn github_status_blocks_only_relevant_confirmed_outage() {
        let raw = br#"{
            "status": { "indicator": "major" },
            "components": [
                { "name": "API Requests", "status": "degraded_performance" },
                { "name": "Pages", "status": "operational" }
            ]
        }"#;
        let status = github_outage_status_from_summary(raw).expect("well-formed status");
        assert!(!status.github_available);
        assert!(status.blocked_actions.contains(&"install".to_string()));
    }

    #[test]
    fn child_catalog_rejects_duplicate_release_versions() {
        let raw = br#"{
            "schema_version": "scryer.plugin.child_catalog.v2",
            "id": "email",
            "name": "Email",
            "description": "Email notifications",
            "plugin_type": "notification",
            "provider_type": "email",
            "publisher": "scryer",
            "support_tier": "official",
            "docs_url": "https://github.com/scryer-media/scryer-plugins",
            "source_repo": "https://github.com/scryer-media/scryer-plugin-email",
            "releases": [
                {
                    "version": "0.1.0",
                    "sdk_constraint": "^0.13",
                    "artifact_manifest_url": "https://github.com/scryer-media/scryer-plugin-email/releases/download/v0.1.0/plugin.manifest.json"
                },
                {
                    "version": "0.1.0",
                    "sdk_constraint": "^0.13",
                    "artifact_manifest_url": "https://github.com/scryer-media/scryer-plugin-email/releases/download/v0.1.0/plugin.manifest.json"
                }
            ]
        }"#;
        let err = parse_and_validate_child_catalog(raw, None).unwrap_err();
        assert!(err.to_string().contains("duplicate release version"));
    }

    #[test]
    fn catalog_v3_rejects_invalid_plugin_min_scryer_version() {
        let raw = br#"{
            "schema_version": "scryer.plugin.catalog.v3",
            "catalog_version": 1,
            "plugins": [
                {
                    "id": "alpha",
                    "name": "Alpha",
                    "description": "Alpha plugin",
                    "plugin_type": "indexer",
                    "provider_type": "alpha",
                    "publisher": "scryer",
                    "support_tier": "official",
                    "status": "active",
                    "docs_url": "https://github.com/scryer-media/alpha",
                    "source_repo": "https://github.com/scryer-media/alpha",
                    "required_signer": {
                        "github_repository": "scryer-media/alpha"
                    },
                    "releases": [
                        {
                            "version": "1.0.0",
                            "sdk_constraint": ">=1.5.0, <1.6.0",
                            "min_scryer_version": "not-semver",
                            "artifacts": [
                                {
                                    "runtime": "wasm32-wasip1",
                                    "required_features": [],
                                    "url": "https://github.com/scryer-media/alpha/releases/download/v1.0.0/alpha.wasm.zst",
                                    "mirror_urls": [],
                                    "signature_url": "https://github.com/scryer-media/alpha/releases/download/v1.0.0/alpha.wasm.zst.bundle.json",
                                    "signature_mirror_urls": [],
                                    "digests": [
                                        "sha256:1111111111111111111111111111111111111111111111111111111111111111"
                                    ],
                                    "wasm_digests": [
                                        "sha256:2222222222222222222222222222222222222222222222222222222222222222"
                                    ],
                                    "bytes": 1234
                                }
                            ]
                        }
                    ]
                }
            ],
            "rule_packs": []
        }"#;

        let err = parse_and_validate_catalog_v3(raw).unwrap_err();

        assert!(
            err.to_string().contains("invalid min_scryer_version"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn catalog_v3_accepts_verified_community_sources() {
        let raw = br#"{
            "schema_version": "scryer.plugin.catalog.v3",
            "catalog_version": 1,
            "plugins": [],
            "rule_packs": [],
            "community_sources": [
                {
                    "id": "community-alpha",
                    "github_repository": "scryer-community/community-alpha",
                    "support_tier": "verified_community"
                }
            ]
        }"#;

        let catalog = parse_and_validate_catalog_v3(raw).expect("catalog should parse");

        assert_eq!(catalog.community_sources.len(), 1);
        assert_eq!(catalog.community_sources[0].id, "community-alpha");
        assert_eq!(
            catalog.community_sources[0].support_tier,
            PluginSupportTier::VerifiedCommunity
        );
    }

    #[test]
    fn catalog_v3_rejects_non_verified_community_sources() {
        let raw = br#"{
            "schema_version": "scryer.plugin.catalog.v3",
            "catalog_version": 1,
            "plugins": [],
            "rule_packs": [],
            "community_sources": [
                {
                    "id": "community-alpha",
                    "github_repository": "scryer-community/community-alpha",
                    "support_tier": "official"
                }
            ]
        }"#;

        let err = parse_and_validate_catalog_v3(raw).unwrap_err();

        assert!(
            err.to_string()
                .contains("support tier must be verified_community"),
            "unexpected error: {err}"
        );
    }

    #[cfg(feature = "runtime-plugin-trust")]
    #[test]
    fn normalize_bundle_cert_wraps_base64_der_as_pem() {
        let der_base64 =
            base64::engine::general_purpose::STANDARD.encode([0x30, 0x03, 0x02, 0x01, 0x05]);
        let pem = normalize_bundle_cert(&der_base64).expect("DER certificate should normalize");
        assert!(pem.starts_with("-----BEGIN CERTIFICATE-----\n"));
        assert!(pem.contains(&der_base64));
        assert!(pem.ends_with("-----END CERTIFICATE-----\n"));
    }

    #[cfg(feature = "runtime-plugin-trust")]
    fn rekor_hashedrekord_body(raw: &[u8], signature: &str, cert_pem: &str) -> String {
        let digest = Sha256::digest(raw);
        let digest = digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        base64::engine::general_purpose::STANDARD.encode(
            serde_json::to_vec(&serde_json::json!({
                "kind": "hashedrekord",
                "apiVersion": "0.0.1",
                "spec": {
                    "data": {"hash": {"algorithm": "sha256", "value": digest}},
                    "signature": {
                        "content": signature,
                        "publicKey": {"content": cert_pem}
                    }
                }
            }))
            .expect("serialize Rekor body"),
        )
    }

    #[cfg(feature = "runtime-plugin-trust")]
    #[tokio::test]
    async fn rekor_hashedrekord_binding_accepts_matching_artifact_signature_and_certificate() {
        let trust_root = SigstoreTrustRoot::new(None)
            .await
            .expect("embedded Sigstore trust root should load");
        let fulcio_certs = trust_root
            .fulcio_certs()
            .expect("trust root should provide Fulcio certificates");
        let certificate = fulcio_certs
            .first()
            .expect("at least one Fulcio certificate");
        let cert_pem = pem_encode_certificate(certificate);
        let raw = b"plugin artifact";
        let signature = base64::engine::general_purpose::STANDARD.encode(b"signature");
        let body = rekor_hashedrekord_body(raw, &signature, &cert_pem);

        verify_rekor_hashedrekord_binding(raw, &signature, &cert_pem, &body)
            .expect("matching Rekor body should bind to the bundle");
    }

    #[cfg(feature = "runtime-plugin-trust")]
    #[test]
    fn rekor_hashedrekord_binding_rejects_signature_transplant() {
        let raw = b"plugin artifact";
        let signature = base64::engine::general_purpose::STANDARD.encode(b"signature");
        let body = rekor_hashedrekord_body(
            raw,
            &base64::engine::general_purpose::STANDARD.encode(b"different signature"),
            "-----BEGIN CERTIFICATE-----\nplaceholder\n-----END CERTIFICATE-----\n",
        );

        let error = verify_rekor_hashedrekord_binding(
            raw,
            &signature,
            "-----BEGIN CERTIFICATE-----\nplaceholder\n-----END CERTIFICATE-----\n",
            &body,
        )
        .expect_err("signature transplant must fail before certificate parsing");
        assert!(error.to_string().contains("signature does not match"));
    }

    #[cfg(feature = "runtime-plugin-trust")]
    #[tokio::test]
    async fn rekor_hashedrekord_binding_rejects_altered_artifact_certificate_and_body() {
        let trust_root = SigstoreTrustRoot::new(None)
            .await
            .expect("embedded Sigstore trust root should load");
        let cert_pems = trust_root
            .fulcio_certs()
            .expect("trust root should provide Fulcio certificates")
            .iter()
            .map(|certificate| pem_encode_certificate(certificate.as_ref()))
            .collect::<Vec<_>>();
        let cert_pem = cert_pems.first().expect("at least one Fulcio certificate");
        let raw = b"plugin artifact";
        let signature = base64::engine::general_purpose::STANDARD.encode(b"signature");
        let body = rekor_hashedrekord_body(raw, &signature, cert_pem);

        let digest_error = verify_rekor_hashedrekord_binding(
            b"altered plugin artifact",
            &signature,
            cert_pem,
            &body,
        )
        .expect_err("Rekor digest must bind to the artifact");
        assert!(digest_error.to_string().contains("digest"));

        let malformed_error =
            verify_rekor_hashedrekord_binding(raw, &signature, cert_pem, "not-base64")
                .expect_err("malformed Rekor body must fail");
        assert!(malformed_error.to_string().contains("encoding"));

        let unsupported_body = base64::engine::general_purpose::STANDARD.encode(
            serde_json::to_vec(&serde_json::json!({
                "kind": "rekord",
                "apiVersion": "0.0.1",
                "spec": {}
            }))
            .expect("serialize unsupported Rekor body"),
        );
        let unsupported_error =
            verify_rekor_hashedrekord_binding(raw, &signature, cert_pem, &unsupported_body)
                .expect_err("unsupported Rekor body must fail");
        assert!(
            unsupported_error
                .to_string()
                .contains("unsupported Rekor body")
        );

        let alternate_cert = cert_pems
            .iter()
            .find(|candidate| candidate.as_bytes() != cert_pem.as_bytes())
            .expect("embedded trust root should include distinct Fulcio certificates");
        let certificate_body = rekor_hashedrekord_body(raw, &signature, alternate_cert);
        let certificate_error =
            verify_rekor_hashedrekord_binding(raw, &signature, cert_pem, &certificate_body)
                .expect_err("Rekor certificate must bind to the bundle certificate");
        assert!(certificate_error.to_string().contains("certificate"));
    }

    #[cfg(feature = "runtime-plugin-trust")]
    #[test]
    fn normalize_sigstore_bundle_rewrites_v03_payloads() {
        let der_base64 = base64::engine::general_purpose::STANDARD.encode([1_u8, 2, 3, 4]);
        let key_id_base64 = base64::engine::general_purpose::STANDARD.encode([0_u8, 1, 2, 3]);
        let bundle = serde_json::json!({
            "mediaType": "application/vnd.dev.sigstore.bundle.v0.3+json",
            "messageSignature": {
                "signature": "sig=="
            },
            "verificationMaterial": {
                "certificate": {
                    "rawBytes": der_base64
                },
                "tlogEntries": [
                    {
                        "logIndex": "12",
                        "logId": {
                            "keyId": key_id_base64
                        },
                        "integratedTime": "34",
                        "inclusionPromise": {
                            "signedEntryTimestamp": "set=="
                        },
                        "canonicalizedBody": "body=="
                    }
                ]
            }
        });

        let normalized =
            normalize_sigstore_bundle(&bundle.to_string()).expect("bundle should normalize");
        let parsed: SignedArtifactBundle =
            serde_json::from_str(&normalized).expect("bundle should parse in legacy shape");
        assert_eq!(parsed.base64_signature, "sig==");
        assert_eq!(
            parsed.cert.lines().next(),
            Some("-----BEGIN CERTIFICATE-----")
        );
        assert_eq!(parsed.rekor_bundle.payload.log_index, 12);
        assert_eq!(parsed.rekor_bundle.payload.integrated_time, 34);
        assert_eq!(parsed.rekor_bundle.payload.log_id, "00010203");
        assert_eq!(parsed.rekor_bundle.payload.body, "body==");
    }

    #[cfg(feature = "runtime-plugin-trust")]
    #[tokio::test]
    async fn sigstore_trust_root_rekor_keys_parse_as_der() {
        let trust_root = SigstoreTrustRoot::new(None)
            .await
            .expect("embedded Sigstore trust root should load");
        let rekor_keys = trust_root
            .rekor_keys()
            .expect("Sigstore trust root should provide Rekor keys");
        assert!(!rekor_keys.is_empty(), "expected at least one Rekor key");
        let parsed = parse_rekor_verification_keys(rekor_keys)
            .expect("embedded Rekor keys should parse as DER verification keys");
        assert!(!parsed.is_empty(), "expected at least one parsed Rekor key");
    }
}

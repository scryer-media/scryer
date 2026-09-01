use std::collections::HashSet;

#[cfg(feature = "runtime-plugin-trust")]
use std::io::Read;

#[cfg(all(test, feature = "runtime-plugin-trust"))]
use base64::Engine;
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
#[cfg(all(test, feature = "runtime-plugin-trust"))]
use sha2::{Digest, Sha256};
#[cfg(all(test, feature = "runtime-plugin-trust"))]
use sigstore::trust::{TrustRoot, sigstore::SigstoreTrustRoot};
use url::Url;

use crate::{AppError, AppResult};
use scryer_domain::PluginSupportTier;

#[cfg(all(test, feature = "runtime-plugin-trust"))]
use super::trust::{
    normalize_bundle_cert, pem_encode_certificate, verify_rekor_hashedrekord_binding,
};

#[cfg(test)]
const CHILD_CATALOG_SCHEMA_VERSION: &str = "scryer.plugin.child_catalog.v2";
pub const PLUGIN_CATALOG_JSON_OUTPUT_LIMIT: u64 = 16 * 1024 * 1024;
pub const PLUGIN_CATALOG_REDIRECT_OUTPUT_LIMIT: u64 = 2 * 1024 * 1024;
pub const PLUGIN_SIGNATURE_BUNDLE_OUTPUT_LIMIT: u64 = 2 * 1024 * 1024;
pub const RULE_PACK_MANIFEST_FALLBACK_OUTPUT_LIMIT: u64 = 32 * 1024 * 1024;
pub const MANUAL_PLUGIN_WASM_OUTPUT_LIMIT: u64 = 128 * 1024 * 1024;
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

// Forward tolerance, deliberately.
//
// Every catalog-v3 wire struct below used to carry `#[serde(deny_unknown_fields)]`,
// which made any field a future catalog gains a hard parse error for the whole
// document — one plugin's new field and this Scryer's plugin catalog goes dark.
// The 0.18.12 `max_scryer_version` addition already had to be worked around by
// publishing a second, field-stripped projection of the whole catalog for older
// clients. Unknown fields are now ignored instead. Integrity does not depend on
// this: the catalog blob is Sigstore-verified against a required signer and
// every artifact is digest-checked before it is used.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RequiredSigner {
    pub github_repository: String,
    #[serde(default)]
    pub github_workflow: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github_ref: Option<String>,
}

pub const CATALOG_V3_SCHEMA_VERSION: &str = "scryer.plugin.catalog.v3";
pub const CATALOG_V3_REDIRECT_SCHEMA_VERSION: &str = "scryer.plugin.catalog.v3.redirect";
pub const CATALOG_V3_REDIRECT_BUNDLE_SUFFIX: &str = ".bundle.json";
pub const CATALOG_V3_RUNTIME_WASIP1: &str = "wasm32-wasip1";
pub const CATALOG_V3_RUNTIME_WASIP2: &str = "wasm32-wasip2";

/// Ordering used when a release ships the same plugin for several WASI targets:
/// a host that satisfies more than one takes the newest it can run.
///
/// Unknown targets sort above every known one on purpose. A target this build
/// has never heard of can only be present in `runtime_capabilities` because the
/// host put it there, which means the host is newer than this table, and the
/// newest thing the host declares is the thing it wants.
const CATALOG_V3_RUNTIME_RANK: &[&str] = &[CATALOG_V3_RUNTIME_WASIP1, CATALOG_V3_RUNTIME_WASIP2];

fn catalog_v3_runtime_rank(runtime: &str) -> usize {
    CATALOG_V3_RUNTIME_RANK
        .iter()
        .position(|known| *known == runtime)
        .unwrap_or(CATALOG_V3_RUNTIME_RANK.len())
}

/// Whether this host can run `artifact`.
///
/// Both halves of an artifact's requirement are opaque capability tokens
/// matched against the set the host declares (WASI target plus wasm features,
/// one namespace — see `scryer_plugins::runtime_features`). Nothing here
/// enumerates the tokens it knows about, which is exactly why a wasip3 artifact
/// row added to the catalog years from now is a skipped artifact on this build
/// rather than a rejected catalog.
pub fn catalog_v3_artifact_is_runnable(
    artifact: &CatalogV3PluginArtifact,
    runtime_capabilities: &HashSet<String>,
) -> bool {
    runtime_capabilities.contains(&normalize_capability_token(&artifact.runtime))
        && artifact
            .required_features
            .iter()
            .all(|feature| runtime_capabilities.contains(&normalize_capability_token(feature)))
        && artifact_encoding_from_url(&artifact.url).is_some()
}

/// How specific a runnable artifact is, for picking between the variants of one
/// release. Higher is better: newest WASI target first, then the most
/// specialised feature set (a `simd128` build over the baseline build).
pub fn catalog_v3_artifact_preference(artifact: &CatalogV3PluginArtifact) -> (usize, usize) {
    (
        catalog_v3_runtime_rank(artifact.runtime.trim()),
        artifact.required_features.len(),
    )
}

fn normalize_capability_token(token: &str) -> String {
    token.trim().to_ascii_lowercase()
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginLifecycleStatus {
    Beta,
    Active,
    Deprecated,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CatalogV3Redirect {
    pub schema_version: String,
    pub catalog_version: u64,
    pub artifacts: Vec<CatalogV3RedirectArtifact>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CatalogV3RedirectArtifact {
    pub url: String,
    #[serde(default)]
    pub mirror_urls: Vec<String>,
    pub signature_url: String,
    #[serde(default)]
    pub signature_mirror_urls: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
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
pub struct CatalogV3CommunitySource {
    pub id: String,
    pub github_repository: String,
    pub support_tier: PluginSupportTier,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
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
pub struct CatalogV3PluginRelease {
    pub version: String,
    pub sdk_constraint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_scryer_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_scryer_version: Option<String>,
    pub artifacts: Vec<CatalogV3PluginArtifact>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
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
pub struct CatalogV3RulePackEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub author: String,
    pub releases: Vec<CatalogV3RulePackRelease>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
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
pub use super::trust::{prime_sigstore_trust_roots, verify_signed_blob};

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

#[cfg(not(feature = "runtime-plugin-trust"))]
pub async fn prime_sigstore_trust_roots() -> AppResult<()> {
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
        let min_scryer_version = release
            .min_scryer_version
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(Version::parse)
            .transpose()
            .map_err(|error| {
                AppError::Validation(format!(
                    "plugin '{}' release '{}' has invalid min_scryer_version: {error}",
                    plugin.id, release.version
                ))
            })?;
        let max_scryer_version = release
            .max_scryer_version
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(Version::parse)
            .transpose()
            .map_err(|error| {
                AppError::Validation(format!(
                    "plugin '{}' release '{}' has invalid max_scryer_version: {error}",
                    plugin.id, release.version
                ))
            })?;
        if min_scryer_version
            .zip(max_scryer_version)
            .is_some_and(|(min, max)| min > max)
        {
            return Err(AppError::Validation(format!(
                "plugin '{}' release '{}' has min_scryer_version greater than max_scryer_version",
                plugin.id, release.version
            )));
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
        // Capability tolerance, deliberately.
        //
        // An artifact's `runtime` and `required_features` say what a host needs
        // in order to run it. They are not a closed vocabulary this build gets
        // to police: when the fleet moves to a target or a wasm feature newer
        // than this binary, the honest answer is "not for me", not "this
        // catalog is corrupt". Rejecting the document here is what took every
        // Scryer at or below 0.18.21 off the catalog the moment one wasip2
        // artifact row was published. Unknown tokens now make a single artifact
        // unrunnable; `select_catalog_release_and_artifact` then falls back to
        // the newest release this host *can* run. Everything below stays an
        // error, because it is integrity, not capability.
        let mut artifact_keys = HashSet::new();
        for artifact in &release.artifacts {
            require_non_empty("artifact runtime", &artifact.runtime)?;
            let mut features = artifact
                .required_features
                .iter()
                .map(|feature| feature.trim().to_ascii_lowercase())
                .collect::<Vec<_>>();
            features.sort();
            features.dedup();
            let artifact_url = artifact.url.trim();
            let encoding = artifact_encoding_from_url(artifact_url).unwrap_or("unknown");
            let artifact_key = format!(
                "{}|{encoding}|{}",
                artifact.runtime.trim().to_ascii_lowercase(),
                features.join(",")
            );
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

    fn test_artifact(runtime: &str, required_features: &[&str]) -> CatalogV3PluginArtifact {
        CatalogV3PluginArtifact {
            runtime: runtime.to_string(),
            required_features: required_features
                .iter()
                .map(|feature| (*feature).to_string())
                .collect(),
            url: "https://cdn.scryer.media/plugins-v3/alpha/plugin.wasm.zst".to_string(),
            mirror_urls: Vec::new(),
            signature_url: "https://cdn.scryer.media/plugins-v3/alpha/plugin.wasm.zst.bundle.zst"
                .to_string(),
            signature_mirror_urls: Vec::new(),
            digests: vec!["blake3:ab".to_string()],
            wasm_digests: vec!["blake3:cd".to_string()],
            bytes: 1,
        }
    }

    fn capabilities(tokens: &[&str]) -> HashSet<String> {
        tokens
            .iter()
            .map(|token| (*token).to_string())
            .collect::<HashSet<_>>()
    }

    #[test]
    fn artifact_is_runnable_matches_target_and_features_as_capability_tokens() {
        let host = capabilities(&["wasm32-wasip2", "simd128"]);

        assert!(catalog_v3_artifact_is_runnable(
            &test_artifact("wasm32-wasip2", &[]),
            &host
        ));
        assert!(catalog_v3_artifact_is_runnable(
            &test_artifact("wasm32-wasip2", &["simd128"]),
            &host
        ));
        assert!(
            !catalog_v3_artifact_is_runnable(&test_artifact("wasm32-wasip1", &[]), &host),
            "a target the host does not declare must not be selected"
        );
        assert!(
            !catalog_v3_artifact_is_runnable(&test_artifact("wasm32-wasip3", &[]), &host),
            "a target this build has never heard of must not be selected"
        );
        assert!(
            !catalog_v3_artifact_is_runnable(
                &test_artifact("wasm32-wasip2", &["simd128", "relaxed-simd"]),
                &host
            ),
            "a feature the host does not declare must not be selected"
        );
    }

    #[test]
    fn artifact_preference_prefers_newest_target_then_most_specific_features() {
        let p3 = catalog_v3_artifact_preference(&test_artifact("wasm32-wasip3", &[]));
        let p2_simd = catalog_v3_artifact_preference(&test_artifact("wasm32-wasip2", &["simd128"]));
        let p2 = catalog_v3_artifact_preference(&test_artifact("wasm32-wasip2", &[]));
        let p1 = catalog_v3_artifact_preference(&test_artifact("wasm32-wasip1", &[]));

        assert!(p3 > p2_simd);
        assert!(p2_simd > p2);
        assert!(p2 > p1);
    }

    #[test]
    fn catalog_v3_tolerates_future_runtimes_features_and_fields() {
        let raw = br#"{
            "schema_version": "scryer.plugin.catalog.v3",
            "catalog_version": 9,
            "provenance": {"cut_by": "a future xtask"},
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
                    "required_signer": { "github_repository": "scryer-media/alpha" },
                    "releases": [
                        {
                            "version": "2.0.0",
                            "sdk_constraint": ">=1.5.0, <1.6.0",
                            "deprecation_notice": "a field from the future",
                            "artifacts": [
                                {
                                    "runtime": "wasm32-wasip3",
                                    "required_features": ["memory64"],
                                    "url": "https://github.com/scryer-media/alpha/releases/download/v2.0.0/alpha.wasm.zst",
                                    "signature_url": "https://github.com/scryer-media/alpha/releases/download/v2.0.0/alpha.wasm.zst.bundle.json",
                                    "digests": ["blake3:11"],
                                    "wasm_digests": ["blake3:22"],
                                    "bytes": 1234,
                                    "provenance_url": "https://example.test/slsa"
                                }
                            ]
                        }
                    ]
                }
            ],
            "rule_packs": []
        }"#;

        let catalog = parse_and_validate_catalog_v3(raw)
            .expect("an unknown runtime, feature token, and field must not fail the catalog");
        let artifact = &catalog.plugins[0].releases[0].artifacts[0];
        assert!(!catalog_v3_artifact_is_runnable(
            artifact,
            &capabilities(&["wasm32-wasip2", "simd128"])
        ));
    }

    #[test]
    fn catalog_v3_still_rejects_integrity_failures() {
        let base = r#"{
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
                    "required_signer": { "github_repository": "scryer-media/alpha" },
                    "releases": [
                        {
                            "version": "1.0.0",
                            "sdk_constraint": ">=1.5.0, <1.6.0",
                            "artifacts": [
                                {
                                    "runtime": "wasm32-wasip2",
                                    "url": "https://github.com/scryer-media/alpha/releases/download/v1.0.0/alpha.wasm.zst",
                                    "signature_url": "https://github.com/scryer-media/alpha/releases/download/v1.0.0/alpha.wasm.zst.bundle.json",
                                    "digests": ["blake3:11"],
                                    "wasm_digests": ["blake3:22"],
                                    "bytes": 1234
                                }
                            ]
                        }
                    ]
                }
            ],
            "rule_packs": []
        }"#;
        parse_and_validate_catalog_v3(base.as_bytes()).expect("baseline fixture should parse");

        let no_digests = base.replace(r#""digests": ["blake3:11"],"#, r#""digests": [],"#);
        assert!(parse_and_validate_catalog_v3(no_digests.as_bytes()).is_err());

        let bad_scheme = base.replace(
            "https://github.com/scryer-media/alpha/releases/download/v1.0.0/alpha.wasm.zst\"",
            "file:///etc/passwd\"",
        );
        assert!(parse_and_validate_catalog_v3(bad_scheme.as_bytes()).is_err());

        let zero_bytes = base.replace(r#""bytes": 1234"#, r#""bytes": 0"#);
        assert!(parse_and_validate_catalog_v3(zero_bytes.as_bytes()).is_err());

        let duplicate_variant = base.replace(
            r#""bytes": 1234
                                }"#,
            r#""bytes": 1234
                                },
                                {
                                    "runtime": "wasm32-wasip2",
                                    "url": "https://github.com/scryer-media/alpha/releases/download/v1.0.0/alpha.wasm.zst",
                                    "signature_url": "https://github.com/scryer-media/alpha/releases/download/v1.0.0/alpha.wasm.zst.bundle.json",
                                    "digests": ["blake3:11"],
                                    "wasm_digests": ["blake3:22"],
                                    "bytes": 1234
                                }"#,
        );
        let error = parse_and_validate_catalog_v3(duplicate_variant.as_bytes())
            .expect_err("duplicate artifact rows are a publisher bug, not a capability gap");
        assert!(error.to_string().contains("duplicate artifact variant"));
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

        let inverted_range = String::from_utf8(raw.to_vec())
            .expect("fixture is utf-8")
            .replace(
                "\"min_scryer_version\": \"not-semver\"",
                "\"min_scryer_version\": \"0.18.12\", \"max_scryer_version\": \"0.18.11\"",
            );
        let err = parse_and_validate_catalog_v3(inverted_range.as_bytes()).unwrap_err();
        assert!(
            err.to_string()
                .contains("min_scryer_version greater than max_scryer_version"),
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
        scryer_outbound_http::install_default_rustls_provider();
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
        scryer_outbound_http::install_default_rustls_provider();
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
}

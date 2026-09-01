#[derive(Clone, Debug)]
struct CatalogPluginResolution {
    catalog_entry: CatalogV3PluginEntry,
    release: CatalogV3PluginRelease,
    artifact: CatalogV3PluginArtifact,
    source_kind: PluginSourceKind,
    effective_support_tier: PluginSupportTier,
    github_repo: GitHubRepo,
}
#[derive(Clone, Debug, Default)]
struct CatalogPluginSourceResolution {
    central: Option<CatalogV3>,
    resolved: Vec<CatalogPluginResolution>,
}
struct PreparedCatalogPluginInstall {
    plugin_id: String,
    expected_plugin_type: String,
    expected_provider_type: String,
    release: DownloadedPluginReleaseContract,
    scryer_constraint: Option<String>,
    source_kind: PluginSourceKind,
    support_tier: PluginSupportTier,
    persisted_wasm_bytes: Vec<u8>,
    runtime_wasm_bytes: Vec<u8>,
    runtime_first_party: bool,
    wasm_encoding: PluginWasmEncoding,
    wasm_digest_algo: String,
    source_url: String,
    publisher: String,
    docs_url: String,
    source_repo: String,
    manifest_url: String,
    wasm_digest: String,
    artifact_digest: String,
    description: String,
}
struct ValidatedCatalogPluginInstall {
    descriptor: PluginDescriptor,
    sdk_constraint: String,
    scryer_constraint: Option<String>,
    source_kind: PluginSourceKind,
    support_tier: PluginSupportTier,
    persisted_wasm_bytes: Vec<u8>,
    runtime_wasm_bytes: Vec<u8>,
    runtime_first_party: bool,
    wasm_encoding: PluginWasmEncoding,
    wasm_digest_algo: String,
    source_url: String,
    publisher: String,
    docs_url: String,
    source_repo: String,
    manifest_url: String,
    wasm_digest: String,
    artifact_digest: String,
    description: String,
}
/// Community rule pack entry from the official catalog.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RulePackRegistryEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub author: String,
    pub version: String,
    pub source_url: String,
    #[serde(default)]
    pub min_scryer_version: Option<String>,
}
impl RulePackRegistryEntry {
    fn from_release(
        value: &CatalogV3RulePackEntry,
        release: &CatalogV3RulePackRelease,
        artifact: &CatalogV3DistributionArtifact,
    ) -> Self {
        Self {
            id: value.id.clone(),
            name: value.name.clone(),
            description: value.description.clone(),
            author: value.author.clone(),
            version: release.version.clone(),
            source_url: artifact.url.clone(),
            min_scryer_version: release.min_scryer_version.clone(),
        }
    }
}
/// Full rule pack JSON fetched from a URL.
#[derive(Clone, Debug, Deserialize)]
struct RulePackManifest {
    #[expect(dead_code)]
    schema_version: u32,
    #[expect(dead_code)]
    id: String,
    rules: Vec<RulePackRule>,
}
struct FetchedCatalogArtifact {
    persisted_wasm_bytes: Vec<u8>,
    wasm_bytes: Vec<u8>,
    artifact_url: String,
    artifact_digest: String,
    wasm_encoding: PluginWasmEncoding,
}
struct FetchedSignedBlob {
    raw: Vec<u8>,
    actual_url: String,
    signature_bundle: Vec<u8>,
}
fn parse_catalog_release_version(
    plugin_id: &str,
    release: &CatalogV3PluginRelease,
) -> Option<semver::Version> {
    semver::Version::parse(release.version.trim_start_matches('v')).map_or_else(
        |error| {
            warn!(
                plugin_id,
                version = release.version.as_str(),
                error = %error,
                "skipping plugin release with invalid version"
            );
            None
        },
        Some,
    )
}
fn parse_catalog_release_sdk_req(
    plugin_id: &str,
    release: &CatalogV3PluginRelease,
) -> Option<semver::VersionReq> {
    let constraint = effective_host_sdk_constraint(None, &release.sdk_constraint);
    semver::VersionReq::parse(constraint.trim()).map_or_else(
        |error| {
            warn!(
                plugin_id,
                version = release.version.as_str(),
                sdk_constraint = constraint.as_str(),
                error = %error,
                "skipping plugin release with invalid sdk_constraint"
            );
            None
        },
        Some,
    )
}
fn catalog_release_is_sdk_compatible(plugin_id: &str, release: &CatalogV3PluginRelease) -> bool {
    let Some(sdk_req) = parse_catalog_release_sdk_req(plugin_id, release) else {
        return false;
    };
    sdk_req.matches(current_sdk_version())
}
fn catalog_release_is_scryer_compatible(plugin_id: &str, release: &CatalogV3PluginRelease) -> bool {
    let parse_bound = |field: &'static str, raw: Option<&str>| {
        let Some(raw) = raw.map(str::trim).filter(|version| !version.is_empty()) else {
            return Some(None);
        };
        match semver::Version::parse(raw) {
            Ok(version) => Some(Some(version)),
            Err(_) => {
                warn!(
                    plugin_id,
                    version = release.version.as_str(),
                    bound = raw,
                    field,
                    "skipping plugin release with invalid Scryer version bound"
                );
                None
            }
        }
    };
    let Some(min_scryer_version) =
        parse_bound("min_scryer_version", release.min_scryer_version.as_deref())
    else {
        return false;
    };
    let Some(max_scryer_version) =
        parse_bound("max_scryer_version", release.max_scryer_version.as_deref())
    else {
        return false;
    };
    min_scryer_version
        .as_ref()
        .is_none_or(|min| current_scryer_version() >= min)
        && max_scryer_version
            .as_ref()
            .is_none_or(|max| current_scryer_version() <= max)
}
fn catalog_release_is_host_compatible(plugin_id: &str, release: &CatalogV3PluginRelease) -> bool {
    catalog_release_is_sdk_compatible(plugin_id, release)
        && catalog_release_is_scryer_compatible(plugin_id, release)
}
fn catalog_release_scryer_constraint(release: &CatalogV3PluginRelease) -> Option<String> {
    let mut bounds = Vec::new();
    if let Some(min) = release
        .min_scryer_version
        .as_deref()
        .map(str::trim)
        .filter(|version| !version.is_empty())
    {
        bounds.push(format!(">={min}"));
    }
    if let Some(max) = release
        .max_scryer_version
        .as_deref()
        .map(str::trim)
        .filter(|version| !version.is_empty())
    {
        bounds.push(format!("<={max}"));
    }
    (!bounds.is_empty()).then(|| bounds.join(", "))
}
#[cfg(test)]
fn latest_compatible_child_release(child: &ChildCatalog) -> Option<ChildCatalogRelease> {
    child
        .releases
        .iter()
        .filter_map(|release| {
            let constraint = effective_host_sdk_constraint(None, &release.sdk_constraint);
            let sdk_req = semver::VersionReq::parse(constraint.trim()).ok()?;
            sdk_req.matches(current_sdk_version()).then_some(release)
        })
        .filter_map(|release| {
            semver::Version::parse(release.version.trim_start_matches('v'))
                .ok()
                .map(|version| (version, release))
        })
        .max_by(|(left, _), (right, _)| left.cmp(right))
        .map(|(_, release)| release.clone())
}
fn installed_catalog_release(
    plugin: &CatalogV3PluginEntry,
    installation: &PluginInstallation,
) -> Option<CatalogV3PluginRelease> {
    plugin
        .releases
        .iter()
        .find(|release| {
            release.version == installation.version
                && release.sdk_constraint == installation.sdk_constraint
        })
        .cloned()
}
fn catalog_artifact_requires_simd(artifact: &CatalogV3PluginArtifact) -> bool {
    artifact.required_features.iter().any(|feature| {
        matches!(
            feature.trim().to_ascii_lowercase().as_str(),
            "simd128" | "relaxed-simd"
        )
    })
}
fn parsed_digest_matches(candidate: &str, algorithm: &str, digest: &str) -> bool {
    parse_digest_string(candidate).is_ok_and(|(candidate_algorithm, candidate_digest)| {
        candidate_algorithm.eq_ignore_ascii_case(algorithm)
            && candidate_digest.eq_ignore_ascii_case(digest)
    })
}
fn installed_wasm_matches_catalog_artifact(
    installation: &PluginInstallation,
    artifact: &CatalogV3PluginArtifact,
) -> bool {
    let (Some(algorithm), Some(digest)) = (
        installation.wasm_digest_algo.as_deref(),
        installation.wasm_digest.as_deref(),
    ) else {
        return false;
    };
    artifact
        .wasm_digests
        .iter()
        .any(|candidate| parsed_digest_matches(candidate, algorithm, digest))
}
fn installed_artifact_matches_catalog_artifact(
    installation: &PluginInstallation,
    artifact: &CatalogV3PluginArtifact,
) -> bool {
    if installation.wasm_digest_algo.is_some() && installation.wasm_digest.is_some() {
        return installed_wasm_matches_catalog_artifact(installation, artifact);
    }
    if let Some(artifact_digest) = installation.artifact_digest.as_deref()
        && let Ok((algorithm, digest)) = parse_digest_string(artifact_digest)
    {
        return artifact
            .digests
            .iter()
            .any(|candidate| parsed_digest_matches(candidate, &algorithm, &digest));
    }
    installation
        .source_url
        .as_deref()
        .is_some_and(|source_url| source_url.trim() == artifact.url.trim())
}
fn same_version_simd_artifact_update_available(
    installation: &PluginInstallation,
    release: &CatalogV3PluginRelease,
    artifact: &CatalogV3PluginArtifact,
) -> bool {
    release.sdk_constraint == installation.sdk_constraint
        && catalog_artifact_requires_simd(artifact)
        && !installed_artifact_matches_catalog_artifact(installation, artifact)
}
fn catalog_plugin_update_available(
    installation: &PluginInstallation,
    resolved: &CatalogPluginResolution,
) -> bool {
    let Some(selected_version) =
        parse_catalog_release_version(&resolved.catalog_entry.id, &resolved.release)
    else {
        return false;
    };
    let Ok(installed_version) = semver::Version::parse(installation.version.as_str()) else {
        return false;
    };
    match selected_version.cmp(&installed_version) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Equal => same_version_simd_artifact_update_available(
            installation,
            &resolved.release,
            &resolved.artifact,
        ),
        std::cmp::Ordering::Less => false,
    }
}
/// Select the best artifact of one release for this host.
///
/// `runtime_capabilities` is the host's capability-token set — WASI targets and
/// wasm features in one namespace, as declared by
/// `scryer_plugins::runtime_features`. An artifact the host cannot run is
/// skipped rather than rejected, so a release that only ships for a newer WASI
/// target simply yields nothing here and the caller falls back to an older
/// release.
fn select_catalog_release_artifact(
    release: &CatalogV3PluginRelease,
    runtime_capabilities: &HashSet<String>,
    cpu_class: crate::services::RuntimePerformanceClass,
) -> Option<CatalogV3PluginArtifact> {
    let preferred_encoding = preferred_plugin_artifact_encoding(cpu_class);
    let mut matching = release
        .artifacts
        .iter()
        .filter(|artifact| catalog_v3_artifact_is_runnable(artifact, runtime_capabilities))
        .cloned()
        .collect::<Vec<_>>();
    matching.sort_by(|left, right| {
        catalog_v3_artifact_preference(right)
            .cmp(&catalog_v3_artifact_preference(left))
            .then_with(|| {
                let left_preferred =
                    artifact_encoding_from_url(&left.url) == Some(preferred_encoding);
                let right_preferred =
                    artifact_encoding_from_url(&right.url) == Some(preferred_encoding);
                right_preferred.cmp(&left_preferred)
            })
    });
    matching.into_iter().next()
}
/// Newest release this host can actually run.
///
/// A release is a candidate only when the host satisfies its SDK and Scryer
/// version bounds *and* one of its artifacts is runnable here. That last clause
/// is the WASI-version fallback: offered `2.1.0` built only for a target this
/// build does not have and `2.0.4` built for one it does, this returns 2.0.4.
fn select_catalog_release_and_artifact(
    plugin: &CatalogV3PluginEntry,
    runtime_capabilities: &HashSet<String>,
    cpu_class: crate::services::RuntimePerformanceClass,
) -> Option<(CatalogV3PluginRelease, CatalogV3PluginArtifact)> {
    plugin
        .releases
        .iter()
        .filter(|release| catalog_release_is_host_compatible(&plugin.id, release))
        .filter_map(|release| {
            select_catalog_release_artifact(release, runtime_capabilities, cpu_class)
                .map(|artifact| (release, artifact))
        })
        .filter_map(|(release, artifact)| {
            parse_catalog_release_version(&plugin.id, release)
                .map(|version| (version, release, artifact))
        })
        .max_by(|(left, _, _), (right, _, _)| left.cmp(right))
        .map(|(_, release, artifact)| (release.clone(), artifact))
}
fn parse_rule_pack_release_version(
    pack_id: &str,
    release: &CatalogV3RulePackRelease,
) -> Option<semver::Version> {
    semver::Version::parse(release.version.trim_start_matches('v')).map_or_else(
        |error| {
            warn!(
                pack_id,
                version = release.version.as_str(),
                error = %error,
                "skipping rule pack release with invalid version"
            );
            None
        },
        Some,
    )
}
fn preferred_distribution_artifact_encoding(
    cpu_class: crate::services::RuntimePerformanceClass,
) -> &'static str {
    preferred_plugin_artifact_encoding(cpu_class)
}
fn select_distribution_artifact(
    artifacts: &[CatalogV3DistributionArtifact],
    cpu_class: crate::services::RuntimePerformanceClass,
) -> Option<CatalogV3DistributionArtifact> {
    let preferred_encoding = preferred_distribution_artifact_encoding(cpu_class);
    let mut matching = artifacts.to_vec();
    matching.sort_by(|left, right| {
        let left_preferred = artifact_encoding_from_url(&left.url) == Some(preferred_encoding);
        let right_preferred = artifact_encoding_from_url(&right.url) == Some(preferred_encoding);
        right_preferred.cmp(&left_preferred)
    });
    matching.into_iter().next()
}
fn select_rule_pack_release_and_artifact(
    pack: &CatalogV3RulePackEntry,
    cpu_class: crate::services::RuntimePerformanceClass,
) -> Option<(CatalogV3RulePackRelease, CatalogV3DistributionArtifact)> {
    pack.releases
        .iter()
        .filter(|release| {
            release
                .min_scryer_version
                .as_ref()
                .and_then(|v| semver::Version::parse(v).ok())
                .is_none_or(|min| current_scryer_version() >= &min)
        })
        .filter_map(|release| {
            select_distribution_artifact(&release.artifacts, cpu_class)
                .map(|artifact| (release, artifact))
        })
        .filter_map(|(release, artifact)| {
            parse_rule_pack_release_version(&pack.id, release)
                .map(|version| (version, release, artifact))
        })
        .max_by(|(left, _, _), (right, _, _)| left.cmp(right))
        .map(|(_, release, artifact)| (release.clone(), artifact))
}
fn installation_is_catalog_official(installation: &PluginInstallation) -> bool {
    installation.source_kind == PluginSourceKind::Downloaded
        && installation.support_tier == PluginSupportTier::Official
        && installation.wasm_digest_algo.is_some()
        && installation.wasm_digest.is_some()
}
fn installation_is_first_party(installation: &PluginInstallation) -> bool {
    installation_is_catalog_official(installation)
}
fn catalog_resolution_is_first_party(resolved: &CatalogPluginResolution) -> bool {
    resolved.source_kind == PluginSourceKind::Downloaded
        && resolved.effective_support_tier == PluginSupportTier::Official
}
const DEFAULT_CATALOG_URL: &str =
    "https://cdn.scryer.media/scryer/catalog/v3/catalog-v3.redirect.json";
const FALLBACK_CATALOG_URL: &str = "https://github.com/scryer-media/scryer-plugins/releases/download/catalog%2Fv3/catalog-v3.redirect.json";
const CATALOG_URL_ENV: &str = "SCRYER_PLUGIN_CATALOG_URL";
const CENTRAL_CATALOG_SOURCE_KEY: &str = "__central_catalog";
const LEGACY_CENTRAL_CATALOG_SOURCE_KEY: &str = "__central_catalog_v2";
const CENTRAL_CATALOG_REPO: &str = "scryer-media/scryer-plugins";
const CENTRAL_CATALOG_WORKFLOW: &str = ".github/workflows/release-plugin-v3.yml";
fn community_catalog_source_key(plugin_id: &str) -> String {
    format!("community:{plugin_id}")
}
fn plugin_catalog_url() -> String {
    std::env::var(CATALOG_URL_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_CATALOG_URL.to_string())
}
fn fallback_plugin_catalog_url() -> &'static str {
    FALLBACK_CATALOG_URL
}
fn signed_catalog_json_bundle_url(url: &str) -> String {
    format!("{url}.bundle.zst")
}
fn provider_catalog_families_for_plugin_type(plugin_type: &str) -> Vec<ProviderCatalogFamily> {
    if is_indexer_plugin_type(plugin_type) {
        return vec![ProviderCatalogFamily::Indexer];
    }

    match plugin_type {
        "download_client" => vec![ProviderCatalogFamily::DownloadClient],
        "notification" => vec![ProviderCatalogFamily::Notification],
        "subtitle_provider" => vec![ProviderCatalogFamily::Subtitle],
        "archive_extractor" => vec![ProviderCatalogFamily::ArchiveExtractor],
        _ => ProviderCatalogFamily::all().into_iter().collect(),
    }
}
async fn fetch_plugin_bytes_from_locations_with_redirect_policy(
    locations: &[String],
    label: &str,
    scope_prefix: &str,
    redirect_policy: PluginRedirectPolicy,
) -> AppResult<(Vec<u8>, String)> {
    let mut last_error = None;
    for (index, url) in locations.iter().enumerate() {
        match fetch_plugin_bytes_with_redirect_policy(
            url,
            label,
            format!("{scope_prefix}:{index}"),
            redirect_policy,
        )
        .await
        {
            Ok(fetched) => return Ok((fetched.bytes, fetched.actual_url)),
            Err(error) => {
                debug!(%url, error = %error, "plugin fetch location failed");
                last_error = Some(error);
            }
        }
    }
    Err(last_error.unwrap_or_else(|| {
        AppError::Repository(format!(
            "failed to download {label}: no candidate URLs were available"
        ))
    }))
}

async fn decode_signature_bundle(bundle: Vec<u8>, actual_url: &str) -> AppResult<Vec<u8>> {
    match artifact_encoding_from_url(actual_url) {
        Some("zst") => {
            decompress_zstd(
                bundle,
                PLUGIN_SIGNATURE_BUNDLE_OUTPUT_LIMIT,
                "plugin signature bundle",
            )
            .await
        }
        Some("br") => {
            decompress_brotli(
                bundle,
                PLUGIN_SIGNATURE_BUNDLE_OUTPUT_LIMIT,
                "plugin signature bundle",
            )
            .await
        }
        _ => bound_uncompressed_bytes(
            bundle,
            PLUGIN_SIGNATURE_BUNDLE_OUTPUT_LIMIT,
            "plugin signature bundle",
        ),
    }
}
async fn decode_catalog_json(raw: Vec<u8>, actual_url: &str, label: &str) -> AppResult<Vec<u8>> {
    match artifact_encoding_from_url(actual_url) {
        Some("zst") => decompress_zstd(raw, PLUGIN_CATALOG_JSON_OUTPUT_LIMIT, label).await,
        Some("br") => decompress_brotli(raw, PLUGIN_CATALOG_JSON_OUTPUT_LIMIT, label).await,
        _ => bound_uncompressed_bytes(raw, PLUGIN_CATALOG_JSON_OUTPUT_LIMIT, label),
    }
}
async fn decode_rule_pack_manifest_bytes(
    compressed_manifest: Vec<u8>,
    actual_url: &str,
    pack_id: &str,
    release: &CatalogV3RulePackRelease,
) -> AppResult<Vec<u8>> {
    let rule_pack_manifest_limit = release
        .rule_pack_bytes
        .unwrap_or(RULE_PACK_MANIFEST_FALLBACK_OUTPUT_LIMIT);
    let manifest_label = format!("rule pack '{pack_id}' manifest");
    match artifact_encoding_from_url(actual_url) {
        Some("br") => {
            decompress_brotli(compressed_manifest, rule_pack_manifest_limit, manifest_label).await
        }
        Some("zst") => {
            decompress_zstd(compressed_manifest, rule_pack_manifest_limit, manifest_label).await
        }
        _ => Err(AppError::Validation(format!(
            "rule pack '{pack_id}' selected artifact '{actual_url}' has unsupported encoding"
        ))),
    }
}
async fn fetch_signed_blob_from_locations(
    data_urls: &[String],
    signature_urls: &[String],
    label: &str,
) -> AppResult<FetchedSignedBlob> {
    fetch_signed_blob_from_locations_with_redirect_policy(
        data_urls,
        signature_urls,
        label,
        PluginRedirectPolicy::Reject,
    )
    .await
}

async fn fetch_signed_blob_from_locations_with_redirect_policy(
    data_urls: &[String],
    signature_urls: &[String],
    label: &str,
    redirect_policy: PluginRedirectPolicy,
) -> AppResult<FetchedSignedBlob> {
    let scope = format!("verified_blob:{}", blake3_digest(label.as_bytes()));
    let (raw, actual_url) = fetch_plugin_bytes_from_locations_with_redirect_policy(
        data_urls,
        label,
        &format!("{scope}:blob"),
        redirect_policy,
    )
    .await?;
    let (bundle, bundle_url) = fetch_plugin_bytes_from_locations_with_redirect_policy(
        signature_urls,
        &format!("{label} signature"),
        &format!("{scope}:signature"),
        redirect_policy,
    )
    .await?;
    let signature_bundle = decode_signature_bundle(bundle, &bundle_url).await?;
    Ok(FetchedSignedBlob {
        raw,
        actual_url,
        signature_bundle,
    })
}

fn central_catalog_required_signer() -> RequiredSigner {
    RequiredSigner {
        github_repository: CENTRAL_CATALOG_REPO.to_string(),
        github_workflow: Some(CENTRAL_CATALOG_WORKFLOW.to_string()),
        github_ref: None,
    }
}

fn central_catalog_redirect_policy() -> PluginRedirectPolicy {
    PluginRedirectPolicy::FollowValidated
}

async fn fetch_verified_central_catalog_blob(
    data_urls: &[String],
    signature_urls: &[String],
    label: &str,
) -> AppResult<(Vec<u8>, String)> {
    let fetched = fetch_signed_blob_from_locations_with_redirect_policy(
        data_urls,
        signature_urls,
        label,
        central_catalog_redirect_policy(),
    )
    .await?;
    verify_signed_blob(
        fetched.raw.clone(),
        fetched.signature_bundle,
        central_catalog_required_signer(),
    )
    .await?;
    Ok((fetched.raw, fetched.actual_url))
}

async fn fetch_verified_catalog_redirect_candidate(
    url: &str,
    label: &str,
) -> AppResult<(CatalogV3Redirect, String)> {
    let data_urls = vec![url.to_string()];
    let signature_urls = vec![redirect_bundle_url_for(url)];
    let (raw, actual_url) =
        fetch_verified_central_catalog_blob(&data_urls, &signature_urls, label).await?;
    let redirect_raw = bound_uncompressed_bytes(
        raw,
        PLUGIN_CATALOG_REDIRECT_OUTPUT_LIMIT,
        "plugin catalog redirect",
    )?;
    let redirect = parse_and_validate_catalog_v3_redirect(&redirect_raw)?;
    Ok((redirect, actual_url))
}
fn validate_community_catalog_v3_delegate(
    source: &CatalogV3CommunitySource,
    repo: &GitHubRepo,
    catalog: &CatalogV3,
) -> AppResult<()> {
    if source.support_tier != PluginSupportTier::VerifiedCommunity {
        return Err(AppError::Validation(format!(
            "community catalog source '{}' approved tier must be verified_community",
            source.id
        )));
    }
    if catalog.plugins.len() != 1 {
        return Err(AppError::Validation(format!(
            "community catalog source '{}' must publish exactly one plugin",
            source.id
        )));
    }
    if !catalog.rule_packs.is_empty() {
        return Err(AppError::Validation(format!(
            "community catalog source '{}' must not publish rule packs",
            source.id
        )));
    }
    if !catalog.community_sources.is_empty() {
        return Err(AppError::Validation(format!(
            "community catalog source '{}' must not publish nested community sources",
            source.id
        )));
    }
    let plugin = &catalog.plugins[0];
    if plugin.id != source.id {
        return Err(AppError::Validation(format!(
            "community catalog source '{}' published plugin '{}'",
            source.id, plugin.id
        )));
    }
    if plugin.support_tier != source.support_tier {
        return Err(AppError::Validation(format!(
            "community catalog source '{}' support tier does not match approved tier",
            source.id
        )));
    }
    if plugin.required_signer.github_repository != repo.slug() {
        return Err(AppError::Validation(format!(
            "community catalog source '{}' signer repo '{}' does not match approved repo '{}'",
            source.id,
            plugin.required_signer.github_repository,
            repo.slug()
        )));
    }
    let source_repo = GitHubRepo::parse(&plugin.source_repo)?;
    if source_repo != *repo {
        return Err(AppError::Validation(format!(
            "community catalog source '{}' source repo '{}' does not match approved repo '{}'",
            source.id,
            source_repo.slug(),
            repo.slug()
        )));
    }
    Ok(())
}
impl AppUseCase {
    async fn cached_central_catalog(&self) -> AppResult<Option<CatalogV3>> {
        let Some(source) = self
            .services
            .customization
            .plugin_installations
            .get_plugin_catalog_source(CENTRAL_CATALOG_SOURCE_KEY)
            .await?
        else {
            return Ok(None);
        };
        let Some(json) = source.catalog_json else {
            return Ok(None);
        };
        match parse_and_validate_catalog_v3(json.as_bytes()) {
            Ok(catalog) => Ok(Some(catalog)),
            Err(error) => {
                warn!(error = %error, "cached central plugin catalog is invalid");
                Ok(None)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct EnvGuard {
        original: Option<std::ffi::OsString>,
    }

    impl EnvGuard {
        fn new() -> Self {
            Self {
                original: std::env::var_os(CATALOG_URL_ENV),
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.original {
                // SAFETY: this test serializes access to the process environment with ENV_LOCK.
                Some(value) => unsafe { std::env::set_var(CATALOG_URL_ENV, value) },
                // SAFETY: this test serializes access to the process environment with ENV_LOCK.
                None => unsafe { std::env::remove_var(CATALOG_URL_ENV) },
            }
        }
    }

    #[test]
    fn plugin_catalog_url_uses_canonical_default_and_env_override() {
        let _lock = ENV_LOCK.lock().expect("lock catalog url env");
        let _guard = EnvGuard::new();

        // SAFETY: this test serializes access to the process environment with ENV_LOCK.
        unsafe { std::env::remove_var(CATALOG_URL_ENV) };
        assert_eq!(
            plugin_catalog_url(),
            "https://cdn.scryer.media/scryer/catalog/v3/catalog-v3.redirect.json"
        );

        // SAFETY: this test serializes access to the process environment with ENV_LOCK.
        unsafe { std::env::set_var(CATALOG_URL_ENV, "   ") };
        assert_eq!(
            plugin_catalog_url(),
            "https://cdn.scryer.media/scryer/catalog/v3/catalog-v3.redirect.json"
        );

        // SAFETY: this test serializes access to the process environment with ENV_LOCK.
        unsafe {
            std::env::set_var(
                CATALOG_URL_ENV,
                " https://example.test/catalog-v3.redirect.json ",
            )
        };
        assert_eq!(
            plugin_catalog_url(),
            "https://example.test/catalog-v3.redirect.json"
        );
    }

    #[test]
    fn central_catalog_signed_blobs_follow_validated_redirects() {
        assert!(matches!(
            central_catalog_redirect_policy(),
            PluginRedirectPolicy::FollowValidated
        ));
    }
}
impl AppUseCase {
    async fn load_rule_pack_catalog(&self) -> AppResult<CatalogV3> {
        if let Some(catalog) = self.cached_central_catalog().await? {
            return Ok(catalog);
        }

        self.refresh_plugin_catalog_internal().await?;
        self.cached_central_catalog().await?.ok_or_else(|| {
            AppError::Repository("central plugin catalog is unavailable".to_string())
        })
    }
}
impl AppUseCase {
    pub async fn refresh_plugin_catalog(&self, actor: &User) -> AppResult<Vec<RegistryPlugin>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        self.refresh_plugin_catalog_internal().await?;
        self.list_available_plugins(actor).await
    }
}
impl AppUseCase {
    pub async fn refresh_plugin_catalog_internal(&self) -> AppResult<()> {
        let (central, redirect_url, _) = match self.fetch_verified_catalog_v3().await {
            Ok(fetched) => fetched,
            Err(error) => {
                let stored_status = self
                    .load_stored_plugin_catalog_status_payload()
                    .await
                    .unwrap_or_default();
                let last_error = error.to_string();
                if let Err(persist_error) = self
                    .persist_plugin_catalog_status_payload(
                        StoredPluginCatalogStatusPayload {
                            github_available: false,
                            blocked_actions: plugin_catalog_blocked_actions(),
                            message: Some(format!("Plugin catalog refresh failed: {last_error}")),
                            restore_warnings: stored_status.restore_warnings,
                            last_error: Some(last_error),
                        },
                        Utc::now(),
                    )
                    .await
                {
                    warn!(
                        error = %persist_error,
                        "failed to persist degraded plugin catalog status"
                    );
                }
                return Err(error);
            }
        };
        let community_sources = central.community_sources.clone();
        let approved_community_source_keys = community_sources
            .iter()
            .map(|source| community_catalog_source_key(&source.id))
            .collect::<std::collections::HashSet<_>>();
        let central_json = serde_json::to_string(&central).map_err(|error| {
            AppError::Repository(format!("failed to serialize plugin catalog cache: {error}"))
        })?;
        let now = Utc::now();
        self.services
            .customization
            .plugin_installations
            .upsert_plugin_catalog_source(&PluginCatalogSource {
                source_key: CENTRAL_CATALOG_SOURCE_KEY.to_string(),
                source_kind: "central".to_string(),
                source_url: redirect_url,
                github_repo: Some(CENTRAL_CATALOG_REPO.to_string()),
                support_tier: PluginSupportTier::Official,
                catalog_json: Some(central_json),
                last_success_at: Some(now),
                last_error: None,
                updated_at: now,
            })
            .await?;
        let stored_status = self.load_stored_plugin_catalog_status_payload().await?;
        self.persist_plugin_catalog_status_payload(
            StoredPluginCatalogStatusPayload {
                github_available: true,
                blocked_actions: Vec::new(),
                message: None,
                restore_warnings: stored_status.restore_warnings,
                last_error: None,
            },
            now,
        )
        .await?;

        let sources = self
            .services
            .customization
            .plugin_installations
            .list_plugin_catalog_sources()
            .await?;

        for stale_source in sources.iter().filter(|source| {
            source.source_key == LEGACY_CENTRAL_CATALOG_SOURCE_KEY
                || source.source_kind == "child"
                || (source.source_kind == "community"
                    && !approved_community_source_keys.contains(&source.source_key))
        }) {
            self.services
                .customization
                .plugin_installations
                .delete_plugin_catalog_source(&stale_source.source_key)
                .await?;
        }

        let mut community_tasks = tokio::task::JoinSet::new();
        for community_source in community_sources {
            let app = self.clone();
            community_tasks.spawn(async move {
                let repo = GitHubRepo::parse(&community_source.github_repository)?;
                let catalog_url = repo.delegated_catalog_v3_url();
                let source_key = community_catalog_source_key(&community_source.id);
                let result = app
                    .fetch_verified_community_catalog_v3(&community_source)
                    .await;
                Ok::<_, AppError>((community_source, repo, catalog_url, source_key, result))
            });
        }
        while let Some(joined) = community_tasks.join_next().await {
            let (community_source, repo, catalog_url, source_key, result) =
                joined.map_err(|error| {
                    AppError::Repository(format!(
                        "community plugin catalog refresh task failed to complete: {error}"
                    ))
                })??;
            match result {
                Ok((catalog, actual_url)) => {
                    let catalog_json = serde_json::to_string(&catalog).map_err(|error| {
                        AppError::Repository(format!(
                            "failed to serialize community plugin catalog cache: {error}"
                        ))
                    })?;
                    let now = Utc::now();
                    self.services
                        .customization
                        .plugin_installations
                        .upsert_plugin_catalog_source(&PluginCatalogSource {
                            source_key: source_key.clone(),
                            source_kind: "community".to_string(),
                            source_url: actual_url,
                            github_repo: Some(repo.slug()),
                            support_tier: community_source.support_tier,
                            catalog_json: Some(catalog_json),
                            last_success_at: Some(now),
                            last_error: None,
                            updated_at: now,
                        })
                        .await?;
                }
                Err(error) => {
                    warn!(
                        source_key = source_key.as_str(),
                        error = %error,
                        "verified community plugin catalog is unavailable"
                    );
                    let now = Utc::now();
                    self.services
                        .customization
                        .plugin_installations
                        .upsert_plugin_catalog_source(&PluginCatalogSource {
                            source_key,
                            source_kind: "community".to_string(),
                            source_url: catalog_url,
                            github_repo: Some(repo.slug()),
                            support_tier: community_source.support_tier,
                            catalog_json: None,
                            last_success_at: None,
                            last_error: Some(error.to_string()),
                            updated_at: now,
                        })
                        .await?;
                }
            }
        }

        for source in sources
            .into_iter()
            .filter(|source| source.source_kind == "manual")
        {
            let source_url = source.source_url.clone();
            let result = async {
                let repo_slug = source.github_repo.as_deref().ok_or_else(|| {
                    AppError::Validation(format!(
                        "manual plugin catalog source '{}' is missing github repo",
                        source.source_key
                    ))
                })?;
                let repo = GitHubRepo::parse(repo_slug)?;
                let catalog_url = if source_url.trim().is_empty() {
                    repo.catalog_v3_url()
                } else {
                    source_url.clone()
                };
                let (_, catalog_json) = self
                    .resolve_manual_plugin_repo_at_url(repo.clone(), &catalog_url)
                    .await?;
                self.upsert_manual_plugin_catalog_source(
                    &repo,
                    &catalog_url,
                    Some(catalog_json),
                    None,
                )
                .await
            }
            .await;

            if let Err(error) = result {
                warn!(
                    source_key = source.source_key.as_str(),
                    error = %error,
                    "verified manual plugin catalog is unavailable"
                );
                if let Some(repo) = source
                    .github_repo
                    .as_deref()
                    .and_then(|repo| GitHubRepo::parse(repo).ok())
                {
                    let catalog_url = if source_url.trim().is_empty() {
                        repo.catalog_v3_url()
                    } else {
                        source_url.clone()
                    };
                    self.upsert_manual_plugin_catalog_source(
                        &repo,
                        &catalog_url,
                        None,
                        Some(error.to_string()),
                    )
                    .await?;
                }
            }
        }

        self.publish_provider_catalog_changed(ProviderCatalogFamily::all().into_iter().collect());
        Ok(())
    }
}
impl AppUseCase {
    async fn fetch_verified_blob_from_locations(
        &self,
        data_urls: &[String],
        signature_urls: &[String],
        signer: &RequiredSigner,
        label: &str,
    ) -> AppResult<(Vec<u8>, String)> {
        let fetched = fetch_signed_blob_from_locations(data_urls, signature_urls, label).await?;
        verify_signed_blob(
            fetched.raw.clone(),
            fetched.signature_bundle,
            signer.clone(),
        )
        .await?;
        Ok((fetched.raw, fetched.actual_url))
    }
}
impl AppUseCase {
    async fn fetch_verified_catalog_redirect(&self) -> AppResult<(CatalogV3Redirect, String)> {
        let primary_url = plugin_catalog_url();
        let fallback_url = fallback_plugin_catalog_url().to_string();
        let candidate_urls = vec![primary_url, fallback_url];
        let mut last_error = None;
        for url in candidate_urls {
            match fetch_verified_catalog_redirect_candidate(&url, "plugin catalog redirect").await {
                Ok((redirect, actual_url)) => return Ok((redirect, actual_url)),
                Err(error) => {
                    debug!(redirect_url = %url, error = %error, "plugin catalog redirect candidate failed");
                    last_error = Some(error);
                }
            }
        }
        Err(last_error.unwrap_or_else(|| {
            AppError::Repository("plugin catalog redirect is unavailable".to_string())
        }))
    }
}
impl AppUseCase {
    async fn fetch_verified_catalog_v3(&self) -> AppResult<(CatalogV3, String, u64)> {
        let (redirect, redirect_url) = self.fetch_verified_catalog_redirect().await?;
        if redirect.artifacts.is_empty() {
            return Err(AppError::Validation(
                "plugin catalog redirect did not contain any artifacts".to_string(),
            ));
        }
        // A catalog-v3 redirect is an ordered ladder of projections of one
        // catalog, oldest-tolerating first, newest last. Shipped clients pick a
        // fixed rung by position — pre-0.18.12 took the first, 0.18.12 through
        // 0.19.5 took the last — which is why those two bands could never be
        // served different content once their tolerances diverged: the ladder
        // has exactly two addressable positions.
        //
        // This walks back instead. Newest rung first, falling back to the rung
        // below whenever one cannot be fetched, verified, or parsed. A rung
        // added for a future Scryer is therefore never a trap for this one, and
        // the ladder can grow without bound.
        let mut last_error = None;
        for (index, artifact) in redirect.artifacts.iter().enumerate().rev() {
            let data_urls = primary_and_mirrors(&artifact.url, &artifact.mirror_urls);
            let signature_urls =
                primary_and_mirrors(&artifact.signature_url, &artifact.signature_mirror_urls);
            let candidate = async {
                let (raw, actual_url) = fetch_verified_central_catalog_blob(
                    &data_urls,
                    &signature_urls,
                    "plugin catalog",
                )
                .await?;
                let decoded = decode_catalog_json(raw, &actual_url, "plugin catalog").await?;
                parse_and_validate_catalog_v3(&decoded)
            }
            .await;
            match candidate {
                Ok(catalog) => return Ok((catalog, redirect_url, redirect.catalog_version)),
                Err(error) => {
                    warn!(
                        projection_index = index,
                        url = artifact.url.as_str(),
                        error = %error,
                        "plugin catalog projection unusable; falling back to the previous one"
                    );
                    last_error = Some(error);
                }
            }
        }
        Err(last_error.unwrap_or_else(|| {
            AppError::Validation(
                "plugin catalog redirect did not yield a usable catalog".to_string(),
            )
        }))
    }

    async fn fetch_verified_community_catalog_v3(
        &self,
        source: &CatalogV3CommunitySource,
    ) -> AppResult<(CatalogV3, String)> {
        let repo = GitHubRepo::parse(&source.github_repository)?;
        let catalog_url = repo.delegated_catalog_v3_url();
        let signer = RequiredSigner {
            github_repository: repo.slug(),
            github_workflow: None,
            github_ref: None,
        };
        let data_urls = vec![catalog_url.clone()];
        let signature_urls = vec![signed_catalog_json_bundle_url(&catalog_url)];
        let (raw, actual_url) = self
            .fetch_verified_blob_from_locations(
                &data_urls,
                &signature_urls,
                &signer,
                "community plugin catalog",
            )
            .await?;
        let decoded = decode_catalog_json(raw, &actual_url, "community plugin catalog").await?;
        let catalog = parse_and_validate_catalog_v3(&decoded)?;
        validate_community_catalog_v3_delegate(source, &repo, &catalog)?;
        Ok((catalog, actual_url))
    }
}
impl AppUseCase {
    async fn resolved_catalog_plugins(&self) -> AppResult<Vec<CatalogPluginResolution>> {
        let sources = self
            .services
            .customization
            .plugin_installations
            .list_plugin_catalog_sources()
            .await?;
        Ok(self
            .resolve_catalog_plugins_from_sources(&sources)
            .await?
            .resolved)
    }

    async fn resolve_catalog_plugins_from_sources(
        &self,
        sources: &[scryer_domain::PluginCatalogSource],
    ) -> AppResult<CatalogPluginSourceResolution> {
        let supported_plugin_features = self.runtime_supported_plugin_required_features();
        let cpu_class = self.runtime_performance().await.cpu_class;
        let central = sources
            .iter()
            .find(|source| source.source_key == CENTRAL_CATALOG_SOURCE_KEY)
            .and_then(|source| source.catalog_json.as_deref())
            .and_then(|json| parse_and_validate_catalog_v3(json.as_bytes()).ok());

        let mut result = Vec::new();
        if let Some(central) = central.as_ref() {
            for entry in &central.plugins {
                let Some((release, artifact)) = select_catalog_release_and_artifact(
                    entry,
                    &supported_plugin_features,
                    cpu_class,
                ) else {
                    continue;
                };
                let github_repo = GitHubRepo::parse(&entry.source_repo)?;
                result.push(CatalogPluginResolution {
                    catalog_entry: entry.clone(),
                    release,
                    artifact,
                    effective_support_tier: entry.support_tier,
                    source_kind: PluginSourceKind::Downloaded,
                    github_repo,
                });
            }
        }

        let mut seen_plugin_ids = result
            .iter()
            .map(|resolved| resolved.catalog_entry.id.clone())
            .collect::<std::collections::HashSet<_>>();
        for source in sources
            .iter()
            .filter(|source| source.source_kind == "community")
        {
            let Some(catalog_json) = source.catalog_json.as_deref() else {
                continue;
            };
            let Some(repo_slug) = source.github_repo.as_deref() else {
                warn!(
                    source_key = source.source_key.as_str(),
                    "cached community plugin catalog source is missing github repo"
                );
                continue;
            };
            let Some(approved_id) = source.source_key.strip_prefix("community:") else {
                warn!(
                    source_key = source.source_key.as_str(),
                    "cached community plugin catalog source has invalid source key"
                );
                continue;
            };
            let community_repo = match GitHubRepo::parse(repo_slug) {
                Ok(repo) => repo,
                Err(error) => {
                    warn!(
                        source_key = source.source_key.as_str(),
                        error = %error,
                        "cached community plugin catalog source has invalid github repo"
                    );
                    continue;
                }
            };
            let catalog = match parse_and_validate_catalog_v3(catalog_json.as_bytes()) {
                Ok(catalog) => catalog,
                Err(error) => {
                    warn!(
                        source_key = source.source_key.as_str(),
                        error = %error,
                        "ignoring invalid cached community plugin catalog"
                    );
                    continue;
                }
            };
            let approved_source = CatalogV3CommunitySource {
                id: approved_id.to_string(),
                github_repository: community_repo.slug(),
                support_tier: source.support_tier,
            };
            if let Err(error) =
                validate_community_catalog_v3_delegate(&approved_source, &community_repo, &catalog)
            {
                warn!(
                    source_key = source.source_key.as_str(),
                    error = %error,
                    "ignoring invalid cached community plugin catalog"
                );
                continue;
            }
            let plugin = catalog.plugins[0].clone();
            if !seen_plugin_ids.insert(plugin.id.clone()) {
                warn!(
                    plugin_id = plugin.id.as_str(),
                    source_key = source.source_key.as_str(),
                    "ignoring duplicate community plugin catalog entry"
                );
                continue;
            }
            let Some((release, artifact)) =
                select_catalog_release_and_artifact(&plugin, &supported_plugin_features, cpu_class)
            else {
                continue;
            };
            result.push(CatalogPluginResolution {
                catalog_entry: plugin,
                release,
                artifact,
                source_kind: PluginSourceKind::Community,
                effective_support_tier: source.support_tier,
                github_repo: community_repo,
            });
        }

        for source in sources
            .iter()
            .filter(|source| source.source_kind == "manual")
            .filter_map(|source| {
                source
                    .catalog_json
                    .as_deref()
                    .zip(source.github_repo.as_deref())
            })
        {
            let (catalog_json, repo_slug) = source;
            let manual_repo = GitHubRepo::parse(repo_slug)?;
            let catalog = parse_and_validate_catalog_v3(catalog_json.as_bytes())?;
            let plugin = single_manual_catalog_plugin(&catalog, &manual_repo)?;
            let Some((release, artifact)) =
                select_catalog_release_and_artifact(&plugin, &supported_plugin_features, cpu_class)
            else {
                continue;
            };
            result.push(CatalogPluginResolution {
                catalog_entry: plugin,
                release,
                artifact,
                source_kind: PluginSourceKind::Manual,
                effective_support_tier: PluginSupportTier::Unverified,
                github_repo: manual_repo,
            });
        }

        Ok(CatalogPluginSourceResolution {
            central,
            resolved: result,
        })
    }
}
impl AppUseCase {
    async fn validate_catalog_install_request(&self, plugin_id: &str) -> AppResult<()> {
        if self
            .services
            .customization
            .plugin_installations
            .get_plugin_installation(plugin_id)
            .await?
            .is_some()
        {
            return Err(AppError::Validation(format!(
                "plugin '{plugin_id}' is already installed"
            )));
        }

        let available = self
            .resolved_catalog_plugins()
            .await?
            .into_iter()
            .any(|plugin| plugin.catalog_entry.id == plugin_id);
        if available {
            Ok(())
        } else {
            Err(AppError::NotFound(format!(
                "plugin '{plugin_id}' is not available from the plugin catalog"
            )))
        }
    }
}
impl AppUseCase {
    async fn validate_catalog_upgrade_request(&self, plugin_id: &str) -> AppResult<()> {
        self.services
            .customization
            .plugin_installations
            .get_plugin_installation(plugin_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("plugin '{plugin_id}' not installed")))?;
        Ok(())
    }
}
impl AppUseCase {
    /// List available community rule packs from the cached central catalog.
    pub async fn list_rule_pack_registry(
        &self,
        actor: &User,
    ) -> AppResult<Vec<RulePackRegistryEntry>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let catalog = self.load_rule_pack_catalog().await?;
        let cpu_class = self.runtime_performance().await.cpu_class;
        Ok(catalog
            .rule_packs
            .into_iter()
            .filter_map(|pack| {
                select_rule_pack_release_and_artifact(&pack, cpu_class).map(
                    |(release, artifact)| {
                        RulePackRegistryEntry::from_release(&pack, &release, &artifact)
                    },
                )
            })
            .collect())
    }
}
impl AppUseCase {
    /// Fetch a community rule pack by its registry ID.
    pub async fn fetch_rule_pack_templates(
        &self,
        actor: &User,
        pack_id: &str,
    ) -> AppResult<Vec<RulePackTemplate>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let packs = self.list_rule_pack_registry(actor).await?;
        let pack = packs
            .iter()
            .find(|p| p.id == pack_id)
            .ok_or_else(|| AppError::NotFound(format!("rule pack {pack_id}")))?;
        let catalog = self.load_rule_pack_catalog().await?;
        let cpu_class = self.runtime_performance().await.cpu_class;
        let pack_entry = catalog
            .rule_packs
            .iter()
            .find(|candidate| candidate.id == pack.id)
            .ok_or_else(|| AppError::NotFound(format!("rule pack {pack_id}")))?;
        let (release, artifact) = select_rule_pack_release_and_artifact(pack_entry, cpu_class)
            .ok_or_else(|| {
                AppError::Validation(format!("rule pack '{pack_id}' has no compatible artifact"))
            })?;
        let signer = RequiredSigner {
            github_repository: CENTRAL_CATALOG_REPO.to_string(),
            github_workflow: Some(CENTRAL_CATALOG_WORKFLOW.to_string()),
            github_ref: None,
        };
        let (compressed_manifest, actual_url) = self
            .fetch_verified_blob_from_locations(
                &primary_and_mirrors(&artifact.url, &artifact.mirror_urls),
                &primary_and_mirrors(&artifact.signature_url, &artifact.signature_mirror_urls),
                &signer,
                "rule pack artifact",
            )
            .await?;
        verify_digest_set(
            "compressed rule pack artifact",
            &artifact.digests,
            &compressed_manifest,
        )?;
        let manifest_bytes =
            decode_rule_pack_manifest_bytes(compressed_manifest, &actual_url, pack_id, &release)
                .await?;
        verify_digest_set(
            "rule pack manifest",
            &release.rule_pack_digests,
            &manifest_bytes,
        )?;
        let manifest: RulePackManifest = serde_json::from_slice(&manifest_bytes)
            .map_err(|e| AppError::Repository(format!("invalid rule pack JSON: {e}")))?;

        Ok(manifest
            .rules
            .into_iter()
            .map(|r| RulePackTemplate {
                id: r.id,
                title: r.title,
                description: r.description,
                category: r.category,
                rego_source: r.rego_source,
                applied_facets: r.applied_facets,
            })
            .collect())
    }
}

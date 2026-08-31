//! Signed release-manifest validation for in-application upgrades.

use std::path::{Component, Path};

use semver::Version;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{AppError, AppResult, plugins::catalog::RequiredSigner};

/// The schema identifier accepted for signed upgrade manifests.
pub const UPGRADE_MANIFEST_SCHEMA_VERSION: &str = "scryer.upgrade.manifest.v1";

/// The maximum accepted signed upgrade-manifest size in bytes.
pub const UPGRADE_MANIFEST_MAX_BYTES: u64 = 262_144;

const SCRYER_RELEASE_REPOSITORY: &str = "scryer-media/scryer";
const SCRYER_RELEASE_WORKFLOW: &str = ".github/workflows/scryer.yml";

/// A signed release manifest describing installable Scryer artifacts.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UpgradeManifest {
    /// The schema identifier for this manifest.
    pub schema: String,
    /// The release tag used in artifact URLs.
    pub tag: String,
    /// The released application version.
    pub version: String,
    /// The artifacts available from this release.
    pub artifacts: Vec<UpgradeArtifact>,
}

/// A release artifact that can be used for an application upgrade.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UpgradeArtifact {
    /// The operating system for this artifact.
    pub platform: UpgradePlatform,
    /// The CPU architecture for this artifact.
    pub arch: UpgradeArchitecture,
    /// The distribution channel encoded by the artifact.
    pub channel: UpgradeChannel,
    /// The release-asset filename.
    pub asset_name: String,
    /// The canonical GitHub Release download URL.
    pub url: String,
    /// The artifact's exact byte length.
    pub size: u64,
    /// The lowercase hexadecimal BLAKE3 hash of the complete artifact.
    pub blake3: String,
    /// The container format of the artifact.
    pub archive: UpgradeArchive,
    /// The regular files contained in a portable archive.
    pub members: Vec<UpgradeArtifactMember>,
}

/// The operating systems supported by an upgrade artifact.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum UpgradePlatform {
    /// macOS.
    Darwin,
    /// Linux.
    Linux,
    /// Windows.
    Windows,
}

impl UpgradePlatform {
    fn as_str(self) -> &'static str {
        match self {
            Self::Darwin => "darwin",
            Self::Linux => "linux",
            Self::Windows => "windows",
        }
    }
}

/// The CPU architectures supported by an upgrade artifact.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub enum UpgradeArchitecture {
    /// 64-bit ARM.
    #[serde(rename = "arm64")]
    Arm64,
    /// 64-bit x86.
    #[serde(rename = "x86_64")]
    X86_64,
}

impl UpgradeArchitecture {
    fn as_str(self) -> &'static str {
        match self {
            Self::Arm64 => "arm64",
            Self::X86_64 => "x86_64",
        }
    }
}

/// The distribution channels supported by an upgrade artifact.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum UpgradeChannel {
    /// A portable archive.
    Portable,
    /// A Windows Installer package.
    Msi,
}

impl UpgradeChannel {
    fn as_str(self) -> &'static str {
        match self {
            Self::Portable => "portable",
            Self::Msi => "msi",
        }
    }
}

/// The container formats supported by an upgrade artifact.
///
/// Every portable artifact — Windows included — is a gzip-compressed tar
/// archive. ZIP is deliberately not a supported upgrade container: the
/// human-facing Windows `.zip` download is not part of the upgrade channel and
/// is never named by a signed manifest.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum UpgradeArchive {
    /// A gzip-compressed tar archive.
    #[serde(rename = "tar.gz")]
    TarGz,
    /// A Windows Installer package.
    #[serde(rename = "msi")]
    Msi,
}

/// A regular file in a portable upgrade archive.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UpgradeArtifactMember {
    /// The slash-separated relative archive path.
    pub path: String,
    /// The member's uncompressed byte length.
    pub size: u64,
    /// Whether the member is executable after extraction.
    pub executable: bool,
}

/// Parses and validates a signed upgrade manifest payload.
pub fn parse_and_validate_upgrade_manifest(raw: &[u8]) -> AppResult<UpgradeManifest> {
    if raw.len() as u64 > UPGRADE_MANIFEST_MAX_BYTES {
        return Err(AppError::Validation(format!(
            "upgrade manifest exceeds the maximum size of {UPGRADE_MANIFEST_MAX_BYTES} bytes"
        )));
    }

    let manifest = serde_json::from_slice::<UpgradeManifest>(raw)
        .map_err(|error| AppError::Validation(format!("invalid upgrade manifest JSON: {error}")))?;

    if manifest.schema != UPGRADE_MANIFEST_SCHEMA_VERSION {
        return Err(AppError::Validation(format!(
            "unsupported upgrade manifest schema '{}'; expected '{UPGRADE_MANIFEST_SCHEMA_VERSION}'",
            manifest.schema
        )));
    }
    if manifest.tag.is_empty() {
        return Err(AppError::Validation(
            "upgrade manifest tag must not be empty".to_string(),
        ));
    }
    if manifest.version.is_empty() {
        return Err(AppError::Validation(
            "upgrade manifest version must not be empty".to_string(),
        ));
    }
    let version = Version::parse(&manifest.version).map_err(|error| {
        AppError::Validation(format!(
            "upgrade manifest version must be a major.minor.patch semver: {error}"
        ))
    })?;
    if !version.pre.is_empty() || !version.build.is_empty() {
        return Err(AppError::Validation(
            "upgrade manifest version must be a major.minor.patch semver without prerelease or build metadata"
                .to_string(),
        ));
    }

    validate_artifact_order(&manifest.artifacts)?;
    for (artifact_index, artifact) in manifest.artifacts.iter().enumerate() {
        validate_artifact(artifact, artifact_index, &manifest.tag)?;
    }

    Ok(manifest)
}

/// Returns the Sigstore identity required for Scryer's release workflow.
pub fn scryer_release_required_signer(release_tag: &str) -> RequiredSigner {
    RequiredSigner {
        github_repository: SCRYER_RELEASE_REPOSITORY.to_string(),
        github_workflow: Some(SCRYER_RELEASE_WORKFLOW.to_string()),
        github_ref: Some(format!("refs/tags/{release_tag}")),
    }
}

fn validate_artifact_order(artifacts: &[UpgradeArtifact]) -> AppResult<()> {
    for pair in artifacts.windows(2) {
        let previous = artifact_sort_key(&pair[0]);
        let current = artifact_sort_key(&pair[1]);
        if current == previous {
            return Err(AppError::Validation(format!(
                "duplicate upgrade manifest artifact for platform '{}', arch '{}', and channel '{}'",
                current.0, current.1, current.2
            )));
        }
        if current < previous {
            return Err(AppError::Validation(
                "upgrade manifest artifacts must be strictly sorted by platform, arch, and channel"
                    .to_string(),
            ));
        }
    }
    Ok(())
}

fn artifact_sort_key(artifact: &UpgradeArtifact) -> (&str, &str, &str) {
    (
        artifact.platform.as_str(),
        artifact.arch.as_str(),
        artifact.channel.as_str(),
    )
}

fn validate_artifact(artifact: &UpgradeArtifact, index: usize, tag: &str) -> AppResult<()> {
    if artifact.asset_name.is_empty() {
        return Err(AppError::Validation(format!(
            "upgrade manifest artifact {index} asset_name must not be empty"
        )));
    }

    let url = Url::parse(&artifact.url).map_err(|error| {
        AppError::Validation(format!(
            "upgrade manifest artifact {index} has an invalid URL: {error}"
        ))
    })?;
    if url.scheme() != "https" {
        return Err(AppError::Validation(format!(
            "upgrade manifest artifact {index} URL must use https"
        )));
    }
    let expected_prefix =
        format!("https://github.com/{SCRYER_RELEASE_REPOSITORY}/releases/download/{tag}/");
    if !artifact.url.starts_with(&expected_prefix) {
        return Err(AppError::Validation(format!(
            "upgrade manifest artifact {index} URL must start with '{expected_prefix}'"
        )));
    }
    if !artifact.url.ends_with(&artifact.asset_name) {
        return Err(AppError::Validation(format!(
            "upgrade manifest artifact {index} URL must end with its asset_name"
        )));
    }

    if !is_lowercase_blake3_hex(&artifact.blake3) {
        return Err(AppError::Validation(format!(
            "upgrade manifest artifact {index} blake3 must be 64 lowercase hexadecimal characters"
        )));
    }

    match (artifact.channel, artifact.archive) {
        (UpgradeChannel::Msi, UpgradeArchive::Msi) => {
            if !artifact.members.is_empty() {
                return Err(AppError::Validation(format!(
                    "upgrade manifest MSI artifact {index} must have no members"
                )));
            }
        }
        (UpgradeChannel::Msi, _) | (UpgradeChannel::Portable, UpgradeArchive::Msi) => {
            return Err(AppError::Validation(format!(
                "upgrade manifest artifact {index} channel and archive must both be MSI or both be non-MSI"
            )));
        }
        (UpgradeChannel::Portable, UpgradeArchive::TarGz) => {
            if artifact.members.is_empty() {
                return Err(AppError::Validation(format!(
                    "upgrade manifest portable artifact {index} must have at least one member"
                )));
            }
            validate_members(&artifact.members, index)?;
        }
    }

    Ok(())
}

fn validate_members(members: &[UpgradeArtifactMember], artifact_index: usize) -> AppResult<()> {
    for pair in members.windows(2) {
        if pair[1].path == pair[0].path {
            return Err(AppError::Validation(format!(
                "upgrade manifest artifact {artifact_index} has duplicate member path '{}'",
                pair[1].path
            )));
        }
        if pair[1].path < pair[0].path {
            return Err(AppError::Validation(format!(
                "upgrade manifest artifact {artifact_index} members must be sorted by path"
            )));
        }
    }

    for member in members {
        validate_member_path(&member.path, artifact_index)?;
    }
    Ok(())
}

fn validate_member_path(path: &str, artifact_index: usize) -> AppResult<()> {
    if path.is_empty() {
        return Err(AppError::Validation(format!(
            "upgrade manifest artifact {artifact_index} member path must not be empty"
        )));
    }
    if path.starts_with('/') || Path::new(path).is_absolute() || has_windows_drive_prefix(path) {
        return Err(AppError::Validation(format!(
            "upgrade manifest artifact {artifact_index} member path must be relative: '{path}'"
        )));
    }
    if path.contains('\\') {
        return Err(AppError::Validation(format!(
            "upgrade manifest artifact {artifact_index} member path must not contain backslashes: '{path}'"
        )));
    }
    if Path::new(path)
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return Err(AppError::Validation(format!(
            "upgrade manifest artifact {artifact_index} member path must not contain '..': '{path}'"
        )));
    }
    Ok(())
}

fn has_windows_drive_prefix(path: &str) -> bool {
    path.as_bytes().get(1) == Some(&b':')
        && path.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
}

fn is_lowercase_blake3_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLAKE3: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn valid_manifest() -> UpgradeManifest {
        UpgradeManifest {
            schema: UPGRADE_MANIFEST_SCHEMA_VERSION.to_string(),
            tag: "v1.2.3".to_string(),
            version: "1.2.3".to_string(),
            artifacts: vec![
                portable_artifact(UpgradePlatform::Darwin, UpgradeArchitecture::Arm64),
                portable_artifact(UpgradePlatform::Linux, UpgradeArchitecture::X86_64),
                msi_artifact(UpgradeArchitecture::Arm64),
                portable_artifact(UpgradePlatform::Windows, UpgradeArchitecture::Arm64),
            ],
        }
    }

    fn portable_artifact(platform: UpgradePlatform, arch: UpgradeArchitecture) -> UpgradeArtifact {
        let asset_name = format!("scryer-{}-{}.tar.gz", platform.as_str(), arch.as_str());
        UpgradeArtifact {
            platform,
            arch,
            channel: UpgradeChannel::Portable,
            url: format!(
                "https://github.com/{SCRYER_RELEASE_REPOSITORY}/releases/download/v1.2.3/{asset_name}"
            ),
            asset_name,
            size: 42,
            blake3: BLAKE3.to_string(),
            archive: UpgradeArchive::TarGz,
            members: vec![UpgradeArtifactMember {
                path: "bin/scryer".to_string(),
                size: 42,
                executable: true,
            }],
        }
    }

    fn msi_artifact(arch: UpgradeArchitecture) -> UpgradeArtifact {
        let asset_name = format!("scryer-windows-{}.msi", arch.as_str());
        UpgradeArtifact {
            platform: UpgradePlatform::Windows,
            arch,
            channel: UpgradeChannel::Msi,
            url: format!(
                "https://github.com/{SCRYER_RELEASE_REPOSITORY}/releases/download/v1.2.3/{asset_name}"
            ),
            asset_name,
            size: 42,
            blake3: BLAKE3.to_string(),
            archive: UpgradeArchive::Msi,
            members: Vec::new(),
        }
    }

    fn assert_rejected(manifest: UpgradeManifest, expected: &str) {
        let raw = serde_json::to_vec(&manifest).expect("serialize manifest");
        let error = parse_and_validate_upgrade_manifest(&raw).expect_err("manifest is rejected");
        assert!(
            error.to_string().contains(expected),
            "expected '{expected}' in '{error}'"
        );
    }

    #[test]
    fn accepts_valid_manifest() {
        let raw = serde_json::to_vec(&valid_manifest()).expect("serialize manifest");
        assert_eq!(
            parse_and_validate_upgrade_manifest(&raw).expect("valid manifest"),
            valid_manifest()
        );
    }

    #[test]
    fn rejects_oversized_manifest() {
        let raw = vec![b' '; UPGRADE_MANIFEST_MAX_BYTES as usize + 1];
        let error = parse_and_validate_upgrade_manifest(&raw).expect_err("oversized manifest");
        assert!(error.to_string().contains("exceeds the maximum size"));
    }

    #[test]
    fn rejects_unknown_fields() {
        let mut value = serde_json::to_value(valid_manifest()).expect("serialize manifest");
        value["unexpected"] = serde_json::Value::Bool(true);
        let raw = serde_json::to_vec(&value).expect("serialize JSON");
        let error = parse_and_validate_upgrade_manifest(&raw).expect_err("unknown field");
        assert!(error.to_string().contains("invalid upgrade manifest JSON"));
    }

    #[test]
    fn rejects_schema_and_version_violations() {
        let mut manifest = valid_manifest();
        manifest.schema = "other".to_string();
        assert_rejected(manifest, "unsupported upgrade manifest schema");

        let mut manifest = valid_manifest();
        manifest.tag.clear();
        assert_rejected(manifest, "tag must not be empty");

        let mut manifest = valid_manifest();
        manifest.version.clear();
        assert_rejected(manifest, "version must not be empty");

        let mut manifest = valid_manifest();
        manifest.version = "1.2".to_string();
        assert_rejected(manifest, "major.minor.patch semver");

        let mut manifest = valid_manifest();
        manifest.version = "1.2.3-rc.1".to_string();
        assert_rejected(manifest, "without prerelease");
    }

    #[test]
    fn rejects_invalid_artifact_urls() {
        let mut manifest = valid_manifest();
        manifest.artifacts[0].url = "not a URL".to_string();
        assert_rejected(manifest, "invalid URL");

        let mut manifest = valid_manifest();
        manifest.artifacts[0].url = manifest.artifacts[0].url.replacen("https", "http", 1);
        assert_rejected(manifest, "must use https");

        let mut manifest = valid_manifest();
        manifest.artifacts[0].url = manifest.artifacts[0]
            .url
            .replace("scryer-media/scryer", "other/repository");
        assert_rejected(manifest, "must start with");

        let mut manifest = valid_manifest();
        manifest.artifacts[0].asset_name = "other.tar.gz".to_string();
        assert_rejected(manifest, "must end with its asset_name");
    }

    #[test]
    fn rejects_unsorted_or_duplicate_artifacts() {
        let mut manifest = valid_manifest();
        manifest.artifacts.swap(0, 1);
        assert_rejected(manifest, "must be strictly sorted");

        let mut manifest = valid_manifest();
        manifest.artifacts.insert(
            1,
            portable_artifact(UpgradePlatform::Darwin, UpgradeArchitecture::Arm64),
        );
        assert_rejected(manifest, "duplicate upgrade manifest artifact");
    }

    #[test]
    fn rejects_inconsistent_archives_and_members() {
        let mut manifest = valid_manifest();
        manifest.artifacts[0].archive = UpgradeArchive::Msi;
        assert_rejected(manifest, "channel and archive");

        let mut manifest = valid_manifest();
        manifest.artifacts[2].archive = UpgradeArchive::TarGz;
        assert_rejected(manifest, "channel and archive");

        let mut manifest = valid_manifest();
        manifest.artifacts[2].members.push(UpgradeArtifactMember {
            path: "unexpected".to_string(),
            size: 1,
            executable: false,
        });
        assert_rejected(manifest, "MSI artifact");

        let mut manifest = valid_manifest();
        manifest.artifacts[0].members.clear();
        assert_rejected(manifest, "at least one member");
    }

    #[test]
    fn rejects_unsafe_duplicate_or_unsorted_member_paths() {
        for (path, expected) in [
            ("/absolute", "must be relative"),
            ("C:/absolute", "must be relative"),
            ("../parent", "must not contain '..'"),
            ("bin\\scryer", "must not contain backslashes"),
            ("", "must not be empty"),
        ] {
            let mut manifest = valid_manifest();
            manifest.artifacts[0].members[0].path = path.to_string();
            assert_rejected(manifest, expected);
        }

        let mut manifest = valid_manifest();
        manifest.artifacts[0].members.push(UpgradeArtifactMember {
            path: "bin/scryer".to_string(),
            size: 1,
            executable: false,
        });
        assert_rejected(manifest, "duplicate member path");

        let mut manifest = valid_manifest();
        manifest.artifacts[0].members.insert(
            0,
            UpgradeArtifactMember {
                path: "z-last".to_string(),
                size: 1,
                executable: false,
            },
        );
        assert_rejected(manifest, "members must be sorted");
    }

    #[test]
    fn rejects_zip_as_an_upgrade_container() {
        let mut value = serde_json::to_value(valid_manifest()).expect("serialize manifest");
        value["artifacts"][3]["archive"] = serde_json::Value::String("zip".to_string());
        let raw = serde_json::to_vec(&value).expect("serialize JSON");
        let error = parse_and_validate_upgrade_manifest(&raw).expect_err("zip is not a container");
        assert!(
            error.to_string().contains("invalid upgrade manifest JSON"),
            "expected a JSON decode failure, got '{error}'"
        );
    }

    #[test]
    fn rejects_malformed_blake3_values() {
        let mut manifest = valid_manifest();
        manifest.artifacts[0].blake3 = "abc".to_string();
        assert_rejected(manifest, "64 lowercase hexadecimal");

        let mut manifest = valid_manifest();
        manifest.artifacts[0].blake3 =
            "A23456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string();
        assert_rejected(manifest, "64 lowercase hexadecimal");
    }

    #[test]
    fn accepts_the_golden_fixture() {
        let raw = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../api/upgrade/manifest.v1.example.json"
        ));
        parse_and_validate_upgrade_manifest(raw).expect("golden fixture is valid");
    }

    #[test]
    fn release_signer_is_pinned_to_the_release_workflow() {
        let signer = scryer_release_required_signer("scryer-v0.19.4");
        assert_eq!(signer.github_repository, SCRYER_RELEASE_REPOSITORY);
        assert_eq!(
            signer.github_workflow.as_deref(),
            Some(SCRYER_RELEASE_WORKFLOW)
        );
        assert_eq!(
            signer.github_ref.as_deref(),
            Some("refs/tags/scryer-v0.19.4")
        );
    }
}

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use chrono::{DateTime, NaiveDate, Utc};
use clap::{Args, Parser, Subcommand, ValueEnum};
use const_oid::db::rfc5280::ID_KP_CODE_SIGNING;
use flate2::read::GzDecoder;
use rustls_pki_types::{CertificateDer, TrustAnchor, UnixTime};
#[cfg(test)]
use scryer_application::application_upgrade::manifest::parse_and_validate_upgrade_manifest;
use scryer_application::{
    PluginDescriptorLoader,
    application_upgrade::manifest::{
        UPGRADE_MANIFEST_SCHEMA_VERSION, UpgradeArchitecture, UpgradeArchive, UpgradeArtifact,
        UpgradeArtifactMember, UpgradeChannel, UpgradeManifest, UpgradePlatform,
    },
};
use scryer_plugins::WasmPluginDescriptorLoader;
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sigstore::{
    cosign::{CosignCapabilities, bundle::SignedArtifactBundle},
    crypto::{CosignVerificationKey, SigningScheme},
    trust::{TrustRoot, sigstore::SigstoreTrustRoot},
};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, OnceLock, mpsc};
use std::thread;
use std::time::{Duration, Instant};
use toml::Value as TomlValue;
use toml_edit::{DocumentMut, value};
use webpki::{EndEntityCert, KeyUsage};
use x509_cert::{
    Certificate,
    der::{Decode, DecodePem, Encode},
    ext::{
        Extension,
        pkix::{SubjectAltName, name::GeneralName},
    },
};
use xtask_support::{
    BOLD, GREEN, RESET, TaskContext, YELLOW, command_available, ok, prefixed_ok, prefixed_step,
    require_command, run_capture, run_checked, run_streaming, step, warn,
};

const PLUGIN_SDK_PACKAGE: &str = "scryer-plugin-sdk";
const PLUGIN_SDK_TAG_PREFIX: &str = "plugin-sdk-v";
const PLUGIN_SDK_MANIFEST: &str = "crates/scryer-plugin-sdk/Cargo.toml";
const PLUGIN_SDK_LIB: &str = "crates/scryer-plugin-sdk/src/lib.rs";
const SCRYER_PROD_PACKAGES: &[&str] = &[
    "scryer",
    "scryer-application",
    "scryer-domain",
    "scryer-infrastructure-acquisition",
    "scryer-infrastructure-configuration",
    "scryer-infrastructure-crypto",
    "scryer-infrastructure-datastore",
    "scryer-infrastructure-identity",
    "scryer-infrastructure-library",
    "scryer-infrastructure-library-search",
    "scryer-infrastructure-metadata",
    "scryer-infrastructure-notifications",
    "scryer-infrastructure-runtime",
    "scryer-infrastructure-sql",
    "scryer-infrastructure-workflow",
    "scryer-interface",
    "scryer-mediainfo",
    "scryer-outbound-http",
    "scryer-plugins",
    "scryer-release-parser",
    "scryer-rules",
    "scryer-tunnel",
];
const SCRYER_CI_CLIPPY_PACKAGES: &[&str] = &[
    "scryer",
    "scryer-application",
    "scryer-domain",
    "scryer-infrastructure-acquisition",
    "scryer-infrastructure-configuration",
    "scryer-infrastructure-crypto",
    "scryer-infrastructure-datastore",
    "scryer-infrastructure-identity",
    "scryer-infrastructure-library",
    "scryer-infrastructure-library-search",
    "scryer-infrastructure-metadata",
    "scryer-infrastructure-notifications",
    "scryer-infrastructure-runtime",
    "scryer-infrastructure-sql",
    "scryer-infrastructure-workflow",
    "scryer-interface",
    "scryer-interface-acquisition",
    "scryer-interface-core",
    "scryer-interface-import",
    "scryer-interface-media",
    "scryer-interface-metadata",
    "scryer-interface-query",
    "scryer-interface-security",
    "scryer-interface-settings",
    "scryer-interface-subscription",
    "scryer-interface-system",
    "scryer-mediainfo",
    "scryer-outbound-http",
    "scryer-plugins",
    "scryer-release-parser",
    "scryer-rules",
    "scryer-tunnel",
];
const RELEASE_DRY_RUN_CACHE_FILE: &str = "tmp/xtask-release-dry-run.json";
const TRASH_GUIDES_SYNC_TIMEOUT: Duration = Duration::from_secs(60);
const TRASH_GUIDES_GENERATED_PATHS: &[&str] = &[
    "crates/scryer-application/src/quality/trash_guides_release_groups.generated.rs",
    "crates/scryer-release-parser/src/trash_guides_parser_knowledge.generated.rs",
    "xtask-trash-guides/generated/latest-summary.txt",
];
const RELEASE_DRY_RUN_BUILTINS_DIR: &str = "tmp/xtask-release-dry-run-builtins";
const RELEASE_NOTES_DIR: &str = "release-notes";
const RELEASE_NOTES_AI_MARKER: &str = "AI generated release notes";
const RELEASE_NOTES_DEFAULT_CODEX_MODEL: &str = "gpt-5.4";
const RELEASE_NOTES_DEFAULT_CODEX_REASONING: &str = "xhigh";
const OFFICIAL_PLUGIN_CATALOG_V3_REDIRECT_URL: &str =
    "https://cdn.scryer.media/scryer/catalog/v3/catalog-v3.redirect.json";
const OFFICIAL_PLUGIN_CATALOG_V3_REDIRECT_BUNDLE_URL: &str =
    "https://cdn.scryer.media/scryer/catalog/v3/catalog-v3.redirect.bundle.json";
const BUILTIN_ASSET_DIR: &str = "crates/scryer-plugins/builtins";
const BUILTIN_VERSION_MANIFEST: &str = "crates/scryer-plugins/builtin-versions.json";
const SIGSTORE_TRUST_ROOT_FILENAME: &str = "sigstore-trusted-root.json";
const SIGSTORE_TRUST_ROOT_PROVENANCE_FILENAME: &str = "sigstore-trusted-root.provenance.json";
const SIGSTORE_TRUST_ROOT_SOURCE: &str = "https://tuf-repo-cdn.sigstore.dev";
const SIGSTORE_TRUST_ROOT_TARGET: &str = "trusted_root.json";
const SIGSTORE_TRUST_ROOT_TIMEOUT: Duration = Duration::from_secs(120);
const OFFICIAL_PLUGIN_REPO: &str = "scryer-media/scryer-plugins";
const OFFICIAL_PLUGIN_V3_RELEASE_WORKFLOW: &str = ".github/workflows/release-plugin-v3.yml";
const SIGSTORE_GITHUB_WORKFLOW_NAME_OID: &str = "1.3.6.1.4.1.57264.1.4";
const SIGSTORE_GITHUB_WORKFLOW_REPOSITORY_OID: &str = "1.3.6.1.4.1.57264.1.5";
const SIGSTORE_GITHUB_WORKFLOW_REF_OID: &str = "1.3.6.1.4.1.57264.1.6";
const RELEASE_LOCAL_PATH_TOKENS: &[&str] = &["~/", "/Users/", "/home/", "C:\\Users\\", "C:/Users/"];
const RELEASE_MACOS_HOME_PATH_COMPONENTS: &[&str] = &[
    "Applications",
    "Desktop",
    "Documents",
    "Downloads",
    "Library",
    "Movies",
    "Music",
    "Pictures",
    "Public",
    "bin",
    "code",
    "dev",
    "src",
    "work",
    "workspace",
];
const RELEASE_SIBLING_E2E_TOKENS: &[&str] = &["../e2e/", "..\\e2e\\"];
const RELEASE_LOCAL_PATH_ALLOWLIST_PREFIXES: &[&str] = &[".github/workflows/"];
const RELEASE_LOCAL_PATH_ALLOWLIST_FILES: &[&str] = &[
    "docker/scryer-e2e-entrypoint.sh",
    "docker/scryer-e2e.Dockerfile",
    "xtask-release/src/main.rs",
];
const RELEASE_SIBLING_E2E_ALLOWLIST_FILES: &[&str] = &["xtask-release/src/main.rs"];
const GRAPHQL_API_COMPAT_STEP: &str = "graphql_api_compat";
const GRAPHQL_API_BASELINE_VERSION: &str = "0.16.3";
const GRAPHQL_SCHEMA_ARTIFACT: &str = "api/graphql/schema.graphql";
const GRAPHQL_SCHEMA_EXPORT_DIR: &str = "target/xtask-release/graphql";
const WINGET_PACKAGE_IDENTIFIER: &str = "ScryerMedia.Scryer";
const WINGET_PACKAGE_NAME: &str = "Scryer";
const WINGET_MONIKER: &str = "scryer";
const WINGET_MANIFEST_VERSION: &str = "1.10.0";
const WINGET_WINDOWS_X64_ASSET: &str = "scryer-windows-x86_64-winget.msi";
const WINGET_WINDOWS_ARM64_ASSET: &str = "scryer-windows-arm64-winget.msi";
const WINGET_WINDOWS_X64_METADATA: &str = "scryer-windows-x86_64-winget.msi.json";
const WINGET_WINDOWS_ARM64_METADATA: &str = "scryer-windows-arm64-winget.msi.json";
const REQUIRED_SCRYER_DRY_RUN_STEPS: &[&str] = &[
    "release_container_contract",
    "builtin_refresh",
    "web_validation",
    "rust_validation",
    GRAPHQL_API_COMPAT_STEP,
    "release_hygiene",
];

type RekorVerificationKeys = BTreeMap<String, CosignVerificationKey>;
type FulcioTrustAnchors = Vec<TrustAnchor<'static>>;

static REKOR_VERIFICATION_KEYS: OnceLock<Result<Arc<RekorVerificationKeys>, String>> =
    OnceLock::new();
static FULCIO_TRUST_ANCHORS: OnceLock<Result<Arc<FulcioTrustAnchors>, String>> = OnceLock::new();

struct BuiltinPluginSpec {
    plugin_id: &'static str,
    artifact_stem: &'static str,
}

/// One built-in's pin: the version *and* the content.
///
/// The digest is TOFU'd from the signed catalog at sync time and asserted on
/// every materialization thereafter. The signature chain proves who built
/// the bytes; this proves they are the bytes the repo reviewed — a catalog
/// that re-points the same version at new artifacts fails the build instead
/// of materializing silently. `cargo xtask builtins sync` refreshes both
/// fields together, so a bump is always a visible diff.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct BuiltinVersionPin {
    version: String,
    /// Bare lowercase blake3 hex of the decompressed WASM artifact.
    wasm_blake3: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct BuiltinVersionManifest {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    plugins: BTreeMap<String, BuiltinVersionPin>,
}

/// Digest equality across the two spellings in play: the catalog and
/// `sync_builtin_plugin` carry `blake3:<hex>`, the manifest pins bare hex.
fn builtin_wasm_digest_matches(pinned: &str, actual: &str) -> bool {
    let normalize = |value: &str| {
        value
            .trim()
            .trim_start_matches("blake3:")
            .to_ascii_lowercase()
    };
    normalize(pinned) == normalize(actual)
}

#[derive(Clone, Debug, Deserialize)]
struct RequiredSigner {
    github_repository: String,
    #[serde(default)]
    github_workflow: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct CatalogV3Redirect {
    artifacts: Vec<CatalogV3CatalogArtifact>,
}

#[derive(Clone, Debug, Deserialize)]
struct CatalogV3CatalogArtifact {
    url: String,
    signature_url: String,
}

#[derive(Clone, Debug, Deserialize)]
struct CatalogV3 {
    plugins: Vec<CatalogV3PluginEntry>,
}

#[derive(Clone, Debug, Deserialize)]
struct CatalogV3PluginEntry {
    id: String,
    description: String,
    required_signer: RequiredSigner,
    releases: Vec<CatalogV3Release>,
}

#[derive(Clone, Debug, Deserialize)]
struct CatalogV3Release {
    version: String,
    #[serde(default)]
    min_scryer_version: Option<String>,
    #[serde(default)]
    max_scryer_version: Option<String>,
    #[serde(default)]
    sdk_constraint: Option<String>,
    artifacts: Vec<CatalogV3PluginArtifact>,
}

#[derive(Clone, Debug, Deserialize)]
struct CatalogV3PluginArtifact {
    runtime: String,
    #[serde(default)]
    required_features: Vec<String>,
    url: String,
    signature_url: String,
    #[serde(default)]
    digests: Vec<String>,
    #[serde(default)]
    wasm_digests: Vec<String>,
}

const BUILTIN_PLUGINS: &[BuiltinPluginSpec] = &[
    BuiltinPluginSpec {
        plugin_id: "newznab",
        artifact_stem: "newznab_indexer",
    },
    BuiltinPluginSpec {
        plugin_id: "torznab",
        artifact_stem: "torznab_indexer",
    },
];

#[derive(Parser)]
#[command(name = "cargo xtask-release")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Release(ReleaseArgs),
    Builtins(BuiltinsArgs),
    Sdk(SdkArgs),
    Ci(CiArgs),
}

#[derive(Args)]
struct BuiltinsArgs {
    #[command(subcommand)]
    command: BuiltinsCommand,
}

#[derive(Subcommand)]
enum BuiltinsCommand {
    Sync,
    Materialize,
}

#[derive(Args)]
struct ReleaseArgs {
    #[arg(long, conflicts_with_all = ["minor", "patch", "version"])]
    major: bool,
    #[arg(long, conflicts_with_all = ["major", "patch", "version"])]
    minor: bool,
    #[arg(long, conflicts_with_all = ["major", "minor", "version"])]
    patch: bool,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    allow_graphql_dangerous: bool,
    version: Option<String>,
}

#[derive(Args)]
struct SdkArgs {
    #[command(subcommand)]
    command: SdkCommand,
}

#[derive(Subcommand)]
enum SdkCommand {
    Release(SdkReleaseArgs),
}

#[derive(Args)]
struct SdkReleaseArgs {
    version: String,
    #[arg(long)]
    dry_run: bool,
}

#[derive(Args)]
struct CiArgs {
    #[command(subcommand)]
    command: CiCommand,
}

#[derive(Subcommand)]
enum CiCommand {
    Clippy(ClippyArgs),
    UpgradeManifest(UpgradeManifestArgs),
    Winget(WingetArgs),
}

#[derive(Args)]
struct ClippyArgs {
    #[arg(long)]
    linux_only: bool,
}

#[derive(Args)]
struct WingetArgs {
    #[arg(long, help = "Scryer version without the scryer-v prefix")]
    version: String,
    #[arg(long, help = "Release tag that owns the Windows assets")]
    tag: Option<String>,
    #[arg(long, default_value = "scryer-media/scryer")]
    repository: String,
    #[arg(long, default_value = "release-artifacts")]
    artifacts_dir: PathBuf,
    #[arg(long, default_value = "target/winget")]
    output_dir: PathBuf,
    #[arg(long, help = "Release date in YYYY-MM-DD format; defaults to today")]
    release_date: Option<String>,
}

#[derive(Args)]
struct UpgradeManifestArgs {
    #[arg(long, help = "Scryer version in major.minor.patch form")]
    version: String,
    #[arg(long, help = "Release tag that owns the upgrade assets")]
    tag: String,
    #[arg(long, default_value = "scryer-media/scryer")]
    repository: String,
    #[arg(long, default_value = "release-artifacts")]
    artifacts_dir: PathBuf,
    #[arg(long, help = "Destination JSON file")]
    output: PathBuf,
}

#[derive(Copy, Clone, Eq, PartialEq, ValueEnum)]
enum VersionBump {
    Patch,
    Minor,
    Major,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReleaseValidationScope {
    Full,
    WebOnly,
    MetaOnly,
}

impl ReleaseValidationScope {
    fn label(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::WebOnly => "web-only",
            Self::MetaOnly => "CI/docs-only",
        }
    }

    fn required_steps(self) -> &'static [&'static str] {
        match self {
            Self::Full => REQUIRED_SCRYER_DRY_RUN_STEPS,
            Self::WebOnly => &[
                "release_container_contract",
                "web_validation",
                "release_hygiene",
            ],
            Self::MetaOnly => &["release_container_contract", "release_hygiene"],
        }
    }
}

fn release_meta_path(path: &Path) -> bool {
    path.starts_with(".github")
        || path.starts_with("docs")
        || path.starts_with("release-notes")
        || path.extension().is_some_and(|extension| extension == "md")
        || matches!(
            path.to_str(),
            Some(
                "LICENSE"
                    | "NOTICE"
                    | "CODEOWNERS"
                    | ".editorconfig"
                    | ".gitignore"
                    | ".gitattributes"
                    | ".markdownlint.json"
                    | "renovate.json"
                    | "dependabot.yml"
            )
        )
}

fn release_web_path(path: &Path) -> bool {
    path.starts_with("apps/scryer-web")
}

fn classify_release_validation_paths<'a>(
    paths: impl IntoIterator<Item = &'a Path>,
) -> ReleaseValidationScope {
    let paths = paths.into_iter().collect::<Vec<_>>();
    if paths.is_empty() {
        return ReleaseValidationScope::MetaOnly;
    }
    if paths.iter().all(|path| release_meta_path(path)) {
        return ReleaseValidationScope::MetaOnly;
    }
    if paths
        .iter()
        .all(|path| release_meta_path(path) || release_web_path(path))
        && paths.iter().any(|path| release_web_path(path))
    {
        return ReleaseValidationScope::WebOnly;
    }
    ReleaseValidationScope::Full
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ReleaseDryRunCache {
    success: bool,
    created_at: String,
    git_commit: String,
    branch: String,
    worktree_clean_at_start: bool,
    release_args: String,
    latest_tag_seen: Option<String>,
    next_version: String,
    tag_name: String,
    catalog_url: String,
    validated_steps: Vec<String>,
    cached_builtins_dir: Option<String>,
    #[serde(default)]
    release_notes_path: Option<String>,
    #[serde(default)]
    release_notes_sha256: Option<String>,
    #[serde(default)]
    catalog_builtin_wasm_blake3: BTreeMap<String, String>,
    failure_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReleaseDryRunExpectations<'a> {
    git_commit: &'a str,
    validation_scope: ReleaseValidationScope,
    release_args: &'a str,
    latest_tag_seen: Option<&'a str>,
    next_version: &'a str,
    tag_name: &'a str,
    release_notes_path: &'a str,
    release_notes_sha256: &'a str,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let ctx = TaskContext::new();

    match cli.command {
        Commands::Release(args) => run_release(&ctx, args),
        Commands::Builtins(args) => match args.command {
            BuiltinsCommand::Sync => {
                let scryer_version = package_version(&ctx.path("crates/scryer/Cargo.toml"))?;
                refresh_builtin_plugins(&ctx, &scryer_version, None, true)?;
                Ok(())
            }
            BuiltinsCommand::Materialize => {
                let scryer_version = package_version(&ctx.path("crates/scryer/Cargo.toml"))?;
                let manifest = load_builtin_version_manifest(&ctx)?;
                refresh_builtin_plugins(&ctx, &scryer_version, Some(&manifest), false)?;
                Ok(())
            }
        },
        Commands::Sdk(args) => match args.command {
            SdkCommand::Release(args) => run_sdk_release(&ctx, args),
        },
        Commands::Ci(args) => match args.command {
            CiCommand::Clippy(args) => run_clippy_ci(&ctx, args),
            CiCommand::UpgradeManifest(args) => run_ci_upgrade_manifest(&ctx, args),
            CiCommand::Winget(args) => run_ci_winget(&ctx, args),
        },
    }
}

fn git_capture(ctx: &TaskContext, args: &[&str]) -> Result<String> {
    let mut command = ctx.command_in("git", &ctx.repo_root);
    command.args(args);
    run_capture(&mut command)
}

fn git_status_porcelain(ctx: &TaskContext) -> Result<String> {
    git_capture(ctx, &["status", "--porcelain"])
}

fn git_tracked_dirty_paths(ctx: &TaskContext) -> Result<Vec<PathBuf>> {
    let mut command = ctx.command_in("git", &ctx.repo_root);
    command.args(["diff", "--name-only", "HEAD", "--"]);
    let output = run_capture(&mut command)?;
    Ok(output
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| ctx.path(line))
        .collect())
}

fn git_tracked_files(ctx: &TaskContext) -> Result<Vec<PathBuf>> {
    let mut command = ctx.command_in("git", &ctx.repo_root);
    command.args(["ls-files", "-z"]);
    let debug = format!("{command:?}");
    let output = command.output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("command failed: {debug}\n{stderr}");
    }

    Ok(output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| ctx.path(String::from_utf8_lossy(entry).as_ref()))
        .collect())
}

fn git_path_output(ctx: &TaskContext, args: &[&str]) -> Result<Vec<PathBuf>> {
    let mut command = ctx.command_in("git", &ctx.repo_root);
    command.args(args);
    let debug = format!("{command:?}");
    let output = command.output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("command failed: {debug}\n{stderr}");
    }

    Ok(output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| ctx.path(String::from_utf8_lossy(entry).as_ref()))
        .collect())
}

fn release_changed_paths(ctx: &TaskContext, latest_tag: Option<&str>) -> Result<Vec<PathBuf>> {
    let Some(latest_tag) = latest_tag else {
        return git_tracked_files(ctx);
    };
    let range = format!("{latest_tag}..HEAD");
    let mut paths = BTreeSet::new();
    for changed in [
        git_path_output(ctx, &["diff", "--name-only", "-z", &range])?,
        git_path_output(ctx, &["diff", "--name-only", "-z"])?,
        git_path_output(ctx, &["diff", "--cached", "--name-only", "-z"])?,
        git_path_output(ctx, &["ls-files", "--others", "--exclude-standard", "-z"])?,
    ] {
        paths.extend(changed);
    }
    Ok(paths.into_iter().collect())
}

fn release_validation_scope(
    ctx: &TaskContext,
    latest_tag: Option<&str>,
) -> Result<ReleaseValidationScope> {
    let paths = release_changed_paths(ctx, latest_tag)?;
    Ok(classify_release_validation_paths(
        paths.iter().map(PathBuf::as_path),
    ))
}

fn git_tracked_cargo_lockfiles(ctx: &TaskContext) -> Result<Vec<PathBuf>> {
    let mut lockfiles = git_tracked_files(ctx)?
        .into_iter()
        .filter(|path| path.file_name().is_some_and(|name| name == "Cargo.lock"))
        .collect::<Vec<_>>();
    lockfiles.sort();
    Ok(lockfiles)
}

fn scan_release_hygiene_content(path: &Path, content: &str) -> Vec<String> {
    let mut violations = Vec::new();
    let path_text = path.to_string_lossy();

    for (line_number, line) in content.lines().enumerate() {
        let line_number = line_number + 1;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if release_hygiene_line_has_local_path_token(line)
            && !release_hygiene_path_is_allowlisted(
                &path_text,
                RELEASE_LOCAL_PATH_ALLOWLIST_PREFIXES,
                RELEASE_LOCAL_PATH_ALLOWLIST_FILES,
            )
        {
            violations.push(format!(
                "{}:{line_number}: local absolute path reference: {trimmed}",
                path.display()
            ));
        }

        if RELEASE_SIBLING_E2E_TOKENS
            .iter()
            .any(|token| line.contains(token))
            && !release_hygiene_path_is_allowlisted(
                &path_text,
                &[],
                RELEASE_SIBLING_E2E_ALLOWLIST_FILES,
            )
        {
            violations.push(format!(
                "{}:{line_number}: sibling e2e repo reference: {trimmed}",
                path.display()
            ));
        }
    }

    violations
}

fn release_hygiene_line_has_local_path_token(line: &str) -> bool {
    RELEASE_LOCAL_PATH_TOKENS.iter().any(|token| {
        if *token == "/Users/" {
            line.split(token).skip(1).any(|tail| {
                tail.split_once('/').is_some_and(|(_, after_user)| {
                    let Some((component, _)) = after_user.split_once('/') else {
                        return false;
                    };
                    RELEASE_MACOS_HOME_PATH_COMPONENTS.contains(&component)
                })
            })
        } else if *token == "/home/" {
            line.split(token).skip(1).any(|tail| {
                tail.split(|character: char| {
                    !character.is_ascii_alphanumeric() && character != '-' && character != '_'
                })
                .next()
                .is_none_or(|component| component != "linuxbrew")
            })
        } else {
            line.contains(token)
        }
    })
}

fn release_hygiene_path_is_allowlisted(
    path_text: &str,
    prefix_allowlist: &[&str],
    file_allowlist: &[&str],
) -> bool {
    prefix_allowlist
        .iter()
        .any(|prefix| path_text.starts_with(prefix))
        || file_allowlist.contains(&path_text)
}

fn release_hygiene_violations(ctx: &TaskContext) -> Result<Vec<String>> {
    let mut violations = Vec::new();

    for path in git_tracked_files(ctx)? {
        let Ok(bytes) = fs::read(&path) else {
            continue;
        };
        if bytes.contains(&0) {
            continue;
        }

        let relative = path
            .strip_prefix(&ctx.repo_root)
            .unwrap_or(path.as_path())
            .to_path_buf();
        let content = String::from_utf8_lossy(&bytes);
        violations.extend(scan_release_hygiene_content(&relative, &content));
    }

    violations.sort();
    Ok(violations)
}

fn latest_prefixed_tag(ctx: &TaskContext, prefix: &str) -> Result<Option<String>> {
    let tags = git_capture(ctx, &["tag", "--sort=-version:refname"])?;
    Ok(tags
        .lines()
        .find(|line| line.starts_with(prefix))
        .map(ToOwned::to_owned))
}

fn current_branch(ctx: &TaskContext) -> Result<String> {
    git_capture(ctx, &["rev-parse", "--abbrev-ref", "HEAD"]).map(|value| value.trim().to_string())
}

fn require_app_release_branch(branch: &str, version: &Version) -> Result<()> {
    let expected = format!("release-{version}");
    if branch != expected {
        bail!(
            "Scryer application releases must run from {expected}; current branch is {branch}. \
             Create {expected} from the current main branch, run the release there, then merge it into main."
        );
    }
    Ok(())
}

fn require_main_merged(ctx: &TaskContext) -> Result<()> {
    let mut command = ctx.command_in("git", &ctx.repo_root);
    command.args(["merge-base", "--is-ancestor", "main", "HEAD"]);
    let status = command.status()?;
    if status.success() {
        return Ok(());
    }
    bail!(
        "release branch does not contain main; merge main into the release branch before running a release dry run"
    );
}

fn current_head_commit(ctx: &TaskContext) -> Result<String> {
    git_capture(ctx, &["rev-parse", "HEAD"]).map(|value| value.trim().to_string())
}

fn prompt_continue_if_dirty(ctx: &TaskContext) -> Result<()> {
    let status = git_status_porcelain(ctx)?;
    if status.trim().is_empty() {
        return Ok(());
    }

    warn("Working tree has uncommitted changes:");
    for line in status.lines() {
        eprintln!("     {line}");
    }
    eprint!("\n   Continue anyway? [y/N] ");
    io::stderr().flush()?;

    let mut response = String::new();
    io::stdin().read_line(&mut response)?;
    let response = response.trim();
    if !matches!(response, "y" | "Y") {
        bail!("aborted");
    }
    Ok(())
}

fn release_args_signature(
    explicit: Option<&Version>,
    bump: VersionBump,
    allow_graphql_dangerous: bool,
) -> String {
    let mut signature = explicit.map_or_else(
        || format!("bump:{}", version_bump_label(bump)),
        |version| format!("version:{version}"),
    );
    if allow_graphql_dangerous {
        signature.push_str(";allow-graphql-dangerous");
    }
    signature
}

fn version_bump_label(bump: VersionBump) -> &'static str {
    match bump {
        VersionBump::Patch => "patch",
        VersionBump::Minor => "minor",
        VersionBump::Major => "major",
    }
}

fn parse_bump(args: &ReleaseArgs) -> Result<(VersionBump, Option<Version>)> {
    let explicit = match &args.version {
        Some(version) => Some(Version::parse(version.trim_start_matches('v'))?),
        None => None,
    };
    let bump = if args.major {
        VersionBump::Major
    } else if args.minor {
        VersionBump::Minor
    } else {
        VersionBump::Patch
    };
    Ok((bump, explicit))
}

fn release_dry_run_cache_path(ctx: &TaskContext) -> PathBuf {
    ctx.path(RELEASE_DRY_RUN_CACHE_FILE)
}

fn release_dry_run_builtins_root(ctx: &TaskContext) -> PathBuf {
    ctx.path(RELEASE_DRY_RUN_BUILTINS_DIR)
}

fn release_dry_run_cache_fingerprint(
    git_commit: &str,
    release_args: &str,
    latest_tag_seen: Option<&str>,
    next_version: &Version,
    tag_name: &str,
) -> String {
    sha256_hex(
        format!(
            "{}\n{}\n{}\n{}\n{}",
            git_commit,
            release_args,
            latest_tag_seen.unwrap_or(""),
            next_version,
            tag_name
        )
        .as_bytes(),
    )
}

fn release_dry_run_cache_dir(
    ctx: &TaskContext,
    git_commit: &str,
    release_args: &str,
    latest_tag_seen: Option<&str>,
    next_version: &Version,
    tag_name: &str,
) -> PathBuf {
    release_dry_run_builtins_root(ctx).join(release_dry_run_cache_fingerprint(
        git_commit,
        release_args,
        latest_tag_seen,
        next_version,
        tag_name,
    ))
}

fn relative_to_repo_root(ctx: &TaskContext, path: &Path) -> Result<String> {
    path.strip_prefix(&ctx.repo_root)
        .with_context(|| format!("{} is not under repo root", path.display()))
        .map(|relative| relative.to_string_lossy().into_owned())
}

fn clear_release_dry_run_cache(ctx: &TaskContext) -> Result<()> {
    let cache_path = release_dry_run_cache_path(ctx);
    if cache_path.exists() {
        fs::remove_file(&cache_path)
            .with_context(|| format!("failed to remove {}", cache_path.display()))?;
    }

    let builtins_root = release_dry_run_builtins_root(ctx);
    if builtins_root.exists() {
        fs::remove_dir_all(&builtins_root)
            .with_context(|| format!("failed to remove {}", builtins_root.display()))?;
    }

    Ok(())
}

fn write_release_dry_run_cache(ctx: &TaskContext, cache: &ReleaseDryRunCache) -> Result<()> {
    let path = release_dry_run_cache_path(ctx);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(&path, serde_json::to_string_pretty(cache)? + "\n")
        .with_context(|| format!("failed to write {}", path.display()))
}

fn canonicalize_json_value(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.into_iter().map(canonicalize_json_value).collect())
        }
        serde_json::Value::Object(map) => {
            let ordered = map
                .into_iter()
                .map(|(key, value)| (key, canonicalize_json_value(value)))
                .collect::<std::collections::BTreeMap<_, _>>();
            let mut canonical = serde_json::Map::with_capacity(ordered.len());
            for (key, value) in ordered {
                canonical.insert(key, value);
            }
            serde_json::Value::Object(canonical)
        }
        other => other,
    }
}

fn canonical_pretty_json<T: Serialize>(value: &T) -> Result<String> {
    let canonical = canonicalize_json_value(serde_json::to_value(value)?);
    Ok(serde_json::to_string_pretty(&canonical)? + "\n")
}

fn load_release_dry_run_cache(ctx: &TaskContext) -> Result<ReleaseDryRunCache> {
    let path = release_dry_run_cache_path(ctx);
    let raw =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_str(&raw).with_context(|| format!("failed to parse {}", path.display()))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join("")
}

fn release_notes_path(ctx: &TaskContext, tag_name: &str) -> PathBuf {
    ctx.path(RELEASE_NOTES_DIR).join(format!("{tag_name}.md"))
}

fn release_notes_path_relative(tag_name: &str) -> String {
    format!("{RELEASE_NOTES_DIR}/{tag_name}.md")
}

fn release_notes_context_path(ctx: &TaskContext, tag_name: &str) -> PathBuf {
    ctx.path("tmp")
        .join("xtask-release-notes")
        .join(format!("{tag_name}-context.md"))
}

fn release_notes_output_path(ctx: &TaskContext, tag_name: &str) -> PathBuf {
    ctx.path("tmp")
        .join("xtask-release-notes")
        .join(format!("{tag_name}-output.md"))
}

fn release_notes_sha256(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| {
        format!(
            "failed to read release notes for checksum at {}",
            path.display()
        )
    })?;
    Ok(sha256_hex(&bytes))
}

fn release_notes_context(
    ctx: &TaskContext,
    latest_tag: Option<&str>,
    tag_name: &str,
    next_version: &Version,
) -> Result<String> {
    let range = latest_tag
        .map(|tag| format!("{tag}..HEAD"))
        .unwrap_or_else(|| "HEAD".to_string());
    let commit_log = git_capture(
        ctx,
        &[
            "log",
            "--no-merges",
            "--no-show-signature",
            "--date=short",
            "--pretty=format:%h %ad %s",
            &range,
        ],
    )
    .unwrap_or_else(|_| String::new());
    let changed_files = if let Some(tag) = latest_tag {
        git_capture(ctx, &["diff", "--name-status", &format!("{tag}..HEAD")])
            .unwrap_or_else(|_| String::new())
    } else {
        git_capture(ctx, &["ls-files"]).unwrap_or_else(|_| String::new())
    };
    let diffstat = if latest_tag.is_some() {
        git_capture(ctx, &["diff", "--stat", &range]).unwrap_or_else(|_| String::new())
    } else {
        String::new()
    };

    Ok(format!(
        r#"# Release Notes Generation Context

Proposed tag: {tag_name}
Proposed version: {next_version}
Previous tag: {previous_tag}
Commit range: {range}

## Required output contract

- Write Markdown only.
- The first line must be exactly `# {tag_name}`.
- Include this line near the top: `{RELEASE_NOTES_AI_MARKER}`.
- Summarize user-facing changes first.
- Keep wording suitable for a GitHub Release.
- Do not mention local filesystem paths.
- Do not include placeholder text.
- Do not use the word `placeholder`; describe missing data directly.

## Commit log

```text
{commit_log}
```

## Changed files

```text
{changed_files}
```

## Diffstat

```text
{diffstat}
```
"#,
        previous_tag = latest_tag.unwrap_or("none"),
    ))
}

fn validate_release_notes_document(
    path: &Path,
    tag_name: &str,
    require_ai_marker: bool,
) -> Result<()> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read release notes at {}", path.display()))?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        bail!("release notes are empty");
    }
    let expected_heading = format!("# {tag_name}");
    let prewritten_heading = format!(
        "# Scryer {} release notes",
        tag_name.trim_start_matches("scryer-v")
    );
    let heading = trimmed.lines().next();
    if heading != Some(expected_heading.as_str())
        && (require_ai_marker || heading != Some(prewritten_heading.as_str()))
    {
        let accepted_headings = if require_ai_marker {
            format!("`{expected_heading}`")
        } else {
            format!("`{expected_heading}` or `{prewritten_heading}`")
        };
        bail!("release notes must start with {accepted_headings}");
    }
    if require_ai_marker && !trimmed.contains(RELEASE_NOTES_AI_MARKER) {
        bail!("generated release notes must include `{RELEASE_NOTES_AI_MARKER}`");
    }
    let lower = trimmed.to_ascii_lowercase();
    for placeholder in ["todo:", "tbd", "placeholder", "<insert", "lorem ipsum"] {
        if lower.contains(placeholder) {
            bail!("generated release notes contain placeholder text: {placeholder}");
        }
    }
    let violations = scan_release_hygiene_content(path, &content);
    if !violations.is_empty() {
        bail!(
            "release notes contain release hygiene violations:\n{}",
            violations.join("\n")
        );
    }
    Ok(())
}

fn validate_release_notes_output(path: &Path, tag_name: &str) -> Result<()> {
    validate_release_notes_document(path, tag_name, true)
}

fn validate_prewritten_release_notes(path: &Path, tag_name: &str) -> Result<()> {
    validate_release_notes_document(path, tag_name, false)
}

fn codex_release_notes_command_for(output_path: &Path, model: &str, reasoning: &str) -> Command {
    let reasoning_config = format!("model_reasoning_effort=\"{reasoning}\"");
    let mut command = Command::new("codex");
    command
        .args(["exec", "--ephemeral", "--sandbox", "read-only", "--model"])
        .arg(model)
        .arg("-c")
        .arg(reasoning_config)
        .arg("--output-last-message")
        .arg(output_path)
        .arg("-");
    command
}

fn codex_release_notes_command(output_path: &Path) -> Command {
    let model = std::env::var("SCRYER_RELEASE_NOTES_CODEX_MODEL")
        .unwrap_or_else(|_| RELEASE_NOTES_DEFAULT_CODEX_MODEL.to_string());
    let reasoning = std::env::var("SCRYER_RELEASE_NOTES_CODEX_REASONING")
        .unwrap_or_else(|_| RELEASE_NOTES_DEFAULT_CODEX_REASONING.to_string());
    codex_release_notes_command_for(output_path, &model, &reasoning)
}

#[expect(
    clippy::too_many_arguments,
    reason = "release notes command invocation passes explicit release context into the generator"
)]
fn run_release_notes_command_with_template(
    ctx: &TaskContext,
    context_path: &Path,
    output_path: &Path,
    tag_name: &str,
    latest_tag: Option<&str>,
    next_version: &Version,
    prompt: &str,
    command_template: Option<&str>,
) -> Result<()> {
    if let Some(command_template) = command_template.filter(|value| !value.trim().is_empty()) {
        let status = Command::new("sh")
            .arg("-c")
            .arg(command_template)
            .current_dir(&ctx.repo_root)
            .env("SCRYER_RELEASE_NOTES_CONTEXT", prompt)
            .env(
                "SCRYER_RELEASE_NOTES_CONTEXT_PATH",
                context_path.as_os_str(),
            )
            .env("SCRYER_RELEASE_NOTES_OUTPUT", output_path.as_os_str())
            .env("SCRYER_RELEASE_TAG", tag_name)
            .env("SCRYER_PREVIOUS_RELEASE_TAG", latest_tag.unwrap_or(""))
            .env("SCRYER_RELEASE_VERSION", next_version.to_string())
            .status()
            .context("failed to run SCRYER_RELEASE_NOTES_COMMAND")?;
        if !status.success() {
            bail!("SCRYER_RELEASE_NOTES_COMMAND failed with status {status}");
        }
        return Ok(());
    }

    require_command("codex")?;
    let mut command = codex_release_notes_command(output_path);
    command.current_dir(&ctx.repo_root).stdin(Stdio::piped());
    let mut child = command
        .spawn()
        .context("failed to start Codex release notes generator")?;
    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| anyhow!("failed to open Codex stdin"))?;
        stdin
            .write_all(prompt.as_bytes())
            .context("failed to write release notes prompt to Codex")?;
    }
    let status = child
        .wait()
        .context("failed waiting for Codex release notes generator")?;
    if !status.success() {
        bail!("Codex release notes generator failed with status {status}");
    }
    Ok(())
}

fn run_release_notes_command(
    ctx: &TaskContext,
    context_path: &Path,
    output_path: &Path,
    tag_name: &str,
    latest_tag: Option<&str>,
    next_version: &Version,
    prompt: &str,
) -> Result<()> {
    let command_template = std::env::var("SCRYER_RELEASE_NOTES_COMMAND").ok();
    run_release_notes_command_with_template(
        ctx,
        context_path,
        output_path,
        tag_name,
        latest_tag,
        next_version,
        prompt,
        command_template.as_deref(),
    )
}

fn generate_release_notes(
    ctx: &TaskContext,
    latest_tag: Option<&str>,
    tag_name: &str,
    next_version: &Version,
) -> Result<(PathBuf, String)> {
    let final_path = release_notes_path(ctx, tag_name);
    let context_path = release_notes_context_path(ctx, tag_name);
    let output_path = release_notes_output_path(ctx, tag_name);
    if let Some(parent) = context_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    if let Some(parent) = final_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let prompt = release_notes_context(ctx, latest_tag, tag_name, next_version)?;
    fs::write(&context_path, &prompt)
        .with_context(|| format!("failed to write {}", context_path.display()))?;
    if output_path.exists() {
        fs::remove_file(&output_path)
            .with_context(|| format!("failed to remove {}", output_path.display()))?;
    }

    run_release_notes_command(
        ctx,
        &context_path,
        &output_path,
        tag_name,
        latest_tag,
        next_version,
        &prompt,
    )?;
    validate_release_notes_output(&output_path, tag_name)?;
    fs::copy(&output_path, &final_path).with_context(|| {
        format!(
            "failed to copy generated release notes from {} to {}",
            output_path.display(),
            final_path.display()
        )
    })?;
    validate_release_notes_output(&final_path, tag_name)?;
    let digest = release_notes_sha256(&final_path)?;
    Ok((final_path, digest))
}

#[derive(Clone, Copy)]
struct UpgradeManifestAssetSpec {
    platform: UpgradePlatform,
    arch: UpgradeArchitecture,
    channel: UpgradeChannel,
    archive: UpgradeArchive,
    asset_name: &'static str,
}

const UPGRADE_MANIFEST_ASSETS: [UpgradeManifestAssetSpec; 8] = [
    UpgradeManifestAssetSpec {
        platform: UpgradePlatform::Linux,
        arch: UpgradeArchitecture::X86_64,
        channel: UpgradeChannel::Portable,
        archive: UpgradeArchive::TarGz,
        asset_name: "scryer-linux-x86_64-portable.tar.gz",
    },
    UpgradeManifestAssetSpec {
        platform: UpgradePlatform::Linux,
        arch: UpgradeArchitecture::Arm64,
        channel: UpgradeChannel::Portable,
        archive: UpgradeArchive::TarGz,
        asset_name: "scryer-linux-arm64-portable.tar.gz",
    },
    UpgradeManifestAssetSpec {
        platform: UpgradePlatform::Darwin,
        arch: UpgradeArchitecture::X86_64,
        channel: UpgradeChannel::Portable,
        archive: UpgradeArchive::TarGz,
        asset_name: "scryer-darwin-x86_64-portable.tar.gz",
    },
    UpgradeManifestAssetSpec {
        platform: UpgradePlatform::Darwin,
        arch: UpgradeArchitecture::Arm64,
        channel: UpgradeChannel::Portable,
        archive: UpgradeArchive::TarGz,
        asset_name: "scryer-darwin-arm64-portable.tar.gz",
    },
    // The human-facing `scryer-windows-<arch>.zip` download is deliberately not
    // listed here: the upgrade channel ships the same `.tar.gz` container on
    // every platform so the engine needs exactly one archive reader.
    UpgradeManifestAssetSpec {
        platform: UpgradePlatform::Windows,
        arch: UpgradeArchitecture::X86_64,
        channel: UpgradeChannel::Portable,
        archive: UpgradeArchive::TarGz,
        asset_name: "scryer-windows-x86_64-portable.tar.gz",
    },
    UpgradeManifestAssetSpec {
        platform: UpgradePlatform::Windows,
        arch: UpgradeArchitecture::Arm64,
        channel: UpgradeChannel::Portable,
        archive: UpgradeArchive::TarGz,
        asset_name: "scryer-windows-arm64-portable.tar.gz",
    },
    UpgradeManifestAssetSpec {
        platform: UpgradePlatform::Windows,
        arch: UpgradeArchitecture::X86_64,
        channel: UpgradeChannel::Msi,
        archive: UpgradeArchive::Msi,
        asset_name: "scryer-windows-x86_64.msi",
    },
    UpgradeManifestAssetSpec {
        platform: UpgradePlatform::Windows,
        arch: UpgradeArchitecture::Arm64,
        channel: UpgradeChannel::Msi,
        archive: UpgradeArchive::Msi,
        asset_name: "scryer-windows-arm64.msi",
    },
];

fn run_ci_upgrade_manifest(ctx: &TaskContext, args: UpgradeManifestArgs) -> Result<()> {
    step("Generating signed upgrade manifest");
    let version = normalize_upgrade_manifest_version(&args.version)?;
    let tag = args.tag.trim();
    if tag.is_empty() {
        bail!("upgrade manifest tag must not be empty");
    }
    let repository = normalize_github_repository(&args.repository)?;
    let artifacts_dir = resolve_ci_path(ctx, args.artifacts_dir);
    let output = resolve_ci_path(ctx, args.output);
    let raw = generate_upgrade_manifest(&version, tag, &repository, &artifacts_dir)?;

    if let Some(parent) = output.parent().filter(|path| !path.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(&output, raw).with_context(|| format!("failed to write {}", output.display()))?;
    ok(format!(
        "Generated upgrade manifest at {}",
        output.display()
    ));
    Ok(())
}

fn resolve_ci_path(ctx: &TaskContext, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        ctx.repo_root.join(path)
    }
}

fn normalize_upgrade_manifest_version(raw: &str) -> Result<String> {
    let version = Version::parse(raw.trim()).with_context(|| {
        format!("upgrade manifest version must be major.minor.patch, got {raw}")
    })?;
    if !version.pre.is_empty() || !version.build.is_empty() {
        bail!("upgrade manifest version must not include prerelease or build metadata");
    }
    Ok(version.to_string())
}

fn generate_upgrade_manifest(
    version: &str,
    tag: &str,
    repository: &str,
    artifacts_dir: &Path,
) -> Result<Vec<u8>> {
    let mut artifacts = UPGRADE_MANIFEST_ASSETS
        .iter()
        .copied()
        .map(|spec| collect_upgrade_manifest_artifact(spec, repository, tag, artifacts_dir))
        .collect::<Result<Vec<_>>>()?;
    artifacts.sort_by_key(upgrade_manifest_artifact_sort_key);

    let manifest = UpgradeManifest {
        schema: UPGRADE_MANIFEST_SCHEMA_VERSION.to_string(),
        tag: tag.to_string(),
        version: version.to_string(),
        artifacts,
    };
    let mut raw = serde_json::to_vec_pretty(&manifest)?;
    raw.push(b'\n');
    Ok(raw)
}

fn collect_upgrade_manifest_artifact(
    spec: UpgradeManifestAssetSpec,
    repository: &str,
    tag: &str,
    artifacts_dir: &Path,
) -> Result<UpgradeArtifact> {
    let path = artifacts_dir.join(spec.asset_name);
    let bytes = fs::read(&path).with_context(|| {
        format!(
            "required upgrade manifest asset is missing or unreadable: {}",
            path.display()
        )
    })?;
    let members = match spec.archive {
        UpgradeArchive::TarGz => collect_tar_gz_members(&path, spec.asset_name)?,
        UpgradeArchive::Msi => Vec::new(),
    };

    Ok(UpgradeArtifact {
        platform: spec.platform,
        arch: spec.arch,
        channel: spec.channel,
        asset_name: spec.asset_name.to_string(),
        url: format!(
            "https://github.com/{repository}/releases/download/{tag}/{}",
            spec.asset_name
        ),
        size: bytes.len() as u64,
        blake3: blake3::hash(&bytes).to_hex().to_string(),
        archive: spec.archive,
        members,
    })
}

fn collect_tar_gz_members(path: &Path, asset_name: &str) -> Result<Vec<UpgradeArtifactMember>> {
    let file =
        fs::File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let decoder = GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    let entries = archive
        .entries()
        .with_context(|| format!("failed to read tar archive {}", path.display()))?;
    let mut members = Vec::new();
    for entry in entries {
        let entry =
            entry.with_context(|| format!("failed to read tar entry in {}", path.display()))?;
        let entry_type = entry.header().entry_type();
        if entry_type.is_dir() {
            continue;
        }
        if entry_type.is_symlink() || entry_type.is_hard_link() {
            bail!("upgrade archive {asset_name} contains a link entry");
        }
        if !entry_type.is_file() {
            bail!("upgrade archive {asset_name} contains a non-regular entry");
        }
        let member_path = archive_member_path(entry.path()?.as_ref(), asset_name)?;
        members.push(UpgradeArtifactMember {
            path: member_path,
            size: entry
                .header()
                .size()
                .with_context(|| format!("failed to read tar member size in {asset_name}"))?,
            executable: entry
                .header()
                .mode()
                .with_context(|| format!("failed to read tar member mode in {asset_name}"))?
                & 0o111
                != 0,
        });
    }
    sort_and_validate_archive_members(&mut members, asset_name)?;
    Ok(members)
}

fn archive_member_path(path: &Path, asset_name: &str) -> Result<String> {
    let value = path
        .to_str()
        .ok_or_else(|| anyhow!("upgrade archive {asset_name} contains a non-UTF-8 member path"))?;
    if value.is_empty()
        || value.starts_with('/')
        || path.is_absolute()
        || has_windows_drive_prefix(value)
    {
        bail!("upgrade archive {asset_name} contains an absolute member path: {value}");
    }
    if value.contains('\\') {
        bail!("upgrade archive {asset_name} contains a backslash member path: {value}");
    }
    if path
        .components()
        .any(|component| component == std::path::Component::ParentDir)
    {
        bail!("upgrade archive {asset_name} contains a parent member path: {value}");
    }
    Ok(value.to_string())
}

fn has_windows_drive_prefix(path: &str) -> bool {
    path.as_bytes().get(1) == Some(&b':')
        && path.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
}

fn sort_and_validate_archive_members(
    members: &mut [UpgradeArtifactMember],
    asset_name: &str,
) -> Result<()> {
    members.sort_by(|left, right| left.path.cmp(&right.path));
    if let Some(duplicate) = members
        .windows(2)
        .find(|pair| pair[0].path == pair[1].path)
        .map(|pair| &pair[0].path)
    {
        bail!("upgrade archive {asset_name} contains duplicate member path: {duplicate}");
    }
    Ok(())
}

fn upgrade_manifest_artifact_sort_key(
    artifact: &UpgradeArtifact,
) -> (&'static str, &'static str, &'static str) {
    (
        upgrade_platform_name(artifact.platform),
        upgrade_architecture_name(artifact.arch),
        upgrade_channel_name(artifact.channel),
    )
}

fn upgrade_platform_name(platform: UpgradePlatform) -> &'static str {
    match platform {
        UpgradePlatform::Darwin => "darwin",
        UpgradePlatform::Linux => "linux",
        UpgradePlatform::Windows => "windows",
    }
}

fn upgrade_architecture_name(architecture: UpgradeArchitecture) -> &'static str {
    match architecture {
        UpgradeArchitecture::Arm64 => "arm64",
        UpgradeArchitecture::X86_64 => "x86_64",
    }
}

fn upgrade_channel_name(channel: UpgradeChannel) -> &'static str {
    match channel {
        UpgradeChannel::Msi => "msi",
        UpgradeChannel::Portable => "portable",
    }
}

fn run_ci_winget(ctx: &TaskContext, args: WingetArgs) -> Result<()> {
    step("Preparing WinGet MSI manifests");
    let version = normalize_winget_version(&args.version)?;
    let tag_name = args
        .tag
        .unwrap_or_else(|| format!("scryer-v{version}"))
        .trim()
        .to_string();
    let expected_tag = format!("scryer-v{version}");
    if tag_name != expected_tag {
        bail!("WinGet tag/version mismatch: expected {expected_tag}, got {tag_name}");
    }
    let repository = normalize_github_repository(&args.repository)?;
    let release_date = args
        .release_date
        .unwrap_or_else(|| Utc::now().date_naive().to_string());
    validate_winget_release_date(&release_date)?;

    let artifacts_dir = if args.artifacts_dir.is_absolute() {
        args.artifacts_dir
    } else {
        ctx.repo_root.join(args.artifacts_dir)
    };
    let output_dir = if args.output_dir.is_absolute() {
        args.output_dir
    } else {
        ctx.repo_root.join(args.output_dir)
    };
    let artifacts = collect_winget_artifacts(&repository, &tag_name, &version, &artifacts_dir)?;
    let manifest_dir = write_winget_manifests(&output_dir, &version, &release_date, &artifacts)?;

    ok(format!(
        "Generated WinGet manifests in {}",
        manifest_dir.display()
    ));
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WingetArtifact {
    architecture: &'static str,
    asset_name: &'static str,
    installer_url: String,
    installer_sha256: String,
    product_code: String,
}

#[derive(Debug, Deserialize)]
struct WingetMsiMetadata {
    architecture: String,
    product_code: String,
    version: String,
}

fn normalize_winget_version(raw: &str) -> Result<Version> {
    let trimmed = raw.trim();
    let version = trimmed
        .strip_prefix("scryer-v")
        .or_else(|| trimmed.strip_prefix('v'))
        .unwrap_or(trimmed);
    Version::parse(version).with_context(|| format!("invalid Scryer version: {raw}"))
}

fn normalize_github_repository(raw: &str) -> Result<String> {
    let repository = raw.trim().trim_matches('/').to_string();
    let mut parts = repository.split('/');
    let owner = parts.next().filter(|part| !part.is_empty());
    let name = parts.next().filter(|part| !part.is_empty());
    if owner.is_none() || name.is_none() || parts.next().is_some() || repository.contains("://") {
        bail!("GitHub repository must be owner/name, got {raw}");
    }
    Ok(repository)
}

fn validate_winget_release_date(release_date: &str) -> Result<()> {
    NaiveDate::parse_from_str(release_date, "%Y-%m-%d")
        .with_context(|| format!("release date must be YYYY-MM-DD, got {release_date}"))?;
    Ok(())
}

fn collect_winget_artifacts(
    repository: &str,
    tag_name: &str,
    version: &Version,
    artifacts_dir: &Path,
) -> Result<Vec<WingetArtifact>> {
    let artifacts = [
        ("x64", WINGET_WINDOWS_X64_ASSET, WINGET_WINDOWS_X64_METADATA),
        (
            "arm64",
            WINGET_WINDOWS_ARM64_ASSET,
            WINGET_WINDOWS_ARM64_METADATA,
        ),
    ];

    artifacts
        .into_iter()
        .map(|(architecture, asset_name, metadata_name)| {
            let path = artifacts_dir.join(asset_name);
            validate_winget_msi(&path)?;
            let metadata_path = artifacts_dir.join(metadata_name);
            let metadata: WingetMsiMetadata = serde_json::from_slice(
                &fs::read(&metadata_path)
                    .with_context(|| format!("failed to read {}", metadata_path.display()))?,
            )
            .with_context(|| format!("failed to parse {}", metadata_path.display()))?;
            if metadata.architecture != architecture {
                bail!(
                    "{} architecture must be {architecture}, got {}",
                    metadata_path.display(),
                    metadata.architecture
                );
            }
            if metadata.version != version.to_string() {
                bail!(
                    "{} version must be {version}, got {}",
                    metadata_path.display(),
                    metadata.version
                );
            }
            if !is_msi_product_code(&metadata.product_code) {
                bail!(
                    "{} does not contain a valid MSI ProductCode: {}",
                    metadata_path.display(),
                    metadata.product_code
                );
            }
            let bytes =
                fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
            let installer_sha256 = sha256_hex(&bytes).to_ascii_uppercase();
            let installer_url = format!(
                "https://github.com/{repository}/releases/download/{tag_name}/{asset_name}"
            );
            Ok(WingetArtifact {
                architecture,
                asset_name,
                installer_url,
                installer_sha256,
                product_code: metadata.product_code,
            })
        })
        .collect()
}

fn validate_winget_msi(path: &Path) -> Result<()> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    if bytes.starts_with(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]) {
        return Ok(());
    }
    bail!("{} is not an MSI compound document", path.display())
}

fn is_msi_product_code(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 38
        && bytes.first() == Some(&b'{')
        && bytes.last() == Some(&b'}')
        && bytes[1..37].iter().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                *byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

fn write_winget_manifests(
    output_dir: &Path,
    version: &Version,
    release_date: &str,
    artifacts: &[WingetArtifact],
) -> Result<PathBuf> {
    let manifest_dir = output_dir
        .join(WINGET_PACKAGE_IDENTIFIER)
        .join(version.to_string());
    if manifest_dir.exists() {
        fs::remove_dir_all(&manifest_dir)
            .with_context(|| format!("failed to clear {}", manifest_dir.display()))?;
    }
    fs::create_dir_all(&manifest_dir)
        .with_context(|| format!("failed to create {}", manifest_dir.display()))?;

    write_text_file(
        &manifest_dir.join(format!("{WINGET_PACKAGE_IDENTIFIER}.yaml")),
        &winget_version_manifest(version),
    )?;
    write_text_file(
        &manifest_dir.join(format!("{WINGET_PACKAGE_IDENTIFIER}.locale.en-US.yaml")),
        &winget_locale_manifest(version),
    )?;
    write_text_file(
        &manifest_dir.join(format!("{WINGET_PACKAGE_IDENTIFIER}.installer.yaml")),
        &winget_installer_manifest(version, release_date, artifacts),
    )?;
    Ok(manifest_dir)
}

fn write_text_file(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(path, content).with_context(|| format!("failed to write {}", path.display()))
}

fn winget_version_manifest(version: &Version) -> String {
    format!(
        "# yaml-language-server: $schema=https://aka.ms/winget-manifest.version.{WINGET_MANIFEST_VERSION}.schema.json\n\n\
PackageIdentifier: {WINGET_PACKAGE_IDENTIFIER}\n\
PackageVersion: {version}\n\
DefaultLocale: en-US\n\
ManifestType: version\n\
ManifestVersion: {WINGET_MANIFEST_VERSION}\n"
    )
}

fn winget_locale_manifest(version: &Version) -> String {
    format!(
        "# yaml-language-server: $schema=https://aka.ms/winget-manifest.defaultLocale.{WINGET_MANIFEST_VERSION}.schema.json\n\n\
PackageIdentifier: {WINGET_PACKAGE_IDENTIFIER}\n\
PackageVersion: {version}\n\
PackageLocale: en-US\n\
Publisher: Scryer Media\n\
PublisherUrl: https://www.scryer.media/\n\
PublisherSupportUrl: https://github.com/scryer-media/scryer/issues\n\
Author: Scryer Media\n\
PackageName: {WINGET_PACKAGE_NAME}\n\
PackageUrl: https://github.com/scryer-media/scryer\n\
License: GPL-3.0\n\
LicenseUrl: https://github.com/scryer-media/scryer/blob/main/LICENSE\n\
Copyright: Copyright (c) Scryer Media\n\
ShortDescription: Self-hosted media acquisition and management platform.\n\
Description: Scryer is a self-hosted media acquisition and management platform.\n\
Moniker: {WINGET_MONIKER}\n\
Tags:\n\
- media\n\
- movies\n\
- self-hosted\n\
- series\n\
ReleaseNotesUrl: https://github.com/scryer-media/scryer/releases/tag/scryer-v{version}\n\
ManifestType: defaultLocale\n\
ManifestVersion: {WINGET_MANIFEST_VERSION}\n"
    )
}

fn winget_installer_manifest(
    version: &Version,
    release_date: &str,
    artifacts: &[WingetArtifact],
) -> String {
    let installers = artifacts
        .iter()
        .map(|artifact| {
            format!(
                "- Architecture: {}\n  InstallerUrl: {}\n  InstallerSha256: {}\n  ProductCode: '{}'",
                artifact.architecture,
                artifact.installer_url,
                artifact.installer_sha256,
                artifact.product_code,
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "# yaml-language-server: $schema=https://aka.ms/winget-manifest.installer.{WINGET_MANIFEST_VERSION}.schema.json\n\n\
PackageIdentifier: {WINGET_PACKAGE_IDENTIFIER}\n\
PackageVersion: {version}\n\
InstallerType: msi\n\
UpgradeBehavior: uninstallPrevious\n\
ReleaseDate: {release_date}\n\
Installers:\n\
{installers}\n\
ManifestType: installer\n\
ManifestVersion: {WINGET_MANIFEST_VERSION}\n"
    )
}

fn cache_builtin_artifacts(cache_dir: &Path, builtins: &[PathBuf]) -> Result<()> {
    if cache_dir.exists() {
        fs::remove_dir_all(cache_dir)
            .with_context(|| format!("failed to clear {}", cache_dir.display()))?;
    }
    fs::create_dir_all(cache_dir)
        .with_context(|| format!("failed to create {}", cache_dir.display()))?;

    for built_wasm in builtins {
        let file_name = built_wasm
            .file_name()
            .ok_or_else(|| anyhow!("missing builtin file name for {}", built_wasm.display()))?;
        let file_name = file_name.to_string_lossy().into_owned();
        let bytes = fs::read(built_wasm)
            .with_context(|| format!("failed to read builtin {}", built_wasm.display()))?;
        let cached = cache_dir.join(file_name);
        fs::write(&cached, bytes).with_context(|| {
            format!(
                "failed to cache builtin {} to {}",
                built_wasm.display(),
                cached.display()
            )
        })?;
    }

    Ok(())
}

fn builtin_version_manifest_path(ctx: &TaskContext) -> PathBuf {
    ctx.path(BUILTIN_VERSION_MANIFEST)
}

fn load_builtin_version_manifest(ctx: &TaskContext) -> Result<BuiltinVersionManifest> {
    let path = builtin_version_manifest_path(ctx);
    let manifest: BuiltinVersionManifest = serde_json::from_slice(
        &fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?,
    )
    .with_context(|| format!("failed to parse {}", path.display()))?;
    validate_builtin_version_manifest(&manifest)
        .with_context(|| format!("invalid {}", path.display()))?;
    Ok(manifest)
}

fn validate_builtin_version_manifest(manifest: &BuiltinVersionManifest) -> Result<()> {
    if manifest.schema_version != 2 {
        bail!(
            "unsupported schemaVersion {} (2 pins version and wasm_blake3; run `cargo xtask builtins sync`)",
            manifest.schema_version
        );
    }
    let expected = BUILTIN_PLUGINS
        .iter()
        .map(|spec| spec.plugin_id)
        .collect::<BTreeSet<_>>();
    let actual = manifest
        .plugins
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if actual != expected {
        bail!("must select exactly the supported built-in plugins");
    }
    for (plugin_id, pin) in &manifest.plugins {
        Version::parse(pin.version.trim_start_matches('v'))
            .with_context(|| format!("has invalid version {} for {plugin_id}", pin.version))?;
        if pin.wasm_blake3.len() != 64
            || !pin
                .wasm_blake3
                .chars()
                .all(|ch| ch.is_ascii_hexdigit() && !ch.is_ascii_uppercase())
        {
            bail!("has invalid wasm_blake3 for {plugin_id}: expected 64 lowercase hex characters");
        }
    }
    Ok(())
}

fn write_builtin_version_manifest(
    ctx: &TaskContext,
    plugins: BTreeMap<String, BuiltinVersionPin>,
) -> Result<()> {
    let path = builtin_version_manifest_path(ctx);
    fs::write(
        &path,
        canonical_pretty_json(&BuiltinVersionManifest {
            schema_version: 2,
            plugins,
        })?,
    )
    .with_context(|| format!("failed to write {}", path.display()))
}

fn builtin_cache_complete(cache_dir: &Path, builtins: &[PathBuf]) -> bool {
    cache_dir.is_dir()
        && builtins.iter().all(|built_wasm| {
            built_wasm
                .file_name()
                .map(|file_name| cache_dir.join(file_name).is_file())
                .unwrap_or(false)
        })
}

fn builtin_cache_matches_catalog_wasm_blake3(
    ctx: &TaskContext,
    cache_dir: &Path,
    expected_digests: &BTreeMap<String, String>,
) -> bool {
    let builtins = builtin_plugin_paths(ctx);
    !expected_digests.is_empty()
        && builtin_cache_complete(cache_dir, &builtins)
        && BUILTIN_PLUGINS.iter().all(|spec| {
            let paths = builtin_asset_paths(ctx, spec);
            let Some(file_name) = paths
                .wasm
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
            else {
                return false;
            };
            let Some(expected) = expected_digests.get(spec.plugin_id) else {
                return false;
            };
            fs::read(cache_dir.join(&file_name))
                .ok()
                .and_then(|bytes| zstd::decode_all(bytes.as_slice()).ok())
                .map(|wasm_bytes| blake3_hex(&wasm_bytes).eq_ignore_ascii_case(expected))
                .unwrap_or(false)
        })
}

fn validate_sigstore_trust_root_receipt(root_bytes: &[u8], receipt_bytes: &[u8]) -> Result<()> {
    let receipt: SigstoreTrustRootProvenance = serde_json::from_slice(receipt_bytes)
        .context("failed to parse Sigstore trust-root provenance receipt")?;
    if receipt.schema_version != 1 {
        bail!(
            "unsupported Sigstore trust-root provenance schema {}",
            receipt.schema_version
        );
    }
    if receipt.source != SIGSTORE_TRUST_ROOT_SOURCE {
        bail!(
            "unexpected Sigstore trust-root provenance source '{}'",
            receipt.source
        );
    }
    if receipt.target != SIGSTORE_TRUST_ROOT_TARGET {
        bail!(
            "unexpected Sigstore trust-root provenance target '{}'",
            receipt.target
        );
    }
    let actual_sha256 = sha256_hex(root_bytes);
    if receipt.sha256 != actual_sha256 {
        bail!(
            "Sigstore trust-root provenance digest mismatch: expected {}, got {actual_sha256}",
            receipt.sha256
        );
    }
    validate_runtime_sigstore_trust_root_document(root_bytes)
}

fn validate_cached_sigstore_trust_root(cache_dir: &Path) -> Result<()> {
    let root_path = cache_dir.join(SIGSTORE_TRUST_ROOT_FILENAME);
    let receipt_path = cache_dir.join(SIGSTORE_TRUST_ROOT_PROVENANCE_FILENAME);
    let root_bytes = fs::read(&root_path)
        .with_context(|| format!("failed to read cached {}", root_path.display()))?;
    let receipt_bytes = fs::read(&receipt_path)
        .with_context(|| format!("failed to read cached {}", receipt_path.display()))?;
    validate_sigstore_trust_root_receipt(&root_bytes, &receipt_bytes)
}

fn restore_builtin_artifacts_from_cache(
    ctx: &TaskContext,
    cache_dir: &Path,
    expected_digests: &BTreeMap<String, String>,
) -> Result<()> {
    let builtins = builtin_materialized_paths(ctx);
    if !builtin_cache_complete(cache_dir, &builtins) {
        bail!(
            "cached builtin artifacts are missing or incomplete under {}",
            cache_dir.display()
        );
    }
    if !builtin_cache_matches_catalog_wasm_blake3(ctx, cache_dir, expected_digests) {
        bail!(
            "cached builtin artifacts under {} differ from catalog wasm digests",
            cache_dir.display()
        );
    }
    validate_cached_sigstore_trust_root(cache_dir).with_context(|| {
        format!(
            "cached Sigstore trust material under {} is invalid",
            cache_dir.display()
        )
    })?;

    for output_wasm in builtins {
        let file_name = output_wasm
            .file_name()
            .ok_or_else(|| anyhow!("missing builtin file name for {}", output_wasm.display()))?;
        let cached = cache_dir.join(file_name);
        if let Some(parent) = output_wasm.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&cached, &output_wasm).with_context(|| {
            format!(
                "failed to restore cached builtin {} to {}",
                cached.display(),
                output_wasm.display()
            )
        })?;
    }

    Ok(())
}

fn release_dry_run_cache_rejection_reason(
    cache: &ReleaseDryRunCache,
    expected: &ReleaseDryRunExpectations<'_>,
    builtins_present: bool,
) -> Option<String> {
    if !cache.success {
        return Some("previous dry run did not complete successfully".to_string());
    }
    if cache.git_commit != expected.git_commit {
        return Some("HEAD commit changed since dry run".to_string());
    }
    if cache.release_args != expected.release_args {
        return Some("release arguments changed since dry run".to_string());
    }
    if cache.latest_tag_seen.as_deref() != expected.latest_tag_seen {
        return Some("latest release tag changed since dry run".to_string());
    }
    if cache.next_version != expected.next_version {
        return Some("computed next version changed since dry run".to_string());
    }
    if cache.tag_name != expected.tag_name {
        return Some("computed release tag changed since dry run".to_string());
    }
    if cache.release_notes_path.as_deref() != Some(expected.release_notes_path) {
        return Some("release notes path changed since dry run".to_string());
    }
    if cache.release_notes_sha256.as_deref() != Some(expected.release_notes_sha256) {
        return Some("release notes changed since dry run".to_string());
    }
    let validated_steps = cache
        .validated_steps
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let missing_steps = expected
        .validation_scope
        .required_steps()
        .iter()
        .copied()
        .filter(|step| !validated_steps.contains(step))
        .collect::<Vec<_>>();
    if !missing_steps.is_empty() {
        return Some(format!(
            "dry run did not record required release-blocking validations: {}",
            missing_steps.join(", ")
        ));
    }
    if !builtins_present {
        return Some("cached builtin artifacts are missing or BLAKE3-mismatched".to_string());
    }
    None
}

fn graphql_api_baseline_version() -> Version {
    Version::parse(GRAPHQL_API_BASELINE_VERSION)
        .expect("GraphQL API baseline version should be valid semver")
}

fn allow_missing_previous_graphql_schema(next_version: &Version) -> bool {
    *next_version == graphql_api_baseline_version()
}

fn next_version(current: &Version, bump: VersionBump) -> Version {
    let mut next = current.clone();
    match bump {
        VersionBump::Patch => {
            next.patch += 1;
        }
        VersionBump::Minor => {
            next.minor += 1;
            next.patch = 0;
        }
        VersionBump::Major => {
            next.major += 1;
            next.minor = 0;
            next.patch = 0;
        }
    }
    next.pre = Default::default();
    next.build = Default::default();
    next
}

fn workspace_member_tomls(ctx: &TaskContext) -> Result<Vec<PathBuf>> {
    let manifest = fs::read_to_string(ctx.path("Cargo.toml"))?;
    let workspace: TomlValue =
        toml::from_str(&manifest).context("failed to parse workspace Cargo.toml")?;
    let members = workspace
        .get("workspace")
        .and_then(|workspace| workspace.get("members"))
        .and_then(TomlValue::as_array)
        .ok_or_else(|| anyhow!("workspace.members missing from Cargo.toml"))?;

    let mut files = Vec::new();
    for member in members {
        let member = member
            .as_str()
            .ok_or_else(|| anyhow!("workspace member is not a string"))?;
        files.push(ctx.repo_root.join(member).join("Cargo.toml"));
    }
    Ok(files)
}

fn package_name(path: &Path) -> Result<String> {
    let manifest = fs::read_to_string(path)
        .with_context(|| format!("failed to read package manifest {}", path.display()))?;
    let document: TomlValue = toml::from_str(&manifest)
        .with_context(|| format!("failed to parse package manifest {}", path.display()))?;
    document
        .get("package")
        .and_then(|package| package.get("name"))
        .and_then(TomlValue::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!("package.name missing from {}", path.display()))
}

fn scryer_release_member_tomls(ctx: &TaskContext) -> Result<Vec<PathBuf>> {
    workspace_member_tomls(ctx)?
        .into_iter()
        .filter_map(|path| match package_name(&path) {
            Ok(name) if !is_scryer_app_release_package(&name) => {
                println!("   excluded non-app release package: {name}");
                None
            }
            Ok(_) => Some(Ok(path)),
            Err(error) => Some(Err(error)),
        })
        .collect()
}

fn is_scryer_app_release_package(name: &str) -> bool {
    !matches!(
        name,
        PLUGIN_SDK_PACKAGE | "xtask" | "xtask-release" | "xtask-migrations" | "xtask-support"
    )
}

fn write_package_version(path: &Path, version: &Version) -> Result<()> {
    let mut document = fs::read_to_string(path)?
        .parse::<DocumentMut>()
        .with_context(|| format!("failed to parse {}", path.display()))?;
    document["package"]["version"] = value(version.to_string());
    fs::write(path, document.to_string())?;
    Ok(())
}

fn refresh_release_cargo_lockfile(ctx: &TaskContext) -> Result<()> {
    step("Refreshing Cargo.lock after version bump");
    let mut generate_lockfile = ctx.release_command_in("cargo", &ctx.repo_root);
    generate_lockfile.arg("generate-lockfile");
    run_checked(&mut generate_lockfile)?;

    let mut metadata = ctx.release_command_in("cargo", &ctx.repo_root);
    metadata.args(["metadata", "--locked", "--format-version", "1"]);
    metadata.stdout(Stdio::null());
    run_checked(&mut metadata)?;
    ok("Cargo.lock refreshed and locked metadata passed");
    Ok(())
}

fn package_version(path: &Path) -> Result<Version> {
    let manifest = fs::read_to_string(path)
        .with_context(|| format!("failed to read package manifest {}", path.display()))?;
    let document: TomlValue = toml::from_str(&manifest)
        .with_context(|| format!("failed to parse package manifest {}", path.display()))?;
    let version = document
        .get("package")
        .and_then(|package| package.get("version"))
        .and_then(TomlValue::as_str)
        .ok_or_else(|| anyhow!("package.version missing from {}", path.display()))?;
    Version::parse(version).with_context(|| format!("invalid package.version {version}"))
}

fn parse_sdk_release_version(raw: &str) -> Result<Version> {
    let version = raw.trim();
    if version.is_empty() {
        bail!("SDK release version is required");
    }
    if version.starts_with('v') {
        bail!("pass SDK versions as plain semver, for example 1.0.0, not v1.0.0");
    }
    Version::parse(version).with_context(|| format!("invalid SDK release version {version}"))
}

fn sdk_release_tag_name(version: &Version) -> String {
    format!("{PLUGIN_SDK_TAG_PREFIX}{version}")
}

fn sdk_runtime_version_from_source(source: &str) -> Result<Version> {
    const PREFIX: &str = "pub const SDK_VERSION: &str = \"";
    for line in source.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(PREFIX) {
            let version = rest
                .strip_suffix("\";")
                .ok_or_else(|| anyhow!("SDK_VERSION declaration is malformed"))?;
            return Version::parse(version)
                .with_context(|| format!("invalid SDK_VERSION constant {version}"));
        }
    }
    bail!("SDK_VERSION declaration missing");
}

fn sdk_runtime_version(path: &Path) -> Result<Version> {
    let source =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    sdk_runtime_version_from_source(&source)
}

fn replace_sdk_runtime_version(source: &str, version: &Version) -> Result<String> {
    const PREFIX: &str = "pub const SDK_VERSION: &str = \"";
    let mut replaced = 0usize;
    let mut output = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with(PREFIX) {
            let indent_len = line.len() - trimmed.len();
            output.push(format!(
                "{}pub const SDK_VERSION: &str = \"{version}\";",
                &line[..indent_len]
            ));
            replaced += 1;
        } else {
            output.push(line.to_string());
        }
    }
    if replaced != 1 {
        bail!("expected exactly one SDK_VERSION declaration, found {replaced}");
    }
    let mut next = output.join("\n");
    if source.ends_with('\n') {
        next.push('\n');
    }
    Ok(next)
}

fn write_sdk_runtime_version(path: &Path, version: &Version) -> Result<()> {
    let source =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    fs::write(path, replace_sdk_runtime_version(&source, version)?)
        .with_context(|| format!("failed to write {}", path.display()))
}

fn validate_sdk_version_sync(ctx: &TaskContext, expected: &Version) -> Result<()> {
    let manifest_version = package_version(&ctx.path(PLUGIN_SDK_MANIFEST))?;
    let runtime_version = sdk_runtime_version(&ctx.path(PLUGIN_SDK_LIB))?;
    if manifest_version != *expected {
        bail!("SDK package version is {manifest_version}, expected {expected}");
    }
    if runtime_version != *expected {
        bail!("SDK_VERSION is {runtime_version}, expected {expected}");
    }
    Ok(())
}

fn status_path(line: &str) -> Option<String> {
    let path = line.get(3..)?.trim();
    let path = path.rsplit_once(" -> ").map_or(path, |(_, next)| next);
    Some(path.trim_matches('"').to_string())
}

fn sdk_release_scoped_path(path: &str) -> bool {
    path == "Cargo.lock"
        || path == ".github/workflows/plugin-sdk.yml"
        || path == "xtask/Cargo.toml"
        || path == "xtask/src/main.rs"
        || path.starts_with("crates/scryer-plugin-sdk/")
}

fn ensure_sdk_release_worktree_scope(ctx: &TaskContext) -> Result<()> {
    let status = git_capture(ctx, &["status", "--porcelain", "--untracked-files=no"])?;
    let unrelated = status
        .lines()
        .filter_map(status_path)
        .filter(|path| !sdk_release_scoped_path(path))
        .collect::<Vec<_>>();
    if !unrelated.is_empty() {
        bail!(
            "SDK release has unrelated tracked changes:\n{}",
            unrelated
                .iter()
                .map(|path| format!("  - {path}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
    Ok(())
}

struct FileSnapshot {
    path: PathBuf,
    bytes: Option<Vec<u8>>,
}

fn snapshot_files(paths: &[PathBuf]) -> Result<Vec<FileSnapshot>> {
    paths
        .iter()
        .map(|path| {
            let bytes = if path.exists() {
                Some(fs::read(path).with_context(|| format!("failed to read {}", path.display()))?)
            } else {
                None
            };
            Ok(FileSnapshot {
                path: path.clone(),
                bytes,
            })
        })
        .collect()
}

fn restore_snapshots(snapshots: Vec<FileSnapshot>) -> Result<()> {
    for snapshot in snapshots {
        match snapshot.bytes {
            Some(bytes) => fs::write(&snapshot.path, bytes)
                .with_context(|| format!("failed to restore {}", snapshot.path.display()))?,
            None if snapshot.path.exists() => fs::remove_file(&snapshot.path)
                .with_context(|| format!("failed to remove {}", snapshot.path.display()))?,
            None => {}
        }
    }
    Ok(())
}

fn changed_file(ctx: &TaskContext, path: &Path) -> Result<bool> {
    let output = git_capture(ctx, &["status", "--short", "--", &path.to_string_lossy()])?;
    Ok(!output.trim().is_empty())
}

fn commit_tracked_changes(
    ctx: &TaskContext,
    paths: &[PathBuf],
    message: &str,
) -> Result<Option<String>> {
    if paths.is_empty() {
        return Ok(None);
    }

    let mut add = ctx.release_command_in("git", &ctx.repo_root);
    add.arg("add");
    add.args(paths);
    run_checked(&mut add)?;

    let mut commit = ctx.release_command_in("git", &ctx.repo_root);
    commit.args(["commit", "-m", message]);
    run_checked(&mut commit)?;

    Ok(Some(current_head_commit(ctx)?))
}

fn add_prod_package_args(command: &mut Command) {
    for package in SCRYER_PROD_PACKAGES {
        command.args(["-p", package]);
    }
}

fn add_ci_clippy_package_args(command: &mut Command) {
    for package in SCRYER_CI_CLIPPY_PACKAGES {
        command.args(["-p", package]);
    }
}

struct BuiltinAssetPaths {
    wasm: PathBuf,
    descriptor_json: PathBuf,
    description: PathBuf,
}

struct BuiltinRefresh {
    paths: Vec<PathBuf>,
    catalog_wasm_blake3: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SigstoreTrustRootProvenance {
    schema_version: u32,
    source: String,
    target: String,
    sha256: String,
    retrieved_at: String,
    sigstore_version: String,
    source_commit: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    github_repository: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    github_workflow_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    github_run_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MaterializedTrustedRootDocument {
    media_type: String,
    certificate_authorities: Vec<MaterializedCertificateAuthority>,
    timestamp_authorities: Vec<MaterializedCertificateAuthority>,
    tlogs: Vec<MaterializedTrustedLog>,
    ctlogs: Vec<MaterializedTrustedLog>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MaterializedCertificateAuthority {
    cert_chain: MaterializedCertificateChain,
    valid_for: MaterializedTimeRange,
}

#[derive(Debug, Deserialize)]
struct MaterializedCertificateChain {
    certificates: Vec<MaterializedCertificate>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MaterializedCertificate {
    raw_bytes: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MaterializedTrustedLog {
    log_id: MaterializedLogId,
    public_key: MaterializedPublicKey,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MaterializedLogId {
    key_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MaterializedPublicKey {
    raw_bytes: String,
    valid_for: MaterializedTimeRange,
}

#[derive(Debug, Deserialize)]
struct MaterializedTimeRange {
    start: String,
    #[serde(default)]
    end: Option<String>,
}

struct VerifiedDownload {
    bytes: Vec<u8>,
    signature_bundle: Vec<u8>,
}

struct BuiltinPluginMaterialization {
    version: String,
    wasm_digest: String,
    catalog_url: String,
    catalog_redirect_bundle_sha256: String,
    catalog_bundle_sha256: String,
    provenance: serde_json::Value,
}

fn builtin_asset_paths_in(dir: &Path, spec: &BuiltinPluginSpec) -> BuiltinAssetPaths {
    BuiltinAssetPaths {
        wasm: dir.join(format!("{}.wasm.zst", spec.artifact_stem)),
        descriptor_json: dir.join(format!("{}.descriptor.json", spec.artifact_stem)),
        description: dir.join(format!("{}.description.txt", spec.artifact_stem)),
    }
}

fn builtin_asset_paths(ctx: &TaskContext, spec: &BuiltinPluginSpec) -> BuiltinAssetPaths {
    builtin_asset_paths_in(&ctx.path(BUILTIN_ASSET_DIR), spec)
}

fn builtin_plugin_paths(ctx: &TaskContext) -> Vec<PathBuf> {
    BUILTIN_PLUGINS
        .iter()
        .flat_map(|spec| {
            let paths = builtin_asset_paths(ctx, spec);
            [paths.wasm, paths.descriptor_json, paths.description]
        })
        .collect()
}

fn builtin_materialized_paths(ctx: &TaskContext) -> Vec<PathBuf> {
    let output_dir = ctx.path(BUILTIN_ASSET_DIR);
    let mut paths = builtin_plugin_paths(ctx);
    let (trust_root, trust_provenance) = sigstore_trust_root_paths(ctx);
    paths.extend([
        trust_root,
        trust_provenance,
        output_dir.join("provenance.json"),
    ]);
    paths.extend([
        output_dir.join("provenance/catalog-v3.redirect.bundle.json"),
        output_dir.join("provenance/catalog-v3.bundle.sigstore"),
    ]);
    paths.extend(BUILTIN_PLUGINS.iter().map(|spec| {
        output_dir.join(format!(
            "provenance/{}.wasm.zst.sigstore",
            spec.artifact_stem
        ))
    }));
    paths
}

fn sigstore_trust_root_paths_in(dir: &Path) -> (PathBuf, PathBuf) {
    (
        dir.join(SIGSTORE_TRUST_ROOT_FILENAME),
        dir.join(SIGSTORE_TRUST_ROOT_PROVENANCE_FILENAME),
    )
}

fn sigstore_trust_root_paths(ctx: &TaskContext) -> (PathBuf, PathBuf) {
    sigstore_trust_root_paths_in(&ctx.path(BUILTIN_ASSET_DIR))
}

fn decode_materialized_trust_base64(value: &str, label: &str) -> Result<Vec<u8>> {
    base64::engine::general_purpose::STANDARD
        .decode(value)
        .with_context(|| format!("failed to decode Sigstore {label}"))
}

fn validate_materialized_time_range(range: &MaterializedTimeRange, label: &str) -> Result<()> {
    let start = DateTime::parse_from_rfc3339(&range.start)
        .with_context(|| format!("invalid Sigstore {label} validity start"))?;
    if let Some(end) = range.end.as_deref() {
        let end = DateTime::parse_from_rfc3339(end)
            .with_context(|| format!("invalid Sigstore {label} validity end"))?;
        if end < start {
            bail!("Sigstore {label} validity ends before it starts");
        }
    }
    Ok(())
}

fn validate_materialized_trusted_logs(logs: &[MaterializedTrustedLog], label: &str) -> Result<()> {
    if logs.is_empty() {
        bail!("Sigstore {label} trust root is empty");
    }
    let mut key_ids = BTreeSet::new();
    for log in logs {
        let key_id =
            decode_materialized_trust_base64(&log.log_id.key_id, &format!("{label} log ID"))?;
        if key_id.is_empty() {
            bail!("Sigstore {label} log ID is empty");
        }
        if !key_ids.insert(key_id) {
            bail!("Sigstore trust root contains a duplicate {label} log ID");
        }
        let key_der = decode_materialized_trust_base64(
            &log.public_key.raw_bytes,
            &format!("{label} public key"),
        )?;
        CosignVerificationKey::try_from_der(&key_der)
            .with_context(|| format!("failed to parse Sigstore {label} public key"))?;
        validate_materialized_time_range(
            &log.public_key.valid_for,
            &format!("{label} public key"),
        )?;
    }
    Ok(())
}

fn validate_materialized_certificate_authorities(
    authorities: &[MaterializedCertificateAuthority],
    label: &str,
) -> Result<()> {
    if authorities.is_empty() {
        bail!("Sigstore {label} trust root is empty");
    }
    for (authority_index, authority) in authorities.iter().enumerate() {
        validate_materialized_time_range(
            &authority.valid_for,
            &format!("{label} authority {authority_index}"),
        )?;
        let mut certificate_ders = Vec::new();
        for (certificate_index, certificate) in authority.cert_chain.certificates.iter().enumerate()
        {
            let der = decode_materialized_trust_base64(
                &certificate.raw_bytes,
                &format!("{label} authority {authority_index} certificate {certificate_index}"),
            )?;
            Certificate::from_der(&der).with_context(|| {
                format!(
                    "failed to parse Sigstore {label} authority {authority_index} certificate {certificate_index}"
                )
            })?;
            certificate_ders.push(der);
        }
        let anchor_der = certificate_ders.last().ok_or_else(|| {
            anyhow!("Sigstore {label} authority {authority_index} has an empty certificate chain")
        })?;
        webpki::anchor_from_trusted_cert(&CertificateDer::from(anchor_der.as_slice()))
            .with_context(|| {
                format!("failed to parse Sigstore {label} authority {authority_index} anchor")
            })?;
    }
    Ok(())
}

fn validate_runtime_sigstore_trust_root_document(raw: &[u8]) -> Result<()> {
    let root: MaterializedTrustedRootDocument = serde_json::from_slice(raw)
        .context("failed to parse materialized Sigstore trusted_root.json")?;
    if !root
        .media_type
        .starts_with("application/vnd.dev.sigstore.trustedroot")
    {
        bail!(
            "unsupported Sigstore trusted-root media type '{}'",
            root.media_type
        );
    }
    validate_materialized_trusted_logs(&root.tlogs, "Rekor")?;
    validate_materialized_trusted_logs(&root.ctlogs, "CT")?;
    validate_materialized_certificate_authorities(&root.certificate_authorities, "Fulcio")?;
    validate_materialized_certificate_authorities(
        &root.timestamp_authorities,
        "timestamp-authority",
    )?;
    Ok(())
}

fn resolved_cargo_package_version(ctx: &TaskContext, package_name: &str) -> Result<String> {
    let lock_path = ctx.path("Cargo.lock");
    let lock: TomlValue = toml::from_str(
        &fs::read_to_string(&lock_path)
            .with_context(|| format!("failed to read {}", lock_path.display()))?,
    )
    .with_context(|| format!("failed to parse {}", lock_path.display()))?;
    let versions = lock
        .get("package")
        .and_then(TomlValue::as_array)
        .into_iter()
        .flatten()
        .filter(|package| package.get("name").and_then(TomlValue::as_str) == Some(package_name))
        .filter_map(|package| package.get("version").and_then(TomlValue::as_str))
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    if versions.len() != 1 {
        bail!(
            "expected exactly one resolved {package_name} version in {}, found {:?}",
            lock_path.display(),
            versions
        );
    }
    Ok(versions.into_iter().next().expect("one version checked"))
}

fn install_xtask_sigstore_trust_material(trust_root: &SigstoreTrustRoot) -> Result<()> {
    let rekor_keys = trust_root
        .rekor_keys()
        .map_err(|error| anyhow!("failed to load Sigstore Rekor public keys: {error}"))?;
    let rekor_keys = Arc::new(parse_rekor_verification_keys(rekor_keys)?);
    if let Some(existing) = REKOR_VERIFICATION_KEYS.get() {
        existing.as_ref().map_err(|error| anyhow!(error.clone()))?;
    } else {
        REKOR_VERIFICATION_KEYS
            .set(Ok(rekor_keys))
            .map_err(|_| anyhow!("failed to initialize Sigstore Rekor keys"))?;
    }

    let fulcio_certs = trust_root
        .fulcio_certs()
        .map_err(|error| anyhow!("failed to load Sigstore Fulcio certificates: {error}"))?;
    let anchors = fulcio_certs
        .iter()
        .map(|cert| {
            webpki::anchor_from_trusted_cert(cert)
                .map(|anchor| anchor.to_owned())
                .map_err(|error| anyhow!(error.to_string()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if anchors.is_empty() {
        bail!("Sigstore Fulcio trust root is empty");
    }
    if let Some(existing) = FULCIO_TRUST_ANCHORS.get() {
        existing.as_ref().map_err(|error| anyhow!(error.clone()))?;
    } else {
        FULCIO_TRUST_ANCHORS
            .set(Ok(Arc::new(anchors)))
            .map_err(|_| anyhow!("failed to initialize Sigstore Fulcio anchors"))?;
    }
    Ok(())
}

fn write_materialized_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("materialized path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent)?;
    let temporary = path.with_extension(format!(
        "{}.tmp-{}",
        path.extension().and_then(OsStr::to_str).unwrap_or("file"),
        std::process::id()
    ));
    fs::write(&temporary, bytes)
        .with_context(|| format!("failed to write {}", temporary.display()))?;
    if path.exists() {
        fs::remove_file(path).with_context(|| format!("failed to replace {}", path.display()))?;
    }
    fs::rename(&temporary, path)
        .with_context(|| format!("failed to activate {}", path.display()))?;
    Ok(())
}

fn materialize_sigstore_trust_root(ctx: &TaskContext, output_dir: &Path) -> Result<()> {
    step("Materializing TUF-verified Sigstore trust root");
    let checkout = tempfile::tempdir().context("failed to create Sigstore TUF checkout")?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to build Sigstore trust-root runtime")?;
    let trust_root = runtime
        .block_on(async {
            tokio::time::timeout(
                SIGSTORE_TRUST_ROOT_TIMEOUT,
                SigstoreTrustRoot::new(Some(checkout.path())),
            )
            .await
        })
        .map_err(|_| {
            anyhow!(
                "timed out after {} seconds loading the Sigstore trust root",
                SIGSTORE_TRUST_ROOT_TIMEOUT.as_secs()
            )
        })?
        .map_err(|error| anyhow!("failed to load Sigstore trust root: {error}"))?;
    let root_bytes = fs::read(checkout.path().join(SIGSTORE_TRUST_ROOT_TARGET))
        .context("failed to read TUF-verified Sigstore trusted_root.json")?;
    validate_runtime_sigstore_trust_root_document(&root_bytes)?;
    install_xtask_sigstore_trust_material(&trust_root)?;

    let sha256 = Sha256::digest(&root_bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let provenance = SigstoreTrustRootProvenance {
        schema_version: 1,
        source: SIGSTORE_TRUST_ROOT_SOURCE.to_string(),
        target: SIGSTORE_TRUST_ROOT_TARGET.to_string(),
        sha256: sha256.clone(),
        retrieved_at: Utc::now().to_rfc3339(),
        sigstore_version: format!(
            "sigstore-rs {}",
            resolved_cargo_package_version(ctx, "sigstore")?
        ),
        source_commit: current_head_commit(ctx)?,
        github_repository: std::env::var("GITHUB_REPOSITORY").ok(),
        github_workflow_ref: std::env::var("GITHUB_WORKFLOW_REF").ok(),
        github_run_id: std::env::var("GITHUB_RUN_ID").ok(),
    };
    let provenance_bytes = canonical_pretty_json(&provenance)?.into_bytes();
    let parsed: SigstoreTrustRootProvenance = serde_json::from_slice(&provenance_bytes)
        .context("failed to validate Sigstore trust-root provenance")?;
    if parsed.sha256 != sha256 {
        bail!("Sigstore trust-root provenance digest mismatch");
    }

    let (root_path, provenance_path) = sigstore_trust_root_paths_in(output_dir);
    write_materialized_file(&root_path, &root_bytes)?;
    if let Err(error) = write_materialized_file(&provenance_path, &provenance_bytes) {
        let _ = fs::remove_file(&root_path);
        return Err(error);
    }
    ok(format!("Materialized Sigstore trust root sha256:{sha256}"));
    Ok(())
}

fn fetch_url_bytes(url: &str) -> Result<Vec<u8>> {
    let response = reqwest::blocking::get(url)
        .with_context(|| format!("failed to fetch {url}"))?
        .error_for_status()
        .with_context(|| format!("request returned error status for {url}"))?;
    Ok(response
        .bytes()
        .with_context(|| format!("failed to read response body for {url}"))?
        .to_vec())
}

fn decode_possibly_zstd_bytes(url: &str, bytes: Vec<u8>) -> Result<Vec<u8>> {
    if url.ends_with(".zst") {
        return zstd::decode_all(bytes.as_slice())
            .with_context(|| format!("failed to decompress {url}"));
    }
    Ok(bytes)
}

fn verify_signed_blob(
    raw: &[u8],
    bundle_raw: &[u8],
    required_signer: &RequiredSigner,
) -> Result<()> {
    let bundle_text = std::str::from_utf8(bundle_raw).context("invalid Sigstore bundle UTF-8")?;
    let bundle_text = normalize_sigstore_bundle(bundle_text)?;
    let rekor_keys = cached_rekor_verification_keys()?;
    let bundle = SignedArtifactBundle::new_verified(bundle_text.as_str(), rekor_keys.as_ref())
        .map_err(|error| anyhow!("Sigstore Rekor bundle verification failed: {error}"))?;
    let cert_pem = normalize_bundle_cert(&bundle.cert)?;
    <sigstore::cosign::Client as CosignCapabilities>::verify_blob(
        &cert_pem,
        &bundle.base64_signature,
        raw,
    )
    .map_err(|error| anyhow!("Sigstore blob signature verification failed: {error}"))?;
    verify_fulcio_certificate_chain(&cert_pem, &bundle)?;
    verify_signer_identity(&cert_pem, required_signer)?;
    Ok(())
}

fn verify_fulcio_certificate_chain(cert_pem: &str, bundle: &SignedArtifactBundle) -> Result<()> {
    let cert = Certificate::from_pem(cert_pem.as_bytes())
        .map_err(|error| anyhow!("failed to parse Sigstore certificate: {error}"))?;
    let cert_der = cert
        .to_der()
        .map_err(|error| anyhow!("failed to encode Sigstore certificate: {error}"))?;
    let cert_der = CertificateDer::from(cert_der.as_slice());
    let end_entity = EndEntityCert::try_from(&cert_der)
        .map_err(|error| anyhow!("invalid Sigstore certificate: {error}"))?;
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
            anyhow!("Sigstore Fulcio certificate chain verification failed: {error}")
        })?;

    Ok(())
}

fn rekor_integrated_time(integrated_time: i64) -> Result<UnixTime> {
    let integrated_time =
        u64::try_from(integrated_time).context("Sigstore Rekor integrated time is negative")?;
    Ok(UnixTime::since_unix_epoch(std::time::Duration::from_secs(
        integrated_time,
    )))
}

fn cached_rekor_verification_keys() -> Result<Arc<RekorVerificationKeys>> {
    REKOR_VERIFICATION_KEYS
        .get()
        .ok_or_else(|| anyhow!("Sigstore trust root was not materialized before verification"))?
        .clone()
        .map_err(anyhow::Error::msg)
}

fn cached_fulcio_trust_anchors() -> Result<Arc<FulcioTrustAnchors>> {
    FULCIO_TRUST_ANCHORS
        .get()
        .ok_or_else(|| anyhow!("Sigstore trust root was not materialized before verification"))?
        .clone()
        .map_err(anyhow::Error::msg)
}

fn parse_rekor_verification_keys(keys: BTreeMap<String, &[u8]>) -> Result<RekorVerificationKeys> {
    let parsed = keys
        .into_iter()
        .filter_map(|(key_id, key)| {
            CosignVerificationKey::from_der(key, &SigningScheme::default())
                .ok()
                .map(|key| (key_id, key))
        })
        .collect::<BTreeMap<_, _>>();
    if parsed.is_empty() {
        bail!("failed to parse any Rekor public keys from the Sigstore trust root");
    }
    Ok(parsed)
}

fn normalize_sigstore_bundle(bundle_text: &str) -> Result<String> {
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
        .ok_or_else(|| anyhow!("Sigstore bundle missing verificationMaterial.tlogEntries[0]"))?;
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
    .context("failed to normalize Sigstore bundle")
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
) -> Result<&'a str> {
    sigstore_bundle_value(value, path)
        .and_then(|value| value.as_str())
        .ok_or_else(|| anyhow!("Sigstore bundle missing {label}"))
}

fn sigstore_bundle_i64_field(value: &serde_json::Value, path: &[&str], label: &str) -> Result<i64> {
    let Some(value) = sigstore_bundle_value(value, path) else {
        bail!("Sigstore bundle missing {label}");
    };
    if let Some(number) = value.as_i64() {
        return Ok(number);
    }
    let Some(number) = value.as_str() else {
        bail!("Sigstore bundle {label} is not an integer");
    };
    number
        .parse::<i64>()
        .with_context(|| format!("Sigstore bundle {label} is not a valid integer"))
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

fn normalize_bundle_cert(cert: &str) -> Result<String> {
    if cert.contains("-----BEGIN CERTIFICATE-----") {
        return Ok(cert.to_string());
    }
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(cert.as_bytes())
        .context("invalid base64 Sigstore certificate")?;
    if let Ok(decoded_text) = String::from_utf8(decoded.clone())
        && decoded_text.contains("-----BEGIN CERTIFICATE-----")
    {
        return Ok(decoded_text);
    }
    Ok(pem_encode_certificate(&decoded))
}

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

fn cert_extension_utf8(cert: &Certificate, oid: &str) -> Result<Option<String>> {
    let Some(extensions) = cert.tbs_certificate().extensions() else {
        return Ok(None);
    };
    extensions
        .iter()
        .find(|ext: &&Extension| ext.extn_id.to_string() == oid)
        .map(|ext| {
            String::from_utf8(ext.extn_value.clone().into_bytes().into_vec())
                .map_err(|_| anyhow!("Sigstore certificate extension {oid} is not valid UTF-8"))
        })
        .transpose()
}

fn cert_subject_uri(cert: &Certificate) -> Result<Option<String>> {
    let san = cert
        .tbs_certificate()
        .get_extension::<SubjectAltName>()
        .map_err(|error| anyhow!("failed to read certificate SAN: {error}"))?
        .map(|(_, san)| san);
    let Some(san) = san else {
        return Ok(None);
    };
    Ok(san.0.iter().find_map(|name| match name {
        GeneralName::UniformResourceIdentifier(uri) => Some(uri.to_string()),
        _ => None,
    }))
}

fn verify_signer_identity(cert_pem: &str, required_signer: &RequiredSigner) -> Result<()> {
    let cert = Certificate::from_pem(cert_pem.as_bytes())
        .map_err(|error| anyhow!("failed to parse Sigstore certificate: {error}"))?;
    let repository = cert_extension_utf8(&cert, SIGSTORE_GITHUB_WORKFLOW_REPOSITORY_OID)?;
    if repository.as_deref() != Some(required_signer.github_repository.as_str()) {
        bail!(
            "Sigstore signer repo mismatch: expected '{}', got '{}'",
            required_signer.github_repository,
            repository.unwrap_or_else(|| "<missing>".to_string())
        );
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
            bail!(
                "Sigstore workflow mismatch for '{}'",
                required_signer.github_repository
            );
        }
    }

    Ok(())
}

fn fetch_verified_bytes(
    _ctx: &TaskContext,
    required_signer: &RequiredSigner,
    url: &str,
    bundle_url: &str,
) -> Result<VerifiedDownload> {
    let blob_bytes = fetch_url_bytes(url)?;
    let bundle_bytes = fetch_url_bytes(bundle_url)?;
    let decoded_bundle = decode_possibly_zstd_bytes(bundle_url, bundle_bytes.clone())?;
    verify_signed_blob(&blob_bytes, &decoded_bundle, required_signer)?;
    Ok(VerifiedDownload {
        bytes: blob_bytes,
        signature_bundle: bundle_bytes,
    })
}

fn blake3_hex(bytes: &[u8]) -> String {
    format!("blake3:{}", blake3::hash(bytes).to_hex())
}

fn require_blake3_bytes(label: &str, expected: &str, bytes: &[u8]) -> Result<()> {
    let actual = blake3_hex(bytes);
    if !actual.eq_ignore_ascii_case(expected) {
        bail!("{label} digest mismatch: expected {expected}, got {actual}");
    }
    Ok(())
}

fn required_blake3_digest<'a>(label: &str, digests: &'a [String]) -> Result<&'a str> {
    digests
        .iter()
        .find(|digest| digest.starts_with("blake3:"))
        .map(String::as_str)
        .ok_or_else(|| anyhow!("{label} is missing a blake3 digest"))
}

fn official_plugin_v3_signer() -> RequiredSigner {
    RequiredSigner {
        github_repository: OFFICIAL_PLUGIN_REPO.to_string(),
        github_workflow: Some(OFFICIAL_PLUGIN_V3_RELEASE_WORKFLOW.to_string()),
    }
}

fn require_official_plugin_v3_signer(plugin_id: &str, signer: &RequiredSigner) -> Result<()> {
    if signer.github_repository != OFFICIAL_PLUGIN_REPO
        || signer.github_workflow.as_deref() != Some(OFFICIAL_PLUGIN_V3_RELEASE_WORKFLOW)
    {
        bail!(
            "{plugin_id}: catalog-v3 entry requires unexpected signer {} workflow {:?}",
            signer.github_repository,
            signer.github_workflow
        );
    }
    Ok(())
}

fn latest_catalog_v3_release<'a>(
    plugin_id: &str,
    releases: &'a [CatalogV3Release],
) -> Result<&'a CatalogV3Release> {
    releases
        .iter()
        .max_by_key(|release| Version::parse(release.version.trim_start_matches('v')).ok())
        .ok_or_else(|| anyhow!("{plugin_id}: catalog-v3 entry has no releases"))
}

fn catalog_release_sdk_matches_host(plugin_id: &str, release: &CatalogV3Release) -> Result<bool> {
    let Some(constraint) = release
        .sdk_constraint
        .as_deref()
        .map(str::trim)
        .filter(|constraint| !constraint.is_empty())
    else {
        return Ok(false);
    };
    let req = VersionReq::parse(constraint).with_context(|| {
        format!(
            "{plugin_id} {} has invalid sdk_constraint {constraint}",
            release.version
        )
    })?;
    let host_sdk = Version::parse(scryer_plugin_sdk::SDK_VERSION)?;
    Ok(req.matches(&host_sdk))
}

fn catalog_release_supports_scryer_version(
    plugin_id: &str,
    release: &CatalogV3Release,
    scryer_version: &Version,
) -> Result<bool> {
    let Some(min_scryer_version) = release
        .min_scryer_version
        .as_deref()
        .map(str::trim)
        .filter(|version| !version.is_empty())
    else {
        return Ok(false);
    };
    let min_scryer_version = Version::parse(min_scryer_version).with_context(|| {
        format!(
            "{plugin_id} {} has invalid min_scryer_version {}",
            release.version, min_scryer_version
        )
    })?;
    let max_scryer_version = release
        .max_scryer_version
        .as_deref()
        .map(str::trim)
        .filter(|version| !version.is_empty())
        .map(Version::parse)
        .transpose()
        .with_context(|| {
            format!(
                "{plugin_id} {} has invalid max_scryer_version",
                release.version
            )
        })?;
    Ok(scryer_version >= &min_scryer_version
        && max_scryer_version
            .as_ref()
            .is_none_or(|max| scryer_version <= max))
}

fn catalog_release_is_builtin_compatible(
    plugin_id: &str,
    release: &CatalogV3Release,
    scryer_version: &Version,
) -> Result<bool> {
    Ok(catalog_release_sdk_matches_host(plugin_id, release)?
        && catalog_release_supports_scryer_version(plugin_id, release, scryer_version)?)
}

fn latest_compatible_catalog_v3_release<'a>(
    plugin_id: &str,
    releases: &'a [CatalogV3Release],
    scryer_version: &Version,
) -> Result<Option<&'a CatalogV3Release>> {
    let mut compatible = Vec::new();
    for release in releases {
        if catalog_release_is_builtin_compatible(plugin_id, release, scryer_version)? {
            compatible.push(release);
        }
    }
    compatible.sort_by_key(|release| Version::parse(release.version.trim_start_matches('v')).ok());
    Ok(compatible.pop())
}

fn baseline_catalog_v3_zstd_artifact<'a>(
    plugin_id: &str,
    release: &'a CatalogV3Release,
) -> Result<&'a CatalogV3PluginArtifact> {
    ["wasm32-wasip2", "wasm32-wasip1"]
        .into_iter()
        .find_map(|runtime| {
            release.artifacts.iter().find(|artifact| {
                artifact.runtime == runtime
                    && artifact.required_features.is_empty()
                    && artifact.url.ends_with(".wasm.zst")
            })
        })
        .ok_or_else(|| {
            anyhow!(
                "{plugin_id} {} has no baseline WASI .wasm.zst artifact",
                release.version
            )
        })
}

fn require_builtin_descriptor_sdk_contract(
    plugin_id: &str,
    sdk_version: &str,
    sdk_constraint: &str,
) -> Result<()> {
    scryer_plugin_sdk::validate_sdk_contract(
        plugin_id,
        sdk_version,
        sdk_constraint,
        scryer_plugin_sdk::SDK_VERSION,
    )
    .map_err(|error| anyhow!(error))
}

fn release_builtin_descriptor_loader(ctx: &TaskContext) -> Result<WasmPluginDescriptorLoader> {
    let cache_dir = ctx.path("tmp/xtask-release-wasmtime");
    scryer_plugins::initialize_wasm_runtime_at(&cache_dir).map_err(|error| {
        anyhow!(
            "failed to initialize release WASM plugin cache at {}: {error}",
            cache_dir.display()
        )
    })?;
    Ok(WasmPluginDescriptorLoader)
}

fn sync_builtin_plugin(
    ctx: &TaskContext,
    output_dir: &Path,
    spec: &BuiltinPluginSpec,
    scryer_version: &Version,
    requested_version: Option<&str>,
) -> Result<BuiltinPluginMaterialization> {
    let catalog_signer = official_plugin_v3_signer();
    let redirect_download = fetch_verified_bytes(
        ctx,
        &catalog_signer,
        OFFICIAL_PLUGIN_CATALOG_V3_REDIRECT_URL,
        OFFICIAL_PLUGIN_CATALOG_V3_REDIRECT_BUNDLE_URL,
    )?;
    let redirect: CatalogV3Redirect = serde_json::from_slice(&redirect_download.bytes)
        .context("failed to parse official plugin catalog-v3 redirect")?;
    let catalog_artifact = redirect
        .artifacts
        .last()
        .ok_or_else(|| anyhow!("official plugin catalog-v3 redirect has no artifacts"))?;
    let catalog_download = fetch_verified_bytes(
        ctx,
        &catalog_signer,
        &catalog_artifact.url,
        &catalog_artifact.signature_url,
    )?;
    let catalog_bytes = decode_possibly_zstd_bytes(&catalog_artifact.url, catalog_download.bytes)?;
    let catalog: CatalogV3 = serde_json::from_slice(&catalog_bytes)
        .context("failed to parse official plugin catalog-v3")?;
    let entry = catalog
        .plugins
        .iter()
        .find(|entry| entry.id == spec.plugin_id)
        .ok_or_else(|| {
            anyhow!(
                "builtin plugin '{}' missing from official catalog",
                spec.plugin_id
            )
        })?;
    require_official_plugin_v3_signer(spec.plugin_id, &entry.required_signer)?;
    latest_catalog_v3_release(spec.plugin_id, &entry.releases)?;
    let release = if let Some(requested_version) = requested_version {
        let release = entry
            .releases
            .iter()
            .find(|release| {
                release.version.trim_start_matches('v') == requested_version.trim_start_matches('v')
            })
            .ok_or_else(|| {
                anyhow!(
                    "catalog-v3 no longer contains pinned builtin {} version {}",
                    spec.plugin_id,
                    requested_version
                )
            })?;
        if !catalog_release_is_builtin_compatible(spec.plugin_id, release, scryer_version)? {
            bail!(
                "pinned builtin {} {} is incompatible with Scryer {} and SDK {}",
                spec.plugin_id,
                requested_version,
                scryer_version,
                scryer_plugin_sdk::SDK_VERSION
            );
        }
        release
    } else {
        latest_compatible_catalog_v3_release(spec.plugin_id, &entry.releases, scryer_version)?
            .ok_or_else(|| {
                anyhow!(
                    "no catalog-v3 release for builtin {} is compatible with Scryer {} and SDK {}",
                    spec.plugin_id,
                    scryer_version,
                    scryer_plugin_sdk::SDK_VERSION
                )
            })?
    };
    let artifact = baseline_catalog_v3_zstd_artifact(spec.plugin_id, release)?;
    let artifact_download = fetch_verified_bytes(
        ctx,
        &entry.required_signer,
        &artifact.url,
        &artifact.signature_url,
    )?;
    require_blake3_bytes(
        "compressed builtin artifact",
        required_blake3_digest("compressed builtin artifact", &artifact.digests)?,
        &artifact_download.bytes,
    )?;
    let wasm_bytes = zstd::decode_all(artifact_download.bytes.as_slice()).with_context(|| {
        format!(
            "failed to decompress builtin artifact for {}",
            spec.plugin_id
        )
    })?;
    let wasm_digest = required_blake3_digest("builtin wasm", &artifact.wasm_digests)?;
    require_blake3_bytes("builtin wasm", wasm_digest, &wasm_bytes)?;
    let mut descriptor = release_builtin_descriptor_loader(ctx)?
        .load_descriptor_from_wasm_bytes(&wasm_bytes)
        .map_err(|error| anyhow!("failed to describe builtin {}: {error}", spec.plugin_id))?;
    if descriptor.id != spec.plugin_id {
        bail!(
            "descriptor id mismatch for {}: got {}",
            spec.plugin_id,
            descriptor.id
        );
    }
    if descriptor.version != release.version {
        bail!(
            "descriptor version mismatch for {}: got {}, expected {}",
            spec.plugin_id,
            descriptor.version,
            release.version
        );
    }
    require_builtin_descriptor_sdk_contract(
        spec.plugin_id,
        &descriptor.sdk_version,
        &descriptor.sdk_constraint,
    )?;
    descriptor.sdk_version = scryer_plugin_sdk::SDK_VERSION.to_string();
    descriptor.sdk_constraint = scryer_plugin_sdk::current_sdk_constraint();

    let paths = builtin_asset_paths_in(output_dir, spec);
    for path in [&paths.wasm, &paths.descriptor_json, &paths.description] {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
    }
    fs::write(&paths.wasm, &artifact_download.bytes)
        .with_context(|| format!("failed to write {}", paths.wasm.display()))?;
    let descriptor_json = canonical_pretty_json(&descriptor)?;
    fs::write(&paths.descriptor_json, descriptor_json.as_bytes())
        .with_context(|| format!("failed to write {}", paths.descriptor_json.display()))?;
    fs::write(
        &paths.description,
        format!("{}\n", entry.description.trim()),
    )
    .with_context(|| format!("failed to write {}", paths.description.display()))?;

    ok(format!(
        "synced builtin {} {} from official catalog-v3",
        spec.plugin_id, release.version
    ));
    let provenance_dir = output_dir.join("provenance");
    fs::create_dir_all(&provenance_dir)?;
    write_materialized_file(
        &provenance_dir.join("catalog-v3.redirect.bundle.json"),
        &redirect_download.signature_bundle,
    )?;
    write_materialized_file(
        &provenance_dir.join("catalog-v3.bundle.sigstore"),
        &catalog_download.signature_bundle,
    )?;
    write_materialized_file(
        &provenance_dir.join(format!("{}.wasm.zst.sigstore", spec.artifact_stem)),
        &artifact_download.signature_bundle,
    )?;

    Ok(BuiltinPluginMaterialization {
        version: release.version.clone(),
        wasm_digest: wasm_digest.to_string(),
        catalog_url: catalog_artifact.url.clone(),
        catalog_redirect_bundle_sha256: sha256_hex(&redirect_download.signature_bundle),
        catalog_bundle_sha256: sha256_hex(&catalog_download.signature_bundle),
        provenance: serde_json::json!({
            "version": release.version,
            "artifact_url": artifact.url,
            "signature_url": artifact.signature_url,
            "compressed_blake3": required_blake3_digest(
                "compressed builtin artifact",
                &artifact.digests,
            )?.trim_start_matches("blake3:"),
            "wasm_blake3": wasm_digest.trim_start_matches("blake3:"),
            "descriptor_sha256": sha256_hex(descriptor_json.as_bytes()),
            "signature_bundle_sha256": sha256_hex(&artifact_download.signature_bundle),
        }),
    })
}

fn publish_materialized_builtin_dir(staging_dir: &Path, output_dir: &Path) -> Result<()> {
    let parent = output_dir
        .parent()
        .ok_or_else(|| anyhow!("builtins output has no parent: {}", output_dir.display()))?;
    fs::create_dir_all(parent)?;
    let previous_dir = staging_dir
        .parent()
        .ok_or_else(|| anyhow!("builtins staging path has no parent"))?
        .join("previous");
    let had_previous = output_dir.exists();
    if had_previous {
        fs::rename(output_dir, &previous_dir).with_context(|| {
            format!(
                "failed to stage previous builtins directory {}",
                output_dir.display()
            )
        })?;
    }
    if let Err(activate_error) = fs::rename(staging_dir, output_dir) {
        if had_previous && let Err(restore_error) = fs::rename(&previous_dir, output_dir) {
            bail!(
                "failed to activate materialized builtins ({activate_error}) and failed to restore the previous directory ({restore_error})"
            );
        }
        return Err(activate_error).with_context(|| {
            format!(
                "failed to activate materialized builtins at {}",
                output_dir.display()
            )
        });
    }
    Ok(())
}

fn refresh_builtin_plugins(
    ctx: &TaskContext,
    scryer_version: &Version,
    pinned_versions: Option<&BuiltinVersionManifest>,
    update_manifest: bool,
) -> Result<BuiltinRefresh> {
    step(if pinned_versions.is_some() {
        "Materializing pinned embedded plugin builtins from the official catalog"
    } else {
        "Syncing embedded plugin builtins from the official catalog"
    });
    let output_dir = ctx.path(BUILTIN_ASSET_DIR);
    let output_parent = output_dir
        .parent()
        .ok_or_else(|| anyhow!("builtins output has no parent: {}", output_dir.display()))?;
    fs::create_dir_all(output_parent)?;
    let staging = tempfile::Builder::new()
        .prefix(".builtins-materializing-")
        .tempdir_in(output_parent)
        .context("failed to create builtins materialization directory")?;
    let staging_dir = staging.path().join("builtins");
    fs::create_dir_all(&staging_dir)?;
    materialize_sigstore_trust_root(ctx, &staging_dir)?;
    let mut catalog_wasm_blake3 = BTreeMap::new();
    let mut selected_versions = BTreeMap::new();
    let mut plugin_provenance = serde_json::Map::new();
    let mut catalog_provenance: Option<(String, String, String)> = None;
    for spec in BUILTIN_PLUGINS {
        let pin = pinned_versions
            .map(|manifest| {
                manifest.plugins.get(spec.plugin_id).ok_or_else(|| {
                    anyhow!(
                        "built-in manifest is missing version for {}",
                        spec.plugin_id
                    )
                })
            })
            .transpose()?;
        let materialized = sync_builtin_plugin(
            ctx,
            &staging_dir,
            spec,
            scryer_version,
            pin.map(|pin| pin.version.as_str()),
        )
        .with_context(|| format!("failed to sync builtin {}", spec.plugin_id))?;
        let version = materialized.version;
        let wasm_digest = materialized.wasm_digest;
        let candidate_catalog = (
            materialized.catalog_url,
            materialized.catalog_redirect_bundle_sha256,
            materialized.catalog_bundle_sha256,
        );
        if let Some(catalog) = catalog_provenance.as_ref()
            && catalog != &candidate_catalog
        {
            bail!("built-in plugins were resolved from inconsistent catalog material");
        }
        catalog_provenance = Some(candidate_catalog);
        plugin_provenance.insert(spec.plugin_id.to_string(), materialized.provenance);
        // The version pin names a release; the digest pin names its bytes. A
        // catalog that re-points the pinned version at different content is a
        // hard failure, never a silent re-materialization — the only way past
        // it is a reviewed manifest bump via `cargo xtask builtins sync`.
        if let Some(pin) = pin
            && !builtin_wasm_digest_matches(&pin.wasm_blake3, &wasm_digest)
        {
            bail!(
                "builtin {} {} WASM digest does not match the pinned wasm_blake3 in \
                 builtin-versions.json (pinned {}, catalog {}); if this change is intended, \
                 refresh the pin with `cargo xtask builtins sync`",
                spec.plugin_id,
                version,
                pin.wasm_blake3,
                wasm_digest
            );
        }
        selected_versions.insert(
            spec.plugin_id.to_string(),
            BuiltinVersionPin {
                version,
                wasm_blake3: wasm_digest
                    .trim_start_matches("blake3:")
                    .to_ascii_lowercase(),
            },
        );
        catalog_wasm_blake3.insert(spec.plugin_id.to_string(), wasm_digest);
    }
    let (catalog_url, catalog_redirect_sha256, catalog_sha256) =
        catalog_provenance.ok_or_else(|| anyhow!("no built-in plugins were materialized"))?;
    let provenance = serde_json::json!({
        "schemaVersion": 1,
        "host": {
            "scryer_version": scryer_version.to_string(),
            "sdk_version": scryer_plugin_sdk::SDK_VERSION,
            "sdk_constraint": scryer_plugin_sdk::current_sdk_constraint(),
        },
        "catalog_redirect_url": OFFICIAL_PLUGIN_CATALOG_V3_REDIRECT_URL,
        "catalog_url": catalog_url,
        "verified_bundles": {
            "catalog_redirect_sha256": catalog_redirect_sha256,
            "catalog_sha256": catalog_sha256,
        },
        "signer": {
            "repository": OFFICIAL_PLUGIN_REPO,
            "workflow": OFFICIAL_PLUGIN_V3_RELEASE_WORKFLOW,
        },
        "plugins": plugin_provenance,
    });
    write_materialized_file(
        &staging_dir.join("provenance.json"),
        canonical_pretty_json(&provenance)?.as_bytes(),
    )?;
    publish_materialized_builtin_dir(&staging_dir, &output_dir)?;
    if update_manifest {
        write_builtin_version_manifest(ctx, selected_versions)?;
    }
    ok(if update_manifest {
        "Embedded plugin builtins refreshed and manifest updated"
    } else {
        "Pinned embedded plugin builtins materialized"
    });
    Ok(BuiltinRefresh {
        paths: builtin_materialized_paths(ctx),
        catalog_wasm_blake3,
    })
}

fn run_clippy_ci(ctx: &TaskContext, args: ClippyArgs) -> Result<()> {
    let linux_image = std::env::var("SCRYER_LINUX_CLIPPY_IMAGE")
        .unwrap_or_else(|_| "rust:1.97.1-bookworm".to_string());
    let linux_platform = std::env::var("SCRYER_LINUX_CLIPPY_PLATFORM").ok();

    if !args.linux_only {
        println!("Running cargo clippy for host target");
        let mut command = ctx.command_in("cargo", &ctx.repo_root);
        command.arg("clippy");
        add_ci_clippy_package_args(&mut command);
        command.args(["--", "-D", "warnings"]);
        run_checked(&mut command)?;
    }

    if command_available("docker")? {
        println!("Running cargo clippy in Linux container: {linux_image}");
        let repo_cache_key = release_cache_key(&ctx.repo_root);
        let platform_key = linux_platform.as_deref().unwrap_or("native");
        let cargo_volume = format!("scryer-clippy-cargo-{repo_cache_key}");
        let target_volume = format!(
            "scryer-clippy-target-{repo_cache_key}-{}",
            docker_cache_key_component(platform_key)
        );
        let work_mount = format!("{}:/work", ctx.repo_root.display());
        let cargo_mount = format!("{cargo_volume}:/cargo");
        let target_mount = format!("{target_volume}:/target");
        let clippy_shell = ci_clippy_shell();
        let mut command = ctx.command("docker");
        command.args(["run", "--rm"]);
        if let Some(platform) = linux_platform.as_deref().filter(|value| !value.is_empty()) {
            command.args(["--platform", platform]);
        }
        command.args([
            "-v",
            &work_mount,
            "-v",
            &cargo_mount,
            "-v",
            &target_mount,
            "-w",
            "/work",
            "-e",
            "CARGO_HOME=/cargo",
            "-e",
            "CARGO_TARGET_DIR=/target",
            "-e",
            "CARGO_INCREMENTAL=0",
            "-e",
            "CARGO_PROFILE_DEV_DEBUG=0",
            "-e",
            "CARGO_PROFILE_DEV_STRIP=debuginfo",
            "-e",
            "CARGO_PROFILE_TEST_DEBUG=0",
            "-e",
            "CARGO_PROFILE_TEST_STRIP=debuginfo",
            "-e",
            "CARGO_TERM_COLOR=always",
            &linux_image,
            "bash",
            "-lc",
            &clippy_shell,
        ]);
        run_checked(&mut command)?;
    } else {
        bail!("cannot run Linux clippy locally; install Docker");
    }

    Ok(())
}

fn ci_clippy_shell() -> String {
    let package_args = SCRYER_CI_CLIPPY_PACKAGES
        .iter()
        .map(|package| format!(" -p {package}"))
        .collect::<String>();
    let mut shell = String::from(
        "set -euo pipefail; /usr/local/cargo/bin/rustup component add clippy; toolchain=\"$('/usr/local/cargo/bin/rustup' show active-toolchain | cut -d' ' -f1)\"; toolchain_bin=\"/usr/local/rustup/toolchains/${toolchain}/bin\"; export PATH=\"${toolchain_bin}:$PATH\"; \"${toolchain_bin}/cargo-clippy\" clippy --locked",
    );
    shell.push_str(&package_args);
    shell.push_str(" -- -D warnings");
    shell
}

fn release_cache_key(path: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(path.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
        .chars()
        .take(12)
        .collect()
}

fn docker_cache_key_component(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect()
}

#[derive(Debug, PartialEq, Eq)]
enum TimedCommandOutcome {
    Success,
    Failed(Option<i32>),
    TimedOut,
}

fn run_command_with_timeout(
    command: &mut Command,
    timeout: Duration,
) -> Result<TimedCommandOutcome> {
    let mut child = command.spawn().context("failed to spawn command")?;
    let deadline = Instant::now() + timeout;

    loop {
        if let Some(status) = child.try_wait().context("failed to poll command")? {
            return Ok(if status.success() {
                TimedCommandOutcome::Success
            } else {
                TimedCommandOutcome::Failed(status.code())
            });
        }

        let now = Instant::now();
        if now >= deadline {
            if let Err(error) = child.kill()
                && error.kind() != io::ErrorKind::InvalidInput
            {
                return Err(error).context("failed to terminate timed-out command");
            }
            child.wait().context("failed to reap timed-out command")?;
            return Ok(TimedCommandOutcome::TimedOut);
        }

        thread::sleep((deadline - now).min(Duration::from_millis(100)));
    }
}

fn sync_trash_guides_for_release(ctx: &TaskContext) -> Result<()> {
    step("Syncing TRaSH Guides knowledge");
    let generated_paths = TRASH_GUIDES_GENERATED_PATHS
        .iter()
        .map(|path| ctx.path(path))
        .collect::<Vec<_>>();
    let snapshots = snapshot_files(&generated_paths)?;
    let mut command = ctx.command_in("cargo", &ctx.repo_root);
    command.args([
        "run",
        "--locked",
        "-p",
        "xtask",
        "--",
        "trash-guides",
        "sync",
    ]);

    match run_command_with_timeout(&mut command, TRASH_GUIDES_SYNC_TIMEOUT) {
        Ok(TimedCommandOutcome::Success) => ok("TRaSH Guides sync complete"),
        Ok(TimedCommandOutcome::Failed(code)) => {
            restore_snapshots(snapshots)?;
            warn(format!(
                "TRaSH Guides sync failed with exit code {}; continuing with existing generated artifacts",
                code.map_or_else(|| "unknown".to_string(), |code| code.to_string())
            ));
        }
        Ok(TimedCommandOutcome::TimedOut) => {
            restore_snapshots(snapshots)?;
            warn(format!(
                "TRaSH Guides sync exceeded {}s; continuing with existing generated artifacts",
                TRASH_GUIDES_SYNC_TIMEOUT.as_secs()
            ));
        }
        Err(error) => {
            restore_snapshots(snapshots)?;
            warn(format!(
                "TRaSH Guides sync could not complete ({error:#}); continuing with existing generated artifacts"
            ));
        }
    }
    Ok(())
}

fn run_release(ctx: &TaskContext, args: ReleaseArgs) -> Result<()> {
    step("Determining next version");
    let latest_tag = latest_prefixed_tag(ctx, "scryer-v")?;
    let validation_scope = release_validation_scope(ctx, latest_tag.as_deref())?;
    let current_version = latest_tag
        .as_deref()
        .map(|tag| Version::parse(tag.trim_start_matches("scryer-v")))
        .transpose()?
        .unwrap_or_else(|| Version::new(0, 0, 0));
    let (bump, explicit) = parse_bump(&args)?;
    let release_args =
        release_args_signature(explicit.as_ref(), bump, args.allow_graphql_dangerous);
    let next_version = explicit.unwrap_or_else(|| next_version(&current_version, bump));
    let tag_name = format!("scryer-v{next_version}");
    let catalog_url = OFFICIAL_PLUGIN_CATALOG_V3_REDIRECT_URL.to_string();

    println!(
        "   Latest tag : {}",
        latest_tag.as_deref().unwrap_or("none")
    );
    println!("   Next tag   : {tag_name}");
    println!("   Validation : {}", validation_scope.label());
    if args.dry_run {
        println!("   {YELLOW}(dry run — no commits, tags, or pushes){RESET}");
    }

    step("Pre-flight checks");
    let tags = git_capture(ctx, &["tag"])?;
    if tags.lines().any(|line| line == tag_name) {
        bail!("Tag {tag_name} already exists");
    }
    let branch = current_branch(ctx)?;
    let git_commit = current_head_commit(ctx)?;
    println!("   Branch : {branch}");
    require_app_release_branch(&branch, &next_version)?;
    require_main_merged(ctx)?;
    let worktree_clean_at_start = git_status_porcelain(ctx)?.trim().is_empty();
    if !worktree_clean_at_start {
        prompt_continue_if_dirty(ctx)?;
    }
    require_command("gh")?;
    run_scryer_release_container_contract_validation(ctx, "[release] ")?;
    ok("Pre-flight OK");

    let builtin_plugin_paths = builtin_plugin_paths(ctx);
    let initial_cache_dir = release_dry_run_cache_dir(
        ctx,
        &git_commit,
        &release_args,
        latest_tag.as_deref(),
        &next_version,
        &tag_name,
    );
    let initial_cache_dir_relative = relative_to_repo_root(ctx, &initial_cache_dir)?;
    let expected_release_notes_path = release_notes_path_relative(&tag_name);
    let expected_release_notes_file = ctx.path(&expected_release_notes_path);
    let expected_release_notes_sha256 = if args.dry_run {
        None
    } else if expected_release_notes_file.is_file() {
        Some(release_notes_sha256(&expected_release_notes_file)?)
    } else {
        None
    };

    let mut reused_dry_run_cache = false;
    if args.dry_run {
        clear_release_dry_run_cache(ctx)?;
        write_release_dry_run_cache(
            ctx,
            &ReleaseDryRunCache {
                success: false,
                created_at: Utc::now().to_rfc3339(),
                git_commit: git_commit.clone(),
                branch: branch.clone(),
                worktree_clean_at_start,
                release_args: release_args.clone(),
                latest_tag_seen: latest_tag.clone(),
                next_version: next_version.to_string(),
                tag_name: tag_name.clone(),
                catalog_url: catalog_url.clone(),
                validated_steps: Vec::new(),
                cached_builtins_dir: Some(initial_cache_dir_relative.clone()),
                release_notes_path: None,
                release_notes_sha256: None,
                catalog_builtin_wasm_blake3: BTreeMap::new(),
                failure_message: Some("dry run did not complete".to_string()),
            },
        )?;
    } else if expected_release_notes_sha256.is_none() {
        bail!(
            "release notes are missing for {tag_name}; run `cargo xtask release --dry-run` first"
        );
    } else if worktree_clean_at_start && release_dry_run_cache_path(ctx).is_file() {
        match load_release_dry_run_cache(ctx) {
            Ok(cache) => {
                let next_version_text = next_version.to_string();
                let expected_release_notes_sha256 =
                    expected_release_notes_sha256.as_deref().unwrap_or_default();
                let cached_builtins_dir = cache
                    .cached_builtins_dir
                    .as_deref()
                    .map(|dir| ctx.path(dir));
                let builtins_present = cached_builtins_dir.as_ref().is_some_and(|dir| {
                    builtin_cache_matches_catalog_wasm_blake3(
                        ctx,
                        dir,
                        &cache.catalog_builtin_wasm_blake3,
                    ) && validate_cached_sigstore_trust_root(dir).is_ok()
                });
                let expected = ReleaseDryRunExpectations {
                    git_commit: &git_commit,
                    validation_scope,
                    release_args: &release_args,
                    latest_tag_seen: latest_tag.as_deref(),
                    next_version: &next_version_text,
                    tag_name: &tag_name,
                    release_notes_path: &expected_release_notes_path,
                    release_notes_sha256: expected_release_notes_sha256,
                };
                if let Some(reason) =
                    release_dry_run_cache_rejection_reason(&cache, &expected, builtins_present)
                {
                    println!("   {YELLOW}Skipping dry-run cache reuse: {reason}{RESET}");
                } else {
                    let cached_builtins_dir = cached_builtins_dir.ok_or_else(|| {
                        anyhow!("dry-run cache did not record builtin artifact directory")
                    })?;
                    step("Restoring bundled plugins from dry-run cache");
                    restore_builtin_artifacts_from_cache(
                        ctx,
                        &cached_builtins_dir,
                        &cache.catalog_builtin_wasm_blake3,
                    )?;
                    ok("Reused dry-run cache; skipping builtin rebuild and validations");
                    reused_dry_run_cache = true;
                }
            }
            Err(error) => {
                println!("   {YELLOW}Skipping dry-run cache reuse: {error:#}{RESET}");
            }
        }
    }

    if !args.dry_run && !reused_dry_run_cache {
        bail!(
            "release requires a successful dry run cache with matching release notes; run `cargo xtask release --dry-run` first"
        );
    }

    if !reused_dry_run_cache {
        if validation_scope == ReleaseValidationScope::Full {
            sync_trash_guides_for_release(ctx)?;
        }
        let builtin_manifest = load_builtin_version_manifest(ctx)?;
        let refreshed_builtins =
            refresh_builtin_plugins(ctx, &next_version, Some(&builtin_manifest), false)?;
        let validation_result = match validation_scope {
            ReleaseValidationScope::Full => {
                step("Running web and Rust validation in parallel");
                let (web_tx, web_rx) = mpsc::channel();
                let (rust_tx, rust_rx) = mpsc::channel();
                let web_ctx = ctx.clone();
                let rust_ctx = ctx.clone();

                thread::spawn(move || {
                    let _ = web_tx.send(run_scryer_web_validation(&web_ctx, "[web] "));
                });
                thread::spawn(move || {
                    let _ = rust_tx.send(run_scryer_rust_validation(&rust_ctx, "[rust] "));
                });

                let web_result = web_rx
                    .recv()
                    .context("web validation thread ended unexpectedly")?;
                let rust_result = rust_rx
                    .recv()
                    .context("rust validation thread ended unexpectedly")?;
                if let Err(error) = &web_result {
                    warn(format!("Web validation failed: {error:#}"));
                }
                if let Err(error) = &rust_result {
                    warn(format!("Rust validation failed: {error:#}"));
                }
                web_result?;
                rust_result?;
                run_scryer_graphql_api_compat_validation(
                    ctx,
                    "[graphql] ",
                    latest_tag.as_deref(),
                    &next_version,
                    args.allow_graphql_dangerous,
                )?;
                run_scryer_release_hygiene_validation(ctx, "[hygiene] ")?;
                ok("Full release validation passed");
                Ok::<(BuiltinRefresh, Vec<String>), anyhow::Error>((
                    refreshed_builtins,
                    validation_scope
                        .required_steps()
                        .iter()
                        .map(|step| (*step).to_string())
                        .collect(),
                ))
            }
            ReleaseValidationScope::WebOnly => {
                step("Running web-only release validation");
                run_scryer_web_validation(ctx, "[web] ")?;
                run_scryer_release_hygiene_validation(ctx, "[hygiene] ")?;
                ok("Web-only release validation passed");
                Ok((
                    refreshed_builtins,
                    validation_scope
                        .required_steps()
                        .iter()
                        .map(|step| (*step).to_string())
                        .collect(),
                ))
            }
            ReleaseValidationScope::MetaOnly => {
                step("Skipping application validation for CI/docs-only release");
                run_scryer_release_hygiene_validation(ctx, "[hygiene] ")?;
                ok("CI/docs-only release validation passed");
                Ok((
                    refreshed_builtins,
                    validation_scope
                        .required_steps()
                        .iter()
                        .map(|step| (*step).to_string())
                        .collect(),
                ))
            }
        };

        if args.dry_run {
            match validation_result {
                Ok((refreshed_builtins, validated_steps)) => {
                    let prewritten_release_notes = expected_release_notes_file.is_file();
                    let (release_notes_path, release_notes_sha256) = if prewritten_release_notes {
                        step("Validating prewritten release notes");
                        validate_prewritten_release_notes(&expected_release_notes_file, &tag_name)?;
                        (
                            expected_release_notes_file.clone(),
                            release_notes_sha256(&expected_release_notes_file)?,
                        )
                    } else {
                        step("Generating AI release notes");
                        generate_release_notes(
                            ctx,
                            latest_tag.as_deref(),
                            &tag_name,
                            &next_version,
                        )?
                    };
                    let release_notes_path_relative =
                        relative_to_repo_root(ctx, &release_notes_path)?;
                    ok(if prewritten_release_notes {
                        format!("Using {release_notes_path_relative}")
                    } else {
                        format!("Generated {release_notes_path_relative}")
                    });

                    let mut prep_changed_paths = git_tracked_dirty_paths(ctx)?;
                    maybe_add_changed_graphql_schema_artifact(ctx, &mut prep_changed_paths)?;
                    if changed_file(ctx, &release_notes_path)?
                        && !prep_changed_paths
                            .iter()
                            .any(|path| path == &release_notes_path)
                    {
                        prep_changed_paths.push(release_notes_path);
                    }
                    let final_git_commit = if !prep_changed_paths.is_empty() {
                        step("Committing release-prep changes");
                        let committed = commit_tracked_changes(
                            ctx,
                            &prep_changed_paths,
                            &format!("release: prep scryer {next_version}"),
                        )?
                        .expect("non-empty tracked changes should produce a commit");
                        ok(format!("Committed release-prep changes in {committed}"));
                        committed
                    } else {
                        ok("No release-prep changes to commit");
                        git_commit.clone()
                    };
                    let final_cache_dir = release_dry_run_cache_dir(
                        ctx,
                        &final_git_commit,
                        &release_args,
                        latest_tag.as_deref(),
                        &next_version,
                        &tag_name,
                    );
                    let final_cache_dir_relative = relative_to_repo_root(ctx, &final_cache_dir)?;
                    cache_builtin_artifacts(&final_cache_dir, &refreshed_builtins.paths)?;
                    write_release_dry_run_cache(
                        ctx,
                        &ReleaseDryRunCache {
                            success: true,
                            created_at: Utc::now().to_rfc3339(),
                            git_commit: final_git_commit,
                            branch: branch.clone(),
                            worktree_clean_at_start,
                            release_args: release_args.clone(),
                            latest_tag_seen: latest_tag.clone(),
                            next_version: next_version.to_string(),
                            tag_name: tag_name.clone(),
                            catalog_url: catalog_url.clone(),
                            validated_steps,
                            cached_builtins_dir: Some(final_cache_dir_relative),
                            release_notes_path: Some(release_notes_path_relative),
                            release_notes_sha256: Some(release_notes_sha256),
                            catalog_builtin_wasm_blake3: refreshed_builtins.catalog_wasm_blake3,
                            failure_message: None,
                        },
                    )?;
                    println!(
                        "\n{YELLOW}{BOLD}Dry run complete — stopping before commit/tag/push.{RESET}"
                    );
                    println!("  Version {next_version} validated OK.");
                    println!(
                        "  Dry-run cache: {}",
                        release_dry_run_cache_path(ctx).display()
                    );
                    return Ok(());
                }
                Err(error) => {
                    return Err(error);
                }
            }
        }

        let _ = validation_result?;
    }

    let workspace_tomls = scryer_release_member_tomls(ctx)?;
    if workspace_tomls.is_empty() {
        bail!("No workspace member Cargo.toml files found");
    }
    step(format!(
        "Updating Scryer application crate versions to {next_version}"
    ));
    for toml_path in &workspace_tomls {
        write_package_version(toml_path, &next_version)?;
        let name = toml_path
            .parent()
            .and_then(Path::file_name)
            .and_then(OsStr::to_str)
            .unwrap_or("unknown");
        println!("   bumped: {name} → {next_version}");
    }
    ok(format!(
        "{} crates updated to {}",
        workspace_tomls.len(),
        next_version
    ));
    refresh_release_cargo_lockfile(ctx)?;

    if reused_dry_run_cache {
        ok("Reused dry-run cache for pre-bump validations");
    }

    if validation_scope == ReleaseValidationScope::Full {
        step("Running cargo check after version bump");
        let mut cargo_check = ctx.release_command_in("cargo", &ctx.repo_root);
        cargo_check.arg("check");
        add_prod_package_args(&mut cargo_check);
        run_checked(&mut cargo_check)?;
        ok("cargo check passed");
    }

    step("Committing version bump");
    let mut changed = Vec::new();
    for path in &workspace_tomls {
        if changed_file(ctx, path)? {
            changed.push(path.clone());
        }
    }
    let cargo_lock = ctx.path("Cargo.lock");
    let npm_lock = ctx.path("apps/scryer-web/package-lock.json");
    if cargo_lock.exists() && changed_file(ctx, &cargo_lock)? {
        changed.push(cargo_lock.clone());
    }
    if npm_lock.exists() && changed_file(ctx, &npm_lock)? {
        changed.push(npm_lock.clone());
    }
    maybe_add_changed_graphql_schema_artifact(ctx, &mut changed)?;
    for path in &builtin_plugin_paths {
        if changed_file(ctx, path)? {
            changed.push(path.clone());
        }
    }
    if !changed.is_empty() {
        let mut add = ctx.release_command_in("git", &ctx.repo_root);
        add.arg("add");
        add.args(&changed);
        run_checked(&mut add)?;
        let mut commit = ctx.release_command_in("git", &ctx.repo_root);
        commit.args([
            "commit",
            "-m",
            &format!("release: bump scryer to {next_version}"),
        ]);
        run_checked(&mut commit)?;
        ok("Committed version bump");
    } else {
        ok("Nothing to commit");
    }

    // Stable release artifacts and GHCR tags are intentionally retained.
    step(format!("Creating signed tag {tag_name}"));
    let mut tag = ctx.release_command_in("git", &ctx.repo_root);
    tag.args(["tag", "-s", &tag_name, "-m", &format!("Release {tag_name}")]);
    run_checked(&mut tag)?;
    ok(format!("Tag {tag_name} created"));

    step("Pushing to origin");
    let mut push_branch = ctx.release_command_in("git", &ctx.repo_root);
    push_branch.args(["push", "origin", &branch]);
    run_checked(&mut push_branch)?;
    let mut push_tag = ctx.release_command_in("git", &ctx.repo_root);
    push_tag.args(["push", "origin", &tag_name]);
    run_checked(&mut push_tag)?;
    ok(format!("Pushed {branch} and tag {tag_name}"));

    println!("\n{GREEN}{BOLD}Released {tag_name}{RESET}");
    Ok(())
}

fn run_sdk_release(ctx: &TaskContext, args: SdkReleaseArgs) -> Result<()> {
    step("Preparing plugin SDK release");
    let version = parse_sdk_release_version(&args.version)?;
    let tag_name = sdk_release_tag_name(&version);
    println!("   SDK version : {version}");
    println!("   Next tag    : {tag_name}");
    if args.dry_run {
        println!("   {YELLOW}(dry run — no commits, tags, or pushes){RESET}");
    }

    step("Pre-flight checks");
    let tags = git_capture(ctx, &["tag"])?;
    if tags.lines().any(|line| line == tag_name) {
        bail!("Tag {tag_name} already exists");
    }
    let branch = current_branch(ctx)?;
    println!("   Branch : {branch}");
    ensure_sdk_release_worktree_scope(ctx)?;
    ok("SDK release scope is clean");

    let sdk_manifest = ctx.path(PLUGIN_SDK_MANIFEST);
    let sdk_lib = ctx.path(PLUGIN_SDK_LIB);
    let cargo_lock = ctx.path("Cargo.lock");
    let snapshots = snapshot_files(&[sdk_manifest.clone(), sdk_lib.clone(), cargo_lock.clone()])?;

    step("Updating SDK version metadata");
    write_package_version(&sdk_manifest, &version)?;
    write_sdk_runtime_version(&sdk_lib, &version)?;
    validate_sdk_version_sync(ctx, &version)?;
    ok("SDK package version and SDK_VERSION match");

    step("Updating Cargo.lock metadata");
    let mut cargo_check = ctx.release_command_in("cargo", &ctx.repo_root);
    cargo_check.args(["check", "-p", PLUGIN_SDK_PACKAGE]);
    run_checked(&mut cargo_check)?;
    ok("Cargo.lock metadata refreshed");

    step("Running SDK validation");
    let mut cargo_test = ctx.release_command_in("cargo", &ctx.repo_root);
    cargo_test.args(["test", "--locked", "-p", PLUGIN_SDK_PACKAGE]);
    run_checked(&mut cargo_test)?;
    let mut cargo_package = ctx.release_command_in("cargo", &ctx.repo_root);
    cargo_package.args([
        "package",
        "--locked",
        "-p",
        PLUGIN_SDK_PACKAGE,
        "--allow-dirty",
    ]);
    run_checked(&mut cargo_package)?;
    ok("SDK validation passed");

    if args.dry_run {
        restore_snapshots(snapshots)?;
        println!("\n{YELLOW}{BOLD}Dry run complete — stopping before commit/tag/push.{RESET}");
        println!("  SDK version {version} validated OK.");
        return Ok(());
    }

    step("Collecting SDK release changes");
    let status = git_capture(ctx, &["status", "--porcelain"])?;
    let mut changed = Vec::new();
    let mut unrelated = Vec::new();
    for path in status.lines().filter_map(status_path) {
        if sdk_release_scoped_path(&path) {
            changed.push(ctx.path(&path));
        } else {
            unrelated.push(path);
        }
    }
    if !unrelated.is_empty() {
        bail!(
            "SDK release produced unrelated changes:\n{}",
            unrelated
                .iter()
                .map(|path| format!("  - {path}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
    if changed.is_empty() {
        ok("No SDK release file changes to commit; tagging current HEAD");
    } else {
        let mut add = ctx.release_command_in("git", &ctx.repo_root);
        add.arg("add");
        add.args(&changed);
        run_checked(&mut add)?;
        let mut commit = ctx.release_command_in("git", &ctx.repo_root);
        commit.args([
            "commit",
            "-m",
            &format!("release: publish scryer-plugin-sdk {version}"),
        ]);
        run_checked(&mut commit)?;
        ok("Committed SDK release");
    }

    step(format!("Creating signed tag {tag_name}"));
    let mut tag = ctx.release_command_in("git", &ctx.repo_root);
    tag.args(["tag", "-s", &tag_name, "-m", &format!("Release {tag_name}")]);
    run_checked(&mut tag)?;
    ok(format!("Tag {tag_name} created"));

    step("Pushing to origin");
    let mut push_branch = ctx.release_command_in("git", &ctx.repo_root);
    push_branch.args(["push", "origin", &branch]);
    run_checked(&mut push_branch)?;
    let mut push_tag = ctx.release_command_in("git", &ctx.repo_root);
    push_tag.args(["push", "origin", &tag_name]);
    run_checked(&mut push_tag)?;
    ok(format!("Pushed {branch} and tag {tag_name}"));
    println!("\n{GREEN}{BOLD}Released {tag_name}{RESET}");
    println!("  crates.io publish will run from the plugin-sdk GitHub Actions workflow.");
    Ok(())
}

fn run_scryer_web_validation(ctx: &TaskContext, prefix: &'static str) -> Result<()> {
    let web_dir = ctx.path("apps/scryer-web");
    prefixed_step(prefix, "Running npm audit fix");
    let mut audit = ctx.release_command_in("npm", &web_dir);
    audit.args(["audit", "fix"]);
    run_streaming(&mut audit, prefix)?;
    prefixed_ok(prefix, "npm audit fix complete");

    prefixed_step(prefix, "Running TypeScript type check");
    let mut lint = ctx.release_command_in("npm", &web_dir);
    lint.args(["run", "lint"]);
    run_streaming(&mut lint, prefix)?;
    prefixed_ok(prefix, "TypeScript type check passed");

    prefixed_step(prefix, "Running GraphQL compatibility checker tests");
    let mut graphql_compat_tests = ctx.release_command_in("npm", &web_dir);
    graphql_compat_tests.args(["run", "test:graphql-compat"]);
    run_streaming(&mut graphql_compat_tests, prefix)?;
    prefixed_ok(prefix, "GraphQL compatibility checker tests passed");

    prefixed_step(prefix, "Running web build");
    let mut build = ctx.release_command_in("npm", &web_dir);
    build
        .env("SCRYER_GRAPHQL_URL", "/graphql")
        .env(
            "SCRYER_METADATA_GATEWAY_GRAPHQL_URL",
            "https://smg.scryer.media/graphql",
        )
        .args(["run", "build"]);
    run_streaming(&mut build, prefix)?;
    prefixed_ok(prefix, "Web build passed");
    Ok(())
}

fn run_scryer_rust_validation(ctx: &TaskContext, prefix: &'static str) -> Result<()> {
    prefixed_step(prefix, "Running cargo fmt --all");
    let mut fmt_fix = ctx.release_command_in("cargo", &ctx.repo_root);
    fmt_fix.args(["fmt", "--all"]);
    run_streaming(&mut fmt_fix, prefix)?;
    prefixed_ok(prefix, "cargo fmt complete");

    prefixed_step(prefix, "Running cargo fmt --all --check");
    let mut fmt = ctx.release_command_in("cargo", &ctx.repo_root);
    fmt.args(["fmt", "--all", "--check"]);
    run_streaming(&mut fmt, prefix)?;
    prefixed_ok(prefix, "cargo fmt passed");

    if !command_available("cargo-nextest")? {
        warn("cargo-nextest not installed — installing");
        let mut install = ctx.release_command_in("cargo", &ctx.repo_root);
        install.args(["install", "--locked", "cargo-nextest"]);
        run_streaming(&mut install, prefix)?;
    }

    let cargo_lockfiles = git_tracked_cargo_lockfiles(ctx)?;
    for cargo_lock in &cargo_lockfiles {
        let cargo_dir = cargo_lock
            .parent()
            .context("tracked Cargo.lock did not have a parent directory")?;
        if !cargo_dir.join("Cargo.toml").is_file() {
            bail!(
                "tracked Cargo.lock has no sibling Cargo.toml: {}",
                cargo_lock.display()
            );
        }
        let display_path = cargo_lock
            .strip_prefix(&ctx.repo_root)
            .unwrap_or(cargo_lock)
            .display();
        prefixed_step(prefix, format!("Updating {display_path} (cargo update)"));
        let mut update = ctx.release_command_in("cargo", cargo_dir);
        update.arg("update");
        run_streaming(&mut update, prefix)?;
        prefixed_ok(prefix, format!("{display_path} updated"));
    }

    prefixed_step(
        prefix,
        "Starting Rust tests while other Rust release validations continue",
    );
    let (nextest_tx, nextest_rx) = mpsc::channel();
    let nextest_ctx = ctx.clone();

    thread::spawn(move || {
        let _ = nextest_tx.send(run_scryer_nextest_validation(&nextest_ctx, "[rust-test] "));
    });

    let mut failures = Vec::new();
    let release_checks_result: Result<()> = (|| {
        if !command_available("cargo-audit")? {
            warn("cargo-audit not installed — installing");
            let mut install = ctx.release_command_in("cargo", &ctx.repo_root);
            install.args(["install", "--locked", "cargo-audit"]);
            run_streaming(&mut install, prefix)?;
        }
        let ignores = [
            // Sigstore reaches rsa for public-key verification only. Scryer does not
            // perform the private-key RSA decryption affected by this timing advisory.
            "RUSTSEC-2023-0071",
            "RUSTSEC-2026-0006",
            "RUSTSEC-2026-0020",
            "RUSTSEC-2026-0021",
            // Extism currently pins wasmtime 41.x upstream, so these remain release
            // blockers until the runtime stack moves onto a patched wasmtime line.
            "RUSTSEC-2026-0085",
            "RUSTSEC-2026-0086",
            "RUSTSEC-2026-0087",
            "RUSTSEC-2026-0088",
            "RUSTSEC-2026-0089",
            "RUSTSEC-2026-0091",
            "RUSTSEC-2026-0092",
            "RUSTSEC-2026-0093",
            "RUSTSEC-2026-0094",
            "RUSTSEC-2026-0095",
            "RUSTSEC-2026-0096",
            "RUSTSEC-2026-0114",
        ];
        warn(format!(
            "Ignoring advisories pending upstream fixes: {}",
            ignores.join(" ")
        ));
        for cargo_lock in &cargo_lockfiles {
            let display_path = cargo_lock
                .strip_prefix(&ctx.repo_root)
                .unwrap_or(cargo_lock)
                .display();
            prefixed_step(prefix, format!("Running cargo audit ({display_path})"));
            let mut audit = ctx.release_command_in("cargo", &ctx.repo_root);
            audit.args(["audit", "--file"]);
            audit.arg(cargo_lock);
            for advisory in ignores {
                audit.args(["--ignore", advisory]);
            }
            run_streaming(&mut audit, prefix)?;
            prefixed_ok(prefix, format!("cargo audit passed ({display_path})"));
        }

        run_scryer_ci_clippy_validation(ctx, "[rust-clippy] ")?;
        Ok(())
    })();

    let nextest_result = nextest_rx
        .recv()
        .context("Rust test validation thread ended unexpectedly")?;

    if let Err(error) = release_checks_result {
        failures.push(format!("Rust release validation failed: {error:#}"));
    }
    if let Err(error) = nextest_result {
        failures.push(format!("Rust test validation failed: {error:#}"));
    }
    if !failures.is_empty() {
        for failure in &failures {
            warn(failure);
        }
        bail!(failures.join("\n\n"));
    }

    prefixed_ok(prefix, "Rust release validations and tests passed");
    Ok(())
}

fn run_scryer_graphql_api_compat_validation(
    ctx: &TaskContext,
    prefix: &'static str,
    latest_tag: Option<&str>,
    next_version: &Version,
    allow_graphql_dangerous: bool,
) -> Result<()> {
    prefixed_step(prefix, "Exporting current GraphQL schema");
    let export_dir = ctx.path(GRAPHQL_SCHEMA_EXPORT_DIR);
    fs::create_dir_all(&export_dir)
        .with_context(|| format!("failed to create {}", export_dir.display()))?;
    let current_schema_path = export_dir.join("schema.graphql");
    let previous_schema_path = export_dir.join("previous-schema.graphql");
    let mut export = ctx.command_in("cargo", &ctx.repo_root);
    export.args([
        "run",
        "--locked",
        "--quiet",
        "-p",
        "scryer-interface",
        "--bin",
        "export-graphql-schema",
    ]);
    let current_sdl = run_capture(&mut export).context("failed to export GraphQL schema")?;
    fs::write(&current_schema_path, current_sdl).with_context(|| {
        format!(
            "failed to write current GraphQL schema to {}",
            current_schema_path.display()
        )
    })?;
    prefixed_ok(prefix, "Current GraphQL schema exported");

    let previous_sdl = read_previous_release_graphql_schema(ctx, latest_tag);
    match previous_sdl {
        Ok(previous_sdl) => {
            fs::write(&previous_schema_path, previous_sdl).with_context(|| {
                format!(
                    "failed to write previous GraphQL schema to {}",
                    previous_schema_path.display()
                )
            })?;
            prefixed_step(prefix, "Checking GraphQL API compatibility");
            let web_dir = ctx.path("apps/scryer-web");
            let mut check = ctx.release_command_in("node", &web_dir);
            check.arg("scripts/check-graphql-schema-compat.mjs");
            check.arg(&previous_schema_path);
            check.arg(&current_schema_path);
            if allow_graphql_dangerous {
                check.arg("--allow-dangerous");
            }
            match run_streaming(&mut check, prefix) {
                Ok(()) => prefixed_ok(prefix, "GraphQL API compatibility passed"),
                Err(error) if schema_breaks_allowed_for_bump(latest_tag, next_version) => {
                    warn(format!(
                        "GraphQL API breaking/dangerous changes detected and PERMITTED: this \
                         release raises the minor or major version (next: {next_version}). The \
                         full change list is streamed above — every break must be enumerated \
                         in the release notes. Checker result: {error:#}"
                    ));
                }
                Err(error) => {
                    return Err(error).with_context(|| {
                        "GraphQL API compatibility failed for a patch release — breaking \
                         schema changes are only permitted when the minor or major version \
                         increases"
                            .to_string()
                    });
                }
            }
        }
        Err(error) if allow_missing_previous_graphql_schema(next_version) => {
            warn(format!(
                "Bootstrapping GraphQL API baseline for {next_version}; previous schema was unavailable: {error:#}"
            ));
        }
        Err(error) => {
            bail!(
                "previous release GraphQL schema is required after {GRAPHQL_API_BASELINE_VERSION}: {error:#}"
            );
        }
    }

    update_graphql_schema_artifact(ctx, &current_schema_path)?;
    prefixed_ok(prefix, "GraphQL schema artifact updated");
    Ok(())
}

fn read_previous_release_graphql_schema(
    ctx: &TaskContext,
    latest_tag: Option<&str>,
) -> Result<String> {
    let latest_tag =
        latest_tag.ok_or_else(|| anyhow!("no previous scryer release tag was found"))?;
    let spec = format!("{latest_tag}:{GRAPHQL_SCHEMA_ARTIFACT}");
    let mut show = ctx.command_in("git", &ctx.repo_root);
    show.args(["show", &spec]);
    run_capture(&mut show)
        .with_context(|| format!("failed to read {GRAPHQL_SCHEMA_ARTIFACT} from {latest_tag}"))
}

/// Breaking/dangerous GraphQL schema changes are permitted only when the
/// release raises the minor or major version (e.g. 0.16.x → 0.17.0 — a major
/// Scryer release under 0.x versioning); patch releases keep the hard
/// compatibility failure.
fn schema_breaks_allowed_for_bump(latest_tag: Option<&str>, next_version: &Version) -> bool {
    let Some(tag) = latest_tag else {
        return false;
    };
    let Ok(previous) = Version::parse(tag.trim_start_matches("scryer-v")) else {
        return false;
    };
    next_version.major > previous.major
        || (next_version.major == previous.major && next_version.minor > previous.minor)
}

fn update_graphql_schema_artifact(ctx: &TaskContext, current_schema_path: &Path) -> Result<()> {
    let artifact = ctx.path(GRAPHQL_SCHEMA_ARTIFACT);
    if let Some(parent) = artifact.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::copy(current_schema_path, &artifact).with_context(|| {
        format!(
            "failed to copy {} to {}",
            current_schema_path.display(),
            artifact.display()
        )
    })?;
    Ok(())
}

fn maybe_add_changed_graphql_schema_artifact(
    ctx: &TaskContext,
    changed: &mut Vec<PathBuf>,
) -> Result<()> {
    let artifact = ctx.path(GRAPHQL_SCHEMA_ARTIFACT);
    if changed_file(ctx, &artifact)? && !changed.iter().any(|path| path == &artifact) {
        changed.push(artifact);
    }
    Ok(())
}

fn run_scryer_release_container_contract_validation(
    ctx: &TaskContext,
    prefix: &'static str,
) -> Result<()> {
    prefixed_step(prefix, "Checking Docker release publish contract");
    let mut command = ctx.command("sh");
    command.arg("docker/validate-release-build-config.sh");
    run_checked(&mut command)?;
    prefixed_ok(prefix, "Docker release publish contract passed");
    Ok(())
}

fn run_scryer_release_hygiene_validation(ctx: &TaskContext, prefix: &'static str) -> Result<()> {
    prefixed_step(prefix, "Checking release hygiene");
    let violations = release_hygiene_violations(ctx)?;
    if !violations.is_empty() {
        bail!(
            "release hygiene check failed:\n{}",
            violations
                .into_iter()
                .map(|violation| format!("  - {violation}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
    prefixed_ok(prefix, "Release hygiene check passed");
    Ok(())
}

fn run_scryer_ci_clippy_validation(ctx: &TaskContext, prefix: &'static str) -> Result<()> {
    prefixed_step(
        prefix,
        "Running CI-equivalent clippy for scryer production binary packages",
    );
    run_clippy_ci(ctx, ClippyArgs { linux_only: true })?;
    prefixed_ok(prefix, "CI clippy passed");
    Ok(())
}

fn run_scryer_nextest_validation(ctx: &TaskContext, prefix: &'static str) -> Result<()> {
    prefixed_step(
        prefix,
        "Running Rust tests for scryer production binary packages",
    );
    let mut nextest = ctx.release_command_in("cargo", &ctx.repo_root);
    nextest.args(["nextest", "run"]);
    add_prod_package_args(&mut nextest);
    nextest.arg("--locked");
    run_streaming(&mut nextest, prefix)?;
    prefixed_ok(prefix, "Rust tests passed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TIMED_COMMAND_CHILD_ENV: &str = "SCRYER_XTASK_TIMED_COMMAND_CHILD";

    fn write_upgrade_manifest_fixture(artifacts_dir: &Path) {
        fs::create_dir_all(artifacts_dir).expect("create fixture artifact directory");
        for spec in UPGRADE_MANIFEST_ASSETS {
            let path = artifacts_dir.join(spec.asset_name);
            match spec.archive {
                UpgradeArchive::TarGz => {
                    write_fixture_tar_gz(&path, spec.asset_name, fixture_members(spec.platform))
                }
                UpgradeArchive::Msi => {
                    fs::write(&path, format!("fixture MSI {}\n", spec.asset_name))
                        .expect("write fixture MSI");
                }
            }
        }
    }

    /// The member layout each platform's portable tarball actually ships.
    fn fixture_members(platform: UpgradePlatform) -> &'static [(&'static str, u32)] {
        match platform {
            UpgradePlatform::Windows => &[
                ("scryer.exe", 0o755),
                ("scryer-tray.exe", 0o755),
                ("LICENSE", 0o644),
                ("README.txt", 0o644),
            ],
            UpgradePlatform::Darwin | UpgradePlatform::Linux => &[("bin/scryer", 0o755)],
        }
    }

    fn write_fixture_tar_gz(path: &Path, asset_name: &str, members: &[(&str, u32)]) {
        let file = fs::File::create(path).expect("create fixture tarball");
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        for (member, mode) in members {
            let content = format!("fixture tar member {member} for {asset_name}\n");
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(*mode);
            header.set_mtime(0);
            header.set_uid(0);
            header.set_gid(0);
            header.set_cksum();
            builder
                .append_data(&mut header, member, content.as_bytes())
                .expect("append fixture tar member");
        }
        let encoder = builder.into_inner().expect("finish fixture tar archive");
        encoder.finish().expect("finish fixture gzip stream");
    }

    #[test]
    fn upgrade_manifest_generation_is_deterministic_and_matches_the_golden_fixture() {
        let tempdir = tempfile::tempdir().expect("create tempdir");
        let artifacts_dir = tempdir.path().join("artifacts");
        write_upgrade_manifest_fixture(&artifacts_dir);

        let first = generate_upgrade_manifest(
            "9.8.7",
            "scryer-v9.8.7",
            "scryer-media/scryer",
            &artifacts_dir,
        )
        .expect("generate first upgrade manifest");
        let second = generate_upgrade_manifest(
            "9.8.7",
            "scryer-v9.8.7",
            "scryer-media/scryer",
            &artifacts_dir,
        )
        .expect("generate second upgrade manifest");

        assert_eq!(first, second);
        parse_and_validate_upgrade_manifest(&first).expect("generated manifest is valid");
        assert_eq!(
            String::from_utf8(first).expect("manifest is UTF-8"),
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../api/upgrade/manifest.v1.example.json"
            ))
        );
    }

    #[test]
    fn upgrade_manifest_generation_requires_all_expected_assets() {
        let tempdir = tempfile::tempdir().expect("create tempdir");
        let artifacts_dir = tempdir.path().join("artifacts");
        write_upgrade_manifest_fixture(&artifacts_dir);
        let missing = artifacts_dir.join("scryer-windows-arm64.msi");
        fs::remove_file(&missing).expect("remove required fixture asset");

        let error = generate_upgrade_manifest(
            "9.8.7",
            "scryer-v9.8.7",
            "scryer-media/scryer",
            &artifacts_dir,
        )
        .expect_err("missing asset must fail manifest generation");
        assert!(error.to_string().contains("scryer-windows-arm64.msi"));
    }

    #[test]
    fn tracked_cargo_lockfiles_include_independent_projects() {
        let ctx = TaskContext::new();
        let lockfiles = git_tracked_cargo_lockfiles(&ctx)
            .expect("list tracked Cargo lockfiles")
            .into_iter()
            .map(|path| {
                path.strip_prefix(&ctx.repo_root)
                    .expect("lockfile is inside repository")
                    .to_path_buf()
            })
            .collect::<BTreeSet<_>>();

        for expected in [
            PathBuf::from("Cargo.lock"),
            PathBuf::from("crates/scryer-release-parser/fuzz/Cargo.lock"),
            PathBuf::from("test-plugins/test-indexer/Cargo.lock"),
        ] {
            assert!(
                lockfiles.contains(&expected),
                "missing {}",
                expected.display()
            );
        }
    }

    #[test]
    fn release_version_bump_targets_include_split_interface_crates() {
        let ctx = TaskContext::new();
        let members = scryer_release_member_tomls(&ctx)
            .expect("resolve release workspace members")
            .into_iter()
            .map(|path| package_name(&path).expect("read release package name"))
            .collect::<BTreeSet<_>>();

        for expected in [
            "scryer-interface-acquisition",
            "scryer-interface-import",
            "scryer-interface-query",
            "scryer-interface-security",
            "scryer-interface-subscription",
            "scryer-interface-system",
        ] {
            assert!(
                members.contains(expected),
                "release workspace members must include {expected}"
            );
        }
    }

    #[test]
    fn release_validation_targets_include_split_infrastructure_crates() {
        for expected in [
            "scryer-infrastructure-acquisition",
            "scryer-infrastructure-configuration",
            "scryer-infrastructure-crypto",
            "scryer-infrastructure-datastore",
            "scryer-infrastructure-identity",
            "scryer-infrastructure-library",
            "scryer-infrastructure-library-search",
            "scryer-infrastructure-metadata",
            "scryer-infrastructure-notifications",
            "scryer-infrastructure-runtime",
            "scryer-infrastructure-sql",
            "scryer-infrastructure-workflow",
        ] {
            assert!(
                SCRYER_PROD_PACKAGES.contains(&expected),
                "release Nextest packages must include {expected}"
            );
            assert!(
                SCRYER_CI_CLIPPY_PACKAGES.contains(&expected),
                "release Clippy packages must include {expected}"
            );
        }
    }

    #[test]
    fn app_release_branch_must_match_target_version() {
        let version = Version::parse("0.17.4").unwrap();
        assert!(require_app_release_branch("release-0.17.4", &version).is_ok());

        let error = require_app_release_branch("main", &version).unwrap_err();
        assert!(error.to_string().contains("release-0.17.4"));
    }

    #[test]
    fn timed_command_reports_success_and_failure() {
        let executable = std::env::current_exe().unwrap();
        let mut success = Command::new(&executable);
        success
            .arg("--list")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        assert_eq!(
            run_command_with_timeout(&mut success, Duration::from_secs(5)).unwrap(),
            TimedCommandOutcome::Success
        );

        let mut failure = Command::new(executable);
        failure
            .arg("--definitely-not-a-test-harness-option")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        assert!(matches!(
            run_command_with_timeout(&mut failure, Duration::from_secs(5)).unwrap(),
            TimedCommandOutcome::Failed(_)
        ));
    }

    #[test]
    fn timed_command_kills_and_reaps_child() {
        if std::env::var_os(TIMED_COMMAND_CHILD_ENV).is_some() {
            thread::sleep(Duration::from_secs(5));
            return;
        }

        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "tests::timed_command_kills_and_reaps_child",
                "--nocapture",
            ])
            .env(TIMED_COMMAND_CHILD_ENV, "1")
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        assert_eq!(
            run_command_with_timeout(&mut command, Duration::from_millis(50)).unwrap(),
            TimedCommandOutcome::TimedOut
        );
    }

    #[test]
    fn file_snapshots_restore_existing_and_remove_new_outputs() {
        let temp = tempfile::tempdir().unwrap();
        let existing = temp.path().join("existing.txt");
        let created = temp.path().join("created.txt");
        fs::write(&existing, b"before").unwrap();
        let snapshots = snapshot_files(&[existing.clone(), created.clone()]).unwrap();

        fs::write(&existing, b"after").unwrap();
        fs::write(&created, b"partial").unwrap();
        restore_snapshots(snapshots).unwrap();

        assert_eq!(fs::read(existing).unwrap(), b"before");
        assert!(!created.exists());
    }

    #[test]
    fn schema_breaks_allowed_only_for_minor_or_major_bump() {
        assert!(schema_breaks_allowed_for_bump(
            Some("scryer-v0.16.8"),
            &Version::new(0, 17, 0)
        ));
        assert!(schema_breaks_allowed_for_bump(
            Some("scryer-v0.17.3"),
            &Version::new(1, 0, 0)
        ));
        assert!(!schema_breaks_allowed_for_bump(
            Some("scryer-v0.17.0"),
            &Version::new(0, 17, 1)
        ));
        assert!(!schema_breaks_allowed_for_bump(
            None,
            &Version::new(0, 17, 0)
        ));
        assert!(!schema_breaks_allowed_for_bump(
            Some("not-a-version"),
            &Version::new(0, 17, 0)
        ));
    }

    fn catalog_v3_plugin_artifact(
        url: &str,
        runtime: &str,
        required_features: Vec<&str>,
    ) -> CatalogV3PluginArtifact {
        CatalogV3PluginArtifact {
            runtime: runtime.to_string(),
            required_features: required_features.into_iter().map(str::to_string).collect(),
            url: url.to_string(),
            signature_url: format!("{url}.bundle.zst"),
            digests: vec!["blake3:compressed".to_string()],
            wasm_digests: vec!["sha256:ignored".to_string(), "blake3:wasm".to_string()],
        }
    }

    #[test]
    fn catalog_v3_signer_uses_release_plugin_v3_workflow() {
        let signer = official_plugin_v3_signer();

        assert_eq!(signer.github_repository, OFFICIAL_PLUGIN_REPO);
        assert_eq!(
            signer.github_workflow.as_deref(),
            Some(OFFICIAL_PLUGIN_V3_RELEASE_WORKFLOW)
        );
    }

    #[test]
    fn baseline_catalog_v3_zstd_artifact_selects_unfeatured_wasip1_zstd() {
        let release = CatalogV3Release {
            version: "0.2.15".to_string(),
            min_scryer_version: Some("0.16.0".to_string()),
            max_scryer_version: None,
            sdk_constraint: Some(">=3.0.0, <4.0.0".to_string()),
            artifacts: vec![
                catalog_v3_plugin_artifact(
                    "https://cdn.example/newznab.wasm.br",
                    "wasm32-wasip1",
                    vec![],
                ),
                catalog_v3_plugin_artifact(
                    "https://cdn.example/newznab-simd.wasm.zst",
                    "wasm32-wasip1",
                    vec!["simd128"],
                ),
                catalog_v3_plugin_artifact(
                    "https://cdn.example/newznab.wasm.zst",
                    "wasm32-wasip1",
                    vec![],
                ),
            ],
        };

        let artifact = baseline_catalog_v3_zstd_artifact("newznab", &release).unwrap();

        assert_eq!(artifact.url, "https://cdn.example/newznab.wasm.zst");
    }

    #[test]
    fn baseline_catalog_v3_zstd_artifact_prefers_wasip2() {
        let release = CatalogV3Release {
            version: "2.0.4".to_string(),
            min_scryer_version: Some("0.18.22".to_string()),
            max_scryer_version: None,
            sdk_constraint: Some(">=3.9.0, <4.0.0".to_string()),
            artifacts: vec![
                catalog_v3_plugin_artifact(
                    "https://cdn.example/newznab-legacy.wasm.zst",
                    "wasm32-wasip1",
                    vec![],
                ),
                catalog_v3_plugin_artifact(
                    "https://cdn.example/newznab-component.wasm.zst",
                    "wasm32-wasip2",
                    vec![],
                ),
            ],
        };

        let artifact = baseline_catalog_v3_zstd_artifact("newznab", &release).unwrap();

        assert_eq!(
            artifact.url,
            "https://cdn.example/newznab-component.wasm.zst"
        );
    }

    #[test]
    fn required_blake3_digest_ignores_other_digest_algorithms() {
        let digests = vec!["sha256:ignored".to_string(), "blake3:expected".to_string()];

        assert_eq!(
            required_blake3_digest("test artifact", &digests).unwrap(),
            "blake3:expected"
        );
        assert!(required_blake3_digest("test artifact", &[]).is_err());
    }

    #[test]
    fn builtin_descriptor_sdk_contract_must_support_host_sdk() {
        assert!(
            require_builtin_descriptor_sdk_contract(
                "newznab",
                scryer_plugin_sdk::SDK_VERSION,
                &scryer_plugin_sdk::current_sdk_constraint(),
            )
            .is_ok()
        );

        let current_sdk = Version::parse(scryer_plugin_sdk::SDK_VERSION).unwrap();
        let mismatched_sdk = Version::new(current_sdk.major, current_sdk.minor + 1, 0).to_string();
        let error = require_builtin_descriptor_sdk_contract("newznab", &mismatched_sdk, "")
            .expect_err("newer catalog SDK must be rejected");
        assert!(error.to_string().contains("host sdk_version"), "{error:#}");
    }

    #[test]
    fn builtin_version_manifest_requires_exact_builtin_set() {
        let pin = |digest: char| BuiltinVersionPin {
            version: "2.0.1".to_string(),
            wasm_blake3: std::iter::repeat_n(digest, 64).collect(),
        };
        let valid = BuiltinVersionManifest {
            schema_version: 2,
            plugins: BTreeMap::from([
                ("newznab".to_string(), pin('a')),
                ("torznab".to_string(), pin('b')),
            ]),
        };
        assert!(validate_builtin_version_manifest(&valid).is_ok());

        let incomplete = BuiltinVersionManifest {
            schema_version: 2,
            plugins: BTreeMap::from([("newznab".to_string(), pin('a'))]),
        };
        assert!(validate_builtin_version_manifest(&incomplete).is_err());

        let mut bad_digest = valid.clone();
        bad_digest
            .plugins
            .get_mut("newznab")
            .expect("newznab pin")
            .wasm_blake3 = "not-hex".to_string();
        assert!(validate_builtin_version_manifest(&bad_digest).is_err());
    }

    #[test]
    fn builtin_wasm_digest_comparison_tolerates_the_prefix_spelling() {
        assert!(builtin_wasm_digest_matches(
            "b9a72d60fb1ea0a933e5c5b3caee581133e562394fc705c75c8e58c5124b9c67",
            "blake3:B9A72D60FB1EA0A933E5C5B3CAEE581133E562394FC705C75C8E58C5124B9C67",
        ));
        assert!(!builtin_wasm_digest_matches(
            "b9a72d60fb1ea0a933e5c5b3caee581133e562394fc705c75c8e58c5124b9c67",
            "blake3:d8cb9d017659a50241b9831164662851b4393718c0f1524b2059f48f909d54c4",
        ));
    }

    #[test]
    fn catalog_builtin_release_must_support_target_scryer_and_sdk() {
        let current_sdk = Version::parse(scryer_plugin_sdk::SDK_VERSION).unwrap();
        let next_sdk_minor_constraint = format!(
            ">={}.{}.0, <{}.0.0",
            current_sdk.major,
            current_sdk.minor + 1,
            current_sdk.major + 1
        );
        let compatible = CatalogV3Release {
            version: "0.2.16".to_string(),
            min_scryer_version: Some("0.16.0".to_string()),
            max_scryer_version: Some("0.16.6".to_string()),
            sdk_constraint: Some(">=3.0.0, <4.0.0".to_string()),
            artifacts: vec![],
        };
        let too_new_scryer = CatalogV3Release {
            version: "0.2.18".to_string(),
            min_scryer_version: Some("0.17.0".to_string()),
            max_scryer_version: None,
            sdk_constraint: Some(">=3.0.0, <4.0.0".to_string()),
            artifacts: vec![],
        };
        let retired_scryer = CatalogV3Release {
            version: "0.2.17".to_string(),
            min_scryer_version: Some("0.16.0".to_string()),
            max_scryer_version: Some("0.16.5".to_string()),
            sdk_constraint: Some(">=3.0.0, <4.0.0".to_string()),
            artifacts: vec![],
        };
        let too_new_sdk = CatalogV3Release {
            version: "0.2.19".to_string(),
            min_scryer_version: Some("0.16.0".to_string()),
            max_scryer_version: None,
            sdk_constraint: Some(next_sdk_minor_constraint),
            artifacts: vec![],
        };
        let target = Version::parse("0.16.6").unwrap();

        assert!(catalog_release_is_builtin_compatible("newznab", &compatible, &target).unwrap());
        assert!(
            !catalog_release_is_builtin_compatible("newznab", &too_new_scryer, &target).unwrap()
        );
        assert!(
            !catalog_release_is_builtin_compatible("newznab", &retired_scryer, &target).unwrap()
        );
        assert!(!catalog_release_is_builtin_compatible("newznab", &too_new_sdk, &target).unwrap());
    }

    fn sample_release_dry_run_cache() -> ReleaseDryRunCache {
        ReleaseDryRunCache {
            success: true,
            created_at: "2026-05-02T00:00:00Z".to_string(),
            git_commit: "abc123".to_string(),
            branch: "main".to_string(),
            worktree_clean_at_start: true,
            release_args: "bump:patch".to_string(),
            latest_tag_seen: Some("scryer-v0.13.1".to_string()),
            next_version: "0.13.2".to_string(),
            tag_name: "scryer-v0.13.2".to_string(),
            catalog_url: OFFICIAL_PLUGIN_CATALOG_V3_REDIRECT_URL.to_string(),
            validated_steps: REQUIRED_SCRYER_DRY_RUN_STEPS
                .iter()
                .map(|step| (*step).to_string())
                .collect(),
            cached_builtins_dir: Some("tmp/cache".to_string()),
            catalog_builtin_wasm_blake3: BTreeMap::from([
                ("newznab".to_string(), "blake3:newznab".to_string()),
                ("torznab".to_string(), "blake3:torznab".to_string()),
            ]),
            release_notes_path: Some("release-notes/scryer-v0.13.2.md".to_string()),
            release_notes_sha256: Some("sha256:release-notes".to_string()),
            failure_message: None,
        }
    }

    fn sample_release_dry_run_expectations<'a>() -> ReleaseDryRunExpectations<'a> {
        ReleaseDryRunExpectations {
            git_commit: "abc123",
            validation_scope: ReleaseValidationScope::Full,
            release_args: "bump:patch",
            latest_tag_seen: Some("scryer-v0.13.1"),
            next_version: "0.13.2",
            tag_name: "scryer-v0.13.2",
            release_notes_path: "release-notes/scryer-v0.13.2.md",
            release_notes_sha256: "sha256:release-notes",
        }
    }

    #[test]
    fn release_validation_scope_is_meta_only_for_ci_and_docs() {
        let paths = [
            Path::new(".github/workflows/scryer.yml"),
            Path::new("README.md"),
            Path::new("docs/releasing.md"),
        ];
        assert_eq!(
            classify_release_validation_paths(paths),
            ReleaseValidationScope::MetaOnly
        );
    }

    #[test]
    fn release_validation_scope_is_meta_only_without_product_changes() {
        assert_eq!(
            classify_release_validation_paths([]),
            ReleaseValidationScope::MetaOnly
        );
    }

    #[test]
    fn release_validation_scope_is_web_only_for_web_and_ci_paths() {
        let paths = [
            Path::new("apps/scryer-web/src/app.tsx"),
            Path::new("apps/scryer-web/package-lock.json"),
            Path::new(".github/workflows/scryer.yml"),
        ];
        assert_eq!(
            classify_release_validation_paths(paths),
            ReleaseValidationScope::WebOnly
        );
    }

    #[test]
    fn release_validation_scope_fails_closed_for_non_web_product_paths() {
        let paths = [
            Path::new("apps/scryer-web/src/app.tsx"),
            Path::new("crates/scryer/src/main.rs"),
        ];
        assert_eq!(
            classify_release_validation_paths(paths),
            ReleaseValidationScope::Full
        );
    }

    #[test]
    fn release_notes_path_uses_tag_name() {
        let ctx = TaskContext::new();
        let path = release_notes_path(&ctx, "scryer-v1.2.3");
        assert_eq!(path, ctx.path("release-notes/scryer-v1.2.3.md"));
        assert_eq!(
            release_notes_path_relative("scryer-v1.2.3"),
            "release-notes/scryer-v1.2.3.md"
        );
    }

    #[test]
    fn release_notes_validation_requires_ai_marker() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("notes.md");
        fs::write(
            &path,
            "# scryer-v1.2.3\n\n- User-facing release note without marker.\n",
        )
        .unwrap();

        let error = validate_release_notes_output(&path, "scryer-v1.2.3").unwrap_err();
        assert!(
            format!("{error:#}")
                .contains("generated release notes must include `AI generated release notes`")
        );
    }

    #[test]
    fn release_notes_validation_accepts_ai_marker() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("notes.md");
        fs::write(
            &path,
            "# scryer-v1.2.3\n\nAI generated release notes\n\n- Improved release notes.\n",
        )
        .unwrap();

        validate_release_notes_output(&path, "scryer-v1.2.3").unwrap();
    }

    #[test]
    fn prewritten_release_notes_accept_human_heading_without_ai_marker() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("notes.md");
        fs::write(
            &path,
            "# Scryer 1.2.3 release notes\n\n- Improved release notes.\n",
        )
        .unwrap();

        validate_prewritten_release_notes(&path, "scryer-v1.2.3").unwrap();
    }

    #[test]
    fn release_notes_codex_command_uses_gpt_54_xhigh_by_default() {
        let command = codex_release_notes_command_for(
            Path::new("release-notes/scryer-v1.2.3.md"),
            RELEASE_NOTES_DEFAULT_CODEX_MODEL,
            RELEASE_NOTES_DEFAULT_CODEX_REASONING,
        );
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert_eq!(command.get_program().to_string_lossy(), "codex");
        assert!(args.windows(2).any(|window| {
            window[0] == "--model" && window[1] == RELEASE_NOTES_DEFAULT_CODEX_MODEL
        }));
        assert!(args.windows(2).any(|window| {
            window[0] == "-c"
                && window[1]
                    == format!(
                        "model_reasoning_effort=\"{}\"",
                        RELEASE_NOTES_DEFAULT_CODEX_REASONING
                    )
        }));
        assert!(!args.contains(&"--ask-for-approval".to_string()));
        assert!(args.contains(&"--output-last-message".to_string()));
        assert_eq!(args.last().map(String::as_str), Some("-"));
    }

    #[test]
    fn release_notes_command_override_receives_environment() {
        let ctx = TaskContext::new();
        let temp = tempfile::tempdir().unwrap();
        let context_path = temp.path().join("context.md");
        let output_path = temp.path().join("output.md");
        fs::write(&context_path, "release context").unwrap();
        let command = r#"test "$SCRYER_RELEASE_NOTES_CONTEXT" = "release context" && test -s "$SCRYER_RELEASE_NOTES_CONTEXT_PATH" && printf '# %s\n\nAI generated release notes\n\n- Generated by override.\n' "$SCRYER_RELEASE_TAG" > "$SCRYER_RELEASE_NOTES_OUTPUT""#;

        run_release_notes_command_with_template(
            &ctx,
            &context_path,
            &output_path,
            "scryer-v1.2.3",
            Some("scryer-v1.2.2"),
            &Version::parse("1.2.3").unwrap(),
            "release context",
            Some(command),
        )
        .unwrap();

        validate_release_notes_output(&output_path, "scryer-v1.2.3").unwrap();
    }

    #[test]
    fn sdk_release_tag_uses_independent_prefix() {
        let version = Version::parse("1.0.0").unwrap();
        assert_eq!(sdk_release_tag_name(&version), "plugin-sdk-v1.0.0");
    }

    #[test]
    fn sdk_release_version_rejects_leading_v() {
        assert!(parse_sdk_release_version("v1.0.0").is_err());
    }

    #[test]
    fn sdk_release_scope_excludes_unrelated_paths() {
        assert!(sdk_release_scoped_path(
            "crates/scryer-plugin-sdk/src/lib.rs"
        ));
        assert!(sdk_release_scoped_path("Cargo.lock"));
        assert!(sdk_release_scoped_path(".github/workflows/plugin-sdk.yml"));
        assert!(sdk_release_scoped_path("xtask/Cargo.toml"));
        assert!(sdk_release_scoped_path("xtask/src/main.rs"));
        assert!(!sdk_release_scoped_path(
            "crates/scryer-application/Cargo.toml"
        ));
    }

    #[test]
    fn sdk_runtime_version_round_trips_constant() {
        let source = "pub const SDK_VERSION: &str = \"1.4.0\";\n";
        let updated =
            replace_sdk_runtime_version(source, &Version::parse("1.0.0").unwrap()).unwrap();
        assert_eq!(
            sdk_runtime_version_from_source(&updated).unwrap(),
            Version::parse("1.0.0").unwrap()
        );
    }

    #[test]
    fn app_release_package_filter_excludes_non_app_release_crates() {
        assert!(!is_scryer_app_release_package("scryer-plugin-sdk"));
        assert!(!is_scryer_app_release_package("xtask"));
        assert!(!is_scryer_app_release_package("xtask-release"));
        assert!(!is_scryer_app_release_package("xtask-migrations"));
        assert!(!is_scryer_app_release_package("xtask-support"));
        assert!(is_scryer_app_release_package("scryer"));
    }

    fn sample_winget_artifacts() -> Vec<WingetArtifact> {
        vec![
            WingetArtifact {
                architecture: "x64",
                asset_name: WINGET_WINDOWS_X64_ASSET,
                installer_url: format!(
                    "https://github.com/scryer-media/scryer/releases/download/scryer-v0.16.5/{WINGET_WINDOWS_X64_ASSET}"
                ),
                installer_sha256: "A".repeat(64),
                product_code: "{12345678-1234-1234-1234-1234567890AB}".to_string(),
            },
            WingetArtifact {
                architecture: "arm64",
                asset_name: WINGET_WINDOWS_ARM64_ASSET,
                installer_url: format!(
                    "https://github.com/scryer-media/scryer/releases/download/scryer-v0.16.5/{WINGET_WINDOWS_ARM64_ASSET}"
                ),
                installer_sha256: "B".repeat(64),
                product_code: "{87654321-4321-4321-4321-BA0987654321}".to_string(),
            },
        ]
    }

    #[test]
    fn winget_installer_manifest_uses_msi_contract() {
        let version = Version::parse("0.16.5").unwrap();
        let manifest =
            winget_installer_manifest(&version, "2026-06-24", &sample_winget_artifacts());

        assert!(manifest.contains("PackageIdentifier: ScryerMedia.Scryer"));
        assert!(manifest.contains("PackageVersion: 0.16.5"));
        assert!(manifest.contains("winget-manifest.installer.1.10.0.schema.json"));
        assert!(manifest.contains("ManifestVersion: 1.10.0"));
        assert!(manifest.contains("InstallerType: msi"));
        assert!(manifest.contains("UpgradeBehavior: uninstallPrevious"));
        assert!(manifest.contains("ProductCode: '{12345678-1234-1234-1234-1234567890AB}'"));
        assert!(manifest.contains("Architecture: x64"));
        assert!(manifest.contains("Architecture: arm64"));
        assert!(manifest.contains(WINGET_WINDOWS_X64_ASSET));
        assert!(manifest.contains(WINGET_WINDOWS_ARM64_ASSET));
        assert!(manifest.contains("ReleaseDate: 2026-06-24"));
    }

    #[test]
    fn msi_product_code_validation_rejects_malformed_values() {
        assert!(is_msi_product_code(
            "{12345678-1234-1234-1234-1234567890AB}"
        ));
        assert!(!is_msi_product_code("12345678-1234-1234-1234-1234567890AB"));
        assert!(!is_msi_product_code(
            "{12345678-1234-1234-1234-1234567890AG}"
        ));
        assert!(!is_msi_product_code(
            "{12345678_1234_1234_1234_1234567890AB}"
        ));
    }

    #[test]
    fn winget_locale_manifest_matches_scryer_identity() {
        let version = Version::parse("0.16.5").unwrap();
        let manifest = winget_locale_manifest(&version);

        assert!(manifest.contains("PackageIdentifier: ScryerMedia.Scryer"));
        assert!(manifest.contains("winget-manifest.defaultLocale.1.10.0.schema.json"));
        assert!(manifest.contains("ManifestVersion: 1.10.0"));
        assert!(manifest.contains("Publisher: Scryer Media"));
        assert!(manifest.contains("PackageName: Scryer"));
        assert!(manifest.contains("License: GPL-3.0"));
        assert!(manifest.contains("Moniker: scryer"));
        assert!(manifest.contains(
            "ReleaseNotesUrl: https://github.com/scryer-media/scryer/releases/tag/scryer-v0.16.5"
        ));
    }

    #[test]
    fn winget_manifest_writer_uses_package_version_directory() {
        let output_dir = tempfile::tempdir().unwrap();
        let version = Version::parse("0.16.5").unwrap();

        let manifest_dir = write_winget_manifests(
            output_dir.path(),
            &version,
            "2026-06-24",
            &sample_winget_artifacts(),
        )
        .unwrap();

        assert_eq!(
            manifest_dir.strip_prefix(output_dir.path()).unwrap(),
            Path::new("ScryerMedia.Scryer").join("0.16.5")
        );
        assert!(
            manifest_dir
                .join("ScryerMedia.Scryer.installer.yaml")
                .is_file()
        );
        assert!(manifest_dir.join("ScryerMedia.Scryer.yaml").is_file());
        assert!(
            fs::read_to_string(manifest_dir.join("ScryerMedia.Scryer.yaml"))
                .unwrap()
                .contains("winget-manifest.version.1.10.0.schema.json")
        );
        assert!(
            manifest_dir
                .join("ScryerMedia.Scryer.locale.en-US.yaml")
                .is_file()
        );
    }

    #[test]
    fn winget_version_and_repository_validation_are_strict() {
        assert_eq!(
            normalize_winget_version("scryer-v0.16.5").unwrap(),
            Version::parse("0.16.5").unwrap()
        );
        assert_eq!(
            normalize_github_repository("/scryer-media/scryer/").unwrap(),
            "scryer-media/scryer"
        );
        assert!(normalize_github_repository("https://github.com/scryer-media/scryer").is_err());
        assert!(validate_winget_release_date("2026/06/24").is_err());
    }

    #[test]
    fn release_args_signature_uses_bump_mode_when_version_not_explicit() {
        assert_eq!(
            release_args_signature(None, VersionBump::Minor, false),
            "bump:minor"
        );
    }

    #[test]
    fn release_args_signature_uses_explicit_version_when_present() {
        let version = Version::parse("1.2.3").unwrap();
        assert_eq!(
            release_args_signature(Some(&version), VersionBump::Patch, false),
            "version:1.2.3"
        );
    }

    #[test]
    fn release_args_signature_includes_graphql_dangerous_override() {
        assert_eq!(
            release_args_signature(None, VersionBump::Patch, true),
            "bump:patch;allow-graphql-dangerous"
        );
    }

    #[test]
    fn release_dry_run_cache_round_trips_through_json() {
        let cache = sample_release_dry_run_cache();
        let json = serde_json::to_string(&cache).unwrap();
        let decoded: ReleaseDryRunCache = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, cache);
    }

    #[test]
    fn canonical_pretty_json_sorts_nested_object_keys() {
        let value = serde_json::json!({
            "version": "1.0.0",
            "provider": {
                "supported_ids": {
                    "series": ["tvdb_id"],
                    "anime": ["tvdb_id"],
                    "movie": ["imdb_id"]
                },
                "rss": true
            },
            "id": "plugin.example"
        });

        assert_eq!(
            canonical_pretty_json(&value).unwrap(),
            concat!(
                "{\n",
                "  \"id\": \"plugin.example\",\n",
                "  \"provider\": {\n",
                "    \"rss\": true,\n",
                "    \"supported_ids\": {\n",
                "      \"anime\": [\n",
                "        \"tvdb_id\"\n",
                "      ],\n",
                "      \"movie\": [\n",
                "        \"imdb_id\"\n",
                "      ],\n",
                "      \"series\": [\n",
                "        \"tvdb_id\"\n",
                "      ]\n",
                "    }\n",
                "  },\n",
                "  \"version\": \"1.0.0\"\n",
                "}\n"
            )
        );
    }

    #[test]
    fn normalize_bundle_cert_wraps_base64_der_as_pem() {
        let der_base64 =
            base64::engine::general_purpose::STANDARD.encode([0x30, 0x03, 0x02, 0x01, 0x05]);
        let pem = normalize_bundle_cert(&der_base64).expect("DER certificate should normalize");
        assert!(pem.starts_with("-----BEGIN CERTIFICATE-----\n"));
        assert!(pem.contains(&der_base64));
        assert!(pem.ends_with("-----END CERTIFICATE-----\n"));
    }

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

    #[test]
    fn release_dry_run_cache_rejects_unsuccessful_prior_run() {
        let mut cache = sample_release_dry_run_cache();
        cache.success = false;
        let reason = release_dry_run_cache_rejection_reason(
            &cache,
            &sample_release_dry_run_expectations(),
            true,
        );
        assert_eq!(
            reason.as_deref(),
            Some("previous dry run did not complete successfully")
        );
    }

    #[test]
    fn release_dry_run_cache_allows_dirty_start_when_other_inputs_match() {
        let mut cache = sample_release_dry_run_cache();
        cache.worktree_clean_at_start = false;
        let reason = release_dry_run_cache_rejection_reason(
            &cache,
            &sample_release_dry_run_expectations(),
            true,
        );
        assert!(reason.is_none());
    }

    #[test]
    fn release_dry_run_cache_rejects_commit_mismatch() {
        let mut cache = sample_release_dry_run_cache();
        cache.git_commit = "def456".to_string();
        let reason = release_dry_run_cache_rejection_reason(
            &cache,
            &sample_release_dry_run_expectations(),
            true,
        );
        assert_eq!(reason.as_deref(), Some("HEAD commit changed since dry run"));
    }

    #[test]
    fn release_dry_run_cache_rejects_args_mismatch() {
        let mut cache = sample_release_dry_run_cache();
        cache.release_args = "bump:minor".to_string();
        let reason = release_dry_run_cache_rejection_reason(
            &cache,
            &sample_release_dry_run_expectations(),
            true,
        );
        assert_eq!(
            reason.as_deref(),
            Some("release arguments changed since dry run")
        );
    }

    #[test]
    fn release_dry_run_cache_rejects_latest_tag_mismatch() {
        let mut cache = sample_release_dry_run_cache();
        cache.latest_tag_seen = Some("scryer-v0.13.0".to_string());
        let reason = release_dry_run_cache_rejection_reason(
            &cache,
            &sample_release_dry_run_expectations(),
            true,
        );
        assert_eq!(
            reason.as_deref(),
            Some("latest release tag changed since dry run")
        );
    }

    #[test]
    fn release_dry_run_cache_rejects_next_tag_mismatch() {
        let mut cache = sample_release_dry_run_cache();
        cache.tag_name = "scryer-v0.13.3".to_string();
        let reason = release_dry_run_cache_rejection_reason(
            &cache,
            &sample_release_dry_run_expectations(),
            true,
        );
        assert_eq!(
            reason.as_deref(),
            Some("computed release tag changed since dry run")
        );
    }

    #[test]
    fn release_dry_run_cache_rejects_release_notes_path_mismatch() {
        let mut cache = sample_release_dry_run_cache();
        cache.release_notes_path = Some("release-notes/scryer-v0.13.3.md".to_string());
        let reason = release_dry_run_cache_rejection_reason(
            &cache,
            &sample_release_dry_run_expectations(),
            true,
        );
        assert_eq!(
            reason.as_deref(),
            Some("release notes path changed since dry run")
        );
    }

    #[test]
    fn release_dry_run_cache_rejects_release_notes_hash_mismatch() {
        let mut cache = sample_release_dry_run_cache();
        cache.release_notes_sha256 = Some("sha256:changed".to_string());
        let reason = release_dry_run_cache_rejection_reason(
            &cache,
            &sample_release_dry_run_expectations(),
            true,
        );
        assert_eq!(
            reason.as_deref(),
            Some("release notes changed since dry run")
        );
    }

    #[test]
    fn release_dry_run_cache_rejects_missing_graphql_api_compat_step() {
        let mut cache = sample_release_dry_run_cache();
        cache
            .validated_steps
            .retain(|step| step != GRAPHQL_API_COMPAT_STEP);
        let reason = release_dry_run_cache_rejection_reason(
            &cache,
            &sample_release_dry_run_expectations(),
            true,
        );
        assert_eq!(
            reason.as_deref(),
            Some(
                "dry run did not record required release-blocking validations: graphql_api_compat"
            )
        );
    }

    #[test]
    fn release_dry_run_cache_accepts_ci_docs_only_required_steps() {
        let mut cache = sample_release_dry_run_cache();
        cache.validated_steps = ReleaseValidationScope::MetaOnly
            .required_steps()
            .iter()
            .map(|step| (*step).to_string())
            .collect();
        let mut expectations = sample_release_dry_run_expectations();
        expectations.validation_scope = ReleaseValidationScope::MetaOnly;
        assert!(release_dry_run_cache_rejection_reason(&cache, &expectations, true).is_none());
    }

    #[test]
    fn graphql_api_compat_allows_missing_previous_schema_for_baseline_bootstrap() {
        let version = Version::parse(GRAPHQL_API_BASELINE_VERSION).unwrap();

        assert!(allow_missing_previous_graphql_schema(&version));
    }

    #[test]
    fn graphql_api_compat_rejects_missing_previous_schema_after_baseline() {
        let version = Version::parse("0.16.4").unwrap();

        assert!(!allow_missing_previous_graphql_schema(&version));
    }

    #[test]
    fn release_dry_run_cache_rejects_missing_cached_builtins() {
        let cache = sample_release_dry_run_cache();
        let reason = release_dry_run_cache_rejection_reason(
            &cache,
            &sample_release_dry_run_expectations(),
            false,
        );
        assert_eq!(
            reason.as_deref(),
            Some("cached builtin artifacts are missing or BLAKE3-mismatched")
        );
    }

    #[test]
    fn release_dry_run_cache_accepts_matching_inputs() {
        let cache = sample_release_dry_run_cache();
        let reason = release_dry_run_cache_rejection_reason(
            &cache,
            &sample_release_dry_run_expectations(),
            true,
        );
        assert!(reason.is_none());
    }

    fn write_test_builtin_cache(ctx: &TaskContext, cache_dir: &Path) -> BTreeMap<String, String> {
        let mut digests = BTreeMap::new();
        for spec in BUILTIN_PLUGINS {
            let paths = builtin_asset_paths(ctx, spec);
            let wasm_bytes = format!("{} wasm", spec.plugin_id).into_bytes();
            let compressed = zstd::encode_all(wasm_bytes.as_slice(), 0).unwrap();
            let wasm_file = paths.wasm.file_name().unwrap();
            fs::write(cache_dir.join(wasm_file), compressed).unwrap();
            digests.insert(spec.plugin_id.to_string(), blake3_hex(&wasm_bytes));

            for sidecar in [paths.descriptor_json, paths.description] {
                let file_name = sidecar.file_name().unwrap();
                fs::write(cache_dir.join(file_name), b"sidecar").unwrap();
            }
        }
        digests
    }

    #[test]
    fn builtin_cache_matches_catalog_wasm_blake3_from_decompressed_wasm() {
        let ctx = TaskContext::new();
        let temp = tempfile::tempdir().unwrap();
        let digests = write_test_builtin_cache(&ctx, temp.path());

        assert!(builtin_cache_matches_catalog_wasm_blake3(
            &ctx,
            temp.path(),
            &digests
        ));
    }

    #[test]
    fn builtin_cache_rejects_catalog_wasm_blake3_mismatch() {
        let ctx = TaskContext::new();
        let temp = tempfile::tempdir().unwrap();
        let mut digests = write_test_builtin_cache(&ctx, temp.path());
        digests.insert("newznab".to_string(), "blake3:wrong".to_string());

        assert!(!builtin_cache_matches_catalog_wasm_blake3(
            &ctx,
            temp.path(),
            &digests
        ));
    }

    fn test_materialized_trust_root(ctx: &TaskContext) -> Vec<u8> {
        fs::read(
            ctx.path(BUILTIN_ASSET_DIR)
                .join(SIGSTORE_TRUST_ROOT_FILENAME),
        )
        .expect("read materialized Sigstore trust root")
    }

    fn test_trust_root_receipt(root_bytes: &[u8]) -> Vec<u8> {
        serde_json::to_vec(&SigstoreTrustRootProvenance {
            schema_version: 1,
            source: SIGSTORE_TRUST_ROOT_SOURCE.to_string(),
            target: SIGSTORE_TRUST_ROOT_TARGET.to_string(),
            sha256: sha256_hex(root_bytes),
            retrieved_at: Utc::now().to_rfc3339(),
            sigstore_version: "sigstore-rs test".to_string(),
            source_commit: "0".repeat(40),
            github_repository: None,
            github_workflow_ref: None,
            github_run_id: None,
        })
        .expect("serialize trust-root receipt")
    }

    #[test]
    fn materialized_trust_root_matches_runtime_requirements() {
        let ctx = TaskContext::new();
        validate_runtime_sigstore_trust_root_document(&test_materialized_trust_root(&ctx))
            .expect("materialized root should satisfy runtime requirements");
    }

    #[test]
    fn materialized_trust_root_rejects_missing_ct_and_timestamp_material() {
        let ctx = TaskContext::new();
        let root = test_materialized_trust_root(&ctx);
        for field in ["ctlogs", "timestampAuthorities"] {
            let mut document: serde_json::Value = serde_json::from_slice(&root).unwrap();
            document[field] = serde_json::Value::Array(Vec::new());
            let error = validate_runtime_sigstore_trust_root_document(
                &serde_json::to_vec(&document).unwrap(),
            )
            .unwrap_err();
            assert!(
                error.to_string().contains("trust root is empty"),
                "unexpected error for {field}: {error:#}"
            );
        }
    }

    #[test]
    fn cached_trust_root_rejects_receipt_digest_mismatch() {
        let ctx = TaskContext::new();
        let root = test_materialized_trust_root(&ctx);
        let valid_receipt = test_trust_root_receipt(&root);
        validate_sigstore_trust_root_receipt(&root, &valid_receipt)
            .expect("matching trust-root receipt should validate");
        let mut receipt: serde_json::Value = serde_json::from_slice(&valid_receipt).unwrap();
        receipt["sha256"] = serde_json::Value::String("0".repeat(64));

        let error =
            validate_sigstore_trust_root_receipt(&root, &serde_json::to_vec(&receipt).unwrap())
                .unwrap_err();
        assert!(
            error.to_string().contains("provenance digest mismatch"),
            "unexpected error: {error:#}"
        );
    }

    #[test]
    fn sigstore_provenance_uses_the_resolved_lockfile_version() {
        let ctx = TaskContext::new();
        let version = resolved_cargo_package_version(&ctx, "sigstore")
            .expect("resolve sigstore from Cargo.lock");
        Version::parse(&version).expect("resolved sigstore version should be semver");
    }

    #[test]
    fn release_hygiene_flags_local_absolute_paths() {
        let violations = scan_release_hygiene_content(
            Path::new("crates/scryer-plugin-sdk/src/lib.rs"),
            "const SDK_ROOT: &str = \"/Users/example/dev/scryer-media/scryer\";",
        );

        assert_eq!(
            violations,
            vec![
                "crates/scryer-plugin-sdk/src/lib.rs:1: local absolute path reference: const SDK_ROOT: &str = \"/Users/example/dev/scryer-media/scryer\";"
                    .to_string()
            ]
        );
    }

    #[test]
    fn release_hygiene_flags_home_relative_paths() {
        let violations = scan_release_hygiene_content(
            Path::new("crates/scryer-application/src/lib.rs"),
            "const DESIGN: &str = \"~/private/design.md\";",
        );

        assert_eq!(
            violations,
            vec![
                "crates/scryer-application/src/lib.rs:1: local absolute path reference: const DESIGN: &str = \"~/private/design.md\";"
                    .to_string()
            ]
        );
    }

    #[test]
    fn release_hygiene_allows_linuxbrew_installation_paths() {
        let violations = scan_release_hygiene_content(
            Path::new("crates/scryer-application/src/application_upgrade/installation.rs"),
            "const BINARY: &str = \"/home/linuxbrew/.linuxbrew/bin/scryer\";",
        );

        assert!(violations.is_empty());
    }

    #[test]
    fn release_hygiene_flags_linux_user_home_paths() {
        let violations = scan_release_hygiene_content(
            Path::new("crates/scryer-application/src/lib.rs"),
            "const DESIGN: &str = \"/home/developer/private/design.md\";",
        );

        assert_eq!(
            violations,
            vec![
                "crates/scryer-application/src/lib.rs:1: local absolute path reference: const DESIGN: &str = \"/home/developer/private/design.md\";"
                    .to_string()
            ]
        );
    }

    #[test]
    fn release_hygiene_allows_users_api_routes() {
        let violations = scan_release_hygiene_content(
            Path::new("crates/scryer-infrastructure-identity/src/external_identity.rs"),
            r#"
                .and(path("/Users/AuthenticateByName"))
                let avatar = format!("{}/Users/jf-user/Images/Primary?tag=tag", server.uri());
            "#,
        );

        assert!(violations.is_empty());
    }

    #[test]
    fn release_hygiene_flags_sibling_e2e_paths() {
        let violations = scan_release_hygiene_content(
            Path::new("crates/scryer-application/src/lib.rs"),
            "let fixture = manifest_dir.join(\"../e2e/testdata\").join(name);",
        );

        assert_eq!(
            violations,
            vec![
                "crates/scryer-application/src/lib.rs:1: sibling e2e repo reference: let fixture = manifest_dir.join(\"../e2e/testdata\").join(name);"
                    .to_string()
            ]
        );
    }

    #[test]
    fn release_hygiene_allows_repo_local_paths() {
        let violations = scan_release_hygiene_content(
            Path::new("crates/scryer-application/src/lib.rs"),
            "let fixture = manifest_dir.join(\"tests/fixtures\").join(name);",
        );

        assert!(violations.is_empty());
    }

    #[test]
    fn release_hygiene_allows_workflow_runner_paths() {
        let violations = scan_release_hygiene_content(
            Path::new(".github/workflows/scryer.yml"),
            "      SCCACHE_DIR: /home/runner/.cache/sccache",
        );

        assert!(violations.is_empty());
    }

    #[test]
    fn release_hygiene_allows_release_tooling_fixture_strings() {
        let violations = scan_release_hygiene_content(
            Path::new("xtask-release/src/main.rs"),
            "const SDK_ROOT: &str = \"/Users/example/dev/scryer-media/scryer\";\nlet fixture = manifest_dir.join(\"../e2e/testdata\").join(name);",
        );

        assert!(violations.is_empty());
    }

    #[test]
    fn materialized_builtin_directory_replaces_the_previous_set_atomically() {
        let tempdir = tempfile::tempdir().expect("create tempdir");
        let output_dir = tempdir.path().join("builtins");
        fs::create_dir_all(&output_dir).expect("create previous output");
        fs::write(output_dir.join("old"), b"old").expect("write previous output");

        let staging_parent = tempdir.path().join("staging");
        let staging_dir = staging_parent.join("builtins");
        fs::create_dir_all(&staging_dir).expect("create staged output");
        fs::write(staging_dir.join("new"), b"new").expect("write staged output");

        publish_materialized_builtin_dir(&staging_dir, &output_dir)
            .expect("publish staged builtins");

        assert_eq!(fs::read(output_dir.join("new")).unwrap(), b"new");
        assert!(!output_dir.join("old").exists());
        assert_eq!(
            fs::read(staging_parent.join("previous/old")).unwrap(),
            b"old"
        );
    }
}

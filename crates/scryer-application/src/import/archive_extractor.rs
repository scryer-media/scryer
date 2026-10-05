//! Archive extraction for the import pipeline.
//!
//! Detects RAR, 7z, ZIP, XZ and PAR2 sets in download directories. Extraction is
//! delegated to the optional archive extraction plugin, which also owns PAR2
//! verification, placement and repair: the plugin scans the read-only source
//! directory it is given for `.par2` sets and repairs internally, emitting the
//! result into its writable output directory. The host neither orchestrates
//! nor observes that step.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crate::import::archive_passwords::{ArchivePasswordCandidates, redact_archive_diagnostic};
use crate::{AppError, AppResult, ArchiveExtractorPluginProvider};
use scryer_plugin_sdk::{
    ArchivePluginFormat, ArchivePluginOperation, ArchivePluginProcessRequest,
    ArchivePluginProcessResponse, ArchivePluginStatus,
};
use tracing::info;

const EXTRACTED_DIR_NAME: &str = "_scryer_extracted";
const ARCHIVE_STAGING_PREFIX: &str = ".scryer-ax-";
#[cfg(test)]
const ARCHIVE_WRITE_PROBE_PREFIX: &str = ".scryer-write-probe-";
const LEGACY_ARCHIVE_STAGING_PREFIX: &str = ".scryer-archive-extract-";
const ARCHIVE_STAGING_OUTPUT_DIR: &str = "out";
const ARCHIVE_WORKSPACE_OWNER_FILE: &str = ".workspace-owner.json";
const ARCHIVE_WORKSPACE_RELEASED_FILE: &str = ".workspace-released";
const ARCHIVE_STAGING_CREATE_ATTEMPTS: usize = 16;
const STALE_ARCHIVE_STAGING_AFTER: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_PLUGIN_OUTPUT_FILES: usize = 20_000;
const MAX_PLUGIN_OUTPUT_DIRECTORIES: usize = 20_000;
const MAX_PLUGIN_OUTPUT_ENTRIES: usize = MAX_PLUGIN_OUTPUT_FILES + MAX_PLUGIN_OUTPUT_DIRECTORIES;
const MAX_PLUGIN_OUTPUT_BYTES: u64 = 2 * 1024 * 1024 * 1024 * 1024;
const MAX_ARCHIVE_DISCOVERY_DEPTH: usize = 16;
const MAX_ARCHIVE_DISCOVERY_DIRECTORIES: usize = MAX_PLUGIN_OUTPUT_DIRECTORIES;
/// Archive sets one download may hold. More than this is refused rather than
/// extracted in part.
const MAX_ARCHIVE_SETS: usize = 256;
/// Levels of archives inside extracted archives that are extracted in turn.
const MAX_NESTED_ARCHIVE_DEPTH: usize = 2;
/// Archives found inside extracted output, across every level.
const MAX_NESTED_ARCHIVES: usize = 16;
const NESTED_OUTPUT_DIR_PREFIX: &str = "nested-";

/// Output counted across every archive set and nested level of one workspace,
/// so the output caps bound the whole extraction rather than each set.
#[derive(Debug, Clone, Copy, Default)]
struct PluginOutputTotals {
    entries: usize,
    directories: usize,
    files: usize,
    bytes: u64,
    execution_time: Duration,
    execution_budget: Duration,
}

#[derive(Debug, Clone)]
pub struct ArchiveExtractionDestination {
    staging_parent: PathBuf,
    stale_cleanup_parents: Vec<PathBuf>,
    import_id: String,
}

impl ArchiveExtractionDestination {
    pub fn new(staging_parent: impl Into<PathBuf>, import_id: impl Into<String>) -> Self {
        Self {
            staging_parent: staging_parent.into(),
            stale_cleanup_parents: Vec::new(),
            import_id: import_id.into(),
        }
    }

    pub fn with_stale_cleanup_parent(mut self, parent: impl Into<PathBuf>) -> Self {
        self.stale_cleanup_parents.push(parent.into());
        self
    }

    pub fn staging_parent(&self) -> &Path {
        &self.staging_parent
    }
}

#[derive(Debug)]
struct ArchiveExtractionWorkspace {
    root: PathBuf,
    output_dir: PathBuf,
    lease: ArchiveWorkspaceLease,
}

#[derive(Debug)]
struct ArchiveWorkspaceLease {
    root: PathBuf,
    published: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ArchiveWorkspaceOwner {
    schema: u32,
    workspace: String,
    import_id: String,
    #[serde(default)]
    proof: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ArchiveWorkspaceRelease {
    schema: u32,
    owner_proof: String,
    released_at: u64,
    proof: String,
}

static ARCHIVE_WORKSPACE_KEY: std::sync::OnceLock<[u8; 32]> = std::sync::OnceLock::new();

/// Initialize ownership authority from the service's private state directory.
/// Media and download directories never supply ownership credentials.
pub fn initialize_archive_workspace_ownership(state_dir: &Path) -> AppResult<()> {
    ARCHIVE_WORKSPACE_KEY.get_or_init(|| archive_workspace_key_for_startup(state_dir));
    Ok(())
}

fn archive_workspace_key_for_startup(state_dir: &Path) -> [u8; 32] {
    load_or_create_archive_workspace_key(state_dir).unwrap_or_else(|error| {
        tracing::warn!(kind = ?error.kind(),
            "archive workspace authority is unavailable; using process-local ownership and preserving older workspaces");
        new_archive_workspace_key()
    })
}

fn new_archive_workspace_key() -> [u8; 32] {
    let mut key = [0; 32];
    key[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    key[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    key
}

fn load_or_create_archive_workspace_key(state_dir: &Path) -> std::io::Result<[u8; 32]> {
    use std::io::{Read, Write};
    std::fs::create_dir_all(state_dir)?;
    let path = state_dir.join("archive-workspace-key");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    match options.open(&path) {
        Ok(mut file) => {
            let key = new_archive_workspace_key();
            file.write_all(&key)?;
            file.sync_all()?;
            Ok(key)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let metadata = std::fs::symlink_metadata(&path)?;
            if !metadata.file_type().is_file() || metadata.len() != 32 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "workspace authority key is not a regular 32-byte file",
                ));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if metadata.permissions().mode() & 0o077 != 0 {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "workspace authority key must be private to the service",
                    ));
                }
            }
            let mut options = std::fs::OpenOptions::new();
            options.read(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.custom_flags(libc::O_NOFOLLOW);
            }
            let file = options.open(&path)?;
            if !file.metadata()?.is_file() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "workspace authority key is not a regular file",
                ));
            }
            let mut bytes = Vec::new();
            file.take(33).read_to_end(&mut bytes)?;
            bytes.try_into().map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "workspace authority key has an invalid length",
                )
            })
        }
        Err(error) => Err(error),
    }
}

fn archive_workspace_key() -> Option<&'static [u8; 32]> {
    #[cfg(test)]
    return Some(ARCHIVE_WORKSPACE_KEY.get_or_init(new_archive_workspace_key));
    #[cfg(not(test))]
    ARCHIVE_WORKSPACE_KEY.get()
}

/// Directory identity survives a same-filesystem move but cannot be supplied
/// by a downloaded marker or copied to another directory.
fn archive_workspace_identity(root: &Path) -> Option<Vec<u8>> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        };
        options.custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT);
    }
    archive_workspace_identity_from_file(&options.open(root).ok()?)
}

fn archive_workspace_identity_from_file(directory: &std::fs::File) -> Option<Vec<u8>> {
    let metadata = directory.metadata().ok()?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return None;
    }
    let mut identity = Vec::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        identity.extend_from_slice(&metadata.dev().to_le_bytes());
        identity.extend_from_slice(&metadata.ino().to_le_bytes());
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
        };
        let mut information: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        if unsafe { GetFileInformationByHandle(directory.as_raw_handle().cast(), &mut information) }
            == 0
        {
            return None;
        }
        identity.extend_from_slice(&information.dwVolumeSerialNumber.to_le_bytes());
        identity.extend_from_slice(&information.nFileIndexHigh.to_le_bytes());
        identity.extend_from_slice(&information.nFileIndexLow.to_le_bytes());
    }
    #[cfg(not(any(unix, windows)))]
    return None;
    // Include birth time when exposed by the filesystem to resist identifier
    // reuse after a directory has been removed.
    if let Ok(created) = metadata.created()
        && let Ok(created) = created.duration_since(SystemTime::UNIX_EPOCH)
    {
        identity.extend_from_slice(&created.as_nanos().to_le_bytes());
    }
    Some(identity)
}

fn archive_owner_proof(root: &Path, owner: &ArchiveWorkspaceOwner) -> Option<blake3::Hash> {
    archive_owner_proof_for_identity(&archive_workspace_identity(root)?, owner)
}

fn archive_owner_proof_for_identity(
    identity: &[u8],
    owner: &ArchiveWorkspaceOwner,
) -> Option<blake3::Hash> {
    let bytes = serde_json::to_vec(&(
        "archive-owner-v2",
        owner.schema,
        &owner.workspace,
        &owner.import_id,
        identity,
    ))
    .ok()?;
    Some(blake3::keyed_hash(archive_workspace_key()?, &bytes))
}

fn archive_release_proof(root: &Path, release: &ArchiveWorkspaceRelease) -> Option<blake3::Hash> {
    let bytes = serde_json::to_vec(&(
        "archive-release-v2",
        release.schema,
        &release.owner_proof,
        release.released_at,
        archive_workspace_identity(root)?,
    ))
    .ok()?;
    Some(blake3::keyed_hash(archive_workspace_key()?, &bytes))
}

fn read_archive_marker<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.file_type().is_file() || metadata.len() > 4096 {
        return None;
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path).ok()?;
    let mut bounded = std::io::Read::take(file, 4097);
    serde_json::from_reader(&mut bounded).ok()
}

fn release_archive_workspace(root: &Path) -> std::io::Result<()> {
    use std::io::Write;
    let owner = archive_workspace_owner(root).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "archive workspace ownership is unverified",
        )
    })?;
    let marker = root.join(ARCHIVE_WORKSPACE_RELEASED_FILE);
    if archive_workspace_release(root).is_some() {
        return Ok(());
    }
    let mut release = ArchiveWorkspaceRelease {
        schema: 2,
        owner_proof: owner.proof,
        released_at: SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_err(std::io::Error::other)?
            .as_secs(),
        proof: String::new(),
    };
    release.proof = archive_release_proof(root, &release)
        .ok_or_else(|| std::io::Error::other("archive release authority is unavailable"))?
        .to_hex()
        .to_string();
    let bytes = serde_json::to_vec(&release)?;
    // A preexisting invalid release file is ambiguous and is preserved.
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(marker)?
        .write_all(&bytes)
}

fn archive_workspace_release(root: &Path) -> Option<SystemTime> {
    let owner = archive_workspace_owner(root)?;
    let release: ArchiveWorkspaceRelease =
        read_archive_marker(&root.join(ARCHIVE_WORKSPACE_RELEASED_FILE))?;
    if release.schema != 2
        || release.owner_proof != owner.proof
        || blake3::Hash::from_hex(&release.proof).ok()? != archive_release_proof(root, &release)?
    {
        return None;
    }
    SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(release.released_at))
}

impl Drop for ArchiveWorkspaceLease {
    fn drop(&mut self) {
        // An unpublished workspace was never handed out, so no video from it
        // can have been imported.
        if !self.published
            && archive_workspace_owner(&self.root).is_some()
            && archive_workspace_inventory(&self.root).permits_release(false)
            && let Err(error) = release_archive_workspace(&self.root)
        {
            tracing::warn!(error = %redact_archive_diagnostic(&error.to_string()), "unpublished archive workspace could not be released");
        }
    }
}

/// Ownership is established when the workspace is created, never inferred
/// from its age or a prefix. Unmarked and legacy directories are preserved.
pub fn is_owned_archive_workspace(root: &Path) -> bool {
    archive_workspace_owner(root).is_some()
}

pub(crate) fn is_owned_archive_workspace_handle(root: &Path, directory: &std::fs::File) -> bool {
    let Some(identity) = archive_workspace_identity_from_file(directory) else {
        return false;
    };
    archive_workspace_owner_for_identity(root, &identity).is_some()
}

fn archive_workspace_owner(root: &Path) -> Option<ArchiveWorkspaceOwner> {
    archive_workspace_owner_for_identity(root, &archive_workspace_identity(root)?)
}

fn archive_workspace_owner_for_identity(
    root: &Path,
    identity: &[u8],
) -> Option<ArchiveWorkspaceOwner> {
    let name = root.file_name()?.to_str()?;
    if !is_archive_workspace_name(name) {
        return None;
    }
    let owner: ArchiveWorkspaceOwner =
        read_archive_marker(&root.join(ARCHIVE_WORKSPACE_OWNER_FILE))?;
    (owner.schema == 2
        && owner.workspace == name
        && !owner.import_id.is_empty()
        && blake3::Hash::from_hex(&owner.proof).ok()?
            == archive_owner_proof_for_identity(identity, &owner)?)
    .then_some(owner)
}

struct ArchivePluginExtraction {
    source_dir: PathBuf,
    archive_path: PathBuf,
    archive_type: ArchiveType,
    format: ArchivePluginFormat,
    password: Option<String>,
    client: Arc<dyn crate::ArchiveExtractorClient>,
    output_dir: PathBuf,
}

/// Archive type detected in a download directory.
#[derive(Debug, Clone, Copy)]
pub enum ArchiveType {
    Rar,
    SevenZip,
    Zip,
    Xz,
    Par2,
}

impl ArchiveType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rar => "RAR",
            Self::SevenZip => "7z",
            Self::Zip => "zip",
            Self::Xz => "xz",
            Self::Par2 => "PAR2",
        }
    }
}

/// Extract every archive set to an owned destination-side staging directory.
/// Loose media never suppresses extraction of other members of the release.
///
/// `is_sample` is the sample rule the import scan will apply afterwards: a
/// video it would discard does not make the download's archives redundant.
///
/// Nested archives are opened even beside video, within shared bounds. Any
/// failed set prevents an incomplete extraction from appearing successful.
///
/// Each set is first attempted without a password. Only when the plugin
/// answers that it needs one (or that the one given is wrong) are the
/// `passwords` candidates tried, in order; any other failure ends the attempts
/// and is returned as it is.
pub async fn extract_archives_if_needed(
    dir: &Path,
    is_sample: fn(&Path) -> bool,
    destination: Option<ArchiveExtractionDestination>,
    passwords: &ArchivePasswordCandidates,
    archive_provider: Option<Arc<dyn ArchiveExtractorPluginProvider>>,
) -> AppResult<Option<PathBuf>> {
    let dir = dir.to_path_buf();
    let sets = {
        let dir = dir.clone();
        tokio::task::spawn_blocking(move || plan_archive_extraction(&dir, is_sample))
            .await
            .map_err(|e| AppError::Repository(format!("archive detection task failed: {e}")))?
            .map_err(classify_archive_failure)?
    };

    if sets.is_empty() {
        return Ok(None);
    }
    let Some(destination) = destination else {
        return Err(AppError::Validation(format!(
            "archive extraction requires a resolved import destination before staging output for {}",
            redact_archive_diagnostic(&dir.to_string_lossy())
        )));
    };
    let mut workspace = ArchiveExtractionWorkspace::create(&destination).await?;

    info!(
        archive = %redact_archive_diagnostic(&sets[0].0.to_string_lossy()),
        archive_type = sets[0].1.as_str(),
        archive_sets = sets.len(),
        workspace = %redact_archive_diagnostic(&workspace.root.to_string_lossy()),
        "extracting archive before import"
    );

    let workspace_root = workspace.root.clone();
    let Some(provider) = archive_provider else {
        cleanup_extracted_dir(&workspace_root).await;
        return Err(AppError::archive_extraction_plugin_required(Some(
            redact_archive_diagnostic(&dir.to_string_lossy()),
        )));
    };

    let extraction =
        extract_into_workspace(&workspace, &dir, sets, is_sample, passwords, &provider).await;

    match extraction {
        Ok(true) => {
            workspace.lease.published = true;
            Ok(Some(workspace_root))
        }
        Ok(false) => {
            info!("archive extracted but no media or subtitle files found in output");
            cleanup_extracted_dir(&workspace_root).await;
            Ok(None)
        }
        Err(error) => {
            abandon_extracted_dir(&workspace_root).await;
            Err(classify_archive_failure(error))
        }
    }
}

fn classify_archive_failure(error: AppError) -> AppError {
    if is_password_required_error(&error) {
        return error;
    }
    match error {
        AppError::Validation(message) | AppError::ArchiveExtractionFailed { message } => {
            AppError::ArchiveExtractionFailed {
                message: redact_archive_diagnostic(&message),
            }
        }
        AppError::Repository(message) => AppError::Repository(redact_archive_diagnostic(&message)),
        error => error,
    }
}

/// Extract every set and its nested archives. A partial result remains a
/// failure even when another set produced valid video.
async fn extract_into_workspace(
    workspace: &ArchiveExtractionWorkspace,
    source: &Path,
    sets: Vec<(PathBuf, ArchiveType)>,
    is_sample: fn(&Path) -> bool,
    passwords: &ArchivePasswordCandidates,
    provider: &Arc<dyn ArchiveExtractorPluginProvider>,
) -> AppResult<bool> {
    let source = if source.is_file() {
        source.parent().unwrap_or(Path::new("."))
    } else {
        source
    };
    let input_bytes = archive_inventory(source, MAX_ARCHIVE_DISCOVERY_DEPTH)?
        .iter()
        .filter_map(|path| std::fs::symlink_metadata(path).ok())
        .filter(|metadata| metadata.is_file())
        .fold(0u64, |sum, metadata| sum.saturating_add(metadata.len()));
    // Give large scene releases proportionally more decoding time. Queue
    // admission is excluded by the host; all candidate attempts share this cap.
    let mut totals = PluginOutputTotals {
        execution_budget: Duration::from_secs(
            (input_bytes / (1024 * 1024)).clamp(60 * 60, 24 * 60 * 60),
        ),
        ..Default::default()
    };
    let mut processed_recovery = std::collections::HashSet::new();
    let mut pass_outputs = Vec::with_capacity(sets.len());
    for (index, (archive_path, archive_type)) in sets.into_iter().enumerate() {
        let output_dir = if index == 0 {
            workspace.output_dir.clone()
        } else {
            workspace
                .root
                .join(format!("{ARCHIVE_STAGING_OUTPUT_DIR}-{}", index + 1))
        };
        let set = ArchiveSetExtraction {
            workspace,
            archive_path: &archive_path,
            archive_type,
            output_dir: &output_dir,
            passwords,
            provider,
        };
        if set
            .extract_or_skip(&mut totals, &mut processed_recovery)
            .await?
        {
            pass_outputs.push(output_dir);
        }
    }

    let mut nested_archives = 0usize;
    for depth in 1..=MAX_NESTED_ARCHIVE_DEPTH + 1 {
        let inner_sets = {
            let outputs = pass_outputs.clone();
            tokio::task::spawn_blocking(move || nested_archive_sets(&outputs))
                .await
                .map_err(|e| {
                    AppError::Repository(format!("archive detection task failed: {e}"))
                })??
        };
        if inner_sets.is_empty() {
            break;
        }
        if depth > MAX_NESTED_ARCHIVE_DEPTH {
            return Err(AppError::Validation(format!(
                "archive holds archives nested more than {MAX_NESTED_ARCHIVE_DEPTH} levels deep"
            )));
        }
        nested_archives += inner_sets.len();
        if nested_archives > MAX_NESTED_ARCHIVES {
            return Err(AppError::Validation(format!(
                "archive holds more than {MAX_NESTED_ARCHIVES} nested archives"
            )));
        }
        info!(
            depth,
            archive_sets = inner_sets.len(),
            "extracting archives found inside extracted output"
        );
        let mut next_outputs = Vec::with_capacity(inner_sets.len());
        for (index, (archive_path, archive_type)) in inner_sets.into_iter().enumerate() {
            let output_dir = workspace
                .root
                .join(format!("{NESTED_OUTPUT_DIR_PREFIX}{depth}-{}", index + 1));
            let set = ArchiveSetExtraction {
                workspace,
                archive_path: &archive_path,
                archive_type,
                output_dir: &output_dir,
                passwords,
                provider,
            };
            if set
                .extract_or_skip(&mut totals, &mut processed_recovery)
                .await?
            {
                next_outputs.push(output_dir);
            }
        }
        pass_outputs = next_outputs;
    }

    Ok(archive_inventory(&workspace.root, 64)?.iter().any(|path| {
        (scryer_domain::is_video_file(path) && !is_sample(path))
            || path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| {
                    matches!(
                        ext.to_ascii_lowercase().as_str(),
                        "srt" | "ass" | "ssa" | "vtt" | "sub" | "idx"
                    )
                })
    }))
}

fn nested_archive_sets(outputs: &[PathBuf]) -> AppResult<Vec<(PathBuf, ArchiveType)>> {
    let mut sets = Vec::new();
    for output in outputs {
        sets.extend(find_archive_sets(output)?);
        if sets.len() > MAX_NESTED_ARCHIVES {
            return Err(AppError::Validation(format!(
                "archive holds more than {MAX_NESTED_ARCHIVES} nested archives"
            )));
        }
    }
    Ok(sets)
}

/// One archive set extracted into its own output directory of the workspace.
struct ArchiveSetExtraction<'a> {
    workspace: &'a ArchiveExtractionWorkspace,
    archive_path: &'a Path,
    archive_type: ArchiveType,
    output_dir: &'a Path,
    passwords: &'a ArchivePasswordCandidates,
    provider: &'a Arc<dyn ArchiveExtractorPluginProvider>,
}

impl ArchiveSetExtraction<'_> {
    /// Stops on a failed set; host errors must not receive a fresh execution
    /// budget for each remaining set.
    async fn extract_or_skip(
        &self,
        totals: &mut PluginOutputTotals,
        processed_recovery: &mut std::collections::HashSet<PathBuf>,
    ) -> AppResult<bool> {
        if matches!(self.archive_type, ArchiveType::Par2)
            && self
                .archive_path
                .canonicalize()
                .is_ok_and(|path| processed_recovery.contains(&path))
        {
            return Ok(false);
        }
        match self.extract_with_candidates(totals).await {
            Ok(paths) => {
                processed_recovery.extend(paths);
                Ok(true)
            }
            Err(error) if is_timeout_error(&error) => Err(error),
            Err(error) => {
                discard_workspace_output_dir(&self.workspace.root, self.output_dir).await;
                tracing::warn!(
                    archive = %redact_archive_diagnostic(&self.archive_path.to_string_lossy()),
                    error = %redact_archive_diagnostic(&error.to_string()),
                    "archive set failed to extract"
                );
                Err(error)
            }
        }
    }

    async fn extract_with_candidates(
        &self,
        totals: &mut PluginOutputTotals,
    ) -> AppResult<Vec<PathBuf>> {
        let selection = self
            .provider
            .select_for_format(archive_plugin_format_for_type(self.archive_type))?
            .ok_or_else(|| AppError::archive_extraction_plugin_required(None))?;
        let mut passwords = self.passwords.clone();
        passwords.extend(selection.passwords);
        let mut rejection = match self.attempt(None, totals, &selection.client).await? {
            ArchiveAttempt::Extracted(paths) => return Ok(paths),
            ArchiveAttempt::PasswordRejected(error) => error,
        };
        for (index, candidate) in passwords.iter().enumerate() {
            match self
                .attempt(Some(candidate.value()), totals, &selection.client)
                .await?
            {
                ArchiveAttempt::Extracted(paths) => {
                    info!(
                        password_source = candidate.source().as_str(),
                        candidate = index + 1,
                        candidates = passwords.len(),
                        "archive password candidate accepted"
                    );
                    return Ok(paths);
                }
                ArchiveAttempt::PasswordRejected(error) => rejection = error,
            }
        }
        if !passwords.is_empty() {
            info!(
                candidates = passwords.len(),
                "no archive password candidate was accepted"
            );
        }
        Err(rejection)
    }

    /// One plugin call into a fresh output directory. Output left by an
    /// attempt the plugin refused for its password is discarded, so a later
    /// attempt starts empty and never counts against the output caps.
    async fn attempt(
        &self,
        password: Option<&str>,
        totals: &mut PluginOutputTotals,
        client: &Arc<dyn crate::ArchiveExtractorClient>,
    ) -> AppResult<ArchiveAttempt> {
        prepare_workspace_output_dir(&self.workspace.root, self.output_dir).await?;

        // The plugin owns PAR2: it is handed the archive's own directory as a
        // read-only source preopen, finds any `.par2` set there itself, and
        // repairs into its writable output directory before (or instead of)
        // extracting.
        let source_dir = self
            .archive_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        ensure_source_outside_output(&source_dir, self.output_dir)?;
        let attempt = extract_with_archive_plugin(
            ArchivePluginExtraction {
                source_dir,
                archive_path: self.archive_path.to_path_buf(),
                archive_type: self.archive_type,
                format: archive_plugin_format_for_type(self.archive_type),
                password: password.map(str::to_string),
                client: Arc::clone(client),
                output_dir: self.output_dir.to_path_buf(),
            },
            totals,
        )
        .await?;
        if matches!(attempt, ArchiveAttempt::PasswordRejected(_)) {
            discard_workspace_output_dir(&self.workspace.root, self.output_dir).await;
        }
        Ok(attempt)
    }
}

/// The outcome of one extraction attempt that did not fail outright. A
/// password rejection is the only outcome that lets the next password
/// candidate be tried.
enum ArchiveAttempt {
    Extracted(Vec<PathBuf>),
    PasswordRejected(AppError),
}

/// Creates an output directory directly inside a staging workspace, unless it
/// already exists and is empty.
async fn prepare_workspace_output_dir(workspace_root: &Path, output_dir: &Path) -> AppResult<()> {
    if output_dir.parent() != Some(workspace_root) {
        return Err(AppError::Validation(format!(
            "archive output directory {} is not inside the staging workspace",
            redact_archive_diagnostic(&output_dir.to_string_lossy())
        )));
    }
    match tokio::fs::create_dir(output_dir).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let mut entries = tokio::fs::read_dir(output_dir).await.map_err(|error| {
                AppError::Repository(format!(
                    "failed to read archive staging output directory {}: {error}",
                    redact_archive_diagnostic(&output_dir.to_string_lossy())
                ))
            })?;
            match entries.next_entry().await {
                Ok(None) => Ok(()),
                _ => Err(AppError::Validation(format!(
                    "archive staging output directory {} is not empty",
                    redact_archive_diagnostic(&output_dir.to_string_lossy())
                ))),
            }
        }
        Err(error) => Err(AppError::Repository(format!(
            "failed to create archive staging output directory {}: {error}",
            redact_archive_diagnostic(&output_dir.to_string_lossy())
        ))),
    }
}

/// Removes an output directory Scryer created directly inside one of its own
/// staging workspaces, and nothing else.
async fn discard_workspace_output_dir(workspace_root: &Path, output_dir: &Path) {
    if archive_workspace_owner(workspace_root).is_some()
        && output_dir.parent() == Some(workspace_root)
        && output_dir
            .file_name()
            .is_some_and(|name| name.to_str().is_some_and(is_workspace_output_dir_name))
    {
        let _ = tokio::fs::remove_dir_all(output_dir).await;
    }
}

fn is_workspace_output_dir_name(name: &str) -> bool {
    name == ARCHIVE_STAGING_OUTPUT_DIR
        || name
            .strip_prefix(ARCHIVE_STAGING_OUTPUT_DIR)
            .and_then(|rest| rest.strip_prefix('-'))
            .is_some_and(|index| !index.is_empty() && index.bytes().all(|b| b.is_ascii_digit()))
        || name
            .strip_prefix(NESTED_OUTPUT_DIR_PREFIX)
            .is_some_and(|rest| {
                !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit() || b == b'-')
            })
}

/// The plugin's source preopen is read-only and its output preopen writable.
/// A source inside (or equal to) the output would let the guest rewrite the
/// archive it is reading.
fn ensure_source_outside_output(source_dir: &Path, output_dir: &Path) -> AppResult<()> {
    let canonical = |path: &Path| {
        path.canonicalize().map_err(|error| {
            AppError::Repository(format!(
                "failed to canonicalize archive path {}: {error}",
                redact_archive_diagnostic(&path.to_string_lossy())
            ))
        })
    };
    if canonical(source_dir)?.starts_with(canonical(output_dir)?) {
        return Err(AppError::Validation(format!(
            "archive source {} lies inside its own extraction output",
            redact_archive_diagnostic(&source_dir.to_string_lossy())
        )));
    }
    Ok(())
}

pub fn archive_extraction_would_be_needed(
    dir: &Path,
    is_sample: fn(&Path) -> bool,
) -> AppResult<bool> {
    Ok(!plan_archive_extraction(dir, is_sample)?.is_empty())
}

/// Check if an extraction error indicates a password-protected archive.
pub fn is_password_required_error(error: &AppError) -> bool {
    matches!(error, AppError::ArchivePasswordRequired { .. })
}

pub fn is_timeout_error(error: &AppError) -> bool {
    matches!(error, AppError::ArchiveExtractionTimedOut { .. })
}

/// The archive sets to extract, each named by its first volume: the download
/// itself when it is an archive file, otherwise every set `find_archive_sets`
/// discovers, including archives beside loose video.
fn plan_archive_extraction(
    dir: &Path,
    _is_sample: fn(&Path) -> bool,
) -> AppResult<Vec<(PathBuf, ArchiveType)>> {
    let metadata = std::fs::symlink_metadata(dir).map_err(|error| {
        AppError::Repository(format!("archive source inspection failed: {error}"))
    })?;
    if metadata.file_type().is_symlink() {
        return Err(AppError::Validation(
            "archive source must not be a symlink".into(),
        ));
    }
    if metadata.is_file() {
        return Ok(archive_type_for_path(dir)
            .map(|archive_type| (dir.to_path_buf(), archive_type))
            .into_iter()
            .collect());
    }

    let sets = find_archive_sets(dir)?;
    if sets.len() > MAX_ARCHIVE_SETS {
        return Err(AppError::Validation(format!(
            "download holds more than {MAX_ARCHIVE_SETS} archive sets"
        )));
    }
    Ok(sets)
}

fn archive_plugin_format_for_type(archive_type: ArchiveType) -> ArchivePluginFormat {
    match archive_type {
        ArchiveType::Rar | ArchiveType::Par2 => ArchivePluginFormat::Rar,
        ArchiveType::SevenZip => ArchivePluginFormat::SevenZip,
        ArchiveType::Zip => ArchivePluginFormat::Zip,
        ArchiveType::Xz => ArchivePluginFormat::Xz,
    }
}

async fn extract_with_archive_plugin(
    request: ArchivePluginExtraction,
    totals: &mut PluginOutputTotals,
) -> AppResult<ArchiveAttempt> {
    let ArchivePluginExtraction {
        source_dir,
        archive_path,
        archive_type,
        format,
        password,
        client,
        output_dir,
    } = request;

    let operation = ArchivePluginOperation::ExtractArchive {
        archive_path: archive_path.to_string_lossy().into_owned(),
        output_dir: output_dir.to_string_lossy().into_owned(),
        format,
        password,
    };
    let request = ArchivePluginProcessRequest { operation };
    let mut limits = scryer_plugin_sdk::ArchiveExtractionLimits {
        max_output_bytes: MAX_PLUGIN_OUTPUT_BYTES.saturating_sub(totals.bytes),
        max_entries: MAX_PLUGIN_OUTPUT_FILES.saturating_sub(totals.files) as u64,
        max_directories: MAX_PLUGIN_OUTPUT_DIRECTORIES.saturating_sub(totals.directories) as u64,
        ..Default::default()
    };
    // Staging output and final placement share the resolved destination volume.
    // Scratch is separately capped on the host's actual scratch volume.
    let scratch_dir = std::env::temp_dir();
    let output_space = crate::filesystem_space_raw(&output_dir)
        .ok()
        .map(|space| usable_archive_space(space.available_bytes));
    let scratch_space = crate::filesystem_space_raw(&scratch_dir)
        .ok()
        .map(|space| usable_archive_space(space.available_bytes));
    if let Some(space) = output_space {
        limits.max_output_bytes = limits.max_output_bytes.min(space);
    }
    if let Some(space) = scratch_space {
        limits.max_scratch_bytes = limits.max_scratch_bytes.min(space);
    }
    if archive_storage_is_shared(&output_dir, &scratch_dir) {
        let source_files = archive_inventory(&source_dir, MAX_ARCHIVE_DISCOVERY_DEPTH)?;
        let copies_sources = source_files.iter().any(|path| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("par2"))
        });
        let staged_bytes = if copies_sources {
            source_files
                .iter()
                .filter_map(|path| std::fs::symlink_metadata(path).ok())
                .fold(0u64, |sum, metadata| sum.saturating_add(metadata.len()))
        } else {
            0
        };
        // Plain streaming decoders do not need another full release copy.
        // Recovery reserves its source bytes once on a shared disk. Split
        // archives are read as streams and do not require a staging copy.
        let scratch_reserve = staged_bytes.saturating_add(64 * 1024 * 1024);
        let available = output_space
            .into_iter()
            .chain(scratch_space)
            .min()
            .unwrap_or(limits.max_output_bytes);
        limits.max_scratch_bytes = limits.max_scratch_bytes.min(scratch_reserve).min(available);
        limits.max_output_bytes = limits
            .max_output_bytes
            .min(available.saturating_sub(limits.max_scratch_bytes));
    }
    if limits.max_output_bytes == 0 || limits.max_scratch_bytes == 0 {
        return Err(AppError::Repository(
            "insufficient disk space for archive output and recovery scratch".into(),
        ));
    }
    let budget = totals
        .execution_budget
        .saturating_sub(totals.execution_time);
    if budget.is_zero() {
        return Err(AppError::archive_extraction_timed_out(
            "archive attempt execution budget exhausted",
        ));
    }
    let (response, elapsed) = client.process_with_budget(request, limits, budget).await?;
    totals.execution_time = totals.execution_time.saturating_add(elapsed);
    let password_rejected = matches!(
        response.status,
        ArchivePluginStatus::PasswordRequired | ArchivePluginStatus::PasswordInvalid
    );
    let (replacements, processed_recovery) = if response.status == ArchivePluginStatus::Ok {
        (
            validate_reported_sources(&source_dir, &response.replaced_source_paths)?,
            validate_reported_sources(&source_dir, &response.processed_recovery_paths)?,
        )
    } else {
        (Vec::new(), Vec::new())
    };
    match handle_archive_plugin_response(archive_type, output_dir.clone(), response, totals) {
        Ok(()) => {
            if !replacements.is_empty() {
                use std::io::Write;
                let mut marker = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(output_dir.parent().unwrap().join(format!(
                        ".source-replacements-{}.json",
                        output_dir.file_name().unwrap().to_string_lossy()
                    )))
                    .map_err(|e| {
                        AppError::Repository(format!("failed to record repaired sources: {e}"))
                    })?;
                let encoded = serde_json::to_vec(&replacements)
                    .map_err(|e| AppError::Repository(e.to_string()))?;
                marker
                    .write_all(&encoded)
                    .map_err(|e| AppError::Repository(e.to_string()))?;
            }
            Ok(ArchiveAttempt::Extracted(processed_recovery))
        }
        Err(error) if password_rejected => Ok(ArchiveAttempt::PasswordRejected(error)),
        Err(error) => Err(error),
    }
}

fn usable_archive_space(available: u64) -> u64 {
    available.saturating_sub((512 * 1024 * 1024).min(available / 20))
}

fn archive_storage_is_shared(output: &Path, scratch: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let (Ok(output), Ok(scratch)) = (std::fs::metadata(output), std::fs::metadata(scratch)) {
            return output.dev() == scratch.dev();
        }
    }
    // Unknown storage identity must not count the same free space twice.
    let _ = (output, scratch);
    true
}

fn validate_reported_sources(source_dir: &Path, paths: &[String]) -> AppResult<Vec<PathBuf>> {
    if paths.len() > MAX_PLUGIN_OUTPUT_ENTRIES {
        return Err(AppError::Validation(
            "archive source metadata exceeds entry limit".into(),
        ));
    }
    let source = source_dir
        .canonicalize()
        .map_err(|error| AppError::Repository(error.to_string()))?;
    paths
        .iter()
        .map(|relative| {
            let path = safe_archive_output_path(&source, relative)?;
            if !std::fs::symlink_metadata(&path)
                .is_ok_and(|metadata| metadata.file_type().is_file())
            {
                return Err(AppError::Validation(
                    "archive source metadata must identify a regular file".into(),
                ));
            }
            let canonical = path
                .canonicalize()
                .map_err(|error| AppError::Repository(error.to_string()))?;
            if !canonical.starts_with(&source) {
                return Err(AppError::Validation(
                    "archive source metadata escapes its source directory".into(),
                ));
            }
            Ok(canonical)
        })
        .collect()
}

pub fn replaced_archive_sources(workspace: &Path) -> AppResult<std::collections::HashSet<PathBuf>> {
    let mut sources = std::collections::HashSet::new();
    if archive_workspace_owner(workspace).is_none() {
        return Ok(sources);
    }
    // Host metadata lives outside plugin-writable output directories. Archive
    // members cannot supply or override this list of replaced source files.
    let entries = std::fs::read_dir(workspace).map_err(|e| AppError::Repository(e.to_string()))?;
    for entry in entries {
        let entry = entry.map_err(|e| AppError::Repository(e.to_string()))?;
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(output_name) = name
            .strip_prefix(".source-replacements-")
            .and_then(|name| name.strip_suffix(".json"))
        else {
            continue;
        };
        if !is_workspace_output_dir_name(output_name)
            || !entry
                .file_type()
                .map_err(|e| AppError::Repository(e.to_string()))?
                .is_file()
        {
            continue;
        }
        let file = std::fs::File::open(path).map_err(|e| AppError::Repository(e.to_string()))?;
        let bounded = std::io::Read::take(file, 8 * 1024 * 1024);
        let replaced: Vec<PathBuf> = serde_json::from_reader(bounded)
            .map_err(|e| AppError::Repository(format!("invalid repaired-source manifest: {e}")))?;
        sources.extend(replaced);
    }
    Ok(sources)
}

impl ArchiveExtractionWorkspace {
    async fn create(destination: &ArchiveExtractionDestination) -> AppResult<Self> {
        tokio::fs::create_dir_all(&destination.staging_parent)
            .await
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to create archive staging parent {}: {error}",
                    redact_archive_diagnostic(&destination.staging_parent.to_string_lossy())
                ))
            })?;
        cleanup_stale_archive_artifacts(&destination.staging_parent).await;
        for parent in &destination.stale_cleanup_parents {
            if parent != &destination.staging_parent {
                cleanup_stale_archive_artifacts(parent).await;
            }
        }

        let destination = destination.clone();
        tokio::task::spawn_blocking(move || Self::create_sync(&destination))
            .await
            .map_err(|error| {
                AppError::Repository(format!("archive workspace creation task failed: {error}"))
            })?
    }

    fn create_sync(destination: &ArchiveExtractionDestination) -> AppResult<Self> {
        if archive_workspace_key().is_none() {
            return Err(AppError::Repository(
                "archive workspace authority is unavailable".into(),
            ));
        }
        std::fs::create_dir_all(&destination.staging_parent).map_err(|error| {
            AppError::Repository(format!("failed to prepare archive staging parent: {error}"))
        })?;
        for _ in 0..ARCHIVE_STAGING_CREATE_ATTEMPTS {
            let root = destination.staging_parent.join(format!(
                "{ARCHIVE_STAGING_PREFIX}{}",
                short_staging_suffix()
            ));
            let mut directory = std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                directory.mode(0o700);
            }
            match directory.create(&root) {
                Ok(()) => {
                    // This guard also runs when an abandoned spawn_blocking
                    // result is dropped after its caller was cancelled.
                    let lease = ArchiveWorkspaceLease {
                        root: root.clone(),
                        published: false,
                    };
                    let mut owner = ArchiveWorkspaceOwner {
                        schema: 2,
                        workspace: root.file_name().unwrap().to_string_lossy().into_owned(),
                        import_id: destination.import_id.clone(),
                        proof: String::new(),
                    };
                    owner.proof = archive_owner_proof(&root, &owner)
                        .ok_or_else(|| {
                            AppError::Repository("archive workspace identity is unavailable".into())
                        })?
                        .to_hex()
                        .to_string();
                    let bytes = serde_json::to_vec(&owner).map_err(|error| {
                        AppError::Repository(format!("failed to encode archive ownership: {error}"))
                    })?;
                    use std::io::Write;
                    std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(root.join(ARCHIVE_WORKSPACE_OWNER_FILE))
                        .and_then(|mut file| file.write_all(&bytes))
                        .map_err(|error| {
                            AppError::Repository(format!(
                                "failed to record archive ownership: {error}"
                            ))
                        })?;
                    let output_dir = root.join(ARCHIVE_STAGING_OUTPUT_DIR);
                    std::fs::create_dir(&output_dir).map_err(|error| {
                        AppError::Repository(format!(
                            "failed to create archive staging output directory {}: {error}",
                            redact_archive_diagnostic(&output_dir.to_string_lossy())
                        ))
                    })?;
                    return Ok(Self {
                        root,
                        output_dir,
                        lease,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(AppError::Repository(format!(
                        "failed to create archive staging directory {}: {error}",
                        redact_archive_diagnostic(&root.to_string_lossy())
                    )));
                }
            }
        }

        Err(AppError::Repository(format!(
            "failed to allocate a unique archive staging directory under {}",
            redact_archive_diagnostic(&destination.staging_parent.to_string_lossy())
        )))
    }
}

#[cfg(test)]
pub(crate) fn create_test_archive_workspace(parent: &Path) -> PathBuf {
    create_test_archive_workspace_owned_by(parent, "test-import")
}

#[cfg(test)]
pub(crate) fn create_test_archive_workspace_owned_by(parent: &Path, owner_id: &str) -> PathBuf {
    let mut workspace = ArchiveExtractionWorkspace::create_sync(
        &ArchiveExtractionDestination::new(parent, owner_id),
    )
    .unwrap();
    workspace.lease.published = true;
    workspace.root
}

fn short_staging_suffix() -> String {
    format!("{:016x}", uuid::Uuid::new_v4().as_u128() as u64)
}

async fn cleanup_stale_archive_artifacts(parent: &Path) {
    cleanup_archive_artifacts_older_than(parent, STALE_ARCHIVE_STAGING_AFTER).await;
}

async fn cleanup_archive_artifacts_older_than(parent: &Path, min_age: Duration) {
    let mut entries = match tokio::fs::read_dir(parent).await {
        Ok(entries) => entries,
        Err(_) => return,
    };
    let now = SystemTime::now();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if archive_workspace_owner(&path).is_none() {
            continue;
        }
        // An active or persisted manual-import reference never expires merely
        // because its directory has not changed. Only explicit release makes
        // an owned workspace eligible for abandoned-cleanup recovery.
        let Some(released_at) = archive_workspace_release(&path) else {
            continue;
        };
        if now
            .duration_since(released_at)
            .is_ok_and(|age| age >= min_age)
        {
            let _ = tokio::fs::remove_dir_all(path).await;
        }
    }
}

fn is_old_rar_volume_extension(ext: &str) -> bool {
    let mut chars = ext.chars();
    matches!(chars.next(), Some('r'..='z')) && ext.len() >= 3 && chars.all(|ch| ch.is_ascii_digit())
}

/// Checks one plugin answer. Output left by a successful call that fails
/// validation, or by a failed call, is removed from this set's own output
/// directory; the caller removes the whole workspace on any error.
fn handle_archive_plugin_response(
    archive_type: ArchiveType,
    output_dir: PathBuf,
    response: ArchivePluginProcessResponse,
    totals: &mut PluginOutputTotals,
) -> AppResult<()> {
    match response.status {
        ArchivePluginStatus::Ok => {
            match validate_archive_plugin_output(&output_dir, &response, *totals) {
                Ok(updated) => {
                    *totals = updated;
                    info!(
                        archive_type = archive_type.as_str(),
                        output = %redact_archive_diagnostic(&output_dir.to_string_lossy()),
                        "archive set extracted"
                    );
                    Ok(())
                }
                Err(error) => {
                    let _ = std::fs::remove_dir_all(&output_dir);
                    Err(error)
                }
            }
        }
        ArchivePluginStatus::UnsupportedFormat => Err(AppError::Validation(format!(
            "archive plugin does not support {} extraction",
            archive_type.as_str()
        ))),
        ArchivePluginStatus::PasswordRequired => Err(AppError::ArchivePasswordRequired {
            message: format!("{} archive requires a password", archive_type.as_str()),
        }),
        ArchivePluginStatus::PasswordInvalid => Err(AppError::ArchivePasswordRequired {
            message: format!("{} archive password is invalid", archive_type.as_str()),
        }),
        ArchivePluginStatus::Failed => {
            let _ = std::fs::remove_dir_all(&output_dir);
            Err(archive_plugin_failure_error(
                response.error_code.as_deref(),
                response.message.as_deref(),
            ))
        }
    }
}

/// Maps a `Failed` archive plugin response onto an `AppError`.
///
/// PAR2 verification and repair now run inside the plugin, so what used to be a
/// native validation error arrives as `Failed` plus a machine-readable
/// `error_code`. Both halves are surfaced (`error_code` used to be dropped
/// whenever a human message was also present), and the one code that maps onto
/// a pre-existing native classification keeps it: a recovery set that cannot
/// reconstruct the payload is a permanent condition in the downloaded data, not
/// a transient host fault.
fn archive_plugin_failure_error(error_code: Option<&str>, message: Option<&str>) -> AppError {
    if error_code == Some("password_or_corruption") {
        // Failed responses remain terminal for this attempt. This classification
        // enables an operator password prompt without advancing the candidate loop.
        return AppError::ArchivePasswordRequired {
            message: "password_or_corruption: the password may be incorrect or the encrypted archive may be damaged; automatic password attempts stopped".into(),
        };
    }
    let text = match (error_code, message) {
        (Some(code), Some(message)) => format!("{code}: {message}"),
        (Some(code), None) => code.to_string(),
        (None, Some(message)) => message.to_string(),
        (None, None) => "archive plugin extraction failed".to_string(),
    };
    let text = redact_archive_diagnostic(&text);

    if error_code.is_some_and(|code| {
        matches!(
            code,
            "par2_insufficient_recovery"
                | "par2_ambiguous_set"
                | "par2_ambiguous_archive"
                | "resource_limit"
                | "invalid_limits"
                | "too_many_entries"
                | "expanded_too_large"
                | "compressed_too_large"
                | "unsafe_input"
                | "unsafe_path"
                | "symlink_entry"
                | "duplicate_path"
                | "duplicate_volume"
                | "missing_volume"
                | "corrupt_archive"
                | "archive_corrupt"
                | "unsupported_method"
                | "unsupported_7z_method"
                | "xz_memory_limit"
                | "duplicate_output_path"
                | "par2_too_large"
                | "par2_duplicate_path"
        )
    }) {
        AppError::ArchiveExtractionFailed { message: text }
    } else {
        AppError::Repository(text)
    }
}

/// Validates one output directory and returns `totals` with its contents added.
/// The caps apply to the running totals, so they bound the whole workspace.
fn validate_archive_plugin_output(
    output_dir: &Path,
    response: &ArchivePluginProcessResponse,
    totals: PluginOutputTotals,
) -> AppResult<PluginOutputTotals> {
    let output_root = output_dir.canonicalize().map_err(|error| {
        AppError::Repository(format!(
            "failed to canonicalize archive plugin output directory {}: {error}",
            redact_archive_diagnostic(&output_dir.to_string_lossy())
        ))
    })?;

    for file in &response.files {
        let path = safe_archive_output_path(output_dir, &file.relative_path)?;
        let metadata = std::fs::symlink_metadata(&path).map_err(|error| {
            AppError::Repository(format!(
                "archive plugin manifest output '{}' is missing or unreadable: {error}",
                redact_archive_diagnostic(&path.to_string_lossy())
            ))
        })?;
        if !metadata.file_type().is_file() {
            return Err(AppError::Validation(format!(
                "archive plugin manifest output is not a regular file: {}",
                redact_archive_diagnostic(&path.to_string_lossy())
            )));
        }
        ensure_path_under_output_with_root(&path, &output_root)?;
        if let Some(expected_size) = file.size
            && expected_size != metadata.len()
        {
            return Err(AppError::Validation(format!(
                "archive plugin manifest size mismatch for {}",
                redact_archive_diagnostic(&path.to_string_lossy())
            )));
        }
    }

    let PluginOutputTotals {
        entries: mut entry_count,
        directories: mut directory_count,
        files: mut file_count,
        bytes: mut expanded_bytes,
        execution_time,
        execution_budget,
    } = totals;
    let mut stack = vec![output_dir.to_path_buf()];
    while let Some(path) = stack.pop() {
        entry_count += 1;
        if entry_count > MAX_PLUGIN_OUTPUT_ENTRIES {
            return Err(AppError::Validation(
                "archive plugin output contains too many entries".to_string(),
            ));
        }
        let metadata = std::fs::symlink_metadata(&path).map_err(|error| {
            AppError::Repository(format!(
                "failed to inspect archive plugin output {}: {error}",
                redact_archive_diagnostic(&path.to_string_lossy())
            ))
        })?;
        if metadata.file_type().is_symlink() {
            return Err(AppError::Validation(format!(
                "archive plugin output contains a symlink: {}",
                redact_archive_diagnostic(&path.to_string_lossy())
            )));
        }
        ensure_path_under_output_with_root(&path, &output_root)?;
        if metadata.is_dir() {
            directory_count += 1;
            if directory_count > MAX_PLUGIN_OUTPUT_DIRECTORIES {
                return Err(AppError::Validation(
                    "archive plugin output contains too many directories".to_string(),
                ));
            }
            for entry in std::fs::read_dir(&path).map_err(|error| {
                AppError::Repository(format!(
                    "failed to read archive plugin output directory {}: {error}",
                    redact_archive_diagnostic(&path.to_string_lossy())
                ))
            })? {
                let entry = entry.map_err(|error| {
                    AppError::Repository(format!(
                        "failed to read archive plugin output entry: {error}"
                    ))
                })?;
                if entry_count + stack.len() >= MAX_PLUGIN_OUTPUT_ENTRIES {
                    return Err(AppError::Validation(
                        "archive plugin output contains too many entries".to_string(),
                    ));
                }
                stack.push(entry.path());
            }
            continue;
        }
        if !metadata.is_file() {
            return Err(AppError::Validation(format!(
                "archive plugin output is not a regular file: {}",
                redact_archive_diagnostic(&path.to_string_lossy())
            )));
        }

        file_count += 1;
        if file_count > MAX_PLUGIN_OUTPUT_FILES {
            return Err(AppError::Validation(
                "archive plugin output contains too many files".to_string(),
            ));
        }
        expanded_bytes = expanded_bytes.checked_add(metadata.len()).ok_or_else(|| {
            AppError::Validation("archive plugin output is too large".to_string())
        })?;
        if expanded_bytes > MAX_PLUGIN_OUTPUT_BYTES {
            return Err(AppError::Validation(format!(
                "archive plugin output exceeds {} bytes",
                MAX_PLUGIN_OUTPUT_BYTES
            )));
        }
    }

    Ok(PluginOutputTotals {
        entries: entry_count,
        directories: directory_count,
        files: file_count,
        bytes: expanded_bytes,
        execution_time,
        execution_budget,
    })
}

fn safe_archive_output_path(output_dir: &Path, entry_name: &str) -> AppResult<PathBuf> {
    if entry_name.trim().is_empty() || entry_name.contains('\\') {
        return Err(AppError::Validation(format!(
            "unsafe archive entry path: {entry_name}"
        )));
    }

    let mut relative = PathBuf::new();
    for component in Path::new(entry_name).components() {
        match component {
            Component::Normal(part) => relative.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(AppError::Validation(format!(
                    "unsafe archive entry path: {entry_name}"
                )));
            }
        }
    }

    if relative.as_os_str().is_empty() {
        return Err(AppError::Validation(format!(
            "unsafe archive entry path: {entry_name}"
        )));
    }

    Ok(output_dir.join(relative))
}

fn ensure_path_under_output_with_root(path: &Path, output_root: &Path) -> AppResult<()> {
    let canonical = path.canonicalize().map_err(|e| {
        AppError::Repository(format!(
            "failed to canonicalize extraction path {}: {e}",
            redact_archive_diagnostic(&path.to_string_lossy())
        ))
    })?;
    if !canonical.starts_with(output_root) {
        return Err(AppError::Validation(format!(
            "archive entry escapes extraction directory: {}",
            redact_archive_diagnostic(&path.to_string_lossy())
        )));
    }
    Ok(())
}

#[cfg(test)]
fn has_video_files(dir: &Path) -> AppResult<bool> {
    has_importable_video_files(dir, |_| false)
}

/// Like `has_video_files`, but a video the import scan would discard as a
/// sample does not count.
#[cfg(test)]
fn has_importable_video_files(dir: &Path, is_sample: fn(&Path) -> bool) -> AppResult<bool> {
    Ok(archive_inventory(dir, 64)?
        .iter()
        .any(|path| scryer_domain::is_video_file(path) && !is_sample(path)))
}

/// Every archive set of a download, each named by its first volume: the top
/// level first, then subdirectories breadth-first in name order.
fn find_archive_sets(dir: &Path) -> AppResult<Vec<(PathBuf, ArchiveType)>> {
    let files = archive_inventory(dir, MAX_ARCHIVE_DISCOVERY_DEPTH)?;
    let mut by_directory = std::collections::BTreeMap::<PathBuf, Vec<PathBuf>>::new();
    for path in files {
        if let Some(parent) = path.parent() {
            by_directory
                .entry(parent.to_path_buf())
                .or_default()
                .push(path);
        }
    }
    let mut directories: Vec<_> = by_directory.into_iter().collect();
    directories.sort_by_key(|(path, _)| (path.components().count(), path.clone()));
    Ok(directories
        .into_iter()
        .flat_map(|(_, paths)| archive_sets_in_paths(paths))
        .collect())
}

/// A bounded inventory that never follows links or silently truncates a source.
fn archive_inventory(dir: &Path, max_depth: usize) -> AppResult<Vec<PathBuf>> {
    let io_error = |error| AppError::Repository(format!("archive discovery failed: {error}"));
    let metadata = std::fs::symlink_metadata(dir).map_err(io_error)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(AppError::Validation(
            "archive source is not a regular directory".into(),
        ));
    }
    let mut queue = std::collections::VecDeque::from([(dir.to_path_buf(), 0usize)]);
    let mut directories = 1usize;
    let mut entries_seen = 0usize;
    let mut files = Vec::new();
    while let Some((parent, depth)) = queue.pop_front() {
        let mut children = Vec::new();
        for entry in std::fs::read_dir(&parent).map_err(io_error)? {
            let entry = entry.map_err(io_error)?;
            entries_seen += 1;
            if entries_seen > MAX_PLUGIN_OUTPUT_ENTRIES {
                return Err(AppError::Validation(
                    "archive discovery entry limit exceeded".into(),
                ));
            }
            let kind = entry.file_type().map_err(io_error)?;
            let path = entry.path();
            if kind.is_dir() && !is_excluded_archive_discovery_dir(&path) {
                directories += 1;
                if depth >= max_depth || directories > MAX_ARCHIVE_DISCOVERY_DIRECTORIES {
                    return Err(AppError::Validation(
                        "archive discovery directory or depth limit exceeded".into(),
                    ));
                }
                children.push(path);
            } else if kind.is_file() {
                files.push(path);
            }
        }
        children.sort();
        queue.extend(children.into_iter().map(|path| (path, depth + 1)));
    }
    Ok(files)
}

fn is_excluded_archive_discovery_dir(path: &Path) -> bool {
    if is_archive_staging_dir(path) {
        return true;
    }
    path.file_name()
        .and_then(|name| name.to_str())
        .is_none_or(|name| {
            name.starts_with('.')
                || name.eq_ignore_ascii_case("sample")
                || name.eq_ignore_ascii_case("samples")
        })
}

/// The archive sets directly inside `dir`, each named by its first volume.
/// RAR volumes of one set collapse onto their first volume. Independent sets
/// of every supported format remain in the inventory.
#[cfg(test)]
fn archive_sets_in_dir(dir: &Path) -> Vec<(PathBuf, ArchiveType)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    archive_sets_in_paths(
        entries
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
            .map(|entry| entry.path())
            .collect(),
    )
}

fn archive_sets_in_paths(paths: Vec<PathBuf>) -> Vec<(PathBuf, ArchiveType)> {
    let mut rar = Vec::new();
    let mut sevenz = Vec::new();
    let mut zip = Vec::new();
    let mut xz = Vec::new();
    let mut par2 = std::collections::BTreeMap::new();

    for path in paths {
        match archive_type_for_path(&path) {
            Some(ArchiveType::Rar) => rar.push(path),
            Some(ArchiveType::SevenZip) => sevenz.push(path),
            Some(ArchiveType::Zip) => zip.push(path),
            Some(ArchiveType::Xz) => xz.push(path),
            Some(ArchiveType::Par2) => {
                let name = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_lowercase();
                let group = name
                    .rsplit_once(".vol")
                    .filter(|(_, suffix)| {
                        suffix.contains('+')
                            && suffix.chars().all(|c| c.is_ascii_digit() || c == '+')
                    })
                    .map_or(name.as_str(), |(stem, _)| stem)
                    .to_string();
                let key = (name != group, name);
                let entry = par2
                    .entry(group)
                    .or_insert_with(|| (key.clone(), path.clone()));
                if key < entry.0 {
                    *entry = (key, path);
                }
            }
            None => {}
        }
    }

    let mut sets: Vec<(PathBuf, ArchiveType)> = Vec::new();
    if !rar.is_empty() {
        rar.sort_by_key(|path| rar_selection_key(path));
        let mut last_group: Option<String> = None;
        for path in rar {
            let (group, _, _) = rar_selection_key(&path);
            if last_group.as_ref() != Some(&group) {
                last_group = Some(group);
                sets.push((path, ArchiveType::Rar));
            }
        }
    }
    sevenz.sort();
    zip.sort();
    xz.sort();
    sets.extend(sevenz.into_iter().map(|path| (path, ArchiveType::SevenZip)));
    sets.extend(zip.into_iter().map(|path| (path, ArchiveType::Zip)));
    sets.extend(xz.into_iter().map(|path| (path, ArchiveType::Xz)));
    // PAR2 metadata identifies obfuscated archives and protected plain media.
    // Successfully handled recovery metadata is skipped at invocation time;
    // independent sets remain discoverable beside conventional archive names.
    sets.extend(
        par2.into_values()
            .map(|(_, path)| (path, ArchiveType::Par2)),
    );
    sets
}

/// The archive type a file starts a set of, when it does. Split volumes
/// (`.7z.001`, `.zip.001`, bare `.001`) start a set only at volume `001`; the
/// plugin is handed that first volume and joins its siblings itself.
fn archive_type_for_path(path: &Path) -> Option<ArchiveType> {
    match path
        .extension()
        .and_then(|extension| extension.to_str())?
        .to_ascii_lowercase()
        .as_str()
    {
        "rar" => Some(ArchiveType::Rar),
        "par2" => Some(ArchiveType::Par2),
        "r00" if !path.with_extension("rar").exists() => Some(ArchiveType::Rar),
        "7z" => Some(ArchiveType::SevenZip),
        "zip" => Some(ArchiveType::Zip),
        "xz" => Some(ArchiveType::Xz),
        "001" => split_set_archive_type(path),
        _ => None,
    }
}

/// The inner format of a split set's first volume: named by the extension
/// before `.001`, or for a bare `.001` read from the volume's signature, so a
/// split video or a split RAR set is not taken for one.
fn split_set_archive_type(first_volume: &Path) -> Option<ArchiveType> {
    const SEVEN_ZIP_SIGNATURE: &[u8] = b"7z\xbc\xaf\x27\x1c";
    const ZIP_SIGNATURE: &[u8] = b"PK\x03\x04";

    let stem = Path::new(first_volume.file_stem()?);
    match stem
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("7z") => return Some(ArchiveType::SevenZip),
        Some("zip") => return Some(ArchiveType::Zip),
        _ => {}
    }
    let mut signature = [0u8; 6];
    let mut file = std::fs::File::open(first_volume).ok()?;
    std::io::Read::read_exact(&mut file, &mut signature).ok()?;
    if signature.starts_with(SEVEN_ZIP_SIGNATURE) {
        Some(ArchiveType::SevenZip)
    } else if signature.starts_with(ZIP_SIGNATURE) {
        Some(ArchiveType::Zip)
    } else {
        None
    }
}

fn rar_selection_key(path: &Path) -> (String, usize, String) {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let (group, index) = rar_volume_info_from_name(&name).unwrap_or_else(|| (name.clone(), 0));
    (group, index, name)
}

fn rar_volume_info_from_name(file_name: &str) -> Option<(String, usize)> {
    if let Some(stem) = file_name.strip_suffix(".rar") {
        if let Some((group, part)) = stem.rsplit_once(".part")
            && let Ok(part_index) = part.parse::<usize>()
            && part_index > 0
        {
            return Some((group.to_string(), part_index - 1));
        }
        return Some((stem.to_string(), 0));
    }

    let (group, extension) = file_name.rsplit_once('.')?;
    if !is_old_rar_volume_extension(extension) {
        return None;
    }
    let mut chars = extension.chars();
    let family = chars.next()?;
    let digits = chars.as_str();
    let number = digits.parse::<usize>().ok()?;
    let family_offset = (family as u8).checked_sub(b'r')? as usize;
    Some((group.to_string(), family_offset * 100 + number + 1))
}

/// Release an abandoned automatic import without touching downloads. An
/// unreadable or unsafe inventory keeps the workspace referenced, and so do
/// subtitles once a video from the workspace may have been imported.
#[derive(Default)]
pub(crate) struct ArchiveWorkspaceReference {
    root: Option<PathBuf>,
    video_imported: bool,
}

impl ArchiveWorkspaceReference {
    pub(crate) fn track(&mut self, root: Option<&Path>) {
        self.root = root.map(Path::to_path_buf);
    }

    pub(crate) fn retain(&mut self) {
        self.root = None;
    }

    /// Record that a video from the workspace may have been placed, so its
    /// subtitles stay available for delivery.
    pub(crate) fn mark_video_imported(&mut self) {
        self.video_imported = true;
    }
}

impl Drop for ArchiveWorkspaceReference {
    fn drop(&mut self) {
        let Some(root) = &self.root else {
            return;
        };
        if archive_workspace_owner(root).is_none() {
            return;
        }
        if !archive_workspace_inventory(root).permits_release(self.video_imported) {
            return;
        }
        if release_archive_workspace(root).is_err() {
            tracing::warn!("abandoned archive workspace could not be released; output preserved");
        }
    }
}

/// What a workspace inventory permits when the workspace is let go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ArchiveWorkspaceInventory {
    /// Only regular files and directories, none of them subtitles.
    Clean,
    /// Regular files and directories, including at least one subtitle.
    HasSubtitles,
    /// A symlink, special file, unreadable entry, or a depth or entry count
    /// beyond the bound. Such a workspace is always preserved.
    Unsafe,
}

impl ArchiveWorkspaceInventory {
    /// Whether the workspace may be released for later removal. Subtitles keep
    /// it only when a video from it was imported and is waiting for them.
    fn permits_release(self, video_imported: bool) -> bool {
        match self {
            Self::Clean => true,
            Self::HasSubtitles => !video_imported,
            Self::Unsafe => false,
        }
    }
}

/// The regular files of a workspace worth knowing about before letting it go.
struct ArchiveWorkspaceScan {
    has_subtitles: bool,
    /// Every video in the workspace, as a path under the scanned root.
    videos: Vec<PathBuf>,
}

/// Walk a workspace within the output bounds. `None` when it holds a symlink,
/// a special file, an unreadable entry, or exceeds the depth or entry bound.
fn scan_archive_workspace(root: &Path) -> Option<ArchiveWorkspaceScan> {
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    let mut entries = 0usize;
    let mut scan = ArchiveWorkspaceScan {
        has_subtitles: false,
        videos: Vec::new(),
    };
    while let Some((parent, depth)) = stack.pop() {
        if depth > 64 || entries > MAX_PLUGIN_OUTPUT_ENTRIES {
            return None;
        }
        let children = std::fs::read_dir(parent).ok()?;
        for child in children {
            let child = child.ok()?;
            entries += 1;
            if entries > MAX_PLUGIN_OUTPUT_ENTRIES {
                return None;
            }
            let kind = child.file_type().ok()?;
            if kind.is_symlink() || (!kind.is_dir() && !kind.is_file()) {
                return None;
            }
            let path = child.path();
            if kind.is_dir() {
                stack.push((path, depth + 1));
            } else if path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| {
                    matches!(
                        ext.to_ascii_lowercase().as_str(),
                        "srt" | "ass" | "ssa" | "vtt" | "sub" | "idx"
                    )
                })
            {
                scan.has_subtitles = true;
            } else if scryer_domain::is_video_file(&path) {
                scan.videos.push(path);
            }
        }
    }
    Some(scan)
}

fn archive_workspace_inventory(root: &Path) -> ArchiveWorkspaceInventory {
    match scan_archive_workspace(root) {
        None => ArchiveWorkspaceInventory::Unsafe,
        Some(scan) if scan.has_subtitles => ArchiveWorkspaceInventory::HasSubtitles,
        Some(_) => ArchiveWorkspaceInventory::Clean,
    }
}

/// Let go of a workspace no video was imported from. A clean workspace is
/// removed at once; one holding subtitles is only released, so the stale
/// sweeper removes it later. An unsafe inventory is preserved.
pub(crate) async fn abandon_extracted_dir(dir: &Path) {
    match archive_workspace_inventory(dir) {
        ArchiveWorkspaceInventory::Clean => cleanup_extracted_dir(dir).await,
        ArchiveWorkspaceInventory::HasSubtitles => {
            if archive_workspace_owner(dir).is_some()
                && let Err(error) = release_archive_workspace(dir)
            {
                tracing::warn!(error = %redact_archive_diagnostic(&error.to_string()), "abandoned archive workspace could not be released; output preserved");
            }
        }
        ArchiveWorkspaceInventory::Unsafe => {}
    }
}

/// Owned workspaces directly under `parent` whose authenticated owner is
/// `owner_id`. Ownership is proven by the keyed marker, never by name or age,
/// so a match identifies the workspace with certainty. The scan is bounded;
/// `complete` is false when the folder could not be read in full, so a
/// workspace of that owner may exist that was not found.
pub(crate) fn owned_archive_workspaces_for(
    parent: &Path,
    owner_id: &str,
) -> crate::import_workflow::OwnedWorkspaceLookup {
    const MAX_SCANNED_ENTRIES: usize = 4096;
    let mut lookup = crate::import_workflow::OwnedWorkspaceLookup {
        workspaces: Vec::new(),
        complete: true,
    };
    let Ok(entries) = std::fs::read_dir(parent) else {
        lookup.complete = !parent.exists();
        return lookup;
    };
    for (index, entry) in entries.enumerate() {
        if index >= MAX_SCANNED_ENTRIES {
            lookup.complete = false;
            break;
        }
        let Ok(entry) = entry else {
            lookup.complete = false;
            continue;
        };
        let path = entry.path();
        if archive_workspace_owned_by(&path, owner_id) {
            lookup.workspaces.push(path);
        }
    }
    lookup
}

/// Whether `root` is an owned workspace whose authenticated owner is `owner_id`.
fn archive_workspace_owned_by(root: &Path, owner_id: &str) -> bool {
    archive_workspace_owner(root).is_some_and(|owner| owner.import_id == owner_id)
}

/// Remove a held workspace an operator released, through the ordinary
/// cleanup. The workspace is preserved when it is not owned, its inventory is
/// unsafe (such as a symlink), or any video in it is not proven imported.
///
/// A video is proven imported only when its path relative to `download_root`
/// (the folder import artifacts record their source paths against) is one of
/// `imported_relative_paths`. A video with no path relative to that folder
/// cannot be matched, so it counts as never imported.
pub(crate) async fn remove_released_held_workspace(
    root: &Path,
    download_root: &Path,
    imported_relative_paths: &std::collections::HashSet<String>,
) -> Result<(), crate::import_workflow::HeldWorkspacePreserved> {
    use crate::import_workflow::HeldWorkspacePreserved;
    if archive_workspace_owner(root).is_none() {
        return Err(HeldWorkspacePreserved::NotOwned);
    }
    let scan = scan_archive_workspace(root).ok_or(HeldWorkspacePreserved::Unsafe)?;
    let proven_imported = |video: &PathBuf| {
        video
            .strip_prefix(download_root)
            .ok()
            .map(crate::stored_paths::path_to_stored_string)
            .filter(|relative| !relative.is_empty())
            .is_some_and(|relative| imported_relative_paths.contains(&relative))
    };
    if !scan.videos.iter().all(proven_imported) {
        return Err(HeldWorkspacePreserved::HoldsUnimportedVideo);
    }
    cleanup_extracted_dir(root).await;
    if root.exists() {
        Err(HeldWorkspacePreserved::RemovalFailed)
    } else {
        Ok(())
    }
}

/// Clean up the extraction directory after import completes.
pub async fn cleanup_extracted_dir(dir: &Path) {
    if archive_workspace_owner(dir).is_none() {
        return;
    }
    // Callers release only after their workflow/selection no longer needs the
    // output. If removal fails, the release record permits a later retry.
    if let Err(error) = release_archive_workspace(dir) {
        tracing::warn!(error = %redact_archive_diagnostic(&error.to_string()), "archive workspace release could not be authenticated");
        return;
    }
    if archive_workspace_release(dir).is_none() {
        return;
    }
    if let Err(error) = tokio::fs::remove_dir_all(dir).await {
        tracing::warn!(error = %redact_archive_diagnostic(&error.to_string()), "released archive workspace could not be removed");
    }
}

/// Whether `source` lies inside an output directory of one of the extractor's
/// own workspaces, staged in a folder that also holds `dest`. Workspaces are
/// only ever staged in the title folder the output is imported into, so a
/// download that merely carries the generated names is never treated as
/// scratch.
pub fn is_archive_workspace_output(source: &Path, dest: &Path) -> bool {
    let Ok(canonical_source) = source.canonicalize() else {
        return false;
    };
    source.ancestors().skip(1).any(|output_dir| {
        output_dir
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(is_workspace_output_dir_name)
            && std::fs::symlink_metadata(output_dir)
                .is_ok_and(|metadata| metadata.file_type().is_dir())
            && output_dir
                .canonicalize()
                .is_ok_and(|output| canonical_source.starts_with(output))
            && output_dir.parent().is_some_and(|workspace| {
                archive_workspace_owner(workspace).is_some()
                    && workspace.parent().is_some_and(|staging_parent| {
                        !staging_parent.as_os_str().is_empty() && dest.starts_with(staging_parent)
                    })
            })
    })
}

fn is_archive_workspace_name(name: &str) -> bool {
    name.strip_prefix(ARCHIVE_STAGING_PREFIX)
        .is_some_and(|suffix| {
            suffix.len() == 16
                && suffix
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        })
}

fn is_archive_staging_dir(dir: &Path) -> bool {
    dir.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name == EXTRACTED_DIR_NAME
                || name.starts_with(ARCHIVE_STAGING_PREFIX)
                || name.starts_with(LEGACY_ARCHIVE_STAGING_PREFIX)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::workflow::{is_sample_file, is_sample_named_file};
    use crate::{ArchiveExtractorClient, ArchiveExtractorPluginProvider};
    use scryer_plugin_sdk::ArchivePluginExtractedFile;
    use std::fs;
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn abandoned_archive_releases_only_owned_output_without_pending_subtitles() {
        let destination = tempfile::tempdir().unwrap();
        let mut workspace = ArchiveExtractionWorkspace::create(&ArchiveExtractionDestination::new(
            destination.path(),
            "synthetic-import",
        ))
        .await
        .unwrap();
        workspace.lease.published = true;
        let root = workspace.root.clone();
        fs::write(workspace.output_dir.join("movie.mkv"), b"synthetic").unwrap();
        let mut reference = ArchiveWorkspaceReference::default();
        reference.track(Some(&root));
        drop(reference);
        assert!(archive_workspace_release(&root).is_some());
        let unrelated = destination.path().join("keep.mkv");
        fs::write(&unrelated, b"keep").unwrap();
        cleanup_extracted_dir(&root).await;
        assert!(!root.exists());
        assert_eq!(fs::read(unrelated).unwrap(), b"keep");

        let workspace = ArchiveExtractionWorkspace::create(&ArchiveExtractionDestination::new(
            destination.path(),
            "pending-import",
        ))
        .await
        .unwrap();
        let pending_path = workspace.output_dir.join(".hidden/Sample/movie.srt");
        fs::create_dir_all(pending_path.parent().unwrap()).unwrap();
        fs::write(&pending_path, b"pending").unwrap();
        let pending_root = workspace.root.clone();
        let mut reference = ArchiveWorkspaceReference::default();
        reference.track(Some(&workspace.root));
        reference.mark_video_imported();
        drop(reference);
        assert!(archive_workspace_release(&workspace.root).is_none());
        let mut workspace = workspace;
        workspace.lease.published = true;
        drop(workspace);
        cleanup_archive_artifacts_older_than(destination.path(), Duration::ZERO).await;
        assert_eq!(fs::read(&pending_path).unwrap(), b"pending");
        assert!(archive_workspace_release(&pending_root).is_none());

        let retained = create_test_archive_workspace(destination.path());
        fs::write(retained.join("out/movie.mkv"), b"selected").unwrap();
        let mut reference = ArchiveWorkspaceReference::default();
        reference.track(Some(&retained));
        reference.retain();
        drop(reference);
        cleanup_archive_artifacts_older_than(destination.path(), Duration::ZERO).await;
        assert!(retained.join("out/movie.mkv").exists());
    }

    fn workspace_with_subtitle(parent: &Path) -> PathBuf {
        let root = create_test_archive_workspace(parent);
        fs::create_dir_all(root.join("out/Subs")).unwrap();
        fs::write(root.join("out/feature.mkv"), b"synthetic video").unwrap();
        fs::write(root.join("out/Subs/feature.srt"), b"synthetic subtitle").unwrap();
        root
    }

    #[tokio::test]
    async fn workspace_without_an_imported_video_is_released_despite_subtitles() {
        let destination = tempfile::tempdir().unwrap();
        let unrelated = destination.path().join("keep.srt");
        fs::write(&unrelated, b"keep").unwrap();

        // Dropped reference, nothing imported: released, then swept.
        let dropped = workspace_with_subtitle(destination.path());
        let mut reference = ArchiveWorkspaceReference::default();
        reference.track(Some(&dropped));
        drop(reference);
        assert!(archive_workspace_release(&dropped).is_some());

        // Abandoned after a failed resolution: released, not removed at once.
        let abandoned = workspace_with_subtitle(destination.path());
        abandon_extracted_dir(&abandoned).await;
        assert!(archive_workspace_release(&abandoned).is_some());
        assert!(abandoned.join("out/Subs/feature.srt").exists());

        // An unpublished workspace was never handed out.
        let mut unpublished = ArchiveExtractionWorkspace::create(
            &ArchiveExtractionDestination::new(destination.path(), "synthetic-unpublished"),
        )
        .await
        .unwrap();
        fs::write(unpublished.output_dir.join("feature.ass"), b"synthetic").unwrap();
        let unpublished_root = unpublished.root.clone();
        unpublished.lease.published = false;
        drop(unpublished);
        assert!(archive_workspace_release(&unpublished_root).is_some());

        // A video was imported and its subtitles are pending: kept.
        let pending = workspace_with_subtitle(destination.path());
        let mut reference = ArchiveWorkspaceReference::default();
        reference.track(Some(&pending));
        reference.mark_video_imported();
        drop(reference);
        assert!(archive_workspace_release(&pending).is_none());

        // The sweeper honours its stale window, then removes only released
        // workspaces.
        cleanup_archive_artifacts_older_than(destination.path(), STALE_ARCHIVE_STAGING_AFTER).await;
        assert!(dropped.exists() && abandoned.exists() && unpublished_root.exists());
        cleanup_archive_artifacts_older_than(destination.path(), Duration::ZERO).await;
        assert!(!dropped.exists());
        assert!(!abandoned.exists());
        assert!(!unpublished_root.exists());
        assert_eq!(
            fs::read(pending.join("out/Subs/feature.srt")).unwrap(),
            b"synthetic subtitle"
        );
        assert_eq!(fs::read(&unrelated).unwrap(), b"keep");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn workspace_with_a_symlink_is_never_released_even_without_an_imported_video() {
        let destination = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("outside.mkv");
        fs::write(&target, b"outside").unwrap();

        let linked = create_test_archive_workspace(destination.path());
        std::os::unix::fs::symlink(&target, linked.join("out/link.mkv")).unwrap();
        let mut reference = ArchiveWorkspaceReference::default();
        reference.track(Some(&linked));
        drop(reference);
        abandon_extracted_dir(&linked).await;
        assert!(archive_workspace_release(&linked).is_none());

        let mut unpublished = ArchiveExtractionWorkspace::create(
            &ArchiveExtractionDestination::new(destination.path(), "synthetic-linked"),
        )
        .await
        .unwrap();
        std::os::unix::fs::symlink(&target, unpublished.output_dir.join("link.srt")).unwrap();
        let unpublished_root = unpublished.root.clone();
        unpublished.lease.published = false;
        drop(unpublished);
        assert!(archive_workspace_release(&unpublished_root).is_none());

        cleanup_archive_artifacts_older_than(destination.path(), Duration::ZERO).await;
        assert!(linked.join("out/link.mkv").symlink_metadata().is_ok());
        assert!(unpublished_root.exists());
        assert_eq!(fs::read(&target).unwrap(), b"outside");
    }

    #[test]
    fn unsafe_workspace_key_keeps_startup_usable_without_modifying_existing_key() {
        let state = tempfile::tempdir().unwrap();
        let path = state.path().join("archive-workspace-key");
        fs::write(&path, b"invalid-key").unwrap();
        let first = archive_workspace_key_for_startup(state.path());
        let second = archive_workspace_key_for_startup(state.path());
        assert_ne!(first, second);
        assert_eq!(fs::read(path).unwrap(), b"invalid-key");
    }

    #[cfg(unix)]
    #[test]
    fn unsafe_workspace_key_permissions_use_ephemeral_authority() {
        use std::os::unix::fs::PermissionsExt;
        let state = tempfile::tempdir().unwrap();
        let path = state.path().join("archive-workspace-key");
        let persistent_key = [17u8; 32];
        fs::write(&path, persistent_key).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert_ne!(
            archive_workspace_key_for_startup(state.path()),
            persistent_key
        );
        assert_eq!(fs::read(&path).unwrap(), persistent_key);
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o644
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            archive_workspace_key_for_startup(state.path()),
            persistent_key
        );
    }

    #[derive(Clone, Default)]
    struct CapturedArchiveLogs(Arc<Mutex<String>>);

    impl tracing::Subscriber for CapturedArchiveLogs {
        fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            struct Fields<'a>(&'a mut String);
            impl tracing::field::Visit for Fields<'_> {
                fn record_debug(&mut self, _: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                    use std::fmt::Write;
                    write!(self.0, "{value:?}\n").unwrap();
                }
            }
            event.record(&mut Fields(&mut self.0.lock().unwrap()));
        }
        fn enter(&self, _: &tracing::span::Id) {}
        fn exit(&self, _: &tracing::span::Id) {}
    }

    #[tokio::test]
    async fn archive_password_annotations_are_absent_from_logs_and_failure_diagnostics() {
        let capture = CapturedArchiveLogs::default();
        let _subscriber = tracing::subscriber::set_default(capture.clone());
        let source = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        fs::write(
            source.path().join("release{{synthetic-secret}}.rar"),
            b"archive",
        )
        .unwrap();
        let provider = Arc::new(RecordingArchiveProvider {
            client: Arc::new(ScriptedArchiveClient {
                emitted: Vec::new(),
                status: ArchivePluginStatus::Failed,
                error_code: Some("corrupt_archive"),
                message: Some("could not read member{{synthetic-secret}}.mkv"),
                copied_bytes: None,
                replaced_sources: Vec::new(),
            }),
            formats: vec![ArchivePluginFormat::Rar],
        });
        let error = extract_archives_if_needed(
            source.path(),
            is_sample_named_file,
            Some(ArchiveExtractionDestination::new(
                destination.path(),
                "diagnostic-test",
            )),
            &Default::default(),
            Some(provider),
        )
        .await
        .unwrap_err();
        assert!(!error.to_string().contains("synthetic-secret"));
        assert!(error.to_string().contains("member[redacted].mkv"));
        let logs = capture.0.lock().unwrap();
        assert!(!logs.contains("synthetic-secret"), "{logs}");
        assert!(logs.contains("release[redacted].rar"), "{logs}");
    }

    struct NestedPasswordClient {
        format: ArchivePluginFormat,
        password: &'static str,
        calls: Arc<Mutex<Vec<Option<String>>>>,
    }

    #[async_trait::async_trait]
    impl ArchiveExtractorClient for NestedPasswordClient {
        async fn process(
            &self,
            request: ArchivePluginProcessRequest,
        ) -> AppResult<ArchivePluginProcessResponse> {
            let ArchivePluginOperation::ExtractArchive {
                output_dir,
                password,
                ..
            } = request.operation
            else {
                panic!("extract expected");
            };
            self.calls.lock().unwrap().push(password.clone());
            let mut response = ArchivePluginProcessResponse {
                status: ArchivePluginStatus::Failed,
                files: Vec::new(),
                replaced_source_paths: Vec::new(),
                processed_recovery_paths: Vec::new(),
                expanded_bytes: None,
                copied_bytes: None,
                staged_bytes: None,
                error_code: None,
                message: None,
            };
            if password.as_deref() != Some(self.password) {
                response.status = if password.is_none() {
                    ArchivePluginStatus::PasswordRequired
                } else {
                    ArchivePluginStatus::PasswordInvalid
                };
                return Ok(response);
            }
            let name = if self.format == ArchivePluginFormat::Rar {
                "nested.zip"
            } else {
                "movie.mkv"
            };
            fs::write(Path::new(&output_dir).join(name), b"fixture").unwrap();
            response.status = ArchivePluginStatus::Ok;
            response.files.push(ArchivePluginExtractedFile {
                relative_path: name.into(),
                size: Some(7),
                checksum: None,
            });
            Ok(response)
        }
    }

    struct NestedPasswordProvider {
        clients: Vec<(
            ArchivePluginFormat,
            Arc<dyn ArchiveExtractorClient>,
            &'static str,
        )>,
        selections: Mutex<Vec<ArchivePluginFormat>>,
    }

    impl ArchiveExtractorPluginProvider for NestedPasswordProvider {
        fn client_for_format(
            &self,
            _: ArchivePluginFormat,
        ) -> Option<Arc<dyn ArchiveExtractorClient>> {
            panic!("selection must be pinned");
        }
        fn available_provider_types(&self) -> Vec<String> {
            vec!["shared-archive-type".into()]
        }
        fn select_for_format(
            &self,
            format: ArchivePluginFormat,
        ) -> AppResult<Option<crate::ArchiveExtractorSelection>> {
            self.selections.lock().unwrap().push(format);
            Ok(self
                .clients
                .iter()
                .find(|(candidate, _, _)| *candidate == format)
                .map(|(_, client, password)| {
                    let mut passwords = ArchivePasswordCandidates::default();
                    passwords.extend_settings(password);
                    crate::ArchiveExtractorSelection {
                        installation_id: Some(format!("installation-{format:?}")),
                        client: client.clone(),
                        passwords,
                    }
                }))
        }
    }

    #[tokio::test]
    async fn archive_nested_sets_use_only_their_selected_installation_passwords() {
        let source = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        fs::write(source.path().join("release.rar"), b"archive").unwrap();
        let rar_calls = Arc::new(Mutex::new(Vec::new()));
        let zip_calls = Arc::new(Mutex::new(Vec::new()));
        let provider = Arc::new(NestedPasswordProvider {
            clients: vec![
                (
                    ArchivePluginFormat::Rar,
                    Arc::new(NestedPasswordClient {
                        format: ArchivePluginFormat::Rar,
                        password: "synthetic-rar",
                        calls: rar_calls.clone(),
                    }),
                    "synthetic-rar",
                ),
                (
                    ArchivePluginFormat::Zip,
                    Arc::new(NestedPasswordClient {
                        format: ArchivePluginFormat::Zip,
                        password: "synthetic-zip",
                        calls: zip_calls.clone(),
                    }),
                    "synthetic-zip",
                ),
            ],
            selections: Mutex::new(Vec::new()),
        });
        let mut passwords = ArchivePasswordCandidates::default();
        passwords.push_operator(Some("synthetic-explicit"));
        extract_archives_if_needed(
            source.path(),
            is_sample_named_file,
            Some(ArchiveExtractionDestination::new(
                destination.path(),
                "nested-installations",
            )),
            &passwords,
            Some(provider.clone()),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            *rar_calls.lock().unwrap(),
            vec![
                None,
                Some("synthetic-explicit".into()),
                Some("synthetic-rar".into())
            ]
        );
        assert_eq!(
            *zip_calls.lock().unwrap(),
            vec![
                None,
                Some("synthetic-explicit".into()),
                Some("synthetic-zip".into())
            ]
        );
        assert_eq!(
            *provider.selections.lock().unwrap(),
            vec![ArchivePluginFormat::Rar, ArchivePluginFormat::Zip]
        );
    }

    struct RecordingArchiveClient {
        operation: Arc<Mutex<Option<ArchivePluginOperation>>>,
        write_output_file: bool,
    }

    #[async_trait::async_trait]
    impl ArchiveExtractorClient for RecordingArchiveClient {
        async fn process(
            &self,
            request: ArchivePluginProcessRequest,
        ) -> AppResult<ArchivePluginProcessResponse> {
            let output_dir = match &request.operation {
                ArchivePluginOperation::ExtractArchive { output_dir, .. } => {
                    PathBuf::from(output_dir)
                }
                ArchivePluginOperation::Inspect { .. } => {
                    return Ok(ArchivePluginProcessResponse {
                        status: ArchivePluginStatus::Failed,
                        files: Vec::new(),
                        replaced_source_paths: Vec::new(),
                        processed_recovery_paths: Vec::new(),
                        expanded_bytes: None,
                        copied_bytes: None,
                        staged_bytes: None,
                        error_code: Some("unsupported_operation".to_string()),
                        message: Some("operation does not extract archive files".to_string()),
                    });
                }
            };
            *self.operation.lock().unwrap() = Some(request.operation);
            let files = if self.write_output_file {
                fs::create_dir_all(&output_dir).unwrap();
                let output_file = output_dir.join("movie.mkv");
                fs::write(&output_file, b"fake video").unwrap();
                vec![ArchivePluginExtractedFile {
                    relative_path: "movie.mkv".to_string(),
                    size: Some(10),
                    checksum: None,
                }]
            } else {
                Vec::new()
            };
            Ok(ArchivePluginProcessResponse {
                status: ArchivePluginStatus::Ok,
                files,
                replaced_source_paths: Vec::new(),
                processed_recovery_paths: Vec::new(),
                expanded_bytes: Some(if self.write_output_file { 10 } else { 0 }),
                copied_bytes: None,
                staged_bytes: None,
                error_code: None,
                message: None,
            })
        }
    }

    struct RecordingArchiveProvider {
        client: Arc<dyn ArchiveExtractorClient>,
        formats: Vec<ArchivePluginFormat>,
    }

    impl ArchiveExtractorPluginProvider for RecordingArchiveProvider {
        fn client_for_format(
            &self,
            format: ArchivePluginFormat,
        ) -> Option<Arc<dyn ArchiveExtractorClient>> {
            self.formats
                .contains(&format)
                .then(|| Arc::clone(&self.client))
        }

        fn available_provider_types(&self) -> Vec<String> {
            vec!["recording".to_string()]
        }
    }

    /// Stands in for the plugin's internal PAR2 pass: writes a caller-supplied
    /// set of plain files into the plugin's writable output directory and
    /// answers with a caller-supplied status, so the host-side consumption of
    /// `files` can be exercised without the native pipeline that used to sit in
    /// front of it.
    struct ScriptedArchiveClient {
        emitted: Vec<(&'static str, &'static [u8])>,
        status: ArchivePluginStatus,
        error_code: Option<&'static str>,
        message: Option<&'static str>,
        copied_bytes: Option<u64>,
        replaced_sources: Vec<String>,
    }

    #[async_trait::async_trait]
    impl ArchiveExtractorClient for ScriptedArchiveClient {
        async fn process(
            &self,
            request: ArchivePluginProcessRequest,
        ) -> AppResult<ArchivePluginProcessResponse> {
            let ArchivePluginOperation::ExtractArchive { output_dir, .. } = &request.operation
            else {
                panic!("expected an extract operation");
            };
            let output_dir = PathBuf::from(output_dir);
            fs::create_dir_all(&output_dir).unwrap();

            let mut files = Vec::new();
            for (relative_path, bytes) in &self.emitted {
                let path = output_dir.join(relative_path);
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).unwrap();
                }
                fs::write(&path, bytes).unwrap();
                files.push(ArchivePluginExtractedFile {
                    relative_path: (*relative_path).to_string(),
                    size: Some(bytes.len() as u64),
                    checksum: None,
                });
            }

            Ok(ArchivePluginProcessResponse {
                status: self.status,
                files,
                replaced_source_paths: self.replaced_sources.clone(),
                processed_recovery_paths: Vec::new(),
                expanded_bytes: None,
                copied_bytes: self.copied_bytes,
                staged_bytes: None,
                error_code: self.error_code.map(ToOwned::to_owned),
                message: self.message.map(ToOwned::to_owned),
            })
        }
    }

    fn scripted_provider(client: ScriptedArchiveClient) -> Arc<dyn ArchiveExtractorPluginProvider> {
        let client: Arc<dyn ArchiveExtractorClient> = Arc::new(client);
        Arc::new(RecordingArchiveProvider {
            client,
            formats: vec![
                ArchivePluginFormat::Rar,
                ArchivePluginFormat::SevenZip,
                ArchivePluginFormat::Zip,
            ],
        })
    }

    fn first_archive_set(dir: &Path) -> Option<(PathBuf, ArchiveType)> {
        find_archive_sets(dir).unwrap().into_iter().next()
    }

    fn operator_password(value: &str) -> ArchivePasswordCandidates {
        let mut candidates = ArchivePasswordCandidates::default();
        candidates.push_operator(Some(value));
        candidates
    }

    /// One scripted plugin answer per extraction call.
    enum PluginStep {
        Extracts,
        Answers(
            ArchivePluginStatus,
            Option<&'static str>,
            Option<&'static str>,
        ),
        TimesOut,
    }

    /// Answers each call with the next scripted step and records the password
    /// every call carried. Every call first writes a partial member into its
    /// output directory, so leftovers from refused attempts would be visible.
    struct SequencedArchiveClient {
        steps: Mutex<std::collections::VecDeque<PluginStep>>,
        passwords: Arc<Mutex<Vec<Option<String>>>>,
    }

    #[async_trait::async_trait]
    impl ArchiveExtractorClient for SequencedArchiveClient {
        async fn process(
            &self,
            request: ArchivePluginProcessRequest,
        ) -> AppResult<ArchivePluginProcessResponse> {
            let ArchivePluginOperation::ExtractArchive {
                output_dir,
                password,
                ..
            } = request.operation
            else {
                panic!("expected an extract operation");
            };
            self.passwords.lock().unwrap().push(password);
            let output_dir = PathBuf::from(output_dir);
            fs::write(output_dir.join("partial.mkv"), b"partial").unwrap();
            let step = self
                .steps
                .lock()
                .unwrap()
                .pop_front()
                .expect("the extractor made more plugin calls than scripted");
            let (status, error_code, message, files) = match step {
                PluginStep::Extracts => (
                    ArchivePluginStatus::Ok,
                    None,
                    None,
                    vec![ArchivePluginExtractedFile {
                        relative_path: "partial.mkv".to_string(),
                        size: Some(7),
                        checksum: None,
                    }],
                ),
                PluginStep::Answers(status, error_code, message) => {
                    (status, error_code, message, Vec::new())
                }
                PluginStep::TimesOut => {
                    return Err(AppError::archive_extraction_timed_out(
                        "archive plugin timed out after 3600 seconds".to_string(),
                    ));
                }
            };
            Ok(ArchivePluginProcessResponse {
                status,
                files,
                replaced_source_paths: Vec::new(),
                processed_recovery_paths: Vec::new(),
                expanded_bytes: None,
                copied_bytes: None,
                staged_bytes: None,
                error_code: error_code.map(ToOwned::to_owned),
                message: message.map(ToOwned::to_owned),
            })
        }
    }

    struct PasswordLoopRun {
        result: AppResult<Option<PathBuf>>,
        passwords: Vec<Option<String>>,
        staging_dirs: usize,
        staged_files: usize,
    }

    fn count_files(dir: &Path) -> usize {
        fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|entry| {
                let path = entry.path();
                if path.is_dir() {
                    count_files(&path)
                } else {
                    usize::from(entry.file_name() != ARCHIVE_WORKSPACE_OWNER_FILE)
                }
            })
            .sum()
    }

    async fn run_password_loop(
        steps: Vec<PluginStep>,
        candidates: &ArchivePasswordCandidates,
    ) -> PasswordLoopRun {
        let source = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        fs::write(source.path().join("quiet.harbor.s02e03.rar"), b"rar").unwrap();
        let passwords = Arc::new(Mutex::new(Vec::new()));
        let client: Arc<dyn ArchiveExtractorClient> = Arc::new(SequencedArchiveClient {
            steps: Mutex::new(steps.into()),
            passwords: Arc::clone(&passwords),
        });
        let provider: Arc<dyn ArchiveExtractorPluginProvider> =
            Arc::new(RecordingArchiveProvider {
                client,
                formats: vec![ArchivePluginFormat::Rar],
            });

        let result = extract_archives_if_needed(
            source.path(),
            is_sample_named_file,
            Some(ArchiveExtractionDestination::new(
                destination.path(),
                "password-loop",
            )),
            candidates,
            Some(provider),
        )
        .await;

        let staging_dirs = fs::read_dir(destination.path())
            .unwrap()
            .flatten()
            .filter(|entry| is_archive_staging_dir(&entry.path()))
            .count();
        let staged_files = count_files(destination.path());
        assert!(source.path().join("quiet.harbor.s02e03.rar").exists());
        let passwords = passwords.lock().unwrap().clone();
        PasswordLoopRun {
            result,
            passwords,
            staging_dirs,
            staged_files,
        }
    }

    fn operator_indexer_and_name_candidates() -> ArchivePasswordCandidates {
        let mut candidates = ArchivePasswordCandidates::default();
        candidates.push_operator(Some("typed-guess"));
        candidates.push_indexer(Some("indexer-secret"));
        candidates.push_release_name(Some("Quiet.Harbor.S02E03{{name-secret}}"));
        candidates
    }

    #[tokio::test]
    async fn archive_workspace_output_is_recognised_only_inside_a_workspace() {
        let root = tempfile::tempdir().unwrap();
        let title = root.path();
        let dest = title.join("Season 02/Quiet Harbor - S02E03.mkv");
        let workspace = ArchiveExtractionWorkspace::create(&ArchiveExtractionDestination::new(
            title,
            "test-import",
        ))
        .await
        .unwrap()
        .root;
        for output in [
            "out/Quiet.Harbor.S02E03.mkv",
            "out/Subs/Quiet.Harbor.S02E03.mkv",
            "out-2/Quiet.Harbor.S02E03.mkv",
            "nested-1-1/Quiet.Harbor.S02E03.mkv",
        ] {
            fs::create_dir_all(workspace.join(output).parent().unwrap()).unwrap();
            fs::write(workspace.join(output), b"video").unwrap();
            assert!(
                is_archive_workspace_output(&workspace.join(output), &dest),
                "{output} is extractor output"
            );
        }
        assert!(!is_archive_workspace_output(&workspace, &dest));
        assert!(!is_archive_workspace_output(&dest, &dest));
        assert!(!is_archive_workspace_output(
            &title.join(format!("{ARCHIVE_STAGING_PREFIX}notes.mkv")),
            &dest
        ));
        // Only the generated name, and only its output directories, count.
        for not_output in [
            "Quiet.Harbor.S02E03.mkv",
            "output/Quiet.Harbor.S02E03.mkv",
            "out-/Quiet.Harbor.S02E03.mkv",
            "nested-/Quiet.Harbor.S02E03.mkv",
        ] {
            assert!(
                !is_archive_workspace_output(&workspace.join(not_output), &dest),
                "{not_output} is not extractor output"
            );
        }
        let downloads = Path::new("/downloads");
        for lookalike in [
            format!("{ARCHIVE_STAGING_PREFIX}downloads"),
            format!("{ARCHIVE_STAGING_PREFIX}0123456789ABCDEF"),
            format!("{ARCHIVE_STAGING_PREFIX}0123456789abcde"),
            format!("{ARCHIVE_STAGING_PREFIX}0123456789abcdef0"),
        ] {
            assert!(
                !is_archive_workspace_output(
                    &downloads
                        .join(&lookalike)
                        .join("out/Quiet.Harbor.S02E03.mkv"),
                    &dest
                ),
                "{lookalike} is not an extractor workspace"
            );
        }
    }

    #[test]
    fn workspace_names_outside_the_destination_title_folder_are_not_scratch() {
        let title = Path::new("/library/Quiet Harbor (2026)");
        let dest = title.join("Season 02/Quiet Harbor - S02E03.mkv");
        let generated = format!("{ARCHIVE_STAGING_PREFIX}0123456789abcdef");
        // A download that carries the generated names is still a download.
        assert!(!is_archive_workspace_output(
            &Path::new("/downloads/Quiet.Harbor.S02")
                .join(&generated)
                .join("out/Quiet.Harbor.S02E03.mkv"),
            &dest
        ));
        // Another title's workspace is not this import's scratch.
        assert!(!is_archive_workspace_output(
            &Path::new("/library/Other Harbor (2026)")
                .join(&generated)
                .join("out/Quiet.Harbor.S02E03.mkv"),
            &dest
        ));
        assert!(!is_archive_workspace_output(
            &Path::new(&generated).join("out/Quiet.Harbor.S02E03.mkv"),
            Path::new("Quiet Harbor - S02E03.mkv")
        ));
    }

    #[tokio::test]
    async fn password_free_success_makes_exactly_one_plugin_call() {
        let run = run_password_loop(
            vec![PluginStep::Extracts],
            &operator_indexer_and_name_candidates(),
        )
        .await;

        assert!(run.result.unwrap().is_some());
        assert_eq!(run.passwords, [None]);
        assert_eq!(run.staging_dirs, 1);
    }

    #[tokio::test]
    async fn password_candidates_are_tried_in_order_until_one_is_accepted() {
        let run = run_password_loop(
            vec![
                PluginStep::Answers(ArchivePluginStatus::PasswordRequired, None, None),
                PluginStep::Answers(ArchivePluginStatus::PasswordInvalid, None, None),
                PluginStep::Extracts,
            ],
            &operator_indexer_and_name_candidates(),
        )
        .await;

        assert!(
            run.result.unwrap().is_some(),
            "the indexer password opens it"
        );
        assert_eq!(
            run.passwords,
            [
                None,
                Some("typed-guess".to_string()),
                Some("indexer-secret".to_string()),
            ]
        );
        // Refused attempts left nothing behind: only the accepted workspace
        // remains, holding only its own output.
        assert_eq!(run.staging_dirs, 1);
        assert_eq!(run.staged_files, 1);
    }

    #[tokio::test]
    async fn a_non_password_failure_on_a_candidate_stops_the_loop_and_is_reported() {
        let run = run_password_loop(
            vec![
                PluginStep::Answers(ArchivePluginStatus::PasswordRequired, None, None),
                PluginStep::Answers(
                    ArchivePluginStatus::Failed,
                    Some("archive_corrupt"),
                    Some("bad block in volume 3"),
                ),
                PluginStep::Extracts,
            ],
            &operator_indexer_and_name_candidates(),
        )
        .await;

        let error = run.result.unwrap_err();
        assert!(
            matches!(error, AppError::ArchiveExtractionFailed { .. }),
            "{error:?}"
        );
        assert!(error.to_string().contains("archive_corrupt"), "{error}");
        assert!(!is_password_required_error(&error), "{error}");
        assert_eq!(run.passwords.len(), 2);
        assert_eq!(run.staging_dirs, 0);
        assert_eq!(run.staged_files, 0);
    }

    #[tokio::test]
    async fn a_timeout_without_a_password_is_never_retried_with_candidates() {
        let run = run_password_loop(
            vec![PluginStep::TimesOut],
            &operator_indexer_and_name_candidates(),
        )
        .await;

        let error = run.result.unwrap_err();
        assert!(is_timeout_error(&error), "{error:?}");
        assert_eq!(run.passwords, [None]);
        assert_eq!(run.staging_dirs, 0);
        assert_eq!(run.staged_files, 0);
    }

    #[tokio::test]
    async fn every_candidate_refused_keeps_the_password_prompt_flow() {
        let run = run_password_loop(
            vec![
                PluginStep::Answers(ArchivePluginStatus::PasswordRequired, None, None),
                PluginStep::Answers(ArchivePluginStatus::PasswordInvalid, None, None),
                PluginStep::Answers(ArchivePluginStatus::PasswordInvalid, None, None),
                PluginStep::Answers(ArchivePluginStatus::PasswordInvalid, None, None),
            ],
            &operator_indexer_and_name_candidates(),
        )
        .await;

        let error = run.result.unwrap_err();
        assert!(is_password_required_error(&error), "{error:?}");
        let text = error.to_string();
        for secret in ["typed-guess", "indexer-secret", "name-secret"] {
            assert!(!text.contains(secret), "{text}");
        }
        assert_eq!(run.passwords.len(), 4);
        assert_eq!(run.staging_dirs, 0);
        assert_eq!(run.staged_files, 0);
    }

    #[tokio::test]
    async fn without_candidates_a_password_prompt_is_returned_after_one_call() {
        let run = run_password_loop(
            vec![PluginStep::Answers(
                ArchivePluginStatus::PasswordRequired,
                None,
                None,
            )],
            &ArchivePasswordCandidates::default(),
        )
        .await;

        let error = run.result.unwrap_err();
        assert!(is_password_required_error(&error), "{error:?}");
        assert!(error.to_string().contains("requires a password"), "{error}");
        assert_eq!(run.passwords, [None]);
        assert_eq!(run.staging_dirs, 0);
        assert_eq!(run.staged_files, 0);
    }

    #[test]
    fn has_video_files_detects_mkv() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("movie.mkv"), b"fake video").unwrap();
        assert!(has_video_files(dir.path()).unwrap());
    }

    #[test]
    fn has_video_files_ignores_non_video() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("readme.txt"), b"text").unwrap();
        fs::write(dir.path().join("archive.rar"), b"rar").unwrap();
        assert!(!has_video_files(dir.path()).unwrap());
    }

    #[test]
    fn has_video_files_recursive() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("subdir");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("episode.mp4"), b"video").unwrap();
        assert!(has_video_files(dir.path()).unwrap());
    }

    #[test]
    fn first_archive_set_prefers_rar() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("release.rar"), b"rar").unwrap();
        fs::write(dir.path().join("release.7z"), b"7z").unwrap();
        let (path, kind) = first_archive_set(dir.path()).unwrap();
        assert!(path.extension().unwrap() == "rar");
        assert!(matches!(kind, ArchiveType::Rar));
    }

    #[test]
    fn first_archive_set_finds_7z() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("release.7z"), b"7z").unwrap();
        let (_, kind) = first_archive_set(dir.path()).unwrap();
        assert!(matches!(kind, ArchiveType::SevenZip));
    }

    #[test]
    fn plan_archive_extraction_accepts_direct_archive_file_paths() {
        for (file_name, expected_type) in [
            ("release.rar", "RAR"),
            ("release.7z", "7z"),
            ("release.zip", "zip"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let archive_path = dir.path().join(file_name);
            fs::write(&archive_path, b"archive").unwrap();

            let (planned_path, archive_type) =
                plan_archive_extraction(&archive_path, is_sample_named_file)
                    .unwrap()
                    .pop()
                    .expect("direct archive file should require extraction");

            assert_eq!(planned_path, archive_path);
            assert_eq!(archive_type.as_str(), expected_type);
        }
    }

    #[test]
    fn first_archive_set_finds_zip() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("release.zip"), b"zip").unwrap();
        let (_, kind) = first_archive_set(dir.path()).unwrap();
        assert!(matches!(kind, ArchiveType::Zip));
    }

    #[test]
    fn first_archive_set_none_for_video_only() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("movie.mkv"), b"video").unwrap();
        assert!(first_archive_set(dir.path()).is_none());
    }

    #[test]
    fn first_archive_set_finds_a_set_in_a_subdirectory() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("Lantern.Vale.2019.1080p.BluRay-NOGRP");
        fs::create_dir(&nested).unwrap();
        fs::write(nested.join("lantern.vale.part02.rar"), b"rar").unwrap();
        fs::write(nested.join("lantern.vale.part01.rar"), b"rar").unwrap();
        fs::write(dir.path().join("lantern.vale.nfo"), b"nfo").unwrap();

        let (path, kind) = first_archive_set(dir.path()).unwrap();

        assert!(matches!(kind, ArchiveType::Rar));
        assert_eq!(path, nested.join("lantern.vale.part01.rar"));
    }

    #[test]
    fn first_archive_set_prefers_the_top_level_over_subdirectories() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("Extras");
        fs::create_dir(&nested).unwrap();
        fs::write(nested.join("extras.rar"), b"rar").unwrap();
        fs::write(dir.path().join("lantern.vale.zip"), b"zip").unwrap();

        let (path, kind) = first_archive_set(dir.path()).unwrap();

        assert!(matches!(kind, ArchiveType::Zip));
        assert_eq!(path, dir.path().join("lantern.vale.zip"));
    }

    #[test]
    fn first_archive_set_searches_subdirectories_in_name_order() {
        let dir = tempfile::tempdir().unwrap();
        for (folder, archive) in [("CD2", "vale.cd2.rar"), ("CD1", "vale.cd1.rar")] {
            let nested = dir.path().join(folder);
            fs::create_dir(&nested).unwrap();
            fs::write(nested.join(archive), b"rar").unwrap();
        }

        let (path, _) = first_archive_set(dir.path()).unwrap();

        assert_eq!(path, dir.path().join("CD1").join("vale.cd1.rar"));
    }

    #[test]
    fn first_archive_set_skips_sample_hidden_and_staging_directories() {
        let dir = tempfile::tempdir().unwrap();
        for folder in [
            "Sample",
            "samples",
            ".hidden",
            EXTRACTED_DIR_NAME,
            &format!("{ARCHIVE_STAGING_PREFIX}0123456789abcdef"),
        ] {
            let nested = dir.path().join(folder);
            fs::create_dir(&nested).unwrap();
            fs::write(nested.join("vale.rar"), b"rar").unwrap();
        }

        assert!(first_archive_set(dir.path()).is_none());
    }

    #[test]
    fn first_archive_set_stops_at_the_depth_bound() {
        let dir = tempfile::tempdir().unwrap();
        let mut within = dir.path().to_path_buf();
        for level in 0..MAX_ARCHIVE_DISCOVERY_DEPTH {
            within = within.join(format!("level{level}"));
        }
        let beyond = within.join("too-deep");
        fs::create_dir_all(&beyond).unwrap();
        fs::write(beyond.join("vale.rar"), b"rar").unwrap();

        assert!(
            find_archive_sets(dir.path())
                .unwrap_err()
                .to_string()
                .contains("depth limit")
        );

        fs::write(within.join("vale.7z"), b"7z").unwrap();
        assert!(
            find_archive_sets(dir.path()).is_err(),
            "never return an incomplete inventory"
        );
    }

    #[test]
    fn archive_sets_in_dir_collapses_rar_volumes_onto_each_first_volume() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "vale.part02.rar",
            "vale.part01.rar",
            "harbor.rar",
            "harbor.r00",
            "extras.7z",
        ] {
            fs::write(dir.path().join(name), b"archive").unwrap();
        }

        let sets = archive_sets_in_dir(dir.path());

        let names: Vec<_> = sets
            .iter()
            .map(|(path, _)| path.file_name().unwrap().to_str().unwrap())
            .collect();
        assert_eq!(names, ["harbor.rar", "vale.part01.rar", "extras.7z"]);
        assert!(
            sets[..2]
                .iter()
                .all(|(_, kind)| matches!(kind, ArchiveType::Rar))
        );
    }

    /// A scene-style release folder: an old-style RAR set, its nfo/sfv, and a
    /// `Sample/` directory holding the only loose video, named as a sample.
    fn scene_rar_release_with_sample(root: &Path) -> PathBuf {
        let release = root.join("Quiet.Harbor.S06E07.1080p.WEB.H264-NOGRP");
        fs::create_dir(&release).unwrap();
        let stem = "quiet.harbor.s06e07.1080p.web.h264-nogrp";
        fs::write(release.join(format!("{stem}.rar")), b"rar").unwrap();
        for volume in 0..=8 {
            fs::write(release.join(format!("{stem}.r{volume:02}")), b"rar").unwrap();
        }
        fs::write(release.join(format!("{stem}.nfo")), b"nfo").unwrap();
        fs::write(release.join(format!("{stem}.sfv")), b"sfv").unwrap();
        let sample = release.join("Sample");
        fs::create_dir(&sample).unwrap();
        fs::write(sample.join(format!("{stem}-sample.mkv")), b"sample video").unwrap();
        release
    }

    /// A sparse video comfortably above the series sample-size threshold.
    fn write_full_size_video(path: &Path) {
        fs::File::create(path)
            .unwrap()
            .set_len(64 * 1024 * 1024)
            .unwrap();
    }

    #[test]
    fn sample_only_series_release_still_plans_extraction() {
        let root = tempfile::tempdir().unwrap();
        let release = scene_rar_release_with_sample(root.path());

        let (archive, kind) = plan_archive_extraction(&release, is_sample_file)
            .unwrap()
            .pop()
            .expect("a series release whose only video is its sample needs its archives");

        assert!(matches!(kind, ArchiveType::Rar));
        assert_eq!(
            archive.file_name().and_then(|name| name.to_str()),
            Some("quiet.harbor.s06e07.1080p.web.h264-nogrp.rar")
        );
        assert!(archive_extraction_would_be_needed(&release, is_sample_file).unwrap());
    }

    #[test]
    fn sample_only_movie_release_still_plans_extraction() {
        let root = tempfile::tempdir().unwrap();
        let release = scene_rar_release_with_sample(root.path());

        assert!(archive_extraction_would_be_needed(&release, is_sample_named_file).unwrap());
    }

    #[test]
    fn real_video_beside_archives_does_not_hide_other_episodes() {
        let root = tempfile::tempdir().unwrap();
        let release = scene_rar_release_with_sample(root.path());
        write_full_size_video(&release.join("quiet.harbor.s06e07.1080p.web.h264-nogrp.mkv"));

        assert!(archive_extraction_would_be_needed(&release, is_sample_file).unwrap());
        assert!(archive_extraction_would_be_needed(&release, is_sample_named_file).unwrap());
    }

    #[test]
    fn small_unnamed_movie_beside_archives_is_not_treated_as_a_sample() {
        let root = tempfile::tempdir().unwrap();
        let release = root.path().join("Tiny.Reel.1931.480p");
        fs::create_dir(&release).unwrap();
        fs::write(release.join("tiny.reel.1931.480p.rar"), b"rar").unwrap();
        fs::write(release.join("tiny.reel.1931.480p.mkv"), b"short film").unwrap();

        assert!(archive_extraction_would_be_needed(&release, is_sample_named_file).unwrap());
    }

    #[test]
    fn sample_only_release_without_archives_plans_nothing() {
        let root = tempfile::tempdir().unwrap();
        let release = root.path().join("Quiet.Harbor.S06E07.1080p.WEB.H264-NOGRP");
        let sample = release.join("Sample");
        fs::create_dir_all(&sample).unwrap();
        fs::write(sample.join("quiet.harbor.s06e07-sample.mkv"), b"sample").unwrap();

        assert!(
            plan_archive_extraction(&release, is_sample_file)
                .unwrap()
                .is_empty()
        );
        assert!(
            plan_archive_extraction(&release, is_sample_named_file)
                .unwrap()
                .is_empty()
        );
    }

    /// The import scans the staging workspace it gets back, never the torrent
    /// folder, so the sample left beside the archives cannot become the movie.
    #[tokio::test]
    async fn sample_only_release_extracts_and_the_scan_sees_only_extracted_video() {
        let root = tempfile::tempdir().unwrap();
        let release = scene_rar_release_with_sample(root.path());
        let destination = tempfile::tempdir().unwrap();
        let provider = scripted_provider(ScriptedArchiveClient {
            emitted: vec![("quiet.harbor.s06e07.1080p.web.h264-nogrp.mkv", b"video")],
            status: ArchivePluginStatus::Ok,
            error_code: None,
            message: None,
            copied_bytes: None,
            replaced_sources: Vec::new(),
        });

        let extracted = extract_archives_if_needed(
            &release,
            is_sample_named_file,
            Some(ArchiveExtractionDestination::new(
                destination.path(),
                "sample-only-release",
            )),
            &ArchivePasswordCandidates::default(),
            Some(provider),
        )
        .await
        .unwrap()
        .expect("the archives must be extracted");

        let scanned = crate::import::workflow::find_video_files(&extracted, false).unwrap();
        assert_eq!(scanned.len(), 1, "{scanned:?}");
        assert_eq!(
            scanned[0].file_name().and_then(|name| name.to_str()),
            Some("quiet.harbor.s06e07.1080p.web.h264-nogrp.mkv")
        );
        assert!(scanned[0].starts_with(destination.path()));
        assert!(
            release
                .join("Sample")
                .join("quiet.harbor.s06e07.1080p.web.h264-nogrp-sample.mkv")
                .exists()
        );
    }

    #[test]
    fn archive_output_path_rejects_traversal() {
        let dir = tempfile::tempdir().unwrap();
        assert!(safe_archive_output_path(dir.path(), "../movie.mkv").is_err());
        assert!(safe_archive_output_path(dir.path(), "/tmp/movie.mkv").is_err());
        assert!(safe_archive_output_path(dir.path(), r"nested\movie.mkv").is_err());
    }

    #[test]
    fn archive_output_path_allows_nested_relative_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = safe_archive_output_path(dir.path(), "Season 1/movie.mkv").unwrap();
        assert_eq!(path, dir.path().join("Season 1").join("movie.mkv"));
    }

    #[tokio::test]
    async fn loose_video_does_not_bypass_archive_destination_requirements() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("movie.mkv"), b"video").unwrap();
        fs::write(dir.path().join("archive.rar"), b"rar").unwrap();
        let result = extract_archives_if_needed(
            dir.path(),
            is_sample_named_file,
            None,
            &ArchivePasswordCandidates::default(),
            None,
        )
        .await
        .unwrap_err();
        assert!(result.to_string().contains("resolved import destination"));
    }

    #[tokio::test]
    async fn rar_archive_requires_archive_plugin() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("archive.rar"), b"rar").unwrap();
        let destination = tempfile::tempdir().unwrap();

        let err = extract_archives_if_needed(
            dir.path(),
            is_sample_named_file,
            Some(ArchiveExtractionDestination::new(
                destination.path(),
                "rar-plugin-required",
            )),
            &ArchivePasswordCandidates::default(),
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err,
            AppError::ArchiveExtractionPluginRequired { .. }
        ));
    }

    #[tokio::test]
    async fn sevenz_archive_requires_archive_plugin() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("archive.7z"), b"7z").unwrap();
        let destination = tempfile::tempdir().unwrap();

        let err = extract_archives_if_needed(
            dir.path(),
            is_sample_named_file,
            Some(ArchiveExtractionDestination::new(
                destination.path(),
                "7z-plugin-required",
            )),
            &ArchivePasswordCandidates::default(),
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err,
            AppError::ArchiveExtractionPluginRequired { .. }
        ));
    }

    #[tokio::test]
    async fn sevenz_archive_requires_plugin_update_when_provider_lacks_format() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("archive.7z"), b"7z").unwrap();
        let destination = tempfile::tempdir().unwrap();
        let operation = Arc::new(Mutex::new(None));
        let client: Arc<dyn ArchiveExtractorClient> = Arc::new(RecordingArchiveClient {
            operation: Arc::clone(&operation),
            write_output_file: false,
        });
        let provider: Arc<dyn ArchiveExtractorPluginProvider> =
            Arc::new(RecordingArchiveProvider {
                client,
                formats: vec![ArchivePluginFormat::Rar, ArchivePluginFormat::Zip],
            });

        let err = extract_archives_if_needed(
            dir.path(),
            is_sample_named_file,
            Some(ArchiveExtractionDestination::new(
                destination.path(),
                "7z-provider-missing-format",
            )),
            &ArchivePasswordCandidates::default(),
            Some(provider),
        )
        .await
        .unwrap_err();

        assert!(matches!(
            err,
            AppError::ArchiveExtractionPluginRequired { .. }
        ));
        assert!(operation.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn sevenz_uses_archive_plugin_extract() {
        let dir = tempfile::tempdir().unwrap();
        let archive_path = dir.path().join("release.7z");
        fs::write(&archive_path, b"7z").unwrap();
        let destination = tempfile::tempdir().unwrap();
        let operation = Arc::new(Mutex::new(None));
        let client: Arc<dyn ArchiveExtractorClient> = Arc::new(RecordingArchiveClient {
            operation: Arc::clone(&operation),
            write_output_file: false,
        });
        let provider: Arc<dyn ArchiveExtractorPluginProvider> =
            Arc::new(RecordingArchiveProvider {
                client,
                formats: vec![
                    ArchivePluginFormat::Rar,
                    ArchivePluginFormat::SevenZip,
                    ArchivePluginFormat::Zip,
                ],
            });

        let result = extract_archives_if_needed(
            &archive_path,
            is_sample_named_file,
            Some(ArchiveExtractionDestination::new(
                destination.path(),
                "7z-plain-extract",
            )),
            &ArchivePasswordCandidates::default(),
            Some(provider),
        )
        .await
        .unwrap();

        assert!(result.is_none());
        let recorded = operation.lock().unwrap().clone().unwrap();
        match recorded {
            ArchivePluginOperation::ExtractArchive {
                archive_path: recorded_archive,
                output_dir,
                format,
                ..
            } => {
                assert_eq!(recorded_archive, archive_path.to_string_lossy());
                assert_eq!(format, ArchivePluginFormat::SevenZip);
                let output_dir = PathBuf::from(output_dir);
                assert!(output_dir.starts_with(destination.path()));
                assert_eq!(
                    output_dir.file_name().and_then(|name| name.to_str()),
                    Some(ARCHIVE_STAGING_OUTPUT_DIR)
                );
            }
            other => panic!("expected extract operation, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn zip_uses_archive_plugin_extract() {
        let dir = tempfile::tempdir().unwrap();
        let archive_path = dir.path().join("release.zip");
        fs::write(&archive_path, b"zip").unwrap();

        let operation = Arc::new(Mutex::new(None));
        let client: Arc<dyn ArchiveExtractorClient> = Arc::new(RecordingArchiveClient {
            operation: Arc::clone(&operation),
            write_output_file: false,
        });
        let destination = tempfile::tempdir().unwrap();
        let provider: Arc<dyn ArchiveExtractorPluginProvider> =
            Arc::new(RecordingArchiveProvider {
                client,
                formats: vec![ArchivePluginFormat::Rar, ArchivePluginFormat::Zip],
            });

        let result = extract_archives_if_needed(
            dir.path(),
            is_sample_named_file,
            Some(ArchiveExtractionDestination::new(
                destination.path(),
                "zip-plain-extract",
            )),
            &ArchivePasswordCandidates::default(),
            Some(provider),
        )
        .await
        .unwrap();

        assert!(result.is_none());
        let recorded = operation.lock().unwrap().clone().unwrap();
        assert!(matches!(
            recorded,
            ArchivePluginOperation::ExtractArchive {
                format: ArchivePluginFormat::Zip,
                ..
            }
        ));
        if let ArchivePluginOperation::ExtractArchive {
            archive_path: recorded_archive,
            output_dir,
            ..
        } = recorded
        {
            assert_eq!(recorded_archive, archive_path.to_string_lossy());
            let output_dir = PathBuf::from(output_dir);
            assert!(output_dir.starts_with(destination.path()));
            assert_eq!(
                output_dir.file_name().and_then(|name| name.to_str()),
                Some(ARCHIVE_STAGING_OUTPUT_DIR)
            );
        }
    }

    #[tokio::test]
    async fn archive_extraction_requires_destination_for_archived_download() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("release.rar"), b"rar").unwrap();

        let error = extract_archives_if_needed(
            dir.path(),
            is_sample_named_file,
            None,
            &ArchivePasswordCandidates::default(),
            None,
        )
        .await
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("requires a resolved import destination")
        );
    }

    #[tokio::test]
    async fn rar_uses_archive_plugin_extract() {
        let source = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        let archive_path = source.path().join("archive.rar");
        fs::write(&archive_path, b"rar").unwrap();

        let operation = Arc::new(Mutex::new(None));
        let client: Arc<dyn ArchiveExtractorClient> = Arc::new(RecordingArchiveClient {
            operation: Arc::clone(&operation),
            write_output_file: true,
        });
        let provider: Arc<dyn ArchiveExtractorPluginProvider> =
            Arc::new(RecordingArchiveProvider {
                client,
                formats: vec![ArchivePluginFormat::Rar, ArchivePluginFormat::Zip],
            });

        let extracted = extract_archives_if_needed(
            &archive_path,
            is_sample_named_file,
            Some(ArchiveExtractionDestination::new(
                destination.path(),
                "import/with spaces",
            )),
            &operator_password("secret"),
            Some(provider),
        )
        .await
        .unwrap()
        .unwrap();

        assert!(extracted.starts_with(destination.path()));
        assert!(!extracted.starts_with(source.path()));
        let staging_name = extracted
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap();
        assert!(staging_name.starts_with(ARCHIVE_STAGING_PREFIX));
        assert!(staging_name.starts_with('.'));
        assert_eq!(staging_name.len(), ARCHIVE_STAGING_PREFIX.len() + 16);
        assert!(
            extracted
                .join(ARCHIVE_STAGING_OUTPUT_DIR)
                .join("movie.mkv")
                .exists()
        );

        let recorded = operation.lock().unwrap().clone().unwrap();
        match recorded {
            ArchivePluginOperation::ExtractArchive {
                output_dir,
                format,
                archive_path: recorded_archive,
                password,
            } => {
                assert_eq!(
                    output_dir,
                    extracted.join(ARCHIVE_STAGING_OUTPUT_DIR).to_string_lossy()
                );
                assert_eq!(format, ArchivePluginFormat::Rar);
                assert_eq!(recorded_archive, archive_path.to_string_lossy());
                // The archive opened without one, so the operator password
                // was never sent.
                assert_eq!(password, None);
            }
            other => panic!("expected extract operation, got {other:?}"),
        }

        cleanup_extracted_dir(&extracted).await;
        assert!(!extracted.exists());
    }

    #[tokio::test]
    async fn cleanup_preserves_legacy_unmarked_extracted_dir() {
        let dir = tempfile::tempdir().unwrap();
        let extracted = dir.path().join(EXTRACTED_DIR_NAME);
        fs::create_dir(&extracted).unwrap();
        fs::write(extracted.join("file.txt"), b"data").unwrap();

        cleanup_extracted_dir(&extracted).await;
        assert!(extracted.exists());
        // Parent still exists
        assert!(dir.path().exists());
    }

    #[tokio::test]
    async fn stale_cleanup_preserves_unowned_paths_regardless_of_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let archive_dir = dir.path().join(format!("{ARCHIVE_STAGING_PREFIX}orphan"));
        let legacy_dir = dir
            .path()
            .join(format!("{LEGACY_ARCHIVE_STAGING_PREFIX}orphan"));
        let probe_file = dir
            .path()
            .join(format!("{ARCHIVE_WRITE_PROBE_PREFIX}leaked"));
        let keep_dir = dir.path().join("Movie (2026)");
        let keep_file = dir.path().join("release.nfo");
        fs::create_dir(&archive_dir).unwrap();
        fs::create_dir(&legacy_dir).unwrap();
        fs::create_dir(&keep_dir).unwrap();
        fs::write(&probe_file, b"probe").unwrap();
        fs::write(&keep_file, b"nfo").unwrap();

        cleanup_archive_artifacts_older_than(dir.path(), Duration::ZERO).await;

        assert!(archive_dir.exists());
        assert!(legacy_dir.exists());
        assert!(probe_file.exists());
        assert!(keep_dir.exists());
        assert!(keep_file.exists());
    }

    #[tokio::test]
    async fn cleanup_removes_archive_staging_dir() {
        let dir = tempfile::tempdir().unwrap();
        let extracted = ArchiveExtractionWorkspace::create(&ArchiveExtractionDestination::new(
            dir.path(),
            "test-import",
        ))
        .await
        .unwrap()
        .root;
        fs::write(extracted.join("file.txt"), b"data").unwrap();

        cleanup_extracted_dir(&extracted).await;
        assert!(!extracted.exists());
        assert!(dir.path().exists());
    }

    #[tokio::test]
    async fn cleanup_refuses_non_extracted_dir() {
        let dir = tempfile::tempdir().unwrap();
        let other = dir.path().join("important_data");
        fs::create_dir(&other).unwrap();
        fs::write(other.join("file.txt"), b"data").unwrap();

        cleanup_extracted_dir(&other).await;
        // Should NOT be deleted: name matches neither legacy nor staging dirs.
        assert!(other.exists());
    }

    #[tokio::test]
    async fn stale_cleanup_requires_ownership_and_reference_release() {
        let parent = tempfile::tempdir().unwrap();
        let active = ArchiveExtractionWorkspace::create(&ArchiveExtractionDestination::new(
            parent.path(),
            "active-selection",
        ))
        .await
        .unwrap();
        let released = ArchiveExtractionWorkspace::create(&ArchiveExtractionDestination::new(
            parent.path(),
            "finished-import",
        ))
        .await
        .unwrap();
        fs::write(active.output_dir.join("episode.mkv"), b"keep").unwrap();
        release_archive_workspace(&released.root).unwrap();
        cleanup_archive_artifacts_older_than(parent.path(), Duration::ZERO).await;
        assert_eq!(
            fs::read(active.output_dir.join("episode.mkv")).unwrap(),
            b"keep"
        );
        assert!(!released.root.exists());
    }

    #[cfg(unix)]
    #[test]
    fn archive_workspace_creation_is_private_to_the_service() {
        use std::os::unix::fs::PermissionsExt;
        let parent = tempfile::tempdir().unwrap();
        let root = create_test_archive_workspace(parent.path());
        assert_eq!(
            fs::metadata(root).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }

    #[cfg(unix)]
    #[test]
    fn archive_workspace_handle_authentication_refuses_a_restored_path_with_foreign_handle() {
        let parent = tempfile::tempdir().unwrap();
        let root = create_test_archive_workspace(parent.path());
        let owned = fs::File::open(&root).unwrap();
        let copied_parent = tempfile::tempdir().unwrap();
        let copied = copied_parent.path().join(root.file_name().unwrap());
        fs::create_dir(&copied).unwrap();
        fs::copy(
            root.join(ARCHIVE_WORKSPACE_OWNER_FILE),
            copied.join(ARCHIVE_WORKSPACE_OWNER_FILE),
        )
        .unwrap();
        let foreign = fs::File::open(&copied).unwrap();
        assert!(is_owned_archive_workspace_handle(&root, &owned));
        // A caller that opened a swapped directory cannot authenticate it by
        // restoring the legitimate pathname before verification.
        assert!(!is_owned_archive_workspace_handle(&root, &foreign));
    }

    #[test]
    fn archive_workspace_authority_key_survives_restart_reads() {
        let state = tempfile::tempdir().unwrap();
        let first = load_or_create_archive_workspace_key(state.path()).unwrap();
        let restarted = load_or_create_archive_workspace_key(state.path()).unwrap();
        assert_eq!(first, restarted);
        assert_eq!(
            fs::metadata(state.path().join("archive-workspace-key"))
                .unwrap()
                .len(),
            32
        );
        assert_eq!(
            blake3::keyed_hash(&first, b"persisted workspace proof"),
            blake3::keyed_hash(&restarted, b"persisted workspace proof")
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(state.path().join("archive-workspace-key"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn archive_workspace_authority_refuses_invalid_existing_key() {
        let state = tempfile::tempdir().unwrap();
        fs::write(state.path().join("archive-workspace-key"), b"invalid").unwrap();
        assert!(load_or_create_archive_workspace_key(state.path()).is_err());
        assert_eq!(
            fs::read(state.path().join("archive-workspace-key")).unwrap(),
            b"invalid"
        );
    }

    #[cfg(unix)]
    #[test]
    fn archive_workspace_authority_refuses_symlink_key() {
        let state = tempfile::tempdir().unwrap();
        let unrelated = tempfile::tempdir().unwrap();
        let protected = unrelated.path().join("preserve");
        fs::write(&protected, [7; 32]).unwrap();
        std::os::unix::fs::symlink(&protected, state.path().join("archive-workspace-key")).unwrap();
        assert!(load_or_create_archive_workspace_key(state.path()).is_err());
        assert_eq!(fs::read(protected).unwrap(), vec![7; 32]);
    }

    #[tokio::test]
    async fn forged_archive_ownership_and_release_markers_preserve_user_files() {
        let parent = tempfile::tempdir().unwrap();
        for (schema, suffix) in [(1, "1111111111111111"), (2, "2222222222222222")] {
            let root = parent
                .path()
                .join(format!("{ARCHIVE_STAGING_PREFIX}{suffix}"));
            fs::create_dir(&root).unwrap();
            fs::write(root.join("download.mkv"), b"preserve download").unwrap();
            fs::write(root.join(ARCHIVE_WORKSPACE_OWNER_FILE), serde_json::to_vec(&serde_json::json!({
                "schema": schema, "workspace": root.file_name().unwrap().to_string_lossy(), "import_id": "forged-import", "proof": "00".repeat(32)
            })).unwrap()).unwrap();
            fs::write(root.join(ARCHIVE_WORKSPACE_RELEASED_FILE), serde_json::to_vec(&serde_json::json!({
                "schema": 2, "owner_proof": "00".repeat(32), "released_at": 0, "proof": "00".repeat(32)
            })).unwrap()).unwrap();
            assert!(!is_owned_archive_workspace(&root));
            cleanup_extracted_dir(&root).await;
        }
        cleanup_archive_artifacts_older_than(parent.path(), Duration::ZERO).await;
        for suffix in ["1111111111111111", "2222222222222222"] {
            assert_eq!(
                fs::read(
                    parent
                        .path()
                        .join(format!("{ARCHIVE_STAGING_PREFIX}{suffix}"))
                        .join("download.mkv")
                )
                .unwrap(),
                b"preserve download"
            );
        }
    }

    #[tokio::test]
    async fn authenticated_archive_markers_cannot_be_transplanted_to_another_directory() {
        let original_parent = tempfile::tempdir().unwrap();
        let root = create_test_archive_workspace(original_parent.path());
        release_archive_workspace(&root).unwrap();
        let copied_parent = tempfile::tempdir().unwrap();
        let copied = copied_parent.path().join(root.file_name().unwrap());
        fs::create_dir(&copied).unwrap();
        for marker in [
            ARCHIVE_WORKSPACE_OWNER_FILE,
            ARCHIVE_WORKSPACE_RELEASED_FILE,
        ] {
            fs::copy(root.join(marker), copied.join(marker)).unwrap();
        }
        fs::write(copied.join("download.mkv"), b"keep copied download").unwrap();
        assert!(is_owned_archive_workspace(&root));
        assert!(!is_owned_archive_workspace(&copied));
        cleanup_extracted_dir(&copied).await;
        cleanup_archive_artifacts_older_than(copied_parent.path(), Duration::ZERO).await;
        assert_eq!(
            fs::read(copied.join("download.mkv")).unwrap(),
            b"keep copied download"
        );
    }

    #[tokio::test]
    async fn forged_release_does_not_expire_an_active_owned_workspace() {
        let parent = tempfile::tempdir().unwrap();
        let root = create_test_archive_workspace(parent.path());
        fs::write(root.join("out/episode.mkv"), b"active media").unwrap();
        let owner = archive_workspace_owner(&root).unwrap();
        fs::write(
            root.join(ARCHIVE_WORKSPACE_RELEASED_FILE),
            serde_json::to_vec(&serde_json::json!({
                "schema": 2, "owner_proof": owner.proof, "released_at": 0, "proof": "00".repeat(32)
            }))
            .unwrap(),
        )
        .unwrap();
        cleanup_archive_artifacts_older_than(parent.path(), Duration::ZERO).await;
        cleanup_extracted_dir(&root).await;
        assert_eq!(
            fs::read(root.join("out/episode.mkv")).unwrap(),
            b"active media"
        );
    }

    #[tokio::test]
    async fn dropping_unpublished_archive_lease_releases_only_its_owned_workspace() {
        let parent = tempfile::tempdir().unwrap();
        let unrelated = parent.path().join("completed-download");
        fs::create_dir(&unrelated).unwrap();
        fs::write(unrelated.join("source.mkv"), b"preserve source").unwrap();
        let workspace = ArchiveExtractionWorkspace::create(&ArchiveExtractionDestination::new(
            parent.path(),
            "cancelled-import",
        ))
        .await
        .unwrap();
        let root = workspace.root.clone();
        assert!(archive_workspace_release(&root).is_none());
        drop(workspace);
        assert!(archive_workspace_release(&root).is_some());
        cleanup_archive_artifacts_older_than(parent.path(), Duration::ZERO).await;
        assert!(!root.exists());
        assert_eq!(
            fs::read(unrelated.join("source.mkv")).unwrap(),
            b"preserve source"
        );
    }

    #[tokio::test]
    async fn dropping_published_archive_workspace_keeps_its_reference_active() {
        let parent = tempfile::tempdir().unwrap();
        let root = create_test_archive_workspace(parent.path());
        fs::write(root.join("out/episode.mkv"), b"selected media").unwrap();
        assert!(archive_workspace_release(&root).is_none());
        cleanup_archive_artifacts_older_than(parent.path(), Duration::ZERO).await;
        assert_eq!(
            fs::read(root.join("out/episode.mkv")).unwrap(),
            b"selected media"
        );
    }

    struct PendingArchiveClient {
        started: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<PathBuf>>>,
    }

    #[async_trait::async_trait]
    impl ArchiveExtractorClient for PendingArchiveClient {
        async fn process(
            &self,
            request: ArchivePluginProcessRequest,
        ) -> AppResult<ArchivePluginProcessResponse> {
            let ArchivePluginOperation::ExtractArchive { output_dir, .. } = request.operation
            else {
                panic!("expected extraction");
            };
            let output = PathBuf::from(output_dir);
            fs::write(output.join("partial.mkv"), b"partial output").unwrap();
            self.started
                .lock()
                .unwrap()
                .take()
                .unwrap()
                .send(output.parent().unwrap().to_path_buf())
                .unwrap();
            std::future::pending().await
        }
    }

    #[tokio::test]
    async fn cancelled_archive_extraction_releases_unpublished_workspace() {
        let source = tempfile::tempdir().unwrap();
        fs::write(source.path().join("episode.zip"), b"source archive").unwrap();
        let destination = tempfile::tempdir().unwrap();
        let (started, ready) = tokio::sync::oneshot::channel();
        let provider: Arc<dyn ArchiveExtractorPluginProvider> =
            Arc::new(RecordingArchiveProvider {
                client: Arc::new(PendingArchiveClient {
                    started: std::sync::Mutex::new(Some(started)),
                }),
                formats: vec![ArchivePluginFormat::Zip],
            });
        let source_path = source.path().to_path_buf();
        let destination_path = destination.path().to_path_buf();
        let task = tokio::spawn(async move {
            extract_archives_if_needed(
                &source_path,
                |_| false,
                Some(ArchiveExtractionDestination::new(
                    destination_path,
                    "cancelled-job",
                )),
                &ArchivePasswordCandidates::default(),
                Some(provider),
            )
            .await
        });
        let root = tokio::time::timeout(Duration::from_secs(60), ready)
            .await
            .unwrap()
            .unwrap();
        task.abort();
        assert!(
            tokio::time::timeout(Duration::from_secs(60), task)
                .await
                .unwrap()
                .unwrap_err()
                .is_cancelled()
        );
        assert!(archive_workspace_release(&root).is_some());
        cleanup_archive_artifacts_older_than(destination.path(), Duration::ZERO).await;
        assert!(!root.exists());
        assert_eq!(
            fs::read(source.path().join("episode.zip")).unwrap(),
            b"source archive"
        );
    }

    #[cfg(unix)]
    #[test]
    fn discovery_does_not_follow_directory_symlinks() {
        let source = tempfile::tempdir().unwrap();
        let unrelated = tempfile::tempdir().unwrap();
        fs::write(unrelated.path().join("episode.rar"), b"archive").unwrap();
        std::os::unix::fs::symlink(unrelated.path(), source.path().join("other")).unwrap();
        assert!(find_archive_sets(source.path()).unwrap().is_empty());
        assert!(has_video_files(source.path()).is_ok());
    }

    #[test]
    fn media_xz_is_discovered_beside_other_archive_formats() {
        let source = tempfile::tempdir().unwrap();
        for name in ["a.rar", "b.7z", "c.zip", "d.mkv.xz"] {
            fs::write(source.path().join(name), b"archive").unwrap();
        }
        let sets = find_archive_sets(source.path()).unwrap();
        assert_eq!(sets.len(), 4);
        assert!(matches!(sets[3].1, ArchiveType::Xz));
    }

    /// PAR2 repair moved inside the plugin. When the recovery set protects plain
    /// media files rather than an archive, the plugin repairs them into its
    /// output directory and reports them in `files` with `copied_bytes` set
    /// instead of `expanded_bytes`. The host must accept those exactly like
    /// extracted archive members.
    #[tokio::test]
    async fn plugin_emitted_plain_files_are_the_deliverable() {
        let source = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        fs::create_dir(source.path().join("Season 1")).unwrap();
        fs::write(source.path().join("abc.mkv"), b"damaged video").unwrap();
        fs::write(source.path().join("release.par2"), b"par2").unwrap();

        let provider = scripted_provider(ScriptedArchiveClient {
            emitted: vec![
                ("Season 1/episode.mkv", b"repaired video"),
                ("release.nfo", b"nfo"),
            ],
            status: ArchivePluginStatus::Ok,
            error_code: None,
            message: None,
            copied_bytes: Some(17),
            replaced_sources: vec!["abc.mkv".into()],
        });

        let extracted = extract_archives_if_needed(
            source.path(),
            is_sample_named_file,
            Some(ArchiveExtractionDestination::new(
                destination.path(),
                "par2-plain-files",
            )),
            &ArchivePasswordCandidates::default(),
            Some(provider),
        )
        .await
        .unwrap()
        .expect("plugin-emitted plain files should be accepted as extraction output");

        assert!(extracted.starts_with(destination.path()));
        assert!(
            extracted
                .join(ARCHIVE_STAGING_OUTPUT_DIR)
                .join("Season 1")
                .join("episode.mkv")
                .exists()
        );
        // The read-only source is untouched by the host.
        assert_eq!(
            fs::read(source.path().join("abc.mkv")).unwrap(),
            b"damaged video"
        );
        assert!(
            replaced_archive_sources(&extracted)
                .unwrap()
                .contains(&source.path().join("abc.mkv").canonicalize().unwrap())
        );
        assert!(source.path().join("release.par2").exists());
        let unrelated = source.path().join("unrelated.mkv");
        fs::write(&unrelated, b"unrelated").unwrap();
        fs::write(
            extracted
                .join(ARCHIVE_STAGING_OUTPUT_DIR)
                .join(".source-replacements-out.json"),
            serde_json::to_vec(&vec![unrelated.canonicalize().unwrap()]).unwrap(),
        )
        .unwrap();
        assert!(
            !replaced_archive_sources(&extracted)
                .unwrap()
                .contains(&unrelated.canonicalize().unwrap())
        );
    }

    /// `par2_insufficient_recovery` used to be a native validation error. It now
    /// arrives as a plugin `Failed` status and must keep both its code and its
    /// permanent (validation) classification.
    #[tokio::test]
    async fn insufficient_par2_recovery_surfaces_as_validation_error() {
        let source = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        fs::write(source.path().join("release.rar"), b"rar").unwrap();

        let provider = scripted_provider(ScriptedArchiveClient {
            emitted: Vec::new(),
            status: ArchivePluginStatus::Failed,
            error_code: Some("par2_insufficient_recovery"),
            message: Some("recovery set cannot reconstruct 3 damaged blocks"),
            copied_bytes: None,
            replaced_sources: Vec::new(),
        });

        let error = extract_archives_if_needed(
            source.path(),
            is_sample_named_file,
            Some(ArchiveExtractionDestination::new(
                destination.path(),
                "par2-insufficient",
            )),
            &ArchivePasswordCandidates::default(),
            Some(provider),
        )
        .await
        .unwrap_err();

        assert!(
            matches!(error, AppError::ArchiveExtractionFailed { .. }),
            "{error:?}"
        );
        let text = error.to_string();
        assert!(text.contains("par2_insufficient_recovery"), "{text}");
        assert!(text.contains("3 damaged blocks"), "{text}");
    }

    #[test]
    fn archive_plugin_failure_keeps_the_error_code_alongside_the_message() {
        let error = archive_plugin_failure_error(Some("par2_scan_failed"), Some("bad header"));
        assert!(matches!(error, AppError::Repository(_)), "{error:?}");
        let text = error.to_string();
        assert!(text.contains("par2_scan_failed"), "{text}");
        assert!(text.contains("bad header"), "{text}");

        assert!(
            archive_plugin_failure_error(None, None)
                .to_string()
                .contains("archive plugin extraction failed")
        );
        assert!(
            archive_plugin_failure_error(Some("only_code"), None)
                .to_string()
                .contains("only_code")
        );
    }
    /// How the scripted plugin treats one archive, keyed by file name.
    enum TreeStep {
        /// Extracts the given members.
        Emits(Vec<(&'static str, &'static [u8])>),
        EmitsHandled(Vec<(&'static str, &'static [u8])>, Vec<&'static str>),
        /// Fails without a password prompt.
        Fails,
        /// Refuses every password but this one, then extracts a video.
        Locked(&'static str),
    }

    type TreeArchiveCall = (String, PathBuf, Option<String>);

    /// Answers extraction calls per archive name, like a torrent holding
    /// several archive sets and archives packed inside archives. Every call
    /// first writes a partial member, so leftovers of refused attempts show.
    struct TreeArchiveClient {
        script: std::collections::HashMap<&'static str, TreeStep>,
        calls: Arc<Mutex<Vec<TreeArchiveCall>>>,
    }

    #[async_trait::async_trait]
    impl ArchiveExtractorClient for TreeArchiveClient {
        async fn process(
            &self,
            request: ArchivePluginProcessRequest,
        ) -> AppResult<ArchivePluginProcessResponse> {
            let ArchivePluginOperation::ExtractArchive {
                archive_path,
                output_dir,
                password,
                ..
            } = request.operation
            else {
                panic!("expected an extract operation");
            };
            let name = Path::new(&archive_path)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned();
            let output_dir = PathBuf::from(output_dir);
            self.calls
                .lock()
                .unwrap()
                .push((name.clone(), output_dir.clone(), password.clone()));
            fs::write(output_dir.join("partial.part"), b"partial").unwrap();

            let answer = |status| ArchivePluginProcessResponse {
                status,
                files: Vec::new(),
                replaced_source_paths: Vec::new(),
                processed_recovery_paths: Vec::new(),
                expanded_bytes: None,
                copied_bytes: None,
                staged_bytes: None,
                error_code: None,
                message: None,
            };
            let emitted: Vec<(&str, &[u8])> =
                match self.script.get(name.as_str()).expect("unscripted archive") {
                    TreeStep::Emits(members) | TreeStep::EmitsHandled(members, _) => {
                        members.clone()
                    }
                    TreeStep::Fails => {
                        return Ok(ArchivePluginProcessResponse {
                            error_code: Some("corrupt_archive".to_string()),
                            ..answer(ArchivePluginStatus::Failed)
                        });
                    }
                    TreeStep::Locked(accepted) => match password.as_deref() {
                        Some(given) if given == *accepted => {
                            vec![("unlocked.mkv", b"video".as_slice())]
                        }
                        Some(_) => return Ok(answer(ArchivePluginStatus::PasswordInvalid)),
                        None => return Ok(answer(ArchivePluginStatus::PasswordRequired)),
                    },
                };
            let mut files = Vec::new();
            for (relative_path, bytes) in emitted {
                fs::write(output_dir.join(relative_path), bytes).unwrap();
                files.push(ArchivePluginExtractedFile {
                    relative_path: relative_path.to_string(),
                    size: Some(bytes.len() as u64),
                    checksum: None,
                });
            }
            let processed_recovery_paths = match self.script.get(name.as_str()).unwrap() {
                TreeStep::EmitsHandled(_, paths) => {
                    paths.iter().map(|path| (*path).to_string()).collect()
                }
                _ => Vec::new(),
            };
            Ok(ArchivePluginProcessResponse {
                files,
                processed_recovery_paths,
                ..answer(ArchivePluginStatus::Ok)
            })
        }
    }

    struct TreeRun {
        result: AppResult<Option<PathBuf>>,
        calls: Vec<(String, PathBuf, Option<String>)>,
        source: tempfile::TempDir,
        destination: tempfile::TempDir,
    }

    impl TreeRun {
        fn staging_dirs(&self) -> Vec<PathBuf> {
            fs::read_dir(self.destination.path())
                .unwrap()
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| is_archive_staging_dir(path))
                .collect()
        }

        /// The download and the unrelated file beside the staging area are
        /// exactly as they were before the extraction.
        fn assert_unrelated_files_preserved(&self, archives: &[&str]) {
            for archive in archives {
                assert_eq!(
                    fs::read(self.source.path().join(archive)).unwrap(),
                    b"archive",
                    "{archive}"
                );
            }
            assert_eq!(
                fs::read(self.source.path().join("quiet.harbor.nfo")).unwrap(),
                b"info"
            );
            assert_eq!(
                fs::read(self.destination.path().join("unrelated.mkv")).unwrap(),
                b"keep me"
            );
        }
    }

    async fn run_tree(
        archives: &[&str],
        script: Vec<(&'static str, TreeStep)>,
        candidates: &ArchivePasswordCandidates,
    ) -> TreeRun {
        let source = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        for archive in archives {
            let path = source.path().join(archive);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"archive").unwrap();
        }
        fs::write(source.path().join("quiet.harbor.nfo"), b"info").unwrap();
        fs::write(destination.path().join("unrelated.mkv"), b"keep me").unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let client: Arc<dyn ArchiveExtractorClient> = Arc::new(TreeArchiveClient {
            script: script.into_iter().collect(),
            calls: Arc::clone(&calls),
        });
        let provider: Arc<dyn ArchiveExtractorPluginProvider> =
            Arc::new(RecordingArchiveProvider {
                client,
                formats: vec![
                    ArchivePluginFormat::Rar,
                    ArchivePluginFormat::SevenZip,
                    ArchivePluginFormat::Zip,
                ],
            });

        let result = extract_archives_if_needed(
            source.path(),
            is_sample_named_file,
            Some(ArchiveExtractionDestination::new(
                destination.path(),
                "archive-tree",
            )),
            candidates,
            Some(provider),
        )
        .await;
        let calls = calls.lock().unwrap().clone();
        TreeRun {
            result,
            calls,
            source,
            destination,
        }
    }

    fn call_names(run: &TreeRun) -> Vec<&str> {
        run.calls.iter().map(|(name, _, _)| name.as_str()).collect()
    }

    fn relative_files(root: &Path) -> Vec<String> {
        let mut files = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if entry.file_name() != ARCHIVE_WORKSPACE_OWNER_FILE {
                    files.push(
                        path.strip_prefix(root)
                            .unwrap()
                            .to_string_lossy()
                            .replace('\\', "/"),
                    );
                }
            }
        }
        files.sort();
        files
    }

    #[tokio::test]
    async fn mixed_named_archive_and_independent_par2_are_both_extracted() {
        let run = run_tree(
            &["episode.zip", "independent.par2"],
            vec![
                (
                    "episode.zip",
                    TreeStep::Emits(vec![("Show.S01E01.mkv", b"one")]),
                ),
                (
                    "independent.par2",
                    TreeStep::Emits(vec![("Show.S01E02.mkv", b"two")]),
                ),
            ],
            &ArchivePasswordCandidates::default(),
        )
        .await;
        assert!(run.result.as_ref().unwrap().is_some());
        assert_eq!(call_names(&run), ["episode.zip", "independent.par2"]);
        run.assert_unrelated_files_preserved(&["episode.zip", "independent.par2"]);
    }

    #[tokio::test]
    async fn handled_recovery_metadata_is_not_extracted_twice() {
        let run = run_tree(
            &["episode.zip", "episode.par2", "independent.par2"],
            vec![
                (
                    "episode.zip",
                    TreeStep::EmitsHandled(vec![("Show.S01E01.mkv", b"one")], vec!["episode.par2"]),
                ),
                ("episode.par2", TreeStep::Fails),
                (
                    "independent.par2",
                    TreeStep::Emits(vec![("Show.S01E02.mkv", b"two")]),
                ),
            ],
            &ArchivePasswordCandidates::default(),
        )
        .await;
        assert!(run.result.as_ref().unwrap().is_some());
        assert_eq!(call_names(&run), ["episode.zip", "independent.par2"]);
    }

    #[tokio::test]
    async fn subtitle_only_archive_output_is_retained_for_loose_video() {
        let run = run_tree(
            &["Episode.mkv", "Subs/subs.rar"],
            vec![(
                "subs.rar",
                TreeStep::Emits(vec![("Episode.eng.srt", b"subtitle")]),
            )],
            &ArchivePasswordCandidates::default(),
        )
        .await;
        let root = run.result.as_ref().unwrap().as_ref().unwrap();
        assert!(root.join("out/Episode.eng.srt").is_file());
        run.assert_unrelated_files_preserved(&["Episode.mkv", "Subs/subs.rar"]);
    }

    #[test]
    fn reported_replacement_paths_must_stay_within_source() {
        let source = tempfile::tempdir().unwrap();
        assert!(validate_reported_sources(source.path(), &["../elsewhere.mkv".into()]).is_err());
        assert!(validate_reported_sources(source.path(), &["missing.mkv".into()]).is_err());
        #[cfg(unix)]
        {
            let outside = tempfile::NamedTempFile::new().unwrap();
            std::os::unix::fs::symlink(outside.path(), source.path().join("link.mkv")).unwrap();
            assert!(validate_reported_sources(source.path(), &["link.mkv".into()]).is_err());
        }
    }

    #[tokio::test]
    async fn every_archive_set_of_a_season_pack_is_extracted_into_its_own_directory() {
        let run = run_tree(
            &[
                "quiet.harbor.s01e01.rar",
                "quiet.harbor.s01e02.rar",
                "Extras/quiet.harbor.extras.zip",
            ],
            vec![
                (
                    "quiet.harbor.s01e01.rar",
                    TreeStep::Emits(vec![("quiet.harbor.s01e01.mkv", b"one")]),
                ),
                (
                    "quiet.harbor.s01e02.rar",
                    TreeStep::Emits(vec![("quiet.harbor.s01e02.mkv", b"two")]),
                ),
                (
                    "quiet.harbor.extras.zip",
                    TreeStep::Emits(vec![("cover.jpg", b"jpg")]),
                ),
            ],
            &ArchivePasswordCandidates::default(),
        )
        .await;

        let root = run
            .result
            .as_ref()
            .unwrap()
            .clone()
            .expect("video extracted");
        assert_eq!(run.staging_dirs(), vec![root.clone()]);
        assert_eq!(
            call_names(&run),
            [
                "quiet.harbor.s01e01.rar",
                "quiet.harbor.s01e02.rar",
                "quiet.harbor.extras.zip"
            ]
        );
        // A set without video keeps its output; so does every other set.
        assert_eq!(
            relative_files(&root),
            [
                "out-2/partial.part",
                "out-2/quiet.harbor.s01e02.mkv",
                "out-3/cover.jpg",
                "out-3/partial.part",
                "out/partial.part",
                "out/quiet.harbor.s01e01.mkv",
            ]
        );
        run.assert_unrelated_files_preserved(&[
            "quiet.harbor.s01e01.rar",
            "quiet.harbor.s01e02.rar",
            "Extras/quiet.harbor.extras.zip",
        ]);
    }

    #[tokio::test]
    async fn a_failed_set_stops_before_unrelated_sets() {
        let run = run_tree(
            &[
                "quiet.harbor.s01e01.rar",
                "quiet.harbor.s01e02.rar",
                "quiet.harbor.s01e03.rar",
            ],
            vec![
                ("quiet.harbor.s01e01.rar", TreeStep::Fails),
                (
                    "quiet.harbor.s01e02.rar",
                    TreeStep::Emits(vec![("quiet.harbor.s01e02.mkv", b"two")]),
                ),
                ("quiet.harbor.s01e03.rar", TreeStep::Locked("unknown")),
            ],
            &ArchivePasswordCandidates::default(),
        )
        .await;

        assert!(
            run.result
                .as_ref()
                .unwrap_err()
                .to_string()
                .contains("corrupt_archive")
        );
        assert_eq!(run.calls.len(), 1);
        assert!(run.staging_dirs().is_empty());
        run.assert_unrelated_files_preserved(&[
            "quiet.harbor.s01e01.rar",
            "quiet.harbor.s01e02.rar",
            "quiet.harbor.s01e03.rar",
        ]);
    }

    #[tokio::test]
    async fn the_first_failure_is_returned_without_spending_remaining_budget() {
        let run = run_tree(
            &[
                "quiet.harbor.s01e01.rar",
                "quiet.harbor.s01e02.rar",
                "Extras/quiet.harbor.extras.zip",
            ],
            vec![
                ("quiet.harbor.s01e01.rar", TreeStep::Fails),
                ("quiet.harbor.s01e02.rar", TreeStep::Locked("unknown")),
                (
                    "quiet.harbor.extras.zip",
                    TreeStep::Emits(vec![("cover.jpg", b"jpg")]),
                ),
            ],
            &ArchivePasswordCandidates::default(),
        )
        .await;

        let error = run.result.as_ref().unwrap_err();
        assert!(error.to_string().contains("corrupt_archive"), "{error}");
        assert_eq!(run.calls.len(), 1);
        assert!(run.staging_dirs().is_empty());
        run.assert_unrelated_files_preserved(&[
            "quiet.harbor.s01e01.rar",
            "quiet.harbor.s01e02.rar",
            "Extras/quiet.harbor.extras.zip",
        ]);
    }

    #[tokio::test]
    async fn a_locked_second_set_is_opened_by_a_candidate_and_keeps_only_its_final_output() {
        let run = run_tree(
            &["quiet.harbor.s01e01.rar", "quiet.harbor.s01e02.rar"],
            vec![
                (
                    "quiet.harbor.s01e01.rar",
                    TreeStep::Emits(vec![("quiet.harbor.s01e01.mkv", b"one")]),
                ),
                (
                    "quiet.harbor.s01e02.rar",
                    TreeStep::Locked("indexer-secret"),
                ),
            ],
            &operator_indexer_and_name_candidates(),
        )
        .await;

        let root = run
            .result
            .as_ref()
            .unwrap()
            .clone()
            .expect("video extracted");
        let passwords: Vec<_> = run
            .calls
            .iter()
            .map(|(name, _, password)| (name.as_str(), password.as_deref()))
            .collect();
        assert_eq!(
            passwords,
            [
                ("quiet.harbor.s01e01.rar", None),
                ("quiet.harbor.s01e02.rar", None),
                ("quiet.harbor.s01e02.rar", Some("typed-guess")),
                ("quiet.harbor.s01e02.rar", Some("indexer-secret")),
            ]
        );
        // Only the accepted attempt's output remains in the second set's
        // directory; the first set's output was never touched.
        assert_eq!(
            relative_files(&root),
            [
                "out-2/partial.part",
                "out-2/unlocked.mkv",
                "out/partial.part",
                "out/quiet.harbor.s01e01.mkv",
            ]
        );
        run.assert_unrelated_files_preserved(&[
            "quiet.harbor.s01e01.rar",
            "quiet.harbor.s01e02.rar",
        ]);
    }

    #[tokio::test]
    async fn an_archive_inside_an_archive_is_extracted_and_left_in_place() {
        let run = run_tree(
            &["quiet.harbor.s01e01.rar"],
            vec![
                (
                    "quiet.harbor.s01e01.rar",
                    TreeStep::Emits(vec![("quiet.harbor.s01e01.zip", b"inner")]),
                ),
                (
                    "quiet.harbor.s01e01.zip",
                    TreeStep::Emits(vec![("quiet.harbor.s01e01.mkv", b"video")]),
                ),
            ],
            &ArchivePasswordCandidates::default(),
        )
        .await;

        let root = run
            .result
            .as_ref()
            .unwrap()
            .clone()
            .expect("video extracted");
        let (_, inner_output, _) = &run.calls[1];
        assert_eq!(inner_output, &root.join("nested-1-1"));
        assert_eq!(
            relative_files(&root),
            [
                "nested-1-1/partial.part",
                "nested-1-1/quiet.harbor.s01e01.mkv",
                "out/partial.part",
                "out/quiet.harbor.s01e01.zip",
            ]
        );
        run.assert_unrelated_files_preserved(&["quiet.harbor.s01e01.rar"]);
    }

    #[tokio::test]
    async fn archives_nested_past_the_limit_without_video_fail_and_are_cleaned_up() {
        let run = run_tree(
            &["level0.rar"],
            vec![
                ("level0.rar", TreeStep::Emits(vec![("level1.rar", b"a")])),
                ("level1.rar", TreeStep::Emits(vec![("level2.rar", b"b")])),
                ("level2.rar", TreeStep::Emits(vec![("level3.rar", b"c")])),
            ],
            &ArchivePasswordCandidates::default(),
        )
        .await;

        let error = run.result.as_ref().unwrap_err();
        assert!(error.to_string().contains("nested more than"), "{error}");
        assert_eq!(call_names(&run), ["level0.rar", "level1.rar", "level2.rar"]);
        assert!(run.staging_dirs().is_empty());
        run.assert_unrelated_files_preserved(&["level0.rar"]);
    }

    #[tokio::test]
    async fn other_video_does_not_hide_excessive_archive_nesting() {
        let run = run_tree(
            &["a.rar", "level0.rar"],
            vec![
                (
                    "a.rar",
                    TreeStep::Emits(vec![("quiet.harbor.mkv", b"video")]),
                ),
                ("level0.rar", TreeStep::Emits(vec![("level1.rar", b"a")])),
                ("level1.rar", TreeStep::Emits(vec![("level2.rar", b"b")])),
                ("level2.rar", TreeStep::Emits(vec![("level3.rar", b"c")])),
            ],
            &ArchivePasswordCandidates::default(),
        )
        .await;

        assert!(
            run.result
                .as_ref()
                .unwrap_err()
                .to_string()
                .contains("nested more than")
        );
        assert_eq!(
            call_names(&run),
            ["a.rar", "level0.rar", "level1.rar", "level2.rar"]
        );
        assert!(run.staging_dirs().is_empty());
        run.assert_unrelated_files_preserved(&["a.rar", "level0.rar"]);
    }

    #[tokio::test]
    async fn too_many_nested_archives_fail_and_are_cleaned_up() {
        const INNER: [&str; MAX_NESTED_ARCHIVES + 1] = [
            "inner01.zip",
            "inner02.zip",
            "inner03.zip",
            "inner04.zip",
            "inner05.zip",
            "inner06.zip",
            "inner07.zip",
            "inner08.zip",
            "inner09.zip",
            "inner10.zip",
            "inner11.zip",
            "inner12.zip",
            "inner13.zip",
            "inner14.zip",
            "inner15.zip",
            "inner16.zip",
            "inner17.zip",
        ];
        let run = run_tree(
            &["outer.rar"],
            vec![(
                "outer.rar",
                TreeStep::Emits(
                    INNER
                        .iter()
                        .map(|name| (*name, b"zip".as_slice()))
                        .collect(),
                ),
            )],
            &ArchivePasswordCandidates::default(),
        )
        .await;

        let error = run.result.as_ref().unwrap_err();
        assert!(error.to_string().contains("nested archives"), "{error}");
        assert_eq!(call_names(&run), ["outer.rar"]);
        assert!(run.staging_dirs().is_empty());
        run.assert_unrelated_files_preserved(&["outer.rar"]);
    }

    #[tokio::test]
    async fn sets_without_video_anywhere_leave_no_workspace() {
        let run = run_tree(
            &["quiet.harbor.rar", "quiet.harbor.extras.rar"],
            vec![
                (
                    "quiet.harbor.rar",
                    TreeStep::Emits(vec![("readme.txt", b"txt")]),
                ),
                (
                    "quiet.harbor.extras.rar",
                    TreeStep::Emits(vec![("cover.jpg", b"jpg")]),
                ),
            ],
            &ArchivePasswordCandidates::default(),
        )
        .await;

        assert!(run.result.as_ref().unwrap().is_none());
        assert_eq!(run.calls.len(), 2);
        assert!(run.staging_dirs().is_empty());
        run.assert_unrelated_files_preserved(&["quiet.harbor.rar", "quiet.harbor.extras.rar"]);
    }

    #[test]
    fn find_archive_sets_lists_every_set_top_level_first() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("b.rar"), b"rar").unwrap();
        fs::write(dir.path().join("a.part1.rar"), b"rar").unwrap();
        fs::write(dir.path().join("a.part2.rar"), b"rar").unwrap();
        fs::create_dir_all(dir.path().join("Disc2")).unwrap();
        fs::write(dir.path().join("Disc2/disc2.7z"), b"7z").unwrap();
        fs::create_dir_all(dir.path().join("Sample")).unwrap();
        fs::write(dir.path().join("Sample/sample.rar"), b"rar").unwrap();

        let names: Vec<_> = find_archive_sets(dir.path())
            .unwrap()
            .into_iter()
            .map(|(path, _)| {
                path.strip_prefix(dir.path())
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        assert_eq!(names, ["a.part1.rar", "b.rar", "Disc2/disc2.7z"]);
    }

    #[test]
    fn output_caps_count_across_every_set_of_a_workspace() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("quiet.harbor.mkv"), b"video").unwrap();
        let response = ArchivePluginProcessResponse {
            status: ArchivePluginStatus::Ok,
            files: Vec::new(),
            replaced_source_paths: Vec::new(),
            processed_recovery_paths: Vec::new(),
            expanded_bytes: None,
            copied_bytes: None,
            staged_bytes: None,
            error_code: None,
            message: None,
        };

        let totals =
            validate_archive_plugin_output(dir.path(), &response, PluginOutputTotals::default())
                .unwrap();
        assert_eq!((totals.files, totals.directories, totals.bytes), (1, 1, 5));

        let near_file_cap = PluginOutputTotals {
            files: MAX_PLUGIN_OUTPUT_FILES,
            ..totals
        };
        let error =
            validate_archive_plugin_output(dir.path(), &response, near_file_cap).unwrap_err();
        assert!(error.to_string().contains("too many files"), "{error}");

        let near_byte_cap = PluginOutputTotals {
            bytes: MAX_PLUGIN_OUTPUT_BYTES - 1,
            ..totals
        };
        let error =
            validate_archive_plugin_output(dir.path(), &response, near_byte_cap).unwrap_err();
        assert!(error.to_string().contains("exceeds"), "{error}");
    }

    #[test]
    fn an_archive_source_inside_its_own_output_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let output = root.path().join("out");
        fs::create_dir_all(output.join("inner")).unwrap();
        let sibling = root.path().join("source");
        fs::create_dir_all(&sibling).unwrap();

        assert!(ensure_source_outside_output(&output, &output).is_err());
        assert!(ensure_source_outside_output(&output.join("inner"), &output).is_err());
        assert!(ensure_source_outside_output(&sibling, &output).is_ok());
        assert!(ensure_source_outside_output(root.path(), &output).is_ok());
    }

    #[tokio::test]
    async fn discarding_an_output_touches_only_output_directories_of_a_staging_workspace() {
        let parent = tempfile::tempdir().unwrap();
        let workspace = ArchiveExtractionWorkspace::create(&ArchiveExtractionDestination::new(
            parent.path(),
            "test-import",
        ))
        .await
        .unwrap()
        .root;
        let plain = parent.path().join("Quiet.Harbor.S01");
        for dir in [
            workspace.join("out-2"),
            workspace.join("nested-1-3"),
            workspace.join("keep"),
            plain.join("out"),
        ] {
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("file.mkv"), b"video").unwrap();
        }

        discard_workspace_output_dir(&workspace, &workspace.join("out-2")).await;
        discard_workspace_output_dir(&workspace, &workspace.join("nested-1-3")).await;
        discard_workspace_output_dir(&workspace, &workspace.join("keep")).await;
        discard_workspace_output_dir(&workspace, &workspace.join("keep/..")).await;
        discard_workspace_output_dir(&workspace, &workspace).await;
        discard_workspace_output_dir(&plain, &plain.join("out")).await;
        discard_workspace_output_dir(&workspace, &plain.join("out")).await;

        assert!(!workspace.join("out-2").exists());
        assert!(!workspace.join("nested-1-3").exists());
        assert!(workspace.join("keep/file.mkv").is_file());
        assert!(plain.join("out/file.mkv").is_file());
    }

    #[tokio::test]
    async fn output_directories_are_only_prepared_inside_the_workspace_and_empty() {
        let parent = tempfile::tempdir().unwrap();
        let workspace = parent.path().join(".scryer-ax-0123456789abcdef");
        fs::create_dir_all(workspace.join("out")).unwrap();

        prepare_workspace_output_dir(&workspace, &workspace.join("out"))
            .await
            .unwrap();
        prepare_workspace_output_dir(&workspace, &workspace.join("out-2"))
            .await
            .unwrap();
        assert!(workspace.join("out-2").is_dir());

        fs::write(workspace.join("out/leftover.mkv"), b"video").unwrap();
        assert!(
            prepare_workspace_output_dir(&workspace, &workspace.join("out"))
                .await
                .is_err()
        );
        assert!(
            prepare_workspace_output_dir(&workspace, &parent.path().join("out"))
                .await
                .is_err()
        );
        assert!(!parent.path().join("out").exists());
        assert!(workspace.join("out/leftover.mkv").is_file());
    }
    #[test]
    fn only_the_first_volume_of_a_split_set_starts_an_archive_set() {
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, bytes: &[u8]| {
            let path = dir.path().join(name);
            fs::write(&path, bytes).unwrap();
            path
        };
        let named_7z = write("quiet.harbor.7z.001", b"not sniffed");
        let named_zip = write("quiet.harbor.ZIP.001", b"not sniffed");
        let later_volume = write("quiet.harbor.7z.002", b"7z\xbc\xaf\x27\x1c\x00\x04");
        let bare_7z = write("quiet.harbor.a.001", b"7z\xbc\xaf\x27\x1c\x00\x04");
        let bare_zip = write("quiet.harbor.b.001", b"PK\x03\x04\x14\x00");
        let split_video = write("quiet.harbor.mkv.001", b"\x1a\x45\xdf\xa3\x01\x00");
        let split_rar = write("quiet.harbor.c.001", b"Rar!\x1a\x07\x01\x00");
        let short = write("quiet.harbor.d.001", b"7z");

        let kind = |path: &Path| archive_type_for_path(path).map(ArchiveType::as_str);
        assert_eq!(kind(&named_7z), Some("7z"));
        assert_eq!(kind(&named_zip), Some("zip"));
        assert_eq!(kind(&bare_7z), Some("7z"));
        assert_eq!(kind(&bare_zip), Some("zip"));
        assert_eq!(kind(&later_volume), None);
        assert_eq!(kind(&split_video), None);
        assert_eq!(kind(&split_rar), None);
        assert_eq!(kind(&short), None);
    }

    #[test]
    fn a_split_set_is_discovered_by_its_first_volume_only() {
        let dir = tempfile::tempdir().unwrap();
        for volume in ["001", "002", "003"] {
            fs::write(
                dir.path().join(format!("quiet.harbor.s01.7z.{volume}")),
                b"7z",
            )
            .unwrap();
        }

        let sets = find_archive_sets(dir.path()).unwrap();
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].0, dir.path().join("quiet.harbor.s01.7z.001"));
        assert!(matches!(sets[0].1, ArchiveType::SevenZip));
    }

    #[tokio::test]
    async fn a_split_set_is_handed_to_the_plugin_as_its_first_volume_and_inner_format() {
        let source = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        for volume in ["001", "002"] {
            fs::write(
                source.path().join(format!("quiet.harbor.zip.{volume}")),
                b"zip",
            )
            .unwrap();
        }
        let operation = Arc::new(Mutex::new(None));
        let client: Arc<dyn ArchiveExtractorClient> = Arc::new(RecordingArchiveClient {
            operation: Arc::clone(&operation),
            write_output_file: true,
        });
        let provider: Arc<dyn ArchiveExtractorPluginProvider> =
            Arc::new(RecordingArchiveProvider {
                client,
                formats: vec![ArchivePluginFormat::Zip],
            });

        let extracted = extract_archives_if_needed(
            source.path(),
            is_sample_named_file,
            Some(ArchiveExtractionDestination::new(
                destination.path(),
                "split",
            )),
            &ArchivePasswordCandidates::default(),
            Some(provider),
        )
        .await
        .unwrap()
        .expect("video extracted");

        assert!(extracted.join("out/movie.mkv").is_file());
        let Some(ArchivePluginOperation::ExtractArchive {
            archive_path,
            format,
            ..
        }) = operation.lock().unwrap().clone()
        else {
            panic!("expected an extract operation");
        };
        assert_eq!(
            PathBuf::from(archive_path),
            source.path().join("quiet.harbor.zip.001")
        );
        assert_eq!(format, ArchivePluginFormat::Zip);
    }

    #[tokio::test]
    async fn split_corruption_preserves_its_actual_diagnosis() {
        let run = run_tree(
            &["quiet.harbor.7z.001", "quiet.harbor.7z.002"],
            vec![("quiet.harbor.7z.001", TreeStep::Fails)],
            &ArchivePasswordCandidates::default(),
        )
        .await;

        let error = run.result.as_ref().unwrap_err();
        assert!(
            matches!(error, AppError::ArchiveExtractionFailed { .. }),
            "{error:?}"
        );
        let message = error.to_string();
        assert!(message.contains("corrupt_archive"), "{message}");
        assert!(!is_password_required_error(error), "{message}");
        assert!(run.staging_dirs().is_empty());
        run.assert_unrelated_files_preserved(&["quiet.harbor.7z.001", "quiet.harbor.7z.002"]);
    }

    #[tokio::test]
    async fn a_split_set_without_video_is_not_a_plugin_upgrade_error() {
        let run = run_tree(
            &["quiet.harbor.7z.001", "quiet.harbor.7z.002"],
            vec![(
                "quiet.harbor.7z.001",
                TreeStep::Emits(vec![("readme.txt", b"txt")]),
            )],
            &ArchivePasswordCandidates::default(),
        )
        .await;

        assert!(run.result.as_ref().unwrap().is_none());
        assert!(run.staging_dirs().is_empty());
        run.assert_unrelated_files_preserved(&["quiet.harbor.7z.001", "quiet.harbor.7z.002"]);
    }

    #[tokio::test]
    async fn a_failing_unsplit_set_is_reported_as_itself() {
        let run = run_tree(
            &["quiet.harbor.7z"],
            vec![("quiet.harbor.7z", TreeStep::Fails)],
            &ArchivePasswordCandidates::default(),
        )
        .await;

        let error = run.result.as_ref().unwrap_err();
        assert!(
            matches!(error, AppError::ArchiveExtractionFailed { .. }),
            "{error:?}"
        );
        assert!(!error.to_string().contains("split"), "{error}");
        assert!(run.staging_dirs().is_empty());
    }
}

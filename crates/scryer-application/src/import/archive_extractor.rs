//! Archive extraction for the import pipeline.
//!
//! Detects RAR, 7z, and zip archives in download directories. Extraction is
//! delegated to the optional archive extraction plugin, which also owns PAR2
//! verification, placement and repair: the plugin scans the read-only source
//! directory it is given for `.par2` sets and repairs internally, emitting the
//! result into its writable output directory. The host neither orchestrates
//! nor observes that step.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crate::import::archive_passwords::ArchivePasswordCandidates;
use crate::{AppError, AppResult, ArchiveExtractorPluginProvider};
use scryer_plugin_sdk::{
    ArchivePluginFormat, ArchivePluginOperation, ArchivePluginProcessRequest,
    ArchivePluginProcessResponse, ArchivePluginStatus,
};
use tracing::info;

const EXTRACTED_DIR_NAME: &str = "_scryer_extracted";
const ARCHIVE_STAGING_PREFIX: &str = ".scryer-ax-";
const ARCHIVE_WRITE_PROBE_PREFIX: &str = ".scryer-write-probe-";
const LEGACY_ARCHIVE_STAGING_PREFIX: &str = ".scryer-archive-extract-";
const ARCHIVE_STAGING_OUTPUT_DIR: &str = "out";
const ARCHIVE_STAGING_CREATE_ATTEMPTS: usize = 16;
const STALE_ARCHIVE_STAGING_AFTER: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_PLUGIN_OUTPUT_FILES: usize = 20_000;
const MAX_PLUGIN_OUTPUT_DIRECTORIES: usize = 20_000;
const MAX_PLUGIN_OUTPUT_ENTRIES: usize = MAX_PLUGIN_OUTPUT_FILES + MAX_PLUGIN_OUTPUT_DIRECTORIES;
const MAX_PLUGIN_OUTPUT_BYTES: u64 = 2 * 1024 * 1024 * 1024 * 1024;
const MAX_ARCHIVE_DISCOVERY_DEPTH: usize = 3;
const MAX_ARCHIVE_DISCOVERY_DIRECTORIES: usize = 256;
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
}

#[derive(Debug, Clone)]
pub struct ArchiveExtractionDestination {
    staging_parent: PathBuf,
    stale_cleanup_parents: Vec<PathBuf>,
    _import_id: String,
}

impl ArchiveExtractionDestination {
    pub fn new(staging_parent: impl Into<PathBuf>, import_id: impl Into<String>) -> Self {
        Self {
            staging_parent: staging_parent.into(),
            stale_cleanup_parents: Vec::new(),
            _import_id: import_id.into(),
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

#[derive(Debug, Clone)]
struct ArchiveExtractionWorkspace {
    root: PathBuf,
    output_dir: PathBuf,
}

struct ArchivePluginExtraction {
    source_dir: PathBuf,
    archive_path: PathBuf,
    archive_type: ArchiveType,
    format: ArchivePluginFormat,
    password: Option<String>,
    provider: Arc<dyn ArchiveExtractorPluginProvider>,
    output_dir: PathBuf,
}

/// Archive type detected in a download directory.
#[derive(Debug, Clone, Copy)]
pub enum ArchiveType {
    Rar,
    SevenZip,
    Zip,
}

impl ArchiveType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rar => "RAR",
            Self::SevenZip => "7z",
            Self::Zip => "zip",
        }
    }
}

/// If the download directory contains no importable video files but has
/// archive files, extract them to a hidden destination-side staging directory
/// and return the path. Returns `None` if no extraction was needed (importable
/// video files exist directly) or the archives held no video.
///
/// `is_sample` is the sample rule the import scan will apply afterwards: a
/// video it would discard does not make the download's archives redundant.
///
/// Every archive set the download holds is extracted into its own output
/// directory of one workspace, and an output that holds no importable video
/// but does hold archives has those extracted too, a bounded number of levels
/// deep. A set that fails is skipped when the rest of the download yields
/// importable video; when nothing does, the first failure is returned and the
/// workspace is removed.
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
            .map_err(|e| AppError::Repository(format!("archive detection task failed: {e}")))??
    };

    if sets.is_empty() {
        return Ok(None);
    }
    let Some(destination) = destination else {
        return Err(AppError::Validation(format!(
            "archive extraction requires a resolved import destination before staging output for {}",
            dir.display()
        )));
    };
    let workspace = ArchiveExtractionWorkspace::create(&destination).await?;

    info!(
        archive = %sets[0].0.display(),
        archive_type = sets[0].1.as_str(),
        archive_sets = sets.len(),
        workspace = %workspace.root.display(),
        "extracting archive before import"
    );

    let workspace_root = workspace.root.clone();
    let Some(provider) = archive_provider else {
        cleanup_extracted_dir(&workspace_root).await;
        return Err(AppError::archive_extraction_plugin_required(Some(
            dir.to_string_lossy().into_owned(),
        )));
    };

    let split_set = sets
        .iter()
        .map(|(path, _)| path.clone())
        .find(|path| is_split_set_first_volume(path));
    let extraction =
        extract_into_workspace(&workspace, sets, is_sample, passwords, &provider).await;

    match extraction {
        Ok(true) => Ok(Some(workspace_root)),
        Ok(false) if let Some(split_set) = split_set => {
            cleanup_extracted_dir(&workspace_root).await;
            Err(split_set_extraction_error(&split_set, None))
        }
        Ok(false) => {
            info!("archive extracted but no video files found in output");
            cleanup_extracted_dir(&workspace_root).await;
            Ok(None)
        }
        Err(error) => {
            cleanup_extracted_dir(&workspace_root).await;
            Err(error)
        }
    }
}

/// Extracts every set, then any archives found in outputs that hold no
/// importable video. Returns whether the workspace ended up holding video.
///
/// A set that fails is skipped and its output discarded, so one broken or
/// locked archive does not cost the import the video its siblings hold. The
/// first failure is returned when no set yields importable video. A timeout is
/// never skipped past: the remaining sets would each wait it out again.
async fn extract_into_workspace(
    workspace: &ArchiveExtractionWorkspace,
    sets: Vec<(PathBuf, ArchiveType)>,
    is_sample: fn(&Path) -> bool,
    passwords: &ArchivePasswordCandidates,
    provider: &Arc<dyn ArchiveExtractorPluginProvider>,
) -> AppResult<bool> {
    let mut totals = PluginOutputTotals::default();
    let mut first_failure = None;
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
        if set.extract_or_skip(&mut totals, &mut first_failure).await? {
            pass_outputs.push(output_dir);
        }
    }

    let mut nested_archives = 0usize;
    for depth in 1..=MAX_NESTED_ARCHIVE_DEPTH + 1 {
        let inner_sets = {
            let outputs = pass_outputs.clone();
            tokio::task::spawn_blocking(move || nested_archive_sets(&outputs, is_sample))
                .await
                .map_err(|e| AppError::Repository(format!("archive detection task failed: {e}")))?
        };
        if inner_sets.is_empty() {
            break;
        }
        if depth > MAX_NESTED_ARCHIVE_DEPTH {
            if has_importable_video_files(&workspace.root, is_sample) {
                info!(
                    depth = MAX_NESTED_ARCHIVE_DEPTH,
                    "leaving archives nested deeper than the extraction limit"
                );
                break;
            }
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
            if set.extract_or_skip(&mut totals, &mut first_failure).await? {
                next_outputs.push(output_dir);
            }
        }
        pass_outputs = next_outputs;
    }

    match first_failure {
        Some(error) if !has_importable_video_files(&workspace.root, is_sample) => Err(error),
        _ => Ok(has_video_files(&workspace.root)),
    }
}

/// The archive sets inside the given extraction outputs whose own output holds
/// no importable video.
fn nested_archive_sets(
    outputs: &[PathBuf],
    is_sample: fn(&Path) -> bool,
) -> Vec<(PathBuf, ArchiveType)> {
    outputs
        .iter()
        .filter(|output| !has_importable_video_files(output, is_sample))
        .flat_map(|output| find_archive_sets(output))
        .collect()
}

/// The error for a split set the plugin could not turn into video. Published
/// plugin versions cannot join split volumes, and the plain failure they give
/// (or an empty output) would read as a broken download; this names the real
/// remedy while keeping the plugin's own words.
fn split_set_extraction_error(first_volume: &Path, detail: Option<String>) -> AppError {
    let name = first_volume
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut message = format!(
        "This import is blocked because {name} is the first volume of a split archive set that the Archive Extraction plugin could not extract into video. Versions of the plugin that cannot join split volumes fail this way: update the Archive Extraction plugin, then re-import."
    );
    if let Some(detail) = detail {
        message.push_str(&format!(" Plugin result: {detail}"));
    }
    AppError::ArchiveExtractionPluginRequired {
        message,
        source_path: Some(first_volume.to_string_lossy().into_owned()),
    }
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
    /// Extracts the set, or on a failure other than a timeout discards its
    /// output, keeps the first such failure and reports the set as skipped.
    async fn extract_or_skip(
        &self,
        totals: &mut PluginOutputTotals,
        first_failure: &mut Option<AppError>,
    ) -> AppResult<bool> {
        match self.extract(totals).await {
            Ok(()) => Ok(true),
            Err(error) if is_timeout_error(&error) => Err(error),
            Err(error) => {
                discard_workspace_output_dir(&self.workspace.root, self.output_dir).await;
                tracing::warn!(
                    archive = %self.archive_path.display(),
                    error = %error,
                    "skipping an archive set that failed to extract"
                );
                first_failure.get_or_insert(error);
                Ok(false)
            }
        }
    }

    async fn extract(&self, totals: &mut PluginOutputTotals) -> AppResult<()> {
        let result = self.extract_with_candidates(totals).await;
        match result {
            Err(error)
                if is_split_set_first_volume(self.archive_path)
                    && !is_timeout_error(&error)
                    && !is_password_required_error(&error)
                    && !matches!(error, AppError::ArchiveExtractionPluginRequired { .. }) =>
            {
                Err(split_set_extraction_error(
                    self.archive_path,
                    Some(error.to_string()),
                ))
            }
            result => result,
        }
    }

    async fn extract_with_candidates(&self, totals: &mut PluginOutputTotals) -> AppResult<()> {
        let mut rejection = match self.attempt(None, totals).await? {
            ArchiveAttempt::Extracted => return Ok(()),
            ArchiveAttempt::PasswordRejected(error) => error,
        };
        for (index, candidate) in self.passwords.iter().enumerate() {
            match self.attempt(Some(candidate.value()), totals).await? {
                ArchiveAttempt::Extracted => {
                    info!(
                        password_source = candidate.source().as_str(),
                        candidate = index + 1,
                        candidates = self.passwords.len(),
                        "archive password candidate accepted"
                    );
                    return Ok(());
                }
                ArchiveAttempt::PasswordRejected(error) => rejection = error,
            }
        }
        if !self.passwords.is_empty() {
            info!(
                candidates = self.passwords.len(),
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
                provider: Arc::clone(self.provider),
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
    Extracted,
    PasswordRejected(AppError),
}

/// Creates an output directory directly inside a staging workspace, unless it
/// already exists and is empty.
async fn prepare_workspace_output_dir(workspace_root: &Path, output_dir: &Path) -> AppResult<()> {
    if output_dir.parent() != Some(workspace_root) {
        return Err(AppError::Validation(format!(
            "archive output directory {} is not inside the staging workspace",
            output_dir.display()
        )));
    }
    match tokio::fs::create_dir(output_dir).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let mut entries = tokio::fs::read_dir(output_dir).await.map_err(|error| {
                AppError::Repository(format!(
                    "failed to read archive staging output directory {}: {error}",
                    output_dir.display()
                ))
            })?;
            match entries.next_entry().await {
                Ok(None) => Ok(()),
                _ => Err(AppError::Validation(format!(
                    "archive staging output directory {} is not empty",
                    output_dir.display()
                ))),
            }
        }
        Err(error) => Err(AppError::Repository(format!(
            "failed to create archive staging output directory {}: {error}",
            output_dir.display()
        ))),
    }
}

/// Removes an output directory Scryer created directly inside one of its own
/// staging workspaces, and nothing else.
async fn discard_workspace_output_dir(workspace_root: &Path, output_dir: &Path) {
    if is_archive_staging_dir(workspace_root)
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
                path.display()
            ))
        })
    };
    if canonical(source_dir)?.starts_with(canonical(output_dir)?) {
        return Err(AppError::Validation(format!(
            "archive source {} lies inside its own extraction output",
            source_dir.display()
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
    let msg = error.to_string().to_ascii_lowercase();
    msg.contains("password") || msg.contains("encrypted") || msg.contains("wrong password")
}

pub fn is_timeout_error(error: &AppError) -> bool {
    matches!(error, AppError::ArchiveExtractionTimedOut { .. })
}

/// The archive sets to extract, each named by its first volume: the download
/// itself when it is an archive file, otherwise every set `find_archive_sets`
/// discovers, unless the download already holds importable video.
fn plan_archive_extraction(
    dir: &Path,
    is_sample: fn(&Path) -> bool,
) -> AppResult<Vec<(PathBuf, ArchiveType)>> {
    if dir.is_file() {
        return Ok(archive_type_for_path(dir)
            .map(|archive_type| (dir.to_path_buf(), archive_type))
            .into_iter()
            .collect());
    }

    // If importable video files already exist, no extraction needed. A release
    // whose only loose video is its sample still needs its archives.
    if has_importable_video_files(dir, is_sample) {
        return Ok(Vec::new());
    }

    let sets = find_archive_sets(dir);
    if sets.len() > MAX_ARCHIVE_SETS {
        return Err(AppError::Validation(format!(
            "download holds more than {MAX_ARCHIVE_SETS} archive sets"
        )));
    }
    Ok(sets)
}

fn archive_plugin_format_for_type(archive_type: ArchiveType) -> ArchivePluginFormat {
    match archive_type {
        ArchiveType::Rar => ArchivePluginFormat::Rar,
        ArchiveType::SevenZip => ArchivePluginFormat::SevenZip,
        ArchiveType::Zip => ArchivePluginFormat::Zip,
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
        provider,
        output_dir,
    } = request;

    let (client, operation) = {
        let Some(client) = provider.client_for_format(format) else {
            return Err(AppError::archive_extraction_plugin_required(Some(
                source_dir.to_string_lossy().into_owned(),
            )));
        };
        let operation = ArchivePluginOperation::ExtractArchive {
            archive_path: archive_path.to_string_lossy().into_owned(),
            output_dir: output_dir.to_string_lossy().into_owned(),
            format,
            password,
        };
        (client, operation)
    };
    let request = ArchivePluginProcessRequest { operation };
    let response = client.process(request).await?;
    let password_rejected = matches!(
        response.status,
        ArchivePluginStatus::PasswordRequired | ArchivePluginStatus::PasswordInvalid
    );
    match handle_archive_plugin_response(archive_type, output_dir, response, totals) {
        Ok(()) => Ok(ArchiveAttempt::Extracted),
        Err(error) if password_rejected => Ok(ArchiveAttempt::PasswordRejected(error)),
        Err(error) => Err(error),
    }
}

impl ArchiveExtractionWorkspace {
    async fn create(destination: &ArchiveExtractionDestination) -> AppResult<Self> {
        tokio::fs::create_dir_all(&destination.staging_parent)
            .await
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to create archive staging parent {}: {error}",
                    destination.staging_parent.display()
                ))
            })?;
        cleanup_stale_archive_artifacts(&destination.staging_parent).await;
        for parent in &destination.stale_cleanup_parents {
            if parent != &destination.staging_parent {
                cleanup_stale_archive_artifacts(parent).await;
            }
        }

        for _ in 0..ARCHIVE_STAGING_CREATE_ATTEMPTS {
            let root = destination.staging_parent.join(format!(
                "{ARCHIVE_STAGING_PREFIX}{}",
                short_staging_suffix()
            ));
            match tokio::fs::create_dir(&root).await {
                Ok(()) => {
                    let output_dir = root.join(ARCHIVE_STAGING_OUTPUT_DIR);
                    tokio::fs::create_dir(&output_dir).await.map_err(|error| {
                        AppError::Repository(format!(
                            "failed to create archive staging output directory {}: {error}",
                            output_dir.display()
                        ))
                    })?;
                    return Ok(Self { root, output_dir });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(AppError::Repository(format!(
                        "failed to create archive staging directory {}: {error}",
                        root.display()
                    )));
                }
            }
        }

        Err(AppError::Repository(format!(
            "failed to allocate a unique archive staging directory under {}",
            destination.staging_parent.display()
        )))
    }
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
        if !is_archive_staging_dir(&path) && !is_archive_write_probe_file(&path) {
            continue;
        }
        let Ok(metadata) = entry.metadata().await else {
            continue;
        };
        if is_archive_staging_dir(&path) && !metadata.is_dir() {
            continue;
        }
        if is_archive_write_probe_file(&path) && !metadata.is_file() {
            continue;
        }
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        if now.duration_since(modified).is_ok_and(|age| age >= min_age) {
            if metadata.is_dir() {
                let _ = tokio::fs::remove_dir_all(path).await;
            } else {
                let _ = tokio::fs::remove_file(path).await;
            }
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
                        output = %output_dir.display(),
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
        ArchivePluginStatus::PasswordRequired => Err(AppError::Validation(format!(
            "{} archive requires a password",
            archive_type.as_str()
        ))),
        ArchivePluginStatus::PasswordInvalid => Err(AppError::Validation(format!(
            "{} archive password is invalid",
            archive_type.as_str()
        ))),
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
    const PAR2_INSUFFICIENT_RECOVERY: &str = "par2_insufficient_recovery";

    let text = match (error_code, message) {
        (Some(code), Some(message)) => format!("{code}: {message}"),
        (Some(code), None) => code.to_string(),
        (None, Some(message)) => message.to_string(),
        (None, None) => "archive plugin extraction failed".to_string(),
    };

    if error_code == Some(PAR2_INSUFFICIENT_RECOVERY) {
        AppError::Validation(text)
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
            output_dir.display()
        ))
    })?;

    for file in &response.files {
        let path = safe_archive_output_path(output_dir, &file.relative_path)?;
        let metadata = std::fs::symlink_metadata(&path).map_err(|error| {
            AppError::Repository(format!(
                "archive plugin manifest output '{}' is missing or unreadable: {error}",
                path.display()
            ))
        })?;
        if !metadata.file_type().is_file() {
            return Err(AppError::Validation(format!(
                "archive plugin manifest output is not a regular file: {}",
                path.display()
            )));
        }
        ensure_path_under_output_with_root(&path, &output_root)?;
        if let Some(expected_size) = file.size
            && expected_size != metadata.len()
        {
            return Err(AppError::Validation(format!(
                "archive plugin manifest size mismatch for {}",
                path.display()
            )));
        }
    }

    let PluginOutputTotals {
        entries: mut entry_count,
        directories: mut directory_count,
        files: mut file_count,
        bytes: mut expanded_bytes,
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
                path.display()
            ))
        })?;
        if metadata.file_type().is_symlink() {
            return Err(AppError::Validation(format!(
                "archive plugin output contains a symlink: {}",
                path.display()
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
                    path.display()
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
                path.display()
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
            path.display()
        ))
    })?;
    if !canonical.starts_with(output_root) {
        return Err(AppError::Validation(format!(
            "archive entry escapes extraction directory: {}",
            path.display()
        )));
    }
    Ok(())
}

fn has_video_files(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && scryer_domain::is_video_file(&path) {
            return true;
        }
        if path.is_dir() && has_video_files(&path) {
            return true;
        }
    }
    false
}

/// Like `has_video_files`, but a video the import scan would discard as a
/// sample does not count.
fn has_importable_video_files(dir: &Path, is_sample: fn(&Path) -> bool) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && scryer_domain::is_video_file(&path) && !is_sample(&path) {
            return true;
        }
        if path.is_dir() && has_importable_video_files(&path, is_sample) {
            return true;
        }
    }
    false
}

/// Every archive set of a download, each named by its first volume: the top
/// level first, then subdirectories breadth-first in name order.
fn find_archive_sets(dir: &Path) -> Vec<(PathBuf, ArchiveType)> {
    archive_discovery_dirs(dir)
        .iter()
        .flat_map(|dir| archive_sets_in_dir(dir))
        .collect()
}

/// The download root followed by its subdirectories, breadth-first in name
/// order, at most `MAX_ARCHIVE_DISCOVERY_DEPTH` levels deep. Sample folders,
/// hidden folders and Scryer's own staging folders are never searched, and
/// symlinked directories are not followed.
fn archive_discovery_dirs(dir: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![dir.to_path_buf()];
    let mut frontier = vec![dir.to_path_buf()];
    for _ in 0..MAX_ARCHIVE_DISCOVERY_DEPTH {
        let mut next = Vec::new();
        for parent in &frontier {
            let Ok(entries) = std::fs::read_dir(parent) else {
                continue;
            };
            let mut children: Vec<PathBuf> = entries
                .flatten()
                .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
                .map(|entry| entry.path())
                .filter(|path| !is_excluded_archive_discovery_dir(path))
                .collect();
            children.sort();
            next.extend(children);
        }
        next.truncate(MAX_ARCHIVE_DISCOVERY_DIRECTORIES.saturating_sub(dirs.len()));
        if next.is_empty() {
            break;
        }
        dirs.extend(next.iter().cloned());
        frontier = next;
    }
    dirs
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
/// Only the preferred archive type present is considered (RAR, then 7z, then
/// zip); RAR volumes of one set collapse onto their first volume.
fn archive_sets_in_dir(dir: &Path) -> Vec<(PathBuf, ArchiveType)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut rar = Vec::new();
    let mut sevenz = Vec::new();
    let mut zip = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        match archive_type_for_path(&path) {
            Some(ArchiveType::Rar) => rar.push(path),
            Some(ArchiveType::SevenZip) => sevenz.push(path),
            Some(ArchiveType::Zip) => zip.push(path),
            None => {}
        }
    }

    if !rar.is_empty() {
        rar.sort_by_key(|path| rar_selection_key(path));
        let mut sets: Vec<(PathBuf, ArchiveType)> = Vec::new();
        let mut last_group: Option<String> = None;
        for path in rar {
            let (group, _, _) = rar_selection_key(&path);
            if last_group.as_ref() != Some(&group) {
                last_group = Some(group);
                sets.push((path, ArchiveType::Rar));
            }
        }
        return sets;
    }
    let (mut paths, archive_type) = if !sevenz.is_empty() {
        (sevenz, ArchiveType::SevenZip)
    } else {
        (zip, ArchiveType::Zip)
    };
    paths.sort();
    paths.into_iter().map(|path| (path, archive_type)).collect()
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
        "7z" => Some(ArchiveType::SevenZip),
        "zip" => Some(ArchiveType::Zip),
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

fn is_split_set_first_volume(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension == "001")
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

/// Clean up the extraction directory after import completes.
pub async fn cleanup_extracted_dir(dir: &Path) {
    if is_archive_staging_dir(dir) {
        let _ = tokio::fs::remove_dir_all(dir).await;
    }
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

fn is_archive_write_probe_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with(ARCHIVE_WRITE_PROBE_PREFIX))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::workflow::{is_sample_file, is_sample_named_file};
    use crate::{ArchiveExtractorClient, ArchiveExtractorPluginProvider};
    use scryer_plugin_sdk::ArchivePluginExtractedFile;
    use std::fs;
    use std::sync::{Arc, Mutex};

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
        find_archive_sets(dir).into_iter().next()
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
                if path.is_dir() { count_files(&path) } else { 1 }
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
        assert!(matches!(error, AppError::Repository(_)), "{error:?}");
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
        assert!(has_video_files(dir.path()));
    }

    #[test]
    fn has_video_files_ignores_non_video() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("readme.txt"), b"text").unwrap();
        fs::write(dir.path().join("archive.rar"), b"rar").unwrap();
        assert!(!has_video_files(dir.path()));
    }

    #[test]
    fn has_video_files_recursive() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("subdir");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("episode.mp4"), b"video").unwrap();
        assert!(has_video_files(dir.path()));
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

        assert!(first_archive_set(dir.path()).is_none());

        fs::write(within.join("vale.7z"), b"7z").unwrap();
        let (path, kind) = first_archive_set(dir.path()).unwrap();
        assert!(matches!(kind, ArchiveType::SevenZip));
        assert_eq!(path, within.join("vale.7z"));
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
        assert_eq!(names, ["harbor.rar", "vale.part01.rar"]);
        assert!(
            sets.iter()
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
    fn real_video_beside_archives_skips_extraction_for_every_rule() {
        let root = tempfile::tempdir().unwrap();
        let release = scene_rar_release_with_sample(root.path());
        write_full_size_video(&release.join("quiet.harbor.s06e07.1080p.web.h264-nogrp.mkv"));

        assert!(!archive_extraction_would_be_needed(&release, is_sample_file).unwrap());
        assert!(!archive_extraction_would_be_needed(&release, is_sample_named_file).unwrap());
    }

    #[test]
    fn small_unnamed_movie_beside_archives_is_not_treated_as_a_sample() {
        let root = tempfile::tempdir().unwrap();
        let release = root.path().join("Tiny.Reel.1931.480p");
        fs::create_dir(&release).unwrap();
        fs::write(release.join("tiny.reel.1931.480p.rar"), b"rar").unwrap();
        fs::write(release.join("tiny.reel.1931.480p.mkv"), b"short film").unwrap();

        assert!(!archive_extraction_would_be_needed(&release, is_sample_named_file).unwrap());
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
    async fn extract_no_op_when_video_exists() {
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
        .unwrap();
        assert!(result.is_none());
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
    async fn cleanup_only_removes_extracted_dir() {
        let dir = tempfile::tempdir().unwrap();
        let extracted = dir.path().join(EXTRACTED_DIR_NAME);
        fs::create_dir(&extracted).unwrap();
        fs::write(extracted.join("file.txt"), b"data").unwrap();

        cleanup_extracted_dir(&extracted).await;
        assert!(!extracted.exists());
        // Parent still exists
        assert!(dir.path().exists());
    }

    #[tokio::test]
    async fn stale_archive_artifact_cleanup_is_prefix_bounded() {
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

        assert!(!archive_dir.exists());
        assert!(!legacy_dir.exists());
        assert!(!probe_file.exists());
        assert!(keep_dir.exists());
        assert!(keep_file.exists());
    }

    #[tokio::test]
    async fn cleanup_removes_archive_staging_dir() {
        let dir = tempfile::tempdir().unwrap();
        let extracted = dir
            .path()
            .join(format!("{ARCHIVE_STAGING_PREFIX}import-123"));
        fs::create_dir(&extracted).unwrap();
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

    /// PAR2 repair moved inside the plugin. When the recovery set protects plain
    /// media files rather than an archive, the plugin repairs them into its
    /// output directory and reports them in `files` with `copied_bytes` set
    /// instead of `expanded_bytes`. The host must accept those exactly like
    /// extracted archive members.
    #[tokio::test]
    async fn plugin_emitted_plain_files_are_the_deliverable() {
        let source = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        fs::write(source.path().join("release.rar"), b"rar").unwrap();
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
        assert!(source.path().join("release.rar").exists());
        assert!(source.path().join("release.par2").exists());
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

        assert!(matches!(error, AppError::Validation(_)), "{error:?}");
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
                expanded_bytes: None,
                copied_bytes: None,
                staged_bytes: None,
                error_code: None,
                message: None,
            };
            let emitted: Vec<(&str, &[u8])> =
                match self.script.get(name.as_str()).expect("unscripted archive") {
                    TreeStep::Emits(members) => members.clone(),
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
            Ok(ArchivePluginProcessResponse {
                files,
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
                } else {
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
    async fn a_failing_set_is_skipped_when_another_set_yields_video() {
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

        let root = run
            .result
            .as_ref()
            .unwrap()
            .clone()
            .expect("video extracted");
        assert_eq!(run.calls.len(), 3);
        assert_eq!(run.staging_dirs(), vec![root.clone()]);
        // The skipped sets leave nothing behind, not even a partial member.
        assert_eq!(
            relative_files(&root),
            ["out-2/partial.part", "out-2/quiet.harbor.s01e02.mkv"]
        );
        run.assert_unrelated_files_preserved(&[
            "quiet.harbor.s01e01.rar",
            "quiet.harbor.s01e02.rar",
            "quiet.harbor.s01e03.rar",
        ]);
    }

    #[tokio::test]
    async fn the_first_failure_is_returned_when_no_set_yields_video() {
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
        assert_eq!(run.calls.len(), 3);
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
    async fn archives_nested_past_the_limit_are_left_when_other_sets_hold_video() {
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

        let root = run
            .result
            .as_ref()
            .unwrap()
            .clone()
            .expect("video extracted");
        assert_eq!(
            call_names(&run),
            ["a.rar", "level0.rar", "level1.rar", "level2.rar"]
        );
        assert!(root.join("nested-2-1/level3.rar").is_file());
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
        let workspace = parent.path().join(".scryer-ax-0123456789abcdef");
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

        let sets = find_archive_sets(dir.path());
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
    async fn a_plugin_that_cannot_join_split_volumes_is_named_as_the_cause() {
        let run = run_tree(
            &["quiet.harbor.7z.001", "quiet.harbor.7z.002"],
            vec![("quiet.harbor.7z.001", TreeStep::Fails)],
            &ArchivePasswordCandidates::default(),
        )
        .await;

        let error = run.result.as_ref().unwrap_err();
        assert!(
            matches!(error, AppError::ArchiveExtractionPluginRequired { .. }),
            "{error:?}"
        );
        let message = error.to_string();
        assert!(message.contains("quiet.harbor.7z.001"), "{message}");
        assert!(message.contains("split archive set"), "{message}");
        assert!(message.contains("corrupt_archive"), "{message}");
        assert!(!is_password_required_error(error), "{message}");
        assert!(run.staging_dirs().is_empty());
        run.assert_unrelated_files_preserved(&["quiet.harbor.7z.001", "quiet.harbor.7z.002"]);
    }

    #[tokio::test]
    async fn a_split_set_that_yields_no_video_is_not_reported_as_an_empty_download() {
        let run = run_tree(
            &["quiet.harbor.7z.001", "quiet.harbor.7z.002"],
            vec![(
                "quiet.harbor.7z.001",
                TreeStep::Emits(vec![("readme.txt", b"txt")]),
            )],
            &ArchivePasswordCandidates::default(),
        )
        .await;

        let error = run.result.as_ref().unwrap_err();
        assert!(
            matches!(error, AppError::ArchiveExtractionPluginRequired { .. }),
            "{error:?}"
        );
        assert!(error.to_string().contains("split archive set"), "{error}");
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
        assert!(matches!(error, AppError::Repository(_)), "{error:?}");
        assert!(!error.to_string().contains("split"), "{error}");
        assert!(run.staging_dirs().is_empty());
    }
}

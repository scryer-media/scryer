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

/// If the download directory contains no video files but has archive files,
/// extract them to a hidden destination-side staging directory and return the path.
/// Returns `None` if no extraction was needed (video files exist directly).
pub async fn extract_archives_if_needed(
    dir: &Path,
    destination: Option<ArchiveExtractionDestination>,
    password: Option<&str>,
    archive_provider: Option<Arc<dyn ArchiveExtractorPluginProvider>>,
) -> AppResult<Option<PathBuf>> {
    let dir = dir.to_path_buf();
    let password = password.map(|s| s.to_string());
    let archive = {
        let dir = dir.clone();
        tokio::task::spawn_blocking(move || plan_archive_extraction(&dir))
            .await
            .map_err(|e| AppError::Repository(format!("archive detection task failed: {e}")))??
    };

    let Some((archive_path, archive_type)) = archive else {
        return Ok(None);
    };
    let Some(destination) = destination else {
        return Err(AppError::Validation(format!(
            "archive extraction requires a resolved import destination before staging output for {}",
            dir.display()
        )));
    };
    let workspace = ArchiveExtractionWorkspace::create(&destination).await?;

    info!(
        archive = %archive_path.display(),
        archive_type = archive_type.as_str(),
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

    // The plugin owns PAR2: it is handed the archive's own directory as a
    // read-only source preopen, finds any `.par2` set there itself, and repairs
    // into its writable output directory before (or instead of) extracting.
    let source_dir = archive_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let extraction = extract_with_archive_plugin(ArchivePluginExtraction {
        source_dir,
        archive_path,
        archive_type,
        format: archive_plugin_format_for_type(archive_type),
        password,
        provider,
        output_dir: workspace.output_dir.clone(),
    })
    .await;

    match extraction {
        Ok(Some(_)) => Ok(Some(workspace_root)),
        Ok(None) => {
            cleanup_extracted_dir(&workspace_root).await;
            Ok(None)
        }
        Err(error) => {
            cleanup_extracted_dir(&workspace_root).await;
            Err(error)
        }
    }
}

pub fn archive_extraction_would_be_needed(dir: &Path) -> AppResult<bool> {
    Ok(plan_archive_extraction(dir)?.is_some())
}

/// Check if an extraction error indicates a password-protected archive.
pub fn is_password_required_error(error: &AppError) -> bool {
    let msg = error.to_string().to_ascii_lowercase();
    msg.contains("password") || msg.contains("encrypted") || msg.contains("wrong password")
}

pub fn is_timeout_error(error: &AppError) -> bool {
    matches!(error, AppError::ArchiveExtractionTimedOut { .. })
}

fn plan_archive_extraction(dir: &Path) -> AppResult<Option<(PathBuf, ArchiveType)>> {
    if dir.is_file() {
        return Ok(archive_type_for_path(dir).map(|archive_type| (dir.to_path_buf(), archive_type)));
    }

    // If video files already exist, no extraction needed.
    if has_video_files(dir) {
        return Ok(None);
    }

    // Look for archives to extract.
    let archive = find_primary_archive(dir);
    let Some((archive_path, archive_type)) = archive else {
        return Ok(None);
    };

    Ok(Some((archive_path, archive_type)))
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
) -> AppResult<Option<PathBuf>> {
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
    handle_archive_plugin_response(archive_type, output_dir, response)
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

fn handle_archive_plugin_response(
    archive_type: ArchiveType,
    output_dir: PathBuf,
    response: ArchivePluginProcessResponse,
) -> AppResult<Option<PathBuf>> {
    match response.status {
        ArchivePluginStatus::Ok => {
            if let Err(error) = validate_archive_plugin_output(&output_dir, &response) {
                let _ = std::fs::remove_dir_all(&output_dir);
                return Err(error);
            }
            verify_extracted_video(archive_type, output_dir)
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

fn validate_archive_plugin_output(
    output_dir: &Path,
    response: &ArchivePluginProcessResponse,
) -> AppResult<()> {
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

    let mut entry_count = 0usize;
    let mut directory_count = 0usize;
    let mut file_count = 0usize;
    let mut expanded_bytes = 0u64;
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

    Ok(())
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

fn verify_extracted_video(
    archive_type: ArchiveType,
    output_dir: PathBuf,
) -> AppResult<Option<PathBuf>> {
    // Verify we got something useful out.
    if has_video_files(&output_dir) {
        info!(
            archive_type = archive_type.as_str(),
            output = %output_dir.display(),
            "archive extraction complete, video files found"
        );
        Ok(Some(output_dir))
    } else {
        info!(
            archive_type = archive_type.as_str(),
            "archive extracted but no video files found in output"
        );
        // Clean up the empty extraction.
        let _ = std::fs::remove_dir_all(&output_dir);
        Ok(None)
    }
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

/// Find the primary archive file in a directory. Prefers RAR, then 7z, then zip.
fn find_primary_archive(dir: &Path) -> Option<(PathBuf, ArchiveType)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return None;
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

    rar.sort_by_key(|path| rar_selection_key(path));
    sevenz.sort();
    zip.sort();

    if let Some(p) = rar.into_iter().next() {
        Some((p, ArchiveType::Rar))
    } else if let Some(p) = sevenz.into_iter().next() {
        Some((p, ArchiveType::SevenZip))
    } else {
        zip.into_iter().next().map(|p| (p, ArchiveType::Zip))
    }
}

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
        _ => None,
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
    fn find_primary_archive_prefers_rar() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("release.rar"), b"rar").unwrap();
        fs::write(dir.path().join("release.7z"), b"7z").unwrap();
        let (path, kind) = find_primary_archive(dir.path()).unwrap();
        assert!(path.extension().unwrap() == "rar");
        assert!(matches!(kind, ArchiveType::Rar));
    }

    #[test]
    fn find_primary_archive_finds_7z() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("release.7z"), b"7z").unwrap();
        let (_, kind) = find_primary_archive(dir.path()).unwrap();
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

            let (planned_path, archive_type) = plan_archive_extraction(&archive_path)
                .unwrap()
                .expect("direct archive file should require extraction");

            assert_eq!(planned_path, archive_path);
            assert_eq!(archive_type.as_str(), expected_type);
        }
    }

    #[test]
    fn find_primary_archive_finds_zip() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("release.zip"), b"zip").unwrap();
        let (_, kind) = find_primary_archive(dir.path()).unwrap();
        assert!(matches!(kind, ArchiveType::Zip));
    }

    #[test]
    fn find_primary_archive_none_for_video_only() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("movie.mkv"), b"video").unwrap();
        assert!(find_primary_archive(dir.path()).is_none());
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
        let result = extract_archives_if_needed(dir.path(), None, None, None)
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
            Some(ArchiveExtractionDestination::new(
                destination.path(),
                "rar-plugin-required",
            )),
            None,
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
            Some(ArchiveExtractionDestination::new(
                destination.path(),
                "7z-plugin-required",
            )),
            None,
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
            Some(ArchiveExtractionDestination::new(
                destination.path(),
                "7z-provider-missing-format",
            )),
            None,
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
            Some(ArchiveExtractionDestination::new(
                destination.path(),
                "7z-plain-extract",
            )),
            None,
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
            Some(ArchiveExtractionDestination::new(
                destination.path(),
                "zip-plain-extract",
            )),
            None,
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

        let error = extract_archives_if_needed(dir.path(), None, None, None)
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
            Some(ArchiveExtractionDestination::new(
                destination.path(),
                "import/with spaces",
            )),
            Some("secret"),
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
                assert_eq!(password.as_deref(), Some("secret"));
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
            Some(ArchiveExtractionDestination::new(
                destination.path(),
                "par2-plain-files",
            )),
            None,
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
            Some(ArchiveExtractionDestination::new(
                destination.path(),
                "par2-insufficient",
            )),
            None,
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
}

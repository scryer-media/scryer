//! Archive extraction for the import pipeline.
//!
//! Detects RAR, 7z, and zip archives in download directories. Extraction is
//! delegated to the optional archive extraction plugin after native PAR2
//! placement/repair handling.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crate::{AppError, AppResult, ArchiveExtractorPluginProvider};
use par2_rs::{
    DiskFileAccess, Par2FileSet, RepairOptions, Repairability, disk::PlacementFileAccess,
    execute_repair_with_options, placement::PlacementPlan, plan_repair, scan_placement,
    verify::verify_all,
};
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
const ARCHIVE_NORMALIZED_INPUT_DIR: &str = "normalized";
const ARCHIVE_REPAIR_INPUT_DIR: &str = "repair";
const ARCHIVE_STAGING_CREATE_ATTEMPTS: usize = 16;
const STALE_ARCHIVE_STAGING_AFTER: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_PLUGIN_OUTPUT_FILES: usize = 20_000;
const MAX_PLUGIN_OUTPUT_DIRECTORIES: usize = 20_000;
const MAX_PLUGIN_OUTPUT_ENTRIES: usize = MAX_PLUGIN_OUTPUT_FILES + MAX_PLUGIN_OUTPUT_DIRECTORIES;
const MAX_PLUGIN_OUTPUT_BYTES: u64 = 2 * 1024 * 1024 * 1024 * 1024;
const MAX_REPAIR_COPY_FALLBACK_BYTES: u64 = 64 * 1024 * 1024;

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

#[derive(Debug, Clone)]
struct ArchiveInputSet {
    source_dir: PathBuf,
    archive_path: PathBuf,
    cleanup_dirs: Vec<PathBuf>,
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

#[derive(Debug, Clone)]
struct Par2PlacementVerification {
    actual_by_canonical: HashMap<String, PathBuf>,
    state: NativePar2State,
    placement_move_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NativePar2State {
    Verified,
    Repairable,
    InsufficientRecoveryData,
    ResourceLimited,
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
    let input_set = match archive_type {
        ArchiveType::Rar | ArchiveType::Zip | ArchiveType::SevenZip
            if archive_provider.is_none() =>
        {
            cleanup_extracted_dir(&workspace_root).await;
            return Err(AppError::archive_extraction_plugin_required(Some(
                dir.to_string_lossy().into_owned(),
            )));
        }
        _ => {
            let source_dir = archive_path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."))
                .to_path_buf();
            let archive_path = archive_path.clone();
            let workspace = workspace.clone();
            tokio::task::spawn_blocking(move || {
                prepare_archive_input_set(&source_dir, &archive_path, archive_type, &workspace)
            })
            .await
            .map_err(|e| AppError::Repository(format!("archive PAR2 task failed: {e}")))
            .and_then(|result| result)
        }
    };
    let input_set = match input_set {
        Ok(input_set) => input_set,
        Err(error) => {
            cleanup_extracted_dir(&workspace_root).await;
            return Err(error);
        }
    };

    let input_cleanup_dirs = input_set.cleanup_dirs.clone();
    let extraction = {
        let format = archive_plugin_format_for_type(archive_type);
        if let Some(provider) = archive_provider {
            extract_with_archive_plugin(ArchivePluginExtraction {
                source_dir: input_set.source_dir,
                archive_path: input_set.archive_path,
                archive_type,
                format,
                password,
                provider,
                output_dir: workspace.output_dir.clone(),
            })
            .await
        } else {
            Err(AppError::archive_extraction_plugin_required(Some(
                dir.to_string_lossy().into_owned(),
            )))
        }
    };

    for cleanup_dir in input_cleanup_dirs {
        cleanup_extracted_dir(&cleanup_dir).await;
    }

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

fn cleanup_archive_artifacts_older_than_sync(parent: &Path, min_age: Duration) {
    let entries = match std::fs::read_dir(parent) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        if !is_archive_staging_dir(&path) && !is_archive_write_probe_file(&path) {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
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
                let _ = std::fs::remove_dir_all(path);
            } else {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

fn cleanup_archive_staging_dirs_sync(dirs: &[PathBuf]) {
    for dir in dirs {
        if is_archive_staging_dir(dir) {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

fn prepare_archive_input_set(
    source_dir: &Path,
    archive_path: &Path,
    archive_type: ArchiveType,
    workspace: &ArchiveExtractionWorkspace,
) -> AppResult<ArchiveInputSet> {
    let par2_paths = find_par2_paths(source_dir);
    if par2_paths.is_empty() {
        return Ok(ArchiveInputSet {
            source_dir: source_dir.to_path_buf(),
            archive_path: archive_path.to_path_buf(),
            cleanup_dirs: Vec::new(),
        });
    }

    let verification = verify_par2_with_placement(source_dir, &par2_paths)?;
    match verification.state {
        NativePar2State::Verified if verification.placement_move_count == 0 => {
            let archive_path = verification.archive_path_for(archive_type, archive_path)?;
            Ok(ArchiveInputSet {
                source_dir: source_dir.to_path_buf(),
                archive_path,
                cleanup_dirs: Vec::new(),
            })
        }
        NativePar2State::Verified => {
            let archive_hint = verification.canonical_hint_for_actual_path(archive_path);
            let (normalized_dir, cleanup_dirs) =
                create_par2_staging_dir(source_dir, workspace, ARCHIVE_NORMALIZED_INPUT_DIR)?;
            let result = (|| {
                stage_par2_input_set(
                    source_dir,
                    &par2_paths,
                    &verification,
                    &normalized_dir,
                    StageMode::CleanNormalize,
                )?;
                let staged_par2_paths = remap_par2_paths(&par2_paths, &normalized_dir)?;
                let staged_verification =
                    verify_par2_with_placement(&normalized_dir, &staged_par2_paths)?;
                if !matches!(staged_verification.state, NativePar2State::Verified) {
                    return Err(AppError::Repository(
                        "PAR2 normalized archive staging did not verify cleanly".to_string(),
                    ));
                }
                let archive_path =
                    staged_verification.archive_path_for(archive_type, &archive_hint)?;
                Ok(ArchiveInputSet {
                    source_dir: normalized_dir,
                    archive_path,
                    cleanup_dirs: cleanup_dirs.clone(),
                })
            })();
            if result.is_err() {
                cleanup_archive_staging_dirs_sync(&cleanup_dirs);
            }
            result
        }
        NativePar2State::Repairable => {
            let archive_hint = verification.canonical_hint_for_actual_path(archive_path);
            let (repair_dir, cleanup_dirs) =
                create_par2_staging_dir(source_dir, workspace, ARCHIVE_REPAIR_INPUT_DIR)?;
            let result = (|| {
                stage_par2_input_set(
                    source_dir,
                    &par2_paths,
                    &verification,
                    &repair_dir,
                    StageMode::Repair,
                )?;
                let staged_par2_paths = remap_par2_paths(&par2_paths, &repair_dir)?;
                repair_par2_in_place(&repair_dir, &staged_par2_paths)?;
                let repaired = verify_par2_with_placement(&repair_dir, &staged_par2_paths)?;
                if !matches!(repaired.state, NativePar2State::Verified) {
                    return Err(AppError::Repository(
                        "PAR2 repair completed but verification still requires repair".to_string(),
                    ));
                }
                let archive_path = repaired.archive_path_for(archive_type, &archive_hint)?;
                Ok(ArchiveInputSet {
                    source_dir: repair_dir,
                    archive_path,
                    cleanup_dirs: cleanup_dirs.clone(),
                })
            })();
            if result.is_err() {
                cleanup_archive_staging_dirs_sync(&cleanup_dirs);
            }
            result
        }
        NativePar2State::InsufficientRecoveryData => Err(AppError::Validation(
            "PAR2 set does not have enough recovery data to repair the archive".to_string(),
        )),
        NativePar2State::ResourceLimited => {
            tracing::warn!(
                source_dir = %source_dir.display(),
                archive = %archive_path.display(),
                "PAR2 verification exceeded resource limits; attempting archive extraction without PAR2 repair"
            );
            Ok(ArchiveInputSet {
                source_dir: source_dir.to_path_buf(),
                archive_path: archive_path.to_path_buf(),
                cleanup_dirs: Vec::new(),
            })
        }
    }
}

fn verify_par2_with_placement(
    source_dir: &Path,
    par2_paths: &[PathBuf],
) -> AppResult<Par2PlacementVerification> {
    let set = Par2FileSet::from_paths(par2_paths)
        .map_err(|error| AppError::Repository(format!("failed to load PAR2 set: {error}")))?;
    validate_par2_file_names(&set)?;
    let plan = scan_placement(source_dir, &set).map_err(|error| {
        AppError::Repository(format!("failed to scan PAR2 file placement: {error}"))
    })?;
    if !plan.conflicts.is_empty() {
        return Err(AppError::Validation(format!(
            "PAR2 placement is ambiguous for {} file(s); refusing to guess archive order",
            plan.conflicts.len()
        )));
    }

    let access = PlacementFileAccess::from_plan(source_dir.to_path_buf(), &set, &plan);
    let verification = verify_all(&set, &access);
    let state = match verification.repairable {
        Repairability::NotNeeded => NativePar2State::Verified,
        Repairability::Repairable { .. } => NativePar2State::Repairable,
        Repairability::Insufficient { .. } => NativePar2State::InsufficientRecoveryData,
        Repairability::ResourceLimited { .. } => NativePar2State::ResourceLimited,
    };

    Ok(Par2PlacementVerification {
        actual_by_canonical: par2_actual_paths_by_canonical(source_dir, &set, &plan),
        state,
        placement_move_count: plan.renames.len() + plan.swaps.len().saturating_mul(2),
    })
}

fn validate_par2_file_names(set: &Par2FileSet) -> AppResult<()> {
    let mut seen = std::collections::HashSet::new();
    for description in set.files.values() {
        let relative = safe_par2_relative_path(&description.filename)?;
        if !seen.insert(relative) {
            return Err(AppError::Validation(format!(
                "PAR2 metadata contains duplicate file path '{}'",
                description.filename
            )));
        }
    }
    Ok(())
}

fn par2_actual_paths_by_canonical(
    source_dir: &Path,
    set: &Par2FileSet,
    plan: &PlacementPlan,
) -> HashMap<String, PathBuf> {
    let mut actual = HashMap::new();
    for description in set.files.values() {
        actual.insert(
            description.filename.clone(),
            source_dir.join(safe_par2_relative_path_lossy(&description.filename)),
        );
    }
    for (left, right) in &plan.swaps {
        actual.insert(
            left.correct_name.clone(),
            source_dir.join(safe_par2_relative_path_lossy(&left.current_name)),
        );
        actual.insert(
            right.correct_name.clone(),
            source_dir.join(safe_par2_relative_path_lossy(&right.current_name)),
        );
    }
    for entry in &plan.renames {
        actual.insert(
            entry.correct_name.clone(),
            source_dir.join(safe_par2_relative_path_lossy(&entry.current_name)),
        );
    }
    actual
}

fn safe_par2_relative_path_lossy(path: &str) -> PathBuf {
    safe_par2_relative_path(path).unwrap_or_else(|_| PathBuf::from(path.replace(['/', '\\'], "_")))
}

fn safe_par2_relative_path(path: &str) -> AppResult<PathBuf> {
    let path = Path::new(path);
    if path.is_absolute() {
        return Err(AppError::Validation(format!(
            "PAR2 filename '{}' is absolute",
            path.display()
        )));
    }
    let mut relative = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => relative.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(AppError::Validation(format!(
                    "PAR2 filename '{}' is unsafe",
                    path.display()
                )));
            }
        }
    }
    if relative.as_os_str().is_empty() {
        return Err(AppError::Validation("PAR2 filename is empty".to_string()));
    }
    Ok(relative)
}

#[derive(Debug, Clone, Copy)]
enum StageMode {
    CleanNormalize,
    Repair,
}

fn create_par2_staging_dir(
    source_dir: &Path,
    workspace: &ArchiveExtractionWorkspace,
    purpose: &str,
) -> AppResult<(PathBuf, Vec<PathBuf>)> {
    cleanup_archive_artifacts_older_than_sync(source_dir, STALE_ARCHIVE_STAGING_AFTER);
    for _ in 0..ARCHIVE_STAGING_CREATE_ATTEMPTS {
        let source_staging_dir = source_dir.join(format!(
            "{ARCHIVE_STAGING_PREFIX}{}-{purpose}",
            short_staging_suffix()
        ));
        match std::fs::create_dir(&source_staging_dir) {
            Ok(()) => return Ok((source_staging_dir.clone(), vec![source_staging_dir])),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                tracing::warn!(
                    source_dir = %source_dir.display(),
                    staging_dir = %source_staging_dir.display(),
                    %error,
                    "could not create source-side PAR2 staging directory; falling back to destination workspace"
                );
                break;
            }
        }
    }

    Ok((workspace.root.join(purpose), Vec::new()))
}

fn stage_par2_input_set(
    source_dir: &Path,
    par2_paths: &[PathBuf],
    verification: &Par2PlacementVerification,
    staging_dir: &Path,
    mode: StageMode,
) -> AppResult<()> {
    std::fs::create_dir_all(staging_dir).map_err(|error| {
        AppError::Repository(format!(
            "failed to create PAR2 staging directory {}: {error}",
            staging_dir.display()
        ))
    })?;

    for (canonical, actual_path) in &verification.actual_by_canonical {
        let relative = safe_par2_relative_path(canonical)?;
        if !actual_path.exists() {
            continue;
        }
        stage_par2_file(actual_path, &staging_dir.join(relative), mode)?;
    }

    for par2_path in par2_paths {
        let file_name = par2_path.file_name().ok_or_else(|| {
            AppError::Validation(format!(
                "PAR2 path '{}' has no file name",
                par2_path.display()
            ))
        })?;
        stage_par2_file(
            par2_path,
            &staging_dir.join(file_name),
            StageMode::CleanNormalize,
        )?;
    }

    tracing::info!(
        source_dir = %source_dir.display(),
        staging_dir = %staging_dir.display(),
        mode = ?mode,
        "staged PAR2 archive inputs"
    );
    Ok(())
}

fn stage_par2_file(source: &Path, destination: &Path, mode: StageMode) -> AppResult<()> {
    let metadata = std::fs::symlink_metadata(source).map_err(|error| {
        AppError::Repository(format!(
            "failed to inspect PAR2 input {}: {error}",
            source.display()
        ))
    })?;
    if metadata.file_type().is_symlink() {
        return Err(AppError::Validation(format!(
            "PAR2 staging refuses symbolic link '{}'",
            source.display()
        )));
    }
    if !metadata.is_file() {
        return Ok(());
    }
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            AppError::Repository(format!(
                "failed to create PAR2 staging parent {}: {error}",
                parent.display()
            ))
        })?;
    }

    let result = match mode {
        StageMode::CleanNormalize => stage_clean_file(source, destination, metadata.len()),
        StageMode::Repair => stage_repair_file(source, destination, metadata.len(), true),
    };
    result.map_err(|error| {
        AppError::Repository(format!(
            "failed to stage PAR2 input '{}' to '{}': {error}",
            source.display(),
            destination.display()
        ))
    })
}

fn remap_par2_paths(par2_paths: &[PathBuf], staging_dir: &Path) -> AppResult<Vec<PathBuf>> {
    par2_paths
        .iter()
        .map(|path| {
            let file_name = path.file_name().ok_or_else(|| {
                AppError::Validation(format!("PAR2 path '{}' has no file name", path.display()))
            })?;
            Ok(staging_dir.join(file_name))
        })
        .collect()
}

fn repair_par2_in_place(source_dir: &Path, par2_paths: &[PathBuf]) -> AppResult<()> {
    let set = Par2FileSet::from_paths(par2_paths).map_err(|error| {
        AppError::Repository(format!("failed to load staged PAR2 set: {error}"))
    })?;
    validate_par2_file_names(&set)?;
    let mut access = DiskFileAccess::new(source_dir.to_path_buf(), &set);
    let verification = verify_all(&set, &access);
    match &verification.repairable {
        Repairability::NotNeeded => return Ok(()),
        Repairability::Insufficient { .. } => {
            return Err(AppError::Validation(
                "PAR2 set does not have enough recovery data".to_string(),
            ));
        }
        Repairability::ResourceLimited { .. } => {
            return Err(AppError::Repository(
                "PAR2 repair exceeded resource limits".to_string(),
            ));
        }
        Repairability::Repairable { .. } => {}
    }

    let plan = plan_repair(&set, &verification)
        .map_err(|error| AppError::Repository(format!("failed to plan PAR2 repair: {error}")))?;
    execute_repair_with_options(&plan, &set, &mut access, &RepairOptions::default())
        .map_err(|error| AppError::Repository(format!("PAR2 repair failed: {error}")))?;

    let post = verify_all(&set, &access);
    if post.needs_repair() {
        return Err(AppError::Repository(
            "PAR2 repair did not verify cleanly after reconstruction".to_string(),
        ));
    }
    Ok(())
}

fn is_old_rar_volume_extension(ext: &str) -> bool {
    let mut chars = ext.chars();
    matches!(chars.next(), Some('r'..='z')) && ext.len() >= 3 && chars.all(|ch| ch.is_ascii_digit())
}

fn stage_clean_file(source: &Path, destination: &Path, len: u64) -> std::io::Result<()> {
    // Clean normalization is read-only and the plugin receives source preopens as
    // read-only, so hardlinks are safe here. If that sandbox contract changes,
    // this path must switch to COW/copy staging before extraction.
    match std::fs::hard_link(source, destination) {
        Ok(()) => Ok(()),
        Err(_) => stage_repair_file(source, destination, len, false),
    }
}

fn stage_repair_file(
    source: &Path,
    destination: &Path,
    len: u64,
    allow_large_copy: bool,
) -> std::io::Result<()> {
    match clone_file_cow(source, destination) {
        Ok(()) => Ok(()),
        Err(error) if !allow_large_copy && len > MAX_REPAIR_COPY_FALLBACK_BYTES => {
            Err(std::io::Error::new(
                error.kind(),
                format!(
                    "copy-on-write staging is unavailable and file is larger than {} bytes",
                    MAX_REPAIR_COPY_FALLBACK_BYTES
                ),
            ))
        }
        Err(_) => std::fs::copy(source, destination).map(|_| ()),
    }
}

#[cfg(target_os = "linux")]
fn clone_file_cow(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::fs::OpenOptions;
    use std::os::fd::AsRawFd;

    const FICLONE: libc::Ioctl = 0x4004_9409;

    let source_file = std::fs::File::open(source)?;
    let destination_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    let result = unsafe {
        libc::ioctl(
            destination_file.as_raw_fd(),
            FICLONE,
            source_file.as_raw_fd(),
        )
    };
    if result == 0 {
        Ok(())
    } else {
        let error = std::io::Error::last_os_error();
        let _ = std::fs::remove_file(destination);
        Err(error)
    }
}

#[cfg(target_os = "macos")]
fn clone_file_cow(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let source = CString::new(source.as_os_str().as_bytes()).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "source path contains NUL")
    })?;
    let destination = CString::new(destination.as_os_str().as_bytes()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "destination path contains NUL",
        )
    })?;
    let result = unsafe { libc::clonefile(source.as_ptr(), destination.as_ptr(), 0) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn clone_file_cow(_source: &Path, _destination: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "copy-on-write file cloning is not implemented for this platform",
    ))
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
            let message = response
                .message
                .or(response.error_code)
                .unwrap_or_else(|| "archive plugin extraction failed".to_string());
            Err(AppError::Repository(message))
        }
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

impl Par2PlacementVerification {
    fn canonical_hint_for_actual_path(&self, hint: &Path) -> PathBuf {
        let hint_name = hint.file_name().and_then(|name| name.to_str());
        for (canonical, actual_path) in &self.actual_by_canonical {
            let actual_matches = actual_path == hint
                || hint_name.is_some_and(|hint_name| {
                    actual_path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|actual_name| actual_name.eq_ignore_ascii_case(hint_name))
                });
            if actual_matches {
                return PathBuf::from(canonical);
            }
        }
        hint.to_path_buf()
    }

    fn archive_path_for(&self, archive_type: ArchiveType, hint: &Path) -> AppResult<PathBuf> {
        match archive_type {
            ArchiveType::Rar => self.rar_first_volume_path(hint),
            ArchiveType::SevenZip => self.single_archive_path(hint, "7z"),
            ArchiveType::Zip => self.single_archive_path(hint, "zip"),
        }
    }

    fn rar_first_volume_path(&self, hint: &Path) -> AppResult<PathBuf> {
        let mut candidates = Vec::new();
        for (canonical, actual_path) in &self.actual_by_canonical {
            let Some(canonical_name) = canonical_file_name(canonical) else {
                continue;
            };
            let Some((group, index)) =
                rar_volume_info_from_name(&canonical_name.to_ascii_lowercase())
            else {
                continue;
            };
            candidates.push(RarVolumeCandidate {
                group,
                index,
                canonical_name,
                actual_path: actual_path.clone(),
            });
        }
        if candidates.is_empty() {
            return Err(AppError::Validation(
                "PAR2 metadata does not describe a RAR archive".to_string(),
            ));
        }

        let group = archive_hint_group(hint, &candidates)
            .or_else(|| single_rar_group(&candidates))
            .ok_or_else(|| {
                AppError::Validation(
                    "PAR2 metadata describes multiple RAR archive sets and the requested archive did not identify one"
                        .to_string(),
                )
            })?;
        let mut selected = candidates
            .into_iter()
            .filter(|candidate| candidate.group == group)
            .collect::<Vec<_>>();
        selected.sort_by(|left, right| {
            left.index
                .cmp(&right.index)
                .then_with(|| left.canonical_name.cmp(&right.canonical_name))
        });
        let Some(first) = selected.first() else {
            return Err(AppError::Validation(
                "PAR2 metadata did not identify a RAR archive set".to_string(),
            ));
        };
        if first.index != 0 {
            return Err(AppError::Validation(
                "PAR2 metadata did not identify the first RAR volume".to_string(),
            ));
        }
        let mut previous = None;
        for candidate in &selected {
            if previous == Some(candidate.index) {
                return Err(AppError::Validation(
                    "PAR2 metadata maps multiple files to the same RAR volume index".to_string(),
                ));
            }
            previous = Some(candidate.index);
        }
        Ok(first.actual_path.clone())
    }

    fn single_archive_path(&self, hint: &Path, extension: &str) -> AppResult<PathBuf> {
        let hint_name = hint
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.to_ascii_lowercase());
        let mut candidates = self
            .actual_by_canonical
            .iter()
            .filter_map(|(canonical, actual_path)| {
                let canonical_name = canonical_file_name(canonical)?;
                canonical_name
                    .to_ascii_lowercase()
                    .ends_with(&format!(".{extension}"))
                    .then_some((canonical_name, actual_path.clone()))
            })
            .collect::<Vec<_>>();

        if let Some(hint_name) = hint_name
            && let Some((_, actual_path)) =
                candidates.iter().find(|(canonical_name, actual_path)| {
                    canonical_name.eq_ignore_ascii_case(&hint_name)
                        || actual_path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .is_some_and(|name| name.eq_ignore_ascii_case(&hint_name))
                })
        {
            return Ok(actual_path.clone());
        }

        candidates.sort_by(|left, right| left.0.cmp(&right.0));
        match candidates.as_slice() {
            [(_, path)] => Ok(path.clone()),
            [] => Err(AppError::Validation(format!(
                "PAR2 metadata does not describe a {extension} archive"
            ))),
            _ => Err(AppError::Validation(format!(
                "PAR2 metadata describes multiple {extension} archives and the requested path did not identify one"
            ))),
        }
    }
}

#[derive(Debug)]
struct RarVolumeCandidate {
    group: String,
    index: usize,
    canonical_name: String,
    actual_path: PathBuf,
}

fn archive_hint_group(hint: &Path, candidates: &[RarVolumeCandidate]) -> Option<String> {
    let hint_name = hint.file_name().and_then(|name| name.to_str())?;
    candidates
        .iter()
        .find(|candidate| {
            candidate.canonical_name.eq_ignore_ascii_case(hint_name)
                || candidate
                    .actual_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.eq_ignore_ascii_case(hint_name))
        })
        .map(|candidate| candidate.group.clone())
}

fn single_rar_group(candidates: &[RarVolumeCandidate]) -> Option<String> {
    let mut groups = candidates
        .iter()
        .map(|candidate| candidate.group.clone())
        .collect::<Vec<_>>();
    groups.sort();
    groups.dedup();
    match groups.as_slice() {
        [group] => Some(group.clone()),
        _ => None,
    }
}

fn canonical_file_name(path: &str) -> Option<String> {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(ToOwned::to_owned)
}

fn find_par2_paths(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut par2 = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if ext == "par2" {
            par2.push(path);
        }
    }
    par2.sort();
    par2
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
    use std::collections::HashMap;
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
    fn par2_rar_resolution_uses_first_volume_even_when_hint_is_later_volume() {
        let dir = tempfile::tempdir().unwrap();
        let part1 = dir.path().join("obfuscated-a.bin");
        let part5 = dir.path().join("obfuscated-e.bin");
        let verification = Par2PlacementVerification {
            actual_by_canonical: HashMap::from([
                ("show.part1.rar".to_string(), part1.clone()),
                ("show.part5.rar".to_string(), part5.clone()),
            ]),
            state: NativePar2State::Verified,
            placement_move_count: 2,
        };

        let resolved = verification
            .archive_path_for(ArchiveType::Rar, &part5)
            .unwrap();

        assert_eq!(resolved, part1);
    }

    #[test]
    fn par2_canonical_hint_maps_obfuscated_actual_name() {
        let dir = tempfile::tempdir().unwrap();
        let actual = dir.path().join("obfuscated-e.bin");
        let verification = Par2PlacementVerification {
            actual_by_canonical: HashMap::from([("show.part5.rar".to_string(), actual.clone())]),
            state: NativePar2State::Verified,
            placement_move_count: 1,
        };

        assert_eq!(
            verification.canonical_hint_for_actual_path(&actual),
            PathBuf::from("show.part5.rar")
        );
    }

    #[test]
    fn par2_resolution_does_not_fallback_to_hint_when_metadata_has_no_matching_archive() {
        let dir = tempfile::tempdir().unwrap();
        let hint = dir.path().join("release.zip");
        let verification = Par2PlacementVerification {
            actual_by_canonical: HashMap::from([(
                "release.nfo".to_string(),
                dir.path().join("release.nfo"),
            )]),
            state: NativePar2State::Verified,
            placement_move_count: 0,
        };

        let error = verification
            .archive_path_for(ArchiveType::Zip, &hint)
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("does not describe a zip archive"),
            "{error}"
        );
    }

    #[test]
    fn par2_rar_resolution_rejects_multiple_sets_without_identifying_hint() {
        let dir = tempfile::tempdir().unwrap();
        let verification = Par2PlacementVerification {
            actual_by_canonical: HashMap::from([
                ("show-a.part1.rar".to_string(), dir.path().join("a1.bin")),
                ("show-b.part1.rar".to_string(), dir.path().join("b1.bin")),
            ]),
            state: NativePar2State::Verified,
            placement_move_count: 2,
        };

        let error = verification
            .archive_path_for(ArchiveType::Rar, &dir.path().join("unknown.part5.rar"))
            .unwrap_err();

        assert!(
            error.to_string().contains("multiple RAR archive sets"),
            "{error}"
        );
    }

    #[test]
    fn par2_relative_path_rejects_escape_before_filesystem_access() {
        assert!(safe_par2_relative_path("../escape.rar").is_err());
        assert!(safe_par2_relative_path("/tmp/escape.rar").is_err());
        assert!(safe_par2_relative_path("nested/volume.rar").is_ok());
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

    #[test]
    fn clean_staging_refuses_large_copy_when_clone_is_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.bin");
        let destination = dir.path().join("destination.bin");
        fs::write(&source, b"source").unwrap();
        fs::write(&destination, b"existing").unwrap();

        let error = stage_repair_file(
            &source,
            &destination,
            MAX_REPAIR_COPY_FALLBACK_BYTES + 1,
            false,
        )
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("copy-on-write staging is unavailable"),
            "{error}"
        );
        assert_eq!(fs::read(&destination).unwrap(), b"existing");
    }

    #[test]
    fn repair_staging_allows_large_copy_when_clone_is_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.bin");
        let destination = dir.path().join("destination.bin");
        fs::write(&source, b"source").unwrap();
        fs::write(&destination, b"existing").unwrap();

        stage_repair_file(
            &source,
            &destination,
            MAX_REPAIR_COPY_FALLBACK_BYTES + 1,
            true,
        )
        .unwrap();

        assert_eq!(fs::read(&destination).unwrap(), b"source");
    }

    #[test]
    fn par2_staging_prefers_source_filesystem() {
        let source = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        let workspace = ArchiveExtractionWorkspace {
            root: destination.path().join("workspace"),
            output_dir: destination.path().join("workspace").join("out"),
        };

        let (staging_dir, cleanup_dirs) =
            create_par2_staging_dir(source.path(), &workspace, ARCHIVE_REPAIR_INPUT_DIR).unwrap();

        assert!(staging_dir.starts_with(source.path()));
        assert_eq!(cleanup_dirs, vec![staging_dir.clone()]);
        assert!(is_archive_staging_dir(&staging_dir));
        std::fs::remove_dir_all(staging_dir).unwrap();
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
        assert!(!extracted.join(ARCHIVE_REPAIR_INPUT_DIR).exists());

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
}

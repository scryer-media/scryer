use crate::{AppError, AppResult};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;
use tracing::{info, warn};

pub const RECYCLE_MANIFEST_SCHEMA: &str = "scryer.recycle-entry.v1";
pub const RECYCLE_STATUS_PENDING: &str = "pending";
pub const RECYCLE_STATUS_COMMITTED: &str = "committed";
pub const RECYCLE_STATUS_QUARANTINED: &str = "quarantined";

const RECYCLE_ROOT_SENTINEL: &str = ".scryer-recycle-root";
const DEFAULT_RETENTION_DAYS: u32 = 7;
const RECYCLE_DIR_NAME: &str = ".scryer-recycle";

/// Configuration for the recycle bin, resolved from application settings.
#[derive(Clone, Debug)]
pub struct RecycleBinConfig {
    pub enabled: bool,
    pub base_path: PathBuf,
    pub retention_days: u32,
    pub cleanup_enabled: bool,
    pub validation_error: Option<String>,
    /// Allowlist of configured media roots the source file must live under before
    /// it may be recycled or (when recycling is disabled) permanently deleted.
    /// An empty list means the roots are unknown (legacy/misconfigured) and the
    /// source-removing operation must fail closed.
    pub source_roots: Vec<PathBuf>,
}

/// Metadata written alongside each recycled file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecycleManifest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_operation_id: Option<String>,
    pub recycled_at: String,
    pub original_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_file_id: Option<String>,
    pub size_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_root: Option<String>,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replacement_file_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replacement_path: Option<String>,
}

/// Result of a successful recycle operation.
#[derive(Debug, Clone)]
pub struct RecycleResult {
    pub entry_id: String,
    pub entry_dir: PathBuf,
    pub recycled_path: PathBuf,
    pub manifest_path: PathBuf,
}

pub(crate) struct ReplacedMediaRecycleMetadata<'a> {
    pub original_path: &'a str,
    pub original_file_id: &'a str,
    pub size_bytes: u64,
    pub title_id: &'a str,
    pub media_root: Option<&'a str>,
}

/// A committed recycle entry that passed local recycle-root checks.
#[derive(Debug, Clone)]
pub struct CommittedRecycleEntry {
    pub entry_dir: PathBuf,
    pub manifest: RecycleManifest,
}

/// A recycle bin entry for listing purposes.
#[derive(Debug, Clone)]
pub struct RecycleEntry {
    /// Directory name, e.g. "20260307_120015437_abc123".
    pub entry_id: String,
    pub manifest: RecycleManifest,
    /// Which media root this entry belongs to.
    pub media_root: String,
}

pub fn validate_recycle_entry_id(entry_id: &str) -> AppResult<()> {
    if entry_id.is_empty()
        || entry_id == "."
        || entry_id == ".."
        || !entry_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Err(AppError::Validation(
            "recycle entry id is not a valid opaque id".into(),
        ));
    }
    Ok(())
}

impl RecycleManifest {
    pub fn pending_upgrade(
        original_path: String,
        original_file_id: String,
        size_bytes: u64,
        title_id: String,
        media_root: Option<String>,
    ) -> Self {
        Self {
            schema: None,
            entry_id: None,
            source_operation_id: Some(scryer_domain::Id::new().0),
            recycled_at: Utc::now().to_rfc3339(),
            original_path,
            original_file_id: Some(original_file_id),
            size_bytes,
            title_id: Some(title_id),
            media_root,
            reason: "upgrade_replaced".to_string(),
            status: Some(RECYCLE_STATUS_PENDING.to_string()),
            replacement_file_id: None,
            replacement_path: None,
        }
    }

    fn is_schema_current(&self) -> bool {
        self.schema.as_deref() == Some(RECYCLE_MANIFEST_SCHEMA)
    }

    fn is_committed(&self) -> bool {
        self.status.as_deref() == Some(RECYCLE_STATUS_COMMITTED)
    }

    fn is_quarantined(&self) -> bool {
        self.status.as_deref() == Some(RECYCLE_STATUS_QUARANTINED)
    }

    pub fn original_path_buf(&self) -> PathBuf {
        crate::stored_paths::stored_path_to_path_buf(&self.original_path)
    }
}

fn manifest_path(entry_dir: &Path) -> PathBuf {
    entry_dir.join("manifest.json")
}

fn sentinel_path(config: &RecycleBinConfig) -> PathBuf {
    config.base_path.join(RECYCLE_ROOT_SENTINEL)
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            std::path::Component::RootDir => normalized.push(component.as_os_str()),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            std::path::Component::Normal(segment) => normalized.push(segment),
        }
    }
    normalized
}

fn generated_entry_id(entry_id: &str) -> bool {
    let mut parts = entry_id.split('_');
    let Some(date) = parts.next() else {
        return false;
    };
    let Some(time) = parts.next() else {
        return false;
    };
    let Some(suffix) = parts.next() else {
        return false;
    };
    parts.next().is_none()
        && date.len() == 8
        && date.bytes().all(|byte| byte.is_ascii_digit())
        && time.len() == 9
        && time.bytes().all(|byte| byte.is_ascii_digit())
        && suffix.len() == 6
        && suffix.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

fn cleanup_ready(config: &RecycleBinConfig) -> bool {
    config.enabled
        && config.cleanup_enabled
        && config.base_path.exists()
        && sentinel_path(config).exists()
}

async fn ensure_recycle_root(config: &RecycleBinConfig) -> AppResult<()> {
    tokio::fs::create_dir_all(&config.base_path)
        .await
        .map_err(|e| {
            AppError::Repository(format!(
                "failed to create recycle directory {}: {}",
                config.base_path.display(),
                e
            ))
        })?;

    let sentinel = sentinel_path(config);
    if !sentinel.exists() {
        tokio::fs::write(&sentinel, RECYCLE_MANIFEST_SCHEMA.as_bytes())
            .await
            .map_err(|e| {
                AppError::Repository(format!(
                    "failed to write recycle root sentinel {}: {}",
                    sentinel.display(),
                    e
                ))
            })?;
    }
    Ok(())
}

fn trusted_committed_entry(
    config: &RecycleBinConfig,
    entry_dir: &Path,
    manifest: &RecycleManifest,
) -> Result<(), String> {
    if !cleanup_ready(config) {
        return Err(config
            .validation_error
            .clone()
            .unwrap_or_else(|| "recycle root is not enabled for cleanup".to_string()));
    }

    let entry_name = entry_dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "recycle entry has no valid directory name".to_string())?;
    validate_recycle_entry_id(entry_name).map_err(|error| error.to_string())?;
    let entry_metadata = std::fs::symlink_metadata(entry_dir)
        .map_err(|error| format!("failed to stat recycle entry directory: {error}"))?;
    if entry_metadata.file_type().is_symlink() {
        return Err("recycle entry directory is a symlink".to_string());
    }
    if !entry_metadata.is_dir() {
        return Err("recycle entry is not a directory".to_string());
    }
    if !generated_entry_id(entry_name) {
        return Err("recycle entry directory was not generated by Scryer".to_string());
    }
    if !manifest.is_schema_current() {
        return Err("recycle manifest schema is missing or unsupported".to_string());
    }
    if manifest.entry_id.as_deref() != Some(entry_name) {
        return Err("recycle manifest entry id does not match directory".to_string());
    }
    if !manifest.is_committed() {
        return Err("recycle entry is not committed".to_string());
    }

    let expected_parent = normalize_path(&config.base_path);
    let actual_parent = entry_dir
        .parent()
        .map(normalize_path)
        .ok_or_else(|| "recycle entry has no parent directory".to_string())?;
    if actual_parent != expected_parent {
        return Err("recycle entry is outside the configured recycle root".to_string());
    }

    Ok(())
}

async fn quarantine_untrusted_committed_entry(
    config: &RecycleBinConfig,
    entry_dir: &Path,
    manifest: &RecycleManifest,
    reason: &str,
) -> AppResult<bool> {
    if !cleanup_ready(config) || !manifest.is_committed() {
        return Ok(false);
    }

    let Some(parent) = entry_dir.parent() else {
        return Ok(false);
    };
    if normalize_path(parent) != normalize_path(&config.base_path) {
        return Ok(false);
    }

    warn!(
        path = %entry_dir.display(),
        reason = %reason,
        "quarantining untrusted committed recycle entry"
    );
    quarantine_entry(entry_dir, manifest, reason).await?;
    Ok(true)
}

async fn read_manifest(entry_dir: &Path) -> AppResult<Option<RecycleManifest>> {
    let path = manifest_path(entry_dir);
    if !path.exists() {
        return Ok(None);
    }
    let manifest_bytes = tokio::fs::read(&path).await.map_err(|e| {
        AppError::Repository(format!(
            "failed to read recycle manifest {}: {}",
            path.display(),
            e
        ))
    })?;
    let manifest: RecycleManifest = serde_json::from_slice(&manifest_bytes).map_err(|e| {
        AppError::Repository(format!(
            "failed to parse recycle manifest {}: {}",
            path.display(),
            e
        ))
    })?;
    Ok(Some(manifest))
}

async fn write_manifest(entry_dir: &Path, manifest: &RecycleManifest) -> AppResult<()> {
    let path = manifest_path(entry_dir);
    let manifest_json = serde_json::to_string_pretty(manifest).map_err(|e| {
        AppError::Repository(format!("failed to serialize recycle manifest: {}", e))
    })?;
    tokio::fs::write(&path, manifest_json.as_bytes())
        .await
        .map_err(|e| {
            AppError::Repository(format!(
                "failed to write recycle manifest {}: {}",
                path.display(),
                e
            ))
        })
}

/// Move a file to the recycle bin instead of deleting it.
///
/// If the recycle bin is disabled or its cleanup path is invalid, returns an error instead
/// of deleting user content directly.
///
/// If the file does not exist, returns `Ok(None)` without error (matches the current
/// `ErrorKind::NotFound` handling in callers).
pub async fn recycle_file(
    config: &RecycleBinConfig,
    source_path: &Path,
    manifest: RecycleManifest,
) -> AppResult<Option<RecycleResult>> {
    recycle_file_inner(config, source_path, source_path, manifest, true).await
}

/// Moves a file into a pending recycle entry that retention housekeeping must
/// ignore until the caller has completed the surrounding move transaction.
pub(crate) async fn recycle_file_pending(
    config: &RecycleBinConfig,
    source_path: &Path,
    manifest: RecycleManifest,
) -> AppResult<Option<RecycleResult>> {
    recycle_file_inner(config, source_path, source_path, manifest, false).await
}

pub(crate) async fn recycle_replaced_media_file(
    config: &RecycleBinConfig,
    source_path: &Path,
    metadata: ReplacedMediaRecycleMetadata<'_>,
    commit_after_move: bool,
) -> AppResult<Option<RecycleResult>> {
    let manifest = RecycleManifest::pending_upgrade(
        metadata.original_path.to_string(),
        metadata.original_file_id.to_string(),
        metadata.size_bytes,
        metadata.title_id.to_string(),
        metadata.media_root.map(str::to_string),
    );
    let payload_name_path = crate::stored_paths::stored_path_to_path_buf(metadata.original_path);
    recycle_file_inner(
        config,
        source_path,
        &payload_name_path,
        manifest,
        commit_after_move,
    )
    .await
}

async fn recycle_file_inner(
    config: &RecycleBinConfig,
    source_path: &Path,
    payload_name_path: &Path,
    mut manifest: RecycleManifest,
    commit_after_move: bool,
) -> AppResult<Option<RecycleResult>> {
    let source_metadata = match tokio::fs::symlink_metadata(source_path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(AppError::Repository(format!(
                "failed to stat source file {} before recycle/delete: {}",
                source_path.display(),
                error
            )));
        }
    };

    // Refuse to act on a path outside the configured media roots. This guards both
    // the permanent-delete branch (recycle disabled) and the recycle move against a
    // stale/corrupt/out-of-root source path.
    ensure_source_within_roots(config, source_path)?;
    if source_metadata.file_type().is_symlink() {
        return Err(AppError::Validation(format!(
            "refusing to delete {} because it is a symlink",
            source_path.display()
        )));
    }
    if !source_metadata.is_file() {
        return Err(AppError::Validation(format!(
            "refusing to delete {} because it is not a regular file",
            source_path.display()
        )));
    }

    if !config.enabled {
        crate::fs_safety::remove_file_safely_if_exists(source_path).await?;
        return Ok(None);
    }

    if !config.cleanup_enabled {
        return Err(AppError::Validation(format!(
            "refusing to recycle {} because the recycle bin path is unsafe: {}",
            source_path.display(),
            config
                .validation_error
                .as_deref()
                .unwrap_or("invalid recycle bin configuration")
        )));
    }

    // Build timestamped directory name: YYYYMMDD_HHMMSSmmm_<6-char-id>
    let now = Utc::now();
    let full_id = scryer_domain::Id::new().0;
    let short_id = &full_id[..6];
    let dir_name = format!("{}_{}", now.format("%Y%m%d_%H%M%S%3f"), short_id);
    let recycle_dir = config.base_path.join(&dir_name);

    ensure_recycle_root(config).await?;
    tokio::fs::create_dir_all(&recycle_dir).await.map_err(|e| {
        AppError::Repository(format!(
            "failed to create recycle directory {}: {}",
            recycle_dir.display(),
            e
        ))
    })?;

    manifest.schema = Some(RECYCLE_MANIFEST_SCHEMA.to_string());
    manifest.entry_id = Some(dir_name.clone());
    manifest
        .source_operation_id
        .get_or_insert_with(|| scryer_domain::Id::new().0);
    manifest.status = Some(RECYCLE_STATUS_PENDING.to_string());
    if let Err(error) = write_manifest(&recycle_dir, &manifest).await {
        let _ = crate::fs_safety::remove_dir_all_safely_if_exists(&recycle_dir).await;
        return Err(error);
    }

    // Move the file into the recycle directory. Raw manifest strings are metadata
    // and may use stored-path encoding; payload names come from a trusted path
    // chosen by the caller.
    let file_name = payload_name_path
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("unknown"));
    let recycled_path = recycle_dir.join(file_name);

    if let Err(error) =
        recycle_source_to_destination(source_path.to_path_buf(), recycled_path.clone()).await
    {
        let _ = crate::fs_safety::remove_dir_all_safely_if_exists(&recycle_dir).await;
        return Err(error);
    }

    if commit_after_move {
        manifest.status = Some(RECYCLE_STATUS_COMMITTED.to_string());
        if let Err(error) = write_manifest(&recycle_dir, &manifest).await {
            warn!(
                error = %error,
                entry_dir = %recycle_dir.display(),
                "file recycled but manifest could not be marked committed; caller may retry commit metadata"
            );
        }
    }

    info!(
        original = %source_path.display(),
        recycled = %recycled_path.display(),
        reason = %manifest.reason,
        "file moved to recycle bin"
    );

    Ok(Some(RecycleResult {
        entry_id: dir_name,
        entry_dir: recycle_dir.clone(),
        recycled_path,
        manifest_path: manifest_path(&recycle_dir),
    }))
}

async fn recycle_source_to_destination(
    source_path: PathBuf,
    recycled_path: PathBuf,
) -> AppResult<()> {
    match tokio::fs::rename(&source_path, &recycled_path).await {
        Ok(()) => return Ok(()),
        Err(error) if is_cross_device_error(&error) => {
            info!(
                source = %source_path.display(),
                recycled = %recycled_path.display(),
                error = %error,
                "recycle rename crossed devices; falling back to copy with sampled verification"
            );
        }
        Err(error) => {
            return Err(AppError::Repository(format!(
                "failed to move {} to recycle bin {}: {}",
                source_path.display(),
                recycled_path.display(),
                error
            )));
        }
    }

    let mut source_file = tokio::fs::File::open(&source_path).await.map_err(|error| {
        AppError::Repository(format!(
            "failed to open source file {} for recycle: {}",
            source_path.display(),
            error
        ))
    })?;
    let mut recycled_file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&recycled_path)
        .await
        .map_err(|error| {
            AppError::Repository(format!(
                "failed to claim recycle destination {}: {}",
                recycled_path.display(),
                error
            ))
        })?;

    if let Err(error) = tokio::io::copy(&mut source_file, &mut recycled_file).await {
        let _ = crate::fs_safety::remove_file_safely_if_exists(&recycled_path).await;
        return Err(AppError::Repository(format!(
            "failed to copy {} to recycle bin {}: {}",
            source_path.display(),
            recycled_path.display(),
            error
        )));
    }
    if let Err(error) = recycled_file.flush().await {
        let _ = crate::fs_safety::remove_file_safely_if_exists(&recycled_path).await;
        return Err(AppError::Repository(format!(
            "failed to flush recycled file {}: {}",
            recycled_path.display(),
            error
        )));
    }
    if let Err(error) = recycled_file.sync_all().await {
        let _ = crate::fs_safety::remove_file_safely_if_exists(&recycled_path).await;
        return Err(AppError::Repository(format!(
            "failed to sync recycled file {}: {}",
            recycled_path.display(),
            error
        )));
    }
    drop(recycled_file);

    if let Err(verify_error) =
        crate::fs_integrity::verify_same_file_async(&source_path, &recycled_path).await
    {
        let _ = crate::fs_safety::remove_file_safely_if_exists(&recycled_path).await;
        return Err(verify_error);
    }
    drop(source_file);

    match crate::fs_safety::remove_file_safely_if_exists(&source_path).await {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = crate::fs_safety::remove_file_safely_if_exists(&recycled_path).await;
            Err(AppError::Repository(format!(
                "failed to remove source file {} after copy to recycle bin: {}",
                source_path.display(),
                error
            )))
        }
    }
}

fn is_cross_device_error(error: &std::io::Error) -> bool {
    matches!(error.raw_os_error(), Some(18) | Some(17))
}

fn lexically_normalize_for_policy(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            std::path::Component::RootDir => normalized.push(component.as_os_str()),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            std::path::Component::Normal(segment) => normalized.push(segment),
        }
    }
    normalized
}

#[cfg(not(windows))]
fn contains_non_windows_separator_ambiguity(path: &Path) -> bool {
    path.components().any(|component| match component {
        std::path::Component::Normal(segment) => segment.to_string_lossy().contains('\\'),
        std::path::Component::Prefix(prefix) => prefix.as_os_str().to_string_lossy().contains('\\'),
        _ => false,
    })
}

pub(crate) fn path_is_under_configured_root(path: &Path, root: &Path) -> bool {
    #[cfg(not(windows))]
    if contains_non_windows_separator_ambiguity(path)
        || contains_non_windows_separator_ambiguity(root)
    {
        return false;
    }

    let normalized_path = lexically_normalize_for_policy(path);
    let normalized_root = lexically_normalize_for_policy(root);
    crate::catalog_workflow::library_path_is_under_root(
        normalized_path.to_string_lossy().as_ref(),
        normalized_root.to_string_lossy().as_ref(),
    )
}

pub(crate) fn restore_destination_is_under_configured_root(path: &Path, root: &Path) -> bool {
    if path.file_name().is_none() {
        return false;
    }
    let Some(parent) = path.parent() else {
        return false;
    };
    path_is_under_configured_root(parent, root)
}

pub(crate) fn restore_destination_is_under_configured_roots(
    path: &Path,
    roots: &[PathBuf],
) -> bool {
    !roots.is_empty()
        && roots
            .iter()
            .any(|root| restore_destination_is_under_configured_root(path, root))
}

fn ensure_restore_destination_within_roots(destination: &Path, roots: &[PathBuf]) -> AppResult<()> {
    if roots.is_empty() {
        return Err(AppError::Validation(format!(
            "refusing to restore {} because no configured media roots are available",
            destination.display()
        )));
    }
    if restore_destination_is_under_configured_roots(destination, roots) {
        return Ok(());
    }
    Err(AppError::Validation(format!(
        "refusing to restore {} because it is outside the configured media roots",
        destination.display()
    )))
}

pub(crate) fn source_file_is_under_configured_root(path: &Path, root: &Path) -> bool {
    if path.file_name().is_none() {
        return false;
    }
    let Some(parent) = path.parent() else {
        return false;
    };
    path_is_under_configured_root(parent, root)
}

/// Refuse to recycle/delete a source path that is not inside any configured media
/// root. Source-removing operations must fail closed when no roots are available.
pub(crate) fn ensure_source_within_roots(
    config: &RecycleBinConfig,
    source_path: &Path,
) -> AppResult<()> {
    if config.source_roots.is_empty() {
        return Err(AppError::Validation(format!(
            "refusing to delete {} because no configured media roots are available",
            source_path.display()
        )));
    }
    if config
        .source_roots
        .iter()
        .any(|root| source_file_is_under_configured_root(source_path, root))
    {
        return Ok(());
    }
    Err(AppError::Validation(format!(
        "refusing to delete {} because it is outside the configured media roots",
        source_path.display()
    )))
}

/// Build restore destination candidates: original path first, then
/// `<stem>-restored.<ext>`, `<stem>-restored-2.<ext>`, and finally generated IDs.
fn restore_candidate_path(original_path: &Path, attempt: u32) -> PathBuf {
    if attempt == 0 {
        return original_path.to_path_buf();
    }

    let parent = original_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let stem = original_path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "restored".to_string());
    let extension = original_path
        .extension()
        .map(|ext| ext.to_string_lossy().into_owned());

    let suffix = if attempt <= 10_000 {
        if attempt == 1 {
            "-restored".to_string()
        } else {
            format!("-restored-{attempt}")
        }
    } else {
        format!("-restored-{}", scryer_domain::Id::new().0)
    };
    let file_name = match &extension {
        Some(ext) => format!("{stem}{suffix}.{ext}"),
        None => format!("{stem}{suffix}"),
    };
    parent.join(file_name)
}

async fn copy_recycled_to_claimed_destination(
    recycled_path: &Path,
    destination: &Path,
    destination_file: tokio::fs::File,
) -> AppResult<()> {
    copy_recycled_to_claimed_destination_with_verifier(
        recycled_path,
        destination,
        destination_file,
        |source, dest| async move { crate::fs_integrity::verify_same_file_async(&source, &dest).await },
    )
    .await
}

async fn copy_recycled_to_claimed_destination_with_verifier<F, Fut>(
    recycled_path: &Path,
    destination: &Path,
    mut destination_file: tokio::fs::File,
    verify: F,
) -> AppResult<()>
where
    F: FnOnce(PathBuf, PathBuf) -> Fut,
    Fut: std::future::Future<Output = AppResult<()>>,
{
    let mut source_file = tokio::fs::File::open(recycled_path)
        .await
        .map_err(|error| {
            AppError::Repository(format!(
                "failed to open recycled file {} for restore: {}",
                recycled_path.display(),
                error
            ))
        })?;
    if let Err(error) = tokio::io::copy(&mut source_file, &mut destination_file).await {
        drop(destination_file);
        let _ = crate::fs_safety::remove_file_safely_if_exists(destination).await;
        return Err(AppError::Repository(format!(
            "failed to restore {} to {}: {}",
            recycled_path.display(),
            destination.display(),
            error
        )));
    }
    if let Err(error) = destination_file.flush().await {
        drop(destination_file);
        let _ = crate::fs_safety::remove_file_safely_if_exists(destination).await;
        return Err(AppError::Repository(format!(
            "failed to flush restored file {}: {}",
            destination.display(),
            error
        )));
    }
    if let Err(error) = destination_file.sync_all().await {
        drop(destination_file);
        let _ = crate::fs_safety::remove_file_safely_if_exists(destination).await;
        return Err(AppError::Repository(format!(
            "failed to sync restored file {}: {}",
            destination.display(),
            error
        )));
    }
    drop(destination_file);

    let recycled_for_verify = recycled_path.to_path_buf();
    let destination_for_verify = destination.to_path_buf();
    if let Err(verify_error) = verify(recycled_for_verify, destination_for_verify).await {
        let _ = crate::fs_safety::remove_file_safely_if_exists(destination).await;
        return Err(verify_error);
    }

    Ok(())
}

async fn restore_without_overwrite(
    recycled_path: &Path,
    original_path: &Path,
    allowed_roots: Option<&[PathBuf]>,
) -> AppResult<PathBuf> {
    for attempt in 0..=20_000u32 {
        let destination = restore_candidate_path(original_path, attempt);
        if let Some(roots) = allowed_roots {
            ensure_restore_destination_within_roots(&destination, roots)?;
        }
        match tokio::fs::hard_link(recycled_path, &destination).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(link_error) => {
                // Hard links are preferred but impossible across devices and on
                // filesystems without link support; claim the destination and
                // copy with verification instead.
                let destination_file = match tokio::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&destination)
                    .await
                {
                    Ok(file) => file,
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => {
                        return Err(AppError::Repository(format!(
                            "failed to claim restore destination {} after hard link failed ({}): {}",
                            destination.display(),
                            link_error,
                            error
                        )));
                    }
                };
                copy_recycled_to_claimed_destination(recycled_path, &destination, destination_file)
                    .await?;
            }
        }

        if destination != original_path {
            warn!(
                original = %original_path.display(),
                restored_to = %destination.display(),
                "original path is occupied; restoring to a -restored sibling to avoid overwriting the live file"
            );
        }

        crate::fs_safety::remove_file_safely(recycled_path)
            .await
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to remove recycled source {} after restore: {}",
                    recycled_path.display(),
                    error
                ))
            })?;
        return Ok(destination);
    }

    Err(AppError::Repository(format!(
        "failed to find a non-colliding restore destination for {}",
        original_path.display()
    )))
}

/// Restore a file from the recycle bin.
///
/// When `overwrite` is false and a live file already occupies `original_path`,
/// the restored file is placed at a `-restored` sibling path instead of clobbering
/// the occupant. Returns the path the file was actually restored to.
pub async fn restore_from_recycle(
    recycled_path: &Path,
    original_path: &Path,
    overwrite: bool,
) -> AppResult<PathBuf> {
    restore_from_recycle_inner(recycled_path, original_path, overwrite, None).await
}

pub(crate) async fn restore_from_recycle_with_roots(
    recycled_path: &Path,
    original_path: &Path,
    overwrite: bool,
    allowed_roots: &[PathBuf],
) -> AppResult<PathBuf> {
    restore_from_recycle_inner(recycled_path, original_path, overwrite, Some(allowed_roots)).await
}

pub(crate) async fn restore_recycled_file_exact(
    recycled_path: &Path,
    destination: &Path,
) -> AppResult<()> {
    link_recycled_file_to_exact_destination(recycled_path, destination).await?;
    crate::fs_safety::remove_file_safely(recycled_path)
        .await
        .map_err(|error| {
            AppError::Repository(format!(
                "failed to remove recycled source {} after exact restore to {}: {}",
                recycled_path.display(),
                destination.display(),
                error
            ))
        })
}

/// Creates an exact restore destination without consuming the recycle entry.
///
/// Callers that need to perform fallible work between exposing the restored file
/// and removing its recycle entry can use this to retain a rollback source.
pub(crate) async fn link_recycled_file_to_exact_destination(
    recycled_path: &Path,
    destination: &Path,
) -> AppResult<()> {
    ensure_recycled_restore_source_is_regular(recycled_path).await?;
    match tokio::fs::symlink_metadata(destination).await {
        Ok(_) => {
            return Err(AppError::Validation(format!(
                "refusing to restore recycled file {} because destination is occupied: {}",
                recycled_path.display(),
                destination.display()
            )));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(AppError::Repository(format!(
                "failed to stat recycle restore destination {}: {}",
                destination.display(),
                error
            )));
        }
    }

    if let Some(parent) = destination.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|error| {
            AppError::Repository(format!(
                "failed to create restore parent directory {}: {}",
                parent.display(),
                error
            ))
        })?;
    }

    match tokio::fs::hard_link(recycled_path, destination).await {
        Ok(()) => Ok(()),
        Err(link_error) => {
            // Same fallback as restore_without_overwrite; either way the
            // recycled source is retained, so callers keep their rollback
            // point until they explicitly remove it.
            let destination_file = tokio::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(destination)
                .await
                .map_err(|error| {
                    AppError::Repository(format!(
                        "failed to claim exact restore destination {} after hard link failed ({}): {}",
                        destination.display(),
                        link_error,
                        error
                    ))
                })?;
            copy_recycled_to_claimed_destination(recycled_path, destination, destination_file).await
        }
    }
}

async fn restore_from_recycle_inner(
    recycled_path: &Path,
    original_path: &Path,
    overwrite: bool,
    allowed_roots: Option<&[PathBuf]>,
) -> AppResult<PathBuf> {
    ensure_recycled_restore_source_is_regular(recycled_path).await?;

    if let Some(roots) = allowed_roots {
        ensure_restore_destination_within_roots(original_path, roots)?;
    }

    // Ensure parent directory exists
    if let Some(parent) = original_path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| {
            AppError::Repository(format!(
                "failed to create parent directory {}: {}",
                parent.display(),
                e
            ))
        })?;
    }

    if !overwrite {
        let destination =
            restore_without_overwrite(recycled_path, original_path, allowed_roots).await?;
        info!(
            restored = %destination.display(),
            "file restored from recycle bin"
        );
        return Ok(destination);
    }

    match tokio::fs::rename(recycled_path, original_path).await {
        Ok(()) => {}
        Err(error) if is_cross_device_error(&error) => {
            // Cross-device restore cannot rename. Prove the restored copy is
            // identical before removing the recycled source; on mismatch,
            // remove the bad restore and keep the recycled copy so the file
            // is never lost.
            tokio::fs::copy(recycled_path, original_path)
                .await
                .map_err(|copy_error| {
                    AppError::Repository(format!(
                        "failed to restore {} to {}: {}",
                        recycled_path.display(),
                        original_path.display(),
                        copy_error
                    ))
                })?;
            if let Err(verify_error) =
                crate::fs_integrity::verify_same_file_async(recycled_path, original_path).await
            {
                let _ = crate::fs_safety::remove_file_safely_if_exists(original_path).await;
                return Err(verify_error);
            }
            let _ = crate::fs_safety::remove_file_safely_if_exists(recycled_path).await;
        }
        Err(error) => {
            return Err(AppError::Repository(format!(
                "failed to move recycled file {} to {}: {}",
                recycled_path.display(),
                original_path.display(),
                error
            )));
        }
    }

    info!(
        restored = %original_path.display(),
        "file restored from recycle bin"
    );

    Ok(original_path.to_path_buf())
}

pub(crate) async fn ensure_recycled_restore_source_is_regular(
    recycled_path: &Path,
) -> AppResult<()> {
    let metadata = match tokio::fs::symlink_metadata(recycled_path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(AppError::Repository(format!(
                "recycled file not found: {}",
                recycled_path.display()
            )));
        }
        Err(error) => {
            return Err(AppError::Repository(format!(
                "failed to stat recycled file {} before restore: {}",
                recycled_path.display(),
                error
            )));
        }
    };
    if metadata.file_type().is_symlink() {
        return Err(AppError::Validation(format!(
            "refusing to restore recycled file {} because it is a symlink",
            recycled_path.display()
        )));
    }
    if !metadata.is_file() {
        return Err(AppError::Validation(format!(
            "refusing to restore recycled file {} because it is not a regular file",
            recycled_path.display()
        )));
    }
    Ok(())
}

pub async fn commit_recycle_entry(
    recycle_result: &Option<RecycleResult>,
    replacement_file_id: &str,
    replacement_path: &Path,
) -> AppResult<()> {
    let Some(result) = recycle_result else {
        return Ok(());
    };

    if !replacement_path.exists() {
        return Err(AppError::Repository(format!(
            "refusing to commit recycle entry {} because replacement file does not exist: {}",
            result.entry_id,
            replacement_path.display()
        )));
    }

    let mut manifest = read_manifest(&result.entry_dir).await?.ok_or_else(|| {
        AppError::Repository(format!("missing recycle manifest {}", result.entry_id))
    })?;
    manifest.status = Some(RECYCLE_STATUS_COMMITTED.to_string());
    manifest.replacement_file_id = Some(replacement_file_id.to_string());
    manifest.replacement_path = Some(crate::stored_paths::path_to_stored_string(replacement_path));
    write_manifest(&result.entry_dir, &manifest).await
}

pub(crate) async fn commit_recycle_entry_without_replacement(
    recycle_result: &RecycleResult,
) -> AppResult<()> {
    let mut manifest = read_manifest(&recycle_result.entry_dir)
        .await?
        .ok_or_else(|| {
            AppError::Repository(format!(
                "missing recycle manifest {}",
                recycle_result.entry_id
            ))
        })?;
    manifest.status = Some(RECYCLE_STATUS_COMMITTED.to_string());
    write_manifest(&recycle_result.entry_dir, &manifest).await
}

pub async fn quarantine_entry(
    entry_dir: &Path,
    manifest: &RecycleManifest,
    reason: &str,
) -> AppResult<()> {
    if manifest.is_quarantined() {
        return Ok(());
    }

    let mut quarantined = manifest.clone();
    quarantined.status = Some(RECYCLE_STATUS_QUARANTINED.to_string());
    quarantined.reason = format!("{}; quarantine: {}", quarantined.reason, reason);
    write_manifest(entry_dir, &quarantined).await
}

async fn trusted_committed_entry_from_dir(
    config: &RecycleBinConfig,
    entry_dir: PathBuf,
) -> AppResult<Option<CommittedRecycleEntry>> {
    let metadata = match std::fs::symlink_metadata(&entry_dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            warn!(
                path = %entry_dir.display(),
                error = %error,
                "failed to inspect recycle entry path, skipping"
            );
            return Ok(None);
        }
    };
    if metadata.file_type().is_symlink() {
        warn!(
            path = %entry_dir.display(),
            "skipping symlinked recycle entry"
        );
        return Ok(None);
    }
    if !metadata.is_dir() {
        return Ok(None);
    }
    let Some(parent) = entry_dir.parent() else {
        return Ok(None);
    };
    if normalize_path(parent) != normalize_path(&config.base_path) {
        warn!(
            path = %entry_dir.display(),
            recycle_root = %config.base_path.display(),
            "skipping recycle entry outside configured recycle root"
        );
        return Ok(None);
    }

    let Some(manifest) = (match read_manifest(&entry_dir).await {
        Ok(manifest) => manifest,
        Err(error) => {
            warn!(path = %entry_dir.display(), error = %error, "failed to inspect recycle entry, skipping");
            return Ok(None);
        }
    }) else {
        return Ok(None);
    };

    if let Err(reason) = trusted_committed_entry(config, &entry_dir, &manifest) {
        if let Err(error) =
            quarantine_untrusted_committed_entry(config, &entry_dir, &manifest, &reason).await
        {
            warn!(
                path = %entry_dir.display(),
                error = %error,
                "failed to quarantine untrusted recycle entry"
            );
        }
        return Ok(None);
    }

    Ok(Some(CommittedRecycleEntry {
        entry_dir,
        manifest,
    }))
}

pub async fn list_committed_entries(
    config: &RecycleBinConfig,
) -> AppResult<Vec<CommittedRecycleEntry>> {
    if !cleanup_ready(config) {
        if let Some(error) = &config.validation_error {
            warn!(error = %error, path = %config.base_path.display(), "recycle bin cleanup disabled");
        }
        return Ok(Vec::new());
    }

    let mut results = Vec::new();
    let mut entries = tokio::fs::read_dir(&config.base_path).await.map_err(|e| {
        AppError::Repository(format!(
            "failed to read recycle bin directory {}: {}",
            config.base_path.display(),
            e
        ))
    })?;

    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| AppError::Repository(format!("failed to read recycle bin entry: {}", e)))?
    {
        if let Some(entry) = trusted_committed_entry_from_dir(config, entry.path()).await? {
            results.push(entry);
        }
    }

    Ok(results)
}

pub async fn list_expired_committed_entries(
    config: &RecycleBinConfig,
) -> AppResult<Vec<CommittedRecycleEntry>> {
    let cutoff = Utc::now() - chrono::Duration::days(config.retention_days as i64);
    let mut results = Vec::new();

    for entry in list_committed_entries(config).await? {
        let recycled_at = match chrono::DateTime::parse_from_rfc3339(&entry.manifest.recycled_at) {
            Ok(dt) => dt.with_timezone(&Utc),
            Err(_) => continue,
        };

        if recycled_at < cutoff {
            results.push(entry);
        }
    }

    Ok(results)
}

/// Pending entries older than this are leftovers from an interrupted move
/// transaction: every live transaction commits or rolls back within seconds.
const STALE_PENDING_RECYCLE_ENTRY_GRACE_HOURS: i64 = 24;

/// Commit pending recycle entries whose surrounding move transaction can no
/// longer be in flight.
///
/// Pending entries are invisible to listing, restore, and retention sweeps by
/// design, so one orphaned by a crash would otherwise hold disk space forever
/// with no operator-visible surface. Committing it makes the file visible in
/// the recycle bin (restorable again) and subject to normal retention expiry.
///
/// Returns the number of entries committed.
pub(crate) async fn reconcile_stale_pending_entries(config: &RecycleBinConfig) -> AppResult<u32> {
    if !cleanup_ready(config) {
        return Ok(0);
    }

    let cutoff = Utc::now() - chrono::Duration::hours(STALE_PENDING_RECYCLE_ENTRY_GRACE_HOURS);
    let mut reconciled = 0u32;
    let mut entries = tokio::fs::read_dir(&config.base_path).await.map_err(|e| {
        AppError::Repository(format!(
            "failed to read recycle bin directory {}: {}",
            config.base_path.display(),
            e
        ))
    })?;

    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| AppError::Repository(format!("failed to read recycle bin entry: {}", e)))?
    {
        let entry_dir = entry.path();
        let Ok(file_type) = entry.file_type().await else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let mut manifest = match read_manifest(&entry_dir).await {
            Ok(Some(manifest)) => manifest,
            Ok(None) => continue,
            Err(error) => {
                warn!(
                    path = %entry_dir.display(),
                    error = %error,
                    "failed to inspect recycle entry during pending reconciliation, skipping"
                );
                continue;
            }
        };
        if manifest.status.as_deref() != Some(RECYCLE_STATUS_PENDING) {
            continue;
        }
        let recycled_at = match chrono::DateTime::parse_from_rfc3339(&manifest.recycled_at) {
            Ok(dt) => dt.with_timezone(&Utc),
            Err(_) => {
                warn!(
                    path = %entry_dir.display(),
                    "skipping pending recycle entry with an unparsable recycled_at"
                );
                continue;
            }
        };
        if recycled_at >= cutoff {
            continue;
        }

        manifest.status = Some(RECYCLE_STATUS_COMMITTED.to_string());
        if let Err(error) = write_manifest(&entry_dir, &manifest).await {
            warn!(
                path = %entry_dir.display(),
                error = %error,
                "failed to commit stale pending recycle entry"
            );
            continue;
        }
        warn!(
            path = %entry_dir.display(),
            reason = %manifest.reason,
            "committed stale pending recycle entry left by an interrupted move transaction"
        );
        reconciled += 1;
    }

    Ok(reconciled)
}

pub(crate) async fn purge_committed_entry(
    config: &RecycleBinConfig,
    entry: &CommittedRecycleEntry,
) -> AppResult<bool> {
    if let Err(reason) = trusted_committed_entry(config, &entry.entry_dir, &entry.manifest) {
        warn!(path = %entry.entry_dir.display(), reason = %reason, "skipping untrusted recycle entry purge");
        if let Err(error) =
            quarantine_untrusted_committed_entry(config, &entry.entry_dir, &entry.manifest, &reason)
                .await
        {
            warn!(
                path = %entry.entry_dir.display(),
                error = %error,
                "failed to quarantine untrusted recycle entry"
            );
        }
        return Ok(false);
    }

    crate::fs_safety::remove_dir_all_safely(&entry.entry_dir).await?;
    Ok(true)
}

/// Purge recycled entries older than `config.retention_days`.
///
/// Returns the count of purged entries.
pub async fn purge_expired(config: &RecycleBinConfig) -> AppResult<u32> {
    let mut purged = 0u32;
    for entry in list_expired_committed_entries(config).await? {
        if entry.manifest.reason == "upgrade_replaced" {
            warn!(
                path = %entry.entry_dir.display(),
                "skipping generic purge for upgrade recycle entry; caller must validate replacement state"
            );
            continue;
        }
        if purge_committed_entry(config, &entry).await? {
            purged += 1;
        }
    }

    if purged > 0 {
        info!(purged, "purged expired recycle bin entries");
    }

    Ok(purged)
}

/// Test helper for purging recycle bin entries that belong to a specific title.
///
/// Returns the count of purged entries.
#[cfg(test)]
async fn purge_for_title(config: &RecycleBinConfig, title_id: &str) -> AppResult<u32> {
    let mut purged = 0u32;

    for entry in list_committed_entries(config).await? {
        if entry.manifest.title_id.as_deref() == Some(title_id)
            && purge_committed_entry(config, &entry).await?
        {
            purged += 1;
        }
    }

    if purged > 0 {
        info!(
            purged,
            title_id, "purged recycle bin entries for deleted title"
        );
    }

    Ok(purged)
}

/// List all entries in a recycle bin directory.
pub async fn list_entries(
    config: &RecycleBinConfig,
    media_root: &str,
) -> AppResult<Vec<RecycleEntry>> {
    let mut results = list_committed_entries(config)
        .await?
        .into_iter()
        .map(|entry| {
            let entry_id = entry
                .entry_dir
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            RecycleEntry {
                entry_id,
                manifest: entry.manifest,
                media_root: media_root.to_string(),
            }
        })
        .collect::<Vec<_>>();

    results.sort_by(|a, b| b.manifest.recycled_at.cmp(&a.manifest.recycled_at));
    Ok(results)
}

pub async fn find_committed_entry(
    config: &RecycleBinConfig,
    entry_id: &str,
) -> AppResult<Option<CommittedRecycleEntry>> {
    validate_recycle_entry_id(entry_id)?;
    if !cleanup_ready(config) {
        return Ok(None);
    }

    let entry_dir = config.base_path.join(entry_id);
    trusted_committed_entry_from_dir(config, entry_dir).await
}

/// Look up a specific trusted committed recycle bin entry by its directory name.
pub async fn find_entry(
    config: &RecycleBinConfig,
    entry_id: &str,
) -> AppResult<Option<(PathBuf, RecycleManifest)>> {
    Ok(find_committed_entry(config, entry_id)
        .await?
        .map(|entry| (entry.entry_dir, entry.manifest)))
}

/// Purge ALL recycle bin entries regardless of age.
pub async fn purge_all(config: &RecycleBinConfig) -> AppResult<u32> {
    let mut purged = 0u32;

    for entry in list_committed_entries(config).await? {
        if entry.manifest.reason == "upgrade_replaced" {
            warn!(
                path = %entry.entry_dir.display(),
                "skipping generic empty for upgrade recycle entry; caller must validate replacement state"
            );
            continue;
        }

        if purge_committed_entry(config, &entry).await? {
            purged += 1;
        }
    }

    if purged > 0 {
        info!(purged, "emptied recycle bin");
    }

    Ok(purged)
}

/// Resolve the media root path for a title's facet.
///
/// Uses the title's owning library roots, falling back to the facet default
/// roots when legacy data points at a missing library.
pub async fn media_root_for_title(
    app: &crate::AppUseCase,
    title: &scryer_domain::Title,
) -> Option<String> {
    app.default_media_root_for_title(title)
        .await
        .map_err(|error| {
            warn!(
                error = %error,
                title_id = %title.id,
                library_id = %title.library_id,
                "failed to resolve media root for title"
            );
        })
        .ok()
}

/// Build a recycle bin config from a file path by walking up to find the media root.
///
/// For use in contexts where `AppUseCase` is not available (e.g., standalone async functions).
/// Defaults: enabled=true, retention_days=7, base_path derived from file's grandparent.
pub fn config_from_file_path(file_path: &Path) -> RecycleBinConfig {
    // Walk up to the grandparent as a rough media root estimate.
    // e.g. /data/movies/Movie (2024)/Movie.mkv → /data/movies/
    let base = file_path
        .parent() // Movie (2024)/
        .and_then(|p| p.parent()) // /data/movies/
        .unwrap_or_else(|| Path::new("/tmp"));

    RecycleBinConfig {
        enabled: true,
        base_path: base.join(RECYCLE_DIR_NAME),
        retention_days: DEFAULT_RETENTION_DAYS,
        cleanup_enabled: true,
        validation_error: None,
        source_roots: vec![base.to_path_buf()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_config(dir: &Path) -> RecycleBinConfig {
        let source_root = dir.parent().unwrap_or(dir).to_path_buf();
        RecycleBinConfig {
            enabled: true,
            base_path: dir.to_path_buf(),
            retention_days: 7,
            cleanup_enabled: true,
            validation_error: None,
            source_roots: vec![source_root],
        }
    }

    fn test_manifest() -> RecycleManifest {
        RecycleManifest {
            schema: None,
            entry_id: None,
            source_operation_id: None,
            recycled_at: Utc::now().to_rfc3339(),
            original_path: "/data/movies/test.mkv".to_string(),
            original_file_id: None,
            size_bytes: 1024,
            title_id: Some("title-123".to_string()),
            media_root: None,
            reason: "title_deleted".to_string(),
            status: None,
            replacement_file_id: None,
            replacement_path: None,
        }
    }

    fn committed_manifest(
        entry_id: &str,
        recycled_at: String,
        original_path: &str,
        title_id: Option<&str>,
        reason: &str,
    ) -> RecycleManifest {
        RecycleManifest {
            schema: Some(RECYCLE_MANIFEST_SCHEMA.to_string()),
            entry_id: Some(entry_id.to_string()),
            source_operation_id: Some("operation-1".to_string()),
            recycled_at,
            original_path: original_path.to_string(),
            original_file_id: None,
            size_bytes: 100,
            title_id: title_id.map(str::to_string),
            media_root: None,
            reason: reason.to_string(),
            status: Some(RECYCLE_STATUS_COMMITTED.to_string()),
            replacement_file_id: None,
            replacement_path: None,
        }
    }

    fn pending_manifest(entry_id: &str, recycled_at: String) -> RecycleManifest {
        let mut manifest = committed_manifest(
            entry_id,
            recycled_at,
            "/data/series/Show/S01E01.mkv",
            Some("title-123"),
            "upgrade_replaced",
        );
        manifest.status = Some(RECYCLE_STATUS_PENDING.to_string());
        manifest
    }

    async fn write_test_sentinel(recycle_dir: &Path) {
        tokio::fs::write(
            recycle_dir.join(RECYCLE_ROOT_SENTINEL),
            RECYCLE_MANIFEST_SCHEMA.as_bytes(),
        )
        .await
        .unwrap();
    }

    async fn write_test_entry(
        recycle_dir: &Path,
        entry_id: &str,
        manifest: &RecycleManifest,
    ) -> PathBuf {
        let entry_dir = recycle_dir.join(entry_id);
        tokio::fs::create_dir_all(&entry_dir).await.unwrap();
        tokio::fs::write(
            entry_dir.join("manifest.json"),
            serde_json::to_string(manifest).unwrap(),
        )
        .await
        .unwrap();
        tokio::fs::write(entry_dir.join("media.mkv"), b"media")
            .await
            .unwrap();
        entry_dir
    }

    async fn read_test_manifest(entry_dir: &Path) -> RecycleManifest {
        let bytes = tokio::fs::read(entry_dir.join("manifest.json"))
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlinked_recycle_entry_is_skipped_without_quarantining_target() {
        let tmp = TempDir::new().unwrap();
        let recycle_dir = tmp.path().join("recycle");
        tokio::fs::create_dir_all(&recycle_dir).await.unwrap();
        write_test_sentinel(&recycle_dir).await;

        let entry_id = "20260205_120000000_sym111";
        let manifest = committed_manifest(
            entry_id,
            Utc::now().to_rfc3339(),
            "/data/movies/Movie/Movie.mkv",
            Some("title-123"),
            "file_deleted",
        );
        let target_parent = tmp.path().join("target");
        let target_dir = write_test_entry(&target_parent, entry_id, &manifest).await;
        let entry_link = recycle_dir.join(entry_id);
        std::os::unix::fs::symlink(&target_dir, &entry_link).unwrap();

        let config = test_config(&recycle_dir);
        let entries = list_committed_entries(&config).await.unwrap();
        let found = find_committed_entry(&config, entry_id).await.unwrap();
        let purged = purge_all(&config).await.unwrap();

        assert!(entries.is_empty());
        assert!(found.is_none());
        assert_eq!(purged, 0);
        assert!(
            std::fs::symlink_metadata(&entry_link)
                .unwrap()
                .file_type()
                .is_symlink(),
            "symlink entry should be left in place"
        );
        let target_manifest = read_test_manifest(&target_dir).await;
        assert_eq!(
            target_manifest.status.as_deref(),
            Some(RECYCLE_STATUS_COMMITTED),
            "symlink target manifest must not be quarantined through the link"
        );
        assert!(!target_manifest.reason.contains("quarantine:"));
    }

    #[tokio::test]
    async fn generated_looking_regular_file_entry_is_skipped_and_left_in_place() {
        let tmp = TempDir::new().unwrap();
        let recycle_dir = tmp.path().join("recycle");
        tokio::fs::create_dir_all(&recycle_dir).await.unwrap();
        write_test_sentinel(&recycle_dir).await;

        let entry_id = "20260205_120000000_file11";
        let entry_file = recycle_dir.join(entry_id);
        tokio::fs::write(&entry_file, b"not a directory")
            .await
            .unwrap();

        let config = test_config(&recycle_dir);
        let entries = list_committed_entries(&config).await.unwrap();
        let found = find_committed_entry(&config, entry_id).await.unwrap();
        let purged = purge_all(&config).await.unwrap();

        assert!(entries.is_empty());
        assert!(found.is_none());
        assert_eq!(purged, 0);
        assert!(entry_file.is_file(), "regular file entry should survive");
    }

    #[tokio::test]
    async fn purge_committed_entry_removes_valid_trusted_entry() {
        let tmp = TempDir::new().unwrap();
        let recycle_dir = tmp.path().join("recycle");
        tokio::fs::create_dir_all(&recycle_dir).await.unwrap();
        write_test_sentinel(&recycle_dir).await;

        let entry_id = "20260205_120000000_ok1111";
        let manifest = committed_manifest(
            entry_id,
            Utc::now().to_rfc3339(),
            "/data/movies/Movie/Movie.mkv",
            Some("title-123"),
            "file_deleted",
        );
        let entry_dir = write_test_entry(&recycle_dir, entry_id, &manifest).await;

        let config = test_config(&recycle_dir);
        let entries = list_committed_entries(&config).await.unwrap();
        assert_eq!(entries.len(), 1);

        let purged = purge_committed_entry(&config, &entries[0]).await.unwrap();

        assert!(purged);
        assert!(!entry_dir.exists(), "valid trusted entry should be purged");
    }

    #[tokio::test]
    async fn committed_untrusted_entry_is_quarantined_when_listed() {
        let tmp = TempDir::new().unwrap();
        let recycle_dir = tmp.path().join("recycle");
        tokio::fs::create_dir_all(&recycle_dir).await.unwrap();
        write_test_sentinel(&recycle_dir).await;

        let entry_id = "20260205_120000000_bad111";
        let mut manifest = committed_manifest(
            entry_id,
            Utc::now().to_rfc3339(),
            "/data/movies/Movie/Movie.mkv",
            Some("title-123"),
            "file_deleted",
        );
        manifest.schema = None;
        let entry_dir = write_test_entry(&recycle_dir, entry_id, &manifest).await;

        let config = test_config(&recycle_dir);
        let entries = list_committed_entries(&config).await.unwrap();

        assert!(entries.is_empty());
        let quarantined = read_test_manifest(&entry_dir).await;
        assert_eq!(
            quarantined.status.as_deref(),
            Some(RECYCLE_STATUS_QUARANTINED)
        );
        assert!(quarantined.reason.contains("quarantine:"));
    }

    #[tokio::test]
    async fn pending_untrusted_entry_is_not_quarantined_or_purged() {
        let tmp = TempDir::new().unwrap();
        let recycle_dir = tmp.path().join("recycle");
        tokio::fs::create_dir_all(&recycle_dir).await.unwrap();
        write_test_sentinel(&recycle_dir).await;

        let entry_id = "20260205_120000000_pen111";
        let mut manifest = pending_manifest(entry_id, Utc::now().to_rfc3339());
        manifest.schema = None;
        let entry_dir = write_test_entry(&recycle_dir, entry_id, &manifest).await;

        let config = test_config(&recycle_dir);
        let purged = purge_expired(&config).await.unwrap();

        assert_eq!(purged, 0);
        assert!(entry_dir.exists());
        let pending = read_test_manifest(&entry_dir).await;
        assert_eq!(pending.status.as_deref(), Some(RECYCLE_STATUS_PENDING));
    }

    #[tokio::test]
    async fn cleanup_not_ready_does_not_quarantine_committed_entries() {
        let tmp = TempDir::new().unwrap();
        let recycle_dir = tmp.path().join("recycle");
        tokio::fs::create_dir_all(&recycle_dir).await.unwrap();

        let entry_id = "20260205_120000000_nos111";
        let mut manifest = committed_manifest(
            entry_id,
            Utc::now().to_rfc3339(),
            "/data/movies/Movie/Movie.mkv",
            Some("title-123"),
            "file_deleted",
        );
        manifest.schema = None;
        let entry_dir = write_test_entry(&recycle_dir, entry_id, &manifest).await;

        let config = test_config(&recycle_dir);
        let entries = list_committed_entries(&config).await.unwrap();

        assert!(entries.is_empty());
        let unchanged = read_test_manifest(&entry_dir).await;
        assert_eq!(unchanged.status.as_deref(), Some(RECYCLE_STATUS_COMMITTED));
    }

    #[tokio::test]
    async fn purge_all_quarantines_untrusted_real_entry() {
        let tmp = TempDir::new().unwrap();
        let recycle_dir = tmp.path().join("recycle");
        tokio::fs::create_dir_all(&recycle_dir).await.unwrap();
        write_test_sentinel(&recycle_dir).await;

        let entry_id = "20260205_120000000_pur111";
        let mut manifest = committed_manifest(
            entry_id,
            Utc::now().to_rfc3339(),
            "/data/movies/Movie/Movie.mkv",
            Some("title-123"),
            "file_deleted",
        );
        manifest.entry_id = Some("different-entry".to_string());
        let entry_dir = write_test_entry(&recycle_dir, entry_id, &manifest).await;

        let config = test_config(&recycle_dir);
        let purged = purge_all(&config).await.unwrap();

        assert_eq!(purged, 0);
        assert!(entry_dir.exists());
        let quarantined = read_test_manifest(&entry_dir).await;
        assert_eq!(
            quarantined.status.as_deref(),
            Some(RECYCLE_STATUS_QUARANTINED)
        );
    }

    #[tokio::test]
    async fn test_recycle_creates_dir_and_manifest() {
        let tmp = TempDir::new().unwrap();
        let recycle_dir = tmp.path().join("recycle");
        let source = tmp.path().join("test.mkv");
        tokio::fs::write(&source, b"video data").await.unwrap();

        let config = test_config(&recycle_dir);
        let result = recycle_file(&config, &source, test_manifest())
            .await
            .unwrap();

        assert!(result.is_some());
        let r = result.unwrap();
        assert!(r.recycled_path.exists());
        assert!(r.manifest_path.exists());
        assert!(!source.exists());

        // Verify manifest is valid JSON
        let bytes = tokio::fs::read(&r.manifest_path).await.unwrap();
        let m: RecycleManifest = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(m.schema.as_deref(), Some(RECYCLE_MANIFEST_SCHEMA));
        assert_eq!(m.status.as_deref(), Some(RECYCLE_STATUS_COMMITTED));
        assert_eq!(m.reason, "title_deleted");
    }

    #[tokio::test]
    async fn recycle_payload_filename_comes_from_source_path_when_manifest_is_encoded() {
        let tmp = TempDir::new().unwrap();
        let recycle_dir = tmp.path().join("recycle");
        let source = tmp.path().join("real-source.mkv");
        tokio::fs::write(&source, b"video data").await.unwrap();

        let config = test_config(&recycle_dir);
        let mut manifest = test_manifest();
        manifest.original_path = "scryer-path-v1:u:/data/movies/%FFmanifest-name.mkv".to_string();

        let result = recycle_file(&config, &source, manifest)
            .await
            .unwrap()
            .expect("recycled");

        assert_eq!(result.recycled_path.file_name(), source.file_name());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn recycle_same_device_uses_rename_fast_path() {
        use std::os::unix::fs::MetadataExt;

        let tmp = TempDir::new().unwrap();
        let recycle_dir = tmp.path().join("recycle");
        let source = tmp.path().join("test.mkv");
        tokio::fs::write(&source, b"video data").await.unwrap();
        let before = std::fs::metadata(&source).unwrap();

        let config = test_config(&recycle_dir);
        let result = recycle_file(&config, &source, test_manifest())
            .await
            .unwrap()
            .expect("recycled");

        let after = std::fs::metadata(&result.recycled_path).unwrap();
        assert_eq!(before.dev(), after.dev());
        assert_eq!(
            before.ino(),
            after.ino(),
            "same-device recycle should rename the file instead of copying it"
        );
        assert!(!source.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn recycle_refuses_symlink_source_without_touching_link_or_target() {
        let tmp = TempDir::new().unwrap();
        let recycle_dir = tmp.path().join("recycle");
        let target = tmp.path().join("target.mkv");
        let link = tmp.path().join("link.mkv");
        tokio::fs::write(&target, b"video data").await.unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let config = test_config(&recycle_dir);
        let result = recycle_file(&config, &link, test_manifest()).await;

        assert!(result.is_err(), "symlink source must be refused");
        assert!(link.exists(), "refused symlink should remain");
        assert!(target.exists(), "symlink target should remain untouched");
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[tokio::test]
    async fn commit_recycle_entry_stores_encoded_replacement_path() {
        use std::os::unix::ffi::OsStringExt;

        let tmp = TempDir::new().unwrap();
        let recycle_dir = tmp.path().join("recycle");
        let source = tmp.path().join("old.mkv");
        tokio::fs::write(&source, b"old video").await.unwrap();
        let config = test_config(&recycle_dir);
        let metadata = ReplacedMediaRecycleMetadata {
            original_path: source.to_str().unwrap(),
            original_file_id: "old-file",
            size_bytes: 9,
            title_id: "title",
            media_root: Some(tmp.path().to_str().unwrap()),
        };
        let result = recycle_replaced_media_file(&config, &source, metadata, false)
            .await
            .unwrap();

        let replacement_name = std::ffi::OsString::from_vec(b"replacement-\xFF.mkv".to_vec());
        let replacement_path = tmp.path().join(replacement_name);
        tokio::fs::write(&replacement_path, b"new video")
            .await
            .unwrap();
        commit_recycle_entry(&result, "replacement-file", &replacement_path)
            .await
            .unwrap();

        let result = result.expect("pending recycle result");
        let manifest = read_test_manifest(&result.entry_dir).await;
        let stored = manifest.replacement_path.expect("replacement path");
        assert!(stored.starts_with("scryer-path-v1:u:"));
        assert_eq!(
            crate::stored_paths::stored_path_to_path_buf(&stored),
            replacement_path
        );
    }

    #[tokio::test]
    async fn test_recycle_disabled_deletes_directly() {
        let tmp = TempDir::new().unwrap();
        let source = tmp.path().join("test.mkv");
        tokio::fs::write(&source, b"video data").await.unwrap();

        let config = RecycleBinConfig {
            enabled: false,
            base_path: tmp.path().join("recycle"),
            retention_days: 7,
            cleanup_enabled: true,
            validation_error: None,
            source_roots: vec![tmp.path().to_path_buf()],
        };

        let result = recycle_file(&config, &source, test_manifest())
            .await
            .unwrap();

        assert!(result.is_none());
        assert!(!source.exists());
    }

    #[tokio::test]
    async fn test_recycle_refuses_source_outside_configured_roots() {
        let tmp = TempDir::new().unwrap();
        let outside = tmp.path().join("outside.mkv");
        tokio::fs::write(&outside, b"video data").await.unwrap();

        // Recycle disabled (would permanently delete), but the source is not under
        // any configured media root, so it must be refused rather than deleted.
        let config = RecycleBinConfig {
            enabled: false,
            base_path: tmp.path().join("recycle"),
            retention_days: 7,
            cleanup_enabled: true,
            validation_error: None,
            source_roots: vec![tmp.path().join("media-root")],
        };

        let result = recycle_file(&config, &outside, test_manifest()).await;
        assert!(result.is_err(), "out-of-root source must be refused");
        assert!(outside.exists(), "refused source must not be deleted");
    }

    #[cfg(windows)]
    #[test]
    fn source_root_containment_accepts_windows_case_and_separator_variants() {
        let config = RecycleBinConfig {
            enabled: false,
            base_path: PathBuf::from(r"C:\Recycle"),
            retention_days: 7,
            cleanup_enabled: true,
            validation_error: None,
            source_roots: vec![PathBuf::from(r"C:\Media\Movies")],
        };

        ensure_source_within_roots(&config, Path::new(r"c:/media/movies/Movie.mkv"))
            .expect("Windows source containment should normalize case and separators");
    }

    #[cfg(not(windows))]
    #[test]
    fn source_root_containment_is_case_sensitive_off_windows() {
        let config = RecycleBinConfig {
            enabled: false,
            base_path: PathBuf::from("/tmp/recycle"),
            retention_days: 7,
            cleanup_enabled: true,
            validation_error: None,
            source_roots: vec![PathBuf::from("/Data/Movies")],
        };

        let result = ensure_source_within_roots(&config, Path::new("/data/movies/Movie.mkv"));
        assert!(
            result.is_err(),
            "non-Windows source containment should fail closed on case mismatch"
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn source_root_containment_rejects_backslash_ambiguity_off_windows() {
        let config = RecycleBinConfig {
            enabled: false,
            base_path: PathBuf::from("/tmp/recycle"),
            retention_days: 7,
            cleanup_enabled: true,
            validation_error: None,
            source_roots: vec![PathBuf::from("/tmp/media")],
        };

        ensure_source_within_roots(&config, Path::new("/tmp/media/Movie.mkv"))
            .expect("ordinary slash-separated path should remain in-root");
        let result = ensure_source_within_roots(&config, Path::new(r"/tmp/media\evil.mkv"));
        assert!(
            result.is_err(),
            "non-Windows source containment must not treat backslash as a separator"
        );
    }

    #[test]
    fn source_root_containment_rejects_parent_dir_escape() {
        let config = RecycleBinConfig {
            enabled: false,
            base_path: PathBuf::from("/tmp/recycle"),
            retention_days: 7,
            cleanup_enabled: true,
            validation_error: None,
            source_roots: vec![PathBuf::from("/tmp/media")],
        };

        let result =
            ensure_source_within_roots(&config, Path::new("/tmp/media/../outside/Movie.mkv"));
        assert!(
            result.is_err(),
            "parent directory components must not escape a configured source root"
        );
    }

    #[test]
    fn source_root_containment_requires_file_parent_under_root() {
        let config = RecycleBinConfig {
            enabled: false,
            base_path: PathBuf::from("/tmp/recycle"),
            retention_days: 7,
            cleanup_enabled: true,
            validation_error: None,
            source_roots: vec![PathBuf::from("/tmp/media")],
        };

        ensure_source_within_roots(&config, Path::new("/tmp/media/Movie.mkv"))
            .expect("source file whose parent is the root should be allowed");
        ensure_source_within_roots(&config, Path::new("/tmp/media/Movies/Movie.mkv"))
            .expect("nested source file under the root should be allowed");
        assert!(
            ensure_source_within_roots(&config, Path::new("/tmp/media")).is_err(),
            "source path equal to the root must be rejected"
        );
        assert!(
            ensure_source_within_roots(&config, Path::new("/tmp/media/.")).is_err(),
            "source path normalized to the root must be rejected"
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn source_and_restore_roots_reject_backslash_ambiguity_off_windows() {
        let ambiguous_root = PathBuf::from(r"/tmp/media\root");
        let config = RecycleBinConfig {
            enabled: false,
            base_path: PathBuf::from("/tmp/recycle"),
            retention_days: 7,
            cleanup_enabled: true,
            validation_error: None,
            source_roots: vec![ambiguous_root.clone()],
        };

        let source_result =
            ensure_source_within_roots(&config, Path::new("/tmp/media/root/Movie.mkv"));
        assert!(
            source_result.is_err(),
            "non-Windows source containment must reject ambiguous raw roots"
        );
        assert!(
            !restore_destination_is_under_configured_roots(
                Path::new("/tmp/media/root/Movie.mkv"),
                &[ambiguous_root],
            ),
            "non-Windows restore containment must reject ambiguous raw roots"
        );
    }

    #[test]
    fn restore_destination_requires_file_under_root() {
        let roots = vec![PathBuf::from("/tmp/media")];

        assert!(
            restore_destination_is_under_configured_roots(
                Path::new("/tmp/media/Movie.mkv"),
                &roots
            ),
            "file whose parent is the root should be allowed"
        );
        assert!(
            restore_destination_is_under_configured_roots(
                Path::new("/tmp/media/Movies/Movie.mkv"),
                &roots
            ),
            "nested file under the root should be allowed"
        );
        assert!(
            !restore_destination_is_under_configured_roots(Path::new("/tmp/media"), &roots),
            "root itself is not a valid restore file destination"
        );
        assert!(
            !restore_destination_is_under_configured_roots(
                Path::new("/tmp/media/../media-restored"),
                &roots
            ),
            "sibling restore candidates must not escape the root"
        );
    }

    #[tokio::test]
    async fn test_recycle_refuses_source_when_roots_are_unknown() {
        let tmp = TempDir::new().unwrap();
        let source = tmp.path().join("source.mkv");
        tokio::fs::write(&source, b"video data").await.unwrap();

        let config = RecycleBinConfig {
            enabled: false,
            base_path: tmp.path().join("recycle"),
            retention_days: 7,
            cleanup_enabled: true,
            validation_error: None,
            source_roots: Vec::new(),
        };

        let result = recycle_file(&config, &source, test_manifest()).await;
        assert!(result.is_err(), "unknown roots must fail closed");
        assert!(source.exists(), "refused source must not be deleted");
    }

    #[tokio::test]
    async fn test_recycle_refuses_existing_directory_source_under_root() {
        let tmp = TempDir::new().unwrap();
        let media_root = tmp.path().join("media");
        let source_dir = media_root.join("Movie");
        tokio::fs::create_dir_all(&source_dir).await.unwrap();
        let config = RecycleBinConfig {
            enabled: false,
            base_path: tmp.path().join("recycle"),
            retention_days: 7,
            cleanup_enabled: true,
            validation_error: None,
            source_roots: vec![media_root],
        };

        let error = recycle_file(&config, &source_dir, test_manifest())
            .await
            .expect_err("directory sources must be refused");
        assert!(
            error.to_string().contains("not a regular file"),
            "unexpected error: {error}"
        );
        assert!(
            source_dir.exists(),
            "refused directory source must be left in place"
        );
    }

    #[tokio::test]
    async fn test_recycle_nonexistent_file_returns_none() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp.path().join("recycle"));

        let result = recycle_file(&config, &tmp.path().join("nope.mkv"), test_manifest())
            .await
            .unwrap();

        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_restore_returns_file() {
        let tmp = TempDir::new().unwrap();
        let recycle_dir = tmp.path().join("recycle");
        let source = tmp.path().join("test.mkv");
        let content = b"video data for restore test";
        tokio::fs::write(&source, content).await.unwrap();

        let config = test_config(&recycle_dir);
        let result = recycle_file(&config, &source, test_manifest())
            .await
            .unwrap()
            .unwrap();

        assert!(!source.exists());

        let restored_to = restore_from_recycle(&result.recycled_path, &source, false)
            .await
            .unwrap();

        assert_eq!(restored_to, source);
        assert!(source.exists());
        let restored = tokio::fs::read(&source).await.unwrap();
        assert_eq!(restored, content);
    }

    #[tokio::test]
    async fn test_exact_restore_returns_file_to_absent_destination() {
        let tmp = TempDir::new().unwrap();
        let recycle_dir = tmp.path().join("recycle");
        let source = tmp.path().join("test.mkv");
        let content = b"video data for exact restore";
        tokio::fs::write(&source, content).await.unwrap();

        let config = test_config(&recycle_dir);
        let result = recycle_file(&config, &source, test_manifest())
            .await
            .unwrap()
            .unwrap();

        restore_recycled_file_exact(&result.recycled_path, &source)
            .await
            .unwrap();

        assert_eq!(tokio::fs::read(&source).await.unwrap(), content);
        assert!(
            !result.recycled_path.exists(),
            "exact restore should remove the recycled source"
        );
    }

    #[tokio::test]
    async fn test_link_exact_restore_retains_source_for_rollback() {
        let tmp = TempDir::new().unwrap();
        let recycle_dir = tmp.path().join("recycle");
        let source = tmp.path().join("test.mkv");
        let content = b"video data for rollback";
        tokio::fs::write(&source, content).await.unwrap();

        let config = test_config(&recycle_dir);
        let result = recycle_file(&config, &source, test_manifest())
            .await
            .unwrap()
            .unwrap();

        link_recycled_file_to_exact_destination(&result.recycled_path, &source)
            .await
            .unwrap();

        assert_eq!(tokio::fs::read(&source).await.unwrap(), content);
        assert_eq!(
            tokio::fs::read(&result.recycled_path).await.unwrap(),
            content
        );
    }

    #[tokio::test]
    async fn test_pending_recycle_entry_is_ignored_by_expiry_sweep_until_committed() {
        let tmp = TempDir::new().unwrap();
        let recycle_dir = tmp.path().join("recycle");
        let source = tmp.path().join("test.mkv");
        tokio::fs::write(&source, b"video data").await.unwrap();

        let mut config = test_config(&recycle_dir);
        config.retention_days = 0;
        let result = recycle_file_pending(&config, &source, test_manifest())
            .await
            .unwrap()
            .unwrap();
        let mut manifest = read_manifest(&result.entry_dir).await.unwrap().unwrap();
        manifest.recycled_at = (Utc::now() - chrono::Duration::days(1)).to_rfc3339();
        write_manifest(&result.entry_dir, &manifest).await.unwrap();

        assert!(
            list_expired_committed_entries(&config)
                .await
                .unwrap()
                .is_empty()
        );

        commit_recycle_entry_without_replacement(&result)
            .await
            .unwrap();
        assert_eq!(
            list_expired_committed_entries(&config).await.unwrap().len(),
            1
        );
    }

    #[tokio::test]
    async fn test_stale_pending_entry_is_committed_by_reconciliation() {
        let tmp = TempDir::new().unwrap();
        let recycle_dir = tmp.path().join("recycle");
        let config = test_config(&recycle_dir);

        let stale_source = tmp.path().join("stale.mkv");
        tokio::fs::write(&stale_source, b"stale video")
            .await
            .unwrap();
        let stale = recycle_file_pending(&config, &stale_source, test_manifest())
            .await
            .unwrap()
            .unwrap();
        let mut stale_manifest = read_manifest(&stale.entry_dir).await.unwrap().unwrap();
        stale_manifest.recycled_at = (Utc::now() - chrono::Duration::days(2)).to_rfc3339();
        write_manifest(&stale.entry_dir, &stale_manifest)
            .await
            .unwrap();

        let fresh_source = tmp.path().join("fresh.mkv");
        tokio::fs::write(&fresh_source, b"fresh video")
            .await
            .unwrap();
        let fresh = recycle_file_pending(&config, &fresh_source, test_manifest())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(reconcile_stale_pending_entries(&config).await.unwrap(), 1);

        let stale_manifest = read_manifest(&stale.entry_dir).await.unwrap().unwrap();
        assert_eq!(
            stale_manifest.status.as_deref(),
            Some(RECYCLE_STATUS_COMMITTED),
            "stale pending entry must become committed"
        );
        let fresh_manifest = read_manifest(&fresh.entry_dir).await.unwrap().unwrap();
        assert_eq!(
            fresh_manifest.status.as_deref(),
            Some(RECYCLE_STATUS_PENDING),
            "a pending entry inside the grace window must stay pending"
        );

        let committed_ids = list_committed_entries(&config)
            .await
            .unwrap()
            .into_iter()
            .filter_map(|entry| entry.manifest.entry_id)
            .collect::<Vec<_>>();
        assert_eq!(committed_ids, vec![stale.entry_id.clone()]);

        assert_eq!(
            reconcile_stale_pending_entries(&config).await.unwrap(),
            0,
            "reconciliation must be idempotent"
        );
    }

    #[tokio::test]
    async fn test_restore_sampled_proof_mismatch_removes_partial_and_keeps_recycled_source() {
        let tmp = TempDir::new().unwrap();
        let recycled_path = tmp.path().join("movie.recycled");
        let destination = tmp.path().join("movie-restored.mkv");
        tokio::fs::write(&recycled_path, b"recycled source bytes")
            .await
            .unwrap();
        let destination_file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)
            .await
            .unwrap();

        let error = copy_recycled_to_claimed_destination_with_verifier(
            &recycled_path,
            &destination,
            destination_file,
            |source, dest| async move {
                tokio::fs::write(&dest, b"mismatched restored bytes")
                    .await
                    .unwrap();
                crate::fs_integrity::verify_same_file_async(&source, &dest).await
            },
        )
        .await
        .expect_err("sampled proof mismatch should fail restore");

        assert!(
            error.to_string().contains("copy verification failed"),
            "unexpected error: {error}"
        );
        assert!(
            recycled_path.exists(),
            "failed verification must keep the recycled source"
        );
        assert!(
            !destination.exists(),
            "failed verification must remove the partial destination"
        );
    }

    #[tokio::test]
    async fn test_exact_restore_refuses_occupied_destination_and_keeps_recycled_source() {
        let tmp = TempDir::new().unwrap();
        let recycled_path = tmp.path().join("movie.recycled");
        let destination = tmp.path().join("movie.mkv");
        tokio::fs::write(&recycled_path, b"old bytes")
            .await
            .unwrap();
        tokio::fs::write(&destination, b"unexpected occupant")
            .await
            .unwrap();

        let error = restore_recycled_file_exact(&recycled_path, &destination)
            .await
            .expect_err("occupied destination must be refused");

        assert!(
            error.to_string().contains("destination is occupied"),
            "unexpected error: {error}"
        );
        assert_eq!(tokio::fs::read(&recycled_path).await.unwrap(), b"old bytes");
        assert_eq!(
            tokio::fs::read(&destination).await.unwrap(),
            b"unexpected occupant"
        );
    }

    #[tokio::test]
    async fn test_restore_diverts_to_restored_sibling_on_conflict() {
        let tmp = TempDir::new().unwrap();
        let recycle_dir = tmp.path().join("recycle");
        let source = tmp.path().join("movie.mkv");
        let recycled_content = b"the recycled (older) file";
        tokio::fs::write(&source, recycled_content).await.unwrap();

        let config = test_config(&recycle_dir);
        let result = recycle_file(&config, &source, test_manifest())
            .await
            .unwrap()
            .unwrap();

        // A new live file now occupies the original path.
        let live_content = b"the current live file";
        tokio::fs::write(&source, live_content).await.unwrap();

        let restored_to = restore_from_recycle(&result.recycled_path, &source, false)
            .await
            .unwrap();

        // Live file must be untouched; restored file lands at a -restored sibling.
        assert_eq!(tokio::fs::read(&source).await.unwrap(), live_content);
        assert_eq!(restored_to, tmp.path().join("movie-restored.mkv"));
        assert_eq!(
            tokio::fs::read(&restored_to).await.unwrap(),
            recycled_content
        );
    }

    #[tokio::test]
    async fn test_restore_retries_existing_restored_sibling() {
        let tmp = TempDir::new().unwrap();
        let recycle_dir = tmp.path().join("recycle");
        let source = tmp.path().join("movie.mkv");
        let first_sibling = tmp.path().join("movie-restored.mkv");
        let recycled_content = b"the recycled file";
        tokio::fs::write(&source, recycled_content).await.unwrap();

        let config = test_config(&recycle_dir);
        let result = recycle_file(&config, &source, test_manifest())
            .await
            .unwrap()
            .unwrap();

        tokio::fs::write(&source, b"current live file")
            .await
            .unwrap();
        tokio::fs::write(&first_sibling, b"previous restored file")
            .await
            .unwrap();

        let restored_to = restore_from_recycle(&result.recycled_path, &source, false)
            .await
            .unwrap();

        let second_sibling = tmp.path().join("movie-restored-2.mkv");
        assert_eq!(restored_to, second_sibling);
        assert_eq!(
            tokio::fs::read(&source).await.unwrap(),
            b"current live file"
        );
        assert_eq!(
            tokio::fs::read(&first_sibling).await.unwrap(),
            b"previous restored file"
        );
        assert_eq!(
            tokio::fs::read(&second_sibling).await.unwrap(),
            recycled_content
        );
        assert!(
            !result.recycled_path.exists(),
            "recycled source should be removed after verified restore"
        );
    }

    #[tokio::test]
    async fn test_restore_overwrite_replaces_existing() {
        let tmp = TempDir::new().unwrap();
        let recycle_dir = tmp.path().join("recycle");
        let source = tmp.path().join("movie.mkv");
        let recycled_content = b"the recycled file to force back";
        tokio::fs::write(&source, recycled_content).await.unwrap();

        let config = test_config(&recycle_dir);
        let result = recycle_file(&config, &source, test_manifest())
            .await
            .unwrap()
            .unwrap();

        tokio::fs::write(&source, b"will be overwritten")
            .await
            .unwrap();

        let restored_to = restore_from_recycle(&result.recycled_path, &source, true)
            .await
            .unwrap();

        assert_eq!(restored_to, source);
        assert_eq!(tokio::fs::read(&source).await.unwrap(), recycled_content);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn restore_without_overwrite_refuses_symlink_recycle_source() {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("target.mkv");
        let recycled_link = tmp.path().join("recycled-link.mkv");
        let destination = tmp.path().join("movie.mkv");
        tokio::fs::write(&target, b"out of recycle bytes")
            .await
            .unwrap();
        std::os::unix::fs::symlink(&target, &recycled_link).unwrap();

        let error = restore_from_recycle(&recycled_link, &destination, false)
            .await
            .expect_err("symlink recycle source should be refused");

        assert!(
            error.to_string().contains("symlink"),
            "unexpected error: {error}"
        );
        assert!(
            tokio::fs::symlink_metadata(&recycled_link)
                .await
                .unwrap()
                .file_type()
                .is_symlink(),
            "refused recycle source link should remain"
        );
        assert!(target.exists(), "symlink target should remain untouched");
        assert!(
            !destination.exists(),
            "restore destination should not be created"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn restore_with_overwrite_refuses_symlink_recycle_source() {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("target.mkv");
        let recycled_link = tmp.path().join("recycled-link.mkv");
        let destination = tmp.path().join("movie.mkv");
        tokio::fs::write(&target, b"out of recycle bytes")
            .await
            .unwrap();
        tokio::fs::write(&destination, b"current live bytes")
            .await
            .unwrap();
        std::os::unix::fs::symlink(&target, &recycled_link).unwrap();

        let error = restore_from_recycle(&recycled_link, &destination, true)
            .await
            .expect_err("symlink recycle source should be refused");

        assert!(
            error.to_string().contains("symlink"),
            "unexpected error: {error}"
        );
        assert!(
            tokio::fs::symlink_metadata(&recycled_link)
                .await
                .unwrap()
                .file_type()
                .is_symlink(),
            "refused recycle source link should remain"
        );
        assert!(target.exists(), "symlink target should remain untouched");
        assert_eq!(
            tokio::fs::read(&destination).await.unwrap(),
            b"current live bytes",
            "overwrite restore should not touch the live destination"
        );
    }

    #[tokio::test]
    async fn test_purge_removes_expired_only() {
        let tmp = TempDir::new().unwrap();
        let recycle_dir = tmp.path().join("recycle");
        tokio::fs::create_dir_all(&recycle_dir).await.unwrap();
        write_test_sentinel(&recycle_dir).await;

        // Create an "expired" entry (recycled 30 days ago)
        let old_id = "20260205_120000000_abc123";
        let old_dir = recycle_dir.join(old_id);
        tokio::fs::create_dir_all(&old_dir).await.unwrap();
        let old_manifest = committed_manifest(
            old_id,
            (Utc::now() - chrono::Duration::days(30)).to_rfc3339(),
            "/old.mkv",
            None,
            "file_deleted",
        );
        tokio::fs::write(
            old_dir.join("manifest.json"),
            serde_json::to_string(&old_manifest).unwrap(),
        )
        .await
        .unwrap();
        tokio::fs::write(old_dir.join("old.mkv"), b"old")
            .await
            .unwrap();

        // Create a "fresh" entry (recycled just now)
        let new_id = "20260307_120000000_def456";
        let new_dir = recycle_dir.join(new_id);
        tokio::fs::create_dir_all(&new_dir).await.unwrap();
        let new_manifest = committed_manifest(
            new_id,
            Utc::now().to_rfc3339(),
            "/new.mkv",
            None,
            "file_deleted",
        );
        tokio::fs::write(
            new_dir.join("manifest.json"),
            serde_json::to_string(&new_manifest).unwrap(),
        )
        .await
        .unwrap();
        tokio::fs::write(new_dir.join("new.mkv"), b"new")
            .await
            .unwrap();

        let config = test_config(&recycle_dir);
        let purged = purge_expired(&config).await.unwrap();

        assert_eq!(purged, 1);
        assert!(!old_dir.exists(), "expired entry should be purged");
        assert!(new_dir.exists(), "fresh entry should survive");
    }

    #[tokio::test]
    async fn test_pending_entry_is_not_purged() {
        let tmp = TempDir::new().unwrap();
        let recycle_dir = tmp.path().join("recycle");
        tokio::fs::create_dir_all(&recycle_dir).await.unwrap();
        write_test_sentinel(&recycle_dir).await;

        let entry_id = "20260205_120000000_pendng";
        let entry_dir = recycle_dir.join(entry_id);
        tokio::fs::create_dir_all(&entry_dir).await.unwrap();
        let manifest = pending_manifest(
            entry_id,
            (Utc::now() - chrono::Duration::days(30)).to_rfc3339(),
        );
        tokio::fs::write(
            entry_dir.join("manifest.json"),
            serde_json::to_string(&manifest).unwrap(),
        )
        .await
        .unwrap();
        tokio::fs::write(entry_dir.join("S01E01.mkv"), b"old")
            .await
            .unwrap();

        let config = test_config(&recycle_dir);
        let purged = purge_expired(&config).await.unwrap();

        assert_eq!(purged, 0);
        assert!(entry_dir.exists(), "pending entry must not be purged");
    }

    #[tokio::test]
    async fn test_purge_requires_recycle_root_sentinel() {
        let tmp = TempDir::new().unwrap();
        let recycle_dir = tmp.path().join("recycle");
        tokio::fs::create_dir_all(&recycle_dir).await.unwrap();

        let entry_id = "20260205_120000000_nosent";
        let entry_dir = recycle_dir.join(entry_id);
        tokio::fs::create_dir_all(&entry_dir).await.unwrap();
        let manifest = committed_manifest(
            entry_id,
            (Utc::now() - chrono::Duration::days(30)).to_rfc3339(),
            "/old.mkv",
            None,
            "file_deleted",
        );
        tokio::fs::write(
            entry_dir.join("manifest.json"),
            serde_json::to_string(&manifest).unwrap(),
        )
        .await
        .unwrap();
        tokio::fs::write(entry_dir.join("old.mkv"), b"old")
            .await
            .unwrap();

        let config = test_config(&recycle_dir);
        let purged = purge_expired(&config).await.unwrap();

        assert_eq!(purged, 0);
        assert!(
            entry_dir.exists(),
            "entries need a root sentinel before purge"
        );
    }

    #[tokio::test]
    async fn test_empty_recycle_bin_skips_malformed_legacy_and_pending_entries() {
        let tmp = TempDir::new().unwrap();
        let recycle_dir = tmp.path().join("recycle");
        tokio::fs::create_dir_all(&recycle_dir).await.unwrap();
        write_test_sentinel(&recycle_dir).await;

        let legacy_dir = recycle_dir.join("20260205_120000000_legacy");
        tokio::fs::create_dir_all(&legacy_dir).await.unwrap();
        let mut legacy_manifest = test_manifest();
        legacy_manifest.recycled_at = (Utc::now() - chrono::Duration::days(30)).to_rfc3339();
        tokio::fs::write(
            legacy_dir.join("manifest.json"),
            serde_json::to_string(&legacy_manifest).unwrap(),
        )
        .await
        .unwrap();
        tokio::fs::write(legacy_dir.join("legacy.mkv"), b"legacy")
            .await
            .unwrap();

        let pending_id = "20260205_120000000_pendng";
        let pending_dir = recycle_dir.join(pending_id);
        tokio::fs::create_dir_all(&pending_dir).await.unwrap();
        let pending = pending_manifest(
            pending_id,
            (Utc::now() - chrono::Duration::days(30)).to_rfc3339(),
        );
        tokio::fs::write(
            pending_dir.join("manifest.json"),
            serde_json::to_string(&pending).unwrap(),
        )
        .await
        .unwrap();
        tokio::fs::write(pending_dir.join("pending.mkv"), b"pending")
            .await
            .unwrap();

        let malformed_dir = recycle_dir.join("20260205_120000000_badbad");
        tokio::fs::create_dir_all(&malformed_dir).await.unwrap();
        tokio::fs::write(malformed_dir.join("manifest.json"), b"{not json")
            .await
            .unwrap();
        tokio::fs::write(malformed_dir.join("bad.mkv"), b"bad")
            .await
            .unwrap();

        let config = test_config(&recycle_dir);
        let purged = purge_all(&config).await.unwrap();

        assert_eq!(purged, 0);
        assert!(legacy_dir.exists(), "legacy entry should be skipped");
        assert!(pending_dir.exists(), "pending entry should be skipped");
        assert!(malformed_dir.exists(), "malformed entry should be skipped");
    }

    #[tokio::test]
    async fn test_purge_for_title_removes_matching_only() {
        let tmp = TempDir::new().unwrap();
        let recycle_dir = tmp.path().join("recycle");
        tokio::fs::create_dir_all(&recycle_dir).await.unwrap();
        write_test_sentinel(&recycle_dir).await;

        let match_id = "20260307_120000000_aaa111";
        let match_dir = recycle_dir.join(match_id);
        tokio::fs::create_dir_all(&match_dir).await.unwrap();
        let match_manifest = committed_manifest(
            match_id,
            Utc::now().to_rfc3339(),
            "/data/movies/Movie/Movie.mkv",
            Some("title-123"),
            "file_deleted",
        );
        tokio::fs::write(
            match_dir.join("manifest.json"),
            serde_json::to_string(&match_manifest).unwrap(),
        )
        .await
        .unwrap();
        tokio::fs::write(match_dir.join("Movie.mkv"), b"data")
            .await
            .unwrap();

        let other_id = "20260307_120000000_bbb222";
        let other_dir = recycle_dir.join(other_id);
        tokio::fs::create_dir_all(&other_dir).await.unwrap();
        let other_manifest = committed_manifest(
            other_id,
            Utc::now().to_rfc3339(),
            "/data/movies/Other/Other.mkv",
            Some("title-456"),
            "file_deleted",
        );
        tokio::fs::write(
            other_dir.join("manifest.json"),
            serde_json::to_string(&other_manifest).unwrap(),
        )
        .await
        .unwrap();
        tokio::fs::write(other_dir.join("Other.mkv"), b"other")
            .await
            .unwrap();

        let config = test_config(&recycle_dir);
        let purged = purge_for_title(&config, "title-123").await.unwrap();

        assert_eq!(purged, 1);
        assert!(!match_dir.exists(), "matching title entry should be purged");
        assert!(other_dir.exists(), "different title entry should survive");
    }
}

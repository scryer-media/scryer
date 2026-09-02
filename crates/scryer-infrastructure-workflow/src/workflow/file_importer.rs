use async_trait::async_trait;
use scryer_application::{
    AppError, AppResult, FileImporter, ImportFileExecutionContext, ImportFilePermissions,
    ImportFileTransferProgress, ImportFileTransferProgressSender,
    fs_integrity::import_content_proof,
    location::verify::{StreamedContentHasher, VerifiedCopier},
};
use scryer_domain::{
    ImportDestinationDisposition, ImportFileIdentity, ImportFileResult, ImportMode,
    ImportSourceCleanupGuard, ImportSourceIdentity, ImportSourceIdentityKind, ImportSourceSnapshot,
    ImportStrategy, ImportTransferPhase, ImportVerification, StreamedContentHashes,
    VerificationDepth,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{self, BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, OnceLock, Weak,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, Instant, SystemTime};

#[cfg(unix)]
use std::ffi::CString;
#[cfg(unix)]
use std::os::unix::{
    ffi::OsStrExt,
    fs::{MetadataExt, PermissionsExt, symlink},
};
#[cfg(windows)]
use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
#[cfg(unix)]
use std::path::Component;
#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS, GetFileInformationByHandle,
};

const TRANSIENT_BAD_FILE_DESCRIPTOR_ERRNO: i32 = 9;
const IMPORT_COPY_MAX_ATTEMPTS: usize = 3;
const IMPORT_COPY_BUFFER_BYTES: usize = 8 * 1024 * 1024;
const IMPORT_FILE_STALL_TIMEOUT_ENV: &str = "SCRYER_IMPORT_FILE_STALL_TIMEOUT_SECONDS";
const DEFAULT_IMPORT_FILE_STALL_TIMEOUT_SECONDS: u64 = 30 * 60;
const MIN_IMPORT_FILE_STALL_TIMEOUT_SECONDS: u64 = 5 * 60;
const MAX_IMPORT_FILE_STALL_TIMEOUT_SECONDS: u64 = 24 * 60 * 60;
const IMPORT_COPY_WORKERS_PER_VOLUME_ENV: &str = "SCRYER_IMPORT_COPY_WORKERS_PER_VOLUME";
const IMPORT_FAST_WORKERS_PER_CLIENT_ENV: &str = "SCRYER_IMPORT_FAST_WORKERS_PER_CLIENT";
const DEFAULT_IMPORT_COPY_WORKERS_PER_VOLUME: usize = 2;
const MAX_IMPORT_COPY_WORKERS_PER_VOLUME: usize = 8;
const DEFAULT_IMPORT_FAST_WORKERS_PER_CLIENT: usize = 1;
const MAX_IMPORT_FAST_WORKERS_PER_CLIENT: usize = 4;
const IMPORT_PREPARATION_WORKERS: usize = 8;
static IMPORT_PLACEMENT_LIMITS: OnceLock<(usize, usize)> = OnceLock::new();

#[derive(Clone)]
struct KeyedImportPermitTable {
    permits: Arc<tokio::sync::Mutex<HashMap<String, Weak<tokio::sync::Semaphore>>>>,
    workers_per_key: usize,
}

impl KeyedImportPermitTable {
    fn new(workers_per_key: usize) -> Self {
        Self {
            permits: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            workers_per_key,
        }
    }

    async fn acquire(&self, key: &str, lane: &'static str) -> tokio::sync::OwnedSemaphorePermit {
        let semaphore = {
            let mut permits = self.permits.lock().await;
            permits.retain(|_, permit| permit.strong_count() > 0);
            if let Some(permit) = permits.get(key).and_then(Weak::upgrade) {
                permit
            } else {
                let permit = Arc::new(tokio::sync::Semaphore::new(self.workers_per_key));
                permits.insert(key.to_string(), Arc::downgrade(&permit));
                permit
            }
        };
        let available_before = semaphore.available_permits();
        let semaphore_for_metrics = semaphore.clone();
        let started = Instant::now();
        let permit = semaphore
            .acquire_owned()
            .await
            .expect("import lane semaphore is never closed");
        let wait = started.elapsed();
        let active_permits = self
            .workers_per_key
            .saturating_sub(semaphore_for_metrics.available_permits());
        metrics::counter!(
            "scryer_import_lane_acquisitions_total",
            "lane" => lane,
            "saturated" => (available_before == 0).to_string()
        )
        .increment(1);
        metrics::histogram!("scryer_import_lane_wait_seconds", "lane" => lane)
            .record(wait.as_secs_f64());
        metrics::histogram!("scryer_import_lane_active_permits", "lane" => lane)
            .record(active_permits as f64);
        tracing::debug!(
            lane,
            lane_key = %opaque_lane_key(key),
            wait_ms = wait.as_millis(),
            active_permits,
            capacity = self.workers_per_key,
            saturated = available_before == 0,
            "acquired import workstream permit"
        );
        permit
    }
}

#[derive(Clone)]
struct ImportPlacementCoordinator {
    preparation: Arc<tokio::sync::Semaphore>,
    fast: KeyedImportPermitTable,
    copy: KeyedImportPermitTable,
}

impl ImportPlacementCoordinator {
    fn from_environment() -> Self {
        let (fast_workers, copy_workers) = *IMPORT_PLACEMENT_LIMITS.get_or_init(|| {
            (
                import_worker_limit_from_environment(
                    IMPORT_FAST_WORKERS_PER_CLIENT_ENV,
                    DEFAULT_IMPORT_FAST_WORKERS_PER_CLIENT,
                    MAX_IMPORT_FAST_WORKERS_PER_CLIENT,
                ),
                import_worker_limit_from_environment(
                    IMPORT_COPY_WORKERS_PER_VOLUME_ENV,
                    DEFAULT_IMPORT_COPY_WORKERS_PER_VOLUME,
                    MAX_IMPORT_COPY_WORKERS_PER_VOLUME,
                ),
            )
        });
        Self::new(fast_workers, copy_workers)
    }

    fn new(fast_workers_per_client: usize, copy_workers_per_volume: usize) -> Self {
        Self {
            preparation: Arc::new(tokio::sync::Semaphore::new(IMPORT_PREPARATION_WORKERS)),
            fast: KeyedImportPermitTable::new(fast_workers_per_client),
            copy: KeyedImportPermitTable::new(copy_workers_per_volume),
        }
    }

    async fn acquire_preparation(&self) -> tokio::sync::OwnedSemaphorePermit {
        self.preparation
            .clone()
            .acquire_owned()
            .await
            .expect("import preparation semaphore is never closed")
    }
}

pub struct FsFileImporter {
    placement: ImportPlacementCoordinator,
    worker_executable: Option<PathBuf>,
}

impl Default for FsFileImporter {
    fn default() -> Self {
        Self::new()
    }
}

impl FsFileImporter {
    pub fn new() -> Self {
        Self {
            placement: ImportPlacementCoordinator::from_environment(),
            worker_executable: None,
        }
    }

    pub fn with_worker_executable(worker_executable: PathBuf) -> Self {
        Self {
            placement: ImportPlacementCoordinator::from_environment(),
            worker_executable: Some(worker_executable),
        }
    }
}

fn import_worker_limit_from_environment(key: &str, default: usize, maximum: usize) -> usize {
    let Ok(raw) = std::env::var(key) else {
        return default;
    };
    let value = raw.trim();
    let parsed = parse_import_worker_limit(Some(value), default, maximum);
    match value.parse::<usize>() {
        Ok(0) | Err(_) => {
            tracing::warn!(
                key,
                value,
                default,
                "invalid import worker limit; using default"
            );
        }
        Ok(requested) if requested > maximum => {
            tracing::warn!(
                key,
                requested,
                maximum,
                "import worker limit exceeds maximum; clamping"
            );
        }
        Ok(_) => {}
    }
    parsed
}

fn parse_import_worker_limit(value: Option<&str>, default: usize, maximum: usize) -> usize {
    match value
        .map(str::trim)
        .and_then(|value| value.parse::<usize>().ok())
    {
        Some(parsed) if parsed > 0 => parsed.min(maximum),
        _ => default,
    }
}

fn opaque_lane_key(key: &str) -> String {
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct FileFingerprint {
    len: u64,
    modified: Option<SystemTime>,
    #[cfg(unix)]
    dev: u64,
    #[cfg(unix)]
    ino: u64,
    #[cfg(unix)]
    uid: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct DirectoryFingerprint {
    #[cfg(unix)]
    dev: u64,
    #[cfg(unix)]
    ino: u64,
    #[cfg(windows)]
    volume_serial_number: u32,
    #[cfg(windows)]
    file_index: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum ImportSourceKind {
    Regular,
    Symlink {
        source_link_target: PathBuf,
        resolved_target: PathBuf,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ImportSourceFingerprint {
    file: FileFingerprint,
    kind: ImportSourceKind,
}

fn cleanup_guard(
    source: &Path,
    dest: &Path,
    source_fingerprint: &ImportSourceFingerprint,
    size: u64,
) -> AppResult<ImportSourceCleanupGuard> {
    let source_proof = import_content_proof(source).map_err(|error| {
        AppError::Repository(format!(
            "failed to build import source cleanup proof for source {}: {}",
            source.display(),
            error
        ))
    })?;
    let dest_proof = import_content_proof(dest).map_err(|error| {
        AppError::Repository(format!(
            "failed to build import source cleanup proof for destination {}: {}",
            dest.display(),
            error
        ))
    })?;
    if source_proof.size_bytes != size || dest_proof.size_bytes != size {
        return Err(AppError::Repository(format!(
            "failed to build import source cleanup proof because source/destination sizes changed: source={} dest={} expected={}",
            source_proof.size_bytes, dest_proof.size_bytes, size
        )));
    }
    if source_proof != dest_proof {
        return Err(AppError::Repository(format!(
            "failed to build import source cleanup proof because source and destination content differ: source={} dest={}",
            source.display(),
            dest.display()
        )));
    }

    Ok(ImportSourceCleanupGuard {
        source_path: source.to_path_buf(),
        dest_path: dest.to_path_buf(),
        size_bytes: size,
        source_identity: source_identity_from_fingerprint(source_fingerprint),
        source_proof,
        dest_proof,
    })
}

fn cleanup_guard_after_placement(
    source_cleanup_required: bool,
    source: &Path,
    dest: &Path,
    source_fingerprint: &ImportSourceFingerprint,
    size: u64,
) -> AppResult<Option<ImportSourceCleanupGuard>> {
    if !source_cleanup_required {
        return Ok(None);
    }

    match cleanup_guard(source, dest, source_fingerprint, size) {
        Ok(guard) => Ok(Some(guard)),
        Err(error) => {
            let _ = std::fs::remove_file(dest);
            Err(error)
        }
    }
}

fn source_identity_from_fingerprint(
    source_fingerprint: &ImportSourceFingerprint,
) -> ImportSourceIdentity {
    ImportSourceIdentity {
        file: import_file_identity_from_fingerprint(&source_fingerprint.file),
        kind: match &source_fingerprint.kind {
            ImportSourceKind::Regular => ImportSourceIdentityKind::Regular,
            ImportSourceKind::Symlink {
                source_link_target,
                resolved_target,
            } => ImportSourceIdentityKind::Symlink {
                source_link_target: source_link_target.clone(),
                resolved_target: resolved_target.clone(),
            },
        },
    }
}

fn stable_import_source_snapshot(
    path: &Path,
    initial_fingerprint: Option<&ImportSourceFingerprint>,
) -> AppResult<ImportSourceSnapshot> {
    let initial = match initial_fingerprint {
        Some(fingerprint) => fingerprint.clone(),
        None => fingerprint_import_source(path)?,
    };
    let proof = import_content_proof(path)?;
    ensure_same_source(path, &initial)?;
    Ok(ImportSourceSnapshot {
        identity: source_identity_from_fingerprint(&initial),
        proof,
    })
}

fn snapshot_import_source_blocking(path: PathBuf) -> AppResult<ImportSourceSnapshot> {
    stable_import_source_snapshot(&path, None)
}

fn ensure_expected_source_snapshot(
    path: &Path,
    current_fingerprint: &ImportSourceFingerprint,
    expected: Option<&ImportSourceSnapshot>,
) -> AppResult<()> {
    let Some(expected) = expected else {
        return Ok(());
    };
    let actual = stable_import_source_snapshot(path, Some(current_fingerprint))?;
    if &actual != expected {
        return Err(AppError::Repository(format!(
            "import source changed after validation: {}",
            path.display()
        )));
    }
    Ok(())
}

fn import_file_identity_from_fingerprint(file: &FileFingerprint) -> ImportFileIdentity {
    ImportFileIdentity {
        len: file.len,
        modified: file.modified,
        #[cfg(unix)]
        dev: file.dev,
        #[cfg(unix)]
        ino: file.ino,
    }
}

fn fingerprint_import_source(path: &Path) -> AppResult<ImportSourceFingerprint> {
    let link_meta = std::fs::symlink_metadata(path).map_err(|e| {
        AppError::Repository(format!(
            "import path not found or inaccessible: {}: {}",
            path.display(),
            e
        ))
    })?;
    let file_type = link_meta.file_type();
    if file_type.is_symlink() {
        let source_link_target = std::fs::read_link(path).map_err(|e| {
            AppError::Repository(format!(
                "failed to read import symlink target: {}: {}",
                path.display(),
                e
            ))
        })?;
        let resolved_target = resolve_symlink_target(path, &source_link_target);
        let target_meta = std::fs::metadata(&resolved_target).map_err(|e| {
            AppError::Repository(format!(
                "import symlink target not found or inaccessible: {} -> {}: {}",
                path.display(),
                resolved_target.display(),
                e
            ))
        })?;
        return Ok(ImportSourceFingerprint {
            file: fingerprint_from_metadata(&target_meta)?,
            kind: ImportSourceKind::Symlink {
                source_link_target,
                resolved_target,
            },
        });
    }
    if !file_type.is_file() {
        return Err(AppError::Repository(format!(
            "import path is not a regular file: {}",
            path.display()
        )));
    }
    Ok(ImportSourceFingerprint {
        file: fingerprint_from_metadata(&link_meta)?,
        kind: ImportSourceKind::Regular,
    })
}

fn fingerprint_from_metadata(metadata: &std::fs::Metadata) -> AppResult<FileFingerprint> {
    if !metadata.is_file() {
        return Err(AppError::Repository(
            "import path is not a regular file".into(),
        ));
    }
    Ok(FileFingerprint {
        len: metadata.len(),
        modified: metadata.modified().ok(),
        #[cfg(unix)]
        dev: metadata.dev(),
        #[cfg(unix)]
        ino: metadata.ino(),
        #[cfg(unix)]
        uid: metadata.uid(),
    })
}

fn directory_fingerprint_from_path(path: &Path) -> AppResult<DirectoryFingerprint> {
    let metadata = std::fs::metadata(path).map_err(|e| {
        AppError::Repository(format!(
            "failed to stat destination directory {}: {}",
            path.display(),
            e
        ))
    })?;
    if !metadata.is_dir() {
        return Err(AppError::Repository(
            "import destination parent is not a directory".into(),
        ));
    }
    #[cfg(unix)]
    {
        Ok(DirectoryFingerprint {
            dev: metadata.dev(),
            ino: metadata.ino(),
        })
    }
    #[cfg(windows)]
    {
        let directory = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)
            .map_err(|e| {
                AppError::Repository(format!(
                    "failed to open destination directory {} for identity check: {}",
                    path.display(),
                    e
                ))
            })?;
        let mut info = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::zeroed();
        let result =
            unsafe { GetFileInformationByHandle(directory.as_raw_handle(), info.as_mut_ptr()) };
        if result == 0 {
            return Err(AppError::Repository(format!(
                "failed to read destination directory identity {}: {}",
                path.display(),
                std::io::Error::last_os_error()
            )));
        }
        let info = unsafe { info.assume_init() };
        let file_index = ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64;
        Ok(DirectoryFingerprint {
            volume_serial_number: info.dwVolumeSerialNumber,
            file_index,
        })
    }
    #[cfg(not(any(unix, windows)))]
    {
        Ok(DirectoryFingerprint {})
    }
}

fn ensure_same_source(path: &Path, expected: &ImportSourceFingerprint) -> AppResult<()> {
    let actual = fingerprint_import_source(path)?;
    if &actual != expected {
        return Err(AppError::Repository(format!(
            "import source changed during import: {}",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(unix)]
fn ensure_same_file_identity(path: &Path, expected: &FileFingerprint) -> AppResult<()> {
    let actual = fingerprint_import_source(path)?.file;
    if actual.dev != expected.dev || actual.ino != expected.ino {
        return Err(AppError::Repository(format!(
            "import destination is not linked to the expected source: {}",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_same_file_identity(path: &Path, expected: &FileFingerprint) -> AppResult<()> {
    let actual = fingerprint_import_source(path)?.file;
    if actual != *expected {
        return Err(AppError::Repository(format!(
            "import destination does not match the expected source: {}",
            path.display()
        )));
    }
    Ok(())
}

fn io_other(error: impl ToString) -> io::Error {
    io::Error::other(error.to_string())
}

fn resolve_symlink_target(source: &Path, target: &Path) -> PathBuf {
    if target.is_absolute() {
        target.to_path_buf()
    } else {
        source
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(target)
    }
}

#[cfg(unix)]
fn build_destination_symlink_target(dest: &Path, resolved_target: &Path) -> PathBuf {
    let dest_parent = dest.parent().unwrap_or_else(|| Path::new("/"));
    relative_path_between(dest_parent, resolved_target)
        .filter(|relative| !relative.as_os_str().is_empty())
        .unwrap_or_else(|| resolved_target.to_path_buf())
}

#[cfg(unix)]
fn relative_path_between(from_dir: &Path, to_path: &Path) -> Option<PathBuf> {
    if !from_dir.is_absolute() || !to_path.is_absolute() {
        return None;
    }

    let from_components = from_dir.components().collect::<Vec<_>>();
    let to_components = to_path.components().collect::<Vec<_>>();
    if !matches!(from_components.first(), Some(Component::RootDir))
        || !matches!(to_components.first(), Some(Component::RootDir))
    {
        return None;
    }

    let mut shared_prefix_len = 0;
    while shared_prefix_len < from_components.len()
        && shared_prefix_len < to_components.len()
        && from_components[shared_prefix_len] == to_components[shared_prefix_len]
    {
        shared_prefix_len += 1;
    }

    let mut relative = PathBuf::new();
    for component in &from_components[shared_prefix_len..] {
        if !matches!(component, Component::CurDir) {
            relative.push("..");
        }
    }
    for component in &to_components[shared_prefix_len..] {
        relative.push(component.as_os_str());
    }

    Some(relative)
}

#[derive(Clone, Copy, Debug, Default)]
struct ImportFileOptions {
    #[cfg(test)]
    force_cross_device_move: bool,
    #[cfg(test)]
    force_copy_verification_failure: bool,
    /// SC-006: flip a byte in the destination after it is written, before it is
    /// verified. The only way to exercise "a corrupted destination copy is
    /// detected before source removal" without an unreliable disk.
    #[cfg(test)]
    force_destination_corruption: bool,
    /// Make the full-depth read-back report itself unsupported, as a filesystem
    /// that refuses the cache-bypassed re-read would (FR-042 quick floor).
    #[cfg(test)]
    force_read_back_unsupported: bool,
    #[cfg(test)]
    force_delete_failure: bool,
    #[cfg(test)]
    force_transient_copy_failures: u8,
    #[cfg(test)]
    force_non_transient_copy_failure: bool,
    #[cfg(all(test, unix))]
    force_foreign_source_uid: bool,
    #[cfg(all(test, unix))]
    force_destination_uid_mismatch: bool,
}

#[cfg(test)]
fn force_cross_device_move(options: &ImportFileOptions) -> bool {
    options.force_cross_device_move
}

#[cfg(not(test))]
fn force_cross_device_move(_: &ImportFileOptions) -> bool {
    false
}

#[cfg(test)]
fn force_copy_verification_failure(options: &ImportFileOptions) -> bool {
    options.force_copy_verification_failure
}

/// The copier the copy path proves with. Tests substitute a read-back opener to
/// reach the quick floor; production always uses the real one.
#[cfg(test)]
fn verified_copier_for(options: &ImportFileOptions) -> VerifiedCopier {
    if options.force_read_back_unsupported {
        return VerifiedCopier::with_read_back_opener(Arc::new(|_: &Path| {
            scryer_application::location::verify::ReadBackHandle::Unsupported(
                "forced unsupported read-back".to_string(),
            )
        }));
    }
    VerifiedCopier::new()
}

#[cfg(not(test))]
fn verified_copier_for(_: &ImportFileOptions) -> VerifiedCopier {
    VerifiedCopier::new()
}

/// Corrupt the destination between write and verification (SC-006).
#[cfg(test)]
fn force_destination_corruption(options: &ImportFileOptions, dest: &Path) {
    if !options.force_destination_corruption {
        return;
    }
    let mut bytes = std::fs::read(dest).expect("read destination to corrupt");
    let last = bytes.last_mut().expect("destination has bytes to corrupt");
    *last ^= 0xff;
    std::fs::write(dest, bytes).expect("write corrupted destination");
}

#[cfg(not(test))]
fn force_destination_corruption(_: &ImportFileOptions, _: &Path) {}

#[cfg(not(test))]
fn force_copy_verification_failure(_: &ImportFileOptions) -> bool {
    false
}

#[cfg(test)]
fn force_delete_failure(options: &ImportFileOptions) -> bool {
    options.force_delete_failure
}

#[cfg(not(test))]
fn force_delete_failure(_: &ImportFileOptions) -> bool {
    false
}

#[cfg(test)]
fn force_copy_attempt_error(
    temp_file: &mut std::fs::File,
    options: &ImportFileOptions,
    attempt: usize,
) -> std::io::Result<()> {
    if options.force_non_transient_copy_failure {
        temp_file.write_all(b"partial non-transient copy failure")?;
        temp_file.flush()?;
        return Err(io_other("forced non-transient copy failure"));
    }
    if attempt <= usize::from(options.force_transient_copy_failures) {
        temp_file.write_all(format!("partial transient copy failure {attempt}").as_bytes())?;
        temp_file.flush()?;
        return Err(std::io::Error::from_raw_os_error(
            TRANSIENT_BAD_FILE_DESCRIPTOR_ERRNO,
        ));
    }
    Ok(())
}

#[cfg(not(test))]
fn force_copy_attempt_error(
    _temp_file: &mut std::fs::File,
    _options: &ImportFileOptions,
    _attempt: usize,
) -> std::io::Result<()> {
    Ok(())
}

#[cfg(all(test, unix))]
fn force_foreign_source_uid(options: &ImportFileOptions) -> bool {
    options.force_foreign_source_uid
}

#[cfg(all(not(test), unix))]
fn force_foreign_source_uid(_: &ImportFileOptions) -> bool {
    false
}

#[cfg(all(test, unix))]
fn force_destination_uid_mismatch(options: &ImportFileOptions) -> bool {
    options.force_destination_uid_mismatch
}

#[cfg(all(not(test), unix))]
fn force_destination_uid_mismatch(_: &ImportFileOptions) -> bool {
    false
}

#[cfg(unix)]
fn effective_uid() -> u32 {
    unsafe { libc::geteuid() as u32 }
}

#[cfg(unix)]
fn source_uid_matches_scryer(
    source_fingerprint: &ImportSourceFingerprint,
    options: &ImportFileOptions,
) -> bool {
    !force_foreign_source_uid(options) && source_fingerprint.file.uid == effective_uid()
}

#[cfg(not(unix))]
fn source_uid_matches_scryer(
    _source_fingerprint: &ImportSourceFingerprint,
    _options: &ImportFileOptions,
) -> bool {
    true
}

#[cfg(unix)]
fn verify_imported_regular_file_uid(path: &Path, options: &ImportFileOptions) -> AppResult<()> {
    if force_destination_uid_mismatch(options) {
        return Err(AppError::Repository(format!(
            "import destination is not owned by the Scryer process uid: {}",
            path.display()
        )));
    }
    let metadata = std::fs::metadata(path).map_err(|error| {
        AppError::Repository(format!(
            "failed to inspect import destination ownership {}: {}",
            path.display(),
            error
        ))
    })?;
    if metadata.is_file() && metadata.uid() != effective_uid() {
        return Err(AppError::Repository(format!(
            "import destination is owned by uid {}, expected Scryer uid {}: {}",
            metadata.uid(),
            effective_uid(),
            path.display()
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
fn verify_imported_regular_file_uid(_path: &Path, _options: &ImportFileOptions) -> AppResult<()> {
    Ok(())
}

#[cfg(unix)]
fn parse_chmod_mask(mask: &str) -> io::Result<u32> {
    if !(mask.len() == 3 || mask.len() == 4)
        || !mask.bytes().all(|byte| (b'0'..=b'7').contains(&byte))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid chmod mask '{mask}'"),
        ));
    }
    u32::from_str_radix(mask, 8).map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))
}

#[cfg(unix)]
fn chmod_target_mode(current_mode: u32, mask: &str, parsed_mask: u32) -> u32 {
    if mask.len() < 4 {
        (current_mode & !0o777) | parsed_mask
    } else {
        parsed_mask
    }
}

#[cfg(unix)]
fn apply_chmod(path: &Path, mask: &str) -> io::Result<()> {
    let metadata = std::fs::metadata(path)?;
    let parsed_mask = parse_chmod_mask(mask)?;
    let target_mode = chmod_target_mode(metadata.permissions().mode(), mask, parsed_mask);
    let mut permissions = metadata.permissions();
    permissions.set_mode(target_mode);
    std::fs::set_permissions(path, permissions)
}

#[cfg(unix)]
fn file_chmod_mask(permissions: &ImportFilePermissions) -> Option<String> {
    if !permissions.set_permissions_linux {
        return None;
    }
    if let Some(mask) = permissions.file_chmod.as_ref() {
        return Some(mask.clone());
    }
    let folder_mask = permissions.folder_chmod.as_ref()?;
    let parsed = parse_chmod_mask(folder_mask).ok()?;
    Some(format!("{:o}", parsed & !0o111))
}

#[cfg(unix)]
fn file_permission_metadata_changes(permissions: &ImportFilePermissions) -> bool {
    permissions.set_permissions_linux
        && (permissions.chown_group.is_some() || file_chmod_mask(permissions).is_some())
}

#[cfg(not(unix))]
fn file_permission_metadata_changes(_permissions: &ImportFilePermissions) -> bool {
    false
}

#[cfg(unix)]
fn resolve_group_id(group: &str) -> io::Result<libc::gid_t> {
    if group.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "group cannot be empty",
        ));
    }

    if group.bytes().all(|byte| byte.is_ascii_digit()) {
        let parsed = group
            .parse::<u32>()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        return Ok(parsed as libc::gid_t);
    }

    let group_name = CString::new(group)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "group contains NUL byte"))?;
    let mut group_record = unsafe { std::mem::zeroed::<libc::group>() };
    let mut result = std::ptr::null_mut();
    let mut buffer_len = unsafe { libc::sysconf(libc::_SC_GETGR_R_SIZE_MAX) };
    if buffer_len < 1024 {
        buffer_len = 16 * 1024;
    }
    let mut buffer = vec![0u8; buffer_len as usize];

    loop {
        let status = unsafe {
            libc::getgrnam_r(
                group_name.as_ptr(),
                &mut group_record,
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut result,
            )
        };
        if status == 0 {
            if result.is_null() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("unknown group '{group}'"),
                ));
            }
            return Ok(group_record.gr_gid);
        }
        if status == libc::ERANGE {
            buffer.resize(buffer.len().saturating_mul(2), 0);
            continue;
        }
        return Err(io::Error::from_raw_os_error(status));
    }
}

#[cfg(unix)]
fn apply_group(path: &Path, group: &str) -> io::Result<()> {
    let gid = resolve_group_id(group)?;
    let c_path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL byte"))?;
    let result = unsafe { libc::chown(c_path.as_ptr(), libc::uid_t::MAX, gid) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(unix)]
pub fn apply_file_permissions_best_effort(path: &Path, permissions: &ImportFilePermissions) {
    if !permissions.set_permissions_linux {
        return;
    }
    if let Some(group) = permissions.chown_group.as_deref()
        && let Err(error) = apply_group(path, group)
    {
        tracing::warn!(
            path = %path.display(),
            group,
            error = %error,
            "unable to apply imported file group"
        );
    }
    if let Some(mask) = file_chmod_mask(permissions)
        && let Err(error) = apply_chmod(path, &mask)
    {
        tracing::warn!(
            path = %path.display(),
            mask,
            error = %error,
            "unable to apply imported file permissions"
        );
    }
}

#[cfg(not(unix))]
pub fn apply_file_permissions_best_effort(_path: &Path, _permissions: &ImportFilePermissions) {}

#[cfg(unix)]
pub fn apply_directory_permissions_best_effort(path: &Path, permissions: &ImportFilePermissions) {
    if !permissions.set_permissions_linux {
        return;
    }
    if let Some(group) = permissions.chown_group.as_deref()
        && let Err(error) = apply_group(path, group)
    {
        tracing::warn!(
            path = %path.display(),
            group,
            error = %error,
            "unable to apply imported folder group"
        );
    }
    if let Some(mask) = permissions.folder_chmod.as_deref()
        && let Err(error) = apply_chmod(path, mask)
    {
        tracing::warn!(
            path = %path.display(),
            mask,
            error = %error,
            "unable to apply imported folder permissions"
        );
    }
}

#[cfg(not(unix))]
pub fn apply_directory_permissions_best_effort(_path: &Path, _permissions: &ImportFilePermissions) {
}

pub fn missing_destination_dirs(parent: &Path) -> Vec<PathBuf> {
    let mut missing = Vec::new();
    let mut current = Some(parent);
    while let Some(path) = current {
        if path.as_os_str().is_empty() || path.exists() {
            break;
        }
        missing.push(path.to_path_buf());
        current = path.parent();
    }
    missing.reverse();
    missing
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ImportDestinationGuard {
    requested_path: PathBuf,
    parent_path: PathBuf,
    approved_parent_canonical: PathBuf,
    approved_parent_fingerprint: DirectoryFingerprint,
}

fn destination_parent_for_guard(dest: &Path) -> &Path {
    dest.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn prepare_import_destination(
    source: &Path,
    dest: &Path,
    permissions: &ImportFilePermissions,
) -> AppResult<(ImportSourceFingerprint, u64, ImportDestinationGuard)> {
    let source_fingerprint = fingerprint_import_source(source)?;
    let size = source_fingerprint.file.len;
    if size == 0 {
        return Err(AppError::Repository(format!(
            "import source is zero bytes: {}",
            source.display()
        )));
    }

    let parent = destination_parent_for_guard(dest);
    let missing_dirs = missing_destination_dirs(parent);
    std::fs::create_dir_all(parent).map_err(|e| {
        AppError::Repository(format!(
            "failed to create destination directory {}: {}",
            parent.display(),
            e
        ))
    })?;
    let approved_parent_canonical = std::fs::canonicalize(parent).map_err(|e| {
        AppError::Repository(format!(
            "failed to inspect destination directory {}: {}",
            parent.display(),
            e
        ))
    })?;
    let approved_parent_fingerprint = directory_fingerprint_from_path(parent)?;
    let destination_guard = ImportDestinationGuard {
        requested_path: dest.to_path_buf(),
        parent_path: parent.to_path_buf(),
        approved_parent_canonical,
        approved_parent_fingerprint,
    };

    for dir in &missing_dirs {
        apply_directory_permissions_best_effort(dir, permissions);
    }
    validate_import_destination_parent(&destination_guard, dest)?;

    Ok((source_fingerprint, size, destination_guard))
}

fn validate_import_destination_parent(
    guard: &ImportDestinationGuard,
    returned_dest: &Path,
) -> AppResult<()> {
    if returned_dest != guard.requested_path {
        return Err(AppError::Repository(format!(
            "import destination changed during placement: expected {} got {}",
            guard.requested_path.display(),
            returned_dest.display()
        )));
    }

    let current_parent_canonical = std::fs::canonicalize(&guard.parent_path).map_err(|e| {
        AppError::Repository(format!(
            "failed to re-check destination directory {}: {}",
            guard.parent_path.display(),
            e
        ))
    })?;
    if current_parent_canonical != guard.approved_parent_canonical {
        return Err(AppError::Repository(format!(
            "import destination parent changed during placement: {} resolved to {} before import and {} after import",
            guard.parent_path.display(),
            guard.approved_parent_canonical.display(),
            current_parent_canonical.display()
        )));
    }
    let current_parent_fingerprint = directory_fingerprint_from_path(&guard.parent_path)?;
    if current_parent_fingerprint != guard.approved_parent_fingerprint {
        return Err(AppError::Repository(format!(
            "import destination parent changed during placement: {}",
            guard.parent_path.display()
        )));
    }

    Ok(())
}

fn ensure_import_destination_absent(dest: &Path) -> AppResult<()> {
    match std::fs::symlink_metadata(dest) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(AppError::Repository(format!(
            "import destination appeared before copy execution: {}",
            dest.display()
        ))),
        Err(error) => Err(AppError::Repository(format!(
            "failed to inspect import destination {} before copy: {error}",
            dest.display()
        ))),
    }
}

fn validate_import_destination_file(guard: &ImportDestinationGuard) -> AppResult<()> {
    let metadata = std::fs::symlink_metadata(&guard.requested_path).map_err(|e| {
        AppError::Repository(format!(
            "failed to inspect imported destination {}: {}",
            guard.requested_path.display(),
            e
        ))
    })?;
    let file_type = metadata.file_type();
    if !file_type.is_file() && !file_type.is_symlink() {
        return Err(AppError::Repository(format!(
            "import destination is not a file or symlink: {}",
            guard.requested_path.display()
        )));
    }

    Ok(())
}

#[cfg(test)]
fn validate_import_destination_guard(
    guard: &ImportDestinationGuard,
    returned_dest: &Path,
) -> AppResult<()> {
    validate_import_destination_parent(guard, returned_dest)?;
    validate_import_destination_file(guard)
}

fn validate_import_destination_guard_after_placement(
    guard: &ImportDestinationGuard,
    returned_dest: &Path,
) -> AppResult<()> {
    validate_import_destination_parent(guard, returned_dest)?;
    match validate_import_destination_file(guard) {
        Ok(()) => Ok(()),
        Err(validation_error) => {
            let validation_message = validation_error.to_string();
            match std::fs::remove_file(returned_dest) {
                Ok(()) => Err(validation_error),
                Err(cleanup_error) if cleanup_error.kind() == std::io::ErrorKind::NotFound => {
                    Err(validation_error)
                }
                Err(cleanup_error) => Err(AppError::Repository(format!(
                    "{validation_message}; additionally failed to remove placed import destination {} after validation failure: {cleanup_error}",
                    returned_dest.display()
                ))),
            }
        }
    }
}

fn import_symlink_source(
    source: &Path,
    dest: &Path,
    source_fingerprint: &ImportSourceFingerprint,
    size: u64,
) -> AppResult<()> {
    #[cfg(not(unix))]
    {
        let _ = dest;
        let _ = source_fingerprint;
        let _ = size;
        Err(AppError::Repository(format!(
            "import path is a symlink, but symlink imports are not supported on this platform: {}",
            source.display()
        )))
    }

    #[cfg(unix)]
    {
        let ImportSourceKind::Symlink {
            resolved_target, ..
        } = &source_fingerprint.kind
        else {
            unreachable!("import_symlink_source called for non-symlink source");
        };
        let symlink_target = build_destination_symlink_target(dest, resolved_target);
        symlink(&symlink_target, dest).map_err(|e| {
            AppError::Repository(format!(
                "failed to create symlink import {} -> {}: {}",
                dest.display(),
                symlink_target.display(),
                e
            ))
        })?;
        let dest_meta = std::fs::symlink_metadata(dest).map_err(|e| {
            AppError::Repository(format!(
                "failed to inspect imported symlink {}: {}",
                dest.display(),
                e
            ))
        })?;
        if !dest_meta.file_type().is_symlink() {
            let _ = std::fs::remove_file(dest);
            return Err(AppError::Repository(format!(
                "import destination is not a symlink: {}",
                dest.display()
            )));
        }
        ensure_same_source(source, source_fingerprint)?;
        let dest_target_meta = std::fs::metadata(dest).map_err(|e| {
            let _ = std::fs::remove_file(dest);
            AppError::Repository(format!(
                "imported symlink target is unavailable: {}: {}",
                dest.display(),
                e
            ))
        })?;
        if dest_target_meta.len() != size {
            let _ = std::fs::remove_file(dest);
            return Err(AppError::Repository(format!(
                "symlink import size mismatch: source={} dest={}",
                size,
                dest_target_meta.len()
            )));
        }

        Ok(())
    }
}

#[derive(Debug)]
struct ImportCopyAttemptError {
    stage: &'static str,
    error: std::io::Error,
}

impl ImportCopyAttemptError {
    fn new(stage: &'static str, error: std::io::Error) -> Self {
        Self { stage, error }
    }

    fn is_transient_file_handle_error(&self) -> bool {
        self.error.raw_os_error() == Some(TRANSIENT_BAD_FILE_DESCRIPTOR_ERRNO)
    }

    fn is_cancellation(&self) -> bool {
        self.stage == "copy cancelled"
    }
}

impl fmt::Display for ImportCopyAttemptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.stage, self.error)
    }
}

fn remove_import_temp_file(temp_dest: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(temp_dest) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
fn sleep_before_import_copy_retry(_delay: Duration) {}

#[cfg(not(test))]
fn sleep_before_import_copy_retry(delay: Duration) {
    std::thread::sleep(delay);
}

fn import_copy_retry_delay(attempt: usize) -> Duration {
    match attempt {
        1 => Duration::from_secs(1),
        _ => Duration::from_secs(3),
    }
}

fn report_import_transfer_progress(
    progress: Option<&ImportFileTransferProgressSender>,
    phase: ImportTransferPhase,
    bytes: u64,
    total_bytes: u64,
) {
    if let Some(progress) = progress {
        let _ = progress.send(ImportFileTransferProgress {
            phase,
            bytes,
            total_bytes,
        });
    }
}

struct ImportCopyAttempt<'a> {
    source: &'a Path,
    dest: &'a Path,
    temp_dest: &'a Path,
    source_fingerprint: &'a ImportSourceFingerprint,
    size: u64,
    progress: Option<&'a ImportFileTransferProgressSender>,
    cancellation: Option<&'a scryer_application::ImportCancellation>,
}

fn copy_regular_source_to_destination_once(
    attempt_context: ImportCopyAttempt<'_>,
    options: ImportFileOptions,
    attempt: usize,
) -> Result<StreamedContentHashes, ImportCopyAttemptError> {
    let ImportCopyAttempt {
        source,
        dest,
        temp_dest,
        source_fingerprint,
        size,
        progress,
        cancellation,
    } = attempt_context;

    if cancellation.is_some_and(scryer_application::ImportCancellation::is_cancelled) {
        return Err(ImportCopyAttemptError::new(
            "copy cancelled",
            io_other("import copy was cancelled before it started"),
        ));
    }

    ensure_same_source(source, source_fingerprint)
        .map_err(io_other)
        .map_err(|error| ImportCopyAttemptError::new("source preflight", error))?;
    let mut source_file = std::fs::File::open(source)
        .map_err(|error| ImportCopyAttemptError::new("source open", error))?;
    let source_open_fingerprint = fingerprint_from_metadata(
        &source_file
            .metadata()
            .map_err(|error| ImportCopyAttemptError::new("source metadata", error))?,
    )
    .map_err(io_other)
    .map_err(|error| ImportCopyAttemptError::new("source validation", error))?;
    if source_open_fingerprint != source_fingerprint.file {
        return Err(ImportCopyAttemptError::new(
            "source validation",
            io_other("import source changed before copy"),
        ));
    }

    let mut temp_file = std::fs::File::create(temp_dest)
        .map_err(|error| ImportCopyAttemptError::new("temp create", error))?;
    force_copy_attempt_error(&mut temp_file, &options, attempt)
        .map_err(|error| ImportCopyAttemptError::new("copy", error))?;
    report_import_transfer_progress(progress, ImportTransferPhase::Copying, 0, size);
    let mut copied = 0u64;
    // FR-045: the same one-pass CRC + BLAKE3 a location operation's copy uses.
    // The source is read exactly once, so the hashes cost nothing beyond the
    // arithmetic; a second pass over multi-gigabyte media would not be
    // affordable and is exactly what D2 exists to avoid.
    let mut hasher = StreamedContentHasher::new();
    let mut buffer = vec![0u8; IMPORT_COPY_BUFFER_BYTES];
    loop {
        if cancellation.is_some_and(scryer_application::ImportCancellation::is_cancelled) {
            return Err(ImportCopyAttemptError::new(
                "copy cancelled",
                io_other("import copy was cancelled"),
            ));
        }
        let read = source_file
            .read(&mut buffer)
            .map_err(|error| ImportCopyAttemptError::new("copy", error))?;
        if read == 0 {
            break;
        }
        temp_file
            .write_all(&buffer[..read])
            .map_err(|error| ImportCopyAttemptError::new("copy", error))?;
        hasher.update(&buffer[..read]);
        copied = copied.saturating_add(read as u64);
        report_import_transfer_progress(progress, ImportTransferPhase::Copying, copied, size);
    }
    report_import_transfer_progress(progress, ImportTransferPhase::Finalizing, copied, size);
    temp_file
        .flush()
        .map_err(|error| ImportCopyAttemptError::new("flush", error))?;
    temp_file
        .sync_all()
        .map_err(|error| ImportCopyAttemptError::new("sync", error))?;
    drop(temp_file);

    ensure_same_source(source, source_fingerprint)
        .map_err(io_other)
        .map_err(|error| ImportCopyAttemptError::new("source verification", error))?;

    scryer_application::fs_safety::rename_into_claimed_destination_blocking(temp_dest, dest)
        .map_err(|error| ImportCopyAttemptError::new("final rename", error))?;

    Ok(hasher.finalize())
}

/// Copies one regular file and proves the destination at `depth` (FR-045).
///
/// Returns the streamed hashes and what verification concluded. A destination
/// that cannot be proven is removed and the import fails: that is the FR-044
/// gate expressed at the only place an import copies bytes — an import that
/// errors here never reaches source cleanup, so a corrupt copy can never cost
/// the user the original.
#[allow(clippy::too_many_arguments)]
fn copy_regular_source_to_destination(
    source: &Path,
    dest: &Path,
    source_fingerprint: &ImportSourceFingerprint,
    size: u64,
    options: ImportFileOptions,
    depth: VerificationDepth,
    progress: Option<&ImportFileTransferProgressSender>,
    cancellation: Option<&scryer_application::ImportCancellation>,
) -> AppResult<ImportVerification> {
    let temp_dest = dest.with_extension("tmp_import");
    let mut attempt = 1usize;

    let hashes = loop {
        if let Err(error) = remove_import_temp_file(&temp_dest) {
            return Err(AppError::Repository(format!(
                "import copy failed before attempt {}: {} -> {}: cleanup of temporary destination {} failed: {}",
                attempt,
                source.display(),
                dest.display(),
                temp_dest.display(),
                error
            )));
        }

        match copy_regular_source_to_destination_once(
            ImportCopyAttempt {
                source,
                dest,
                temp_dest: &temp_dest,
                source_fingerprint,
                size,
                progress,
                cancellation,
            },
            options,
            attempt,
        ) {
            Ok(hashes) => break hashes,
            Err(error) => {
                let cancelled = error.is_cancellation();
                let should_retry =
                    error.is_transient_file_handle_error() && attempt < IMPORT_COPY_MAX_ATTEMPTS;
                let _ = remove_import_temp_file(&temp_dest);
                if cancelled {
                    return Err(AppError::canceled(format!(
                        "filesystem copy was cancelled: {} -> {}",
                        source.display(),
                        dest.display()
                    )));
                }
                if should_retry {
                    sleep_before_import_copy_retry(import_copy_retry_delay(attempt));
                    attempt += 1;
                    continue;
                }

                return Err(AppError::Repository(format!(
                    "import copy failed after {} attempt(s): {} -> {}: {}",
                    attempt,
                    source.display(),
                    dest.display(),
                    error
                )));
            }
        }
    };

    if force_copy_verification_failure(&options) {
        let _ = std::fs::remove_file(dest);
        return Err(AppError::Repository(format!(
            "copy verification failed for test: {}",
            dest.display()
        )));
    }

    ensure_same_source(source, source_fingerprint)?;
    let dest_fingerprint = fingerprint_import_source(dest)?.file;

    if dest_fingerprint.len != size {
        let _ = std::fs::remove_file(dest);
        return Err(AppError::Repository(format!(
            "copy size mismatch: source={} dest={}",
            size, dest_fingerprint.len
        )));
    }

    force_destination_corruption(&options, dest);

    // Already on a blocking thread, so the blocking twin is the right one.
    // `Full` degrades to the quick floor by itself when the destination cannot
    // be read back (FR-042), which is why a filesystem that refuses a
    // cache-bypassed re-read still imports rather than failing.
    let assessment = verified_copier_for(&options).verify_blocking(source, dest, &hashes, depth);
    let verification = ImportVerification {
        hashes,
        depth: assessment.depth,
        outcome: assessment.outcome,
        detail: assessment.detail,
    };

    if !verification.permits_source_removal() {
        let detail = verification
            .detail
            .clone()
            .unwrap_or_else(|| "no detail recorded".to_string());
        let _ = std::fs::remove_file(dest);
        return Err(AppError::Repository(format!(
            "copy verification failed at depth {} for {}: {} ({detail})",
            verification.depth.label(),
            dest.display(),
            verification.outcome.as_str(),
        )));
    }

    Ok(verification)
}

fn remove_import_source_after_verified_import_blocking(
    guard: ImportSourceCleanupGuard,
    final_dest_path: PathBuf,
    options: ImportFileOptions,
) -> AppResult<()> {
    if guard.source_path == final_dest_path {
        return Err(AppError::Repository(format!(
            "refusing to remove import source because it is the library file: {}",
            guard.source_path.display()
        )));
    }

    let dest_proof = import_content_proof(&final_dest_path).map_err(|_| {
        AppError::Repository(format!(
            "import source cleanup failed because destination is missing or inaccessible: {}",
            final_dest_path.display()
        ))
    })?;
    if dest_proof != guard.dest_proof {
        return Err(AppError::Repository(format!(
            "import source cleanup failed because destination proof changed: {} expected_size={} actual_size={} expected_blake3={} actual_blake3={}",
            final_dest_path.display(),
            guard.dest_proof.size_bytes,
            dest_proof.size_bytes,
            guard.dest_proof.sample_blake3,
            dest_proof.sample_blake3
        )));
    }

    let source_fingerprint = fingerprint_import_source(&guard.source_path).map_err(|_| {
        AppError::Repository(format!(
            "import source cleanup failed because source is missing or inaccessible: {}",
            guard.source_path.display()
        ))
    })?;
    if source_identity_from_fingerprint(&source_fingerprint) != guard.source_identity {
        return Err(AppError::Repository(format!(
            "import source cleanup failed because source changed: {}",
            guard.source_path.display()
        )));
    }
    let source_proof = import_content_proof(&guard.source_path).map_err(|_| {
        AppError::Repository(format!(
            "import source cleanup failed because source is missing or inaccessible: {}",
            guard.source_path.display()
        ))
    })?;
    if source_proof != guard.source_proof {
        return Err(AppError::Repository(format!(
            "import source cleanup failed because source proof changed: {} expected_size={} actual_size={} expected_blake3={} actual_blake3={}",
            guard.source_path.display(),
            guard.source_proof.size_bytes,
            source_proof.size_bytes,
            guard.source_proof.sample_blake3,
            source_proof.sample_blake3
        )));
    }

    let remove_result = if force_delete_failure(&options) {
        Err(io_other("forced source delete failure for test"))
    } else {
        std::fs::remove_file(&guard.source_path)
    };

    if let Err(error) = remove_result {
        return Err(AppError::Repository(format!(
            "import source cleanup failed after destination verification; failed to remove source {}: {}",
            guard.source_path.display(),
            error
        )));
    }

    Ok(())
}

fn finalize_imported_regular_destination(
    dest: &Path,
    options: &ImportFileOptions,
    permissions: &ImportFilePermissions,
) -> AppResult<()> {
    if let Err(error) = verify_imported_regular_file_uid(dest, options) {
        let _ = std::fs::remove_file(dest);
        return Err(error);
    }
    apply_file_permissions_best_effort(dest, permissions);
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PreparedImportPlacement {
    source: PathBuf,
    dest: PathBuf,
    #[serde(skip)]
    options: ImportFileOptions,
    /// The operator's verification depth for this import (FR-042/045).
    /// Serialized, unlike `options`: the worker process performs the copy, so
    /// the depth has to survive the handoff. Defaults to `Full` for a request
    /// written by an older peer — never to the weaker setting.
    #[serde(default)]
    verification_depth: VerificationDepth,
    source_cleanup_required: bool,
    #[serde(skip)]
    progress: Option<ImportFileTransferProgressSender>,
    #[serde(skip)]
    cancellation: Option<scryer_application::ImportCancellation>,
    permissions: ImportFilePermissions,
    source_fingerprint: ImportSourceFingerprint,
    size: u64,
    destination_guard: ImportDestinationGuard,
    volume_key: String,
}

enum FastPlacementResult {
    Finished(ImportFileResult),
    CopyRequired(PreparedImportPlacement),
}

#[expect(
    clippy::too_many_arguments,
    reason = "import preparation carries source, destination, transfer, permission, and verification context"
)]
fn prepare_import_placement_blocking(
    source: PathBuf,
    dest: PathBuf,
    options: ImportFileOptions,
    verification_depth: VerificationDepth,
    source_cleanup_required: bool,
    expected_source: Option<ImportSourceSnapshot>,
    progress: Option<ImportFileTransferProgressSender>,
    permissions: ImportFilePermissions,
) -> AppResult<PreparedImportPlacement> {
    let (source_fingerprint, size, destination_guard) =
        prepare_import_destination(&source, &dest, &permissions)?;
    ensure_expected_source_snapshot(&source, &source_fingerprint, expected_source.as_ref())?;
    let volume_key = destination_volume_key(&destination_guard);
    Ok(PreparedImportPlacement {
        source,
        dest,
        options,
        verification_depth,
        source_cleanup_required,
        progress,
        cancellation: None,
        permissions,
        source_fingerprint,
        size,
        destination_guard,
        volume_key,
    })
}

#[cfg(unix)]
fn destination_volume_key(guard: &ImportDestinationGuard) -> String {
    format!("unix-dev:{}", guard.approved_parent_fingerprint.dev)
}

#[cfg(windows)]
fn destination_volume_key(guard: &ImportDestinationGuard) -> String {
    use std::path::{Component, Prefix};

    let root = match guard.approved_parent_canonical.components().next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
                format!("{}:", char::from(letter).to_ascii_lowercase())
            }
            Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => format!(
                "//{}/{}",
                server.to_string_lossy().to_ascii_lowercase(),
                share.to_string_lossy().to_ascii_lowercase()
            ),
            other => format!("{other:?}"),
        },
        _ => "unknown-root".to_string(),
    };
    format!(
        "windows-volume:{}:{root}",
        guard.approved_parent_fingerprint.volume_serial_number
    )
}

#[cfg(not(any(unix, windows)))]
fn destination_volume_key(_guard: &ImportDestinationGuard) -> String {
    "unknown-volume".to_string()
}

fn finish_prepared_import(
    prepared: PreparedImportPlacement,
    strategy: ImportStrategy,
) -> AppResult<ImportFileResult> {
    let source_cleanup = cleanup_guard_after_placement(
        prepared.source_cleanup_required,
        &prepared.source,
        &prepared.dest,
        &prepared.source_fingerprint,
        prepared.size,
    )?;
    Ok(ImportFileResult {
        strategy,
        source_path: prepared.source,
        dest_path: prepared.dest,
        size_bytes: prepared.size,
        destination_disposition: ImportDestinationDisposition::Created,
        source_cleanup,
        // A rename, hardlink, or symlink placement copies no bytes (FR-032):
        // there is nothing to verify and nothing to hash.
        verification: None,
    })
}

fn try_fast_import_placement_blocking(
    prepared: PreparedImportPlacement,
) -> AppResult<FastPlacementResult> {
    ensure_same_source(&prepared.source, &prepared.source_fingerprint)?;
    validate_import_destination_parent(&prepared.destination_guard, &prepared.dest)?;
    if let Some(existing) = reconcile_existing_destination(
        &prepared.source,
        &prepared.dest,
        &prepared.source_fingerprint,
        prepared.size,
        &prepared.destination_guard,
        prepared.source_cleanup_required,
    )? {
        return Ok(FastPlacementResult::Finished(existing));
    }

    if let ImportSourceKind::Symlink { .. } = &prepared.source_fingerprint.kind {
        import_symlink_source(
            &prepared.source,
            &prepared.dest,
            &prepared.source_fingerprint,
            prepared.size,
        )?;
        validate_import_destination_guard_after_placement(
            &prepared.destination_guard,
            &prepared.dest,
        )?;
        return finish_prepared_import(prepared, ImportStrategy::Symlink)
            .map(FastPlacementResult::Finished);
    }

    let hardlink_uid_safe =
        source_uid_matches_scryer(&prepared.source_fingerprint, &prepared.options);
    let mut partial_destination_created = false;
    if !force_cross_device_move(&prepared.options) && hardlink_uid_safe {
        match std::fs::hard_link(&prepared.source, &prepared.dest) {
            Ok(()) => {
                partial_destination_created = true;
                if let Err(error) = ensure_same_source(
                    &prepared.source,
                    &prepared.source_fingerprint,
                )
                .and_then(|_| {
                    ensure_same_file_identity(&prepared.dest, &prepared.source_fingerprint.file)
                }) {
                    let _ = std::fs::remove_file(&prepared.dest);
                    return Err(error);
                }
                match std::fs::metadata(&prepared.dest) {
                    Ok(dest_meta) if dest_meta.len() == prepared.size => {
                        validate_import_destination_guard_after_placement(
                            &prepared.destination_guard,
                            &prepared.dest,
                        )?;
                        finalize_imported_regular_destination(
                            &prepared.dest,
                            &prepared.options,
                            &prepared.permissions,
                        )?;
                        return finish_prepared_import(prepared, ImportStrategy::HardLink)
                            .map(FastPlacementResult::Finished);
                    }
                    Ok(dest_meta) => tracing::warn!(
                        source_size = prepared.size,
                        destination_size = dest_meta.len(),
                        "hard link size mismatch; handing import to copy lane"
                    ),
                    Err(error) => tracing::warn!(
                        error = %error,
                        "hard link destination stat failed; handing import to copy lane"
                    ),
                }
            }
            Err(error) if scryer_application::fs_safety::is_cross_device_error(&error) => {
                tracing::debug!(error = %error, "hard link is ineligible; handing import to copy lane");
            }
            Err(error) => {
                tracing::warn!(error = %error, "hard link failed; handing import to copy lane");
            }
        }
    }

    if partial_destination_created {
        std::fs::remove_file(&prepared.dest).map_err(|error| {
            AppError::Repository(format!(
                "failed to remove partial hardlink destination {} before copy: {error}",
                prepared.dest.display()
            ))
        })?;
    }
    ensure_import_destination_absent(&prepared.dest)?;
    Ok(FastPlacementResult::CopyRequired(prepared))
}

fn reconcile_existing_destination(
    source: &Path,
    dest: &Path,
    source_fingerprint: &ImportSourceFingerprint,
    size: u64,
    destination_guard: &ImportDestinationGuard,
    source_cleanup_required: bool,
) -> AppResult<Option<ImportFileResult>> {
    let metadata = match std::fs::symlink_metadata(dest) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(AppError::Repository(format!(
                "failed to inspect existing import destination {}: {error}",
                dest.display()
            )));
        }
        Ok(metadata) => metadata,
    };
    if source == dest {
        return Err(AppError::ManualReconciliationRequired(format!(
            "import source and destination are the same path: {}",
            dest.display()
        )));
    }
    if !metadata.file_type().is_file() && !metadata.file_type().is_symlink() {
        return Err(AppError::ManualReconciliationRequired(format!(
            "import destination conflict is not a regular file: {}",
            dest.display()
        )));
    }

    ensure_same_source(source, source_fingerprint)?;
    validate_import_destination_parent(destination_guard, dest)?;
    validate_import_destination_file(destination_guard)?;
    let source_proof = import_content_proof(source).map_err(|error| {
        AppError::Repository(format!(
            "failed to prove import source content {}: {error}",
            source.display()
        ))
    })?;
    let destination_proof = import_content_proof(dest).map_err(|error| {
        AppError::Repository(format!(
            "failed to prove import destination content {}: {error}",
            dest.display()
        ))
    })?;
    if source_proof != destination_proof || source_proof.size_bytes != size {
        return Err(AppError::ManualReconciliationRequired(format!(
            "import destination appeared with different content: {}",
            dest.display()
        )));
    }

    let source_cleanup = if source_cleanup_required {
        Some(cleanup_guard(source, dest, source_fingerprint, size)?)
    } else {
        None
    };
    Ok(Some(ImportFileResult {
        strategy: if source_cleanup_required {
            ImportStrategy::Move
        } else {
            ImportStrategy::Copy
        },
        source_path: source.to_path_buf(),
        dest_path: dest.to_path_buf(),
        size_bytes: size,
        destination_disposition: ImportDestinationDisposition::AlreadyPresent,
        source_cleanup,
        // This call copied nothing: the destination was already there and was
        // proved equal to the source by the sampled proof above.
        verification: None,
    }))
}

fn copy_prepared_import_blocking(prepared: PreparedImportPlacement) -> AppResult<ImportFileResult> {
    ensure_same_source(&prepared.source, &prepared.source_fingerprint)?;
    validate_import_destination_parent(&prepared.destination_guard, &prepared.dest)?;
    ensure_import_destination_absent(&prepared.dest)?;
    let verification = copy_regular_source_to_destination(
        &prepared.source,
        &prepared.dest,
        &prepared.source_fingerprint,
        prepared.size,
        prepared.options,
        prepared.verification_depth,
        prepared.progress.as_ref(),
        prepared.cancellation.as_ref(),
    )?;
    validate_import_destination_guard_after_placement(&prepared.destination_guard, &prepared.dest)?;
    finalize_imported_regular_destination(
        &prepared.dest,
        &prepared.options,
        &prepared.permissions,
    )?;
    // The source-cleanup guard is only built once verification passed, so
    // FR-044 holds structurally: an unproven copy has no guard to remove
    // anything with.
    let mut result = finish_prepared_import(prepared, ImportStrategy::Copy)?;
    result.verification = Some(verification);
    Ok(result)
}

#[expect(
    clippy::too_many_arguments,
    reason = "single-process placement carries the same context the worker protocol splits across messages"
)]
fn import_hardlink_or_copy_blocking(
    source: PathBuf,
    dest: PathBuf,
    options: ImportFileOptions,
    verification_depth: VerificationDepth,
    source_cleanup_required: bool,
    expected_source: Option<ImportSourceSnapshot>,
    progress: Option<ImportFileTransferProgressSender>,
    permissions: ImportFilePermissions,
) -> AppResult<ImportFileResult> {
    let (source_fingerprint, size, destination_guard) =
        prepare_import_destination(&source, &dest, &permissions)?;
    ensure_expected_source_snapshot(&source, &source_fingerprint, expected_source.as_ref())?;
    if let Some(existing) = reconcile_existing_destination(
        &source,
        &dest,
        &source_fingerprint,
        size,
        &destination_guard,
        source_cleanup_required,
    )? {
        return Ok(existing);
    }

    if let ImportSourceKind::Symlink { .. } = &source_fingerprint.kind {
        import_symlink_source(&source, &dest, &source_fingerprint, size)?;
        validate_import_destination_guard_after_placement(&destination_guard, &dest)?;
        let source_cleanup = cleanup_guard_after_placement(
            source_cleanup_required,
            &source,
            &dest,
            &source_fingerprint,
            size,
        )?;
        return Ok(ImportFileResult {
            strategy: ImportStrategy::Symlink,
            source_path: source,
            dest_path: dest,
            size_bytes: size,
            destination_disposition: ImportDestinationDisposition::Created,
            source_cleanup,
            verification: None,
        });
    }

    let hardlink_uid_safe = source_uid_matches_scryer(&source_fingerprint, &options);
    if !hardlink_uid_safe {
        tracing::info!(
            source = %source.display(),
            dest = %dest.display(),
            "skipping hardlink import because source uid differs from Scryer process uid"
        );
    }

    if !force_cross_device_move(&options) && hardlink_uid_safe {
        match std::fs::hard_link(&source, &dest) {
            Ok(()) => {
                if let Err(error) = ensure_same_source(&source, &source_fingerprint)
                    .and_then(|_| ensure_same_file_identity(&dest, &source_fingerprint.file))
                {
                    let _ = std::fs::remove_file(&dest);
                    return Err(error);
                }
                match std::fs::metadata(&dest) {
                    Ok(dest_meta) if dest_meta.len() == size => {
                        validate_import_destination_guard_after_placement(
                            &destination_guard,
                            &dest,
                        )?;
                        if file_permission_metadata_changes(&permissions) {
                            tracing::info!(
                                source = %source.display(),
                                dest = %dest.display(),
                                "applying import permissions to hardlink; metadata changes affect the shared download-side inode"
                            );
                        }
                        finalize_imported_regular_destination(&dest, &options, &permissions)?;
                        let source_cleanup = cleanup_guard_after_placement(
                            source_cleanup_required,
                            &source,
                            &dest,
                            &source_fingerprint,
                            size,
                        )?;
                        return Ok(ImportFileResult {
                            strategy: ImportStrategy::HardLink,
                            source_path: source,
                            dest_path: dest,
                            size_bytes: size,
                            destination_disposition: ImportDestinationDisposition::Created,
                            source_cleanup,
                            verification: None,
                        });
                    }
                    Ok(dest_meta) => {
                        let _ = std::fs::remove_file(&dest);
                        tracing::warn!(
                            "hard link size mismatch: source={} dest={}, falling back to copy",
                            size,
                            dest_meta.len()
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            "hard link created but dest stat failed: {}, falling back to copy",
                            e
                        );
                    }
                }
            }
            Err(e) if scryer_application::fs_safety::is_cross_device_error(&e) => {
                tracing::info!(
                    "hard link failed (cross-device), falling back to copy: {} -> {}",
                    source.display(),
                    dest.display()
                );
            }
            Err(e) => {
                tracing::warn!(
                    "hard link failed: {}, falling back to copy: {} -> {}",
                    e,
                    source.display(),
                    dest.display()
                );
            }
        }
    }

    let verification = copy_regular_source_to_destination(
        &source,
        &dest,
        &source_fingerprint,
        size,
        options,
        verification_depth,
        progress.as_ref(),
        None,
    )?;
    validate_import_destination_guard_after_placement(&destination_guard, &dest)?;
    finalize_imported_regular_destination(&dest, &options, &permissions)?;

    let source_cleanup = cleanup_guard_after_placement(
        source_cleanup_required,
        &source,
        &dest,
        &source_fingerprint,
        size,
    )?;
    Ok(ImportFileResult {
        strategy: ImportStrategy::Copy,
        source_path: source,
        dest_path: dest,
        size_bytes: size,
        destination_disposition: ImportDestinationDisposition::Created,
        source_cleanup,
        verification: Some(verification),
    })
}

/// Single-process placement at the default depth, for callers that name none.
fn import_file_blocking(
    source: PathBuf,
    dest: PathBuf,
    mode: ImportMode,
    options: ImportFileOptions,
    expected_source: Option<ImportSourceSnapshot>,
    progress: Option<ImportFileTransferProgressSender>,
    permissions: ImportFilePermissions,
) -> AppResult<ImportFileResult> {
    import_file_blocking_at_depth(
        source,
        dest,
        mode,
        options,
        VerificationDepth::default(),
        expected_source,
        progress,
        permissions,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "single-process placement carries the same context the worker protocol splits across messages"
)]
fn import_file_blocking_at_depth(
    source: PathBuf,
    dest: PathBuf,
    mode: ImportMode,
    options: ImportFileOptions,
    verification_depth: VerificationDepth,
    expected_source: Option<ImportSourceSnapshot>,
    progress: Option<ImportFileTransferProgressSender>,
    permissions: ImportFilePermissions,
) -> AppResult<ImportFileResult> {
    match mode {
        ImportMode::HardlinkOrCopy => import_hardlink_or_copy_blocking(
            source,
            dest,
            options,
            verification_depth,
            false,
            expected_source,
            progress,
            permissions,
        ),
        ImportMode::Move => import_hardlink_or_copy_blocking(
            source,
            dest,
            options,
            verification_depth,
            true,
            expected_source,
            progress,
            permissions,
        ),
    }
}

const IMPORT_FILE_WORKER_PROTOCOL_VERSION: u16 = 1;
static IMPORT_FILE_WORKER_NONCE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
enum ImportFileWorkerRequest {
    Snapshot {
        version: u16,
        nonce: u64,
        source: PathBuf,
    },
    Prepare {
        version: u16,
        nonce: u64,
        source: PathBuf,
        dest: PathBuf,
        mode: ImportMode,
        expected_source: Option<ImportSourceSnapshot>,
        permissions: ImportFilePermissions,
        /// FR-042/045. Defaulted rather than required so a request from an
        /// older peer verifies at full depth, never at the weaker one.
        #[serde(default)]
        verification_depth: VerificationDepth,
    },
    FastPlacement {
        version: u16,
        nonce: u64,
        prepared: PreparedImportPlacement,
    },
    Copy {
        version: u16,
        nonce: u64,
        prepared: PreparedImportPlacement,
    },
    Cleanup {
        version: u16,
        nonce: u64,
        guard: ImportSourceCleanupGuard,
        final_dest_path: PathBuf,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
enum ImportFileWorkerEvent {
    Stage {
        nonce: u64,
        stage: String,
    },
    Prepared {
        nonce: u64,
        prepared: PreparedImportPlacement,
    },
    CopyRequired {
        nonce: u64,
        volume_key: String,
    },
    Progress {
        nonce: u64,
        phase: ImportTransferPhase,
        bytes: u64,
        total_bytes: u64,
    },
    SnapshotFinished {
        nonce: u64,
        snapshot: ImportSourceSnapshot,
    },
    ImportFinished {
        nonce: u64,
        result: ImportFileResult,
    },
    CleanupFinished {
        nonce: u64,
    },
    Error {
        nonce: u64,
        message: String,
    },
}

fn write_import_worker_event(
    output: &Arc<std::sync::Mutex<std::io::Stdout>>,
    event: &ImportFileWorkerEvent,
) -> io::Result<()> {
    let mut output = output
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    serde_json::to_writer(&mut *output, event).map_err(io::Error::other)?;
    output.write_all(b"\n")?;
    output.flush()
}

fn read_import_worker_request(
    input: &mut impl std::io::BufRead,
) -> Result<ImportFileWorkerRequest, String> {
    let mut line = String::new();
    if input
        .read_line(&mut line)
        .map_err(|error| error.to_string())?
        == 0
    {
        return Err("import worker control pipe closed".to_string());
    }
    serde_json::from_str(&line).map_err(|error| format!("invalid import worker request: {error}"))
}

fn import_worker_request_nonce(request: &ImportFileWorkerRequest) -> (u16, u64) {
    match request {
        ImportFileWorkerRequest::Snapshot { version, nonce, .. }
        | ImportFileWorkerRequest::Prepare { version, nonce, .. }
        | ImportFileWorkerRequest::FastPlacement { version, nonce, .. }
        | ImportFileWorkerRequest::Copy { version, nonce, .. }
        | ImportFileWorkerRequest::Cleanup { version, nonce, .. } => (*version, *nonce),
    }
}

fn start_import_worker_stdin_death_watch() {
    std::thread::spawn(|| {
        let stdin = std::io::stdin();
        let mut control = std::io::BufReader::new(stdin.lock());
        let mut line = String::new();
        if control.read_line(&mut line).is_err() || line.is_empty() {
            std::process::exit(3);
        }
    });
}

/// Run the hidden filesystem worker protocol. The host invokes this before
/// normal application startup so a blocked kernel filesystem call remains
/// independently killable.
pub fn run_import_file_worker() -> i32 {
    let stdin = std::io::stdin();
    let mut input = std::io::BufReader::new(stdin.lock());
    let output = Arc::new(std::sync::Mutex::new(std::io::stdout()));
    let request = match read_import_worker_request(&mut input) {
        Ok(request) => request,
        Err(error) => {
            let _ = write_import_worker_event(
                &output,
                &ImportFileWorkerEvent::Error {
                    nonce: 0,
                    message: error,
                },
            );
            return 2;
        }
    };
    let (version, nonce) = import_worker_request_nonce(&request);
    if version != IMPORT_FILE_WORKER_PROTOCOL_VERSION {
        let _ = write_import_worker_event(
            &output,
            &ImportFileWorkerEvent::Error {
                nonce,
                message: format!("unsupported import worker protocol version {version}"),
            },
        );
        return 2;
    }
    drop(input);
    start_import_worker_stdin_death_watch();

    let result: Result<(), String> = match request {
        ImportFileWorkerRequest::Snapshot { source, .. } => {
            let _ = write_import_worker_event(
                &output,
                &ImportFileWorkerEvent::Stage {
                    nonce,
                    stage: "snapshot".to_string(),
                },
            );
            snapshot_import_source_blocking(source)
                .map_err(|error| error.to_string())
                .and_then(|snapshot| {
                    write_import_worker_event(
                        &output,
                        &ImportFileWorkerEvent::SnapshotFinished { nonce, snapshot },
                    )
                    .map_err(|error| error.to_string())
                })
        }
        ImportFileWorkerRequest::Cleanup {
            guard,
            final_dest_path,
            ..
        } => {
            let _ = write_import_worker_event(
                &output,
                &ImportFileWorkerEvent::Stage {
                    nonce,
                    stage: "cleanup".to_string(),
                },
            );
            remove_import_source_after_verified_import_blocking(
                guard,
                final_dest_path,
                ImportFileOptions::default(),
            )
            .map_err(|error| error.to_string())
            .and_then(|()| {
                write_import_worker_event(
                    &output,
                    &ImportFileWorkerEvent::CleanupFinished { nonce },
                )
                .map_err(|error| error.to_string())
            })
        }
        ImportFileWorkerRequest::Prepare {
            source,
            dest,
            mode,
            expected_source,
            permissions,
            verification_depth,
            ..
        } => {
            let _ = write_import_worker_event(
                &output,
                &ImportFileWorkerEvent::Stage {
                    nonce,
                    stage: "prepare".to_string(),
                },
            );
            let source_cleanup_required = mode == ImportMode::Move;
            prepare_import_placement_blocking(
                source,
                dest,
                ImportFileOptions::default(),
                verification_depth,
                source_cleanup_required,
                expected_source,
                None,
                permissions,
            )
            .map_err(|error| error.to_string())
            .and_then(|prepared| {
                write_import_worker_event(
                    &output,
                    &ImportFileWorkerEvent::Prepared { nonce, prepared },
                )
                .map_err(|error| error.to_string())
            })
        }
        ImportFileWorkerRequest::FastPlacement { mut prepared, .. } => {
            let _ = write_import_worker_event(
                &output,
                &ImportFileWorkerEvent::Stage {
                    nonce,
                    stage: "fast_placement".to_string(),
                },
            );
            prepared.options = ImportFileOptions::default();
            prepared.progress = None;
            try_fast_import_placement_blocking(prepared)
                .map_err(|error| error.to_string())
                .and_then(|result| match result {
                    FastPlacementResult::Finished(result) => write_import_worker_event(
                        &output,
                        &ImportFileWorkerEvent::ImportFinished { nonce, result },
                    )
                    .map_err(|error| error.to_string()),
                    FastPlacementResult::CopyRequired(prepared) => write_import_worker_event(
                        &output,
                        &ImportFileWorkerEvent::CopyRequired {
                            nonce,
                            volume_key: prepared.volume_key,
                        },
                    )
                    .map_err(|error| error.to_string()),
                })
        }
        ImportFileWorkerRequest::Copy { mut prepared, .. } => {
            let _ = write_import_worker_event(
                &output,
                &ImportFileWorkerEvent::Stage {
                    nonce,
                    stage: "copy".to_string(),
                },
            );
            let (progress_tx, mut progress_rx) =
                tokio::sync::mpsc::unbounded_channel::<ImportFileTransferProgress>();
            let progress_output = output.clone();
            let progress_thread = std::thread::spawn(move || {
                while let Some(progress) = progress_rx.blocking_recv() {
                    let _ = write_import_worker_event(
                        &progress_output,
                        &ImportFileWorkerEvent::Progress {
                            nonce,
                            phase: progress.phase,
                            bytes: progress.bytes,
                            total_bytes: progress.total_bytes,
                        },
                    );
                }
            });
            prepared.options = ImportFileOptions::default();
            prepared.progress = Some(progress_tx);
            let outcome = copy_prepared_import_blocking(prepared)
                .map_err(|error| error.to_string())
                .and_then(|result| {
                    write_import_worker_event(
                        &output,
                        &ImportFileWorkerEvent::ImportFinished { nonce, result },
                    )
                    .map_err(|error| error.to_string())
                });
            let _ = progress_thread.join();
            outcome
        }
    };

    if let Err(message) = result {
        let _ =
            write_import_worker_event(&output, &ImportFileWorkerEvent::Error { nonce, message });
        return 1;
    }
    0
}

struct ImportFileWorkerSession {
    nonce: u64,
    child: Option<tokio::process::Child>,
    _stdin: tokio::process::ChildStdin,
    lines: tokio::io::Lines<tokio::io::BufReader<tokio::process::ChildStdout>>,
    last_stage: Option<String>,
    last_progress_phase: Option<String>,
    last_progress_bytes: u64,
}

fn import_file_stall_timeout() -> Duration {
    static TIMEOUT: OnceLock<Duration> = OnceLock::new();
    *TIMEOUT.get_or_init(|| {
        let raw = std::env::var(IMPORT_FILE_STALL_TIMEOUT_ENV).ok();
        let seconds = parse_import_file_stall_timeout(raw.as_deref());
        if raw.is_some()
            && raw
                .as_deref()
                .map(str::trim)
                .and_then(|value| value.parse::<u64>().ok())
                != Some(seconds)
        {
            tracing::warn!(
                key = IMPORT_FILE_STALL_TIMEOUT_ENV,
                value = raw.as_deref().unwrap_or_default(),
                seconds,
                "invalid or out-of-range import file stall timeout was adjusted"
            );
        }
        Duration::from_secs(seconds)
    })
}

fn parse_import_file_stall_timeout(raw: Option<&str>) -> u64 {
    match raw
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<u64>().ok())
    {
        Some(seconds) if seconds < MIN_IMPORT_FILE_STALL_TIMEOUT_SECONDS => {
            DEFAULT_IMPORT_FILE_STALL_TIMEOUT_SECONDS
        }
        Some(seconds) if seconds > MAX_IMPORT_FILE_STALL_TIMEOUT_SECONDS => {
            MAX_IMPORT_FILE_STALL_TIMEOUT_SECONDS
        }
        Some(seconds) => seconds,
        None => DEFAULT_IMPORT_FILE_STALL_TIMEOUT_SECONDS,
    }
}

async fn spawn_import_file_worker(
    executable: &Path,
    request: &ImportFileWorkerRequest,
) -> AppResult<ImportFileWorkerSession> {
    use std::process::Stdio;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

    let nonce = import_worker_request_nonce(request).1;
    let mut child = tokio::process::Command::new(executable)
        .arg("__import-file-worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| {
            AppError::Repository(format!("failed to start import file worker: {error}"))
        })?;
    let mut stdin = child.stdin.take().ok_or_else(|| {
        AppError::Repository("import file worker stdin was unavailable".to_string())
    })?;
    let stdout = child.stdout.take().ok_or_else(|| {
        AppError::Repository("import file worker stdout was unavailable".to_string())
    })?;
    let mut payload = serde_json::to_vec(request).map_err(|error| {
        AppError::Repository(format!("failed to encode import worker request: {error}"))
    })?;
    payload.push(b'\n');
    stdin.write_all(&payload).await.map_err(|error| {
        AppError::Repository(format!("failed to send import worker request: {error}"))
    })?;
    stdin.flush().await.map_err(|error| {
        AppError::Repository(format!("failed to flush import worker request: {error}"))
    })?;
    Ok(ImportFileWorkerSession {
        nonce,
        child: Some(child),
        _stdin: stdin,
        lines: tokio::io::BufReader::new(stdout).lines(),
        last_stage: None,
        last_progress_phase: None,
        last_progress_bytes: 0,
    })
}

async fn hold_unconfirmed_import_worker(
    mut child: tokio::process::Child,
    reason: String,
) -> AppError {
    loop {
        tracing::error!(reason, pid = ?child.id(), "import file worker termination is unconfirmed; retaining import ownership");
        match child.try_wait() {
            Ok(Some(status)) => {
                return AppError::ManualReconciliationRequired(format!(
                    "filesystem worker termination was eventually confirmed ({status}); inspect source and destination before retrying"
                ));
            }
            Ok(None) => {}
            Err(error) => tracing::error!(
                reason,
                error = %error,
                pid = ?child.id(),
                "failed to check import file worker termination; retaining import ownership"
            ),
        }
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}

async fn cancel_stalled_import_worker(session: &mut ImportFileWorkerSession) -> AppError {
    let mut child = session
        .child
        .take()
        .expect("active import worker session owns its child process");
    let pid = child.id();
    if let Err(error) = child.start_kill() {
        return hold_unconfirmed_import_worker(
            child,
            format!("failed to signal stalled worker {pid:?}: {error}"),
        )
        .await;
    }
    match tokio::time::timeout(Duration::from_secs(30), child.wait()).await {
        Ok(Ok(status)) => AppError::ManualReconciliationRequired(format!(
            "filesystem worker {pid:?} made no progress for {} seconds and was terminated ({status}); inspect source and destination before retrying",
            import_file_stall_timeout().as_secs()
        )),
        other => {
            let reason = format!("stalled worker {pid:?} kill could not be confirmed: {other:?}");
            hold_unconfirmed_import_worker(child, reason).await
        }
    }
}

async fn cancel_import_worker(session: &mut ImportFileWorkerSession) -> AppError {
    let mut child = session
        .child
        .take()
        .expect("active import worker session owns its child process");
    let pid = child.id();
    if let Err(error) = child.start_kill() {
        return hold_unconfirmed_import_worker(
            child,
            format!("failed to signal cancelled worker {pid:?}: {error}"),
        )
        .await;
    }
    match tokio::time::timeout(Duration::from_secs(30), child.wait()).await {
        Ok(Ok(status)) => AppError::canceled(format!(
            "filesystem worker {pid:?} was cancelled ({status})"
        )),
        other => {
            hold_unconfirmed_import_worker(
                child,
                format!("cancelled worker {pid:?} stop could not be confirmed: {other:?}"),
            )
            .await
        }
    }
}

async fn next_import_worker_event_or_cancel(
    session: &mut ImportFileWorkerSession,
    cancellation: Option<&scryer_application::ImportCancellation>,
) -> AppResult<ImportFileWorkerEvent> {
    let Some(cancellation) = cancellation else {
        return next_import_worker_event(session).await;
    };
    tokio::select! {
        biased;
        event = next_import_worker_event(session) => event,
        _ = cancellation.cancelled() => Err(cancel_import_worker(session).await),
    }
}

async fn wait_for_import_operation_or_cancel<T>(
    cancellation: Option<&scryer_application::ImportCancellation>,
    operation: impl std::future::Future<Output = T>,
) -> AppResult<T> {
    let Some(cancellation) = cancellation else {
        return Ok(operation.await);
    };
    tokio::select! {
        _ = cancellation.cancelled() => Err(AppError::canceled("import was cancelled before it started")),
        result = operation => Ok(result),
    }
}

async fn wait_for_import_worker_exit(
    session: &mut ImportFileWorkerSession,
    context: &str,
) -> AppResult<()> {
    let mut child = session
        .child
        .take()
        .expect("active import worker session owns its child process");
    match tokio::time::timeout(import_file_stall_timeout(), child.wait()).await {
        Ok(Ok(status)) if status.success() => Ok(()),
        Ok(Ok(status)) => Err(AppError::ManualReconciliationRequired(format!(
            "filesystem worker exited unexpectedly {context} ({status}); inspect source and destination"
        ))),
        Ok(Err(error)) => Err(AppError::ManualReconciliationRequired(format!(
            "filesystem worker exit could not be confirmed {context}: {error}; inspect source and destination"
        ))),
        Err(_) => {
            session.child = Some(child);
            Err(cancel_stalled_import_worker(session).await)
        }
    }
}

async fn next_import_worker_event(
    session: &mut ImportFileWorkerSession,
) -> AppResult<ImportFileWorkerEvent> {
    let timeout = import_file_stall_timeout();
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let line = match tokio::time::timeout_at(deadline, session.lines.next_line()).await {
            Ok(Ok(Some(line))) => line,
            Ok(Ok(None)) => {
                return Err(AppError::ManualReconciliationRequired(
                    "filesystem worker exited before reporting a verified result; inspect source and destination"
                        .to_string(),
                ));
            }
            Ok(Err(error)) => {
                return Err(AppError::ManualReconciliationRequired(format!(
                    "filesystem worker output failed: {error}; inspect source and destination"
                )));
            }
            Err(_) => return Err(cancel_stalled_import_worker(session).await),
        };
        let event: ImportFileWorkerEvent = serde_json::from_str(&line).map_err(|error| {
            AppError::ManualReconciliationRequired(format!(
                "filesystem worker returned invalid output: {error}; inspect source and destination"
            ))
        })?;
        let nonce = match &event {
            ImportFileWorkerEvent::Stage { nonce, .. }
            | ImportFileWorkerEvent::Prepared { nonce, .. }
            | ImportFileWorkerEvent::CopyRequired { nonce, .. }
            | ImportFileWorkerEvent::Progress { nonce, .. }
            | ImportFileWorkerEvent::SnapshotFinished { nonce, .. }
            | ImportFileWorkerEvent::ImportFinished { nonce, .. }
            | ImportFileWorkerEvent::CleanupFinished { nonce }
            | ImportFileWorkerEvent::Error { nonce, .. } => *nonce,
        };
        if nonce != session.nonce {
            return Err(AppError::ManualReconciliationRequired(
                "filesystem worker response identity mismatch; inspect source and destination"
                    .to_string(),
            ));
        }
        match &event {
            ImportFileWorkerEvent::Stage { stage, .. } => {
                if session.last_stage.as_deref() == Some(stage) {
                    continue;
                }
                session.last_stage = Some(stage.clone());
            }
            ImportFileWorkerEvent::Progress { phase, bytes, .. } => {
                let phase = format!("{phase:?}");
                let phase_advanced = session.last_progress_phase.as_deref() != Some(&phase);
                if !phase_advanced && *bytes <= session.last_progress_bytes {
                    continue;
                }
                session.last_progress_phase = Some(phase);
                session.last_progress_bytes = *bytes;
            }
            _ => {}
        }
        return Ok(event);
    }
}

#[async_trait]
impl FileImporter for FsFileImporter {
    /// A location operation writes its own destination, then asks for the same
    /// modes an import would have applied (FR-031). Best-effort for the same
    /// reason imports are: a filesystem that refuses a chmod or a chown is not
    /// a reason to fail a move whose bytes are already verified.
    async fn apply_placed_file_permissions(
        &self,
        path: &Path,
        permissions: &ImportFilePermissions,
    ) -> AppResult<()> {
        let path = path.to_path_buf();
        let permissions = permissions.clone();
        tokio::task::spawn_blocking(move || {
            apply_file_permissions_best_effort(&path, &permissions);
        })
        .await
        .map_err(|error| {
            AppError::Repository(format!("permission task panicked: {error}"))
        })
    }

    async fn apply_placed_directory_permissions(
        &self,
        path: &Path,
        permissions: &ImportFilePermissions,
    ) -> AppResult<()> {
        let path = path.to_path_buf();
        let permissions = permissions.clone();
        tokio::task::spawn_blocking(move || {
            apply_directory_permissions_best_effort(&path, &permissions);
        })
        .await
        .map_err(|error| {
            AppError::Repository(format!("permission task panicked: {error}"))
        })
    }

    async fn snapshot_import_source(&self, source: &Path) -> AppResult<ImportSourceSnapshot> {
        if let Some(executable) = &self.worker_executable {
            let nonce = IMPORT_FILE_WORKER_NONCE.fetch_add(1, Ordering::Relaxed);
            let request = ImportFileWorkerRequest::Snapshot {
                version: IMPORT_FILE_WORKER_PROTOCOL_VERSION,
                nonce,
                source: source.to_path_buf(),
            };
            let mut session = spawn_import_file_worker(executable, &request).await?;
            loop {
                match next_import_worker_event(&mut session).await? {
                    ImportFileWorkerEvent::Stage { .. } => {}
                    ImportFileWorkerEvent::SnapshotFinished { snapshot, .. } => {
                        return Ok(snapshot);
                    }
                    ImportFileWorkerEvent::Error { message, .. } => {
                        return Err(AppError::Repository(message));
                    }
                    _ => {
                        return Err(AppError::ManualReconciliationRequired(
                            "filesystem worker returned an unexpected snapshot response"
                                .to_string(),
                        ));
                    }
                }
            }
        }
        let source = source.to_path_buf();

        tokio::task::spawn_blocking(move || snapshot_import_source_blocking(source))
            .await
            .map_err(|e| AppError::Repository(format!("import snapshot task panicked: {}", e)))?
    }

    async fn import_file(
        &self,
        source: &Path,
        dest: &Path,
        mode: ImportMode,
        expected_source: Option<&ImportSourceSnapshot>,
    ) -> AppResult<ImportFileResult> {
        self.import_file_with_progress(source, dest, mode, expected_source, None)
            .await
    }

    async fn import_file_with_progress(
        &self,
        source: &Path,
        dest: &Path,
        mode: ImportMode,
        expected_source: Option<&ImportSourceSnapshot>,
        progress: Option<ImportFileTransferProgressSender>,
    ) -> AppResult<ImportFileResult> {
        self.import_file_with_progress_and_permissions(
            source,
            dest,
            mode,
            expected_source,
            progress,
            &ImportFilePermissions::default(),
        )
        .await
    }

    async fn import_file_with_progress_and_permissions(
        &self,
        source: &Path,
        dest: &Path,
        mode: ImportMode,
        expected_source: Option<&ImportSourceSnapshot>,
        progress: Option<ImportFileTransferProgressSender>,
        permissions: &ImportFilePermissions,
    ) -> AppResult<ImportFileResult> {
        let source = source.to_path_buf();
        let dest = dest.to_path_buf();
        let expected_source = expected_source.cloned();
        let permissions = permissions.clone();

        tokio::task::spawn_blocking(move || {
            import_file_blocking(
                source,
                dest,
                mode,
                ImportFileOptions::default(),
                expected_source,
                progress,
                permissions,
            )
        })
        .await
        .map_err(|e| AppError::Repository(format!("import task panicked: {}", e)))?
    }

    async fn import_file_with_execution_context(
        &self,
        source: &Path,
        dest: &Path,
        mode: ImportMode,
        expected_source: Option<&ImportSourceSnapshot>,
        progress: Option<ImportFileTransferProgressSender>,
        permissions: &ImportFilePermissions,
        context: &ImportFileExecutionContext,
    ) -> AppResult<ImportFileResult> {
        // No depth was named, so the copy path proves at the default rather
        // than at the weaker setting (FR-042: never a silent downgrade).
        self.place_import_file(
            source,
            dest,
            mode,
            expected_source,
            progress,
            permissions,
            context,
            VerificationDepth::default(),
        )
        .await
    }

    async fn import_file_verified_with_execution_context(
        &self,
        source: &Path,
        dest: &Path,
        mode: ImportMode,
        expected_source: Option<&ImportSourceSnapshot>,
        progress: Option<ImportFileTransferProgressSender>,
        permissions: &ImportFilePermissions,
        context: &ImportFileExecutionContext,
        depth: VerificationDepth,
    ) -> AppResult<ImportFileResult> {
        self.place_import_file(
            source,
            dest,
            mode,
            expected_source,
            progress,
            permissions,
            context,
            depth,
        )
        .await
    }

    async fn remove_import_source_after_verified_import(
        &self,
        guard: ImportSourceCleanupGuard,
        final_dest_path: &Path,
    ) -> AppResult<()> {
        let final_dest_path = final_dest_path.to_path_buf();

        tokio::task::spawn_blocking(move || {
            remove_import_source_after_verified_import_blocking(
                guard,
                final_dest_path,
                ImportFileOptions::default(),
            )
        })
        .await
        .map_err(|e| AppError::Repository(format!("import cleanup task panicked: {}", e)))?
    }

    async fn remove_import_source_after_verified_import_with_context(
        &self,
        guard: ImportSourceCleanupGuard,
        final_dest_path: &Path,
        context: &ImportFileExecutionContext,
    ) -> AppResult<()> {
        let client_key = context.client_lane_key().to_string();
        let _fast_permit = self
            .placement
            .fast
            .acquire(&client_key, "move-cleanup")
            .await;
        if let Some(executable) = &self.worker_executable {
            let nonce = IMPORT_FILE_WORKER_NONCE.fetch_add(1, Ordering::Relaxed);
            let request = ImportFileWorkerRequest::Cleanup {
                version: IMPORT_FILE_WORKER_PROTOCOL_VERSION,
                nonce,
                guard,
                final_dest_path: final_dest_path.to_path_buf(),
            };
            let mut session = spawn_import_file_worker(executable, &request).await?;
            loop {
                match next_import_worker_event(&mut session).await? {
                    ImportFileWorkerEvent::Stage { .. } => {}
                    ImportFileWorkerEvent::CleanupFinished { .. } => return Ok(()),
                    ImportFileWorkerEvent::Error { message, .. } => {
                        return Err(AppError::Repository(message));
                    }
                    _ => {
                        return Err(AppError::ManualReconciliationRequired(
                            "filesystem worker returned an unexpected cleanup response; inspect source and destination"
                                .to_string(),
                        ));
                    }
                }
            }
        }
        let final_dest_path = final_dest_path.to_path_buf();
        tokio::task::spawn_blocking(move || {
            remove_import_source_after_verified_import_blocking(
                guard,
                final_dest_path,
                ImportFileOptions::default(),
            )
        })
        .await
        .map_err(|error| AppError::Repository(format!("import cleanup task panicked: {error}")))?
    }
}

impl FsFileImporter {
    /// The one placement implementation. Both trait entry points land here; the
    /// only thing that varies is the depth the copy path proves at (FR-045).
    #[expect(
        clippy::too_many_arguments,
        reason = "file placement keeps transfer, permission, source-snapshot, lane, and verification context explicit"
    )]
    async fn place_import_file(
        &self,
        source: &Path,
        dest: &Path,
        mode: ImportMode,
        expected_source: Option<&ImportSourceSnapshot>,
        progress: Option<ImportFileTransferProgressSender>,
        permissions: &ImportFilePermissions,
        context: &ImportFileExecutionContext,
        verification_depth: VerificationDepth,
    ) -> AppResult<ImportFileResult> {
        let cancellation = context.cancellation_token();
        if let Some(executable) = &self.worker_executable {
            let preparation_permit = wait_for_import_operation_or_cancel(
                cancellation.as_ref(),
                self.placement.acquire_preparation(),
            )
            .await?;
            let prepare_request = ImportFileWorkerRequest::Prepare {
                version: IMPORT_FILE_WORKER_PROTOCOL_VERSION,
                nonce: IMPORT_FILE_WORKER_NONCE.fetch_add(1, Ordering::Relaxed),
                source: source.to_path_buf(),
                dest: dest.to_path_buf(),
                mode,
                expected_source: expected_source.cloned(),
                permissions: permissions.clone(),
                verification_depth,
            };
            let mut session = spawn_import_file_worker(executable, &prepare_request).await?;
            let prepared = loop {
                match next_import_worker_event_or_cancel(&mut session, cancellation.as_ref())
                    .await?
                {
                    ImportFileWorkerEvent::Stage { .. } => {}
                    ImportFileWorkerEvent::Prepared { prepared, .. } => {
                        wait_for_import_worker_exit(&mut session, "after preparing import").await?;
                        break prepared;
                    }
                    ImportFileWorkerEvent::Error { message, .. } => {
                        return Err(AppError::Repository(message));
                    }
                    _ => {
                        return Err(AppError::ManualReconciliationRequired(
                            "filesystem worker returned an unexpected preparation response; inspect source and destination"
                                .to_string(),
                        ));
                    }
                }
            };
            drop(preparation_permit);

            let client_key = context.client_lane_key().to_string();
            let mut fast_permit = Some(
                wait_for_import_operation_or_cancel(
                    cancellation.as_ref(),
                    self.placement.fast.acquire(&client_key, "fast"),
                )
                .await?,
            );
            context.mark_active_import_placing().await;
            let volume_key = prepared.volume_key.clone();
            let mut prepared = Some(prepared);
            let fast_request = ImportFileWorkerRequest::FastPlacement {
                version: IMPORT_FILE_WORKER_PROTOCOL_VERSION,
                nonce: IMPORT_FILE_WORKER_NONCE.fetch_add(1, Ordering::Relaxed),
                prepared: prepared
                    .as_ref()
                    .expect("prepared import is available before copy handoff")
                    .clone(),
            };
            let mut session = spawn_import_file_worker(executable, &fast_request).await?;
            let mut copy_permit = None;
            let mut copy_finalizing = false;
            loop {
                match next_import_worker_event_or_cancel(
                    &mut session,
                    if copy_finalizing {
                        None
                    } else {
                        cancellation.as_ref()
                    },
                )
                .await?
                {
                    ImportFileWorkerEvent::Stage { .. } => {}
                    ImportFileWorkerEvent::Progress {
                        phase,
                        bytes,
                        total_bytes,
                        ..
                    } => {
                        if phase == ImportTransferPhase::Finalizing && copy_permit.is_some() {
                            copy_finalizing = true;
                            context.mark_active_import_finalizing().await;
                        }
                        if let Some(progress) = &progress {
                            let _ = progress.send(ImportFileTransferProgress {
                                phase,
                                bytes,
                                total_bytes,
                            });
                        }
                    }
                    ImportFileWorkerEvent::CopyRequired {
                        volume_key: reported_volume_key,
                        ..
                    } => {
                        if reported_volume_key != volume_key {
                            return Err(AppError::ManualReconciliationRequired(
                                "filesystem worker changed destination volume identity; inspect the destination before retrying"
                                    .to_string(),
                            ));
                        }
                        wait_for_import_worker_exit(&mut session, "after requesting copy").await?;
                        drop(fast_permit.take());
                        tracing::debug!(
                            client_key = %opaque_lane_key(&client_key),
                            volume_key = %opaque_lane_key(&volume_key),
                            "released fast import lane before copy fallback"
                        );
                        copy_permit = Some(
                            wait_for_import_operation_or_cancel(
                                cancellation.as_ref(),
                                self.placement.copy.acquire(&volume_key, "copy"),
                            )
                            .await?,
                        );
                        copy_finalizing = false;
                        context.mark_active_import_copying().await;
                        let copy_request = ImportFileWorkerRequest::Copy {
                            version: IMPORT_FILE_WORKER_PROTOCOL_VERSION,
                            nonce: IMPORT_FILE_WORKER_NONCE.fetch_add(1, Ordering::Relaxed),
                            prepared: prepared
                                .take()
                                .expect("prepared import is consumed once by copy handoff"),
                        };
                        session = spawn_import_file_worker(executable, &copy_request).await?;
                    }
                    ImportFileWorkerEvent::ImportFinished { result, .. } => {
                        drop(copy_permit);
                        return Ok(result);
                    }
                    ImportFileWorkerEvent::Error { message, .. } => {
                        return Err(AppError::Repository(message));
                    }
                    _ => {
                        return Err(AppError::ManualReconciliationRequired(
                            "filesystem worker returned an unexpected import response; inspect source and destination"
                                .to_string(),
                        ));
                    }
                }
            }
        }
        let source = source.to_path_buf();
        let dest = dest.to_path_buf();
        let expected_source = expected_source.cloned();
        let permissions = permissions.clone();
        let source_cleanup_required = mode == ImportMode::Move;
        let preparation_permit = wait_for_import_operation_or_cancel(
            cancellation.as_ref(),
            self.placement.acquire_preparation(),
        )
        .await?;
        let mut prepared = tokio::task::spawn_blocking(move || {
            prepare_import_placement_blocking(
                source,
                dest,
                ImportFileOptions::default(),
                verification_depth,
                source_cleanup_required,
                expected_source,
                progress,
                permissions,
            )
        })
        .await
        .map_err(|error| {
            AppError::Repository(format!("import preparation task panicked: {error}"))
        })??;
        drop(preparation_permit);

        let client_key = context.client_lane_key().to_string();
        let fast_permit = wait_for_import_operation_or_cancel(
            cancellation.as_ref(),
            self.placement.fast.acquire(&client_key, "fast"),
        )
        .await?;
        context.mark_active_import_placing().await;
        prepared.cancellation = cancellation.clone();
        let fast_result =
            tokio::task::spawn_blocking(move || try_fast_import_placement_blocking(prepared))
                .await
                .map_err(|error| {
                    AppError::Repository(format!("fast import task panicked: {error}"))
                })??;

        match fast_result {
            FastPlacementResult::Finished(result) => Ok(result),
            FastPlacementResult::CopyRequired(prepared) => {
                let volume_key = prepared.volume_key.clone();
                drop(fast_permit);
                tracing::debug!(
                    client_key = %opaque_lane_key(&client_key),
                    volume_key = %opaque_lane_key(&volume_key),
                    "released fast import lane before copy fallback"
                );
                let _copy_permit = wait_for_import_operation_or_cancel(
                    cancellation.as_ref(),
                    self.placement.copy.acquire(&volume_key, "copy"),
                )
                .await?;
                context.mark_active_import_copying().await;
                tokio::task::spawn_blocking(move || copy_prepared_import_blocking(prepared))
                    .await
                    .map_err(|error| {
                        AppError::Repository(format!("copy import task panicked: {error}"))
                    })?
            }
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Write};
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::fmt::MakeWriter;

    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;

    #[test]
    fn import_worker_limits_default_validate_and_clamp() {
        assert_eq!(parse_import_worker_limit(None, 2, 8), 2);
        assert_eq!(parse_import_worker_limit(Some(""), 2, 8), 2);
        assert_eq!(parse_import_worker_limit(Some("invalid"), 2, 8), 2);
        assert_eq!(parse_import_worker_limit(Some("0"), 2, 8), 2);
        assert_eq!(parse_import_worker_limit(Some(" 3 "), 2, 8), 3);
        assert_eq!(parse_import_worker_limit(Some("99"), 2, 8), 8);
    }

    #[test]
    fn import_file_stall_timeout_is_generous_and_clamped() {
        assert_eq!(
            parse_import_file_stall_timeout(None),
            DEFAULT_IMPORT_FILE_STALL_TIMEOUT_SECONDS
        );
        assert_eq!(
            parse_import_file_stall_timeout(Some("")),
            DEFAULT_IMPORT_FILE_STALL_TIMEOUT_SECONDS
        );
        assert_eq!(
            parse_import_file_stall_timeout(Some("invalid")),
            DEFAULT_IMPORT_FILE_STALL_TIMEOUT_SECONDS
        );
        assert_eq!(
            parse_import_file_stall_timeout(Some("60")),
            DEFAULT_IMPORT_FILE_STALL_TIMEOUT_SECONDS
        );
        assert_eq!(parse_import_file_stall_timeout(Some("3600")), 3600);
        assert_eq!(
            parse_import_file_stall_timeout(Some("999999")),
            MAX_IMPORT_FILE_STALL_TIMEOUT_SECONDS
        );
    }

    #[tokio::test]
    async fn copy_lanes_are_volume_scoped_and_do_not_block_fast_lanes() {
        let coordinator = ImportPlacementCoordinator::new(1, 2);
        let first_copy = coordinator.copy.acquire("volume-a", "copy").await;
        let second_copy = coordinator.copy.acquire("volume-a", "copy").await;
        let third_copy = tokio::spawn({
            let copy = coordinator.copy.clone();
            async move { copy.acquire("volume-a", "copy").await }
        });
        tokio::task::yield_now().await;
        assert!(
            !third_copy.is_finished(),
            "a third copy to one volume must wait"
        );

        let fast = tokio::time::timeout(
            Duration::from_secs(1),
            coordinator.fast.acquire("client-a", "fast"),
        )
        .await
        .expect("a hardlink lane must remain available while copies are blocked");
        let other_volume = tokio::time::timeout(
            Duration::from_secs(1),
            coordinator.copy.acquire("volume-b", "copy"),
        )
        .await
        .expect("a different volume must have independent copy capacity");

        drop(fast);
        drop(other_volume);
        drop(first_copy);
        let third_copy = tokio::time::timeout(Duration::from_secs(1), third_copy)
            .await
            .expect("waiting copy should start after one same-volume slot is released")
            .expect("copy waiter should not panic");
        drop(third_copy);
        drop(second_copy);
    }

    #[tokio::test]
    async fn fast_lanes_are_client_scoped() {
        let coordinator = ImportPlacementCoordinator::new(1, 2);
        let first = coordinator.fast.acquire("client-a", "fast").await;
        let same_client = tokio::spawn({
            let fast = coordinator.fast.clone();
            async move { fast.acquire("client-a", "fast").await }
        });
        tokio::task::yield_now().await;
        assert!(!same_client.is_finished());

        let other_client = tokio::time::timeout(
            Duration::from_secs(1),
            coordinator.fast.acquire("client-b", "fast"),
        )
        .await
        .expect("different clients should place fast imports concurrently");
        drop(other_client);
        drop(first);
        let same_client = tokio::time::timeout(Duration::from_secs(1), same_client)
            .await
            .expect("same client should continue after its lane is released")
            .expect("fast waiter should not panic");
        drop(same_client);
    }

    #[test]
    fn roots_on_the_same_physical_volume_share_a_copy_key() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source.mkv");
        std::fs::write(&source, b"video").expect("write source");
        let first = prepare_import_placement_blocking(
            source.clone(),
            temp.path().join("library-a/title.mkv"),
            ImportFileOptions::default(),
            VerificationDepth::default(),
            false,
            None,
            None,
            ImportFilePermissions::default(),
        )
        .expect("prepare first destination");
        let second = prepare_import_placement_blocking(
            source,
            temp.path().join("library-b/title.mkv"),
            ImportFileOptions::default(),
            VerificationDepth::default(),
            false,
            None,
            None,
            ImportFilePermissions::default(),
        )
        .expect("prepare second destination");
        assert_eq!(first.volume_key, second.volume_key);
    }

    #[test]
    fn fast_handoff_preserves_a_destination_created_after_preparation() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source.mkv");
        let dest = temp.path().join("library/title.mkv");
        std::fs::write(&source, b"source").expect("write source");
        let prepared = prepare_import_placement_blocking(
            source,
            dest.clone(),
            ImportFileOptions::default(),
            VerificationDepth::default(),
            false,
            None,
            None,
            ImportFilePermissions::default(),
        )
        .expect("prepare destination");
        std::fs::write(&dest, b"foreign destination").expect("create racing destination");

        let error = match try_fast_import_placement_blocking(prepared) {
            Err(error) => error,
            Ok(_) => panic!("destination race must stop the handoff"),
        };
        assert!(error.to_string().contains("destination appeared"));
        assert_eq!(
            std::fs::read(&dest).expect("read destination"),
            b"foreign destination"
        );
    }

    #[test]
    fn copy_lane_revalidates_destination_after_wait() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source.mkv");
        let dest = temp.path().join("library/title.mkv");
        std::fs::write(&source, b"source").expect("write source");
        let prepared = prepare_import_placement_blocking(
            source,
            dest.clone(),
            ImportFileOptions::default(),
            VerificationDepth::default(),
            false,
            None,
            None,
            ImportFilePermissions::default(),
        )
        .expect("prepare destination");
        std::fs::write(&dest, b"foreign destination").expect("create racing destination");

        let error = copy_prepared_import_blocking(prepared)
            .expect_err("destination race must stop copy execution");
        assert!(error.to_string().contains("destination appeared"));
        assert_eq!(
            std::fs::read(&dest).expect("read destination"),
            b"foreign destination"
        );
    }

    #[cfg(unix)]
    #[derive(Clone)]
    struct SharedLogWriter {
        buffer: Arc<Mutex<Vec<u8>>>,
    }

    #[cfg(unix)]
    impl Write for SharedLogWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.buffer.lock().expect("lock log buffer").extend(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[cfg(unix)]
    impl<'a> MakeWriter<'a> for SharedLogWriter {
        type Writer = SharedLogWriter;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    #[cfg(unix)]
    fn capture_logs<R>(f: impl FnOnce() -> R) -> (R, String) {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::fmt()
            .with_writer(SharedLogWriter {
                buffer: buffer.clone(),
            })
            .with_ansi(false)
            .without_time()
            .finish();

        let result = tracing::subscriber::with_default(subscriber, f);
        let logs = String::from_utf8(buffer.lock().expect("lock log buffer").clone())
            .expect("logs should be UTF-8");
        (result, logs)
    }

    #[tokio::test]
    async fn hardlink_or_copy_preserves_regular_source() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let dest = dir.path().join("library").join("Imported.Movie.mkv");

        let result = FsFileImporter::new()
            .import_file(&source, &dest, ImportMode::HardlinkOrCopy, None)
            .await
            .expect("import file");

        assert_eq!(result.size_bytes, 16);
        assert!(matches!(
            result.strategy,
            ImportStrategy::HardLink | ImportStrategy::Copy
        ));
        assert!(result.source_cleanup.is_none());
        assert!(source.exists());
        assert_eq!(
            std::fs::read(&dest).expect("read dest"),
            b"fake video bytes"
        );
    }

    #[cfg(unix)]
    #[test]
    fn hardlink_permission_metadata_change_is_logged() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let probe = dir.path().join("probe.mkv");
        if std::fs::hard_link(&source, &probe).is_err() {
            return;
        }
        std::fs::remove_file(&probe).expect("remove probe");
        let dest = dir.path().join("library").join("Imported.Movie.mkv");

        let (result, logs) = capture_logs(|| {
            import_file_blocking(
                source.clone(),
                dest,
                ImportMode::HardlinkOrCopy,
                ImportFileOptions::default(),
                None,
                None,
                ImportFilePermissions {
                    set_permissions_linux: true,
                    file_chmod: Some("640".to_string()),
                    folder_chmod: None,
                    chown_group: None,
                },
            )
        });
        let result = result.expect("import file");

        assert_eq!(result.strategy, ImportStrategy::HardLink);
        assert!(
            logs.contains("metadata changes affect the shared download-side inode"),
            "expected hardlink metadata log, got: {logs}"
        );
    }

    #[tokio::test]
    async fn snapshot_import_source_is_stable_for_regular_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");

        let importer = FsFileImporter::new();
        let snapshot = importer
            .snapshot_import_source(&source)
            .await
            .expect("snapshot source");
        let second_snapshot = importer
            .snapshot_import_source(&source)
            .await
            .expect("snapshot source again");

        assert_eq!(snapshot, second_snapshot);
        assert!(matches!(
            snapshot.identity.kind,
            ImportSourceIdentityKind::Regular
        ));
        assert_eq!(snapshot.proof.size_bytes, 16);
    }

    #[tokio::test]
    async fn import_file_rejects_replaced_regular_source_after_snapshot() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let dest = dir.path().join("library").join("Imported.Movie.mkv");

        let importer = FsFileImporter::new();
        let snapshot = importer
            .snapshot_import_source(&source)
            .await
            .expect("snapshot source");
        std::fs::write(&source, b"changed video bytes").expect("replace source");

        let error = importer
            .import_file(&source, &dest, ImportMode::HardlinkOrCopy, Some(&snapshot))
            .await
            .expect_err("changed source should fail import");

        assert!(
            error
                .to_string()
                .contains("import source changed after validation")
        );
        assert!(!dest.exists());
    }

    #[test]
    fn destination_guard_rejects_parent_replacement_after_approval() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let parent = dir.path().join("library");
        let dest = parent.join("Imported.Movie.mkv");
        let (_source_fingerprint, _size, guard) =
            prepare_import_destination(&source, &dest, &ImportFilePermissions::default())
                .expect("prepare destination");

        let old_parent = dir.path().join("library-old");
        std::fs::rename(&parent, &old_parent).expect("replace approved parent");
        std::fs::create_dir_all(&parent).expect("create replacement parent");
        std::fs::write(&dest, b"fake video bytes").expect("write destination");

        let error = validate_import_destination_guard(&guard, &dest)
            .expect_err("changed parent should be rejected");
        assert!(
            error
                .to_string()
                .contains("import destination parent changed during placement")
        );
    }

    #[test]
    fn destination_guard_allows_child_creation_in_approved_parent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let parent = dir.path().join("library");
        let dest = parent.join("Imported.Movie.mkv");
        let (_source_fingerprint, _size, guard) =
            prepare_import_destination(&source, &dest, &ImportFilePermissions::default())
                .expect("prepare destination");

        std::fs::write(&dest, b"fake video bytes").expect("write destination");

        validate_import_destination_guard(&guard, &dest)
            .expect("normal child creation should not change parent identity");
    }

    #[test]
    fn destination_guard_after_placement_preserves_files_when_parent_changed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let parent = dir.path().join("library");
        let dest = parent.join("Imported.Movie.mkv");
        let (_source_fingerprint, _size, guard) =
            prepare_import_destination(&source, &dest, &ImportFilePermissions::default())
                .expect("prepare destination");

        std::fs::write(&dest, b"placed video bytes").expect("write placed destination");
        let old_parent = dir.path().join("library-old");
        std::fs::rename(&parent, &old_parent).expect("replace approved parent");
        std::fs::create_dir_all(&parent).expect("create replacement parent");
        std::fs::write(&dest, b"replacement parent bytes").expect("write replacement occupant");

        let error = validate_import_destination_guard_after_placement(&guard, &dest)
            .expect_err("changed parent should be rejected");
        assert!(
            error
                .to_string()
                .contains("import destination parent changed during placement")
        );
        assert_eq!(
            std::fs::read(old_parent.join("Imported.Movie.mkv")).expect("read placed destination"),
            b"placed video bytes"
        );
        assert_eq!(
            std::fs::read(&dest).expect("read replacement occupant"),
            b"replacement parent bytes"
        );
    }

    #[test]
    fn destination_guard_after_placement_reports_cleanup_failure() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let parent = dir.path().join("library");
        let dest = parent.join("Imported.Movie.mkv");
        let (_source_fingerprint, _size, guard) =
            prepare_import_destination(&source, &dest, &ImportFilePermissions::default())
                .expect("prepare destination");

        std::fs::create_dir(&dest).expect("create non-file destination");

        let error = validate_import_destination_guard_after_placement(&guard, &dest)
            .expect_err("directory destination should be rejected");
        let message = error.to_string();
        assert!(message.contains("import destination is not a file or symlink"));
        assert!(message.contains("additionally failed to remove placed import destination"));
        assert!(dest.exists());
    }

    #[test]
    fn identical_existing_destination_is_reconciled_without_overwrite() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        let dest = dir.path().join("library").join("Imported.Movie.mkv");
        std::fs::create_dir_all(dest.parent().expect("parent")).expect("create library");
        std::fs::write(&source, b"same video bytes").expect("write source");
        std::fs::write(&dest, b"same video bytes").expect("write destination");

        let result = import_file_blocking(
            source.clone(),
            dest.clone(),
            ImportMode::HardlinkOrCopy,
            ImportFileOptions::default(),
            None,
            None,
            ImportFilePermissions::default(),
        )
        .expect("reconcile identical destination");

        assert_eq!(result.strategy, ImportStrategy::Copy);
        assert_eq!(std::fs::read(&source).expect("source"), b"same video bytes");
        assert_eq!(
            std::fs::read(&dest).expect("destination"),
            b"same video bytes"
        );
    }

    #[test]
    fn different_existing_destination_requires_reconciliation_and_preserves_both_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        let dest = dir.path().join("library").join("Imported.Movie.mkv");
        std::fs::create_dir_all(dest.parent().expect("parent")).expect("create library");
        std::fs::write(&source, b"source video bytes").expect("write source");
        std::fs::write(&dest, b"other video bytes!").expect("write destination");

        let error = import_file_blocking(
            source.clone(),
            dest.clone(),
            ImportMode::HardlinkOrCopy,
            ImportFileOptions::default(),
            None,
            None,
            ImportFilePermissions::default(),
        )
        .expect_err("different destination must conflict");

        assert!(matches!(error, AppError::ManualReconciliationRequired(_)));
        assert_eq!(
            std::fs::read(&source).expect("source"),
            b"source video bytes"
        );
        assert_eq!(
            std::fs::read(&dest).expect("destination"),
            b"other video bytes!"
        );
    }

    #[tokio::test]
    async fn move_mode_places_regular_source_and_returns_cleanup_guard() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let dest = dir.path().join("library").join("Imported.Movie.mkv");

        let result = FsFileImporter::new()
            .import_file(&source, &dest, ImportMode::Move, None)
            .await
            .expect("place file");

        assert!(matches!(
            result.strategy,
            ImportStrategy::HardLink | ImportStrategy::Copy
        ));
        assert_eq!(result.size_bytes, 16);
        assert!(result.source_cleanup.is_some());
        assert!(source.exists());
        assert_eq!(
            std::fs::read(&dest).expect("read dest"),
            b"fake video bytes"
        );
    }

    // ── FR-045: the download-client copy path proves what it copied ─────────

    /// A cross-device import copy computes both hashes during the copy and
    /// proves the destination at the configured depth.
    #[test]
    fn cross_device_copy_verifies_at_full_depth_and_reports_both_hashes() {
        let source_dir = tempfile::tempdir().expect("source tempdir");
        let dest_dir = tempfile::tempdir().expect("dest tempdir");
        let source = source_dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let dest = dest_dir.path().join("Imported.Movie.mkv");

        let result = import_file_blocking_at_depth(
            source.clone(),
            dest.clone(),
            ImportMode::Move,
            ImportFileOptions {
                force_cross_device_move: true,
                ..Default::default()
            },
            VerificationDepth::Full,
            None,
            None,
            ImportFilePermissions::default(),
        )
        .expect("cross-device copy");

        let verification = result.verification.expect("a copy is verified");
        assert_eq!(
            verification.outcome,
            scryer_domain::FileVerificationOutcome::Verified
        );
        assert_eq!(verification.depth.applied, VerificationDepth::Full);
        assert!(!verification.depth.fell_back);
        assert_eq!(verification.stamp(), "verified (full)");

        // The hashes are of the bytes, computed in the same pass that wrote
        // them (D2) — not a second read. Compared against the same hasher fed
        // directly, so a copy loop that dropped or double-fed a buffer would
        // show up here.
        let mut expected = StreamedContentHasher::new();
        expected.update(b"fake video bytes");
        assert_eq!(verification.hashes, expected.finalize());
        assert_eq!(
            verification.hashes.size_bytes,
            b"fake video bytes".len() as u64
        );
        assert_eq!(
            verification.hashes.crc_algorithm,
            scryer_domain::MoveCrcAlgorithm::Crc64Nvme
        );
        assert_eq!(verification.hashes.full_blake3.len(), 64);
    }

    /// US9 scenario 2: quick check records the reduced guarantee so it is
    /// auditable, and is not reported as a fallback.
    #[test]
    fn cross_device_copy_at_quick_depth_records_the_reduced_guarantee() {
        let source_dir = tempfile::tempdir().expect("source tempdir");
        let dest_dir = tempfile::tempdir().expect("dest tempdir");
        let source = source_dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let dest = dest_dir.path().join("Imported.Movie.mkv");

        let result = import_file_blocking_at_depth(
            source,
            dest,
            ImportMode::Move,
            ImportFileOptions {
                force_cross_device_move: true,
                ..Default::default()
            },
            VerificationDepth::Quick,
            None,
            None,
            ImportFilePermissions::default(),
        )
        .expect("cross-device copy");

        let verification = result.verification.expect("a copy is verified");
        assert_eq!(
            verification.outcome,
            scryer_domain::FileVerificationOutcome::Verified
        );
        assert_eq!(verification.depth.applied, VerificationDepth::Quick);
        assert!(
            !verification.depth.fell_back,
            "quick by preference is not a fallback"
        );
        assert_eq!(verification.stamp(), "verified (quick)");
    }

    /// SC-006: a byte flipped after write is caught before the source is
    /// touched. The import fails, so there is no cleanup guard and no path to
    /// source removal (FR-044) — and the source is still there.
    #[test]
    fn a_corrupted_destination_fails_the_import_and_leaves_the_source_alone() {
        let source_dir = tempfile::tempdir().expect("source tempdir");
        let dest_dir = tempfile::tempdir().expect("dest tempdir");
        let source = source_dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let dest = dest_dir.path().join("Imported.Movie.mkv");

        let error = import_file_blocking_at_depth(
            source.clone(),
            dest.clone(),
            ImportMode::Move,
            ImportFileOptions {
                force_cross_device_move: true,
                force_destination_corruption: true,
                ..Default::default()
            },
            VerificationDepth::Full,
            None,
            None,
            ImportFilePermissions::default(),
        )
        .expect_err("a corrupted destination must not be reported as an import");

        assert!(
            error.to_string().contains("copy verification failed"),
            "unexpected error: {error}"
        );
        assert!(source.exists(), "the source is never touched on a failure");
        assert!(
            !dest.exists(),
            "the unproven destination is removed rather than left to be imported"
        );
    }

    /// FR-042: full is requested, the read-back cannot run, and verification
    /// lands on the quick floor rather than failing the import. Behavioural
    /// compatibility — a filesystem that cannot be re-read still imports.
    #[test]
    fn a_read_back_that_cannot_run_falls_back_to_the_quick_floor() {
        let source_dir = tempfile::tempdir().expect("source tempdir");
        let dest_dir = tempfile::tempdir().expect("dest tempdir");
        let source = source_dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let dest = dest_dir.path().join("Imported.Movie.mkv");

        let result = import_file_blocking_at_depth(
            source.clone(),
            dest.clone(),
            ImportMode::Move,
            ImportFileOptions {
                force_cross_device_move: true,
                force_read_back_unsupported: true,
                ..Default::default()
            },
            VerificationDepth::Full,
            None,
            None,
            ImportFilePermissions::default(),
        )
        .expect("an unsupported read-back must not fail the import");

        let verification = result.verification.expect("a copy is verified");
        assert_eq!(
            verification.outcome,
            scryer_domain::FileVerificationOutcome::Verified
        );
        assert_eq!(verification.depth.requested, VerificationDepth::Full);
        assert_eq!(verification.depth.applied, VerificationDepth::Quick);
        assert!(
            verification.depth.fell_back,
            "the reduced guarantee must be recorded as a fallback"
        );
        assert_eq!(verification.stamp(), "verified (quick (fallback))");
        assert!(
            result.source_cleanup.is_some(),
            "the quick floor still unblocks source removal"
        );
        assert!(dest.exists());
    }

    /// FR-032: a same-filesystem placement copies no bytes, so there is nothing
    /// to hash and nothing to verify. The fast path stays untouched.
    #[test]
    fn a_hardlink_placement_carries_no_verification() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let dest = dir.path().join("library").join("Imported.Movie.mkv");

        let result = import_file_blocking_at_depth(
            source,
            dest,
            ImportMode::HardlinkOrCopy,
            ImportFileOptions::default(),
            VerificationDepth::Full,
            None,
            None,
            ImportFilePermissions::default(),
        )
        .expect("same-filesystem placement");

        assert_eq!(result.strategy, ImportStrategy::HardLink);
        assert!(
            result.verification.is_none(),
            "a hardlink copies no bytes, so it proves and hashes nothing"
        );
    }


    #[test]
    fn move_mode_cross_device_fallback_copies_then_cleanup_deletes_source() {
        let source_dir = tempfile::tempdir().expect("source tempdir");
        let dest_dir = tempfile::tempdir().expect("dest tempdir");
        let source = source_dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let dest = dest_dir.path().join("Imported.Movie.mkv");

        let result = import_file_blocking(
            source.clone(),
            dest.clone(),
            ImportMode::Move,
            ImportFileOptions {
                force_cross_device_move: true,
                ..Default::default()
            },
            None,
            None,
            ImportFilePermissions::default(),
        )
        .expect("move placement fallback");

        assert_eq!(result.strategy, ImportStrategy::Copy);
        assert!(source.exists());
        assert_eq!(
            std::fs::read(&dest).expect("read dest"),
            b"fake video bytes"
        );

        remove_import_source_after_verified_import_blocking(
            result.source_cleanup.expect("cleanup guard"),
            dest.clone(),
            ImportFileOptions::default(),
        )
        .expect("source cleanup");

        assert!(!source.exists());
        assert_eq!(
            std::fs::read(&dest).expect("read dest after cleanup"),
            b"fake video bytes"
        );
    }

    #[cfg(unix)]
    #[test]
    fn hardlink_or_copy_copies_foreign_uid_source() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let dest = dir.path().join("library").join("Imported.Movie.mkv");

        let result = import_file_blocking(
            source.clone(),
            dest.clone(),
            ImportMode::HardlinkOrCopy,
            ImportFileOptions {
                force_foreign_source_uid: true,
                ..Default::default()
            },
            None,
            None,
            ImportFilePermissions::default(),
        )
        .expect("import file");

        assert_eq!(result.strategy, ImportStrategy::Copy);
        assert!(result.source_cleanup.is_none());
        assert!(source.exists());
        assert_eq!(
            std::fs::read(&dest).expect("read dest"),
            b"fake video bytes"
        );
        assert_eq!(
            std::fs::metadata(&dest).expect("dest metadata").uid(),
            effective_uid()
        );
    }

    #[cfg(unix)]
    #[test]
    fn move_mode_copies_foreign_uid_source_then_cleanup_deletes_source() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let dest = dir.path().join("library").join("Imported.Movie.mkv");

        let result = import_file_blocking(
            source.clone(),
            dest.clone(),
            ImportMode::Move,
            ImportFileOptions {
                force_foreign_source_uid: true,
                ..Default::default()
            },
            None,
            None,
            ImportFilePermissions::default(),
        )
        .expect("move import");

        assert_eq!(result.strategy, ImportStrategy::Copy);
        assert!(source.exists());

        remove_import_source_after_verified_import_blocking(
            result.source_cleanup.expect("cleanup guard"),
            dest.clone(),
            ImportFileOptions::default(),
        )
        .expect("source cleanup");

        assert!(!source.exists());
        assert_eq!(
            std::fs::metadata(&dest).expect("dest metadata").uid(),
            effective_uid()
        );
    }

    #[cfg(unix)]
    #[test]
    fn final_uid_mismatch_fails_import_and_removes_destination() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let dest = dir.path().join("library").join("Imported.Movie.mkv");

        let error = import_file_blocking(
            source.clone(),
            dest.clone(),
            ImportMode::HardlinkOrCopy,
            ImportFileOptions {
                force_destination_uid_mismatch: true,
                ..Default::default()
            },
            None,
            None,
            ImportFilePermissions::default(),
        )
        .expect_err("uid mismatch should fail import");

        assert!(matches!(error, AppError::Repository(_)));
        assert!(!dest.exists());
        assert!(source.exists());
    }

    #[cfg(unix)]
    #[test]
    fn enabled_permissions_apply_file_mask_and_created_folder_mask() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let library_dir = dir.path().join("library");
        let dest = library_dir.join("Imported.Movie.mkv");
        let egid = unsafe { libc::getegid() };

        import_file_blocking(
            source,
            dest.clone(),
            ImportMode::HardlinkOrCopy,
            ImportFileOptions {
                force_foreign_source_uid: true,
                ..Default::default()
            },
            None,
            None,
            ImportFilePermissions {
                set_permissions_linux: true,
                file_chmod: Some("2640".to_string()),
                folder_chmod: Some("2750".to_string()),
                chown_group: Some(egid.to_string()),
            },
        )
        .expect("import file");

        assert_eq!(std::fs::metadata(&dest).expect("dest metadata").gid(), egid);
        assert_eq!(
            std::fs::metadata(&dest).expect("dest metadata").mode() & 0o7777,
            0o2640
        );
        assert_eq!(
            std::fs::metadata(&library_dir)
                .expect("library dir metadata")
                .gid(),
            egid
        );
        assert_eq!(
            std::fs::metadata(&library_dir)
                .expect("library dir metadata")
                .mode()
                & 0o7777,
            0o2750
        );
    }

    #[cfg(unix)]
    #[test]
    fn missing_file_mask_derives_from_folder_mask_without_execute_bits() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let dest = dir.path().join("library").join("Imported.Movie.mkv");

        import_file_blocking(
            source,
            dest.clone(),
            ImportMode::HardlinkOrCopy,
            ImportFileOptions {
                force_foreign_source_uid: true,
                ..Default::default()
            },
            None,
            None,
            ImportFilePermissions {
                set_permissions_linux: true,
                file_chmod: None,
                folder_chmod: Some("755".to_string()),
                chown_group: None,
            },
        )
        .expect("import file");

        assert_eq!(
            std::fs::metadata(&dest).expect("dest metadata").mode() & 0o7777,
            0o644
        );
    }

    #[cfg(unix)]
    #[test]
    fn numeric_group_chown_keeps_destination_uid() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let dest = dir.path().join("library").join("Imported.Movie.mkv");
        let egid = unsafe { libc::getegid() };

        import_file_blocking(
            source,
            dest.clone(),
            ImportMode::HardlinkOrCopy,
            ImportFileOptions {
                force_foreign_source_uid: true,
                ..Default::default()
            },
            None,
            None,
            ImportFilePermissions {
                set_permissions_linux: true,
                file_chmod: None,
                folder_chmod: None,
                chown_group: Some(egid.to_string()),
            },
        )
        .expect("import file");

        let metadata = std::fs::metadata(&dest).expect("dest metadata");
        assert_eq!(metadata.uid(), effective_uid());
        assert_eq!(metadata.gid(), egid);
    }

    #[cfg(unix)]
    #[test]
    fn empty_group_is_invalid_input() {
        let error = resolve_group_id("").expect_err("empty group should be rejected");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn cross_device_copy_reports_transfer_progress() {
        let source_dir = tempfile::tempdir().expect("source tempdir");
        let dest_dir = tempfile::tempdir().expect("dest tempdir");
        let source = source_dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let dest = dest_dir.path().join("Imported.Movie.mkv");
        let total_bytes = std::fs::metadata(&source).expect("source metadata").len();
        let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel();

        let result = import_file_blocking(
            source.clone(),
            dest.clone(),
            ImportMode::Move,
            ImportFileOptions {
                force_cross_device_move: true,
                ..Default::default()
            },
            None,
            Some(progress_tx),
            ImportFilePermissions::default(),
        )
        .expect("move placement fallback");

        assert_eq!(result.strategy, ImportStrategy::Copy);

        let mut updates = Vec::new();
        while let Ok(update) = progress_rx.try_recv() {
            updates.push(update);
        }

        assert!(updates.iter().any(|update| {
            update.phase == ImportTransferPhase::Copying
                && update.bytes == 0
                && update.total_bytes == total_bytes
        }));
        assert!(updates.iter().any(|update| {
            update.phase == ImportTransferPhase::Copying
                && update.bytes == total_bytes
                && update.total_bytes == total_bytes
        }));
        assert!(updates.iter().any(|update| {
            update.phase == ImportTransferPhase::Finalizing
                && update.bytes == total_bytes
                && update.total_bytes == total_bytes
        }));
    }

    #[test]
    fn move_mode_copy_failure_leaves_source() {
        let source_dir = tempfile::tempdir().expect("source tempdir");
        let dest_dir = tempfile::tempdir().expect("dest tempdir");
        let source = source_dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let dest = dest_dir.path().join("Imported.Movie.mkv");
        std::fs::create_dir(dest.with_extension("tmp_import")).expect("create temp conflict");

        let error = import_file_blocking(
            source.clone(),
            dest.clone(),
            ImportMode::Move,
            ImportFileOptions {
                force_cross_device_move: true,
                ..Default::default()
            },
            None,
            None,
            ImportFilePermissions::default(),
        )
        .expect_err("copy should fail");

        assert!(error.to_string().contains("import copy failed"));
        assert!(source.exists());
        assert!(!dest.exists());
    }

    #[test]
    fn move_mode_copy_retries_transient_bad_file_descriptor() {
        let source_dir = tempfile::tempdir().expect("source tempdir");
        let dest_dir = tempfile::tempdir().expect("dest tempdir");
        let source = source_dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let dest = dest_dir.path().join("Imported.Movie.mkv");
        let temp_dest = dest.with_extension("tmp_import");

        let result = import_file_blocking(
            source.clone(),
            dest.clone(),
            ImportMode::Move,
            ImportFileOptions {
                force_cross_device_move: true,
                force_transient_copy_failures: 1,
                ..Default::default()
            },
            None,
            None,
            ImportFilePermissions::default(),
        )
        .expect("copy should retry and succeed");

        assert_eq!(result.strategy, ImportStrategy::Copy);
        assert!(source.exists());
        assert_eq!(
            std::fs::read(&dest).expect("read dest"),
            b"fake video bytes"
        );
        assert!(!temp_dest.exists());
    }

    #[test]
    fn move_mode_copy_exhausts_transient_bad_file_descriptor_retries() {
        let source_dir = tempfile::tempdir().expect("source tempdir");
        let dest_dir = tempfile::tempdir().expect("dest tempdir");
        let source = source_dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let dest = dest_dir.path().join("Imported.Movie.mkv");
        let temp_dest = dest.with_extension("tmp_import");

        let error = import_file_blocking(
            source.clone(),
            dest.clone(),
            ImportMode::Move,
            ImportFileOptions {
                force_cross_device_move: true,
                force_transient_copy_failures: 3,
                ..Default::default()
            },
            None,
            None,
            ImportFilePermissions::default(),
        )
        .expect_err("copy should fail after retry budget");

        let message = error.to_string();
        assert!(message.contains("import copy failed after 3 attempt(s)"));
        assert!(message.contains("copy: Bad file descriptor"));
        assert!(source.exists());
        assert!(!dest.exists());
        assert!(!temp_dest.exists());
    }

    #[test]
    fn move_mode_copy_does_not_retry_non_transient_copy_error() {
        let source_dir = tempfile::tempdir().expect("source tempdir");
        let dest_dir = tempfile::tempdir().expect("dest tempdir");
        let source = source_dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let dest = dest_dir.path().join("Imported.Movie.mkv");
        let temp_dest = dest.with_extension("tmp_import");

        let error = import_file_blocking(
            source.clone(),
            dest.clone(),
            ImportMode::Move,
            ImportFileOptions {
                force_cross_device_move: true,
                force_non_transient_copy_failure: true,
                ..Default::default()
            },
            None,
            None,
            ImportFilePermissions::default(),
        )
        .expect_err("copy should fail without retry");

        let message = error.to_string();
        assert!(message.contains("import copy failed after 1 attempt(s)"));
        assert!(message.contains("copy: forced non-transient copy failure"));
        assert!(source.exists());
        assert!(!dest.exists());
        assert!(!temp_dest.exists());
    }

    #[test]
    fn move_mode_verification_failure_leaves_source() {
        let source_dir = tempfile::tempdir().expect("source tempdir");
        let dest_dir = tempfile::tempdir().expect("dest tempdir");
        let source = source_dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let dest = dest_dir.path().join("Imported.Movie.mkv");

        let error = import_file_blocking(
            source.clone(),
            dest.clone(),
            ImportMode::Move,
            ImportFileOptions {
                force_cross_device_move: true,
                force_copy_verification_failure: true,
                ..Default::default()
            },
            None,
            None,
            ImportFilePermissions::default(),
        )
        .expect_err("verification should fail");

        assert!(error.to_string().contains("copy verification failed"));
        assert!(source.exists());
        assert!(!dest.exists());
    }

    #[test]
    fn move_mode_cleanup_delete_failure_reports_failure_without_removing_dest() {
        let source_dir = tempfile::tempdir().expect("source tempdir");
        let dest_dir = tempfile::tempdir().expect("dest tempdir");
        let source = source_dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let dest = dest_dir.path().join("Imported.Movie.mkv");

        let result = import_file_blocking(
            source.clone(),
            dest.clone(),
            ImportMode::Move,
            ImportFileOptions {
                force_cross_device_move: true,
                ..Default::default()
            },
            None,
            None,
            ImportFilePermissions::default(),
        )
        .expect("place file");

        let error = remove_import_source_after_verified_import_blocking(
            result.source_cleanup.expect("cleanup guard"),
            dest.clone(),
            ImportFileOptions {
                force_delete_failure: true,
                ..Default::default()
            },
        )
        .expect_err("delete should fail");

        assert!(error.to_string().contains("failed to remove source"));
        assert!(source.exists());
        assert!(dest.exists());
    }

    #[test]
    fn move_mode_cleanup_refuses_changed_source() {
        let source_dir = tempfile::tempdir().expect("source tempdir");
        let dest_dir = tempfile::tempdir().expect("dest tempdir");
        let source = source_dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let dest = dest_dir.path().join("Imported.Movie.mkv");

        let result = import_file_blocking(
            source.clone(),
            dest.clone(),
            ImportMode::Move,
            ImportFileOptions {
                force_cross_device_move: true,
                ..Default::default()
            },
            None,
            None,
            ImportFilePermissions::default(),
        )
        .expect("place file");

        std::fs::write(&source, b"different video bytes").expect("change source");
        let error = remove_import_source_after_verified_import_blocking(
            result.source_cleanup.expect("cleanup guard"),
            dest.clone(),
            ImportFileOptions::default(),
        )
        .expect_err("changed source should fail cleanup");

        assert!(error.to_string().contains("source changed"));
        assert!(source.exists());
        assert!(dest.exists());
    }

    #[test]
    fn move_mode_cleanup_refuses_missing_source() {
        let source_dir = tempfile::tempdir().expect("source tempdir");
        let dest_dir = tempfile::tempdir().expect("dest tempdir");
        let source = source_dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let dest = dest_dir.path().join("Imported.Movie.mkv");

        let result = import_file_blocking(
            source.clone(),
            dest.clone(),
            ImportMode::Move,
            ImportFileOptions {
                force_cross_device_move: true,
                ..Default::default()
            },
            None,
            None,
            ImportFilePermissions::default(),
        )
        .expect("place file");

        std::fs::remove_file(&source).expect("remove source");
        let error = remove_import_source_after_verified_import_blocking(
            result.source_cleanup.expect("cleanup guard"),
            dest.clone(),
            ImportFileOptions::default(),
        )
        .expect_err("missing source should fail cleanup");

        assert!(error.to_string().contains("source is missing"));
        assert!(dest.exists());
    }

    #[test]
    fn move_mode_cleanup_refuses_missing_destination() {
        let source_dir = tempfile::tempdir().expect("source tempdir");
        let dest_dir = tempfile::tempdir().expect("dest tempdir");
        let source = source_dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let dest = dest_dir.path().join("Imported.Movie.mkv");

        let result = import_file_blocking(
            source.clone(),
            dest.clone(),
            ImportMode::Move,
            ImportFileOptions {
                force_cross_device_move: true,
                ..Default::default()
            },
            None,
            None,
            ImportFilePermissions::default(),
        )
        .expect("place file");

        std::fs::remove_file(&dest).expect("remove dest");
        let error = remove_import_source_after_verified_import_blocking(
            result.source_cleanup.expect("cleanup guard"),
            dest.clone(),
            ImportFileOptions::default(),
        )
        .expect_err("missing dest should fail cleanup");

        assert!(error.to_string().contains("destination is missing"));
        assert!(source.exists());
    }

    #[test]
    fn move_mode_cleanup_refuses_changed_destination_with_same_size() {
        let source_dir = tempfile::tempdir().expect("source tempdir");
        let dest_dir = tempfile::tempdir().expect("dest tempdir");
        let source = source_dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let dest = dest_dir.path().join("Imported.Movie.mkv");

        let result = import_file_blocking(
            source.clone(),
            dest.clone(),
            ImportMode::Move,
            ImportFileOptions {
                force_cross_device_move: true,
                ..Default::default()
            },
            None,
            None,
            ImportFilePermissions::default(),
        )
        .expect("place file");

        std::fs::write(&dest, b"same size change").expect("change dest");
        let error = remove_import_source_after_verified_import_blocking(
            result.source_cleanup.expect("cleanup guard"),
            dest.clone(),
            ImportFileOptions::default(),
        )
        .expect_err("changed dest should fail cleanup");

        assert!(error.to_string().contains("destination proof changed"));
        assert!(source.exists());
        assert!(dest.exists());
    }

    #[test]
    fn move_mode_cleanup_refuses_changed_destination_tail_sample() {
        let source_dir = tempfile::tempdir().expect("source tempdir");
        let dest_dir = tempfile::tempdir().expect("dest tempdir");
        let source = source_dir.path().join("source.mkv");
        let mut bytes =
            vec![b'a'; scryer_application::fs_integrity::IMPORT_CONTENT_PROOF_SAMPLE_BYTES + 1024];
        bytes[scryer_application::fs_integrity::IMPORT_CONTENT_PROOF_SAMPLE_BYTES + 1023] = b'z';
        std::fs::write(&source, &bytes).expect("write source");
        let dest = dest_dir.path().join("Imported.Movie.mkv");

        let result = import_file_blocking(
            source.clone(),
            dest.clone(),
            ImportMode::Move,
            ImportFileOptions {
                force_cross_device_move: true,
                ..Default::default()
            },
            None,
            None,
            ImportFilePermissions::default(),
        )
        .expect("place file");

        let mut changed_bytes = bytes;
        changed_bytes[scryer_application::fs_integrity::IMPORT_CONTENT_PROOF_SAMPLE_BYTES + 1023] =
            b'y';
        std::fs::write(&dest, &changed_bytes).expect("change dest tail");
        let error = remove_import_source_after_verified_import_blocking(
            result.source_cleanup.expect("cleanup guard"),
            dest.clone(),
            ImportFileOptions::default(),
        )
        .expect_err("changed dest tail should fail cleanup");

        assert!(error.to_string().contains("destination proof changed"));
        assert!(source.exists());
        assert!(dest.exists());
    }

    #[test]
    fn move_mode_cleanup_refuses_source_as_final_destination() {
        let source_dir = tempfile::tempdir().expect("source tempdir");
        let dest_dir = tempfile::tempdir().expect("dest tempdir");
        let source = source_dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let dest = dest_dir.path().join("Imported.Movie.mkv");

        let result = import_file_blocking(
            source.clone(),
            dest.clone(),
            ImportMode::Move,
            ImportFileOptions {
                force_cross_device_move: true,
                ..Default::default()
            },
            None,
            None,
            ImportFilePermissions::default(),
        )
        .expect("place file");

        let error = remove_import_source_after_verified_import_blocking(
            result.source_cleanup.expect("cleanup guard"),
            source.clone(),
            ImportFileOptions::default(),
        )
        .expect_err("same source and final dest should fail cleanup");

        assert!(error.to_string().contains("library file"));
        assert!(source.exists());
        assert!(dest.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn import_file_preserves_symlink_sources() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source_target = dir.path().join("source-target.mkv");
        std::fs::write(&source_target, b"fake video bytes").expect("write target");
        let source_link = dir.path().join("source-link.mkv");
        let relative_target = PathBuf::from("source-target.mkv");
        symlink(&relative_target, &source_link).expect("create source symlink");

        let dest_dir = tempfile::tempdir().expect("dest tempdir");
        let dest_path = dest_dir.path().join("Imported.Movie.mkv");
        let result = FsFileImporter::new()
            .import_file(&source_link, &dest_path, ImportMode::HardlinkOrCopy, None)
            .await
            .expect("import symlink");

        assert_eq!(result.strategy, ImportStrategy::Symlink);
        assert_eq!(result.size_bytes, 16);
        assert!(
            std::fs::symlink_metadata(&dest_path)
                .expect("dest metadata")
                .file_type()
                .is_symlink()
        );
        assert!(
            !std::fs::read_link(&dest_path)
                .expect("read dest symlink")
                .is_absolute()
        );
        assert_eq!(
            std::fs::canonicalize(&dest_path).expect("canonicalize dest symlink"),
            std::fs::canonicalize(&source_target).expect("canonicalize source target")
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn import_file_rejects_retargeted_symlink_source_after_snapshot() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source_target = dir.path().join("source-target.mkv");
        let replacement_target = dir.path().join("replacement-target.mkv");
        std::fs::write(&source_target, b"fake video bytes").expect("write target");
        std::fs::write(&replacement_target, b"other video bytes").expect("write replacement");
        let source_link = dir.path().join("source-link.mkv");
        symlink(PathBuf::from("source-target.mkv"), &source_link).expect("create source symlink");

        let importer = FsFileImporter::new();
        let snapshot = importer
            .snapshot_import_source(&source_link)
            .await
            .expect("snapshot symlink source");
        std::fs::remove_file(&source_link).expect("remove old source symlink");
        symlink(PathBuf::from("replacement-target.mkv"), &source_link)
            .expect("retarget source symlink");

        let dest_dir = tempfile::tempdir().expect("dest tempdir");
        let dest_path = dest_dir.path().join("Imported.Movie.mkv");
        let error = importer
            .import_file(
                &source_link,
                &dest_path,
                ImportMode::HardlinkOrCopy,
                Some(&snapshot),
            )
            .await
            .expect_err("changed symlink source should fail import");

        assert!(
            error
                .to_string()
                .contains("import source changed after validation")
        );
        assert!(!dest_path.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn move_mode_cleanup_removes_source_symlink_only() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source_target = dir.path().join("source-target.mkv");
        std::fs::write(&source_target, b"fake video bytes").expect("write target");
        let source_link = dir.path().join("source-link.mkv");
        symlink(PathBuf::from("source-target.mkv"), &source_link).expect("create source symlink");

        let dest_dir = tempfile::tempdir().expect("dest tempdir");
        let dest_path = dest_dir.path().join("Imported.Movie.mkv");
        let result = FsFileImporter::new()
            .import_file(&source_link, &dest_path, ImportMode::Move, None)
            .await
            .expect("import symlink");

        assert_eq!(result.strategy, ImportStrategy::Symlink);
        assert!(source_link.exists());
        assert!(source_target.exists());
        assert!(dest_path.exists());

        FsFileImporter::new()
            .remove_import_source_after_verified_import(
                result.source_cleanup.expect("cleanup guard"),
                &dest_path,
            )
            .await
            .expect("cleanup symlink source");

        assert!(!source_link.exists());
        assert!(source_target.exists());
        assert!(dest_path.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn import_file_rejects_broken_symlink_sources() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source_link = dir.path().join("broken-link.mkv");
        symlink(PathBuf::from("missing-target.mkv"), &source_link).expect("create broken symlink");

        let dest_dir = tempfile::tempdir().expect("dest tempdir");
        let dest_path = dest_dir.path().join("Imported.Movie.mkv");
        let error = FsFileImporter::new()
            .import_file(&source_link, &dest_path, ImportMode::HardlinkOrCopy, None)
            .await
            .expect_err("broken symlink should fail");

        assert!(
            error
                .to_string()
                .contains("import symlink target not found")
        );
    }
}

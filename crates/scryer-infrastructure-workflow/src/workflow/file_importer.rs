use async_trait::async_trait;
use scryer_application::{
    AppError, AppResult, FileImporter, ImportFileExecutionContext, ImportFilePermissions,
    ImportFileTransferProgress, ImportFileTransferProgressSender, RefusalFact, RefusalFactValue,
    disk::{
        self, AsideRemoval, Cancel, Disk, Done, Doubt, EntryAlias, EntryKind, LinkAfter, LinkProof,
        Mark, Open, Parents, PathReading, Phase, Placed, Progress, Refused, Removal, Remove,
        Sameness, StagedCause, StagedCopy, StagedCopyRefusal, StagedStep, Unproven, Watch,
    },
    fs_integrity::import_content_proof,
    location::verify::persisted_hashes,
};
use scryer_domain::{
    ImportDestinationDisposition, ImportFileIdentity, ImportFileResult, ImportMode,
    ImportSourceCleanupGuard, ImportSourceDisposition, ImportSourceIdentity,
    ImportSourceIdentityKind, ImportSourceSnapshot, ImportStrategy, ImportTransferPhase,
    ImportVerification, VerificationDepth,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{self, BufRead, Write};
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

fn import_io_reason(error: &io::Error) -> &'static str {
    use scryer_application::error_reasons::import;
    if let Some(reason) = error
        .get_ref()
        .and_then(|source| source.downcast_ref::<AppError>())
        .and_then(AppError::reason)
    {
        match reason {
            import::FILE_NOT_FOUND
            | import::PERMISSION_DENIED
            | import::DISK_FULL
            | import::IO_FAILED => return reason,
            _ => {}
        }
    }
    match error.kind() {
        io::ErrorKind::NotFound => import::FILE_NOT_FOUND,
        io::ErrorKind::PermissionDenied => import::PERMISSION_DENIED,
        io::ErrorKind::StorageFull => import::DISK_FULL,
        _ => import::IO_FAILED,
    }
}

fn import_io_error(error: &io::Error, message: String) -> AppError {
    AppError::RepositoryFailed {
        message,
        reason: import_io_reason(error),
    }
}

/// Context may add diagnostics but must not discard a routed failure fact.
fn import_error_context(error: &AppError, message: String) -> AppError {
    match error {
        AppError::RepositoryFailed { reason, .. } => AppError::RepositoryFailed { message, reason },
        AppError::RepositoryFailedWithFacts { reason, facts, .. } => {
            AppError::RepositoryFailedWithFacts {
                message,
                reason,
                facts: facts.clone(),
            }
        }
        _ => AppError::Repository(message),
    }
}

/// A copy that fails on a dropped file handle is made twice more, after these
/// pauses.
const IMPORT_COPY_RETRY_PAUSES: [Duration; 2] = [Duration::from_secs(1), Duration::from_secs(3)];
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

/// Destination volumes where a successful hard link could not be verified to
/// share its source's file identity. Later imports to such a volume skip the
/// link attempt and copy. Held for the process lifetime only, so a remount or
/// restart probes the volume again.
#[derive(Clone, Default)]
struct UnverifiableHardLinkVolumes {
    volumes: Arc<std::sync::Mutex<HashSet<String>>>,
}

impl UnverifiableHardLinkVolumes {
    fn shared() -> Self {
        static SHARED: OnceLock<UnverifiableHardLinkVolumes> = OnceLock::new();
        SHARED.get_or_init(Self::default).clone()
    }

    fn contains(&self, volume_key: &str) -> bool {
        self.volumes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains(volume_key)
    }

    /// Returns whether the volume was newly recorded.
    fn insert(&self, volume_key: &str) -> bool {
        self.volumes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(volume_key.to_string())
    }
}

#[derive(Clone)]
struct ImportPlacementCoordinator {
    preparation: Arc<tokio::sync::Semaphore>,
    fast: KeyedImportPermitTable,
    copy: scryer_application::location::transfer::CopyCoordinator,
    unverifiable_hard_link_volumes: UnverifiableHardLinkVolumes,
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
        let mut coordinator = Self::new(fast_workers, copy_workers);
        coordinator.copy = scryer_application::location::transfer::shared();
        coordinator.unverifiable_hard_link_volumes = UnverifiableHardLinkVolumes::shared();
        coordinator
    }

    fn new(fast_workers_per_client: usize, copy_workers_per_volume: usize) -> Self {
        Self {
            preparation: Arc::new(tokio::sync::Semaphore::new(IMPORT_PREPARATION_WORKERS)),
            fast: KeyedImportPermitTable::new(fast_workers_per_client),
            copy: scryer_application::location::transfer::CopyCoordinator::new(
                copy_workers_per_volume,
            ),
            unverifiable_hard_link_volumes: UnverifiableHardLinkVolumes::default(),
        }
    }

    /// Carries what this process knows about the placement into the fast lane,
    /// which may run in a separate worker process.
    fn ready_fast_placement(
        &self,
        prepared: &mut PreparedImportPlacement,
        archive_workspace_source: bool,
    ) {
        prepared.archive_workspace_source = archive_workspace_source;
        prepared.hard_link_identity_unverifiable = self
            .unverifiable_hard_link_volumes
            .contains(&prepared.volume_key);
    }

    fn record_hard_link_identity_mismatch(
        &self,
        volume_key: &str,
        dest: &Path,
        mismatch: &HardLinkIdentityMismatch,
    ) {
        if self.unverifiable_hard_link_volumes.insert(volume_key) {
            tracing::warn!(
                volume_key = %opaque_lane_key(volume_key),
                dest = %dest.display(),
                source_identity = %mark_label(&mismatch.source),
                linked_identity = %mark_label(&mismatch.linked),
                "hard link succeeded but the destination reports a different file identity; this filesystem does not expose stable file identity across paths (FUSE/sshfs, CIFS without serverino, mergerfs path-hash inodes), so imports to this volume copy instead of hard linking until restart"
            );
        } else {
            tracing::debug!(
                volume_key = %opaque_lane_key(volume_key),
                dest = %dest.display(),
                source_identity = %mark_label(&mismatch.source),
                linked_identity = %mark_label(&mismatch.linked),
                "hard link identity is unverifiable on this volume; copying"
            );
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
        import_error_context(
            &error,
            format!(
                "failed to build import source cleanup proof for source {}: {}",
                source.display(),
                error
            ),
        )
    })?;
    let dest_proof = import_content_proof(dest).map_err(|error| {
        import_error_context(
            &error,
            format!(
                "failed to build import source cleanup proof for destination {}: {}",
                dest.display(),
                error
            ),
        )
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
        adopted_destination_mark: None,
    })
}

/// The cleanup guard of a move whose destination this import has just
/// placed. When it cannot be built, what this import placed is taken back
/// as [`take_back_placed`] says.
fn cleanup_guard_after_placement(
    source_cleanup_required: bool,
    source: &Path,
    dest: &Path,
    source_fingerprint: &ImportSourceFingerprint,
    size: u64,
    landed: Landed<'_>,
) -> AppResult<Option<ImportSourceCleanupGuard>> {
    if !source_cleanup_required {
        return Ok(None);
    }

    cleanup_guard(source, dest, source_fingerprint, size)
        .map(Some)
        .map_err(|error| took_back(error, source, dest, landed))
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
    read_import_source(path).map(|(fingerprint, _)| fingerprint)
}

/// The fingerprint, and the mark of the name itself from the same read, for
/// a hard link to be proven against.
// disk-guard: the fingerprint holds the raw device, inode and owner uid, and a
// failure's own error kind names the import reason; a reading carries neither.
#[allow(clippy::disallowed_methods)]
fn read_import_source(path: &Path) -> AppResult<(ImportSourceFingerprint, Mark)> {
    let link_meta = std::fs::symlink_metadata(path).map_err(|e| {
        import_io_error(
            &e,
            format!(
                "import path not found or inaccessible: {}: {}",
                path.display(),
                e
            ),
        )
    })?;
    let file_type = link_meta.file_type();
    if file_type.is_symlink() {
        let source_link_target = std::fs::read_link(path).map_err(|e| {
            import_io_error(
                &e,
                format!(
                    "failed to read import symlink target: {}: {}",
                    path.display(),
                    e
                ),
            )
        })?;
        let resolved_target = resolve_symlink_target(path, &source_link_target);
        let target_meta = std::fs::metadata(&resolved_target).map_err(|e| {
            import_io_error(
                &e,
                format!(
                    "import symlink target not found or inaccessible: {} -> {}: {}",
                    path.display(),
                    resolved_target.display(),
                    e
                ),
            )
        })?;
        let fingerprint = ImportSourceFingerprint {
            file: fingerprint_from_metadata(&target_meta)?,
            kind: ImportSourceKind::Symlink {
                source_link_target,
                resolved_target,
            },
        };
        return Ok((fingerprint, Mark::of(&link_meta)));
    }
    if !file_type.is_file() {
        return Err(AppError::Repository(format!(
            "import path is not a regular file: {}",
            path.display()
        )));
    }
    let fingerprint = ImportSourceFingerprint {
        file: fingerprint_from_metadata(&link_meta)?,
        kind: ImportSourceKind::Regular,
    };
    Ok((fingerprint, Mark::of(&link_meta)))
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

// disk-guard: the persisted guard compares raw device and inode, or the Windows
// volume serial and file index from the handle, which the owner does not serve.
#[allow(clippy::disallowed_methods)]
fn directory_fingerprint_from_path(path: &Path) -> AppResult<DirectoryFingerprint> {
    let metadata = std::fs::metadata(path).map_err(|e| {
        import_io_error(
            &e,
            format!(
                "failed to stat destination directory {}: {}",
                path.display(),
                e
            ),
        )
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
                import_io_error(
                    &e,
                    format!(
                        "failed to open destination directory {} for identity check: {}",
                        path.display(),
                        e
                    ),
                )
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
    same_source_mark(path, expected).map(drop)
}

/// [`ensure_same_source`], handing back the mark of the source's name from
/// that same read.
fn same_source_mark(path: &Path, expected: &ImportSourceFingerprint) -> AppResult<Mark> {
    let (actual, mark) = read_import_source(path)?;
    if &actual != expected {
        return Err(AppError::Repository(format!(
            "import source changed during import: {}",
            path.display()
        )));
    }
    Ok(mark)
}

#[cfg(unix)]
fn same_file_identity(actual: &FileFingerprint, expected: &FileFingerprint) -> bool {
    actual.dev == expected.dev && actual.ino == expected.ino
}

#[cfg(not(unix))]
fn same_file_identity(actual: &FileFingerprint, expected: &FileFingerprint) -> bool {
    actual == expected
}

/// A hard link that succeeded but whose destination reports a different file
/// identity than its source.
///
/// Filesystems that synthesise inode numbers per path (sshfs and other FUSE
/// mounts, CIFS without `serverino`, mergerfs `inodecalc=path-hash`) report this
/// for a genuine link, so the mismatch proves only that the link cannot be
/// verified. The link is never trusted; the import goes to the copy lane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct HardLinkIdentityMismatch {
    /// The source as it was read before the link, for the log.
    source: String,
    /// The new name as it was read after the link, for the log.
    linked: String,
}

/// What became of an attempt to hard link an import source.
enum ImportLink {
    /// Linked, and the new name proved to be the source at its length.
    Proven,
    /// This destination cannot hold a hard link to the source.
    NotPossible,
    /// The link was not made, in the disk's own words.
    Failed(String),
    /// Linked but not proven. The disk service removed the link again.
    Unproven(LinkDoubt),
    /// Linked but not proven, and the new name was already gone when the
    /// disk service went to remove it. Carries the service's words.
    UnprovenGone(String),
    /// Linked but not proven, and another writer's regular file now holds
    /// the name, which the disk service left alone.
    Occupied,
}

/// Why a hard link that was made is not trusted.
enum LinkDoubt {
    Identity(HardLinkIdentityMismatch),
    /// What the new name resolves to has another length or could not be
    /// read.
    Length(PathReading),
}

/// A stored mark in a few words for a log line.
fn mark_label(stored: &str) -> String {
    Mark::from_stored(stored).map_or_else(|| "unread".to_owned(), |mark| mark.label())
}

fn io_other(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> io::Error {
    io::Error::other(error)
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
    /// Flip a byte in the destination after it is written, before it is
    /// verified. The only way to exercise "a corrupted destination copy is
    /// detected before source removal" without an unreliable disk.
    #[cfg(test)]
    force_destination_corruption: bool,
    /// Make the full-depth read-back report itself unsupported, as a filesystem
    /// that refuses the cache-bypassed re-read would, which lands on the quick floor.
    #[cfg(test)]
    force_read_back_unsupported: bool,
    #[cfg(test)]
    force_delete_failure: bool,
    #[cfg(all(test, unix))]
    force_foreign_source_uid: bool,
    #[cfg(all(test, unix))]
    destination_uid_override: Option<u32>,
    /// Make a successful hard link report a different file identity, as a
    /// filesystem that synthesises inode numbers per path does.
    #[cfg(test)]
    force_hard_link_identity_mismatch: bool,
    #[cfg(test)]
    rename_fault: RenameFault,
    #[cfg(test)]
    link_answer: LinkAnswer,
    /// Runs once a copy holds the destination name, before the import reads
    /// the source and the copy again.
    #[cfg(test)]
    after_copy_promoted: Option<fn(&Path, &Path)>,
    /// Runs once a copy is verified, before the import builds its cleanup
    /// guard.
    #[cfg(test)]
    after_copy_verified: Option<fn(&Path, &Path)>,
    /// Runs once a copy's proof is read, before the import acts on it.
    #[cfg(test)]
    after_copy_assessed: Option<fn(&Path, &Path)>,
    /// Runs once a hard link proved, before the source is read again.
    #[cfg(test)]
    after_hard_link: Option<fn(&Path, &Path)>,
}

/// What the disk service is made to answer for an unproven hard link, in
/// place of making one: states a test cannot reach between the link and its
/// proof.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum LinkAnswer {
    #[default]
    Real,
    /// The link did not prove, and something else had already removed it.
    AlreadyGone,
    /// The source was swapped for another file, and the name holds another
    /// writer's regular file, which the service left alone.
    LeftAloneAfterSourceSwap,
    /// The source is unchanged, and the name holds another writer's regular
    /// file, which the service left alone.
    LeftAlone,
}

/// The answer a test made the disk service give, as the refusal's parts.
#[cfg(test)]
fn forced_link_answer(
    answer: LinkAnswer,
    source: &Path,
    dest: &Path,
) -> Option<(Refused, String, Option<Unproven>)> {
    let after = match answer {
        LinkAnswer::Real => return None,
        LinkAnswer::AlreadyGone => LinkAfter::AlreadyGone,
        LinkAnswer::LeftAloneAfterSourceSwap => {
            let swapped = source.with_file_name("swapped-in.mkv");
            std::fs::write(&swapped, b"a different, longer payload").expect("write swap");
            std::fs::rename(&swapped, source).expect("swap the source");
            std::fs::write(dest, b"another writer").expect("write occupant");
            LinkAfter::LeftAlone
        }
        LinkAnswer::LeftAlone => {
            std::fs::write(dest, b"another writer").expect("write occupant");
            LinkAfter::LeftAlone
        }
    };
    let linked = match std::fs::symlink_metadata(dest) {
        Ok(metadata) => Mark::of(&metadata),
        Err(_) => Mark::of(&std::fs::symlink_metadata(source).expect("source")),
    };
    Some((
        Refused::DestinationExists,
        format!("{} is no longer the link made there", dest.display()),
        Some(Unproven {
            doubt: Doubt::OtherFile { linked },
            after,
        }),
    ))
}

/// A fault the archive-output rename lane is made to hit.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum RenameFault {
    #[default]
    None,
    /// The rename is refused and does not happen.
    Refused,
    /// The rename is refused after another writer filled the reservation.
    RefusedAfterReservationFilled,
    /// The rename happens but reports failure, as a network filesystem can
    /// when its reply is lost and the retried request finds no source.
    ReportedAfterTakingEffect,
    /// The rename happens, then the renamed file is moved aside and the
    /// destination path comes to name an unrelated file before it is checked.
    DestinationReplacedAfterRename,
    DestinationReplacedWithSameSizeAfterRename,
}

#[cfg(test)]
thread_local! {
    static HARD_LINK_ATTEMPTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Hard links the source to `dest` and has the disk service prove the new
/// name against `seen`, the source as this import last read it. A link that
/// does not prove is removed by the service before it answers; one it could
/// not remove is an error, as is a new name that cannot be read or is no
/// regular file. A name the service left alone because another writer's
/// regular file holds it answers as an occupied destination.
#[cfg_attr(not(test), allow(unused_variables))]
fn hard_link_import_source(
    source: &Path,
    dest: &Path,
    fingerprint: &ImportSourceFingerprint,
    seen: &Mark,
    options: &ImportFileOptions,
) -> AppResult<ImportLink> {
    #[cfg(test)]
    HARD_LINK_ATTEMPTS.with(|attempts| attempts.set(attempts.get() + 1));
    #[cfg(test)]
    let forced = Mark::from_stored("unknown-volume").expect("the unknown lane is readable");
    #[cfg(test)]
    let seen = match options.force_hard_link_identity_mismatch {
        true => &forced,
        false => seen,
    };
    #[cfg(test)]
    let forced = forced_link_answer(options.link_answer, source, dest);
    #[cfg(not(test))]
    let forced = None;
    let (refused, words, unproven) = match forced {
        Some(answer) => answer,
        None => {
            let policy = disk::placement::proven_link();
            let proof = LinkProof::NameAndLength;
            match Disk::process().place_proven_blocking(source, dest, policy, seen, proof) {
                Ok(Placed {
                    done: Done::HardLinked,
                    ..
                }) => return Ok(ImportLink::Proven),
                Ok(_) => {
                    return Err(AppError::Repository(format!(
                        "the link to {} was not made as a hard link",
                        dest.display()
                    )));
                }
                Err(refusal) => {
                    let words = refusal.to_string();
                    (refusal.refused, words, refusal.unproven)
                }
            }
        }
    };
    let unread = |error: io::Error| {
        let said = format!(
            "import path not found or inaccessible: {}: {error}",
            dest.display()
        );
        import_io_error(&error, said)
    };
    let Some(Unproven { doubt, after, .. }) = unproven else {
        return Ok(match refused {
            Refused::CannotLink(_) => ImportLink::NotPossible,
            _ => ImportLink::Failed(words),
        });
    };
    match (doubt, after) {
        // The read that failed names the reason, whatever became of the name.
        (Doubt::Unread(error), _) => Err(unread(error.into())),
        (_, LinkAfter::NotRemoved) => Err(link_not_removed(dest, &words)),
        (_, LinkAfter::AlreadyGone) => Ok(ImportLink::UnprovenGone(words)),
        (Doubt::OtherFile { linked }, _) if linked.kind() != Some(EntryKind::File) => {
            // A source swapped for something else before the link call is a
            // source that changed. The name is left as it is either way.
            ensure_same_source(source, fingerprint)?;
            Err(AppError::Repository(format!(
                "import path is not a regular file: {}",
                dest.display()
            )))
        }
        // Another regular file took the name after the link was made, and
        // the service left it alone. The name is occupied, and answers as an
        // occupied name does here; nothing is copied over it.
        (Doubt::OtherFile { .. }, LinkAfter::LeftAlone) => {
            ensure_same_source(source, fingerprint)?;
            Ok(ImportLink::Occupied)
        }
        (Doubt::OtherFile { linked }, _) => Ok(ImportLink::Unproven(LinkDoubt::Identity(
            HardLinkIdentityMismatch {
                source: seen.to_stored(),
                linked: linked.to_stored(),
            },
        ))),
        (Doubt::OtherLength(resolved), _) => Ok(ImportLink::Unproven(LinkDoubt::Length(resolved))),
    }
}

/// An unproven link the disk service did not remove, as an import says it.
fn link_not_removed(dest: &Path, words: &str) -> AppError {
    AppError::Repository(format!(
        "failed to remove partial hardlink destination {} before copy: {words}",
        dest.display()
    ))
}

/// Renames over the empty reservation the rename lane made at `dest`.
fn rename_into_reservation(source: &Path, dest: &Path) -> io::Result<()> {
    Disk::process().move_replacing_blocking(source, dest)?;
    Ok(())
}

#[cfg(test)]
fn rename_archive_output(
    source: &Path,
    dest: &Path,
    options: &ImportFileOptions,
) -> io::Result<()> {
    let refused = || io::Error::new(io::ErrorKind::CrossesDevices, "forced rename failure");
    match options.rename_fault {
        RenameFault::Refused => Err(refused()),
        RenameFault::RefusedAfterReservationFilled => {
            std::fs::write(dest, b"another writer")?;
            Err(refused())
        }
        RenameFault::ReportedAfterTakingEffect => {
            std::fs::rename(source, dest)?;
            Err(io::Error::new(
                io::ErrorKind::NotFound,
                "forced failure reported for a rename that took effect",
            ))
        }
        RenameFault::DestinationReplacedAfterRename => {
            std::fs::rename(source, dest)?;
            std::fs::rename(dest, dest.with_extension("moved-aside"))?;
            std::fs::write(dest, b"unrelated file")
        }
        RenameFault::DestinationReplacedWithSameSizeAfterRename => {
            let size = std::fs::metadata(source)?.len() as usize;
            std::fs::rename(source, dest)?;
            std::fs::rename(dest, dest.with_extension("moved-aside"))?;
            std::fs::write(dest, vec![b'x'; size])
        }
        RenameFault::None => rename_into_reservation(source, dest),
    }
}

#[cfg(not(test))]
fn rename_archive_output(source: &Path, dest: &Path, _: &ImportFileOptions) -> io::Result<()> {
    rename_into_reservation(source, dest)
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
fn verified_copier_for(options: &ImportFileOptions) -> Disk {
    if options.force_read_back_unsupported {
        return Disk::process().with_read_back_opener(Arc::new(|_: &Path| {
            scryer_application::location::verify::ReadBackHandle::Unsupported(
                "forced unsupported read-back".to_string(),
            )
        }));
    }
    Disk::process()
}

#[cfg(not(test))]
fn verified_copier_for(_: &ImportFileOptions) -> Disk {
    Disk::process()
}

/// Corrupt the destination between write and verification.
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

#[cfg(all(test, unix))]
fn force_foreign_source_uid(options: &ImportFileOptions) -> bool {
    options.force_foreign_source_uid
}

#[cfg(all(not(test), unix))]
fn force_foreign_source_uid(_: &ImportFileOptions) -> bool {
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
// disk-guard: the owning uid is not part of a reading.
#[allow(clippy::disallowed_methods)]
fn log_imported_destination_ownership(path: &Path, _options: &ImportFileOptions) {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) => {
            tracing::debug!(dest = %path.display(), %error, "could not inspect import destination ownership for diagnostics");
            return;
        }
    };
    let destination_uid = metadata.uid();
    #[cfg(test)]
    let destination_uid = _options.destination_uid_override.unwrap_or(destination_uid);
    let process_uid = effective_uid();
    if destination_uid != process_uid {
        tracing::debug!(
            dest = %path.display(),
            destination_uid,
            process_uid,
            "import destination ownership differs from process uid; ownership is diagnostic only"
        );
    }
}

#[cfg(not(unix))]
fn log_imported_destination_ownership(_path: &Path, _options: &ImportFileOptions) {}

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
// disk-guard: the owner has no verb that sets an operator-chosen mode.
#[allow(clippy::disallowed_methods)]
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
// disk-guard: the owner has no verb that changes a file's group.
#[allow(clippy::disallowed_methods)]
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
    let disk = Disk::process();
    let mut missing = Vec::new();
    let mut current = Some(parent);
    while let Some(path) = current {
        if path.as_os_str().is_empty() || disk.follow_blocking(path).exists() == Some(true) {
            break;
        }
        missing.push(path.to_path_buf());
        current = path.parent();
    }
    missing.reverse();
    missing
}

/// [`missing_destination_dirs`] for a caller on the runtime: the same look
/// at each folder, each one a bounded read off the runtime. A look that does
/// not answer leaves the list empty, so no folder gets permissions on a
/// guess.
pub async fn missing_destination_dirs_off_runtime(parent: &Path) -> Vec<PathBuf> {
    let disk = Disk::process();
    let mut missing = Vec::new();
    let mut current = Some(parent);
    while let Some(path) = current {
        if path.as_os_str().is_empty() {
            break;
        }
        let reading = disk.follow(path).await;
        if reading.exists() == Some(true) {
            break;
        }
        if matches!(
            reading.unreachable_cause(),
            Some(disk::reason::DISK_TIMED_OUT | disk::reason::DISK_PATH_UNREACHABLE)
        ) {
            tracing::warn!(
                parent = %parent.display(),
                folder = %path.display(),
                state = reading.describe_absence(),
                "could not tell which folders this creates; none gets the configured permissions"
            );
            return Vec::new();
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
    prepare_marked_import_destination(source, dest, permissions)
        .map(|(fingerprint, _, size, guard)| (fingerprint, size, guard))
}

/// [`prepare_import_destination`], handing back the mark of the source's
/// name from the read that fingerprinted it.
fn prepare_marked_import_destination(
    source: &Path,
    dest: &Path,
    permissions: &ImportFilePermissions,
) -> AppResult<(ImportSourceFingerprint, Mark, u64, ImportDestinationGuard)> {
    let (source_fingerprint, source_mark) = read_import_source(source)?;
    let size = source_fingerprint.file.len;
    if size == 0 {
        return Err(AppError::Repository(format!(
            "import source is zero bytes: {}",
            source.display()
        )));
    }

    let parent = destination_parent_for_guard(dest);
    let missing_dirs = missing_destination_dirs(parent);
    let disk = Disk::process();
    let created = disk.create_dir_blocking(parent, Parents::Create);
    created.map_err(io::Error::from).map_err(|e| {
        import_io_error(
            &e,
            format!(
                "failed to create destination directory {}: {}",
                parent.display(),
                e
            ),
        )
    })?;
    let canonical = disk.canonicalize_blocking(parent).map_err(io::Error::from);
    let approved_parent_canonical = canonical.map_err(|e| {
        import_io_error(
            &e,
            format!(
                "failed to inspect destination directory {}: {}",
                parent.display(),
                e
            ),
        )
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

    Ok((source_fingerprint, source_mark, size, destination_guard))
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

    let canonical = Disk::process().canonicalize_blocking(&guard.parent_path);
    let current_parent_canonical = canonical.map_err(io::Error::from).map_err(|e| {
        import_io_error(
            &e,
            format!(
                "failed to re-check destination directory {}: {}",
                guard.parent_path.display(),
                e
            ),
        )
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
    let reading = Disk::process().uncached_blocking(dest);
    match reading.exists() {
        Some(false) => Ok(()),
        Some(true) => Err(import_destination_occupied(dest)),
        None => Err(AppError::Repository(format!(
            "failed to inspect import destination {} before copy: {}",
            dest.display(),
            reading.describe_absence()
        ))),
    }
}

/// Something other than this import holds the destination name, so nothing
/// is copied to it.
fn import_destination_occupied(dest: &Path) -> AppError {
    AppError::RepositoryFailed {
        message: format!(
            "import destination appeared before copy execution: {}",
            dest.display()
        ),
        reason: scryer_application::error_reasons::import::DESTINATION_OCCUPIED,
    }
}

/// The existing destination is the source's own entry: a move that removed
/// the source would remove the only copy, so it is refused for manual
/// reconciliation.
fn import_destination_is_source(source: &Path, dest: &Path) -> AppError {
    AppError::ManualReconciliationRefused {
        message: format!(
            "import destination {} is the source {} itself, so a move would remove the only copy; nothing was removed",
            dest.display(),
            source.display()
        ),
        reason: scryer_application::error_reasons::import::DESTINATION_IS_SOURCE,
    }
}

/// Whether the existing destination is the source's own entry could not be
/// told; `why` says which read or rename failed.
fn import_destination_alias_unknown(source: &Path, dest: &Path, why: &str) -> AppError {
    AppError::RepositoryFailed {
        message: format!(
            "could not tell whether import destination {} names the source {} itself: {why}; nothing was removed",
            dest.display(),
            source.display()
        ),
        reason: scryer_application::error_reasons::import::DESTINATION_ALIAS_UNKNOWN,
    }
}

// disk-guard: a failure's own error kind names the import reason, which a
// reading does not carry.
#[allow(clippy::disallowed_methods)]
fn validate_import_destination_file(guard: &ImportDestinationGuard) -> AppResult<()> {
    let metadata = std::fs::symlink_metadata(&guard.requested_path).map_err(|e| {
        import_io_error(
            &e,
            format!(
                "failed to inspect imported destination {}: {}",
                guard.requested_path.display(),
                e
            ),
        )
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
    source: &Path,
    returned_dest: &Path,
    landed: Landed<'_>,
) -> AppResult<()> {
    validate_import_destination_parent(guard, returned_dest)?;
    validate_import_destination_file(guard)
        .map_err(|error| took_back(error, source, returned_dest, landed))
}

// disk-guard: a failure's own error kind names the import reason, which the
// owner's link and reading verbs do not keep for a link it cannot make.
#[allow(clippy::disallowed_methods)]
fn import_symlink_source(
    source: &Path,
    dest: &Path,
    source_fingerprint: &ImportSourceFingerprint,
    size: u64,
) -> AppResult<Mark> {
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
            import_io_error(
                &e,
                format!(
                    "failed to create symlink import {} -> {}: {}",
                    dest.display(),
                    symlink_target.display(),
                    e
                ),
            )
        })?;
        let dest_meta = std::fs::symlink_metadata(dest).map_err(|e| {
            import_io_error(
                &e,
                format!(
                    "failed to inspect imported symlink {}: {}",
                    dest.display(),
                    e
                ),
            )
        })?;
        if !dest_meta.file_type().is_symlink() {
            // Something other than the link this import made holds the name;
            // it is not this import's to remove.
            return Err(AppError::Repository(format!(
                "import destination is not a symlink: {}",
                dest.display()
            )));
        }
        // The link this import made, from the read just taken.
        let made = Mark::of(&dest_meta);
        ensure_same_source(source, source_fingerprint)?;
        let dest_target_meta = std::fs::metadata(dest).map_err(|e| {
            took_back(
                AppError::Repository(format!(
                    "imported symlink target is unavailable: {}: {}",
                    dest.display(),
                    e
                )),
                source,
                dest,
                Landed::Link(&made),
            )
        })?;
        if dest_target_meta.len() != size {
            return Err(took_back(
                AppError::Repository(format!(
                    "symlink import size mismatch: source={} dest={}",
                    size,
                    dest_target_meta.len()
                )),
                source,
                dest,
                Landed::Link(&made),
            ));
        }

        Ok(made)
    }
}

/// Removes a file whose presence this import has just proved, where a file
/// already gone is a failure as well: something else changed the path.
fn remove_placed_file(path: &Path) -> io::Result<()> {
    match Disk::process().remove_blocking(path, Remove::File)? {
        Removal::Removed => Ok(()),
        Removal::AlreadyAbsent => Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} is already gone", path.display()),
        )),
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

/// A staged copy the disk service refused, as an import reports it. A source
/// whose name no longer reads as the file is read once more, the import's own
/// way against `fingerprint`, for what it is now.
fn import_copy_refusal(
    source: &Path,
    dest: &Path,
    temp_dest: &Path,
    fingerprint: &ImportSourceFingerprint,
    refusal: StagedCopyRefusal,
) -> AppError {
    let changed = || {
        let now = ensure_same_source(source, fingerprint);
        io_other(now.err().unwrap_or_else(|| {
            AppError::Repository(format!(
                "import source changed during import: {}",
                source.display()
            ))
        }))
    };
    let (source, dest) = (source.display(), dest.display());
    let attempts = refusal.attempts;
    let unread = |error: io::Error| {
        let said = format!("import path not found or inaccessible: {source}: {error}");
        io_other(import_io_error(&error, said))
    };
    let (stage, error) = match (refusal.step, refusal.cause) {
        (_, StagedCause::Cancelled) => {
            return AppError::canceled(format!(
                "filesystem copy was cancelled: {source} -> {dest}"
            ));
        }
        (StagedStep::ClearStaged, StagedCause::Io(error)) => {
            return import_io_error(
                &error,
                format!(
                    "import copy failed before attempt {attempts}: {source} -> {dest}: cleanup of temporary destination {} failed: {error}",
                    temp_dest.display()
                ),
            );
        }
        (StagedStep::SourceBefore, StagedCause::Io(error)) => ("source preflight", unread(error)),
        (StagedStep::SourceBefore, _) => ("source preflight", changed()),
        (StagedStep::SourceAfter, StagedCause::Io(error)) => ("source verification", unread(error)),
        (StagedStep::SourceAfter, _) => ("source verification", changed()),
        (StagedStep::SourceHandle, StagedCause::Io(error)) => ("source metadata", error),
        (_, StagedCause::SourceChanged) => (
            "source validation",
            io_other("import source changed before copy"),
        ),
        (_, StagedCause::SourceNotAFile) => (
            "source validation",
            io_other(AppError::Repository(
                "import path is not a regular file".into(),
            )),
        ),
        (StagedStep::OpenSource, StagedCause::Io(error)) => ("source open", error),
        (StagedStep::OpenStaged, StagedCause::Io(error)) => ("temp create", error),
        (StagedStep::Flush, StagedCause::Io(error)) => ("flush", error),
        (StagedStep::Sync, StagedCause::Io(error)) => ("sync", error),
        (StagedStep::Promote, StagedCause::Io(error)) => ("final rename", error),
        (StagedStep::Transfer, StagedCause::Io(error)) => ("copy", error),
    };
    import_io_error(
        &error,
        format!(
            "import copy failed after {attempts} attempt(s): {source} -> {dest}: {stage}: {error}"
        ),
    )
}

/// Copies one regular file through the disk service and proves the
/// destination at `depth`.
///
/// Returns the streamed hashes and what verification concluded. A destination
/// that cannot be proven is removed and the import fails. An import that
/// errors here never reaches source cleanup, so a corrupt copy can never cost
/// the user the original.
#[allow(clippy::too_many_arguments)]
fn copy_regular_source_to_destination(
    source: &Path,
    dest: &Path,
    source_fingerprint: &ImportSourceFingerprint,
    source_mark: &Mark,
    size: u64,
    options: ImportFileOptions,
    depth: VerificationDepth,
    progress: Option<&ImportFileTransferProgressSender>,
    cancellation: Option<&scryer_application::ImportCancellation>,
) -> AppResult<(ImportVerification, Option<Mark>)> {
    let temp_dest = scryer_application::import_staging_path(dest);
    let sender = progress.cloned();
    let watch = Watch {
        // The finalizing phase covers the flush, the sync and the rename.
        progress: Progress::to(move |phase, bytes| {
            let phase = match phase {
                Phase::Copying => ImportTransferPhase::Copying,
                Phase::Finishing => ImportTransferPhase::Finalizing,
                _ => return,
            };
            report_import_transfer_progress(sender.as_ref(), phase, bytes, size);
        }),
        cancel: match cancellation.cloned() {
            Some(cancellation) => Cancel::when(move || cancellation.is_cancelled()),
            None => Cancel::never(),
        },
    };
    // The import's own landing, every part of it stated: the destination's
    // name with `.tmp_import` added is this import's and is cleared, the folder is not fsynced, the
    // source's permissions are not carried, and the source is checked by
    // name before and after and through the handle the copy reads.
    let landing = StagedCopy {
        staged: &temp_dest,
        seen: source_mark,
        sync_folder: false,
        carry_permissions: false,
        bad_handle_pauses: &IMPORT_COPY_RETRY_PAUSES,
        chunk_bytes: IMPORT_COPY_BUFFER_BYTES,
    };
    let copied = Disk::process()
        .copy_staged_blocking(source, dest, &landing, &watch)
        .map_err(|refusal| {
            import_copy_refusal(source, dest, &temp_dest, source_fingerprint, refusal)
        })?;
    if copied.sync == disk::SyncOutcome::Unconfirmed {
        // Verification below, not the sync, guards the source before cleanup.
        tracing::debug!(
            dest = %dest.display(),
            "import destination filesystem does not support fsync; relying on copy verification"
        );
    }
    let hashes = persisted_hashes(copied.hashes);

    if force_copy_verification_failure(&options) {
        let _ = Disk::process().remove_blocking(dest, Remove::File);
        return Err(AppError::Repository(format!(
            "copy verification failed for test: {}",
            dest.display()
        )));
    }

    // The copy has taken the destination name, which was free before it. What
    // is under the name is read first, and a check that fails after this
    // removes it only while it is still the file read here, as the size check
    // below does. A name that does not read is left as it is.
    // The file the copy promoted, read through its own handle before it
    // took the name: a later read of the name may already be another file.
    // Where that mark tells no file apart from another, only its length and
    // times can, and a platform that sets the write time at the close would
    // make the import's own copy read as rewritten; there the read of the
    // name made next, right after the promotion, marks it instead, but only
    // when that read is a regular file of the copied length: anything else
    // under the name is not the copy, and the handle's mark is kept.
    let promoted = copied.promoted;
    #[cfg(test)]
    if let Some(after_promoted) = options.after_copy_promoted {
        after_promoted(source, dest);
    }
    let (dest_fingerprint, landed) = match read_import_source(dest) {
        Ok((fingerprint, named)) => {
            let landed = match promoted {
                Some(mark)
                    if mark.same_file(&mark) == Sameness::CannotTell
                        && named.kind() == Some(EntryKind::File)
                        && named.is_len(size) =>
                {
                    Some(named)
                }
                promoted => promoted,
            };
            (fingerprint.file, landed)
        }
        Err(error) => {
            return Err(took_back(
                error,
                source,
                dest,
                Landed::Copy {
                    placed: promoted.as_ref(),
                    source: source_mark,
                },
            ));
        }
    };
    if let Err(error) = ensure_same_source(source, source_fingerprint) {
        return Err(took_back(
            error,
            source,
            dest,
            Landed::Copy {
                placed: landed.as_ref(),
                source: source_mark,
            },
        ));
    }

    if dest_fingerprint.len != size {
        return Err(took_back(
            AppError::Repository(format!(
                "copy size mismatch: source={} dest={}",
                size, dest_fingerprint.len
            )),
            source,
            dest,
            Landed::Copy {
                placed: landed.as_ref(),
                source: source_mark,
            },
        ));
    }

    force_destination_corruption(&options, dest);

    // Already on a blocking thread, so the blocking twin is the right one.
    // `Full` degrades to the quick floor by itself when the destination cannot
    // be read back, which is why a filesystem that refuses a
    // cache-bypassed re-read still imports rather than failing.
    let sender = progress.cloned();
    let last_report = std::sync::Mutex::new(None::<Instant>);
    let verification_progress = scryer_application::location::verify::CopyProgress::none()
        .with_transfer_sink(move |phase, bytes| {
            let mut last = last_report
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if bytes == 0
                || bytes >= size
                || last.is_none_or(|at| at.elapsed() >= Duration::from_secs(1))
            {
                report_import_transfer_progress(sender.as_ref(), phase, bytes, size);
                *last = Some(Instant::now());
            }
        });
    let assessment = scryer_application::location::verify::verify_blocking(
        &verified_copier_for(&options),
        source,
        dest,
        &hashes,
        depth,
        &verification_progress,
    );
    report_import_transfer_progress(progress, ImportTransferPhase::Finalizing, size, size);
    #[cfg(test)]
    if let Some(after_assessed) = options.after_copy_assessed {
        after_assessed(source, dest);
    }
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
        return Err(took_back(
            AppError::Repository(format!(
                "copy verification failed at depth {} for {}: {} ({detail})",
                verification.depth.label(),
                dest.display(),
                verification.outcome.as_str(),
            )),
            source,
            dest,
            Landed::Copy {
                placed: landed.as_ref(),
                source: source_mark,
            },
        ));
    }

    #[cfg(test)]
    if let Some(after_verified) = options.after_copy_verified {
        after_verified(source, dest);
    }
    Ok((verification, landed))
}

/// What an import placed at its destination, and the mark it is known by.
#[derive(Clone, Copy)]
enum Landed<'a> {
    /// The staged file a copy promoted, `None` when the copy kept no mark,
    /// and the source as the copy read it.
    Copy {
        placed: Option<&'a Mark>,
        source: &'a Mark,
    },
    /// A second name of the source's own file, marked as the source was
    /// read before the link.
    HardLink(&'a Mark),
    /// The link this import made, as it was read once made.
    Link(&'a Mark),
}

impl<'a> Landed<'a> {
    fn mark(self) -> Option<&'a Mark> {
        match self {
            Self::Copy { placed, .. } => placed,
            Self::HardLink(mark) | Self::Link(mark) => Some(mark),
        }
    }
}

/// What a take-back did with the destination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TakeBack {
    Removed,
    /// The name holds something else, does not read, or has no mark; or a
    /// hard link's source did not read.
    Kept,
    /// The source name is gone, or holds another file than the one the
    /// import placed from, so the destination may be the last copy.
    KeptForSource,
    /// A copy whose source did not read: nothing says the source still
    /// holds the data, so the copy may be the last one.
    KeptSourceUnread,
}

/// Removes what an import placed at `dest` after a check on it failed.
///
/// The source is read first. While its name is gone, or holds another file
/// than the one a copy read or a hard link shares, the destination may be
/// the last copy of the data and is kept. A copy is removed only while the
/// source reads and is still the file the copy was read from, so a copy
/// whose source does not read is kept at its final name too. A hard link
/// whose source does not read is kept, since it is the source's own file; a
/// link, which holds no data, is taken back.
///
/// Then the destination is removed only while its name still holds what
/// `landed` marks. Anything else there, a name that does not read, or no
/// mark, is kept. At most two reads, both on this failure path.
fn take_back_placed(source: &Path, dest: &Path, landed: Landed<'_>) -> TakeBack {
    let Some(placed) = landed.mark() else {
        tracing::warn!(
            dest = %dest.display(),
            "what the import placed carries no mark to tell it by; it is kept"
        );
        return TakeBack::Kept;
    };
    // What the source name must still hold for the destination to be only a
    // copy; a link import placed no data of its own.
    let held = match landed {
        Landed::Copy { source, .. } => Some(source),
        Landed::HardLink(mark) => Some(mark),
        Landed::Link(_) => None,
    };
    let source_gone = || {
        tracing::warn!(
            source = %source.display(),
            dest = %dest.display(),
            "the import source no longer holds what the import placed, so the destination may be its only copy; it is kept"
        );
        TakeBack::KeptForSource
    };
    match Disk::process().fingerprint_blocking(source) {
        Ok(now) if held.is_some_and(|held| !held.still_marks(&now)) => return source_gone(),
        Ok(_) => {}
        Err(PathReading::Missing { .. }) => return source_gone(),
        Err(_) if matches!(landed, Landed::HardLink(_)) => {
            tracing::warn!(
                source = %source.display(),
                dest = %dest.display(),
                "the import source did not read, so the hard link may be its only name; it is kept"
            );
            return TakeBack::Kept;
        }
        Err(_) if matches!(landed, Landed::Copy { .. }) => {
            tracing::warn!(
                source = %source.display(),
                dest = %dest.display(),
                "the import source did not read, so the copy may be the only one; it is kept"
            );
            return TakeBack::KeptSourceUnread;
        }
        Err(_) => {}
    }
    let still_placed = Disk::process()
        .fingerprint_blocking(dest)
        .is_ok_and(|now| placed.still_marks(&now));
    if still_placed {
        let _ = Disk::process().remove_blocking(dest, Remove::File);
        TakeBack::Removed
    } else {
        tracing::warn!(
            dest = %dest.display(),
            "what the import placed is no longer what holds the destination name, or the name does not read; it is kept"
        );
        TakeBack::Kept
    }
}

/// [`take_back_placed`] after `error`, which says so when the destination
/// is kept because the source no longer holds the data or did not read. The
/// failure's routing fact is kept.
fn took_back(error: AppError, source: &Path, dest: &Path, landed: Landed<'_>) -> AppError {
    let taken_back = take_back_placed(source, dest, landed);
    let why = match taken_back {
        TakeBack::KeptForSource => "the source no longer holds this file",
        TakeBack::KeptSourceUnread => "the source did not read",
        TakeBack::Removed | TakeBack::Kept => return error,
    };
    if !matches!(
        error,
        AppError::Repository(_)
            | AppError::RepositoryFailed { .. }
            | AppError::RepositoryFailedWithFacts { .. }
    ) {
        return error;
    }
    let message = format!(
        "{error}; {why}, so {} is kept as what may be its only copy",
        dest.display()
    );
    match (taken_back, landed) {
        (TakeBack::KeptForSource, Landed::Copy { .. }) => AppError::RepositoryFailedWithFacts {
            message,
            reason: scryer_application::error_reasons::import::COPY_KEPT_SOURCE_GONE,
            facts: vec![RefusalFact {
                key: scryer_application::error_reasons::import::KEPT_PATH_FACT,
                value: RefusalFactValue::Text(dest.display().to_string()),
            }],
        },
        _ => import_error_context(&error, message),
    }
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

    if let Some(stored) = &guard.adopted_destination_mark {
        return remove_source_beside_adopted_destination(
            &guard.source_path,
            &final_dest_path,
            stored,
            &options,
        );
    }

    let remove_result = if force_delete_failure(&options) {
        Err(io_other("forced source delete failure for test"))
    } else {
        remove_placed_file(&guard.source_path)
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

/// Removes a move's source beside a destination that was already there and
/// was adopted, once the filesystem shows the two are separate entries. See
/// [`Disk::remove_source_beside_blocking`]. Anything but a removal keeps the
/// source's data and refuses.
fn remove_source_beside_adopted_destination(
    source: &Path,
    dest: &Path,
    stored: &str,
    options: &ImportFileOptions,
) -> AppResult<()> {
    let Some(adopted) = Mark::from_stored(stored) else {
        return Err(import_destination_alias_unknown(
            source,
            dest,
            "its adopted mark does not read",
        ));
    };
    if force_delete_failure(options) {
        return Err(AppError::Repository(format!(
            "import source cleanup failed after destination verification; failed to remove source {}: forced source delete failure for test",
            source.display()
        )));
    }
    match Disk::process().remove_source_beside_blocking(source, dest, &adopted) {
        AsideRemoval::Removed => Ok(()),
        AsideRemoval::SameEntry => Err(import_destination_is_source(source, dest)),
        AsideRemoval::DestinationChanged => Err(AppError::Repository(format!(
            "import source cleanup failed because destination changed: {}; nothing was removed",
            dest.display()
        ))),
        AsideRemoval::DestinationUnread(reading) => Err(import_destination_alias_unknown(
            source,
            dest,
            &reading.describe_absence().to_string(),
        )),
        AsideRemoval::NotSetAside(error) => Err(import_destination_alias_unknown(
            source,
            dest,
            &error.to_string(),
        )),
        AsideRemoval::NotRemoved(error) => Err(AppError::Repository(format!(
            "import source cleanup failed after destination verification; failed to remove source {}: {error}",
            source.display()
        ))),
        AsideRemoval::PutBackFailed { at, error } => Err(import_destination_alias_unknown(
            source,
            dest,
            &format!(
                "the source was set aside at {} and could not be put back: {error}",
                at.display()
            ),
        )),
    }
}

fn finalize_imported_regular_destination(
    dest: &Path,
    options: &ImportFileOptions,
    permissions: &ImportFilePermissions,
) {
    log_imported_destination_ownership(dest, options);
    apply_file_permissions_best_effort(dest, permissions);
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PreparedImportPlacement {
    source: PathBuf,
    dest: PathBuf,
    #[serde(skip)]
    options: ImportFileOptions,
    /// The operator's verification depth for this import.
    /// Serialized, unlike `options`: the worker process performs the copy, so
    /// the depth has to survive the handoff. Required on the wire: every peer
    /// that speaks this protocol version writes it, and a missing depth is a
    /// malformed request rather than an invitation to guess one.
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
    /// The source is output the archive extractor wrote into its own
    /// workspace, which is Scryer's scratch and may be placed by rename.
    #[serde(default)]
    archive_workspace_source: bool,
    /// A hard link to this destination volume already proved unverifiable in
    /// this process, so the fast lane hands straight to the copy lane.
    #[serde(default)]
    hard_link_identity_unverifiable: bool,
}

enum FastPlacementResult {
    Finished(ImportFileResult),
    CopyRequired {
        prepared: PreparedImportPlacement,
        hard_link_identity_mismatch: Option<HardLinkIdentityMismatch>,
    },
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
        archive_workspace_source: false,
        hard_link_identity_unverifiable: false,
    })
}

/// The destination volume as the disk service names it, from the key the
/// prepared placement already carries: no probe. A key the service did not
/// write shares the unknown lane.
fn copy_lane(volume_key: &str) -> Mark {
    Mark::from_stored(volume_key).unwrap_or_else(|| {
        tracing::warn!(
            volume_key = %opaque_lane_key(volume_key),
            "an import copy's volume key was not read; it shares the unknown lane"
        );
        Mark::from_stored("unknown-volume").expect("the unknown lane is always readable")
    })
}

/// The key the disk service writes for the volume the approved destination
/// folder resolved to, from the number its fingerprint already holds.
#[cfg_attr(not(any(unix, windows)), allow(unused_variables))]
fn destination_volume_key(guard: &ImportDestinationGuard) -> String {
    #[cfg(unix)]
    let number = guard.approved_parent_fingerprint.dev;
    #[cfg(windows)]
    let number = u64::from(guard.approved_parent_fingerprint.volume_serial_number);
    #[cfg(not(any(unix, windows)))]
    let number = 0;
    Mark::on_volume(number).to_stored()
}

/// `landed` is what this import placed at the destination; see
/// [`take_back_placed`].
fn finish_prepared_import(
    prepared: PreparedImportPlacement,
    strategy: ImportStrategy,
    landed: Landed<'_>,
) -> AppResult<ImportFileResult> {
    let source_cleanup = cleanup_guard_after_placement(
        prepared.source_cleanup_required,
        &prepared.source,
        &prepared.dest,
        &prepared.source_fingerprint,
        prepared.size,
        landed,
    )?;
    Ok(ImportFileResult {
        strategy,
        source_path: prepared.source,
        dest_path: prepared.dest,
        size_bytes: prepared.size,
        destination_disposition: ImportDestinationDisposition::Created,
        source_disposition: ImportSourceDisposition::Retained,
        source_cleanup,
        // A rename, hardlink, or symlink placement copies no bytes:
        // there is nothing to verify and nothing to hash.
        verification: None,
    })
}

/// Archive output renamed into place leaves nothing at its source path, so a
/// move import has no source left to clean up and builds no guard.
fn finish_renamed_archive_output(prepared: PreparedImportPlacement) -> ImportFileResult {
    ImportFileResult {
        strategy: ImportStrategy::Move,
        source_path: prepared.source,
        dest_path: prepared.dest,
        size_bytes: prepared.size,
        destination_disposition: ImportDestinationDisposition::Created,
        source_disposition: ImportSourceDisposition::RenamedIntoPlace,
        source_cleanup: None,
        verification: None,
    }
}

fn try_fast_import_placement_blocking(
    prepared: PreparedImportPlacement,
) -> AppResult<FastPlacementResult> {
    let source_mark = same_source_mark(&prepared.source, &prepared.source_fingerprint)?;
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
        let made = import_symlink_source(
            &prepared.source,
            &prepared.dest,
            &prepared.source_fingerprint,
            prepared.size,
        )?;
        validate_import_destination_guard_after_placement(
            &prepared.destination_guard,
            &prepared.source,
            &prepared.dest,
            Landed::Link(&made),
        )?;
        return finish_prepared_import(prepared, ImportStrategy::Symlink, Landed::Link(&made))
            .map(FastPlacementResult::Finished);
    }

    let hardlink_uid_safe =
        source_uid_matches_scryer(&prepared.source_fingerprint, &prepared.options);
    if prepared.archive_workspace_source
        && hardlink_uid_safe
        && rename_archive_workspace_output(&prepared)?
    {
        return Ok(FastPlacementResult::Finished(
            finish_renamed_archive_output(prepared),
        ));
    }

    let mut hard_link_identity_mismatch = None;
    if prepared.hard_link_identity_unverifiable {
        tracing::debug!(
            volume_key = %opaque_lane_key(&prepared.volume_key),
            "hard link identity is unverifiable on this volume; handing import to copy lane"
        );
    } else if !force_cross_device_move(&prepared.options) && hardlink_uid_safe {
        let linked = hard_link_import_source(
            &prepared.source,
            &prepared.dest,
            &prepared.source_fingerprint,
            &source_mark,
            &prepared.options,
        )?;
        #[cfg(test)]
        if matches!(linked, ImportLink::Proven)
            && let Some(after_linked) = prepared.options.after_hard_link
        {
            after_linked(&prepared.source, &prepared.dest);
        }
        // The source is read again whether or not the link proved. A link
        // that did not prove is already gone.
        let proven = matches!(linked, ImportLink::Proven);
        if matches!(linked, ImportLink::Proven | ImportLink::Unproven(_))
            && let Err(error) = ensure_same_source(&prepared.source, &prepared.source_fingerprint)
        {
            if proven {
                return Err(took_back(
                    error,
                    &prepared.source,
                    &prepared.dest,
                    Landed::HardLink(&source_mark),
                ));
            }
            return Err(error);
        }
        match linked {
            ImportLink::Proven => {
                validate_import_destination_guard_after_placement(
                    &prepared.destination_guard,
                    &prepared.source,
                    &prepared.dest,
                    Landed::HardLink(&source_mark),
                )?;
                finalize_imported_regular_destination(
                    &prepared.dest,
                    &prepared.options,
                    &prepared.permissions,
                );
                return finish_prepared_import(
                    prepared,
                    ImportStrategy::HardLink,
                    Landed::HardLink(&source_mark),
                )
                .map(FastPlacementResult::Finished);
            }
            ImportLink::Unproven(LinkDoubt::Identity(mismatch)) => {
                hard_link_identity_mismatch = Some(mismatch);
            }
            ImportLink::Unproven(LinkDoubt::Length(linked)) => match linked.size() {
                Some(destination_size) => tracing::warn!(
                    source_size = prepared.size,
                    destination_size,
                    "hard link size mismatch; handing import to copy lane"
                ),
                None => tracing::warn!(
                    error = linked.describe_absence(),
                    "hard link destination stat failed; handing import to copy lane"
                ),
            },
            // Nothing is read or written for this one before it is refused.
            ImportLink::UnprovenGone(words) => {
                return Err(link_not_removed(&prepared.dest, &words));
            }
            // The service already read the name holding another file.
            ImportLink::Occupied => {
                return Err(import_destination_occupied(&prepared.dest));
            }
            ImportLink::NotPossible => {
                tracing::debug!("hard link is ineligible; handing import to copy lane");
            }
            ImportLink::Failed(error) => {
                tracing::warn!(error = %error, "hard link failed; handing import to copy lane");
            }
        }
    }

    ensure_import_destination_absent(&prepared.dest)?;
    Ok(FastPlacementResult::CopyRequired {
        prepared,
        hard_link_identity_mismatch,
    })
}

/// Places a file the archive extractor wrote into its own workspace by renaming
/// it. The workspace is staged under the title folder, so the rename stays on
/// one volume and needs no link identity. `Ok(false)` hands the import to the
/// hard-link and copy lanes unchanged.
///
/// Once the rename has happened the destination holds the only copy of the
/// output, so nothing here removes or moves it again.
fn rename_archive_workspace_output(prepared: &PreparedImportPlacement) -> AppResult<bool> {
    // Capture the same content proof used by the copy/cleanup lanes before
    // rename removes the source path. Inodes may change on path-hashed mounts.
    let source =
        stable_import_source_snapshot(&prepared.source, Some(&prepared.source_fingerprint))?;
    // Reserve the name so a file that appeared since the destination was
    // checked is never replaced by the rename.
    let reservation = match Disk::process().open_blocking(&prepared.dest, Open::create_new()) {
        Ok(file) => file
            .metadata()
            .ok()
            .and_then(|metadata| fingerprint_from_metadata(&metadata).ok()),
        Err(error) => {
            tracing::debug!(
                error = %error,
                "archive workspace rename could not reserve the destination; trying the link and copy lanes"
            );
            return Ok(false);
        }
    };
    if let Err(error) = rename_archive_output(&prepared.source, &prepared.dest, &prepared.options) {
        return release_archive_rename_reservation(prepared, reservation.as_ref(), &error);
    }

    let placed = validate_import_destination_parent(&prepared.destination_guard, &prepared.dest)
        .and_then(|()| {
            let fingerprint = fingerprint_import_source(&prepared.dest)?;
            if !matches!(fingerprint.kind, ImportSourceKind::Regular) {
                return Err(AppError::Repository(format!(
                    "renamed import destination is not a regular file: {}",
                    prepared.dest.display()
                )));
            }
            let destination = stable_import_source_snapshot(&prepared.dest, Some(&fingerprint))?;
            if destination.proof != source.proof {
                return Err(AppError::Repository(format!(
                    "renamed import destination content does not match the source: {}",
                    prepared.dest.display()
                )));
            }
            Ok(())
        });
    let Err(error) = placed else {
        log_imported_destination_ownership(&prepared.dest, &prepared.options);
        apply_file_permissions_best_effort(&prepared.dest, &prepared.permissions);
        return Ok(true);
    };

    // A failed check can mean the destination path no longer names the file
    // that was renamed, and the workspace is removed after the import, so
    // moving whatever is there back into it could cost an unrelated file.
    Err(AppError::ManualReconciliationRequired(format!(
        "archive output was renamed to {} but the placement could not be verified, so it was left in place: {error}",
        prepared.dest.display()
    )))
}

/// Settles a rename that reported failure. The empty reservation is removed
/// only while the source is provably still in place: a network filesystem can
/// report failure for a rename that took effect, and the destination is then
/// the only copy of the output.
// disk-guard: the reservation is matched on its raw device and inode, which a
// reading keeps opaque.
#[allow(clippy::disallowed_methods)]
fn release_archive_rename_reservation(
    prepared: &PreparedImportPlacement,
    reservation: Option<&FileFingerprint>,
    rename_error: &io::Error,
) -> AppResult<bool> {
    if let Err(source_error) = ensure_same_source(&prepared.source, &prepared.source_fingerprint) {
        return Err(AppError::ManualReconciliationRequired(format!(
            "renaming archive output {} to {} reported failure ({rename_error}) but the source did not stay in place ({source_error}); nothing was removed",
            prepared.source.display(),
            prepared.dest.display()
        )));
    }
    match std::fs::symlink_metadata(&prepared.dest) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Ok(metadata) if is_unused_rename_reservation(&metadata, reservation) => {
            if let Err(error) = Disk::process().remove_blocking(&prepared.dest, Remove::File) {
                return Err(AppError::Repository(format!(
                    "failed to release destination reservation {}: {}",
                    prepared.dest.display(),
                    error.words()
                )));
            }
        }
        _ => {
            return Err(AppError::ManualReconciliationRequired(format!(
                "destination {} changed while archive output was being renamed into it; nothing was removed",
                prepared.dest.display()
            )));
        }
    }
    tracing::debug!(
        error = %rename_error,
        "archive workspace rename is ineligible; trying the link and copy lanes"
    );
    Ok(false)
}

/// The destination is still the empty file the rename lane reserved.
fn is_unused_rename_reservation(
    metadata: &std::fs::Metadata,
    reservation: Option<&FileFingerprint>,
) -> bool {
    fingerprint_from_metadata(metadata).is_ok_and(|current| {
        current.len == 0
            && reservation.is_none_or(|reserved| same_file_identity(&current, reserved))
    })
}

fn reconcile_existing_destination(
    source: &Path,
    dest: &Path,
    source_fingerprint: &ImportSourceFingerprint,
    size: u64,
    destination_guard: &ImportDestinationGuard,
    source_cleanup_required: bool,
) -> AppResult<Option<ImportFileResult>> {
    let existing = match Disk::process().fingerprint_blocking(dest) {
        Ok(mark) => mark,
        Err(reading) if reading.exists() == Some(false) => return Ok(None),
        Err(reading) => {
            return Err(AppError::Repository(format!(
                "failed to inspect existing import destination {}: {}",
                dest.display(),
                reading.describe_absence()
            )));
        }
    };
    let Some(kind) = existing.kind() else {
        return Err(AppError::Repository(format!(
            "failed to inspect existing import destination {}: no kind was read",
            dest.display()
        )));
    };
    if source == dest {
        return Err(AppError::ManualReconciliationRequired(format!(
            "import source and destination are the same path: {}",
            dest.display()
        )));
    }
    if !matches!(kind, EntryKind::File | EntryKind::Symlink) {
        return Err(AppError::ManualReconciliationRequired(format!(
            "import destination conflict is not a regular file: {}",
            dest.display()
        )));
    }

    // A destination that names the source's own entry, through a linked or
    // mounted folder, another spelling, or a link that resolves to it, proves
    // equal to the source; a move that then removed the source would leave
    // nothing there. One cheap look refuses the plain cases here. The rest is
    // settled when the source is removed: it is set aside first, and removed
    // only while the destination still holds the file adopted here.
    if source_cleanup_required {
        match Disk::process().entry_alias_blocking(source, dest, &existing) {
            EntryAlias::Distinct => {}
            EntryAlias::Same => return Err(import_destination_is_source(source, dest)),
            EntryAlias::CannotTell => {
                return Err(import_destination_alias_unknown(
                    source,
                    dest,
                    "the source did not read",
                ));
            }
        }
    }
    ensure_same_source(source, source_fingerprint)?;
    validate_import_destination_parent(destination_guard, dest)?;
    validate_import_destination_file(destination_guard)?;
    let source_proof = import_content_proof(source).map_err(|error| {
        import_error_context(
            &error,
            format!(
                "failed to prove import source content {}: {error}",
                source.display()
            ),
        )
    })?;
    let destination_proof = import_content_proof(dest).map_err(|error| {
        import_error_context(
            &error,
            format!(
                "failed to prove import destination content {}: {error}",
                dest.display()
            ),
        )
    })?;
    if source_proof != destination_proof || source_proof.size_bytes != size {
        return Err(AppError::ManualReconciliationRequired(format!(
            "import destination appeared with different content: {}",
            dest.display()
        )));
    }

    let source_cleanup = if source_cleanup_required {
        Some(ImportSourceCleanupGuard {
            adopted_destination_mark: Some(existing.to_stored()),
            ..cleanup_guard(source, dest, source_fingerprint, size)?
        })
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
        source_disposition: ImportSourceDisposition::Retained,
        source_cleanup,
        // This call copied nothing: the destination was already there and was
        // proved equal to the source by the sampled proof above.
        verification: None,
    }))
}

fn copy_prepared_import_blocking(prepared: PreparedImportPlacement) -> AppResult<ImportFileResult> {
    let source_mark = same_source_mark(&prepared.source, &prepared.source_fingerprint)?;
    validate_import_destination_parent(&prepared.destination_guard, &prepared.dest)?;
    ensure_import_destination_absent(&prepared.dest)?;
    let (verification, landed) = copy_regular_source_to_destination(
        &prepared.source,
        &prepared.dest,
        &prepared.source_fingerprint,
        &source_mark,
        prepared.size,
        prepared.options,
        prepared.verification_depth,
        prepared.progress.as_ref(),
        prepared.cancellation.as_ref(),
    )?;
    validate_import_destination_guard_after_placement(
        &prepared.destination_guard,
        &prepared.source,
        &prepared.dest,
        Landed::Copy {
            placed: landed.as_ref(),
            source: &source_mark,
        },
    )?;
    finalize_imported_regular_destination(&prepared.dest, &prepared.options, &prepared.permissions);
    // The source-cleanup guard is only built once verification passed, so
    // An unproven copy therefore has no guard to remove
    // anything with.
    let mut result = finish_prepared_import(
        prepared,
        ImportStrategy::Copy,
        Landed::Copy {
            placed: landed.as_ref(),
            source: &source_mark,
        },
    )?;
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
    let (source_fingerprint, source_mark, size, destination_guard) =
        prepare_marked_import_destination(&source, &dest, &permissions)?;
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
        let made = import_symlink_source(&source, &dest, &source_fingerprint, size)?;
        validate_import_destination_guard_after_placement(
            &destination_guard,
            &source,
            &dest,
            Landed::Link(&made),
        )?;
        let source_cleanup = cleanup_guard_after_placement(
            source_cleanup_required,
            &source,
            &dest,
            &source_fingerprint,
            size,
            Landed::Link(&made),
        )?;
        return Ok(ImportFileResult {
            strategy: ImportStrategy::Symlink,
            source_path: source,
            dest_path: dest,
            size_bytes: size,
            destination_disposition: ImportDestinationDisposition::Created,
            source_disposition: ImportSourceDisposition::Retained,
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
        let linked =
            hard_link_import_source(&source, &dest, &source_fingerprint, &source_mark, &options)?;
        #[cfg(test)]
        if matches!(linked, ImportLink::Proven)
            && let Some(after_linked) = options.after_hard_link
        {
            after_linked(&source, &dest);
        }
        // The source is read again whether or not the link proved. A link
        // that did not prove is already gone.
        let proven = matches!(linked, ImportLink::Proven);
        if !matches!(
            linked,
            ImportLink::NotPossible | ImportLink::Failed(_) | ImportLink::Occupied
        ) && let Err(error) = ensure_same_source(&source, &source_fingerprint)
        {
            if proven {
                return Err(took_back(
                    error,
                    &source,
                    &dest,
                    Landed::HardLink(&source_mark),
                ));
            }
            return Err(error);
        }
        match linked {
            ImportLink::Proven => {
                validate_import_destination_guard_after_placement(
                    &destination_guard,
                    &source,
                    &dest,
                    Landed::HardLink(&source_mark),
                )?;
                if file_permission_metadata_changes(&permissions) {
                    tracing::info!(
                        source = %source.display(),
                        dest = %dest.display(),
                        "applying import permissions to hardlink; metadata changes affect the shared download-side inode"
                    );
                }
                finalize_imported_regular_destination(&dest, &options, &permissions);
                let source_cleanup = cleanup_guard_after_placement(
                    source_cleanup_required,
                    &source,
                    &dest,
                    &source_fingerprint,
                    size,
                    Landed::HardLink(&source_mark),
                )?;
                return Ok(ImportFileResult {
                    strategy: ImportStrategy::HardLink,
                    source_path: source,
                    dest_path: dest,
                    size_bytes: size,
                    destination_disposition: ImportDestinationDisposition::Created,
                    source_disposition: ImportSourceDisposition::Retained,
                    source_cleanup,
                    verification: None,
                });
            }
            ImportLink::Unproven(LinkDoubt::Identity(mismatch)) => tracing::warn!(
                dest = %dest.display(),
                source_identity = %mark_label(&mismatch.source),
                linked_identity = %mark_label(&mismatch.linked),
                "hard link succeeded but the destination reports a different file identity; this filesystem does not expose stable file identity across paths (FUSE/sshfs, CIFS without serverino, mergerfs path-hash inodes), falling back to copy"
            ),
            ImportLink::Unproven(LinkDoubt::Length(linked)) => match linked.size() {
                Some(dest_len) => tracing::warn!(
                    "hard link size mismatch: source={} dest={}, falling back to copy",
                    size,
                    dest_len
                ),
                None => tracing::warn!(
                    "hard link created but dest stat failed: {}, falling back to copy",
                    linked.describe_absence()
                ),
            },
            // Another writer's file holds the name; nothing is copied over
            // it, as a copy would only be refused when it lands.
            ImportLink::Occupied => return Err(import_destination_occupied(&dest)),
            ImportLink::UnprovenGone(words) => tracing::warn!(
                "hard link did not prove and was already gone: {}, falling back to copy: {} -> {}",
                words,
                source.display(),
                dest.display()
            ),
            ImportLink::NotPossible => {
                tracing::info!(
                    "hard link is not possible here, falling back to copy: {} -> {}",
                    source.display(),
                    dest.display()
                );
            }
            ImportLink::Failed(e) => {
                tracing::warn!(
                    "hard link failed: {}, falling back to copy: {} -> {}",
                    e,
                    source.display(),
                    dest.display()
                );
            }
        }
    }

    let (verification, landed) = copy_regular_source_to_destination(
        &source,
        &dest,
        &source_fingerprint,
        &source_mark,
        size,
        options,
        verification_depth,
        progress.as_ref(),
        None,
    )?;
    validate_import_destination_guard_after_placement(
        &destination_guard,
        &source,
        &dest,
        Landed::Copy {
            placed: landed.as_ref(),
            source: &source_mark,
        },
    )?;
    finalize_imported_regular_destination(&dest, &options, &permissions);

    let source_cleanup = cleanup_guard_after_placement(
        source_cleanup_required,
        &source,
        &dest,
        &source_fingerprint,
        size,
        Landed::Copy {
            placed: landed.as_ref(),
            source: &source_mark,
        },
    )?;
    Ok(ImportFileResult {
        strategy: ImportStrategy::Copy,
        source_path: source,
        dest_path: dest,
        size_bytes: size,
        destination_disposition: ImportDestinationDisposition::Created,
        source_disposition: ImportSourceDisposition::Retained,
        source_cleanup,
        verification: Some(verification),
    })
}

/// Single-process placement for callers that name no depth. They get full
/// verification: only the operator's preference may choose the quick floor.
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
        VerificationDepth::Full,
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

const IMPORT_FILE_WORKER_PROTOCOL_VERSION: u16 = 3;
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
        /// Required: the parent always sends the resolved depth,
        /// and a request without one is rejected instead of being verified at
        /// whatever the type's default happens to be.
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
        #[serde(default)]
        hard_link_identity_mismatch: Option<HardLinkIdentityMismatch>,
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
        error: ImportFileWorkerError,
    },
}

/// Failure reported by the import file worker, with the variant the parent
/// needs preserved across the process boundary.
///
/// The parent decides between "park as Import Blocked" and "schedule an
/// automatic retry" by matching on `AppError::ManualReconciliationRequired`,
/// and treats `AppError::Canceled` as an operator stop rather than a failure,
/// so both must survive serialisation as themselves. Everything else the
/// worker's blocking functions produce is already `AppError::Repository`; any
/// other variant is carried as `Repository` with its rendered text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ImportFileWorkerError {
    Repository {
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        /// Where a failed copy was kept, for a failure that names one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kept_path: Option<String>,
    },
    ManualReconciliationRequired {
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    Canceled {
        message: String,
    },
}

impl From<AppError> for ImportFileWorkerError {
    fn from(error: AppError) -> Self {
        match error {
            AppError::ManualReconciliationRequired(message) => Self::ManualReconciliationRequired {
                message,
                reason: None,
            },
            AppError::ManualReconciliationRefused { message, reason } => {
                Self::ManualReconciliationRequired {
                    message,
                    reason: Some(reason.to_string()),
                }
            }
            AppError::Canceled(message) => Self::Canceled { message },
            AppError::Repository(message) | AppError::RepositoryFailure { message, .. } => {
                Self::Repository {
                    message,
                    reason: None,
                    kept_path: None,
                }
            }
            AppError::RepositoryFailed { message, reason } => Self::Repository {
                message,
                reason: Some(reason.to_string()),
                kept_path: None,
            },
            AppError::RepositoryFailedWithFacts {
                message,
                reason,
                facts,
            } => Self::Repository {
                message,
                reason: Some(reason.to_string()),
                kept_path: facts.into_iter().find_map(|fact| match fact {
                    RefusalFact {
                        key: scryer_application::error_reasons::import::KEPT_PATH_FACT,
                        value: RefusalFactValue::Text(path),
                    } => Some(path),
                    _ => None,
                }),
            },
            other => Self::Repository {
                message: other.to_string(),
                reason: None,
                kept_path: None,
            },
        }
    }
}

impl From<io::Error> for ImportFileWorkerError {
    fn from(error: io::Error) -> Self {
        Self::Repository {
            message: format!("failed to write import worker event: {error}"),
            reason: Some(import_io_reason(&error).to_string()),
            kept_path: None,
        }
    }
}

impl From<ImportFileWorkerError> for AppError {
    fn from(error: ImportFileWorkerError) -> Self {
        match error {
            ImportFileWorkerError::Repository {
                message,
                reason,
                kept_path,
            } => match (import_reason(reason.as_deref()), kept_path) {
                (Some(reason), Some(path)) => AppError::RepositoryFailedWithFacts {
                    message,
                    reason,
                    facts: vec![RefusalFact {
                        key: scryer_application::error_reasons::import::KEPT_PATH_FACT,
                        value: RefusalFactValue::Text(path),
                    }],
                },
                (Some(reason), None) => AppError::RepositoryFailed { message, reason },
                (None, _) => AppError::Repository(message),
            },
            ImportFileWorkerError::ManualReconciliationRequired { message, reason } => {
                match import_reason(reason.as_deref()) {
                    Some(reason) => AppError::ManualReconciliationRefused { message, reason },
                    None => AppError::ManualReconciliationRequired(message),
                }
            }
            ImportFileWorkerError::Canceled { message } => AppError::Canceled(message),
        }
    }
}

/// The import reason a worker named, as this build knows it; `None` for one
/// it does not.
fn import_reason(reason: Option<&str>) -> Option<&'static str> {
    let reason = reason?;
    scryer_application::error_reasons::import::ALL
        .iter()
        .copied()
        .find(|known| *known == reason)
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
                    error: ImportFileWorkerError::Repository {
                        message: error,
                        reason: None,
                        kept_path: None,
                    },
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
                error: ImportFileWorkerError::Repository {
                    message: format!("unsupported import worker protocol version {version}"),
                    reason: None,
                    kept_path: None,
                },
            },
        );
        return 2;
    }
    drop(input);
    start_import_worker_stdin_death_watch();

    let result: Result<(), ImportFileWorkerError> = match request {
        ImportFileWorkerRequest::Snapshot { source, .. } => {
            let _ = write_import_worker_event(
                &output,
                &ImportFileWorkerEvent::Stage {
                    nonce,
                    stage: "snapshot".to_string(),
                },
            );
            snapshot_import_source_blocking(source)
                .map_err(ImportFileWorkerError::from)
                .and_then(|snapshot| {
                    write_import_worker_event(
                        &output,
                        &ImportFileWorkerEvent::SnapshotFinished { nonce, snapshot },
                    )
                    .map_err(ImportFileWorkerError::from)
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
            .map_err(ImportFileWorkerError::from)
            .and_then(|()| {
                write_import_worker_event(
                    &output,
                    &ImportFileWorkerEvent::CleanupFinished { nonce },
                )
                .map_err(ImportFileWorkerError::from)
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
            .map_err(ImportFileWorkerError::from)
            .and_then(|prepared| {
                write_import_worker_event(
                    &output,
                    &ImportFileWorkerEvent::Prepared { nonce, prepared },
                )
                .map_err(ImportFileWorkerError::from)
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
                .map_err(ImportFileWorkerError::from)
                .and_then(|result| match result {
                    FastPlacementResult::Finished(result) => write_import_worker_event(
                        &output,
                        &ImportFileWorkerEvent::ImportFinished { nonce, result },
                    )
                    .map_err(ImportFileWorkerError::from),
                    FastPlacementResult::CopyRequired {
                        prepared,
                        hard_link_identity_mismatch,
                    } => write_import_worker_event(
                        &output,
                        &ImportFileWorkerEvent::CopyRequired {
                            nonce,
                            volume_key: prepared.volume_key,
                            hard_link_identity_mismatch,
                        },
                    )
                    .map_err(ImportFileWorkerError::from),
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
                .map_err(ImportFileWorkerError::from)
                .and_then(|result| {
                    write_import_worker_event(
                        &output,
                        &ImportFileWorkerEvent::ImportFinished { nonce, result },
                    )
                    .map_err(ImportFileWorkerError::from)
                });
            let _ = progress_thread.join();
            outcome
        }
    };

    if let Err(error) = result {
        let _ = write_import_worker_event(&output, &ImportFileWorkerEvent::Error { nonce, error });
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
    /// modes an import would have applied. Best-effort for the same
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
        .map_err(|error| AppError::Repository(format!("permission task panicked: {error}")))
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
        .map_err(|error| AppError::Repository(format!("permission task panicked: {error}")))
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
                    ImportFileWorkerEvent::Error { error, .. } => {
                        return Err(error.into());
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
        // No depth was named, so the copy path proves at full depth: only the
        // operator's preference may choose the quick floor (never a
        // silent downgrade).
        self.place_import_file(
            source,
            dest,
            mode,
            expected_source,
            progress,
            permissions,
            context,
            VerificationDepth::Full,
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
                    ImportFileWorkerEvent::Error { error, .. } => {
                        return Err(error.into());
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
    /// only thing that varies is the depth the copy path proves at.
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
            let mut prepared = loop {
                match next_import_worker_event_or_cancel(&mut session, cancellation.as_ref())
                    .await?
                {
                    ImportFileWorkerEvent::Stage { .. } => {}
                    ImportFileWorkerEvent::Prepared { prepared, .. } => {
                        wait_for_import_worker_exit(&mut session, "after preparing import").await?;
                        break prepared;
                    }
                    ImportFileWorkerEvent::Error { error, .. } => {
                        return Err(error.into());
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
            self.placement
                .ready_fast_placement(&mut prepared, context.archive_workspace_source());

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
                        if phase == ImportTransferPhase::Verifying {
                            drop(copy_permit.take());
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
                        hard_link_identity_mismatch,
                        ..
                    } => {
                        if reported_volume_key != volume_key {
                            return Err(AppError::ManualReconciliationRequired(
                                "filesystem worker changed destination volume identity; inspect the destination before retrying"
                                    .to_string(),
                            ));
                        }
                        if let Some(mismatch) = &hard_link_identity_mismatch {
                            self.placement.record_hard_link_identity_mismatch(
                                &volume_key,
                                dest,
                                mismatch,
                            );
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
                                self.placement.copy.acquire_on(&copy_lane(&volume_key)),
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
                    ImportFileWorkerEvent::Error { error, .. } => {
                        return Err(error.into());
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
        self.placement
            .ready_fast_placement(&mut prepared, context.archive_workspace_source());
        let fast_result =
            tokio::task::spawn_blocking(move || try_fast_import_placement_blocking(prepared))
                .await
                .map_err(|error| {
                    AppError::Repository(format!("fast import task panicked: {error}"))
                })??;

        match fast_result {
            FastPlacementResult::Finished(result) => Ok(result),
            FastPlacementResult::CopyRequired {
                mut prepared,
                hard_link_identity_mismatch,
            } => {
                let volume_key = prepared.volume_key.clone();
                if let Some(mismatch) = &hard_link_identity_mismatch {
                    self.placement.record_hard_link_identity_mismatch(
                        &volume_key,
                        &prepared.dest,
                        mismatch,
                    );
                }
                drop(fast_permit);
                tracing::debug!(
                    client_key = %opaque_lane_key(&client_key),
                    volume_key = %opaque_lane_key(&volume_key),
                    "released fast import lane before copy fallback"
                );
                report_import_transfer_progress(
                    prepared.progress.as_ref(),
                    ImportTransferPhase::Waiting,
                    0,
                    prepared.size,
                );
                let mut copy_permit = Some(
                    wait_for_import_operation_or_cancel(
                        cancellation.as_ref(),
                        self.placement.copy.acquire_on(&copy_lane(&volume_key)),
                    )
                    .await?,
                );
                context.mark_active_import_copying().await;
                let forward = prepared.progress.take();
                let (sender, mut receiver) =
                    tokio::sync::mpsc::unbounded_channel::<ImportFileTransferProgress>();
                prepared.progress = Some(sender);
                let mut task =
                    tokio::task::spawn_blocking(move || copy_prepared_import_blocking(prepared));
                loop {
                    tokio::select! {
                        result = &mut task => break result.map_err(|error| AppError::Repository(format!("copy import task panicked: {error}")))?,
                        Some(update) = receiver.recv() => {
                            if update.phase == ImportTransferPhase::Finalizing {
                                context.mark_active_import_finalizing().await;
                            }
                            if update.phase == ImportTransferPhase::Verifying {
                                drop(copy_permit.take());
                            }
                            if let Some(sender) = &forward { let _ = sender.send(update); }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip_worker_error(error: AppError) -> AppError {
        let event = ImportFileWorkerEvent::Error {
            nonce: 7,
            error: ImportFileWorkerError::from(error),
        };
        let line = serde_json::to_string(&event).expect("event serialises");
        match serde_json::from_str::<ImportFileWorkerEvent>(&line).expect("event parses") {
            ImportFileWorkerEvent::Error { nonce, error } => {
                assert_eq!(nonce, 7);
                AppError::from(error)
            }
            other => panic!("unexpected event {other:?}"),
        }
    }

    // The parent routes on these variants, so they must come back as
    // themselves after crossing the worker's stdout, not as repository text.
    #[test]
    fn worker_error_round_trip_preserves_routed_variants() {
        let detail = "import destination appeared with different content: /x";
        let error = round_trip_worker_error(AppError::ManualReconciliationRequired(detail.into()));
        assert!(
            matches!(&error, AppError::ManualReconciliationRequired(message) if message == detail),
            "got {error:?}"
        );
        assert_eq!(
            error.to_string(),
            format!("manual reconciliation required: {detail}")
        );

        let error = round_trip_worker_error(AppError::canceled("operator stop"));
        assert!(matches!(&error, AppError::Canceled(message) if message == "operator stop"));

        let error = round_trip_worker_error(AppError::Repository("disk full".into()));
        assert!(
            matches!(&error, AppError::Repository(message) | AppError::RepositoryFailure { message, .. } if message == "disk full")
        );
    }

    // Anything the worker's functions do not produce today still crosses as a
    // repository error carrying the rendered text, rather than failing to
    // serialise.
    #[test]
    fn worker_error_round_trip_flattens_unrouted_variants() {
        let rendered = AppError::Validation("bad request".into()).to_string();
        let error = round_trip_worker_error(AppError::Validation("bad request".into()));
        assert!(
            matches!(&error, AppError::Repository(message) | AppError::RepositoryFailure { message, .. } if *message == rendered),
            "got {error:?}"
        );
    }

    #[test]
    fn worker_error_round_trip_preserves_io_kind_without_reading_its_message() {
        use scryer_application::error_reasons::import;
        for (kind, reason) in [
            (io::ErrorKind::NotFound, import::FILE_NOT_FOUND),
            (io::ErrorKind::PermissionDenied, import::PERMISSION_DENIED),
            (io::ErrorKind::StorageFull, import::DISK_FULL),
            (io::ErrorKind::Other, import::IO_FAILED),
        ] {
            let error = io::Error::new(kind, "wording has no meaning");
            let result =
                round_trip_worker_error(import_io_error(&error, "context may change".into()));
            assert_eq!(result.reason(), Some(reason));
        }
    }

    /// A refusal's import reason, and the path a failed copy was kept at,
    /// cross the worker's stdout as themselves; a reason this build does not
    /// know falls back to the bare failure.
    #[test]
    fn worker_error_round_trip_preserves_import_reasons_and_the_kept_path() {
        use scryer_application::error_reasons::import;
        let source = Path::new("/srv/fixture-media/downloads/sample.mkv");
        let dest = Path::new("/srv/fixture-media/library/Sample.Title.mkv");

        let error = round_trip_worker_error(import_destination_is_source(source, dest));
        assert!(error.requires_manual_reconciliation(), "{error:?}");
        assert_eq!(error.reason(), Some(import::DESTINATION_IS_SOURCE));

        for error in [
            import_destination_alias_unknown(source, dest, "the source did not read"),
            import_destination_occupied(dest),
        ] {
            let reason = error.reason();
            assert_eq!(round_trip_worker_error(error).reason(), reason);
        }

        let kept = AppError::RepositoryFailedWithFacts {
            message: "copy verification failed".into(),
            reason: import::COPY_KEPT_SOURCE_GONE,
            facts: vec![RefusalFact {
                key: import::KEPT_PATH_FACT,
                value: RefusalFactValue::Text(dest.display().to_string()),
            }],
        };
        let error = round_trip_worker_error(kept.clone());
        assert_eq!(error.reason(), Some(import::COPY_KEPT_SOURCE_GONE));
        assert_eq!(error.refusal_facts(), kept.refusal_facts());

        let unknown = ImportFileWorkerError::ManualReconciliationRequired {
            message: "x".into(),
            reason: Some("IMPORT_NOT_A_CODE_THIS_BUILD_KNOWS".into()),
        };
        assert!(matches!(
            AppError::from(unknown),
            AppError::ManualReconciliationRequired(_)
        ));
    }

    #[test]
    fn worker_error_wire_shape_is_tagged_by_kind() {
        let error = ImportFileWorkerError::from(AppError::ManualReconciliationRequired("x".into()));
        let json = serde_json::to_value(&error).expect("serialises");
        assert_eq!(json["kind"], "manual_reconciliation_required");
        assert_eq!(json["message"], "x");
    }

    #[test]
    fn worker_prepare_request_requires_a_verification_depth() {
        let request = |depth: Option<&str>| {
            let mut value = serde_json::json!({
                "command": "prepare",
                "version": IMPORT_FILE_WORKER_PROTOCOL_VERSION,
                "nonce": 7,
                "source": "/synthetic/source.mkv",
                "dest": "/synthetic/library/destination.mkv",
                "mode": "hardlink_or_copy",
                "expected_source": null,
                "permissions": ImportFilePermissions::default(),
            });
            if let Some(depth) = depth {
                value["verification_depth"] = serde_json::json!(depth);
            }
            serde_json::from_value::<ImportFileWorkerRequest>(value)
        };

        assert!(
            request(None).is_err(),
            "a prepare request without a depth must not fall back to a default"
        );
        for (wire, expected) in [
            ("full", VerificationDepth::Full),
            ("quick", VerificationDepth::Quick),
        ] {
            match request(Some(wire)).expect("prepare request with depth parses") {
                ImportFileWorkerRequest::Prepare {
                    verification_depth, ..
                } => assert_eq!(verification_depth, expected),
                other => panic!("unexpected request: {other:?}"),
            }
        }
    }
    #[cfg(unix)]
    use std::io::{self, Write};
    #[cfg(unix)]
    use std::sync::{Arc, Mutex};
    #[cfg(unix)]
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

    /// Returns once every task on the runtime is parked. Only valid on a
    /// paused clock: tokio advances paused time only when no task can run, so
    /// this sleep cannot complete while a spawned waiter still has work.
    async fn run_until_idle() {
        tokio::time::sleep(Duration::from_millis(1)).await;
    }

    #[tokio::test(start_paused = true)]
    async fn copy_lanes_are_volume_scoped_and_do_not_block_fast_lanes() {
        let coordinator = ImportPlacementCoordinator::new(1, 2);
        let volume = tempfile::tempdir().expect("tempdir");
        let lane = Disk::process()
            .landing_volume_blocking(volume.path())
            .expect("volume");
        let first_copy = coordinator.copy.acquire_on(&lane).await;
        let second_copy = coordinator.copy.acquire_on(&lane).await;
        let third_copy = tokio::spawn({
            let copy = coordinator.copy.clone();
            async move { copy.acquire_on(&lane).await }
        });
        run_until_idle().await;
        assert!(
            !third_copy.is_finished(),
            "a third copy to one volume must wait"
        );

        let fast = tokio::time::timeout(
            Duration::from_secs(30),
            coordinator.fast.acquire("client-a", "fast"),
        )
        .await
        .expect("a hardlink lane must remain available while copies are blocked");

        drop(fast);
        drop(first_copy);
        let third_copy = tokio::time::timeout(Duration::from_secs(30), third_copy)
            .await
            .expect("waiting copy should start after one same-volume slot is released")
            .expect("copy waiter should not panic");
        drop(third_copy);
        drop(second_copy);
    }

    #[tokio::test(start_paused = true)]
    async fn fast_lanes_are_client_scoped() {
        let coordinator = ImportPlacementCoordinator::new(1, 2);
        let first = coordinator.fast.acquire("client-a", "fast").await;
        let same_client = tokio::spawn({
            let fast = coordinator.fast.clone();
            async move { fast.acquire("client-a", "fast").await }
        });
        run_until_idle().await;
        assert!(!same_client.is_finished());

        let other_client = tokio::time::timeout(
            Duration::from_secs(30),
            coordinator.fast.acquire("client-b", "fast"),
        )
        .await
        .expect("different clients should place fast imports concurrently");
        drop(other_client);
        drop(first);
        let same_client = tokio::time::timeout(Duration::from_secs(30), same_client)
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
        // The key in hand is the one the disk service gives the same folder,
        // so admission needs no probe of its own.
        let parent = Disk::process()
            .landing_volume_blocking(&temp.path().join("library-a"))
            .expect("destination parent");
        assert_eq!(first.volume_key, parent.to_stored());
        assert_eq!(
            copy_lane(&first.volume_key).to_stored(),
            first.volume_key,
            "the key reads back as the same lane"
        );
        assert_eq!(copy_lane("fixture-nonsense").to_stored(), "unknown-volume");
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

    const SYNTHETIC_PAYLOAD: &[u8] = b"synthetic video payload";

    fn hard_links_supported(dir: &Path) -> bool {
        let probe_source = dir.join("link-probe-source");
        let probe_link = dir.join("link-probe-link");
        std::fs::write(&probe_source, b"probe").expect("write link probe");
        let supported = std::fs::hard_link(&probe_source, &probe_link).is_ok();
        let _ = std::fs::remove_file(&probe_link);
        std::fs::remove_file(&probe_source).expect("remove link probe");
        supported
    }

    fn prepare_in_process(
        source: &Path,
        dest: &Path,
        options: ImportFileOptions,
        source_cleanup_required: bool,
    ) -> PreparedImportPlacement {
        prepare_import_placement_blocking(
            source.to_path_buf(),
            dest.to_path_buf(),
            options,
            VerificationDepth::default(),
            source_cleanup_required,
            None,
            None,
            ImportFilePermissions::default(),
        )
        .expect("prepare placement")
    }

    fn hard_link_attempts() -> usize {
        HARD_LINK_ATTEMPTS.with(std::cell::Cell::get)
    }

    /// Runs the fast lane the way the parent does and, when it hands off,
    /// records what it reported and lands the file through the copy lane.
    fn place_in_process(
        coordinator: &ImportPlacementCoordinator,
        mut prepared: PreparedImportPlacement,
        archive_workspace_source: bool,
    ) -> (ImportFileResult, Option<HardLinkIdentityMismatch>) {
        coordinator.ready_fast_placement(&mut prepared, archive_workspace_source);
        match try_fast_import_placement_blocking(prepared).expect("fast placement") {
            FastPlacementResult::Finished(result) => (result, None),
            FastPlacementResult::CopyRequired {
                prepared,
                hard_link_identity_mismatch,
            } => {
                assert!(
                    !prepared.dest.exists(),
                    "the copy lane must start from an absent destination"
                );
                if let Some(mismatch) = &hard_link_identity_mismatch {
                    coordinator.record_hard_link_identity_mismatch(
                        &prepared.volume_key,
                        &prepared.dest,
                        mismatch,
                    );
                }
                let result = copy_prepared_import_blocking(prepared).expect("copy lane");
                (result, hard_link_identity_mismatch)
            }
        }
    }

    #[cfg(unix)]
    fn assert_no_stray_link(source: &Path) {
        assert_eq!(
            std::fs::metadata(source).expect("source metadata").nlink(),
            1,
            "no link to the source may survive an unverifiable hard link"
        );
    }

    #[test]
    fn unverifiable_hard_link_identity_hands_import_to_copy_lane() {
        let temp = tempfile::tempdir().expect("tempdir");
        if !hard_links_supported(temp.path()) {
            return;
        }
        let source = temp.path().join("source.mkv");
        let dest = temp.path().join("library/title.mkv");
        std::fs::write(&source, SYNTHETIC_PAYLOAD).expect("write source");
        let coordinator = ImportPlacementCoordinator::new(1, 2);
        let options = ImportFileOptions {
            force_hard_link_identity_mismatch: true,
            ..Default::default()
        };

        let attempts_before = hard_link_attempts();
        let (result, mismatch) = place_in_process(
            &coordinator,
            prepare_in_process(&source, &dest, options, false),
            false,
        );

        assert_eq!(hard_link_attempts(), attempts_before + 1);
        assert!(mismatch.is_some(), "the identity mismatch must be reported");
        assert_eq!(result.strategy, ImportStrategy::Copy);
        assert!(result.verification.is_some());
        assert_eq!(std::fs::read(&dest).expect("read dest"), SYNTHETIC_PAYLOAD);
        assert_eq!(
            std::fs::read(&source).expect("read source"),
            SYNTHETIC_PAYLOAD
        );
        #[cfg(unix)]
        assert_no_stray_link(&source);
    }

    #[test]
    fn a_volume_with_unverifiable_link_identity_skips_the_hard_link() {
        let temp = tempfile::tempdir().expect("tempdir");
        if !hard_links_supported(temp.path()) {
            return;
        }
        let first_source = temp.path().join("first.mkv");
        let second_source = temp.path().join("second.mkv");
        std::fs::write(&first_source, SYNTHETIC_PAYLOAD).expect("write first source");
        std::fs::write(&second_source, SYNTHETIC_PAYLOAD).expect("write second source");
        let coordinator = ImportPlacementCoordinator::new(1, 2);
        let mismatching = ImportFileOptions {
            force_hard_link_identity_mismatch: true,
            ..Default::default()
        };
        let first = prepare_in_process(
            &first_source,
            &temp.path().join("library/first.mkv"),
            mismatching,
            false,
        );
        let volume_key = first.volume_key.clone();
        let _ = place_in_process(&coordinator, first, false);

        // The second placement links normally if attempted, so only the
        // remembered volume keeps it off the hard-link lane.
        let second_dest = temp.path().join("library/second.mkv");
        let second = prepare_in_process(
            &second_source,
            &second_dest,
            ImportFileOptions::default(),
            false,
        );
        assert_eq!(second.volume_key, volume_key);
        let attempts_before = hard_link_attempts();
        let (result, mismatch) = place_in_process(&coordinator, second, false);

        assert_eq!(
            hard_link_attempts(),
            attempts_before,
            "a remembered volume must not attempt a hard link"
        );
        assert!(mismatch.is_none());
        assert_eq!(result.strategy, ImportStrategy::Copy);
        assert_eq!(
            std::fs::read(&second_dest).expect("read dest"),
            SYNTHETIC_PAYLOAD
        );
        #[cfg(unix)]
        assert_no_stray_link(&second_source);

        let fresh = ImportPlacementCoordinator::new(1, 2);
        let third_dest = temp.path().join("library/third.mkv");
        let (result, _) = place_in_process(
            &fresh,
            prepare_in_process(
                &second_source,
                &third_dest,
                ImportFileOptions::default(),
                false,
            ),
            false,
        );
        assert_eq!(
            result.strategy,
            ImportStrategy::HardLink,
            "another coordinator has not seen the volume"
        );
    }

    #[test]
    fn single_process_import_copies_when_hard_link_identity_is_unverifiable() {
        let temp = tempfile::tempdir().expect("tempdir");
        if !hard_links_supported(temp.path()) {
            return;
        }
        let source = temp.path().join("source.mkv");
        let dest = temp.path().join("library/title.mkv");
        std::fs::write(&source, SYNTHETIC_PAYLOAD).expect("write source");

        let result = import_file_blocking(
            source.clone(),
            dest.clone(),
            ImportMode::HardlinkOrCopy,
            ImportFileOptions {
                force_hard_link_identity_mismatch: true,
                ..Default::default()
            },
            None,
            None,
            ImportFilePermissions::default(),
        )
        .expect("import file");

        assert_eq!(result.strategy, ImportStrategy::Copy);
        assert_eq!(std::fs::read(&dest).expect("read dest"), SYNTHETIC_PAYLOAD);
        assert_eq!(
            std::fs::read(&source).expect("read source"),
            SYNTHETIC_PAYLOAD
        );
        #[cfg(unix)]
        assert_no_stray_link(&source);
    }

    fn archive_workspace_fixture(temp: &Path) -> (PathBuf, PathBuf) {
        let title_dir = temp.join("Synthetic Show (2026)");
        let output_dir = title_dir.join(".scryer-ax-0123456789abcdef/out");
        std::fs::create_dir_all(&output_dir).expect("create workspace output");
        let source = output_dir.join("Synthetic.Show.S01E01.1080p.mkv");
        std::fs::write(&source, SYNTHETIC_PAYLOAD).expect("write workspace output");
        let dest = title_dir.join("Season 01/Synthetic Show - S01E01 - 1080p.mkv");
        (source, dest)
    }

    #[test]
    fn archive_workspace_output_is_placed_by_rename() {
        let temp = tempfile::tempdir().expect("tempdir");
        let (source, dest) = archive_workspace_fixture(temp.path());
        let coordinator = ImportPlacementCoordinator::new(1, 2);

        let attempts_before = hard_link_attempts();
        let (result, _) = place_in_process(
            &coordinator,
            prepare_in_process(&source, &dest, ImportFileOptions::default(), false),
            true,
        );

        assert_eq!(result.strategy, ImportStrategy::Move);
        assert_eq!(
            result.source_disposition,
            ImportSourceDisposition::RenamedIntoPlace
        );
        assert_eq!(hard_link_attempts(), attempts_before);
        assert!(result.source_cleanup.is_none());
        assert!(result.verification.is_none());
        assert!(!source.exists(), "the workspace file is moved, not linked");
        assert_eq!(std::fs::read(&dest).expect("read dest"), SYNTHETIC_PAYLOAD);
    }

    #[test]
    fn archive_workspace_rename_failure_falls_through_to_the_link_lane() {
        let temp = tempfile::tempdir().expect("tempdir");
        let (source, dest) = archive_workspace_fixture(temp.path());
        let coordinator = ImportPlacementCoordinator::new(1, 2);
        let options = ImportFileOptions {
            rename_fault: RenameFault::Refused,
            ..Default::default()
        };

        let (result, _) = place_in_process(
            &coordinator,
            prepare_in_process(&source, &dest, options, false),
            true,
        );

        assert!(matches!(
            result.strategy,
            ImportStrategy::HardLink | ImportStrategy::Copy
        ));
        assert_eq!(
            std::fs::read(&source).expect("read source"),
            SYNTHETIC_PAYLOAD
        );
        assert_eq!(std::fs::read(&dest).expect("read dest"), SYNTHETIC_PAYLOAD);
    }

    #[test]
    fn move_imports_of_archive_workspace_output_are_placed_by_rename() {
        let temp = tempfile::tempdir().expect("tempdir");
        let (source, dest) = archive_workspace_fixture(temp.path());
        let sibling = source.with_file_name("Synthetic.Show.S01E02.1080p.mkv");
        std::fs::write(&sibling, b"sibling payload").expect("write sibling output");
        let coordinator = ImportPlacementCoordinator::new(1, 2);

        let attempts_before = hard_link_attempts();
        let (result, _) = place_in_process(
            &coordinator,
            prepare_in_process(&source, &dest, ImportFileOptions::default(), true),
            true,
        );

        assert_eq!(result.strategy, ImportStrategy::Move);
        assert_eq!(
            result.source_disposition,
            ImportSourceDisposition::RenamedIntoPlace
        );
        assert!(
            result.source_cleanup.is_none(),
            "nothing is left at the source path for a guard to remove"
        );
        assert!(result.verification.is_none());
        assert_eq!(hard_link_attempts(), attempts_before);
        assert!(!source.exists(), "the workspace file is moved, not copied");
        assert_eq!(std::fs::read(&dest).expect("read dest"), SYNTHETIC_PAYLOAD);
        assert_eq!(
            std::fs::read(&sibling).expect("read sibling"),
            b"sibling payload",
            "other extraction output is untouched"
        );
    }

    #[test]
    fn move_import_whose_rename_fails_keeps_the_cleanup_guard() {
        let temp = tempfile::tempdir().expect("tempdir");
        let (source, dest) = archive_workspace_fixture(temp.path());
        let coordinator = ImportPlacementCoordinator::new(1, 2);
        let options = ImportFileOptions {
            rename_fault: RenameFault::Refused,
            ..Default::default()
        };

        let (result, _) = place_in_process(
            &coordinator,
            prepare_in_process(&source, &dest, options, true),
            true,
        );

        assert!(matches!(
            result.strategy,
            ImportStrategy::HardLink | ImportStrategy::Copy
        ));
        assert_eq!(result.source_disposition, ImportSourceDisposition::Retained);
        assert!(result.source_cleanup.is_some());
        assert!(source.exists(), "the cleanup guard owns source removal");
        assert_eq!(std::fs::read(&dest).expect("read dest"), SYNTHETIC_PAYLOAD);
    }

    #[test]
    fn move_import_outside_an_extractor_workspace_keeps_the_cleanup_guard() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp
            .path()
            .join("downloads/Synthetic.Show.S01E01.1080p.mkv");
        std::fs::create_dir_all(source.parent().expect("parent")).expect("create downloads");
        std::fs::write(&source, SYNTHETIC_PAYLOAD).expect("write download");
        let dest = temp
            .path()
            .join("library/Season 01/Synthetic Show - S01E01 - 1080p.mkv");
        let coordinator = ImportPlacementCoordinator::new(1, 2);

        let (result, _) = place_in_process(
            &coordinator,
            prepare_in_process(&source, &dest, ImportFileOptions::default(), true),
            false,
        );

        assert_eq!(result.source_disposition, ImportSourceDisposition::Retained);
        assert!(result.source_cleanup.is_some());
        assert!(source.exists(), "the cleanup guard owns source removal");
    }

    fn rename_lane_error(source: &Path, dest: &Path, fault: RenameFault) -> AppError {
        let coordinator = ImportPlacementCoordinator::new(1, 2);
        let options = ImportFileOptions {
            rename_fault: fault,
            ..Default::default()
        };
        let mut prepared = prepare_in_process(source, dest, options, true);
        coordinator.ready_fast_placement(&mut prepared, true);
        match try_fast_import_placement_blocking(prepared) {
            Ok(_) => panic!("the rename lane must stop the import"),
            Err(error) => error,
        }
    }

    #[test]
    fn renamed_archive_output_failing_its_check_is_left_in_place() {
        let temp = tempfile::tempdir().expect("tempdir");
        let (source, dest) = archive_workspace_fixture(temp.path());

        let attempts_before = hard_link_attempts();
        let error = rename_lane_error(&source, &dest, RenameFault::DestinationReplacedAfterRename);

        assert!(
            matches!(error, AppError::ManualReconciliationRequired(_)),
            "{error}"
        );
        assert_eq!(hard_link_attempts(), attempts_before);
        assert_eq!(
            std::fs::read(&dest).expect("read dest"),
            b"unrelated file",
            "the file the destination path now names is not moved"
        );
        assert!(
            !source.exists(),
            "nothing is moved back into the workspace, which is removed after the import"
        );
        assert_eq!(
            std::fs::read(dest.with_extension("moved-aside")).expect("read renamed output"),
            SYNTHETIC_PAYLOAD
        );
    }

    #[test]
    fn renamed_archive_output_replaced_with_same_size_requires_reconciliation() {
        let temp = tempfile::tempdir().expect("tempdir");
        let (source, dest) = archive_workspace_fixture(temp.path());
        let attempts_before = hard_link_attempts();
        let error = rename_lane_error(
            &source,
            &dest,
            RenameFault::DestinationReplacedWithSameSizeAfterRename,
        );

        assert!(
            matches!(error, AppError::ManualReconciliationRequired(_)),
            "{error}"
        );
        assert!(
            error.to_string().contains("content does not match"),
            "{error}"
        );
        assert_eq!(hard_link_attempts(), attempts_before);
        assert_eq!(
            std::fs::read(&dest).expect("replacement preserved"),
            vec![b'x'; SYNTHETIC_PAYLOAD.len()]
        );
        assert_eq!(
            std::fs::read(dest.with_extension("moved-aside")).expect("output preserved"),
            SYNTHETIC_PAYLOAD
        );
        assert!(!source.exists(), "no file is moved back into the workspace");
    }

    #[test]
    fn rename_reported_failed_after_taking_effect_removes_nothing() {
        let temp = tempfile::tempdir().expect("tempdir");
        let (source, dest) = archive_workspace_fixture(temp.path());

        let attempts_before = hard_link_attempts();
        let error = rename_lane_error(&source, &dest, RenameFault::ReportedAfterTakingEffect);

        assert!(
            matches!(error, AppError::ManualReconciliationRequired(_)),
            "{error}"
        );
        assert_eq!(hard_link_attempts(), attempts_before);
        assert!(!source.exists());
        assert_eq!(
            std::fs::read(&dest).expect("read dest"),
            SYNTHETIC_PAYLOAD,
            "the only copy of the output stays at the destination"
        );
    }

    #[test]
    fn filled_rename_reservation_is_left_alone() {
        let temp = tempfile::tempdir().expect("tempdir");
        let (source, dest) = archive_workspace_fixture(temp.path());

        let attempts_before = hard_link_attempts();
        let error = rename_lane_error(&source, &dest, RenameFault::RefusedAfterReservationFilled);

        assert!(
            matches!(error, AppError::ManualReconciliationRequired(_)),
            "{error}"
        );
        assert_eq!(hard_link_attempts(), attempts_before);
        assert_eq!(std::fs::read(&dest).expect("read dest"), b"another writer");
        assert_eq!(
            std::fs::read(&source).expect("read source"),
            SYNTHETIC_PAYLOAD
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
            .with_max_level(tracing::Level::DEBUG)
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
        let source_mark = Disk::process()
            .fingerprint_blocking(&source)
            .expect("source mark");
        let placed = Disk::process().fingerprint_blocking(&dest).ok();
        let old_parent = dir.path().join("library-old");
        std::fs::rename(&parent, &old_parent).expect("replace approved parent");
        std::fs::create_dir_all(&parent).expect("create replacement parent");
        std::fs::write(&dest, b"replacement parent bytes").expect("write replacement occupant");

        let error = validate_import_destination_guard_after_placement(
            &guard,
            &source,
            &dest,
            Landed::Copy {
                placed: placed.as_ref(),
                source: &source_mark,
            },
        )
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

    /// A destination that fails its check is taken back only while it is
    /// still what the import placed: a folder, or a socket someone else put
    /// at the name, is kept, and the check's own failure is reported.
    #[cfg(unix)]
    #[test]
    fn destination_guard_after_placement_keeps_what_the_import_did_not_place() {
        for socket in [false, true] {
            let dir = tempfile::tempdir().expect("tempdir");
            let source = dir.path().join("source.mkv");
            std::fs::write(&source, b"fake video bytes").expect("write source");
            let parent = dir.path().join("library");
            let dest = parent.join("Imported.Movie.mkv");
            let (_source_fingerprint, _size, guard) =
                prepare_import_destination(&source, &dest, &ImportFilePermissions::default())
                    .expect("prepare destination");
            std::fs::write(&dest, b"placed video bytes").expect("write placed destination");
            let source_mark = Disk::process()
                .fingerprint_blocking(&source)
                .expect("source mark");
            let placed = Disk::process().fingerprint_blocking(&dest).ok();
            // Moved aside, not removed, so its file number stays taken and
            // what is put at the name next cannot be numbered as it was.
            std::fs::rename(&dest, dir.path().join("placed-aside.mkv"))
                .expect("move the placed file aside");
            let _listener = match socket {
                true => Some(std::os::unix::net::UnixListener::bind(&dest).expect("bind socket")),
                false => {
                    std::fs::create_dir(&dest).expect("create non-file destination");
                    None
                }
            };

            let error = validate_import_destination_guard_after_placement(
                &guard,
                &source,
                &dest,
                Landed::Copy {
                    placed: placed.as_ref(),
                    source: &source_mark,
                },
            )
            .expect_err("a destination that is not a file is rejected");
            assert!(
                error
                    .to_string()
                    .contains("import destination is not a file or symlink")
            );
            assert!(
                std::fs::symlink_metadata(&dest).is_ok(),
                "what the import did not place is kept"
            );
        }
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

    /// The shape the application layer accepts from a move: a source cleanup
    /// guard, beside a verification that permits removal when there is one,
    /// or a rename that needs neither.
    #[cfg(unix)]
    fn application_accepts_move(result: &ImportFileResult) -> bool {
        match result.source_disposition {
            ImportSourceDisposition::RenamedIntoPlace => {
                result.source_cleanup.is_none() && result.verification.is_none()
            }
            _ => {
                result.source_cleanup.is_some()
                    && result
                        .verification
                        .as_ref()
                        .is_none_or(|verification| verification.permits_source_removal())
            }
        }
    }

    /// Moves a source onto a destination `arrange` made beside it, equal in
    /// content, and finishes the move as the application does. Says whether
    /// the import was accepted, and what then reads through each name.
    #[cfg(unix)]
    fn move_onto_existing(
        arrange: &dyn Fn(&Path, &Path) -> PathBuf,
    ) -> (bool, Option<Vec<u8>>, Option<Vec<u8>>, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("downloads").join("source.mkv");
        std::fs::create_dir_all(source.parent().expect("parent")).expect("downloads");
        std::fs::write(&source, b"only video bytes").expect("write source");
        let dest = arrange(dir.path(), &source);
        let result = import_file_blocking(
            source.clone(),
            dest.clone(),
            ImportMode::Move,
            ImportFileOptions::default(),
            None,
            None,
            ImportFilePermissions::default(),
        );
        let finished = result.and_then(|result| {
            assert!(application_accepts_move(&result), "{result:?}");
            let guard = result.source_cleanup.clone().expect("a move guard");
            remove_import_source_after_verified_import_blocking(
                guard,
                dest.clone(),
                ImportFileOptions::default(),
            )
        });
        let accepted = match finished {
            Ok(()) => true,
            Err(error) => {
                assert!(error.requires_manual_reconciliation(), "{error:?}");
                assert_eq!(
                    error.reason(),
                    Some(scryer_application::error_reasons::import::DESTINATION_IS_SOURCE),
                    "{error:?}"
                );
                false
            }
        };
        assert!(
            !disk::aside_path_for(&source).exists(),
            "nothing is left at the set-aside name"
        );
        let kept = std::fs::read(&source).ok();
        let through_dest = std::fs::read(&dest).ok();
        (accepted, kept, through_dest, dir)
    }

    /// A destination that names the source's own entry is not a copy of it:
    /// a move that removed the source would leave nothing behind, so it is
    /// refused and both names are kept.
    #[cfg(unix)]
    #[test]
    fn a_destination_that_names_the_source_itself_never_lets_a_move_remove_it() {
        let bytes = Some(b"only video bytes".to_vec());
        let library = |root: &Path| {
            let library = root.join("library");
            std::fs::create_dir_all(&library).expect("library");
            library
        };

        // A symlink to the source.
        let (accepted, kept, through_dest, _dir) = move_onto_existing(&|root, source| {
            let dest = library(root).join("Imported.Movie.mkv");
            std::os::unix::fs::symlink(source, &dest).expect("symlink");
            dest
        });
        assert!(!accepted, "a symlink to the source is not the import");
        assert_eq!((kept, through_dest), (bytes.clone(), bytes.clone()));

        // The source's own name, reached through a symlinked folder.
        let (accepted, kept, through_dest, _dir) = move_onto_existing(&|root, source| {
            let alias = root.join("library");
            std::os::unix::fs::symlink(source.parent().expect("parent"), &alias)
                .expect("folder symlink");
            alias.join("source.mkv")
        });
        assert!(!accepted, "the source's own entry is not the import");
        assert_eq!((kept, through_dest), (bytes.clone(), bytes.clone()));

        // A link to a link to the source.
        let (accepted, kept, through_dest, _dir) = move_onto_existing(&|root, source| {
            let hop = root.join("hop.mkv");
            std::os::unix::fs::symlink(source, &hop).expect("hop");
            let dest = library(root).join("Imported.Movie.mkv");
            std::os::unix::fs::symlink(&hop, &dest).expect("symlink");
            dest
        });
        assert!(
            !accepted,
            "a chain of links to the source is not the import"
        );
        assert_eq!((kept, through_dest), (bytes.clone(), bytes));
    }

    /// Another spelling of the source's own name, in its own folder: on a
    /// volume that does not tell case apart it is the source's entry, and the
    /// move is refused with both kept; on one that does, it is another file,
    /// and the move finishes as it did before any alias check.
    #[cfg(unix)]
    #[test]
    fn a_case_variant_destination_is_judged_by_the_volume() {
        let probe = tempfile::tempdir().expect("tempdir");
        std::fs::write(probe.path().join("probe"), b"").expect("probe");
        let folds_case = probe.path().join("PROBE").exists();

        let (accepted, kept, through_dest, _dir) = move_onto_existing(&|_, source| {
            let dest = source.with_file_name("SOURCE.mkv");
            if !folds_case {
                std::fs::write(&dest, b"only video bytes").expect("second file");
            }
            dest
        });
        let bytes = Some(b"only video bytes".to_vec());
        if folds_case {
            assert!(!accepted, "one entry under two spellings is not the import");
            assert_eq!((kept, through_dest), (bytes.clone(), bytes));
        } else {
            assert!(accepted, "another file under another case is the import");
            assert_eq!((kept, through_dest), (None, bytes));
        }
    }

    /// A destination that is another entry is the import, and the move
    /// finishes by removing the source name: a second name of the source
    /// file, and a symlink to another file with the same content.
    #[cfg(unix)]
    #[test]
    fn an_equal_destination_that_is_another_entry_finishes_the_move() {
        let library = |root: &Path| {
            let library = root.join("library");
            std::fs::create_dir_all(&library).expect("library");
            library
        };

        // A second name of the source file, as an interrupted hard-link move
        // leaves it.
        let (accepted, kept, through_dest, _dir) = move_onto_existing(&|root, source| {
            let dest = library(root).join("Imported.Movie.mkv");
            std::fs::hard_link(source, &dest).expect("hard link");
            dest
        });
        assert!(accepted, "a second name of the source is the import");
        assert_eq!(kept, None, "the source name is removed");
        assert_eq!(through_dest.as_deref(), Some(&b"only video bytes"[..]));

        // A symlink to an unrelated file with the same content, as a library
        // built from symlinks holds.
        let (accepted, kept, through_dest, dir) = move_onto_existing(&|root, _| {
            let elsewhere = root.join("elsewhere.mkv");
            std::fs::write(&elsewhere, b"only video bytes").expect("elsewhere");
            let dest = library(root).join("Imported.Movie.mkv");
            std::os::unix::fs::symlink(&elsewhere, &dest).expect("symlink");
            dest
        });
        assert!(accepted, "a link to another equal file is the import");
        assert_eq!(kept, None, "the source name is removed");
        assert_eq!(through_dest.as_deref(), Some(&b"only video bytes"[..]));
        assert_eq!(
            std::fs::read(dir.path().join("elsewhere.mkv")).expect("elsewhere"),
            b"only video bytes"
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

    // ── The download-client copy path proves what it copied ─────────

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
        // them — not a second read. Compared against the same hasher fed
        // directly, so a copy loop that dropped or double-fed a buffer would
        // show up here.
        let mut expected = scryer_application::location::verify::StreamedContentHasher::new();
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

    /// A byte flipped after write is caught before the source is
    /// touched. The import fails, so there is no cleanup guard and no path to
    /// source removal — and the source is still there.
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

    /// Full is requested, the read-back cannot run, and verification
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

    /// A same-filesystem placement copies no bytes, so there is nothing
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
    fn mapped_destination_ownership_accepts_regular_placement_lanes() {
        for prepared_lane in [false, true] {
            for move_mode in [false, true] {
                for scenario in [
                    "hardlink",
                    "cross_device",
                    "foreign_source",
                    "unverifiable_link",
                ] {
                    let dir = tempfile::tempdir().expect("tempdir");
                    assert!(
                        hard_links_supported(dir.path()),
                        "fixture requires hard links"
                    );
                    let source = dir.path().join("source.mkv");
                    let sibling = dir.path().join("unrelated.mkv");
                    let dest = dir.path().join("library/title.mkv");
                    std::fs::write(&source, SYNTHETIC_PAYLOAD).expect("write source");
                    std::fs::write(&sibling, b"unrelated").expect("write sibling");
                    let destination_uid = effective_uid() ^ 1;
                    let options = ImportFileOptions {
                        destination_uid_override: Some(destination_uid),
                        force_cross_device_move: scenario == "cross_device",
                        force_foreign_source_uid: scenario == "foreign_source",
                        force_hard_link_identity_mismatch: scenario == "unverifiable_link",
                        ..Default::default()
                    };
                    let (result, logs) = capture_logs(|| {
                        if prepared_lane {
                            let coordinator = ImportPlacementCoordinator::new(1, 2);
                            let (result, mismatch) = place_in_process(
                                &coordinator,
                                prepare_in_process(&source, &dest, options, move_mode),
                                false,
                            );
                            assert_eq!(mismatch.is_some(), scenario == "unverifiable_link");
                            result
                        } else {
                            import_file_blocking(
                                source.clone(),
                                dest.clone(),
                                if move_mode {
                                    ImportMode::Move
                                } else {
                                    ImportMode::HardlinkOrCopy
                                },
                                options,
                                None,
                                None,
                                ImportFilePermissions::default(),
                            )
                            .expect("mapped ownership must not reject placement")
                        }
                    });
                    assert!(
                        logs.contains(&format!("destination_uid={destination_uid}")),
                        "{logs}"
                    );
                    assert!(
                        logs.contains(&format!("process_uid={}", effective_uid())),
                        "{logs}"
                    );
                    assert_eq!(
                        result.strategy,
                        if scenario == "hardlink" {
                            ImportStrategy::HardLink
                        } else {
                            ImportStrategy::Copy
                        }
                    );
                    assert_eq!(result.verification.is_some(), scenario != "hardlink");
                    assert_eq!(result.source_disposition, ImportSourceDisposition::Retained);
                    assert_eq!(
                        std::fs::read(&source).expect("source before cleanup"),
                        SYNTHETIC_PAYLOAD
                    );
                    assert_eq!(result.source_cleanup.is_some(), move_mode);
                    if let Some(guard) = result.source_cleanup {
                        remove_import_source_after_verified_import_blocking(
                            guard,
                            dest.clone(),
                            options,
                        )
                        .expect("guarded cleanup");
                        assert!(!source.exists());
                    }
                    assert_eq!(
                        std::fs::read(&dest).expect("destination"),
                        SYNTHETIC_PAYLOAD
                    );
                    assert_eq!(std::fs::read(&sibling).expect("sibling"), b"unrelated");
                }
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn mapped_destination_ownership_accepts_archive_rename() {
        for move_mode in [false, true] {
            let dir = tempfile::tempdir().expect("tempdir");
            let (source, dest) = archive_workspace_fixture(dir.path());
            let sibling = source.with_file_name("unrelated.mkv");
            std::fs::write(&sibling, b"unrelated").expect("write sibling");
            let destination_uid = effective_uid() ^ 1;
            let options = ImportFileOptions {
                destination_uid_override: Some(destination_uid),
                ..Default::default()
            };
            let coordinator = ImportPlacementCoordinator::new(1, 2);
            let attempts_before = hard_link_attempts();
            let ((result, _), logs) = capture_logs(|| {
                place_in_process(
                    &coordinator,
                    prepare_in_process(&source, &dest, options, move_mode),
                    true,
                )
            });
            assert!(
                logs.contains(&format!("destination_uid={destination_uid}")),
                "{logs}"
            );
            assert_eq!(result.strategy, ImportStrategy::Move);
            assert_eq!(
                result.source_disposition,
                ImportSourceDisposition::RenamedIntoPlace
            );
            assert!(result.source_cleanup.is_none());
            assert_eq!(hard_link_attempts(), attempts_before);
            assert!(!source.exists());
            assert_eq!(
                std::fs::read(&dest).expect("destination"),
                SYNTHETIC_PAYLOAD
            );
            assert_eq!(std::fs::read(&sibling).expect("sibling"), b"unrelated");
        }
    }

    #[cfg(unix)]
    #[test]
    fn mapped_destination_ownership_keeps_permission_failures_best_effort() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mkv");
        let dest = dir.path().join("library/title.mkv");
        std::fs::write(&source, SYNTHETIC_PAYLOAD).expect("write source");
        let (result, logs) = capture_logs(|| {
            import_file_blocking(
                source.clone(),
                dest.clone(),
                ImportMode::HardlinkOrCopy,
                ImportFileOptions {
                    force_cross_device_move: true,
                    destination_uid_override: Some(effective_uid() ^ 1),
                    ..Default::default()
                },
                None,
                None,
                ImportFilePermissions {
                    set_permissions_linux: true,
                    file_chmod: Some("invalid".to_string()),
                    ..Default::default()
                },
            )
        });
        assert!(
            result
                .expect("permissions remain best effort")
                .source_cleanup
                .is_none()
        );
        assert!(logs.contains("invalid chmod mask"), "{logs}");
        assert_eq!(std::fs::read(&source).expect("source"), SYNTHETIC_PAYLOAD);
        assert_eq!(
            std::fs::read(&dest).expect("destination"),
            SYNTHETIC_PAYLOAD
        );
    }

    #[cfg(unix)]
    #[test]
    fn mapped_destination_ownership_does_not_bypass_verification_or_cleanup_guards() {
        for failure in ["verification", "source_changed", "destination_changed"] {
            let dir = tempfile::tempdir().expect("tempdir");
            let source = dir.path().join("source.mkv");
            let sibling = dir.path().join("unrelated.mkv");
            let dest = dir.path().join("library/title.mkv");
            std::fs::write(&source, SYNTHETIC_PAYLOAD).expect("write source");
            std::fs::write(&sibling, b"unrelated").expect("write sibling");
            let options = ImportFileOptions {
                force_cross_device_move: true,
                force_copy_verification_failure: failure == "verification",
                destination_uid_override: Some(effective_uid() ^ 1),
                ..Default::default()
            };
            let result = import_file_blocking(
                source.clone(),
                dest.clone(),
                ImportMode::Move,
                options,
                None,
                None,
                ImportFilePermissions::default(),
            );
            if failure == "verification" {
                assert!(
                    result
                        .expect_err("verification must fail")
                        .to_string()
                        .contains("copy verification failed")
                );
                assert!(!dest.exists());
                assert_eq!(std::fs::read(&source).expect("source"), SYNTHETIC_PAYLOAD);
            } else {
                let guard = result
                    .expect("placement")
                    .source_cleanup
                    .expect("cleanup guard");
                let mut changed = SYNTHETIC_PAYLOAD.to_vec();
                changed[0] ^= 1;
                let changed_path = if failure == "source_changed" {
                    &source
                } else {
                    &dest
                };
                std::fs::write(changed_path, &changed).expect("change guarded file");
                let error = remove_import_source_after_verified_import_blocking(
                    guard,
                    dest.clone(),
                    options,
                )
                .expect_err("changed content must prevent cleanup");
                assert!(error.to_string().contains(if failure == "source_changed" {
                    "source"
                } else {
                    "destination proof changed"
                }));
                assert_eq!(
                    std::fs::read(&source).expect("source retained"),
                    if failure == "source_changed" {
                        changed.as_slice()
                    } else {
                        SYNTHETIC_PAYLOAD
                    }
                );
                assert_eq!(
                    std::fs::read(&dest).expect("destination retained"),
                    if failure == "destination_changed" {
                        changed.as_slice()
                    } else {
                        SYNTHETIC_PAYLOAD
                    }
                );
            }
            assert_eq!(std::fs::read(&sibling).expect("sibling"), b"unrelated");
        }
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

    /// The copy lane alone, on a source read just before it.
    fn copy_alone(
        source: &Path,
        dest: &Path,
        size: u64,
        before_copy: impl FnOnce(),
    ) -> (
        AppResult<ImportVerification>,
        Vec<ImportFileTransferProgress>,
    ) {
        let (fingerprint, mark) = read_import_source(source).expect("read source");
        before_copy();
        let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel();
        let result = copy_regular_source_to_destination(
            source,
            dest,
            &fingerprint,
            &mark,
            size,
            ImportFileOptions::default(),
            VerificationDepth::Quick,
            Some(&progress_tx),
            None,
        )
        .map(|(verification, _)| verification);
        let mut updates = Vec::new();
        while let Ok(update) = progress_rx.try_recv() {
            updates.push(update);
        }
        (result, updates)
    }

    #[test]
    fn a_copy_reports_finalizing_when_its_stream_ends_whatever_size_was_expected() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("sample-source.mkv");
        std::fs::write(&source, b"synthetic video bytes").expect("write source");
        let dest = temp.path().join("Sample.Title.mkv");
        let real = std::fs::metadata(&source).expect("source metadata").len();

        let (result, updates) = copy_alone(&source, &dest, real + 7, || {});

        result.expect_err("the size expected is not the size copied");
        let seen: Vec<_> = updates
            .iter()
            .map(|update| (update.phase, update.bytes))
            .collect();
        assert_eq!(
            seen,
            [
                (ImportTransferPhase::Copying, 0),
                (ImportTransferPhase::Copying, real),
                (ImportTransferPhase::Finalizing, real),
            ]
        );
    }

    #[test]
    fn a_copy_that_cannot_stage_reports_no_progress() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("sample-source.mkv");
        std::fs::write(&source, b"synthetic video bytes").expect("write source");
        let dest = temp.path().join("no-such-folder").join("Sample.Title.mkv");

        let (result, updates) = copy_alone(&source, &dest, 21, || {});

        result.expect_err("the staged file cannot be made");
        assert_eq!(updates.len(), 0, "{updates:?}");
    }

    #[cfg(unix)]
    #[test]
    fn a_source_replaced_by_a_dangling_link_is_a_file_not_found() {
        use scryer_application::error_reasons::import;
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("sample-source.mkv");
        std::fs::write(&source, b"synthetic video bytes").expect("write source");
        let dest = temp.path().join("Sample.Title.mkv");

        let (result, _) = copy_alone(&source, &dest, 21, || {
            std::fs::remove_file(&source).expect("remove source");
            std::os::unix::fs::symlink(temp.path().join("sample-gone.mkv"), &source)
                .expect("link source");
        });

        let error = result.expect_err("the source is gone");
        assert_eq!(error.reason(), Some(import::FILE_NOT_FOUND));
        assert!(!dest.exists() && !scryer_application::import_staging_path(&dest).exists());
    }

    #[test]
    fn a_source_replaced_by_a_folder_is_not_a_regular_file() {
        use scryer_application::error_reasons::import;
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("sample-source.mkv");
        std::fs::write(&source, b"synthetic video bytes").expect("write source");
        let dest = temp.path().join("Sample.Title.mkv");

        let (result, _) = copy_alone(&source, &dest, 21, || {
            std::fs::remove_file(&source).expect("remove source");
            std::fs::create_dir(&source).expect("folder at source");
        });

        let error = result.expect_err("the source is a folder");
        assert_eq!(error.reason(), Some(import::IO_FAILED));
        let said = error.to_string();
        assert!(
            said.contains("source preflight: repository: import path is not a regular file"),
            "{said}"
        );
        assert!(!dest.exists() && !scryer_application::import_staging_path(&dest).exists());
    }

    /// What the disk service says of a copy that did not land, as an import
    /// words and codes it.
    #[test]
    fn a_refused_copy_is_worded_and_coded_by_where_it_stopped() {
        use scryer_application::error_reasons::import;
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("sample-source.mkv");
        std::fs::write(&source, b"synthetic video bytes").expect("write source");
        let dest = temp.path().join("Sample.Title.mkv");
        let staged = scryer_application::import_staging_path(&dest);
        let fingerprint = fingerprint_import_source(&source).expect("read source");
        let refused = |step, cause, attempts| {
            let refusal = StagedCopyRefusal {
                step,
                cause,
                attempts,
                left: None,
            };
            import_copy_refusal(&source, &dest, &staged, &fingerprint, refusal)
        };
        let io = |kind: io::ErrorKind| StagedCause::Io(kind.into());
        let raw = |code| StagedCause::Io(io::Error::from_raw_os_error(code));

        let dropped = refused(StagedStep::Transfer, raw(9), 3);
        let said = dropped.to_string();
        assert!(
            said.contains("import copy failed after 3 attempt(s)"),
            "{said}"
        );
        #[cfg(unix)]
        assert!(said.contains("copy: Bad file descriptor"), "{said}");
        assert_eq!(dropped.reason(), Some(import::IO_FAILED));

        let said = refused(StagedStep::Transfer, raw(5), 1).to_string();
        assert!(
            said.contains("import copy failed after 1 attempt(s)"),
            "{said}"
        );

        let missing = refused(StagedStep::SourceBefore, io(io::ErrorKind::NotFound), 1);
        assert_eq!(missing.reason(), Some(import::FILE_NOT_FOUND));
        let full = refused(StagedStep::Transfer, io(io::ErrorKind::StorageFull), 1);
        assert_eq!(full.reason(), Some(import::DISK_FULL));

        let uncleared = refused(
            StagedStep::ClearStaged,
            io(io::ErrorKind::PermissionDenied),
            2,
        );
        let said = uncleared.to_string();
        assert!(
            said.contains("import copy failed before attempt 2"),
            "{said}"
        );
        assert_eq!(uncleared.reason(), Some(import::PERMISSION_DENIED));

        assert!(refused(StagedStep::Transfer, StagedCause::Cancelled, 1).is_canceled());

        // The source changed and changed back: the import's own read passes,
        // and the copy is still refused.
        let restored = refused(StagedStep::SourceAfter, StagedCause::SourceChanged, 1);
        let said = restored.to_string();
        assert!(
            said.contains("source verification: repository: import source changed during import"),
            "{said}"
        );
        assert_eq!(restored.reason(), Some(import::IO_FAILED));

        let not_a_file = refused(StagedStep::SourceHandle, StagedCause::SourceNotAFile, 1);
        let said = not_a_file.to_string();
        assert!(
            said.contains("source validation: repository: import path is not a regular file"),
            "{said}"
        );
        assert_eq!(not_a_file.reason(), Some(import::IO_FAILED));
    }

    /// A copy that holds the destination name and then finds the source
    /// changed takes back that copy, and nothing beside it.
    #[test]
    fn a_source_that_changes_once_its_copy_landed_takes_the_copy_back() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("sample-source.mkv");
        let dest = temp.path().join("Sample.Title.mkv");
        let neighbour = temp.path().join("Sample.Other.mkv");
        std::fs::write(&source, b"synthetic video bytes").expect("write source");
        std::fs::write(&neighbour, b"unrelated").expect("write neighbour");
        let (fingerprint, mark) = read_import_source(&source).expect("read source");
        let size = std::fs::metadata(&source).expect("source metadata").len();
        let options = ImportFileOptions {
            after_copy_promoted: Some(|source, _| {
                std::fs::write(source, b"synthetic bytes, rewritten and longer")
                    .expect("rewrite source");
            }),
            ..Default::default()
        };

        let error = copy_regular_source_to_destination(
            &source,
            &dest,
            &fingerprint,
            &mark,
            size,
            options,
            VerificationDepth::Quick,
            None,
            None,
        )
        .expect_err("the source changed");

        let said = error.to_string();
        assert!(
            said.contains("import source changed during import"),
            "{said}"
        );
        assert!(!dest.exists(), "the copy this import landed is taken back");
        assert!(!scryer_application::import_staging_path(&dest).exists());
        assert_eq!(std::fs::read(&neighbour).expect("neighbour"), b"unrelated");
        assert_eq!(
            std::fs::read(&source).expect("source"),
            b"synthetic bytes, rewritten and longer"
        );
    }

    /// A file that took the destination name after the copy landed there is
    /// not this import's copy, so a failing check after the promotion leaves
    /// it where it is.
    #[test]
    fn a_failing_check_after_the_copy_landed_never_removes_a_file_that_replaced_it() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("sample-source.mkv");
        let dest = temp.path().join("Sample.Title.mkv");
        std::fs::write(&source, b"synthetic video bytes").expect("write source");
        let (fingerprint, mark) = read_import_source(&source).expect("read source");
        let size = std::fs::metadata(&source).expect("source metadata").len();
        let options = ImportFileOptions {
            after_copy_promoted: Some(|source, dest| {
                let other = dest.with_file_name("Sample.Other.mkv");
                std::fs::write(&other, b"someone else's file").expect("write other");
                std::fs::rename(&other, dest).expect("replace the copy");
                std::fs::write(source, b"synthetic bytes, rewritten and longer")
                    .expect("rewrite source");
            }),
            ..Default::default()
        };

        let error = copy_regular_source_to_destination(
            &source,
            &dest,
            &fingerprint,
            &mark,
            size,
            options,
            VerificationDepth::Quick,
            None,
            None,
        )
        .expect_err("the source changed");

        assert!(
            error
                .to_string()
                .contains("import source changed during import"),
            "{error}"
        );
        assert_eq!(
            std::fs::read(&dest).expect("the replacing file is kept"),
            b"someone else's file"
        );
    }

    /// A move whose cleanup guard cannot be built after the copy was
    /// verified takes back only the copy it made: a file another writer put
    /// at the destination name since is kept, and so is the source.
    #[test]
    fn a_failing_cleanup_guard_never_removes_a_file_that_replaced_the_copy() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("sample-source.mkv");
        let dest = temp.path().join("Sample.Title.mkv");
        std::fs::write(&source, b"synthetic video bytes").expect("write source");

        import_file_blocking_at_depth(
            source.clone(),
            dest.clone(),
            ImportMode::Move,
            ImportFileOptions {
                force_cross_device_move: true,
                after_copy_verified: Some(|_, dest| {
                    let other = dest.with_file_name("Sample.Other.mkv");
                    std::fs::write(&other, b"someone else's file").expect("write other");
                    std::fs::rename(&other, dest).expect("replace the copy");
                }),
                ..Default::default()
            },
            VerificationDepth::Quick,
            None,
            None,
            ImportFilePermissions::default(),
        )
        .expect_err("the destination no longer matches the source");

        assert_eq!(
            std::fs::read(&dest).expect("the replacing file is kept"),
            b"someone else's file"
        );
        assert_eq!(
            std::fs::read(&source).expect("the source is kept"),
            b"synthetic video bytes"
        );
    }

    /// A move whose source is removed by someone else once the copy was
    /// verified keeps the copy: it is the only one left. So does one whose
    /// source name was made to hold another file: the copy is the last copy
    /// of what the name held.
    #[test]
    fn a_copy_whose_source_vanished_is_kept_as_the_last_copy() {
        let gone: fn(&Path, &Path) = |source, _| {
            std::fs::remove_file(source).expect("remove source");
        };
        let replaced: fn(&Path, &Path) = |source, _| {
            let other = source.with_file_name("sample-other.mkv");
            std::fs::write(&other, b"another file").expect("write other");
            std::fs::rename(&other, source).expect("replace source");
        };
        for after_copy_verified in [gone, replaced] {
            let temp = tempfile::tempdir().expect("tempdir");
            let source = temp.path().join("sample-source.mkv");
            let dest = temp.path().join("Sample.Title.mkv");
            std::fs::write(&source, b"synthetic video bytes").expect("write source");

            let error = import_file_blocking_at_depth(
                source.clone(),
                dest.clone(),
                ImportMode::Move,
                ImportFileOptions {
                    force_cross_device_move: true,
                    after_copy_verified: Some(after_copy_verified),
                    ..Default::default()
                },
                VerificationDepth::Quick,
                None,
                None,
                ImportFilePermissions::default(),
            )
            .expect_err("the source no longer holds the file");

            use scryer_application::error_reasons::import;
            assert_eq!(error.reason(), Some(import::COPY_KEPT_SOURCE_GONE));
            assert_eq!(
                error.refusal_facts(),
                [RefusalFact {
                    key: import::KEPT_PATH_FACT,
                    value: RefusalFactValue::Text(dest.display().to_string()),
                }]
            );
            assert_eq!(
                std::fs::read(&dest).expect("the copy is kept"),
                b"synthetic video bytes"
            );
        }
    }

    /// A copy that failed its proof is kept at its final name when the
    /// source then does not read: nothing says the source still holds the
    /// data, so the copy may be the only one. The source is untouched.
    #[cfg(unix)]
    #[test]
    fn a_copy_that_failed_its_proof_is_kept_when_the_source_does_not_read() {
        use std::os::unix::fs::PermissionsExt;

        /// Opens the folder again when dropped, so a failing assertion never
        /// leaves the temporary folder undeletable.
        struct Reopen(PathBuf);
        impl Drop for Reopen {
            fn drop(&mut self) {
                let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
            }
        }

        let temp = tempfile::tempdir().expect("tempdir");
        let downloads = temp.path().join("downloads");
        std::fs::create_dir_all(&downloads).expect("downloads");
        let source = downloads.join("sample-source.mkv");
        let dest = temp.path().join("library/Sample.Title.mkv");
        std::fs::create_dir_all(dest.parent().unwrap()).expect("library");
        std::fs::write(&source, b"synthetic video bytes").expect("write source");

        // A process that reads through a closed folder anyway, as root does,
        // cannot make the source fail to read; the test would prove nothing.
        let reopen = Reopen(downloads.clone());
        std::fs::set_permissions(&downloads, std::fs::Permissions::from_mode(0o000))
            .expect("close the source folder");
        let still_reads = std::fs::symlink_metadata(&source).is_ok();
        std::fs::set_permissions(&downloads, std::fs::Permissions::from_mode(0o755))
            .expect("reopen the source folder");
        if still_reads {
            return;
        }

        let error = import_file_blocking_at_depth(
            source.clone(),
            dest.clone(),
            ImportMode::Move,
            ImportFileOptions {
                force_cross_device_move: true,
                force_destination_corruption: true,
                after_copy_assessed: Some(|source, _| {
                    let folder = source.parent().unwrap();
                    std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o000))
                        .expect("close the source folder");
                }),
                ..Default::default()
            },
            VerificationDepth::Full,
            None,
            None,
            ImportFilePermissions::default(),
        )
        .expect_err("a corrupted copy must not be imported");
        drop(reopen);

        assert!(
            error.to_string().contains("copy verification failed"),
            "unexpected error: {error}"
        );
        assert!(
            dest.exists(),
            "the copy is kept while the source does not read"
        );
        assert_eq!(
            std::fs::read(&source).expect("the source is untouched"),
            b"synthetic video bytes"
        );
    }

    /// A hard link whose source name is removed, or made to name another
    /// file, before the source is read again is the last name of the data,
    /// so a failing import keeps it, in one process and on the fast lane.
    #[cfg(unix)]
    #[test]
    fn a_hard_link_whose_source_name_went_is_kept_as_the_last_name() {
        let gone: fn(&Path, &Path) = |source, _| {
            std::fs::remove_file(source).expect("remove source");
        };
        let replaced: fn(&Path, &Path) = |source, _| {
            let other = source.with_file_name("sample-other.mkv");
            std::fs::write(&other, b"another file").expect("write other");
            std::fs::rename(&other, source).expect("replace source");
        };
        for (after_hard_link, fast) in [
            (gone, false),
            (replaced, false),
            (gone, true),
            (replaced, true),
        ] {
            let temp = tempfile::tempdir().expect("tempdir");
            let source = temp.path().join("sample-source.mkv");
            let dest = temp.path().join("library/Sample.Title.mkv");
            std::fs::create_dir_all(dest.parent().unwrap()).expect("library");
            std::fs::write(&source, b"synthetic video bytes").expect("write source");
            let options = ImportFileOptions {
                after_hard_link: Some(after_hard_link),
                ..Default::default()
            };
            let error = if fast {
                let prepared = prepare_import_placement_blocking(
                    source.clone(),
                    dest.clone(),
                    options,
                    VerificationDepth::default(),
                    true,
                    None,
                    None,
                    ImportFilePermissions::default(),
                )
                .expect("prepare placement");
                match try_fast_import_placement_blocking(prepared) {
                    Err(error) => error,
                    Ok(_) => panic!("the source changed"),
                }
            } else {
                import_file_blocking_at_depth(
                    source.clone(),
                    dest.clone(),
                    ImportMode::Move,
                    options,
                    VerificationDepth::default(),
                    None,
                    None,
                    ImportFilePermissions::default(),
                )
                .expect_err("the source changed")
            };
            assert_eq!(
                std::fs::read(&dest).expect("the link is kept"),
                b"synthetic video bytes",
                "fast={fast}: {error}"
            );
        }
    }

    fn import_in_one_process(
        source: &Path,
        dest: &Path,
        options: ImportFileOptions,
    ) -> AppResult<ImportFileResult> {
        import_hardlink_or_copy_blocking(
            source.to_path_buf(),
            dest.to_path_buf(),
            options,
            VerificationDepth::default(),
            false,
            None,
            None,
            ImportFilePermissions::default(),
        )
    }

    fn fast_lane_error(source: &Path, dest: &Path, options: ImportFileOptions) -> AppError {
        let coordinator = ImportPlacementCoordinator::new(1, 2);
        let mut prepared = prepare_in_process(source, dest, options, false);
        coordinator.ready_fast_placement(&mut prepared, false);
        match try_fast_import_placement_blocking(prepared) {
            Err(error) => error,
            Ok(_) => panic!("the fast lane must refuse"),
        }
    }

    /// The link did not prove and was gone before it could be removed. The
    /// single-process lane copies instead; the fast lane refuses, as it
    /// cannot tell what took the name.
    #[test]
    fn an_unproven_link_that_is_already_gone_answers_by_lane() {
        let options = ImportFileOptions {
            link_answer: LinkAnswer::AlreadyGone,
            ..Default::default()
        };
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("sample-source.mkv");
        let dest = temp.path().join("library/Sample.Title.mkv");
        std::fs::write(&source, b"synthetic video bytes").expect("write source");

        let result = import_in_one_process(&source, &dest, options).expect("the copy lands");
        assert_eq!(result.strategy, ImportStrategy::Copy);
        assert_eq!(
            std::fs::read(&dest).expect("dest"),
            b"synthetic video bytes"
        );
        assert_eq!(
            std::fs::read(&source).expect("source"),
            b"synthetic video bytes"
        );

        let dest = temp.path().join("library/Sample.Second.mkv");
        let said = fast_lane_error(&source, &dest, options).to_string();
        assert!(
            said.contains("failed to remove partial hardlink destination"),
            "{said}"
        );
        assert!(!dest.exists());
        assert_eq!(
            std::fs::read(&source).expect("source"),
            b"synthetic video bytes"
        );
    }

    /// The source was swapped and another writer's file holds the name, which
    /// the disk service left alone. Both lanes say the source changed, and
    /// neither touches that file.
    #[test]
    fn a_link_left_alone_beside_a_swapped_source_is_a_changed_source() {
        let options = ImportFileOptions {
            link_answer: LinkAnswer::LeftAloneAfterSourceSwap,
            ..Default::default()
        };
        for fast in [false, true] {
            let temp = tempfile::tempdir().expect("tempdir");
            let source = temp.path().join("sample-source.mkv");
            let dest = temp.path().join("library/Sample.Title.mkv");
            std::fs::write(&source, b"synthetic video bytes").expect("write source");

            let error = match fast {
                true => fast_lane_error(&source, &dest, options),
                false => import_in_one_process(&source, &dest, options)
                    .err()
                    .expect("the source changed"),
            };
            let said = error.to_string();
            assert!(
                said.contains("import source changed during import"),
                "fast {fast}: {said}"
            );
            assert!(
                !said.contains("is not a regular file"),
                "fast {fast}: {said}"
            );
            assert_eq!(std::fs::read(&dest).expect("dest"), b"another writer");
            assert_eq!(
                std::fs::read(&source).expect("source"),
                b"a different, longer payload"
            );
        }
    }

    /// Another writer's file took the name after the link was made, and the
    /// disk service left it alone. Both lanes answer as for an occupied name:
    /// nothing is copied over that file, and the source stays.
    #[test]
    fn a_link_left_alone_for_another_writer_is_an_occupied_name() {
        let options = ImportFileOptions {
            link_answer: LinkAnswer::LeftAlone,
            ..Default::default()
        };
        for fast in [false, true] {
            let temp = tempfile::tempdir().expect("tempdir");
            let source = temp.path().join("sample-source.mkv");
            let dest = temp.path().join("library/Sample.Title.mkv");
            std::fs::write(&source, b"synthetic video bytes").expect("write source");

            let outcome = match fast {
                true => Err(fast_lane_error(&source, &dest, options)),
                false => import_in_one_process(&source, &dest, options),
            };
            let error = outcome.err().expect("the name is occupied");
            assert_eq!(
                error.reason(),
                Some(scryer_application::error_reasons::import::DESTINATION_OCCUPIED),
                "fast {fast}: {error}"
            );
            assert_eq!(
                std::fs::read(&dest).expect("dest"),
                b"another writer",
                "fast {fast}"
            );
            assert_eq!(
                std::fs::read(&source).expect("source"),
                b"synthetic video bytes"
            );
        }
    }

    #[test]
    fn move_mode_copy_failure_leaves_source() {
        let source_dir = tempfile::tempdir().expect("source tempdir");
        let dest_dir = tempfile::tempdir().expect("dest tempdir");
        let source = source_dir.path().join("source.mkv");
        std::fs::write(&source, b"fake video bytes").expect("write source");
        let dest = dest_dir.path().join("Imported.Movie.mkv");
        std::fs::create_dir(scryer_application::import_staging_path(&dest))
            .expect("create temp conflict");

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

        let guard = result.source_cleanup.expect("cleanup guard");
        let error = remove_import_source_after_verified_import_blocking(
            guard.clone(),
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
        remove_import_source_after_verified_import_blocking(
            guard,
            dest.clone(),
            ImportFileOptions::default(),
        )
        .expect("cleanup retry must converge after the transient failure");
        assert!(!source.exists());
        assert_eq!(std::fs::read(&dest).unwrap(), b"fake video bytes");
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

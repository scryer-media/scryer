use crate::disk::reason::{DISK_DESTINATION_EXISTS, DISK_PATH_MISSING};
use crate::disk::{
    Access, Check, Disk, DiskError, Done, EntryKind, FreeNameMove, Open, Parents, PathReading,
    Placed, Refused, Removal, Remove, Replace, Visited, placement,
};
use crate::location::model::VerificationDepth;
use crate::location::verify::{
    CopyProgress, DestinationClaim, copy_and_verify, copy_and_verify_reporting,
};
use crate::{AppError, AppResult};

fn repository(error: DiskError) -> AppError {
    AppError::Repository(error.to_string())
}

fn unreadable(path: &Path, reading: PathReading) -> AppError {
    AppError::Repository(format!(
        "{}: {}",
        path.display(),
        reading.describe_absence()
    ))
}

/// The kind `reading` shows at `path`; an error when it shows none.
fn known_kind(path: &Path, reading: PathReading) -> AppResult<EntryKind> {
    reading.kind().ok_or_else(|| unreadable(path, reading))
}
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

pub const RECYCLE_MANIFEST_SCHEMA: &str = "scryer.recycle-entry.v1";
pub const RECYCLE_STATUS_PENDING: &str = "pending";
/// Only the owning conflict journal may commit these entries; stale-entry
/// reconciliation and retention must leave interrupted/ambiguous staging alone.
pub const RECYCLE_STATUS_CONFLICT_PENDING: &str = "conflict_pending";
pub const RECYCLE_STATUS_COMMITTED: &str = "committed";
pub const RECYCLE_STATUS_QUARANTINED: &str = "quarantined";

const RECYCLE_ROOT_SENTINEL: &str = ".scryer-recycle-root";
const DEFAULT_RETENTION_DAYS: u32 = 7;
pub(crate) const RECYCLE_DIR_NAME: &str = ".scryer-recycle";

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
    /// The episodes the recycled file covered, sorted, when an upgrade of an
    /// episode file recycled it. Entries written before this was recorded,
    /// and entries of movies, have none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode_ids: Option<Vec<String>>,
    /// Decision-time columns of the media row that tracked the recycled file.
    /// A rescan of the restored file can re-parse its name and re-probe its
    /// streams, but it cannot recover what the file scored when it was grabbed,
    /// so a restore copies these back onto the recreated row. Entries recycled
    /// before this was recorded have none; their restored rows stay unscored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_row: Option<RecycledMediaRowSnapshot>,
}

/// The parts of a `media_files` row that were decided at acquisition time and
/// that a library scan cannot recompute from the file on disk.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecycledMediaRowSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acquisition_score: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scoring_log: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub announced_size_bytes: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indexer_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grabbed_release_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grabbed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edition: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_hash: Option<String>,
    /// The frozen indexer listing the grab read, as the opaque JSON the row
    /// stored. Manifests written before this was recorded have none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_listing_json: Option<String>,
}

impl RecycledMediaRowSnapshot {
    pub fn from_media_file(file: &crate::TitleMediaFile) -> Self {
        Self {
            acquisition_score: file.acquisition_score,
            scoring_log: file.scoring_log.clone(),
            announced_size_bytes: file.announced_size_bytes,
            scene_name: file.scene_name.clone(),
            release_group: file.release_group.clone(),
            indexer_source: file.indexer_source.clone(),
            grabbed_release_title: file.grabbed_release_title.clone(),
            grabbed_at: file.grabbed_at.clone(),
            edition: file.edition.clone(),
            release_hash: file.release_hash.clone(),
            release_listing_json: file.release_listing_json.clone(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

/// Result of a successful recycle operation.
#[derive(Debug, Clone)]
pub struct RecycleResult {
    pub entry_id: String,
    pub entry_dir: PathBuf,
    pub recycled_path: PathBuf,
    pub manifest_path: PathBuf,
}

/// A reserved entry whose path is durable before a conflict disposes its source.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreparedRecycleFile {
    pub entry_id: String,
    pub original_path: String,
    pub recycled_path: String,
    pub source_operation_id: String,
}

/// The receipt of an operator-confirmed recycle of a reported file: the
/// reported file's reserved entry, and the entry reserved for its
/// hard-linked download copy once the copy's move is about to run. Saved
/// before each move, so a retry knows both payload paths.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaIssueRecycleReceipt {
    #[serde(flatten)]
    pub file: PreparedRecycleFile,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linked_copy: Option<PreparedRecycleFile>,
}

pub(crate) struct ReplacedMediaRecycleMetadata<'a> {
    pub original_path: &'a str,
    pub original_file_id: &'a str,
    pub size_bytes: u64,
    pub title_id: &'a str,
    pub media_root: Option<&'a str>,
    pub media_row: Option<RecycledMediaRowSnapshot>,
    /// The episodes the replaced file covered; empty for a movie or when
    /// they are not known.
    pub episode_ids: Vec<String>,
}

/// A committed recycle entry that passed local recycle-root checks.
#[derive(Debug, Clone)]
pub struct CommittedRecycleEntry {
    pub entry_dir: PathBuf,
    pub manifest: RecycleManifest,
}

/// A quarantined entry, laid out as Scryer wrote it, of a file an upgrade
/// replaced. Its payload has not been looked at.
#[derive(Debug, Clone)]
pub(crate) struct QuarantinedUpgradeEntry {
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
        return Err(AppError::validation_refused(
            crate::error_reasons::library::RECYCLE_ENTRY_ID_NOT_VALID_OPAQUE_ID,
            "recycle entry id is not a valid opaque id",
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
            episode_ids: None,
            media_row: None,
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

    /// Why the file was recycled, without the `; quarantine: …` a quarantine
    /// adds after it.
    pub(crate) fn recycled_reason(&self) -> &str {
        self.reason.split(';').next().unwrap_or_default().trim()
    }

    pub fn original_path_buf(&self) -> PathBuf {
        crate::stored_paths::stored_path_to_path_buf(&self.original_path)
    }

    /// Whether the file was recycled before `cutoff`. An unreadable time
    /// never expires.
    pub(crate) fn recycled_before(&self, cutoff: chrono::DateTime<Utc>) -> bool {
        chrono::DateTime::parse_from_rfc3339(&self.recycled_at)
            .is_ok_and(|recycled_at| recycled_at.with_timezone(&Utc) < cutoff)
    }
}

fn manifest_path(entry_dir: &Path) -> PathBuf {
    entry_dir.join("manifest.json")
}

fn payload_relative_path(original: &Path) -> PathBuf {
    let name = original
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("unknown"));
    if name.to_str().is_some_and(|name| {
        name.trim_end_matches(['.', ' '])
            .eq_ignore_ascii_case("manifest.json")
    }) {
        Path::new(".scryer-payload").join(name)
    } else {
        PathBuf::from(name)
    }
}

fn prepared_payload_location(entry: &Path, original: &Path, payload: &Path) -> bool {
    entry.join(payload_relative_path(original)) == payload
        || original.file_name().is_some_and(|name| {
            entry.join(name) == payload || entry.join(".scryer-payload").join(name) == payload
        })
}

async fn payload_is_manifest(entry: &Path, payload: &Path) -> AppResult<bool> {
    let metadata = manifest_path(entry);
    if payload == metadata {
        return Ok(true);
    }
    let disk = Disk::process();
    match disk.canonicalize(payload).await {
        Ok(payload) => Ok(payload == disk.canonicalize(metadata).await.map_err(repository)?),
        Err(error) if error.reason() == DISK_PATH_MISSING => Ok(false),
        Err(error) => Err(repository(error)),
    }
}

async fn inspect_payload_location(entry: &Path, payload: &Path) -> AppResult<()> {
    let invalid = || {
        AppError::validation_refused(
            crate::error_reasons::library::REGULAR_FILE_REQUIRED,
            "The recycle payload path changed; its files were preserved",
        )
    };
    let disk = Disk::process();
    if !known_kind(entry, disk.uncached(entry).await)?.eq(&EntryKind::Directory) {
        return Err(invalid());
    }
    let relative = payload.strip_prefix(entry).map_err(|_| invalid())?;
    let canonical_entry = disk.canonicalize(entry).await.map_err(repository)?;
    crate::location::title_folder_recovery::inspect_path(&canonical_entry.join(relative), true)
        .await
}

/// Locate a new safe payload or a legacy direct-layout payload. Entry metadata
/// is never a payload, even when the original name aliases it on this filesystem.
pub(crate) async fn recycled_payload_path(
    entry: &Path,
    manifest: &RecycleManifest,
) -> AppResult<PathBuf> {
    let original = manifest.original_path_buf();
    let mapped = entry.join(payload_relative_path(&original));
    inspect_payload_location(entry, &mapped).await?;
    let reading = Disk::process().uncached(&mapped).await;
    match reading.exists() {
        Some(true) if !payload_is_manifest(entry, &mapped).await? => return Ok(mapped),
        Some(_) => {}
        None => return Err(unreadable(&mapped, reading)),
    }
    let legacy = entry.join(
        original
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new("unknown")),
    );
    inspect_payload_location(entry, &legacy).await?;
    if !payload_is_manifest(entry, &legacy).await? {
        return Ok(legacy);
    }
    let separated = entry.join(".scryer-payload").join(
        original
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new("unknown")),
    );
    inspect_payload_location(entry, &separated).await?;
    if Disk::process().uncached(&separated).await.exists() == Some(true)
        && !payload_is_manifest(entry, &separated).await?
    {
        return Ok(separated);
    }
    Err(AppError::validation_refused(
        crate::error_reasons::library::REGULAR_FILE_REQUIRED,
        "The recycled payload is unavailable; entry metadata was preserved",
    ))
}

/// The entries of the recycle root that `keep` accepts, kinds unread unless
/// `keep` reads them.
async fn recycle_root_entries(
    config: &RecycleBinConfig,
    keep: fn(&Visited) -> bool,
) -> AppResult<Vec<PathBuf>> {
    let entries = Disk::process().entries(&config.base_path, keep).await;
    let entries = entries
        .map_err(|error| AppError::Repository(format!("failed to read recycle bin: {error}")))?;
    Ok(entries.into_iter().map(Visited::into_path).collect())
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
        && Disk::process().follow_blocking(&config.base_path).exists() == Some(true)
        && Disk::process()
            .follow_blocking(&sentinel_path(config))
            .exists()
            == Some(true)
}

async fn ensure_recycle_root(config: &RecycleBinConfig) -> AppResult<()> {
    let disk = Disk::process();
    disk.create_dir(&config.base_path, Parents::Create)
        .await
        .map_err(|e| {
            AppError::Repository(format!(
                "failed to create recycle directory {}: {}",
                config.base_path.display(),
                e
            ))
        })?;

    let sentinel = sentinel_path(config);
    if disk.follow(&sentinel).await.exists() != Some(true) {
        let schema = RECYCLE_MANIFEST_SCHEMA.as_bytes().to_vec();
        disk.write(&sentinel, schema, Replace::Allowed, Access::Default)
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
    trusted_entry_layout(config, entry_dir, manifest)?;
    committed_status(manifest)
}

fn committed_status(manifest: &RecycleManifest) -> Result<(), String> {
    if !manifest.is_committed() {
        return Err("recycle entry is not committed".to_string());
    }
    Ok(())
}

/// Every check of [`trusted_committed_entry`] but the entry's status: the
/// directory is one Scryer generated, directly under the configured root,
/// and its manifest is current and names it.
fn trusted_entry_layout(
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
    let entry_kind = known_kind(entry_dir, Disk::process().uncached_blocking(entry_dir))
        .map_err(|error| format!("failed to stat recycle entry directory: {error}"))?;
    if entry_kind == EntryKind::Symlink {
        return Err("recycle entry directory is a symlink".to_string());
    }
    if entry_kind != EntryKind::Directory {
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
    // Read first: a missing manifest answers NotFound from the read itself,
    // so the common case costs one call. Any other failure falls back to the
    // existence check, which still decides "no manifest" (for example a
    // directory that cannot be searched) exactly as before.
    let disk = Disk::process();
    let manifest_bytes = match disk.read(&path).await {
        Ok(bytes) => bytes,
        Err(error) if error.reason() == DISK_PATH_MISSING => return Ok(None),
        Err(_) if disk.follow(&path).await.exists() != Some(true) => return Ok(None),
        Err(error) => return Err(repository(error)),
    };
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
    let bytes = manifest_json.into_bytes();
    (Disk::process()
        .write(&path, bytes, Replace::Allowed, Access::Default)
        .await)
        .map_err(repository)
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
    depth: VerificationDepth,
) -> AppResult<Option<RecycleResult>> {
    recycle_file_inner(
        config,
        source_path,
        source_path,
        manifest,
        true,
        RecycleStep::Move(depth),
    )
    .await
}

/// Moves a file into a pending recycle entry that retention housekeeping must
/// ignore until the caller has completed the surrounding move transaction.
pub(crate) async fn recycle_file_pending(
    config: &RecycleBinConfig,
    source_path: &Path,
    manifest: RecycleManifest,
    depth: VerificationDepth,
) -> AppResult<Option<RecycleResult>> {
    recycle_file_inner(
        config,
        source_path,
        source_path,
        manifest,
        false,
        RecycleStep::Move(depth),
    )
    .await
}

pub(crate) async fn recycle_replaced_media_file(
    config: &RecycleBinConfig,
    source_path: &Path,
    metadata: ReplacedMediaRecycleMetadata<'_>,
    commit_after_move: bool,
    depth: VerificationDepth,
) -> AppResult<Option<RecycleResult>> {
    let mut manifest = RecycleManifest::pending_upgrade(
        metadata.original_path.to_string(),
        metadata.original_file_id.to_string(),
        metadata.size_bytes,
        metadata.title_id.to_string(),
        metadata.media_root.map(str::to_string),
    );
    manifest.media_row = metadata.media_row;
    let mut episode_ids = metadata.episode_ids;
    episode_ids.sort();
    episode_ids.dedup();
    manifest.episode_ids = (!episode_ids.is_empty()).then_some(episode_ids);
    let payload_name_path = crate::stored_paths::stored_path_to_path_buf(metadata.original_path);
    recycle_file_inner(
        config,
        source_path,
        &payload_name_path,
        manifest,
        commit_after_move,
        RecycleStep::Move(depth),
    )
    .await
}

pub(crate) async fn finish_conflict_recycle(
    config: &RecycleBinConfig,
    prepared: &PreparedRecycleFile,
) -> AppResult<()> {
    let invalid = || {
        AppError::validation_refused(
            crate::error_reasons::library::RECYCLE_BIN_PATH_UNSAFE,
            "The reserved recycle entry changed; remaining files were preserved",
        )
    };
    let original = crate::stored_paths::stored_path_to_path_buf(&prepared.original_path);
    let payload = crate::stored_paths::stored_path_to_path_buf(&prepared.recycled_path);
    let entry = config.base_path.join(&prepared.entry_id);
    if !config.enabled
        || !config.cleanup_enabled
        || Path::new(&prepared.entry_id).components().count() != 1
        || !matches!(
            Path::new(&prepared.entry_id).components().next(),
            Some(std::path::Component::Normal(_))
        )
        || !prepared_payload_location(&entry, &original, &payload)
    {
        return Err(invalid());
    }
    ensure_recycle_root(config).await?;
    inspect_payload_location(&entry, &payload).await?;
    if payload_is_manifest(&entry, &payload).await? {
        return Err(invalid());
    }
    crate::location::title_folder_recovery::inspect_path(&payload, false).await?;
    let mut manifest = read_manifest(&entry).await?.ok_or_else(invalid)?;
    if manifest.entry_id.as_deref() != Some(&prepared.entry_id)
        || manifest.original_path != prepared.original_path
        || manifest.source_operation_id.as_deref() != Some(&prepared.source_operation_id)
        || !matches!(
            manifest.status.as_deref(),
            Some(RECYCLE_STATUS_CONFLICT_PENDING | RECYCLE_STATUS_COMMITTED)
        )
    {
        return Err(invalid());
    }
    let partial = crate::location::verify::partial_destination_path(&payload);
    if Disk::process().uncached(&partial).await.exists() != Some(false) {
        return Err(invalid());
    }
    if manifest.status.as_deref() == Some(RECYCLE_STATUS_CONFLICT_PENDING) {
        manifest.status = Some(RECYCLE_STATUS_COMMITTED.to_owned());
        manifest.recycled_at = Utc::now().to_rfc3339();
        write_manifest(&entry, &manifest).await?;
    }
    Ok(())
}

pub(crate) async fn prepare_conflict_recycle(
    config: &RecycleBinConfig,
    source: &Path,
    manifest: RecycleManifest,
) -> AppResult<PreparedRecycleFile> {
    if !config.enabled {
        return Err(AppError::validation_refused(
            crate::error_reasons::library::RECYCLE_BIN_PATH_UNSAFE,
            "A recycle entry cannot be reserved while the bin is disabled",
        ));
    }
    let original_path = manifest.original_path.clone();
    let source_operation_id = manifest.source_operation_id.clone().ok_or_else(|| {
        AppError::validation_refused(
            crate::error_reasons::library::RECYCLE_BIN_PATH_UNSAFE,
            "A reserved recycle entry requires its operation identity",
        )
    })?;
    let result = recycle_file_inner(
        config,
        source,
        source,
        manifest,
        false,
        RecycleStep::Prepare,
    )
    .await?
    .ok_or_else(|| {
        AppError::validation_refused(
            crate::error_reasons::library::REGULAR_FILE_REQUIRED,
            "The file disappeared before its recycle entry was reserved",
        )
    })?;
    Ok(PreparedRecycleFile {
        entry_id: result.entry_id,
        original_path,
        recycled_path: crate::stored_paths::path_to_stored_string(&result.recycled_path),
        source_operation_id,
    })
}

async fn pending_conflict_recycle(
    config: &RecycleBinConfig,
    source: &Path,
    prepared: &PreparedRecycleFile,
) -> AppResult<(PathBuf, PathBuf, RecycleManifest)> {
    let entry = config.base_path.join(&prepared.entry_id);
    let payload = crate::stored_paths::stored_path_to_path_buf(&prepared.recycled_path);
    let invalid = || {
        AppError::validation_refused(
            crate::error_reasons::library::RECYCLE_BIN_PATH_UNSAFE,
            "The reserved recycle entry changed; the source was preserved",
        )
    };
    if !config.enabled
        || !config.cleanup_enabled
        || Path::new(&prepared.entry_id).components().count() != 1
        || !matches!(
            Path::new(&prepared.entry_id).components().next(),
            Some(std::path::Component::Normal(_))
        )
        || crate::stored_paths::stored_path_to_path_buf(&prepared.original_path) != source
        || !prepared_payload_location(&entry, source, &payload)
    {
        return Err(invalid());
    }
    ensure_source_within_roots(config, source)?;
    ensure_recycle_root(config).await?;
    crate::location::title_folder_recovery::inspect_path(&entry, false).await?;
    inspect_payload_location(&entry, &payload).await?;
    let manifest = read_manifest(&entry).await?.ok_or_else(invalid)?;
    if manifest.entry_id.as_deref() != Some(&prepared.entry_id)
        || manifest.original_path != prepared.original_path
        || manifest.source_operation_id.as_deref() != Some(&prepared.source_operation_id)
        || manifest.status.as_deref() != Some(RECYCLE_STATUS_CONFLICT_PENDING)
    {
        return Err(invalid());
    }
    Ok((entry, payload, manifest))
}

pub(crate) async fn renew_conflict_recycle(
    config: &RecycleBinConfig,
    source: &Path,
    prepared: &PreparedRecycleFile,
) -> AppResult<PreparedRecycleFile> {
    let (_, payload, mut manifest) = pending_conflict_recycle(config, source, prepared).await?;
    if payload_is_manifest(&config.base_path.join(&prepared.entry_id), &payload).await? {
        manifest.recycled_at = Utc::now().to_rfc3339();
        return prepare_conflict_recycle(config, source, manifest).await;
    }
    let partial = crate::location::verify::partial_destination_path(&payload);
    let reading = Disk::process().uncached(&partial).await;
    match reading.exists() {
        Some(true) => {
            // A suffix and a pending entry do not prove ownership of these
            // bytes. Preserve the abandoned entry and reserve another entry;
            // the caller journals this replacement before executing it.
            manifest.recycled_at = Utc::now().to_rfc3339();
            prepare_conflict_recycle(config, source, manifest).await
        }
        Some(false) => Ok(prepared.clone()),
        None => Err(unreadable(&partial, reading)),
    }
}

pub(crate) async fn execute_conflict_recycle(
    config: &RecycleBinConfig,
    source: &Path,
    prepared: &PreparedRecycleFile,
    expected_source_version: &str,
    depth: VerificationDepth,
) -> AppResult<()> {
    let (_, payload, _) = pending_conflict_recycle(config, source, prepared).await?;
    let invalid = || {
        AppError::validation_refused(
            crate::error_reasons::library::RECYCLE_BIN_PATH_UNSAFE,
            "The reserved recycle entry changed; the source was preserved",
        )
    };
    if payload_is_manifest(&config.base_path.join(&prepared.entry_id), &payload).await? {
        return Err(invalid());
    }
    let partial = crate::location::verify::partial_destination_path(&payload);
    if Disk::process().uncached(&partial).await.exists() != Some(false) {
        return Err(invalid());
    }
    if crate::location::resolution::file_version(source).await? != expected_source_version {
        return Err(invalid());
    }
    let reading = Disk::process().uncached(&payload).await;
    match reading.exists() {
        Some(true) => {
            // A cross-volume copy can have been published before its selected
            // source pathname was retired. Resume only the reserved, complete
            // historical copy; a partial or replaced payload is preserved.
            if !reading.is(EntryKind::File) {
                return Err(invalid());
            }
            crate::location::title_folder_recovery::inspect_path(&payload, false).await?;
            // The payload must hold what the source holds, at the user's
            // depth, and neither may change while they are compared. At full
            // depth a hash kept of a file that still fits it answers.
            let check = Disk::process()
                .check(
                    crate::disk::CheckSide::new(payload.as_path()),
                    crate::disk::CheckSide::new(source),
                    crate::location::verify::disk_depth(depth),
                )
                .await;
            if check != Check::Same {
                return Err(invalid());
            }
            let removal = Disk::process().remove(source, Remove::File).await;
            if removal.map_err(repository)? == Removal::AlreadyAbsent {
                let gone = format!("{} was already gone", source.display());
                return Err(AppError::Repository(gone));
            }
        }
        Some(false) => recycle_source_to_destination(source.to_owned(), payload, depth).await?,
        None => return Err(unreadable(&payload, reading)),
    }
    finish_conflict_recycle(config, prepared).await
}

/// Move a reviewed file into its reserved recycle entry and commit it. The
/// moved file must have one name, or, with `linked_copy`, exactly one other
/// name: that path, proven to be the same file.
pub(crate) async fn execute_new_conflict_recycle(
    config: &RecycleBinConfig,
    source: &Path,
    prepared: &PreparedRecycleFile,
    expected_source_version: &str,
    linked_copy: Option<&Path>,
) -> AppResult<()> {
    let (_, payload, _) = pending_conflict_recycle(config, source, prepared).await?;
    let (version, _) = crate::location::resolution::regular_file_snapshot(source).await?;
    if version != expected_source_version {
        return Err(AppError::validation_refused(
            crate::delete_refusal::PREVIEW_STALE,
            "the reviewed source changed",
        ));
    }
    let before = crate::fs_integrity::import_content_proof_async(source)
        .await
        .map_err(|error| {
            AppError::validation_refused(
                crate::error_reasons::media::RECYCLE_FAILED,
                format!("could not verify the reviewed source before recycling: {error}"),
            )
        })?;
    // Do not use the ordinary recycle path's copy-then-unlink fallback. It
    // cannot condition the final unlink on the reviewed source identity.
    let disk = Disk::process();
    match disk.move_onto_free_name(source, &payload).await {
        Ok(FreeNameMove::Moved) => {}
        Err(error) if error.reason() == crate::disk::reason::DISK_MOVE_NEEDS_COPY => {
            return Err(AppError::validation_refused(
                crate::error_reasons::media::RECYCLE_FAILED,
                "cross-device recycling is unavailable; the reviewed source was preserved",
            ));
        }
        Err(error) => {
            return Err(AppError::validation_refused(
                crate::error_reasons::media::RECYCLE_FAILED,
                format!(
                    "the reviewed file could not be moved without overwriting: {}",
                    error.words()
                ),
            ));
        }
        Ok(FreeNameMove::NotOffered) => {
            return Err(AppError::validation_refused(
                crate::error_reasons::media::RECYCLE_FAILED,
                "this filesystem does not support exclusive recycling",
            ));
        }
    }
    match (
        reviewed_recycle_identity_matches(&payload, expected_source_version, linked_copy).await,
        crate::fs_integrity::import_content_proof_async(&payload).await,
    ) {
        (Ok(true), Ok(after)) if after == before => {}
        (identity, proof) => {
            let rollback = disk.move_onto_free_name(&payload, source).await;
            let detail = match (identity, proof) {
                (Ok(false), _) => "the source changed during the recycle move".to_string(),
                (_, Err(error)) => format!("the recycled payload could not be verified: {error}"),
                _ => "the recycled payload changed identity".to_string(),
            };
            let rollback_message = match rollback {
                Ok(FreeNameMove::Moved) => "the source was restored".to_string(),
                Err(error) => {
                    format!(
                        "rollback failed: {}; the payload is preserved in the recycle bin",
                        error.words()
                    )
                }
                Ok(FreeNameMove::NotOffered) => {
                    "rollback was unavailable; the payload is preserved in the recycle bin"
                        .to_string()
                }
            };
            return Err(AppError::validation_refused(
                crate::delete_refusal::PREVIEW_STALE,
                format!("{detail}; {rollback_message}"),
            ));
        }
    }
    if let Err(error) = finish_conflict_recycle(config, prepared).await {
        let rollback = disk.move_onto_free_name(&payload, source).await;
        let detail = match rollback {
            Ok(FreeNameMove::Moved) => {
                "the source was restored after the recycle receipt could not be committed"
                    .to_string()
            }
            Err(error) => {
                format!(
                    "rollback failed: {}; the payload is preserved in the recycle bin",
                    error.words()
                )
            }
            Ok(FreeNameMove::NotOffered) => {
                "rollback was unavailable; the payload is preserved in the recycle bin".to_string()
            }
        };
        return Err(AppError::Repository(format!(
            "could not commit the recycle receipt: {error}; {detail}"
        )));
    }
    Ok(())
}

async fn reviewed_recycle_identity_matches(
    path: &Path,
    expected_version: &str,
    linked_copy: Option<&Path>,
) -> AppResult<bool> {
    let (version, mark) = crate::location::resolution::regular_file_snapshot(path).await?;
    // Rename changes Unix ctime, but not the device, inode, length or mtime.
    let same_content = crate::disk::StoredVersion::new(&version)
        .same_content(crate::disk::StoredVersion::new(expected_version))
        == crate::disk::Sameness::Same;
    if !cfg!(unix) || !same_content {
        return Ok(false);
    }
    if mark.has_one_name() {
        return Ok(true);
    }
    // The only other name allowed is the confirmed linked copy, read once.
    let Some(copy) = linked_copy.filter(|_| mark.has_two_names()) else {
        return Ok(false);
    };
    Ok(match Disk::process().fingerprint(copy).await {
        Ok(copy_mark) => {
            copy_mark.has_two_names() && copy_mark.same_file(&mark) == crate::disk::Sameness::Same
        }
        Err(_) => false,
    })
}

/// Finish a reviewed recycle whose move already ran. The payload must have
/// one name, or, with `linked_copy`, exactly one other name: that path,
/// proven to be the same file.
pub(crate) async fn recover_reviewed_recycle(
    config: &RecycleBinConfig,
    issue: &scryer_domain::MediaIssue,
    receipt: &PreparedRecycleFile,
    linked_copy: Option<&Path>,
) -> AppResult<()> {
    let payload = crate::stored_paths::stored_path_to_path_buf(&receipt.recycled_path);
    let entry = config.base_path.join(&receipt.entry_id);
    let original = crate::stored_paths::stored_path_to_path_buf(&issue.file_path_snapshot);
    if receipt.source_operation_id != issue.id
        || receipt.original_path != issue.file_path_snapshot
        || !prepared_payload_location(&entry, &original, &payload)
        || !matches!(
            Path::new(&receipt.entry_id).components().next(),
            Some(std::path::Component::Normal(_))
        )
        || Path::new(&receipt.entry_id).components().count() != 1
    {
        return Err(AppError::validation_refused(
            crate::delete_refusal::PREVIEW_STALE,
            "the recycle receipt no longer matches the reviewed file",
        ));
    }
    crate::location::title_folder_recovery::inspect_path(&payload, false).await?;
    if !reviewed_recycle_identity_matches(&payload, &issue.file_version, linked_copy).await? {
        return Err(AppError::validation_refused(
            crate::delete_refusal::PREVIEW_STALE,
            "the recycle payload no longer matches the reviewed file",
        ));
    }
    let proof = crate::fs_integrity::import_content_proof_async(&payload).await?;
    if !crate::media_issues::content_matches_proof(&issue.content, &proof) {
        return Err(AppError::validation_refused(
            crate::delete_refusal::PREVIEW_STALE,
            "the recycle payload no longer matches the reviewed content",
        ));
    }
    finish_conflict_recycle(config, receipt).await
}

#[cfg(all(test, unix))]
mod conflict_recycle_tests {
    use super::*;

    #[tokio::test]
    async fn prepared_conflict_recycle_recovers_published_copy_and_preserves_invalid_payloads() {
        for (valid, depth) in [
            (false, VerificationDepth::Quick),
            (false, VerificationDepth::Full),
            (true, VerificationDepth::Quick),
            (true, VerificationDepth::Full),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let root = std::fs::canonicalize(temp.path()).unwrap();
            let source = root.join("selected.mkv");
            let external = root.join("external.mkv");
            let unrelated = root.join("unrelated.txt");
            std::fs::write(&source, b"historical movie").unwrap();
            std::fs::hard_link(&source, &external).unwrap();
            std::fs::write(&unrelated, b"unrelated content").unwrap();
            let config = RecycleBinConfig {
                enabled: true,
                base_path: root.join("bin"),
                retention_days: 7,
                cleanup_enabled: true,
                validation_error: None,
                source_roots: vec![root.clone()],
            };
            let original = crate::stored_paths::path_to_stored_string(&source);
            let mut manifest = RecycleManifest::pending_upgrade(
                original,
                "synthetic-file".into(),
                16,
                "synthetic-title".into(),
                None,
            );
            manifest.source_operation_id = Some("synthetic-operation".into());
            manifest.recycled_at = (Utc::now() - chrono::Duration::days(30)).to_rfc3339();
            let previous_recycled_at = manifest.recycled_at.clone();
            let prepared = prepare_conflict_recycle(&config, &source, manifest)
                .await
                .unwrap();
            let version = crate::location::resolution::file_version(&source)
                .await
                .unwrap();
            let payload = crate::stored_paths::stored_path_to_path_buf(&prepared.recycled_path);
            // Exact cross-volume crash window: a new inode was published, but
            // its original pathname has not yet been unlinked.
            if valid {
                std::fs::copy(&source, &payload).unwrap();
            } else {
                std::fs::write(&payload, b"partial").unwrap();
            }
            let outcome =
                execute_conflict_recycle(&config, &source, &prepared, &version, depth).await;
            assert_eq!(outcome.is_ok(), valid, "{outcome:?}");
            assert_eq!(source.exists(), !valid);
            assert_eq!(std::fs::read(&external).unwrap(), b"historical movie");
            assert_eq!(std::fs::read(&unrelated).unwrap(), b"unrelated content");
            assert_eq!(
                std::fs::read(&payload).unwrap(),
                if valid {
                    b"historical movie".as_slice()
                } else {
                    b"partial".as_slice()
                }
            );
            if valid {
                let committed_at = read_manifest(payload.parent().unwrap())
                    .await
                    .unwrap()
                    .unwrap()
                    .recycled_at;
                finish_conflict_recycle(&config, &prepared).await.unwrap();
                let committed = read_manifest(payload.parent().unwrap())
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(committed.recycled_at, committed_at);
                assert!(
                    chrono::DateTime::parse_from_rfc3339(&committed.recycled_at).unwrap()
                        > chrono::DateTime::parse_from_rfc3339(&previous_recycled_at).unwrap()
                );
                assert_eq!(
                    read_manifest(payload.parent().unwrap())
                        .await
                        .unwrap()
                        .unwrap()
                        .status
                        .as_deref(),
                    Some(RECYCLE_STATUS_COMMITTED)
                );
            }
        }
    }
    #[tokio::test]
    async fn prepared_conflict_recycle_renews_occupied_staging_without_touching_it() {
        for (partial_contents, published_payload) in [
            (b"historical movie".as_slice(), false),
            (b"partial".as_slice(), false),
            (b"unrelated staging occupant".as_slice(), false),
            (b"unrelated staging occupant".as_slice(), true),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let root = std::fs::canonicalize(temp.path()).unwrap();
            let source = root.join("selected.mkv");
            let external = root.join("external.mkv");
            std::fs::write(&source, b"historical movie").unwrap();
            std::fs::hard_link(&source, &external).unwrap();
            let config = RecycleBinConfig {
                enabled: true,
                base_path: root.join("bin"),
                retention_days: 7,
                cleanup_enabled: true,
                validation_error: None,
                source_roots: vec![root.clone()],
            };
            let mut manifest = RecycleManifest::pending_upgrade(
                crate::stored_paths::path_to_stored_string(&source),
                "synthetic-file".into(),
                16,
                "synthetic-title".into(),
                None,
            );
            manifest.source_operation_id = Some("synthetic-operation".into());
            let abandoned = prepare_conflict_recycle(&config, &source, manifest)
                .await
                .unwrap();
            let partial = crate::location::verify::partial_destination_path(
                &crate::stored_paths::stored_path_to_path_buf(&abandoned.recycled_path),
            );
            let mut old_manifest = read_manifest(partial.parent().unwrap())
                .await
                .unwrap()
                .unwrap();
            old_manifest.recycled_at = (Utc::now() - chrono::Duration::days(30)).to_rfc3339();
            write_manifest(partial.parent().unwrap(), &old_manifest)
                .await
                .unwrap();
            if published_payload {
                std::fs::copy(
                    &source,
                    crate::stored_paths::stored_path_to_path_buf(&abandoned.recycled_path),
                )
                .unwrap();
            }
            std::fs::write(&partial, partial_contents).unwrap();
            let version = crate::location::resolution::file_version(&source)
                .await
                .unwrap();
            let renewed = renew_conflict_recycle(&config, &source, &abandoned)
                .await
                .unwrap();
            assert_ne!(renewed.entry_id, abandoned.entry_id);
            // The caller persists `renewed` in the existing operation's intent
            // before executing. No ownership is inferred for the old partial,
            // even when its bytes happen to be a complete historical copy.
            execute_conflict_recycle(
                &config,
                &source,
                &renewed,
                &version,
                VerificationDepth::Full,
            )
            .await
            .unwrap();
            assert!(!source.exists());
            assert_eq!(std::fs::read(&partial).unwrap(), partial_contents);
            assert_eq!(std::fs::read(&external).unwrap(), b"historical movie");
            assert_eq!(
                read_manifest(partial.parent().unwrap())
                    .await
                    .unwrap()
                    .unwrap()
                    .status
                    .as_deref(),
                Some(RECYCLE_STATUS_CONFLICT_PENDING)
            );
            assert_eq!(reconcile_stale_pending_entries(&config).await.unwrap(), 0);
            let committed = list_committed_entries(&config).await.unwrap();
            assert_eq!(committed.len(), 1);
            assert_eq!(
                committed[0].manifest.entry_id.as_deref(),
                Some(renewed.entry_id.as_str())
            );
            assert!(purge_committed_entry(&config, &committed[0]).await.unwrap());
            assert_eq!(std::fs::read(&partial).unwrap(), partial_contents);
            if published_payload {
                assert_eq!(
                    std::fs::read(crate::stored_paths::stored_path_to_path_buf(
                        &abandoned.recycled_path
                    ))
                    .unwrap(),
                    b"historical movie"
                );
            }
            assert_eq!(std::fs::read(&external).unwrap(), b"historical movie");
        }
    }
}

/// What a recycle does once its entry is made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecycleStep {
    /// Reserve the entry only; the file is moved later.
    Prepare,
    /// Move the file in now. A move that must copy proves the copy at the
    /// user's depth before the source goes.
    Move(VerificationDepth),
}

async fn recycle_file_inner(
    config: &RecycleBinConfig,
    source_path: &Path,
    payload_name_path: &Path,
    mut manifest: RecycleManifest,
    commit_after_move: bool,
    step: RecycleStep,
) -> AppResult<Option<RecycleResult>> {
    let prepare_only = step == RecycleStep::Prepare;
    let source_reading = Disk::process().uncached(source_path).await;
    match source_reading.exists() {
        Some(true) => {}
        Some(false) => return Ok(None),
        None => {
            return Err(AppError::Repository(format!(
                "failed to stat source file {} before recycle/delete: {}",
                source_path.display(),
                source_reading.describe_absence()
            )));
        }
    }

    // Refuse to act on a path outside the configured media roots. This guards both
    // the permanent-delete branch (recycle disabled) and the recycle move against a
    // stale/corrupt/out-of-root source path.
    ensure_source_within_roots(config, source_path)?;
    if source_reading.is(EntryKind::Symlink) {
        return Err(AppError::validation_refused(
            crate::error_reasons::library::SYMLINK_REFUSED,
            format!(
                "refusing to delete {} because it is a symlink",
                source_path.display()
            ),
        ));
    }
    if !source_reading.is(EntryKind::File) {
        return Err(AppError::validation_refused(
            crate::error_reasons::library::REGULAR_FILE_REQUIRED,
            format!(
                "refusing to delete {} because it is not a regular file",
                source_path.display()
            ),
        ));
    }

    if !config.enabled {
        crate::fs_safety::remove_file_safely_if_exists(source_path).await?;
        return Ok(None);
    }

    if !config.cleanup_enabled {
        return Err(AppError::validation_refused(
            crate::error_reasons::library::RECYCLE_BIN_PATH_UNSAFE,
            format!(
                "refusing to recycle {} because the recycle bin path is unsafe: {}",
                source_path.display(),
                config
                    .validation_error
                    .as_deref()
                    .unwrap_or("invalid recycle bin configuration")
            ),
        ));
    }

    // Build timestamped directory name: YYYYMMDD_HHMMSSmmm_<6-char-id>
    let now = Utc::now();
    let full_id = scryer_domain::Id::new().0;
    let short_id = &full_id[..6];
    let dir_name = format!("{}_{}", now.format("%Y%m%d_%H%M%S%3f"), short_id);
    let recycle_dir = config.base_path.join(&dir_name);

    ensure_recycle_root(config).await?;
    let disk = Disk::process();
    disk.create_dir(&recycle_dir, Parents::MustExist)
        .await
        .map_err(|e| {
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
    manifest.status = Some(
        if prepare_only {
            RECYCLE_STATUS_CONFLICT_PENDING
        } else {
            RECYCLE_STATUS_PENDING
        }
        .to_string(),
    );
    if let Err(error) = write_manifest(&recycle_dir, &manifest).await {
        let _ = crate::fs_safety::remove_dir_all_safely_if_exists(&recycle_dir).await;
        return Err(error);
    }

    // Move the file into the recycle directory. Raw manifest strings are metadata
    // and may use stored-path encoding; payload names come from a trusted path
    // chosen by the caller.
    let mut recycled_path = recycle_dir.join(payload_relative_path(payload_name_path));
    if payload_is_manifest(&recycle_dir, &recycled_path).await? {
        recycled_path = recycle_dir.join(".scryer-payload").join(
            payload_name_path
                .file_name()
                .unwrap_or_else(|| std::ffi::OsStr::new("unknown")),
        );
    }
    if recycled_path.parent() != Some(recycle_dir.as_path()) {
        let parent = recycled_path.parent().expect("nested payload parent");
        disk.create_dir(parent, Parents::MustExist)
            .await
            .map_err(repository)?;
    }

    let depth = match step {
        RecycleStep::Prepare => {
            return Ok(Some(RecycleResult {
                entry_id: dir_name,
                entry_dir: recycle_dir.clone(),
                recycled_path,
                manifest_path: manifest_path(&recycle_dir),
            }));
        }
        RecycleStep::Move(depth) => depth,
    };

    if let Err(error) =
        recycle_source_to_destination(source_path.to_path_buf(), recycled_path.clone(), depth).await
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
    depth: VerificationDepth,
) -> AppResult<()> {
    let policy = placement::rename_onto_held_name();
    match (Disk::process().place(&source_path, &recycled_path, policy)).await {
        Ok(_) => return Ok(()),
        Err(error) if error.refused == Refused::NeedsCopy => {
            info!(
                source = %source_path.display(),
                recycled = %recycled_path.display(),
                error = %error,
                "recycle rename crossed devices; falling back to a verified copy"
            );
        }
        Err(error) => {
            return Err(AppError::Repository(format!(
                "failed to move {} to recycle bin {}: could not move {}: {}",
                source_path.display(),
                recycled_path.display(),
                source_path.display(),
                error
            )));
        }
    }

    recycle_by_verified_copy(&source_path, &recycled_path, CopyProgress::none(), depth).await
}

/// Copy `source_path` into the recycle bin, prove the copy at `depth`, and
/// only then remove the source. `recycled_path` must not exist yet.
async fn recycle_by_verified_copy(
    source_path: &Path,
    recycled_path: &Path,
    progress: CopyProgress,
    depth: VerificationDepth,
) -> AppResult<()> {
    proven_copy(
        source_path,
        recycled_path,
        CopyTarget::Unclaimed,
        progress,
        depth,
    )
    .await?;

    match crate::fs_safety::remove_file_safely_if_exists(source_path).await {
        Ok(()) => Ok(()),
        Err(error) => {
            // The proven copy is the file's only sure home unless the source
            // still reads as present: a source that is gone, or whose name
            // does not answer, keeps its recycled copy.
            let source_present = Disk::process().uncached(source_path).await.exists();
            let kept = if source_present == Some(true) {
                let _ = crate::fs_safety::remove_file_safely_if_exists(recycled_path).await;
                ""
            } else {
                tracing::warn!(
                    source = %source_path.display(),
                    recycled = %recycled_path.display(),
                    "the source could not be removed after its proven copy and does not read as present; the recycled copy is kept"
                );
                "; the recycled copy is kept"
            };
            Err(AppError::Repository(format!(
                "failed to remove source file {} after copy to recycle bin: {}{kept}",
                source_path.display(),
                error
            )))
        }
    }
}

/// Who holds the name a quick verified copy lands on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CopyTarget {
    /// Nothing yet: the copy claims the name itself.
    Unclaimed,
    /// An empty file this operation made to reserve the name.
    OwnClaim,
}

/// Copy one file through the verified copier and prove it at `depth`, the
/// user's verification setting for this request. Anything short of a verified
/// copy removes what the copy wrote and fails, so the caller never removes a
/// source on an unproven destination.
async fn proven_copy(
    source: &Path,
    destination: &Path,
    target: CopyTarget,
    progress: CopyProgress,
    depth: VerificationDepth,
) -> AppResult<()> {
    let claim = match target {
        CopyTarget::Unclaimed => DestinationClaim::ClaimHere,
        CopyTarget::OwnClaim => DestinationClaim::AlreadyHeld,
    };
    let copied = copy_and_verify_reporting(
        &Disk::process(),
        source,
        destination,
        placement::verified_copy(depth, claim),
        &progress,
    )
    .await;
    let verified = match copied {
        Ok(verified) => verified,
        Err(failure) => {
            // A copy that claims its own name never leaves anything under it
            // on failure, and whatever holds that name is not this copy's. A
            // name this operation reserved is its own to clean up.
            if target == CopyTarget::OwnClaim {
                let _ = crate::fs_safety::remove_file_safely_if_exists(destination).await;
            }
            return Err(failure.error);
        }
    };
    if verified.permits_source_removal() {
        return Ok(());
    }
    let _ = crate::fs_safety::remove_file_safely_if_exists(destination).await;
    let detail = verified
        .detail
        .map(|detail| format!(": {detail}"))
        .unwrap_or_default();
    Err(AppError::Repository(format!(
        "copy verification failed for {} -> {}{detail}",
        source.display(),
        destination.display()
    )))
}

pub(crate) use scryer_disk::path_is_under_configured_root;

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
        return Err(AppError::validation_refused(
            crate::error_reasons::library::MEDIA_ROOT_REQUIRED,
            format!(
                "refusing to restore {} because no configured media roots are available",
                destination.display()
            ),
        ));
    }
    if restore_destination_is_under_configured_roots(destination, roots) {
        return Ok(());
    }
    Err(AppError::validation_refused(
        crate::error_reasons::library::PATH_OUTSIDE_LIBRARY_ROOT,
        format!(
            "refusing to restore {} because it is outside the configured media roots",
            destination.display()
        ),
    ))
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
        return Err(AppError::validation_refused(
            crate::error_reasons::library::MEDIA_ROOT_REQUIRED,
            format!(
                "refusing to delete {} because no configured media roots are available",
                source_path.display()
            ),
        ));
    }
    if config
        .source_roots
        .iter()
        .any(|root| source_file_is_under_configured_root(source_path, root))
    {
        return Ok(());
    }
    Err(AppError::validation_refused(
        crate::error_reasons::library::PATH_OUTSIDE_LIBRARY_ROOT,
        format!(
            "refusing to delete {} because it is outside the configured media roots",
            source_path.display()
        ),
    ))
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
    let mut file_name = original_path
        .file_stem()
        .unwrap_or_else(|| std::ffi::OsStr::new("restored"))
        .to_os_string();

    let suffix = if attempt <= 10_000 {
        if attempt == 1 {
            "-restored".to_string()
        } else {
            format!("-restored-{attempt}")
        }
    } else {
        format!("-restored-{}", scryer_domain::Id::new().0)
    };
    file_name.push(suffix);
    if let Some(extension) = original_path.extension() {
        file_name.push(".");
        file_name.push(extension);
    }
    parent.join(file_name)
}

/// Copy a recycled file onto a destination name the caller already claimed.
/// The recycled source is never touched here.
async fn copy_recycled_to_claimed_destination(
    recycled_path: &Path,
    destination: &Path,
    progress: CopyProgress,
    depth: VerificationDepth,
) -> AppResult<()> {
    proven_copy(
        recycled_path,
        destination,
        CopyTarget::OwnClaim,
        progress,
        depth,
    )
    .await
}

async fn restore_without_overwrite(
    recycled_path: &Path,
    original_path: &Path,
    allowed_roots: Option<&[PathBuf]>,
    depth: VerificationDepth,
) -> AppResult<PathBuf> {
    let disk = Disk::process();
    let policy = placement::recycle_restore_link();
    // The recycled file's name and the folder every candidate name is in are
    // read once, not once per name tried.
    let mut read = None;
    for attempt in 0..=20_000u32 {
        let destination = restore_candidate_path(original_path, attempt);
        if let Some(roots) = allowed_roots {
            ensure_restore_destination_within_roots(&destination, roots)?;
        }
        let placed = match &read {
            Some(read) => {
                disk.place_after_reading(recycled_path, &destination, policy, read)
                    .await
            }
            None => match disk
                .read_before_placing(recycled_path, &destination, policy)
                .await
            {
                Ok(first) => {
                    let read = read.insert(first);
                    disk.place_after_reading(recycled_path, &destination, policy, read)
                        .await
                }
                Err(refusal) => Err(refusal),
            },
        };
        match placed {
            Ok(Placed {
                done: Done::HardLinked,
                ..
            }) => {}
            // Only a link shares the recycled file's bytes; anything else
            // the owner did is not proof enough to remove the recycled copy.
            Ok(_) => {
                return Err(AppError::Repository(format!(
                    "{} was put back at {} without a hard link, so the recycled copy was kept",
                    recycled_path.display(),
                    destination.display()
                )));
            }
            Err(refusal) if refusal.refused == Refused::DestinationExists => continue,
            Err(link_error) => {
                // Hard links are preferred but impossible across devices and on
                // filesystems without link support; claim the destination and
                // copy with verification instead.
                match disk.open(&destination, Open::create_new()).await {
                    Ok(claimed) => drop(claimed),
                    Err(error) if error.reason() == DISK_DESTINATION_EXISTS => continue,
                    Err(error) => {
                        return Err(AppError::Repository(format!(
                            "failed to claim restore destination {} after hard link failed ({}): {}",
                            destination.display(),
                            link_error,
                            error.words()
                        )));
                    }
                };
                copy_recycled_to_claimed_destination(
                    recycled_path,
                    &destination,
                    CopyProgress::none(),
                    depth,
                )
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
    depth: VerificationDepth,
) -> AppResult<PathBuf> {
    restore_from_recycle_inner(recycled_path, original_path, overwrite, None, depth).await
}

pub(crate) async fn restore_from_recycle_with_roots(
    recycled_path: &Path,
    original_path: &Path,
    overwrite: bool,
    allowed_roots: &[PathBuf],
    depth: VerificationDepth,
) -> AppResult<PathBuf> {
    restore_from_recycle_inner(
        recycled_path,
        original_path,
        overwrite,
        Some(allowed_roots),
        depth,
    )
    .await
}

pub(crate) async fn restore_recycled_file_exact(
    recycled_path: &Path,
    destination: &Path,
    depth: VerificationDepth,
) -> AppResult<()> {
    link_recycled_file_to_exact_destination(recycled_path, destination, depth).await?;
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
    depth: VerificationDepth,
) -> AppResult<()> {
    ensure_recycled_restore_source_is_regular(recycled_path).await?;
    let disk = Disk::process();
    let reading = disk.uncached(destination).await;
    match reading.exists() {
        Some(true) => {
            return Err(AppError::validation_refused(
                crate::error_reasons::library::DESTINATION_OCCUPIED,
                format!(
                    "refusing to restore recycled file {} because destination is occupied: {}",
                    recycled_path.display(),
                    destination.display()
                ),
            ));
        }
        Some(false) => {}
        None => {
            return Err(AppError::Repository(format!(
                "failed to stat recycle restore destination {}: {}",
                destination.display(),
                reading.describe_absence()
            )));
        }
    }

    if let Some(parent) = destination.parent() {
        let made = disk.create_dir(parent, Parents::Create).await;
        made.map_err(std::io::Error::from).map_err(|error| {
            AppError::Repository(format!(
                "failed to create restore parent directory {}: {}",
                parent.display(),
                error
            ))
        })?;
    }

    let policy = placement::recycle_restore_link();
    match disk.place(recycled_path, destination, policy).await {
        Ok(_) => Ok(()),
        Err(link_error) => {
            // Same fallback as restore_without_overwrite; either way the
            // recycled source is retained, so callers keep their rollback
            // point until they explicitly remove it.
            let claimed = disk.open(destination, Open::create_new()).await;
            let claimed = claimed.map_err(std::io::Error::from).map_err(|error| {
                AppError::Repository(format!(
                    "failed to claim exact restore destination {} after hard link failed ({}): {}",
                    destination.display(),
                    link_error,
                    error
                ))
            })?;
            drop(claimed);
            copy_recycled_to_claimed_destination(
                recycled_path,
                destination,
                CopyProgress::none(),
                depth,
            )
            .await
        }
    }
}

/// Restore across filesystems onto `original_path`, replacing whatever file
/// holds that name. The restored copy is proven before the recycled source is
/// removed; an unproven restore is removed and the recycled copy is kept, so
/// the file is never lost.
async fn restore_over_by_verified_copy(
    recycled_path: &Path,
    original_path: &Path,
    progress: CopyProgress,
    depth: VerificationDepth,
) -> AppResult<()> {
    let reading = Disk::process().uncached(original_path).await;
    match reading.exists() {
        Some(true) => {
            proven_replacement(recycled_path, original_path, progress, depth).await?;
        }
        Some(false) => {
            proven_copy(
                recycled_path,
                original_path,
                CopyTarget::Unclaimed,
                progress,
                depth,
            )
            .await?;
        }
        None => return Err(unreadable(original_path, reading)),
    }
    let _ = crate::fs_safety::remove_file_safely_if_exists(recycled_path).await;
    Ok(())
}

/// Replace the file at `destination` with a proven copy of `source`, never
/// writing into the file that is there.
///
/// The copy is written under the staging name beside `destination` and proven
/// at `depth`; only then does a rename give it the destination name. The
/// rename replaces the directory entry alone, so a file the old name shared
/// with other names (a hard link to a seeding download, say) keeps its bytes
/// under those names. Anything short of that rename leaves `destination`
/// untouched and removes what the copy wrote under the staging name; a
/// staging name some other file already holds refuses the copy and is left
/// alone. `source` is never touched.
async fn proven_replacement(
    source: &Path,
    destination: &Path,
    progress: CopyProgress,
    depth: VerificationDepth,
) -> AppResult<()> {
    let staged = crate::disk::resumable_staging_path(destination);
    let copied = copy_and_verify_reporting(
        &Disk::process(),
        source,
        &staged,
        placement::staged_replacement_copy(depth),
        &progress,
    )
    .await;
    let verified = match copied {
        Ok(verified) => verified,
        Err(failure) => {
            // The copy created the staging name only if it wrote there; a
            // name it could not create is some other file's.
            if failure.destination_written {
                let _ = crate::fs_safety::remove_file_safely_if_exists(&staged).await;
            }
            return Err(failure.error);
        }
    };
    if !verified.permits_source_removal() {
        let _ = crate::fs_safety::remove_file_safely_if_exists(&staged).await;
        let detail = verified
            .detail
            .map(|detail| format!(": {detail}"))
            .unwrap_or_default();
        return Err(AppError::Repository(format!(
            "copy verification failed for {} -> {}{detail}",
            source.display(),
            staged.display()
        )));
    }
    match Disk::process()
        .place(&staged, destination, placement::rename_onto_held_name())
        .await
    {
        Ok(_) => Ok(()),
        Err(error) => {
            let _ = crate::fs_safety::remove_file_safely_if_exists(&staged).await;
            Err(AppError::Repository(format!(
                "failed to replace {} with its proven restore {}: {}",
                destination.display(),
                staged.display(),
                error
            )))
        }
    }
}

async fn restore_from_recycle_inner(
    recycled_path: &Path,
    original_path: &Path,
    overwrite: bool,
    allowed_roots: Option<&[PathBuf]>,
    depth: VerificationDepth,
) -> AppResult<PathBuf> {
    ensure_recycled_restore_source_is_regular(recycled_path).await?;

    if let Some(roots) = allowed_roots {
        ensure_restore_destination_within_roots(original_path, roots)?;
    }

    // Ensure parent directory exists
    if let Some(parent) = original_path.parent() {
        Disk::process()
            .create_dir(parent, Parents::Create)
            .await
            .map_err(|e| {
                AppError::Repository(format!(
                    "failed to create parent directory {}: {}",
                    parent.display(),
                    e
                ))
            })?;
    }

    if !overwrite {
        let destination =
            restore_without_overwrite(recycled_path, original_path, allowed_roots, depth).await?;
        info!(
            restored = %destination.display(),
            "file restored from recycle bin"
        );
        return Ok(destination);
    }

    let policy = placement::rename_onto_held_name();
    match (Disk::process().place(recycled_path, original_path, policy)).await {
        Ok(_) => {}
        Err(error) if error.refused == Refused::NeedsCopy => {
            restore_over_by_verified_copy(
                recycled_path,
                original_path,
                CopyProgress::none(),
                depth,
            )
            .await?;
        }
        Err(error) => {
            return Err(AppError::Repository(format!(
                "failed to move recycled file {} to {}: could not move {}: {}",
                recycled_path.display(),
                original_path.display(),
                recycled_path.display(),
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

/// Refuse a recycled payload that is missing, a symlink or not a regular
/// file. Returns the payload's size when the reading carries it.
pub(crate) async fn ensure_recycled_restore_source_is_regular(
    recycled_path: &Path,
) -> AppResult<Option<u64>> {
    let reading = Disk::process().uncached(recycled_path).await;
    match reading.exists() {
        Some(true) => {}
        Some(false) => {
            return Err(AppError::Repository(format!(
                "recycled file not found: {}",
                recycled_path.display()
            )));
        }
        None => return Err(unreadable(recycled_path, reading)),
    }
    if reading.is(EntryKind::Symlink) {
        return Err(AppError::validation_refused(
            crate::error_reasons::library::SYMLINK_REFUSED,
            format!(
                "refusing to restore recycled file {} because it is a symlink",
                recycled_path.display()
            ),
        ));
    }
    if !reading.is(EntryKind::File) {
        return Err(AppError::validation_refused(
            crate::error_reasons::library::REGULAR_FILE_REQUIRED,
            format!(
                "refusing to restore recycled file {} because it is not a regular file",
                recycled_path.display()
            ),
        ));
    }
    Ok(reading.size())
}

pub async fn commit_recycle_entry(
    recycle_result: &Option<RecycleResult>,
    replacement_file_id: &str,
    replacement_path: &Path,
) -> AppResult<()> {
    let Some(result) = recycle_result else {
        return Ok(());
    };

    if Disk::process().follow(replacement_path).await.exists() != Some(true) {
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
    Ok(inspect_entry_dir(config, entry_dir).await.committed)
}

/// What one look at an entry directory found.
struct InspectedEntry {
    /// The title the manifest names, whenever the directory is a real entry
    /// directory under the root with a readable manifest.
    title_id: Option<Option<String>>,
    /// The entry, when it is trusted and committed.
    committed: Option<CommittedRecycleEntry>,
    /// The upgrade the manifest records, when the entry is laid out as
    /// Scryer wrote it and is committed or quarantined.
    upgrade_link: Option<(String, UpgradeLink)>,
    /// The entry, when it is quarantined, laid out as Scryer wrote it, and
    /// was recycled by an upgrade. Read from the manifest alone.
    quarantined_upgrade: Option<QuarantinedUpgradeEntry>,
}

impl InspectedEntry {
    fn none() -> Self {
        Self {
            title_id: None,
            committed: None,
            upgrade_link: None,
            quarantined_upgrade: None,
        }
    }
}

/// An upgrade a recycle entry records: the file it recycled was replaced by
/// `replacement_file_id`, a file of `title_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UpgradeLink {
    pub title_id: String,
    pub replacement_file_id: String,
}

/// The next step from a file id along the upgrades the bin records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UpgradeLinkStep<'a> {
    /// No entry records an upgrade of this file.
    Missing,
    /// Entries record different upgrades of this file.
    Ambiguous,
    Link(&'a UpgradeLink),
}

/// The upgrades the entries of one bin record, by the file each replaced,
/// gathered from the manifests one listing already read.
#[derive(Debug, Default)]
pub(crate) struct UpgradeLinks {
    by_original: std::collections::HashMap<String, Option<UpgradeLink>>,
}

impl UpgradeLinks {
    fn record(&mut self, original_file_id: String, link: UpgradeLink) {
        match self.by_original.entry(original_file_id) {
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert(Some(link));
            }
            std::collections::hash_map::Entry::Occupied(mut slot) => {
                if slot.get().as_ref() != Some(&link) {
                    slot.insert(None);
                }
            }
        }
    }

    pub(crate) fn step(&self, file_id: &str) -> UpgradeLinkStep<'_> {
        match self.by_original.get(file_id) {
            None => UpgradeLinkStep::Missing,
            Some(None) => UpgradeLinkStep::Ambiguous,
            Some(Some(link)) => UpgradeLinkStep::Link(link),
        }
    }
}

/// The upgrade an entry's manifest records, when it records one completely.
fn manifest_upgrade_link(manifest: &RecycleManifest) -> Option<(String, UpgradeLink)> {
    if manifest.recycled_reason() != "upgrade_replaced" {
        return None;
    }
    let non_empty = |value: &Option<String>| {
        value
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    let original = non_empty(&manifest.original_file_id)?;
    let replacement = non_empty(&manifest.replacement_file_id)?;
    if original == replacement {
        return None;
    }
    Some((
        original,
        UpgradeLink {
            title_id: non_empty(&manifest.title_id)?,
            replacement_file_id: replacement,
        },
    ))
}

/// The trusted committed entry at `entry_dir`, plus the title its manifest
/// names whenever the directory is a real entry directory under the root with a
/// readable manifest. Manifests never change their title once written, so that
/// title is stable for the life of the directory.
async fn inspect_entry_dir(config: &RecycleBinConfig, entry_dir: PathBuf) -> InspectedEntry {
    let reading = Disk::process().uncached(&entry_dir).await;
    if reading.exists().is_none() {
        warn!(
            path = %entry_dir.display(),
            reason = ?reading.unknown_reason(),
            "failed to inspect recycle entry path, skipping"
        );
    }
    if reading.is(EntryKind::Symlink) {
        warn!(
            path = %entry_dir.display(),
            "skipping symlinked recycle entry"
        );
        return InspectedEntry::none();
    }
    if !reading.is(EntryKind::Directory) {
        return InspectedEntry::none();
    }
    let Some(parent) = entry_dir.parent() else {
        return InspectedEntry::none();
    };
    if normalize_path(parent) != normalize_path(&config.base_path) {
        warn!(
            path = %entry_dir.display(),
            recycle_root = %config.base_path.display(),
            "skipping recycle entry outside configured recycle root"
        );
        return InspectedEntry::none();
    }

    let Some(manifest) = (match read_manifest(&entry_dir).await {
        Ok(manifest) => manifest,
        Err(error) => {
            warn!(path = %entry_dir.display(), error = %error, "failed to inspect recycle entry, skipping");
            return InspectedEntry::none();
        }
    }) else {
        return InspectedEntry::none();
    };
    let title_id = Some(manifest.title_id.clone());
    let layout = trusted_entry_layout(config, &entry_dir, &manifest);
    let upgrade_link = (layout.is_ok() && (manifest.is_committed() || manifest.is_quarantined()))
        .then(|| manifest_upgrade_link(&manifest))
        .flatten();
    let quarantined_upgrade = layout.is_ok()
        && manifest.is_quarantined()
        && manifest.recycled_reason() == "upgrade_replaced";

    if let Err(reason) = layout.and_then(|()| committed_status(&manifest)) {
        if let Err(error) =
            quarantine_untrusted_committed_entry(config, &entry_dir, &manifest, &reason).await
        {
            warn!(
                path = %entry_dir.display(),
                error = %error,
                "failed to quarantine untrusted recycle entry"
            );
        }
        return InspectedEntry {
            title_id,
            committed: None,
            upgrade_link,
            quarantined_upgrade: quarantined_upgrade.then_some(QuarantinedUpgradeEntry {
                entry_dir,
                manifest,
            }),
        };
    }

    if let Err(error) = recycled_payload_path(&entry_dir, &manifest).await {
        warn!(path = %entry_dir.display(), error = %error, "recycle metadata cannot be used as its payload; entry preserved");
        return InspectedEntry {
            title_id,
            committed: None,
            upgrade_link,
            quarantined_upgrade: None,
        };
    }
    InspectedEntry {
        title_id,
        committed: Some(CommittedRecycleEntry {
            entry_dir,
            manifest,
        }),
        upgrade_link,
        quarantined_upgrade: None,
    }
}

/// Remembers, per recycle root, which title each entry directory belongs to,
/// so one deletion job reads each manifest once instead of once per title.
///
/// Every title still lists the root's directory names, so entries created
/// after an earlier title are seen, and still fully re-inspects every entry
/// that is or may be its own. Only directories already known to belong to a
/// different title are passed over.
#[derive(Debug, Default)]
pub struct TitleRecycleEntryIndex {
    roots: std::collections::HashMap<
        PathBuf,
        std::collections::HashMap<std::ffi::OsString, Option<String>>,
    >,
}

/// The same entries, in the same order and with the same inspection, that
/// [`list_committed_entries`] returns for this title.
#[cfg(test)]
pub(crate) async fn list_committed_entries_for_title(
    config: &RecycleBinConfig,
    title_id: &str,
    index: &mut TitleRecycleEntryIndex,
) -> AppResult<Vec<CommittedRecycleEntry>> {
    Ok(
        list_committed_entries_for_title_with_upgrade_links(config, title_id, index)
            .await?
            .0,
    )
}

/// The same entries, in the same order and with the same inspection, that
/// [`list_committed_entries`] returns for this title, plus the upgrades of
/// this title that the manifests it read record.
pub(crate) async fn list_committed_entries_for_title_with_upgrade_links(
    config: &RecycleBinConfig,
    title_id: &str,
    index: &mut TitleRecycleEntryIndex,
) -> AppResult<(Vec<CommittedRecycleEntry>, UpgradeLinks)> {
    if !cleanup_ready(config) {
        if let Some(error) = &config.validation_error {
            warn!(error = %error, path = %config.base_path.display(), "recycle bin cleanup disabled");
        }
        return Ok((Vec::new(), UpgradeLinks::default()));
    }

    #[cfg(test)]
    test_bin_calls::record(test_bin_calls::LIST_FOR_TITLE, &config.base_path);
    let known = index
        .roots
        .entry(normalize_path(&config.base_path))
        .or_default();
    let mut results = Vec::new();
    let mut links = UpgradeLinks::default();
    for entry in recycle_root_entries(config, |_| true).await? {
        let name = entry.file_name().unwrap_or_default().to_os_string();
        if let Some(owner) = known.get(&name)
            && owner.as_deref() != Some(title_id)
        {
            continue;
        }
        let InspectedEntry {
            title_id: owner,
            committed,
            upgrade_link,
            quarantined_upgrade: _,
        } = inspect_entry_dir(config, entry).await;
        if let Some((original_file_id, link)) = upgrade_link
            && link.title_id == title_id
        {
            links.record(original_file_id, link);
        }
        match owner {
            Some(owner) => {
                known.insert(name, owner);
            }
            None => {
                known.remove(&name);
            }
        }
        if let Some(committed) = committed
            && committed.manifest.title_id.as_deref() == Some(title_id)
        {
            results.push(committed);
        }
    }

    Ok((results, links))
}

/// Each listing, lookup and purge of a recycle root's entries, so tests can
/// assert which operations touch a bin and how often.
#[cfg(test)]
pub(crate) mod test_bin_calls {
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    /// A full listing of the root's entries.
    pub(crate) const LIST: &str = "list";
    /// A listing of the root for one title's entries.
    pub(crate) const LIST_FOR_TITLE: &str = "list_for_title";
    /// A lookup of one entry by id.
    pub(crate) const FIND: &str = "find";
    /// A purge of one entry.
    pub(crate) const PURGE: &str = "purge";

    static CALLS: Mutex<Vec<(&'static str, PathBuf)>> = Mutex::new(Vec::new());

    pub(super) fn record(kind: &'static str, root: &Path) {
        CALLS.lock().unwrap().push((kind, root.to_path_buf()));
    }

    /// Every call on `root`, of any kind.
    pub(crate) fn count(root: &Path) -> usize {
        CALLS
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, called)| called.as_path() == root)
            .count()
    }

    /// The calls of one kind on `root`.
    pub(crate) fn count_kind(root: &Path, kind: &str) -> usize {
        CALLS
            .lock()
            .unwrap()
            .iter()
            .filter(|(called_kind, called)| *called_kind == kind && called.as_path() == root)
            .count()
    }
}

pub async fn list_committed_entries(
    config: &RecycleBinConfig,
) -> AppResult<Vec<CommittedRecycleEntry>> {
    Ok(list_committed_entries_reporting_withheld(config).await?.0)
}

/// [`list_committed_entries`], plus the title of every entry directory under
/// the root it withheld (pending, quarantined or untrusted) whose manifest
/// names one. The listing and the inspection of each entry are the same.
pub(crate) async fn list_committed_entries_reporting_withheld(
    config: &RecycleBinConfig,
) -> AppResult<(Vec<CommittedRecycleEntry>, Vec<String>)> {
    let listing = list_committed_entries_with_upgrade_links(config).await?;
    Ok((listing.committed, listing.withheld_titles))
}

/// One listing of a recycle root.
#[derive(Debug, Default)]
pub(crate) struct RecycleRootListing {
    /// The trusted committed entries.
    pub committed: Vec<CommittedRecycleEntry>,
    /// The title of every entry directory withheld from `committed` whose
    /// manifest names one.
    pub withheld_titles: Vec<String>,
    /// The upgrades recorded by the committed and quarantined entries the
    /// listing read.
    pub upgrade_links: UpgradeLinks,
    /// The quarantined entries of files an upgrade replaced, for the pass
    /// that checks whether the library now covers what each held.
    pub quarantined_upgrades: Vec<QuarantinedUpgradeEntry>,
}

/// [`list_committed_entries_reporting_withheld`], plus the upgrades the
/// manifests it already read record. The listing and the inspection of each
/// entry are the same.
pub(crate) async fn list_committed_entries_with_upgrade_links(
    config: &RecycleBinConfig,
) -> AppResult<RecycleRootListing> {
    if !cleanup_ready(config) {
        if let Some(error) = &config.validation_error {
            warn!(error = %error, path = %config.base_path.display(), "recycle bin cleanup disabled");
        }
        return Ok(RecycleRootListing::default());
    }

    #[cfg(test)]
    test_bin_calls::record(test_bin_calls::LIST, &config.base_path);
    let mut listing = RecycleRootListing::default();
    for entry in recycle_root_entries(config, |_| true).await? {
        let inspected = inspect_entry_dir(config, entry).await;
        if let Some((original_file_id, link)) = inspected.upgrade_link {
            listing.upgrade_links.record(original_file_id, link);
        }
        listing
            .quarantined_upgrades
            .extend(inspected.quarantined_upgrade);
        match (inspected.title_id, inspected.committed) {
            (_, Some(committed)) => listing.committed.push(committed),
            (Some(Some(title_id)), None) => listing.withheld_titles.push(title_id),
            _ => {}
        }
    }

    Ok(listing)
}

/// The moment before which an entry of `config` has expired.
pub(crate) fn expiry_cutoff(config: &RecycleBinConfig) -> chrono::DateTime<Utc> {
    Utc::now() - chrono::Duration::days(config.retention_days as i64)
}

/// Whether `entry` was recycled before `cutoff`. An unreadable time never
/// expires.
pub(crate) fn recycled_before(
    entry: &CommittedRecycleEntry,
    cutoff: chrono::DateTime<Utc>,
) -> bool {
    entry.manifest.recycled_before(cutoff)
}

pub async fn list_expired_committed_entries(
    config: &RecycleBinConfig,
) -> AppResult<Vec<CommittedRecycleEntry>> {
    let cutoff = expiry_cutoff(config);
    Ok(list_committed_entries(config)
        .await?
        .into_iter()
        .filter(|entry| recycled_before(entry, cutoff))
        .collect())
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
    let directories = |entry: &Visited| entry.kind().is_ok_and(|kind| kind == EntryKind::Directory);
    for entry_dir in recycle_root_entries(config, directories).await? {
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
    #[cfg(test)]
    test_bin_calls::record(test_bin_calls::PURGE, &config.base_path);
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

    if recycled_payload_path(&entry.entry_dir, &entry.manifest)
        .await
        .is_err()
    {
        return Ok(false);
    }
    crate::fs_safety::remove_dir_all_safely(&entry.entry_dir).await?;
    Ok(true)
}

/// Removes a quarantined upgrade entry the caller has just shown the library
/// no longer needs. Only an entry still laid out as Scryer wrote it, still
/// quarantined, still an upgrade's, and whose payload is its own goes; any
/// other is kept and `false` is returned.
pub(crate) async fn purge_quarantined_upgrade_entry(
    config: &RecycleBinConfig,
    entry: &QuarantinedUpgradeEntry,
) -> AppResult<bool> {
    if let Err(reason) = trusted_entry_layout(config, &entry.entry_dir, &entry.manifest) {
        warn!(path = %entry.entry_dir.display(), reason = %reason, "skipping untrusted quarantined recycle entry purge");
        return Ok(false);
    }
    if !entry.manifest.is_quarantined() || entry.manifest.recycled_reason() != "upgrade_replaced" {
        return Ok(false);
    }
    if recycled_payload_path(&entry.entry_dir, &entry.manifest)
        .await
        .is_err()
    {
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

    #[cfg(test)]
    test_bin_calls::record(test_bin_calls::FIND, &config.base_path);
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

// ── Moving entries to a new recycle location ─────────────────────────────────
//
// The bin lists, restores and purges only entries under the currently
// configured location. When that location changes, committed entries in the
// previous location are moved so they stay managed. The previous copy of an
// entry is removed only after the new copy has been proven complete, and any
// failure leaves the entry exactly where it was.

/// One entry a location change could not move. It stays at `from_path`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecycleEntryRelocationFailure {
    pub entry_id: String,
    pub from_path: String,
    pub reason: String,
}

/// What a location change did with the entries of the previous location.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecycleBinRelocationReport {
    pub moved_count: u32,
    pub failures: Vec<RecycleEntryRelocationFailure>,
}

impl RecycleBinRelocationReport {
    pub fn is_empty(&self) -> bool {
        self.moved_count == 0 && self.failures.is_empty()
    }

    pub(crate) fn merge(&mut self, other: RecycleBinRelocationReport) {
        self.moved_count += other.moved_count;
        self.failures.extend(other.failures);
    }
}

/// A destination bin and the media roots whose entries belong in it.
#[derive(Debug, Clone)]
pub(crate) struct RecycleRelocationTarget {
    pub base_path: PathBuf,
    pub media_roots: Vec<PathBuf>,
}

/// Entries of one previous bin and where they go. With a single target every
/// entry goes there; with several, each entry follows the media root its
/// original path lives under.
#[derive(Debug, Clone)]
pub(crate) struct RecycleRelocationPlan {
    pub from: PathBuf,
    pub targets: Vec<RecycleRelocationTarget>,
}

/// How a relocation moves an entry, replaceable so tests can force the copy
/// path and inject verification failures.
#[derive(Clone)]
struct RelocationOps {
    /// Try an atomic no-replace rename before copying.
    allow_rename: bool,
    /// Copies and proves each file of an entry that cannot be renamed.
    copier: Disk,
    /// The user's verification depth for this relocation's copies.
    depth: VerificationDepth,
}

/// The full depth the relocation tests were written for.
#[cfg(test)]
impl Default for RelocationOps {
    fn default() -> Self {
        Self::at(VerificationDepth::Full)
    }
}

impl RelocationOps {
    fn at(depth: VerificationDepth) -> Self {
        Self {
            allow_rename: true,
            copier: Disk::process(),
            depth,
        }
    }
}

/// Move every committed entry of each plan's previous bin to its new bin.
///
/// Only directories that carry a valid manifest for their own entry id are
/// touched; anything else in the previous bin, and the previous bin itself,
/// is left as it is. An entry is never written over an existing entry.
pub(crate) async fn relocate_recycle_entries(
    plans: Vec<RecycleRelocationPlan>,
    depth: VerificationDepth,
) -> RecycleBinRelocationReport {
    let mut report = RecycleBinRelocationReport::default();
    for plan in plans {
        report.merge(relocate_plan(&plan, RelocationOps::at(depth)).await);
    }
    report
}

// disk-guard: entries listed before a failed listing step are still relocated, and owner
// listings fail whole.
#[allow(clippy::disallowed_methods)]
async fn relocate_plan(
    plan: &RecycleRelocationPlan,
    ops: RelocationOps,
) -> RecycleBinRelocationReport {
    let mut report = RecycleBinRelocationReport::default();
    let from = &plan.from;
    let from_display = from.to_string_lossy().into_owned();

    match tokio::fs::metadata(from).await {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => return report,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return report,
        Err(error) => {
            report.failures.push(RecycleEntryRelocationFailure {
                entry_id: String::new(),
                from_path: from_display,
                reason: format!("the previous recycle folder could not be read: {error}"),
            });
            return report;
        }
    }
    // Without the root sentinel the bin never managed this folder, so it holds
    // nothing that is ours to move.
    if !matches!(
        tokio::fs::symlink_metadata(from.join(RECYCLE_ROOT_SENTINEL)).await,
        Ok(metadata) if metadata.is_file()
    ) {
        return report;
    }

    let mut entry_dirs = Vec::new();
    match tokio::fs::read_dir(from).await {
        Ok(mut entries) => loop {
            match entries.next_entry().await {
                Ok(Some(entry)) => entry_dirs.push(entry.path()),
                Ok(None) => break,
                Err(error) => {
                    report.failures.push(RecycleEntryRelocationFailure {
                        entry_id: String::new(),
                        from_path: from_display.clone(),
                        reason: format!("the previous recycle folder could not be listed: {error}"),
                    });
                    break;
                }
            }
        },
        Err(error) => {
            report.failures.push(RecycleEntryRelocationFailure {
                entry_id: String::new(),
                from_path: from_display,
                reason: format!("the previous recycle folder could not be listed: {error}"),
            });
            return report;
        }
    }
    entry_dirs.sort();

    let mut prepared_targets: Vec<(PathBuf, Result<(), String>)> = Vec::new();
    for entry_dir in entry_dirs {
        let Some((entry_id, manifest)) = recognized_entry(&entry_dir).await else {
            continue;
        };
        let fail = |reason: String| RecycleEntryRelocationFailure {
            entry_id: entry_id.clone(),
            from_path: entry_dir.to_string_lossy().into_owned(),
            reason,
        };
        if !manifest.is_committed() {
            let status = manifest.status.as_deref().unwrap_or("unknown");
            report.failures.push(fail(format!(
                "the entry is not committed (status {status}), so it was left in place"
            )));
            continue;
        }
        let Some(target) = relocation_target_for(plan, &manifest) else {
            report.failures.push(fail(
                "no new recycle folder matches the library this entry came from".to_string(),
            ));
            continue;
        };
        if normalize_path(target) == normalize_path(from) {
            continue;
        }

        let prepared = match prepared_targets
            .iter()
            .find(|(base, _)| base == target)
            .map(|(_, result)| result.clone())
        {
            Some(result) => result,
            None => {
                let result = prepare_relocation_target(target).await;
                prepared_targets.push((target.clone(), result.clone()));
                result
            }
        };
        if let Err(reason) = prepared {
            report.failures.push(fail(reason));
            continue;
        }

        match relocate_entry(&entry_dir, &target.join(&entry_id), &manifest, &ops).await {
            Ok(RelocatedEntry::Moved) => report.moved_count += 1,
            Ok(RelocatedEntry::MovedWithLeftovers(reason)) => {
                report.moved_count += 1;
                report.failures.push(fail(reason));
            }
            Err(reason) => report.failures.push(fail(reason)),
        }
    }

    if report.moved_count > 0 || !report.failures.is_empty() {
        info!(
            from = %from.display(),
            moved = report.moved_count,
            failed = report.failures.len(),
            "moved recycle bin entries to the new recycle location"
        );
    }
    report
}

/// The entry id and manifest of a directory the bin created, or `None` for
/// anything else, which relocation must leave alone.
async fn recognized_entry(entry_dir: &Path) -> Option<(String, RecycleManifest)> {
    let entry_id = entry_dir.file_name()?.to_str()?.to_string();
    if validate_recycle_entry_id(&entry_id).is_err() || !generated_entry_id(&entry_id) {
        return None;
    }
    let disk = Disk::process();
    if !disk.uncached(entry_dir).await.is(EntryKind::Directory)
        || !disk
            .uncached(manifest_path(entry_dir))
            .await
            .is(EntryKind::File)
    {
        return None;
    }
    let manifest = read_manifest(entry_dir).await.ok()??;
    if !manifest.is_schema_current() || manifest.entry_id.as_deref() != Some(entry_id.as_str()) {
        return None;
    }
    Some((entry_id, manifest))
}

fn relocation_target_for<'a>(
    plan: &'a RecycleRelocationPlan,
    manifest: &RecycleManifest,
) -> Option<&'a PathBuf> {
    if let [target] = plan.targets.as_slice() {
        return Some(&target.base_path);
    }
    let original_path = manifest.original_path_buf();
    plan.targets
        .iter()
        .flat_map(|target| {
            target
                .media_roots
                .iter()
                .map(move |root| (root, &target.base_path))
        })
        .filter(|(root, _)| source_file_is_under_configured_root(&original_path, root))
        .max_by_key(|(root, _)| root.components().count())
        .map(|(_, base)| base)
        .or_else(|| {
            let media_root = manifest.media_root.as_deref()?.trim();
            plan.targets
                .iter()
                .find(|target| {
                    target
                        .media_roots
                        .iter()
                        .any(|root| normalize_path(root) == normalize_path(Path::new(media_root)))
                })
                .map(|target| &target.base_path)
        })
}

async fn prepare_relocation_target(base_path: &Path) -> Result<(), String> {
    let config = RecycleBinConfig {
        enabled: true,
        base_path: base_path.to_path_buf(),
        retention_days: DEFAULT_RETENTION_DAYS,
        cleanup_enabled: true,
        validation_error: None,
        source_roots: Vec::new(),
    };
    ensure_recycle_root(&config)
        .await
        .map_err(|error| format!("the new recycle folder could not be prepared: {error}"))
}

enum RelocatedEntry {
    Moved,
    /// The entry is complete at its new location, but part of the previous
    /// copy could not be removed.
    MovedWithLeftovers(String),
}

async fn relocate_entry(
    entry_dir: &Path,
    dest: &Path,
    manifest: &RecycleManifest,
    ops: &RelocationOps,
) -> Result<RelocatedEntry, String> {
    let reading = Disk::process().uncached(dest).await;
    match reading.exists() {
        Some(true) => return Err(relocation_collision(dest)),
        Some(false) => {}
        None => {
            return Err(format!(
                "could not check the destination {}",
                unreadable(dest, reading)
            ));
        }
    }

    if ops.allow_rename {
        // A same-filesystem rename moves the entry in one step, so the entry
        // is only ever in one place and there is no copy to verify. Only the
        // no-replace rename is used: a plain rename would replace an empty
        // directory that appeared at the destination.
        match Disk::process().move_onto_free_name(entry_dir, dest).await {
            Ok(FreeNameMove::Moved) => return Ok(RelocatedEntry::Moved),
            Ok(FreeNameMove::NotOffered) => {}
            Err(error) => match error.reason() {
                crate::disk::reason::DISK_DESTINATION_EXISTS
                | crate::disk::reason::DISK_DIRECTORY_NOT_EMPTY => {
                    return Err(relocation_collision(dest));
                }
                crate::disk::reason::DISK_MOVE_NEEDS_COPY => {}
                _ => {
                    return Err(format!(
                        "could not move the entry to {}: {}",
                        dest.display(),
                        error.words()
                    ));
                }
            },
        }
    }

    copy_and_verify_entry(entry_dir, dest, manifest, ops).await?;

    // The new copy is proven complete; only now does the previous one go.
    match crate::fs_safety::remove_dir_all_safely(entry_dir).await {
        Ok(()) => Ok(RelocatedEntry::Moved),
        Err(error) => Ok(RelocatedEntry::MovedWithLeftovers(format!(
            "the entry was copied to {} and verified, but the previous copy could not be fully removed: {error}",
            dest.display()
        ))),
    }
}

fn relocation_collision(dest: &Path) -> String {
    format!(
        "an entry with the same id already exists at {}; nothing was overwritten",
        dest.display()
    )
}

/// Copy `entry_dir` to a new `dest` and prove every file arrived intact. On
/// failure the partial copy this call created is removed; the source is never
/// touched.
// disk-guard: a failed copy is removed with std's tree removal, which reads no extra
// metadata per entry.
#[allow(clippy::disallowed_methods)]
async fn copy_and_verify_entry(
    entry_dir: &Path,
    dest: &Path,
    manifest: &RecycleManifest,
    ops: &RelocationOps,
) -> Result<(), String> {
    let (dirs, files) = collect_entry_tree_off_runtime(entry_dir).await?;

    // Creating the directory itself is the claim: it fails when anything
    // already holds the name, so the copy never lands in someone else's entry.
    match tokio::fs::create_dir(dest).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(relocation_collision(dest));
        }
        Err(error) => {
            return Err(format!(
                "could not create {} for the copy: {error}",
                dest.display()
            ));
        }
    }

    let outcome = match copy_entry_tree(entry_dir, dest, &dirs, &files, ops).await {
        Ok(()) => verify_entry_copy(dest, &files, manifest).await,
        Err(reason) => Err(reason),
    };
    if let Err(reason) = outcome {
        return Err(match tokio::fs::remove_dir_all(dest).await {
            Ok(()) => reason,
            Err(error) => format!(
                "{reason}; the partial copy at {} could not be removed: {error}",
                dest.display()
            ),
        });
    }
    Ok(())
}

async fn collect_entry_tree_off_runtime(
    entry_dir: &Path,
) -> Result<(Vec<PathBuf>, Vec<PathBuf>), String> {
    let entry_dir = entry_dir.to_path_buf();
    tokio::task::spawn_blocking(move || collect_entry_tree(&entry_dir))
        .await
        .map_err(|error| format!("the copy task failed: {error}"))?
}

/// Relative directories and files of an entry. Anything other than plain
/// directories and regular files makes the entry unsafe to copy.
fn collect_entry_tree(entry_dir: &Path) -> Result<(Vec<PathBuf>, Vec<PathBuf>), String> {
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    let mut pending = vec![PathBuf::new()];
    while let Some(relative) = pending.pop() {
        let absolute = entry_dir.join(&relative);
        let entries = Disk::process().list_blocking(&absolute);
        for entry in entries.map_err(|error| format!("could not read: {error}"))? {
            let child = relative.join(entry.path.file_name().unwrap_or_default());
            match entry.reading.kind() {
                Some(EntryKind::Symlink) => {
                    let at = entry.path.display();
                    return Err(format!("the entry contains a symbolic link at {at}"));
                }
                Some(EntryKind::Directory) => {
                    dirs.push(child.clone());
                    pending.push(child);
                }
                Some(EntryKind::File) => files.push(child),
                Some(EntryKind::Other) => {
                    return Err(format!(
                        "the entry contains something that is not a regular file at {}",
                        entry.path.display()
                    ));
                }
                None => return Err(format!("could not inspect {}", entry.path.display())),
            }
        }
    }
    dirs.sort();
    files.sort();
    Ok((dirs, files))
}

/// Copy every file through the verified copier. A file counts only when its
/// copy was read back in full and matched, and the source still holds the
/// bytes that were copied.
async fn copy_entry_tree(
    entry_dir: &Path,
    dest: &Path,
    dirs: &[PathBuf],
    files: &[PathBuf],
    ops: &RelocationOps,
) -> Result<(), String> {
    for dir in dirs {
        let target = dest.join(dir);
        (Disk::process()
            .create_dir(&target, Parents::MustExist)
            .await)
            .map_err(|error| format!("could not create: {error}"))?;
    }
    for file in files {
        let source = entry_dir.join(file);
        let target = dest.join(file);
        let verified = copy_and_verify(
            &ops.copier,
            &source,
            &target,
            placement::verified_copy(ops.depth, DestinationClaim::ClaimHere),
            &CopyProgress::none(),
        )
        .await
        .map_err(|error| {
            format!(
                "could not copy {} to {}: {error}",
                source.display(),
                target.display()
            )
        })?;
        if !verified.permits_source_removal() || verified.depth.fell_back {
            let detail = verified
                .detail
                .map(|detail| format!(": {detail}"))
                .unwrap_or_default();
            return Err(format!(
                "verification failed: {} was not proven to match the entry's copy{detail}",
                target.display()
            ));
        }
        // The entry's file must still hold what was copied, at the same
        // depth: at full depth its full hash is the copy's, at quick
        // depth it and the copy are sampled now.
        let held = match ops.depth {
            VerificationDepth::Full => {
                match Disk::process()
                    .full_hash(crate::disk::HashAsk::new(source.as_path()))
                    .await
                {
                    crate::disk::FullHashAnswer::Read(known)
                    | crate::disk::FullHashAnswer::Stored(known)
                    | crate::disk::FullHashAnswer::Cached(known) => {
                        verified.hashes.as_ref().is_some_and(|copied| {
                            copied.full_blake3 == known.blake3
                                && copied.size_bytes == known.size_bytes
                        })
                    }
                    crate::disk::FullHashAnswer::Unreadable(why) => {
                        return Err(format!("could not re-check {}: {why}", source.display()));
                    }
                    _ => false,
                }
            }
            VerificationDepth::Quick => {
                match Disk::process()
                    .check(
                        source.as_path(),
                        target.as_path(),
                        crate::disk::Depth::Quick,
                    )
                    .await
                {
                    Check::Same => true,
                    Check::Unread(why) => {
                        return Err(format!("could not re-check {}: {why}", source.display()));
                    }
                    Check::Differs | Check::Changed => false,
                }
            }
        };
        if !held {
            return Err(format!(
                "{} changed while it was being copied",
                source.display()
            ));
        }
    }
    Ok(())
}

async fn verify_entry_copy(
    dest: &Path,
    files: &[PathBuf],
    manifest: &RecycleManifest,
) -> Result<(), String> {
    let (_, dest_files) = collect_entry_tree_off_runtime(dest).await?;
    if dest_files != files {
        return Err(format!(
            "the copy at {} does not hold the same files as the entry",
            dest.display()
        ));
    }

    let manifest_bytes = (Disk::process().read(manifest_path(dest)).await)
        .map_err(|error| format!("could not read the copied manifest: {error}"))?;
    let copied_manifest: RecycleManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("the copied manifest could not be parsed: {error}"))?;
    if !copied_manifest.is_schema_current()
        || copied_manifest.entry_id != manifest.entry_id
        || copied_manifest.recycled_at != manifest.recycled_at
    {
        return Err("the copied manifest does not match the entry".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(any(unix, windows))]
    #[test]
    fn native_restore_names_and_root_boundaries_preserve_opaque_identity() {
        use crate::stored_paths::stored_path_to_path_buf;
        let (platform, markers) = if cfg!(windows) {
            ("w", ["%uD800", "%uD801"])
        } else {
            ("u", ["%FF", "%FE"])
        };
        let [root, other] = markers.map(|marker| {
            stored_path_to_path_buf(&format!("scryer-path-v1:{platform}:/media/root{marker}"))
        });
        assert_eq!(root.to_string_lossy(), other.to_string_lossy());
        let source = root.join(stored_path_to_path_buf(&format!(
            "scryer-path-v1:{platform}:movie{}.mkv",
            markers[0]
        )));
        assert!(path_is_under_configured_root(&source, &root));
        assert!(!path_is_under_configured_root(&source, &other));
        assert!(!path_is_under_configured_root(
            &root.join("../outside.mkv"),
            &root
        ));
        let sibling = restore_candidate_path(&source, 2);
        let expected = root.join(stored_path_to_path_buf(&format!(
            "scryer-path-v1:{platform}:movie{}-restored-2.mkv",
            markers[0]
        )));
        assert_eq!(sibling, expected);
    }
    use crate::location::verify::{ReadBackHandle, open_cache_bypassed};
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
            episode_ids: None,
            media_row: None,
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
            episode_ids: None,
            media_row: None,
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

    #[test]
    fn recycle_manifest_media_row_round_trips_and_legacy_manifests_parse() {
        let mut manifest = test_manifest();
        manifest.media_row = Some(RecycledMediaRowSnapshot {
            acquisition_score: Some(512),
            scoring_log: Some("invented scoring log".to_string()),
            ..Default::default()
        });
        let encoded = serde_json::to_value(&manifest).unwrap();
        let decoded: RecycleManifest = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded.media_row, manifest.media_row);

        let mut legacy = serde_json::to_value(test_manifest()).unwrap();
        assert!(
            legacy.get("media_row").is_none(),
            "an absent snapshot is not written"
        );
        legacy
            .as_object_mut()
            .unwrap()
            .insert("schema".to_string(), RECYCLE_MANIFEST_SCHEMA.into());
        let parsed: RecycleManifest = serde_json::from_value(legacy).unwrap();
        assert_eq!(parsed.media_row, None);
        assert_eq!(parsed.schema.as_deref(), Some(RECYCLE_MANIFEST_SCHEMA));
    }

    #[test]
    fn recycle_manifest_release_listing_round_trips_and_older_snapshots_parse() {
        let mut manifest = test_manifest();
        manifest.media_row = Some(RecycledMediaRowSnapshot {
            acquisition_score: Some(64),
            release_listing_json: Some(r#"{"votes":{"up":2},"password":false}"#.to_string()),
            ..Default::default()
        });
        let encoded = serde_json::to_string(&manifest).unwrap();
        let decoded: RecycleManifest = serde_json::from_str(&encoded).unwrap();
        assert_eq!(
            decoded
                .media_row
                .as_ref()
                .and_then(|row| row.release_listing_json.as_deref()),
            Some(r#"{"votes":{"up":2},"password":false}"#)
        );

        let older = serde_json::json!({
            "schema": RECYCLE_MANIFEST_SCHEMA,
            "recycled_at": "2031-01-02T03:04:05Z",
            "original_path": "/data/movies/Invented Film (2031)/Invented Film.mkv",
            "size_bytes": 10,
            "reason": "upgrade_replaced",
            "media_row": { "acquisition_score": 12, "release_hash": "abc123" }
        });
        let parsed: RecycleManifest = serde_json::from_value(older).unwrap();
        let row = parsed.media_row.expect("older snapshot still parses");
        assert_eq!(row.acquisition_score, Some(12));
        assert_eq!(row.release_hash.as_deref(), Some("abc123"));
        assert_eq!(row.release_listing_json, None);
    }

    #[test]
    fn recycle_payload_mapping_reserves_metadata_aliases() {
        for name in [
            "manifest.json",
            "Manifest.JSON",
            "MANIFEST.JSON.",
            "manifest.json ",
            "manifest.json. ",
        ] {
            assert_eq!(
                payload_relative_path(Path::new(name)),
                Path::new(".scryer-payload").join(name)
            );
        }
        assert_eq!(
            payload_relative_path(Path::new("ordinary.mkv")),
            PathBuf::from("ordinary.mkv")
        );
    }

    #[tokio::test]
    async fn recycle_metadata_named_payloads_roundtrip_without_overwriting_manifest() {
        let names = [
            "manifest.json",
            "Manifest.JSON",
            "MANIFEST.JSON",
            "ordinary.mkv",
            "MANIFE~1.JSO",
            // Trailing dots and spaces are only distinct names off Windows.
            #[cfg(unix)]
            "manifest.json.",
            #[cfg(unix)]
            "manifest.json ",
        ];
        for name in names {
            for prepared in [false, true] {
                let temp = TempDir::new().unwrap();
                let root = std::fs::canonicalize(temp.path()).unwrap();
                let source = root.join(name);
                let bytes = b"original operator file; never entry metadata";
                std::fs::write(&source, bytes).unwrap();
                let config = test_config(&root.join("bin"));
                let mut manifest = test_manifest();
                manifest.original_path = crate::stored_paths::path_to_stored_string(&source);
                manifest.source_operation_id = Some("synthetic-operation".into());
                manifest.size_bytes = bytes.len() as u64;
                let (entry, payload) = if prepared {
                    let receipt = prepare_conflict_recycle(&config, &source, manifest)
                        .await
                        .unwrap();
                    let version = crate::location::resolution::file_version(&source)
                        .await
                        .unwrap();
                    execute_conflict_recycle(
                        &config,
                        &source,
                        &receipt,
                        &version,
                        VerificationDepth::Full,
                    )
                    .await
                    .unwrap();
                    (
                        config.base_path.join(&receipt.entry_id),
                        crate::stored_paths::stored_path_to_path_buf(&receipt.recycled_path),
                    )
                } else {
                    let result = recycle_file(&config, &source, manifest, VerificationDepth::Full)
                        .await
                        .unwrap()
                        .unwrap();
                    (result.entry_dir, result.recycled_path)
                };
                assert!(!source.exists(), "{name}, prepared={prepared}");
                assert_eq!(std::fs::read(&payload).unwrap(), bytes);
                let manifest = read_manifest(&entry).await.unwrap().unwrap();
                assert_eq!(manifest.status.as_deref(), Some(RECYCLE_STATUS_COMMITTED));
                assert_eq!(manifest.size_bytes, bytes.len() as u64);
                assert_ne!(payload, manifest_path(&entry));
                assert_eq!(
                    recycled_payload_path(&entry, &manifest).await.unwrap(),
                    payload
                );
                let listed = list_committed_entries(&config).await.unwrap();
                assert_eq!(listed.len(), 1);
                let restored = restore_from_recycle_with_roots(
                    &payload,
                    &source,
                    false,
                    &[root],
                    VerificationDepth::Full,
                )
                .await
                .unwrap();
                assert_eq!(restored, source);
                assert_eq!(std::fs::read(restored).unwrap(), bytes);
                assert_eq!(
                    read_manifest(&entry).await.unwrap().unwrap().original_path,
                    manifest.original_path
                );
            }
        }
    }

    #[tokio::test]
    async fn recycle_legacy_pending_receipts_preserve_metadata_and_restore_original_name() {
        for name in ["manifest.json", "Manifest.JSON", "ordinary.mkv"] {
            let temp = TempDir::new().unwrap();
            let root = std::fs::canonicalize(temp.path()).unwrap();
            let source = root.join(name);
            let bytes = b"legacy pathname payload";
            std::fs::write(&source, bytes).unwrap();
            let config = test_config(&root.join("bin"));
            let mut manifest = test_manifest();
            manifest.original_path = crate::stored_paths::path_to_stored_string(&source);
            manifest.source_operation_id = Some("synthetic-operation".into());
            let mut legacy = prepare_conflict_recycle(&config, &source, manifest)
                .await
                .unwrap();
            let old_entry = config.base_path.join(&legacy.entry_id);
            legacy.recycled_path = crate::stored_paths::path_to_stored_string(old_entry.join(name));
            let metadata_bytes = std::fs::read(manifest_path(&old_entry)).unwrap();
            let aliases_metadata = payload_is_manifest(
                &old_entry,
                &crate::stored_paths::stored_path_to_path_buf(&legacy.recycled_path),
            )
            .await
            .unwrap();
            let version = crate::location::resolution::file_version(&source)
                .await
                .unwrap();
            if aliases_metadata {
                assert!(
                    execute_conflict_recycle(
                        &config,
                        &source,
                        &legacy,
                        &version,
                        VerificationDepth::Full
                    )
                    .await
                    .is_err()
                );
                assert_eq!(std::fs::read(&source).unwrap(), bytes);
            }
            let receipt = renew_conflict_recycle(&config, &source, &legacy)
                .await
                .unwrap();
            assert_eq!(receipt.entry_id != legacy.entry_id, aliases_metadata);
            assert_eq!(
                std::fs::read(manifest_path(&old_entry)).unwrap(),
                metadata_bytes
            );
            execute_conflict_recycle(
                &config,
                &source,
                &receipt,
                &version,
                VerificationDepth::Full,
            )
            .await
            .unwrap();
            let entry = config.base_path.join(&receipt.entry_id);
            let manifest = read_manifest(&entry).await.unwrap().unwrap();
            let payload = recycled_payload_path(&entry, &manifest).await.unwrap();
            assert_eq!(std::fs::read(&payload).unwrap(), bytes);
            restore_from_recycle_with_roots(
                &payload,
                &source,
                false,
                &[root],
                VerificationDepth::Full,
            )
            .await
            .unwrap();
            assert_eq!(std::fs::read(&source).unwrap(), bytes);
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn recycle_payload_lookup_preserves_symlinked_parent_and_external_contents() {
        let temp = TempDir::new().unwrap();
        let root = std::fs::canonicalize(temp.path()).unwrap();
        let source = root.join("manifest.json");
        std::fs::write(&source, b"selected original").unwrap();
        let config = test_config(&root.join("bin"));
        let mut manifest = test_manifest();
        manifest.original_path = crate::stored_paths::path_to_stored_string(&source);
        let result = recycle_file(&config, &source, manifest, VerificationDepth::Full)
            .await
            .unwrap()
            .unwrap();
        let manifest = read_manifest(&result.entry_dir).await.unwrap().unwrap();
        let owned = result.entry_dir.join("preserved-original");
        std::fs::rename(result.recycled_path.parent().unwrap(), &owned).unwrap();
        let external = root.join("external");
        std::fs::create_dir(&external).unwrap();
        std::fs::write(external.join("manifest.json"), b"unrelated external").unwrap();
        std::os::unix::fs::symlink(&external, result.entry_dir.join(".scryer-payload")).unwrap();
        assert!(
            recycled_payload_path(&result.entry_dir, &manifest)
                .await
                .is_err()
        );
        assert!(list_committed_entries(&config).await.unwrap().is_empty());
        assert!(
            !purge_committed_entry(
                &config,
                &CommittedRecycleEntry {
                    entry_dir: result.entry_dir,
                    manifest
                }
            )
            .await
            .unwrap()
        );
        assert_eq!(
            std::fs::read(external.join("manifest.json")).unwrap(),
            b"unrelated external"
        );
        assert_eq!(
            std::fs::read(owned.join("manifest.json")).unwrap(),
            b"selected original"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn prepared_recycle_preserves_source_when_payload_parent_becomes_symlink() {
        let temp = TempDir::new().unwrap();
        let root = std::fs::canonicalize(temp.path()).unwrap();
        let source = root.join("manifest.json");
        std::fs::write(&source, b"selected original").unwrap();
        let config = test_config(&root.join("bin"));
        let mut manifest = test_manifest();
        manifest.original_path = crate::stored_paths::path_to_stored_string(&source);
        manifest.source_operation_id = Some("synthetic-operation".into());
        let receipt = prepare_conflict_recycle(&config, &source, manifest)
            .await
            .unwrap();
        let entry = config.base_path.join(&receipt.entry_id);
        std::fs::rename(
            entry.join(".scryer-payload"),
            entry.join("preserved-empty-directory"),
        )
        .unwrap();
        let external = root.join("external");
        std::fs::create_dir(&external).unwrap();
        std::os::unix::fs::symlink(&external, entry.join(".scryer-payload")).unwrap();
        let version = crate::location::resolution::file_version(&source)
            .await
            .unwrap();
        assert!(
            renew_conflict_recycle(&config, &source, &receipt)
                .await
                .is_err()
        );
        assert!(
            execute_conflict_recycle(
                &config,
                &source,
                &receipt,
                &version,
                VerificationDepth::Full
            )
            .await
            .is_err()
        );
        assert_eq!(std::fs::read(&source).unwrap(), b"selected original");
        assert!(!external.join("manifest.json").exists());
        assert_eq!(
            read_manifest(&entry)
                .await
                .unwrap()
                .unwrap()
                .status
                .as_deref(),
            Some(RECYCLE_STATUS_CONFLICT_PENDING)
        );
    }

    #[tokio::test]
    async fn media_issue_recycle_refuses_a_replaced_source_and_preserves_both_files() {
        let temp = TempDir::new().unwrap();
        let root = std::fs::canonicalize(temp.path()).unwrap();
        let source = root.join("reported.mkv");
        let original = root.join("original.mkv");
        std::fs::write(&source, b"first file").unwrap();
        let config = test_config(&root.join("bin"));
        let mut manifest = test_manifest();
        manifest.original_path = crate::stored_paths::path_to_stored_string(&source);
        manifest.source_operation_id = Some("synthetic-media-issue".to_owned());
        let version = crate::location::resolution::file_version(&source)
            .await
            .unwrap();
        let receipt = prepare_conflict_recycle(&config, &source, manifest)
            .await
            .unwrap();
        std::fs::rename(&source, &original).unwrap();
        std::fs::write(&source, b"first file").unwrap();
        let refusal = execute_new_conflict_recycle(&config, &source, &receipt, &version, None)
            .await
            .unwrap_err();
        assert_eq!(
            refusal.validation_reason(),
            Some(crate::delete_refusal::PREVIEW_STALE)
        );
        assert_eq!(std::fs::read(source).unwrap(), b"first file");
        assert_eq!(std::fs::read(original).unwrap(), b"first file");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn media_issue_recycle_accepts_a_legacy_version_and_preserves_unrelated_files() {
        let temp = TempDir::new().unwrap();
        let root = std::fs::canonicalize(temp.path()).unwrap();
        let source = root.join("reported.mkv");
        let unrelated = root.join("unrelated.mkv");
        std::fs::write(&source, b"reviewed bytes").unwrap();
        std::fs::write(&unrelated, b"unrelated bytes").unwrap();
        std::fs::File::open(&source)
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH - std::time::Duration::from_millis(1500))
            .unwrap();
        let config = test_config(&root.join("bin"));
        let mut manifest = test_manifest();
        manifest.original_path = crate::stored_paths::path_to_stored_string(&source);
        manifest.source_operation_id = Some("synthetic-media-issue".to_owned());
        let version = crate::location::resolution::file_version(&source)
            .await
            .unwrap();
        let receipt = prepare_conflict_recycle(&config, &source, manifest)
            .await
            .unwrap();

        execute_new_conflict_recycle(&config, &source, &receipt, &version, None)
            .await
            .unwrap();

        assert!(!source.exists());
        assert_eq!(
            std::fs::read(crate::stored_paths::stored_path_to_path_buf(
                &receipt.recycled_path
            ))
            .unwrap(),
            b"reviewed bytes"
        );
        assert_eq!(std::fs::read(unrelated).unwrap(), b"unrelated bytes");
    }

    /// A recycle that stopped after its move, before the entry was committed,
    /// finishes from the saved receipt. The reported path is already empty,
    /// so revalidating it first would refuse a move that only needs finishing.
    #[cfg(unix)]
    #[tokio::test]
    async fn media_issue_recycle_interrupted_after_the_move_finishes_from_its_receipt() {
        let temp = TempDir::new().unwrap();
        let root = std::fs::canonicalize(temp.path()).unwrap();
        let source = root.join("reported.mkv");
        let unrelated = root.join("unrelated.mkv");
        std::fs::write(&source, b"reviewed bytes").unwrap();
        std::fs::write(&unrelated, b"unrelated bytes").unwrap();
        let config = test_config(&root.join("bin"));
        let mut manifest = test_manifest();
        manifest.original_path = crate::stored_paths::path_to_stored_string(&source);
        manifest.source_operation_id = Some("synthetic-media-issue".to_owned());
        let version = crate::location::resolution::file_version(&source)
            .await
            .unwrap();
        let proof = crate::fs_integrity::import_content_proof_async(&source)
            .await
            .unwrap();
        let receipt = prepare_conflict_recycle(&config, &source, manifest)
            .await
            .unwrap();

        // The move runs, then the recycle fails before committing the entry.
        let payload = crate::stored_paths::stored_path_to_path_buf(&receipt.recycled_path);
        std::fs::create_dir_all(payload.parent().unwrap()).unwrap();
        std::fs::rename(&source, &payload).unwrap();

        let issue = scryer_domain::MediaIssue {
            id: "synthetic-media-issue".to_string(),
            library_id: "library-fixture".to_string(),
            title_id: "title-fixture".to_string(),
            media_file_id: "media-file-fixture".to_string(),
            file_path_snapshot: crate::stored_paths::path_to_stored_string(&source),
            file_version: version,
            content: scryer_domain::BlocklistContentIdentity {
                scheme: crate::fs_integrity::IMPORT_CONTENT_PROOF_SCHEME.to_string(),
                size_bytes: proof.size_bytes,
                sample_bytes: proof.sample_bytes,
                sample_blake3: proof.sample_blake3.clone(),
            },
            reporter_user_id: "user-fixture".to_string(),
            reason: scryer_domain::MediaIssueReason::Corrupted,
            details: None,
            remediation: scryer_domain::MediaIssueRemediation::RecycleAndSearch,
            authorized_remediation: Some(scryer_domain::MediaIssueRemediation::RecycleAndSearch),
            status: scryer_domain::MediaIssueStatus::Approved,
            affected_episode_ids: Vec::new(),
            affected_series_movie_link_ids: Vec::new(),
            rule_decision_id: None,
            blocklist_id: None,
            reviewed_by_user_id: None,
            reviewed_at: None,
            execution_status: scryer_domain::MediaIssueExecutionStatus::Recycling,
            execution_error_code: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        };
        recover_reviewed_recycle(&config, &issue, &receipt, None)
            .await
            .unwrap();

        assert!(!source.exists());
        assert_eq!(std::fs::read(&payload).unwrap(), b"reviewed bytes");
        assert_eq!(std::fs::read(unrelated).unwrap(), b"unrelated bytes");

        // A receipt that no longer matches the report is refused and moves
        // nothing back or further.
        let mut other = issue.clone();
        other.id = "another-media-issue".to_string();
        let refusal = recover_reviewed_recycle(&config, &other, &receipt, None)
            .await
            .unwrap_err();
        assert_eq!(
            refusal.validation_reason(),
            Some(crate::delete_refusal::PREVIEW_STALE)
        );
        assert_eq!(std::fs::read(&payload).unwrap(), b"reviewed bytes");
    }

    /// A receipt saved before linked copies existed reads back unchanged, and
    /// one without a linked copy is written exactly as before.
    #[test]
    fn a_media_issue_receipt_without_a_linked_copy_keeps_its_saved_form() {
        let file = PreparedRecycleFile {
            entry_id: "synthetic-entry".to_string(),
            original_path: "/synthetic/reported.mkv".to_string(),
            recycled_path: "/synthetic/bin/synthetic-entry/reported.mkv".to_string(),
            source_operation_id: "synthetic-media-issue".to_string(),
        };
        let saved = serde_json::to_string(&file).unwrap();
        let read: MediaIssueRecycleReceipt = serde_json::from_str(&saved).unwrap();
        assert_eq!(read.file, file);
        assert_eq!(read.linked_copy, None);
        assert_eq!(serde_json::to_string(&read).unwrap(), saved);
    }

    /// A reported file whose other name is a proven linked copy moves into
    /// its entry and keeps that copy; with a third name it is moved back.
    #[cfg(unix)]
    #[tokio::test]
    async fn media_issue_recycle_moves_one_name_of_a_proven_linked_pair_only() {
        for third_name in [false, true] {
            let temp = TempDir::new().unwrap();
            let root = std::fs::canonicalize(temp.path()).unwrap();
            let source = root.join("reported.mkv");
            let copy = root.join("download-copy.mkv");
            let unrelated = root.join("unrelated.mkv");
            std::fs::write(&source, b"reviewed bytes").unwrap();
            std::fs::hard_link(&source, &copy).unwrap();
            if third_name {
                std::fs::hard_link(&source, root.join("third-name.mkv")).unwrap();
            }
            std::fs::write(&unrelated, b"unrelated bytes").unwrap();
            let config = test_config(&root.join("bin"));
            let mut manifest = test_manifest();
            manifest.original_path = crate::stored_paths::path_to_stored_string(&source);
            manifest.source_operation_id = Some("synthetic-media-issue".to_owned());
            let version = crate::location::resolution::file_version(&source)
                .await
                .unwrap();
            let receipt = prepare_conflict_recycle(&config, &source, manifest)
                .await
                .unwrap();

            let result =
                execute_new_conflict_recycle(&config, &source, &receipt, &version, Some(&copy))
                    .await;

            let payload = crate::stored_paths::stored_path_to_path_buf(&receipt.recycled_path);
            if third_name {
                assert!(result.is_err());
                assert_eq!(std::fs::read(&source).unwrap(), b"reviewed bytes");
                assert!(!payload.exists());
            } else {
                result.unwrap();
                assert!(!source.exists());
                assert_eq!(std::fs::read(&payload).unwrap(), b"reviewed bytes");
            }
            assert_eq!(std::fs::read(&copy).unwrap(), b"reviewed bytes");
            assert_eq!(std::fs::read(&unrelated).unwrap(), b"unrelated bytes");
        }
    }

    #[tokio::test]
    async fn test_recycle_creates_dir_and_manifest() {
        let tmp = TempDir::new().unwrap();
        let recycle_dir = tmp.path().join("recycle");
        let source = tmp.path().join("test.mkv");
        tokio::fs::write(&source, b"video data").await.unwrap();

        let config = test_config(&recycle_dir);
        let result = recycle_file(&config, &source, test_manifest(), VerificationDepth::Full)
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

        let result = recycle_file(&config, &source, manifest, VerificationDepth::Full)
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
        let result = recycle_file(&config, &source, test_manifest(), VerificationDepth::Full)
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
        let result = recycle_file(&config, &link, test_manifest(), VerificationDepth::Full).await;

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
            media_row: None,
            episode_ids: Vec::new(),
        };
        let result =
            recycle_replaced_media_file(&config, &source, metadata, false, VerificationDepth::Full)
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
    async fn an_upgrade_entry_records_the_episodes_its_file_covered() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp.path().join("recycle"));
        let mut recorded = Vec::new();
        for (name, episode_ids) in [
            (
                "episode.mkv",
                vec![
                    "synthetic-episode-2".to_string(),
                    "synthetic-episode-1".to_string(),
                    "synthetic-episode-2".to_string(),
                ],
            ),
            ("movie.mkv", Vec::new()),
        ] {
            let source = tmp.path().join(name);
            tokio::fs::write(&source, b"old video").await.unwrap();
            let metadata = ReplacedMediaRecycleMetadata {
                original_path: source.to_str().unwrap(),
                original_file_id: "old-file",
                size_bytes: 9,
                title_id: "title",
                media_root: Some(tmp.path().to_str().unwrap()),
                media_row: None,
                episode_ids,
            };
            let result = recycle_replaced_media_file(
                &config,
                &source,
                metadata,
                false,
                VerificationDepth::Full,
            )
            .await
            .unwrap();
            let replacement = tmp.path().join(format!("new-{name}"));
            tokio::fs::write(&replacement, b"new video").await.unwrap();
            commit_recycle_entry(&result, "new-file", &replacement)
                .await
                .unwrap();
            let entry_dir = result.expect("pending recycle result").entry_dir;
            recorded.push(read_test_manifest(&entry_dir).await.episode_ids);
        }

        assert_eq!(
            recorded,
            vec![
                Some(vec![
                    "synthetic-episode-1".to_string(),
                    "synthetic-episode-2".to_string()
                ]),
                None
            ]
        );
        let mut older = serde_json::to_value(test_manifest()).unwrap();
        older.as_object_mut().unwrap().remove("episode_ids");
        let decoded: RecycleManifest = serde_json::from_value(older).unwrap();
        assert_eq!(decoded.episode_ids, None);
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

        let result = recycle_file(&config, &source, test_manifest(), VerificationDepth::Full)
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

        let result =
            recycle_file(&config, &outside, test_manifest(), VerificationDepth::Full).await;
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

        let result = recycle_file(&config, &source, test_manifest(), VerificationDepth::Full).await;
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

        let error = recycle_file(
            &config,
            &source_dir,
            test_manifest(),
            VerificationDepth::Full,
        )
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

        let result = recycle_file(
            &config,
            &tmp.path().join("nope.mkv"),
            test_manifest(),
            VerificationDepth::Full,
        )
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
        let result = recycle_file(&config, &source, test_manifest(), VerificationDepth::Full)
            .await
            .unwrap()
            .unwrap();

        assert!(!source.exists());

        let restored_to = restore_from_recycle(
            &result.recycled_path,
            &source,
            false,
            VerificationDepth::Full,
        )
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
        let result = recycle_file(&config, &source, test_manifest(), VerificationDepth::Full)
            .await
            .unwrap()
            .unwrap();

        restore_recycled_file_exact(&result.recycled_path, &source, VerificationDepth::Full)
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
        let result = recycle_file(&config, &source, test_manifest(), VerificationDepth::Full)
            .await
            .unwrap()
            .unwrap();

        link_recycled_file_to_exact_destination(
            &result.recycled_path,
            &source,
            VerificationDepth::Full,
        )
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
        let result =
            recycle_file_pending(&config, &source, test_manifest(), VerificationDepth::Full)
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
        let stale = recycle_file_pending(
            &config,
            &stale_source,
            test_manifest(),
            VerificationDepth::Full,
        )
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
        let fresh = recycle_file_pending(
            &config,
            &fresh_source,
            test_manifest(),
            VerificationDepth::Full,
        )
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

    /// Overwrites `path` at the moment the copier starts proving it, which
    /// is after the copy finished and before anything is compared.
    fn damage_when_verification_starts(path: &Path, bytes: &'static [u8]) -> CopyProgress {
        let path = path.to_path_buf();
        CopyProgress::none().with_transfer_sink(move |phase, verified_bytes| {
            if phase == scryer_domain::ImportTransferPhase::Verifying && verified_bytes == 0 {
                std::fs::write(&path, bytes).unwrap();
            }
        })
    }

    #[tokio::test]
    async fn recycle_by_copy_removes_the_source_once_the_copy_is_proven() {
        let tmp = TempDir::new().unwrap();
        let source = tmp.path().join("library").join("sample-feature.mkv");
        let recycled = tmp.path().join("bin").join("sample-feature.mkv");
        let bystander = tmp.path().join("library").join("other-feature.mkv");
        tokio::fs::create_dir_all(source.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::create_dir_all(recycled.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&source, b"library bytes").await.unwrap();
        tokio::fs::write(&bystander, b"unrelated").await.unwrap();

        recycle_by_verified_copy(
            &source,
            &recycled,
            CopyProgress::none(),
            VerificationDepth::Full,
        )
        .await
        .unwrap();

        assert!(!source.exists(), "the proven copy replaces the source");
        assert_eq!(std::fs::read(&recycled).unwrap(), b"library bytes");
        assert_eq!(std::fs::read(&bystander).unwrap(), b"unrelated");
    }

    #[tokio::test]
    async fn recycle_by_copy_keeps_the_source_when_the_copy_does_not_match() {
        let tmp = TempDir::new().unwrap();
        let source = tmp.path().join("sample-feature.mkv");
        let recycled = tmp.path().join("bin").join("sample-feature.mkv");
        tokio::fs::create_dir_all(recycled.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&source, b"library bytes").await.unwrap();

        let error = recycle_by_verified_copy(
            &source,
            &recycled,
            damage_when_verification_starts(&recycled, b"damaged"),
            VerificationDepth::Full,
        )
        .await
        .expect_err("an unproven copy must fail the recycle");

        assert!(
            error.to_string().contains("copy verification failed"),
            "unexpected error: {error}"
        );
        assert_eq!(std::fs::read(&source).unwrap(), b"library bytes");
        assert!(!recycled.exists(), "the unproven copy is removed");
    }

    #[tokio::test]
    async fn recycle_by_copy_never_replaces_an_occupied_destination() {
        let tmp = TempDir::new().unwrap();
        let source = tmp.path().join("sample-feature.mkv");
        let recycled = tmp.path().join("bin").join("sample-feature.mkv");
        tokio::fs::create_dir_all(recycled.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&source, b"library bytes").await.unwrap();
        tokio::fs::write(&recycled, b"already here").await.unwrap();

        recycle_by_verified_copy(
            &source,
            &recycled,
            CopyProgress::none(),
            VerificationDepth::Full,
        )
        .await
        .expect_err("an occupied destination must fail the recycle");

        assert_eq!(std::fs::read(&source).unwrap(), b"library bytes");
        assert_eq!(std::fs::read(&recycled).unwrap(), b"already here");
    }

    /// Puts a folder's mode back when the test ends, however it ends, so the
    /// temporary tree can be read and cleaned up.
    #[cfg(unix)]
    struct ModeBack(PathBuf);

    #[cfg(unix)]
    impl Drop for ModeBack {
        fn drop(&mut self) {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
        }
    }

    #[cfg(unix)]
    fn set_mode(folder: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(folder, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    /// Sets `folder`'s mode once the copy is written, as its proof starts.
    #[cfg(unix)]
    fn set_mode_when_verification_starts(folder: &Path, mode: u32) -> CopyProgress {
        let folder = folder.to_path_buf();
        CopyProgress::none().with_transfer_sink(move |phase, verified_bytes| {
            if phase == scryer_domain::ImportTransferPhase::Verifying && verified_bytes == 0 {
                set_mode(&folder, mode);
            }
        })
    }

    /// Whether this user is held to `folder`'s mode; a superuser is not, and
    /// the tests that need a refused removal do not apply to it.
    #[cfg(unix)]
    fn modes_bind(folder: &Path) -> bool {
        let _back = ModeBack(folder.to_path_buf());
        set_mode(folder, 0o555);
        let probe = folder.join("mode-probe");
        let wrote = std::fs::write(&probe, b"").is_ok();
        set_mode(folder, 0o755);
        let _ = std::fs::remove_file(&probe);
        !wrote
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_recycled_copy_goes_when_its_source_could_not_be_removed_but_is_still_there() {
        let tmp = TempDir::new().unwrap();
        let library = tmp.path().join("library");
        let source = library.join("sample-feature.mkv");
        let recycled = tmp.path().join("bin").join("sample-feature.mkv");
        let bystander = tmp.path().join("bin").join("other-feature.mkv");
        std::fs::create_dir_all(&library).unwrap();
        std::fs::create_dir_all(recycled.parent().unwrap()).unwrap();
        std::fs::write(&source, b"library bytes").unwrap();
        std::fs::write(&bystander, b"unrelated").unwrap();
        if !modes_bind(&library) {
            return;
        }
        let _back = ModeBack(library.clone());

        recycle_by_verified_copy(
            &source,
            &recycled,
            set_mode_when_verification_starts(&library, 0o555),
            VerificationDepth::Full,
        )
        .await
        .expect_err("a source that cannot be removed fails the recycle");

        assert_eq!(std::fs::read(&source).unwrap(), b"library bytes");
        assert!(
            !recycled.exists(),
            "the source still reads as present, so its copy is not kept"
        );
        assert_eq!(std::fs::read(&bystander).unwrap(), b"unrelated");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_recycled_copy_stays_when_its_source_could_not_be_removed_and_does_not_answer() {
        let tmp = TempDir::new().unwrap();
        let library = tmp.path().join("library");
        let source = library.join("sample-feature.mkv");
        let recycled = tmp.path().join("bin").join("sample-feature.mkv");
        let bystander = tmp.path().join("bin").join("other-feature.mkv");
        std::fs::create_dir_all(&library).unwrap();
        std::fs::create_dir_all(recycled.parent().unwrap()).unwrap();
        std::fs::write(&source, b"library bytes").unwrap();
        std::fs::write(&bystander, b"unrelated").unwrap();
        if !modes_bind(&library) {
            return;
        }
        let disk = Disk::process();
        // Declared first, so they drop last.
        let _roots = crate::disk::test_support::install_volumes(
            &disk,
            [(library.clone(), crate::disk::VolumeClass::LibraryRoot)],
        );
        let _back = ModeBack(library.clone());
        // Once the copy is written the source can no longer be removed, and
        // a read of its name no longer answers. The copy's proof still reads
        // the source through its own open.
        let silenced = std::sync::Arc::new(std::sync::Mutex::new(None));
        let progress = {
            let (library, source, silenced) = (library.clone(), source.clone(), silenced.clone());
            CopyProgress::none().with_transfer_sink(move |phase, verified_bytes| {
                if phase == scryer_domain::ImportTransferPhase::Verifying && verified_bytes == 0 {
                    set_mode(&library, 0o555);
                    *silenced.lock().unwrap() =
                        crate::disk::test_support::silence_path(&Disk::process(), &source);
                }
            })
        };

        recycle_by_verified_copy(&source, &recycled, progress, VerificationDepth::Full)
            .await
            .expect_err("a source that cannot be removed fails the recycle");
        assert!(
            silenced.lock().unwrap().take().is_some(),
            "the source's name was silenced"
        );

        assert_eq!(
            std::fs::read(&recycled).unwrap(),
            b"library bytes",
            "a source that does not read as present keeps its proven copy"
        );
        assert_eq!(std::fs::read(&source).unwrap(), b"library bytes");
        assert_eq!(std::fs::read(&bystander).unwrap(), b"unrelated");
    }

    #[tokio::test]
    async fn a_restore_that_fails_before_it_writes_leaves_the_file_it_would_replace() {
        let tmp = TempDir::new().unwrap();
        let recycled = tmp.path().join("bin").join("sample-feature.recycled");
        let original = tmp.path().join("library").join("sample-feature.mkv");
        let bystander = tmp.path().join("library").join("other-feature.mkv");
        std::fs::create_dir_all(original.parent().unwrap()).unwrap();
        std::fs::write(&original, b"the occupant").unwrap();
        std::fs::write(&bystander, b"unrelated").unwrap();

        proven_replacement(
            &recycled,
            &original,
            CopyProgress::none(),
            VerificationDepth::Quick,
        )
        .await
        .expect_err("a missing recycled file fails the restore");

        assert_eq!(std::fs::read(&original).unwrap(), b"the occupant");
        assert_eq!(std::fs::read(&bystander).unwrap(), b"unrelated");
        assert!(
            !crate::disk::resumable_staging_path(&original).exists(),
            "nothing is left under the staging name"
        );
    }

    /// An overwrite restore replaces the name, never the file behind it: a
    /// second name the occupant shared (a hard link to a seeding download,
    /// say) keeps the occupant's bytes, and the restored name holds the
    /// recycled bytes.
    #[tokio::test]
    async fn an_overwrite_restore_leaves_the_occupants_other_names_their_bytes() {
        let tmp = TempDir::new().unwrap();
        let recycled = tmp.path().join("bin").join("sample-feature.recycled");
        let original = tmp.path().join("library").join("sample-feature.mkv");
        let seeding = tmp.path().join("downloads").join("sample-feature.mkv");
        std::fs::create_dir_all(recycled.parent().unwrap()).unwrap();
        std::fs::create_dir_all(original.parent().unwrap()).unwrap();
        std::fs::create_dir_all(seeding.parent().unwrap()).unwrap();
        std::fs::write(&recycled, b"the recycled bytes").unwrap();
        std::fs::write(&seeding, b"the seeding payload").unwrap();
        if std::fs::hard_link(&seeding, &original).is_err() {
            eprintln!("skipping: hard links are unavailable here");
            return;
        }

        proven_replacement(
            &recycled,
            &original,
            CopyProgress::none(),
            VerificationDepth::Full,
        )
        .await
        .expect("the overwrite restore lands");

        assert_eq!(std::fs::read(&original).unwrap(), b"the recycled bytes");
        assert_eq!(
            std::fs::read(&seeding).unwrap(),
            b"the seeding payload",
            "the occupant's other name keeps its bytes"
        );
        assert_eq!(std::fs::read(&recycled).unwrap(), b"the recycled bytes");
        assert!(!crate::disk::resumable_staging_path(&original).exists());
    }

    /// A staging name some other file already holds refuses the replacement
    /// and is left exactly as it was, and so is the occupant.
    #[tokio::test]
    async fn an_overwrite_restore_refuses_a_held_staging_name_and_keeps_it() {
        let tmp = TempDir::new().unwrap();
        let recycled = tmp.path().join("bin").join("sample-feature.recycled");
        let original = tmp.path().join("library").join("sample-feature.mkv");
        let staged = crate::disk::resumable_staging_path(&original);
        std::fs::create_dir_all(recycled.parent().unwrap()).unwrap();
        std::fs::create_dir_all(original.parent().unwrap()).unwrap();
        std::fs::write(&recycled, b"the recycled bytes").unwrap();
        std::fs::write(&original, b"the occupant").unwrap();
        std::fs::write(&staged, b"someone else's partial").unwrap();

        proven_replacement(
            &recycled,
            &original,
            CopyProgress::none(),
            VerificationDepth::Quick,
        )
        .await
        .expect_err("a held staging name refuses the restore");

        assert_eq!(std::fs::read(&original).unwrap(), b"the occupant");
        assert_eq!(std::fs::read(&staged).unwrap(), b"someone else's partial");
        assert_eq!(std::fs::read(&recycled).unwrap(), b"the recycled bytes");
    }

    #[tokio::test]
    async fn a_reserved_name_is_released_when_its_restore_fails_before_it_writes() {
        let tmp = TempDir::new().unwrap();
        let recycled = tmp.path().join("bin").join("sample-feature.recycled");
        let reserved = tmp
            .path()
            .join("library")
            .join("sample-feature-restored.mkv");
        let bystander = tmp.path().join("library").join("other-feature.mkv");
        std::fs::create_dir_all(reserved.parent().unwrap()).unwrap();
        std::fs::write(&reserved, b"").unwrap();
        std::fs::write(&bystander, b"unrelated").unwrap();

        proven_copy(
            &recycled,
            &reserved,
            CopyTarget::OwnClaim,
            CopyProgress::none(),
            VerificationDepth::Quick,
        )
        .await
        .expect_err("a missing recycled file fails the restore");

        assert!(
            !reserved.exists(),
            "the name this restore reserved is released"
        );
        assert_eq!(std::fs::read(&bystander).unwrap(), b"unrelated");
    }

    #[tokio::test]
    async fn test_restore_sampled_proof_mismatch_removes_partial_and_keeps_recycled_source() {
        let tmp = TempDir::new().unwrap();
        let recycled_path = tmp.path().join("movie.recycled");
        let destination = tmp.path().join("movie-restored.mkv");
        tokio::fs::write(&recycled_path, b"recycled source bytes")
            .await
            .unwrap();
        tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)
            .await
            .unwrap();

        let error = copy_recycled_to_claimed_destination(
            &recycled_path,
            &destination,
            damage_when_verification_starts(&destination, b"mismatched restored bytes"),
            VerificationDepth::Full,
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

        let error =
            restore_recycled_file_exact(&recycled_path, &destination, VerificationDepth::Full)
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
        let result = recycle_file(&config, &source, test_manifest(), VerificationDepth::Full)
            .await
            .unwrap()
            .unwrap();

        // A new live file now occupies the original path.
        let live_content = b"the current live file";
        tokio::fs::write(&source, live_content).await.unwrap();

        let restored_to = restore_from_recycle(
            &result.recycled_path,
            &source,
            false,
            VerificationDepth::Full,
        )
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
        let result = recycle_file(&config, &source, test_manifest(), VerificationDepth::Full)
            .await
            .unwrap()
            .unwrap();

        tokio::fs::write(&source, b"current live file")
            .await
            .unwrap();
        tokio::fs::write(&first_sibling, b"previous restored file")
            .await
            .unwrap();

        let restored_to = restore_from_recycle(
            &result.recycled_path,
            &source,
            false,
            VerificationDepth::Full,
        )
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
    async fn restore_over_by_copy_replaces_the_occupant_and_removes_the_recycled_copy() {
        let tmp = TempDir::new().unwrap();
        let recycled = tmp.path().join("bin").join("sample-feature.mkv");
        let original = tmp.path().join("library").join("sample-feature.mkv");
        let bystander = tmp.path().join("library").join("other-feature.mkv");
        tokio::fs::create_dir_all(recycled.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::create_dir_all(original.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&recycled, b"recycled bytes")
            .await
            .unwrap();
        tokio::fs::write(&original, b"the occupant, which is longer")
            .await
            .unwrap();
        tokio::fs::write(&bystander, b"unrelated").await.unwrap();

        restore_over_by_verified_copy(
            &recycled,
            &original,
            CopyProgress::none(),
            VerificationDepth::Full,
        )
        .await
        .unwrap();

        assert_eq!(std::fs::read(&original).unwrap(), b"recycled bytes");
        assert!(!recycled.exists(), "the proven restore replaces the copy");
        assert_eq!(std::fs::read(&bystander).unwrap(), b"unrelated");
    }

    #[tokio::test]
    async fn restore_over_by_copy_fills_an_absent_destination() {
        let tmp = TempDir::new().unwrap();
        let recycled = tmp.path().join("sample-feature.recycled");
        let original = tmp.path().join("sample-feature.mkv");
        tokio::fs::write(&recycled, b"recycled bytes")
            .await
            .unwrap();

        restore_over_by_verified_copy(
            &recycled,
            &original,
            CopyProgress::none(),
            VerificationDepth::Full,
        )
        .await
        .unwrap();

        assert_eq!(std::fs::read(&original).unwrap(), b"recycled bytes");
        assert!(!recycled.exists());
    }

    #[tokio::test]
    async fn restore_over_by_copy_keeps_the_recycled_copy_when_the_restore_does_not_match() {
        let tmp = TempDir::new().unwrap();
        let recycled = tmp.path().join("sample-feature.recycled");
        let original = tmp.path().join("sample-feature.mkv");
        tokio::fs::write(&recycled, b"recycled bytes")
            .await
            .unwrap();

        let error = restore_over_by_verified_copy(
            &recycled,
            &original,
            damage_when_verification_starts(&original, b"damaged"),
            VerificationDepth::Full,
        )
        .await
        .expect_err("an unproven restore must fail");

        assert!(
            error.to_string().contains("copy verification failed"),
            "unexpected error: {error}"
        );
        assert_eq!(std::fs::read(&recycled).unwrap(), b"recycled bytes");
        assert!(!original.exists(), "the unproven restore is removed");
    }

    #[tokio::test]
    async fn an_unproven_overwrite_restore_leaves_the_occupant_and_keeps_the_recycled_copy() {
        let tmp = TempDir::new().unwrap();
        let recycled = tmp.path().join("sample-feature.recycled");
        let original = tmp.path().join("sample-feature.mkv");
        let staged = crate::disk::resumable_staging_path(&original);
        tokio::fs::write(&recycled, b"recycled bytes")
            .await
            .unwrap();
        tokio::fs::write(&original, b"the occupant").await.unwrap();

        let error = restore_over_by_verified_copy(
            &recycled,
            &original,
            damage_when_verification_starts(&staged, b"damaged"),
            VerificationDepth::Full,
        )
        .await
        .expect_err("an unproven restore must fail");

        assert!(
            error.to_string().contains("copy verification failed"),
            "unexpected error: {error}"
        );
        assert_eq!(std::fs::read(&original).unwrap(), b"the occupant");
        assert_eq!(std::fs::read(&recycled).unwrap(), b"recycled bytes");
        assert!(!staged.exists(), "the unproven staged copy is removed");
    }

    /// A restore onto a name the caller holds is not refused, and so cleans
    /// nothing up, when the root's reads do not answer: the copy runs on its
    /// own thread and the name holds the proven restore.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_restore_onto_a_held_name_is_not_refused_when_its_root_does_not_answer() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("library");
        let recycled = tmp.path().join("recycle").join("sample-feature.recycled");
        let original = root.join("sample-feature.mkv");
        std::fs::create_dir_all(recycled.parent().unwrap()).unwrap();
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&recycled, b"recycled bytes").unwrap();
        // The empty file this restore made to reserve the name.
        std::fs::write(&original, b"").unwrap();
        let disk = Disk::process();
        // Declared first, so it drops last: the process's own roots come back
        // after the root is released, however the test ends.
        let _roots = crate::disk::test_support::install_volumes(
            &disk,
            [(root.clone(), crate::disk::VolumeClass::LibraryRoot)],
        );
        let silent = crate::disk::test_support::silence_root(&disk, &original)
            .expect("the destination is under a configured root");
        assert_eq!(
            disk.uncached(&original).await.exists(),
            None,
            "the root's bounded reads are refused"
        );

        let restored = tokio::time::timeout(
            std::time::Duration::from_secs(120),
            proven_copy(
                &recycled,
                &original,
                CopyTarget::OwnClaim,
                CopyProgress::none(),
                VerificationDepth::Quick,
            ),
        )
        .await
        .expect("the restore ends");
        drop(silent);

        restored.expect("the restore is not refused");
        assert_eq!(std::fs::read(&original).unwrap(), b"recycled bytes");
        assert_eq!(std::fs::read(&recycled).unwrap(), b"recycled bytes");
    }

    #[tokio::test]
    async fn test_restore_overwrite_replaces_existing() {
        let tmp = TempDir::new().unwrap();
        let recycle_dir = tmp.path().join("recycle");
        let source = tmp.path().join("movie.mkv");
        let recycled_content = b"the recycled file to force back";
        tokio::fs::write(&source, recycled_content).await.unwrap();

        let config = test_config(&recycle_dir);
        let result = recycle_file(&config, &source, test_manifest(), VerificationDepth::Full)
            .await
            .unwrap()
            .unwrap();

        tokio::fs::write(&source, b"will be overwritten")
            .await
            .unwrap();

        let restored_to = restore_from_recycle(
            &result.recycled_path,
            &source,
            true,
            VerificationDepth::Full,
        )
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

        let error =
            restore_from_recycle(&recycled_link, &destination, false, VerificationDepth::Full)
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

        let error =
            restore_from_recycle(&recycled_link, &destination, true, VerificationDepth::Full)
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

    const RELOCATION_RECYCLED_AT: &str = "2031-01-02T03:04:05+00:00";

    fn copy_only_ops() -> RelocationOps {
        RelocationOps {
            allow_rename: false,
            ..RelocationOps::default()
        }
    }

    /// Copy-only relocation whose read-back of the payload is decided by
    /// `payload`; every other file is read back normally.
    fn ops_with_payload_read_back(
        payload: impl Fn(&Path) -> ReadBackHandle + Send + Sync + 'static,
    ) -> RelocationOps {
        RelocationOps {
            allow_rename: false,
            depth: VerificationDepth::Full,
            copier: Disk::process().with_read_back_opener(std::sync::Arc::new(
                move |path: &Path| {
                    if path.file_name().is_some_and(|name| name == "media.mkv") {
                        payload(path)
                    } else {
                        open_cache_bypassed(path)
                    }
                },
            )),
        }
    }

    fn single_target_plan(from: &Path, to: &Path) -> RecycleRelocationPlan {
        RecycleRelocationPlan {
            from: from.to_path_buf(),
            targets: vec![RecycleRelocationTarget {
                base_path: to.to_path_buf(),
                media_roots: Vec::new(),
            }],
        }
    }

    async fn seed_relocation_bin(bin: &Path, entry_id: &str) -> PathBuf {
        tokio::fs::create_dir_all(bin).await.unwrap();
        write_test_sentinel(bin).await;
        let manifest = committed_manifest(
            entry_id,
            RELOCATION_RECYCLED_AT.to_string(),
            "/data/movies/Invented Film (2031)/Invented Film.mkv",
            Some("title-relocate"),
            "file_deleted",
        );
        write_test_entry(bin, entry_id, &manifest).await
    }

    fn tree_snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut snapshot = Vec::new();
        let mut pending = vec![dir.to_path_buf()];
        while let Some(current) = pending.pop() {
            for entry in std::fs::read_dir(&current).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                let relative = path.strip_prefix(dir).unwrap().to_path_buf();
                if entry.file_type().unwrap().is_dir() {
                    snapshot.push((relative, Vec::new()));
                    pending.push(path);
                } else {
                    snapshot.push((relative, std::fs::read(&path).unwrap()));
                }
            }
        }
        snapshot.sort();
        snapshot
    }

    async fn assert_moved_entry(new_bin: &Path, entry_id: &str) {
        let entries = list_committed_entries(&test_config(new_bin)).await.unwrap();
        assert_eq!(
            entries.len(),
            1,
            "the moved entry is listed at the new location"
        );
        assert_eq!(entries[0].manifest.entry_id.as_deref(), Some(entry_id));
        assert_eq!(
            entries[0].manifest.recycled_at, RELOCATION_RECYCLED_AT,
            "the entry keeps its original recycled-at time"
        );
        assert_eq!(
            std::fs::read(new_bin.join(entry_id).join("media.mkv")).unwrap(),
            b"media"
        );
    }

    #[tokio::test]
    async fn relocation_renames_entry_into_new_location() {
        let tmp = TempDir::new().unwrap();
        let old_bin = tmp.path().join("old-bin");
        let new_bin = tmp.path().join("new-bin");
        let entry_id = "20310102_030405000_mov111";
        let old_entry = seed_relocation_bin(&old_bin, entry_id).await;

        let report = relocate_plan(
            &single_target_plan(&old_bin, &new_bin),
            RelocationOps::default(),
        )
        .await;

        assert_eq!(report.moved_count, 1);
        assert!(report.failures.is_empty(), "{:?}", report.failures);
        assert!(!old_entry.exists(), "the previous copy is gone");
        assert!(old_bin.is_dir(), "the previous folder itself is kept");
        assert_moved_entry(&new_bin, entry_id).await;
    }

    #[tokio::test]
    async fn relocation_copies_verifies_then_removes_previous_copy() {
        let tmp = TempDir::new().unwrap();
        let old_bin = tmp.path().join("old-bin");
        let new_bin = tmp.path().join("new-bin");
        let entry_id = "20310102_030405000_cpy111";
        let old_entry = seed_relocation_bin(&old_bin, entry_id).await;
        tokio::fs::create_dir(old_entry.join("extras"))
            .await
            .unwrap();
        tokio::fs::write(old_entry.join("extras").join("sidecar.srt"), b"subtitle")
            .await
            .unwrap();

        let report = relocate_plan(&single_target_plan(&old_bin, &new_bin), copy_only_ops()).await;

        assert_eq!(report.moved_count, 1);
        assert!(report.failures.is_empty(), "{:?}", report.failures);
        assert!(
            !old_entry.exists(),
            "the verified copy replaces the previous one"
        );
        assert_moved_entry(&new_bin, entry_id).await;
        assert_eq!(
            std::fs::read(new_bin.join(entry_id).join("extras").join("sidecar.srt")).unwrap(),
            b"subtitle"
        );
    }

    // A sidecar the copy cannot read fails the entry part-way through.
    #[cfg(unix)]
    #[tokio::test]
    async fn relocation_copy_failure_keeps_entry_and_removes_partial_copy() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new().unwrap();
        let old_bin = tmp.path().join("old-bin");
        let new_bin = tmp.path().join("new-bin");
        let entry_id = "20310102_030405000_cfl111";
        let old_entry = seed_relocation_bin(&old_bin, entry_id).await;
        let sidecar = old_entry.join("zz-unreadable.nfo");
        tokio::fs::write(&sidecar, b"sidecar").await.unwrap();
        let before = tree_snapshot(&old_bin);
        std::fs::set_permissions(&sidecar, std::fs::Permissions::from_mode(0o000)).unwrap();

        let report = relocate_plan(&single_target_plan(&old_bin, &new_bin), copy_only_ops()).await;

        std::fs::set_permissions(&sidecar, std::fs::Permissions::from_mode(0o644)).unwrap();

        assert_eq!(report.moved_count, 0);
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].entry_id, entry_id);
        assert_eq!(
            report.failures[0].from_path,
            old_entry.to_string_lossy().into_owned()
        );
        assert!(
            report.failures[0].reason.contains("could not copy"),
            "{}",
            report.failures[0].reason
        );
        assert_eq!(tree_snapshot(&old_bin), before, "the entry is left intact");
        assert!(
            !new_bin.join(entry_id).exists(),
            "the partial copy is cleaned up"
        );
    }

    #[tokio::test]
    async fn relocation_verification_failure_keeps_entry_and_removes_copy() {
        let tmp = TempDir::new().unwrap();
        let old_bin = tmp.path().join("old-bin");
        let new_bin = tmp.path().join("new-bin");
        let entry_id = "20310102_030405000_vfy111";
        seed_relocation_bin(&old_bin, entry_id).await;
        let before = tree_snapshot(&old_bin);

        // The read-back sees different bytes than the copy wrote.
        let ops = ops_with_payload_read_back(|path| {
            open_cache_bypassed(&manifest_path(path.parent().unwrap()))
        });
        let report = relocate_plan(&single_target_plan(&old_bin, &new_bin), ops).await;

        assert_eq!(report.moved_count, 0);
        assert_eq!(report.failures.len(), 1);
        assert!(
            report.failures[0].reason.contains("verification failed"),
            "{}",
            report.failures[0].reason
        );
        assert_eq!(tree_snapshot(&old_bin), before, "the entry is left intact");
        assert!(
            !new_bin.join(entry_id).exists(),
            "the unverified copy is removed"
        );
    }

    #[tokio::test]
    async fn relocation_refuses_a_copy_it_could_only_sample() {
        let tmp = TempDir::new().unwrap();
        let old_bin = tmp.path().join("old-bin");
        let new_bin = tmp.path().join("new-bin");
        let entry_id = "20310102_030405000_smp111";
        seed_relocation_bin(&old_bin, entry_id).await;
        let before = tree_snapshot(&old_bin);

        let ops = ops_with_payload_read_back(|_| {
            ReadBackHandle::Unsupported("the read-back cannot run here".to_string())
        });
        let report = relocate_plan(&single_target_plan(&old_bin, &new_bin), ops).await;

        assert_eq!(report.moved_count, 0);
        assert_eq!(report.failures.len(), 1);
        assert!(
            report.failures[0].reason.contains("verification failed"),
            "{}",
            report.failures[0].reason
        );
        assert_eq!(tree_snapshot(&old_bin), before, "the entry is left intact");
        assert!(
            !new_bin.join(entry_id).exists(),
            "the unproven copy is removed"
        );
    }

    #[tokio::test]
    async fn relocation_keeps_an_entry_that_changed_while_it_was_copied() {
        let tmp = TempDir::new().unwrap();
        let old_bin = tmp.path().join("old-bin");
        let new_bin = tmp.path().join("new-bin");
        let entry_id = "20310102_030405000_chg111";
        let old_entry = seed_relocation_bin(&old_bin, entry_id).await;
        let payload = old_entry.join("media.mkv");

        // Same size, different bytes, written after the copy read the file.
        let changed_payload = payload.clone();
        let ops = ops_with_payload_read_back(move |path| {
            std::fs::write(&changed_payload, b"MEDIA").unwrap();
            open_cache_bypassed(path)
        });
        let report = relocate_plan(&single_target_plan(&old_bin, &new_bin), ops).await;

        assert_eq!(report.moved_count, 0);
        assert_eq!(report.failures.len(), 1);
        assert!(
            report.failures[0]
                .reason
                .contains("changed while it was being copied"),
            "{}",
            report.failures[0].reason
        );
        assert_eq!(std::fs::read(&payload).unwrap(), b"MEDIA");
        assert!(manifest_path(&old_entry).is_file(), "the entry is kept");
        assert!(
            !new_bin.join(entry_id).exists(),
            "the copy of the earlier bytes is removed"
        );
    }

    #[tokio::test]
    async fn relocation_leaves_everything_without_a_valid_manifest_untouched() {
        for ops in [RelocationOps::default(), copy_only_ops()] {
            let tmp = TempDir::new().unwrap();
            let old_bin = tmp.path().join("old-bin");
            let new_bin = tmp.path().join("new-bin");
            let entry_id = "20310102_030405000_val111";
            seed_relocation_bin(&old_bin, entry_id).await;

            tokio::fs::write(old_bin.join("notes.txt"), b"operator notes")
                .await
                .unwrap();
            let no_manifest = old_bin.join("20310102_030405000_nom111");
            tokio::fs::create_dir(&no_manifest).await.unwrap();
            tokio::fs::write(no_manifest.join("media.mkv"), b"unmanaged")
                .await
                .unwrap();
            let unparsable = old_bin.join("20310102_030405000_bad111");
            tokio::fs::create_dir(&unparsable).await.unwrap();
            tokio::fs::write(unparsable.join("manifest.json"), b"{ not json")
                .await
                .unwrap();
            let mismatched_id = committed_manifest(
                "20310102_030405000_zzz999",
                RELOCATION_RECYCLED_AT.to_string(),
                "/data/movies/Other/Other.mkv",
                None,
                "file_deleted",
            );
            write_test_entry(&old_bin, "20310102_030405000_mis111", &mismatched_id).await;
            let plain_dir = old_bin.join("keep-me");
            tokio::fs::create_dir(&plain_dir).await.unwrap();
            tokio::fs::write(plain_dir.join("file.bin"), b"keep")
                .await
                .unwrap();

            let mut expected = tree_snapshot(&old_bin);
            expected.retain(|(path, _)| !path.starts_with(entry_id));

            let report = relocate_plan(&single_target_plan(&old_bin, &new_bin), ops).await;

            assert_eq!(report.moved_count, 1, "only the valid entry moves");
            assert!(report.failures.is_empty(), "{:?}", report.failures);
            assert_eq!(
                tree_snapshot(&old_bin),
                expected,
                "everything else in the previous folder is left exactly as it was"
            );
            let moved = std::fs::read_dir(&new_bin)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .filter(|name| name != RECYCLE_ROOT_SENTINEL)
                .collect::<Vec<_>>();
            assert_eq!(moved, vec![entry_id.to_string()]);
        }
    }

    #[tokio::test]
    async fn relocation_without_root_sentinel_moves_nothing() {
        let tmp = TempDir::new().unwrap();
        let old_bin = tmp.path().join("old-bin");
        let new_bin = tmp.path().join("new-bin");
        let entry_id = "20310102_030405000_sen111";
        seed_relocation_bin(&old_bin, entry_id).await;
        tokio::fs::remove_file(old_bin.join(RECYCLE_ROOT_SENTINEL))
            .await
            .unwrap();
        let before = tree_snapshot(&old_bin);

        let report = relocate_plan(
            &single_target_plan(&old_bin, &new_bin),
            RelocationOps::default(),
        )
        .await;

        assert!(report.is_empty());
        assert_eq!(tree_snapshot(&old_bin), before);
        assert!(!new_bin.exists());
    }

    #[tokio::test]
    async fn relocation_never_overwrites_an_entry_at_the_destination() {
        for ops in [RelocationOps::default(), copy_only_ops()] {
            let tmp = TempDir::new().unwrap();
            let old_bin = tmp.path().join("old-bin");
            let new_bin = tmp.path().join("new-bin");
            let entry_id = "20310102_030405000_col111";
            seed_relocation_bin(&old_bin, entry_id).await;
            let occupant = new_bin.join(entry_id);
            tokio::fs::create_dir_all(&occupant).await.unwrap();
            tokio::fs::write(occupant.join("occupant.bin"), b"already here")
                .await
                .unwrap();
            let old_before = tree_snapshot(&old_bin);
            let new_before = tree_snapshot(&occupant);

            let report = relocate_plan(&single_target_plan(&old_bin, &new_bin), ops).await;

            assert_eq!(report.moved_count, 0);
            assert_eq!(report.failures.len(), 1);
            assert!(
                report.failures[0].reason.contains("already exists"),
                "{}",
                report.failures[0].reason
            );
            assert_eq!(tree_snapshot(&old_bin), old_before);
            assert_eq!(tree_snapshot(&occupant), new_before);
        }
    }

    #[tokio::test]
    async fn relocation_reports_and_keeps_uncommitted_entries() {
        let tmp = TempDir::new().unwrap();
        let old_bin = tmp.path().join("old-bin");
        let new_bin = tmp.path().join("new-bin");
        tokio::fs::create_dir_all(&old_bin).await.unwrap();
        write_test_sentinel(&old_bin).await;
        let entry_id = "20310102_030405000_pen111";
        let pending = write_test_entry(
            &old_bin,
            entry_id,
            &pending_manifest(entry_id, RELOCATION_RECYCLED_AT.to_string()),
        )
        .await;

        let report = relocate_plan(
            &single_target_plan(&old_bin, &new_bin),
            RelocationOps::default(),
        )
        .await;

        assert_eq!(report.moved_count, 0);
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].entry_id, entry_id);
        assert!(pending.join("media.mkv").exists());
        assert!(!new_bin.join(entry_id).exists());
    }

    #[tokio::test]
    async fn relocation_routes_entries_to_the_bin_of_their_library_root() {
        let tmp = TempDir::new().unwrap();
        let old_bin = tmp.path().join("shared-bin");
        let root_a = tmp.path().join("library-a");
        let root_b = tmp.path().join("library-b");
        tokio::fs::create_dir_all(&old_bin).await.unwrap();
        write_test_sentinel(&old_bin).await;
        let entry_a = "20310102_030405000_aaa111";
        let entry_b = "20310102_030405000_bbb111";
        let entry_lost = "20310102_030405000_lst111";
        for (entry_id, original_path) in [
            (entry_a, root_a.join("Film A/Film A.mkv")),
            (entry_b, root_b.join("Film B/Film B.mkv")),
            (entry_lost, tmp.path().join("elsewhere/Film C.mkv")),
        ] {
            let manifest = committed_manifest(
                entry_id,
                RELOCATION_RECYCLED_AT.to_string(),
                &original_path.to_string_lossy(),
                None,
                "file_deleted",
            );
            write_test_entry(&old_bin, entry_id, &manifest).await;
        }
        let plan = RecycleRelocationPlan {
            from: old_bin.clone(),
            targets: vec![
                RecycleRelocationTarget {
                    base_path: root_a.join(RECYCLE_DIR_NAME),
                    media_roots: vec![root_a.clone()],
                },
                RecycleRelocationTarget {
                    base_path: root_b.join(RECYCLE_DIR_NAME),
                    media_roots: vec![root_b.clone()],
                },
            ],
        };

        let report = relocate_plan(&plan, RelocationOps::default()).await;

        assert_eq!(report.moved_count, 2);
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].entry_id, entry_lost);
        assert!(root_a.join(RECYCLE_DIR_NAME).join(entry_a).is_dir());
        assert!(root_b.join(RECYCLE_DIR_NAME).join(entry_b).is_dir());
        assert!(
            old_bin.join(entry_lost).is_dir(),
            "an unroutable entry stays put"
        );
    }

    fn bin_tree(dir: &Path) -> std::collections::BTreeMap<PathBuf, Option<Vec<u8>>> {
        let mut snapshot = std::collections::BTreeMap::new();
        let mut pending = vec![dir.to_path_buf()];
        while let Some(current) = pending.pop() {
            let Ok(entries) = std::fs::read_dir(&current) else {
                continue;
            };
            for entry in entries {
                let path = entry.unwrap().path();
                let relative = path.strip_prefix(dir).unwrap().to_path_buf();
                if path.is_dir() {
                    snapshot.insert(relative, None);
                    pending.push(path);
                } else {
                    snapshot.insert(relative, Some(std::fs::read(&path).unwrap()));
                }
            }
        }
        snapshot
    }

    /// One deletion job over several titles: each title's listing through the
    /// job index matches a fresh full listing taken at the same moment, and
    /// leaves the bin in the same state, while entries appear, commit, vanish,
    /// gain a manifest, get repaired, and the bin briefly stops being ready.
    #[tokio::test]
    async fn title_listing_through_job_index_matches_a_fresh_full_listing() {
        const RECYCLED_AT: &str = "2026-02-05T12:00:00+00:00";
        let entry = |suffix: &str| format!("20260205_120000000_{suffix}");
        let committed = |suffix: &str, title: &str| {
            committed_manifest(
                &entry(suffix),
                RECYCLED_AT.to_string(),
                "/data/series/Synthetic Show/media.mkv",
                Some(title),
                "file_deleted",
            )
        };
        let tmp = TempDir::new().unwrap();
        let bins = [tmp.path().join("full"), tmp.path().join("indexed")];

        for bin in &bins {
            tokio::fs::create_dir_all(bin).await.unwrap();
            write_test_sentinel(bin).await;
            write_test_entry(bin, &entry("aaa001"), &committed("aaa001", "title-one")).await;
            write_test_entry(bin, &entry("aaa002"), &committed("aaa002", "title-two")).await;
            let mut pending = committed("aaa003", "title-two");
            pending.status = Some(RECYCLE_STATUS_PENDING.to_string());
            write_test_entry(bin, &entry("aaa003"), &pending).await;
            write_test_entry(bin, &entry("aaa004"), &committed("aaa004", "title-three")).await;
            write_test_entry(bin, &entry("aaa005"), &committed("aaa005", "title-other")).await;
            // A directory whose manifest has not been written yet.
            tokio::fs::create_dir_all(bin.join(entry("aaa006")))
                .await
                .unwrap();
            // A torn manifest.
            let torn = write_test_entry(bin, &entry("aaa007"), &committed("aaa007", "x")).await;
            tokio::fs::write(torn.join("manifest.json"), b"{\"schema\":")
                .await
                .unwrap();
            // A committed entry that is not trusted is quarantined on sight.
            let mut untrusted = committed("aaa008", "title-one");
            untrusted.schema = None;
            write_test_entry(bin, &entry("aaa008"), &untrusted).await;
            let mut no_title = committed("aaa009", "x");
            no_title.title_id = None;
            write_test_entry(bin, &entry("aaa009"), &no_title).await;
        }

        let mut index = TitleRecycleEntryIndex::default();
        // (title, purge what was listed)
        let steps = [
            ("title-one", true),
            ("title-two", false),
            ("title-three", true),
            ("title-two", true),
            ("title-one", true),
            ("title-three", true),
        ];
        for (step, (title, purge)) in steps.into_iter().enumerate() {
            for bin in &bins {
                match step {
                    1 => {
                        // Created after the first title, and committed in place.
                        write_test_entry(bin, &entry("bbb001"), &committed("bbb001", "title-two"))
                            .await;
                        let mut flipped = read_test_manifest(&bin.join(entry("aaa003"))).await;
                        flipped.status = Some(RECYCLE_STATUS_COMMITTED.to_string());
                        write_manifest(&bin.join(entry("aaa003")), &flipped)
                            .await
                            .unwrap();
                    }
                    2 => {
                        write_manifest(
                            &bin.join(entry("aaa006")),
                            &committed("aaa006", "title-three"),
                        )
                        .await
                        .unwrap();
                        tokio::fs::write(bin.join(entry("aaa006")).join("media.mkv"), b"media")
                            .await
                            .unwrap();
                        write_manifest(
                            &bin.join(entry("aaa007")),
                            &committed("aaa007", "title-three"),
                        )
                        .await
                        .unwrap();
                        write_test_entry(bin, &entry("bbb002"), &committed("bbb002", "title-one"))
                            .await;
                    }
                    3 => {
                        tokio::fs::remove_file(bin.join(RECYCLE_ROOT_SENTINEL))
                            .await
                            .unwrap();
                    }
                    4 => {
                        write_test_sentinel(bin).await;
                        crate::fs_safety::remove_dir_all_safely(&bin.join(entry("aaa005")))
                            .await
                            .unwrap();
                    }
                    _ => {}
                }
            }

            let full = list_committed_entries(&test_config(&bins[0]))
                .await
                .unwrap()
                .into_iter()
                .filter(|listed| listed.manifest.title_id.as_deref() == Some(title))
                .collect::<Vec<_>>();
            let indexed =
                list_committed_entries_for_title(&test_config(&bins[1]), title, &mut index)
                    .await
                    .unwrap();
            let names = |entries: &[CommittedRecycleEntry]| {
                entries
                    .iter()
                    .map(|listed| {
                        (
                            listed.entry_dir.file_name().unwrap().to_owned(),
                            listed.manifest.clone(),
                        )
                    })
                    .map(|(name, manifest)| (name, serde_json::to_string(&manifest).unwrap()))
                    .collect::<Vec<_>>()
            };
            assert_eq!(names(&full), names(&indexed), "step {step} ({title})");
            if step == 1 {
                assert_eq!(
                    indexed.len(),
                    3,
                    "the committed, the newly committed and the new entry are listed"
                );
            }
            if purge {
                for (bin, listed) in [(&bins[0], &full), (&bins[1], &indexed)] {
                    for committed in listed.iter() {
                        assert!(
                            purge_committed_entry(&test_config(bin), committed)
                                .await
                                .unwrap()
                        );
                    }
                }
            }
            assert_eq!(
                bin_tree(&bins[0]),
                bin_tree(&bins[1]),
                "bin state after step {step} ({title})"
            );
        }
        let remaining = bin_tree(&bins[1]);
        assert!(remaining.contains_key(&PathBuf::from(entry("aaa008"))));
        assert!(remaining.contains_key(&PathBuf::from(entry("aaa009"))));
        assert!(!remaining.contains_key(&PathBuf::from(entry("aaa006"))));
        // The second title was purged only while its bin was not ready.
        assert!(remaining.contains_key(&PathBuf::from(entry("bbb001"))));
        assert!(!remaining.contains_key(&PathBuf::from(entry("bbb002"))));
    }

    /// The manifest read without an existence check first gives the answer
    /// the check-then-read did for every shape an entry directory can take.
    #[tokio::test]
    async fn manifest_read_without_a_prior_exists_check_answers_the_same() {
        async fn check_then_read(entry_dir: &Path) -> AppResult<Option<RecycleManifest>> {
            let path = manifest_path(entry_dir);
            if !path.exists() {
                return Ok(None);
            }
            let bytes = tokio::fs::read(&path)
                .await
                .map_err(|e| AppError::Repository(e.to_string()))?;
            serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|e| AppError::Repository(e.to_string()))
        }
        fn shape(outcome: &AppResult<Option<RecycleManifest>>) -> String {
            match outcome {
                Ok(Some(manifest)) => serde_json::to_string(manifest).unwrap(),
                Ok(None) => "none".to_string(),
                Err(_) => "error".to_string(),
            }
        }

        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
        let valid = base.join("valid");
        std::fs::create_dir(&valid).unwrap();
        std::fs::write(
            manifest_path(&valid),
            r#"{"recycled_at":"2026-01-01T00:00:00Z","original_path":"/synthetic/a.mkv","size_bytes":3,"reason":"synthetic"}"#,
        )
        .unwrap();
        let missing = base.join("missing");
        std::fs::create_dir(&missing).unwrap();
        let garbled = base.join("garbled");
        std::fs::create_dir(&garbled).unwrap();
        std::fs::write(manifest_path(&garbled), b"{not json").unwrap();
        let manifest_is_dir = base.join("manifest-is-dir");
        std::fs::create_dir_all(manifest_path(&manifest_is_dir)).unwrap();
        let entry_is_file = base.join("entry-is-file");
        std::fs::write(&entry_is_file, b"synthetic").unwrap();
        let absent_entry = base.join("absent-entry");
        #[cfg_attr(not(unix), allow(unused_mut))]
        let mut cases = vec![
            (valid, "valid"),
            (missing, "none"),
            (garbled, "error"),
            (manifest_is_dir, "error"),
            (entry_is_file, "none"),
            (absent_entry, "none"),
        ];
        #[cfg(unix)]
        let unsearchable = {
            use std::os::unix::fs::PermissionsExt;
            let dangling = base.join("dangling");
            std::fs::create_dir(&dangling).unwrap();
            std::os::unix::fs::symlink(base.join("nowhere.json"), manifest_path(&dangling))
                .unwrap();
            cases.push((dangling, "none"));
            let unsearchable = base.join("unsearchable");
            std::fs::create_dir(&unsearchable).unwrap();
            std::fs::write(manifest_path(&unsearchable), b"{}").unwrap();
            std::fs::set_permissions(&unsearchable, std::fs::Permissions::from_mode(0o000))
                .unwrap();
            // Unreadable to an ordinary user; a privileged runner reads it,
            // and both paths then agree on the parse error instead.
            cases.push((unsearchable.clone(), "either"));
            unsearchable
        };

        for (entry_dir, expected) in &cases {
            let before = check_then_read(entry_dir).await;
            let after = read_manifest(entry_dir).await;
            assert_eq!(shape(&after), shape(&before), "{}", entry_dir.display());
            match *expected {
                "valid" => assert!(matches!(after, Ok(Some(_)))),
                "none" => assert!(matches!(after, Ok(None))),
                "error" => assert!(after.is_err()),
                _ => {}
            }
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&unsearchable, std::fs::Permissions::from_mode(0o755))
                .unwrap();
        }
    }
}

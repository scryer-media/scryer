//! Quality-upgrade workflow for media files.
//!
//! When a new import scores higher than an existing file for the same title,
//! the replacement is imported and validated before the old file is recycled
//! or deleted.

use crate::domain_events::{
    DomainEventActor, created_media_update, deleted_media_update, modified_media_update,
    new_title_domain_event, title_context_snapshot,
};
use crate::recycle_bin::{self, RecycleBinConfig};
use crate::stored_paths::{path_to_stored_string, stored_path_to_path_buf};
use crate::types::TitleMediaFile;
use crate::{AppError, AppResult, AppUseCase, InsertMediaFileInput};
use chrono::{DateTime, Duration, Utc};
use scryer_domain::{
    DomainEventPayload, ImportMode, ImportSourceCleanupGuard, MediaFileDeletedEventData,
    MediaFileDeletedReason, MediaFileUpgradedEventData, Title, User,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Result of a successful upgrade operation.
#[derive(Debug)]
pub struct UpgradeOutcome {
    pub old_score: i32,
    pub new_score: i32,
    pub new_file_id: String,
    /// Size of the replacement file, so callers can report imported bytes
    /// without re-reading the media file row.
    pub new_size_bytes: i64,
    pub recycle_entry_committed: bool,
    pub source_cleanup: Option<Box<ImportSourceCleanupGuard>>,
    pub final_path_string: String,
    pub(crate) destination_permit: crate::import_workflow::ImportDestinationPermit,
}

pub enum UpgradeResult {
    Upgraded(UpgradeOutcome),
    Rejected(crate::post_download_gate::ImportedFileRejection),
}

pub(crate) struct UpgradeRecycleContext {
    pub(crate) media_root: String,
    pub(crate) recycle_config: RecycleBinConfig,
}

pub(crate) async fn resolve_old_file_recycle_context(
    app: &AppUseCase,
    title: &Title,
    existing_file: &TitleMediaFile,
) -> AppResult<UpgradeRecycleContext> {
    let old_path = stored_path_to_path_buf(&existing_file.file_path);
    let media_roots = app
        .all_library_root_folders_for_facet(&title.facet)
        .await?
        .into_iter()
        .filter(|root| root.library_id == title.library_id)
        .map(|root| root.path.trim().to_string())
        .filter(|path| !path.is_empty())
        .collect::<Vec<_>>();

    if media_roots.is_empty() {
        return Err(AppError::Validation(format!(
            "refusing to upgrade {} because library {} has no configured media roots for old-file cleanup",
            title.name, title.library_id
        )));
    }

    let source_roots = media_roots
        .iter()
        .map(|root| stored_path_to_path_buf(root))
        .collect::<Vec<_>>();
    let old_file_media_root =
        crate::fs_safety::most_specific_containing_root(&old_path, &source_roots).ok_or_else(
            || {
                AppError::Validation(format!(
                    "refusing to upgrade {} because old file {} is outside the current media roots for library {}; keep the old root configured until existing files are moved, replaced, or deleted",
                    title.name,
                    old_path.display(),
                    title.library_id
                ))
            },
        )?;
    crate::fs_safety::ensure_root_available(&old_file_media_root)?;

    let recycle_config = app
        .recycle_bin_configs_for_media_roots(media_roots)
        .await
        .into_iter()
        .find_map(|(media_root, config)| {
            if media_root.trim().is_empty()
                || configured_roots_match(&stored_path_to_path_buf(&media_root), &old_file_media_root)
            {
                Some(config)
            } else {
                None
            }
        })
        .ok_or_else(|| {
            AppError::Validation(format!(
                "refusing to upgrade {} because no recycle bin config could be resolved for old file root {}",
                title.name,
                old_file_media_root.display()
            ))
        })?;

    Ok(UpgradeRecycleContext {
        media_root: path_to_stored_string(&old_file_media_root),
        recycle_config,
    })
}

fn configured_roots_match(left: &Path, right: &Path) -> bool {
    recycle_bin::path_is_under_configured_root(left, right)
        && recycle_bin::path_is_under_configured_root(right, left)
}

/// Execute a guarded file upgrade: import and validate replacement, then retire old.
///
/// The old file is not recycled or deleted until the replacement file is on disk,
/// represented in storage, linked, and validated.
#[expect(
    clippy::too_many_arguments,
    reason = "upgrade execution coordinates file movement, scoring, and persistence state in one transaction"
)]
pub(crate) async fn execute_upgrade(
    app: &AppUseCase,
    actor: &User,
    import_id: &str,
    title: &Title,
    existing_file: &TitleMediaFile,
    source_path: &std::path::Path,
    dest_path: &std::path::Path,
    prepared: &crate::post_download_gate::PreparedImportCandidate,
    stored_quality_label: Option<&str>,
    final_score: i32,
    old_score: i32,
    post_download_scoring_log: Option<String>,
    target_episode_ids: &[String],
    replacement_media_root: Option<&str>,
    old_file_media_root: Option<&str>,
    recycle_config: &RecycleBinConfig,
    import_mode: ImportMode,
    announced_size_bytes: Option<i64>,
    completed: Option<&scryer_domain::CompletedDownload>,
) -> AppResult<UpgradeResult> {
    let audit_actor = DomainEventActor::from(actor);

    let old_path = stored_path_to_path_buf(&existing_file.file_path);
    ensure_old_file_disposition_ready(recycle_config, &old_path)?;
    let dest_path_string = path_to_stored_string(dest_path);
    let source_path_string = path_to_stored_string(source_path);

    let scoring_log = upgrade_scoring_log(
        old_score,
        final_score,
        post_download_scoring_log,
        &prepared.rescore_changes,
    );

    let same_final_path = old_path == dest_path;
    let import_path = if same_final_path {
        sibling_guard_path(dest_path, "replacement")
    } else {
        dest_path.to_path_buf()
    };

    let replacement = prepare_replacement_before_old_removal(
        app,
        import_id,
        title,
        existing_file,
        source_path,
        &import_path,
        dest_path_string.clone(),
        same_final_path,
        prepared,
        stored_quality_label,
        final_score,
        target_episode_ids,
        replacement_media_root,
        &scoring_log,
        &source_path_string,
        import_mode,
        announced_size_bytes,
        completed,
    )
    .await?;

    let recycle_entry_committed = finalize_prepared_upgrade(
        app,
        title,
        existing_file,
        &replacement,
        recycle_config,
        &old_path,
        replacement_media_root,
        old_file_media_root,
    )
    .await?;

    append_upgrade_event(
        app,
        audit_actor.clone(),
        title,
        existing_file,
        UpgradeEventDetails {
            new_file_id: &replacement.new_file_id,
            new_size_bytes: replacement.new_size_bytes,
            dest_path_string: &replacement.final_path_string,
            old_score,
            final_score,
            episode_ids: target_episode_ids,
        },
    )
    .await?;

    if recycle_entry_committed {
        append_upgrade_recycle_event(
            app,
            audit_actor.clone(),
            title,
            existing_file,
            target_episode_ids,
        )
        .await;
    }

    Ok(UpgradeResult::Upgraded(UpgradeOutcome {
        old_score,
        new_score: final_score,
        new_file_id: replacement.new_file_id,
        new_size_bytes: replacement.new_size_bytes,
        recycle_entry_committed,
        source_cleanup: replacement.source_cleanup.map(Box::new),
        final_path_string: replacement.final_path_string,
        destination_permit: replacement.destination_permit,
    }))
}

pub async fn finalize_upgrade_source_cleanup(
    app: &AppUseCase,
    outcome: &UpgradeOutcome,
    completed: Option<&scryer_domain::CompletedDownload>,
) -> AppResult<()> {
    let Some(guard) = outcome.source_cleanup.as_deref().cloned() else {
        return Ok(());
    };
    let execution_context = crate::ImportFileExecutionContext::new(
        completed.map_or("", |item| item.client_id.as_str()),
        completed.map_or("", |item| item.client_type.as_str()),
    );
    app.services
        .workflow
        .file_importer
        .remove_import_source_after_verified_import_with_context(
            guard,
            &stored_path_to_path_buf(&outcome.final_path_string),
            &execution_context,
        )
        .await
}

struct PreparedUpgradeReplacement {
    new_file_id: String,
    reused_existing: bool,
    destination_created: bool,
    /// Size of the replacement file, captured from the import result so the
    /// upgrade event can report it without a second stat or media-file read.
    new_size_bytes: i64,
    import_path: PathBuf,
    final_path_string: String,
    same_final_path: bool,
    source_cleanup: Option<ImportSourceCleanupGuard>,
    destination_permit: crate::import_workflow::ImportDestinationPermit,
}

enum OldFileDisposition {
    Noop,
    PendingRecycle(recycle_bin::RecycleResult),
    Backup(PathBuf),
}

const SAME_PATH_UPGRADE_GUARD_SCHEMA: &str = "scryer.same-path-upgrade-guard.v1";
const UPGRADE_GUARD_PHASE_PLANNED: &str = "planned";
const UPGRADE_GUARD_PHASE_OLD_MOVED: &str = "old_moved";
const UPGRADE_GUARD_PHASE_REPLACEMENT_MOVED: &str = "replacement_moved";
const UPGRADE_GUARD_PHASE_DB_SWAPPED: &str = "db_swapped";
const UPGRADE_GUARD_PHASE_DISPOSED: &str = "disposed";
const SAME_PATH_UPGRADE_GUARD_DIR: &str = ".scryer-upgrade-guards";
const SAME_PATH_UPGRADE_GUARD_STALE_AFTER_SECONDS: i64 = 15 * 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SamePathUpgradeGuardManifest {
    schema: String,
    phase: String,
    title_id: String,
    old_file_id: String,
    old_size_bytes: u64,
    replacement_file_id: String,
    final_path: String,
    backup_path: String,
    staged_replacement_path: String,
    replacement_path: String,
    media_root: String,
    created_at: String,
    updated_at: String,
}

impl SamePathUpgradeGuardManifest {
    fn new(
        title: &Title,
        existing_file: &TitleMediaFile,
        replacement: &PreparedUpgradeReplacement,
        final_path: &Path,
        backup_path: &Path,
        media_root: &Path,
    ) -> Self {
        let now = Utc::now().to_rfc3339();
        Self {
            schema: SAME_PATH_UPGRADE_GUARD_SCHEMA.to_string(),
            phase: UPGRADE_GUARD_PHASE_PLANNED.to_string(),
            title_id: title.id.clone(),
            old_file_id: existing_file.id.clone(),
            old_size_bytes: existing_file.size_bytes as u64,
            replacement_file_id: replacement.new_file_id.clone(),
            final_path: path_to_stored_string(final_path),
            backup_path: path_to_stored_string(backup_path),
            staged_replacement_path: path_to_stored_string(&replacement.import_path),
            replacement_path: replacement.final_path_string.clone(),
            media_root: path_to_stored_string(media_root),
            created_at: now.clone(),
            updated_at: now,
        }
    }
}

fn same_path_upgrade_guard_dir(media_root: &Path) -> PathBuf {
    media_root.join(SAME_PATH_UPGRADE_GUARD_DIR)
}

fn same_path_upgrade_guard_path(media_root: &Path, backup_path: &Path) -> PathBuf {
    let file_name = backup_path
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new(".scryer-upgrade-old"));
    same_path_upgrade_guard_dir(media_root)
        .join(format!("{}.guard.json", file_name.to_string_lossy()))
}

async fn write_same_path_upgrade_guard(
    guard_path: &Path,
    manifest: &SamePathUpgradeGuardManifest,
) -> AppResult<()> {
    if let Some(parent) = guard_path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|error| {
            AppError::Repository(format!(
                "failed to create same-path upgrade guard directory {}: {}",
                parent.display(),
                error
            ))
        })?;
    }
    let bytes = serde_json::to_vec_pretty(manifest).map_err(|error| {
        AppError::Repository(format!(
            "failed to encode same-path upgrade guard {}: {}",
            guard_path.display(),
            error
        ))
    })?;
    tokio::fs::write(guard_path, bytes).await.map_err(|error| {
        AppError::Repository(format!(
            "failed to write same-path upgrade guard {}: {}",
            guard_path.display(),
            error
        ))
    })
}

async fn remove_same_path_upgrade_guard_file(guard_path: &Path) {
    if let Err(error) = crate::fs_safety::remove_file_safely_if_exists(guard_path).await {
        tracing::warn!(
            error = %error,
            guard = %guard_path.display(),
            "failed to remove same-path upgrade guard manifest"
        );
        return;
    }

    if let Some(parent) = guard_path.parent()
        && parent.file_name().and_then(|name| name.to_str()) == Some(SAME_PATH_UPGRADE_GUARD_DIR)
        && let Err(error) = tokio::fs::remove_dir(parent).await
        && error.kind() != std::io::ErrorKind::NotFound
        && error.kind() != std::io::ErrorKind::DirectoryNotEmpty
    {
        tracing::debug!(
            error = %error,
            dir = %parent.display(),
            "same-path upgrade guard directory could not be removed after cleanup"
        );
    }
}

async fn read_same_path_upgrade_guard(
    guard_path: &Path,
) -> AppResult<SamePathUpgradeGuardManifest> {
    let bytes = tokio::fs::read(guard_path).await.map_err(|error| {
        AppError::Repository(format!(
            "failed to read same-path upgrade guard {}: {}",
            guard_path.display(),
            error
        ))
    })?;
    serde_json::from_slice(&bytes).map_err(|error| {
        AppError::Repository(format!(
            "failed to decode same-path upgrade guard {}: {}",
            guard_path.display(),
            error
        ))
    })
}

async fn update_same_path_upgrade_guard_phase(guard_path: &Path, phase: &str) -> AppResult<()> {
    let mut manifest = read_same_path_upgrade_guard(guard_path).await?;
    manifest.phase = phase.to_string();
    manifest.updated_at = Utc::now().to_rfc3339();
    write_same_path_upgrade_guard(guard_path, &manifest).await
}

struct ValidatedSamePathUpgradeGuard {
    manifest: SamePathUpgradeGuardManifest,
    updated_at: DateTime<Utc>,
    final_path: PathBuf,
    backup_path: PathBuf,
}

pub(crate) async fn reconcile_same_path_upgrade_guards_locked(app: &AppUseCase) -> AppResult<u32> {
    let mut reconciled = 0u32;
    for root in app.all_library_root_folders().await? {
        let root_path = stored_path_to_path_buf(&root.path);
        reconciled += reconcile_same_path_upgrade_guards_under_root(app, &root_path).await?;
    }
    Ok(reconciled)
}

pub(crate) async fn same_path_upgrade_guard_media_file_ids_locked(
    app: &AppUseCase,
) -> AppResult<HashSet<String>> {
    let mut protected = HashSet::new();
    for root in app.all_library_root_folders().await? {
        let root_path = stored_path_to_path_buf(&root.path);
        let guard_dir = same_path_upgrade_guard_dir(&root_path);
        let mut entries = match tokio::fs::read_dir(&guard_dir).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    dir = %guard_dir.display(),
                    "failed to collect same-path upgrade guard protected ids"
                );
                continue;
            }
        };
        while let Some(entry) = entries.next_entry().await.map_err(|error| {
            AppError::Repository(format!(
                "failed to read same-path upgrade guard protected-id entry under {}: {}",
                guard_dir.display(),
                error
            ))
        })? {
            let path = entry.path();
            let Ok(file_type) = entry.file_type().await else {
                continue;
            };
            if file_type.is_symlink()
                || !file_type.is_file()
                || !is_same_path_upgrade_guard_file(&path)
            {
                continue;
            }
            let Ok(manifest) = read_same_path_upgrade_guard(&path).await else {
                continue;
            };
            if manifest.schema == SAME_PATH_UPGRADE_GUARD_SCHEMA {
                protected.insert(manifest.old_file_id.clone());
                protected.insert(manifest.replacement_file_id.clone());
            }
            if validate_same_path_upgrade_guard(&root_path, &guard_dir, &path, manifest.clone())
                .is_err()
            {
                continue;
            }
        }
    }
    Ok(protected)
}

async fn reconcile_same_path_upgrade_guards_under_root(
    app: &AppUseCase,
    root: &Path,
) -> AppResult<u32> {
    let mut reconciled = 0u32;
    let guard_dir = same_path_upgrade_guard_dir(root);
    let mut entries = match tokio::fs::read_dir(&guard_dir).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => {
            tracing::warn!(
                error = %error,
                dir = %guard_dir.display(),
                "failed to scan same-path upgrade guard directory"
            );
            return Ok(0);
        }
    };

    while let Some(entry) = entries.next_entry().await.map_err(|error| {
        AppError::Repository(format!(
            "failed to read same-path upgrade guard scan entry under {}: {}",
            guard_dir.display(),
            error
        ))
    })? {
        let path = entry.path();
        let file_type = match entry.file_type().await {
            Ok(file_type) => file_type,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    path = %path.display(),
                    "failed to read file type while scanning same-path upgrade guards"
                );
                continue;
            }
        };
        if file_type.is_symlink() || !file_type.is_file() || !is_same_path_upgrade_guard_file(&path)
        {
            continue;
        }
        match reconcile_same_path_upgrade_guard(app, root, &guard_dir, &path).await {
            Ok(true) => reconciled += 1,
            Ok(false) => {}
            Err(error) => tracing::warn!(
                error = %error,
                guard = %path.display(),
                "failed to reconcile same-path upgrade guard"
            ),
        }
    }

    Ok(reconciled)
}

fn is_same_path_upgrade_guard_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.starts_with(".scryer-upgrade-old-") && name.ends_with(".guard.json"))
        .unwrap_or(false)
}

fn validate_same_path_upgrade_guard(
    root: &Path,
    guard_dir: &Path,
    guard_path: &Path,
    manifest: SamePathUpgradeGuardManifest,
) -> Result<ValidatedSamePathUpgradeGuard, String> {
    if manifest.schema != SAME_PATH_UPGRADE_GUARD_SCHEMA {
        return Err(format!("unknown schema {}", manifest.schema));
    }
    if guard_path.parent() != Some(guard_dir) || !is_same_path_upgrade_guard_file(guard_path) {
        return Err("guard path is not in the dedicated guard directory".to_string());
    }

    let updated_at = DateTime::parse_from_rfc3339(&manifest.updated_at)
        .map_err(|error| format!("invalid updated_at timestamp: {error}"))?
        .with_timezone(&Utc);
    let manifest_root = stored_path_to_path_buf(&manifest.media_root);
    if manifest_root != root {
        return Err(format!(
            "manifest media root {} does not match scanned root {}",
            manifest_root.display(),
            root.display()
        ));
    }

    let final_path = stored_path_to_path_buf(&manifest.final_path);
    let backup_path = stored_path_to_path_buf(&manifest.backup_path);
    let staged_replacement_path = stored_path_to_path_buf(&manifest.staged_replacement_path);
    let replacement_path = stored_path_to_path_buf(&manifest.replacement_path);

    for (role, path) in [
        ("final", &final_path),
        ("backup", &backup_path),
        ("staged replacement", &staged_replacement_path),
        ("replacement", &replacement_path),
    ] {
        if !recycle_bin::restore_destination_is_under_configured_root(path, root) {
            return Err(format!(
                "{role} path is outside the scanned media root: {}",
                path.display()
            ));
        }
    }
    if replacement_path != final_path {
        return Err("replacement path does not match final path".to_string());
    }
    let final_parent = final_path
        .parent()
        .ok_or_else(|| "final path has no parent".to_string())?;
    if backup_path.parent() != Some(final_parent) {
        return Err("backup path is not a sibling of the final path".to_string());
    }
    if staged_replacement_path.parent() != Some(final_parent) {
        return Err("staged replacement path is not a sibling of the final path".to_string());
    }
    if !backup_path
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.starts_with(".scryer-upgrade-old-"))
        .unwrap_or(false)
    {
        return Err("backup filename is not an upgrade-old guard filename".to_string());
    }
    if !staged_replacement_path
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.starts_with(".scryer-upgrade-replacement-"))
        .unwrap_or(false)
    {
        return Err(
            "staged replacement filename is not an upgrade-replacement guard filename".to_string(),
        );
    }
    if same_path_upgrade_guard_path(root, &backup_path) != guard_path {
        return Err("guard filename does not match backup path".to_string());
    }

    Ok(ValidatedSamePathUpgradeGuard {
        manifest,
        updated_at,
        final_path,
        backup_path,
    })
}

fn same_path_upgrade_guard_is_recent(updated_at: DateTime<Utc>) -> bool {
    Utc::now().signed_duration_since(updated_at)
        < Duration::seconds(SAME_PATH_UPGRADE_GUARD_STALE_AFTER_SECONDS)
}

async fn same_path_guard_regular_file_exists(path: &Path, role: &str) -> AppResult<bool> {
    let metadata = match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(AppError::Repository(format!(
                "failed to stat same-path upgrade {role} {}: {}",
                path.display(),
                error
            )));
        }
    };
    if metadata.file_type().is_symlink() {
        return Err(AppError::Validation(format!(
            "refusing same-path upgrade recovery because {role} is a symlink: {}",
            path.display()
        )));
    }
    if !metadata.is_file() {
        return Err(AppError::Validation(format!(
            "refusing same-path upgrade recovery because {role} is not a regular file: {}",
            path.display()
        )));
    }
    Ok(true)
}

async fn old_media_row_active(
    app: &AppUseCase,
    manifest: &SamePathUpgradeGuardManifest,
) -> AppResult<bool> {
    Ok(app
        .services
        .library
        .media_files
        .get_media_file_by_id(&manifest.old_file_id)
        .await?
        .is_some())
}

async fn reconcile_same_path_upgrade_guard(
    app: &AppUseCase,
    root: &Path,
    guard_dir: &Path,
    guard_path: &Path,
) -> AppResult<bool> {
    let manifest = read_same_path_upgrade_guard(guard_path).await?;
    let validated = match validate_same_path_upgrade_guard(root, guard_dir, guard_path, manifest) {
        Ok(validated) => validated,
        Err(reason) => {
            tracing::warn!(
                guard = %guard_path.display(),
                reason = %reason,
                "leaving untrusted same-path upgrade guard in place"
            );
            return Ok(false);
        }
    };
    if same_path_upgrade_guard_is_recent(validated.updated_at) {
        tracing::debug!(
            guard = %guard_path.display(),
            updated_at = %validated.updated_at,
            "skipping recent same-path upgrade guard"
        );
        return Ok(false);
    }

    match validated.manifest.phase.as_str() {
        UPGRADE_GUARD_PHASE_PLANNED => {
            if !same_path_guard_regular_file_exists(&validated.backup_path, "backup").await? {
                remove_same_path_upgrade_guard_file(guard_path).await;
                return Ok(true);
            }
            if old_media_row_active(app, &validated.manifest).await? {
                return restore_same_path_guard_before_db_swap(
                    app,
                    &validated.manifest,
                    guard_path,
                    &validated.final_path,
                    &validated.backup_path,
                )
                .await;
            }
            tracing::warn!(
                guard = %guard_path.display(),
                old_file_id = %validated.manifest.old_file_id,
                "same-path upgrade guard is planned with backup present but old row is inactive; leaving in place"
            );
            Ok(false)
        }
        UPGRADE_GUARD_PHASE_OLD_MOVED => {
            if old_media_row_active(app, &validated.manifest).await? {
                restore_same_path_guard_before_db_swap(
                    app,
                    &validated.manifest,
                    guard_path,
                    &validated.final_path,
                    &validated.backup_path,
                )
                .await
            } else {
                tracing::warn!(
                    guard = %guard_path.display(),
                    old_file_id = %validated.manifest.old_file_id,
                    "same-path upgrade guard is old_moved but old row is inactive; leaving in place"
                );
                Ok(false)
            }
        }
        UPGRADE_GUARD_PHASE_REPLACEMENT_MOVED => {
            if old_media_row_active(app, &validated.manifest).await? {
                restore_same_path_guard_before_db_swap(
                    app,
                    &validated.manifest,
                    guard_path,
                    &validated.final_path,
                    &validated.backup_path,
                )
                .await
            } else {
                dispose_same_path_guard_after_confirmed_db_swap(
                    app,
                    &validated.manifest,
                    guard_path,
                    &validated.backup_path,
                )
                .await
            }
        }
        UPGRADE_GUARD_PHASE_DB_SWAPPED => {
            dispose_same_path_guard_after_confirmed_db_swap(
                app,
                &validated.manifest,
                guard_path,
                &validated.backup_path,
            )
            .await
        }
        UPGRADE_GUARD_PHASE_DISPOSED => {
            remove_same_path_upgrade_guard_file(guard_path).await;
            Ok(true)
        }
        other => {
            tracing::warn!(
                guard = %guard_path.display(),
                phase = %other,
                "leaving same-path upgrade guard with unknown phase in place"
            );
            Ok(false)
        }
    }
}

async fn restore_same_path_guard_before_db_swap(
    app: &AppUseCase,
    manifest: &SamePathUpgradeGuardManifest,
    guard_path: &Path,
    final_path: &Path,
    backup_path: &Path,
) -> AppResult<bool> {
    if !same_path_guard_regular_file_exists(backup_path, "backup").await? {
        tracing::warn!(
            guard = %guard_path.display(),
            backup = %backup_path.display(),
            "same-path upgrade guard cannot restore because backup is missing"
        );
        return Ok(false);
    }

    restore_same_path_backup(final_path, backup_path).await?;

    let staged_replacement_path = stored_path_to_path_buf(&manifest.staged_replacement_path);
    remove_imported_replacement(&staged_replacement_path).await;

    if let Err(error) = app
        .delete_media_file_record_with_dependents(&manifest.replacement_file_id)
        .await
    {
        tracing::warn!(
            error = %error,
            file_id = %manifest.replacement_file_id,
            "same-path upgrade guard restored old file but replacement DB row cleanup failed"
        );
    }
    remove_same_path_upgrade_guard_file(guard_path).await;
    Ok(true)
}

async fn dispose_same_path_guard_after_confirmed_db_swap(
    app: &AppUseCase,
    manifest: &SamePathUpgradeGuardManifest,
    guard_path: &Path,
    backup_path: &Path,
) -> AppResult<bool> {
    let old_row_active = app
        .services
        .library
        .media_files
        .get_media_file_by_id(&manifest.old_file_id)
        .await?
        .is_some();
    if old_row_active {
        tracing::warn!(
            guard = %guard_path.display(),
            old_file_id = %manifest.old_file_id,
            "same-path upgrade guard says DB swapped but old row is still active; leaving in place"
        );
        return Ok(false);
    }

    let replacement = match app
        .services
        .library
        .media_files
        .get_media_file_by_id(&manifest.replacement_file_id)
        .await?
    {
        Some(replacement) => replacement,
        None => {
            tracing::warn!(
                guard = %guard_path.display(),
                replacement_file_id = %manifest.replacement_file_id,
                "same-path upgrade guard cannot dispose backup because replacement row is missing"
            );
            return Ok(false);
        }
    };
    if replacement.file_path != manifest.replacement_path
        || !stored_path_to_path_buf(&replacement.file_path).exists()
        || replacement.title_id != manifest.title_id
    {
        tracing::warn!(
            guard = %guard_path.display(),
            replacement_file_id = %manifest.replacement_file_id,
            "same-path upgrade guard replacement validation failed; leaving backup in place"
        );
        return Ok(false);
    }
    if !same_path_guard_regular_file_exists(backup_path, "backup").await? {
        remove_same_path_upgrade_guard_file(guard_path).await;
        return Ok(true);
    }

    let manifest_media_root = stored_path_to_path_buf(&manifest.media_root);
    let recycle_config = app
        .recycle_bin_config_for_media_root_path(Some(&manifest_media_root))
        .await;
    if recycle_config.enabled {
        let recycle_metadata = recycle_bin::ReplacedMediaRecycleMetadata {
            original_path: &manifest.final_path,
            original_file_id: &manifest.old_file_id,
            size_bytes: manifest.old_size_bytes,
            title_id: &manifest.title_id,
            media_root: Some(&manifest.media_root),
        };
        let recycle_result = recycle_bin::recycle_replaced_media_file(
            &recycle_config,
            backup_path,
            recycle_metadata,
            true,
        )
        .await?;
        let replacement_path = stored_path_to_path_buf(&manifest.replacement_path);
        recycle_bin::commit_recycle_entry(
            &recycle_result,
            &manifest.replacement_file_id,
            &replacement_path,
        )
        .await?;
    } else {
        recycle_bin::ensure_source_within_roots(&recycle_config, backup_path)?;
        remove_old_file_after_verified_upgrade(backup_path).await?;
    }
    update_same_path_upgrade_guard_phase(guard_path, UPGRADE_GUARD_PHASE_DISPOSED).await?;
    remove_same_path_upgrade_guard_file(guard_path).await;
    Ok(true)
}

fn ensure_old_file_disposition_ready(
    recycle_config: &RecycleBinConfig,
    old_path: &Path,
) -> AppResult<()> {
    if recycle_config.source_roots.is_empty() {
        return Err(AppError::Validation(
            "refusing to upgrade because no configured media roots are available for old-file cleanup"
                .to_string(),
        ));
    }
    if recycle_config.enabled && !recycle_config.cleanup_enabled {
        return Err(AppError::Validation(format!(
            "refusing to upgrade because the recycle bin path is unsafe: {}",
            recycle_config
                .validation_error
                .as_deref()
                .unwrap_or("invalid recycle bin configuration")
        )));
    }
    recycle_bin::ensure_source_within_roots(recycle_config, old_path)?;
    Ok(())
}

fn upgrade_scoring_log(
    old_score: i32,
    final_score: i32,
    post_download_scoring_log: Option<String>,
    rescore_changes: &[String],
) -> String {
    if let Some(log) = post_download_scoring_log {
        let post_download = serde_json::from_str::<serde_json::Value>(&log)
            .unwrap_or_else(|_| serde_json::Value::String(log));
        return serde_json::to_string(&serde_json::json!({
            "kind": "post_download_upgrade_score",
            "old_score": old_score,
            "new_score": final_score,
            "delta": final_score - old_score,
            "post_download": post_download,
        }))
        .unwrap_or_else(|_| {
            format!(
                "upgrade {} -> {} (delta {})",
                old_score,
                final_score,
                final_score - old_score
            )
        });
    }

    format!(
        "upgrade {} -> {} (delta {}){}",
        old_score,
        final_score,
        final_score - old_score,
        if rescore_changes.is_empty() {
            String::new()
        } else {
            format!("; rescore: {}", rescore_changes.join(", "))
        }
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "preparing a replacement needs import, metadata, scoring, and episode-link context"
)]
async fn prepare_replacement_before_old_removal(
    app: &AppUseCase,
    import_id: &str,
    title: &Title,
    existing_file: &TitleMediaFile,
    source_path: &Path,
    import_path: &Path,
    final_path_string: String,
    same_final_path: bool,
    prepared: &crate::post_download_gate::PreparedImportCandidate,
    stored_quality_label: Option<&str>,
    final_score: i32,
    target_episode_ids: &[String],
    replacement_media_root: Option<&str>,
    scoring_log: &str,
    source_path_string: &str,
    import_mode: ImportMode,
    announced_size_bytes: Option<i64>,
    completed: Option<&scryer_domain::CompletedDownload>,
) -> AppResult<PreparedUpgradeReplacement> {
    let import_path_string = path_to_stored_string(import_path);
    // The replacement is transferred exactly like a first import: through the
    // record-progress importer, so the copy reports `import_transfer_*` on the
    // import record (the row shows "Copying x / y" instead of nothing — every
    // upgrade used the raw importer and never wrote progress) and the
    // library's resolved file permissions are applied instead of the
    // importer defaults.
    let destination_ownership = crate::import_workflow::ImportDestinationOwnership::upgrade(
        target_episode_ids,
        existing_file,
    );
    let file_result = crate::import_workflow::import_file_with_record_progress(
        app,
        import_id,
        &title.library_id,
        &title.facet,
        &destination_ownership,
        source_path,
        import_path,
        import_mode,
        Some(&prepared.source_snapshot),
        completed,
    )
    .await
    .map_err(|err| {
        AppError::Repository(format!(
            "upgrade import failed before old file removal: {err}"
        ))
    })?;

    let media_file_input = InsertMediaFileInput {
        title_id: title.id.clone(),
        file_path: import_path_string.clone(),
        size_bytes: file_result.size_bytes as i64,
        announced_size_bytes: crate::canonical_scoring::persisted_announced_size_bytes(
            file_result.size_bytes as i64,
            announced_size_bytes,
        ),
        quality_label: stored_quality_label
            .map(str::to_string)
            .or_else(|| prepared.parsed.quality.clone()),
        scene_name: Some(prepared.parsed.raw_title.clone()),
        release_group: prepared.parsed.release_group.clone(),
        source_type: crate::release_parser::parsed_release_source_type(&prepared.parsed),
        resolution: stored_quality_label
            .map(str::to_string)
            .or_else(|| prepared.parsed.quality.clone()),
        video_codec_parsed: prepared.parsed.video_codec,
        audio_codec_parsed: prepared.parsed.audio.as_ref().map(ToString::to_string),
        audio_channels_parsed: prepared.parsed.audio_channels.clone(),
        original_file_path: Some(source_path_string.to_string()),
        acquisition_score: Some(final_score),
        scoring_log: Some(scoring_log.to_string()),
        ..Default::default()
    };
    let persistence = match file_result
        .insert_or_reuse_media_file(app, &media_file_input)
        .await
    {
        Ok(persistence) => persistence,
        Err(err) => {
            if matches!(
                file_result.destination_disposition,
                scryer_domain::ImportDestinationDisposition::Created
            ) {
                remove_imported_replacement(import_path).await;
            }
            return Err(AppError::Repository(format!(
                "failed to insert replacement media file before old file removal: {err}"
            )));
        }
    };
    let new_file_id = persistence.media_file_id;
    let reused_existing = persistence.reused_existing;
    let destination_created = persistence.destination_created;
    crate::post_download_gate::persist_media_analysis_result(
        &app.services.library.media_files,
        &new_file_id,
        prepared.accepted.as_ref(),
    )
    .await;

    if let Err(reason) = validate_replacement_media_file(
        app,
        &new_file_id,
        &import_path_string,
        &title.id,
        replacement_media_root,
    )
    .await
    {
        rollback_new_replacement(
            app,
            &new_file_id,
            import_path,
            reused_existing,
            destination_created,
        )
        .await;
        return Err(AppError::Repository(format!(
            "replacement validation failed before old file removal: {reason}"
        )));
    }

    Ok(PreparedUpgradeReplacement {
        new_file_id,
        reused_existing,
        destination_created,
        new_size_bytes: file_result.size_bytes as i64,
        import_path: import_path.to_path_buf(),
        final_path_string,
        same_final_path,
        source_cleanup: file_result.source_cleanup.clone(),
        destination_permit: file_result.destination_permit(),
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "finalizing an upgrade needs replacement and old-file root context"
)]
async fn finalize_prepared_upgrade(
    app: &AppUseCase,
    title: &Title,
    existing_file: &TitleMediaFile,
    replacement: &PreparedUpgradeReplacement,
    recycle_config: &RecycleBinConfig,
    old_path: &Path,
    replacement_media_root: Option<&str>,
    old_file_media_root: Option<&str>,
) -> AppResult<bool> {
    if replacement.same_final_path {
        finalize_same_path_upgrade(
            app,
            title,
            existing_file,
            replacement,
            recycle_config,
            old_path,
            replacement_media_root,
            old_file_media_root,
        )
        .await
    } else {
        finalize_distinct_path_upgrade(
            app,
            title,
            existing_file,
            replacement,
            recycle_config,
            old_path,
            replacement_media_root,
            old_file_media_root,
        )
        .await
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "distinct-path finalization needs replacement and old-file root context"
)]
async fn finalize_distinct_path_upgrade(
    app: &AppUseCase,
    title: &Title,
    existing_file: &TitleMediaFile,
    replacement: &PreparedUpgradeReplacement,
    recycle_config: &RecycleBinConfig,
    old_path: &Path,
    replacement_media_root: Option<&str>,
    old_file_media_root: Option<&str>,
) -> AppResult<bool> {
    let old_disposition = match prepare_old_file_disposition_for_upgrade(
        recycle_config,
        existing_file,
        old_path,
        &existing_file.file_path,
        title,
        old_file_media_root,
    )
    .await
    {
        Ok(disposition) => disposition,
        Err(error) => {
            rollback_new_replacement(
                app,
                &replacement.new_file_id,
                &replacement.import_path,
                replacement.reused_existing,
                replacement.destination_created,
            )
            .await;
            return Err(error);
        }
    };

    if let Err(error) = app
        .services
        .library
        .media_files
        .replace_media_file_for_upgrade(
            &existing_file.id,
            &replacement.new_file_id,
            &replacement.final_path_string,
        )
        .await
    {
        let rollback_result = rollback_old_file_disposition(&old_disposition, old_path).await;
        rollback_new_replacement(
            app,
            &replacement.new_file_id,
            &replacement.import_path,
            replacement.reused_existing,
            replacement.destination_created,
        )
        .await;
        return match rollback_result {
            Ok(()) => Err(AppError::Repository(format!(
                "failed to replace media file record after old-file disposition; restored old file: {error}"
            ))),
            Err(rollback_error) => Err(AppError::Repository(format!(
                "failed to replace media file record after old-file disposition: {error}; failed to restore old file {}: {rollback_error}",
                old_path.display()
            ))),
        };
    }
    validate_replacement_media_file(
        app,
        &replacement.new_file_id,
        &replacement.final_path_string,
        &title.id,
        replacement_media_root,
    )
    .await
    .map_err(|reason| {
        AppError::Repository(format!(
            "replacement validation failed after old row removal; old-file disposition left guarded: {reason}"
        ))
    })?;
    validate_original_inactive_for_delete(
        app,
        &existing_file.id,
        &existing_file.file_path,
        &replacement.new_file_id,
    )
    .await
    .map_err(|reason| {
        AppError::Repository(format!(
            "old file deletion blocked after replacement validation: {reason}"
        ))
    })?;

    let replacement_physical_path = stored_path_to_path_buf(&replacement.final_path_string);
    commit_old_file_disposition_after_db_success(
        old_disposition,
        &replacement.new_file_id,
        &replacement_physical_path,
    )
    .await
}

async fn prepare_old_file_disposition_for_upgrade(
    recycle_config: &RecycleBinConfig,
    existing_file: &TitleMediaFile,
    old_file_source_path: &Path,
    manifest_original_path: &str,
    title: &Title,
    media_root: Option<&str>,
) -> AppResult<OldFileDisposition> {
    if recycle_config.enabled {
        let metadata = recycle_bin::ReplacedMediaRecycleMetadata {
            original_path: manifest_original_path,
            original_file_id: &existing_file.id,
            size_bytes: existing_file.size_bytes as u64,
            title_id: &title.id,
            media_root,
        };
        return recycle_bin::recycle_replaced_media_file(
            recycle_config,
            old_file_source_path,
            metadata,
            false,
        )
        .await
        .map(|result| {
            result
                .map(OldFileDisposition::PendingRecycle)
                .unwrap_or(OldFileDisposition::Noop)
        });
    }

    recycle_bin::ensure_source_within_roots(recycle_config, old_file_source_path)?;
    let metadata = match tokio::fs::symlink_metadata(old_file_source_path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(OldFileDisposition::Noop);
        }
        Err(error) => {
            return Err(AppError::Repository(format!(
                "failed to stat old file {} before upgrade disposition: {}",
                old_file_source_path.display(),
                error
            )));
        }
    };
    if metadata.file_type().is_symlink() {
        return Err(AppError::Validation(format!(
            "refusing to dispose old upgrade file {} because it is a symlink",
            old_file_source_path.display()
        )));
    }
    if !metadata.is_file() {
        return Err(AppError::Validation(format!(
            "refusing to dispose old upgrade file {} because it is not a regular file",
            old_file_source_path.display()
        )));
    }

    let backup_path = sibling_guard_path(old_file_source_path, "old");
    tokio::fs::rename(old_file_source_path, &backup_path)
        .await
        .map_err(|error| {
            AppError::Repository(format!(
                "failed to move old file into upgrade guard before DB swap: {} -> {}: {}",
                old_file_source_path.display(),
                backup_path.display(),
                error
            ))
        })?;
    Ok(OldFileDisposition::Backup(backup_path))
}

async fn commit_old_file_disposition_after_db_success(
    disposition: OldFileDisposition,
    replacement_file_id: &str,
    replacement_path: &Path,
) -> AppResult<bool> {
    match disposition {
        OldFileDisposition::Noop => Ok(false),
        OldFileDisposition::PendingRecycle(result) => {
            recycle_bin::commit_recycle_entry(&Some(result), replacement_file_id, replacement_path)
                .await?;
            Ok(true)
        }
        OldFileDisposition::Backup(backup_path) => {
            remove_old_file_after_verified_upgrade(&backup_path).await?;
            Ok(false)
        }
    }
}

async fn rollback_old_file_disposition(
    disposition: &OldFileDisposition,
    original_path: &Path,
) -> AppResult<()> {
    match disposition {
        OldFileDisposition::Noop => Ok(()),
        OldFileDisposition::PendingRecycle(result) => {
            recycle_bin::restore_recycled_file_exact(&result.recycled_path, original_path).await?;
            if let Err(error) =
                crate::fs_safety::remove_dir_all_safely_if_exists(&result.entry_dir).await
            {
                tracing::warn!(
                    error = %error,
                    entry_dir = %result.entry_dir.display(),
                    "old file restored but pending recycle entry directory could not be removed"
                );
            }
            Ok(())
        }
        OldFileDisposition::Backup(backup_path) => {
            restore_backup_to_absent_path(original_path, backup_path).await
        }
    }
}

fn resolve_same_path_upgrade_guard_root(
    recycle_config: &RecycleBinConfig,
    media_root: Option<&str>,
    old_path: &Path,
) -> AppResult<PathBuf> {
    if let Some(media_root) = media_root.map(str::trim).filter(|root| !root.is_empty()) {
        let root = stored_path_to_path_buf(media_root);
        if recycle_bin::restore_destination_is_under_configured_root(old_path, &root) {
            return Ok(root);
        }
        return Err(AppError::Validation(format!(
            "refusing same-path upgrade guard because old file {} is outside media root {}",
            old_path.display(),
            root.display()
        )));
    }

    recycle_config
        .source_roots
        .iter()
        .find(|root| recycle_bin::restore_destination_is_under_configured_root(old_path, root))
        .cloned()
        .ok_or_else(|| {
            AppError::Validation(format!(
                "refusing same-path upgrade guard because no configured media root contains {}",
                old_path.display()
            ))
        })
}

#[expect(
    clippy::too_many_arguments,
    reason = "same-path finalization needs replacement and old-file root context"
)]
async fn finalize_same_path_upgrade(
    app: &AppUseCase,
    title: &Title,
    existing_file: &TitleMediaFile,
    replacement: &PreparedUpgradeReplacement,
    recycle_config: &RecycleBinConfig,
    old_path: &Path,
    replacement_media_root: Option<&str>,
    old_file_media_root: Option<&str>,
) -> AppResult<bool> {
    let _guard = app
        .runtime
        .imports
        .same_path_upgrade_guard_lock
        .lock()
        .await;
    let guard_root =
        resolve_same_path_upgrade_guard_root(recycle_config, old_file_media_root, old_path)?;
    let backup_path = sibling_guard_path(old_path, "old");
    let guard_path = same_path_upgrade_guard_path(&guard_root, &backup_path);
    let guard = SamePathUpgradeGuardManifest::new(
        title,
        existing_file,
        replacement,
        old_path,
        &backup_path,
        &guard_root,
    );
    write_same_path_upgrade_guard(&guard_path, &guard).await?;

    if let Err(error) = tokio::fs::rename(old_path, &backup_path).await {
        remove_same_path_upgrade_guard_file(&guard_path).await;
        rollback_new_replacement(
            app,
            &replacement.new_file_id,
            &replacement.import_path,
            replacement.reused_existing,
            replacement.destination_created,
        )
        .await;
        return Err(AppError::Repository(format!(
            "failed to move old file aside before same-path upgrade: {} -> {}: {}",
            old_path.display(),
            backup_path.display(),
            error
        )));
    }
    update_same_path_upgrade_guard_phase(&guard_path, UPGRADE_GUARD_PHASE_OLD_MOVED).await?;

    if let Err(error) = tokio::fs::rename(&replacement.import_path, old_path).await {
        if let Err(restore_error) = restore_same_path_backup(old_path, &backup_path).await {
            tracing::error!(
                error = %restore_error,
                backup = %backup_path.display(),
                final_path = %old_path.display(),
                "failed to restore old file after same-path replacement move failure; the original is preserved at the backup path"
            );
            rollback_new_replacement(
                app,
                &replacement.new_file_id,
                &replacement.import_path,
                replacement.reused_existing,
                replacement.destination_created,
            )
            .await;
            return Err(AppError::Repository(format!(
                "failed to move verified replacement into final path: {} -> {}: {}; failed to restore old file from backup {} to {}: {restore_error}",
                replacement.import_path.display(),
                old_path.display(),
                error,
                backup_path.display(),
                old_path.display()
            )));
        }
        remove_same_path_upgrade_guard_file(&guard_path).await;
        rollback_new_replacement(
            app,
            &replacement.new_file_id,
            &replacement.import_path,
            replacement.reused_existing,
            replacement.destination_created,
        )
        .await;
        return Err(AppError::Repository(format!(
            "failed to move verified replacement into final path: {} -> {}: {}",
            replacement.import_path.display(),
            old_path.display(),
            error
        )));
    }
    update_same_path_upgrade_guard_phase(&guard_path, UPGRADE_GUARD_PHASE_REPLACEMENT_MOVED)
        .await?;

    if let Err(error) = app
        .services
        .library
        .media_files
        .replace_media_file_for_upgrade(
            &existing_file.id,
            &replacement.new_file_id,
            &replacement.final_path_string,
        )
        .await
    {
        if let Err(restore_error) = restore_same_path_backup(old_path, &backup_path).await {
            tracing::error!(
                error = %restore_error,
                backup = %backup_path.display(),
                final_path = %old_path.display(),
                "failed to restore old file after same-path DB swap failure; the original is preserved at the backup path"
            );
            rollback_new_replacement(
                app,
                &replacement.new_file_id,
                &replacement.import_path,
                replacement.reused_existing,
                replacement.destination_created,
            )
            .await;
            return Err(AppError::Repository(format!(
                "failed to replace same-path media file record after guarded swap: {error}; failed to restore old file from backup {} to {}: {restore_error}",
                backup_path.display(),
                old_path.display()
            )));
        }
        remove_same_path_upgrade_guard_file(&guard_path).await;
        rollback_new_replacement(
            app,
            &replacement.new_file_id,
            &replacement.import_path,
            replacement.reused_existing,
            replacement.destination_created,
        )
        .await;
        return Err(AppError::Repository(format!(
            "failed to replace same-path media file record after guarded swap: {error}"
        )));
    }
    update_same_path_upgrade_guard_phase(&guard_path, UPGRADE_GUARD_PHASE_DB_SWAPPED).await?;

    if let Err(reason) = validate_replacement_media_file(
        app,
        &replacement.new_file_id,
        &replacement.final_path_string,
        &title.id,
        replacement_media_root,
    )
    .await
    {
        return Err(AppError::Repository(format!(
            "replacement validation failed after same-path swap; old file kept at {}: {reason}",
            backup_path.display()
        )));
    }
    validate_original_inactive_for_delete(
        app,
        &existing_file.id,
        &existing_file.file_path,
        &replacement.new_file_id,
    )
    .await
    .map_err(|reason| {
        AppError::Repository(format!(
            "same-path old file deletion blocked after replacement validation; old file kept at {}: {reason}",
            backup_path.display()
        ))
    })?;

    let replacement_physical_path = stored_path_to_path_buf(&replacement.final_path_string);
    let recycled = dispose_old_file_after_verified_upgrade(
        recycle_config,
        existing_file,
        &backup_path,
        &existing_file.file_path,
        title,
        old_file_media_root,
        &replacement.new_file_id,
        &replacement_physical_path,
    )
    .await?;
    update_same_path_upgrade_guard_phase(&guard_path, UPGRADE_GUARD_PHASE_DISPOSED).await?;
    remove_same_path_upgrade_guard_file(&guard_path).await;
    Ok(recycled)
}

#[expect(
    clippy::too_many_arguments,
    reason = "old-file disposition needs original, replacement, and recycle context"
)]
async fn dispose_old_file_after_verified_upgrade(
    recycle_config: &RecycleBinConfig,
    existing_file: &TitleMediaFile,
    old_file_source_path: &Path,
    manifest_original_path: &str,
    title: &Title,
    media_root: Option<&str>,
    replacement_file_id: &str,
    replacement_path: &Path,
) -> AppResult<bool> {
    if !recycle_config.enabled {
        recycle_bin::ensure_source_within_roots(recycle_config, old_file_source_path)?;
        remove_old_file_after_verified_upgrade(old_file_source_path).await?;
        return Ok(false);
    }

    let metadata = recycle_bin::ReplacedMediaRecycleMetadata {
        original_path: manifest_original_path,
        original_file_id: &existing_file.id,
        size_bytes: existing_file.size_bytes as u64,
        title_id: &title.id,
        media_root,
    };
    let recycle_result = recycle_bin::recycle_replaced_media_file(
        recycle_config,
        old_file_source_path,
        metadata,
        true,
    )
    .await?;

    if recycle_result.is_none() {
        return Ok(false);
    }

    if let Err(error) =
        recycle_bin::commit_recycle_entry(&recycle_result, replacement_file_id, replacement_path)
            .await
    {
        tracing::warn!(
            error = %error,
            file_id = %replacement_file_id,
            "replacement imported but recycle entry could not be committed; it will not auto-purge"
        );
        return Ok(false);
    }

    Ok(true)
}

async fn validate_replacement_media_file(
    app: &AppUseCase,
    replacement_file_id: &str,
    replacement_path: &str,
    title_id: &str,
    media_root: Option<&str>,
) -> Result<(), String> {
    let replacement = app
        .services
        .library
        .media_files
        .get_media_file_by_id(replacement_file_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "replacement media file row is missing".to_string())?;

    if replacement.file_path != replacement_path {
        return Err(format!(
            "replacement media file path mismatch: expected={} db={}",
            replacement_path, replacement.file_path
        ));
    }
    let replacement_path_buf = stored_path_to_path_buf(&replacement.file_path);
    if !replacement_path_buf.exists() {
        return Err(format!(
            "replacement media file does not exist on disk: {}",
            replacement.file_path
        ));
    }
    if replacement.title_id != title_id {
        return Err(format!(
            "replacement title mismatch: expected={} db={}",
            title_id, replacement.title_id
        ));
    }
    if let Some(media_root) = media_root.map(str::trim).filter(|root| !root.is_empty())
        && !recycle_bin::path_is_under_configured_root(
            &replacement_path_buf,
            &stored_path_to_path_buf(media_root),
        )
    {
        return Err(format!(
            "replacement path is outside media root: replacement={} root={}",
            replacement.file_path, media_root
        ));
    }

    Ok(())
}

async fn validate_original_inactive_for_delete(
    app: &AppUseCase,
    original_file_id: &str,
    original_path: &str,
    replacement_file_id: &str,
) -> Result<(), String> {
    if app
        .services
        .library
        .media_files
        .get_media_file_by_id(original_file_id)
        .await
        .map_err(|error| error.to_string())?
        .is_some()
    {
        return Err("original media file row is still active".to_string());
    }

    if let Some(active_at_original_path) = app
        .services
        .library
        .media_files
        .get_media_file_by_path(original_path)
        .await
        .map_err(|error| error.to_string())?
        && active_at_original_path.id != replacement_file_id
    {
        return Err(format!(
            "original path is active for a different media file: {}",
            active_at_original_path.id
        ));
    }

    Ok(())
}

async fn rollback_new_replacement(
    app: &AppUseCase,
    new_file_id: &str,
    path: &Path,
    reused_existing: bool,
    destination_created: bool,
) {
    if !reused_existing {
        let _ = app
            .delete_media_file_record_with_dependents(new_file_id)
            .await;
    }
    if !reused_existing && destination_created {
        remove_imported_replacement(path).await;
    }
}

fn sibling_guard_path(path: &Path, label: &str) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("media"));
    parent.join(format!(
        ".scryer-upgrade-{}-{}-{}",
        label,
        scryer_domain::Id::new().0,
        file_name.to_string_lossy()
    ))
}

async fn restore_same_path_backup(final_path: &Path, backup_path: &Path) -> AppResult<()> {
    if !same_path_guard_regular_file_exists(backup_path, "backup").await? {
        return Err(AppError::Repository(format!(
            "failed to restore old file after guarded same-path upgrade failure because backup is missing: {}",
            backup_path.display()
        )));
    }

    let moved_occupant = if same_path_guard_regular_file_exists(final_path, "final occupant")
        .await?
    {
        let moved_aside = sibling_guard_path(final_path, "rollback-occupant");
        tokio::fs::rename(final_path, &moved_aside)
            .await
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to move occupied final path aside before restoring backup: {} -> {}: {}",
                    final_path.display(),
                    moved_aside.display(),
                    error
                ))
            })?;
        Some(moved_aside)
    } else {
        None
    };

    if let Err(error) = tokio::fs::rename(backup_path, final_path).await {
        if let Some(moved_aside) = moved_occupant.as_ref() {
            match tokio::fs::rename(moved_aside, final_path).await {
                Ok(()) => {
                    return Err(AppError::Repository(format!(
                        "failed to restore old file after guarded same-path upgrade failure: {} -> {}: {}; moved-aside occupant {} was restored",
                        backup_path.display(),
                        final_path.display(),
                        error,
                        moved_aside.display()
                    )));
                }
                Err(restore_error) => {
                    return Err(AppError::Repository(format!(
                        "failed to restore old file after guarded same-path upgrade failure: {} -> {}: {}; moved-aside occupant {} could not be restored to {}: {}",
                        backup_path.display(),
                        final_path.display(),
                        error,
                        moved_aside.display(),
                        final_path.display(),
                        restore_error
                    )));
                }
            }
        }
        return Err(AppError::Repository(format!(
            "failed to restore old file after guarded same-path upgrade failure: {} -> {}: {}",
            backup_path.display(),
            final_path.display(),
            error
        )));
    }

    if let Some(moved_aside) = moved_occupant
        && let Err(error) = crate::fs_safety::remove_file_safely_if_exists(&moved_aside).await
    {
        tracing::warn!(
            error = %error,
            path = %moved_aside.display(),
            "old file restored but moved-aside same-path replacement could not be removed"
        );
    }

    Ok(())
}

async fn restore_backup_to_absent_path(final_path: &Path, backup_path: &Path) -> AppResult<()> {
    match tokio::fs::symlink_metadata(final_path).await {
        Ok(_) => {
            return Err(AppError::Validation(format!(
                "refusing to restore old upgrade backup {} because destination is occupied: {}",
                backup_path.display(),
                final_path.display()
            )));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(AppError::Repository(format!(
                "failed to stat old upgrade restore destination {}: {}",
                final_path.display(),
                error
            )));
        }
    }
    if !same_path_guard_regular_file_exists(backup_path, "backup").await? {
        return Err(AppError::Repository(format!(
            "failed to restore old upgrade backup because backup is missing: {}",
            backup_path.display()
        )));
    }
    tokio::fs::rename(backup_path, final_path)
        .await
        .map_err(|error| {
            AppError::Repository(format!(
                "failed to restore old upgrade backup: {} -> {}: {}",
                backup_path.display(),
                final_path.display(),
                error
            ))
        })
}

async fn remove_old_file_after_verified_upgrade(path: &Path) -> AppResult<()> {
    if let Err(error) = crate::fs_safety::remove_file_safely_if_exists(path).await {
        return Err(AppError::Repository(format!(
            "failed to remove old file after replacement validation {}: {}",
            path.display(),
            error
        )));
    }
    Ok(())
}

struct UpgradeEventDetails<'a> {
    new_file_id: &'a str,
    new_size_bytes: i64,
    dest_path_string: &'a str,
    old_score: i32,
    final_score: i32,
    episode_ids: &'a [String],
}

async fn append_upgrade_event(
    app: &AppUseCase,
    actor: DomainEventActor,
    title: &Title,
    existing_file: &TitleMediaFile,
    details: UpgradeEventDetails<'_>,
) -> AppResult<()> {
    let media_updates = if existing_file.file_path == details.dest_path_string {
        vec![modified_media_update(details.dest_path_string.to_string())]
    } else {
        vec![
            deleted_media_update(existing_file.file_path.clone()),
            created_media_update(details.dest_path_string.to_string()),
        ]
    };
    let mut episode_ids = details.episode_ids.to_vec();
    if episode_ids.is_empty()
        && let Some(episode_id) = existing_file.episode_id.clone()
    {
        episode_ids.push(episode_id);
    }
    episode_ids.sort();
    episode_ids.dedup();
    app.append_domain_event(new_title_domain_event(
        actor,
        title,
        DomainEventPayload::MediaFileUpgraded(MediaFileUpgradedEventData {
            title: title_context_snapshot(title),
            media_updates,
            episode_ids,
            previous_file_id: Some(existing_file.id.clone()),
            current_file_id: Some(details.new_file_id.to_string()),
            old_score: Some(details.old_score),
            new_score: Some(details.final_score),
            size_bytes: Some(details.new_size_bytes),
        }),
    ))
    .await
    .map(|_| ())
}

async fn append_upgrade_recycle_event(
    app: &AppUseCase,
    actor: DomainEventActor,
    title: &Title,
    existing_file: &TitleMediaFile,
    target_episode_ids: &[String],
) {
    let mut episode_ids = target_episode_ids.to_vec();
    if episode_ids.is_empty()
        && let Some(episode_id) = existing_file.episode_id.clone()
    {
        episode_ids.push(episode_id);
    }
    episode_ids.sort();
    episode_ids.dedup();
    let _ = app
        .append_domain_event(new_title_domain_event(
            actor,
            title,
            DomainEventPayload::MediaFileDeleted(MediaFileDeletedEventData {
                title: title_context_snapshot(title),
                media_updates: vec![deleted_media_update(existing_file.file_path.clone())],
                file_id: Some(existing_file.id.clone()),
                reason: MediaFileDeletedReason::UpgradeCleanup,
                episode_ids,
            }),
        ))
        .await
        .inspect_err(|error| {
            tracing::warn!(
                error = %error,
                file_id = %existing_file.id,
                "old media file recycled during upgrade but audit event could not be recorded"
            );
        });
}

async fn remove_imported_replacement(dest_path: &std::path::Path) {
    if let Err(remove_err) = crate::fs_safety::remove_file_safely_if_exists(dest_path).await {
        tracing::error!(
            error = %remove_err,
            path = %dest_path.display(),
            "failed to remove imported replacement after upgrade database failure"
        );
    }
}

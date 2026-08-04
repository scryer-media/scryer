use super::*;
use crate::domain_events::{
    DomainEventActor, deleted_media_update, new_title_domain_event, title_context_snapshot,
};
use crate::events::retention::{
    OPERATIONAL_DOMAIN_EVENT_RETENTION_DAYS, operational_domain_event_types,
    user_facing_domain_event_types,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

const RELEASE_DECISION_RETENTION_DAYS: i64 = 30;
const RELEASE_ATTEMPT_RETENTION_DAYS: i64 = 90;
const DOWNLOAD_DELETE_RETENTION_DAYS: i64 = 7;
const DISCOVERY_SUCCESSFUL_GENERATIONS_TO_RETAIN: usize = 2;
const DISCOVERY_DIAGNOSTIC_RETENTION_DAYS: i64 = 30;
const TITLE_IMAGE_BLOB_GC_BATCH_SIZE: u32 = 100;
const TITLE_IMAGE_BLOB_GC_MAX_BATCHES: usize = 10;
const RECYCLE_BIN_BATCH_MAX_ITEMS: usize = 250;

fn normalize_recycle_batch_ids(entry_ids: Vec<String>) -> AppResult<Vec<String>> {
    if entry_ids.is_empty() {
        return Err(AppError::Validation(
            "select at least one recycle-bin item".to_string(),
        ));
    }
    if entry_ids.len() > RECYCLE_BIN_BATCH_MAX_ITEMS {
        return Err(AppError::Validation(format!(
            "recycle-bin batches are limited to {RECYCLE_BIN_BATCH_MAX_ITEMS} items"
        )));
    }

    let requested_count = entry_ids.len();
    let mut normalized = entry_ids;
    normalized.sort();
    normalized.dedup();
    if normalized.len() != requested_count {
        return Err(AppError::Validation(
            "recycle-bin batches cannot include the same item more than once".to_string(),
        ));
    }
    for entry_id in &normalized {
        crate::recycle_bin::validate_recycle_entry_id(entry_id)?;
    }
    Ok(normalized)
}

async fn restore_target_fingerprint(path: &Path) -> AppResult<String> {
    match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) if metadata.file_type().is_symlink() => Ok("symlink".to_string()),
        Ok(metadata) if metadata.is_file() => {
            let modified = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|duration| format!("{}:{}", duration.as_secs(), duration.subsec_nanos()))
                .unwrap_or_else(|| "unknown".to_string());
            Ok(format!("file:{}:{modified}", metadata.len()))
        }
        Ok(_) => Ok("non-file".to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok("missing".to_string()),
        Err(error) => Err(AppError::Repository(format!(
            "failed to inspect restore destination {}: {}",
            path.display(),
            error
        ))),
    }
}

async fn validate_replace_existing_restore_target(
    path: &Path,
    recycle_config: &crate::recycle_bin::RecycleBinConfig,
) -> AppResult<()> {
    let metadata = match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(AppError::Repository(format!(
                "failed to inspect restore destination {}: {}",
                path.display(),
                error
            )));
        }
    };

    if metadata.file_type().is_symlink() {
        return Err(AppError::Validation(format!(
            "refusing to replace restore destination {} because it is a symlink",
            path.display()
        )));
    }
    if !metadata.is_file() {
        return Err(AppError::Validation(format!(
            "refusing to replace restore destination {} because it is not a regular file",
            path.display()
        )));
    }
    if !recycle_config.enabled || !recycle_config.cleanup_enabled {
        return Err(AppError::Validation(format!(
            "refusing to replace {} because the recycle bin is unavailable",
            path.display()
        )));
    }

    Ok(())
}

#[derive(Clone, Debug)]
struct RecycleEntryLibrary {
    id: String,
    name: String,
}

#[derive(Clone, Debug)]
struct RecycleRootLibrary {
    media_root: String,
    normalized_media_root: String,
    library: RecycleEntryLibrary,
}

struct RestoreRecycledItemContext {
    entry_dir: PathBuf,
    manifest: crate::recycle_bin::RecycleManifest,
    library_id: String,
    library_roots: Vec<PathBuf>,
    recycle_config: crate::recycle_bin::RecycleBinConfig,
}

#[derive(Clone, Debug)]
pub struct RestoreRecycledItemJobAccepted {
    pub job_run: JobRun,
}

fn recycle_path_is_under_root(path: &str, root: &str) -> bool {
    let path = crate::stored_paths::stored_path_to_path_buf(path);
    let root = crate::stored_paths::stored_path_to_path_buf(root);
    crate::recycle_bin::path_is_under_configured_root(&path, &root)
}

fn recycle_library_root_paths(
    library: &RecycleEntryLibrary,
    roots: &[RecycleRootLibrary],
) -> Vec<PathBuf> {
    roots
        .iter()
        .filter(|root| root.library.id == library.id)
        .map(|root| crate::stored_paths::stored_path_to_path_buf(&root.media_root))
        .collect()
}

fn recycle_restore_destination_is_under_library_roots(
    path: &Path,
    library_roots: &[PathBuf],
) -> bool {
    crate::recycle_bin::restore_destination_is_under_configured_roots(path, library_roots)
}

fn recycled_item_from_entry(
    entry: crate::recycle_bin::RecycleEntry,
    library: &RecycleEntryLibrary,
    title_name: Option<String>,
) -> RecycledItem {
    let original_path = entry.manifest.original_path_buf();
    let file_name = original_path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();

    RecycledItem {
        id: entry.entry_id,
        original_path: entry.manifest.original_path,
        file_name,
        size_bytes: entry.manifest.size_bytes,
        title_id: entry.manifest.title_id,
        title_name,
        reason: entry.manifest.reason,
        recycled_at: entry.manifest.recycled_at,
        media_root: entry.media_root,
        library_id: library.id.clone(),
        library_name: library.name.clone(),
    }
}

impl AppUseCase {
    /// Resolve media root paths and their recycle configs.
    async fn resolve_all_recycle_configs(
        &self,
    ) -> Vec<(String, crate::recycle_bin::RecycleBinConfig)> {
        let mut media_roots = Vec::new();
        let mut seen_roots = HashSet::new();

        match self.recycle_root_libraries().await {
            Ok(roots) => {
                for root in roots {
                    if seen_roots.insert(root.normalized_media_root) {
                        media_roots.push(root.media_root);
                    }
                }
            }
            Err(error) => {
                warn!(
                    error = %error,
                    "failed to resolve library roots for recycle bin housekeeping"
                );
            }
        }

        self.recycle_bin_configs_for_media_roots(media_roots).await
    }

    async fn recycle_root_libraries(&self) -> AppResult<Vec<RecycleRootLibrary>> {
        Ok(self
            .all_library_root_folders()
            .await?
            .into_iter()
            .map(|root| RecycleRootLibrary {
                media_root: root.path,
                normalized_media_root: root.normalized_path,
                library: RecycleEntryLibrary {
                    id: root.library_id,
                    name: root.library_name,
                },
            })
            .collect())
    }

    async fn resolve_recycle_entry_library(
        &self,
        entry: &crate::recycle_bin::RecycleEntry,
        roots: &[RecycleRootLibrary],
    ) -> AppResult<Option<RecycleEntryLibrary>> {
        let title_library_id = match entry.manifest.title_id.as_deref() {
            Some(title_id) => self
                .services
                .catalog
                .titles
                .get_by_id(title_id)
                .await?
                .map(|title| title.library_id),
            None => None,
        };

        Ok(resolve_recycle_entry_library_with_title(
            entry,
            roots,
            title_library_id.as_deref(),
        ))
    }
}

fn resolve_recycle_entry_library_with_title(
    entry: &crate::recycle_bin::RecycleEntry,
    roots: &[RecycleRootLibrary],
    title_library_id: Option<&str>,
) -> Option<RecycleEntryLibrary> {
    if let Some(title_library_id) = title_library_id
        && let Some(root) = roots
            .iter()
            .find(|root| root.library.id == title_library_id)
    {
        return Some(root.library.clone());
    }

    if let Some(root) = roots.iter().find(|root| {
        recycle_path_is_under_root(&entry.manifest.original_path, root.media_root.as_str())
    }) {
        return Some(root.library.clone());
    }

    let normalized_media_root =
        crate::catalog_workflow::normalize_library_root_path(&entry.media_root);
    if normalized_media_root.is_empty() {
        return None;
    }

    roots
        .iter()
        .find(|root| root.normalized_media_root == normalized_media_root)
        .map(|root| root.library.clone())
}

impl AppUseCase {
    async fn require_recycle_bin_page_access(&self, actor: &User) -> AppResult<()> {
        if self
            .has_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?
            || self
                .has_any_granted_library_permission(
                    actor,
                    scryer_domain::LibraryPermission::ManageTitles,
                )
                .await?
        {
            return Ok(());
        }

        Err(AppError::Unauthorized(
            "You do not have permission to view the recycle bin".to_string(),
        ))
    }

    async fn selected_recycle_library_ids(
        &self,
        actor: &User,
        library_ids: Option<Vec<String>>,
    ) -> AppResult<HashSet<String>> {
        let allowed = self
            .granted_library_ids_for_permission(
                actor,
                None,
                scryer_domain::LibraryPermission::ManageTitles,
            )
            .await?
            .into_iter()
            .collect::<HashSet<_>>();

        let Some(library_ids) = library_ids else {
            return Ok(allowed);
        };

        let requested = library_ids
            .into_iter()
            .map(|library_id| library_id.trim().to_string())
            .filter(|library_id| !library_id.is_empty())
            .collect::<HashSet<_>>();

        if requested.is_empty() {
            return Ok(allowed);
        }

        Ok(allowed
            .into_iter()
            .filter(|library_id| requested.contains(library_id))
            .collect())
    }

    async fn purge_expired_recycle_entries(
        &self,
        media_root: &str,
        config: &crate::recycle_bin::RecycleBinConfig,
    ) -> AppResult<u32> {
        let mut purged = 0u32;
        for entry in crate::recycle_bin::list_expired_committed_entries(config).await? {
            let Some(entry_id) = entry.manifest.entry_id.as_deref() else {
                warn!(
                    path = %entry.entry_dir.display(),
                    "skipping recycle entry with no identifier during expiry sweep"
                );
                continue;
            };
            let guard_key = format!("recycle-entry:{entry_id}");
            let Some(_move_guard) = self
                .runtime
                .jobs
                .interactive_operation_guards
                .try_acquire(&guard_key)
                .await
            else {
                info!(
                    entry_id,
                    "skipping recycle entry while an interactive recycle operation is moving it"
                );
                continue;
            };
            if self
                .purge_recycle_entry_after_validation(
                    media_root,
                    config,
                    &entry,
                    DomainEventActor::system(),
                )
                .await?
            {
                purged += 1;
            }
        }
        Ok(purged)
    }

    pub(crate) async fn purge_recycle_entry_after_validation(
        &self,
        media_root: &str,
        config: &crate::recycle_bin::RecycleBinConfig,
        entry: &crate::recycle_bin::CommittedRecycleEntry,
        actor: impl Into<DomainEventActor>,
    ) -> AppResult<bool> {
        let actor = actor.into();
        if let Err(reason) = self
            .validate_recycle_entry_before_permanent_delete(&entry.manifest)
            .await
        {
            warn!(
                media_root = %media_root,
                path = %entry.entry_dir.display(),
                reason = %reason,
                "quarantining recycle entry that failed purge validation"
            );
            if let Err(error) =
                crate::recycle_bin::quarantine_entry(&entry.entry_dir, &entry.manifest, &reason)
                    .await
            {
                warn!(
                    media_root = %media_root,
                    path = %entry.entry_dir.display(),
                    error = %error,
                    "failed to quarantine unsafe recycle entry"
                );
            }
            return Ok(false);
        }

        let purged = crate::recycle_bin::purge_committed_entry(config, entry).await?;
        if purged {
            self.record_recycle_entry_purged_event(actor, &entry.manifest)
                .await;
        }
        Ok(purged)
    }

    async fn record_recycle_entry_purged_event(
        &self,
        actor: DomainEventActor,
        manifest: &crate::recycle_bin::RecycleManifest,
    ) {
        let Some(title_id) = manifest.title_id.as_deref() else {
            return;
        };
        let title = match self.services.catalog.titles.get_by_id(title_id).await {
            Ok(Some(title)) => title,
            Ok(None) => return,
            Err(error) => {
                warn!(
                    title_id = %title_id,
                    error = %error,
                    "recycle entry purged but title could not be loaded for audit event"
                );
                return;
            }
        };
        let event = new_title_domain_event(
            actor,
            &title,
            scryer_domain::DomainEventPayload::MediaFileDeleted(
                scryer_domain::MediaFileDeletedEventData {
                    title: title_context_snapshot(&title),
                    media_updates: vec![deleted_media_update(manifest.original_path.clone())],
                    file_id: manifest.original_file_id.clone(),
                    reason: scryer_domain::MediaFileDeletedReason::RecycleBinPurged,
                    episode_ids: Vec::new(),
                },
            ),
        );
        if let Err(error) = self.append_domain_event(event).await {
            warn!(
                title_id = %title_id,
                error = %error,
                "recycle entry purged but audit event could not be recorded"
            );
        }
    }

    async fn validate_recycle_entry_before_permanent_delete(
        &self,
        manifest: &crate::recycle_bin::RecycleManifest,
    ) -> Result<(), String> {
        if manifest.reason != "upgrade_replaced" {
            return Ok(());
        }

        let title_id = manifest
            .title_id
            .as_deref()
            .ok_or_else(|| "missing title id".to_string())?;
        let original_file_id = manifest
            .original_file_id
            .as_deref()
            .ok_or_else(|| "missing original media file id".to_string())?;
        let media_root = manifest
            .media_root
            .as_deref()
            .ok_or_else(|| "missing media root".to_string())?;
        if media_root.trim().is_empty() {
            return Err("missing media root".to_string());
        }
        let media_root_path = crate::stored_paths::stored_path_to_path_buf(media_root);
        let original_path = crate::stored_paths::stored_path_to_path_buf(&manifest.original_path);
        if !crate::recycle_bin::path_is_under_configured_root(&original_path, &media_root_path) {
            return Err(format!(
                "original path is outside manifest media root: original={} root={}",
                manifest.original_path, media_root
            ));
        }

        let replacement_file_id = manifest
            .replacement_file_id
            .as_deref()
            .ok_or_else(|| "missing replacement media file id".to_string())?;
        let replacement_path = manifest
            .replacement_path
            .as_deref()
            .ok_or_else(|| "missing replacement media file path".to_string())?;
        let replacement = self
            .services
            .library
            .media_files
            .get_media_file_by_id(replacement_file_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "replacement media file row is missing".to_string())?;

        if replacement.file_path != replacement_path {
            return Err(format!(
                "replacement media file path mismatch: manifest={} db={}",
                replacement_path, replacement.file_path
            ));
        }
        let replacement_path_buf =
            crate::stored_paths::stored_path_to_path_buf(&replacement.file_path);
        if !replacement_path_buf.exists() {
            return Err(format!(
                "replacement media file does not exist on disk: {}",
                replacement.file_path
            ));
        }
        if replacement.title_id != title_id {
            return Err(format!(
                "replacement title mismatch: manifest={} db={}",
                title_id, replacement.title_id
            ));
        }
        if !crate::recycle_bin::path_is_under_configured_root(
            &replacement_path_buf,
            &media_root_path,
        ) {
            return Err(format!(
                "replacement path is outside manifest media root: replacement={} root={}",
                replacement.file_path, media_root
            ));
        }
        if self
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
        if let Some(active_at_original_path) = self
            .services
            .library
            .media_files
            .get_media_file_by_path(&manifest.original_path)
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

    pub async fn run_housekeeping(&self, actor: &User) -> AppResult<HousekeepingReport> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        self.run_scheduled_housekeeping().await
    }

    pub(crate) async fn run_scheduled_housekeeping(&self) -> AppResult<HousekeepingReport> {
        info!("starting housekeeping");
        let orphaned_media_files = {
            let _same_path_upgrade_guard = self
                .runtime
                .imports
                .same_path_upgrade_guard_lock
                .lock()
                .await;
            match crate::import::upgrade::reconcile_same_path_upgrade_guards_locked(self).await {
                Ok(reconciled) if reconciled > 0 => {
                    info!(reconciled, "reconciled same-path upgrade guards")
                }
                Ok(_) => {}
                Err(error) => warn!(
                    error = %error,
                    "failed to reconcile same-path upgrade guards"
                ),
            }

            let protected_upgrade_file_ids =
                match crate::import::upgrade::same_path_upgrade_guard_media_file_ids_locked(self)
                    .await
                {
                    Ok(ids) => ids,
                    Err(error) => {
                        warn!(
                            error = %error,
                            "failed to collect same-path upgrade guard protected media files"
                        );
                        HashSet::new()
                    }
                };

            // 1. Orphaned media files (file_path no longer exists on disk).
            // Root availability is checked before probing the file path so a
            // disconnected media root cannot make every row look orphaned.
            let all_files = self
                .services
                .workflow
                .housekeeping
                .list_media_files_with_roots()
                .await?;
            let mut orphaned_media_files = 0u32;
            for media_file in all_files {
                if protected_upgrade_file_ids.contains(&media_file.media_file_id) {
                    continue;
                }

                let file_path = crate::stored_paths::stored_path_to_path_buf(&media_file.file_path);
                let roots = media_file
                    .root_paths
                    .iter()
                    .map(|root| crate::stored_paths::stored_path_to_path_buf(root))
                    .collect::<Vec<_>>();
                if let Err(error) =
                    crate::fs_safety::resolve_available_root_for_path(&file_path, &roots)
                {
                    warn!(
                        error = %error,
                        file_id = %media_file.media_file_id,
                        path = %file_path.display(),
                        "skipping orphan media-file cleanup because media root is unavailable"
                    );
                    continue;
                }

                match std::fs::symlink_metadata(&file_path) {
                    Ok(_) => continue,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        warn!(
                            error = %error,
                            file_id = %media_file.media_file_id,
                            path = %file_path.display(),
                            "skipping orphan media-file cleanup because file status could not be read"
                        );
                        continue;
                    }
                }

                if let Err(error) = self
                    .delete_media_file_record_with_dependents(&media_file.media_file_id)
                    .await
                {
                    warn!(
                        error = %error,
                        file_id = %media_file.media_file_id,
                        "skipping orphan media-file cleanup because row cleanup failed"
                    );
                    continue;
                }
                orphaned_media_files = orphaned_media_files.saturating_add(1);
            }
            orphaned_media_files
        };

        let general_settings = self.general_settings().await?;
        let history_retention_days = general_settings.history_retention_days as i64;
        let user_facing_domain_event_types = user_facing_domain_event_types();
        let operational_domain_event_types = operational_domain_event_types();

        let stale_release_decisions = self
            .services
            .workflow
            .housekeeping
            .delete_release_decisions_older_than(RELEASE_DECISION_RETENTION_DAYS)
            .await?;
        let stale_release_attempts = self
            .services
            .workflow
            .housekeeping
            .delete_release_attempts_older_than(RELEASE_ATTEMPT_RETENTION_DAYS)
            .await?;

        let (
            stale_history_events,
            stale_domain_events,
            stale_download_import_artifacts,
            stale_import_history,
            stale_download_queue_deletes,
            stale_rule_set_history,
        ) = if general_settings.keep_history_forever {
            (0, 0, 0, 0, 0, 0)
        } else {
            (
                self.services
                    .workflow
                    .housekeeping
                    .delete_history_events_older_than(history_retention_days)
                    .await?,
                self.services
                    .workflow
                    .housekeeping
                    .delete_domain_events_older_than_for_types(
                        history_retention_days,
                        &user_facing_domain_event_types,
                    )
                    .await?,
                self.services
                    .workflow
                    .housekeeping
                    .delete_download_import_artifacts_older_than(history_retention_days)
                    .await?,
                self.services
                    .workflow
                    .housekeeping
                    .delete_terminal_imports_older_than(history_retention_days)
                    .await?,
                self.services
                    .workflow
                    .housekeeping
                    .delete_terminal_download_queue_commands_older_than(
                        DOWNLOAD_DELETE_RETENTION_DAYS,
                    )
                    .await?,
                self.services
                    .workflow
                    .housekeeping
                    .delete_rule_set_history_older_than(history_retention_days)
                    .await?,
            )
        };
        let stale_operational_domain_events = self
            .services
            .workflow
            .housekeeping
            .delete_domain_events_older_than_for_types(
                OPERATIONAL_DOMAIN_EVENT_RETENTION_DAYS,
                &operational_domain_event_types,
            )
            .await?;

        let stale_history_records = stale_release_decisions
            + stale_release_attempts
            + stale_operational_domain_events
            + stale_history_events
            + stale_domain_events
            + stale_download_import_artifacts
            + stale_import_history
            + stale_download_queue_deletes
            + stale_rule_set_history;

        // 3. Stale staged NZB artifacts (> 1 hour old)
        let now = self.runtime.environment.now();
        let staged_nzb_artifacts_pruned = self
            .services
            .workflow
            .staged_nzb_store
            .prune_staged_nzbs_older_than(now - chrono::Duration::hours(1))
            .await?;

        // 4. Purge expired recycle bin entries (per media root)
        let mut recycled_purged = 0u32;
        for (media_root, config) in self.resolve_all_recycle_configs().await {
            match self
                .purge_expired_recycle_entries(&media_root, &config)
                .await
            {
                Ok(n) => recycled_purged += n,
                Err(e) => info!(error = %e, media_root = %media_root, "recycle bin purge failed"),
            }
        }

        // 5. Commit stale pending recycle entries orphaned by interrupted move
        // transactions (per media root). This deliberately runs after the
        // expiry purge: an entry committed here stays restorable for at least
        // one housekeeping interval before any sweep can expire it.
        let mut recycled_pending_reconciled = 0u32;
        for (media_root, config) in self.resolve_all_recycle_configs().await {
            match crate::recycle_bin::reconcile_stale_pending_entries(&config).await {
                Ok(n) => recycled_pending_reconciled += n,
                Err(e) => info!(
                    error = %e,
                    media_root = %media_root,
                    "recycle bin pending reconciliation failed"
                ),
            }
        }

        // 6. Discovery history retention.
        let discovery_pruned_runs = match self
            .services
            .library
            .discovery
            .prune_discovery_history(
                DISCOVERY_DEFAULT_SCOPE_KEY,
                DISCOVERY_SUCCESSFUL_GENERATIONS_TO_RETAIN,
                now - chrono::Duration::days(DISCOVERY_DIAGNOSTIC_RETENTION_DAYS),
            )
            .await
        {
            Ok(report) => report.runs_deleted as u32,
            Err(error) => {
                warn!(
                    error = %error,
                    "failed to prune discovery history during housekeeping"
                );
                0
            }
        };

        {
            let _title_image_maintenance_guard = self
                .runtime
                .catalog
                .title_image_maintenance_lock
                .write()
                .await;
            for _ in 0..TITLE_IMAGE_BLOB_GC_MAX_BATCHES {
                let pruned = self
                    .services
                    .workflow
                    .housekeeping
                    .prune_unreferenced_title_image_blobs(TITLE_IMAGE_BLOB_GC_BATCH_SIZE)
                    .await?;
                if pruned < TITLE_IMAGE_BLOB_GC_BATCH_SIZE {
                    break;
                }
            }
        }

        self.services
            .workflow
            .housekeeping
            .run_database_maintenance()
            .await?;

        let report = HousekeepingReport {
            orphaned_media_files,
            stale_release_decisions,
            stale_release_attempts,
            stale_history_events,
            stale_history_records,
            staged_nzb_artifacts_pruned,
            recycled_purged,
            recycled_pending_reconciled,
            discovery_pruned_runs,
            ran_at: self.runtime.environment.now().to_rfc3339(),
        };

        info!(
            orphaned_media_files,
            stale_release_decisions,
            stale_release_attempts,
            stale_history_events,
            stale_operational_domain_events,
            stale_domain_events,
            stale_download_import_artifacts,
            stale_import_history,
            stale_download_queue_deletes,
            stale_rule_set_history,
            stale_history_records,
            staged_nzb_artifacts_pruned,
            recycled_purged,
            recycled_pending_reconciled,
            discovery_pruned_runs,
            "housekeeping completed"
        );

        Ok(report)
    }

    /// List all items across all recycle bins, sorted newest first.
    pub async fn list_recycled_items(
        &self,
        actor: &scryer_domain::User,
        library_ids: Option<Vec<String>>,
    ) -> AppResult<Vec<RecycledItem>> {
        self.require_recycle_bin_page_access(actor).await?;
        let selected_library_ids = self
            .selected_recycle_library_ids(actor, library_ids)
            .await?;
        if selected_library_ids.is_empty() {
            return Ok(Vec::new());
        }

        let roots = self.recycle_root_libraries().await?;

        let mut all_entries = Vec::new();
        let mut list_tasks = tokio::task::JoinSet::new();
        for (media_root, config) in self.resolve_all_recycle_configs().await {
            list_tasks.spawn(async move {
                let entries = crate::recycle_bin::list_entries(&config, &media_root).await;
                (media_root, entries)
            });
        }

        while let Some(result) = list_tasks.join_next().await {
            match result {
                Ok((_media_root, Ok(entries))) => {
                    all_entries.extend(entries);
                }
                Ok((media_root, Err(e))) => {
                    info!(error = %e, media_root = %media_root, "failed to list recycle entries")
                }
                Err(error) => {
                    info!(error = %error, "recycle entry list task failed")
                }
            }
        }

        let mut title_ids = all_entries
            .iter()
            .filter_map(|entry| entry.manifest.title_id.clone())
            .collect::<Vec<_>>();
        title_ids.sort();
        title_ids.dedup();
        let titles_by_id = self.recycle_entry_titles_by_id(title_ids).await?;

        let mut items = Vec::new();
        for entry in all_entries {
            let title = entry
                .manifest
                .title_id
                .as_deref()
                .and_then(|title_id| titles_by_id.get(title_id));
            let Some(library) = resolve_recycle_entry_library_with_title(
                &entry,
                &roots,
                title.map(|(library_id, _)| library_id.as_str()),
            ) else {
                continue;
            };
            if selected_library_ids.contains(&library.id) {
                items.push(recycled_item_from_entry(
                    entry,
                    &library,
                    title.map(|(_, title_name)| title_name.clone()),
                ));
            }
        }

        items.sort_by(|a, b| b.recycled_at.cmp(&a.recycled_at));
        Ok(items)
    }

    async fn recycle_entry_titles_by_id(
        &self,
        mut title_ids: Vec<String>,
    ) -> AppResult<HashMap<String, (String, String)>> {
        const MAX_CONCURRENT_TITLE_LOOKUPS: usize = 16;

        title_ids.sort();
        title_ids.dedup();
        let mut remaining_ids = title_ids.into_iter();
        let mut lookups = tokio::task::JoinSet::new();
        let mut titles_by_id = HashMap::new();

        for _ in 0..MAX_CONCURRENT_TITLE_LOOKUPS {
            let Some(title_id) = remaining_ids.next() else {
                break;
            };
            let titles = self.services.catalog.titles.clone();
            lookups.spawn(async move {
                let title = titles.get_by_id(&title_id).await;
                (title_id, title)
            });
        }

        while let Some(result) = lookups.join_next().await {
            let (title_id, title) = result.map_err(|error| {
                AppError::Repository(format!("recycle-bin title lookup task failed: {error}"))
            })?;
            if let Some(title) = title? {
                titles_by_id.insert(title_id, (title.library_id, title.name));
            }

            if let Some(next_title_id) = remaining_ids.next() {
                let titles = self.services.catalog.titles.clone();
                lookups.spawn(async move {
                    let title = titles.get_by_id(&next_title_id).await;
                    (next_title_id, title)
                });
            }
        }

        Ok(titles_by_id)
    }

    pub async fn start_restore_recycled_item_job(
        &self,
        actor: &scryer_domain::User,
        entry_id: &str,
    ) -> AppResult<RestoreRecycledItemJobAccepted> {
        let guard_key = format!("recycle-entry:{entry_id}");
        let restore_guard = self
            .runtime
            .jobs
            .interactive_operation_guards
            .try_acquire(&guard_key)
            .await
            .ok_or_else(|| {
                AppError::Validation(format!(
                    "a recycle restore is already running for {entry_id}"
                ))
            })?;
        let library_id = self.validate_restore_recycled_item(actor, entry_id).await?;

        let now = chrono::Utc::now();
        let mut run = JobRunRecord {
            id: Id::new().0,
            job_key: JobKey::RecycleBinRestore,
            operation_type: format!("recycle_bin_restore:{library_id}:{entry_id}"),
            status: JobRunStatus::Running,
            trigger_source: JobTriggerSource::Manual,
            actor_user_id: Some(actor.id.clone()),
            progress_json: Some(
                serde_json::json!({
                    "status": JobRunStatus::Running.as_str(),
                    "phase": "queued",
                    "entryId": entry_id,
                })
                .to_string(),
            ),
            summary_json: None,
            summary_text: None,
            error_text: None,
            started_at: now,
            completed_at: None,
            created_at: now,
            updated_at: now,
        };
        run = self.services.events.job_runs.create_job_run(&run).await?;
        let job_run = JobRun::from_record(&run, None);
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(job_run.clone())
            .await;
        let actor_event = DomainEventActor::from(actor);
        let _ = self
            .append_domain_event(crate::domain_events::new_job_run_domain_event(
                actor_event.clone(),
                run.id.clone(),
                scryer_domain::DomainEventPayload::JobRunStarted(
                    scryer_domain::JobRunStartedEventData {
                        run_id: run.id.clone(),
                        job_key: run.job_key.as_str().to_string(),
                        operation_type: run.operation_type.clone(),
                        trigger_source: run.trigger_source.as_str().to_string(),
                    },
                ),
            ))
            .await;

        let app = self.clone();
        let actor = actor.clone();
        let entry_id = entry_id.to_string();
        tokio::spawn(async move {
            app.run_restore_recycled_item_job(run, actor_event, actor, entry_id, restore_guard)
                .await;
        });

        Ok(RestoreRecycledItemJobAccepted { job_run })
    }

    pub async fn preview_restore_recycled_items(
        &self,
        actor: &scryer_domain::User,
        entry_ids: Vec<String>,
    ) -> AppResult<crate::RecycleRestorePreview> {
        let entry_ids = normalize_recycle_batch_ids(entry_ids)?;
        let mut items = Vec::with_capacity(entry_ids.len());
        let mut fingerprint_parts = Vec::with_capacity(entry_ids.len());
        for entry_id in entry_ids {
            let context = self.resolve_restore_recycled_item(actor, &entry_id).await?;
            let original_path = context.manifest.original_path_buf();
            let target_state = restore_target_fingerprint(&original_path).await?;
            fingerprint_parts.push(format!(
                "{}:{}:{}",
                entry_id,
                original_path.display(),
                target_state
            ));
            items.push(crate::RecycleRestorePreviewItem {
                id: entry_id,
                original_path: context.manifest.original_path,
                destination_occupied: target_state != "missing",
            });
        }
        Ok(crate::RecycleRestorePreview {
            fingerprint: fingerprint_parts.join("\n"),
            items,
        })
    }

    pub async fn start_restore_recycled_items_job(
        &self,
        actor: &scryer_domain::User,
        entry_ids: Vec<String>,
        conflict_policy: crate::RecycleRestoreConflictPolicy,
        preview_fingerprint: &str,
    ) -> AppResult<crate::RecycleBinBatchJobAccepted> {
        let entry_ids = normalize_recycle_batch_ids(entry_ids)?;
        let mut guards = self.acquire_recycle_entry_guards(&entry_ids).await?;

        let mut fingerprint_parts = Vec::with_capacity(entry_ids.len());
        let mut destination_keys = HashSet::new();
        let mut expected_target_states = HashMap::with_capacity(entry_ids.len());
        let mut library_ids = HashSet::new();
        for entry_id in &entry_ids {
            let context = self.resolve_restore_recycled_item(actor, entry_id).await?;
            let original_path = context.manifest.original_path_buf();
            let target_state = restore_target_fingerprint(&original_path).await?;
            if conflict_policy == crate::RecycleRestoreConflictPolicy::ReplaceExisting
                && target_state != "missing"
            {
                validate_replace_existing_restore_target(&original_path, &context.recycle_config)
                    .await?;
            }
            fingerprint_parts.push(format!(
                "{}:{}:{}",
                entry_id,
                original_path.display(),
                target_state
            ));
            if !destination_keys.insert(format!("recycle-destination:{}", original_path.display()))
            {
                return Err(AppError::Validation(
                    "selected recycle entries cannot share the same restore destination"
                        .to_string(),
                ));
            }
            expected_target_states.insert(entry_id.clone(), target_state);
            library_ids.insert(context.library_id);
        }
        let fingerprint = fingerprint_parts.join("\n");
        if preview_fingerprint != fingerprint {
            return Err(AppError::Validation(
                "recycle restore preview is stale; review the conflict choice again".to_string(),
            ));
        }

        let mut destination_keys = destination_keys.into_iter().collect::<Vec<_>>();
        destination_keys.sort();
        for guard_key in destination_keys {
            let guard = self
                .runtime
                .jobs
                .interactive_operation_guards
                .try_acquire(&guard_key)
                .await
                .ok_or_else(|| {
                    AppError::Validation(
                        "a recycle restore is already running for one of the selected destinations"
                            .to_string(),
                    )
                })?;
            guards.push(guard);
        }

        for entry_id in &entry_ids {
            let context = self.resolve_restore_recycled_item(actor, entry_id).await?;
            let current_state =
                restore_target_fingerprint(&context.manifest.original_path_buf()).await?;
            if expected_target_states.get(entry_id) != Some(&current_state) {
                return Err(AppError::Validation(
                    "recycle restore preview is stale; review the conflict choice again"
                        .to_string(),
                ));
            }
        }

        let mut library_ids = library_ids.into_iter().collect::<Vec<_>>();
        library_ids.sort();
        let (run, job_run, actor_event) = self
            .create_recycle_batch_job_run(
                actor,
                JobKey::RecycleBinRestore,
                format!(
                    "recycle_bin_restore_batch:{}:{}",
                    library_ids.join(","),
                    entry_ids.len()
                ),
                &entry_ids,
            )
            .await?;
        let app = self.clone();
        let actor = actor.clone();
        let job_entry_ids = entry_ids.clone();
        tokio::spawn(async move {
            app.run_restore_recycled_items_job(
                run,
                actor_event,
                actor,
                job_entry_ids,
                conflict_policy,
                expected_target_states,
                guards,
            )
            .await;
        });

        Ok(crate::RecycleBinBatchJobAccepted { entry_ids, job_run })
    }

    pub async fn start_purge_recycled_items_job(
        &self,
        actor: &scryer_domain::User,
        entry_ids: Vec<String>,
    ) -> AppResult<crate::RecycleBinBatchJobAccepted> {
        let entry_ids = normalize_recycle_batch_ids(entry_ids)?;
        let guards = self.acquire_recycle_entry_guards(&entry_ids).await?;
        let available = self.list_recycled_items(actor, None).await?;
        let available_by_id = available
            .into_iter()
            .map(|item| (item.id, item.library_id))
            .collect::<HashMap<_, _>>();
        let mut library_ids = HashSet::new();
        for entry_id in &entry_ids {
            let library_id = available_by_id
                .get(entry_id)
                .ok_or_else(|| AppError::NotFound(format!("recycle entry {entry_id}")))?;
            library_ids.insert(library_id.clone());
        }

        let mut library_ids = library_ids.into_iter().collect::<Vec<_>>();
        library_ids.sort();
        let (run, job_run, actor_event) = self
            .create_recycle_batch_job_run(
                actor,
                JobKey::RecycleBinPurge,
                format!(
                    "recycle_bin_purge_batch:{}:{}",
                    library_ids.join(","),
                    entry_ids.len()
                ),
                &entry_ids,
            )
            .await?;
        let app = self.clone();
        let actor = actor.clone();
        let job_entry_ids = entry_ids.clone();
        tokio::spawn(async move {
            app.run_purge_recycled_items_job(run, actor_event, actor, job_entry_ids, guards)
                .await;
        });

        Ok(crate::RecycleBinBatchJobAccepted { entry_ids, job_run })
    }

    async fn acquire_recycle_entry_guards(
        &self,
        entry_ids: &[String],
    ) -> AppResult<Vec<tokio::sync::OwnedMutexGuard<()>>> {
        let mut guards = Vec::with_capacity(entry_ids.len());
        for entry_id in entry_ids {
            let guard_key = format!("recycle-entry:{entry_id}");
            let guard = self
                .runtime
                .jobs
                .interactive_operation_guards
                .try_acquire(&guard_key)
                .await
                .ok_or_else(|| {
                    AppError::Validation(format!(
                        "a recycle operation is already running for {entry_id}"
                    ))
                })?;
            guards.push(guard);
        }
        Ok(guards)
    }

    async fn create_recycle_batch_job_run(
        &self,
        actor: &scryer_domain::User,
        job_key: JobKey,
        operation_type: String,
        entry_ids: &[String],
    ) -> AppResult<(JobRunRecord, JobRun, DomainEventActor)> {
        let now = chrono::Utc::now();
        let mut run = JobRunRecord {
            id: Id::new().0,
            job_key,
            operation_type,
            status: JobRunStatus::Running,
            trigger_source: JobTriggerSource::Manual,
            actor_user_id: Some(actor.id.clone()),
            progress_json: Some(
                serde_json::json!({
                    "status": JobRunStatus::Running.as_str(),
                    "phase": "queued",
                    "total": entry_ids.len(),
                    "completed": 0,
                    "entryIds": entry_ids,
                })
                .to_string(),
            ),
            summary_json: None,
            summary_text: None,
            error_text: None,
            started_at: now,
            completed_at: None,
            created_at: now,
            updated_at: now,
        };
        run = self.services.events.job_runs.create_job_run(&run).await?;
        let job_run = JobRun::from_record(&run, None);
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(job_run.clone())
            .await;
        let actor_event = DomainEventActor::from(actor);
        let _ = self
            .append_domain_event(crate::domain_events::new_job_run_domain_event(
                actor_event.clone(),
                run.id.clone(),
                scryer_domain::DomainEventPayload::JobRunStarted(
                    scryer_domain::JobRunStartedEventData {
                        run_id: run.id.clone(),
                        job_key: run.job_key.as_str().to_string(),
                        operation_type: run.operation_type.clone(),
                        trigger_source: run.trigger_source.as_str().to_string(),
                    },
                ),
            ))
            .await;
        Ok((run, job_run, actor_event))
    }

    /// Restore a single recycled item back to its original path.
    pub async fn restore_recycled_item(
        &self,
        actor: &scryer_domain::User,
        entry_id: &str,
    ) -> AppResult<bool> {
        let context = self.resolve_restore_recycled_item(actor, entry_id).await?;
        self.restore_recycled_item_from_context(
            actor,
            context,
            crate::RecycleRestoreConflictPolicy::KeepBoth,
        )
        .await
    }

    async fn validate_restore_recycled_item(
        &self,
        actor: &scryer_domain::User,
        entry_id: &str,
    ) -> AppResult<String> {
        self.resolve_restore_recycled_item(actor, entry_id)
            .await
            .map(|context| context.library_id)
    }

    async fn resolve_restore_recycled_item(
        &self,
        actor: &scryer_domain::User,
        entry_id: &str,
    ) -> AppResult<RestoreRecycledItemContext> {
        let roots = self.recycle_root_libraries().await?;

        for (media_root, config) in self.resolve_all_recycle_configs().await {
            if let Some((entry_dir, manifest)) =
                crate::recycle_bin::find_entry(&config, entry_id).await?
            {
                let entry = crate::recycle_bin::RecycleEntry {
                    entry_id: entry_id.to_string(),
                    manifest: manifest.clone(),
                    media_root,
                };
                let library = self
                    .resolve_recycle_entry_library(&entry, &roots)
                    .await?
                    .ok_or_else(|| {
                        AppError::Unauthorized("You do not have access to this library".to_string())
                    })?;
                self.require_granted_library_permission(
                    actor,
                    &library.id,
                    scryer_domain::LibraryPermission::ManageTitles,
                )
                .await?;
                let original_path = manifest.original_path_buf();
                let library_roots = recycle_library_root_paths(&library, &roots);
                if !recycle_restore_destination_is_under_library_roots(
                    &original_path,
                    &library_roots,
                ) {
                    return Err(AppError::Validation(format!(
                        "refusing to restore recycle entry {} because original path is outside the resolved library roots: {}",
                        entry_id, manifest.original_path
                    )));
                }

                let file_name = original_path
                    .file_name()
                    .unwrap_or_else(|| std::ffi::OsStr::new("unknown"));
                let recycled_file = entry_dir.join(file_name);
                crate::recycle_bin::ensure_recycled_restore_source_is_regular(&recycled_file)
                    .await?;

                return Ok(RestoreRecycledItemContext {
                    entry_dir,
                    manifest,
                    library_id: library.id,
                    library_roots,
                    recycle_config: config,
                });
            }
        }

        Err(AppError::NotFound(format!("recycle entry {}", entry_id)))
    }

    async fn restore_recycled_item_from_context(
        &self,
        actor: &scryer_domain::User,
        context: RestoreRecycledItemContext,
        conflict_policy: crate::RecycleRestoreConflictPolicy,
    ) -> AppResult<bool> {
        let original_path = context.manifest.original_path_buf();
        let file_name = original_path
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new("unknown"));
        let recycled_file = context.entry_dir.join(file_name);

        let restored_to = match conflict_policy {
            crate::RecycleRestoreConflictPolicy::KeepBoth => {
                crate::recycle_bin::restore_from_recycle_with_roots(
                    &recycled_file,
                    &original_path,
                    false,
                    &context.library_roots,
                )
                .await?
            }
            crate::RecycleRestoreConflictPolicy::ReplaceExisting => {
                self.restore_recycled_item_replacing_existing(
                    &context,
                    &recycled_file,
                    &original_path,
                )
                .await?
            }
        };
        if let Err(error) =
            crate::fs_safety::remove_dir_all_safely_if_exists(&context.entry_dir).await
        {
            tracing::warn!(
                error = %error,
                entry_dir = %context.entry_dir.display(),
                restored_to = %restored_to.display(),
                "failed to remove recycle entry directory after restore"
            );
        }
        if let Some(title_id) = context.manifest.title_id.as_deref() {
            let restored_library_file = crate::LibraryFile {
                path: restored_to.to_string_lossy().to_string(),
                display_name: original_path
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default()
                    .to_string(),
                nfo_path: None,
                size_bytes: tokio::fs::metadata(&restored_to)
                    .await
                    .ok()
                    .and_then(|metadata| i64::try_from(metadata.len()).ok()),
                source_signature_scheme: None,
                source_signature_value: None,
            };
            match self.services.catalog.titles.get_by_id(title_id).await {
                Ok(Some(title)) => {
                    if let Err(error) = self
                        .scan_title_library_with_discovered_files(
                            actor,
                            title,
                            vec![restored_library_file],
                        )
                        .await
                    {
                        tracing::warn!(
                            error = %error,
                            title_id,
                            restored_to = %restored_to.display(),
                            "failed to scan title after restoring recycled file"
                        );
                    }
                }
                Ok(None) => {
                    tracing::warn!(
                        title_id,
                        restored_to = %restored_to.display(),
                        "skipping restored file scan because the title no longer exists"
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        title_id,
                        restored_to = %restored_to.display(),
                        "failed to load title before restored file scan"
                    );
                }
            }
        }
        Ok(true)
    }

    async fn restore_recycled_item_replacing_existing(
        &self,
        context: &RestoreRecycledItemContext,
        recycled_file: &Path,
        original_path: &Path,
    ) -> AppResult<PathBuf> {
        let occupant_metadata = match tokio::fs::symlink_metadata(original_path).await {
            Ok(metadata) => Some(metadata),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(AppError::Repository(format!(
                    "failed to inspect restore destination {}: {}",
                    original_path.display(),
                    error
                )));
            }
        };

        if occupant_metadata.is_none() {
            crate::recycle_bin::restore_recycled_file_exact(recycled_file, original_path).await?;
            return Ok(original_path.to_path_buf());
        }

        if !context.recycle_config.enabled || !context.recycle_config.cleanup_enabled {
            return Err(AppError::Validation(format!(
                "refusing to replace {} because the recycle bin is unavailable",
                original_path.display()
            )));
        }

        let original_path_text = original_path.to_string_lossy().to_string();
        let incumbent = self
            .services
            .library
            .media_files
            .get_media_file_by_path(&original_path_text)
            .await?;
        let metadata = occupant_metadata.expect("checked occupied restore destination");
        let displaced_manifest = crate::recycle_bin::RecycleManifest {
            schema: None,
            entry_id: None,
            source_operation_id: None,
            recycled_at: chrono::Utc::now().to_rfc3339(),
            original_path: original_path_text,
            original_file_id: incumbent.as_ref().map(|file| file.id.clone()),
            size_bytes: metadata.len(),
            title_id: incumbent
                .as_ref()
                .map(|file| file.title_id.clone())
                .or_else(|| context.manifest.title_id.clone()),
            media_root: context.manifest.media_root.clone(),
            reason: "restore_replaced".to_string(),
            status: None,
            replacement_file_id: None,
            replacement_path: None,
        };
        let displaced = crate::recycle_bin::recycle_file_pending(
            &context.recycle_config,
            original_path,
            displaced_manifest,
        )
        .await?
        .ok_or_else(|| {
            AppError::Repository(format!(
                "restore destination {} disappeared before it could be recycled",
                original_path.display()
            ))
        })?;

        if let Err(error) = crate::recycle_bin::link_recycled_file_to_exact_destination(
            recycled_file,
            original_path,
        )
        .await
        {
            match crate::recycle_bin::restore_recycled_file_exact(
                &displaced.recycled_path,
                original_path,
            )
            .await
            {
                Ok(()) => {
                    let _ = crate::fs_safety::remove_dir_all_safely_if_exists(&displaced.entry_dir)
                        .await;
                }
                Err(rollback_error) => {
                    return Err(AppError::Repository(format!(
                        "failed to restore recycled file after moving the current file aside: {error}; rollback also failed: {rollback_error}"
                    )));
                }
            }
            return Err(error);
        }

        if let Some(incumbent) = incumbent {
            if let Err(error) = self
                .delete_media_file_record_with_dependents(&incumbent.id)
                .await
            {
                if let Err(remove_error) = crate::fs_safety::remove_file_safely(original_path).await
                {
                    return Err(AppError::Repository(format!(
                        "failed to clean up the replaced media record after restoring {}: {error}; the restored file could not be removed for rollback: {remove_error}",
                        original_path.display()
                    )));
                }

                match crate::recycle_bin::restore_recycled_file_exact(
                    &displaced.recycled_path,
                    original_path,
                )
                .await
                {
                    Ok(()) => {
                        let _ =
                            crate::fs_safety::remove_dir_all_safely_if_exists(&displaced.entry_dir)
                                .await;
                    }
                    Err(rollback_error) => {
                        return Err(AppError::Repository(format!(
                            "failed to clean up the replaced media record after restoring {}: {error}; rollback also failed: {rollback_error}",
                            original_path.display()
                        )));
                    }
                }

                return Err(error);
            }
        }

        if let Err(error) = crate::fs_safety::remove_file_safely(recycled_file).await {
            tracing::warn!(
                error = %error,
                recycled_file = %recycled_file.display(),
                original_path = %original_path.display(),
                "restored file is linked in place but its recycle source could not be removed"
            );
        }
        if let Err(error) =
            crate::recycle_bin::commit_recycle_entry_without_replacement(&displaced).await
        {
            tracing::warn!(
                error = %error,
                entry_id = %displaced.entry_id,
                "replacement recycle entry remains pending after a completed restore"
            );
        }
        Ok(original_path.to_path_buf())
    }

    async fn run_restore_recycled_items_job(
        &self,
        mut run: JobRunRecord,
        actor_event: DomainEventActor,
        actor: scryer_domain::User,
        entry_ids: Vec<String>,
        conflict_policy: crate::RecycleRestoreConflictPolicy,
        expected_target_states: HashMap<String, String>,
        _guards: Vec<tokio::sync::OwnedMutexGuard<()>>,
    ) {
        let total = entry_ids.len();
        let mut results = Vec::with_capacity(total);
        let mut succeeded = 0usize;
        for (index, entry_id) in entry_ids.iter().enumerate() {
            let result = async {
                let context = self.resolve_restore_recycled_item(&actor, entry_id).await?;
                let current_state =
                    restore_target_fingerprint(&context.manifest.original_path_buf()).await?;
                if expected_target_states.get(entry_id) != Some(&current_state) {
                    return Err(AppError::Validation(
                        "restore destination changed after confirmation; preview and retry this item"
                            .to_string(),
                    ));
                }
                self.restore_recycled_item_from_context(&actor, context, conflict_policy)
                    .await
            }
            .await;
            match result {
                Ok(_) => {
                    succeeded += 1;
                    results.push(serde_json::json!({ "id": entry_id, "status": "completed" }));
                }
                Err(error) => results.push(serde_json::json!({
                    "id": entry_id,
                    "status": "failed",
                    "error": error.to_string(),
                })),
            }
            if let Err(error) = self
                .update_recycle_batch_progress(&mut run, index + 1, total, succeeded, &results)
                .await
            {
                warn!(error = %error, run_id = %run.id, "failed to update recycle restore batch progress");
            }
        }
        if let Err(error) = self
            .finish_recycle_batch_job(run, actor_event, "restore", total, succeeded, results)
            .await
        {
            warn!(error = %error, "failed to finish recycle restore batch job");
        }
    }

    async fn run_purge_recycled_items_job(
        &self,
        mut run: JobRunRecord,
        actor_event: DomainEventActor,
        actor: scryer_domain::User,
        entry_ids: Vec<String>,
        _guards: Vec<tokio::sync::OwnedMutexGuard<()>>,
    ) {
        let total = entry_ids.len();
        let mut results = Vec::with_capacity(total);
        let mut succeeded = 0usize;
        for (index, entry_id) in entry_ids.iter().enumerate() {
            match self.delete_recycled_item(&actor, entry_id).await {
                Ok(true) => {
                    succeeded += 1;
                    results.push(serde_json::json!({ "id": entry_id, "status": "completed" }));
                }
                Ok(false) => results.push(serde_json::json!({
                    "id": entry_id,
                    "status": "quarantined",
                    "error": "the recycle entry could not be safely purged",
                })),
                Err(error) => results.push(serde_json::json!({
                    "id": entry_id,
                    "status": "failed",
                    "error": error.to_string(),
                })),
            }
            if let Err(error) = self
                .update_recycle_batch_progress(&mut run, index + 1, total, succeeded, &results)
                .await
            {
                warn!(error = %error, run_id = %run.id, "failed to update recycle purge batch progress");
            }
        }
        if let Err(error) = self
            .finish_recycle_batch_job(run, actor_event, "purge", total, succeeded, results)
            .await
        {
            warn!(error = %error, "failed to finish recycle purge batch job");
        }
    }

    async fn update_recycle_batch_progress(
        &self,
        run: &mut JobRunRecord,
        completed: usize,
        total: usize,
        succeeded: usize,
        results: &[serde_json::Value],
    ) -> AppResult<()> {
        run.progress_json = Some(
            serde_json::json!({
                "status": JobRunStatus::Running.as_str(),
                "phase": "running",
                "completed": completed,
                "total": total,
                "succeeded": succeeded,
                "failed": completed.saturating_sub(succeeded),
                "items": results,
            })
            .to_string(),
        );
        run.updated_at = chrono::Utc::now();
        let updated = self.services.events.job_runs.update_job_run(run).await?;
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(JobRun::from_record(&updated, None))
            .await;
        *run = updated;
        Ok(())
    }

    async fn finish_recycle_batch_job(
        &self,
        mut run: JobRunRecord,
        actor: DomainEventActor,
        action: &str,
        total: usize,
        succeeded: usize,
        results: Vec<serde_json::Value>,
    ) -> AppResult<()> {
        let failed = total.saturating_sub(succeeded);
        let status = if failed == 0 {
            JobRunStatus::Completed
        } else if succeeded == 0 {
            JobRunStatus::Failed
        } else {
            JobRunStatus::Warning
        };
        let completed_at = chrono::Utc::now();
        run.status = status;
        run.progress_json = Some(
            serde_json::json!({
                "status": status.as_str(),
                "phase": "completed",
                "completed": total,
                "total": total,
                "succeeded": succeeded,
                "failed": failed,
                "items": results,
            })
            .to_string(),
        );
        run.summary_text = Some(format!(
            "Recycle-bin {action}: {succeeded} of {total} item{} completed",
            if total == 1 { "" } else { "s" }
        ));
        run.summary_json = Some(
            serde_json::json!({
                "action": action,
                "total": total,
                "succeeded": succeeded,
                "failed": failed,
                "items": results,
            })
            .to_string(),
        );
        run.error_text = (failed > 0).then(|| format!("{failed} recycle-bin item(s) failed"));
        run.completed_at = Some(completed_at);
        run.updated_at = completed_at;
        let updated = self.services.events.job_runs.update_job_run(&run).await?;
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(JobRun::from_record(&updated, None))
            .await;
        let payload = if status == JobRunStatus::Failed {
            scryer_domain::DomainEventPayload::JobRunFailed(scryer_domain::JobRunFailedEventData {
                run_id: updated.id.clone(),
                job_key: updated.job_key.as_str().to_string(),
                error_text: updated.error_text.clone(),
            })
        } else {
            scryer_domain::DomainEventPayload::JobRunCompleted(
                scryer_domain::JobRunCompletedEventData {
                    run_id: updated.id.clone(),
                    job_key: updated.job_key.as_str().to_string(),
                    summary_text: updated.summary_text.clone(),
                },
            )
        };
        let _ = self
            .append_domain_event(crate::domain_events::new_job_run_domain_event(
                actor,
                updated.id.clone(),
                payload,
            ))
            .await;
        Ok(())
    }

    async fn run_restore_recycled_item_job(
        &self,
        run: JobRunRecord,
        actor_event: DomainEventActor,
        actor: scryer_domain::User,
        entry_id: String,
        _restore_guard: tokio::sync::OwnedMutexGuard<()>,
    ) {
        let result = self.restore_recycled_item(&actor, &entry_id).await;
        let (status, summary_text, error_text) = match result {
            Ok(_) => (
                JobRunStatus::Completed,
                "Restored recycled item".to_string(),
                None,
            ),
            Err(error) => (
                JobRunStatus::Failed,
                "Failed to restore recycled item".to_string(),
                Some(error.to_string()),
            ),
        };
        if let Err(error) = self
            .finish_restore_recycled_item_job(
                run,
                actor_event,
                status,
                &entry_id,
                summary_text,
                error_text,
            )
            .await
        {
            warn!(error = %error, entry_id = %entry_id, "failed to finish recycle restore job");
        }
    }

    async fn finish_restore_recycled_item_job(
        &self,
        mut run: JobRunRecord,
        actor: DomainEventActor,
        status: JobRunStatus,
        entry_id: &str,
        summary_text: String,
        error_text: Option<String>,
    ) -> AppResult<()> {
        let completed_at = chrono::Utc::now();
        run.status = status;
        run.progress_json = Some(
            serde_json::json!({
                "status": status.as_str(),
                "phase": "completed",
                "entryId": entry_id,
            })
            .to_string(),
        );
        run.summary_text = Some(summary_text);
        run.summary_json = Some(serde_json::json!({ "entryId": entry_id }).to_string());
        run.error_text = error_text;
        run.completed_at = Some(completed_at);
        run.updated_at = completed_at;
        let updated = self.services.events.job_runs.update_job_run(&run).await?;
        self.runtime
            .jobs
            .job_run_tracker
            .upsert_active_run(JobRun::from_record(&updated, None))
            .await;
        let payload = if status == JobRunStatus::Failed {
            scryer_domain::DomainEventPayload::JobRunFailed(scryer_domain::JobRunFailedEventData {
                run_id: updated.id.clone(),
                job_key: updated.job_key.as_str().to_string(),
                error_text: updated.error_text.clone(),
            })
        } else {
            scryer_domain::DomainEventPayload::JobRunCompleted(
                scryer_domain::JobRunCompletedEventData {
                    run_id: updated.id.clone(),
                    job_key: updated.job_key.as_str().to_string(),
                    summary_text: updated.summary_text.clone(),
                },
            )
        };
        let _ = self
            .append_domain_event(crate::domain_events::new_job_run_domain_event(
                actor,
                updated.id.clone(),
                payload,
            ))
            .await;
        Ok(())
    }

    /// Permanently delete a single recycled item.
    pub async fn delete_recycled_item(
        &self,
        actor: &scryer_domain::User,
        entry_id: &str,
    ) -> AppResult<bool> {
        let roots = self.recycle_root_libraries().await?;

        for (media_root, config) in self.resolve_all_recycle_configs().await {
            if let Some(committed_entry) =
                crate::recycle_bin::find_committed_entry(&config, entry_id).await?
            {
                let entry = crate::recycle_bin::RecycleEntry {
                    entry_id: entry_id.to_string(),
                    manifest: committed_entry.manifest.clone(),
                    media_root: media_root.clone(),
                };
                let library = self
                    .resolve_recycle_entry_library(&entry, &roots)
                    .await?
                    .ok_or_else(|| {
                        AppError::Unauthorized("You do not have access to this library".to_string())
                    })?;
                self.require_granted_library_permission(
                    actor,
                    &library.id,
                    scryer_domain::LibraryPermission::ManageTitles,
                )
                .await?;

                return self
                    .purge_recycle_entry_after_validation(
                        &media_root,
                        &config,
                        &committed_entry,
                        actor,
                    )
                    .await;
            }
        }

        Err(AppError::NotFound(format!("recycle entry {}", entry_id)))
    }

    /// Empty all recycle bins across all media roots.
    pub async fn empty_recycle_bin(
        &self,
        actor: &scryer_domain::User,
        library_ids: Option<Vec<String>>,
    ) -> AppResult<u32> {
        self.require_recycle_bin_page_access(actor).await?;
        let selected_library_ids = self
            .selected_recycle_library_ids(actor, library_ids)
            .await?;
        if selected_library_ids.is_empty() {
            return Ok(0);
        }

        let roots = self.recycle_root_libraries().await?;

        let mut total = 0u32;
        for (media_root, config) in self.resolve_all_recycle_configs().await {
            match crate::recycle_bin::list_committed_entries(&config).await {
                Ok(entries) => {
                    for entry in entries {
                        let entry_id = entry
                            .entry_dir
                            .file_name()
                            .map(|name| name.to_string_lossy().to_string())
                            .unwrap_or_default();
                        let recycle_entry = crate::recycle_bin::RecycleEntry {
                            entry_id,
                            manifest: entry.manifest.clone(),
                            media_root: media_root.clone(),
                        };
                        let Some(library) = self
                            .resolve_recycle_entry_library(&recycle_entry, &roots)
                            .await?
                        else {
                            continue;
                        };
                        if !selected_library_ids.contains(&library.id) {
                            continue;
                        }
                        match self
                            .purge_recycle_entry_after_validation(
                                &media_root,
                                &config,
                                &entry,
                                actor,
                            )
                            .await
                        {
                            Ok(true) => total += 1,
                            Ok(false) => {}
                            Err(error) => warn!(
                                path = %entry.entry_dir.display(),
                                error = %error,
                                "failed to empty recycle entry"
                            ),
                        }
                    }
                }
                Err(e) => {
                    info!(error = %e, media_root = %media_root, "failed to empty recycle bin")
                }
            }
        }
        Ok(total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_library() -> RecycleEntryLibrary {
        RecycleEntryLibrary {
            id: "library-1".to_string(),
            name: "Library".to_string(),
        }
    }

    #[test]
    fn recycle_library_root_paths_use_raw_media_root_for_filesystem_policy() {
        let library = test_library();
        let roots = vec![RecycleRootLibrary {
            media_root: r"/tmp/media\raw".to_string(),
            normalized_media_root: "/tmp/media/raw".to_string(),
            library: library.clone(),
        }];

        let policy_roots = recycle_library_root_paths(&library, &roots);
        assert_eq!(policy_roots, vec![PathBuf::from(r"/tmp/media\raw")]);

        #[cfg(not(windows))]
        assert!(
            !recycle_restore_destination_is_under_library_roots(
                Path::new("/tmp/media/raw/Movie.mkv"),
                &policy_roots
            ),
            "non-Windows restore validation must fail closed for ambiguous raw roots"
        );
    }

    #[test]
    fn recycle_restore_destination_accepts_normal_raw_root() {
        let library = test_library();
        let roots = vec![RecycleRootLibrary {
            media_root: "/tmp/media".to_string(),
            normalized_media_root: "/tmp/media".to_string(),
            library: library.clone(),
        }];

        let policy_roots = recycle_library_root_paths(&library, &roots);
        assert!(
            recycle_restore_destination_is_under_library_roots(
                Path::new("/tmp/media/Movie.mkv"),
                &policy_roots
            ),
            "normal raw roots should still allow file destinations under the root"
        );
    }
}

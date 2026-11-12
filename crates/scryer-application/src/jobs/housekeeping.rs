use super::*;
use crate::domain_events::{
    DomainEventActor, created_media_update, deleted_media_update, new_title_domain_event,
    title_context_snapshot,
};
use crate::events::retention::{
    ACQUISITION_TELEMETRY_RETENTION_DAYS, OPERATIONAL_DOMAIN_EVENT_RETENTION_DAYS,
    acquisition_telemetry_domain_event_types, operational_domain_event_types,
    user_facing_domain_event_types,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

const RELEASE_DECISION_RETENTION_DAYS: i64 = 30;
const WORKFLOW_COMPLETED_RETENTION_DAYS: i64 = 7;
const WORKFLOW_WARNING_FAILED_RETENTION_DAYS: i64 = 30;
const RELEASE_ATTEMPT_RETENTION_DAYS: i64 = 90;
pub const INDEXER_ERROR_RETENTION_DAYS: i64 = 30;
const LIST_SYNC_RUN_RETENTION_DAYS: i64 = 7;
const DOWNLOAD_DELETE_RETENTION_DAYS: i64 = 7;
const DISCOVERY_SUCCESSFUL_GENERATIONS_TO_RETAIN: usize = 1;
const DISCOVERY_DIAGNOSTIC_RETENTION_DAYS: i64 = 30;
const TITLE_IMAGE_BLOB_GC_BATCH_SIZE: u32 = 100;
const TITLE_IMAGE_BLOB_GC_MAX_BATCHES: usize = 10;
const RECYCLE_BIN_BATCH_MAX_ITEMS: usize = 250;

/// How long a deleted title's recycle-cleanup record is kept: the history
/// retention period, or forever when history is kept forever.
pub(crate) fn recycle_purge_record_retention_days(
    settings: &crate::GeneralSettings,
) -> Option<i64> {
    (!settings.keep_history_forever).then_some(i64::from(settings.history_retention_days))
}

fn normalize_recycle_batch_ids(entry_ids: Vec<String>) -> AppResult<Vec<String>> {
    if entry_ids.is_empty() {
        return Err(AppError::validation_refused(
            crate::error_reasons::jobs::RECYCLE_BIN_ITEM_REQUIRED,
            "select at least one recycle-bin item".to_string(),
        ));
    }
    if entry_ids.len() > RECYCLE_BIN_BATCH_MAX_ITEMS {
        return Err(AppError::validation_refused(
            crate::error_reasons::jobs::RECYCLE_BIN_BATCHES_LIMITED_ITEMS,
            format!("recycle-bin batches are limited to {RECYCLE_BIN_BATCH_MAX_ITEMS} items"),
        ));
    }

    let requested_count = entry_ids.len();
    let mut normalized = entry_ids;
    normalized.sort();
    normalized.dedup();
    if normalized.len() != requested_count {
        return Err(AppError::validation_refused(
            crate::error_reasons::jobs::RECYCLE_BIN_BATCHES_CANNOT_INCLUDE_SAME_ITEM_MORE_ONCE,
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
        return Err(AppError::validation_refused(
            crate::error_reasons::jobs::SYMLINK_REFUSED,
            format!(
                "refusing to replace restore destination {} because it is a symlink",
                path.display()
            ),
        ));
    }
    if !metadata.is_file() {
        return Err(AppError::validation_refused(
            crate::error_reasons::jobs::REGULAR_FILE_REQUIRED,
            format!(
                "refusing to replace restore destination {} because it is not a regular file",
                path.display()
            ),
        ));
    }
    if !recycle_config.enabled || !recycle_config.cleanup_enabled {
        return Err(AppError::validation_refused(
            crate::error_reasons::jobs::REPLACE_RECYCLE_BIN_UNAVAILABLE,
            format!(
                "refusing to replace {} because the recycle bin is unavailable",
                path.display()
            ),
        ));
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
    title_facet: Option<scryer_domain::MediaFacet>,
    retention_days: u32,
) -> RecycledItem {
    let original_path = entry.manifest.original_path_buf();
    let file_name = original_path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    let scheduled_deletion_at = chrono::DateTime::parse_from_rfc3339(&entry.manifest.recycled_at)
        .map(|recycled_at| {
            (recycled_at.with_timezone(&chrono::Utc)
                + chrono::Duration::days(i64::from(retention_days)))
            .to_rfc3339()
        })
        .unwrap_or_else(|_| entry.manifest.recycled_at.clone());

    RecycledItem {
        id: entry.entry_id,
        original_path: entry.manifest.original_path,
        file_name,
        size_bytes: entry.manifest.size_bytes,
        title_id: entry.manifest.title_id,
        title_name,
        title_facet,
        episode_id: None,
        original_file_id: entry.manifest.original_file_id,
        replacement_file_id: entry.manifest.replacement_file_id,
        reason: entry.manifest.reason,
        recycled_at: entry.manifest.recycled_at,
        scheduled_deletion_at,
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
        self.resolve_all_recycle_configs_with_libraries()
            .await
            .0
            .into_iter()
            .map(|(media_root, config, _)| (media_root, config))
            .collect()
    }

    /// [`Self::resolve_all_recycle_configs`], with the libraries whose roots
    /// each bin serves: every library for a custom bin, the libraries of its
    /// root otherwise. The flag is false when the roots could not be read.
    async fn resolve_all_recycle_configs_with_libraries(
        &self,
    ) -> (
        Vec<(
            String,
            crate::recycle_bin::RecycleBinConfig,
            HashSet<String>,
        )>,
        bool,
    ) {
        let mut media_roots = Vec::new();
        let mut seen_roots = HashSet::new();
        let mut libraries_by_root = HashMap::<String, HashSet<String>>::new();
        let mut all_libraries = HashSet::new();

        let resolved = match self.recycle_root_libraries().await {
            Ok(roots) => {
                for root in roots {
                    all_libraries.insert(root.library.id.clone());
                    libraries_by_root
                        .entry(root.normalized_media_root.clone())
                        .or_default()
                        .insert(root.library.id);
                    if seen_roots.insert(root.normalized_media_root) {
                        media_roots.push(root.media_root);
                    }
                }
                true
            }
            Err(error) => {
                warn!(
                    error = %error,
                    "failed to resolve library roots for recycle bin housekeeping"
                );
                false
            }
        };

        let configs = self
            .recycle_bin_configs_for_media_roots(media_roots)
            .await
            .into_iter()
            .map(|(media_root, config)| {
                let libraries = if media_root.is_empty() {
                    all_libraries.clone()
                } else {
                    libraries_by_root
                        .get(&crate::catalog_workflow::normalize_library_root_path(
                            &media_root,
                        ))
                        .cloned()
                        .unwrap_or_default()
                };
                (media_root, config, libraries)
            })
            .collect();
        (configs, resolved)
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

/// The names in the recycle bin directory that are among `wanted`, from one
/// read of the directory. Only names are read: no entry is opened or probed.
/// A failed read of any part of the directory fails the whole bin, as the
/// listing it stands in for does.
async fn selected_names_in_recycle_bin(
    base_path: &std::path::Path,
    wanted: &HashSet<&str>,
) -> AppResult<Vec<String>> {
    let mut entries = tokio::fs::read_dir(base_path).await.map_err(|error| {
        AppError::Repository(format!(
            "failed to read recycle bin directory {}: {error}",
            base_path.display()
        ))
    })?;
    let mut names = Vec::new();
    while let Some(entry) = entries.next_entry().await.map_err(|error| {
        AppError::Repository(format!("failed to read recycle bin entry: {error}"))
    })? {
        if let Some(name) = entry.file_name().to_str()
            && wanted.contains(name)
        {
            names.push(name.to_string());
        }
    }
    Ok(names)
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
                .has_any_library_permission(actor, scryer_domain::LibraryPermission::ManageTitles)
                .await?
        {
            return Ok(());
        }

        Err(AppError::unauthorized_reason(
            crate::error_reasons::jobs::VIEW_RECYCLE_BIN_PERMISSION_DENIED,
            "You do not have permission to view the recycle bin".to_string(),
        ))
    }

    async fn selected_recycle_library_ids(
        &self,
        actor: &User,
        library_ids: Option<Vec<String>>,
    ) -> AppResult<HashSet<String>> {
        let allowed = self
            .authorized_library_ids(actor, None, scryer_domain::LibraryPermission::ManageTitles)
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

    /// Purge the expired entries of one bin, and every entry of a recorded
    /// deleted title whose library the bin serves. The bin is listed once.
    async fn purge_expired_recycle_entries(
        &self,
        media_root: &str,
        config: &crate::recycle_bin::RecycleBinConfig,
        bin_libraries: &HashSet<String>,
        recorded: &mut crate::title_delete::RecordedTitleRecyclePurges,
    ) -> AppResult<u32> {
        if config.validation_error.is_some() {
            recorded.keep_libraries(bin_libraries);
        }
        let cutoff = crate::recycle_bin::expiry_cutoff(config);
        let (entries, withheld) =
            crate::recycle_bin::list_committed_entries_reporting_withheld(config).await?;
        for title_id in &withheld {
            if recorded.claims(title_id, bin_libraries) {
                recorded.keep(title_id);
            }
        }
        let mut purged = 0u32;
        for entry in entries {
            if let Some(title_id) = entry
                .manifest
                .title_id
                .clone()
                .filter(|title_id| recorded.claims(title_id, bin_libraries))
            {
                let mut outcome = crate::title_delete::RecyclePurgeOutcome::default();
                self.purge_listed_title_recycle_entry(
                    config,
                    &title_id,
                    entry,
                    DomainEventActor::system(),
                    &mut outcome,
                )
                .await;
                purged += outcome.purged as u32;
                if outcome.incomplete() {
                    recorded.keep(&title_id);
                }
                continue;
            }
            if !crate::recycle_bin::recycled_before(&entry, cutoff) {
                continue;
            }
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

    /// [`Self::purge_recycle_bins_with_record_retention`] with the history
    /// retention the general settings give, as the scheduled pass runs it.
    #[cfg(test)]
    pub(crate) async fn purge_recycle_bins_for_housekeeping(&self) -> u32 {
        let settings = self
            .general_settings()
            .await
            .expect("general settings should load");
        self.purge_recycle_bins_with_record_retention(recycle_purge_record_retention_days(
            &settings,
        ))
        .await
    }

    /// The housekeeping recycle pass: each bin is listed once, its expired
    /// entries are purged, and so is every entry of a deleted title recorded
    /// for cleanup. First, records older than `record_retention_days` are
    /// dropped and their entries left to the bin's own retention; with `None`
    /// (history kept forever) no record is dropped that way. Returns how many
    /// entries were purged.
    pub(crate) async fn purge_recycle_bins_with_record_retention(
        &self,
        record_retention_days: Option<i64>,
    ) -> u32 {
        if let Some(days) = record_retention_days {
            self.drop_expired_title_recycle_purges(days).await;
        }
        let mut recycled_purged = 0u32;
        let mut recorded = self.load_recorded_title_recycle_purges().await;
        let (recycle_bins, bins_resolved) = self.resolve_all_recycle_configs_with_libraries().await;
        recorded.set_bins_resolved(bins_resolved);
        for (media_root, config, bin_libraries) in recycle_bins {
            match self
                .purge_expired_recycle_entries(&media_root, &config, &bin_libraries, &mut recorded)
                .await
            {
                Ok(n) => recycled_purged += n,
                Err(e) => {
                    recorded.keep_libraries(&bin_libraries);
                    info!(error = %e, media_root = %media_root, "recycle bin purge failed")
                }
            }
        }
        if !recorded.is_empty() {
            self.finish_recorded_title_recycle_purges(recorded).await;
        }
        recycled_purged
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

    pub(crate) async fn record_recycle_entry_purged_event(
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
                    reason: Self::purged_recycle_reason(manifest),
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

    /// Why a recycled entry was purged, preserving the reason it was recycled.
    ///
    /// A copy recycled because an upgrade replaced it is still an upgrade
    /// cleanup as far as subscribers are concerned: "File Deleted for Upgrade"
    /// is the toggle that describes it, and a subscriber to plain deletions did
    /// not ask about it. Discarding that origin here is what made every purge
    /// arrive as an ordinary deletion.
    ///
    /// Quarantined entries carry their original reason with a `; quarantine: …`
    /// suffix, so compare against the reason itself rather than the whole
    /// string.
    fn purged_recycle_reason(
        manifest: &crate::recycle_bin::RecycleManifest,
    ) -> scryer_domain::MediaFileDeletedReason {
        let reason = manifest.reason.split(';').next().unwrap_or_default().trim();
        if reason == "upgrade_replaced" {
            scryer_domain::MediaFileDeletedReason::RecycleBinPurgedAfterUpgrade
        } else {
            scryer_domain::MediaFileDeletedReason::RecycleBinPurged
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
        let (orphaned_media_files, promotions) = {
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
            let mut promotions = crate::catalog::workflow::PrimaryPromotionBatch::default();
            let mut unprobed = all_files
                .into_iter()
                .filter(|media_file| {
                    !protected_upgrade_file_ids.contains(&media_file.media_file_id)
                })
                .collect::<std::collections::VecDeque<_>>();
            let mut root_checks = OrphanSweepRootChecks::new();
            while !unprobed.is_empty() {
                // The probes are blocking filesystem calls, so they run off the
                // runtime. Each blocking pass stops at the first orphan it
                // finds, so every row is still removed straight after its own
                // file probe. A root's availability is proven once per window
                // of rows and re-proven when the next window starts.
                let (rest, checks, orphan) = tokio::task::spawn_blocking(move || {
                    let orphan = next_orphan_media_file(
                        &mut unprobed,
                        ORPHAN_SWEEP_PROBES_PER_PASS,
                        &mut root_checks,
                    );
                    (unprobed, root_checks, orphan)
                })
                .await
                .map_err(|error| {
                    AppError::Repository(format!("orphan media-file probe task failed: {error}"))
                })?;
                unprobed = rest;
                root_checks = checks;
                let Some(media_file) = orphan else {
                    continue;
                };

                self.record_primary_promotion_candidate(
                    &mut promotions,
                    &media_file.title_id,
                    &media_file.media_file_id,
                )
                .await;
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
            (orphaned_media_files, promotions)
        };
        self.finish_primary_promotion_batch(&DomainEventActor::system(), promotions)
            .await;

        let general_settings = self.general_settings().await?;
        let history_retention_days = general_settings.history_retention_days as i64;
        let user_facing_domain_event_types = user_facing_domain_event_types();
        let operational_domain_event_types = operational_domain_event_types();

        let stale_workflow_operations = self
            .services
            .workflow
            .housekeeping
            .delete_stale_workflow_operations(
                WORKFLOW_COMPLETED_RETENTION_DAYS,
                WORKFLOW_WARNING_FAILED_RETENTION_DAYS,
            )
            .await?;
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
        let stale_indexer_errors = self
            .services
            .integrations
            .indexer_errors
            .delete_older_than(
                self.runtime.environment.now()
                    - chrono::Duration::days(INDEXER_ERROR_RETENTION_DAYS),
            )
            .await?;
        let stale_list_sync_runs = self
            .services
            .lists
            .subscriptions
            .delete_sync_runs_older_than(
                self.runtime.environment.now()
                    - chrono::Duration::days(LIST_SYNC_RUN_RETENTION_DAYS),
            )
            .await?;

        let (
            stale_history_events,
            stale_domain_events,
            stale_download_import_artifacts,
            stale_import_history,
            stale_download_queue_deletes,
            stale_rule_set_history,
            stale_maintenance_evaluation_runs,
            stale_held_action_runs,
        ) = if general_settings.keep_history_forever {
            (0, 0, 0, 0, 0, 0, 0, 0)
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
                self.services
                    .workflow
                    .housekeeping
                    .delete_maintenance_evaluation_runs_older_than(history_retention_days)
                    .await?,
                self.services
                    .workflow
                    .housekeeping
                    .delete_held_lifecycle_action_runs_older_than(history_retention_days)
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
        let stale_acquisition_telemetry = self
            .services
            .workflow
            .housekeeping
            .delete_domain_events_older_than_for_types(
                ACQUISITION_TELEMETRY_RETENTION_DAYS,
                &acquisition_telemetry_domain_event_types(),
            )
            .await?;

        let stale_history_records = stale_release_decisions
            + stale_workflow_operations
            + stale_release_attempts
            + stale_operational_domain_events
            + stale_acquisition_telemetry
            + stale_history_events
            + stale_domain_events
            + stale_download_import_artifacts
            + stale_import_history
            + stale_download_queue_deletes
            + stale_rule_set_history
            + stale_maintenance_evaluation_runs
            + stale_held_action_runs
            + stale_list_sync_runs;

        // 3. Stale staged NZB artifacts (> 1 hour old)
        let now = self.runtime.environment.now();
        let staged_nzb_artifacts_pruned = self
            .services
            .workflow
            .staged_nzb_store
            .prune_staged_nzbs_older_than(now - chrono::Duration::hours(1))
            .await?;

        // 4. Purge expired recycle bin entries, and the entries of deleted
        // titles (per media root)
        let recycled_purged = self
            .purge_recycle_bins_with_record_retention(recycle_purge_record_retention_days(
                &general_settings,
            ))
            .await;

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
            stale_indexer_errors,
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
            stale_workflow_operations,
            stale_indexer_errors,
            stale_history_events,
            stale_operational_domain_events,
            stale_domain_events,
            stale_download_import_artifacts,
            stale_import_history,
            stale_download_queue_deletes,
            stale_rule_set_history,
            stale_list_sync_runs,
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
                let retention_days = config.retention_days;
                let entries = crate::recycle_bin::list_entries(&config, &media_root).await;
                (media_root, retention_days, entries)
            });
        }

        while let Some(result) = list_tasks.join_next().await {
            match result {
                Ok((_media_root, retention_days, Ok(entries))) => {
                    all_entries.extend(entries.into_iter().map(|entry| (entry, retention_days)));
                }
                Ok((media_root, _, Err(e))) => {
                    info!(error = %e, media_root = %media_root, "failed to list recycle entries")
                }
                Err(error) => {
                    info!(error = %error, "recycle entry list task failed")
                }
            }
        }

        let mut title_ids = all_entries
            .iter()
            .filter_map(|(entry, _)| entry.manifest.title_id.clone())
            .collect::<Vec<_>>();
        title_ids.sort();
        title_ids.dedup();
        let titles_by_id = self.recycle_entry_titles_by_id(title_ids).await?;

        let mut items = Vec::new();
        for (entry, retention_days) in all_entries {
            let title = entry
                .manifest
                .title_id
                .as_deref()
                .and_then(|title_id| titles_by_id.get(title_id));
            let Some(library) = resolve_recycle_entry_library_with_title(
                &entry,
                &roots,
                title.map(|(library_id, _, _)| library_id.as_str()),
            ) else {
                continue;
            };
            if selected_library_ids.contains(&library.id) {
                items.push(recycled_item_from_entry(
                    entry,
                    &library,
                    title.map(|(_, title_name, _)| title_name.clone()),
                    title
                        .filter(|(library_id, _, _)| library_id == &library.id)
                        .map(|(_, _, facet)| facet.clone()),
                    retention_days,
                ));
            }
        }

        items.sort_by(|a, b| b.recycled_at.cmp(&a.recycled_at));
        Ok(items)
    }

    /// List files due by the requested horizon (including overdue files), soonest first.
    /// Disabled or invalid recycle bins cannot schedule automatic deletion.
    pub async fn list_recycled_items_deleting_within(
        &self,
        actor: &scryer_domain::User,
        library_ids: Option<Vec<String>>,
        hours: i32,
    ) -> AppResult<Vec<RecycledItem>> {
        let items = self.list_recycled_items(actor, library_ids).await?;
        let active_roots: HashSet<String> = self
            .resolve_all_recycle_configs()
            .await
            .into_iter()
            .filter(|(_, config)| config.enabled && config.cleanup_enabled)
            .map(|(root, _)| root)
            .collect();
        let deadline =
            self.runtime.environment.now() + chrono::Duration::hours(i64::from(hours.clamp(1, 72)));
        let mut due = items
            .into_iter()
            .filter(|item| active_roots.contains(&item.media_root))
            .filter_map(|item| {
                let scheduled =
                    chrono::DateTime::parse_from_rfc3339(&item.scheduled_deletion_at).ok()?;
                (scheduled <= deadline).then_some((scheduled, item))
            })
            .collect::<Vec<_>>();
        due.sort_by(|(left_time, left), (right_time, right)| {
            left_time
                .cmp(right_time)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(due.into_iter().map(|(_, item)| item).collect())
    }

    /// Read the acquisition/import history tied to one authorized recycle entry.
    pub async fn recycled_item_history(
        &self,
        actor: &scryer_domain::User,
        entry_id: &str,
    ) -> AppResult<crate::RecycledItemHistory> {
        self.require_recycle_bin_page_access(actor).await?;
        crate::recycle_bin::validate_recycle_entry_id(entry_id)?;
        let roots = self.recycle_root_libraries().await?;
        for (media_root, config) in self.resolve_all_recycle_configs().await {
            let Some((_, manifest)) = crate::recycle_bin::find_entry(&config, entry_id).await?
            else {
                continue;
            };
            let entry = crate::recycle_bin::RecycleEntry {
                entry_id: entry_id.to_string(),
                manifest,
                media_root,
            };
            let library = self
                .resolve_recycle_entry_library(&entry, &roots)
                .await?
                .ok_or_else(|| {
                    AppError::unauthorized_reason(
                        crate::error_reasons::jobs::LIBRARY_ACCESS_DENIED,
                        "You do not have access to this library",
                    )
                })?;
            self.require_library_permission(
                actor,
                &library.id,
                scryer_domain::LibraryPermission::ManageTitles,
            )
            .await?;
            // A mounted bin may belong to another catalog. Never substitute a
            // similarly named title or read events from a title that moved libraries.
            if let Some(title_id) = entry.manifest.title_id.as_deref()
                && let Some(title) = self.services.catalog.titles.get_by_id(title_id).await?
                && title.library_id == library.id
            {
                return super::recycle_history::load_summary(self, &entry.manifest, title_id).await;
            }
            return Ok(crate::RecycledItemHistory {
                records: Vec::new(),
                truncated: false,
            });
        }
        Err(AppError::not_found_reason(
            crate::error_reasons::jobs::RECYCLE_ENTRY_NOT_FOUND,
            format!("recycle entry {entry_id}"),
        ))
    }

    async fn recycle_entry_titles_by_id(
        &self,
        mut title_ids: Vec<String>,
    ) -> AppResult<HashMap<String, (String, String, scryer_domain::MediaFacet)>> {
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
                titles_by_id.insert(title_id, (title.library_id, title.name, title.facet));
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
                AppError::validation_refused(
                    crate::error_reasons::jobs::RECYCLE_RESTORE_ALREADY_RUNNING,
                    format!("a recycle restore is already running for {entry_id}"),
                )
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
        self.restore_coordinator().spawn_task(async move {
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
                crate::stored_paths::path_to_stored_string(&original_path),
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
                crate::stored_paths::path_to_stored_string(&original_path),
                target_state
            ));
            if !destination_keys.insert(format!(
                "recycle-destination:{}",
                crate::stored_paths::path_to_stored_string(&original_path)
            )) {
                return Err(AppError::validation_refused(crate::error_reasons::jobs::SELECTED_RECYCLE_ENTRIES_CANNOT_SHARE_SAME_RESTORE_DESTINATION, "selected recycle entries cannot share the same restore destination"
                        .to_string()));
            }
            expected_target_states.insert(entry_id.clone(), target_state);
            library_ids.insert(context.library_id);
        }
        let fingerprint = fingerprint_parts.join("\n");
        if preview_fingerprint != fingerprint {
            return Err(AppError::validation_refused(
                crate::error_reasons::jobs::RECYCLE_RESTORE_PREVIEW_STALE,
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
                    AppError::validation_refused(crate::error_reasons::jobs::RECYCLE_RESTORE_ALREADY_RUNNING_ONE_SELECTED_DESTINATIONS, "a recycle restore is already running for one of the selected destinations"
                            .to_string())
                })?;
            guards.push(guard);
        }

        for entry_id in &entry_ids {
            let context = self.resolve_restore_recycled_item(actor, entry_id).await?;
            let current_state =
                restore_target_fingerprint(&context.manifest.original_path_buf()).await?;
            if expected_target_states.get(entry_id) != Some(&current_state) {
                return Err(AppError::validation_refused(
                    crate::error_reasons::jobs::RECYCLE_RESTORE_PREVIEW_STALE,
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
        self.restore_coordinator().spawn_task(async move {
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
        let library_ids = self.recycle_purge_selection(actor, &entry_ids).await?;
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
        self.restore_coordinator().spawn_task(async move {
            app.run_purge_recycled_items_job(run, actor_event, actor, job_entry_ids, guards)
                .await;
        });

        Ok(crate::RecycleBinBatchJobAccepted { entry_ids, job_run })
    }

    /// The sorted libraries of the selected entries, refusing the batch when
    /// any selected entry is not a trusted committed entry of a library the
    /// actor may manage. This is exactly the answer the full recycle-bin
    /// listing gives for those ids.
    ///
    /// Only the selected entries are opened: each bin's directory names are
    /// read once and only the names that were selected are inspected, and
    /// their titles' libraries are read in one batched statement. A damaged
    /// entry that was not selected is therefore quarantined by the next
    /// listing or expiry sweep rather than by this request; a selected one is
    /// still quarantined here, by the same inspection.
    pub(crate) async fn recycle_purge_selection(
        &self,
        actor: &scryer_domain::User,
        entry_ids: &[String],
    ) -> AppResult<Vec<String>> {
        let not_found = |entry_id: &str| {
            AppError::not_found_reason(
                crate::error_reasons::jobs::RECYCLE_ENTRY_NOT_FOUND,
                format!("recycle entry {entry_id}"),
            )
        };
        self.require_recycle_bin_page_access(actor).await?;
        let selected_library_ids = self.selected_recycle_library_ids(actor, None).await?;
        if selected_library_ids.is_empty() {
            return Err(not_found(entry_ids.first().map_or("", String::as_str)));
        }
        let roots = self.recycle_root_libraries().await?;
        let wanted = entry_ids.iter().map(String::as_str).collect::<HashSet<_>>();

        let mut found = Vec::new();
        for (media_root, config) in self.resolve_all_recycle_configs().await {
            // A disabled bin lists nothing; the readiness of an enabled one
            // is checked again by the lookup of each selected entry.
            if !(config.enabled && config.cleanup_enabled) {
                continue;
            }
            let names = match selected_names_in_recycle_bin(&config.base_path, &wanted).await {
                Ok(names) => names,
                Err(error) => {
                    info!(error = %error, media_root = %media_root, "failed to list recycle entries");
                    continue;
                }
            };
            for name in names {
                if let Some(entry) =
                    crate::recycle_bin::find_committed_entry(&config, &name).await?
                {
                    found.push(crate::recycle_bin::RecycleEntry {
                        entry_id: name,
                        manifest: entry.manifest,
                        media_root: media_root.clone(),
                    });
                }
            }
        }

        let mut title_ids = found
            .iter()
            .filter_map(|entry| entry.manifest.title_id.clone())
            .collect::<Vec<_>>();
        title_ids.sort();
        title_ids.dedup();
        let title_libraries = if title_ids.is_empty() {
            HashMap::new()
        } else {
            self.services
                .catalog
                .titles
                .list_library_ids_for_titles(&title_ids)
                .await?
                .into_iter()
                .collect::<HashMap<_, _>>()
        };

        let mut library_by_entry = HashMap::new();
        for entry in &found {
            let title_library_id = entry
                .manifest
                .title_id
                .as_deref()
                .and_then(|title_id| title_libraries.get(title_id))
                .map(String::as_str);
            if let Some(library) =
                resolve_recycle_entry_library_with_title(entry, &roots, title_library_id)
                && selected_library_ids.contains(&library.id)
            {
                library_by_entry
                    .entry(entry.entry_id.as_str())
                    .or_insert(library.id);
            }
        }

        let mut library_ids = HashSet::new();
        for entry_id in entry_ids {
            let library_id = library_by_entry
                .get(entry_id.as_str())
                .ok_or_else(|| not_found(entry_id))?;
            library_ids.insert(library_id.clone());
        }
        let mut library_ids = library_ids.into_iter().collect::<Vec<_>>();
        library_ids.sort();
        Ok(library_ids)
    }

    /// The selection as the full recycle-bin listing answers it, kept to
    /// compare the narrow lookup against.
    #[cfg(test)]
    pub(crate) async fn recycle_purge_selection_by_listing(
        &self,
        actor: &scryer_domain::User,
        entry_ids: &[String],
    ) -> AppResult<Vec<String>> {
        let available = self.list_recycled_items(actor, None).await?;
        let available_by_id = available
            .into_iter()
            .map(|item| (item.id, item.library_id))
            .collect::<HashMap<_, _>>();
        let mut library_ids = HashSet::new();
        for entry_id in entry_ids {
            let library_id = available_by_id.get(entry_id).ok_or_else(|| {
                AppError::not_found_reason(
                    crate::error_reasons::jobs::RECYCLE_ENTRY_NOT_FOUND,
                    format!("recycle entry {entry_id}"),
                )
            })?;
            library_ids.insert(library_id.clone());
        }
        let mut library_ids = library_ids.into_iter().collect::<Vec<_>>();
        library_ids.sort();
        Ok(library_ids)
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
                    AppError::validation_refused(
                        crate::error_reasons::jobs::RECYCLE_OPERATION_ALREADY_RUNNING,
                        format!("a recycle operation is already running for {entry_id}"),
                    )
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
                        AppError::unauthorized_reason(
                            crate::error_reasons::jobs::LIBRARY_ACCESS_DENIED,
                            "You do not have access to this library".to_string(),
                        )
                    })?;
                self.require_library_permission(
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
                    return Err(AppError::validation_refused(
                        crate::error_reasons::jobs::PATH_OUTSIDE_LIBRARY_ROOT,
                        format!(
                            "refusing to restore recycle entry {} because original path is outside the resolved library roots: {}",
                            entry_id, manifest.original_path
                        ),
                    ));
                }

                crate::folder_ownership::ensure_restore_ownership(
                    self,
                    manifest.title_id.as_deref(),
                    &original_path,
                )
                .await?;
                let recycled_file =
                    crate::recycle_bin::recycled_payload_path(&entry_dir, &manifest).await?;
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

        Err(AppError::not_found_reason(
            crate::error_reasons::jobs::RECYCLE_ENTRY_NOT_FOUND,
            format!("recycle entry {}", entry_id),
        ))
    }

    async fn restore_recycled_item_from_context(
        &self,
        actor: &scryer_domain::User,
        context: RestoreRecycledItemContext,
        conflict_policy: crate::RecycleRestoreConflictPolicy,
    ) -> AppResult<bool> {
        // Restoring writes a file back into the title's folder, which an
        // in-flight operation is copying out of (FR-084).
        if let Some(title_id) = context.manifest.title_id.as_deref() {
            self.ensure_location_ownership_allows_title(
                &crate::location::ownership_guard::RECYCLE_RESTORE_ENTRY,
                title_id,
            )
            .await?;
        }
        let original_path = context.manifest.original_path_buf();
        crate::folder_ownership::ensure_restore_ownership(
            self,
            context.manifest.title_id.as_deref(),
            &original_path,
        )
        .await?;
        let recycled_file =
            crate::recycle_bin::recycled_payload_path(&context.entry_dir, &context.manifest)
                .await?;

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
            self.track_restored_recycled_file(
                actor,
                title_id,
                &context.manifest,
                &context.entry_dir,
                &original_path,
                &restored_to,
            )
            .await;
        }
        Ok(true)
    }

    /// Rescans a restored file's title so the file is tracked again, puts back
    /// the acquisition-time columns the rescan cannot recompute, and records
    /// the restore. Failures here are logged, not returned: the file is already
    /// back on disk and the recycle entry is gone, so failing the restore would
    /// only misreport what happened.
    async fn track_restored_recycled_file(
        &self,
        actor: &scryer_domain::User,
        title_id: &str,
        manifest: &crate::recycle_bin::RecycleManifest,
        entry_dir: &Path,
        original_path: &Path,
        restored_to: &Path,
    ) {
        let restored_path = crate::stored_paths::path_to_stored_string(restored_to);
        let restored_library_file = crate::LibraryFile {
            path: restored_path.clone(),
            display_name: original_path
                .file_stem()
                .map(|value| value.to_string_lossy().into_owned())
                .unwrap_or_default(),
            nfo_path: None,
            size_bytes: tokio::fs::metadata(restored_to)
                .await
                .ok()
                .and_then(|metadata| i64::try_from(metadata.len()).ok()),
            source_signature_scheme: None,
            source_signature_value: None,
        };
        let title = match self.services.catalog.titles.get_by_id(title_id).await {
            Ok(Some(title)) => title,
            Ok(None) => {
                tracing::warn!(
                    title_id,
                    restored_to = %restored_to.display(),
                    "skipping restored file scan because the title no longer exists"
                );
                return;
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    title_id,
                    restored_to = %restored_to.display(),
                    "failed to load title before restored file scan"
                );
                return;
            }
        };
        if let Err(error) = self
            .scan_title_library_with_discovered_files(
                actor,
                title.clone(),
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

        let restored_row = match self
            .services
            .library
            .media_files
            .get_media_file_by_path(&restored_path)
            .await
        {
            Ok(row) => row,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    restored_to = %restored_to.display(),
                    "failed to load the media row of a restored file"
                );
                None
            }
        };
        if let Some(row) = restored_row.as_ref() {
            self.restore_recycled_media_row_snapshot(manifest, row)
                .await;
        }
        let episode_ids = match restored_row.as_ref() {
            Some(row) => self.restored_file_episode_ids(&title.id, &row.id).await,
            None => Vec::new(),
        };

        let recycle_entry_id = manifest.entry_id.clone().or_else(|| {
            entry_dir
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
        });
        let event = new_title_domain_event(
            DomainEventActor::from(actor),
            &title,
            scryer_domain::DomainEventPayload::MediaFileRestored(
                scryer_domain::MediaFileRestoredEventData {
                    title: title_context_snapshot(&title),
                    media_updates: vec![created_media_update(restored_path)],
                    file_id: restored_row.map(|row| row.id),
                    original_path: Some(manifest.original_path.clone()),
                    recycle_entry_id,
                    episode_ids,
                },
            ),
        );
        if let Err(error) = self.append_domain_event(event).await {
            tracing::warn!(
                title_id,
                error = %error,
                restored_to = %restored_to.display(),
                "recycled file restored but the restore event could not be recorded"
            );
        }
    }

    /// Copies the recycled row's acquisition-time columns onto the row the
    /// rescan recreated. The manifest snapshot is authoritative; entries
    /// without one fall back to the original row when it still exists (a
    /// location operation recycles a redundant source copy while its row
    /// follows the file to the destination).
    async fn restore_recycled_media_row_snapshot(
        &self,
        manifest: &crate::recycle_bin::RecycleManifest,
        restored_row: &crate::TitleMediaFile,
    ) {
        let snapshot = match manifest.media_row.clone() {
            Some(snapshot) => Some(snapshot),
            None => match manifest.original_file_id.as_deref() {
                Some(original_file_id) if original_file_id != restored_row.id => match self
                    .services
                    .library
                    .media_files
                    .get_media_file_by_id(original_file_id)
                    .await
                {
                    Ok(original) => original
                        .as_ref()
                        .map(crate::recycle_bin::RecycledMediaRowSnapshot::from_media_file),
                    Err(error) => {
                        tracing::warn!(
                            error = %error,
                            original_file_id,
                            "failed to load the original media row of a restored file"
                        );
                        None
                    }
                },
                _ => None,
            },
        };
        let Some(snapshot) = snapshot.filter(|snapshot| !snapshot.is_empty()) else {
            return;
        };
        if let Err(error) = self
            .services
            .library
            .media_files
            .restore_media_file_acquisition_metadata(&restored_row.id, &snapshot)
            .await
        {
            tracing::warn!(
                error = %error,
                file_id = %restored_row.id,
                "failed to restore acquisition metadata onto a restored media row"
            );
        }
    }

    async fn restored_file_episode_ids(&self, title_id: &str, file_id: &str) -> Vec<String> {
        match self
            .services
            .library
            .media_files
            .list_media_files_for_title(title_id)
            .await
        {
            Ok(files) => {
                let mut episode_ids = Vec::new();
                for episode_id in files
                    .into_iter()
                    .filter(|file| file.id == file_id)
                    .filter_map(|file| file.episode_id)
                {
                    if !episode_ids.contains(&episode_id) {
                        episode_ids.push(episode_id);
                    }
                }
                episode_ids
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    title_id,
                    file_id,
                    "failed to load episode links of a restored file"
                );
                Vec::new()
            }
        }
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
            return Err(AppError::validation_refused(
                crate::error_reasons::jobs::REPLACE_RECYCLE_BIN_UNAVAILABLE,
                format!(
                    "refusing to replace {} because the recycle bin is unavailable",
                    original_path.display()
                ),
            ));
        }

        let original_path_text = crate::stored_paths::path_to_stored_string(original_path);
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
            media_row: incumbent
                .as_ref()
                .map(crate::recycle_bin::RecycledMediaRowSnapshot::from_media_file),
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

        if let Some(incumbent) = incumbent
            && let Err(error) = self
                .delete_media_file_record_with_dependents(&incumbent.id)
                .await
        {
            if let Err(remove_error) = crate::fs_safety::remove_file_safely(original_path).await {
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
                    let _ = crate::fs_safety::remove_dir_all_safely_if_exists(&displaced.entry_dir)
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

    #[allow(clippy::too_many_arguments)] // The spawned job boundary owns its inputs explicitly.
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
                    return Err(AppError::validation_refused(crate::error_reasons::jobs::RESTORE_DESTINATION_CHANGED_AFTER_CONFIRMATION, "restore destination changed after confirmation; preview and retry this item"
                            .to_string()));
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
                        AppError::unauthorized_reason(
                            crate::error_reasons::jobs::LIBRARY_ACCESS_DENIED,
                            "You do not have access to this library".to_string(),
                        )
                    })?;
                self.require_library_permission(
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

        Err(AppError::not_found_reason(
            crate::error_reasons::jobs::RECYCLE_ENTRY_NOT_FOUND,
            format!("recycle entry {}", entry_id),
        ))
    }

    /// Accept a tracked background purge without scanning recycle directories in the request.
    pub async fn start_empty_recycle_bin_job(
        &self,
        actor: &scryer_domain::User,
        library_ids: Option<Vec<String>>,
    ) -> AppResult<JobRun> {
        self.require_recycle_bin_page_access(actor).await?;
        // Resolve "all accessible" now so a later grant cannot expand this request.
        let mut library_ids = self
            .selected_recycle_library_ids(actor, library_ids)
            .await?
            .into_iter()
            .collect::<Vec<_>>();
        library_ids.sort();
        let guard = self
            .runtime
            .jobs
            .interactive_operation_guards
            .try_acquire("recycle-bin-empty")
            .await
            .ok_or_else(|| {
                AppError::validation_refused(
                    crate::error_reasons::jobs::EMPTYING_RECYCLE_BIN_ALREADY_RUNNING,
                    "emptying the recycle bin is already running",
                )
            })?;
        let (run, job_run, actor_event) = self
            .create_recycle_batch_job_run(
                actor,
                JobKey::RecycleBinPurge,
                format!("recycle_bin_empty:{}", library_ids.join(",")),
                &[],
            )
            .await?;
        let app = self.clone();
        let actor = actor.clone();
        self.restore_coordinator().spawn_task(async move {
            let _guard = guard;
            // An empty explicit scope must not become "all accessible" on revalidation.
            let result = if library_ids.is_empty() {
                Ok(0)
            } else {
                app.empty_recycle_bin(&actor, Some(library_ids)).await
            };
            let (total, succeeded, results) = match result {
                Ok(count) => (count as usize, count as usize, Vec::new()),
                Err(error) => (
                    1,
                    0,
                    vec![serde_json::json!({
                        "status": "failed",
                        "error": error.to_string(),
                    })],
                ),
            };
            if let Err(error) = app
                .finish_recycle_batch_job(run, actor_event, "empty", total, succeeded, results)
                .await
            {
                warn!(error = %error, "failed to finish empty recycle bin job");
            }
        });
        Ok(job_run)
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

/// How many catalog rows one blocking orphan-sweep pass may probe before it
/// hands control back, so a large library does not hold one blocking thread
/// for the whole sweep. It is also how many rows one media-root availability
/// verdict is reused for before the root is proven again.
const ORPHAN_SWEEP_PROBES_PER_PASS: usize = 256;

/// Media-root availability verdicts for the orphan sweep, each reused for at
/// most [`ORPHAN_SWEEP_PROBES_PER_PASS`] rows.
///
/// Proving a root available costs an lstat, an opendir and a readdir, which
/// on a network share are round trips. The verdict is taken once per root per
/// window of rows instead of once per row, and every window starts over, so a
/// root that disconnects mid-sweep is seen again within one window.
struct OrphanSweepRootChecks {
    verdicts: std::collections::HashMap<PathBuf, crate::AppResult<()>>,
    rows_in_window: usize,
}

impl OrphanSweepRootChecks {
    fn new() -> Self {
        Self {
            verdicts: std::collections::HashMap::new(),
            rows_in_window: 0,
        }
    }

    /// The root verdict for one row: the same answer
    /// [`crate::fs_safety::resolve_available_root_for_path`] gives, with the
    /// filesystem part taken at most once per root per window.
    fn check(&mut self, path: &Path, roots: &[PathBuf]) -> crate::AppResult<()> {
        if self.rows_in_window >= ORPHAN_SWEEP_PROBES_PER_PASS {
            self.verdicts.clear();
            self.rows_in_window = 0;
        }
        self.rows_in_window += 1;
        let root = crate::fs_safety::containing_root_for_path(path, roots)?;
        self.verdicts
            .entry(root)
            .or_insert_with_key(|root| crate::fs_safety::ensure_root_available(root))
            .clone()
    }
}

/// What the sweep decided about one catalog row.
#[derive(Debug)]
enum OrphanProbe {
    /// The file is on disk; the row stays.
    Present,
    /// The root is available and the file is proven missing.
    Orphan,
    /// The root could not be proven available; the row is skipped.
    RootRefused(crate::AppError),
    /// The file's status could not be read; the row is skipped.
    StatusUnreadable(std::io::Error),
}

/// Decides one catalog row. Root availability is checked before probing the
/// file path so a disconnected media root cannot make every row look
/// orphaned. Adds one lstat of the file per row; the root probes come from
/// `root_checks`.
///
/// Blocking: call it from a blocking thread.
fn probe_orphan_media_file(
    media_file: &crate::HousekeepingMediaFileRootRow,
    root_checks: &mut OrphanSweepRootChecks,
) -> OrphanProbe {
    let file_path = crate::stored_paths::stored_path_to_path_buf(&media_file.file_path);
    let roots = media_file
        .root_paths
        .iter()
        .map(|root| crate::stored_paths::stored_path_to_path_buf(root))
        .collect::<Vec<_>>();
    if let Err(error) = root_checks.check(&file_path, &roots) {
        return OrphanProbe::RootRefused(error);
    }
    match std::fs::symlink_metadata(&file_path) {
        Ok(_) => OrphanProbe::Present,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => OrphanProbe::Orphan,
        Err(error) => OrphanProbe::StatusUnreadable(error),
    }
}

/// Probes catalog rows from the front of `files` until one is proven missing
/// on disk, and returns it. Rows whose root is unavailable, or whose status
/// cannot be read, are skipped with a warning; rows that exist are dropped.
/// Stops after `budget` rows without an orphan and returns `None`.
///
/// Blocking: call it from a blocking thread.
fn next_orphan_media_file(
    files: &mut std::collections::VecDeque<crate::HousekeepingMediaFileRootRow>,
    budget: usize,
    root_checks: &mut OrphanSweepRootChecks,
) -> Option<crate::HousekeepingMediaFileRootRow> {
    for _ in 0..budget {
        let media_file = files.pop_front()?;
        match probe_orphan_media_file(&media_file, root_checks) {
            OrphanProbe::Present => continue,
            OrphanProbe::Orphan => return Some(media_file),
            OrphanProbe::RootRefused(error) => {
                warn!(
                    error = %error,
                    file_id = %media_file.media_file_id,
                    path = %crate::stored_paths::stored_path_to_path_buf(&media_file.file_path).display(),
                    "skipping orphan media-file cleanup because media root is unavailable"
                );
                continue;
            }
            OrphanProbe::StatusUnreadable(error) => {
                warn!(
                    error = %error,
                    file_id = %media_file.media_file_id,
                    path = %crate::stored_paths::stored_path_to_path_buf(&media_file.file_path).display(),
                    "skipping orphan media-file cleanup because file status could not be read"
                );
                continue;
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sweep's decisions as they were made inline, row by row, before the
    /// probes moved to a blocking thread.
    fn inline_orphan_decisions(files: &[crate::HousekeepingMediaFileRootRow]) -> Vec<String> {
        let mut orphans = Vec::new();
        for media_file in files {
            let file_path = crate::stored_paths::stored_path_to_path_buf(&media_file.file_path);
            let roots = media_file
                .root_paths
                .iter()
                .map(|root| crate::stored_paths::stored_path_to_path_buf(root))
                .collect::<Vec<_>>();
            if crate::fs_safety::resolve_available_root_for_path(&file_path, &roots).is_err() {
                continue;
            }
            match std::fs::symlink_metadata(&file_path) {
                Ok(_) => continue,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => continue,
            }
            orphans.push(media_file.media_file_id.clone());
        }
        orphans
    }

    #[test]
    fn blocking_orphan_probe_decides_exactly_like_the_inline_sweep() {
        let temp = tempfile::tempdir().expect("temp dir");
        let mounted = temp.path().join("mounted");
        let empty = temp.path().join("empty");
        let unmounted = temp.path().join("unmounted");
        std::fs::create_dir_all(mounted.join("Sample Show")).expect("mounted root");
        std::fs::create_dir_all(&empty).expect("empty root");
        std::fs::write(mounted.join("Sample Show").join("present.mkv"), b"x").expect("present");
        std::fs::write(mounted.join("Sample Show").join("kept.mkv"), b"x").expect("kept");

        let row = |id: &str, path: std::path::PathBuf, roots: &[&std::path::Path]| {
            crate::HousekeepingMediaFileRootRow {
                media_file_id: id.to_string(),
                title_id: format!("title-{id}"),
                file_path: path.to_string_lossy().into_owned(),
                library_id: "library-1".to_string(),
                root_paths: roots
                    .iter()
                    .map(|root| root.to_string_lossy().into_owned())
                    .collect(),
            }
        };
        let mut files = Vec::new();
        for index in 0..40 {
            let id = format!("file-{index:03}");
            let (path, roots): (std::path::PathBuf, Vec<&std::path::Path>) = match index % 6 {
                0 => (
                    mounted.join("Sample Show").join("present.mkv"),
                    vec![&mounted],
                ),
                1 => (
                    mounted
                        .join("Sample Show")
                        .join(format!("gone-{index}.mkv")),
                    vec![&mounted],
                ),
                2 => (empty.join(format!("gone-{index}.mkv")), vec![&empty]),
                3 => (
                    unmounted.join(format!("gone-{index}.mkv")),
                    vec![&unmounted],
                ),
                4 => (
                    temp.path().join(format!("outside-{index}.mkv")),
                    vec![&mounted],
                ),
                _ => (
                    mounted.join("Sample Show").join("kept.mkv"),
                    vec![&empty, &mounted],
                ),
            };
            files.push(row(&id, path, &roots));
        }

        let expected = inline_orphan_decisions(&files);
        assert!(!expected.is_empty(), "the fixture must contain orphans");

        for budget in [1, 3, ORPHAN_SWEEP_PROBES_PER_PASS] {
            let mut pending = files
                .iter()
                .cloned()
                .collect::<std::collections::VecDeque<_>>();
            let mut root_checks = OrphanSweepRootChecks::new();
            let mut found = Vec::new();
            while !pending.is_empty() {
                if let Some(orphan) = next_orphan_media_file(&mut pending, budget, &mut root_checks)
                {
                    found.push(orphan.media_file_id);
                }
            }
            assert_eq!(found, expected, "budget {budget}");
        }
    }

    /// One row's decision as a comparable value: the kind, and for a refused
    /// root the refusal's reason code.
    fn probe_label(probe: &OrphanProbe) -> String {
        match probe {
            OrphanProbe::Present => "present".to_string(),
            OrphanProbe::Orphan => "orphan".to_string(),
            OrphanProbe::RootRefused(error) => {
                format!("refused:{}", error.validation_reason().unwrap_or("none"))
            }
            OrphanProbe::StatusUnreadable(_) => "unreadable".to_string(),
        }
    }

    /// The per-row check the sweep made before root verdicts were reused.
    fn per_row_probe_label(media_file: &crate::HousekeepingMediaFileRootRow) -> String {
        let file_path = crate::stored_paths::stored_path_to_path_buf(&media_file.file_path);
        let roots = media_file
            .root_paths
            .iter()
            .map(|root| crate::stored_paths::stored_path_to_path_buf(root))
            .collect::<Vec<_>>();
        if let Err(error) = crate::fs_safety::resolve_available_root_for_path(&file_path, &roots) {
            return format!("refused:{}", error.validation_reason().unwrap_or("none"));
        }
        match std::fs::symlink_metadata(&file_path) {
            Ok(_) => "present".to_string(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => "orphan".to_string(),
            Err(_) => "unreadable".to_string(),
        }
    }

    fn sweep_row(
        id: &str,
        path: &std::path::Path,
        roots: &[&std::path::Path],
    ) -> crate::HousekeepingMediaFileRootRow {
        crate::HousekeepingMediaFileRootRow {
            media_file_id: id.to_string(),
            title_id: format!("title-{id}"),
            file_path: path.to_string_lossy().into_owned(),
            library_id: "library-1".to_string(),
            root_paths: roots
                .iter()
                .map(|root| root.to_string_lossy().into_owned())
                .collect(),
        }
    }

    #[test]
    fn reused_root_verdicts_sweep_and_refuse_the_same_rows_as_per_row_checks() {
        let temp = tempfile::tempdir().expect("temp dir");
        let mounted = temp.path().join("Emberfall");
        let empty = temp.path().join("Cinderwake");
        let unmounted = temp.path().join("Ashgrove");
        let linked = temp.path().join("Ashgrove Link");
        std::fs::create_dir_all(mounted.join("Season 01")).expect("mounted root");
        std::fs::create_dir_all(&empty).expect("empty root");
        std::fs::write(mounted.join("Season 01").join("present.mkv"), b"x").expect("present");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&mounted, &linked).expect("symlinked root");

        // More rows than one window, so verdicts are both reused and renewed.
        let mut files = Vec::new();
        for index in 0..(ORPHAN_SWEEP_PROBES_PER_PASS * 2 + 37) {
            let id = format!("file-{index:04}");
            let row = match index % 7 {
                0 => sweep_row(
                    &id,
                    &mounted.join("Season 01").join("present.mkv"),
                    &[&mounted],
                ),
                1 => sweep_row(
                    &id,
                    &mounted.join("Season 01").join(format!("gone-{index}.mkv")),
                    &[&mounted],
                ),
                2 => sweep_row(&id, &empty.join(format!("gone-{index}.mkv")), &[&empty]),
                3 => sweep_row(
                    &id,
                    &unmounted.join(format!("gone-{index}.mkv")),
                    &[&unmounted],
                ),
                4 => sweep_row(
                    &id,
                    &temp.path().join(format!("outside-{index}.mkv")),
                    &[&mounted],
                ),
                5 => sweep_row(&id, &linked.join(format!("gone-{index}.mkv")), &[&linked]),
                _ => sweep_row(
                    &id,
                    &mounted.join("Season 01").join("present.mkv"),
                    &[&empty, &mounted],
                ),
            };
            files.push(row);
        }

        let expected = files.iter().map(per_row_probe_label).collect::<Vec<_>>();
        let mut root_checks = OrphanSweepRootChecks::new();
        let actual = files
            .iter()
            .map(|row| probe_label(&probe_orphan_media_file(row, &mut root_checks)))
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
        let empty_reason = crate::error_reasons::fs_safety::CONFIGURED_MEDIA_ROOT_EMPTY;
        let mut required = vec![
            "present".to_string(),
            "orphan".to_string(),
            format!("refused:{empty_reason}"),
            format!(
                "refused:{}",
                crate::error_reasons::fs_safety::CONFIGURED_MEDIA_ROOT_UNAVAILABLE
            ),
            format!("refused:{}", crate::delete_refusal::PATH_OUTSIDE_ROOTS),
        ];
        if cfg!(unix) {
            required.push(format!(
                "refused:{}",
                crate::error_reasons::fs_safety::CONFIGURED_MEDIA_ROOT_SYMLINK
            ));
        }
        for label in &required {
            assert!(
                expected.iter().any(|seen| seen == label),
                "the fixture must produce {label}: {expected:?}"
            );
        }
    }

    #[test]
    fn a_root_that_empties_mid_sweep_is_refused_from_the_next_window() {
        let temp = tempfile::tempdir().expect("temp dir");
        let root = temp.path().join("Emberfall");
        std::fs::create_dir_all(&root).expect("root");
        let marker = root.join("mounted");
        std::fs::write(&marker, b"x").expect("marker");

        // Every row's file is missing, so a row is an orphan exactly while its
        // root is believed available.
        let files = (0..(ORPHAN_SWEEP_PROBES_PER_PASS * 2 + 10))
            .map(|index| {
                sweep_row(
                    &format!("file-{index:04}"),
                    &root.join(format!("gone-{index}.mkv")),
                    &[&root],
                )
            })
            .collect::<Vec<_>>();

        let disconnect_after = 10;
        let mut root_checks = OrphanSweepRootChecks::new();
        let mut labels = Vec::new();
        for (index, row) in files.iter().enumerate() {
            if index == disconnect_after {
                // The share drops: the mount point is left as an empty folder.
                std::fs::remove_file(&marker).expect("unmount");
            }
            labels.push(probe_label(&probe_orphan_media_file(row, &mut root_checks)));
        }

        let first_refused = labels
            .iter()
            .position(|label| label != "orphan")
            .expect("the emptied root must be refused");
        assert_eq!(
            first_refused, ORPHAN_SWEEP_PROBES_PER_PASS,
            "the root is re-proven when the next window starts"
        );
        let refused_empty = format!(
            "refused:{}",
            crate::error_reasons::fs_safety::CONFIGURED_MEDIA_ROOT_EMPTY
        );
        assert!(
            labels[first_refused..]
                .iter()
                .all(|label| *label == refused_empty),
            "{labels:?}"
        );
    }

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

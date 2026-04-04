use super::*;
use tracing::info;

const MEDIA_ROOT_KEYS: [(&str, &str); 3] = [
    ("series.path", "/data/series"),
    ("anime.path", "/data/anime"),
    ("movies.path", "/data/movies"),
];

impl AppUseCase {
    /// Resolve media root paths and their recycle configs.
    async fn resolve_all_recycle_configs(
        &self,
    ) -> Vec<(String, crate::recycle_bin::RecycleBinConfig)> {
        let mut configs = Vec::with_capacity(MEDIA_ROOT_KEYS.len());
        for (key, default) in &MEDIA_ROOT_KEYS {
            let media_root = self
                .read_setting_string_value_for_scope(super::SETTINGS_SCOPE_MEDIA, key, None)
                .await
                .ok()
                .flatten()
                .unwrap_or_else(|| default.to_string());
            let config = crate::recycle_bin::resolve_recycle_config(self, Some(&media_root)).await;
            configs.push((media_root, config));
        }
        configs
    }

    pub async fn run_housekeeping(&self) -> AppResult<HousekeepingReport> {
        info!("starting housekeeping");

        // 1. Orphaned media files (file_path no longer exists on disk)
        let all_files = self
            .services
            .housekeeping
            .list_all_media_file_paths()
            .await?;
        let orphan_ids: Vec<String> = all_files
            .into_iter()
            .filter(|(_, path)| !std::path::Path::new(path).exists())
            .map(|(id, _)| id)
            .collect();
        let orphaned_media_files = if !orphan_ids.is_empty() {
            self.services
                .housekeeping
                .delete_media_files_by_ids(&orphan_ids)
                .await?
        } else {
            0
        };

        // 2. Stale release decisions (> 30 days)
        let stale_release_decisions = self
            .services
            .housekeeping
            .delete_release_decisions_older_than(30)
            .await?;

        // 3. Stale release attempts (> 90 days, non-pending)
        let stale_release_attempts = self
            .services
            .housekeeping
            .delete_release_attempts_older_than(90)
            .await?;

        // 4. Expired event outboxes (dispatched > 7 days ago)
        let expired_event_outboxes = self
            .services
            .housekeeping
            .delete_dispatched_event_outboxes_older_than(7)
            .await?;

        // 5. Stale history events (> 365 days)
        let stale_history_events = self
            .services
            .housekeeping
            .delete_history_events_older_than(365)
            .await?;
        let stale_domain_events = self
            .services
            .housekeeping
            .delete_domain_events_older_than(365)
            .await?;

        // 6. Stale staged NZB artifacts (> 1 hour old)
        let staged_nzb_artifacts_pruned = self
            .services
            .staged_nzb_store
            .prune_staged_nzbs_older_than(chrono::Utc::now() - chrono::Duration::hours(1))
            .await?;

        // 7. Purge expired recycle bin entries (per media root)
        let mut recycled_purged = 0u32;
        for (media_root, config) in self.resolve_all_recycle_configs().await {
            match crate::recycle_bin::purge_expired(&config).await {
                Ok(n) => recycled_purged += n,
                Err(e) => info!(error = %e, media_root = %media_root, "recycle bin purge failed"),
            }
        }

        let report = HousekeepingReport {
            orphaned_media_files,
            stale_release_decisions,
            stale_release_attempts,
            expired_event_outboxes,
            stale_history_events,
            staged_nzb_artifacts_pruned,
            recycled_purged,
            ran_at: chrono::Utc::now().to_rfc3339(),
        };

        info!(
            orphaned_media_files,
            stale_release_decisions,
            stale_release_attempts,
            expired_event_outboxes,
            stale_history_events,
            stale_domain_events,
            staged_nzb_artifacts_pruned,
            recycled_purged,
            "housekeeping completed"
        );

        Ok(report)
    }

    /// List all items across all recycle bins, sorted newest first.
    pub async fn list_recycled_items(
        &self,
        actor: &scryer_domain::User,
    ) -> AppResult<Vec<crate::recycle_bin::RecycleEntry>> {
        require(actor, &scryer_domain::Entitlement::ManageConfig)?;

        let mut all_entries = Vec::new();
        for (media_root, config) in self.resolve_all_recycle_configs().await {
            match crate::recycle_bin::list_entries(&config, &media_root).await {
                Ok(entries) => all_entries.extend(entries),
                Err(e) => {
                    info!(error = %e, media_root = %media_root, "failed to list recycle entries")
                }
            }
        }

        all_entries.sort_by(|a, b| b.manifest.recycled_at.cmp(&a.manifest.recycled_at));
        Ok(all_entries)
    }

    /// Restore a single recycled item back to its original path.
    pub async fn restore_recycled_item(
        &self,
        actor: &scryer_domain::User,
        entry_id: &str,
    ) -> AppResult<bool> {
        require(actor, &scryer_domain::Entitlement::ManageConfig)?;

        for (_media_root, config) in self.resolve_all_recycle_configs().await {
            if let Some((entry_dir, manifest)) =
                crate::recycle_bin::find_entry(&config, entry_id).await?
            {
                let original_path = std::path::Path::new(&manifest.original_path);
                let file_name = original_path
                    .file_name()
                    .unwrap_or_else(|| std::ffi::OsStr::new("unknown"));
                let recycled_file = entry_dir.join(file_name);

                if !recycled_file.exists() {
                    return Err(AppError::Repository(format!(
                        "recycled file not found in entry: {}",
                        recycled_file.display()
                    )));
                }

                crate::recycle_bin::restore_from_recycle(&recycled_file, original_path).await?;
                let _ = tokio::fs::remove_dir_all(&entry_dir).await;
                return Ok(true);
            }
        }

        Err(AppError::NotFound(format!("recycle entry {}", entry_id)))
    }

    /// Permanently delete a single recycled item.
    pub async fn delete_recycled_item(
        &self,
        actor: &scryer_domain::User,
        entry_id: &str,
    ) -> AppResult<bool> {
        require(actor, &scryer_domain::Entitlement::ManageConfig)?;

        for (_media_root, config) in self.resolve_all_recycle_configs().await {
            if let Some((entry_dir, _manifest)) =
                crate::recycle_bin::find_entry(&config, entry_id).await?
            {
                tokio::fs::remove_dir_all(&entry_dir).await.map_err(|e| {
                    AppError::Repository(format!(
                        "failed to delete recycle entry {}: {}",
                        entry_dir.display(),
                        e
                    ))
                })?;
                return Ok(true);
            }
        }

        Err(AppError::NotFound(format!("recycle entry {}", entry_id)))
    }

    /// Empty all recycle bins across all media roots.
    pub async fn empty_recycle_bin(&self, actor: &scryer_domain::User) -> AppResult<u32> {
        require(actor, &scryer_domain::Entitlement::ManageConfig)?;

        let mut total = 0u32;
        for (media_root, config) in self.resolve_all_recycle_configs().await {
            match crate::recycle_bin::purge_all(&config).await {
                Ok(n) => total += n,
                Err(e) => {
                    info!(error = %e, media_root = %media_root, "failed to empty recycle bin")
                }
            }
        }
        Ok(total)
    }
}

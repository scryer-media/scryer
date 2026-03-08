use super::*;
use tracing::info;

impl AppUseCase {
    pub async fn run_housekeeping(&self) -> AppResult<HousekeepingReport> {
        info!("starting housekeeping");

        // 1. Orphaned media files (file_path no longer exists on disk)
        let all_files = self.services.housekeeping.list_all_media_file_paths().await?;
        let orphan_ids: Vec<String> = all_files
            .into_iter()
            .filter(|(_, path)| !std::path::Path::new(path).exists())
            .map(|(id, _)| id)
            .collect();
        let orphaned_media_files = if !orphan_ids.is_empty() {
            self.services.housekeeping.delete_media_files_by_ids(&orphan_ids).await?
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

        // 6. Purge expired recycle bin entries (per media root)
        let mut recycled_purged = 0u32;
        let path_keys = [
            ("series.path", "/media/series"),
            ("anime.path", "/media/anime"),
            ("movies.path", "/media/movies"),
        ];
        for (key, default) in &path_keys {
            let media_root = self
                .read_setting_string_value_for_scope(super::SETTINGS_SCOPE_MEDIA, key, None)
                .await
                .ok()
                .flatten()
                .unwrap_or_else(|| default.to_string());
            let config =
                crate::recycle_bin::resolve_recycle_config(self, Some(&media_root)).await;
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
            recycled_purged,
            ran_at: chrono::Utc::now().to_rfc3339(),
        };

        info!(
            orphaned_media_files,
            stale_release_decisions,
            stale_release_attempts,
            expired_event_outboxes,
            stale_history_events,
            recycled_purged,
            "housekeeping completed"
        );

        Ok(report)
    }
}

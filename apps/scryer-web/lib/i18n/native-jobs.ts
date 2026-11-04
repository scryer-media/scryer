type Translate = (
  key: string,
  values?: Record<string, string | number | boolean | null | undefined>,
) => string;

const nativeTextKeys: Record<string, string> = {
  "Library Scan: Movies": "jobs.builtin.LibraryScanMovies.name",
  "Scan the movies library and import missing movie files.":
    "jobs.builtin.LibraryScanMovies.description",
  "Library Scan: Series": "jobs.builtin.LibraryScanSeries.name",
  "Scan the series library and reconcile episodic files.":
    "jobs.builtin.LibraryScanSeries.description",
  "Library Scan: Anime": "jobs.builtin.LibraryScanAnime.name",
  "Scan the anime library and reconcile episodic files.":
    "jobs.builtin.LibraryScanAnime.description",
  "Background Library Refresh: Movies":
    "jobs.builtin.BackgroundLibraryRefreshMovies.name",
  "Lightweight movie library refresh for newly discovered folders or files.":
    "jobs.builtin.BackgroundLibraryRefreshMovies.description",
  "Background Library Refresh: Series":
    "jobs.builtin.BackgroundLibraryRefreshSeries.name",
  "Lightweight series refresh that discovers new folders and additive file changes.":
    "jobs.builtin.BackgroundLibraryRefreshSeries.description",
  "Background Library Refresh: Anime":
    "jobs.builtin.BackgroundLibraryRefreshAnime.name",
  "Lightweight anime refresh that discovers new folders and additive file changes.":
    "jobs.builtin.BackgroundLibraryRefreshAnime.description",
  "Prowlarr Sync": "jobs.builtin.ProwlarrSync.name",
  "Sync enabled managed Prowlarr parents and reconcile child indexers.":
    "jobs.builtin.ProwlarrSync.description",
  "RSS Sync": "jobs.builtin.RssSync.name",
  "Fetch RSS feeds from enabled indexers and evaluate new releases.":
    "jobs.builtin.RssSync.description",
  "Subtitle Search": "jobs.builtin.SubtitleSearch.name",
  "Search for missing subtitles for monitored media.":
    "jobs.builtin.SubtitleSearch.description",
  "Plugin Catalog Refresh": "jobs.builtin.PluginRegistryRefresh.name",
  "Refresh the installed plugin catalog metadata.":
    "jobs.builtin.PluginRegistryRefresh.description",
  Housekeeping: "jobs.builtin.Housekeeping.name",
  "Clean stale records and purge expired artifacts.":
    "jobs.builtin.Housekeeping.description",
  "Health Checks": "jobs.builtin.HealthChecks.name",
  "Run configured system health checks.":
    "jobs.builtin.HealthChecks.description",
  "Automatic Backup": "jobs.builtin.AutoBackup.name",
  "Create a daily backup snapshot and keep the newest successful automatic backups.":
    "jobs.builtin.AutoBackup.description",
  "Pending Release Processing": "jobs.builtin.PendingReleaseProcessing.name",
  "Compatibility job; pending releases are re-evaluated during RSS sync.":
    "jobs.builtin.PendingReleaseProcessing.description",
  "Staged NZB Prune": "jobs.builtin.StagedNzbPrune.name",
  "Prune expired staged NZB artifacts.":
    "jobs.builtin.StagedNzbPrune.description",
  "Full Hash Backfill": "jobs.builtin.FullHashBackfill.name",
  "Slowly compute missing full-file content hashes so duplicate detection and move verification have something to compare.":
    "jobs.builtin.FullHashBackfill.description",
  "Discovery Sync": "jobs.builtin.DiscoverySync.name",
  "Evaluate local discovery freshness and refresh SMG discovery snapshots.":
    "jobs.builtin.DiscoverySync.description",
  "Artwork Encoding": "jobs.builtin.ArtworkEncoding.name",
  "Encode pending artwork overnight; Run now overrides the nightly window.":
    "jobs.builtin.ArtworkEncoding.description",
  "Title Image Cache Refresh": "jobs.builtin.TitleImageCacheRefresh.name",
  "Refresh remote artwork URLs from SMG and rebuild locally processed title images.":
    "jobs.builtin.TitleImageCacheRefresh.description",
  "Title Deletion": "jobs.builtin.TitleDeletion.name",
  "Delete selected titles from the catalog.":
    "jobs.builtin.TitleDeletion.description",
  "Title Rename": "jobs.builtin.TitleRename.name",
  "Rename files for selected titles.": "jobs.builtin.TitleRename.description",
  "Refresh File Scores": "jobs.builtin.MediaFileRescore.name",
  "Evaluate selected files under current scoring rules.":
    "jobs.builtin.MediaFileRescore.description",
  "Media File Deletion": "jobs.builtin.MediaFileDeletion.name",
  "Delete a media file from disk and the catalog.":
    "jobs.builtin.MediaFileDeletion.description",
  "Recycle Bin Restore": "jobs.builtin.RecycleBinRestore.name",
  "Restore a recycled file to its library.":
    "jobs.builtin.RecycleBinRestore.description",
  "Recycle Bin Purge": "jobs.builtin.RecycleBinPurge.name",
  "Permanently delete selected recycled files.":
    "jobs.builtin.RecycleBinPurge.description",
  "Acquisition Search": "jobs.builtin.AcquisitionSearch.name",
  "Interactive acquisition search over the selected wanted/upgrade scopes.":
    "jobs.builtin.AcquisitionSearch.description",
  "Application Upgrade": "jobs.builtin.ApplicationUpgrade.name",
  "Download, verify, and apply a signed application upgrade.":
    "jobs.builtin.ApplicationUpgrade.description",
  "Maintenance Rule Evaluation": "jobs.builtin.MaintenanceRuleEvaluation.name",
  "Evaluate enabled maintenance rules and reconcile lifecycle candidates. Never executes an action.":
    "jobs.builtin.MaintenanceRuleEvaluation.description",
  "Maintenance Action Handling": "jobs.builtin.LifecycleActionHandling.name",
  "Execute due maintenance candidates of armed observe-mode rules, behind the instance effect gates and full safety rechecks.":
    "jobs.builtin.LifecycleActionHandling.description",
  "Media Server Signal Sync": "jobs.builtin.MediaServerSignalSync.name",
  "Read played state for verified linked accounts on enabled media-server connections and store it as normalized watch signals.":
    "jobs.builtin.MediaServerSignalSync.description",
  "List Sync": "jobs.builtin.ListSync.name",
  "Read due list subscriptions, then add, request, or record their titles according to each list's settings.":
    "jobs.builtin.ListSync.description",
  "Location Operation": "jobs.builtin.LocationOperation.name",
  "Move title content and catalog placement between roots or libraries.":
    "jobs.builtin.LocationOperation.description",
  "Custom Job": "jobs.builtin.CustomJob.name",
  "Run a user-defined script on its own schedule.":
    "jobs.builtin.CustomJob.description",
  "Manual only": "jobs.builtinText.Manual_only",
  "Every 2 hours": "jobs.builtinText.Every_2_hours",
  "Every 6 hours": "jobs.builtinText.Every_6_hours",
  "Every 8 hours": "jobs.builtinText.Every_8_hours",
  "Every 12 hours": "jobs.builtinText.Every_12_hours",
  "Every 24 hours": "jobs.builtinText.Every_24_hours",
  "Every 15 minutes": "jobs.builtinText.Every_15_minutes",
  "Every hour": "jobs.builtinText.Every_hour",
  "Daily at configured local time":
    "jobs.builtinText.Daily_at_configured_local_time",
  "Dynamic discovery evaluator with daily backstop":
    "jobs.builtinText.Dynamic_discovery_evaluator_with_daily_backstop",
  "Every 30 minutes, 02:00–06:00 host local time":
    "jobs.builtinText.Every_30_minutes_02_00_06_00_host_local_time",
  "Re-evaluated during RSS sync":
    "jobs.builtinText.Re_evaluated_during_RSS_sync",
  "Scheduler-paced per indexer (15-minute target)":
    "jobs.builtinText.Scheduler_paced_per_indexer_15_minute_target",
  "Based on subtitle settings interval":
    "jobs.builtinText.Based_on_subtitle_settings_interval",
  "Encode pending artwork. Run now overrides the nightly window. Waiting for pending artwork.":
    "jobs.builtinText.Encode_pending_artwork_Run_now_overrides_the_nightly_window_Waiting_for_pending_artwork",
  "Encode pending artwork. Run now overrides the nightly window. Waiting for the nightly window.":
    "jobs.artworkWait.nightly",
  "Encode pending artwork. Run now overrides the nightly window. Waiting for library scans.":
    "jobs.artworkWait.scan",
};

/** Translate known native metadata only, never user script names or arbitrary logs. */
export function translateNativeJobText(
  jobKey: string,
  text: string,
  t: Translate,
): string {
  if (jobKey === "CUSTOM_JOB") return text;
  const window =
    /^Daily (\d{2}:\d{2})–(\d{2}:\d{2}) host local time; host now (.+)$/.exec(
      text,
    );
  if (window)
    return t("jobs.artworkWindow", {
      start: window[1],
      end: window[2],
      clock: window[3],
    });
  const key = nativeTextKeys[text];
  return key ? t(key) : text;
}

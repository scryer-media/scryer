import type { LibraryScanProgress } from "./library-scans";

export type JobCategory =
  | "library"
  | "acquisition"
  | "maintenance"
  | "subtitles"
  | "system";

export type JobSection = "primary" | "maintenance";

export type JobScheduleKind = "manual" | "interval" | "startup_interval";

export type JobTriggerSource =
  | "manual"
  | "scheduled_startup"
  | "scheduled_interval"
  | "system_internal";

export type JobRunStatus =
  | "queued"
  | "discovering"
  | "running"
  | "completed"
  | "warning"
  | "failed";

export type JobKey =
  | "library_scan_movies"
  | "library_scan_series"
  | "library_scan_anime"
  | "background_library_refresh_movies"
  | "background_library_refresh_series"
  | "background_library_refresh_anime"
  | "rss_sync"
  | "subtitle_search"
  | "metadata_refresh"
  | "plugin_registry_refresh"
  | "housekeeping"
  | "health_checks"
  | "wanted_sync"
  | "pending_release_processing"
  | "staged_nzb_prune";

export type JobScheduleInfo = {
  kind: JobScheduleKind;
  description: string;
  intervalSeconds: number | null;
  initialDelaySeconds: number | null;
  nextRunAt: string | null;
};

export type JobDefinition = {
  key: JobKey;
  displayName: string;
  description: string;
  category: JobCategory;
  section: JobSection;
  manualTriggerAllowed: boolean;
  usesLibraryScanProgress: boolean;
  schedule: JobScheduleInfo;
};

export type JobRun = {
  id: string;
  jobKey: JobKey;
  displayName: string;
  category: JobCategory;
  section: JobSection;
  status: JobRunStatus;
  triggerSource: JobTriggerSource;
  startedAt: string;
  completedAt: string | null;
  summaryText: string | null;
  errorText: string | null;
  progressJson: string | null;
  libraryScanProgress: LibraryScanProgress | null;
};

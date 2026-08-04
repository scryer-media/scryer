import type { LibraryScanProgress } from "./library-scans";

export type JobCategory =
  | "LIBRARY"
  | "ACQUISITION"
  | "MAINTENANCE"
  | "SUBTITLES"
  | "SYSTEM";

export type JobSection = "PRIMARY" | "MAINTENANCE";

export type JobScheduleKind =
  | "MANUAL"
  | "INTERVAL"
  | "STARTUP_AND_INTERVAL"
  | "DAILY_AT_TIME";

export type JobTriggerSource =
  | "MANUAL"
  | "SCHEDULED_STARTUP"
  | "SCHEDULED_INTERVAL"
  | "SCHEDULED_DAILY"
  | "SYSTEM_INTERNAL";

export type JobRunStatus =
  | "QUEUED"
  | "DISCOVERING"
  | "RUNNING"
  | "COMPLETED"
  | "WARNING"
  | "FAILED";

export type JobKey =
  | "LIBRARY_SCAN_MOVIES"
  | "LIBRARY_SCAN_SERIES"
  | "LIBRARY_SCAN_ANIME"
  | "BACKGROUND_LIBRARY_REFRESH_MOVIES"
  | "BACKGROUND_LIBRARY_REFRESH_SERIES"
  | "BACKGROUND_LIBRARY_REFRESH_ANIME"
  | "RSS_SYNC"
  | "SUBTITLE_SEARCH"
  | "PLUGIN_REGISTRY_REFRESH"
  | "HOUSEKEEPING"
  | "HEALTH_CHECKS"
  | "PENDING_RELEASE_PROCESSING"
  | "STAGED_NZB_PRUNE"
  | "TITLE_IMAGE_CACHE_REFRESH"
  | "TITLE_DELETION"
  | "MEDIA_FILE_DELETION"
  | "RECYCLE_BIN_RESTORE"
  | "RECYCLE_BIN_PURGE"
  | "ACQUISITION_SEARCH"
  | "AUTO_BACKUP";

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
  summaryJson: unknown;
  summaryText: string | null;
  errorText: string | null;
  progressJson: unknown;
  libraryScanProgress: LibraryScanProgress | null;
};

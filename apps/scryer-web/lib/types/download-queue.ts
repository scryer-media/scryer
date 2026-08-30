import type { ReleaseQueueScope } from "./releases";

export type DownloadQueueState =
  | "QUEUED"
  | "DOWNLOADING"
  | "VERIFYING"
  | "REPAIRING"
  | "EXTRACTING"
  | "PAUSED"
  | "COMPLETED"
  | "IMPORT_PENDING"
  | "WARNING"
  | "FAILED";

export type ImportStatus =
  | "PENDING"
  | "RUNNING"
  | "PROCESSING"
  | "COMPLETED"
  | "FAILED"
  | "SKIPPED";

export type ImportErrorCode =
  | "FILE_NOT_FOUND"
  | "EPISODE_NOT_FOUND"
  | "EPISODE_LOOKUP_FAILED"
  | "SOURCE_JOB_FAILED"
  | "POLICY_MISMATCH"
  | "IO_FAILED"
  | "PERMISSION_DENIED"
  | "DISK_FULL"
  | "UNKNOWN";

export type DownloadQueueDeleteStatus =
  | "QUEUED"
  | "RUNNING"
  | "COMPLETED"
  | "FAILED";

export type TrackedDownloadState =
  | "DOWNLOADING"
  | "IMPORT_PENDING"
  | "IMPORTING"
  | "IMPORTED"
  | "IMPORTED_SEEDING"
  | "IMPORT_BLOCKED"
  | "FAILED_PENDING"
  | "FAILED"
  | "IGNORED";

export type TrackedDownloadStatus = "OK" | "WARNING" | "ERROR";

export type DownloadSeedingState =
  | "NONE"
  | "SEEDING"
  | "GOAL_MET"
  | "HELD_PRIVATE"
  | "NEVER_REMOVE";

export type DownloadDisplayState =
  | "QUEUED"
  | "DOWNLOADING"
  | "PAUSED"
  | "POST_PROCESSING"
  | "COMPLETED"
  | "IMPORTED_SEEDING"
  | "FAILED"
  | "WARNING"
  | "IMPORTING"
  | "IMPORT_PENDING"
  | "IMPORT_BLOCKED"
  | "IMPORT_FAILED"
  | "IGNORED"
  | "REMOVING"
  | "REMOVE_FAILED";

export type DownloadActivityFilter =
  | "ALL"
  | "DOWNLOADING"
  | "QUEUED"
  | "PAUSED"
  | "POST_PROCESSING"
  | "SEEDING"
  | "WARNING";

export type DownloadImportFilter =
  | "ALL"
  | "ATTENTION"
  | "IMPORTING"
  | "PENDING"
  | "BLOCKED"
  | "FAILED";

export type DownloadActivityStatus = Exclude<DownloadActivityFilter, "ALL">;
export type DownloadImportStatus = Exclude<DownloadImportFilter, "ALL" | "ATTENTION">;
export type ActivitySortKey = "TITLE" | "CLIENT" | "STATUS" | "PROGRESS" | "SIZE";
export type SortDirection = "ASC" | "DESC";
export type SortConfig = {
  key: ActivitySortKey;
  direction: SortDirection;
};

export type TitleMatchType =
  | "SUBMISSION"
  | "CLIENT_PARAMETER"
  | "TITLE_PARSE"
  | "ID_ONLY"
  | "UNMATCHED";

export type DownloadQueueItem = {
  id: string;
  titleId: string | null;
  episodeId: string | null;
  titleName: string;
  facet: string | null;
  isScryerOrigin: boolean;
  sourceProvider: string | null;
  clientId: string;
  clientName: string;
  clientType: string;
  state: DownloadQueueState;
  displayState: DownloadDisplayState;
  progressPercent: number;
  importTransferPhase: "EXTRACTING" | "COPYING" | "FINALIZING" | null;
  importTransferBytes: number | null;
  importTransferTotalBytes: number | null;
  importTransferStartedAt: string | null;
  importTransferUpdatedAt: string | null;
  sizeBytes: number | null;
  remainingSeconds: number | null;
  queuedAt: string | null;
  lastUpdatedAt: string | null;
  attentionRequired: boolean;
  attentionReason: string | null;
  downloadClientItemId: string;
  downloadId: string | null;
  importStatus: ImportStatus | null;
  importErrorCode: ImportErrorCode | null;
  importErrorMessage: string | null;
  importedAt: string | null;
  deleteStatus: DownloadQueueDeleteStatus | null;
  deleteErrorMessage: string | null;
  trackedState: TrackedDownloadState | null;
  trackedStatus: TrackedDownloadStatus | null;
  trackedStatusMessages: string[];
  trackedMatchType: TitleMatchType | null;
  // Seeding progress. Every queue document selects these, so they are always
  // present on the wire; each is nullable because the observation, the goal and
  // the private flag are independently unknowable. `null` means "not observed"
  // and must never be rendered as zero, and `isPrivate: null` never means public.
  seedingState: DownloadSeedingState | null;
  seedRatio: number | null;
  seedRatioGoal: number | null;
  seedTimeSeconds: number | null;
  seedTimeGoalSeconds: number | null;
  isPrivate: boolean | null;
  queueScope: ReleaseQueueScope | null;
};

export type ActiveImportStream = {
  id: string;
  importId: string;
  libraryId: string;
  facet: string;
  sourcePath: string;
  destinationPath: string;
  phase: "QUEUED" | "EXTRACTING" | "PLACING" | "COPYING" | "FINALIZING";
  bytes: number;
  totalBytes: number;
  queuedAt: string;
  startedAt: string | null;
  updatedAt: string;
  cancellable: boolean;
  cancellationRequested: boolean;
};

export type DownloadHistoryPage = {
  items: DownloadQueueItem[];
  hasMore: boolean;
  totalCount: number;
  availableClients: DownloadClientFilterOption[];
};

export type DownloadImportPage = {
  items: DownloadQueueItem[];
  hasMore: boolean;
  totalCount: number;
};

export type DownloadClientFilterOption = {
  clientId: string;
  clientName: string;
  clientType: string;
};

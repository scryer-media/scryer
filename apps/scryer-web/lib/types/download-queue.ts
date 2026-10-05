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

export type HeldImportSourcesReason =
  | "SUBTITLES_PENDING"
  | "SOURCE_CLEANUP_INCOMPLETE"
  | "UNKNOWN"
  | "ARCHIVE_EXTRACTION_FAILED";

export type HeldDownloadClientPolicy = "REMOVES" | "REMOVES_AFTER_SEEDING" | "KEEPS" | "UNKNOWN";

export type HeldImportSourcesSettlement =
  | "IMPORTED"
  | "AWAITING_IMPORT"
  | "UNCHANGED"
  | "UNTRACKED"
  | "UNPROVEN"
  | "NOT_SETTLED";

export type HeldWorkspacePreservedReason =
  | "NOT_OWNED"
  | "UNSAFE"
  | "HOLDS_UNIMPORTED_VIDEO"
  | "IN_USE"
  | "UNVERIFIED"
  | "REMOVAL_FAILED"
  | "DOWNLOAD_NOT_IMPORTED";

/** Held sources of a download: every import and title a release covers. */
export type HeldImportSources = {
  importId: string;
  importIds: string[];
  titleNames: string[];
  reason: HeldImportSourcesReason;
  clientPolicy: HeldDownloadClientPolicy;
};

/** Outcome of releasing a download's held sources. */
export type HeldImportSourcesReleased = {
  importId: string;
  releasedImportIds: string[];
  settlement: HeldImportSourcesSettlement;
  clientPolicy: HeldDownloadClientPolicy;
  workspaceRemoved: boolean;
  workspacesRemoved: number;
  preservedWorkspaceReasons: HeldWorkspacePreservedReason[];
  workspaceLookupIncomplete: boolean;
};

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
  importTransferPhase: "WAITING" | "EXTRACTING" | "COPYING" | "VERIFYING" | "FINALIZING" | null;
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
  passwordFailureCode?: "archive_password_required" | "archive_password_or_corruption" | null;
  passwordRetryImportId?: string | null;
  /** Held sources of this download that the viewer may release. */
  heldImportSources?: HeldImportSources | null;
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
  /**
   * Which import actions this download offers. The server decides once, so the
   * activity rows, the dashboard rows and the title overviews cannot each
   * answer it differently for the same download.
   */
  importActions: DownloadImportActions;
};

export type DownloadImportActions = {
  /** Manual import that opens the file-mapping dialog first (series, anime). */
  manualImportInteractive: boolean;
  /** Manual import that runs without the dialog (movie). */
  manualImportDirect: boolean;
  assignTitle: boolean;
  ignore: boolean;
  markFailed: boolean;
};

export type ActiveImportStream = {
  id: string;
  importId: string;
  libraryId: string;
  facet: string;
  sourcePath: string;
  destinationPath: string;
  phase: "QUEUED" | "EXTRACTING" | "PLACING" | "COPYING" | "VERIFYING" | "FINALIZING";
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

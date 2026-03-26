export type DownloadQueueState =
  | "queued"
  | "downloading"
  | "verifying"
  | "repairing"
  | "extracting"
  | "paused"
  | "completed"
  | "import_pending"
  | "failed";

export type ImportStatus =
  | "pending"
  | "running"
  | "processing"
  | "completed"
  | "failed"
  | "skipped";

export type TrackedDownloadState =
  | "downloading"
  | "import_pending"
  | "importing"
  | "imported"
  | "import_blocked"
  | "failed_pending"
  | "failed"
  | "ignored";

export type TrackedDownloadStatus = "ok" | "warning" | "error";

export type TitleMatchType =
  | "submission"
  | "client_parameter"
  | "title_parse"
  | "id_only"
  | "unmatched";

export type DownloadQueueItem = {
  id: string;
  titleId: string | null;
  titleName: string;
  facet: string | null;
  isScryerOrigin: boolean;
  clientId: string;
  clientName: string;
  clientType: string;
  state: DownloadQueueState;
  progressPercent: number;
  sizeBytes: string | null;
  remainingSeconds: number | null;
  queuedAt: string | null;
  lastUpdatedAt: string | null;
  attentionRequired: boolean;
  attentionReason: string | null;
  downloadClientItemId: string;
  importStatus: ImportStatus | null;
  importErrorMessage: string | null;
  importedAt: string | null;
  trackedState: TrackedDownloadState | null;
  trackedStatus: TrackedDownloadStatus | null;
  trackedStatusMessages: string[];
  trackedMatchType: TitleMatchType | null;
};

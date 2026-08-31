import type { Facet } from "./titles";

export type ImportType =
  | "MOVIE_DOWNLOAD"
  | "SERIES_DOWNLOAD"
  | "MANUAL_IMPORT"
  | "RENAME_PREVIEW"
  | "RENAME_APPLY_TITLE"
  | "RENAME_APPLY_FACET"
  | "RENAME_APPLY_RESULT"
  | "RENAME_IO_FAILED"
  | "RENAME_MOVE"
  | "RENAME_STALE_PLAN";

export type ImportRecordStatus =
  | "PENDING"
  | "RUNNING"
  | "PROCESSING"
  | "COMPLETED"
  | "FAILED"
  | "SKIPPED";

export type ImportDecision =
  | "IMPORTED"
  | "REJECTED"
  | "SKIPPED"
  | "CONFLICT"
  | "UNMATCHED"
  | "FAILED";

export type ImportSkipReason =
  | "ALREADY_IMPORTED"
  | "DUPLICATE_FILE"
  | "POST_DOWNLOAD_RULE_BLOCKED"
  | "POLICY_MISMATCH"
  | "UNRESOLVED_IDENTITY"
  | "UNPARSEABLE_EPISODE"
  | "NO_VIDEO_FILES"
  | "DOWNLOAD_IN_PROGRESS"
  | "DISK_FULL"
  | "PERMISSION_DENIED"
  | "PASSWORD_REQUIRED";

export type ImportRecord = {
  id: string;
  sourceSystem: string;
  sourceRef: string;
  sourceTitle: string | null;
  facet: Facet | null;
  importType: ImportType;
  status: ImportRecordStatus;
  errorMessage: string | null;
  decision: ImportDecision | null;
  skipReason: ImportSkipReason | null;
  titleId: string | null;
  sourcePath: string | null;
  destPath: string | null;
  startedAt: string | null;
  finishedAt: string | null;
  createdAt: string;
};

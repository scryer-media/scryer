export type ImportType =
  | "movie_download"
  | "tv_download"
  | "rename_preview"
  | "rename_apply_title"
  | "rename_apply_facet"
  | "rename_apply_result"
  | "rename_io_failed"
  | "rename_move"
  | "rename_stale_plan";

export type ImportRecordStatus =
  | "pending"
  | "running"
  | "processing"
  | "completed"
  | "failed"
  | "skipped";

export type ImportDecision =
  | "imported"
  | "rejected"
  | "skipped"
  | "conflict"
  | "unmatched"
  | "failed";

export type ImportSkipReason =
  | "already_imported"
  | "duplicate_file"
  | "post_download_rule_blocked"
  | "policy_mismatch"
  | "unresolved_identity"
  | "no_video_files"
  | "disk_full"
  | "permission_denied"
  | "password_required";

export type ImportRecord = {
  id: string;
  sourceSystem: string;
  sourceRef: string;
  sourceTitle: string | null;
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

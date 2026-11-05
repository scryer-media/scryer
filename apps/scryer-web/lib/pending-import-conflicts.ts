import type { LongValue } from "./location-operations.ts";

/** Supported conflict actions are offered only for pending movie folders. */
export function isResolvableConflict(item: {
  facet: string;
  status: string;
  reason: string;
}): boolean {
  return (
    item.facet === "MOVIE" &&
    item.status === "PENDING" &&
    item.reason === "title_already_owns_another_folder"
  );
}

export type ConflictPolicy = "ADOPT_ALL" | "KEEP_ONE";
export type ConflictLeftovers = "PRESERVE_ALL" | "MEDIA_ONLY";
export type ConflictInput = {
  pendingImportIds: string[];
  policy: ConflictPolicy;
  leftovers: ConflictLeftovers;
};
export type ConflictRow = {
  pendingImportId: string;
  titleId: string;
  titleName: string;
  survivorPath: string;
  primaryPath: string;
  moveBytes: LongValue;
  recycleBytes: LongValue;
  recycleCount: number;
  collisionRenames: number;
  collisionRenameSamples: { sourcePath: string; destinationPath: string }[];
  leftoverCount: number;
  leftoverBytes: LongValue;
  leftoverPaths: string[];
  recyclePaths: string[];
  filesSampled: boolean;
  errorReason: string | null;
  errorMessage: string | null;
};
export type ConflictPreview = {
  planFingerprint: string;
  totalCount: number;
  actionableCount: number;
  failedCount: number;
  sampled: boolean;
  rows: ConflictRow[];
  permanentDeleteCount: number;
  permanentDeleteBytes: LongValue;
  requiresPermanentDeleteConfirmation: boolean;
  permanentDeletePaths: string[];
  permanentDeleteSampled: boolean;
};
export type ConflictTask = {
  jobRunId: string;
  totalCount: number;
  settledCount: number;
  failedCount: number;
  operationIds: string[];
  completed: boolean;
};

export function canApplyConflictPreview(
  preview: Pick<ConflictPreview, "actionableCount"> | null,
  loading: boolean,
  applying: boolean,
): boolean {
  return !loading && !applying && (preview?.actionableCount ?? 0) > 0;
}

export const previewPendingImportConflictsMutation = `mutation PreviewPendingImportConflicts($input: PendingImportConflictInput!) {
  previewPendingImportConflicts(input: $input) {
    planFingerprint totalCount actionableCount failedCount sampled permanentDeleteCount permanentDeleteBytes
    requiresPermanentDeleteConfirmation permanentDeletePaths permanentDeleteSampled
    rows { pendingImportId titleId titleName survivorPath primaryPath moveBytes recycleBytes recycleCount collisionRenames leftoverCount leftoverBytes leftoverPaths recyclePaths filesSampled errorReason errorMessage collisionRenameSamples { sourcePath destinationPath } }
  }
}`;

const taskFields = `jobRunId totalCount settledCount failedCount operationIds completed`;
export const resolvePendingImportConflictsMutation = `mutation ResolvePendingImportConflicts($input: ResolvePendingImportConflictsInput!) {
  resolvePendingImportConflicts(input: $input) { ${taskFields} }
}`;
export const pendingImportConflictTaskQuery = `query PendingImportConflictTask($jobRunId: ID!) {
  pendingImportConflictTask(jobRunId: $jobRunId) { ${taskFields} }
}`;

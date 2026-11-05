import type { DownloadQueueItem } from "../types/download-queue.ts";

type ProgressItem = Pick<
  DownloadQueueItem,
  | "state"
  | "displayState"
  | "importStatus"
  | "progressPercent"
  | "importTransferPhase"
  | "importTransferBytes"
  | "importTransferTotalBytes"
  | "importTransferStartedAt"
>;

export type MovieOverviewProgress = {
  percent: number;
  indeterminate: boolean;
  measuredTransfer: boolean;
  phaseKey: string;
  bytes: number | null;
  total: number | null;
  importing: boolean;
  phase: ProgressItem["importTransferPhase"];
  startedAt: ProgressItem["importTransferStartedAt"];
};

export function movieOverviewProgress(
  item: ProgressItem,
  previous?: MovieOverviewProgress,
): MovieOverviewProgress {
  const importing =
    item.displayState === "IMPORTING" ||
    item.importStatus === "PENDING" ||
    item.importStatus === "RUNNING" ||
    item.importStatus === "PROCESSING";
  const sameAttempt =
    previous?.importing &&
    (item.importTransferStartedAt === null ||
      item.importTransferStartedAt === previous.startedAt);
  const samePhase =
    item.importTransferPhase === null ||
    item.importTransferPhase === previous?.phase;
  const phase =
    importing && sameAttempt && item.importTransferPhase === null
      ? previous.phase
      : item.importTransferPhase;
  const startedAt =
    importing && sameAttempt && item.importTransferStartedAt === null
      ? previous.startedAt
      : item.importTransferStartedAt;
  const bytes = item.importTransferBytes;
  const total = item.importTransferTotalBytes;
  const measuredTransfer =
    importing &&
    (item.importTransferPhase === "COPYING" ||
      item.importTransferPhase === "VERIFYING" ||
      item.importTransferPhase === "EXTRACTING") &&
    bytes !== null &&
    Number.isFinite(bytes) &&
    bytes >= 0 &&
    total !== null &&
    Number.isFinite(total) &&
    total > 0;
  // Partial updates must not replace the last measured progress with either
  // downloader completion or an indeterminate animation.
  if (importing && !measuredTransfer && sameAttempt && samePhase && previous) {
    return previous;
  }
  // A completed client download is not completed import work. Missing transfer
  // counters, waiting and finalization have no meaningful percentage.
  const indeterminate = importing
    ? !measuredTransfer
    : item.displayState === "QUEUED" ||
      (item.displayState === "POST_PROCESSING" && item.progressPercent >= 100);
  const percent = indeterminate
    ? 0
    : measuredTransfer
      ? Math.min(100, Math.round((bytes / total) * 100))
      : Number.isFinite(item.progressPercent)
        ? Math.min(100, Math.max(0, Math.round(item.progressPercent)))
        : 0;
  // Each phase measures different work. Remount its bar instead of animating
  // from the previous phase's 100% down to the new phase's first observation.
  const phaseKey = importing
    ? `import:${startedAt ?? "pending"}:${phase ?? "pending"}`
    : `client:${item.state}:${item.displayState}`;
  return {
    bytes: measuredTransfer ? bytes : null,
    total: measuredTransfer ? total : null,
    importing,
    phase,
    startedAt,
    percent,
    indeterminate,
    measuredTransfer,
    phaseKey: `${phaseKey}:${indeterminate ? "unknown" : "measured"}`,
  };
}

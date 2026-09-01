import type { ActivitySection } from "@/components/root/types";
import type { DownloadQueueItem } from "@/lib/types";
import { useTranslate } from "@/lib/context/translate-context";
import {
  buildQueueStatusDetail,
  normalizeQueueState,
} from "@/lib/utils/download-queue";
import { manualImportActions } from "@/lib/utils/manual-import-actions";

export type TranslateFn = ReturnType<typeof useTranslate>;

export type ActivityTab = Exclude<ActivitySection, "history">;

export const queueStateClasses: Record<string, string> = {
  queued: "border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] text-[var(--scry-warning-text)]",
  downloading: "border-[var(--scry-info-border)] bg-[var(--scry-info-bg)] text-[var(--scry-info-text)]",
  post_processing: "border-[var(--scry-info-border)] bg-[var(--scry-info-bg)] text-[var(--scry-info-text)]",
  paused: "border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] text-[var(--scry-warning-text)]",
  completed: "border-[var(--scry-success-border)] bg-[var(--scry-success-bg)] text-[var(--scry-success-text)]",
  // Imported and still seeding is a healthy post-import state, so it stays in
  // the success family that `completed` uses; the stronger border is what makes
  // it legible as its own thing rather than a warning.
  imported_seeding:
    "border-[var(--scry-success-border-strong)] bg-[var(--scry-success-bg)] text-[var(--scry-success-text)]",
  importing: "border-[var(--scry-info-border)] bg-[var(--scry-info-bg)] text-[var(--scry-info-text)]",
  removing: "border-[var(--scry-info-border)] bg-[var(--scry-info-bg)] text-[var(--scry-info-text)]",
  import_pending: "border-[rgba(var(--scry-accent-rgb),0.4)] bg-[rgba(var(--scry-accent-rgb),0.1)] text-[var(--scry-accent-text)]",
  import_blocked: "border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] text-[var(--scry-warning-text)]",
  import_failed: "border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] text-[var(--scry-danger-text)]",
  ignored: "border-[var(--scry-info-border)] bg-[var(--scry-info-bg)] text-[var(--scry-info-text)]",
  remove_failed: "border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] text-[var(--scry-danger-text)]",
  // A recoverable client problem: loud enough to be noticed, not dressed as a
  // dead grab.
  warning: "border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] text-[var(--scry-warning-text)]",
  failed: "border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] text-[var(--scry-danger-text)]",
};

export const queueStateLabels: Record<string, string> = {
  queued: "queue.state.queued",
  downloading: "queue.state.downloading",
  post_processing: "queue.state.postProcessing",
  paused: "queue.state.paused",
  completed: "queue.state.completed",
  imported_seeding: "queue.state.importedSeeding",
  importing: "queue.state.importing",
  removing: "queue.deleting",
  import_pending: "queue.state.importPending",
  import_blocked: "queue.state.importBlocked",
  import_failed: "queue.manualImportFailed",
  ignored: "queue.state.ignored",
  remove_failed: "queue.removeFailed",
  warning: "queue.state.warning",
  failed: "queue.state.failed",
};

export const queueStateAttention: Record<string, boolean> = {
  warning: true,
  failed: true,
  importing: true,
  removing: true,
  import_pending: true,
  import_blocked: true,
  import_failed: true,
  remove_failed: true,
};

export function compareStrings(left: string, right: string): number {
  return left.localeCompare(right, undefined, { sensitivity: "base" });
}

export function activityStatusRank(tab: ActivityTab, displayState: string): number {
  switch (tab) {
    case "import":
      switch (displayState.toLowerCase()) {
        case "importing":
          return 0;
        case "import_pending":
          return 1;
        case "import_blocked":
          return 2;
        case "import_failed":
          return 3;
        default:
          return 99;
      }
    case "activity":
    default:
      switch (displayState.toLowerCase()) {
        case "downloading":
          return 0;
        case "queued":
          return 1;
        case "paused":
          return 2;
        case "post_processing":
          return 3;
        case "warning":
          return 4;
        default:
          return 99;
      }
  }
}

export type QueueRowPresentation = {
  stateKey: string;
  trackedStateKey: string;
  trackedMatchTypeKey: string;
  displayStateKey: string;
  /** Key the status badge renders under; normally the projected display state. */
  statusBadgeKey: string;
  percent: number;
  remainingLabel: string | null;
  hasTransferProgress: boolean;
  needsManualImport: boolean;
  statusLabel: string;
  failureReason: string;
  hasStatusDetails: boolean;
  hasExpandableDetails: boolean;
  displayTitle: string;
  releaseTitle: string;
  canPause: boolean;
  canResume: boolean;
  canAssignTitle: boolean;
  canIgnore: boolean;
  canMarkFailed: boolean;
  canInteractiveManualImport: boolean;
  canDirectManualImport: boolean;
};

export function deriveQueueRowPresentation(
  queueItem: DownloadQueueItem,
  t: TranslateFn,
): QueueRowPresentation {
  const stateKey = normalizeQueueState(queueItem.state);
  const trackedStateKey = normalizeQueueState(queueItem.trackedState);
  const trackedMatchTypeKey = normalizeQueueState(queueItem.trackedMatchType);
  const displayStateKey = queueItem.displayState;
  const statusBadgeKey = displayStateKey;
  const reportedFailureReason = buildQueueStatusDetail(queueItem);
  const facetKey = normalizeQueueState(queueItem.facet);
  const failureReason =
    reportedFailureReason || displayStateKey !== "IMPORT_BLOCKED"
      ? reportedFailureReason
      : !queueItem.titleId
        ? t("queue.blockReasonFallbackUnassigned")
        : facetKey === "series" || facetKey === "anime"
          ? t("queue.blockReasonFallbackEpisodic")
          : t("queue.blockReasonFallbackReview");
  const transferBytes = parseByteCount(queueItem.importTransferBytes);
  const transferTotalBytes = parseByteCount(queueItem.importTransferTotalBytes);
  const hasTransferProgress =
    displayStateKey === "IMPORTING" &&
    queueItem.importTransferPhase !== null &&
    transferBytes !== null &&
    transferTotalBytes !== null &&
    transferTotalBytes > 0;
  const percent = hasTransferProgress
    ? formatProgress((transferBytes / transferTotalBytes) * 100)
    : formatProgress(queueItem.progressPercent);
  const remainingLabel = hasTransferProgress
    ? `${formatByteCount(transferBytes)} / ${formatByteCount(transferTotalBytes)}`
    : formatRemainingDuration(queueItem.remainingSeconds);
  const needsManualImport =
    queueItem.attentionRequired ||
    queueStateAttention[stateKey] ||
    queueStateAttention[displayStateKey.toLowerCase()];
  const postProcessingStatusKey =
    stateKey === "verifying"
      ? "queue.state.verifying"
      : stateKey === "repairing"
        ? "queue.state.repairing"
        : stateKey === "extracting"
          ? "queue.state.extracting"
          : "queue.state.postProcessing";
  const statusLabel =
    displayStateKey === "IGNORED"
      ? t("queue.state.ignored")
      : queueItem.importTransferPhase === "EXTRACTING"
        ? t("queue.transfer.extracting")
        : queueItem.importTransferPhase === "COPYING"
        ? t("queue.transfer.copying")
        : queueItem.importTransferPhase === "FINALIZING"
          ? t("queue.transfer.finalizing")
          : displayStateKey === "POST_PROCESSING"
            ? t(postProcessingStatusKey)
            : t(queueStateLabels[statusBadgeKey.toLowerCase()] ?? "queue.state.unknown");
  const hasStatusDetails =
    (stateKey === "failed" ||
      stateKey === "warning" ||
      displayStateKey === "REMOVE_FAILED" ||
      displayStateKey === "IMPORT_BLOCKED" ||
      displayStateKey === "IMPORT_FAILED") &&
    failureReason.length > 0;
  const manualActions = manualImportActions({
    displayState: displayStateKey,
    facet: queueItem.facet,
    hasTitle: Boolean(queueItem.titleId),
  });
  const canAssignTitle =
    trackedStateKey === "import_blocked" &&
    displayStateKey !== "IMPORTING" &&
    displayStateKey !== "REMOVING";
  const canIgnore =
    (trackedStateKey === "import_blocked" || displayStateKey === "IMPORT_FAILED") &&
    displayStateKey !== "IMPORTING" &&
    displayStateKey !== "REMOVING";
  const canMarkFailed =
    (trackedStateKey === "import_blocked" ||
      trackedStateKey === "import_pending" ||
      trackedStateKey === "failed_pending") &&
    displayStateKey !== "IMPORTING" &&
    displayStateKey !== "REMOVING";
  const releaseTitle =
    queueItem.titleName.trim() || queueItem.downloadClientItemId.trim() || "—";
  const displayTitle = releaseTitle;
  const hasExpandableDetails =
    (displayStateKey === "IMPORT_BLOCKED" ||
      displayStateKey === "IMPORT_FAILED" ||
      displayStateKey === "REMOVE_FAILED" ||
      // The client's message is the whole point of a warning: it is what tells
      // the operator which recoverable problem to go and fix.
      displayStateKey === "WARNING") &&
    (failureReason.length > 0 || releaseTitle !== "—");

  return {
    stateKey,
    trackedStateKey,
    trackedMatchTypeKey,
    displayStateKey,
    statusBadgeKey,
    percent,
    remainingLabel,
    hasTransferProgress,
    needsManualImport,
    statusLabel,
    failureReason,
    hasStatusDetails,
    hasExpandableDetails,
    displayTitle,
    releaseTitle,
    canPause: stateKey === "downloading" || stateKey === "queued",
    canResume: stateKey === "paused",
    canAssignTitle,
    canIgnore,
    canMarkFailed,
    canInteractiveManualImport: manualActions.interactive,
    canDirectManualImport: manualActions.direct,
  };
}

export function downloadQueueItemRowSelectorKey(
  queueItem: DownloadQueueItem,
  fallbackKey: string,
): string {
  if (queueItem.downloadId?.trim()) {
    return queueItem.downloadId.trim();
  }

  const ownerKey = queueItem.clientId.trim() || queueItem.clientType.trim();
  const itemKey = queueItem.downloadClientItemId.trim() || queueItem.id.trim();
  const queuedAt = queueItem.queuedAt?.trim();
  const selectorParts = [ownerKey, itemKey, queuedAt].filter(Boolean);
  return selectorParts.length >= 2 ? selectorParts.join("::") : fallbackKey;
}

export function canIgnoreImportItem(queueItem: DownloadQueueItem): boolean {
  const trackedStateKey = normalizeQueueState(queueItem.trackedState);
  const displayStateKey = normalizeQueueState(queueItem.displayState);
  return (
    (trackedStateKey === "import_blocked" || displayStateKey === "import_failed") &&
    displayStateKey !== "importing" &&
    displayStateKey !== "removing"
  );
}

export function canDeleteImportItem(queueItem: DownloadQueueItem): boolean {
  const displayStateKey = normalizeQueueState(queueItem.displayState);
  return displayStateKey !== "importing" && displayStateKey !== "removing";
}

export function parseByteCount(sizeBytes: number | string | null): number | null {
  if (sizeBytes === null || sizeBytes === "") {
    return null;
  }
  const bytes = typeof sizeBytes === "number" ? sizeBytes : Number.parseFloat(sizeBytes);
  if (!Number.isFinite(bytes) || bytes < 0) {
    return null;
  }
  return bytes;
}

export function formatByteCount(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) {
    return "—";
  }
  if (bytes === 0) {
    return "0 B";
  }
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  let value = bytes;
  let index = 0;
  while (value >= 1024 && index < units.length - 1) {
    value /= 1024;
    index += 1;
  }
  return `${value.toFixed(value >= 10 || index === 0 ? 0 : 1)} ${units[index]}`;
}

export function formatBytes(sizeBytes: number | string | null): string {
  const bytes = parseByteCount(sizeBytes);
  return bytes === null ? "—" : formatByteCount(bytes);
}

export function formatProgress(progressPercent: number): number {
  if (!Number.isFinite(progressPercent)) {
    return 0;
  }
  if (progressPercent < 0) {
    return 0;
  }
  if (progressPercent > 100) {
    return 100;
  }
  return Math.round(progressPercent);
}

export function effectiveQueueItemProgress(queueItem: DownloadQueueItem): number {
  const transferBytes = parseByteCount(queueItem.importTransferBytes);
  const transferTotalBytes = parseByteCount(queueItem.importTransferTotalBytes);
  if (
    queueItem.displayState === "IMPORTING" &&
    queueItem.importTransferPhase !== null &&
    transferBytes !== null &&
    transferTotalBytes !== null &&
    transferTotalBytes > 0
  ) {
    return formatProgress((transferBytes / transferTotalBytes) * 100);
  }
  return formatProgress(queueItem.progressPercent);
}

export function formatRemainingDuration(remainingSeconds: number | null): string | null {
  if (remainingSeconds === null || !Number.isFinite(remainingSeconds)) {
    return null;
  }
  const totalSeconds = Math.max(0, Math.floor(remainingSeconds));
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;
  if (hours > 0) {
    return `${hours}:${minutes.toString().padStart(2, "0")}:${seconds
      .toString()
      .padStart(2, "0")}`;
  }
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
}

export function getProgressBarColor(stateKey: string): string {
  switch (stateKey.toLowerCase()) {
    case "completed":
      return "bg-[var(--scry-success-solid)]";
    case "failed":
    case "remove_failed":
      return "bg-[var(--scry-danger-solid)]";
    case "paused":
    case "warning":
      return "bg-[var(--scry-warning-solid)]";
    case "import_pending":
      return "bg-[rgb(var(--scry-accent-rgb))]";
    case "downloading":
    case "removing":
      return "bg-[var(--scry-info-solid)]";
    case "post_processing":
      return "bg-[var(--scry-info-solid)]";
    case "queued":
      return "bg-gray-400";
    default:
      return "bg-muted-foreground";
  }
}


import {
  ArrowDownToLine,
  ChevronDown,
  ChevronUp,
  CircleOff,
  Link2,
  Loader2,
  Pause,
  Play,
  Trash2,
} from "lucide-react";
import { Fragment, type UIEvent, useCallback, useRef, useState } from "react";

import { Button } from "@/components/ui/button";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { ActivityProgressBar } from "@/components/views/activity-progress-bar";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import type { DownloadQueueItem } from "@/lib/types";
import { useTranslate } from "@/lib/context/translate-context";
import { useIsMobile } from "@/lib/hooks/use-mobile";

type TranslateFn = ReturnType<typeof useTranslate>;

type QueueMode = "scryer" | "all" | "history";

type ActivityViewState = {
  queueItems: DownloadQueueItem[];
  queueLoading: boolean;
  queueError: string | null;
  lastRefreshedAt: Date | null;
  requestManualImport: (item: DownloadQueueItem) => Promise<void>;
  requestAssignTitle: (item: DownloadQueueItem) => Promise<void>;
  requestIgnore: (item: DownloadQueueItem) => Promise<void>;
  requestPause: (item: DownloadQueueItem) => Promise<void>;
  requestResume: (item: DownloadQueueItem) => Promise<void>;
  requestDelete: (item: DownloadQueueItem) => Promise<void>;
  queueMode: QueueMode;
  setQueueMode: (queueMode: QueueMode) => void;
  historyHasMore: boolean;
  historyLoadingMore: boolean;
  requestMoreHistory: () => Promise<void>;
};

const queueStateClasses: Record<string, string> = {
  queued: "border-amber-500/40 bg-amber-500/10 text-amber-200",
  downloading: "border-sky-500/40 bg-sky-500/10 text-sky-200",
  post_processing: "border-cyan-500/40 bg-cyan-500/10 text-cyan-200",
  paused: "border-purple-500/40 bg-purple-500/10 text-purple-200",
  completed: "border-emerald-500/40 bg-emerald-500/15 dark:bg-emerald-500/10 text-emerald-700 dark:text-emerald-200",
  import_pending: "border-indigo-500/40 bg-indigo-500/10 text-indigo-200",
  import_blocked: "border-amber-500/40 bg-amber-500/10 text-amber-200",
  failed: "border-rose-500/40 bg-rose-500/10 text-rose-200",
};

const queueStateLabels: Record<string, string> = {
  queued: "queue.state.queued",
  downloading: "queue.state.downloading",
  post_processing: "queue.state.postProcessing",
  paused: "queue.state.paused",
  completed: "queue.state.completed",
  import_pending: "queue.state.importPending",
  import_blocked: "queue.state.importBlocked",
  failed: "queue.state.failed",
};

const queueStateAttention: Record<string, boolean> = {
  failed: true,
  import_pending: true,
  import_blocked: true,
};

type QueueRowPresentation = {
  stateKey: string;
  trackedStateKey: string;
  trackedMatchTypeKey: string;
  displayStateKey: string;
  percent: number;
  remainingLabel: string | null;
  needsManualImport: boolean;
  statusLabel: string;
  failureReason: string;
  hasStatusDetails: boolean;
  hasExpandableDetails: boolean;
  releaseTitle: string;
  canPause: boolean;
  canResume: boolean;
  canAssignTitle: boolean;
  canIgnore: boolean;
  canInteractiveManualImport: boolean;
  canDirectManualImport: boolean;
};

function normalizeQueueState(state: string | null | undefined): string {
  return (state ?? "").trim().toLowerCase();
}

function buildStatusDetail(queueItem: DownloadQueueItem): string {
  const messages = [
    ...(queueItem.trackedStatusMessages ?? []),
    queueItem.attentionReason,
    queueItem.importErrorMessage,
  ]
    .map((value) => value?.trim())
    .filter((value): value is string => Boolean(value));

  return Array.from(new Set(messages)).join("\n");
}

function isPostProcessingReason(reason: string | null | undefined): boolean {
  if (!reason) return false;
  const normalized = reason.toUpperCase();
  return (
    normalized.includes("PP_QUEUED") ||
    normalized.includes("POSTPROCESSING") ||
    normalized.includes("UNPACKING") ||
    normalized.includes("REPAIRING") ||
    normalized.includes("VERIFYING") ||
    normalized.includes("RENAMING") ||
    normalized.includes("MOVING") ||
    normalized.includes("EXECUTING_SCRIPT")
  );
}

function deriveDisplayState(
  queueItem: DownloadQueueItem,
  stateKey: string,
  trackedStateKey: string,
  failureReason: string,
): string {
  if (trackedStateKey === "import_blocked" || trackedStateKey === "import_pending") {
    return trackedStateKey;
  }

  const importStatusKey = normalizeQueueState(queueItem.importStatus);
  const canDeriveBlockedState =
    trackedStateKey.length === 0 &&
    failureReason.length > 0 &&
    (stateKey === "completed" || stateKey === "import_pending" || stateKey === "failed") &&
    (importStatusKey === "skipped" || importStatusKey === "failed");
  if (canDeriveBlockedState) {
    return "import_blocked";
  }

  const stateKeyValue = stateKey;
  if (
    stateKeyValue === "extracting" ||
    stateKeyValue === "verifying" ||
    stateKeyValue === "repairing"
  ) {
    return "post_processing";
  }
  if (
    stateKeyValue === "downloading" &&
    isPostProcessingReason(queueItem.attentionReason)
  ) {
    return "post_processing";
  }
  return stateKeyValue;
}

function deriveQueueRowPresentation(
  queueItem: DownloadQueueItem,
  t: TranslateFn,
): QueueRowPresentation {
  const stateKey = normalizeQueueState(queueItem.state);
  const trackedStateKey = normalizeQueueState(queueItem.trackedState);
  const trackedMatchTypeKey = normalizeQueueState(queueItem.trackedMatchType);
  const failureReason = buildStatusDetail(queueItem);
  const displayStateKey = deriveDisplayState(
    queueItem,
    stateKey,
    trackedStateKey,
    failureReason,
  );
  const percent = formatProgress(queueItem.progressPercent);
  const remainingLabel = formatRemainingDuration(queueItem.remainingSeconds);
  const needsManualImport =
    queueItem.attentionRequired ||
    queueStateAttention[stateKey] ||
    queueStateAttention[displayStateKey];
  const stageLabel =
    queueItem.attentionReason?.trim() ??
    queueItem.trackedStatusMessages[0]?.trim() ??
    "";
  const statusLabel =
    displayStateKey === "post_processing" && stageLabel.length > 0
      ? stageLabel
      : t(queueStateLabels[displayStateKey] ?? "queue.state.unknown");
  const hasStatusDetails =
    (stateKey === "failed" || displayStateKey === "import_blocked") &&
    failureReason.length > 0;
  const isCompleted = stateKey === "completed" || stateKey === "import_pending";
  const canAssignTitle = trackedStateKey === "import_blocked";
  const canIgnore = trackedStateKey === "import_blocked";
  const canInteractiveManualImport =
    Boolean(queueItem.titleId) &&
    (queueItem.facet === "tv" || queueItem.facet === "anime") &&
    trackedStateKey === "import_blocked";
  const canDirectManualImport =
    Boolean(queueItem.titleId) &&
    ((isCompleted && needsManualImport) ||
      (trackedStateKey === "import_blocked" && queueItem.facet === "movie"));
  const releaseTitle =
    queueItem.titleName.trim() || queueItem.downloadClientItemId.trim() || "\u2014";
  const hasExpandableDetails =
    displayStateKey === "import_blocked" &&
    (failureReason.length > 0 || releaseTitle !== "\u2014");

  return {
    stateKey,
    trackedStateKey,
    trackedMatchTypeKey,
    displayStateKey,
    percent,
    remainingLabel,
    needsManualImport,
    statusLabel,
    failureReason,
    hasStatusDetails,
    hasExpandableDetails,
    releaseTitle,
    canPause: stateKey === "downloading" || stateKey === "queued",
    canResume: stateKey === "paused",
    canAssignTitle,
    canIgnore,
    canInteractiveManualImport,
    canDirectManualImport,
  };
}

function ActivityQueueStatusBadge({
  stateKey,
  statusLabel,
  isExpandable,
  isExpanded,
  detailId,
  expandLabel,
  onToggle,
}: {
  stateKey: string;
  statusLabel: string;
  isExpandable: boolean;
  isExpanded: boolean;
  detailId: string;
  expandLabel: string;
  onToggle: () => void;
}) {
  const className = `inline-flex items-center gap-1.5 rounded border px-2 py-1 text-xs font-medium ${queueStateClasses[stateKey] ?? "border-border bg-muted text-card-foreground"}`;

  if (!isExpandable) {
    return <span className={className}>{statusLabel}</span>;
  }

  return (
    <button
      type="button"
      className={className}
      aria-expanded={isExpanded}
      aria-controls={detailId}
      aria-label={`${statusLabel}. ${expandLabel}`}
      onClick={onToggle}
    >
      <span>{statusLabel}</span>
      {isExpanded ? (
        <ChevronUp className="h-3.5 w-3.5 opacity-80" aria-hidden="true" />
      ) : (
        <ChevronDown className="h-3.5 w-3.5 opacity-80" aria-hidden="true" />
      )}
    </button>
  );
}

function ActivityQueueDetailsPanel({
  detailId,
  releaseTitle,
  failureReason,
  t,
}: {
  detailId: string;
  releaseTitle: string;
  failureReason: string;
  t: TranslateFn;
}) {
  return (
    <div
      id={detailId}
      className="rounded-lg border border-amber-500/25 bg-amber-500/5 p-3"
    >
      <div className="grid gap-4 md:grid-cols-2">
        <div>
          <p className="text-[11px] font-semibold uppercase tracking-[0.16em] text-muted-foreground">
            {t("queue.releaseTitle")}
          </p>
          <p className="mt-1 break-words text-sm text-foreground">{releaseTitle}</p>
        </div>
        <div>
          <p className="text-[11px] font-semibold uppercase tracking-[0.16em] text-muted-foreground">
            {t("queue.blockReason")}
          </p>
          <p className="mt-1 whitespace-pre-wrap break-words text-sm text-foreground">
            {failureReason || "\u2014"}
          </p>
        </div>
      </div>
    </div>
  );
}

function formatBytes(sizeBytes: string | null): string {
  if (!sizeBytes) {
    return "\u2014";
  }
  const bytes = Number.parseFloat(sizeBytes);
  if (!Number.isFinite(bytes) || bytes < 0) {
    return "\u2014";
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

function formatProgress(progressPercent: number): number {
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

function formatRemainingDuration(remainingSeconds: number | null): string | null {
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

function getProgressBarColor(stateKey: string): string {
  switch (stateKey) {
    case "completed":
      return "bg-emerald-500";
    case "failed":
      return "bg-rose-500";
    case "paused":
      return "bg-amber-500";
    case "import_pending":
      return "bg-indigo-500";
    case "downloading":
      return "bg-sky-500";
    case "post_processing":
      return "bg-cyan-500";
    case "queued":
      return "bg-gray-400";
    default:
      return "bg-muted-foreground";
  }
}


export function ActivityView({ state }: { state: ActivityViewState }) {
  const t = useTranslate();
  const isMobile = useIsMobile();
  const {
    queueItems,
    queueLoading,
    queueError,
    requestManualImport,
    requestAssignTitle,
    requestIgnore,
    requestPause,
    requestResume,
    requestDelete,
    queueMode,
    setQueueMode,
    historyHasMore,
    historyLoadingMore,
    requestMoreHistory,
  } = state;
  const [manualImportingId, setManualImportingId] = useState<string | null>(null);
  const [actionLoadingId, setActionLoadingId] = useState<string | null>(null);
  const [deleteConfirmItem, setDeleteConfirmItem] = useState<DownloadQueueItem | null>(null);
  const [deleteInProgress, setDeleteInProgress] = useState(false);
  const [rowActionBusy, setRowActionBusy] = useState<Record<string, true>>({});
  const [expandedItemIds, setExpandedItemIds] = useState<Record<string, true>>({});
  const rowActionBusyRef = useRef<Record<string, true>>({});
  const scrollHeightClass = isMobile ? "max-h-[70vh]" : "max-h-[1700px]";

  const setRowBusy = useCallback((rowId: string, busy: boolean) => {
    rowActionBusyRef.current = busy
      ? { ...rowActionBusyRef.current, [rowId]: true }
      : Object.fromEntries(
          Object.entries(rowActionBusyRef.current).filter(([id]) => id !== rowId),
        );
    setRowActionBusy((current) => {
      if (!busy) {
        const { [rowId]: _removed, ...next } = current;
        return next;
      }
      if (current[rowId]) {
        return current;
      }
      return {
        ...current,
        [rowId]: true,
      };
    });
  }, []);

  const handleDelete = useCallback(async () => {
    if (!deleteConfirmItem) return;
    setRowBusy(deleteConfirmItem.id, true);
    setDeleteInProgress(true);
    try {
      await requestDelete(deleteConfirmItem);
    } finally {
      setDeleteInProgress(false);
      setRowBusy(deleteConfirmItem.id, false);
      setDeleteConfirmItem(null);
    }
  }, [deleteConfirmItem, requestDelete, setRowBusy]);

  const toggleExpandedDetails = useCallback((rowId: string) => {
    setExpandedItemIds((current) => {
      if (current[rowId]) {
        const { [rowId]: _removed, ...next } = current;
        return next;
      }

      return {
        ...current,
        [rowId]: true,
      };
    });
  }, []);

  const handleResultsScroll = useCallback(
    (event: UIEvent<HTMLDivElement>) => {
      if (
        queueMode !== "history" ||
        historyLoadingMore ||
        !historyHasMore ||
        queueLoading
      ) {
        return;
      }

      const element = event.currentTarget;
      if (element.scrollHeight - element.scrollTop - element.clientHeight <= 160) {
        void requestMoreHistory();
      }
    },
    [historyHasMore, historyLoadingMore, queueLoading, queueMode, requestMoreHistory],
  );

  return (
    <>
      <ConfirmDialog
        open={deleteConfirmItem !== null}
        title={t("queue.deleteConfirmTitle")}
        description={t("queue.deleteConfirmDescription")}
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        isBusy={deleteInProgress}
        onConfirm={handleDelete}
        onCancel={() => {
          if (deleteConfirmItem) {
            setRowBusy(deleteConfirmItem.id, false);
          }
          setDeleteConfirmItem(null);
        }}
      />
      <Card>
        <CardHeader className="space-y-3">
          <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
            <CardTitle>{t("activity.title")}</CardTitle>
            <div className="overflow-x-auto">
              <ToggleGroup
                type="single"
                value={queueMode}
                onValueChange={(nextValue) => {
                  if (
                    nextValue === "scryer" ||
                    nextValue === "all" ||
                    nextValue === "history"
                  ) {
                    setQueueMode(nextValue);
                    return;
                  }
                  setQueueMode("scryer");
                }}
                aria-label={t("activity.filterToggleLabel")}
                size="lg"
                className="h-14 min-w-max rounded-xl border-0 bg-card/80 divide-x divide-border/40"
              >
                <ToggleGroupItem
                  value="scryer"
                  size="lg"
                  className="h-full min-w-28 rounded-none px-4 text-sm font-semibold sm:min-w-36 sm:px-6 sm:text-base first:rounded-l-xl last:rounded-r-xl data-[state=off]:bg-accent/80 data-[state=off]:text-foreground data-[state=off]:hover:bg-accent/80 data-[state=on]:bg-primary data-[state=on]:text-primary-foreground data-[state=on]:border-0 data-[state=on]:shadow-none"
                >
                  {t("activity.scryerOnly")}
                </ToggleGroupItem>
                <ToggleGroupItem
                  value="all"
                  size="lg"
                  className="h-full min-w-28 rounded-none px-4 text-sm font-semibold sm:min-w-36 sm:px-6 sm:text-base first:rounded-l-xl last:rounded-r-xl data-[state=off]:bg-accent/80 data-[state=off]:text-foreground data-[state=off]:hover:bg-accent/80 data-[state=on]:bg-primary data-[state=on]:text-primary-foreground data-[state=on]:border-0 data-[state=on]:shadow-none"
                >
                  {t("activity.allActivity")}
                </ToggleGroupItem>
                <ToggleGroupItem
                  value="history"
                  size="lg"
                  className="h-full min-w-24 rounded-none px-4 text-sm font-semibold sm:min-w-28 sm:px-6 sm:text-base first:rounded-l-xl last:rounded-r-xl data-[state=off]:bg-accent/80 data-[state=off]:text-foreground data-[state=off]:hover:bg-accent/80 data-[state=on]:bg-primary data-[state=on]:text-primary-foreground data-[state=on]:border-0 data-[state=on]:shadow-none"
                >
                  {t("activity.history")}
                </ToggleGroupItem>
              </ToggleGroup>
            </div>
          </div>
        </CardHeader>
        <CardContent className="space-y-4">

          {queueLoading && queueItems.length === 0 ? <p>{t("label.loading")}</p> : null}
          {queueError ? (
            <p className="rounded border border-rose-500/40 bg-rose-950/40 p-2 text-sm text-rose-200">
              {queueError}
            </p>
          ) : null}

          {isMobile ? (
            queueItems.length === 0 && !queueLoading ? (
              <p className="text-sm text-muted-foreground">{t("queue.empty")}</p>
            ) : (
              <div
                onScroll={handleResultsScroll}
                className={`${scrollHeightClass} overflow-y-auto pr-1`}
              >
                <div className="space-y-3">
                  {queueItems.map((queueItem) => {
                    const row = deriveQueueRowPresentation(queueItem, t);
                    const manualImportPending = manualImportingId === queueItem.id;
                    const isActionLoading = actionLoadingId === queueItem.id;
                    const isRowBusy = rowActionBusy[queueItem.id] ?? rowActionBusyRef.current[queueItem.id] ?? false;
                    const isRowBlocked = isRowBusy || manualImportPending || isActionLoading;
                    const isDeleteConfirming = deleteConfirmItem?.id === queueItem.id;
                    const isRowFullyBusy = isRowBlocked || isDeleteConfirming;
                    const isExpanded = Boolean(expandedItemIds[queueItem.id]);
                    const detailId = `activity-queue-details-${queueItem.id}`;
                    const rowActionVisualClass = isRowFullyBusy
                      ? "pointer-events-none opacity-45 grayscale"
                      : "";

                    return (
                      <div key={queueItem.id} className="rounded-xl border border-border bg-card/40 p-3">
                        <div className="flex items-start justify-between gap-3">
                          <div className="min-w-0 flex-1">
                            <p className="break-words text-sm font-medium text-foreground">
                              {queueItem.titleName || "\u2014"}
                            </p>
                            <p className="mt-1 text-xs text-muted-foreground">
                              {queueItem.clientName || queueItem.clientType} • {queueItem.clientType}
                            </p>
                          </div>
                          <div className="shrink-0">
                            <ActivityQueueStatusBadge
                              stateKey={row.displayStateKey}
                              statusLabel={row.statusLabel}
                              isExpandable={row.hasExpandableDetails}
                              isExpanded={isExpanded}
                              detailId={detailId}
                              expandLabel={t(
                                isExpanded ? "queue.hideDetails" : "queue.showDetails",
                              )}
                              onToggle={() => toggleExpandedDetails(queueItem.id)}
                            />
                          </div>
                        </div>
                        {queueItem.importErrorMessage && !row.hasStatusDetails ? (
                          <p className="mt-2 break-words text-xs text-rose-400">{queueItem.importErrorMessage}</p>
                        ) : null}
                        {row.hasExpandableDetails && isExpanded ? (
                          <div className="mt-3">
                            <ActivityQueueDetailsPanel
                              detailId={detailId}
                              releaseTitle={row.releaseTitle}
                              failureReason={row.failureReason}
                              t={t}
                            />
                          </div>
                        ) : null}
                        <div className="mt-3">
                          <ActivityProgressBar
                            percent={row.percent}
                            remainingLabel={row.remainingLabel}
                            colorClass={getProgressBarColor(row.displayStateKey)}
                          />
                        </div>
                        <div className="mt-3 flex items-center justify-between text-xs text-muted-foreground">
                          <span>{formatBytes(queueItem.sizeBytes)}</span>
                        </div>
                        <div className="mt-3 flex flex-wrap gap-2">
                          {row.canPause && (
                            <Button
                              type="button"
                              size="sm"
                              variant="secondary"
                              className={`flex-1 ${rowActionVisualClass}`}
                              disabled={isRowFullyBusy}
                              onClick={() => {
                                if (rowActionBusyRef.current[queueItem.id] || isActionLoading || isRowBlocked) {
                                  return;
                                }
                                setActionLoadingId(queueItem.id);
                                setRowBusy(queueItem.id, true);
                                void requestPause(queueItem).finally(() => {
                                  setRowBusy(queueItem.id, false);
                                  setActionLoadingId((c) => (c === queueItem.id ? null : c));
                                });
                              }}
                            >
                              <Pause className="h-4 w-4" />
                              <span>{t("queue.pause")}</span>
                            </Button>
                          )}
                          {row.canResume && (
                            <Button
                              type="button"
                              size="sm"
                              variant="secondary"
                              className={`flex-1 ${rowActionVisualClass}`}
                              disabled={isRowFullyBusy}
                              onClick={() => {
                                if (rowActionBusyRef.current[queueItem.id] || isActionLoading || isRowBlocked) {
                                  return;
                                }
                                setActionLoadingId(queueItem.id);
                                setRowBusy(queueItem.id, true);
                                void requestResume(queueItem).finally(() => {
                                  setRowBusy(queueItem.id, false);
                                  setActionLoadingId((c) => (c === queueItem.id ? null : c));
                                });
                              }}
                            >
                              <Play className="h-4 w-4" />
                              <span>{t("queue.resume")}</span>
                            </Button>
                          )}
                          {(row.canInteractiveManualImport || row.canDirectManualImport) && (
                            <Button
                              type="button"
                              size="sm"
                              variant="secondary"
                              className={`flex-1 ${rowActionVisualClass}`}
                              disabled={isRowFullyBusy}
                              onClick={() => {
                                if (rowActionBusyRef.current[queueItem.id] || isActionLoading || isRowBlocked) {
                                  return;
                                }
                                if (manualImportPending) return;
                                setManualImportingId(queueItem.id);
                                setRowBusy(queueItem.id, true);
                                void requestManualImport(queueItem).finally(() => {
                                  setRowBusy(queueItem.id, false);
                                  setManualImportingId((current) =>
                                    current === queueItem.id ? null : current,
                                  );
                                });
                              }}
                            >
                              {manualImportPending ? (
                                <Loader2 className="h-4 w-4 animate-spin" />
                              ) : (
                                <ArrowDownToLine className="h-4 w-4" />
                              )}
                              <span>{manualImportPending ? t("queue.manualImporting") : t("queue.manualImportTooltip")}</span>
                            </Button>
                          )}
                          {row.canAssignTitle && (
                            <Button
                              type="button"
                              size="sm"
                              variant="secondary"
                              className={`flex-1 ${rowActionVisualClass}`}
                              disabled={isRowFullyBusy}
                              onClick={() => {
                                if (rowActionBusyRef.current[queueItem.id] || isActionLoading || isRowBlocked) {
                                  return;
                                }
                                setActionLoadingId(queueItem.id);
                                setRowBusy(queueItem.id, true);
                                void requestAssignTitle(queueItem).finally(() => {
                                  setRowBusy(queueItem.id, false);
                                  setActionLoadingId((current) => (current === queueItem.id ? null : current));
                                });
                              }}
                            >
                              <Link2 className="h-4 w-4" />
                              <span>
                                {row.trackedMatchTypeKey === "unmatched" || !queueItem.titleId
                                  ? t("queue.assignTitle")
                                  : t("queue.reassignTitle")}
                              </span>
                            </Button>
                          )}
                          {row.canIgnore && (
                            <Button
                              type="button"
                              size="sm"
                              variant="secondary"
                              className={`flex-1 ${rowActionVisualClass}`}
                              disabled={isRowFullyBusy}
                              onClick={() => {
                                if (rowActionBusyRef.current[queueItem.id] || isActionLoading || isRowBlocked) {
                                  return;
                                }
                                setActionLoadingId(queueItem.id);
                                setRowBusy(queueItem.id, true);
                                void requestIgnore(queueItem).finally(() => {
                                  setRowBusy(queueItem.id, false);
                                  setActionLoadingId((current) => (current === queueItem.id ? null : current));
                                });
                              }}
                            >
                              <CircleOff className="h-4 w-4" />
                              <span>{t("queue.ignore")}</span>
                            </Button>
                          )}
                          <Button
                            type="button"
                            size="sm"
                            variant="destructive"
                            className={`flex-1 ${rowActionVisualClass}`}
                            disabled={isRowFullyBusy}
                            onClick={() => {
                              if (rowActionBusyRef.current[queueItem.id] || isActionLoading || isRowBlocked) {
                                return;
                              }
                              setRowBusy(queueItem.id, true);
                              setDeleteConfirmItem(queueItem);
                            }}
                          >
                            <Trash2 className="h-4 w-4" />
                            <span>{t("label.delete")}</span>
                          </Button>
                        </div>
                      </div>
                    );
                  })}
                  {queueMode === "history" && historyLoadingMore ? (
                    <div className="flex items-center justify-center py-3 text-sm text-muted-foreground">
                      <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                      {t("label.loading")}
                    </div>
                  ) : null}
                </div>
              </div>
            )
          ) : (
            <div
              onScroll={handleResultsScroll}
              className={`${scrollHeightClass} overflow-y-auto rounded-xl border border-border/60`}
            >
              <div className="overflow-x-auto">
                <Table className="table-fixed min-w-[820px]">
                  <TableHeader>
                    <TableRow>
                      <TableHead className="w-[34%] min-w-0">{t("queue.title")}</TableHead>
                      <TableHead className="w-32 min-w-0">{t("queue.client")}</TableHead>
                      <TableHead className="w-44 min-w-0">{t("queue.status")}</TableHead>
                      <TableHead className="w-52 min-w-52">{t("queue.progress")}</TableHead>
                      <TableHead className="w-24 min-w-24">{t("queue.size")}</TableHead>
                      <TableHead className="w-44 min-w-44 text-right">{t("label.actions")}</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {queueItems.length === 0 && !queueLoading ? (
                      <TableRow>
                        <TableCell colSpan={6} className="text-sm text-muted-foreground">
                          {t("queue.empty")}
                        </TableCell>
                      </TableRow>
                    ) : (
                      queueItems.map((queueItem) => {
                        const row = deriveQueueRowPresentation(queueItem, t);
                        const manualImportPending = manualImportingId === queueItem.id;
                        const isActionLoading = actionLoadingId === queueItem.id;
                        const isRowBusy =
                          rowActionBusy[queueItem.id] ??
                          rowActionBusyRef.current[queueItem.id] ??
                          false;
                        const isRowBlocked =
                          isRowBusy || manualImportPending || isActionLoading;
                        const isDeleteConfirming = deleteConfirmItem?.id === queueItem.id;
                        const isRowFullyBusy = isRowBlocked || isDeleteConfirming;
                        const rowActionVisualClass = isRowFullyBusy
                          ? "pointer-events-none opacity-45 grayscale"
                          : "";
                        const isExpanded = Boolean(expandedItemIds[queueItem.id]);
                        const detailId = `activity-queue-details-${queueItem.id}`;

                        return (
                          <Fragment key={queueItem.id}>
                            <TableRow>
                              <TableCell className="min-w-0">
                                <p className="break-words whitespace-normal text-sm">
                                  {queueItem.titleName || "\u2014"}
                                </p>
                              </TableCell>
                              <TableCell className="min-w-0 align-middle">
                                <p className="break-words whitespace-normal text-sm">
                                  {queueItem.clientName || queueItem.clientType}
                                </p>
                                <p className="text-xs text-muted-foreground">
                                  {queueItem.clientType}
                                </p>
                              </TableCell>
                              <TableCell className="min-w-0 align-middle">
                                <ActivityQueueStatusBadge
                                  stateKey={row.displayStateKey}
                                  statusLabel={row.statusLabel}
                                  isExpandable={row.hasExpandableDetails}
                                  isExpanded={isExpanded}
                                  detailId={detailId}
                                  expandLabel={t(
                                    isExpanded
                                      ? "queue.hideDetails"
                                      : "queue.showDetails",
                                  )}
                                  onToggle={() => toggleExpandedDetails(queueItem.id)}
                                />
                                {queueItem.importErrorMessage && !row.hasStatusDetails && (
                                  <p
                                    className="mt-1 max-w-full break-words whitespace-normal text-xs text-rose-400"
                                    title={queueItem.importErrorMessage}
                                  >
                                    {queueItem.importErrorMessage}
                                  </p>
                                )}
                              </TableCell>
                              <TableCell className="w-52 min-w-52 align-middle">
                                <ActivityProgressBar
                                  percent={row.percent}
                                  remainingLabel={row.remainingLabel}
                                  colorClass={getProgressBarColor(row.displayStateKey)}
                                />
                              </TableCell>
                              <TableCell className="w-24 min-w-24 align-middle">
                                {formatBytes(queueItem.sizeBytes)}
                              </TableCell>
                              <TableCell className="w-44 min-w-44 align-middle text-right">
                                <div className="flex items-center justify-end gap-2">
                                  {row.canPause && (
                                    <Button
                                      type="button"
                                      size="sm"
                                      variant="secondary"
                                      className={`h-10 w-10 border border-border/50 bg-muted/70 text-foreground hover:bg-accent/90 ${rowActionVisualClass}`}
                                      disabled={isRowFullyBusy}
                                      title={t("queue.pause")}
                                      aria-label={t("queue.pause")}
                                      onClick={() => {
                                        if (rowActionBusyRef.current[queueItem.id] || isActionLoading || isRowBlocked) {
                                          return;
                                        }
                                        setActionLoadingId(queueItem.id);
                                        setRowBusy(queueItem.id, true);
                                        void requestPause(queueItem).finally(() => {
                                          setRowBusy(queueItem.id, false);
                                          setActionLoadingId((c) => (c === queueItem.id ? null : c));
                                        });
                                      }}
                                    >
                                      <Pause className="h-6 w-6" />
                                    </Button>
                                  )}
                                  {row.canResume && (
                                    <Button
                                      type="button"
                                      size="sm"
                                      variant="secondary"
                                      className={`h-10 w-10 border border-border/50 bg-muted/70 text-foreground hover:bg-accent/90 ${rowActionVisualClass}`}
                                      disabled={isRowFullyBusy}
                                      title={t("queue.resume")}
                                      aria-label={t("queue.resume")}
                                      onClick={() => {
                                        if (rowActionBusyRef.current[queueItem.id] || isActionLoading || isRowBlocked) {
                                          return;
                                        }
                                        setActionLoadingId(queueItem.id);
                                        setRowBusy(queueItem.id, true);
                                        void requestResume(queueItem).finally(() => {
                                          setRowBusy(queueItem.id, false);
                                          setActionLoadingId((c) => (c === queueItem.id ? null : c));
                                        });
                                      }}
                                    >
                                      <Play className="h-6 w-6" />
                                    </Button>
                                  )}
                                  {(row.canInteractiveManualImport || row.canDirectManualImport) && (
                                    <Button
                                      type="button"
                                      size="sm"
                                      variant="secondary"
                                      className={`h-10 w-10 border border-emerald-500/60 dark:border-emerald-500/50 bg-emerald-600/20 dark:bg-emerald-600/15 text-emerald-700 dark:text-emerald-200 hover:bg-emerald-600/30 dark:hover:bg-emerald-600/25 ${rowActionVisualClass}`}
                                      disabled={isRowFullyBusy}
                                      title={manualImportPending ? t("queue.manualImporting") : t("queue.manualImportTooltip")}
                                      aria-label={manualImportPending ? t("queue.manualImporting") : t("queue.manualImportTooltip")}
                                      onClick={() => {
                                        if (rowActionBusyRef.current[queueItem.id] || isActionLoading || isRowBlocked) {
                                          return;
                                        }
                                        if (manualImportPending) return;
                                        setManualImportingId(queueItem.id);
                                        setRowBusy(queueItem.id, true);
                                        void requestManualImport(queueItem).finally(() => {
                                          setRowBusy(queueItem.id, false);
                                          setManualImportingId((current) =>
                                            current === queueItem.id ? null : current,
                                          );
                                        });
                                      }}
                                    >
                                      {manualImportPending ? (
                                        <Loader2 className="h-5 w-5 animate-spin" />
                                      ) : (
                                        <ArrowDownToLine className="h-5 w-5" />
                                      )}
                                    </Button>
                                  )}
                                  {row.canAssignTitle && (
                                    <Button
                                      type="button"
                                      size="sm"
                                      variant="secondary"
                                      className={`h-10 w-10 border border-amber-500/60 bg-amber-600/15 text-amber-200 hover:bg-amber-600/25 ${rowActionVisualClass}`}
                                      disabled={isRowFullyBusy}
                                      title={row.trackedMatchTypeKey === "unmatched" || !queueItem.titleId ? t("queue.assignTitle") : t("queue.reassignTitle")}
                                      aria-label={row.trackedMatchTypeKey === "unmatched" || !queueItem.titleId ? t("queue.assignTitle") : t("queue.reassignTitle")}
                                      onClick={() => {
                                        if (rowActionBusyRef.current[queueItem.id] || isActionLoading || isRowBlocked) {
                                          return;
                                        }
                                        setActionLoadingId(queueItem.id);
                                        setRowBusy(queueItem.id, true);
                                        void requestAssignTitle(queueItem).finally(() => {
                                          setRowBusy(queueItem.id, false);
                                          setActionLoadingId((current) => (current === queueItem.id ? null : current));
                                        });
                                      }}
                                    >
                                      <Link2 className="h-5 w-5" />
                                    </Button>
                                  )}
                                  {row.canIgnore && (
                                    <Button
                                      type="button"
                                      size="sm"
                                      variant="secondary"
                                      className={`h-10 w-10 border border-border/50 bg-muted/70 text-foreground hover:bg-accent/90 ${rowActionVisualClass}`}
                                      disabled={isRowFullyBusy}
                                      title={t("queue.ignore")}
                                      aria-label={t("queue.ignore")}
                                      onClick={() => {
                                        if (rowActionBusyRef.current[queueItem.id] || isActionLoading || isRowBlocked) {
                                          return;
                                        }
                                        setActionLoadingId(queueItem.id);
                                        setRowBusy(queueItem.id, true);
                                        void requestIgnore(queueItem).finally(() => {
                                          setRowBusy(queueItem.id, false);
                                          setActionLoadingId((current) => (current === queueItem.id ? null : current));
                                        });
                                      }}
                                    >
                                      <CircleOff className="h-5 w-5" />
                                    </Button>
                                  )}
                                  <Button
                                    type="button"
                                    size="sm"
                                    variant="secondary"
                                    className={`h-10 w-10 border border-rose-500/50 bg-rose-600/15 text-rose-300 hover:bg-rose-600/25 ${rowActionVisualClass}`}
                                    disabled={isRowFullyBusy}
                                    title={t("label.delete")}
                                    aria-label={t("label.delete")}
                                    onClick={() => {
                                      if (rowActionBusyRef.current[queueItem.id] || isActionLoading || isRowBlocked) {
                                        return;
                                      }
                                      setRowBusy(queueItem.id, true);
                                      setDeleteConfirmItem(queueItem);
                                    }}
                                  >
                                    <Trash2 className="h-6 w-6" />
                                  </Button>
                                </div>
                              </TableCell>
                            </TableRow>
                            {row.hasExpandableDetails && isExpanded ? (
                              <TableRow>
                                <TableCell colSpan={6} className="bg-muted/10 p-3">
                                  <ActivityQueueDetailsPanel
                                    detailId={detailId}
                                    releaseTitle={row.releaseTitle}
                                    failureReason={row.failureReason}
                                    t={t}
                                  />
                                </TableCell>
                              </TableRow>
                            ) : null}
                          </Fragment>
                        );
                      })
                    )}
                    {queueMode === "history" && historyLoadingMore ? (
                      <TableRow>
                        <TableCell colSpan={6} className="py-4 text-center text-sm text-muted-foreground">
                          <span className="inline-flex items-center">
                            <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                            {t("label.loading")}
                          </span>
                        </TableCell>
                      </TableRow>
                    ) : null}
                  </TableBody>
                </Table>
              </div>
            </div>
          )}
        </CardContent>
      </Card>

    </>
  );
}


import {
  ArrowDownToLine,
  ChevronDown,
  ChevronRight,
  Loader2,
  Pause,
  Play,
  RefreshCw,
  Trash2,
} from "lucide-react";
import { useCallback, useRef, useState } from "react";

import { Button } from "@/components/ui/button";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import {
  HoverCard,
  HoverCardContent,
  HoverCardTrigger,
} from "@/components/ui/hover-card";
import { Progress } from "@/components/ui/progress";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import type { DownloadQueueItem, ImportRecord } from "@/lib/types";
import { useTranslate } from "@/lib/context/translate-context";
import { useIsMobile } from "@/lib/hooks/use-mobile";

type QueueMode = "scryer" | "all" | "history";

type ActivityViewState = {
  queueItems: DownloadQueueItem[];
  queueLoading: boolean;
  queueError: string | null;
  lastRefreshedAt: Date | null;
  requestManualImport: (item: DownloadQueueItem) => Promise<void>;
  requestPause: (item: DownloadQueueItem) => Promise<void>;
  requestResume: (item: DownloadQueueItem) => Promise<void>;
  requestDelete: (item: DownloadQueueItem) => Promise<void>;
  queueMode: QueueMode;
  setQueueMode: (queueMode: QueueMode) => void;
  importHistory: ImportRecord[];
  importHistoryLoading: boolean;
  refreshImportHistory: () => Promise<void>;
};

const queueStateClasses: Record<string, string> = {
  queued: "border-amber-500/40 bg-amber-500/10 text-amber-200",
  downloading: "border-sky-500/40 bg-sky-500/10 text-sky-200",
  post_processing: "border-cyan-500/40 bg-cyan-500/10 text-cyan-200",
  paused: "border-purple-500/40 bg-purple-500/10 text-purple-200",
  completed: "border-emerald-500/40 bg-emerald-500/15 dark:bg-emerald-500/10 text-emerald-700 dark:text-emerald-200",
  import_pending: "border-indigo-500/40 bg-indigo-500/10 text-indigo-200",
  failed: "border-rose-500/40 bg-rose-500/10 text-rose-200",
};

const queueStateLabels: Record<string, string> = {
  queued: "queue.state.queued",
  downloading: "queue.state.downloading",
  post_processing: "queue.state.postProcessing",
  paused: "queue.state.paused",
  completed: "queue.state.completed",
  import_pending: "queue.state.importPending",
  importpending: "queue.state.importPending",
  failed: "queue.state.failed",
};

const queueStateAttention: Record<string, boolean> = {
  failed: true,
  import_pending: true,
  importpending: true,
};

const importStatusClasses: Record<string, string> = {
  completed: "border-emerald-500/40 bg-emerald-500/15 dark:bg-emerald-500/10 text-emerald-700 dark:text-emerald-200",
  failed: "border-rose-500/40 bg-rose-500/10 text-rose-200",
  skipped: "border-amber-500/40 bg-amber-500/10 text-amber-200",
  processing: "border-sky-500/40 bg-sky-500/10 text-sky-200",
  queued: "border-border/40 bg-muted-foreground/10 text-card-foreground",
};

function normalizeQueueState(state: string): string {
  const normalized = state.trim().toLowerCase();
  return normalized === "importpending" ? "import_pending" : normalized;
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

function deriveDisplayState(queueItem: DownloadQueueItem): string {
  const stateKey = normalizeQueueState(queueItem.state);
  if (stateKey === "extracting" || stateKey === "verifying" || stateKey === "repairing") {
    return "post_processing";
  }
  if (stateKey === "downloading" && isPostProcessingReason(queueItem.attentionReason)) {
    return "post_processing";
  }
  return stateKey;
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

function formatTimestamp(ts: string | null): string {
  if (!ts) return "\u2014";
  try {
    return new Intl.DateTimeFormat(undefined, {
      dateStyle: "short",
      timeStyle: "short",
    }).format(new Date(ts));
  } catch {
    return ts;
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
    requestPause,
    requestResume,
    requestDelete,
    queueMode,
    setQueueMode,
    importHistory,
    importHistoryLoading,
    refreshImportHistory,
  } = state;
  const [manualImportingId, setManualImportingId] = useState<string | null>(null);
  const [actionLoadingId, setActionLoadingId] = useState<string | null>(null);
  const [deleteConfirmItem, setDeleteConfirmItem] = useState<DownloadQueueItem | null>(null);
  const [deleteInProgress, setDeleteInProgress] = useState(false);
  const [rowActionBusy, setRowActionBusy] = useState<Record<string, true>>({});
  const rowActionBusyRef = useRef<Record<string, true>>({});
  const [importHistoryExpanded, setImportHistoryExpanded] = useState(false);

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
              <div className="space-y-3">
                {queueItems.map((queueItem) => {
                    const stateKey = normalizeQueueState(queueItem.state);
                    const displayStateKey = deriveDisplayState(queueItem);
                    const percent = formatProgress(queueItem.progressPercent);
                    const remainingLabel = formatRemainingDuration(queueItem.remainingSeconds);
                    const needsManualImport =
                      queueItem.attentionRequired || queueStateAttention[stateKey];
                    const manualImportPending = manualImportingId === queueItem.id;
                    const isActionLoading = actionLoadingId === queueItem.id;
                    const isRowBusy = rowActionBusy[queueItem.id] ?? rowActionBusyRef.current[queueItem.id] ?? false;
                    const isRowBlocked = isRowBusy || manualImportPending || isActionLoading;
                    const isDeleteConfirming = deleteConfirmItem?.id === queueItem.id;
                    const isRowFullyBusy = isRowBlocked || isDeleteConfirming;

                    const canPause = stateKey === "downloading" || stateKey === "queued";
                    const canResume = stateKey === "paused";
                    const isCompleted = stateKey === "completed" || stateKey === "import_pending";
                    const failureReason = (
                      queueItem.attentionReason ||
                      queueItem.importErrorMessage ||
                      ""
                    ).trim();
                    const stageLabel = queueItem.attentionReason?.trim() ?? "";
                    const statusLabel =
                      displayStateKey === "post_processing" && stageLabel.length > 0
                        ? stageLabel
                        : t(queueStateLabels[displayStateKey] ?? "queue.state.unknown");
                    const failedReason = stateKey === "failed" && failureReason.length > 0;
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
                            {failedReason ? (
                              <HoverCard openDelay={250} closeDelay={75}>
                                <HoverCardTrigger asChild>
                                  <button
                                    type="button"
                                    className={`inline-flex items-center rounded border px-2 py-1 text-xs font-medium ${queueStateClasses[displayStateKey] ?? "border-border bg-muted text-card-foreground"}`}
                                  >
                                    {statusLabel}
                                  </button>
                                </HoverCardTrigger>
                                <HoverCardContent sideOffset={4} className="max-w-sm text-sm">
                                  <p className="whitespace-pre-wrap break-words text-foreground">
                                    {failureReason}
                                  </p>
                                </HoverCardContent>
                              </HoverCard>
                            ) : (
                              <span
                                className={`inline-flex items-center rounded border px-2 py-1 text-xs font-medium ${queueStateClasses[displayStateKey] ?? "border-border bg-muted text-card-foreground"}`}
                              >
                                {statusLabel}
                              </span>
                            )}
                          </div>
                        </div>
                        {queueItem.importErrorMessage && !failedReason ? (
                          <p className="mt-2 break-words text-xs text-rose-400">{queueItem.importErrorMessage}</p>
                        ) : null}
                        <div className="mt-3 space-y-1">
                          <div className="flex items-center justify-between text-xs">
                            <p className="font-semibold text-foreground">{percent}%</p>
                            <p className="text-muted-foreground">{remainingLabel ?? "\u2014"}</p>
                          </div>
                          <Progress
                            value={percent}
                            className="h-2.5 bg-muted/90"
                            indicatorClassName={getProgressBarColor(displayStateKey)}
                          />
                        </div>
                        <div className="mt-3 flex items-center justify-between text-xs text-muted-foreground">
                          <span>{formatBytes(queueItem.sizeBytes)}</span>
                        </div>
                        <div className="mt-3 flex flex-wrap gap-2">
                          {canPause && (
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
                          {canResume && (
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
                          {isCompleted && needsManualImport && (
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
              </div>
            )
          ) : (
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
                    const stateKey = normalizeQueueState(queueItem.state);
                    const displayStateKey = deriveDisplayState(queueItem);
                    const percent = formatProgress(queueItem.progressPercent);
                    const remainingLabel = formatRemainingDuration(queueItem.remainingSeconds);
                    const needsManualImport =
                      queueItem.attentionRequired || queueStateAttention[stateKey];
                    const manualImportPending = manualImportingId === queueItem.id;
                    const isActionLoading = actionLoadingId === queueItem.id;
                    const isRowBusy = rowActionBusy[queueItem.id] ?? rowActionBusyRef.current[queueItem.id] ?? false;
                    const isRowBlocked = isRowBusy || manualImportPending || isActionLoading;
                    const isDeleteConfirming = deleteConfirmItem?.id === queueItem.id;
                    const isRowFullyBusy = isRowBlocked || isDeleteConfirming;

                    const canPause = stateKey === "downloading" || stateKey === "queued";
                    const canResume = stateKey === "paused";
                    const isCompleted = stateKey === "completed" || stateKey === "import_pending";
                    const failureReason = (
                      queueItem.attentionReason ||
                      queueItem.importErrorMessage ||
                      ""
                    ).trim();
                    const stageLabel = queueItem.attentionReason?.trim() ?? "";
                    const statusLabel =
                      displayStateKey === "post_processing" && stageLabel.length > 0
                        ? stageLabel
                        : t(queueStateLabels[displayStateKey] ?? "queue.state.unknown");
                    const failedReason = stateKey === "failed" && failureReason.length > 0;
                    const rowActionVisualClass = isRowFullyBusy
                      ? "pointer-events-none opacity-45 grayscale"
                      : "";

                    return (
                      <TableRow key={queueItem.id}>
                        <TableCell className="min-w-0">
                          <p className="break-words whitespace-normal text-sm">{queueItem.titleName || "\u2014"}</p>
                        </TableCell>
                        <TableCell className="min-w-0 align-middle">
                          <p className="break-words whitespace-normal text-sm">
                            {queueItem.clientName || queueItem.clientType}
                          </p>
                          <p className="text-xs text-muted-foreground">{queueItem.clientType}</p>
                        </TableCell>
                        <TableCell className="min-w-0 align-middle">
                          {failedReason ? (
                            <HoverCard openDelay={250} closeDelay={75}>
                              <HoverCardTrigger asChild>
                                <button
                                  type="button"
                                  className={`inline-flex items-center rounded border px-2 py-1 text-xs font-medium ${queueStateClasses[displayStateKey] ?? "border-border bg-muted text-card-foreground"}`}
                                >
                                  {statusLabel}
                                </button>
                              </HoverCardTrigger>
                              <HoverCardContent sideOffset={4} className="max-w-sm text-sm">
                                <p className="whitespace-pre-wrap break-words text-foreground">
                                  {failureReason}
                                </p>
                              </HoverCardContent>
                            </HoverCard>
                          ) : (
                            <span
                              className={`inline-flex items-center rounded border px-2 py-1 text-xs font-medium ${queueStateClasses[displayStateKey] ?? "border-border bg-muted text-card-foreground"}`}
                            >
                              {statusLabel}
                            </span>
                          )}
                          {queueItem.importErrorMessage && !failedReason && (
                            <p
                              className="mt-1 max-w-full break-words whitespace-normal text-xs text-rose-400"
                              title={queueItem.importErrorMessage}
                            >
                              {queueItem.importErrorMessage}
                            </p>
                          )}
                        </TableCell>
                        <TableCell className="w-52 min-w-52 align-middle">
                          <div className="mb-1 flex items-center justify-between text-xs">
                            <p className="font-semibold text-foreground">{percent}%</p>
                            <p className="text-muted-foreground">{remainingLabel ?? "\u2014"}</p>
                          </div>
                          <Progress
                            value={percent}
                            className="h-2.5 bg-muted/90"
                            indicatorClassName={getProgressBarColor(displayStateKey)}
                          />
                        </TableCell>
                        <TableCell className="w-24 min-w-24 align-middle">{formatBytes(queueItem.sizeBytes)}</TableCell>
                        <TableCell className="w-44 min-w-44 align-middle text-right">
                          <div className="flex items-center justify-end gap-2">
                            {canPause && (
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
                            {canResume && (
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
                            {isCompleted && needsManualImport && (
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
                    );
                    })
                  )}
                </TableBody>
              </Table>
            </div>
          )}
        </CardContent>
      </Card>

      {/* Import History Section */}
      <Card className="mt-6">
        <CardHeader>
          <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
            <button
              type="button"
              className="flex items-center gap-2 text-left"
              onClick={() => setImportHistoryExpanded((prev) => !prev)}
            >
              {importHistoryExpanded ? (
                <ChevronDown className="h-4 w-4 text-muted-foreground" />
              ) : (
                <ChevronRight className="h-4 w-4 text-muted-foreground" />
              )}
              <CardTitle>{t("importHistory.title")}</CardTitle>
              <span className="text-xs text-muted-foreground">({importHistory.length})</span>
            </button>
            {importHistoryExpanded && (
              <Button
                type="button"
                className="w-full sm:w-auto"
                size="sm"
                variant="secondary"
                disabled={importHistoryLoading}
                onClick={() => {
                  void refreshImportHistory();
                }}
              >
                <RefreshCw className="mr-2 h-4 w-4" />
                {importHistoryLoading ? t("label.refreshing") : t("label.refresh")}
              </Button>
            )}
          </div>
        </CardHeader>
        {importHistoryExpanded && (
          <CardContent>
            {importHistoryLoading && importHistory.length === 0 ? (
              <p className="text-sm text-muted-foreground">{t("label.loading")}</p>
            ) : importHistory.length === 0 ? (
              <p className="text-sm text-muted-foreground">{t("importHistory.empty")}</p>
            ) : (
              isMobile ? (
                <div className="space-y-2">
                  {importHistory.map((record) => (
                    <div key={record.id} className="rounded-lg border border-border bg-card/30 p-3">
                      <div className="flex items-start justify-between gap-3">
                        <div className="min-w-0">
                          <p className="break-words text-sm font-medium text-foreground" title={record.sourceTitle ?? record.sourceRef}>
                            {record.sourceTitle ?? record.sourceRef}
                          </p>
                          {record.sourceTitle ? (
                            <p className="text-xs text-muted-foreground" title={record.sourceRef}>
                              {record.sourceRef}
                            </p>
                          ) : null}
                          <p className="text-xs text-muted-foreground">{record.importType}</p>
                        </div>
                        <span
                          className={`inline-flex items-center rounded border px-2 py-1 text-xs font-medium ${importStatusClasses[record.status] ?? "border-border bg-muted text-card-foreground"}`}
                        >
                          {record.status}
                        </span>
                      </div>
                      {(record.decision || record.skipReason) ? (
                        <div className="mt-2 text-xs">
                          {record.decision ? <span>{record.decision}</span> : null}
                          {record.skipReason ? <p className="text-muted-foreground">{record.skipReason}</p> : null}
                        </div>
                      ) : null}
                      {record.errorMessage ? (
                        <p className="mt-2 break-words text-xs text-rose-400">{record.errorMessage}</p>
                      ) : null}
                      <p className="mt-2 text-xs text-muted-foreground">{formatTimestamp(record.createdAt)}</p>
                    </div>
                  ))}
                </div>
              ) : (
                <div className="overflow-x-auto">
                  <Table className="min-w-[720px]">
                    <TableHeader>
                      <TableRow>
                        <TableHead>{t("importHistory.status")}</TableHead>
                        <TableHead>{t("importHistory.sourceRef")}</TableHead>
                        <TableHead>{t("importHistory.decision")}</TableHead>
                        <TableHead>{t("importHistory.error")}</TableHead>
                        <TableHead>{t("importHistory.createdAt")}</TableHead>
                      </TableRow>
                    </TableHeader>
                    <TableBody>
                      {importHistory.map((record) => (
                        <TableRow key={record.id}>
                          <TableCell>
                            <span
                              className={`inline-flex items-center rounded border px-2 py-1 text-xs font-medium ${importStatusClasses[record.status] ?? "border-border bg-muted text-card-foreground"}`}
                            >
                              {record.status}
                            </span>
                          </TableCell>
                          <TableCell>
                            <p
                              className="max-w-xs whitespace-normal break-words text-sm"
                              title={record.sourceTitle ?? record.sourceRef}
                            >
                              {record.sourceTitle ?? record.sourceRef}
                            </p>
                            {record.sourceTitle ? (
                              <p className="text-xs text-muted-foreground" title={record.sourceRef}>
                                {record.sourceRef}
                              </p>
                            ) : null}
                            <p className="text-xs text-muted-foreground">{record.importType}</p>
                          </TableCell>
                          <TableCell>
                            {record.decision && (
                              <span className="text-xs">{record.decision}</span>
                            )}
                            {record.skipReason && (
                              <p className="text-xs text-muted-foreground">{record.skipReason}</p>
                            )}
                          </TableCell>
                          <TableCell>
                            {record.errorMessage ? (
                              <p
                                className="max-w-xs whitespace-normal break-words text-xs text-rose-400"
                                title={record.errorMessage}
                              >
                                {record.errorMessage}
                              </p>
                            ) : (
                              <span className="text-xs text-muted-foreground/60">{"\u2014"}</span>
                            )}
                          </TableCell>
                          <TableCell>
                            <p className="text-xs text-muted-foreground">
                              {formatTimestamp(record.createdAt)}
                            </p>
                          </TableCell>
                        </TableRow>
                      ))}
                    </TableBody>
                  </Table>
                </div>
              )
            )}
          </CardContent>
        )}
      </Card>
    </>
  );
}

import { ArrowDown, ArrowUp } from "lucide-react";
import { useCallback, useMemo, useState } from "react";

import { Button } from "@/components/ui/button";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { useLibraryScanProgress } from "@/lib/context/library-scan-progress-context";
import { useTranslate } from "@/lib/context/translate-context";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import type { Facet, JobDefinition, JobKey, JobRun, LibraryScanStatus } from "@/lib/types";
import type { UiDateTimeFormat } from "@/lib/types/settings";
import { formatUiDate, formatUiDateTime, formatUiTime } from "@/lib/utils/date-format";
import { isTerminalJobRunStatus } from "@/lib/utils/job-runs";
import { defaultLibraryIdForFacet } from "@/lib/utils/library-scan-sessions";
import { cn } from "@/lib/utils";

const JOBS_PANEL_CLASS =
  "overflow-hidden rounded-[14px] border border-[var(--scry-border)] bg-[var(--scry-surf)] shadow-[0_10px_24px_rgba(0,0,0,0.16)]";
const JOBS_PANEL_HEADER_CLASS =
  "border-b border-[var(--scry-border3)] bg-[linear-gradient(180deg,rgba(255,255,255,0.035),rgba(255,255,255,0))] px-4 py-3";
const JOBS_PANEL_TITLE_CLASS = "text-[15px] font-semibold text-[var(--scry-ink2)]";
const JOBS_INSET_CLASS =
  "rounded-[12px] border border-[var(--scry-line2)] bg-[var(--scry-card2)]";
const JOBS_MUTED_TEXT_CLASS = "text-[var(--scry-muted3)]";

type SystemJobsViewState = {
  jobs: JobDefinition[];
  activeRuns: JobRun[];
  recentRuns: JobRun[];
  selectedJobKey: JobKey | null;
  selectedJobHistory: JobRun[];
  jobHistoryLoading: boolean;
  triggeringKeys: Partial<Record<JobKey, boolean>>;
  onSelectJob: (jobKey: JobKey | null) => void;
  onTriggerJob: (jobKey: JobKey) => void;
};

type HealthCheckIssue = {
  source: string;
  status: string;
  message: string;
};

type SortKey = "name" | "nextRun" | "lastRun" | "status";
type SortDirection = "asc" | "desc";

type JobTableRow = {
  job: JobDefinition;
  activeRun: JobRun | null;
  activeLibraryScan: ReturnType<ReturnType<typeof useLibraryScanProgress>["getActiveSession"]> | null;
  lastRun: JobRun | null;
  status: JobRun["status"] | "idle";
  isDisabled: boolean;
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function formatDate(
  value: string | null | undefined,
  t: ReturnType<typeof useTranslate>,
  dateTimeFormat: UiDateTimeFormat,
): string {
  if (!value) {
    return t("jobs.never");
  }
  return formatUiDateTime(value, dateTimeFormat, { fallback: value });
}

function renderTableDateTime(
  value: string | null | undefined,
  t: ReturnType<typeof useTranslate>,
  dateTimeFormat: UiDateTimeFormat,
) {
  if (!value) {
    return t("jobs.never");
  }

  return (
    <div className="space-y-0.5 leading-tight">
      <div>{formatUiDate(value, dateTimeFormat, { fallback: value })}</div>
      <div>{formatUiTime(value, dateTimeFormat, { fallback: "" })}</div>
    </div>
  );
}

function runStatusTone(status: JobRun["status"] | "idle"): string {
  switch (status) {
    case "FAILED":
      return "text-[var(--scry-danger-text-soft)]";
    case "WARNING":
      return "text-[var(--scry-warning-text)]";
    case "COMPLETED":
      return "text-[var(--scry-success-text-soft)]";
    case "QUEUED":
    case "DISCOVERING":
    case "RUNNING":
      return "text-[var(--scry-info-text-soft)]";
    default:
      return "text-muted-foreground";
  }
}

function runStatusLabel(
  status: JobRun["status"] | "idle",
  t: ReturnType<typeof useTranslate>,
): string {
  switch (status) {
    case "idle":
      return t("jobs.status.idle");
    case "QUEUED":
      return t("jobs.status.queued");
    case "DISCOVERING":
      return t("jobs.status.discovering");
    case "RUNNING":
      return t("jobs.status.running");
    case "COMPLETED":
      return t("jobs.status.completed");
    case "WARNING":
      return t("jobs.status.warning");
    case "FAILED":
      return t("jobs.status.failed");
  }
}

function parseHealthCheckIssues(run: JobRun): HealthCheckIssue[] {
  if (run.jobKey !== "HEALTH_CHECKS" || !run.summaryJson) {
    return [];
  }

  try {
    const parsed = run.summaryJson;
    if (!isRecord(parsed) || !Array.isArray(parsed.checks)) {
      return [];
    }

    return parsed.checks.flatMap((check) => {
      if (
        !isRecord(check) ||
        typeof check.source !== "string" ||
        typeof check.status !== "string" ||
        typeof check.message !== "string" ||
        check.status === "ok"
      ) {
        return [];
      }

      return [
        {
          source: check.source,
          status: check.status,
          message: check.message,
        },
      ];
    });
  } catch {
    return [];
  }
}

function formatHealthCheckSource(source: string): string {
  const withSpaces = source.replace(/([a-z0-9])([A-Z])/g, "$1 $2").trim();
  return withSpaces.length > 0 ? withSpaces : source;
}

function healthCheckStatusTone(status: string): string {
  switch (status) {
    case "error":
      return "text-[var(--scry-danger-text-soft)]";
    case "warning":
      return "text-[var(--scry-warning-text)]";
    case "ok":
      return "text-[var(--scry-success-text-soft)]";
    default:
      return "text-muted-foreground";
  }
}

function formatHealthCheckStatus(status: string): string {
  if (!status) {
    return "Unknown";
  }
  return `${status.charAt(0).toUpperCase()}${status.slice(1)}`;
}

function triggerSourceLabel(
  triggerSource: JobRun["triggerSource"],
  t: ReturnType<typeof useTranslate>,
): string {
  switch (triggerSource) {
    case "MANUAL":
      return t("jobs.triggerSource.manual");
    case "SCHEDULED_STARTUP":
      return t("jobs.triggerSource.scheduledStartup");
    case "SCHEDULED_INTERVAL":
      return t("jobs.triggerSource.scheduledInterval");
    case "SCHEDULED_DAILY":
      return t("jobs.triggerSource.scheduledDaily");
    case "SYSTEM_INTERNAL":
      return t("jobs.triggerSource.systemInternal");
  }
}

function libraryFacetForJob(jobKey: JobKey): Facet | null {
  switch (jobKey) {
    case "LIBRARY_SCAN_MOVIES":
    case "BACKGROUND_LIBRARY_REFRESH_MOVIES":
      return "MOVIE";
    case "LIBRARY_SCAN_SERIES":
    case "BACKGROUND_LIBRARY_REFRESH_SERIES":
      return "SERIES";
    case "LIBRARY_SCAN_ANIME":
    case "BACKGROUND_LIBRARY_REFRESH_ANIME":
      return "ANIME";
    default:
      return null;
  }
}

function isRunButtonDisabled(
  hasActiveExecution: boolean,
  isTriggering: boolean,
): boolean {
  if (isTriggering) {
    return true;
  }

  if (hasActiveExecution) {
    return true;
  }

  return false;
}

function compareText(left: string, right: string): number {
  return left.localeCompare(right, undefined, { sensitivity: "base", numeric: true });
}

function compareMaybeDates(
  left: string | null | undefined,
  right: string | null | undefined,
): number {
  if (!left && !right) {
    return 0;
  }
  if (!left) {
    return 1;
  }
  if (!right) {
    return -1;
  }

  const leftTime = new Date(left).getTime();
  const rightTime = new Date(right).getTime();

  if (Number.isNaN(leftTime) && Number.isNaN(rightTime)) {
    return compareText(left, right);
  }
  if (Number.isNaN(leftTime)) {
    return 1;
  }
  if (Number.isNaN(rightTime)) {
    return -1;
  }

  return leftTime - rightTime;
}

function statusSortWeight(status: JobRun["status"] | "idle"): number {
  switch (status) {
    case "RUNNING":
    case "DISCOVERING":
    case "QUEUED":
      return 0;
    case "FAILED":
      return 1;
    case "WARNING":
      return 2;
    case "COMPLETED":
      return 3;
    case "idle":
    default:
      return 4;
  }
}

function jobStatusFromLibraryScanStatus(
  status: LibraryScanStatus,
): JobRun["status"] {
  switch (status) {
    case "DISCOVERING":
      return "DISCOVERING";
    case "RUNNING":
      return "RUNNING";
    case "COMPLETED":
      return "COMPLETED";
    case "CANCELED":
    case "WARNING":
      return "WARNING";
    case "FAILED":
      return "FAILED";
  }
}

function isStaleActiveRun(
  activeRun: JobRun | null | undefined,
  lastRun: JobRun | null | undefined,
): boolean {
  if (!activeRun || !lastRun || !isTerminalJobRunStatus(lastRun.status)) {
    return false;
  }

  if (lastRun.id === activeRun.id) {
    return true;
  }

  return lastRun.startedAt.localeCompare(activeRun.startedAt) >= 0;
}

export function SystemJobsView({ state }: { state: SystemJobsViewState }) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const { getActiveSession } = useLibraryScanProgress();
  const [sortKey, setSortKey] = useState<SortKey>("name");
  const [sortDirection, setSortDirection] = useState<SortDirection>("asc");
  const {
    jobs,
    activeRuns,
    recentRuns,
    selectedJobKey,
    selectedJobHistory,
    jobHistoryLoading,
    triggeringKeys,
    onSelectJob,
    onTriggerJob,
  } = state;

  const selectedJob = useMemo(
    () => jobs.find((job) => job.key === selectedJobKey) ?? null,
    [jobs, selectedJobKey],
  );

  const activeRunsByJob = useMemo(
    () => Object.fromEntries(activeRuns.map((run) => [run.jobKey, run])),
    [activeRuns],
  );

  const lastRunsByJob = useMemo(() => {
    const map = new Map<JobKey, JobRun>();
    for (const run of recentRuns) {
      if (!map.has(run.jobKey)) {
        map.set(run.jobKey, run);
      }
    }
    return map;
  }, [recentRuns]);

  const defaultSortDirectionFor = useCallback((key: SortKey): SortDirection => {
    switch (key) {
      case "lastRun":
        return "desc";
      default:
        return "asc";
    }
  }, []);

  const handleSort = useCallback((nextKey: SortKey) => {
    if (sortKey === nextKey) {
      setSortDirection((currentDirection) => (currentDirection === "asc" ? "desc" : "asc"));
      return;
    }

    setSortKey(nextKey);
    setSortDirection(defaultSortDirectionFor(nextKey));
  }, [defaultSortDirectionFor, sortKey]);

  const renderSortIcon = useCallback((key: SortKey) => {
    if (sortKey !== key) {
      return null;
    }

    return sortDirection === "asc"
      ? <ArrowUp className="h-3.5 w-3.5" />
      : <ArrowDown className="h-3.5 w-3.5" />;
  }, [sortDirection, sortKey]);

  const renderSortableHeader = useCallback((
    key: SortKey,
    label: string,
    className?: string,
  ) => (
    <TableHead
      className={className}
      aria-sort={
        sortKey === key
          ? sortDirection === "asc"
            ? "ascending"
            : "descending"
          : "none"
      }
    >
      <button
        type="button"
        className="inline-flex w-full items-center gap-1 text-left font-semibold text-[var(--scry-muted2)] transition-colors hover:text-[var(--scry-ink2)]"
        onClick={() => handleSort(key)}
      >
        <span>{label}</span>
        {renderSortIcon(key)}
      </button>
    </TableHead>
  ), [handleSort, renderSortIcon, sortDirection, sortKey]);

  const jobRows = useMemo<JobTableRow[]>(() =>
    jobs.map((job) => {
      const rawActiveRun = activeRunsByJob[job.key];
      const libraryFacet = libraryFacetForJob(job.key);
      const activeLibraryScan =
        job.usesLibraryScanProgress && libraryFacet
          ? getActiveSession(
              libraryFacet,
              defaultLibraryIdForFacet(libraryFacet),
            )
          : null;
      const recentRun = lastRunsByJob.get(job.key) ?? null;
      const activeRun = isStaleActiveRun(rawActiveRun, recentRun) ? null : (rawActiveRun ?? null);
      const lastRun = activeRun ?? recentRun;
      const status =
        activeRun?.status ??
        (activeLibraryScan ? jobStatusFromLibraryScanStatus(activeLibraryScan.status) : null) ??
        lastRun?.status ??
        "idle";
      const isTriggering = Boolean(triggeringKeys[job.key]);
      const hasActiveExecution = Boolean(activeRun) || Boolean(activeLibraryScan);
      const isDisabled = isRunButtonDisabled(hasActiveExecution, isTriggering);

      return {
        job,
        activeRun: activeRun ?? null,
        activeLibraryScan,
        lastRun,
        status,
        isDisabled,
      };
    }), [activeRunsByJob, getActiveSession, jobs, lastRunsByJob, triggeringKeys]);

  const sortedJobRows = useMemo(() => {
    const factor = sortDirection === "asc" ? 1 : -1;

    return [...jobRows].sort((left, right) => {
      const delta = (() => {
        switch (sortKey) {
          case "name":
            return compareText(left.job.displayName, right.job.displayName);
          case "nextRun":
            return compareMaybeDates(left.job.schedule.nextRunAt, right.job.schedule.nextRunAt);
          case "lastRun":
            return compareMaybeDates(
              left.lastRun?.completedAt ?? left.lastRun?.startedAt ?? null,
              right.lastRun?.completedAt ?? right.lastRun?.startedAt ?? null,
            );
          case "status":
            return (
              statusSortWeight(left.status) - statusSortWeight(right.status) ||
              compareText(runStatusLabel(left.status, t), runStatusLabel(right.status, t))
            );
          default:
            return 0;
        }
      })();

      if (delta !== 0) {
        return delta * factor;
      }

      return compareText(left.job.displayName, right.job.displayName);
    });
  }, [jobRows, sortDirection, sortKey, t]);

  const renderRows = (rows: JobTableRow[]) =>
    rows.map(({ job, lastRun, status, isDisabled }) => (
      <TableRow
        key={job.key}
        className="cursor-pointer border-[var(--scry-border3)] hover:bg-[var(--scry-hover)]"
        onClick={() => onSelectJob(job.key)}
      >
        <TableCell className="min-w-0">
          <div className="space-y-1">
            <p className="font-medium text-[var(--scry-ink2)]">{job.displayName}</p>
            <p className={`text-xs ${JOBS_MUTED_TEXT_CLASS}`}>{job.description}</p>
            {job.key === "ARTWORK_ENCODING" && lastRun?.summaryText ? (
              <p className={`text-xs ${JOBS_MUTED_TEXT_CLASS}`}>{lastRun.summaryText}</p>
            ) : null}
          </div>
        </TableCell>
        <TableCell className={`w-[14rem] max-w-[14rem] ${JOBS_MUTED_TEXT_CLASS}`}>
          {job.schedule.description}
        </TableCell>
        <TableCell className={`w-[10.5rem] min-w-[10.5rem] ${JOBS_MUTED_TEXT_CLASS}`}>
          {renderTableDateTime(job.schedule.nextRunAt, t, dateTimeFormat)}
        </TableCell>
        <TableCell className={`w-[10.5rem] min-w-[10.5rem] ${JOBS_MUTED_TEXT_CLASS}`}>
          {renderTableDateTime(
            lastRun?.completedAt ?? lastRun?.startedAt ?? null,
            t,
            dateTimeFormat,
          )}
        </TableCell>
        <TableCell className="w-[7.5rem] min-w-[7.5rem]">
          <span className={runStatusTone(status)}>{runStatusLabel(status, t)}</span>
        </TableCell>
        <TableCell className="min-w-[6rem] text-right">
          {job.manualTriggerAllowed ? (
            <Button
              size="sm"
              variant="primary"
              disabled={isDisabled}
              onClick={(event) => {
                event.stopPropagation();
                onTriggerJob(job.key);
              }}
            >
              {t(job.key === "ARTWORK_ENCODING" ? "jobs.action.runArtworkNow" : "jobs.action.run")}
            </Button>
          ) : null}
        </TableCell>
        </TableRow>
    ));

  const renderMobileCards = (rows: JobTableRow[]) =>
    rows.map(({ job, lastRun, status, isDisabled }) => (
      <div
        key={job.key}
        className={`${JOBS_INSET_CLASS} p-4`}
      >
        <div
          className="cursor-pointer space-y-3"
          onClick={() => onSelectJob(job.key)}
          role="button"
          tabIndex={0}
          onKeyDown={(event) => {
            if (event.key === "Enter" || event.key === " ") {
              event.preventDefault();
              onSelectJob(job.key);
            }
          }}
        >
          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0 space-y-1">
              <p className="text-sm font-semibold text-[var(--scry-ink2)]">
                {job.displayName}
              </p>
              <p className={`text-xs leading-relaxed ${JOBS_MUTED_TEXT_CLASS}`}>
                {job.description}
              </p>
              {job.key === "ARTWORK_ENCODING" && lastRun?.summaryText ? (
                <p className={`text-xs ${JOBS_MUTED_TEXT_CLASS}`}>{lastRun.summaryText}</p>
              ) : null}
            </div>
            <span
              className={cn(
                "shrink-0 rounded-full border border-[var(--scry-border3)] bg-[var(--scry-inset)] px-2.5 py-1 text-[11px] font-medium",
                runStatusTone(status),
              )}
            >
              {runStatusLabel(status, t)}
            </span>
          </div>

          <div className="grid gap-3 sm:grid-cols-3">
            <div className="space-y-1">
              <p className={`text-[11px] font-semibold uppercase tracking-[0.12em] ${JOBS_MUTED_TEXT_CLASS}`}>
                {t("jobs.column.schedule")}
              </p>
              <p className="text-sm text-[var(--scry-ink2)]">{job.schedule.description}</p>
            </div>
            <div className="space-y-1">
              <p className={`text-[11px] font-semibold uppercase tracking-[0.12em] ${JOBS_MUTED_TEXT_CLASS}`}>
                {t("jobs.column.nextRun")}
              </p>
              <p className="text-sm text-[var(--scry-ink2)]">
                {formatDate(job.schedule.nextRunAt, t, dateTimeFormat)}
              </p>
            </div>
            <div className="space-y-1">
              <p className={`text-[11px] font-semibold uppercase tracking-[0.12em] ${JOBS_MUTED_TEXT_CLASS}`}>
                {t("jobs.column.lastRun")}
              </p>
              <p className="text-sm text-[var(--scry-ink2)]">
                {formatDate(
                  lastRun?.completedAt ?? lastRun?.startedAt ?? null,
                  t,
                  dateTimeFormat,
                )}
              </p>
            </div>
          </div>
        </div>

        <div className="mt-4 flex items-center justify-between gap-3 border-t border-[var(--scry-border3)] pt-3">
          <button
            type="button"
            className={`text-xs font-medium underline-offset-4 hover:text-[var(--scry-ink2)] hover:underline ${JOBS_MUTED_TEXT_CLASS}`}
            onClick={() => onSelectJob(job.key)}
          >
            {t("jobs.recentRuns")}
          </button>
          {job.manualTriggerAllowed ? (
            <Button
              size="sm"
              variant="primary"
              disabled={isDisabled}
              onClick={(event) => {
                event.stopPropagation();
                onTriggerJob(job.key);
              }}
            >
              {t(job.key === "ARTWORK_ENCODING" ? "jobs.action.runArtworkNow" : "jobs.action.run")}
            </Button>
          ) : null}
        </div>
      </div>
    ));

  return (
    <>
      <div className="space-y-4 text-sm">
        <section className={JOBS_PANEL_CLASS}>
          <div className={JOBS_PANEL_HEADER_CLASS}>
            <h2 className={JOBS_PANEL_TITLE_CLASS}>Job schedule</h2>
          </div>
          <div className="p-4 sm:p-5 md:p-0">
            <div className="space-y-3 md:hidden">
              <div className="flex flex-wrap gap-2">
                {([
                  ["name", t("jobs.column.name")],
                  ["nextRun", t("jobs.column.nextRun")],
                  ["lastRun", t("jobs.column.lastRun")],
                  ["status", t("jobs.column.status")],
                ] as const).map(([key, label]) => (
                  <Button
                    key={key}
                    type="button"
                    size="xs"
                    variant={sortKey === key ? "secondary" : "outline"}
                    onClick={() => handleSort(key)}
                  >
                    {label}
                    {renderSortIcon(key)}
                  </Button>
                ))}
              </div>
              <div className="space-y-3">{renderMobileCards(sortedJobRows)}</div>
            </div>

            <div className="hidden overflow-x-auto md:block">
              <Table className="table-fixed">
                <TableHeader>
                  <TableRow className="border-[var(--scry-border3)] bg-[var(--scry-inset)] hover:bg-[var(--scry-inset)]">
                    {renderSortableHeader("name", t("jobs.column.name"))}
                    <TableHead className={`w-[14rem] font-semibold ${JOBS_MUTED_TEXT_CLASS}`}>
                      {t("jobs.column.schedule")}
                    </TableHead>
                    {renderSortableHeader(
                      "nextRun",
                      t("jobs.column.nextRun"),
                      "w-[10.5rem]",
                    )}
                    {renderSortableHeader(
                      "lastRun",
                      t("jobs.column.lastRun"),
                      "w-[10.5rem]",
                    )}
                    {renderSortableHeader(
                      "status",
                      t("jobs.column.status"),
                      "w-[7.5rem]",
                    )}
                    <TableHead className="w-[6rem]" />
                  </TableRow>
                </TableHeader>
                <TableBody>{renderRows(sortedJobRows)}</TableBody>
              </Table>
            </div>
          </div>
        </section>
      </div>

      <Sheet
        open={Boolean(selectedJob)}
        onOpenChange={(open) => onSelectJob(open ? selectedJobKey : null)}
      >
        <SheetContent
          side="right"
          className="border-l border-[var(--scry-border)] bg-[var(--scry-surf)] text-[var(--scry-ink2)] sm:max-w-2xl"
        >
          {selectedJob ? (
            <>
              <SheetHeader className="border-b border-[var(--scry-border3)] bg-[linear-gradient(180deg,rgba(255,255,255,0.035),rgba(255,255,255,0))]">
                <SheetTitle className="text-[var(--scry-ink2)]">
                  {selectedJob.displayName}
                </SheetTitle>
                <SheetDescription className={JOBS_MUTED_TEXT_CLASS}>
                  {selectedJob.description}
                </SheetDescription>
              </SheetHeader>

              <div className="flex-1 space-y-4 overflow-y-auto px-4 pb-4">
                <div className={`${JOBS_INSET_CLASS} p-3`}>
                  <p className={`text-xs uppercase tracking-wide ${JOBS_MUTED_TEXT_CLASS}`}>
                    {t("jobs.schedule")}
                  </p>
                  <p className="mt-1 text-sm text-[var(--scry-ink2)]">
                    {selectedJob.schedule.description}
                  </p>
                  <p className={`mt-1 text-xs ${JOBS_MUTED_TEXT_CLASS}`}>
                    {t("jobs.nextRunPrefix", {
                      value: formatDate(selectedJob.schedule.nextRunAt, t, dateTimeFormat),
                    })}
                  </p>
                </div>

                <div className="flex gap-2">
                  {(() => {
                    const activeRun = activeRunsByJob[selectedJob.key] ?? null;
                    const recentRun = lastRunsByJob.get(selectedJob.key) ?? null;
                    const libraryFacet = libraryFacetForJob(selectedJob.key);
                    const activeLibraryScan =
                      selectedJob.usesLibraryScanProgress && libraryFacet
                        ? getActiveSession(
                            libraryFacet,
                            defaultLibraryIdForFacet(libraryFacet),
                          )
                        : null;
                    const effectiveActiveRun = isStaleActiveRun(activeRun, recentRun)
                      ? null
                      : activeRun;
                    const isTriggering = Boolean(triggeringKeys[selectedJob.key]);
                    const isDisabled = isRunButtonDisabled(
                      Boolean(effectiveActiveRun) || Boolean(activeLibraryScan),
                      isTriggering,
                    );

                    if (!selectedJob.manualTriggerAllowed) {
                      return null;
                    }

                    return (
                      <Button
                        variant="primary"
                        onClick={() => onTriggerJob(selectedJob.key)}
                        disabled={isDisabled}
                      >
                        {t(selectedJob.key === "ARTWORK_ENCODING" ? "jobs.action.runArtworkNow" : "jobs.action.runNow")}
                      </Button>
                    );
                  })()}
                </div>

                <div className="space-y-2">
                  <p className="text-sm font-medium text-[var(--scry-ink2)]">
                    {t("jobs.recentRuns")}
                  </p>
                  {jobHistoryLoading ? (
                    <p className={`text-sm ${JOBS_MUTED_TEXT_CLASS}`}>
                      {t("jobs.loadingRecentRuns")}
                    </p>
                  ) : selectedJobHistory.length === 0 ? (
                    <p className={`text-sm ${JOBS_MUTED_TEXT_CLASS}`}>
                      {t("jobs.noRunsYet")}
                    </p>
                  ) : (
                    <div className="space-y-2">
                      {selectedJobHistory.map((run) => {
                        const healthCheckIssues = parseHealthCheckIssues(run);

                        return (
                          <div key={run.id} className={`${JOBS_INSET_CLASS} p-3`}>
                            <div className="flex items-start justify-between gap-3">
                              <div className="space-y-1">
                                <p className={runStatusTone(run.status)}>
                                  {runStatusLabel(run.status, t)}
                                </p>
                                <p className={`text-xs ${JOBS_MUTED_TEXT_CLASS}`}>
                                  {t("jobs.startedAt", {
                                    value: formatDate(run.startedAt, t, dateTimeFormat),
                                  })}
                                </p>
                                <p className={`text-xs ${JOBS_MUTED_TEXT_CLASS}`}>
                                  {t("jobs.completedAt", {
                                    value: formatDate(run.completedAt, t, dateTimeFormat),
                                  })}
                                </p>
                                {run.summaryText ? (
                                  <p className="text-sm text-[var(--scry-ink2)]">
                                    {run.summaryText}
                                  </p>
                                ) : null}
                                {run.errorText ? (
                                  <p className="text-sm text-[var(--scry-danger-text-soft)]">{run.errorText}</p>
                                ) : null}
                              </div>
                              <p className={`text-xs ${JOBS_MUTED_TEXT_CLASS}`}>
                                {triggerSourceLabel(run.triggerSource, t)}
                              </p>
                            </div>

                            {healthCheckIssues.length > 0 ? (
                              <div className="mt-3 rounded-[10px] border border-[var(--scry-border3)] bg-[var(--scry-inset)] p-3">
                                <p className={`text-xs uppercase tracking-wide ${JOBS_MUTED_TEXT_CLASS}`}>
                                  {t("jobs.healthCheckIssues")}
                                </p>
                                <div className="mt-2 space-y-2">
                                  {healthCheckIssues.map((issue, index) => (
                                    <div
                                      key={`${run.id}-${issue.source}-${index}`}
                                      className="rounded-[9px] border border-[var(--scry-border3)] bg-[var(--scry-card2)] p-2"
                                    >
                                      <div className="flex items-start justify-between gap-3">
                                        <p className="text-sm font-medium text-[var(--scry-ink2)]">
                                          {formatHealthCheckSource(issue.source)}
                                        </p>
                                        <span
                                          className={`text-xs ${healthCheckStatusTone(issue.status)}`}
                                        >
                                          {formatHealthCheckStatus(issue.status)}
                                        </span>
                                      </div>
                                      <p className={`mt-1 text-sm ${JOBS_MUTED_TEXT_CLASS}`}>
                                        {issue.message}
                                      </p>
                                    </div>
                                  ))}
                                </div>
                              </div>
                            ) : null}
                          </div>
                        );
                      })}
                    </div>
                  )}
                </div>
              </div>
            </>
          ) : null}
        </SheetContent>
      </Sheet>
    </>
  );
}

import { useMemo } from "react";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { useTranslate } from "@/lib/context/translate-context";
import type { JobDefinition, JobKey, JobRun } from "@/lib/types";

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

function formatDate(
  value: string | null | undefined,
  t: ReturnType<typeof useTranslate>,
): string {
  if (!value) {
    return t("jobs.never");
  }
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}

function runStatusTone(status: JobRun["status"] | "idle"): string {
  switch (status) {
    case "failed":
      return "text-red-400";
    case "warning":
      return "text-amber-400";
    case "completed":
      return "text-emerald-400";
    case "queued":
    case "discovering":
    case "running":
      return "text-sky-400";
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
    case "queued":
      return t("jobs.status.queued");
    case "discovering":
      return t("jobs.status.discovering");
    case "running":
      return t("jobs.status.running");
    case "completed":
      return t("jobs.status.completed");
    case "warning":
      return t("jobs.status.warning");
    case "failed":
      return t("jobs.status.failed");
  }
}

function triggerSourceLabel(
  triggerSource: JobRun["triggerSource"],
  t: ReturnType<typeof useTranslate>,
): string {
  switch (triggerSource) {
    case "manual":
      return t("jobs.triggerSource.manual");
    case "scheduled_startup":
      return t("jobs.triggerSource.scheduledStartup");
    case "scheduled_interval":
      return t("jobs.triggerSource.scheduledInterval");
    case "system_internal":
      return t("jobs.triggerSource.systemInternal");
  }
}

export function SystemJobsView({ state }: { state: SystemJobsViewState }) {
  const t = useTranslate();
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

  const primaryJobs = jobs.filter((job) => job.section === "primary");
  const maintenanceJobs = jobs.filter((job) => job.section === "maintenance");

  const renderRows = (items: JobDefinition[]) =>
    items.map((job) => {
      const activeRun = activeRunsByJob[job.key];
      const lastRun = activeRun ?? lastRunsByJob.get(job.key) ?? null;
      const status = lastRun?.status ?? "idle";

      return (
        <TableRow
          key={job.key}
          className="cursor-pointer hover:bg-muted/30"
          onClick={() => onSelectJob(job.key)}
        >
          <TableCell>
            <div className="space-y-1">
              <p className="font-medium text-foreground">{job.displayName}</p>
              <p className="text-xs text-muted-foreground">{job.description}</p>
            </div>
          </TableCell>
          <TableCell className="capitalize text-muted-foreground">
            {t(`jobs.category.${job.category}`)}
          </TableCell>
          <TableCell className="text-muted-foreground">{job.schedule.description}</TableCell>
          <TableCell className="text-muted-foreground">
            {formatDate(job.schedule.nextRunAt, t)}
          </TableCell>
          <TableCell className="text-muted-foreground">
            {formatDate(lastRun?.completedAt ?? lastRun?.startedAt ?? null, t)}
          </TableCell>
          <TableCell>
            <span className={runStatusTone(status)}>{runStatusLabel(status, t)}</span>
          </TableCell>
          <TableCell>
            <Button
              size="sm"
              variant="outline"
              disabled={Boolean(activeRun) || Boolean(triggeringKeys[job.key])}
              onClick={(event) => {
                event.stopPropagation();
                onTriggerJob(job.key);
              }}
            >
              {Boolean(activeRun) || Boolean(triggeringKeys[job.key])
                ? t("jobs.action.running")
                : t("jobs.action.run")}
            </Button>
          </TableCell>
        </TableRow>
      );
    });

  return (
    <>
      <div className="space-y-6">
        <Card>
          <CardHeader>
            <CardTitle>{t("jobs.title")}</CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            {activeRuns.length > 0 ? (
              <div className="space-y-2">
                <p className="text-sm font-medium text-foreground">{t("jobs.activeRuns")}</p>
                <div className="grid gap-3 md:grid-cols-2">
                  {activeRuns.map((run) => (
                    <div key={run.id} className="rounded-lg border border-border bg-muted/20 p-3">
                      <div className="flex items-center justify-between gap-3">
                        <div>
                          <p className="font-medium text-foreground">{run.displayName}</p>
                          <p className="text-xs text-muted-foreground">
                            {run.summaryText ?? t("jobs.runSummaryRunning")}
                          </p>
                        </div>
                        <span className={runStatusTone(run.status)}>
                          {runStatusLabel(run.status, t)}
                        </span>
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            ) : null}

            <div className="space-y-2">
              <p className="text-sm font-medium text-foreground">{t("jobs.primary")}</p>
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>{t("jobs.column.name")}</TableHead>
                    <TableHead>{t("jobs.column.category")}</TableHead>
                    <TableHead>{t("jobs.column.schedule")}</TableHead>
                    <TableHead>{t("jobs.column.nextRun")}</TableHead>
                    <TableHead>{t("jobs.column.lastRun")}</TableHead>
                    <TableHead>{t("jobs.column.status")}</TableHead>
                    <TableHead>{t("jobs.column.trigger")}</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>{renderRows(primaryJobs)}</TableBody>
              </Table>
            </div>

            <div className="space-y-2">
              <p className="text-sm font-medium text-foreground">{t("jobs.maintenance")}</p>
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>{t("jobs.column.name")}</TableHead>
                    <TableHead>{t("jobs.column.category")}</TableHead>
                    <TableHead>{t("jobs.column.schedule")}</TableHead>
                    <TableHead>{t("jobs.column.nextRun")}</TableHead>
                    <TableHead>{t("jobs.column.lastRun")}</TableHead>
                    <TableHead>{t("jobs.column.status")}</TableHead>
                    <TableHead>{t("jobs.column.trigger")}</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>{renderRows(maintenanceJobs)}</TableBody>
              </Table>
            </div>
          </CardContent>
        </Card>
      </div>

      <Sheet open={Boolean(selectedJob)} onOpenChange={(open) => onSelectJob(open ? selectedJobKey : null)}>
        <SheetContent side="right" className="sm:max-w-xl">
          {selectedJob ? (
            <>
              <SheetHeader>
                <SheetTitle>{selectedJob.displayName}</SheetTitle>
                <SheetDescription>{selectedJob.description}</SheetDescription>
              </SheetHeader>

              <div className="flex-1 space-y-4 overflow-y-auto px-4 pb-4">
                <div className="rounded-lg border border-border bg-muted/20 p-3">
                  <p className="text-xs uppercase tracking-wide text-muted-foreground">
                    {t("jobs.schedule")}
                  </p>
                  <p className="mt-1 text-sm text-foreground">{selectedJob.schedule.description}</p>
                  <p className="mt-1 text-xs text-muted-foreground">
                    {t("jobs.nextRunPrefix", {
                      value: formatDate(selectedJob.schedule.nextRunAt, t),
                    })}
                  </p>
                </div>

                <div className="flex gap-2">
                  <Button
                    onClick={() => onTriggerJob(selectedJob.key)}
                    disabled={Boolean(activeRunsByJob[selectedJob.key]) || Boolean(triggeringKeys[selectedJob.key])}
                  >
                    {Boolean(activeRunsByJob[selectedJob.key]) || Boolean(triggeringKeys[selectedJob.key])
                      ? t("jobs.action.running")
                      : t("jobs.action.runNow")}
                  </Button>
                </div>

                <div className="space-y-2">
                  <p className="text-sm font-medium text-foreground">{t("jobs.recentRuns")}</p>
                  {jobHistoryLoading ? (
                    <p className="text-sm text-muted-foreground">{t("jobs.loadingRecentRuns")}</p>
                  ) : selectedJobHistory.length === 0 ? (
                    <p className="text-sm text-muted-foreground">{t("jobs.noRunsYet")}</p>
                  ) : (
                    <div className="space-y-2">
                      {selectedJobHistory.map((run) => (
                        <div key={run.id} className="rounded-lg border border-border p-3">
                          <div className="flex items-start justify-between gap-3">
                            <div className="space-y-1">
                              <p className={runStatusTone(run.status)}>
                                {runStatusLabel(run.status, t)}
                              </p>
                              <p className="text-xs text-muted-foreground">
                                {t("jobs.startedAt", { value: formatDate(run.startedAt, t) })}
                              </p>
                              <p className="text-xs text-muted-foreground">
                                {t("jobs.completedAt", { value: formatDate(run.completedAt, t) })}
                              </p>
                              {run.summaryText ? (
                                <p className="text-sm text-foreground">{run.summaryText}</p>
                              ) : null}
                              {run.errorText ? (
                                <p className="text-sm text-red-400">{run.errorText}</p>
                              ) : null}
                            </div>
                            <p className="text-xs text-muted-foreground">
                              {triggerSourceLabel(run.triggerSource, t)}
                            </p>
                          </div>
                        </div>
                      ))}
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

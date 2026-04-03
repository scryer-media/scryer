import * as React from "react";
import { CheckCircle2, CircleAlert, Loader2 } from "lucide-react";
import { useClient } from "urql";

import { LibraryScanToast } from "@/components/root/library-scan-toast";
import { toast } from "@/components/ui/sonner";
import { useTranslate } from "@/lib/context/translate-context";
import {
  activeJobRunsQuery,
  jobRunEventsSubscription,
} from "@/lib/graphql/queries";
import { useDeferredWsSubscription } from "@/lib/hooks/use-deferred-ws-subscription";
import {
  isTerminalJobRunStatus,
  normalizeJobRun,
  preferJobRunSnapshot,
} from "@/lib/utils/job-runs";
import type { JobRun } from "@/lib/types";

const TERMINAL_TOAST_DURATION_MS = 6_000;

function shouldShowBackgroundLibraryToast(run: JobRun): boolean {
  const scan = run.libraryScanProgress;
  if (!scan || scan.mode !== "additive") {
    return false;
  }
  if (run.status === "failed" || run.status === "warning") {
    return true;
  }
  if (scan.metadataProgress.total > 0 || scan.fileProgress.total > 0) {
    return true;
  }
  return Boolean(
    scan.summary &&
      (scan.summary.imported > 0 || scan.summary.matched > 0 || scan.summary.unmatched > 0),
  );
}

function JobRunToast({ run }: { run: JobRun }) {
  const t = useTranslate();
  const terminal = isTerminalJobRunStatus(run.status);
  const icon =
    run.status === "failed" ? (
      <CircleAlert className="h-4 w-4 text-red-400" />
    ) : terminal ? (
      <CheckCircle2 className="h-4 w-4 text-emerald-400" />
    ) : (
      <Loader2 className="h-4 w-4 animate-spin text-sky-400" />
    );

  return (
    <div className="w-[min(24rem,calc(100vw-3rem))] p-4">
      <div className="space-y-1">
        <div className="flex items-center gap-2">
          <p className="text-sm font-semibold text-foreground">{run.displayName}</p>
          {icon}
        </div>
        <p className="text-xs text-muted-foreground">
          {run.errorText ??
            run.summaryText ??
            (terminal ? t("jobs.runSummaryCompleted") : t("jobs.runSummaryRunning"))}
        </p>
      </div>
    </div>
  );
}

export function JobRunProvider({ children }: { children: React.ReactNode }) {
  const client = useClient();
  const t = useTranslate();
  const [runsById, setRunsById] = React.useState<Record<string, JobRun>>({});
  const dismissTimersRef = React.useRef<Record<string, ReturnType<typeof setTimeout>>>({});

  const upsertRun = React.useCallback((run: JobRun) => {
    setRunsById((current) => ({
      ...current,
      [run.id]: preferJobRunSnapshot(current[run.id], run),
    }));
  }, []);

  React.useEffect(() => {
    let cancelled = false;
    (async () => {
      const { data, error } = await client.query(activeJobRunsQuery, {}).toPromise();
      if (cancelled || error) {
        if (error) {
          console.error("[job-runs] failed to load active jobs:", error);
        }
        return;
      }

      const rawRuns: unknown[] = Array.isArray(data?.activeJobRuns) ? data.activeJobRuns : [];
      const normalizedRuns = rawRuns
        .map(normalizeJobRun)
        .filter((run): run is JobRun => run !== null);

      setRunsById((current) => {
        const next = { ...current };
        for (const run of normalizedRuns) {
          next[run.id] = preferJobRunSnapshot(next[run.id], run);
        }
        return next;
      });
    })();

    return () => {
      cancelled = true;
    };
  }, [client]);

  useDeferredWsSubscription<{ data?: { jobRunEvents?: unknown } }>({
    requestKey: "jobRunEvents",
    request: { query: jobRunEventsSubscription },
    onNext(result) {
      const normalized = normalizeJobRun(result.data?.jobRunEvents);
      if (normalized) {
        upsertRun(normalized);
      }
    },
    onError(error) {
      console.error("[job-runs] subscription error:", error);
    },
  });

  React.useEffect(() => {
    for (const run of Object.values(runsById)) {
      const isBackgroundLibraryRun =
        run.libraryScanProgress?.mode === "additive" && shouldShowBackgroundLibraryToast(run);
      const isGenericToast = !run.libraryScanProgress;
      const shouldRender = isBackgroundLibraryRun || isGenericToast;

      if (!shouldRender) {
        continue;
      }

      if (isTerminalJobRunStatus(run.status)) {
        const existingTimer = dismissTimersRef.current[run.id];
        if (!existingTimer) {
          dismissTimersRef.current[run.id] = setTimeout(() => {
            toast.dismiss(run.id);
            setRunsById((current) => {
              const next = { ...current };
              delete next[run.id];
              return next;
            });
            delete dismissTimersRef.current[run.id];
          }, TERMINAL_TOAST_DURATION_MS);
        }
      } else if (dismissTimersRef.current[run.id]) {
        clearTimeout(dismissTimersRef.current[run.id]);
        delete dismissTimersRef.current[run.id];
      }

      const scan = run.libraryScanProgress;
      if (isBackgroundLibraryRun && scan) {
        toast.custom(
          () => (
            <LibraryScanToast
              session={scan}
              t={t}
              titleOverride={run.displayName}
            />
          ),
          {
            id: run.id,
            className: "rounded-lg overflow-hidden p-0",
            duration: isTerminalJobRunStatus(run.status)
              ? TERMINAL_TOAST_DURATION_MS
              : Infinity,
          },
        );
        continue;
      }

      toast.custom(() => <JobRunToast run={run} />, {
        id: run.id,
        className: "rounded-lg overflow-hidden p-0",
        duration: isTerminalJobRunStatus(run.status)
          ? TERMINAL_TOAST_DURATION_MS
          : Infinity,
      });
    }
  }, [runsById, t]);

  React.useEffect(
    () => () => {
      for (const timer of Object.values(dismissTimersRef.current)) {
        clearTimeout(timer);
      }
    },
    [],
  );

  return <>{children}</>;
}

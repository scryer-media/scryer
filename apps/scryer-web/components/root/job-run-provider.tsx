import * as React from "react";
import { useClient } from "urql";

import { toast } from "@/components/ui/sonner";
import { useTranslate } from "@/lib/context/translate-context";
import {
  activeJobRunsQuery,
  jobRunsQuery,
  jobRunEventsSubscription,
} from "@/lib/graphql/queries";
import { useDeferredWsSubscription } from "@/lib/hooks/use-deferred-ws-subscription";
import {
  isTerminalJobRunStatus,
  normalizeJobRun,
  preferJobRunSnapshot,
} from "@/lib/utils/job-runs";
import type { JobKey, JobRun } from "@/lib/types";

const TERMINAL_TOAST_DURATION_MS = 6_000;
const INTERACTIVE_JOB_RECONCILE_DELAYS_MS = [1_000, 5_000, 15_000] as const;
const INTERACTIVE_JOB_RECONCILE_INTERVAL_MS = 30_000;
const INTERACTIVE_JOB_RECONCILE_LIMIT = 25;

type JobRunToastContextValue = {
  registerInteractiveJobRun: (
    run: JobRun,
    onTerminal?: (run: JobRun) => void,
  ) => () => void;
};

const JobRunToastContext = React.createContext<JobRunToastContextValue | null>(null);

function usesDedicatedLibraryScanToast(jobKey: JobKey): boolean {
  return (
    jobKey === "LIBRARY_SCAN_MOVIES" ||
    jobKey === "LIBRARY_SCAN_SERIES" ||
    jobKey === "LIBRARY_SCAN_ANIME" ||
    jobKey === "BACKGROUND_LIBRARY_REFRESH_MOVIES" ||
    jobKey === "BACKGROUND_LIBRARY_REFRESH_SERIES" ||
    jobKey === "BACKGROUND_LIBRARY_REFRESH_ANIME"
  );
}

export function JobRunProvider({
  children,
  enabled = true,
}: {
  children: React.ReactNode;
  enabled?: boolean;
}) {
  const client = useClient();
  const t = useTranslate();
  const [runsById, setRunsById] = React.useState<Record<string, JobRun>>({});
  const dismissTimersRef = React.useRef<Record<string, ReturnType<typeof setTimeout>>>({});
  const reconcileTimersRef = React.useRef<Record<string, ReturnType<typeof setTimeout>[]>>({});
  const reconcileIntervalsRef = React.useRef<Record<string, ReturnType<typeof setInterval>>>({});
  const interactiveRunIdsRef = React.useRef(new Set<string>());
  const terminalCallbacksRef = React.useRef<
    Record<string, Set<(run: JobRun) => void>>
  >({});

  const upsertRun = React.useCallback((run: JobRun) => {
    setRunsById((current) => ({
      ...current,
      [run.id]: preferJobRunSnapshot(current[run.id], run),
    }));
  }, []);

  const clearReconcileTimers = React.useCallback((runId: string) => {
    const timers = reconcileTimersRef.current[runId];
    if (timers) {
      for (const timer of timers) {
        clearTimeout(timer);
      }
      delete reconcileTimersRef.current[runId];
    }
    const interval = reconcileIntervalsRef.current[runId];
    if (interval) {
      clearInterval(interval);
      delete reconcileIntervalsRef.current[runId];
    }
  }, []);

  const reconcileInteractiveRun = React.useCallback(
    async (run: JobRun) => {
      if (!interactiveRunIdsRef.current.has(run.id)) {
        clearReconcileTimers(run.id);
        return;
      }

      try {
        const { data, error } = await client
          .query<{ jobRuns?: unknown[] }>(
            jobRunsQuery,
            { jobKey: run.jobKey, limit: INTERACTIVE_JOB_RECONCILE_LIMIT },
            { requestPolicy: "network-only" },
          )
          .toPromise();
        if (error) {
          throw error;
        }

        const authoritativeRun = (Array.isArray(data?.jobRuns) ? data.jobRuns : [])
          .map(normalizeJobRun)
          .find((candidate): candidate is JobRun => candidate?.id === run.id);
        if (!authoritativeRun) {
          return;
        }

        upsertRun(authoritativeRun);
        if (isTerminalJobRunStatus(authoritativeRun.status)) {
          clearReconcileTimers(authoritativeRun.id);
        }
      } catch (error) {
        console.error("[job-runs] failed to reconcile interactive job:", error);
      }
    },
    [clearReconcileTimers, client, upsertRun],
  );

  const scheduleInteractiveRunReconciliation = React.useCallback(
    (run: JobRun) => {
      clearReconcileTimers(run.id);
      if (isTerminalJobRunStatus(run.status)) {
        return;
      }

      reconcileTimersRef.current[run.id] = INTERACTIVE_JOB_RECONCILE_DELAYS_MS.map((delayMs) =>
        setTimeout(() => {
          void reconcileInteractiveRun(run);
        }, delayMs),
      );
      reconcileIntervalsRef.current[run.id] = setInterval(() => {
        void reconcileInteractiveRun(run);
      }, INTERACTIVE_JOB_RECONCILE_INTERVAL_MS);
    },
    [clearReconcileTimers, reconcileInteractiveRun],
  );

  const registerInteractiveJobRun = React.useCallback(
    (run: JobRun, onTerminal?: (run: JobRun) => void) => {
      interactiveRunIdsRef.current.add(run.id);
      upsertRun(run);
      scheduleInteractiveRunReconciliation(run);

      if (!onTerminal) {
        return () => {};
      }
      if (isTerminalJobRunStatus(run.status)) {
        queueMicrotask(() => onTerminal(run));
        return () => {};
      }

      const callbacks = (terminalCallbacksRef.current[run.id] ??= new Set());
      callbacks.add(onTerminal);
      return () => {
        const current = terminalCallbacksRef.current[run.id];
        current?.delete(onTerminal);
        if (current?.size === 0) {
          delete terminalCallbacksRef.current[run.id];
        }
      };
    },
    [scheduleInteractiveRunReconciliation, upsertRun],
  );

  React.useEffect(() => {
    if (!enabled) {
      setRunsById({});
      return;
    }
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
  }, [client, enabled]);

  useDeferredWsSubscription<{ data?: { jobRunEvents?: unknown } }>({
    enabled,
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
    const idsToPrune: string[] = [];

    for (const run of Object.values(runsById)) {
      if (isTerminalJobRunStatus(run.status)) {
        clearReconcileTimers(run.id);
        const callbacks = terminalCallbacksRef.current[run.id];
        delete terminalCallbacksRef.current[run.id];
        callbacks?.forEach((callback) => callback(run));
      }

      const isInteractiveRun = interactiveRunIdsRef.current.has(run.id);
      const shouldRender =
        isInteractiveRun && !usesDedicatedLibraryScanToast(run.jobKey);

      if (!shouldRender) {
        if (isTerminalJobRunStatus(run.status)) {
          idsToPrune.push(run.id);
        }
        continue;
      }

      if (isTerminalJobRunStatus(run.status)) {
        const existingTimer = dismissTimersRef.current[run.id];
        if (!existingTimer) {
          dismissTimersRef.current[run.id] = setTimeout(() => {
            setRunsById((current) => {
              const next = { ...current };
              delete next[run.id];
              return next;
            });
            interactiveRunIdsRef.current.delete(run.id);
            delete dismissTimersRef.current[run.id];
          }, TERMINAL_TOAST_DURATION_MS);
        }
      } else if (dismissTimersRef.current[run.id]) {
        clearTimeout(dismissTimersRef.current[run.id]);
        delete dismissTimersRef.current[run.id];
      }

      const description =
        run.errorText ??
        run.summaryText ??
        (isTerminalJobRunStatus(run.status)
          ? t("jobs.runSummaryCompleted")
          : t("jobs.runSummaryRunning"));

      if (run.status === "FAILED") {
        toast.error(run.displayName, {
          id: run.id,
          description,
          duration: TERMINAL_TOAST_DURATION_MS,
        });
        continue;
      }

      if (run.status === "WARNING") {
        toast.warning(run.displayName, {
          id: run.id,
          description,
          duration: TERMINAL_TOAST_DURATION_MS,
        });
        continue;
      }

      if (run.status === "COMPLETED") {
        toast.success(run.displayName, {
          id: run.id,
          description,
          duration: TERMINAL_TOAST_DURATION_MS,
        });
        continue;
      }

      toast.loading(run.displayName, {
        id: run.id,
        description,
        duration: Infinity,
      });
    }

    if (idsToPrune.length > 0) {
      setRunsById((current) => {
        const next = { ...current };
        for (const id of idsToPrune) {
          delete next[id];
          interactiveRunIdsRef.current.delete(id);
        }
        return next;
      });
    }
  }, [clearReconcileTimers, runsById, t]);

  React.useEffect(
    () => () => {
      for (const timer of Object.values(dismissTimersRef.current)) {
        clearTimeout(timer);
      }
      for (const timers of Object.values(reconcileTimersRef.current)) {
        for (const timer of timers) {
          clearTimeout(timer);
        }
      }
      reconcileTimersRef.current = {};
      for (const interval of Object.values(reconcileIntervalsRef.current)) {
        clearInterval(interval);
      }
      reconcileIntervalsRef.current = {};
      terminalCallbacksRef.current = {};
    },
    [],
  );

  const contextValue = React.useMemo<JobRunToastContextValue>(() => ({
    registerInteractiveJobRun,
  }), [registerInteractiveJobRun]);

  return (
    <JobRunToastContext.Provider value={contextValue}>
      {children}
    </JobRunToastContext.Provider>
  );
}

export function useJobRunToasts(): JobRunToastContextValue {
  const context = React.useContext(JobRunToastContext);

  if (!context) {
    throw new Error("useJobRunToasts must be used within a JobRunProvider");
  }

  return context;
}

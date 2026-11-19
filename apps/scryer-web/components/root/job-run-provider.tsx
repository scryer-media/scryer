import * as React from "react";
import { useClient } from "urql";

import {
  isEpisodeSubtitleBatch,
  subtitleBatchProgressText,
} from "@/lib/utils/episode-subtitle-search";
import { toast } from "@/components/ui/sonner";
import { Button } from "@/components/ui/button";
import { jobRunDisplayName } from "@/lib/i18n/native-jobs";
import { useTranslate } from "@/lib/context/translate-context";
import {
  acquisitionProgress,
  automaticSearchOutcomeKey,
  shouldRestoreAutomaticSearch,
} from "@/lib/utils/automatic-search";
import { ruleImportRunFailureText } from "@/lib/utils/external-rule-import";
import {
  activeJobRunsQuery,
  jobRunsQuery,
  jobRunEventsSubscription,
} from "@/lib/graphql/queries";
import { cancelAcquisitionSearchMutation } from "@/lib/graphql/mutations";
import { useSessionUser } from "@/lib/hooks/use-auth";
import { useDeferredWsSubscription } from "@/lib/hooks/use-deferred-ws-subscription";
import { normalizeApplicationUpgradeProgress } from "@/lib/utils/application-upgrade-progress";
import { APP_PERMISSIONS, hasAppPermission } from "@/lib/utils/permissions";
import { useBackoffPoller } from "@/lib/hooks/use-backoff-poller";
import {
  judgeTrackedJobRuns,
  trackedJobRun,
  type TrackedJobRun,
} from "@/lib/utils/tracked-job-runs";
import {
  isTerminalJobRunStatus,
  libraryDeletionProgress,
  normalizeJobRun,
  preferJobRunSnapshot,
} from "@/lib/utils/job-runs";
import type { JobKey, JobRun } from "@/lib/types";

const TERMINAL_TOAST_DURATION_MS = 6_000;
// Waits between polls for runs this page waits on; the last one repeats.
const INTERACTIVE_JOB_RECONCILE_DELAYS_MS = [
  1_000, 5_000, 15_000, 30_000,
] as const;
const INTERACTIVE_JOB_RECONCILE_LIMIT = 25;
const APPLICATION_UPGRADE_RUN_STORAGE_KEY = "scryer:application-upgrade-run";

type JobRunToastContextValue = {
  /** Every run the stream and the active-runs load have surfaced, newest state each. */
  runs: JobRun[];
  cancelAcquisitionSearch: (runId: string) => Promise<void>;
  registerInteractiveJobRun: (
    run: JobRun,
    onTerminal?: (run: JobRun) => void,
  ) => () => void;
};

const JobRunToastContext = React.createContext<JobRunToastContextValue | null>(
  null,
);

// Jobs whose progress and outcome are shown where they were started, so the
// shared job toast stays out of the way: library scans have their own toast,
// and a grab reports on the button or dialog that asked for it.
function reportsOutsideJobToast(jobKey: JobKey): boolean {
  return (
    jobKey === "RELEASE_GRAB" ||
    jobKey === "LIBRARY_SCAN_MOVIES" ||
    jobKey === "LIBRARY_SCAN_SERIES" ||
    jobKey === "LIBRARY_SCAN_ANIME" ||
    jobKey === "BACKGROUND_LIBRARY_REFRESH_MOVIES" ||
    jobKey === "BACKGROUND_LIBRARY_REFRESH_SERIES" ||
    jobKey === "BACKGROUND_LIBRARY_REFRESH_ANIME"
  );
}

// Runs pages show as busy while they are active (a search button spins while
// its title is searched), so they are followed until they end even when this
// page did not start them: a missed end must not leave a button spinning.
function followsUntilEnded(run: JobRun): boolean {
  return run.jobKey === "ACQUISITION_SEARCH";
}

function readRememberedApplicationUpgradeRunId(): string | null {
  if (typeof window === "undefined") {
    return null;
  }
  try {
    const value = window.sessionStorage.getItem(
      APPLICATION_UPGRADE_RUN_STORAGE_KEY,
    );
    return value?.trim() || null;
  } catch {
    return null;
  }
}

function rememberApplicationUpgradeRun(run: JobRun) {
  if (run.jobKey !== "APPLICATION_UPGRADE" || typeof window === "undefined") {
    return;
  }
  try {
    window.sessionStorage.setItem(APPLICATION_UPGRADE_RUN_STORAGE_KEY, run.id);
  } catch {
    // Storage can be unavailable in a restricted browser context.
  }
}

function forgetApplicationUpgradeRun(run: JobRun) {
  if (run.jobKey !== "APPLICATION_UPGRADE" || typeof window === "undefined") {
    return;
  }
  try {
    window.sessionStorage.removeItem(APPLICATION_UPGRADE_RUN_STORAGE_KEY);
  } catch {
    // Storage can be unavailable in a restricted browser context.
  }
}

function applicationUpgradeToastDetails(
  run: JobRun,
  t: ReturnType<typeof useTranslate>,
): { title: string; description: string } | null {
  if (run.jobKey !== "APPLICATION_UPGRADE") {
    return null;
  }

  const progress = normalizeApplicationUpgradeProgress(run.progressJson);
  if (run.status === "COMPLETED") {
    return {
      title: t("appUpgrade.title"),
      description: progress?.targetVersion
        ? t("appUpgrade.success", { version: progress.targetVersion })
        : t("jobs.runSummaryCompleted"),
    };
  }

  if (run.status === "FAILED") {
    return {
      title: t("appUpgrade.title"),
      description: run.errorText ?? progress?.error ?? t("appUpgrade.failed"),
    };
  }

  return {
    title: t("appUpgrade.title"),
    description: t(`appUpgrade.phase.${progress?.phase ?? "unknown"}`),
  };
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
  // The server only lets system-settings managers cancel a search, so the toast
  // offers the button to them alone.
  const sessionUser = useSessionUser();
  const canCancelAcquisitionSearch = hasAppPermission(
    sessionUser,
    APP_PERMISSIONS.manageSystemSettings,
  );
  const cancelAcquisitionSearch = React.useCallback(
    async (runId: string) => {
      try {
        const { data, error } = await client
          .mutation(cancelAcquisitionSearchMutation, { id: runId })
          .toPromise();
        if (error) {
          throw error;
        }
        const payload = data?.cancelAcquisitionSearch as
          { accepted?: unknown } | undefined;
        toast.info(
          t(
            payload?.accepted === true
              ? "jobs.cancelSearchRequested"
              : "jobs.cancelSearchNotRunning",
          ),
        );
      } catch (error) {
        console.error("[job-runs] failed to cancel acquisition search:", error);
        toast.error(t("jobs.cancelSearchFailed"));
      }
    },
    [client, t],
  );
  const [runsById, setRunsById] = React.useState<Record<string, JobRun>>({});
  // A terminal event can arrive and leave the visible list before its start response.
  const terminalSnapshotsRef = React.useRef(new Map<string, JobRun>());
  const dismissTimersRef = React.useRef<
    Record<string, ReturnType<typeof setTimeout>>
  >({});
  // Runs this page started and is waiting on. The job stream normally ends
  // them; the poll below corrects a missed end and lets go of runs it can no
  // longer see, so nothing waits on them for ever.
  const trackedRunsRef = React.useRef(new Map<string, TrackedJobRun>());
  const interactiveRunIdsRef = React.useRef(new Set<string>());
  const terminalCallbacksRef = React.useRef<
    Record<string, Set<(run: JobRun) => void>>
  >({});

  const upsertRun = React.useCallback((run: JobRun) => {
    const accepted = preferJobRunSnapshot(
      terminalSnapshotsRef.current.get(run.id),
      run,
    );
    if (isTerminalJobRunStatus(accepted.status)) {
      terminalSnapshotsRef.current.set(run.id, accepted);
      if (terminalSnapshotsRef.current.size > 100) {
        const oldest = terminalSnapshotsRef.current.keys().next().value;
        if (oldest) terminalSnapshotsRef.current.delete(oldest);
      }
    }
    const tracked = trackedRunsRef.current.get(run.id);
    if (tracked) {
      tracked.run = preferJobRunSnapshot(tracked.run, accepted);
    }
    setRunsById((current) => ({
      ...current,
      [run.id]: preferJobRunSnapshot(current[run.id], accepted),
    }));
  }, []);

  const untrackRun = React.useCallback((runId: string) => {
    trackedRunsRef.current.delete(runId);
  }, []);

  // Stop waiting on a run that cannot be followed any more. Whoever waits on
  // it is handed the last state seen, which has not ended.
  const releaseTrackedRun = React.useCallback(
    (run: JobRun) => {
      untrackRun(run.id);
      interactiveRunIdsRef.current.delete(run.id);
      toast.dismiss(run.id);
      setRunsById((current) => {
        if (!(run.id in current)) return current;
        const next = { ...current };
        delete next[run.id];
        return next;
      });
      const callbacks = terminalCallbacksRef.current[run.id];
      delete terminalCallbacksRef.current[run.id];
      callbacks?.forEach((callback) => callback(run));
    },
    [untrackRun],
  );

  const reconcileTrackedRuns = React.useCallback(async (): Promise<boolean> => {
    const tracked = trackedRunsRef.current;
    if (tracked.size === 0) return false;

    const jobKeys = [
      ...new Set([...tracked.values()].map((entry) => entry.run.jobKey)),
    ];
    const listedByJobKey = new Map<JobKey, JobRun[]>();
    await Promise.all(
      jobKeys.map(async (jobKey) => {
        try {
          const { data, error } = await client
            .query<{ jobRuns?: unknown[] }>(
              jobRunsQuery,
              { jobKey, limit: INTERACTIVE_JOB_RECONCILE_LIMIT },
              { requestPolicy: "network-only" },
            )
            .toPromise();
          if (error) throw error;
          listedByJobKey.set(
            jobKey,
            (Array.isArray(data?.jobRuns) ? data.jobRuns : [])
              .map(normalizeJobRun)
              .filter((run): run is JobRun => run !== null),
          );
        } catch (error) {
          console.error(
            "[job-runs] failed to reconcile interactive job:",
            error,
          );
        }
      }),
    );

    const { found, released } = judgeTrackedJobRuns(tracked, listedByJobKey);
    for (const run of found) upsertRun(run);
    for (const run of released) releaseTrackedRun(run);
    return tracked.size > 0;
  }, [client, releaseTrackedRun, upsertRun]);

  // Stops by itself once a poll finds nothing left to follow, pauses while
  // the tab is hidden, and stops when the provider unmounts.
  const reconcilePoller = useBackoffPoller(
    INTERACTIVE_JOB_RECONCILE_DELAYS_MS,
    reconcileTrackedRuns,
  );

  // Follow a run until it ends. A run already followed keeps its place in
  // the waits, so progress events do not bring the next poll forward.
  const trackRun = React.useCallback(
    (run: JobRun) => {
      if (isTerminalJobRunStatus(run.status)) return;
      if (trackedRunsRef.current.has(run.id)) return;
      trackedRunsRef.current.set(run.id, trackedJobRun(run));
      reconcilePoller.restart();
    },
    [reconcilePoller],
  );

  const registerInteractiveJobRun = React.useCallback(
    (snapshot: JobRun, onTerminal?: (run: JobRun) => void) => {
      const run = preferJobRunSnapshot(
        terminalSnapshotsRef.current.get(snapshot.id),
        snapshot,
      );
      interactiveRunIdsRef.current.add(run.id);
      rememberApplicationUpgradeRun(run);
      upsertRun(run);
      trackRun(run);

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
    [trackRun, upsertRun],
  );

  React.useEffect(() => {
    if (!enabled) {
      setRunsById({});
      return;
    }
    let cancelled = false;
    (async () => {
      const { data, error } = await client
        .query(activeJobRunsQuery, {})
        .toPromise();
      if (cancelled || error) {
        if (error) {
          console.error("[job-runs] failed to load active jobs:", error);
        }
        return;
      }

      const rawRuns: unknown[] = Array.isArray(data?.activeJobRuns)
        ? data.activeJobRuns
        : [];
      const normalizedRuns = rawRuns
        .map(normalizeJobRun)
        .filter((run): run is JobRun => run !== null);

      for (const run of normalizedRuns) {
        if (shouldRestoreAutomaticSearch(run) || isEpisodeSubtitleBatch(run)) {
          registerInteractiveJobRun(run);
        } else {
          upsertRun(run);
          if (followsUntilEnded(run)) trackRun(run);
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [client, enabled, registerInteractiveJobRun, trackRun, upsertRun]);

  React.useEffect(() => {
    if (!enabled) {
      return;
    }
    const runId = readRememberedApplicationUpgradeRunId();
    if (!runId) {
      return;
    }

    let cancelled = false;
    void (async () => {
      try {
        const { data, error } = await client
          .query<{ jobRuns?: unknown[] }>(
            jobRunsQuery,
            {
              jobKey: "APPLICATION_UPGRADE",
              limit: INTERACTIVE_JOB_RECONCILE_LIMIT,
            },
            { requestPolicy: "network-only" },
          )
          .toPromise();
        if (cancelled || error) {
          return;
        }

        const run = (Array.isArray(data?.jobRuns) ? data.jobRuns : [])
          .map(normalizeJobRun)
          .find((candidate): candidate is JobRun => candidate?.id === runId);
        if (!run) {
          return;
        }

        interactiveRunIdsRef.current.add(run.id);
        upsertRun(run);
        trackRun(run);
      } catch (error) {
        console.error(
          "[job-runs] failed to reconcile application upgrade:",
          error,
        );
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [client, enabled, trackRun, upsertRun]);

  useDeferredWsSubscription<{ data?: { jobRunEvents?: unknown } }>({
    enabled,
    requestKey: "jobRunEvents",
    request: { query: jobRunEventsSubscription },
    onNext(result) {
      const normalized = normalizeJobRun(result.data?.jobRunEvents);
      if (normalized) {
        upsertRun(normalized);
        if (followsUntilEnded(normalized)) trackRun(normalized);
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
        untrackRun(run.id);
        forgetApplicationUpgradeRun(run);
        const callbacks = terminalCallbacksRef.current[run.id];
        delete terminalCallbacksRef.current[run.id];
        callbacks?.forEach((callback) => callback(run));
      }

      const isInteractiveRun = interactiveRunIdsRef.current.has(run.id);
      const shouldRender =
        isInteractiveRun && !reportsOutsideJobToast(run.jobKey);

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

      const defaultDescription =
        run.errorText ??
        run.summaryText ??
        (isTerminalJobRunStatus(run.status)
          ? t("jobs.runSummaryCompleted")
          : t("jobs.runSummaryRunning"));
      const upgradeToast = applicationUpgradeToastDetails(run, t);
      const deletionProgress = libraryDeletionProgress(run);
      const progressDescription =
        subtitleBatchProgressText(run, t) ??
        (deletionProgress
          ? t(
              deletionProgress.phase === "clearing"
                ? "settings.libraryDeleteProgressClearing"
                : "settings.libraryDeleteProgressDeleting",
              {
                processed: deletionProgress.processed,
                total: deletionProgress.total,
              },
            )
          : null);
      const title = isEpisodeSubtitleBatch(run)
        ? t("subtitle.batchSearch")
        : (upgradeToast?.title ?? jobRunDisplayName(run, t));
      const outcomeKey = automaticSearchOutcomeKey(run);
      const searchProgress = acquisitionProgress(run);
      const description =
        progressDescription ??
        ruleImportRunFailureText(run, t) ??
        run.errorText ??
        (outcomeKey
          ? t(outcomeKey, {
              processed: Number(searchProgress.processed ?? 0),
              grabbed: Number(searchProgress.grabbedCount ?? 0),
              failed: Number(searchProgress.failedCount ?? 0),
            })
          : run.jobKey === "ACQUISITION_SEARCH" &&
              !isTerminalJobRunStatus(run.status)
            ? t("wanted.searchJobProgress", {
                processed: Number(searchProgress.processed ?? 0),
                total: Number(searchProgress.total ?? 0),
                title:
                  typeof searchProgress.currentTitle === "string"
                    ? searchProgress.currentTitle
                    : "",
              })
            : (upgradeToast?.description ?? defaultDescription));

      if (run.status === "FAILED") {
        toast.error(title, {
          id: run.id,
          description,
          duration: TERMINAL_TOAST_DURATION_MS,
        });
        continue;
      }

      if (run.status === "WARNING") {
        toast.warning(title, {
          id: run.id,
          description,
          duration: TERMINAL_TOAST_DURATION_MS,
        });
        continue;
      }

      if (run.status === "COMPLETED") {
        toast.success(title, {
          id: run.id,
          description,
          duration: TERMINAL_TOAST_DURATION_MS,
        });
        continue;
      }

      toast.loading(title, {
        id: run.id,
        description,
        duration: Infinity,
        action:
          run.jobKey === "ACQUISITION_SEARCH" && canCancelAcquisitionSearch ? (
            <Button
              type="button"
              size="sm"
              variant="destructive"
              aria-label={t("label.cancel")}
              title={t("label.cancel")}
              onClick={() => void cancelAcquisitionSearch(run.id)}
            >
              {t("label.cancel")}
            </Button>
          ) : undefined,
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
    // Nothing left to follow: no poll waits for the next tick.
    if (trackedRunsRef.current.size === 0) {
      reconcilePoller.stop();
    }
  }, [
    canCancelAcquisitionSearch,
    cancelAcquisitionSearch,
    reconcilePoller,
    runsById,
    t,
    untrackRun,
  ]);

  React.useEffect(
    () => () => {
      for (const timer of Object.values(dismissTimersRef.current)) {
        clearTimeout(timer);
      }
      dismissTimersRef.current = {};
      // Whoever still waits is handed the last state seen, so their wait
      // ends with the provider instead of never.
      const callbacksByRun = terminalCallbacksRef.current;
      terminalCallbacksRef.current = {};
      for (const [runId, callbacks] of Object.entries(callbacksByRun)) {
        const run = trackedRunsRef.current.get(runId)?.run;
        if (run) callbacks.forEach((callback) => callback(run));
      }
      trackedRunsRef.current.clear();
    },
    [],
  );

  const runs = React.useMemo(() => Object.values(runsById), [runsById]);

  const contextValue = React.useMemo<JobRunToastContextValue>(
    () => ({
      runs,
      cancelAcquisitionSearch,
      registerInteractiveJobRun,
    }),
    [cancelAcquisitionSearch, registerInteractiveJobRun, runs],
  );

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

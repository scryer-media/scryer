import { memo, useCallback, useEffect, useMemo, useState } from "react";
import { useClient } from "urql";

import { SystemJobsView } from "@/components/views/system-jobs-view";
import { useJobRunToasts } from "@/components/root/job-run-provider";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import {
  jobRunEventsSubscription,
  jobRunsQuery,
  jobsQuery,
  recentJobRunsQuery,
} from "@/lib/graphql/queries";
import { triggerJobMutation } from "@/lib/graphql/mutations";
import { useDeferredWsSubscription } from "@/lib/hooks/use-deferred-ws-subscription";
import {
  normalizeJobRun,
  preferJobRunSnapshot,
} from "@/lib/utils/job-runs";
import type {
  JobCategory,
  JobDefinition,
  JobKey,
  JobRun,
  JobScheduleKind,
  JobSection,
} from "@/lib/types";

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function normalizeJobKey(value: unknown): JobKey {
  return typeof value === "string" ? (value as JobKey) : "RSS_SYNC";
}

function normalizeCategory(value: unknown): JobCategory {
  switch (value) {
    case "LIBRARY":
    case "ACQUISITION":
    case "MAINTENANCE":
    case "SUBTITLES":
    case "SYSTEM":
      return value;
    default:
      return "SYSTEM";
  }
}

function normalizeSection(value: unknown): JobSection {
  return value === "MAINTENANCE" ? "MAINTENANCE" : "PRIMARY";
}

function normalizeScheduleKind(value: unknown): JobScheduleKind {
  switch (value) {
    case "MANUAL":
    case "INTERVAL":
    case "STARTUP_AND_INTERVAL":
    case "DAILY_AT_TIME":
      return value;
    default:
      return "MANUAL";
  }
}

function normalizeJobDefinition(value: unknown): JobDefinition | null {
  if (!isRecord(value) || typeof value.key !== "string") {
    return null;
  }

  const schedule = isRecord(value.schedule) ? value.schedule : {};

  return {
    key: normalizeJobKey(value.key),
    displayName: typeof value.displayName === "string" ? value.displayName : value.key,
    description: typeof value.description === "string" ? value.description : "",
    category: normalizeCategory(value.category),
    section: normalizeSection(value.section),
    manualTriggerAllowed: value.manualTriggerAllowed === true,
    usesLibraryScanProgress: value.usesLibraryScanProgress === true,
    schedule: {
      kind: normalizeScheduleKind(schedule.kind),
      description: typeof schedule.description === "string" ? schedule.description : "",
      intervalSeconds: typeof schedule.intervalSeconds === "number" ? schedule.intervalSeconds : null,
      initialDelaySeconds:
        typeof schedule.initialDelaySeconds === "number" ? schedule.initialDelaySeconds : null,
      nextRunAt: typeof schedule.nextRunAt === "string" ? schedule.nextRunAt : null,
    },
  };
}

export const SystemJobsContainer = memo(function SystemJobsContainer() {
  const client = useClient();
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const { registerInteractiveJobRun } = useJobRunToasts();
  const [jobs, setJobs] = useState<JobDefinition[]>([]);
  const [activeRunsById, setActiveRunsById] = useState<Record<string, JobRun>>({});
  const [recentRuns, setRecentRuns] = useState<JobRun[]>([]);
  const [selectedJobKey, setSelectedJobKey] = useState<JobKey | null>(null);
  const [jobHistoryByKey, setJobHistoryByKey] = useState<Partial<Record<JobKey, JobRun[]>>>({});
  const [jobHistoryLoading, setJobHistoryLoading] = useState(false);
  const [triggeringKeys, setTriggeringKeys] = useState<Partial<Record<JobKey, boolean>>>({});

  useEffect(() => {
    let cancelled = false;
    (async () => {
      const [{ data: jobsData, error: jobsError }, { data: recentData, error: recentError }] = await Promise.all([
        client.query(jobsQuery, {}).toPromise(),
        client.query(recentJobRunsQuery, { limit: 50 }).toPromise(),
      ]);

      if (cancelled) {
        return;
      }
      const firstError = jobsError ?? recentError;
      if (firstError) {
        setGlobalStatus(firstError.message);
        return;
      }

      setJobs(
        ((Array.isArray(jobsData?.jobs) ? jobsData.jobs : []) as unknown[])
          .map(normalizeJobDefinition)
          .filter((job): job is JobDefinition => job !== null),
      );
      setRecentRuns(
        ((Array.isArray(recentData?.recentJobRuns) ? recentData.recentJobRuns : []) as unknown[])
          .map(normalizeJobRun)
          .filter((run): run is JobRun => run !== null),
      );
    })();

    return () => {
      cancelled = true;
    };
  }, [client, setGlobalStatus]);

  useEffect(() => {
    let cancelled = false;
    const refreshSchedule = async () => {
      const { data, error } = await client
        .query(jobsQuery, {}, { requestPolicy: "network-only" })
        .toPromise();
      if (cancelled || error) return;
      setJobs(
        ((Array.isArray(data?.jobs) ? data.jobs : []) as unknown[])
          .map(normalizeJobDefinition)
          .filter((job): job is JobDefinition => job !== null),
      );
    };
    // Host time and the next nightly window can change without a job event.
    const timer = window.setInterval(refreshSchedule, 60_000);
    window.addEventListener("focus", refreshSchedule);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
      window.removeEventListener("focus", refreshSchedule);
    };
  }, [client]);

  useDeferredWsSubscription<{ data?: { jobRunEvents?: unknown } }>({
    requestKey: "jobRunEvents.jobsPage",
    request: { query: jobRunEventsSubscription },
    onNext(result) {
      const normalized = normalizeJobRun(result.data?.jobRunEvents);
      if (!normalized) {
        return;
      }

      setActiveRunsById((current) => {
        const next = { ...current };
        if (normalized.completedAt || normalized.status === "COMPLETED" || normalized.status === "WARNING" || normalized.status === "FAILED") {
          delete next[normalized.id];
        } else {
          next[normalized.id] = preferJobRunSnapshot(current[normalized.id], normalized);
        }
        return next;
      });

      setRecentRuns((current) => {
        const deduped = current.filter((run) => run.id !== normalized.id);
        return [normalized, ...deduped].slice(0, 50);
      });

      setJobHistoryByKey((current) => {
        const history = current[normalized.jobKey];
        if (!history) {
          return current;
        }
        return {
          ...current,
          [normalized.jobKey]: [normalized, ...history.filter((run) => run.id !== normalized.id)].slice(0, 10),
        };
      });
    },
    onError(error) {
      console.error("[system-jobs] subscription error:", error);
    },
  });

  useEffect(() => {
    if (!selectedJobKey) {
      return;
    }

    let cancelled = false;
    setJobHistoryLoading(true);
    client
      .query(jobRunsQuery, { jobKey: selectedJobKey, limit: 10 })
      .toPromise()
      .then(({ data, error }) => {
        if (cancelled) {
          return;
        }
        if (error) {
          setGlobalStatus(error.message);
          return;
        }
        setJobHistoryByKey((current) => ({
          ...current,
          [selectedJobKey]: ((Array.isArray(data?.jobRuns) ? data.jobRuns : []) as unknown[])
            .map(normalizeJobRun)
            .filter((run): run is JobRun => run !== null),
        }));
      })
      .finally(() => {
        if (!cancelled) {
          setJobHistoryLoading(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [client, selectedJobKey, setGlobalStatus]);

  const onTriggerJob = useCallback(
    async (jobKey: JobKey) => {
      setTriggeringKeys((current) => ({ ...current, [jobKey]: true }));
      try {
        const { data, error } = await client
          .mutation(triggerJobMutation, { jobKey })
          .toPromise();
        if (error) {
          throw error;
        }
        const normalized = normalizeJobRun(data?.triggerJob);
        if (normalized) {
          registerInteractiveJobRun(normalized);
          setActiveRunsById((current) => ({ ...current, [normalized.id]: normalized }));
          setRecentRuns((current) => [normalized, ...current.filter((run) => run.id !== normalized.id)].slice(0, 50));
          setJobHistoryByKey((current) => ({
            ...current,
            [normalized.jobKey]: [
              normalized,
              ...(current[normalized.jobKey] ?? []).filter((run) => run.id !== normalized.id),
            ].slice(0, 10),
          }));
        }
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("jobs.failedToTrigger"));
      } finally {
        setTriggeringKeys((current) => ({ ...current, [jobKey]: false }));
      }
    },
    [client, registerInteractiveJobRun, setGlobalStatus, t],
  );

  const activeRuns = useMemo(
    () =>
      Object.values(activeRunsById).sort((left, right) =>
        left.startedAt.localeCompare(right.startedAt),
      ),
    [activeRunsById],
  );

  return (
    <SystemJobsView
      state={{
        jobs,
        activeRuns,
        recentRuns,
        selectedJobKey,
        selectedJobHistory: selectedJobKey ? jobHistoryByKey[selectedJobKey] ?? [] : [],
        jobHistoryLoading,
        triggeringKeys,
        onSelectJob: setSelectedJobKey,
        onTriggerJob,
      }}
    />
  );
});

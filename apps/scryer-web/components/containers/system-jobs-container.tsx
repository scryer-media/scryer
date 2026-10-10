import { memo, useCallback, useEffect, useMemo, useState } from "react";
import { useClient } from "urql";
import { useSearchParams } from "react-router";

import { ScriptEditorDialogs } from "@/components/common/script-editor-dialogs";
import { SystemJobsView, type CustomJobEntry } from "@/components/views/system-jobs-view";
import { useJobRunToasts } from "@/components/root/job-run-provider";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import {
  jobRunEventsSubscription,
  jobRunsQuery,
  jobsQuery,
  latestJobRunsQuery,
  recentJobRunsQuery,
} from "@/lib/graphql/queries";
import { triggerJobMutation } from "@/lib/graphql/mutations";
import { useDeferredWsSubscription } from "@/lib/hooks/use-deferred-ws-subscription";
import { useScriptEditor } from "@/lib/hooks/use-script-editor";
import {
  jobInstanceKey,
  jobTargetOf,
  mergeLatestJobRun,
  normalizeCustomJobId,
  normalizeJobRun,
  preferJobRunSnapshot,
  runAwaitingScriptOutput,
  SCRIPT_OUTPUT_POLL_DELAYS_MS,
} from "@/lib/utils/job-runs";
import type {
  JobCategory,
  JobDefinition,
  JobKey,
  JobRun,
  JobScheduleKind,
  JobSection,
  JobTarget,
  PostProcessingScript,
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
    case "CRON":
    case "WEEKLY_AT_TIME":
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
    customJobId: normalizeCustomJobId(value.customJobId),
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

function normalizeJobs(value: unknown): JobDefinition[] {
  return ((Array.isArray(value) ? value : []) as unknown[])
    .map(normalizeJobDefinition)
    .filter((job): job is JobDefinition => job !== null);
}

/**
 * The job a scheduled script runs as. The server lists a definition only for
 * enabled scripts; a disabled one is shown from the script itself.
 */
function customJobDefinition(
  script: PostProcessingScript,
  definition: JobDefinition | undefined,
): JobDefinition {
  if (definition) {
    return { ...definition, displayName: script.name, description: script.description };
  }
  return {
    key: "CUSTOM_JOB",
    customJobId: script.id,
    displayName: script.name,
    description: script.description,
    category: "SYSTEM",
    section: "PRIMARY",
    manualTriggerAllowed: script.enabled,
    usesLibraryScanProgress: false,
    schedule: {
      kind: "MANUAL",
      description: script.scheduleDescription ?? "",
      intervalSeconds: null,
      initialDelaySeconds: null,
      nextRunAt: null,
    },
  };
}

export const SystemJobsContainer = memo(function SystemJobsContainer() {
  const client = useClient();
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const { registerInteractiveJobRun } = useJobRunToasts();
  const [searchParams, setSearchParams] = useSearchParams();
  const selectedJobRunId = searchParams.get("jobRun")?.trim() || null;
  const [jobs, setJobs] = useState<JobDefinition[]>([]);
  const [activeRunsById, setActiveRunsById] = useState<Record<string, JobRun>>({});
  const [recentRuns, setRecentRuns] = useState<JobRun[]>([]);
  // Per-job state is keyed by `jobInstanceKey`, so each user-defined job is
  // tracked apart from the others that share the CUSTOM_JOB key.
  const [lastRunsByJob, setLastRunsByJob] = useState<Partial<Record<string, JobRun>>>({});
  const [selectedJob, setSelectedJob] = useState<JobTarget | null>(null);
  const [jobHistoryByKey, setJobHistoryByKey] = useState<Partial<Record<string, JobRun[]>>>({});
  const [jobHistoryLoading, setJobHistoryLoading] = useState(false);
  const [triggeringKeys, setTriggeringKeys] = useState<Partial<Record<string, boolean>>>({});
  const selectedInstanceKey = selectedJob ? jobInstanceKey(selectedJob) : null;
  const selectedJobKey = selectedJob?.jobKey ?? null;
  const selectedCustomJobId = selectedJob?.customJobId ?? null;

  const refreshJobs = useCallback(async () => {
    const { data, error } = await client
      .query(jobsQuery, {}, { requestPolicy: "network-only" })
      .toPromise();
    if (error) return;
    setJobs(normalizeJobs(data?.jobs));
  }, [client]);
  const scriptEditor = useScriptEditor("SCHEDULE", {
    onChanged: () => void refreshJobs(),
  });
  const { loadRunsForScript, scriptRuns, scripts: scheduledScripts } = scriptEditor;

  useEffect(() => {
    let cancelled = false;
    (async () => {
      const [
        { data: jobsData, error: jobsError },
        { data: recentData, error: recentError },
        { data: latestData, error: latestError },
      ] = await Promise.all([
        client.query(jobsQuery, {}).toPromise(),
        client.query(recentJobRunsQuery, { limit: 50 }).toPromise(),
        client.query(latestJobRunsQuery, {}).toPromise(),
      ]);

      if (cancelled) {
        return;
      }
      const firstError = jobsError ?? recentError ?? latestError;
      if (firstError) {
        setGlobalStatus(firstError.message, { level: "ERROR" });
        return;
      }

      setJobs(normalizeJobs(jobsData?.jobs));
      setRecentRuns(
        ((Array.isArray(recentData?.recentJobRuns) ? recentData.recentJobRuns : []) as unknown[])
          .map(normalizeJobRun)
          .filter((run): run is JobRun => run !== null),
      );
      // Runs that arrived over the subscription while this loaded are kept
      // when they are newer than what the load returned.
      setLastRunsByJob((current) =>
        ((Array.isArray(latestData?.latestJobRuns) ? latestData.latestJobRuns : []) as unknown[])
          .map(normalizeJobRun)
          .filter((run): run is JobRun => run !== null)
          .reduce(mergeLatestJobRun, current),
      );
    })();

    return () => {
      cancelled = true;
    };
  }, [client, setGlobalStatus]);

  useEffect(() => {
    if (!selectedJobRunId) {
      return;
    }
    const run = recentRuns.find((candidate) => candidate.id === selectedJobRunId);
    if (run) {
      setSelectedJob(jobTargetOf(run));
    }
  }, [recentRuns, selectedJobRunId]);

  useEffect(() => {
    const refreshSchedule = () => void refreshJobs();
    // Host time and the next nightly window can change without a job event.
    const timer = window.setInterval(refreshSchedule, 60_000);
    window.addEventListener("focus", refreshSchedule);
    return () => {
      window.clearInterval(timer);
      window.removeEventListener("focus", refreshSchedule);
    };
  }, [refreshJobs]);

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
      setLastRunsByJob((current) => mergeLatestJobRun(current, normalized));

      const instanceKey = jobInstanceKey(normalized);
      setJobHistoryByKey((current) => {
        const history = current[instanceKey];
        if (!history) {
          return current;
        }
        return {
          ...current,
          [instanceKey]: [normalized, ...history.filter((run) => run.id !== normalized.id)].slice(0, 10),
        };
      });
      // A finished user-defined job has new captured output to show.
      if (
        normalized.customJobId &&
        normalized.customJobId === selectedCustomJobId &&
        normalized.completedAt
      ) {
        void loadRunsForScript(normalized.customJobId);
      }
    },
    onError(error) {
      console.error("[system-jobs] subscription error:", error);
    },
  });

  useEffect(() => {
    if (!selectedJobKey || !selectedInstanceKey) {
      return;
    }

    let cancelled = false;
    setJobHistoryLoading(true);
    if (selectedCustomJobId) {
      void loadRunsForScript(selectedCustomJobId);
    }
    client
      .query(jobRunsQuery, {
        jobKey: selectedJobKey,
        ...(selectedCustomJobId ? { customJobId: selectedCustomJobId } : {}),
        limit: selectedJobRunId ? 50 : 10,
      }, { requestPolicy: "network-only" })
      .toPromise()
      .then(({ data, error }) => {
        if (cancelled) {
          return;
        }
        if (error) {
          setGlobalStatus(error.message, { level: "ERROR" });
          return;
        }
        setJobHistoryByKey((current) => ({
          ...current,
          [selectedInstanceKey]: ((Array.isArray(data?.jobRuns) ? data.jobRuns : []) as unknown[])
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
  }, [
    client,
    loadRunsForScript,
    selectedCustomJobId,
    selectedInstanceKey,
    selectedJobKey,
    selectedJobRunId,
    setGlobalStatus,
  ]);

  // Fire-and-forget scripts record their output after the job run finishes:
  // reload it with a bounded backoff until it settles or the sheet closes.
  const awaitingOutputRunId =
    selectedCustomJobId && selectedInstanceKey
      ? runAwaitingScriptOutput(
          jobHistoryByKey[selectedInstanceKey] ?? [],
          scriptRuns[selectedCustomJobId] ?? [],
        )
      : null;
  useEffect(() => {
    if (!selectedCustomJobId || !awaitingOutputRunId) {
      return;
    }
    let attempt = 0;
    let timer: number | undefined;
    const schedule = () => {
      const delay = SCRIPT_OUTPUT_POLL_DELAYS_MS[attempt];
      if (delay === undefined) return;
      attempt += 1;
      timer = window.setTimeout(() => {
        void loadRunsForScript(selectedCustomJobId);
        schedule();
      }, delay);
    };
    schedule();
    return () => {
      window.clearTimeout(timer);
    };
  }, [awaitingOutputRunId, loadRunsForScript, selectedCustomJobId]);

  const onSelectJob = useCallback(
    (target: JobTarget | null) => {
      const nextKey = target ? jobInstanceKey(target) : null;
      if (selectedJobRunId && nextKey !== selectedInstanceKey) {
        const next = new URLSearchParams(searchParams.toString());
        next.delete("jobRun");
        setSearchParams(next, { replace: true });
      }
      setSelectedJob(target);
    },
    [searchParams, selectedInstanceKey, selectedJobRunId, setSearchParams],
  );

  const onTriggerJob = useCallback(
    async (target: JobTarget) => {
      const instanceKey = jobInstanceKey(target);
      setTriggeringKeys((current) => ({ ...current, [instanceKey]: true }));
      try {
        const { data, error } = await client
          .mutation(triggerJobMutation, {
            jobKey: target.jobKey,
            ...(target.customJobId ? { customJobId: target.customJobId } : {}),
          })
          .toPromise();
        if (error) {
          throw error;
        }
        const normalized = normalizeJobRun(data?.triggerJob);
        if (normalized) {
          registerInteractiveJobRun(normalized);
          setActiveRunsById((current) => ({ ...current, [normalized.id]: normalized }));
          setRecentRuns((current) => [normalized, ...current.filter((run) => run.id !== normalized.id)].slice(0, 50));
          setLastRunsByJob((current) => mergeLatestJobRun(current, normalized));
          const runKey = jobInstanceKey(normalized);
          setJobHistoryByKey((current) => ({
            ...current,
            [runKey]: [
              normalized,
              ...(current[runKey] ?? []).filter((run) => run.id !== normalized.id),
            ].slice(0, 10),
          }));
        }
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("jobs.failedToTrigger"), { level: "ERROR" });
      } finally {
        setTriggeringKeys((current) => ({ ...current, [instanceKey]: false }));
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

  const builtInJobs = useMemo(() => jobs.filter((job) => job.key !== "CUSTOM_JOB"), [jobs]);
  const customJobs = useMemo<CustomJobEntry[]>(
    () =>
      scheduledScripts.map((script) => ({
        script,
        job: customJobDefinition(
          script,
          jobs.find((job) => job.key === "CUSTOM_JOB" && job.customJobId === script.id),
        ),
      })),
    [jobs, scheduledScripts],
  );

  return (
    <>
      <SystemJobsView
        state={{
          jobs: builtInJobs,
          customJobs,
          activeRuns,
          lastRunsByJob,
          selectedInstanceKey,
          selectedJobRunId,
          selectedJobHistory: selectedInstanceKey ? jobHistoryByKey[selectedInstanceKey] ?? [] : [],
          selectedScriptRuns: selectedCustomJobId ? scriptRuns[selectedCustomJobId] ?? [] : null,
          jobHistoryLoading,
          triggeringKeys,
          onSelectJob,
          onTriggerJob,
          customJobEditor: {
            isOpen: scriptEditor.isEditorOpen,
            isEditing: scriptEditor.editingScriptId !== null,
            draft: scriptEditor.scriptDraft,
            setDraft: scriptEditor.setScriptDraft,
            mutatingScriptId: scriptEditor.mutatingScriptId,
            onSubmit: scriptEditor.submitScript,
            onCancel: scriptEditor.requestCloseEditor,
            onAdd: scriptEditor.requestCreateEditor,
            onEdit: scriptEditor.requestEditScript,
            onToggle: scriptEditor.toggleScript,
            onDelete: scriptEditor.requestDeleteScript,
          },
        }}
      />
      <ScriptEditorDialogs
        state={scriptEditor.dialogs}
        ids={{
          deleteConfirm: "jobs-custom-delete-confirm",
          inlineShellContent: "settings-post-processing-inline-shell-confirm",
          inlineShellAccept: "settings-post-processing-inline-shell-confirm-accept",
          inlineShellCancel: "settings-post-processing-inline-shell-confirm-cancel",
        }}
      />
    </>
  );
});

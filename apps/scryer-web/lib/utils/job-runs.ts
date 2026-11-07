import type {
  JobCategory,
  JobKey,
  JobRun,
  JobRunStatus,
  JobSection,
  JobTarget,
  JobTriggerSource,
  LibraryScanMode,
  LibraryScanProgress,
  LibraryScanStatus,
} from "@/lib/types";

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function normalizeLibraryScanFacet(
  value: unknown,
): LibraryScanProgress["facet"] {
  return value === "ANIME" ? "ANIME" : value === "SERIES" ? "SERIES" : "MOVIE";
}

function normalizeLibraryScanMode(value: unknown): LibraryScanMode {
  return value === "ADDITIVE" ? "ADDITIVE" : "FULL";
}

function normalizeLibraryScanStatus(value: unknown): LibraryScanStatus {
  switch (value) {
    case "DISCOVERING":
    case "RUNNING":
    case "COMPLETED":
    case "CANCELED":
    case "WARNING":
    case "FAILED":
      return value;
    default:
      return "RUNNING";
  }
}

export function normalizeJobRunStatus(value: unknown): JobRunStatus {
  switch (value) {
    case "QUEUED":
    case "DISCOVERING":
    case "RUNNING":
    case "COMPLETED":
    case "WARNING":
    case "FAILED":
      return value;
    default:
      return "RUNNING";
  }
}

function normalizeNumber(value: unknown): number {
  return typeof value === "number" && Number.isFinite(value) ? value : 0;
}

function normalizeJobKey(value: unknown): JobKey {
  return typeof value === "string" ? (value as JobKey) : "RSS_SYNC";
}

export function normalizeCustomJobId(value: unknown): string | null {
  return typeof value === "string" && value.length > 0 ? value : null;
}

/**
 * The key one job instance is tracked under on the Jobs page. Every
 * user-defined job shares the CUSTOM_JOB key, so those are keyed by their own
 * id; built-in jobs keep their job key.
 */
export function jobInstanceKey(
  target:
    | { customJobId?: string | null; key: JobKey }
    | { customJobId?: string | null; jobKey: JobKey },
): string {
  if (target.customJobId) {
    return target.customJobId;
  }
  return "key" in target ? target.key : target.jobKey;
}

export function jobTargetOf(
  target:
    | { customJobId?: string | null; key: JobKey }
    | { customJobId?: string | null; jobKey: JobKey },
): JobTarget {
  return {
    jobKey: "key" in target ? target.key : target.jobKey,
    customJobId: target.customJobId ?? null,
  };
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

function normalizeTriggerSource(value: unknown): JobTriggerSource {
  switch (value) {
    case "MANUAL":
    case "SCHEDULED_STARTUP":
    case "SCHEDULED_INTERVAL":
    case "SCHEDULED_DAILY":
    case "SYSTEM_INTERNAL":
      return value;
    default:
      return "MANUAL";
  }
}

export function normalizeLibraryScanProgress(
  value: unknown,
): LibraryScanProgress | null {
  if (!isRecord(value) || typeof value.sessionId !== "string") {
    return null;
  }

  return {
    sessionId: value.sessionId,
    facet: normalizeLibraryScanFacet(value.facet),
    libraryId: typeof value.libraryId === "string" ? value.libraryId : null,
    mode: normalizeLibraryScanMode(value.mode),
    status: normalizeLibraryScanStatus(value.status),
    startedAt:
      typeof value.startedAt === "string"
        ? value.startedAt
        : new Date().toISOString(),
    updatedAt:
      typeof value.updatedAt === "string"
        ? value.updatedAt
        : new Date().toISOString(),
    foundTitles: normalizeNumber(value.foundTitles),
    titleMatchTotalKnown: value.titleMatchTotalKnown === true,
    hydrationTotalKnown: value.hydrationTotalKnown === true,
    mediaAnalysisTotalKnown: value.mediaAnalysisTotalKnown === true,
    titleMatchProgress: {
      total: normalizeNumber(
        isRecord(value.titleMatchProgress) ? value.titleMatchProgress.total : 0,
      ),
      completed: normalizeNumber(
        isRecord(value.titleMatchProgress)
          ? value.titleMatchProgress.completed
          : 0,
      ),
      failed: normalizeNumber(
        isRecord(value.titleMatchProgress)
          ? value.titleMatchProgress.failed
          : 0,
      ),
    },
    hydrationProgress: {
      total: normalizeNumber(
        isRecord(value.hydrationProgress) ? value.hydrationProgress.total : 0,
      ),
      completed: normalizeNumber(
        isRecord(value.hydrationProgress)
          ? value.hydrationProgress.completed
          : 0,
      ),
      failed: normalizeNumber(
        isRecord(value.hydrationProgress) ? value.hydrationProgress.failed : 0,
      ),
    },
    mediaAnalysisProgress: {
      total: normalizeNumber(
        isRecord(value.mediaAnalysisProgress)
          ? value.mediaAnalysisProgress.total
          : 0,
      ),
      completed: normalizeNumber(
        isRecord(value.mediaAnalysisProgress)
          ? value.mediaAnalysisProgress.completed
          : 0,
      ),
      failed: normalizeNumber(
        isRecord(value.mediaAnalysisProgress)
          ? value.mediaAnalysisProgress.failed
          : 0,
      ),
    },
    summary: isRecord(value.summary)
      ? {
          scanned: normalizeNumber(value.summary.scanned),
          matched: normalizeNumber(value.summary.matched),
          imported: normalizeNumber(value.summary.imported),
          skipped: normalizeNumber(value.summary.skipped),
          unmatched: normalizeNumber(value.summary.unmatched),
        }
      : null,
  };
}

export function normalizeJobRun(value: unknown): JobRun | null {
  if (!isRecord(value) || typeof value.id !== "string") {
    return null;
  }

  return {
    id: value.id,
    jobKey: normalizeJobKey(value.jobKey),
    customJobId: normalizeCustomJobId(value.customJobId),
    displayName:
      typeof value.displayName === "string" ? value.displayName : "Job",
    category: normalizeCategory(value.category),
    section: normalizeSection(value.section),
    status: normalizeJobRunStatus(value.status),
    triggerSource: normalizeTriggerSource(value.triggerSource),
    startedAt:
      typeof value.startedAt === "string"
        ? value.startedAt
        : new Date().toISOString(),
    completedAt:
      typeof value.completedAt === "string" ? value.completedAt : null,
    summaryJson: value.summaryJson ?? null,
    summaryText:
      typeof value.summaryText === "string" ? value.summaryText : null,
    errorText: typeof value.errorText === "string" ? value.errorText : null,
    progressJson: value.progressJson ?? null,
    libraryScanProgress: normalizeLibraryScanProgress(
      value.libraryScanProgress,
    ),
  };
}

export type FullHashBackfillFailure = {
  mediaFileId: string;
  path: string;
  reason: string;
};

export type FullHashBackfillFailures = {
  failures: FullHashBackfillFailure[];
  /** Failures the run counted but did not list. */
  notListed: number;
};

/** Reads the files a full-hash backfill run could not hash from its summary. */
export function parseFullHashBackfillFailures(
  run: Pick<JobRun, "jobKey" | "summaryJson">,
): FullHashBackfillFailures {
  const empty = { failures: [], notListed: 0 };
  if (run.jobKey !== "FULL_HASH_BACKFILL" || run.summaryJson == null) {
    return empty;
  }

  let parsed: unknown = run.summaryJson;
  if (typeof parsed === "string") {
    try {
      parsed = JSON.parse(parsed);
    } catch {
      return empty;
    }
  }
  if (!isRecord(parsed) || !Array.isArray(parsed.failures)) {
    return empty;
  }

  const failures = parsed.failures.flatMap(
    (failure): FullHashBackfillFailure[] => {
      if (
        !isRecord(failure) ||
        typeof failure.path !== "string" ||
        typeof failure.reason !== "string"
      ) {
        return [];
      }
      return [
        {
          mediaFileId:
            typeof failure.mediaFileId === "string" ? failure.mediaFileId : "",
          path: failure.path,
          reason: failure.reason,
        },
      ];
    },
  );
  const failed = normalizeNumber(parsed.failed);
  const notListed =
    parsed.failuresTruncated === true
      ? Math.max(failed - failures.length, 0)
      : 0;
  return { failures, notListed };
}

export function isTerminalJobRunStatus(status: JobRunStatus): boolean {
  return status === "COMPLETED" || status === "WARNING" || status === "FAILED";
}

export function preferJobRunSnapshot(
  existing: JobRun | undefined,
  incoming: JobRun,
): JobRun {
  if (!existing) {
    return incoming;
  }
  if (existing.id === incoming.id && existing.jobKey === "SOURCE_IMPORT") {
    const previous = isRecord(existing.progressJson)
      ? normalizeNumber(existing.progressJson.revision)
      : 0;
    const next = isRecord(incoming.progressJson)
      ? normalizeNumber(incoming.progressJson.revision)
      : 0;
    if (previous !== next) return next > previous ? incoming : existing;
  }
  if (
    isTerminalJobRunStatus(existing.status) &&
    !isTerminalJobRunStatus(incoming.status)
  ) {
    return existing;
  }
  if (
    existing.id === incoming.id &&
    existing.jobKey === "ACQUISITION_SEARCH" &&
    !isTerminalJobRunStatus(incoming.status)
  ) {
    const previous = isRecord(existing.progressJson)
      ? normalizeNumber(existing.progressJson.revision)
      : 0;
    const next = isRecord(incoming.progressJson)
      ? normalizeNumber(incoming.progressJson.revision)
      : 0;
    if (previous > next) return existing;
  }
  if (
    existing.completedAt &&
    incoming.completedAt &&
    existing.completedAt.localeCompare(incoming.completedAt) > 0
  ) {
    return existing;
  }
  if (
    existing.startedAt.localeCompare(incoming.startedAt) > 0 &&
    !incoming.completedAt
  ) {
    return existing;
  }
  return incoming;
}

/**
 * Fold one run into the latest run kept per job instance (see
 * {@link jobInstanceKey}). A newer snapshot of the kept run replaces it; a
 * different run replaces it only when it started no earlier.
 */
export function mergeLatestJobRun(
  current: Partial<Record<string, JobRun>>,
  run: JobRun,
): Partial<Record<string, JobRun>> {
  const key = jobInstanceKey(run);
  const existing = current[key];
  let next: JobRun;
  if (!existing) {
    next = run;
  } else if (existing.id === run.id) {
    next = preferJobRunSnapshot(existing, run);
  } else if (Date.parse(run.startedAt) >= Date.parse(existing.startedAt)) {
    next = run;
  } else {
    return current;
  }
  return next === existing ? current : { ...current, [key]: next };
}

/** The script run a custom job run produced, as recorded in its summary. */
export function scriptRunIdOf(run: Pick<JobRun, "summaryJson">): string | null {
  let parsed: unknown = run.summaryJson;
  if (typeof parsed === "string") {
    try {
      parsed = JSON.parse(parsed);
    } catch {
      return null;
    }
  }
  if (typeof parsed !== "object" || parsed === null) return null;
  const value = (parsed as Record<string, unknown>).script_run_id;
  if (typeof value === "string" && value.trim()) return value.trim();
  if (typeof value === "number" && Number.isFinite(value)) return String(value);
  return null;
}

/** Finds the script run joined to a job run through its summary. */
export function scriptRunForJobRun<T extends { id: string }>(
  run: Pick<JobRun, "summaryJson">,
  scriptRuns: readonly T[],
): T | null {
  const scriptRunId = scriptRunIdOf(run);
  if (!scriptRunId) return null;
  return scriptRuns.find((scriptRun) => scriptRun.id === scriptRunId) ?? null;
}

/** Waits between captured-output reloads; they add up to about two minutes. */
export const SCRIPT_OUTPUT_POLL_DELAYS_MS: readonly number[] = [
  1_000, 2_000, 5_000, 5_000, 10_000, 10_000, 15_000, 15_000, 15_000, 20_000,
  20_000,
];

/**
 * The newest finished run whose captured output has not arrived yet: its
 * script run is missing, or still recorded as running. Null when there is
 * nothing to wait for.
 */
export function runAwaitingScriptOutput(
  history: readonly Pick<
    JobRun,
    "id" | "status" | "summaryJson" | "startedAt"
  >[],
  scriptRuns: readonly { id: string; status: string }[],
): string | null {
  const newestFinished = history
    .filter((run) => isTerminalJobRunStatus(run.status))
    .reduce<(typeof history)[number] | null>(
      (newest, run) =>
        newest === null ||
        Date.parse(run.startedAt) > Date.parse(newest.startedAt)
          ? run
          : newest,
      null,
    );
  if (!newestFinished) return null;
  const scriptRun = scriptRunForJobRun(newestFinished, scriptRuns);
  if (scriptRun && scriptRun.status !== "running") return null;
  return newestFinished.id;
}

/** A run counts as long-running once it has been active at least this long. */
export const LONG_RUNNING_JOB_THRESHOLD_MS = 30_000;

function progressString(run: Pick<JobRun, "progressJson">, key: string) {
  const progress = run.progressJson;
  if (!isRecord(progress)) return null;
  const value = progress[key];
  return typeof value === "string" && value.trim() !== "" ? value.trim() : null;
}

/**
 * Activity jobs that have been running for at least the threshold, one per
 * job. Only location operations have a panel on Activity, so only they are
 * counted. A folder conflict task reports through its own run and runs each
 * conflict as a child location operation with a run of its own; the child is
 * part of the task, so it is not counted again.
 */
export function longRunningJobs(
  runs: readonly Pick<
    JobRun,
    "id" | "jobKey" | "status" | "startedAt" | "progressJson"
  >[],
  nowMs: number,
): { count: number; nextChangeAtMs: number | null } {
  const active = runs.filter(
    (run) =>
      run.jobKey === "LOCATION_OPERATION" &&
      (run.status === "RUNNING" || run.status === "DISCOVERING"),
  );
  const childOperationIds = new Set(
    active
      .map((run) => progressString(run, "current_operation_id"))
      .filter((id): id is string => id !== null),
  );
  let count = 0;
  let nextChangeAtMs: number | null = null;
  for (const run of active) {
    const operationId = progressString(run, "operationId");
    if (operationId !== null && childOperationIds.has(operationId)) continue;
    const startedAtMs = Date.parse(run.startedAt);
    if (!Number.isFinite(startedAtMs)) continue;
    const longRunningAtMs = startedAtMs + LONG_RUNNING_JOB_THRESHOLD_MS;
    if (longRunningAtMs <= nowMs) {
      count += 1;
    } else if (nextChangeAtMs === null || longRunningAtMs < nextChangeAtMs) {
      nextChangeAtMs = longRunningAtMs;
    }
  }
  return { count, nextChangeAtMs };
}

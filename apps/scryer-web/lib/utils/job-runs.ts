import type {
  JobCategory,
  JobKey,
  JobRun,
  JobRunStatus,
  JobSection,
  JobTriggerSource,
  LibraryScanMode,
  LibraryScanProgress,
  LibraryScanStatus,
} from "@/lib/types";

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function normalizeLibraryScanFacet(value: unknown): LibraryScanProgress["facet"] {
  return value === "anime" ? "anime" : value === "tv" ? "tv" : "movie";
}

function normalizeLibraryScanMode(value: unknown): LibraryScanMode {
  return value === "additive" ? "additive" : "full";
}

function normalizeLibraryScanStatus(value: unknown): LibraryScanStatus {
  switch (value) {
    case "discovering":
    case "running":
    case "completed":
    case "warning":
    case "failed":
      return value;
    default:
      return "running";
  }
}

export function normalizeJobRunStatus(value: unknown): JobRunStatus {
  switch (value) {
    case "queued":
    case "discovering":
    case "running":
    case "completed":
    case "warning":
    case "failed":
      return value;
    default:
      return "running";
  }
}

function normalizeNumber(value: unknown): number {
  return typeof value === "number" && Number.isFinite(value) ? value : 0;
}

function normalizeJobKey(value: unknown): JobKey {
  return typeof value === "string" ? (value as JobKey) : "rss_sync";
}

function normalizeCategory(value: unknown): JobCategory {
  switch (value) {
    case "library":
    case "acquisition":
    case "maintenance":
    case "subtitles":
    case "system":
      return value;
    default:
      return "system";
  }
}

function normalizeSection(value: unknown): JobSection {
  return value === "maintenance" ? "maintenance" : "primary";
}

function normalizeTriggerSource(value: unknown): JobTriggerSource {
  switch (value) {
    case "manual":
    case "scheduled_startup":
    case "scheduled_interval":
    case "system_internal":
      return value;
    default:
      return "manual";
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
    metadataTotalKnown: value.metadataTotalKnown === true,
    fileTotalKnown: value.fileTotalKnown === true,
    metadataProgress: {
      total: normalizeNumber(
        isRecord(value.metadataProgress) ? value.metadataProgress.total : 0,
      ),
      completed: normalizeNumber(
        isRecord(value.metadataProgress) ? value.metadataProgress.completed : 0,
      ),
      failed: normalizeNumber(
        isRecord(value.metadataProgress) ? value.metadataProgress.failed : 0,
      ),
    },
    fileProgress: {
      total: normalizeNumber(
        isRecord(value.fileProgress) ? value.fileProgress.total : 0,
      ),
      completed: normalizeNumber(
        isRecord(value.fileProgress) ? value.fileProgress.completed : 0,
      ),
      failed: normalizeNumber(
        isRecord(value.fileProgress) ? value.fileProgress.failed : 0,
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
    displayName: typeof value.displayName === "string" ? value.displayName : "Job",
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
    summaryText: typeof value.summaryText === "string" ? value.summaryText : null,
    errorText: typeof value.errorText === "string" ? value.errorText : null,
    progressJson:
      typeof value.progressJson === "string" ? value.progressJson : null,
    libraryScanProgress: normalizeLibraryScanProgress(value.libraryScanProgress),
  };
}

export function isTerminalJobRunStatus(status: JobRunStatus): boolean {
  return status === "completed" || status === "warning" || status === "failed";
}

export function preferJobRunSnapshot(
  existing: JobRun | undefined,
  incoming: JobRun,
): JobRun {
  if (!existing) {
    return incoming;
  }
  if (isTerminalJobRunStatus(existing.status) && !isTerminalJobRunStatus(incoming.status)) {
    return existing;
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

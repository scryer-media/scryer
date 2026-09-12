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
        isRecord(value.titleMatchProgress) ? value.titleMatchProgress.completed : 0,
      ),
      failed: normalizeNumber(
        isRecord(value.titleMatchProgress) ? value.titleMatchProgress.failed : 0,
      ),
    },
    hydrationProgress: {
      total: normalizeNumber(
        isRecord(value.hydrationProgress) ? value.hydrationProgress.total : 0,
      ),
      completed: normalizeNumber(
        isRecord(value.hydrationProgress) ? value.hydrationProgress.completed : 0,
      ),
      failed: normalizeNumber(
        isRecord(value.hydrationProgress) ? value.hydrationProgress.failed : 0,
      ),
    },
    mediaAnalysisProgress: {
      total: normalizeNumber(
        isRecord(value.mediaAnalysisProgress) ? value.mediaAnalysisProgress.total : 0,
      ),
      completed: normalizeNumber(
        isRecord(value.mediaAnalysisProgress)
          ? value.mediaAnalysisProgress.completed
          : 0,
      ),
      failed: normalizeNumber(
        isRecord(value.mediaAnalysisProgress) ? value.mediaAnalysisProgress.failed : 0,
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
    summaryJson: value.summaryJson ?? null,
    summaryText: typeof value.summaryText === "string" ? value.summaryText : null,
    errorText: typeof value.errorText === "string" ? value.errorText : null,
    progressJson: value.progressJson ?? null,
    libraryScanProgress: normalizeLibraryScanProgress(value.libraryScanProgress),
  };
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
  if (isTerminalJobRunStatus(existing.status) && !isTerminalJobRunStatus(incoming.status)) {
    return existing;
  }
  if (existing.id === incoming.id && existing.jobKey === "ACQUISITION_SEARCH" &&
      !isTerminalJobRunStatus(incoming.status)) {
    const previous = isRecord(existing.progressJson) ? normalizeNumber(existing.progressJson.revision) : 0;
    const next = isRecord(incoming.progressJson) ? normalizeNumber(incoming.progressJson.revision) : 0;
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

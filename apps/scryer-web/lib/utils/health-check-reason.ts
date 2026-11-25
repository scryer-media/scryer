type Translate = (
  key: string,
  values?: Record<string, string | number | boolean | null | undefined>,
) => string;

/** Labels for the health-check reasons the server sends with their counts. */
export const HEALTH_CHECK_REASON_KEYS: Record<string, string> = {
  ARCHIVE_WORKSPACE_SCRATCH_IN_USE:
    "jobs.healthCheckReason.ARCHIVE_WORKSPACE_SCRATCH_IN_USE",
  ARCHIVE_WORKSPACE_EXTRACTIONS_BLOCKED:
    "jobs.healthCheckReason.ARCHIVE_WORKSPACE_EXTRACTIONS_BLOCKED",
  ARCHIVE_WORKSPACE_UNMARKED_ENTRIES:
    "jobs.healthCheckReason.ARCHIVE_WORKSPACE_UNMARKED_ENTRIES",
};

export type HealthCheckFacts = Record<string, number>;

/** Read a check's facts: counts by name; anything else is dropped. */
export function parseHealthCheckFacts(value: unknown): HealthCheckFacts {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return {};
  }
  return Object.fromEntries(
    Object.entries(value).filter(
      (entry): entry is [string, number] =>
        typeof entry[1] === "number" && Number.isFinite(entry[1]),
    ),
  );
}

/**
 * The text for a health-check result: its reason's label with the counts
 * filled in, or the server's sentence for a check without a reason this
 * client knows.
 */
export function healthCheckMessage(
  issue: { reason?: string | null; facts?: HealthCheckFacts; message: string },
  t: Translate,
): string {
  const key =
    issue.reason && Object.hasOwn(HEALTH_CHECK_REASON_KEYS, issue.reason)
      ? HEALTH_CHECK_REASON_KEYS[issue.reason]
      : null;
  return key ? t(key, issue.facts ?? {}) : issue.message;
}

import type { Translate } from "@/components/root/types";

/**
 * One rejection bucket from the backend's `NoAutoEligibleRelease` error,
 * carried in the GraphQL error extensions as `autoDecisionReasons`.
 */
export type AutoSearchRejectionReason = {
  code: string;
  summary: string;
  count: number;
  /** Rule codes that vetoed the release when `code` is `quality_blocked`. */
  blockCodes: string[];
};

export type AutoSearchRejection = {
  candidateCount: number;
  reasons: AutoSearchRejectionReason[];
};

const MAX_REASONS_IN_MESSAGE = 3;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function parseReason(value: unknown): AutoSearchRejectionReason | null {
  if (!isRecord(value) || typeof value.code !== "string") {
    return null;
  }
  const count =
    typeof value.count === "number" && Number.isFinite(value.count)
      ? Math.max(0, Math.trunc(value.count))
      : 0;
  const blockCodes = Array.isArray(value.blockCodes)
    ? value.blockCodes.filter((code): code is string => typeof code === "string" && code.trim() !== "")
    : [];
  return {
    code: value.code,
    summary: typeof value.summary === "string" ? value.summary : "",
    count,
    blockCodes,
  };
}

/**
 * Extract the automatic-search rejection diagnostics from a failed
 * `queueBestRelease` mutation, or `null` when the error is something else.
 */
export function autoSearchRejectionFromError(error: unknown): AutoSearchRejection | null {
  if (!isRecord(error) || !Array.isArray(error.graphQLErrors)) {
    return null;
  }
  for (const graphQlError of error.graphQLErrors) {
    if (!isRecord(graphQlError) || !isRecord(graphQlError.extensions)) {
      continue;
    }
    const { extensions } = graphQlError;
    if (!Array.isArray(extensions.autoDecisionReasons)) {
      continue;
    }
    const reasons = extensions.autoDecisionReasons
      .map(parseReason)
      .filter((reason): reason is AutoSearchRejectionReason => reason !== null);
    const rawCount = extensions.autoCandidateCount;
    const candidateCount =
      typeof rawCount === "number" && Number.isFinite(rawCount)
        ? Math.max(0, Math.trunc(rawCount))
        : reasons.reduce((total, reason) => total + reason.count, 0);
    return { candidateCount, reasons };
  }
  return null;
}

function formatReason(reason: AutoSearchRejectionReason): string {
  const label = reason.blockCodes.length > 0
    ? `${reason.summary}: ${reason.blockCodes.join(", ")}`
    : reason.summary;
  return `${label} (${reason.count})`;
}

/**
 * Build the status line shown when an automatic search ran but queued
 * nothing. Returns `null` when the error is not that outcome, so callers fall
 * back to their generic failure message.
 */
export function autoSearchOutcomeMessage(
  error: unknown,
  t: Translate,
  name: string,
): string | null {
  const rejection = autoSearchRejectionFromError(error);
  if (!rejection) {
    return null;
  }
  if (rejection.candidateCount === 0 || rejection.reasons.length === 0) {
    return t("status.autoSearchNoCandidates", { name });
  }
  const reasons = rejection.reasons
    .slice(0, MAX_REASONS_IN_MESSAGE)
    .map(formatReason)
    .join("; ");
  return t("status.autoSearchAllRejected", {
    name,
    count: rejection.candidateCount,
    reasons,
  });
}

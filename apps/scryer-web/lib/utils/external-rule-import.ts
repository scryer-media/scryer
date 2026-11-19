import {
  ERROR_REASON_KEYS,
  type ErrorTranslate,
} from "../graphql/error-message.ts";
import type {
  ExternalRuleImportCounts,
  ExternalRuleImportFact,
  ExternalRuleImportItem,
  ExternalRuleImportItemState,
  ExternalRuleImportPhase,
  ExternalRuleImportSource,
  ExternalRuleImportStatus,
} from "../types/external-import.ts";
import { normalizeJobRunStatus } from "./job-runs.ts";

const SOURCES: readonly ExternalRuleImportSource[] = [
  "SONARR",
  "RADARR",
  "MAINTAINERR",
];
const PHASES: readonly ExternalRuleImportPhase[] = [
  "QUEUED",
  "FETCHING",
  "PLANNING",
  "SAVING",
  "FINISHED",
];
const ITEM_STATES: readonly ExternalRuleImportItemState[] = [
  "PENDING",
  "SAVED",
  "SAVED_NEEDS_REVIEW",
  "SAVED_FAILED_TRANSLATION",
  "SKIPPED_EXISTS",
  "FAILED",
];

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function text(value: unknown): string | null {
  return typeof value === "string" ? value : null;
}

function count(value: unknown): number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0
    ? value
    : 0;
}

function oneOf<T extends string>(
  value: unknown,
  allowed: readonly T[],
  fallback: T,
): T {
  return typeof value === "string" &&
    (allowed as readonly string[]).includes(value)
    ? (value as T)
    : fallback;
}

function normalizeFacts(value: unknown): ExternalRuleImportFact[] {
  if (!Array.isArray(value)) return [];
  return value.flatMap((fact) =>
    isRecord(fact) && typeof fact.code === "string"
      ? [
          {
            code: fact.code,
            message: text(fact.message) ?? "",
            details: fact.details ?? null,
          },
        ]
      : [],
  );
}

function normalizeItem(value: unknown): ExternalRuleImportItem | null {
  if (!isRecord(value) || typeof value.key !== "string") return null;
  return {
    key: value.key,
    sourceName: text(value.sourceName) ?? value.key,
    savedName: text(value.savedName),
    ruleSetId: text(value.ruleSetId),
    // An unknown future state is shown as failed rather than as saved.
    state: oneOf(value.state, ITEM_STATES, "FAILED"),
    reason: text(value.reason),
    message: text(value.message),
    facts: normalizeFacts(value.facts),
  };
}

function normalizeCounts(value: unknown): ExternalRuleImportCounts {
  const counts = isRecord(value) ? value : {};
  return {
    total: count(counts.total),
    pending: count(counts.pending),
    saved: count(counts.saved),
    savedNeedsReview: count(counts.savedNeedsReview),
    savedFailedTranslation: count(counts.savedFailedTranslation),
    skippedExists: count(counts.skippedExists),
    failed: count(counts.failed),
  };
}

/** Reads an `externalRuleImportStatus` payload; null when it is not one. */
export function normalizeExternalRuleImportStatus(
  value: unknown,
): ExternalRuleImportStatus | null {
  if (!isRecord(value) || typeof value.runId !== "string") return null;
  return {
    runId: value.runId,
    status: normalizeJobRunStatus(value.status),
    active: value.active === true,
    revision: count(value.revision),
    updatedAt: text(value.updatedAt) ?? "",
    source: oneOf(value.source, SOURCES, "SONARR"),
    baseUrl: text(value.baseUrl) ?? "",
    phase: oneOf(value.phase, PHASES, "FINISHED"),
    reason: text(value.reason),
    message: text(value.message),
    counts: normalizeCounts(value.counts),
    items: Array.isArray(value.items)
      ? value.items.flatMap((item) => normalizeItem(item) ?? [])
      : [],
    facts: normalizeFacts(value.facts),
  };
}

/**
 * Display copy for a reason in a rule import report. The code decides the
 * copy; the server sentence is shown only for a code this client does not know.
 */
export function ruleImportReasonMessage(
  reason: string | null,
  serverMessage: string | null,
  fallback: string,
  translate: ErrorTranslate,
): string {
  const key = reason ? ERROR_REASON_KEYS[reason] : undefined;
  if (key) return translate(key);
  return serverMessage?.trim() || fallback;
}

/** Fact code of an item whose profiles name no Scryer quality profile. */
export const RULE_IMPORT_PROFILE_SCORES_UNMATCHED =
  "RULE_IMPORT_PROFILE_SCORES_UNMATCHED";

/** Facts that describe an item's scoring rather than a translator finding. */
const SCORING_FACT_CODES: ReadonlySet<string> = new Set([
  RULE_IMPORT_PROFILE_SCORES_UNMATCHED,
  "RULE_IMPORT_PROFILE_SCORES_APPLIED",
  "RULE_IMPORT_FORMAT_UNSCORED",
]);

function profileNamesDetail(
  item: ExternalRuleImportItem,
  field: "unmatchedProfiles" | "duplicateProfiles",
): string[] {
  const fact = item.facts.find(
    (candidate) => candidate.code === RULE_IMPORT_PROFILE_SCORES_UNMATCHED,
  );
  const names = isRecord(fact?.details) ? fact.details[field] : null;
  return Array.isArray(names)
    ? names.filter((name): name is string => typeof name === "string")
    : [];
}

/** The Arr profile names a review item could not match, in source order. */
export function unmatchedProfileNames(item: ExternalRuleImportItem): string[] {
  return profileNamesDetail(item, "unmatchedProfiles");
}

/** Arr profile names the source repeats with different scores, in source order. */
export function duplicateProfileNames(item: ExternalRuleImportItem): string[] {
  return profileNamesDetail(item, "duplicateProfiles");
}

/** A translator finding attached to an item, with where in the format it is. */
export type RuleImportDiagnostic = {
  code: string;
  message: string;
  location: string | null;
  /** Zero-based index of the condition the finding is about, if any. */
  specificationIndex: number | null;
};

/** Every fact of an item except those describing its scoring. */
export function ruleImportDiagnostics(
  item: ExternalRuleImportItem,
): RuleImportDiagnostic[] {
  return item.facts.flatMap((fact) => {
    if (SCORING_FACT_CODES.has(fact.code)) return [];
    const details = isRecord(fact.details) ? fact.details : {};
    const index = details.specificationIndex;
    return [
      {
        code: fact.code,
        message: fact.message,
        location: text(details.location),
        specificationIndex:
          typeof index === "number" && Number.isSafeInteger(index) && index >= 0
            ? index
            : null,
      },
    ];
  });
}

/** Phase and counts from a running import's job-run progress. */
export function ruleImportProgress(value: unknown): {
  phase: ExternalRuleImportPhase;
  counts: ExternalRuleImportCounts;
} | null {
  if (!isRecord(value)) return null;
  return {
    phase: oneOf(value.phase, PHASES, "QUEUED"),
    counts: normalizeCounts(value.counts),
  };
}

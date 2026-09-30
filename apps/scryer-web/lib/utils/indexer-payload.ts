import type { IndexerDraft, IndexerRecord } from "@/lib/types/indexers";

/**
 * Reads the query-budget field as typed. Blank means no budget; anything else
 * must be a whole number of at least 1, which is what the server accepts.
 */
export function parseIndexerQueryBudget(
  raw: string,
): { valid: true; value: number | null } | { valid: false } {
  const trimmed = raw.trim();
  if (trimmed === "") {
    return { valid: true, value: null };
  }
  if (!/^\d+$/.test(trimmed)) {
    return { valid: false };
  }
  const value = Number(trimmed);
  if (!Number.isSafeInteger(value) || value < 1) {
    return { valid: false };
  }
  return { valid: true, value };
}

/**
 * Reads the request-interval field as typed. Blank means no interval, which
 * the scheduler treats the same as zero seconds, so blank saves as 0 rather
 * than leaving a previous interval in place.
 */
export function parseIndexerRateLimitSeconds(
  raw: string,
): { valid: true; value: number } | { valid: false } {
  const trimmed = raw.trim();
  if (trimmed === "") {
    return { valid: true, value: 0 };
  }
  if (!/^\d+$/.test(trimmed)) {
    return { valid: false };
  }
  const value = Number(trimmed);
  if (!Number.isSafeInteger(value)) {
    return { valid: false };
  }
  return { valid: true, value };
}

/** The editor's text for a saved indexer's budget. */
export function indexerQueryBudgetDraftValue(
  indexer: Pick<IndexerRecord, "maxQueriesPerMinute">,
): string {
  return indexer.maxQueriesPerMinute == null
    ? ""
    : String(indexer.maxQueriesPerMinute);
}

/** The editor's text for a saved indexer's request interval; 0 shows blank. */
export function indexerRateLimitSecondsDraftValue(
  indexer: Pick<IndexerRecord, "rateLimitSeconds">,
): string {
  return indexer.rateLimitSeconds == null || indexer.rateLimitSeconds <= 0
    ? ""
    : String(indexer.rateLimitSeconds);
}

/**
 * The fields the create and update indexer mutations share, built from the
 * editor draft. An unparsable budget or interval becomes null here; callers
 * check {@link parseIndexerQueryBudget} and
 * {@link parseIndexerRateLimitSeconds} first and refuse to save it.
 */
export function buildIndexerSavePayload<TConfig>(
  draft: IndexerDraft,
  providerType: string,
  config: TConfig,
) {
  const budget = parseIndexerQueryBudget(draft.maxQueriesPerMinute);
  const interval = parseIndexerRateLimitSeconds(draft.rateLimitSeconds);
  return {
    name: draft.name.trim(),
    providerType,
    proxyConfigId: draft.proxyConfigId,
    downloadClientId: draft.downloadClientId,
    seedingProfileId: draft.seedingProfileId,
    rateLimitSeconds: interval.valid ? interval.value : null,
    maxQueriesPerMinute: budget.valid ? budget.value : null,
    isEnabled: draft.isEnabled,
    enableInteractiveSearch: draft.enableInteractiveSearch,
    enableAutoSearch: draft.enableAutoSearch,
    config,
  };
}

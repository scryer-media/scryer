import type { Translate } from "@/components/root/types";
import type { InteractiveSearchIndexerProgress } from "@/lib/graphql/release-search";
import { ERROR_REASON_KEYS } from "../graphql/error-message.ts";
import { INDEXER_QUERY_BUDGET_REASON_KEYS } from "./interactive-search-presentation.ts";

/** What a search reports about why it did not ask an indexer. */
export type IndexerSkipFacts = Pick<
  InteractiveSearchIndexerProgress,
  | "skipReason"
  | "skippedUntil"
  | "failureReason"
  | "failureReasonCode"
  | "retryAfterSeconds"
>;

/**
 * Why an indexer was not asked, in the viewer's language: the skip reason,
 * else a known reason code, else the server's sentence. A wait that lifts on
 * its own reads as the viewer's local time when the server sent the instant,
 * or as a duration when it sent only the delay. `formatTime` turns the
 * server's instant into the viewer's local time.
 */
export function indexerSkipReasonText(
  entry: IndexerSkipFacts,
  t: Translate,
  formatTime: (instant: string) => string,
): string | undefined {
  const until = entry.skippedUntil ? formatTime(entry.skippedUntil) : null;
  switch (entry.skipReason) {
    case "INDEXER_DISABLED":
      return t("indexerSearch.skipReason.indexerDisabled");
    case "TEMPORARILY_DISABLED":
      return until
        ? t("indexerSearch.skipReason.temporarilyDisabledUntil", {
            time: until,
          })
        : t("indexerSearch.skipReason.temporarilyDisabled");
    case "BACKED_OFF":
      return until
        ? t("indexerSearch.skipReason.backedOffUntil", { time: until })
        : t("indexerSearch.skipReason.backedOff");
    case "NO_TEXT_SEARCH":
      return t("indexerSearch.skipReason.noTextSearch");
    case "QUERY_BUDGET":
      if (until) {
        return t("indexerSearch.skipReason.queryBudgetUntil", { time: until });
      }
      return entry.retryAfterSeconds != null
        ? t("indexerSearch.skipReason.queryBudgetRetryAfter", {
            seconds: entry.retryAfterSeconds,
          })
        : t("indexerSearch.skipReason.queryBudget");
    case "MONTHLY_LIMIT": {
      const code = entry.failureReasonCode;
      return code && Object.hasOwn(ERROR_REASON_KEYS, code)
        ? t(ERROR_REASON_KEYS[code]!)
        : (entry.failureReason ?? undefined);
    }
    default: {
      const code = entry.failureReasonCode;
      return code && Object.hasOwn(INDEXER_QUERY_BUDGET_REASON_KEYS, code)
        ? t(INDEXER_QUERY_BUDGET_REASON_KEYS[code]!)
        : (entry.failureReason ?? undefined);
    }
  }
}

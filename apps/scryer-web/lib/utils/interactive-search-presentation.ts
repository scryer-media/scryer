import type { InteractiveSearchIndexerProgress } from "@/lib/graphql/release-search";
import {
  summarizeUnroutableResults,
  type UnroutableResultsSummary,
} from "./unroutable-results.ts";

/** `failureReasonCode` of an indexer its query budget stopped part way through. */
export const QUERY_BUDGET_PARTLY_SEARCHED = "INDEXER_QUERY_BUDGET_PARTLY_SEARCHED";

/**
 * Why an indexer's query budget cut a search short, as the server sends it in
 * an indexer's `failureReasonCode`. A reason this client does not know shows
 * the server's sentence.
 */
export const INDEXER_QUERY_BUDGET_REASON_KEYS: Readonly<
  Record<string, string>
> = {
  INDEXER_QUERY_BUDGET_NOT_SEARCHED: "indexerSearch.skipReason.queryBudget",
  [QUERY_BUDGET_PARTLY_SEARCHED]: "indexerSearch.queryBudget.partlySearched",
};

type InteractiveSearchPresentationInput = {
  hasSnapshot: boolean;
  loading: boolean;
  resultCount: number;
  indexers: InteractiveSearchIndexerProgress[];
};

export type InteractiveSearchPresentation = {
  showInitialLoader: boolean;
  showResults: boolean;
  showProgress: boolean;
  showFinalSummary: boolean;
  /** Indexers that have finished, whichever way (completed, failed, or skipped). */
  completedIndexerCount: number;
  /** Indexers that actually ran a search and answered. */
  searchedIndexerCount: number;
  totalIndexerCount: number;
  failedIndexerNames: string[];
  /**
   * Indexers the run never queried (routing/scope/capability, budget), with
   * the server's sentence when known and the coded facts to translate it.
   */
  skippedIndexers: Array<{
    name: string;
    reason: string | null;
    skipReason: InteractiveSearchIndexerProgress["skipReason"] | null;
    skippedUntil: string | null;
    failureReasonCode: string | null;
    retryAfterSeconds: number | null;
  }>;
  /**
   * Indexers that answered some requests before their query budget stopped
   * the rest, with the seconds until they can be asked again when known.
   * Their results are kept.
   */
  partlySearchedIndexers: Array<{
    name: string;
    retryAfterSeconds: number | null;
  }>;
  /** Releases set aside because no enabled download client accepts them, per reason. */
  unroutableResults: UnroutableResultsSummary[];
};

export function deriveInteractiveSearchPresentation({
  hasSnapshot,
  loading,
  resultCount,
  indexers,
}: InteractiveSearchPresentationInput): InteractiveSearchPresentation {
  const completedIndexerCount = indexers.filter(
    (indexer) =>
      indexer.status === "COMPLETED" ||
      indexer.status === "FAILED" ||
      indexer.status === "SKIPPED",
  ).length;
  const searchedIndexerCount = indexers.filter(
    (indexer) => indexer.status === "COMPLETED",
  ).length;

  return {
    showInitialLoader: loading && resultCount === 0,
    showResults: resultCount > 0,
    showProgress: loading,
    showFinalSummary: hasSnapshot && !loading,
    completedIndexerCount,
    searchedIndexerCount,
    totalIndexerCount: indexers.length,
    failedIndexerNames: indexers
      .filter((indexer) => indexer.status === "FAILED")
      .map((indexer) => indexer.name),
    skippedIndexers: indexers
      .filter((indexer) => indexer.status === "SKIPPED")
      .map((indexer) => ({
        name: indexer.name,
        reason: indexer.failureReason,
        skipReason: indexer.skipReason ?? null,
        skippedUntil: indexer.skippedUntil ?? null,
        failureReasonCode: indexer.failureReasonCode ?? null,
        retryAfterSeconds: indexer.retryAfterSeconds ?? null,
      })),
    partlySearchedIndexers: indexers
      .filter(
        (indexer) =>
          indexer.status === "COMPLETED" &&
          indexer.failureReasonCode === QUERY_BUDGET_PARTLY_SEARCHED,
      )
      .map((indexer) => ({
        name: indexer.name,
        retryAfterSeconds: indexer.retryAfterSeconds ?? null,
      })),
    unroutableResults: summarizeUnroutableResults(indexers),
  };
}

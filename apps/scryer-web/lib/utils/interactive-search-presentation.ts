import type { InteractiveSearchIndexerProgress } from "@/lib/graphql/release-search";

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
  /** Indexers the run never queried (routing/scope/capability), with the reason when known. */
  skippedIndexers: Array<{ name: string; reason: string | null }>;
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
      })),
  };
}

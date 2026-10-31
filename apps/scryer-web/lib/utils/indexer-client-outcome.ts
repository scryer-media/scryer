import type { IndexerClientRequestOutcome } from "../graphql/indexer-clients.ts";

export const INDEXER_CLIENT_OUTCOME_KEYS: Readonly<
  Record<IndexerClientRequestOutcome, string>
> = {
  CAPS_READY: "indexerClients.outcome.caps",
  SEARCH_READY: "indexerClients.outcome.search",
  SEARCH_PARTIAL: "indexerClients.outcome.partial",
  ARTIFACT_READY: "indexerClients.outcome.artifact",
  REFUSED: "indexerClients.outcome.refused",
  UNAVAILABLE: "indexerClients.outcome.unavailable",
  FAILED: "indexerClients.outcome.failed",
};

/** A request outcome in the viewer's language; a value this client does not know reads as unknown. */
export function indexerClientOutcomeMessage(
  outcome: string,
  t: (key: string) => string,
): string {
  return t(
    Object.hasOwn(INDEXER_CLIENT_OUTCOME_KEYS, outcome)
      ? INDEXER_CLIENT_OUTCOME_KEYS[outcome as IndexerClientRequestOutcome]
      : "indexerClients.outcome.unknown",
  );
}

import assert from "node:assert/strict";
import test from "node:test";
import { indexerClientOutcomeMessage } from "./indexer-client-outcome.ts";

test("client outcomes translate enum values and read unknown values as unknown", () => {
  const t = (key: string) => `translated:${key}`;
  assert.equal(
    indexerClientOutcomeMessage("SEARCH_PARTIAL", t),
    "translated:indexerClients.outcome.partial",
  );
  assert.equal(
    indexerClientOutcomeMessage("UNAVAILABLE", t),
    "translated:indexerClients.outcome.unavailable",
  );
  assert.equal(
    indexerClientOutcomeMessage("FUTURE_OUTCOME", t),
    "translated:indexerClients.outcome.unknown",
  );
  assert.equal(
    indexerClientOutcomeMessage("toString", t),
    "translated:indexerClients.outcome.unknown",
  );
});

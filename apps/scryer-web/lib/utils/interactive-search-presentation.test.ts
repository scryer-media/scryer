import assert from "node:assert/strict";
import test from "node:test";

import type { InteractiveSearchIndexerProgress } from "@/lib/graphql/release-search";
import { deriveInteractiveSearchPresentation } from "./interactive-search-presentation.ts";

function indexer(
  name: string,
  status: InteractiveSearchIndexerProgress["status"],
): InteractiveSearchIndexerProgress {
  return {
    indexerId: name.toLowerCase(),
    name,
    priority: 0,
    status,
    resultCount: 0,
    elapsedMs: null,
    failureReason: null,
  };
}

test("running search with no results shows only the initial loader", () => {
  const presentation = deriveInteractiveSearchPresentation({
    hasSnapshot: true,
    loading: true,
    resultCount: 0,
    indexers: [indexer("Fast", "SEARCHING")],
  });

  assert.equal(presentation.showInitialLoader, true);
  assert.equal(presentation.showResults, false);
  assert.equal(presentation.showProgress, true);
});

test("running search with partial results shows results and live progress", () => {
  const presentation = deriveInteractiveSearchPresentation({
    hasSnapshot: true,
    loading: true,
    resultCount: 3,
    indexers: [indexer("Fast", "COMPLETED"), indexer("Slow", "SEARCHING")],
  });

  assert.equal(presentation.showInitialLoader, false);
  assert.equal(presentation.showResults, true);
  assert.equal(presentation.showProgress, true);
  assert.equal(presentation.completedIndexerCount, 1);
  assert.equal(presentation.totalIndexerCount, 2);
});

test("completed search shows results and the final summary", () => {
  const presentation = deriveInteractiveSearchPresentation({
    hasSnapshot: true,
    loading: false,
    resultCount: 4,
    indexers: [indexer("Fast", "COMPLETED")],
  });

  assert.equal(presentation.showInitialLoader, false);
  assert.equal(presentation.showResults, true);
  assert.equal(presentation.showProgress, false);
  assert.equal(presentation.showFinalSummary, true);
});

test("failed and skipped indexers count as finished and are each reported", () => {
  const presentation = deriveInteractiveSearchPresentation({
    hasSnapshot: true,
    loading: false,
    resultCount: 2,
    indexers: [indexer("Broken", "FAILED"), indexer("Disabled", "SKIPPED")],
  });

  assert.equal(presentation.completedIndexerCount, 2);
  assert.equal(presentation.totalIndexerCount, 2);
  assert.deepEqual(presentation.failedIndexerNames, ["Broken"]);
  assert.deepEqual(presentation.skippedIndexers, [{ name: "Disabled", reason: null }]);
});

test("the final summary counts indexers that searched, not sources that returned results", () => {
  // Four indexers ran: two answered (one of them with nothing), one failed,
  // one was skipped. "2 sources in the results" is not the story — 2/4
  // searched, 1 failed, 1 skipped is.
  const presentation = deriveInteractiveSearchPresentation({
    hasSnapshot: true,
    loading: false,
    resultCount: 15,
    indexers: [
      indexer("Fast", "COMPLETED"),
      indexer("Empty", "COMPLETED"),
      indexer("Broken", "FAILED"),
      { ...indexer("Cooling", "SKIPPED"), failureReason: "temporarily disabled" },
    ],
  });

  assert.equal(presentation.searchedIndexerCount, 2);
  assert.equal(presentation.completedIndexerCount, 4);
  assert.equal(presentation.totalIndexerCount, 4);
  assert.deepEqual(presentation.failedIndexerNames, ["Broken"]);
  assert.deepEqual(presentation.skippedIndexers, [
    { name: "Cooling", reason: "temporarily disabled" },
  ]);
});

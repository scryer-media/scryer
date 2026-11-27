import assert from "node:assert/strict";
import test from "node:test";

import { indexerSkipReasonText, type IndexerSkipFacts } from "./indexer-skip-reason.ts";

// Echoes the key and its values, so a test sees what would be translated.
const t = (key: string, values?: Record<string, unknown>) =>
  values ? `${key} ${JSON.stringify(values)}` : key;
const localTime = (instant: string) => `local(${instant})`;

const budgetSkip: IndexerSkipFacts = {
  skipReason: "QUERY_BUDGET",
  skippedUntil: null,
  failureReason: "not asked: over its query budget until 2026-01-01T00:00:40+00:00",
  failureReasonCode: "INDEXER_QUERY_BUDGET_NOT_SEARCHED",
  retryAfterSeconds: null,
};

test("a query budget skip reads in the viewer's local time, not the server sentence", () => {
  assert.equal(
    indexerSkipReasonText(
      { ...budgetSkip, skippedUntil: "2026-01-01T00:00:40Z", retryAfterSeconds: 40 },
      t,
      localTime,
    ),
    'indexerSearch.skipReason.queryBudgetUntil {"time":"local(2026-01-01T00:00:40Z)"}',
  );
});

test("a query budget skip with only a delay reads as a duration", () => {
  assert.equal(
    indexerSkipReasonText({ ...budgetSkip, retryAfterSeconds: 40 }, t, localTime),
    'indexerSearch.skipReason.queryBudgetRetryAfter {"seconds":40}',
  );
  assert.equal(
    indexerSkipReasonText(budgetSkip, t, localTime),
    "indexerSearch.skipReason.queryBudget",
  );
});

test("a known query budget code is translated without a skip reason", () => {
  assert.equal(
    indexerSkipReasonText({ ...budgetSkip, skipReason: null }, t, localTime),
    "indexerSearch.skipReason.queryBudget",
  );
});

test("an unknown code falls back to the server sentence", () => {
  assert.equal(
    indexerSkipReasonText(
      {
        skipReason: null,
        skippedUntil: null,
        failureReason: "routed elsewhere",
        failureReasonCode: "SOME_FUTURE_CODE",
        retryAfterSeconds: null,
      },
      t,
      localTime,
    ),
    "routed elsewhere",
  );
});

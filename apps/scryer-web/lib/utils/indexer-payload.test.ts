import assert from "node:assert/strict";
import test from "node:test";

import type { IndexerDraft } from "@/lib/types/indexers";

import {
  buildIndexerSavePayload,
  indexerQueryBudgetDraftValue,
  parseIndexerQueryBudget,
} from "./indexer-payload.ts";

function draft(overrides: Partial<IndexerDraft> = {}): IndexerDraft {
  return {
    name: "  Synthetic Indexer  ",
    providerType: "newznab",
    proxyConfigId: null,
    downloadClientId: null,
    seedingProfileId: null,
    storedSecretKeys: [],
    maxQueriesPerMinute: "",
    isEnabled: true,
    enableInteractiveSearch: true,
    enableAutoSearch: true,
    configValues: {},
    ...overrides,
  };
}

test("the save payload carries the typed query budget", () => {
  const payload = buildIndexerSavePayload(
    draft({ maxQueriesPerMinute: " 12 " }),
    "newznab",
    undefined,
  );
  assert.equal(payload.maxQueriesPerMinute, 12);
  assert.equal(payload.name, "Synthetic Indexer");
});

test("a blank query budget saves as null so an update clears it", () => {
  const payload = buildIndexerSavePayload(draft(), "newznab", undefined);
  assert.equal(payload.maxQueriesPerMinute, null);
});

test("only whole numbers of at least one are budgets", () => {
  assert.deepEqual(parseIndexerQueryBudget(""), { valid: true, value: null });
  assert.deepEqual(parseIndexerQueryBudget("30"), { valid: true, value: 30 });
  for (const raw of ["0", "-3", "2.5", "ten", "1e3"]) {
    assert.deepEqual(parseIndexerQueryBudget(raw), { valid: false }, raw);
  }
});

test("a saved budget seeds the editor and a missing one leaves it blank", () => {
  assert.equal(indexerQueryBudgetDraftValue({ maxQueriesPerMinute: 45 }), "45");
  assert.equal(indexerQueryBudgetDraftValue({ maxQueriesPerMinute: null }), "");
});

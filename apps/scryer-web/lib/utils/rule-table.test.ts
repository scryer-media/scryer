import assert from "node:assert/strict";
import test from "node:test";

import {
  sortRequestRules,
  sortScoringRules,
  type RequestRuleSortKey,
  type ScoringRuleSortKey,
} from "./rule-table.ts";

type ScoringRule = Parameters<typeof sortScoringRules>[0][number];
type RequestRule = Parameters<typeof sortRequestRules>[0][number] & {
  libraries: string;
};

const ids = (rules: { id: string }[]) => rules.map((rule) => rule.id);

function scoring(id: string, overrides: Partial<ScoringRule> = {}) {
  return {
    id,
    name: id,
    description: null,
    appliedFacets: [],
    enabled: true,
    ...overrides,
  } satisfies ScoringRule;
}

function request(id: string, overrides: Partial<RequestRule> = {}) {
  return {
    id,
    name: id,
    description: null,
    enabled: true,
    decisionCount: 0,
    libraries: "All libraries",
    ...overrides,
  } satisfies RequestRule;
}

const scoringSort = (key: ScoringRuleSortKey, descending = false) => ({
  key,
  descending,
});
const requestSort = (key: RequestRuleSortKey, descending = false) => ({
  key,
  descending,
});

test("scoring rules sort by name naturally and keep global rules last by facet", () => {
  const rules = [
    scoring("rule 10"),
    scoring("rule 2", { appliedFacets: ["series"] }),
    scoring("rule 1", { appliedFacets: ["anime"] }),
  ];
  assert.deepEqual(ids(sortScoringRules(rules, scoringSort("name"))), [
    "rule 1",
    "rule 2",
    "rule 10",
  ]);
  for (const descending of [false, true]) {
    assert.equal(
      sortScoringRules(rules, scoringSort("appliedFacets", descending)).at(-1)
        ?.id,
      "rule 10",
    );
  }
});

test("enabled sorts disabled rules first, then by name", () => {
  const rules = [scoring("b"), scoring("a"), scoring("c", { enabled: false })];
  assert.deepEqual(ids(sortScoringRules(rules, scoringSort("enabled"))), [
    "c",
    "a",
    "b",
  ]);
});

test("request rules sort by the library text shown and by decision count", () => {
  const rules = [
    request("one", { libraries: "Movies", decisionCount: 4 }),
    request("two", { libraries: "Anime", decisionCount: 9 }),
    request("three", { libraries: "TV", decisionCount: 1 }),
  ];
  const label = (rule: RequestRule) => rule.libraries;
  assert.deepEqual(
    ids(sortRequestRules(rules, requestSort("libraries"), label)),
    ["two", "one", "three"],
  );
  assert.deepEqual(
    ids(sortRequestRules(rules, requestSort("decisions", true), label)),
    ["two", "one", "three"],
  );
});

import assert from "node:assert/strict";
import test from "node:test";

import {
  lifecycleRulePage,
  nextLifecycleRuleSort,
  sortLifecycleRules,
} from "./lifecycle-rule-table.ts";

type Rule = Parameters<typeof sortLifecycleRules>[0][number];

function rule(id: string, overrides: Partial<Rule> = {}): Rule {
  return {
    id,
    name: id,
    description: null,
    graceDays: 7,
    nextRunAt: null,
    state: "DISABLED",
    ...overrides,
  };
}

const ids = (rules: Rule[]) => rules.map((entry) => entry.id);

test("names sort naturally, and ties fall back to the name", () => {
  const rules = [rule("rule 10"), rule("Rule 2"), rule("rule 1")];
  assert.deepEqual(
    ids(sortLifecycleRules(rules, { key: "name", descending: false })),
    ["rule 1", "Rule 2", "rule 10"],
  );
  assert.deepEqual(
    ids(sortLifecycleRules(rules, { key: "name", descending: true })),
    ["rule 10", "Rule 2", "rule 1"],
  );
  const tied = [
    rule("b", { graceDays: 3 }),
    rule("a", { graceDays: 3 }),
    rule("c", { graceDays: 1 }),
  ];
  assert.deepEqual(
    ids(sortLifecycleRules(tied, { key: "graceDays", descending: false })),
    ["c", "a", "b"],
  );
});

test("a rule with no description or no planned run sorts last either way", () => {
  const rules = [
    rule("none"),
    rule("soon", { nextRunAt: "2026-11-18T01:00:00Z", description: "alpha" }),
    rule("later", { nextRunAt: "2026-11-19T01:00:00Z", description: "beta" }),
  ];
  for (const descending of [false, true]) {
    assert.equal(
      ids(sortLifecycleRules(rules, { key: "schedule", descending })).at(-1),
      "none",
    );
    assert.equal(
      ids(sortLifecycleRules(rules, { key: "description", descending })).at(-1),
      "none",
    );
  }
  assert.deepEqual(
    ids(sortLifecycleRules(rules, { key: "schedule", descending: false })),
    ["soon", "later", "none"],
  );
});

test("state sorts from disabled to enabled", () => {
  const rules = [
    rule("on", { state: "ENABLED" }),
    rule("off"),
    rule("dry", { state: "DRY_RUN" }),
  ];
  assert.deepEqual(
    ids(sortLifecycleRules(rules, { key: "state", descending: false })),
    ["off", "dry", "on"],
  );
});

test("pressing the active column reverses it; another column starts ascending", () => {
  const current = { key: "name", descending: false } as const;
  assert.deepEqual(nextLifecycleRuleSort(current, "name"), {
    key: "name",
    descending: true,
  });
  assert.deepEqual(
    nextLifecycleRuleSort({ key: "name", descending: true }, "state"),
    {
      key: "state",
      descending: false,
    },
  );
});

test("pages hold fifteen rows and a page past the end shows the last one", () => {
  const rows = Array.from({ length: 31 }, (_, index) => index);
  const first = lifecycleRulePage(rows, 0);
  assert.deepEqual(
    [first.rows.length, first.pageCount, first.start, first.end, first.total],
    [15, 3, 1, 15, 31],
  );
  const last = lifecycleRulePage(rows, 9);
  assert.deepEqual(
    [last.page, last.rows, last.start, last.end],
    [2, [30], 31, 31],
  );
  const empty = lifecycleRulePage([], 4);
  assert.deepEqual(
    [empty.page, empty.pageCount, empty.start, empty.end],
    [0, 1, 0, 0],
  );
});

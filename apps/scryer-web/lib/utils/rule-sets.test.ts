import assert from "node:assert/strict";
import test from "node:test";

import ruleInputContract from "../contracts/rule-input-contract.json" with { type: "json" };
import type { RuleSetRecord } from "../types/rule-sets.ts";
import {
  copyRuleSetDraft,
  createRuleSetInput,
  isUserOwnedRuleSet,
} from "./rule-sets.ts";

const managedRule: RuleSetRecord = {
  id: "managed-1",
  name: "Built-in quality guard",
  description: "Prefer reliable releases.",
  managedTagFilter: null,
  regoSource: "package scryer.rules.managed.quality_guard",
  enabled: false,
  priority: 42,
  appliedFacets: ["movies", "series"],
  isManaged: true,
  managedKey: "quality_guard",
  createdAt: "2026-01-01T00:00:00Z",
  updatedAt: "2026-01-01T00:00:00Z",
};

test("copying a managed rule creates a user-owned draft without managed metadata", () => {
  const draft = copyRuleSetDraft(managedRule);

  assert.deepEqual(draft, {
    name: "Copy of Built-in quality guard",
    description: managedRule.description,
    regoSource: managedRule.regoSource,
    enabled: false,
    priority: managedRule.priority,
    appliedFacets: ["movies", "series"],
  });
  assert.equal(Object.hasOwn(draft, "id"), false);
  assert.equal(Object.hasOwn(draft, "managedKey"), false);
  assert.notEqual(draft.appliedFacets, managedRule.appliedFacets);
});

test("create input round-trips enabled and omits managed metadata", () => {
  const input = createRuleSetInput(copyRuleSetDraft(managedRule));

  assert.equal(input.enabled, false);
  assert.deepEqual(input.appliedFacets, ["movies", "series"]);
  assert.equal(Object.hasOwn(input, "id"), false);
  assert.equal(Object.hasOwn(input, "isManaged"), false);
  assert.equal(Object.hasOwn(input, "managedKey"), false);
});

test("managed rules remain guarded from user-owned edit and delete actions", () => {
  assert.equal(isUserOwnedRuleSet(managedRule), false);
  assert.equal(isUserOwnedRuleSet({ ...managedRule, isManaged: false }), true);
});

test("rule input reference renders release guide facts as a string array", () => {
  const releaseSection = ruleInputContract.sections.find(
    (section) => section.path === "input.release",
  );
  const guideFacts = releaseSection?.fields.find(
    (field) => field.field === "guide_facts",
  );

  assert.deepEqual(guideFacts, {
    field: "guide_facts",
    type: "string[]",
    descKey: "settings.refReleaseGuideFacts",
  });
});

import test from "node:test";
import assert from "node:assert/strict";

import type { RequestShaping } from "../types/request-rule-sets.ts";
import {
  REQUEST_STARTER_SOURCE,
  requestApprovalPrefill,
  requestDeclaredActionEntries,
  requestRuleReadsRequester,
  requestShapingConflicts,
  requestShapingEntries,
} from "./request-rule-sets.ts";

test("a matcher reading requester fields raises the permission hint", () => {
  assert.equal(
    requestRuleReadsRequester(
      'deny if {\n  input.requester.username == "sample-member"\n}',
    ),
    true,
  );
  assert.equal(
    requestRuleReadsRequester(
      'approve if {\n  "auto_approve" in input["requester"].library_permissions\n}',
    ),
    true,
  );
});

test("requester mentioned only in a comment or another field does not", () => {
  assert.equal(
    requestRuleReadsRequester(
      '# input.requester is not read here\napprove if {\n  input.request.origin == "web"\n}',
    ),
    false,
  );
  assert.equal(
    requestRuleReadsRequester(
      'manual if {\n  input.request.origin == "api"\n}',
    ),
    false,
  );
});

// ── Declared actions, shaping and the approver's prefill ──────────────

const FORMAT = {
  leaseDays: (days: number) => (days === 0 ? "forever" : `${days}d`),
  monitor: (value: string) => `monitor:${value}`,
  languages: (codes: string[]) => codes.join("+"),
};

function shaping(overrides: Partial<RequestShaping> = {}): RequestShaping {
  return {
    qualityProfile: null,
    leaseDays: null,
    monitor: null,
    requiredAudioLanguages: null,
    preferredAudioLanguages: null,
    notifications: [],
    conflicts: [],
    ...overrides,
  };
}

test("declared request heads carry the values the API lists for them, each head once", () => {
  const entries = requestDeclaredActionEntries(
    {
      heads: ["APPROVE", "SET_LEASE_DAYS", "SET_MONITOR", "SET_REQUIRED_AUDIO_LANGUAGES", "ADD_TAGS", "APPROVE", "ARCHIVE"],
      qualityProfiles: [],
      leaseDays: [0, 30],
      monitors: ["FUTURE_EPISODES"],
      requiredAudioLanguages: ["eng", "jpn"],
      preferredAudioLanguages: [],
      addTags: ["sample-tag"],
      notifications: [],
    },
    FORMAT,
  );
  assert.deepEqual(entries, [
    { head: "APPROVE", values: [] },
    { head: "SET_LEASE_DAYS", values: ["forever", "30d"] },
    { head: "SET_MONITOR", values: ["monitor:FUTURE_EPISODES"] },
    { head: "SET_REQUIRED_AUDIO_LANGUAGES", values: ["eng+jpn"] },
    { head: "ADD_TAGS", values: ["sample-tag"] },
    // A head the API has no value field for yet is listed bare.
    { head: "ARCHIVE", values: [] },
  ]);
  assert.deepEqual(requestDeclaredActionEntries(null, FORMAT), []);
});

test("shaping lists each shaped value with the rule that set it, and conflicts once each", () => {
  const value = shaping({
    qualityProfile: { value: "Sample Profile", ruleSetName: "Sample rule" },
    leaseDays: { value: 0 },
    requiredAudioLanguages: { value: ["eng"], ruleSetName: "Sample rule" },
    notifications: [{ value: "Sample notice", ruleSetName: "Other rule" }],
    conflicts: ["SET_MONITOR", "SET_MONITOR", "SET_PREFERRED_AUDIO_LANGUAGES"],
  });
  assert.deepEqual(requestShapingEntries(value, FORMAT), [
    { head: "SET_QUALITY_PROFILE", value: "Sample Profile", ruleSetName: "Sample rule" },
    { head: "SET_LEASE_DAYS", value: "forever", ruleSetName: null },
    { head: "SET_REQUIRED_AUDIO_LANGUAGES", value: "eng", ruleSetName: "Sample rule" },
    { head: "NOTIFY", value: "Sample notice", ruleSetName: "Other rule" },
  ]);
  assert.deepEqual(requestShapingConflicts(value), [
    "SET_MONITOR",
    "SET_PREFERRED_AUDIO_LANGUAGES",
  ]);
  assert.deepEqual(requestShapingEntries(null, FORMAT), []);
  assert.deepEqual(requestShapingConflicts(undefined), []);
});

const PROFILES = [
  { id: "profile-1", name: "Sample Profile" },
  { id: "profile-2", name: "Other Profile" },
];

test("a request held for review prefills the approver's form and marks each suggestion's rule", () => {
  const prefill = requestApprovalPrefill(
    {
      effectiveOutcome: "MANUAL_REVIEW",
      shaping: shaping({
        qualityProfile: { value: "Sample Profile", ruleSetName: "Profile rule" },
        leaseDays: { value: 30, ruleSetName: "Lease rule" },
        monitor: { value: "FUTURE_EPISODES" },
        requiredAudioLanguages: { value: ["eng"], ruleSetName: "Language rule" },
        preferredAudioLanguages: { value: ["jpn", "eng"] },
      }),
    },
    "SERIES",
    PROFILES,
  );
  assert.deepEqual(prefill, {
    qualityProfileId: "profile-1",
    monitorType: "FUTURE_EPISODES",
    lease: { forever: false, days: 30 },
    requiredAudioLanguages: ["eng"],
    preferredAudioLanguages: ["jpn", "eng"],
    suggested: {
      SET_QUALITY_PROFILE: "Profile rule",
      SET_MONITOR: "",
      SET_LEASE_DAYS: "Lease rule",
      SET_REQUIRED_AUDIO_LANGUAGES: "Language rule",
      SET_PREFERRED_AUDIO_LANGUAGES: "",
    },
  });
});

test("a zero-day lease prefills forever", () => {
  const prefill = requestApprovalPrefill(
    { effectiveOutcome: "MANUAL_REVIEW", shaping: shaping({ leaseDays: { value: 0 } }) },
    "MOVIE",
    PROFILES,
  );
  assert.deepEqual(prefill.lease, { forever: true });
  assert.equal(prefill.suggested.SET_LEASE_DAYS, "");
});

test("the prefill leaves out what it cannot offer instead of guessing", () => {
  const prefill = requestApprovalPrefill(
    {
      effectiveOutcome: "MANUAL_REVIEW",
      shaping: shaping({
        qualityProfile: { value: "Missing Profile" },
        // A movie library offers no episode monitoring.
        monitor: { value: "FUTURE_EPISODES" },
        leaseDays: { value: 99999 },
      }),
    },
    "MOVIE",
    PROFILES,
  );
  assert.deepEqual(prefill, { suggested: {} });
});

test("only a request held for review is prefilled", () => {
  const value = shaping({ qualityProfile: { value: "Sample Profile" } });
  for (const effectiveOutcome of ["APPROVED", "DENIED"])
    assert.deepEqual(
      requestApprovalPrefill({ effectiveOutcome, shaping: value }, "SERIES", PROFILES),
      { suggested: {} },
    );
  assert.deepEqual(requestApprovalPrefill(null, "SERIES", PROFILES), { suggested: {} });
});

test("the starter source shapes with add_tags and never the retired tag check", () => {
  assert.match(REQUEST_STARTER_SOURCE, /\badd_tags\b/);
  assert.doesNotMatch(REQUEST_STARTER_SOURCE, /tags contains/);
  assert.equal(REQUEST_STARTER_SOURCE.includes("package "), false);
  assert.equal(REQUEST_STARTER_SOURCE.includes("import rego.v1"), false);
});

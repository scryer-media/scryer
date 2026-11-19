import assert from "node:assert/strict";
import test from "node:test";

import {
  normalizeExternalRuleImportStatus,
  ruleImportDiagnostics,
  ruleImportProgress,
  ruleImportReasonMessage,
  unmatchedProfileNames,
} from "./external-rule-import.ts";

test("a rule import report is read field by field", () => {
  const report = normalizeExternalRuleImportStatus({
    runId: "run-1",
    status: "WARNING",
    active: false,
    revision: 7,
    updatedAt: "2026-01-01T00:00:00Z",
    source: "MAINTAINERR",
    baseUrl: "http://127.0.0.1:6246",
    phase: "FINISHED",
    reason: null,
    message: null,
    counts: { total: 2, saved: 1, failed: 1 },
    items: [
      {
        key: "group:1",
        sourceName: "Synthetic Group",
        state: "SAVED_NEEDS_REVIEW",
        facts: [{ code: "RULE_IMPORT_USERNAMES_UNAVAILABLE", message: "x" }],
      },
      { key: "group:2", state: "SOMETHING_NEW" },
      { noKey: true },
    ],
    facts: [],
  });
  assert.ok(report);
  assert.equal(report.status, "WARNING");
  assert.equal(report.counts.saved, 1);
  assert.equal(report.counts.pending, 0);
  assert.equal(report.items.length, 2);
  assert.equal(report.items[0].state, "SAVED_NEEDS_REVIEW");
  assert.equal(report.items[0].facts[0].details, null);
  assert.equal(report.items[1].state, "FAILED");
  assert.equal(report.items[1].sourceName, "group:2");
  assert.equal(normalizeExternalRuleImportStatus({}), null);
});

test("report reasons use their translation and keep the server sentence for unknown codes", () => {
  const translate = (key: string) => `t:${key}`;
  assert.equal(
    ruleImportReasonMessage(
      "EXTERNAL_IMPORT_SOURCE_UNAUTHORIZED",
      "server words",
      "fallback",
      translate,
    ),
    "t:error.reason.EXTERNAL_IMPORT_SOURCE_UNAUTHORIZED",
  );
  assert.equal(
    ruleImportReasonMessage(
      "RULE_IMPORT_TRANSLATOR_UNAVAILABLE",
      null,
      "fallback",
      translate,
    ),
    "t:error.reason.RULE_IMPORT_TRANSLATOR_UNAVAILABLE",
  );
  assert.equal(
    ruleImportReasonMessage(
      "FUTURE_CODE",
      "server words",
      "fallback",
      translate,
    ),
    "server words",
  );
  assert.equal(
    ruleImportReasonMessage(null, null, "fallback", translate),
    "fallback",
  );
});

test("review items list the profile names Scryer could not match", () => {
  const item = {
    key: "customformat:1",
    sourceName: "Sample Format Alpha",
    savedName: "(Radarr) REVIEW - Sample Format Alpha",
    ruleSetId: "rule-1",
    state: "SAVED_NEEDS_REVIEW" as const,
    reason: "RULE_IMPORT_PROFILE_SCORES_UNMATCHED",
    message: null,
    facts: [
      {
        code: "RULE_IMPORT_PROFILE_SCORES_UNMATCHED",
        message: "server copy",
        details: {
          unmatchedProfiles: ["Sample Profile B", 7, "Sample Profile C"],
          scores: [],
        },
      },
    ],
  };
  assert.deepEqual(unmatchedProfileNames(item), [
    "Sample Profile B",
    "Sample Profile C",
  ]);
  assert.deepEqual(unmatchedProfileNames({ ...item, facts: [] }), []);
});

test("diagnostics are the facts that carry a location", () => {
  const item = {
    key: "customformat:2",
    sourceName: "Sample Format Beta",
    savedName: null,
    ruleSetId: null,
    state: "FAILED" as const,
    reason: "RULE_IMPORT_FORMAT_UNTRANSLATABLE",
    message: null,
    facts: [
      {
        code: "arr.year.unmapped",
        message: "Year cannot be mapped.",
        details: { severity: "ERROR", location: "$.specifications[0]" },
      },
      { code: "RULE_IMPORT_FORMAT_UNSCORED", message: "info", details: null },
    ],
  };
  assert.deepEqual(ruleImportDiagnostics(item), [
    {
      code: "arr.year.unmapped",
      message: "Year cannot be mapped.",
      location: "$.specifications[0]",
    },
  ]);
});

test("running progress reads its phase and counts defensively", () => {
  assert.equal(ruleImportProgress(null), null);
  const progress = ruleImportProgress({
    phase: "SAVING",
    counts: { total: 4, saved: 2, failed: -1 },
  });
  assert.equal(progress?.phase, "SAVING");
  assert.equal(progress?.counts.total, 4);
  assert.equal(progress?.counts.saved, 2);
  assert.equal(progress?.counts.failed, 0);
  assert.equal(ruleImportProgress({ phase: "LATER" })?.phase, "QUEUED");
});

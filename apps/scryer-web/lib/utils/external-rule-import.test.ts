import assert from "node:assert/strict";
import test from "node:test";

import {
  normalizeExternalRuleImportStatus,
  ruleImportReasonMessage,
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

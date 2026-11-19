import assert from "node:assert/strict";
import test from "node:test";

import en from "../i18n/locales/en.ts";
import ru from "../i18n/locales/ru.ts";
import {
  MAINTAINERR_DIAGNOSTIC_KEYS,
  maintainerrDiagnosticMessage,
  maintainerrDiagnostics,
} from "./maintainerr-rule-import.ts";

const item = {
  key: "rulegroup:3",
  sourceName: "Sample Group Charlie",
  savedName: "(Maintainerr) FAILED - Sample Group Charlie",
  ruleSetId: "rule-3",
  state: "SAVED_FAILED_TRANSLATION" as const,
  reason: "RULE_IMPORT_GROUP_UNTRANSLATABLE",
  message: null,
  facts: [
    {
      code: "maintainerr.fact.approximate",
      message: "server words",
      details: { severity: "WARNING", ruleIndex: 0 },
    },
    {
      code: "maintainerr.property.unmapped",
      message: "server words",
      details: { severity: "ERROR", ruleIndex: 1 },
    },
    {
      code: "maintainerr.action.unsupported",
      message: "server words",
      details: { severity: "ERROR", ruleIndex: null },
    },
    {
      code: "LIFECYCLE_PERSON_TARGETING_PERMISSION_REQUIRED",
      message: "restates the item reason",
      details: null,
    },
  ],
};

test("every diagnostic of an item is listed, errors first, with or without a rule", () => {
  assert.deepEqual(
    maintainerrDiagnostics(item).map((diagnostic) => [
      diagnostic.code,
      diagnostic.severity,
      diagnostic.ruleIndex,
    ]),
    [
      ["maintainerr.property.unmapped", "ERROR", 1],
      ["maintainerr.action.unsupported", "ERROR", null],
      ["maintainerr.fact.approximate", "WARNING", 0],
    ],
  );
});

test("diagnostics use their translation and keep the server sentence for unknown codes", () => {
  const translate = (key: string) => `t:${key}`;
  assert.equal(
    maintainerrDiagnosticMessage(
      { code: "maintainerr.property.unmapped", message: "server words" },
      translate,
    ),
    "t:settings.maintainerrImportCode.propertyUnmapped",
  );
  assert.equal(
    maintainerrDiagnosticMessage(
      { code: "maintainerr.future.code", message: "server words" },
      translate,
    ),
    "server words",
  );
});

test("every translator code has copy in English and Russian", () => {
  const entries = Object.entries(MAINTAINERR_DIAGNOSTIC_KEYS);
  assert.equal(entries.length, 25);
  for (const [code, key] of entries) {
    assert.ok(key in en, `no English copy for ${code}`);
    assert.ok(key in ru, `no Russian copy for ${code}`);
  }
});

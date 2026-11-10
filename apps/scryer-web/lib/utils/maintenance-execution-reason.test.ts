import assert from "node:assert/strict";
import test from "node:test";

import en from "../i18n/locales/en.ts";
import {
  MAINTENANCE_EXECUTION_REASON_KEYS,
  maintenanceExecutionReasonText,
} from "./maintenance-execution-reason.ts";

const dictionary = en as Record<string, string>;
const t = (key: string) => dictionary[key] ?? key;

test("every translated execution reason has an English string", () => {
  for (const [code, key] of Object.entries(MAINTENANCE_EXECUTION_REASON_KEYS)) {
    assert.equal(typeof dictionary[key], "string", `${code} → ${key}`);
  }
});

test("a watch re-check hold is said in translation", () => {
  assert.equal(
    maintenanceExecutionReasonText(t, "watch_recheck_newer_play"),
    dictionary["settings.maintenanceExecutionReason.watchRecheckNewerPlay"],
  );
});

test("a code the client does not translate is shown as written", () => {
  assert.equal(
    maintenanceExecutionReasonText(t, "active_acquisition"),
    "active_acquisition",
  );
});

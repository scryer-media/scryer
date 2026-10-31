import assert from "node:assert/strict";
import test from "node:test";

import en from "../i18n/locales/en.ts";
import {
  RESTORE_REFUSAL_KEYS,
  restoreErrorMessage,
  restoreJobError,
  restoreRefusalKey,
} from "./restore-error-message.ts";

const dictionary = en as Record<string, string>;
const t = (key: string) => dictionary[key] ?? key;

function refusal(message: string, extensions: Record<string, unknown>) {
  return { graphQLErrors: [{ message, extensions }] };
}

test("every restore reason has English copy", () => {
  for (const key of Object.values(RESTORE_REFUSAL_KEYS)) {
    assert.ok(dictionary[key], `${key} is missing from en.ts`);
  }
});

test("a refused restore request is explained from its reason, not its sentence", () => {
  const error = refusal("validation: some other wording", {
    code: "VALIDATION_ERROR",
    reason: "RESTORE_MAINTENANCE_ACTIVE",
  });

  assert.equal(
    restoreRefusalKey(error),
    "setup.restoreRefusal.maintenanceActive",
  );
  assert.equal(
    restoreErrorMessage(error, t, "Failed"),
    dictionary["setup.restoreRefusal.maintenanceActive"],
  );
});

test("a failed attempt's reason is translated", () => {
  const error = restoreJobError(
    { reason: "RESTORE_DEADLINE_EXCEEDED", message: null },
    "Failed",
  );

  assert.equal(
    restoreErrorMessage(error, t, "Failed"),
    dictionary["setup.restoreRefusal.deadlineExceeded"],
  );
});

test("a failed attempt with an unknown reason shows the server's sentence", () => {
  const error = restoreJobError(
    {
      reason: "SECURITY_BACKUP_SYNTHETIC_REFUSAL",
      message: "validation: synthetic sentence",
    },
    "Failed",
  );

  assert.equal(restoreRefusalKey(error), null);
  assert.equal(restoreErrorMessage(error, t, "Failed"), "synthetic sentence");
});

test("a failed attempt with neither sentence nor known reason shows the fallback", () => {
  const error = restoreJobError({ reason: null, message: null }, "Failed");

  assert.equal(restoreErrorMessage(error, t, "Failed"), "Failed");
});

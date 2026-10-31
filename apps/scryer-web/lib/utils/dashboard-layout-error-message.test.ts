import assert from "node:assert/strict";
import test from "node:test";

import en from "../i18n/locales/en.ts";
import {
  DASHBOARD_LAYOUT_REFUSAL_KEYS,
  dashboardLayoutErrorMessage,
  dashboardLayoutRefusalKey,
} from "./dashboard-layout-error-message.ts";

const dictionary = en as Record<string, string>;
const t = (key: string) => dictionary[key] ?? key;

function refusal(message: string, extensions: Record<string, unknown>) {
  return { graphQLErrors: [{ message, extensions }] };
}

test("a named layout refusal is shown in translation, whatever the sentence says", () => {
  const error = refusal("validation: panel stats appears twice", {
    code: "VALIDATION_ERROR",
    reason: "SETTINGS_DASHBOARD_PANEL_ID_DUPLICATED",
  });

  assert.equal(
    dashboardLayoutRefusalKey(error),
    "dashboard.layout.refusal.panelIdDuplicate",
  );
  assert.equal(
    dashboardLayoutErrorMessage(error, t, "Save failed"),
    dictionary["dashboard.layout.refusal.panelIdDuplicate"],
  );
});

test("an unknown reason falls back to the server's sentence", () => {
  const error = refusal("validation: the layout could not be stored", {
    code: "VALIDATION_ERROR",
    reason: "SETTINGS_DASHBOARD_SOMETHING_NEW",
  });

  assert.equal(dashboardLayoutRefusalKey(error), null);
  assert.equal(
    dashboardLayoutErrorMessage(error, t, "Save failed"),
    "the layout could not be stored",
  );
});

test("a sentence without a reason is never matched on its text", () => {
  const error = refusal("validation: panel stats appears twice", {
    code: "VALIDATION_ERROR",
  });

  assert.equal(dashboardLayoutRefusalKey(error), null);
  assert.equal(
    dashboardLayoutErrorMessage(error, t, "Save failed"),
    "panel stats appears twice",
  );
});

test("a reason on another kind of error is not read as a layout refusal", () => {
  const error = refusal("not allowed", {
    code: "UNAUTHORIZED",
    reason: "SETTINGS_DASHBOARD_PANEL_ID_DUPLICATED",
  });

  assert.equal(dashboardLayoutRefusalKey(error), null);
});

test("every mapped reason has an English sentence", () => {
  for (const [reason, key] of Object.entries(DASHBOARD_LAYOUT_REFUSAL_KEYS)) {
    assert.match(reason, /^SETTINGS_DASHBOARD_[A-Z_]+$/);
    assert.equal(typeof dictionary[key], "string", key);
    assert.notEqual(dictionary[key], "", key);
  }
});

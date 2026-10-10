import assert from "node:assert/strict";
import test from "node:test";

import en from "../i18n/locales/en.ts";
import { LIST_REFUSAL_KEYS, listErrorMessage, listRefusalKey } from "./list-error-message.ts";

const dictionary = en as Record<string, string>;
const t = (key: string) => dictionary[key] ?? key;

function refusal(message: string, extensions: Record<string, unknown>) {
  return { graphQLErrors: [{ message, extensions }] };
}

test("a named lists refusal is shown in translation, not as the server's sentence", () => {
  const error = refusal("validation: this list is already followed", {
    code: "VALIDATION_ERROR",
    reason: "LIST_ALREADY_FOLLOWED",
  });

  assert.equal(listRefusalKey(error), "lists.refusal.alreadyFollowed");
  assert.equal(listErrorMessage(error, t, "Update failed"), "This list is already followed.");
});

test("the translation follows the reason, whatever the server's sentence says", () => {
  const error = refusal("validation: Lantern Harbor picks is on the shelf twice", {
    code: "VALIDATION_ERROR",
    reason: "LIST_ALREADY_FOLLOWED",
  });

  assert.equal(listErrorMessage(error, t, "Update failed"), "This list is already followed.");
});

test("a reason this client does not know falls back to the server's sentence", () => {
  const error = refusal("validation: Sort order is required", {
    code: "VALIDATION_ERROR",
    reason: "LIST_PARAM_REQUIRED",
  });

  assert.equal(listRefusalKey(error), null);
  assert.equal(listErrorMessage(error, t, "Update failed"), "Sort order is required");
});

test("a validation failure without a reason shows the server's sentence as before", () => {
  const error = refusal("validation: the provider answered with nothing to follow", {
    code: "VALIDATION_ERROR",
  });

  assert.equal(listRefusalKey(error), null);
  assert.equal(
    listErrorMessage(error, t, "Update failed"),
    "the provider answered with nothing to follow",
  );
});

test("a reason on another kind of error is not read as a lists refusal", () => {
  const error = refusal("validation: that address is not allowed", {
    code: "PUBLIC_URL_REJECTED",
    reason: "LIST_ALREADY_FOLLOWED",
  });

  assert.equal(listRefusalKey(error), null);
  assert.equal(listErrorMessage(error, t, "Update failed"), "that address is not allowed");
});

test("a failure with no sentence at all shows the caller's fallback", () => {
  assert.equal(listErrorMessage(null, t, "Update failed"), "Update failed");
});

test("every known reason is a lists reason with an English sentence", () => {
  for (const [reason, key] of Object.entries(LIST_REFUSAL_KEYS)) {
    assert.match(reason, /^LIST_[A-Z0-9]+(?:_[A-Z0-9]+)*$/, reason);
    assert.ok(dictionary[key]?.trim(), `${reason} names the missing key ${key}`);
  }
});

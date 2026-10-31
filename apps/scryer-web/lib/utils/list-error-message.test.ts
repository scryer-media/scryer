import assert from "node:assert/strict";
import test from "node:test";

import en from "../i18n/locales/en.ts";
import {
  LIST_REFUSAL_KEYS,
  listErrorMessage,
  listRefusalKey,
} from "./list-error-message.ts";

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
  assert.equal(
    listErrorMessage(error, t, "Update failed"),
    "This list is already followed.",
  );
});

test("the translation follows the reason, whatever the server's sentence says", () => {
  const error = refusal(
    "validation: Lantern Harbor picks is on the shelf twice",
    {
      code: "VALIDATION_ERROR",
      reason: "LIST_ALREADY_FOLLOWED",
    },
  );

  assert.equal(
    listErrorMessage(error, t, "Update failed"),
    "This list is already followed.",
  );
});

test("a reason this client does not know falls back to the server's sentence", () => {
  const error = refusal("validation: Sort order is required", {
    code: "VALIDATION_ERROR",
    reason: "LIST_PARAM_REQUIRED",
  });

  assert.equal(listRefusalKey(error), null);
  assert.equal(
    listErrorMessage(error, t, "Update failed"),
    "Sort order is required",
  );
});

test("a validation failure without a reason shows the server's sentence as before", () => {
  const error = refusal(
    "validation: the provider answered with nothing to follow",
    {
      code: "VALIDATION_ERROR",
    },
  );

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
  assert.equal(
    listErrorMessage(error, t, "Update failed"),
    "that address is not allowed",
  );
});

test("a failed account sign-in is explained from its reason, not its sentence", () => {
  const error = refusal(
    "validation: list account authentication failed: provider_not_configured",
    {
      code: "VALIDATION_ERROR",
      reason: "LIST_ACCOUNT_AUTH_PROVIDER_NOT_CONFIGURED",
    },
  );

  assert.equal(
    listRefusalKey(error),
    "lists.refusal.accountAuthProviderNotConfigured",
  );
  assert.equal(
    listErrorMessage(error, t, "Account linking failed"),
    dictionary["lists.refusal.accountAuthProviderNotConfigured"],
  );
});

test("a rate-limited account sign-in is explained although it is not a validation error", () => {
  const error = refusal("list account authentication failed: rate_limited", {
    code: "TEMPORARY_UNAVAILABLE",
    reason: "LIST_ACCOUNT_AUTH_RATE_LIMITED",
    retryAfterSeconds: 30,
  });

  assert.equal(listRefusalKey(error), "lists.refusal.accountAuthRateLimited");
});

test("a sign-in sentence without a reason is never matched on its text", () => {
  const error = refusal(
    "validation: list account authentication failed: provider_not_configured",
    {
      code: "VALIDATION_ERROR",
    },
  );

  assert.equal(listRefusalKey(error), null);
  assert.equal(
    listErrorMessage(error, t, "Account linking failed"),
    "list account authentication failed: provider_not_configured",
  );
});

test("every account sign-in failure the server can name has a sentence", () => {
  const codes = [
    "access_denied",
    "authorization_failed",
    "busy",
    "instance_auth_required",
    "instance_auth_unavailable",
    "invalid_provider_app",
    "invalid_provider_response",
    "invalid_request",
    "provider_not_configured",
    "provider_rejected",
    "provider_unavailable",
    "rate_limited",
    "reconnect_required",
    "relay_unavailable",
    "transport_unavailable",
    "unsupported_exchange",
    "unsupported_poll",
    "unsupported_provider",
    "unsupported_provider_app",
  ];
  for (const code of codes) {
    const reason = `LIST_ACCOUNT_AUTH_${code.toUpperCase()}`;
    assert.ok(Object.hasOwn(LIST_REFUSAL_KEYS, reason), reason);
  }
  const named = Object.keys(LIST_REFUSAL_KEYS).filter(
    (reason) =>
      reason.startsWith("LIST_ACCOUNT_AUTH_") &&
      reason !== "LIST_ACCOUNT_AUTH_UNAVAILABLE",
  );
  assert.equal(named.length, codes.length);
});

test("a failure with no sentence at all shows the caller's fallback", () => {
  assert.equal(listErrorMessage(null, t, "Update failed"), "Update failed");
});

test("every known reason is a lists reason with an English sentence", () => {
  for (const [reason, key] of Object.entries(LIST_REFUSAL_KEYS)) {
    assert.match(reason, /^LIST_[A-Z0-9]+(?:_[A-Z0-9]+)*$/, reason);
    assert.ok(
      dictionary[key]?.trim(),
      `${reason} names the missing key ${key}`,
    );
  }
});

test("a key the provider refuses is explained as the key's fault", () => {
  const error = refusal("validation: the provider refused this key", {
    code: "VALIDATION_ERROR",
    reason: "LIST_ACCOUNT_KEY_REJECTED",
  });

  assert.equal(listRefusalKey(error), "lists.refusal.accountKeyRejected");
  assert.equal(
    listErrorMessage(error, t, "Account linking failed"),
    dictionary["lists.refusal.accountKeyRejected"],
  );
});

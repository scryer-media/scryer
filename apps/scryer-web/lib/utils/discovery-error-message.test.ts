import assert from "node:assert/strict";
import test from "node:test";

import en from "../i18n/locales/en.ts";
import {
  DISCOVERY_REFUSAL_KEYS,
  discoveryErrorMessage,
  discoveryRefusalKey,
} from "./discovery-error-message.ts";

const dictionary = en as Record<string, string>;
const t = (key: string) => dictionary[key] ?? key;

function refusal(message: string, extensions: Record<string, unknown>) {
  return { graphQLErrors: [{ message, extensions }] };
}

test("a neighbour anchor refusal is shown in translation, not as the server's sentence", () => {
  const error = refusal('"plex:movie:1" is not a discovery target key', {
    code: "VALIDATION_ERROR",
    reason: "DISCOVERY_NEIGHBOR_TARGET_KEY_UNSUPPORTED",
  });

  assert.equal(
    discoveryRefusalKey(error),
    "discovery.refusal.neighborTargetKeyUnsupported",
  );
  assert.equal(
    discoveryErrorMessage(error, t, "Request failed"),
    "This title can't be explored further.",
  );
});

test("a facet mismatch has its own translation", () => {
  const error = refusal('"tmdb:series:1" does not name a movie title', {
    code: "VALIDATION_ERROR",
    reason: "DISCOVERY_NEIGHBOR_TARGET_KEY_FACET_MISMATCH",
  });

  assert.equal(
    discoveryErrorMessage(error, t, "Request failed"),
    dictionary["discovery.refusal.neighborTargetKeyFacetMismatch"],
  );
});

test("a reason under another code, or no reason, is not translated as a refusal", () => {
  assert.equal(
    discoveryRefusalKey(
      refusal("busy", {
        code: "TEMPORARY_UNAVAILABLE",
        reason: "DISCOVERY_NEIGHBOR_TARGET_KEY_UNSUPPORTED",
      }),
    ),
    null,
  );
  assert.equal(discoveryRefusalKey(new Error("offline")), null);
});

test("every discovery refusal key has an English string", () => {
  for (const key of Object.values(DISCOVERY_REFUSAL_KEYS)) {
    assert.ok(dictionary[key], key);
  }
});

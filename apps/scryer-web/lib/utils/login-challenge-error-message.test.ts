import assert from "node:assert/strict";
import test from "node:test";

import en from "../i18n/locales/en.ts";
import {
  LOGIN_CHALLENGE_REFUSAL_KEYS,
  loginChallengeErrorMessage,
  loginChallengeRefusalKey,
} from "./login-challenge-error-message.ts";

const dictionary = en as Record<string, string>;
const t = (key: string) => dictionary[key] ?? key;

function refusal(message: string, extensions: Record<string, unknown>) {
  return { graphQLErrors: [{ message, extensions }] };
}

test("a named sign-in image refusal is shown in translation", () => {
  const error = refusal("The indexer's sign-in challenge is a 'image/svg+xml' file", {
    code: "VALIDATION_ERROR",
    reason: "INDEXER_LOGIN_CHALLENGE_IMAGE_TYPE_UNSUPPORTED",
  });

  assert.equal(
    loginChallengeRefusalKey(error),
    "settings.indexerLoginChallengeRefusal.imageTypeUnsupported",
  );
  assert.equal(
    loginChallengeErrorMessage(error, t, "Load failed"),
    dictionary["settings.indexerLoginChallengeRefusal.imageTypeUnsupported"],
  );
});

test("a failure without a known reason shows the server's sentence", () => {
  const error = refusal("Sample Tracker refused the connection", {
    code: "VALIDATION_ERROR",
    reason: "PLUGIN_AUTH_FAILED",
  });

  assert.equal(loginChallengeRefusalKey(error), null);
  assert.equal(
    loginChallengeErrorMessage(error, t, "Load failed"),
    "Sample Tracker refused the connection",
  );
});

test("every sign-in image reason has an English translation", () => {
  for (const key of Object.values(LOGIN_CHALLENGE_REFUSAL_KEYS)) {
    assert.ok(dictionary[key], `missing translation for ${key}`);
  }
});

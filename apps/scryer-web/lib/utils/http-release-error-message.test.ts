import assert from "node:assert/strict";
import test from "node:test";
import en from "../i18n/locales/en.ts";
import {
  grabErrorMessage,
  HTTP_RELEASE_REFUSAL_KEYS,
} from "./http-release-error-message.ts";

const t = (key: string) => en[key] ?? key;

const refusal = (reason: string, code = "VALIDATION_ERROR") => ({
  graphQLErrors: [
    { message: "changed server wording", extensions: { code, reason } },
  ],
});

test("each HTTP release refusal is translated whatever the server says", () => {
  for (const [reason, key] of Object.entries(HTTP_RELEASE_REFUSAL_KEYS)) {
    assert.ok(en[key], `${key} has English text`);
    assert.equal(grabErrorMessage(refusal(reason), t, "failed"), en[key]);
  }
});

test("an HTTP release reason this client does not know shows the server sentence", () => {
  assert.equal(
    grabErrorMessage(refusal("HTTP_RELEASE_FUTURE_CONDITION"), t, "failed"),
    "changed server wording",
  );
});

test("a reason on a non-validation error is not read as an HTTP release refusal", () => {
  assert.equal(
    grabErrorMessage(
      refusal("HTTP_RELEASE_LIVE", "REPOSITORY_ERROR"),
      t,
      "failed",
    ),
    "changed server wording",
  );
});

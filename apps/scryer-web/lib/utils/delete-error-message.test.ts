import assert from "node:assert/strict";
import test from "node:test";
import en from "../i18n/locales/en.ts";
import { deleteErrorMessage } from "./delete-error-message.ts";

const t = (key: string) => en[key] ?? key;

test("file deletion translates each shared refusal independently of server wording", () => {
  for (const [reason, expected] of [
    ["DELETE_PREVIEW_STALE", "titleDelete.refusal.preview"],
    ["DELETE_TYPED_CONFIRMATION_REQUIRED", "titleDelete.refusal.confirmation"],
    ["DELETE_ROOT_UNAVAILABLE", "titleDelete.refusal.unavailable"],
    ["DELETE_PATH_OUTSIDE_ROOTS", "titleDelete.refusal.outsideRoots"],
  ]) {
    const error = { graphQLErrors: [{ message: "changed server wording", extensions: {
      code: "VALIDATION_ERROR", reason,
    } }] };
    assert.equal(deleteErrorMessage(error, t, "failed"), en[expected]);
  }
});

test("unknown file deletion reasons preserve the server explanation", () => {
  const error = { graphQLErrors: [{ message: "validation: new condition", extensions: {
    code: "VALIDATION_ERROR", reason: "DELETE_FUTURE_CONDITION",
  } }] };
  assert.equal(deleteErrorMessage(error, t, "failed"), "new condition");
});

test("file deletion does not treat non-validation failures as validation refusals", () => {
  const error = { graphQLErrors: [{ message: "repository unavailable", extensions: {
    code: "REPOSITORY_ERROR", reason: "DELETE_PREVIEW_STALE",
  } }] };
  assert.equal(deleteErrorMessage(error, t, "failed"), "repository unavailable");
});

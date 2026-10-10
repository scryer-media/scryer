import test from "node:test";
import assert from "node:assert/strict";

import { resolveStatusToastLevel } from "./status-toast.ts";

// Failures as the server words them. None says "failed" or "error", which is
// why the level has to come from the code path that caught them.
const UNKEYWORDED_FAILURES = [
  "nzb download payload was not valid xml: unexpected end of file",
  "category_mismatch: indexer category 'TV > HD' contradicts the movie subject",
  "this list is already followed",
];

test("a status toasts at the level its caller stated", () => {
  for (const message of UNKEYWORDED_FAILURES) {
    assert.equal(resolveStatusToastLevel(message, { level: "ERROR" }), "ERROR", message);
  }
  assert.equal(resolveStatusToastLevel("Queued Lantern Harbor", { level: "SUCCESS" }), "SUCCESS");
  assert.equal(resolveStatusToastLevel("3 imported, 1 skipped", { level: "WARNING" }), "WARNING");
});

test("the wording never changes the stated level", () => {
  // Each sentence reads like a different level than the one its caller gave.
  assert.equal(resolveStatusToastLevel("Download marked failed.", { level: "SUCCESS" }), "SUCCESS");
  assert.equal(resolveStatusToastLevel("The tag could not be saved.", { level: "ERROR" }), "ERROR");
  assert.equal(resolveStatusToastLevel("Queued Lantern Harbor", { level: "ERROR" }), "ERROR");
  assert.equal(resolveStatusToastLevel("Title is required.", { level: "WARNING" }), "WARNING");
});

test("a status raised without a level does not toast, whatever it says", () => {
  for (const message of [
    ...UNKEYWORDED_FAILURES,
    "Request failed",
    "[GraphQL] validation: no download client enabled for library movie_default_library",
    "Queued Lantern Harbor",
    "Settings saved.",
    "Rename apply complete: 12 applied, 1 skipped, 2 failed.",
    "Searching…",
  ]) {
    assert.equal(resolveStatusToastLevel(message), null, message);
    assert.equal(resolveStatusToastLevel(message, {}), null, message);
  }
});

test("an empty status never toasts, whatever level was asked for", () => {
  assert.equal(resolveStatusToastLevel("", { level: "SUCCESS" }), null);
  assert.equal(resolveStatusToastLevel("   ", { level: "ERROR" }), null);
});

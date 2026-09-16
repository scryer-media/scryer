import test from "node:test";
import assert from "node:assert/strict";

import { classifyStatusToastLevel, resolveStatusToastLevel } from "./status-toast.ts";

test("raw GraphQL validation queue failure classifies as error", () => {
  assert.equal(
    classifyStatusToastLevel(
      "[GraphQL] validation: no download client enabled for library movie_default_library",
    ),
    "ERROR",
  );
});

test("normalized validation queue failure classifies as error", () => {
  assert.equal(
    classifyStatusToastLevel("no download client enabled for library movie_default_library"),
    "ERROR",
  );
});

test("suppressed validation prompts still do not toast", () => {
  assert.equal(classifyStatusToastLevel("validation: title is required"), null);
  assert.equal(classifyStatusToastLevel("title is required"), null);
});

test("rename apply completion with only applied items classifies as success", () => {
  assert.equal(
    classifyStatusToastLevel("Rename apply complete: 12 applied, 0 skipped, 0 failed."),
    "SUCCESS",
  );
});

test("rename apply completion with skipped items classifies as warning", () => {
  assert.equal(
    classifyStatusToastLevel("Rename apply complete: 12 applied, 1 skipped, 0 failed."),
    "WARNING",
  );
});

test("rename apply completion with failed items classifies as error", () => {
  assert.equal(
    classifyStatusToastLevel("Rename apply complete: 12 applied, 1 skipped, 2 failed."),
    "ERROR",
  );
});

// Normalized artifact grab failures, as the queue catch blocks hand them over.
// None carries a keyword the text classifier recognises, which is why those
// catch blocks pass an explicit level instead of relying on the wording.
const ARTIFACT_GRAB_FAILURES = [
  "nzb download payload was not valid xml: unexpected end of file",
  "category_mismatch: indexer category 'TV > HD' contradicts the movie subject",
  "The indexer no longer serves this download (HTTP 410 Gone): the search result has expired, search again for a fresh link.",
];

test("artifact grab failures carry no keyword the classifier recognises", () => {
  for (const message of ARTIFACT_GRAB_FAILURES) {
    assert.equal(classifyStatusToastLevel(message), null, message);
  }
});

test("an explicit level wins over the wording", () => {
  for (const message of ARTIFACT_GRAB_FAILURES) {
    assert.equal(resolveStatusToastLevel(message, { level: "ERROR" }), "ERROR", message);
  }
  // Even over wording that would classify differently or suppress the toast.
  assert.equal(resolveStatusToastLevel("title is required", { level: "ERROR" }), "ERROR");
  assert.equal(resolveStatusToastLevel("Queued Paperman", { level: "ERROR" }), "ERROR");
});

test("without an explicit level the wording still decides", () => {
  assert.equal(resolveStatusToastLevel("Queued Paperman"), "SUCCESS");
  assert.equal(resolveStatusToastLevel("title is required"), null);
  assert.equal(resolveStatusToastLevel("request failed", {}), "ERROR");
});

test("an empty status never toasts, whatever level was asked for", () => {
  assert.equal(resolveStatusToastLevel("   ", { level: "ERROR" }), null);
});


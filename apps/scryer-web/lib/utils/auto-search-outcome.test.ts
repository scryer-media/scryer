import test from "node:test";
import assert from "node:assert/strict";

import {
  autoSearchOutcomeMessage,
  autoSearchRejectionFromError,
  reportAutomaticSearchFailure,
} from "./auto-search-outcome.ts";

const t = (key: string, values?: Record<string, unknown>) => {
  switch (key) {
    case "status.autoSearchNoCandidates":
      return `No release found for ${values?.name}: no indexer returned a candidate.`;
    case "status.autoSearchAllRejected":
      return `No release found for ${values?.name}: automatic search rejected all ${values?.count} candidates. ${values?.reasons}`;
    default:
      return key;
  }
};

function rejectionError(extensions: Record<string, unknown>) {
  return {
    graphQLErrors: [
      {
        message: "validation: no auto-eligible release found",
        extensions: { code: "VALIDATION_ERROR", ...extensions },
      },
    ],
  };
}

test("returns null for errors that are not automatic-search rejections", () => {
  assert.equal(autoSearchRejectionFromError(new Error("network down")), null);
  assert.equal(
    autoSearchRejectionFromError({
      graphQLErrors: [{ message: "validation: bad", extensions: { code: "VALIDATION_ERROR" } }],
    }),
    null,
  );
  assert.equal(autoSearchOutcomeMessage(new Error("network down"), t, "Silver Horizon"), null);
});

test("names the blocking rule when every candidate was quality-blocked", () => {
  const error = rejectionError({
    autoCandidateCount: 4,
    autoDecisionReasons: [
      {
        code: "quality_blocked",
        summary: "quality profile blocked this release",
        count: 3,
        blockCodes: ["managed_required_audio_missing"],
      },
      {
        code: "title_mismatch",
        summary: "release title does not match the target title",
        count: 1,
        blockCodes: [],
      },
    ],
  });

  const message = autoSearchOutcomeMessage(error, t, "Silver Horizon");
  assert.equal(
    message,
    "No release found for Silver Horizon: automatic search rejected all 4 candidates. " +
      "quality profile blocked this release: managed_required_audio_missing (3); " +
      "release title does not match the target title (1)",
  );
});

test("reports an empty candidate set separately from a rejected one", () => {
  const error = rejectionError({ autoCandidateCount: 0, autoDecisionReasons: [] });
  const message = autoSearchOutcomeMessage(error, t, "Silver Horizon");
  assert.equal(
    message,
    "No release found for Silver Horizon: no indexer returned a candidate.",
  );
});

test("caps the reason list and tolerates malformed extension entries", () => {
  const reasons = ["a", "b", "c", "d"].map((code, index) => ({
    code,
    summary: `reason ${code}`,
    count: 4 - index,
    blockCodes: [],
  }));
  const error = rejectionError({
    autoCandidateCount: 10,
    autoDecisionReasons: [...reasons, "garbage", { count: 1 }],
  });

  const rejection = autoSearchRejectionFromError(error);
  assert.equal(rejection?.candidateCount, 10);
  assert.equal(rejection?.reasons.length, 4);
  const message = autoSearchOutcomeMessage(error, t, "X");
  assert.ok(message?.includes("reason c (2)"));
  assert.ok(!message?.includes("reason d"));
});

test("a search that found nothing to grab is reported as a warning, not a failure", () => {
  const reported: Array<[string, string | undefined]> = [];
  reportAutomaticSearchFailure(
    (status, options) => reported.push([status, options?.level]),
    t,
    rejectionError({ autoCandidateCount: 0, autoDecisionReasons: [] }),
    "Silver Horizon",
  );
  assert.deepEqual(reported, [
    ["No release found for Silver Horizon: no indexer returned a candidate.", "WARNING"],
  ]);
});

test("a search that could not run is reported as an error whatever the server said", () => {
  const reported: Array<[string, string | undefined]> = [];
  reportAutomaticSearchFailure(
    (status, options) => reported.push([status, options?.level]),
    t,
    new Error("category_mismatch: indexer category 'TV > HD' contradicts the movie subject"),
    "Silver Horizon",
  );
  assert.deepEqual(reported, [
    ["category_mismatch: indexer category 'TV > HD' contradicts the movie subject", "ERROR"],
  ]);
});

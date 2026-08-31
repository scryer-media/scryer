import assert from "node:assert/strict";
import test from "node:test";

import { parseDecisionExplanation } from "./release-decision-explanation.ts";

test("extracts scoring entries from the full release-decision explanation", () => {
  assert.deepEqual(
    parseDecisionExplanation({
      candidate: { source: "synthetic-indexer" },
      quality_profile_decision: {
        scoring_log: [
          { code: "quality_tier", delta: 1000 },
          { code: "preferred_protocol", delta: 50 },
        ],
      },
    }),
    [
      { code: "quality_tier", delta: 1000 },
      { code: "preferred_protocol", delta: 50 },
    ],
  );
});

test("preserves direct scoring-array explanations", () => {
  assert.deepEqual(
    parseDecisionExplanation([
      { code: "release_group", delta: -25 },
      { code: "revision", delta: 75 },
    ]),
    [
      { code: "release_group", delta: -25 },
      { code: "revision", delta: 75 },
    ],
  );
});

test("drops malformed explanation entries without hiding valid entries", () => {
  assert.deepEqual(
    parseDecisionExplanation({
      quality_profile_decision: {
        scoring_log: [
          null,
          { code: "", delta: 1 },
          { code: "not-finite", delta: Number.POSITIVE_INFINITY },
          { code: "missing-delta" },
          { code: "valid", delta: 10 },
        ],
      },
    }),
    [{ code: "valid", delta: 10 }],
  );
  assert.deepEqual(parseDecisionExplanation(null), []);
  assert.deepEqual(parseDecisionExplanation({ quality_profile_decision: {} }), []);
});

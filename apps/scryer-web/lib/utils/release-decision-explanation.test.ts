import assert from "node:assert/strict";
import test from "node:test";

import { parseDecisionExplanation, scoringEntryText } from "./release-decision-explanation.ts";

test("preserves recoverable penalties and explicit zero-point rejections", () => {
  const entries = parseDecisionExplanation([
    { code: "pack", delta: -10000, kind: "score_contribution" },
    { code: "boost", delta: 20000, kind: "score_contribution" },
    { code: "codec", delta: 0, kind: "mandatory_rejection" },
    { code: "minimum", delta: 0, kind: "final_score_rejection" },
    { code: "historical", delta: -10000 },
  ]);
  assert.equal(entries[0].kind, "score_contribution");
  assert.equal(entries[4].kind, undefined);
  const t = (key: string) => key;
  assert.deepEqual(entries.map((entry) => scoringEntryText(entry, t)), [
    "-10000", "+20000", "scoring.mandatoryRejection", "scoring.finalScoreRejection", "-10000",
  ]);
  assert.equal(entries.slice(0, 4).reduce((score, entry) => score + entry.delta, 0), 10000);
});

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

import assert from "node:assert/strict";
import test from "node:test";

import {
  formatScore,
  parseScoreLog,
  scoreEntryTone,
  toScoreLogEntries,
} from "./score-log.ts";

test("reads every rule source shape the backend records", () => {
  assert.deepEqual(
    toScoreLogEntries([
      {
        code: "search_user",
        delta: 30,
        kind: "score_contribution",
        source: "user:abc",
        ruleSetName: "Synthetic pack",
      },
      {
        code: "search_system",
        delta: -5,
        source: "system:def",
        ruleSetName: "Synthetic system",
      },
      {
        code: "search_builtin",
        delta: 100,
        source: "builtin",
        ruleSetName: null,
      },
      {
        code: "saved_named",
        delta: 20,
        kind: "score_contribution",
        source: "Synthetic pack",
      },
      {
        code: "saved_builtin",
        delta: 10,
        kind: "score_contribution",
        source: null,
      },
      {
        code: "saved_system",
        delta: 15,
        kind: "score_contribution",
        source: "Synthetic system",
        origin: "system",
      },
      {
        code: "acquired_user",
        delta: 40,
        source: { kind: "user_rule", id: "abc", name: "Synthetic pack" },
      },
      { code: "acquired_builtin", delta: 50, source: { kind: "builtin" } },
    ]),
    [
      {
        code: "search_user",
        delta: 30,
        kind: "score_contribution",
        origin: "user",
        ruleSetName: "Synthetic pack",
      },
      {
        code: "search_system",
        delta: -5,
        origin: "system",
        ruleSetName: "Synthetic system",
      },
      { code: "search_builtin", delta: 100 },
      {
        code: "saved_named",
        delta: 20,
        kind: "score_contribution",
        ruleSetName: "Synthetic pack",
      },
      { code: "saved_builtin", delta: 10, kind: "score_contribution" },
      {
        code: "saved_system",
        delta: 15,
        kind: "score_contribution",
        ruleSetName: "Synthetic system",
        origin: "system",
      },
      {
        code: "acquired_user",
        delta: 40,
        origin: "user",
        ruleSetName: "Synthetic pack",
      },
      { code: "acquired_builtin", delta: 50 },
    ],
  );
});

test("parses the stored acquisition log string with its release score", () => {
  const parsed = parseScoreLog(
    JSON.stringify({
      kind: "post_download_acquisition_score",
      release_score: 120,
      scoring_log: [
        {
          code: "quality_resolution_1080p",
          delta: 100,
          kind: "score_contribution",
          source: { kind: "builtin" },
        },
      ],
    }),
  );
  assert.equal(parsed.total, 120);
  assert.deepEqual(parsed.entries, [
    {
      code: "quality_resolution_1080p",
      delta: 100,
      kind: "score_contribution",
    },
  ]);
  assert.deepEqual(parseScoreLog("not json"), { entries: [] });
  assert.deepEqual(parseScoreLog(null), { entries: [] });
});

test("formats and tones scores like the interactive search", () => {
  assert.equal(formatScore(700), "+700");
  assert.equal(formatScore(-25), "-25");
  assert.equal(formatScore(0), "0");
  assert.equal(formatScore(null), "—");
  assert.equal(
    scoreEntryTone({ delta: 0, kind: "mandatory_rejection" }),
    "negative",
  );
  assert.equal(scoreEntryTone({ delta: 0 }), "positive");
});

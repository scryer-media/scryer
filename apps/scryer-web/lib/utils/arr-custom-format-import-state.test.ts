import assert from "node:assert/strict";
import test from "node:test";
import type { Diagnostic, ImportedCustomFormat } from "@/lib/arr-custom-format/types";
import {
  candidateScores,
  collectFormatScores,
  defaultScore,
  isCurrentTranslationRequest,
  recommendScore,
  sourceFacets,
} from "./arr-custom-format-import-state.ts";

function format(overrides: Partial<ImportedCustomFormat> = {}): ImportedCustomFormat {
  return {
    id: "format-id",
    name: "Format",
    source: "sonarr",
    specifications: [{ implementation: "ReleaseTitleSpecification", name: "Title", fields: { value: "HEVC" }, negate: false, required: true, index: 0 }],
    suggestedScores: {},
    ...overrides,
  };
}

test("exported scores take priority, with default preferred among profile choices", () => {
  const source = format({ suggestedScores: { default: 10 } });
  assert.deepEqual(candidateScores(source), [10]);
  assert.equal(defaultScore(source, []), "10");
  assert.equal(
    defaultScore(format({ suggestedScores: { default: 10, anime: 15 } }), []),
    "10",
  );
  const conflict: Diagnostic = {
    code: "arr.score.conflict",
    level: "error",
    message: "Choose explicitly.",
    formatId: source.id,
  };
  assert.equal(defaultScore(source, [conflict]), "");
  assert.equal(defaultScore(format({ name: "Extras", suggestedScores: { default: 300 } }), []), "300");
  assert.equal(recommendScore(format({ suggestedScores: { default: 10, anime: 15 } }), [])?.reasonKey, "settings.arrImportScoreReasonDefault");
});

test("clear preference and penalty names get conservative editable starting scores", () => {
  for (const [name, expected] of [
    ["Prefer HEVC", 100], ["Preferred WEB-DL", 100], ["Boost lossless audio", 100],
    ["LQ", -100], ["Low-quality groups", -100], ["Bad Dual Groups", -100],
    ["Avoid CAM", -100], ["Block unwanted releases", -100],
    ["Extras", -1000], ["Samples", -1000], ["Block extras", -1000],
    ["Prefer extras", 100],
  ] as const) {
    const recommendation = recommendScore(format({ name }), []);
    assert.equal(recommendation?.value, expected, name);
    assert.equal(defaultScore(format({ name }), []), String(expected), name);
    assert.ok(recommendation.value > -9000, name);
  }
  const imported = format({ name: "Prefer HEVC" });
  assert.deepEqual(collectFormatScores([imported], { [imported.id]: "250" }), { scores: { [imported.id]: 250 } });
});

test("neutral names, ambiguous exports, and malformed conditions do not invent scores", () => {
  for (const name of ["HEVC", "HDR", "1080p", "Release group", "No extras", "Not low quality", "LQless"]) {
    assert.equal(recommendScore(format({ name }), []), null, name);
  }
  assert.equal(recommendScore(format({ name: "Prefer HEVC", suggestedScores: { anime: 10, movies: 20 } }), []), null);
  assert.equal(recommendScore(format({ name: "Prefer HEVC", suggestedScores: { default: Infinity } }), []), null);
  assert.equal(recommendScore(format({ name: "Prefer HEVC", specifications: [] }), []), null);
  const malformed = format({ name: "Prefer HEVC", inspectionDiagnostics: [{ code: "arr.specification.invalid", level: "error", message: "Invalid", formatId: "format-id" }] });
  assert.equal(recommendScore(malformed, []), null);
});

test("negate alone never establishes a preference or an extras penalty", () => {
  const negated = format().specifications.map((spec) => ({ ...spec, negate: true }));
  assert.equal(recommendScore(format({ name: "HEVC", specifications: negated }), []), null);
  assert.equal(recommendScore(format({ name: "Extras", specifications: negated }), []), null);
});

test("hard-block scores require an explicit valid exported weight or user entry", () => {
  assert.equal(defaultScore(format({ name: "Block everything" }), []), "-100");
  assert.equal(defaultScore(format({ suggestedScores: { default: -10000 } }), []), "-10000");
  assert.deepEqual(candidateScores(format({ suggestedScores: { bad: 2147483648, fraction: 1.5, valid: 10 } })), [10]);
});

test("requires one signed 32-bit integer score per custom format", () => {
  const first = format({ id: "first" });
  const second = format({ id: "second", name: "Second" });
  assert.deepEqual(collectFormatScores([first, second], { first: "-9000", second: "25" }), {
    scores: { first: -9000, second: 25 },
  });
  for (const invalid of ["1.5", "Infinity", "9007199254740992", "2147483648", "-2147483649"]) {
    assert.deepEqual(collectFormatScores([first], { first: invalid }), { missingFormat: first });
  }
});

test("uses Arr-specific default facets and rejects stale worker responses", () => {
  assert.deepEqual(sourceFacets("radarr"), ["movie"]);
  assert.deepEqual(sourceFacets("sonarr"), ["series", "anime"]);
  assert.equal(isCurrentTranslationRequest(4, 4), true);
  assert.equal(isCurrentTranslationRequest(5, 4), false);
});

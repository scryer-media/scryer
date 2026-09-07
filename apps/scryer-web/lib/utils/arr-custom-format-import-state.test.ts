import assert from "node:assert/strict";
import test from "node:test";
import type { Diagnostic, ImportedCustomFormat } from "@/lib/arr-custom-format/types";
import {
  candidateScores,
  collectFormatScores,
  defaultScore,
  isCurrentTranslationRequest,
  sourceFacets,
} from "./arr-custom-format-import-state.ts";

function format(overrides: Partial<ImportedCustomFormat> = {}): ImportedCustomFormat {
  return {
    id: "format-id",
    name: "Format",
    source: "sonarr",
    specifications: [],
    suggestedScores: {},
    ...overrides,
  };
}

test("pre-fills only an unambiguous non-conflicting suggested score", () => {
  const source = format({ suggestedScores: { default: 10 } });
  assert.deepEqual(candidateScores(source), [10]);
  assert.equal(defaultScore(source, []), "10");
  assert.equal(
    defaultScore(format({ suggestedScores: { default: 10, anime: 15 } }), []),
    "",
  );
  const conflict: Diagnostic = {
    code: "arr.score.conflict",
    level: "error",
    message: "Choose explicitly.",
    formatId: source.id,
  };
  assert.equal(defaultScore(source, [conflict]), "");
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

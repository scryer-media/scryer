import assert from "node:assert/strict";
import test from "node:test";
import { deparse } from "rego-deparser";
import { compileLeafSpecification } from "./specifications.ts";
import type { ArrSource } from "./types.ts";

function compile(implementation: string, fields: Record<string, unknown>, source: ArrSource = "radarr", negate = false) {
  const result = compileLeafSpecification({ implementation, fields, negate, required: true, name: "condition", index: 0 }, {
    id: "format", name: "Format", source, specifications: [], suggestedScores: {},
  });
  assert.ok(result);
  return { ...result, source: deparse({ rules: [result.body, ...(result.alternatives ?? [])].map((body) => ({ head: { name: "matches" }, body })) }) };
}

test("source IDs are app-specific and retain remux distinctions", () => {
  assert.match(compile("SourceSpecification", { value: 3 }, "sonarr").source, /WEB-DL/);
  assert.match(compile("SourceSpecification", { value: 3 }, "radarr").source, /TELECINE/);
  assert.match(compile("SourceSpecification", { value: 6 }, "sonarr").source, /is_remux == false/);
  assert.match(compile("SourceSpecification", { value: 7 }, "sonarr").source, /is_remux == true/);
  assert.equal(compile("SourceSpecification", { value: 2 }, "sonarr").diagnostics[0]?.level, "error");
});

test("resolution accepts progressive and interlaced tokens at the same height", () => {
  const result = compile("ResolutionSpecification", { value: 1080 });
  assert.match(result.source, /1080p/);
  assert.match(result.source, /1080i/);
  assert.match(compile("ResolutionSpecification", { value: 0 }).source, /quality == null/);
  assert.equal(compile("ResolutionSpecification", { value: 4320 }).diagnostics[0]?.level, "error");
});

test("indexer freeleech uses metadata, with either explicit flag or volume factor", () => {
  const result = compile("IndexerFlagSpecification", { value: 1 });
  assert.equal(result.alternatives?.length, 1);
  assert.match(result.source, /freeleech/);
  assert.match(result.source, /downloadvolumefactor/);
  assert.doesNotMatch(result.source, /password|proper|repack/);
  assert.match(compile("IndexerFlagSpecification", { value: 8 }, "sonarr").source, /internal/);
  assert.equal(compile("IndexerFlagSpecification", { value: 8 }, "radarr").diagnostics[0]?.level, "error");
});

test("language exceptions and negation keep null guard and use actual original metadata", () => {
  const result = compile("LanguageSpecification", { value: -2, exceptLanguage: true }, "sonarr", true);
  assert.equal(result.handlesNegation, true);
  assert.match(result.source, /is_array\(input.release.languages_audio\)/);
  assert.match(result.source, /not scryer.lang_matches/);
  assert.match(result.source, /input.context.original_language/);
  assert.match(result.source, /== 0/);
  assert.doesNotMatch(result.source, /inferred_original/);
  for (const value of [19, 30, 37, 999]) {
    assert.equal(compile("LanguageSpecification", { value }).diagnostics[0]?.level, "error");
  }
});

test("size converts GiB with exclusive lower and inclusive upper bounds", () => {
  const result = compile("SizeSpecification", { min: 1, max: 2 });
  assert.match(result.source, /size_bytes > 1073741824/);
  assert.match(result.source, /size_bytes <= 2147483648/);
  // .NET rounds midpoint values to even, including half-byte boundaries.
  assert.match(compile("SizeSpecification", { min: 2.5 / 1024 ** 3, max: 3.5 / 1024 ** 3 }).source, /size_bytes > 2/);
  assert.match(compile("SizeSpecification", { min: 2.5 / 1024 ** 3, max: 3.5 / 1024 ** 3 }).source, /size_bytes <= 4/);
  assert.equal(compile("SizeSpecification", { min: 0, max: Number.MAX_VALUE }).diagnostics[0]?.level, "error");
});

test("year is disabled because release year cannot reproduce Radarr's metadata fallback", () => {
  const result = compile("YearSpecification", { min: 2000, max: 2020 });
  assert.equal(result.diagnostics[0]?.code, "arr.year.unmapped");
  assert.equal(result.diagnostics[0]?.level, "error");
});

test("unrecognized fields and conditions cannot silently weaken formats", () => {
  assert.equal(compile("SourceSpecification", { value: 7, futureOption: true }).diagnostics[0]?.code, "arr.fields.unsupported");
  assert.equal(compile("FutureSpecification", { value: 1 }).diagnostics[0]?.code, "arr.implementation.unsupported");
});

import assert from "node:assert/strict";
import test from "node:test";

import { inspectCustomFormats } from "./inspection.ts";

test("inspection accepts an Arr export array and preserves score choices", () => {
  const result = inspectCustomFormats(JSON.stringify([
    {
      trash_id: "one",
      name: "Imported format",
      trash_scores: { default: 100, anime: -10_000 },
      specifications: [{ name: "Title", implementation: "ReleaseTitleSpecification", required: true, negate: false, fields: { value: "test" } }],
    },
  ]), "sonarr");

  assert.equal(result.fatal, false);
  assert.deepEqual(result.formats[0]?.suggestedScores, { default: 100, anime: -10_000 });
  assert.deepEqual(result.formats[0]?.specifications[0]?.fields, { value: "test" });
});

test("inspection remains safe for malformed or structurally invalid JSON", () => {
  assert.equal(inspectCustomFormats("{ nope", "radarr").fatal, true);
  const result = inspectCustomFormats({ name: "missing specs" }, "radarr");
  assert.equal(result.fatal, true);
  assert.equal(result.diagnostics[0]?.code, "arr.format.specifications_missing");
});

test("inspection accepts Arr's field-array export shape", () => {
  const result = inspectCustomFormats({
    id: 12,
    name: "Array fields",
    specifications: [{
      name: "Title",
      implementation: "ReleaseTitleSpecification",
      fields: [{ name: "value", value: "word" }],
    }],
  }, "radarr");

  assert.equal(result.formats[0]?.id, "$:12");
  assert.equal(result.formats[0]?.specifications[0]?.fields.value, "word");
});

test("invalid optional specifications remain errors on their whole format", () => {
  const result = inspectCustomFormats({ name: "Partial", specifications: [
    { implementation: "ReleaseTitleSpecification", fields: { value: "valid" } },
    { implementation: "ReleaseTitleSpecification", required: "false", fields: { value: "invalid" } },
  ] }, "radarr");
  assert.equal(result.formats.length, 1);
  assert.equal(result.formats[0]?.inspectionDiagnostics?.[0]?.level, "error");
  assert.equal(result.diagnostics[0]?.formatId, result.formats[0]?.id);
});

test("duplicate export IDs remain distinct within a batch", () => {
  const format = { id: 1, name: "Same", specifications: [{ implementation: "SourceSpecification", fields: { value: 7 } }] };
  const result = inspectCustomFormats([format, format], "radarr");
  assert.notEqual(result.formats[0]?.id, result.formats[1]?.id);
});

test("conflicting scores can be resolved without disabling the format", () => {
  const result = inspectCustomFormats({ name: "Scores", score: 20, trash_scores: { default: 10 }, specifications: [{ implementation: "SourceSpecification", fields: { value: 7 } }] }, "radarr");
  assert.equal(result.diagnostics[0]?.level, "warning");
  assert.equal(result.diagnostics[0]?.formatId, result.formats[0]?.id);
  assert.deepEqual(Object.values(result.formats[0]!.suggestedScores).sort(), [10, 20]);
});

test("inspection bounds pasted bytes and batch size", () => {
  assert.equal(inspectCustomFormats(" ".repeat(1_000_001), "sonarr").diagnostics[0]?.code, "arr.json.too_large");
  assert.equal(inspectCustomFormats(Array(1_001).fill({}), "sonarr").diagnostics[0]?.code, "arr.formats.too_many");
});

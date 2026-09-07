import assert from "node:assert/strict";
import test from "node:test";

import { inspectCustomFormats } from "./inspection.ts";
import { translateCustomFormats } from "./translate.ts";

function format(specifications: unknown[]) {
  const inspected = inspectCustomFormats({ id: "format-id", name: "Import test", specifications }, "radarr");
  assert.equal(inspected.fatal, false);
  return inspected.formats;
}

function scores(formats: ReturnType<typeof format>, value: number) {
  return { [formats[0]!.id]: value };
}

test("translation emits AST-deparsed Rego and honors a negated required specification", () => {
  const imported = format([
    { name: "wanted", implementation: "ReleaseTitleSpecification", required: true, negate: false, fields: { value: "wanted" } },
    { name: "excluded", implementation: "ReleaseTitleSpecification", required: true, negate: true, fields: { value: "excluded" } },
  ]);
  const result = translateCustomFormats(imported, scores(imported, 42));

  assert.equal(result.formats[0]?.status, "translated");
  assert.match(result.regoSource, /import rego\.v1/);
  assert.match(result.regoSource, /regex\.match\("\(\?i\)wanted", regex\.replace\(input\.release\.raw_title/);
  assert.match(result.regoSource, /not _arr_0_spec_1/);
  assert.match(result.regoSource, /score_entry\["import_test_1"\] := 42/);
});

test("required specifications alone gate a group; optional siblings do not become mandatory", () => {
  const imported = format([
    { name: "required", implementation: "ReleaseTitleSpecification", required: true, negate: false, fields: { value: "required" } },
    { name: "optional", implementation: "ReleaseTitleSpecification", required: false, negate: false, fields: { value: "optional" } },
  ]);
  const result = translateCustomFormats(imported, scores(imported, 1));

  const group = result.regoSource.match(/_arr_0_group_0 if \{([\s\S]*?)\n\}/)?.[1] ?? "";
  assert.match(group, /_arr_0_spec_0/);
  assert.doesNotMatch(group, /_arr_0_spec_1/);
});

test("all-optional groups are a disjunction", () => {
  const imported = format([
    { name: "first", implementation: "ReleaseTitleSpecification", required: false, negate: false, fields: { value: "first" } },
    { name: "second", implementation: "ReleaseTitleSpecification", required: false, negate: false, fields: { value: "second" } },
  ]);
  const result = translateCustomFormats(imported, scores(imported, 1));

  assert.match(result.regoSource, /_arr_0_group_0 if \{\n {2}_arr_0_spec_0\n\}/);
  assert.match(result.regoSource, /_arr_0_group_0 if \{\n {2}_arr_0_spec_1\n\}/);
});

test("one incompatible optional condition disables the whole imported format", () => {
  const imported = format([
    { name: "valid", implementation: "ReleaseTitleSpecification", required: true, negate: false, fields: { value: "valid" } },
    { name: "unsupported", implementation: "ReleaseTitleSpecification", required: false, negate: false, fields: { value: "(foo)\\1" } },
  ]);
  const result = translateCustomFormats(imported, scores(imported, 7));

  assert.equal(result.formats[0]?.status, "disabled");
  assert.equal(result.formats[0]?.diagnostics[0]?.code, "regex.backreference_unsupported");
  assert.match(result.regoSource, /score_entry\["import_test_1"\] := 7 if \{\n {2}false\n\}/);
});

test("positional lookarounds use source-position relations", () => {
  const imported = format([
    { name: "position", implementation: "ReleaseTitleSpecification", required: true, negate: false, fields: { value: "foo(?!bar)" } },
  ]);
  const result = translateCustomFormats(imported, scores(imported, 7));

  assert.equal(result.formats[0]?.status, "translated");
  assert.match(result.regoSource, /numbers\.range/);
  assert.match(result.regoSource, /not _arr_0_spec_0_position_1/);
  assert.doesNotMatch(result.regoSource, /foo\(\?!bar\)/);
});

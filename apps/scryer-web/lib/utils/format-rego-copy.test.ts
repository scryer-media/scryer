import assert from "node:assert/strict";
import test from "node:test";
import { formatRegoCopy } from "./format-rego-copy.ts";

test("expands compact generated rule bodies and spaces expression separators", () => {
  const source = 'trash_group_string(field) := value if { value := object.get(input.release,field,""); is_string(value) }';
  assert.equal(formatRegoCopy(source), [
    "trash_group_string(field) := value if {",
    '  value := object.get(input.release, field, "")',
    "  is_string(value)",
    "}",
  ].join("\n"));
});

test("preserves quoted strings, escapes, raw data and comments as opaque text", () => {
  const quoted = String.raw`"quote: \"; if { x := y }, \\n"`;
  const raw = "`first; { if # ,\n  second \" } :=`";
  const source = `# metadata: { untouched; }\nvalue := ${raw}\nf if { x := ${quoted}; x != "" # keep; } and comment spacing\n}\n`;
  const formatted = formatRegoCopy(source);
  assert.ok(formatted.includes(quoted));
  assert.ok(formatted.includes(raw));
  assert.ok(formatted.includes("# metadata: { untouched; }"));
  assert.ok(formatted.includes('# keep; } and comment spacing\n}'));
  assert.equal(formatRegoCopy(formatted), formatted);
});

test("keeps collection and comprehension structure and unary/scientific numbers", () => {
  const source = 'f(x) := y if { weights := {"a":-10000,"b":1e+10}; y := [v | v := x[_]; v>0]; z := {v | v := x[_]}; y != [] }';
  const formatted = formatRegoCopy(source);
  assert.ok(formatted.includes('weights := {"a": -10000, "b": 1e+10}'));
  assert.ok(formatted.includes("y := [v | v := x[_]; v > 0]"));
  assert.ok(formatted.includes("z := {v | v := x[_]}"));
  assert.equal(formatRegoCopy(formatted), formatted);
});

test("indents multiline lookup objects without changing keys or values", () => {
  const source = 'groups := {\n"ONE": {"tier":1},\n"TWO": {"tier":2},\n}\nf if{groups["ONE"].tier==1}\n';
  assert.equal(formatRegoCopy(source), 'groups := {\n  "ONE": {"tier": 1},\n  "TWO": {"tier": 2},\n}\nf if {\n  groups["ONE"].tier == 1\n}\n');
});

test("formats nested bodies and else without detaching the else clause", () => {
  const source = 'f := true if { true } else := false if { false }\ng := true if { [v | v := [1,2][_]] != [] }';
  assert.equal(formatRegoCopy(source), 'f := true if {\n  true\n} else := false if {\n  false\n}\ng := true if {\n  [v | v := [1, 2][_]] != []\n}');
});

test("is idempotent with blank lines, CRLF and existing indentation", () => {
  for (const source of ["", "\n\n", "\n\nimport rego.v1\n\n", 'f if {\r\n\ttrue\r\n}\r\n', 'f if { true;\n\nfalse }']) {
    const formatted = formatRegoCopy(source);
    assert.equal(formatRegoCopy(formatted), formatted);
  }
});

test("ordinary identifiers cannot be mistaken for delimiters", () => {
  assert.equal(formatRegoCopy('f if { constructor := {"__proto__":1}; constructor["__proto__"] == 1 }'), 'f if {\n  constructor := {"__proto__": 1}\n  constructor["__proto__"] == 1\n}');
});

test("retains original source when delimiters or strings are incomplete", () => {
  for (const source of ['f if { true', 'x := "unterminated', 'x := `unterminated', 'x := ([)]', 'x := "escaped\\"', 'f if { # missing close }']) {
    assert.equal(formatRegoCopy(source), source);
  }
});

import assert from "node:assert/strict";
import test from "node:test";
import type { Module, Rule } from "rego-deparser";

import { ref } from "./ast.ts";
import { evaluate } from "./__fixtures__/evaluate-ast.ts";
import { compileRegex } from "./translate.ts";

test("position relations preserve a local negative lookahead", () => {
  const spec = { implementation: "ReleaseTitleSpecification", name: "title", negate: false, required: true, fields: { value: "foo(?!bar)" }, index: 0 };
  const compiled = compileRegex("foo(?!bar)", ref("input", "release", "raw_title"), "format", spec, "rx");
  assert.deepEqual(compiled.diagnostics, []);
  const rule: Rule = { head: { name: "matches" }, body: compiled.body };
  const module: Module = { rules: [...(compiled.rules ?? []), rule] };
  assert.equal(evaluate(module, { release: { raw_title: "foobar" } }, "matches"), false);
  assert.equal(evaluate(module, { release: { raw_title: "foo" } }, "matches"), true);
});

const cases: Array<[string, string[]]> = [
  ["foo(?=bar\\b)", ["foobar", "foobarX", "foobar!", "xxfoobar!", "foobarX foobar!"]],
  ["foo(?!bar|baz)", ["foobar", "foobaz", "fooqux", "foobar fooqux", "FOOqux"]],
  ["(?:^foo)(?=bar)", ["foobar", "xfoobar", "foobar\n", "x\nfoobar"]],
  ["(?<=foo)bar", ["foobar", "xxfoobar", "barfoo", "bar"]],
  ["(?<!foo|xx)bar", ["foobar", "xxbar", "xbar", "bar", "foobar bar"]],
  ["(?<!foo.*)bar", ["bar", "fooXXbar", "foo\nbar", "xxbar"]],
  ["\\bfoo(?=x)", ["foox", "afoox", "!foox", "😀foox"]],
  ["\\Bfoo(?=x)", ["foox", "afoox", "!foox", "😀foox"]],
  ["foo(?=x$)", ["foox", "foox\n", "foox\r\n", "fooxX"]],
  ["foo(?=|bar)", ["foo", "foobar", "fooz"]],
  ["foo(?!|bar)", ["foo", "foobar", "fooz"]],
  ["a+(?=ab)", ["aab", "aaab", "ab", "baab"]],
  ["(?:a|aa)(?=ab)", ["aab", "aaab", "ab", "baab"]],
  ["foo(?=b(?!az))", ["foobar", "foobaz", "foob", "foox"]],
  ["(?:(?=foo)foo|bar)(?=x)", ["foox", "barx", "foobar", "xbarx"]],
];

for (const [pattern, inputs] of cases) test(`emitted AST matches reference regex behavior: ${pattern}`, () => {
  const spec = { implementation: "ReleaseTitleSpecification", name: "title", negate: false, required: true, fields: { value: pattern }, index: 0 };
  const compiled = compileRegex(pattern, ref("input", "release", "raw_title"), "format", spec, "rx");
  assert.deepEqual(compiled.diagnostics, []);
  const module: Module = { rules: [...(compiled.rules ?? []), { head: { name: "matches" }, body: compiled.body }] };
  for (const raw_title of inputs) {
    // .NET's non-multiline `$` permits a final LF, but not a preceding CR.
    const referencePattern = pattern.replaceAll("$", "(?=\\n?$)(?!\\r)");
    assert.equal(evaluate(module, { release: { raw_title } }, "matches"), new RegExp(referencePattern, "iu").test(raw_title), JSON.stringify({ pattern, raw_title }));
  }
});

test("unsupported regex modes and capture state disable compilation", () => {
  for (const pattern of ["(?s)foo", "(?m)foo", "(?x)foo", "(?-i)foo", "(?>foo)", "foo++", "(foo)\\1", "\\Gfoo", "(?=foo)*", "foo\u0001"]) {
    const compiled = compileRegex(pattern, ref("input", "release", "raw_title"), "format", { implementation: "ReleaseTitleSpecification", name: "title", negate: false, required: false, fields: { value: pattern }, index: 0 }, "rx");
    assert.equal(compiled.diagnostics[0]?.level, "error", pattern);
  }
});

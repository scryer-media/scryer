import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { EditorState, type Transaction } from "@codemirror/state";
import { snippet, nextSnippetField } from "@codemirror/autocomplete";
import type { EditorView } from "@codemirror/view";
import {
  regoCatalog,
  regoContracts,
  regoFields,
  regoFieldCompletions,
  regoCompletionContext,
  normalizeRegoPath,
  REGO_INDEX_SOURCE,
  type RegoFamily,
} from "./rego-assistance.ts";
import {
  graphQlRuleValidationResult,
  regoDiagnostics,
  regoSourceLine,
} from "./rego-diagnostics.ts";

// The test runs from the source tree in CI and from a compiled copy locally, so
// the repository root is found by walking up rather than by a fixed depth.
function repoRoot(): string {
  let dir = dirname(fileURLToPath(import.meta.url));
  while (!existsSync(join(dir, "crates", "scryer-rules"))) {
    const parent = dirname(dir);
    if (parent === dir)
      throw new Error("repository root with crates/scryer-rules not found");
    dir = parent;
  }
  return dir;
}

for (const family of ["release", "request", "maintenance"] as RegoFamily[]) {
  test(`${family} completions use the exact backend contract`, () => {
    const file = family === "release" ? "rule" : family;
    const backend = JSON.parse(
      readFileSync(
        join(
          repoRoot(),
          "crates",
          "scryer-rules",
          `${file}-input-contract.json`,
        ),
        "utf8",
      ),
    );
    assert.deepEqual(regoContracts[family], backend);
    const fields = regoFields(family);
    for (const section of backend.sections)
      for (const field of section.fields) {
        assert.equal(
          fields.find((item) => item.path === `${section.path}.${field.field}`)
            ?.type,
          field.type,
        );
        assert.equal(
          fields.find((item) => item.path === `${section.path}.${field.field}`)
            ?.descKey,
          field.descKey,
        );
      }
  });
}

test("scoring fields, nullable group, arrays and family isolation", () => {
  assert.equal(
    regoFieldCompletions("release", "input.release").find(
      (f) => f.path === "input.release.protocol",
    )?.type,
    "string?",
  );
  assert.ok(
    regoFieldCompletions("release", "input.context").some(
      (f) => f.path === "input.context.tags",
    ),
  );
  assert.match(
    regoFieldCompletions("release", "input.release").find(
      (f) => f.path === "input.release.release_group",
    )!.type,
    /string\?/,
  );
  for (const index of ["0", "_", "i", " 12 "]) {
    const source = `input.file.audio_streams[${index}].co`;
    const context = regoCompletionContext(source, source.length)!;
    assert.equal(context.parent, "input.file.audio_streams[]");
    assert.equal(context.prefix, "co");
    assert.ok(
      regoFieldCompletions("release", context.parent).some((f) =>
        f.path.endsWith(".codec"),
      ),
    );
  }
  assert.deepEqual(regoFieldCompletions("request", "input.release"), []);
  assert.deepEqual(regoFieldCompletions("maintenance", "input.requester"), []);
  assert.deepEqual(regoFieldCompletions("release", "input.release.extra"), []);
  assert.ok(
    regoFields("release").find((f) => f.path === "input.file")?.availabilityKey,
  );
});

test("cursor context excludes comments, strings and unsupported aliases", () => {
  for (const source of [
    "# input.",
    '"input.',
    "`raw\ninput.",
    '"escaped \\" input.',
    "alias.field.",
    "alias.input.context.",
  ]) {
    assert.equal(regoCompletionContext(source, source.length), null, source);
  }
  const source = "# comment\ninput.context.";
  assert.equal(
    regoCompletionContext(source, source.length)?.parent,
    "input.context",
  );
  assert.equal(regoCompletionContext("input.", 6)?.from, 6);
  const numbered = "input.release.is_hdr10";
  assert.equal(
    regoCompletionContext(numbered, numbered.length)?.prefix,
    "is_hdr10",
  );
});

test("the bracket index accepts exactly what the whitespace-ambiguous form accepted", () => {
  const ambiguous = /^\[\s*(?:\d+|[A-Za-z_]\w*)?\s*\]$/;
  const current = new RegExp(`^${REGO_INDEX_SOURCE}$`);
  const alphabet = ["[", "]", " ", "\t", "1", "a", "_", "-"];
  let candidates = [""];
  for (let length = 1; length <= 6; length++) {
    candidates = candidates.flatMap((prefix) =>
      alphabet.map((char) => prefix + char),
    );
    for (const candidate of candidates) {
      assert.equal(
        current.test(candidate),
        ambiguous.test(candidate),
        JSON.stringify(candidate),
      );
    }
  }
  assert.equal(
    normalizeRegoPath("input.file.audio_streams[ ].codec"),
    "input.file.audio_streams[].codec",
  );
  assert.equal(
    normalizeRegoPath("input.file.audio_streams[ 3 ][i]"),
    "input.file.audio_streams[][]",
  );
});

test("a long run of blank bracket groups does not stall completion or normalization", () => {
  const blanks = "[  ]".repeat(200);
  for (const tail of ["-", " ", "["]) {
    const source = `input${blanks}${tail}`;
    const context = regoCompletionContext(source, source.length);
    assert.equal(context?.parent ?? "", "");
  }
  const typed = `input.file.audio_streams${blanks}.co`;
  const context = regoCompletionContext(typed, typed.length)!;
  assert.equal(context.parent, `input.file.audio_streams${"[]".repeat(200)}`);
  assert.equal(context.prefix, "co");
  assert.equal(
    normalizeRegoPath(`input${"[  ".repeat(200)}`),
    `input${"[  ".repeat(200)}`,
  );
});

test("every field and catalog item has a localized description", () => {
  const english = readFileSync(
    new URL("../i18n/locales/en.ts", import.meta.url),
    "utf8",
  );
  for (const family of ["release", "request", "maintenance"] as RegoFamily[]) {
    for (const field of regoFields(family))
      assert.ok(english.includes(`"${field.descKey}"`), field.descKey);
  }
  for (const item of [...regoCatalog.helpers, ...regoCatalog.snippets])
    assert.ok(english.includes(`"${item.descKey}"`), item.descKey);
});

test("diagnostics account for inserted package and import lines", () => {
  assert.equal(regoSourceLine("match if { true }", 3), 1);
  assert.equal(regoSourceLine("import rego.v1\nmatch if { true }", 3), 2);
  assert.equal(regoSourceLine("package example\nmatch if { true }", 3), 2);
  assert.equal(
    regoSourceLine("package example\nimport rego.v1\nmatch if { true }", 3),
    3,
  );
  const result = {
    valid: false,
    errors: ["new compiler words"],
    diagnostics: [
      {
        code: "RULE_COMPILATION_ERROR",
        message: "new compiler words",
        line: 3,
        column: 4,
      },
    ],
  };
  assert.equal(regoDiagnostics("match :=", result)[0]?.line, 1);
  assert.deepEqual(
    regoDiagnostics("match :=", { ...result, unavailable: true }),
    [],
  );
  assert.deepEqual(
    regoDiagnostics("match :=", {
      valid: false,
      errors: ["no reliable position"],
    }),
    [],
  );
  assert.deepEqual(
    regoDiagnostics("match :=", {
      valid: false,
      errors: ["wrapper.rego:20:3"],
    }),
    [],
  );
});

test("native snippets insert their defaults and select editable placeholders", () => {
  for (const item of regoCatalog.snippets) {
    const editor = {
      state: EditorState.create({ doc: item.label }),
      dispatch(transaction: Transaction) {
        editor.state = transaction.state;
      },
    };
    const view = editor as unknown as EditorView;
    snippet(item.source)(view, { label: item.label }, 0, item.label.length);
    assert.equal(
      editor.state.doc.toString().replace(/\s+/g, " "),
      item.source.replace(/\$\{\d+:([^}]+)\}/g, "$1").replace(/\s+/g, " "),
    );
    const placeholders = [...item.source.matchAll(/\$\{\d+:([^}]+)\}/g)].map(
      (match) => match[1],
    );
    for (let i = 0; i < placeholders.length; i++) {
      const selection = editor.state.selection.main;
      assert.equal(
        editor.state.sliceDoc(selection.from, selection.to),
        placeholders[i],
      );
      if (i + 1 < placeholders.length)
        assert.equal(nextSnippetField(view), true);
    }
    assert.equal(
      nextSnippetField(view),
      false,
      "Tab falls through after snippet fields",
    );
  }
});

test("owned input path diagnostics use structured facts after rewording", () => {
  const source = "match if { input.subject.unknown }";
  const result = {
    valid: false,
    errors: ["arbitrary prose with no location"],
    diagnostics: [
      {
        code: "RULE_UNKNOWN_INPUT_PATH",
        message: "arbitrary prose",
        path: "input.subject.unknown",
      },
    ],
  };
  assert.deepEqual(regoDiagnostics(source, result), [
    { line: 1, column: 12, message: "arbitrary prose" },
  ]);
  assert.deepEqual(regoDiagnostics(`${source}\n${source}`, result), []);
});

test("save refusal diagnostics retain editor placement after sentence changes", () => {
  const result = graphQlRuleValidationResult(
    {
      graphQLErrors: [
        {
          message: "new save wording",
          extensions: {
            code: "VALIDATION_ERROR",
            reason: "RULE_VALIDATION_FAILED",
            diagnostics: [
              {
                code: "RULE_COMPILATION_ERROR",
                message: "new compiler wording",
                line: 3,
                column: 4,
                path: null,
              },
            ],
          },
        },
      ],
    },
    "Save refused",
  );
  assert.deepEqual(regoDiagnostics("match :=", result), [
    { line: 1, column: 4, message: "new compiler wording" },
  ]);
});

test("a diagnostics-shaped extension on another error family cannot paint rule markers", () => {
  for (const code of [
    "PLUGIN_FAILED",
    "NOT_FOUND",
    "UNAUTHORIZED",
    "UNKNOWN_FAMILY",
  ]) {
    const result = graphQlRuleValidationResult(
      {
        graphQLErrors: [
          {
            message: "unrelated failure",
            extensions: {
              code,
              reason: "RULE_VALIDATION_FAILED",
              diagnostics: [
                {
                  code: "RULE_COMPILATION_ERROR",
                  message: "spoofed diagnostic",
                  line: 3,
                  column: 4,
                },
              ],
            },
          },
        ],
      },
      "Failed",
    );
    assert.deepEqual(result.diagnostics, []);
    assert.deepEqual(regoDiagnostics("match :=", result), []);
  }
});

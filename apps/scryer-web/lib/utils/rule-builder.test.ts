import assert from "node:assert/strict";
import test from "node:test";

import {
  RULE_BUILDER_MODEL_VERSION,
  RULE_BUILDER_PROBLEM_CODES,
  emptyRuleBuilderModel,
  generateRuleSource,
  isBlankRuleBuilderModel,
  isRuleBuilderSource,
  parseRuleBuilderModel,
  ruleBuilderConflicts,
  ruleBuilderFields,
  ruleBuilderMissingRequirements,
  ruleBuilderOperator as op,
  ruleBuilderOptionSources,
  type RuleBuilderModel,
  type RuleBuilderProblem,
  type RuleBuilderSchema,
} from "./rule-builder.ts";

/// A small schema that exercises every part of the generator without leaning
/// on any real family.
const SCHEMA: RuleBuilderSchema = {
  family: "sample",
  idPrefix: "sample-builder",
  preamble: "package sample\n\n",
  helperRule: "matches",
  contract: {
    sections: [
      {
        path: "input.item",
        fields: [
          { field: "flag", type: "bool", descKey: "sample.flag" },
          { field: "size", type: "number", descKey: "sample.size" },
          { field: "name", type: "string", descKey: "sample.name" },
          { field: "seen_at", type: "string", descKey: "sample.seenAt" },
          { field: "labels", type: "string[]", descKey: "sample.labels" },
          { field: "color", type: "string", descKey: "sample.color" },
          { field: "hidden", type: "string", descKey: "sample.hidden" },
          { field: "parts", type: "object[]", descKey: "sample.parts" },
          {
            field: "wide_only",
            type: "bool",
            descKey: "sample.wideOnly",
            scopes: ["WIDE"],
          },
          { field: "Bad-Name", type: "bool", descKey: "sample.bad" },
        ],
      },
      {
        path: "input.extra",
        fields: [{ field: "count", type: "number", descKey: "sample.count" }],
      },
    ],
  },
  conditionSources: [
    {
      section: "input.item",
      label: "field",
      exclude: ["hidden"],
      fields: {
        color: {
          kind: "choice",
          choices: [
            { value: "red", labelKey: "sample.red" },
            { value: "blue", labelKey: "sample.blue" },
          ],
        },
      },
    },
    { section: "input.extra", label: "path", variants: ["WIDE"] },
  ],
  typeRules: [
    { type: "bool", kind: "bool" },
    { type: "number", kind: "number" },
    { type: "string[]", kind: "list" },
    { type: "string", suffix: "_at", kind: "time" },
    { type: "string", kind: "text" },
  ],
  kinds: {
    bool: { operators: [op("IS_TRUE", "none", "{path}")] },
    number: {
      operators: [
        op("GREATER_THAN", "number", "is_number({path})\n{path} > {value}"),
      ],
    },
    text: { operators: [op("EQUALS", "text", "{path} == {value}")] },
    time: {
      operators: [op("MORE_THAN_DAYS_AGO", "days", "age({path}) > {value}")],
    },
    list: { operators: [op("CONTAINS", "text", "{value} in {path}")] },
    choice: { operators: [op("EQUALS", "choice", "{path} == {value}")] },
  },
  outputs: [
    {
      id: "KEEP",
      head: "keep",
      labelKey: "sample.keep",
      value: { kind: "flag" },
      template: "{head} if {helper}",
      conflictsWith: ["DROP"],
    },
    {
      id: "DROP",
      head: "drop",
      labelKey: "sample.drop",
      value: { kind: "flag" },
      template: "{head} if {helper}",
    },
    {
      id: "WIDEN",
      head: "widen",
      labelKey: "sample.widen",
      value: { kind: "flag" },
      template: "{head} if {helper}",
      variants: ["WIDE"],
    },
    {
      id: "NOTE",
      head: "note",
      labelKey: "sample.note",
      value: { kind: "text", maxBytes: 8, pattern: "[a-z ]+" },
      template: "{head} := {value} if {helper}",
    },
    {
      id: "PICK",
      head: "pick",
      labelKey: "sample.pick",
      value: { kind: "choice", options: "picks" },
      template: "{head} := {value} if {helper}",
    },
    {
      id: "WEIGHT",
      head: "weight",
      labelKey: "sample.weight",
      value: { kind: "number", min: -10, max: 10 },
      template: "{head} := {value} if {helper}",
    },
    {
      id: "LABELS",
      head: "labels",
      labelKey: "sample.labels",
      value: { kind: "text_list", options: "labels", maxItems: 2 },
      template: "{head} := [{items}] if {helper}",
    },
    {
      id: "EACH",
      head: "each",
      labelKey: "sample.each",
      value: { kind: "text_list", options: "labels" },
      template: "{head} contains {item} if {helper}",
      perItem: true,
    },
    {
      id: "ENTRY",
      head: "entry",
      labelKey: "sample.entry",
      value: {
        kind: "structured",
        parts: [
          {
            id: "code",
            kind: "text",
            labelKey: "sample.code",
            pattern: "[a-z_]+",
            maxBytes: 6,
          },
          {
            id: "points",
            kind: "number",
            labelKey: "sample.points",
            min: 0,
            max: 100,
          },
        ],
      },
      template: "{head}[{part.code}] := {part.points} if {helper}",
    },
    {
      id: "AFTER",
      head: "after",
      labelKey: "sample.after",
      value: { kind: "flag" },
      template: "{head} if {helper}",
      requires: ["NOTE"],
    },
  ],
  outputRules: [
    { rule: "at_least_one" },
    { rule: "at_most_one", of: ["PICK", "WEIGHT"] },
  ],
  labels: {
    outputs: "sample.outputs",
    outputsHelp: "sample.outputsHelp",
    conditionsHelp: "sample.conditionsHelp",
  },
};

function model(overrides: Partial<RuleBuilderModel> = {}): RuleBuilderModel {
  return {
    version: RULE_BUILDER_MODEL_VERSION,
    family: "sample",
    join: "ALL",
    conditions: [{ field: "input.item.flag", operator: "IS_TRUE", value: "" }],
    outputs: { KEEP: true },
    ...overrides,
  };
}

function source(candidate: RuleBuilderModel, variant: string | null = null) {
  const result = generateRuleSource(SCHEMA, candidate, variant);
  assert.equal(result.ok, true, JSON.stringify(result));
  return result.ok ? result.source : "";
}

function problems(
  candidate: RuleBuilderModel,
  variant: string | null = null,
): RuleBuilderProblem[] {
  const result = generateRuleSource(SCHEMA, candidate, variant);
  assert.equal(result.ok, false);
  return result.ok ? [] : result.problems;
}

function codes(candidate: RuleBuilderModel, variant: string | null = null) {
  return problems(candidate, variant).map((problem) => problem.code);
}

// ── Shape ─────────────────────────────────────────────────────────────

test("ALL writes one helper body and ANY writes one body per condition", () => {
  const conditions = [
    { field: "input.item.flag", operator: "IS_TRUE", value: "" },
    { field: "input.item.size", operator: "GREATER_THAN", value: "2.5" },
  ];
  assert.equal(
    source(model({ conditions })),
    "package sample\n\nmatches if {\n\tinput.item.flag\n\tis_number(input.item.size)\n\tinput.item.size > 2.5\n}\n\nkeep if matches\n",
  );
  assert.equal(
    source(model({ join: "ANY", conditions })),
    "package sample\n\nmatches if {\n\tinput.item.flag\n}\n\nmatches if {\n\tis_number(input.item.size)\n\tinput.item.size > 2.5\n}\n\nkeep if matches\n",
  );
});

test("outputs are written in schema order whatever order the model lists them", () => {
  const outputs = {
    EACH: ["b", "a"],
    WEIGHT: "-3",
    NOTE: "hi",
    LABELS: ["x", "y"],
    ENTRY: { code: "bonus", points: "7" },
    KEEP: true as const,
  };
  const expected =
    'package sample\n\nmatches if {\n\tinput.item.flag\n}\n\nkeep if matches\n\nnote := "hi" if matches\n\nweight := -3 if matches\n\nlabels := ["x", "y"] if matches\n\neach contains "b" if matches\n\neach contains "a" if matches\n\nentry["bonus"] := 7 if matches\n';
  assert.equal(source(model({ outputs })), expected);
  const reversed = Object.fromEntries(Object.entries(outputs).reverse());
  assert.equal(source(model({ outputs: reversed })), expected);
});

test("generation is deterministic and leaves the model untouched", () => {
  const candidate = model({
    join: "ANY",
    conditions: [
      { field: "input.item.name", operator: "EQUALS", value: "a" },
      { field: "input.item.labels", operator: "CONTAINS", value: "b" },
    ],
    outputs: { KEEP: true, EACH: ["c"] },
  });
  const before = JSON.stringify(candidate);
  const first = generateRuleSource(SCHEMA, candidate, null);
  const second = generateRuleSource(SCHEMA, structuredClone(candidate), null);
  assert.deepEqual(first, second);
  assert.equal(JSON.stringify(candidate), before);
});

test("text values are escaped Rego literals and never read as placeholders", () => {
  const written = source(
    model({
      conditions: [
        {
          field: "input.item.name",
          operator: "EQUALS",
          value: 'say "hi" \\ {path} {value}',
        },
        {
          field: "input.item.labels",
          operator: "CONTAINS",
          value: "Ünïcødé 😀",
        },
      ],
      outputs: { KEEP: true, EACH: ["{helper}", 'q"uote'] },
    }),
  );
  assert.ok(
    written.includes(
      '\tinput.item.name == "say \\"hi\\" \\\\ {path} {value}"\n',
    ),
  );
  assert.ok(written.includes('\t"Ünïcødé 😀" in input.item.labels\n'));
  assert.ok(written.includes('each contains "{helper}" if matches\n'));
  assert.ok(written.includes('each contains "q\\"uote" if matches\n'));
});

test("isRuleBuilderSource accepts the source as written or without its preamble only", () => {
  const candidate = model();
  const written = source(candidate);
  assert.equal(isRuleBuilderSource(SCHEMA, candidate, null, written), true);
  assert.equal(
    isRuleBuilderSource(
      SCHEMA,
      candidate,
      null,
      written.slice(SCHEMA.preamble.length),
    ),
    true,
  );
  assert.equal(
    isRuleBuilderSource(SCHEMA, candidate, null, `${written}\n`),
    false,
  );
  assert.equal(
    isRuleBuilderSource(SCHEMA, candidate, null, written.replace("\t", "  ")),
    false,
  );
  assert.equal(
    isRuleBuilderSource(SCHEMA, model({ outputs: {} }), null, ""),
    false,
  );
});

// ── Fields ────────────────────────────────────────────────────────────

test("fields come from the contract sections, kinds and overrides the schema names", () => {
  const narrow = ruleBuilderFields(SCHEMA, null);
  assert.deepEqual(
    narrow.map((field) => [field.path, field.label, field.kind]),
    [
      ["input.item.flag", "flag", "bool"],
      ["input.item.size", "size", "number"],
      ["input.item.name", "name", "text"],
      ["input.item.seen_at", "seen_at", "time"],
      ["input.item.labels", "labels", "list"],
      ["input.item.color", "color", "choice"],
    ],
  );
  const wide = ruleBuilderFields(SCHEMA, "WIDE").map((field) => [
    field.path,
    field.label,
  ]);
  assert.deepEqual(wide.slice(-2), [
    ["input.item.wide_only", "wide_only"],
    ["input.extra.count", "extra.count"],
  ]);
});

// ── Refusals ──────────────────────────────────────────────────────────

test("a model with no condition or no output is refused", () => {
  assert.deepEqual(codes(model({ conditions: [] })), ["BUILDER_NO_CONDITIONS"]);
  assert.deepEqual(codes(model({ outputs: {} })), ["BUILDER_NO_OUTPUTS"]);
  assert.deepEqual(codes(emptyRuleBuilderModel(SCHEMA)), [
    "BUILDER_FACT_REQUIRED",
    "BUILDER_NO_OUTPUTS",
  ]);
});

test("unfinished, unknown or out-of-variant conditions are refused with their row", () => {
  const result = problems(
    model({
      conditions: [
        { field: "input.item.flag", operator: "IS_TRUE", value: "" },
        { field: "input.item.hidden", operator: "EQUALS", value: "x" },
        { field: "input.item.parts", operator: "CONTAINS", value: "x" },
        { field: "input.extra.count", operator: "GREATER_THAN", value: "1" },
        { field: "input.item.flag", operator: "", value: "" },
        { field: "input.item.flag", operator: "EQUALS", value: "x" },
        { field: "", operator: "", value: "" },
      ],
    }),
  );
  assert.deepEqual(result, [
    { code: "BUILDER_FACT_UNKNOWN", conditionIndex: 1 },
    { code: "BUILDER_FACT_UNKNOWN", conditionIndex: 2 },
    { code: "BUILDER_FACT_UNKNOWN", conditionIndex: 3 },
    { code: "BUILDER_OPERATOR_INVALID", conditionIndex: 4 },
    { code: "BUILDER_OPERATOR_INVALID", conditionIndex: 5 },
    { code: "BUILDER_FACT_REQUIRED", conditionIndex: 6 },
  ]);
  assert.equal(
    generateRuleSource(
      SCHEMA,
      model({
        conditions: [
          { field: "input.extra.count", operator: "GREATER_THAN", value: "1" },
        ],
      }),
      "WIDE",
    ).ok,
    true,
  );
});

test("condition values that cannot be written are refused", () => {
  const condition = (field: string, operator: string, value: string) =>
    codes(model({ conditions: [{ field, operator, value }] }));

  for (const value of ["", " "])
    assert.deepEqual(condition("input.item.size", "GREATER_THAN", value), [
      "BUILDER_VALUE_REQUIRED",
    ]);
  for (const value of [
    "1e3",
    "0x10",
    "01",
    "1.",
    ".5",
    "1.1234567",
    "Infinity",
    " 1",
    "9007199254740993",
  ])
    assert.deepEqual(
      condition("input.item.size", "GREATER_THAN", value),
      ["BUILDER_VALUE_NOT_NUMBER"],
      value,
    );
  for (const value of ["-1", "1.5", "100001", "07"])
    assert.deepEqual(
      condition("input.item.seen_at", "MORE_THAN_DAYS_AGO", value),
      ["BUILDER_VALUE_NOT_DAYS"],
      value,
    );
  for (const value of [
    "line\nbreak",
    "tab\there",
    "nul\u0000",
    "sep ",
    "c1\u0085",
    "lone\ud800",
    "lone\udc00x",
  ])
    assert.deepEqual(
      condition("input.item.name", "EQUALS", value),
      ["BUILDER_VALUE_UNREPRESENTABLE"],
      JSON.stringify(value),
    );
  assert.deepEqual(condition("input.item.color", "EQUALS", ""), [
    "BUILDER_VALUE_REQUIRED",
  ]);
  assert.deepEqual(condition("input.item.color", "EQUALS", "green"), [
    "BUILDER_VALUE_NOT_OFFERED",
  ]);
  assert.equal(
    generateRuleSource(
      SCHEMA,
      model({
        conditions: [
          { field: "input.item.color", operator: "EQUALS", value: "red" },
        ],
      }),
      null,
    ).ok,
    true,
  );
});

test("output values are checked against their constraints", () => {
  const output = (outputs: RuleBuilderModel["outputs"]) =>
    problems(model({ outputs }));

  assert.deepEqual(output({ NOTE: " " }), [
    { code: "BUILDER_VALUE_REQUIRED", output: "NOTE" },
  ]);
  assert.deepEqual(output({ NOTE: "two\nlines" }), [
    { code: "BUILDER_VALUE_UNREPRESENTABLE", output: "NOTE" },
  ]);
  assert.deepEqual(output({ NOTE: "far too long" }), [
    { code: "BUILDER_VALUE_TOO_LONG", output: "NOTE" },
  ]);
  assert.deepEqual(output({ NOTE: "ABC" }), [
    { code: "BUILDER_VALUE_INVALID", output: "NOTE" },
  ]);
  // The byte limit counts UTF-8: five letters, ten bytes.
  assert.deepEqual(output({ NOTE: "ééééé" }), [
    { code: "BUILDER_VALUE_TOO_LONG", output: "NOTE" },
  ]);
  assert.deepEqual(output({ PICK: "" }), [
    { code: "BUILDER_VALUE_REQUIRED", output: "PICK" },
  ]);
  assert.deepEqual(output({ WEIGHT: "1.5" }), [
    { code: "BUILDER_VALUE_NOT_WHOLE", output: "WEIGHT" },
  ]);
  assert.deepEqual(output({ WEIGHT: "11" }), [
    { code: "BUILDER_VALUE_OUT_OF_RANGE", output: "WEIGHT" },
  ]);
  assert.deepEqual(output({ WEIGHT: "-11" }), [
    { code: "BUILDER_VALUE_OUT_OF_RANGE", output: "WEIGHT" },
  ]);
  assert.deepEqual(output({ LABELS: [] }), [
    { code: "BUILDER_VALUE_REQUIRED", output: "LABELS" },
  ]);
  assert.deepEqual(output({ LABELS: ["a", "b", "c"] }), [
    { code: "BUILDER_VALUE_TOO_MANY", output: "LABELS" },
  ]);
  assert.deepEqual(output({ EACH: ["a", "a"] }), [
    { code: "BUILDER_VALUE_DUPLICATE", output: "EACH" },
  ]);
  assert.deepEqual(output({ EACH: ["a", ""] }), [
    { code: "BUILDER_VALUE_REQUIRED", output: "EACH" },
  ]);
  assert.deepEqual(output({ ENTRY: { code: "Bad", points: "1" } }), [
    { code: "BUILDER_VALUE_INVALID", output: "ENTRY", part: "code" },
  ]);
  assert.deepEqual(output({ ENTRY: { code: "ok", points: "101" } }), [
    { code: "BUILDER_VALUE_OUT_OF_RANGE", output: "ENTRY", part: "points" },
  ]);
  assert.deepEqual(output({ ENTRY: { code: "ok", points: "" } }), [
    { code: "BUILDER_VALUE_REQUIRED", output: "ENTRY", part: "points" },
  ]);
  assert.equal(
    source(model({ outputs: { WEIGHT: "-10", NOTE: "ab cd" } })).includes(
      "weight := -10",
    ),
    true,
  );
});

test("conflicts are read both ways and every chosen output is reported", () => {
  assert.deepEqual(problems(model({ outputs: { KEEP: true, DROP: true } })), [
    { code: "BUILDER_OUTPUTS_CONFLICT", output: "KEEP", outputs: ["DROP"] },
    { code: "BUILDER_OUTPUTS_CONFLICT", output: "DROP", outputs: ["KEEP"] },
  ]);
  assert.deepEqual(
    ruleBuilderConflicts(SCHEMA, "DROP", { KEEP: true, NOTE: "a" }),
    ["KEEP"],
  );
  assert.deepEqual(
    ruleBuilderConflicts(SCHEMA, "NOTE", { KEEP: true, DROP: true }),
    [],
  );
});

test("at_most_one and exactly_one groups conflict within the group", () => {
  assert.deepEqual(codes(model({ outputs: { PICK: "a", WEIGHT: "1" } })), [
    "BUILDER_OUTPUTS_CONFLICT",
    "BUILDER_OUTPUTS_CONFLICT",
  ]);
  const exactly: RuleBuilderSchema = {
    ...SCHEMA,
    outputRules: [{ rule: "exactly_one", of: ["KEEP", "WEIGHT"] }],
  };
  const exactlyCodes = (outputs: RuleBuilderModel["outputs"]) => {
    const result = generateRuleSource(exactly, model({ outputs }), null);
    return result.ok ? [] : result.problems;
  };
  assert.deepEqual(exactlyCodes({ NOTE: "a" }), [
    { code: "BUILDER_OUTPUT_REQUIRED", outputs: ["KEEP", "WEIGHT"] },
  ]);
  assert.deepEqual(
    exactlyCodes({ KEEP: true, WEIGHT: "1" }).map((problem) => problem.code),
    ["BUILDER_OUTPUTS_CONFLICT", "BUILDER_OUTPUTS_CONFLICT"],
  );
  assert.deepEqual(exactlyCodes({ WEIGHT: "1" }), []);
});

test("an output limited to some variants is refused in others", () => {
  assert.deepEqual(problems(model({ outputs: { WIDEN: true } }), null), [
    { code: "BUILDER_OUTPUT_NOT_IN_SCOPE", output: "WIDEN" },
  ]);
  assert.deepEqual(problems(model({ outputs: { WIDEN: true } }), "NARROW"), [
    { code: "BUILDER_OUTPUT_NOT_IN_SCOPE", output: "WIDEN" },
  ]);
  assert.equal(
    source(model({ outputs: { WIDEN: true } }), "WIDE").endsWith(
      "widen if matches\n",
    ),
    true,
  );
});

test("an output that needs another is refused until both are chosen", () => {
  assert.deepEqual(problems(model({ outputs: { AFTER: true } })), [
    { code: "BUILDER_OUTPUT_NEEDS_OTHER", output: "AFTER", outputs: ["NOTE"] },
  ]);
  assert.deepEqual(
    ruleBuilderMissingRequirements(SCHEMA, "AFTER", { NOTE: "a" }),
    [],
  );
  assert.equal(
    source(model({ outputs: { AFTER: true, NOTE: "a" } })).endsWith(
      "after if matches\n",
    ),
    true,
  );
});

test("an output the schema does not know is refused", () => {
  assert.deepEqual(problems(model({ outputs: { KEEP: true, LATER: true } })), [
    { code: "BUILDER_OUTPUT_UNKNOWN", output: "LATER" },
  ]);
});

test("option sources are only those the chosen outputs need", () => {
  assert.deepEqual(ruleBuilderOptionSources(SCHEMA, null), []);
  assert.deepEqual(ruleBuilderOptionSources(SCHEMA, model()), []);
  assert.deepEqual(
    ruleBuilderOptionSources(
      SCHEMA,
      model({ outputs: { EACH: [], PICK: "", LABELS: [] } }),
    ),
    ["picks", "labels"],
  );
});

test("option sources include the catalogs of the conditions' fields", () => {
  const schema: RuleBuilderSchema = {
    ...SCHEMA,
    conditionSources: [
      {
        section: "input.item",
        label: "field",
        fields: {
          owner_id: { options: "users" },
          watcher_ids: { options: "users" },
          list_ids: { options: "lists" },
        },
      },
    ],
  };
  const condition = (field: string) => ({
    field,
    operator: "EQUALS",
    value: "",
  });
  assert.deepEqual(
    ruleBuilderOptionSources(
      schema,
      model({
        conditions: [
          condition("input.item.owner_id"),
          condition("input.item.flag"),
          condition(""),
        ],
        actions: [
          {
            output: "PICK",
            join: "ALL",
            conditions: [
              condition("input.item.watcher_ids"),
              condition("input.item.list_ids"),
            ],
          },
        ],
        outputs: { PICK: "" },
      }),
    ),
    ["users", "lists", "picks"],
  );
});

test("problem codes are distinct and builder-prefixed", () => {
  assert.equal(
    new Set(RULE_BUILDER_PROBLEM_CODES).size,
    RULE_BUILDER_PROBLEM_CODES.length,
  );
  for (const code of RULE_BUILDER_PROBLEM_CODES)
    assert.match(code, /^BUILDER_[A-Z_]+$/);
});

// ── Documents ─────────────────────────────────────────────────────────

test("a stored document parses back to the same model", () => {
  const candidate = model({
    join: "ANY",
    outputs: {
      KEEP: true,
      NOTE: "a",
      LABELS: ["x"],
      ENTRY: { code: "a", points: "1" },
    },
  });
  assert.deepEqual(
    parseRuleBuilderModel(SCHEMA, JSON.parse(JSON.stringify(candidate))),
    candidate,
  );
  assert.equal(isBlankRuleBuilderModel(emptyRuleBuilderModel(SCHEMA)), true);
  assert.equal(isBlankRuleBuilderModel(candidate), false);
});

test("documents this build does not recognise do not parse", () => {
  const good = model();
  const bad: unknown[] = [
    null,
    "model",
    [],
    { ...good, version: 9 },
    { ...good, version: 1 },
    { ...good, family: "other" },
    { ...good, join: "SOME" },
    { ...good, conditions: "none" },
    {
      ...good,
      conditions: [
        { field: "input.item.flag", operator: "MATCHES", value: "" },
      ],
    },
    { ...good, conditions: [{ field: 1, operator: "IS_TRUE", value: "" }] },
    { ...good, outputs: { KEEP: "yes" } },
    { ...good, outputs: { NOTE: 3 } },
    { ...good, outputs: { LABELS: "x" } },
    { ...good, outputs: { LABELS: [1] } },
    { ...good, outputs: { ENTRY: { code: "a" } } },
    { ...good, outputs: { ENTRY: { code: "a", points: "1", extra: "" } } },
    { ...good, outputs: { LATER: true } },
    { ...good, outputs: null },
  ];
  for (const document of bad)
    assert.equal(
      parseRuleBuilderModel(SCHEMA, document),
      null,
      JSON.stringify(document),
    );
});

test("a document from an earlier version does not attach", () => {
  const earlier = {
    version: 1,
    join: "ALL",
    conditions: [{ name: "flag", operator: "IS_TRUE", value: "" }],
    picked: { KEEP: true },
  };
  assert.equal(parseRuleBuilderModel(SCHEMA, earlier), null);
  assert.equal(
    parseRuleBuilderModel(SCHEMA, { ...earlier, family: "sample" }),
    null,
  );
});

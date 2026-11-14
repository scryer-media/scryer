import assert from "node:assert/strict";
import test from "node:test";

import builderFixtures from "../contracts/maintenance-builder-fixtures.json" with { type: "json" };
import maintenanceInputContract from "../contracts/maintenance-input-contract.json" with { type: "json" };
import en from "../i18n/locales/en.ts";
import ru from "../i18n/locales/ru.ts";
import type {
  MaintenanceRuleScope,
  MaintenanceRuleSetDetail,
} from "../types/maintenance-rule-sets.ts";
import {
  MAINTENANCE_BUILDER_OPERATOR_KEYS,
  MAINTENANCE_BUILDER_PROBLEM_CODES,
  MAINTENANCE_BUILDER_PROBLEM_KEYS,
  type MaintenanceBuilderModel,
  type MaintenanceBuilderOperator,
  type MaintenanceBuilderProblemCode,
  emptyMaintenanceBuilderModel,
  generateMaintenanceRuleSource,
  isMaintenanceBuilderSource,
  maintenanceBuilderConflicts,
  maintenanceBuilderFacts,
  maintenanceBuilderOperators,
  parseMaintenanceBuilderModel,
} from "./maintenance-rule-builder.ts";
import {
  MAINTENANCE_STARTER_SOURCE,
  createMaintenanceRuleSetInput,
  initialMaintenanceRuleDraft,
  maintenanceBuilderModelToSave,
  maintenanceRuleDraftFromDetail,
  newMaintenanceRuleBuilderDraft,
  updateMaintenanceRuleMatcherInput,
  withMaintenanceAdvancedEditor,
  withMaintenanceBuilderModel,
  withMaintenanceRuleSourceEdit,
} from "./maintenance-rule-sets.ts";

type Fixture = {
  id: string;
  scope: MaintenanceRuleScope;
  model: MaintenanceBuilderModel;
  source: string;
};

const fixtures = builderFixtures as Fixture[];

function fixture(id: string): Fixture {
  const found = fixtures.find((entry) => entry.id === id);
  assert.ok(found, id);
  return found;
}

function model(
  overrides: Partial<MaintenanceBuilderModel> = {},
): MaintenanceBuilderModel {
  return {
    version: 1,
    join: "ALL",
    conditions: [{ fact: "monitored", operator: "IS_TRUE", value: "" }],
    actions: { UNMONITOR: true },
    ...overrides,
  };
}

function problems(
  candidate: MaintenanceBuilderModel,
  scope: MaintenanceRuleScope = "TITLE",
): MaintenanceBuilderProblemCode[] {
  const result = generateMaintenanceRuleSource(candidate, scope);
  assert.equal(result.ok, false);
  return result.ok ? [] : result.problems.map((problem) => problem.code);
}

function detailWith(
  regoSource: string,
  builderModel: unknown,
  subjectKind: MaintenanceRuleScope = "TITLE",
): MaintenanceRuleSetDetail {
  const declaredActions = {
    heads: [],
    destructive: false,
    qualityProfiles: [],
    addTags: [],
    removeTags: [],
    libraries: [],
    roots: [],
    notifications: [],
  };
  return {
    ruleSet: {
      id: "rule-sample",
      name: "Sample rule",
      description: null,
      state: "DISABLED",
      enabledArming: "REVERSIBLE",
      editedSinceEnabled: false,
      systemOwned: false,
      libraryIds: [],
      subjectKind,
      currentRevisionNumber: 1,
      graceDays: 0,
      declaredActions,
      createdAt: "2026-01-01T00:00:00Z",
      updatedAt: "2026-01-01T00:00:00Z",
    },
    revision: {
      id: "rev-sample",
      ruleSetId: "rule-sample",
      revisionNumber: 1,
      regoSource,
      graceDays: 0,
      storageRootId: null,
      matcherContentHash: "blake3:sample",
      createdBy: "operator",
      createdAt: "2026-01-01T00:00:00Z",
    },
    declaredActions,
    builderModel,
  } as MaintenanceRuleSetDetail;
}

const PREAMBLE = "package rules\nimport rego.v1\n\n";

// ── Pinned sources ────────────────────────────────────────────────────

test("every fixture model generates exactly its pinned source", () => {
  assert.ok(fixtures.length > 0);
  for (const entry of fixtures) {
    const result = generateMaintenanceRuleSource(entry.model, entry.scope);
    assert.deepEqual(result, { ok: true, source: entry.source }, entry.id);
  }
});

test("ALL joins the conditions in one body and ANY writes one body per condition", () => {
  assert.equal(
    fixture("all-bool-and-days").source,
    `${PREAMBLE}rule_applies if {\n\tinput.facts.monitored\n\tdays_since(input.facts.added_at) > 90\n\tnot input.facts.has_file\n}\n\nunmonitor if rule_applies\n\ndelete_files if rule_applies\n\nnotify := "Removed an old title" if rule_applies\n`,
  );
  assert.equal(
    fixture("title-move-to-root").source,
    `${PREAMBLE}rule_applies if {\n\tinput.facts.watched_by_anyone\n}\n\nrule_applies if {\n\tinput.facts.storage_root_id == "sample-root"\n}\n\nmove_to_root := "/data/sample root" if rule_applies\n\nunmonitor if rule_applies\n`,
  );
});

test("string values are written as escaped Rego literals", () => {
  const source = fixture("escaping").source;
  assert.ok(
    source.includes(
      '\tinput.facts.added_by_username == "sample \\"quoted\\" \\\\ user"\n',
    ),
  );
  assert.ok(source.includes('\t"Ünïcødé 標籤 😀" in input.facts.tags\n'));
  assert.ok(
    source.includes('add_tags := ["tag \\"one\\"", "tag\\\\two"] if rule_applies'),
  );
  assert.ok(
    source.includes('notify := "Say \\"hi\\" to C:\\\\sample\\\\path" if rule_applies'),
  );
});

test("heads come out in execution order whatever order the model lists them", () => {
  const entry = fixture("every-title-action-in-order");
  const heads = entry.source
    .split("\n")
    .filter((line) => line.endsWith(" if rule_applies"))
    .map((line) => line.split(/[ :]/)[0]);
  assert.deepEqual(heads, [
    "add_tags",
    "remove_tags",
    "set_quality_profile",
    "search",
    "fetch_subtitles",
    "unmonitor",
    "delete_files",
    "notify",
  ]);
  const reversed = Object.fromEntries(
    Object.entries(entry.model.actions).reverse(),
  );
  assert.deepEqual(
    generateMaintenanceRuleSource(
      { ...entry.model, actions: reversed },
      entry.scope,
    ),
    { ok: true, source: entry.source },
  );
});

test("the fixtures cover every operator, every action and both joins", () => {
  const operators = new Set(
    fixtures.flatMap((entry) =>
      entry.model.conditions.map((condition) => condition.operator),
    ),
  );
  assert.deepEqual(
    [...operators].sort(),
    Object.keys(MAINTENANCE_BUILDER_OPERATOR_KEYS).sort(),
  );
  const heads = new Set(
    fixtures.flatMap((entry) => Object.keys(entry.model.actions)),
  );
  assert.deepEqual(
    [...heads].sort(),
    [
      "ADD_TAGS",
      "DELETE_FILES",
      "DELETE_TITLE",
      "FETCH_SUBTITLES",
      "MONITOR",
      "MOVE_TO_LIBRARY",
      "MOVE_TO_ROOT",
      "NOTIFY",
      "REMOVE_TAGS",
      "SEARCH",
      "SET_QUALITY_PROFILE",
      "UNMONITOR",
    ],
  );
  assert.deepEqual(
    [...new Set(fixtures.map((entry) => entry.model.join))].sort(),
    ["ALL", "ANY"],
  );
});

test("generation is deterministic and does not change the model", () => {
  for (const entry of fixtures) {
    const before = JSON.stringify(entry.model);
    const first = generateMaintenanceRuleSource(entry.model, entry.scope);
    const second = generateMaintenanceRuleSource(
      structuredClone(entry.model),
      entry.scope,
    );
    assert.deepEqual(first, second, entry.id);
    assert.equal(JSON.stringify(entry.model), before, entry.id);
  }
});

test("generated sources keep the shape the editor and server expect", () => {
  for (const entry of fixtures) {
    assert.ok(entry.source.startsWith(PREAMBLE), entry.id);
    assert.ok(entry.source.endsWith("}\n") || entry.source.endsWith("rule_applies\n"));
    assert.equal(entry.source.endsWith("\n\n"), false, entry.id);
    assert.equal(/^ +\S/m.test(entry.source), false, entry.id);
  }
});

// ── Refusals ──────────────────────────────────────────────────────────

test("a model with no condition or no action is refused", () => {
  assert.deepEqual(problems(model({ conditions: [] })), [
    "BUILDER_NO_CONDITIONS",
  ]);
  assert.deepEqual(problems(model({ actions: {} })), ["BUILDER_NO_ACTIONS"]);
  assert.deepEqual(problems(emptyMaintenanceBuilderModel()), [
    "BUILDER_FACT_REQUIRED",
    "BUILDER_NO_ACTIONS",
  ]);
});

test("unfinished or unknown conditions are refused with their row", () => {
  const result = generateMaintenanceRuleSource(
    model({
      conditions: [
        { fact: "monitored", operator: "IS_TRUE", value: "" },
        { fact: "not_a_fact", operator: "IS_TRUE", value: "" },
        { fact: "monitored", operator: "", value: "" },
        { fact: "monitored", operator: "GREATER_THAN", value: "1" },
        { fact: "files", operator: "IS_EMPTY", value: "" },
      ],
    }),
    "TITLE",
  );
  assert.deepEqual(result, {
    ok: false,
    problems: [
      { code: "BUILDER_FACT_UNKNOWN", conditionIndex: 1 },
      { code: "BUILDER_OPERATOR_INVALID", conditionIndex: 2 },
      { code: "BUILDER_OPERATOR_INVALID", conditionIndex: 3 },
      { code: "BUILDER_FACT_UNKNOWN", conditionIndex: 4 },
    ],
  });
});

test("values that cannot be written are refused rather than written", () => {
  const condition = (
    fact: string,
    operator: MaintenanceBuilderOperator,
    value: string,
  ) => problems(model({ conditions: [{ fact, operator, value }] }));

  for (const value of ["", "  "])
    assert.deepEqual(condition("file_count", "EQUALS", value), [
      "BUILDER_VALUE_REQUIRED",
    ]);
  for (const value of ["1e3", "0x10", "01", "1.", ".5", "1.1234567", "Infinity", " 1", "9007199254740993"])
    assert.deepEqual(condition("file_count", "EQUALS", value), [
      "BUILDER_VALUE_NOT_NUMBER",
    ], value);
  for (const value of ["-1", "1.5", "100001", "07"])
    assert.deepEqual(condition("added_at", "MORE_THAN_DAYS_AGO", value), [
      "BUILDER_VALUE_NOT_DAYS",
    ], value);
  for (const value of ["line\nbreak", "tab\there", "nul\u0000", "sep\u2028", "c1\u0085", "lone\ud800", "lone\udc00x"])
    assert.deepEqual(condition("request_origin", "EQUALS", value), [
      "BUILDER_VALUE_UNREPRESENTABLE",
    ], JSON.stringify(value));
  assert.deepEqual(condition("tags", "CONTAINS", ""), ["BUILDER_VALUE_REQUIRED"]);
});

test("action values are checked the same way", () => {
  assert.deepEqual(problems(model({ actions: { NOTIFY: " " } })), [
    "BUILDER_VALUE_REQUIRED",
  ]);
  assert.deepEqual(problems(model({ actions: { NOTIFY: "two\nlines" } })), [
    "BUILDER_VALUE_UNREPRESENTABLE",
  ]);
  assert.deepEqual(problems(model({ actions: { ADD_TAGS: [] } })), [
    "BUILDER_VALUE_REQUIRED",
  ]);
  assert.deepEqual(
    problems(model({ actions: { REMOVE_TAGS: ["sample", "sample"] } })),
    ["BUILDER_VALUE_DUPLICATE"],
  );
  assert.deepEqual(
    problems(model({ actions: { SET_QUALITY_PROFILE: "" } })),
    ["BUILDER_VALUE_REQUIRED"],
  );
});

test("title-only actions are refused outside a title rule", () => {
  for (const actions of [
    { DELETE_TITLE: true as const },
    { MOVE_TO_LIBRARY: "Sample Library" },
    { MOVE_TO_ROOT: "/data/sample" },
  ]) {
    assert.deepEqual(problems(model({ actions }), "EPISODE"), [
      "BUILDER_ACTION_NOT_IN_SCOPE",
    ]);
    assert.equal(
      generateMaintenanceRuleSource(model({ actions }), "TITLE").ok,
      true,
    );
  }
});

test("actions the server never applies together are refused and reported", () => {
  const conflicting: MaintenanceBuilderModel["actions"][] = [
    { MONITOR: true, UNMONITOR: true },
    { MONITOR: true, DELETE_FILES: true },
    { MONITOR: true, DELETE_TITLE: true },
    { MOVE_TO_LIBRARY: "Sample Library", MOVE_TO_ROOT: "/data/sample" },
    { MOVE_TO_LIBRARY: "Sample Library", DELETE_FILES: true },
    { MOVE_TO_ROOT: "/data/sample", DELETE_TITLE: true },
  ];
  for (const actions of conflicting) {
    const codes = problems(model({ actions }));
    assert.deepEqual(codes, ["BUILDER_ACTIONS_CONFLICT", "BUILDER_ACTIONS_CONFLICT"]);
  }
  assert.deepEqual(
    maintenanceBuilderConflicts("MONITOR", {
      UNMONITOR: true,
      DELETE_FILES: true,
      SEARCH: true,
    }),
    ["UNMONITOR", "DELETE_FILES"],
  );
  assert.deepEqual(
    maintenanceBuilderConflicts("SEARCH", { UNMONITOR: true }),
    [],
  );
});

// ── Facts and operators ───────────────────────────────────────────────

test("facts come from the input contract and leave out object lists", () => {
  const section = maintenanceInputContract.sections.find(
    (entry) => entry.path === "input.facts",
  );
  assert.ok(section);
  const offered = maintenanceBuilderFacts("TITLE");
  const offeredNames = new Set(offered.map((fact) => fact.name));
  for (const field of section.fields) {
    const objectList = field.type.endsWith("[]") && field.type !== "string[]";
    assert.equal(offeredNames.has(field.field), !objectList, field.field);
  }
  for (const fact of offered) {
    assert.ok(maintenanceBuilderOperators(fact.kind).length > 0, fact.name);
    assert.ok(fact.descKey in en, fact.descKey);
  }
  assert.equal(
    offered.find((fact) => fact.name === "added_at")?.kind,
    "timestamp",
  );
  assert.equal(offered.find((fact) => fact.name === "tags")?.kind, "string_list");
});

test("every operator and problem code has a translated label", () => {
  const enStrings = en as Record<string, string>;
  const ruStrings = ru as Record<string, string>;
  for (const code of MAINTENANCE_BUILDER_PROBLEM_CODES) {
    const key = MAINTENANCE_BUILDER_PROBLEM_KEYS[code];
    assert.ok(enStrings[key], key);
    assert.ok(ruStrings[key], key);
  }
  for (const key of Object.values(MAINTENANCE_BUILDER_OPERATOR_KEYS)) {
    assert.ok(enStrings[key], key);
    assert.ok(ruStrings[key], key);
  }
  assert.equal(
    new Set(MAINTENANCE_BUILDER_PROBLEM_CODES).size,
    MAINTENANCE_BUILDER_PROBLEM_CODES.length,
  );
  for (const code of MAINTENANCE_BUILDER_PROBLEM_CODES)
    assert.match(code, /^BUILDER_[A-Z_]+$/);
});

// ── Model documents ───────────────────────────────────────────────────

test("a stored model document parses back to the same model", () => {
  for (const entry of fixtures) {
    assert.deepEqual(
      parseMaintenanceBuilderModel(JSON.parse(JSON.stringify(entry.model))),
      entry.model,
      entry.id,
    );
  }
});

test("documents this build does not recognise do not parse", () => {
  const good = fixture("all-bool-and-days").model;
  const bad: unknown[] = [
    null,
    "model",
    [],
    { ...good, version: 2 },
    { ...good, join: "SOME" },
    { ...good, conditions: "none" },
    { ...good, conditions: [{ fact: "monitored", operator: "MATCHES", value: "" }] },
    { ...good, conditions: [{ fact: 1, operator: "IS_TRUE", value: "" }] },
    { ...good, actions: { UNMONITOR: "yes" } },
    { ...good, actions: { NOTIFY: 3 } },
    { ...good, actions: { ADD_TAGS: "sample" } },
    { ...good, actions: { ADD_TAGS: [1] } },
    { ...good, actions: { REQUEST_REVIEW: true } },
    { ...good, actions: null },
  ];
  for (const document of bad)
    assert.equal(
      parseMaintenanceBuilderModel(document),
      null,
      JSON.stringify(document),
    );
});

// ── Drafts, saving, detaching ─────────────────────────────────────────

test("a new rule starts in the builder with nothing to save yet", () => {
  const draft = newMaintenanceRuleBuilderDraft();
  assert.deepEqual(draft.builderModel, emptyMaintenanceBuilderModel());
  assert.equal(draft.regoSource, "");
  assert.equal(initialMaintenanceRuleDraft().builderModel, null);
});

test("a builder change writes the generated source into the draft", () => {
  const entry = fixture("all-bool-and-days");
  const draft = withMaintenanceBuilderModel(
    newMaintenanceRuleBuilderDraft(),
    entry.model,
  );
  assert.equal(draft.regoSource, entry.source);
  assert.deepEqual(maintenanceBuilderModelToSave(draft), entry.model);
  assert.deepEqual(createMaintenanceRuleSetInput(draft).builderModel, entry.model);
  assert.deepEqual(
    updateMaintenanceRuleMatcherInput("rule-sample", draft).builderModel,
    entry.model,
  );

  const unfinished = withMaintenanceBuilderModel(draft, {
    ...entry.model,
    actions: {},
  });
  assert.equal(unfinished.regoSource, "");
  assert.equal(maintenanceBuilderModelToSave(unfinished), null);
});

test("a hand edit detaches the rule once and for good", () => {
  const entry = fixture("any-string-lists");
  const attached = withMaintenanceBuilderModel(
    newMaintenanceRuleBuilderDraft(),
    entry.model,
  );

  const unchanged = withMaintenanceRuleSourceEdit(attached, entry.source);
  assert.equal(unchanged.detached, false);
  assert.deepEqual(unchanged.draft.builderModel, entry.model);

  const edited = withMaintenanceRuleSourceEdit(
    attached,
    `${entry.source}\nsearch if rule_applies\n`,
  );
  assert.equal(edited.detached, true);
  assert.equal(edited.draft.builderModel, null);
  assert.equal(maintenanceBuilderModelToSave(edited.draft), null);
  assert.equal(createMaintenanceRuleSetInput(edited.draft).builderModel, null);

  // Typing the generated source back does not reattach it.
  const retyped = withMaintenanceRuleSourceEdit(edited.draft, entry.source);
  assert.equal(retyped.detached, false);
  assert.equal(retyped.draft.builderModel, null);
});

test("switching to Advanced keeps the generated source, or starts a blank rule from the starter", () => {
  const entry = fixture("title-move-to-library");
  const attached = withMaintenanceBuilderModel(
    newMaintenanceRuleBuilderDraft(),
    entry.model,
  );
  assert.equal(withMaintenanceAdvancedEditor(attached), attached);

  const blank = withMaintenanceAdvancedEditor(newMaintenanceRuleBuilderDraft());
  assert.equal(blank.regoSource, MAINTENANCE_STARTER_SOURCE);
  assert.equal(blank.builderModel, null);
});

test("a stored model opens in the builder only while it still regenerates the stored source", () => {
  const entry = fixture("all-strings-and-timestamps");

  const exact = maintenanceRuleDraftFromDetail(
    detailWith(entry.source, entry.model),
  );
  assert.deepEqual(exact.builderModel, entry.model);

  // The API hands stored sources back without the package and import lines.
  const stripped = entry.source.slice(PREAMBLE.length);
  const served = maintenanceRuleDraftFromDetail(detailWith(stripped, entry.model));
  assert.deepEqual(served.builderModel, entry.model);
  assert.equal(served.regoSource, stripped);
  assert.deepEqual(maintenanceBuilderModelToSave(served), entry.model);

  const handEdited = maintenanceRuleDraftFromDetail(
    detailWith(`${entry.source}\n# note\n`, entry.model),
  );
  assert.equal(handEdited.builderModel, null);

  const stale = maintenanceRuleDraftFromDetail(
    detailWith(fixture("title-move-to-library").source, entry.model),
  );
  assert.equal(stale.builderModel, null);
  assert.equal(stale.regoSource, fixture("title-move-to-library").source);

  const otherScope = maintenanceRuleDraftFromDetail(
    detailWith(
      fixture("title-delete-title").source,
      fixture("title-delete-title").model,
      "EPISODE",
    ),
  );
  assert.equal(otherScope.builderModel, null);

  for (const builderModel of [null, undefined, { version: 9 }]) {
    assert.equal(
      maintenanceRuleDraftFromDetail(detailWith(entry.source, builderModel))
        .builderModel,
      null,
    );
  }
});

test("only the generated source, as written or as served, counts as the builder's", () => {
  const entry = fixture("episode-scope");
  assert.equal(isMaintenanceBuilderSource(entry.model, "EPISODE", entry.source), true);
  assert.equal(
    isMaintenanceBuilderSource(
      entry.model,
      "EPISODE",
      entry.source.slice(PREAMBLE.length),
    ),
    true,
  );
  assert.equal(
    isMaintenanceBuilderSource(entry.model, "EPISODE", `${entry.source}\n`),
    false,
  );
  assert.equal(
    isMaintenanceBuilderSource(entry.model, "EPISODE", entry.source.replace(/\t/g, "  ")),
    false,
  );
});

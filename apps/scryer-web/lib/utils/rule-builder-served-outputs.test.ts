import assert from "node:assert/strict";
import test from "node:test";

import en from "../i18n/locales/en.ts";
import { regoHeadAssists } from "./rego-assistance.ts";
import {
  RULE_BUILDER_CATALOGS,
  ruleBuilderCatalogMisses,
} from "./rule-builder-catalog-presentation.ts";
import { LIFECYCLE_SERVED_PAYLOAD } from "./rule-builder-contract-fixture.ts";
import { LIFECYCLE_RULE_BUILDER_BASE } from "./lifecycle-rule-builder-schema.ts";
import {
  generateRuleSource,
  isRuleBuilderOutputInVariant,
  ruleBuilderConflicts,
  ruleBuilderOptionSources,
  ruleBuilderOutputHelp,
  ruleBuilderOutputLabel,
  ruleBuilderOutputsFromPayload,
  ruleBuilderSchemaFromPayload,
  type RuleBuilderModel,
  type RuleBuilderOutputDescriptor,
  type RuleBuilderSchemaPayload,
} from "./rule-builder.ts";
import {
  isRuleOutputDestructive,
  ruleOutputHeadHelp,
  ruleOutputHeadLabel,
  ruleOutputSnippet,
  ruleOutputSyntax,
  sortByRuleOutputOrder,
  templateRuleOutputs,
} from "./rule-output-vocabulary.ts";

/// Outputs this build has never seen, as a newer server would serve them: a
/// flag for titles only that never goes with unmonitoring, a list from a
/// catalog the web cannot load, and a value kind the web does not know.
const ARCHIVE: RuleBuilderOutputDescriptor = {
  id: "ARCHIVE",
  head: "archive",
  labelKey: "ruleOutput.lifecycle.archive",
  helpKey: "ruleOutput.lifecycle.archive.help",
  valueKind: "FLAG",
  template: "{head} if {helper}",
  perItem: false,
  destructive: false,
  variants: ["TITLE"],
  conflictsWith: ["UNMONITOR"],
};

const SET_LABELS: RuleBuilderOutputDescriptor = {
  id: "SET_LABELS",
  head: "set_labels",
  labelKey: "ruleOutput.lifecycle.set_labels",
  helpKey: null,
  valueKind: "TEXT_LIST",
  options: "shelfLabels",
  maxItems: 2,
  maxBytes: 8,
  pattern: "[a-z]+",
  template: "{head} := [{items}] if {helper}",
  perItem: false,
  destructive: false,
  variants: null,
  conflictsWith: [],
};

const HOLD_FOR: RuleBuilderOutputDescriptor = {
  id: "HOLD_FOR",
  head: "hold_for",
  labelKey: "ruleOutput.lifecycle.hold_for",
  valueKind: "DURATION",
  template: "{head} := {value} if {helper}",
  perItem: false,
  destructive: false,
  conflictsWith: [],
};

/// The server lists every output under the subject it acts on, so the unseen
/// title outputs come listed under the title.
const PAYLOAD: RuleBuilderSchemaPayload = {
  ...LIFECYCLE_SERVED_PAYLOAD,
  outputs: [ARCHIVE, ...LIFECYCLE_SERVED_PAYLOAD.outputs, HOLD_FOR, SET_LABELS],
  subjects: (LIFECYCLE_SERVED_PAYLOAD.subjects ?? []).map((subject) =>
    subject.inputPath === null
      ? { ...subject, outputs: ["ARCHIVE", ...subject.outputs, "HOLD_FOR", "SET_LABELS"] }
      : subject,
  ),
};

const SCHEMA = ruleBuilderSchemaFromPayload(LIFECYCLE_RULE_BUILDER_BASE, PAYLOAD);
const DESCRIPTORS = PAYLOAD.outputs;

/// Translates from English, handing a missing key back as the real helper does.
const t = (key: string) => (en as Record<string, string>)[key] ?? key;

function model(outputs: RuleBuilderModel["outputs"]): RuleBuilderModel {
  return {
    version: 2,
    family: "lifecycle",
    join: "ALL",
    conditions: [{ field: "input.facts.monitored", operator: "IS_TRUE", value: "" }],
    outputs,
  };
}

function problemCodes(result: ReturnType<typeof generateRuleSource>): string[] {
  return result.ok ? [] : result.problems.map((problem) => problem.code);
}

// ── The builder ───────────────────────────────────────────────────────

test("the builder lists an unseen output where the server puts it and skips an unknown kind", () => {
  const { outputs, skipped } = ruleBuilderOutputsFromPayload(PAYLOAD);
  const ids = outputs.map((output) => output.id);
  assert.equal(ids[0], "ARCHIVE");
  assert.equal(ids.at(-1), "SET_LABELS");
  assert.equal(ids.includes("HOLD_FOR"), false);
  assert.deepEqual(skipped, ["HOLD_FOR"]);
  assert.deepEqual(
    outputs.find((output) => output.id === "SET_LABELS")?.value,
    { kind: "text_list", options: "shelfLabels", maxItems: 2, maxBytes: 8, pattern: "[a-z]+" },
  );
});

test("an unseen output generates its template byte for byte, in the served order", () => {
  const result = generateRuleSource(
    SCHEMA,
    model({ SET_LABELS: ["shelf", "box"], NOTIFY: "Sample notice", ARCHIVE: true }),
    "TITLE",
  );
  assert.deepEqual(result, {
    ok: true,
    source: [
      "package rules",
      "import rego.v1",
      "",
      "rule_applies if {",
      "\tinput.facts.monitored == true",
      "}",
      "",
      "archive if rule_applies",
      "",
      'notify := "Sample notice" if rule_applies',
      "",
      'set_labels := ["shelf", "box"] if rule_applies',
      "",
    ].join("\n"),
  });
});

test("an unseen output's limits are enforced from its descriptor", () => {
  assert.deepEqual(
    problemCodes(generateRuleSource(SCHEMA, model({ SET_LABELS: ["Shelf"] }), "TITLE")),
    ["BUILDER_VALUE_INVALID"],
  );
  assert.deepEqual(
    problemCodes(generateRuleSource(SCHEMA, model({ SET_LABELS: ["overlylong"] }), "TITLE")),
    ["BUILDER_VALUE_TOO_LONG"],
  );
  assert.deepEqual(
    problemCodes(
      generateRuleSource(SCHEMA, model({ SET_LABELS: ["a", "b", "c"] }), "TITLE"),
    ),
    ["BUILDER_VALUE_TOO_MANY"],
  );
});

test("an unseen output keeps to its variants and its conflicts", () => {
  const archive = SCHEMA.outputs.find((output) => output.id === "ARCHIVE");
  assert.ok(archive);
  assert.equal(isRuleBuilderOutputInVariant(archive, "TITLE"), true);
  assert.equal(isRuleBuilderOutputInVariant(archive, "EPISODE"), false);
  // Lifecycle actions are placed by subject: one the server lists only
  // under the title is out of scope on a season row.
  assert.deepEqual(
    problemCodes(
      generateRuleSource(
        SCHEMA,
        {
          ...model({ ARCHIVE: true }),
          actions: [{ subject: "SEASON", output: "ARCHIVE", join: "ALL", conditions: [] }],
        },
        null,
      ),
    ),
    // The row's action is set aside, so the rule is left with none.
    ["BUILDER_OUTPUT_NOT_IN_SCOPE", "BUILDER_NO_OUTPUTS"],
  );

  // Read in both directions.
  const both = { ARCHIVE: true, UNMONITOR: true } as const;
  assert.deepEqual(ruleBuilderConflicts(SCHEMA, "ARCHIVE", both), ["UNMONITOR"]);
  assert.deepEqual(ruleBuilderConflicts(SCHEMA, "UNMONITOR", both), ["ARCHIVE"]);
  assert.deepEqual(
    problemCodes(generateRuleSource(SCHEMA, model(both), "TITLE")),
    ["BUILDER_OUTPUTS_CONFLICT", "BUILDER_OUTPUTS_CONFLICT"],
  );
});

test("a catalog the web cannot load is asked for, but presented as free text", () => {
  assert.deepEqual(
    ruleBuilderOptionSources(SCHEMA, model({ SET_LABELS: [], ADD_TAGS: [] })),
    ["titleTags", "shelfLabels"],
  );
  assert.equal(Object.hasOwn(RULE_BUILDER_CATALOGS, "shelfLabels"), false);
  assert.equal(Object.hasOwn(RULE_BUILDER_CATALOGS, "titleTags"), true);
});

test("an untranslated output is labelled by its head and has no help", () => {
  const archive = SCHEMA.outputs.find((output) => output.id === "ARCHIVE");
  assert.ok(archive);
  assert.equal(ruleBuilderOutputLabel(t, archive), "archive");
  assert.equal(ruleBuilderOutputHelp(t, archive), null);

  const translated = (key: string) =>
    key === ARCHIVE.labelKey ? "Archive" : key === ARCHIVE.helpKey ? "Moves it away." : t(key);
  assert.equal(ruleBuilderOutputLabel(translated, archive), "Archive");
  assert.equal(ruleBuilderOutputHelp(translated, archive), "Moves it away.");
});

// ── Summaries and reference ───────────────────────────────────────────

test("a summary labels heads from the descriptors, then by head, then by id", () => {
  assert.equal(
    ruleOutputHeadLabel(t, DESCRIPTORS, "DELETE_FILES"),
    en["ruleOutput.lifecycle.delete_files"],
  );
  assert.equal(ruleOutputHeadLabel(t, DESCRIPTORS, "ARCHIVE"), "archive");
  assert.equal(ruleOutputHeadLabel(t, DESCRIPTORS, "SOME_LATER_HEAD"), "SOME_LATER_HEAD");
  // While the descriptors load every head shows by its id.
  assert.equal(ruleOutputHeadLabel(t, [], "DELETE_FILES"), "DELETE_FILES");
  assert.equal(ruleOutputHeadHelp(t, ARCHIVE), null);
  assert.equal(isRuleOutputDestructive(DESCRIPTORS, "DELETE_TITLE"), true);
  assert.equal(isRuleOutputDestructive(DESCRIPTORS, "ARCHIVE"), false);
  assert.equal(isRuleOutputDestructive([], "DELETE_TITLE"), false);
});

test("summaries order heads as served and put unknown ones last, as given", () => {
  assert.deepEqual(
    sortByRuleOutputOrder(
      DESCRIPTORS,
      ["NOTIFY", "LATER_ONE", "SET_LABELS", "ARCHIVE", "LATER_TWO", "ADD_TAGS"],
      (id) => id,
    ),
    ["ARCHIVE", "ADD_TAGS", "NOTIFY", "SET_LABELS", "LATER_ONE", "LATER_TWO"],
  );
  assert.deepEqual(sortByRuleOutputOrder([], ["B", "A"], (id) => id), ["B", "A"]);
});

test("the reference writes each head's syntax from its template", () => {
  const syntax = (id: string) =>
    ruleOutputSyntax(DESCRIPTORS.find((descriptor) => descriptor.id === id)!);
  assert.equal(syntax("ARCHIVE"), "archive if { ... }");
  assert.equal(syntax("SET_LABELS"), 'set_labels := ["..."] if { ... }');
  assert.equal(syntax("NOTIFY"), 'notify := "..." if { ... }');
});

test("templates name the heads they define, in served order", () => {
  const source = "archive if rule_applies\n\nadd_tags := [\"x\"] if rule_applies\n";
  assert.deepEqual(templateRuleOutputs(source, DESCRIPTORS), ["ARCHIVE", "ADD_TAGS"]);
});

// ── The Rego editor ───────────────────────────────────────────────────

test("the editor completes every head its variant offers, with a snippet from its template", () => {
  const title = regoHeadAssists(DESCRIPTORS, t, "TITLE");
  const episode = regoHeadAssists(DESCRIPTORS, t, "EPISODE");
  assert.equal(title.length, DESCRIPTORS.length);
  // Lifecycle heads name their own level, so only an unseen head that lists
  // variants is left out of one.
  assert.equal(episode.some((head) => head.head === "archive"), false);
  assert.ok(episode.some((head) => head.head === "delete_episode_files"));
  assert.ok(episode.some((head) => head.head === "delete_files"));

  assert.deepEqual(
    title.find((head) => head.head === "archive"),
    {
      head: "archive",
      detail: "archive",
      documentation: null,
      snippet: "archive if {\n\t${1:true}\n}",
    },
  );
  assert.deepEqual(
    title.find((head) => head.head === "set_labels"),
    {
      head: "set_labels",
      detail: "set_labels",
      documentation: null,
      snippet: 'set_labels := ["${1:}"] if {\n\t${2:true}\n}',
    },
  );
  const notify = title.find((head) => head.head === "notify");
  assert.equal(notify?.detail, en["ruleOutput.lifecycle.notify"]);
});

test("a translated head carries its help into completion and hover", () => {
  const translated = (key: string) =>
    key === ARCHIVE.labelKey ? "Archive" : key === ARCHIVE.helpKey ? "Moves it away." : t(key);
  assert.deepEqual(
    regoHeadAssists([ARCHIVE], translated, null),
    [
      {
        head: "archive",
        detail: "Archive",
        documentation: "Moves it away.",
        snippet: "archive if {\n\t${1:true}\n}",
      },
    ],
  );
});

test("the editor offers no heads while the descriptors are loading or failed", () => {
  assert.deepEqual(regoHeadAssists([], t, "TITLE"), []);
});

test("a number head's snippet starts at its minimum", () => {
  assert.equal(
    ruleOutputSnippet({
      ...ARCHIVE,
      id: "KEEP_DAYS",
      head: "keep_days",
      valueKind: "NUMBER",
      min: 1,
      template: "{head} := {value} if {helper}",
    }),
    "keep_days := ${1:1} if {\n\t${2:true}\n}",
  );
});

// ── Catalog checks ────────────────────────────────────────────────────

test("a picked value is only called missing against a loaded, non-empty catalog", () => {
  assert.deepEqual(ruleBuilderCatalogMisses(null, ["Gone"]), []);
  assert.deepEqual(ruleBuilderCatalogMisses(undefined, ["Gone"]), []);
  assert.deepEqual(ruleBuilderCatalogMisses([], ["Gone"]), []);
  assert.deepEqual(
    ruleBuilderCatalogMisses(["Sample Profile"], ["Sample Profile", "Gone", ""]),
    ["Gone"],
  );
  assert.deepEqual(
    ruleBuilderCatalogMisses(
      [{ value: "root-1", label: "/data/sample" }],
      ["root-1", "root-2"],
    ),
    ["root-2"],
  );
});

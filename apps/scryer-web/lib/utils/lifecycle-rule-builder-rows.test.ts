import assert from "node:assert/strict";
import test from "node:test";

import { LIFECYCLE_CONTRACT_SCHEMA as SCHEMA } from "./rule-builder-contract-fixture.ts";
import {
  addRuleBuilderAction,
  generateRuleSource,
  isRuleBuilderSource,
  parseRuleBuilderModel,
  ruleBuilderActionRows,
  ruleBuilderSubjectFields,
  setRuleBuilderActionOutput,
  setRuleBuilderActionSubject,
  type RuleBuilderModel,
} from "./rule-builder.ts";

const TITLE_MONITORED = {
  field: "input.facts.monitored",
  operator: "IS_TRUE",
  value: "",
} as const;
const SEASON_UNMONITORED = {
  field: "input.season.facts.monitored",
  operator: "IS_FALSE",
  value: "",
} as const;

function mixed(): RuleBuilderModel {
  return {
    version: 2,
    family: "lifecycle",
    join: "ALL",
    conditions: [{ ...TITLE_MONITORED }],
    outputs: { ADD_TAGS: ["Reviewed"], DELETE_SEASON_FILES: true },
    actions: [
      { subject: "TITLE", output: "ADD_TAGS", join: "ALL", conditions: [] },
      {
        subject: "SEASON",
        output: "DELETE_SEASON_FILES",
        join: "ALL",
        conditions: [{ ...SEASON_UNMONITORED }],
      },
    ],
  };
}

function problems(model: RuleBuilderModel) {
  const result = generateRuleSource(SCHEMA, model, null);
  return result.ok ? [] : result.problems;
}

test("a mixed-level rule writes each head against its own helper and round-trips", () => {
  const model = mixed();
  const result = generateRuleSource(SCHEMA, model, null);
  assert.ok(result.ok, JSON.stringify(result));
  assert.equal(
    result.source,
    [
      "package rules",
      "import rego.v1",
      "",
      "rule_applies if {",
      "\tinput.facts.monitored == true",
      "}",
      "",
      "rule_applies_2 if {",
      "\trule_applies",
      "\tinput.season.facts.monitored == false",
      "}",
      "",
      'add_tags := ["Reviewed"] if rule_applies',
      "",
      "delete_season_files if rule_applies_2",
      "",
    ].join("\n"),
  );
  const stored = JSON.parse(JSON.stringify(model));
  assert.deepEqual(parseRuleBuilderModel(SCHEMA, stored), model);
  assert.equal(isRuleBuilderSource(SCHEMA, model, null, result.source), true);
});

test("the shared conditions and a title row read only title facts", () => {
  const title = ruleBuilderSubjectFields(SCHEMA, null, SCHEMA.subjects![0]!);
  assert.equal(
    title.some((field) => field.path.startsWith("input.season.")),
    false,
  );
  const season = SCHEMA.subjects!.find((subject) => subject.id === "SEASON")!;
  const seasonFields = ruleBuilderSubjectFields(SCHEMA, null, season);
  assert.ok(seasonFields.some((field) => field.path === "input.facts.monitored"));
  assert.ok(
    seasonFields.some((field) => field.path === "input.season.facts.monitored"),
  );
  assert.equal(
    seasonFields.some((field) => field.path.startsWith("input.episode.")),
    false,
  );

  const sharedSeason = { ...mixed(), conditions: [{ ...SEASON_UNMONITORED }] };
  assert.deepEqual(
    problems(sharedSeason).map((problem) => problem.code),
    ["BUILDER_FACT_UNKNOWN"],
  );
});

test("each action is offered only under its own subject", () => {
  const model = mixed();
  model.actions![0] = {
    subject: "TITLE",
    output: "DELETE_SEASON_FILES",
    join: "ALL",
    conditions: [],
  };
  model.actions![1] = { ...model.actions![1]!, output: "ADD_TAGS" };
  const codes = problems(model).map((problem) => [problem.code, problem.actionIndex]);
  assert.deepEqual(codes.slice(0, 2), [
    ["BUILDER_OUTPUT_NOT_IN_SCOPE", 0],
    ["BUILDER_OUTPUT_NOT_IN_SCOPE", 1],
  ]);
});

test("moving a row keeps an action the new subject has and clears one it lacks", () => {
  let model: RuleBuilderModel = {
    version: 2,
    family: "lifecycle",
    join: "ALL",
    conditions: [{ ...TITLE_MONITORED }],
    outputs: {},
    actions: [],
  };
  model = addRuleBuilderAction(SCHEMA, model);
  model = setRuleBuilderActionOutput(SCHEMA, model, 0, "UNMONITOR");
  model = addRuleBuilderAction(SCHEMA, model);
  model = setRuleBuilderActionOutput(SCHEMA, model, 1, "ADD_TAGS");

  const moved = setRuleBuilderActionSubject(SCHEMA, model, 0, "SEASON");
  assert.deepEqual(ruleBuilderActionRows(SCHEMA, moved)[0], {
    subject: "SEASON",
    output: "UNMONITOR_SEASON",
    join: "ALL",
    conditions: [],
  });
  assert.equal(Object.hasOwn(moved.outputs, "UNMONITOR"), false);
  assert.equal(moved.outputs.UNMONITOR_SEASON, true);
  // The other row is untouched.
  assert.deepEqual(ruleBuilderActionRows(SCHEMA, moved)[1], ruleBuilderActionRows(SCHEMA, model)[1]);

  const cleared = setRuleBuilderActionSubject(SCHEMA, moved, 1, "EPISODE");
  assert.deepEqual(ruleBuilderActionRows(SCHEMA, cleared)[1], {
    subject: "EPISODE",
    output: "",
    join: "ALL",
    conditions: [],
  });
  assert.equal(Object.hasOwn(cleared.outputs, "ADD_TAGS"), false);
  assert.deepEqual(
    problems(cleared).map((problem) => [problem.code, problem.actionIndex]),
    [["BUILDER_ACTION_REQUIRED", 1]],
  );
});

test("a condition the new subject cannot read is reported against its row, not dropped", () => {
  const moved = setRuleBuilderActionSubject(SCHEMA, mixed(), 1, "EPISODE");
  const row = ruleBuilderActionRows(SCHEMA, moved)[1]!;
  assert.equal(row.output, "DELETE_EPISODE_FILES");
  assert.deepEqual(row.conditions, [SEASON_UNMONITORED]);
  assert.deepEqual(
    problems(moved).map((problem) => [problem.code, problem.actionIndex, problem.conditionIndex]),
    [["BUILDER_FACT_UNKNOWN", 1, 0]],
  );
});

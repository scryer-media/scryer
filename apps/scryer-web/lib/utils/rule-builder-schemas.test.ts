import assert from "node:assert/strict";
import test from "node:test";

import maintenanceFixtures from "../contracts/maintenance-builder-fixtures.json" with { type: "json" };
import regoAssistance from "../contracts/rego-assistance.json" with { type: "json" };
import requestFixtures from "../contracts/request-builder-fixtures.json" with { type: "json" };
import scoringFixtures from "../contracts/scoring-builder-fixtures.json" with { type: "json" };
import cs from "../i18n/locales/cs.ts";
import en from "../i18n/locales/en.ts";
import ru from "../i18n/locales/ru.ts";
import zhHK from "../i18n/locales/zh_HK.ts";
import zhTW from "../i18n/locales/zh_TW.ts";
import { MAINTENANCE_RULE_BUILDER_SCHEMA } from "./maintenance-rule-builder-schema.ts";
import {
  MAINTENANCE_ACTION_HEAD_ORDER,
  MAINTENANCE_ACTION_HEAD_RULE_NAMES,
} from "./maintenance-rule-sets.ts";
import { REQUEST_RULE_BUILDER_SCHEMA } from "./request-rule-builder-schema.ts";
import {
  RULE_BUILDER_PROBLEM_CODES,
  RULE_BUILDER_PROBLEM_KEYS,
  generateRuleSource,
  parseRuleBuilderModel,
  ruleBuilderFields,
  type RuleBuilderModel,
  type RuleBuilderSchema,
} from "./rule-builder.ts";
import { SCORING_RULE_BUILDER_SCHEMA } from "./scoring-rule-builder-schema.ts";

type Fixture = {
  id: string;
  scope: string | null;
  model: unknown;
  source: string;
};

type Family = {
  name: string;
  schema: RuleBuilderSchema;
  fixtures: Fixture[];
  /// Every variant a rule of the family can be written for.
  variants: (string | null)[];
};

const FAMILIES: Family[] = [
  {
    name: "maintenance",
    schema: MAINTENANCE_RULE_BUILDER_SCHEMA,
    fixtures: maintenanceFixtures as Fixture[],
    variants: ["TITLE", "SEASON", "EPISODE"],
  },
  {
    name: "scoring",
    schema: SCORING_RULE_BUILDER_SCHEMA,
    fixtures: scoringFixtures as Fixture[],
    variants: [null],
  },
  {
    name: "request",
    schema: REQUEST_RULE_BUILDER_SCHEMA,
    fixtures: requestFixtures as Fixture[],
    variants: [null],
  },
];

function parsed(family: Family, entry: Fixture): RuleBuilderModel {
  const model = parseRuleBuilderModel(family.schema, entry.model);
  assert.ok(model, `${family.name}/${entry.id}`);
  return model;
}

// ── Pinned sources ────────────────────────────────────────────────────

for (const family of FAMILIES) {
  test(`every ${family.name} fixture generates exactly its pinned source`, () => {
    assert.ok(family.fixtures.length > 0);
    for (const entry of family.fixtures) {
      const result = generateRuleSource(family.schema, parsed(family, entry), entry.scope);
      assert.deepEqual(result, { ok: true, source: entry.source }, entry.id);
      assert.ok(entry.source.startsWith(family.schema.preamble), entry.id);
      assert.equal(entry.source.endsWith("\n\n"), false, entry.id);
      assert.equal(/^ +\S/m.test(entry.source), false, entry.id);
    }
  });

  test(`${family.name} fixtures cover every comparison, every output and both joins`, () => {
    const offered = new Map<string, string>();
    for (const variant of family.variants)
      for (const field of ruleBuilderFields(family.schema, variant))
        offered.set(field.path, field.kind);
    const expected = new Set<string>();
    for (const kind of new Set(offered.values()))
      for (const operator of family.schema.kinds[kind].operators)
        expected.add(`${kind}:${operator.id}`);

    const covered = new Set<string>();
    const outputs = new Set<string>();
    const joins = new Set<string>();
    for (const entry of family.fixtures) {
      const model = parsed(family, entry);
      joins.add(model.join);
      for (const id of Object.keys(model.outputs)) outputs.add(id);
      for (const condition of model.conditions) {
        const kind = offered.get(condition.field);
        assert.ok(kind, `${entry.id}: ${condition.field}`);
        covered.add(`${kind}:${condition.operator}`);
      }
    }
    assert.deepEqual([...covered].sort(), [...expected].sort());
    assert.deepEqual(
      [...outputs].sort(),
      family.schema.outputs.map((output) => output.id).sort(),
    );
    assert.deepEqual([...joins].sort(), ["ALL", "ANY"]);
  });

  test(`${family.name} fixtures use only the model shape this build writes`, () => {
    for (const entry of family.fixtures) {
      const model = parsed(family, entry);
      if ((entry.model as { version?: unknown }).version === model.version) {
        assert.deepEqual(model, entry.model, entry.id);
      }
      assert.equal(model.family, family.schema.family, entry.id);
    }
  });
}

test("maintenance fixtures are stored version 1 documents and upgrade in memory", () => {
  for (const entry of maintenanceFixtures as Fixture[]) {
    const document = entry.model as {
      version: number;
      conditions: { fact: string }[];
      actions: Record<string, unknown>;
    };
    assert.equal(document.version, 1, entry.id);
    const model = parsed(FAMILIES[0], entry);
    assert.equal(model.version, 2);
    assert.deepEqual(
      model.conditions.map((condition) => condition.field),
      document.conditions.map((condition) =>
        condition.fact === "" ? "" : `input.facts.${condition.fact}`,
      ),
      entry.id,
    );
    assert.deepEqual(model.outputs, document.actions, entry.id);
  }
});

test("scoring and request fixtures are scope-free version 2 documents with synthetic names", () => {
  for (const entry of [...scoringFixtures, ...requestFixtures] as Fixture[]) {
    assert.equal(entry.scope, null, entry.id);
    assert.equal((entry.model as { version: number }).version, 2, entry.id);
  }
});

// ── Contract drift ────────────────────────────────────────────────────

for (const family of FAMILIES) {
  test(`every ${family.name} schema reference exists in its input contract`, () => {
    const { contract } = family.schema;
    const types = new Set(
      contract.sections.flatMap((section) => section.fields.map((field) => field.type)),
    );
    for (const rule of family.schema.typeRules)
      assert.ok(types.has(rule.type), `type ${rule.type}`);
    for (const source of family.schema.conditionSources) {
      const section = contract.sections.find((entry) => entry.path === source.section);
      assert.ok(section, source.section);
      const names = new Set(section.fields.map((field) => field.field));
      for (const name of source.exclude ?? [])
        assert.ok(names.has(name), `${source.section}.${name}`);
      for (const name of Object.keys(source.fields ?? {}))
        assert.ok(names.has(name), `${source.section}.${name}`);
    }
    for (const variant of family.variants)
      assert.ok(ruleBuilderFields(family.schema, variant).length > 0, String(variant));
  });
}

test("maintenance outputs are the action heads in the order the server applies them", () => {
  const outputs = MAINTENANCE_RULE_BUILDER_SCHEMA.outputs;
  assert.deepEqual(
    outputs.map((output) => output.id),
    [...MAINTENANCE_ACTION_HEAD_ORDER],
  );
  for (const output of outputs)
    assert.equal(
      output.head,
      MAINTENANCE_ACTION_HEAD_RULE_NAMES[output.id as keyof typeof MAINTENANCE_ACTION_HEAD_RULE_NAMES],
      output.id,
    );
});

test("maintenance conditions offer every fact except object lists", () => {
  const section = MAINTENANCE_RULE_BUILDER_SCHEMA.contract.sections.find(
    (entry) => entry.path === "input.facts",
  );
  assert.ok(section);
  const offered = new Map(
    ruleBuilderFields(MAINTENANCE_RULE_BUILDER_SCHEMA, "TITLE").map((field) => [
      field.label,
      field.kind,
    ]),
  );
  for (const field of section.fields) {
    const objectList = field.type.endsWith("[]") && field.type !== "string[]";
    assert.equal(offered.has(field.field), !objectList, field.field);
  }
  assert.equal(offered.get("added_at"), "timestamp");
  assert.equal(offered.get("tags"), "string_list");
});

test("scoring outputs call only helpers the Rego reference lists", () => {
  const helpers = new Set(regoAssistance.helpers.map((helper) => helper.name));
  for (const output of SCORING_RULE_BUILDER_SCHEMA.outputs)
    for (const call of output.template.match(/scryer\.[a-z_]+/g) ?? [])
      assert.ok(helpers.has(call), `${output.id}: ${call}`);
});

test("request rules leave requester fields out of the builder", () => {
  for (const field of ruleBuilderFields(REQUEST_RULE_BUILDER_SCHEMA, null))
    assert.equal(field.path.startsWith("input.requester"), false, field.path);
});

// ── Labels ────────────────────────────────────────────────────────────

/// Every locale that carries the builder's own strings.
const BUILDER_LOCALES: [string, Record<string, string>][] = [
  ["en", en as Record<string, string>],
  ["cs", cs as Record<string, string>],
  ["ru", ru as Record<string, string>],
  ["zh_HK", zhHK as Record<string, string>],
  ["zh_TW", zhTW as Record<string, string>],
];

function builderKeys(schema: RuleBuilderSchema): string[] {
  const keys = new Set<string>(Object.values(schema.labels).filter(Boolean) as string[]);
  for (const kind of Object.values(schema.kinds))
    for (const operator of kind.operators) keys.add(operator.labelKey);
  for (const source of schema.conditionSources)
    for (const override of Object.values(source.fields ?? {}))
      for (const choice of override.choices ?? []) keys.add(choice.labelKey);
  for (const output of schema.outputs) {
    keys.add(output.labelKey);
    const value = output.value;
    if ("placeholderKey" in value && value.placeholderKey) keys.add(value.placeholderKey);
    if (value.kind === "text_list" && value.emptyKey) keys.add(value.emptyKey);
    if (value.kind === "structured")
      for (const part of value.parts) {
        keys.add(part.labelKey);
        if (part.placeholderKey) keys.add(part.placeholderKey);
      }
  }
  return [...keys];
}

/// Keys the builder owns. Anything else it reuses, such as a facet's name,
/// keeps the coverage it already has.
const BUILDER_KEY =
  /^settings\.(ruleBuilder|scoringBuilder|requestBuilder|maintenanceBuilder)/;

for (const family of FAMILIES) {
  test(`every ${family.name} builder label is translated`, () => {
    for (const key of builderKeys(family.schema)) {
      assert.ok((en as Record<string, string>)[key], `en: ${key}`);
      if (!BUILDER_KEY.test(key)) continue;
      for (const [name, strings] of BUILDER_LOCALES)
        assert.ok(strings[key], `${name}: ${key}`);
    }
    for (const variant of family.variants)
      for (const field of ruleBuilderFields(family.schema, variant))
        assert.ok((en as Record<string, string>)[field.descKey], field.descKey);
  });
}

test("every problem code has a translated message", () => {
  for (const code of RULE_BUILDER_PROBLEM_CODES)
    for (const [name, strings] of BUILDER_LOCALES)
      assert.ok(strings[RULE_BUILDER_PROBLEM_KEYS[code]], `${name}: ${code}`);
});

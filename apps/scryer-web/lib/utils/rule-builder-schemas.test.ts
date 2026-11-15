import assert from "node:assert/strict";
import test from "node:test";

import maintenanceFixtures from "../contracts/maintenance-builder-fixtures.json" with { type: "json" };
import regoAssistance from "../contracts/rego-assistance.json" with { type: "json" };
import requestFixtures from "../contracts/request-builder-fixtures.json" with { type: "json" };
import operatorSemantics from "../contracts/rule-builder-operator-semantics.json" with { type: "json" };
import scoringFixtures from "../contracts/scoring-builder-fixtures.json" with { type: "json" };
import cs from "../i18n/locales/cs.ts";
import en from "../i18n/locales/en.ts";
import ru from "../i18n/locales/ru.ts";
import zhHK from "../i18n/locales/zh_HK.ts";
import zhTW from "../i18n/locales/zh_TW.ts";
import { RULE_BUILDER_CATALOGS } from "./rule-builder-catalog-presentation.ts";
import {
  MAINTENANCE_CONTRACT_SCHEMA,
  MAINTENANCE_SERVED_PAYLOAD,
  REQUEST_CONTRACT_SCHEMA,
  REQUEST_SERVED_PAYLOAD,
  SCORING_CONTRACT_SCHEMA,
} from "./rule-builder-contract-fixture.ts";
import {
  RULE_BUILDER_PROBLEM_CODES,
  RULE_BUILDER_PROBLEM_KEYS,
  generateRuleSource,
  parseRuleBuilderModel,
  ruleBuilderFields,
  type RuleBuilderModel,
  type RuleBuilderSchema,
  type RuleBuilderSchemaPayload,
} from "./rule-builder.ts";

/// A fixture's `cases` say whether `rule_applies` holds for an input. They are
/// checked by evaluating the pinned source; this build has no Rego engine, so
/// here only their shape is held.
type FixtureCase = { input: Record<string, unknown>; expected: boolean };

type Fixture = {
  id: string;
  scope: string | null;
  model: unknown;
  source: string;
  cases: FixtureCase[];
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
    schema: MAINTENANCE_CONTRACT_SCHEMA,
    fixtures: maintenanceFixtures as Fixture[],
    variants: ["TITLE", "SEASON", "EPISODE"],
  },
  {
    name: "scoring",
    schema: SCORING_CONTRACT_SCHEMA,
    fixtures: scoringFixtures as Fixture[],
    variants: [null],
  },
  {
    name: "request",
    schema: REQUEST_CONTRACT_SCHEMA,
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
      assert.deepEqual(model, entry.model, entry.id);
      assert.equal(model.family, family.schema.family, entry.id);
    }
  });

  test(`every ${family.name} fixture has cases on both sides of its rule`, () => {
    for (const entry of family.fixtures) {
      assert.ok(Array.isArray(entry.cases), entry.id);
      assert.ok(entry.cases.some((entry) => entry.expected === true), entry.id);
      assert.ok(entry.cases.some((entry) => entry.expected === false), entry.id);
      for (const entryCase of entry.cases) {
        assert.deepEqual(Object.keys(entryCase).sort(), ["expected", "input"], entry.id);
        assert.equal(typeof entryCase.expected, "boolean", entry.id);
        assert.equal(typeof entryCase.input, "object", entry.id);
        // `days_since` reads the clock from the input.
        if (family.name === "maintenance")
          assert.equal(typeof entryCase.input.evaluation_time, "string", entry.id);
      }
    }
  });
}

test("every fixture is a current version document", () => {
  for (const family of FAMILIES)
    for (const entry of family.fixtures)
      assert.equal((entry.model as { version: number }).version, 2, entry.id);
});

test("scoring and request fixtures are scope-free version 2 documents with synthetic names", () => {
  for (const entry of [...scoringFixtures, ...requestFixtures] as Fixture[]) {
    assert.equal(entry.scope, null, entry.id);
    assert.equal((entry.model as { version: number }).version, 2, entry.id);
  }
});

// ── Operator semantics ────────────────────────────────────────────────

/// One comparison on one field, the lines the builder writes for it, and
/// whether it holds for present values, for `null` (only on a field whose
/// contract type is nullable) and for a missing field.
type SemanticsRow = {
  field: string;
  operator: string;
  value: string;
  lines: string[];
  present: { value: unknown; expected: boolean }[];
  null?: boolean;
  absent: boolean;
};

type SemanticsFamily = {
  variant: string | null;
  evaluationTime?: string;
  rows: SemanticsRow[];
};

const SEMANTICS = operatorSemantics as Record<string, SemanticsFamily>;

/// The output a semantics row is generated with; it only has to be valid.
const SEMANTICS_OUTPUT: Record<string, Record<string, unknown>> = {
  maintenance: { NOTIFY: "Sample notice" },
  request: { MANUAL: true },
  scoring: { BLOCK: { code: "sample_code" } },
};

/// Operators that ask about a missing value, so may hold for one.
const ASKS_ABOUT_MISSING = new Set(["HAS_NO_VALUE"]);

function contractType(schema: RuleBuilderSchema, path: string): string | undefined {
  for (const section of schema.contract.sections)
    for (const field of section.fields)
      if (`${section.path}.${field.field}` === path) return field.type;
  return undefined;
}

for (const family of FAMILIES) {
  const semantics = SEMANTICS[family.name];

  test(`every ${family.name} comparison writes the lines its semantics row pins`, () => {
    assert.ok(semantics, family.name);
    for (const row of semantics.rows) {
      const result = generateRuleSource(
        family.schema,
        {
          version: 2,
          family: family.schema.family,
          join: "ALL",
          conditions: [{ field: row.field, operator: row.operator, value: row.value }],
          outputs: SEMANTICS_OUTPUT[family.name],
        } as RuleBuilderModel,
        semantics.variant,
      );
      assert.ok(result.ok, `${row.field} ${row.operator}`);
      const body = result.source.split(`${family.schema.helperRule} if {\n`)[1].split("\n}\n")[0];
      assert.deepEqual(
        body.split("\n").map((line) => line.replace(/^\t/, "")),
        row.lines,
        `${row.field} ${row.operator}`,
      );
    }
  });

  test(`${family.name} semantics cover every comparison of every kind it offers`, () => {
    const offered = new Map<string, string>();
    for (const variant of family.variants)
      for (const field of ruleBuilderFields(family.schema, variant))
        offered.set(field.path, field.kind);
    const expected = new Set<string>();
    for (const kind of new Set(offered.values()))
      for (const operator of family.schema.kinds[kind].operators)
        expected.add(`${kind}:${operator.id}`);
    const covered = new Set(
      semantics.rows.map((row) => `${offered.get(row.field)}:${row.operator}`),
    );
    assert.deepEqual([...covered].sort(), [...expected].sort());
    if (family.name === "maintenance") assert.ok(semantics.evaluationTime);
  });

  test(`${family.name} comparisons hold for a missing or null value only when they ask about one`, () => {
    for (const row of semantics.rows) {
      const label = `${row.field} ${row.operator}`;
      const outcomes = [...row.present.map((entry) => entry.expected), row.null, row.absent];
      assert.ok(outcomes.includes(true) && outcomes.includes(false), label);
      const nullable = contractType(family.schema, row.field)?.endsWith("?") ?? false;
      assert.equal(row.null !== undefined, nullable, label);
      const omittedList =
        family.schema.conditionSources.some(
          (source) =>
            row.field.startsWith(`${source.section}.`) &&
            source.fields?.[row.field.slice(source.section.length + 1)]?.kind ===
              "omitted_when_empty_string_list",
        );
      // A list the input leaves out when empty is an empty list, so it lacks
      // every value and is empty; nothing else holds for a missing value
      // unless it asks about one.
      const holdsWhenMissing =
        ASKS_ABOUT_MISSING.has(row.operator) ||
        (omittedList && (row.operator === "NOT_CONTAINS" || row.operator === "IS_EMPTY"));
      assert.equal(row.absent, holdsWhenMissing, label);
      if (row.null !== undefined) assert.equal(row.null, ASKS_ABOUT_MISSING.has(row.operator), label);
    }
  });
}

test("a moment still to come is never some days ago", () => {
  const rows = SEMANTICS.maintenance.rows.filter(
    (row) => row.operator === "MORE_THAN_DAYS_AGO" || row.operator === "LESS_THAN_DAYS_AGO",
  );
  assert.equal(rows.length, 2);
  const now = Date.parse(SEMANTICS.maintenance.evaluationTime ?? "");
  for (const row of rows) {
    const future = row.present.filter((entry) => Date.parse(String(entry.value)) > now);
    assert.ok(future.length > 0, row.operator);
    for (const entry of future) assert.equal(entry.expected, false, row.operator);
  }
  assert.ok(
    rows
      .find((row) => row.operator === "LESS_THAN_DAYS_AGO")
      ?.lines.includes("days_since(input.facts.added_at) >= 0"),
  );
});

test("a false flag is written as a comparison, so a missing one is not false", () => {
  for (const family of FAMILIES) {
    const row = SEMANTICS[family.name].rows.find((entry) => entry.operator === "IS_FALSE");
    assert.ok(row, family.name);
    assert.deepEqual(row.lines, [`${row.field} == false`]);
    assert.equal(row.absent, false);
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

const SERVED: [string, RuleBuilderSchemaPayload][] = [
  ["maintenance", MAINTENANCE_SERVED_PAYLOAD],
  ["request", REQUEST_SERVED_PAYLOAD],
];

for (const [name, payload] of SERVED) {
  test(`the ${name} served outputs are well formed and all reach the builder`, () => {
    assert.equal(payload.family, name);
    const ids = payload.outputs.map((output) => output.id);
    const heads = payload.outputs.map((output) => output.head);
    assert.equal(new Set(ids).size, ids.length);
    assert.equal(new Set(heads).size, heads.length);
    for (const output of payload.outputs) {
      assert.equal(output.labelKey, `ruleOutput.${name}.${output.head}`, output.id);
      if (output.helpKey) assert.equal(output.helpKey, `${output.labelKey}.help`, output.id);
      for (const other of output.conflictsWith) assert.ok(ids.includes(other), output.id);
      if (output.options)
        assert.ok(Object.hasOwn(RULE_BUILDER_CATALOGS, output.options), output.options);
    }
    const schema = name === "maintenance" ? MAINTENANCE_CONTRACT_SCHEMA : REQUEST_CONTRACT_SCHEMA;
    assert.deepEqual(
      schema.outputs.map((output) => output.id),
      ids,
    );
  });
}

test("maintenance conditions offer every fact except object lists", () => {
  const section = MAINTENANCE_CONTRACT_SCHEMA.contract.sections.find(
    (entry) => entry.path === "input.facts",
  );
  assert.ok(section);
  const offered = new Map(
    ruleBuilderFields(MAINTENANCE_CONTRACT_SCHEMA, "TITLE").map((field) => [
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
  for (const output of SCORING_CONTRACT_SCHEMA.outputs)
    for (const call of output.template.match(/scryer\.[a-z_]+/g) ?? [])
      assert.ok(helpers.has(call), `${output.id}: ${call}`);
});

test("request rules leave requester fields out of the builder", () => {
  for (const field of ruleBuilderFields(REQUEST_CONTRACT_SCHEMA, null))
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
    if (output.value.kind === "structured")
      for (const part of output.value.parts) {
        keys.add(part.labelKey);
        if (part.placeholderKey) keys.add(part.placeholderKey);
      }
  }
  return [...keys];
}

/// Keys the builder owns. Anything else it reuses, such as a facet's name,
/// keeps the coverage it already has.
const BUILDER_KEY =
  /^(settings\.(ruleBuilder|scoringBuilder|requestBuilder|maintenanceBuilder)|ruleOutput\.)/;

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

test("every served output's label and help tag is translated in every builder locale", () => {
  for (const [, payload] of SERVED)
    for (const output of payload.outputs) {
      const keys = [output.labelKey, ...(output.helpKey ? [output.helpKey] : [])];
      // A placeholder is optional, but once English has one every locale does.
      const placeholder = `${output.labelKey}.placeholder`;
      if ((en as Record<string, string>)[placeholder]) keys.push(placeholder);
      for (const key of keys)
        for (const [name, strings] of BUILDER_LOCALES)
          assert.ok(strings[key], `${name}: ${key}`);
    }
});

test("every catalog the builder presents has its strings in every builder locale", () => {
  for (const presentation of Object.values(RULE_BUILDER_CATALOGS))
    for (const key of [presentation.placeholderKey, presentation.emptyKey].filter(Boolean) as string[])
      for (const [name, strings] of BUILDER_LOCALES)
        assert.ok(strings[key], `${name}: ${key}`);
});

test("every problem code has a translated message", () => {
  for (const code of RULE_BUILDER_PROBLEM_CODES)
    for (const [name, strings] of BUILDER_LOCALES)
      assert.ok(strings[RULE_BUILDER_PROBLEM_KEYS[code]], `${name}: ${code}`);
});

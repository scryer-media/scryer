import assert from "node:assert/strict";
import test from "node:test";

import {
  initialMaintenanceRuleDraft,
  maintenanceRuleBuilder,
} from "../utils/maintenance-rule-sets.ts";
import {
  initialRequestRuleDraft,
  requestRuleBuilder,
} from "../utils/request-rule-sets.ts";
import {
  MAINTENANCE_CONTRACT_SCHEMA,
  REQUEST_CONTRACT_SCHEMA,
  SCORING_CONTRACT_SCHEMA,
} from "../utils/rule-builder-contract-fixture.ts";
import {
  generateRuleSource,
  parseRuleBuilderModel,
  RULE_BUILDER_MODEL_VERSION,
  type RuleBuilderModel,
  type RuleBuilderSchema,
} from "../utils/rule-builder.ts";
import {
  initialRuleEditorMode,
  openRuleTemplateDraft,
  ruleBuilderModelToSave,
  withTemplateRuleBuilderModel,
  type RuleBuilderBinding,
  type RuleBuilderDraft,
} from "../utils/rule-builder-mode.ts";
import {
  RULE_SET_INITIAL_DRAFT,
  scoringRuleBuilder,
} from "../utils/rule-sets.ts";
import { MAINTENANCE_RULE_TEMPLATES } from "./maintenance-rule-templates.ts";
import { REQUEST_RULE_TEMPLATES } from "./request-rule-templates.ts";
import { RULE_TEMPLATES } from "./rule-templates.ts";
import {
  RULE_TEMPLATE_BUILDER_MODELS,
  ruleTemplateBuilderModel,
} from "./rule-template-builder-models.ts";

/// The largest builder document the server stores.
const MAX_BUILDER_MODEL_BYTES = 64 * 1024;

type Template = { id: string; regoSource: string };

/// What picking a template gives: the draft the gallery fills in, the draft
/// the editor opens, and the model a save would store.
type Opened = {
  initial: RuleBuilderDraft;
  next: RuleBuilderDraft;
  saved: RuleBuilderModel | null;
};

type Family = {
  schema: RuleBuilderSchema;
  templates: Template[];
  /// The templates the builder cannot express, which keep opening as Rego.
  advanced: string[];
  /// The source a template holds for what the builder writes: the whole
  /// module, or for scoring the body the editor shows, which the server
  /// completes with its package and import on save.
  stored: (written: string) => string;
  variant: (template: Template) => string | null;
  open: (template: Template, schema: RuleBuilderSchema | null) => Opened;
};

function family<D extends RuleBuilderDraft, T extends Template>(spec: {
  schema: RuleBuilderSchema;
  templates: T[];
  advanced: string[];
  editorBody?: boolean;
  binding: (schema: RuleBuilderSchema | null) => RuleBuilderBinding<D>;
  draft: (template: T) => D;
}): Family {
  const find = (template: Template) =>
    spec.templates.find((entry) => entry.id === template.id)!;
  return {
    schema: spec.schema,
    templates: spec.templates,
    advanced: spec.advanced,
    stored: (written) =>
      spec.editorBody ? written.slice(spec.schema.preamble.length) : written,
    variant: (template) =>
      spec.binding(spec.schema).variant(spec.draft(find(template))),
    open: (template, schema) => {
      const binding = spec.binding(schema);
      const initial = spec.draft(find(template));
      const next = withTemplateRuleBuilderModel(
        binding,
        initial,
        ruleTemplateBuilderModel(schema, template.id),
      );
      return { initial, next, saved: ruleBuilderModelToSave(binding, next) };
    },
  };
}

const FAMILIES: Family[] = [
  family({
    schema: MAINTENANCE_CONTRACT_SCHEMA,
    binding: maintenanceRuleBuilder,
    templates: MAINTENANCE_RULE_TEMPLATES,
    draft: (template) => ({
      ...initialMaintenanceRuleDraft(),
      regoSource: template.regoSource,
      subjectKind: template.subjectKind ?? "TITLE",
    }),
    advanced: [
      // Reads each file's height through `some file in input.facts.files`.
      "four-k-purge",
      // The system-owned rule; its source is the one the server ships.
      "expired-request-leases",
    ],
  }),
  family({
    schema: REQUEST_CONTRACT_SCHEMA,
    binding: requestRuleBuilder,
    templates: REQUEST_RULE_TEMPLATES,
    draft: (template) => ({
      ...initialRequestRuleDraft(),
      regoSource: template.regoSource,
    }),
    advanced: [
      // Each reads `input.requester`, which the builder leaves to Rego.
      "named-requesters-family-rated",
      "short-lease-approval",
      "low-resolution-approval",
    ],
  }),
  family({
    schema: SCORING_CONTRACT_SCHEMA,
    binding: scoringRuleBuilder,
    templates: RULE_TEMPLATES,
    editorBody: true,
    draft: (template) => ({
      ...RULE_SET_INITIAL_DRAFT,
      regoSource: template.regoSource,
    }),
    advanced: [
      "freeleech-bonus",
      "halfleech-bonus",
      "well-seeded-bonus",
      "block-exe-torrents",
      "prefer-web-dl",
      "prefer-hevc",
      "size-limits",
      "prefer-atmos-audio",
      "release-group-scores",
      "block-obfuscated-retagged",
    ],
  }),
];

test("every template model writes its template's source byte for byte", () => {
  for (const { schema, templates, stored, variant } of FAMILIES) {
    for (const template of templates) {
      const model = ruleTemplateBuilderModel(schema, template.id);
      if (!model) continue;
      const result = generateRuleSource(schema, model, variant(template));
      assert.ok(result.ok, `${template.id}: ${JSON.stringify(result)}`);
      assert.equal(stored(result.source), template.regoSource, template.id);
    }
  }
});

test("every template model passes the checks a saved builder rule passes", () => {
  for (const { schema, templates } of FAMILIES) {
    for (const template of templates) {
      const model = ruleTemplateBuilderModel(schema, template.id);
      if (!model) continue;
      // The server stores a document that names the rule's own family and a
      // positive version, within a size bound.
      assert.equal(model.family, schema.family, template.id);
      assert.equal(model.version, RULE_BUILDER_MODEL_VERSION, template.id);
      assert.ok(
        new TextEncoder().encode(JSON.stringify(model)).length <=
          MAX_BUILDER_MODEL_BYTES,
        template.id,
      );
      // The client reads the stored document back as the same model.
      assert.deepEqual(
        parseRuleBuilderModel(schema, JSON.parse(JSON.stringify(model))),
        model,
        template.id,
      );
    }
  }
});

test("picking a template the builder expresses opens it in the builder, filled in", () => {
  for (const { schema, templates, open } of FAMILIES) {
    for (const template of templates) {
      const model = ruleTemplateBuilderModel(schema, template.id);
      if (!model) continue;
      const { next, saved } = open(template, schema);
      assert.deepEqual(next.builderModel, model, template.id);
      assert.equal(next.regoSource, template.regoSource, template.id);
      assert.equal(initialRuleEditorMode(next), "simple", template.id);
      assert.deepEqual(saved, model, template.id);
    }
  }
});

test("a template the builder cannot express keeps opening as Rego", () => {
  for (const { schema, templates, advanced, open } of FAMILIES) {
    const withoutModel = templates
      .filter((template) => !ruleTemplateBuilderModel(schema, template.id))
      .map((template) => template.id);
    assert.deepEqual(withoutModel, advanced, schema.family);
    for (const id of advanced) {
      const template = templates.find((entry) => entry.id === id)!;
      const { initial, next, saved } = open(template, schema);
      assert.deepEqual(next, initial, id);
      assert.equal(initialRuleEditorMode(next), "edit", id);
      assert.equal(saved, null, id);
    }
  }
});

test("before the schema is read every template opens as Rego", () => {
  for (const { templates, open } of FAMILIES) {
    for (const template of templates) {
      const { initial, next } = open(template, null);
      assert.deepEqual(next, initial, template.id);
      assert.equal(initialRuleEditorMode(next), "edit", template.id);
    }
  }
});

test("a model that no longer writes its template's source opens it as Rego", () => {
  const model = ruleTemplateBuilderModel(
    MAINTENANCE_CONTRACT_SCHEMA,
    "dead-wanted-entries",
  );
  assert.ok(model);
  const initial = {
    ...initialMaintenanceRuleDraft(),
    regoSource:
      "package rules\nimport rego.v1\n\nunmonitor if input.facts.monitored\n",
  };
  const next = withTemplateRuleBuilderModel(
    maintenanceRuleBuilder(MAINTENANCE_CONTRACT_SCHEMA),
    initial,
    model,
  );
  assert.deepEqual(next, initial);
});

test("every model belongs to a template of its family", () => {
  for (const { schema, templates } of FAMILIES) {
    const ids = new Set(templates.map((template) => template.id));
    for (const id of Object.keys(
      RULE_TEMPLATE_BUILDER_MODELS[schema.family] ?? {},
    )) {
      assert.ok(
        ids.has(id),
        `${schema.family} has a model for no template: ${id}`,
      );
    }
  }
  assert.deepEqual(
    Object.keys(RULE_TEMPLATE_BUILDER_MODELS).sort(),
    FAMILIES.map(({ schema }) => schema.family).sort(),
  );
});

/// A template pick on a page whose schema read is still out: the editor the
/// pick drives, and a read the test settles.
function pickTemplate(id: string, schemaInHand: boolean) {
  const template = MAINTENANCE_RULE_TEMPLATES.find((entry) => entry.id === id)!;
  let settle: (schema: RuleBuilderSchema | null) => void = () => {};
  const read = new Promise<RuleBuilderSchema | null>((resolve) => {
    settle = resolve;
  });
  const opened: TitleDraft[] = [];
  const editor = { shown: null as TitleDraft | null };
  const done = openRuleTemplateDraft<TitleDraft>({
    schema: schemaInHand ? MAINTENANCE_CONTRACT_SCHEMA : null,
    whenSchemaReady: () => read,
    binding: maintenanceRuleBuilder,
    draft: {
      ...initialMaintenanceRuleDraft(),
      regoSource: template.regoSource,
      subjectKind: template.subjectKind ?? "TITLE",
    },
    model: (schema) => ruleTemplateBuilderModel(schema, id),
    open: (draft) => {
      opened.push(draft);
      editor.shown = draft;
    },
    isShowing: (draft) => editor.shown === draft,
  });
  return { opened, editor, done, settle };
}

type TitleDraft = ReturnType<typeof initialMaintenanceRuleDraft>;

test("with the schema in hand a template opens in the builder at once", async () => {
  const pick = pickTemplate("dead-wanted-entries", true);
  assert.equal(pick.opened.length, 1);
  assert.equal(initialRuleEditorMode(pick.opened[0]!), "simple");
  await pick.done;
  assert.equal(pick.opened.length, 1);
});

test("a template opens as Rego at once and moves to the builder when the read settles", async () => {
  const pick = pickTemplate("dead-wanted-entries", false);
  assert.equal(pick.opened.length, 1);
  assert.equal(initialRuleEditorMode(pick.opened[0]!), "edit");
  pick.settle(MAINTENANCE_CONTRACT_SCHEMA);
  await pick.done;
  assert.equal(pick.opened.length, 2);
  assert.equal(initialRuleEditorMode(pick.opened[1]!), "simple");
  assert.equal(pick.opened[1]!.regoSource, pick.opened[0]!.regoSource);
});

test("a read that settles after the operator typed keeps the typing", async () => {
  const pick = pickTemplate("dead-wanted-entries", false);
  const typed = { ...pick.opened[0]!, name: "typed while the read was out" };
  pick.editor.shown = typed;
  pick.settle(MAINTENANCE_CONTRACT_SCHEMA);
  await pick.done;
  assert.equal(pick.opened.length, 1);
  assert.equal(pick.editor.shown, typed);
});

test("a read that settles after the editor closed or opened something else opens nothing", async () => {
  const pick = pickTemplate("dead-wanted-entries", false);
  pick.editor.shown = null;
  pick.settle(MAINTENANCE_CONTRACT_SCHEMA);
  await pick.done;
  assert.equal(pick.opened.length, 1);
  assert.equal(pick.editor.shown, null);
});

test("a failed or unanswered read leaves the template open as Rego", async () => {
  const failed = pickTemplate("dead-wanted-entries", false);
  failed.settle(null);
  await failed.done;
  assert.equal(failed.opened.length, 1);
  assert.equal(initialRuleEditorMode(failed.editor.shown!), "edit");
  assert.equal(
    failed.editor.shown!.regoSource,
    MAINTENANCE_RULE_TEMPLATES.find((t) => t.id === "dead-wanted-entries")!
      .regoSource,
  );

  const unanswered = pickTemplate("dead-wanted-entries", false);
  assert.equal(unanswered.opened.length, 1);
  assert.equal(initialRuleEditorMode(unanswered.editor.shown!), "edit");
});

test("a template the builder cannot express opens once, as Rego", async () => {
  const pick = pickTemplate("four-k-purge", false);
  pick.settle(MAINTENANCE_CONTRACT_SCHEMA);
  await pick.done;
  assert.equal(pick.opened.length, 1);
  assert.equal(initialRuleEditorMode(pick.opened[0]!), "edit");
});

import assert from "node:assert/strict";
import test from "node:test";

import maintenanceFixtures from "../contracts/maintenance-builder-fixtures.json" with { type: "json" };
import requestFixtures from "../contracts/request-builder-fixtures.json" with { type: "json" };
import scoringFixtures from "../contracts/scoring-builder-fixtures.json" with { type: "json" };
import type {
  MaintenanceRuleScope,
  MaintenanceRuleSetDetail,
} from "../types/maintenance-rule-sets.ts";
import type { RequestRuleSetDetail } from "../types/request-rule-sets.ts";
import type { RuleSetRecord } from "../types/rule-sets.ts";
import {
  MAINTENANCE_RULE_BUILDER,
  MAINTENANCE_STARTER_SOURCE,
  copyMaintenanceRuleDraft,
  createMaintenanceRuleSetInput,
  initialMaintenanceRuleDraft,
  maintenanceRuleDraftFromDetail,
  newMaintenanceRuleBuilderDraft,
  updateMaintenanceRuleMatcherInput,
} from "./maintenance-rule-sets.ts";
import {
  REQUEST_RULE_BUILDER,
  REQUEST_STARTER_SOURCE,
  copyRequestRuleDraft,
  createRequestRuleSetInput,
  initialRequestRuleDraft,
  newRequestRuleBuilderDraft,
  requestRuleDraftFromDetail,
  updateRequestRuleMatcherInput,
} from "./request-rule-sets.ts";
import {
  emptyRuleBuilderModel,
  parseRuleBuilderModel,
  type RuleBuilderModel,
} from "./rule-builder.ts";
import {
  initialRuleEditorMode,
  isIncompleteRuleBuilderDraft,
  nextRuleEditorMode,
  ruleBuilderModelToSave,
  ruleRegoEditorNeedsConfirmation,
  withRegeneratedRuleSource,
  withRuleBuilderModel,
  withRuleRegoEditor,
  withRuleSourceEdit,
  type RuleBuilderBinding,
  type RuleBuilderDraft,
} from "./rule-builder-mode.ts";
import {
  RULE_SET_INITIAL_DRAFT,
  SCORING_RULE_BUILDER,
  SCORING_STARTER_SOURCE,
  copyRuleSetDraft,
  createRuleSetInput,
  newScoringRuleBuilderDraft,
  ruleSetDraftFromRecord,
  updateRuleSetInput,
} from "./rule-sets.ts";

const PREAMBLE = "package rules\nimport rego.v1\n\n";

type Fixture = { id: string; scope: string | null; model: unknown; source: string };

/// One family's draft helpers, so every family is held to the same rules.
type Family<D extends RuleBuilderDraft> = {
  name: string;
  binding: RuleBuilderBinding<D>;
  starter: string;
  fixtures: Fixture[];
  newDraft: () => D;
  handWrittenDraft: () => D;
  /// The draft a saved rule opens as.
  fromStored: (regoSource: string, builderModel: unknown) => D;
  copy: (regoSource: string, builderModel: unknown) => D;
  createModel: (draft: D) => unknown;
  updateModel: (draft: D) => unknown;
};

function maintenanceDetail(
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
      schedule: {
        kind: "INTERVAL",
        everySeconds: 8 * 60 * 60,
        timeLocal: null,
        days: null,
        expression: null,
      },
      nextRunAt: null,
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

function scoringRecord(regoSource: string, builderModel: unknown): RuleSetRecord {
  return {
    id: "rule-sample",
    name: "Sample rule",
    description: "",
    managedTagFilter: null,
    regoSource,
    enabled: true,
    priority: 0,
    appliedFacets: [],
    isManaged: false,
    managedKey: null,
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:00:00Z",
    builderModel,
  };
}

function requestDetail(regoSource: string, builderModel: unknown): RequestRuleSetDetail {
  return {
    ruleSet: {
      id: "rule-sample",
      name: "sample_rule",
      description: null,
      libraryIds: [],
    },
    revision: { id: "rev-sample", ruleSetId: "rule-sample", regoSource },
    builderModel,
  } as unknown as RequestRuleSetDetail;
}

const maintenance: Family<ReturnType<typeof initialMaintenanceRuleDraft>> = {
  name: "maintenance",
  binding: MAINTENANCE_RULE_BUILDER,
  starter: MAINTENANCE_STARTER_SOURCE,
  fixtures: (maintenanceFixtures as Fixture[]).filter((entry) => entry.scope === "TITLE"),
  newDraft: newMaintenanceRuleBuilderDraft,
  handWrittenDraft: initialMaintenanceRuleDraft,
  fromStored: (source, model) => maintenanceRuleDraftFromDetail(maintenanceDetail(source, model)),
  copy: (source, model) => copyMaintenanceRuleDraft(maintenanceDetail(source, model)),
  createModel: (draft) => createMaintenanceRuleSetInput(draft).builderModel,
  updateModel: (draft) => updateMaintenanceRuleMatcherInput("rule-sample", draft).builderModel,
};

const scoring: Family<typeof RULE_SET_INITIAL_DRAFT> = {
  name: "scoring",
  binding: SCORING_RULE_BUILDER,
  starter: SCORING_STARTER_SOURCE,
  fixtures: scoringFixtures as Fixture[],
  newDraft: newScoringRuleBuilderDraft,
  handWrittenDraft: () => ({ ...RULE_SET_INITIAL_DRAFT }),
  fromStored: (source, model) => ruleSetDraftFromRecord(scoringRecord(source, model)),
  copy: (source, model) => copyRuleSetDraft(scoringRecord(source, model)),
  createModel: (draft) => createRuleSetInput(draft).builderModel,
  updateModel: (draft) => updateRuleSetInput("rule-sample", draft).builderModel,
};

const request: Family<ReturnType<typeof initialRequestRuleDraft>> = {
  name: "request",
  binding: REQUEST_RULE_BUILDER,
  starter: REQUEST_STARTER_SOURCE,
  fixtures: requestFixtures as Fixture[],
  newDraft: newRequestRuleBuilderDraft,
  handWrittenDraft: initialRequestRuleDraft,
  fromStored: (source, model) => requestRuleDraftFromDetail(requestDetail(source, model)),
  copy: (source, model) => copyRequestRuleDraft(requestDetail(source, model)),
  createModel: (draft) => createRequestRuleSetInput(draft).builderModel,
  updateModel: (draft) => updateRequestRuleMatcherInput("rule-sample", draft).builderModel,
};

function modelOf<D extends RuleBuilderDraft>(family: Family<D>, entry: Fixture): RuleBuilderModel {
  const model = parseRuleBuilderModel(family.binding.schema, entry.model);
  assert.ok(model, entry.id);
  return model;
}

function suite<D extends RuleBuilderDraft>(family: Family<D>) {
  const [first, second] = family.fixtures;

  test(`${family.name}: a new rule opens in Simple with nothing to save, a hand-written one in Advanced edit`, () => {
    const draft = family.newDraft();
    assert.deepEqual(draft.builderModel, emptyRuleBuilderModel(family.binding.schema));
    assert.equal(draft.regoSource, "");
    assert.equal(initialRuleEditorMode(draft), "simple");
    assert.equal(isIncompleteRuleBuilderDraft(family.binding, draft), true);
    assert.equal(ruleBuilderModelToSave(family.binding, draft), null);

    const handWritten = family.handWrittenDraft();
    assert.equal(handWritten.builderModel, null);
    assert.equal(handWritten.regoSource, family.starter);
    assert.equal(initialRuleEditorMode(handWritten), "edit");
    assert.equal(isIncompleteRuleBuilderDraft(family.binding, handWritten), false);
  });

  test(`${family.name}: a builder change writes the generated source and saves the model`, () => {
    const model = modelOf(family, first);
    const draft = withRuleBuilderModel(family.binding, family.newDraft(), model);
    assert.equal(draft.regoSource, first.source);
    assert.deepEqual(ruleBuilderModelToSave(family.binding, draft), model);
    assert.deepEqual(family.createModel(draft), model);
    assert.deepEqual(family.updateModel(draft), model);
    assert.equal(isIncompleteRuleBuilderDraft(family.binding, draft), false);

    const unfinished = withRuleBuilderModel(family.binding, draft, { ...model, outputs: {} });
    assert.equal(unfinished.regoSource, "");
    assert.equal(ruleBuilderModelToSave(family.binding, unfinished), null);
    assert.equal(isIncompleteRuleBuilderDraft(family.binding, unfinished), true);
  });

  test(`${family.name}: an attached draft moves between Simple and Advanced view and keeps its model`, () => {
    const model = modelOf(family, first);
    const attached = withRuleBuilderModel(family.binding, family.newDraft(), model);
    assert.equal(nextRuleEditorMode(attached, "simple", "view"), "view");
    assert.equal(nextRuleEditorMode(attached, "view", "simple"), "simple");
    // Neither move touches the draft, so the model and source are what Simple left.
    assert.deepEqual(attached.builderModel, model);
    assert.equal(attached.regoSource, first.source);
    assert.deepEqual(family.updateModel(attached), model);
  });

  test(`${family.name}: the Rego of an attached draft cannot be edited`, () => {
    const attached = withRuleBuilderModel(family.binding, family.newDraft(), modelOf(family, first));
    assert.equal(withRuleSourceEdit(attached, `${first.source}# note\n`), attached);
  });

  test(`${family.name}: entering edit detaches the draft even with its source unchanged`, () => {
    const model = modelOf(family, first);
    const attached = withRuleBuilderModel(family.binding, family.newDraft(), model);
    assert.equal(ruleRegoEditorNeedsConfirmation(attached), true);

    const editing = withRuleRegoEditor(family.binding, attached);
    assert.equal(editing.builderModel, null);
    assert.equal(editing.regoSource, first.source);
    assert.equal(ruleBuilderModelToSave(family.binding, editing), null);
    assert.equal(family.createModel(editing), null);
    assert.equal(family.updateModel(editing), null);

    // From edit there is no way back to the builder.
    assert.equal(nextRuleEditorMode(editing, "edit", "simple"), "edit");
    assert.equal(nextRuleEditorMode(editing, "edit", "view"), "edit");
    assert.equal(nextRuleEditorMode(attached, "edit", "simple"), "edit");

    const edited = withRuleSourceEdit(editing, `${first.source}# note\n`);
    assert.equal(edited.regoSource, `${first.source}# note\n`);
    assert.equal(edited.builderModel, null);
    // Typing the generated source back does not reattach it.
    const retyped = withRuleSourceEdit(edited, first.source);
    assert.equal(retyped.builderModel, null);
    assert.equal(family.updateModel(retyped), null);
  });

  test(`${family.name}: an untouched builder enters edit without confirming, from the starter`, () => {
    const blank = family.newDraft();
    assert.equal(ruleRegoEditorNeedsConfirmation(blank), false);
    const editing = withRuleRegoEditor(family.binding, blank);
    assert.equal(editing.regoSource, family.starter);
    assert.equal(editing.builderModel, null);

    const handWritten = family.handWrittenDraft();
    assert.equal(ruleRegoEditorNeedsConfirmation(handWritten), false);
    assert.equal(withRuleRegoEditor(family.binding, handWritten), handWritten);
  });

  test(`${family.name}: a rule saved without a model opens in edit even when its source is the builder's`, () => {
    const reopened = family.fromStored(first.source, null);
    assert.equal(reopened.builderModel, null);
    assert.equal(initialRuleEditorMode(reopened), "edit");
    assert.equal(nextRuleEditorMode(reopened, "edit", "simple"), "edit");
    assert.equal(nextRuleEditorMode(reopened, "edit", "view"), "edit");
    assert.equal(family.updateModel(reopened), null);
  });

  test(`${family.name}: a stored model opens in Simple only while it regenerates the stored source`, () => {
    const model = modelOf(family, first);

    const exact = family.fromStored(first.source, first.model);
    assert.deepEqual(exact.builderModel, model);
    assert.equal(initialRuleEditorMode(exact), "simple");

    // The API hands stored sources back without the package and import lines.
    const stripped = first.source.slice(PREAMBLE.length);
    const served = family.fromStored(stripped, first.model);
    assert.deepEqual(served.builderModel, model);
    assert.equal(served.regoSource, stripped);
    assert.deepEqual(family.updateModel(served), model);

    const handEdited = family.fromStored(`${first.source}\n# note\n`, first.model);
    assert.equal(handEdited.builderModel, null);
    assert.equal(initialRuleEditorMode(handEdited), "edit");

    const stale = family.fromStored(second.source, first.model);
    assert.equal(stale.builderModel, null);
    assert.equal(stale.regoSource, second.source);

    for (const stored of [null, undefined, { version: 9 }, { ...model, family: "other" }])
      assert.equal(family.fromStored(first.source, stored).builderModel, null);
  });

  test(`${family.name}: a copy of a builder rule stays in the builder`, () => {
    const copy = family.copy(first.source, first.model);
    assert.deepEqual(copy.builderModel, modelOf(family, first));
    assert.equal(copy.regoSource, first.source);
    assert.deepEqual(family.createModel(copy), modelOf(family, first));
    assert.equal(family.copy(`${first.source}# note\n`, first.model).builderModel, null);
    assert.equal(family.copy(first.source, null).builderModel, null);
  });
}

suite(maintenance);
suite(scoring);
suite(request);

test("maintenance: a scope change rewrites an attached rule for the new scope", () => {
  const entry = (maintenanceFixtures as Fixture[]).find(
    (candidate) => candidate.id === "title-delete-title",
  );
  assert.ok(entry);
  const model = modelOf(maintenance, entry);
  const attached = withRuleBuilderModel(
    MAINTENANCE_RULE_BUILDER,
    newMaintenanceRuleBuilderDraft(),
    model,
  );
  assert.equal(attached.regoSource, entry.source);

  const episode = withRegeneratedRuleSource(MAINTENANCE_RULE_BUILDER, {
    ...attached,
    subjectKind: "EPISODE",
  });
  assert.equal(episode.regoSource, "");
  assert.deepEqual(episode.builderModel, model);
  assert.equal(isIncompleteRuleBuilderDraft(MAINTENANCE_RULE_BUILDER, episode), true);

  // A rule stored for one scope does not open in the builder under another.
  assert.equal(
    maintenanceRuleDraftFromDetail(maintenanceDetail(entry.source, entry.model, "EPISODE"))
      .builderModel,
    null,
  );

  const detached = { ...attached, builderModel: null };
  assert.equal(withRegeneratedRuleSource(MAINTENANCE_RULE_BUILDER, detached), detached);
});

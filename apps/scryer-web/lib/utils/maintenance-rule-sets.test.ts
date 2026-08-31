import assert from "node:assert/strict";
import test from "node:test";

import maintenanceInputContract from "../contracts/maintenance-input-contract.json" with { type: "json" };
import en from "../i18n/locales/en.ts";
import type {
  MaintenanceActionDescriptor,
  MaintenanceRuleSetDetail,
} from "../types/maintenance-rule-sets.ts";
import {
  MAINTENANCE_PREVIEW_LIMIT_DEFAULT,
  MAINTENANCE_PREVIEW_LIMIT_MAX,
  MAINTENANCE_STARTER_SOURCE,
  actionKindLabelKey,
  actionRequiresTargetQualityProfile,
  clampMaintenancePreviewLimit,
  copyMaintenanceRuleDraft,
  createMaintenanceRuleSetInput,
  initialMaintenanceRuleDraft,
  maintenancePreviewInput,
  maintenanceRuleDraftFromDetail,
  riskClassBadgeTone,
  titleScopedActionDescriptors,
  updateMaintenanceRuleMatcherInput,
  updateMaintenanceRuleMetadataInput,
} from "./maintenance-rule-sets.ts";

const descriptors: MaintenanceActionDescriptor[] = [
  {
    kind: "DO_NOTHING",
    supportedSubjects: ["MOVIE", "SHOW", "SEASON", "EPISODE"],
    riskClass: "NONE",
    effectClasses: [],
    timingMode: "IMMEDIATE",
    allowedRepeatModes: ["ONCE"],
    requiresTargetQualityProfile: false,
  },
  {
    kind: "DELETE_TITLE_AND_FILES",
    supportedSubjects: ["MOVIE", "SHOW"],
    riskClass: "HIGH",
    effectClasses: ["DELETE_FILES"],
    timingMode: "GRACE",
    allowedRepeatModes: ["ONCE"],
    requiresTargetQualityProfile: false,
  },
  {
    kind: "UNMONITOR_SEASON_THEN_UNMONITOR_SHOW_IF_EMPTY",
    supportedSubjects: ["SEASON"],
    riskClass: "LOW",
    effectClasses: ["UNMONITOR"],
    timingMode: "GRACE",
    allowedRepeatModes: ["ONCE"],
    requiresTargetQualityProfile: false,
  },
  {
    kind: "CHANGE_QUALITY_PROFILE_AND_SEARCH_IF_CHANGED",
    supportedSubjects: ["MOVIE", "SHOW"],
    riskClass: "MEDIUM",
    effectClasses: ["SEARCH"],
    timingMode: "GRACE",
    allowedRepeatModes: ["EVERY_RUN"],
    requiresTargetQualityProfile: true,
  },
];

const detail: MaintenanceRuleSetDetail = {
  ruleSet: {
    id: "rule-1",
    name: "Stale unmonitored titles",
    description: "Unmonitored titles that still hold files.",
    enabled: false,
    evaluationMode: "DISABLED",
    libraryIds: ["lib-movies", "lib-series"],
    subjectKind: "TITLE",
    currentRevisionNumber: 3,
    graceDays: 30,
    actionSpec: {
      kind: "CHANGE_QUALITY_PROFILE_AND_SEARCH_IF_CHANGED",
      schemaVersion: 1,
      targetQualityProfileId: "profile-uhd",
    },
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-02T00:00:00Z",
  },
  revision: {
    id: "rev-3",
    ruleSetId: "rule-1",
    revisionNumber: 3,
    regoSource: "match if {\n\tinput.facts.has_file.value\n}\n",
    graceDays: 14,
    matcherContentHash: "blake3:abcdef",
    createdBy: "operator",
    createdAt: "2026-01-02T00:00:00Z",
  },
  actionSpec: {
    kind: "CHANGE_QUALITY_PROFILE_AND_SEARCH_IF_CHANGED",
    schemaVersion: 1,
    targetQualityProfileId: "profile-7",
  },
};

test("a new draft starts from the starter matcher with no package or import line", () => {
  const draft = initialMaintenanceRuleDraft();

  assert.equal(draft.regoSource, MAINTENANCE_STARTER_SOURCE);
  assert.equal(draft.regoSource.includes("package "), false);
  assert.equal(draft.regoSource.includes("import rego.v1"), false);
  assert.equal(draft.actionKind, "DO_NOTHING");
  assert.equal(draft.graceDays, 0);
  assert.deepEqual(draft.libraryIds, []);
});

test("a draft loaded from a rule set round-trips its revision and action", () => {
  const draft = maintenanceRuleDraftFromDetail(detail);

  assert.equal(draft.name, detail.ruleSet.name);
  assert.equal(draft.regoSource, detail.revision.regoSource);
  assert.equal(draft.graceDays, 14);
  assert.equal(draft.actionKind, "CHANGE_QUALITY_PROFILE_AND_SEARCH_IF_CHANGED");
  assert.equal(draft.targetQualityProfileId, "profile-7");
  assert.deepEqual(draft.libraryIds, ["lib-movies", "lib-series"]);
  assert.notEqual(draft.libraryIds, detail.ruleSet.libraryIds);
});

test("copying a rule set renames the draft and keeps nothing that identifies the original", () => {
  const draft = copyMaintenanceRuleDraft(detail);

  assert.equal(draft.name, "Copy of Stale unmonitored titles");
  assert.equal(Object.hasOwn(draft, "id"), false);
  assert.equal(Object.hasOwn(draft, "enabled"), false);
  assert.equal(Object.hasOwn(draft, "evaluationMode"), false);
});

test("create input carries the action and drops empty optional fields", () => {
  const input = createMaintenanceRuleSetInput(
    { ...maintenanceRuleDraftFromDetail(detail), description: "  ", libraryIds: [] },
    descriptors,
  );

  assert.equal(input.name, "Stale unmonitored titles");
  assert.equal(input.description, undefined);
  assert.equal(input.libraryIds, undefined);
  assert.deepEqual(input.action, {
    kind: "CHANGE_QUALITY_PROFILE_AND_SEARCH_IF_CHANGED",
    targetQualityProfileId: "profile-7",
  });
  assert.equal(input.graceDays, 14);
  assert.equal(Object.hasOwn(input, "enabled"), false);
});

test("an action that does not take a profile never sends a stale profile id", () => {
  const draft = { ...maintenanceRuleDraftFromDetail(detail), actionKind: "DELETE_TITLE_AND_FILES" as const };
  const input = createMaintenanceRuleSetInput(draft, descriptors);

  assert.deepEqual(input.action, {
    kind: "DELETE_TITLE_AND_FILES",
    targetQualityProfileId: undefined,
  });
});

test("matcher and metadata updates split along the API's versioned and unversioned halves", () => {
  const draft = maintenanceRuleDraftFromDetail(detail);
  const matcher = updateMaintenanceRuleMatcherInput("rule-1", draft, descriptors);
  const metadata = updateMaintenanceRuleMetadataInput("rule-1", draft);

  assert.deepEqual(Object.keys(matcher).sort(), [
    "action",
    "graceDays",
    "id",
    "regoSource",
  ]);
  assert.deepEqual(Object.keys(metadata).sort(), [
    "description",
    "id",
    "libraryIds",
    "name",
  ]);
});

test("only descriptors that support a movie or a show are offerable for a title rule", () => {
  const offerable = titleScopedActionDescriptors(descriptors).map((d) => d.kind);

  assert.deepEqual(offerable, [
    "DO_NOTHING",
    "DELETE_TITLE_AND_FILES",
    "CHANGE_QUALITY_PROFILE_AND_SEARCH_IF_CHANGED",
  ]);
  assert.equal(
    offerable.includes("UNMONITOR_SEASON_THEN_UNMONITOR_SHOW_IF_EMPTY"),
    false,
  );
});

test("the quality-profile field follows the descriptor rather than a hardcoded kind", () => {
  assert.equal(
    actionRequiresTargetQualityProfile(
      descriptors,
      "CHANGE_QUALITY_PROFILE_AND_SEARCH_IF_CHANGED",
    ),
    true,
  );
  assert.equal(
    actionRequiresTargetQualityProfile(descriptors, "DELETE_TITLE_AND_FILES"),
    false,
  );
  assert.equal(actionRequiresTargetQualityProfile([], "DO_NOTHING"), false);
});

test("a destructive action reads as destructive", () => {
  assert.equal(riskClassBadgeTone("HIGH"), "negative");
  assert.equal(riskClassBadgeTone("MEDIUM"), "warning");
  assert.equal(riskClassBadgeTone("NONE"), "neutral");
});

test("the preview limit stays inside the API's cap", () => {
  assert.equal(clampMaintenancePreviewLimit(0), 1);
  assert.equal(clampMaintenancePreviewLimit(500), MAINTENANCE_PREVIEW_LIMIT_MAX);
  assert.equal(clampMaintenancePreviewLimit(Number.NaN), MAINTENANCE_PREVIEW_LIMIT_DEFAULT);
  assert.equal(clampMaintenancePreviewLimit(20), 20);
});

test("preview sends either a stored rule or an inline draft, never both", () => {
  const stored = maintenancePreviewInput({
    ruleSetId: "rule-1",
    draft: maintenanceRuleDraftFromDetail(detail),
    descriptors,
    libraryId: "lib-movies",
    limit: 10,
  });
  assert.deepEqual(stored, {
    ruleSetId: "rule-1",
    libraryId: "lib-movies",
    limit: 10,
  });

  const inline = maintenancePreviewInput({
    draft: maintenanceRuleDraftFromDetail(detail),
    descriptors,
    libraryId: "lib-movies",
    limit: 999,
  });
  assert.equal(Object.hasOwn(inline, "ruleSetId"), false);
  assert.equal(inline.regoSource, detail.revision.regoSource);
  assert.equal(inline.limit, MAINTENANCE_PREVIEW_LIMIT_MAX);
});

test("explicit title ids replace the library-and-limit subject selection", () => {
  const input = maintenancePreviewInput({
    ruleSetId: "rule-1",
    titleIds: ["title-1", "title-2"],
    libraryId: "lib-movies",
    limit: 20,
  });

  assert.deepEqual(input, {
    ruleSetId: "rule-1",
    titleIds: ["title-1", "title-2"],
  });
});

test("every action kind in the pinned enum has a label", () => {
  const kinds = [
    "DO_NOTHING",
    "UNMONITOR_SCOPE_KEEP_FILES",
    "DELETE_TITLE_AND_FILES",
    "UNMONITOR_TITLE_DELETE_ALL_FILES",
    "UNMONITOR_SHOW_DELETE_EXISTING_FILES",
    "UNMONITOR_SCOPE_DELETE_FILES",
    "UNMONITOR_SEASON_DELETE_FILES_THEN_DELETE_SHOW_IF_EMPTY",
    "UNMONITOR_SEASON_THEN_UNMONITOR_SHOW_IF_EMPTY",
    "CHANGE_QUALITY_PROFILE_AND_SEARCH_IF_CHANGED",
  ];

  for (const kind of kinds) {
    const key = actionKindLabelKey(kind);
    assert.ok(key, `${kind} has no label key`);
    assert.equal(typeof en[key], "string", `${key} is not in the default locale`);
  }
  assert.equal(actionKindLabelKey("SOME_FUTURE_KIND"), null);
});

test("the maintenance reference table documents the observation envelope", () => {
  const monitored = maintenanceInputContract.sections.find(
    (section) => section.path === "input.facts.monitored",
  );

  assert.deepEqual(
    monitored?.fields.map((field) => field.field),
    ["status", "value", "observed_at", "reason"],
  );
});

test("every contract title and description key resolves in the default locale", () => {
  const missing: string[] = [];
  for (const section of maintenanceInputContract.sections) {
    if (typeof en[section.titleKey] !== "string") {
      missing.push(`${section.path} -> ${section.titleKey}`);
    }
    for (const field of section.fields) {
      if (typeof en[field.descKey] !== "string") {
        missing.push(`${section.path}.${field.field} -> ${field.descKey}`);
      }
    }
  }

  assert.deepEqual(missing, []);
});

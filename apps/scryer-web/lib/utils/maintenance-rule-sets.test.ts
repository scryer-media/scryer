import assert from "node:assert/strict";
import test from "node:test";

import maintenanceInputContract from "../contracts/maintenance-input-contract.json" with { type: "json" };
import en from "../i18n/locales/en.ts";
import type {
  MaintenanceActionDescriptor,
  MaintenanceCandidateState,
  MaintenanceEffectArming,
  MaintenanceInstanceGates,
  MaintenanceRuleSetDetail,
} from "../types/maintenance-rule-sets.ts";
import {
  MAINTENANCE_FILTER_ALL,
  MAINTENANCE_GATE_ORDER,
  MAINTENANCE_PREVIEW_LIMIT_DEFAULT,
  MAINTENANCE_PREVIEW_LIMIT_MAX,
  MAINTENANCE_STARTER_SOURCE,
  actionKindLabelKey,
  actionRequiresTargetQualityProfile,
  armingOptionsFor,
  candidateStateBadgeTone,
  candidateStateLabelKey,
  clampMaintenancePreviewLimit,
  copyMaintenanceRuleDraft,
  createMaintenanceRuleSetInput,
  destructiveArmingOfferable,
  effectArmingBadgeTone,
  effectArmingLabelKey,
  evaluationModeHelpKey,
  evaluationModeLabelKey,
  excludeMaintenanceSubjectInput,
  gateHelpKey,
  gateLabelKey,
  initialMaintenanceRuleDraft,
  isNonTerminalCandidateState,
  maintenanceCountdown,
  maintenanceFilterArgument,
  maintenancePreviewInput,
  maintenanceRuleDraftFromDetail,
  maintenanceStatusBanner,
  maintenanceStatusBannerKeys,
  nonTerminalCandidateCount,
  parseAcknowledgedCandidateCountMismatch,
  riskClassBadgeTone,
  runStatusBadgeTone,
  runStatusLabelKey,
  setMaintenanceRuleArmingInput,
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
    effectArming: "NONE",
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
    regoSource: "match if {\n\tinput.facts.has_file\n}\n",
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

test("the maintenance reference table documents bare facts and full envelopes", () => {
  // The simple surface: a fact is its value, with no envelope fields to wade
  // through.
  const facts = maintenanceInputContract.sections.find(
    (section) => section.path === "input.facts",
  );
  const monitored = facts?.fields.find((field) => field.field === "monitored");
  assert.equal(monitored?.type, "bool");

  // The advanced surface documents the envelope, under its own namespace.
  const observed = maintenanceInputContract.sections.find(
    (section) => section.path === "input.observations.monitored",
  );
  assert.deepEqual(
    observed?.fields.map((field) => field.field),
    ["status", "value", "observed_at", "reason"],
  );
});

// ── Operating a rule ──────────────────────────────────────────────────

const allGatesOff: MaintenanceInstanceGates = {
  evaluationEnabled: false,
  resultDisplayEnabled: false,
  presentationEffectsEnabled: false,
  reversibleEffectsEnabled: false,
  destructiveEffectsEnabled: false,
};

const activeRule = { evaluationMode: "OBSERVE" } as const;
const disabledRule = { evaluationMode: "DISABLED" } as const;

test("a reader who cannot see the gates is told so rather than told nothing runs", () => {
  const banner = maintenanceStatusBanner(null, [activeRule]);

  assert.equal(banner.variant, "gatesUnknown");
  assert.equal(banner.tone, "info");
});

test("the evaluation gate outranks every other reason the page might be quiet", () => {
  assert.equal(
    maintenanceStatusBanner(allGatesOff, [activeRule]).variant,
    "evaluationDisabled",
  );
});

test("evaluation on with nothing but disabled rules reads as no active rules", () => {
  const gates = { ...allGatesOff, evaluationEnabled: true };

  assert.equal(
    maintenanceStatusBanner(gates, [disabledRule]).variant,
    "noActiveRules",
  );
  assert.equal(maintenanceStatusBanner(gates, []).variant, "noActiveRules");
});

test("an evaluating instance with every effect gate shut says so plainly", () => {
  const banner = maintenanceStatusBanner(
    { ...allGatesOff, evaluationEnabled: true },
    [activeRule],
  );

  assert.equal(banner.variant, "effectsDisabled");
  assert.equal(banner.tone, "info");
});

test("the destructive gate is the only banner that raises its voice", () => {
  const reversible = maintenanceStatusBanner(
    { ...allGatesOff, evaluationEnabled: true, reversibleEffectsEnabled: true },
    [activeRule],
  );
  const destructive = maintenanceStatusBanner(
    {
      ...allGatesOff,
      evaluationEnabled: true,
      reversibleEffectsEnabled: true,
      destructiveEffectsEnabled: true,
    },
    [activeRule],
  );

  assert.equal(reversible.variant, "reversibleArmed");
  assert.equal(reversible.tone, "info");
  assert.equal(destructive.variant, "destructiveArmed");
  assert.equal(destructive.tone, "warning");
});

test("every banner variant and gate has real copy in the default locale", () => {
  const variants = [
    "gatesUnknown",
    "evaluationDisabled",
    "noActiveRules",
    "effectsDisabled",
    "reversibleArmed",
    "destructiveArmed",
  ] as const;

  for (const variant of variants) {
    const { titleKey, bodyKey } = maintenanceStatusBannerKeys(variant);
    assert.equal(typeof en[titleKey], "string", `${titleKey} is missing`);
    assert.equal(typeof en[bodyKey], "string", `${bodyKey} is missing`);
  }

  assert.equal(MAINTENANCE_GATE_ORDER.length, 5);
  for (const gate of MAINTENANCE_GATE_ORDER) {
    assert.equal(typeof en[gateLabelKey(gate)], "string", `${gate} has no label`);
    assert.equal(typeof en[gateHelpKey(gate)], "string", `${gate} has no help`);
  }
});

test("destructive arming is only offered for an action that actually deletes", () => {
  assert.equal(
    destructiveArmingOfferable(descriptors, "DELETE_TITLE_AND_FILES"),
    true,
  );
  assert.equal(
    destructiveArmingOfferable(
      descriptors,
      "CHANGE_QUALITY_PROFILE_AND_SEARCH_IF_CHANGED",
    ),
    false,
  );
  assert.deepEqual(armingOptionsFor(descriptors, "DO_NOTHING"), [
    "NONE",
    "REVERSIBLE",
  ]);
  assert.deepEqual(armingOptionsFor(descriptors, "DELETE_TITLE_AND_FILES"), [
    "NONE",
    "REVERSIBLE",
    "DESTRUCTIVE",
  ]);
  /// An action kind this build has no descriptor for never offers the
  /// destructive rung.
  assert.deepEqual(armingOptionsFor([], "DELETE_TITLE_AND_FILES"), [
    "NONE",
    "REVERSIBLE",
  ]);
});

test("arming reads at a glance, with destructive the only destructive-toned rung", () => {
  const armings: MaintenanceEffectArming[] = ["NONE", "REVERSIBLE", "DESTRUCTIVE"];

  assert.equal(effectArmingBadgeTone("DESTRUCTIVE"), "negative");
  assert.equal(effectArmingBadgeTone("REVERSIBLE"), "warning");
  assert.equal(effectArmingBadgeTone("NONE"), "neutral");
  for (const arming of armings) {
    const key = effectArmingLabelKey(arming);
    assert.ok(key, `${arming} has no label key`);
    assert.equal(typeof en[key], "string", `${key} is not in the default locale`);
  }
  assert.equal(effectArmingLabelKey("SOME_FUTURE_ARMING"), null);
});

test("every evaluation mode carries a label and an explanation", () => {
  for (const mode of ["DISABLED", "SHADOW", "OBSERVE"]) {
    const labelKey = evaluationModeLabelKey(mode);
    const helpKey = evaluationModeHelpKey(mode);
    assert.ok(labelKey, `${mode} has no label key`);
    assert.ok(helpKey, `${mode} has no help key`);
    assert.equal(typeof en[labelKey], "string", `${labelKey} is missing`);
    assert.equal(typeof en[helpKey], "string", `${helpKey} is missing`);
  }
  assert.equal(evaluationModeHelpKey("SOME_FUTURE_MODE"), null);
});

test("only a destructive arming carries an acknowledgement", () => {
  assert.deepEqual(setMaintenanceRuleArmingInput("rule-1", "REVERSIBLE", 7), {
    id: "rule-1",
    arming: "REVERSIBLE",
    acknowledgedCandidateCount: undefined,
  });
  assert.deepEqual(setMaintenanceRuleArmingInput("rule-1", "DESTRUCTIVE", 7), {
    id: "rule-1",
    arming: "DESTRUCTIVE",
    acknowledgedCandidateCount: 7,
  });
});

test("the count-mismatch message shape the dialog re-asks against is pinned", () => {
  const message =
    "destructive arming requires acknowledging the current candidate count (12)";

  assert.equal(parseAcknowledgedCandidateCountMismatch(message), 12);
  assert.equal(
    parseAcknowledgedCandidateCountMismatch(`[GraphQL] ${message}`),
    12,
  );
  assert.equal(parseAcknowledgedCandidateCountMismatch(message.toUpperCase()), 12);
  assert.equal(parseAcknowledgedCandidateCountMismatch("rule not found"), null);
});

// ── Candidates ────────────────────────────────────────────────────────

test("every candidate state has a label and a tone", () => {
  const states: MaintenanceCandidateState[] = [
    "OBSERVING",
    "PENDING_ACTION",
    "DUE",
    "EXECUTING",
    "SUCCEEDED",
    "FAILED",
    "CANCELED",
    "EXCLUDED",
    "BLOCKED",
  ];

  for (const state of states) {
    const key = candidateStateLabelKey(state);
    assert.ok(key, `${state} has no label key`);
    assert.equal(typeof en[key], "string", `${key} is not in the default locale`);
  }
  assert.equal(candidateStateLabelKey("SOME_FUTURE_STATE"), null);

  assert.equal(candidateStateBadgeTone("OBSERVING"), "neutral");
  assert.equal(candidateStateBadgeTone("PENDING_ACTION"), "info");
  assert.equal(candidateStateBadgeTone("DUE"), "info");
  assert.equal(candidateStateBadgeTone("EXECUTING"), "info");
  assert.equal(candidateStateBadgeTone("SUCCEEDED"), "positive");
  assert.equal(candidateStateBadgeTone("FAILED"), "negative");
  assert.equal(candidateStateBadgeTone("CANCELED"), "neutral");
  assert.equal(candidateStateBadgeTone("EXCLUDED"), "neutral");
  assert.equal(candidateStateBadgeTone("BLOCKED"), "warning");
  assert.equal(candidateStateBadgeTone("SOME_FUTURE_STATE"), "neutral");
});

test("only the states an armed handler could still act on are acknowledged", () => {
  const candidates = [
    { state: "OBSERVING" as const },
    { state: "PENDING_ACTION" as const },
    { state: "DUE" as const },
    { state: "EXECUTING" as const },
    { state: "BLOCKED" as const },
    { state: "SUCCEEDED" as const },
    { state: "FAILED" as const },
    { state: "CANCELED" as const },
    { state: "EXCLUDED" as const },
  ];

  assert.equal(nonTerminalCandidateCount(candidates), 5);
  assert.equal(isNonTerminalCandidateState("SUCCEEDED"), false);
  assert.equal(isNonTerminalCandidateState("BLOCKED"), true);
});

test("the due countdown picks its unit by magnitude and names overdue as overdue", () => {
  const now = Date.parse("2026-03-01T00:00:00Z");
  const at = (offsetMs: number) => new Date(now + offsetMs).toISOString();

  assert.deepEqual(maintenanceCountdown(at(3 * 24 * 60 * 60_000 + 1000), now), {
    overdue: false,
    labelKey: "settings.maintenanceCountdownInDays",
    values: { count: 3 },
  });
  assert.deepEqual(maintenanceCountdown(at(5 * 60 * 60_000), now), {
    overdue: false,
    labelKey: "settings.maintenanceCountdownInHours",
    values: { count: 5 },
  });
  assert.deepEqual(maintenanceCountdown(at(90_000), now), {
    overdue: false,
    labelKey: "settings.maintenanceCountdownInMinutes",
    values: { count: 1 },
  });
  assert.deepEqual(maintenanceCountdown(at(-2 * 24 * 60 * 60_000), now), {
    overdue: true,
    labelKey: "settings.maintenanceCountdownOverdueDays",
    values: { count: 2 },
  });
  assert.deepEqual(maintenanceCountdown(at(-30_000), now), {
    overdue: true,
    labelKey: "settings.maintenanceCountdownDueNow",
    values: { count: 0 },
  });
  assert.equal(maintenanceCountdown("not a date", now), null);
});

test("every countdown phrasing exists in the default locale", () => {
  const keys = [
    "settings.maintenanceCountdownDueNow",
    "settings.maintenanceCountdownInMinutes",
    "settings.maintenanceCountdownInHours",
    "settings.maintenanceCountdownInDays",
    "settings.maintenanceCountdownOverdueMinutes",
    "settings.maintenanceCountdownOverdueHours",
    "settings.maintenanceCountdownOverdueDays",
  ];

  for (const key of keys) {
    assert.equal(typeof en[key], "string", `${key} is not in the default locale`);
  }
});

test("run statuses fall back to the raw value the API sent", () => {
  for (const status of [
    "running",
    "succeeded",
    "failed",
    "held",
    "already_satisfied",
    "skipped",
  ]) {
    const key = runStatusLabelKey(status);
    assert.ok(key, `${status} has no label key`);
    assert.equal(typeof en[key], "string", `${key} is not in the default locale`);
  }

  assert.equal(runStatusLabelKey("SUCCEEDED"), "settings.maintenanceRunStatusSucceeded");
  assert.equal(runStatusLabelKey("some_future_status"), null);
  assert.equal(runStatusBadgeTone("failed"), "negative");
  assert.equal(runStatusBadgeTone("already_satisfied"), "warning");
  assert.equal(runStatusBadgeTone("some_future_status"), "neutral");
});

test("the all-filter sentinel never reaches the API as a rule or library id", () => {
  assert.equal(MAINTENANCE_FILTER_ALL, "all");
  assert.equal(maintenanceFilterArgument(MAINTENANCE_FILTER_ALL), undefined);
  assert.equal(maintenanceFilterArgument(""), undefined);
  assert.equal(maintenanceFilterArgument("rule-1"), "rule-1");
});

test("an exclusion drops the fields the API treats as absent rather than empty", () => {
  assert.deepEqual(
    excludeMaintenanceSubjectInput({
      titleId: "title-1",
      ruleSetId: "",
      reason: "   ",
    }),
    { titleId: "title-1", ruleSetId: undefined, reason: undefined },
  );
  assert.deepEqual(
    excludeMaintenanceSubjectInput({
      titleId: "title-1",
      ruleSetId: "rule-1",
      reason: " keeping this one ",
    }),
    { titleId: "title-1", ruleSetId: "rule-1", reason: "keeping this one" },
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

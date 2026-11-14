import assert from "node:assert/strict";
import test from "node:test";

import maintenanceInputContract from "../contracts/maintenance-input-contract.json" with { type: "json" };
import en from "../i18n/locales/en.ts";
import type {
  MaintenanceActionHead,
  MaintenanceActionStepProgress,
  MaintenanceCandidateState,
  MaintenanceDeclaredActions,
  MaintenanceRuleSetDetail,
  MaintenanceRuleState,
} from "../types/maintenance-rule-sets.ts";
import {
  MAINTENANCE_ACTION_HEAD_ORDER,
  MAINTENANCE_ACTION_HEAD_RULE_NAMES,
  MAINTENANCE_FILTER_ALL,
  MAINTENANCE_PREVIEW_LIMIT_DEFAULT,
  MAINTENANCE_PREVIEW_LIMIT_MAX,
  MAINTENANCE_STARTER_SOURCE,
  actionHeadLabelKey,
  actionKindLabelKey,
  candidateStateBadgeTone,
  candidateStateLabelKey,
  clampMaintenancePreviewLimit,
  copyMaintenanceRuleDraft,
  createMaintenanceRuleSetInput,
  decidedActionEntries,
  declaredActionEntries,
  excludeMaintenanceSubjectInput,
  initialMaintenanceRuleDraft,
  isDestructiveActionHead,
  isNonTerminalCandidateState,
  maintenanceCountdown,
  maintenanceFilterArgument,
  maintenancePreviewInput,
  maintenanceSearchHistoryState,
  maintenanceRuleDraftFromDetail,
  nonTerminalCandidateCount,
  parseAcknowledgedCandidateCountMismatch,
  runStatusBadgeTone,
  ruleStateHelpKey,
  ruleStateLabelKey,
  ruleStateNeedsConfirmation,
  ruleStateOptionsFor,
  runStatusLabelKey,
  setMaintenanceRuleStateVariables,
  templateActionHeads,
  updateMaintenanceRuleMatcherInput,
  updateMaintenanceRuleMetadataInput,
} from "./maintenance-rule-sets.ts";

function declared(
  overrides: Partial<MaintenanceDeclaredActions> = {},
): MaintenanceDeclaredActions {
  return {
    heads: [],
    destructive: false,
    qualityProfiles: [],
    addTags: [],
    removeTags: [],
    libraries: [],
    roots: [],
    notifications: [],
    ...overrides,
  };
}

const profileThenSearch = declared({
  heads: ["SEARCH", "SET_QUALITY_PROFILE"],
  qualityProfiles: ["Compact 1080p"],
});

const detail: MaintenanceRuleSetDetail = {
  ruleSet: {
    id: "rule-1",
    name: "Stale unmonitored titles",
    description: "Unmonitored titles that still hold files.",
    state: "DISABLED",
    enabledArming: "REVERSIBLE",
    editedSinceEnabled: false,
    systemOwned: false,
    libraryIds: ["lib-movies", "lib-series"],
    subjectKind: "TITLE",
    currentRevisionNumber: 3,
    graceDays: 30,
    declaredActions: profileThenSearch,
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-02T00:00:00Z",
  },
  revision: {
    id: "rev-3",
    ruleSetId: "rule-1",
    revisionNumber: 3,
    regoSource:
      'set_quality_profile := "Compact 1080p" if input.facts.has_file\n\nsearch if input.facts.has_file\n',
    graceDays: 14,
    storageRootId: null,
    matcherContentHash: "blake3:abcdef",
    createdBy: "operator",
    createdAt: "2026-01-02T00:00:00Z",
  },
  declaredActions: profileThenSearch,
};

const ACTION_FIELDS = ["action", "actionSequence", "actionSpec"];

function assertNoActionFields(input: object) {
  for (const field of ACTION_FIELDS)
    assert.equal(Object.hasOwn(input, field), false, field);
}

test("a new draft starts from a starter source that declares an action", () => {
  const draft = initialMaintenanceRuleDraft();

  assert.equal(draft.regoSource, MAINTENANCE_STARTER_SOURCE);
  assert.equal(draft.regoSource.includes("package "), false);
  assert.equal(draft.regoSource.includes("import rego.v1"), false);
  assert.equal(/^match\b/m.test(draft.regoSource), false);
  assert.ok(templateActionHeads(draft.regoSource).length > 0);
  assertNoActionFields(draft);
  assert.equal(draft.graceDays, 0);
  assert.deepEqual(draft.libraryIds, []);
});

test("a draft loaded from a rule set round-trips its revision", () => {
  const draft = maintenanceRuleDraftFromDetail(detail);

  assert.equal(draft.name, detail.ruleSet.name);
  assert.equal(draft.regoSource, detail.revision.regoSource);
  assert.equal(draft.graceDays, 14);
  assert.deepEqual(draft.libraryIds, ["lib-movies", "lib-series"]);
  assert.notEqual(draft.libraryIds, detail.ruleSet.libraryIds);
  assertNoActionFields(draft);
});

test("copying a rule set renames the draft and keeps nothing that identifies the original", () => {
  const draft = copyMaintenanceRuleDraft(detail);

  assert.equal(draft.name, "Copy of Stale unmonitored titles");
  assert.equal(draft.regoSource, detail.revision.regoSource);
  assert.equal(Object.hasOwn(draft, "id"), false);
  assert.equal(Object.hasOwn(draft, "enabled"), false);
  assert.equal(Object.hasOwn(draft, "evaluationMode"), false);
});

test("create input sends the source as the whole rule and no separate action", () => {
  const input = createMaintenanceRuleSetInput({
    ...maintenanceRuleDraftFromDetail(detail),
    description: "  ",
    libraryIds: [],
  });

  assert.equal(input.name, "Stale unmonitored titles");
  assert.equal(input.description, undefined);
  assert.equal(input.libraryIds, undefined);
  assert.equal(input.regoSource, detail.revision.regoSource);
  assert.equal(input.graceDays, 14);
  assertNoActionFields(input);
  assert.equal(Object.hasOwn(input, "enabled"), false);
});

test("matcher and metadata updates split along the API's versioned and unversioned halves", () => {
  const draft = maintenanceRuleDraftFromDetail(detail);
  const matcher = updateMaintenanceRuleMatcherInput("rule-1", draft);
  const metadata = updateMaintenanceRuleMetadataInput("rule-1", draft);

  assert.deepEqual(Object.keys(matcher).sort(), [
    "builderModel",
    "graceDays",
    "id",
    "regoSource",
    "storageRootId",
  ]);
  assert.deepEqual(Object.keys(metadata).sort(), [
    "description",
    "id",
    "libraryIds",
    "name",
  ]);
});

test("a selected root follows create, update, and inline preview inputs", () => {
  const draft = {
    ...maintenanceRuleDraftFromDetail(detail),
    storageRootId: "root-a",
  };

  assert.equal(createMaintenanceRuleSetInput(draft).storageRootId, "root-a");
  assert.equal(
    updateMaintenanceRuleMatcherInput("rule-1", draft).storageRootId,
    "root-a",
  );
  assert.equal(maintenancePreviewInput({ draft }).storageRootId, "root-a");
  assert.equal(
    updateMaintenanceRuleMatcherInput(
      "rule-1",
      maintenanceRuleDraftFromDetail(detail),
    ).storageRootId,
    null,
  );
});

test("the preview limit stays inside the API's cap", () => {
  assert.equal(clampMaintenancePreviewLimit(0), 1);
  assert.equal(
    clampMaintenancePreviewLimit(500),
    MAINTENANCE_PREVIEW_LIMIT_MAX,
  );
  assert.equal(
    clampMaintenancePreviewLimit(Number.NaN),
    MAINTENANCE_PREVIEW_LIMIT_DEFAULT,
  );
  assert.equal(clampMaintenancePreviewLimit(20), 20);
});

test("preview sends either a stored rule or an inline draft, never both", () => {
  const stored = maintenancePreviewInput({
    ruleSetId: "rule-1",
    draft: maintenanceRuleDraftFromDetail(detail),
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
    libraryId: "lib-movies",
    limit: 999,
  });
  assert.equal(Object.hasOwn(inline, "ruleSetId"), false);
  assert.equal(inline.regoSource, detail.revision.regoSource);
  assert.equal(inline.limit, MAINTENANCE_PREVIEW_LIMIT_MAX);
  assertNoActionFields(inline);
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

// ── Declared and decided actions ──────────────────────────────────────

test("declared actions read in execution order with the values each head carries", () => {
  const entries = declaredActionEntries(
    declared({
      heads: [
        "NOTIFY",
        "DELETE_TITLE",
        "UNMONITOR",
        "ADD_TAGS",
        "MOVE_TO_ROOT",
        "SET_QUALITY_PROFILE",
        "ADD_TAGS",
      ],
      destructive: true,
      qualityProfiles: ["Compact 1080p", "HD-720p"],
      addTags: ["needs-review"],
      roots: ["/media/archive"],
      notifications: ["Retiring this title"],
    }),
  );

  assert.deepEqual(entries, [
    { head: "ADD_TAGS", values: ["needs-review"] },
    { head: "SET_QUALITY_PROFILE", values: ["Compact 1080p", "HD-720p"] },
    { head: "MOVE_TO_ROOT", values: ["/media/archive"] },
    { head: "UNMONITOR", values: [] },
    { head: "DELETE_TITLE", values: [] },
    { head: "NOTIFY", values: ["Retiring this title"] },
  ]);
  assert.deepEqual(declaredActionEntries(null), []);
  assert.deepEqual(declaredActionEntries(declared()), []);
});

test("a subject's decided actions keep the server's order and single values", () => {
  const entries = decidedActionEntries({
    heads: ["SET_QUALITY_PROFILE", "SEARCH", "MOVE_TO_LIBRARY", "NOTIFY"],
    qualityProfile: "Compact 1080p",
    addTags: [],
    removeTags: [],
    library: "Archive",
    root: null,
    notification: "Moved to the archive",
  });

  assert.deepEqual(entries, [
    { head: "SET_QUALITY_PROFILE", values: ["Compact 1080p"] },
    { head: "SEARCH", values: [] },
    { head: "MOVE_TO_LIBRARY", values: ["Archive"] },
    { head: "NOTIFY", values: ["Moved to the archive"] },
  ]);
  // Historical candidates carry no decided actions at all.
  assert.deepEqual(decidedActionEntries(null), []);
});

test("every action head has a label, a rule name and a place in the order", () => {
  const heads: MaintenanceActionHead[] = [
    "UNMONITOR",
    "DELETE_FILES",
    "DELETE_TITLE",
    "SEARCH",
    "SET_QUALITY_PROFILE",
    "ADD_TAGS",
    "REMOVE_TAGS",
    "MONITOR",
    "MOVE_TO_LIBRARY",
    "MOVE_TO_ROOT",
    "NOTIFY",
    "FETCH_SUBTITLES",
  ];
  assert.deepEqual(
    [...MAINTENANCE_ACTION_HEAD_ORDER].sort(),
    [...heads].sort(),
  );
  for (const head of heads) {
    const key = actionHeadLabelKey(head);
    assert.ok(key, `${head} has no label key`);
    assert.equal(
      typeof en[key],
      "string",
      `${key} is not in the default locale`,
    );
    assert.equal(MAINTENANCE_ACTION_HEAD_RULE_NAMES[head], head.toLowerCase());
    assert.equal(
      isDestructiveActionHead(head),
      head === "DELETE_FILES" || head === "DELETE_TITLE",
      head,
    );
  }
  assert.equal(actionHeadLabelKey("SOME_FUTURE_HEAD"), null);
});

test("template heads come from top-level rule names, not helpers or references", () => {
  const source = [
    "package rules",
    "import rego.v1",
    "",
    "# delete_title if nothing",
    "stale if {",
    "\tnot input.facts.monitored",
    "\tsearch_count := 0",
    "}",
    "",
    'notify := "Going away" if stale',
    "unmonitor if stale",
    "delete_files if stale",
    "matched_before if unmonitor",
    "",
  ].join("\n");

  assert.deepEqual(templateActionHeads(source), [
    "UNMONITOR",
    "DELETE_FILES",
    "NOTIFY",
  ]);
  assert.deepEqual(templateActionHeads("match if { true }\n"), []);
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
    assert.equal(
      typeof en[key],
      "string",
      `${key} is not in the default locale`,
    );
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

const reviewedDeleteRule = {
  id: "rule-1",
  declaredActions: declared({ heads: ["DELETE_TITLE"], destructive: true }),
  currentRevisionNumber: 3,
  libraryIds: ["library-a"],
};

test("enabled is offered only for a rule whose action changes something", () => {
  assert.deepEqual(ruleStateOptionsFor({ enabledArming: "NONE" }), [
    "DISABLED",
    "DRY_RUN",
  ]);
  for (const enabledArming of ["REVERSIBLE", "DESTRUCTIVE"] as const) {
    assert.deepEqual(ruleStateOptionsFor({ enabledArming }), [
      "DISABLED",
      "DRY_RUN",
      "ENABLED",
    ]);
  }
});

test("only enabling a rule whose source declares a delete asks for confirmation", () => {
  const states: MaintenanceRuleState[] = ["DISABLED", "DRY_RUN", "ENABLED"];
  // The newer heads change things but never delete, so they never confirm.
  const reversible = declared({
    heads: ["MOVE_TO_LIBRARY", "MOVE_TO_ROOT", "NOTIFY", "FETCH_SUBTITLES"],
    libraries: ["Archive"],
    roots: ["/media/archive"],
    notifications: ["Moved"],
  });
  for (const state of states) {
    assert.equal(
      ruleStateNeedsConfirmation({ declaredActions: reversible }, state),
      false,
    );
    for (const head of ["DELETE_FILES", "DELETE_TITLE"] as const) {
      assert.equal(
        ruleStateNeedsConfirmation(
          { declaredActions: declared({ heads: [head], destructive: true }) },
          state,
        ),
        state === "ENABLED",
        `${head} ${state}`,
      );
    }
  }
});

test("a state change carries the reviewed revision, scope and count only when it confirms", () => {
  assert.deepEqual(
    setMaintenanceRuleStateVariables(
      { ...reviewedDeleteRule, declaredActions: profileThenSearch },
      "ENABLED",
      7,
    ),
    { ruleSetId: "rule-1", state: "ENABLED", confirmation: undefined },
  );
  assert.equal(
    setMaintenanceRuleStateVariables(reviewedDeleteRule, "DRY_RUN", 7)
      .confirmation,
    undefined,
  );
  const variables = setMaintenanceRuleStateVariables(
    reviewedDeleteRule,
    "ENABLED",
    7,
  );
  reviewedDeleteRule.libraryIds.push("library-b");
  assert.deepEqual(variables, {
    ruleSetId: "rule-1",
    state: "ENABLED",
    confirmation: {
      revisionNumber: 3,
      libraryIds: ["library-a"],
      candidateCount: 7,
    },
  });
  reviewedDeleteRule.libraryIds.pop();
});

test("a global delete rule confirms an explicit empty scope", () => {
  const variables = setMaintenanceRuleStateVariables(
    { ...reviewedDeleteRule, libraryIds: [] },
    "ENABLED",
    0,
  );
  assert.deepEqual(variables.confirmation, {
    revisionNumber: 3,
    libraryIds: [],
    candidateCount: 0,
  });
});

test("every rule state has real copy in the default locale", () => {
  for (const state of ["DISABLED", "DRY_RUN", "ENABLED"]) {
    for (const key of [ruleStateLabelKey(state), ruleStateHelpKey(state)]) {
      assert.ok(key, `${state} has no key`);
      assert.equal(typeof en[key], "string", `${key} is missing`);
    }
  }
  assert.equal(en["settings.maintenanceRuleStateDisabled"], "Disabled");
  assert.equal(en["settings.maintenanceRuleStateDryRun"], "Dry run");
  assert.equal(en["settings.maintenanceRuleStateEnabled"], "Enabled");
  assert.equal(ruleStateLabelKey("SOME_FUTURE_STATE"), null);
});

test("count acknowledgement reads a numeric fact and ignores prose", () => {
  const error = (
    value: unknown,
    reason = "MAINTENANCE_ARM_COUNT_MISMATCH",
  ) => ({
    graphQLErrors: [
      {
        message: "any words (999)",
        extensions: { code: "VALIDATION_ERROR", reason, currentCount: value },
      },
    ],
  });
  assert.equal(parseAcknowledgedCandidateCountMismatch(error(12)), 12);
  assert.equal(parseAcknowledgedCandidateCountMismatch(error(0)), 0);
  for (const value of [-1, 1.5, "12", Number.NaN, Number.POSITIVE_INFINITY])
    assert.equal(parseAcknowledgedCandidateCountMismatch(error(value)), null);
  assert.equal(
    parseAcknowledgedCandidateCountMismatch(error(12, "OTHER")),
    null,
  );
  assert.equal(
    parseAcknowledgedCandidateCountMismatch(
      "acknowledging the current candidate count (12)",
    ),
    null,
  );
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
    assert.equal(
      typeof en[key],
      "string",
      `${key} is not in the default locale`,
    );
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
    assert.equal(
      typeof en[key],
      "string",
      `${key} is not in the default locale`,
    );
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
    assert.equal(
      typeof en[key],
      "string",
      `${key} is not in the default locale`,
    );
  }

  assert.equal(
    runStatusLabelKey("SUCCEEDED"),
    "settings.maintenanceRunStatusSucceeded",
  );
  assert.equal(runStatusLabelKey("some_future_status"), null);
  assert.equal(runStatusBadgeTone("failed"), "negative");
  assert.equal(runStatusBadgeTone("already_satisfied"), "warning");
  assert.equal(runStatusBadgeTone("some_future_status"), "neutral");
});

test("search history stays waiting or skipped until a durable receipt is accepted", () => {
  assert.equal(maintenanceSearchHistoryState(undefined), "waiting");
  assert.equal(
    maintenanceSearchHistoryState({
      step: {
        id: "search",
        kind: "SEARCH",
        parameters: {
          includeDescendants: null,
          targetQualityProfileId: null,
          searchCondition: "UNCONDITIONAL",
          tags: [],
        },
      },
      run: {
        stepId: "search",
        stepKind: "search",
        state: "skipped",
        attempt: 0,
        holdReason: null,
        error: null,
        createdAt: "2026-01-01T00:00:00Z",
        updatedAt: "2026-01-01T00:00:00Z",
        finishedAt: "2026-01-01T00:00:00Z",
      },
      receipts: [],
    } satisfies MaintenanceActionStepProgress),
    "skipped",
  );
  assert.equal(
    maintenanceSearchHistoryState({
      step: {
        id: "search",
        kind: "SEARCH",
        parameters: {
          includeDescendants: null,
          targetQualityProfileId: null,
          searchCondition: "UNCONDITIONAL",
          tags: [],
        },
      },
      run: null,
      receipts: [
        {
          dispatchAttempt: 1,
          logicalRequestKey: "request",
          jobRunId: "job-1",
          state: "accepted",
          createdAt: "2026-01-01T00:00:00Z",
          updatedAt: "2026-01-01T00:00:00Z",
        },
      ],
    } satisfies MaintenanceActionStepProgress),
    "accepted",
  );
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

test("scope is copied, sent on create and preview, and immutable on matcher updates", () => {
  const scoped: MaintenanceRuleSetDetail = {
    ...detail,
    ruleSet: { ...detail.ruleSet, subjectKind: "EPISODE" },
  };
  const draft = copyMaintenanceRuleDraft(scoped);
  assert.equal(draft.subjectKind, "EPISODE");
  assert.equal(createMaintenanceRuleSetInput(draft).subjectKind, "EPISODE");
  assert.equal(
    maintenancePreviewInput({ draft, libraryId: "series" }).subjectKind,
    "EPISODE",
  );
  assert.equal(
    Object.hasOwn(
      updateMaintenanceRuleMatcherInput("rule", draft),
      "subjectKind",
    ),
    false,
  );
});

test("exact subject preview retains its owning title and the draft library scope", () => {
  const draft = {
    ...initialMaintenanceRuleDraft(),
    subjectKind: "EPISODE" as const,
    libraryIds: ["series"],
  };
  const input = maintenancePreviewInput({
    draft,
    titleIds: ["show"],
    subjectIds: ["episode"],
    libraryId: "ignored",
  });
  assert.ok("titleIds" in input);
  assert.ok("subjectIds" in input);
  assert.deepEqual(input.titleIds, ["show"]);
  assert.deepEqual(input.subjectIds, ["episode"]);
  assert.deepEqual(input.libraryIds, ["series"]);
  assert.equal(Object.hasOwn(input, "libraryId"), false);
});

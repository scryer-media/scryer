import type {
  MaintenanceActionDescriptor,
  MaintenanceActionKind,
  MaintenanceCandidate,
  MaintenanceCandidateState,
  MaintenanceEffectArming,
  MaintenanceInstanceGates,
  MaintenancePreviewOutcome,
  MaintenanceRiskClass,
  MaintenanceRuleSetDetail,
  MaintenanceRuleSetDraft,
  MaintenanceRuleSetRecord,
} from "@/lib/types/maintenance-rule-sets";

/// The API caps a preview run at 50 titles and defaults to 20.
export const MAINTENANCE_PREVIEW_LIMIT_MAX = 50;
export const MAINTENANCE_PREVIEW_LIMIT_DEFAULT = 20;

/// Starter matcher for a new rule. No `package` or `import` line: the API
/// generates both and strips them back off when it hands the source to the
/// editor, so a template carrying them would vanish on the first round trip.
export const MAINTENANCE_STARTER_SOURCE = `# Maintenance matcher. Define \`match\` to select a subject, \`reasons\` to
# explain why, and \`unknown\` when the rule cannot see enough to decide.
#
# Every fact is a three-valued envelope: check \`.status == "known"\` before
# trusting \`.value\`. A fact Scryer could not resolve stays unknown rather
# than reading as false, 0, or "".

match if {
	input.facts.monitored.status == "known"
	not input.facts.monitored.value
	input.facts.has_file.status == "known"
	input.facts.has_file.value
}

reasons contains "unmonitored_with_files" if {
	match
}

# Prefer an explicit unknown over a confident wrong answer.
unknown if {
	input.facts.monitored.status == "unknown"
}
`;

const DEFAULT_ACTION_KIND: MaintenanceActionKind = "DO_NOTHING";

export function initialMaintenanceRuleDraft(): MaintenanceRuleSetDraft {
  return {
    name: "",
    description: "",
    regoSource: MAINTENANCE_STARTER_SOURCE,
    actionKind: DEFAULT_ACTION_KIND,
    targetQualityProfileId: "",
    graceDays: 0,
    libraryIds: [],
  };
}

export function maintenanceRuleDraftFromDetail(
  detail: MaintenanceRuleSetDetail,
): MaintenanceRuleSetDraft {
  return {
    name: detail.ruleSet.name,
    description: detail.ruleSet.description ?? "",
    regoSource: detail.revision.regoSource,
    actionKind: detail.actionSpec.kind,
    targetQualityProfileId: detail.actionSpec.targetQualityProfileId ?? "",
    graceDays: detail.revision.graceDays,
    libraryIds: [...detail.ruleSet.libraryIds],
  };
}

export function copyMaintenanceRuleDraft(
  detail: MaintenanceRuleSetDetail,
): MaintenanceRuleSetDraft {
  return {
    ...maintenanceRuleDraftFromDetail(detail),
    name: `Copy of ${detail.ruleSet.name}`,
  };
}

/// Action payload shared by create and matcher-update. `targetQualityProfileId`
/// is only sent for actions that declare they need one, so switching away from
/// a profile-changing action does not leave a stale profile on the rule.
export function maintenanceActionInput(
  draft: MaintenanceRuleSetDraft,
  descriptors: MaintenanceActionDescriptor[],
) {
  const needsProfile = actionRequiresTargetQualityProfile(
    descriptors,
    draft.actionKind,
  );
  const profileId = draft.targetQualityProfileId.trim();
  return {
    kind: draft.actionKind,
    targetQualityProfileId: needsProfile && profileId ? profileId : undefined,
  };
}

export function createMaintenanceRuleSetInput(
  draft: MaintenanceRuleSetDraft,
  descriptors: MaintenanceActionDescriptor[],
) {
  return {
    name: draft.name.trim(),
    description: draft.description.trim() || undefined,
    regoSource: draft.regoSource,
    action: maintenanceActionInput(draft, descriptors),
    graceDays: draft.graceDays,
    libraryIds: draft.libraryIds.length > 0 ? [...draft.libraryIds] : undefined,
  };
}

export function updateMaintenanceRuleMatcherInput(
  id: string,
  draft: MaintenanceRuleSetDraft,
  descriptors: MaintenanceActionDescriptor[],
) {
  return {
    id,
    regoSource: draft.regoSource,
    action: maintenanceActionInput(draft, descriptors),
    graceDays: draft.graceDays,
  };
}

export function updateMaintenanceRuleMetadataInput(
  id: string,
  draft: MaintenanceRuleSetDraft,
) {
  return {
    id,
    name: draft.name.trim(),
    description: draft.description.trim() || undefined,
    libraryIds: [...draft.libraryIds],
  };
}

/// Descriptors offerable for a title-scoped rule. The season- and episode-only
/// kinds stay in the enum but are never selectable here, and the filter reads
/// the descriptors rather than hardcoding which kinds those are.
export function titleScopedActionDescriptors(
  descriptors: MaintenanceActionDescriptor[],
): MaintenanceActionDescriptor[] {
  return descriptors.filter((descriptor) =>
    descriptor.supportedSubjects.some(
      (subject) => subject === "MOVIE" || subject === "SHOW",
    ),
  );
}

export function descriptorForActionKind(
  descriptors: MaintenanceActionDescriptor[],
  kind: MaintenanceActionKind,
): MaintenanceActionDescriptor | null {
  return descriptors.find((descriptor) => descriptor.kind === kind) ?? null;
}

export function actionRequiresTargetQualityProfile(
  descriptors: MaintenanceActionDescriptor[],
  kind: MaintenanceActionKind,
): boolean {
  return descriptorForActionKind(descriptors, kind)?.requiresTargetQualityProfile ?? false;
}

export function clampMaintenancePreviewLimit(limit: number): number {
  if (!Number.isFinite(limit)) {
    return MAINTENANCE_PREVIEW_LIMIT_DEFAULT;
  }
  return Math.min(MAINTENANCE_PREVIEW_LIMIT_MAX, Math.max(1, Math.trunc(limit)));
}

type MaintenancePreviewInputOptions = {
  ruleSetId?: string | null;
  draft?: MaintenanceRuleSetDraft;
  descriptors?: MaintenanceActionDescriptor[];
  libraryId?: string | null;
  limit?: number;
  titleIds?: string[];
};

/// Build the preview input. The API accepts either a stored rule set or an
/// inline draft as the matcher, and either explicit title ids or a library plus
/// a limit as the subject set; sending both halves of either pair is ambiguous,
/// so a stored rule set wins over a draft and explicit ids win over a library.
export function maintenancePreviewInput({
  ruleSetId,
  draft,
  descriptors = [],
  libraryId,
  limit,
  titleIds,
}: MaintenancePreviewInputOptions) {
  const matcher = ruleSetId
    ? { ruleSetId }
    : draft
      ? {
          regoSource: draft.regoSource,
          action: maintenanceActionInput(draft, descriptors),
          graceDays: draft.graceDays,
        }
      : {};
  const subjects =
    titleIds && titleIds.length > 0
      ? { titleIds: [...titleIds] }
      : {
          libraryId: libraryId || undefined,
          limit: clampMaintenancePreviewLimit(
            limit ?? MAINTENANCE_PREVIEW_LIMIT_DEFAULT,
          ),
        };
  return { ...matcher, ...subjects };
}

/// Badge tone for a descriptor's risk class. HIGH reads as destructive because
/// those actions delete files.
export function riskClassBadgeTone(
  risk: MaintenanceRiskClass | string,
): "neutral" | "info" | "warning" | "negative" {
  switch (risk) {
    case "HIGH":
      return "negative";
    case "MEDIUM":
      return "warning";
    case "LOW":
      return "info";
    default:
      return "neutral";
  }
}

export function previewOutcomeBadgeTone(
  outcome: MaintenancePreviewOutcome | null,
): "neutral" | "positive" | "warning" | "negative" {
  switch (outcome) {
    case "MATCH":
      return "positive";
    case "NO_MATCH":
      return "neutral";
    case "UNKNOWN":
      return "warning";
    default:
      return "negative";
  }
}

const ACTION_KIND_LABEL_KEYS: Record<MaintenanceActionKind, string> = {
  DO_NOTHING: "settings.maintenanceActionDoNothing",
  UNMONITOR_SCOPE_KEEP_FILES: "settings.maintenanceActionUnmonitorScopeKeepFiles",
  DELETE_TITLE_AND_FILES: "settings.maintenanceActionDeleteTitleAndFiles",
  UNMONITOR_TITLE_DELETE_ALL_FILES:
    "settings.maintenanceActionUnmonitorTitleDeleteAllFiles",
  UNMONITOR_SHOW_DELETE_EXISTING_FILES:
    "settings.maintenanceActionUnmonitorShowDeleteExistingFiles",
  UNMONITOR_SCOPE_DELETE_FILES: "settings.maintenanceActionUnmonitorScopeDeleteFiles",
  UNMONITOR_SEASON_DELETE_FILES_THEN_DELETE_SHOW_IF_EMPTY:
    "settings.maintenanceActionUnmonitorSeasonDeleteFilesThenDeleteShowIfEmpty",
  UNMONITOR_SEASON_THEN_UNMONITOR_SHOW_IF_EMPTY:
    "settings.maintenanceActionUnmonitorSeasonThenUnmonitorShowIfEmpty",
  CHANGE_QUALITY_PROFILE_AND_SEARCH_IF_CHANGED:
    "settings.maintenanceActionChangeQualityProfileAndSearchIfChanged",
};

/// Translation key for an action kind, or null when the API sends a kind this
/// build does not know. Callers render the raw kind rather than an empty cell.
export function actionKindLabelKey(kind: string): string | null {
  return ACTION_KIND_LABEL_KEYS[kind as MaintenanceActionKind] ?? null;
}

const RISK_CLASS_LABEL_KEYS: Record<MaintenanceRiskClass, string> = {
  NONE: "settings.maintenanceRiskNone",
  LOW: "settings.maintenanceRiskLow",
  MEDIUM: "settings.maintenanceRiskMedium",
  HIGH: "settings.maintenanceRiskHigh",
};

export function riskClassLabelKey(risk: string): string | null {
  return RISK_CLASS_LABEL_KEYS[risk as MaintenanceRiskClass] ?? null;
}

const EVALUATION_MODE_LABEL_KEYS: Record<string, string> = {
  DISABLED: "settings.maintenanceModeDisabled",
  SHADOW: "settings.maintenanceModeShadow",
  OBSERVE: "settings.maintenanceModeObserve",
};

export function evaluationModeLabelKey(mode: string): string | null {
  return EVALUATION_MODE_LABEL_KEYS[mode] ?? null;
}

const PREVIEW_OUTCOME_LABEL_KEYS: Record<MaintenancePreviewOutcome, string> = {
  MATCH: "settings.maintenancePreviewOutcomeMatch",
  NO_MATCH: "settings.maintenancePreviewOutcomeNoMatch",
  UNKNOWN: "settings.maintenancePreviewOutcomeUnknown",
};

export function previewOutcomeLabelKey(outcome: string): string | null {
  return PREVIEW_OUTCOME_LABEL_KEYS[outcome as MaintenancePreviewOutcome] ?? null;
}

// ── Operating a rule: modes, arming, gates ────────────────────────────

const EFFECT_ARMING_LABEL_KEYS: Record<MaintenanceEffectArming, string> = {
  NONE: "settings.maintenanceArmingNone",
  REVERSIBLE: "settings.maintenanceArmingReversible",
  DESTRUCTIVE: "settings.maintenanceArmingDestructive",
};

export function effectArmingLabelKey(arming: string): string | null {
  return EFFECT_ARMING_LABEL_KEYS[arming as MaintenanceEffectArming] ?? null;
}

/// Badge tone for a rule's arming. `DESTRUCTIVE` reads destructive because a
/// rule armed that far can delete files the moment its gate opens.
export function effectArmingBadgeTone(
  arming: MaintenanceEffectArming | string,
): "neutral" | "info" | "warning" | "negative" {
  switch (arming) {
    case "DESTRUCTIVE":
      return "negative";
    case "REVERSIBLE":
      return "warning";
    default:
      return "neutral";
  }
}

const EVALUATION_MODE_HELP_KEYS: Record<MaintenanceEvaluationModeKey, string> = {
  DISABLED: "settings.maintenanceModeDisabledHelp",
  SHADOW: "settings.maintenanceModeShadowHelp",
  OBSERVE: "settings.maintenanceModeObserveHelp",
};

type MaintenanceEvaluationModeKey = "DISABLED" | "SHADOW" | "OBSERVE";

export function evaluationModeHelpKey(mode: string): string | null {
  return EVALUATION_MODE_HELP_KEYS[mode as MaintenanceEvaluationModeKey] ?? null;
}

/// `DESTRUCTIVE` is only offered for a rule whose action actually deletes
/// something. Read from the descriptor rather than a hardcoded kind list, so a
/// new high-risk action is covered the day the API ships it.
export function destructiveArmingOfferable(
  descriptors: MaintenanceActionDescriptor[],
  kind: MaintenanceActionKind | string,
): boolean {
  return (
    descriptorForActionKind(descriptors, kind as MaintenanceActionKind)?.riskClass ===
    "HIGH"
  );
}

export function armingOptionsFor(
  descriptors: MaintenanceActionDescriptor[],
  kind: MaintenanceActionKind | string,
): MaintenanceEffectArming[] {
  return destructiveArmingOfferable(descriptors, kind)
    ? ["NONE", "REVERSIBLE", "DESTRUCTIVE"]
    : ["NONE", "REVERSIBLE"];
}

/// The five gates in the order the panel renders them: what may run, then what
/// may be seen, then the three effect classes from least to most damaging.
export const MAINTENANCE_GATE_ORDER = [
  "evaluationEnabled",
  "resultDisplayEnabled",
  "presentationEffectsEnabled",
  "reversibleEffectsEnabled",
  "destructiveEffectsEnabled",
] as const satisfies readonly (keyof MaintenanceInstanceGates)[];

export type MaintenanceStatusBannerVariant =
  | "gatesUnknown"
  | "evaluationDisabled"
  | "noActiveRules"
  | "effectsDisabled"
  | "reversibleArmed"
  | "destructiveArmed";

/// What the page says about itself, derived rather than hardcoded: the section
/// used to carry a permanent "nothing runs yet" notice, and that is no longer
/// true of every instance.
///
/// `gates` is null when the gates query failed, which for this page means the
/// reader is not a system administrator and cannot see the instance state.
export function maintenanceStatusBanner(
  gates: MaintenanceInstanceGates | null,
  ruleSets: Pick<MaintenanceRuleSetRecord, "evaluationMode">[],
): { variant: MaintenanceStatusBannerVariant; tone: "info" | "warning" } {
  if (!gates) {
    return { variant: "gatesUnknown", tone: "info" };
  }
  if (!gates.evaluationEnabled) {
    return { variant: "evaluationDisabled", tone: "info" };
  }
  if (!ruleSets.some((ruleSet) => ruleSet.evaluationMode !== "DISABLED")) {
    return { variant: "noActiveRules", tone: "info" };
  }
  if (gates.destructiveEffectsEnabled) {
    return { variant: "destructiveArmed", tone: "warning" };
  }
  if (gates.reversibleEffectsEnabled || gates.presentationEffectsEnabled) {
    return { variant: "reversibleArmed", tone: "info" };
  }
  return { variant: "effectsDisabled", tone: "info" };
}

const STATUS_BANNER_KEYS: Record<
  MaintenanceStatusBannerVariant,
  { titleKey: string; bodyKey: string }
> = {
  gatesUnknown: {
    titleKey: "settings.maintenanceStatusUnknownTitle",
    bodyKey: "settings.maintenanceStatusUnknownBody",
  },
  evaluationDisabled: {
    titleKey: "settings.maintenanceStatusEvaluationOffTitle",
    bodyKey: "settings.maintenanceStatusEvaluationOffBody",
  },
  noActiveRules: {
    titleKey: "settings.maintenanceStatusNoRulesTitle",
    bodyKey: "settings.maintenanceStatusNoRulesBody",
  },
  effectsDisabled: {
    titleKey: "settings.maintenanceStatusEffectsOffTitle",
    bodyKey: "settings.maintenanceStatusEffectsOffBody",
  },
  reversibleArmed: {
    titleKey: "settings.maintenanceStatusReversibleTitle",
    bodyKey: "settings.maintenanceStatusReversibleBody",
  },
  destructiveArmed: {
    titleKey: "settings.maintenanceStatusDestructiveTitle",
    bodyKey: "settings.maintenanceStatusDestructiveBody",
  },
};

export function maintenanceStatusBannerKeys(
  variant: MaintenanceStatusBannerVariant,
): { titleKey: string; bodyKey: string } {
  return STATUS_BANNER_KEYS[variant];
}

const GATE_LABEL_KEYS: Record<keyof MaintenanceInstanceGates, string> = {
  evaluationEnabled: "settings.maintenanceGateEvaluation",
  resultDisplayEnabled: "settings.maintenanceGateResultDisplay",
  presentationEffectsEnabled: "settings.maintenanceGatePresentationEffects",
  reversibleEffectsEnabled: "settings.maintenanceGateReversibleEffects",
  destructiveEffectsEnabled: "settings.maintenanceGateDestructiveEffects",
};

const GATE_HELP_KEYS: Record<keyof MaintenanceInstanceGates, string> = {
  evaluationEnabled: "settings.maintenanceGateEvaluationHelp",
  resultDisplayEnabled: "settings.maintenanceGateResultDisplayHelp",
  presentationEffectsEnabled: "settings.maintenanceGatePresentationEffectsHelp",
  reversibleEffectsEnabled: "settings.maintenanceGateReversibleEffectsHelp",
  destructiveEffectsEnabled: "settings.maintenanceGateDestructiveEffectsHelp",
};

export function gateLabelKey(gate: keyof MaintenanceInstanceGates): string {
  return GATE_LABEL_KEYS[gate];
}

export function gateHelpKey(gate: keyof MaintenanceInstanceGates): string {
  return GATE_HELP_KEYS[gate];
}

/// Sentinel for "no filter". The select primitive refuses an empty-string
/// option value, and `all` is how the rest of the settings filters spell it.
export const MAINTENANCE_FILTER_ALL = "all";

/// Translate a filter select's value into the query argument: the sentinel and
/// the empty string both mean "send nothing and let the API decide".
export function maintenanceFilterArgument(value: string): string | undefined {
  return !value || value === MAINTENANCE_FILTER_ALL ? undefined : value;
}

// ── Candidates ────────────────────────────────────────────────────────

const CANDIDATE_STATE_LABEL_KEYS: Record<MaintenanceCandidateState, string> = {
  OBSERVING: "settings.maintenanceCandidateStateObserving",
  PENDING_ACTION: "settings.maintenanceCandidateStatePendingAction",
  DUE: "settings.maintenanceCandidateStateDue",
  EXECUTING: "settings.maintenanceCandidateStateExecuting",
  SUCCEEDED: "settings.maintenanceCandidateStateSucceeded",
  FAILED: "settings.maintenanceCandidateStateFailed",
  CANCELED: "settings.maintenanceCandidateStateCanceled",
  EXCLUDED: "settings.maintenanceCandidateStateExcluded",
  BLOCKED: "settings.maintenanceCandidateStateBlocked",
};

export function candidateStateLabelKey(state: string): string | null {
  return CANDIDATE_STATE_LABEL_KEYS[state as MaintenanceCandidateState] ?? null;
}

/// Tone map for the candidate lifecycle. `BLOCKED` is a warning rather than a
/// failure: a safety precondition refused the action, which is the system
/// working, but it is also the state an operator has to go look at.
export function candidateStateBadgeTone(
  state: MaintenanceCandidateState | string,
): "neutral" | "info" | "positive" | "warning" | "negative" {
  switch (state) {
    case "PENDING_ACTION":
    case "DUE":
    case "EXECUTING":
      return "info";
    case "SUCCEEDED":
      return "positive";
    case "FAILED":
      return "negative";
    case "BLOCKED":
      return "warning";
    default:
      return "neutral";
  }
}

/// Candidate states that are still moving. Arming a rule destructively has to
/// acknowledge exactly these, because they are the ones an armed handler could
/// still act on.
export const MAINTENANCE_NON_TERMINAL_CANDIDATE_STATES = [
  "OBSERVING",
  "PENDING_ACTION",
  "DUE",
  "EXECUTING",
  "BLOCKED",
] as const satisfies readonly MaintenanceCandidateState[];

export function isNonTerminalCandidateState(state: string): boolean {
  return (MAINTENANCE_NON_TERMINAL_CANDIDATE_STATES as readonly string[]).includes(
    state,
  );
}

/// How many of a rule's candidates an armed handler could still act on. This is
/// the number destructive arming makes the operator acknowledge.
export function nonTerminalCandidateCount(
  candidates: Pick<MaintenanceCandidate, "state">[],
): number {
  return candidates.filter((candidate) => isNonTerminalCandidateState(candidate.state))
    .length;
}

const MINUTE_MS = 60_000;
const HOUR_MS = 60 * MINUTE_MS;
const DAY_MS = 24 * HOUR_MS;

export type MaintenanceCountdown = {
  overdue: boolean;
  labelKey: string;
  values: { count: number };
};

/// Turn a candidate's `dueAt` into a countdown the table can render: "in 3d"
/// while the grace clock runs, "overdue by 2h" once it has elapsed. The unit is
/// chosen by magnitude and carried in the key, so every locale writes its own
/// phrasing rather than receiving a pre-formatted English string.
///
/// `now` is a parameter rather than a call to the clock so the caller controls
/// the tick and the behaviour is testable.
export function maintenanceCountdown(
  dueAt: string,
  now: number = Date.now(),
): MaintenanceCountdown | null {
  const due = Date.parse(dueAt);
  if (!Number.isFinite(due)) {
    return null;
  }
  const deltaMs = due - now;
  const overdue = deltaMs < 0;
  const magnitude = Math.abs(deltaMs);

  if (magnitude < MINUTE_MS) {
    return {
      overdue,
      labelKey: "settings.maintenanceCountdownDueNow",
      values: { count: 0 },
    };
  }

  const unit =
    magnitude >= DAY_MS
      ? { count: Math.floor(magnitude / DAY_MS), suffix: "Days" }
      : magnitude >= HOUR_MS
        ? { count: Math.floor(magnitude / HOUR_MS), suffix: "Hours" }
        : { count: Math.floor(magnitude / MINUTE_MS), suffix: "Minutes" };

  return {
    overdue,
    labelKey: `settings.maintenanceCountdown${overdue ? "Overdue" : "In"}${unit.suffix}`,
    values: { count: unit.count },
  };
}

// ── Runs ──────────────────────────────────────────────────────────────

const RUN_STATUS_LABEL_KEYS: Record<string, string> = {
  running: "settings.maintenanceRunStatusRunning",
  succeeded: "settings.maintenanceRunStatusSucceeded",
  failed: "settings.maintenanceRunStatusFailed",
  held: "settings.maintenanceRunStatusHeld",
  already_satisfied: "settings.maintenanceRunStatusAlreadySatisfied",
  skipped: "settings.maintenanceRunStatusSkipped",
};

/// Run statuses are plain strings on the API, so an unknown one renders as
/// itself rather than as an empty cell.
export function runStatusLabelKey(status: string): string | null {
  return RUN_STATUS_LABEL_KEYS[status.toLowerCase()] ?? null;
}

export function runStatusBadgeTone(
  status: string,
): "neutral" | "info" | "positive" | "warning" | "negative" {
  switch (status.toLowerCase()) {
    case "running":
      return "info";
    case "succeeded":
      return "positive";
    case "failed":
      return "negative";
    case "held":
    case "already_satisfied":
      return "warning";
    default:
      return "neutral";
  }
}

/// The server rejects a destructive arming whose acknowledged count no longer
/// matches, and puts the current count in the message. Pull it out so the
/// confirm dialog can re-ask against the number the server actually has,
/// instead of making the operator refresh the page to find out.
///
/// Pinned shape:
/// `destructive arming requires acknowledging the current candidate count (N)`
export function parseAcknowledgedCandidateCountMismatch(
  message: string,
): number | null {
  const match = /acknowledging the current candidate count \((\d+)\)/i.exec(message);
  if (!match) {
    return null;
  }
  const count = Number.parseInt(match[1], 10);
  return Number.isFinite(count) ? count : null;
}

export function setMaintenanceRuleArmingInput(
  id: string,
  arming: MaintenanceEffectArming,
  acknowledgedCandidateCount?: number,
) {
  return {
    id,
    arming,
    /// Only a destructive arming carries an acknowledgement; sending one for
    /// `NONE` or `REVERSIBLE` would invite the server to check a number the UI
    /// never showed anyone.
    acknowledgedCandidateCount:
      arming === "DESTRUCTIVE" ? acknowledgedCandidateCount : undefined,
  };
}

export function excludeMaintenanceSubjectInput(options: {
  titleId: string;
  ruleSetId?: string | null;
  reason?: string;
}) {
  const reason = options.reason?.trim();
  return {
    titleId: options.titleId,
    ruleSetId: options.ruleSetId || undefined,
    reason: reason || undefined,
  };
}

import type {
  MaintenanceActionDescriptor,
  MaintenanceActionKind,
  MaintenancePreviewOutcome,
  MaintenanceRiskClass,
  MaintenanceRuleSetDetail,
  MaintenanceRuleSetDraft,
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

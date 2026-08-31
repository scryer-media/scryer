/// Maintenance rules are authored and previewed here, but nothing evaluates or
/// executes them yet: the API saves every rule set disabled and no scheduler
/// reads them. Preview is the only path that runs a matcher, and it touches
/// nothing in the library.

/// Lifecycle mode of a stored rule set. The API pins new rule sets to
/// `DISABLED`; the other modes exist for later waves.
export type MaintenanceEvaluationMode = "DISABLED" | "SHADOW" | "OBSERVE";

/// Subjects an action descriptor declares support for. The UI only offers
/// title-scoped rules, so only `MOVIE` and `SHOW` descriptors are selectable.
export type MaintenanceSubjectScope = "MOVIE" | "SHOW" | "SEASON" | "EPISODE";

export type MaintenanceRiskClass = "NONE" | "LOW" | "MEDIUM" | "HIGH";

export type MaintenanceActionKind =
  | "DO_NOTHING"
  | "UNMONITOR_SCOPE_KEEP_FILES"
  | "DELETE_TITLE_AND_FILES"
  | "UNMONITOR_TITLE_DELETE_ALL_FILES"
  | "UNMONITOR_SHOW_DELETE_EXISTING_FILES"
  | "UNMONITOR_SCOPE_DELETE_FILES"
  | "UNMONITOR_SEASON_DELETE_FILES_THEN_DELETE_SHOW_IF_EMPTY"
  | "UNMONITOR_SEASON_THEN_UNMONITOR_SHOW_IF_EMPTY"
  | "CHANGE_QUALITY_PROFILE_AND_SEARCH_IF_CHANGED";

export type MaintenancePreviewOutcome = "MATCH" | "NO_MATCH" | "UNKNOWN";

export type MaintenanceRuleSetRecord = {
  id: string;
  name: string;
  description: string | null;
  enabled: boolean;
  evaluationMode: MaintenanceEvaluationMode;
  libraryIds: string[];
  /// Granularity the rule set is scoped to. Kept as a plain string because the
  /// rule-set vocabulary (title/season/episode) is not the same enum as an
  /// action descriptor's `supportedSubjects` (movie/show/season/episode).
  subjectKind: string;
  currentRevisionNumber: number;
  /// Action and grace period of the revision in force, carried on the list
  /// payload so rendering badges never needs a per-rule detail fetch.
  graceDays: number;
  actionSpec: MaintenanceActionSpec;
  createdAt: string;
  updatedAt: string;
};

export type MaintenanceRuleRevision = {
  id: string;
  ruleSetId: string;
  /// Editor-stripped source: the API removes the package and import lines it
  /// generates, so what round-trips through the editor is what the author wrote.
  regoSource: string;
  revisionNumber: number;
  graceDays: number;
  matcherContentHash: string;
  createdBy: string | null;
  createdAt: string;
};

export type MaintenanceActionSpec = {
  kind: MaintenanceActionKind;
  schemaVersion: number;
  targetQualityProfileId: string | null;
};

export type MaintenanceRuleSetDetail = {
  ruleSet: MaintenanceRuleSetRecord;
  revision: MaintenanceRuleRevision;
  actionSpec: MaintenanceActionSpec;
};

export type MaintenanceActionDescriptor = {
  kind: MaintenanceActionKind;
  supportedSubjects: MaintenanceSubjectScope[];
  riskClass: MaintenanceRiskClass;
  effectClasses: string[];
  timingMode: string;
  allowedRepeatModes: string[];
  requiresTargetQualityProfile: boolean;
};

export type MaintenanceRuleSetDraft = {
  name: string;
  description: string;
  regoSource: string;
  actionKind: MaintenanceActionKind;
  targetQualityProfileId: string;
  graceDays: number;
  libraryIds: string[];
};

export type MaintenanceValidationResult = {
  valid: boolean;
  errors: string[];
};

export type MaintenancePreviewTitle = {
  titleId: string;
  titleName: string;
  facet: string;
  libraryId: string;
  /// Null when the matcher errored for this title; `error` then carries why.
  outcome: MaintenancePreviewOutcome | null;
  reasonCodes: string[];
  error: string | null;
};

export type MaintenancePreviewResult = {
  ruleSetId: string | null;
  matcherContentHash: string;
  evaluatedAt: string;
  titles: MaintenancePreviewTitle[];
};

/// Where a preview run gets its matcher from: the stored revision of a saved
/// rule set, or the unsaved editor draft.
export type MaintenancePreviewSource = "stored" | "draft";

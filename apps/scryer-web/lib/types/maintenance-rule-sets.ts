/// Maintenance rules are authored, armed, and operated here. What actually
/// happens to a matching title is decided by two independent controls: the
/// instance-wide gates (which the whole instance shares) and each rule's own
/// evaluation mode and effect arming. A rule only acts when both agree.

/// Lifecycle mode of a stored rule set. Creation always stores `DISABLED`, so
/// arming a rule is always a deliberate second step.
export type MaintenanceEvaluationMode = "DISABLED" | "SHADOW" | "OBSERVE";

/// How far a rule set's effects are armed. `NONE` evaluates without acting,
/// `REVERSIBLE` permits low- and medium-risk actions, and `DESTRUCTIVE`
/// additionally permits the high-risk actions that delete files.
export type MaintenanceEffectArming = "NONE" | "REVERSIBLE" | "DESTRUCTIVE";

/// Rule scope is distinct from an action's movie/show/season/episode vocabulary.
export type MaintenanceRuleScope = "TITLE" | "SEASON" | "EPISODE";

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
  | "CHANGE_QUALITY_PROFILE_AND_SEARCH_IF_CHANGED"
  | "ADD_TAGS"
  | "REMOVE_TAGS";

export type MaintenancePreviewOutcome = "MATCH" | "NO_MATCH" | "UNKNOWN";

export type MaintenanceRuleSetRecord = {
  id: string;
  name: string;
  description: string | null;
  enabled: boolean;
  evaluationMode: MaintenanceEvaluationMode;
  /// How far this rule's effects are armed, independently of its mode. A rule
  /// can evaluate in `OBSERVE` while still armed to `NONE`.
  effectArming: MaintenanceEffectArming;
  /// An existing destructive title rule must be reviewed after its newly
  /// available show facts changed what it can select.
  destructiveRearmRequired: boolean;
  libraryIds: string[];
  /// Immutable granularity, separate from an action's supported subjects.
  subjectKind: MaintenanceRuleScope;
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
  /// Configured root for storage facts and root-filtered file deletion.
  storageRootId: string | null;
  matcherContentHash: string;
  createdBy: string | null;
  createdAt: string;
};

export type MaintenanceActionSpec = {
  kind: MaintenanceActionKind;
  schemaVersion: number;
  targetQualityProfileId: string | null;
  /// Labels the tag actions write. Empty for every other kind, so a caller can
  /// render "what would this rule do" without branching on the kind first.
  tags: string[];
};

export type MaintenanceRuleSetDetail = {
  ruleSet: MaintenanceRuleSetRecord;
  revision: MaintenanceRuleRevision;
  actionSpec: MaintenanceActionSpec;
};

export type MaintenanceActionDescriptor = {
  supportedRuleScopes: MaintenanceRuleScope[];
  kind: MaintenanceActionKind;
  supportedSubjects: MaintenanceSubjectScope[];
  riskClass: MaintenanceRiskClass;
  effectClasses: string[];
  timingMode: string;
  allowedRepeatModes: string[];
  requiresTargetQualityProfile: boolean;
  /// True for the tag actions, which need at least one registry-defined label
  /// before the rule can be saved.
  requiresTags: boolean;
  /// The server permits this action when a configured storage root scopes the
  /// rule. This remains API-owned so the editor cannot drift from execution.
  supportsStorageScope: boolean;
};

export type MaintenanceRuleSetDraft = {
  subjectKind: MaintenanceRuleScope;
  name: string;
  description: string;
  regoSource: string;
  actionKind: MaintenanceActionKind;
  targetQualityProfileId: string;
  /// Labels a tag action writes. Held on the draft even while another action is
  /// selected, so switching back and forth does not lose what was picked; the
  /// input builder drops them for kinds that take none.
  tags: string[];
  graceDays: number;
  /// Empty when storage capacity and root-filtered deletion do not apply.
  storageRootId: string;
  libraryIds: string[];
};

export type MaintenanceValidationResult = {
  valid: boolean;
  errors: string[];
};

export type MaintenanceTestSubject = { titleId: string; subjectId: string };

export type MaintenancePreviewTitle = {
  excluded: boolean;
  dueAt: string | null;
  dueAtIsEstimate: boolean;
  subjectLabel: string;
  fileCount: number;
  totalSizeBytes: number;
  /// File summary restricted to the configured storage root. Null means the
  /// root could not be resolved for this preview subject.
  storageRootFileCount: number | null;
  storageRootTotalSizeBytes: number | null;
  subjectKind: string;
  subjectId: string;
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

/// Lifecycle state of one candidate. The states past `OBSERVING` are written by
/// the action handler; a shadow rule only ever produces the first three.
export type MaintenanceCandidateState =
  | "OBSERVING"
  | "PENDING_ACTION"
  | "DUE"
  | "EXECUTING"
  | "SUCCEEDED"
  | "FAILED"
  | "CANCELED"
  | "EXCLUDED"
  | "BLOCKED";

/// One subject's membership in one rule set. Nothing here has acted on the
/// subject: a candidate records that a rule matched it and how much of the
/// grace period is left.
export type MaintenanceCandidate = {
  fileCount: number | null;
  totalSizeBytes: number | null;
  subjectLabel: string;
  subjectKind: string;
  subjectId: string;
  id: string;
  ruleSetId: string;
  ruleName: string;
  revisionNumber: number;
  titleId: string;
  titleName: string;
  libraryId: string;
  facet: string;
  state: MaintenanceCandidateState;
  stateReason: string;
  reasonCodes: string[];
  actionKind: MaintenanceActionKind;
  graceDays: number;
  matchGeneration: number;
  firstMatchedAt: string;
  lastMatchedAt: string;
  dueAt: string;
  /// Set while the latest evaluation could not decide, null otherwise.
  heldSince: string | null;
  updatedAt: string;
};

/// One rule set's pass through one evaluation run. Runs carry counts rather
/// than subjects, which is why the result-display gate does not hide them.
export type MaintenanceEvaluationRun = {
  id: string;
  ruleSetId: string;
  revisionNumber: number;
  /// Free-form on purpose: `running`, `succeeded`, `failed`, and whatever the
  /// API adds later. Rendered through a label map that falls back to the raw
  /// value.
  status: string;
  startedAt: string;
  finishedAt: string | null;
  evaluatedCount: number;
  matchedCount: number;
  noMatchCount: number;
  unknownCount: number;
  errorCount: number;
  durationMs: number | null;
  error: string | null;
};

/// One attempt by the action handler to execute one candidate's action.
export type MaintenanceActionRun = {
  subjectLabel: string;
  detail: string;
  subjectKind: string;
  subjectId: string;
  id: string;
  ruleSetId: string;
  candidateId: string;
  titleId: string;
  titleName: string;
  actionKind: MaintenanceActionKind;
  matchGeneration: number;
  attempt: number;
  /// Free-form for the same reason as an evaluation run's status; includes
  /// `already_satisfied` for an action that found nothing left to do.
  status: string;
  /// Why a safety precondition held the action back, when one did.
  holdReason: string | null;
  error: string | null;
  startedAt: string;
  finishedAt: string | null;
};

/// The five independent instance-wide gates. Every one defaults off, and a rule
/// only ever acts where its own arming and the matching gate agree.
export type MaintenanceInstanceGates = {
  evaluationEnabled: boolean;
  resultDisplayEnabled: boolean;
  presentationEffectsEnabled: boolean;
  reversibleEffectsEnabled: boolean;
  destructiveEffectsEnabled: boolean;
};

/// Which gate a switch drives. Kept as a union so the panel can render the five
/// switches from one list without a stringly-typed indexer.
export type MaintenanceGateKey = keyof MaintenanceInstanceGates;

/// A subject a maintenance rule must never act on.
export type MaintenanceExclusion = {
  subjectLabel: string;
  subjectKind: string;
  subjectId: string;
  id: string;
  /// Null when the exclusion is global rather than confined to one rule.
  ruleSetId: string | null;
  titleId: string;
  titleName: string;
  reason: string;
  createdBy: string | null;
  createdAt: string;
};

/// Outcome of asking for an immediate evaluation or action pass.
export type MaintenanceTriggerResult = {
  started: boolean;
  message: string | null;
};

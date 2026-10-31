/// Maintenance rules are authored and operated here. What actually happens to
/// a matching title is decided by two controls: the two instance-wide switches
/// and each rule's own state. A rule only acts when both allow it.

/// What a stored rule does. Creation always stores `DISABLED`, so turning a
/// rule on is always a deliberate second step; `DRY_RUN` records matches
/// without changing anything.
export type MaintenanceRuleState = "DISABLED" | "DRY_RUN" | "ENABLED";

/// The class of effects a rule's action can have. `DESTRUCTIVE` actions delete
/// files, so enabling such a rule needs a confirmation.
export type MaintenanceEffectArming = "NONE" | "REVERSIBLE" | "DESTRUCTIVE";

/// Rule scope is distinct from an action's movie/show/season/episode vocabulary.
export type MaintenanceRuleScope = "TITLE" | "SEASON" | "EPISODE";

export type MaintenanceSubjectScope = "MOVIE" | "SHOW" | "SEASON" | "EPISODE";

export type MaintenanceRiskClass = "NONE" | "LOW" | "MEDIUM" | "HIGH";

export type MaintenanceActionKind =
  | "ACTION_SEQUENCE"
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

export type MaintenanceActionStepKind =
  | "UNMONITOR"
  | "DELETE_FILES"
  | "CHANGE_QUALITY_PROFILE"
  | "SEARCH"
  | "ADD_TAGS"
  | "REMOVE_TAGS"
  | "DELETE_TITLE_AND_FILES";

export type MaintenanceActionStepParameters = {
  includeDescendants: boolean | null;
  targetQualityProfileId: string | null;
  searchCondition: MaintenanceSearchCondition | null;
  tags: string[];
};

export type MaintenanceSearchCondition =
  "UNCONDITIONAL" | "PREVIOUS_PROFILE_CHANGED";

export type MaintenanceActionStep = {
  id: string;
  kind: MaintenanceActionStepKind;
  parameters: MaintenanceActionStepParameters;
};

export type MaintenanceActionSequence = {
  schemaVersion: number;
  steps: MaintenanceActionStep[];
};

export type MaintenancePreviewOutcome = "MATCH" | "NO_MATCH" | "UNKNOWN";

export type MaintenanceRuleSetRecord = {
  id: string;
  name: string;
  description: string | null;
  state: MaintenanceRuleState;
  /// What `ENABLED` would allow for this rule's action. `NONE` means the rule
  /// can only run as a dry run.
  enabledArming: MaintenanceEffectArming;
  /// The rule was enabled, then its matcher, action or scope changed, so it
  /// dropped back to a dry run until it is enabled again.
  editedSinceEnabled: boolean;
  /// Shipped with Scryer: name, matcher and action are fixed and it cannot be
  /// deleted. State, grace period and libraries stay editable.
  systemOwned: boolean;
  libraryIds: string[];
  /// Immutable granularity, separate from an action's supported subjects.
  subjectKind: MaintenanceRuleScope;
  currentRevisionNumber: number;
  /// Action and grace period of the revision in force, carried on the list
  /// payload so rendering badges never needs a per-rule detail fetch.
  graceDays: number;
  actionSpec: MaintenanceActionSpec;
  /// Present for schema-2 revisions. The legacy action spec stays in place so
  /// old revisions remain readable without being silently converted.
  actionSequence?: MaintenanceActionSequence | null;
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
  actionSequence: MaintenanceActionSequence | null;
};

/// The sequence catalog is owned by the API. The editor only uses these
/// advertised capabilities to offer steps and fields; it never recreates the
/// effect or dependency matrix locally.
export type MaintenanceActionStepDescriptor = {
  id: string;
  kind: MaintenanceActionStepKind;
  label: string;
  supportedSubjects: MaintenanceSubjectScope[];
  riskClass: MaintenanceRiskClass;
  effectClasses: string[];
  parameterSchema:
    "none" | "unmonitor" | "change_quality_profile" | "search" | "tags";
  requires: string[];
  terminal: boolean;
  completionPolicy: "accepted" | "completed";
  storageRootAllowed: boolean;
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
  /// Schema-2 action definition. An empty list is intentionally observe-only;
  /// the editor does not synthesize a do-nothing step.
  actionSequence: MaintenanceActionSequence;
  graceDays: number;
  /// Empty when storage capacity and root-filtered deletion do not apply.
  storageRootId: string;
  libraryIds: string[];
};

export type MaintenanceValidationResult = {
  valid: boolean;
  errors: string[];
  diagnostics?: import("../utils/rego-diagnostics").RuleValidationDiagnostic[];
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
/// the action handler; a dry run rule only ever produces the first three.
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

/// Latest durable outcome for one ordered action sequence step.
export type MaintenanceActionStepRun = {
  stepId: string;
  stepKind: string;
  state: string;
  attempt: number;
  holdReason: string | null;
  error: string | null;
  createdAt: string;
  updatedAt: string;
  finishedAt: string | null;
};

/// Durable evidence for a job requested by a sequence step. An accepted
/// receipt means the request was accepted; it is not a download completion.
export type MaintenanceActionJobReceipt = {
  dispatchAttempt: number;
  logicalRequestKey: string;
  jobRunId: string | null;
  state: string;
  createdAt: string;
  updatedAt: string;
};

/// One configured action sequence step with its latest checkpoint and receipts.
export type MaintenanceActionStepProgress = {
  step: MaintenanceActionStep;
  run: MaintenanceActionStepRun | null;
  receipts: MaintenanceActionJobReceipt[];
};

/// One subject's membership in one rule set. Nothing here has acted on the
/// subject: a candidate records that a rule matched it and how much of the
/// grace period is left.
export type MaintenanceCandidate = {
  fileCount: number | null;
  totalSizeBytes: number | null;
  /// File summary restricted to the candidate revision's configured storage
  /// root. Null means no root was selected or exact ownership is unknown.
  storageRootFileCount: number | null;
  storageRootTotalSizeBytes: number | null;
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
  actionSequence: MaintenanceActionSequence | null;
  sequenceSteps: MaintenanceActionStepProgress[];
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
/// than subjects, which is why switching maintenance off does not hide them.
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
  actionSequence: MaintenanceActionSequence | null;
  sequenceSteps: MaintenanceActionStepProgress[];
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

/// The two instance-wide switches. A new instance has maintenance disabled and
/// actions paused, and an enabled rule only acts while both allow it.
export type MaintenanceInstanceGates = {
  enabled: boolean;
  actionsPaused: boolean;
};

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

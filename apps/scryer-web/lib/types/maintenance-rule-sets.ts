/// Maintenance rules are authored and operated here. What actually happens to
/// a matching title is decided by two controls: the two instance-wide switches
/// and each rule's own state. A rule only acts when both allow it.

/// What a stored rule does. Creation always stores `DISABLED`, so turning a
/// rule on is always a deliberate second step; `DRY_RUN` records matches
/// without changing anything.
export type MaintenanceRuleState = "DISABLED" | "DRY_RUN" | "ENABLED";

/// The class of effects a rule's actions can have. `DESTRUCTIVE` rules delete
/// files or titles, so enabling such a rule needs a confirmation.
export type MaintenanceEffectArming = "NONE" | "REVERSIBLE" | "DESTRUCTIVE";

/// Rule scope is distinct from an action's movie/show/season/episode vocabulary.
export type MaintenanceRuleScope = "TITLE" | "SEASON" | "EPISODE";

/// An action a rule declares by defining a rule of that name in its source,
/// as the API names it (`DELETE_FILES`). Any other rule name in the source is
/// the author's own helper. The heads a build knows come from the served
/// output descriptors, so this is any string the server sends.
export type MaintenanceActionHead = string;

/// What a rule's source can do, read from the source itself. The value lists
/// hold every literal the source can produce for that head, so a rule with an
/// `else` chain lists each branch.
export type MaintenanceDeclaredActions = {
  heads: MaintenanceActionHead[];
  /// The rule defines `delete_files` or `delete_title`.
  destructive: boolean;
  qualityProfiles: string[];
  addTags: string[];
  removeTags: string[];
  libraries: string[];
  roots: string[];
  notifications: string[];
};

/// What a rule decided for one subject, with heads in the order the server
/// applies them.
export type MaintenanceDecidedActions = {
  heads: MaintenanceActionHead[];
  qualityProfile: string | null;
  addTags: string[];
  removeTags: string[];
  library: string | null;
  root: string | null;
  notification: string | null;
};

/// Action kinds recorded on candidates and action runs written before rules
/// declared their actions in source. Kept so history stays readable.
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
  /// What `ENABLED` would allow for this rule's actions. `NONE` means the rule
  /// can only run as a dry run.
  enabledArming: MaintenanceEffectArming;
  /// The rule was enabled, then its source or scope changed, so it
  /// dropped back to a dry run until it is enabled again.
  editedSinceEnabled: boolean;
  /// Shipped with Scryer: name and source are fixed and it cannot be
  /// deleted. State, grace period and libraries stay editable.
  systemOwned: boolean;
  libraryIds: string[];
  /// Immutable granularity, separate from an action's supported subjects.
  subjectKind: MaintenanceRuleScope;
  currentRevisionNumber: number;
  /// Grace period and declared actions of the revision in force, carried on
  /// the list payload so rendering badges never needs a per-rule detail fetch.
  graceDays: number;
  declaredActions: MaintenanceDeclaredActions;
  /// When the rule runs. One run evaluates the rule, then applies its
  /// candidates whose grace period has ended. A rule setting like its
  /// libraries: changing it makes no new revision.
  schedule: import("./scripts").ScriptSchedule;
  /// Null for a manual schedule and for a disabled rule.
  nextRunAt: string | null;
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

export type MaintenanceRuleSetDetail = {
  ruleSet: MaintenanceRuleSetRecord;
  revision: MaintenanceRuleRevision;
  declaredActions: MaintenanceDeclaredActions;
  /// The simple builder's model, stored opaquely next to the rule. Null when
  /// the rule was never built with it or was detached from it. Read through
  /// `parseRuleBuilderModel`; only the detail query selects it.
  builderModel?: unknown;
};

export type MaintenanceRuleSetDraft = {
  subjectKind: MaintenanceRuleScope;
  name: string;
  description: string;
  /// Declares both which subjects match and what happens to them.
  regoSource: string;
  graceDays: number;
  /// Empty when storage capacity and root-filtered deletion do not apply.
  storageRootId: string;
  libraryIds: string[];
  schedule: import("./scripts").ScriptSchedule;
  /// Set while the rule is still the simple builder's: `regoSource` is then
  /// what the builder generated from this model. Null once the source is
  /// edited by hand, and for every rule the builder did not make.
  builderModel: import("../utils/rule-builder").RuleBuilderModel | null;
};

export type MaintenanceValidationResult = {
  valid: boolean;
  errors: string[];
  diagnostics?: import("../utils/rego-diagnostics").RuleValidationDiagnostic[];
  /// Findings that do not block saving, such as a rule nothing references.
  warnings?: import("../utils/rego-diagnostics").RuleValidationDiagnostic[];
  /// Null when the source could not be parsed.
  declaredActions?: MaintenanceDeclaredActions | null;
  /// Validation could not be reached, so nothing above describes the source.
  unavailable?: boolean;
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
  /// What the rule decided for this subject. Null when it did not match.
  actions?: MaintenanceDecidedActions | null;
  /// The input document the rule evaluated for this subject. Null unless the
  /// caller may manage permissions, because it names who watched and who
  /// requested the subject.
  facts?: Record<string, unknown> | null;
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
  /// What the rule decided for this subject.
  actions: MaintenanceDecidedActions | null;
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

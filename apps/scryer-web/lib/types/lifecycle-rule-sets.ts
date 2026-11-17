/// Lifecycle rules are authored and operated here. What actually happens to
/// a matching title is decided by two controls: the two instance-wide switches
/// and each rule's own state. A rule only acts when both allow it.

/// What a stored rule does. Creation always stores `DISABLED`, so turning a
/// rule on is always a deliberate second step; `DRY_RUN` records matches
/// without changing anything.
export type LifecycleRuleState = "DISABLED" | "DRY_RUN" | "ENABLED";

/// An action a rule declares by defining a rule of that name in its source,
/// as the API names it (`DELETE_FILES`). Any other rule name in the source is
/// the author's own helper. The heads a build knows come from the served
/// output descriptors, so this is any string the server sends.
export type LifecycleActionHead = string;

/// What a rule's source can do, read from the source itself. The value lists
/// hold every literal the source can produce for that head, so a rule with an
/// `else` chain lists each branch.
export type LifecycleDeclaredActions = {
  heads: LifecycleActionHead[];
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
export type LifecycleDecidedActions = {
  heads: LifecycleActionHead[];
  qualityProfile: string | null;
  addTags: string[];
  removeTags: string[];
  library: string | null;
  root: string | null;
  notification: string | null;
};

/// The action kind on a candidate or action run. The server records
/// ACTION_SEQUENCE for every one and puts the steps on its sequence; the other
/// values mirror the rest of the server enum.
export type LifecycleActionKind =
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

export type LifecycleActionStepKind =
  | "UNMONITOR"
  | "DELETE_FILES"
  | "CHANGE_QUALITY_PROFILE"
  | "SEARCH"
  | "ADD_TAGS"
  | "REMOVE_TAGS"
  | "DELETE_TITLE_AND_FILES"
  | "MONITOR"
  | "FETCH_SUBTITLES"
  | "MOVE_TO_LIBRARY"
  | "MOVE_TO_ROOT";

/// The step parameters the progress list shows.
export type LifecycleActionStepParameters = {
  /// The library a move step names, as the rule wrote it.
  targetLibrary: string | null;
  /// The library root a move step names, as the rule wrote it.
  targetRoot: string | null;
};

export type LifecycleActionStep = {
  id: string;
  kind: LifecycleActionStepKind;
  parameters: LifecycleActionStepParameters;
};

export type LifecycleActionSequence = {
  steps: LifecycleActionStep[];
};

export type LifecyclePreviewOutcome = "MATCH" | "NO_MATCH" | "UNKNOWN";

export type LifecycleRuleSetRecord = {
  id: string;
  name: string;
  description: string | null;
  state: LifecycleRuleState;
  /// The rule was enabled, then its source or scope changed, so it
  /// dropped back to a dry run until it is enabled again.
  editedSinceEnabled: boolean;
  /// Shipped with Scryer: name and source are fixed and it cannot be
  /// deleted. State, grace period and libraries stay editable.
  systemOwned: boolean;
  libraryIds: string[];
  currentRevisionNumber: number;
  /// Grace period and declared actions of the revision in force, carried on
  /// the list payload so rendering badges never needs a per-rule detail fetch.
  graceDays: number;
  declaredActions: LifecycleDeclaredActions;
  /// When the rule runs. One run evaluates the rule, then applies its
  /// candidates whose grace period has ended. A rule setting like its
  /// libraries: changing it makes no new revision.
  schedule: import("./scripts").ScriptSchedule;
  /// Null for a manual schedule and for a disabled rule.
  nextRunAt: string | null;
  /// The most titles one run acts on. Required while the source deletes
  /// files or titles; null means no limit.
  maxTitlesPerRun: number | null;
  /// Set when the stored rule could not be loaded for evaluation, as a
  /// reason code; the rule does not run until it is saved again.
  loadRefusal: string | null;
  createdAt: string;
  updatedAt: string;
};

export type LifecycleRuleRevision = {
  id: string;
  ruleSetId: string;
  /// Editor-stripped source: the API removes the package and import lines it
  /// generates, so what round-trips through the editor is what the author wrote.
  regoSource: string;
  revisionNumber: number;
  graceDays: number;
  sourceContentHash: string;
  createdBy: string | null;
  createdAt: string;
};

export type LifecycleRuleSetDetail = {
  ruleSet: LifecycleRuleSetRecord;
  revision: LifecycleRuleRevision;
  declaredActions: LifecycleDeclaredActions;
  /// The simple builder's model, stored opaquely next to the rule. Null when
  /// the rule was never built with it or was detached from it. Read through
  /// `parseRuleBuilderModel`; only the detail query selects it.
  builderModel?: unknown;
};

export type LifecycleRuleSetDraft = {
  name: string;
  description: string;
  /// Declares both which subjects match and what happens to them.
  regoSource: string;
  graceDays: number;
  libraryIds: string[];
  schedule: import("./scripts").ScriptSchedule;
  /// Null for no limit; required while the source deletes files or titles.
  maxTitlesPerRun: number | null;
  /// Set while the rule is still the simple builder's: `regoSource` is then
  /// what the builder generated from this model. Null once the source is
  /// edited by hand, and for every rule the builder did not make.
  builderModel: import("../utils/rule-builder").RuleBuilderModel | null;
};

export type LifecycleValidationResult = {
  valid: boolean;
  errors: string[];
  diagnostics?: import("../utils/rego-diagnostics").RuleValidationDiagnostic[];
  /// Findings that do not block saving, such as a rule nothing references.
  warnings?: import("../utils/rego-diagnostics").RuleValidationDiagnostic[];
  /// Null when the source could not be parsed.
  declaredActions?: LifecycleDeclaredActions | null;
  /// Validation could not be reached, so nothing above describes the source.
  unavailable?: boolean;
};

export type LifecycleTestSubject = { titleId: string; subjectId: string };

export type LifecyclePreviewTitle = {
  excluded: boolean;
  dueAt: string | null;
  dueAtIsEstimate: boolean;
  subjectLabel: string;
  fileCount: number;
  totalSizeBytes: number;
  subjectKind: string;
  subjectId: string;
  titleId: string;
  titleName: string;
  facet: string;
  libraryId: string;
  /// Null when the matcher errored for this title; `error` then carries why.
  outcome: LifecyclePreviewOutcome | null;
  reasonCodes: string[];
  error: string | null;
  /// Stable code for `error`; null exactly when `error` is null.
  errorCode?: string | null;
  /// What the rule decided for this subject. Null when it did not match.
  actions?: LifecycleDecidedActions | null;
  /// The input document the rule evaluated for this subject. Null unless the
  /// caller may manage permissions, because it names who watched and who
  /// requested the subject.
  facts?: Record<string, unknown> | null;
};

export type LifecyclePreviewResult = {
  ruleSetId: string | null;
  sourceContentHash: string;
  evaluatedAt: string;
  titles: LifecyclePreviewTitle[];
};

/// Where a preview run gets its matcher from: the stored revision of a saved
/// rule set, or the unsaved editor draft.
export type LifecyclePreviewSource = "stored" | "draft";

/// Lifecycle state of one candidate. The states past `OBSERVING` are written by
/// the action handler; a dry run rule only ever produces the first three.
export type LifecycleCandidateState =
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
export type LifecycleActionStepRun = {
  state: string;
  holdReason: string | null;
  error: string | null;
};

/// Durable evidence for a job requested by a sequence step. An accepted
/// receipt means the request was accepted; it is not a download completion.
export type LifecycleActionJobReceipt = {
  jobRunId: string | null;
  state: string;
};

/// One configured action sequence step, by id, with its latest checkpoint and
/// receipts. The step itself is read from the sequence.
export type LifecycleActionStepProgress = {
  step: Pick<LifecycleActionStep, "id">;
  run: LifecycleActionStepRun | null;
  receipts: LifecycleActionJobReceipt[];
};

/// One subject's membership in one rule set. Nothing here has acted on the
/// subject: a candidate records that a rule matched it and how much of the
/// grace period is left.
export type LifecycleCandidate = {
  fileCount: number | null;
  totalSizeBytes: number | null;
  subjectLabel: string;
  subjectKind: string;
  subjectId: string;
  id: string;
  ruleSetId: string;
  ruleName: string;
  titleId: string;
  titleName: string;
  state: LifecycleCandidateState;
  stateReason: string;
  reasonCodes: string[];
  /// What the rule decided for this subject.
  actions: LifecycleDecidedActions | null;
  actionSequence: LifecycleActionSequence | null;
  sequenceSteps: LifecycleActionStepProgress[];
  firstMatchedAt: string;
  dueAt: string;
  /// Set while the latest evaluation could not decide, null otherwise.
  heldSince: string | null;
};

/// One rule set's pass through one evaluation run. Runs carry counts rather
/// than subjects, which is why switching lifecycle off does not hide them.
export type LifecycleEvaluationRun = {
  id: string;
  ruleSetId: string;
  revisionNumber: number;
  /// Free-form on purpose: `running`, `succeeded`, `failed`, and whatever the
  /// API adds later. Rendered through a label map that falls back to the raw
  /// value.
  status: string;
  startedAt: string;
  evaluatedCount: number;
  matchedCount: number;
  noMatchCount: number;
  unknownCount: number;
  errorCount: number;
  durationMs: number | null;
  error: string | null;
};

/// One attempt by the action handler to execute one candidate's action.
export type LifecycleActionRun = {
  subjectLabel: string;
  detail: string;
  subjectKind: string;
  id: string;
  titleName: string;
  actionKind: LifecycleActionKind;
  actionSequence: LifecycleActionSequence | null;
  sequenceSteps: LifecycleActionStepProgress[];
  attempt: number;
  /// Free-form for the same reason as an evaluation run's status; includes
  /// `already_satisfied` for an action that found nothing left to do.
  status: string;
  /// Why a safety precondition held the action back, when one did.
  holdReason: string | null;
  error: string | null;
  startedAt: string;
};

/// The two instance-wide switches. A new instance has lifecycle disabled and
/// actions paused, and an enabled rule only acts while both allow it.
export type LifecycleInstanceGates = {
  enabled: boolean;
  actionsPaused: boolean;
};

export type LifecycleGateKey = keyof LifecycleInstanceGates;

/// A subject a lifecycle rule must never act on.
export type LifecycleExclusion = {
  subjectLabel: string;
  subjectKind: string;
  id: string;
  /// Null when the exclusion is global rather than confined to one rule.
  ruleSetId: string | null;
  titleName: string;
  reason: string;
  createdAt: string;
};

/// Outcome of asking for an immediate evaluation or action pass.
export type LifecycleTriggerResult = {
  started: boolean;
  message: string | null;
};

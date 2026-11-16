import { graphQlErrorNumberExtension } from "../graphql/error-message.ts";
import type { RuleBuilderSchema } from "./rule-builder.ts";
import {
  attachedRuleBuilderModel,
  ruleBuilderModelToSave,
  withNewRuleBuilderModel,
  type RuleBuilderBinding,
} from "./rule-builder-mode.ts";
import {
  normalizeScriptSchedule,
  scheduleForKind,
  toScriptScheduleInput,
} from "./script-schedule.ts";
import type { ScriptSchedule } from "../types/scripts.ts";
import type {
  LifecycleActionKind,
  LifecycleActionStepKind,
  LifecycleActionStepProgress,
  LifecycleCandidate,
  LifecycleCandidateState,
  LifecycleDecidedActions,
  LifecycleDeclaredActions,
  LifecyclePreviewOutcome,
  LifecycleRuleSetDetail,
  LifecycleRuleSetDraft,
  LifecycleRuleSetRecord,
  LifecycleRuleState,
} from "@/lib/types/lifecycle-rule-sets";

/// The API caps a preview run at 50 titles and defaults to 20.
export const LIFECYCLE_PREVIEW_LIMIT_MAX = 50;
export const LIFECYCLE_PREVIEW_LIMIT_DEFAULT = 20;

/// Starter source for a new rule. No `package` or `import` line: the API
/// generates both and strips them back off when it hands the source to the
/// editor, so a template carrying them would vanish on the first round trip.
export const LIFECYCLE_STARTER_SOURCE = `# Lifecycle rule. Defining a rule named after an action declares that
# action: unmonitor, monitor, search, fetch_subtitles, delete_files and
# delete_title take \`if { ... }\`; set_quality_profile, move_to_library,
# move_to_root and notify take a string, add_tags and remove_tags a list.
# Any other rule name is your own helper. \`reasons\` explains the choice.
#
# Facts are plain values: \`input.facts.monitored\` is the boolean itself. A
# fact with nothing to report is simply missing, so \`not input.facts.has_file\`
# is how you ask for "no files". If a fact Scryer could not observe is one this
# rule reads, Scryer holds the subject for you — you never write that guard.
# \`days_since(ts)\` turns a timestamp fact into whole days.

unmonitored_with_files if {
	not input.facts.monitored
	input.facts.has_file
}

add_tags := ["needs-review"] if unmonitored_with_files

reasons contains "unmonitored_with_files" if unmonitored_with_files
`;

/// One action as a row shows it: the head plus the values it carries. Heads
/// without a value (unmonitor, search, ...) carry an empty list. The label,
/// order and destructive mark come from the served output descriptors.
export type LifecycleActionEntry = {
  head: string;
  values: string[];
};

/// Everything a rule's source can do, as the server lists it, with every
/// literal each valued head can produce. The API carries a valued head's
/// literals in a field of its own, so only those fields are read here; a head
/// added later is listed without values until the API carries them.
export function declaredActionEntries(
  declared: LifecycleDeclaredActions | null | undefined,
): LifecycleActionEntry[] {
  if (!declared) return [];
  const values: Record<string, string[]> = {
    SET_QUALITY_PROFILE: declared.qualityProfiles,
    ADD_TAGS: declared.addTags,
    REMOVE_TAGS: declared.removeTags,
    MOVE_TO_LIBRARY: declared.libraries,
    MOVE_TO_ROOT: declared.roots,
    NOTIFY: declared.notifications,
  };
  return [...new Set(declared.heads)].map((head) => ({
    head,
    values: [...(values[head] ?? [])],
  }));
}

/// What a rule decided for one subject. The server already lists heads in the
/// order it applies them, so that order is kept as sent.
export function decidedActionEntries(
  decided: LifecycleDecidedActions | null | undefined,
): LifecycleActionEntry[] {
  if (!decided) return [];
  const single = (value: string | null) => (value ? [value] : []);
  const values: Record<string, string[]> = {
    SET_QUALITY_PROFILE: single(decided.qualityProfile),
    ADD_TAGS: decided.addTags,
    REMOVE_TAGS: decided.removeTags,
    MOVE_TO_LIBRARY: single(decided.library),
    MOVE_TO_ROOT: single(decided.root),
    NOTIFY: single(decided.notification),
  };
  return decided.heads.map((head) => ({
    head,
    values: [...(values[head] ?? [])],
  }));
}

/// How often a new rule runs until the operator picks otherwise. The server
/// applies the same default to a rule created without a schedule.
export const LIFECYCLE_DEFAULT_SCHEDULE_SECONDS = 8 * 60 * 60;

export function defaultLifecycleRuleSchedule(): ScriptSchedule {
  return {
    ...scheduleForKind("INTERVAL", null),
    everySeconds: LIFECYCLE_DEFAULT_SCHEDULE_SECONDS,
  };
}

/// A draft written as Rego from the start, as templates and the advanced
/// editor are.
export function initialLifecycleRuleDraft(): LifecycleRuleSetDraft {
  return {
    subjectKind: "TITLE",
    name: "",
    description: "",
    regoSource: LIFECYCLE_STARTER_SOURCE,
    graceDays: 0,
    storageRootId: "",
    libraryIds: [],
    schedule: defaultLifecycleRuleSchedule(),
    maxTitlesPerRun: null,
    builderModel: null,
  };
}

export type LifecycleRuleBuilderBinding =
  RuleBuilderBinding<LifecycleRuleSetDraft>;

/// How lifecycle drafts meet the simple builder: a rule is written for its
/// scope. `schema` is null until the served outputs are read.
export function lifecycleRuleBuilder(
  schema: RuleBuilderSchema | null,
): LifecycleRuleBuilderBinding {
  return {
    schema,
    variant: (draft) => draft.subjectKind,
    starterSource: LIFECYCLE_STARTER_SOURCE,
  };
}

/// A new rule starts in the simple builder with nothing chosen, so it has no
/// source until the builder can write one. Without a schema it starts as Rego.
export function newLifecycleRuleBuilderDraft(
  binding: LifecycleRuleBuilderBinding,
): LifecycleRuleSetDraft {
  return withNewRuleBuilderModel(binding, initialLifecycleRuleDraft());
}

/// A stored model opens in the builder only while it still regenerates the
/// stored source.
export function lifecycleRuleDraftFromDetail(
  detail: LifecycleRuleSetDetail,
  binding: LifecycleRuleBuilderBinding,
): LifecycleRuleSetDraft {
  return {
    subjectKind: detail.ruleSet.subjectKind,
    name: detail.ruleSet.name,
    description: detail.ruleSet.description ?? "",
    regoSource: detail.revision.regoSource,
    graceDays: detail.revision.graceDays,
    storageRootId: detail.revision.storageRootId ?? "",
    libraryIds: [...detail.ruleSet.libraryIds],
    schedule:
      normalizeScriptSchedule(detail.ruleSet.schedule) ??
      defaultLifecycleRuleSchedule(),
    maxTitlesPerRun: detail.ruleSet.maxTitlesPerRun ?? null,
    builderModel: attachedRuleBuilderModel(
      binding.schema,
      detail.builderModel,
      detail.ruleSet.subjectKind,
      detail.revision.regoSource,
    ),
  };
}

export function copyLifecycleRuleDraft(
  detail: LifecycleRuleSetDetail,
  binding: LifecycleRuleBuilderBinding,
): LifecycleRuleSetDraft {
  return {
    ...lifecycleRuleDraftFromDetail(detail, binding),
    name: `Copy of ${detail.ruleSet.name}`,
  };
}

/// The source declares the actions, so create carries no separate action.
export function createLifecycleRuleSetInput(
  draft: LifecycleRuleSetDraft,
  binding: LifecycleRuleBuilderBinding,
) {
  return {
    subjectKind: draft.subjectKind,
    name: draft.name.trim(),
    description: draft.description.trim() || undefined,
    regoSource: draft.regoSource,
    graceDays: draft.graceDays,
    storageRootId: draft.storageRootId || undefined,
    libraryIds: draft.libraryIds.length > 0 ? [...draft.libraryIds] : undefined,
    schedule: toScriptScheduleInput(draft.schedule),
    maxTitlesPerRun: draft.maxTitlesPerRun ?? undefined,
    builderModel: ruleBuilderModelToSave(binding, draft),
  };
}

/// The matcher half without the per-run maximum: what decides whether a
/// save writes a new revision.
function lifecycleRuleMatcherRevision(
  id: string,
  draft: LifecycleRuleSetDraft,
  binding: LifecycleRuleBuilderBinding,
) {
  return {
    id,
    regoSource: draft.regoSource,
    graceDays: draft.graceDays,
    // Always send the explicit editor value. Old clients omit this field and
    // preserve a root they cannot render; this client may intentionally clear it.
    storageRootId: draft.storageRootId || null,
    // Null clears a stored model, so a hand-edited rule stops claiming one.
    // Without a schema nothing could be checked, so the field is left out:
    // the server then keeps the stored model only while the source is
    // unchanged.
    ...(binding.schema
      ? { builderModel: ruleBuilderModelToSave(binding, draft) }
      : {}),
  };
}

/// The per-run maximum travels with the source, so a source that starts to
/// delete is saved together with the limit it needs. Null clears it.
export function updateLifecycleRuleMatcherInput(
  id: string,
  draft: LifecycleRuleSetDraft,
  binding: LifecycleRuleBuilderBinding,
) {
  return {
    ...lifecycleRuleMatcherRevision(id, draft, binding),
    maxTitlesPerRun: draft.maxTitlesPerRun,
  };
}

export function updateLifecycleRuleMetadataInput(
  id: string,
  draft: LifecycleRuleSetDraft,
) {
  return {
    id,
    name: draft.name.trim(),
    description: draft.description.trim() || undefined,
    libraryIds: [...draft.libraryIds],
    schedule: toScriptScheduleInput(draft.schedule),
    maxTitlesPerRun: draft.maxTitlesPerRun,
  };
}

/// What saving an edited rule sends. Each half goes only when the editor
/// changed something it carries, so a schedule, name or library change alone
/// never writes a new matcher revision, and so never drops an Enabled rule
/// to Dry run. The per-run maximum is a rule setting: a change to it alone
/// goes on the metadata half.
export function lifecycleRuleUpdatePlan(
  id: string,
  baseline: LifecycleRuleSetDraft,
  draft: LifecycleRuleSetDraft,
  binding: LifecycleRuleBuilderBinding,
) {
  const metadata = updateLifecycleRuleMetadataInput(id, draft);
  const same = (left: unknown, right: unknown) =>
    JSON.stringify(left) === JSON.stringify(right);
  return {
    matcher: same(
      lifecycleRuleMatcherRevision(id, draft, binding),
      lifecycleRuleMatcherRevision(id, baseline, binding),
    )
      ? null
      : updateLifecycleRuleMatcherInput(id, draft, binding),
    metadata: same(metadata, updateLifecycleRuleMetadataInput(id, baseline))
      ? null
      : metadata,
  };
}

export function clampLifecyclePreviewLimit(limit: number): number {
  if (!Number.isFinite(limit)) {
    return LIFECYCLE_PREVIEW_LIMIT_DEFAULT;
  }
  return Math.min(
    LIFECYCLE_PREVIEW_LIMIT_MAX,
    Math.max(1, Math.trunc(limit)),
  );
}

type LifecyclePreviewInputOptions = {
  ruleSetId?: string | null;
  draft?: LifecycleRuleSetDraft;
  libraryId?: string | null;
  limit?: number;
  titleIds?: string[];
  subjectIds?: string[];
};

/// Build the preview input. The API accepts either a stored rule set or an
/// inline draft as the matcher, and either explicit title ids or a library plus
/// a limit as the subject set; sending both halves of either pair is ambiguous,
/// so a stored rule set wins over a draft and explicit ids win over a library.
export function lifecyclePreviewInput({
  ruleSetId,
  draft,
  libraryId,
  limit,
  titleIds,
  subjectIds,
}: LifecyclePreviewInputOptions) {
  const matcher = ruleSetId
    ? { ruleSetId }
    : draft
      ? {
          subjectKind: draft.subjectKind,
          libraryIds: [...draft.libraryIds],
          regoSource: draft.regoSource,
          graceDays: draft.graceDays,
          storageRootId: draft.storageRootId || undefined,
        }
      : {};
  const subjects =
    titleIds && titleIds.length > 0
      ? {
          titleIds: [...titleIds],
          ...(subjectIds?.length ? { subjectIds: [...subjectIds] } : {}),
        }
      : {
          libraryId: libraryId || undefined,
          limit: clampLifecyclePreviewLimit(
            limit ?? LIFECYCLE_PREVIEW_LIMIT_DEFAULT,
          ),
        };
  return { ...matcher, ...subjects };
}

export function previewOutcomeBadgeTone(
  outcome: LifecyclePreviewOutcome | null,
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

/// Action kinds and step kinds only label candidates and action runs recorded
/// before rules declared their actions in source.
const ACTION_KIND_LABEL_KEYS: Record<LifecycleActionKind, string> = {
  DO_NOTHING: "settings.lifecycleActionDoNothing",
  UNMONITOR_SCOPE_KEEP_FILES:
    "settings.lifecycleActionUnmonitorScopeKeepFiles",
  DELETE_TITLE_AND_FILES: "settings.lifecycleActionDeleteTitleAndFiles",
  UNMONITOR_TITLE_DELETE_ALL_FILES:
    "settings.lifecycleActionUnmonitorTitleDeleteAllFiles",
  UNMONITOR_SHOW_DELETE_EXISTING_FILES:
    "settings.lifecycleActionUnmonitorShowDeleteExistingFiles",
  UNMONITOR_SCOPE_DELETE_FILES:
    "settings.lifecycleActionUnmonitorScopeDeleteFiles",
  UNMONITOR_SEASON_DELETE_FILES_THEN_DELETE_SHOW_IF_EMPTY:
    "settings.lifecycleActionUnmonitorSeasonDeleteFilesThenDeleteShowIfEmpty",
  UNMONITOR_SEASON_THEN_UNMONITOR_SHOW_IF_EMPTY:
    "settings.lifecycleActionUnmonitorSeasonThenUnmonitorShowIfEmpty",
  CHANGE_QUALITY_PROFILE_AND_SEARCH_IF_CHANGED:
    "settings.lifecycleActionChangeQualityProfileAndSearchIfChanged",
  ADD_TAGS: "settings.lifecycleActionAddTags",
  REMOVE_TAGS: "settings.lifecycleActionRemoveTags",
  ACTION_SEQUENCE: "settings.lifecycleActionSequence",
};

/// Translation key for an action kind, or null when the API sends a kind this
/// build does not know. Callers render the raw kind rather than an empty cell.
export function actionKindLabelKey(kind: string): string | null {
  return ACTION_KIND_LABEL_KEYS[kind as LifecycleActionKind] ?? null;
}

const ACTION_STEP_KIND_LABEL_KEYS: Record<LifecycleActionStepKind, string> = {
  UNMONITOR: "settings.lifecycleSequenceStepUnmonitor",
  DELETE_FILES: "settings.lifecycleSequenceStepDeleteFiles",
  CHANGE_QUALITY_PROFILE:
    "settings.lifecycleSequenceStepChangeQualityProfile",
  SEARCH: "settings.lifecycleSequenceStepSearch",
  ADD_TAGS: "settings.lifecycleSequenceStepAddTags",
  REMOVE_TAGS: "settings.lifecycleSequenceStepRemoveTags",
  DELETE_TITLE_AND_FILES: "settings.lifecycleSequenceStepDeleteTitleAndFiles",
  MONITOR: "settings.lifecycleSequenceStepMonitor",
  FETCH_SUBTITLES: "settings.lifecycleSequenceStepFetchSubtitles",
  MOVE_TO_LIBRARY: "settings.lifecycleSequenceStepMoveToLibrary",
  MOVE_TO_ROOT: "settings.lifecycleSequenceStepMoveToRoot",
};

export function actionStepKindLabelKey(kind: string): string | null {
  return ACTION_STEP_KIND_LABEL_KEYS[kind as LifecycleActionStepKind] ?? null;
}

const PREVIEW_OUTCOME_LABEL_KEYS: Record<LifecyclePreviewOutcome, string> = {
  MATCH: "settings.lifecyclePreviewOutcomeMatch",
  NO_MATCH: "settings.lifecyclePreviewOutcomeNoMatch",
  UNKNOWN: "settings.lifecyclePreviewOutcomeUnknown",
};

export function previewOutcomeLabelKey(outcome: string): string | null {
  return (
    PREVIEW_OUTCOME_LABEL_KEYS[outcome as LifecyclePreviewOutcome] ?? null
  );
}

// ── Operating a rule: state and switches ──────────────────────────────

const RULE_STATE_KEYS: Record<
  LifecycleRuleState,
  { label: string; help: string }
> = {
  DISABLED: {
    label: "settings.lifecycleRuleStateDisabled",
    help: "settings.lifecycleRuleStateDisabledHelp",
  },
  DRY_RUN: {
    label: "settings.lifecycleRuleStateDryRun",
    help: "settings.lifecycleRuleStateDryRunHelp",
  },
  ENABLED: {
    label: "settings.lifecycleRuleStateEnabled",
    help: "settings.lifecycleRuleStateEnabledHelp",
  },
};

export function ruleStateLabelKey(state: string): string | null {
  return RULE_STATE_KEYS[state as LifecycleRuleState]?.label ?? null;
}

export function ruleStateHelpKey(state: string): string | null {
  return RULE_STATE_KEYS[state as LifecycleRuleState]?.help ?? null;
}

/// A rule whose action never changes anything can only run as a dry run, so
/// `ENABLED` is not offered for it.
export function ruleStateOptionsFor(
  record: Pick<LifecycleRuleSetRecord, "enabledArming">,
): LifecycleRuleState[] {
  return record.enabledArming === "NONE"
    ? ["DISABLED", "DRY_RUN"]
    : ["DISABLED", "DRY_RUN", "ENABLED"];
}

/// Enabling a rule whose source declares `delete_files` or `delete_title` is
/// the one state change that confirms first, against the candidates it would
/// act on.
export function ruleStateNeedsConfirmation(
  record: Pick<LifecycleRuleSetRecord, "declaredActions">,
  state: LifecycleRuleState,
): boolean {
  return state === "ENABLED" && record.declaredActions.destructive;
}

/// Sentinel for "no filter". The select primitive refuses an empty-string
/// option value, and `all` is how the rest of the settings filters spell it.
export const LIFECYCLE_FILTER_ALL = "all";

/// Translate a filter select's value into the query argument: the sentinel and
/// the empty string both mean "send nothing and let the API decide".
export function lifecycleFilterArgument(value: string): string | undefined {
  return !value || value === LIFECYCLE_FILTER_ALL ? undefined : value;
}

// ── Candidates ────────────────────────────────────────────────────────

const CANDIDATE_STATE_LABEL_KEYS: Record<LifecycleCandidateState, string> = {
  OBSERVING: "settings.lifecycleCandidateStateObserving",
  PENDING_ACTION: "settings.lifecycleCandidateStatePendingAction",
  DUE: "settings.lifecycleCandidateStateDue",
  EXECUTING: "settings.lifecycleCandidateStateExecuting",
  SUCCEEDED: "settings.lifecycleCandidateStateSucceeded",
  FAILED: "settings.lifecycleCandidateStateFailed",
  CANCELED: "settings.lifecycleCandidateStateCanceled",
  EXCLUDED: "settings.lifecycleCandidateStateExcluded",
  BLOCKED: "settings.lifecycleCandidateStateBlocked",
};

export function candidateStateLabelKey(state: string): string | null {
  return CANDIDATE_STATE_LABEL_KEYS[state as LifecycleCandidateState] ?? null;
}

/// Tone map for the candidate lifecycle. `BLOCKED` is a warning rather than a
/// failure: a safety precondition refused the action, which is the system
/// working, but it is also the state an operator has to go look at.
export function candidateStateBadgeTone(
  state: LifecycleCandidateState | string,
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

/// Candidate states that are still moving. Enabling a rule that deletes files
/// has to acknowledge exactly these, because they are the ones the handler
/// could still act on.
export const LIFECYCLE_NON_TERMINAL_CANDIDATE_STATES = [
  "OBSERVING",
  "PENDING_ACTION",
  "DUE",
  "EXECUTING",
  "BLOCKED",
] as const satisfies readonly LifecycleCandidateState[];

export function isNonTerminalCandidateState(state: string): boolean {
  return (
    LIFECYCLE_NON_TERMINAL_CANDIDATE_STATES as readonly string[]
  ).includes(state);
}

/// How many of a rule's candidates the handler could still act on. This is the
/// number enabling a deleting rule makes the operator acknowledge.
export function nonTerminalCandidateCount(
  candidates: Pick<LifecycleCandidate, "state">[],
): number {
  return candidates.filter((candidate) =>
    isNonTerminalCandidateState(candidate.state),
  ).length;
}

const MINUTE_MS = 60_000;
const HOUR_MS = 60 * MINUTE_MS;
const DAY_MS = 24 * HOUR_MS;

export type LifecycleCountdown = {
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
export function lifecycleCountdown(
  dueAt: string,
  now: number = Date.now(),
): LifecycleCountdown | null {
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
      labelKey: "settings.lifecycleCountdownDueNow",
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
    labelKey: `settings.lifecycleCountdown${overdue ? "Overdue" : "In"}${unit.suffix}`,
    values: { count: unit.count },
  };
}

// ── Runs ──────────────────────────────────────────────────────────────

const RUN_STATUS_LABEL_KEYS: Record<string, string> = {
  running: "settings.lifecycleRunStatusRunning",
  succeeded: "settings.lifecycleRunStatusSucceeded",
  failed: "settings.lifecycleRunStatusFailed",
  held: "settings.lifecycleRunStatusHeld",
  already_satisfied: "settings.lifecycleRunStatusAlreadySatisfied",
  skipped: "settings.lifecycleRunStatusSkipped",
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

/// Search is only recorded as requested when its durable receipt was accepted.
/// A configured but unstarted step stays waiting, and a conditional search can
/// truthfully remain skipped without being mistaken for a dispatched job.
export function lifecycleSearchHistoryState(
  progress: LifecycleActionStepProgress | undefined,
): string {
  if (
    progress?.receipts.some(
      (receipt) =>
        receipt.state === "accepted" || receipt.state === "completed",
    )
  ) {
    return "accepted";
  }
  return progress?.run?.state.toLowerCase() ?? "waiting";
}

/** Candidate count travels as a numeric extension beside the stable refusal. */
export function parseAcknowledgedCandidateCountMismatch(
  error: unknown,
): number | null {
  return graphQlErrorNumberExtension(
    error,
    "VALIDATION_ERROR",
    "LIFECYCLE_ARM_COUNT_MISMATCH",
    "currentCount",
  );
}

export function setLifecycleRuleStateVariables(
  record: Pick<
    LifecycleRuleSetRecord,
    "id" | "declaredActions" | "currentRevisionNumber" | "libraryIds"
  >,
  state: LifecycleRuleState,
  candidateCount?: number,
) {
  return {
    ruleSetId: record.id,
    state,
    /// Only enabling a rule that deletes files carries a confirmation; sending
    /// one otherwise would invite the server to check a number the UI never
    /// showed anyone.
    confirmation: ruleStateNeedsConfirmation(record, state)
      ? {
          revisionNumber: record.currentRevisionNumber,
          libraryIds: record.libraryIds.slice(),
          candidateCount: candidateCount ?? 0,
        }
      : undefined,
  };
}

export function excludeLifecycleSubjectInput(options: {
  titleId: string;
  subjectKind?: string;
  subjectId?: string;
  ruleSetId?: string | null;
  reason?: string;
}) {
  const reason = options.reason?.trim();
  return {
    titleId: options.titleId,
    ...(options.subjectKind
      ? { subjectKind: options.subjectKind.toUpperCase() }
      : {}),
    ...(options.subjectId ? { subjectId: options.subjectId } : {}),
    ruleSetId: options.ruleSetId || undefined,
    reason: reason || undefined,
  };
}

/// Codes the server reports when a stored rule cannot be loaded for
/// evaluation. Any other code reads as the generic notice.
const LIFECYCLE_LOAD_REFUSAL_KEYS: Readonly<Record<string, string>> = {
  LIFECYCLE_RULE_STORED_ACTIONS_NOT_DECLARED:
    "error.reason.LIFECYCLE_RULE_STORED_ACTIONS_NOT_DECLARED",
  LIFECYCLE_RULE_ARMED_WITHOUT_MAX_TITLES_PER_RUN:
    "error.reason.LIFECYCLE_RULE_ARMED_WITHOUT_MAX_TITLES_PER_RUN",
};

export function lifecycleLoadRefusalKey(code: string): string {
  return (
    LIFECYCLE_LOAD_REFUSAL_KEYS[code] ?? "settings.lifecycleRuleLoadRefused"
  );
}

/// Preview failures the web words itself. The evaluator's own failures carry
/// the engine's detail, which only the server's text has.
const LIFECYCLE_PREVIEW_ERROR_KEYS: Readonly<Record<string, string>> = {
  LIFECYCLE_PREVIEW_NO_DECISION:
    "error.reason.LIFECYCLE_PREVIEW_NO_DECISION",
};

export function lifecyclePreviewErrorText(
  title: { error: string | null; errorCode?: string | null },
  translate: (key: string) => string,
): string {
  const key = title.errorCode
    ? LIFECYCLE_PREVIEW_ERROR_KEYS[title.errorCode]
    : undefined;
  return key ? translate(key) : (title.error ?? "");
}

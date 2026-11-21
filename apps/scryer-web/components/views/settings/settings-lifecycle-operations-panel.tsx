import { formatBytes } from "@/lib/utils/activity-utils";
import {
  lifecycleExecutionCauseText,
  lifecycleExecutionReasonText,
} from "@/lib/utils/lifecycle-execution-reason";
import { useState } from "react";
import type * as React from "react";
import { Ban, Play, Trash2, Zap } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { IconButton } from "@/components/ui/icon-button";
import { TableDisclosureButton } from "@/components/ui/table-disclosure-button";
import { Label } from "@/components/ui/label";
import { SettingsToggleSwitch } from "@/components/common/settings-toggle-switch";
import { SingleSelectField } from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useTranslate } from "@/lib/context/translate-context";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import type { Translate } from "@/components/root/types";
import type { UiDateTimeFormat } from "@/lib/types/settings";
import type {
  LifecycleActionRun,
  LifecycleActionSequence,
  LifecycleActionStepProgress,
  LifecycleCandidate,
  LifecycleCandidateState,
  LifecycleEvaluationRun,
  LifecycleExclusion,
  LifecycleGateKey,
  LifecycleInstanceGates,
  LifecycleRuleSetRecord,
} from "@/lib/types/lifecycle-rule-sets";
import {
  LIFECYCLE_FILTER_ALL,
  actionKindLabelKey,
  actionStepKindLabelKey,
  candidateStateBadgeTone,
  candidateStateLabelKey,
  lifecycleSearchHistoryState,
  lifecycleCountdown,
  runStatusBadgeTone,
  runStatusLabelKey,
} from "@/lib/utils/lifecycle-rule-sets";
import { formatUiDateTime } from "@/lib/utils/date-format";
import { selectorId } from "@/lib/utils/dom-ids";
import { parseLifecycleFileResults } from "@/lib/utils/lifecycle-file-results";
import { LoadingMark } from "@/components/common/loading-mark";
import { InfoHelp } from "@/components/common/info-help";
import {
  PaneEmpty,
  PaneError,
  PaneSection,
  RefreshButton,
  TABLE_SHELL,
  TOOLBAR_SELECT_LABEL,
  TOOLBAR_SHELL,
} from "@/components/views/settings/rule-pane-section";
import { LifecycleDecidedActionsSummary } from "@/components/views/settings/lifecycle-action-summary";

export type LifecycleLibraryOption = { id: string; name: string };

/// Every candidate lifecycle state, in the order the filter offers them. Kept
/// here rather than derived from the loaded rows so the filter does not shrink
/// to whatever happens to be on screen.
const CANDIDATE_STATE_FILTER_OPTIONS: LifecycleCandidateState[] = [
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

function timestamp(
  value: string | null | undefined,
  format: UiDateTimeFormat,
): string {
  if (!value) return "—";
  return formatUiDateTime(value, format, { fallback: value });
}

/// An action run names its action by kind, with the steps of its sequence
/// shown beside it.
function ActionKindLabel({ kind }: { kind: string }) {
  const t = useTranslate();
  const labelKey = actionKindLabelKey(kind);
  return <>{labelKey ? t(labelKey) : kind}</>;
}

function LifecycleSequenceProgress({
  sequence,
  progress,
}: {
  sequence: LifecycleActionSequence | null;
  progress: LifecycleActionStepProgress[];
}) {
  const t = useTranslate();
  if (sequence === null) return null;
  if (sequence.steps.length === 0) {
    return (
      <span className="text-xs text-muted-foreground">
        {t("settings.lifecycleSequenceObserveOnly")}
      </span>
    );
  }

  const progressByStepId = new Map(
    progress.map((entry) => [entry.step.id, entry]),
  );
  return (
    <ol className="mt-1 space-y-1 text-xs text-muted-foreground">
      {sequence.steps.map((step, index) => {
        const entry = progressByStepId.get(step.id);
        const acceptedReceipt = entry?.receipts.find(
          (item) => item.state === "accepted" || item.state === "completed",
        );
        const searchState = lifecycleSearchHistoryState(entry);
        const stepLabelKey = actionStepKindLabelKey(step.kind);
        return (
          <li
            key={step.id}
            className="flex flex-wrap items-center gap-x-1.5 gap-y-0.5"
          >
            <span className="font-medium text-foreground">
              {index + 1}. {stepLabelKey ? t(stepLabelKey) : step.kind}
            </span>
            {step.parameters.targetLibrary || step.parameters.targetRoot ? (
              <span>
                · {step.parameters.targetLibrary || step.parameters.targetRoot}
              </span>
            ) : null}
            {step.kind === "SEARCH" ? (
              acceptedReceipt ? (
                <>
                  <span>
                    {t("settings.lifecycleSequenceHistorySearchRequested")}
                  </span>
                  <span>·</span>
                  <span>
                    {t("settings.lifecycleSequenceHistorySearchAccepted")}
                  </span>
                  {acceptedReceipt.jobRunId ? (
                    <a
                      className="text-primary underline-offset-2 hover:underline"
                      href={`/system/jobs?jobRun=${encodeURIComponent(acceptedReceipt.jobRunId)}`}
                    >
                      {t("settings.lifecycleSequenceHistoryJob", {
                        id: acceptedReceipt.jobRunId,
                      })}
                    </a>
                  ) : null}
                </>
              ) : searchState === "waiting" ? (
                <span>· {t("settings.lifecycleSequenceHistoryWaiting")}</span>
              ) : (
                <>
                  <span>·</span>
                  <RunStatusBadge status={searchState} />
                </>
              )
            ) : entry?.run ? (
              <>
                <span>·</span>
                <RunStatusBadge status={entry.run.state} />
              </>
            ) : (
              <span>· {t("settings.lifecycleSequenceHistoryWaiting")}</span>
            )}
            {entry?.run?.holdReason ? (
              <span className="text-[var(--scry-warning-text)]">
                {lifecycleExecutionReasonText(t, entry.run.holdReason)}
              </span>
            ) : null}
            {entry?.run?.causeCode ? (
              <span className="text-[var(--scry-warning-text)]">
                {lifecycleExecutionCauseText(t, entry.run.causeCode)}
              </span>
            ) : null}
            {entry?.run?.error ? (
              <span className="text-[var(--scry-danger-text)]">
                {entry.run.error}
              </span>
            ) : null}
          </li>
        );
      })}
    </ol>
  );
}

/// Run statuses are free-form strings on the API. An unrecognized one renders
/// as itself instead of vanishing into an empty cell.
function RunStatusBadge({ status }: { status: string }) {
  const t = useTranslate();
  const labelKey = runStatusLabelKey(status);
  return (
    <Badge tone={runStatusBadgeTone(status)}>
      {labelKey ? t(labelKey) : status}
    </Badge>
  );
}

function ReasonCodes({ codes }: { codes: string[] }) {
  if (codes.length === 0) return <>—</>;
  return (
    <div className="flex flex-wrap gap-1">
      {codes.map((code) => (
        <code
          key={code}
          data-code-font
          className="rounded bg-muted px-1 py-0.5 text-xs"
        >
          {code}
        </code>
      ))}
    </div>
  );
}

// ── Instance switches ─────────────────────────────────────────────────

/// The two switches every lifecycle pane shows above its content, in order.
const INSTANCE_SWITCHES: {
  key: LifecycleGateKey;
  label: string;
  help: string;
}[] = [
  {
    key: "enabled",
    label: "settings.lifecycleSwitchEnabled",
    help: "settings.lifecycleSwitchEnabledHelp",
  },
  {
    key: "actionsPaused",
    label: "settings.lifecycleSwitchActionsPaused",
    help: "settings.lifecycleSwitchActionsPausedHelp",
  },
];

export type LifecycleInstanceSwitchesProps = {
  gates: LifecycleInstanceGates | null;
  /// True when the switches query was refused, which on this page means the
  /// reader is not a system administrator.
  gatesLocked: boolean;
  savingSwitch: LifecycleGateKey | null;
  onSwitchChange: (key: LifecycleGateKey, value: boolean) => void;
};

export function LifecycleInstanceSwitches({
  gates,
  gatesLocked,
  savingSwitch,
  onSwitchChange,
}: LifecycleInstanceSwitchesProps) {
  const t = useTranslate();

  if (gatesLocked || !gates) {
    return (
      <p
        id="settings-lifecycle-switches-locked"
        className="rounded border border-[var(--scry-info-border)] bg-[var(--scry-info-bg)] px-3 py-2 text-xs text-[var(--scry-info-text)]"
      >
        {t("settings.lifecycleSwitchesLocked")}
      </p>
    );
  }
  return (
    <div id="settings-lifecycle-switches" className="grid gap-3 md:grid-cols-2">
      {INSTANCE_SWITCHES.map(({ key, label, help }) => (
        <div
          key={key}
          className="flex items-center justify-between gap-4 rounded-[14px] border border-[var(--scry-border3)] bg-[var(--scry-surfC)] px-5 py-4"
        >
          <div className="min-w-0 space-y-1">
            <Label
              htmlFor={selectorId("settings-lifecycle-switch", key)}
              className="font-display text-[16px] font-bold text-[var(--scry-ink)]"
            >
              {t(label)}
            </Label>
            <p className="text-[13px] text-[var(--scry-muted)]">{t(help)}</p>
          </div>
          <div className="flex shrink-0 items-center gap-2">
            {savingSwitch === key ? (
              <LoadingMark className="h-4 w-4 text-muted-foreground" />
            ) : null}
            <SettingsToggleSwitch
              id={selectorId("settings-lifecycle-switch", key)}
              size="lg"
              checked={gates[key]}
              disabled={savingSwitch !== null}
              ariaLabel={t(label)}
              onChange={(next) => onSwitchChange(key, next)}
            />
          </div>
        </div>
      ))}
    </div>
  );
}

/// A title, and under it the season or episode a row is about.
function SubjectCell({
  titleName,
  subjectKind,
  subjectLabel,
  children,
}: {
  titleName: string;
  subjectKind: string;
  subjectLabel: string;
  children?: React.ReactNode;
}) {
  return (
    <div className="flex min-w-0 flex-col gap-0.5">
      <span className="text-[13.5px] font-medium text-[var(--scry-ink2)]">
        {titleName}
      </span>
      {subjectKind !== "title" ? (
        <span className="text-xs text-[var(--scry-muted)]">{subjectLabel}</span>
      ) : null}
      {children}
    </div>
  );
}

// ── Candidates ────────────────────────────────────────────────────────

function CandidateDueCell({ dueAt }: { dueAt: string }) {
  const t = useTranslate();
  const countdown = lifecycleCountdown(dueAt);
  if (!countdown) {
    return <span className="text-muted-foreground">{dueAt}</span>;
  }
  return (
    <span
      className={
        countdown.overdue
          ? "font-medium text-[var(--scry-warning-text)]"
          : "text-[var(--scry-ink2)]"
      }
    >
      {t(countdown.labelKey, countdown.values)}
    </span>
  );
}

export type LifecycleCandidatesPanelProps = {
  candidates: LifecycleCandidate[];
  candidatesLoading: boolean;
  candidatesError: string | null;
  ruleSetRecords: LifecycleRuleSetRecord[];
  libraries: LifecycleLibraryOption[];
  /// Null for a reader who cannot see the instance switches; the empty state
  /// then stops guessing why the list is empty.
  gates: LifecycleInstanceGates | null;
  candidateRuleFilter: string;
  setCandidateRuleFilter: (id: string) => void;
  candidateStateFilter: string;
  setCandidateStateFilter: (state: string) => void;
  candidateLibraryFilter: string;
  setCandidateLibraryFilter: (id: string) => void;
  refreshCandidates: () => void;
  excludeCandidate: (candidate: LifecycleCandidate) => void;
};

function candidatesEmptyKey(
  gates: LifecycleInstanceGates | null,
  ruleSetRecords: LifecycleRuleSetRecord[],
): string {
  if (gates && !gates.enabled) {
    return "settings.lifecycleCandidatesEmptyLifecycleOff";
  }
  if (!ruleSetRecords.some((record) => record.state !== "DISABLED")) {
    return "settings.lifecycleCandidatesEmptyAllDisabled";
  }
  return "settings.lifecycleCandidatesEmpty";
}

/// A candidate opens to the detail its row has no room for: why it matched,
/// why it is held, and how far its action sequence has got.
function candidateHasDetail(candidate: LifecycleCandidate): boolean {
  return (
    candidate.reasonCodes.length > 0 ||
    Boolean(candidate.stateReason) ||
    Boolean(candidate.actionSequence?.steps.length)
  );
}

function CandidateDetail({ candidate }: { candidate: LifecycleCandidate }) {
  const t = useTranslate();
  return (
    <div className="grid gap-4 py-1 md:grid-cols-2">
      <div className="space-y-1.5">
        <p className="text-xs font-medium text-[var(--scry-muted)]">
          {t("settings.lifecyclePreviewColReasons")}
        </p>
        <ReasonCodes codes={candidate.reasonCodes} />
        {candidate.stateReason ? (
          <p className="text-xs text-[var(--scry-warning-text)]">
            {lifecycleExecutionReasonText(t, candidate.stateReason)}
          </p>
        ) : null}
        {candidate.stateCauseCode ? (
          <p className="text-xs text-[var(--scry-warning-text)]">
            {lifecycleExecutionCauseText(t, candidate.stateCauseCode)}
          </p>
        ) : null}
      </div>
      {candidate.actionSequence?.steps.length ? (
        <div className="space-y-1.5">
          <p className="text-xs font-medium text-[var(--scry-muted)]">
            {t("settings.lifecycleRuleAction")}
          </p>
          <LifecycleSequenceProgress
            sequence={candidate.actionSequence}
            progress={candidate.sequenceSteps}
          />
        </div>
      ) : null}
    </div>
  );
}

function CandidateRows({
  candidate,
  expanded,
  onToggle,
  excludeCandidate,
}: {
  candidate: LifecycleCandidate;
  expanded: boolean;
  onToggle: () => void;
  excludeCandidate: (candidate: LifecycleCandidate) => void;
}) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const stateKey = candidateStateLabelKey(candidate.state);
  const stateReason = candidate.stateReason
    ? lifecycleExecutionReasonText(t, candidate.stateReason)
    : undefined;
  const detailId = selectorId(
    "settings-lifecycle-candidate-detail",
    candidate.id,
  );
  const hasDetail = candidateHasDetail(candidate);
  return (
    <>
      <TableRow
        data-ui="settings-table-row"
        id={selectorId("settings-lifecycle-candidate-row", candidate.id)}
        className={expanded ? "border-b-0" : undefined}
      >
        <TableCell className="w-[1%] pr-0">
          {hasDetail ? (
            <TableDisclosureButton
              id={selectorId(
                "settings-lifecycle-candidate-toggle",
                candidate.id,
              )}
              expanded={expanded}
              aria-controls={expanded ? detailId : undefined}
              aria-label={candidate.titleName}
              onClick={onToggle}
            />
          ) : null}
        </TableCell>
        <TableCell>
          <SubjectCell
            titleName={candidate.titleName}
            subjectKind={candidate.subjectKind}
            subjectLabel={candidate.subjectLabel}
          >
            {candidate.fileCount != null ? (
              <span className="text-xs text-[var(--scry-muted)]">
                {t("settings.lifecycleFileSummary", {
                  count: candidate.fileCount,
                  size: formatBytes(candidate.totalSizeBytes),
                })}
              </span>
            ) : null}
          </SubjectCell>
        </TableCell>
        <TableCell>
          <div className="flex flex-col gap-1">
            <span className="text-[13px] text-[var(--scry-ink2)]">
              {candidate.ruleName}
            </span>
            {candidate.actions ? (
              <LifecycleDecidedActionsSummary
                actions={candidate.actions}
                idPrefix={selectorId(
                  "settings-lifecycle-candidate-action",
                  candidate.id,
                )}
              />
            ) : null}
          </div>
        </TableCell>
        <TableCell>
          <div className="flex flex-col gap-1">
            <Badge
              tone={candidateStateBadgeTone(candidate.state)}
              title={stateReason}
            >
              {stateKey ? t(stateKey) : candidate.state}
            </Badge>
            {candidate.heldSince ? (
              <span
                className="whitespace-nowrap text-xs text-[var(--scry-warning-text)]"
                title={stateReason}
              >
                {t("settings.lifecycleCandidatesHeldSince", {
                  time: timestamp(candidate.heldSince, dateTimeFormat),
                })}
              </span>
            ) : null}
          </div>
        </TableCell>
        <TableCell className="hidden whitespace-nowrap text-[var(--scry-muted)] lg:table-cell">
          {timestamp(candidate.firstMatchedAt, dateTimeFormat)}
        </TableCell>
        <TableCell
          className="whitespace-nowrap"
          title={timestamp(candidate.dueAt, dateTimeFormat)}
        >
          <CandidateDueCell dueAt={candidate.dueAt} />
        </TableCell>
        <TableCell className="w-[1%] text-right">
          <IconButton
            id={selectorId(
              "settings-lifecycle-candidate-exclude",
              candidate.id,
            )}
            label={t("settings.lifecycleCandidateExclude")}
            tone="delete"
            onClick={() => excludeCandidate(candidate)}
          >
            <Ban className="h-4 w-4" />
          </IconButton>
        </TableCell>
      </TableRow>
      {expanded && hasDetail ? (
        <TableRow id={detailId} className="bg-[var(--scry-surfC)]">
          <TableCell />
          <TableCell colSpan={6}>
            <CandidateDetail candidate={candidate} />
          </TableCell>
        </TableRow>
      ) : null}
    </>
  );
}

export function LifecycleCandidatesPanel({
  candidates,
  candidatesLoading,
  candidatesError,
  ruleSetRecords,
  libraries,
  gates,
  candidateRuleFilter,
  setCandidateRuleFilter,
  candidateStateFilter,
  setCandidateStateFilter,
  candidateLibraryFilter,
  setCandidateLibraryFilter,
  refreshCandidates,
  excludeCandidate,
}: LifecycleCandidatesPanelProps) {
  const t = useTranslate();
  const [expandedIds, setExpandedIds] = useState<ReadonlySet<string>>(
    () => new Set(),
  );
  const toggle = (id: string) =>
    setExpandedIds((current) => {
      const next = new Set(current);
      if (!next.delete(id)) next.add(id);
      return next;
    });

  return (
    <PaneSection
      id="settings-lifecycle-candidates"
      title={t("settings.lifecycleCandidatesTitle")}
      help={t("settings.lifecycleCandidatesSubtitle")}
      count={
        candidatesLoading && candidates.length === 0
          ? undefined
          : candidates.length
      }
      actions={
        <RefreshButton
          id="settings-lifecycle-candidates-refresh"
          loading={candidatesLoading}
          onClick={refreshCandidates}
        />
      }
    >
      <div className={TOOLBAR_SHELL}>
        <SingleSelectField
          id="settings-lifecycle-candidates-rule"
          label={t("settings.lifecycleCandidatesFilterRule")}
          labelClassName={TOOLBAR_SELECT_LABEL}
          size="compact"
          chrome="toolbar"
          className="md:w-[240px]"
          value={candidateRuleFilter}
          onValueChange={setCandidateRuleFilter}
          options={[
            {
              value: LIFECYCLE_FILTER_ALL,
              label: t("settings.lifecycleCandidatesAllRules"),
            },
            ...ruleSetRecords.map((record) => ({
              value: record.id,
              label: record.name,
            })),
          ]}
        />
        <SingleSelectField
          id="settings-lifecycle-candidates-state"
          label={t("settings.lifecycleCandidatesFilterState")}
          labelClassName={TOOLBAR_SELECT_LABEL}
          size="compact"
          chrome="toolbar"
          className="md:w-[200px]"
          value={candidateStateFilter}
          onValueChange={setCandidateStateFilter}
          options={[
            {
              value: LIFECYCLE_FILTER_ALL,
              label: t("settings.lifecycleCandidatesAllStates"),
            },
            ...CANDIDATE_STATE_FILTER_OPTIONS.map((state) => {
              const labelKey = candidateStateLabelKey(state);
              return { value: state, label: labelKey ? t(labelKey) : state };
            }),
          ]}
        />
        <SingleSelectField
          id="settings-lifecycle-candidates-library"
          label={t("settings.lifecycleCandidatesFilterLibrary")}
          labelClassName={TOOLBAR_SELECT_LABEL}
          size="compact"
          chrome="toolbar"
          className="md:w-[200px]"
          value={candidateLibraryFilter}
          onValueChange={setCandidateLibraryFilter}
          options={[
            {
              value: LIFECYCLE_FILTER_ALL,
              label: t("settings.lifecycleCandidatesAllLibraries"),
            },
            ...libraries.map((library) => ({
              value: library.id,
              label: library.name,
            })),
          ]}
        />
      </div>

      <PaneError message={candidatesError} />

      {candidates.length === 0 ? (
        <PaneEmpty
          id="settings-lifecycle-candidates-empty"
          loading={candidatesLoading}
        >
          {t(candidatesEmptyKey(gates, ruleSetRecords))}
        </PaneEmpty>
      ) : (
        <Table
          id="settings-lifecycle-candidates-table"
          wrapperClassName={TABLE_SHELL}
        >
          <TableHeader>
            <TableRow>
              <TableHead className="w-[1%]" />
              <TableHead>{t("label.title")}</TableHead>
              <TableHead>{t("settings.lifecycleCandidatesColRule")}</TableHead>
              <TableHead>{t("settings.lifecycleCandidatesColState")}</TableHead>
              <TableHead className="hidden whitespace-nowrap lg:table-cell">
                {t("settings.lifecycleCandidatesColFirstMatched")}
              </TableHead>
              <TableHead className="whitespace-nowrap">
                {t("settings.lifecycleCandidatesColDue")}
              </TableHead>
              <TableHead className="w-[1%] text-right">
                {t("label.actions")}
              </TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {candidates.map((candidate) => (
              <CandidateRows
                key={candidate.id}
                candidate={candidate}
                expanded={expandedIds.has(candidate.id)}
                onToggle={() => toggle(candidate.id)}
                excludeCandidate={excludeCandidate}
              />
            ))}
          </TableBody>
        </Table>
      )}
    </PaneSection>
  );
}

// ── Runs and history ──────────────────────────────────────────────────

function runCounts(run: LifecycleEvaluationRun, t: Translate): string {
  return t("settings.lifecycleRunCounts", {
    evaluated: run.evaluatedCount,
    matched: run.matchedCount,
    noMatch: run.noMatchCount,
    unknown: run.unknownCount,
    errors: run.errorCount,
  });
}

/// A run's status, with the hold reason and error that explain it under the
/// badge rather than in columns of their own that are empty on most rows.
function RunOutcome({
  status,
  holdReason,
  causeCode,
  error,
  children,
}: {
  status: string;
  holdReason?: string | null;
  causeCode?: string | null;
  error: string | null;
  children?: React.ReactNode;
}) {
  const t = useTranslate();
  return (
    <div className="flex max-w-[420px] flex-col gap-1">
      <RunStatusBadge status={status} />
      {holdReason ? (
        <span className="text-xs text-[var(--scry-warning-text)]">
          {lifecycleExecutionReasonText(t, holdReason)}
        </span>
      ) : null}
      {causeCode ? (
        <span className="text-xs text-[var(--scry-warning-text)]">
          {lifecycleExecutionCauseText(t, causeCode)}
        </span>
      ) : null}
      {error ? (
        <span
          className="line-clamp-3 whitespace-normal text-xs text-[var(--scry-danger-text)]"
          title={error}
        >
          {error}
        </span>
      ) : null}
      {children}
    </div>
  );
}

export type LifecycleRunsPanelProps = {
  evaluationRuns: LifecycleEvaluationRun[];
  actionRuns: LifecycleActionRun[];
  runsError: string | null;
  ruleSetRecords: LifecycleRuleSetRecord[];
  runScopeRuleSetId: string;
  setRunScopeRuleSetId: (id: string) => void;
  evaluationTriggering: boolean;
  actionTriggering: boolean;
  runEvaluationNow: () => void;
  runActionHandlerNow: () => void;
  refreshRuns: () => void;
  runsLoading: boolean;
};

export function LifecycleRunsPanel({
  evaluationRuns,
  actionRuns,
  runsError,
  ruleSetRecords,
  runScopeRuleSetId,
  setRunScopeRuleSetId,
  evaluationTriggering,
  actionTriggering,
  runEvaluationNow,
  runActionHandlerNow,
  refreshRuns,
  runsLoading,
}: LifecycleRunsPanelProps) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const ruleNames = new Map(
    ruleSetRecords.map((record) => [record.id, record.name]),
  );
  const everyRule = runScopeRuleSetId === LIFECYCLE_FILTER_ALL;

  return (
    <div id="settings-lifecycle-runs" className="space-y-8">
      <div className="space-y-2">
        <div className={TOOLBAR_SHELL}>
          <SingleSelectField
            id="settings-lifecycle-run-scope"
            label={t("settings.lifecycleRunScope")}
            labelClassName={TOOLBAR_SELECT_LABEL}
            size="compact"
            chrome="toolbar"
            className="md:w-[260px]"
            value={runScopeRuleSetId}
            onValueChange={setRunScopeRuleSetId}
            options={[
              {
                value: LIFECYCLE_FILTER_ALL,
                label: t("settings.lifecycleRunScopeAll"),
              },
              ...ruleSetRecords.map((record) => ({
                value: record.id,
                label: record.name,
              })),
            ]}
          />
          <div className="flex flex-wrap items-center gap-2 md:ml-auto">
            <RefreshButton
              id="settings-lifecycle-runs-refresh"
              loading={runsLoading}
              onClick={refreshRuns}
            />
            <Button
              id="settings-lifecycle-run-evaluation-now"
              type="button"
              variant="secondary"
              size="sm"
              disabled={evaluationTriggering}
              onClick={runEvaluationNow}
            >
              {evaluationTriggering ? (
                <LoadingMark className="h-4 w-4" />
              ) : (
                <Play className="h-4 w-4" />
              )}
              {evaluationTriggering
                ? t("settings.lifecycleRunEvaluationRunning")
                : t("settings.lifecycleRunEvaluationNow")}
            </Button>
            {/* Rules act only through their own fires, so acting now is
                always for the one rule chosen beside it. */}
            <span className="inline-flex items-center gap-1">
              <Button
                id="settings-lifecycle-run-actions-now"
                type="button"
                variant="secondary"
                size="sm"
                disabled={actionTriggering || everyRule}
                aria-describedby={
                  everyRule
                    ? "settings-lifecycle-run-actions-needs-rule"
                    : undefined
                }
                onClick={runActionHandlerNow}
              >
                {actionTriggering ? (
                  <LoadingMark className="h-4 w-4" />
                ) : (
                  <Zap className="h-4 w-4" />
                )}
                {actionTriggering
                  ? t("settings.lifecycleRunActionsRunning")
                  : t("settings.lifecycleRunActionsNow")}
              </Button>
              <InfoHelp
                text={t("settings.lifecycleRunActionsHeldHelp")}
                ariaLabel={t("settings.lifecycleRunActionsNow")}
              />
            </span>
          </div>
        </div>
        {everyRule ? (
          <p
            id="settings-lifecycle-run-actions-needs-rule"
            className="px-1 text-xs text-[var(--scry-muted)]"
          >
            {t("settings.lifecycleRunActionsNeedsRule")}
          </p>
        ) : null}
        <PaneError message={runsError} />
      </div>

      <PaneSection
        id="settings-lifecycle-evaluation-runs-section"
        title={t("settings.lifecycleEvaluationRunsTitle")}
        count={
          runsLoading && evaluationRuns.length === 0
            ? undefined
            : evaluationRuns.length
        }
      >
        {evaluationRuns.length === 0 ? (
          <PaneEmpty
            id="settings-lifecycle-evaluation-runs-empty"
            loading={runsLoading}
          >
            {t("settings.lifecycleEvaluationRunsEmpty")}
          </PaneEmpty>
        ) : (
          <Table
            id="settings-lifecycle-evaluation-runs"
            wrapperClassName={TABLE_SHELL}
          >
            <TableHeader>
              <TableRow>
                <TableHead className="whitespace-nowrap">
                  {t("settings.lifecycleRunColStarted")}
                </TableHead>
                <TableHead>
                  {t("settings.lifecycleCandidatesColRule")}
                </TableHead>
                <TableHead>{t("settings.lifecycleRunColStatus")}</TableHead>
                <TableHead>{t("settings.lifecycleRunColCounts")}</TableHead>
                <TableHead className="whitespace-nowrap text-right">
                  {t("settings.lifecycleRunColDuration")}
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {evaluationRuns.map((run) => (
                <TableRow
                  key={run.id}
                  data-ui="settings-table-row"
                  id={selectorId(
                    "settings-lifecycle-evaluation-run-row",
                    run.id,
                  )}
                >
                  <TableCell className="whitespace-nowrap text-[var(--scry-muted)]">
                    {timestamp(run.startedAt, dateTimeFormat)}
                  </TableCell>
                  <TableCell>
                    <span className="inline-flex flex-wrap items-center gap-1.5">
                      <span className="text-[13px] font-medium text-[var(--scry-ink2)]">
                        {ruleNames.get(run.ruleSetId) ?? run.ruleSetId}
                      </span>
                      <Badge tone="outline">
                        {t("settings.lifecycleRunRevision", {
                          revision: run.revisionNumber,
                        })}
                      </Badge>
                    </span>
                  </TableCell>
                  <TableCell>
                    <RunOutcome status={run.status} error={run.error} />
                  </TableCell>
                  <TableCell className="text-xs text-[var(--scry-muted)]">
                    {runCounts(run, t)}
                  </TableCell>
                  <TableCell className="whitespace-nowrap text-right text-[var(--scry-muted)]">
                    {run.durationMs === null
                      ? "—"
                      : t("settings.lifecycleRunDuration", {
                          seconds: (run.durationMs / 1000).toFixed(1),
                        })}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        )}
      </PaneSection>

      <PaneSection
        id="settings-lifecycle-action-runs-section"
        title={t("settings.lifecycleActionRunsTitle")}
        count={
          runsLoading && actionRuns.length === 0 ? undefined : actionRuns.length
        }
      >
        {actionRuns.length === 0 ? (
          <PaneEmpty
            id="settings-lifecycle-action-runs-empty"
            loading={runsLoading}
          >
            {t("settings.lifecycleActionRunsEmpty")}
          </PaneEmpty>
        ) : (
          <Table
            id="settings-lifecycle-action-runs"
            wrapperClassName={TABLE_SHELL}
          >
            <TableHeader>
              <TableRow>
                <TableHead className="whitespace-nowrap">
                  {t("settings.lifecycleRunColStarted")}
                </TableHead>
                <TableHead>{t("label.title")}</TableHead>
                <TableHead>{t("settings.lifecycleRuleAction")}</TableHead>
                <TableHead>{t("settings.lifecycleRunColStatus")}</TableHead>
                <TableHead className="w-[1%] whitespace-nowrap text-right">
                  {t("settings.lifecycleActionRunColAttempt")}
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {actionRuns.map((run) => (
                <TableRow
                  key={run.id}
                  data-ui="settings-table-row"
                  id={selectorId("settings-lifecycle-action-run-row", run.id)}
                  className="[&>td]:align-top"
                >
                  <TableCell className="whitespace-nowrap text-[var(--scry-muted)]">
                    {timestamp(run.startedAt, dateTimeFormat)}
                  </TableCell>
                  <TableCell>
                    <SubjectCell
                      titleName={run.titleName}
                      subjectKind={run.subjectKind}
                      subjectLabel={run.subjectLabel}
                    />
                  </TableCell>
                  <TableCell>
                    <span className="text-[13px] text-[var(--scry-ink2)]">
                      <ActionKindLabel kind={run.actionKind} />
                    </span>
                    <LifecycleSequenceProgress
                      sequence={run.actionSequence}
                      progress={run.sequenceSteps}
                    />
                  </TableCell>
                  <TableCell>
                    <RunOutcome
                      status={run.status}
                      holdReason={run.holdReason}
                      causeCode={run.causeCode}
                      error={run.error}
                    >
                      <LifecycleFileResults detail={run.detail} />
                    </RunOutcome>
                  </TableCell>
                  <TableCell className="text-right tabular-nums text-[var(--scry-muted)]">
                    {run.attempt}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        )}
      </PaneSection>
    </div>
  );
}

// ── Exclusions ────────────────────────────────────────────────────────

export type LifecycleExclusionsPanelProps = {
  exclusions: LifecycleExclusion[];
  exclusionsError: string | null;
  ruleSetRecords: LifecycleRuleSetRecord[];
  removingExclusionId: string | null;
  removeExclusion: (exclusion: LifecycleExclusion) => void;
};

export function LifecycleExclusionsPanel({
  exclusions,
  exclusionsError,
  ruleSetRecords,
  removingExclusionId,
  removeExclusion,
}: LifecycleExclusionsPanelProps) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const ruleNames = new Map(
    ruleSetRecords.map((record) => [record.id, record.name]),
  );

  return (
    <PaneSection
      id="settings-lifecycle-exclusions-section"
      title={t("settings.lifecycleExclusionsTitle")}
      help={t("settings.lifecycleExclusionsSubtitle")}
      count={exclusions.length}
    >
      <PaneError message={exclusionsError} />
      {exclusions.length === 0 ? (
        <PaneEmpty id="settings-lifecycle-exclusions-empty" loading={false}>
          {t("settings.lifecycleExclusionsEmpty")}
        </PaneEmpty>
      ) : (
        <Table
          id="settings-lifecycle-exclusions"
          wrapperClassName={TABLE_SHELL}
        >
          <TableHeader>
            <TableRow>
              <TableHead>{t("label.title")}</TableHead>
              <TableHead>{t("settings.lifecycleExclusionColScope")}</TableHead>
              <TableHead className="hidden md:table-cell">
                {t("settings.lifecycleExclusionColReason")}
              </TableHead>
              <TableHead className="hidden whitespace-nowrap md:table-cell">
                {t("settings.lifecycleExclusionColCreated")}
              </TableHead>
              <TableHead className="w-[1%] text-right">
                {t("label.actions")}
              </TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {exclusions.map((exclusion) => (
              <TableRow
                key={exclusion.id}
                data-ui="settings-table-row"
                id={selectorId(
                  "settings-lifecycle-exclusion-row",
                  exclusion.id,
                )}
              >
                <TableCell>
                  <SubjectCell
                    titleName={exclusion.titleName}
                    subjectKind={exclusion.subjectKind}
                    subjectLabel={exclusion.subjectLabel}
                  />
                </TableCell>
                <TableCell>
                  {exclusion.ruleSetId ? (
                    <Badge tone="outline">
                      {ruleNames.get(exclusion.ruleSetId) ??
                        exclusion.ruleSetId}
                    </Badge>
                  ) : (
                    <Badge tone="neutral">
                      {t("settings.lifecycleExclusionScopeGlobal")}
                    </Badge>
                  )}
                </TableCell>
                <TableCell
                  className="hidden max-w-[320px] truncate text-[var(--scry-muted)] md:table-cell"
                  title={exclusion.reason || undefined}
                >
                  {exclusion.reason || "—"}
                </TableCell>
                <TableCell className="hidden whitespace-nowrap text-[var(--scry-muted)] md:table-cell">
                  {timestamp(exclusion.createdAt, dateTimeFormat)}
                </TableCell>
                <TableCell className="text-right">
                  <IconButton
                    id={selectorId(
                      "settings-lifecycle-exclusion-remove",
                      exclusion.id,
                    )}
                    label={t("settings.lifecycleExclusionRemove")}
                    tone="delete"
                    disabled={removingExclusionId === exclusion.id}
                    onClick={() => removeExclusion(exclusion)}
                  >
                    <Trash2 className="h-4 w-4" />
                  </IconButton>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}
    </PaneSection>
  );
}

function LifecycleFileResults({ detail }: { detail: string }) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const results = parseLifecycleFileResults(detail);
  if (!results) return null;
  return (
    <details className="group whitespace-normal">
      <summary className="cursor-pointer text-xs font-medium text-[var(--scry-accent-text)] hover:underline">
        {t("settings.lifecycleFileResults")} ({results.files.length})
      </summary>
      <div className="mt-2 space-y-1.5 rounded-[10px] border border-[var(--scry-border3)] bg-[var(--scry-surfC)] p-2.5">
        <p className="text-xs text-[var(--scry-muted)]">
          {results.totalSizeBytes != null
            ? `${t("settings.lifecycleFileSummary", {
                count: results.files.length,
                size: formatBytes(results.totalSizeBytes),
              })} · `
            : null}
          {t("settings.lifecycleFileProgress", {
            completed: results.completedFileCount,
            remaining: results.remainingFileCount,
          })}
        </p>
        {results.graceDeadline ? (
          <p className="text-xs text-[var(--scry-muted)]">
            {t("settings.lifecyclePreviewDue", {
              time: formatUiDateTime(results.graceDeadline, dateTimeFormat),
            })}
          </p>
        ) : null}
        <ul className="space-y-1">
          {results.files.map((file) => (
            <li
              key={file.fileId}
              className="flex flex-col gap-0.5 border-t border-[var(--scry-border3)] pt-1 text-xs first:border-t-0 first:pt-0"
            >
              <span
                data-code-font
                className="break-all text-[var(--scry-ink2)]"
              >
                {file.filePath ?? file.fileId}
              </span>
              <span
                className={
                  file.error
                    ? "text-[var(--scry-danger-text)]"
                    : "text-[var(--scry-muted)]"
                }
              >
                {file.error ??
                  (file.completed
                    ? t("settings.lifecycleFileDeleted")
                    : t("settings.lifecycleFilePending"))}
              </span>
            </li>
          ))}
        </ul>
      </div>
    </details>
  );
}

import { formatBytes } from "@/lib/utils/activity-utils";
import { lifecycleExecutionReasonText } from "@/lib/utils/lifecycle-execution-reason";
import { Ban, Play, RefreshCw, Trash2 } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { IconButton } from "@/components/ui/icon-button";
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
import { HelpLabel } from "@/components/common/info-help";
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

/// Candidates and runs recorded before rules declared their actions in source
/// name their action by kind and sequence; these render that history.
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
    <div
      id="settings-lifecycle-switches"
      className="grid gap-3 md:grid-cols-2"
    >
      {INSTANCE_SWITCHES.map(({ key, label, help }) => (
        <div
          key={key}
          className="flex items-center justify-between gap-3 rounded border border-border bg-muted/30 px-3 py-2.5"
        >
          <div className="space-y-1">
            <Label htmlFor={selectorId("settings-lifecycle-switch", key)}>
              <HelpLabel label={t(label)} help={t(help)} />
            </Label>
          </div>
          <div className="flex shrink-0 items-center gap-2">
            {savingSwitch === key ? (
              <LoadingMark className="h-4 w-4 text-muted-foreground" />
            ) : null}
            <SettingsToggleSwitch
              id={selectorId("settings-lifecycle-switch", key)}
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
          ? "text-[var(--scry-warning-text)]"
          : "text-muted-foreground"
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
  const dateTimeFormat = useUiDateTimeFormat();

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">
          <HelpLabel
            label={t("settings.lifecycleCandidatesTitle")}
            help={t("settings.lifecycleCandidatesSubtitle")}
          />
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="grid gap-3 md:grid-cols-4">
          <SingleSelectField
            id="settings-lifecycle-candidates-rule"
            label={t("settings.lifecycleCandidatesFilterRule")}
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
          <div className="flex items-end">
            <Button
              id="settings-lifecycle-candidates-refresh"
              type="button"
              variant="secondary"
              disabled={candidatesLoading}
              onClick={refreshCandidates}
            >
              {candidatesLoading ? (
                <LoadingMark className="mr-2 h-4 w-4" />
              ) : (
                <RefreshCw className="mr-2 h-4 w-4" />
              )}
              {t("label.refresh")}
            </Button>
          </div>
        </div>

        {candidatesError ? (
          <pre className="overflow-x-auto whitespace-pre-wrap rounded-[9px] border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] p-3 font-mono text-[12px] leading-5 text-[var(--scry-danger-text)]">
            {candidatesError}
          </pre>
        ) : null}

        <div className="overflow-x-auto">
          <Table id="settings-lifecycle-candidates-table">
            <TableHeader>
              <TableRow>
                <TableHead>{t("label.title")}</TableHead>
                <TableHead>
                  {t("settings.lifecycleCandidatesColRule")}
                </TableHead>
                <TableHead className="w-[150px]">
                  {t("settings.lifecycleCandidatesColState")}
                </TableHead>
                <TableHead>
                  {t("settings.lifecyclePreviewColReasons")}
                </TableHead>
                <TableHead className="whitespace-nowrap">
                  {t("settings.lifecycleCandidatesColFirstMatched")}
                </TableHead>
                <TableHead className="whitespace-nowrap">
                  {t("settings.lifecycleCandidatesColDue")}
                </TableHead>
                <TableHead className="text-right">
                  {t("label.actions")}
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {candidates.map((candidate) => {
                const stateKey = candidateStateLabelKey(candidate.state);
                return (
                  <TableRow
                    key={candidate.id}
                    data-ui="settings-table-row"
                    id={selectorId(
                      "settings-lifecycle-candidate-row",
                      candidate.id,
                    )}
                  >
                    <TableCell className="font-medium">
                      <div className="flex flex-col gap-1">
                        <span>{candidate.titleName}</span>
                        {candidate.subjectKind !== "title" && (
                          <span className="text-sm text-muted-foreground">
                            {candidate.subjectLabel}
                          </span>
                        )}
                        {candidate.fileCount != null && (
                          <span className="text-xs text-muted-foreground">
                            {t("settings.lifecycleFileSummary", {
                              count: candidate.fileCount,
                              size: formatBytes(candidate.totalSizeBytes),
                            })}
                          </span>
                        )}
                        {candidate.storageRootFileCount !== null &&
                        candidate.storageRootTotalSizeBytes !== null ? (
                          <span className="text-xs text-muted-foreground">
                            {t("settings.lifecycleStorageRootFileSummary", {
                              count: candidate.storageRootFileCount,
                              size: formatBytes(
                                candidate.storageRootTotalSizeBytes,
                              ),
                            })}
                          </span>
                        ) : null}
                      </div>
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      <div className="flex flex-col">
                        <span>{candidate.ruleName}</span>
                        {candidate.actions ? (
                          <div className="mt-0.5">
                            <LifecycleDecidedActionsSummary
                              actions={candidate.actions}
                              idPrefix={selectorId(
                                "settings-lifecycle-candidate-action",
                                candidate.id,
                              )}
                            />
                          </div>
                        ) : null}
                        {candidate.actionSequence?.steps.length ? (
                          <LifecycleSequenceProgress
                            sequence={candidate.actionSequence}
                            progress={candidate.sequenceSteps}
                          />
                        ) : null}
                      </div>
                    </TableCell>
                    <TableCell>
                      <div className="flex flex-col gap-1">
                        <Badge
                          tone={candidateStateBadgeTone(candidate.state)}
                          title={
                            candidate.stateReason
                              ? lifecycleExecutionReasonText(
                                  t,
                                  candidate.stateReason,
                                )
                              : undefined
                          }
                        >
                          {stateKey ? t(stateKey) : candidate.state}
                        </Badge>
                        {candidate.heldSince ? (
                          <span
                            className="text-xs text-[var(--scry-warning-text)]"
                            title={
                              candidate.stateReason
                                ? lifecycleExecutionReasonText(
                                    t,
                                    candidate.stateReason,
                                  )
                                : undefined
                            }
                          >
                            {t("settings.lifecycleCandidatesHeldSince", {
                              time: timestamp(
                                candidate.heldSince,
                                dateTimeFormat,
                              ),
                            })}
                          </span>
                        ) : null}
                      </div>
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      <ReasonCodes codes={candidate.reasonCodes} />
                      {candidate.stateReason ? (
                        <p className="mt-1 max-w-[280px] truncate text-xs">
                          {lifecycleExecutionReasonText(
                            t,
                            candidate.stateReason,
                          )}
                        </p>
                      ) : null}
                    </TableCell>
                    <TableCell className="whitespace-nowrap text-muted-foreground">
                      {timestamp(candidate.firstMatchedAt, dateTimeFormat)}
                    </TableCell>
                    <TableCell
                      className="whitespace-nowrap"
                      title={timestamp(candidate.dueAt, dateTimeFormat)}
                    >
                      <CandidateDueCell dueAt={candidate.dueAt} />
                    </TableCell>
                    <TableCell className="text-right">
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
                );
              })}
              {candidates.length === 0 && !candidatesLoading ? (
                <TableRow>
                  <TableCell colSpan={7} className="text-muted-foreground">
                    {t(candidatesEmptyKey(gates, ruleSetRecords))}
                  </TableCell>
                </TableRow>
              ) : null}
            </TableBody>
          </Table>
        </div>
      </CardContent>
    </Card>
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

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">
          {t("settings.lifecycleRunsTitle")}
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="grid gap-3 md:grid-cols-4">
          <SingleSelectField
            id="settings-lifecycle-run-scope"
            label={t("settings.lifecycleRunScope")}
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
          <div className="flex items-end">
            <Button
              id="settings-lifecycle-run-evaluation-now"
              type="button"
              variant="secondary"
              disabled={evaluationTriggering}
              onClick={runEvaluationNow}
            >
              <Play className="mr-2 h-4 w-4" />
              {evaluationTriggering
                ? t("settings.lifecycleRunEvaluationRunning")
                : t("settings.lifecycleRunEvaluationNow")}
            </Button>
          </div>
          <div className="flex flex-col justify-end gap-1">
            {/* Rules act only through their own fires, so acting now is
                always for the one rule chosen above. */}
            <Button
              id="settings-lifecycle-run-actions-now"
              type="button"
              variant="secondary"
              disabled={
                actionTriggering || runScopeRuleSetId === LIFECYCLE_FILTER_ALL
              }
              aria-describedby={
                runScopeRuleSetId === LIFECYCLE_FILTER_ALL
                  ? "settings-lifecycle-run-actions-needs-rule"
                  : undefined
              }
              onClick={runActionHandlerNow}
            >
              <Play className="mr-2 h-4 w-4" />
              {actionTriggering
                ? t("settings.lifecycleRunActionsRunning")
                : t("settings.lifecycleRunActionsNow")}
            </Button>
            {runScopeRuleSetId === LIFECYCLE_FILTER_ALL ? (
              <p
                id="settings-lifecycle-run-actions-needs-rule"
                className="text-xs text-muted-foreground"
              >
                {t("settings.lifecycleRunActionsNeedsRule")}
              </p>
            ) : null}
            <p
              id={selectorId("settings-lifecycle-run-actions-held-help")}
              className="max-w-xs text-xs text-muted-foreground"
            >
              {t("settings.lifecycleRunActionsHeldHelp")}
            </p>
          </div>
          <div className="flex items-end">
            <Button
              id="settings-lifecycle-runs-refresh"
              type="button"
              variant="secondary"
              disabled={runsLoading}
              onClick={refreshRuns}
            >
              {runsLoading ? (
                <LoadingMark className="mr-2 h-4 w-4" />
              ) : (
                <RefreshCw className="mr-2 h-4 w-4" />
              )}
              {t("label.refresh")}
            </Button>
          </div>
        </div>

        {runsError ? (
          <pre className="overflow-x-auto whitespace-pre-wrap rounded-[9px] border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] p-3 font-mono text-[12px] leading-5 text-[var(--scry-danger-text)]">
            {runsError}
          </pre>
        ) : null}

        <div className="space-y-2">
          <h4 className="text-sm font-semibold">
            {t("settings.lifecycleEvaluationRunsTitle")}
          </h4>
          <div className="overflow-x-auto">
            <Table id="settings-lifecycle-evaluation-runs">
              <TableHeader>
                <TableRow>
                  <TableHead className="whitespace-nowrap">
                    {t("settings.lifecycleRunColStarted")}
                  </TableHead>
                  <TableHead>
                    {t("settings.lifecycleCandidatesColRule")}
                  </TableHead>
                  <TableHead className="w-[130px]">
                    {t("settings.lifecycleRunColStatus")}
                  </TableHead>
                  <TableHead>{t("settings.lifecycleRunColCounts")}</TableHead>
                  <TableHead className="whitespace-nowrap">
                    {t("settings.lifecycleRunColDuration")}
                  </TableHead>
                  <TableHead>{t("settings.lifecycleRunColError")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {evaluationRuns.map((run) => {
                  const rule = ruleSetRecords.find(
                    (record) => record.id === run.ruleSetId,
                  );
                  return (
                    <TableRow
                      key={run.id}
                      data-ui="settings-table-row"
                      id={selectorId(
                        "settings-lifecycle-evaluation-run-row",
                        run.id,
                      )}
                    >
                      <TableCell className="whitespace-nowrap text-muted-foreground">
                        {timestamp(run.startedAt, dateTimeFormat)}
                      </TableCell>
                      <TableCell>
                        {rule?.name ?? run.ruleSetId}
                        <span className="ml-1 text-xs text-muted-foreground">
                          {t("settings.lifecycleRunRevision", {
                            revision: run.revisionNumber,
                          })}
                        </span>
                      </TableCell>
                      <TableCell>
                        <RunStatusBadge status={run.status} />
                      </TableCell>
                      <TableCell className="text-muted-foreground">
                        {runCounts(run, t)}
                      </TableCell>
                      <TableCell className="whitespace-nowrap text-muted-foreground">
                        {run.durationMs === null
                          ? "—"
                          : t("settings.lifecycleRunDuration", {
                              seconds: (run.durationMs / 1000).toFixed(1),
                            })}
                      </TableCell>
                      <TableCell className="max-w-[240px] truncate text-[var(--scry-danger-text)]">
                        {run.error ?? "—"}
                      </TableCell>
                    </TableRow>
                  );
                })}
                {evaluationRuns.length === 0 ? (
                  <TableRow>
                    <TableCell colSpan={6} className="text-muted-foreground">
                      {t("settings.lifecycleEvaluationRunsEmpty")}
                    </TableCell>
                  </TableRow>
                ) : null}
              </TableBody>
            </Table>
          </div>
        </div>

        <div className="space-y-2">
          <h4 className="text-sm font-semibold">
            {t("settings.lifecycleActionRunsTitle")}
          </h4>
          <div className="overflow-x-auto">
            <Table id="settings-lifecycle-action-runs">
              <TableHeader>
                <TableRow>
                  <TableHead className="whitespace-nowrap">
                    {t("settings.lifecycleRunColStarted")}
                  </TableHead>
                  <TableHead>{t("label.title")}</TableHead>
                  <TableHead>{t("settings.lifecycleRuleAction")}</TableHead>
                  <TableHead className="w-[150px]">
                    {t("settings.lifecycleRunColStatus")}
                  </TableHead>
                  <TableHead className="text-center">
                    {t("settings.lifecycleActionRunColAttempt")}
                  </TableHead>
                  <TableHead>
                    {t("settings.lifecycleActionRunColHoldReason")}
                  </TableHead>
                  <TableHead>{t("settings.lifecycleRunColError")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {actionRuns.map((run) => (
                  <TableRow
                    key={run.id}
                    data-ui="settings-table-row"
                    id={selectorId(
                      "settings-lifecycle-action-run-row",
                      run.id,
                    )}
                  >
                    <TableCell className="whitespace-nowrap text-muted-foreground">
                      {timestamp(run.startedAt, dateTimeFormat)}
                    </TableCell>
                    <TableCell className="font-medium">
                      {run.titleName}
                      {run.subjectKind !== "title" && (
                        <div className="text-sm text-muted-foreground">
                          {run.subjectLabel}
                        </div>
                      )}
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      <ActionKindLabel kind={run.actionKind} />
                      <LifecycleSequenceProgress
                        sequence={run.actionSequence}
                        progress={run.sequenceSteps}
                      />
                    </TableCell>
                    <TableCell>
                      <RunStatusBadge status={run.status} />
                    </TableCell>
                    <TableCell className="text-center text-muted-foreground">
                      {run.attempt}
                    </TableCell>
                    <TableCell className="max-w-[220px] truncate text-[var(--scry-warning-text)]">
                      {run.holdReason
                        ? lifecycleExecutionReasonText(t, run.holdReason)
                        : "—"}
                    </TableCell>
                    <TableCell className="min-w-[220px] whitespace-normal text-[var(--scry-danger-text)]">
                      {run.error ?? "—"}
                      <LifecycleFileResults detail={run.detail} />
                    </TableCell>
                  </TableRow>
                ))}
                {actionRuns.length === 0 ? (
                  <TableRow>
                    <TableCell colSpan={7} className="text-muted-foreground">
                      {t("settings.lifecycleActionRunsEmpty")}
                    </TableCell>
                  </TableRow>
                ) : null}
              </TableBody>
            </Table>
          </div>
        </div>
      </CardContent>
    </Card>
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

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">
          <HelpLabel
            label={t("settings.lifecycleExclusionsTitle")}
            help={t("settings.lifecycleExclusionsSubtitle")}
          />
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-3">
        {exclusionsError ? (
          <pre className="overflow-x-auto whitespace-pre-wrap rounded-[9px] border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] p-3 font-mono text-[12px] leading-5 text-[var(--scry-danger-text)]">
            {exclusionsError}
          </pre>
        ) : null}
        <div className="overflow-x-auto">
          <Table id="settings-lifecycle-exclusions">
            <TableHeader>
              <TableRow>
                <TableHead>{t("label.title")}</TableHead>
                <TableHead>
                  {t("settings.lifecycleExclusionColScope")}
                </TableHead>
                <TableHead>
                  {t("settings.lifecycleExclusionColReason")}
                </TableHead>
                <TableHead className="whitespace-nowrap">
                  {t("settings.lifecycleExclusionColCreated")}
                </TableHead>
                <TableHead className="text-right">
                  {t("label.actions")}
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {exclusions.map((exclusion) => {
                const rule = exclusion.ruleSetId
                  ? ruleSetRecords.find(
                      (record) => record.id === exclusion.ruleSetId,
                    )
                  : null;
                return (
                  <TableRow
                    key={exclusion.id}
                    data-ui="settings-table-row"
                    id={selectorId(
                      "settings-lifecycle-exclusion-row",
                      exclusion.id,
                    )}
                  >
                    <TableCell className="font-medium">
                      {exclusion.titleName}
                      {exclusion.subjectKind !== "title" && (
                        <div className="text-sm text-muted-foreground">
                          {exclusion.subjectLabel}
                        </div>
                      )}
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {exclusion.ruleSetId
                        ? (rule?.name ?? exclusion.ruleSetId)
                        : t("settings.lifecycleExclusionScopeGlobal")}
                    </TableCell>
                    <TableCell className="max-w-[280px] truncate text-muted-foreground">
                      {exclusion.reason || "—"}
                    </TableCell>
                    <TableCell className="whitespace-nowrap text-muted-foreground">
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
                );
              })}
              {exclusions.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={5} className="text-muted-foreground">
                    {t("settings.lifecycleExclusionsEmpty")}
                  </TableCell>
                </TableRow>
              ) : null}
            </TableBody>
          </Table>
        </div>
      </CardContent>
    </Card>
  );
}

function LifecycleFileResults({ detail }: { detail: string }) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const results = parseLifecycleFileResults(detail);
  if (!results) return null;
  return (
    <details className="whitespace-normal text-foreground">
      <summary className="cursor-pointer text-sm">
        {t("settings.lifecycleFileResults")} ({results.files.length})
      </summary>
      {results.storageRootId ? (
        <p className="text-xs text-muted-foreground">
          {t("settings.lifecycleRuleStorageRoot")}:{" "}
          <code>{results.storageRootId}</code>
        </p>
      ) : null}
      {results.totalSizeBytes != null ? (
        <p className="text-xs text-muted-foreground">
          {t("settings.lifecycleFileSummary", {
            count: results.files.length,
            size: formatBytes(results.totalSizeBytes),
          })}
        </p>
      ) : null}
      <p className="text-xs text-muted-foreground">
        {t("settings.lifecycleFileProgress", {
          completed: results.completedFileCount,
          remaining: results.remainingFileCount,
        })}
      </p>
      {results.graceDeadline ? (
        <p className="text-xs text-muted-foreground">
          {t("settings.lifecyclePreviewDue", {
            time: formatUiDateTime(results.graceDeadline, dateTimeFormat),
          })}
        </p>
      ) : null}
      <Table>
        <TableBody>
          {results.files.map((file) => (
            <TableRow key={file.fileId} data-ui="settings-table-row">
              <TableCell className="break-all text-xs">
                {file.filePath ?? file.fileId}
              </TableCell>
              <TableCell className="text-xs">
                {file.error ??
                  (file.completed
                    ? t("settings.lifecycleFileDeleted")
                    : t("settings.lifecycleFilePending"))}
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </details>
  );
}

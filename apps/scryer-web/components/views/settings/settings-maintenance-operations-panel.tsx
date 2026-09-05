import * as React from "react";
import { Ban, Loader2, Play, RefreshCw, Trash2 } from "lucide-react";
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
  MaintenanceActionRun,
  MaintenanceCandidate,
  MaintenanceCandidateState,
  MaintenanceEvaluationRun,
  MaintenanceExclusion,
  MaintenanceGateKey,
  MaintenanceInstanceGates,
  MaintenanceRuleSetRecord,
} from "@/lib/types/maintenance-rule-sets";
import {
  MAINTENANCE_FILTER_ALL,
  MAINTENANCE_GATE_ORDER,
  actionKindLabelKey,
  candidateStateBadgeTone,
  candidateStateLabelKey,
  gateHelpKey,
  gateLabelKey,
  maintenanceCountdown,
  runStatusBadgeTone,
  runStatusLabelKey,
} from "@/lib/utils/maintenance-rule-sets";
import { formatUiDateTime } from "@/lib/utils/date-format";
import { selectorId } from "@/lib/utils/dom-ids";

export type MaintenanceLibraryOption = { id: string; name: string };

/// Every candidate lifecycle state, in the order the filter offers them. Kept
/// here rather than derived from the loaded rows so the filter does not shrink
/// to whatever happens to be on screen.
const CANDIDATE_STATE_FILTER_OPTIONS: MaintenanceCandidateState[] = [
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

function ActionKindLabel({ kind }: { kind: string }) {
  const t = useTranslate();
  const labelKey = actionKindLabelKey(kind);
  return <>{labelKey ? t(labelKey) : kind}</>;
}

/// The labels a tag action would write on the candidate's subject. Rendered
/// beside the action name because "add tags" on its own does not say which
/// tags, and that is the whole of what the action does.
function TagPatchSummary({
  patch,
}: {
  patch: { kind: string; tags: string[] } | undefined;
}) {
  const t = useTranslate();
  if (!patch || patch.tags.length === 0) {
    return null;
  }
  const key =
    patch.kind === "REMOVE_TAGS"
      ? "settings.maintenanceCandidatesTagPatchRemove"
      : "settings.maintenanceCandidatesTagPatchAdd";
  return (
    <span className="text-xs text-muted-foreground">
      {t(key, { tags: patch.tags.join(", ") })}
    </span>
  );
}

/// Run statuses are free-form strings on the API. An unrecognized one renders
/// as itself instead of vanishing into an empty cell.
function RunStatusBadge({ status }: { status: string }) {
  const t = useTranslate();
  const labelKey = runStatusLabelKey(status);
  return <Badge tone={runStatusBadgeTone(status)}>{labelKey ? t(labelKey) : status}</Badge>;
}

function ReasonCodes({ codes }: { codes: string[] }) {
  if (codes.length === 0) return <>—</>;
  return (
    <div className="flex flex-wrap gap-1">
      {codes.map((code) => (
        <code key={code} data-code-font className="rounded bg-muted px-1 py-0.5 text-xs">
          {code}
        </code>
      ))}
    </div>
  );
}

// ── Gates ─────────────────────────────────────────────────────────────

export type MaintenanceGatesPanelProps = {
  gates: MaintenanceInstanceGates | null;
  /// True when the gates query was refused, which on this page means the reader
  /// is not a system administrator.
  gatesLocked: boolean;
  savingGate: MaintenanceGateKey | null;
  onGateChange: (gate: MaintenanceGateKey, enabled: boolean) => void;
};

export function MaintenanceGatesPanel({
  gates,
  gatesLocked,
  savingGate,
  onGateChange,
}: MaintenanceGatesPanelProps) {
  const t = useTranslate();

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">{t("settings.maintenanceGatesTitle")}</CardTitle>
        <p className="text-xs text-muted-foreground">
          {t("settings.maintenanceGatesSubtitle")}
        </p>
      </CardHeader>
      <CardContent>
        {gatesLocked || !gates ? (
          <p
            id="settings-maintenance-gates-locked"
            className="rounded border border-[var(--scry-info-border)] bg-[var(--scry-info-bg)] px-3 py-2 text-xs text-[var(--scry-info-text)]"
          >
            {t("settings.maintenanceGatesLocked")}
          </p>
        ) : (
          <div id="settings-maintenance-gates" className="space-y-3">
            {MAINTENANCE_GATE_ORDER.map((gate) => {
              const destructive = gate === "destructiveEffectsEnabled";
              return (
                <div
                  key={gate}
                  className={`flex flex-col gap-2 rounded border px-3 py-2.5 sm:flex-row sm:items-center sm:justify-between ${
                    destructive
                      ? "border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)]"
                      : "border-border bg-muted/30"
                  }`}
                >
                  <div className="space-y-1">
                    <Label
                      htmlFor={selectorId("settings-maintenance-gate", gate)}
                      className={
                        destructive ? "text-[var(--scry-danger-text)]" : undefined
                      }
                    >
                      {t(gateLabelKey(gate))}
                    </Label>
                    <p
                      className={`text-xs ${
                        destructive
                          ? "text-[var(--scry-danger-text)]"
                          : "text-muted-foreground"
                      }`}
                    >
                      {t(gateHelpKey(gate))}
                    </p>
                  </div>
                  <div className="flex shrink-0 items-center gap-2">
                    {savingGate === gate ? (
                      <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
                    ) : null}
                    <SettingsToggleSwitch
                      id={selectorId("settings-maintenance-gate", gate)}
                      checked={gates[gate]}
                      disabled={savingGate !== null}
                      ariaLabel={t(gateLabelKey(gate))}
                      onChange={(next) => onGateChange(gate, next)}
                    />
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </CardContent>
    </Card>
  );
}

// ── Candidates ────────────────────────────────────────────────────────

function CandidateDueCell({ dueAt }: { dueAt: string }) {
  const t = useTranslate();
  const countdown = maintenanceCountdown(dueAt);
  if (!countdown) {
    return <span className="text-muted-foreground">{dueAt}</span>;
  }
  return (
    <span
      className={
        countdown.overdue ? "text-[var(--scry-warning-text)]" : "text-muted-foreground"
      }
    >
      {t(countdown.labelKey, countdown.values)}
    </span>
  );
}

export type MaintenanceCandidatesPanelProps = {
  candidates: MaintenanceCandidate[];
  candidatesLoading: boolean;
  candidatesError: string | null;
  ruleSetRecords: MaintenanceRuleSetRecord[];
  libraries: MaintenanceLibraryOption[];
  /// Gates are null for a reader who cannot see them; the empty state then
  /// stops guessing why the list is empty.
  gates: MaintenanceInstanceGates | null;
  candidateRuleFilter: string;
  setCandidateRuleFilter: (id: string) => void;
  candidateStateFilter: string;
  setCandidateStateFilter: (state: string) => void;
  candidateLibraryFilter: string;
  setCandidateLibraryFilter: (id: string) => void;
  refreshCandidates: () => void;
  excludeCandidate: (candidate: MaintenanceCandidate) => void;
};

function candidatesEmptyKey(
  gates: MaintenanceInstanceGates | null,
  ruleSetRecords: MaintenanceRuleSetRecord[],
): string {
  if (gates && !gates.evaluationEnabled) {
    return "settings.maintenanceCandidatesEmptyEvaluationOff";
  }
  if (!ruleSetRecords.some((record) => record.evaluationMode !== "DISABLED")) {
    return "settings.maintenanceCandidatesEmptyNoActiveRules";
  }
  return "settings.maintenanceCandidatesEmpty";
}

export function MaintenanceCandidatesPanel({
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
}: MaintenanceCandidatesPanelProps) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();

  /// A candidate carries no mode of its own; the shadow marker comes from the
  /// rule that opened it, which is why this admin view always asks for shadow
  /// candidates and then labels them here.
  // What a tag rule would write, keyed by rule so a candidate row can say
  // "add needs-review" rather than only naming the action. The rule records
  // already carry the action spec of the revision in force, so this costs no
  // extra request.
  const tagPatchByRuleId = React.useMemo(() => {
    const patches = new Map<string, { kind: string; tags: string[] }>();
    for (const record of ruleSetRecords) {
      const tags = record.actionSpec?.tags ?? [];
      if (tags.length > 0) {
        patches.set(record.id, { kind: record.actionSpec.kind, tags });
      }
    }
    return patches;
  }, [ruleSetRecords]);

  const shadowRuleIds = React.useMemo(
    () =>
      new Set(
        ruleSetRecords
          .filter((record) => record.evaluationMode === "SHADOW")
          .map((record) => record.id),
      ),
    [ruleSetRecords],
  );

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">
          {t("settings.maintenanceCandidatesTitle")}
        </CardTitle>
        <p className="text-xs text-muted-foreground">
          {t("settings.maintenanceCandidatesSubtitle")}
        </p>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="grid gap-3 md:grid-cols-4">
          <SingleSelectField
            id="settings-maintenance-candidates-rule"
            label={t("settings.maintenanceCandidatesFilterRule")}
            value={candidateRuleFilter}
            onValueChange={setCandidateRuleFilter}
            options={[
              {
                value: MAINTENANCE_FILTER_ALL,
                label: t("settings.maintenanceCandidatesAllRules"),
              },
              ...ruleSetRecords.map((record) => ({
                value: record.id,
                label: record.name,
              })),
            ]}
          />
          <SingleSelectField
            id="settings-maintenance-candidates-state"
            label={t("settings.maintenanceCandidatesFilterState")}
            value={candidateStateFilter}
            onValueChange={setCandidateStateFilter}
            options={[
              {
                value: MAINTENANCE_FILTER_ALL,
                label: t("settings.maintenanceCandidatesAllStates"),
              },
              ...CANDIDATE_STATE_FILTER_OPTIONS.map((state) => {
                const labelKey = candidateStateLabelKey(state);
                return { value: state, label: labelKey ? t(labelKey) : state };
              }),
            ]}
          />
          <SingleSelectField
            id="settings-maintenance-candidates-library"
            label={t("settings.maintenanceCandidatesFilterLibrary")}
            value={candidateLibraryFilter}
            onValueChange={setCandidateLibraryFilter}
            options={[
              {
                value: MAINTENANCE_FILTER_ALL,
                label: t("settings.maintenanceCandidatesAllLibraries"),
              },
              ...libraries.map((library) => ({
                value: library.id,
                label: library.name,
              })),
            ]}
          />
          <div className="flex items-end">
            <Button
              id="settings-maintenance-candidates-refresh"
              type="button"
              variant="secondary"
              disabled={candidatesLoading}
              onClick={refreshCandidates}
            >
              <RefreshCw
                className={`mr-2 h-4 w-4 ${candidatesLoading ? "animate-spin" : ""}`}
              />
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
          <Table id="settings-maintenance-candidates-table">
            <TableHeader>
              <TableRow>
                <TableHead>{t("label.title")}</TableHead>
                <TableHead>{t("settings.maintenanceCandidatesColRule")}</TableHead>
                <TableHead className="w-[150px]">
                  {t("settings.maintenanceCandidatesColState")}
                </TableHead>
                <TableHead>{t("settings.maintenancePreviewColReasons")}</TableHead>
                <TableHead className="whitespace-nowrap">
                  {t("settings.maintenanceCandidatesColFirstMatched")}
                </TableHead>
                <TableHead className="whitespace-nowrap">
                  {t("settings.maintenanceCandidatesColDue")}
                </TableHead>
                <TableHead className="text-right">{t("label.actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {candidates.map((candidate) => {
                const stateKey = candidateStateLabelKey(candidate.state);
                return (
                  <TableRow
                    key={candidate.id}
                    data-ui="settings-table-row"
                    id={selectorId("settings-maintenance-candidate-row", candidate.id)}
                  >
                    <TableCell className="font-medium">
                      <div className="flex flex-col gap-1">
                        <span>{candidate.titleName}</span>
                        {shadowRuleIds.has(candidate.ruleSetId) ? (
                          <Badge tone="outline" className="w-fit">
                            {t("settings.maintenanceCandidatesShadowBadge")}
                          </Badge>
                        ) : null}
                      </div>
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      <div className="flex flex-col">
                        <span>{candidate.ruleName}</span>
                        <span className="text-xs">
                          <ActionKindLabel kind={candidate.actionKind} />
                        </span>
                        <TagPatchSummary
                          patch={tagPatchByRuleId.get(candidate.ruleSetId)}
                        />
                      </div>
                    </TableCell>
                    <TableCell>
                      <div className="flex flex-col gap-1">
                        <Badge
                          tone={candidateStateBadgeTone(candidate.state)}
                          title={candidate.stateReason || undefined}
                        >
                          {stateKey ? t(stateKey) : candidate.state}
                        </Badge>
                        {candidate.heldSince ? (
                          <span
                            className="text-xs text-[var(--scry-warning-text)]"
                            title={candidate.stateReason || undefined}
                          >
                            {t("settings.maintenanceCandidatesHeldSince", {
                              time: timestamp(candidate.heldSince, dateTimeFormat),
                            })}
                          </span>
                        ) : null}
                      </div>
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      <ReasonCodes codes={candidate.reasonCodes} />
                      {candidate.stateReason ? (
                        <p className="mt-1 max-w-[280px] truncate text-xs">
                          {candidate.stateReason}
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
                          "settings-maintenance-candidate-exclude",
                          candidate.id,
                        )}
                        label={t("settings.maintenanceCandidateExclude")}
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

function runCounts(run: MaintenanceEvaluationRun, t: Translate): string {
  return t("settings.maintenanceRunCounts", {
    evaluated: run.evaluatedCount,
    matched: run.matchedCount,
    noMatch: run.noMatchCount,
    unknown: run.unknownCount,
    errors: run.errorCount,
  });
}

export type MaintenanceRunsPanelProps = {
  evaluationRuns: MaintenanceEvaluationRun[];
  actionRuns: MaintenanceActionRun[];
  runsError: string | null;
  ruleSetRecords: MaintenanceRuleSetRecord[];
  runScopeRuleSetId: string;
  setRunScopeRuleSetId: (id: string) => void;
  evaluationTriggering: boolean;
  actionTriggering: boolean;
  runEvaluationNow: () => void;
  runActionHandlerNow: () => void;
  refreshRuns: () => void;
  runsLoading: boolean;
};

export function MaintenanceRunsPanel({
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
}: MaintenanceRunsPanelProps) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">{t("settings.maintenanceRunsTitle")}</CardTitle>
        <p className="text-xs text-muted-foreground">
          {t("settings.maintenanceRunsSubtitle")}
        </p>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="grid gap-3 md:grid-cols-4">
          <SingleSelectField
            id="settings-maintenance-run-scope"
            label={t("settings.maintenanceRunScope")}
            value={runScopeRuleSetId}
            onValueChange={setRunScopeRuleSetId}
            options={[
              {
                value: MAINTENANCE_FILTER_ALL,
                label: t("settings.maintenanceRunScopeAll"),
              },
              ...ruleSetRecords.map((record) => ({
                value: record.id,
                label: record.name,
              })),
            ]}
          />
          <div className="flex items-end">
            <Button
              id="settings-maintenance-run-evaluation-now"
              type="button"
              variant="secondary"
              disabled={evaluationTriggering}
              onClick={runEvaluationNow}
            >
              <Play className="mr-2 h-4 w-4" />
              {evaluationTriggering
                ? t("settings.maintenanceRunEvaluationRunning")
                : t("settings.maintenanceRunEvaluationNow")}
            </Button>
          </div>
          <div className="flex items-end">
            <Button
              id="settings-maintenance-run-actions-now"
              type="button"
              variant="secondary"
              disabled={actionTriggering}
              onClick={runActionHandlerNow}
            >
              <Play className="mr-2 h-4 w-4" />
              {actionTriggering
                ? t("settings.maintenanceRunActionsRunning")
                : t("settings.maintenanceRunActionsNow")}
            </Button>
          </div>
          <div className="flex items-end">
            <Button
              id="settings-maintenance-runs-refresh"
              type="button"
              variant="secondary"
              disabled={runsLoading}
              onClick={refreshRuns}
            >
              <RefreshCw
                className={`mr-2 h-4 w-4 ${runsLoading ? "animate-spin" : ""}`}
              />
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
            {t("settings.maintenanceEvaluationRunsTitle")}
          </h4>
          <div className="overflow-x-auto">
            <Table id="settings-maintenance-evaluation-runs">
              <TableHeader>
                <TableRow>
                  <TableHead className="whitespace-nowrap">
                    {t("settings.maintenanceRunColStarted")}
                  </TableHead>
                  <TableHead>{t("settings.maintenanceCandidatesColRule")}</TableHead>
                  <TableHead className="w-[130px]">
                    {t("settings.maintenanceRunColStatus")}
                  </TableHead>
                  <TableHead>{t("settings.maintenanceRunColCounts")}</TableHead>
                  <TableHead className="whitespace-nowrap">
                    {t("settings.maintenanceRunColDuration")}
                  </TableHead>
                  <TableHead>{t("settings.maintenanceRunColError")}</TableHead>
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
                      id={selectorId("settings-maintenance-evaluation-run-row", run.id)}
                    >
                      <TableCell className="whitespace-nowrap text-muted-foreground">
                        {timestamp(run.startedAt, dateTimeFormat)}
                      </TableCell>
                      <TableCell>
                        {rule?.name ?? run.ruleSetId}
                        <span className="ml-1 text-xs text-muted-foreground">
                          {t("settings.maintenanceRunRevision", {
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
                          : t("settings.maintenanceRunDuration", {
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
                      {t("settings.maintenanceEvaluationRunsEmpty")}
                    </TableCell>
                  </TableRow>
                ) : null}
              </TableBody>
            </Table>
          </div>
        </div>

        <div className="space-y-2">
          <h4 className="text-sm font-semibold">
            {t("settings.maintenanceActionRunsTitle")}
          </h4>
          <div className="overflow-x-auto">
            <Table id="settings-maintenance-action-runs">
              <TableHeader>
                <TableRow>
                  <TableHead className="whitespace-nowrap">
                    {t("settings.maintenanceRunColStarted")}
                  </TableHead>
                  <TableHead>{t("label.title")}</TableHead>
                  <TableHead>{t("settings.maintenanceRuleAction")}</TableHead>
                  <TableHead className="w-[150px]">
                    {t("settings.maintenanceRunColStatus")}
                  </TableHead>
                  <TableHead className="text-center">
                    {t("settings.maintenanceActionRunColAttempt")}
                  </TableHead>
                  <TableHead>
                    {t("settings.maintenanceActionRunColHoldReason")}
                  </TableHead>
                  <TableHead>{t("settings.maintenanceRunColError")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {actionRuns.map((run) => (
                  <TableRow
                    key={run.id}
                    data-ui="settings-table-row"
                    id={selectorId("settings-maintenance-action-run-row", run.id)}
                  >
                    <TableCell className="whitespace-nowrap text-muted-foreground">
                      {timestamp(run.startedAt, dateTimeFormat)}
                    </TableCell>
                    <TableCell className="font-medium">{run.titleName}</TableCell>
                    <TableCell className="text-muted-foreground">
                      <ActionKindLabel kind={run.actionKind} />
                    </TableCell>
                    <TableCell>
                      <RunStatusBadge status={run.status} />
                    </TableCell>
                    <TableCell className="text-center text-muted-foreground">
                      {run.attempt}
                    </TableCell>
                    <TableCell className="max-w-[220px] truncate text-[var(--scry-warning-text)]">
                      {run.holdReason ?? "—"}
                    </TableCell>
                    <TableCell className="max-w-[220px] truncate text-[var(--scry-danger-text)]">
                      {run.error ?? "—"}
                    </TableCell>
                  </TableRow>
                ))}
                {actionRuns.length === 0 ? (
                  <TableRow>
                    <TableCell colSpan={7} className="text-muted-foreground">
                      {t("settings.maintenanceActionRunsEmpty")}
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

export type MaintenanceExclusionsPanelProps = {
  exclusions: MaintenanceExclusion[];
  exclusionsError: string | null;
  ruleSetRecords: MaintenanceRuleSetRecord[];
  removingExclusionId: string | null;
  removeExclusion: (exclusion: MaintenanceExclusion) => void;
};

export function MaintenanceExclusionsPanel({
  exclusions,
  exclusionsError,
  ruleSetRecords,
  removingExclusionId,
  removeExclusion,
}: MaintenanceExclusionsPanelProps) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">
          {t("settings.maintenanceExclusionsTitle")}
        </CardTitle>
        <p className="text-xs text-muted-foreground">
          {t("settings.maintenanceExclusionsSubtitle")}
        </p>
      </CardHeader>
      <CardContent className="space-y-3">
        {exclusionsError ? (
          <pre className="overflow-x-auto whitespace-pre-wrap rounded-[9px] border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] p-3 font-mono text-[12px] leading-5 text-[var(--scry-danger-text)]">
            {exclusionsError}
          </pre>
        ) : null}
        <div className="overflow-x-auto">
          <Table id="settings-maintenance-exclusions">
            <TableHeader>
              <TableRow>
                <TableHead>{t("label.title")}</TableHead>
                <TableHead>{t("settings.maintenanceExclusionColScope")}</TableHead>
                <TableHead>{t("settings.maintenanceExclusionColReason")}</TableHead>
                <TableHead className="whitespace-nowrap">
                  {t("settings.maintenanceExclusionColCreated")}
                </TableHead>
                <TableHead className="text-right">{t("label.actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {exclusions.map((exclusion) => {
                const rule = exclusion.ruleSetId
                  ? ruleSetRecords.find((record) => record.id === exclusion.ruleSetId)
                  : null;
                return (
                  <TableRow
                    key={exclusion.id}
                    data-ui="settings-table-row"
                    id={selectorId("settings-maintenance-exclusion-row", exclusion.id)}
                  >
                    <TableCell className="font-medium">{exclusion.titleName}</TableCell>
                    <TableCell className="text-muted-foreground">
                      {exclusion.ruleSetId
                        ? (rule?.name ?? exclusion.ruleSetId)
                        : t("settings.maintenanceExclusionScopeGlobal")}
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
                          "settings-maintenance-exclusion-remove",
                          exclusion.id,
                        )}
                        label={t("settings.maintenanceExclusionRemove")}
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
                    {t("settings.maintenanceExclusionsEmpty")}
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

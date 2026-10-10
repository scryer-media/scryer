import { useState, type ReactNode } from "react";

import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Button } from "@/components/ui/button";
import { useTranslate } from "@/lib/context/translate-context";
import type { PostProcessingScriptRun } from "@/lib/types/scripts";
import { selectorId } from "@/lib/utils/dom-ids";

/** Which captured stream the output column shows. */
export type ScriptOutputFilter = "combined" | "stdout" | "stderr";

const OUTPUT_FILTERS: readonly ScriptOutputFilter[] = ["combined", "stdout", "stderr"];

/** Combined / Stdout / Stderr buttons; each button's id is `<id>-<filter>`. */
export function ScriptOutputFilterGroup({
  id,
  value,
  onChange,
  className,
}: {
  id: string;
  value: ScriptOutputFilter;
  onChange: (value: ScriptOutputFilter) => void;
  className?: string;
}) {
  const t = useTranslate();
  return (
    <div
      id={id}
      role="group"
      aria-label={t("script.output.filter")}
      className={className ?? "flex justify-end gap-1"}
    >
      {OUTPUT_FILTERS.map((filter) => (
        <Button
          key={filter}
          id={`${id}-${filter}`}
          type="button"
          size="xs"
          variant={value === filter ? "secondary" : "outline"}
          aria-pressed={value === filter}
          onClick={() => onChange(filter)}
        >
          {t(`script.output.filter.${filter}`)}
        </Button>
      ))}
    </div>
  );
}

export function scriptRunStatusColor(status: string): string {
  switch (status) {
    case "success":
      return "text-[var(--scry-success-text-soft)]";
    case "failed":
      return "text-[var(--scry-danger-text-soft)]";
    case "timeout":
      return "text-[var(--scry-warning-text)]";
    case "running":
      return "text-[var(--scry-info-text-soft)]";
    default:
      return "text-muted-foreground";
  }
}

export function formatScriptRunDuration(ms: number | null): string {
  if (ms == null) return "--";
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

/** Element ids for each part of a run row, so each page keeps its own. */
export type ScriptRunsTableIds = {
  empty: (scriptId: string) => string;
  row: (run: PostProcessingScriptRun) => string;
  status: (run: PostProcessingScriptRun) => string;
  exitCode?: (run: PostProcessingScriptRun) => string;
  stdout: (run: PostProcessingScriptRun) => string;
  stderr: (run: PostProcessingScriptRun) => string;
  /** The output filter group; each button is `<group>-<filter>`. */
  outputFilter: string;
};

export const SETTINGS_SCRIPT_RUN_IDS: ScriptRunsTableIds = {
  empty: (scriptId) => selectorId("settings-post-processing-no-runs", scriptId),
  row: (run) =>
    selectorId(
      "settings-post-processing-run-row",
      run.status,
      run.titleName || run.titleId || "unknown-title",
      run.id,
    ),
  status: (run) => selectorId("settings-post-processing-run-status", run.id),
  stdout: (run) => selectorId("settings-post-processing-run-stdout", run.id),
  stderr: (run) => selectorId("settings-post-processing-run-stderr", run.id),
  outputFilter: "settings-post-processing-output-filter",
};

export type ScriptRunsLeadingColumn = {
  header: string;
  render: (run: PostProcessingScriptRun) => ReactNode;
};

function renderRunTitle(run: PostProcessingScriptRun): ReactNode {
  return run.titleName || run.titleId || "--";
}

/** Recent runs of one script with their status, duration and captured output. */
export function ScriptRunsTable({
  scriptId,
  runs,
  noRunsLabel,
  outputNotCapturedLabel,
  ids = SETTINGS_SCRIPT_RUN_IDS,
  leadingColumn,
}: {
  scriptId: string;
  runs: PostProcessingScriptRun[];
  noRunsLabel: string;
  outputNotCapturedLabel: string;
  ids?: ScriptRunsTableIds;
  leadingColumn?: ScriptRunsLeadingColumn;
}) {
  const t = useTranslate();
  const [outputFilter, setOutputFilter] = useState<ScriptOutputFilter>("combined");
  const leading: ScriptRunsLeadingColumn = leadingColumn ?? {
    header: t("label.title"),
    render: renderRunTitle,
  };

  if (runs.length === 0) {
    return (
      <p id={ids.empty(scriptId)} className="px-3 py-4 text-xs text-muted-foreground">
        {noRunsLabel}
      </p>
    );
  }
  const showStdout = outputFilter !== "stderr";
  const showStderr = outputFilter !== "stdout";
  const showStreamLabels = outputFilter === "combined";
  return (
    <div className="space-y-2">
      <ScriptOutputFilterGroup
        id={ids.outputFilter}
        value={outputFilter}
        onChange={setOutputFilter}
      />
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>{leading.header}</TableHead>
            <TableHead>{t("label.status")}</TableHead>
            <TableHead>{t("label.duration")}</TableHead>
            <TableHead>{t("label.output")}</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {runs.map((run) => {
            const stdout = showStdout ? run.stdoutTail : null;
            const stderr = showStderr ? run.stderrTail : null;
            return (
              <TableRow data-ui="settings-table-row" key={run.id} id={ids.row(run)}>
                <TableCell className="text-xs">{leading.render(run)}</TableCell>
                <TableCell>
                  <span
                    id={ids.status(run)}
                    className={`text-xs font-medium capitalize ${scriptRunStatusColor(run.status)}`}
                  >
                    {run.status}
                    {run.exitCode != null && run.status === "failed" ? (
                      <span id={ids.exitCode?.(run)}>{` (exit ${run.exitCode})`}</span>
                    ) : null}
                  </span>
                </TableCell>
                <TableCell className="text-xs">{formatScriptRunDuration(run.durationMs)}</TableCell>
                <TableCell className="max-w-[400px]">
                  {stdout || stderr ? (
                    <div className="space-y-1">
                      {stdout ? (
                        <div>
                          {showStreamLabels ? (
                            <p className="mb-0.5 text-[10px] font-medium text-muted-foreground">
                              {t("script.output.stream.stdout")}
                            </p>
                          ) : null}
                          <pre
                            id={ids.stdout(run)}
                            className="max-h-24 overflow-auto whitespace-pre-wrap rounded bg-muted/50 p-1.5 font-[var(--font-code)] text-[10px] leading-relaxed text-muted-foreground"
                          >
                            {stdout}
                          </pre>
                        </div>
                      ) : null}
                      {stderr ? (
                        <div>
                          {showStreamLabels ? (
                            <p className="mb-0.5 text-[10px] font-medium text-muted-foreground">
                              {t("script.output.stream.stderr")}
                            </p>
                          ) : null}
                          <pre
                            id={ids.stderr(run)}
                            className="max-h-24 overflow-auto whitespace-pre-wrap rounded bg-[var(--scry-danger-bg)] p-1.5 font-[var(--font-code)] text-[10px] leading-relaxed text-[var(--scry-danger-text)]"
                          >
                            {stderr}
                          </pre>
                        </div>
                      ) : null}
                    </div>
                  ) : (
                    <span className="text-[10px] text-muted-foreground">{outputNotCapturedLabel}</span>
                  )}
                </TableCell>
              </TableRow>
            );
          })}
        </TableBody>
      </Table>
    </div>
  );
}

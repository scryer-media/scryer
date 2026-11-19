import * as React from "react";
import { useClient } from "urql";

import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import { startExternalRuleImportMutation } from "@/lib/graphql/mutations";
import { externalRuleImportStatusQuery } from "@/lib/graphql/queries";
import { useTranslate } from "@/lib/context/translate-context";
import { useTrackedJobRuns } from "@/lib/hooks/use-tracked-job-runs";
import type {
  ExternalRuleImportInput,
  ExternalRuleImportSource,
  ExternalRuleImportStatus,
} from "@/lib/types/external-import";
import type { JobRun } from "@/lib/types/jobs";
import { normalizeExternalRuleImportStatus } from "@/lib/utils/external-rule-import";
import { isTerminalJobRunStatus, normalizeJobRun } from "@/lib/utils/job-runs";

export type ExternalRuleImportController = {
  /** The run this view started, as the job-run stream last reported it. */
  run: JobRun | null;
  /** The source the current run reads from. */
  source: ExternalRuleImportSource | null;
  /** True from acceptance until the run reaches a terminal state. */
  running: boolean;
  /** The final per-item report, loaded once the run has finished. */
  report: ExternalRuleImportStatus | null;
  starting: boolean;
  loadingReport: boolean;
  /** Display copy for the last start failure. */
  error: string | null;
  /** Display copy for the last report load failure; `reloadReport` retries. */
  reportError: string | null;
  start: (input: ExternalRuleImportInput) => Promise<JobRun | null>;
  /** Reloads the report of the current run, for example after a reconnect. */
  reloadReport: () => Promise<void>;
  reset: () => void;
};

export type ExternalRuleImportOptions = {
  /** Called once any run this controller started reaches a terminal state. */
  onTerminal?: (run: JobRun) => void;
};

/** True while an import is starting, running or loading its report. */
export function ruleImportBusy(
  controller: Pick<
    ExternalRuleImportController,
    "starting" | "running" | "loadingReport"
  >,
): boolean {
  return controller.starting || controller.running || controller.loadingReport;
}

function sameInput(
  left: ExternalRuleImportInput,
  right: ExternalRuleImportInput,
): boolean {
  return (
    left.source === right.source &&
    left.baseUrl.trim() === right.baseUrl.trim() &&
    (left.apiKey ?? "") === (right.apiKey ?? "")
  );
}

function newRequestKey(): string {
  return globalThis.crypto.randomUUID();
}

/**
 * Starts an external rule import and follows it. Progress comes from the
 * job-run provider, which listens to the event stream, so this hook runs no
 * poller of its own; it reads the report once when the run reaches a terminal
 * state. Mount it above any dialog that shows the run, so closing the dialog
 * neither drops the run nor its terminal callback.
 */
export function useExternalRuleImport(
  options: ExternalRuleImportOptions = {},
): ExternalRuleImportController {
  const client = useClient();
  const t = useTranslate();
  const trackJobRun = useTrackedJobRuns();
  const { onTerminal } = options;
  const onTerminalRef = React.useRef(onTerminal);
  React.useEffect(() => {
    onTerminalRef.current = onTerminal;
  }, [onTerminal]);
  const [run, setRun] = React.useState<JobRun | null>(null);
  const [source, setSource] = React.useState<ExternalRuleImportSource | null>(
    null,
  );
  const [report, setReport] = React.useState<ExternalRuleImportStatus | null>(
    null,
  );
  const [starting, setStarting] = React.useState(false);
  const [loadingReport, setLoadingReport] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [reportError, setReportError] = React.useState<string | null>(null);
  // A start whose response was lost is resent with the same key, so the
  // server returns the run it already accepted instead of starting another.
  const pendingRequestRef = React.useRef<{
    key: string;
    input: ExternalRuleImportInput;
  } | null>(null);
  const currentRunIdRef = React.useRef<string | null>(null);

  const loadReport = React.useCallback(
    async (runId: string) => {
      setLoadingReport(true);
      setReportError(null);
      try {
        const { data, error: queryError } = await client
          .query<{ externalRuleImportStatus?: unknown }>(
            externalRuleImportStatusQuery,
            { runId },
            { requestPolicy: "network-only" },
          )
          .toPromise();
        if (queryError) throw queryError;
        const next = normalizeExternalRuleImportStatus(
          data?.externalRuleImportStatus,
        );
        if (!next) throw new Error(t("status.apiError"));
        if (currentRunIdRef.current === runId) {
          setReport(next);
          setReportError(null);
        }
      } catch (loadError: unknown) {
        if (currentRunIdRef.current === runId) {
          setReportError(
            userFacingGraphQlErrorMessage(loadError, t("status.apiError"), t),
          );
        }
      } finally {
        if (currentRunIdRef.current === runId) setLoadingReport(false);
      }
    },
    [client, t],
  );

  const start = React.useCallback(
    async (input: ExternalRuleImportInput) => {
      const pending = pendingRequestRef.current;
      const requestKey =
        pending && sameInput(pending.input, input)
          ? pending.key
          : newRequestKey();
      pendingRequestRef.current = { key: requestKey, input };
      setStarting(true);
      setError(null);
      try {
        const { data, error: mutationError } = await client
          .mutation<{ startExternalRuleImport?: unknown }>(
            startExternalRuleImportMutation,
            {
              requestKey,
              input: {
                source: input.source,
                baseUrl: input.baseUrl.trim(),
                apiKey: input.source === "MAINTAINERR" ? null : input.apiKey,
              },
            },
          )
          .toPromise();
        if (mutationError) {
          // A refusal is an answer; only a lost response keeps the key.
          if (!mutationError.networkError) pendingRequestRef.current = null;
          throw mutationError;
        }
        const accepted = normalizeJobRun(data?.startExternalRuleImport);
        if (!accepted) throw new Error(t("status.apiError"));
        pendingRequestRef.current = null;
        currentRunIdRef.current = accepted.id;
        setRun(accepted);
        setSource(input.source);
        setReport(null);
        setReportError(null);
        trackJobRun(accepted, (terminalRun) => {
          // A run the viewer has since cleared may still have saved rules.
          onTerminalRef.current?.(terminalRun);
          if (currentRunIdRef.current !== terminalRun.id) return;
          setRun(terminalRun);
          void loadReport(terminalRun.id);
        });
        return accepted;
      } catch (startError: unknown) {
        setError(
          userFacingGraphQlErrorMessage(startError, t("status.apiError"), t),
        );
        return null;
      } finally {
        setStarting(false);
      }
    },
    [client, loadReport, t, trackJobRun],
  );

  const reloadReport = React.useCallback(async () => {
    const runId = currentRunIdRef.current;
    if (runId) await loadReport(runId);
  }, [loadReport]);

  const reset = React.useCallback(() => {
    currentRunIdRef.current = null;
    pendingRequestRef.current = null;
    setRun(null);
    setSource(null);
    setReport(null);
    setError(null);
    setReportError(null);
    setLoadingReport(false);
  }, []);

  return {
    run,
    source,
    running: run !== null && !isTerminalJobRunStatus(run.status),
    report,
    starting,
    loadingReport,
    error,
    reportError,
    start,
    reloadReport,
    reset,
  };
}

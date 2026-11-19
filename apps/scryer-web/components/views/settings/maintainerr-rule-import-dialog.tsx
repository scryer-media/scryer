import * as React from "react";
import { AlertCircle } from "lucide-react";
import { LoadingMark } from "@/components/common/loading-mark";
import { useJobRunToasts } from "@/components/root/job-run-provider";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useTranslate } from "@/lib/context/translate-context";
import {
  ruleImportBusy,
  type ExternalRuleImportController,
} from "@/lib/hooks/use-external-rule-import";
import type {
  ExternalRuleImportItem,
  ExternalRuleImportItemState,
} from "@/lib/types/external-import";
import {
  ruleImportProgress,
  ruleImportReasonMessage,
} from "@/lib/utils/external-rule-import";
import {
  maintainerrDiagnosticMessage,
  maintainerrDiagnostics,
} from "@/lib/utils/maintainerr-rule-import";

type MaintainerrRuleImportDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /**
   * Owned by the page, so a run keeps being followed after the dialog closes
   * and reopening shows it.
   */
  controller: ExternalRuleImportController;
};

function ItemStateLabel({ state }: { state: ExternalRuleImportItemState }) {
  const t = useTranslate();
  switch (state) {
    case "SAVED":
    case "SAVED_NEEDS_REVIEW":
      return <>{t("settings.maintainerrImportStateSaved")}</>;
    case "SAVED_FAILED_TRANSLATION":
      return <>{t("settings.maintainerrImportStateSavedFailedTranslation")}</>;
    case "SKIPPED_EXISTS":
      return <>{t("settings.maintainerrImportStateSkipped")}</>;
    case "PENDING":
      return <>{t("settings.maintainerrImportStatePending")}</>;
    default:
      return <>{t("settings.maintainerrImportStateFailed")}</>;
  }
}

function ReportItem({
  item,
  index,
}: {
  item: ExternalRuleImportItem;
  index: number;
}) {
  const t = useTranslate();
  // A skipped group wrote nothing, so it has nothing to check.
  const diagnostics =
    item.state === "SKIPPED_EXISTS" ? [] : maintainerrDiagnostics(item);
  return (
    <li
      id={`settings-lifecycle-rules-maintainerr-item-${index}`}
      data-state={item.state}
      className="rounded border border-border p-2 text-sm"
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <span className="font-medium">{item.savedName ?? item.sourceName}</span>
        <span className="text-xs text-muted-foreground">
          <ItemStateLabel state={item.state} />
        </span>
      </div>
      {/* The state label already says skipped or untranslated; only a
          failed save has a refusal of its own to show. */}
      {item.state === "FAILED" && item.reason ? (
        <p className="mt-1 text-xs text-muted-foreground">
          {ruleImportReasonMessage(
            item.reason,
            item.message,
            t("error.reason.RULE_IMPORT_ITEM_FAILED"),
            t,
          )}
        </p>
      ) : null}
      {diagnostics.length ? (
        <ul className="mt-1 list-disc space-y-0.5 pl-5 text-xs text-muted-foreground">
          {diagnostics.map((diagnostic, diagnosticIndex) => {
            const message = maintainerrDiagnosticMessage(diagnostic, t);
            const severity =
              diagnostic.severity === "ERROR"
                ? t("settings.maintainerrImportSeverityError")
                : t("settings.maintainerrImportSeverityWarning");
            return (
              <li key={diagnosticIndex} data-code={diagnostic.code}>
                {diagnostic.ruleIndex === null
                  ? t("settings.maintainerrImportDiagnostic", {
                      severity,
                      message,
                    })
                  : t("settings.maintainerrImportDiagnosticAtRule", {
                      severity,
                      message,
                      rule: diagnostic.ruleIndex + 1,
                    })}
              </li>
            );
          })}
        </ul>
      ) : null}
    </li>
  );
}

/**
 * Reads the rule groups of a running Maintainerr. The server saves each group
 * as its own disabled lifecycle rule; this dialog starts the run and shows its
 * progress and report.
 */
export function MaintainerrRuleImportDialog({
  open,
  onOpenChange,
  controller,
}: MaintainerrRuleImportDialogProps) {
  const t = useTranslate();
  const { runs } = useJobRunToasts();
  const {
    run,
    running,
    report,
    starting,
    loadingReport,
    error,
    reportError,
    start,
    reloadReport,
    reset,
  } = controller;
  const [baseUrl, setBaseUrl] = React.useState("");

  // The stream keeps the shared run list current; the controller only holds
  // the accepted and the terminal snapshot.
  const liveRun = React.useMemo(
    () =>
      run ? (runs.find((candidate) => candidate.id === run.id) ?? run) : null,
    [run, runs],
  );
  const busy = ruleImportBusy(controller);

  const progress = running ? ruleImportProgress(liveRun?.progressJson) : null;
  const counts = report?.counts ?? progress?.counts ?? null;

  // A run that stopped before saving anything keeps the form and its URL, so
  // the operator can correct it and start again.
  const runFailure = report?.reason
    ? ruleImportReasonMessage(
        report.reason,
        report.message,
        t("settings.maintainerrImportRunFailed"),
        t,
      )
    : null;
  // An interrupted save still has saved rules to report.
  const finished =
    Boolean(report) && (!runFailure || Boolean(report?.items.length));
  const failure = error ?? runFailure;

  const begin = React.useCallback(() => {
    void start({ source: "MAINTAINERR", baseUrl, apiKey: null });
  }, [baseUrl, start]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        id="settings-lifecycle-rules-maintainerr-dialog"
        className="max-h-[90vh] overflow-y-auto sm:max-w-2xl"
      >
        <DialogHeader>
          <DialogTitle>{t("settings.maintainerrImportTitle")}</DialogTitle>
          <DialogDescription className="sr-only">
            {t("settings.maintainerrImportDescription")}
          </DialogDescription>
        </DialogHeader>

        <div className="min-w-0 space-y-4">
          {finished ? null : (
            <>
              <label className="block">
                <Label
                  className="mb-2 block"
                  htmlFor="settings-lifecycle-rules-maintainerr-url"
                >
                  {t("settings.maintainerrImportBaseUrl")}
                </Label>
                <Input
                  id="settings-lifecycle-rules-maintainerr-url"
                  type="url"
                  value={baseUrl}
                  disabled={busy}
                  placeholder="http://localhost:6246"
                  onChange={(event) => setBaseUrl(event.target.value)}
                />
              </label>
            </>
          )}

          {failure ? (
            <div
              id="settings-lifecycle-rules-maintainerr-error"
              className="flex gap-2 rounded border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] px-3 py-2 text-sm text-[var(--scry-danger-text)]"
            >
              <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" />
              <div className="min-w-0 whitespace-pre-wrap break-words">
                {failure}
              </div>
            </div>
          ) : null}

          {reportError && !loadingReport ? (
            <div
              id="settings-lifecycle-rules-maintainerr-report-error"
              className="flex flex-wrap items-start gap-2 rounded border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] px-3 py-2 text-sm text-[var(--scry-danger-text)]"
            >
              <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" />
              <div className="min-w-0 flex-1 whitespace-pre-wrap">
                {reportError}
              </div>
              <Button
                id="settings-lifecycle-rules-maintainerr-retry"
                type="button"
                variant="secondary"
                className="h-7 px-2 text-xs"
                onClick={() => void reloadReport()}
              >
                {t("label.retry")}
              </Button>
            </div>
          ) : null}

          {running || loadingReport ? (
            <p
              id="settings-lifecycle-rules-maintainerr-progress"
              className="flex items-center gap-2 text-sm text-muted-foreground"
            >
              <LoadingMark className="h-4 w-4" />
              {counts && counts.total > 0
                ? t("settings.maintainerrImportRunningCount", {
                    done: counts.total - counts.pending,
                    total: counts.total,
                  })
                : t("settings.maintainerrImportRunning")}
            </p>
          ) : null}

          {report && finished ? (
            <div
              id="settings-lifecycle-rules-maintainerr-report"
              className="space-y-3 rounded border border-border p-3"
            >
              {runFailure ? (
                <p className="text-sm text-[var(--scry-danger-text)]">
                  {runFailure}
                </p>
              ) : null}
              {counts ? (
                <p className="text-sm font-medium">
                  {t("settings.maintainerrImportCounts", {
                    total: counts.total,
                    saved: counts.saved + counts.savedNeedsReview,
                    untranslated: counts.savedFailedTranslation,
                    skipped: counts.skippedExists,
                    failed: counts.failed,
                  })}
                </p>
              ) : null}
              {report.items.length ? (
                <ul className="space-y-2">
                  {report.items.map((item, index) => (
                    <ReportItem key={item.key} item={item} index={index} />
                  ))}
                </ul>
              ) : null}
            </div>
          ) : null}
        </div>

        <DialogFooter>
          <Button
            id="settings-lifecycle-rules-maintainerr-close"
            type="button"
            variant="secondary"
            onClick={() => onOpenChange(false)}
          >
            {run ? t("label.close") : t("label.cancel")}
          </Button>
          {finished || (run && !report && !busy) ? (
            <Button
              id="settings-lifecycle-rules-maintainerr-again"
              type="button"
              onClick={reset}
            >
              {t("settings.maintainerrImportStartAnother")}
            </Button>
          ) : (
            <Button
              id="settings-lifecycle-rules-maintainerr-start"
              type="button"
              disabled={busy || !baseUrl.trim()}
              onClick={begin}
            >
              {starting ? <LoadingMark className="mr-2 h-4 w-4" /> : null}
              {t("settings.maintainerrImportStart")}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

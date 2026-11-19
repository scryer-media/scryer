import * as React from "react";
import { AlertCircle } from "lucide-react";
import { LoadingMark } from "@/components/common/loading-mark";
import { Button } from "@/components/ui/button";
import { DialogFooter } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useTranslate } from "@/lib/context/translate-context";
import { useExternalRuleImport } from "@/lib/hooks/use-external-rule-import";
import type {
  ExternalRuleImportItem,
  ExternalRuleImportItemState,
} from "@/lib/types/external-import";
import type { ArrSource } from "@/lib/utils/arr-custom-format-import-state";
import {
  ruleImportDiagnostics,
  ruleImportProgress,
  ruleImportReasonMessage,
  unmatchedProfileNames,
} from "@/lib/utils/external-rule-import";

type ArrRuleImportConnectPanelProps = {
  source: ArrSource;
  onBusyChange: (busy: boolean) => void;
  onClose: () => void;
  /** Called once a run's report has loaded, so the rule list can refresh. */
  onImported: () => void;
};

function ItemStateLabel({ state }: { state: ExternalRuleImportItemState }) {
  const t = useTranslate();
  switch (state) {
    case "SAVED":
      return <>{t("settings.arrImportStateSaved")}</>;
    case "SAVED_NEEDS_REVIEW":
      return <>{t("settings.arrImportStateNeedsReview")}</>;
    case "SAVED_FAILED_TRANSLATION":
      return <>{t("settings.arrImportStateSavedFailedTranslation")}</>;
    case "SKIPPED_EXISTS":
      return <>{t("settings.arrImportStateSkipped")}</>;
    case "PENDING":
      return <>{t("settings.arrImportStatePending")}</>;
    default:
      return <>{t("settings.arrImportStateFailed")}</>;
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
  const unmatched = unmatchedProfileNames(item);
  const diagnostics =
    item.state === "FAILED" ? ruleImportDiagnostics(item) : [];
  return (
    <li
      id={`settings-rules-arr-connect-item-${index}`}
      data-state={item.state}
      className="rounded border border-border p-2 text-sm"
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <span className="font-medium">{item.savedName ?? item.sourceName}</span>
        <span className="text-xs text-muted-foreground">
          <ItemStateLabel state={item.state} />
        </span>
      </div>
      {item.reason ? (
        <p className="mt-1 text-xs text-muted-foreground">
          {ruleImportReasonMessage(
            item.reason,
            item.message,
            t("error.reason.RULE_IMPORT_ITEM_FAILED"),
            t,
          )}
        </p>
      ) : null}
      {unmatched.length ? (
        <p className="mt-1 text-xs text-muted-foreground">
          {t("settings.arrImportUnmatchedProfiles", {
            names: unmatched.join(", "),
          })}
        </p>
      ) : null}
      {diagnostics.length ? (
        <ul className="mt-1 list-disc space-y-0.5 pl-5 text-xs text-muted-foreground">
          {diagnostics.map((diagnostic, diagnosticIndex) => (
            <li key={diagnosticIndex}>
              {diagnostic.location
                ? t("settings.arrImportDiagnosticAt", {
                    message: diagnostic.message,
                    location: diagnostic.location,
                  })
                : diagnostic.message}
            </li>
          ))}
        </ul>
      ) : null}
    </li>
  );
}

/**
 * Reads custom formats and quality-profile scores from a running Sonarr or
 * Radarr. The server saves each format as its own disabled rule; this panel
 * starts the run and shows its report. The API key lives in component state
 * only.
 */
export function ArrRuleImportConnectPanel({
  source,
  onBusyChange,
  onClose,
  onImported,
}: ArrRuleImportConnectPanelProps) {
  const t = useTranslate();
  const controller = useExternalRuleImport();
  const { run, report, starting, loadingReport, error, start, reset } =
    controller;
  const [baseUrl, setBaseUrl] = React.useState("");
  const [apiKey, setApiKey] = React.useState("");

  const running = Boolean(run) && !report && !error;
  const busy = starting || running || loadingReport;

  React.useEffect(() => {
    onBusyChange(busy);
  }, [busy, onBusyChange]);

  const reportedRunId = report?.runId ?? null;
  React.useEffect(() => {
    if (reportedRunId) onImported();
  }, [reportedRunId, onImported]);

  const progress = running ? ruleImportProgress(run?.progressJson) : null;
  const counts = report?.counts ?? progress?.counts ?? null;

  const begin = React.useCallback(() => {
    void start({
      source: source === "radarr" ? "RADARR" : "SONARR",
      baseUrl,
      apiKey,
    });
  }, [apiKey, baseUrl, source, start]);

  return (
    <>
      <div className="space-y-4">
        <p className="text-sm text-muted-foreground">
          {t("settings.arrImportConnectDescription")}
        </p>
        <label className="block">
          <Label
            className="mb-2 block"
            htmlFor="settings-rules-arr-connect-url"
          >
            {t("settings.arrImportBaseUrl")}
          </Label>
          <Input
            id="settings-rules-arr-connect-url"
            type="url"
            value={baseUrl}
            disabled={busy}
            placeholder={
              source === "radarr"
                ? "http://localhost:7878"
                : "http://localhost:8989"
            }
            onChange={(event) => setBaseUrl(event.target.value)}
          />
        </label>
        <label className="block">
          <Label
            className="mb-2 block"
            htmlFor="settings-rules-arr-connect-api-key"
          >
            {t("settings.arrImportApiKey")}
          </Label>
          <Input
            id="settings-rules-arr-connect-api-key"
            type="password"
            autoComplete="off"
            value={apiKey}
            disabled={busy}
            onChange={(event) => setApiKey(event.target.value)}
          />
        </label>

        {error ? (
          <div className="flex gap-2 rounded border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] px-3 py-2 text-sm text-[var(--scry-danger-text)]">
            <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" />
            <div className="whitespace-pre-wrap">{error}</div>
          </div>
        ) : null}

        {running || loadingReport ? (
          <p className="flex items-center gap-2 text-sm text-muted-foreground">
            <LoadingMark className="h-4 w-4" />
            {t("settings.arrImportRunning")}
          </p>
        ) : null}

        {report ? (
          <div
            id="settings-rules-arr-connect-report"
            className="space-y-3 rounded border border-border p-3"
          >
            {report.reason ? (
              <p className="text-sm text-[var(--scry-danger-text)]">
                {ruleImportReasonMessage(
                  report.reason,
                  report.message,
                  t("settings.arrImportRunFailed"),
                  t,
                )}
              </p>
            ) : null}
            {counts ? (
              <p className="text-sm font-medium">
                {t("settings.arrImportCounts", {
                  total: counts.total,
                  saved: counts.saved,
                  review: counts.savedNeedsReview,
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
        <Button type="button" variant="secondary" onClick={onClose}>
          {report ? t("label.close") : t("label.cancel")}
        </Button>
        {report ? (
          <Button
            id="settings-rules-arr-connect-again"
            type="button"
            onClick={reset}
          >
            {t("settings.arrImportStartAnother")}
          </Button>
        ) : (
          <Button
            id="settings-rules-arr-connect-start"
            type="button"
            disabled={busy || !baseUrl.trim() || !apiKey.trim()}
            onClick={begin}
          >
            {starting ? <LoadingMark className="mr-2 h-4 w-4" /> : null}
            {t("settings.arrImportStart")}
          </Button>
        )}
      </DialogFooter>
    </>
  );
}

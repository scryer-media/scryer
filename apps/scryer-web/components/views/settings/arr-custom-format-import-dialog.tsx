import * as React from "react";
import { AlertCircle, Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input, integerInputProps } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { useTranslate } from "@/lib/context/translate-context";
import type {
  ArrSource,
  Diagnostic,
  ImportedCustomFormat,
  InspectionResult,
  TranslationResult,
} from "@/lib/arr-custom-format/types";
import {
  candidateScores,
  collectFormatScores,
  defaultScore,
  isCurrentTranslationRequest,
  recommendScore,
  sourceFacets,
} from "@/lib/utils/arr-custom-format-import-state";

export type ArrCustomFormatDraft = {
  name: string;
  description: string;
  regoSource: string;
  appliedFacets: string[];
  translationDiagnostics: string[];
};

type ArrCustomFormatImportDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onApply: (draft: ArrCustomFormatDraft) => void;
};

function diagnosticText(diagnostic: Diagnostic): string {
  return diagnostic.message;
}

function formatId(format: ImportedCustomFormat, index: number): string {
  return format.id || `format-${index}`;
}

export default function ArrCustomFormatImportDialog({
  open,
  onOpenChange,
  onApply,
}: ArrCustomFormatImportDialogProps) {
  const t = useTranslate();
  const [source, setSource] = React.useState<ArrSource>("sonarr");
  const [json, setJson] = React.useState("");
  const [inspection, setInspection] = React.useState<InspectionResult | null>(null);
  const [scores, setScores] = React.useState<Record<string, string>>({});
  const [reviewing, setReviewing] = React.useState(false);
  const [translating, setTranslating] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const requestId = React.useRef(0);
  const worker = React.useRef<Worker | null>(null);

  const cancelTranslation = React.useCallback(() => {
    requestId.current += 1;
    worker.current?.terminate();
    worker.current = null;
    setTranslating(false);
  }, []);

  React.useEffect(() => () => worker.current?.terminate(), []);

  const close = React.useCallback(() => {
    cancelTranslation();
    setReviewing(false);
    onOpenChange(false);
  }, [cancelTranslation, onOpenChange]);

  const resetReview = React.useCallback((nextJson?: string, nextSource?: ArrSource) => {
    requestId.current += 1;
    setInspection(null);
    setScores({});
    setError(null);
    if (nextJson !== undefined) setJson(nextJson);
    if (nextSource !== undefined) setSource(nextSource);
  }, []);

  const review = React.useCallback(async () => {
    const thisRequest = ++requestId.current;
    setReviewing(true);
    setError(null);
    try {
      const { inspectCustomFormats } = await import("@/lib/arr-custom-format/inspection");
      const result = inspectCustomFormats(json, source);
      if (requestId.current !== thisRequest) return;
      if (result.fatal || !result.formats?.length) {
        const diagnostics = result.diagnostics?.map(diagnosticText).filter(Boolean) ?? [];
        setInspection(null);
        setError(diagnostics.join("\n") || t("settings.arrImportNoFormats"));
        return;
      }
      setInspection(result);
      setScores(
        Object.fromEntries(
          result.formats.map((format, index) => [
            formatId(format, index),
            defaultScore(format, result.diagnostics),
          ]),
        ),
      );
    } catch (caught) {
      if (requestId.current !== thisRequest) return;
      setInspection(null);
      setError(caught instanceof Error ? caught.message : t("settings.arrImportReviewFailed"));
    } finally {
      if (requestId.current === thisRequest) setReviewing(false);
    }
  }, [json, source, t]);

  const translate = React.useCallback(() => {
    if (!inspection?.formats?.length) return;
    const scoreResult = collectFormatScores(inspection.formats, scores);
    if ("missingFormat" in scoreResult) {
      setError(t("settings.arrImportScoreRequired", { name: scoreResult.missingFormat.name }));
      return;
    }
    const scoreValues = scoreResult.scores;

    const thisRequest = ++requestId.current;
    cancelTranslation();
    requestId.current = thisRequest;
    setError(null);
    setTranslating(true);
    let nextWorker: Worker | null = null;
    try {
      const createdWorker = new Worker(
        new URL("../../../workers/arr-custom-format-translator.worker.ts", import.meta.url),
        { type: "module" },
      );
      nextWorker = createdWorker;
      worker.current = createdWorker;
      createdWorker.onmessage = (event: MessageEvent<
        | { type: "complete"; requestId: number; result: TranslationResult }
        | { type: "error"; requestId: number; message: string }
      >) => {
        if (!isCurrentTranslationRequest(requestId.current, event.data.requestId)) return;
        worker.current?.terminate();
        worker.current = null;
        setTranslating(false);
        if (event.data.type === "error") {
          setError(event.data.message);
          return;
        }
        const result = event.data.result;
        const translated = result.formats.filter((format) => format.status === "translated").length;
        const diagnostics = [
          t("settings.arrImportResultSummary", {
            translated,
            disabled: result.formats.length - translated,
          }),
          ...result.diagnostics.map(diagnosticText),
          ...result.formats.flatMap((format) => format.diagnostics.map(diagnosticText)),
        ].filter(Boolean);
        onApply({
          name: result.name || t("settings.arrImportDraftName"),
          description: result.description || t("settings.arrImportDraftDescription"),
          regoSource: result.regoSource,
          appliedFacets: result.appliedFacets ?? sourceFacets(source),
          translationDiagnostics: [...new Set(diagnostics)],
        });
        onOpenChange(false);
      };
      createdWorker.onerror = () => {
        if (!isCurrentTranslationRequest(requestId.current, thisRequest)) return;
        createdWorker.terminate();
        worker.current = null;
        setTranslating(false);
        setError(t("settings.arrImportTranslationFailed"));
      };
      createdWorker.postMessage({
        type: "translate",
        requestId: thisRequest,
        formats: inspection.formats,
        scores: scoreValues,
      });
    } catch (caught) {
      nextWorker?.terminate();
      if (worker.current === nextWorker) worker.current = null;
      if (requestId.current === thisRequest) {
        setTranslating(false);
        setError(caught instanceof Error ? caught.message : t("settings.arrImportTranslationFailed"));
      }
    }
  }, [cancelTranslation, inspection, onApply, onOpenChange, scores, source, t]);

  const formats = inspection?.formats ?? [];
  const diagnostics = inspection?.diagnostics?.map(diagnosticText).filter(Boolean) ?? [];

  return (
    <Dialog open={open} onOpenChange={(nextOpen) => (nextOpen ? onOpenChange(true) : close())}>
      <DialogContent
        id="settings-rules-arr-custom-format-import-dialog"
        className="max-h-[90vh] overflow-y-auto sm:max-w-2xl"
        onInteractOutside={(event) => translating && event.preventDefault()}
      >
        <DialogHeader>
          <DialogTitle>{t("settings.arrImportTitle")}</DialogTitle>
          <DialogDescription>{t("settings.arrImportDescription")}</DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          <fieldset>
            <Label className="mb-2 block">{t("settings.arrImportSource")}</Label>
            <div className="flex gap-2">
              {(["sonarr", "radarr"] as const).map((option) => (
                <Button
                  key={option}
                  type="button"
                  variant={source === option ? "default" : "secondary"}
                  disabled={reviewing || translating}
                  onClick={() => resetReview(undefined, option)}
                >
                  {option === "sonarr" ? "Sonarr" : "Radarr"}
                </Button>
              ))}
            </div>
          </fieldset>

          <label className="block">
            <Label className="mb-2 block" htmlFor="settings-rules-arr-custom-format-json">
              {t("settings.arrImportJson")}
            </Label>
            <Textarea
              id="settings-rules-arr-custom-format-json"
              value={json}
              rows={10}
              disabled={reviewing || translating}
              placeholder={t("settings.arrImportJsonPlaceholder")}
              onChange={(event) => resetReview(event.target.value)}
            />
          </label>

          {error ? (
            <div className="flex gap-2 rounded border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] px-3 py-2 text-sm text-[var(--scry-danger-text)]">
              <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" />
              <pre className="whitespace-pre-wrap font-sans">{error}</pre>
            </div>
          ) : null}

          {inspection ? (
            <div className="space-y-3 rounded border border-border p-3">
              <p className="text-sm font-medium">
                {t("settings.arrImportReview", { count: formats.length })}
              </p>
              {diagnostics.length ? (
                <ul className="list-disc space-y-1 pl-5 text-xs text-muted-foreground">
                  {diagnostics.map((diagnostic, index) => <li key={index}>{diagnostic}</li>)}
                </ul>
              ) : null}
              <div className="space-y-3">
                {formats.map((format, index) => {
                  const id = formatId(format, index);
                  const suggested = candidateScores(format);
                  const recommendation = recommendScore(format, inspection.diagnostics);
                  return (
                    <div key={id} className="rounded border border-border p-3">
                      <div className="flex flex-wrap items-center justify-between gap-3">
                        <div>
                          <p className="font-medium">{format.name || id}</p>
                        </div>
                        <label className="w-32">
                          <Label className="mb-1 block text-xs" htmlFor={`settings-rules-arr-score-${index}`}>
                            {t("settings.arrImportScore")}
                          </Label>
                          <Input
                            id={`settings-rules-arr-score-${index}`}
                            {...integerInputProps}
                            value={scores[id] ?? ""}
                            disabled={translating}
                            onChange={(event) =>
                              setScores((previous) => ({ ...previous, [id]: event.target.value }))
                            }
                          />
                        </label>
                      </div>
                      {recommendation ? (
                        <div className="mt-2 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                          <p>
                            {t("settings.arrImportSuggestedScore", { score: recommendation.value > 0 ? `+${recommendation.value}` : recommendation.value })}
                            {" "}{t(recommendation.reasonKey)}
                          </p>
                          <Button
                            type="button"
                            variant="secondary"
                            className="h-7 px-2 text-xs"
                            disabled={translating || scores[id] === String(recommendation.value)}
                            onClick={() => setScores((previous) => ({ ...previous, [id]: String(recommendation.value) }))}
                          >
                            {t("settings.arrImportUseSuggestion")}
                          </Button>
                        </div>
                      ) : null}
                      {suggested.length > 1 ? (
                        <div className="mt-2 flex flex-wrap items-center gap-1.5">
                          <span className="text-xs text-muted-foreground">
                            {t("settings.arrImportAmbiguousScore")}
                          </span>
                          {Object.entries(format.suggestedScores).filter(([, value]) => suggested.includes(value)).map(([label, value]) => (
                            <Button
                              key={label}
                              type="button"
                              variant="secondary"
                              className="h-7 px-2 text-xs"
                              disabled={translating}
                              onClick={() => setScores((previous) => ({ ...previous, [id]: String(value) }))}
                            >
                              {label}: {value}
                            </Button>
                          ))}
                        </div>
                      ) : null}
                    </div>
                  );
                })}
              </div>
            </div>
          ) : null}
        </div>

        <DialogFooter>
          <Button type="button" variant="secondary" onClick={close}>
            {t("label.cancel")}
          </Button>
          {inspection ? (
            <Button type="button" disabled={translating} onClick={translate}>
              {translating ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
              {t("settings.arrImportTranslate")}
            </Button>
          ) : (
            <Button type="button" disabled={reviewing || !json.trim()} onClick={() => void review()}>
              {reviewing ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
              {t("settings.arrImportReviewAction")}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

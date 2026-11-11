import * as React from "react";
import { AlertCircle } from "lucide-react";
import { productLogoUrl } from "@/components/setup/import/import-instance-pill";
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
import { useClient } from "urql";
import { useTranslate } from "@/lib/context/translate-context";
import { translateArrCustomFormatsMutation } from "@/lib/graphql/mutations";
import { makeAbortableFetch } from "@/lib/graphql/urql-client";
import {
  type ArrCustomFormatTranslation,
  type ArrDiagnostic,
  type ArrSource,
  type ReviewedCustomFormat,
  arrImportErrorMessage,
  candidateScores,
  collectFormatScores,
  defaultScore,
  isCurrentTranslationRequest,
  recommendScore,
  reviewedFormat,
  sourceFacets,
} from "@/lib/utils/arr-custom-format-import-state";
import { LoadingMark } from "@/components/common/loading-mark";

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

type Review = {
  formats: ReviewedCustomFormat[];
  diagnostics: ArrDiagnostic[];
};

function diagnosticText(diagnostic: ArrDiagnostic): string {
  return diagnostic.message;
}

export default function ArrCustomFormatImportDialog({
  open,
  onOpenChange,
  onApply,
}: ArrCustomFormatImportDialogProps) {
  const t = useTranslate();
  const client = useClient();
  const [source, setSource] = React.useState<ArrSource>("sonarr");
  const [json, setJson] = React.useState("");
  const [inspection, setInspection] = React.useState<Review | null>(null);
  const [scores, setScores] = React.useState<Record<string, string>>({});
  const [reviewing, setReviewing] = React.useState(false);
  const [translating, setTranslating] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const requestId = React.useRef(0);
  const pending = React.useRef<AbortController | null>(null);

  const cancelRequest = React.useCallback(() => {
    requestId.current += 1;
    pending.current?.abort();
    pending.current = null;
    setReviewing(false);
    setTranslating(false);
  }, []);

  React.useEffect(() => () => pending.current?.abort(), []);

  const close = React.useCallback(() => {
    cancelRequest();
    onOpenChange(false);
  }, [cancelRequest, onOpenChange]);

  const resetReview = React.useCallback(
    (nextJson?: string, nextSource?: ArrSource) => {
      cancelRequest();
      setInspection(null);
      setScores({});
      setError(null);
      if (nextJson !== undefined) setJson(nextJson);
      if (nextSource !== undefined) setSource(nextSource);
    },
    [cancelRequest],
  );

  /**
   * Sends the paste to the server. Resolves null after a failure, which it
   * reports, or once a newer request, a cancellation or new input replaced it.
   */
  const requestTranslation = React.useCallback(
    async (
      scoreValues: { formatId: string; score: number }[],
      fallback: string,
      setBusy: (busy: boolean) => void,
    ): Promise<ArrCustomFormatTranslation | null> => {
      cancelRequest();
      const thisRequest = requestId.current;
      const abort = new AbortController();
      pending.current = abort;
      setError(null);
      setBusy(true);
      try {
        const { data, error: failure } = await client
          .mutation(
            translateArrCustomFormatsMutation,
            {
              input: {
                sourceApplication: source === "radarr" ? "RADARR" : "SONARR",
                customFormatJson: json,
                scores: scoreValues,
              },
            },
            { fetch: makeAbortableFetch(abort.signal) },
          )
          .toPromise();
        if (!isCurrentTranslationRequest(requestId.current, thisRequest))
          return null;
        setBusy(false);
        const result = data?.translateArrCustomFormats as
          | ArrCustomFormatTranslation
          | undefined;
        if (failure || !result) {
          setError(arrImportErrorMessage(failure, t, fallback));
          return null;
        }
        return result;
      } finally {
        if (pending.current === abort) pending.current = null;
      }
    },
    [cancelRequest, client, json, source, t],
  );

  const review = React.useCallback(async () => {
    const translation = await requestTranslation(
      [],
      t("settings.arrImportReviewFailed"),
      setReviewing,
    );
    if (!translation) return;
    if (!translation.formats.length) {
      const diagnostics = translation.inspectionDiagnostics
        .map(diagnosticText)
        .filter(Boolean);
      setError(diagnostics.join("\n") || t("settings.arrImportNoFormats"));
      return;
    }
    const formats = translation.formats.map(reviewedFormat);
    const diagnostics = translation.inspectionDiagnostics;
    setInspection({ formats, diagnostics });
    setScores(
      Object.fromEntries(
        formats.map((format) => [format.id, defaultScore(format, diagnostics)]),
      ),
    );
  }, [requestTranslation, t]);

  const translate = React.useCallback(async () => {
    if (!inspection?.formats.length) return;
    const scoreResult = collectFormatScores(inspection.formats, scores);
    if ("missingFormat" in scoreResult) {
      setError(
        t("settings.arrImportScoreRequired", {
          name: scoreResult.missingFormat.name,
        }),
      );
      return;
    }
    const translation = await requestTranslation(
      scoreResult.scores,
      t("settings.arrImportTranslationFailed"),
      setTranslating,
    );
    if (!translation) return;
    const translated = translation.formats.filter(
      (format) => format.status === "TRANSLATED",
    ).length;
    const diagnostics = [
      t("settings.arrImportResultSummary", {
        translated,
        disabled: translation.formats.length - translated,
      }),
      ...translation.diagnostics.map(diagnosticText),
      ...translation.formats.flatMap((format) =>
        format.diagnostics.map(diagnosticText),
      ),
    ].filter(Boolean);
    onApply({
      name: translation.name || t("settings.arrImportDraftName"),
      description:
        translation.description || t("settings.arrImportDraftDescription"),
      regoSource: translation.regoSource,
      appliedFacets: translation.appliedFacets ?? sourceFacets(source),
      translationDiagnostics: [...new Set(diagnostics)],
    });
    onOpenChange(false);
  }, [inspection, onApply, onOpenChange, requestTranslation, scores, source, t]);

  const formats = inspection?.formats ?? [];
  const diagnostics =
    inspection?.diagnostics.map(diagnosticText).filter(Boolean) ?? [];

  return (
    <Dialog
      open={open}
      onOpenChange={(nextOpen) => (nextOpen ? onOpenChange(true) : close())}
    >
      <DialogContent
        id="settings-rules-arr-custom-format-import-dialog"
        className="max-h-[90vh] overflow-y-auto sm:max-w-2xl"
        onInteractOutside={(event) => translating && event.preventDefault()}
      >
        <DialogHeader>
          <DialogTitle>{t("settings.arrImportTitle")}</DialogTitle>
          <DialogDescription>
            {t("settings.arrImportDescription")}
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          <fieldset>
            <Label className="mb-2 block">
              {t("settings.arrImportSource")}
            </Label>
            <div className="flex gap-2">
              {(["sonarr", "radarr"] as const).map((option) => (
                <Button
                  key={option}
                  id={`settings-rules-arr-source-${option}`}
                  type="button"
                  variant={source === option ? "default" : "secondary"}
                  disabled={reviewing || translating}
                  onClick={() => resetReview(undefined, option)}
                >
                  <img
                    src={
                      productLogoUrl(
                        option === "sonarr" ? "SONARR" : "RADARR",
                      ) ?? undefined
                    }
                    alt=""
                    aria-hidden
                    className="h-4 w-4 shrink-0 object-contain"
                  />
                  {option === "sonarr" ? "Sonarr" : "Radarr"}
                </Button>
              ))}
            </div>
          </fieldset>

          <label className="block">
            <Label
              className="mb-2 block"
              htmlFor="settings-rules-arr-custom-format-json"
            >
              {t("settings.arrImportJson")}
            </Label>
            <Textarea
              id="settings-rules-arr-custom-format-json"
              className="bg-field font-[family-name:var(--font-code)] text-foreground"
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
              <div className="whitespace-pre-wrap">{error}</div>
            </div>
          ) : null}

          {inspection ? (
            <div className="space-y-3 rounded border border-border p-3">
              <p className="text-sm font-medium">
                {t("settings.arrImportReview", { count: formats.length })}
              </p>
              {diagnostics.length ? (
                <ul className="list-disc space-y-1 pl-5 text-xs text-muted-foreground">
                  {diagnostics.map((diagnostic, index) => (
                    <li key={index}>{diagnostic}</li>
                  ))}
                </ul>
              ) : null}
              <div className="space-y-3">
                {formats.map((format, index) => {
                  const id = format.id;
                  const suggested = candidateScores(format);
                  const recommendation = recommendScore(
                    format,
                    inspection.diagnostics,
                  );
                  return (
                    <div key={id} className="rounded border border-border p-3">
                      <div className="flex flex-wrap items-center justify-between gap-3">
                        <div>
                          <p className="font-medium">{format.name || id}</p>
                        </div>
                        <label className="w-32">
                          <Label
                            className="mb-1 block text-xs"
                            htmlFor={`settings-rules-arr-score-${index}`}
                          >
                            {t("settings.arrImportScore")}
                          </Label>
                          <Input
                            id={`settings-rules-arr-score-${index}`}
                            {...integerInputProps}
                            value={scores[id] ?? ""}
                            disabled={translating}
                            onChange={(event) =>
                              setScores((previous) => ({
                                ...previous,
                                [id]: event.target.value,
                              }))
                            }
                          />
                        </label>
                      </div>
                      {recommendation ? (
                        <div className="mt-2 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                          <p>
                            {t("settings.arrImportSuggestedScore", {
                              score:
                                recommendation.value > 0
                                  ? `+${recommendation.value}`
                                  : recommendation.value,
                            })}{" "}
                            {t(recommendation.reasonKey)}
                          </p>
                          <Button
                            type="button"
                            variant="secondary"
                            className="h-7 px-2 text-xs"
                            disabled={
                              translating ||
                              scores[id] === String(recommendation.value)
                            }
                            onClick={() =>
                              setScores((previous) => ({
                                ...previous,
                                [id]: String(recommendation.value),
                              }))
                            }
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
                          {Object.entries(format.suggestedScores)
                            .filter(([, value]) => suggested.includes(value))
                            .map(([label, value]) => (
                              <Button
                                key={label}
                                type="button"
                                variant="secondary"
                                className="h-7 px-2 text-xs"
                                disabled={translating}
                                onClick={() =>
                                  setScores((previous) => ({
                                    ...previous,
                                    [id]: String(value),
                                  }))
                                }
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
            <Button
              id="settings-rules-arr-translate"
              type="button"
              disabled={translating}
              onClick={() => void translate()}
            >
              {translating ? <LoadingMark className="mr-2 h-4 w-4" /> : null}
              {t("settings.arrImportTranslate")}
            </Button>
          ) : (
            <Button
              type="button"
              id="settings-rules-arr-review"
              disabled={reviewing || !json.trim()}
              onClick={() => void review()}
            >
              {reviewing ? <LoadingMark className="mr-2 h-4 w-4" /> : null}
              {t("settings.arrImportReviewAction")}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

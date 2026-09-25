import { Check, ClipboardCheck } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  SetupBackButton,
  SetupPanel,
  SetupPrimaryButton,
  SetupStepHeader,
} from "./setup-chrome";
import { Card, CardContent } from "@/components/ui/card";
import { Progress } from "@/components/ui/progress";
import type {
  ExternalImportMonitorWarmupPhaseProgress,
  ExternalImportMonitorWarmupProgress,
} from "@/lib/types/external-import";
import { SCORING_PERSONA_CHOICES } from "@/lib/constants/quality-profiles";
import type { FacetQualityPrefs, ViewCategoryId } from "@/lib/types/quality-profiles";

interface SummaryItem {
  label: string;
  value: string;
  code?: boolean;
}

interface SetupSummaryViewProps {
  t: (key: string) => string;
  facetPrefs: Record<ViewCategoryId, FacetQualityPrefs>;
  moviesPaths: string[];
  seriesPaths: string[];
  animePaths?: string[];
  downloadClientName: string;
  indexerName: string;
  importedDcCount?: number;
  importedIdxCount?: number;
  monitorWarmupProgress?: ExternalImportMonitorWarmupProgress | null;
  monitorWarmupError?: string | null;
  onFinish?: () => void;
  onImportOnly?: () => void;
  onImportAndScan?: () => void;
  onBack: () => void;
  finishing: boolean;
  finishingAction?: "finish" | "importOnly" | "importAndScan" | null;
}

function formatFacetPrefs(
  facetPrefs: Record<ViewCategoryId, FacetQualityPrefs>,
  t: (key: string) => string,
): string {
  const FACET_LABELS: Record<ViewCategoryId, string> = {
    MOVIE: t("setup.facetMovies"),
    SERIES: t("setup.facetSeries"),
    ANIME: t("setup.facetAnime"),
  };
  return (["MOVIE", "SERIES", "ANIME"] as ViewCategoryId[])
    .map((facet) => {
      const p = facetPrefs[facet];
      const quality = formatQualityTarget(p.quality);
      const personaChoice = SCORING_PERSONA_CHOICES.find((choice) => choice.value === p.persona);
      const persona = personaChoice ? t(personaChoice.labelKey) : p.persona;
      return `${FACET_LABELS[facet]}: ${quality} ${persona}`;
    })
    .join(", ");
}

function formatQualityTarget(target: FacetQualityPrefs["quality"]): string {
  switch (target) {
    case "8k":
      return "8K";
    case "4k":
      return "4K";
    case "1080p":
      return "1080P";
  }
}

function activeWarmupPhaseState(
  progress: ExternalImportMonitorWarmupProgress,
  t: (key: string) => string,
): {
  label: string;
  totalKnown: boolean;
  phaseProgress: ExternalImportMonitorWarmupPhaseProgress;
} {
  switch (progress.phase) {
    // Prowlarr discovery sessions report this phase; the monitor warmup this
    // view renders never enters it, but the union must stay exhaustive.
    case "LOADING_INDEXERS":
      return {
        label: t("setup.monitorWarmupLoadingIndexers"),
        totalKnown: progress.overallTotalKnown,
        phaseProgress: progress.overallProgress,
      };
    case "LOADING_MOVIES":
      return {
        label: t("setup.monitorWarmupLoadingMovies"),
        totalKnown: progress.moviesTotalKnown,
        phaseProgress: progress.moviesProgress,
      };
    case "LOADING_SERIES":
      return {
        label: t("setup.monitorWarmupLoadingSeries"),
        totalKnown: progress.seriesTotalKnown,
        phaseProgress: progress.seriesProgress,
      };
    case "LOADING_EPISODES":
      return {
        label: t("setup.monitorWarmupLoadingEpisodes"),
        totalKnown: progress.episodeFetchTotalKnown,
        phaseProgress: progress.episodeFetchProgress,
      };
    case "BUILDING_SNAPSHOT":
      return {
        label: t("setup.monitorWarmupBuildingSnapshot"),
        totalKnown: progress.snapshotBuildTotalKnown,
        phaseProgress: progress.snapshotBuildProgress,
      };
    case "READY":
      return {
        label: t("setup.monitorWarmupReady"),
        totalKnown: progress.overallTotalKnown,
        phaseProgress: progress.overallProgress,
      };
  }
}

export function SetupSummaryView({
  t,
  facetPrefs,
  moviesPaths,
  seriesPaths,
  animePaths,
  downloadClientName,
  indexerName,
  importedDcCount,
  importedIdxCount,
  monitorWarmupProgress,
  monitorWarmupError,
  onFinish,
  onImportOnly,
  onImportAndScan,
  onBack,
  finishing,
  finishingAction,
}: SetupSummaryViewProps) {
  const isImportPath = importedDcCount !== undefined || importedIdxCount !== undefined;
  const mediaPathsSummary = [...moviesPaths, ...seriesPaths, ...(animePaths ?? [])].join(", ");
  const warmupPhaseState = monitorWarmupProgress
    ? activeWarmupPhaseState(monitorWarmupProgress, t)
    : null;
  const showWarmupCard = Boolean(
    isImportPath &&
      (monitorWarmupError ||
        (monitorWarmupProgress && monitorWarmupProgress.status !== "COMPLETED")),
  );
  const warmupPercent =
    warmupPhaseState &&
    warmupPhaseState.totalKnown &&
    warmupPhaseState.phaseProgress.total > 0
      ? Math.round(
          (warmupPhaseState.phaseProgress.completed / warmupPhaseState.phaseProgress.total) * 100,
        )
      : null;

  const items: SummaryItem[] = [
    { label: t("setup.summaryPersona"), value: formatFacetPrefs(facetPrefs, t) },
    { label: t("setup.summaryMediaPaths"), value: mediaPathsSummary, code: true },
  ];

  if (isImportPath) {
    if (importedDcCount !== undefined && importedDcCount > 0) {
      items.push({
        label: t("setup.summaryDownloadClient"),
        value: `${importedDcCount} ${t("setup.summaryImportedClients")}`,
      });
    }
    if (importedIdxCount !== undefined && importedIdxCount > 0) {
      items.push({
        label: t("setup.summaryIndexer"),
        value: `${importedIdxCount} ${t("setup.summaryImportedIndexers")}`,
      });
    }
  } else {
    items.push({ label: t("setup.summaryDownloadClient"), value: downloadClientName });
    items.push({ label: t("setup.summaryIndexer"), value: indexerName });
  }

  return (
    <SetupPanel className="flex flex-col gap-6">
      <SetupStepHeader
        icon={ClipboardCheck}
        title={t("setup.summaryTitle")}
        subtitle={t("setup.summaryDescription")}
      />
      <Card className="mx-auto w-full max-w-md">
        <CardContent className="flex flex-col gap-3 p-5">
          {items.map((item) => (
            <div key={item.label} className="flex items-start gap-3">
              <Check className="mt-0.5 h-4 w-4 flex-none text-[var(--scry-success-text-soft)]" />
              <div>
                <p className="text-sm font-medium">{item.label}</p>
                <p className={item.code ? "font-[var(--font-code)] text-sm text-muted-foreground" : "text-sm text-muted-foreground"}>
                  {item.value}
                </p>
              </div>
            </div>
          ))}
        </CardContent>
      </Card>
      {showWarmupCard ? (
        <Card className="mx-auto w-full max-w-md border-[var(--scry-success-border)]">
          <CardContent className="flex flex-col gap-3 p-5">
            <div className="space-y-1">
              <p className="text-sm font-medium">{t("setup.monitorWarmupTitle")}</p>
              <p className="text-sm text-muted-foreground">
                {monitorWarmupError
                  ? t("setup.monitorWarmupFailed")
                  : monitorWarmupProgress?.status === "FAILED"
                  ? t("setup.monitorWarmupFailed")
                  : monitorWarmupProgress?.status === "CANCELED"
                    ? t("setup.monitorWarmupCanceled")
                    : t("setup.monitorWarmupDescription")}
              </p>
            </div>
            {monitorWarmupProgress && warmupPhaseState ? (
              <div className="space-y-2">
                <div className="flex items-center justify-between gap-3 text-sm">
                  <span className="font-medium">{warmupPhaseState.label}</span>
                  {warmupPercent !== null ? <span>{warmupPercent}%</span> : null}
                </div>
                <Progress
                  value={warmupPercent ?? undefined}
                  indeterminate={warmupPercent === null}
                  indicatorClassName="bg-[var(--scry-success-solid)]"
                />
                <p className="text-xs text-muted-foreground">
                  {warmupPhaseState.totalKnown
                    ? `${warmupPhaseState.phaseProgress.completed} / ${warmupPhaseState.phaseProgress.total}`
                    : t("setup.monitorWarmupQueued")}
                </p>
                {monitorWarmupProgress.errorMessage ? (
                  <p className="text-xs text-destructive">
                    {monitorWarmupProgress.errorMessage}
                  </p>
                ) : null}
              </div>
            ) : monitorWarmupError ? (
              <p className="text-xs text-destructive">{monitorWarmupError}</p>
            ) : null}
          </CardContent>
        </Card>
      ) : null}
      <div id="setup-summary-view" className="flex justify-between pt-2">
        <SetupBackButton id="setup-summary-back" onClick={onBack}>
          {t("setup.back")}
        </SetupBackButton>
        {isImportPath && onImportOnly && onImportAndScan ? (
          <div className="flex items-center gap-2">
            <Button
              id="setup-summary-import-only"
              variant="outline"
              onClick={onImportOnly}
              disabled={finishing}
            >
              {finishingAction === "importOnly"
                ? t("setup.importing")
                : t("setup.importOnly")}
            </Button>
            <Button id="setup-summary-import-and-scan" onClick={onImportAndScan} disabled={finishing}>
              {finishingAction === "importAndScan"
                ? t("setup.importing")
                : t("setup.importAndScan")}
            </Button>
          </div>
        ) : (
          <SetupPrimaryButton id="setup-summary-finish" onClick={onFinish} disabled={finishing}>
            {finishing ? t("label.saving") : t("setup.finish")}
          </SetupPrimaryButton>
        )}
      </div>
    </SetupPanel>
  );
}

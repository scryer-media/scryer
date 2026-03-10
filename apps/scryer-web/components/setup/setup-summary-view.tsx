import { Check } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import type { FacetQualityPrefs, ViewCategoryId } from "@/lib/types/quality-profiles";

interface SummaryItem {
  label: string;
  value: string;
}

interface SetupSummaryViewProps {
  t: (key: string) => string;
  facetPrefs: Record<ViewCategoryId, FacetQualityPrefs>;
  moviesPath: string;
  seriesPath: string;
  animePath?: string;
  downloadClientName: string;
  indexerName: string;
  importedDcCount?: number;
  importedIdxCount?: number;
  onFinish: () => void;
  onBack: () => void;
  finishing: boolean;
}

function formatFacetPrefs(
  facetPrefs: Record<ViewCategoryId, FacetQualityPrefs>,
  t: (key: string) => string,
): string {
  const FACET_LABELS: Record<ViewCategoryId, string> = {
    movie: t("setup.facetMovies"),
    series: t("setup.facetSeries"),
    anime: t("setup.facetAnime"),
  };
  return (["movie", "series", "anime"] as ViewCategoryId[])
    .map((facet) => {
      const p = facetPrefs[facet];
      const quality = p.quality === "4k" ? "4K" : "1080P";
      const persona = t(`qualityProfile.persona${p.persona}`);
      return `${FACET_LABELS[facet]}: ${quality} ${persona}`;
    })
    .join(", ");
}

export function SetupSummaryView({
  t,
  facetPrefs,
  moviesPath,
  seriesPath,
  animePath,
  downloadClientName,
  indexerName,
  importedDcCount,
  importedIdxCount,
  onFinish,
  onBack,
  finishing,
}: SetupSummaryViewProps) {
  const isImportPath = importedDcCount !== undefined || importedIdxCount !== undefined;

  const items: SummaryItem[] = [
    { label: t("setup.summaryPersona"), value: formatFacetPrefs(facetPrefs, t) },
    { label: t("setup.summaryMediaPaths"), value: [moviesPath, seriesPath, animePath].filter(Boolean).join(", ") },
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
    <div className="flex flex-col gap-6">
      <div className="text-center">
        <h2 className="text-xl font-semibold">{t("setup.summaryTitle")}</h2>
        <p className="mt-1 text-sm text-muted-foreground">{t("setup.summaryDescription")}</p>
      </div>
      <Card className="mx-auto w-full max-w-md">
        <CardContent className="flex flex-col gap-3 p-5">
          {items.map((item) => (
            <div key={item.label} className="flex items-start gap-3">
              <Check className="mt-0.5 h-4 w-4 flex-none text-emerald-500" />
              <div>
                <p className="text-sm font-medium">{item.label}</p>
                <p className="text-sm text-muted-foreground">{item.value}</p>
              </div>
            </div>
          ))}
        </CardContent>
      </Card>
      <div className="flex justify-between pt-2">
        <Button variant="ghost" onClick={onBack}>{t("setup.back")}</Button>
        <Button onClick={onFinish} disabled={finishing}>
          {finishing ? t("setup.saving") : t("setup.finish")}
        </Button>
      </div>
    </div>
  );
}

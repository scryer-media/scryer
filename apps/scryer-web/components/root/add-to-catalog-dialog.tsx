import * as React from "react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Checkbox } from "@/components/ui/checkbox";
import { useTranslate } from "@/lib/context/translate-context";
import { defaultMonitorTypeForFacet, sectionLabelForFacet } from "@/lib/facets/helpers";
import { selectPosterVariantUrl } from "@/lib/utils/poster-images";
import { TitlePoster } from "@/components/title-poster";
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import type { Facet } from "@/lib/types";
import type {
  CatalogQualityProfileOption,
  MetadataCatalogAddOptions,
  MetadataCatalogMonitorType,
  RootFolderOption,
} from "@/lib/hooks/use-global-search";

type AddToCatalogDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  result: MetadataTvdbSearchItem;
  facet: Facet;
  catalogQualityProfileOptions: CatalogQualityProfileOption[];
  defaultQualityProfileId: string;
  rootFolders: RootFolderOption[];
  onSubmit: (
    result: MetadataTvdbSearchItem,
    facet: Facet,
    options: MetadataCatalogAddOptions,
  ) => Promise<string | null>;
};

/** Sentinel used by callers when the dialog is closed so they don't need to pass null. */
export const EMPTY_SEARCH_RESULT: MetadataTvdbSearchItem = {
  tvdbId: "",
  name: "",
  imdbId: null,
  slug: null,
  type: null,
  year: null,
  status: null,
  overview: null,
  popularity: null,
  posterUrl: null,
  language: null,
  runtimeMinutes: null,
  sortTitle: null,
};

function buildDefaultDraft(
  facet: Facet,
  defaultQualityProfileId: string,
): MetadataCatalogAddOptions {
  return {
    qualityProfileId: defaultQualityProfileId,
    seasonFolder: facet !== "movie",
    monitorType: defaultMonitorTypeForFacet(facet),
    ...(facet === "movie" ? { minAvailability: "announced" } : {}),
    ...(facet === "anime"
      ? {
          monitorSpecials: false,
          interSeasonMovies: true,
        }
      : {}),
  };
}

export function AddToCatalogDialog({
  open,
  onOpenChange,
  result,
  facet,
  catalogQualityProfileOptions,
  defaultQualityProfileId,
  rootFolders,
  onSubmit,
}: AddToCatalogDialogProps) {
  const t = useTranslate();
  const [draft, setDraft] = React.useState<MetadataCatalogAddOptions>(() =>
    buildDefaultDraft(facet, defaultQualityProfileId),
  );
  const [isSubmitting, setIsSubmitting] = React.useState(false);

  // Reset draft when dialog opens
  React.useEffect(() => {
    if (!open) return;
    setDraft(buildDefaultDraft(facet, defaultQualityProfileId));
    setIsSubmitting(false);
  }, [open, facet, defaultQualityProfileId]);

  const qualityProfileValue =
    draft.qualityProfileId || defaultQualityProfileId;

  const handleSubmit = React.useCallback(async () => {
    const qpId = (draft.qualityProfileId || defaultQualityProfileId).trim();
    if (!qpId) return;

    setIsSubmitting(true);
    try {
      const titleId = await onSubmit(result, facet, { ...draft, qualityProfileId: qpId });
      if (titleId) {
        onOpenChange(false);
      }
    } finally {
      setIsSubmitting(false);
    }
  }, [draft, defaultQualityProfileId, onSubmit, result, facet, onOpenChange]);

  const update = React.useCallback(
    (patch: Partial<MetadataCatalogAddOptions>) => {
      setDraft((prev) => ({ ...prev, ...patch }));
    },
    [],
  );

  const monitorOptions: Array<{ value: MetadataCatalogMonitorType; label: string }> =
    facet === "movie"
      ? [
          { value: "monitored", label: t("search.monitorType.monitored") },
          { value: "unmonitored", label: t("search.monitorType.unmonitored") },
        ]
      : [
          { value: "futureEpisodes", label: t("search.monitorType.futureEpisodes") },
          {
            value: "missingAndFutureEpisodes",
            label: t("search.monitorType.missingAndFutureEpisodes"),
          },
          { value: "allEpisodes", label: t("search.monitorType.allEpisodes") },
          { value: "none", label: t("search.monitorType.none") },
        ];

  const posterUrl = selectPosterVariantUrl(result.posterUrl, "w70");

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <div className="flex gap-3">
            <div className="h-20 w-14 flex-none overflow-hidden rounded-md border border-border bg-muted">
              {posterUrl ? (
                <TitlePoster
                  src={posterUrl}
                  alt={t("media.posterAlt", { name: result.name })}
                  className="h-full w-full object-cover"
                />
              ) : (
                <div className="flex h-full w-full items-center justify-center text-xs text-muted-foreground">
                  {t("label.noArt")}
                </div>
              )}
            </div>
            <div className="min-w-0">
              <DialogTitle className="text-base">{result.name}</DialogTitle>
              <DialogDescription>
                {sectionLabelForFacet(t, facet)}
                {result.year ? ` \u2022 ${result.year}` : ""}
                {result.slug ? ` \u2022 ${result.slug}` : ""}
              </DialogDescription>
            </div>
          </div>
          {result.overview ? (
            <p className="mt-2 text-xs text-muted-foreground line-clamp-3">
              {result.overview}
            </p>
          ) : null}
        </DialogHeader>

        <div className="grid gap-3 sm:grid-cols-2">
          {/* Quality Profile — all facets */}
          <label className="space-y-1">
            <span className="block text-xs font-medium text-card-foreground">
              {t("search.addConfigQualityProfile")}
            </span>
            <Select
              value={catalogQualityProfileOptions.length > 0 ? qualityProfileValue : ""}
              onValueChange={(v) => update({ qualityProfileId: v })}
              disabled={isSubmitting || catalogQualityProfileOptions.length === 0}
            >
              <SelectTrigger className="h-9 w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {catalogQualityProfileOptions.length === 0 ? (
                  <SelectItem value="__none" disabled>
                    {t("search.addConfigNoQualityProfiles")}
                  </SelectItem>
                ) : (
                  catalogQualityProfileOptions.map((profile) => (
                    <SelectItem key={profile.id} value={profile.id}>
                      {profile.name}
                    </SelectItem>
                  ))
                )}
              </SelectContent>
            </Select>
          </label>

          {/* Root Folder */}
          {rootFolders.length >= 1 ? (
            <label className="space-y-1">
              <span className="block text-xs font-medium text-card-foreground">
                {t("search.addConfigRootFolder")}
              </span>
              <Select
                value={
                  draft.rootFolder ||
                  rootFolders.find((rf) => rf.isDefault)?.path ||
                  rootFolders[0]?.path ||
                  ""
                }
                onValueChange={(v) => update({ rootFolder: v })}
                disabled={isSubmitting}
              >
                <SelectTrigger className="h-9 w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {rootFolders.map((rf) => (
                    <SelectItem key={rf.path} value={rf.path}>
                      {rf.path}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </label>
          ) : null}

          {/* Season Folder — tv + anime */}
          {facet !== "movie" ? (
            <label className="space-y-1">
              <span className="block text-xs font-medium text-card-foreground">
                {t("search.addConfigSeasonFolder")}
              </span>
              <Select
                value={draft.seasonFolder ? "enabled" : "disabled"}
                onValueChange={(v) => update({ seasonFolder: v === "enabled" })}
                disabled={isSubmitting}
              >
                <SelectTrigger className="h-9 w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="enabled">{t("search.seasonFolder.enabled")}</SelectItem>
                  <SelectItem value="disabled">{t("search.seasonFolder.disabled")}</SelectItem>
                </SelectContent>
              </Select>
            </label>
          ) : null}

          {/* Monitored checkbox — movie only */}
          {facet === "movie" ? (
            <label className="flex items-center gap-2 sm:col-span-2">
              <Checkbox
                checked={draft.monitorType === "monitored"}
                onCheckedChange={(v) =>
                  update({ monitorType: v === true ? "monitored" : "unmonitored" })
                }
                disabled={isSubmitting}
              />
              <span className="text-sm text-card-foreground">
                {t("title.monitored")}
              </span>
            </label>
          ) : (
            /* Monitor Type — tv + anime */
            <label className="space-y-1">
              <span className="block text-xs font-medium text-card-foreground">
                {t("search.addConfigMonitorType")}
              </span>
              <Select
                value={draft.monitorType}
                onValueChange={(v) =>
                  update({ monitorType: v as MetadataCatalogMonitorType })
                }
                disabled={isSubmitting}
              >
                <SelectTrigger className="h-9 w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {monitorOptions.map((option) => (
                    <SelectItem key={option.value} value={option.value}>
                      {option.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </label>
          )}
        </div>

        <DialogFooter>
          <Button
            type="button"
            variant="outline"
            onClick={() => onOpenChange(false)}
            disabled={isSubmitting}
          >
            {t("label.cancel")}
          </Button>
          <Button
            type="button"
            onClick={() => void handleSubmit()}
            disabled={isSubmitting || !qualityProfileValue}
            className="bg-emerald-600 text-foreground hover:bg-emerald-500"
          >
            {isSubmitting ? t("search.adding") : t("title.addToCatalog")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

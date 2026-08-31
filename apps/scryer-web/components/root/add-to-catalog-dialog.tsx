import * as React from "react";
import { Loader2, Plus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogFooter } from "@/components/ui/dialog";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { useTranslate } from "@/lib/context/translate-context";
import { defaultMonitorTypeForFacet } from "@/lib/facets/helpers";
import { CatalogActionDialogSummary } from "@/components/root/catalog-action-dialog-summary";
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import type { Facet } from "@/lib/types";
import type {
  CatalogQualityProfileOption,
  MetadataCatalogAddOptions,
  MetadataCatalogMonitorType,
} from "@/lib/hooks/use-global-search";
import type { LibraryRecord, RootFolderOption } from "@/lib/types/titles";
import {
  canSubmitCatalogAdd,
  catalogAddDraftResetKey,
  catalogAddOptionsForSubmit,
  catalogQualityProfileSelectValue,
  defaultCatalogRootFolderId,
  draftForCatalogLibrary,
  inheritedCatalogQualityProfileLabel,
  INHERIT_CATALOG_QUALITY_PROFILE_VALUE,
} from "@/lib/utils/catalog-add-quality-profile";

type AddToCatalogDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  result: MetadataTvdbSearchItem;
  facet: Facet;
  catalogQualityProfileOptions: CatalogQualityProfileOption[];
  catalogConfigLoading: boolean;
  defaultQualityProfileId: string;
  manageableLibraries: LibraryRecord[];
  rootFolderOptions: RootFolderOption[];
  onAdd: (
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
  defaultLibraryId?: string,
  defaultRootFolderId?: string,
): MetadataCatalogAddOptions {
  return {
    libraryId: defaultLibraryId,
    rootFolderId: defaultRootFolderId,
    seasonFolder: facet !== "MOVIE",
    monitorType: defaultMonitorTypeForFacet(facet),
    ...(facet === "MOVIE" ? { minAvailability: "announced" } : {}),
    ...(facet === "ANIME"
      ? {
          monitorSpecials: false,
          interSeasonMovies: true,
        }
      : {}),
  };
}

function defaultLibrary(libraries: LibraryRecord[]): LibraryRecord | null {
  return libraries.find((library) => library.isDefault) || libraries[0] || null;
}

function defaultRootFolderId(
  rootFolders: Array<{ id?: string; isDefault: boolean }>,
): string | undefined {
  return defaultCatalogRootFolderId(rootFolders);
}

export function AddToCatalogDialog({
  open,
  onOpenChange,
  result,
  facet,
  catalogQualityProfileOptions,
  catalogConfigLoading,
  defaultQualityProfileId,
  manageableLibraries,
  rootFolderOptions,
  onAdd,
}: AddToCatalogDialogProps) {
  const t = useTranslate();
  const libraries = manageableLibraries;
  const fallbackRootFolders = rootFolderOptions;
  const [draft, setDraft] = React.useState<MetadataCatalogAddOptions>(() =>
    buildDefaultDraft(
      facet,
      defaultLibrary(libraries)?.id,
      defaultRootFolderId(defaultLibrary(libraries)?.roots ?? fallbackRootFolders),
    ),
  );
  const [isSubmitting, setIsSubmitting] = React.useState(false);
  const draftResetKeyRef = React.useRef<string | null>(null);
  const nextDefaultLibrary = defaultLibrary(libraries);
  const nextDefaultLibraryId = nextDefaultLibrary?.id;
  const nextDefaultRootFolderId = defaultRootFolderId(
    nextDefaultLibrary?.roots ?? fallbackRootFolders,
  );
  const draftResetKey = catalogAddDraftResetKey(
    facet,
    String(result.smgId ?? result.tvdbId ?? result.name),
    nextDefaultLibraryId,
    nextDefaultRootFolderId,
  );

  // Reset draft when dialog opens
  React.useEffect(() => {
    if (!open) {
      draftResetKeyRef.current = null;
      return;
    }
    if (draftResetKeyRef.current === draftResetKey) {
      return;
    }
    draftResetKeyRef.current = draftResetKey;
    setDraft(
      buildDefaultDraft(
        facet,
        nextDefaultLibraryId,
        nextDefaultRootFolderId,
      ),
    );
    setIsSubmitting(false);
  }, [
    draftResetKey,
    facet,
    nextDefaultLibraryId,
    nextDefaultRootFolderId,
    open,
  ]);

  const selectedLibrary =
    libraries.find((library) => library.id === draft.libraryId) ||
    defaultLibrary(libraries) ||
    null;
  const qualityProfileValue = catalogQualityProfileSelectValue(draft.qualityProfileId);
  const inheritedQualityProfileLabel = inheritedCatalogQualityProfileLabel(
    selectedLibrary,
    defaultQualityProfileId,
    catalogQualityProfileOptions,
    t("search.addConfigInheritLibrary"),
  );
  const selectedRootFolders = selectedLibrary?.roots ?? fallbackRootFolders;
  const selectableRootFolders = selectedRootFolders.flatMap((rootFolder) => {
    const id = rootFolder.id?.trim();
    return id ? [{ ...rootFolder, id }] : [];
  });
  const draftRootFolderId = draft.rootFolderId?.trim();
  const effectiveRootFolderId =
    draftRootFolderId &&
    selectableRootFolders.some((rootFolder) => rootFolder.id === draftRootFolderId)
      ? draftRootFolderId
      : defaultRootFolderId(selectableRootFolders) || "";
  const libraryRequired = libraries.length > 0;
  const hasCatalogDestination =
    libraries.length > 0 || selectableRootFolders.length > 0;
  const qualityProfileSelectionDisabled =
    isSubmitting || catalogConfigLoading || catalogQualityProfileOptions.length === 0;
  const submitAllowed = canSubmitCatalogAdd({
    catalogConfigLoading,
    qualityProfileCount: catalogQualityProfileOptions.length,
    hasCatalogDestination,
    libraryRequired,
    hasSelectedLibrary: selectedLibrary !== null,
  });

  const handleSubmit = React.useCallback(async () => {
    const libraryId = selectedLibrary?.id?.trim();
    if (!submitAllowed || (libraryRequired && !libraryId)) return;

    setIsSubmitting(true);
    try {
      const titleId = await onAdd(result, facet, catalogAddOptionsForSubmit({
        ...draft,
        libraryId,
        rootFolderId: effectiveRootFolderId || undefined,
      }));
      if (titleId) {
        onOpenChange(false);
      }
    } finally {
      setIsSubmitting(false);
    }
  }, [
    draft,
    facet,
    libraryRequired,
    onAdd,
    onOpenChange,
    result,
    effectiveRootFolderId,
    selectedLibrary,
    submitAllowed,
  ]);

  const update = React.useCallback(
    (patch: Partial<MetadataCatalogAddOptions>) => {
      setDraft((prev) => ({ ...prev, ...patch }));
    },
    [],
  );

  const monitorOptions: Array<{ value: MetadataCatalogMonitorType; label: string }> =
    facet === "MOVIE"
      ? [
          { value: "MONITORED", label: t("search.monitorType.monitored") },
          { value: "UNMONITORED", label: t("search.monitorType.unmonitored") },
        ]
      : [
          { value: "FUTURE_EPISODES", label: t("search.monitorType.futureEpisodes") },
          {
            value: "MISSING_AND_FUTURE_EPISODES",
            label: t("search.monitorType.missingAndFutureEpisodes"),
          },
          { value: "ALL_EPISODES", label: t("search.monitorType.allEpisodes") },
          { value: "NONE", label: t("search.monitorType.none") },
        ];

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        id="add-to-catalog-dialog"
        className="max-h-[90vh] gap-0 overflow-y-auto p-0 sm:max-w-5xl"
      >
        <CatalogActionDialogSummary result={result} facet={facet} mode="add" />

        <div className="space-y-6 p-5 sm:p-7">
          <div className="grid gap-4 sm:grid-cols-2">
          {libraries.length >= 1 ? (
            <label className="space-y-1 sm:col-span-2">
              <span className="block text-xs font-medium text-card-foreground">
                {t("search.addConfigLibrary")}
              </span>
              <Select
                value={selectedLibrary?.id || ""}
                onValueChange={(v) => {
                  const library = libraries.find((candidate) => candidate.id === v);
                  setDraft((previous) =>
                    draftForCatalogLibrary(
                      previous,
                      v,
                      library?.roots ?? fallbackRootFolders,
                    ),
                  );
                }}
                disabled={isSubmitting || libraries.length === 1}
              >
                <SelectTrigger id="add-to-catalog-library" className="h-12 w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {libraries.map((library) => (
                    <SelectItem key={library.id} value={library.id}>
                      {library.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </label>
          ) : null}

          {(
            <label className="space-y-1">
              <span className="block text-xs font-medium text-card-foreground">
                {t("search.addConfigQualityProfile")}
              </span>
              <Select
                value={catalogQualityProfileOptions.length > 0 ? qualityProfileValue : ""}
                onValueChange={(v) =>
                  update({
                    qualityProfileId:
                      v === INHERIT_CATALOG_QUALITY_PROFILE_VALUE ? undefined : v,
                  })
                }
                disabled={qualityProfileSelectionDisabled}
              >
                <SelectTrigger
                  id="add-to-catalog-quality-profile"
                  className="h-12 w-full"
                  aria-busy={catalogConfigLoading}
                >
                  <SelectValue placeholder={catalogConfigLoading ? t("label.loading") : undefined} />
                </SelectTrigger>
                <SelectContent>
                  {catalogQualityProfileOptions.length === 0 ? (
                    <SelectItem value="__none" disabled>
                      {t("search.addConfigNoQualityProfiles")}
                    </SelectItem>
                  ) : (
                    <>
                      <SelectItem value={INHERIT_CATALOG_QUALITY_PROFILE_VALUE}>
                        {inheritedQualityProfileLabel}
                      </SelectItem>
                      {catalogQualityProfileOptions.map((profile) => (
                        <SelectItem key={profile.id} value={profile.id}>
                          {profile.name}
                        </SelectItem>
                      ))}
                    </>
                  )}
                </SelectContent>
              </Select>
            </label>
          )}

          {/* Root Folder */}
          {selectableRootFolders.length >= 1 ? (
            <label className="space-y-1">
              <span className="block text-xs font-medium text-card-foreground">
                {t("search.addConfigRootFolder")}
              </span>
              <Select
                value={effectiveRootFolderId}
                onValueChange={(v) => update({ rootFolderId: v })}
                disabled={isSubmitting}
              >
                <SelectTrigger
                  id="add-to-catalog-root-folder"
                  className="h-12 w-full font-[var(--font-code)]"
                >
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {selectableRootFolders.map((rf) => (
                    <SelectItem key={rf.id} value={rf.id}>
                      <span className="font-[var(--font-code)]">{rf.path}</span>
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </label>
          ) : null}

          {/* Season Folder — series + anime */}
          {facet !== "MOVIE" ? (
            <label className="space-y-1">
              <span className="block text-xs font-medium text-card-foreground">
                {t("search.addConfigSeasonFolder")}
              </span>
              <Select
                value={draft.seasonFolder ? "enabled" : "disabled"}
                onValueChange={(v) => update({ seasonFolder: v === "enabled" })}
                disabled={isSubmitting}
              >
                <SelectTrigger
                  id="add-to-catalog-season-folder"
                  className="h-12 w-full"
                >
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
          {facet === "MOVIE" ? (
            <label className="flex items-center gap-4 rounded-xl border border-primary/30 bg-primary/10 p-4 sm:col-span-2">
              <Switch
                id="add-to-catalog-monitored"
                checked={draft.monitorType === "MONITORED"}
                onCheckedChange={(v) =>
                  update({ monitorType: v === true ? "MONITORED" : "UNMONITORED" })
                }
                disabled={isSubmitting}
                size="lg"
              />
              <span className="min-w-0">
                <span className="block text-base font-semibold text-card-foreground">
                  {t("title.monitored")}
                </span>
                <span className="block text-sm text-muted-foreground">
                  {t("search.monitorType.monitored")}
                </span>
              </span>
            </label>
          ) : (
            /* Monitor Type — series + anime */
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
                <SelectTrigger
                  id="add-to-catalog-monitor-type"
                  className="h-12 w-full"
                >
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

        {catalogConfigLoading ? (
          <div
            id="add-to-catalog-config-loading"
            className="flex items-center gap-2 rounded-md border border-dashed border-border/80 bg-muted/30 px-3 py-2 text-sm text-muted-foreground"
          >
            <Loader2 className="h-4 w-4 animate-spin text-primary" />
            <span>{t("label.loading")}</span>
          </div>
        ) : null}

          <DialogFooter className="items-stretch gap-3 sm:items-center">
          <Button
            id="add-to-catalog-cancel"
            type="button"
            variant="outline"
            onClick={() => onOpenChange(false)}
            disabled={isSubmitting}
            className="h-12 px-8"
          >
            {t("label.cancel")}
          </Button>
          <Button
            id="add-to-catalog-submit"
            type="button"
            onClick={() => void handleSubmit()}
            disabled={
              isSubmitting ||
              !qualityProfileValue ||
              !submitAllowed
            }
            className="h-12 gap-2 bg-primary px-8 text-primary-foreground hover:bg-primary/90"
          >
            {isSubmitting ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              <Plus className="h-5 w-5" />
            )}
            {isSubmitting ? t("search.adding") : t("title.addToCatalog")}
          </Button>
          </DialogFooter>
        </div>
      </DialogContent>
    </Dialog>
  );
}

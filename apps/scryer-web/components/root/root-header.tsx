
import * as React from "react";
import { Eye, EyeOff, Loader2, Plus, Search, X } from "lucide-react";
import { useTheme } from "next-themes";
import ScryerLogo from "@/components/scryer-logo";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { RouteCommandPalette } from "@/components/common/route-command-palette";
import type { ViewId } from "@/components/root/types";
import { useTranslate } from "@/lib/context/translate-context";
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import type { Facet } from "@/lib/types";
import type {
  CatalogQualityProfileOption,
  MetadataCatalogAddOptions,
  MetadataCatalogMonitorType,
} from "@/lib/hooks/use-global-search";
import type { RouteCommandPaletteConfig } from "@/components/common/route-command-palette";
import { useIsMobile } from "@/lib/hooks/use-mobile";
import { MobileSearchOverlay } from "@/components/root/mobile-search-overlay";
import { FACET_REGISTRY } from "@/lib/facets/registry";
import {
  sectionLabelForFacet,
  viewFromFacet,
  defaultMonitorTypeForFacet,
} from "@/lib/facets/helpers";
import { selectPosterVariantUrl } from "@/lib/utils/poster-images";
import { useSearchContext } from "@/lib/context/search-context";
import { cn } from "@/lib/utils";


type RootHeaderProps = {
  routeCommandPalette?: RouteCommandPaletteConfig;
  onOpenOverview?: (targetView: ViewId, titleId: string) => void;
};

function catalogFacetFromString(facet: string): Facet {
  return facet === "movie" ? "movie" : facet === "anime" ? "anime" : "tv";
}

function renderMetadataResultKey(section: string, tvdbId: string, name: string, year?: number | null) {
  return `${section}-${tvdbId}-${name}-${year ?? ""}`;
}

function SearchSectionLoading({ label }: { label: string }) {
  return (
    <div className="flex min-h-24 items-center gap-3 rounded-lg border border-dashed border-border/80 bg-muted/30 px-4 py-3 text-sm text-muted-foreground">
      <Loader2 className="h-4 w-4 animate-spin text-emerald-500" />
      <span>{label}</span>
    </div>
  );
}

export const RootHeader = React.memo(function RootHeader({
  routeCommandPalette,
  onOpenOverview,
}: RootHeaderProps) {
  const searchState = useSearchContext();
  const {
    resolveDefaultQualityProfileIdForFacet,
    animeCatalogDefaults,
    addMetadataSearchResultToCatalog,
    closeGlobalSearchPanel,
    openGlobalSearchPanel,
    setGlobalSearch,
    globalSearchInputRef,
    isMetadataSearchResultInCatalog,
    catalogQualityProfileOptions,
    rootFoldersByFacet,
  } = searchState;
  const t = useTranslate();
  const isMobile = useIsMobile();
  const { theme } = useTheme();
  const searchShellRef = React.useRef<HTMLDivElement>(null);
  const searchPanelRef = React.useRef<HTMLDivElement>(null);
  const hasAnyMatches =
    searchState.catalogSearchResults.length > 0 ||
    FACET_REGISTRY.some((f) => (searchState.metadataSearchResults[f.metadataKey] ?? []).length > 0);
  const showSectionResults =
    searchState.catalogSearchLoading || searchState.metadataSearchLoading || hasAnyMatches;

  const catalogSearchSections = React.useMemo(
    () => Object.fromEntries(
      FACET_REGISTRY.map((f) => [
        f.id,
        searchState.catalogSearchResults.filter((title) => catalogFacetFromString(title.facet) === f.id),
      ]),
    ) as Record<Facet, import("@/lib/types").TitleRecord[]>,
    [searchState.catalogSearchResults],
  );
  const [expandedMetadataCardKey, setExpandedMetadataCardKey] = React.useState<string | null>(null);
  const [metadataAddDrafts, setMetadataAddDrafts] = React.useState<
    Record<string, MetadataCatalogAddOptions>
  >({});
  const [metadataAddInFlightKeys, setMetadataAddInFlightKeys] = React.useState<
    Record<string, boolean>
  >({});
  const [metadataAddedKeys, setMetadataAddedKeys] = React.useState<
    Record<string, boolean>
  >({});

  const defaultAddOptionsForFacet = React.useCallback(
    (facet: Facet): MetadataCatalogAddOptions => ({
      qualityProfileId: resolveDefaultQualityProfileIdForFacet(facet),
      seasonFolder: facet !== "movie",
      monitorType: defaultMonitorTypeForFacet(facet),
      ...(facet === "movie" ? { minAvailability: "announced" } : {}),
      ...(facet === "anime"
        ? {
            monitorSpecials: animeCatalogDefaults.monitorSpecials,
            interSeasonMovies: animeCatalogDefaults.interSeasonMovies,
          }
        : {}),
    }),
    [animeCatalogDefaults, resolveDefaultQualityProfileIdForFacet],
  );

  const toggleMetadataAddOptionsCard = React.useCallback(
    (cardKey: string, facet: Facet) => {
      setExpandedMetadataCardKey((current) => (current === cardKey ? null : cardKey));
      setMetadataAddDrafts((previous) => {
        if (previous[cardKey]) {
          return previous;
        }
        return {
          ...previous,
          [cardKey]: defaultAddOptionsForFacet(facet),
        };
      });
    },
    [defaultAddOptionsForFacet],
  );

  const updateMetadataAddDraft = React.useCallback(
    (cardKey: string, facet: Facet, patch: Partial<MetadataCatalogAddOptions>) => {
      setMetadataAddDrafts((previous) => {
        const current = previous[cardKey] ?? defaultAddOptionsForFacet(facet);
        const next: MetadataCatalogAddOptions = {
          ...current,
          ...patch,
        };
        if (
          current.qualityProfileId === next.qualityProfileId &&
          current.seasonFolder === next.seasonFolder &&
          current.monitorType === next.monitorType &&
          current.minAvailability === next.minAvailability &&
          current.monitorSpecials === next.monitorSpecials &&
          current.interSeasonMovies === next.interSeasonMovies &&
          current.rootFolder === next.rootFolder
        ) {
          return previous;
        }
        return {
          ...previous,
          [cardKey]: next,
        };
      });
    },
    [defaultAddOptionsForFacet],
  );

  const submitMetadataAddFromCard = React.useCallback(
    async (result: MetadataTvdbSearchItem, facet: Facet, cardKey: string) => {
      const draft = metadataAddDrafts[cardKey] ?? defaultAddOptionsForFacet(facet);
      const qualityProfileId = (draft.qualityProfileId || resolveDefaultQualityProfileIdForFacet(facet)).trim();
      if (!qualityProfileId) {
        return;
      }

      setMetadataAddInFlightKeys((previous) => ({
        ...previous,
        [cardKey]: true,
      }));
      try {
        const titleId = await addMetadataSearchResultToCatalog(result, facet, {
          ...draft,
          qualityProfileId,
        });
        if (!titleId) {
          return;
        }

        setMetadataAddedKeys((previous) => ({ ...previous, [cardKey]: true }));
        setExpandedMetadataCardKey((current) => (current === cardKey ? null : current));
        onOpenOverview?.(viewFromFacet(facet), titleId);
        closeGlobalSearchPanel();
      } finally {
        setMetadataAddInFlightKeys((previous) => {
          if (!previous[cardKey]) {
            return previous;
          }
          const next = { ...previous };
          delete next[cardKey];
          return next;
        });
      }
    },
    [
      defaultAddOptionsForFacet,
      metadataAddDrafts,
      addMetadataSearchResultToCatalog,
      closeGlobalSearchPanel,
      onOpenOverview,
      resolveDefaultQualityProfileIdForFacet,
    ],
  );

  const handleSearchSubmit = React.useCallback(
    (event: React.FormEvent<HTMLFormElement>) => {
      event.preventDefault();
    },
    [],
  );

  const handleSearchChange = React.useCallback(
    (event: React.ChangeEvent<HTMLInputElement>) => {
      setGlobalSearch(event.target.value);
      openGlobalSearchPanel();
    },
    [openGlobalSearchPanel, setGlobalSearch],
  );

  const handleSearchFocus = React.useCallback(() => {
    openGlobalSearchPanel(isMobile || undefined);
  }, [openGlobalSearchPanel, isMobile]);

  const handleSearchEscape = React.useCallback(
    (event: React.KeyboardEvent<HTMLInputElement>) => {
      if (event.key !== "Escape") {
        return;
      }
      closeGlobalSearchPanel();
      globalSearchInputRef.current?.blur();
    },
    [globalSearchInputRef, closeGlobalSearchPanel],
  );

  const handleClearSearch = React.useCallback(() => {
    setGlobalSearch("");
    globalSearchInputRef.current?.focus();
  }, [globalSearchInputRef, setGlobalSearch]);

  const renderCatalogSection = React.useCallback(
    (items: import("@/lib/types").TitleRecord[], facet: Facet) => {
      return items.map((title) => {
        const targetView: ViewId = viewFromFacet(facet);
        const tvdbId = title.externalIds
          .find((externalId) => externalId.source.toLowerCase() === "tvdb")
          ?.value.trim();
        const posterUrl = selectPosterVariantUrl(title.posterUrl, "w70");
        return (
          <button
            key={title.id}
            type="button"
            onClick={() => {
              closeGlobalSearchPanel();
              onOpenOverview?.(targetView, title.id);
            }}
            className="block w-full rounded-lg border border-border bg-card/60 p-3 text-left hover:bg-accent/80"
            aria-label={title.name}
          >
            <div className="mb-2 flex min-h-20 items-start gap-3">
              <div className="h-20 w-14 flex-none overflow-hidden rounded-md border border-border bg-muted">
                {posterUrl ? (
                  <img
                    src={posterUrl}
                    alt={t("media.posterAlt", { name: title.name })}
                    className="h-full w-full object-cover"
                    loading="lazy"
                  />
                ) : (
                  <div className="flex h-full w-full items-center justify-center text-xs text-muted-foreground">
                    {t("label.noArt")}
                  </div>
                )}
              </div>
              <div className="min-w-0">
                <p className="text-sm font-medium text-foreground">{title.name}</p>
                <p className="text-xs text-muted-foreground">
                  {sectionLabelForFacet(t, facet)} • {title.monitored ? t("label.yes") : t("label.no")}
                  {tvdbId ? <> • {tvdbId}</> : null}
                </p>
              </div>
            </div>
          </button>
        );
      });
    },
    [closeGlobalSearchPanel, onOpenOverview, t],
  );

  const handleSearchPanelBackdropMouseDown = React.useCallback(() => {
    closeGlobalSearchPanel();
    globalSearchInputRef.current?.blur();
  }, [globalSearchInputRef, closeGlobalSearchPanel]);

  React.useEffect(() => {
    if (!searchState.isGlobalSearchPanelOpen || isMobile) {
      return;
    }

    const handleGlobalSearchPanelPointerDown = (event: PointerEvent) => {
      const target = event.target as Node | null;
      const targetElement = target instanceof Element ? target : null;
      if (target && searchShellRef.current?.contains(target)) {
        return;
      }
      if (targetElement?.closest("[data-slot='select-content']")) {
        return;
      }
      closeGlobalSearchPanel();
      globalSearchInputRef.current?.blur();
    };

    window.addEventListener("pointerdown", handleGlobalSearchPanelPointerDown);
    return () => window.removeEventListener("pointerdown", handleGlobalSearchPanelPointerDown);
  }, [
    closeGlobalSearchPanel,
    globalSearchInputRef,
    isMobile,
    searchState.isGlobalSearchPanelOpen,
  ]);

  React.useEffect(() => {
    if (!searchState.isGlobalSearchPanelOpen || isMobile) {
      return;
    }

    const handleGlobalSearchPanelEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") {
        return;
      }
      closeGlobalSearchPanel();
      globalSearchInputRef.current?.blur();
    };

    window.addEventListener("keydown", handleGlobalSearchPanelEscape);
    return () => window.removeEventListener("keydown", handleGlobalSearchPanelEscape);
  }, [
    closeGlobalSearchPanel,
    globalSearchInputRef,
    isMobile,
    searchState.isGlobalSearchPanelOpen,
  ]);

  React.useEffect(() => {
    if (!searchState.isGlobalSearchPanelOpen) {
      setExpandedMetadataCardKey(null);
    }
  }, [searchState.isGlobalSearchPanelOpen]);

  const renderMetadataSection = React.useCallback(
    (items: MetadataTvdbSearchItem[], facet: Facet, section: string) => {
      return items.map((result) => {
        const tvdbId = String(result.tvdbId).trim();
        const isInCatalog = isMetadataSearchResultInCatalog(facet, result);
        const cardKey = renderMetadataResultKey(section, tvdbId, result.name, result.year);
        const draft = metadataAddDrafts[cardKey] ?? defaultAddOptionsForFacet(facet);
        const qualityProfileValue =
          draft.qualityProfileId || resolveDefaultQualityProfileIdForFacet(facet);
        const isExpanded = expandedMetadataCardKey === cardKey && !isInCatalog;
        const isAdding = Boolean(metadataAddInFlightKeys[cardKey]);
        const isAdded = Boolean(metadataAddedKeys[cardKey]);
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
          <div
            key={cardKey}
            className="rounded-lg border border-border bg-card/60 p-3"
          >
            <div className="mb-2 flex items-start justify-between gap-3">
              <div className="flex min-h-20 gap-3">
                <div className="h-20 w-14 flex-none overflow-hidden rounded-md border border-border bg-muted">
                  {posterUrl ? (
                    <img
                      src={posterUrl}
                      alt={t("media.posterAlt", { name: result.name })}
                      className="h-full w-full object-cover"
                      loading="lazy"
                    />
                  ) : (
                    <div className="flex h-full w-full items-center justify-center text-xs text-muted-foreground">
                      {t("label.noArt")}
                    </div>
                  )}
                </div>
                <div className="min-w-0">
                  <p className="text-sm font-medium text-foreground">{result.name}</p>
                  <p className="text-xs text-muted-foreground">
                    {result.type || t("label.unknownType")} • {result.year ? result.year : t("label.yearUnknown")} • {result.slug || t("label.unknown")}
                  </p>
                  {result.overview ? (
                    <p className="mt-2 text-xs text-muted-foreground line-clamp-2">
                      {result.overview}
                    </p>
                  ) : null}
                </div>
              </div>
              <div className="flex items-center self-center">
                <Button
                  type="button"
                  variant={isInCatalog || isAdded ? "secondary" : "default"}
                  className={
                    isInCatalog || isAdded
                      ? "h-10 w-10 bg-accent text-card-foreground px-0"
                      : "h-10 w-10 bg-emerald-500 text-foreground hover:bg-emerald-600 px-0"
                  }
                  onClick={() => toggleMetadataAddOptionsCard(cardKey, facet)}
                  disabled={isInCatalog || isAdding || isAdded || isExpanded}
                  aria-label={isInCatalog || isAdded ? t("search.alreadyCataloged") : t("search.configureAdd")}
                  title={isInCatalog || isAdded ? t("search.alreadyCataloged") : t("search.configureAdd")}
                >
                  <Plus className="h-4 w-4" />
                </Button>
              </div>
            </div>
            <div
              className={`overflow-hidden transition-[max-height,opacity,transform,margin] duration-300 ease-out ${
                isExpanded
                  ? "mt-3 max-h-[640px] translate-y-0 opacity-100"
                  : "mt-0 max-h-0 -translate-y-1 opacity-0 pointer-events-none"
              }`}
            >
              <div className="grid gap-3 rounded-xl border border-border bg-card p-3 md:grid-cols-3">
                <label className="space-y-1">
                  <span className="block text-xs font-medium text-card-foreground">
                    {t("search.addConfigQualityProfile")}
                  </span>
                  <Select
                    value={catalogQualityProfileOptions.length > 0 ? qualityProfileValue : ""}
                    onValueChange={(v) =>
                      updateMetadataAddDraft(cardKey, facet, {
                        qualityProfileId: v,
                      })
                    }
                    disabled={isAdding || catalogQualityProfileOptions.length === 0}
                  >
                    <SelectTrigger className="h-9 w-full">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {catalogQualityProfileOptions.length === 0 ? (
                        <SelectItem value="__none" disabled>{t("search.addConfigNoQualityProfiles")}</SelectItem>
                      ) : (
                        catalogQualityProfileOptions.map((profile: CatalogQualityProfileOption) => (
                          <SelectItem key={profile.id} value={profile.id}>
                            {profile.name}
                          </SelectItem>
                        ))
                      )}
                    </SelectContent>
                  </Select>
                </label>
                {rootFoldersByFacet[facet].length >= 2 ? (
                  <label className="space-y-1">
                    <span className="block text-xs font-medium text-card-foreground">
                      {t("search.addConfigRootFolder")}
                    </span>
                    <Select
                      value={draft.rootFolder || rootFoldersByFacet[facet].find((rf) => rf.isDefault)?.path || rootFoldersByFacet[facet][0]?.path || ""}
                      onValueChange={(v) =>
                        updateMetadataAddDraft(cardKey, facet, { rootFolder: v })
                      }
                      disabled={isAdding}
                    >
                      <SelectTrigger className="h-9 w-full">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        {rootFoldersByFacet[facet].map((rf) => (
                          <SelectItem key={rf.path} value={rf.path}>
                            {rf.path.split("/").filter(Boolean).pop() || rf.path}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </label>
                ) : null}
                {facet !== "movie" ? (
                  <label className="space-y-1">
                    <span className="block text-xs font-medium text-card-foreground">
                      {t("search.addConfigSeasonFolder")}
                    </span>
                    <Select
                      value={draft.seasonFolder ? "enabled" : "disabled"}
                      onValueChange={(v) =>
                        updateMetadataAddDraft(cardKey, facet, {
                          seasonFolder: v === "enabled",
                        })
                      }
                      disabled={isAdding}
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
                {facet === "anime" ? (
                  <>
                    <label className="space-y-1">
                      <span className="block text-xs font-medium text-card-foreground">
                        {t("settings.monitorSpecialsLabel")}
                      </span>
                      <div className="flex min-h-9 w-full items-center">
                        <button
                          type="button"
                          className="inline-flex items-center justify-center disabled:opacity-50"
                          onClick={() =>
                            updateMetadataAddDraft(cardKey, facet, {
                              monitorSpecials: draft.monitorSpecials === false,
                            })
                          }
                          disabled={isAdding}
                        >
                          {draft.monitorSpecials !== false ? (
                            <Eye className="h-5 w-5 text-emerald-600 dark:text-emerald-300" />
                          ) : (
                            <EyeOff className="h-5 w-5 text-rose-600 dark:text-rose-300" />
                          )}
                        </button>
                      </div>
                    </label>
                    <label className="space-y-1">
                      <span className="block text-xs font-medium text-card-foreground">
                        {t("settings.interSeasonMoviesLabel")}
                      </span>
                      <div className="flex min-h-9 w-full items-center">
                        <button
                          type="button"
                          className="inline-flex items-center justify-center disabled:opacity-50"
                          onClick={() =>
                            updateMetadataAddDraft(cardKey, facet, {
                              interSeasonMovies: draft.interSeasonMovies === false,
                            })
                          }
                          disabled={isAdding}
                        >
                          {draft.interSeasonMovies !== false ? (
                            <Eye className="h-5 w-5 text-emerald-600 dark:text-emerald-300" />
                          ) : (
                            <EyeOff className="h-5 w-5 text-rose-600 dark:text-rose-300" />
                          )}
                        </button>
                      </div>
                    </label>
                  </>
                ) : null}
                {facet === "movie" ? (
                  <>
                    <label className="space-y-1">
                      <span className="block text-xs font-medium text-card-foreground">
                        {t("title.monitored")}
                      </span>
                      <div className="flex min-h-9 w-full items-center">
                        <button
                          type="button"
                          className="inline-flex items-center justify-center disabled:opacity-50"
                          onClick={() =>
                            updateMetadataAddDraft(cardKey, facet, {
                              monitorType: draft.monitorType === "monitored" ? "unmonitored" : "monitored",
                            })
                          }
                          disabled={isAdding}
                        >
                          {draft.monitorType === "monitored" ? (
                            <Eye className="h-5 w-5 text-emerald-600 dark:text-emerald-300" />
                          ) : (
                            <EyeOff className="h-5 w-5 text-rose-600 dark:text-rose-300" />
                          )}
                        </button>
                      </div>
                    </label>
                  </>
                ) : (
                  <label className="space-y-1">
                    <span className="block text-xs font-medium text-card-foreground">
                      {t("search.addConfigMonitorType")}
                    </span>
                    <Select
                      value={draft.monitorType}
                      onValueChange={(v) =>
                        updateMetadataAddDraft(cardKey, facet, {
                          monitorType: v as MetadataCatalogMonitorType,
                        })
                      }
                      disabled={isAdding}
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
                <div className="flex items-end md:col-span-3">
                  <Button
                    type="button"
                    onClick={() => void submitMetadataAddFromCard(result, facet, cardKey)}
                    disabled={isAdding || !qualityProfileValue}
                    className="h-9 w-full bg-emerald-600 text-foreground hover:bg-emerald-500"
                  >
                    {isAdding ? t("search.adding") : t("title.addToCatalog")}
                  </Button>
                </div>
              </div>
            </div>
          </div>
        );
      });
    },
    [
      catalogQualityProfileOptions,
      defaultAddOptionsForFacet,
      expandedMetadataCardKey,
      isMetadataSearchResultInCatalog,
      metadataAddDrafts,
      metadataAddedKeys,
      metadataAddInFlightKeys,
      resolveDefaultQualityProfileIdForFacet,
      rootFoldersByFacet,
      submitMetadataAddFromCard,
      t,
      toggleMetadataAddOptionsCard,
      updateMetadataAddDraft,
    ],
  );

  return (
    <>
      <header
        data-slot="root-header"
        className="relative sticky top-0 z-50 border-b border-border bg-background/90 pt-safe backdrop-blur"
      >
        <RouteCommandPalette config={routeCommandPalette} />
        <div className="mx-auto flex w-full max-w-[1480px] items-center gap-3 px-3 py-3 pr-14 sm:gap-4 sm:pr-3">
          <div
            className="shrink-0"
            style={{ fontFamily: "var(--font-inter), ui-sans-serif, system-ui, -apple-system, sans-serif" }}
          >
            <ScryerLogo />
            <span data-slot="brand-wordmark" className="ml-3 hidden text-2xl font-bold tracking-tight text-foreground sm:inline">
              Scryer
            </span>
          </div>
          <form
            className="relative ml-auto flex w-full min-w-0 items-center gap-3"
            onSubmit={handleSearchSubmit}
          >
            <div ref={searchShellRef} className="relative flex-1">
              <Search className="pointer-events-none absolute left-3 top-1/2 h-5 w-5 -translate-y-1/2 text-muted-foreground sm:h-7 sm:w-7" />
              <Input
                ref={searchState.globalSearchInputRef}
                value={searchState.globalSearch}
                onChange={handleSearchChange}
                onFocus={handleSearchFocus}
                onKeyDown={handleSearchEscape}
                data-ui="global-search"
                className={cn(
                  "h-12 w-full pl-10 pr-3 text-base placeholder:text-base sm:h-14 sm:pl-12 sm:text-xl sm:placeholder:text-xl placeholder-heading-font",
                  theme !== "pride" && "border-emerald-500/70 focus-visible:border-emerald-400 focus-visible:ring-emerald-400/45",
                )}
                placeholder={t("search.globalPlaceholder")}
                aria-label={t("search.globalPlaceholder")}
              />
              {searchState.globalSearch && !isMobile ? (
                <button
                  type="button"
                  className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground transition hover:text-foreground"
                  onMouseDown={handleClearSearch}
                  aria-label={t("label.clear")}
                >
                  <X className="h-6 w-6" />
                </button>
              ) : null}
              {searchState.isGlobalSearchPanelOpen && !isMobile ? (
                <div
                  ref={searchPanelRef}
                  data-slot="global-search-panel"
                  className="absolute left-0 top-full z-30 mt-2 w-full max-h-[65vh] overflow-y-auto rounded-xl border border-border bg-card p-4 shadow-lg"
                >
                {showSectionResults ? (
                  <div className="space-y-4">
                    <section className="space-y-2">
                      <h3 className="text-sm font-semibold text-foreground">{t("search.catalog")}</h3>
                      <div className="grid gap-4 md:grid-cols-3">
                        {FACET_REGISTRY.map((f) => {
                          const items = catalogSearchSections[f.id] ?? [];
                          return (
                            <div key={f.id} className="space-y-2">
                              <h4 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                                {sectionLabelForFacet(t, f.id)}
                              </h4>
                              {searchState.catalogSearchLoading ? (
                                <SearchSectionLoading label={t("label.loading")} />
                              ) : items.length === 0 ? (
                                <p className="text-sm text-muted-foreground">
                                  {t("search.noCatalogMatches")}
                                </p>
                              ) : (
                                renderCatalogSection(items, f.id)
                              )}
                            </div>
                          );
                        })}
                      </div>
                    </section>
                      <div className={`grid gap-4 md:grid-cols-${FACET_REGISTRY.length}`}>
                        {FACET_REGISTRY.map((f) => {
                          const items = searchState.metadataSearchResults[f.metadataKey] ?? [];
                          return (
                            <section key={f.id} className="space-y-2">
                              <h3 className="text-sm font-semibold text-foreground">
                                {sectionLabelForFacet(t, f.id)}
                              </h3>
                              {searchState.metadataSearchLoading ? (
                                <SearchSectionLoading label={t("label.loading")} />
                              ) : items.length === 0 ? (
                                <p className="text-sm text-muted-foreground">
                                  {t("search.noMetadataMatches")}
                                </p>
                              ) : (
                                renderMetadataSection(items, f.id, f.metadataKey)
                              )}
                            </section>
                          );
                        })}
                      </div>
                    </div>
                  ) : searchState.searching ? (
                    <div className="flex items-center gap-3 py-3">
                      <Loader2 className="h-5 w-5 animate-spin text-emerald-500" />
                      <p className="text-sm text-muted-foreground">{t("label.searching")}</p>
                    </div>
                  ) : (
                    <p className="text-sm text-muted-foreground">{t("status.nothingFound")}</p>
                  )}
                </div>
              ) : null}
            </div>
          </form>
        </div>
      </header>
      {searchState.isGlobalSearchPanelOpen && !isMobile ? (
        <div
          className="fixed inset-0 z-40 bg-background/80 backdrop-blur-sm"
          onMouseDown={handleSearchPanelBackdropMouseDown}
          aria-hidden="true"
        />
      ) : null}
      {searchState.isGlobalSearchPanelOpen && isMobile ? (
        <MobileSearchOverlay
          onClose={searchState.closeGlobalSearchPanel}
          onOpenOverview={onOpenOverview}
        />
      ) : null}
    </>
  );
});

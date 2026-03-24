
import * as React from "react";
import { Loader2, Plus, Search, X } from "lucide-react";
import { useTheme } from "next-themes";
import ScryerLogo from "@/components/scryer-logo";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { RouteCommandPalette } from "@/components/common/route-command-palette";
import type { ViewId } from "@/components/root/types";
import { useTranslate } from "@/lib/context/translate-context";
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import type { Facet } from "@/lib/types";
import type { MetadataCatalogAddOptions } from "@/lib/hooks/use-global-search";
import type { RouteCommandPaletteConfig } from "@/components/common/route-command-palette";
import { useIsMobile } from "@/lib/hooks/use-mobile";
import { MobileSearchOverlay } from "@/components/root/mobile-search-overlay";
import { FACET_REGISTRY } from "@/lib/facets/registry";
import {
  sectionLabelForFacet,
  viewFromFacet,
} from "@/lib/facets/helpers";
import { selectPosterVariantUrl } from "@/lib/utils/poster-images";
import { TitlePoster } from "@/components/title-poster";
import { useSearchContext } from "@/lib/context/search-context";
import { cn } from "@/lib/utils";
import { AddToCatalogDialog, EMPTY_SEARCH_RESULT } from "@/components/root/add-to-catalog-dialog";


type RootHeaderProps = {
  routeCommandPalette?: RouteCommandPaletteConfig;
  onOpenOverview?: (targetView: ViewId, titleId: string) => void;
};

function catalogFacetFromString(facet: string): Facet {
  return facet === "movie" ? "movie" : facet === "anime" ? "anime" : "tv";
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
    addMetadataSearchResultToCatalog,
    closeGlobalSearchPanel,
    openGlobalSearchPanel,
    forceSearchGlobal,
    setGlobalSearch,
    globalSearchInputRef,
    isMetadataSearchResultInCatalog,
    catalogQualityProfileOptions,
    rootFoldersByFacet,
    catalogSearchResults,
    catalogSearchLoading,
    metadataSearchResults,
    metadataSearchLoading,
    isGlobalSearchPanelOpen,
    globalSearch,
    searching,
  } = searchState;
  const t = useTranslate();
  const isMobile = useIsMobile();
  const { theme } = useTheme();
  const searchShellRef = React.useRef<HTMLDivElement>(null);
  const searchPanelRef = React.useRef<HTMLDivElement>(null);
  const hasAnyMatches =
    catalogSearchResults.length > 0 ||
    FACET_REGISTRY.some((f) => (metadataSearchResults[f.metadataKey] ?? []).length > 0);
  const showSectionResults =
    catalogSearchLoading || metadataSearchLoading || hasAnyMatches;

  const catalogSearchSections = React.useMemo(
    () => Object.fromEntries(
      FACET_REGISTRY.map((f) => [
        f.id,
        catalogSearchResults.filter((title) => catalogFacetFromString(title.facet) === f.id),
      ]),
    ) as Record<Facet, import("@/lib/types").TitleRecord[]>,
    [catalogSearchResults],
  );
  const [addDialogTarget, setAddDialogTarget] = React.useState<{
    result: MetadataTvdbSearchItem;
    facet: Facet;
  } | null>(null);

  const handleAddDialogSubmit = React.useCallback(
    async (result: MetadataTvdbSearchItem, facet: Facet, options: MetadataCatalogAddOptions) => {
      const titleId = await addMetadataSearchResultToCatalog(result, facet, options);
      if (titleId) {
        onOpenOverview?.(viewFromFacet(facet), titleId);
        closeGlobalSearchPanel();
      }
      return titleId;
    },
    [addMetadataSearchResultToCatalog, closeGlobalSearchPanel, onOpenOverview],
  );

  const handleSearchSubmit = React.useCallback(
    (event: React.FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      void forceSearchGlobal();
    },
    [forceSearchGlobal],
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
                {(posterUrl || title.posterSourceUrl) ? (
                  <TitlePoster
                    src={posterUrl}
                    sourceSrc={title.posterSourceUrl}
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
    if (!isGlobalSearchPanelOpen || isMobile) {
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
      if (targetElement?.closest("[data-slot='dialog-overlay'], [data-slot='dialog-content']")) {
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
    isGlobalSearchPanelOpen,
  ]);

  React.useEffect(() => {
    if (!isGlobalSearchPanelOpen || isMobile) {
      return;
    }

    const handleGlobalSearchPanelEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") {
        return;
      }
      if (addDialogTarget !== null) {
        return;
      }
      closeGlobalSearchPanel();
      globalSearchInputRef.current?.blur();
    };

    window.addEventListener("keydown", handleGlobalSearchPanelEscape);
    return () => window.removeEventListener("keydown", handleGlobalSearchPanelEscape);
  }, [
    addDialogTarget,
    closeGlobalSearchPanel,
    globalSearchInputRef,
    isMobile,
    isGlobalSearchPanelOpen,
  ]);

  const renderMetadataSection = React.useCallback(
    (items: MetadataTvdbSearchItem[], facet: Facet, _section: string) => {
      return items.map((result) => {
        const isInCatalog = isMetadataSearchResultInCatalog(facet, result);
        const posterUrl = selectPosterVariantUrl(result.posterUrl, "w70");
        return (
          <div
            key={`${facet}-${result.tvdbId}-${result.name}`}
            className="rounded-lg border border-border bg-card/60 p-3"
          >
            <div className="flex items-start justify-between gap-3">
              <div className="flex min-h-20 gap-3">
                <div className="h-20 w-14 flex-none overflow-hidden rounded-md border border-border bg-muted">
                  {posterUrl ? (
                    <TitlePoster
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
                  variant={isInCatalog ? "secondary" : "default"}
                  className={
                    isInCatalog
                      ? "h-10 w-10 bg-accent text-card-foreground px-0"
                      : "h-10 w-10 bg-emerald-500 text-foreground hover:bg-emerald-600 px-0"
                  }
                  onClick={() => setAddDialogTarget({ result, facet })}
                  disabled={isInCatalog}
                  aria-label={isInCatalog ? t("search.alreadyCataloged") : t("search.configureAdd")}
                  title={isInCatalog ? t("search.alreadyCataloged") : t("search.configureAdd")}
                >
                  <Plus className="h-4 w-4" />
                </Button>
              </div>
            </div>
          </div>
        );
      });
    },
    [isMetadataSearchResultInCatalog, t],
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
            <div className="flex flex-col items-center">
              <ScryerLogo />
              <span data-slot="brand-wordmark" className="hidden text-3xl font-bold tracking-tight text-foreground sm:block">
                Scryer
              </span>
            </div>
          </div>
          <form
            className="relative ml-auto flex w-full min-w-0 items-center gap-3"
            onSubmit={handleSearchSubmit}
          >
            <div ref={searchShellRef} className="relative flex-1">
              <Search className="pointer-events-none absolute left-3 top-1/2 h-5 w-5 -translate-y-1/2 text-muted-foreground sm:h-7 sm:w-7" />
              <Input
                ref={globalSearchInputRef}
                value={globalSearch}
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
              {globalSearch && !isMobile ? (
                <button
                  type="button"
                  className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground transition hover:text-foreground"
                  onMouseDown={handleClearSearch}
                  aria-label={t("label.clear")}
                >
                  <X className="h-6 w-6" />
                </button>
              ) : null}
              {isGlobalSearchPanelOpen && !isMobile ? (
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
                              {catalogSearchLoading ? (
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
                          const items = metadataSearchResults[f.metadataKey] ?? [];
                          return (
                            <section key={f.id} className="space-y-2">
                              <h3 className="text-sm font-semibold text-foreground">
                                {sectionLabelForFacet(t, f.id)}
                              </h3>
                              {metadataSearchLoading ? (
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
                  ) : searching ? (
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
      {isGlobalSearchPanelOpen && !isMobile ? (
        <div
          className="fixed inset-0 z-40 bg-background/80 backdrop-blur-sm"
          onMouseDown={handleSearchPanelBackdropMouseDown}
          aria-hidden="true"
        />
      ) : null}
      {isGlobalSearchPanelOpen && isMobile ? (
        <MobileSearchOverlay
          onClose={closeGlobalSearchPanel}
          onOpenOverview={onOpenOverview}
        />
      ) : null}
      <AddToCatalogDialog
        open={addDialogTarget !== null}
        onOpenChange={(open) => { if (!open) setAddDialogTarget(null); }}
        result={addDialogTarget?.result ?? EMPTY_SEARCH_RESULT}
        facet={addDialogTarget?.facet ?? "tv"}
        catalogQualityProfileOptions={catalogQualityProfileOptions}
        defaultQualityProfileId={resolveDefaultQualityProfileIdForFacet(addDialogTarget?.facet ?? "tv")}
        rootFolders={rootFoldersByFacet[addDialogTarget?.facet ?? "tv"]}
        onSubmit={handleAddDialogSubmit}
      />
    </>
  );
});

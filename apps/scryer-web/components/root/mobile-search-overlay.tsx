
import * as React from "react";
import { ArrowLeft, Loader2, Plus, Search, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import type { ViewId } from "@/components/root/types";
import { useTranslate } from "@/lib/context/translate-context";
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import type { Facet } from "@/lib/types";
import type { MetadataCatalogAddOptions } from "@/lib/hooks/use-global-search";
import { FACET_REGISTRY } from "@/lib/facets/registry";
import {
  sectionLabelForFacet,
  viewFromFacet,
} from "@/lib/facets/helpers";
import { useSearchContext } from "@/lib/context/search-context";
import { selectPosterVariantUrl } from "@/lib/utils/poster-images";
import { TitlePoster } from "@/components/title-poster";
import { AddToCatalogDialog, EMPTY_SEARCH_RESULT } from "@/components/root/add-to-catalog-dialog";

type MobileSearchOverlayProps = {
  onClose: () => void;
  onOpenOverview?: (targetView: ViewId, titleId: string) => void;
};

function catalogFacetFromString(facet: string): Facet {
  return facet === "movie" ? "movie" : facet === "anime" ? "anime" : "tv";
}

function SearchSectionLoading({ label }: { label: string }) {
  return (
    <div className="flex min-h-20 items-center gap-3 rounded-lg border border-dashed border-border/80 bg-muted/30 px-4 py-3 text-sm text-muted-foreground">
      <Loader2 className="h-4 w-4 animate-spin text-emerald-500" />
      <span>{label}</span>
    </div>
  );
}

export function MobileSearchOverlay({
  onClose,
  onOpenOverview,
}: MobileSearchOverlayProps) {
  const searchState = useSearchContext();
  const t = useTranslate();
  const inputRef = React.useRef<HTMLInputElement>(null);
  const [addDialogTarget, setAddDialogTarget] = React.useState<{
    result: MetadataTvdbSearchItem;
    facet: Facet;
  } | null>(null);

  // Focus the input when the overlay mounts.
  // Mobile Safari restricts focus() to user-gesture contexts, so we also
  // use autoFocus on the input and retry with a short delay as a fallback.
  React.useEffect(() => {
    inputRef.current?.focus();
    const timer = setTimeout(() => inputRef.current?.focus(), 50);
    return () => clearTimeout(timer);
  }, []);

  // Prevent body scroll while overlay is open
  React.useEffect(() => {
    const original = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = original;
    };
  }, []);

  const hasMetadataMatches = FACET_REGISTRY.some(
    (f) => (searchState.metadataSearchResults[f.metadataKey] ?? []).length > 0,
  );

  const catalogSearchSections = React.useMemo(
    () => Object.fromEntries(
      FACET_REGISTRY.map((f) => [
        f.id,
        searchState.catalogSearchResults.filter((title) => catalogFacetFromString(title.facet) === f.id),
      ]),
    ) as Record<Facet, import("@/lib/types").TitleRecord[]>,
    [searchState.catalogSearchResults],
  );

  const {
    resolveDefaultQualityProfileIdForFacet,
    animeCatalogDefaults,
    addMetadataSearchResultToCatalog,
    isMetadataSearchResultInCatalog,
    catalogQualityProfileOptions,
    rootFoldersByFacet,
  } = searchState;

  const handleAddDialogSubmit = React.useCallback(
    async (result: MetadataTvdbSearchItem, facet: Facet, options: MetadataCatalogAddOptions) => {
      const titleId = await addMetadataSearchResultToCatalog(result, facet, options);
      if (titleId) {
        onOpenOverview?.(viewFromFacet(facet), titleId);
        onClose();
      }
      return titleId;
    },
    [addMetadataSearchResultToCatalog, onClose, onOpenOverview],
  );

  const renderCatalogItem = React.useCallback(
    (title: import("@/lib/types").TitleRecord, facet: "movie" | "tv" | "anime") => {
      const targetView: ViewId = facet === "tv" ? "series" : facet === "anime" ? "anime" : "movies";
      const tvdbId = title.externalIds
        .find((externalId) => externalId.source.toLowerCase() === "tvdb")
        ?.value.trim();
      const posterUrl = selectPosterVariantUrl(title.posterUrl, "w70");

      return (
        <button
          key={title.id}
          type="button"
          onClick={() => {
            onClose();
            onOpenOverview?.(targetView, title.id);
          }}
          className="block w-full rounded-lg border border-border bg-card/60 p-3 text-left active:bg-accent/80"
          aria-label={title.name}
        >
          <div className="flex min-h-[44px] items-center gap-3">
            <div className="h-16 w-11 flex-none overflow-hidden rounded-md border border-border bg-muted">
              {(posterUrl || title.posterSourceUrl) ? (
                <TitlePoster
                  src={posterUrl}
                  sourceSrc={title.posterSourceUrl}
                  alt={t("media.posterAlt", { name: title.name })}
                  className="h-full w-full object-cover"
                  loading="lazy"
                />
              ) : (
                <div className="flex h-full w-full items-center justify-center text-[10px] text-muted-foreground">
                  {t("label.noArt")}
                </div>
              )}
            </div>
            <div className="min-w-0 flex-1">
              <p className="text-sm font-medium text-foreground">{title.name}</p>
              <p className="text-xs text-muted-foreground">
                {sectionLabelForFacet(t, facet)} {title.monitored ? `• ${t("label.yes")}` : ""}
                {tvdbId ? ` • ${tvdbId}` : ""}
              </p>
            </div>
          </div>
        </button>
      );
    },
    [onClose, onOpenOverview, t],
  );

  const renderMetadataItem = React.useCallback(
    (result: MetadataTvdbSearchItem, facet: "movie" | "tv" | "anime") => {
      const isInCatalog = isMetadataSearchResultInCatalog(facet, result);
      const posterUrl = selectPosterVariantUrl(result.posterUrl, "w70");

      return (
        <div key={`${facet}-${result.tvdbId}-${result.name}`} className="rounded-lg border border-border bg-card/60 p-3">
          <div className="flex min-h-[44px] items-center gap-3">
            <div className="h-16 w-11 flex-none overflow-hidden rounded-md border border-border bg-muted">
              {posterUrl ? (
                <TitlePoster
                  src={posterUrl}
                  alt={t("media.posterAlt", { name: result.name })}
                  className="h-full w-full object-cover"
                  loading="lazy"
                />
              ) : (
                <div className="flex h-full w-full items-center justify-center text-[10px] text-muted-foreground">
                  {t("label.noArt")}
                </div>
              )}
            </div>
            <div className="min-w-0 flex-1">
              <p className="text-sm font-medium text-foreground">{result.name}</p>
              <p className="text-xs text-muted-foreground">
                {result.type || t("label.unknownType")} • {result.year || t("label.yearUnknown")}
              </p>
            </div>
            <Button
              type="button"
              variant={isInCatalog ? "secondary" : "default"}
              className={
                isInCatalog
                  ? "h-10 w-10 flex-none bg-accent text-card-foreground px-0"
                  : "h-10 w-10 flex-none bg-emerald-500 text-foreground hover:bg-emerald-600 px-0"
              }
              onClick={() => setAddDialogTarget({ result, facet })}
              disabled={isInCatalog}
              aria-label={isInCatalog ? t("search.alreadyCataloged") : t("search.configureAdd")}
            >
              <Plus className="h-4 w-4" />
            </Button>
          </div>

          {result.overview ? (
            <p className="mt-2 text-xs text-muted-foreground line-clamp-2">{result.overview}</p>
          ) : null}
        </div>
      );
    },
    [isMetadataSearchResultInCatalog, t],
  );

  const renderCatalogSection = (
    items: import("@/lib/types").TitleRecord[],
    facet: Facet,
    loading: boolean,
  ) => {
    if (!loading && items.length === 0) return null;
    return (
      <div className="space-y-2">
        <h4 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
          {sectionLabelForFacet(t, facet)}
        </h4>
        {loading ? (
          <SearchSectionLoading label={t("label.loading")} />
        ) : (
          <div className="space-y-2">
            {items.slice(0, 3).map((title) => renderCatalogItem(title, facet))}
          </div>
        )}
      </div>
    );
  };

  const renderMetadataSection = (
    items: MetadataTvdbSearchItem[],
    facet: Facet,
    _section: string,
    loading: boolean,
  ) => {
    if (!loading && items.length === 0) return null;
    return (
      <div className="space-y-2">
        <h4 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
          {sectionLabelForFacet(t, facet)}
        </h4>
        {loading ? (
          <SearchSectionLoading label={t("label.loading")} />
        ) : (
          <div className="space-y-2">
            {items.slice(0, 3).map((result) => renderMetadataItem(result, facet))}
          </div>
        )}
      </div>
    );
  };

  const hasCatalog = catalogSearchSections.movie.length > 0 || catalogSearchSections.tv.length > 0 || catalogSearchSections.anime.length > 0;
  const showCatalogSection = searchState.catalogSearchLoading || hasCatalog;
  const showMetadataSection = searchState.metadataSearchLoading || hasMetadataMatches;
  const showSectionResults = showCatalogSection || showMetadataSection;

  return (
    <div className="fixed inset-0 z-50 flex flex-col bg-background">
      {/* Sticky search header */}
      <div className="flex items-center gap-2 border-b border-border bg-background px-3 py-3 pb-safe">
        <button
          type="button"
          onClick={onClose}
          className="flex h-10 w-10 flex-none items-center justify-center rounded-lg text-muted-foreground active:bg-accent"
          aria-label={t("label.back")}
        >
          <ArrowLeft className="h-5 w-5" />
        </button>
        <div className="relative flex-1">
          <Search className="pointer-events-none absolute left-3 top-1/2 h-5 w-5 -translate-y-1/2 text-muted-foreground" />
          <Input
            ref={inputRef}
            value={searchState.globalSearch}
            onChange={(e) => searchState.setGlobalSearch(e.target.value)}
            className="h-10 w-full border-emerald-500/70 pl-10 text-base placeholder-heading-font focus-visible:border-emerald-400 focus-visible:ring-emerald-400/45"
            placeholder={t("search.globalPlaceholder")}
            aria-label={t("search.globalPlaceholder")}
            autoFocus
          />
          {searchState.globalSearch ? (
            <button
              type="button"
              className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground"
              onClick={() => {
                searchState.setGlobalSearch("");
                inputRef.current?.focus();
              }}
              aria-label={t("label.clear")}
            >
              <X className="h-5 w-5" />
            </button>
          ) : null}
        </div>
      </div>


      {/* Scrollable results */}
      <div className="flex-1 overflow-y-auto px-3 py-4 pb-safe">
        {showSectionResults ? (
          <div className="space-y-6">
            {showCatalogSection ? (
              <section className="space-y-3">
                <h3 className="text-sm font-semibold text-foreground">{t("search.catalog")}</h3>
                <div className="space-y-3">
                  {FACET_REGISTRY.map((f) =>
                    renderCatalogSection(
                      catalogSearchSections[f.id] ?? [],
                      f.id,
                      searchState.catalogSearchLoading,
                    ),
                  )}
                </div>
              </section>
            ) : null}

            {showMetadataSection ? (
              <section className="space-y-3">
                <h3 className="text-sm font-semibold text-foreground">{t("search.metadataSearch")}</h3>
                <div className="space-y-3">
                  {FACET_REGISTRY.map((f) =>
                    renderMetadataSection(
                      searchState.metadataSearchResults[f.metadataKey] ?? [],
                      f.id,
                      f.metadataKey,
                      searchState.metadataSearchLoading,
                    ),
                  )}
                </div>
              </section>
            ) : null}
          </div>
        ) : searchState.searching ? (
          <div className="flex items-center gap-3 py-6">
            <Loader2 className="h-5 w-5 animate-spin text-emerald-500" />
            <p className="text-sm text-muted-foreground">{t("label.searching")}</p>
          </div>
        ) : searchState.globalSearch ? (
          <p className="py-6 text-center text-sm text-muted-foreground">{t("status.nothingFound")}</p>
        ) : (
          <p className="py-6 text-center text-sm text-muted-foreground">{t("search.globalPlaceholder")}</p>
        )}
      </div>
      <AddToCatalogDialog
        open={addDialogTarget !== null}
        onOpenChange={(open) => { if (!open) setAddDialogTarget(null); }}
        result={addDialogTarget?.result ?? EMPTY_SEARCH_RESULT}
        facet={addDialogTarget?.facet ?? "tv"}
        catalogQualityProfileOptions={catalogQualityProfileOptions}
        defaultQualityProfileId={resolveDefaultQualityProfileIdForFacet(addDialogTarget?.facet ?? "tv")}
        rootFolders={rootFoldersByFacet[addDialogTarget?.facet ?? "tv"]}
        animeCatalogDefaults={animeCatalogDefaults}
        onSubmit={handleAddDialogSubmit}
      />
    </div>
  );
}

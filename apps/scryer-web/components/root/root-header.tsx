
import * as React from "react";
import { Loader2, Monitor, Moon, Plus, Search, Sun, X } from "lucide-react";
import { useTheme } from "next-themes";
import ScryerLogo from "@/components/scryer-logo";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { RouteCommandPalette } from "@/components/common/route-command-palette";
import type { Translate } from "@/components/root/types";
import type { ViewId } from "@/components/root/types";
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import type { Facet, TitleRecord } from "@/lib/types";
import type {
  CatalogQualityProfileOption,
  MetadataCatalogAddOptions,
  MetadataCatalogMonitorType,
  MetadataSearchResults,
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

type RootHeaderProps = {
  t: Translate;
  globalSearch: string;
  onGlobalSearchChange: (value: string) => void;
  searching: boolean;
  globalSearchInputRef: React.RefObject<HTMLInputElement | null>;
  globalStatus: string;
  catalogSearchResults: TitleRecord[];
  metadataSearchResults: MetadataSearchResults;
  routeCommandPalette?: RouteCommandPaletteConfig;
  isGlobalSearchPanelOpen: boolean;
  onOpenGlobalSearchPanel: () => void;
  onCloseGlobalSearchPanel: () => void;
  catalogQualityProfileOptions: CatalogQualityProfileOption[];
  resolveDefaultQualityProfileIdForFacet: (facet: Facet) => string;
  onAddMetadataSearchResultToCatalog: (
    result: MetadataTvdbSearchItem,
    facet: Facet,
    options: MetadataCatalogAddOptions,
  ) => Promise<string | null>;
  isMetadataSearchResultInCatalog: (facet: Facet, result: MetadataTvdbSearchItem) => boolean;
  onOpenOverview?: (targetView: ViewId, titleId: string) => void;
};

function catalogFacetFromString(facet: string): Facet {
  return facet === "movie" ? "movie" : facet === "anime" ? "anime" : "tv";
}

function renderMetadataResultKey(section: string, tvdbId: string, name: string, year?: number | null) {
  return `${section}-${tvdbId}-${name}-${year ?? ""}`;
}

export const RootHeader = React.memo(function RootHeader({
  t,
  globalSearch,
  onGlobalSearchChange,
  searching,
  globalSearchInputRef,
  globalStatus,
  catalogSearchResults,
  metadataSearchResults,
  routeCommandPalette,
  isGlobalSearchPanelOpen,
  onOpenGlobalSearchPanel,
  onCloseGlobalSearchPanel,
  catalogQualityProfileOptions,
  resolveDefaultQualityProfileIdForFacet,
  onAddMetadataSearchResultToCatalog,
  isMetadataSearchResultInCatalog,
  onOpenOverview,
}: RootHeaderProps) {
  const isMobile = useIsMobile();
  const { theme, setTheme } = useTheme();
  const [mounted, setMounted] = React.useState(false);
  React.useEffect(() => setMounted(true), []);
  const cycleTheme = React.useCallback(() => {
    if (theme === "light") setTheme("dark");
    else if (theme === "dark") setTheme("system");
    else setTheme("light");
  }, [theme, setTheme]);
  const searchPanelRef = React.useRef<HTMLDivElement>(null);
  const hasAnyMatches =
    catalogSearchResults.length > 0 ||
    FACET_REGISTRY.some((f) => (metadataSearchResults[f.metadataKey] ?? []).length > 0);

  const catalogSearchSections = React.useMemo(
    () => Object.fromEntries(
      FACET_REGISTRY.map((f) => [
        f.id,
        catalogSearchResults.filter((title) => catalogFacetFromString(title.facet) === f.id),
      ]),
    ) as Record<Facet, TitleRecord[]>,
    [catalogSearchResults],
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
    }),
    [resolveDefaultQualityProfileIdForFacet],
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
          current.minAvailability === next.minAvailability
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
        const titleId = await onAddMetadataSearchResultToCatalog(result, facet, {
          ...draft,
          qualityProfileId,
        });
        if (!titleId) {
          return;
        }

        setMetadataAddedKeys((previous) => ({ ...previous, [cardKey]: true }));
        setExpandedMetadataCardKey((current) => (current === cardKey ? null : current));
        onOpenOverview?.(viewFromFacet(facet), titleId);
        onCloseGlobalSearchPanel();
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
      onAddMetadataSearchResultToCatalog,
      onCloseGlobalSearchPanel,
      onOpenOverview,
      resolveDefaultQualityProfileIdForFacet,
    ],
  );

  const handleSearchSubmit = React.useCallback(
    (event: React.FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      onCloseGlobalSearchPanel();
    },
    [onCloseGlobalSearchPanel],
  );

  const handleSearchChange = React.useCallback(
    (event: React.ChangeEvent<HTMLInputElement>) => {
      onGlobalSearchChange(event.target.value);
      onOpenGlobalSearchPanel();
    },
    [onOpenGlobalSearchPanel, onGlobalSearchChange],
  );

  const handleSearchFocus = React.useCallback(() => {
    onOpenGlobalSearchPanel();
  }, [onOpenGlobalSearchPanel]);

  const handleSearchBlur = React.useCallback(
    (event: React.FocusEvent<HTMLInputElement>) => {
      const nextTarget = event.relatedTarget as Node | null;
      if (nextTarget && searchPanelRef.current?.contains(nextTarget)) {
        return;
      }
      onCloseGlobalSearchPanel();
    },
    [onCloseGlobalSearchPanel],
  );

  const handleSearchEscape = React.useCallback(
    (event: React.KeyboardEvent<HTMLInputElement>) => {
      if (event.key !== "Escape") {
        return;
      }
      onCloseGlobalSearchPanel();
      globalSearchInputRef.current?.blur();
    },
    [globalSearchInputRef, onCloseGlobalSearchPanel],
  );

  const handleClearSearch = React.useCallback(() => {
    onGlobalSearchChange("");
    globalSearchInputRef.current?.focus();
  }, [globalSearchInputRef, onGlobalSearchChange]);

  const renderCatalogSection = React.useCallback(
    (items: TitleRecord[], facet: Facet) => {
      return items.map((title) => {
        const targetView: ViewId = viewFromFacet(facet);
        const tvdbId = title.externalIds
          .find((externalId) => externalId.source.toLowerCase() === "tvdb")
          ?.value.trim();
        return (
          <button
            key={title.id}
            type="button"
            onClick={() => {
              onCloseGlobalSearchPanel();
              onOpenOverview?.(targetView, title.id);
            }}
            className="block rounded-lg border border-border bg-card/60 p-3 hover:bg-accent/80"
            aria-label={title.name}
          >
            <div className="mb-2 flex min-h-20 items-start gap-3">
              <div className="h-20 w-14 flex-none overflow-hidden rounded-md border border-border bg-muted">
                {title.posterUrl ? (
                  <img
                    src={title.posterUrl}
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
    [onCloseGlobalSearchPanel, onOpenOverview, t],
  );

  const handleSearchPanelBackdropMouseDown = React.useCallback(() => {
    onCloseGlobalSearchPanel();
    globalSearchInputRef.current?.blur();
  }, [globalSearchInputRef, onCloseGlobalSearchPanel]);

  React.useEffect(() => {
    if (!isGlobalSearchPanelOpen) {
      setExpandedMetadataCardKey(null);
    }
  }, [isGlobalSearchPanelOpen]);

  const renderMetadataSection = React.useCallback(
    (items: MetadataTvdbSearchItem[], facet: Facet, section: string) => {
      return items.map((result) => {
        const tvdbId = String(result.tvdb_id).trim();
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
        return (
          <div
            key={cardKey}
            className="rounded-lg border border-border bg-card/60 p-3"
          >
            <div className="mb-2 flex items-start justify-between gap-3">
              <div className="flex min-h-20 gap-3">
                <div className="h-20 w-14 flex-none overflow-hidden rounded-md border border-border bg-muted">
                  {result.poster_url ? (
                    <img
                      src={result.poster_url}
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
                        catalogQualityProfileOptions.map((profile) => (
                          <SelectItem key={profile.id} value={profile.id}>
                            {profile.name}
                          </SelectItem>
                        ))
                      )}
                    </SelectContent>
                  </Select>
                </label>
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
                {facet === "movie" ? (
                  <>
                    <label className="space-y-1">
                      <span className="block text-xs font-medium text-card-foreground">
                        {t("title.monitored")}
                      </span>
                      <div className="flex min-h-9 w-full items-center">
                        <Checkbox
                          className="h-8 w-8"
                          checked={draft.monitorType === "monitored"}
                          onCheckedChange={(checked) =>
                            updateMetadataAddDraft(cardKey, facet, {
                              monitorType: checked ? "monitored" : "unmonitored",
                            })
                          }
                          disabled={isAdding}
                        />
                      </div>
                    </label>
                    <label className="space-y-1">
                      <span className="block text-xs font-medium text-card-foreground">
                        {t("settings.minAvailabilityLabel")}
                      </span>
                      <Select
                        value={draft.minAvailability ?? "announced"}
                        onValueChange={(v) =>
                          updateMetadataAddDraft(cardKey, facet, {
                            minAvailability: v,
                          })
                        }
                        disabled={isAdding}
                      >
                        <SelectTrigger className="h-9 w-full">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value="announced">{t("settings.minAvailability.announced")}</SelectItem>
                          <SelectItem value="in_cinemas">{t("settings.minAvailability.in_cinemas")}</SelectItem>
                          <SelectItem value="released">{t("settings.minAvailability.released")}</SelectItem>
                        </SelectContent>
                      </Select>
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
      submitMetadataAddFromCard,
      t,
      toggleMetadataAddOptionsCard,
      updateMetadataAddDraft,
    ],
  );

  return (
    <>
      <header className="relative sticky top-0 z-50 border-b border-border bg-background/90 pt-safe backdrop-blur">
        <RouteCommandPalette config={routeCommandPalette} />
        <div className="mx-auto flex w-full max-w-[1480px] items-center gap-4 px-3 py-3">
          <div className="flex items-center" style={{ fontFamily: "var(--font-inter), ui-sans-serif, system-ui, -apple-system, sans-serif" }}>
            <ScryerLogo />
            <span className="ml-3 text-2xl font-bold tracking-tight text-foreground">Scryer</span>
          </div>
          <form
            className="relative ml-auto flex w-full items-center gap-3"
            onSubmit={handleSearchSubmit}
          >
            <div className="relative flex-1">
              <Search className="pointer-events-none absolute left-3 top-1/2 h-7 w-7 -translate-y-1/2 text-muted-foreground" />
              <Input
                ref={globalSearchInputRef}
                value={globalSearch}
                onChange={handleSearchChange}
                onFocus={isMobile ? undefined : handleSearchFocus}
                onClick={isMobile ? handleSearchFocus : undefined}
                onBlur={isMobile ? undefined : handleSearchBlur}
                onKeyDown={handleSearchEscape}
                className="h-14 w-full border-emerald-500/70 pl-12 text-xl placeholder:text-xl md:text-xl md:placeholder:text-xl placeholder-heading-font focus-visible:border-emerald-400 focus-visible:ring-emerald-400/45"
                placeholder={t("search.globalPlaceholder")}
                aria-label={t("search.globalPlaceholder")}
                readOnly={isMobile}
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
                  className="absolute left-0 top-full z-30 mt-2 w-full max-h-[65vh] overflow-y-auto rounded-xl border border-border bg-card p-4 shadow-lg"
                >
                {hasAnyMatches ? (
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
                              {items.length === 0 ? (
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
                              {items.length === 0 ? (
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
        <div className="pointer-events-none absolute right-3 top-0 z-20 flex pt-safe">
          {mounted ? (
            <Button
              type="button"
              variant="ghost"
              size="icon"
              onClick={cycleTheme}
              title={theme === "light" ? "Light" : theme === "dark" ? "Dark" : "System"}
              className="pointer-events-auto mt-3"
            >
              {theme === "light" ? (
                <Sun className="h-5 w-5" />
              ) : theme === "dark" ? (
                <Moon className="h-5 w-5" />
              ) : (
                <Monitor className="h-5 w-5" />
              )}
            </Button>
          ) : (
            <div className="mt-3 h-9 w-9" />
          )}
        </div>
        <div className="border-t border-border px-4 py-1 text-xs text-muted-foreground">
          <div className="mx-auto flex max-w-[1480px] items-center justify-between gap-2">
            <span>
              {t("search.globalStatusPrefix")}: {globalStatus}
            </span>
          </div>
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
          t={t}
          globalSearch={globalSearch}
          onGlobalSearchChange={onGlobalSearchChange}
          searching={searching}
          globalStatus={globalStatus}
          catalogSearchResults={catalogSearchResults}
          metadataSearchResults={metadataSearchResults}
          onClose={onCloseGlobalSearchPanel}
          catalogQualityProfileOptions={catalogQualityProfileOptions}
          resolveDefaultQualityProfileIdForFacet={resolveDefaultQualityProfileIdForFacet}
          onAddMetadataSearchResultToCatalog={onAddMetadataSearchResultToCatalog}
          isMetadataSearchResultInCatalog={isMetadataSearchResultInCatalog}
          onOpenOverview={onOpenOverview}
        />
      ) : null}
    </>
  );
});

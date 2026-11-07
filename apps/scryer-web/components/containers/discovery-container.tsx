import {
  memo,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useLocation } from "react-router";
import { useClient } from "urql";
import {
  AddToCatalogDialog,
  EMPTY_SEARCH_RESULT,
} from "@/components/root/add-to-catalog-dialog";
import { RequestMediaDialog } from "@/components/root/request-media-dialog";
import {
  DiscoveryView,
  type DiscoverySectionGridModel,
} from "@/components/views/discovery-view";
import {
  discoveryHomeCardsQuery,
  discoveryHomeFilterOptionsQuery,
  discoveryItemDetailQuery,
  discoverySectionPageQuery,
} from "@/lib/graphql/queries";
import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import { CATALOG_TITLES_REFRESH_EVENT } from "@/lib/events/catalog-titles";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useSearchContext } from "@/lib/context/search-context";
import { useTranslate } from "@/lib/context/translate-context";
import {
  uiSettingsInputFromSettings,
  useUiSettings,
} from "@/lib/context/ui-settings-context";
import type { LocaleCode } from "@/lib/i18n";
import { discoveryItemDisplayTitle } from "@/lib/utils/discovery-display";
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import {
  discoveryItemFacet,
  externalIdsFromDiscoverySignals,
} from "@/lib/utils/discovery-actions";
import {
  appendDiscoverySectionPage,
  discoveryBrowseSectionId,
  discoverySectionPagingFor,
  discoverySectionQueryKey,
  nextDiscoverySectionPage,
  startDiscoverySectionPaging,
  type DiscoverySectionPaging,
} from "@/lib/utils/discovery-section-paging";
import { discoverySectionIdFromPath } from "@/lib/utils/routing";
import {
  browserCacheStore,
  browserCacheZstdCodec,
} from "@/lib/utils/browser-cache";
import {
  discoveryHomeCacheKey,
  readCachedDiscoveryHome,
  writeCachedDiscoveryHome,
} from "@/lib/utils/discovery-home-cache";
import type {
  DiscoveryHomeCard,
  DiscoveryHomeFilterOptions,
  DiscoveryHomeFilters,
  DiscoveryHomeInput,
  DiscoveryHomePayload,
  DiscoveryHomeSection,
  DiscoveryItem,
  DiscoverySectionPagePayload,
  DiscoverySectionSort,
  ExternalId,
  Facet,
} from "@/lib/types";

type DiscoveryContainerProps = {
  userId: string | null | undefined;
  uiLanguage: LocaleCode;
  authorizationSignature: string;
  canManageTitle: boolean;
  canRequestMedia: boolean;
};

const DISCOVERY_HOME_LIMIT_PER_SECTION = 18;

const DISCOVERY_HOME_INPUT: DiscoveryHomeInput = {
  includePublic: true,
  includePersonalized: true,
  includeUnresolved: true,
  limitPerSection: DISCOVERY_HOME_LIMIT_PER_SECTION,
};

const DISCOVERY_FACETS: Facet[] = ["MOVIE", "SERIES", "ANIME"];

function externalIdsForDiscoveryItem(item: DiscoveryItem): ExternalId[] {
  return externalIdsFromDiscoverySignals(item);
}

function metadataResultForDiscoveryItem(
  item: DiscoveryItem,
): MetadataTvdbSearchItem {
  const externalIds = externalIdsForDiscoveryItem(item);
  return {
    smgId:
      Number(
        externalIds.find((externalId) => externalId.source === "smg")?.value,
      ) || null,
    tvdbId:
      externalIds.find((externalId) => externalId.source === "tvdb")?.value ??
      "",
    tmdbId:
      Number(
        externalIds.find((externalId) => externalId.source === "tmdb")?.value,
      ) || null,
    name: discoveryItemDisplayTitle(item),
    imdbId:
      externalIds.find((externalId) => externalId.source === "imdb")?.value ??
      null,
    externalIds,
    slug: null,
    type: item.contentType ?? item.targetKind,
    year: item.year,
    status: item.statusTags[0] ?? null,
    overview: item.overview,
    popularity: item.rankScore,
    posterUrl: item.posterUrl,
    backgroundUrl: item.backgroundUrl,
    language: null,
    runtimeMinutes: null,
    sortTitle: item.sortTitle,
    rating: item.rating,
    ratingSource: item.sources[0] ?? item.bestSource,
    externalRatings: item.externalRatings ?? [],
  };
}

export const DiscoveryContainer = memo(function DiscoveryContainer({
  userId,
  uiLanguage,
  authorizationSignature,
  canManageTitle,
  canRequestMedia,
}: DiscoveryContainerProps) {
  const t = useTranslate();
  const client = useClient();
  const setGlobalStatus = useGlobalStatus();
  const clientRef = useRef(client);
  const setGlobalStatusRef = useRef(setGlobalStatus);
  const tRef = useRef(t);
  const {
    addMetadataSearchResultToCatalog,
    catalogConfigLoading,
    catalogQualityProfileOptions,
    ensureCatalogConfigReady,
    librariesByFacet,
    requestMetadataSearchResult,
    requestableLibrariesByFacet,
    resolveDefaultQualityProfileIdForFacet,
    rootFoldersByFacet,
  } = useSearchContext();
  const manageableFacets = useMemo(
    () =>
      DISCOVERY_FACETS.filter(
        (facet) => (librariesByFacet[facet] ?? []).length > 0,
      ),
    [librariesByFacet],
  );
  const requestableFacets = useMemo(
    () =>
      DISCOVERY_FACETS.filter(
        (facet) => (requestableLibrariesByFacet[facet] ?? []).length > 0,
      ),
    [requestableLibrariesByFacet],
  );
  const [home, setHome] = useState<DiscoveryHomePayload | null>(null);
  // The home input the shown home answers; a refresh of it skips the cache.
  const shownHomeInputRef = useRef<DiscoveryHomeInput | null>(null);
  const [filters, setFilters] = useState<DiscoveryHomeFilters>({});
  const [filterOptions, setFilterOptions] =
    useState<DiscoveryHomeFilterOptions>({
      genres: [],
      themes: [],
      studioSlugs: [],
    });
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selectedItem, setSelectedItem] = useState<DiscoveryItem | null>(null);
  const [addDialogOpen, setAddDialogOpen] = useState(false);
  const [requestDialogOpen, setRequestDialogOpen] = useState(false);
  const refreshRequestIdRef = useRef(0);
  const filterOptionsRequestIdRef = useRef(0);
  const mountedRef = useRef(true);
  const scopeKeyRef = useRef<string | null>(null);
  const authorizationSignatureRef = useRef(authorizationSignature);
  const location = useLocation();
  const routeSectionId = discoverySectionIdFromPath(location.pathname);
  const sectionId = discoveryBrowseSectionId(routeSectionId, filters);
  const { uiSettings, uiSettingsLoaded, setUiSettings, saveUiSettings } =
    useUiSettings();
  const [sectionSort, setSectionSort] = useState<{
    sectionId: string | null;
    sort: DiscoverySectionSort;
  }>({ sectionId: null, sort: "RELEVANCE" });
  const [sectionIdentity, setSectionIdentity] = useState<{
    sectionId: string;
    section: DiscoveryHomeSection;
  } | null>(null);
  // Bumped to reload an open grid from its first page, e.g. after an add.
  const [sectionReload, setSectionReload] = useState(0);
  const [sectionPaging, setSectionPaging] = useState<
    DiscoverySectionPaging<DiscoveryHomeCard>
  >(() => startDiscoverySectionPaging(""));
  const sectionPagingRef = useRef(sectionPaging);
  const [sectionLoadingKey, setSectionLoadingKey] = useState<string | null>(
    null,
  );
  const sectionInFlightRef = useRef<string | null>(null);
  // The query whose last page request failed; automatic paging stops there
  // until "load more" retries it.
  const [sectionFailedKey, setSectionFailedKey] = useState<string | null>(null);

  useLayoutEffect(() => {
    authorizationSignatureRef.current = authorizationSignature;
    filterOptionsRequestIdRef.current += 1;
    setFilterOptions({
      genres: [],
      themes: [],
      studioSlugs: [],
    });
    setSelectedItem(null);
    setAddDialogOpen(false);
    setRequestDialogOpen(false);
  }, [authorizationSignature]);

  useEffect(() => {
    clientRef.current = client;
    setGlobalStatusRef.current = setGlobalStatus;
    tRef.current = t;
  }, [client, setGlobalStatus, t]);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      refreshRequestIdRef.current += 1;
    };
  }, []);

  const homeInput = useMemo<DiscoveryHomeInput>(
    () => ({ ...DISCOVERY_HOME_INPUT, filters }),
    [filters],
  );

  const refreshHome = useCallback(async () => {
    const requestId = refreshRequestIdRef.current + 1;
    refreshRequestIdRef.current = requestId;
    const scopeKey = JSON.stringify({
      userId,
      uiLanguage,
      authorizationSignature,
    });
    const sameScope = scopeKeyRef.current === scopeKey;
    scopeKeyRef.current = scopeKey;
    if (!mountedRef.current || refreshRequestIdRef.current !== requestId) {
      return;
    }
    if (!sameScope) {
      setHome(null);
      shownHomeInputRef.current = null;
    }
    setLoading(true);
    setError(null);
    const current = () =>
      mountedRef.current && refreshRequestIdRef.current === requestId;
    // The last home read draws at once; the server's answer replaces it only
    // when it differs, and is kept for the next visit.
    const cacheKey = discoveryHomeCacheKey(
      userId,
      uiLanguage,
      authorizationSignature,
      filters,
    );
    const store = cacheKey ? browserCacheStore() : null;
    let answered = false;
    let shownJson: string | null = null;
    if (store && cacheKey && shownHomeInputRef.current !== homeInput) {
      void readCachedDiscoveryHome(store, cacheKey, browserCacheZstdCodec).then(
        (cached) => {
          if (!cached || answered || !current()) return;
          shownJson = JSON.stringify(cached);
          shownHomeInputRef.current = homeInput;
          setHome(cached);
        },
      );
    }
    try {
      const { data, error: queryError } = await clientRef.current
        .query(
          discoveryHomeCardsQuery,
          { input: homeInput },
          { requestPolicy: "network-only" },
        )
        .toPromise();
      answered = true;
      if (queryError) {
        throw queryError;
      }
      if (!current()) {
        return;
      }
      const loaded = (data?.discoveryHomeCards ??
        null) as DiscoveryHomePayload | null;
      const loadedJson = JSON.stringify(loaded);
      shownHomeInputRef.current = homeInput;
      if (loadedJson !== shownJson) {
        setHome(loaded);
        if (store && cacheKey)
          window.setTimeout(() => {
            void writeCachedDiscoveryHome(
              store,
              cacheKey,
              loaded,
              browserCacheZstdCodec,
            );
          }, 0);
      }
    } catch (caught) {
      answered = true;
      if (!current()) {
        return;
      }
      const message =
        caught instanceof Error
          ? caught.message
          : tRef.current("discovery.failedToLoad");
      setError(message);
      setGlobalStatusRef.current(message, { level: "ERROR" });
    } finally {
      if (mountedRef.current && refreshRequestIdRef.current === requestId) {
        setLoading(false);
      }
    }
  }, [authorizationSignature, filters, homeInput, uiLanguage, userId]);

  const refresh = useCallback(async () => {
    if (sectionId !== null) {
      setSectionReload((current) => current + 1);
      return;
    }
    await refreshHome();
  }, [refreshHome, sectionId]);

  useEffect(() => {
    if (sectionId === null) {
      void refreshHome();
    }
  }, [refreshHome, sectionId]);

  const effectiveSectionSort: DiscoverySectionSort =
    sectionSort.sectionId === sectionId ? sectionSort.sort : "RELEVANCE";
  const hideOwned = uiSettings.discoveryGridHideOwned;
  const hideRequested = uiSettings.discoveryGridHideRequested;
  // Everything that decides the grid's membership and order, plus the viewer
  // scope and an explicit reload, so any change starts over at offset zero.
  const sectionQueryKey =
    sectionId === null
      ? null
      : `${discoverySectionQueryKey({
          sectionId,
          sort: effectiveSectionSort,
          filters,
          hideOwned,
          hideRequested,
        })}|${JSON.stringify([
          userId ?? null,
          uiLanguage,
          authorizationSignature,
          sectionReload,
        ])}`;
  const sectionQueryKeyRef = useRef(sectionQueryKey);
  useLayoutEffect(() => {
    sectionQueryKeyRef.current = sectionQueryKey;
  }, [sectionQueryKey]);

  const loadNextSectionPage = useCallback(async () => {
    if (sectionId === null || sectionQueryKey === null) {
      return;
    }
    const queryKey = sectionQueryKey;
    const state = discoverySectionPagingFor(sectionPagingRef.current, queryKey);
    const next = nextDiscoverySectionPage(state);
    if (!next) {
      return;
    }
    const flightKey = `${queryKey}@${next.offset}`;
    if (sectionInFlightRef.current === flightKey) {
      return;
    }
    sectionInFlightRef.current = flightKey;
    setSectionLoadingKey(queryKey);
    setSectionFailedKey(null);
    try {
      const { data, error: queryError } = await clientRef.current
        .query(
          discoverySectionPageQuery,
          {
            input: {
              sectionId,
              sectionRailLimit: DISCOVERY_HOME_LIMIT_PER_SECTION,
              includeUnresolved: DISCOVERY_HOME_INPUT.includeUnresolved,
              includeOwned: !hideOwned,
              hideRequested,
              homeFilters: filters,
              sort: effectiveSectionSort,
              limit: next.limit,
              offset: next.offset,
            },
          },
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (queryError) {
        throw queryError;
      }
      if (!mountedRef.current || sectionQueryKeyRef.current !== queryKey) {
        return;
      }
      const payload = (data?.discoveryItems ??
        null) as DiscoverySectionPagePayload | null;
      const pageSection = payload?.section ?? null;
      const appended = appendDiscoverySectionPage(
        discoverySectionPagingFor(sectionPagingRef.current, queryKey),
        {
          queryKey,
          offset: next.offset,
          items: pageSection?.items ?? [],
          totalCount: payload?.totalCount ?? 0,
          generationIds: pageSection?.generationIds ?? [],
        },
      );
      sectionPagingRef.current = appended;
      setSectionPaging(appended);
      if (next.offset > 0 && appended.offset === 0) {
        // The section was rebuilt between pages; reload it from the top.
        setSectionReload((current) => current + 1);
        return;
      }
      if (pageSection) {
        setSectionIdentity({
          sectionId,
          section: { ...pageSection, items: [] },
        });
      }
    } catch (caught) {
      if (!mountedRef.current || sectionQueryKeyRef.current !== queryKey) {
        return;
      }
      setGlobalStatusRef.current(
        userFacingGraphQlErrorMessage(
          caught,
          tRef.current("discovery.failedToLoad"),
          tRef.current,
        ),
        { level: "ERROR" },
      );
      setSectionFailedKey(queryKey);
    } finally {
      if (sectionInFlightRef.current === flightKey) {
        sectionInFlightRef.current = null;
        setSectionLoadingKey((current) =>
          current === queryKey ? null : current,
        );
      }
    }
  }, [
    effectiveSectionSort,
    filters,
    hideOwned,
    hideRequested,
    sectionId,
    sectionQueryKey,
  ]);

  // A new query (section, sort, filters, toggles, scope or reload) clears the
  // grid and asks for its first page. Toggles wait for the saved settings so
  // the first page is not fetched under the defaults and then thrown away.
  useEffect(() => {
    if (sectionQueryKey === null || !uiSettingsLoaded) {
      return;
    }
    if (sectionPagingRef.current.queryKey !== sectionQueryKey) {
      const fresh =
        startDiscoverySectionPaging<DiscoveryHomeCard>(sectionQueryKey);
      sectionPagingRef.current = fresh;
      setSectionPaging(fresh);
    }
    void loadNextSectionPage();
  }, [loadNextSectionPage, sectionQueryKey, uiSettingsLoaded]);

  const saveGridPreference = useCallback(
    async (
      change: Partial<
        Pick<
          typeof uiSettings,
          "discoveryGridHideOwned" | "discoveryGridHideRequested"
        >
      >,
    ) => {
      const previous = uiSettings;
      setUiSettings({ ...uiSettings, ...change });
      try {
        const result = await saveUiSettings((latest) => ({
          ...uiSettingsInputFromSettings(latest),
          ...change,
        }));
        if (result.error || !result.data?.setMyUiSettings) {
          throw (
            result.error ??
            new Error(t("discovery.section.preferenceSaveFailed"))
          );
        }
        setUiSettings(result.data.setMyUiSettings);
      } catch (caught) {
        setUiSettings(previous);
        setGlobalStatus(
          userFacingGraphQlErrorMessage(
            caught,
            t("discovery.section.preferenceSaveFailed"),
            t,
          ),
          { level: "ERROR" },
        );
      }
    },
    [saveUiSettings, setGlobalStatus, setUiSettings, t, uiSettings],
  );

  const handleSectionSortChange = useCallback(
    (sort: DiscoverySectionSort) => setSectionSort({ sectionId, sort }),
    [sectionId],
  );
  const handleHideOwnedChange = useCallback(
    (discoveryGridHideOwned: boolean) =>
      void saveGridPreference({ discoveryGridHideOwned }),
    [saveGridPreference],
  );
  const handleHideRequestedChange = useCallback(
    (discoveryGridHideRequested: boolean) =>
      void saveGridPreference({ discoveryGridHideRequested }),
    [saveGridPreference],
  );
  const handleLoadMore = useCallback(() => {
    void loadNextSectionPage();
  }, [loadNextSectionPage]);

  const sectionGrid = useMemo<DiscoverySectionGridModel | null>(() => {
    if (sectionId === null || sectionQueryKey === null) {
      return null;
    }
    const paging = discoverySectionPagingFor(sectionPaging, sectionQueryKey);
    const pageFailed = sectionFailedKey === sectionQueryKey;
    return {
      sectionId,
      section:
        sectionIdentity?.sectionId === sectionId
          ? sectionIdentity.section
          : null,
      items: paging.items,
      totalCount: paging.totalCount,
      loadingPage:
        sectionLoadingKey === sectionQueryKey ||
        (paging.totalCount === null && !paging.exhausted && !pageFailed),
      exhausted: paging.exhausted,
      pageFailed,
      sort: effectiveSectionSort,
      hideOwned,
      hideRequested,
      onSortChange: handleSectionSortChange,
      onHideOwnedChange: handleHideOwnedChange,
      onHideRequestedChange: handleHideRequestedChange,
      onLoadMore: handleLoadMore,
    };
  }, [
    effectiveSectionSort,
    handleHideOwnedChange,
    handleHideRequestedChange,
    handleLoadMore,
    handleSectionSortChange,
    hideOwned,
    hideRequested,
    sectionId,
    sectionFailedKey,
    sectionIdentity,
    sectionLoadingKey,
    sectionPaging,
    sectionQueryKey,
  ]);

  useEffect(() => {
    // A global-search add leaves the panel open over this view, so the rails
    // have to hear about the new title instead of waiting for a remount.
    const handleCatalogTitlesRefresh = () => {
      void refresh();
    };

    window.addEventListener(
      CATALOG_TITLES_REFRESH_EVENT,
      handleCatalogTitlesRefresh,
    );
    return () =>
      window.removeEventListener(
        CATALOG_TITLES_REFRESH_EVENT,
        handleCatalogTitlesRefresh,
      );
  }, [refresh]);

  useEffect(() => {
    const requestId = filterOptionsRequestIdRef.current + 1;
    filterOptionsRequestIdRef.current = requestId;
    void client
      .query(
        discoveryHomeFilterOptionsQuery,
        {
          input: {
            includePublic: DISCOVERY_HOME_INPUT.includePublic,
            includePersonalized: DISCOVERY_HOME_INPUT.includePersonalized,
            includeUnresolved: DISCOVERY_HOME_INPUT.includeUnresolved,
          },
        },
        { requestPolicy: "network-only" },
      )
      .toPromise()
      .then(({ data, error: queryError }) => {
        if (
          queryError ||
          !mountedRef.current ||
          filterOptionsRequestIdRef.current !== requestId
        ) {
          return;
        }
        const nextOptions = data?.discoveryHomeFilterOptions as
          DiscoveryHomeFilterOptions | null | undefined;
        if (nextOptions) {
          setFilterOptions(nextOptions);
        }
      });
  }, [authorizationSignature, client, uiLanguage, userId]);

  const selectedFacet = selectedItem
    ? (discoveryItemFacet(selectedItem) ?? "MOVIE")
    : "MOVIE";
  const selectedResult = selectedItem
    ? metadataResultForDiscoveryItem(selectedItem)
    : EMPTY_SEARCH_RESULT;

  const handleAction = useCallback(
    async (item: DiscoveryHomeCard) => {
      const actionAuthorizationSignature = authorizationSignature;
      if (item.ownedInInput) {
        return;
      }
      const facet = discoveryItemFacet(item);
      if (!facet) {
        setGlobalStatus(t("status.apiError"), { level: "ERROR" });
        return;
      }
      const canManageFacet = (librariesByFacet[facet] ?? []).length > 0;
      const canRequestFacet =
        (requestableLibrariesByFacet[facet] ?? []).length > 0;
      if (!canManageFacet && !canRequestFacet) {
        setGlobalStatus(t("status.permissionDenied"));
        return;
      }

      try {
        await ensureCatalogConfigReady(facet);
        if (
          authorizationSignatureRef.current !== actionAuthorizationSignature
        ) {
          return;
        }
        const { data, error: detailError } = await client
          .query(
            discoveryItemDetailQuery,
            {
              input: {
                targetKey: item.targetKey,
                includeUnresolved: true,
              },
            },
            { requestPolicy: "network-only" },
          )
          .toPromise();
        if (
          authorizationSignatureRef.current !== actionAuthorizationSignature
        ) {
          return;
        }
        if (detailError) {
          throw detailError;
        }
        const detail = data?.discoveryItemDetail as
          DiscoveryItem | null | undefined;
        if (!detail) {
          throw new Error(t("status.permissionDenied"));
        }
        const detailFacet = discoveryItemFacet(detail);
        if (!detailFacet || detailFacet !== facet) {
          throw new Error(t("status.permissionDenied"));
        }
        const canManageDetailFacet =
          (librariesByFacet[detailFacet] ?? []).length > 0;
        const canRequestDetailFacet =
          (requestableLibrariesByFacet[detailFacet] ?? []).length > 0;
        if (!canManageDetailFacet && !canRequestDetailFacet) {
          throw new Error(t("status.permissionDenied"));
        }
        setSelectedItem(detail);
        if (canManageDetailFacet) {
          setAddDialogOpen(true);
        } else {
          setRequestDialogOpen(true);
        }
      } catch (caught) {
        setGlobalStatus(
          caught instanceof Error ? caught.message : t("status.apiError"),
          { level: "ERROR" },
        );
      }
    },
    [
      authorizationSignature,
      client,
      ensureCatalogConfigReady,
      librariesByFacet,
      requestableLibrariesByFacet,
      setGlobalStatus,
      t,
    ],
  );

  const handleAddDialogOpenChange = useCallback((open: boolean) => {
    setAddDialogOpen(open);
    if (!open) {
      setSelectedItem(null);
    }
  }, []);

  const handleRequestDialogOpenChange = useCallback((open: boolean) => {
    setRequestDialogOpen(open);
    if (!open) {
      setSelectedItem(null);
    }
  }, []);

  return (
    <>
      <DiscoveryView
        home={sectionGrid ? null : home}
        sectionGrid={sectionGrid}
        loading={loading}
        error={error}
        manageableFacets={manageableFacets}
        requestableFacets={requestableFacets}
        filterOptions={filterOptions}
        onFiltersChange={setFilters}
        onRefresh={refresh}
        onAction={handleAction}
      />
      {canManageTitle && manageableFacets.length > 0 ? (
        <AddToCatalogDialog
          open={addDialogOpen}
          onOpenChange={handleAddDialogOpenChange}
          result={selectedResult}
          facet={selectedFacet}
          catalogQualityProfileOptions={catalogQualityProfileOptions}
          catalogConfigLoading={catalogConfigLoading}
          defaultQualityProfileId={resolveDefaultQualityProfileIdForFacet(
            selectedFacet,
          )}
          manageableLibraries={librariesByFacet[selectedFacet] ?? []}
          rootFolderOptions={rootFoldersByFacet[selectedFacet] ?? []}
          onAdd={(result, facet, options) =>
            // The rails reload on the catalog-titles event, same as an add made
            // from global search.
            addMetadataSearchResultToCatalog(result, facet, options)
          }
        />
      ) : null}
      {canRequestMedia && requestableFacets.length > 0 ? (
        <RequestMediaDialog
          open={requestDialogOpen}
          onOpenChange={handleRequestDialogOpenChange}
          result={selectedResult}
          facet={selectedFacet}
          requestableLibraries={
            requestableLibrariesByFacet[selectedFacet] ?? []
          }
          qualityProfileOptions={catalogQualityProfileOptions}
          onRequest={async (result, facet, options) => {
            const accepted = await requestMetadataSearchResult(
              result,
              facet,
              options,
            );
            if (accepted) {
              await refresh();
            }
            return accepted;
          }}
        />
      ) : null}
    </>
  );
});

import {
  memo,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useClient } from "urql";
import {
  AddToCatalogDialog,
  EMPTY_SEARCH_RESULT,
} from "@/components/root/add-to-catalog-dialog";
import { RequestMediaDialog } from "@/components/root/request-media-dialog";
import { DiscoveryView } from "@/components/views/discovery-view";
import {
  discoveryHomeCardsQuery,
  discoveryHomeFilterOptionsQuery,
  discoveryItemDetailQuery,
} from "@/lib/graphql/queries";
import { CATALOG_TITLES_REFRESH_EVENT } from "@/lib/events/catalog-titles";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useSearchContext } from "@/lib/context/search-context";
import { useTranslate } from "@/lib/context/translate-context";
import type { LocaleCode } from "@/lib/i18n";
import { discoveryItemDisplayTitle } from "@/lib/utils/discovery-display";
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import {
  discoveryItemFacet,
  externalIdsFromDiscoverySignals,
} from "@/lib/utils/discovery-actions";
import type {
  DiscoveryHomeCard,
  DiscoveryHomeFilterOptions,
  DiscoveryHomeFilters,
  DiscoveryHomeInput,
  DiscoveryHomePayload,
  DiscoveryItem,
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

const DISCOVERY_HOME_INPUT: DiscoveryHomeInput = {
  includePublic: true,
  includePersonalized: true,
  includeUnresolved: true,
  limitPerSection: 18,
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
    smgId: Number(
      externalIds.find((externalId) => externalId.source === "smg")?.value,
    ) || null,
    tvdbId:
      externalIds.find((externalId) => externalId.source === "tvdb")?.value ?? "",
    tmdbId: Number(
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
  const [filters, setFilters] = useState<DiscoveryHomeFilters>({});
  const [filterOptions, setFilterOptions] = useState<DiscoveryHomeFilterOptions>({
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

  const refresh = useCallback(async () => {
    const requestId = refreshRequestIdRef.current + 1;
    refreshRequestIdRef.current = requestId;
    const scopeKey = JSON.stringify({ userId, uiLanguage, authorizationSignature });
    const sameScope = scopeKeyRef.current === scopeKey;
    scopeKeyRef.current = scopeKey;
    if (!mountedRef.current || refreshRequestIdRef.current !== requestId) {
      return;
    }
    if (!sameScope) {
      setHome(null);
    }
    setLoading(true);
    setError(null);
    try {
      const { data, error: queryError } = await clientRef.current
        .query(
          discoveryHomeCardsQuery,
          { input: homeInput },
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (queryError) {
        throw queryError;
      }
      if (!mountedRef.current || refreshRequestIdRef.current !== requestId) {
        return;
      }
      setHome((data?.discoveryHomeCards ?? null) as DiscoveryHomePayload | null);
    } catch (caught) {
      if (!mountedRef.current || refreshRequestIdRef.current !== requestId) {
        return;
      }
      const message =
        caught instanceof Error
          ? caught.message
          : tRef.current("discovery.failedToLoad");
      setError(message);
      setGlobalStatusRef.current(message);
    } finally {
      if (mountedRef.current && refreshRequestIdRef.current === requestId) {
        setLoading(false);
      }
    }
  }, [authorizationSignature, homeInput, uiLanguage, userId]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

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
          | DiscoveryHomeFilterOptions
          | null
          | undefined;
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
        setGlobalStatus(t("status.apiError"));
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
        const detail = data?.discoveryItemDetail as DiscoveryItem | null | undefined;
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
        home={home}
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
          requestableLibraries={requestableLibrariesByFacet[selectedFacet] ?? []}
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

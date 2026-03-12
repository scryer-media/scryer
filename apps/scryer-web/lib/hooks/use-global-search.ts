import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useClient } from "urql";
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import type { AdminSetting } from "@/lib/types/admin-settings";
import type { Facet, Release, TitleRecord } from "@/lib/types";
import type { ViewCategoryId } from "@/lib/types/quality-profiles";
import type { LocaleCode } from "@/lib/i18n";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import {
  mediaSettingsInitQuery,
  metadataMovieQuery,
  metadataSeriesQuery,
  searchMetadataMultiQuery,
  searchMetadataQuery,
  searchQuery,
  titlesQuery,
} from "@/lib/graphql/queries";
import { scryerFetch } from "@/lib/graphql/urql-client";
import { addTitleMutation } from "@/lib/graphql/mutations";
import {
  QUALITY_PROFILE_CATALOG_KEY,
  QUALITY_PROFILE_ID_KEY,
  QUALITY_PROFILE_INHERIT_VALUE,
} from "@/lib/constants/settings";
import {
  coerceProfileSetting,
  resolveQualityProfileCatalogState,
} from "@/lib/utils/quality-profiles";
import { getSettingDisplayValue } from "@/lib/utils/settings";
import { FACET_REGISTRY, facetById } from "@/lib/facets/registry";

export type MetadataSearchResults = Record<string, MetadataTvdbSearchItem[]>;

type NzbSearchOptions = {
  imdbId?: string | null;
  tvdbId?: string | null;
  limit?: number;
};

export type CatalogQualityProfileOption = {
  id: string;
  name: string;
};

export type MetadataCatalogMonitorType =
  | "monitored"
  | "unmonitored"
  | "futureEpisodes"
  | "missingAndFutureEpisodes"
  | "allEpisodes"
  | "none";

export type MetadataCatalogAddOptions = {
  qualityProfileId: string;
  seasonFolder: boolean;
  monitorType: MetadataCatalogMonitorType;
  minAvailability?: string;
};

function isMetadataEmpty(results: MetadataSearchResults): boolean {
  return Object.values(results).every((arr) => arr.length === 0);
}

const AUTOCOMPLETE_MIN_CHARS = 2;
const AUTOCOMPLETE_DEBOUNCE_MS = 250;
const AUTOCOMPLETE_LIMIT = 10;

type UseGlobalSearchArgs = {
  queueFacet: Facet;
  uiLanguage: LocaleCode;
  onCatalogChanged?: () => void;
};

export interface UseGlobalSearchResult {
  globalSearch: string;
  setGlobalSearch: (value: string) => void;
  globalSearchInputRef: React.RefObject<HTMLInputElement | null>;
  searching: boolean;
  catalogSearchLoading: boolean;
  metadataSearchLoading: boolean;
  searchResults: Release[];
  tvdbCandidates: MetadataTvdbSearchItem[];
  selectedTvdbId: string | null;
  selectedTvdb: MetadataTvdbSearchItem | null;
  runNzbSearch: (params: {
    query: string;
    imdbId: string | null;
    tvdbId: string | null;
    category: string | null;
  }) => Promise<Release[]>;
  runTvdbSearch: (query: string) => Promise<MetadataTvdbSearchItem[]>;
  handleGlobalSearchSubmit: () => Promise<void>;
  selectTvdbCandidate: (candidate: MetadataTvdbSearchItem) => void;
  searchNzbForSelectedTvdb: () => Promise<void>;
  setSelectedTvdbId: (value: string | null) => void;
  runSearch: (
    query: string,
    category?: string | null,
    options?: NzbSearchOptions,
  ) => Promise<Release[]>;
  setSearching: (value: boolean) => void;
  setTvdbCandidates: (value: MetadataTvdbSearchItem[]) => void;
  setSearchResults: (value: Release[]) => void;
  catalogSearchResults: TitleRecord[];
  metadataSearchResults: MetadataSearchResults;
  isGlobalSearchPanelOpen: boolean;
  openGlobalSearchPanel: () => void;
  closeGlobalSearchPanel: () => void;
  catalogQualityProfileOptions: CatalogQualityProfileOption[];
  resolveDefaultQualityProfileIdForFacet: (facet: Facet) => string;
  addMetadataSearchResultToCatalog: (
    result: MetadataTvdbSearchItem,
    facet: Facet,
    options: MetadataCatalogAddOptions,
  ) => Promise<string | null>;
  isMetadataSearchResultInCatalog: (
    facet: Facet,
    result: MetadataTvdbSearchItem,
  ) => boolean;
  queueFacet: Facet;
  setQueueFacet: (value: Facet) => void;
  catalogChangeSignal: number;
}

function monitorTypeToMonitored(monitorType: MetadataCatalogMonitorType): boolean {
  return monitorType !== "unmonitored" && monitorType !== "none";
}

export function useGlobalSearch({
  queueFacet: initialQueueFacet,
  uiLanguage,
  onCatalogChanged,
}: UseGlobalSearchArgs): UseGlobalSearchResult {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const [queueFacet, setQueueFacet] = useState<Facet>(initialQueueFacet);
  const [catalogChangeSignal, setCatalogChangeSignal] = useState(0);
  const sortByRelevance = useCallback((results: MetadataTvdbSearchItem[], query: string) => {
    const q = query.trim().toLowerCase();

    function score(item: MetadataTvdbSearchItem): number {
      const name = (item.name || "").toLowerCase();
      const pop = Math.max(item.popularity ?? 0, 1);
      if (name === q) return 1e9 + pop;
      if (name.startsWith(q)) return pop * 5;
      if (name.includes(q)) return pop * 3;
      return pop;
    }

    return [...results].sort((left, right) => {
      const ls = score(left);
      const rs = score(right);
      if (ls !== rs) return rs - ls;
      return (right.year ?? 0) - (left.year ?? 0);
    });
  }, []);

  const [globalSearch, setGlobalSearch] = useState("");
  const globalSearchInputRef = useRef<HTMLInputElement>(null);
  const [searching, setSearching] = useState(false);
  const [catalogSearchLoading, setCatalogSearchLoading] = useState(false);
  const [metadataSearchLoading, setMetadataSearchLoading] = useState(false);
  const [searchResults, setSearchResults] = useState<Release[]>([]);
  const [tvdbCandidates, setTvdbCandidates] = useState<MetadataTvdbSearchItem[]>([]);
  const [selectedTvdbId, setSelectedTvdbId] = useState<string | null>(null);
  const [catalogSearchResults, setCatalogSearchResults] = useState<TitleRecord[]>([]);
  const [metadataSearchResults, setMetadataSearchResults] = useState<MetadataSearchResults>(
    () => Object.fromEntries(FACET_REGISTRY.map((f) => [f.metadataKey, []])),
  );
  const [catalogQualityProfileOptions, setCatalogQualityProfileOptions] = useState<
    CatalogQualityProfileOption[]
  >([]);
  const [globalQualityProfileId, setGlobalQualityProfileId] = useState<string>(
    QUALITY_PROFILE_INHERIT_VALUE,
  );
  const [categoryQualityProfileOverrides, setCategoryQualityProfileOverrides] = useState<
    Record<ViewCategoryId, string>
  >(
    () => Object.fromEntries(FACET_REGISTRY.map((f) => [f.scopeId, QUALITY_PROFILE_INHERIT_VALUE])) as Record<ViewCategoryId, string>,
  );
  const [isGlobalSearchPanelOpen, setIsGlobalSearchPanelOpen] = useState(false);
  const autocompleteRequestId = useRef(0);
  const autocompleteAbortRef = useRef<AbortController | null>(null);

  const isTitleInCatalogByFacet = useMemo(() => {
    const buckets = Object.fromEntries(
      FACET_REGISTRY.map((f) => [f.id, new Set<string>()]),
    ) as Record<Facet, Set<string>>;

    for (const title of catalogSearchResults) {
      const facet: Facet = title.facet === "movie" ? "movie" : title.facet === "anime" ? "anime" : "tv";
      const tvdbIds = title.externalIds
        .filter((externalId) => externalId.source.toLowerCase() === "tvdb")
        .map((externalId) => externalId.value.trim())
        .filter(Boolean);

      tvdbIds.forEach((id) => buckets[facet].add(id));
    }

    return buckets;
  }, [catalogSearchResults]);

  const selectedTvdb = useMemo(() => {
    if (!selectedTvdbId) {
      return null;
    }
    return tvdbCandidates.find((item) => String(item.tvdbId) === selectedTvdbId) ?? null;
  }, [selectedTvdbId, tvdbCandidates]);

  const catalogQualityProfileIdSet = useMemo(
    () => new Set(catalogQualityProfileOptions.map((profile) => profile.id)),
    [catalogQualityProfileOptions],
  );

  const resolveDefaultQualityProfileIdForFacet = useCallback(
    (facet: Facet) => {
      const scopeId = facetById(facet)?.scopeId ?? "movie";
      const overrideProfileId = coerceProfileSetting(
        categoryQualityProfileOverrides[scopeId],
      );
      if (
        overrideProfileId &&
        overrideProfileId !== QUALITY_PROFILE_INHERIT_VALUE &&
        catalogQualityProfileIdSet.has(overrideProfileId)
      ) {
        return overrideProfileId;
      }

      const normalizedGlobalProfileId = coerceProfileSetting(globalQualityProfileId);
      if (
        normalizedGlobalProfileId &&
        normalizedGlobalProfileId !== QUALITY_PROFILE_INHERIT_VALUE &&
        catalogQualityProfileIdSet.has(normalizedGlobalProfileId)
      ) {
        return normalizedGlobalProfileId;
      }

      return catalogQualityProfileOptions[0]?.id ?? "";
    },
    [
      catalogQualityProfileIdSet,
      catalogQualityProfileOptions,
      categoryQualityProfileOverrides,
      globalQualityProfileId,
    ],
  );

  const refreshCatalogQualityProfileState = useCallback(async () => {
    try {
      const { data, error } = await client.query(mediaSettingsInitQuery, {}).toPromise();
      if (error) throw error;

      const profileCatalogRecord = data.systemSettings.items.find(
        (item: AdminSetting) => item.keyName === QUALITY_PROFILE_CATALOG_KEY,
      );
      const qualityCatalogRaw =
        data.systemSettings.qualityProfiles ?? getSettingDisplayValue(profileCatalogRecord);
      const parsedProfiles = resolveQualityProfileCatalogState(qualityCatalogRaw).profiles.map(
        (profile) => ({
          id: profile.id.trim(),
          name: profile.name.trim() || profile.id.trim(),
        }),
      );

      setCatalogQualityProfileOptions((previous) =>
        previous.length === parsedProfiles.length &&
        previous.every(
          (item, index) =>
            item.id === parsedProfiles[index]?.id &&
            item.name === parsedProfiles[index]?.name,
        )
          ? previous
          : parsedProfiles,
      );

      const globalProfileRecord = data.systemSettings.items.find(
        (item: AdminSetting) => item.keyName === QUALITY_PROFILE_ID_KEY,
      );
      const nextGlobalProfileId = coerceProfileSetting(
        getSettingDisplayValue(globalProfileRecord),
      );
      setGlobalQualityProfileId((previous) =>
        previous === nextGlobalProfileId ? previous : nextGlobalProfileId,
      );

      const movieProfileRecord = data.movieSettings.items.find(
        (item: AdminSetting) => item.keyName === QUALITY_PROFILE_ID_KEY,
      );
      const seriesProfileRecord = data.seriesSettings.items.find(
        (item: AdminSetting) => item.keyName === QUALITY_PROFILE_ID_KEY,
      );
      const animeProfileRecord = data.animeSettings.items.find(
        (item: AdminSetting) => item.keyName === QUALITY_PROFILE_ID_KEY,
      );

      const nextOverrides: Record<ViewCategoryId, string> = {
        movie: coerceProfileSetting(getSettingDisplayValue(movieProfileRecord)),
        series: coerceProfileSetting(getSettingDisplayValue(seriesProfileRecord)),
        anime: coerceProfileSetting(getSettingDisplayValue(animeProfileRecord)),
      };
      setCategoryQualityProfileOverrides((previous) =>
        previous.movie === nextOverrides.movie &&
        previous.series === nextOverrides.series &&
        previous.anime === nextOverrides.anime
          ? previous
          : nextOverrides,
      );
    } catch {
      // ignore settings fetch failures here; search remains functional
    }
  }, [client]);

  useEffect(() => {
    void refreshCatalogQualityProfileState();
  }, [refreshCatalogQualityProfileState]);

  const isMetadataSearchResultInAnyCatalog = useCallback(
    (result: MetadataTvdbSearchItem) => {
      const tvdbId = String(result.tvdbId).trim();
      if (!tvdbId) return false;
      return Object.values(isTitleInCatalogByFacet).some((bucket) => bucket.has(tvdbId));
    },
    [isTitleInCatalogByFacet],
  );

  const isMetadataSearchResultInCatalog = useCallback(
    (_facet: Facet, result: MetadataTvdbSearchItem) => isMetadataSearchResultInAnyCatalog(result),
    [isMetadataSearchResultInAnyCatalog],
  );

  const filterMetadataSearchResults = useCallback(
    (facet: Facet, results: MetadataTvdbSearchItem[]) =>
      results.filter((result) => !isMetadataSearchResultInCatalog(facet, result)),
    [isMetadataSearchResultInCatalog],
  );

  const mapFacetToTvdbType = useCallback((facet: Facet) => {
    return facetById(facet)?.tvdbSearchType ?? "series";
  }, []);

  const resolveCatalogPosterUrl = useCallback(
    async (title: TitleRecord): Promise<TitleRecord> => {
      if (title.posterUrl) {
        return title;
      }

      const tvdbId = title.externalIds
        .find((externalId) => externalId.source.toLowerCase() === "tvdb")
        ?.value.trim();
      if (!tvdbId) {
        return title;
      }

      try {
        if (title.facet === "movie") {
          const tvdbIdNum = parseInt(tvdbId, 10);
          if (isNaN(tvdbIdNum)) return title;
          const { data, error } = await client.query(metadataMovieQuery, {
            tvdbId: tvdbIdNum,
            language: uiLanguage,
          }).toPromise();
          if (error || !data?.metadataMovie?.posterUrl) return title;
          return { ...title, posterUrl: data.metadataMovie.posterUrl };
        }

        const { data, error } = await client.query(metadataSeriesQuery, {
          id: tvdbId,
          includeEpisodes: false,
          language: uiLanguage,
        }).toPromise();
        if (error || !data?.metadataSeries?.posterUrl) return title;
        return { ...title, posterUrl: data.metadataSeries.posterUrl };
      } catch {
        return title;
      }
    },
    [client, uiLanguage],
  );

  const emptyMetadataSearchResults = useMemo<MetadataSearchResults>(
    () => Object.fromEntries(FACET_REGISTRY.map((f) => [f.metadataKey, []])),
    [],
  );

  const runNzbSearch = useCallback(
    async ({
      query,
      imdbId,
      tvdbId,
      category,
    }: {
      query: string;
      imdbId: string | null;
      tvdbId: string | null;
      category: string | null;
    }) => {
      setSearching(true);
      const extraMessage = category ? ` ${t("label.category")}: ${category}` : "";
      setGlobalStatus(
        t("status.searchingNzb", {
          query,
          category: extraMessage,
        }),
      );
      try {
        const { data: searchData, error: searchError } = await client.query(searchQuery, {
          query,
          imdbId,
          tvdbId,
          category,
          limit: 15,
        }).toPromise();
        if (searchError) throw searchError;
        setSearchResults(searchData.searchIndexers || []);
        setGlobalStatus(
          t("status.foundNzb", { count: searchData.searchIndexers?.length || 0 }),
        );
        return searchData.searchIndexers || [];
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.apiError"));
        setSearchResults([]);
        return [];
      } finally {
        setSearching(false);
      }
    },
    [client, t, setGlobalStatus],
  );

  const runTvdbSearch = useCallback(
    async (query: string) => {
      setGlobalStatus(t("status.searchingTvdb", { query }));
      try {
        const { data: searchData, error: searchError } = await client.query(searchMetadataQuery, {
          query,
          type: mapFacetToTvdbType(queueFacet),
          limit: 12,
          language: uiLanguage,
        }).toPromise();
        if (searchError) throw searchError;
        const matches = sortByRelevance(
          (searchData.searchMetadata || []).filter(
            (item: MetadataTvdbSearchItem) => !isMetadataSearchResultInAnyCatalog(item),
          ),
          query,
        );
        setTvdbCandidates(matches);
        setSelectedTvdbId(null);
        setSearchResults([]);
        setGlobalStatus(matches.length ? t("status.foundTvdb", { count: matches.length }) : t("status.nothingFound"));
        return matches;
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.apiError"));
        setTvdbCandidates([]);
        setSelectedTvdbId(null);
        setSearchResults([]);
        return [];
      }
    },
    [client, isMetadataSearchResultInAnyCatalog, mapFacetToTvdbType, queueFacet, setGlobalStatus, t, uiLanguage, sortByRelevance],
  );

  const runMetadataAutocomplete = useCallback(
    async (query: string) => {
      const trimmed = query.trim();
      if (!trimmed) {
        setCatalogSearchLoading(false);
        setMetadataSearchLoading(false);
        setCatalogSearchResults((previous) => (previous.length === 0 ? previous : []));
        setMetadataSearchResults((previous) => {
          if (isMetadataEmpty(previous)) {
            return previous;
          }
          return emptyMetadataSearchResults;
        });
        setGlobalStatus(t("label.ready"));
        return;
      }

      const requestId = ++autocompleteRequestId.current;
      setSearching(true);
      setCatalogSearchLoading(true);
      setMetadataSearchLoading(true);

      // Abort previous in-flight autocomplete HTTP requests so cancellation
      // propagates through Rust all the way to the SMG database query.
      autocompleteAbortRef.current?.abort();
      const abortController = new AbortController();
      autocompleteAbortRef.current = abortController;
      const { signal } = abortController;
      const abortableFetch: typeof fetch = (input, init) =>
        scryerFetch(input, { ...init, signal });

      // Fire both queries in parallel but render each result as it arrives
      // so the fast catalog query populates immediately while the metadata
      // spinner keeps spinning.

      const catalogPromise = client.query(titlesQuery, {
        query: trimmed,
        facet: null,
      }, { fetch: abortableFetch }).toPromise()
        .then(async ({ data, error }) => {
          if (error) throw error;
          if (requestId !== autocompleteRequestId.current) return;
          const catalogEntries = data.titles || [];
          const enriched = await Promise.all(
            catalogEntries.map((title: TitleRecord) => resolveCatalogPosterUrl(title)),
          );
          if (requestId !== autocompleteRequestId.current) return;
          const next = enriched.slice(0, AUTOCOMPLETE_LIMIT);
          setCatalogSearchResults((previous) =>
            previous.length === next.length &&
            previous.every(
              (item, index) =>
                item.id === next[index]?.id &&
                (item.posterUrl ?? null) === (next[index]?.posterUrl ?? null),
            )
              ? previous
              : next,
          );
        })
        .finally(() => {
          if (requestId !== autocompleteRequestId.current) return;
          setCatalogSearchLoading(false);
        });

      const metadataPromise = client.query(searchMetadataMultiQuery, {
        query: trimmed,
        limit: AUTOCOMPLETE_LIMIT,
        language: uiLanguage,
      }, { fetch: abortableFetch }).toPromise()
        .then(({ data, error }) => {
          if (error) throw error;
          if (requestId !== autocompleteRequestId.current) return;
          const multi = data.searchMetadataMulti ?? { movies: [], series: [], anime: [] };
          const movieResults = sortByRelevance(
            filterMetadataSearchResults("movie", (multi.movies || []) as MetadataTvdbSearchItem[]),
            trimmed,
          );
          const animeResults = sortByRelevance(
            filterMetadataSearchResults("anime", (multi.anime || []) as MetadataTvdbSearchItem[]),
            trimmed,
          );
          const animeTvdbIds = new Set(
            animeResults.map((item) => String(item.tvdbId).trim()),
          );
          const seriesResults = sortByRelevance(
            filterMetadataSearchResults("tv", (multi.series || []) as MetadataTvdbSearchItem[]).filter(
              (item) => !animeTvdbIds.has(String(item.tvdbId).trim()),
            ),
            trimmed,
          );
          const nextMetadata: MetadataSearchResults = {
            movie: movieResults,
            series: seriesResults,
            anime: animeResults,
          };
          setMetadataSearchResults((previous) => {
            const unchanged = Object.keys(nextMetadata).every((key) => {
              const prev = previous[key] ?? [];
              const next = nextMetadata[key] ?? [];
              return prev.length === next.length && prev.every((item, i) => item.tvdbId === next[i]?.tvdbId);
            });
            return unchanged ? previous : nextMetadata;
          });
        })
        .finally(() => {
          if (requestId !== autocompleteRequestId.current) return;
          setMetadataSearchLoading(false);
        });

      const [catalogResult, metadataResult] = await Promise.allSettled([
        catalogPromise,
        metadataPromise,
      ]);

      if (requestId !== autocompleteRequestId.current) return;

      // Surface errors from either leg (suppress AbortError — the request
      // was intentionally cancelled by a newer autocomplete keystroke).
      const isAbortError = (err: unknown): boolean =>
        err != null &&
        typeof err === "object" &&
        "networkError" in err &&
        (err as { networkError?: { name?: string } }).networkError?.name === "AbortError";

      if (catalogResult.status === "rejected" && !isAbortError(catalogResult.reason)) {
        const msg = catalogResult.reason instanceof Error ? catalogResult.reason.message : t("status.apiError");
        setGlobalStatus(msg);
        setCatalogSearchResults((prev) => (prev.length === 0 ? prev : []));
      }
      if (metadataResult.status === "rejected" && !isAbortError(metadataResult.reason)) {
        const msg = metadataResult.reason instanceof Error ? metadataResult.reason.message : t("status.apiError");
        setGlobalStatus(msg);
        setMetadataSearchResults((prev) => (isMetadataEmpty(prev) ? prev : emptyMetadataSearchResults));
      }

      setSearching(false);
    },
    [
      filterMetadataSearchResults,
      client,
      t,
      setGlobalStatus,
      uiLanguage,
      emptyMetadataSearchResults,
      resolveCatalogPosterUrl,
      sortByRelevance,
    ],
  );

  useEffect(() => {
    const trimmed = globalSearch.trim();

    if (trimmed.length < AUTOCOMPLETE_MIN_CHARS) {
      autocompleteAbortRef.current?.abort();
      autocompleteAbortRef.current = null;
      setCatalogSearchLoading(false);
      setMetadataSearchLoading(false);
      setCatalogSearchResults((previous) => (previous.length === 0 ? previous : []));
      setMetadataSearchResults((previous) => {
        if (previous.movie.length === 0 && previous.series.length === 0 && previous.anime.length === 0) {
          return previous;
        }
        return emptyMetadataSearchResults;
      });
      setIsGlobalSearchPanelOpen((isOpen) => (isOpen ? false : isOpen));
      return;
    }

    const debounceTimer = window.setTimeout(() => {
      void runMetadataAutocomplete(trimmed);
    }, AUTOCOMPLETE_DEBOUNCE_MS);

    return () => {
      window.clearTimeout(debounceTimer);
    };
  }, [globalSearch, runMetadataAutocomplete, emptyMetadataSearchResults]);

  const openGlobalSearchPanel = useCallback(() => {
    if (globalSearch.trim().length >= AUTOCOMPLETE_MIN_CHARS) {
      setIsGlobalSearchPanelOpen(true);
    }
  }, [globalSearch]);

  const closeGlobalSearchPanel = useCallback(() => {
    setIsGlobalSearchPanelOpen(false);
  }, []);

  useEffect(() => {
    const handleShortcut = (event: KeyboardEvent) => {
      if (event.key !== "/") {
        return;
      }

      if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) {
        return;
      }

      const target = event.target as HTMLElement | null;
      if (
        target?.tagName === "INPUT" ||
        target?.tagName === "TEXTAREA" ||
        target?.isContentEditable ||
        target?.tagName === "SELECT"
      ) {
        return;
      }

      event.preventDefault();
      globalSearchInputRef.current?.focus();
      globalSearchInputRef.current?.select();
    };

    window.addEventListener("keydown", handleShortcut);
    return () => window.removeEventListener("keydown", handleShortcut);
  }, []);

  const selectTvdbCandidate = useCallback((candidate: MetadataTvdbSearchItem) => {
    setSelectedTvdbId(String(candidate.tvdbId));
    setGlobalStatus(t("status.selectedTvdb", { name: candidate.name }));
  }, [t, setGlobalStatus]);

  const searchNzbForSelectedTvdb = useCallback(async () => {
    if (!selectedTvdb) {
      setGlobalStatus(t("status.tvdbQueueTip"));
      return;
    }

    const tvdbId = String(selectedTvdb.tvdbId).trim();
    const imdbId = selectedTvdb.imdbId ? String(selectedTvdb.imdbId).trim() : "";
    const useMoviePath = queueFacet === "movie";
    if (useMoviePath && !imdbId) {
      setGlobalStatus(t("status.tvdbRequiredImdb"));
      return;
    }
    if (!useMoviePath && (!tvdbId || tvdbId === "0")) {
      setGlobalStatus(t("status.tvdbNoValidId"));
      return;
    }

    const category = useMoviePath ? null : "5070";
    const selectedName = (selectedTvdb.name || "").trim();
    if (!selectedName) {
      setGlobalStatus(t("status.tvdbNeedsTitle"));
      return;
    }

    const results = await runNzbSearch({
      query: selectedName,
      imdbId: useMoviePath ? imdbId : null,
      tvdbId: useMoviePath ? null : tvdbId,
      category,
    });

    if (results.length === 0) {
      setGlobalStatus(t("status.noNzbFound"));
    } else {
      const source = category ? ` category=${category}` : "";
      setGlobalStatus(t("status.nzbFoundForTitle", { count: results.length, name: selectedName, source }));
    }
  }, [runNzbSearch, queueFacet, selectedTvdb, setGlobalStatus, t]);

  const addMetadataSearchResultToCatalog = useCallback(
    async (
      result: MetadataTvdbSearchItem,
      facet: Facet,
      options: MetadataCatalogAddOptions,
    ) => {
      const name = result.name.trim();
      if (!name) {
        setGlobalStatus(t("status.titleRequired"));
        return null;
      }

      const qualityProfileId = (
        options.qualityProfileId || resolveDefaultQualityProfileIdForFacet(facet)
      ).trim();
      if (!qualityProfileId) {
        setGlobalStatus(t("search.addConfigNoQualityProfiles"));
        return null;
      }

      const monitored = monitorTypeToMonitored(options.monitorType);

      const tvdbId = String(result.tvdbId).trim();
      const imdbId = result.imdbId?.trim();
      const externalIds = [
        ...(tvdbId ? [{ source: "tvdb", value: tvdbId }] : []),
        ...(imdbId ? [{ source: "imdb", value: imdbId }] : []),
      ];
      const tags = [
        `scryer:quality-profile:${qualityProfileId}`,
        `scryer:monitor-type:${options.monitorType}`,
        ...(facet === "movie"
          ? []
          : [`scryer:season-folder:${options.seasonFolder ? "enabled" : "disabled"}`]),
      ];

      try {
        const { data: addData, error: addError } = await client.mutation(addTitleMutation, {
          input: {
            name,
            facet,
            monitored,
            tags,
            externalIds,
            ...(facet === "movie" && options.minAvailability ? { minAvailability: options.minAvailability } : {}),
            posterUrl: result.posterUrl || undefined,
            year: result.year ?? undefined,
            overview: result.overview || undefined,
            sortTitle: result.sortTitle || undefined,
            slug: result.slug || undefined,
            runtimeMinutes: result.runtimeMinutes ?? undefined,
            language: result.language || undefined,
            contentStatus: result.status || undefined,
          },
        }).toPromise();
        if (addError) throw addError;
        setGlobalStatus(
          t(
            monitored
              ? "status.catalogAddSuccessAutoSearch"
              : "status.catalogAddSuccess",
            { name: addData.addTitle.title.name },
          ),
        );
        await runMetadataAutocomplete(globalSearch.trim());
        onCatalogChanged?.();
        setCatalogChangeSignal((v) => v + 1);
        return addData.addTitle?.title?.id?.trim() || null;
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.queueFailed"));
        return null;
      }
    },
    [
      globalSearch,
      onCatalogChanged,
      resolveDefaultQualityProfileIdForFacet,
      runMetadataAutocomplete,
      client,
      setGlobalStatus,
      t,
    ],
  );

  const runSearch = useCallback(
    async (
      query: string,
      category: string | null = null,
      options?: NzbSearchOptions,
    ) => {
      setSearching(true);
      setGlobalStatus(t("status.searchingByQuery", { query }));
      const imdbId = options?.imdbId?.trim() || null;
      const tvdbId = options?.tvdbId?.trim() || null;
      const limit = Math.max(1, Math.min(options?.limit ?? 15, 100));
      try {
        const { data: searchData, error: searchError } = await client.query(searchQuery, {
          query,
          imdbId,
          tvdbId,
          category,
          limit,
        }).toPromise();
        if (searchError) throw searchError;
        setSearchResults(searchData.searchIndexers || []);
        setGlobalStatus(t("status.searchingByQuery", { count: searchData.searchIndexers?.length || 0, query }));
        return searchData.searchIndexers || [];
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.apiError"));
        return [];
      } finally {
        setSearching(false);
      }
    },
    [client, t, setGlobalStatus],
  );

  const handleGlobalSearchSubmit = useCallback(async () => {
    if (!globalSearch.trim()) {
      return;
    }
    setTvdbCandidates([]);
    setSelectedTvdbId(null);
    await runSearch(globalSearch.trim());
  }, [globalSearch, runSearch]);

  return {
    globalSearch,
    setGlobalSearch,
    globalSearchInputRef,
    searching,
    catalogSearchLoading,
    metadataSearchLoading,
    searchResults,
    tvdbCandidates,
    selectedTvdbId,
    selectedTvdb,
    runNzbSearch,
    runTvdbSearch,
    handleGlobalSearchSubmit,
    selectTvdbCandidate,
    searchNzbForSelectedTvdb,
    setSelectedTvdbId,
    runSearch,
    setSearching,
    setTvdbCandidates,
    setSearchResults,
    catalogSearchResults,
    metadataSearchResults,
    isGlobalSearchPanelOpen,
    openGlobalSearchPanel,
    closeGlobalSearchPanel,
    catalogQualityProfileOptions,
    resolveDefaultQualityProfileIdForFacet,
    addMetadataSearchResultToCatalog,
    isMetadataSearchResultInCatalog,
    queueFacet,
    setQueueFacet,
    catalogChangeSignal,
  };
}

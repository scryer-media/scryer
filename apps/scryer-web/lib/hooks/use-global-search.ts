import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useClient } from "urql";
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import type { ExternalId, Facet, TitleRecord } from "@/lib/types";
import type { ViewCategoryId } from "@/lib/types/quality-profiles";
import type { LocaleCode } from "@/lib/i18n";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { showCatalogAddToast } from "@/components/root/catalog-add-toast";
import {
  catalogSearchTitlesQuery,
  globalSearchInitQuery,
  globalSearchRequesterInitQuery,
  metadataMovieQuery,
  metadataSeriesQuery,
  requestableLibrariesQuery,
  searchMetadataMultiQuery,
  searchMetadataQuery,
  titlesByExternalIdsQuery,
} from "@/lib/graphql/queries";
import {
  isAbortError,
  makeAbortableFetch,
} from "@/lib/graphql/urql-client";
import { addTitleMutation, submitMediaRequestMutation } from "@/lib/graphql/mutations";
import {
  ANIME_INTER_SEASON_MOVIES_KEY,
  ANIME_MONITOR_SPECIALS_KEY,
  QUALITY_PROFILE_CATALOG_KEY,
  QUALITY_PROFILE_ID_KEY,
  QUALITY_PROFILE_INHERIT_VALUE,
  REQUEST_QUALITY_PROFILE_IDS_KEY,
} from "@/lib/constants/settings";
import {
  coerceProfileSetting,
  qualityProfileSettingsToCategoryOverrides,
} from "@/lib/utils/quality-profiles";
import { FACET_REGISTRY, facetById } from "@/lib/facets/registry";
import { useSettingsSubscription } from "@/lib/hooks/use-settings-subscription";
import { dispatchCatalogTitlesRefresh } from "@/lib/events/catalog-titles";
import { dispatchNavigationBadgesRefresh } from "@/lib/events/navigation-badges";
import type { AuthUser } from "@/lib/hooks/use-auth";
import {
  authorizationCacheSignature,
  hasAnyLibraryPermission,
  LIBRARY_PERMISSIONS,
} from "@/lib/utils/permissions";

export type MetadataSearchResults = Record<string, MetadataTvdbSearchItem[]>;

export type CatalogQualityProfileOption = {
  id: string;
  name: string;
};

export type MetadataCatalogMonitorType =
  | "MONITORED"
  | "UNMONITORED"
  | "FUTURE_EPISODES"
  | "MISSING_AND_FUTURE_EPISODES"
  | "ALL_EPISODES"
  | "NONE";

export type { RootFolderOption } from "@/lib/types/titles";
import type { LibraryRecord, RootFolderOption } from "@/lib/types/titles";

export type MetadataCatalogAddOptions = {
  libraryId?: string;
  qualityProfileId?: string;
  seasonFolder: boolean;
  monitorType: MetadataCatalogMonitorType;
  minAvailability?: string;
  monitorSpecials?: boolean;
  interSeasonMovies?: boolean;
  rootFolderId?: string;
};

export type CatalogAddFeedback = {
  /**
   * Supplied by surfaces that can navigate to the new title. When present the
   * success toast grows a "View in catalog" button, so the add flow no longer
   * has to yank the page out from under someone adding several titles.
   */
  onViewInCatalog?: (titleId: string) => void;
};

export type MetadataCatalogRequestOptions = {
  libraryId: string;
  requestedQualityProfileId?: string;
  requestedMonitorType?: MetadataCatalogMonitorType;
};

export type AnimeCatalogDefaults = {
  monitorSpecials: boolean;
  interSeasonMovies: boolean;
};

function isMetadataEmpty(results: MetadataSearchResults): boolean {
  return Object.values(results).every((arr) => arr.length === 0);
}

function hasOpenDialogContent(): boolean {
  if (typeof document === "undefined") {
    return false;
  }

  return document.querySelector("[data-slot='dialog-content'][data-state='open']") !== null;
}

function normalizeOrderedLookupValues(values: string[]): string[] {
  const seen = new Set<string>();
  const normalized: string[] = [];
  for (const value of values) {
    const trimmed = value.trim();
    if (!trimmed || seen.has(trimmed)) {
      continue;
    }
    seen.add(trimmed);
    normalized.push(trimmed);
  }
  return normalized;
}

function titleMetadataIds(title: TitleRecord, source: "smg" | "tvdb"): string[] {
  return (title.externalIds ?? [])
    .filter((externalId) => externalId.source.toLowerCase() === source)
    .map((externalId) => externalId.value.trim())
    .filter(Boolean);
}

function buildCatalogTitleLookupByMetadataId(titles: TitleRecord[]): Record<string, TitleRecord> {
  const lookup: Record<string, TitleRecord> = {};
  for (const title of titles) {
    for (const source of ["smg", "tvdb"] as const) {
      for (const id of titleMetadataIds(title, source)) {
        const key = `${source}:${id}`;
        if (!(key in lookup)) {
          lookup[key] = title;
        }
      }
    }
  }
  return lookup;
}

function metadataResultTvdbId(result: MetadataTvdbSearchItem): string {
  return String(result.tvdbId).trim();
}

function metadataResultSmgId(result: MetadataTvdbSearchItem): string {
  return result.smgId == null ? "" : String(result.smgId).trim();
}

function metadataResultCatalogLookupKeys(result: MetadataTvdbSearchItem): string[] {
  const smgId = metadataResultSmgId(result);
  const tvdbId = metadataResultTvdbId(result);
  return [
    ...(smgId ? [`smg:${smgId}`] : []),
    ...(tvdbId ? [`tvdb:${tvdbId}`] : []),
  ];
}

function isMetadataResultCataloged(
  lookup: Record<string, TitleRecord>,
  result: MetadataTvdbSearchItem,
): boolean {
  return metadataResultCatalogLookupKeys(result).some((key) => lookup[key] !== undefined);
}

function filterCatalogedMetadataResults(
  results: MetadataTvdbSearchItem[],
  lookup: Record<string, TitleRecord>,
): MetadataTvdbSearchItem[] {
  return results.filter((result) => !isMetadataResultCataloged(lookup, result));
}

function mergeCatalogResults(
  prioritized: TitleRecord[],
  fallback: TitleRecord[],
): TitleRecord[] {
  const merged: TitleRecord[] = [];
  const seen = new Set<string>();
  for (const title of [...prioritized, ...fallback]) {
    if (seen.has(title.id)) {
      continue;
    }
    seen.add(title.id);
    merged.push(title);
  }
  return merged;
}

function sameExternalIds(
  previous: ExternalId[] | null | undefined,
  next: ExternalId[] | null | undefined,
): boolean {
  const previousIds = previous ?? [];
  const nextIds = next ?? [];
  return (
    previousIds.length === nextIds.length &&
    previousIds.every(
      (item, index) =>
        item.source === nextIds[index]?.source &&
        item.value === nextIds[index]?.value,
    )
  );
}

function sameStringList(
  previous: string[] | null | undefined,
  next: string[] | null | undefined,
): boolean {
  const previousItems = previous ?? [];
  const nextItems = next ?? [];
  return (
    previousItems.length === nextItems.length &&
    previousItems.every((item, index) => item === nextItems[index])
  );
}

function sameTitleList(
  previous: TitleRecord[],
  next: TitleRecord[],
): boolean {
  return (
    previous.length === next.length &&
    previous.every((item, index) => {
      const nextItem = next[index];
      return (
        nextItem !== undefined &&
        item.id === nextItem.id &&
        item.name === nextItem.name &&
        item.facet === nextItem.facet &&
        item.libraryId === nextItem.libraryId &&
        item.monitored === nextItem.monitored &&
        sameStringList(item.tags, nextItem.tags) &&
        (item.libraryName ?? null) === (nextItem.libraryName ?? null) &&
        (item.librarySlug ?? null) === (nextItem.librarySlug ?? null) &&
        (item.slug ?? null) === (nextItem.slug ?? null) &&
        (item.sortTitle ?? null) === (nextItem.sortTitle ?? null) &&
        (item.year ?? null) === (nextItem.year ?? null) &&
        (item.posterUrl ?? null) === (nextItem.posterUrl ?? null) &&
        (item.posterSourceUrl ?? null) === (nextItem.posterSourceUrl ?? null) &&
        (item.rootFolderId ?? null) === (nextItem.rootFolderId ?? null) &&
        (item.rootFolderPath ?? null) === (nextItem.rootFolderPath ?? null) &&
        (item.qualityTier ?? null) === (nextItem.qualityTier ?? null) &&
        (item.currentQualityTier ?? null) ===
          (nextItem.currentQualityTier ?? null) &&
        (item.sizeBytes ?? null) === (nextItem.sizeBytes ?? null) &&
        (item.episodesOwned ?? null) === (nextItem.episodesOwned ?? null) &&
        (item.episodesMonitored ?? null) ===
          (nextItem.episodesMonitored ?? null) &&
        (item.episodesTotal ?? null) === (nextItem.episodesTotal ?? null) &&
        (item.contentStatus ?? null) === (nextItem.contentStatus ?? null) &&
        (item.metadataFetchedAt ?? null) === (nextItem.metadataFetchedAt ?? null) &&
        (item.createdAt ?? null) === (nextItem.createdAt ?? null) &&
        sameExternalIds(item.externalIds, nextItem.externalIds)
      );
    })
  );
}

function sameCatalogLookup(
  previous: Record<string, TitleRecord>,
  next: Record<string, TitleRecord>,
): boolean {
  const previousKeys = Object.keys(previous);
  const nextKeys = Object.keys(next);
  return (
    previousKeys.length === nextKeys.length &&
    previousKeys.every((key) => previous[key]?.id === next[key]?.id)
  );
}

const AUTOCOMPLETE_MIN_CHARS = 2;
const AUTOCOMPLETE_DEBOUNCE_MS = 250;
const AUTOCOMPLETE_LIMIT = 20;
const EMPTY_QUERY_CATALOG_LIMIT = 12;

type UseGlobalSearchArgs = {
  authenticatedUser: AuthUser;
  queueFacet: Facet;
  uiLanguage: LocaleCode;
};

type CatalogConfigAccessMode = "manager" | "requester";

export interface UseGlobalSearchResult {
  globalSearch: string;
  setGlobalSearch: (value: string) => void;
  globalSearchInputRef: React.RefObject<HTMLInputElement | null>;
  searching: boolean;
  catalogSearchLoading: boolean;
  metadataSearchLoading: boolean;
  tvdbCandidates: MetadataTvdbSearchItem[];
  runTvdbSearch: (query: string) => Promise<MetadataTvdbSearchItem[]>;
  forceSearchGlobal: (queryOverride?: string) => Promise<void>;
  setTvdbCandidates: (value: MetadataTvdbSearchItem[]) => void;
  catalogSearchResults: TitleRecord[];
  metadataSearchResults: MetadataSearchResults;
  isGlobalSearchPanelOpen: boolean;
  openGlobalSearchPanel: (force?: boolean) => void;
  closeGlobalSearchPanel: () => void;
  clearGlobalSearch: () => void;
  resetGlobalSearch: () => void;
  catalogQualityProfileOptions: CatalogQualityProfileOption[];
  catalogConfigLoading: boolean;
  ensureCatalogConfigReady: (facet: Facet) => Promise<void>;
  isCatalogConfigReady: (facet: Facet) => boolean;
  resolveDefaultQualityProfileIdForFacet: (facet: Facet) => string;
  animeCatalogDefaults: AnimeCatalogDefaults;
  addMetadataSearchResultToCatalog: (
    result: MetadataTvdbSearchItem,
    facet: Facet,
    options: MetadataCatalogAddOptions,
    feedback?: CatalogAddFeedback,
  ) => Promise<string | null>;
  requestMetadataSearchResult: (
    result: MetadataTvdbSearchItem,
    facet: Facet,
    options: MetadataCatalogRequestOptions,
  ) => Promise<boolean>;
  isMetadataSearchResultInCatalog: (
    facet: Facet,
    result: MetadataTvdbSearchItem,
  ) => boolean;
  rootFoldersByFacet: Record<Facet, RootFolderOption[]>;
  librariesByFacet: Record<Facet, LibraryRecord[]>;
  requestableLibrariesByFacet: Record<Facet, LibraryRecord[]>;
  queueFacet: Facet;
  setQueueFacet: (value: Facet) => void;
  catalogChangeSignal: number;
}

function monitorTypeToMonitored(monitorType: MetadataCatalogMonitorType): boolean {
  return monitorType !== "UNMONITORED" && monitorType !== "NONE";
}

function normalizeCatalogAddRequestKey(
  facet: Facet,
  externalIds: ExternalId[],
): string {
  const normalizedIds = [...externalIds]
    .map((externalId) => ({
      source: externalId.source.trim().toLowerCase(),
      value: externalId.value.trim(),
    }))
    .filter((externalId) => externalId.source && externalId.value)
    .sort((left, right) => {
      const sourceCompare = left.source.localeCompare(right.source);
      if (sourceCompare !== 0) {
        return sourceCompare;
      }
      return left.value.localeCompare(right.value);
    })
    .map((externalId) => `${externalId.source}:${externalId.value}`)
    .join("|");

  return `${facet}|${normalizedIds}`;
}

function metadataResultExternalIds(result: MetadataTvdbSearchItem): ExternalId[] {
  const smgId = metadataResultSmgId(result);
  const tvdbId = String(result.tvdbId).trim();
  const tmdbId = result.tmdbId == null ? "" : String(result.tmdbId).trim();
  const imdbId = result.imdbId?.trim();
  const seen = new Set<string>();
  const ids: ExternalId[] = [];
  for (const externalId of [
    ...(result.externalIds ?? []),
    ...(smgId ? [{ source: "smg", value: smgId }] : []),
    ...(tvdbId ? [{ source: "tvdb", value: tvdbId }] : []),
    ...(tmdbId ? [{ source: "tmdb", value: tmdbId }] : []),
    ...(imdbId ? [{ source: "imdb", value: imdbId }] : []),
  ]) {
    const source = externalId.source.trim().toLowerCase();
    const value = externalId.value.trim();
    const key = `${source}:${value}`;
    if (!source || !value || seen.has(key)) {
      continue;
    }
    seen.add(key);
    ids.push({ source, value });
  }
  return ids;
}

function librariesByFacetFromList(libraries: LibraryRecord[]): Record<Facet, LibraryRecord[]> {
  return libraries.reduce(
    (acc: Record<Facet, LibraryRecord[]>, library: LibraryRecord) => {
      acc[library.facet]?.push(library);
      return acc;
    },
    { MOVIE: [], SERIES: [], ANIME: [] },
  );
}

const EMPTY_LIBRARIES_BY_FACET: Record<Facet, LibraryRecord[]> = {
  MOVIE: [],
  SERIES: [],
  ANIME: [],
};

function sameRootFolderOptions(
  previous: RootFolderOption[],
  next: RootFolderOption[],
): boolean {
  return (
    previous.length === next.length &&
    previous.every((entry, index) => {
      const candidate = next[index];
      return (
        candidate !== undefined &&
        (entry.id ?? null) === (candidate.id ?? null) &&
        entry.path === candidate.path &&
        entry.isDefault === candidate.isDefault
      );
    })
  );
}

function sameLibrariesByFacet(
  previous: Record<Facet, LibraryRecord[]>,
  next: Record<Facet, LibraryRecord[]>,
): boolean {
  return (["MOVIE", "SERIES", "ANIME"] as Facet[]).every((facet) => {
    const previousFacetLibraries = previous[facet];
    const nextFacetLibraries = next[facet];
    return (
      previousFacetLibraries.length === nextFacetLibraries.length &&
      previousFacetLibraries.every((entry, index) => {
        const candidate = nextFacetLibraries[index];
        return (
          candidate &&
          entry.id === candidate.id &&
          entry.name === candidate.name &&
          entry.slug === candidate.slug &&
          entry.isDefault === candidate.isDefault &&
          (entry.qualityProfileId ?? null) ===
            (candidate.qualityProfileId ?? null) &&
          (entry.requestQualityProfileDefaultId ?? null) ===
            (candidate.requestQualityProfileDefaultId ?? null) &&
          (entry.requestQualityProfileIds ?? []).join("|") ===
            (candidate.requestQualityProfileIds ?? []).join("|") &&
          sameRootFolderOptions(entry.roots, candidate.roots)
        );
      })
    );
  });
}

export function useGlobalSearch({
  authenticatedUser,
  queueFacet: initialQueueFacet,
  uiLanguage,
}: UseGlobalSearchArgs): UseGlobalSearchResult {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const catalogAuthorizationSignature = authorizationCacheSignature(authenticatedUser);
  const canManageTitle = hasAnyLibraryPermission(
    authenticatedUser,
    LIBRARY_PERMISSIONS.manageTitles,
  );
  const canViewCatalog = hasAnyLibraryPermission(
    authenticatedUser,
    LIBRARY_PERMISSIONS.view,
  );
  const [queueFacet, setQueueFacet] = useState<Facet>(initialQueueFacet);
  const catalogChangeSignal = 0;
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
  const [tvdbCandidates, setTvdbCandidates] = useState<MetadataTvdbSearchItem[]>([]);
  const [catalogSearchResults, setCatalogSearchResults] = useState<TitleRecord[]>([]);
  const [catalogTitlesByTvdbId, setCatalogTitlesByTvdbId] = useState<
    Record<string, TitleRecord>
  >({});
  const [metadataSearchResults, setMetadataSearchResults] = useState<MetadataSearchResults>(
    () => Object.fromEntries(FACET_REGISTRY.map((f) => [f.metadataKey, []])),
  );
  const [catalogQualityProfileOptions, setCatalogQualityProfileOptions] = useState<
    CatalogQualityProfileOption[]
  >([]);
  const [globalQualityProfileId, setGlobalQualityProfileId] = useState<string>(
    QUALITY_PROFILE_INHERIT_VALUE,
  );
  const [animeCatalogDefaults, setAnimeCatalogDefaults] = useState<AnimeCatalogDefaults>({
    monitorSpecials: true,
    interSeasonMovies: true,
  });
  const [categoryQualityProfileOverrides, setCategoryQualityProfileOverrides] = useState<
    Record<ViewCategoryId, string>
  >(
    () => Object.fromEntries(FACET_REGISTRY.map((f) => [f.scopeId, QUALITY_PROFILE_INHERIT_VALUE])) as Record<ViewCategoryId, string>,
  );
  const [isGlobalSearchPanelOpen, setIsGlobalSearchPanelOpen] = useState(false);
  const [catalogConfigLoading, setCatalogConfigLoading] = useState(false);
  const [rootFoldersByFacet, setRootFoldersByFacet] = useState<Record<Facet, RootFolderOption[]>>(
    () => ({ MOVIE: [], SERIES: [], ANIME: [] }),
  );
  const [librariesByFacet, setLibrariesByFacet] = useState<Record<Facet, LibraryRecord[]>>(
    () => ({ MOVIE: [], SERIES: [], ANIME: [] }),
  );
  const [requestableLibrariesByFacet, setRequestableLibrariesByFacet] = useState<
    Record<Facet, LibraryRecord[]>
  >(() => ({ MOVIE: [], SERIES: [], ANIME: [] }));
  const [catalogConfigAuthorizationSignature, setCatalogConfigAuthorizationSignature] =
    useState<string | null>(null);
  const catalogConfigMatchesAuthorization =
    catalogConfigAuthorizationSignature === catalogAuthorizationSignature;
  const visibleLibrariesByFacet = catalogConfigMatchesAuthorization
    ? librariesByFacet
    : EMPTY_LIBRARIES_BY_FACET;
  const visibleRequestableLibrariesByFacet = catalogConfigMatchesAuthorization
    ? requestableLibrariesByFacet
    : EMPTY_LIBRARIES_BY_FACET;
  const forcedOpenRef = useRef(false);
  const autocompleteRequestId = useRef(0);
  const autocompleteAbortRef = useRef<AbortController | null>(null);
  const autocompleteDebounceTimerRef = useRef<number | null>(null);
  const skipNextAutocompleteQueryRef = useRef<string | null>(null);
  const pendingCatalogAddKeysRef = useRef<Set<string>>(new Set());
  const pendingRequestKeysRef = useRef<Set<string>>(new Set());
  const catalogConfigRefreshPromiseRef = useRef<{
    mode: CatalogConfigAccessMode;
    authorizationSignature: string;
    promise: Promise<void>;
  } | null>(null);
  const catalogConfigRefreshTokenRef = useRef(0);
  const catalogConfigLoadedRef = useRef(false);
  const catalogAuthorizationSignatureRef = useRef(catalogAuthorizationSignature);
  useEffect(() => {
    catalogAuthorizationSignatureRef.current = catalogAuthorizationSignature;
  }, [catalogAuthorizationSignature]);

  const cancelAutocomplete = useCallback(() => {
    if (autocompleteDebounceTimerRef.current !== null) {
      window.clearTimeout(autocompleteDebounceTimerRef.current);
      autocompleteDebounceTimerRef.current = null;
    }
    autocompleteRequestId.current += 1;
    autocompleteAbortRef.current?.abort();
    autocompleteAbortRef.current = null;
    setSearching(false);
    setCatalogSearchLoading(false);
    setMetadataSearchLoading(false);
  }, []);

  const catalogQualityProfileIdSet = useMemo(
    () => new Set(catalogQualityProfileOptions.map((profile) => profile.id)),
    [catalogQualityProfileOptions],
  );

  const resolveDefaultQualityProfileIdForFacet = useCallback(
    (facet: Facet) => {
      const scopeId = facetById(facet)?.scopeId ?? "MOVIE";
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

  const isCatalogConfigReady = useCallback(
    (facet: Facet) =>
      visibleRequestableLibrariesByFacet[facet].length > 0 ||
      (catalogQualityProfileOptions.length > 0 &&
        (visibleLibrariesByFacet[facet].length > 0 ||
          rootFoldersByFacet[facet].length > 0)),
    [
      catalogQualityProfileOptions,
      visibleLibrariesByFacet,
      visibleRequestableLibrariesByFacet,
      rootFoldersByFacet,
    ],
  );

  const refreshCatalogQualityProfileState = useCallback(async () => {
    const accessMode: CatalogConfigAccessMode = canManageTitle
      ? "manager"
      : "requester";
    if (
      catalogConfigRefreshPromiseRef.current?.mode === accessMode &&
      catalogConfigRefreshPromiseRef.current.authorizationSignature ===
        catalogAuthorizationSignature
    ) {
      return catalogConfigRefreshPromiseRef.current.promise;
    }

    const refreshToken = catalogConfigRefreshTokenRef.current + 1;
    catalogConfigRefreshTokenRef.current = refreshToken;
    const isCurrentRefresh = () =>
      catalogConfigRefreshTokenRef.current === refreshToken &&
      catalogAuthorizationSignatureRef.current === catalogAuthorizationSignature;

    const refreshPromise = (async () => {
      setCatalogConfigLoading(true);
      try {
        const { data, error } = await client
          .query(
            canManageTitle ? globalSearchInitQuery : globalSearchRequesterInitQuery,
            {},
            { requestPolicy: "network-only" },
          )
          .toPromise();
        if (error) throw error;
        if (!isCurrentRefresh()) return;
        catalogConfigLoadedRef.current = true;
        setCatalogConfigAuthorizationSignature(catalogAuthorizationSignature);

        const parsedProfiles = (data.qualityProfileSettings?.profiles ?? []).map(
          (profile: { id: string; name: string }) => ({
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

        const nextGlobalProfileId = coerceProfileSetting(
          data.qualityProfileSettings?.globalProfileId ?? "",
        );
        setGlobalQualityProfileId((previous) =>
          previous === nextGlobalProfileId ? previous : nextGlobalProfileId,
        );

        const nextOverrides: Record<ViewCategoryId, string> =
          qualityProfileSettingsToCategoryOverrides(data.qualityProfileSettings);
        setCategoryQualityProfileOverrides((previous) =>
          previous.MOVIE === nextOverrides.MOVIE &&
          previous.SERIES === nextOverrides.SERIES &&
          previous.ANIME === nextOverrides.ANIME
            ? previous
            : nextOverrides,
        );

        if (canManageTitle) {
          const nextAnimeDefaults: AnimeCatalogDefaults = {
            monitorSpecials: data.animeSettings?.monitorSpecials ?? false,
            interSeasonMovies: data.animeSettings?.interSeasonMovies ?? true,
          };
          setAnimeCatalogDefaults((previous) =>
            previous.monitorSpecials === nextAnimeDefaults.monitorSpecials &&
            previous.interSeasonMovies === nextAnimeDefaults.interSeasonMovies
              ? previous
              : nextAnimeDefaults,
          );

          const nextRootFolders: Record<Facet, RootFolderOption[]> = {
            MOVIE: data.movieSettings?.rootFolders ?? [],
            SERIES: data.seriesSettings?.rootFolders ?? [],
            ANIME: data.animeSettings?.rootFolders ?? [],
          };
          setRootFoldersByFacet((previous) => {
            const same = (["MOVIE", "SERIES", "ANIME"] as Facet[]).every((f) => {
              const prev = previous[f];
              const next = nextRootFolders[f];
              return prev.length === next.length && prev.every((e, i) => e.path === next[i]?.path && e.isDefault === next[i]?.isDefault);
            });
            return same ? previous : nextRootFolders;
          });
        }

        const nextLibrariesByFacet = librariesByFacetFromList(
          data.manageableLibraries ?? [],
        );
        const nextRequestableLibrariesByFacet = librariesByFacetFromList(
          data.requestableLibraries ?? [],
        );
        setLibrariesByFacet((previous) => {
          return sameLibrariesByFacet(previous, nextLibrariesByFacet)
            ? previous
            : nextLibrariesByFacet;
        });
        setRequestableLibrariesByFacet((previous) => {
          return sameLibrariesByFacet(previous, nextRequestableLibrariesByFacet)
            ? previous
            : nextRequestableLibrariesByFacet;
        });
      } catch {
        try {
          const { data, error } = await client
            .query(requestableLibrariesQuery, {}, { requestPolicy: "network-only" })
            .toPromise();
          if (error) throw error;
          if (!isCurrentRefresh()) return;
          const nextRequestableLibrariesByFacet = librariesByFacetFromList(
            data?.requestableLibraries ?? [],
          );
          setRequestableLibrariesByFacet((previous) => {
            return sameLibrariesByFacet(previous, nextRequestableLibrariesByFacet)
              ? previous
              : nextRequestableLibrariesByFacet;
          });
          catalogConfigLoadedRef.current = true;
          setCatalogConfigAuthorizationSignature(catalogAuthorizationSignature);
        } catch {
          // ignore requestable library fallback failures here; search remains functional
        }
        // ignore settings fetch failures here; search remains functional
      } finally {
        if (isCurrentRefresh()) {
          setCatalogConfigLoading(false);
          if (
            catalogConfigRefreshPromiseRef.current?.mode === accessMode &&
            catalogConfigRefreshPromiseRef.current.authorizationSignature ===
              catalogAuthorizationSignature
          ) {
            catalogConfigRefreshPromiseRef.current = null;
          }
        }
      }
    })();

    catalogConfigRefreshPromiseRef.current = {
      mode: accessMode,
      authorizationSignature: catalogAuthorizationSignature,
      promise: refreshPromise,
    };
    return refreshPromise;
  }, [canManageTitle, catalogAuthorizationSignature, client]);

  const ensureCatalogConfigReady = useCallback(
    async (facet: Facet) => {
      if (isCatalogConfigReady(facet)) {
        return;
      }
      await refreshCatalogQualityProfileState();
    },
    [isCatalogConfigReady, refreshCatalogQualityProfileState],
  );

  const primeCatalogConfigForMetadataActions = useCallback(async () => {
    if (
      catalogConfigLoadedRef.current &&
      catalogConfigMatchesAuthorization
    ) {
      return;
    }

    try {
      await refreshCatalogQualityProfileState();
    } catch {
      // Search should still render results if config priming unexpectedly fails.
    }
  }, [catalogConfigMatchesAuthorization, refreshCatalogQualityProfileState]);

  useEffect(() => {
    catalogConfigRefreshTokenRef.current += 1;
    catalogConfigRefreshPromiseRef.current = null;
    catalogConfigLoadedRef.current = false;
    setCatalogConfigAuthorizationSignature(null);
    setCatalogConfigLoading(false);
    setRootFoldersByFacet({ MOVIE: [], SERIES: [], ANIME: [] });
    setLibrariesByFacet({ MOVIE: [], SERIES: [], ANIME: [] });
    setRequestableLibrariesByFacet({ MOVIE: [], SERIES: [], ANIME: [] });
  }, [catalogAuthorizationSignature]);

  useEffect(() => {
    void refreshCatalogQualityProfileState();
  }, [refreshCatalogQualityProfileState]);

  // Re-fetch search config when settings change (cross-client via WebSocket).
  const searchSettingsKeys = useMemo(
    () =>
      new Set([
        QUALITY_PROFILE_CATALOG_KEY,
        QUALITY_PROFILE_ID_KEY,
        REQUEST_QUALITY_PROFILE_IDS_KEY,
        ...FACET_REGISTRY.map((f) => f.rootFoldersKey),
        ...FACET_REGISTRY.map((f) => f.folderSettingKey),
        ANIME_MONITOR_SPECIALS_KEY,
        ANIME_INTER_SEASON_MOVIES_KEY,
      ]),
    [],
  );

  useSettingsSubscription(
    useCallback(
      (keys: string[]) => {
        if (keys.some((k) => searchSettingsKeys.has(k))) {
          void refreshCatalogQualityProfileState();
        }
      },
      [searchSettingsKeys, refreshCatalogQualityProfileState],
    ),
    { enabled: canViewCatalog },
  );

  const isMetadataSearchResultInAnyCatalog = useCallback(
    (result: MetadataTvdbSearchItem) => isMetadataResultCataloged(catalogTitlesByTvdbId, result),
    [catalogTitlesByTvdbId],
  );

  const isMetadataSearchResultInCatalog = useCallback(
    (_facet: Facet, result: MetadataTvdbSearchItem) => isMetadataSearchResultInAnyCatalog(result),
    [isMetadataSearchResultInAnyCatalog],
  );

  const mapFacetToTvdbType = useCallback((facet: Facet) => {
    return facetById(facet)?.tvdbSearchType ?? "series";
  }, []);

  const resolveCatalogPosterUrl = useCallback(
    async (title: TitleRecord): Promise<TitleRecord> => {
      if (title.posterUrl) {
        return title;
      }

      const tvdbId = (title.externalIds ?? [])
        .find((externalId) => externalId.source.toLowerCase() === "tvdb")
        ?.value.trim();
      const smgId = (title.externalIds ?? [])
        .find((externalId) => externalId.source.toLowerCase() === "smg")
        ?.value.trim();
      if (title.facet === "MOVIE" && !smgId && !tvdbId) {
        return title;
      }
      if (title.facet !== "MOVIE" && !tvdbId) {
        return title;
      }

      try {
        if (title.facet === "MOVIE") {
          const { data, error } = await client.query(metadataMovieQuery, {
            input: {
              smgId: smgId ? Number(smgId) : undefined,
              tvdbId: tvdbId || undefined,
              language: uiLanguage,
            },
          }).toPromise();
          if (error || !data?.metadataMovie?.posterUrl) return title;
          return { ...title, posterUrl: data.metadataMovie.posterUrl };
        }

        const { data, error } = await client.query(metadataSeriesQuery, {
          input: {
            tvdbId,
            includeEpisodes: false,
            language: uiLanguage,
          },
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
  const emptyCatalogTitlesByTvdbId = useMemo<Record<string, TitleRecord>>(() => ({}), []);

  useEffect(() => {
    if (canViewCatalog) {
      return;
    }

    cancelAutocomplete();
    setCatalogSearchResults((previous) => (previous.length === 0 ? previous : []));
    setCatalogTitlesByTvdbId((previous) =>
      Object.keys(previous).length === 0 ? previous : emptyCatalogTitlesByTvdbId,
    );
  }, [canViewCatalog, cancelAutocomplete, emptyCatalogTitlesByTvdbId]);

  const lookupCatalogTitlesByExternalIds = useCallback(
    async (
      source: string,
      values: string[],
      fetchOverride?: typeof fetch,
    ): Promise<TitleRecord[]> => {
      const normalizedValues = normalizeOrderedLookupValues(values);
      if (!source.trim() || normalizedValues.length === 0) {
        return [];
      }

      const { data, error } = await client.query(
        titlesByExternalIdsQuery,
        {
          source: source.trim(),
          values: normalizedValues,
        },
        fetchOverride ? { fetch: fetchOverride } : undefined,
      ).toPromise();
      if (error) {
        throw error;
      }
      return (data?.titlesByExternalIds ?? []) as TitleRecord[];
    },
    [client],
  );

  const lookupCatalogTitlesByTvdbIds = useCallback(
    async (tvdbIds: string[], fetchOverride?: typeof fetch): Promise<TitleRecord[]> =>
      lookupCatalogTitlesByExternalIds("tvdb", tvdbIds, fetchOverride),
    [lookupCatalogTitlesByExternalIds],
  );

  const lookupCatalogTitlesForMetadataResults = useCallback(
    async (
      results: MetadataTvdbSearchItem[],
      fetchOverride?: typeof fetch,
    ): Promise<TitleRecord[]> => {
      const [smgMatches, tvdbMatches] = await Promise.all([
        lookupCatalogTitlesByExternalIds(
          "smg",
          results.map(metadataResultSmgId),
          fetchOverride,
        ),
        lookupCatalogTitlesByTvdbIds(
          results.map(metadataResultTvdbId),
          fetchOverride,
        ),
      ]);
      return mergeCatalogResults(smgMatches, tvdbMatches);
    },
    [lookupCatalogTitlesByExternalIds, lookupCatalogTitlesByTvdbIds],
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
        const rankedMatches = sortByRelevance(
          (searchData.searchMetadata || []) as MetadataTvdbSearchItem[],
          query,
        );
        const catalogLookup = canViewCatalog
          ? buildCatalogTitleLookupByMetadataId(
              await lookupCatalogTitlesForMetadataResults(rankedMatches),
            )
          : {};
        const matches = rankedMatches.filter(
          (item: MetadataTvdbSearchItem) => !isMetadataResultCataloged(catalogLookup, item),
        );
        setTvdbCandidates(matches);
        setGlobalStatus(matches.length ? t("status.foundTvdb", { count: matches.length }) : t("status.nothingFound"));
        return matches;
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.apiError"));
        setTvdbCandidates([]);
        return [];
      }
    },
    [
      client,
      canViewCatalog,
      lookupCatalogTitlesForMetadataResults,
      mapFacetToTvdbType,
      queueFacet,
      setGlobalStatus,
      sortByRelevance,
      t,
      uiLanguage,
    ],
  );

  const runMetadataAutocomplete = useCallback(
    async (query: string, options?: { surfaceErrors?: boolean }) => {
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
        setCatalogTitlesByTvdbId((previous) =>
          Object.keys(previous).length === 0 ? previous : emptyCatalogTitlesByTvdbId,
        );
        if (options?.surfaceErrors !== false) {
          setGlobalStatus(t("label.ready"));
        }
        return;
      }

      const requestId = ++autocompleteRequestId.current;
      setSearching(true);
      setCatalogSearchLoading(canViewCatalog);
      setMetadataSearchLoading(true);

      // Abort previous in-flight autocomplete HTTP requests so cancellation
      // propagates through Rust all the way to the SMG database query.
      autocompleteAbortRef.current?.abort();
      const abortController = new AbortController();
      autocompleteAbortRef.current = abortController;
      const abortableFetch = makeAbortableFetch(abortController.signal);
      let directCatalogEntries: TitleRecord[] = [];
      let promotedCatalogEntries: TitleRecord[] = [];
      if (!canViewCatalog) {
        setCatalogSearchResults((previous) => (previous.length === 0 ? previous : []));
        setCatalogTitlesByTvdbId((previous) =>
          Object.keys(previous).length === 0 ? previous : emptyCatalogTitlesByTvdbId,
        );
      }

      // Fire both queries in parallel but render each result as it arrives
      // so the fast catalog query populates immediately while the metadata
      // spinner keeps spinning.

      const catalogPromise = canViewCatalog
        ? client.query(catalogSearchTitlesQuery, {
            query: trimmed,
            facet: null,
            limit: AUTOCOMPLETE_LIMIT,
          }, { fetch: abortableFetch }).toPromise()
            .then(async ({ data, error }) => {
              if (error) throw error;
              if (requestId !== autocompleteRequestId.current) return;
              const catalogEntries = (data?.titles?.items ?? []) as TitleRecord[];
              const enriched = await Promise.all(
                catalogEntries.map((title: TitleRecord) => resolveCatalogPosterUrl(title)),
              );
              if (requestId !== autocompleteRequestId.current) return;
              directCatalogEntries = enriched;
              const next = directCatalogEntries.slice(0, AUTOCOMPLETE_LIMIT);
              setCatalogSearchResults((previous) =>
                sameTitleList(previous, next) ? previous : next,
              );
            })
            .finally(() => {
              if (requestId !== autocompleteRequestId.current) return;
              setCatalogSearchLoading(false);
            })
        : Promise.resolve();

      const metadataPromise = client.query(searchMetadataMultiQuery, {
        query: trimmed,
        limit: AUTOCOMPLETE_LIMIT,
        language: uiLanguage,
      }, { fetch: abortableFetch }).toPromise()
        .then(async ({ data, error }) => {
          if (error) throw error;
          if (requestId !== autocompleteRequestId.current) return;
          const multi = data.searchMetadataMulti ?? { movies: [], series: [], anime: [] };
          const rankedMovies = sortByRelevance(
            (multi.movies || []) as MetadataTvdbSearchItem[],
            trimmed,
          );
          const rankedAnime = sortByRelevance(
            (multi.anime || []) as MetadataTvdbSearchItem[],
            trimmed,
          );
          const rankedSeries = sortByRelevance(
            (multi.series || []) as MetadataTvdbSearchItem[],
            trimmed,
          );
          if (canViewCatalog) {
            promotedCatalogEntries = await lookupCatalogTitlesForMetadataResults(
              [...rankedMovies, ...rankedAnime, ...rankedSeries],
              abortableFetch,
            );
          }
          if (requestId !== autocompleteRequestId.current) return;
          const nextCatalogLookup = buildCatalogTitleLookupByMetadataId(promotedCatalogEntries);
          if (canViewCatalog) {
            setCatalogTitlesByTvdbId((previous) =>
              sameCatalogLookup(previous, nextCatalogLookup) ? previous : nextCatalogLookup,
            );
          }
          const movieResults = canViewCatalog
            ? filterCatalogedMetadataResults(rankedMovies, nextCatalogLookup)
            : rankedMovies;
          const animeResults = canViewCatalog
            ? filterCatalogedMetadataResults(rankedAnime, nextCatalogLookup)
            : rankedAnime;
          const animeTvdbIds = new Set(animeResults.map((item) => metadataResultTvdbId(item)));
          const seriesResults = (
            canViewCatalog
              ? filterCatalogedMetadataResults(rankedSeries, nextCatalogLookup)
              : rankedSeries
          ).filter((item) => !animeTvdbIds.has(metadataResultTvdbId(item)));
          const nextMetadata: MetadataSearchResults = {
            movie: movieResults,
            series: seriesResults,
            anime: animeResults,
          };
          await primeCatalogConfigForMetadataActions();
          if (requestId !== autocompleteRequestId.current) return;
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
      if (
        options?.surfaceErrors !== false &&
        catalogResult.status === "rejected" &&
        !isAbortError(catalogResult.reason)
      ) {
        const msg = catalogResult.reason instanceof Error ? catalogResult.reason.message : t("status.apiError");
        setGlobalStatus(msg);
      }
      if (metadataResult.status === "rejected" && !isAbortError(metadataResult.reason)) {
        if (options?.surfaceErrors !== false) {
          const msg = metadataResult.reason instanceof Error ? metadataResult.reason.message : t("status.apiError");
          setGlobalStatus(msg);
        }
        setMetadataSearchResults((prev) => (isMetadataEmpty(prev) ? prev : emptyMetadataSearchResults));
        setCatalogTitlesByTvdbId((previous) =>
          Object.keys(previous).length === 0 ? previous : emptyCatalogTitlesByTvdbId,
        );
        promotedCatalogEntries = [];
      }

      const mergedCatalogEntries = canViewCatalog
        ? mergeCatalogResults(promotedCatalogEntries, directCatalogEntries)
        : [];
      const nextCatalogResults = canViewCatalog
        ? (
            await Promise.all(mergedCatalogEntries.map((title) => resolveCatalogPosterUrl(title)))
          ).slice(0, AUTOCOMPLETE_LIMIT)
        : [];
      if (requestId !== autocompleteRequestId.current) return;
      setCatalogSearchResults((previous) =>
        sameTitleList(previous, nextCatalogResults) ? previous : nextCatalogResults,
      );

      setSearching(false);
    },
    [
      client,
      canViewCatalog,
      emptyCatalogTitlesByTvdbId,
      emptyMetadataSearchResults,
      lookupCatalogTitlesForMetadataResults,
      primeCatalogConfigForMetadataActions,
      resolveCatalogPosterUrl,
      setGlobalStatus,
      sortByRelevance,
      t,
      uiLanguage,
    ],
  );

  const runEmptyCatalogPreload = useCallback(async () => {
    if (!canViewCatalog) {
      setCatalogSearchResults((previous) => (previous.length === 0 ? previous : []));
      setCatalogSearchLoading(false);
      setMetadataSearchLoading(false);
      setSearching(false);
      return;
    }

    const requestId = ++autocompleteRequestId.current;
    setSearching(true);
    setCatalogSearchLoading(true);
    setMetadataSearchLoading(false);

    autocompleteAbortRef.current?.abort();
    const abortController = new AbortController();
    autocompleteAbortRef.current = abortController;
    const abortableFetch = makeAbortableFetch(abortController.signal);

    try {
      const { data, error } = await client.query(catalogSearchTitlesQuery, {
        query: null,
        facet: null,
        limit: EMPTY_QUERY_CATALOG_LIMIT,
      }, { fetch: abortableFetch }).toPromise();
      if (error) throw error;
      if (requestId !== autocompleteRequestId.current) return;

      const next = ((data?.titles?.items ?? []) as TitleRecord[]).slice(0, EMPTY_QUERY_CATALOG_LIMIT);
      setCatalogSearchResults((previous) =>
        sameTitleList(previous, next) ? previous : next,
      );
      setGlobalStatus(t("label.ready"));
    } catch (error) {
      if (requestId !== autocompleteRequestId.current || isAbortError(error)) return;
      const msg = error instanceof Error ? error.message : t("status.apiError");
      setGlobalStatus(msg);
      setCatalogSearchResults((previous) => (previous.length === 0 ? previous : []));
    } finally {
      if (requestId === autocompleteRequestId.current) {
        setCatalogSearchLoading(false);
        setSearching(false);
      }
    }
  }, [canViewCatalog, client, setGlobalStatus, t]);

  useEffect(() => {
    const trimmed = globalSearch.trim();

    if (skipNextAutocompleteQueryRef.current === trimmed) {
      skipNextAutocompleteQueryRef.current = null;
      return;
    }
    skipNextAutocompleteQueryRef.current = null;

    if (trimmed.length < AUTOCOMPLETE_MIN_CHARS) {
      cancelAutocomplete();
      setMetadataSearchResults((previous) => {
        if (previous.movie.length === 0 && previous.series.length === 0 && previous.anime.length === 0) {
          return previous;
        }
        return emptyMetadataSearchResults;
      });
      setCatalogTitlesByTvdbId((previous) =>
        Object.keys(previous).length === 0 ? previous : emptyCatalogTitlesByTvdbId,
      );
      if (
        trimmed.length === 0 &&
        isGlobalSearchPanelOpen &&
        forcedOpenRef.current &&
        canViewCatalog
      ) {
        void runEmptyCatalogPreload();
      } else {
        setCatalogSearchResults((previous) => (previous.length === 0 ? previous : []));
      }
      // Don't auto-close when the panel was force-opened (mobile overlay).
      if (!forcedOpenRef.current) {
        setIsGlobalSearchPanelOpen((isOpen) => (isOpen ? false : isOpen));
      }
      return;
    }

    const debounceTimer = window.setTimeout(() => {
      autocompleteDebounceTimerRef.current = null;
      void runMetadataAutocomplete(trimmed);
    }, AUTOCOMPLETE_DEBOUNCE_MS);
    autocompleteDebounceTimerRef.current = debounceTimer;

    return () => {
      window.clearTimeout(debounceTimer);
      if (autocompleteDebounceTimerRef.current === debounceTimer) {
        autocompleteDebounceTimerRef.current = null;
      }
    };
  }, [
    cancelAutocomplete,
    canViewCatalog,
    emptyCatalogTitlesByTvdbId,
    emptyMetadataSearchResults,
    globalSearch,
    isGlobalSearchPanelOpen,
    runEmptyCatalogPreload,
    runMetadataAutocomplete,
  ]);

  useEffect(() => {
    return () => {
      autocompleteAbortRef.current?.abort();
    };
  }, []);

  const openGlobalSearchPanel = useCallback((force?: boolean) => {
    if (force) {
      forcedOpenRef.current = true;
      setIsGlobalSearchPanelOpen(true);
      return;
    }
    if (globalSearch.trim().length >= AUTOCOMPLETE_MIN_CHARS) {
      setIsGlobalSearchPanelOpen(true);
    }
  }, [globalSearch]);

  const closeGlobalSearchPanel = useCallback(() => {
    forcedOpenRef.current = false;
    setIsGlobalSearchPanelOpen(false);
  }, []);

  const clearGlobalSearchState = useCallback(() => {
    cancelAutocomplete();
    setGlobalSearch("");
    setCatalogSearchResults((previous) => (previous.length === 0 ? previous : []));
    setMetadataSearchResults((previous) =>
      isMetadataEmpty(previous) ? previous : emptyMetadataSearchResults,
    );
    setCatalogTitlesByTvdbId((previous) =>
      Object.keys(previous).length === 0 ? previous : emptyCatalogTitlesByTvdbId,
    );
  }, [
    cancelAutocomplete,
    emptyCatalogTitlesByTvdbId,
    emptyMetadataSearchResults,
  ]);

  const clearGlobalSearch = useCallback(() => {
    forcedOpenRef.current = true;
    clearGlobalSearchState();
    setIsGlobalSearchPanelOpen(true);
  }, [clearGlobalSearchState]);

  const resetGlobalSearch = useCallback(() => {
    forcedOpenRef.current = false;
    clearGlobalSearchState();
    setIsGlobalSearchPanelOpen(false);
  }, [clearGlobalSearchState]);

  useEffect(() => {
    const handleShortcut = (event: KeyboardEvent) => {
      const key = event.key.toLowerCase();
      const isSlashShortcut =
        key === "/" && !event.altKey && !event.ctrlKey && !event.metaKey && !event.shiftKey;
      const isCommandShortcut =
        key === "k" && (event.metaKey || event.ctrlKey) && !event.altKey && !event.shiftKey;

      if (!isSlashShortcut && !isCommandShortcut) {
        return;
      }

      const target = event.target as HTMLElement | null;
      const isTypingTarget =
        target?.tagName === "INPUT" ||
        target?.tagName === "TEXTAREA" ||
        target?.isContentEditable ||
        target?.tagName === "SELECT";
      if (isSlashShortcut && isTypingTarget) {
        return;
      }

      if (hasOpenDialogContent()) {
        event.preventDefault();
        return;
      }

      event.preventDefault();
      if (isCommandShortcut && isGlobalSearchPanelOpen) {
        closeGlobalSearchPanel();
        globalSearchInputRef.current?.blur();
        return;
      }

      forcedOpenRef.current = true;
      setIsGlobalSearchPanelOpen(true);
      window.requestAnimationFrame(() => {
        globalSearchInputRef.current?.focus();
        globalSearchInputRef.current?.select();
      });
    };

    window.addEventListener("keydown", handleShortcut);
    return () => window.removeEventListener("keydown", handleShortcut);
  }, [closeGlobalSearchPanel, isGlobalSearchPanelOpen]);

  const addMetadataSearchResultToCatalog = useCallback(
    async (
      result: MetadataTvdbSearchItem,
      facet: Facet,
      options: MetadataCatalogAddOptions,
      feedback?: CatalogAddFeedback,
    ) => {
      const name = result.name.trim();
      if (!name) {
        setGlobalStatus(t("status.titleRequired"));
        return null;
      }

      const monitored = monitorTypeToMonitored(options.monitorType);

      const externalIds = metadataResultExternalIds(result);
      const requestKey = normalizeCatalogAddRequestKey(facet, externalIds);
      if (pendingCatalogAddKeysRef.current.has(requestKey)) {
        return null;
      }
      pendingCatalogAddKeysRef.current.add(requestKey);
      try {
        const { data: addData, error: addError } = await client.mutation(addTitleMutation, {
          input: {
            name,
            facet,
            libraryId: options.libraryId || undefined,
            monitored,
            tags: [],
            options: {
              qualityProfileId: options.qualityProfileId?.trim() || undefined,
              rootFolderId: options.rootFolderId || undefined,
              monitorType: options.monitorType,
              ...(facet === "MOVIE"
                ? {}
                : { useSeasonFolders: options.seasonFolder }),
              ...(facet === "ANIME"
                ? {
                    monitorSpecials: options.monitorSpecials !== false,
                    interSeasonMovies: options.interSeasonMovies !== false,
                  }
                : {}),
            },
            externalIds,
            smgId: result.smgId ?? undefined,
            tvdbId: metadataResultTvdbId(result) || undefined,
            tmdbId: result.tmdbId ?? undefined,
            imdbId: result.imdbId?.trim() || undefined,
            ...(facet === "MOVIE" && options.minAvailability ? { minAvailability: options.minAvailability } : {}),
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
        const addedName = addData.addTitle.title.name;
        // The status line keeps the full sentence; the toast carries the same
        // news with artwork, so it renders the card instead of the plain text.
        setGlobalStatus(
          t(
            monitored
              ? "status.catalogAddSuccessAutoSearch"
              : "status.catalogAddSuccess",
            { name: addedName },
          ),
          { suppressToast: true },
        );
        const titleId = addData.addTitle?.title?.id?.trim() || null;
        showCatalogAddToast({
          titleName: addedName,
          year: result.year,
          posterUrl: result.posterUrl,
          headline: t("toast.catalogAdded"),
          note: monitored ? t("toast.catalogAddedAutoSearch") : null,
          posterEmptyLabel: t("label.noArt"),
          viewLabel: t("toast.viewInCatalog"),
          dismissLabel: t("label.dismiss"),
          onView:
            titleId && feedback?.onViewInCatalog
              ? () => feedback.onViewInCatalog?.(titleId)
              : undefined,
        });
        // Whatever view is mounted behind the search panel still shows the
        // pre-add catalog, and nothing navigates now, so tell it to reload.
        dispatchCatalogTitlesRefresh({ facet, titleId });
        void runMetadataAutocomplete(globalSearch.trim(), { surfaceErrors: false });
        return titleId;
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.queueFailed"));
        return null;
      } finally {
        pendingCatalogAddKeysRef.current.delete(requestKey);
      }
    },
    [
      globalSearch,
      runMetadataAutocomplete,
      client,
      setGlobalStatus,
      t,
    ],
  );

  const requestMetadataSearchResult = useCallback(
    async (
      result: MetadataTvdbSearchItem,
      facet: Facet,
      options: MetadataCatalogRequestOptions,
    ) => {
      const name = result.name.trim();
      const libraryId = options.libraryId.trim();
      if (!name || !libraryId) {
        setGlobalStatus(t("status.titleRequired"));
        return false;
      }

      const externalIds = metadataResultExternalIds(result);
      const requestKey = normalizeCatalogAddRequestKey(facet, externalIds);
      if (pendingRequestKeysRef.current.has(requestKey)) {
        return false;
      }
      pendingRequestKeysRef.current.add(requestKey);
      try {
        const { error } = await client.mutation(submitMediaRequestMutation, {
          input: {
            libraryId,
            facet,
            title: name,
            externalIds,
            year: result.year ?? undefined,
            overview: result.overview || undefined,
            sortTitle: result.sortTitle || undefined,
            slug: result.slug || undefined,
            runtimeMinutes: result.runtimeMinutes ?? undefined,
            language: result.language || undefined,
            contentStatus: result.status || undefined,
            requestedQualityProfileId: options.requestedQualityProfileId || undefined,
            requestedMonitorType: options.requestedMonitorType || undefined,
          },
        }).toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.requestSubmitted", { name }));
        dispatchNavigationBadgesRefresh();
        void runMetadataAutocomplete(globalSearch.trim(), { surfaceErrors: false });
        return true;
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.queueFailed"));
        return false;
      } finally {
        pendingRequestKeysRef.current.delete(requestKey);
      }
    },
    [client, globalSearch, runMetadataAutocomplete, setGlobalStatus, t],
  );

  /** Force-trigger global search (bypasses autocomplete min-char threshold). */
  const forceSearchGlobal = useCallback(async (queryOverride?: string) => {
    const trimmed = (queryOverride ?? globalSearch).trim();
    if (!trimmed) return;
    cancelAutocomplete();
    if (queryOverride !== undefined && trimmed !== globalSearch.trim()) {
      skipNextAutocompleteQueryRef.current = trimmed;
      setGlobalSearch(trimmed);
    }
    setIsGlobalSearchPanelOpen(true);
    await runMetadataAutocomplete(trimmed);
  }, [cancelAutocomplete, globalSearch, runMetadataAutocomplete]);

  return {
    globalSearch,
    setGlobalSearch,
    globalSearchInputRef,
    searching,
    catalogSearchLoading,
    metadataSearchLoading,
    tvdbCandidates,
    runTvdbSearch,
    forceSearchGlobal,
    setTvdbCandidates,
    catalogSearchResults,
    metadataSearchResults,
    isGlobalSearchPanelOpen,
    openGlobalSearchPanel,
    closeGlobalSearchPanel,
    clearGlobalSearch,
    resetGlobalSearch,
    catalogQualityProfileOptions,
    catalogConfigLoading,
    ensureCatalogConfigReady,
    isCatalogConfigReady,
    resolveDefaultQualityProfileIdForFacet,
    animeCatalogDefaults,
    addMetadataSearchResultToCatalog,
    requestMetadataSearchResult,
    isMetadataSearchResultInCatalog,
    rootFoldersByFacet,
    librariesByFacet: visibleLibrariesByFacet,
    requestableLibrariesByFacet: visibleRequestableLibrariesByFacet,
    queueFacet,
    setQueueFacet,
    catalogChangeSignal,
  };
}

import {
  filterRouteCommandItems,
  type RouteCommandItem,
} from "../common/route-command-types.ts";
import type { Translate } from "./types.ts";
import {
  FACET_REGISTRY,
  type FacetDefinition,
} from "../../lib/facets/registry.ts";
import type { MetadataTvdbSearchItem } from "../../lib/graphql/smg-queries.ts";
import type { MetadataSearchResults } from "../../lib/hooks/use-global-search.ts";
import type {
  Facet,
  LibraryRecord,
  TitleRecord,
} from "../../lib/types/titles.ts";

export type GlobalSearchTabKey = "all" | "library" | "actions" | Facet;
export type GlobalSearchFilterKey = Exclude<GlobalSearchTabKey, "all">;

export const GLOBAL_SEARCH_ALL_CATALOG_RESULT_LIMIT = 4;
export const GLOBAL_SEARCH_ALL_METADATA_RESULT_LIMIT = 6;
export const GLOBAL_SEARCH_ALL_ROUTE_COMMAND_LIMIT = 4;
export const GLOBAL_SEARCH_ALL_ROUTE_COMMAND_DESKTOP_LIMIT = 6;

const GLOBAL_SEARCH_FILTER_ORDER: GlobalSearchFilterKey[] = [
  "library",
  ...FACET_REGISTRY.map((f) => f.id),
  "actions",
];

export type GlobalSearchTab = {
  key: GlobalSearchTabKey;
  label: string;
  count: number;
};

export type CatalogSearchSections = Record<Facet, TitleRecord[]>;

export type VisibleCatalogResult = {
  facet: Facet;
  title: TitleRecord;
};

export type MetadataSearchActionState = {
  isInCatalog: boolean;
  isUnavailable: boolean;
  opensRequestDialog: boolean;
  disabled: boolean;
  actionLabel: string;
  actionTitle: string;
  inlineActionLabel: string;
};

export function isGlobalSearchFilterKey(
  key: GlobalSearchTabKey,
): key is GlobalSearchFilterKey {
  return key !== "all";
}

export function isGlobalSearchFilterSelected(
  selectedFilters: readonly GlobalSearchFilterKey[],
  key: GlobalSearchTabKey,
): boolean {
  return key === "all"
    ? selectedFilters.length === 0
    : selectedFilters.includes(key);
}

export function normalizeGlobalSearchFilterSelection(
  selectedFilters: readonly GlobalSearchFilterKey[],
  availableTabs?: readonly GlobalSearchTab[],
): GlobalSearchFilterKey[] {
  const availableKeys = new Set<GlobalSearchFilterKey>(
    (availableTabs?.map((tab) => tab.key) ?? [
      "all",
      ...GLOBAL_SEARCH_FILTER_ORDER,
    ]).filter(isGlobalSearchFilterKey),
  );
  const selected = new Set(
    selectedFilters.filter((key) => availableKeys.has(key)),
  );
  return GLOBAL_SEARCH_FILTER_ORDER.filter(
    (key) => availableKeys.has(key) && selected.has(key),
  );
}

export function toggleGlobalSearchFilterSelection(
  selectedFilters: readonly GlobalSearchFilterKey[],
  key: GlobalSearchTabKey,
  availableTabs?: readonly GlobalSearchTab[],
): GlobalSearchFilterKey[] {
  if (key === "all") {
    return [];
  }
  const normalized = normalizeGlobalSearchFilterSelection(
    selectedFilters,
    availableTabs,
  );
  const selected = new Set(normalized);
  if (selected.has(key)) {
    selected.delete(key);
  } else {
    selected.add(key);
  }
  return normalizeGlobalSearchFilterSelection([...selected], availableTabs);
}

export function catalogFacetFromString(facet: string): Facet {
  return facet === "MOVIE" ? "MOVIE" : facet === "ANIME" ? "ANIME" : "SERIES";
}

function filterKeyToFacet(key: GlobalSearchFilterKey): Facet | null {
  return FACET_REGISTRY.some((f) => f.id === key) ? (key as Facet) : null;
}

function selectedFacetFilters(
  selectedFilters: readonly GlobalSearchFilterKey[],
): Facet[] {
  return selectedFilters.flatMap((key) => {
    const facet = filterKeyToFacet(key);
    return facet ? [facet] : [];
  });
}

function filtersFromActiveTab(
  activeTab: GlobalSearchTabKey,
): GlobalSearchFilterKey[] {
  return activeTab === "all" ? [] : [activeTab];
}

export function buildMetadataSearchActionState({
  isInCatalog,
  canAdd,
  canRequest,
  resultName,
  t,
}: {
  isInCatalog: boolean;
  canAdd: boolean;
  canRequest: boolean;
  resultName: string;
  t: Translate;
}): MetadataSearchActionState {
  const opensRequestDialog = !canAdd && canRequest;
  const isUnavailable = !isInCatalog && !canAdd && !canRequest;
  const disabled = isInCatalog || isUnavailable;
  const actionLabel = isInCatalog
    ? t("search.alreadyCataloged")
    : isUnavailable
      ? t("search.unavailable")
      : opensRequestDialog
        ? t("search.request")
        : t("search.configureAdd");
  const inlineActionLabel = isInCatalog
    ? t("search.cataloged")
    : isUnavailable
      ? t("search.unavailable")
      : opensRequestDialog
        ? t("search.request")
        : t("search.add");

  return {
    isInCatalog,
    isUnavailable,
    opensRequestDialog,
    disabled,
    actionLabel,
    actionTitle: `${actionLabel}: ${resultName}`,
    inlineActionLabel,
  };
}

export function buildCatalogSearchSections(
  catalogSearchResults: TitleRecord[],
  searchValue: string,
): CatalogSearchSections {
  const query = searchValue.trim().toLowerCase();
  const rank = (title: TitleRecord) => {
    const name = title.name.trim().toLowerCase();
    if (!query || name === query) return 0;
    if (name.startsWith(query)) return 1;
    const matchIndex = name.indexOf(query);
    return matchIndex >= 0 ? 2 + matchIndex : 3 + name.length;
  };

  const buckets = Object.fromEntries(
    FACET_REGISTRY.map((f) => [f.id, [] as TitleRecord[]]),
  ) as CatalogSearchSections;
  // A title held by several libraries is one result, not one per library;
  // the card reads the other libraries from `buildCatalogLibraryMembers`.
  for (const title of dedupeCatalogResultsByIdentity(catalogSearchResults)) {
    buckets[catalogFacetFromString(title.facet)].push(title);
  }
  if (query) {
    for (const facet of FACET_REGISTRY) {
      buckets[facet.id].sort((a, b) => rank(a) - rank(b));
    }
  }
  return buckets;
}

function catalogTitleExternalId(
  title: TitleRecord,
  source: "smg" | "tvdb" | "tmdb" | "imdb",
): string | null {
  const value = (title.externalIds ?? [])
    .find((externalId) => externalId.source.trim().toLowerCase() === source)
    ?.value.trim();
  return value || null;
}

/**
 * The identity a catalog row shares with its copies in other libraries. Two
 * rows are the same title to the searcher when they carry the same SMG or TVDB
 * id within a facet; a row with neither stands alone. The facet is part of the
 * key on purpose: the same show in a series library and an anime library is
 * listed under each section, since each section offers its own libraries.
 */
export function catalogTitleIdentityKey(title: TitleRecord): string {
  const facet = catalogFacetFromString(title.facet);
  const smgId = catalogTitleExternalId(title, "smg");
  if (smgId) {
    return `${facet}|smg:${smgId}`;
  }
  const tvdbId = catalogTitleExternalId(title, "tvdb");
  if (tvdbId) {
    return `${facet}|tvdb:${tvdbId}`;
  }
  return `${facet}|title:${title.id}`;
}

/** Every library row of each title, keyed by identity, in result order. */
export type CatalogLibraryMembers = Record<string, TitleRecord[]>;

export function buildCatalogLibraryMembers(
  titles: TitleRecord[],
): CatalogLibraryMembers {
  const members: CatalogLibraryMembers = {};
  for (const title of titles) {
    const key = catalogTitleIdentityKey(title);
    const existing = members[key];
    if (!existing) {
      members[key] = [title];
    } else if (!existing.some((member) => member.id === title.id)) {
      existing.push(title);
    }
  }
  return members;
}

/** One row per title: the first library row stands for the rest. */
export function dedupeCatalogResultsByIdentity(
  titles: TitleRecord[],
): TitleRecord[] {
  return Object.values(buildCatalogLibraryMembers(titles)).map(
    (members) => members[0],
  );
}

export type CatalogResultLibraryAction = "add" | "request";

/**
 * What one "In Library" card can do beyond viewing (FR: multi-library search).
 *
 * - `members` are the rows that hold the title, one per library; the card
 *   views the only one directly and offers a choice when there are several.
 * - `addableLibraries` and `requestableLibraries` are the libraries of that
 *   facet that do not hold it yet, split by what the viewer may do there.
 * - `action` is the one button the card shows: adding wins over requesting
 *   when both are possible, and neither shows once every library has it,
 *   which is always the case with a single library.
 */
export type CatalogResultPresentation = {
  members: TitleRecord[];
  addableLibraries: LibraryRecord[];
  requestableLibraries: LibraryRecord[];
  action: CatalogResultLibraryAction | null;
};

export function buildCatalogResultPresentation({
  title,
  libraryMembers,
  manageableLibraries,
  requestableLibraries,
  canAdd,
}: {
  title: TitleRecord;
  libraryMembers: CatalogLibraryMembers;
  manageableLibraries: LibraryRecord[];
  requestableLibraries: LibraryRecord[];
  /** False until the add configuration (quality profiles) has loaded. */
  canAdd: boolean;
}): CatalogResultPresentation {
  const members = libraryMembers[catalogTitleIdentityKey(title)] ?? [title];
  const held = new Set(members.map((member) => member.libraryId));
  const addableLibraries = manageableLibraries.filter(
    (library) => !held.has(library.id),
  );
  const requestable = requestableLibraries.filter(
    (library) => !held.has(library.id),
  );
  const action: CatalogResultLibraryAction | null =
    canAdd && addableLibraries.length > 0
      ? "add"
      : requestable.length > 0
        ? "request"
        : null;
  return {
    members,
    addableLibraries,
    requestableLibraries: requestable,
    action,
  };
}

/**
 * The add and request dialogs work from a metadata search result. A catalog
 * row already carries the same identity and the fields the add mutation sends,
 * so a title that is in one library can be offered to another without a second
 * metadata lookup. Null when the row has no metadata identity at all, since
 * nothing could be added or requested from it.
 */
export function metadataSearchItemFromCatalogTitle(
  title: TitleRecord,
): MetadataTvdbSearchItem | null {
  const smgId = catalogTitleExternalId(title, "smg");
  const tvdbId = catalogTitleExternalId(title, "tvdb");
  if (!smgId && !tvdbId) {
    return null;
  }
  const tmdbId = catalogTitleExternalId(title, "tmdb");
  const parsedSmgId = smgId ? Number(smgId) : Number.NaN;
  const parsedTmdbId = tmdbId ? Number(tmdbId) : Number.NaN;
  return {
    tvdbId: tvdbId ?? "",
    smgId: Number.isFinite(parsedSmgId) ? parsedSmgId : null,
    tmdbId: Number.isFinite(parsedTmdbId) ? parsedTmdbId : null,
    name: title.name,
    imdbId: title.imdbId?.trim() || catalogTitleExternalId(title, "imdb"),
    externalIds: (title.externalIds ?? []).map((externalId) => ({
      source: externalId.source,
      value: externalId.value,
    })),
    slug: title.slug ?? null,
    type: null,
    year: title.year ?? null,
    status: title.contentStatus ?? null,
    overview: title.overview ?? null,
    popularity: title.popularity ?? null,
    posterUrl: title.posterUrl ?? null,
    backgroundUrl: title.backgroundUrl ?? null,
    language: title.language ?? null,
    runtimeMinutes: title.runtimeMinutes ?? null,
    sortTitle: title.sortTitle ?? null,
  };
}

export function buildMetadataResultCounts(
  metadataSearchResults: MetadataSearchResults,
): Record<Facet, number> {
  return Object.fromEntries(
    FACET_REGISTRY.map((f) => [
      f.id,
      (metadataSearchResults[f.metadataKey] ?? []).length,
    ]),
  ) as Record<Facet, number>;
}

export function countMetadataResults(
  metadataResultCounts: Record<Facet, number>,
): number {
  return FACET_REGISTRY.reduce(
    (total, f) => total + metadataResultCounts[f.id],
    0,
  );
}

export function filterGlobalSearchRouteCommands(
  routeCommandItems: RouteCommandItem[],
  searchValue: string,
): RouteCommandItem[] {
  if (searchValue.trim().length === 0) {
    return routeCommandItems;
  }
  return filterRouteCommandItems(routeCommandItems, searchValue);
}

export function getVisibleRouteCommandResults(
  activeTab: GlobalSearchTabKey,
  routeCommandResults: RouteCommandItem[],
  allLimit = GLOBAL_SEARCH_ALL_ROUTE_COMMAND_LIMIT,
): RouteCommandItem[] {
  return getVisibleRouteCommandResultsForFilters(
    filtersFromActiveTab(activeTab),
    routeCommandResults,
    allLimit,
  );
}

export function getVisibleRouteCommandResultsForFilters(
  selectedFilters: readonly GlobalSearchFilterKey[],
  routeCommandResults: RouteCommandItem[],
  allLimit = GLOBAL_SEARCH_ALL_ROUTE_COMMAND_LIMIT,
): RouteCommandItem[] {
  if (selectedFilters.length === 0) {
    return routeCommandResults.slice(0, allLimit);
  }
  if (selectedFilters.includes("actions")) {
    return routeCommandResults;
  }
  return [];
}

export function countHiddenRouteCommandResults(
  activeTab: GlobalSearchTabKey,
  routeCommandResults: RouteCommandItem[],
  visibleRouteCommandResults: RouteCommandItem[],
): number {
  return countHiddenRouteCommandResultsForFilters(
    filtersFromActiveTab(activeTab),
    routeCommandResults,
    visibleRouteCommandResults,
  );
}

export function countHiddenRouteCommandResultsForFilters(
  selectedFilters: readonly GlobalSearchFilterKey[],
  routeCommandResults: RouteCommandItem[],
  visibleRouteCommandResults: RouteCommandItem[],
): number {
  if (selectedFilters.length !== 0) {
    return 0;
  }
  return Math.max(
    routeCommandResults.length - visibleRouteCommandResults.length,
    0,
  );
}

export function getVisibleMetadataResults<T>(
  activeTab: GlobalSearchTabKey,
  metadataResults: T[],
  allLimit = GLOBAL_SEARCH_ALL_METADATA_RESULT_LIMIT,
): T[] {
  return getVisibleMetadataResultsForFilters(
    filtersFromActiveTab(activeTab),
    metadataResults,
    allLimit,
  );
}

export function getVisibleMetadataResultsForFilters<T>(
  selectedFilters: readonly GlobalSearchFilterKey[],
  metadataResults: T[],
  allLimit = GLOBAL_SEARCH_ALL_METADATA_RESULT_LIMIT,
): T[] {
  if (selectedFilters.length === 0) {
    return metadataResults.slice(0, allLimit);
  }
  if (selectedFacetFilters(selectedFilters).length === 0) {
    return [];
  }
  return metadataResults;
}

export function countHiddenMetadataResults<T>(
  activeTab: GlobalSearchTabKey,
  metadataResults: T[],
  visibleMetadataResults: T[],
): number {
  return countHiddenMetadataResultsForFilters(
    filtersFromActiveTab(activeTab),
    metadataResults,
    visibleMetadataResults,
  );
}

export function countHiddenMetadataResultsForFilters<T>(
  selectedFilters: readonly GlobalSearchFilterKey[],
  metadataResults: T[],
  visibleMetadataResults: T[],
): number {
  if (selectedFilters.length !== 0) {
    return 0;
  }
  return Math.max(metadataResults.length - visibleMetadataResults.length, 0);
}

export function countHiddenCatalogResults(
  activeTab: GlobalSearchTabKey,
  visibleCatalogCount: number,
  visibleCatalogResults: VisibleCatalogResult[],
): number {
  return countHiddenCatalogResultsForFilters(
    filtersFromActiveTab(activeTab),
    visibleCatalogCount,
    visibleCatalogResults,
  );
}

export function countHiddenCatalogResultsForFilters(
  selectedFilters: readonly GlobalSearchFilterKey[],
  visibleCatalogCount: number,
  visibleCatalogResults: VisibleCatalogResult[],
): number {
  if (selectedFilters.length !== 0) {
    return 0;
  }
  return Math.max(visibleCatalogCount - visibleCatalogResults.length, 0);
}

export function getVisibleCatalogFacets(
  activeTab: GlobalSearchTabKey,
  canViewCatalog: boolean,
): FacetDefinition[] {
  return getVisibleCatalogFacetsForFilters(
    filtersFromActiveTab(activeTab),
    canViewCatalog,
  );
}

export function getVisibleCatalogFacetsForFilters(
  selectedFilters: readonly GlobalSearchFilterKey[],
  canViewCatalog: boolean,
): FacetDefinition[] {
  if (!canViewCatalog) {
    return [];
  }

  if (selectedFilters.length === 0 || selectedFilters.includes("library")) {
    return FACET_REGISTRY;
  }

  const selectedFacets = new Set(selectedFacetFilters(selectedFilters));
  return FACET_REGISTRY.filter((f) => selectedFacets.has(f.id));
}

export function getVisibleMetadataFacets(
  activeTab: GlobalSearchTabKey,
): FacetDefinition[] {
  return getVisibleMetadataFacetsForFilters(filtersFromActiveTab(activeTab));
}

export function getVisibleMetadataFacetsForFilters(
  selectedFilters: readonly GlobalSearchFilterKey[],
): FacetDefinition[] {
  if (selectedFilters.length === 0) {
    return FACET_REGISTRY;
  }

  const selectedFacets = new Set(selectedFacetFilters(selectedFilters));
  return FACET_REGISTRY.filter((f) => selectedFacets.has(f.id));
}

export function getMetadataSectionFacets({
  activeTab,
  metadataSearchLoading,
  metadataResultCounts,
}: {
  activeTab: GlobalSearchTabKey;
  metadataSearchLoading: boolean;
  metadataResultCounts: Record<Facet, number>;
}): FacetDefinition[] {
  return getMetadataSectionFacetsForFilters({
    selectedFilters: filtersFromActiveTab(activeTab),
    metadataSearchLoading,
    metadataResultCounts,
  });
}

export function getMetadataSectionFacetsForFilters({
  selectedFilters,
  metadataSearchLoading,
  metadataResultCounts,
}: {
  selectedFilters: readonly GlobalSearchFilterKey[];
  metadataSearchLoading: boolean;
  metadataResultCounts: Record<Facet, number>;
}): FacetDefinition[] {
  const visibleMetadataFacets =
    getVisibleMetadataFacetsForFilters(selectedFilters);
  return metadataSearchLoading
    ? visibleMetadataFacets
    : visibleMetadataFacets.filter((f) => metadataResultCounts[f.id] > 0);
}

export function countVisibleCatalogResults(
  visibleCatalogFacets: FacetDefinition[],
  catalogSearchSections: CatalogSearchSections,
): number {
  return visibleCatalogFacets.reduce(
    (total, f) => total + (catalogSearchSections[f.id]?.length ?? 0),
    0,
  );
}

export function getVisibleCatalogResults({
  activeTab,
  canViewCatalog,
  catalogSearchSections,
  visibleCatalogFacets,
  allLimit,
}: {
  activeTab: GlobalSearchTabKey;
  canViewCatalog: boolean;
  catalogSearchSections: CatalogSearchSections;
  visibleCatalogFacets: FacetDefinition[];
  allLimit: number;
}): VisibleCatalogResult[] {
  return getVisibleCatalogResultsForFilters({
    selectedFilters: filtersFromActiveTab(activeTab),
    canViewCatalog,
    catalogSearchSections,
    visibleCatalogFacets,
    allLimit,
  });
}

export function getVisibleCatalogResultsForFilters({
  selectedFilters,
  canViewCatalog,
  catalogSearchSections,
  visibleCatalogFacets,
  allLimit,
}: {
  selectedFilters: readonly GlobalSearchFilterKey[];
  canViewCatalog: boolean;
  catalogSearchSections: CatalogSearchSections;
  visibleCatalogFacets: FacetDefinition[];
  allLimit: number;
}): VisibleCatalogResult[] {
  const picked: VisibleCatalogResult[] = [];
  if (!canViewCatalog) {
    return picked;
  }

  if (visibleCatalogFacets.length === 0) {
    return picked;
  }

  const indices: Record<string, number> = {};
  const limit =
    selectedFilters.length === 0 ? allLimit : Number.POSITIVE_INFINITY;
  while (picked.length < limit) {
    let added = false;
    for (const f of visibleCatalogFacets) {
      if (picked.length >= limit) break;
      const bucket = catalogSearchSections[f.id] ?? [];
      const idx = indices[f.id] ?? 0;
      if (idx < bucket.length) {
        picked.push({ facet: f.id, title: bucket[idx] });
        indices[f.id] = idx + 1;
        added = true;
      }
    }
    if (!added) break;
  }
  return picked;
}

export function buildGlobalSearchTabs({
  canViewCatalog,
  catalogSearchSections,
  metadataResultCount,
  metadataResultCounts,
  routeCommandResultCount,
  visibleCatalogResultCount,
  t,
}: {
  canViewCatalog: boolean;
  catalogSearchSections: CatalogSearchSections;
  metadataResultCount: number;
  metadataResultCounts: Record<Facet, number>;
  routeCommandResultCount: number;
  visibleCatalogResultCount: number;
  t: Translate;
}): GlobalSearchTab[] {
  return [
    {
      key: "all",
      label: t("search.tabAll"),
      count:
        visibleCatalogResultCount +
        metadataResultCount +
        routeCommandResultCount,
    },
    ...(canViewCatalog
      ? [
          {
            key: "library" as GlobalSearchTabKey,
            label: t("search.tabLibrary"),
            count: visibleCatalogResultCount,
          },
        ]
      : []),
    ...FACET_REGISTRY.map((f) => ({
      key: f.id as GlobalSearchTabKey,
      label: t(f.navLabelKey),
      count:
        (canViewCatalog ? (catalogSearchSections[f.id]?.length ?? 0) : 0) +
        metadataResultCounts[f.id],
    })),
    ...(routeCommandResultCount > 0
      ? [
          {
            key: "actions" as GlobalSearchTabKey,
            label: t("search.actionsAndSettings"),
            count: routeCommandResultCount,
          },
        ]
      : []),
  ];
}

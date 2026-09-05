import { userTitleTags } from "./title-tags.ts";

export type TitleCatalogQuickFilters = {
  monitored: boolean;
  unmonitored: boolean;
  continuing: boolean;
  ended: boolean;
};

export type TitleCatalogAdvancedFilters = {
  rootFolderIds: string[];
  genreTagKeys: string[];
  themeTagKeys: string[];
  /**
   * Administrator-defined user tags, any-of. Distinct from the two canonical
   * tag key lists above: those are SMG-derived genre and theme keys, these are
   * labels from the title-tag registry and travel as
   * `TitleCatalogFilterInput.tags`.
   */
  userTagLabels: string[];
  minimumYear: number | null;
  maximumYear: number | null;
  minimumRating: number | null;
};

export type TitleCatalogSortStateLike = {
  key: string;
  direction: string;
};

export type TitleCatalogProjection = {
  library: boolean;
  quality: boolean;
  size: boolean;
  episodes: boolean;
  runtime: boolean;
  root: boolean;
  ratings: boolean;
  movieMedia: boolean;
  popularity: boolean;
};

export type TitleCatalogQueryOptions = {
  facet: string;
  libraryIds: string[];
  query: string;
  filters: TitleCatalogQuickFilters;
  advancedFilters?: TitleCatalogAdvancedFilters;
  sort: TitleCatalogSortStateLike;
  projection?: TitleCatalogProjection;
  limit: number;
  offset: number;
};

export const EMPTY_TITLE_QUICK_FILTERS: TitleCatalogQuickFilters = {
  monitored: false,
  unmonitored: false,
  continuing: false,
  ended: false,
};

export const EMPTY_TITLE_ADVANCED_FILTERS: TitleCatalogAdvancedFilters = {
  rootFolderIds: [],
  genreTagKeys: [],
  themeTagKeys: [],
  userTagLabels: [],
  minimumYear: null,
  maximumYear: null,
  minimumRating: null,
};

const EMPTY_TITLE_CATALOG_PROJECTION: TitleCatalogProjection = {
  library: false,
  quality: false,
  size: false,
  episodes: false,
  runtime: false,
  root: false,
  ratings: false,
  movieMedia: false,
  popularity: false,
};

const TITLE_CATALOG_SORT_KEYS: Record<string, string> = {
  name: "TITLE",
  library: "LIBRARY",
  monitored: "MONITORED",
  quality: "QUALITY",
  episodes: "EPISODES",
  status: "STATUS",
  added: "ADDED",
  size: "SIZE",
  year: "YEAR",
  runtime: "RUNTIME",
  root: "ROOT",
  popularity: "POPULARITY",
  resolution: "MEDIA_RESOLUTION",
  hdr: "MEDIA_HDR",
  audioCodec: "MEDIA_AUDIO_CODEC",
  ratingScryer: "RATING_SCRYER",
  ratingImdb: "RATING_IMDB",
  ratingRottenTomatoes: "RATING_ROTTEN_TOMATOES",
  ratingPopcornmeter: "RATING_POPCORNMETER",
  ratingMetacritic: "RATING_METACRITIC",
  ratingMetacriticUser: "RATING_METACRITIC_USER",
  ratingLetterboxd: "RATING_LETTERBOXD",
  ratingTmdb: "RATING_TMDB",
  ratingTvdb: "RATING_TVDB",
  ratingTrakt: "RATING_TRAKT",
  ratingMyanimelist: "RATING_MYANIMELIST",
  ratingAnilist: "RATING_ANILIST",
  ratingAnidb: "RATING_ANIDB",
  ratingMdblist: "RATING_MDBLIST",
};

const SHARED_RATING_COLUMN_KEYS = new Set([
  "ratingImdb",
  "ratingRottenTomatoes",
  "ratingPopcornmeter",
  "ratingMetacritic",
  "ratingMetacriticUser",
  "ratingLetterboxd",
  "ratingTmdb",
  "ratingTvdb",
  "ratingTrakt",
  "ratingMdblist",
]);

const ANIME_RATING_COLUMN_KEYS = new Set([
  "ratingImdb",
  "ratingTmdb",
  "ratingTvdb",
  "ratingTrakt",
  "ratingMyanimelist",
  "ratingAnilist",
  "ratingAnidb",
  "ratingMdblist",
]);

function normalizedFacet(facet: string) {
  const lowered = facet.toLowerCase();
  return lowered === "movie" || lowered === "series" || lowered === "anime"
    ? lowered
    : null;
}

export function titleCatalogSortInput(sort: TitleCatalogSortStateLike) {
  const key = TITLE_CATALOG_SORT_KEYS[sort.key] ?? "SIZE";

  return {
    key,
    // Normalize here so stale persisted sort state (pre-0.17 lowercase) can
    // never reach the SortDirectionValue enum argument.
    direction: sort.direction.toUpperCase() === "DESC" ? "DESC" : "ASC",
  };
}

export function titleCatalogFilterInput(
  filters: TitleCatalogQuickFilters,
  advancedFilters: TitleCatalogAdvancedFilters = EMPTY_TITLE_ADVANCED_FILTERS,
) {
  const monitored =
    filters.monitored === filters.unmonitored
      ? null
      : filters.monitored
        ? true
        : false;
  const contentStatuses = [
    filters.continuing ? "CONTINUING" : null,
    filters.ended ? "ENDED" : null,
  ].filter((value): value is string => Boolean(value));

  const rootFolderIds = [...advancedFilters.rootFolderIds].sort();
  const genreTagKeys = [...advancedFilters.genreTagKeys].sort();
  const themeTagKeys = [...advancedFilters.themeTagKeys].sort();
  // User tags reach the wire in the registry's own normal form, so a stale
  // persisted filter and a freshly picked one produce the same query key.
  const tags = userTitleTags(advancedFilters.userTagLabels);
  const minimumRating =
    advancedFilters.minimumRating != null && advancedFilters.minimumRating > 0
      ? advancedFilters.minimumRating
      : null;

  if (
    monitored === null &&
    contentStatuses.length === 0 &&
    rootFolderIds.length === 0 &&
    genreTagKeys.length === 0 &&
    themeTagKeys.length === 0 &&
    tags.length === 0 &&
    advancedFilters.minimumYear === null &&
    advancedFilters.maximumYear === null &&
    minimumRating === null
  ) {
    return null;
  }

  return {
    monitored,
    contentStatuses,
    ...(rootFolderIds.length > 0 ? { rootFolderIds } : {}),
    ...(genreTagKeys.length > 0 ? { genreTagKeys } : {}),
    ...(themeTagKeys.length > 0 ? { themeTagKeys } : {}),
    ...(tags.length > 0 ? { tags } : {}),
    ...(advancedFilters.minimumYear !== null
      ? { minimumYear: advancedFilters.minimumYear }
      : {}),
    ...(advancedFilters.maximumYear !== null
      ? { maximumYear: advancedFilters.maximumYear }
      : {}),
    ...(minimumRating !== null ? { minimumRating } : {}),
  };
}

export function titleCatalogProjectionSignature(
  projection: TitleCatalogProjection | undefined,
) {
  const normalized = projection ?? EMPTY_TITLE_CATALOG_PROJECTION;
  return [
    normalized.library && "library",
    normalized.quality && "quality",
    normalized.size && "size",
    normalized.episodes && "episodes",
    normalized.runtime && "runtime",
    normalized.root && "root",
    normalized.ratings && "ratings",
    normalized.movieMedia && "movieMedia",
    normalized.popularity && "popularity",
  ]
    .filter(Boolean)
    .join(":");
}

export function titleCatalogProjectionForTable({
  facet,
  visibleColumns,
  sort,
}: {
  facet: string;
  visibleColumns: Partial<Record<string, boolean>>;
  sort: TitleCatalogSortStateLike;
}): TitleCatalogProjection {
  const next = { ...EMPTY_TITLE_CATALOG_PROJECTION };
  const activeFacet = normalizedFacet(facet);
  const supportedRatingColumnKeys =
    activeFacet === "anime" ? ANIME_RATING_COLUMN_KEYS : SHARED_RATING_COLUMN_KEYS;
  const selectedOrSorted = (key: string) =>
    visibleColumns[key] === true || sort.key === key;
  const anyRatingSelectedOrSorted = Object.keys(visibleColumns).some(
    (key) => supportedRatingColumnKeys.has(key) && visibleColumns[key] === true,
  ) || supportedRatingColumnKeys.has(sort.key);

  next.library = selectedOrSorted("library");
  next.quality = selectedOrSorted("quality");
  next.size = selectedOrSorted("size");
  next.episodes = activeFacet !== "movie" && selectedOrSorted("episodes");
  next.runtime = selectedOrSorted("runtime");
  next.root = selectedOrSorted("root");
  next.ratings = anyRatingSelectedOrSorted;
  next.movieMedia =
    activeFacet === "movie" &&
    (selectedOrSorted("resolution") ||
      selectedOrSorted("hdr") ||
      selectedOrSorted("audioCodec"));
  next.popularity = activeFacet === "movie" && selectedOrSorted("popularity");

  return next;
}

export function titleCatalogQueryKey({
  facet,
  query,
  libraryIds,
  filters,
  advancedFilters,
  sort,
  projection,
}: Pick<
  TitleCatalogQueryOptions,
  | "facet"
  | "query"
  | "libraryIds"
  | "filters"
  | "advancedFilters"
  | "sort"
  | "projection"
>) {
  return JSON.stringify({
    facet,
    query: query.trim(),
    libraryIds: [...libraryIds].sort(),
    filter: titleCatalogFilterInput(filters, advancedFilters),
    sort: titleCatalogSortInput(sort),
    projection: titleCatalogProjectionSignature(projection),
  });
}

export function buildTitleCatalogQueryVariables({
  facet,
  libraryIds,
  query,
  filters,
  advancedFilters,
  sort,
  limit,
  offset,
}: TitleCatalogQueryOptions) {
  return {
    facet,
    libraryIds: libraryIds.length > 0 ? libraryIds : null,
    query: query.trim() || null,
    filter: titleCatalogFilterInput(filters, advancedFilters),
    sort: titleCatalogSortInput(sort),
    limit,
    offset,
  };
}

import assert from "node:assert/strict";
import test from "node:test";

import {
  EMPTY_TITLE_ADVANCED_FILTERS,
  EMPTY_TITLE_QUICK_FILTERS,
  buildTitleCatalogQueryVariables,
  titleCatalogProjectionForTable,
  titleCatalogQueryKey,
  titleCatalogSortInput,
  type TitleCatalogAdvancedFilters,
} from "./title-catalog-query.ts";

test("title catalog variables include quick filters like 0.16.6", () => {
  const variables = buildTitleCatalogQueryVariables({
    facet: "series",
    libraryIds: ["series-main"],
    query: " Fringe ",
    filters: {
      monitored: true,
      unmonitored: false,
      continuing: true,
      ended: false,
    },
    sort: { key: "name", direction: "asc" },
    limit: 300,
    offset: 0,
  });

  assert.deepEqual(variables, {
    facet: "series",
    libraryIds: ["series-main"],
    query: "Fringe",
    filter: {
      monitored: true,
      contentStatuses: ["CONTINUING"],
    },
    sort: { key: "TITLE", direction: "ASC" },
    limit: 300,
    offset: 0,
  });
});

test("title catalog variables include normalized advanced filters", () => {
  const variables = buildTitleCatalogQueryVariables({
    facet: "movie",
    libraryIds: ["library-1"],
    query: "",
    filters: EMPTY_TITLE_QUICK_FILTERS,
    advancedFilters: {
      rootFolderIds: ["root-2", "root-1"],
      genreTagKeys: ["canonical:genre:beta", "canonical:genre:alpha"],
      themeTagKeys: ["canonical:theme:sample"],
      userTagLabels: ["Needs  Review", "keep"],
      minimumYear: 2000,
      maximumYear: 2020,
      minimumRating: 7.5,
      presences: [],
      needsAttention: false,
    },
    sort: { key: "name", direction: "asc" },
    limit: 72,
    offset: 0,
  });

  assert.deepEqual(variables.filter, {
    monitored: null,
    contentStatuses: [],
    rootFolderIds: ["root-1", "root-2"],
    genreTagKeys: ["canonical:genre:alpha", "canonical:genre:beta"],
    themeTagKeys: ["canonical:theme:sample"],
    tags: ["keep", "needs review"],
    minimumYear: 2000,
    maximumYear: 2020,
    minimumRating: 7.5,
  });
});

test("a user-tag filter alone is enough to send a filter input", () => {
  const variables = buildTitleCatalogQueryVariables({
    facet: "series",
    libraryIds: [],
    query: "",
    filters: EMPTY_TITLE_QUICK_FILTERS,
    advancedFilters: {
      ...EMPTY_TITLE_ADVANCED_FILTERS,
      userTagLabels: ["keep"],
    },
    sort: { key: "name", direction: "asc" },
    limit: 72,
    offset: 0,
  });

  assert.deepEqual(variables.filter, {
    monitored: null,
    contentStatuses: [],
    tags: ["keep"],
  });
});

test("a presence filter alone is enough to send a filter input", () => {
  const variables = buildTitleCatalogQueryVariables({
    facet: "series",
    libraryIds: [],
    query: "",
    filters: EMPTY_TITLE_QUICK_FILTERS,
    advancedFilters: {
      ...EMPTY_TITLE_ADVANCED_FILTERS,
      presences: ["PARTIAL"],
    },
    sort: { key: "name", direction: "asc" },
    limit: 72,
    offset: 0,
  });

  assert.deepEqual(variables.filter, {
    monitored: null,
    contentStatuses: [],
    presences: ["PARTIAL"],
  });
});

test("the attention toggle is the only boolean that reaches the wire as true", () => {
  const forAttention = (needsAttention: boolean) =>
    buildTitleCatalogQueryVariables({
      facet: "movie",
      libraryIds: [],
      query: "",
      filters: EMPTY_TITLE_QUICK_FILTERS,
      advancedFilters: { ...EMPTY_TITLE_ADVANCED_FILTERS, needsAttention },
      sort: { key: "name", direction: "asc" },
      limit: 72,
      offset: 0,
    }).filter;

  assert.deepEqual(forAttention(true), {
    monitored: null,
    contentStatuses: [],
    needsAttention: true,
  });
  // Off is the absence of the filter, not a request for titles without problems.
  assert.equal(forAttention(false), null);
});

test("the query key separates presences and ignores picking none", () => {
  const base = {
    facet: "series",
    query: "",
    libraryIds: [],
    filters: EMPTY_TITLE_QUICK_FILTERS,
    sort: { key: "name", direction: "asc" },
  };
  const key = (presences: TitleCatalogAdvancedFilters["presences"]) =>
    titleCatalogQueryKey({
      ...base,
      advancedFilters: { ...EMPTY_TITLE_ADVANCED_FILTERS, presences },
    });

  assert.notEqual(key(["MISSING"]), key([]));
  assert.notEqual(key(["MISSING"]), key(["COMPLETE"]));
  assert.notEqual(key(["MISSING"]), key(["PARTIAL"]));
  // Two picks are one filter however they were clicked, which is why the list is
  // sorted before it reaches the key.
  assert.equal(key(["PARTIAL", "MISSING"]), key(["MISSING", "PARTIAL"]));
  assert.equal(key([]), key([]));
});

test("user-tag filters round-trip through the query key and drop reserved entries", () => {
  const base = {
    facet: "movie",
    query: "",
    libraryIds: [],
    filters: EMPTY_TITLE_QUICK_FILTERS,
    sort: { key: "name", direction: "asc" },
  };

  const unfilteredKey = titleCatalogQueryKey({
    ...base,
    advancedFilters: EMPTY_TITLE_ADVANCED_FILTERS,
  });
  const taggedKey = titleCatalogQueryKey({
    ...base,
    advancedFilters: {
      ...EMPTY_TITLE_ADVANCED_FILTERS,
      userTagLabels: ["keep"],
    },
  });
  // Selection order and spelling are not part of the filter's identity, but
  // which labels were picked is.
  const reorderedKey = titleCatalogQueryKey({
    ...base,
    advancedFilters: {
      ...EMPTY_TITLE_ADVANCED_FILTERS,
      userTagLabels: ["archive", "KEEP"],
    },
  });
  const sameSetKey = titleCatalogQueryKey({
    ...base,
    advancedFilters: {
      ...EMPTY_TITLE_ADVANCED_FILTERS,
      userTagLabels: ["keep", "archive"],
    },
  });
  // A reserved entry can never reach the wire, so it cannot change the key.
  const reservedKey = titleCatalogQueryKey({
    ...base,
    advancedFilters: {
      ...EMPTY_TITLE_ADVANCED_FILTERS,
      userTagLabels: ["scryer:quality-profile:hd"],
    },
  });

  assert.notEqual(unfilteredKey, taggedKey);
  assert.notEqual(taggedKey, reorderedKey);
  assert.equal(reorderedKey, sameSetKey);
  assert.equal(reservedKey, unfilteredKey);
});

test("title catalog query key changes when advanced filters change", () => {
  const base = {
    facet: "series",
    query: "",
    libraryIds: [],
    filters: EMPTY_TITLE_QUICK_FILTERS,
    sort: { key: "name", direction: "asc" },
  };

  const unfilteredKey = titleCatalogQueryKey({
    ...base,
    advancedFilters: EMPTY_TITLE_ADVANCED_FILTERS,
  });
  const filteredKey = titleCatalogQueryKey({
    ...base,
    advancedFilters: {
      ...EMPTY_TITLE_ADVANCED_FILTERS,
      rootFolderIds: ["root-1"],
    },
  });

  assert.notEqual(unfilteredKey, filteredKey);
});

test("title catalog variables send all libraries as null", () => {
  const variables = buildTitleCatalogQueryVariables({
    facet: "movie",
    libraryIds: [],
    query: "",
    filters: EMPTY_TITLE_QUICK_FILTERS,
    sort: { key: "added", direction: "desc" },
    limit: 300,
    offset: 0,
  });

  assert.equal(variables.libraryIds, null);
  assert.equal(variables.query, null);
  assert.equal(variables.filter, null);
  assert.deepEqual(variables.sort, { key: "ADDED", direction: "DESC" });
});

test("title catalog query key changes when quick filters change", () => {
  const base = {
    facet: "anime",
    query: "",
    libraryIds: [],
    sort: { key: "name", direction: "asc" },
  };

  const unfilteredKey = titleCatalogQueryKey({
    ...base,
    filters: EMPTY_TITLE_QUICK_FILTERS,
  });
  const filteredKey = titleCatalogQueryKey({
    ...base,
    filters: {
      ...EMPTY_TITLE_QUICK_FILTERS,
      ended: true,
    },
  });

  assert.notEqual(unfilteredKey, filteredKey);
});

test("title catalog sort input maps optional table columns", () => {
  assert.deepEqual(titleCatalogSortInput({ key: "runtime", direction: "desc" }), {
    key: "RUNTIME",
    direction: "DESC",
  });
  assert.deepEqual(
    titleCatalogSortInput({ key: "ratingMetacriticUser", direction: "desc" }),
    {
      key: "RATING_METACRITIC_USER",
      direction: "DESC",
    },
  );
  assert.deepEqual(
    titleCatalogSortInput({ key: "audioCodec", direction: "asc" }),
    {
      key: "MEDIA_AUDIO_CODEC",
      direction: "ASC",
    },
  );
});

test("title catalog projection requests only visible or sorted optional fields", () => {
  const movieProjection = titleCatalogProjectionForTable({
    facet: "movie",
    visibleColumns: {
      resolution: true,
      ratingImdb: true,
      episodes: true,
    },
    sort: { key: "popularity", direction: "desc" },
  });

  assert.equal(movieProjection.movieMedia, true);
  assert.equal(movieProjection.ratings, true);
  assert.equal(movieProjection.popularity, true);
  assert.equal(movieProjection.episodes, false);
  assert.equal(movieProjection.quality, false);
  assert.equal(movieProjection.profile, false);

  const seriesProjection = titleCatalogProjectionForTable({
    facet: "series",
    visibleColumns: {
      runtime: true,
      episodes: true,
      popularity: true,
      hdr: true,
    },
    sort: { key: "name", direction: "asc" },
  });

  assert.equal(seriesProjection.runtime, true);
  assert.equal(seriesProjection.episodes, true);
  assert.equal(seriesProjection.popularity, false);
  assert.equal(seriesProjection.movieMedia, false);
});

test("title catalog projection splits media quality from the quality profile", () => {
  const movieProjection = titleCatalogProjectionForTable({
    facet: "movie",
    visibleColumns: { quality: true },
    sort: { key: "profile", direction: "asc" },
  });
  assert.equal(movieProjection.quality, true);
  assert.equal(movieProjection.profile, true);

  // Series and anime have no media Quality column, only Profile.
  const seriesProjection = titleCatalogProjectionForTable({
    facet: "series",
    visibleColumns: { quality: true, profile: true },
    sort: { key: "name", direction: "asc" },
  });
  assert.equal(seriesProjection.quality, false);
  assert.equal(seriesProjection.profile, true);

  assert.deepEqual(
    titleCatalogSortInput({ key: "profile", direction: "asc" }),
    { key: "PROFILE", direction: "ASC" },
  );
  assert.deepEqual(
    titleCatalogSortInput({ key: "quality", direction: "asc" }),
    { key: "QUALITY", direction: "ASC" },
  );
});

test("title catalog projection ignores ratings unsupported by active facet", () => {
  const projection = titleCatalogProjectionForTable({
    facet: "series",
    visibleColumns: {
      ratingAnilist: true,
      ratingAnidb: true,
    },
    sort: { key: "ratingAnilist", direction: "desc" },
  });

  assert.equal(projection.ratings, false);
});

test("title catalog query key changes when projection changes", () => {
  const base = {
    facet: "movie",
    query: "",
    libraryIds: [],
    filters: EMPTY_TITLE_QUICK_FILTERS,
    sort: { key: "name", direction: "asc" },
  };

  const baseKey = titleCatalogQueryKey({
    ...base,
    projection: titleCatalogProjectionForTable({
      facet: "movie",
      visibleColumns: {},
      sort: base.sort,
    }),
  });
  const projectedKey = titleCatalogQueryKey({
    ...base,
    projection: titleCatalogProjectionForTable({
      facet: "movie",
      visibleColumns: { ratingImdb: true },
      sort: base.sort,
    }),
  });

  assert.notEqual(baseKey, projectedKey);
});

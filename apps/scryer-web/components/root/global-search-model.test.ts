import assert from "node:assert/strict";
import test from "node:test";

import type { Translate } from "@/components/root/types";
import type { RouteCommandItem } from "@/components/common/route-command-types";
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import type { Facet, LibraryRecord, TitleRecord } from "@/lib/types";
import {
  buildCatalogLibraryMembers,
  buildCatalogResultPresentation,
  buildCatalogSearchSections,
  catalogTitleIdentityKey,
  dedupeCatalogResultsByIdentity,
  metadataSearchItemFromCatalogTitle,
  representativeCatalogMember,
  buildGlobalSearchTabs,
  buildMetadataSearchActionState,
  buildMetadataResultCounts,
  countHiddenCatalogResults,
  countHiddenCatalogResultsForFilters,
  countHiddenMetadataResults,
  countHiddenMetadataResultsForFilters,
  countHiddenRouteCommandResults,
  countHiddenRouteCommandResultsForFilters,
  countMetadataResults,
  filterGlobalSearchRouteCommands,
  GLOBAL_SEARCH_ALL_CATALOG_RESULT_LIMIT,
  GLOBAL_SEARCH_ALL_METADATA_RESULT_LIMIT,
  GLOBAL_SEARCH_ALL_ROUTE_COMMAND_DESKTOP_LIMIT,
  GLOBAL_SEARCH_ALL_ROUTE_COMMAND_LIMIT,
  getVisibleCatalogFacets,
  getVisibleCatalogFacetsForFilters,
  getVisibleCatalogResults,
  getVisibleCatalogResultsForFilters,
  getVisibleMetadataResults,
  getVisibleMetadataResultsForFilters,
  getVisibleRouteCommandResults,
  getVisibleRouteCommandResultsForFilters,
  isGlobalSearchFilterSelected,
  normalizeGlobalSearchFilterSelection,
  toggleGlobalSearchFilterSelection,
} from "./global-search-model.ts";

const t: Translate = (key) => key;

function title(id: string, name: string, facet: Facet): TitleRecord {
  return {
    id,
    name,
    facet,
    libraryId: `${facet}-library`,
    monitored: true,
    tags: [],
  };
}

function metadata(
  name: string,
  identity: { smgId?: number | null; tvdbId?: string } = {},
): MetadataTvdbSearchItem {
  return {
    smgId: identity.smgId ?? null,
    tvdbId: identity.tvdbId ?? `tvdb-${name}`,
    name,
    imdbId: null,
    slug: null,
    type: null,
    year: null,
    status: null,
    overview: null,
    popularity: null,
    posterUrl: null,
    language: null,
    runtimeMinutes: null,
    sortTitle: null,
  };
}

test("buildCatalogSearchSections buckets by facet and ranks query matches", () => {
  const sections = buildCatalogSearchSections(
    [
      title("m3", "The Green Mile", "MOVIE"),
      title("a1", "Green Green", "ANIME"),
      title("m2", "Green Zone", "MOVIE"),
      title("m1", "Green", "MOVIE"),
      title("s1", "Greenleaf", "SERIES"),
    ],
    "green",
  );

  assert.deepEqual(
    sections.MOVIE.map((entry) => entry.id),
    ["m1", "m2", "m3"],
  );
  assert.deepEqual(
    sections.SERIES.map((entry) => entry.id),
    ["s1"],
  );
  assert.deepEqual(
    sections.ANIME.map((entry) => entry.id),
    ["a1"],
  );
});

test("getVisibleCatalogResults interleaves all-tab library results and preserves type tabs", () => {
  const sections = buildCatalogSearchSections(
    [
      title("m1", "Movie One", "MOVIE"),
      title("m2", "Movie Two", "MOVIE"),
      title("s1", "Series One", "SERIES"),
      title("a1", "Anime One", "ANIME"),
    ],
    "",
  );

  const allRows = getVisibleCatalogResults({
    activeTab: "all",
    canViewCatalog: true,
    catalogSearchSections: sections,
    visibleCatalogFacets: getVisibleCatalogFacets("all", true),
    allLimit: GLOBAL_SEARCH_ALL_CATALOG_RESULT_LIMIT,
  });

  assert.deepEqual(
    allRows.map(({ facet, title: entry }) => `${facet}:${entry.id}`),
    ["MOVIE:m1", "SERIES:s1", "ANIME:a1", "MOVIE:m2"],
  );
  assert.equal(countHiddenCatalogResults("all", 4, allRows), 0);

  const movieRows = getVisibleCatalogResults({
    activeTab: "MOVIE",
    canViewCatalog: true,
    catalogSearchSections: sections,
    visibleCatalogFacets: getVisibleCatalogFacets("MOVIE", true),
    allLimit: 1,
  });

  assert.deepEqual(
    movieRows.map(({ facet, title: entry }) => `${facet}:${entry.id}`),
    ["MOVIE:m1", "MOVIE:m2"],
  );
  assert.equal(countHiddenCatalogResults("MOVIE", 2, movieRows), 0);
});

test("global search filter selection is additive with All as clear", () => {
  const tabs = buildGlobalSearchTabs({
    canViewCatalog: true,
    catalogSearchSections: {
      MOVIE: [],
      SERIES: [],
      ANIME: [],
    },
    metadataResultCount: 0,
    metadataResultCounts: { MOVIE: 0, SERIES: 0, ANIME: 0 },
    routeCommandResultCount: 2,
    visibleCatalogResultCount: 0,
    t,
  });

  let selected = toggleGlobalSearchFilterSelection([], "MOVIE", tabs);
  selected = toggleGlobalSearchFilterSelection(selected, "actions", tabs);

  assert.deepEqual(selected, ["MOVIE", "actions"]);
  assert.equal(isGlobalSearchFilterSelected(selected, "all"), false);
  assert.equal(isGlobalSearchFilterSelected(selected, "MOVIE"), true);
  assert.equal(isGlobalSearchFilterSelected(selected, "actions"), true);

  selected = toggleGlobalSearchFilterSelection(selected, "MOVIE", tabs);
  assert.deepEqual(selected, ["actions"]);
  assert.deepEqual(toggleGlobalSearchFilterSelection(selected, "all", tabs), []);

  assert.deepEqual(
    normalizeGlobalSearchFilterSelection(["MOVIE", "actions"], [
      { key: "all", label: "All", count: 0 },
      { key: "MOVIE", label: "Movies", count: 0 },
    ]),
    ["MOVIE"],
  );
});

test("selection-aware catalog results add selected filters", () => {
  const sections = buildCatalogSearchSections(
    [
      title("m1", "Movie One", "MOVIE"),
      title("m2", "Movie Two", "MOVIE"),
      title("s1", "Series One", "SERIES"),
      title("a1", "Anime One", "ANIME"),
    ],
    "",
  );

  const movieSeriesFacets = getVisibleCatalogFacetsForFilters(
    ["MOVIE", "SERIES"],
    true,
  );
  const selectedRows = getVisibleCatalogResultsForFilters({
    selectedFilters: ["MOVIE", "SERIES"],
    canViewCatalog: true,
    catalogSearchSections: sections,
    visibleCatalogFacets: movieSeriesFacets,
    allLimit: GLOBAL_SEARCH_ALL_CATALOG_RESULT_LIMIT,
  });

  assert.deepEqual(
    selectedRows.map(({ facet, title: entry }) => `${facet}:${entry.id}`),
    ["MOVIE:m1", "SERIES:s1", "MOVIE:m2"],
  );
  assert.equal(
    countHiddenCatalogResultsForFilters(
      ["MOVIE", "SERIES"],
      3,
      selectedRows,
    ),
    0,
  );

  assert.deepEqual(
    getVisibleCatalogFacetsForFilters(["actions"], true).map((f) => f.id),
    [],
  );
});

test("getVisibleRouteCommandResults previews commands in All and shows all commands in Actions", () => {
  const commands: RouteCommandItem[] = Array.from(
    { length: 8 },
    (_, index) => ({
      id: `command-${index}`,
      label: `Command ${index}`,
      description: `Command description ${index}`,
      onSelect: () => {},
    }),
  );

  assert.deepEqual(
    getVisibleRouteCommandResults("all", commands).map((command) => command.id),
    commands
      .slice(0, GLOBAL_SEARCH_ALL_ROUTE_COMMAND_LIMIT)
      .map((command) => command.id),
  );
  assert.deepEqual(
    getVisibleRouteCommandResults(
      "all",
      commands,
      GLOBAL_SEARCH_ALL_ROUTE_COMMAND_DESKTOP_LIMIT,
    ).map((command) => command.id),
    commands
      .slice(0, GLOBAL_SEARCH_ALL_ROUTE_COMMAND_DESKTOP_LIMIT)
      .map((command) => command.id),
  );
  assert.deepEqual(
    getVisibleRouteCommandResults("actions", commands).map(
      (command) => command.id,
    ),
    commands.map((command) => command.id),
  );
  assert.deepEqual(getVisibleRouteCommandResults("MOVIE", commands), []);

  const allPreview = getVisibleRouteCommandResults("all", commands);
  assert.equal(
    countHiddenRouteCommandResults("all", commands, allPreview),
    commands.length - GLOBAL_SEARCH_ALL_ROUTE_COMMAND_LIMIT,
  );
  assert.equal(
    countHiddenRouteCommandResults(
      "all",
      commands,
      getVisibleRouteCommandResults(
        "all",
        commands,
        GLOBAL_SEARCH_ALL_ROUTE_COMMAND_DESKTOP_LIMIT,
      ),
    ),
    commands.length - GLOBAL_SEARCH_ALL_ROUTE_COMMAND_DESKTOP_LIMIT,
  );
  assert.equal(
    countHiddenRouteCommandResults(
      "actions",
      commands,
      getVisibleRouteCommandResults("actions", commands),
    ),
    0,
  );
  assert.equal(
    countHiddenRouteCommandResults("MOVIE", commands, []),
    0,
  );
  assert.deepEqual(
    getVisibleRouteCommandResultsForFilters(["MOVIE", "actions"], commands).map(
      (command) => command.id,
    ),
    commands.map((command) => command.id),
  );
  assert.equal(
    countHiddenRouteCommandResultsForFilters(
      ["MOVIE", "actions"],
      commands,
      commands,
    ),
    0,
  );
});

test("filterGlobalSearchRouteCommands keeps command shortcuts available before typing", () => {
  const commands: RouteCommandItem[] = [
    {
      id: "settings-profile",
      label: "Settings / Profile",
      description: "Profile",
      groupLabel: "Settings",
      keywords: ["settings", "profile", "account"],
      onSelect: () => {},
    },
    {
      id: "wanted-items",
      label: "Wanted / Wanted Items",
      description: "Wanted Items",
      groupLabel: "Automation",
      keywords: ["wanted", "missing"],
      onSelect: () => {},
    },
  ];

  assert.deepEqual(
    filterGlobalSearchRouteCommands(commands, "").map((command) => command.id),
    ["settings-profile", "wanted-items"],
  );
  assert.deepEqual(
    filterGlobalSearchRouteCommands(commands, "profile").map(
      (command) => command.id,
    ),
    ["settings-profile"],
  );
});

test("getVisibleMetadataResults previews rails in All and expands type tabs", () => {
  const results = Array.from({ length: 8 }, (_, index) => `result-${index}`);

  assert.deepEqual(
    getVisibleMetadataResults("all", results),
    results.slice(0, GLOBAL_SEARCH_ALL_METADATA_RESULT_LIMIT),
  );
  assert.deepEqual(getVisibleMetadataResults("MOVIE", results), results);
  assert.deepEqual(getVisibleMetadataResults("library", results), []);
  assert.deepEqual(getVisibleMetadataResults("actions", results), []);

  const allPreview = getVisibleMetadataResults("all", results);
  assert.equal(
    countHiddenMetadataResults("all", results, allPreview),
    results.length - GLOBAL_SEARCH_ALL_METADATA_RESULT_LIMIT,
  );
  assert.equal(
    countHiddenMetadataResults("SERIES", results, results),
    0,
  );
  assert.deepEqual(
    getVisibleMetadataResultsForFilters(["MOVIE", "SERIES"], results),
    results,
  );
  assert.deepEqual(getVisibleMetadataResultsForFilters(["library"], results), []);
  assert.equal(
    countHiddenMetadataResultsForFilters(["MOVIE"], results, results),
    0,
  );
});

test("buildGlobalSearchTabs keeps catalog, metadata, and route command counts aligned", () => {
  const catalogSearchSections = buildCatalogSearchSections(
    [title("m1", "Movie One", "MOVIE"), title("s1", "Series One", "SERIES")],
    "",
  );
  const metadataResultCounts = buildMetadataResultCounts({
    movie: [metadata("Remote Movie", { smgId: 202, tvdbId: "" })],
    series: [metadata("Remote Series"), metadata("Another Series")],
    anime: [],
  });
  const metadataResultCount = countMetadataResults(metadataResultCounts);

  const tabs = buildGlobalSearchTabs({
    canViewCatalog: true,
    catalogSearchSections,
    metadataResultCount,
    metadataResultCounts,
    routeCommandResultCount: 2,
    visibleCatalogResultCount: 2,
    t,
  });

  assert.deepEqual(
    tabs.map((tab) => [tab.key, tab.count]),
    [
      ["all", 7],
      ["library", 2],
      ["MOVIE", 2],
      ["SERIES", 3],
      ["ANIME", 0],
      ["actions", 2],
    ],
  );
  assert.equal(
    tabs.find((tab) => tab.key === "actions")?.label,
    "search.actionsAndSettings",
  );
});

test("buildGlobalSearchTabs hides the actions tab when no commands match", () => {
  const catalogSearchSections = buildCatalogSearchSections(
    [title("m1", "Movie One", "MOVIE")],
    "",
  );
  const metadataResultCounts = buildMetadataResultCounts({
    movie: [],
    series: [],
    anime: [],
  });

  const tabs = buildGlobalSearchTabs({
    canViewCatalog: true,
    catalogSearchSections,
    metadataResultCount: 0,
    metadataResultCounts,
    routeCommandResultCount: 0,
    visibleCatalogResultCount: 1,
    t,
  });

  assert.equal(
    tabs.some((tab) => tab.key === "actions"),
    false,
  );
});

test("buildMetadataSearchActionState preserves add, request, cataloged, and unavailable behavior", () => {
  assert.deepEqual(
    buildMetadataSearchActionState({
      isInCatalog: true,
      canAdd: true,
      canRequest: true,
      resultName: "Cataloged",
      t,
    }),
    {
      isInCatalog: true,
      isUnavailable: false,
      opensRequestDialog: false,
      disabled: true,
      actionLabel: "search.alreadyCataloged",
      actionTitle: "search.alreadyCataloged: Cataloged",
      inlineActionLabel: "search.cataloged",
    },
  );

  assert.equal(
    buildMetadataSearchActionState({
      isInCatalog: false,
      canAdd: false,
      canRequest: true,
      resultName: "Requestable",
      t,
    }).opensRequestDialog,
    true,
  );

  assert.equal(
    buildMetadataSearchActionState({
      isInCatalog: false,
      canAdd: true,
      canRequest: false,
      resultName: "Addable",
      t,
    }).inlineActionLabel,
    "search.add",
  );

  assert.equal(
    buildMetadataSearchActionState({
      isInCatalog: false,
      canAdd: false,
      canRequest: false,
      resultName: "Unavailable",
      t,
    }).disabled,
    true,
  );
});

function libraryTitle(
  id: string,
  name: string,
  facet: Facet,
  libraryId: string,
  identity: { smgId?: string; tvdbId?: string } = {},
): TitleRecord {
  return {
    ...title(id, name, facet),
    libraryId,
    libraryName: `Library ${libraryId}`,
    externalIds: [
      ...(identity.smgId ? [{ source: "smg", value: identity.smgId }] : []),
      ...(identity.tvdbId ? [{ source: "TVDB", value: identity.tvdbId }] : []),
    ],
  };
}

function library(id: string, facet: Facet, isDefault = false): LibraryRecord {
  return { id, facet, name: `Library ${id}`, slug: id, isDefault, roots: [] };
}

test("a copy that was just added to a second library does not become the card's face", () => {
  const settled = {
    ...libraryTitle("t-main", "Sample Show", "ANIME", "anime-main", { tvdbId: "424536" }),
    posterUrl: "/posters/sample.jpg",
    metadataFetchedAt: "2026-01-01T00:00:00Z",
    createdAt: "2025-12-01T00:00:00Z",
  };
  const fresh = {
    ...libraryTitle("t-kids", "Sample Show", "ANIME", "anime-kids", { tvdbId: "424536" }),
    createdAt: "2026-09-11T00:00:00Z",
  };
  // The new row can come back first from the search; the card still wears
  // the settled copy's art.
  assert.equal(representativeCatalogMember([fresh, settled]).id, "t-main");
  assert.deepEqual(
    dedupeCatalogResultsByIdentity([fresh, settled]).map((item) => item.id),
    ["t-main"],
  );
  // With nothing to choose between them, the older copy stands for both.
  const bare = { ...fresh, createdAt: "2026-09-12T00:00:00Z" };
  const older = { ...libraryTitle("t-old", "Sample Show", "ANIME", "anime-old", { tvdbId: "424536" }), createdAt: "2026-09-10T00:00:00Z" };
  assert.equal(representativeCatalogMember([bare, older]).id, "t-old");
  // Both libraries are still listed for the View picker.
  assert.deepEqual(
    buildCatalogLibraryMembers([fresh, settled])[catalogTitleIdentityKey(settled)].map((m) => m.id),
    ["t-kids", "t-main"],
  );
});

test("a title held by several libraries is one In Library result", () => {
  const primary = libraryTitle("t-main", "Sample Show", "ANIME", "anime-main", {
    tvdbId: "424536",
  });
  const secondary = libraryTitle("t-kids", "Sample Show", "ANIME", "anime-kids", {
    tvdbId: "424536",
  });
  // The same show catalogued as a series is its own result: series libraries
  // are offered under the series section, not the anime one.
  const asSeries = libraryTitle("t-series", "Sample Show", "SERIES", "series-main", {
    tvdbId: "424536",
  });
  const loner = libraryTitle("t-alone", "Sample Loner", "ANIME", "anime-main");

  assert.equal(catalogTitleIdentityKey(primary), catalogTitleIdentityKey(secondary));
  assert.notEqual(catalogTitleIdentityKey(primary), catalogTitleIdentityKey(asSeries));
  // Without any metadata id the row can only stand for itself.
  assert.equal(catalogTitleIdentityKey(loner), "ANIME|title:t-alone");
  // An SMG id wins over the TVDB id, matching how the metadata lookup keys.
  assert.equal(
    catalogTitleIdentityKey(
      libraryTitle("t-movie", "Sample Film", "MOVIE", "movies", {
        smgId: "77",
        tvdbId: "1",
      }),
    ),
    "MOVIE|smg:77",
  );

  const members = buildCatalogLibraryMembers([primary, secondary, asSeries, loner, secondary]);
  assert.deepEqual(
    members[catalogTitleIdentityKey(primary)]?.map((member) => member.id),
    ["t-main", "t-kids"],
  );
  assert.deepEqual(
    dedupeCatalogResultsByIdentity([primary, secondary, asSeries, loner]).map(
      (entry) => entry.id,
    ),
    ["t-main", "t-series", "t-alone"],
  );
  const sections = buildCatalogSearchSections([primary, secondary, asSeries, loner], "");
  assert.deepEqual(
    sections.ANIME.map((entry) => entry.id),
    ["t-main", "t-alone"],
  );
  assert.deepEqual(
    sections.SERIES.map((entry) => entry.id),
    ["t-series"],
  );
});

test("an In Library card offers the libraries that do not hold the title yet", () => {
  const main = library("anime-main", "ANIME", true);
  const kids = library("anime-kids", "ANIME");
  const archive = library("anime-archive", "ANIME");
  const held = libraryTitle("t-main", "Sample Show", "ANIME", "anime-main", {
    tvdbId: "424536",
  });
  const alsoHeld = libraryTitle("t-kids", "Sample Show", "ANIME", "anime-kids", {
    tvdbId: "424536",
  });
  const libraryMembers = buildCatalogLibraryMembers([held, alsoHeld]);

  // A manager adds; the library it already sits in is never offered.
  const manager = buildCatalogResultPresentation({
    title: held,
    libraryMembers,
    manageableLibraries: [main, kids, archive],
    requestableLibraries: [],
    canAdd: true,
  });
  assert.deepEqual(
    manager.members.map((member) => member.id),
    ["t-main", "t-kids"],
  );
  assert.deepEqual(
    manager.addableLibraries.map((entry) => entry.id),
    ["anime-archive"],
  );
  assert.equal(manager.action, "add");

  // Adding waits for the add configuration; requesting does not need it.
  const managerBeforeConfig = buildCatalogResultPresentation({
    title: held,
    libraryMembers,
    manageableLibraries: [main, kids, archive],
    requestableLibraries: [archive],
    canAdd: false,
  });
  assert.equal(managerBeforeConfig.action, "request");
  assert.deepEqual(
    managerBeforeConfig.requestableLibraries.map((entry) => entry.id),
    ["anime-archive"],
  );

  // A requester only ever requests, and only for a library without it.
  const requester = buildCatalogResultPresentation({
    title: held,
    libraryMembers,
    manageableLibraries: [],
    requestableLibraries: [main, archive],
    canAdd: false,
  });
  assert.equal(requester.action, "request");
  assert.deepEqual(
    requester.requestableLibraries.map((entry) => entry.id),
    ["anime-archive"],
  );

  // Every library already has it: nothing to add or request, only a choice
  // of where to view it.
  const everywhere = buildCatalogResultPresentation({
    title: held,
    libraryMembers,
    manageableLibraries: [main, kids],
    requestableLibraries: [main, kids],
    canAdd: true,
  });
  assert.equal(everywhere.action, null);
  assert.equal(everywhere.members.length, 2);

  // A single-library setup keeps today's behaviour: the title is in the only
  // library, so the card just views it.
  const single = buildCatalogResultPresentation({
    title: held,
    libraryMembers: buildCatalogLibraryMembers([held]),
    manageableLibraries: [main],
    requestableLibraries: [main],
    canAdd: true,
  });
  assert.equal(single.action, null);
  assert.deepEqual(
    single.members.map((member) => member.id),
    ["t-main"],
  );

  // A row the members map has never seen still stands for itself.
  const unknown = buildCatalogResultPresentation({
    title: held,
    libraryMembers: {},
    manageableLibraries: [main, kids],
    requestableLibraries: [],
    canAdd: true,
  });
  assert.deepEqual(
    unknown.members.map((member) => member.id),
    ["t-main"],
  );
  assert.deepEqual(
    unknown.addableLibraries.map((entry) => entry.id),
    ["anime-kids"],
  );
});

test("a catalog row becomes the search item the add and request dialogs take", () => {
  const row: TitleRecord = {
    ...libraryTitle("t-main", "Sample Show", "ANIME", "anime-main", {
      smgId: "9001",
      tvdbId: "424536",
    }),
    year: 2019,
    slug: "sample-show",
    sortTitle: "sample show",
    contentStatus: "Continuing",
    overview: "Synthetic overview.",
    posterUrl: "https://example.invalid/poster.jpg",
    runtimeMinutes: 24,
    language: "jpn",
  };
  row.externalIds = [
    ...(row.externalIds ?? []),
    { source: "tmdb", value: "555" },
    { source: "imdb", value: "tt0000001" },
  ];

  const item = metadataSearchItemFromCatalogTitle(row);
  assert.ok(item);
  assert.equal(item.tvdbId, "424536");
  assert.equal(item.smgId, 9001);
  assert.equal(item.tmdbId, 555);
  assert.equal(item.imdbId, "tt0000001");
  assert.equal(item.name, "Sample Show");
  assert.equal(item.year, 2019);
  assert.equal(item.status, "Continuing");
  assert.equal(item.overview, "Synthetic overview.");
  assert.equal(item.posterUrl, "https://example.invalid/poster.jpg");
  assert.equal(item.runtimeMinutes, 24);
  assert.equal(item.language, "jpn");
  assert.equal(item.sortTitle, "sample show");
  assert.equal(item.slug, "sample-show");
  assert.equal(item.externalIds?.length, 4);

  // A movie known only to SMG still carries an identity the dialogs accept.
  const movie = metadataSearchItemFromCatalogTitle(
    libraryTitle("t-movie", "Sample Film", "MOVIE", "movies", { smgId: "77" }),
  );
  assert.equal(movie?.tvdbId, "");
  assert.equal(movie?.smgId, 77);

  // Without any metadata id there is nothing to add or request from.
  assert.equal(
    metadataSearchItemFromCatalogTitle(
      libraryTitle("t-alone", "Sample Loner", "ANIME", "anime-main"),
    ),
    null,
  );
});

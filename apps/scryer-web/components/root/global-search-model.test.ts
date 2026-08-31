import assert from "node:assert/strict";
import test from "node:test";

import type { Translate } from "@/components/root/types";
import type { RouteCommandItem } from "@/components/common/route-command-types";
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import type { Facet, TitleRecord } from "@/lib/types";
import {
  buildCatalogSearchSections,
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

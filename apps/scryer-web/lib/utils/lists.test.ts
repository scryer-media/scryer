import assert from "node:assert/strict";
import test from "node:test";

import type {
  ListProviderManifest,
  ListProviderSettingField,
  ListSubscriptionDraft,
  TitleListMembership,
} from "../types/lists.ts";
import {
  defaultListRoute,
  draftToSubscribeInput,
  EMPTY_LIST_FILTER,
  findListFilter,
  splitListValues,
  withListFilter,
  exclusionInputFromTitle,
  isListModeSelectable,
  listCoverageSegments,
  listDraftProblems,
  listIntervalParts,
  listProviderSettingChanges,
  listProviderSettingsMissing,
  titleListProvenance,
  listMembershipStateLabelKey,
  listMembershipStateTone,
  listModeLabelKey,
  listSyncStateTone,
  listMembershipRowId,
  listSyncPollDelayMs,
  listSyncWatchSettled,
  LIST_SYNC_POLL_BUDGET_MS,
  listUrlPatternToRegExp,
  missingSourceParams,
  parseExternalIdList,
  PUBLIC_LIST_MODES,
  publicProviders,
  recognizeListUrl,
} from "./lists.ts";

function manifest(overrides: Partial<ListProviderManifest> = {}): ListProviderManifest {
  return {
    providerType: "sample",
    name: "Sample Lists",
    summary: null,
    blurb: null,
    tile: { bg: "#123456", ink: "#ffffff", abbr: "SL" },
    coverage: ["MOVIE", "SERIES"],
    groups: [
      {
        label: "Charts",
        authBadge: "NO_ACCOUNT",
        items: [
          {
            id: "trending",
            name: "Trending",
            description: null,
            kinds: ["MOVIE"],
            sourceType: "chart:trending",
            params: [],
            personal: false,
            defaultIntervalSeconds: 21_600,
          },
          {
            id: "mine",
            name: "My picks",
            description: null,
            kinds: ["MOVIE"],
            sourceType: "watchlist",
            params: [],
            personal: true,
            defaultIntervalSeconds: 3_600,
          },
        ],
      },
      {
        label: "Your account",
        authBadge: "MEMBER_ACCOUNT",
        items: [
          {
            id: "status",
            name: "Status",
            description: null,
            kinds: ["SERIES"],
            sourceType: "status:planning",
            params: [],
            personal: false,
            defaultIntervalSeconds: 3_600,
          },
        ],
      },
    ],
    notes: [],
    urlPatterns: [
      {
        pattern: "(?i)^https://lists\\.example\\.test/u/(?P<owner>[^/]+)/l/(?P<slug>[^/?#]+)",
        sourceType: "user_list",
        captures: [
          { group: "owner", param: "owner" },
          { group: "slug", param: "list" },
        ],
      },
    ],
    configFields: [],
    ...overrides,
  };
}

test("coverage segments follow a fixed order, skip empty counts and never overflow", () => {
  const segments = listCoverageSegments({
    total: 10,
    inLibrary: 4,
    added: 2,
    requested: 0,
    held: 1,
    filtered: 0,
    excluded: 1,
    unresolved: 0,
  });
  assert.deepEqual(
    segments.map((segment) => [segment.key, segment.fraction]),
    [
      ["inLibrary", 0.4],
      ["added", 0.2],
      ["held", 0.1],
      ["excluded", 0.1],
    ],
  );

  const shrunk = listCoverageSegments({
    total: 2,
    inLibrary: 3,
    added: 1,
    requested: 0,
    held: 0,
    filtered: 0,
    excluded: 0,
    unresolved: 0,
  });
  assert.equal(shrunk.reduce((acc, segment) => acc + segment.fraction, 0), 1);
  assert.deepEqual(
    listCoverageSegments({ total: 0, inLibrary: 0, added: 0, requested: 0, held: 0, filtered: 0, excluded: 0, unresolved: 0 }),
    [],
  );
});

test("server URL patterns become JavaScript patterns with named groups", () => {
  const regex = listUrlPatternToRegExp("(?i)^https://x\\.test/(?P<id>\\d+)$");
  assert.ok(regex);
  assert.equal(regex.flags, "i");
  assert.equal(regex.exec("HTTPS://X.TEST/42")?.groups?.id, "42");
  assert.equal(listUrlPatternToRegExp("(unclosed"), null);
});

test("a recognised URL yields the provider source with captured parameters", () => {
  const recognition = recognizeListUrl(
    "  https://lists.example.test/u/sample-owner/l/weekend%20picks?sort=rank ",
    [manifest()],
  );
  assert.ok(recognition);
  assert.equal(recognition.manifest.providerType, "sample");
  assert.deepEqual(recognition.source, {
    provider: "sample",
    sourceType: "user_list",
    params: [
      { key: "owner", value: "sample-owner" },
      { key: "list", value: "weekend picks" },
    ],
    url: "https://lists.example.test/u/sample-owner/l/weekend%20picks?sort=rank",
  });
  assert.equal(recognizeListUrl("https://elsewhere.test/list/1", [manifest()]), null);
  assert.equal(recognizeListUrl("   ", [manifest()]), null);
});

test("the public catalog drops member-account groups and personal items", () => {
  const [provider] = publicProviders([manifest()]);
  assert.deepEqual(
    provider.groups.map((group) => [group.label, group.items.map((item) => item.id)]),
    [["Charts", ["trending"]]],
  );
  const accountOnly = manifest({
    providerType: "account-only",
    groups: [{ label: "Yours", authBadge: "MEMBER_ACCOUNT", items: manifest().groups[1].items }],
  });
  assert.deepEqual(publicProviders([accountOnly]), []);
});

test("public modes exclude member requests and discover cannot be picked yet", () => {
  assert.equal(PUBLIC_LIST_MODES.includes("REQUEST"), false);
  assert.equal(isListModeSelectable("DISCOVER"), false);
  assert.equal(isListModeSelectable("HOLD"), true);
  assert.equal(listModeLabelKey("SEARCH"), "lists.mode.search");
});

test("state labels and tones cover every sync and membership state", () => {
  assert.equal(listSyncStateTone("FAIL"), "negative");
  assert.equal(listSyncStateTone("OK"), "positive");
  assert.equal(listMembershipStateLabelKey("BLOCKED_PERMISSION"), "lists.membershipState.blockedPermission");
  assert.equal(listMembershipStateLabelKey("IN_LIBRARY"), "lists.membershipState.inLibrary");
  assert.equal(listMembershipStateTone("REJECTED"), "negative");
  assert.equal(listMembershipStateTone("UNRESOLVED"), "warning");
});

test("provider intervals read in the largest whole unit", () => {
  assert.deepEqual(listIntervalParts(86_400 * 2), { key: "lists.interval.days", count: 2 });
  assert.deepEqual(listIntervalParts(21_600), { key: "lists.interval.hours", count: 6 });
  assert.deepEqual(listIntervalParts(5_400), { key: "lists.interval.minutes", count: 90 });
  assert.deepEqual(listIntervalParts(10), { key: "lists.interval.minutes", count: 1 });
});

test("required source parameters must be filled", () => {
  const definitions = [
    { key: "person", label: "Person", type: "TEXT" as const, options: [], required: true },
    { key: "sort", label: "Sort", type: "ENUM" as const, options: ["rank"], required: false },
  ];
  assert.deepEqual(missingSourceParams(definitions, [{ key: "person", value: "  " }]), ["person"]);
  assert.deepEqual(missingSourceParams(definitions, [{ key: "person", value: "sample-person" }]), []);
});

test("a follow draft needs a name, kinds, a selectable mode and a route per kind", () => {
  const libraries = [
    { id: "lib-movies", facet: "MOVIE" as const, isDefault: true, roots: [{ id: "root-a", isDefault: true }] },
  ];
  const draft: ListSubscriptionDraft = {
    name: "Weekend picks",
    kinds: ["MOVIE", "SERIES"],
    mode: "ADD",
    routes: [defaultListRoute("MOVIE", libraries, "MONITORED")],
    filters: [],
    maxPerSync: null,
    onLeave: "KEEP",
  };
  assert.deepEqual(listDraftProblems(draft), ["lists.follow.problem.route"]);
  assert.deepEqual(listDraftProblems({ ...draft, kinds: ["MOVIE"] }), []);
  assert.deepEqual(listDraftProblems({ ...draft, kinds: ["MOVIE"], mode: "DISCOVER" }), [
    "lists.follow.problem.mode",
  ]);
  assert.deepEqual(listDraftProblems({ ...draft, name: " ", kinds: [] }), [
    "lists.follow.problem.name",
    "lists.follow.problem.kinds",
  ]);
});

test("subscribe input is public, drops blank parameters and routes for unfollowed kinds", () => {
  const libraries = [
    { id: "lib-movies", facet: "MOVIE" as const, isDefault: true, roots: [{ id: "root-a", isDefault: true }] },
    { id: "lib-shows", facet: "SERIES" as const, isDefault: true, roots: [] },
  ];
  const input = draftToSubscribeInput(
    {
      provider: "sample",
      sourceType: "user_list",
      params: [
        { key: "owner", value: "sample-owner" },
        { key: "sort", value: "" },
      ],
      url: null,
    },
    {
      name: "  Weekend picks ",
      kinds: ["MOVIE"],
      mode: "SEARCH",
      routes: [defaultListRoute("MOVIE", libraries, "MONITORED"), defaultListRoute("SERIES", libraries, "ALL_EPISODES")],
      filters: [],
      maxPerSync: 5,
      onLeave: "TAG",
    },
  );
  assert.equal(input.scope, "PUBLIC");
  assert.equal(input.name, "Weekend picks");
  assert.deepEqual(input.params, [{ key: "owner", value: "sample-owner" }]);
  assert.deepEqual(input.routes.map((route) => [route.kind, route.libraryId, route.rootFolderId]), [
    ["MOVIE", "lib-movies", "root-a"],
  ]);
  assert.equal(input.maxPerSync, 5);
});

test("deleted titles become all-lists exclusions only when they carry external ids", () => {
  assert.deepEqual(
    exclusionInputFromTitle({
      facet: "MOVIE",
      name: "Sample Feature",
      year: 2001,
      externalIds: [
        { source: "tmdb", value: "101" },
        { source: "imdb", value: " " },
      ],
    }),
    {
      kind: "MOVIE",
      externalIds: [{ source: "tmdb", value: "101" }],
      displayTitle: "Sample Feature",
      year: 2001,
      scope: "ALL_LISTS",
      subscriptionId: null,
    },
  );
  assert.equal(exclusionInputFromTitle({ facet: "SERIES", name: "Sample Show", externalIds: [] }), null);
});

test("typed external ids parse as source:value pairs", () => {
  assert.deepEqual(parseExternalIdList("TMDB:101, imdb:tt0000001  bad :x tvdb:"), [
    { source: "tmdb", value: "101" },
    { source: "imdb", value: "tt0000001" },
  ]);
});

test("filters are replaced by kind and removed with null", () => {
  const first = withListFilter([], "RELEASED_ONLY", EMPTY_LIST_FILTER);
  const second = withListFilter(first, "RELEASE_YEAR", { ...EMPTY_LIST_FILTER, from: 1990 });
  const third = withListFilter(second, "RELEASED_ONLY", null);
  assert.deepEqual(third.map((filter) => [filter.kind, filter.from]), [["RELEASE_YEAR", 1990]]);
  const updated = withListFilter(second, "RELEASE_YEAR", { ...EMPTY_LIST_FILTER, from: 2000 });
  assert.deepEqual(updated.map((filter) => filter.kind), ["RELEASED_ONLY", "RELEASE_YEAR"]);
  assert.equal(findListFilter(updated, "RELEASE_YEAR")?.from, 2000);
  assert.deepEqual(splitListValues(" horror, ,documentary "), ["horror", "documentary"]);
});

test("membership rows get ids scoped to their list", () => {
  assert.equal(listMembershipRowId("sub-1", "tmdb:603"), "list-membership-sub-1-tmdb-603");
  assert.notEqual(listMembershipRowId("sub-1", "tmdb:603"), listMembershipRowId("sub-2", "tmdb:603"));
});

test("a sync watch settles on a new run, a new sync time or a state change", () => {
  const baseline = { lastAt: "2026-01-01T00:00:00Z", state: "OK" as const, runIds: ["run-1"] };
  assert.equal(listSyncWatchSettled(baseline, { ...baseline }), false);
  assert.equal(listSyncWatchSettled(baseline, { ...baseline, runIds: ["run-2", "run-1"] }), true);
  assert.equal(listSyncWatchSettled(baseline, { ...baseline, lastAt: "2026-01-01T00:05:00Z" }), true);
  assert.equal(listSyncWatchSettled(baseline, { ...baseline, state: "FAIL" }), true);
  // Without panel data the run ids cannot decide either way.
  assert.equal(listSyncWatchSettled({ ...baseline, runIds: null }, { ...baseline, runIds: ["run-9"] }), false);
  // A first sync settles once the list has a sync time.
  assert.equal(
    listSyncWatchSettled(
      { lastAt: null, state: "NEW", runIds: [] },
      { lastAt: null, state: "NEW", runIds: [] },
    ),
    false,
  );
  assert.equal(
    listSyncWatchSettled(
      { lastAt: null, state: "NEW", runIds: null },
      { lastAt: "2026-01-01T00:05:00Z", state: "NEW", runIds: null },
    ),
    true,
  );
});

test("sync polling backs off to a ceiling and stops at its budget", () => {
  assert.deepEqual(
    [0, 1, 2, 3, 4, 5, 9].map((attempt) => listSyncPollDelayMs(attempt, 0)),
    [1_000, 2_000, 3_000, 5_000, 8_000, 10_000, 10_000],
  );
  assert.equal(listSyncPollDelayMs(6, LIST_SYNC_POLL_BUDGET_MS - 10_000), 10_000);
  assert.equal(listSyncPollDelayMs(6, LIST_SYNC_POLL_BUDGET_MS - 9_999), null);
});

function settingField(overrides: Partial<ListProviderSettingField> = {}): ListProviderSettingField {
  return {
    key: "base_url",
    label: "Base URL",
    helpText: null,
    type: "STRING",
    required: false,
    secret: false,
    isSet: false,
    value: null,
    ...overrides,
  };
}

test("provider settings send only the fields the user changed", () => {
  const fields = [
    settingField({ key: "base_url", isSet: true, value: "https://feeds.example.test" }),
    settingField({ key: "region", isSet: true, value: "north" }),
    settingField({ key: "api_key", type: "PASSWORD", secret: true, isSet: true }),
  ];
  assert.deepEqual(listProviderSettingChanges(fields, {}), []);
  assert.deepEqual(
    listProviderSettingChanges(fields, {
      base_url: { value: " https://feeds.example.test ", clear: false },
      region: { value: "south", clear: false },
    }),
    [{ key: "region", value: "south" }],
  );
});

test("a blank plain value clears it but a blank secret keeps the stored one", () => {
  const fields = [
    settingField({ key: "region", isSet: true, value: "north" }),
    settingField({ key: "api_key", type: "PASSWORD", secret: true, isSet: true }),
  ];
  assert.deepEqual(
    listProviderSettingChanges(fields, {
      region: { value: "  ", clear: false },
      api_key: { value: "", clear: false },
    }),
    [{ key: "region", value: null }],
  );
  assert.deepEqual(listProviderSettingChanges(fields, { api_key: { value: "", clear: true } }), [
    { key: "api_key", value: null },
  ]);
  assert.deepEqual(listProviderSettingChanges(fields, { api_key: { value: " fixture-key ", clear: false } }), [
    { key: "api_key", value: "fixture-key" },
  ]);
});

test("a required field is missing until something is stored or typed", () => {
  const fields = [
    settingField({ key: "api_key", type: "PASSWORD", secret: true, required: true }),
    settingField({ key: "region", required: true, isSet: true, value: "north" }),
  ];
  assert.deepEqual(listProviderSettingsMissing(fields, {}), ["api_key"]);
  assert.deepEqual(listProviderSettingsMissing(fields, { api_key: { value: "fixture-key", clear: false } }), []);
  assert.deepEqual(listProviderSettingsMissing(fields, { region: { value: "", clear: false } }), [
    "api_key",
    "region",
  ]);
});

function titleMembership(overrides: Partial<TitleListMembership> = {}): TitleListMembership {
  return {
    subscriptionId: "list-a",
    name: "Fixture list A",
    state: "IN_LIBRARY",
    addedByList: false,
    leftAt: null,
    ...overrides,
  };
}

test("a title names the list that added it while that list still holds it", () => {
  assert.equal(titleListProvenance([]), null);
  assert.equal(titleListProvenance([titleMembership()]), null);
  assert.deepEqual(
    titleListProvenance([
      titleMembership({ subscriptionId: "list-a", name: "Fixture list A", addedByList: true, leftAt: "2026-01-02T00:00:00Z" }),
      titleMembership({ subscriptionId: "list-b", name: "Fixture list B", addedByList: true, state: "ADDED" }),
    ]),
    { kind: "added", name: "Fixture list B" },
  );
});

test("once every adding list dropped the title it shows the one it left last", () => {
  assert.deepEqual(
    titleListProvenance([
      titleMembership({ subscriptionId: "list-a", name: "Fixture list A", addedByList: true, leftAt: "2026-01-02T00:00:00Z" }),
      titleMembership({ subscriptionId: "list-b", name: "Fixture list B", addedByList: true, leftAt: "2026-03-04T00:00:00Z" }),
      titleMembership({ subscriptionId: "list-c", name: "Fixture list C" }),
    ]),
    { kind: "left", name: "Fixture list B" },
  );
});

import assert from "node:assert/strict";
import test from "node:test";

import type {
  ListProviderManifest,
  ListProviderSettingField,
  ListSourceDraft,
  ListSubscription,
  ListSubscriptionDraft,
  MemberListPolicy,
  TitleListMembership,
} from "../types/lists.ts";
import {
  DEFAULT_LIST_MAX_PER_SYNC,
  defaultListRoute,
  draftToSubscribeInput,
  emptyListDraft,
  draftToUpdateInput,
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
  rollBackMemberListPolicy,
  titleListProvenance,
  listMembershipStateLabelKey,
  listMembershipStateTone,
  listModeLabelKey,
  listSyncStateTone,
  listMembershipRowId,
  listSyncPollDelayMs,
  listSyncWatchSchedule,
  listSyncWatchSettled,
  LIST_SYNC_POLL_BUDGET_MS,
  listUrlPatternToRegExp,
  missingSourceParams,
  parseExternalIdList,
  PUBLIC_LIST_MODES,
  publicProviders,
  recognizeListUrl,
  titleListMembershipChanged,
  followListOfferedKinds,
  subscriptionToDraft,
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

test("a malformed percent sequence in a pasted URL keeps the raw capture instead of throwing", () => {
  for (const [slug, expected] of [
    ["weekend%", "weekend%"],
    ["weekend%2", "weekend%2"],
    ["weekend%zz", "weekend%zz"],
    ["%E0%A4%A", "%E0%A4%A"],
  ] as const) {
    const recognition = recognizeListUrl(`https://lists.example.test/u/sample%20owner/l/${slug}`, [manifest()]);
    assert.ok(recognition, slug);
    assert.equal(recognition.source.sourceType, "user_list");
    assert.deepEqual(recognition.source.params, [
      { key: "owner", value: "sample owner" },
      { key: "list", value: expected },
    ]);
  }
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

test("one sync poll follows every queued list and drops each at its own budget", () => {
  const now = 1_000_000;
  const many = new Map(Array.from({ length: 40 }, (_, index) => [`list-${index}`, now] as const));
  const first = listSyncWatchSchedule(0, many, now);
  assert.deepEqual(first && { delay: first.delay, kept: first.keep.length }, { delay: 1_000, kept: 40 });

  const mixed = new Map([
    ["queued-long-ago", now - (LIST_SYNC_POLL_BUDGET_MS - 5_000)],
    ["queued-just-now", now],
  ]);
  assert.deepEqual(listSyncWatchSchedule(5, mixed, now), { delay: 10_000, keep: ["queued-just-now"] });
  assert.deepEqual(listSyncWatchSchedule(0, mixed, now), {
    delay: 1_000,
    keep: ["queued-long-ago", "queued-just-now"],
  });

  assert.equal(listSyncWatchSchedule(0, new Map(), now), null);
  assert.equal(
    listSyncWatchSchedule(6, new Map([["spent", now - LIST_SYNC_POLL_BUDGET_MS]]), now),
    null,
  );
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
    options: [],
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

function typenamePaths(value: unknown, path = "$"): string[] {
  if (Array.isArray(value)) return value.flatMap((entry, index) => typenamePaths(entry, `${path}[${index}]`));
  if (value === null || typeof value !== "object") return [];
  return Object.entries(value).flatMap(([key, entry]) =>
    key === "__typename" ? [`${path}.${key}`] : typenamePaths(entry, `${path}.${key}`),
  );
}

test("inputs built from query results carry no __typename at any depth", () => {
  const subscription = {
    __typename: "ListSubscriptionPayload",
    id: "sub-1",
    scope: "PUBLIC",
    name: "Fixture list",
    providerUrl: null,
    source: {
      __typename: "ListSourcePayload",
      provider: "sample",
      sourceType: "user_list",
      params: [{ __typename: "ListParamPayload", key: "owner", value: "sample-owner" }],
    },
    kinds: ["MOVIE", "SERIES"],
    enabled: true,
    mode: "HOLD",
    routes: [
      {
        __typename: "ListRoutePayload",
        kind: "MOVIE",
        libraryId: "lib-movies",
        qualityProfileId: "qp-1",
        rootFolderId: "root-a",
        monitorType: "MONITORED",
        minAvailability: "announced",
        useSeasonFolders: null,
        releaseNumbering: null,
        tags: ["from-list"],
      },
      {
        __typename: "ListRoutePayload",
        kind: "SERIES",
        libraryId: "lib-shows",
        qualityProfileId: null,
        rootFolderId: null,
        monitorType: "ALL_EPISODES",
        minAvailability: null,
        useSeasonFolders: true,
        releaseNumbering: "AUTO",
        tags: [],
      },
    ],
    filters: [
      {
        __typename: "ListFilterPayload",
        kind: "RATING_AT_LEAST",
        scale: "tmdb",
        value: 7.5,
        from: null,
        to: null,
        values: [],
      },
    ],
    maxPerSync: 3,
    onLeave: "LOG",
    intervalSeconds: 3600,
    sync: { __typename: "ListSyncStatusPayload" },
    counts: { __typename: "ListCountsPayload", total: 0 },
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:00:00Z",
  } as unknown as ListSubscription;

  const draft = subscriptionToDraft(subscription);
  assert.deepEqual(typenamePaths(draft), []);

  const update = draftToUpdateInput(draft);
  assert.deepEqual(typenamePaths(update), []);
  assert.deepEqual(update.routes, [
    {
      kind: "MOVIE",
      libraryId: "lib-movies",
      qualityProfileId: "qp-1",
      rootFolderId: "root-a",
      monitorType: "MONITORED",
      minAvailability: "announced",
      useSeasonFolders: null,
      releaseNumbering: null,
      tags: ["from-list"],
    },
    {
      kind: "SERIES",
      libraryId: "lib-shows",
      qualityProfileId: null,
      rootFolderId: null,
      monitorType: "ALL_EPISODES",
      minAvailability: null,
      useSeasonFolders: true,
      releaseNumbering: "AUTO",
      tags: [],
    },
  ]);
  assert.deepEqual(update.filters, [
    { kind: "RATING_AT_LEAST", scale: "tmdb", value: 7.5, from: null, to: null, values: [] },
  ]);

  const source = {
    provider: subscription.source.provider,
    sourceType: subscription.source.sourceType,
    params: subscription.source.params,
    url: null,
  } satisfies ListSourceDraft;
  const input = draftToSubscribeInput(source, {
    ...draft,
    routes: subscription.routes,
    filters: subscription.filters,
  });
  assert.deepEqual(typenamePaths(input), []);
  assert.deepEqual(input.params, [{ key: "owner", value: "sample-owner" }]);
  assert.equal(input.routes.length, 2);
  assert.equal(input.routes[0]?.qualityProfileId, "qp-1");
  assert.deepEqual(input.filters[0]?.kind, "RATING_AT_LEAST");
  assert.equal(input.mode, "HOLD");
  assert.equal(input.onLeave, "LOG");
  assert.equal(input.maxPerSync, 3);
});

test("a new follow starts with a per-sync cap that can be cleared", () => {
  const draft = emptyListDraft("Fixture list", ["MOVIE"]);
  assert.equal(draft.maxPerSync, DEFAULT_LIST_MAX_PER_SYNC);
  assert.equal(DEFAULT_LIST_MAX_PER_SYNC, 25);
  const cleared = listDraftProblems({ ...draft, maxPerSync: null });
  assert.equal(cleared.includes("lists.follow.problem.maxPerSync"), false);
});

test("the follow form offers the kinds the source declares, keeping kinds already saved", () => {
  const catalogItem = manifest().groups[0].items[0];
  assert.deepEqual(
    followListOfferedKinds({ sourceKinds: catalogItem.kinds, providerCoverage: manifest().coverage }),
    ["MOVIE"],
  );
  assert.deepEqual(
    followListOfferedKinds({
      sourceKinds: ["MOVIE"],
      providerCoverage: ["MOVIE", "SERIES"],
      savedKinds: ["SERIES"],
    }),
    ["MOVIE", "SERIES"],
  );
  assert.deepEqual(
    followListOfferedKinds({ sourceKinds: [], providerCoverage: ["SERIES", "MOVIE"] }),
    ["MOVIE", "SERIES"],
  );
  assert.deepEqual(followListOfferedKinds({}), ["MOVIE", "SERIES", "ANIME"]);
  assert.deepEqual(
    followListOfferedKinds({ requestedKinds: ["ANIME"], sourceKinds: ["MOVIE"], providerCoverage: ["MOVIE"] }),
    ["ANIME"],
  );
});

test("a title's list line refreshes when a list adds or drops that title", () => {
  const event = (eventType: string, titleId: string | null) => ({
    sequence: 1,
    eventId: "event-1",
    eventType,
    titleId,
    facet: null,
    streamKind: null,
    streamId: null,
  });
  const changed = titleListMembershipChanged("title-7");
  assert.equal(changed(event("LIST_TITLE_ADDED", "title-7")), true);
  assert.equal(changed(event("LIST_TITLE_LEFT", "title-7")), true);
  assert.equal(changed(event("LIST_TITLE_ADDED", "title-8")), false);
  assert.equal(changed(event("TITLE_UPDATED", "title-7")), false);
  assert.equal(changed(event("LIST_TITLE_LEFT", null)), false);
  assert.equal(titleListMembershipChanged(null)(event("LIST_TITLE_ADDED", "title-7")), false);
});

test("a failed policy change rolls back only that member's row", () => {
  const member = (id: string, policy: MemberListPolicy["policy"]): MemberListPolicy => ({
    user: { id, username: `member-${id}` },
    policy,
    listRequestsLast30d: 0,
  });
  const before = member("a", "APPROVAL");
  // Member a's change to AUTO is in flight while member b's change has already saved.
  const current = [member("a", "AUTO"), member("b", "NONE")];

  assert.deepEqual(rollBackMemberListPolicy(current, before, "AUTO"), [
    member("a", "APPROVAL"),
    member("b", "NONE"),
  ]);

  // A later change to the same member that already landed is not undone.
  const later = [member("a", "NONE"), member("b", "NONE")];
  assert.deepEqual(rollBackMemberListPolicy(later, before, "AUTO"), later);
});

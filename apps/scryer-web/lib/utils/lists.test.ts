import assert from "node:assert/strict";
import test from "node:test";
import {
  listMembershipMetadataLookup,
  listTitleWithoutYear,
  listMembershipReasonKey,
  listPreviewMetadataLookup,
  listMembershipTitleHref,
  sortListSubscriptions,
} from "./lists.ts";

test("list titles remove only a matching year suffix before rendering the separate year", () => {
  assert.equal(
    listTitleWithoutYear("The Grand Tour (2016)", 2016),
    "The Grand Tour",
  );
  assert.equal(listTitleWithoutYear("INVINCIBLE (2021) ", 2021), "INVINCIBLE");
  assert.equal(listTitleWithoutYear("Example (2021)", 2022), "Example (2021)");
  assert.equal(listTitleWithoutYear("Example (2021)", null), "Example (2021)");
  assert.equal(listTitleWithoutYear("1917", 2019), "1917");
  assert.equal(
    listTitleWithoutYear("Example (Part 2)", 2021),
    "Example (Part 2)",
  );
  assert.equal(listTitleWithoutYear("(2021)", 2021), "(2021)");
});

test("a list entry's reason code resolves to its sentence, and an unknown code to none", () => {
  assert.equal(
    listMembershipReasonKey("media_type_not_included"),
    "lists.reason.mediaTypeNotIncluded",
  );
  assert.equal(
    listMembershipReasonKey("missing_unreleased"),
    "lists.reason.missingReleaseDate",
  );
  assert.equal(listMembershipReasonKey("adult"), "lists.reason.adult");
  assert.equal(
    listMembershipReasonKey("missing_adult"),
    "lists.reason.missingAdult",
  );
  assert.equal(listMembershipReasonKey("constructor"), null);
  assert.equal(listMembershipReasonKey("some_future_reason"), null);
  assert.equal(listMembershipReasonKey(null), null);
});

test("followed lists sort by name, and by last sync as failed, waiting, synced, then switched off", () => {
  const list = (
    name: string,
    state: "OK" | "NEW" | "FAIL",
    lastAt: string | null,
    enabled = true,
  ) => ({
    name,
    enabled,
    sync: {
      state,
      lastAt,
      nextAt: null,
      errorMessage: null,
      errorAt: null,
      pausedUntil: null,
    },
  });
  const lists = [
    list("delta", "OK", "2026-01-02T00:00:00Z"),
    list("Alpha", "OK", "2026-01-03T00:00:00Z"),
    list("charlie", "FAIL", "2026-01-01T00:00:00Z"),
    list("Bravo 10", "OK", "2026-01-04T00:00:00Z", false),
    list("Bravo 2", "NEW", null),
  ];
  const names = (sorted: { name: string }[]) =>
    sorted.map((entry) => entry.name);

  assert.deepEqual(names(sortListSubscriptions(lists, null)), names(lists));
  assert.deepEqual(
    names(sortListSubscriptions(lists, { key: "name", descending: false })),
    ["Alpha", "Bravo 2", "Bravo 10", "charlie", "delta"],
  );
  assert.deepEqual(
    names(sortListSubscriptions(lists, { key: "name", descending: true })),
    ["delta", "charlie", "Bravo 10", "Bravo 2", "Alpha"],
  );
  // A switched-off list sorts as off even though its last sync succeeded.
  assert.deepEqual(
    names(sortListSubscriptions(lists, { key: "sync", descending: false })),
    ["charlie", "Bravo 2", "Alpha", "delta", "Bravo 10"],
  );
  assert.deepEqual(
    names(sortListSubscriptions(lists, { key: "sync", descending: true })),
    ["Bravo 10", "delta", "Alpha", "Bravo 2", "charlie"],
  );
});

test("a list entry is looked up by its matched title, then by the first id of its own kind", () => {
  assert.deepEqual(
    listMembershipMetadataLookup({
      kind: "MOVIE",
      smgTitleId: 41,
      externalIds: [{ source: "imdb", value: "tt0000001" }],
    }),
    { movie: true, input: { smgId: 41 } },
  );
  assert.deepEqual(
    listMembershipMetadataLookup({
      kind: "ANIME",
      smgTitleId: null,
      externalIds: [
        { source: "mal", value: "9001" },
        { source: "tmdb", kind: "movie", value: "77" },
        { source: "TMDB", kind: "series", value: " 88 " },
      ],
    }),
    { movie: false, input: { tmdbId: 88 } },
  );
  assert.deepEqual(
    listMembershipMetadataLookup({
      kind: "SERIES",
      smgTitleId: null,
      externalIds: [
        { source: "tvdb", value: "5005" },
        { source: "imdb", value: "tt0000002" },
      ],
    }),
    { movie: false, input: { tvdbId: "5005" } },
  );
  assert.deepEqual(
    listMembershipMetadataLookup({
      kind: "MOVIE",
      smgTitleId: null,
      externalIds: [
        { source: "tmdb", value: "abc" },
        { source: "imdb", value: "tt0000003" },
      ],
    }),
    { movie: true, input: { imdbId: "tt0000003" } },
  );
  assert.equal(
    listMembershipMetadataLookup({
      kind: "ANIME",
      smgTitleId: null,
      externalIds: [{ source: "anilist", value: "12" }],
    }),
    null,
  );
});

test("a previewed title is looked up by its matched title and kind, and not at all without both", () => {
  assert.deepEqual(
    listPreviewMetadataLookup({ kind: "MOVIE", canonicalSmgId: 7 }),
    { movie: true, input: { smgId: 7 } },
  );
  assert.deepEqual(
    listPreviewMetadataLookup({ kind: "ANIME", canonicalSmgId: 8 }),
    { movie: false, input: { smgId: 8 } },
  );
  assert.equal(
    listPreviewMetadataLookup({ kind: "SERIES", canonicalSmgId: null }),
    null,
  );
  assert.equal(
    listPreviewMetadataLookup({ kind: null, canonicalSmgId: 9 }),
    null,
  );
});

test("membership title links use library IDs for every facet and omit unlinked titles", () => {
  assert.equal(
    listMembershipTitleHref("MOVIE", "movie-1"),
    "/movies?id=movie-1",
  );
  assert.equal(
    listMembershipTitleHref("SERIES", "series-1"),
    "/series?id=series-1",
  );
  assert.equal(
    listMembershipTitleHref("ANIME", "anime-1"),
    "/anime?id=anime-1",
  );
  assert.equal(
    listMembershipTitleHref("ANIME", "parent-1", "movie&1"),
    "/anime?id=parent-1&seriesMovie=movie%261",
  );
  assert.equal(
    listMembershipTitleHref("ANIME", "title&other=1"),
    "/anime?id=title%26other%3D1",
  );
  assert.equal(listMembershipTitleHref("ANIME", null), null);
  assert.equal(listMembershipTitleHref("ANIME", "  "), null);
});

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
  providerLogoSrc,
  defaultListRoute,
  draftToSubscribeInput,
  emptyListDraft,
  draftToUpdateInput,
  EMPTY_LIST_FILTER,
  findListFilter,
  inheritedListEpisodePolicyLabelKey,
  splitListValues,
  withListFilter,
  exclusionFieldsFromMetadataResult,
  exclusionInputFromTitle,
  isListModeSelectable,
  isListSourceFollowed,
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
  LIST_SYNC_RUNS_SHOWN,
  listCoverageSegmentTone,
  listSyncStateLabelKey,
  shownListSyncRuns,
  listSyncStateTone,
  listMembershipRowId,
  listUrlPatternToRegExp,
  missingSourceParams,
  formatExternalIdList,
  parseExternalIdList,
  PUBLIC_LIST_MODES,
  publicProviders,
  recognizeListUrl,
  titleListMembershipChanged,
  latestRead,
  followedListsChanged,
  listAccountsChanged,
  listExclusionsChanged,
  listProviderAppsChanged,
  listProvidersChanged,
  listTabCountsChanged,
  memberListPoliciesChanged,
  followListOfferedKinds,
  isListMediaParam,
  listKindsFromSourceParams,
  subscriptionToDraft,
} from "./lists.ts";

function manifest(
  overrides: Partial<ListProviderManifest> = {},
): ListProviderManifest {
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
        pattern:
          "(?i)^https://lists\\.example\\.test/u/(?P<owner>[^/]+)/l/(?P<slug>[^/?#]+)",
        sourceType: "user_list",
        captures: [
          { group: "owner", param: "owner" },
          { group: "slug", param: "list" },
        ],
      },
    ],
    configFields: [],
    memberKey: null,
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
  assert.equal(
    shrunk.reduce((acc, segment) => acc + segment.fraction, 0),
    1,
  );
  assert.deepEqual(
    listCoverageSegments({
      total: 0,
      inLibrary: 0,
      added: 0,
      requested: 0,
      held: 0,
      filtered: 0,
      excluded: 0,
      unresolved: 0,
    }),
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
  assert.equal(
    recognizeListUrl("https://elsewhere.test/list/1", [manifest()]),
    null,
  );
  assert.equal(recognizeListUrl("   ", [manifest()]), null);
});

test("a malformed percent sequence in a pasted URL keeps the raw capture instead of throwing", () => {
  for (const [slug, expected] of [
    ["weekend%", "weekend%"],
    ["weekend%2", "weekend%2"],
    ["weekend%zz", "weekend%zz"],
    ["%E0%A4%A", "%E0%A4%A"],
  ] as const) {
    const recognition = recognizeListUrl(
      `https://lists.example.test/u/sample%20owner/l/${slug}`,
      [manifest()],
    );
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
    provider.groups.map((group) => [
      group.label,
      group.items.map((item) => item.id),
    ]),
    [["Charts", ["trending"]]],
  );
  const accountOnly = manifest({
    providerType: "account-only",
    groups: [
      {
        label: "Yours",
        authBadge: "MEMBER_ACCOUNT",
        items: manifest().groups[1].items,
      },
    ],
  });
  assert.deepEqual(publicProviders([accountOnly]), []);
});

test("public modes exclude member requests and discover cannot be picked yet", () => {
  assert.equal(PUBLIC_LIST_MODES.includes("REQUEST"), false);
  assert.equal(isListModeSelectable("DISCOVER"), false);
  assert.equal(isListModeSelectable("HOLD"), true);
  assert.equal(listModeLabelKey("ADD"), "lists.mode.add");
});

test("state labels and tones cover every sync and membership state", () => {
  assert.equal(listSyncStateTone("FAIL"), "negative");
  assert.equal(listSyncStateTone("OK"), "positive");
  assert.equal(listSyncStateLabelKey("NEW"), "lists.syncState.new");
  assert.equal(listSyncStateLabelKey("NEW", null), "lists.syncState.new");
  assert.equal(
    listSyncStateLabelKey("NEW", "2026-10-09T12:00:00Z"),
    "lists.syncState.waiting",
  );
  assert.equal(
    listSyncStateLabelKey("OK", "2026-10-09T12:00:00Z"),
    "lists.syncState.ok",
  );
  assert.equal(
    listMembershipStateLabelKey("BLOCKED_PERMISSION"),
    "lists.membershipState.blockedPermission",
  );
  assert.equal(
    listMembershipStateLabelKey("IN_LIBRARY"),
    "lists.membershipState.inLibrary",
  );
  assert.equal(listMembershipStateTone("REJECTED"), "negative");
  assert.equal(listMembershipStateTone("UNRESOLVED"), "warning");
  assert.equal(
    listCoverageSegmentTone("added"),
    listMembershipStateTone("ADDED"),
  );
  assert.equal(
    listCoverageSegmentTone("unresolved"),
    listMembershipStateTone("UNRESOLVED"),
  );
});

test("a source counts as followed only on the same provider, source type and parameters", () => {
  const followed = [
    { source: { provider: "anilist", sourceType: "popular", params: [] } },
    {
      source: {
        provider: "trakt",
        sourceType: "user_list",
        params: [
          { key: "user", value: "someone" },
          { key: "list", value: "favourites" },
        ],
      },
    },
  ];
  assert.equal(
    isListSourceFollowed(followed, {
      provider: "anilist",
      sourceType: "popular",
      params: [],
    }),
    true,
  );
  assert.equal(
    isListSourceFollowed(followed, {
      provider: "anilist",
      sourceType: "trending",
      params: [],
    }),
    false,
  );
  assert.equal(
    isListSourceFollowed(followed, {
      provider: "imdb",
      sourceType: "popular",
      params: [],
    }),
    false,
  );
  assert.equal(
    isListSourceFollowed(followed, {
      provider: "trakt",
      sourceType: "user_list",
      params: [
        { key: "list", value: "favourites " },
        { key: "user", value: "someone" },
      ],
    }),
    true,
  );
  assert.equal(
    isListSourceFollowed(followed, {
      provider: "trakt",
      sourceType: "user_list",
      params: [
        { key: "user", value: "someone" },
        { key: "list", value: "watchlist" },
      ],
    }),
    false,
  );
  assert.equal(
    isListSourceFollowed(followed, {
      provider: "trakt",
      sourceType: "user_list",
      params: [{ key: "user", value: "someone" }],
    }),
    false,
  );
});

test("a list followed through one linked account is not followed through another", () => {
  const followed = [
    {
      accountId: "account-one",
      source: { provider: "mal", sourceType: "plan_to_watch", params: [] },
    },
    {
      accountId: null,
      source: { provider: "mal", sourceType: "top", params: [] },
    },
  ];
  const personal = { provider: "mal", sourceType: "plan_to_watch", params: [] };
  assert.equal(
    isListSourceFollowed(followed, { ...personal, accountId: "account-one" }),
    true,
  );
  assert.equal(
    isListSourceFollowed(followed, { ...personal, accountId: "account-two" }),
    false,
  );
  // A personal follow does not mark the public list of the same source, nor the other way round.
  assert.equal(isListSourceFollowed(followed, personal), false);
  assert.equal(
    isListSourceFollowed(followed, {
      provider: "mal",
      sourceType: "top",
      params: [],
    }),
    true,
  );
  assert.equal(
    isListSourceFollowed(followed, {
      provider: "mal",
      sourceType: "top",
      params: [],
      accountId: "account-one",
    }),
    false,
  );
});

test("personal lists sort by the account they are read through, then by name", () => {
  const list = (name: string, account: string) => ({
    name,
    account,
    enabled: true,
    sync: {
      state: "OK" as const,
      lastAt: null,
      nextAt: null,
      errorMessage: null,
      errorAt: null,
      pausedUntil: null,
    },
  });
  const lists = [
    list("Watching", "second"),
    list("Planning", "First"),
    list("Watching", "First"),
    list("Completed", "second"),
  ];
  const shown = (descending: boolean) =>
    sortListSubscriptions(
      lists,
      { key: "account", descending },
      (entry) => entry.account,
    ).map((entry) => `${entry.account}/${entry.name}`);
  assert.deepEqual(shown(false), [
    "First/Planning",
    "First/Watching",
    "second/Completed",
    "second/Watching",
  ]);
  // Reversing the accounts keeps each account's lists in name order.
  assert.deepEqual(shown(true), [
    "second/Completed",
    "second/Watching",
    "First/Planning",
    "First/Watching",
  ]);
});

test("provider intervals read in the largest whole unit", () => {
  assert.deepEqual(listIntervalParts(86_400 * 2), {
    key: "lists.interval.days",
    count: 2,
  });
  assert.deepEqual(listIntervalParts(21_600), {
    key: "lists.interval.hours",
    count: 6,
  });
  assert.deepEqual(listIntervalParts(5_400), {
    key: "lists.interval.minutes",
    count: 90,
  });
  assert.deepEqual(listIntervalParts(10), {
    key: "lists.interval.minutes",
    count: 1,
  });
});

test("required source parameters must be filled", () => {
  const definitions = [
    {
      key: "person",
      label: "Person",
      type: "TEXT" as const,
      options: [],
      required: true,
    },
    {
      key: "sort",
      label: "Sort",
      type: "ENUM" as const,
      options: ["rank"],
      required: false,
    },
  ];
  assert.deepEqual(
    missingSourceParams(definitions, [{ key: "person", value: "  " }]),
    ["person"],
  );
  assert.deepEqual(
    missingSourceParams(definitions, [
      { key: "person", value: "sample-person" },
    ]),
    [],
  );
});

test("a follow draft needs a name, kinds, a selectable mode and a route per kind", () => {
  const libraries = [
    {
      id: "lib-movies",
      facet: "MOVIE" as const,
      isDefault: true,
      roots: [{ id: "root-a", isDefault: true }],
    },
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
  assert.deepEqual(
    listDraftProblems({ ...draft, kinds: ["MOVIE"], mode: "DISCOVER" }),
    ["lists.follow.problem.mode"],
  );
  assert.deepEqual(listDraftProblems({ ...draft, name: " ", kinds: [] }), [
    "lists.follow.problem.name",
    "lists.follow.problem.kinds",
  ]);
});

test("subscribe input is public, drops blank parameters and routes for unfollowed kinds", () => {
  const libraries = [
    {
      id: "lib-movies",
      facet: "MOVIE" as const,
      isDefault: true,
      roots: [{ id: "root-a", isDefault: true }],
    },
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
      mode: "ADD",
      routes: [
        defaultListRoute("MOVIE", libraries, "MONITORED"),
        defaultListRoute("SERIES", libraries, "ALL_EPISODES"),
      ],
      filters: [],
      maxPerSync: 5,
      onLeave: "TAG",
    },
  );
  assert.equal(input.scope, "PUBLIC");
  assert.equal(input.name, "Weekend picks");
  assert.deepEqual(input.params, [{ key: "owner", value: "sample-owner" }]);
  assert.deepEqual(
    input.routes.map((route) => [
      route.kind,
      route.libraryId,
      route.rootFolderId,
    ]),
    [["MOVIE", "lib-movies", "root-a"]],
  );
  assert.equal(input.maxPerSync, 5);
});

test("a picked search result fills the exclusion form with its name, year and ids", () => {
  const fields = exclusionFieldsFromMetadataResult(
    {
      name: "Sample Film",
      year: 2031,
      tvdbId: "",
      smgId: 41,
      tmdbId: 9001,
      imdbId: "tt0000041",
      externalIds: [{ source: "TMDB", kind: "movie", value: "9001" }],
    },
    "MOVIE",
  );
  assert.deepEqual(fields, {
    title: "Sample Film",
    year: "2031",
    ids: "tmdb:movie:9001, smg:title:41, imdb:tt0000041",
  });
  // The kinds survive the round trip through the form field.
  assert.deepEqual(parseExternalIdList(fields.ids), [
    { source: "tmdb", kind: "movie", value: "9001" },
    { source: "smg", kind: "title", value: "41" },
    { source: "imdb", value: "tt0000041" },
  ]);
  assert.equal(
    exclusionFieldsFromMetadataResult(
      { name: "Sample Show", year: null, tvdbId: "77", imdbId: null },
      "SERIES",
    ).year,
    "",
  );
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
  assert.equal(
    exclusionInputFromTitle({
      facet: "SERIES",
      name: "Sample Show",
      externalIds: [],
    }),
    null,
  );
});

test("typed external ids parse as source:value pairs", () => {
  assert.deepEqual(
    parseExternalIdList("TMDB:101, imdb:tt0000001  bad :x tvdb:"),
    [
      { source: "tmdb", value: "101" },
      { source: "imdb", value: "tt0000001" },
    ],
  );
});

test("typed external ids with a kind parse as source:kind:value", () => {
  assert.deepEqual(
    parseExternalIdList("TMDB:TV:202, tmdb:movie:303 tvdb::404 a:b:c:d"),
    [
      { source: "tmdb", kind: "tv", value: "202" },
      { source: "tmdb", kind: "movie", value: "303" },
    ],
  );
  assert.equal(
    formatExternalIdList([
      { source: "tmdb", kind: "series", value: "505" },
      { source: "tvdb", kind: null, value: "606" },
    ]),
    "tmdb:series:505, tvdb:606",
  );
});

test("filters are replaced by kind and removed with null", () => {
  const first = withListFilter([], "RELEASED_ONLY", EMPTY_LIST_FILTER);
  const second = withListFilter(first, "RELEASE_YEAR", {
    ...EMPTY_LIST_FILTER,
    from: 1990,
  });
  const third = withListFilter(second, "RELEASED_ONLY", null);
  assert.deepEqual(
    third.map((filter) => [filter.kind, filter.from]),
    [["RELEASE_YEAR", 1990]],
  );
  const updated = withListFilter(second, "RELEASE_YEAR", {
    ...EMPTY_LIST_FILTER,
    from: 2000,
  });
  assert.deepEqual(
    updated.map((filter) => filter.kind),
    ["RELEASED_ONLY", "RELEASE_YEAR"],
  );
  assert.equal(findListFilter(updated, "RELEASE_YEAR")?.from, 2000);
  assert.deepEqual(splitListValues(" horror, ,documentary "), [
    "horror",
    "documentary",
  ]);
});

test("membership rows get ids scoped to their list", () => {
  assert.equal(
    listMembershipRowId("sub-1", "tmdb:603"),
    "list-membership-sub-1-tmdb-603",
  );
  assert.notEqual(
    listMembershipRowId("sub-1", "tmdb:603"),
    listMembershipRowId("sub-2", "tmdb:603"),
  );
});

test("a collapsed sync history shows its newest runs and knows when older ones exist", () => {
  const runs = Array.from(
    { length: LIST_SYNC_RUNS_SHOWN + 1 },
    (_, index) => `run-${index}`,
  );
  const collapsed = shownListSyncRuns(runs, false);
  assert.equal(collapsed.runs.length, LIST_SYNC_RUNS_SHOWN);
  assert.equal(collapsed.runs[0], "run-0");
  assert.equal(collapsed.more, true);
  assert.deepEqual(
    shownListSyncRuns(runs.slice(0, LIST_SYNC_RUNS_SHOWN), false),
    {
      runs: runs.slice(0, LIST_SYNC_RUNS_SHOWN),
      more: false,
    },
  );
  assert.deepEqual(shownListSyncRuns(runs, true), { runs, more: false });
});

function settingField(
  overrides: Partial<ListProviderSettingField> = {},
): ListProviderSettingField {
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
    settingField({
      key: "base_url",
      isSet: true,
      value: "https://feeds.example.test",
    }),
    settingField({ key: "region", isSet: true, value: "north" }),
    settingField({
      key: "api_key",
      type: "PASSWORD",
      secret: true,
      isSet: true,
    }),
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
    settingField({
      key: "api_key",
      type: "PASSWORD",
      secret: true,
      isSet: true,
    }),
  ];
  assert.deepEqual(
    listProviderSettingChanges(fields, {
      region: { value: "  ", clear: false },
      api_key: { value: "", clear: false },
    }),
    [{ key: "region", value: null }],
  );
  assert.deepEqual(
    listProviderSettingChanges(fields, { api_key: { value: "", clear: true } }),
    [{ key: "api_key", value: null }],
  );
  assert.deepEqual(
    listProviderSettingChanges(fields, {
      api_key: { value: " fixture-key ", clear: false },
    }),
    [{ key: "api_key", value: "fixture-key" }],
  );
});

test("a required field is missing until something is stored or typed", () => {
  const fields = [
    settingField({
      key: "api_key",
      type: "PASSWORD",
      secret: true,
      required: true,
    }),
    settingField({
      key: "region",
      required: true,
      isSet: true,
      value: "north",
    }),
  ];
  assert.deepEqual(listProviderSettingsMissing(fields, {}), ["api_key"]);
  assert.deepEqual(
    listProviderSettingsMissing(fields, {
      api_key: { value: "fixture-key", clear: false },
    }),
    [],
  );
  assert.deepEqual(
    listProviderSettingsMissing(fields, {
      region: { value: "", clear: false },
    }),
    ["api_key", "region"],
  );
});

function titleMembership(
  overrides: Partial<TitleListMembership> = {},
): TitleListMembership {
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
      titleMembership({
        subscriptionId: "list-a",
        name: "Fixture list A",
        addedByList: true,
        leftAt: "2026-01-02T00:00:00Z",
      }),
      titleMembership({
        subscriptionId: "list-b",
        name: "Fixture list B",
        addedByList: true,
        state: "ADDED",
      }),
    ]),
    { kind: "added", name: "Fixture list B" },
  );
});

test("once every adding list dropped the title it shows the one it left last", () => {
  assert.deepEqual(
    titleListProvenance([
      titleMembership({
        subscriptionId: "list-a",
        name: "Fixture list A",
        addedByList: true,
        leftAt: "2026-01-02T00:00:00Z",
      }),
      titleMembership({
        subscriptionId: "list-b",
        name: "Fixture list B",
        addedByList: true,
        leftAt: "2026-03-04T00:00:00Z",
      }),
      titleMembership({ subscriptionId: "list-c", name: "Fixture list C" }),
    ]),
    { kind: "left", name: "Fixture list B" },
  );
});

function typenamePaths(value: unknown, path = "$"): string[] {
  if (Array.isArray(value))
    return value.flatMap((entry, index) =>
      typenamePaths(entry, `${path}[${index}]`),
    );
  if (value === null || typeof value !== "object") return [];
  return Object.entries(value).flatMap(([key, entry]) =>
    key === "__typename"
      ? [`${path}.${key}`]
      : typenamePaths(entry, `${path}.${key}`),
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
      params: [
        { __typename: "ListParamPayload", key: "owner", value: "sample-owner" },
      ],
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
    {
      kind: "RATING_AT_LEAST",
      scale: "tmdb",
      value: 7.5,
      from: null,
      to: null,
      values: [],
      facet: null,
      matchAny: false,
      minimums: [],
      unresolvedLabels: [],
    },
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

test("media selectors fold into Include across providers without hiding other source controls", () => {
  const param = {
    key: "type",
    label: "Type",
    type: "ENUM" as const,
    options: ["all", "movies", "shows", "anime"],
    required: false,
  };
  assert.equal(isListMediaParam(param), true);
  assert.equal(
    isListMediaParam({ ...param, key: "kind", options: ["movie", "series"] }),
    true,
  );
  assert.equal(
    isListMediaParam({
      ...param,
      key: "status",
      options: ["watching", "completed"],
    }),
    false,
  );
  assert.equal(
    isListMediaParam({
      ...param,
      key: "credit",
      options: ["all", "cast", "crew"],
    }),
    false,
  );
  assert.deepEqual(
    listKindsFromSourceParams(
      ["MOVIE", "SERIES", "ANIME"],
      [param],
      [{ key: "type", value: "shows" }],
      true,
    ),
    ["SERIES"],
  );
  assert.deepEqual(
    listKindsFromSourceParams(
      ["MOVIE", "ANIME"],
      [param],
      [{ key: "type", value: "anime" }],
      true,
    ),
    ["MOVIE", "ANIME"],
  );
  assert.deepEqual(
    listKindsFromSourceParams(
      ["MOVIE", "SERIES"],
      [{ ...param, options: ["movie", "series"] }],
      [],
      true,
    ),
    ["MOVIE"],
  );
  assert.deepEqual(
    listKindsFromSourceParams(
      ["MOVIE", "SERIES"],
      [param],
      [{ key: "type", value: "__include__" }],
      true,
    ),
    ["MOVIE", "SERIES"],
  );
  assert.deepEqual(
    listKindsFromSourceParams(
      ["MOVIE", "SERIES", "ANIME"],
      [{ ...param, options: ["movies", "shows"] }],
      [{ key: "type", value: "shows" }],
      true,
    ),
    ["SERIES", "ANIME"],
  );
});

test("the follow form offers the kinds the source declares, keeping kinds already saved", () => {
  const catalogItem = manifest().groups[0].items[0];
  assert.deepEqual(
    followListOfferedKinds({
      sourceKinds: catalogItem.kinds,
      providerCoverage: manifest().coverage,
    }),
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
    followListOfferedKinds({
      sourceKinds: [],
      providerCoverage: ["SERIES", "MOVIE"],
    }),
    ["MOVIE", "SERIES"],
  );
  assert.deepEqual(followListOfferedKinds({}), ["MOVIE", "SERIES", "ANIME"]);
  assert.deepEqual(
    followListOfferedKinds({
      requestedKinds: ["ANIME"],
      sourceKinds: ["MOVIE"],
      providerCoverage: ["MOVIE"],
    }),
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
  assert.equal(
    titleListMembershipChanged(null)(event("LIST_TITLE_ADDED", "title-7")),
    false,
  );
});

test("each part of a lists surface reads again only for the changes it shows", () => {
  const event = (eventType: string, payloadJson?: unknown) => ({
    sequence: 1,
    eventId: "event-1",
    eventType,
    titleId: null,
    facet: null,
    streamKind: null,
    streamId: null,
    payloadJson,
  });
  const setting = (resourceType: unknown) =>
    event("CONFIGURATION_CHANGED", {
      type: "configuration_changed",
      data: { resource_type: resourceType },
    });

  for (const changed of [
    event("LIST_SYNC_FINISHED"),
    event("LIST_SUBSCRIPTION_CHANGED"),
  ]) {
    assert.equal(followedListsChanged(changed), true);
    assert.equal(listTabCountsChanged(changed), true);
    assert.equal(listExclusionsChanged(changed), false);
    assert.equal(listProvidersChanged(changed), false);
  }
  // Unlinking an account or changing a policy turns a member's lists off.
  assert.equal(followedListsChanged(setting("list_account")), true);
  assert.equal(followedListsChanged(setting("list_member_policy")), true);
  assert.equal(listAccountsChanged(setting("list_account")), true);
  assert.equal(listAccountsChanged(setting("list_exclusion")), false);
  assert.equal(memberListPoliciesChanged(setting("list_member_policy")), true);
  assert.equal(listExclusionsChanged(setting("list_exclusion")), true);
  assert.equal(followedListsChanged(setting("list_exclusion")), false);
  assert.equal(listTabCountsChanged(setting("list_exclusion")), true);
  assert.equal(listProvidersChanged(setting("list_provider_settings")), true);
  assert.equal(listProvidersChanged(setting("list_provider_app")), true);
  assert.equal(
    listProviderAppsChanged(setting("list_provider_settings")),
    false,
  );
  assert.equal(listTabCountsChanged(setting("list_provider_app")), false);

  // Other settings, and a configuration event that names nothing, refresh no part.
  for (const other of [
    setting("indexer"),
    setting(null),
    event("CONFIGURATION_CHANGED"),
    event("CONFIGURATION_CHANGED", "list_exclusion"),
    event("TITLE_UPDATED"),
  ]) {
    assert.equal(listTabCountsChanged(other), false);
    assert.equal(listProvidersChanged(other), false);
    assert.equal(listAccountsChanged(other), false);
    assert.equal(memberListPoliciesChanged(other), false);
  }
});

test("only the newest read lands, and a local change turns reads in flight stale", () => {
  const reads = latestRead();
  const first = reads.begin();
  assert.equal(first(), true);
  const second = reads.begin();
  assert.equal(first(), false, "an earlier read answering late is dropped");
  assert.equal(second(), true);
  reads.invalidate();
  assert.equal(
    second(),
    false,
    "a read started before a local change cannot undo it",
  );
  assert.equal(reads.begin()(), true);
});

test("a failed policy change rolls back only that member's row", () => {
  const member = (
    id: string,
    policy: MemberListPolicy["policy"],
  ): MemberListPolicy => ({
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

test("list providers with a shipped logo resolve to it; the rest keep their abbreviation", () => {
  const expected: Record<string, string> = {
    tmdb: "/rating-sources/tmdb.svg",
    imdb: "/rating-sources/imdb.svg",
    trakt: "/rating-sources/trakt.svg",
    anilist: "/media-sites/anilist.svg",
    mal: "/media-sites/mal.svg",
    tvdb: "/media-sites/tvdb.svg",
    mdblist: "/rating-sources/mdblist.avif",
    plex: "/auth-providers/plex.svg",
    simkl: "/plugin-logos/svg/simkl.svg",
  };
  for (const [providerType, src] of Object.entries(expected)) {
    assert.equal(providerLogoSrc(providerType), src, providerType);
  }

  for (const providerType of ["custom", "some-future-plugin", "", "  "]) {
    assert.equal(
      providerLogoSrc(providerType),
      null,
      JSON.stringify(providerType),
    );
  }
});

test("an inherited episode policy names what the route's library resolves it to", () => {
  const anime = {
    settings: {
      monitorSpecials: true,
      fillerPolicy: "SKIP_FILLER",
      recapPolicy: "DOWNLOAD_ALL",
    },
  };
  assert.equal(
    inheritedListEpisodePolicyLabelKey("MONITOR_SPECIALS", "ANIME", anime),
    "search.seasonFolder.enabled",
  );
  assert.equal(
    inheritedListEpisodePolicyLabelKey("FILLER_POLICY", "ANIME", anime),
    "settings.fillerPolicySkipFiller",
  );
  assert.equal(
    inheritedListEpisodePolicyLabelKey("RECAP_POLICY", "ANIME", anime),
    "settings.recapPolicyDownloadAll",
  );

  const other = {
    settings: {
      monitorSpecials: false,
      fillerPolicy: "DOWNLOAD_ALL",
      recapPolicy: "SKIP_RECAP",
    },
  };
  assert.equal(
    inheritedListEpisodePolicyLabelKey("MONITOR_SPECIALS", "ANIME", other),
    "search.seasonFolder.disabled",
  );
  assert.equal(
    inheritedListEpisodePolicyLabelKey("FILLER_POLICY", "ANIME", other),
    "settings.fillerPolicyDownloadAll",
  );
  assert.equal(
    inheritedListEpisodePolicyLabelKey("RECAP_POLICY", "ANIME", other),
    "settings.recapPolicySkipRecap",
  );

  // Series libraries carry no specials setting: inheriting never monitors them.
  assert.equal(
    inheritedListEpisodePolicyLabelKey("MONITOR_SPECIALS", "SERIES", null),
    "search.seasonFolder.disabled",
  );

  // Settings the viewer may not read leave the choice unlabelled, not guessed.
  for (const library of [null, undefined, {}, { settings: null }]) {
    assert.equal(
      inheritedListEpisodePolicyLabelKey("MONITOR_SPECIALS", "ANIME", library),
      null,
    );
    assert.equal(
      inheritedListEpisodePolicyLabelKey("FILLER_POLICY", "ANIME", library),
      null,
    );
    assert.equal(
      inheritedListEpisodePolicyLabelKey("RECAP_POLICY", "ANIME", library),
      null,
    );
  }
});

import assert from "node:assert/strict";
import test from "node:test";

import type { InteractiveSearchIndexerProgress } from "@/lib/graphql/release-search";
import type { Release } from "@/lib/types";
import {
  addSavedIndexerSearch,
  buildIndexerSearchFacets,
  downloadableReleases,
  EMPTY_INDEXER_SEARCH_FILTERS,
  filterIndexerSearchReleases,
  formatReleaseAge,
  formatReleaseSize,
  indexerHealthTone,
  indexerSearchRowKey,
  isDownloadableRelease,
  MAX_SAVED_INDEXER_SEARCHES,
  mergeIndexerProgress,
  mergeIndexerSearchReleases,
  parseCategoryList,
  parseSavedIndexerSearches,
  releaseFacetKeys,
  releaseSizeBoundsGiB,
  resolutionBucket,
  sortIndexerSearchReleases,
  summarizeIndexerHealth,
} from "./indexer-search.ts";

const GIB = 1024 * 1024 * 1024;

function release(overrides: Partial<Release> & { title: string }): Release {
  return {
    source: "NZBgeek",
    link: null,
    downloadUrl: `https://example.test/${overrides.title}`,
    sourceKind: "NZB_URL",
    sizeBytes: 10 * GIB,
    publishedAt: "2026-08-01T00:00:00Z",
    ...overrides,
  };
}

function parsed(fields: Partial<NonNullable<Release["parsedRelease"]>>) {
  return {
    rawTitle: "raw",
    normalizedTitle: "raw",
    isDualAudio: false,
    isAtmos: false,
    isDolbyVision: false,
    detectedHdr: false,
    parseConfidence: 1,
    isProperUpload: false,
    revision: 0,
    isRemux: false,
    isBdDisk: false,
    isAiEnhanced: false,
    ...fields,
  };
}

function indexer(
  overrides: Partial<InteractiveSearchIndexerProgress> & { name: string },
): InteractiveSearchIndexerProgress {
  return {
    indexerId: overrides.name.toLowerCase(),
    priority: 1,
    status: "COMPLETED",
    resultCount: 0,
    elapsedMs: 100,
    failureReason: null,
    rateLimited: false,
    ...overrides,
  };
}

test("resolution buckets collapse the parser's raw height token", () => {
  assert.equal(resolutionBucket("2160p"), "2160p");
  assert.equal(resolutionBucket("4320p"), "2160p");
  assert.equal(resolutionBucket("1440p"), "1080p");
  assert.equal(resolutionBucket("720p"), "720p");
  assert.equal(resolutionBucket("480p"), "SD");
  assert.equal(resolutionBucket(null), null);
  assert.equal(resolutionBucket("WEB-DL"), null);
});

test("facet keys cover every rail group", () => {
  const keys = releaseFacetKeys(
    release({
      title: "Movie.2160p.REMUX",
      source: "abNZB",
      sourceKind: "TORRENT_FILE",
      freeleech: true,
      languages: ["eng", " eng ", "ger"],
      parsedRelease: parsed({
        quality: "2160p",
        source: "BluRay",
        videoCodec: "x265",
        audio: "TrueHD",
        audioChannels: " 7.1 ",
        audioLanguages: ["deu", "jpn"],
        isRemux: true,
        isAtmos: true,
        isDolbyVision: true,
        detectedHdr: true,
        isProperUpload: true,
      }),
    }),
  );

  assert.deepEqual(keys.sort(), [
    "audio:atmos",
    "audio:dolbyVision",
    "audio:hdr",
    "audioChannels:7.1",
    "audioCodec:TrueHD",
    "flags:freeleech",
    "flags:proper",
    "indexer:abNZB",
    "language:deu",
    "language:eng",
    "language:jpn",
    "protocol:torrent",
    "resolution:2160p",
    "source:REMUX",
    "videoCodec:x265",
  ]);
});

test("rail groups follow the fixed order and languages read as names", () => {
  const groups = buildIndexerSearchFacets([
    release({
      title: "Sample.Feature.2160p",
      sourceKind: "TORRENT_FILE",
      freeleech: true,
      languages: ["eng", "fre"],
      parsedRelease: parsed({
        quality: "2160p",
        source: "WEB-DL",
        videoCodec: "x265",
        audio: "DDP",
        audioChannels: "5.1",
        isAtmos: true,
      }),
    }),
  ]);

  assert.deepEqual(
    groups.map((group) => group.key),
    [
      "protocol",
      "resolution",
      "source",
      "videoCodec",
      "audioCodec",
      "audioChannels",
      "audio",
      "language",
      "flags",
      "indexer",
    ],
  );
  const language = groups.find((group) => group.key === "language");
  assert.deepEqual(
    language?.items.map((item) => [item.value, item.label]),
    [
      ["eng", "English"],
      ["fra", "French"],
    ],
  );
});

test("with nothing selected, facet counts cover the whole result set", () => {
  const releases = [
    release({
      title: "one",
      parsedRelease: parsed({ quality: "1080p", source: "WEB-DL" }),
    }),
    release({
      title: "two",
      parsedRelease: parsed({ quality: "1080p", source: "BluRay" }),
    }),
    release({
      title: "three",
      sourceKind: "MAGNET_URI",
      source: "TorrentLeech",
      parsedRelease: parsed({ quality: "2160p", source: "WEB-DL" }),
    }),
  ];

  const groups = buildIndexerSearchFacets(releases);
  const protocol = groups.find((group) => group.key === "protocol");
  assert.deepEqual(
    protocol?.items.map((item) => [item.value, item.count]),
    [
      ["usenet", 2],
      ["torrent", 1],
    ],
  );

  const resolution = groups.find((group) => group.key === "resolution");
  assert.deepEqual(
    resolution?.items.map((item) => item.value),
    ["2160p", "1080p"],
  );

  const source = groups.find((group) => group.key === "source");
  assert.equal(source?.items.find((item) => item.value === "WEB-DL")?.count, 2);

  const indexers = groups.find((group) => group.key === "indexer");
  assert.deepEqual(indexers?.items.map((item) => item.value).sort(), [
    "NZBgeek",
    "TorrentLeech",
  ]);
});

function facetCounts(
  groups: ReturnType<typeof buildIndexerSearchFacets>,
  groupKey: string,
): [string, number][] {
  return (
    groups
      .find((group) => group.key === groupKey)
      ?.items.map((item): [string, number] => [item.value, item.count]) ?? []
  );
}

const NOW = Date.parse("2026-09-02T00:00:00Z");

function disjunctiveFixture(): Release[] {
  return [
    release({
      title: "Sample.Alpha.2160p.BluRay.DV",
      parsedRelease: parsed({
        quality: "2160p",
        source: "BluRay",
        isDolbyVision: true,
      }),
    }),
    release({
      title: "Sample.Beta.2160p.BluRay",
      parsedRelease: parsed({ quality: "2160p", source: "BluRay" }),
    }),
    release({
      title: "Sample.Gamma.1080p.WEB-DL",
      parsedRelease: parsed({ quality: "1080p", source: "WEB-DL" }),
    }),
    release({
      title: "Sample.Delta.1080p.WEB-DL",
      sizeBytes: 1 * GIB,
      parsedRelease: parsed({ quality: "1080p", source: "WEB-DL" }),
    }),
    release({
      title: "Sample.Epsilon.720p.HDTV",
      parsedRelease: parsed({ quality: "720p", source: "HDTV" }),
    }),
  ];
}

test("a selection narrows the counts of every other group", () => {
  const groups = buildIndexerSearchFacets(
    disjunctiveFixture(),
    { ...EMPTY_INDEXER_SEARCH_FILTERS, facets: ["audio:dolbyVision"] },
    NOW,
  );

  // Only the BluRay release carries Dolby Vision, so every other source is
  // still listed, in full-set order, with nothing left to show.
  assert.deepEqual(facetCounts(groups, "source"), [
    ["BluRay", 1],
    ["WEB-DL", 0],
    ["HDTV", 0],
  ]);
  assert.deepEqual(facetCounts(groups, "resolution"), [
    ["2160p", 1],
    ["1080p", 0],
    ["720p", 0],
  ]);
});

test("a group's own selection leaves its own counts alone", () => {
  const releases = disjunctiveFixture();
  const unfiltered = facetCounts(buildIndexerSearchFacets(releases), "source");
  const groups = buildIndexerSearchFacets(
    releases,
    { ...EMPTY_INDEXER_SEARCH_FILTERS, facets: ["source:HDTV"] },
    NOW,
  );

  assert.deepEqual(facetCounts(groups, "source"), unfiltered);
  // Across groups the selection still narrows.
  assert.deepEqual(facetCounts(groups, "resolution"), [
    ["2160p", 0],
    ["1080p", 0],
    ["720p", 1],
  ]);
});

test("counts combine selections from several groups", () => {
  const groups = buildIndexerSearchFacets(
    disjunctiveFixture(),
    {
      ...EMPTY_INDEXER_SEARCH_FILTERS,
      facets: ["resolution:1080p", "resolution:720p", "source:WEB-DL"],
    },
    NOW,
  );

  // Source counts see only the resolution selection (1080p OR 720p).
  assert.deepEqual(facetCounts(groups, "source"), [
    ["BluRay", 0],
    ["WEB-DL", 2],
    ["HDTV", 1],
  ]);
  // Resolution counts see only the source selection.
  assert.deepEqual(facetCounts(groups, "resolution"), [
    ["2160p", 0],
    ["1080p", 2],
    ["720p", 0],
  ]);
  // A third group sees both: the two 1080p WEB-DL releases.
  assert.deepEqual(facetCounts(groups, "protocol"), [["usenet", 2]]);
});

test("size, seeder and age limits narrow every count", () => {
  const groups = buildIndexerSearchFacets(
    disjunctiveFixture(),
    { ...EMPTY_INDEXER_SEARCH_FILTERS, minSizeGiB: 4 },
    NOW,
  );

  assert.deepEqual(facetCounts(groups, "source"), [
    ["BluRay", 2],
    ["WEB-DL", 1],
    ["HDTV", 1],
  ]);

  const aged = buildIndexerSearchFacets(
    [
      ...disjunctiveFixture(),
      release({
        title: "Sample.Zeta.1080p.WEB-DL",
        publishedAt: "2020-01-01T00:00:00Z",
        parsedRelease: parsed({ quality: "1080p", source: "WEB-DL" }),
      }),
    ],
    { ...EMPTY_INDEXER_SEARCH_FILTERS, maxAgeDays: 365 },
    NOW,
  );
  assert.deepEqual(facetCounts(aged, "source"), [
    ["WEB-DL", 2],
    ["BluRay", 2],
    ["HDTV", 1],
  ]);
});

test("a selected item nothing matches stays listed", () => {
  const groups = buildIndexerSearchFacets(
    disjunctiveFixture(),
    { ...EMPTY_INDEXER_SEARCH_FILTERS, facets: ["indexer:QuietIndexer"] },
    NOW,
  );
  assert.deepEqual(facetCounts(groups, "indexer"), [
    ["NZBgeek", 5],
    ["QuietIndexer", 0],
  ]);
  assert.deepEqual(facetCounts(groups, "source"), [
    ["BluRay", 0],
    ["WEB-DL", 0],
    ["HDTV", 0],
  ]);
});

test("a release with several languages counts under each one", () => {
  const releases = [
    release({ title: "Sample.Multi.1080p", languages: ["eng", "ger"] }),
    release({ title: "Sample.English.1080p", languages: ["eng"] }),
    release({ title: "Sample.Silent.1080p" }),
  ];
  assert.deepEqual(
    facetCounts(buildIndexerSearchFacets(releases), "language"),
    [
      ["eng", 2],
      ["deu", 1],
    ],
  );

  const filtered = filterIndexerSearchReleases(
    releases,
    { ...EMPTY_INDEXER_SEARCH_FILTERS, facets: ["language:deu"] },
    NOW,
  );
  assert.deepEqual(
    filtered.map((entry) => entry.title),
    ["Sample.Multi.1080p"],
  );
});

test("audio channels list widest first as the parser reports them", () => {
  const releases = [
    release({
      title: "Sample.Stereo",
      parsedRelease: parsed({ audioChannels: "2.0" }),
    }),
    release({
      title: "Sample.Wide",
      parsedRelease: parsed({ audioChannels: "7.1" }),
    }),
    release({
      title: "Sample.Surround.A",
      parsedRelease: parsed({ audioChannels: "5.1" }),
    }),
    release({
      title: "Sample.Surround.B",
      parsedRelease: parsed({ audioChannels: "5.1" }),
    }),
    release({
      title: "Sample.Mono",
      parsedRelease: parsed({ audioChannels: "1.0" }),
    }),
    release({ title: "Sample.Undetected", parsedRelease: parsed({}) }),
  ];
  assert.deepEqual(
    facetCounts(buildIndexerSearchFacets(releases), "audioChannels"),
    [
      ["7.1", 1],
      ["5.1", 2],
      ["2.0", 1],
      ["1.0", 1],
    ],
  );
});

test("audio channel selections narrow other groups and are narrowed by them", () => {
  const releases = [
    release({
      title: "Sample.Alpha.2160p.7.1",
      parsedRelease: parsed({ quality: "2160p", audioChannels: "7.1" }),
    }),
    release({
      title: "Sample.Beta.2160p.5.1",
      parsedRelease: parsed({ quality: "2160p", audioChannels: "5.1" }),
    }),
    release({
      title: "Sample.Gamma.1080p.5.1",
      parsedRelease: parsed({ quality: "1080p", audioChannels: "5.1" }),
    }),
    release({
      title: "Sample.Delta.1080p.2.0",
      parsedRelease: parsed({ quality: "1080p", audioChannels: "2.0" }),
    }),
  ];

  const byChannels = buildIndexerSearchFacets(
    releases,
    { ...EMPTY_INDEXER_SEARCH_FILTERS, facets: ["audioChannels:5.1"] },
    NOW,
  );
  assert.deepEqual(facetCounts(byChannels, "resolution"), [
    ["2160p", 1],
    ["1080p", 1],
  ]);
  // Its own selection leaves the channel counts at the full set.
  assert.deepEqual(facetCounts(byChannels, "audioChannels"), [
    ["7.1", 1],
    ["5.1", 2],
    ["2.0", 1],
  ]);

  const byResolution = buildIndexerSearchFacets(
    releases,
    { ...EMPTY_INDEXER_SEARCH_FILTERS, facets: ["resolution:1080p"] },
    NOW,
  );
  assert.deepEqual(facetCounts(byResolution, "audioChannels"), [
    ["7.1", 0],
    ["5.1", 1],
    ["2.0", 1],
  ]);

  const filtered = filterIndexerSearchReleases(
    releases,
    {
      ...EMPTY_INDEXER_SEARCH_FILTERS,
      facets: ["audioChannels:5.1", "audioChannels:2.0", "resolution:1080p"],
    },
    NOW,
  );
  assert.deepEqual(
    filtered.map((entry) => entry.title),
    ["Sample.Gamma.1080p.5.1", "Sample.Delta.1080p.2.0"],
  );
});

test("languages join indexer reports and the parsed title, once per release", () => {
  const releases = [
    // The indexer and the title name the same languages in different spellings.
    release({
      title: "Sample.Dual.1080p",
      languages: ["English", "ger"],
      parsedRelease: parsed({ audioLanguages: ["eng", "deu"] }),
    }),
    // Only the title names a language.
    release({
      title: "Sample.Titled.1080p",
      parsedRelease: parsed({ audioLanguages: ["jpn"] }),
    }),
    // Only the indexer reports one.
    release({ title: "Sample.Reported.1080p", languages: ["eng"] }),
  ];

  const groups = buildIndexerSearchFacets(releases);
  assert.deepEqual(facetCounts(groups, "language"), [
    ["eng", 2],
    ["deu", 1],
    ["jpn", 1],
  ]);
  assert.deepEqual(
    groups
      .find((group) => group.key === "language")
      ?.items.map((item) => item.label),
    ["English", "German", "Japanese"],
  );
  assert.deepEqual(
    releaseFacetKeys(releases[0]).filter((key) => key.startsWith("language:")),
    ["language:eng", "language:deu"],
  );

  const filtered = filterIndexerSearchReleases(
    releases,
    { ...EMPTY_INDEXER_SEARCH_FILTERS, facets: ["language:jpn"] },
    NOW,
  );
  assert.deepEqual(
    filtered.map((entry) => entry.title),
    ["Sample.Titled.1080p"],
  );
});

test("facets OR within a group and AND across groups", () => {
  const now = Date.parse("2026-09-02T00:00:00Z");
  const releases = [
    release({
      title: "usenet-1080p",
      parsedRelease: parsed({ quality: "1080p" }),
    }),
    release({
      title: "usenet-2160p",
      parsedRelease: parsed({ quality: "2160p" }),
    }),
    release({
      title: "torrent-1080p",
      sourceKind: "TORRENT_FILE",
      parsedRelease: parsed({ quality: "1080p" }),
    }),
  ];

  const bothResolutions = filterIndexerSearchReleases(
    releases,
    {
      ...EMPTY_INDEXER_SEARCH_FILTERS,
      facets: ["resolution:1080p", "resolution:2160p"],
    },
    now,
  );
  assert.equal(bothResolutions.length, 3);

  const usenet1080 = filterIndexerSearchReleases(
    releases,
    {
      ...EMPTY_INDEXER_SEARCH_FILTERS,
      facets: ["resolution:1080p", "protocol:usenet"],
    },
    now,
  );
  assert.deepEqual(
    usenet1080.map((entry) => entry.title),
    ["usenet-1080p"],
  );
});

test("advanced limits filter on size, seeders and age", () => {
  const now = Date.parse("2026-09-02T00:00:00Z");
  const releases = [
    release({ title: "small", sizeBytes: 2 * GIB }),
    release({ title: "large", sizeBytes: 90 * GIB }),
    release({
      title: "thin-swarm",
      sourceKind: "TORRENT_FILE",
      sizeBytes: 10 * GIB,
      seeders: 1,
    }),
    release({
      title: "ancient",
      sizeBytes: 10 * GIB,
      publishedAt: "2020-01-01T00:00:00Z",
    }),
  ];

  const filtered = filterIndexerSearchReleases(
    releases,
    {
      ...EMPTY_INDEXER_SEARCH_FILTERS,
      minSizeGiB: 4,
      maxSizeGiB: 80,
      minSeeders: 3,
      maxAgeDays: 365,
    },
    now,
  );

  assert.deepEqual(
    filtered.map((entry) => entry.title),
    [],
  );

  // A usenet result reports no swarm, so the seeder floor never excludes it.
  const usenetOnly = filterIndexerSearchReleases(
    [release({ title: "usenet", sizeBytes: 10 * GIB })],
    { ...EMPTY_INDEXER_SEARCH_FILTERS, minSeeders: 50 },
    now,
  );
  assert.equal(usenetOnly.length, 1);
});

test("a retry merges rows onto their own identity and appends new ones", () => {
  const first = release({ title: "known", sizeBytes: 1 * GIB });
  const retried = release({ title: "known", sizeBytes: 1 * GIB, seeders: 12 });
  const fresh = release({ title: "new-from-retry" });

  const merged = mergeIndexerSearchReleases([first], [retried, fresh]);

  assert.equal(merged.length, 2);
  assert.equal(indexerSearchRowKey(merged[0]!), indexerSearchRowKey(first));
  assert.equal(merged[0]?.seeders, 12);
  assert.equal(merged[1]?.title, "new-from-retry");
});

test("a retry replaces the health entry of the indexer it re-ran", () => {
  const base = [
    indexer({ name: "healthy" }),
    indexer({ name: "broken", status: "FAILED", failureReason: "timeout" }),
  ];
  const merged = mergeIndexerProgress(base, [
    indexer({ name: "broken", resultCount: 4 }),
  ]);

  assert.equal(merged.length, 2);
  assert.equal(merged[1]?.status, "COMPLETED");
  assert.equal(merged[1]?.resultCount, 4);
});

test("health tones read slowness from elapsed time, not from a status", () => {
  assert.equal(indexerHealthTone(indexer({ name: "a", elapsedMs: 400 })), "ok");
  assert.equal(
    indexerHealthTone(indexer({ name: "b", elapsedMs: 2_500 })),
    "slow",
  );
  assert.equal(
    indexerHealthTone(indexer({ name: "c", status: "FAILED" })),
    "failed",
  );
  assert.equal(
    indexerHealthTone(indexer({ name: "d", status: "SKIPPED" })),
    "skipped",
  );
  assert.equal(
    indexerHealthTone(indexer({ name: "e", status: "SEARCHING" })),
    "pending",
  );
});

test("a rate-limited indexer reads as cooling down, not as a failure", () => {
  const cooling = indexer({
    name: "indexer-a.example",
    status: "FAILED",
    rateLimited: true,
    failureReason:
      "indexer is cooling down after a rate limit; retry after 120s",
  });
  assert.equal(indexerHealthTone(cooling), "cooling");
  // With results already in hand the row stays partial: the user has
  // something to look at, and partial already reads as "not everything".
  assert.equal(indexerHealthTone({ ...cooling, resultCount: 12 }), "partial");
  // Without the flag the same status is still a plain failure.
  assert.equal(indexerHealthTone({ ...cooling, rateLimited: false }), "failed");
});

test("incomplete searches with results are partial and remain retryable", () => {
  const entry = indexer({ name: "partial", status: "FAILED", resultCount: 50 });
  assert.equal(indexerHealthTone(entry), "partial");
  assert.equal(indexerHealthTone({ ...entry, resultCount: 0 }), "failed");
  assert.deepEqual(summarizeIndexerHealth([entry]).failedIndexerIds, [
    entry.indexerId,
  ]);
});

test("the health summary counts what is still outstanding", () => {
  const summary = summarizeIndexerHealth([
    indexer({ name: "a", elapsedMs: 400 }),
    indexer({ name: "b", status: "SEARCHING", elapsedMs: null }),
    indexer({ name: "c", status: "FAILED", elapsedMs: 5_000 }),
  ]);

  assert.equal(summary.total, 3);
  assert.equal(summary.answered, 2);
  assert.equal(summary.pending, 1);
  assert.deepEqual(summary.failedIndexerIds, ["c"]);
  assert.equal(summary.elapsedMs, 5_000);
});

test("size and age render coarsely", () => {
  assert.equal(formatReleaseSize(76.4 * GIB), "76.4 GiB");
  assert.equal(formatReleaseSize(700 * 1024 * 1024), "700 MiB");
  assert.equal(formatReleaseSize(null), "—");

  assert.deepEqual(formatReleaseAge(5 * 3_600_000), {
    unitKey: "indexerSearch.age.hours",
    value: 5,
  });
  assert.deepEqual(formatReleaseAge(10 * 86_400_000), {
    unitKey: "indexerSearch.age.days",
    value: 10,
  });
  assert.deepEqual(formatReleaseAge(180 * 86_400_000), {
    unitKey: "indexerSearch.age.months",
    value: 6,
  });
  assert.deepEqual(formatReleaseAge(1_000 * 86_400_000), {
    unitKey: "indexerSearch.age.years",
    value: 2,
  });
  assert.equal(formatReleaseAge(null), null);
});

test("size bounds round outwards and ignore sizeless results", () => {
  assert.deepEqual(
    releaseSizeBoundsGiB([
      release({ title: "a", sizeBytes: 1.25 * GIB }),
      release({ title: "b", sizeBytes: 9.11 * GIB }),
      release({ title: "c", sizeBytes: null }),
    ]),
    [1.2, 9.2],
  );
  assert.equal(
    releaseSizeBoundsGiB([release({ title: "a", sizeBytes: null })]),
    null,
  );
});

test("category lists accept commas, spaces and duplicates", () => {
  assert.deepEqual(parseCategoryList("2000, 2045 2045, tv"), ["2000", "2045"]);
  assert.deepEqual(parseCategoryList("   "), []);
});

test("saved searches are newest-first, deduplicated and capped", () => {
  let saved = addSavedIndexerSearch([], {
    query: "dune",
    indexerIds: [],
    categories: [],
  });
  saved = addSavedIndexerSearch(saved, {
    query: "expanse",
    indexerIds: ["a"],
    categories: ["5000"],
  });
  saved = addSavedIndexerSearch(saved, {
    query: "dune",
    indexerIds: ["b"],
    categories: [],
  });

  assert.deepEqual(
    saved.map((entry) => entry.query),
    ["dune", "expanse"],
  );
  assert.deepEqual(saved[0]?.indexerIds, ["b"]);

  let capped: ReturnType<typeof addSavedIndexerSearch> = [];
  for (let index = 0; index < MAX_SAVED_INDEXER_SEARCHES + 5; index += 1) {
    capped = addSavedIndexerSearch(capped, {
      query: `query-${index}`,
      indexerIds: [],
      categories: [],
    });
  }
  assert.equal(capped.length, MAX_SAVED_INDEXER_SEARCHES);
  assert.equal(capped[0]?.query, `query-${MAX_SAVED_INDEXER_SEARCHES + 4}`);

  assert.deepEqual(
    addSavedIndexerSearch(saved, {
      query: "   ",
      indexerIds: [],
      categories: [],
    }),
    saved,
  );
});

test("stored saved searches survive junk in localStorage", () => {
  assert.deepEqual(parseSavedIndexerSearches(null), []);
  assert.deepEqual(parseSavedIndexerSearches("not json"), []);
  assert.deepEqual(parseSavedIndexerSearches('{"query":"x"}'), []);
  assert.deepEqual(
    parseSavedIndexerSearches(
      JSON.stringify([
        {
          query: "dune",
          kind: "MOVIE",
          indexerIds: ["a", 2],
          categories: null,
        },
        { query: "", kind: "MOVIE" },
      ]),
    ),
    [{ query: "dune", indexerIds: ["a"], categories: [] }],
  );
});

test("only http(s) releases can be downloaded to the browser", () => {
  const usenet = release({ title: "Paperman.2012.1080p" });
  const magnet = release({
    title: "Paperman.2012.2160p",
    downloadUrl: null,
    link: "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567",
    sourceKind: "MAGNET_URI",
  });
  const torrentFile = release({
    title: "Paperman.2012.720p",
    downloadUrl: null,
    link: "https://tracker.test/a.torrent",
    sourceKind: "TORRENT_FILE",
  });
  const nothing = release({
    title: "Paperman.2012.480p",
    downloadUrl: null,
    link: null,
  });

  assert.equal(isDownloadableRelease(usenet), true);
  // The link is the fallback when the payload carries no download URL.
  assert.equal(isDownloadableRelease(torrentFile), true);
  assert.equal(isDownloadableRelease(magnet), false);
  assert.equal(isDownloadableRelease(nothing), false);

  assert.deepEqual(
    downloadableReleases([usenet, magnet, torrentFile, nothing]).map(
      (entry) => entry.title,
    ),
    [usenet.title, torrentFile.title],
  );
});

test("every data column sorts in both directions with missing values last", () => {
  const a = release({
    title: "alpha",
    source: "alpha",
    sizeBytes: 2,
    publishedAt: "2026-01-02",
    seeders: 2,
  });
  const b = release({
    title: "Beta",
    source: "Beta",
    sizeBytes: 10,
    publishedAt: "2026-01-01",
    grabs: 10,
  });
  const missing = release({
    title: "",
    source: null,
    sizeBytes: null,
    publishedAt: "invalid",
  });
  for (const column of [
    "release",
    "indexer",
    "size",
    "age",
    "peers",
  ] as const) {
    assert.deepEqual(
      sortIndexerSearchReleases([missing, b, a], `${column}-asc`),
      [a, b, missing],
      column,
    );
    assert.deepEqual(
      sortIndexerSearchReleases([a, missing, b], `${column}-desc`),
      [b, a, missing],
      column,
    );
  }
});

test("ties use row identity independently of arrival order", () => {
  const a = release({ title: "same", downloadUrl: "https://example.test/a" });
  const b = release({ title: "same", downloadUrl: "https://example.test/b" });
  for (const key of [
    "release-asc",
    "release-desc",
    "size-asc",
    "size-desc",
  ] as const) {
    assert.deepEqual(
      sortIndexerSearchReleases([b, a], key),
      sortIndexerSearchReleases([a, b], key),
    );
  }
});

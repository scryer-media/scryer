import assert from "node:assert/strict";
import test from "node:test";

import type { InteractiveSearchIndexerProgress } from "@/lib/graphql/release-search";
import type { Release } from "@/lib/types";
import {
  addSavedIndexerSearch,
  buildIndexerSearchFacets,
  EMPTY_INDEXER_SEARCH_FILTERS,
  filterIndexerSearchReleases,
  formatReleaseAge,
  formatReleaseSize,
  indexerHealthTone,
  indexerSearchRowKey,
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

test("facet keys cover protocol, resolution, source, audio, flags and indexer", () => {
  const keys = releaseFacetKeys(
    release({
      title: "Movie.2160p.REMUX",
      source: "abNZB",
      sourceKind: "TORRENT_FILE",
      freeleech: true,
      parsedRelease: parsed({
        quality: "2160p",
        source: "BluRay",
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
    "flags:freeleech",
    "flags:proper",
    "indexer:abNZB",
    "protocol:torrent",
    "resolution:2160p",
    "source:REMUX",
  ]);
});

test("facet counts are computed over the whole result set", () => {
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
  assert.equal(
    source?.items.find((item) => item.value === "WEB-DL")?.count,
    2,
  );

  const indexers = groups.find((group) => group.key === "indexer");
  assert.deepEqual(
    indexers?.items.map((item) => item.value).sort(),
    ["NZBgeek", "TorrentLeech"],
  );
});

test("facets OR within a group and AND across groups", () => {
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
    Date.now(),
  );
  assert.equal(bothResolutions.length, 3);

  const usenet1080 = filterIndexerSearchReleases(
    releases,
    {
      ...EMPTY_INDEXER_SEARCH_FILTERS,
      facets: ["resolution:1080p", "protocol:usenet"],
    },
    Date.now(),
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

test("sorting covers every offered order", () => {
  const releases = [
    release({
      title: "old-big",
      sizeBytes: 50 * GIB,
      publishedAt: "2026-01-01T00:00:00Z",
      source: "slow-indexer",
      seeders: 2,
    }),
    release({
      title: "new-small",
      sizeBytes: 5 * GIB,
      publishedAt: "2026-08-01T00:00:00Z",
      source: "fast-indexer",
      seeders: 40,
    }),
  ];
  const priorities = new Map([
    ["fast-indexer", 5],
    ["slow-indexer", 1],
  ]);

  assert.equal(
    sortIndexerSearchReleases(releases, "newest", priorities)[0]?.title,
    "new-small",
  );
  assert.equal(
    sortIndexerSearchReleases(releases, "age", priorities)[0]?.title,
    "old-big",
  );
  assert.equal(
    sortIndexerSearchReleases(releases, "size", priorities)[0]?.title,
    "old-big",
  );
  assert.equal(
    sortIndexerSearchReleases(releases, "seeders", priorities)[0]?.title,
    "new-small",
  );
  assert.equal(
    sortIndexerSearchReleases(releases, "priority", priorities)[0]?.title,
    "old-big",
  );
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
  assert.equal(releaseSizeBoundsGiB([release({ title: "a", sizeBytes: null })]), null);
});

test("category lists accept commas, spaces and duplicates", () => {
  assert.deepEqual(parseCategoryList("2000, 2045 2045, tv"), ["2000", "2045"]);
  assert.deepEqual(parseCategoryList("   "), []);
});

test("saved searches are newest-first, deduplicated and capped", () => {
  let saved = addSavedIndexerSearch([], {
    query: "dune",
    kind: "MOVIE",
    indexerIds: [],
    categories: [],
  });
  saved = addSavedIndexerSearch(saved, {
    query: "expanse",
    kind: "SERIES",
    indexerIds: ["a"],
    categories: ["5000"],
  });
  saved = addSavedIndexerSearch(saved, {
    query: "dune",
    kind: "MOVIE",
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
      kind: "RAW",
      indexerIds: [],
      categories: [],
    });
  }
  assert.equal(capped.length, MAX_SAVED_INDEXER_SEARCHES);
  assert.equal(capped[0]?.query, `query-${MAX_SAVED_INDEXER_SEARCHES + 4}`);

  assert.deepEqual(
    addSavedIndexerSearch(saved, {
      query: "   ",
      kind: "RAW",
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
        { query: "dune", kind: "MOVIE", indexerIds: ["a", 2], categories: null },
        { query: "", kind: "MOVIE" },
      ]),
    ),
    [{ query: "dune", kind: "MOVIE", indexerIds: ["a"], categories: [] }],
  );
});

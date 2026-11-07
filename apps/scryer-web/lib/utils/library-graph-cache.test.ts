import assert from "node:assert/strict";
import test from "node:test";

import type { LibraryGraphResult } from "../types/library-graph.ts";
import type { TitleRecord } from "../types/titles.ts";
import {
  decodeLibraryGraph,
  decodeTitleListing,
  encodeLibraryGraph,
  encodeTitleListing,
  libraryGraphCacheKey,
  readCachedGraphTitles,
  readCachedLibraryGraph,
  writeCachedGraphTitles,
  writeCachedLibraryGraph,
} from "./library-graph-cache.ts";
import {
  browserCacheZstdCodec,
  type BrowserCacheStore,
} from "./browser-cache.ts";

const graph: LibraryGraphResult = {
  generatedAt: "2026-01-01T00:00:00Z",
  nodes: [
    { subjectKey: "s:a", targetKey: "s:a", clusterId: 0 },
    { subjectKey: "s:b", targetKey: "t:b", clusterId: -1 },
  ],
  edges: [
    {
      from: "s:a",
      to: "s:b",
      weight: 0.5,
      kinds: ["semantic", "relation:sequel"],
    },
  ],
  clusters: [
    { id: 0, labelKey: null, label: "Synthetic", medoid: "s:a", size: 1 },
  ],
  subjectCount: 2,
  truncated: false,
};

function memoryStorage(limit = Infinity): BrowserCacheStore & {
  items: Map<string, Uint8Array>;
} {
  const items = new Map<string, Uint8Array>();
  return {
    items,
    get: async (key) => items.get(key) ?? null,
    async set(key, value) {
      const used = [...items].reduce(
        (sum, [k, v]) => sum + (k === key ? 0 : v.length),
        0,
      );
      if (used + value.length > limit) throw new Error("quota");
      items.set(key, value);
    },
    delete: async (key) => void items.delete(key),
    keys: async () => [...items.keys()],
  };
}

const request = { facet: "MOVIE" as const, libraryIds: ["l2", "l1"] };

test("a graph survives the compact encoding unchanged", () => {
  assert.deepEqual(decodeLibraryGraph(encodeLibraryGraph(graph)), graph);
});

test("a listing survives the field-wise encoding, missing fields included", () => {
  const titles = [
    { id: "t1", name: "Synthetic One", year: 2001, ratings: { rating: 7 } },
    { id: "t2", name: "Synthetic Two", year: null },
    { id: "t3", monitored: true },
  ] as unknown as TitleRecord[];
  assert.deepEqual(decodeTitleListing(encodeTitleListing(titles)), titles);
  assert.deepEqual(decodeTitleListing(encodeTitleListing([])), []);
});

test("a stored graph reads back through zstd, and each viewer has their own", async () => {
  const storage = memoryStorage();
  const key = libraryGraphCacheKey("viewer-1", request);
  await writeCachedLibraryGraph(storage, key, graph, browserCacheZstdCodec);
  assert.deepEqual(
    await readCachedLibraryGraph(storage, key, browserCacheZstdCodec),
    graph,
  );
  assert.equal(
    await readCachedLibraryGraph(
      storage,
      libraryGraphCacheKey("viewer-2", request),
      browserCacheZstdCodec,
    ),
    null,
  );
  assert.equal(
    key,
    libraryGraphCacheKey("viewer-1", { ...request, libraryIds: ["l1", "l2"] }),
  );
});

test("a facet keeps one entry, a damaged one is dropped, and with no room other viewers give way", async () => {
  const storage = memoryStorage();
  const all = libraryGraphCacheKey("v", { facet: "MOVIE", libraryIds: [] });
  const one = libraryGraphCacheKey("v", { facet: "MOVIE", libraryIds: ["l1"] });
  const series = libraryGraphCacheKey("v", { facet: "SERIES", libraryIds: [] });
  await writeCachedLibraryGraph(storage, all, graph, browserCacheZstdCodec);
  await writeCachedLibraryGraph(storage, series, graph, browserCacheZstdCodec);
  await writeCachedLibraryGraph(storage, one, graph, browserCacheZstdCodec);
  assert.deepEqual([...storage.items.keys()].sort(), [one, series].sort());
  storage.items.set(one, new Uint8Array([1, 2, 3]));
  assert.equal(
    await readCachedLibraryGraph(storage, one, browserCacheZstdCodec),
    null,
  );
  assert.equal(storage.items.has(one), false);
  await writeCachedLibraryGraph(storage, series, null, browserCacheZstdCodec);
  assert.equal(storage.items.has(series), false);

  const full = memoryStorage(10);
  const otherViewer = libraryGraphCacheKey("w", request);
  full.items.set(otherViewer, new Uint8Array([1]));
  await writeCachedLibraryGraph(full, all, graph, browserCacheZstdCodec);
  assert.deepEqual([...full.items.keys()], []);
});

test("the listing is kept beside the graph and replaces only its own kind", async () => {
  const storage = memoryStorage();
  const titles = [
    { id: "t1", name: "Synthetic One" },
    { id: "t2", name: "Synthetic Two" },
  ] as unknown as TitleRecord[];
  const graphKey = libraryGraphCacheKey("v", request);
  const titlesKey = libraryGraphCacheKey("v", request, "titles");
  const otherTitles = libraryGraphCacheKey(
    "v",
    { facet: "MOVIE", libraryIds: [] },
    "titles",
  );
  await writeCachedLibraryGraph(
    storage,
    graphKey,
    graph,
    browserCacheZstdCodec,
  );
  await writeCachedGraphTitles(
    storage,
    otherTitles,
    titles,
    browserCacheZstdCodec,
  );
  await writeCachedGraphTitles(
    storage,
    titlesKey,
    titles,
    browserCacheZstdCodec,
  );
  assert.deepEqual(
    [...storage.items.keys()].sort(),
    [graphKey, titlesKey].sort(),
  );
  assert.deepEqual(
    await readCachedGraphTitles(storage, titlesKey, browserCacheZstdCodec),
    titles,
  );
  assert.deepEqual(
    await readCachedLibraryGraph(storage, graphKey, browserCacheZstdCodec),
    graph,
  );
});

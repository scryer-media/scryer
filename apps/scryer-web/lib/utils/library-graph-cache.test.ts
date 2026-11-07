import assert from "node:assert/strict";
import test from "node:test";

import type { LibraryGraphResult } from "../types/library-graph.ts";
import {
  decodeLibraryGraph,
  encodeLibraryGraph,
  libraryGraphCacheKey,
  libraryGraphZstdCodec,
  readCachedLibraryGraph,
  writeCachedLibraryGraph,
  type LibraryGraphStorage,
} from "./library-graph-cache.ts";

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

function memoryStorage(limit = Infinity): LibraryGraphStorage & {
  items: Map<string, string>;
} {
  const items = new Map<string, string>();
  return {
    items,
    get length() {
      return items.size;
    },
    key: (index) => [...items.keys()][index] ?? null,
    getItem: (key) => items.get(key) ?? null,
    setItem(key, value) {
      const used = [...items].reduce(
        (sum, [k, v]) => sum + (k === key ? 0 : v.length),
        0,
      );
      if (used + value.length > limit) throw new Error("quota");
      items.set(key, value);
    },
    removeItem: (key) => void items.delete(key),
  };
}

const request = { facet: "MOVIE" as const, libraryIds: ["l2", "l1"] };

test("a graph survives the compact encoding unchanged", () => {
  assert.deepEqual(decodeLibraryGraph(encodeLibraryGraph(graph)), graph);
});

test("a stored graph reads back through zstd, and each viewer has their own", async () => {
  const storage = memoryStorage();
  const key = libraryGraphCacheKey("viewer-1", request);
  await writeCachedLibraryGraph(storage, key, graph, libraryGraphZstdCodec);
  assert.deepEqual(
    await readCachedLibraryGraph(storage, key, libraryGraphZstdCodec),
    graph,
  );
  assert.equal(
    await readCachedLibraryGraph(
      storage,
      libraryGraphCacheKey("viewer-2", request),
      libraryGraphZstdCodec,
    ),
    null,
  );
  assert.equal(
    key,
    libraryGraphCacheKey("viewer-1", { ...request, libraryIds: ["l1", "l2"] }),
  );
});

test("a facet keeps one entry, a damaged one is dropped, and no room keeps nothing", async () => {
  const storage = memoryStorage();
  const all = libraryGraphCacheKey("v", { facet: "MOVIE", libraryIds: [] });
  const one = libraryGraphCacheKey("v", { facet: "MOVIE", libraryIds: ["l1"] });
  const series = libraryGraphCacheKey("v", { facet: "SERIES", libraryIds: [] });
  await writeCachedLibraryGraph(storage, all, graph, libraryGraphZstdCodec);
  await writeCachedLibraryGraph(storage, series, graph, libraryGraphZstdCodec);
  await writeCachedLibraryGraph(storage, one, graph, libraryGraphZstdCodec);
  assert.deepEqual([...storage.items.keys()].sort(), [one, series].sort());
  storage.items.set(one, "not a graph");
  assert.equal(
    await readCachedLibraryGraph(storage, one, libraryGraphZstdCodec),
    null,
  );
  assert.equal(storage.items.has(one), false);
  await writeCachedLibraryGraph(storage, series, null, libraryGraphZstdCodec);
  assert.equal(storage.items.has(series), false);

  const full = memoryStorage(10);
  full.items.set("unrelated", "kept");
  await writeCachedLibraryGraph(full, all, graph, libraryGraphZstdCodec);
  assert.deepEqual([...full.items.keys()], ["unrelated"]);
});

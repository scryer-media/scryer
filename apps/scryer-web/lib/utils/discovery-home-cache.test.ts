import assert from "node:assert/strict";
import test from "node:test";

import type { DiscoveryHomePayload } from "../types/discovery.ts";
import {
  browserCacheZstdCodec,
  type BrowserCacheStore,
} from "./browser-cache.ts";
import {
  discoveryHomeCacheKey,
  readCachedDiscoveryHome,
  writeCachedDiscoveryHome,
} from "./discovery-home-cache.ts";

function memoryStore(): BrowserCacheStore & { items: Map<string, Uint8Array> } {
  const items = new Map<string, Uint8Array>();
  return {
    items,
    get: async (key) => items.get(key) ?? null,
    set: async (key, value) => void items.set(key, value),
    delete: async (key) => void items.delete(key),
    keys: async () => [...items.keys()],
  };
}

const home = {
  status: "READY",
  heroItem: null,
  publicSections: [],
  personalizedSections: [],
  completeCollection: null,
  browseLabels: [],
  canViewPersonalized: true,
} as unknown as DiscoveryHomePayload;

test("only an unfiltered home of a known viewer is kept", () => {
  assert.equal(discoveryHomeCacheKey(null, "en", "sig", {}), null);
  assert.equal(
    discoveryHomeCacheKey("v", "en", "sig", { minimumYear: 1990 }),
    null,
  );
  assert.notEqual(
    discoveryHomeCacheKey("v", "en", "sig", {
      genreTagKeys: [],
      minimumRating: null,
    }),
    null,
  );
  assert.notEqual(
    discoveryHomeCacheKey("v", "en", "sig", {}),
    discoveryHomeCacheKey("v", "de", "sig", {}),
  );
});

test("a stored home reads back, and another language replaces it", async () => {
  const store = memoryStore();
  const english = discoveryHomeCacheKey("v", "en", "sig", {})!;
  const german = discoveryHomeCacheKey("v", "de", "sig", {})!;
  await writeCachedDiscoveryHome(store, english, home, browserCacheZstdCodec);
  assert.deepEqual(
    await readCachedDiscoveryHome(store, english, browserCacheZstdCodec),
    home,
  );
  await writeCachedDiscoveryHome(store, german, home, browserCacheZstdCodec);
  assert.deepEqual([...store.items.keys()], [german]);
});

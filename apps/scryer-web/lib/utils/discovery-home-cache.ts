// The unfiltered discovery home, kept in the browser cache so the page draws
// at once on a return visit while the server is asked again.

import type {
  DiscoveryHomeFilters,
  DiscoveryHomePayload,
} from "../types/discovery.ts";
import {
  browserCacheKey,
  readBrowserCache,
  writeBrowserCache,
  type BrowserCacheCodec,
  type BrowserCacheStore,
} from "./browser-cache.ts";

function unfiltered(filters: DiscoveryHomeFilters) {
  return Object.values(filters).every(
    (value) =>
      value === null ||
      value === undefined ||
      (Array.isArray(value) && value.length === 0),
  );
}

/**
 * The key the viewer's home is kept under, or null when it is not kept: with
 * no viewer, or while filters narrow it. The language and the viewer's
 * permissions pick the variant, so a change to either never shows the old one.
 */
export function discoveryHomeCacheKey(
  viewer: string | null | undefined,
  uiLanguage: string,
  authorizationSignature: string,
  filters: DiscoveryHomeFilters,
): string | null {
  if (!viewer || !unfiltered(filters)) return null;
  return browserCacheKey(
    viewer,
    "DISCOVERY",
    "home",
    JSON.stringify([uiLanguage, authorizationSignature]),
  );
}

export function readCachedDiscoveryHome(
  store: BrowserCacheStore,
  key: string,
  codec: () => Promise<BrowserCacheCodec>,
): Promise<DiscoveryHomePayload | null> {
  return readBrowserCache(store, key, codec, (value) => {
    if (!value || typeof value !== "object" || !("publicSections" in value))
      throw new Error("not a discovery home");
    return value as DiscoveryHomePayload;
  });
}

/** Stores the home; a null home clears the entry. */
export function writeCachedDiscoveryHome(
  store: BrowserCacheStore,
  key: string,
  home: DiscoveryHomePayload | null,
  codec: () => Promise<BrowserCacheCodec>,
): Promise<void> {
  return writeBrowserCache(store, key, home, codec);
}

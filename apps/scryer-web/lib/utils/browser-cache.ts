// Copies of what the server last answered, kept in the browser's IndexedDB,
// so a view draws at once from them while the server is asked again.
// Each value is stored as zstd-compressed JSON under a key naming the viewer,
// the area, what the entry holds and the variant it was read for.

export type BrowserCacheCodec = {
  compress(bytes: Uint8Array): Uint8Array;
  decompress(bytes: Uint8Array): Uint8Array;
};

/** The store the cache needs; IndexedDB in the app. */
export type BrowserCacheStore = {
  get(key: string): Promise<Uint8Array | null>;
  set(key: string, value: Uint8Array): Promise<void>;
  delete(key: string): Promise<void>;
  keys(): Promise<string[]>;
};

const FORMAT = "v1";
// Fast enough to write without a pause the viewer notices, and still
// several times smaller than the JSON.
const ZSTD_LEVEL = 6;

function segment(value: string) {
  return encodeURIComponent(value).replaceAll(".", "%2E");
}

/**
 * The key of one entry. A viewer keeps one entry per area and kind: storing
 * another variant replaces it. Other viewers never read it.
 */
export function browserCacheKey(
  viewer: string,
  area: string,
  kind: string,
  variant: string,
) {
  return [FORMAT, viewer, area, kind, variant].map(segment).join(".");
}

async function remove(store: BrowserCacheStore, key: string) {
  try {
    await store.delete(key);
  } catch {
    // A store that cannot be written holds nothing to clear.
  }
}

/** The stored value, or null when there is none or it cannot be read. */
export async function readBrowserCache<T>(
  store: BrowserCacheStore,
  key: string,
  codec: () => Promise<BrowserCacheCodec>,
  decode: (value: unknown) => T,
): Promise<T | null> {
  let stored: Uint8Array | null;
  try {
    stored = await store.get(key);
  } catch {
    return null;
  }
  if (!stored) return null;
  try {
    const { decompress } = await codec();
    const json = new TextDecoder().decode(decompress(stored));
    return decode(JSON.parse(json));
  } catch {
    // A damaged entry is dropped; the server's copy replaces it.
    await remove(store, key);
    return null;
  }
}

/**
 * Stores the value under the key, replacing the viewer's other variants of
 * that kind in the area. With no room, other viewers' and areas' entries
 * give way; a value that still does not fit is simply not kept. A null value
 * clears the entry.
 */
export async function writeBrowserCache(
  store: BrowserCacheStore,
  key: string,
  value: unknown,
  codec: () => Promise<BrowserCacheCodec>,
): Promise<void> {
  let keys: string[];
  try {
    keys = await store.keys();
  } catch {
    return;
  }
  // `<format>.<viewer>.<area>.<kind>.` and the area's own prefix.
  const kindPrefix = key.slice(0, key.lastIndexOf(".") + 1);
  const ownArea = kindPrefix.slice(
    0,
    kindPrefix.lastIndexOf(".", kindPrefix.length - 2) + 1,
  );
  for (const other of keys)
    if (
      other !== key &&
      (other.startsWith(kindPrefix) || !other.startsWith(`${FORMAT}.`))
    )
      await remove(store, other);
  if (value === null) {
    await remove(store, key);
    return;
  }
  let stored: Uint8Array;
  try {
    const { compress } = await codec();
    stored = compress(new TextEncoder().encode(JSON.stringify(value)));
  } catch {
    return;
  }
  try {
    await store.set(key, stored);
    return;
  } catch {
    // Most likely out of room: make it, then try once more.
  }
  for (const other of keys)
    if (other !== key && !other.startsWith(ownArea)) await remove(store, other);
  try {
    await store.set(key, stored);
  } catch {
    await remove(store, key);
  }
}

let zstd: Promise<BrowserCacheCodec> | null = null;

/** The zstd codec, loaded on first use. */
export function browserCacheZstdCodec(): Promise<BrowserCacheCodec> {
  zstd ??= import("@bokuweb/zstd-wasm")
    .then(async ({ init, compress, decompress }) => {
      await init();
      return {
        compress: (bytes: Uint8Array) => compress(bytes, ZSTD_LEVEL),
        decompress: (bytes: Uint8Array) => decompress(bytes),
      };
    })
    .catch((error: unknown) => {
      zstd = null;
      throw error;
    });
  return zstd;
}

const DATABASE = "scryer.cache";
const ENTRIES = "entries";

async function request<T>(open: Promise<IDBRequest<T>>): Promise<T> {
  const pending = await open;
  return new Promise((resolve, reject) => {
    pending.onsuccess = () => resolve(pending.result);
    pending.onerror = () => reject(pending.error);
  });
}

let store: BrowserCacheStore | null | undefined;

/** The browser's IndexedDB store, or null where it is unavailable. */
export function browserCacheStore(): BrowserCacheStore | null {
  if (store !== undefined) return store;
  if (typeof indexedDB === "undefined") return (store = null);
  let database: Promise<IDBDatabase> | null = null;
  const open = () => {
    database ??= new Promise<IDBDatabase>((resolve, reject) => {
      const opening = indexedDB.open(DATABASE, 1);
      opening.onupgradeneeded = () => {
        opening.result.createObjectStore(ENTRIES);
      };
      opening.onsuccess = () => resolve(opening.result);
      opening.onerror = () => reject(opening.error);
    }).catch((error: unknown) => {
      database = null;
      throw error;
    });
    return database;
  };
  const entries = async (mode: IDBTransactionMode) =>
    (await open()).transaction(ENTRIES, mode).objectStore(ENTRIES);
  store = {
    async get(key) {
      const value: unknown = await request(
        entries("readonly").then((entry) => entry.get(key)),
      );
      return value instanceof Uint8Array ? value : null;
    },
    async set(key, value) {
      await request(
        entries("readwrite").then((entry) => entry.put(value, key)),
      );
    },
    async delete(key) {
      await request(entries("readwrite").then((entry) => entry.delete(key)));
    },
    async keys() {
      const keys = await request(
        entries("readonly").then((entry) => entry.getAllKeys()),
      );
      return keys.filter((key): key is string => typeof key === "string");
    },
  };
  return store;
}

// A copy of each facet's library graph, and of the catalog listing drawn over
// it, kept in the browser's IndexedDB, so the graph view draws and opens up at
// once from the last ones it read while the server is asked again.
// Both run to megabytes of JSON, so each is compressed with zstd, and the
// graph is stored compactly first: node keys once, edges and kinds by index.

import type {
  LibraryGraphCluster,
  LibraryGraphEdge,
  LibraryGraphNode,
  LibraryGraphResult,
} from "../types/library-graph.ts";
import type { TitleRecord } from "../types/titles.ts";
import type { LibraryGraphRequest } from "./library-graph-source.ts";

export type LibraryGraphCodec = {
  compress(bytes: Uint8Array): Uint8Array;
  decompress(bytes: Uint8Array): Uint8Array;
};

/** The store the cache needs; IndexedDB in the app. */
export type LibraryGraphStore = {
  get(key: string): Promise<Uint8Array | null>;
  set(key: string, value: Uint8Array): Promise<void>;
  delete(key: string): Promise<void>;
  keys(): Promise<string[]>;
};

const FORMAT = "v1";
// Fast enough to write without a pause the viewer notices, and still
// several times smaller than the encoded graph.
const ZSTD_LEVEL = 6;

type CompactGraph = {
  g: string;
  // Subject key, target key (0 when it is the subject key), cluster id.
  n: [string, string | 0, number][];
  k: string[];
  // From and to node indexes, weight, then kind indexes.
  e: number[][];
  c: [number, string | null, string, string, number][];
  s?: number;
  t?: boolean;
};

export function encodeLibraryGraph(graph: LibraryGraphResult): CompactGraph {
  const nodeIndex = new Map<string, number>();
  graph.nodes.forEach((node, index) => nodeIndex.set(node.subjectKey, index));
  const kindIndex = new Map<string, number>();
  const kinds: string[] = [];
  const edges: number[][] = [];
  for (const edge of graph.edges) {
    const from = nodeIndex.get(edge.from);
    const to = nodeIndex.get(edge.to);
    if (from === undefined || to === undefined) continue;
    const row = [from, to, edge.weight];
    for (const kind of edge.kinds) {
      let index = kindIndex.get(kind);
      if (index === undefined) {
        index = kinds.push(kind) - 1;
        kindIndex.set(kind, index);
      }
      row.push(index);
    }
    edges.push(row);
  }
  return {
    g: graph.generatedAt,
    n: graph.nodes.map((node) => [
      node.subjectKey,
      node.targetKey === node.subjectKey ? 0 : node.targetKey,
      node.clusterId,
    ]),
    k: kinds,
    e: edges,
    c: graph.clusters.map((cluster) => [
      cluster.id,
      cluster.labelKey,
      cluster.label,
      cluster.medoid,
      cluster.size,
    ]),
    ...(graph.subjectCount === undefined ? {} : { s: graph.subjectCount }),
    ...(graph.truncated === undefined ? {} : { t: graph.truncated }),
  };
}

export function decodeLibraryGraph(compact: CompactGraph): LibraryGraphResult {
  const nodes: LibraryGraphNode[] = compact.n.map(
    ([subjectKey, targetKey, clusterId]) => ({
      subjectKey,
      targetKey: targetKey === 0 ? subjectKey : targetKey,
      clusterId,
    }),
  );
  const edges: LibraryGraphEdge[] = compact.e.map(
    ([from, to, weight, ...kinds]) => ({
      from: nodes[from].subjectKey,
      to: nodes[to].subjectKey,
      weight,
      kinds: kinds.map((index) => compact.k[index]),
    }),
  );
  const clusters: LibraryGraphCluster[] = compact.c.map(
    ([id, labelKey, label, medoid, size]) => ({
      id,
      labelKey,
      label,
      medoid,
      size,
    }),
  );
  return {
    generatedAt: compact.g,
    nodes,
    edges,
    clusters,
    ...(compact.s === undefined ? {} : { subjectCount: compact.s }),
    ...(compact.t === undefined ? {} : { truncated: compact.t }),
  };
}

// A listing is stored by field, one array per field, which zstd packs about
// a fifth smaller than one object per title. `a` names, per field, the titles
// that lack it, so a record reads back with the keys it had.
type CompactTitles = {
  k: string[];
  v: unknown[][];
  a?: Record<number, number[]>;
};

export function encodeTitleListing(
  titles: readonly TitleRecord[],
): CompactTitles {
  const fields: string[] = [];
  const index = new Map<string, number>();
  for (const title of titles)
    for (const field of Object.keys(title))
      if (!index.has(field)) index.set(field, fields.push(field) - 1);
  const absent: Record<number, number[]> = {};
  const values = fields.map((field, column) =>
    titles.map((title, row) => {
      if (Object.hasOwn(title, field))
        return (title as Record<string, unknown>)[field];
      (absent[column] ??= []).push(row);
      return null;
    }),
  );
  return {
    k: fields,
    v: values,
    ...(Object.keys(absent).length ? { a: absent } : {}),
  };
}

export function decodeTitleListing(compact: CompactTitles): TitleRecord[] {
  const rows = compact.v[0]?.length ?? 0;
  const titles = Array.from(
    { length: rows },
    () => ({}) as Record<string, unknown>,
  );
  compact.k.forEach((field, column) => {
    const missing = new Set(compact.a?.[column]);
    compact.v[column].forEach((value, row) => {
      if (!missing.has(row)) titles[row][field] = value;
    });
  });
  return titles as TitleRecord[];
}

/** What an entry holds: the graph, or the catalog listing drawn over it. */
export type LibraryGraphCacheKind = "graph" | "titles";

/** The viewer's own entries for one facet; other viewers never read them. */
function facetPrefix(viewer: string, facet: string) {
  return `${FORMAT}.${encodeURIComponent(viewer)}.${facet}.`;
}

export function libraryGraphCacheKey(
  viewer: string,
  request: LibraryGraphRequest,
  kind: LibraryGraphCacheKind = "graph",
) {
  const libraries = [...request.libraryIds].sort().join(",");
  return `${facetPrefix(viewer, request.facet)}${kind}.${libraries || "*"}`;
}

async function remove(store: LibraryGraphStore, key: string) {
  try {
    await store.delete(key);
  } catch {
    // A store that cannot be written holds nothing to clear.
  }
}

async function readCompressed<T>(
  store: LibraryGraphStore,
  key: string,
  codec: () => Promise<LibraryGraphCodec>,
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
 * Stores the value under the key, replacing the viewer's other entries of
 * that kind for the facet. With no room, other viewers' and facets' entries
 * give way; a value that still does not fit is simply not kept. A null value
 * clears the entry.
 */
async function writeCompressed(
  store: LibraryGraphStore,
  key: string,
  value: unknown,
  codec: () => Promise<LibraryGraphCodec>,
): Promise<void> {
  let keys: string[];
  try {
    keys = await store.keys();
  } catch {
    return;
  }
  // `<format>.<viewer>.<facet>.<kind>.` and the facet's own prefix.
  const kindPrefix = key.slice(0, key.lastIndexOf(".") + 1);
  const ownFacet = kindPrefix.slice(
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
    if (other !== key && !other.startsWith(ownFacet))
      await remove(store, other);
  try {
    await store.set(key, stored);
  } catch {
    await remove(store, key);
  }
}

/** The stored graph, or null when there is none or it cannot be read. */
export function readCachedLibraryGraph(
  store: LibraryGraphStore,
  key: string,
  codec: () => Promise<LibraryGraphCodec>,
): Promise<LibraryGraphResult | null> {
  return readCompressed(store, key, codec, (value) =>
    decodeLibraryGraph(value as CompactGraph),
  );
}

/** Stores the graph; a null graph clears the entry. */
export function writeCachedLibraryGraph(
  store: LibraryGraphStore,
  key: string,
  graph: LibraryGraphResult | null,
  codec: () => Promise<LibraryGraphCodec>,
): Promise<void> {
  return writeCompressed(store, key, graph && encodeLibraryGraph(graph), codec);
}

/** The stored catalog listing, or null when there is none or it is unreadable. */
export function readCachedGraphTitles(
  store: LibraryGraphStore,
  key: string,
  codec: () => Promise<LibraryGraphCodec>,
): Promise<TitleRecord[] | null> {
  return readCompressed(store, key, codec, (value) =>
    decodeTitleListing(value as CompactTitles),
  );
}

/** Stores the complete catalog listing the graph was drawn over. */
export function writeCachedGraphTitles(
  store: LibraryGraphStore,
  key: string,
  titles: readonly TitleRecord[],
  codec: () => Promise<LibraryGraphCodec>,
): Promise<void> {
  return writeCompressed(store, key, encodeTitleListing(titles), codec);
}

let zstd: Promise<LibraryGraphCodec> | null = null;

/** The zstd codec, loaded on first use. */
export function libraryGraphZstdCodec(): Promise<LibraryGraphCodec> {
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

const DATABASE = "scryer.libraryGraph";
const ENTRIES = "entries";

async function request<T>(open: Promise<IDBRequest<T>>): Promise<T> {
  const pending = await open;
  return new Promise((resolve, reject) => {
    pending.onsuccess = () => resolve(pending.result);
    pending.onerror = () => reject(pending.error);
  });
}

let store: LibraryGraphStore | null | undefined;

/** The browser's IndexedDB store, or null where it is unavailable. */
export function libraryGraphStore(): LibraryGraphStore | null {
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

// A copy of each facet's library graph kept in the browser, so the graph view
// draws at once from the last one it read while the server is asked again.
// A graph runs to megabytes of JSON, so it is stored compactly: node keys once,
// edges and kinds by index, the whole compressed with zstd.

import type {
  LibraryGraphCluster,
  LibraryGraphEdge,
  LibraryGraphNode,
  LibraryGraphResult,
} from "../types/library-graph.ts";
import type { LibraryGraphRequest } from "./library-graph-source.ts";

export type LibraryGraphCodec = {
  compress(bytes: Uint8Array): Uint8Array;
  decompress(bytes: Uint8Array): Uint8Array;
};

/** The storage the cache needs; `localStorage` in the app. */
export type LibraryGraphStorage = Pick<
  Storage,
  "getItem" | "setItem" | "removeItem" | "key" | "length"
>;

const KEY_PREFIX = "scryer.libraryGraph.";
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

/** The viewer's own entry for one facet; other viewers never read it. */
function facetPrefix(viewer: string, request: LibraryGraphRequest) {
  return `${KEY_PREFIX}${FORMAT}.${encodeURIComponent(viewer)}.${request.facet}.`;
}

export function libraryGraphCacheKey(
  viewer: string,
  request: LibraryGraphRequest,
) {
  const libraries = [...request.libraryIds].sort().join(",");
  return `${facetPrefix(viewer, request)}${libraries || "*"}`;
}

function toBase64(bytes: Uint8Array) {
  let binary = "";
  for (let offset = 0; offset < bytes.length; offset += 0x8000)
    binary += String.fromCharCode(...bytes.subarray(offset, offset + 0x8000));
  return btoa(binary);
}

function fromBase64(text: string) {
  const binary = atob(text);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1)
    bytes[index] = binary.charCodeAt(index);
  return bytes;
}

function storedKeys(storage: LibraryGraphStorage) {
  const keys: string[] = [];
  for (let index = 0; index < storage.length; index += 1) {
    const key = storage.key(index);
    if (key?.startsWith(KEY_PREFIX)) keys.push(key);
  }
  return keys;
}

function remove(storage: LibraryGraphStorage, key: string) {
  try {
    storage.removeItem(key);
  } catch {
    // Storage that cannot be written holds nothing to clear.
  }
}

/** The stored graph, or null when there is none or it cannot be read. */
export async function readCachedLibraryGraph(
  storage: LibraryGraphStorage,
  key: string,
  codec: () => Promise<LibraryGraphCodec>,
): Promise<LibraryGraphResult | null> {
  let stored: string | null;
  try {
    stored = storage.getItem(key);
  } catch {
    return null;
  }
  if (!stored) return null;
  try {
    const { decompress } = await codec();
    const json = new TextDecoder().decode(decompress(fromBase64(stored)));
    return decodeLibraryGraph(JSON.parse(json) as CompactGraph);
  } catch {
    // A damaged entry is dropped; the server's graph replaces it.
    remove(storage, key);
    return null;
  }
}

/**
 * Stores the graph under the key, replacing the viewer's other entries for
 * the facet. With no room, every other stored graph gives way; a graph that
 * still does not fit is simply not kept. A null graph clears the entry.
 */
export async function writeCachedLibraryGraph(
  storage: LibraryGraphStorage,
  key: string,
  graph: LibraryGraphResult | null,
  codec: () => Promise<LibraryGraphCodec>,
): Promise<void> {
  let keys: string[];
  try {
    keys = storedKeys(storage);
  } catch {
    return;
  }
  const facet = key.slice(0, key.lastIndexOf(".") + 1);
  for (const other of keys)
    if (
      other !== key &&
      (other.startsWith(facet) || !other.startsWith(`${KEY_PREFIX}${FORMAT}.`))
    )
      remove(storage, other);
  if (!graph) {
    remove(storage, key);
    return;
  }
  let value: string;
  try {
    const { compress } = await codec();
    const json = JSON.stringify(encodeLibraryGraph(graph));
    value = toBase64(compress(new TextEncoder().encode(json)));
  } catch {
    return;
  }
  try {
    storage.setItem(key, value);
    return;
  } catch {
    // Most likely out of room: make it, then try once more.
  }
  for (const other of keys) if (other !== key) remove(storage, other);
  try {
    storage.setItem(key, value);
  } catch {
    remove(storage, key);
  }
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

/** The browser's local storage, or null where it is unavailable. */
export function libraryGraphStorage(): LibraryGraphStorage | null {
  try {
    return typeof window === "undefined" ? null : window.localStorage;
  } catch {
    return null;
  }
}

// The library graph and the catalog listing drawn over it, kept in the
// browser cache so the graph view draws and opens up at once on a return
// visit. Both run to megabytes of JSON, so each is laid out compactly before
// it is compressed: the graph's node keys once with edges and kinds by index,
// the listing one array per field.

import type {
  LibraryGraphCluster,
  LibraryGraphEdge,
  LibraryGraphNode,
  LibraryGraphResult,
} from "../types/library-graph.ts";
import type { TitleRecord } from "../types/titles.ts";
import {
  browserCacheKey,
  readBrowserCache,
  writeBrowserCache,
  type BrowserCacheCodec,
  type BrowserCacheStore,
} from "./browser-cache.ts";
import type { LibraryGraphRequest } from "./library-graph-source.ts";

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

export function libraryGraphCacheKey(
  viewer: string,
  request: LibraryGraphRequest,
  kind: LibraryGraphCacheKind = "graph",
) {
  const libraries = [...request.libraryIds].sort().join(",");
  return browserCacheKey(viewer, request.facet, kind, libraries || "*");
}

/** The stored graph, or null when there is none or it cannot be read. */
export function readCachedLibraryGraph(
  store: BrowserCacheStore,
  key: string,
  codec: () => Promise<BrowserCacheCodec>,
): Promise<LibraryGraphResult | null> {
  return readBrowserCache(store, key, codec, (value) =>
    decodeLibraryGraph(value as CompactGraph),
  );
}

/** Stores the graph; a null graph clears the entry. */
export function writeCachedLibraryGraph(
  store: BrowserCacheStore,
  key: string,
  graph: LibraryGraphResult | null,
  codec: () => Promise<BrowserCacheCodec>,
): Promise<void> {
  return writeBrowserCache(
    store,
    key,
    graph && encodeLibraryGraph(graph),
    codec,
  );
}

/** The stored catalog listing, or null when there is none or it is unreadable. */
export function readCachedGraphTitles(
  store: BrowserCacheStore,
  key: string,
  codec: () => Promise<BrowserCacheCodec>,
): Promise<TitleRecord[] | null> {
  return readBrowserCache(store, key, codec, (value) =>
    decodeTitleListing(value as CompactTitles),
  );
}

/** Stores the complete catalog listing the graph was drawn over. */
export function writeCachedGraphTitles(
  store: BrowserCacheStore,
  key: string,
  titles: readonly TitleRecord[],
  codec: () => Promise<BrowserCacheCodec>,
): Promise<void> {
  return writeBrowserCache(store, key, encodeTitleListing(titles), codec);
}

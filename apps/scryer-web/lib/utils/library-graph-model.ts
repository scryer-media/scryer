// Turns a library graph and the catalog's current titles into what the graph
// view draws: nodes joined to titles and pruned to the filtered set, node
// sizes from a chosen metric, collapsed clusters above a size limit, and the
// localized names of clusters and edge kinds.

import type { Facet, TitleRecord } from "../types/titles.ts";
import {
  UNPLACED_CLUSTER_ID,
  type LibraryGraphCluster,
  type LibraryGraphResult,
} from "../types/library-graph.ts";
import {
  clustersToCollapse,
  collapseClusters,
  graphStateFromResult,
  pruneToVisible,
  type GraphNode,
  type GraphState,
} from "./library-graph-merge.ts";
import { discoveryBrowseLabelTitle } from "./discovery-sections.ts";

/** Above this many nodes, clusters collapse to one node each until expanded. */
export const LIBRARY_GRAPH_COLLAPSE_THRESHOLD = 5000;

export type LibraryGraphSizeMetric = "ratingEvidence" | "rating" | "popularity";

export const LIBRARY_GRAPH_SIZE_METRICS: readonly LibraryGraphSizeMetric[] = [
  "ratingEvidence",
  "rating",
  "popularity",
];

export const LIBRARY_GRAPH_MIN_NODE_SIZE = 4;
export const LIBRARY_GRAPH_MAX_NODE_SIZE = 18;

type Translate = (
  key: string,
  values?: Record<string, string | number | boolean | null | undefined>,
) => string;

function normalizeKey(value: string | null | undefined): string {
  return (value ?? "").trim().toLowerCase();
}

function canonicalIdValue(value: string): string {
  return /^\d+$/.test(value) ? String(Number.parseInt(value, 10)) : value;
}

function facetSlug(facet: Facet): string {
  return facet.toLowerCase();
}

/**
 * The keys a discovery target key can match a catalog title by: the whole key,
 * and `source:id` when the key reads `source:kind:id`.
 */
export function targetKeyMatchKeys(targetKey: string): string[] {
  const keys = new Set<string>();
  const whole = normalizeKey(targetKey);
  if (whole) {
    keys.add(whole);
  }
  const parts = targetKey
    .split(":")
    .map((part) => part.trim())
    .filter(Boolean);
  if (parts.length >= 3) {
    const source = normalizeKey(parts[0]);
    const value = normalizeKey(parts.slice(2).join(":"));
    if (source && value) {
      keys.add(`${source}:${value}`);
      keys.add(`${source}:${canonicalIdValue(value)}`);
    }
  }
  return Array.from(keys);
}

/** The keys a catalog title answers to, from its IMDb id and external ids. */
export function titleMatchKeys(title: TitleRecord): string[] {
  const keys = new Set<string>();
  const add = (source: string, rawValue: string | null | undefined) => {
    const normalizedSource = normalizeKey(source);
    const value = normalizeKey(rawValue);
    if (!normalizedSource || !value) {
      return;
    }
    for (const candidate of new Set([value, canonicalIdValue(value)])) {
      keys.add(`${normalizedSource}:${candidate}`);
      keys.add(`${normalizedSource}:${facetSlug(title.facet)}:${candidate}`);
      if (title.facet === "ANIME") {
        keys.add(`${normalizedSource}:series:${candidate}`);
      }
    }
  };
  add("imdb", title.imdbId);
  for (const externalId of title.externalIds ?? []) {
    add(externalId.source, externalId.value);
  }
  return Array.from(keys);
}

/**
 * The discovery target key a catalog title would carry, from its first
 * external id, or null when it has none.
 */
export function titleTargetKey(title: TitleRecord): string | null {
  const first = (title.externalIds ?? []).find(
    (externalId) => externalId.source.trim() && externalId.value.trim(),
  );
  if (first) {
    return `${normalizeKey(first.source)}:${facetSlug(title.facet)}:${first.value.trim()}`;
  }
  return title.imdbId?.trim()
    ? `imdb:${facetSlug(title.facet)}:${title.imdbId.trim()}`
    : null;
}

/** A lookup from target key to the catalog title it names. */
export function titleIndexByMatchKey(
  titles: readonly TitleRecord[],
): Map<string, TitleRecord> {
  const index = new Map<string, TitleRecord>();
  for (const title of titles) {
    for (const key of titleMatchKeys(title)) {
      if (!index.has(key)) {
        index.set(key, title);
      }
    }
  }
  return index;
}

export function titleForTargetKey(
  targetKey: string,
  index: ReadonlyMap<string, TitleRecord>,
): TitleRecord | null {
  for (const key of targetKeyMatchKeys(targetKey)) {
    const title = index.get(key);
    if (title) {
      return title;
    }
  }
  return null;
}

/** The raw value a title has for a size metric, or null when it has none. */
export function titleSizeMetricValue(
  title: TitleRecord,
  metric: LibraryGraphSizeMetric,
): number | null {
  switch (metric) {
    case "rating": {
      const rating = title.ratings?.rating;
      return typeof rating === "number" && Number.isFinite(rating)
        ? rating
        : null;
    }
    case "popularity": {
      const popularity = title.popularity;
      return typeof popularity === "number" && Number.isFinite(popularity)
        ? popularity
        : null;
    }
    default: {
      // How much rating evidence backs the title: the votes behind its
      // external ratings, or the number of rating sources when no source
      // reports votes. Log-scaled so a few huge titles do not dwarf the rest.
      const ratings = title.ratings;
      if (!ratings) {
        return null;
      }
      const votes = ratings.externalRatings.reduce(
        (total, rating) =>
          total +
          (typeof rating.votes === "number" && rating.votes > 0
            ? rating.votes
            : 0),
        0,
      );
      if (votes > 0) {
        return Math.log10(1 + votes);
      }
      const sources = Math.max(
        ratings.ratingSources.length,
        ratings.externalRatings.length,
      );
      return sources > 0 ? Math.log10(1 + sources) : null;
    }
  }
}

/**
 * Node sizes for the given titles: the metric scaled between the minimum and
 * maximum node size. A title with no value, or a set where every value is the
 * same, sits at the minimum and the midpoint respectively.
 */
export function nodeSizesByTitleId(
  titles: readonly TitleRecord[],
  metric: LibraryGraphSizeMetric,
): Map<string, number> {
  const values = new Map<string, number | null>();
  let min = Number.POSITIVE_INFINITY;
  let max = Number.NEGATIVE_INFINITY;
  for (const title of titles) {
    const value = titleSizeMetricValue(title, metric);
    values.set(title.id, value);
    if (value !== null) {
      min = Math.min(min, value);
      max = Math.max(max, value);
    }
  }
  const sizes = new Map<string, number>();
  const span = LIBRARY_GRAPH_MAX_NODE_SIZE - LIBRARY_GRAPH_MIN_NODE_SIZE;
  for (const [id, value] of values) {
    if (value === null) {
      sizes.set(id, LIBRARY_GRAPH_MIN_NODE_SIZE);
    } else if (max <= min) {
      sizes.set(id, LIBRARY_GRAPH_MIN_NODE_SIZE + span / 2);
    } else {
      sizes.set(
        id,
        LIBRARY_GRAPH_MIN_NODE_SIZE + (span * (value - min)) / (max - min),
      );
    }
  }
  return sizes;
}

export type LibraryGraphJoin = {
  graph: GraphState;
  titleBySubjectKey: Map<string, TitleRecord>;
};

/**
 * The graph joined to the catalog's current titles. Nodes whose target is not
 * among the titles (filtered out by facet chips, library filters or search)
 * are pruned with their edges, and cluster sizes follow.
 */
export function joinLibraryGraphToTitles(
  result: LibraryGraphResult,
  titles: readonly TitleRecord[],
): LibraryGraphJoin {
  const index = titleIndexByMatchKey(titles);
  const titleBySubjectKey = new Map<string, TitleRecord>();
  for (const node of result.nodes) {
    const title = titleForTargetKey(node.targetKey, index);
    if (title) {
      titleBySubjectKey.set(node.subjectKey, title);
    }
  }
  const graph = pruneToVisible(
    graphStateFromResult(result),
    new Set(titleBySubjectKey.keys()),
  );
  return { graph, titleBySubjectKey };
}

/** The graph as drawn: clusters collapsed when the graph is too large. */
export function displayedLibraryGraph(
  graph: GraphState,
  expandedClusterIds: ReadonlySet<number>,
  threshold: number = LIBRARY_GRAPH_COLLAPSE_THRESHOLD,
): GraphState {
  return collapseClusters(
    graph,
    clustersToCollapse(graph, threshold, expandedClusterIds),
  );
}

/** Whether a node stands in for a collapsed cluster. */
export function isCollapsedClusterNode(node: GraphNode): boolean {
  return node.collapsedClusterId !== undefined;
}

function clusterLabelKeyCandidates(labelKey: string): string[] {
  const trimmed = labelKey.trim();
  const parts = trimmed.split(":");
  // A key may carry its family (`theme:isekai`); the label table is keyed by
  // the bare label.
  return parts.length > 1 ? [trimmed, parts[parts.length - 1]] : [trimmed];
}

/**
 * A cluster's name: its label key through the discovery label table, else the
 * medoid title's name, else the label the result carried.
 */
export function libraryGraphClusterName(
  cluster: LibraryGraphCluster,
  titleBySubjectKey: ReadonlyMap<string, TitleRecord>,
  t: Translate,
): string {
  if (cluster.labelKey) {
    for (const candidate of clusterLabelKeyCandidates(cluster.labelKey)) {
      const label = discoveryBrowseLabelTitle(candidate, t);
      if (label) {
        return label;
      }
    }
  }
  const medoid = titleBySubjectKey.get(cluster.medoid);
  if (medoid?.name) {
    return t("libraryGraph.clusterAroundTitle", { title: medoid.name });
  }
  return cluster.label;
}

/** The name of the group unplaced nodes collapse into. */
export function libraryGraphGroupName(
  clusterId: number,
  clusters: readonly LibraryGraphCluster[],
  titleBySubjectKey: ReadonlyMap<string, TitleRecord>,
  t: Translate,
): string {
  if (clusterId === UNPLACED_CLUSTER_ID) {
    return t("libraryGraph.unplaced");
  }
  const cluster = clusters.find((entry) => entry.id === clusterId);
  return cluster
    ? libraryGraphClusterName(cluster, titleBySubjectKey, t)
    : t("libraryGraph.unplaced");
}

const RELATION_KIND_KEYS: Record<string, string> = {
  sequel: "libraryGraph.edgeKind.sequel",
  prequel: "libraryGraph.edgeKind.prequel",
  co_watch: "libraryGraph.edgeKind.coWatch",
};

/** One edge kind in the viewer's language. */
export function libraryGraphEdgeKindLabel(kind: string, t: Translate): string {
  const parts = kind.trim().split(":");
  switch (parts[0]) {
    case "semantic":
      return t("libraryGraph.edgeKind.semantic");
    case "relation": {
      const key = RELATION_KIND_KEYS[parts[1] ?? ""];
      return key ? t(key) : t("libraryGraph.edgeKind.relation");
    }
    case "affinity": {
      const label = parts[parts.length - 1] ?? "";
      const title =
        parts.length > 1 ? discoveryBrowseLabelTitle(label, t) : null;
      return title
        ? t("libraryGraph.edgeKind.affinity", { label: title })
        : t("libraryGraph.edgeKind.sharedTaste");
    }
    default:
      return t("libraryGraph.edgeKind.other");
  }
}

/** Every distinct kind label of an edge, in a stable order. */
export function libraryGraphEdgeKindLabels(
  kinds: readonly string[],
  t: Translate,
): string[] {
  return Array.from(
    new Set(kinds.map((kind) => libraryGraphEdgeKindLabel(kind, t))),
  );
}

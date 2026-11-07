// Turns a library graph and the catalog's current titles into what the graph
// view draws: nodes joined to titles and pruned to the filtered set, node
// sizes from a chosen metric, collapsed clusters above a size limit, and the
// localized names of clusters and edge kinds.

import type { CanonicalMediaTag } from "../types/canonical-tags.ts";
import type { Facet, TitleRecord } from "../types/titles.ts";
import {
  UNPLACED_CLUSTER_ID,
  type LibraryGraphCluster,
  type LibraryGraphEdge,
  type LibraryGraphResult,
} from "../types/library-graph.ts";
import {
  clampEdgeWeight,
  clustersToCollapse,
  edgePairKey,
  collapseClusters,
  graphStateFromResult,
  pruneToVisible,
  type GraphNode,
  type GraphState,
} from "./library-graph-merge.ts";
import { discoveryBrowseLabelTitle } from "./discovery-sections.ts";
import { convexHull, type LayoutPoint } from "./library-graph-layout.ts";
import {
  assignRelatedSpiralSlots,
  packRelatedCircles,
} from "./library-graph-spiral-slots.ts";

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

// The rest of this section mirrors how the server keys discovery targets
// (the discovery records module in scryer-application), so the graph joins
// on exactly the keys the server and the gateway use.

type DiscoverySource = "tvdb" | "tmdb" | "mal" | "anidb" | "anilist" | "imdb";

/** The order the server prefers sources in when a title has several. */
const DISCOVERY_SOURCE_PRIORITY: readonly DiscoverySource[] = [
  "tvdb",
  "tmdb",
  "mal",
  "anidb",
  "anilist",
  "imdb",
];

function discoverySource(raw: string): DiscoverySource | null {
  switch (normalizeKey(raw)) {
    case "imdb":
    case "imdb_id":
      return "imdb";
    case "tvdb":
    case "thetvdb":
    case "tvdb_show":
    case "tvdb_series":
    case "tvdb_movie":
      return "tvdb";
    case "tmdb":
    case "themoviedb":
    case "tmdb_tv":
    case "tmdb_show":
    case "tmdb_series":
    case "tmdb_movie":
      return "tmdb";
    case "anidb":
      return "anidb";
    case "mal":
    case "myanimelist":
      return "mal";
    case "anilist":
    case "anilist_anime":
    case "anilist:anime":
      return "anilist";
    default:
      return null;
  }
}

/** An IMDb id as `tt` and its digits, or null. */
function normalizedImdbId(raw: string): string | null {
  const value = raw.trim().toLowerCase();
  if (!value) {
    return null;
  }
  const match = /tt(\d+)/.exec(value);
  if (match) {
    return `tt${match[1]}`;
  }
  return /^\d+$/.test(value) ? `tt${value}` : null;
}

/** A positive numeric id, taken from the last `:` part, or null. */
function positiveNumericId(raw: string): string | null {
  const value = raw.trim();
  const last = value.slice(value.lastIndexOf(":") + 1).trim();
  if (!/^\d+$/.test(last)) {
    return null;
  }
  const id = BigInt(last);
  return id > 0n ? id.toString() : null;
}

function discoveryIdValue(source: DiscoverySource, raw: string): string | null {
  return source === "imdb" ? normalizedImdbId(raw) : positiveNumericId(raw);
}

/**
 * The target key an id names for a title of `facet`, in the server's forms:
 * `tvdb`/`tmdb` under `movie` or `series` (anime counts as series), MAL,
 * AniDB and AniList always under `anime`, and IMDb under `title`.
 */
function discoveryTargetKey(
  source: DiscoverySource,
  value: string,
  facet: Facet,
): string {
  switch (source) {
    case "tvdb":
    case "tmdb":
      return `${source}:${facet === "MOVIE" ? "movie" : "series"}:${value}`;
    case "imdb":
      return `imdb:title:${value}`;
    default:
      return `${source}:anime:${value}`;
  }
}

function titleDiscoveryIds(title: TitleRecord): Map<DiscoverySource, string> {
  const ids = new Map<DiscoverySource, string>();
  const add = (rawSource: string, rawValue: string | null | undefined) => {
    const source = discoverySource(rawSource);
    const value =
      source && rawValue ? discoveryIdValue(source, rawValue) : null;
    if (source && value && !ids.has(source)) {
      ids.set(source, value);
    }
  };
  for (const externalId of title.externalIds ?? []) {
    add(externalId.source, externalId.value);
  }
  add("imdb", title.imdbId);
  return ids;
}

/**
 * The key a discovery target key matches a catalog title by: the key in its
 * normalized form. Keys keep their kind, so `tmdb:movie:123` never matches a
 * series with TMDB id 123; a key without a kind matches nothing.
 */
export function targetKeyMatchKeys(targetKey: string): string[] {
  const parts = targetKey.split(":").map((part) => part.trim());
  if (parts.length < 3) {
    return [];
  }
  const source = discoverySource(parts[0]);
  const kind = normalizeKey(parts[1]);
  const value = source
    ? discoveryIdValue(source, parts.slice(2).join(":"))
    : null;
  if (!source || !kind || !value) {
    return [];
  }
  return [`${source}:${kind}:${value}`];
}

/** The target keys the server would give a catalog title, one per source. */
export function titleMatchKeys(title: TitleRecord): string[] {
  return Array.from(titleDiscoveryIds(title), ([source, value]) =>
    discoveryTargetKey(source, value, title.facet),
  );
}

/**
 * The discovery target key the server prefers for a catalog title, from its
 * ids in the server's source order, or null when it has none.
 */
export function titleTargetKey(title: TitleRecord): string | null {
  const ids = titleDiscoveryIds(title);
  for (const source of DISCOVERY_SOURCE_PRIORITY) {
    const value = ids.get(source);
    if (value) {
      return discoveryTargetKey(source, value, title.facet);
    }
  }
  return null;
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
 * maximum node size (popularity by rank, so one outlier cannot flatten it). A title with no value, or a set where every value is the
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
  if (metric === "popularity") {
    // Popularity is heavy-tailed: one huge title would shrink the rest to
    // nothing on a straight scale. Size by rank among the shown titles
    // instead, so a library of popular titles still spreads out.
    const ranked = [...values.values()]
      .filter((value): value is number => value !== null)
      .sort((a, b) => a - b);
    // Tied values share the middle of their ranks.
    const ranks = new Map<number, number>();
    for (let first = 0; first < ranked.length;) {
      let last = first;
      while (last + 1 < ranked.length && ranked[last + 1] === ranked[first])
        last += 1;
      ranks.set(ranked[first], (first + last) / 2);
      first = last + 1;
    }
    for (const [id, value] of values) {
      if (value === null) {
        sizes.set(id, LIBRARY_GRAPH_MIN_NODE_SIZE);
      } else if (ranked[0] === ranked[ranked.length - 1]) {
        sizes.set(id, LIBRARY_GRAPH_MIN_NODE_SIZE + span / 2);
      } else {
        sizes.set(
          id,
          LIBRARY_GRAPH_MIN_NODE_SIZE +
            (span * ranks.get(value)!) / (ranked.length - 1),
        );
      }
    }
    return sizes;
  }
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

/** Neighborhood membership and connections are available before catalog metadata. */
export function libraryGraphOverviewPreview(
  result: LibraryGraphResult,
): LibraryGraphJoin {
  return {
    graph: {
      nodes: result.nodes.map((node) => ({ ...node, ghost: false })),
      edges: result.edges,
      clusters: result.clusters,
    },
    titleBySubjectKey: new Map(),
  };
}

function sameData(a: unknown, b: unknown): boolean {
  if (Object.is(a, b)) return true;
  if (
    typeof a !== "object" ||
    typeof b !== "object" ||
    a === null ||
    b === null ||
    Array.isArray(a) !== Array.isArray(b)
  )
    return false;
  if (Array.isArray(a)) {
    const other = b as unknown[];
    if (a.length !== other.length) return false;
    for (let index = 0; index < a.length; index += 1)
      if (!sameData(a[index], other[index])) return false;
    return true;
  }
  const left = a as Record<string, unknown>;
  const right = b as Record<string, unknown>;
  const keys = Object.keys(left);
  if (keys.length !== Object.keys(right).length) return false;
  for (const key of keys)
    if (!Object.hasOwn(right, key) || !sameData(left[key], right[key]))
      return false;
  return true;
}

/**
 * Whether a refetched catalog listing says exactly what the last one did. A
 * refetch builds new title objects even when nothing changed, and joining a
 * large library again re-lays out and redraws the whole graph.
 */
// Backdrops fill in a few titles at a time while artwork is fetched, and the
// graph never draws them; the selection card reads them from the live listing.
const UNDRAWN_TITLE_FIELDS: ReadonlySet<string> = new Set([
  "backgroundUrl",
  "backgroundSourceUrl",
]);

/** Whether a refetched listing changes nothing the graph draws. */
export function sameTitleRecords(
  previous: readonly TitleRecord[],
  next: readonly TitleRecord[],
) {
  if (previous === next) return true;
  if (previous.length !== next.length) return false;
  for (let index = 0; index < previous.length; index += 1) {
    const left = previous[index] as unknown as Record<string, unknown>;
    const right = next[index] as unknown as Record<string, unknown>;
    if (left === right) continue;
    const drawn = (record: Record<string, unknown>) =>
      Object.keys(record).filter((key) => !UNDRAWN_TITLE_FIELDS.has(key));
    const keys = drawn(left);
    if (keys.length !== drawn(right).length) return false;
    for (const key of keys)
      if (!Object.hasOwn(right, key) || !sameData(left[key], right[key]))
        return false;
  }
  return true;
}

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

/**
 * The graph as drawn: clusters collapsed when the graph is too large. Whether
 * it is too large is read from `owned`, the library's own nodes, so explored
 * titles shown beside them never tip the view into collapsing.
 */
export function displayedLibraryGraph(
  graph: GraphState,
  expandedClusterIds: ReadonlySet<number>,
  threshold: number = LIBRARY_GRAPH_COLLAPSE_THRESHOLD,
  owned: GraphState = graph,
  selectedClusters: ReadonlySet<number> = new Set(),
): GraphState {
  const large = owned.nodes.length > threshold;
  const selected = Array.from(selectedClusters)
    .sort((a, b) => a - b)
    .join(",");
  let filtered = foldBases.get(graph);
  if (
    !filtered ||
    filtered.owned !== owned ||
    filtered.large !== large ||
    filtered.selected !== selected
  ) {
    filtered = {
      owned,
      large,
      selected,
      graph: foldBase(graph, owned, large, selectedClusters),
    };
    foldBases.set(graph, filtered);
  }
  return collapseClusters(
    filtered.graph,
    large
      ? clustersToCollapse(filtered.graph, -1, expandedClusterIds)
      : new Set(),
  );
}

/** The graph a fold starts from, kept per graph so each fold reuses the
 * collapse index built for it. */
const foldBases = new WeakMap<
  GraphState,
  { owned: GraphState; large: boolean; selected: string; graph: GraphState }
>();

function foldBase(
  graph: GraphState,
  owned: GraphState,
  large: boolean,
  selectedClusters: ReadonlySet<number>,
): GraphState {
  const named = libraryGraphNamedClusterIds(owned.clusters);
  // Unnamed communities are not meaningful neighbourhoods. Keep their
  // titles reachable through the existing unplaced group, without inventing
  // thousands of "Around …" neighbourhoods.
  const grouped = large
    ? {
        ...graph,
        nodes: graph.nodes.map((node) =>
          named.has(node.clusterId)
            ? node
            : { ...node, clusterId: UNPLACED_CLUSTER_ID },
        ),
        clusters: graph.clusters.filter((cluster) => named.has(cluster.id)),
      }
    : graph;
  return selectedClusters.size === 0
    ? grouped
    : pruneToVisible(
        grouped,
        new Set(
          grouped.nodes
            .filter((node) => selectedClusters.has(node.clusterId))
            .map((node) => node.subjectKey),
        ),
      );
}

/** Whether a node stands in for a collapsed cluster. */
export function isCollapsedClusterNode(node: GraphNode): boolean {
  return node.collapsedClusterId !== undefined;
}

/** How strongly cross-neighbourhood ties pull a title toward its spiral's
 * facing side, relative to its ties within the neighbourhood. */
const CROSS_LEAN = 0.1;

/** Stable nested positions at library scale; zoom never restarts a force
 * simulation or moves the neighbourhood being inspected. */
export function largeLibraryGraphLayout(graph: GraphState) {
  const named = libraryGraphNamedClusterIds(graph.clusters);
  const groups = new Map<number, GraphNode[]>();
  const groupByKey = new Map<string, number>();
  for (const node of graph.nodes) {
    const id = named.has(node.clusterId) ? node.clusterId : UNPLACED_CLUSTER_ID;
    const members = groups.get(id) ?? [];
    members.push(node);
    groups.set(id, members);
    groupByKey.set(node.subjectKey, id);
  }
  const ids = [...groups.keys()].sort((a, b) => a - b);
  const slotOf = new Map(ids.map((id, index) => [id, index]));
  // Partition once, so each neighbourhood only examines its own relations;
  // relations across neighbourhoods decide which neighbourhoods sit together.
  const edgesByGroup = new Map<number, LibraryGraphEdge[]>();
  const crossing = new Map<string, LibraryGraphEdge>();
  for (const edge of graph.edges) {
    const id = groupByKey.get(edge.from);
    const other = groupByKey.get(edge.to);
    if (id === undefined || other === undefined) continue;
    if (id !== other) {
      const pair = edgePairKey(edge.from, edge.to);
      const weight = clampEdgeWeight(edge.weight);
      if (weight > clampEdgeWeight(crossing.get(pair)?.weight ?? 0))
        crossing.set(pair, edge);
      continue;
    }
    const edges = edgesByGroup.get(id) ?? [];
    edges.push(edge);
    edgesByGroup.set(id, edges);
  }
  // Canonical order keeps the floating-point sums independent of arrival.
  const crossLinks = [...crossing]
    .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
    .map(([, edge]) => edge);
  const affinity = new Float64Array(ids.length * ids.length);
  for (const edge of crossLinks) {
    const a = slotOf.get(groupByKey.get(edge.from)!)!;
    const b = slotOf.get(groupByKey.get(edge.to)!)!;
    const weight = clampEdgeWeight(edge.weight);
    affinity[a * ids.length + b] += weight;
    affinity[b * ids.length + a] += weight;
  }
  const radii = ids.map((id) => 24 * Math.sqrt(groups.get(id)!.length));
  const centres = packRelatedCircles(radii, affinity, 160);
  // A title related to other neighbourhoods leans toward the side of its own
  // spiral that faces them, so cross-neighbourhood relations stay short.
  const lean = new Map<string, { x: number; y: number; weight: number }>();
  for (const edge of crossLinks) {
    const weight = clampEdgeWeight(edge.weight);
    for (const [key, otherKey] of [
      [edge.from, edge.to],
      [edge.to, edge.from],
    ]) {
      const own = centres[slotOf.get(groupByKey.get(key)!)!];
      const other = centres[slotOf.get(groupByKey.get(otherKey)!)!];
      const length = Math.hypot(other.x - own.x, other.y - own.y);
      if (length < 1e-9) continue;
      const entry = lean.get(key) ?? { x: 0, y: 0, weight: 0 };
      entry.x += ((other.x - own.x) / length) * weight;
      entry.y += ((other.y - own.y) / length) * weight;
      entry.weight += weight;
      lean.set(key, entry);
    }
  }
  const positions = new Map<string, { x: number; y: number }>();
  const hulls = new Map<number, LayoutPoint[]>();
  const dots = new Map<number, LayoutPoint[]>();
  const regions: GraphDetailRegion[] = [];
  ids.forEach((id, index) => {
    const { x, y } = centres[index];
    const members = groups
      .get(id)!
      .slice()
      .sort((a, b) => a.subjectKey.localeCompare(b.subjectKey));
    positions.set(`cluster:${id}`, { x, y });
    const radius = 24 * Math.sqrt(members.length);
    regions.push({
      id,
      left: x - radius,
      right: x + radius,
      top: y - radius,
      bottom: y + radius,
    });
    const points = members.map((_node, rank) => {
      const angle = rank * Math.PI * (3 - Math.sqrt(5));
      const distance = 24 * Math.sqrt(rank + 0.5);
      return {
        x: x + Math.cos(angle) * distance,
        y: y + Math.sin(angle) * distance,
      };
    });
    const assignment = assignRelatedSpiralSlots(
      members.map((node) => node.subjectKey),
      edgesByGroup.get(id) ?? [],
      points,
      members.map((node) => {
        const entry = lean.get(node.subjectKey);
        if (!entry) return undefined;
        // Opposing directions cancel toward the centre; the pull is weaker
        // than the title's own neighbourhood ties.
        return {
          x: x + (entry.x / entry.weight) * radius,
          y: y + (entry.y / entry.weight) * radius,
          weight: Math.min(1, entry.weight * CROSS_LEAN),
        };
      }),
    );
    members.forEach((node, index) => {
      positions.set(node.subjectKey, points[assignment[index]]);
    });
    hulls.set(id, convexHull(points));
    // A bounded sample of the actual title positions, so overview dots and
    // the detail boundary occupy the same space through the crossfade.
    const count = Math.min(600, points.length);
    dots.set(
      id,
      Array.from(
        { length: count },
        (_, rank) => points[Math.floor((rank * points.length) / count)],
      ),
    );
  });
  const extent = Math.max(
    160,
    ...regions.flatMap((region) => [
      Math.abs(region.left),
      Math.abs(region.right),
      Math.abs(region.top),
      Math.abs(region.bottom),
    ]),
  );
  return { positions, regions, extent, hulls, dots };
}

/** Grow posters with their local spacing instead of canceling camera zoom. */
export function graphDetailPosterSize(
  baseSize: number,
  spacingPx: number,
): number {
  const magnification =
    Number.isFinite(spacingPx) && spacingPx > 0 ? spacingPx / 60 : 1;
  return Math.min(28, Math.max(12, baseSize * 1.5)) * magnification;
}

/** Smooth overlap between neighborhood and title rendering in both directions. */
export function graphDetailOpacity(spacingPx: number): number {
  if (!Number.isFinite(spacingPx)) return 0;
  const t = Math.max(0, Math.min(1, (spacingPx - 40) / 30));
  return t * t * (3 - 2 * t);
}

export type GraphDetailRegion = {
  id: number;
  left: number;
  right: number;
  top: number;
  bottom: number;
};

/** Bounds of the legend selection; clearing it fits the whole overview. */
export function graphNeighborhoodBounds(
  regions: readonly GraphDetailRegion[],
  selected: ReadonlySet<number>,
): Omit<GraphDetailRegion, "id"> | null {
  const included = regions.filter(
    (region) => selected.size === 0 || selected.has(region.id),
  );
  if (!included.length) return null;
  return {
    left: Math.min(...included.map((region) => region.left)),
    right: Math.max(...included.map((region) => region.right)),
    top: Math.min(...included.map((region) => region.top)),
    bottom: Math.max(...included.map((region) => region.bottom)),
  };
}

/** Reveal nearby neighbourhoods when titles have room; use separate thresholds
 * for entering and leaving detail so trackpad motion cannot flicker the view. */
export function graphDetailClusters(
  previous: ReadonlySet<number>,
  regions: readonly GraphDetailRegion[],
  spacingPx: number,
  width: number,
  height: number,
): ReadonlySet<number> {
  if (width <= 0 || height <= 0 || !Number.isFinite(spacingPx)) return previous;
  const next = new Set<number>();
  for (const region of regions) {
    const margin = previous.has(region.id) ? 160 : 80;
    const nearby =
      region.right >= -margin &&
      region.left <= width + margin &&
      region.bottom >= -margin &&
      region.top <= height + margin;
    if (nearby && spacingPx >= (previous.has(region.id) ? 24 : 32))
      next.add(region.id);
  }
  return next.size === previous.size &&
    [...next].every((id) => previous.has(id))
    ? previous
    : next;
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

/** The fewest titles a neighbourhood needs to be named and outlined. */
export const LIBRARY_GRAPH_MIN_NAMED_CLUSTER_SIZE = 3;

/**
 * The clusters drawn as neighbourhoods: those with a label and at least
 * `LIBRARY_GRAPH_MIN_NAMED_CLUSTER_SIZE` titles. The rest are only titles,
 * with no outline, name or spot of their own.
 */
export function libraryGraphNamedClusterIds(
  clusters: readonly LibraryGraphCluster[],
): Set<number> {
  return new Set(
    clusters
      .filter(
        (cluster) =>
          cluster.labelKey !== null &&
          cluster.size >= LIBRARY_GRAPH_MIN_NAMED_CLUSTER_SIZE,
      )
      .map((cluster) => cluster.id),
  );
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
  recommendation: "libraryGraph.edgeKind.recommendation",
  // A provider's "similar titles" list reads the same as a similar link.
  similar: "libraryGraph.edgeKind.semantic",
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

/** At most this many shared genres and themes explain one link. */
export const NEIGHBOR_REASON_TERM_LIMIT = 4;

/** Every title of the anime facet carries this genre, so it explains nothing. */
const UNINFORMATIVE_TERM_KEYS = new Set(["canonical:genre:anime"]);

const SOURCE_NAMES: Record<string, string> = {
  anilist: "AniList",
  anidb: "AniDB",
  imdb: "IMDb",
  kitsu: "Kitsu",
  mal: "MyAnimeList",
  simkl: "Simkl",
  tmdb: "TMDB",
  trakt: "Trakt",
  tvdb: "TVDB",
};

/** The relation kinds a reason's services line can speak for. */
export type NeighborReasonSourceKind = "recommendation" | "similar";

/** Why a More Like This result sits next to the title it was found from. */
export type NeighborReason = {
  /** The relation edge kinds the services line does not already state. */
  relations: string[];
  /**
   * What the services line says the services did: recommended it, or listed
   * it as similar. Null when no line names them: there are none, or the link
   * is of another kind, whose label already says what it is.
   */
  sourceKind: NeighborReasonSourceKind | null;
  /** The services the link came from, by display name. */
  sources: string[];
  /** Genre and theme names both titles carry, themes first. */
  shared: string[];
};

type ReasonItem = {
  sources?: readonly string[] | null;
  canonicalTags?: readonly CanonicalMediaTag[] | null;
};

/** A source id's display name; an unknown id shows as given. */
export function discoverySourceName(source: string): string {
  const id = source.trim().toLowerCase();
  return SOURCE_NAMES[id] ?? source.trim();
}

/**
 * Why `item` is a neighbour of a title carrying `anchorTags`: how it is
 * related, which services recommend it, and the genres and themes the two
 * share. Spoiler tags never explain a link.
 */
export function neighborReason(
  anchorTags: readonly CanonicalMediaTag[] | null | undefined,
  item: ReasonItem,
  kinds: readonly string[],
): NeighborReason {
  const anchorKeys = new Set(
    (anchorTags ?? []).filter((tag) => !tag.isSpoiler).map((tag) => tag.key),
  );
  const rank = (tag: CanonicalMediaTag) => (tag.category === "theme" ? 0 : 1);
  const shared = (item.canonicalTags ?? [])
    .filter(
      (tag) =>
        !tag.isSpoiler &&
        (tag.category === "theme" || tag.category === "genre") &&
        anchorKeys.has(tag.key) &&
        !UNINFORMATIVE_TERM_KEYS.has(tag.key),
    )
    .sort(
      (a, b) =>
        rank(a) - rank(b) ||
        (b.confidence ?? 0) - (a.confidence ?? 0) ||
        a.name.localeCompare(b.name),
    );
  const names: string[] = [];
  for (const tag of shared) {
    const name = tag.name.trim();
    if (name && !names.includes(name)) {
      names.push(name);
    }
    if (names.length >= NEIGHBOR_REASON_TERM_LIMIT) {
      break;
    }
  }
  const sources = Array.from(
    new Set(
      (item.sources ?? [])
        .filter((source) => source.trim() !== "")
        .map(discoverySourceName),
    ),
  );
  const relationOf = (kind: string) => kind.trim().split(":")[1] ?? "";
  const recommended = kinds.some(
    (kind) => relationOf(kind) === "recommendation",
  );
  const similar = kinds.some((kind) => relationOf(kind) === "similar");
  let sourceKind: NeighborReasonSourceKind | null = null;
  if (sources.length > 0) {
    sourceKind = recommended ? "recommendation" : similar ? "similar" : null;
  }
  return {
    // A plain recommendation is never a label of its own; a similar link is
    // one unless the services line already says similar.
    relations: kinds.filter((kind) => {
      const relation = relationOf(kind);
      return (
        relation !== "recommendation" &&
        !(relation === "similar" && sourceKind === "similar")
      );
    }),
    sourceKind,
    sources,
    shared: names,
  };
}

/** A neighbour reason as tooltip lines in the viewer's language. */
export function neighborReasonLines(
  reason: NeighborReason,
  t: Translate,
): string[] {
  const lines: string[] = [];
  const relations = libraryGraphEdgeKindLabels(reason.relations, t);
  if (relations.length > 0) {
    lines.push(relations.join(", "));
  }
  if (reason.sourceKind !== null && reason.sources.length > 0) {
    lines.push(
      t(
        reason.sourceKind === "similar"
          ? "libraryGraph.reason.similarOn"
          : "libraryGraph.reason.recommendedBy",
        { sources: reason.sources.join(", ") },
      ),
    );
  }
  if (reason.shared.length > 0) {
    lines.push(
      t("libraryGraph.reason.shares", { terms: reason.shared.join(", ") }),
    );
  }
  return lines;
}

/** The most named sections a focused view splits its recommendations into. */
export const RECOMMENDATION_SECTION_LIMIT = 5;

/** A run of recommendations in one neighbourhood or sharing a genre or theme. */
export type RecommendationSection = {
  /** The genre or theme key naming the section, null for the ones left over. */
  tagKey: string | null;
  /** The section's name, or null for the ones left over. */
  name: string | null;
  keys: string[];
};

/** A neighbourhood the server found among an anchor's recommendations. */
export type RecommendationGroup = {
  labelKey: string | null;
  label: string;
  /** Its recommendations' keys. */
  keys: readonly string[];
};

/**
 * Recommendations split by the server's neighbourhoods, in the server's
 * order. A neighbourhood needs a genre or theme and two of the given
 * recommendations to be named; one known only by the title it centres on is
 * not;
 * past `limit` named sections, and for recommendations no neighbourhood
 * holds, they form a last unnamed section. With fewer than two named
 * sections, everything is one unnamed section. Null when no neighbourhood
 * holds any of them.
 */
function groupedRecommendationSections(
  keys: readonly string[],
  groups: readonly RecommendationGroup[],
  limit: number,
): RecommendationSection[] | null {
  const present = new Set(keys);
  const claimed = new Set<string>();
  const sections: RecommendationSection[] = [];
  let covered = false;
  // Server neighbourhoods describe connectivity, so several can share one
  // category. Combine them before applying section size and count limits.
  const categories = new Map<string, RecommendationGroup>();
  const mergedGroups: RecommendationGroup[] = [];
  for (const group of groups) {
    const category = group.labelKey
      ? clusterLabelKeyCandidates(group.labelKey).at(-1)
      : null;
    const existing = category ? categories.get(category) : undefined;
    if (existing) {
      existing.keys = [...existing.keys, ...group.keys];
    } else {
      const merged = { ...group, keys: [...group.keys] };
      mergedGroups.push(merged);
      if (category) categories.set(category, merged);
    }
  }
  for (const group of mergedGroups) {
    const members = new Set(
      group.keys.filter((key) => present.has(key) && !claimed.has(key)),
    );
    if (members.size === 0) {
      continue;
    }
    covered = true;
    if (!group.labelKey || members.size < 2 || sections.length >= limit) {
      continue;
    }
    for (const key of members) {
      claimed.add(key);
    }
    const name = group.label.trim();
    sections.push({
      tagKey: group.labelKey,
      name: name || null,
      keys: keys.filter((key) => members.has(key)),
    });
  }
  if (!covered) {
    return null;
  }
  if (sections.length < 2) {
    return keys.length > 0
      ? [{ tagKey: null, name: null, keys: [...keys] }]
      : [];
  }
  const rest = keys.filter((key) => !claimed.has(key));
  if (rest.length > 0) {
    sections.push({ tagKey: null, name: null, keys: rest });
  }
  return sections;
}

/**
 * A named section's name in the viewer's language: a neighbourhood's genre or
 * theme the way the library graph names its clusters, else the name as given.
 * Null for the ones left over.
 */
export function recommendationSectionName(
  section: RecommendationSection,
  t: Translate,
): string | null {
  if (section.name === null) {
    return null;
  }
  if (section.tagKey) {
    for (const candidate of clusterLabelKeyCandidates(section.tagKey)) {
      const label = discoveryBrowseLabelTitle(candidate, t);
      if (label) {
        return label;
      }
    }
  }
  return section.name;
}

/**
 * Recommendations split into sections. When the server grouped them into
 * neighbourhoods, those decide the sections. Otherwise they are grouped by
 * the genre or theme most of them share. Each
 * round takes the tag that covers the most recommendations not yet grouped,
 * themes before genres on a tie. A tag carried by more than two thirds of
 * them, or by only one, does not tell them apart and is skipped. The
 * rest form a last unnamed section. Order within a section follows the
 * input. With fewer than two named sections, everything is one unnamed
 * section.
 */
export function recommendationSections(
  items: readonly {
    key: string;
    tags: readonly CanonicalMediaTag[] | null | undefined;
  }[],
  limit: number = RECOMMENDATION_SECTION_LIMIT,
  groups: readonly RecommendationGroup[] = [],
): RecommendationSection[] {
  const keys = items.map((item) => item.key);
  const grouped = groupedRecommendationSections(keys, groups, limit);
  if (grouped) {
    return grouped;
  }
  const tagsByKey = new Map<string, Map<string, CanonicalMediaTag>>();
  const carriers = new Map<string, Set<string>>();
  for (const item of items) {
    const tags = new Map<string, CanonicalMediaTag>();
    for (const tag of item.tags ?? []) {
      if (
        tag.isSpoiler ||
        (tag.category !== "theme" && tag.category !== "genre") ||
        UNINFORMATIVE_TERM_KEYS.has(tag.key) ||
        !tag.name.trim()
      ) {
        continue;
      }
      tags.set(tag.key, tag);
      const set = carriers.get(tag.key) ?? new Set<string>();
      set.add(item.key);
      carriers.set(tag.key, set);
    }
    tagsByKey.set(item.key, tags);
  }
  const tagFor = (tagKey: string) => {
    for (const tags of tagsByKey.values()) {
      const tag = tags.get(tagKey);
      if (tag) {
        return tag;
      }
    }
    return null;
  };
  const unassigned = new Set(keys);
  const sections: RecommendationSection[] = [];
  while (sections.length < limit && unassigned.size > 1) {
    let best: { tag: CanonicalMediaTag; cover: number } | null = null;
    for (const [tagKey, holders] of carriers) {
      // A tag most recommendations carry does not tell them apart.
      if (holders.size > (keys.length * 2) / 3) {
        continue;
      }
      let cover = 0;
      for (const key of holders) {
        if (unassigned.has(key)) {
          cover += 1;
        }
      }
      const tag = tagFor(tagKey);
      if (cover < 2 || !tag) {
        continue;
      }
      const better =
        !best ||
        cover > best.cover ||
        (cover === best.cover &&
          ((tag.category === "theme" && best.tag.category !== "theme") ||
            (tag.category === best.tag.category && tag.key < best.tag.key)));
      if (better) {
        best = { tag, cover };
      }
    }
    if (!best) {
      break;
    }
    const holders = carriers.get(best.tag.key) ?? new Set<string>();
    const members = keys.filter(
      (key) => unassigned.has(key) && holders.has(key),
    );
    for (const key of members) {
      unassigned.delete(key);
    }
    sections.push({
      tagKey: best.tag.key,
      name: best.tag.name.trim(),
      keys: members,
    });
  }
  if (sections.length < 2) {
    return keys.length > 0 ? [{ tagKey: null, name: null, keys }] : [];
  }
  const rest = keys.filter((key) => unassigned.has(key));
  if (rest.length > 0) {
    sections.push({ tagKey: null, name: null, keys: rest });
  }
  return sections;
}

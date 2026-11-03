// Turns a library graph and the catalog's current titles into what the graph
// view draws: nodes joined to titles and pruned to the filtered set, node
// sizes from a chosen metric, collapsed clusters above a size limit, and the
// localized names of clusters and edge kinds.

import type { CanonicalMediaTag } from "../types/canonical-tags.ts";
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
): GraphState {
  return collapseClusters(
    graph,
    clustersToCollapse(owned, threshold, expandedClusterIds),
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
  recommendation: "libraryGraph.edgeKind.recommendation",
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

/** Why a More Like This result sits next to the title it was found from. */
export type NeighborReason = {
  /** The relation edge kinds, other than a plain recommendation. */
  relations: string[];
  /** The services that recommend it, by display name. */
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
  return {
    relations: kinds.filter(
      (kind) => kind.trim().split(":")[1] !== "recommendation",
    ),
    sources: Array.from(
      new Set(
        (item.sources ?? [])
          .filter((source) => source.trim() !== "")
          .map(discoverySourceName),
      ),
    ),
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
  if (reason.sources.length > 0) {
    lines.push(
      t("libraryGraph.reason.recommendedBy", {
        sources: reason.sources.join(", "),
      }),
    );
  }
  if (reason.shared.length > 0) {
    lines.push(
      t("libraryGraph.reason.shares", { terms: reason.shared.join(", ") }),
    );
  }
  return lines;
}

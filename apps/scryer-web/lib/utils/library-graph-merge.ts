// Pure operations on a library graph held in the client: folding a second
// result into it, pruning it to what is visible, collapsing clusters into one
// node each, and evicting the least recently used extra nodes.

import {
  UNPLACED_CLUSTER_ID,
  type LibraryGraphCluster,
  type LibraryGraphEdge,
  type LibraryGraphNode,
  type LibraryGraphResult,
} from "../types/library-graph.ts";

export type GraphNode = LibraryGraphNode & {
  /** True for a node that came from a later result and is not in the library. */
  ghost: boolean;
  /** Set when this node stands in for a whole collapsed cluster. */
  collapsedClusterId?: number;
  /** For a collapsed cluster node, how many nodes it stands in for. */
  memberCount?: number;
};

export type GraphState = {
  nodes: GraphNode[];
  edges: LibraryGraphEdge[];
  clusters: LibraryGraphCluster[];
};

/** The key two edges share when they join the same two nodes, in either direction. */
export function edgePairKey(a: string, b: string): string {
  return a < b ? `${a}\u0000${b}` : `${b}\u0000${a}`;
}

/** The subject key of the node that stands in for a collapsed cluster. */
export function clusterNodeKey(clusterId: number): string {
  return `cluster:${clusterId}`;
}

/** A graph state holding one result, every node owned. */
export function graphStateFromResult(result: LibraryGraphResult): GraphState {
  return mergeGraphStates(
    { nodes: [], edges: [], clusters: [] },
    result.nodes.map((node) => ({ ...node, ghost: false })),
    result.edges,
    result.clusters,
  );
}

/** An edge weight as a finite number in [0, 1]; anything else counts as 0. */
export function clampEdgeWeight(weight: number): number {
  return Number.isFinite(weight) ? Math.min(1, Math.max(0, weight)) : 0;
}

function sortedUnion(a: readonly string[], b: readonly string[]): string[] {
  return Array.from(new Set([...a, ...b])).sort();
}

function dedupeEdges(
  edges: Iterable<LibraryGraphEdge>,
  nodeKeys: ReadonlySet<string>,
): LibraryGraphEdge[] {
  const byPair = new Map<string, LibraryGraphEdge>();
  for (const edge of edges) {
    if (edge.from === edge.to) {
      continue;
    }
    if (!nodeKeys.has(edge.from) || !nodeKeys.has(edge.to)) {
      continue;
    }
    const key = edgePairKey(edge.from, edge.to);
    const existing = byPair.get(key);
    const weight = clampEdgeWeight(edge.weight);
    if (!existing) {
      byPair.set(key, {
        from: edge.from,
        to: edge.to,
        weight,
        kinds: sortedUnion([], edge.kinds),
      });
      continue;
    }
    existing.weight = Math.max(existing.weight, weight);
    existing.kinds = sortedUnion(existing.kinds, edge.kinds);
  }
  return Array.from(byPair.values());
}

function withClusterSizes(
  clusters: readonly LibraryGraphCluster[],
  nodes: readonly GraphNode[],
): LibraryGraphCluster[] {
  const sizes = new Map<number, number>();
  for (const node of nodes) {
    sizes.set(node.clusterId, (sizes.get(node.clusterId) ?? 0) + 1);
  }
  return clusters
    .filter((cluster) => (sizes.get(cluster.id) ?? 0) > 0)
    .map((cluster) => ({ ...cluster, size: sizes.get(cluster.id) ?? 0 }));
}

function mergeGraphStates(
  base: GraphState,
  incomingNodes: readonly GraphNode[],
  incomingEdges: readonly LibraryGraphEdge[],
  incomingClusters: readonly LibraryGraphCluster[],
): GraphState {
  const nodes = new Map<string, GraphNode>();
  for (const node of base.nodes) {
    nodes.set(node.subjectKey, node);
  }
  for (const node of incomingNodes) {
    if (!nodes.has(node.subjectKey)) {
      nodes.set(node.subjectKey, node);
    }
  }
  const clusters = new Map<number, LibraryGraphCluster>();
  for (const cluster of [...base.clusters, ...incomingClusters]) {
    if (!clusters.has(cluster.id)) {
      clusters.set(cluster.id, cluster);
    }
  }
  const nodeList = Array.from(nodes.values());
  return {
    nodes: nodeList,
    edges: dedupeEdges(
      [...base.edges, ...incomingEdges],
      new Set(nodes.keys()),
    ),
    clusters: withClusterSizes(Array.from(clusters.values()), nodeList),
  };
}

/**
 * Folds a later result (titles reached by traversing from the library) into
 * the graph. A node whose subject or target is already in the graph snaps to
 * that node, an owned one first, and its edges follow it; every other node
 * joins as a ghost in the cluster of its strongest owned neighbour, or
 * unplaced. Edges joining the same pair merge: their kinds are united and the
 * heavier weight is kept. The later result's clusters are not adopted.
 */
export function mergeLibraryGraphResult(
  base: GraphState,
  incoming: LibraryGraphResult,
): GraphState {
  const bySubject = new Map(base.nodes.map((node) => [node.subjectKey, node]));
  const byTarget = new Map<string, GraphNode>();
  for (const node of base.nodes) {
    if (!node.targetKey.trim()) {
      continue;
    }
    const current = byTarget.get(node.targetKey);
    if (!current || (current.ghost && !node.ghost)) {
      byTarget.set(node.targetKey, node);
    }
  }

  const snapTo = new Map<string, string>();
  const ghosts: GraphNode[] = [];
  for (const node of incoming.nodes) {
    // A blank target names no title, so it never joins two nodes.
    const hasTarget = node.targetKey.trim() !== "";
    const existing =
      bySubject.get(node.subjectKey) ??
      (hasTarget ? byTarget.get(node.targetKey) : undefined);
    if (existing) {
      snapTo.set(node.subjectKey, existing.subjectKey);
      continue;
    }
    if (snapTo.has(node.subjectKey)) {
      continue;
    }
    snapTo.set(node.subjectKey, node.subjectKey);
    const ghost: GraphNode = {
      subjectKey: node.subjectKey,
      targetKey: node.targetKey,
      clusterId: UNPLACED_CLUSTER_ID,
      ghost: true,
    };
    ghosts.push(ghost);
    // A later node naming the same new title snaps to this one.
    if (hasTarget) {
      byTarget.set(node.targetKey, ghost);
    }
  }

  const edges = incoming.edges.map((edge) => ({
    ...edge,
    weight: clampEdgeWeight(edge.weight),
    from: snapTo.get(edge.from) ?? edge.from,
    to: snapTo.get(edge.to) ?? edge.to,
  }));

  // A ghost settles in the cluster of the owned node it is most strongly
  // linked to; ties go to the lower subject key so the result is stable.
  for (const ghost of ghosts) {
    let best: { weight: number; key: string; clusterId: number } | null = null;
    for (const edge of edges) {
      const other =
        edge.from === ghost.subjectKey
          ? edge.to
          : edge.to === ghost.subjectKey
            ? edge.from
            : null;
      const neighbour = other ? bySubject.get(other) : undefined;
      if (!neighbour || neighbour.ghost) {
        continue;
      }
      if (
        !best ||
        edge.weight > best.weight ||
        (edge.weight === best.weight && neighbour.subjectKey < best.key)
      ) {
        best = {
          weight: edge.weight,
          key: neighbour.subjectKey,
          clusterId: neighbour.clusterId,
        };
      }
    }
    if (best) {
      ghost.clusterId = best.clusterId;
    }
  }

  return mergeGraphStates(base, ghosts, edges, []);
}

/** The graph limited to the given nodes and the edges between them. */
export function pruneToVisible(
  graph: GraphState,
  visibleSubjectKeys: ReadonlySet<string>,
): GraphState {
  const nodes = graph.nodes.filter((node) =>
    visibleSubjectKeys.has(node.subjectKey),
  );
  return {
    nodes,
    edges: graph.edges.filter(
      (edge) =>
        visibleSubjectKeys.has(edge.from) && visibleSubjectKeys.has(edge.to),
    ),
    clusters: withClusterSizes(graph.clusters, nodes),
  };
}

/**
 * The clusters to collapse: none while the graph has no more nodes than the
 * threshold, otherwise every cluster (unplaced nodes count as one) the viewer
 * has not expanded.
 */
export function clustersToCollapse(
  graph: GraphState,
  threshold: number,
  expandedClusterIds: ReadonlySet<number>,
): Set<number> {
  if (graph.nodes.length <= threshold) {
    return new Set();
  }
  const ids = new Set<number>();
  for (const node of graph.nodes) {
    if (!expandedClusterIds.has(node.clusterId)) {
      ids.add(node.clusterId);
    }
  }
  return ids;
}

/** The expanded cluster set with one cluster toggled. */
export function toggleClusterExpanded(
  expandedClusterIds: ReadonlySet<number>,
  clusterId: number,
): Set<number> {
  const next = new Set(expandedClusterIds);
  if (next.has(clusterId)) {
    next.delete(clusterId);
  } else {
    next.add(clusterId);
  }
  return next;
}

/**
 * Replaces each collapsed cluster's nodes with one node standing in for the
 * cluster. Edges inside a collapsed cluster drop out; edges leaving it are
 * redirected to the cluster node and merge with others joining the same pair.
 */
export function collapseClusters(
  graph: GraphState,
  collapsedClusterIds: ReadonlySet<number>,
): GraphState {
  if (collapsedClusterIds.size === 0) {
    return graph;
  }
  const index = collapseIndex(graph);
  const collapsed = new Set<number>();
  const nodes: GraphNode[] = [];
  for (const node of graph.nodes) {
    if (!collapsedClusterIds.has(node.clusterId)) {
      nodes.push(node);
      continue;
    }
    if (collapsed.has(node.clusterId)) continue;
    collapsed.add(node.clusterId);
    nodes.push({
      subjectKey: clusterNodeKey(node.clusterId),
      targetKey: index.standTargets.get(node.clusterId) ?? "",
      clusterId: node.clusterId,
      ghost: false,
      collapsedClusterId: node.clusterId,
      memberCount: index.memberCounts.get(node.clusterId) ?? 0,
    });
  }
  const edges: LibraryGraphEdge[] = [];
  for (const { edge, a, b } of index.direct) {
    if (!collapsedClusterIds.has(a) && !collapsedClusterIds.has(b)) {
      edges.push(edge);
    }
  }
  for (const { edge, a, b } of index.toNode) {
    if (collapsedClusterIds.has(a) && !collapsedClusterIds.has(b)) {
      edges.push(edge);
    }
  }
  for (const { edge, a, b } of index.between) {
    if (collapsedClusterIds.has(a) && collapsedClusterIds.has(b)) {
      edges.push(edge);
    }
  }
  return { nodes, edges, clusters: graph.clusters };
}

type IndexedEdge = { edge: LibraryGraphEdge; a: number; b: number };

/**
 * Every edge a collapse can draw, merged once per graph. A fold then only
 * picks among them: `direct` edges join two titles (clusters `a` and `b`),
 * `toNode` edges join cluster `a`'s node to a title in cluster `b`, and
 * `between` edges join the nodes of clusters `a` and `b`. Merging keeps the
 * strongest weight and every kind, so the pick matches collapsing afresh.
 */
type CollapseIndex = {
  standTargets: Map<number, string>;
  memberCounts: Map<number, number>;
  direct: IndexedEdge[];
  toNode: IndexedEdge[];
  between: IndexedEdge[];
};

const collapseIndexes = new WeakMap<GraphState, CollapseIndex>();

function collapseIndex(graph: GraphState): CollapseIndex {
  const cached = collapseIndexes.get(graph);
  if (cached) return cached;
  const clusterOf = new Map<string, number>();
  const targetBySubject = new Map<string, string>();
  const memberCounts = new Map<number, number>();
  for (const node of graph.nodes) {
    const key = node.subjectKey;
    if (!targetBySubject.has(key)) targetBySubject.set(key, node.targetKey);
    if (!clusterOf.has(key)) clusterOf.set(key, node.clusterId);
    // A collapsed cluster counts the library's titles only; explored ones
    // folded into it are not members.
    memberCounts.set(
      node.clusterId,
      (memberCounts.get(node.clusterId) ?? 0) + (node.ghost ? 0 : 1),
    );
  }
  const standTargets = new Map<number, string>();
  for (const cluster of graph.clusters) {
    standTargets.set(cluster.id, targetBySubject.get(cluster.medoid) ?? "");
  }
  const direct = new Map<string, IndexedEdge>();
  const toNode = new Map<string, IndexedEdge>();
  const between = new Map<string, IndexedEdge>();
  const merge = (
    into: Map<string, IndexedEdge>,
    from: string,
    to: string,
    a: number,
    b: number,
    source: LibraryGraphEdge,
  ) => {
    const key = edgePairKey(from, to);
    const weight = clampEdgeWeight(source.weight);
    const existing = into.get(key);
    if (!existing) {
      into.set(key, {
        edge: { from, to, weight, kinds: sortedUnion([], source.kinds) },
        a,
        b,
      });
      return;
    }
    existing.edge.weight = Math.max(existing.edge.weight, weight);
    existing.edge.kinds = sortedUnion(existing.edge.kinds, source.kinds);
  };
  for (const edge of graph.edges) {
    if (edge.from === edge.to) continue;
    const a = clusterOf.get(edge.from);
    const b = clusterOf.get(edge.to);
    if (a === undefined || b === undefined) continue;
    merge(direct, edge.from, edge.to, a, b, edge);
    if (a === b) continue;
    const fromCluster = clusterNodeKey(a);
    const toCluster = clusterNodeKey(b);
    merge(toNode, fromCluster, edge.to, a, b, edge);
    merge(toNode, toCluster, edge.from, b, a, edge);
    merge(between, fromCluster, toCluster, a, b, edge);
  }
  const index: CollapseIndex = {
    standTargets,
    memberCounts,
    direct: Array.from(direct.values()),
    toNode: Array.from(toNode.values()),
    between: Array.from(between.values()),
  };
  collapseIndexes.set(graph, index);
  return index;
}

/**
 * Evicts ghost nodes, least recently used first, until the graph holds no more
 * than `cap` nodes. Owned nodes and nodes the caller pins are never evicted,
 * so a graph whose kept nodes alone exceed the cap stays above it. A node
 * never used counts as oldest; ties go to the lower subject key.
 */
export function pruneLeastRecentlyUsed(
  graph: GraphState,
  cap: number,
  lastUsed: ReadonlyMap<string, number>,
  isPinned: (node: GraphNode) => boolean = () => false,
): { graph: GraphState; evicted: string[] } {
  const excess = graph.nodes.length - Math.max(0, cap);
  if (excess <= 0) {
    return { graph, evicted: [] };
  }
  const candidates = graph.nodes
    .filter((node) => node.ghost && !isPinned(node))
    .sort((a, b) => {
      const usedA = lastUsed.get(a.subjectKey) ?? Number.NEGATIVE_INFINITY;
      const usedB = lastUsed.get(b.subjectKey) ?? Number.NEGATIVE_INFINITY;
      if (usedA !== usedB) {
        return usedA < usedB ? -1 : 1;
      }
      return a.subjectKey < b.subjectKey ? -1 : 1;
    });
  const evicted = candidates.slice(0, excess).map((node) => node.subjectKey);
  const evictedSet = new Set(evicted);
  const kept = new Set(
    graph.nodes
      .filter((node) => !evictedSet.has(node.subjectKey))
      .map((node) => node.subjectKey),
  );
  return { graph: pruneToVisible(graph, kept), evicted };
}

/**
 * One node and everything one link from it, with the links among them: those
 * that leave that node and those between its neighbours. Null when the node is
 * not in the graph.
 */
export function egoGraphState(
  graph: GraphState,
  center: string,
): GraphState | null {
  if (!graph.nodes.some((node) => node.subjectKey === center)) {
    return null;
  }
  const keep = new Set<string>([center]);
  for (const edge of graph.edges) {
    if (edge.from === center || edge.to === center) {
      keep.add(edge.from);
      keep.add(edge.to);
    }
  }
  const nodes = graph.nodes.filter((node) => keep.has(node.subjectKey));
  return {
    nodes,
    edges: graph.edges.filter(
      (edge) => keep.has(edge.from) && keep.has(edge.to),
    ),
    clusters: withClusterSizes(graph.clusters, nodes),
  };
}

/**
 * The neighbours of `center` in rings: library titles first, then explored
 * titles, each heaviest link first and then by subject key. Empty rings are
 * left out.
 */
export function egoRingGroups(ego: GraphState, center: string): string[][] {
  const weights = new Map<string, number>();
  for (const edge of ego.edges) {
    const other =
      edge.from === center ? edge.to : edge.to === center ? edge.from : null;
    if (other && other !== center) {
      weights.set(other, Math.max(weights.get(other) ?? 0, edge.weight));
    }
  }
  const order = (a: GraphNode, b: GraphNode) => {
    const byWeight =
      (weights.get(b.subjectKey) ?? 0) - (weights.get(a.subjectKey) ?? 0);
    if (byWeight !== 0) {
      return byWeight;
    }
    return a.subjectKey < b.subjectKey ? -1 : 1;
  };
  const neighbours = ego.nodes.filter((node) => weights.has(node.subjectKey));
  return [
    neighbours.filter((node) => !node.ghost).sort(order),
    neighbours.filter((node) => node.ghost).sort(order),
  ]
    .filter((ring) => ring.length > 0)
    .map((ring) => ring.map((node) => node.subjectKey));
}

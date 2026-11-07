// Exploring outward from the library graph: a title's More Like This results
// join the graph as ghost nodes linked to it, and a ghost can be explored in
// turn. The explored state is a trail of expansions held only for the
// session. Everything here is pure so the view can keep it in React state.

import {
  UNPLACED_CLUSTER_ID,
  type LibraryGraphEdge,
  type LibraryGraphNode,
} from "../types/library-graph.ts";
import {
  clampEdgeWeight,
  mergeLibraryGraphResult,
  type GraphState,
} from "./library-graph-merge.ts";
import type { LayoutPoint } from "./library-graph-layout.ts";
import { hashString } from "./library-graph-layout.ts";
import { targetKeyMatchKeys } from "./library-graph-model.ts";

/** At most this many ghost nodes stay live; older expansions are pruned. */
export const LIBRARY_GRAPH_GHOST_CAP = 200;

/** The edge weight of a result that carries no rank score. */
export const DEFAULT_TRAVERSAL_EDGE_WEIGHT = 0.5;

/** Subject keys of ghost nodes start with this, so they never meet a server key. */
export const GHOST_SUBJECT_KEY_PREFIX = "ghost:";

/** How far from its anchor a new ghost starts, in layout units. */
const GHOST_SEED_DISTANCE = 40;

/** What traversal needs of a discovery item. */
export type TraversalItem = {
  targetKey: string;
  resolvedTitleId?: string | null;
  relationTypes?: readonly string[] | null;
  relationSubtypes?: readonly string[] | null;
  rankScore?: number | null;
  /** The anchor's neighbourhood this result falls into, when the server grouped them. */
  groupId?: number | null;
  externalIds?: ReadonlyArray<{
    source?: string | null;
    kind?: string | null;
    id?: string | null;
    key?: string | null;
  }> | null;
};

/** A neighbourhood the server found among an anchor's results. */
export type TraversalGroupInput = {
  id: number;
  labelKey: string | null;
  label: string;
};

/** A neighbourhood of one expansion, with the subject keys of its results. */
export type TraversalGroup = TraversalGroupInput & {
  /** Owned or ghost subject keys, in result order. */
  keys: string[];
};

export type TraversalExpansion = {
  /** The subject key of the node that was expanded. */
  anchor: string;
  /** Edges from the anchor to each result, owned or ghost. */
  edges: LibraryGraphEdge[];
  /** The ghost subject keys this expansion reached. */
  ghostKeys: string[];
  /**
   * The server's neighbourhoods among the results, in its order, each with at
   * least one result; empty when the results were not grouped.
   */
  groups: TraversalGroup[];
  /** When it was last expanded, on the traversal's own clock. */
  expandedAt: number;
};

export type TraversalState<T extends TraversalItem> = {
  /** The breadcrumb trail, oldest first. */
  expansions: TraversalExpansion[];
  /** The discovery item behind each live ghost. */
  ghostItems: ReadonlyMap<string, T>;
  clock: number;
};

export function emptyTraversal<T extends TraversalItem>(): TraversalState<T> {
  return { expansions: [], ghostItems: new Map(), clock: 0 };
}

export function isGhostSubjectKey(subjectKey: string): boolean {
  return subjectKey.startsWith(GHOST_SUBJECT_KEY_PREFIX);
}

function kindPart(value: string): string {
  return value.trim().toLowerCase().replace(/\s+/g, "_");
}

function distinctParts(values: readonly string[] | null | undefined): string[] {
  return Array.from(
    new Set((values ?? []).map(kindPart).filter((part) => part !== "")),
  );
}

/**
 * The edge kinds a More Like This result links by, straight from its relation
 * types: `relation:<type>`, or `relation:<type>:<subtype>` for each subtype
 * when the item has one relation type and subtypes. Subtypes cannot be paired
 * with several types, so those keep the bare type. A result with no relation
 * type is a plain recommendation.
 */
export function moreLikeThisEdgeKinds(item: TraversalItem): string[] {
  const types = distinctParts(item.relationTypes);
  const subtypes = distinctParts(item.relationSubtypes);
  if (types.length === 0) {
    return ["relation:recommendation"];
  }
  if (types.length === 1 && subtypes.length > 0) {
    return subtypes.map((subtype) => `relation:${types[0]}:${subtype}`).sort();
  }
  return types.map((type) => `relation:${type}`).sort();
}

/**
 * Edge weights for a batch of results: the rank score where one is given,
 * scaled by the batch's highest when scores run above 1 so they stay in
 * [0, 1], and the default weight where none is.
 */
export function moreLikeThisEdgeWeights(
  items: readonly TraversalItem[],
): number[] {
  const scores = items.map((item) =>
    typeof item.rankScore === "number" &&
    Number.isFinite(item.rankScore) &&
    item.rankScore >= 0
      ? item.rankScore
      : null,
  );
  const max = scores.reduce<number>(
    (highest, score) => (score === null ? highest : Math.max(highest, score)),
    0,
  );
  return scores.map((score) => {
    if (score === null) {
      return DEFAULT_TRAVERSAL_EDGE_WEIGHT;
    }
    return clampEdgeWeight(max > 1 ? score / max : score);
  });
}

/**
 * Every target key an item can be matched by, normalized: its own target key
 * and one per external id that names a kind.
 */
export function itemMatchKeys(item: TraversalItem): string[] {
  const keys = new Set<string>(targetKeyMatchKeys(item.targetKey));
  for (const externalId of item.externalIds ?? []) {
    const candidates = [
      externalId.key ?? "",
      externalId.source && externalId.kind && externalId.id
        ? `${externalId.source}:${externalId.kind}:${externalId.id}`
        : "",
    ];
    for (const candidate of candidates) {
      for (const key of targetKeyMatchKeys(candidate)) {
        keys.add(key);
      }
    }
  }
  return Array.from(keys);
}

export type OwnedNodeEntry = {
  subjectKey: string;
  targetKey: string;
  /** The catalog title the node stands for, when it is joined to one. */
  titleId?: string | null;
  /** Extra keys the title is known by, such as its other sources' ids. */
  matchKeys?: readonly string[];
};

export type OwnedNodeIndex = {
  byMatchKey: ReadonlyMap<string, string>;
  byTitleId: ReadonlyMap<string, string>;
};

/** A lookup from every key an owned node is known by to its subject key. */
export function ownedNodeIndex(
  entries: Iterable<OwnedNodeEntry>,
): OwnedNodeIndex {
  const byMatchKey = new Map<string, string>();
  const byTitleId = new Map<string, string>();
  for (const entry of entries) {
    for (const key of [
      ...targetKeyMatchKeys(entry.targetKey),
      ...(entry.matchKeys ?? []).flatMap(targetKeyMatchKeys),
    ]) {
      if (!byMatchKey.has(key)) {
        byMatchKey.set(key, entry.subjectKey);
      }
    }
    const titleId = entry.titleId?.trim();
    if (titleId && !byTitleId.has(titleId)) {
      byTitleId.set(titleId, entry.subjectKey);
    }
  }
  return { byMatchKey, byTitleId };
}

/** The owned node an item is already in the graph as, or null. */
export function ownedSubjectForItem(
  item: TraversalItem,
  owned: OwnedNodeIndex,
): string | null {
  const titleId = item.resolvedTitleId?.trim();
  if (titleId) {
    const subject = owned.byTitleId.get(titleId);
    if (subject) {
      return subject;
    }
  }
  for (const key of itemMatchKeys(item)) {
    const subject = owned.byMatchKey.get(key);
    if (subject) {
      return subject;
    }
  }
  return null;
}

/** The ghost subject key for an item not in the library, or null when it names nothing. */
export function ghostSubjectKey(item: TraversalItem): string | null {
  const [canonical] = targetKeyMatchKeys(item.targetKey);
  const key = canonical ?? item.targetKey.trim().toLowerCase();
  return key ? `${GHOST_SUBJECT_KEY_PREFIX}${key}` : null;
}

/**
 * The ghosts still live: those an expansion reached, and those that were
 * expanded themselves.
 */
export function liveGhostKeys(
  expansions: readonly TraversalExpansion[],
): Set<string> {
  const live = new Set<string>();
  for (const expansion of expansions) {
    for (const key of expansion.ghostKeys) {
      live.add(key);
    }
    if (isGhostSubjectKey(expansion.anchor)) {
      live.add(expansion.anchor);
    }
  }
  return live;
}

function withLiveItems<T extends TraversalItem>(
  state: TraversalState<T>,
  expansions: TraversalExpansion[],
): TraversalState<T> {
  const live = liveGhostKeys(expansions);
  const ghostItems = new Map<string, T>();
  for (const [key, item] of state.ghostItems) {
    if (live.has(key)) {
      ghostItems.set(key, item);
    }
  }
  return { expansions, ghostItems, clock: state.clock };
}

/**
 * Drops the least recently expanded expansions, other than `keepAnchor`'s,
 * until no more than `cap` ghosts are live. A dropped expansion's ghosts go
 * unless another live expansion reached them or they were expanded too.
 */
export function pruneTraversal<T extends TraversalItem>(
  state: TraversalState<T>,
  cap: number = LIBRARY_GRAPH_GHOST_CAP,
  keepAnchor: string | null = null,
): TraversalState<T> {
  let expansions = state.expansions;
  while (liveGhostKeys(expansions).size > Math.max(0, cap)) {
    let oldest: TraversalExpansion | null = null;
    for (const expansion of expansions) {
      if (expansion.anchor === keepAnchor) {
        continue;
      }
      if (!oldest || expansion.expandedAt < oldest.expandedAt) {
        oldest = expansion;
      }
    }
    if (!oldest) {
      break;
    }
    const dropped = oldest;
    expansions = expansions.filter((expansion) => expansion !== dropped);
  }
  return expansions === state.expansions
    ? state
    : withLiveItems(state, expansions);
}

/**
 * Adds one node's More Like This results to the explored state. Results that
 * are owned titles link to the owned node; the rest become ghosts, one per
 * title however many results or expansions name it. Expanding a node again
 * replaces its results in place on the trail. Afterwards the state is pruned
 * to the ghost cap, never dropping this expansion.
 */
export function expandTraversal<T extends TraversalItem>(
  state: TraversalState<T>,
  anchor: string,
  items: readonly T[],
  owned: OwnedNodeIndex,
  cap: number = LIBRARY_GRAPH_GHOST_CAP,
  groups: readonly TraversalGroupInput[] = [],
): TraversalState<T> {
  const weights = moreLikeThisEdgeWeights(items);
  const groupKeys = new Map<number, string[]>(
    groups.map((group) => [group.id, []]),
  );
  const grouped = new Set<string>();
  const edgesByTarget = new Map<string, LibraryGraphEdge>();
  const ghostItems = new Map(state.ghostItems);
  const ghostKeys: string[] = [];
  items.forEach((item, index) => {
    const ownedSubject = ownedSubjectForItem(item, owned);
    const target = ownedSubject ?? ghostSubjectKey(item);
    if (!target || target === anchor) {
      return;
    }
    if (!ownedSubject && !edgesByTarget.has(target)) {
      ghostKeys.push(target);
      ghostItems.set(target, item);
    }
    const members =
      item.groupId === null || item.groupId === undefined
        ? undefined
        : groupKeys.get(item.groupId);
    // A title several results name stays in the first one's group.
    if (members && !grouped.has(target)) {
      grouped.add(target);
      members.push(target);
    }
    const kinds = moreLikeThisEdgeKinds(item);
    const existing = edgesByTarget.get(target);
    if (existing) {
      existing.weight = Math.max(existing.weight, weights[index]);
      existing.kinds = Array.from(
        new Set([...existing.kinds, ...kinds]),
      ).sort();
      return;
    }
    edgesByTarget.set(target, {
      from: anchor,
      to: target,
      weight: weights[index],
      kinds,
    });
  });
  const clock = state.clock + 1;
  const expansion: TraversalExpansion = {
    anchor,
    edges: Array.from(edgesByTarget.values()),
    ghostKeys,
    groups: groups
      .map((group) => ({
        id: group.id,
        labelKey: group.labelKey,
        label: group.label,
        keys: groupKeys.get(group.id) ?? [],
      }))
      .filter((group) => group.keys.length > 0),
    expandedAt: clock,
  };
  const position = state.expansions.findIndex(
    (entry) => entry.anchor === anchor,
  );
  const expansions =
    position < 0
      ? [...state.expansions, expansion]
      : state.expansions.map((entry, index) =>
          index === position ? expansion : entry,
        );
  return pruneTraversal(
    withLiveItems({ expansions, ghostItems, clock }, expansions),
    cap,
    anchor,
  );
}

/** The explored state cut back to the trail up to and including `index`. */
export function collapseTraversalToCrumb<T extends TraversalItem>(
  state: TraversalState<T>,
  index: number,
): TraversalState<T> {
  if (index >= state.expansions.length - 1) {
    return state;
  }
  return withLiveItems(
    state,
    state.expansions.slice(0, Math.max(0, index + 1)),
  );
}

/**
 * The owned graph with the explored ghosts and links folded in. Ghosts settle
 * in the cluster of their strongest owned neighbour, else of their strongest
 * placed ghost neighbour, else stay unplaced. Links whose anchor is not in the
 * graph (filtered out) are left out.
 */
export function traversalGraphState<T extends TraversalItem>(
  base: GraphState,
  state: TraversalState<T>,
): GraphState {
  if (state.expansions.length === 0) {
    return base;
  }
  const nodes: LibraryGraphNode[] = [];
  for (const [subjectKey, item] of state.ghostItems) {
    nodes.push({
      subjectKey,
      targetKey: item.targetKey,
      clusterId: UNPLACED_CLUSTER_ID,
    });
  }
  const merged = mergeLibraryGraphResult(base, {
    generatedAt: "",
    nodes,
    edges: state.expansions.flatMap((expansion) => expansion.edges),
    clusters: [],
  });
  // Ghosts reached only through other ghosts take their cluster, nearest
  // placed ghost first. Only links touching a ghost (the explored ones, after
  // snapping) can place one, so the library's own links are not rescanned.
  const byKey = new Map(merged.nodes.map((node) => [node.subjectKey, node]));
  const ghostLinks = new Map<string, { other: string; weight: number }[]>();
  const link = (from: string, to: string, weight: number) => {
    if (!byKey.get(from)?.ghost) {
      return;
    }
    const links = ghostLinks.get(from);
    if (links) {
      links.push({ other: to, weight });
    } else {
      ghostLinks.set(from, [{ other: to, weight }]);
    }
  };
  for (const edge of merged.edges) {
    link(edge.from, edge.to, edge.weight);
    link(edge.to, edge.from, edge.weight);
  }
  for (let pass = 0; pass < state.expansions.length; pass += 1) {
    let changed = false;
    for (const [subjectKey, links] of ghostLinks) {
      const node = byKey.get(subjectKey);
      if (!node || node.clusterId !== UNPLACED_CLUSTER_ID) {
        continue;
      }
      let best: { weight: number; clusterId: number } | null = null;
      for (const { other, weight } of links) {
        const neighbour = byKey.get(other);
        if (!neighbour || neighbour.clusterId === UNPLACED_CLUSTER_ID) {
          continue;
        }
        if (!best || weight > best.weight) {
          best = { weight, clusterId: neighbour.clusterId };
        }
      }
      if (best) {
        node.clusterId = best.clusterId;
        changed = true;
      }
    }
    if (!changed) {
      break;
    }
  }
  return merged;
}

/**
 * Whether an explored title is one of the viewer's own titles, though it has
 * no node in the graph as drawn (added after the graph was built, cut from a
 * truncated graph, or filtered out). Scryer names the local title only for a
 * title in a library the viewer can read; otherwise the title counts when one
 * of its keys names a title in the catalog listing.
 */
export function isLibraryItem(
  item: TraversalItem,
  catalogMatchKeys: { has(key: string): boolean },
): boolean {
  if (item.resolvedTitleId?.trim()) {
    return true;
  }
  return itemMatchKeys(item).some((key) => catalogMatchKeys.has(key));
}

/**
 * The target key to ask for an explored title's neighbours by: the first of
 * its keys in the discovery vocabulary, or null when it has none.
 */
export function neighborTargetKey(item: TraversalItem): string | null {
  return itemMatchKeys(item)[0] ?? null;
}

/**
 * The explored state with one ghost marked as the local title it was just
 * added as. The trail, links and recency stay as they were.
 */
export function stampGhostTitle<T extends TraversalItem>(
  state: TraversalState<T>,
  ghostKey: string,
  titleId: string,
): TraversalState<T> {
  const item = state.ghostItems.get(ghostKey);
  if (!item || item.resolvedTitleId === titleId) {
    return state;
  }
  const ghostItems = new Map(state.ghostItems);
  ghostItems.set(ghostKey, { ...item, resolvedTitleId: titleId });
  return { ...state, ghostItems };
}

/**
 * Orders "More like this" replies against everything that starts the
 * exploration over. A reply is applied only while no later request has
 * started and the exploration has not been reset or reloaded since.
 */
export type ExpansionGuard = { current: number };

/** Starts a request; its token stays current until the next start or reset. */
export function beginExpansion(guard: ExpansionGuard): number {
  guard.current += 1;
  return guard.current;
}

/** Retires every request in flight, so none of their replies is applied. */
export function invalidateExpansions(guard: ExpansionGuard): void {
  guard.current += 1;
}

/** Whether a request's reply may still be applied. */
export function isCurrentExpansion(
  guard: ExpansionGuard,
  token: number,
): boolean {
  return guard.current === token;
}

/**
 * Layout starting points with each new ghost placed beside the node that
 * reached it, so adding ghosts settles the layout instead of restarting it.
 * Nodes that already have a position keep it.
 */
export function seedGhostPositions(
  previous: ReadonlyMap<string, LayoutPoint>,
  expansions: readonly TraversalExpansion[],
): Map<string, LayoutPoint> {
  const seeded = new Map(previous);
  for (const expansion of expansions) {
    const anchor = seeded.get(expansion.anchor);
    if (!anchor) {
      continue;
    }
    for (const key of expansion.ghostKeys) {
      if (seeded.has(key)) {
        continue;
      }
      const angle = ((hashString(key) % 3600) / 3600) * 2 * Math.PI;
      seeded.set(key, {
        x: anchor.x + Math.cos(angle) * GHOST_SEED_DISTANCE,
        y: anchor.y + Math.sin(angle) * GHOST_SEED_DISTANCE,
      });
    }
  }
  return seeded;
}

export type KeyboardNode = {
  key: string;
  clusterId: number;
  label: string;
};

function keyboardOrder(a: KeyboardNode, b: KeyboardNode): number {
  const byLabel = a.label.localeCompare(b.label);
  if (byLabel !== 0) {
    return byLabel;
  }
  return a.key < b.key ? -1 : a.key > b.key ? 1 : 0;
}

/** The first node of a cluster in keyboard order, or null when it has none. */
export function firstNodeInCluster(
  nodes: readonly KeyboardNode[],
  clusterId: number,
): string | null {
  const members = nodes
    .filter((node) => node.clusterId === clusterId)
    .sort(keyboardOrder);
  return members[0]?.key ?? null;
}

/**
 * The node `step` places on from `current` among the nodes of its cluster, in
 * label order and wrapping round. With no current node (or one no longer
 * drawn) it is the first node of the lowest placed cluster.
 */
export function nextNodeInCluster(
  nodes: readonly KeyboardNode[],
  current: string | null,
  step: number,
): string | null {
  const from = current ? nodes.find((node) => node.key === current) : undefined;
  if (!from) {
    const clusterIds = Array.from(
      new Set(nodes.map((node) => node.clusterId)),
    ).sort((a, b) =>
      a === UNPLACED_CLUSTER_ID ? 1 : b === UNPLACED_CLUSTER_ID ? -1 : a - b,
    );
    return clusterIds.length > 0
      ? firstNodeInCluster(nodes, clusterIds[0])
      : null;
  }
  const members = nodes
    .filter((node) => node.clusterId === from.clusterId)
    .sort(keyboardOrder);
  const index = members.findIndex((node) => node.key === from.key);
  const count = members.length;
  return members[(((index + step) % count) + count) % count].key;
}

/**
 * Groups of drawn nodes that no library cluster chip leads to, such as
 * explored titles that settled in no cluster, with how many nodes each holds.
 * Keyboard users reach them through a chip of their own.
 */
export function uncoveredKeyboardGroups(
  nodes: readonly KeyboardNode[],
  coveredClusterIds: ReadonlySet<number>,
): { id: number; count: number }[] {
  const counts = new Map<number, number>();
  for (const node of nodes) {
    if (!coveredClusterIds.has(node.clusterId)) {
      counts.set(node.clusterId, (counts.get(node.clusterId) ?? 0) + 1);
    }
  }
  return Array.from(counts, ([id, count]) => ({ id, count })).sort((a, b) =>
    a.id === UNPLACED_CLUSTER_ID
      ? 1
      : b.id === UNPLACED_CLUSTER_ID
        ? -1
        : a.id - b.id,
  );
}

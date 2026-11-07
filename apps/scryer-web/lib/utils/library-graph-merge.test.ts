import assert from "node:assert/strict";
import test from "node:test";

import type { LibraryGraphResult } from "../types/library-graph.ts";
import {
  clusterNodeKey,
  clustersToCollapse,
  collapseClusters,
  edgePairKey,
  egoGraphState,
  egoRingGroups,
  graphStateFromResult,
  mergeLibraryGraphResult,
  pruneLeastRecentlyUsed,
  pruneToVisible,
  toggleClusterExpanded,
  type GraphState,
} from "./library-graph-merge.ts";

function owned(): GraphState {
  return graphStateFromResult({
    generatedAt: "2026-01-01T00:00:00Z",
    nodes: [
      { subjectKey: "s:a", targetKey: "tmdb:movie:1", clusterId: 0 },
      { subjectKey: "s:b", targetKey: "tmdb:movie:2", clusterId: 0 },
      { subjectKey: "s:c", targetKey: "tmdb:movie:3", clusterId: 1 },
    ],
    edges: [
      { from: "s:a", to: "s:b", weight: 0.4, kinds: ["semantic"] },
      // The same pair again, reversed: one edge with both kinds and the
      // heavier weight.
      { from: "s:b", to: "s:a", weight: 0.7, kinds: ["relation:sequel"] },
      { from: "s:b", to: "s:c", weight: 0.3, kinds: ["relation:co_watch"] },
    ],
    clusters: [
      { id: 0, labelKey: "isekai", label: "Isekai", medoid: "s:a", size: 2 },
      { id: 1, labelKey: null, label: "Other", medoid: "s:c", size: 1 },
    ],
  });
}

function edgeBetween(graph: GraphState, a: string, b: string) {
  return graph.edges.find(
    (edge) => edgePairKey(edge.from, edge.to) === edgePairKey(a, b),
  );
}

test("collapsing thousands of medoids visits node identities linearly", () => {
  let reads = 0;
  const count = 10_000;
  const graph: GraphState = {
    nodes: Array.from({ length: count }, (_, i) => ({
      get subjectKey() {
        reads++;
        return `s:${i}`;
      },
      targetKey: `tmdb:movie:${i + 1}`,
      clusterId: i,
      ghost: false,
    })),
    edges: [],
    clusters: Array.from({ length: count }, (_, i) => ({
      id: i,
      medoid: `s:${i}`,
      labelKey: null,
      label: "",
      size: 1,
    })),
  };
  const result = collapseClusters(
    graph,
    new Set(graph.clusters.map((c) => c.id)),
  );
  assert.ok(reads <= count * 4, `expected linear identity reads, got ${reads}`);
  assert.equal(result.nodes.length, count);
  result.nodes.forEach((node, i) => {
    assert.equal(node.targetKey, `tmdb:movie:${i + 1}`);
    assert.equal(node.memberCount, 1);
  });
});

test("a result's duplicate edges merge by pair with kinds united and the max weight", () => {
  const graph = owned();
  assert.equal(graph.edges.length, 2);
  const ab = edgeBetween(graph, "s:a", "s:b");
  assert.deepEqual(ab?.kinds, ["relation:sequel", "semantic"]);
  assert.equal(ab?.weight, 0.7);
  assert.ok(graph.nodes.every((node) => !node.ghost));
});

test("a ghost that resolves to an owned title snaps to that node", () => {
  const incoming: LibraryGraphResult = {
    generatedAt: "2026-01-02T00:00:00Z",
    nodes: [
      // Another subject key for an owned title: it snaps to s:c.
      { subjectKey: "x:c", targetKey: "tmdb:movie:3", clusterId: 5 },
      { subjectKey: "x:new", targetKey: "tmdb:movie:99", clusterId: 5 },
    ],
    edges: [
      { from: "x:c", to: "s:a", weight: 0.9, kinds: ["semantic"] },
      {
        from: "x:new",
        to: "x:c",
        weight: 0.6,
        kinds: ["affinity:theme:isekai"],
      },
      { from: "x:new", to: "s:a", weight: 0.2, kinds: ["semantic"] },
      // Snapping turns this into a self-loop, which is dropped.
      { from: "x:c", to: "s:c", weight: 1, kinds: ["semantic"] },
    ],
    clusters: [
      { id: 5, labelKey: null, label: "Elsewhere", medoid: "x:new", size: 2 },
    ],
  };
  const merged = mergeLibraryGraphResult(owned(), incoming);

  assert.deepEqual(merged.nodes.map((node) => node.subjectKey).sort(), [
    "s:a",
    "s:b",
    "s:c",
    "x:new",
  ]);
  const ghost = merged.nodes.find((node) => node.subjectKey === "x:new");
  assert.equal(ghost?.ghost, true);
  // It settles beside its strongest owned neighbour, s:c in cluster 1.
  assert.equal(ghost?.clusterId, 1);
  assert.equal(edgeBetween(merged, "s:c", "s:a")?.weight, 0.9);
  assert.equal(edgeBetween(merged, "x:new", "s:c")?.weight, 0.6);
  assert.ok(merged.edges.every((edge) => edge.from !== edge.to));
  // The later result's clusters are not adopted; sizes follow membership.
  assert.deepEqual(
    merged.clusters.map((cluster) => [cluster.id, cluster.size]),
    [
      [0, 2],
      [1, 2],
    ],
  );
});

test("merging the same ghost twice keeps one node and unites its edge kinds", () => {
  const ghostResult = (
    kinds: string[],
    weight: number,
  ): LibraryGraphResult => ({
    generatedAt: "2026-01-02T00:00:00Z",
    nodes: [{ subjectKey: "x:g", targetKey: "tmdb:movie:50", clusterId: 0 }],
    edges: [{ from: "x:g", to: "s:b", weight, kinds }],
    clusters: [],
  });
  const once = mergeLibraryGraphResult(owned(), ghostResult(["semantic"], 0.5));
  const twice = mergeLibraryGraphResult(
    once,
    ghostResult(["relation:co_watch"], 0.3),
  );
  assert.equal(
    twice.nodes.filter((node) => node.subjectKey === "x:g").length,
    1,
  );
  const edge = edgeBetween(twice, "x:g", "s:b");
  assert.deepEqual(edge?.kinds, ["relation:co_watch", "semantic"]);
  assert.equal(edge?.weight, 0.5);
});

test("pruning to a visible set drops other nodes, their edges and empty clusters", () => {
  const pruned = pruneToVisible(owned(), new Set(["s:a", "s:b"]));
  assert.deepEqual(
    pruned.nodes.map((node) => node.subjectKey),
    ["s:a", "s:b"],
  );
  assert.equal(pruned.edges.length, 1);
  assert.deepEqual(
    pruned.clusters.map((cluster) => cluster.id),
    [0],
  );
});

test("clusters collapse only above the threshold, and expanded ones stay open", () => {
  const graph = owned();
  assert.equal(clustersToCollapse(graph, 3, new Set()).size, 0);
  assert.deepEqual(
    Array.from(clustersToCollapse(graph, 2, new Set())).sort(),
    [0, 1],
  );
  const expanded = toggleClusterExpanded(new Set(), 1);
  assert.deepEqual(Array.from(clustersToCollapse(graph, 2, expanded)), [0]);
  assert.equal(toggleClusterExpanded(expanded, 1).size, 0);
});

test("a collapsed cluster becomes one node and its outside edges follow it", () => {
  const collapsed = collapseClusters(owned(), new Set([0]));
  const stand = collapsed.nodes.find(
    (node) => node.subjectKey === clusterNodeKey(0),
  );
  assert.equal(stand?.collapsedClusterId, 0);
  assert.equal(stand?.memberCount, 2);
  // The medoid's target names the cluster node.
  assert.equal(stand?.targetKey, "tmdb:movie:1");
  assert.deepEqual(collapsed.nodes.map((node) => node.subjectKey).sort(), [
    clusterNodeKey(0),
    "s:c",
  ]);
  // The inside edge is gone; the bridge now joins the cluster node.
  assert.equal(collapsed.edges.length, 1);
  assert.equal(edgeBetween(collapsed, clusterNodeKey(0), "s:c")?.weight, 0.3);
  // Nothing collapsed leaves the graph as it was.
  const graph = owned();
  assert.equal(collapseClusters(graph, new Set()), graph);
});

test("LRU pruning evicts the oldest ghosts and never an owned node", () => {
  const incoming: LibraryGraphResult = {
    generatedAt: "2026-01-02T00:00:00Z",
    nodes: ["g1", "g2", "g3"].map((key, index) => ({
      subjectKey: key,
      targetKey: `tmdb:movie:${100 + index}`,
      clusterId: 0,
    })),
    edges: [
      { from: "g1", to: "s:a", weight: 0.5, kinds: ["semantic"] },
      { from: "g2", to: "s:a", weight: 0.5, kinds: ["semantic"] },
      { from: "g3", to: "g2", weight: 0.5, kinds: ["semantic"] },
    ],
    clusters: [],
  };
  const merged = mergeLibraryGraphResult(owned(), incoming);
  assert.equal(merged.nodes.length, 6);
  const lastUsed = new Map([
    ["g1", 30],
    ["g2", 10],
    ["s:a", 1],
  ]);
  // g3 was never used and g2 is the older of the rest.
  const { graph, evicted } = pruneLeastRecentlyUsed(merged, 4, lastUsed);
  assert.deepEqual(evicted, ["g3", "g2"]);
  assert.deepEqual(graph.nodes.map((node) => node.subjectKey).sort(), [
    "g1",
    "s:a",
    "s:b",
    "s:c",
  ]);
  assert.ok(
    graph.edges.every((edge) => edge.from !== "g2" && edge.to !== "g2"),
  );

  // A pinned ghost is kept; a cap below the owned count stops at the owned set.
  const pinned = pruneLeastRecentlyUsed(
    merged,
    0,
    lastUsed,
    (node) => node.subjectKey === "g1",
  );
  assert.deepEqual(pinned.evicted, ["g3", "g2"]);
  assert.equal(pinned.graph.nodes.length, 4);

  // Under the cap nothing moves.
  assert.deepEqual(pruneLeastRecentlyUsed(merged, 10, lastUsed).evicted, []);
});

test("two new nodes naming the same title become one ghost", () => {
  const merged = mergeLibraryGraphResult(owned(), {
    generatedAt: "2026-01-02T00:00:00Z",
    nodes: [
      { subjectKey: "x:one", targetKey: "tmdb:movie:70", clusterId: 4 },
      { subjectKey: "x:two", targetKey: "tmdb:movie:70", clusterId: 4 },
    ],
    edges: [
      { from: "x:one", to: "s:a", weight: 0.4, kinds: ["semantic"] },
      { from: "x:two", to: "s:a", weight: 0.6, kinds: ["relation:co_watch"] },
    ],
    clusters: [],
  });
  const ghosts = merged.nodes.filter((node) => node.ghost);
  assert.deepEqual(
    ghosts.map((node) => node.subjectKey),
    ["x:one"],
  );
  const edge = edgeBetween(merged, "x:one", "s:a");
  assert.deepEqual(edge?.kinds, ["relation:co_watch", "semantic"]);
  assert.equal(edge?.weight, 0.6);
});

test("edge weights are clamped to [0, 1] and a non-number counts as 0", () => {
  const graph = graphStateFromResult({
    generatedAt: "2026-01-01T00:00:00Z",
    nodes: [
      { subjectKey: "s:a", targetKey: "tmdb:movie:1", clusterId: 0 },
      { subjectKey: "s:b", targetKey: "tmdb:movie:2", clusterId: 1 },
      { subjectKey: "s:c", targetKey: "tmdb:movie:3", clusterId: 2 },
    ],
    edges: [
      { from: "s:a", to: "s:b", weight: Number.NaN, kinds: ["semantic"] },
      { from: "s:b", to: "s:c", weight: 7, kinds: ["semantic"] },
      { from: "s:a", to: "s:c", weight: -2, kinds: ["semantic"] },
    ],
    clusters: [],
  });
  assert.equal(edgeBetween(graph, "s:a", "s:b")?.weight, 0);
  assert.equal(edgeBetween(graph, "s:b", "s:c")?.weight, 1);
  assert.equal(edgeBetween(graph, "s:a", "s:c")?.weight, 0);

  // A NaN edge never wins a ghost's placement over a real one.
  const merged = mergeLibraryGraphResult(graph, {
    generatedAt: "2026-01-02T00:00:00Z",
    nodes: [{ subjectKey: "x:g", targetKey: "tmdb:movie:80", clusterId: 9 }],
    edges: [
      { from: "x:g", to: "s:a", weight: Number.NaN, kinds: ["semantic"] },
      { from: "x:g", to: "s:c", weight: 0.3, kinds: ["semantic"] },
    ],
    clusters: [],
  });
  assert.equal(
    merged.nodes.find((node) => node.subjectKey === "x:g")?.clusterId,
    2,
  );
  assert.ok(merged.edges.every((edge) => Number.isFinite(edge.weight)));
});

test("ghosts with blank target keys stay separate", () => {
  const merged = mergeLibraryGraphResult(owned(), {
    generatedAt: "2026-01-02T00:00:00Z",
    nodes: [
      { subjectKey: "x:blank-1", targetKey: "", clusterId: 4 },
      { subjectKey: "x:blank-2", targetKey: "  ", clusterId: 4 },
    ],
    edges: [
      { from: "x:blank-1", to: "s:a", weight: 0.4, kinds: ["semantic"] },
      { from: "x:blank-2", to: "s:c", weight: 0.4, kinds: ["semantic"] },
    ],
    clusters: [],
  });
  assert.deepEqual(
    merged.nodes
      .filter((node) => node.ghost)
      .map((node) => node.subjectKey)
      .sort(),
    ["x:blank-1", "x:blank-2"],
  );
  assert.ok(edgeBetween(merged, "x:blank-1", "s:a"));
  assert.ok(edgeBetween(merged, "x:blank-2", "s:c"));
});

test("a focused view keeps one node, its neighbours and the links among them", () => {
  const graph = owned();
  const ego = egoGraphState(graph, "s:a");
  assert.ok(ego);
  assert.deepEqual(ego.nodes.map((node) => node.subjectKey).sort(), [
    "s:a",
    "s:b",
  ]);
  // c is not a neighbour of a, so it and the b-c link are left out.
  assert.equal(ego.edges.length, 1);
  assert.equal(egoGraphState(graph, "s:missing"), null);

  const fromB = egoGraphState(graph, "s:b");
  assert.ok(fromB);
  assert.deepEqual(fromB.nodes.map((node) => node.subjectKey).sort(), [
    "s:a",
    "s:b",
    "s:c",
  ]);

  // Links between the neighbours stay; one leaving the neighbourhood does not.
  const ring: GraphState = {
    nodes: ["s:a", "s:b", "s:c", "s:d"].map((subjectKey) => ({
      subjectKey,
      targetKey: subjectKey,
      clusterId: 0,
      ghost: false,
    })),
    edges: [
      { from: "s:a", to: "s:b", weight: 0.5, kinds: ["semantic"] },
      { from: "s:a", to: "s:c", weight: 0.5, kinds: ["semantic"] },
      { from: "s:b", to: "s:c", weight: 0.3, kinds: ["semantic"] },
      { from: "s:c", to: "s:d", weight: 0.3, kinds: ["semantic"] },
    ],
    clusters: [],
  };
  const fromA = egoGraphState(ring, "s:a");
  assert.ok(fromA);
  assert.deepEqual(
    fromA.edges.map((edge) => `${edge.from}-${edge.to}`),
    ["s:a-s:b", "s:a-s:c", "s:b-s:c"],
  );
});

test("focused rings put library titles inside explored ones, heaviest link first", () => {
  const graph: GraphState = {
    nodes: [
      { subjectKey: "s:hub", targetKey: "t:hub", clusterId: 0, ghost: false },
      { subjectKey: "s:light", targetKey: "t:1", clusterId: 0, ghost: false },
      { subjectKey: "s:heavy", targetKey: "t:2", clusterId: 0, ghost: false },
      { subjectKey: "s:ghost-b", targetKey: "t:3", clusterId: 0, ghost: true },
      { subjectKey: "s:ghost-a", targetKey: "t:4", clusterId: 0, ghost: true },
    ],
    edges: [
      { from: "s:hub", to: "s:light", weight: 0.2, kinds: [] },
      { from: "s:heavy", to: "s:hub", weight: 0.9, kinds: [] },
      { from: "s:hub", to: "s:ghost-b", weight: 0.5, kinds: [] },
      { from: "s:hub", to: "s:ghost-a", weight: 0.5, kinds: [] },
    ],
    clusters: [],
  };
  assert.deepEqual(egoRingGroups(graph, "s:hub"), [
    ["s:heavy", "s:light"],
    ["s:ghost-a", "s:ghost-b"],
  ]);
  // With no explored neighbours there is only the library ring.
  const libraryOnly = { ...graph, edges: graph.edges.slice(0, 2) };
  assert.deepEqual(egoRingGroups(libraryOnly, "s:hub"), [
    ["s:heavy", "s:light"],
  ]);
});

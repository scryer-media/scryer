import assert from "node:assert/strict";
import test from "node:test";

import { UNPLACED_CLUSTER_ID } from "../types/library-graph.ts";
import { graphStateFromResult } from "./library-graph-merge.ts";
import {
  DEFAULT_TRAVERSAL_EDGE_WEIGHT,
  beginExpansion,
  collapseTraversalToCrumb,
  emptyTraversal,
  expandTraversal,
  ghostSubjectKey,
  invalidateExpansions,
  isCurrentExpansion,
  isLibraryItem,
  itemMatchKeys,
  liveGhostKeys,
  moreLikeThisEdgeKinds,
  moreLikeThisEdgeWeights,
  nextNodeInCluster,
  firstNodeInCluster,
  ownedNodeIndex,
  ownedSubjectForItem,
  pruneTraversal,
  neighborTargetKey,
  stampGhostTitle,
  uncoveredKeyboardGroups,
  seedGhostPositions,
  traversalGraphState,
  type TraversalItem,
  type TraversalState,
} from "./library-graph-traversal.ts";

type Item = TraversalItem & { name: string };

function item(targetKey: string, extra: Partial<Item> = {}): Item {
  return { targetKey, name: `Synthetic ${targetKey}`, ...extra };
}

function ownedGraph() {
  return graphStateFromResult({
    generatedAt: "2026-01-01T00:00:00Z",
    nodes: [
      { subjectKey: "s:a", targetKey: "tmdb:movie:1", clusterId: 0 },
      { subjectKey: "s:b", targetKey: "tmdb:movie:2", clusterId: 0 },
      { subjectKey: "s:c", targetKey: "tvdb:movie:3", clusterId: 1 },
    ],
    edges: [{ from: "s:a", to: "s:b", weight: 0.5, kinds: ["semantic"] }],
    clusters: [
      { id: 0, labelKey: null, label: "Zero", medoid: "s:a", size: 2 },
      { id: 1, labelKey: null, label: "One", medoid: "s:c", size: 1 },
    ],
  });
}

function ownedIndex() {
  return ownedNodeIndex([
    { subjectKey: "s:a", targetKey: "tmdb:movie:1", titleId: "title-a" },
    { subjectKey: "s:b", targetKey: "tmdb:movie:2", titleId: "title-b" },
    {
      subjectKey: "s:c",
      targetKey: "tvdb:movie:3",
      titleId: "title-c",
      // The title is also known by its IMDb id.
      matchKeys: ["imdb:title:tt0000303"],
    },
  ]);
}

test("relation types map straight onto edge kinds", () => {
  assert.deepEqual(moreLikeThisEdgeKinds(item("tmdb:movie:9")), [
    "relation:recommendation",
  ]);
  assert.deepEqual(
    moreLikeThisEdgeKinds(
      item("tmdb:movie:9", { relationTypes: ["Sequel", " co_watch "] }),
    ),
    ["relation:co_watch", "relation:sequel"],
  );
  // One type carries its subtypes.
  assert.deepEqual(
    moreLikeThisEdgeKinds(
      item("tmdb:movie:9", {
        relationTypes: ["adaptation"],
        relationSubtypes: ["source material", "manga"],
      }),
    ),
    ["relation:adaptation:manga", "relation:adaptation:source_material"],
  );
  // Several types cannot be paired with subtypes, so the bare types stand.
  assert.deepEqual(
    moreLikeThisEdgeKinds(
      item("tmdb:movie:9", {
        relationTypes: ["sequel", "spinoff"],
        relationSubtypes: ["direct"],
      }),
    ),
    ["relation:sequel", "relation:spinoff"],
  );
  // Blank entries are ignored.
  assert.deepEqual(
    moreLikeThisEdgeKinds(item("tmdb:movie:9", { relationTypes: [" ", ""] })),
    ["relation:recommendation"],
  );
});

test("edge weights come from rank scores, scaled into range, else the default", () => {
  assert.deepEqual(
    moreLikeThisEdgeWeights([
      item("a", { rankScore: 0.8 }),
      item("b", { rankScore: null }),
      item("c", { rankScore: Number.NaN }),
    ]),
    [0.8, DEFAULT_TRAVERSAL_EDGE_WEIGHT, DEFAULT_TRAVERSAL_EDGE_WEIGHT],
  );
  assert.deepEqual(
    moreLikeThisEdgeWeights([
      item("a", { rankScore: 40 }),
      item("b", { rankScore: 10 }),
    ]),
    [1, 0.25],
  );
});

test("an item's match keys cover its target key and its external ids", () => {
  assert.deepEqual(
    itemMatchKeys(
      item("tmdb:movie:0042", {
        externalIds: [
          { source: "imdb", kind: "title", id: "tt0000303", key: null },
          { source: "tvdb", kind: null, id: "5", key: "tvdb:series:5" },
          { source: "mal", kind: null, id: "7", key: null },
        ],
      }),
    ),
    ["tmdb:movie:42", "imdb:title:tt0000303", "tvdb:series:5"],
  );
});

test("an item snaps to the owned node by title id, then by any of its keys", () => {
  const owned = ownedIndex();
  assert.equal(
    ownedSubjectForItem(
      item("tmdb:movie:999", { resolvedTitleId: "title-b" }),
      owned,
    ),
    "s:b",
  );
  assert.equal(ownedSubjectForItem(item("tmdb:movie:01"), owned), "s:a");
  assert.equal(
    ownedSubjectForItem(
      item("tmdb:movie:777", {
        externalIds: [
          { source: "imdb", kind: "title", id: "tt0000303", key: null },
        ],
      }),
      owned,
    ),
    "s:c",
  );
  // The same id under another kind is a different title.
  assert.equal(ownedSubjectForItem(item("tmdb:series:1"), owned), null);
  assert.equal(ghostSubjectKey(item("TMDB:movie:0050")), "ghost:tmdb:movie:50");
  assert.equal(ghostSubjectKey(item("  ")), null);
});

test("expanding links owned results to their nodes and dedupes ghosts", () => {
  const state = expandTraversal(
    emptyTraversal<Item>(),
    "s:a",
    [
      item("tmdb:movie:2", { relationTypes: ["sequel"], rankScore: 0.9 }),
      item("tmdb:movie:50", { rankScore: 0.3 }),
      // The same ghost twice: one node, kinds united, heavier weight kept.
      item("tmdb:movie:050", { relationTypes: ["co_watch"], rankScore: 0.6 }),
      // The anchor itself never links to itself.
      item("tmdb:movie:1"),
    ],
    ownedIndex(),
  );
  assert.equal(state.expansions.length, 1);
  const [expansion] = state.expansions;
  assert.deepEqual(expansion.ghostKeys, ["ghost:tmdb:movie:50"]);
  assert.deepEqual(expansion.edges, [
    { from: "s:a", to: "s:b", weight: 0.9, kinds: ["relation:sequel"] },
    {
      from: "s:a",
      to: "ghost:tmdb:movie:50",
      weight: 0.6,
      kinds: ["relation:co_watch", "relation:recommendation"],
    },
  ]);
  assert.deepEqual(Array.from(state.ghostItems.keys()), [
    "ghost:tmdb:movie:50",
  ]);

  const graph = traversalGraphState(ownedGraph(), state);
  const ghost = graph.nodes.find(
    (node) => node.subjectKey === "ghost:tmdb:movie:50",
  );
  assert.equal(ghost?.ghost, true);
  // It settles in its anchor's cluster.
  assert.equal(ghost?.clusterId, 0);
  // No owned title was duplicated.
  assert.equal(graph.nodes.filter((node) => !node.ghost).length, 3);
  assert.ok(
    graph.edges.some(
      (edge) =>
        (edge.from === "s:a" && edge.to === "s:b") ||
        (edge.from === "s:b" && edge.to === "s:a"),
    ),
  );
});

test("an expansion keeps the server's groups with each one's subject keys", () => {
  const state = expandTraversal(
    emptyTraversal<Item>(),
    "s:a",
    [
      item("tmdb:movie:2", { groupId: 1 }),
      item("tmdb:movie:60", { groupId: 0 }),
      item("tmdb:movie:61", { groupId: 1 }),
      // The same ghost again under another group stays in its first one.
      item("tmdb:movie:060", { groupId: 1 }),
      item("tmdb:movie:62", { groupId: null }),
      // An id the groups do not name joins none.
      item("tmdb:movie:63", { groupId: 7 }),
    ],
    ownedIndex(),
    undefined,
    [
      { id: 1, labelKey: "theme:fixture-bravo", label: "Fixture Bravo" },
      { id: 0, labelKey: null, label: "Synthetic Sixty" },
      // A group none of the results fell into is dropped.
      { id: 2, labelKey: null, label: "Synthetic Empty" },
    ],
  );
  assert.deepEqual(state.expansions[0].groups, [
    {
      id: 1,
      labelKey: "theme:fixture-bravo",
      label: "Fixture Bravo",
      keys: ["s:b", "ghost:tmdb:movie:61"],
    },
    {
      id: 0,
      labelKey: null,
      label: "Synthetic Sixty",
      keys: ["ghost:tmdb:movie:60"],
    },
  ]);
  // Without groups an expansion has none.
  const plain = expandTraversal(
    emptyTraversal<Item>(),
    "s:a",
    [item("tmdb:movie:60", { groupId: 0 })],
    ownedIndex(),
  );
  assert.deepEqual(plain.expansions[0].groups, []);
});

test("a ghost reached from another ghost takes its cluster, and expanding again replaces in place", () => {
  let state = expandTraversal(
    emptyTraversal<Item>(),
    "s:c",
    [item("tmdb:movie:60")],
    ownedIndex(),
  );
  state = expandTraversal(
    state,
    "ghost:tmdb:movie:60",
    [item("tmdb:movie:61"), item("tmdb:movie:62")],
    ownedIndex(),
  );
  const graph = traversalGraphState(ownedGraph(), state);
  const clusterOf = (key: string) =>
    graph.nodes.find((node) => node.subjectKey === key)?.clusterId;
  assert.equal(clusterOf("ghost:tmdb:movie:60"), 1);
  assert.equal(clusterOf("ghost:tmdb:movie:61"), 1);

  const again = expandTraversal(
    state,
    "s:c",
    [item("tmdb:movie:60"), item("tmdb:movie:63")],
    ownedIndex(),
  );
  assert.deepEqual(
    again.expansions.map((expansion) => expansion.anchor),
    ["s:c", "ghost:tmdb:movie:60"],
  );
  assert.ok(again.expansions[0].expandedAt > again.expansions[1].expandedAt);
  assert.ok(liveGhostKeys(again.expansions).has("ghost:tmdb:movie:63"));
});

test("links from an anchor that is filtered out are left out", () => {
  const state = expandTraversal(
    emptyTraversal<Item>(),
    "s:gone",
    [item("tmdb:movie:70")],
    ownedIndex(),
  );
  const graph = traversalGraphState(ownedGraph(), state);
  assert.equal(
    graph.edges.some((edge) => edge.from === "s:gone" || edge.to === "s:gone"),
    false,
  );
});

function batch(start: number, count: number): Item[] {
  return Array.from({ length: count }, (_, index) =>
    item(`tmdb:movie:${start + index}`),
  );
}

test("the ghost cap prunes the least recently expanded expansion first", () => {
  const owned = ownedIndex();
  let state = expandTraversal(
    emptyTraversal<Item>(),
    "s:a",
    batch(100, 3),
    owned,
    5,
  );
  state = expandTraversal(state, "s:b", batch(200, 2), owned, 5);
  // Touch the oldest so the second one becomes least recently expanded.
  state = expandTraversal(state, "s:a", batch(100, 3), owned, 5);
  state = expandTraversal(state, "s:c", batch(300, 2), owned, 5);
  assert.deepEqual(
    state.expansions.map((expansion) => expansion.anchor),
    ["s:a", "s:c"],
  );
  assert.equal(liveGhostKeys(state.expansions).size, 5);
  assert.equal(state.ghostItems.has("ghost:tmdb:movie:200"), false);
  assert.equal(state.ghostItems.has("ghost:tmdb:movie:100"), true);
});

test("pruning keeps ghosts another live expansion reached or that were expanded", () => {
  const owned = ownedIndex();
  let state = expandTraversal(
    emptyTraversal<Item>(),
    "s:a",
    [item("tmdb:movie:400"), item("tmdb:movie:401")],
    owned,
  );
  state = expandTraversal(
    state,
    "s:b",
    [item("tmdb:movie:401"), item("tmdb:movie:402")],
    owned,
  );
  // The oldest expansion goes; the ghost the second one also reached stays.
  const shared = pruneTraversal(state, 2, "s:b");
  assert.deepEqual(
    shared.expansions.map((expansion) => expansion.anchor),
    ["s:b"],
  );
  assert.deepEqual(Array.from(shared.ghostItems.keys()).sort(), [
    "ghost:tmdb:movie:401",
    "ghost:tmdb:movie:402",
  ]);

  state = expandTraversal(
    state,
    "ghost:tmdb:movie:400",
    [item("tmdb:movie:403")],
    owned,
  );
  // Both library expansions go, oldest first; the expanded ghost stays even
  // though the expansion that reached it is gone, and the one being kept is
  // never dropped.
  const pruned = pruneTraversal(state, 0, "ghost:tmdb:movie:400");
  assert.deepEqual(
    pruned.expansions.map((expansion) => expansion.anchor),
    ["ghost:tmdb:movie:400"],
  );
  assert.deepEqual(Array.from(pruned.ghostItems.keys()).sort(), [
    "ghost:tmdb:movie:400",
    "ghost:tmdb:movie:403",
  ]);
});

test("collapsing to a crumb keeps the trail up to it and its ghosts", () => {
  const owned = ownedIndex();
  let state: TraversalState<Item> = emptyTraversal<Item>();
  state = expandTraversal(state, "s:a", [item("tmdb:movie:500")], owned);
  state = expandTraversal(
    state,
    "ghost:tmdb:movie:500",
    [item("tmdb:movie:501")],
    owned,
  );
  state = expandTraversal(
    state,
    "ghost:tmdb:movie:501",
    [item("tmdb:movie:502")],
    owned,
  );
  const collapsed = collapseTraversalToCrumb(state, 0);
  assert.deepEqual(
    collapsed.expansions.map((expansion) => expansion.anchor),
    ["s:a"],
  );
  assert.deepEqual(Array.from(collapsed.ghostItems.keys()), [
    "ghost:tmdb:movie:500",
  ]);
  // The last crumb is the current state.
  assert.equal(collapseTraversalToCrumb(state, 2), state);
  // Before the first crumb is the library alone.
  assert.equal(collapseTraversalToCrumb(state, -1).expansions.length, 0);
  assert.equal(collapseTraversalToCrumb(state, -1).ghostItems.size, 0);
});

test("new ghosts start beside their anchor and placed nodes keep their spot", () => {
  const state = expandTraversal(
    emptyTraversal<Item>(),
    "s:a",
    [item("tmdb:movie:600"), item("tmdb:movie:601")],
    ownedIndex(),
  );
  const previous = new Map([
    ["s:a", { x: 100, y: -50 }],
    ["ghost:tmdb:movie:601", { x: 7, y: 7 }],
  ]);
  const seeded = seedGhostPositions(previous, state.expansions);
  const ghost = seeded.get("ghost:tmdb:movie:600");
  assert.ok(ghost);
  assert.ok(Math.hypot(ghost.x - 100, ghost.y + 50) <= 40.0001);
  assert.deepEqual(seeded.get("ghost:tmdb:movie:601"), { x: 7, y: 7 });
  // Seeding is stable.
  assert.deepEqual(
    seedGhostPositions(previous, state.expansions).get("ghost:tmdb:movie:600"),
    ghost,
  );
});

test("keyboard focus cycles through the current cluster in label order", () => {
  const nodes = [
    { key: "k:3", clusterId: 1, label: "Gamma" },
    { key: "k:1", clusterId: 1, label: "Alpha" },
    { key: "k:2", clusterId: 1, label: "Beta" },
    { key: "k:9", clusterId: 0, label: "Zeta" },
    { key: "k:u", clusterId: UNPLACED_CLUSTER_ID, label: "Aardvark" },
  ];
  assert.equal(nextNodeInCluster(nodes, null, 1), "k:9");
  assert.equal(nextNodeInCluster(nodes, "k:1", 1), "k:2");
  assert.equal(nextNodeInCluster(nodes, "k:3", 1), "k:1");
  assert.equal(nextNodeInCluster(nodes, "k:1", -1), "k:3");
  assert.equal(nextNodeInCluster(nodes, "k:9", 1), "k:9");
  assert.equal(nextNodeInCluster(nodes, "k:missing", 1), "k:9");
  assert.equal(nextNodeInCluster([], null, 1), null);
  assert.equal(firstNodeInCluster(nodes, 1), "k:1");
  assert.equal(firstNodeInCluster(nodes, 5), null);
});

test("a reload or reset retires a More Like This reply still in flight", () => {
  const guard = { current: 0 };
  const beforeReload = beginExpansion(guard);
  assert.equal(isCurrentExpansion(guard, beforeReload), true);
  // The library selection changes and the graph reloads before the reply.
  invalidateExpansions(guard);
  assert.equal(isCurrentExpansion(guard, beforeReload), false);

  // A request started after the reload applies; a later one supersedes it.
  const afterReload = beginExpansion(guard);
  assert.equal(isCurrentExpansion(guard, afterReload), true);
  const later = beginExpansion(guard);
  assert.equal(isCurrentExpansion(guard, afterReload), false);
  assert.equal(isCurrentExpansion(guard, later), true);
});

test("a neighbour reply's owned titles snap onto their nodes, from any anchor", () => {
  // Neighbours of a ghost anchor: one result is an owned title Scryer resolved
  // under another source's key, one is owned by key alone, one is new.
  const fromOwned = expandTraversal(
    emptyTraversal<Item>(),
    "s:a",
    [item("tmdb:movie:61")],
    ownedIndex(),
  );
  const ghost = ghostSubjectKey(item("tmdb:movie:61"));
  assert.ok(ghost);
  const state = expandTraversal(
    fromOwned,
    ghost,
    [
      item("tvdb:movie:888", { resolvedTitleId: "title-b" }),
      item("imdb:title:tt0000303"),
      item("tmdb:movie:62"),
    ],
    ownedIndex(),
  );
  const graph = traversalGraphState(ownedGraph(), state);
  const linked = (to: string) =>
    graph.edges.some(
      (edge) =>
        (edge.from === ghost && edge.to === to) ||
        (edge.to === ghost && edge.from === to),
    );
  assert.equal(linked("s:b"), true);
  assert.equal(linked("s:c"), true);
  assert.equal(linked("ghost:tmdb:movie:62"), true);
  // Owned results join no ghost of their own.
  assert.deepEqual(
    graph.nodes
      .filter((node) => node.ghost)
      .map((node) => node.subjectKey)
      .sort(),
    ["ghost:tmdb:movie:61", "ghost:tmdb:movie:62"],
  );
});

test("an explored title is asked about by its first key in the discovery vocabulary", () => {
  assert.equal(neighborTargetKey(item("TMDB:movie:0071")), "tmdb:movie:71");
  assert.equal(
    neighborTargetKey(
      item("unknown:thing:1", {
        externalIds: [
          { source: "imdb", kind: "title", id: "tt0000072", key: null },
        ],
      }),
    ),
    "imdb:title:tt0000072",
  );
  assert.equal(neighborTargetKey(item("unknown:thing:1")), null);
});

test("an explored title is in the library when Scryer resolved it or the listing names it", () => {
  const listing = new Set(["tmdb:movie:81"]);
  assert.equal(
    isLibraryItem(
      item("tmdb:movie:80", { resolvedTitleId: "title-80" }),
      listing,
    ),
    true,
  );
  assert.equal(isLibraryItem(item("TMDB:movie:081"), listing), true);
  assert.equal(
    isLibraryItem(item("tmdb:movie:82", { resolvedTitleId: "  " }), listing),
    false,
  );
  assert.equal(isLibraryItem(item("tmdb:movie:83"), listing), false);
});

test("an added ghost is stamped with its new title without moving the trail", () => {
  const explored = expandTraversal(
    emptyTraversal<Item>(),
    "s:a",
    [item("tmdb:movie:91"), item("tmdb:movie:92")],
    ownedIndex(),
  );
  const key = ghostSubjectKey(item("tmdb:movie:91"));
  assert.ok(key);
  const stamped = stampGhostTitle(explored, key, "title-91");
  assert.equal(stamped.ghostItems.get(key)?.resolvedTitleId, "title-91");
  assert.equal(isLibraryItem(stamped.ghostItems.get(key)!, new Set()), true);
  assert.equal(stamped.expansions, explored.expansions);
  assert.equal(stamped.clock, explored.clock);
  // Unknown ghosts and repeat stamps leave the state as it was.
  assert.equal(stampGhostTitle(explored, "ghost:tmdb:movie:99", "x"), explored);
  assert.equal(stampGhostTitle(stamped, key, "title-91"), stamped);
});

test("explored titles in no library cluster get a keyboard group of their own", () => {
  const nodes = [
    { key: "s:a", clusterId: 0, label: "Synthetic A" },
    { key: "ghost:tmdb:movie:1", clusterId: 0, label: "Synthetic G1" },
    {
      key: "ghost:tmdb:movie:2",
      clusterId: UNPLACED_CLUSTER_ID,
      label: "Synthetic G2",
    },
    {
      key: "ghost:tmdb:movie:3",
      clusterId: UNPLACED_CLUSTER_ID,
      label: "Synthetic G3",
    },
  ];
  // The library has no unplaced titles, so no chip leads to the unplaced ghosts.
  assert.deepEqual(uncoveredKeyboardGroups(nodes, new Set([0])), [
    { id: UNPLACED_CLUSTER_ID, count: 2 },
  ]);
  assert.equal(
    firstNodeInCluster(nodes, UNPLACED_CLUSTER_ID),
    "ghost:tmdb:movie:2",
  );
  // When a library chip covers them, no extra group is needed.
  assert.deepEqual(
    uncoveredKeyboardGroups(nodes, new Set([0, UNPLACED_CLUSTER_ID])),
    [],
  );
});

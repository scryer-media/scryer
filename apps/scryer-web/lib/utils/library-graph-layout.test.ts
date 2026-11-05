import assert from "node:assert/strict";
import test from "node:test";

import { libraryGraphFixture } from "./library-graph-fixture.ts";
import {
  CLUSTER_RING_WIDTH,
  clusterCenters,
  convexHull,
  createLayout,
  egoLayout,
  layoutIsSettled,
  layoutLoopDecision,
  layoutPositions,
  stepLayout,
  tweenPositions,
} from "./library-graph-layout.ts";

function fixtureLayoutInput() {
  const graph = libraryGraphFixture({ facet: "MOVIE" });
  return {
    nodes: graph.nodes.map((node) => ({
      key: node.subjectKey,
      clusterId: node.clusterId,
    })),
    edges: graph.edges,
  };
}

function settle(layout: ReturnType<typeof createLayout>) {
  let steps = 0;
  while (stepLayout(layout, 50) > 0) {
    steps += 50;
    assert.ok(steps < 10_000, "layout never settled");
  }
  return steps;
}

test("the same graph always lays out the same way", () => {
  const { nodes, edges } = fixtureLayoutInput();
  const first = createLayout(nodes, edges);
  const second = createLayout(nodes, edges);
  settle(first);
  settle(second);
  assert.ok(layoutIsSettled(first));
  assert.deepEqual(layoutPositions(first), layoutPositions(second));
});

test("clusters gather around their own centres", () => {
  const { nodes, edges } = fixtureLayoutInput();
  const layout = createLayout(nodes, edges);
  settle(layout);
  const positions = layoutPositions(layout);
  const centers = clusterCenters(nodes.map((node) => node.clusterId));
  for (const node of nodes) {
    const point = positions.get(node.key);
    assert.ok(point && Number.isFinite(point.x) && Number.isFinite(point.y));
    // Each node is nearer its own cluster's centre than any other cluster's.
    const own = centers.get(node.clusterId);
    assert.ok(own);
    const ownDistance = Math.hypot(point.x - own.x, point.y - own.y);
    for (const [id, center] of centers) {
      if (id !== node.clusterId) {
        assert.ok(
          ownDistance < Math.hypot(point.x - center.x, point.y - center.y),
          `${node.key} drifted toward cluster ${id}`,
        );
      }
    }
  }
});

test("pruning the node set settles from the old positions instead of restarting", () => {
  const { nodes, edges } = fixtureLayoutInput();
  const full = createLayout(nodes, edges);
  const fullSteps = settle(full);
  const before = layoutPositions(full);

  // Drop every fifth node, as a facet chip or library filter would.
  const kept = nodes.filter((_, index) => index % 5 !== 0);
  const resettled = createLayout(kept, edges, before);
  // It starts exactly where the nodes were.
  for (const node of kept) {
    assert.deepEqual(
      layoutPositions(resettled).get(node.key),
      before.get(node.key),
    );
  }
  const resettleSteps = settle(resettled);
  assert.ok(
    resettleSteps < fullSteps,
    `settling took ${resettleSteps} steps, a fresh layout ${fullSteps}`,
  );
  // And it stays near where each node was.
  for (const node of kept) {
    const was = before.get(node.key);
    const now = layoutPositions(resettled).get(node.key);
    assert.ok(was && now);
    assert.ok(Math.hypot(now.x - was.x, now.y - was.y) < 120);
  }
});

test("the convex hull keeps the outer points in order", () => {
  const hull = convexHull([
    { x: 0, y: 0 },
    { x: 2, y: 0 },
    { x: 1, y: 1 },
    { x: 2, y: 2 },
    { x: 0, y: 2 },
    { x: 0, y: 2 },
  ]);
  assert.deepEqual(hull, [
    { x: 0, y: 0 },
    { x: 2, y: 0 },
    { x: 2, y: 2 },
    { x: 0, y: 2 },
  ]);
  assert.deepEqual(convexHull([{ x: 1, y: 1 }]), [{ x: 1, y: 1 }]);
});

test("filtering out a whole cluster leaves the other clusters where they were", () => {
  const { nodes, edges } = fixtureLayoutInput();
  const allClusterIds = nodes.map((node) => node.clusterId);
  const full = createLayout(nodes, edges, new Map(), allClusterIds);
  settle(full);
  const before = layoutPositions(full);

  // Drop cluster 2 and the one unplaced node.
  const kept = nodes.filter(
    (node) => node.clusterId !== 2 && node.clusterId !== -1,
  );
  const drift = (layout: ReturnType<typeof createLayout>) => {
    settle(layout);
    const after = layoutPositions(layout);
    const distances = kept
      .map((node) => {
        const was = before.get(node.key);
        const now = after.get(node.key);
        assert.ok(was && now);
        // Measured with the oval's widening taken out, so a drift counts
        // the same across and down.
        return Math.hypot((now.x - was.x) / CLUSTER_RING_WIDTH, now.y - was.y);
      })
      .sort((a, b) => a - b);
    return {
      median: distances[distances.length >> 1],
      max: distances[distances.length - 1],
    };
  };

  const resettled = createLayout(kept, edges, before, allClusterIds);
  assert.deepEqual(resettled.centers, full.centers);
  const settled = drift(resettled);
  // Nodes that were linked into the dropped cluster ease inward a little;
  // the rest barely move.
  assert.ok(settled.median < 40, `median drift ${settled.median}`);
  assert.ok(settled.max < 120, `max drift ${settled.max}`);

  // Centres taken from the surviving nodes alone move whole clusters.
  const fromSurvivors = createLayout(kept, edges, before);
  assert.notDeepEqual(fromSurvivors.centers, full.centers);
  assert.ok(drift(fromSurvivors).median > 100);
});

test("the layout loop pauses while its container has no size and resumes after", () => {
  const { nodes, edges } = fixtureLayoutInput();
  const layout = createLayout(nodes, edges);
  assert.equal(layoutLoopDecision({ width: 0, height: 0 }, layout), "pause");
  assert.equal(layoutLoopDecision({ width: 320, height: 0 }, layout), "pause");
  assert.equal(layoutLoopDecision({ width: 320, height: 480 }, layout), "step");
  settle(layout);
  assert.equal(layoutLoopDecision({ width: 320, height: 480 }, layout), "done");
  assert.equal(layoutLoopDecision({ width: 0, height: 0 }, layout), "done");
  assert.equal(layoutLoopDecision({ width: 320, height: 480 }, null), "done");
});

test("a focused layout rings each group around the centre, inner groups first", () => {
  const library = ["n:1", "n:2", "n:3"];
  const explored = ["x:1", "x:2", "x:3", "x:4"];
  const positions = egoLayout("c", [[library], [explored]], 200, 10);
  assert.deepEqual(positions.get("c"), { x: 0, y: 0 });
  const radius = (key: string) => {
    const point = positions.get(key);
    assert.ok(point);
    return Math.hypot(point.x, point.y);
  };
  for (const key of library) {
    assert.ok(Math.abs(radius(key) - 100) < 1e-9, `${key} at ${radius(key)}`);
  }
  for (const key of explored) {
    assert.ok(Math.abs(radius(key) - 200) < 1e-9, `${key} at ${radius(key)}`);
  }
  // The first node of the inner ring sits straight up from the centre.
  const first = positions.get("n:1");
  assert.ok(first && Math.abs(first.x) < 1e-9 && first.y < 0);
});

test("a crowded ring spills onto further rings and never past the outer radius", () => {
  const many = Array.from({ length: 60 }, (_, index) => `n:${index}`);
  const positions = egoLayout("c", [[many]], 100, 20);
  assert.equal(positions.size, 61);
  const radii = new Set(
    many.map((key) => {
      const point = positions.get(key);
      assert.ok(point);
      return Math.round(Math.hypot(point.x, point.y));
    }),
  );
  assert.ok(radii.size > 1, "everything landed on one ring");
  assert.ok(Math.max(...radii) <= 100);
  // Nothing at all to place leaves the centre alone.
  assert.equal(egoLayout("c", [[[]], [["c"]]], 100, 20).size, 1);
});

test("tweening eases each node from where it was to where it goes", () => {
  const from = new Map([["a", { x: 0, y: 0 }]]);
  const to = new Map([
    ["a", { x: 10, y: -10 }],
    ["b", { x: 5, y: 5 }],
  ]);
  assert.deepEqual(tweenPositions(from, to, 0).get("a"), { x: 0, y: 0 });
  assert.deepEqual(tweenPositions(from, to, 1).get("a"), { x: 10, y: -10 });
  const half = tweenPositions(from, to, 0.5).get("a");
  // Ease-out: more than halfway at half time.
  assert.ok(half && half.x > 5 && half.x < 10);
  // A node with no start is already where it goes.
  assert.deepEqual(tweenPositions(from, to, 0.2).get("b"), { x: 5, y: 5 });
});

test("each section of a ring keeps to its own arc, with a gap between sections", () => {
  const sections = [
    ["a:1", "a:2", "a:3"],
    ["b:1", "b:2"],
  ];
  const positions = egoLayout("c", [sections], 100, 5);
  // Angles clockwise from straight up, in slots of a full turn over the
  // five nodes and two gaps.
  const slot = (key: string) => {
    const point = positions.get(key);
    assert.ok(point);
    let angle = Math.atan2(point.y, point.x) + Math.PI / 2;
    if (angle < 0) {
      angle += Math.PI * 2;
    }
    return Math.round((angle / (Math.PI * 2)) * 7);
  };
  assert.deepEqual(
    ["a:1", "a:2", "a:3", "b:1", "b:2"].map(slot),
    [0, 1, 2, 4, 5],
  );
});

test("a weakly linked title stays near the rest of its cluster", () => {
  // Cluster 0's core is tied hard to cluster 1's, which drags it off its
  // ring spot; its last title has one weak link into the core.
  const core = ["a:1", "a:2", "a:3", "a:4", "a:5"];
  const other = ["b:1", "b:2", "b:3", "b:4", "b:5"];
  const nodes = [
    ...core.map((key) => ({ key, clusterId: 0 })),
    { key: "a:weak", clusterId: 0 },
    ...other.map((key) => ({ key, clusterId: 1 })),
  ];
  const edges = [
    ...core.flatMap((from, i) =>
      core.slice(i + 1).map((to) => ({ from, to, weight: 1 })),
    ),
    ...other.flatMap((from, i) =>
      other.slice(i + 1).map((to) => ({ from, to, weight: 1 })),
    ),
    ...core.map((from, i) => ({ from, to: other[i], weight: 1 })),
    { from: "a:weak", to: "a:1", weight: 0.05 },
  ];
  const layout = createLayout(nodes, edges);
  settle(layout);
  const positions = layoutPositions(layout);
  const middle = core.reduce(
    (total, key) => {
      const point = positions.get(key);
      assert.ok(point);
      return {
        x: total.x + point.x / core.length,
        y: total.y + point.y / core.length,
      };
    },
    { x: 0, y: 0 },
  );
  const weak = positions.get("a:weak");
  assert.ok(weak);
  const spread = Math.max(
    ...core.map((key) => {
      const point = positions.get(key);
      assert.ok(point);
      return Math.hypot(point.x - middle.x, point.y - middle.y);
    }),
  );
  const distance = Math.hypot(weak.x - middle.x, weak.y - middle.y);
  assert.ok(
    distance < spread * 3,
    `weak title ${distance.toFixed(0)} from the core, core spread ${spread.toFixed(0)}`,
  );
});

test("a title with no links rests at the edge of the graph, not off on its own", () => {
  const { nodes, edges } = fixtureLayoutInput();
  const loners = [
    { key: "loner:1", clusterId: 40 },
    { key: "loner:2", clusterId: 41 },
  ];
  const layout = createLayout([...nodes, ...loners], edges);
  settle(layout);
  const positions = layoutPositions(layout);
  const linked = new Set(edges.flatMap((edge) => [edge.from, edge.to]));
  for (const loner of loners) {
    const point = positions.get(loner.key);
    assert.ok(point);
    const nearest = Math.min(
      ...nodes
        .filter((node) => linked.has(node.key))
        .map((node) => {
          const other = positions.get(node.key);
          assert.ok(other);
          return Math.hypot(other.x - point.x, other.y - point.y);
        }),
    );
    assert.ok(nearest < 120, `${loner.key} is ${nearest.toFixed(0)} away`);
  }
});

test("small disconnected neighbourhoods stay near the unplaced series", () => {
  const nodes = [
    ...[0, 1].flatMap((clusterId) =>
      Array.from({ length: 4 }, (_, index) => ({
        key: `series:${clusterId}:${index}`,
        clusterId,
      })),
    ),
    ...Array.from({ length: 14 }, (_, index) => ({
      key: `unplaced:${index}`,
      clusterId: -1,
      free: true,
    })),
  ];
  const edges = [
    ...[0, 1].flatMap((clusterId) => {
      const members = nodes.filter((node) => node.clusterId === clusterId);
      return members.flatMap((node, index) =>
        members.slice(index + 1).map((other) => ({
          from: node.key,
          to: other.key,
          weight: 0.5,
        })),
      );
    }),
    { from: "unplaced:4", to: "unplaced:5", weight: 0.5 },
  ];
  const layout = createLayout(nodes, edges);
  settle(layout);
  const positions = layoutPositions(layout);

  for (const clusterId of [0, 1]) {
    const members = nodes.filter((node) => node.clusterId === clusterId);
    const unplaced = nodes.filter((node) => node.clusterId === -1);
    const gap = Math.min(
      ...members.flatMap((member) =>
        unplaced.map((other) => {
          const from = positions.get(member.key);
          const to = positions.get(other.key);
          assert.ok(from && to);
          return Math.hypot(from.x - to.x, from.y - to.y);
        }),
      ),
    );
    assert.ok(
      gap < 120,
      `neighbourhood ${clusterId} is ${gap.toFixed(0)} away`,
    );
  }

  const points = [...positions.values()];
  for (let index = 0; index < points.length; index += 1) {
    for (const other of points.slice(index + 1)) {
      const point = points[index];
      assert.ok(Math.hypot(point.x - other.x, point.y - other.y) > 20);
    }
  }
});

test("the layout comes out wider than tall, for wide screens", () => {
  const { nodes, edges } = fixtureLayoutInput();
  const layout = createLayout(nodes, edges);
  settle(layout);
  const points = [...layoutPositions(layout).values()];
  const span = (values: number[]) => Math.max(...values) - Math.min(...values);
  const width = span(points.map((point) => point.x));
  const height = span(points.map((point) => point.y));
  assert.ok(width > height * 1.4, `width ${width}, height ${height}`);
});

/** Whether the point lies inside the counter-clockwise convex polygon. */
function insideHull(
  hull: readonly { x: number; y: number }[],
  point: { x: number; y: number },
): boolean {
  if (hull.length < 3) {
    return false;
  }
  return hull.every((from, index) => {
    const to = hull[(index + 1) % hull.length];
    return (
      (to.x - from.x) * (point.y - from.y) -
        (to.y - from.y) * (point.x - from.x) >
      0
    );
  });
}

// The shape of a real library's graph, with numbered titles: its cluster of
// each title, then its links as [title, title, weight]. Its neighbourhoods
// are densely linked to one another, so a layout that ignores neighbourhoods
// draws titles inside other neighbourhoods' outlines.
const DENSE_CLUSTERS = [
  1, 1, 0, 0, 3, 4, 3, 2, 0, 1, 1, 3, 2, 3, 0, 0, 1, 0, 0, 5, 2, 0, 2, 0, 1, 0,
  0, 0, 2, 3, 0, 6, 0, 3, 0, 2, 0, 3, 3, 7, 0, 0, 1, 0, 0, 2, 0, 1, 0, 1, 0, 3,
  8, 0, 0, 9, 0, 2, 1, 0, 10, 0, 1, 2, 11, 1, 1, 12, 1, 2,
];
const DENSE_NAMED = new Set([0, 1, 2, 3]);
const DENSE_EDGES: [number, number, number][] = [
  [0, 1, 0.35],
  [0, 8, 0.35],
  [0, 65, 0.35],
  [1, 2, 0.8],
  [1, 11, 0.57],
  [1, 18, 0.58],
  [1, 22, 0.8],
  [1, 24, 0.8],
  [1, 25, 0.56],
  [1, 27, 0.35],
  [1, 30, 0.8],
  [1, 31, 0.35],
  [1, 32, 0.56],
  [1, 36, 0.8],
  [1, 37, 0.8],
  [1, 41, 0.6],
  [1, 42, 0.8],
  [1, 53, 0.35],
  [1, 62, 0.8],
  [1, 66, 0.8],
  [1, 68, 0.35],
  [2, 3, 0.8],
  [2, 9, 0.8],
  [2, 14, 0.8],
  [2, 18, 0.8],
  [2, 21, 0.8],
  [2, 24, 0.8],
  [2, 27, 0.57],
  [2, 30, 0.8],
  [2, 44, 0.8],
  [2, 53, 0.8],
  [2, 69, 0.8],
  [3, 10, 0.35],
  [3, 11, 0.61],
  [3, 14, 0.62],
  [3, 15, 0.6],
  [3, 17, 0.8],
  [3, 18, 0.61],
  [3, 23, 0.35],
  [3, 25, 0.8],
  [3, 26, 0.8],
  [3, 27, 0.8],
  [3, 31, 0.35],
  [3, 32, 0.57],
  [3, 34, 0.8],
  [3, 44, 0.35],
  [3, 46, 0.8],
  [3, 50, 0.35],
  [3, 52, 0.35],
  [3, 55, 0.35],
  [3, 58, 0.8],
  [3, 61, 0.8],
  [3, 66, 0.8],
  [3, 68, 0.35],
  [4, 6, 0.8],
  [4, 7, 0.8],
  [4, 13, 0.8],
  [4, 22, 0.8],
  [4, 29, 0.8],
  [4, 37, 0.8],
  [4, 38, 0.8],
  [4, 51, 0.6],
  [4, 54, 0.6],
  [4, 66, 0.8],
  [6, 11, 0.6],
  [6, 13, 0.8],
  [6, 16, 0.8],
  [6, 22, 0.8],
  [6, 25, 0.8],
  [6, 29, 0.8],
  [6, 37, 0.8],
  [6, 38, 0.8],
  [6, 51, 0.6],
  [6, 54, 0.8],
  [7, 12, 0.8],
  [7, 13, 0.8],
  [7, 20, 0.8],
  [7, 22, 0.8],
  [7, 33, 0.8],
  [7, 39, 0.6],
  [7, 51, 0.8],
  [7, 55, 0.6],
  [7, 69, 0.8],
  [8, 9, 0.8],
  [8, 10, 0.35],
  [8, 16, 0.8],
  [8, 18, 0.8],
  [8, 22, 0.8],
  [8, 25, 0.6],
  [8, 26, 0.8],
  [8, 31, 0.35],
  [8, 32, 0.57],
  [8, 34, 0.8],
  [8, 36, 0.35],
  [8, 40, 0.6],
  [8, 47, 0.8],
  [8, 59, 0.58],
  [8, 67, 0.57],
  [9, 11, 0.8],
  [9, 14, 0.56],
  [9, 16, 0.8],
  [9, 24, 0.8],
  [9, 30, 0.8],
  [9, 44, 0.35],
  [9, 47, 0.6],
  [9, 49, 0.8],
  [9, 66, 0.8],
  [10, 12, 0.8],
  [10, 18, 0.56],
  [10, 24, 0.8],
  [10, 25, 0.57],
  [10, 31, 0.8],
  [10, 60, 0.57],
  [10, 67, 0.61],
  [10, 69, 0.8],
  [11, 13, 0.8],
  [11, 16, 0.8],
  [11, 17, 0.8],
  [11, 25, 0.8],
  [11, 29, 0.8],
  [11, 32, 0.8],
  [11, 36, 0.6],
  [11, 37, 0.8],
  [11, 38, 0.8],
  [11, 45, 0.56],
  [11, 48, 0.8],
  [11, 50, 0.8],
  [11, 54, 0.65],
  [11, 65, 0.35],
  [11, 67, 0.59],
  [11, 68, 0.8],
  [12, 19, 0.8],
  [12, 20, 0.8],
  [12, 24, 0.8],
  [12, 28, 0.8],
  [12, 33, 0.8],
  [12, 36, 0.8],
  [12, 37, 0.55],
  [12, 39, 0.8],
  [12, 69, 0.55],
  [13, 17, 0.8],
  [13, 18, 0.8],
  [13, 22, 0.8],
  [13, 29, 0.8],
  [13, 33, 0.8],
  [13, 35, 0.8],
  [13, 51, 0.6],
  [14, 26, 0.8],
  [14, 49, 0.8],
  [14, 69, 0.8],
  [15, 18, 0.6],
  [15, 23, 0.6],
  [15, 56, 0.6],
  [16, 25, 0.8],
  [16, 36, 0.56],
  [16, 37, 0.35],
  [16, 41, 0.6],
  [16, 42, 0.8],
  [16, 46, 0.8],
  [16, 49, 0.8],
  [16, 59, 0.8],
  [16, 66, 0.8],
  [16, 68, 0.8],
  [17, 21, 0.8],
  [17, 22, 0.8],
  [17, 27, 0.8],
  [17, 29, 0.8],
  [17, 31, 0.6],
  [17, 32, 0.8],
  [17, 67, 0.57],
  [18, 20, 0.8],
  [18, 21, 0.8],
  [18, 24, 0.57],
  [18, 25, 0.69],
  [18, 30, 0.8],
  [18, 32, 0.8],
  [18, 34, 0.8],
  [18, 41, 0.6],
  [18, 46, 0.8],
  [18, 49, 0.8],
  [18, 50, 0.57],
  [18, 53, 0.8],
  [18, 56, 0.57],
  [18, 59, 0.65],
  [18, 60, 0.55],
  [18, 67, 0.59],
  [20, 26, 0.8],
  [20, 28, 0.8],
  [20, 57, 0.6],
  [20, 63, 0.8],
  [20, 66, 0.8],
  [20, 69, 0.8],
  [21, 24, 0.8],
  [21, 26, 0.8],
  [21, 33, 0.35],
  [21, 34, 0.8],
  [21, 43, 0.6],
  [21, 47, 0.35],
  [21, 50, 0.35],
  [21, 52, 0.35],
  [21, 55, 0.35],
  [21, 56, 0.6],
  [21, 59, 0.8],
  [22, 28, 0.8],
  [22, 29, 0.8],
  [22, 33, 0.8],
  [22, 35, 0.8],
  [22, 39, 0.8],
  [22, 51, 0.6],
  [22, 55, 0.55],
  [22, 69, 0.8],
  [23, 43, 0.6],
  [23, 56, 0.6],
  [24, 30, 0.59],
  [24, 32, 0.57],
  [24, 47, 0.8],
  [24, 49, 0.8],
  [24, 69, 0.8],
  [25, 26, 0.8],
  [25, 30, 0.8],
  [25, 31, 0.35],
  [25, 32, 0.8],
  [25, 34, 0.8],
  [25, 36, 0.35],
  [25, 40, 0.35],
  [25, 46, 0.8],
  [25, 48, 0.8],
  [25, 52, 0.8],
  [25, 53, 0.8],
  [25, 54, 0.8],
  [25, 59, 0.8],
  [25, 61, 0.8],
  [25, 62, 0.6],
  [25, 66, 0.8],
  [25, 67, 0.8],
  [25, 68, 0.8],
  [26, 27, 0.61],
  [26, 30, 0.8],
  [26, 46, 0.8],
  [26, 67, 0.8],
  [27, 32, 0.8],
  [27, 36, 0.56],
  [27, 40, 0.8],
  [27, 41, 0.35],
  [27, 43, 0.8],
  [27, 47, 0.35],
  [27, 61, 0.8],
  [28, 33, 0.8],
  [28, 35, 0.8],
  [28, 45, 0.8],
  [28, 51, 0.8],
  [28, 57, 1],
  [28, 63, 0.8],
  [28, 69, 0.8],
  [29, 33, 0.8],
  [29, 35, 0.8],
  [29, 38, 0.8],
  [29, 54, 0.8],
  [30, 34, 0.8],
  [30, 44, 0.8],
  [30, 46, 0.8],
  [31, 67, 0.6],
  [31, 68, 0.35],
  [32, 34, 0.8],
  [32, 41, 0.35],
  [32, 46, 0.8],
  [32, 48, 0.6],
  [32, 50, 0.8],
  [32, 53, 0.8],
  [32, 59, 0.8],
  [32, 60, 0.8],
  [32, 61, 0.8],
  [32, 67, 0.58],
  [33, 39, 0.8],
  [34, 41, 0.6],
  [34, 56, 0.57],
  [34, 61, 0.8],
  [34, 66, 0.8],
  [34, 67, 0.58],
  [34, 68, 0.8],
  [35, 45, 0.56],
  [36, 48, 0.8],
  [36, 65, 0.35],
  [36, 66, 0.35],
  [36, 68, 0.35],
  [37, 38, 0.8],
  [37, 43, 0.6],
  [37, 51, 0.8],
  [38, 51, 0.58],
  [38, 54, 0.8],
  [38, 67, 0.8],
  [39, 44, 0.35],
  [39, 45, 0.56],
  [39, 54, 0.8],
  [39, 55, 0.8],
  [40, 50, 0.6],
  [41, 66, 0.6],
  [41, 68, 0.6],
  [42, 44, 0.35],
  [42, 47, 0.8],
  [42, 49, 0.8],
  [42, 52, 0.8],
  [42, 58, 0.35],
  [43, 61, 0.8],
  [43, 68, 0.6],
  [44, 45, 0.35],
  [44, 47, 0.35],
  [44, 48, 0.35],
  [44, 49, 0.35],
  [44, 50, 0.35],
  [44, 58, 0.35],
  [45, 57, 0.6],
  [45, 62, 0.56],
  [45, 63, 0.55],
  [45, 69, 0.62],
  [46, 59, 0.8],
  [47, 49, 0.8],
  [49, 52, 0.8],
  [49, 54, 0.35],
  [49, 55, 0.35],
  [49, 58, 0.35],
  [49, 66, 0.8],
  [50, 53, 0.8],
  [51, 63, 0.8],
  [52, 58, 0.8],
  [53, 61, 0.8],
  [53, 62, 0.8],
  [54, 59, 0.8],
  [55, 69, 0.55],
  [57, 69, 0.8],
  [59, 61, 0.8],
  [59, 66, 0.8],
  [60, 67, 0.58],
  [62, 66, 0.6],
  [66, 68, 1],
  [66, 69, 0.8],
  [68, 69, 0.8],
];

test("no title sits inside another neighbourhood's outline", () => {
  const nodes = DENSE_CLUSTERS.map((clusterId, index) => ({
    key: `t:${index}`,
    clusterId,
    free: !DENSE_NAMED.has(clusterId),
  }));
  const edges = DENSE_EDGES.map(([from, to, weight]) => ({
    from: `t:${from}`,
    to: `t:${to}`,
    weight,
  }));
  const layout = createLayout(nodes, edges, new Map(), DENSE_NAMED);
  settle(layout);
  const positions = layoutPositions(layout);
  const point = (key: string) => {
    const found = positions.get(key);
    assert.ok(found);
    return found;
  };
  for (const clusterId of DENSE_NAMED) {
    const hull = convexHull(
      nodes
        .filter((node) => node.clusterId === clusterId)
        .map((node) => point(node.key)),
    );
    for (const node of nodes) {
      if (node.clusterId === clusterId) {
        continue;
      }
      assert.ok(
        !insideHull(hull, point(node.key)),
        `${node.key} sits inside cluster ${clusterId}'s outline`,
      );
    }
  }
});

test("free titles take no ring spot and keep beside the titles they link to", () => {
  const { nodes, edges } = fixtureLayoutInput();
  const named = nodes.filter((node) => node.clusterId <= 1);
  const linkedTo = named[0].key;
  const free = [
    { key: "free:linked", clusterId: 50, free: true },
    { key: "free:alone-1", clusterId: 51, free: true },
    { key: "free:alone-2", clusterId: 52, free: true },
  ];
  const layout = createLayout(
    [...named, ...free],
    [
      ...edges.filter(
        (edge) =>
          named.some((node) => node.key === edge.from) &&
          named.some((node) => node.key === edge.to),
      ),
      { from: "free:linked", to: linkedTo, weight: 0.6 },
    ],
  );
  assert.deepEqual(
    [...layout.centers.keys()].sort((a, b) => a - b),
    [...new Set(named.map((node) => node.clusterId))].sort((a, b) => a - b),
  );
  settle(layout);
  const positions = layoutPositions(layout);
  const linkedPoint = positions.get("free:linked");
  const target = positions.get(linkedTo);
  assert.ok(linkedPoint && target);
  const near = Math.hypot(linkedPoint.x - target.x, linkedPoint.y - target.y);
  assert.ok(near < 140, `linked free title is ${near.toFixed(0)} away`);
  // Every title fits inside the frame the view keeps round the ring.
  let extent = 0;
  for (const center of layout.centers.values()) {
    extent = Math.max(extent, Math.abs(center.x), Math.abs(center.y));
  }
  for (const [key, point] of positions) {
    assert.ok(
      Math.max(Math.abs(point.x), Math.abs(point.y)) < extent + 220,
      `${key} at ${point.x.toFixed(0)},${point.y.toFixed(0)} is outside the frame`,
    );
  }
});

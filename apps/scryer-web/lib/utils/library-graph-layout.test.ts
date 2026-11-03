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

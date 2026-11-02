import assert from "node:assert/strict";
import test from "node:test";

import { libraryGraphFixture } from "./library-graph-fixture.ts";
import {
  clusterCenters,
  convexHull,
  createLayout,
  layoutIsSettled,
  layoutLoopDecision,
  layoutPositions,
  stepLayout,
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
        return Math.hypot(now.x - was.x, now.y - was.y);
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

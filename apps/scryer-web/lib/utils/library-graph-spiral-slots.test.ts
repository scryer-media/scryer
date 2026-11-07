import assert from "node:assert/strict";
import test from "node:test";
import type { LibraryGraphEdge } from "../types/library-graph.ts";
import {
  assignRelatedSpiralSlots,
  packRelatedCircles,
  relatedSpiralSlotPasses,
} from "./library-graph-spiral-slots.ts";
import { largeLibraryGraphLayout } from "./library-graph-model.ts";
import type { GraphState } from "./library-graph-merge.ts";

function fixture(count: number) {
  const keys = Array.from(
    { length: count },
    (_, index) => `title:${String(index).padStart(5, "0")}`,
  );
  const slots = keys.map((_, rank) => {
    const angle = rank * Math.PI * (3 - Math.sqrt(5));
    const radius = 24 * Math.sqrt(rank + 0.5);
    return { x: Math.cos(angle) * radius, y: Math.sin(angle) * radius };
  });
  return { keys, slots };
}
const link = (from: string, to: string, weight = 1): LibraryGraphEdge => ({
  from,
  to,
  weight,
  kinds: ["semantic"],
});
const distance = (a: { x: number; y: number }, b: { x: number; y: number }) =>
  Math.hypot(a.x - b.x, a.y - b.y);

test("unrelated titles keep their existing spiral slots, including empty and singleton graphs", () => {
  for (const count of [0, 1, 60]) {
    const { keys, slots } = fixture(count);
    assert.deepEqual(
      [...assignRelatedSpiralSlots(keys, [], slots)],
      keys.map((_, index) => index),
    );
  }
  const { keys, slots } = fixture(60);
  const invalid = [
    link(keys[0], keys[0]),
    link(keys[0], "missing"),
    link(keys[0], keys[1], NaN),
    link(keys[0], keys[2], -1),
  ];
  assert.deepEqual(
    [...assignRelatedSpiralSlots(keys, invalid, slots)],
    keys.map((_, index) => index),
  );
});

test("a heavy link gets a nearby two-dimensional slot ahead of many weak links", () => {
  const { keys, slots } = fixture(100);
  const edges = keys.slice(1, -1).map((key) => link(keys[0], key, 0.01));
  edges.push(link(keys[0], keys[99], 1));
  const assignment = assignRelatedSpiralSlots(keys, edges, slots);
  const a = slots[assignment[0]];
  const b = slots[assignment[99]];
  const nearest = Math.min(
    ...slots.filter((point) => point !== a).map((point) => distance(a, point)),
  );
  assert.ok(distance(a, b) <= nearest * 2);
  assert.ok(distance(a, b) < distance(slots[0], slots[99]) / 4);
  // Consecutive spiral ranks rotate 137 degrees; ordering the linked keys
  // consecutively alone does not guarantee nearby positions.
  assert.ok(distance(a, b) < distance(slots[0], slots[1]));
  assert.equal(new Set(assignment).size, keys.length);
});

test("coarse related groups occupy shorter links on exactly the same point cloud", () => {
  const { keys, slots } = fixture(120);
  const edges: LibraryGraphEdge[] = [];
  for (let group = 0; group < 8; group++) {
    const members = keys.filter((_, index) => index % 8 === group);
    for (let a = 0; a < members.length; a++)
      for (let b = a + 1; b < members.length; b++)
        edges.push(link(members[a], members[b]));
    edges.push(link(members[0], keys[(group + 1) % 8], 0.02));
  }
  const assignment = assignRelatedSpiralSlots(keys, edges, slots);
  assert.deepEqual(
    [...assignment].sort((a, b) => a - b),
    keys.map((_, index) => index),
  );
  const index = new Map(keys.map((key, rank) => [key, rank]));
  const cost = (positions: Int32Array) =>
    edges
      .filter((edge) => edge.weight === 1)
      .reduce(
        (sum, edge) =>
          sum +
          distance(
            slots[positions[index.get(edge.from)!]],
            slots[positions[index.get(edge.to)!]],
          ),
        0,
      );
  const original = Int32Array.from(keys.map((_, rank) => rank));
  assert.ok(cost(assignment) < cost(original) * 0.6);
  const passes = relatedSpiralSlotPasses(keys, edges, slots);
  assert.equal(passes.length, 3);
  const weightedCost = (positions: Int32Array) =>
    edges.reduce(
      (sum, edge) =>
        sum +
        edge.weight *
          distance(
            slots[positions[index.get(edge.from)!]],
            slots[positions[index.get(edge.to)!]],
          ),
      0,
    );
  let previousCost = weightedCost(original);
  for (const pass of passes) {
    assert.equal(new Set(pass).size, keys.length);
    assert.ok(weightedCost(pass) <= previousCost);
    previousCost = weightedCost(pass);
  }
  assert.deepEqual(assignment, passes[2]);
  const reordered = edges
    .map((edge) => ({ ...edge, from: edge.to, to: edge.from }))
    .reverse();
  assert.deepEqual(
    assignRelatedSpiralSlots(keys, reordered, slots),
    assignment,
  );
  assert.deepEqual(
    assignRelatedSpiralSlots(keys, [...reordered, ...edges], slots),
    assignment,
  );
});

test("layout keeps neighbourhood boundaries and overview dots while assigning related titles within each", () => {
  const { keys } = fixture(100);
  const graph: GraphState = {
    clusters: [0, 1].map((id) => ({
      id,
      labelKey: "theme:thriller",
      label: "Fixture",
      medoid: keys[id],
      size: 50,
    })),
    nodes: keys.map((key, index) => ({
      subjectKey: key,
      targetKey: `tmdb:movie:${index + 1}`,
      clusterId: index % 2,
      ghost: false,
    })),
    edges: [link(keys[0], keys[98]), link(keys[1], keys[99])],
  };
  const original = largeLibraryGraphLayout({ ...graph, edges: [] });
  const related = largeLibraryGraphLayout(graph);
  assert.deepEqual(related.regions, original.regions);
  assert.deepEqual(related.hulls, original.hulls);
  assert.deepEqual(related.dots, original.dots);
  assert.equal(related.extent, original.extent);
  assert.deepEqual(
    largeLibraryGraphLayout({
      ...graph,
      nodes: [...graph.nodes].reverse(),
      edges: [...graph.edges].reverse(),
    }),
    related,
  );
  for (const region of related.regions) {
    const points = (layout: typeof related) =>
      graph.nodes
        .filter((node) => node.clusterId === region.id)
        .map((node) => JSON.stringify(layout.positions.get(node.subjectKey)))
        .sort();
    assert.deepEqual(points(related), points(original));
  }
  // A relation crossing two neighbourhoods cannot move a title into the other.
  const crossing = largeLibraryGraphLayout({
    ...graph,
    edges: [link(keys[0], keys[1])],
  });
  for (const node of graph.nodes) {
    const region = crossing.regions.find(
      (region) => region.id === node.clusterId,
    )!;
    const point = crossing.positions.get(node.subjectKey)!;
    assert.ok(
      point.x >= region.left &&
        point.x <= region.right &&
        point.y >= region.top &&
        point.y <= region.bottom,
    );
  }
});

test("related neighbourhoods sit together and bridging titles face each other", () => {
  const { keys } = fixture(160);
  const cluster = (id: number) => ({
    id,
    labelKey: "theme:thriller",
    label: "Fixture",
    medoid: keys[id * 40],
    size: 40,
  });
  const nodes = keys.map((key, index) => ({
    subjectKey: key,
    targetKey: `tmdb:movie:${index + 1}`,
    clusterId: Math.floor(index / 40),
    ghost: false,
  }));
  const edges: LibraryGraphEdge[] = [];
  for (let group = 0; group < 4; group++)
    for (let a = 1; a < 40; a++)
      edges.push(link(keys[group * 40], keys[group * 40 + a], 0.6));
  // Neighbourhoods 0 and 3 share several titles; 1 and 2 relate to nothing
  // outside themselves.
  for (let a = 1; a <= 6; a++) edges.push(link(keys[a], keys[120 + a], 0.8));
  const layout = largeLibraryGraphLayout({
    clusters: [0, 1, 2, 3].map(cluster),
    nodes,
    edges,
  });
  const centre = (id: number) => layout.positions.get(`cluster:${id}`)!;
  const gaps = [1, 2, 3].map((id) => distance(centre(0), centre(id)));
  assert.equal(Math.min(...gaps), gaps[2]);
  // Regions never overlap.
  for (const a of layout.regions)
    for (const b of layout.regions)
      if (a.id < b.id)
        assert.ok(
          distance(centre(a.id), centre(b.id)) >=
            (a.right - a.left) / 2 + (b.right - b.left) / 2,
        );
  const facing = (key: string, own: number, other: number) => {
    const point = layout.positions.get(key)!;
    const from = centre(own);
    const to = centre(other);
    return (
      (point.x - from.x) * (to.x - from.x) +
        (point.y - from.y) * (to.y - from.y) >
      0
    );
  };
  for (let a = 1; a <= 6; a++) {
    assert.ok(facing(keys[a], 0, 3));
    assert.ok(facing(keys[120 + a], 3, 0));
  }
  assert.deepEqual(
    largeLibraryGraphLayout({
      clusters: [3, 2, 1, 0].map(cluster),
      nodes: [...nodes].reverse(),
      edges: edges
        .map((edge) => ({ ...edge, from: edge.to, to: edge.from }))
        .reverse(),
    }),
    layout,
  );
});

test("circle packing places each circle once without overlap", () => {
  const radii = Array.from({ length: 40 }, (_, index) => 20 + (index % 7) * 30);
  const affinity = new Float64Array(radii.length * radii.length);
  for (let a = 0; a < radii.length; a++) {
    const b = (a * 7 + 3) % radii.length;
    if (a === b) continue;
    affinity[a * radii.length + b] += 1;
    affinity[b * radii.length + a] += 1;
  }
  const centres = packRelatedCircles(radii, affinity, 10);
  assert.equal(centres.length, radii.length);
  for (let a = 0; a < radii.length; a++)
    for (let b = a + 1; b < radii.length; b++)
      assert.ok(
        distance(centres[a], centres[b]) >= radii[a] + radii[b] + 10 - 1e-6,
      );
  assert.deepEqual(packRelatedCircles([5], new Float64Array(1), 10), [
    { x: 0, y: 0 },
  ]);
});

test("local refinements bring isolated pairs together after a dense component fills the center", () => {
  const { keys, slots } = fixture(300);
  const edges: LibraryGraphEdge[] = [];
  for (let a = 0; a < 80; a++)
    for (let b = a + 1; b < 80; b++) edges.push(link(keys[a], keys[b], 0.5));
  const pairs = Array.from({ length: 50 }, (_, rank) => [
    80 + rank * 2,
    219 - rank * 2,
  ]);
  for (const [a, b] of pairs) edges.push(link(keys[a], keys[b]));
  const passes = relatedSpiralSlotPasses(keys, edges, slots);
  const longest = passes.map((assignment) =>
    Math.max(
      ...pairs.map(([a, b]) =>
        distance(slots[assignment[a]], slots[assignment[b]]),
      ),
    ),
  );
  // These components only have one edge each. No pair should be stranded
  // across the cloud, and the additional passes must do actual refinement.
  assert.ok(longest[1] < longest[0]);
  assert.ok(longest[2] < longest[1]);
  assert.ok(longest[2] < 60);
  assert.equal(new Set(passes[2]).size, keys.length);
  assert.deepEqual(
    assignRelatedSpiralSlots(keys, [...edges].reverse(), slots),
    passes[2],
  );
});

test("a large hub and disconnected titles consume each slot once", () => {
  const { keys, slots } = fixture(10_000);
  const edges = keys.slice(1, 8_000).map((key) => link(keys[0], key, 0.8));
  const assignment = assignRelatedSpiralSlots(keys, edges, slots);
  assert.equal(new Set(assignment).size, keys.length);
  assert.ok(assignment.every((slot) => slot >= 0 && slot < keys.length));
});

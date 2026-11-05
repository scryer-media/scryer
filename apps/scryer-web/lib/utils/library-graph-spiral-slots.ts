import type { LibraryGraphEdge } from "../types/library-graph.ts";
import type { LayoutPoint } from "./library-graph-layout.ts";
import { clampEdgeWeight, edgePairKey } from "./library-graph-merge.ts";

type SlotTree = {
  left: number;
  right: number;
  top: number;
  bottom: number;
  remaining: number;
  parent?: SlotTree;
  children?: [SlotTree, SlotTree];
  slots?: number[];
};

/** Occupied subtrees are pruned, including when a hub fills its nearby slots. */
function availableSlots(points: readonly LayoutPoint[]) {
  const used = new Uint8Array(points.length);
  const leaves: SlotTree[] = [];
  const build = (indices: number[], parent?: SlotTree): SlotTree => {
    const tree: SlotTree = {
      left: Infinity,
      right: -Infinity,
      top: Infinity,
      bottom: -Infinity,
      remaining: indices.length,
      parent,
    };
    for (const index of indices) {
      const point = points[index];
      tree.left = Math.min(tree.left, point.x);
      tree.right = Math.max(tree.right, point.x);
      tree.top = Math.min(tree.top, point.y);
      tree.bottom = Math.max(tree.bottom, point.y);
    }
    if (indices.length <= 16) {
      tree.slots = indices;
      for (const index of indices) leaves[index] = tree;
    } else {
      const axis = tree.right - tree.left >= tree.bottom - tree.top ? "x" : "y";
      indices.sort((a, b) => points[a][axis] - points[b][axis] || a - b);
      const middle = indices.length >> 1;
      tree.children = [
        build(indices.slice(0, middle), tree),
        build(indices.slice(middle), tree),
      ];
    }
    return tree;
  };
  const root = build(
    Array.from({ length: points.length }, (_, index) => index),
  );
  const boxDistance = (tree: SlotTree, point: LayoutPoint) => {
    const dx = Math.max(tree.left - point.x, 0, point.x - tree.right);
    const dy = Math.max(tree.top - point.y, 0, point.y - tree.bottom);
    return dx * dx + dy * dy;
  };
  return {
    used,
    reset() {
      used.fill(0);
      const reset = (tree: SlotTree): number => {
        tree.remaining =
          tree.slots?.length ??
          tree.children!.reduce((sum, child) => sum + reset(child), 0);
        return tree.remaining;
      };
      reset(root);
    },
    near(point: LayoutPoint) {
      let bucket = root;
      while (bucket.children) {
        const [a, b] = bucket.children;
        bucket = boxDistance(a, point) <= boxDistance(b, point) ? a : b;
      }
      return bucket.slots!;
    },
    take(point: LayoutPoint) {
      // Coarse nearest-bucket descent bounds each lookup to one tree path
      // and at most 16 slots, even for a dense hub or a nearly full spiral.
      let bucket = root;
      while (bucket.children) {
        const [a, b] = bucket.children;
        bucket =
          a.remaining &&
          (!b.remaining || boxDistance(a, point) <= boxDistance(b, point))
            ? a
            : b;
      }
      let best = -1;
      let distance = Infinity;
      for (const index of bucket.slots!) {
        if (used[index]) continue;
        const dx = points[index].x - point.x;
        const dy = points[index].y - point.y;
        const candidate = dx * dx + dy * dy;
        if (candidate < distance || (candidate === distance && index < best)) {
          best = index;
          distance = candidate;
        }
      }
      used[best] = 1;
      let tree: SlotTree | undefined = leaves[best];
      while (tree) {
        tree.remaining--;
        tree = tree.parent;
      }
      return best;
    },
  };
}

/**
 * Assign canonical title keys to a fixed spiral's slots. The strongest ties
 * grow first; each title takes a free slot near its neighbours' weighted
 * centre. Two bounded local-swap passes then shorten incident relationships
 * without changing any spiral points or accepting a worse assignment.
 */
export function relatedSpiralSlotPasses(
  keys: readonly string[],
  edges: readonly LibraryGraphEdge[],
  slots: readonly LayoutPoint[],
): Int32Array[] {
  const count = keys.length;
  let assignment = Int32Array.from({ length: count }, (_, index) => index);
  if (count < 2 || edges.length === 0)
    return [assignment, assignment, assignment];
  const indexByKey = new Map(keys.map((key, index) => [key, index]));
  const byPair = new Map<string, { a: number; b: number; weight: number }>();
  for (const edge of edges) {
    const from = indexByKey.get(edge.from);
    const to = indexByKey.get(edge.to);
    const weight = clampEdgeWeight(edge.weight);
    if (from === undefined || to === undefined || from === to || weight === 0)
      continue;
    const pair = edgePairKey(edge.from, edge.to);
    if (weight > (byPair.get(pair)?.weight ?? 0))
      byPair.set(pair, {
        a: Math.min(from, to),
        b: Math.max(from, to),
        weight,
      });
  }
  if (!byPair.size) return [assignment, assignment, assignment];
  // Canonical accumulation order makes both floating-point scores and ties
  // independent of the order in which upstream nodes and edges arrived.
  const links = [...byPair.values()].sort((a, b) => a.a - b.a || a.b - b.b);
  const degree = new Float64Array(count);
  const offsets = new Uint32Array(count + 1);
  for (const edge of links) {
    degree[edge.a] += edge.weight;
    degree[edge.b] += edge.weight;
    offsets[edge.a + 1]++;
    offsets[edge.b + 1]++;
  }
  for (let index = 1; index <= count; index++)
    offsets[index] += offsets[index - 1];
  const cursor = offsets.slice();
  const neighbours = new Uint32Array(links.length * 2);
  const weights = new Float64Array(links.length * 2);
  for (const edge of links) {
    for (const [from, to] of [
      [edge.a, edge.b],
      [edge.b, edge.a],
    ]) {
      const at = cursor[from]++;
      neighbours[at] = to;
      weights[at] = edge.weight;
    }
  }

  const support = new Float64Array(count);
  const strongest = new Float64Array(count);
  const centreX = new Float64Array(count);
  const centreY = new Float64Array(count);
  // An indexed heap holds at most one entry per title, rather than one entry
  // per score update in a dense neighbourhood.
  const heap: number[] = [];
  const heapIndex = new Int32Array(count).fill(-1);
  const preferred = (a: number, b: number) =>
    strongest[a] > strongest[b] ||
    (strongest[a] === strongest[b] &&
      (support[a] > support[b] ||
        (support[a] === support[b] &&
          (degree[a] > degree[b] || (degree[a] === degree[b] && a < b)))));
  const swap = (a: number, b: number) => {
    [heap[a], heap[b]] = [heap[b], heap[a]];
    heapIndex[heap[a]] = a;
    heapIndex[heap[b]] = b;
  };
  const raise = (node: number) => {
    let index = heapIndex[node];
    if (index < 0) {
      index = heap.length;
      heap.push(node);
      heapIndex[node] = index;
    }
    while (index > 0) {
      const parent = (index - 1) >> 1;
      if (!preferred(heap[index], heap[parent])) break;
      swap(index, parent);
      index = parent;
    }
  };
  const pop = () => {
    const node = heap[0];
    const last = heap.pop()!;
    heapIndex[node] = -1;
    if (heap.length) {
      heap[0] = last;
      heapIndex[last] = 0;
      let index = 0;
      while (index * 2 + 1 < heap.length) {
        let child = index * 2 + 1;
        if (child + 1 < heap.length && preferred(heap[child + 1], heap[child]))
          child++;
        if (!preferred(heap[child], heap[index])) break;
        swap(child, index);
        index = child;
      }
    }
    return node;
  };

  const seeds = Array.from({ length: count }, (_, index) => index).sort(
    (a, b) => degree[b] - degree[a] || a - b,
  );
  const available = availableSlots(slots);
  const score = (candidate: Int32Array) =>
    links.reduce((sum, edge) => {
      const a = slots[candidate[edge.a]];
      const b = slots[candidate[edge.b]];
      return sum + edge.weight * ((a.x - b.x) ** 2 + (a.y - b.y) ** 2);
    }, 0);
  const original = assignment;
  assignment = new Int32Array(count).fill(-1);
  let seed = 0;
  let firstFreeSlot = 0;
  for (let placed = 0; placed < count; placed++) {
    while (assignment[seeds[seed]] >= 0) seed++;
    const node = heap.length ? pop() : seeds[seed];
    while (available.used[firstFreeSlot]) firstFreeSlot++;
    const target =
      support[node] > 0
        ? { x: centreX[node] / support[node], y: centreY[node] / support[node] }
        : slots[firstFreeSlot];
    const slot = available.take(target);
    assignment[node] = slot;
    const point = slots[slot];
    for (let at = offsets[node]; at < offsets[node + 1]; at++) {
      const neighbour = neighbours[at];
      if (assignment[neighbour] >= 0) continue;
      const weight = weights[at];
      support[neighbour] += weight;
      strongest[neighbour] = Math.max(strongest[neighbour], weight);
      centreX[neighbour] += point.x * weight;
      centreY[neighbour] += point.y * weight;
      raise(neighbour);
    }
  }
  if (score(original) < score(assignment)) assignment = original;
  const passes = [assignment.slice()];
  const occupant = new Int32Array(count);
  for (let node = 0; node < count; node++) occupant[assignment[node]] = node;
  const distanceSquared = (a: LayoutPoint, b: LayoutPoint) =>
    (a.x - b.x) ** 2 + (a.y - b.y) ** 2;
  const incidentDelta = (node: number, other: number, next: LayoutPoint) => {
    const current = slots[assignment[node]];
    let delta = 0;
    for (let at = offsets[node]; at < offsets[node + 1]; at++) {
      const neighbour = neighbours[at];
      // The edge between swapped endpoints keeps the same length.
      if (neighbour === other) continue;
      const point = slots[assignment[neighbour]];
      delta +=
        weights[at] *
        (distanceSquared(next, point) - distanceSquared(current, point));
    }
    return delta;
  };
  for (let pass = 0; pass < 2; pass++) {
    for (const node of seeds) {
      const edgeCount = offsets[node + 1] - offsets[node];
      if (!edgeCount) continue;
      let x = 0,
        y = 0,
        strongestAt = offsets[node];
      for (let at = offsets[node]; at < offsets[node + 1]; at++) {
        const point = slots[assignment[neighbours[at]]];
        x += point.x * weights[at];
        y += point.y * weights[at];
        if (weights[at] > weights[strongestAt]) strongestAt = at;
      }
      // At most two nearby buckets (32 slots), with a fixed incident-edge
      // budget. Hubs cannot turn refinement into an all-pairs operation.
      const candidates = new Set([
        ...available.near({ x: x / degree[node], y: y / degree[node] }),
        ...available.near(slots[assignment[neighbours[strongestAt]]]),
      ]);
      let budget = 2048;
      let bestDelta = -1e-6;
      let bestSlot = -1;
      const current = slots[assignment[node]];
      for (const slot of candidates) {
        const other = occupant[slot];
        if (other === node) continue;
        const work = edgeCount + offsets[other + 1] - offsets[other];
        if (work > budget) continue;
        budget -= work;
        const delta =
          incidentDelta(node, other, slots[slot]) +
          incidentDelta(other, node, current);
        if (delta < bestDelta) {
          bestDelta = delta;
          bestSlot = slot;
        }
      }
      if (bestSlot >= 0) {
        const oldSlot = assignment[node];
        const other = occupant[bestSlot];
        assignment[node] = bestSlot;
        assignment[other] = oldSlot;
        occupant[bestSlot] = node;
        occupant[oldSlot] = other;
      }
    }
    passes.push(assignment.slice());
  }
  return passes;
}

export function assignRelatedSpiralSlots(
  keys: readonly string[],
  edges: readonly LibraryGraphEdge[],
  slots: readonly LayoutPoint[],
): Int32Array {
  return relatedSpiralSlotPasses(keys, edges, slots)[2];
}

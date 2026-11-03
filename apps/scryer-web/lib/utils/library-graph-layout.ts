// A deterministic force layout for the library graph. Clusters start around a
// circle in cluster id order and every node starts near its cluster's centre,
// jittered by a hash of its key, so the same graph always lays out the same
// way. Nodes that already have a position keep it: when the node set changes,
// the layout settles from where it was instead of starting over.

export type LayoutPoint = { x: number; y: number };

export type LayoutNodeInput = {
  key: string;
  clusterId: number;
};

export type LayoutEdgeInput = {
  from: string;
  to: string;
  weight: number;
};

export type LayoutState = {
  keys: string[];
  clusterIds: number[];
  x: Float64Array;
  y: Float64Array;
  centers: Map<number, LayoutPoint>;
  edges: Array<{ a: number; b: number; weight: number }>;
  /** Cooling factor: how far a node may move in one step. */
  temperature: number;
};

const CLUSTER_RING_RADIUS = 260;
const START_JITTER = 70;
const REPULSION_RANGE = 90;
const REPULSION = 900;
const SPRING_LENGTH = 34;
const SPRING = 0.04;
const CLUSTER_GRAVITY = 0.012;
const START_TEMPERATURE = 24;
const SETTLE_TEMPERATURE = 6;
const COOLING = 0.97;
const MIN_TEMPERATURE = 0.05;

/** A stable 32-bit hash of a string. */
export function hashString(value: string): number {
  let hash = 0x811c9dc5;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193);
  }
  return hash >>> 0;
}

function unitFromHash(hash: number): number {
  return hash / 4294967296;
}

/** Where each cluster's nodes gather: a circle, in cluster id order. */
export function clusterCenters(
  clusterIds: Iterable<number>,
): Map<number, LayoutPoint> {
  const ids = Array.from(new Set(clusterIds)).sort((a, b) => a - b);
  const centers = new Map<number, LayoutPoint>();
  if (ids.length === 1) {
    centers.set(ids[0], { x: 0, y: 0 });
    return centers;
  }
  const radius = CLUSTER_RING_RADIUS * Math.max(1, Math.sqrt(ids.length / 3));
  ids.forEach((id, position) => {
    const angle = (2 * Math.PI * position) / ids.length - Math.PI / 2;
    centers.set(id, {
      x: Math.cos(angle) * radius,
      y: Math.sin(angle) * radius,
    });
  });
  return centers;
}

/**
 * A layout over the given nodes and edges. Nodes found in `previous` keep that
 * position; the rest start beside their cluster's centre. When most nodes
 * already had a position the layout starts cool, so the next steps settle it
 * rather than shake it apart.
 *
 * `allClusterIds` names every cluster of the whole graph, before any filter.
 * Cluster centres come from it, so filtering a whole cluster out leaves every
 * other cluster where it was.
 */
export function createLayout(
  nodes: readonly LayoutNodeInput[],
  edges: readonly LayoutEdgeInput[],
  previous: ReadonlyMap<string, LayoutPoint> = new Map(),
  allClusterIds: Iterable<number> = [],
): LayoutState {
  const centers = clusterCenters([
    ...allClusterIds,
    ...nodes.map((node) => node.clusterId),
  ]);
  const keys = nodes.map((node) => node.key);
  const clusterIds = nodes.map((node) => node.clusterId);
  const x = new Float64Array(nodes.length);
  const y = new Float64Array(nodes.length);
  const indexByKey = new Map<string, number>();
  let kept = 0;
  nodes.forEach((node, index) => {
    indexByKey.set(node.key, index);
    const prior = previous.get(node.key);
    if (prior) {
      x[index] = prior.x;
      y[index] = prior.y;
      kept += 1;
      return;
    }
    const center = centers.get(node.clusterId) ?? { x: 0, y: 0 };
    const hash = hashString(node.key);
    const angle = unitFromHash(hash) * 2 * Math.PI;
    const distance =
      START_JITTER * (0.2 + unitFromHash(hashString(`r:${node.key}`)));
    x[index] = center.x + Math.cos(angle) * distance;
    y[index] = center.y + Math.sin(angle) * distance;
  });
  const layoutEdges: LayoutState["edges"] = [];
  for (const edge of edges) {
    const a = indexByKey.get(edge.from);
    const b = indexByKey.get(edge.to);
    if (a === undefined || b === undefined || a === b) {
      continue;
    }
    layoutEdges.push({ a, b, weight: Math.max(0, edge.weight) });
  }
  const settling = nodes.length > 0 && kept / nodes.length >= 0.5;
  return {
    keys,
    clusterIds,
    x,
    y,
    centers,
    edges: layoutEdges,
    temperature: settling ? SETTLE_TEMPERATURE : START_TEMPERATURE,
  };
}

/** Whether the layout has cooled to rest. */
export function layoutIsSettled(state: LayoutState): boolean {
  return state.temperature <= MIN_TEMPERATURE;
}

export type LayoutLoopDecision = "step" | "pause" | "done";

/**
 * What the animation loop does next. A settled (or missing) layout is done.
 * A container with no width or height (a hidden drawer, a collapsed pane)
 * pauses the loop; the next non-zero size resumes it from where it stopped.
 */
export function layoutLoopDecision(
  container: { width: number; height: number },
  layout: LayoutState | null,
): LayoutLoopDecision {
  if (!layout || layoutIsSettled(layout)) {
    return "done";
  }
  return container.width > 0 && container.height > 0 ? "step" : "pause";
}

/**
 * Advances the layout by up to `iterations` steps and returns how many it
 * ran; a settled layout runs none. Repulsion only acts within a short range,
 * found through a grid, so a step stays linear in the node count.
 */
export function stepLayout(state: LayoutState, iterations: number): number {
  const count = state.keys.length;
  const dx = new Float64Array(count);
  const dy = new Float64Array(count);
  let ran = 0;
  for (let step = 0; step < iterations; step += 1) {
    if (layoutIsSettled(state)) {
      break;
    }
    dx.fill(0);
    dy.fill(0);

    const grid = new Map<string, number[]>();
    for (let index = 0; index < count; index += 1) {
      const cell = `${Math.floor(state.x[index] / REPULSION_RANGE)}:${Math.floor(state.y[index] / REPULSION_RANGE)}`;
      const bucket = grid.get(cell);
      if (bucket) {
        bucket.push(index);
      } else {
        grid.set(cell, [index]);
      }
    }
    for (let index = 0; index < count; index += 1) {
      const cx = Math.floor(state.x[index] / REPULSION_RANGE);
      const cy = Math.floor(state.y[index] / REPULSION_RANGE);
      for (let ox = -1; ox <= 1; ox += 1) {
        for (let oy = -1; oy <= 1; oy += 1) {
          const bucket = grid.get(`${cx + ox}:${cy + oy}`);
          if (!bucket) {
            continue;
          }
          for (const other of bucket) {
            if (other <= index) {
              continue;
            }
            let ddx = state.x[index] - state.x[other];
            let ddy = state.y[index] - state.y[other];
            let distanceSq = ddx * ddx + ddy * ddy;
            if (distanceSq === 0) {
              // Coincident nodes part along a direction fixed by their order.
              ddx = 0.01 * (1 + (index % 7));
              ddy = 0.01 * (1 + (other % 5));
              distanceSq = ddx * ddx + ddy * ddy;
            }
            if (distanceSq > REPULSION_RANGE * REPULSION_RANGE) {
              continue;
            }
            const force = REPULSION / distanceSq;
            const distance = Math.sqrt(distanceSq);
            const fx = (ddx / distance) * force;
            const fy = (ddy / distance) * force;
            dx[index] += fx;
            dy[index] += fy;
            dx[other] -= fx;
            dy[other] -= fy;
          }
        }
      }
    }

    for (const edge of state.edges) {
      const ddx = state.x[edge.b] - state.x[edge.a];
      const ddy = state.y[edge.b] - state.y[edge.a];
      const distance = Math.sqrt(ddx * ddx + ddy * ddy) || 0.01;
      const force = SPRING * edge.weight * (distance - SPRING_LENGTH);
      const fx = (ddx / distance) * force;
      const fy = (ddy / distance) * force;
      dx[edge.a] += fx;
      dy[edge.a] += fy;
      dx[edge.b] -= fx;
      dy[edge.b] -= fy;
    }

    for (let index = 0; index < count; index += 1) {
      const center = state.centers.get(state.clusterIds[index]) ?? {
        x: 0,
        y: 0,
      };
      dx[index] += (center.x - state.x[index]) * CLUSTER_GRAVITY;
      dy[index] += (center.y - state.y[index]) * CLUSTER_GRAVITY;
      const length = Math.sqrt(dx[index] ** 2 + dy[index] ** 2);
      if (length > 0) {
        const limited = Math.min(length, state.temperature);
        state.x[index] += (dx[index] / length) * limited;
        state.y[index] += (dy[index] / length) * limited;
      }
    }
    state.temperature *= COOLING;
    ran += 1;
  }
  return ran;
}

export function layoutPositions(state: LayoutState): Map<string, LayoutPoint> {
  const positions = new Map<string, LayoutPoint>();
  state.keys.forEach((key, index) => {
    positions.set(key, { x: state.x[index], y: state.y[index] });
  });
  return positions;
}

function cross(o: LayoutPoint, a: LayoutPoint, b: LayoutPoint): number {
  return (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x);
}

/** The convex hull of the points, counter-clockwise, without repeats. */
export function convexHull(points: readonly LayoutPoint[]): LayoutPoint[] {
  const sorted = [...points].sort((a, b) => a.x - b.x || a.y - b.y);
  const unique = sorted.filter(
    (point, index) =>
      index === 0 ||
      point.x !== sorted[index - 1].x ||
      point.y !== sorted[index - 1].y,
  );
  if (unique.length < 3) {
    return unique;
  }
  const lower: LayoutPoint[] = [];
  for (const point of unique) {
    while (
      lower.length >= 2 &&
      cross(lower[lower.length - 2], lower[lower.length - 1], point) <= 0
    ) {
      lower.pop();
    }
    lower.push(point);
  }
  const upper: LayoutPoint[] = [];
  for (let index = unique.length - 1; index >= 0; index -= 1) {
    const point = unique[index];
    while (
      upper.length >= 2 &&
      cross(upper[upper.length - 2], upper[upper.length - 1], point) <= 0
    ) {
      upper.pop();
    }
    upper.push(point);
  }
  lower.pop();
  upper.pop();
  return [...lower, ...upper];
}

/** The most rings a focused view spreads its neighbours over. */
const MAX_EGO_RINGS = 6;

/**
 * Positions for a focused node and its neighbours: the node at the origin and
 * each group around it, inner groups first, in the order given. A group is a
 * list of sections; each section keeps to one arc of the circle, with an empty
 * slot between sections. A group too crowded for one ring at `minSpacing`
 * apart spreads over several, its nodes stepping between them, and every
 * group starts on a ring of its own. Rings are evenly spaced out to
 * `outerRadius`; with more neighbours than the rings can hold at that
 * spacing, the outermost ring takes the rest.
 */
export function egoLayout(
  center: string,
  groups: readonly (readonly (readonly string[])[])[],
  outerRadius: number,
  minSpacing: number,
): Map<string, LayoutPoint> {
  const positions = new Map<string, LayoutPoint>([[center, { x: 0, y: 0 }]]);
  const placed = groups
    .map((sections) =>
      sections
        .map((section) => section.filter((key) => key !== center))
        .filter((section) => section.length > 0),
    )
    .filter((sections) => sections.length > 0);
  if (placed.length === 0) {
    return positions;
  }
  // Slots per group: its nodes and one gap after each section, when it has
  // more than one.
  const slotsOf = (sections: readonly (readonly string[])[]) =>
    sections.reduce((total, section) => total + section.length, 0) +
    (sections.length > 1 ? sections.length : 0);
  const slots = placed.map(slotsOf);
  // The fewest rings that hold every group at the spacing asked for.
  let ringsPerGroup: number[] = [];
  let totalRings = 0;
  for (let count = placed.length; count <= MAX_EGO_RINGS; count += 1) {
    const step = outerRadius / count;
    ringsPerGroup = [];
    let ring = 0;
    for (const size of slots) {
      let room = 0;
      let used = 0;
      while (room < size) {
        ring += 1;
        used += 1;
        // Only at the most rings allowed does the outer ring take the rest.
        room +=
          ring >= count && count >= MAX_EGO_RINGS
            ? size
            : Math.max(
                1,
                Math.floor(
                  (2 * Math.PI * step * ring) / Math.max(1, minSpacing),
                ),
              );
      }
      ringsPerGroup.push(used);
    }
    totalRings = ring;
    if (ring <= count) {
      break;
    }
  }
  const step = outerRadius / Math.max(1, totalRings);
  let firstRing = 0;
  placed.forEach((sections, groupIndex) => {
    const rings = ringsPerGroup[groupIndex];
    const size = slots[groupIndex];
    // Alternate groups start half a slot round, so their nodes interleave.
    const offset = groupIndex % 2 === 0 ? 0 : Math.PI / size;
    let slot = 0;
    for (const section of sections) {
      for (const key of section) {
        const radius = step * (firstRing + 1 + (slot % rings));
        const angle = -Math.PI / 2 + offset + (2 * Math.PI * slot) / size;
        positions.set(key, {
          x: Math.cos(angle) * radius,
          y: Math.sin(angle) * radius,
        });
        slot += 1;
      }
      if (sections.length > 1) {
        slot += 1;
      }
    }
    firstRing += rings;
  });
  return positions;
}

/** Ease-out cubic over [0, 1]. */
export function easeOutCubic(progress: number): number {
  const clamped = Math.min(1, Math.max(0, progress));
  return 1 - (1 - clamped) ** 3;
}

/**
 * Each target point a fraction of the way from its start, eased. A target
 * with no start is already there.
 */
export function tweenPositions(
  from: ReadonlyMap<string, LayoutPoint>,
  to: ReadonlyMap<string, LayoutPoint>,
  progress: number,
): Map<string, LayoutPoint> {
  const eased = easeOutCubic(progress);
  const positions = new Map<string, LayoutPoint>();
  for (const [key, target] of to) {
    const start = from.get(key) ?? target;
    positions.set(key, {
      x: start.x + (target.x - start.x) * eased,
      y: start.y + (target.y - start.y) * eased,
    });
  }
  return positions;
}

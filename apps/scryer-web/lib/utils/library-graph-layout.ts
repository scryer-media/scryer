// A deterministic force layout for the library graph. Clusters start around a
// circle in cluster id order and every node starts near its cluster's centre,
// jittered by a hash of its key, so the same graph always lays out the same
// way. Nodes that already have a position keep it: when the node set changes,
// the layout settles from where it was instead of starting over.

export type LayoutPoint = { x: number; y: number };

export type LayoutNodeInput = {
  key: string;
  clusterId: number;
  /**
   * A title in no named neighbourhood. It takes no spot on the ring and keeps
   * to its own links instead, so it never drags a ring spot of its own out
   * past the named neighbourhoods.
   */
  free?: boolean;
};

export type LayoutEdgeInput = {
  from: string;
  to: string;
  weight: number;
};

export type LayoutState = {
  keys: string[];
  clusterIds: number[];
  /** 1 for a title in no named neighbourhood; see `LayoutNodeInput.free`. */
  free: Uint8Array;
  x: Float64Array;
  y: Float64Array;
  centers: Map<number, LayoutPoint>;
  edges: Array<{ a: number; b: number; weight: number }>;
  /** Cooling factor: how far a node may move in one step. */
  temperature: number;
  /** Steps run so far, which paces the nearest-linked search. */
  steps: number;
  /** For each unlinked title, the linked title it rests beside, or -1. */
  nearestLinked: Int32Array;
};

// Keep sparse neighbourhoods near the centre; hull separation makes room for
// larger groups without reserving empty space around every small one.
const CLUSTER_RING_RADIUS = 110;
/** How much wider than tall the ring of clusters is. */
export const CLUSTER_RING_WIDTH = 1.8;
const START_JITTER = 70;
const REPULSION_RANGE = 90;
const REPULSION = 900;
const SPRING_LENGTH = 34;
const SPRING = 0.04;
/** Pull toward the cluster's own titles, so a weakly linked one stays near. */
const CLUSTER_GRAVITY = 0.04;
/** Pull toward the cluster's spot on the ring, keeping clusters apart. */
const RING_GRAVITY = 0.012;
/**
 * How much a link between two named neighbourhoods pulls, against a link
 * inside one. A few links across would otherwise drag two neighbourhoods into
 * one another.
 */
const CROSS_CLUSTER_SPRING = 0.25;
/** Pull on a free title toward the titles it links to. */
const FREE_GRAVITY = 0.05;
/** A faint pull on every free title toward the middle of the graph. */
const CENTRE_GRAVITY = 0.004;
/**
 * Room kept between a neighbourhood's outline and any title not in it, in
 * layout units; the drawn outline is padded past its outer titles.
 */
const HULL_MARGIN = 22;
/** How hard a title inside another neighbourhood's outline is pushed out. */
const HULL_PUSH = 0.35;
/** Steps between fresh searches for an unlinked title's nearest linked one. */
const NEAREST_LINKED_REFRESH_STEPS = 10;
/** Grid cell size for finding the outlines near a title. */
const OUTLINE_CELL = 120;
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

/**
 * Where each cluster's nodes gather: a wide oval, in cluster id order, since
 * the screens the graph is drawn on are wider than they are tall.
 */
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
      x: Math.cos(angle) * radius * CLUSTER_RING_WIDTH,
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
  // Only named neighbourhoods take a spot on the ring.
  const centers = clusterCenters([
    ...allClusterIds,
    ...nodes.filter((node) => !node.free).map((node) => node.clusterId),
  ]);
  const keys = nodes.map((node) => node.key);
  const clusterIds = nodes.map((node) => node.clusterId);
  const free = Uint8Array.from(nodes, (node) => (node.free ? 1 : 0));
  const x = new Float64Array(nodes.length);
  const y = new Float64Array(nodes.length);
  const indexByKey = new Map<string, number>();
  let kept = 0;
  // A title with no links, or in no named neighbourhood, starts in the middle:
  // it has no ring spot, or one too far to travel before the layout cools.
  const present = new Set(keys);
  const linkedKeys = new Set<string>();
  for (const edge of edges) {
    if (
      edge.from !== edge.to &&
      present.has(edge.from) &&
      present.has(edge.to)
    ) {
      linkedKeys.add(edge.from);
      linkedKeys.add(edge.to);
    }
  }
  nodes.forEach((node, index) => {
    indexByKey.set(node.key, index);
    const prior = previous.get(node.key);
    if (prior) {
      x[index] = prior.x;
      y[index] = prior.y;
      kept += 1;
      return;
    }
    const center =
      node.free || (linkedKeys.size > 0 && !linkedKeys.has(node.key))
        ? { x: 0, y: 0 }
        : (centers.get(node.clusterId) ?? { x: 0, y: 0 });
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
    free,
    x,
    y,
    centers,
    edges: layoutEdges,
    temperature: settling ? SETTLE_TEMPERATURE : START_TEMPERATURE,
    steps: 0,
    nearestLinked: new Int32Array(nodes.length).fill(-1),
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
  if (layoutIsSettled(state)) {
    return 0;
  }
  const count = state.keys.length;
  const dx = new Float64Array(count);
  const dy = new Float64Array(count);
  const neighbourX = new Float64Array(count);
  const neighbourY = new Float64Array(count);
  const neighbourWeight = new Float64Array(count);
  const linked = new Uint8Array(count);
  for (const edge of state.edges) {
    linked[edge.a] = 1;
    linked[edge.b] = 1;
  }
  const linkedIndexes: number[] = [];
  for (let index = 0; index < count; index += 1) {
    if (linked[index]) {
      linkedIndexes.push(index);
    }
  }
  let ran = 0;
  // The forces act in a space squeezed across by the ring's width, so the
  // layout they settle is round there and comes out that much wider than
  // tall, to suit wide screens.
  for (let index = 0; index < count; index += 1) {
    state.x[index] /= CLUSTER_RING_WIDTH;
  }
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
      const across =
        !state.free[edge.a] &&
        !state.free[edge.b] &&
        state.clusterIds[edge.a] !== state.clusterIds[edge.b];
      const force =
        SPRING *
        edge.weight *
        (across ? CROSS_CLUSTER_SPRING : 1) *
        (distance - SPRING_LENGTH);
      const fx = (ddx / distance) * force;
      const fy = (ddy / distance) * force;
      dx[edge.a] += fx;
      dy[edge.a] += fy;
      dx[edge.b] -= fx;
      dy[edge.b] -= fy;
    }

    // A title with no links at all is drawn to the nearest linked title
    // rather than to a ring spot, which the linked titles have been pulled
    // well inside of, so it rests on the edge of the graph instead of off on
    // its own. The search is linear, so it reruns only every few steps.
    if (state.steps % NEAREST_LINKED_REFRESH_STEPS === 0) {
      for (let index = 0; index < count; index += 1) {
        if (linked[index]) {
          continue;
        }
        let best = -1;
        let bestDistance = Number.POSITIVE_INFINITY;
        for (const other of linkedIndexes) {
          const distance =
            (state.x[other] - state.x[index]) ** 2 +
            (state.y[other] - state.y[index]) ** 2;
          if (distance < bestDistance) {
            bestDistance = distance;
            best = other;
          }
        }
        state.nearestLinked[index] = best;
      }
    }

    // Where each free title's links pull it: the weighted middle of the
    // titles it links to.
    neighbourX.fill(0);
    neighbourY.fill(0);
    neighbourWeight.fill(0);
    for (const edge of state.edges) {
      const weight = edge.weight + 0.01;
      if (state.free[edge.a]) {
        neighbourX[edge.a] += state.x[edge.b] * weight;
        neighbourY[edge.a] += state.y[edge.b] * weight;
        neighbourWeight[edge.a] += weight;
      }
      if (state.free[edge.b]) {
        neighbourX[edge.b] += state.x[edge.a] * weight;
        neighbourY[edge.b] += state.y[edge.a] * weight;
        neighbourWeight[edge.b] += weight;
      }
    }

    // Each named neighbourhood's middle and reach this step. A title drawn
    // only to a fixed ring spot would park there whenever its few links are
    // weak, far from the rest of its neighbourhood.
    const middles = new Map<
      number,
      { x: number; y: number; n: number; reach: number; members: number[] }
    >();
    for (let index = 0; index < count; index += 1) {
      if (state.free[index]) {
        continue;
      }
      const id = state.clusterIds[index];
      const middle = middles.get(id);
      if (middle) {
        middle.x += state.x[index];
        middle.y += state.y[index];
        middle.n += 1;
        middle.members.push(index);
      } else {
        middles.set(id, {
          x: state.x[index],
          y: state.y[index],
          n: 1,
          reach: 0,
          members: [index],
        });
      }
    }
    for (const middle of middles.values()) {
      middle.x /= middle.n;
      middle.y /= middle.n;
      for (const member of middle.members) {
        middle.reach = Math.max(
          middle.reach,
          Math.hypot(state.x[member] - middle.x, state.y[member] - middle.y),
        );
      }
    }

    // No title sits inside another neighbourhood's outline. The outline lies
    // within the circle round the neighbourhood's middle out to its farthest
    // title, so a title inside that circle, plus a margin, is pushed out,
    // and the neighbourhood is pushed back as much, spread over its titles.
    // The circles are found through a grid, so this stays near linear.
    const outlineGrid = new Map<string, number[]>();
    const outlines = [...middles.entries()];
    outlines.forEach(([, middle], outline) => {
      const limit = middle.reach + HULL_MARGIN;
      const minX = Math.floor((middle.x - limit) / OUTLINE_CELL);
      const maxX = Math.floor((middle.x + limit) / OUTLINE_CELL);
      const minY = Math.floor((middle.y - limit) / OUTLINE_CELL);
      const maxY = Math.floor((middle.y + limit) / OUTLINE_CELL);
      for (let cx = minX; cx <= maxX; cx += 1) {
        for (let cy = minY; cy <= maxY; cy += 1) {
          const cell = `${cx}:${cy}`;
          const bucket = outlineGrid.get(cell);
          if (bucket) {
            bucket.push(outline);
          } else {
            outlineGrid.set(cell, [outline]);
          }
        }
      }
    });
    for (let index = 0; index < count; index += 1) {
      const bucket = outlineGrid.get(
        `${Math.floor(state.x[index] / OUTLINE_CELL)}:${Math.floor(state.y[index] / OUTLINE_CELL)}`,
      );
      if (!bucket) {
        continue;
      }
      for (const outline of bucket) {
        const [id, middle] = outlines[outline];
        if (!state.free[index] && state.clusterIds[index] === id) {
          continue;
        }
        let ddx = state.x[index] - middle.x;
        let ddy = state.y[index] - middle.y;
        let distance = Math.sqrt(ddx * ddx + ddy * ddy);
        const limit = middle.reach + HULL_MARGIN;
        if (distance >= limit) {
          continue;
        }
        if (distance === 0) {
          ddx = 0.01 * (1 + (index % 7));
          ddy = 0.01 * (1 + (outline % 5));
          distance = Math.sqrt(ddx * ddx + ddy * ddy);
        }
        const push = (limit - distance) * HULL_PUSH;
        const fx = (ddx / distance) * push;
        const fy = (ddy / distance) * push;
        dx[index] += fx;
        dy[index] += fy;
        for (const member of middle.members) {
          dx[member] -= fx / middle.n;
          dy[member] -= fy / middle.n;
        }
      }
    }

    for (let index = 0; index < count; index += 1) {
      const nearest = linked[index] ? -1 : state.nearestLinked[index];
      if (state.free[index]) {
        // A free title keeps to its links, or beside the nearest linked
        // title, and drifts faintly toward the middle of the graph.
        if (neighbourWeight[index] > 0) {
          const towardX = neighbourX[index] / neighbourWeight[index];
          const towardY = neighbourY[index] / neighbourWeight[index];
          dx[index] += (towardX - state.x[index]) * FREE_GRAVITY;
          dy[index] += (towardY - state.y[index]) * FREE_GRAVITY;
        } else if (nearest >= 0) {
          dx[index] += (state.x[nearest] - state.x[index]) * CLUSTER_GRAVITY;
          dy[index] += (state.y[nearest] - state.y[index]) * CLUSTER_GRAVITY;
        }
        dx[index] -= state.x[index] * CENTRE_GRAVITY;
        dy[index] -= state.y[index] * CENTRE_GRAVITY;
      } else {
        const ring = state.centers.get(state.clusterIds[index]);
        const center =
          nearest >= 0
            ? { x: state.x[nearest], y: state.y[nearest] }
            : ring
              ? { x: ring.x / CLUSTER_RING_WIDTH, y: ring.y }
              : { x: 0, y: 0 };
        const pull = nearest >= 0 ? CLUSTER_GRAVITY : RING_GRAVITY;
        const middle = middles.get(state.clusterIds[index]);
        if (middle && middle.n > 1) {
          dx[index] += (middle.x - state.x[index]) * CLUSTER_GRAVITY;
          dy[index] += (middle.y - state.y[index]) * CLUSTER_GRAVITY;
        }
        dx[index] += (center.x - state.x[index]) * pull;
        dy[index] += (center.y - state.y[index]) * pull;
      }
      const length = Math.sqrt(dx[index] ** 2 + dy[index] ** 2);
      if (length > 0) {
        const limited = Math.min(length, state.temperature);
        state.x[index] += (dx[index] / length) * limited;
        state.y[index] += (dy[index] / length) * limited;
      }
    }
    state.steps += 1;
    state.temperature *= COOLING;
    ran += 1;
  }
  for (let index = 0; index < count; index += 1) {
    state.x[index] *= CLUSTER_RING_WIDTH;
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

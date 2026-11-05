import { GRAPH_POSTER_ASPECT_RATIO } from "./library-graph-posters.ts";

type Point = { x: number; y: number };

export type GraphRelation = {
  key: string;
  from: string;
  to: string;
  color: string;
  size: number;
  kinds: string[];
};

export type GraphRelationNode = {
  point: Point;
  size: number;
  opacity: number;
};

export type GraphRelationHit = GraphRelation & {
  start: Point;
  end: Point;
  width: number;
  opacity: number;
};

/** Clamp a relation to the poster bounds, including diagonal connections. */
export function graphRelationSegment(
  a: GraphRelationNode,
  b: GraphRelationNode,
) {
  const dx = b.point.x - a.point.x;
  const dy = b.point.y - a.point.y;
  if (dx === 0 && dy === 0) return null;
  const inset = (size: number) =>
    size / Math.max(Math.abs(dx) / GRAPH_POSTER_ASPECT_RATIO, Math.abs(dy));
  const from = inset(a.size);
  const to = inset(b.size);
  if (from + to >= 1) return null;
  return {
    start: { x: a.point.x + dx * from, y: a.point.y + dy * from },
    end: { x: b.point.x - dx * to, y: b.point.y - dy * to },
  };
}

/** Camera frames draw only visible relations; no renderer buffer is rewritten. */
export function drawGraphRelations(
  context: CanvasRenderingContext2D,
  relations: Iterable<GraphRelation>,
  nodes: ReadonlyMap<string, GraphRelationNode>,
  focus: { key: string; accent: string } | null,
): GraphRelationHit[] {
  const batches = new Map<
    string,
    { color: string; opacity: number; width: number; lines: GraphRelationHit[] }
  >();
  const hits: GraphRelationHit[] = [];
  for (const edge of relations) {
    const a = nodes.get(edge.from);
    const b = nodes.get(edge.to);
    if (!a || !b) continue;
    // A deliberate hover already passed poster readability and intent
    // checks. Its connections must not inherit the overview's zoom fade.
    const opacity = focus ? 1 : Math.min(a.opacity, b.opacity);
    if (
      opacity <= 0 ||
      (focus && edge.from !== focus.key && edge.to !== focus.key)
    )
      continue;
    const segment = graphRelationSegment(a, b);
    if (!segment) continue;
    const color = focus ? focus.accent : edge.color;
    const alpha = opacity * (focus ? 0.9 : 1);
    // Relations stay in screen pixels as posters grow. Scaling their width
    // with camera magnification turns a dense graph into expensive overdraw.
    const width = edge.size + (focus ? 1 : 0);
    const hit = { ...edge, ...segment, width, opacity: alpha };
    hits.push(hit);
    const key = `${color}|${alpha}|${width}`;
    const batch = batches.get(key);
    if (batch) batch.lines.push(hit);
    else batches.set(key, { color, opacity: alpha, width, lines: [hit] });
  }
  context.save();
  context.lineCap = "round";
  for (const batch of batches.values()) {
    context.strokeStyle = batch.color;
    context.globalAlpha = batch.opacity;
    context.lineWidth = batch.width;
    // Bound each path's rasterization work when thousands of lines cross.
    for (let offset = 0; offset < batch.lines.length; offset += 128) {
      context.beginPath();
      for (const line of batch.lines.slice(offset, offset + 128)) {
        context.moveTo(line.start.x, line.start.y);
        context.lineTo(line.end.x, line.end.y);
      }
      context.stroke();
    }
  }
  context.restore();
  return hits;
}

/** Edge tooltips only hit painted segments whose two posters are readable. */
export function hitGraphRelation(
  lines: readonly GraphRelationHit[],
  point: Point,
  hoverable: ReadonlySet<string>,
): GraphRelationHit | null {
  let found: GraphRelationHit | null = null;
  let nearest = Infinity;
  for (const line of lines) {
    if (!hoverable.has(line.from) || !hoverable.has(line.to)) continue;
    const dx = line.end.x - line.start.x;
    const dy = line.end.y - line.start.y;
    const length = dx * dx + dy * dy;
    const t = length
      ? Math.max(
          0,
          Math.min(
            1,
            ((point.x - line.start.x) * dx + (point.y - line.start.y) * dy) /
              length,
          ),
        )
      : 0;
    const distance = Math.hypot(
      point.x - line.start.x - t * dx,
      point.y - line.start.y - t * dy,
    );
    if (distance <= line.width / 2 + 3 && distance < nearest) {
      found = line;
      nearest = distance;
    }
  }
  return found;
}

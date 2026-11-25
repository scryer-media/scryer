import type { TourRect } from "./placement.ts";

/** The pen strokes of a hand-drawn arrow, as SVG path data. */
export type TourArrow = {
  /** The arrow's body: one bowed curve from the card to the spotlight. */
  body: string;
  /** A second, slightly offset pass over the body, as a pen would leave. */
  sketch: string;
  /** The arrowhead: two short strokes meeting at the tip. */
  head: string;
};

/** Space left between the card's edge and the arrow's tail. */
const TAIL_GAP = 10;
/** Space left between the arrow's tip and the spotlight. */
const TIP_GAP = 10;
/** Arrows shorter than this read as a smudge and are not drawn. */
const MIN_ARROW_LENGTH = 28;

type Point = { x: number; y: number };

function center(rect: TourRect): Point {
  return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
}

function contains(rect: TourRect, point: Point): boolean {
  return (
    point.x >= rect.left &&
    point.x <= rect.left + rect.width &&
    point.y >= rect.top &&
    point.y <= rect.top + rect.height
  );
}

/** Where the ray from the middle of `rect` toward `toward` leaves it. */
function exitPoint(rect: TourRect, toward: Point): Point {
  const middle = center(rect);
  const dx = toward.x - middle.x;
  const dy = toward.y - middle.y;
  const scale = Math.min(
    dx === 0 ? Infinity : rect.width / 2 / Math.abs(dx),
    dy === 0 ? Infinity : rect.height / 2 / Math.abs(dy),
  );
  return { x: middle.x + dx * scale, y: middle.y + dy * scale };
}

/** The point of `rect` nearest to `point`. */
function nearestPoint(rect: TourRect, point: Point): Point {
  return {
    x: Math.min(Math.max(point.x, rect.left), rect.left + rect.width),
    y: Math.min(Math.max(point.y, rect.top), rect.top + rect.height),
  };
}

/** A small seeded generator, so a step draws the same arrow every time. */
function seededRandom(seed: string): () => number {
  let state = 2166136261;
  for (let index = 0; index < seed.length; index += 1) {
    state = Math.imul(state ^ seed.charCodeAt(index), 16777619);
  }
  return () => {
    state = Math.imul(state ^ (state >>> 15), 2246822507);
    state = Math.imul(state ^ (state >>> 13), 3266489909);
    state ^= state >>> 16;
    return (state >>> 0) / 4294967296;
  };
}

const format = (value: number) => Math.round(value * 10) / 10;
const point = (p: Point) => `${format(p.x)} ${format(p.y)}`;

/**
 * A playful hand-drawn arrow from the card to the spotlight: a bowed curve
 * that leaves the card on the side facing the target and lands just short of
 * it, with a second sketchy pass and a lopsided head. `seed` (the step id)
 * fixes the wobble, so an arrow does not change shape between renders.
 * Returns null when the two overlap or sit too close for an arrow to read.
 */
export function tourArrow(
  card: TourRect,
  spot: TourRect,
  seed: string,
): TourArrow | null {
  const spotCenter = center(spot);
  if (contains(card, spotCenter)) return null;

  const exit = exitPoint(card, spotCenter);
  const landing = nearestPoint(spot, exit);
  const dx = landing.x - exit.x;
  const dy = landing.y - exit.y;
  const distance = Math.hypot(dx, dy);
  const length = distance - TAIL_GAP - TIP_GAP;
  if (length < MIN_ARROW_LENGTH) return null;

  const ux = dx / distance;
  const uy = dy / distance;
  // The normal to the arrow's line; the curve bows along it.
  const nx = -uy;
  const ny = ux;
  const random = seededRandom(seed);
  const along = (from: Point, by: number, bow: number): Point => ({
    x: from.x + ux * by + nx * bow,
    y: from.y + uy * by + ny * bow,
  });

  const tail = along(exit, TAIL_GAP, 0);
  const tip = along(exit, TAIL_GAP + length, 0);
  const bow =
    (random() < 0.5 ? -1 : 1) * Math.min(length * (0.3 + random() * 0.15), 110);
  // The curve swings out early and comes in at a slant, like a flick of the
  // wrist, rather than a symmetric arc.
  const first = along(tail, length * 0.2, bow);
  const second = along(tail, length * 0.75, bow * 0.55);
  const body = `M ${point(tail)} C ${point(first)}, ${point(second)}, ${point(tip)}`;

  const jitter = (p: Point, amount: number): Point => ({
    x: p.x + (random() - 0.5) * amount,
    y: p.y + (random() - 0.5) * amount,
  });
  const sketch = `M ${point(jitter(tail, 3))} C ${point(jitter(first, 8))}, ${point(jitter(second, 8))}, ${point(jitter(tip, 2))}`;

  // The head follows the curve's direction where it lands.
  const tx = tip.x - second.x;
  const ty = tip.y - second.y;
  const tangent = Math.hypot(tx, ty) || 1;
  const back = Math.atan2(-ty / tangent, -tx / tangent);
  const barb = (angle: number, size: number): Point => ({
    x: tip.x + Math.cos(back + angle) * size,
    y: tip.y + Math.sin(back + angle) * size,
  });
  const spread = 0.45 + random() * 0.15;
  const head = `M ${point(barb(-spread, 15))} L ${point(tip)} L ${point(barb(spread + 0.08, 12))}`;

  return { body, sketch, head };
}

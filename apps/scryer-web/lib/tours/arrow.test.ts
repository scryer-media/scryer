import assert from "node:assert/strict";
import test from "node:test";

import { tourArrow } from "./arrow.ts";

const card = { top: 330, left: 510, width: 420, height: 240 };

function endpoints(path: string): { from: number[]; to: number[] } {
  const numbers = path.match(/-?\d+(\.\d+)?/g)!.map(Number);
  return { from: numbers.slice(0, 2), to: numbers.slice(-2) };
}

function inside(rect: typeof card, [x, y]: number[]): boolean {
  return (
    x >= rect.left &&
    x <= rect.left + rect.width &&
    y >= rect.top &&
    y <= rect.top + rect.height
  );
}

test("an arrow runs from outside the card to just short of the spotlight", () => {
  const sidebarItem = { top: 140, left: 10, width: 230, height: 40 };
  const arrow = tourArrow(card, sidebarItem, "rules");
  assert.ok(arrow);
  const { from, to } = endpoints(arrow.body);
  assert.ok(!inside(card, from), "the tail leaves the card");
  assert.ok(!inside(sidebarItem, to), "the tip stops short of the target");
  // The tip lands nearer the target than the tail does.
  const distanceTo = ([x, y]: number[]) =>
    Math.hypot(x - (10 + 115), y - (140 + 20));
  assert.ok(distanceTo(to) < distanceTo(from));
});

test("a step draws the same arrow every time, and steps differ", () => {
  const target = { top: 20, left: 400, width: 600, height: 40 };
  assert.deepEqual(
    tourArrow(card, target, "search"),
    tourArrow(card, target, "search"),
  );
  assert.notDeepEqual(
    tourArrow(card, target, "search"),
    tourArrow(card, target, "libraries"),
  );
});

test("no arrow when the card covers the target or sits right beside it", () => {
  assert.equal(
    tourArrow(card, { top: 400, left: 600, width: 50, height: 20 }, "x"),
    null,
  );
  assert.equal(
    tourArrow(card, { top: 300, left: 600, width: 100, height: 20 }, "x"),
    null,
  );
});

import assert from "node:assert/strict";
import test from "node:test";
import {
  dashboardDropRegions,
  nearestOpenSpot,
} from "./dashboard-drop-targets.ts";

test("an empty grid is reachable one panel height deep across every column", () => {
  assert.deepEqual(dashboardDropRegions([], { w: 4, h: 3 }, 12), [
    { x: 0, y: 0, w: 12, h: 3 },
  ]);
});

test("a gap too narrow for the panel is not a target", () => {
  // Columns 4-5 are free beside two panels but the dragged panel is 3 wide.
  const others = [
    { x: 0, y: 0, w: 4, h: 4 },
    { x: 6, y: 0, w: 6, h: 4 },
  ];
  assert.deepEqual(dashboardDropRegions(others, { w: 3, h: 2 }, 12), [
    { x: 0, y: 4, w: 12, h: 2 },
  ]);
});

test("a gap the panel fits in is a target, merged down its rows", () => {
  const others = [
    { x: 0, y: 0, w: 4, h: 4 },
    { x: 8, y: 0, w: 4, h: 4 },
  ];
  assert.deepEqual(dashboardDropRegions(others, { w: 4, h: 2 }, 12), [
    { x: 4, y: 0, w: 4, h: 4 },
    { x: 0, y: 4, w: 12, h: 2 },
  ]);
});

test("a panel wider than the grid is clamped to its columns", () => {
  assert.deepEqual(dashboardDropRegions([], { w: 20, h: 1 }, 6), [
    { x: 0, y: 0, w: 6, h: 1 },
  ]);
});

test("the open area reaches down to the requested row count", () => {
  const others = [{ x: 0, y: 0, w: 12, h: 2 }];
  assert.deepEqual(dashboardDropRegions(others, { w: 4, h: 2 }, 12, 10), [
    { x: 0, y: 2, w: 12, h: 8 },
  ]);
});

test("a free position is its own nearest open spot", () => {
  const others = [{ x: 0, y: 0, w: 4, h: 4 }];
  assert.deepEqual(nearestOpenSpot(others, { x: 6, y: 1, w: 4, h: 2 }, 12), {
    x: 6,
    y: 1,
  });
});

test("a position over another panel moves to the closest free one", () => {
  const others = [{ x: 0, y: 0, w: 6, h: 10 }];
  // Just inside the left panel's right edge: one column over is free.
  assert.deepEqual(nearestOpenSpot(others, { x: 5, y: 3, w: 4, h: 2 }, 12), {
    x: 6,
    y: 3,
  });
  // Near its bottom: dropping under it is closer than crossing the columns.
  assert.deepEqual(nearestOpenSpot(others, { x: 1, y: 9, w: 4, h: 2 }, 12, 4), {
    x: 1,
    y: 10,
  });
});

test("a full grid sends the panel below everything", () => {
  const others = [{ x: 0, y: 0, w: 12, h: 3 }];
  assert.deepEqual(nearestOpenSpot(others, { x: 2, y: 0, w: 4, h: 2 }, 12), {
    x: 2,
    y: 3,
  });
});

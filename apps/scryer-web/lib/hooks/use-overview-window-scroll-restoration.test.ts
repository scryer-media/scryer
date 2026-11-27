import assert from "node:assert/strict";
import test from "node:test";

import {
  pickScrollAnchors,
  resolveAnchoredScrollTop,
} from "./use-overview-window-scroll-restoration.ts";

const ROW_HEIGHT = 100;

/** Rows stacked from `firstTop`, three to a line, as a grid lays them out. */
function gridRows(keys: string[], firstTop: number) {
  return keys.map((key, index) => {
    const top = firstTop + Math.floor(index / 3) * ROW_HEIGHT;
    return { key, top, bottom: top + ROW_HEIGHT };
  });
}

test("anchors are the first rows still showing, with their offset from the viewport top", () => {
  const rows = gridRows(["a", "b", "c", "d", "e", "f", "g", "h", "i"], -150);
  let reads = 0;
  const anchors = pickScrollAnchors(
    rows.length,
    (index) => {
      reads += 1;
      return rows[index] ?? null;
    },
    0,
  );
  assert.deepEqual(anchors, [
    { key: "d", offset: -50 },
    { key: "e", offset: -50 },
    { key: "f", offset: -50 },
  ]);
  assert.ok(reads <= 8, `read ${reads} rows for a list of ${rows.length}`);
});

test("no rows gives no anchors, and rows all above the viewport give none", () => {
  assert.deepEqual(
    pickScrollAnchors(0, () => null, 0),
    [],
  );
  const rows = gridRows(["a", "b", "c"], -500);
  assert.deepEqual(
    pickScrollAnchors(rows.length, (index) => rows[index] ?? null, 0),
    [],
  );
});

test("the anchored offset puts the row back where it was after rows above changed height", () => {
  const anchors = [{ key: "d", offset: -50 }];
  // The row now sits 70px lower than it did: the rows above grew.
  const next = resolveAnchoredScrollTop(anchors, 1000, 200, (key) =>
    key === "d" ? 220 : null,
  );
  assert.equal(next, 1070);
});

test("a removed anchor falls back to the next one, and none present gives null", () => {
  const anchors = [
    { key: "d", offset: -50 },
    { key: "e", offset: -50 },
  ];
  const next = resolveAnchoredScrollTop(anchors, 1000, 200, (key) =>
    key === "e" ? 150 : null,
  );
  assert.equal(next, 1000);
  assert.equal(
    resolveAnchoredScrollTop(anchors, 1000, 200, () => null),
    null,
  );
});

test("the anchored offset never goes above the top", () => {
  const next = resolveAnchoredScrollTop(
    [{ key: "a", offset: 40 }],
    10,
    0,
    () => 0,
  );
  assert.equal(next, 0);
});

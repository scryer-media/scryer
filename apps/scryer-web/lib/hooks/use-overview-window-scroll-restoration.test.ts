import assert from "node:assert/strict";
import test from "node:test";

import {
  anchoredScrollTop,
  captureScrollAnchors,
  pickScrollAnchors,
  resolveAnchoredScrollTop,
  type ScrollTarget,
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

const ANCHOR_ATTRIBUTE = "data-catalog-title-id";

/** A container holding rows laid out like `gridRows`, as the DOM reports them. */
function fakeRowRoot(keys: string[], firstTop: number) {
  const rows = gridRows(keys, firstTop).map((row) => ({
    getAttribute: (name: string) =>
      name === ANCHOR_ATTRIBUTE ? row.key : null,
    getBoundingClientRect: () => ({ top: row.top, bottom: row.bottom }),
    key: row.key,
  }));
  const root = {
    querySelectorAll: (selector: string) => {
      assert.equal(selector, `[${ANCHOR_ATTRIBUTE}]`);
      return rows;
    },
    querySelector: (selector: string) =>
      rows.find((row) => selector === `[${ANCHOR_ATTRIBUTE}="${row.key}"]`) ??
      null,
  };
  return root as unknown as ParentNode;
}

test("window mode takes anchors only from the list's own container", () => {
  const page = { scrollY: 400 } as unknown as ScrollTarget;
  const grid = fakeRowRoot(["a", "b", "c", "d", "e", "f"], -150);
  assert.deepEqual(
    captureScrollAnchors(page, "window", ANCHOR_ATTRIBUTE, grid),
    [
      { key: "d", offset: -50 },
      { key: "e", offset: -50 },
      { key: "f", offset: -50 },
    ],
  );
  // With no container mounted there is nothing to anchor to, rather than
  // whatever else on the page carries the attribute.
  assert.deepEqual(
    captureScrollAnchors(page, "window", ANCHOR_ATTRIBUTE, null),
    [],
  );
});

test("window mode restores to the row found in the list's container", () => {
  const css = globalThis as { CSS?: { escape: (value: string) => string } };
  const previousCss = css.CSS;
  css.CSS = { escape: (value) => value };
  try {
    const page = { scrollY: 400 } as unknown as ScrollTarget;
    // Row "d" now sits 30px lower than its saved offset of -50.
    const grid = fakeRowRoot(["a", "b", "c", "d", "e", "f"], -120);
    assert.equal(
      anchoredScrollTop(
        page,
        "window",
        ANCHOR_ATTRIBUTE,
        [{ key: "d", offset: -50 }],
        grid,
      ),
      430,
    );
    assert.equal(
      anchoredScrollTop(
        page,
        "window",
        ANCHOR_ATTRIBUTE,
        [{ key: "d", offset: -50 }],
        null,
      ),
      null,
    );
  } finally {
    css.CSS = previousCss;
  }
});

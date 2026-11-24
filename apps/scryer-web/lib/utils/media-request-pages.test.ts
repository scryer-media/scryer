import assert from "node:assert/strict";
import test from "node:test";

import { loadAllMediaRequestPages } from "./media-request-pages.ts";

const rowsOf = (count: number) =>
  Array.from({ length: count }, (_, index) => ({ id: `request-${index}` }));

async function loadWith(total: number, pageSize: number) {
  const all = rowsOf(total);
  const calls: Array<[number, number]> = [];
  const loaded = await loadAllMediaRequestPages(async (limit, offset) => {
    calls.push([limit, offset]);
    return { rows: all.slice(offset, offset + limit) };
  }, pageSize);
  return { loaded, calls, all };
}

test("every page is read in order until a short one", async () => {
  for (const total of [0, 1, 2, 3, 4, 5, 7]) {
    const { loaded, calls, all } = await loadWith(total, 2);
    assert.deepEqual(loaded.rows, all);
    assert.equal(loaded.error, undefined);
    assert.equal(calls.length, Math.floor(total / 2) + 1);
    assert.deepEqual(
      calls,
      calls.map((_, index) => [2, index * 2]),
    );
  }
});

test("a request that moves between pages is kept once", async () => {
  const pages = [
    [{ id: "a" }, { id: "b" }],
    [{ id: "b" }, { id: "c" }],
    [{ id: "d" }],
  ];
  const loaded = await loadAllMediaRequestPages(
    async (_limit, offset) => ({ rows: pages[offset / 2] ?? [] }),
    2,
  );
  assert.deepEqual(
    loaded.rows.map((row) => row.id),
    ["a", "b", "c", "d"],
  );
});

test("a failed page fails the load", async () => {
  const failure = new Error("page failed");
  const loaded = await loadAllMediaRequestPages(
    async (_limit, offset) =>
      offset === 0
        ? { rows: rowsOf(2) }
        : { rows: [] as Array<{ id: string }>, error: failure },
    2,
  );
  assert.deepEqual(loaded.rows, []);
  assert.equal(loaded.error, failure);
});

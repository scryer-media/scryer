import assert from "node:assert/strict";
import test from "node:test";
import { loadCatalogBulk } from "./catalog-bulk-load.ts";

test("four requests start together, finish out of order, and publish in offset order", async () => {
  const requests: number[] = [];
  const pages = Array.from({ length: 4 }, () =>
    Promise.withResolvers<{ items: number[]; hasMore: boolean }>(),
  );
  const progress: number[] = [];
  const loading = loadCatalogBulk({
    offset: 2,
    pageSize: 2,
    isCurrent: () => true,
    loadPage: (offset) => {
      requests.push(offset);
      return pages[(offset - 2) / 2].promise;
    },
    onProgress: (count) => progress.push(count),
  });
  assert.deepEqual(requests, [2, 4, 6, 8]);
  pages[3].resolve({ items: [], hasMore: false });
  pages[2].resolve({ items: [6], hasMore: false });
  pages[1].resolve({ items: [4, 5], hasMore: true });
  assert.deepEqual(progress, []);
  pages[0].resolve({ items: [2, 3], hasMore: true });
  assert.deepEqual(await loading, [2, 3, 4, 5, 6]);
  assert.deepEqual(progress, [7]);
});

test("ten thousand rows load completely with at most four requests in flight", async () => {
  let active = 0,
    peak = 0;
  const progress: number[] = [];
  const items = await loadCatalogBulk({
    offset: 300,
    pageSize: 300,
    isCurrent: () => true,
    loadPage: async (offset) => {
      peak = Math.max(peak, ++active);
      await Promise.resolve();
      active--;
      const end = Math.min(10_000, offset + 300);
      return {
        items: Array.from(
          { length: Math.max(0, end - offset) },
          (_, i) => offset + i,
        ),
        hasMore: end < 10_000,
      };
    },
    onProgress: (count) => progress.push(count),
  });
  assert.equal(peak, 4);
  assert.deepEqual(
    items,
    Array.from({ length: 9700 }, (_, i) => i + 300),
  );
  assert.equal(progress.at(-1), 10_000);
  assert.ok(progress.every((count, i) => i === 0 || count > progress[i - 1]));
});

test("stale requests cannot publish progress or schedule another wave", async () => {
  let current = true,
    requests = 0;
  const page = Promise.withResolvers<{ items: number[]; hasMore: boolean }>();
  const loading = loadCatalogBulk({
    offset: 0,
    pageSize: 1,
    isCurrent: () => current,
    loadPage: () => {
      requests++;
      return page.promise;
    },
    onProgress: () => assert.fail("stale progress"),
  });
  current = false;
  page.resolve({ items: [1], hasMore: true });
  assert.equal(await loading, null);
  assert.equal(requests, 4);
});

test("failure returns no partial catalog and a fresh submission can retry", async () => {
  let fail = true,
    requests = 0;
  const options = {
    offset: 0,
    pageSize: 1,
    isCurrent: () => true,
    loadPage: async (offset: number) => {
      requests++;
      if (fail && offset === 1) throw new Error("fixture failure");
      return { items: offset < 3 ? [offset] : [], hasMore: offset < 2 };
    },
    onProgress: () => {},
  };
  await assert.rejects(loadCatalogBulk(options));
  assert.equal(requests, 4);
  fail = false;
  assert.deepEqual(await loadCatalogBulk(options), [0, 1, 2]);
});

test("a short nonterminal page fails rather than skipping rows", async () => {
  await assert.rejects(
    loadCatalogBulk({
      offset: 0,
      pageSize: 300,
      isCurrent: () => true,
      loadPage: async () => ({ items: [], hasMore: true }),
      onProgress: () => assert.fail("incomplete progress"),
    }),
  );
});

import assert from "node:assert/strict";
import test from "node:test";

import type { DownloadQueueItem } from "../types/download-queue.ts";
import {
  type DownloadQueueRetainedPage,
  downloadQueueSyncRefreshRanges,
  flattenDownloadQueuePages,
  markDownloadQueuePagesStale,
  mergeDownloadQueuePageRange,
  nextContiguousDownloadQueueOffset,
  retainedDownloadQueuePageNeedsRefresh,
  shouldApplyDownloadQueuePageResponse,
  shouldRefreshDownloadQueueSync,
} from "./download-queue-page.ts";

function item(id: string, clientId = "client-1"): DownloadQueueItem {
  return {
    id,
    clientId,
    clientType: "qbittorrent",
    downloadClientItemId: id,
  } as DownloadQueueItem;
}

test("queue page ranges append infinitely and deduplicate stable identities", () => {
  let pages = mergeDownloadQueuePageRange(
    new Map(),
    Array.from({ length: 50 }, (_, index) => item(`item-${index}`)),
    0,
    50,
    { reset: true, revision: 1, totalCount: 99 },
  );
  pages = mergeDownloadQueuePageRange(
    pages,
    [item("item-49"), ...Array.from({ length: 49 }, (_, index) => item(`item-${index + 50}`))],
    50,
    50,
    { reset: false, revision: 1, totalCount: 99 },
  );

  const rows = flattenDownloadQueuePages(pages);
  assert.equal(rows.length, 99);
  assert.equal(rows[0].id, "item-0");
  assert.equal(rows.at(-1)?.id, "item-98");
});

test("queue refreshes retain unchanged item references", () => {
  const original = item("unchanged");
  const pages = mergeDownloadQueuePageRange(
    new Map(),
    [original],
    0,
    50,
    { reset: true, revision: 1, totalCount: 1 },
  );
  const unchanged = mergeDownloadQueuePageRange(
    pages,
    [{ ...original }],
    0,
    50,
    { reset: false, revision: 2, totalCount: 1 },
  );
  const changed = mergeDownloadQueuePageRange(
    unchanged,
    [{ ...original, progressPercent: 1 }],
    0,
    50,
    { reset: false, revision: 3, totalCount: 1 },
  );

  assert.equal(unchanged.get(0)?.items[0], original);
  assert.notEqual(changed.get(0)?.items[0], original);
});

test("filter or sort resets discard retained queue pages", () => {
  const existing = new Map<number, DownloadQueueRetainedPage>([
    [0, { items: [item("old-0")], revision: 1, stale: false }],
    [50, { items: [item("old-50")], revision: 1, stale: false }],
  ]);
  const reset = mergeDownloadQueuePageRange(existing, [item("new-0")], 0, 50, {
    reset: true,
    revision: 2,
    totalCount: 1,
  });
  assert.deepEqual(flattenDownloadQueuePages(reset).map((row) => row.id), ["new-0"]);
});

test("queue shrink evicts retained pages and recomputes the contiguous offset", () => {
  const existing = new Map<number, DownloadQueueRetainedPage>([
    [0, { items: Array.from({ length: 50 }, (_, index) => item(`old-${index}`)), revision: 1, stale: false }],
    [50, { items: Array.from({ length: 50 }, (_, index) => item(`old-${index + 50}`)), revision: 1, stale: false }],
    [100, { items: [item("old-100")], revision: 1, stale: false }],
  ]);
  const next = mergeDownloadQueuePageRange(
    existing,
    Array.from({ length: 42 }, (_, index) => item(`new-${index}`)),
    0,
    200,
    { reset: false, revision: 2, totalCount: 42, markRetainedStale: true },
  );
  assert.deepEqual([...next.keys()], [0]);
  assert.equal(nextContiguousDownloadQueueOffset(next, 42), 42);
});

test("retained pages become stale by revision and refresh when visible", () => {
  const pages = new Map<number, DownloadQueueRetainedPage>([
    [0, { items: [item("first")], revision: 3, stale: false }],
    [50, { items: [item("retained")], revision: 2, stale: false }],
  ]);
  const stale = markDownloadQueuePagesStale(pages, 3);
  assert.equal(stale.get(0)?.stale, false);
  assert.equal(stale.get(50)?.stale, true);
  assert.equal(retainedDownloadQueuePageNeedsRefresh(stale, 51, 3), true);
});

test("queue sync refreshes the first 200 rows and the visible retained page", () => {
  assert.deepEqual(downloadQueueSyncRefreshRanges(150, 120), [{ offset: 0, limit: 150 }]);
  assert.deepEqual(downloadQueueSyncRefreshRanges(350, 275), [
    { offset: 0, limit: 200 },
    { offset: 250, limit: 50 },
  ]);
});

test("queue sync suppresses hidden or inactive refreshes", () => {
  assert.equal(shouldRefreshDownloadQueueSync(true, "hidden"), false);
  assert.equal(shouldRefreshDownloadQueueSync(false, "visible"), false);
  assert.equal(shouldRefreshDownloadQueueSync(true, "visible"), true);
});

test("old or pre-sync queue responses cannot overwrite newer pages", () => {
  assert.equal(shouldApplyDownloadQueuePageResponse(4, 5, 5), false);
  assert.equal(shouldApplyDownloadQueuePageResponse(5, 5, 6), false);
  assert.equal(shouldApplyDownloadQueuePageResponse(6, 5, 6), true);
});

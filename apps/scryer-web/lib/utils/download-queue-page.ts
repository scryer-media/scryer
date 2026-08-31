import type { DownloadQueueItem } from "../types/download-queue";
import { sameDownloadQueueItem } from "./download-queue.ts";

export const DOWNLOAD_QUEUE_PAGE_SIZE = 50;
export const DOWNLOAD_QUEUE_SYNC_REFRESH_LIMIT = 200;

export type DownloadQueueRetainedPage = {
  items: DownloadQueueItem[];
  revision: number;
  stale: boolean;
};

function queueIdentity(item: DownloadQueueItem): string {
  const client = item.clientId.trim() || item.clientType.trim().toLowerCase();
  return `${client}:${item.downloadClientItemId}`;
}

export function flattenDownloadQueuePages(
  pages: Map<number, DownloadQueueRetainedPage>,
): DownloadQueueItem[] {
  const seen = new Set<string>();
  const items: DownloadQueueItem[] = [];
  for (const [, page] of [...pages.entries()].sort(([left], [right]) => left - right)) {
    for (const item of page.items) {
      const key = queueIdentity(item);
      if (!seen.has(key)) {
        seen.add(key);
        items.push(item);
      }
    }
  }
  return items;
}

export function mergeDownloadQueuePageRange(
  current: Map<number, DownloadQueueRetainedPage>,
  items: DownloadQueueItem[],
  offset: number,
  limit: number,
  options: {
    reset: boolean;
    revision: number;
    totalCount: number;
    markRetainedStale?: boolean;
  },
): Map<number, DownloadQueueRetainedPage> {
  const coveredEnd = offset + limit;
  const previousItems = new Map<string, DownloadQueueItem>();
  if (!options.reset) {
    for (const [pageOffset, page] of current) {
      const overlaps =
        pageOffset < coveredEnd && pageOffset + page.items.length > offset;
      if (overlaps) {
        for (const item of page.items) {
          previousItems.set(queueIdentity(item), item);
        }
      }
    }
  }
  const reconciledItems = items.map((item) => {
    const previous = previousItems.get(queueIdentity(item));
    return previous && sameDownloadQueueItem(previous, item) ? previous : item;
  });
  const next = options.reset
    ? new Map<number, DownloadQueueRetainedPage>()
    : new Map(
        [...current].map(([pageOffset, page]) => [
          pageOffset,
          options.markRetainedStale && page.revision < options.revision
            ? { ...page, stale: true }
            : page,
        ]),
      );
  for (const [pageOffset, page] of next) {
    const overlaps =
      pageOffset < coveredEnd && pageOffset + page.items.length > offset;
    if (
      pageOffset >= options.totalCount ||
      (overlaps && page.revision <= options.revision)
    ) {
      next.delete(pageOffset);
    }
  }
  for (
    let index = 0;
    index < reconciledItems.length;
    index += DOWNLOAD_QUEUE_PAGE_SIZE
  ) {
    next.set(offset + index, {
      items: reconciledItems.slice(index, index + DOWNLOAD_QUEUE_PAGE_SIZE),
      revision: options.revision,
      stale: false,
    });
  }
  return next;
}

export function markDownloadQueuePagesStale(
  current: Map<number, DownloadQueueRetainedPage>,
  revision: number,
): Map<number, DownloadQueueRetainedPage> {
  return new Map(
    [...current].map(([offset, page]) => [
      offset,
      page.revision < revision ? { ...page, stale: true } : page,
    ]),
  );
}

export function nextContiguousDownloadQueueOffset(
  pages: Map<number, DownloadQueueRetainedPage>,
  totalCount: number,
): number {
  let offset = 0;
  while (offset < totalCount) {
    const page = pages.get(offset);
    if (!page || page.stale || page.items.length === 0) {
      break;
    }
    offset += page.items.length;
    if (page.items.length < DOWNLOAD_QUEUE_PAGE_SIZE) {
      break;
    }
  }
  return Math.min(offset, totalCount);
}

export function retainedDownloadQueuePageNeedsRefresh(
  pages: Map<number, DownloadQueueRetainedPage>,
  offset: number,
  targetRevision: number,
): boolean {
  const pageOffset =
    Math.floor(Math.max(0, offset) / DOWNLOAD_QUEUE_PAGE_SIZE) *
    DOWNLOAD_QUEUE_PAGE_SIZE;
  const page = pages.get(pageOffset);
  return Boolean(page && (page.stale || page.revision < targetRevision));
}

export function downloadQueueSyncRefreshRanges(
  loadedOffset: number,
  visibleOffset: number,
): Array<{ offset: number; limit: number }> {
  const ranges = [
    {
      offset: 0,
      limit: Math.min(
        DOWNLOAD_QUEUE_SYNC_REFRESH_LIMIT,
        Math.max(DOWNLOAD_QUEUE_PAGE_SIZE, loadedOffset),
      ),
    },
  ];
  const visiblePageOffset =
    Math.floor(Math.max(0, visibleOffset) / DOWNLOAD_QUEUE_PAGE_SIZE) *
    DOWNLOAD_QUEUE_PAGE_SIZE;
  if (
    loadedOffset > DOWNLOAD_QUEUE_SYNC_REFRESH_LIMIT &&
    visiblePageOffset >= DOWNLOAD_QUEUE_SYNC_REFRESH_LIMIT
  ) {
    ranges.push({ offset: visiblePageOffset, limit: DOWNLOAD_QUEUE_PAGE_SIZE });
  }
  return ranges;
}

export function shouldRefreshDownloadQueueSync(enabled: boolean, visibility: string): boolean {
  return enabled && visibility !== "hidden";
}

export function shouldApplyDownloadQueuePageResponse(
  responseRevision: number,
  currentRevision: number,
  minimumRevision: number,
): boolean {
  return responseRevision >= currentRevision && responseRevision >= minimumRevision;
}

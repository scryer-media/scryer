import type { DownloadQueueItem } from "@/lib/types";
import { normalizeQueueState } from "./download-queue.ts";

/**
 * The slice of a queue item the catalog download indicator cares about. Kept as
 * a `Pick` so the predicate stays trivially testable and works for both live
 * subscription payloads and query results.
 */
export type CatalogDownloadActivityInput = Pick<
  DownloadQueueItem,
  "titleId" | "displayState"
>;

export type PendingDownloadActivityInput = Pick<
  DownloadQueueItem,
  "displayState"
>;

/**
 * Display states that mean "work is still pending for this title" — everything
 * from sitting in the client queue through the import finishing.
 *
 * Deliberately excluded: `paused` (user parked it), `import_blocked` /
 * `import_failed` / `failed` / `remove_failed` (needs a human, not progress),
 * `removing` (being torn down), and `completed` / `ignored` (historical).
 */
const PENDING_CATALOG_DOWNLOAD_DISPLAY_STATES: ReadonlySet<string> = new Set([
  "queued",
  "downloading",
  "post_processing",
  "import_pending",
  "importing",
]);

/**
 * True when a queue item represents live work that should surface as a
 * pulsing "Downloading" pill.
 */
export function isPendingDownloadQueueItem(
  item: PendingDownloadActivityInput,
): boolean {
  return PENDING_CATALOG_DOWNLOAD_DISPLAY_STATES.has(
    normalizeQueueState(item.displayState),
  );
}

export function isPendingCatalogDownloadQueueItem(
  item: CatalogDownloadActivityInput,
): boolean {
  // Items with no linked title cannot be attributed to a catalog row.
  return Boolean(item.titleId?.trim()) && isPendingDownloadQueueItem(item);
}

/**
 * Collapse a queue snapshot into the set of title ids with pending work.
 * Several queue items for one title collapse to a single entry.
 */
export function collectActiveDownloadTitleIds(
  items: readonly CatalogDownloadActivityInput[],
): Set<string> {
  const titleIds = new Set<string>();
  for (const item of items) {
    if (isPendingCatalogDownloadQueueItem(item)) {
      titleIds.add(item.titleId as string);
    }
  }
  return titleIds;
}

import assert from "node:assert/strict";
import test, { after, before } from "node:test";
import { fileURLToPath } from "node:url";
import type { DownloadQueueItem } from "../types/download-queue.ts";
import { createServer, type ViteDevServer } from "vite";

const WEB_ROOT = fileURLToPath(new URL("../..", import.meta.url));

type DownloadQueueModule = {
  IMPORT_ATTENTION_STATUSES: string[];
  sameDownloadQueueItem: (
    current: DownloadQueueItem,
    next: DownloadQueueItem,
  ) => boolean;
  reconcileDownloadQueueItems: (
    current: DownloadQueueItem[],
    next: DownloadQueueItem[],
  ) => DownloadQueueItem[];
  isManualImportRequiredQueueItem: (item: DownloadQueueItem) => boolean;
  mergeAuthoritativeQueueItems: (
    authoritativeItems: DownloadQueueItem[],
    previousItems: DownloadQueueItem[],
  ) => DownloadQueueItem[];
  matchesActivityStatuses: (
    item: Pick<DownloadQueueItem, "displayState">,
    statuses: string[],
  ) => boolean;
  matchesImportStatuses: (
    item: Pick<DownloadQueueItem, "displayState">,
    statuses: string[],
  ) => boolean;
  sortDownloadQueueItems: (items: DownloadQueueItem[]) => DownloadQueueItem[];
};

let server: ViteDevServer;
let downloadQueue: DownloadQueueModule;

before(async () => {
  server = await createServer({
    root: WEB_ROOT,
    server: { middlewareMode: true },
    appType: "custom",
    logLevel: "silent",
  });
  downloadQueue = (await server.ssrLoadModule(
    "/lib/utils/download-queue.ts",
  )) as unknown as DownloadQueueModule;
});

after(async () => {
  await server.close();
});

function queueItem(overrides: Partial<DownloadQueueItem> = {}): DownloadQueueItem {
  return {
    id: "qbittorrent:abc",
    titleId: "title-1",
    episodeId: null,
    titleName: "Example.Release.2026.1080p",
    facet: "MOVIE",
    isScryerOrigin: true,
    sourceProvider: null,
    clientId: "client-1",
    clientName: "qBittorrent",
    clientType: "qbittorrent",
    state: "DOWNLOADING",
    displayState: "DOWNLOADING",
    progressPercent: 42,
    importTransferPhase: null,
    importTransferBytes: null,
    importTransferTotalBytes: null,
    importTransferStartedAt: null,
    importTransferUpdatedAt: null,
    sizeBytes: 1024,
    remainingSeconds: null,
    queuedAt: "2026-08-21T10:00:00Z",
    lastUpdatedAt: "2026-08-21T10:00:00Z",
    attentionRequired: false,
    attentionReason: null,
    downloadClientItemId: "abc",
    downloadId: "scryer-download:abc",
    importStatus: null,
    importErrorCode: null,
    importErrorMessage: null,
    importedAt: null,
    deleteStatus: null,
    deleteErrorMessage: null,
    trackedState: "DOWNLOADING",
    trackedStatus: "OK",
    trackedStatusMessages: [],
    trackedMatchType: "SUBMISSION",
    seedingState: null,
    seedRatio: null,
    seedRatioGoal: null,
    seedTimeSeconds: null,
    seedTimeGoalSeconds: null,
    isPrivate: null,
    queueScope: null,
    ...overrides,
  };
}

test("queue item equality ignores regenerated equivalent payload objects", () => {
  const current = queueItem({
    trackedStatusMessages: ["waiting for import"],
    queueScope: {
      __typename: "EpisodeSetScopePayload",
      episodeIds: ["episode-1", "episode-2"],
    },
  });
  const next = {
    ...current,
    trackedStatusMessages: [...current.trackedStatusMessages],
    queueScope: {
      __typename: "EpisodeSetScopePayload" as const,
      episodeIds: ["episode-1", "episode-2"],
    },
  };

  assert.equal(downloadQueue.sameDownloadQueueItem(current, next), true);
  assert.equal(
    downloadQueue.sameDownloadQueueItem(
      current,
      { ...next, progressPercent: next.progressPercent + 1 },
    ),
    false,
  );
});

test("queue reconciliation retains unchanged rows and list identity", () => {
  const first = queueItem();
  const second = queueItem({ id: "qbittorrent:def", downloadClientItemId: "def" });
  const current = [first, second];

  const unchanged = downloadQueue.reconcileDownloadQueueItems(current, [
    { ...first, trackedStatusMessages: [...first.trackedStatusMessages] },
    { ...second, queueScope: null },
  ]);
  assert.equal(unchanged, current);

  const updated = downloadQueue.reconcileDownloadQueueItems(current, [
    { ...first, progressPercent: 43 },
    { ...second, queueScope: null },
  ]);
  assert.notEqual(updated, current);
  assert.notEqual(updated[0], first);
  assert.equal(updated[1], second);
});

const warnedItem = () =>
  queueItem({
    state: "WARNING",
    displayState: "WARNING",
    attentionRequired: true,
    attentionReason: "files are missing from the save path",
    lastUpdatedAt: "2026-08-21T10:01:00Z",
  });

test("a warning supersedes the downloading row it replaces", () => {
  // The transient-merge keeps the previous row when it outranks the
  // authoritative one; an unranked WARNING would pin the row to a stale
  // DOWNLOADING badge for as long as the query is the only observer.
  const merged = downloadQueue.mergeAuthoritativeQueueItems(
    [warnedItem()],
    [queueItem()],
  );

  assert.equal(merged.length, 1);
  assert.equal(merged[0].displayState, "WARNING");
  assert.equal(
    merged[0].attentionReason,
    "files are missing from the save path",
  );
});

test("a warned row does not fall back to the previous state on the next poll", () => {
  const first = downloadQueue.mergeAuthoritativeQueueItems(
    [warnedItem()],
    [queueItem()],
  );
  const second = downloadQueue.mergeAuthoritativeQueueItems(
    [warnedItem()],
    first,
  );

  assert.equal(second[0].displayState, "WARNING");
});

test("a warned row is still replaced once the client recovers", () => {
  const recovered = downloadQueue.mergeAuthoritativeQueueItems(
    [queueItem({ lastUpdatedAt: "2026-08-21T10:02:00Z", progressPercent: 55 })],
    [warnedItem()],
  );

  assert.equal(recovered[0].displayState, "DOWNLOADING");
});

test("a warned row filters with the activity chips", () => {
  const item = { displayState: "WARNING" as const };

  assert.equal(downloadQueue.matchesActivityStatuses(item, ["WARNING"]), true);
  assert.equal(
    downloadQueue.matchesActivityStatuses(item, ["DOWNLOADING", "QUEUED"]),
    false,
  );
});

test("active imports are exclusive to the live Activity stream", () => {
  assert.deepEqual(downloadQueue.IMPORT_ATTENTION_STATUSES, [
    "PENDING",
    "BLOCKED",
    "FAILED",
  ]);
  assert.equal(
    downloadQueue.matchesImportStatuses(
      { displayState: "IMPORTING" },
      downloadQueue.IMPORT_ATTENTION_STATUSES,
    ),
    false,
  );
  assert.equal(
    downloadQueue.isManualImportRequiredQueueItem(
      queueItem({
        importStatus: "RUNNING",
        state: "COMPLETED",
        displayState: "IMPORTING",
      }),
    ),
    false,
  );
});

test("warned rows sort with the other attention states", () => {
  const sorted = downloadQueue.sortDownloadQueueItems([
    warnedItem(),
    queueItem({ id: "b", downloadClientItemId: "b" }),
  ]);

  assert.deepEqual(
    sorted.map((item) => item.displayState),
    ["DOWNLOADING", "WARNING"],
  );
});

import assert from "node:assert/strict";
import test from "node:test";

import type { DownloadDisplayState } from "@/lib/types/download-queue";
import {
  collectActiveDownloadTitleIds,
  isPendingDownloadQueueItem,
  isPendingCatalogDownloadQueueItem,
  type CatalogDownloadActivityInput,
} from "./catalog-download-activity.ts";

function queueItem(
  displayState: DownloadDisplayState,
  titleId: string | null = "title-1",
): CatalogDownloadActivityInput {
  return { titleId, displayState };
}

const PENDING_DISPLAY_STATES: DownloadDisplayState[] = [
  "QUEUED",
  "DOWNLOADING",
  "POST_PROCESSING",
  "IMPORT_PENDING",
  "IMPORTING",
];

const NON_PENDING_DISPLAY_STATES: DownloadDisplayState[] = [
  "PAUSED",
  "COMPLETED",
  "IMPORTED_SEEDING",
  "FAILED",
  "IMPORT_BLOCKED",
  "IMPORT_FAILED",
  "IGNORED",
  "REMOVING",
  "REMOVE_FAILED",
];

test("every pending lifecycle state counts as catalog download activity", () => {
  for (const displayState of PENDING_DISPLAY_STATES) {
    assert.equal(
      isPendingCatalogDownloadQueueItem(queueItem(displayState)),
      true,
      `${displayState} should count as pending`,
    );
  }
});

test("pending lifecycle state detection does not require a title", () => {
  assert.equal(
    isPendingDownloadQueueItem({ displayState: "DOWNLOADING" }),
    true,
  );
  assert.equal(isPendingDownloadQueueItem({ displayState: "PAUSED" }), false);
});

test("paused, blocked, failed, removed, and historical states never count", () => {
  for (const displayState of NON_PENDING_DISPLAY_STATES) {
    assert.equal(
      isPendingCatalogDownloadQueueItem(queueItem(displayState)),
      false,
      `${displayState} should not count as pending`,
    );
  }
});

test("state matching is case- and whitespace-insensitive", () => {
  assert.equal(
    isPendingCatalogDownloadQueueItem({
      titleId: "title-1",
      displayState: " downloading " as DownloadDisplayState,
    }),
    true,
  );
  assert.equal(
    isPendingCatalogDownloadQueueItem({
      titleId: "title-1",
      displayState: "Post_Processing" as DownloadDisplayState,
    }),
    true,
  );
  assert.equal(
    isPendingCatalogDownloadQueueItem({
      titleId: "title-1",
      displayState: "" as DownloadDisplayState,
    }),
    false,
  );
});

test("queue items with no linked title never count", () => {
  for (const displayState of PENDING_DISPLAY_STATES) {
    assert.equal(
      isPendingCatalogDownloadQueueItem(queueItem(displayState, null)),
      false,
      `${displayState} without a titleId should not count`,
    );
    assert.equal(
      isPendingCatalogDownloadQueueItem(queueItem(displayState, "")),
      false,
      `${displayState} with an empty titleId should not count`,
    );
    assert.equal(
      isPendingCatalogDownloadQueueItem(queueItem(displayState, "   ")),
      false,
      `${displayState} with a blank titleId should not count`,
    );
  }
});

test("collecting title ids dedupes multiple queue items for one title", () => {
  const titleIds = collectActiveDownloadTitleIds([
    queueItem("DOWNLOADING", "title-1"),
    queueItem("QUEUED", "title-1"),
    queueItem("IMPORTING", "title-1"),
    queueItem("QUEUED", "title-2"),
  ]);

  assert.deepEqual([...titleIds].sort(), ["title-1", "title-2"]);
});

test("collecting title ids keeps a title whose other items are inert", () => {
  const titleIds = collectActiveDownloadTitleIds([
    queueItem("PAUSED", "title-1"),
    queueItem("FAILED", "title-1"),
    queueItem("DOWNLOADING", "title-1"),
  ]);

  assert.deepEqual([...titleIds], ["title-1"]);
});

test("collecting title ids drops titles with only inert items", () => {
  const titleIds = collectActiveDownloadTitleIds([
    queueItem("PAUSED", "title-1"),
    queueItem("IMPORT_BLOCKED", "title-2"),
    queueItem("COMPLETED", "title-3"),
    queueItem("DOWNLOADING", null),
  ]);

  assert.equal(titleIds.size, 0);
});

test("an empty queue snapshot yields an empty set", () => {
  assert.equal(collectActiveDownloadTitleIds([]).size, 0);
});

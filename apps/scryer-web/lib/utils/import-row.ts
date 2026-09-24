import type { DownloadQueueItem } from "../types/download-queue.ts";
import { buildCalendarEventHref } from "./calendar-event-href.ts";

export function importTitleHref(item: Pick<DownloadQueueItem, "titleId" | "facet" | "trackedMatchType">): string | null {
  if (!item.titleId || !item.facet || item.trackedMatchType === "UNMATCHED") return null;
  return buildCalendarEventHref({ id: "", titleId: item.titleId, titleFacet: item.facet });
}

export function isWaitingForDiskSpace(item: Pick<DownloadQueueItem,
  "displayState" | "importErrorCode" | "importErrorMessage" | "trackedStatusMessages"
>): boolean {
  if (!["IMPORT_PENDING", "IMPORT_BLOCKED", "IMPORT_FAILED"].includes(item.displayState)) return false;
  if (item.importErrorCode === "DISK_FULL") return true;
  if (item.importErrorCode && item.importErrorCode !== "UNKNOWN") return false;
  // Match the existing admission-check format, never a generic mention of space.
  const format = /\binsufficient disk space: \d+\.\d GB available, need \d+\.\d GB\b/;
  return [item.importErrorMessage, ...item.trackedStatusMessages].some(
    (message) => typeof message === "string" && format.test(message),
  );
}

export function diskSpaceReason(item: Pick<DownloadQueueItem, "importErrorMessage" | "trackedStatusMessages">): string {
  const messages = [...new Set([...item.trackedStatusMessages, item.importErrorMessage]
    .filter((message): message is string => Boolean(message?.trim()))
    .map((message) => message.trim()))];
  return messages.filter((message) => !messages.some((other) =>
    other.length > message.length && other.includes(message.replace(/\.$/, "")),
  )).join("\n");
}

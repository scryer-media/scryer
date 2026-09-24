import assert from "node:assert/strict";
import { test } from "node:test";
import { diskSpaceReason, importTitleHref, isWaitingForDiskSpace } from "./import-row.ts";

test("disk reasons keep retry information without repeating the measurement", () => {
  const reason = "insufficient disk space: 0.5 GB available, need 12.0 GB";
  const retry = `${reason}. Retrying automatically (attempt 25).`;
  assert.equal(diskSpaceReason({ importErrorMessage: reason, trackedStatusMessages: [retry, retry] }), retry);
  assert.equal(diskSpaceReason({ importErrorMessage: `${reason}.`, trackedStatusMessages: [reason] }), `${reason}.`);
});

test("matched facets use existing overview routes; unmatched and unknown identities stay plain", () => {
  for (const [facet, route] of [["MOVIE", "movies"], ["SERIES", "series"], ["ANIME", "anime"]]) {
    const href = importTitleHref({ titleId: "title-1", facet, trackedMatchType: "SUBMISSION" });
    assert.ok(href?.startsWith(`/${route}`), `${facet}: ${href}`);
    assert.equal(new URL(href!, "https://example.test").searchParams.get("id"), "title-1");
  }
  assert.equal(importTitleHref({ titleId: "title-1", facet: "SERIES", trackedMatchType: "UNMATCHED" }), null);
  assert.equal(importTitleHref({ titleId: null, facet: "SERIES", trackedMatchType: null }), null);
  assert.equal(importTitleHref({ titleId: "title-1", facet: "UNKNOWN", trackedMatchType: null }), null);
});

test("disk-space labels prefer typed errors and accept only the existing legacy format", () => {
  const item = { displayState: "IMPORT_PENDING" as const, importErrorCode: null, importErrorMessage: null, trackedStatusMessages: [] as string[] };
  assert.equal(isWaitingForDiskSpace({ ...item, importErrorCode: "DISK_FULL" }), true);
  const legacy = { ...item, trackedStatusMessages: ["insufficient disk space: 0.5 GB available, need 12.0 GB. Retrying automatically"] };
  assert.equal(isWaitingForDiskSpace(legacy), true);
  assert.equal(isWaitingForDiskSpace({ ...legacy, importErrorCode: "PERMISSION_DENIED" }), false);
  assert.equal(isWaitingForDiskSpace({ ...legacy, displayState: "COMPLETED" }), false);
  assert.equal(isWaitingForDiskSpace({ ...item, importErrorMessage: "could not measure disk space" }), false);
  assert.equal(isWaitingForDiskSpace({ ...item, importErrorMessage: "insufficient disk space" }), false);
});

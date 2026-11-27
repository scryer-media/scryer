import assert from "node:assert/strict";
import test from "node:test";
import en from "../i18n/locales/en.ts";
import {
  IMPORT_LINK_FALLBACK_KEYS,
  IMPORT_PLACEMENT_KEYS,
  failedLinkPlacements,
  importLinkFallbackLabelKey,
  importPlacementLabelKey,
} from "./history-placements.ts";

test("every placement and fallback code has an English label", () => {
  for (const key of [
    ...Object.values(IMPORT_PLACEMENT_KEYS),
    ...Object.values(IMPORT_LINK_FALLBACK_KEYS),
  ]) {
    assert.ok(en[key]?.trim(), key);
  }
});

test("an unknown code has no label key, so the caller shows the code", () => {
  assert.equal(importPlacementLabelKey("REFLINK"), null);
  assert.equal(importLinkFallbackLabelKey("SOMETHING_NEW"), null);
  assert.equal(importPlacementLabelKey("COPY"), "history.placementCopy");
});

test("only copies after a failed hard link are singled out", () => {
  const failed = {
    path: "/library/Synthetic Movie (2001)/movie.mkv",
    placement: "COPY" as const,
    linkFallback: {
      cause: "FAILED" as const,
      reason: "DISK_OPERATION_FAILED",
      osErrorCode: 22,
    },
  };
  const placements = [
    failed,
    {
      path: "/library/Synthetic Movie (2001)/extra.mkv",
      placement: "COPY" as const,
      linkFallback: {
        cause: "NOT_POSSIBLE" as const,
        reason: "DISK_LINK_DIFFERENT_VOLUME",
        osErrorCode: 18,
      },
    },
    {
      path: "/library/Synthetic Movie (2001)/sample.mkv",
      placement: "HARD_LINK" as const,
      linkFallback: null,
    },
  ];
  assert.deepEqual(failedLinkPlacements(placements), [failed]);
  assert.deepEqual(failedLinkPlacements(null), []);
});

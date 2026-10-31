import test from "node:test";
import assert from "node:assert/strict";
import { t } from "../i18n/index.ts";
import {
  IMPORT_SKIP_REASON_KEYS,
  importSkipReasonLabelKey,
} from "./import-skip-reason.ts";

test("all public skip conditions have labels, including source races", () => {
  for (const key of Object.values(IMPORT_SKIP_REASON_KEYS))
    assert.notEqual(t(key, "eng"), key);
  assert.equal(
    importSkipReasonLabelKey("source_changed"),
    "importHistory.skipReason.SOURCE_CHANGED",
  );
  assert.equal(
    importSkipReasonLabelKey("SOURCE_UNAVAILABLE"),
    "importHistory.skipReason.SOURCE_UNAVAILABLE",
  );
  assert.equal(
    importSkipReasonLabelKey("SCENE_SUBTITLE_PENDING"),
    "importHistory.skipReason.SCENE_SUBTITLE_PENDING",
  );
  assert.equal(importSkipReasonLabelKey("future_unknown"), null);
});

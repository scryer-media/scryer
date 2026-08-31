import assert from "node:assert/strict";
import test from "node:test";

import { normalizeApplicationUpgradeProgress } from "./application-upgrade-progress.ts";

test("normalizes application upgrade progress from a JSON payload", () => {
  assert.deepEqual(
    normalizeApplicationUpgradeProgress(
      JSON.stringify({
        status: "running",
        phase: "downloading",
        downloadedBytes: 512,
        totalBytes: 1024,
        targetVersion: "1.2.3",
        targetTag: "v1.2.3",
        error: null,
      }),
    ),
    {
      status: "running",
      phase: "downloading",
      downloadedBytes: 512,
      totalBytes: 1024,
      targetVersion: "1.2.3",
      targetTag: "v1.2.3",
      error: null,
    },
  );
});

test("normalizes an object payload and preserves a generic unknown phase", () => {
  assert.deepEqual(
    normalizeApplicationUpgradeProgress({
      phase: "future_phase",
      downloadedBytes: -1,
      totalBytes: Number.POSITIVE_INFINITY,
      error: "could not apply update",
    }),
    {
      status: null,
      phase: "unknown",
      downloadedBytes: null,
      totalBytes: null,
      targetVersion: null,
      targetTag: null,
      error: "could not apply update",
    },
  );
});

test("returns null for malformed or non-object progress", () => {
  assert.equal(normalizeApplicationUpgradeProgress("not json"), null);
  assert.equal(normalizeApplicationUpgradeProgress("[]"), null);
  assert.equal(normalizeApplicationUpgradeProgress(null), null);
});

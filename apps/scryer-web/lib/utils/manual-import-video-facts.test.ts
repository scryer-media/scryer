import assert from "node:assert/strict";
import test from "node:test";

import { formatManualImportVideoFacts } from "./manual-import-video-facts.ts";

test("manual import video facts render the useful media summary", () => {
  assert.equal(
    formatManualImportVideoFacts({
      containerFormat: "matroska",
      videoCodec: "hevc",
      audioCodec: "eac3",
      videoWidth: 3840,
      videoHeight: 1604,
      durationSeconds: 6016,
    }),
    "matroska · 3840×1604 · HEVC · E-AC-3 · 1h 40m",
  );
});

test("manual import video facts explain legacy formats without probe details", () => {
  assert.equal(
    formatManualImportVideoFacts(null),
    "Media details unavailable for this format",
  );
});

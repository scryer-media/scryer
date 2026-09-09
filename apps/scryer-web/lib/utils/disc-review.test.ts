import assert from "node:assert/strict";
import test from "node:test";
import type { MediaAnalysisAttempt, MediaDiscMetadata, MediaDiscTitle } from "../types/media-analysis.ts";
import { discEpisodeSelections, discReviewInventory, discTitleIdentity } from "./disc-review.ts";

const report: MediaAnalysisAttempt["report"] = {
  status: "INCOMPLETE", bytesRead: 0, seeks: 0, elapsedMs: 1, budgetExhausted: false, warnings: [],
};
function title(id: string, aliases: string[] = []): MediaDiscTitle {
  return { id, aliases, durationSeconds: 1800, angleCount: 1, segments: [], chapters: [], streams: [], report };
}
const saved: MediaDiscMetadata = {
  discType: "bluray", filesystem: "UDF", volumeLabel: null, titles: [title("00001"), title("00002")],
  selectedTitleId: "00001", automaticSelection: false,
  selection: { titleId: "00001", episodeMappings: [{ discTitleId: "00002", episodeIds: ["episode-2"] }] },
};
const inspected: MediaDiscMetadata = {
  ...saved, titles: [title("00003", ["00002"]), title("00004")], selectedTitleId: "00004",
  selection: { titleId: "00004", episodeMappings: [{ discTitleId: "00004", episodeIds: ["rejected-episode"] }] },
};
const attempt: MediaAnalysisAttempt = { revision: 1, attemptedAt: "2026-09-08T00:00:00Z", succeeded: false, report, disc: inspected };

test("failed disc inspection shows new titles while keeping saved selection and coverage", () => {
  const inventory = discReviewInventory(saved, attempt);
  assert.deepEqual(inventory?.titles.map((item) => item.id), ["00003", "00004"]);
  assert.equal(discTitleIdentity(saved.selection.titleId!, inventory), "00001", "retain the missing override for explicit review");
  assert.deepEqual(discEpisodeSelections(saved, inventory), { "00003": "episode-2" });
  assert.equal(saved.selection.titleId, "00001");
  assert.equal(saved.selection.episodeMappings[0].discTitleId, "00002");
});

test("legacy or successful attempts retain the stored inventory", () => {
  assert.equal(discReviewInventory(saved), saved);
  assert.equal(discReviewInventory(saved, { ...attempt, disc: undefined }), saved);
  assert.equal(discReviewInventory(saved, { ...attempt, succeeded: true }), saved);
  assert.deepEqual(discEpisodeSelections(saved, { ...inspected, titles: [] }), { "00002": "episode-2" });
  assert.equal(discReviewInventory(null, attempt), inspected);
});

import assert from "node:assert/strict";
import test from "node:test";
import type { MediaAnalysisAttempt, MediaAnalysisDetails, MediaDiscMetadata, MediaDiscTitle, MediaStreamDetail } from "../types/media-analysis.ts";
import {
  discEpisodeSelections,
  discNeedsReview,
  discOutcome,
  discReviewInventory,
  discTitleAudioCount,
  discTitleIdentity,
  discTitleLabel,
  discTitleSelectable,
  discTitleVideoSummary,
  formatDiscDuration,
  hasDiscInventory,
} from "./disc-review.ts";

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

test("disc controls are offered only when an inventory exists", () => {
  assert.equal(hasDiscInventory(saved), true, "stored disc metadata is reviewable");
  assert.equal(hasDiscInventory(null, attempt), true, "a failed inspection supplies an inventory of its own");
  assert.equal(hasDiscInventory(null), false, "a plain file has nothing to review");
  assert.equal(hasDiscInventory(null, { ...attempt, disc: undefined }), false);
  assert.equal(hasDiscInventory(null, { ...attempt, succeeded: true, disc: inspected }), false, "a successful attempt defers to the stored inventory");
});

test("legacy or successful attempts retain the stored inventory", () => {
  assert.equal(discReviewInventory(saved), saved);
  assert.equal(discReviewInventory(saved, { ...attempt, disc: undefined }), saved);
  assert.equal(discReviewInventory(saved, { ...attempt, succeeded: true }), saved);
  assert.deepEqual(discEpisodeSelections(saved, { ...inspected, titles: [] }), { "00002": "episode-2" });
  assert.equal(discReviewInventory(null, attempt), inspected);
});

const complete = { ...report, status: "COMPLETE" as const };
function analysisFor(disc: MediaDiscMetadata | null, status: MediaAnalysisAttempt["report"]["status"] = "COMPLETE"): MediaAnalysisDetails {
  return {
    revision: 1, durationSeconds: 1800, overallBitrateBps: null, selectedProgramId: null, selectedVideoId: null,
    report: { ...complete, status }, programs: [], streams: [], chapters: [], attachments: [], captionServices: [], disc,
  } as unknown as MediaAnalysisDetails;
}

test("disc outcome tells the four states apart and flags only the two needing a person", () => {
  const automatic: MediaDiscMetadata = {
    ...saved, titles: [{ ...title("00001"), report: complete }], selectedTitleId: "00001", automaticSelection: true,
    selection: { titleId: null, episodeMappings: [] },
  };
  assert.deepEqual(discOutcome(analysisFor(automatic)), { kind: "automatic", titleId: "00001" });
  assert.deepEqual(discOutcome(analysisFor({ ...automatic, automaticSelection: false })), { kind: "manual", titleId: "00001" });
  assert.deepEqual(discOutcome(analysisFor({ ...automatic, selectedTitleId: null })), { kind: "unselected" });
  assert.deepEqual(discOutcome(analysisFor(automatic, "INCOMPLETE")), { kind: "incomplete", status: "INCOMPLETE" });
  assert.deepEqual(discOutcome(analysisFor(automatic), attempt), { kind: "incomplete", status: "INCOMPLETE" },
    "a failed re-inspection outranks the saved outcome");
  assert.equal(discOutcome(analysisFor(null)), null, "a plain file has no disc outcome");
  assert.equal(discOutcome(null), null);
  assert.equal(discNeedsReview({ kind: "automatic", titleId: "00001" }), false);
  assert.equal(discNeedsReview({ kind: "manual", titleId: "00001" }), false);
  assert.equal(discNeedsReview({ kind: "unselected" }), true);
  assert.equal(discNeedsReview({ kind: "incomplete", status: "ENCRYPTED" }), true);
  assert.equal(discNeedsReview(null), false);
});

test("disc title presentation helpers", () => {
  assert.equal(formatDiscDuration(6725), "1h 52m");
  assert.equal(formatDiscDuration(1390), "23m 10s");
  assert.equal(formatDiscDuration(45), "45s");
  assert.equal(formatDiscDuration(null), null);
  assert.equal(formatDiscDuration(Number.NaN), null);
  const video = { kind: "VIDEO", codec: "hevc", width: 3840, height: 2160 } as MediaStreamDetail;
  const audio = { kind: "AUDIO", codec: "truehd" } as MediaStreamDetail;
  const rich: MediaDiscTitle = { ...title("00007"), report: complete, streams: [video, audio, audio], durationSeconds: 6725 };
  assert.equal(discTitleVideoSummary(rich), "2160p HEVC");
  assert.equal(discTitleVideoSummary(title("00008")), null);
  assert.equal(discTitleAudioCount(rich), 2);
  assert.equal(discTitleSelectable(rich), true);
  assert.equal(discTitleSelectable(title("00009")), false, "an incompletely inspected title cannot be chosen");
  const t = (key: string, values?: Record<string, string | number>) =>
    key === "mediaFile.discTitleIdentity" ? `Title ${values?.id}` : key === "label.unknown" ? "Unknown" : key;
  assert.equal(discTitleLabel(rich, t), "Title 00007 · 1h 52m · complete");
  assert.equal(discTitleLabel({ ...title("00009"), durationSeconds: null }, t), "Title 00009 · Unknown · incomplete");
});

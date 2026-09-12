import assert from "node:assert/strict";
import test from "node:test";
import {
  episodeAvailabilityPill,
  type EpisodeMediaAvailability,
} from "./episode-media-availability.ts";

const translate = (key: string) =>
  ({
    "episode.fileOnDisk": "On disk",
    "mediaFile.pendingScan": "Pending scan",
    "mediaFile.scanFailed": "Scan failed",
    "mediaFile.reviewRequired": "Review required",
    "episode.missing": "Missing",
  })[key] ?? key;

function availability(
  state: EpisodeMediaAvailability["state"],
  primaryQualityLabel: string | null = null,
): EpisodeMediaAvailability {
  return { state, primaryQualityLabel };
}

test("episode availability maps every compact state to its collapsed-row pill", () => {
  assert.deepEqual(
    episodeAvailabilityPill(availability("AVAILABLE", "1080p"), translate),
    { tone: "positive", label: "1080p" },
  );
  assert.deepEqual(
    episodeAvailabilityPill(availability("AVAILABLE"), translate),
    { tone: "positive", label: "On disk" },
  );
  assert.deepEqual(
    episodeAvailabilityPill(availability("PENDING_SCAN"), translate),
    { tone: "warning", label: "Pending scan" },
  );
  assert.deepEqual(
    episodeAvailabilityPill(availability("SCAN_FAILED"), translate),
    { tone: "negative", label: "Scan failed" },
  );
  assert.deepEqual(
    episodeAvailabilityPill(availability("REVIEW_REQUIRED"), translate),
    { tone: "warning", label: "Review required" },
  );
  assert.deepEqual(
    episodeAvailabilityPill(availability("MISSING"), translate),
    { tone: "warning", label: "Missing" },
  );
  assert.equal(
    episodeAvailabilityPill(availability("UNMONITORED"), translate),
    null,
  );
});

export type EpisodeMediaAvailability = {
  state:
    | "AVAILABLE"
    | "PENDING_SCAN"
    | "SCAN_FAILED"
    | "REVIEW_REQUIRED"
    | "MISSING"
    | "UNMONITORED";
  primaryQualityLabel: string | null;
};

export type EpisodeAvailabilityPill = {
  tone: "positive" | "warning" | "negative";
  label: string;
};

export function episodeAvailabilityPill(
  availability: EpisodeMediaAvailability,
  translate: (key: string) => string,
): EpisodeAvailabilityPill | null {
  switch (availability.state) {
    case "AVAILABLE":
      return {
        tone: "positive",
        label: availability.primaryQualityLabel || translate("episode.fileOnDisk"),
      };
    case "PENDING_SCAN":
      return { tone: "warning", label: translate("mediaFile.pendingScan") };
    case "REVIEW_REQUIRED":
      return { tone: "warning", label: translate("mediaFile.reviewRequired") };
    case "SCAN_FAILED":
      return { tone: "negative", label: translate("mediaFile.scanFailed") };
    case "MISSING":
      return { tone: "warning", label: translate("episode.missing") };
    case "UNMONITORED":
      return null;
  }
}

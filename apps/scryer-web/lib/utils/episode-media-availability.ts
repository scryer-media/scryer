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

export type EpisodeRowAvailabilityPill = EpisodeAvailabilityPill & {
  muted: boolean;
};

export function episodeAvailabilityPill(
  availability: EpisodeMediaAvailability,
  translate: (key: string) => string,
): EpisodeAvailabilityPill | null {
  switch (availability.state) {
    case "AVAILABLE":
      return {
        tone: "positive",
        label:
          availability.primaryQualityLabel || translate("episode.fileOnDisk"),
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

/**
 * Episode rows say when there is no file even for an unmonitored episode.
 * UNMONITORED is only ever reported for an episode with no file. Having a file
 * or not is the normal case for a season, so only states that need attention
 * stay at full strength.
 */
export function episodeRowAvailabilityPill(
  availability: EpisodeMediaAvailability,
  translate: (key: string) => string,
): EpisodeRowAvailabilityPill {
  const pill = episodeAvailabilityPill(availability, translate);
  if (!pill || availability.state === "MISSING") {
    return {
      tone: "warning",
      label: translate("episode.missing"),
      muted: true,
    };
  }
  return { ...pill, muted: availability.state === "AVAILABLE" };
}

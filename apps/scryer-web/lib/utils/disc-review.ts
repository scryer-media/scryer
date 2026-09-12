import type {
  MediaAnalysisAttempt,
  MediaAnalysisDetails,
  MediaDiscMetadata,
  MediaDiscTitle,
  MediaProbeStatus,
} from "../types/media-analysis";

/**
 * The title inventory a person should be looking at: a failed inspection's own
 * titles when it produced any, otherwise the saved ones.
 */
export function discReviewInventory(saved: MediaDiscMetadata | null, attempt?: MediaAnalysisAttempt | null) {
  return attempt && !attempt.succeeded ? attempt.disc ?? saved : saved;
}

/** Whether this file is a disc image with titles to show at all. */
export function hasDiscInventory(saved: MediaDiscMetadata | null, attempt?: MediaAnalysisAttempt | null) {
  return discReviewInventory(saved, attempt) != null;
}

export function discTitleIdentity(id: string, inventory: MediaDiscMetadata | null) {
  return inventory?.titles.find((title) => title.id === id || title.aliases.includes(id))?.id ?? id;
}

export function discEpisodeSelections(saved: MediaDiscMetadata | null, inventory: MediaDiscMetadata | null) {
  return Object.fromEntries((saved?.selection.episodeMappings ?? []).map((mapping) => [
    discTitleIdentity(mapping.discTitleId, inventory), mapping.episodeIds[0] ?? "",
  ]));
}

/**
 * What the disc is doing right now, in the four states a person can be told:
 * a title plays and Scryer picked it, a title plays and the user picked it, no
 * title could be picked, or the chosen title's inspection did not complete.
 *
 * Only the last two need anyone's attention. The analyzer picks the longest
 * complete title on its own, so a clean movie disc never asks for a choice.
 */
export type DiscOutcome =
  | { kind: "automatic"; titleId: string }
  | { kind: "manual"; titleId: string }
  | { kind: "unselected" }
  | { kind: "incomplete"; status: MediaProbeStatus };

export function discOutcome(
  analysis: MediaAnalysisDetails | null | undefined,
  attempt?: MediaAnalysisAttempt | null,
): DiscOutcome | null {
  if (!analysis || !hasDiscInventory(analysis.disc, attempt)) return null;
  if (attempt && !attempt.succeeded) return { kind: "incomplete", status: attempt.report.status };
  const disc = analysis.disc;
  if (!disc || disc.selectedTitleId == null) return { kind: "unselected" };
  if (analysis.report.status !== "COMPLETE") return { kind: "incomplete", status: analysis.report.status };
  return { kind: disc.automaticSelection ? "automatic" : "manual", titleId: disc.selectedTitleId };
}

/** Whether the outcome is one a person has to act on. */
export function discNeedsReview(outcome: DiscOutcome | null): boolean {
  return outcome?.kind === "unselected" || outcome?.kind === "incomplete";
}

/** Only a completely inspected title can be chosen; the backend refuses the rest. */
export function discTitleSelectable(title: MediaDiscTitle): boolean {
  return title.report.status === "COMPLETE";
}

/** "1h 52m", "23m 08s", "45s"; null when the disc never established a duration. */
export function formatDiscDuration(seconds: number | null | undefined): string | null {
  if (seconds == null || !Number.isFinite(seconds) || seconds < 0) return null;
  const total = Math.round(seconds);
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const rest = total % 60;
  if (hours > 0) return `${hours}h ${String(minutes).padStart(2, "0")}m`;
  if (minutes > 0) return `${minutes}m ${String(rest).padStart(2, "0")}s`;
  return `${rest}s`;
}

/** "1080p H264" from the title's main video stream, or null when it has none. */
export function discTitleVideoSummary(title: MediaDiscTitle): string | null {
  const video = title.streams.find((stream) => stream.kind === "VIDEO");
  if (!video) return null;
  const parts = [video.height ? `${video.height}p` : null, video.codec ? video.codec.toUpperCase() : null]
    .filter((part): part is string => part != null);
  return parts.length ? parts.join(" ") : null;
}

export function discTitleAudioCount(title: MediaDiscTitle): number {
  return title.streams.filter((stream) => stream.kind === "AUDIO").length;
}

/** One-line label for a title where only text fits: "Title 00002 · 1h 52m · complete". */
export function discTitleLabel(
  title: MediaDiscTitle,
  t: (key: string, values?: Record<string, string | number>) => string,
): string {
  return [
    t("mediaFile.discTitleIdentity", { id: title.id }),
    formatDiscDuration(title.durationSeconds) ?? t("label.unknown"),
    title.report.status.toLowerCase(),
  ].join(" · ");
}

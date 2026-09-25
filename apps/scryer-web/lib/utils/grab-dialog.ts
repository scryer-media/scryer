import type { Release } from "@/lib/types/releases";
import type { TitleRecord } from "@/lib/types/titles";

export type TitleGapLabel = {
  /** i18n key describing the gap. */
  key: string;
  /** Interpolation values for that key. */
  params?: Record<string, number>;
  /** A satisfied title reads green; an outstanding gap reads neutral. */
  complete: boolean;
};

/**
 * Gap a candidate title still has, from the counts the catalog search already
 * returns. Episodic titles report how many monitored episodes are missing;
 * a movie is simply owned or wanted.
 */
export function titleGapLabel(title: TitleRecord): TitleGapLabel {
  if (isEpisodicFacet(title.facet)) {
    const monitored = title.episodesMonitored ?? title.episodesTotal ?? 0;
    const missing = Math.max(0, monitored - (title.episodesOwned ?? 0));
    return missing > 0
      ? { key: "grabDialog.gap.missing", params: { count: missing }, complete: false }
      : { key: "grabDialog.gap.complete", complete: true };
  }
  return titleHoldsFile(title)
    ? { key: "grabDialog.gap.complete", complete: true }
    : { key: "grabDialog.gap.wanted", complete: false };
}

function isEpisodicFacet(facet: string | null | undefined): boolean {
  const normalized = facet?.toUpperCase();
  return normalized === "SERIES" || normalized === "ANIME";
}

/** True when the target already holds media, so replacing a file is on offer (D7). */
export function titleHoldsFile(title: TitleRecord): boolean {
  return (title.episodesOwned ?? 0) > 0 || (title.sizeBytes ?? 0) > 0;
}

/** True when the chosen target takes a season/episode narrowing. */
export function titleIsEpisodic(title: TitleRecord | null): boolean {
  return title != null && isEpisodicFacet(title.facet);
}

/**
 * Distinct profile block codes across the releases being grabbed. Non-empty
 * means the operator has to acknowledge the rejection before the CTA enables.
 */
export function releaseRejectionCodes(releases: readonly Release[]): string[] {
  const codes = new Set<string>();
  for (const release of releases) {
    for (const code of release.qualityProfileDecision?.blockCodes ?? []) {
      codes.add(code);
    }
  }
  return [...codes];
}

/**
 * Season/episode narrowing sent to the token mutation. Both blank means
 * "resolve it from the release name", which is what the server does by
 * default (D11). A season alone targets the whole season. The server rejects
 * an episode without a season, so that sends nothing and the dialog blocks the
 * grab instead.
 */
export function episodeSubjectInput(
  season: string,
  episode: string,
): { season?: string; episode?: string } {
  const trimmedSeason = season.trim();
  const trimmedEpisode = episode.trim();
  if (!trimmedSeason) {
    return {};
  }
  if (!trimmedEpisode) {
    return { season: trimmedSeason };
  }
  return { season: trimmedSeason, episode: trimmedEpisode };
}

/** True when an episode is filled without a season — the server rejects that. */
export function episodeSubjectIncomplete(season: string, episode: string): boolean {
  return season.trim().length === 0 && episode.trim().length > 0;
}

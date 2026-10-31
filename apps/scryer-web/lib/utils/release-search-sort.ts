import type { Release } from "../types/index.ts";

export type ReleaseSearchSortKey = "recommended" | "score" | "size";
export type ReleaseSearchSortDirection = "asc" | "desc";

export function sortReleaseSearchResults(
  releases: Release[],
  key: ReleaseSearchSortKey,
  direction: ReleaseSearchSortDirection,
): Release[] {
  // The backend owns quality, eligibility and size-fit ranking. Recommended
  // always preserves that order, including when an incremental snapshot lands.
  if (key === "recommended") return releases;
  const factor = direction === "asc" ? 1 : -1;
  const value = (release: Release) =>
    key === "score"
      ? (release.qualityProfileDecision?.releaseScore ??
        Number.NEGATIVE_INFINITY)
      : (release.sizeBytes ?? 0);
  return [...releases].sort((left, right) => {
    const a = value(left);
    const b = value(right);
    // Ties retain server order, including two unscored results (-Infinity).
    return a === b ? 0 : (a < b ? -1 : 1) * factor;
  });
}

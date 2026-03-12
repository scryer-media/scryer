export const QUALITY_PROFILE_PREFIX = "scryer:quality-profile:";
export const ROOT_FOLDER_PREFIX = "scryer:root-folder:";
export const MONITOR_TYPE_PREFIX = "scryer:monitor-type:";
export const SEASON_FOLDER_PREFIX = "scryer:season-folder:";
export const MAL_SCORE_PREFIX = "scryer:mal-score:";
export const ANIME_MEDIA_TYPE_PREFIX = "scryer:anime-media-type:";
export const ANIME_STATUS_PREFIX = "scryer:anime-status:";
export const MONITOR_SPECIALS_PREFIX = "scryer:monitor-specials:";
export const INTER_SEASON_MOVIES_PREFIX = "scryer:inter-season-movies:";
export const FILLER_POLICY_PREFIX = "scryer:filler-policy:";
export const RECAP_POLICY_PREFIX = "scryer:recap-policy:";

/** Extract the value portion of a prefixed tag, or null if not present. */
export function getTagValue(tags: string[], prefix: string): string | null {
  const match = tags.find((t) => t.startsWith(prefix));
  return match ? match.slice(prefix.length) : null;
}

/** Replace (or append) a prefixed tag with a new value. */
export function setTagValue(
  tags: string[],
  prefix: string,
  value: string,
): string[] {
  const filtered = tags.filter((t) => !t.startsWith(prefix));
  return [...filtered, `${prefix}${value}`];
}

/** Remove the tag matching the given prefix. */
export function removeTagByPrefix(tags: string[], prefix: string): string[] {
  return tags.filter((t) => !t.startsWith(prefix));
}

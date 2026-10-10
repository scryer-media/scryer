/**
 * The title an episode shows anywhere in the UI. Upstream metadata can drop
 * an episode's title, which then reads as a translated "TBA" rather than a
 * blank or a missing segment.
 */
export function episodeTitleOrTba(
  title: string | null | undefined,
  t: (key: string) => string,
): string {
  return title?.trim() || t("episode.titleTba");
}

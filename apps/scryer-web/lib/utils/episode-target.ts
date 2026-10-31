import { episodeTitleOrTba } from "./episode-title.ts";
import { compareManualImportSeasonLabels } from "./manual-import-actions.ts";

/** An episode a file can be assigned to in the episode target picker. */
export type EpisodeTargetEpisode = {
  id: string;
  seasonNumber: string | null;
  episodeNumber: string | null;
  absoluteNumber?: string | null;
  title: string | null;
};

/** A series movie a manual import can target instead of an episode. */
export type EpisodeTargetSeriesMovie = {
  seriesMovieLinkId: string;
  movieTitle: string;
  year: number | null;
  runtimeMinutes: number | null;
};

export const UNASSIGNED_TARGET = "__unassigned__";
export const EPISODE_TARGET_PREFIX = "episode:";
export const SERIES_MOVIE_TARGET_PREFIX = "series-movie:";

/**
 * The picker label for an episode. Without `t` (search matching) an untitled
 * episode contributes only its numbers, so searching "TBA" matches nothing.
 */
export function episodeTargetLabel(
  ep: EpisodeTargetEpisode,
  t?: (key: string) => string,
): string {
  const season = ep.seasonNumber?.replace(/\D/g, "") ?? "";
  const episode = ep.episodeNumber?.replace(/\D/g, "") ?? "";
  const seasonTag = season ? season.padStart(2, "0") : "??";
  const episodeTag = episode ? episode.padStart(2, "0") : "??";
  const tag = `S${seasonTag}E${episodeTag}`;
  const absolute = ep.absoluteNumber?.trim();
  const title = t ? episodeTitleOrTba(ep.title, t) : ep.title?.trim();
  if (absolute) {
    return title ? `${tag} (${absolute}) ${title}` : `${tag} (${absolute})`;
  }
  return title ? `${tag} - ${title}` : tag;
}

export function seriesMovieTargetLabel(
  movie: EpisodeTargetSeriesMovie,
): string {
  const year = movie.year ? ` (${movie.year})` : "";
  const runtime = movie.runtimeMinutes ? ` • ${movie.runtimeMinutes} min` : "";
  return `${movie.movieTitle}${year}${runtime}`;
}

/**
 * Episodes grouped under `Season NN` keys, seasons in numeric order and
 * episodes in episode-number order within each season.
 */
export function groupEpisodesBySeason<TEpisode extends EpisodeTargetEpisode>(
  episodes: readonly TEpisode[],
): Map<string, TEpisode[]> {
  const groups = new Map<string, TEpisode[]>();
  for (const ep of episodes) {
    const season = ep.seasonNumber?.replace(/\D/g, "") ?? "0";
    const key = `Season ${season.padStart(2, "0")}`;
    const group = groups.get(key) ?? [];
    group.push(ep);
    groups.set(key, group);
  }
  for (const [key, group] of groups) {
    groups.set(
      key,
      group.sort((a, b) => {
        const aNum = Number.parseInt(
          a.episodeNumber?.replace(/\D/g, "") ?? "0",
          10,
        );
        const bNum = Number.parseInt(
          b.episodeNumber?.replace(/\D/g, "") ?? "0",
          10,
        );
        return aNum - bNum;
      }),
    );
  }
  return new Map(
    Array.from(groups.entries()).sort(([left], [right]) =>
      compareManualImportSeasonLabels(left, right),
    ),
  );
}

// One file can cover several episodes (`Show.S01E27E28...`), so an episode
// target carries the whole set. A single id keeps the value it always had.
export function episodeTargetValue(episodeIds: readonly string[]): string {
  return `${EPISODE_TARGET_PREFIX}${episodeIds.join(",")}`;
}

export function episodeTargetIds(value: string): string[] {
  if (!value.startsWith(EPISODE_TARGET_PREFIX)) return [];
  return value
    .slice(EPISODE_TARGET_PREFIX.length)
    .split(",")
    .map((id) => id.trim())
    .filter((id) => id.length > 0);
}

export function seriesMovieTargetValue(seriesMovieLinkId: string): string {
  return `${SERIES_MOVIE_TARGET_PREFIX}${seriesMovieLinkId}`;
}

/** The picker value that selects exactly these episodes, or unassigned. */
export function episodeSelectionTargetValue(
  episodeIds: readonly string[],
): string {
  return episodeIds.length > 0
    ? episodeTargetValue(episodeIds)
    : UNASSIGNED_TARGET;
}

export type EpisodeTargetRow =
  | { kind: "group"; key: string; label: string }
  | { kind: "option"; key: string; label: string; value: string };

export type EpisodeTargetRowLabels = {
  /** Label of the "leave unassigned" option; omit to offer no such option. */
  unassigned?: string;
  seriesMovies: string;
  /** Display label for a `Season NN` group key. */
  seasonGroup: (groupKey: string) => string;
};

/** The picker's rows for a filter query, groups before their options. */
export function buildEpisodeTargetRows<TEpisode extends EpisodeTargetEpisode>(
  groupedEpisodes: ReadonlyMap<string, readonly TEpisode[]>,
  seriesMovies: readonly EpisodeTargetSeriesMovie[],
  query: string,
  t: (key: string) => string,
  labels: EpisodeTargetRowLabels,
): EpisodeTargetRow[] {
  const normalizedQuery = query.trim().toLocaleLowerCase();
  const matches = (label: string, groupLabel = "") =>
    normalizedQuery.length === 0 ||
    `${groupLabel} ${label}`.toLocaleLowerCase().includes(normalizedQuery);
  const rows: EpisodeTargetRow[] = [];

  if (labels.unassigned !== undefined && matches(labels.unassigned)) {
    rows.push({
      kind: "option",
      key: UNASSIGNED_TARGET,
      label: labels.unassigned,
      value: UNASSIGNED_TARGET,
    });
  }

  const matchingSeriesMovies = seriesMovies.filter((movie) =>
    matches(seriesMovieTargetLabel(movie), labels.seriesMovies),
  );
  if (matchingSeriesMovies.length > 0) {
    rows.push({
      kind: "group",
      key: "group:series-movies",
      label: labels.seriesMovies,
    });
    matchingSeriesMovies.forEach((movie) => {
      const value = seriesMovieTargetValue(movie.seriesMovieLinkId);
      rows.push({
        kind: "option",
        key: value,
        label: seriesMovieTargetLabel(movie),
        value,
      });
    });
  }

  groupedEpisodes.forEach((episodesInSeason, groupKey) => {
    const groupLabel = labels.seasonGroup(groupKey);
    const matchingEpisodes = episodesInSeason.filter((episode) =>
      matches(episodeTargetLabel(episode), groupLabel),
    );
    if (matchingEpisodes.length === 0) {
      return;
    }
    rows.push({ kind: "group", key: `group:${groupKey}`, label: groupLabel });
    matchingEpisodes.forEach((episode) => {
      const value = episodeTargetValue([episode.id]);
      rows.push({
        kind: "option",
        key: value,
        label: episodeTargetLabel(episode, t),
        value,
      });
    });
  });

  return rows;
}

/** The season number a `Season NN` group key names, without padding. */
export function seasonGroupNumber(groupKey: string): string {
  const digits = groupKey.match(/\d+/)?.[0] ?? "0";
  return String(Number.parseInt(digits, 10));
}

/** Whether two episode selections name the same episodes, in any order. */
export function sameEpisodeSelection(
  left: readonly string[],
  right: readonly string[],
): boolean {
  const leftSet = new Set(left);
  const rightSet = new Set(right);
  if (leftSet.size !== rightSet.size) return false;
  for (const id of leftSet) {
    if (!rightSet.has(id)) return false;
  }
  return true;
}

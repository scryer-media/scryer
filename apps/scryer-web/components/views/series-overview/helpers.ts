import type {
  CollectionEpisode,
  EpisodeMediaFile,
  SeriesMovieLink,
  TitleCollection,
  TitleReleaseBlocklistEntry,
} from "@/components/containers/series-overview-container";
import type { UiDateTimeFormat } from "@/lib/types/settings";
// Relative import (not the `@/` alias) so this module stays loadable by plain
// `node --test`, which cannot resolve bundler path aliases. The type-only alias
// imports above are erased by type stripping and are fine.
import { formatUiDate } from "../../../lib/utils/date-format.ts";

export function formatDate(
  iso: string | null | undefined,
  dateTimeFormat: UiDateTimeFormat,
) {
  return formatUiDate(iso, dateTimeFormat, { fallback: "—" });
}

export function formatRuntimeFromMinutes(runtimeMinutes: number | null | undefined) {
  if (!runtimeMinutes || runtimeMinutes <= 0) {
    return null;
  }
  const hours = Math.floor(runtimeMinutes / 60);
  const minutes = runtimeMinutes % 60;
  if (hours === 0) {
    return `${minutes}m`;
  }
  return minutes > 0 ? `${hours}h ${minutes}m` : `${hours}h`;
}

export function formatRuntimeFromSeconds(runtimeSeconds: number | null | undefined) {
  if (!runtimeSeconds || runtimeSeconds <= 0) {
    return null;
  }
  return formatRuntimeFromMinutes(Math.floor(runtimeSeconds / 60));
}

export function dedupeInsensitive(values: string[]) {
  const seen = new Set<string>();
  const result: string[] = [];
  for (const value of values) {
    const trimmed = value?.trim();
    if (!trimmed) continue;
    const key = trimmed.toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    result.push(trimmed);
  }
  return result;
}

export function formatFileSize(bytes: number) {
  if (bytes <= 0) return "—";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  const val = bytes / Math.pow(1024, i);
  return `${val.toFixed(i > 0 ? 1 : 0)} ${units[i]}`;
}

export function deriveMediaFileQualityLabel(
  file: Pick<EpisodeMediaFile, "qualityLabel" | "resolution" | "videoWidth" | "videoHeight">,
) {
  if (file.videoWidth != null && file.videoWidth > 0) {
    if (file.videoWidth >= 3840) return "4K";
    if (file.videoWidth >= 1920) return "1080p";
    if (file.videoWidth >= 1280) return "720p";
  }
  if (file.videoHeight != null && file.videoHeight > 0) {
    return `${file.videoHeight}p`;
  }
  const parsedLabel = file.qualityLabel?.trim() || file.resolution?.trim();
  return parsedLabel || null;
}

export function parseSeasonSortValue(collection: TitleCollection) {
  const key = collection.narrativeOrder ?? collection.collectionIndex ?? "";
  const match = key.match(/\d+(\.\d+)?/);
  if (!match) {
    const fallback = `${collection.collectionIndex ?? ""} ${collection.label ?? ""}`;
    const fallbackMatch = fallback.match(/\d+/);
    return fallbackMatch ? Number.parseInt(fallbackMatch[0], 10) : Number.MAX_SAFE_INTEGER;
  }
  return Number.parseFloat(match[0]);
}

export function isSpecialsCollection(collection: TitleCollection) {
  return collection.collectionType === "SPECIALS"
    || (collection.collectionType === "SEASON" && parseSeasonSortValue(collection) === 0);
}

export function seasonHeading(
  collection: TitleCollection,
  t: (key: string, values?: Record<string, string | number | boolean | null | undefined>) => string,
) {
  const label = collection.label?.trim();
  if (isSpecialsCollection(collection)) {
    return t("title.specials");
  }
  const indexValue = collection.collectionIndex.trim();
  const normalizedIndex = indexValue.match(/^\d+$/)
    ? indexValue === "0"
      ? t("title.specials")
      : t("title.seasonNumber", { number: indexValue })
    : indexValue;
  if (label && normalizedIndex && normalizedIndex !== t("title.specials")) {
    return `${normalizedIndex}: ${label}`;
  }
  if (label) {
    return label;
  }
  return normalizedIndex.length > 0
    ? normalizedIndex
    : t("title.seasonNumber", { number: "" }).trim();
}

export function episodeSortValue(episode: CollectionEpisode) {
  if (!episode.episodeNumber) {
    return Number.MAX_SAFE_INTEGER;
  }
  const match = episode.episodeNumber.match(/\d+/);
  if (!match) {
    return Number.MAX_SAFE_INTEGER;
  }
  return Number.parseInt(match[0], 10);
}

export function isEpisodeCountableForProgress(episode: CollectionEpisode) {
  const title = episode.title?.trim();
  const airDate = episode.airDate?.trim();

  if (!title || !airDate) {
    return false;
  }

  const normalizedTitle = title.toUpperCase();
  return normalizedTitle !== "TBA" && normalizedTitle !== "TBD";
}

export function parseNumberToken(raw: string | null | undefined): number | null {
  const match = raw?.match(/\d+/);
  if (!match) {
    return null;
  }
  const value = Number.parseInt(match[0], 10);
  return Number.isFinite(value) ? value : null;
}

export function episodeKey(season: number, episode: number): string {
  return `${season}-${episode}`;
}

export function extractEpisodeKeysFromReleaseTitle(raw: string | null | undefined): Set<string> {
  if (!raw) {
    return new Set();
  }
  const title = raw.toUpperCase();
  const keys = new Set<string>();

  const seasonEpisodePattern = /S(\d{1,3})E(\d{1,4})(?:E(\d{1,4}))?/g;
  for (const match of title.matchAll(seasonEpisodePattern)) {
    const season = Number.parseInt(match[1], 10);
    const firstEpisode = Number.parseInt(match[2], 10);
    if (!Number.isFinite(season) || !Number.isFinite(firstEpisode)) {
      continue;
    }
    keys.add(episodeKey(season, firstEpisode));
    if (match[3]) {
      const secondEpisode = Number.parseInt(match[3], 10);
      if (Number.isFinite(secondEpisode)) {
        keys.add(episodeKey(season, secondEpisode));
      }
    }
  }

  const xPattern = /\b(\d{1,3})X(\d{1,4})(?:-(\d{1,4}))?\b/g;
  for (const match of title.matchAll(xPattern)) {
    const season = Number.parseInt(match[1], 10);
    const firstEpisode = Number.parseInt(match[2], 10);
    if (!Number.isFinite(season) || !Number.isFinite(firstEpisode)) {
      continue;
    }
    keys.add(episodeKey(season, firstEpisode));
    if (match[3]) {
      const secondEpisode = Number.parseInt(match[3], 10);
      if (Number.isFinite(secondEpisode)) {
        keys.add(episodeKey(season, secondEpisode));
      }
    }
  }

  return keys;
}

export function blocklistEntryMatchesEpisode(
  entry: TitleReleaseBlocklistEntry,
  episode: CollectionEpisode,
  collection: TitleCollection,
): boolean {
  // A blocklist entry blocks a release for the whole title, so it is shown
  // against the episodes its name identifies -- and against every episode when
  // the name identifies none.
  const keys = extractEpisodeKeysFromReleaseTitle(entry.releaseName);
  if (keys.size === 0) {
    return true;
  }

  const season = parseNumberToken(episode.seasonNumber) ?? parseNumberToken(collection.collectionIndex);
  const episodeNumber = parseNumberToken(episode.episodeNumber);
  if (season == null || episodeNumber == null) {
    return false;
  }
  return keys.has(episodeKey(season, episodeNumber));
}

export type SeriesTimelineItem =
  | {
      kind: "collection";
      key: string;
      collection: TitleCollection;
      sortValue: number;
    }
  | {
      kind: "seriesMovie";
      key: string;
      link: SeriesMovieLink;
      sortValue: number;
    };

export function seriesMovieTimelineSortValue(link: SeriesMovieLink) {
  const narrativeOrder = link.narrativeOrder?.trim();
  if (narrativeOrder && /^-?\d+(?:\.\d+)?$/.test(narrativeOrder)) {
    const value = Number.parseFloat(narrativeOrder);
    if (Number.isFinite(value)) {
      return value;
    }
  }

  if (link.afterSeason != null) {
    return link.afterSeason + 0.5;
  }

  if (link.beforeSeason != null) {
    return link.beforeSeason - 0.5;
  }

  return 0.5;
}

export function buildSeriesTimelineItems(
  collections: TitleCollection[],
  seriesMovieLinks: SeriesMovieLink[],
): SeriesTimelineItem[] {
  const items: SeriesTimelineItem[] = [
    ...collections.map((collection) => ({
      kind: "collection" as const,
      key: `s-${collection.id}`,
      collection,
      sortValue: parseSeasonSortValue(collection),
    })),
    ...seriesMovieLinks.map((link) => ({
      kind: "seriesMovie" as const,
      key: `m-${link.id}`,
      link,
      sortValue: seriesMovieTimelineSortValue(link),
    })),
  ];

  return items.sort(compareSeriesTimelineItems);
}

function compareSeriesTimelineItems(
  left: SeriesTimelineItem,
  right: SeriesTimelineItem,
) {
  const leftSpecials = left.kind === "collection" && isSpecialsCollection(left.collection);
  const rightSpecials = right.kind === "collection" && isSpecialsCollection(right.collection);
  if (leftSpecials !== rightSpecials) {
    return leftSpecials ? 1 : -1;
  }

  if (left.sortValue !== right.sortValue) {
    return right.sortValue - left.sortValue;
  }

  if (left.kind !== right.kind) {
    return left.kind === "collection" ? -1 : 1;
  }

  if (left.kind === "collection" && right.kind === "collection") {
    return right.collection.collectionIndex.localeCompare(left.collection.collectionIndex)
      || right.collection.id.localeCompare(left.collection.id);
  }

  if (left.kind === "seriesMovie" && right.kind === "seriesMovie") {
    return left.link.movie.title.localeCompare(right.link.movie.title)
      || left.link.id.localeCompare(right.link.id);
  }

  return 0;
}

/**
 * Sort DB collections: non-specials descending (newest first), specials (season 0) at the end.
 */
export function sortDbCollections(collections: TitleCollection[]) {
  return [...collections].sort((left, right) => {
    const leftVal = parseSeasonSortValue(left);
    const rightVal = parseSeasonSortValue(right);
    if (leftVal === 0 && rightVal !== 0) return 1;
    if (rightVal === 0 && leftVal !== 0) return -1;
    if (leftVal !== rightVal) return rightVal - leftVal;
    return right.collectionIndex.localeCompare(left.collectionIndex);
  });
}

/**
 * Find the key of the most recent (highest-numbered, non-specials) season to auto-expand.
 */
export function findLatestSeasonKey(collections: TitleCollection[]): string | null {
  if (collections.length === 0) return null;
  const nonSpecials = collections.filter((c) => !isSpecialsCollection(c));
  if (nonSpecials.length === 0) return null;
  const latest = nonSpecials.reduce((best, current) =>
    parseSeasonSortValue(current) > parseSeasonSortValue(best)
      ? current
      : best,
  );
  return `s-${latest.id}`;
}

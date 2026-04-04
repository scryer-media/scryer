import type {
  CollectionEpisode,
  EpisodeMediaFile,
  InterstitialMovieMetadata,
  TitleCollection,
  TitleReleaseBlocklistEntry,
} from "@/components/containers/series-overview-container";

export function formatDate(iso: string | null | undefined) {
  if (!iso) {
    return "—";
  }
  try {
    return new Date(iso).toLocaleDateString(undefined, {
      year: "numeric",
      month: "short",
      day: "numeric",
    });
  } catch {
    return iso;
  }
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

export function getImdbUrl(imdbId: string | null | undefined) {
  if (!imdbId) return null;
  const trimmed = imdbId.trim();
  if (!trimmed) return null;
  if (trimmed.startsWith("tt")) {
    return `https://www.imdb.com/title/${trimmed}`;
  }
  return `https://www.imdb.com/find?q=${encodeURIComponent(trimmed)}&s=tt`;
}

export function getTvdbMovieUrl(metadata: InterstitialMovieMetadata) {
  const tvdbId = String(metadata.tvdbId).trim();
  if (!tvdbId) return null;
  const slug = metadata.slug?.trim();
  const base = "https://www.thetvdb.com";
  if (slug) {
    return `${base}/movies/${tvdbId}-${encodeURIComponent(slug)}`;
  }
  return `${base}/?id=${encodeURIComponent(tvdbId)}`;
}

export function normalizeMovieCollectionLabel(label: string | null | undefined) {
  if (!label) return null;
  const trimmed = label.trim();
  if (!trimmed) return null;
  return /^movie\s+\d+$/i.test(trimmed) ? null : trimmed;
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
  if (collection.collectionType === "interstitial") {
    return false;
  }
  return collection.collectionType === "specials"
    || (collection.collectionType === "season" && parseSeasonSortValue(collection) === 0);
}

export function seasonHeading(collection: TitleCollection) {
  if (collection.collectionType === "interstitial") {
    return collection.label?.trim() || "Movie";
  }
  const label = collection.label?.trim();
  if (isSpecialsCollection(collection)) {
    return !label || /^season\s*0+$/i.test(label) ? "Specials" : label;
  }
  const indexValue = collection.collectionIndex.trim();
  const normalizedIndex = indexValue.match(/^\d+$/)
    ? indexValue === "0"
      ? "Specials"
      : `Season ${indexValue}`
    : indexValue;
  if (label && normalizedIndex && normalizedIndex !== "Specials") {
    return `${normalizedIndex}: ${label}`;
  }
  if (label) {
    return label;
  }
  return normalizedIndex.length > 0 ? normalizedIndex : "Season";
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
  const season = parseNumberToken(episode.seasonNumber) ?? parseNumberToken(collection.collectionIndex);
  const episodeNumber = parseNumberToken(episode.episodeNumber);
  if (season == null || episodeNumber == null) {
    return false;
  }
  const keys = extractEpisodeKeysFromReleaseTitle(entry.sourceTitle);
  return keys.has(episodeKey(season, episodeNumber));
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

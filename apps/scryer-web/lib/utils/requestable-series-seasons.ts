import { parseSearchSeason } from "./automatic-search.ts";

type CatalogSeason = {
  collectionType: string;
  collectionIndex: string;
  label: string | null;
  monitored: boolean;
  episodeRecordsTotal: number | null;
  episodesMonitored: number | null;
};

export function requestableSeriesSeasons(
  collections: readonly CatalogSeason[],
) {
  const seasons = new Map<number, string>();
  for (const collection of collections) {
    if (collection.collectionType.toUpperCase() !== "SEASON") continue;
    const number = parseSearchSeason(collection.collectionIndex);
    if (number === null || number === 0 || collection.episodeRecordsTotal === 0)
      continue;
    if (
      collection.monitored &&
      collection.episodeRecordsTotal !== null &&
      collection.episodesMonitored !== null &&
      collection.episodesMonitored >= collection.episodeRecordsTotal
    ) {
      continue;
    }
    seasons.set(number, collection.label?.trim() ?? "");
  }
  return [...seasons.entries()].sort(([left], [right]) => left - right);
}

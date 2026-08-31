import type { DownloadQueueItem } from "@/lib/types/download-queue";
import { isPendingDownloadQueueItem } from "./catalog-download-activity.ts";

export type EpisodeDownloadActivityInput = Pick<
  DownloadQueueItem,
  "displayState" | "episodeId" | "queueScope"
>;

type EpisodeIdRecord = {
  id: string;
};

export function coveredEpisodeIdsForQueueItem(
  item: EpisodeDownloadActivityInput,
  episodesByCollection: Readonly<Record<string, readonly EpisodeIdRecord[]>>,
): string[] {
  const episodeIds = new Set<string>();
  if (item.episodeId) {
    episodeIds.add(item.episodeId);
  }

  const scope = item.queueScope;
  if (!scope) {
    return [...episodeIds];
  }

  if (scope.__typename === "EpisodeScopePayload") {
    episodeIds.add(scope.episodeId);
  } else if (scope.__typename === "EpisodeSetScopePayload") {
    for (const episodeId of scope.episodeIds) {
      episodeIds.add(episodeId);
    }
  } else if (scope.__typename === "CollectionScopePayload") {
    for (const episode of episodesByCollection[scope.collectionId] ?? []) {
      episodeIds.add(episode.id);
    }
  }

  return [...episodeIds];
}

export function collectActiveDownloadEpisodeIds(
  items: readonly EpisodeDownloadActivityInput[],
  episodesByCollection: Readonly<Record<string, readonly EpisodeIdRecord[]>>,
): Set<string> {
  const episodeIds = new Set<string>();
  for (const item of items) {
    if (!isPendingDownloadQueueItem(item)) {
      continue;
    }

    for (const episodeId of coveredEpisodeIdsForQueueItem(
      item,
      episodesByCollection,
    )) {
      episodeIds.add(episodeId);
    }
  }
  return episodeIds;
}

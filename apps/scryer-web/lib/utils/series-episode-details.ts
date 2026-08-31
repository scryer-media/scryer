export type EpisodeDetailMergeEpisode = {
  id: string;
  overview?: string | null;
  imageUrl?: string | null;
  playbackLinks?: unknown;
};

export type EpisodeDetailMergeCollection<
  Episode extends EpisodeDetailMergeEpisode = EpisodeDetailMergeEpisode,
> = {
  id: string;
  episodes?: Episode[] | null;
};

export function episodeIdsForCollections(
  collections: readonly EpisodeDetailMergeCollection[],
): Set<string> {
  const ids = new Set<string>();
  for (const collection of collections) {
    for (const episode of collection.episodes ?? []) {
      ids.add(episode.id);
    }
  }
  return ids;
}

export function episodeIdsForEpisodeRecord(
  episodesByCollection: Record<string, readonly EpisodeDetailMergeEpisode[]>,
): Set<string> {
  const ids = new Set<string>();
  for (const episodes of Object.values(episodesByCollection)) {
    for (const episode of episodes) {
      ids.add(episode.id);
    }
  }
  return ids;
}

export function mergeLoadedEpisodeDetailsForCollections<
  Episode extends EpisodeDetailMergeEpisode,
>(
  nextCollections: readonly EpisodeDetailMergeCollection<Episode>[],
  currentEpisodesByCollection: Record<
    string,
    readonly EpisodeDetailMergeEpisode[]
  >,
  loadedEpisodeIds: ReadonlySet<string>,
): Record<string, Episode[]> {
  const loadedDetailsByEpisodeId = new Map<
    string,
    Pick<EpisodeDetailMergeEpisode, "overview" | "imageUrl" | "playbackLinks">
  >();
  for (const episodes of Object.values(currentEpisodesByCollection)) {
    for (const episode of episodes) {
      if (!loadedEpisodeIds.has(episode.id)) {
        continue;
      }
      loadedDetailsByEpisodeId.set(episode.id, {
        overview: episode.overview,
        imageUrl: episode.imageUrl,
        playbackLinks: episode.playbackLinks,
      });
    }
  }

  return Object.fromEntries(
    nextCollections.map((collection) => [
      collection.id,
      (collection.episodes ?? []).map((episode) => {
        const loadedDetail = loadedDetailsByEpisodeId.get(episode.id);
        return loadedDetail
          ? {
              ...episode,
              overview: loadedDetail.overview ?? episode.overview ?? null,
              imageUrl: loadedDetail.imageUrl ?? episode.imageUrl ?? null,
              playbackLinks: (loadedDetail.playbackLinks ??
                episode.playbackLinks) as Episode["playbackLinks"],
            }
          : episode;
      }),
    ]),
  );
}

export function pruneEpisodeRecord<Value>(
  current: Record<string, Value>,
  episodeIds: ReadonlySet<string>,
): Record<string, Value> {
  if (Object.keys(current).every((episodeId) => episodeIds.has(episodeId))) {
    return current;
  }

  return Object.fromEntries(
    Object.entries(current).filter(([episodeId]) => episodeIds.has(episodeId)),
  );
}

export function pruneSeriesMovieLinkMediaFiles<
  File extends { episodeId: string | null },
>(
  current: Record<string, File[]>,
  episodeIds: ReadonlySet<string>,
): Record<string, File[]> {
  let changed = false;
  const next: Record<string, File[]> = {};
  for (const [linkId, files] of Object.entries(current)) {
    const retained = files.filter(
      (file) => file.episodeId === null || episodeIds.has(file.episodeId),
    );
    if (retained.length === 0) {
      changed = true;
      continue;
    }
    if (retained.length !== files.length) {
      changed = true;
      next[linkId] = retained;
    } else {
      next[linkId] = files;
    }
  }

  return changed ? next : current;
}

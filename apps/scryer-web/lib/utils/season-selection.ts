// No imports, so this module stays loadable by plain `node --test`.

/**
 * Seasons whose header checkbox was ticked before their episodes loaded, and
 * that have since loaded. `episodeIds` are the episodes to add to the
 * selection, minus any an in-flight deletion job holds; `remaining` are the
 * seasons still waiting on their episodes. Null when no queued season has
 * loaded yet, so the caller can skip a state update.
 */
export function resolveQueuedSeasonSelections(
  queuedCollectionIds: ReadonlySet<string>,
  episodesByCollection: Readonly<Record<string, readonly { id: string }[]>>,
  lockedEpisodeIds: ReadonlySet<string> | undefined,
): { episodeIds: string[]; remaining: Set<string> } | null {
  const episodeIds: string[] = [];
  const remaining = new Set<string>();
  let resolvedAny = false;

  for (const collectionId of queuedCollectionIds) {
    const episodes = episodesByCollection[collectionId];
    if (!episodes) {
      remaining.add(collectionId);
      continue;
    }
    resolvedAny = true;
    for (const episode of episodes) {
      if (!(lockedEpisodeIds?.has(episode.id) ?? false)) {
        episodeIds.push(episode.id);
      }
    }
  }

  return resolvedAny ? { episodeIds, remaining } : null;
}

/**
 * Whether a season's header checkbox has anything to select. A loaded season
 * counts its unlocked episodes; a season still collapsed and unloaded counts
 * on the episode total the overview carries for it, and is selectable unless
 * that total is known to be zero.
 */
export function seasonHasSelectableEpisodes({
  episodesReady,
  selectableEpisodeCount,
  episodeRecordsTotal,
}: {
  episodesReady: boolean;
  selectableEpisodeCount: number;
  episodeRecordsTotal: number | null | undefined;
}): boolean {
  if (episodesReady) {
    return selectableEpisodeCount > 0;
  }
  return typeof episodeRecordsTotal === "number"
    ? episodeRecordsTotal > 0
    : true;
}

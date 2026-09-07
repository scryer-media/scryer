import type { Release, ReleaseQueueScope } from "@/lib/types/releases";

export type QueueDownloadScopeInput =
  | { episode: string }
  | { episodeSet: string[] }
  | { collection: string }
  | { seriesMovie: string }
  | { title: boolean };

function queueScopeToInput(scope: ReleaseQueueScope): QueueDownloadScopeInput | null {
  switch (scope.__typename) {
    case "EpisodeScopePayload":
      return scope.episodeId ? { episode: scope.episodeId } : null;
    case "EpisodeSetScopePayload":
      return scope.episodeIds.length > 0 ? { episodeSet: scope.episodeIds } : null;
    case "CollectionScopePayload":
      return scope.collectionId ? { collection: scope.collectionId } : null;
    case "SeriesMovieScopePayload":
      return scope.seriesMovieLinkId ? { seriesMovie: scope.seriesMovieLinkId } : null;
    case "TitleScopePayload":
      return { title: true };
    case "OrphanScopePayload":
      // No QueueDownloadScopeInput variant expresses an orphan scope; callers
      // fall back (typically to a whole-title submission).
      return null;
    default:
      return null;
  }
}

export function releaseQueueScopeInput(
  release: Pick<Release, "queueScope">,
  fallback: QueueDownloadScopeInput,
): QueueDownloadScopeInput {
  const candidateScope = release.queueScope ? queueScopeToInput(release.queueScope) : null;
  return candidateScope ?? fallback;
}

export function releaseSupportsAdditionalFileQueue(
  release: Pick<Release, "queueScope">,
  titleFacet: string | null | undefined,
): boolean {
  const scope = release.queueScope ? queueScopeToInput(release.queueScope) : null;
  if (!scope) {
    return false;
  }
  if ("episode" in scope) {
    return true;
  }
  if ("seriesMovie" in scope) {
    return true;
  }
  if ("title" in scope) {
    return titleFacet?.toUpperCase() === "MOVIE";
  }
  return false;
}

/// Whether the release's resolved scope brings down more than the one episode a
/// single-episode grab would. Read off the scope the backend already signed —
/// the same coverage that admitted the release into an episode search — rather
/// than re-reading the release name, so the badge cannot disagree with what
/// queueing it would actually do.
///
/// A one-member episode set is not a pack: it covers exactly the wanted episode.
export function releaseCoversMultipleEpisodes(
  release: Pick<Release, "queueScope">,
): boolean {
  const scope = release.queueScope;
  if (!scope) {
    return false;
  }
  switch (scope.__typename) {
    case "CollectionScopePayload":
      return Boolean(scope.collectionId);
    case "EpisodeSetScopePayload":
      return scope.episodeIds.length > 1;
    default:
      return false;
  }
}

export function hasPrimaryMediaFile(
  files: readonly { role?: string | null }[] | null | undefined,
): boolean {
  return files?.some((file) => file.role?.toLowerCase() === "primary") ?? false;
}

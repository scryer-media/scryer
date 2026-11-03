import { videoFileStem } from "./media-formats.ts";

export type CurrentReleaseFile = {
  role: string;
  grabbedReleaseTitle?: string | null;
  sceneName?: string | null;
  originalFilePath?: string | null;
  filePath: string;
  episodeId?: string | null;
  episode?: { collectionId: string | null } | null;
  seriesMovieLinkIds?: string[];
};

export type CurrentReleaseScope = {
  episodeId?: string | null;
  collectionId?: string | null;
  seriesMovieLinkId?: string | null;
};

export function releaseIdentityName(value: string): string {
  return value.trim().toLowerCase();
}

function fileStem(path: string | null | undefined): string {
  return videoFileStem(path ?? "");
}

export function currentReleaseNames(
  files: readonly CurrentReleaseFile[],
  scope: CurrentReleaseScope = {},
): Set<string> {
  const names = new Set<string>();
  for (const file of files) {
    if (!["primary", "additional"].includes(file.role.toLowerCase())) continue;
    if (scope.episodeId && file.episodeId !== scope.episodeId) continue;
    if (scope.collectionId && file.episode?.collectionId !== scope.collectionId)
      continue;
    if (
      scope.seriesMovieLinkId &&
      !file.seriesMovieLinkIds?.includes(scope.seriesMovieLinkId)
    )
      continue;
    // Prefer the persisted grab identity. A renamed library filename must
    // never make another release with the same quality appear owned, so the
    // on-disk name is consulted only when a row carries no other evidence:
    // the backend's release identity falls back to that stem the same way.
    const releaseName =
      file.grabbedReleaseTitle?.trim() || file.sceneName?.trim();
    const originalName = fileStem(file.originalFilePath);
    const name = releaseName || originalName || fileStem(file.filePath);
    if (name) names.add(releaseIdentityName(name));
  }
  return names;
}

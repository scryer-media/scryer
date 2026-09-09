import type { MediaAnalysisAttempt, MediaDiscMetadata } from "../types/media-analysis";

export function discReviewInventory(saved: MediaDiscMetadata | null, attempt?: MediaAnalysisAttempt | null) {
  return attempt && !attempt.succeeded ? attempt.disc ?? saved : saved;
}

export function discTitleIdentity(id: string, inventory: MediaDiscMetadata | null) {
  return inventory?.titles.find((title) => title.id === id || title.aliases.includes(id))?.id ?? id;
}

export function discEpisodeSelections(saved: MediaDiscMetadata | null, inventory: MediaDiscMetadata | null) {
  return Object.fromEntries((saved?.selection.episodeMappings ?? []).map((mapping) => [
    discTitleIdentity(mapping.discTitleId, inventory), mapping.episodeIds[0] ?? "",
  ]));
}

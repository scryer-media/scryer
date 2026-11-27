import { EPISODE_TARGET_PREFIX, episodeTargetIds } from "./episode-target.ts";

/** The server's estimated comparison of a file with what its episodes hold. */
export type ManualImportVerdict =
  "UPGRADE" | "EQUAL" | "DOWNGRADE" | "NO_CURRENT_FILE";

export type ManualImportVerdictRequest = {
  candidateId: string;
  episodeIds: string[];
};

/** An estimate, remembered with the mapping it was asked for. */
export type ManualImportVerdictEstimate = {
  key: string;
  verdict: ManualImportVerdict | null;
  episodeCount: number;
};

export const MANUAL_IMPORT_VERDICT_TONES: Record<
  ManualImportVerdict,
  "positive" | "neutral" | "warning" | "info"
> = {
  UPGRADE: "positive",
  EQUAL: "neutral",
  DOWNGRADE: "warning",
  NO_CURRENT_FILE: "info",
};

/** Identifies one mapping of a file, whatever order its episodes are in. */
export function manualImportVerdictKey(episodeIds: readonly string[]): string {
  return [...episodeIds].sort().join(",");
}

/** The episodes an episode target maps a file to; empty for any other target. */
export function manualImportTargetEpisodeIds(
  target: string | undefined,
): string[] {
  if (!target || !target.startsWith(EPISODE_TARGET_PREFIX)) return [];
  return episodeTargetIds(target);
}

/**
 * The episode mappings not yet asked about: a file whose mapping changed since
 * its last request, and only files mapped to episodes.
 */
export function pendingManualImportVerdictRequests(
  mappings: Readonly<Record<string, string>>,
  requested: Readonly<Record<string, string>>,
): ManualImportVerdictRequest[] {
  const pending: ManualImportVerdictRequest[] = [];
  for (const [candidateId, target] of Object.entries(mappings)) {
    const episodeIds = manualImportTargetEpisodeIds(target);
    if (episodeIds.length === 0) continue;
    if (requested[candidateId] === manualImportVerdictKey(episodeIds)) continue;
    pending.push({ candidateId, episodeIds });
  }
  return pending;
}

/** The estimate for a file's current mapping, or null when it is stale or absent. */
export function currentManualImportVerdict(
  estimate: ManualImportVerdictEstimate | undefined,
  target: string | undefined,
): ManualImportVerdictEstimate | null {
  const episodeIds = manualImportTargetEpisodeIds(target);
  if (!estimate || episodeIds.length === 0 || !estimate.verdict) return null;
  return estimate.key === manualImportVerdictKey(episodeIds) ? estimate : null;
}

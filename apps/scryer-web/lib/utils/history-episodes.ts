export type HistoryEpisodeDisplay = {
  id: string;
  seasonNumber: string | number | null;
  episodeNumber: string | number | null;
  episodeLabel: string | null;
  title: string | null;
};

function episodeNumberValue(value: string | number | null): number {
  const match = String(value ?? "").match(/\d+/);
  return match ? Number.parseInt(match[0], 10) : Number.MAX_SAFE_INTEGER;
}

export function compareHistoryEpisodes(
  left: HistoryEpisodeDisplay | null,
  right: HistoryEpisodeDisplay | null,
): number {
  if (!left || !right) {
    return left ? -1 : right ? 1 : 0;
  }

  return (
    episodeNumberValue(left.seasonNumber) - episodeNumberValue(right.seasonNumber) ||
    episodeNumberValue(left.episodeNumber) - episodeNumberValue(right.episodeNumber) ||
    left.id.localeCompare(right.id)
  );
}

export function formatHistoryEpisodeLabel(
  episode: HistoryEpisodeDisplay | null,
  episodeId: string,
): string {
  if (!episode) {
    return episodeId;
  }

  const seasonNumber = episodeNumberValue(episode.seasonNumber);
  const episodeNumber = episodeNumberValue(episode.episodeNumber);
  const numberedLabel =
    seasonNumber !== Number.MAX_SAFE_INTEGER && episodeNumber !== Number.MAX_SAFE_INTEGER
      ? `S${String(seasonNumber).padStart(2, "0")}E${String(episodeNumber).padStart(2, "0")}`
      : null;
  const label = numberedLabel || episode.episodeLabel?.trim() || "Episode";
  const title = episode.title?.trim();

  return title && title !== label ? `${label} · ${title}` : label;
}

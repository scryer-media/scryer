/**
 * Series and season progress: episodes on disk over episodes expected on
 * disk. An episode is expected once it has a file, or once it is monitored
 * and has aired. With nothing expected the progress is complete.
 */
export type EpisodeProgressCounts = {
  onDisk: number;
  expected: number;
  /** Every episode record, specials and placeholders included. */
  total: number | null;
};

function count(value: number | null | undefined) {
  return typeof value === "number" && Number.isFinite(value)
    ? Math.max(0, value)
    : null;
}

/** The progress counts the server reported, or null when it reported none. */
export function episodeProgressCounts({
  episodesOnDisk,
  episodesExpected,
  episodeRecordsTotal,
}: {
  episodesOnDisk?: number | null;
  episodesExpected?: number | null;
  episodeRecordsTotal?: number | null;
}): EpisodeProgressCounts | null {
  const expected = count(episodesExpected);
  if (expected == null) {
    return null;
  }
  return {
    onDisk: count(episodesOnDisk) ?? 0,
    expected,
    total: count(episodeRecordsTotal),
  };
}

export function episodeProgressPercent({
  onDisk,
  expected,
}: EpisodeProgressCounts) {
  return expected > 0 ? Math.min(100, (onDisk / expected) * 100) : 100;
}

export function episodeProgressText({ onDisk, expected }: EpisodeProgressCounts) {
  return `${onDisk} / ${expected}`;
}

/** A local calendar day as `YYYY-MM-DD`, the form air dates are compared in. */
export function localCalendarDay(date: Date) {
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${date.getFullYear()}-${month}-${day}`;
}

/**
 * Counts loaded episodes the way the server does, for a season whose counts
 * were not part of the response. `today` is a `YYYY-MM-DD` day; an episode
 * has aired when its air date's day is on or before it.
 */
export function countEpisodeProgress(
  episodes: readonly {
    monitored?: boolean | null;
    airDate?: string | null;
    hasFile: boolean;
  }[],
  today: string,
): EpisodeProgressCounts {
  let onDisk = 0;
  let expected = 0;
  for (const episode of episodes) {
    const airDay = episode.airDate?.trim().slice(0, 10) ?? "";
    const aired = airDay !== "" && airDay <= today;
    if (episode.hasFile) {
      onDisk += 1;
    }
    if (episode.hasFile || (episode.monitored === true && aired)) {
      expected += 1;
    }
  }
  return { onDisk, expected, total: episodes.length };
}

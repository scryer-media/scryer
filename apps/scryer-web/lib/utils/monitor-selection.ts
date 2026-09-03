import type {
  MonitorSelectionDraft,
  MonitorSelectionMovieDraft,
} from "@/lib/types/titles";

export type { MonitorSelectionDraft, MonitorSelectionMovieDraft };

/**
 * Mirrors `MonitorSelectionMovie::canonical_key` in `scryer-domain`: the first
 * id that exists wins, so both halves agree on a movie's identity.
 */
const MOVIE_KEY_SOURCES = ["tvdb", "tmdb", "imdb", "anidb", "mal"] as const;

export const EMPTY_MONITOR_SELECTION: MonitorSelectionDraft = {
  seasonNumbers: [],
  seriesMovies: [],
};

export function monitorSelectionMovieKey(
  movie: MonitorSelectionMovieDraft,
): string | null {
  for (const source of MOVIE_KEY_SOURCES) {
    const match = movie.externalIds.find(
      (externalId) =>
        externalId.source.trim().toLowerCase() === source &&
        externalId.value.trim(),
    );
    if (match) {
      return `${source}:${match.value.trim()}`;
    }
  }
  return null;
}

export function normalizeMonitorSelection(
  selection: MonitorSelectionDraft,
): MonitorSelectionDraft {
  const seasonNumbers = [...new Set(selection.seasonNumbers)].sort(
    (left, right) => left - right,
  );
  const seriesMoviesByKey = new Map<string, MonitorSelectionMovieDraft>();
  for (const movie of selection.seriesMovies) {
    const key = monitorSelectionMovieKey(movie);
    if (!key) {
      continue;
    }
    seriesMoviesByKey.set(key, movie);
  }
  const seriesMovies = [...seriesMoviesByKey.entries()]
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([, movie]) => movie);
  return { seasonNumbers, seriesMovies };
}

export function isMonitorSelectionEmpty(
  selection: MonitorSelectionDraft | null | undefined,
): boolean {
  if (!selection) {
    return true;
  }
  const normalized = normalizeMonitorSelection(selection);
  return (
    normalized.seasonNumbers.length === 0 && normalized.seriesMovies.length === 0
  );
}

/**
 * The GraphQL input shape is the draft shape, so this only normalizes and drops
 * empty selections — the API rejects those, and callers use `undefined` to mean
 * "not advanced, send nothing".
 */
export function monitorSelectionInput(
  selection: MonitorSelectionDraft | null | undefined,
): MonitorSelectionDraft | undefined {
  if (isMonitorSelectionEmpty(selection)) {
    return undefined;
  }
  return normalizeMonitorSelection(selection as MonitorSelectionDraft);
}

export function monitorSelectionFromRecord(
  record: MonitorSelectionDraft | null | undefined,
): MonitorSelectionDraft | null {
  if (!record) {
    return null;
  }
  const normalized = normalizeMonitorSelection({
    seasonNumbers: record.seasonNumbers ?? [],
    seriesMovies: (record.seriesMovies ?? []).map((movie) => ({
      name: movie.name,
      externalIds: movie.externalIds ?? [],
    })),
  });
  return isMonitorSelectionEmpty(normalized) ? null : normalized;
}

export type MonitorSelectionSummaryLabels = {
  specials: string;
  season: (seasonNumber: number) => string;
};

/**
 * Card summary for the approver: `Season 1, Season 3, Specials · Movies: …`.
 */
export function monitorSelectionSummaryParts(
  selection: MonitorSelectionDraft | null | undefined,
  labels: MonitorSelectionSummaryLabels,
): { seasons: string[]; movies: string[] } {
  const normalized = monitorSelectionFromRecord(selection);
  if (!normalized) {
    return { seasons: [], movies: [] };
  }
  return {
    seasons: normalized.seasonNumbers.map((seasonNumber) =>
      seasonNumber === 0 ? labels.specials : labels.season(seasonNumber),
    ),
    movies: normalized.seriesMovies.map((movie) => movie.name.trim()).filter(Boolean),
  };
}

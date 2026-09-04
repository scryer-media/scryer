import * as React from "react";
import { AlertTriangle, Loader2 } from "lucide-react";
import { useClient } from "urql";

import { CheckboxField } from "@/components/ui/checkbox";
import { useTranslate } from "@/lib/context/translate-context";
import { metadataSeriesQuery } from "@/lib/graphql/queries";
import { getGraphqlLanguage } from "@/lib/graphql/urql-client";
import type { ExternalId, Facet } from "@/lib/types/titles";
import {
  monitorSelectionMovieKey,
  normalizeMonitorSelection,
  type MonitorSelectionDraft,
  type MonitorSelectionMovieDraft,
} from "@/lib/utils/monitor-selection";
import { selectorId } from "@/lib/utils/dom-ids";
import { cn } from "@/lib/utils";

type SeasonChoice = {
  number: number;
  label: string;
};

type SeriesMovieChoice = {
  key: string;
  name: string;
  year: number | null;
  canon: boolean;
  externalIds: ExternalId[];
};

type SeriesMonitorChoices = {
  seasons: SeasonChoice[];
  seriesMovies: SeriesMovieChoice[];
};

type MetadataSeasonNode = {
  number?: number | null;
  label?: string | null;
};

type MetadataAnimeMovieNode = {
  name?: string | null;
  year?: number | null;
  associationConfidence?: string | null;
  continuityStatus?: string | null;
  externalIds?: ExternalId[] | null;
};

/**
 * Mirrors the backend's association filter: low-confidence associations are not
 * offered, because they are not linked to the series either.
 */
const OFFERED_ASSOCIATION_CONFIDENCE = new Set(["medium", "high"]);

/**
 * Season and movie metadata is only ever fetched because the user picked
 * ADVANCED, and the picker unmounts as soon as they pick something else. This
 * cache keeps that one fetch alive across such toggles (and across dialogs in
 * the same page session) so flipping the select never re-queries SMG.
 */
const CHOICES_CACHE_LIMIT = 20;
const choicesCache = new Map<string, Promise<SeriesMonitorChoices>>();

function cacheChoices(
  key: string,
  load: () => Promise<SeriesMonitorChoices>,
): Promise<SeriesMonitorChoices> {
  const cached = choicesCache.get(key);
  if (cached) {
    return cached;
  }
  const pending = load().catch((error: unknown) => {
    // A failed lookup must stay retryable.
    choicesCache.delete(key);
    throw error;
  });
  choicesCache.set(key, pending);
  while (choicesCache.size > CHOICES_CACHE_LIMIT) {
    const oldest = choicesCache.keys().next();
    if (oldest.done) {
      break;
    }
    choicesCache.delete(oldest.value);
  }
  return pending;
}

function compareSeasonNumbers(
  left: { number: number },
  right: { number: number },
): number {
  if (left.number === 0 || right.number === 0) {
    return left.number === right.number ? 0 : left.number === 0 ? 1 : -1;
  }
  return left.number - right.number;
}

function toSeasonChoices(nodes: MetadataSeasonNode[]): SeasonChoice[] {
  const byNumber = new Map<number, SeasonChoice>();
  for (const node of nodes) {
    const number = node.number;
    if (typeof number !== "number" || !Number.isFinite(number) || number < 0) {
      continue;
    }
    if (byNumber.has(number)) {
      continue;
    }
    byNumber.set(number, { number, label: node.label?.trim() ?? "" });
  }
  // Specials (season 0) read best after the numbered seasons.
  return [...byNumber.values()].sort(compareSeasonNumbers);
}

function toSeriesMovieChoices(
  nodes: MetadataAnimeMovieNode[],
): SeriesMovieChoice[] {
  const byKey = new Map<string, SeriesMovieChoice>();
  for (const node of nodes) {
    const confidence = node.associationConfidence?.trim().toLowerCase() ?? "";
    if (!OFFERED_ASSOCIATION_CONFIDENCE.has(confidence)) {
      continue;
    }
    const externalIds = (node.externalIds ?? []).filter(
      (externalId) => externalId.source?.trim() && externalId.value?.trim(),
    );
    const name = node.name?.trim() ?? "";
    const key = monitorSelectionMovieKey({ name, externalIds });
    if (!key || !name || byKey.has(key)) {
      continue;
    }
    byKey.set(key, {
      key,
      name,
      year: node.year ?? null,
      canon: node.continuityStatus?.trim().toLowerCase() === "canon",
      externalIds,
    });
  }
  return [...byKey.values()].sort((left, right) => left.key.localeCompare(right.key));
}

function defaultSelection(choices: SeriesMonitorChoices): MonitorSelectionDraft {
  return normalizeMonitorSelection({
    // Specials stay off by default, matching the non-advanced default.
    seasonNumbers: choices.seasons
      .filter((season) => season.number !== 0)
      .map((season) => season.number),
    seriesMovies: choices.seriesMovies
      .filter((movie) => movie.canon)
      .map((movie) => ({ name: movie.name, externalIds: movie.externalIds })),
  });
}

type MonitorSelectionPickerProps = {
  facet: Facet;
  tvdbId: string;
  value: MonitorSelectionDraft;
  onChange: (value: MonitorSelectionDraft) => void;
  onLoadingChange?: (loading: boolean) => void;
  disabled?: boolean;
  idPrefix: string;
  className?: string;
};

export function MonitorSelectionPicker({
  facet,
  tvdbId,
  value,
  onChange,
  onLoadingChange,
  disabled = false,
  idPrefix,
  className,
}: MonitorSelectionPickerProps) {
  const t = useTranslate();
  const client = useClient();
  const normalizedTvdbId = tvdbId.trim();
  const [choices, setChoices] = React.useState<SeriesMonitorChoices | null>(null);
  const [loading, setLoading] = React.useState(false);
  const [loadError, setLoadError] = React.useState<string | null>(null);
  const [retryCount, setRetryCount] = React.useState(0);
  // `onChange` identity changes with every parent render; keeping the callbacks
  // and the current draft in refs stops the load effect from re-running (and
  // re-seeding the defaults) on every keystroke elsewhere in the dialog.
  const onChangeRef = React.useRef(onChange);
  const onLoadingChangeRef = React.useRef(onLoadingChange);
  const seededSelectionRef = React.useRef<string | null>(null);
  const hasSelection =
    value.seasonNumbers.length > 0 || value.seriesMovies.length > 0;
  const hasSelectionRef = React.useRef(hasSelection);

  // Declared before the load effect so the refs are current whenever it re-runs.
  React.useEffect(() => {
    onChangeRef.current = onChange;
    onLoadingChangeRef.current = onLoadingChange;
    hasSelectionRef.current = hasSelection;
  });

  React.useEffect(() => {
    if (!normalizedTvdbId) {
      setChoices(null);
      setLoadError(null);
      setLoading(false);
      onLoadingChangeRef.current?.(false);
      return;
    }

    let active = true;
    const language = getGraphqlLanguage();
    const cacheKey = `${normalizedTvdbId}|${language}`;
    setLoading(true);
    setLoadError(null);
    onLoadingChangeRef.current?.(true);

    const pending = cacheChoices(cacheKey, async () => {
      const { data, error } = await client
        .query(metadataSeriesQuery, {
          input: {
            tvdbId: normalizedTvdbId,
            includeEpisodes: false,
            language,
          },
        })
        .toPromise();
      if (error) {
        throw error;
      }
      const series = data?.metadataSeries;
      if (!series) {
        throw new Error("metadataSeries returned no series");
      }
      return {
        seasons: toSeasonChoices(series.seasons ?? []),
        seriesMovies: toSeriesMovieChoices(series.animeMovies ?? []),
      };
    });

    void pending
      .then((loaded) => {
        if (!active) return;
        setChoices(loaded);
        // Only seed defaults when the caller opened with nothing picked; a
        // prefilled draft (an existing request being approved or edited) is the
        // user's own selection and must survive.
        if (!hasSelectionRef.current && seededSelectionRef.current !== cacheKey) {
          seededSelectionRef.current = cacheKey;
          onChangeRef.current(defaultSelection(loaded));
        }
      })
      .catch((error: unknown) => {
        if (!active) return;
        setChoices(null);
        setLoadError(
          error instanceof Error ? error.message : t("status.apiError"),
        );
      })
      .finally(() => {
        if (!active) return;
        setLoading(false);
        onLoadingChangeRef.current?.(false);
      });

    return () => {
      active = false;
    };
  }, [client, normalizedTvdbId, retryCount, t]);

  const selectedSeasons = React.useMemo(
    () => new Set(value.seasonNumbers),
    [value.seasonNumbers],
  );
  const selectedMovieKeys = React.useMemo(
    () =>
      new Set(
        value.seriesMovies
          .map((movie) => monitorSelectionMovieKey(movie))
          .filter((key): key is string => key !== null),
      ),
    [value.seriesMovies],
  );

  const emit = React.useCallback(
    (next: MonitorSelectionDraft) => {
      onChangeRef.current(normalizeMonitorSelection(next));
    },
    [],
  );

  const toggleSeason = (seasonNumber: number, checked: boolean) => {
    emit({
      ...value,
      seasonNumbers: checked
        ? [...value.seasonNumbers, seasonNumber]
        : value.seasonNumbers.filter((number) => number !== seasonNumber),
    });
  };

  const toggleMovie = (movie: SeriesMovieChoice, checked: boolean) => {
    const withoutMovie = value.seriesMovies.filter(
      (candidate) => monitorSelectionMovieKey(candidate) !== movie.key,
    );
    const nextMovie: MonitorSelectionMovieDraft = {
      name: movie.name,
      externalIds: movie.externalIds,
    };
    emit({
      ...value,
      seriesMovies: checked ? [...withoutMovie, nextMovie] : withoutMovie,
    });
  };

  const seasonLabel = (season: SeasonChoice): string => {
    if (season.number === 0) {
      return t("monitorSelection.specials");
    }
    return season.label
      ? t("monitorSelection.seasonNamed", {
          number: season.number,
          name: season.label,
        })
      : t("monitorSelection.season", { number: season.number });
  };

  const movieLabel = (movie: SeriesMovieChoice): string =>
    movie.year ? `${movie.name} (${movie.year})` : movie.name;

  if (!normalizedTvdbId) {
    return (
      <div
        id={`${idPrefix}-monitor-selection-unavailable`}
        className={cn(
          "flex items-start gap-2 rounded-[10px] border border-[var(--scry-border3)] bg-[var(--scry-inset)] px-3 py-2 text-sm text-[var(--scry-muted)]",
          className,
        )}
      >
        <AlertTriangle className="mt-0.5 h-4 w-4 flex-none text-[var(--scry-warning-text)]" />
        <span>{t("monitorSelection.unavailable")}</span>
      </div>
    );
  }

  return (
    <div
      id={`${idPrefix}-monitor-selection`}
      className={cn(
        "space-y-4 rounded-[10px] border border-[var(--scry-border3)] bg-[var(--scry-inset)] p-3",
        className,
      )}
    >
      {loading ? (
        <div
          id={`${idPrefix}-monitor-selection-loading`}
          className="flex items-center gap-2 text-sm text-[var(--scry-muted)]"
        >
          <Loader2 className="h-4 w-4 animate-spin text-[var(--scry-accent)]" />
          <span>{t("monitorSelection.loading")}</span>
        </div>
      ) : null}

      {!loading && loadError ? (
        <div
          id={`${idPrefix}-monitor-selection-error`}
          className="flex flex-wrap items-center gap-2 text-sm text-[var(--scry-danger)]"
        >
          <AlertTriangle className="h-4 w-4 flex-none" />
          <span className="min-w-0 flex-1">{t("monitorSelection.loadFailed")}</span>
          <button
            type="button"
            id={`${idPrefix}-monitor-selection-retry`}
            className="text-xs font-medium text-[var(--scry-accent-text)] underline underline-offset-2"
            onClick={() => setRetryCount((count) => count + 1)}
          >
            {t("monitorSelection.retry")}
          </button>
        </div>
      ) : null}

      {!loading && !loadError && choices ? (
        <>
          <section className="space-y-2">
            <header className="flex items-center justify-between gap-3">
              <span className="text-xs font-semibold uppercase tracking-wide text-[var(--scry-faint)]">
                {t("monitorSelection.seasons")}
              </span>
              {choices.seasons.length > 0 ? (
                <span className="flex items-center gap-3">
                  <button
                    type="button"
                    id={`${idPrefix}-monitor-selection-seasons-all`}
                    className="text-xs font-medium text-[var(--scry-accent-text)] underline underline-offset-2 disabled:opacity-50"
                    disabled={disabled}
                    onClick={() =>
                      emit({
                        ...value,
                        seasonNumbers: choices.seasons.map((season) => season.number),
                      })
                    }
                  >
                    {t("monitorSelection.selectAll")}
                  </button>
                  <button
                    type="button"
                    id={`${idPrefix}-monitor-selection-seasons-clear`}
                    className="text-xs font-medium text-[var(--scry-accent-text)] underline underline-offset-2 disabled:opacity-50"
                    disabled={disabled}
                    onClick={() => emit({ ...value, seasonNumbers: [] })}
                  >
                    {t("monitorSelection.clear")}
                  </button>
                </span>
              ) : null}
            </header>
            {choices.seasons.length === 0 ? (
              <p className="text-sm text-[var(--scry-muted)]">
                {t("monitorSelection.noSeasons")}
              </p>
            ) : (
              // CSS columns fill top-to-bottom, so seasons read 1, 2, 3 down the
              // first column rather than across the row.
              <div className="-mb-2 gap-x-2 sm:columns-2 lg:columns-3">
                {choices.seasons.map((season) => (
                  <CheckboxField
                    key={season.number}
                    id={selectorId(idPrefix, "season", String(season.number))}
                    size="compact"
                    className="mb-2 break-inside-avoid"
                    label={seasonLabel(season)}
                    checked={selectedSeasons.has(season.number)}
                    disabled={disabled}
                    onCheckedChange={(checked) =>
                      toggleSeason(season.number, checked === true)
                    }
                  />
                ))}
              </div>
            )}
          </section>

          {facet === "ANIME" && choices.seriesMovies.length > 0 ? (
            <section className="space-y-2">
              <header className="flex items-center justify-between gap-3">
                <span className="text-xs font-semibold uppercase tracking-wide text-[var(--scry-faint)]">
                  {t("monitorSelection.seriesMovies")}
                </span>
                <span className="flex items-center gap-3">
                  <button
                    type="button"
                    id={`${idPrefix}-monitor-selection-movies-all`}
                    className="text-xs font-medium text-[var(--scry-accent-text)] underline underline-offset-2 disabled:opacity-50"
                    disabled={disabled}
                    onClick={() =>
                      emit({
                        ...value,
                        seriesMovies: choices.seriesMovies.map((movie) => ({
                          name: movie.name,
                          externalIds: movie.externalIds,
                        })),
                      })
                    }
                  >
                    {t("monitorSelection.selectAll")}
                  </button>
                  <button
                    type="button"
                    id={`${idPrefix}-monitor-selection-movies-clear`}
                    className="text-xs font-medium text-[var(--scry-accent-text)] underline underline-offset-2 disabled:opacity-50"
                    disabled={disabled}
                    onClick={() => emit({ ...value, seriesMovies: [] })}
                  >
                    {t("monitorSelection.clear")}
                  </button>
                </span>
              </header>
              <div className="-mb-2 gap-x-2 sm:columns-2">
                {choices.seriesMovies.map((movie) => (
                  <CheckboxField
                    key={movie.key}
                    id={selectorId(idPrefix, "movie", movie.key)}
                    size="compact"
                    className="mb-2 break-inside-avoid"
                    label={movieLabel(movie)}
                    checked={selectedMovieKeys.has(movie.key)}
                    disabled={disabled}
                    onCheckedChange={(checked) => toggleMovie(movie, checked === true)}
                  />
                ))}
              </div>
            </section>
          ) : null}

          {!hasSelection ? (
            <p
              id={`${idPrefix}-monitor-selection-empty`}
              className="text-sm text-[var(--scry-warning-text)]"
            >
              {t("monitorSelection.emptyHint")}
            </p>
          ) : null}
        </>
      ) : null}
    </div>
  );
}

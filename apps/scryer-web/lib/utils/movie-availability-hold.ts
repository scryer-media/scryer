import type { MovieAvailabilityRecord } from "../types/titles.ts";

/**
 * What a movie's overview says while automatic search waits for the movie's
 * availability: the state, the effective date, and the reason.
 */
export type MovieAvailabilityHold = {
  status: MovieAvailabilityRecord["status"];
  statusLabel: string;
  /** The effective date, with the Calendar's estimate label when estimated. */
  dateLabel: string | null;
  /** Null for a reason code this client does not know. */
  reasonLabel: string | null;
};

/** The reason codes this client can explain, each with its i18n key. */
const MOVIE_AVAILABILITY_REASON_KEYS: Record<string, string> = {
  release_dates_unknown:
    "settings.movieAvailability.reason.release_dates_unknown",
  release_market_refresh_pending:
    "settings.movieAvailability.reason.release_market_refresh_pending",
  theatrical_release_date_unknown:
    "settings.movieAvailability.reason.theatrical_release_date_unknown",
  home_release_date_unknown:
    "settings.movieAvailability.reason.home_release_date_unknown",
  home_release_date_unusable:
    "settings.movieAvailability.reason.home_release_date_unusable",
  home_release_date_estimated_worldwide:
    "settings.movieAvailability.reason.home_release_date_estimated_worldwide",
  home_release_date_estimated_from_worldwide_theatrical:
    "settings.movieAvailability.reason.home_release_date_estimated_from_worldwide_theatrical",
  theatrical_release_date_estimated_worldwide:
    "settings.movieAvailability.reason.theatrical_release_date_estimated_worldwide",
  theatrical_release_date_estimated_from_worldwide_home:
    "settings.movieAvailability.reason.theatrical_release_date_estimated_from_worldwide_home",
  waiting_for_release_date:
    "settings.movieAvailability.reason.waiting_for_release_date",
  effective_release_date_out_of_range:
    "settings.movieAvailability.reason.effective_release_date_out_of_range",
};

/**
 * Returns null once the movie is available, when its availability is not
 * known to this view, or when automatic search is not waiting on it: an
 * unmonitored movie is not searched, and one with a file is not held.
 */
export function movieAvailabilityHold(
  availability: MovieAvailabilityRecord | null | undefined,
  translate: (key: string) => string,
  movie: { monitored: boolean; hasFile: boolean },
): MovieAvailabilityHold | null {
  if (
    !availability ||
    availability.status === "available" ||
    !movie.monitored ||
    movie.hasFile
  ) {
    return null;
  }
  const reasonKey = Object.hasOwn(
    MOVIE_AVAILABILITY_REASON_KEYS,
    availability.reason,
  )
    ? MOVIE_AVAILABILITY_REASON_KEYS[availability.reason]
    : null;
  const date = formatEffectiveDate(
    availability.effectiveDate,
    translate("calendar.locale"),
  );
  return {
    status: availability.status,
    statusLabel: translate(
      `settings.movieAvailabilityStatus.${availability.status}`,
    ),
    dateLabel: date
      ? availability.estimated
        ? `${date} (${translate("settings.movieAvailability.estimated")})`
        : date
      : null,
    reasonLabel: reasonKey ? translate(reasonKey) : null,
  };
}

function formatEffectiveDate(
  effectiveDate: string | null,
  locale: string,
): string | null {
  if (!effectiveDate) return null;
  const [year, month, day] = effectiveDate.slice(0, 10).split("-").map(Number);
  if (!year || !month || !day) return effectiveDate;
  return new Intl.DateTimeFormat(locale, {
    month: "short",
    day: "numeric",
    year: "numeric",
  }).format(new Date(year, month - 1, day));
}

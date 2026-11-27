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
  reasonLabel: string;
};

/**
 * Returns null once the movie is available, or when its availability is not
 * known to this view; automatic search only waits on the other states.
 */
export function movieAvailabilityHold(
  availability: MovieAvailabilityRecord | null | undefined,
  translate: (key: string) => string,
): MovieAvailabilityHold | null {
  if (!availability || availability.status === "available") {
    return null;
  }
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
    reasonLabel: translate(
      `settings.movieAvailability.reason.${availability.reason}`,
    ),
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

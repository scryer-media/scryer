import { Clock } from "lucide-react";
import { useTranslate } from "@/lib/context/translate-context";
import { cn } from "@/lib/utils";
import type { Release } from "@/lib/types";

const MOVIE_AVAILABILITY_REASON_CODE_PATTERN = /: ([a-z_]+)(?: \(|$)/;
const MOVIE_AVAILABILITY_DATE_PATTERN = /\((\d{4}-\d{2}-\d{2})\)/;

type AvailabilityResult = Pick<
  Release,
  "autoDecisionCode" | "autoDecisionSummary"
>;

export function isMovieAvailabilityBlocked(
  result: AvailabilityResult,
): boolean {
  return result.autoDecisionCode === "movie_availability_blocked";
}

/**
 * Automatic search holds this release until the movie reaches its configured
 * availability. A manual queue is never refused for it; the badge only says
 * why automation is waiting. Nothing renders for any other decision.
 */
export function MovieAvailabilityBadge({
  result,
  className,
}: {
  result: AvailabilityResult;
  className?: string;
}) {
  const t = useTranslate();
  if (!isMovieAvailabilityBlocked(result)) {
    return null;
  }
  const summary = result.autoDecisionSummary ?? "";
  const reasonCode = summary.match(MOVIE_AVAILABILITY_REASON_CODE_PATTERN)?.[1];
  const date = summary.match(MOVIE_AVAILABILITY_DATE_PATTERN)?.[1];
  const reason = reasonCode
    ? `${t(`settings.movieAvailability.reason.${reasonCode}`)}${date ? ` (${date})` : ""}`
    : summary;
  return (
    <span
      data-ui="movie-availability-badge"
      data-reason={reasonCode}
      className={cn(
        "inline-flex max-w-full items-center gap-1 rounded-[6px] bg-[var(--scry-warning-bg-strong)] px-2 py-0.5 text-[10px] font-bold text-[var(--scry-warning-text)]",
        className,
      )}
    >
      <Clock className="h-2.5 w-2.5 shrink-0" />
      <span className="min-w-0 break-words">
        {t("nzb.movieAvailabilityWarning", { reason })}
      </span>
    </span>
  );
}

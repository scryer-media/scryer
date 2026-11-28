import * as React from "react";
import { Copy } from "lucide-react";

import { cn } from "@/lib/utils";
import type { SameFilmPlace } from "@/lib/utils/same-film";

/** Stands in for the linked names inside a translated sentence. */
export const SAME_FILM_TITLES_SLOT = "\u0000";

/**
 * Says that the film is tracked in another place too, linking to each.
 * Renders nothing when there is no other place.
 */
export function SameFilmNotice({
  places,
  sentence,
  className,
}: {
  places: readonly SameFilmPlace[];
  /** Translated sentence with `SAME_FILM_TITLES_SLOT` where the names go. */
  sentence: string;
  className?: string;
}) {
  if (places.length === 0) return null;
  const [before, after = ""] = sentence.split(SAME_FILM_TITLES_SLOT);
  return (
    <p
      data-ui="same-film-notice"
      className={cn(
        "flex min-w-0 items-start gap-1.5 text-[11.5px] leading-4 text-[var(--scry-muted2)]",
        className,
      )}
    >
      <Copy className="mt-px h-3.5 w-3.5 shrink-0" aria-hidden="true" />
      <span className="min-w-0 break-words">
        {before}
        {places.map((place, index) => (
          <React.Fragment key={place.key}>
            {index > 0 ? ", " : null}
            {place.href ? (
              <a
                href={place.href}
                className="font-semibold text-[var(--scry-ink2)] hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--scry-focus)]"
              >
                {place.name}
              </a>
            ) : (
              <span className="font-semibold">{place.name}</span>
            )}
          </React.Fragment>
        ))}
        {after}
      </span>
    </p>
  );
}

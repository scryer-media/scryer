import type * as React from "react";
import { CalendarDays, Languages, Tags } from "lucide-react";

import { TitlePoster } from "@/components/title-poster";
import { TitleRatingsDisplay } from "@/components/common/title-ratings-display";
import { formatRuntimeFromMinutes } from "@/components/views/series-overview/helpers";
import { formatLanguage } from "@/lib/utils/media-info-format";
import type { TitleExternalRating } from "@/lib/utils/title-ratings";

/** The popover classes every list title flyout shares; the body below fills it edge to edge. */
export const LIST_TITLE_FLYOUT_CLASS =
  "z-[90] w-[400px] max-w-[calc(100vw-24px)] max-h-[var(--radix-popover-content-available-height)] overflow-y-auto overflow-x-hidden rounded-xl border-[var(--scry-border3)] p-0 text-[var(--scry-ink)] shadow-xl";

type ListTitleFlyoutBodyProps = {
  title: string;
  /** The kind's icon and name, when the kind is known. */
  kindIcon?: React.ReactNode;
  kindLabel?: string | null;
  year?: number | null;
  runtimeMinutes?: number | null;
  posterUrl?: string | null;
  /** The backdrop behind the poster; the poster itself stands in when there is none. */
  backgroundUrl?: string | null;
  externalRatings?: TitleExternalRating[];
  overview?: string | null;
  genres?: string[];
  originalLanguage?: string | null;
  releaseDate?: string | null;
  /** A closing line, such as where the title stands on its list. */
  footer?: React.ReactNode;
};

/**
 * What a list knows of one title: its poster over its backdrop, then ratings,
 * description and genres. Anything unknown is left out.
 */
export function ListTitleFlyoutBody({
  title,
  kindIcon,
  kindLabel,
  year,
  runtimeMinutes,
  posterUrl,
  backgroundUrl,
  externalRatings,
  overview,
  genres,
  originalLanguage,
  releaseDate,
  footer,
}: ListTitleFlyoutBodyProps) {
  const art = backgroundUrl ?? posterUrl ?? null;
  const facts = [kindLabel, year, formatRuntimeFromMinutes(runtimeMinutes)]
    .filter(Boolean)
    .join(" · ");

  return (
    <div data-ui="list-title-flyout">
      <div className="relative overflow-hidden">
        {art ? (
          <img
            src={art}
            alt=""
            aria-hidden="true"
            data-ui="list-title-flyout-art"
            // A poster standing in for the backdrop is softened so it reads as a backdrop.
            className={`absolute inset-0 h-full w-full object-cover${backgroundUrl ? "" : " scale-125 blur-xl"}`}
          />
        ) : null}
        <div
          aria-hidden="true"
          className="absolute inset-0 bg-gradient-to-t from-popover via-popover/80 to-popover/25"
        />
        <div className="relative flex items-end gap-3 px-4 pb-3 pt-4">
          {posterUrl ? (
            <div className="w-[84px] flex-none overflow-hidden rounded-[8px] border border-[var(--scry-border3)] bg-[var(--scry-inset)] shadow-lg">
              <TitlePoster
                src={posterUrl}
                alt=""
                className="aspect-[2/3] h-full w-full object-cover"
              />
            </div>
          ) : null}
          <div className="min-w-0 space-y-1.5">
            {facts ? (
              <p className="flex items-center gap-1.5 text-xs text-[var(--scry-ink2)]">
                {kindIcon}
                {facts}
              </p>
            ) : null}
            <h3 className="font-display text-base font-semibold leading-snug">
              {title}
            </h3>
            <TitleRatingsDisplay externalRatings={externalRatings} />
          </div>
        </div>
      </div>
      <div className="space-y-2.5 px-4 pb-4 pt-1">
        {overview ? (
          <p
            data-ui="list-title-flyout-overview"
            className="line-clamp-6 text-sm leading-relaxed text-[var(--scry-ink2)]"
          >
            {overview}
          </p>
        ) : null}
        {genres?.length ? (
          <p className="flex gap-2 text-sm text-[var(--scry-ink2)]">
            <Tags aria-hidden="true" className="mt-0.5 h-4 w-4 shrink-0" />
            {genres.join(" · ")}
          </p>
        ) : null}
        {originalLanguage ? (
          <p className="flex gap-2 text-sm text-[var(--scry-muted)]">
            <Languages aria-hidden="true" className="h-4 w-4 shrink-0" />
            {formatLanguage(originalLanguage)}
          </p>
        ) : null}
        {releaseDate ? (
          <p className="flex gap-2 text-sm text-[var(--scry-muted)]">
            <CalendarDays aria-hidden="true" className="h-4 w-4 shrink-0" />
            <time dateTime={releaseDate}>{releaseDate}</time>
          </p>
        ) : null}
        {footer ? (
          <div className="text-xs text-[var(--scry-muted)]">{footer}</div>
        ) : null}
      </div>
    </div>
  );
}

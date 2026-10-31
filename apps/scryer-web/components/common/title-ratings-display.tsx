import type { ReactNode } from "react";

import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import {
  compactRatingNumber,
  normalizedRatingSource,
  ratingSourceInfo,
  ratingValueLabel,
  topOrderedRatingSource,
  visibleTitleExternalRatings,
  type RatingSourceInfo,
  type TitleExternalRating,
} from "@/lib/utils/title-ratings";
import { cn } from "@/lib/utils";

export type TitleRatingsDisplayVariant = "default" | "hero";

type TitleRatingsDisplayProps = {
  externalRatings?: TitleExternalRating[];
  fallbackRating?: number | null;
  fallbackSources?: readonly string[];
  variant?: TitleRatingsDisplayVariant;
  className?: string;
};

const ROTTEN_TOMATOES_CRITIC_SOURCE_IDS = new Set([
  "rottentomatoes",
  "tomatoes",
]);
const ROTTEN_TOMATOES_AUDIENCE_SOURCE_IDS = new Set([
  "audience",
  "popcorn",
  "popcornmeter",
]);
const METACRITIC_CRITIC_SOURCE_IDS = new Set(["metacritic"]);
const METACRITIC_USER_SOURCE_IDS = new Set(["mcuser", "metacriticuser"]);

type RatingPillEntry =
  | {
      key: string;
      kind: "single";
      rating: TitleExternalRating;
    }
  | {
      key: string;
      kind: "rotten-tomatoes" | "metacritic";
      ratings: TitleExternalRating[];
    };

function RatingSourceLogo({
  source,
  variant,
}: {
  source: RatingSourceInfo;
  variant: TitleRatingsDisplayVariant;
}) {
  if (!source.logoSrc) {
    return null;
  }

  return (
    <img
      src={source.logoSrc}
      alt=""
      aria-hidden="true"
      className={cn(
        "shrink-0 object-contain",
        variant === "hero" ? "h-4 w-4" : "h-3.5 w-3.5",
      )}
      loading="lazy"
    />
  );
}

function RatingPill({
  ariaLabel,
  tooltipLabel,
  variant,
  children,
}: {
  ariaLabel: string;
  tooltipLabel: string;
  variant: TitleRatingsDisplayVariant;
  children: ReactNode;
}) {
  const className = cn(
    "inline-flex items-center gap-1.5 rounded border",
    variant === "hero"
      ? "border-white/15 bg-black/35 px-2.5 py-1.5 text-sm shadow-[inset_0_1px_0_rgba(255,255,255,0.08)] backdrop-blur-sm"
      : "border-border/70 bg-background/45 px-2 py-1 text-xs",
  );

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span aria-label={ariaLabel} className={className}>
          {children}
        </span>
      </TooltipTrigger>
      <TooltipContent>{tooltipLabel}</TooltipContent>
    </Tooltip>
  );
}

function RatingValue({
  value,
  variant,
}: {
  value: string;
  variant: TitleRatingsDisplayVariant;
}) {
  return (
    <span
      className={cn(
        "font-[var(--font-code)]",
        variant === "hero" ? "text-white" : "text-card-foreground",
      )}
    >
      {value}
    </span>
  );
}

function hasSourceId(rating: TitleExternalRating, sourceIds: Set<string>) {
  return sourceIds.has(normalizedRatingSource(rating.source));
}

function firstRatingWithSourceId(
  ratings: TitleExternalRating[],
  sourceIds: Set<string>,
) {
  return ratings.find((rating) => hasSourceId(rating, sourceIds));
}

function isRottenTomatoesRating(rating: TitleExternalRating) {
  return (
    hasSourceId(rating, ROTTEN_TOMATOES_CRITIC_SOURCE_IDS) ||
    hasSourceId(rating, ROTTEN_TOMATOES_AUDIENCE_SOURCE_IDS)
  );
}

function isMetacriticRating(rating: TitleExternalRating) {
  return (
    hasSourceId(rating, METACRITIC_CRITIC_SOURCE_IDS) ||
    hasSourceId(rating, METACRITIC_USER_SOURCE_IDS)
  );
}

function groupedRatingPillEntries(
  ratings: TitleExternalRating[],
): RatingPillEntry[] {
  const entries: RatingPillEntry[] = [];
  const emittedGroupKeys = new Set<string>();

  for (const rating of ratings) {
    if (isRottenTomatoesRating(rating)) {
      if (!emittedGroupKeys.has("rotten-tomatoes")) {
        const groupRatings = [
          firstRatingWithSourceId(ratings, ROTTEN_TOMATOES_CRITIC_SOURCE_IDS),
          firstRatingWithSourceId(ratings, ROTTEN_TOMATOES_AUDIENCE_SOURCE_IDS),
        ].filter((entry): entry is TitleExternalRating => entry !== undefined);
        entries.push({
          key: "rotten-tomatoes",
          kind: "rotten-tomatoes",
          ratings: groupRatings,
        });
        emittedGroupKeys.add("rotten-tomatoes");
      }
      continue;
    }

    if (isMetacriticRating(rating)) {
      if (!emittedGroupKeys.has("metacritic")) {
        const groupRatings = [
          firstRatingWithSourceId(ratings, METACRITIC_CRITIC_SOURCE_IDS),
          firstRatingWithSourceId(ratings, METACRITIC_USER_SOURCE_IDS),
        ].filter((entry): entry is TitleExternalRating => entry !== undefined);
        entries.push({
          key: "metacritic",
          kind: "metacritic",
          ratings: groupRatings,
        });
        emittedGroupKeys.add("metacritic");
      }
      continue;
    }

    entries.push({
      key: `${rating.source}-${entries.length}`,
      kind: "single",
      rating,
    });
  }

  return entries;
}

function ratingAriaLabel(rating: TitleExternalRating) {
  const source = ratingSourceInfo(rating.source);
  return `${source.label}: ${ratingValueLabel(rating, source)}`;
}

function renderSingleRatingPill(
  rating: TitleExternalRating,
  variant: TitleRatingsDisplayVariant,
) {
  const source = ratingSourceInfo(rating.source);
  const value = ratingValueLabel(rating, source);

  return (
    <RatingPill
      key={rating.source}
      ariaLabel={`${source.label}: ${value}`}
      tooltipLabel={source.label}
      variant={variant}
    >
      <RatingSourceLogo source={source} variant={variant} />
      <RatingValue value={value} variant={variant} />
    </RatingPill>
  );
}

function renderStaticRatingPill({
  sourceName,
  value,
  variant,
}: {
  sourceName: string;
  value: string;
  variant: TitleRatingsDisplayVariant;
}) {
  const source = ratingSourceInfo(sourceName);

  return (
    <RatingPill
      key={sourceName}
      ariaLabel={`${source.label}: ${value}`}
      tooltipLabel={source.label}
      variant={variant}
    >
      <RatingSourceLogo source={source} variant={variant} />
      <RatingValue value={value} variant={variant} />
    </RatingPill>
  );
}

function renderNeutralRatingPill({
  value,
  variant,
}: {
  value: string;
  variant: TitleRatingsDisplayVariant;
}) {
  return (
    <RatingPill
      key="rating-score"
      ariaLabel={`Rating: ${value}`}
      tooltipLabel={`Rating: ${value}`}
      variant={variant}
    >
      <RatingValue value={value} variant={variant} />
    </RatingPill>
  );
}

function renderRottenTomatoesPill(
  ratings: TitleExternalRating[],
  variant: TitleRatingsDisplayVariant,
) {
  const tooltipLabel = ratings
    .map((rating) => ratingSourceInfo(rating.source).label)
    .join(" / ");

  return (
    <RatingPill
      key="rotten-tomatoes"
      ariaLabel={ratings.map(ratingAriaLabel).join(", ")}
      tooltipLabel={tooltipLabel}
      variant={variant}
    >
      {ratings.map((rating) => {
        const source = ratingSourceInfo(rating.source);
        return (
          <span
            key={rating.source}
            className="inline-flex items-center gap-1.5"
          >
            <RatingSourceLogo source={source} variant={variant} />
            <RatingValue
              value={ratingValueLabel(rating, source)}
              variant={variant}
            />
          </span>
        );
      })}
    </RatingPill>
  );
}

function renderMetacriticPill(
  ratings: TitleExternalRating[],
  variant: TitleRatingsDisplayVariant,
) {
  const source = ratingSourceInfo("metacritic");
  const tooltipLabel = ratings
    .map((rating) => ratingSourceInfo(rating.source).label)
    .join(" / ");

  return (
    <RatingPill
      key="metacritic"
      ariaLabel={ratings.map(ratingAriaLabel).join(", ")}
      tooltipLabel={tooltipLabel}
      variant={variant}
    >
      <RatingSourceLogo source={source} variant={variant} />
      {ratings.map((rating, index) => {
        const ratingSource = ratingSourceInfo(rating.source);
        return (
          <span
            key={rating.source}
            className="inline-flex items-center gap-1.5"
          >
            {index > 0 ? (
              <span
                aria-hidden="true"
                className={
                  variant === "hero" ? "text-white/55" : "text-muted-foreground"
                }
              >
                |
              </span>
            ) : null}
            <RatingValue
              value={ratingValueLabel(rating, ratingSource)}
              variant={variant}
            />
          </span>
        );
      })}
    </RatingPill>
  );
}

function renderRatingPillEntry(
  entry: RatingPillEntry,
  variant: TitleRatingsDisplayVariant,
) {
  if (entry.kind === "single") {
    return renderSingleRatingPill(entry.rating, variant);
  }
  if (entry.kind === "rotten-tomatoes") {
    return renderRottenTomatoesPill(entry.ratings, variant);
  }
  return renderMetacriticPill(entry.ratings, variant);
}

export function TitleRatingsDisplay({
  externalRatings = [],
  fallbackRating = null,
  fallbackSources = [],
  variant = "default",
  className,
}: TitleRatingsDisplayProps) {
  const visibleExternalRatings = visibleTitleExternalRatings(externalRatings);
  const ratingPillEntries = groupedRatingPillEntries(visibleExternalRatings);
  if (visibleExternalRatings.length === 0 && fallbackRating == null) {
    return null;
  }

  const fallbackTopSource = topOrderedRatingSource(fallbackSources);

  return (
    <TooltipProvider delayDuration={200}>
      <div className={cn("flex flex-wrap gap-2", className)}>
        {ratingPillEntries.map((entry) =>
          renderRatingPillEntry(entry, variant),
        )}
        {visibleExternalRatings.length === 0 && fallbackRating != null
          ? fallbackTopSource != null
            ? renderStaticRatingPill({
                sourceName: fallbackTopSource,
                value: compactRatingNumber(fallbackRating),
                variant,
              })
            : renderNeutralRatingPill({
                value: compactRatingNumber(fallbackRating),
                variant,
              })
          : null}
      </div>
    </TooltipProvider>
  );
}

import { ExternalLink } from "lucide-react";
import type { InterstitialMovieMetadata } from "@/components/containers/series-overview-container";
import { selectPosterVariantUrl } from "@/lib/utils/poster-images";
import { TitlePoster } from "@/components/title-poster";
import { getImdbUrl, getTvdbMovieUrl, formatRuntimeFromMinutes } from "./helpers";

type InterstitialMoviePanelProps = {
  movie: InterstitialMovieMetadata;
  hasFile?: boolean;
  monitored?: boolean;
};

export function InterstitialMoviePanel({ movie, hasFile, monitored }: InterstitialMoviePanelProps) {
  const imdbUrl = getImdbUrl(movie.imdbId);
  const tvdbUrl = getTvdbMovieUrl(movie);
  const runtime = formatRuntimeFromMinutes(movie.runtimeMinutes);
  const posterUrl = selectPosterVariantUrl(movie.posterUrl, "w250");
  const badges = buildMovieBadges(movie, hasFile, monitored);

  return (
    <div className="flex flex-col gap-4 sm:flex-row sm:items-start">
      <div className="shrink-0">
        {posterUrl ? (
          <TitlePoster
            src={posterUrl}
            alt={movie.name}
            className="h-auto w-28 rounded-lg object-cover shadow-md sm:w-[140px]"
          />
        ) : (
          <div className="flex h-40 w-28 items-center justify-center rounded-lg bg-muted text-sm text-muted-foreground/60 sm:h-[210px] sm:w-[140px]">
            No Poster
          </div>
        )}
      </div>
      <div className="min-w-0 flex-1">
        <p className="text-sm font-semibold text-card-foreground">{movie.name}</p>
        {badges.length > 0 ? (
          <div className="mt-2 flex flex-wrap gap-2">
            {badges.map((badge) => (
              <span
                key={`${badge.label}-${badge.tone}`}
                className={`rounded-full border px-2 py-0.5 text-[11px] font-medium ${badgeClassName(badge.tone)}`}
              >
                {badge.label}
              </span>
            ))}
          </div>
        ) : null}
        <div className="mt-1 flex flex-wrap gap-2 text-xs text-muted-foreground">
          {movie.year ? (
            <span>{movie.year}</span>
          ) : null}
          {runtime ? (
            <span>{runtime}</span>
          ) : null}
          {movie.contentStatus ? (
            <span className="capitalize">{movie.contentStatus}</span>
          ) : null}
        </div>
        {movie.overview ? (
          <p className="mt-3 text-sm leading-relaxed text-muted-foreground">{movie.overview}</p>
        ) : (
          <p className="mt-3 text-sm italic text-muted-foreground/60">No description available.</p>
        )}
        {movie.signalSummary ? (
          <p className="mt-2 text-xs text-muted-foreground/80">{movie.signalSummary}</p>
        ) : null}
        <div className="mt-3 flex flex-wrap gap-2 text-sm">
          {imdbUrl ? (
            <a
              href={imdbUrl}
              target="_blank"
              rel="noreferrer"
              className="inline-flex h-10 items-center gap-2 rounded-md border border-border bg-card/45 px-3 py-2 text-xs text-card-foreground hover:bg-muted"
              aria-label="Open on IMDb"
            >
              <ExternalLink className="h-3.5 w-3.5 text-muted-foreground" />
              IMDb
            </a>
          ) : null}
          {tvdbUrl ? (
            <a
              href={tvdbUrl}
              target="_blank"
              rel="noreferrer"
              className="inline-flex h-10 items-center gap-2 rounded-md border border-border bg-card/45 px-3 py-2 text-xs text-card-foreground hover:bg-muted"
              aria-label="Open on TVDB"
            >
              <ExternalLink className="h-3.5 w-3.5 text-muted-foreground" />
              TVDB
            </a>
          ) : null}
        </div>
      </div>
    </div>
  );
}

function buildMovieBadges(movie: InterstitialMovieMetadata, hasFile?: boolean, monitored?: boolean): Array<{ label: string; tone: "emerald" | "amber" | "slate" | "sky" | "red" }> {
  const badges: Array<{ label: string; tone: "emerald" | "amber" | "slate" | "sky" | "red" }> = [];

  // File status badge
  if (hasFile === true) {
    badges.push({ label: "Downloaded", tone: "emerald" });
  } else if (monitored === true && hasFile === false) {
    badges.push({ label: "Missing", tone: "red" });
  } else if (monitored === false) {
    badges.push({ label: "Unmonitored", tone: "slate" });
  }

  if (movie.movieForm === "recap") {
    badges.push({ label: "Recap", tone: "slate" });
  } else if (movie.movieForm === "special") {
    badges.push({ label: "Special", tone: "slate" });
  } else if (movie.continuityStatus === "filler") {
    badges.push({ label: "Filler", tone: "slate" });
  } else if (movie.continuityStatus === "canon") {
    badges.push({ label: "Canon", tone: "emerald" });
  } else if (movie.continuityStatus === "mixed") {
    badges.push({ label: "Mixed", tone: "amber" });
  }

  return badges;
}

function badgeClassName(tone: "emerald" | "amber" | "slate" | "sky" | "red") {
  switch (tone) {
    case "emerald":
      return "border-emerald-500/30 bg-emerald-500/10 text-emerald-200";
    case "amber":
      return "border-amber-500/30 bg-amber-500/10 text-amber-100";
    case "sky":
      return "border-sky-500/30 bg-sky-500/10 text-sky-100";
    case "red":
      return "border-red-500/30 bg-red-500/10 text-red-200";
    default:
      return "border-border bg-muted/30 text-muted-foreground";
  }
}

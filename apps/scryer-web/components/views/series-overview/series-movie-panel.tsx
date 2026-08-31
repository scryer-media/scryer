import type { SeriesMovieLink } from "@/components/containers/series-overview-container";
import type { ReactNode } from "react";
import { useTranslate } from "@/lib/context/translate-context";
import { selectPosterVariantUrl } from "@/lib/utils/poster-images";
import { TitlePoster } from "@/components/title-poster";
import { localizedTitleStatus } from "../overview-localization";
import {
  AnidbExternalLink,
  ImdbExternalLink,
  MalExternalLink,
  TmdbExternalLink,
  TvdbMovieExternalLink,
} from "@/components/common/external-media-links";
import { formatRuntimeFromMinutes } from "./helpers";
import { TitleRatingsStrip } from "../title-ratings-strip";
import { TitleCastStrip } from "../title-cast-strip";
import { TitleDubCastStrip } from "../title-dub-cast-strip";
import { titleCastOriginalCredits } from "@/lib/utils/title-cast";

type SeriesMoviePanelProps = {
  link: SeriesMovieLink;
  hasFile?: boolean;
  filesOnDisk?: ReactNode;
};

export function SeriesMoviePanel({
  link,
  hasFile,
  filesOnDisk,
}: SeriesMoviePanelProps) {
  const t = useTranslate();
  const movie = link.movie;
  const runtime = formatRuntimeFromMinutes(movie.runtimeMinutes);
  const posterUrl = selectPosterVariantUrl(movie.posterUrl, "w250");
  const badges = buildMovieBadges(link, hasFile, t);
  const localizedStatus = localizedTitleStatus(t, movie.contentStatus);

  return (
    <div className="space-y-5">
      <div className="flex flex-col gap-4 sm:flex-row sm:items-start">
        <div className="shrink-0">
          {posterUrl ? (
            <TitlePoster
              src={posterUrl}
              alt={movie.title}
              className="h-auto w-28 rounded-lg object-cover shadow-md sm:w-[140px]"
            />
          ) : (
            <div className="flex h-40 w-28 items-center justify-center rounded-lg bg-muted text-sm text-muted-foreground/60 sm:h-[210px] sm:w-[140px]">
              {t("title.noPoster")}
            </div>
          )}
        </div>
        <div className="min-w-0 flex-1">
          <h3 className="text-xl font-bold leading-tight text-card-foreground">
            {movie.title}
            {movie.year ? (
              <span className="font-medium text-muted-foreground"> ({movie.year})</span>
            ) : null}
          </h3>
          {badges.length > 0 ? (
            <div className="mt-2 flex flex-wrap gap-2">
              {badges.map((badge) => (
                <span
                  key={`${badge.label}-${badge.tone}`}
                  className={`inline-flex h-6 items-center rounded-[7px] border px-2.5 text-[11px] font-semibold ${badgeClassName(badge.tone)}`}
                >
                  {badge.label}
                </span>
              ))}
            </div>
          ) : null}
          <div className="mt-2 flex flex-wrap gap-2 text-xs font-medium text-muted-foreground">
            {runtime ? <span>{runtime}</span> : null}
            {localizedStatus ? <span>{localizedStatus}</span> : null}
          </div>
          <TitleRatingsStrip ratings={movie.ratings} />
          {movie.overview ? (
            <p className="mt-3 text-sm leading-relaxed text-muted-foreground">{movie.overview}</p>
          ) : (
            <p className="mt-3 text-sm italic text-muted-foreground/60">
              {t("title.descriptionUnavailable")}
            </p>
          )}
          {link.signalSummary ? (
            <p className="mt-2 text-xs text-muted-foreground/80">{link.signalSummary}</p>
          ) : null}
          <div className="mt-3 flex flex-wrap gap-2 text-sm">
            <ImdbExternalLink imdbId={movie.imdbId} size="compact" />
            <TvdbMovieExternalLink tvdbId={movie.tvdbId} slug={movie.slug} size="compact" />
            <TmdbExternalLink mediaType="movie" tmdbId={movie.tmdbId} size="compact" />
            <MalExternalLink malId={movie.malId} size="compact" />
            <AnidbExternalLink anidbId={movie.anidbId} size="compact" />
          </div>
        </div>
      </div>
      {filesOnDisk}
      <TitleCastStrip credits={titleCastOriginalCredits(movie.credits)} />
      <TitleDubCastStrip credits={movie.credits} />
    </div>
  );
}

function buildMovieBadges(
  link: SeriesMovieLink,
  hasFile: boolean | undefined,
  t: (key: string, values?: Record<string, string | number | boolean | null | undefined>) => string,
): Array<{ label: string; tone: "emerald" | "amber" | "slate" | "red" }> {
  const badges: Array<{ label: string; tone: "emerald" | "amber" | "slate" | "red" }> = [];

  if (hasFile === true) {
    badges.push({ label: t("history.downloadCompleted"), tone: "emerald" });
  } else if (link.monitored) {
    badges.push({ label: t("episode.missing"), tone: "red" });
  } else {
    badges.push({ label: t("search.monitorType.unmonitored"), tone: "slate" });
  }

  if (link.metadataActive === false) {
    badges.push({ label: "Metadata inactive", tone: "amber" });
  }

  if (link.movieForm === "recap") {
    badges.push({ label: t("episode.recap"), tone: "slate" });
  } else if (link.movieForm === "special") {
    badges.push({ label: t("episode.special"), tone: "slate" });
  } else if (link.continuityStatus === "filler") {
    badges.push({ label: t("episode.filler"), tone: "slate" });
  } else if (link.continuityStatus === "canon") {
    badges.push({ label: t("title.canon"), tone: "emerald" });
  } else if (link.continuityStatus === "mixed") {
    badges.push({ label: t("title.mixed"), tone: "amber" });
  }

  return badges;
}

function badgeClassName(tone: "emerald" | "amber" | "slate" | "red") {
  switch (tone) {
    case "emerald":
      return "border-[var(--scry-success-border)] bg-[var(--scry-success-bg)] text-[var(--scry-success-text)]";
    case "amber":
      return "border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] text-[var(--scry-warning-text)]";
    case "red":
      return "border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] text-[var(--scry-danger-text)]";
    default:
      return "border-border bg-muted/30 text-muted-foreground";
  }
}

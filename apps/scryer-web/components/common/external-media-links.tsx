import { useTranslate } from "@/lib/context/translate-context";
import { cn } from "@/lib/utils";
import {
  buildTvdbMovieUrl,
  buildTvdbSeriesUrl,
} from "@/lib/utils/external-media-links";
import { ExternalLink } from "lucide-react";

const imdbLogoUrl = `${import.meta.env.BASE_URL}media-sites/imdb.svg`;
const tvdbLogoUrl = `${import.meta.env.BASE_URL}media-sites/tvdb.svg`;
const tmdbLogoUrl = `${import.meta.env.BASE_URL}media-sites/tmdb.svg`;
const malLogoUrl = `${import.meta.env.BASE_URL}media-sites/mal.svg`;
const anilistLogoUrl = `${import.meta.env.BASE_URL}media-sites/anilist.svg`;
const anidbLogoUrl = `${import.meta.env.BASE_URL}media-sites/anidb.png`;

type ExternalMediaLinkButtonProps = {
  href: string | null;
  site: string;
  label?: string;
  logoSrc: string;
  size?: ExternalMediaLinkSize;
};

type ExternalMediaLinkSize = "default" | "compact";
type ExternalMediaLinkSizeProps = { size?: ExternalMediaLinkSize };

export function ExternalMediaLinkButton({
  href,
  site,
  label = site,
  logoSrc,
  size = "default",
}: ExternalMediaLinkButtonProps) {
  const t = useTranslate();
  if (!href) {
    return null;
  }

  return (
    <a
      href={href}
      target="_blank"
      rel="noreferrer"
      className={cn(
        "inline-flex items-center gap-2 rounded-md border transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
        size === "compact"
          ? "h-8 border-[var(--scry-border3)] bg-background/80 px-2.5 py-1 text-xs font-semibold text-[var(--scry-muted2)] hover:bg-[var(--scry-hover)]"
          : "h-12 border-border bg-card/45 px-3 py-2 text-base hover:bg-muted",
      )}
      aria-label={t("external.openOn", { site })}
    >
      <img
        src={logoSrc}
        alt={site}
        className={cn(size === "compact" ? "h-4 w-4" : "h-8 w-8")}
      />
      <span
        className={cn(
          size === "compact"
            ? "text-[var(--scry-muted2)]"
            : "text-muted-foreground",
        )}
      >
        {label}
      </span>
      <ExternalLink
        className={cn(
          "shrink-0",
          size === "compact"
            ? "h-3.5 w-3.5 text-[var(--scry-muted3)]"
            : "h-4 w-4 text-muted-foreground",
        )}
        aria-hidden="true"
      />
    </a>
  );
}

export function ImdbExternalLink({
  imdbId,
  size,
}: { imdbId: string | null | undefined } & ExternalMediaLinkSizeProps) {
  return (
    <ExternalMediaLinkButton
      href={imdbTitleUrl(imdbId)}
      site="IMDb"
      logoSrc={imdbLogoUrl}
      size={size}
    />
  );
}

export function TvdbMovieExternalLink({
  slug,
  tvdbId,
  size,
}: {
  slug?: string | null;
  tvdbId: string | null | undefined;
} & ExternalMediaLinkSizeProps) {
  return (
    <ExternalMediaLinkButton
      href={buildTvdbMovieUrl(tvdbId, slug)}
      site="TVDB"
      logoSrc={tvdbLogoUrl}
      size={size}
    />
  );
}

export function TvdbSeriesExternalLink({
  slug,
  tvdbId,
  size,
}: {
  slug?: string | null;
  tvdbId?: string | null;
} & ExternalMediaLinkSizeProps) {
  return (
    <ExternalMediaLinkButton
      href={buildTvdbSeriesUrl(slug, tvdbId)}
      site="TVDB"
      logoSrc={tvdbLogoUrl}
      size={size}
    />
  );
}

export function TmdbExternalLink({
  mediaType,
  tmdbId,
  size,
}: {
  mediaType: "movie" | "tv";
  tmdbId: string | null | undefined;
} & ExternalMediaLinkSizeProps) {
  return (
    <ExternalMediaLinkButton
      href={tmdbUrl(tmdbId, mediaType)}
      site="TMDB"
      logoSrc={tmdbLogoUrl}
      size={size}
    />
  );
}

export function MalExternalLink({
  malId,
  size,
}: { malId: string | null | undefined } & ExternalMediaLinkSizeProps) {
  return (
    <ExternalMediaLinkButton
      href={numericProviderUrl("https://myanimelist.net/anime", malId)}
      site="MyAnimeList"
      label="MAL"
      logoSrc={malLogoUrl}
      size={size}
    />
  );
}

export function AnilistExternalLink({
  anilistId,
  size,
}: { anilistId: string | null | undefined } & ExternalMediaLinkSizeProps) {
  return (
    <ExternalMediaLinkButton
      href={numericProviderUrl("https://anilist.co/anime", anilistId)}
      site="AniList"
      logoSrc={anilistLogoUrl}
      size={size}
    />
  );
}

export function AnidbExternalLink({
  anidbId,
  size,
}: { anidbId: string | null | undefined } & ExternalMediaLinkSizeProps) {
  return (
    <ExternalMediaLinkButton
      href={numericProviderUrl("https://anidb.net/anime", anidbId)}
      site="AniDB"
      logoSrc={anidbLogoUrl}
      size={size}
    />
  );
}

function imdbTitleUrl(imdbId: string | null | undefined) {
  const trimmed = imdbId?.trim();
  if (!trimmed) {
    return null;
  }
  if (trimmed.startsWith("tt")) {
    return `https://www.imdb.com/title/${trimmed}`;
  }
  return `https://www.imdb.com/find?q=${encodeURIComponent(trimmed)}&s=tt`;
}

function tmdbUrl(tmdbId: string | null | undefined, mediaType: "movie" | "tv") {
  const trimmed = tmdbId?.trim();
  if (!trimmed) {
    return null;
  }
  return `https://www.themoviedb.org/${mediaType}/${encodeURIComponent(trimmed)}`;
}

function numericProviderUrl(
  baseUrl: string,
  providerId: string | null | undefined,
) {
  const trimmed = providerId?.trim();
  if (!trimmed) {
    return null;
  }
  return `${baseUrl}/${encodeURIComponent(trimmed)}`;
}

import { Clock3 } from "lucide-react";

import { ExternalMediaLinkButton } from "@/components/common/external-media-links";
import { TitlePoster } from "@/components/title-poster";
import {
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { TitleRatingsStrip } from "@/components/views/title-ratings-strip";
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import type { Facet } from "@/lib/types";
import {
  selectBackdropVariantUrl,
  selectPosterVariantUrl,
} from "@/lib/utils/poster-images";

type CatalogActionDialogSummaryProps = {
  result: MetadataTvdbSearchItem;
  facet: Facet;
  mode: "add" | "request";
};

type ExternalSiteLink = {
  id: string;
  site: string;
  label: string;
  href: string;
  logoSrc: string;
};

const mediaSiteLogo = (path: string) => `${import.meta.env.BASE_URL}${path}`;

const MEDIA_SITE_LOGOS: Record<string, string> = {
  anidb: mediaSiteLogo("media-sites/anidb.png"),
  anilist: mediaSiteLogo("media-sites/anilist.svg"),
  imdb: mediaSiteLogo("media-sites/imdb.svg"),
  mal: mediaSiteLogo("media-sites/mal.svg"),
  tmdb: mediaSiteLogo("media-sites/tmdb.svg"),
  trakt: mediaSiteLogo("rating-sources/trakt.svg"),
  tvdb: mediaSiteLogo("media-sites/tvdb.svg"),
};

function normalizeSource(value: string) {
  return value.trim().toLowerCase().replace(/[\s_.-]+/g, "");
}

const EXTERNAL_SOURCE_ALIASES: Record<string, string> = {
  anidb: "anidb",
  anidbnet: "anidb",
  anilist: "anilist",
  anilistco: "anilist",
  imdb: "imdb",
  imdbcom: "imdb",
  mal: "mal",
  myanimelist: "mal",
  myanimelistnet: "mal",
  themoviedb: "tmdb",
  themoviedborg: "tmdb",
  tmdb: "tmdb",
  trakt: "trakt",
  traktv: "trakt",
  thetvdb: "tvdb",
  thetvdbcom: "tvdb",
  tvdb: "tvdb",
};

function canonicalExternalSource(value: string) {
  return EXTERNAL_SOURCE_ALIASES[normalizeSource(value)] ?? normalizeSource(value);
}

function externalIdMap(result: MetadataTvdbSearchItem) {
  const ids = new Map<string, string>();
  for (const externalId of result.externalIds ?? []) {
    const source = canonicalExternalSource(externalId.source);
    const value = externalId.value.trim();
    if (source && value && !ids.has(source)) {
      ids.set(source, value);
    }
  }
  if (result.imdbId?.trim() && !ids.has("imdb")) {
    ids.set("imdb", result.imdbId.trim());
  }
  if (result.smgId != null && !ids.has("smg")) {
    ids.set("smg", String(result.smgId));
  }
  if (result.tmdbId != null && !ids.has("tmdb")) {
    ids.set("tmdb", String(result.tmdbId));
  }
  if (result.tvdbId.trim() && !ids.has("tvdb")) {
    ids.set("tvdb", result.tvdbId.trim());
  }
  return ids;
}

function tmdbPathForFacet(facet: Facet, type: string | null | undefined) {
  const normalizedType = type?.trim().toLowerCase();
  if (normalizedType === "movie" || facet === "MOVIE") {
    return "movie";
  }
  return "tv";
}

function tvdbPathForFacet(facet: Facet, type: string | null | undefined) {
  const normalizedType = type?.trim().toLowerCase();
  if (normalizedType === "movie" || facet === "MOVIE") {
    return "movie";
  }
  return "series";
}

function externalLinks(result: MetadataTvdbSearchItem, facet: Facet): ExternalSiteLink[] {
  const ids = externalIdMap(result);
  const links: ExternalSiteLink[] = [];
  const seen = new Set<string>();
  const push = (link: ExternalSiteLink | null) => {
    if (!link || seen.has(link.id)) {
      return;
    }
    seen.add(link.id);
    links.push(link);
  };

  const imdbId = ids.get("imdb");
  push(
    imdbId
      ? {
          id: "imdb",
          site: "IMDb",
          label: "IMDb",
          href: `https://www.imdb.com/title/${encodeURIComponent(imdbId)}/`,
          logoSrc: MEDIA_SITE_LOGOS.imdb,
        }
      : null,
  );

  const tmdbId = ids.get("tmdb");
  push(
    tmdbId
      ? {
          id: "tmdb",
          site: "TMDB",
          label: "TMDB",
          href: `https://www.themoviedb.org/${tmdbPathForFacet(
            facet,
            result.type,
          )}/${encodeURIComponent(tmdbId)}`,
          logoSrc: MEDIA_SITE_LOGOS.tmdb,
        }
      : null,
  );

  const tvdbId = ids.get("tvdb");
  push(
    tvdbId
      ? {
          id: "tvdb",
          site: "TVDB",
          label: "TVDB",
          href: `https://thetvdb.com/dereferrer/${tvdbPathForFacet(
            facet,
            result.type,
          )}/${encodeURIComponent(tvdbId)}`,
          logoSrc: MEDIA_SITE_LOGOS.tvdb,
        }
      : null,
  );

  const malId = ids.get("mal") ?? ids.get("myanimelist");
  push(
    malId
      ? {
          id: "mal",
          site: "MyAnimeList",
          label: "MAL",
          href: `https://myanimelist.net/anime/${encodeURIComponent(malId)}`,
          logoSrc: MEDIA_SITE_LOGOS.mal,
        }
      : null,
  );

  const anilistId = ids.get("anilist");
  push(
    anilistId
      ? {
          id: "anilist",
          site: "AniList",
          label: "AniList",
          href: `https://anilist.co/anime/${encodeURIComponent(anilistId)}`,
          logoSrc: MEDIA_SITE_LOGOS.anilist,
        }
      : null,
  );

  const anidbId = ids.get("anidb");
  push(
    anidbId
      ? {
          id: "anidb",
          site: "AniDB",
          label: "AniDB",
          href: `https://anidb.net/anime/${encodeURIComponent(anidbId)}`,
          logoSrc: MEDIA_SITE_LOGOS.anidb,
        }
      : null,
  );

  return links;
}

function runtimeLabel(runtimeMinutes: number | null) {
  if (runtimeMinutes == null || runtimeMinutes <= 0) {
    return null;
  }
  const hours = Math.floor(runtimeMinutes / 60);
  const minutes = runtimeMinutes % 60;
  if (hours <= 0) {
    return `${minutes}m`;
  }
  return minutes > 0 ? `${hours}h ${minutes}m` : `${hours}h`;
}

export function CatalogActionDialogSummary({
  result,
  facet,
}: CatalogActionDialogSummaryProps) {
  const posterUrl = selectPosterVariantUrl(result.posterUrl, "w250");
  const backgroundUrl =
    selectBackdropVariantUrl(result.backgroundUrl, "original") ??
    selectPosterVariantUrl(result.posterUrl, "original");
  const links = externalLinks(result, facet);
  const runtime = runtimeLabel(result.runtimeMinutes);
  const ratingSources = [
    result.ratingSource,
    ...(result.ratingSources ?? []),
  ].filter((source): source is string => Boolean(source?.trim()));

  return (
    <DialogHeader className="relative isolate overflow-hidden rounded-t-lg border-b border-border/70 p-5 text-left sm:p-7">
      {backgroundUrl ? (
        <>
          <img
            src={backgroundUrl}
            alt=""
            aria-hidden="true"
            className="absolute inset-0 -z-20 h-full w-full scale-105 object-cover opacity-55 blur-md"
          />
          <div
            aria-hidden="true"
            className="absolute inset-0 -z-10 bg-gradient-to-r from-background/92 via-background/72 to-background/34"
          />
        </>
      ) : null}
      <div className="grid gap-5 sm:grid-cols-[minmax(120px,180px)_1fr] sm:gap-7">
        <div className="mx-auto aspect-[2/3] w-36 overflow-hidden rounded-xl border border-border/80 bg-muted shadow-2xl sm:mx-0 sm:w-full">
          {posterUrl ? (
            <TitlePoster
              src={posterUrl}
              alt={result.name}
              className="h-full w-full object-cover"
            />
          ) : (
            <div className="flex h-full w-full items-center justify-center px-4 text-center text-xs text-muted-foreground">
              No art
            </div>
          )}
        </div>

        <div className="min-w-0 space-y-4">
          <div className="min-w-0">
            <DialogTitle className="text-3xl font-bold leading-tight tracking-normal text-foreground sm:text-4xl">
              {result.name}
              {result.year ? (
                <span className="font-semibold text-muted-foreground">
                  {" "}
                  ({result.year})
                </span>
              ) : null}
            </DialogTitle>
            <DialogDescription className="sr-only">
              {result.overview || result.name}
            </DialogDescription>
          </div>

          <TitleRatingsStrip
            ratings={{
              rating: result.rating ?? null,
              ratingSources,
              externalRatings: result.externalRatings ?? [],
            }}
            variant="hero"
          />

          {runtime ? (
            <div className="flex flex-wrap items-center gap-2">
              <span className="inline-flex items-center gap-1.5 px-1 py-1 text-sm font-medium text-muted-foreground">
                <Clock3 className="h-4 w-4" />
                {runtime}
              </span>
            </div>
          ) : null}

          {result.overview ? (
            <p className="max-w-3xl text-sm leading-6 text-muted-foreground sm:text-base">
              {result.overview}
            </p>
          ) : null}

          {links.length > 0 ? (
            <div className="flex flex-wrap items-center gap-2 pt-1">
              {links.map((link) => (
                <ExternalMediaLinkButton
                  key={link.id}
                  href={link.href}
                  site={link.site}
                  label={link.label}
                  logoSrc={link.logoSrc}
                  size="compact"
                />
              ))}
            </div>
          ) : null}
        </div>
      </div>
    </DialogHeader>
  );
}

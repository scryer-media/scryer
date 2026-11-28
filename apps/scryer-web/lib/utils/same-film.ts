import { facetById } from "../facets/registry.ts";
import type { Facet, SameFilmPairing } from "../types/titles.ts";

/** A place the same film is also tracked, ready to link to. */
export type SameFilmPlace = {
  key: string;
  name: string;
  href: string | null;
};

function titleHref(
  facet: Facet | null,
  titleId: string | null,
  seriesMovieLinkId?: string,
): string | null {
  const id = titleId?.trim();
  const definition = facet ? facetById(facet) : undefined;
  if (!definition || !id) return null;
  const params = new URLSearchParams({ id });
  if (seriesMovieLinkId) params.set("seriesMovie", seriesMovieLinkId);
  return `/${definition.viewId}?${params}`;
}

/** The shows that hold this film as a series movie, for a movie title. */
export function seriesMoviePlaces(
  pairings: readonly SameFilmPairing[] | null | undefined,
): SameFilmPlace[] {
  const seen = new Set<string>();
  const places: SameFilmPlace[] = [];
  for (const pairing of pairings ?? []) {
    if (seen.has(pairing.seriesMovieLinkId)) continue;
    seen.add(pairing.seriesMovieLinkId);
    places.push({
      key: pairing.seriesMovieLinkId,
      name: pairing.seriesTitleName,
      href: titleHref(
        pairing.seriesTitleFacet,
        pairing.seriesTitleId,
        pairing.seriesMovieLinkId,
      ),
    });
  }
  return places;
}

/** The movie titles that hold each series movie of a show, by link id. */
export function movieTitlePlacesByLink(
  pairings: readonly SameFilmPairing[] | null | undefined,
  seriesTitleId: string,
): Map<string, SameFilmPlace[]> {
  const byLink = new Map<string, SameFilmPlace[]>();
  for (const pairing of pairings ?? []) {
    if (pairing.seriesTitleId !== seriesTitleId || !pairing.movieTitleId) {
      continue;
    }
    const places = byLink.get(pairing.seriesMovieLinkId) ?? [];
    if (places.some((place) => place.key === pairing.movieTitleId)) continue;
    places.push({
      key: pairing.movieTitleId,
      name: pairing.movieTitleName ?? "",
      href: titleHref(pairing.movieTitleFacet, pairing.movieTitleId),
    });
    byLink.set(pairing.seriesMovieLinkId, places);
  }
  return byLink;
}

/**
 * The TMDB movie id of a search result about to be added, or null when it is
 * not a film: a Movies result's TMDB id, or a TMDB id of kind `movie`.
 */
export function filmTmdbIdForAdd(
  result: {
    tmdbId?: number | null;
    externalIds?: ReadonlyArray<{
      source: string;
      kind?: string | null;
      value: string;
    }>;
  },
  facet: Facet,
): string | null {
  if (facet === "MOVIE" && result.tmdbId) return String(result.tmdbId);
  const kinded = result.externalIds?.find(
    (id) =>
      id.source.trim().toLowerCase() === "tmdb" &&
      id.kind?.trim().toLowerCase() === "movie" &&
      id.value.trim() !== "",
  );
  return kinded ? kinded.value.trim() : null;
}

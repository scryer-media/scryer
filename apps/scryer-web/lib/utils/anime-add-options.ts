import type { Facet } from "@/lib/types";

type AnimeAddChoices = {
  monitorSpecials?: boolean;
  interSeasonMovies?: boolean;
};

/**
 * The anime-only add options for an `addTitle` request. A choice the user did
 * not make is left out, so the server applies the library's anime settings.
 */
export function animeAddTitleOptions(
  facet: Facet,
  choices: AnimeAddChoices,
): AnimeAddChoices {
  if (facet !== "ANIME") {
    return {};
  }
  return {
    ...(choices.monitorSpecials !== undefined
      ? { monitorSpecials: choices.monitorSpecials }
      : {}),
    ...(choices.interSeasonMovies !== undefined
      ? { interSeasonMovies: choices.interSeasonMovies }
      : {}),
  };
}

/**
 * Whether Advanced starts with specials selected, or null while that is not
 * known yet. The chosen library's resolved setting decides, as it does for the
 * add itself; the instance's setting stands in only when the library's could
 * not be read (`librarySpecials` loaded as null).
 */
export function advancedSpecialsDefault({
  facet,
  libraryId,
  librarySpecials,
  instanceSpecials,
}: {
  facet: Facet;
  libraryId: string | null;
  /** Undefined while the library's setting is loading. */
  librarySpecials: boolean | null | undefined;
  /** Null while the instance's setting is unread or unreadable. */
  instanceSpecials: boolean | null;
}): boolean | null {
  if (facet !== "ANIME") return false;
  if (!libraryId) return instanceSpecials ?? false;
  if (librarySpecials === undefined) return null;
  return librarySpecials ?? instanceSpecials ?? false;
}

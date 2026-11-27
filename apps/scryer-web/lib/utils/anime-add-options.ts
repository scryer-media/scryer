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

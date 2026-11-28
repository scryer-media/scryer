import assert from "node:assert/strict";
import test from "node:test";

import type { SameFilmPairing } from "../types/titles.ts";
import {
  filmTmdbIdForAdd,
  movieTitlePlacesByLink,
  seriesMoviePlaces,
} from "./same-film.ts";

const pairing = (
  overrides: Partial<SameFilmPairing> = {},
): SameFilmPairing => ({
  tmdbId: "880101",
  seriesMovieLinkId: "link-1",
  seriesTitleId: "show-1",
  seriesTitleName: "Fixture Show",
  seriesTitleFacet: "ANIME",
  movieTitleId: "movie-1",
  movieTitleName: "Fixture Film",
  movieTitleFacet: "MOVIE",
  ...overrides,
});

test("a movie links to each show that holds it, opening the series movie", () => {
  const places = seriesMoviePlaces([pairing(), pairing()]);
  assert.equal(places.length, 1);
  assert.equal(places[0].name, "Fixture Show");
  assert.match(places[0].href ?? "", /\?id=show-1&seriesMovie=link-1$/);
});

test("a show links each series movie to the movie titles of the same film", () => {
  const byLink = movieTitlePlacesByLink(
    [
      pairing(),
      pairing({ movieTitleId: "movie-2", movieTitleName: "Fixture Film 2" }),
      pairing({ seriesMovieLinkId: "link-2", seriesTitleId: "show-2" }),
      pairing({ seriesMovieLinkId: "link-3", movieTitleId: null }),
    ],
    "show-1",
  );
  assert.deepEqual([...byLink.keys()], ["link-1"]);
  assert.deepEqual(
    byLink.get("link-1")?.map((place) => place.name),
    ["Fixture Film", "Fixture Film 2"],
  );
  assert.match(byLink.get("link-1")?.[0].href ?? "", /\?id=movie-1$/);
});

test("no pairings give no places", () => {
  assert.deepEqual(seriesMoviePlaces(undefined), []);
  assert.equal(movieTitlePlacesByLink(null, "show-1").size, 0);
});

test("only a film being added is looked up by its TMDB movie id", () => {
  assert.equal(filmTmdbIdForAdd({ tmdbId: 880101 }, "MOVIE"), "880101");
  // A series' TMDB id numbers a show, not a film.
  assert.equal(filmTmdbIdForAdd({ tmdbId: 880101 }, "SERIES"), null);
  assert.equal(
    filmTmdbIdForAdd(
      {
        tmdbId: 990101,
        externalIds: [{ source: "tmdb", kind: "movie", value: " 880101 " }],
      },
      "ANIME",
    ),
    "880101",
  );
});

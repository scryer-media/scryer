import assert from "node:assert/strict";
import test from "node:test";

import {
  externalRatingLabelForAliases,
  ratingSourceInfo,
  ratingValueLabel,
  topOrderedRatingSource,
  visibleTitleExternalRatings,
  type TitleExternalRating,
} from "./title-ratings.ts";

function rating(source: string): TitleExternalRating {
  return {
    source,
    value: 7.5,
    score: null,
    normalized: 7.5,
    votes: null,
    url: "",
  };
}

function ratingWith(
  source: string,
  value: number,
  score: number,
  normalized: number,
): TitleExternalRating {
  return {
    source,
    value,
    score,
    normalized,
    votes: null,
    url: "",
  };
}

test("MDB Rotten Tomatoes sources render as percentages with logos", () => {
  for (const source of ["rottentomatoes", "tomatoes"]) {
    const info = ratingSourceInfo(source);
    assert.equal(info.logoSrc, "/rating-sources/rotten-tomatoes.svg");
    assert.equal(info.format, "percent");
    assert.equal(
      ratingValueLabel(
        {
          source,
          value: 94,
          score: null,
          normalized: 9.4,
          votes: null,
          url: "",
        },
        info,
      ),
      "94%",
    );
  }
});

test("MDB Rotten Tomatoes audience sources use the Popcornmeter logo", () => {
  for (const source of ["audience", "popcorn", "popcornmeter"]) {
    const info = ratingSourceInfo(source);
    assert.equal(info.logoSrc, "/rating-sources/popcornmeter.svg");
    assert.equal(info.format, "percent");
    assert.equal(
      ratingValueLabel(
        {
          source,
          value: 94,
          score: null,
          normalized: 9.4,
          votes: null,
          url: "",
        },
        info,
      ),
      "94%",
    );
  }
});

test("MDB normalized-only percentage ratings expand from a ten-point scale", () => {
  assert.equal(
    ratingValueLabel({
      source: "tomatoes",
      value: null,
      score: null,
      normalized: 8.7,
      votes: null,
      url: "",
    }),
    "87%",
  );
});

test("Metacritic ratings render on a hundred-point scale", () => {
  const info = ratingSourceInfo("metacritic");
  assert.equal(info.logoSrc, "/rating-sources/metacritic.svg");
  assert.equal(
    ratingValueLabel({
      source: "metacritic",
      value: 7.5,
      score: null,
      normalized: 7.5,
      votes: null,
      url: "",
    }),
    "75",
  );
});

test("Metacritic user ratings reuse the Metacritic logo", () => {
  for (const source of ["MC User", "metacritic-user"]) {
    const info = ratingSourceInfo(source);
    assert.equal(info.label, "Metacritic User");
    assert.equal(info.logoSrc, "/rating-sources/metacritic.svg");
    assert.equal(info.format, "hundred");
  }
});

test("visible external ratings keep Rotten Tomatoes and Popcornmeter adjacent", () => {
  assert.deepEqual(
    visibleTitleExternalRatings([
      rating("tmdb"),
      rating("popcornmeter"),
      rating("metacritic"),
      rating("rottentomatoes"),
      rating("imdb"),
    ]).map((entry) => entry.source),
    ["imdb", "rottentomatoes", "popcornmeter", "metacritic", "tmdb"],
  );
});

test("visible external ratings drop Roger Ebert sources", () => {
  assert.deepEqual(
    visibleTitleExternalRatings([
      rating("IMDb"),
      rating("Roger Ebert"),
      rating("RogerEbert.com"),
      rating("ebert"),
    ]).map((entry) => entry.source),
    ["IMDb"],
  );
});

test("visible external ratings collapse duplicate source aliases", () => {
  assert.deepEqual(
    visibleTitleExternalRatings([
      rating("mal"),
      rating("my-anime-list"),
      rating("tmdb"),
    ]).map((entry) => entry.source),
    ["tmdb", "mal"],
  );
});

test("external-only ratings resolve table column aliases", () => {
  const ratings: TitleExternalRating[] = [
    ratingWith("mdblist", 86, 86, 8.6),
    ratingWith("imdb", 8.1, 81, 8.1),
    ratingWith("tomatoes", 95, 95, 9.5),
    ratingWith("popcorn", 90, 90, 9),
    ratingWith("metacriticuser", 8, 80, 8),
    ratingWith("tmdb", 79, 79, 7.9),
    ratingWith("trakt", 80, 80, 8),
  ];

  assert.equal(externalRatingLabelForAliases(ratings, ["imdb"]), "8.1");
  assert.equal(externalRatingLabelForAliases(ratings, ["tomatoes"]), "95%");
  assert.equal(externalRatingLabelForAliases(ratings, ["popcorn"]), "90%");
  assert.equal(
    externalRatingLabelForAliases(ratings, ["metacriticuser", "mcuser"]),
    "80",
  );
  assert.equal(externalRatingLabelForAliases(ratings, ["tmdb"]), "79");
  assert.equal(externalRatingLabelForAliases(ratings, ["trakt"]), "80");
  assert.equal(externalRatingLabelForAliases(ratings, ["mdblist"]), "86");
});

test("MAL rating pill reuses the existing media-site logo", () => {
  for (const source of [
    "mal",
    "myanimelist",
    "my-anime-list",
    "myanimelist.net",
  ]) {
    const info = ratingSourceInfo(source);
    assert.equal(info.label, "MyAnimeList");
    assert.equal(info.logoSrc, "/media-sites/mal.svg");
  }
});

test("TVDB rating pill reuses the existing media-site logo", () => {
  for (const source of ["tvdb", "thetvdb", "the-tvdb"]) {
    const info = ratingSourceInfo(source);
    assert.equal(info.label, "TVDB");
    assert.equal(info.logoSrc, "/media-sites/tvdb.svg");
  }
});

test("top ordered rating source respects RATING_SOURCE_ORDER priority", () => {
  assert.equal(topOrderedRatingSource(["trakt", "tmdb", "imdb"]), "imdb");
  assert.equal(topOrderedRatingSource(["tmdb", "trakt"]), "tmdb");
});

test("top ordered rating source ranks anime sources by known priority", () => {
  assert.equal(topOrderedRatingSource(["anidb", "anilist", "mal"]), "mal");
  assert.equal(topOrderedRatingSource(["anidb", "anilist"]), "anilist");
});

test("top ordered rating source ignores blank entries", () => {
  assert.equal(topOrderedRatingSource(["", "   ", "tmdb"]), "tmdb");
});

test("top ordered rating source returns the sole attributed source", () => {
  assert.equal(topOrderedRatingSource(["trakt"]), "trakt");
});

test("top ordered rating source returns null without attribution", () => {
  assert.equal(topOrderedRatingSource([]), null);
  assert.equal(topOrderedRatingSource(["", "  "]), null);
});

test("top ordered rating source keeps first unknown source when none are ranked", () => {
  assert.equal(topOrderedRatingSource(["custom-a", "custom-b"]), "custom-a");
});

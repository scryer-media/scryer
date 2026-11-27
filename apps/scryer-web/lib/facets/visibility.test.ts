import assert from "node:assert/strict";
import test from "node:test";

import { discoveryItemFacet } from "../utils/discovery-actions.ts";
import {
  combineHiddenFacets,
  filterVisibleFacets,
  hiddenFacetsFromUiSettings,
  hiddenFacetsToUiSettings,
  isFacetVisible,
  NO_HIDDEN_FACETS,
  visibleFacetOr,
  visibleFacets,
  visibleFacetValues,
} from "./visibility.ts";

const ANIME_HIDDEN = hiddenFacetsFromUiSettings(["ANIME"]);

test("saved hidden facets drop unknown values and never hide every facet", () => {
  assert.deepEqual(
    [...hiddenFacetsFromUiSettings(["ANIME", "CARTOONS", "ANIME"])],
    ["ANIME"],
  );
  assert.equal(
    hiddenFacetsFromUiSettings(["MOVIES", "SERIES", "ANIME"]).size,
    0,
  );
  assert.deepEqual(
    hiddenFacetsToUiSettings(hiddenFacetsFromUiSettings(["ANIME", "MOVIES"])),
    ["MOVIES", "ANIME"],
  );
});

test("visible facets keep registry order and accept every facet spelling", () => {
  assert.deepEqual(
    visibleFacets(ANIME_HIDDEN).map((facet) => facet.id),
    ["MOVIE", "SERIES"],
  );
  for (const spelling of ["ANIME", "anime", " Anime "]) {
    assert.equal(isFacetVisible(spelling, ANIME_HIDDEN), false, spelling);
  }
  for (const spelling of ["MOVIE", "MOVIES", "movies", "movie", "series"]) {
    assert.equal(isFacetVisible(spelling, ANIME_HIDDEN), true, spelling);
  }
  assert.equal(isFacetVisible(null, ANIME_HIDDEN), true);
  assert.equal(isFacetVisible("music", ANIME_HIDDEN), true);
});

test("filters keep facet-less items and values a caller must still show", () => {
  const rows = [
    { id: 1, facet: "ANIME" },
    { id: 2, facet: "MOVIE" },
    { id: 3, facet: null },
  ];
  assert.deepEqual(
    filterVisibleFacets(rows, (row) => row.facet, ANIME_HIDDEN).map(
      (row) => row.id,
    ),
    [2, 3],
  );
  assert.deepEqual(
    visibleFacetValues(["movie", "series", "anime"], ANIME_HIDDEN),
    ["movie", "series"],
  );
  assert.deepEqual(
    visibleFacetValues(["movie", "series", "anime"], ANIME_HIDDEN, ["ANIME"]),
    ["movie", "series", "anime"],
  );
});

test("a hidden facet falls back to the first visible facet", () => {
  assert.equal(visibleFacetOr("ANIME", ANIME_HIDDEN), "MOVIE");
  assert.equal(visibleFacetOr("SERIES", ANIME_HIDDEN), "SERIES");
  assert.equal(
    visibleFacetOr("MOVIE", hiddenFacetsFromUiSettings(["MOVIES"])),
    "SERIES",
  );
});

test("an anime film is a Movie while Anime is hidden and Anime otherwise", () => {
  const spiritedAwayShaped = { contentType: "anime", targetKind: "movie" };
  assert.equal(discoveryItemFacet(spiritedAwayShaped), "ANIME");
  assert.equal(
    discoveryItemFacet(spiritedAwayShaped, NO_HIDDEN_FACETS),
    "ANIME",
  );
  assert.equal(discoveryItemFacet(spiritedAwayShaped, ANIME_HIDDEN), "MOVIE");
});

test("an anime series stays Anime, and is filtered out, while Anime is hidden", () => {
  const animeSeries = { contentType: "anime", targetKind: "series" };
  const facet = discoveryItemFacet(animeSeries, ANIME_HIDDEN);
  assert.equal(facet, "ANIME");
  assert.equal(isFacetVisible(facet, ANIME_HIDDEN), false);
  assert.equal(
    discoveryItemFacet(
      { contentType: "movie", targetKind: "movie" },
      ANIME_HIDDEN,
    ),
    "MOVIE",
  );
});

test("instance-disabled facets add to the user's hidden ones without hiding all", () => {
  assert.equal(combineHiddenFacets(ANIME_HIDDEN, []), ANIME_HIDDEN);
  assert.deepEqual(
    [...combineHiddenFacets(NO_HIDDEN_FACETS, ["ANIME", "bogus"])],
    ["ANIME"],
  );
  assert.deepEqual(
    [
      ...combineHiddenFacets(hiddenFacetsFromUiSettings(["MOVIES"]), ["ANIME"]),
    ].sort(),
    ["ANIME", "MOVIE"],
  );
  // The user hid the two facets the instance left on: only the instance's stand.
  assert.deepEqual(
    [
      ...combineHiddenFacets(hiddenFacetsFromUiSettings(["MOVIES", "SERIES"]), [
        "ANIME",
      ]),
    ],
    ["ANIME"],
  );
  // A stored list that turns every facet off turns none off.
  assert.equal(
    combineHiddenFacets(ANIME_HIDDEN, ["MOVIE", "SERIES", "ANIME"]),
    ANIME_HIDDEN,
  );
});

import assert from "node:assert/strict";
import test from "node:test";

import { FACET_IDS } from "./registry.ts";
import {
  facetSelectOptions,
  isFacetSelected,
  LOWERCASE_FACET_IDS,
  toggleFacetValue,
} from "./selection.ts";

test("options resolve in either stored spelling", () => {
  assert.deepEqual(
    facetSelectOptions(LOWERCASE_FACET_IDS).map((option) => [
      option.value,
      option.facet.id,
    ]),
    [
      ["movie", "MOVIE"],
      ["series", "SERIES"],
      ["anime", "ANIME"],
    ],
  );
  assert.deepEqual(
    facetSelectOptions(FACET_IDS).map((option) => option.facet.id),
    ["MOVIE", "SERIES", "ANIME"],
  );
});

test("unknown values are dropped rather than rendered", () => {
  assert.deepEqual(
    facetSelectOptions(["movie", "book"]).map((option) => option.value),
    ["movie"],
  );
});

test("selection matching ignores the stored spelling", () => {
  assert.equal(isFacetSelected(["MOVIE"], "movie"), true);
  assert.equal(isFacetSelected(["movie"], "MOVIE"), true);
  assert.equal(isFacetSelected(["series"], "movie"), false);
  assert.equal(isFacetSelected([], "movie"), false);
});

test("toggling keeps the caller's spelling and never duplicates", () => {
  assert.deepEqual(toggleFacetValue([], "movie", true), ["movie"]);
  assert.deepEqual(toggleFacetValue(["movie"], "series", true), [
    "movie",
    "series",
  ]);
  assert.deepEqual(toggleFacetValue(["movie", "series"], "movie", false), [
    "series",
  ]);
  // A value stored in the other spelling is replaced, not duplicated.
  assert.deepEqual(toggleFacetValue(["MOVIE"], "movie", true), ["movie"]);
  assert.deepEqual(toggleFacetValue(["MOVIE"], "movie", false), []);
});

import test from "node:test";
import assert from "node:assert/strict";

import { capsCategoryScopes, overlayCapsCategories } from "./indexer-caps-categories.ts";

test("numeric ranges classify standard newznab trees", () => {
  assert.deepEqual(capsCategoryScopes({ code: "2045", label: null }), ["MOVIE"]);
  assert.deepEqual(capsCategoryScopes({ code: "5040", label: null }), ["SERIES", "ANIME"]);
  assert.deepEqual(capsCategoryScopes({ code: "3010", label: "Audio/MP3" }), []);
  assert.deepEqual(capsCategoryScopes({ code: "7020", label: "Books/Comics" }), []);
});

test("names win over numbers and custom ids are offered everywhere", () => {
  assert.deepEqual(capsCategoryScopes({ code: "100020", label: "Movies-DE" }), ["MOVIE"]);
  assert.deepEqual(capsCategoryScopes({ code: "100030", label: "TV/Anime" }), [
    "ANIME",
    "SERIES",
  ]);
  assert.deepEqual(capsCategoryScopes({ code: "100030", label: "TV-DE" }), [
    "SERIES",
    "ANIME",
  ]);
  assert.deepEqual(capsCategoryScopes({ code: "100040", label: "Serien" }), [
    "MOVIE",
    "SERIES",
    "ANIME",
  ]);
  assert.deepEqual(capsCategoryScopes({ code: "abc", label: null }), [
    "MOVIE",
    "SERIES",
    "ANIME",
  ]);
  assert.deepEqual(capsCategoryScopes({ code: "5070", label: "TV/Anime" }), [
    "ANIME",
    "SERIES",
  ]);
});

test("overlay splits known labels from extra categories per scope", () => {
  const known = new Set(["2000", "2040", "5000", "5040"]);
  const categories = [
    { code: "2000", label: "Movies" },
    { code: "2040", label: "Movies/HD" },
    { code: "2040", label: "duplicate" },
    { code: "100020", label: "Movies-DE" },
    { code: "5040", label: " TV/HD " },
    { code: "100030", label: "TV-DE" },
    { code: "3000", label: "Audio" },
    { code: "150000", label: null },
    { code: "  ", label: "blank" },
  ];

  const movie = overlayCapsCategories("MOVIE", categories, known);
  assert.deepEqual(Array.from(movie.labelsByCode.entries()), [
    ["2000", "Movies"],
    ["2040", "Movies/HD"],
    ["5040", "TV/HD"],
  ]);
  assert.deepEqual(movie.extraCategories, [
    { code: "100020", label: "Movies-DE" },
    { code: "150000", label: null },
  ]);

  const series = overlayCapsCategories("SERIES", categories, known);
  assert.deepEqual(series.extraCategories, [
    { code: "100030", label: "TV-DE" },
    { code: "150000", label: null },
  ]);
});

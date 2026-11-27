import assert from "node:assert/strict";
import test from "node:test";

import {
  advancedSpecialsDefault,
  animeAddTitleOptions,
} from "./anime-add-options.ts";

test("an anime add without choices leaves the anime settings in charge", () => {
  assert.deepEqual(animeAddTitleOptions("ANIME", {}), {});
});

test("an anime add sends only the choices the user made", () => {
  assert.deepEqual(animeAddTitleOptions("ANIME", { monitorSpecials: false }), {
    monitorSpecials: false,
  });
  assert.deepEqual(
    animeAddTitleOptions("ANIME", {
      monitorSpecials: true,
      interSeasonMovies: false,
    }),
    { monitorSpecials: true, interSeasonMovies: false },
  );
});

test("other facets never send anime options", () => {
  assert.deepEqual(
    animeAddTitleOptions("SERIES", { monitorSpecials: true }),
    {},
  );
  assert.deepEqual(
    animeAddTitleOptions("MOVIE", { interSeasonMovies: true }),
    {},
  );
});

test("Advanced seeds specials from the chosen library's resolved setting", () => {
  const base = { facet: "ANIME" as const, libraryId: "library-anime" };
  assert.equal(
    advancedSpecialsDefault({
      ...base,
      librarySpecials: true,
      instanceSpecials: false,
    }),
    true,
  );
  assert.equal(
    advancedSpecialsDefault({
      ...base,
      librarySpecials: false,
      instanceSpecials: true,
    }),
    false,
  );
});

test("Advanced waits until the library's specials setting is known", () => {
  assert.equal(
    advancedSpecialsDefault({
      facet: "ANIME",
      libraryId: "library-anime",
      librarySpecials: undefined,
      instanceSpecials: true,
    }),
    null,
  );
});

test("an unreadable library setting falls back to the instance's", () => {
  assert.equal(
    advancedSpecialsDefault({
      facet: "ANIME",
      libraryId: "library-anime",
      librarySpecials: null,
      instanceSpecials: true,
    }),
    true,
  );
  assert.equal(
    advancedSpecialsDefault({
      facet: "ANIME",
      libraryId: "library-anime",
      librarySpecials: null,
      instanceSpecials: null,
    }),
    false,
  );
});

test("specials are not seeded outside anime", () => {
  assert.equal(
    advancedSpecialsDefault({
      facet: "SERIES",
      libraryId: "library-series",
      librarySpecials: undefined,
      instanceSpecials: true,
    }),
    false,
  );
});

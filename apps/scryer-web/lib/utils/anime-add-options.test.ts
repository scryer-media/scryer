import assert from "node:assert/strict";
import test from "node:test";

import { animeAddTitleOptions } from "./anime-add-options.ts";

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

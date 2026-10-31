import assert from "node:assert/strict";
import test from "node:test";

import {
  buildTvdbMovieUrl,
  buildTvdbSeriesUrl,
} from "./external-media-links.ts";

test("TVDB movie links use slug URLs when available", () => {
  assert.equal(
    buildTvdbMovieUrl("123", "glass-harbor"),
    "https://thetvdb.com/movies/glass-harbor",
  );
});

test("TVDB movie fallback uses dereferrer movie URLs", () => {
  assert.equal(
    buildTvdbMovieUrl("123"),
    "https://thetvdb.com/dereferrer/movie/123",
  );
});

test("TVDB series links use slug URLs when available", () => {
  assert.equal(
    buildTvdbSeriesUrl("bluey", "353546"),
    "https://thetvdb.com/series/bluey",
  );
  assert.equal(
    buildTvdbSeriesUrl("bluey specials", "353546"),
    "https://thetvdb.com/series/bluey%20specials",
  );
});

test("TVDB series fallback uses dereferrer series URLs", () => {
  assert.equal(
    buildTvdbSeriesUrl(null, "353546"),
    "https://thetvdb.com/dereferrer/series/353546",
  );
});

test("a series without a TVDB id gets no TVDB link, even with a slug", () => {
  assert.equal(buildTvdbSeriesUrl("tmdb-primary-series", null), null);
  assert.equal(buildTvdbSeriesUrl("tmdb-primary-series", " "), null);
});

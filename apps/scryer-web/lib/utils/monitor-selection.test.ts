import assert from "node:assert/strict";
import test from "node:test";

import {
  isMonitorSelectionEmpty,
  monitorSelectionFromRecord,
  monitorSelectionInput,
  monitorSelectionMovieKey,
  monitorSelectionSummaryParts,
  normalizeMonitorSelection,
} from "./monitor-selection.ts";

test("movie keys follow the domain's source precedence", () => {
  assert.equal(
    monitorSelectionMovieKey({
      name: "Bridge Movie",
      externalIds: [
        { source: "MAL", value: "40456" },
        { source: "tmdb", value: "438759" },
        { source: "tvdb", value: "131963" },
      ],
    }),
    "tvdb:131963",
  );
  assert.equal(
    monitorSelectionMovieKey({
      name: "No ids",
      externalIds: [{ source: "anilist", value: "1" }],
    }),
    null,
  );
});

test("normalizing sorts seasons, drops keyless movies, and dedupes", () => {
  const normalized = normalizeMonitorSelection({
    seasonNumbers: [3, 0, 1, 3],
    seriesMovies: [
      { name: "Keyless", externalIds: [] },
      { name: "Second", externalIds: [{ source: "tvdb", value: "2" }] },
      { name: "First", externalIds: [{ source: "tvdb", value: "1" }] },
      { name: "Second again", externalIds: [{ source: "tvdb", value: "2" }] },
    ],
  });
  assert.deepEqual(normalized.seasonNumbers, [0, 1, 3]);
  assert.deepEqual(
    normalized.seriesMovies.map((movie) => movie.name),
    ["First", "Second again"],
  );
});

test("empty selections never reach the API", () => {
  assert.equal(isMonitorSelectionEmpty(null), true);
  assert.equal(
    isMonitorSelectionEmpty({
      seasonNumbers: [],
      seriesMovies: [{ name: "Keyless", externalIds: [] }],
    }),
    true,
  );
  assert.equal(monitorSelectionInput({ seasonNumbers: [], seriesMovies: [] }), undefined);
  assert.deepEqual(monitorSelectionInput({ seasonNumbers: [2, 1], seriesMovies: [] }), {
    seasonNumbers: [1, 2],
    seriesMovies: [],
  });
  assert.equal(monitorSelectionFromRecord({ seasonNumbers: [], seriesMovies: [] }), null);
});

test("the card summary renders specials and movie names", () => {
  const parts = monitorSelectionSummaryParts(
    {
      seasonNumbers: [1, 0],
      seriesMovies: [
        { name: "Iron Rail", externalIds: [{ source: "tvdb", value: "131963" }] },
      ],
    },
    {
      specials: "Specials",
      season: (seasonNumber) => `Season ${seasonNumber}`,
    },
  );
  assert.deepEqual(parts.seasons, ["Season 1", "Specials"]);
  assert.deepEqual(parts.movies, ["Iron Rail"]);
});

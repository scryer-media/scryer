import assert from "node:assert/strict";
import test from "node:test";

import {
  resolveQueuedSeasonSelections,
  seasonHasSelectableEpisodes,
} from "./season-selection.ts";

const episodes = (...ids: string[]) => ids.map((id) => ({ id }));

test("a queued season waits until its episodes load", () => {
  assert.equal(
    resolveQueuedSeasonSelections(new Set(["season-1"]), {}, undefined),
    null,
  );
});

test("a loaded queued season selects its episodes and leaves the queue", () => {
  const resolved = resolveQueuedSeasonSelections(
    new Set(["season-1", "season-2"]),
    { "season-1": episodes("ep-1", "ep-2") },
    undefined,
  );

  assert.deepEqual(resolved?.episodeIds, ["ep-1", "ep-2"]);
  assert.deepEqual([...(resolved?.remaining ?? [])], ["season-2"]);
});

test("episodes locked by an in-flight deletion stay out of a queued selection", () => {
  const resolved = resolveQueuedSeasonSelections(
    new Set(["season-1"]),
    { "season-1": episodes("ep-1", "ep-2", "ep-3") },
    new Set(["ep-2"]),
  );

  assert.deepEqual(resolved?.episodeIds, ["ep-1", "ep-3"]);
  assert.equal(resolved?.remaining.size, 0);
});

test("a season that loaded empty clears from the queue with nothing selected", () => {
  const resolved = resolveQueuedSeasonSelections(
    new Set(["season-1"]),
    { "season-1": [] },
    undefined,
  );

  assert.deepEqual(resolved, { episodeIds: [], remaining: new Set() });
});

test("an unloaded season is selectable unless its episode total is zero", () => {
  assert.equal(
    seasonHasSelectableEpisodes({
      episodesReady: false,
      selectableEpisodeCount: 0,
      episodeRecordsTotal: 10,
    }),
    true,
  );
  assert.equal(
    seasonHasSelectableEpisodes({
      episodesReady: false,
      selectableEpisodeCount: 0,
      episodeRecordsTotal: null,
    }),
    true,
  );
  assert.equal(
    seasonHasSelectableEpisodes({
      episodesReady: false,
      selectableEpisodeCount: 0,
      episodeRecordsTotal: 0,
    }),
    false,
  );
});

test("a loaded season is selectable only while an unlocked episode remains", () => {
  assert.equal(
    seasonHasSelectableEpisodes({
      episodesReady: true,
      selectableEpisodeCount: 0,
      episodeRecordsTotal: 10,
    }),
    false,
  );
  assert.equal(
    seasonHasSelectableEpisodes({
      episodesReady: true,
      selectableEpisodeCount: 2,
      episodeRecordsTotal: 10,
    }),
    true,
  );
});

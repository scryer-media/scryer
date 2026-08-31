import assert from "node:assert/strict";
import test from "node:test";

import {
  compareHistoryEpisodes,
  formatHistoryEpisodeLabel,
  type HistoryEpisodeDisplay,
} from "./history-episodes.ts";

function episode(
  id: string,
  seasonNumber: number,
  episodeNumber: number,
  title: string,
): HistoryEpisodeDisplay {
  return {
    id,
    seasonNumber,
    episodeNumber,
    episodeLabel: title,
    title,
  };
}

test("history episode labels use SxxEyy and show the title once", () => {
  const item = episode("episode-1", 2, 3, "The Substitute");

  assert.equal(formatHistoryEpisodeLabel(item, item.id), "S02E03 · The Substitute");
});

test("history episodes sort numerically by season then episode", () => {
  const items = [
    episode("s10e2", 10, 2, "Season Ten"),
    episode("s2e10", 2, 10, "Episode Ten"),
    episode("s2e1", 2, 1, "Episode One"),
  ];

  assert.deepEqual(items.sort(compareHistoryEpisodes).map((item) => item.id), [
    "s2e1",
    "s2e10",
    "s10e2",
  ]);
});

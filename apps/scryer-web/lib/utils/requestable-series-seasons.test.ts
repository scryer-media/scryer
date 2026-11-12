import assert from "node:assert/strict";
import test from "node:test";

import { requestableSeriesSeasons } from "./requestable-series-seasons.ts";

test("only offers known regular seasons that still need monitoring", () => {
  const season = {
    collectionType: "SEASON",
    collectionIndex: "2",
    label: null,
    monitored: true,
    episodeRecordsTotal: 6,
    episodesMonitored: 0,
  };

  assert.deepEqual(
    requestableSeriesSeasons([
      {
        ...season,
        collectionIndex: "5",
        monitored: false,
        episodesMonitored: 6,
      },
      {
        ...season,
        collectionIndex: "3",
        label: " New season ",
        episodesMonitored: 5,
      },
      { ...season, collectionIndex: "1", episodesMonitored: 6 },
      { ...season, collectionIndex: "0" },
      { ...season, collectionIndex: "7", episodeRecordsTotal: 0 },
      { ...season, collectionIndex: "8", collectionType: "SPECIALS" },
      { ...season, collectionIndex: "future" },
      season,
    ]),
    [
      [2, ""],
      [3, "New season"],
      [5, ""],
    ],
  );
});

import assert from "node:assert/strict";
import test from "node:test";

import {
  countEpisodeProgress,
  episodeProgressCounts,
  episodeProgressPercent,
  episodeProgressText,
  utcCalendarDay,
} from "./episode-progress.ts";

test("progress is episodes on disk over episodes expected", () => {
  const counts = episodeProgressCounts({
    episodesOnDisk: 3,
    episodesExpected: 6,
    episodeRecordsTotal: 11,
  });
  assert.deepEqual(counts, { onDisk: 3, expected: 6, total: 11 });
  assert.equal(episodeProgressPercent(counts!), 50);
  assert.equal(episodeProgressText(counts!), "3 / 6");
});

test("a series with nothing expected shows complete", () => {
  const counts = episodeProgressCounts({
    episodesOnDisk: 0,
    episodesExpected: 0,
    episodeRecordsTotal: 4,
  });
  assert.equal(episodeProgressPercent(counts!), 100);
  assert.equal(episodeProgressText(counts!), "0 / 0");
});

test("no reported counts means no progress", () => {
  assert.equal(episodeProgressCounts({ episodesOnDisk: 2 }), null);
});

test("loaded episodes count the way the server does", () => {
  const counts = countEpisodeProgress(
    [
      // Monitored and aired, no file: expected, missing.
      { monitored: true, airDate: "2026-04-30", hasFile: false },
      // Monitored, airs today: expected.
      { monitored: true, airDate: "2026-05-01T20:00:00Z", hasFile: false },
      // Monitored, not aired yet: not expected.
      { monitored: true, airDate: "2026-05-02", hasFile: false },
      // Unmonitored with a file: counted.
      { monitored: false, airDate: "2026-01-01", hasFile: true },
      // Undated with a file: counted.
      { monitored: true, airDate: null, hasFile: true },
      // Unmonitored, aired, no file: not expected.
      { monitored: false, airDate: "2026-01-02", hasFile: false },
      // Monitored with only a year or a month: not aired yet.
      { monitored: true, airDate: "2026", hasFile: false },
      { monitored: true, airDate: "2026-04", hasFile: false },
    ],
    "2026-05-01",
  );
  assert.deepEqual(counts, { onDisk: 2, expected: 4, total: 8 });
});

test("the calendar day is the UTC date", () => {
  // 23:30 at UTC-7 on Jan 5 is already Jan 6 in UTC.
  assert.equal(
    utcCalendarDay(new Date("2026-01-05T23:30:00-07:00")),
    "2026-01-06",
  );
  assert.equal(
    utcCalendarDay(new Date("2026-01-05T23:30:00Z")),
    "2026-01-05",
  );
});

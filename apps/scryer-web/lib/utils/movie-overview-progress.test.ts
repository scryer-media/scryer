import assert from "node:assert/strict";
import test from "node:test";
import { movieOverviewProgress } from "./movie-overview-progress.ts";

const copying = {
  state: "COMPLETED",
  displayState: "IMPORTING",
  importStatus: "RUNNING",
  progressPercent: 100,
  importTransferPhase: "COPYING",
  importTransferBytes: 9,
  importTransferTotalBytes: 100,
  importTransferStartedAt: "2026-11-01T00:00:00Z",
} as const;

test("movie import never falls back to the completed download during counter gaps", () => {
  const sequence = [
    copying,
    { ...copying, importTransferBytes: null, importTransferTotalBytes: null },
    { ...copying, importTransferBytes: 12 },
    { ...copying, importTransferPhase: null },
    { ...copying, importTransferBytes: 15 },
  ];
  let previous = movieOverviewProgress(copying);
  assert.deepEqual(
    sequence.map((item) => {
      const progress = movieOverviewProgress(item, previous);
      previous = progress;
      return progress.indeterminate ? null : progress.percent;
    }),
    [9, 9, 12, 12, 15],
  );
  assert.equal(
    movieOverviewProgress({
      ...copying,
      displayState: "COMPLETED",
      importTransferPhase: null,
    }).indeterminate,
    true,
  );
});

test("waiting and finalization do not reuse byte counters or downloader completion", () => {
  for (const phase of ["WAITING", "FINALIZING"] as const) {
    const progress = movieOverviewProgress({
      ...copying,
      importTransferPhase: phase,
      importTransferBytes: 100,
    });
    assert.equal(progress.indeterminate, true);
    assert.equal(progress.measuredTransfer, false);
  }
  for (const total of [null, 0, Number.NaN, Number.POSITIVE_INFINITY]) {
    assert.equal(
      movieOverviewProgress({ ...copying, importTransferTotalBytes: total })
        .indeterminate,
      true,
    );
  }
});

test("new phases and retries reset animation; updates within a phase retain it", () => {
  const copy = movieOverviewProgress(copying);
  assert.equal(
    copy.phaseKey,
    movieOverviewProgress({ ...copying, importTransferBytes: 40 }).phaseKey,
  );
  const verify = movieOverviewProgress({
    ...copying,
    importTransferPhase: "VERIFYING",
    importTransferBytes: 0,
  });
  assert.equal(verify.percent, 0);
  assert.notEqual(copy.phaseKey, verify.phaseKey);
  assert.notEqual(
    copy.phaseKey,
    movieOverviewProgress({
      ...copying,
      importTransferStartedAt: "2026-11-01T01:00:00Z",
    }).phaseKey,
  );
});

test("client progress and actual completion remain determinate", () => {
  const downloading = {
    ...copying,
    state: "DOWNLOADING",
    displayState: "DOWNLOADING",
    importStatus: null,
    importTransferPhase: null,
    progressPercent: 65,
  } as const;
  assert.equal(movieOverviewProgress(downloading).percent, 65);
  assert.equal(
    movieOverviewProgress({
      ...downloading,
      state: "VERIFYING",
      displayState: "POST_PROCESSING",
      progressPercent: 100,
    }).indeterminate,
    true,
  );
  const done = movieOverviewProgress({
    ...copying,
    displayState: "COMPLETED",
    importStatus: "COMPLETED",
    importTransferPhase: null,
  });
  assert.equal(done.percent, 100);
  assert.equal(done.indeterminate, false);
});

test("counter gaps preserve percentage, byte label and animation identity", () => {
  const previous = movieOverviewProgress(copying);
  const gap = movieOverviewProgress(
    {
      ...copying,
      importTransferPhase: null,
      importTransferStartedAt: null,
      importTransferBytes: null,
      importTransferTotalBytes: null,
    },
    previous,
  );
  assert.strictEqual(gap, previous);
  assert.strictEqual(
    movieOverviewProgress({ ...copying, importTransferBytes: null }, gap),
    previous,
  );
  assert.equal(
    movieOverviewProgress({ ...copying, importTransferBytes: 20 }, gap).percent,
    20,
  );
  for (const item of [
    { ...copying, importTransferPhase: "VERIFYING", importTransferBytes: null },
    {
      ...copying,
      importTransferStartedAt: "2026-11-01T01:00:00Z",
      importTransferBytes: null,
    },
  ] as const) {
    const reset = movieOverviewProgress(item, gap);
    assert.equal(reset.indeterminate, true);
    assert.notEqual(reset.phaseKey, gap.phaseKey);
  }
});

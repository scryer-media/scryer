import assert from "node:assert/strict";
import test from "node:test";

import {
  judgeTrackedJobRuns,
  trackedJobRun,
  TRACKED_RUN_FAILURES_BEFORE_RELEASE,
  TRACKED_RUN_MISSES_BEFORE_RELEASE,
  type TrackedJobRun,
} from "./tracked-job-runs.ts";
import type { JobKey, JobRun, JobRunStatus } from "../types/jobs.ts";

function grabRun(id: string, status: JobRunStatus = "RUNNING"): JobRun {
  return {
    id,
    jobKey: "RELEASE_GRAB",
    customJobId: null,
    displayName: "Grab",
    category: "SYSTEM",
    section: "PRIMARY",
    status,
    triggerSource: "MANUAL",
    startedAt: "2026-01-01T00:00:00Z",
    completedAt: null,
    summaryJson: null,
    summaryText: null,
    errorText: null,
    progressJson: null,
    libraryScanProgress: null,
  } as JobRun;
}

function watching(...runs: JobRun[]): Map<string, TrackedJobRun> {
  return new Map(runs.map((run) => [run.id, trackedJobRun(run)]));
}

const listed = (runs: JobRun[]) =>
  new Map<JobKey, JobRun[]>([["RELEASE_GRAB", runs]]);

test("a run that has ended is reported and no longer watched", () => {
  const tracked = watching(grabRun("grab-1"));

  const { found, released } = judgeTrackedJobRuns(
    tracked,
    listed([grabRun("grab-1", "COMPLETED")]),
  );

  assert.deepEqual(
    found.map((run) => run.status),
    ["COMPLETED"],
  );
  assert.deepEqual(released, []);
  assert.equal(tracked.size, 0);
});

test("a run that stays absent from its listing is let go", () => {
  const tracked = watching(grabRun("grab-1"));
  const released: JobRun[] = [];

  for (let poll = 0; poll < TRACKED_RUN_MISSES_BEFORE_RELEASE; poll += 1) {
    assert.equal(tracked.size, 1, "still watched before the last miss");
    released.push(...judgeTrackedJobRuns(tracked, listed([])).released);
  }

  assert.deepEqual(
    released.map((run) => run.id),
    ["grab-1"],
  );
  assert.equal(tracked.size, 0);
});

test("seeing a run again starts its misses over", () => {
  const tracked = watching(grabRun("grab-1"));
  for (let poll = 1; poll < TRACKED_RUN_MISSES_BEFORE_RELEASE; poll += 1) {
    judgeTrackedJobRuns(tracked, listed([]));
  }

  judgeTrackedJobRuns(tracked, listed([grabRun("grab-1")]));
  const { released } = judgeTrackedJobRuns(tracked, listed([]));

  assert.deepEqual(released, []);
  assert.equal(tracked.get("grab-1")?.misses, 1);
});

test("a listing that keeps failing lets go of its runs only", () => {
  const grab = grabRun("grab-1");
  const search = {
    ...grabRun("search-1"),
    jobKey: "ACQUISITION_SEARCH" as const,
  };
  const tracked = watching(grab, search);
  const released: JobRun[] = [];

  // The grab listing loads every time and still shows the grab; the search
  // listing never loads.
  for (let poll = 0; poll < TRACKED_RUN_FAILURES_BEFORE_RELEASE; poll += 1) {
    assert.equal(tracked.has("search-1"), true, "still watched");
    released.push(...judgeTrackedJobRuns(tracked, listed([grab])).released);
  }

  assert.deepEqual(
    released.map((run) => run.id),
    ["search-1"],
  );
  assert.deepEqual([...tracked.keys()], ["grab-1"]);
  assert.equal(tracked.get("grab-1")?.misses, 0);
});

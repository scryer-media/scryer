import type { JobKey, JobRun } from "@/lib/types";
import { isTerminalJobRunStatus } from "./job-runs.ts";

/** Polls in a row that may miss a watched run before it is let go. */
export const TRACKED_RUN_MISSES_BEFORE_RELEASE = 5;

/** Polls in a row whose listing may fail to load before its runs are let go. */
export const TRACKED_RUN_FAILURES_BEFORE_RELEASE = 10;

export type TrackedJobRun = {
  /** The newest state seen for the run. */
  run: JobRun;
  /** Polls in a row whose listing of the run's job did not include it. */
  misses: number;
  /** Polls in a row whose listing of the run's job failed to load. */
  failures: number;
};

export function trackedJobRun(run: JobRun): TrackedJobRun {
  return { run, misses: 0, failures: 0 };
}

/**
 * Compare the watched runs with the runs the server listed for their jobs.
 *
 * `listedByJobKey` holds the jobs whose listing loaded; a job left out failed
 * to load. A listed run is returned in `found`, and stops being watched once
 * it has ended. A run is returned in `released`, and stops being watched,
 * when it is missing from its listing too many times in a row (it aged out of
 * the list, or the viewer cannot see it) or its listing fails to load too
 * many times in a row.
 */
export function judgeTrackedJobRuns(
  tracked: Map<string, TrackedJobRun>,
  listedByJobKey: ReadonlyMap<JobKey, readonly JobRun[]>,
): { found: JobRun[]; released: JobRun[] } {
  const found: JobRun[] = [];
  const released: JobRun[] = [];
  for (const [runId, entry] of [...tracked]) {
    const listed = listedByJobKey.get(entry.run.jobKey);
    if (!listed) {
      entry.failures += 1;
      if (entry.failures >= TRACKED_RUN_FAILURES_BEFORE_RELEASE) {
        tracked.delete(runId);
        released.push(entry.run);
      }
      continue;
    }
    entry.failures = 0;
    const run = listed.find((candidate) => candidate.id === runId);
    if (run) {
      entry.misses = 0;
      found.push(run);
      if (isTerminalJobRunStatus(run.status)) tracked.delete(runId);
      continue;
    }
    entry.misses += 1;
    if (entry.misses >= TRACKED_RUN_MISSES_BEFORE_RELEASE) {
      tracked.delete(runId);
      released.push(entry.run);
    }
  }
  return { found, released };
}

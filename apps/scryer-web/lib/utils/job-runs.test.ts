import assert from "node:assert/strict";
import test from "node:test";

import {
  jobInstanceKey,
  libraryDeletionFailureReason,
  libraryDeletionProgress,
  longRunningJobs,
  mergeLatestJobRun,
  normalizeJobRun,
  preferJobRunSnapshot,
  parseFullHashBackfillFailures,
  runAwaitingScriptOutput,
  scriptRunForJobRun,
  scriptRunIdOf,
} from "./job-runs.ts";
import type { JobRun } from "../types/jobs.ts";

test("source import retries replace terminal snapshots by durable revision", () => {
  const failed = jobRun({
    id: "source-run",
    jobKey: "SOURCE_IMPORT",
    status: "WARNING",
    progressJson: { revision: 5 },
    startedAt: "2026-01-01T00:00:00Z",
  });
  const retry = {
    ...failed,
    status: "RUNNING" as const,
    completedAt: null,
    progressJson: { revision: 6 },
  };
  assert.equal(preferJobRunSnapshot(failed, retry), retry);
  assert.equal(preferJobRunSnapshot(retry, failed), retry);
  const completed = {
    ...retry,
    status: "COMPLETED" as const,
    progressJson: { revision: 7 },
  };
  assert.equal(preferJobRunSnapshot(retry, completed), completed);
  assert.equal(preferJobRunSnapshot(completed, retry), completed);
});

test("full-hash backfill failures are read with their path and reason", () => {
  const parsed = parseFullHashBackfillFailures({
    jobKey: "FULL_HASH_BACKFILL",
    summaryJson: {
      failed: 2,
      failures: [
        {
          mediaFileId: "file-000",
          path: "/synthetic/a.mkv",
          reason: "file changed while hashing",
        },
        {
          mediaFileId: "file-001",
          path: "/synthetic/b.mkv",
          reason: "permission denied",
        },
      ],
      failuresTruncated: false,
    },
  });

  assert.deepEqual(parsed, {
    failures: [
      {
        mediaFileId: "file-000",
        path: "/synthetic/a.mkv",
        reason: "file changed while hashing",
      },
      {
        mediaFileId: "file-001",
        path: "/synthetic/b.mkv",
        reason: "permission denied",
      },
    ],
    notListed: 0,
  });
});

test("a truncated failure list reports how many were not listed", () => {
  const parsed = parseFullHashBackfillFailures({
    jobKey: "FULL_HASH_BACKFILL",
    summaryJson: JSON.stringify({
      failed: 150,
      failures: [
        {
          mediaFileId: "file-000",
          path: "/synthetic/a.mkv",
          reason: "read error",
        },
      ],
      failuresTruncated: true,
    }),
  });

  assert.equal(parsed.failures.length, 1);
  assert.equal(parsed.notListed, 149);
});

test("garbage, legacy, and other jobs' summaries yield no failures", () => {
  const empty = { failures: [], notListed: 0 };
  for (const summaryJson of [
    null,
    "not json",
    42,
    [],
    {},
    { failed: 3 },
    { failures: "nope" },
  ]) {
    assert.deepEqual(
      parseFullHashBackfillFailures({
        jobKey: "FULL_HASH_BACKFILL",
        summaryJson,
      }),
      empty,
    );
  }
  assert.deepEqual(
    parseFullHashBackfillFailures({
      jobKey: "FULL_HASH_BACKFILL",
      summaryJson: {
        failures: [
          null,
          { path: 1, reason: "x" },
          { path: "/synthetic/c.mkv" },
        ],
      },
    }),
    empty,
  );
  assert.deepEqual(
    parseFullHashBackfillFailures({
      jobKey: "HEALTH_CHECKS",
      summaryJson: { failures: [{ path: "/synthetic/a.mkv", reason: "x" }] },
    }),
    empty,
  );
});

function jobRun(
  overrides: Partial<JobRun> & Pick<JobRun, "id" | "jobKey" | "startedAt">,
): JobRun {
  return {
    customJobId: null,
    displayName: overrides.jobKey,
    category: "SYSTEM",
    section: "PRIMARY",
    status: "COMPLETED",
    triggerSource: "SCHEDULED_INTERVAL",
    completedAt: overrides.startedAt,
    summaryJson: null,
    summaryText: null,
    errorText: null,
    progressJson: null,
    libraryScanProgress: null,
    ...overrides,
  } as JobRun;
}

test("each job keeps its own latest run however often another job runs", () => {
  let latest: Partial<Record<string, JobRun>> = {};
  latest = mergeLatestJobRun(
    latest,
    jobRun({
      id: "housekeeping-1",
      jobKey: "HOUSEKEEPING",
      startedAt: "2026-01-01T00:00:00Z",
    }),
  );
  for (let minute = 10; minute < 60; minute += 1) {
    latest = mergeLatestJobRun(
      latest,
      jobRun({
        id: `rss-${minute}`,
        jobKey: "RSS_SYNC",
        startedAt: `2026-01-01T00:${minute}:00Z`,
      }),
    );
  }
  assert.equal(latest.HOUSEKEEPING?.id, "housekeeping-1");
  assert.equal(latest.RSS_SYNC?.id, "rss-59");
});

test("an older run never displaces a newer one, and a snapshot updates its own run", () => {
  const running = jobRun({
    id: "rss-2",
    jobKey: "RSS_SYNC",
    startedAt: "2026-01-01T00:02:00Z",
    status: "RUNNING",
    completedAt: null,
  });
  let latest = mergeLatestJobRun({}, running);
  const older = jobRun({
    id: "rss-1",
    jobKey: "RSS_SYNC",
    startedAt: "2026-01-01T00:01:00Z",
  });
  assert.equal(mergeLatestJobRun(latest, older), latest);
  latest = mergeLatestJobRun(latest, {
    ...running,
    status: "COMPLETED",
    completedAt: "2026-01-01T00:03:00Z",
  });
  assert.equal(latest.RSS_SYNC?.status, "COMPLETED");
});

test("built-in and user-defined jobs are tracked under keys that never collide", () => {
  assert.equal(
    jobInstanceKey({ key: "RSS_SYNC", customJobId: null }),
    "RSS_SYNC",
  );
  assert.equal(
    jobInstanceKey({ jobKey: "CUSTOM_JOB", customJobId: "script-a" }),
    "script-a",
  );
  assert.notEqual(
    jobInstanceKey({ jobKey: "CUSTOM_JOB", customJobId: "script-a" }),
    jobInstanceKey({ jobKey: "CUSTOM_JOB", customJobId: "script-b" }),
  );
});

test("each user-defined job keeps its own latest run alongside built-in jobs", () => {
  let latest: Partial<Record<string, JobRun>> = {};
  latest = mergeLatestJobRun(
    latest,
    jobRun({
      id: "rss-1",
      jobKey: "RSS_SYNC",
      startedAt: "2026-01-01T00:00:00Z",
    }),
  );
  latest = mergeLatestJobRun(
    latest,
    jobRun({
      id: "custom-a-1",
      jobKey: "CUSTOM_JOB",
      customJobId: "script-a",
      startedAt: "2026-01-01T00:01:00Z",
    }),
  );
  latest = mergeLatestJobRun(
    latest,
    jobRun({
      id: "custom-b-1",
      jobKey: "CUSTOM_JOB",
      customJobId: "script-b",
      startedAt: "2026-01-01T00:02:00Z",
    }),
  );

  assert.equal(latest.RSS_SYNC?.id, "rss-1");
  assert.equal(latest["script-a"]?.id, "custom-a-1");
  assert.equal(latest["script-b"]?.id, "custom-b-1");
  assert.equal(latest.CUSTOM_JOB, undefined);
});

test("a run's custom job id is read from the payload and absent ids become null", () => {
  const custom = normalizeJobRun({
    id: "run-1",
    jobKey: "CUSTOM_JOB",
    customJobId: "script-a",
    startedAt: "2026-01-01T00:00:00Z",
  });
  const builtIn = normalizeJobRun({ id: "run-2", jobKey: "RSS_SYNC" });

  assert.equal(custom?.customJobId, "script-a");
  assert.equal(builtIn?.customJobId, null);
});

test("a custom job run joins to its script run through the summary", () => {
  const scriptRuns = [{ id: "script-run-1" }, { id: "script-run-2" }];

  assert.equal(
    scriptRunIdOf({ summaryJson: { script_run_id: "script-run-2" } }),
    "script-run-2",
  );
  assert.equal(
    scriptRunIdOf({ summaryJson: '{"script_run_id":"script-run-1"}' }),
    "script-run-1",
  );
  assert.equal(scriptRunIdOf({ summaryJson: "not json" }), null);
  assert.equal(scriptRunIdOf({ summaryJson: null }), null);
  assert.equal(
    scriptRunForJobRun(
      { summaryJson: { script_run_id: "script-run-2" } },
      scriptRuns,
    )?.id,
    "script-run-2",
  );
  assert.equal(
    scriptRunForJobRun({ summaryJson: { script_run_id: "gone" } }, scriptRuns),
    null,
  );
});

test("the newest finished run waits until its script run has settled", () => {
  const older = jobRun({
    id: "run-old",
    jobKey: "CUSTOM_JOB",
    status: "COMPLETED",
    startedAt: "2026-01-01T00:00:00Z",
    summaryJson: { script_run_id: "script-run-old" },
  });
  const newer = jobRun({
    id: "run-new",
    jobKey: "CUSTOM_JOB",
    status: "COMPLETED",
    startedAt: "2026-01-01T00:05:00Z",
    summaryJson: { script_run_id: "script-run-new" },
  });
  const active = jobRun({
    id: "run-active",
    jobKey: "CUSTOM_JOB",
    status: "RUNNING",
    startedAt: "2026-01-01T00:09:00Z",
  });
  const settledOld = { id: "script-run-old", status: "success" };

  assert.equal(
    runAwaitingScriptOutput([older, newer, active], [settledOld]),
    "run-new",
  );
  assert.equal(
    runAwaitingScriptOutput(
      [older, newer],
      [settledOld, { id: "script-run-new", status: "running" }],
    ),
    "run-new",
  );
  assert.equal(
    runAwaitingScriptOutput(
      [older, newer],
      [settledOld, { id: "script-run-new", status: "failed" }],
    ),
    null,
  );
  assert.equal(runAwaitingScriptOutput([active], []), null);
});

test("long-running jobs count each active location operation once it passes the threshold", () => {
  const now = Date.parse("2026-01-01T00:10:00Z");
  const runs = [
    jobRun({
      id: "move",
      jobKey: "LOCATION_OPERATION",
      status: "RUNNING",
      startedAt: "2026-01-01T00:05:00Z",
      progressJson: { operationId: "op-1" },
    }),
    jobRun({
      id: "fresh-move",
      jobKey: "LOCATION_OPERATION",
      status: "DISCOVERING",
      startedAt: "2026-01-01T00:09:50Z",
      progressJson: { operationId: "op-2" },
    }),
    jobRun({
      id: "done",
      jobKey: "LOCATION_OPERATION",
      status: "COMPLETED",
      startedAt: "2026-01-01T00:00:00Z",
    }),
    jobRun({
      id: "queued",
      jobKey: "LOCATION_OPERATION",
      status: "QUEUED",
      startedAt: "2026-01-01T00:00:00Z",
    }),
  ];
  assert.deepEqual(longRunningJobs(runs, now), {
    count: 1,
    nextChangeAtMs: Date.parse("2026-01-01T00:10:20Z"),
  });
  assert.deepEqual(longRunningJobs(runs, now + 20_000), {
    count: 2,
    nextChangeAtMs: null,
  });
});

test("a folder conflict task counts once, not again for its current operation", () => {
  const now = Date.parse("2026-01-01T00:10:00Z");
  const runs = [
    jobRun({
      id: "task",
      jobKey: "LOCATION_OPERATION",
      status: "RUNNING",
      triggerSource: "MANUAL",
      startedAt: "2026-01-01T00:00:00Z",
      progressJson: { current_operation_id: "conflict:task:3" },
    }),
    jobRun({
      id: "child",
      jobKey: "LOCATION_OPERATION",
      status: "RUNNING",
      triggerSource: "MANUAL",
      startedAt: "2026-01-01T00:08:00Z",
      progressJson: { operationId: "conflict:task:3" },
    }),
  ];
  assert.equal(longRunningJobs(runs, now).count, 1);
});

test("jobs without a panel on Activity never count as long-running", () => {
  const now = Date.parse("2026-01-01T04:00:00Z");
  const runs = (
    [
      ["ARTWORK_ENCODING", "SCHEDULED_DAILY"],
      ["LIBRARY_SCAN_MOVIES", "MANUAL"],
      ["ACQUISITION_SEARCH", "SYSTEM_INTERNAL"],
    ] as const
  ).map(([jobKey, triggerSource]) =>
    jobRun({
      id: jobKey,
      jobKey,
      status: "RUNNING",
      triggerSource,
      startedAt: "2026-01-01T00:00:00Z",
    }),
  );
  assert.deepEqual(longRunningJobs(runs, now), {
    count: 0,
    nextChangeAtMs: null,
  });
});

test("library deletion runs report their pass while running and their reason when failed", () => {
  const running = jobRun({
    id: "library-delete",
    jobKey: "LIBRARY_DELETION",
    status: "RUNNING",
    startedAt: "2026-01-01T00:00:00Z",
    progressJson: JSON.stringify({
      phase: "deleting",
      processed: 64,
      total: 150,
    }),
  });
  assert.deepEqual(libraryDeletionProgress(running), {
    phase: "deleting",
    processed: 64,
    total: 150,
  });
  assert.equal(
    libraryDeletionProgress({ ...running, progressJson: { phase: "queued" } }),
    null,
  );
  assert.equal(libraryDeletionProgress({ ...running, status: "FAILED" }), null);
  assert.equal(
    libraryDeletionProgress({ ...running, jobKey: "TITLE_DELETION" }),
    null,
  );

  const failed = {
    ...running,
    status: "FAILED" as const,
    summaryJson: { reason: "LIBRARY_DELETION_ALREADY_RUNNING" },
  };
  assert.equal(
    libraryDeletionFailureReason(failed),
    "LIBRARY_DELETION_ALREADY_RUNNING",
  );
  assert.equal(
    libraryDeletionFailureReason({ ...failed, summaryJson: { reason: null } }),
    null,
  );
});

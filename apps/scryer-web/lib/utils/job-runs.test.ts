import assert from "node:assert/strict";
import test from "node:test";

import { parseFullHashBackfillFailures } from "./job-runs.ts";

test("full-hash backfill failures are read with their path and reason", () => {
  const parsed = parseFullHashBackfillFailures({
    jobKey: "FULL_HASH_BACKFILL",
    summaryJson: {
      failed: 2,
      failures: [
        { mediaFileId: "file-000", path: "/synthetic/a.mkv", reason: "file changed while hashing" },
        { mediaFileId: "file-001", path: "/synthetic/b.mkv", reason: "permission denied" },
      ],
      failuresTruncated: false,
    },
  });

  assert.deepEqual(parsed, {
    failures: [
      { mediaFileId: "file-000", path: "/synthetic/a.mkv", reason: "file changed while hashing" },
      { mediaFileId: "file-001", path: "/synthetic/b.mkv", reason: "permission denied" },
    ],
    notListed: 0,
  });
});

test("a truncated failure list reports how many were not listed", () => {
  const parsed = parseFullHashBackfillFailures({
    jobKey: "FULL_HASH_BACKFILL",
    summaryJson: JSON.stringify({
      failed: 150,
      failures: [{ mediaFileId: "file-000", path: "/synthetic/a.mkv", reason: "read error" }],
      failuresTruncated: true,
    }),
  });

  assert.equal(parsed.failures.length, 1);
  assert.equal(parsed.notListed, 149);
});

test("garbage, legacy, and other jobs' summaries yield no failures", () => {
  const empty = { failures: [], notListed: 0 };
  for (const summaryJson of [null, "not json", 42, [], {}, { failed: 3 }, { failures: "nope" }]) {
    assert.deepEqual(
      parseFullHashBackfillFailures({ jobKey: "FULL_HASH_BACKFILL", summaryJson }),
      empty,
    );
  }
  assert.deepEqual(
    parseFullHashBackfillFailures({
      jobKey: "FULL_HASH_BACKFILL",
      summaryJson: { failures: [null, { path: 1, reason: "x" }, { path: "/synthetic/c.mkv" }] },
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

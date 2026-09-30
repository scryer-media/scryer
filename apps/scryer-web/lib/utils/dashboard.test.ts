import assert from "node:assert/strict";
import test from "node:test";

import {
  aggregateClientActivity,
  attentionTotal,
  compareProviderRows,
  formatCompactAge,
  formatStorageSize,
  formatTerabytes,
  groupStorageRootsByLibrary,
  isBreakingVersionChange,
  isProviderCoolingDown,
  isProviderErroring,
  summarizeIndexerHealth,
  usagePercent,
  usageTone,
  type StorageRootUsage,
} from "./dashboard.ts";

test("the usage ramp changes tone and tag at 65, 80 and 90 percent", () => {
  assert.deepEqual(usageTone(0), { tone: "success", tag: "none" });
  assert.deepEqual(usageTone(64.9), { tone: "success", tag: "none" });
  assert.deepEqual(usageTone(65), { tone: "warning", tag: "none" });
  assert.deepEqual(usageTone(79.9), { tone: "warning", tag: "none" });
  assert.deepEqual(usageTone(80), { tone: "warning", tag: "low" });
  assert.deepEqual(usageTone(89.9), { tone: "warning", tag: "low" });
  assert.deepEqual(usageTone(90), { tone: "danger", tag: "crit" });
  assert.deepEqual(usageTone(100), { tone: "danger", tag: "crit" });
});

test("usage percent reports unknown rather than zero when a figure is missing", () => {
  assert.equal(usagePercent(50, 200), 25);
  assert.equal(usagePercent(0, 200), 0);
  assert.equal(usagePercent(null, 200), null);
  assert.equal(usagePercent(50, null), null);
  assert.equal(usagePercent(50, 0), null);
  // A filesystem reporting more used than total must not exceed a full ring.
  assert.equal(usagePercent(300, 200), 100);
});

test("terabytes keep one decimal and unknown sizes stay unknown", () => {
  assert.equal(formatTerabytes(11_600_000_000_000), "11.6");
  assert.equal(formatTerabytes(0), "0.0");
  assert.equal(formatTerabytes(null), null);
  assert.equal(formatTerabytes(-1), null);
});

test("storage sizes use GB below one TB and TB at or above one TB", () => {
  assert.deepEqual(formatStorageSize(999_000_000_000), {
    value: "999.0",
    unit: "GB",
  });
  assert.deepEqual(formatStorageSize(1_000_000_000_000), {
    value: "1.0",
    unit: "TB",
  });
  assert.equal(formatStorageSize(null), null);
  assert.equal(formatStorageSize(-1), null);
});

test("compact ages bucket into minutes, hours and days", () => {
  const now = Date.parse("2026-08-18T12:00:00Z");
  assert.equal(formatCompactAge("2026-08-18T11:59:30Z", now), "now");
  assert.equal(formatCompactAge("2026-08-18T11:19:00Z", now), "41m");
  assert.equal(formatCompactAge("2026-08-18T06:00:00Z", now), "6h");
  assert.equal(formatCompactAge("2026-08-15T12:00:00Z", now), "3d");
  // A clock skewed into the future clamps instead of printing a negative age.
  assert.equal(formatCompactAge("2026-08-19T12:00:00Z", now), "now");
  assert.equal(formatCompactAge(null, now), null);
  assert.equal(formatCompactAge("not a date", now), null);
});

test("only a major version bump counts as a breaking plugin update", () => {
  assert.equal(isBreakingVersionChange("1.9.7", "2.4.1"), true);
  assert.equal(isBreakingVersionChange("v1.0.0", "v2.0.0"), true);
  assert.equal(isBreakingVersionChange("1.9.7", "1.10.0"), false);
  assert.equal(isBreakingVersionChange("2.0.0", "2.0.1"), false);
  // A downgrade is not a break, and neither is an unparseable version.
  assert.equal(isBreakingVersionChange("2.0.0", "1.0.0"), false);
  assert.equal(isBreakingVersionChange("nightly", "2.0.0"), false);
  assert.equal(isBreakingVersionChange(null, "2.0.0"), false);
});

function root(overrides: Partial<StorageRootUsage>): StorageRootUsage {
  return {
    path: "/data/movies",
    libraryId: "library-movies",
    libraryName: "Movies",
    facet: "MOVIE",
    usedBytes: 1,
    totalBytes: 10,
    ...overrides,
  };
}

test("storage roots group by library, ordered by the worst root descending", () => {
  const groups = groupStorageRootsByLibrary([
    root({ path: "/data/movies", libraryId: "movies", libraryName: "Movies", usedBytes: 5, totalBytes: 10 }),
    root({ path: "/data/series", libraryId: "series", libraryName: "Series", usedBytes: 9, totalBytes: 10 }),
    root({ path: "/data/4k", libraryId: "movies", libraryName: "Movies", usedBytes: 97, totalBytes: 100 }),
    root({ path: "/data/anime", libraryId: "anime", libraryName: "Anime", usedBytes: 1, totalBytes: 10 }),
  ]);

  assert.deepEqual(
    groups.map((group) => group.libraryId),
    ["movies", "series", "anime"],
  );
  // Within a library the fullest root leads.
  assert.deepEqual(
    groups[0].roots.map((entry) => entry.path),
    ["/data/4k", "/data/movies"],
  );
});

test("roots with unknown usage sort last and never rank a library as worst", () => {
  const groups = groupStorageRootsByLibrary([
    root({ path: "/mnt/unknown", libraryId: "unknown", libraryName: "Unknown", usedBytes: null, totalBytes: null }),
    root({ path: "/data/movies", libraryId: "movies", libraryName: "Movies", usedBytes: 5, totalBytes: 10 }),
    root({ path: "/data/movies-b", libraryId: "movies", libraryName: "Movies", usedBytes: null, totalBytes: null }),
  ]);

  assert.deepEqual(
    groups.map((group) => group.libraryId),
    ["movies", "unknown"],
  );
  assert.deepEqual(
    groups[0].roots.map((entry) => entry.path),
    ["/data/movies", "/data/movies-b"],
  );
});

test("provider health ignores disabled providers and reads status or error", () => {
  assert.equal(isProviderErroring(false, "unhealthy", "boom"), false);
  assert.equal(isProviderErroring(true, "unhealthy", null), true);
  assert.equal(isProviderErroring(true, "healthy", "boom"), true);
  assert.equal(isProviderErroring(true, "healthy", null), false);
  assert.equal(isProviderErroring(true, null, "   "), false);
});

test("both provider status vocabularies count as failing", () => {
  // Indexers say unhealthy; download clients say error/failed. A client that
  // reports a bad status but no error text must not render as OK.
  for (const status of ["unhealthy", "error", "failed", "FAILED", " Error "]) {
    assert.equal(isProviderErroring(true, status, null), true, status);
  }
  for (const status of ["healthy", "ok", "idle", ""]) {
    assert.equal(isProviderErroring(true, status, null), false, status);
  }
});

test("indexer health counts exclude disabled indexers from both sides", () => {
  const summary = summarizeIndexerHealth([
    { isEnabled: true, lastHealthStatus: "healthy", lastErrorMessage: null },
    { isEnabled: true, lastHealthStatus: "healthy", lastErrorMessage: null },
    { isEnabled: true, lastHealthStatus: "unhealthy", lastErrorMessage: "auth failed" },
    { isEnabled: false, lastHealthStatus: "unhealthy", lastErrorMessage: "auth failed" },
  ]);

  assert.deepEqual(summary, { healthy: 2, enabled: 3, erroring: 1 });
});

test("a cooling indexer is quiet, not erroring", () => {
  const soon = new Date(Date.now() + 5 * 60_000).toISOString();
  const past = new Date(Date.now() - 60_000).toISOString();
  const summary = summarizeIndexerHealth([
    { isEnabled: true, lastHealthStatus: "healthy", lastErrorMessage: null },
    // Stale error text from before the cooldown must not tip it into erroring.
    {
      isEnabled: true,
      lastHealthStatus: "unhealthy",
      lastErrorMessage: "rate limited",
      rateLimitedUntil: soon,
    },
    // An elapsed cooldown counts again, the same as any other indexer.
    {
      isEnabled: true,
      lastHealthStatus: "unhealthy",
      lastErrorMessage: "auth failed",
      rateLimitedUntil: past,
    },
  ]);

  // Cooling still counts as enabled and healthy: it is quiet on request.
  assert.deepEqual(summary, { healthy: 2, enabled: 3, erroring: 1 });
  assert.equal(isProviderCoolingDown(soon), true);
  assert.equal(isProviderCoolingDown(past), false);
  assert.equal(isProviderCoolingDown(null), false);
});

test("per-client counts split active from queued and drop terminal rows", () => {
  const counts = aggregateClientActivity([
    { clientId: "sab", displayState: "DOWNLOADING" },
    { clientId: "sab", displayState: "IMPORTING" },
    { clientId: "sab", displayState: "QUEUED" },
    { clientId: "sab", displayState: "COMPLETED" },
    { clientId: "sab", displayState: "FAILED" },
    { clientId: "qbit", displayState: "PAUSED" },
    { clientId: "   ", displayState: "DOWNLOADING" },
  ]);

  assert.deepEqual(counts.get("sab"), { active: 2, queued: 1 });
  assert.deepEqual(counts.get("qbit"), { active: 0, queued: 1 });
  assert.equal(counts.has("   "), false);
});

test("attention total sums the four operator queues", () => {
  assert.equal(
    attentionTotal({ requests: 7, imports: 4, pluginUpdates: 2, indexerErrors: 1 }),
    14,
  );
  assert.equal(
    attentionTotal({ requests: 0, imports: 0, pluginUpdates: 0, indexerErrors: 0 }),
    0,
  );
});

test("provider rows needing attention sort ahead of busier healthy rows", () => {
  const rows = [
    { needsAttention: false, usage: 500, name: "Busy" },
    { needsAttention: true, usage: 0, name: "Broken" },
    { needsAttention: false, usage: 20, name: "Quiet" },
  ];
  assert.deepEqual(
    [...rows].sort(compareProviderRows).map((row) => row.name),
    ["Broken", "Busy", "Quiet"],
  );
});

test("equally healthy providers sort by usage then name", () => {
  const rows = [
    { needsAttention: false, usage: 10, name: "Beta" },
    { needsAttention: false, usage: 10, name: "Alpha" },
    { needsAttention: false, usage: 90, name: "Zulu" },
  ];
  assert.deepEqual(
    [...rows].sort(compareProviderRows).map((row) => row.name),
    ["Zulu", "Alpha", "Beta"],
  );
});

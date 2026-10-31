import assert from "node:assert/strict";
import test from "node:test";
import {
  earliestSearchExpiry,
  expireIndexerSearches,
  indexerSearchExpiresAt,
} from "./indexer-search-expiry.ts";

test("completed searches expire five minutes after server completion, not receipt", () => {
  assert.equal(
    indexerSearchExpiresAt({
      startedAt: "2026-01-01T00:00:00Z",
      completedAt: "2026-01-01T00:00:30Z",
    }),
    Date.parse("2026-01-01T00:05:30Z"),
  );
});

test("partial and cancelled searches without completion expire conservatively", () => {
  assert.equal(
    indexerSearchExpiresAt({ startedAt: "2026-01-01T00:00:00Z" }),
    Date.parse("2026-01-01T00:05:00Z"),
  );
  assert.equal(indexerSearchExpiresAt({}), 0);
  assert.equal(indexerSearchExpiresAt({ completedAt: "invalid" }), 0);
});

test("retry jobs do not extend the original results' lifetime", () => {
  const deadlines = new Map([
    ["original", 100],
    ["retry", 200],
  ]);
  assert.equal(earliestSearchExpiry(deadlines), 100);
  deadlines.set("retry", 300);
  assert.equal(earliestSearchExpiry(deadlines), 100);
  deadlines.clear();
  assert.equal(earliestSearchExpiry(deadlines), null);
});

const owners = new Map([
  ["row-a", "original"],
  ["row-b", "original"],
  ["row-c", "retry"],
]);

test("nothing expires before the earliest deadline", () => {
  const deadlines = new Map([
    ["original", 100],
    ["retry", 200],
  ]);
  assert.equal(
    expireIndexerSearches({
      deadlines,
      rowOwners: owners,
      activeSearchId: "retry",
      now: 99,
    }),
    null,
  );
});

test("the original search expiring drops only its rows and leaves a running retry alone", () => {
  const deadlines = new Map([
    ["original", 100],
    ["retry", 200],
  ]);
  const expiry = expireIndexerSearches({
    deadlines,
    rowOwners: owners,
    activeSearchId: "retry",
    now: 100,
  });
  assert.ok(expiry);
  assert.deepEqual([...expiry.expired], ["original"]);
  assert.deepEqual([...expiry.deadlines], [["retry", 200]]);
  assert.deepEqual([...expiry.rowOwners], [["row-c", "retry"]]);
  assert.equal(expiry.activeExpired, false);
  assert.deepEqual(["row-a", "row-b", "row-c"].filter(expiry.isRowLive), [
    "row-c",
  ]);
  assert.equal(earliestSearchExpiry(expiry.deadlines), 200);
  // The caller's maps are left untouched.
  assert.equal(deadlines.size, 2);
  assert.equal(owners.size, 3);
});

test("a row a retry found again belongs to the retry and outlives the original", () => {
  const rowOwners = new Map([...owners, ["row-a", "retry"]]);
  const expiry = expireIndexerSearches({
    deadlines: new Map([
      ["original", 100],
      ["retry", 200],
    ]),
    rowOwners,
    activeSearchId: "retry",
    now: 150,
  });
  assert.ok(expiry);
  assert.deepEqual(["row-a", "row-b", "row-c"].filter(expiry.isRowLive), [
    "row-a",
    "row-c",
  ]);
});

test("a retry reaching its own deadline is reported as the active search expiring", () => {
  const expiry = expireIndexerSearches({
    deadlines: new Map([["retry", 200]]),
    rowOwners: new Map([["row-c", "retry"]]),
    activeSearchId: "retry",
    now: 250,
  });
  assert.ok(expiry);
  assert.equal(expiry.activeExpired, true);
  assert.equal(expiry.deadlines.size, 0);
  assert.equal(expiry.rowOwners.size, 0);
  assert.equal(expiry.isRowLive("row-c"), false);
});

test("a finished search expiring does not claim the active search expired", () => {
  const expiry = expireIndexerSearches({
    deadlines: new Map([["original", 100]]),
    rowOwners: owners,
    activeSearchId: null,
    now: 100,
  });
  assert.ok(expiry);
  assert.equal(expiry.activeExpired, false);
});

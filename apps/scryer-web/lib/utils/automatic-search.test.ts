import assert from "node:assert/strict";
import { test } from "node:test";
import { automaticSearchOutcomeKey, matchesActiveAutomaticSearch, parseSearchSeason, shouldRestoreAutomaticSearch } from "./automatic-search.ts";
import { normalizeJobRun, preferJobRunSnapshot } from "./job-runs.ts";

function run(progress: Record<string, unknown> = {}, status = "RUNNING") {
  return normalizeJobRun({
    id: "search-1", jobKey: "ACQUISITION_SEARCH", displayName: "Search",
    status, startedAt: "2026-09-12T00:00:00Z",
    progressJson: { search: { titleId: "title-1", seasonNumber: 0, intent: "AUTOMATIC" }, ...progress },
  })!;
}

test("specials are valid and malformed season labels are not silently coerced", () => {
  assert.equal(parseSearchSeason("0"), 0);
  assert.equal(parseSearchSeason(" 7 "), 7);
  for (const value of [null, undefined, "", "Season 1", "-1", "1.2", "999999999999"]) {
    assert.equal(parseSearchSeason(value), null);
  }
});

test("search activity follows title and season scopes across navigation", () => {
  const active = run();
  assert.equal(matchesActiveAutomaticSearch(active, "title-1"), true);
  assert.equal(matchesActiveAutomaticSearch(active, "title-1", 0), true);
  assert.equal(matchesActiveAutomaticSearch(active, "title-1", 1), false);
  assert.equal(matchesActiveAutomaticSearch(active, "title-2"), false);
  assert.equal(matchesActiveAutomaticSearch(run({}, "COMPLETED"), "title-1"), false);
  const wholeTitle = run({ search: { titleId: "title-1", seasonNumber: null } });
  assert.equal(matchesActiveAutomaticSearch(wholeTitle, "title-1", 7), true);
});

test("late start and fallback responses cannot revive a completed search", () => {
  const terminal = run({ outcome: "no_eligible_work", revision: 3 }, "COMPLETED");
  assert.equal(preferJobRunSnapshot(terminal, run({ revision: 0 })), terminal);
  assert.equal(preferJobRunSnapshot(terminal, run({ revision: 2 })), terminal);
});

test("out of order running snapshots retain the freshest progress", () => {
  const latest = run({ revision: 8, processed: 6 });
  assert.equal(preferJobRunSnapshot(latest, run({ revision: 3, processed: 1 })), latest);
  const newer = run({ revision: 9, processed: 7 });
  assert.equal(preferJobRunSnapshot(latest, newer), newer);
});

test("empty eligibility and completed searches with no acceptable releases have distinct feedback", () => {
  assert.equal(automaticSearchOutcomeKey(run({ outcome: "no_eligible_work" }, "COMPLETED")), "wanted.searchNoEligibleWork");
  assert.equal(automaticSearchOutcomeKey(run({ outcome: "no_acceptable_releases" }, "COMPLETED")), "wanted.searchNoAcceptableReleases");
  assert.equal(automaticSearchOutcomeKey(run({ outcome: "downloads_submitted" }, "COMPLETED")), "wanted.searchJobComplete");
  assert.equal(automaticSearchOutcomeKey(run({ outcome: "cancelled" }, "WARNING")), "wanted.searchJobCancelled");
  assert.equal(automaticSearchOutcomeKey(run()), null);
});

test("historical jobs without search metadata remain readable", () => {
  const old = run({ search: undefined });
  assert.equal(matchesActiveAutomaticSearch(old, "title-1"), false);
  assert.equal(automaticSearchOutcomeKey(old), null);
});

test("restored automatic searches resume reconciliation without reviving completed jobs", () => {
  assert.equal(shouldRestoreAutomaticSearch(run()), true);
  assert.equal(shouldRestoreAutomaticSearch(run({}, "COMPLETED")), false);
  assert.equal(shouldRestoreAutomaticSearch(run({ search: undefined })), false);
  assert.equal(shouldRestoreAutomaticSearch(run({ search: { intent: "WANTED", titleId: "title-1" } })), false);
  const completed = run({ revision: 4 }, "COMPLETED");
  assert.equal(shouldRestoreAutomaticSearch(preferJobRunSnapshot(completed, run())), false);
});

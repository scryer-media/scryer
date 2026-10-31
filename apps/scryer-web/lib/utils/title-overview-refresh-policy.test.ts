import assert from "node:assert/strict";
import test from "node:test";

import {
  drainDeferredCollectionEpisodeRefresh,
  planCollectionEpisodeRefresh,
  shouldHandleTitleOverviewActivity,
  TITLE_OVERVIEW_IMPORT_REFRESH_KINDS,
  titleOverviewReactiveRefreshKinds,
  titleOverviewReactiveRefreshPlan,
} from "./title-overview-refresh-policy.ts";

// The kinds both title overviews refresh on, for the title and its downloads.
const importKinds = TITLE_OVERVIEW_IMPORT_REFRESH_KINDS;

test("title overview activity gate ignores other and missing title ids", () => {
  assert.equal(shouldHandleTitleOverviewActivity("current", "other"), false);
  assert.equal(shouldHandleTitleOverviewActivity("current", null), false);
  assert.equal(shouldHandleTitleOverviewActivity("current", undefined), false);
  assert.equal(shouldHandleTitleOverviewActivity(null, "current"), false);
  assert.equal(shouldHandleTitleOverviewActivity("current", "current"), true);
});

test("title overview refresh kinds include policy-managed activity", () => {
  const kinds = titleOverviewReactiveRefreshKinds(importKinds);

  assert.equal(kinds.has("movie_downloaded"), true);
  assert.equal(kinds.has("series_episode_imported"), true);
  assert.equal(kinds.has("file_analyzed"), true);
  assert.equal(kinds.has("subtitle_downloaded"), true);
  assert.equal(kinds.has("metadata_hydration_started"), true);
  assert.equal(kinds.has("metadata_hydration_completed"), true);
  assert.equal(kinds.has("metadata_hydration_failed"), true);
});

test("file analyzed activity is a debounced refresh", () => {
  assert.deepEqual(
    titleOverviewReactiveRefreshPlan("file_analyzed", importKinds),
    {
      type: "refresh",
      mode: "bulk",
    },
  );
});

test("subtitle activity is an immediate refresh", () => {
  assert.deepEqual(
    titleOverviewReactiveRefreshPlan("subtitle_downloaded", importKinds),
    {
      type: "refresh",
      mode: "immediate",
    },
  );
});

test("import lifecycle activity is an immediate refresh", () => {
  for (const kind of [
    "movie_downloaded",
    "series_episode_imported",
    "file_upgraded",
    "import_rejected",
  ]) {
    assert.deepEqual(titleOverviewReactiveRefreshPlan(kind, importKinds), {
      type: "refresh",
      mode: "immediate",
    });
  }
});

test("hydration activity reports UI-only transitions except completed", () => {
  assert.deepEqual(
    titleOverviewReactiveRefreshPlan("metadata_hydration_started", importKinds),
    { type: "hydrationStarted" },
  );
  assert.deepEqual(
    titleOverviewReactiveRefreshPlan(
      "metadata_hydration_completed",
      importKinds,
    ),
    { type: "hydrationCompleted" },
  );
  assert.deepEqual(
    titleOverviewReactiveRefreshPlan("metadata_hydration_failed", importKinds),
    { type: "hydrationFailed" },
  );
});

test("a refresh arriving during the first load is re-run when that load resolves", () => {
  // The import lands while the open season's first SeriesCollectionEpisodes
  // query is still in flight. Dropping that refresh leaves the page showing
  // the pre-import episode list until the next navigation.
  const inFlight = new Set(["season-1"]);
  const plan = planCollectionEpisodeRefresh(["season-1"], inFlight);
  assert.deepEqual(plan.refreshNow, []);
  assert.deepEqual(plan.deferred, ["season-1"]);

  const drained = drainDeferredCollectionEpisodeRefresh(
    new Set(plan.deferred),
    "season-1",
  );
  assert.deepEqual(drained.rerun, ["season-1"]);
  assert.deepEqual([...drained.remaining], []);
});

test("collections with no load in flight refresh immediately", () => {
  const plan = planCollectionEpisodeRefresh(
    ["season-1", "season-2"],
    new Set(["season-2"]),
  );
  assert.deepEqual(plan.refreshNow, ["season-1"]);
  assert.deepEqual(plan.deferred, ["season-2"]);
});

test("a resolved load leaves the other deferred collections pending", () => {
  const drained = drainDeferredCollectionEpisodeRefresh(
    new Set(["season-1", "season-2"]),
    "season-1",
  );
  assert.deepEqual(drained.rerun, ["season-1"]);
  assert.deepEqual([...drained.remaining], ["season-2"]);
});

test("a load resolving with nothing deferred re-runs nothing", () => {
  const drained = drainDeferredCollectionEpisodeRefresh(new Set(), "season-1");
  assert.deepEqual(drained.rerun, []);
  assert.deepEqual([...drained.remaining], []);
});

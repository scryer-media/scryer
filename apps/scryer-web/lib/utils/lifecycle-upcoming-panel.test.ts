import assert from "node:assert/strict";
import test from "node:test";

import {
  LIFECYCLE_UPCOMING_ACTION_GROUPS,
  LIFECYCLE_UPCOMING_NAME_MAX_LENGTH,
  lifecycleUpcomingActionHeads,
  lifecycleUpcomingGroupHead,
  lifecycleUpcomingPanelSettings,
  lifecycleUpcomingSettingsValue,
  lifecycleUpcomingTiming,
} from "./lifecycle-upcoming-panel.ts";

const DEFAULTS = { actions: [], name: "" };

test("anything that is not a settings object reads as the defaults", () => {
  for (const saved of [undefined, null, "notify", 7, true, ["notify"]]) {
    assert.deepEqual(lifecycleUpcomingPanelSettings(saved), DEFAULTS);
  }
});

test("settings keep known groups in apply order and ignore the rest", () => {
  assert.deepEqual(
    lifecycleUpcomingPanelSettings({
      actions: ["notify", "delete_files", "explode", 3, null, "notify"],
      name: "Soon",
      columns: 4,
    }),
    { actions: ["delete_files", "notify"], name: "Soon" },
  );
  assert.deepEqual(
    lifecycleUpcomingPanelSettings({ actions: "notify", name: 12 }),
    DEFAULTS,
  );
});

test("a name is trimmed and cut to the limit", () => {
  assert.equal(
    lifecycleUpcomingPanelSettings({ name: "  Due soon  " }).name,
    "Due soon",
  );
  const long = lifecycleUpcomingPanelSettings({ name: "x".repeat(200) }).name;
  assert.equal(long.length, LIFECYCLE_UPCOMING_NAME_MAX_LENGTH);
  assert.equal(lifecycleUpcomingPanelSettings({ name: "   " }).name, "");
});

test("the defaults save as no settings at all", () => {
  assert.equal(lifecycleUpcomingSettingsValue(DEFAULTS), null);
  assert.equal(
    lifecycleUpcomingSettingsValue({ actions: [], name: "  " }),
    null,
  );
  assert.deepEqual(
    lifecycleUpcomingSettingsValue({ actions: ["search"], name: "" }),
    { actions: ["search"], name: "" },
  );
});

test("every group expands to its title head first, then its narrower forms", () => {
  assert.equal(lifecycleUpcomingActionHeads([]), undefined);
  for (const group of LIFECYCLE_UPCOMING_ACTION_GROUPS) {
    const heads = lifecycleUpcomingActionHeads([group]);
    assert.ok(heads && heads.length > 0, group);
    assert.equal(heads[0], lifecycleUpcomingGroupHead(group));
    assert.equal(heads[0], group.toUpperCase());
  }
  assert.deepEqual(lifecycleUpcomingActionHeads(["delete_files", "search"]), [
    "DELETE_FILES",
    "DELETE_SEASON_FILES",
    "DELETE_EPISODE_FILES",
    "SEARCH",
  ]);
});

test("timing reads due for due states and past times, otherwise a countdown", () => {
  const now = Date.parse("2026-03-01T12:00:00Z");
  const inTwoDays = "2026-03-03T13:00:00Z";
  assert.deepEqual(
    lifecycleUpcomingTiming({ state: "DUE", dueAt: inTwoDays }, now),
    { due: true },
  );
  assert.deepEqual(
    lifecycleUpcomingTiming({ state: "PENDING_ACTION", dueAt: inTwoDays }, now),
    { due: true },
  );
  assert.deepEqual(
    lifecycleUpcomingTiming(
      { state: "OBSERVING", dueAt: "2026-03-01T11:00:00Z" },
      now,
    ),
    { due: true },
  );
  assert.deepEqual(
    lifecycleUpcomingTiming({ state: "OBSERVING", dueAt: inTwoDays }, now),
    {
      due: false,
      labelKey: "settings.lifecycleCountdownInDays",
      values: { count: 2 },
    },
  );
  assert.deepEqual(
    lifecycleUpcomingTiming(
      { state: "OBSERVING", dueAt: "2026-03-01T15:30:00Z" },
      now,
    ),
    {
      due: false,
      labelKey: "settings.lifecycleCountdownInHours",
      values: { count: 3 },
    },
  );
  assert.deepEqual(
    lifecycleUpcomingTiming({ state: "OBSERVING", dueAt: "soon" }, now),
    { due: false, labelKey: null, values: { count: 0 } },
  );
});

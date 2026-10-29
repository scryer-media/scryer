import assert from "node:assert/strict";
import test from "node:test";
import { resolveDefaultMonitorType } from "./default-monitor-type.ts";

test("saved episodic defaults are scoped by the selected library's value", () => {
  assert.equal(resolveDefaultMonitorType("SERIES", "MISSING_AND_FUTURE_EPISODES", "FUTURE_EPISODES"), "MISSING_AND_FUTURE_EPISODES");
  assert.equal(resolveDefaultMonitorType("ANIME", "NONE", "FUTURE_EPISODES"), "NONE");
  assert.equal(resolveDefaultMonitorType("SERIES", null, "FUTURE_EPISODES"), "FUTURE_EPISODES");
});
test("advanced selection and values for the wrong facet cannot become defaults", () => {
  for (const value of ["ADVANCED", "MONITORED", "unknown", ""]) {
    assert.equal(resolveDefaultMonitorType("SERIES", value, "FUTURE_EPISODES"), "FUTURE_EPISODES");
  }
  assert.equal(resolveDefaultMonitorType("MOVIE", "ALL_EPISODES", "MONITORED"), "MONITORED");
  assert.equal(resolveDefaultMonitorType("MOVIE", "UNMONITORED", "MONITORED"), "UNMONITORED");
});

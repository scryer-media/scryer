import assert from "node:assert/strict";
import test from "node:test";
import { createWantedDetailsRequests } from "./wanted-details-request.ts";

test("late details and queue refreshes cannot populate another Wanted row", () => {
  const requests = createWantedDetailsRequests();
  const first = requests.select("first");
  const refresh = requests.refresh("first")!;
  const second = requests.select("second");
  assert.equal(requests.isCurrent(first), false);
  assert.equal(requests.isCurrent(refresh), false);
  assert.equal(requests.refresh("first"), null);
  assert.equal(requests.isCurrent(second), true);
});

test("collapse, reopen, and newer refreshes invalidate earlier responses", () => {
  const requests = createWantedDetailsRequests();
  const first = requests.select("first");
  requests.select(null);
  assert.equal(requests.refresh("first"), null);
  requests.select("first");
  assert.equal(requests.isCurrent(first), false);
  const older = requests.refresh("first")!;
  const newer = requests.refresh("first")!;
  assert.equal(requests.isCurrent(older), false);
  assert.equal(requests.isCurrent(newer), true);
});

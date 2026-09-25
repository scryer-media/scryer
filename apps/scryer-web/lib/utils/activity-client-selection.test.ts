import assert from "node:assert/strict";
import test from "node:test";

import { reconcileActivityClientSelection } from "./activity-client-selection.ts";

test("the all-clients selection survives the client list arriving and briefly emptying", () => {
  let selection: string[] | null = null;
  for (const available of [[], ["client-a"], [], ["client-a", "client-b"]]) {
    selection = reconcileActivityClientSelection(selection, available);
    assert.equal(selection, null);
  }
});

test("an explicit selection is kept while the client list is briefly empty", () => {
  const selection = ["client-a"];
  assert.equal(reconcileActivityClientSelection(selection, []), selection);
});

test("an explicit selection drops clients that are no longer available", () => {
  assert.deepEqual(
    reconcileActivityClientSelection(["client-a", "client-b"], ["client-b", "client-c"]),
    ["client-b"],
  );
});

test("an unchanged explicit selection keeps its identity", () => {
  const selection = ["client-a"];
  assert.equal(reconcileActivityClientSelection(selection, ["client-a", "client-b"]), selection);
});

test("a selection the user cleared stays cleared", () => {
  const selection: string[] = [];
  assert.equal(reconcileActivityClientSelection(selection, ["client-a"]), selection);
});

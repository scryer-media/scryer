import assert from "node:assert/strict";
import test from "node:test";
import { renameExplorerTab, restoreExplorerTabNames } from "./api-explorer-tabs.ts";

test("tab labels are independent of queries and preserve GraphiQL's active-tab identity", () => {
  const tabs = [
    { title: "Lookup", query: "query Lookup { scryerVersion }", operationName: "Lookup" },
    { title: "Other", query: "{ scryerVersion }", operationName: null },
  ];
  const updated = renameExplorerTab(tabs, 0, "  My integration draft  ")!;
  assert.notEqual(updated, tabs);
  assert.equal(updated[0], tabs[0]);
  assert.equal(updated[1], tabs[1]);
  assert.equal(updated[0].title, "My integration draft");
  assert.equal(updated[0].query, "query Lookup { scryerVersion }");
  assert.equal(updated[0].operationName, "Lookup");
  assert.equal(updated[1].title, "Other");
});

test("explicit tab names survive storage round-trips and subsequent editor changes", () => {
  const named = renameExplorerTab([{ title: "<untitled>" }], 0, "Draft")!;
  const reloaded = JSON.parse(JSON.stringify(named));
  reloaded[0].title = "OperationNameAfterEditing";
  const restored = restoreExplorerTabNames(reloaded)!;
  assert.equal(restored[0].title, "Draft");
  assert.equal(restored[0], reloaded[0]);
  assert.equal(restoreExplorerTabNames(restored), null);
});

test("unnamed tabs retain automatic labels and invalid renames change nothing", () => {
  const tabs = [{ title: "Automatic" }];
  assert.equal(restoreExplorerTabNames(tabs), null);
  assert.equal(renameExplorerTab(tabs, 0, "  "), null);
  assert.equal(renameExplorerTab(tabs, 1, "Name"), null);
  assert.deepEqual(tabs, [{ title: "Automatic" }]);
});

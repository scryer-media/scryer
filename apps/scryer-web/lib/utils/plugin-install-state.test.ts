import assert from "node:assert/strict";
import test from "node:test";

import {
  applySuccessfulPluginOperationState,
  createPluginOperations,
} from "./plugin-install-state.ts";

test("successful upgrades advance installed state", () => {
  const result = applySuccessfulPluginOperationState(
    {
      installInProgress: true,
      isInstalled: true,
      updateAvailable: true,
      installedVersion: "1.0.0",
      latestVersion: "1.1.0",
      version: "1.1.0",
    },
    "UPGRADE",
  );

  assert.equal(result.installInProgress, false);
  assert.equal(result.updateAvailable, false);
  assert.equal(result.installedVersion, "1.1.0");
});

test("an operation can be ended only once", () => {
  const operations = createPluginOperations();
  const install = operations.begin("plugin-1");

  assert.equal(operations.end("plugin-1", install), true);
  assert.equal(operations.end("plugin-1", install), false);
  assert.equal(operations.size(), 0);
});

test("word about an earlier operation cannot end a later one", () => {
  const operations = createPluginOperations();
  const first = operations.begin("plugin-1");
  const second = operations.begin("plugin-1");

  assert.equal(operations.isCurrent("plugin-1", first), false);
  assert.equal(operations.end("plugin-1", first), false);
  assert.equal(operations.isCurrent("plugin-1", second), true);
});

test("a plugin list loaded before an operation began does not end it", () => {
  const operations = createPluginOperations();
  const mark = operations.mark();
  operations.begin("plugin-1");

  const idle = [{ id: "plugin-1", installInProgress: false }];
  assert.deepEqual(operations.settledBy(idle, mark), []);
  assert.equal(
    operations.settledBy(idle, operations.mark()).length,
    1,
    "a list loaded afterwards does",
  );
});

test("the plugin list settles each plugin on its own", () => {
  const operations = createPluginOperations();
  const first = operations.begin("plugin-1");
  operations.begin("plugin-2");
  const gone = operations.begin("plugin-3");
  const mark = operations.mark();

  const settled = operations.settledBy(
    [
      { id: "plugin-1", installInProgress: false },
      { id: "plugin-2", installInProgress: true },
    ],
    mark,
  );

  assert.deepEqual(settled, [
    { pluginId: "plugin-1", operation: first },
    { pluginId: "plugin-3", operation: gone },
  ]);
});

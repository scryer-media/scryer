import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { classifyStatusToastLevel } from "../utils/status-toast.ts";

const container = readFileSync(
  new URL("../../components/containers/media-content-container.tsx", import.meta.url),
  "utf8",
);
const panel = readFileSync(
  new URL("../../components/views/media-content/media-library-settings-panel.tsx", import.meta.url),
  "utf8",
);

for (const [handler, nextHandler, statusKey, message] of [
  ["createLibrary", "updateLibrary", "libraryCreated", "Library created."],
  ["updateLibrary", "deleteLibrary", "librarySaved", "Library saved."],
]) {
  test(`${handler} emits success only through the global status toast`, () => {
    const start = container.indexOf(`  const ${handler} = React.useCallback(`);
    const end = container.indexOf(`  const ${nextHandler} = React.useCallback(`, start);
    assert.ok(start >= 0 && end > start, "library mutation handler is present");
    const body = container.slice(start, end);

    assert.equal(
      body.split(`setGlobalStatus(t("settings.${statusKey}"))`).length - 1,
      1,
      "preserve one shared status notification",
    );
    assert.doesNotMatch(body, /toast\.success\(/, "do not emit a second Sonner toast");
    assert.equal(classifyStatusToastLevel(message), "SUCCESS");
  });
}

test("Save & Scan uses the shared library save handler once before starting the scan", () => {
  const start = panel.indexOf("  const handleSaveAndScanLibrary = async () => {");
  const end = panel.indexOf("  const handleDeleteLibrary", start);
  assert.ok(start >= 0 && end > start);
  const body = panel.slice(start, end);
  assert.equal(body.split("await handleSaveLibrary()").length - 1, 1);
  assert.ok(body.indexOf("await handleSaveLibrary()") < body.indexOf("onScan(libraryId)"));
  assert.doesNotMatch(body, /toast\.success\(/);
});

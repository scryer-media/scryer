import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const container = readFileSync(
  new URL("../../components/containers/media-content-container.tsx", import.meta.url),
  "utf8",
);
const panel = readFileSync(
  new URL("../../components/views/media-content/media-library-settings-panel.tsx", import.meta.url),
  "utf8",
);

for (const [handler, nextHandler, statusKey] of [
  ["createLibrary", "updateLibrary", "libraryCreated"],
  ["updateLibrary", "deleteLibrary", "librarySaved"],
]) {
  test(`${handler} emits success only through the global status toast`, () => {
    const start = container.indexOf(`  const ${handler} = React.useCallback(`);
    const end = container.indexOf(`  const ${nextHandler} = React.useCallback(`, start);
    assert.ok(start >= 0 && end > start, "library mutation handler is present");
    const body = container.slice(start, end);

    assert.equal(
      body.split(`setGlobalStatus(t("settings.${statusKey}"), { level: "SUCCESS" })`).length - 1,
      1,
      "preserve one shared status notification, raised as a success",
    );
    assert.doesNotMatch(body, /toast\.success\(/, "do not emit a second Sonner toast");
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

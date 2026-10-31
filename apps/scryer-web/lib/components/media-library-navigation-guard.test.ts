import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const panel = readFileSync(
  new URL(
    "../../components/views/media-content/media-library-settings-panel.tsx",
    import.meta.url,
  ),
  "utf8",
);

function assertPanelMatches(pattern: RegExp, message: string) {
  // assert.match would print the whole panel source on failure.
  assert.ok(pattern.test(panel), `${message} (${pattern})`);
}

function sliceBetween(
  source: string,
  startMarker: string,
  endMarker: string,
): string {
  const start = source.indexOf(startMarker);
  const end = source.indexOf(endMarker, start);
  assert.ok(start >= 0 && end > start, `${startMarker.trim()} is present`);
  return source.slice(start, end);
}

// The container drops `saving` once its mutation and library refreshes settle,
// but the panel still loads the saved settings and, for a new library, switches
// out of "new" mode. Until then the draft still reads as unsaved, so the panel's
// own save must keep the navigation guard down through that adoption.
test("a library save keeps the unsaved-changes guard down until the panel adopts what it saved", () => {
  const body = sliceBetween(
    panel,
    "  const handleSaveLibrary = async () => {",
    "  const handleSaveAndScanLibrary",
  );

  const raised = body.indexOf("setSaveInProgress(true)");
  assert.ok(raised >= 0, "the panel marks its own save as in progress");
  assert.ok(
    raised < body.indexOf("await onCreateLibrary("),
    "raised before a create",
  );
  assert.ok(
    raised < body.indexOf("await onUpdateLibrary("),
    "raised before an update",
  );

  const finallyAt = body.lastIndexOf("} finally {");
  const released = body.indexOf("setSaveInProgress(false)", finallyAt);
  assert.ok(
    finallyAt >= 0 && released > finallyAt,
    "released in a finally, so a failed save guards the draft again",
  );
  for (const adoption of [
    'setMode("existing")',
    "setActiveLibraryId(created.id)",
    "hydrateSavedSettings(refreshedSettings)",
  ]) {
    const at = body.lastIndexOf(adoption);
    assert.ok(
      at >= 0 && at < finallyAt,
      `${adoption} runs before the guard is released`,
    );
  }

  assertPanelMatches(
    /const saveInFlight = saving \|\| saveInProgress;/,
    "the guard counts the panel's save as well as the container's",
  );
  assertPanelMatches(
    /const shouldBlockNavigation = shouldBlockLibraryNavigation\(\{\s*hasDraftChanges,\s*saveInFlight,?\s*\}\);/,
    "navigation blocking follows that guard",
  );
  assertPanelMatches(
    /useBlocker\(shouldBlockNavigation\)/,
    "the router blocker uses it",
  );
});

// `useBlocker` keeps a blocked navigation blocked until `proceed` or `reset`,
// whatever the blocker function says afterwards.
test("an unsaved-changes prompt that outlives its changes completes the requested move", () => {
  assertPanelMatches(
    /resolveLibraryDiscardPrompt\(\{[\s\S]*?blockerState: libraryNavigationBlocker\.state/,
    "the panel resolves its discard prompt from the router blocker",
  );
  assertPanelMatches(
    /open=\{libraryDiscardPrompt === "open"\}/,
    "the dialog only asks while unsaved changes remain",
  );
  assertPanelMatches(
    /if \(libraryDiscardPrompt === "settled"\) \{\s*handleConfirmDiscardLibraryChanges\(\);/,
    "a settled prompt completes the navigation or library switch",
  );
});

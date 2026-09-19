import assert from "node:assert/strict";
import test from "node:test";

import { createMemoryRouter, IDLE_BLOCKER } from "react-router";

import {
  resolveLibraryDiscardPrompt,
  shouldBlockLibraryNavigation,
} from "./library-navigation-guard.ts";

type DataRouter = ReturnType<typeof createMemoryRouter>;

type PanelState = {
  hasDraftChanges: boolean;
  saveInFlight: boolean;
  pendingLibrarySelection: string | null;
  activeLibraryId: string | null;
};

const BLOCKER_KEY = "library-settings";

function libraryRouter(): DataRouter {
  return createMemoryRouter(
    [{ path: "/settings/libraries" }, { path: "/movies" }],
    { initialEntries: ["/settings/libraries"] },
  );
}

// Wires the guard into a real router the way the panel does: `useBlocker`
// registers the blocker function, each render resolves the discard prompt,
// and a settled prompt finishes whatever move the user asked for.
function mountLibraryGuard(router: DataRouter, panel: PanelState) {
  router.getBlocker(BLOCKER_KEY, () => shouldBlockLibraryNavigation(panel));
  const blocker = () => router.state.blockers.get(BLOCKER_KEY) ?? IDLE_BLOCKER;
  const prompt = () =>
    resolveLibraryDiscardPrompt({
      hasDraftChanges: panel.hasDraftChanges,
      blockerState: blocker().state,
      pendingLibrarySelection: panel.pendingLibrarySelection,
    });
  const render = () => {
    if (prompt() !== "settled") {
      return;
    }
    const current = blocker();
    if (current.state === "blocked") {
      current.proceed?.();
      return;
    }
    if (panel.pendingLibrarySelection !== null) {
      panel.activeLibraryId = panel.pendingLibrarySelection;
      panel.pendingLibrarySelection = null;
    }
  };
  return { blocker, prompt, render };
}

async function waitForPathname(router: DataRouter, pathname: string) {
  if (router.state.location.pathname === pathname) {
    return;
  }
  await new Promise<void>((resolve, reject) => {
    // A hang guard only: the wait ends on the router's own state change.
    const timer = setTimeout(() => {
      unsubscribe();
      reject(
        new Error(
          `navigation to ${pathname} never completed; still at ${router.state.location.pathname}`,
        ),
      );
    }, 30_000);
    const unsubscribe = router.subscribe((state) => {
      if (state.location.pathname === pathname) {
        clearTimeout(timer);
        unsubscribe();
        resolve();
      }
    });
  });
}

test("a navigation blocked for unsaved changes goes through once those changes are saved", async () => {
  const router = libraryRouter();
  try {
    // A new library whose save has landed, but the panel still reads the draft
    // as unsaved because it has not adopted the created library yet.
    const panel: PanelState = {
      hasDraftChanges: true,
      saveInFlight: false,
      pendingLibrarySelection: null,
      activeLibraryId: null,
    };
    const guard = mountLibraryGuard(router, panel);

    await router.navigate("/movies");
    assert.equal(guard.blocker().state, "blocked");
    assert.equal(guard.prompt(), "open");

    // The panel adopts the saved library, so nothing is unsaved any more.
    panel.hasDraftChanges = false;
    guard.render();

    await waitForPathname(router, "/movies");
    assert.notEqual(guard.blocker().state, "blocked");
    assert.equal(guard.prompt(), "idle");
  } finally {
    router.dispose();
  }
});

test("a library switch held for unsaved changes is applied once those changes are saved", () => {
  const router = libraryRouter();
  try {
    const panel: PanelState = {
      hasDraftChanges: true,
      saveInFlight: false,
      pendingLibrarySelection: "library-2",
      activeLibraryId: null,
    };
    const guard = mountLibraryGuard(router, panel);
    assert.equal(guard.prompt(), "open");

    panel.hasDraftChanges = false;
    guard.render();

    assert.equal(panel.activeLibraryId, "library-2");
    assert.equal(panel.pendingLibrarySelection, null);
    assert.equal(guard.prompt(), "idle");
  } finally {
    router.dispose();
  }
});

test("a prompt for changes that are still unsaved stays open and keeps the navigation blocked", async () => {
  const router = libraryRouter();
  try {
    const panel: PanelState = {
      hasDraftChanges: true,
      saveInFlight: false,
      pendingLibrarySelection: null,
      activeLibraryId: "library-1",
    };
    const guard = mountLibraryGuard(router, panel);

    await router.navigate("/movies");
    guard.render();

    assert.equal(guard.blocker().state, "blocked");
    assert.equal(guard.prompt(), "open");
    assert.equal(router.state.location.pathname, "/settings/libraries");
  } finally {
    router.dispose();
  }
});

test("a navigation while a save is in flight is not blocked", async () => {
  const router = libraryRouter();
  try {
    const panel: PanelState = {
      hasDraftChanges: true,
      saveInFlight: true,
      pendingLibrarySelection: null,
      activeLibraryId: null,
    };
    const guard = mountLibraryGuard(router, panel);

    await router.navigate("/movies");

    assert.equal(router.state.location.pathname, "/movies");
    assert.equal(guard.prompt(), "idle");
  } finally {
    router.dispose();
  }
});

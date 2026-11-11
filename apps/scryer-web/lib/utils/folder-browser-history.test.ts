import assert from "node:assert/strict";
import test from "node:test";

import { createMemoryRouter, IDLE_BLOCKER, type To } from "react-router";

import {
  folderBrowserHistoryEntry,
  folderBrowserHistoryState,
  folderBrowserPopAction,
  isFolderBrowserHistoryStep,
  newFolderBrowserSession,
} from "./folder-browser-history.ts";

type DataRouter = ReturnType<typeof createMemoryRouter>;

function settingsRouter(): DataRouter {
  return createMemoryRouter(
    [{ path: "/settings/libraries" }, { path: "/movies" }],
    { initialEntries: ["/movies", "/settings/libraries"], initialIndex: 1 },
  );
}

async function settled(router: DataRouter, key: string) {
  for (let attempt = 0; attempt < 1000; attempt += 1) {
    if (router.state.location.key !== key) return;
    await new Promise((resolve) => setImmediate(resolve));
  }
  throw new Error("router did not move");
}

async function step(router: DataRouter, to: To | number, state?: unknown) {
  const key = router.state.location.key;
  if (typeof to === "number") {
    await router.navigate(to);
  } else {
    await router.navigate(to, { state });
  }
  await settled(router, key);
}

function pushFolder(
  router: DataRouter,
  session: string,
  path: string,
  depth: number,
) {
  const { pathname, search, hash, state } = router.state.location;
  return step(
    router,
    { pathname, search, hash },
    folderBrowserHistoryState(state, { session, path, depth }),
  );
}

test("the entry keeps unrelated route state and rejects malformed entries", () => {
  const state = folderBrowserHistoryState(
    { libraryGraph: { scope: "movie" } },
    { session: "s", path: "/media/Shows", depth: 2 },
  );
  assert.deepEqual((state as Record<string, unknown>).libraryGraph, {
    scope: "movie",
  });
  assert.deepEqual(folderBrowserHistoryEntry(structuredClone(state)), {
    session: "s",
    path: "/media/Shows",
    depth: 2,
  });
  for (const broken of [
    null,
    { folderBrowser: null },
    { folderBrowser: { session: "s", path: "/a", depth: 0 } },
    { folderBrowser: { session: "s", path: 7, depth: 1 } },
  ]) {
    assert.equal(folderBrowserHistoryEntry(broken), null);
  }
  assert.notEqual(newFolderBrowserSession(1), newFolderBrowserSession(1));
});

test("back and forward retrace the folders, and back past the start closes", async () => {
  const router = settingsRouter();
  const session = "open-1";
  const pageKey = router.state.location.key;
  await pushFolder(router, session, "/", 1);
  await pushFolder(router, session, "/media", 2);
  await pushFolder(router, session, "/media/Shows", 3);

  const popped: Array<ReturnType<typeof folderBrowserPopAction>> = [];
  const pop = async (delta: number) => {
    await step(router, delta);
    assert.equal(router.state.historyAction, "POP");
    assert.equal(router.state.location.pathname, "/settings/libraries");
    popped.push(folderBrowserPopAction(router.state.location.state, session));
  };
  await pop(-1);
  await pop(-1);
  await pop(1);
  await pop(-1);
  await pop(-1);
  assert.deepEqual(popped, [
    { kind: "browse", path: "/media", depth: 2 },
    { kind: "browse", path: "/", depth: 1 },
    { kind: "browse", path: "/media", depth: 2 },
    { kind: "browse", path: "/", depth: 1 },
    { kind: "close" },
  ]);
  assert.equal(router.state.location.key, pageKey);
  router.dispose();
});

test("closing steps back over every entry the browser added", async () => {
  const router = settingsRouter();
  const session = "open-2";
  const pageKey = router.state.location.key;
  await pushFolder(router, session, "/", 1);
  await pushFolder(router, session, "/media", 2);
  const depth = folderBrowserHistoryEntry(router.state.location.state)?.depth;
  assert.equal(depth, 2);
  await step(router, -(depth ?? 0));
  assert.equal(router.state.location.key, pageKey);
  assert.equal(router.state.location.pathname, "/settings/libraries");
  // One more back leaves the page, as it would have before the browser opened.
  await step(router, -1);
  assert.equal(router.state.location.pathname, "/movies");
  router.dispose();
});

test("an unsaved-changes guard holds leaving the page but not folder steps", async () => {
  const router = settingsRouter();
  const session = "open-3";
  router.getBlocker(
    "unsaved",
    ({ currentLocation, nextLocation }) =>
      !isFolderBrowserHistoryStep(currentLocation, nextLocation),
  );
  const blocker = () =>
    (router.state.blockers.get("unsaved") ?? IDLE_BLOCKER).state;

  await pushFolder(router, session, "/", 1);
  await pushFolder(router, session, "/media", 2);
  await step(router, -1);
  assert.equal(
    folderBrowserHistoryEntry(router.state.location.state)?.path,
    "/",
  );
  await step(router, -1);
  assert.equal(router.state.location.pathname, "/settings/libraries");
  assert.equal(folderBrowserHistoryEntry(router.state.location.state), null);
  assert.equal(blocker(), "unblocked");

  await router.navigate("/movies");
  assert.equal(blocker(), "blocked");
  assert.equal(router.state.location.pathname, "/settings/libraries");
  router.deleteBlocker("unsaved");
  router.dispose();
});

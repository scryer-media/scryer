import assert from "node:assert/strict";
import test from "node:test";

import { isolatePlexPopup } from "./plex-oauth.ts";

test("isolates the Plex popup before it can navigate", () => {
  let closed = 0;
  const popup = {
    opener: {} as Window,
    close: () => {
      closed += 1;
    },
  } as unknown as Window;

  isolatePlexPopup(popup);

  assert.equal(popup.opener, null);
  assert.equal(closed, 0);
});

test("closes the Plex popup when opener isolation is rejected", () => {
  let closed = 0;
  const popup = {
    close: () => {
      closed += 1;
    },
  } as Window;
  Object.defineProperty(popup, "opener", {
    get: () => window,
    set: () => {
      throw new Error("opener is immutable");
    },
  });

  assert.throws(() => isolatePlexPopup(popup), /Unable to isolate/);
  assert.equal(closed, 1);
});

import assert from "node:assert/strict";
import test from "node:test";

import {
  canSubmitJellyfinLink,
  effectiveEmbyLinkMode,
  selectableUsername,
  serverAccountOptions,
  shouldListServerAccounts,
} from "./external-account-link-gate.ts";

const ready = {
  connectionId: "jellyfin-main",
  username: "someone",
  busy: false,
};

test("Jellyfin linking requires a connection and username", () => {
  assert.equal(canSubmitJellyfinLink(ready), true);
  assert.equal(canSubmitJellyfinLink({ ...ready, connectionId: "" }), false);
  assert.equal(canSubmitJellyfinLink({ ...ready, connectionId: null }), false);
  assert.equal(canSubmitJellyfinLink({ ...ready, username: "" }), false);
  assert.equal(canSubmitJellyfinLink({ ...ready, username: "   " }), false);
  assert.equal(canSubmitJellyfinLink({ ...ready, busy: true }), false);
});

test("Jellyfin linking does not require a password", () => {
  // Regression guard for scryer-media/scryer#81: a passwordless Jellyfin
  // account could not be linked because the UI demanded a password.
  assert.equal(canSubmitJellyfinLink(ready), true);
});

test("Emby Connect link mode is coerced to local when the selected connection disables Connect", () => {
  assert.equal(effectiveEmbyLinkMode("CONNECT", false), "LOCAL");
  assert.equal(effectiveEmbyLinkMode("CONNECT", true), "CONNECT");
  assert.equal(effectiveEmbyLinkMode("LOCAL", true), "LOCAL");
});

test("server accounts are listed only for connection managers on a username link", () => {
  const draft = {
    provider: "JELLYFIN" as const,
    embyMode: "LOCAL" as const,
    connectionId: "jellyfin-emberfall",
    canManageConnections: true,
  };
  assert.equal(shouldListServerAccounts(draft), true);
  assert.equal(
    shouldListServerAccounts({ ...draft, canManageConnections: false }),
    false,
  );
  assert.equal(shouldListServerAccounts({ ...draft, connectionId: "" }), false);
  assert.equal(shouldListServerAccounts({ ...draft, provider: "PLEX" }), false);
  assert.equal(shouldListServerAccounts({ ...draft, provider: null }), false);
  assert.equal(shouldListServerAccounts({ ...draft, provider: "EMBY" }), true);
  assert.equal(
    shouldListServerAccounts({
      ...draft,
      provider: "EMBY",
      embyMode: "CONNECT",
    }),
    false,
  );
});

test("server account options keep linked accounts visible but unpickable", () => {
  assert.deepEqual(
    serverAccountOptions([
      { username: "Ashgrove", linked: true },
      { username: " Cinderwake ", linked: false },
      { username: "  ", linked: false },
      { username: "Cinderwake", linked: false },
    ]),
    [
      { value: "Ashgrove", label: "Ashgrove", linked: true },
      { value: "Cinderwake", label: "Cinderwake", linked: false },
    ],
  );
});

test("the free-text field stays when the list failed or offers nothing to pick", () => {
  assert.equal(serverAccountOptions(null), null);
  assert.equal(serverAccountOptions([]), null);
  assert.equal(
    serverAccountOptions([{ username: "Ashgrove", linked: true }]),
    null,
  );
});

test("a typed username survives only when it is a pickable choice", () => {
  const options = serverAccountOptions([
    { username: "Ashgrove", linked: true },
    { username: "Emberfall", linked: false },
  ]);
  assert.equal(selectableUsername("Emberfall", options), "Emberfall");
  assert.equal(selectableUsername("Ashgrove", options), "");
  assert.equal(selectableUsername("Cinderwake", options), "");
  assert.equal(selectableUsername("Cinderwake", null), "Cinderwake");
});

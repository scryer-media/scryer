import assert from "node:assert/strict";
import test from "node:test";

import type { MediaServerConnectionDraft } from "../types/index.ts";
import { normalizeMediaServerConnectionDraft } from "./media-server-input.ts";

function draft(overrides: Partial<MediaServerConnectionDraft>): MediaServerConnectionDraft {
  return {
    provider: "JELLYFIN",
    displayName: "Living room",
    baseUrl: "",
    externalUrl: "",
    enabled: true,
    loginEnabled: false,
    linkingEnabled: false,
    autoAddEnabled: false,
    defaultAppPermissions: [],
    defaultLibraryGrants: [],
    machineIdPresent: false,
    plexServerId: "",
    apiKey: "",
    clearApiKey: false,
    jellyfinCredentialMode: "apiKey",
    embyConnectionMode: "LOCAL",
    embyLocalSetupMethod: "API_KEY",
    embyConnectEnabled: false,
    embyConnectUsernameOrEmail: "",
    embyConnectPassword: "",
    embyConnectServerId: "",
    embyDiscoveredServers: [],
    adminUsername: "",
    adminPassword: "",
    pathMappingsText: "",
    ...overrides,
  };
}

test("a media server address is taken as it was typed", () => {
  // A LAN address on the server's own port is plain HTTP.
  assert.equal(
    normalizeMediaServerConnectionDraft(draft({ baseUrl: "192.168.1.5:32400" })).baseUrl,
    "http://192.168.1.5:32400",
  );
  assert.equal(
    normalizeMediaServerConnectionDraft(draft({ baseUrl: "jellyfin.lan/" })).baseUrl,
    "http://jellyfin.lan",
  );
  // The public URL is usually a name on the internet.
  assert.equal(
    normalizeMediaServerConnectionDraft(draft({ externalUrl: "plex.example.com" }))
      .externalUrl,
    "https://plex.example.com",
  );
  // A scheme the operator wrote stands.
  assert.equal(
    normalizeMediaServerConnectionDraft(draft({ baseUrl: "http://plex.example.com/" }))
      .baseUrl,
    "http://plex.example.com",
  );
});

test("empty and already-clean addresses leave the draft untouched", () => {
  // Plex may have no local address at all; that must not become a URL.
  const plex = draft({ provider: "PLEX", baseUrl: "", externalUrl: "" });
  assert.equal(normalizeMediaServerConnectionDraft(plex), plex);
  const clean = draft({
    baseUrl: "https://jellyfin.example.com",
    externalUrl: "https://watch.example.com",
  });
  assert.equal(normalizeMediaServerConnectionDraft(clean), clean);
  // A value we should not touch is handed on as typed.
  const credentials = draft({ baseUrl: "http://admin:pw@jellyfin.lan:8096" });
  assert.equal(normalizeMediaServerConnectionDraft(credentials).baseUrl, credentials.baseUrl);
});

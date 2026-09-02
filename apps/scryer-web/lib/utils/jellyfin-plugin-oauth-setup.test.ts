import assert from "node:assert/strict";
import test from "node:test";

import {
  automaticLinkingStatus,
  canCreateJellyfinPluginClient,
  canStartJellyfinPluginClientCreation,
  createdJellyfinPluginClientForCallback,
  jellyfinPluginCallbackUrl,
  jellyfinPluginClientCreateDecision,
  normalizedPublicJellyfinBaseUrl,
  prefillJellyfinPublicBaseUrl,
  reconcileCreatedJellyfinPluginClient,
  shouldApplyJellyfinPluginOAuthReload,
  type JellyfinMediaServerConnection,
  type OAuthClientRegistrationForJellyfin,
} from "./jellyfin-plugin-oauth-setup.ts";

const BASE_URL = "https://jellyfin.example.test";
const CALLBACK_URL = `${BASE_URL}/Scryer/Auth/Callback`;

const eligibleConnection: JellyfinMediaServerConnection = {
  enabled: true,
  linkingEnabled: true,
  apiKeyPresent: true,
  externalUrl: BASE_URL,
};

const client = (
  clientId: string,
  overrides: Partial<OAuthClientRegistrationForJellyfin> = {},
): OAuthClientRegistrationForJellyfin => ({
  clientId,
  redirectUris: [CALLBACK_URL],
  enabled: true,
  source: "CUSTOM",
  ...overrides,
});

test("standalone Jellyfin OAuth setup is valid with zero media-server connections", () => {
  assert.equal(normalizedPublicJellyfinBaseUrl(`${BASE_URL}/`), BASE_URL);
  assert.equal(jellyfinPluginCallbackUrl(BASE_URL), CALLBACK_URL);
  assert.equal(prefillJellyfinPublicBaseUrl([]), null);
  assert.equal(automaticLinkingStatus(BASE_URL, []), "not-ready");
  assert.equal(jellyfinPluginClientCreateDecision([], CALLBACK_URL), "create");
});

test("public URL validation rejects non-HTTPS and credential-bearing callback sources", () => {
  for (const value of [
    "http://jellyfin.example.test",
    "https://operator:secret@jellyfin.example.test",
    "https://jellyfin.example.test/?ignored=value",
    "https://jellyfin.example.test/#fragment",
  ]) {
    assert.equal(normalizedPublicJellyfinBaseUrl(value), null, value);
  }
});

test("exactly one eligible connection pre-fills, while zero or multiple never do", () => {
  assert.equal(prefillJellyfinPublicBaseUrl([eligibleConnection]), BASE_URL);
  assert.equal(prefillJellyfinPublicBaseUrl([]), null);
  assert.equal(
    prefillJellyfinPublicBaseUrl([eligibleConnection, { ...eligibleConnection }]),
    null,
  );
  assert.equal(automaticLinkingStatus(BASE_URL, [eligibleConnection]), "ready");
  assert.equal(
    automaticLinkingStatus(BASE_URL, [eligibleConnection, { ...eligibleConnection }]),
    "ambiguous",
  );
});

test("disabled, non-linking, or uncredentialed connections never prefill or enable auto-linking", () => {
  for (const connection of [
    { ...eligibleConnection, enabled: false },
    { ...eligibleConnection, linkingEnabled: false },
    { ...eligibleConnection, apiKeyPresent: false },
  ]) {
    assert.equal(prefillJellyfinPublicBaseUrl([connection]), null);
    assert.equal(automaticLinkingStatus(BASE_URL, [connection]), "not-ready");
  }
});

test("multiple exact custom client matches are ambiguous and cannot be reused", () => {
  assert.equal(jellyfinPluginClientCreateDecision([client("first")], CALLBACK_URL), "reuse");
  assert.equal(
    jellyfinPluginClientCreateDecision([client("first"), client("second")], CALLBACK_URL),
    "ambiguous",
  );
});

test("disabled, managed, and multi-callback clients cannot satisfy the plugin callback", () => {
  for (const ineligibleClient of [
    client("disabled", { enabled: false }),
    client("managed", { source: "MANAGED" }),
    client("multi-callback", { redirectUris: [CALLBACK_URL, "https://other.example.test/callback"] }),
  ]) {
    assert.equal(
      jellyfinPluginClientCreateDecision([ineligibleClient], CALLBACK_URL),
      "create",
    );
  }
});

test("stale reloads and stale created clients cannot overwrite current OAuth setup", () => {
  assert.equal(shouldApplyJellyfinPluginOAuthReload(1, 2), false);
  assert.equal(shouldApplyJellyfinPluginOAuthReload(2, 2), true);
  assert.equal(
    reconcileCreatedJellyfinPluginClient(
      { clientId: "created", callbackUrl: CALLBACK_URL },
      [client("other")],
    ),
    null,
  );
  assert.deepEqual(
    reconcileCreatedJellyfinPluginClient(
      { clientId: "created", callbackUrl: CALLBACK_URL },
      [client("created")],
    ),
    { clientId: "created", callbackUrl: CALLBACK_URL },
  );
});

test("changing or invalidating the base URL clears the created-client reconciliation state", () => {
  const created = { clientId: "created", callbackUrl: CALLBACK_URL };
  assert.deepEqual(createdJellyfinPluginClientForCallback(created, CALLBACK_URL), created);
  assert.equal(
    createdJellyfinPluginClientForCallback(
      created,
      jellyfinPluginCallbackUrl("https://other-jellyfin.example.test"),
    ),
    null,
  );
  assert.equal(createdJellyfinPluginClientForCallback(created, null), null);
});

test("an in-flight create blocks a second create until the first operation releases it", () => {
  assert.equal(canStartJellyfinPluginClientCreation(null), true);
  assert.equal(canStartJellyfinPluginClientCreation(CALLBACK_URL), false);
  assert.equal(canStartJellyfinPluginClientCreation(null), true);
});

test("an unreconciled client blocks duplicate creation without blocking list recovery", () => {
  assert.equal(canCreateJellyfinPluginClient(false, "not-configured"), true);
  assert.equal(canCreateJellyfinPluginClient(false, "ready"), true);
  assert.equal(canCreateJellyfinPluginClient(false, "reconciling"), false);
  assert.equal(canCreateJellyfinPluginClient(false, "ambiguous"), false);
  assert.equal(canCreateJellyfinPluginClient(true, "not-configured"), false);
});

import assert from "node:assert/strict";
import test from "node:test";

import {
  automaticLinkingStatus,
  canCreateJellyfinPluginClient,
  canStartJellyfinPluginClientCreation,
  createdJellyfinPluginClientForCallback,
  isJellyfinPluginClientRegistration,
  jellyfinConnectionIneligibilityReasons,
  jellyfinPluginCallbackUrl,
  jellyfinPluginClientRegistrations,
  jellyfinPluginClientStatus,
  jellyfinPluginClientCreateDecision,
  jellyfinPluginCreateNeedsReconciliation,
  jellyfinPublicBaseUrlFromCallback,
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
  displayName: "Jellyfin",
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
  kind: "JELLYFIN_PLUGIN",
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

test("an ineligible connection still prefills so its reason is visible in settings", () => {
  for (const connection of [
    { ...eligibleConnection, enabled: false },
    { ...eligibleConnection, linkingEnabled: false },
    { ...eligibleConnection, apiKeyPresent: false },
  ]) {
    assert.equal(prefillJellyfinPublicBaseUrl([connection]), BASE_URL);
    assert.equal(automaticLinkingStatus(BASE_URL, [connection]), "not-ready");
  }
  assert.equal(prefillJellyfinPublicBaseUrl([{ ...eligibleConnection, externalUrl: null }]), null);
  assert.equal(
    prefillJellyfinPublicBaseUrl([{ ...eligibleConnection, externalUrl: "http://insecure.test" }]),
    null,
  );
});

test("ineligibility reasons name the failing condition of every connection on that URL", () => {
  assert.deepEqual(jellyfinConnectionIneligibilityReasons(BASE_URL, [eligibleConnection]), []);
  assert.deepEqual(
    jellyfinConnectionIneligibilityReasons(BASE_URL, [
      { ...eligibleConnection, displayName: "Off", enabled: false },
      { ...eligibleConnection, displayName: "Unlinked", linkingEnabled: false },
      { ...eligibleConnection, displayName: "Keyless", apiKeyPresent: false },
    ]),
    [
      { displayName: "Off", reason: "is disabled" },
      { displayName: "Unlinked", reason: "has account linking disabled" },
      { displayName: "Keyless", reason: "has no API key" },
    ],
  );
  // The first failing condition wins, matching the backend's diagnostic order.
  assert.deepEqual(
    jellyfinConnectionIneligibilityReasons(BASE_URL, [
      { ...eligibleConnection, enabled: false, linkingEnabled: false, apiKeyPresent: false },
    ]),
    [{ displayName: "Jellyfin", reason: "is disabled" }],
  );
  // A connection on another public URL is not this section's problem.
  assert.deepEqual(
    jellyfinConnectionIneligibilityReasons(BASE_URL, [
      { ...eligibleConnection, enabled: false, externalUrl: "https://other.example.test" },
    ]),
    [],
  );
  assert.deepEqual(jellyfinConnectionIneligibilityReasons(null, [eligibleConnection]), []);
  assert.deepEqual(jellyfinConnectionIneligibilityReasons(BASE_URL, null), []);
});

test("a plugin client is recognized by its stored kind, never by its callback shape", () => {
  assert.equal(jellyfinPublicBaseUrlFromCallback(CALLBACK_URL), BASE_URL);
  assert.equal(jellyfinPublicBaseUrlFromCallback(`${BASE_URL}/other`), null);
  assert.equal(jellyfinPublicBaseUrlFromCallback("http://insecure.test/Scryer/Auth/Callback"), null);

  assert.equal(isJellyfinPluginClientRegistration(client("plugin")), true);
  // A disabled plugin client is still the plugin client; it belongs in the Jellyfin section
  // rather than dropping into the generic custom-application list.
  assert.equal(isJellyfinPluginClientRegistration(client("disabled", { enabled: false })), true);
  // A plugin-shaped callback on a plain custom client proves nothing.
  assert.equal(isJellyfinPluginClientRegistration(client("look-alike", { kind: "CUSTOM" })), false);
  assert.equal(
    isJellyfinPluginClientRegistration(client("managed", { source: "MANAGED", kind: "CUSTOM" })),
    false,
  );

  assert.deepEqual(
    jellyfinPluginClientRegistrations([
      client("plugin"),
      client("look-alike", { kind: "CUSTOM" }),
      client("no-callback", { redirectUris: [] }),
      client("odd-callback", { redirectUris: ["https://service.example.test/oauth/callback"] }),
    ]).map((registeredClient) => [
      registeredClient.clientId,
      registeredClient.callbackUrl,
      registeredClient.publicBaseUrl,
    ]),
    [
      ["plugin", CALLBACK_URL, BASE_URL],
      ["no-callback", null, null],
      ["odd-callback", "https://service.example.test/oauth/callback", null],
    ],
  );
});

test("multiple exact custom client matches are ambiguous and cannot be reused", () => {
  assert.equal(jellyfinPluginClientCreateDecision([client("first")], CALLBACK_URL), "reuse");
  assert.equal(
    jellyfinPluginClientCreateDecision([client("first"), client("second")], CALLBACK_URL),
    "ambiguous",
  );
});

test("only an enabled plugin-kind client registered for the callback can be reused", () => {
  for (const ineligibleClient of [
    client("disabled", { enabled: false }),
    client("look-alike", { kind: "CUSTOM" }),
    client("other-callback", { redirectUris: ["https://other.example.test/callback"] }),
  ]) {
    assert.equal(
      jellyfinPluginClientCreateDecision([ineligibleClient], CALLBACK_URL),
      "create",
    );
  }
  // A plugin client that also lists other callbacks still serves this one.
  assert.equal(
    jellyfinPluginClientCreateDecision(
      [client("multi-callback", {
        redirectUris: [CALLBACK_URL, "https://other.example.test/callback"],
      })],
      CALLBACK_URL,
    ),
    "reuse",
  );
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

test("an uncertain create outcome stays reconciling until the client list resolves it", () => {
  assert.equal(jellyfinPluginCreateNeedsReconciliation(undefined), true);
  assert.equal(jellyfinPluginCreateNeedsReconciliation(null), true);
  assert.equal(jellyfinPluginCreateNeedsReconciliation({ clientId: "created" }), false);
  assert.equal(jellyfinPluginClientStatus(0, false, true), "reconciling");
  assert.equal(canCreateJellyfinPluginClient(false, "reconciling"), false);
  assert.equal(jellyfinPluginClientStatus(1, false, true), "ready");
  assert.equal(jellyfinPluginClientStatus(2, false, true), "ambiguous");
  assert.equal(jellyfinPluginClientStatus(0, false, false), "not-configured");
});

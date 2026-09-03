import assert from "node:assert/strict";
import test from "node:test";
import {
  isOAuthAuthenticationError,
  isReplayablePendingOAuthDecision,
  oauthAuthorizationRequestFingerprint,
  oauthAuthorizationRequestFromSearch,
  pendingOAuthDecisionFor,
  PENDING_OAUTH_DECISION_TTL_MS,
} from "./oauth-authorization-request.ts";

const AUTHORIZATION_SEARCH =
  "?response_type=code&client_id=client-a&redirect_uri=https%3A%2F%2Fjellyfin.example%2Fcallback&code_challenge=challenge&code_challenge_method=S256&scope=library+jellyfin-link&state=state-a";

test("OAuth authorization request parsing reads the current query values", () => {
  assert.deepEqual(
    oauthAuthorizationRequestFromSearch(AUTHORIZATION_SEARCH),
    {
      responseType: "code",
      clientId: "client-a",
      redirectUri: "https://jellyfin.example/callback",
      codeChallenge: "challenge",
      codeChallengeMethod: "S256",
      scope: "library jellyfin-link",
      state: "state-a",
    },
  );
});

test("OAuth authentication errors are distinct from transient preview failures", () => {
  assert.equal(isOAuthAuthenticationError({ response: { status: 401 } }), true);
  assert.equal(
    isOAuthAuthenticationError({ graphQLErrors: [{ extensions: { code: "UNAUTHORIZED" } }] }),
    true,
  );
  assert.equal(isOAuthAuthenticationError({ response: { status: 503 } }), false);
  assert.equal(isOAuthAuthenticationError(new Error("offline")), false);
});

test("a pending approval is bound to every parameter of the request it was given for", async () => {
  const request = oauthAuthorizationRequestFromSearch(AUTHORIZATION_SEARCH);
  const fingerprint = await oauthAuthorizationRequestFingerprint(request);

  assert.match(fingerprint, /^[a-f0-9]{64}$/);
  assert.equal(fingerprint.includes("state-a"), false);
  assert.equal(fingerprint.includes("challenge"), false);

  for (const changedSearch of [
    AUTHORIZATION_SEARCH.replace("client_id=client-a", "client_id=client-b"),
    AUTHORIZATION_SEARCH.replace("state=state-a", "state=state-b"),
    AUTHORIZATION_SEARCH.replace("scope=library+jellyfin-link", "scope=library"),
    AUTHORIZATION_SEARCH.replace("code_challenge=challenge", "code_challenge=other"),
    AUTHORIZATION_SEARCH.replace("code_challenge_method=S256", "code_challenge_method=plain"),
    AUTHORIZATION_SEARCH.replace("response_type=code", "response_type=token"),
    AUTHORIZATION_SEARCH.replace(
      "redirect_uri=https%3A%2F%2Fjellyfin.example%2Fcallback",
      "redirect_uri=https%3A%2F%2Fevil.example%2Fcallback",
    ),
  ]) {
    assert.notEqual(
      await oauthAuthorizationRequestFingerprint(
        oauthAuthorizationRequestFromSearch(changedSearch),
      ),
      fingerprint,
      changedSearch,
    );
  }
});

test("a stored approval replays only while it is unexpired, approved, and an exact match", async () => {
  const request = oauthAuthorizationRequestFromSearch(AUTHORIZATION_SEARCH);
  const fingerprint = await oauthAuthorizationRequestFingerprint(request);
  const decision = await pendingOAuthDecisionFor(request, 1_000);

  assert.equal(decision.expiresAt, 1_000 + PENDING_OAUTH_DECISION_TTL_MS);
  assert.equal(isReplayablePendingOAuthDecision(decision, fingerprint, 1_000), true);
  assert.equal(
    isReplayablePendingOAuthDecision(decision, fingerprint, decision.expiresAt),
    false,
  );
  assert.equal(isReplayablePendingOAuthDecision(decision, "other-request", 1_000), false);
  assert.equal(
    isReplayablePendingOAuthDecision({ ...decision, approved: false }, fingerprint, 1_000),
    false,
  );
  assert.equal(isReplayablePendingOAuthDecision(null, fingerprint, 1_000), false);
  assert.equal(isReplayablePendingOAuthDecision("approved", fingerprint, 1_000), false);
});

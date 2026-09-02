import assert from "node:assert/strict";
import test from "node:test";
import {
  isOAuthAuthenticationError,
  oauthAuthorizationRequestFromSearch,
} from "./oauth-authorization-request.ts";

test("OAuth authorization request parsing reads the current query values", () => {
  assert.deepEqual(
    oauthAuthorizationRequestFromSearch(
      "?response_type=code&client_id=client-a&redirect_uri=https%3A%2F%2Fjellyfin.example%2Fcallback&code_challenge=challenge&code_challenge_method=S256&scope=library+jellyfin-link&state=state-a",
    ),
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

import assert from "node:assert/strict";
import test from "node:test";
import { createOAuthClientRegistrationMutation } from "./mutations.ts";

test("OAuth client creation aliases the schema field to the UI response key", () => {
  assert.match(
    createOAuthClientRegistrationMutation,
    /createOAuthClientRegistration: createOauthClientRegistration\(input: \$input\)/,
  );
});

import assert from "node:assert/strict";
import test from "node:test";

import {
  consumeReauthenticatedAction,
  dismissReauthenticationAction,
  queueReauthenticationAction,
} from "./security-factor-reauthentication.ts";

test("pending actions wait for a later reauthentication generation", () => {
  const pending = queueReauthenticationAction({ kind: "add-passkey" }, 4);

  assert.deepEqual(consumeReauthenticatedAction(pending, 4), {
    pending,
    action: null,
  });
  assert.deepEqual(consumeReauthenticatedAction(pending, 5), {
    pending: null,
    action: { kind: "add-passkey" },
  });
});

test("a consumed action cannot be reused by a later generation", () => {
  const pending = queueReauthenticationAction({ kind: "start-totp-enrollment" }, 1);
  const resumed = consumeReauthenticatedAction(pending, 2);

  assert.deepEqual(consumeReauthenticatedAction(resumed.pending, 3), {
    pending: null,
    action: null,
  });
});

test("dismissing reauthentication clears the pending action", () => {
  assert.equal(dismissReauthenticationAction(), null);
});

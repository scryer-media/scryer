import assert from "node:assert/strict";
import test from "node:test";

import { shouldLoadProfilePasskeys } from "./profile-passkey-gate.ts";

const enabled = {
  authLoading: false,
  effectiveFormLoginEnabled: true,
  passkeyEnabled: true,
  userId: "user-1",
};

test("Passkey loading waits for enabled form authentication", () => {
  assert.equal(shouldLoadProfilePasskeys(enabled), true);
  assert.equal(
    shouldLoadProfilePasskeys({ ...enabled, authLoading: true }),
    false,
  );
  assert.equal(
    shouldLoadProfilePasskeys({ ...enabled, effectiveFormLoginEnabled: false }),
    false,
  );
  assert.equal(
    shouldLoadProfilePasskeys({ ...enabled, passkeyEnabled: false }),
    false,
  );
});

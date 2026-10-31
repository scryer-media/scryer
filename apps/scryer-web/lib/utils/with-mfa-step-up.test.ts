import assert from "node:assert/strict";
import test from "node:test";
import { withMfaStepUp } from "./with-mfa-step-up.ts";

const challenge = {
  error: { graphQLErrors: [{ extensions: { code: "MFA_STEP_UP_REQUIRED" } }] },
};

test("successful operations do not prompt for MFA or run twice", async () => {
  let calls = 0;
  const result = await withMfaStepUp(
    async () => {
      calls++;
      return { error: undefined, data: "created" };
    },
    async () => {
      assert.fail("unexpected verification");
    },
  );
  assert.equal(result?.data, "created");
  assert.equal(calls, 1);
});

test("MFA challenge waits for verification and retries with the verified token", async () => {
  const tokens: (string | undefined)[] = [];
  let complete: (token: string) => void = () =>
    assert.fail("verification not requested");
  const verified = new Promise<string>((resolve) => {
    complete = resolve;
  });
  const result = withMfaStepUp(
    async (token) => {
      tokens.push(token);
      return token ? { error: undefined } : challenge;
    },
    () => verified,
  );
  await Promise.resolve();
  assert.deepEqual(tokens, [undefined]);
  complete("verified-session");
  assert.deepEqual(await result, { error: undefined });
  assert.deepEqual(tokens, [undefined, "verified-session"]);
});

test("cancelling MFA never retries the protected operation", async () => {
  let calls = 0;
  assert.equal(
    await withMfaStepUp(
      async () => {
        calls++;
        return challenge;
      },
      async () => null,
    ),
    null,
  );
  assert.equal(calls, 1);
});

test("ordinary errors do not open MFA or retry", async () => {
  const failure = { error: new Error("network unavailable") };
  assert.equal(
    await withMfaStepUp(
      async () => failure,
      async () => assert.fail("unexpected verification"),
    ),
    failure,
  );
});

test("a repeated challenge is returned without a retry loop", async () => {
  let calls = 0;
  let prompts = 0;
  assert.equal(
    await withMfaStepUp(
      async () => {
        calls++;
        return challenge;
      },
      async () => {
        prompts++;
        return "verified-session";
      },
    ),
    challenge,
  );
  assert.equal(calls, 2);
  assert.equal(prompts, 1);
});

test("failed verification never retries key creation", async () => {
  let calls = 0;
  await assert.rejects(
    withMfaStepUp(
      async () => {
        calls++;
        return challenge;
      },
      async () => {
        throw new Error("verification failed");
      },
    ),
    /verification failed/,
  );
  assert.equal(calls, 1);
});

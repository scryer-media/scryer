import assert from "node:assert/strict";
import test from "node:test";
import {
  captureListAccountReturn,
  consumeListAccountReturn,
  finishCurrentListAccountLink,
  LIST_ACCOUNT_LINK_TYPE,
  LIST_ACCOUNT_RELAY_ORIGIN,
  listAccountCompletionInput,
  LIST_ACCOUNT_POLL_INTERVAL_MS,
  listAccountPollRetryDelay,
  listAccountPollStep,
  listAccountReturnMatches,
  parseListAccountPayload,
  parsePendingListAccountLink,
  validateListAccountMessage,
  type PendingListAccountLink,
} from "./list-account-link.ts";

const origin = "https://instance.example";
const pending: PendingListAccountLink = {
  sessionId: "pending-session",
  state: "signed-state",
  provider: "trakt",
  authorizationOrigin: LIST_ACCOUNT_RELAY_ORIGIN,
  expiresAt: "2040-01-01T00:00:00Z",
};
const payload = { type: LIST_ACCOUNT_LINK_TYPE, provider: "trakt", state: pending.state, exchange_code: "single-use-receipt" } as const;

test("account-link messages require the pending popup, exact origin, provider, state and unconsumed session", () => {
  const popup = {};
  const event = { origin: LIST_ACCOUNT_RELAY_ORIGIN, source: popup, data: payload };
  assert.deepEqual(validateListAccountMessage(event, pending, popup, origin, false), payload);
  assert.equal(validateListAccountMessage({ ...event, source: {} }, pending, popup, origin, false), null);
  assert.equal(validateListAccountMessage({ ...event, origin: `${LIST_ACCOUNT_RELAY_ORIGIN}.invalid` }, pending, popup, origin, false), null);
  assert.equal(validateListAccountMessage({ ...event, data: { ...payload, state: "other-state" } }, pending, popup, origin, false), null);
  assert.equal(validateListAccountMessage({ ...event, data: { ...payload, provider: "anilist" } }, pending, popup, origin, false), null);
  assert.equal(validateListAccountMessage(event, pending, popup, origin, true), null);
  assert.equal(validateListAccountMessage(event, pending, popup, origin, false, Date.parse(pending.expiresAt)), null);
  assert.equal(validateListAccountMessage(event, pending, null, origin, false), null);
  assert.deepEqual(validateListAccountMessage({ ...event, origin }, pending, popup, origin, false), payload);
});

test("callbacks accept only code receipts, with exact Simkl issuer and unambiguous errors", () => {
  assert.equal(parseListAccountPayload({ ...payload, access_token: "secret" }), null);
  assert.equal(parseListAccountPayload({ ...payload, code: "other" }), null);
  assert.equal(parseListAccountPayload({ ...payload, error: "denied" }), null);
  assert.equal(parseListAccountPayload({ ...payload, provider: "simkl" }), null);
  assert.equal(parseListAccountPayload({ ...payload, provider: "simkl", iss: "https://simkl.com.invalid" }), null);
  assert.ok(parseListAccountPayload({ ...payload, provider: "simkl", iss: "https://simkl.com" }));
  assert.ok(parseListAccountPayload({ type: LIST_ACCOUNT_LINK_TYPE, provider: "mal", state: "state", code: "one-use-code" }));
  assert.equal(parseListAccountPayload({ ...payload, exchange_code: "x".repeat(8193) }), null);
  assert.equal(parseListAccountPayload([]), null);
});

test("return fragments are erased before decoding and malformed or oversized content fails closed", () => {
  let erased = false;
  const history = { replaceState: (_state: unknown, _unused: string, path?: string | URL | null) => { assert.equal(path, "/lists/oauth/return"); erased = true; } };
  const location = { pathname: "/lists/oauth/return", search: "", hash: `#${encodeURIComponent(JSON.stringify(payload))}` };
  assert.deepEqual(consumeListAccountReturn(location, history, pending), payload);
  assert.ok(erased);
  assert.equal(consumeListAccountReturn({ ...location, hash: "#%ZZ" }, history, pending), null);
  assert.equal(consumeListAccountReturn({ ...location, hash: `#${"x".repeat(16001)}` }, history, pending), null);
});

test("capture erases material even when browser storage throws", () => {
  const order: string[] = [];
  const result = captureListAccountReturn(
    { origin, pathname: "/lists/oauth/return", search: "", hash: `#${encodeURIComponent(JSON.stringify(payload))}` },
    { replaceState: () => { order.push("erase"); } },
    () => { order.push("storage"); throw new Error("storage disabled"); },
  );
  assert.equal(order[0], "erase");
  assert.deepEqual(result.payload, payload);
  assert.equal(result.pending, null);
});

test("fallback navigation recovers only its own unexpired session and supplies a one-use code to the backend", () => {
  const restored = parsePendingListAccountLink(JSON.stringify(pending), origin);
  assert.deepEqual(restored, pending);
  const returned = captureListAccountReturn(
    { origin, pathname: "/lists/oauth/return", search: "", hash: `#${encodeURIComponent(JSON.stringify(payload))}` },
    { replaceState: () => undefined },
    () => JSON.stringify(pending),
  );
  assert.ok(listAccountReturnMatches(returned.payload, returned.pending));
  assert.deepEqual(listAccountCompletionInput(pending, payload), { sessionId: pending.sessionId, provider: "trakt", state: pending.state, code: "single-use-receipt", issuer: null });
  assert.equal(parsePendingListAccountLink(JSON.stringify({ ...pending, authorizationOrigin: "https://other.example" }), origin), null);
  assert.equal(parsePendingListAccountLink(JSON.stringify(pending), origin, Date.parse(pending.expiresAt)), null);
  assert.equal(listAccountReturnMatches({ ...payload, state: "other-state" }, restored), false);
});

test("BYO callbacks require a pending provider and reject repeated and token parameters", () => {
  const history = { replaceState: () => undefined };
  const location = { pathname: "/prefix/lists/oauth/callback", search: "?state=signed-state&code=provider-code", hash: "" };
  assert.deepEqual(consumeListAccountReturn(location, history, pending), { type: LIST_ACCOUNT_LINK_TYPE, provider: "trakt", state: pending.state, code: "provider-code" });
  assert.equal(consumeListAccountReturn(location, history, null), null);
  assert.equal(consumeListAccountReturn({ ...location, search: `${location.search}&state=other` }, history, pending), null);
  assert.equal(consumeListAccountReturn({ ...location, search: `${location.search}&access_token=secret` }, history, pending), null);
});

test("dropped requests and passing instance errors are retried; every other instance error ends polling", () => {
  const graphQl = (...codes: unknown[]) => ({ graphQLErrors: codes.map((code) => ({ message: "failed", extensions: { code } })) });
  const unavailable = listAccountPollStep("UNAVAILABLE");
  assert.ok(unavailable.kind === "wait");
  for (const reason of [
    { networkError: new Error("offline"), graphQLErrors: [] },
    { networkError: new Error("bad gateway") },
    graphQl("INTERNAL_ERROR"),
    graphQl("TEMPORARY_UNAVAILABLE"),
    graphQl("INTERNAL_ERROR", "TEMPORARY_UNAVAILABLE"),
  ]) {
    assert.equal(listAccountPollRetryDelay(reason), unavailable.delayMs);
  }
  for (const code of ["NOT_FOUND", "UNAUTHORIZED", "VALIDATION_ERROR", 42]) {
    assert.equal(listAccountPollRetryDelay(graphQl(code)), null);
  }
  assert.equal(listAccountPollRetryDelay(graphQl("INTERNAL_ERROR", "VALIDATION_ERROR")), null);
  assert.equal(listAccountPollRetryDelay({ networkError: new Error("partial"), ...graphQl("VALIDATION_ERROR") }), null);
  assert.equal(listAccountPollRetryDelay(new Error("thrown")), null);
  assert.equal(listAccountPollRetryDelay(null), null);
});

test("poll statuses keep the normal pace, slow down when unavailable, wait longest when rate-limited, and otherwise fail", () => {
  assert.deepEqual(listAccountPollStep("PENDING"), { kind: "wait", delayMs: LIST_ACCOUNT_POLL_INTERVAL_MS, retrying: false });
  assert.deepEqual(listAccountPollStep("BUSY"), { kind: "wait", delayMs: LIST_ACCOUNT_POLL_INTERVAL_MS, retrying: false });
  const unavailable = listAccountPollStep("UNAVAILABLE");
  const limited = listAccountPollStep("RATE_LIMITED");
  assert.ok(unavailable.kind === "wait" && unavailable.retrying && unavailable.delayMs > LIST_ACCOUNT_POLL_INTERVAL_MS);
  assert.ok(limited.kind === "wait" && limited.retrying && unavailable.kind === "wait" && limited.delayMs >= 30000 && limited.delayMs > unavailable.delayMs);
  for (const status of ["FAILED", "EXPIRED", "LINKED", "pending", "", undefined, 3]) {
    assert.deepEqual(listAccountPollStep(status), { kind: "failed" });
  }
});

test("cancelled account refreshes cannot finish or fail a replacement link", { timeout: 10000 }, async () => {
  for (const reject of [false, true]) {
    let current = true;
    let release!: () => void;
    let failRefresh!: (reason: unknown) => void;
    const effects: string[] = [];
    const refresh = new Promise<void>((resolve, rejectPromise) => {
      release = resolve;
      failRefresh = rejectPromise;
    });
    const completion = finishCurrentListAccountLink(
      () => current,
      async (isCurrent) => {
        await refresh;
        if (isCurrent()) effects.push("accounts");
      },
      () => effects.push("close-popup-and-clear-storage"),
      () => effects.push("replace-status"),
    );
    current = false;
    if (reject) failRefresh(new Error("old refresh failed"));
    else release();
    await completion;
    assert.deepEqual(effects, []);
  }
});

test("current account refresh completes once and reports its own failure", { timeout: 10000 }, async () => {
  const effects: string[] = [];
  await finishCurrentListAccountLink(() => true, async () => { effects.push("accounts"); }, () => effects.push("finish"), () => effects.push("fail"));
  assert.deepEqual(effects, ["accounts", "finish"]);
  await finishCurrentListAccountLink(() => true, async () => { throw new Error("current refresh failed"); }, () => effects.push("finish"), () => effects.push("fail"));
  assert.deepEqual(effects, ["accounts", "finish", "fail"]);
});

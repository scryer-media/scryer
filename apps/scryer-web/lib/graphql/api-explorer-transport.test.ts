import assert from "node:assert/strict";
import test from "node:test";
import { API_EXPLORER_MODE_HEADER, createApiExplorerTransport, explorerHeaders } from "./api-explorer-transport.ts";

test("editor headers cannot replace the authenticated identity or selected mode", () => {
  const headers = explorerHeaders({
    Authorization: "Bearer forged", "X-Scryer-Web-Client": "forged",
    [API_EXPLORER_MODE_HEADER]: "oauth", "X-Custom": "kept",
  }, "session", "api-key");
  assert.equal(headers.get("authorization"), "Bearer session");
  assert.equal(headers.get(API_EXPLORER_MODE_HEADER), "api-key");
  assert.equal(headers.get("x-scryer-web-client"), null);
  assert.equal(headers.get("x-custom"), "kept");
  assert.equal(explorerHeaders(headers, null, "oauth").get("authorization"), null);
});

test("HTTP calls use current authentication and disposal cancels previous mode operations", async () => {
  let token = "first-session";
  const requests: RequestInit[] = [];
  const transport = createApiExplorerTransport({
    mode: "api-key", url: "http://localhost/base/graphql",
    getToken: () => token, getProof: async () => null, onUnavailable: () => {},
    fetch: async (_input, init) => {
      requests.push(init!);
      return Response.json({ data: { __typename: "QueryRoot" } });
    },
  });
  await transport.fetcher({ query: "{ __typename }" });
  token = "second-session";
  await transport.fetcher({ query: "{ __typename }" }, { headers: { Authorization: "Bearer override", [API_EXPLORER_MODE_HEADER]: "oauth" } });
  assert.equal(new Headers(requests[0].headers).get("authorization"), "Bearer first-session");
  assert.equal(new Headers(requests[1].headers).get("authorization"), "Bearer second-session");
  assert.equal(new Headers(requests[1].headers).get(API_EXPLORER_MODE_HEADER), "api-key");
  assert.equal(requests[1].credentials, "include");
  transport.dispose();
  assert.equal(requests[0].signal?.aborted, true);
  assert.throws(() => transport.fetcher({ query: "{ __typename }" }), /abort/i);
});

test("API-key subscriptions are rejected locally and never sent over HTTP", async () => {
  let calls = 0;
  const transport = createApiExplorerTransport({
    mode: "api-key", url: "http://localhost/graphql",
    getToken: () => null, getProof: async () => null, onUnavailable: () => {},
    fetch: async () => { calls++; return Response.json({ data: {} }); },
  });
  const result = await transport.fetcher({ query: "subscription { events { id } }" });
  assert.match(JSON.stringify(result), /API keys do not support subscriptions/);
  assert.equal(calls, 0);
  transport.dispose();
});

test("backend explorer revocation refreshes eligibility, unrelated errors do not", async () => {
  let unavailable = 0;
  let errorCode = "OTHER_ERROR";
  const transport = createApiExplorerTransport({
    mode: "api-key", url: "http://localhost/graphql",
    getToken: () => null, getProof: async () => null, onUnavailable: () => { unavailable++; },
    fetch: async () => Response.json({ errors: [{ message: "denied", extensions: { code: errorCode } }] }, { status: 403 }),
  });
  await transport.fetcher({ query: "{ __typename }" });
  assert.equal(unavailable, 0);
  errorCode = "API_EXPLORER_UNAVAILABLE";
  await transport.fetcher({ query: "{ __typename }" });
  assert.equal(unavailable, 1);
  transport.dispose();
});

test("OAuth subscriptions use isolated connections and fixed session credentials", async (t) => {
  const sockets: FakeSocket[] = [];
  const messages: Array<{ type: string; id?: string; payload?: Record<string, unknown> }> = [];
  class FakeSocket {
    static CONNECTING = 0;
    static OPEN = 1;
    static CLOSING = 2;
    static CLOSED = 3;
    readyState = 0;
    url: string;
    onopen: (() => void) | null = null;
    onmessage: ((event: { data: string }) => void) | null = null;
    onclose: ((event: { code: number; reason: string; wasClean: boolean }) => void) | null = null;
    constructor(url: string) {
      this.url = url;
      sockets.push(this);
      queueMicrotask(() => { this.readyState = 1; this.onopen?.(); });
    }
    send(text: string) {
      const message = JSON.parse(text);
      messages.push(message);
      queueMicrotask(() => {
        if (message.type === "connection_init") {
          this.onmessage?.({ data: JSON.stringify({ type: "connection_ack" }) });
        } else if (message.type === "subscribe") {
          this.onmessage?.({ data: JSON.stringify({ type: "next", id: message.id, payload: { data: { event: "ready" } } }) });
        }
      });
    }
    close(code = 1000, reason = "") {
      this.readyState = 3;
      this.onclose?.({ code, reason, wasClean: true });
    }
  }
  const original = globalThis.WebSocket;
  Object.defineProperty(globalThis, "WebSocket", { value: FakeSocket, configurable: true, writable: true });
  t.after(() => { globalThis.WebSocket = original; });
  for (const token of ["real-session", null]) {
    const transport = createApiExplorerTransport({
      mode: "oauth", url: "https://localhost/scryer/graphql",
      getToken: () => token, getProof: async () => "validated-proof", onUnavailable: () => {},
      fetch: async () => { throw new Error("subscriptions must not use HTTP"); },
    });
    t.after(() => transport.dispose());
    const result = await transport.fetcher({ query: "subscription { event }" }, {
      headers: { Authorization: "Bearer forged", [API_EXPLORER_MODE_HEADER]: "api-key" },
    });
    assert.ok(result && Symbol.asyncIterator in result);
    const iterator = result[Symbol.asyncIterator]();
    assert.deepEqual((await iterator.next()).value, { data: { event: "ready" } });
    const payload = messages.findLast((message) => message.type === "connection_init")!.payload;
    assert.equal(payload?.[API_EXPLORER_MODE_HEADER], "oauth");
    assert.equal(payload?.Authorization, token ? `Bearer ${token}` : undefined);
    assert.equal(payload?.authlessWebClientProof, token ? undefined : "validated-proof");
    assert.equal(sockets.at(-1)?.url, "wss://localhost/scryer/graphql/ws");
    transport.dispose();
    await new Promise((resolve) => setImmediate(resolve));
    assert.equal(sockets.at(-1)?.readyState, FakeSocket.CLOSED);
    await iterator.return?.();
  }
  assert.equal(sockets.length, 2);
});

import assert from "node:assert/strict";
import test from "node:test";
import { createCatalogPageRequestGate } from "./catalog-page-request.ts";

test(
  "page failure suppresses repeated scroll events until explicit retry",
  { timeout: 30_000 },
  async () => {
    const gate = createCatalogPageRequestGate();
    let calls = 0;
    const request = async () => {
      calls++;
      throw new Error("RATE_LIMITED");
    };
    await assert.rejects(gate.run(request), /RATE_LIMITED/);
    await Promise.all(Array.from({ length: 100 }, () => gate.run(request)));
    assert.equal(calls, 1);
    gate.reset();
    assert.equal(
      await gate.run(async () => {
        calls++;
        return "page";
      }),
      "page",
    );
    assert.equal(calls, 2);
    assert.equal(await gate.run(async () => "next page"), "next page");
  },
);

test(
  "concurrent scroll callbacks dispatch only one pending page",
  { timeout: 30_000 },
  async () => {
    const gate = createCatalogPageRequestGate();
    const response = Promise.withResolvers<string>();
    const first = gate.run(() => response.promise);
    assert.equal(
      await gate.run(async () => {
        assert.fail("duplicate request");
      }),
      undefined,
    );
    response.resolve("page");
    assert.equal(await first, "page");
  },
);

test(
  "superseded failure does not pause a new catalog query or release its pending request",
  { timeout: 30_000 },
  async () => {
    const gate = createCatalogPageRequestGate();
    const oldResponse = Promise.withResolvers<string>();
    const old = gate.run(() => oldResponse.promise);
    const rejected = assert.rejects(old, /offline/);
    gate.reset();
    const newResponse = Promise.withResolvers<string>();
    const current = gate.run(() => newResponse.promise);
    oldResponse.reject(new Error("offline"));
    await rejected;
    assert.equal(gate.paused, false);
    assert.equal(
      await gate.run(async () => {
        assert.fail("duplicate current request");
      }),
      undefined,
    );
    newResponse.resolve("new page");
    assert.equal(await current, "new page");
  },
);

test("a failed explicit retry pauses again", { timeout: 30_000 }, async () => {
  const gate = createCatalogPageRequestGate();
  for (let attempt = 0; attempt < 2; attempt++) {
    gate.reset();
    await assert.rejects(
      gate.run(async () => {
        throw new Error("offline");
      }),
      /offline/,
    );
    assert.equal(gate.paused, true);
    assert.equal(
      await gate.run(async () => {
        assert.fail("automatic retry");
      }),
      undefined,
    );
  }
});

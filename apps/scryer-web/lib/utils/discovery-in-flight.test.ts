import assert from "node:assert/strict";
import test from "node:test";
import { DiscoveryInFlight } from "./discovery-in-flight.ts";

test("shares pending requests but never caches completed data", async () => {
  const requests = new DiscoveryInFlight<number>();
  const pending = Promise.withResolvers<number>();
  let calls = 0;
  const load = () => {
    calls++;
    return pending.promise;
  };
  const first = requests.run("scope", load);
  const second = requests.run("scope", load);
  assert.equal(first, second);
  pending.resolve(1);
  assert.equal(await first, 1);
  assert.equal(calls, 1);
  assert.equal(await requests.run("scope", async () => 2), 2);
});

test("scope invalidation cannot let an old completion evict new work", async () => {
  const requests = new DiscoveryInFlight<number>();
  const old = Promise.withResolvers<number>();
  const current = Promise.withResolvers<number>();
  const oldRequest = requests.run("scope", () => old.promise);
  requests.clear();
  const currentRequest = requests.run("scope", () => current.promise);
  old.resolve(1);
  await oldRequest;
  assert.equal(
    requests.run("scope", async () => 3),
    currentRequest,
  );
  current.resolve(2);
  assert.equal(await currentRequest, 2);
});

test("mutation refresh waits for pending transport work and reads again", async () => {
  for (const fails of [false, true]) {
    const requests = new DiscoveryInFlight<number>();
    const pending = Promise.withResolvers<number>();
    let settled = false;
    const first = requests.run("scope", () =>
      pending.promise.finally(() => {
        settled = true;
      }),
    );
    const firstResult = first.catch(() => undefined);
    const refreshed = requests.run(
      "scope",
      async () => {
        assert.equal(settled, true);
        return 2;
      },
      true,
    );
    assert.notEqual(first, refreshed);
    assert.equal(
      requests.run("scope", async () => 3),
      refreshed,
    );
    if (fails) pending.reject(new Error("old request failed"));
    else pending.resolve(1);
    assert.equal(await refreshed, 2);
    await firstResult;
  }
});

test("different scopes proceed independently and failures permit retry", async () => {
  const requests = new DiscoveryInFlight<number>();
  const blocked = Promise.withResolvers<number>();
  const waiting = requests.run("old", () => blocked.promise);
  assert.equal(await requests.run("new", async () => 2), 2);
  await assert.rejects(
    requests.run("failure", async () => {
      throw new Error("failed");
    }),
  );
  assert.equal(await requests.run("failure", async () => 3), 3);
  blocked.resolve(1);
  await waiting;
});

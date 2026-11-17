import assert from "node:assert/strict";
import { test } from "node:test";
import { slowLane, SLOW_LANE_WIDTH } from "./slow-lane.ts";

function controlledFetch() {
  const pending: Array<{ url: string; finish: () => void }> = [];
  const send = ((input: RequestInfo | URL) =>
    new Promise<Response>((resolve) => {
      pending.push({
        url: String(input),
        finish: () => resolve(new Response("{}")),
      });
    })) as typeof fetch;
  return { pending, send };
}

const settle = () => new Promise((resolve) => setImmediate(resolve));

test("only the lane width is in flight and the rest go out in order", async () => {
  const { pending, send } = controlledFetch();
  const lane = slowLane(send);
  const total = SLOW_LANE_WIDTH + 2;
  const requests = Array.from({ length: total }, (_, index) =>
    lane(`request-${index}`),
  );
  await settle();
  assert.equal(pending.length, SLOW_LANE_WIDTH);

  pending[0].finish();
  await requests[0];
  await settle();
  assert.equal(pending.length, SLOW_LANE_WIDTH + 1);
  assert.equal(pending.at(-1)?.url, `request-${SLOW_LANE_WIDTH}`);

  for (let index = 1; index < total; index++) {
    await settle();
    pending[index].finish();
    await requests[index];
  }
  assert.equal(pending.length, total);
});

test("a failed request frees its place", async () => {
  const lane = slowLane((() =>
    Promise.reject(new TypeError("offline"))) as typeof fetch);
  for (let index = 0; index < SLOW_LANE_WIDTH + 1; index++) {
    await assert.rejects(lane("request"), TypeError);
  }
});

test("a request cancelled while waiting is never sent", async () => {
  const { pending, send } = controlledFetch();
  const lane = slowLane(send);
  const running = Array.from({ length: SLOW_LANE_WIDTH }, (_, index) =>
    lane(`running-${index}`),
  );
  const controller = new AbortController();
  const cancelled = lane("cancelled", { signal: controller.signal });
  const next = lane("next");
  await settle();
  controller.abort();
  await assert.rejects(cancelled, { name: "AbortError" });

  pending[0].finish();
  await running[0];
  await settle();
  assert.deepEqual(
    pending.map((request) => request.url),
    [
      ...Array.from({ length: SLOW_LANE_WIDTH }, (_, index) => `running-${index}`),
      "next",
    ],
  );
  pending.slice(1).forEach((request) => request.finish());
  await Promise.all([...running, next]);
});

import assert from "node:assert/strict";
import { test } from "node:test";
import {
  combinePanelStates,
  createDashboardRefresh,
  initialDashboardPanelStates,
} from "./dashboard-refresh.ts";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

test("panels publish independently and coalesce stale work into one trailing refresh", async () => {
  const states = initialDashboardPanelStates();
  const refresh = createDashboardRefresh((key, patch) =>
    Object.assign(states[key], patch),
  );
  const first = deferred<() => void>();
  const started = deferred<void>();
  const published: string[] = [];
  let loads = 0;
  const pending = refresh.run("recent", async () => {
    loads += 1;
    started.resolve();
    return first.promise;
  });
  await started.promise;
  await refresh.run("overview", async () => () => {
    published.push("overview");
  });
  assert.equal(states.overview.ready, true);
  assert.equal(states.recent.ready, false);
  const reload = async () => {
    loads += 1;
    return () => {
      published.push("fresh");
    };
  };
  assert.equal(refresh.run("recent", reload, true), pending);
  assert.equal(refresh.run("recent", reload, true), pending);
  first.resolve(() => {
    published.push("stale");
  });
  await pending;
  assert.equal(loads, 2);
  assert.deepEqual(published, ["overview", "fresh"]);
  assert.equal(states.recent.loading, false);
});

test("errors preserve ready content and disposed requests cannot publish", async () => {
  const states = initialDashboardPanelStates();
  const refresh = createDashboardRefresh((key, patch) =>
    Object.assign(states[key], patch),
  );
  await refresh.run("storage", async () => () => {});
  await refresh.run("storage", async () => {
    throw new Error("offline");
  });
  assert.equal(states.storage.ready, true);
  assert.equal(states.storage.error, "offline");
  const old = deferred<() => void>();
  const started = deferred<void>();
  let published = false;
  const pending = refresh.run("recent", async () => {
    started.resolve();
    return old.promise;
  });
  await started.promise;
  refresh.dispose();
  refresh.activate();
  await refresh.run("recent", async () => () => {});
  old.resolve(() => {
    published = true;
  });
  await pending;
  assert.equal(published, false);
  assert.equal(states.recent.ready, true);
});

test("overlapping polls do not starve a slow panel", async () => {
  const refresh = createDashboardRefresh(() => {});
  const response = deferred<() => void>();
  const started = deferred<void>();
  let published = false;
  const pending = refresh.run("recent", async () => {
    started.resolve();
    return response.promise;
  });
  await started.promise;
  for (let i = 0; i < 3; i += 1) {
    assert.equal(
      refresh.run("recent", async () => {
        assert.fail("poll started another request");
      }),
      pending,
    );
  }
  response.resolve(() => {
    published = true;
  });
  await pending;
  assert.equal(published, true);
});

test("a panel fed by two loads waits for both and shows the first error", () => {
  const ready = { loading: false, ready: true, error: null };
  const loading = { loading: true, ready: false, error: null };
  const failed = { loading: false, ready: false, error: "queue failed" };
  assert.deepEqual(combinePanelStates(ready, loading), { loading: true, ready: false, error: null });
  assert.deepEqual(combinePanelStates(ready, failed), { loading: false, ready: false, error: "queue failed" });
  assert.deepEqual(combinePanelStates(ready, ready), { loading: false, ready: true, error: null });
});

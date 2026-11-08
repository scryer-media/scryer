import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import ts from "typescript";
import type { Client } from "urql";
import type {
  InteractiveSearchProgress,
  runIterativeReleaseSearch,
} from "./release-search.ts";
import { boundedQuery, QueryTimeoutError } from "./bounded-query.ts";

function runner(
  onSignal?: (signal: AbortSignal) => void,
): typeof runIterativeReleaseSearch {
  const source = readFileSync(
    new URL("./release-search.ts", import.meta.url),
    "utf8",
  );
  const code = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS },
  }).outputText;
  const dependencies: Record<string, unknown> = {
    "./mutations": {
      startInteractiveReleaseSearchMutation: "start",
      continueInteractiveReleaseSearchMutation: "continue",
      cancelInteractiveReleaseSearchMutation: "cancel",
    },
    "./queries": { interactiveReleaseSearchQuery: "poll" },
    "./release-search-input": {},
    "./urql-client": {
      isAbortError: () => false,
      makeAbortableFetch: (signal: AbortSignal) => {
        onSignal?.(signal);
        return undefined;
      },
    },
    "./bounded-query": { boundedQuery, QueryTimeoutError },
  };
  const module = {
    exports: {} as {
      runIterativeReleaseSearch: typeof runIterativeReleaseSearch;
    },
  };
  new Function("require", "module", "exports", "window", code)(
    (id: string) => {
      assert.ok(id in dependencies, `unexpected dependency ${id}`);
      return dependencies[id];
    },
    module,
    module.exports,
    { setTimeout, clearTimeout },
  );
  return module.exports.runIterativeReleaseSearch;
}

function operation<T>(value: T) {
  return {
    toPromise: async () => value,
    subscribe(next: (result: T) => void) {
      next(value);
      return { unsubscribe() {} };
    },
  };
}

test("raw continuation forwards the opaque cursor and keeps aggregate size separate", async () => {
  const calls: unknown[] = [];
  const updates: InteractiveSearchProgress[] = [];
  const result = {
    title: "software",
    source: "json-source",
    link: null,
    downloadUrl: null,
    sizeBytes: null,
    publishedAt: null,
  };
  const client = {
    mutation(document: string, variables: unknown) {
      calls.push({ document, variables });
      return operation({
        data: {
          continueInteractiveReleaseSearch: {
            id: "generation",
            state: "COMPLETED",
            results: [result],
            indexers: [],
            nextCursor: "next-opaque-cursor",
            deadlineReached: true,
            failureReasonCode: null,
          },
        },
      });
    },
  } as unknown as Client;
  const rows = await runner()(
    client,
    { query: "ignored by continuation", limit: 400, aggregatePageSize: 100 },
    {
      after: "opaque-cursor",
      onUpdate: (update) => updates.push(update),
    },
  );
  assert.deepEqual(calls, [
    { document: "continue", variables: { after: "opaque-cursor", first: 100 } },
  ]);
  assert.deepEqual(rows, [result]);
  assert.equal(updates[0].nextCursor, "next-opaque-cursor");
  assert.equal(updates[0].deadlineReached, true);
});

test("an initial raw request forwards both page sizes and preserves coded failures", async () => {
  const calls: unknown[] = [];
  const updates: InteractiveSearchProgress[] = [];
  const input = {
    query: "custom",
    categories: ["100001"],
    limit: 400,
    aggregatePageSize: 100,
  };
  const client = {
    mutation(document: string, variables: unknown) {
      calls.push({ document, variables });
      return operation({
        data: {
          startInteractiveReleaseSearch: {
            id: "generation",
            state: "COMPLETED",
            results: [],
            indexers: [],
            nextCursor: null,
            deadlineReached: false,
            failureReasonCode: "RAW_SEARCH_ALL_SOURCES_LIMITED",
          },
        },
      });
    },
  } as unknown as Client;
  await runner()(client, input, { onUpdate: (update) => updates.push(update) });
  assert.deepEqual(calls, [{ document: "start", variables: { input } }]);
  assert.equal(updates[0].failureReasonCode, "RAW_SEARCH_ALL_SOURCES_LIMITED");
  assert.equal(updates[0].nextCursor, null);
});

test(
  "an evicted session ends progress and retains streamed results",
  { timeout: 10_000 },
  async (t) => {
    t.mock.timers.enable({ apis: ["setTimeout", "Date"], now: 0 });
    const started = Promise.withResolvers<void>();
    const updates: InteractiveSearchProgress[] = [];
    const failures: unknown[] = [];
    const result = {
      title: "Example Movie",
      source: "fixture",
      downloadUrl: "https://fixture.invalid/release",
    };
    const client = {
      mutation() {
        return operation({
          data: {
            startInteractiveReleaseSearch: {
              id: "session-1",
              state: "RUNNING",
              results: [result],
              indexers: [],
            },
          },
        });
      },
      query() {
        return operation({ data: { interactiveReleaseSearch: null } });
      },
    } as unknown as Client;
    const request = runner()(
      client,
      { titleId: "movie-1" },
      {
        onUpdate(snapshot) {
          updates.push(snapshot);
          if (snapshot.state === "RUNNING") started.resolve();
        },
        onError: (error) => failures.push(error),
      },
    );
    await started.promise;
    t.mock.timers.tick(1_000);
    assert.deepEqual(await request, [result]);
    assert.equal(updates.at(-1)?.state, "CANCELLED");
    assert.equal(failures.length, 1);
  },
);

test(
  "a hung poll is bounded and releases the subscription",
  { timeout: 10_000 },
  async (t) => {
    t.mock.timers.enable({ apis: ["setTimeout", "Date"], now: 0 });
    const started = Promise.withResolvers<void>();
    const polling = Promise.withResolvers<void>();
    const updates: InteractiveSearchProgress[] = [];
    const failures: unknown[] = [];
    const partial = { title: "Partial result", source: "fixture" };
    let unsubscribed = 0;
    let fetchSignal: AbortSignal | undefined;
    const client = {
      mutation(document: string) {
        return operation({
          data: {
            startInteractiveReleaseSearch: {
              id: "session-2",
              state: document === "cancel" ? "CANCELLED" : "RUNNING",
              results: [partial],
              indexers: [],
            },
          },
        });
      },
      query() {
        polling.resolve();
        return {
          toPromise: () => new Promise(() => {}),
          subscribe() {
            return {
              unsubscribe() {
                unsubscribed++;
              },
            };
          },
        };
      },
    } as unknown as Client;
    const request = runner((signal) => {
      fetchSignal = signal;
    })(
      client,
      { titleId: "movie-2" },
      {
        onUpdate(snapshot) {
          updates.push(snapshot);
          if (snapshot.state === "RUNNING") started.resolve();
        },
        onError: (error) => failures.push(error),
      },
    );
    await started.promise;
    t.mock.timers.tick(1_000);
    await polling.promise;
    t.mock.timers.tick(90_000);
    assert.deepEqual(await request, [partial]);
    assert.equal(unsubscribed, 1);
    assert.equal(updates.at(-1)?.state, "CANCELLED");
    assert.ok(failures[0] instanceof QueryTimeoutError);
    assert.equal(fetchSignal?.aborted, true);
  },
);

test(
  "a hung start rejects at the same client deadline",
  { timeout: 10_000 },
  async (t) => {
    t.mock.timers.enable({ apis: ["setTimeout", "Date"], now: 0 });
    let unsubscribed = 0;
    let fetchSignal: AbortSignal | undefined;
    const client = {
      mutation() {
        return {
          subscribe() {
            return {
              unsubscribe() {
                unsubscribed++;
              },
            };
          },
        };
      },
    } as unknown as Client;
    const request = runner((signal) => {
      fetchSignal = signal;
    })(client, { titleId: "movie-start" });
    const rejected = assert.rejects(request, QueryTimeoutError);
    t.mock.timers.tick(90_000);
    await rejected;
    assert.equal(unsubscribed, 1);
    assert.equal(fetchSignal?.aborted, true);
  },
);

test(
  "cancellation interrupts a hung poll without reporting a failure",
  { timeout: 10_000 },
  async (t) => {
    t.mock.timers.enable({ apis: ["setTimeout", "Date"], now: 0 });
    const started = Promise.withResolvers<void>();
    const polling = Promise.withResolvers<void>();
    const controller = new AbortController();
    const failures: unknown[] = [];
    let unsubscribed = 0;
    let cancellations = 0;
    const client = {
      mutation(document: string) {
        if (document === "cancel") cancellations++;
        return operation({
          data: {
            startInteractiveReleaseSearch: {
              id: "session-cancel",
              state: "RUNNING",
              results: [],
              indexers: [],
            },
          },
        });
      },
      query() {
        polling.resolve();
        return {
          subscribe() {
            return {
              unsubscribe() {
                unsubscribed++;
              },
            };
          },
        };
      },
    } as unknown as Client;
    const request = runner()(
      client,
      { titleId: "movie-cancel" },
      {
        signal: controller.signal,
        onUpdate(snapshot) {
          if (snapshot.state === "RUNNING") started.resolve();
        },
        onError: (error) => failures.push(error),
      },
    );
    await started.promise;
    t.mock.timers.tick(1_000);
    await polling.promise;
    controller.abort();
    assert.deepEqual(await request, []);
    assert.equal(unsubscribed, 1);
    assert.equal(cancellations, 1);
    assert.deepEqual(failures, []);
  },
);

test(
  "a terminal poll preserves the server's completed state",
  { timeout: 10_000 },
  async (t) => {
    t.mock.timers.enable({ apis: ["setTimeout", "Date"], now: 0 });
    const started = Promise.withResolvers<void>();
    const updates: InteractiveSearchProgress[] = [];
    const completed = {
      id: "session-complete",
      state: "COMPLETED",
      results: [{ title: "Final result", source: "fixture" }],
      indexers: [],
    };
    const client = {
      mutation() {
        return operation({
          data: {
            startInteractiveReleaseSearch: {
              ...completed,
              state: "RUNNING",
              results: [],
            },
          },
        });
      },
      query() {
        return operation({ data: { interactiveReleaseSearch: completed } });
      },
    } as unknown as Client;
    const request = runner()(
      client,
      { titleId: "movie-complete" },
      {
        onUpdate(snapshot) {
          updates.push(snapshot);
          if (snapshot.state === "RUNNING") started.resolve();
        },
      },
    );

    await started.promise;
    t.mock.timers.tick(1_000);
    assert.deepEqual(await request, completed.results);
    assert.equal(updates.at(-1)?.state, "COMPLETED");
  },
);

test(
  "a transient poll failure still recovers on the next poll",
  { timeout: 10_000 },
  async (t) => {
    t.mock.timers.enable({ apis: ["setTimeout", "Date"], now: 0 });
    const started = Promise.withResolvers<void>();
    const retried = Promise.withResolvers<void>();
    t.mock.method(console, "warn", () => retried.resolve());
    let polls = 0;
    const job = {
      id: "session-retry",
      state: "RUNNING",
      results: [],
      indexers: [],
    };
    const client = {
      mutation() {
        return operation({ data: { startInteractiveReleaseSearch: job } });
      },
      query() {
        polls++;
        return operation(
          polls === 1
            ? { error: new Error("fixture network failure") }
            : {
                data: {
                  interactiveReleaseSearch: { ...job, state: "COMPLETED" },
                },
              },
        );
      },
    } as unknown as Client;
    const request = runner()(
      client,
      { titleId: "movie-retry" },
      {
        onUpdate(snapshot) {
          if (snapshot.state === "RUNNING") started.resolve();
        },
      },
    );
    await started.promise;
    t.mock.timers.tick(1_000);
    await retried.promise;
    t.mock.timers.tick(1_000);
    assert.deepEqual(await request, []);
    assert.equal(polls, 2);
  },
);

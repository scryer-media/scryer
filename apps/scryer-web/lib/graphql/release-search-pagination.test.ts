import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import ts from "typescript";
import type { Client } from "urql";
import type {
  InteractiveSearchProgress,
  runIterativeReleaseSearch,
} from "./release-search.ts";

function runner(): typeof runIterativeReleaseSearch {
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
      makeAbortableFetch: () => undefined,
    },
  };
  const module = {
    exports: {} as {
      runIterativeReleaseSearch: typeof runIterativeReleaseSearch;
    },
  };
  new Function("require", "module", "exports", code)(
    (id: string) => {
      assert.ok(id in dependencies, `unexpected dependency ${id}`);
      return dependencies[id];
    },
    module,
    module.exports,
  );
  return module.exports.runIterativeReleaseSearch;
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
      return {
        toPromise: async () => ({
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
        }),
      };
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
      return {
        toPromise: async () => ({
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
        }),
      };
    },
  } as unknown as Client;
  await runner()(client, input, { onUpdate: (update) => updates.push(update) });
  assert.deepEqual(calls, [{ document: "start", variables: { input } }]);
  assert.equal(updates[0].failureReasonCode, "RAW_SEARCH_ALL_SOURCES_LIMITED");
  assert.equal(updates[0].nextCursor, null);
});

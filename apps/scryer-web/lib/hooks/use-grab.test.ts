import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import test from "node:test";
import ts from "typescript";

const require = createRequire(import.meta.url);
type Run = { id: string; jobKey: string; status: string };
type Settle = (run: Run) => void;

// Run the real hook with React's hooks replaced and every import stubbed:
// `settle` stands in for the job provider ending the wait.
function loadGrab(grabResult: unknown) {
  const waits: Settle[] = [];
  const queries: unknown[] = [];
  const client = {
    mutation: () => ({
      toPromise: async () => ({
        data: {
          startGrab: {
            jobRun: { id: "grab-1", jobKey: "RELEASE_GRAB", status: "RUNNING" },
          },
        },
      }),
    }),
    query: (_query: unknown, variables: unknown) => {
      queries.push(variables);
      return {
        toPromise: async () => ({ data: { grabJobResult: grabResult } }),
      };
    },
  };
  const stubs: Record<string, unknown> = {
    react: {
      ...require("react"),
      useCallback: (callback: unknown) => callback,
    },
    urql: { useClient: () => client },
    "@/components/root/job-run-provider": {
      useJobRunToasts: () => ({
        registerInteractiveJobRun: (_run: Run, onTerminal: Settle) => {
          waits.push(onTerminal);
          return () => {};
        },
      }),
    },
    "@/lib/context/translate-context": {
      useTranslate: () => (key: string) => key,
    },
    "@/lib/graphql/mutations": { startGrabMutation: "mutation StartGrab" },
    "@/lib/graphql/queries": { grabJobResultQuery: "query GrabJobResult" },
    "@/lib/utils/job-runs": {
      normalizeJobRun: (value: unknown) => value ?? null,
      isTerminalJobRunStatus: (status: string) =>
        status === "COMPLETED" || status === "WARNING" || status === "FAILED",
    },
  };
  const file = path.resolve(import.meta.dirname, "use-grab.ts");
  const code = ts.transpileModule(readFileSync(file, "utf8"), {
    compilerOptions: {
      module: ts.ModuleKind.CommonJS,
      target: ts.ScriptTarget.ES2022,
    },
    fileName: file,
  }).outputText;
  const module = { exports: {} as Record<string, unknown> };
  new Function("require", "module", "exports", code)(
    (id: string) => {
      if (!(id in stubs)) throw new Error(`unexpected import ${id}`);
      return stubs[id];
    },
    module,
    module.exports,
  );
  // React's hooks are stubbed, so the hook runs as a plain function here.
  const grabHook = module.exports.useGrab as () => (
    request: object,
  ) => Promise<unknown>;
  return { grab: grabHook(), waits, queries };
}

// Waits for the grab to start waiting on its run; the bound only turns a
// grab that never waits into a failure instead of a hang.
async function untilWaiting(waits: Settle[]) {
  for (let turn = 0; waits.length === 0; turn += 1) {
    if (turn > 10_000) throw new Error("the grab never waited on its run");
    await new Promise((done) => setImmediate(done));
  }
}

test("a grab that ends returns its outcome", async () => {
  const outcome = { pending: { id: "pending-1", grabbed: true } };
  const { grab, waits } = loadGrab(outcome);

  const result = grab({ pending: "pending-1" });
  await untilWaiting(waits);
  waits[0]({ id: "grab-1", jobKey: "RELEASE_GRAB", status: "COMPLETED" });

  assert.deepEqual(await result, outcome);
});

test("a grab the provider lets go of before it ends rejects", async () => {
  const { grab, waits, queries } = loadGrab(null);

  const result = grab({ pending: "pending-1" });
  await untilWaiting(waits);
  waits[0]({ id: "grab-1", jobKey: "RELEASE_GRAB", status: "RUNNING" });

  await assert.rejects(result, { message: "status.grabOutcomeUnknown" });
  assert.deepEqual(queries, [], "no outcome is asked for");
});

test("a grab whose outcome is gone says whether it failed", async () => {
  const failed = loadGrab(null);
  const failedResult = failed.grab({ pending: "pending-1" });
  await untilWaiting(failed.waits);
  failed.waits[0]({ id: "grab-1", jobKey: "RELEASE_GRAB", status: "FAILED" });
  await assert.rejects(failedResult, { message: "status.grabFailed" });

  const done = loadGrab(null);
  const doneResult = done.grab({ pending: "pending-1" });
  await untilWaiting(done.waits);
  done.waits[0]({ id: "grab-1", jobKey: "RELEASE_GRAB", status: "COMPLETED" });
  await assert.rejects(doneResult, { message: "status.grabResultUnavailable" });
});

import assert from "node:assert/strict";
import { readFileSync, existsSync } from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import test from "node:test";
import ts from "typescript";

const require = createRequire(import.meta.url);
const webRoot = path.resolve(import.meta.dirname, "../..");

type RefreshRequest = { titleId: string };

// Run the real hook bodies with effects applied synchronously and refs kept
// across renders, so the subscription-start plumbing can be exercised without
// a browser or a live socket.
function harness(overrides: Record<string, unknown>) {
  const refs: { current: unknown }[] = [];
  let cursor = 0;
  const react = {
    ...require("react"),
    useRef(initial: unknown) {
      const index = cursor++;
      refs[index] ??= { current: initial };
      return refs[index];
    },
    useMemo: (fn: () => unknown) => fn(),
    useEffect: (fn: () => void) => {
      fn();
    },
    useLayoutEffect: (fn: () => void) => {
      fn();
    },
  };
  const cache = new Map<string, Record<string, unknown>>();
  function load(file: string): Record<string, unknown> {
    const cached = cache.get(file);
    if (cached) return cached;
    const module = { exports: {} as Record<string, unknown> };
    cache.set(file, module.exports);
    const code = ts.transpileModule(readFileSync(file, "utf8"), {
      compilerOptions: {
        module: ts.ModuleKind.CommonJS,
        target: ts.ScriptTarget.ES2022,
      },
    }).outputText;
    const localRequire = (id: string): unknown => {
      if (id === "react") return react;
      if (id in overrides) return overrides[id];
      if (!id.startsWith("@/") && !id.startsWith(".")) return require(id);
      const base = id.startsWith("@/")
        ? path.join(webRoot, id.slice(2))
        : path.resolve(path.dirname(file), id);
      const resolved = [base, base + ".ts", base + ".tsx"].find(existsSync);
      assert.ok(resolved, id);
      return load(resolved);
    };
    new Function("require", "module", "exports", code)(
      localRequire,
      module,
      module.exports,
    );
    cache.set(file, module.exports);
    return module.exports;
  }
  return {
    render(file: string, name: string, args: object) {
      cursor = 0;
      const hook = load(path.join(webRoot, "lib/hooks", file))[name] as (
        args: object,
      ) => unknown;
      return hook(args);
    },
  };
}

test("the title overview catches up once the activity subscription starts", () => {
  const refreshed: RefreshRequest[] = [];
  let streamOptions: { onStart?: () => void } = {};
  const h = harness({
    "@/lib/context/reactive-refresh-context": {
      useReactiveRefresh: () => ({
        queueTitleSidePanelOverviewRefresh: (request: RefreshRequest) => {
          refreshed.push(request);
        },
      }),
    },
    "@/lib/hooks/use-activity-event-stream": {
      useActivityEventStream: (options: { onStart?: () => void }) => {
        streamOptions = options;
      },
    },
  });

  h.render(
    "use-title-overview-reactive-refresh.ts",
    "useTitleOverviewReactiveRefresh",
    {
      titleId: "title-under-test",
      blocklistLimit: 10,
      projection: "FULL",
      applyOverviewSnapshot: () => {},
      importKinds: new Set(["import_completed"]),
    },
  );

  // The page's initial read already happened; nothing has been requested yet.
  assert.equal(refreshed.length, 0);

  assert.ok(streamOptions.onStart, "the hook must subscribe with an onStart");
  streamOptions.onStart?.();

  assert.equal(
    refreshed.length,
    1,
    "the blind window between the initial read and the live socket is caught up once",
  );
  assert.equal(refreshed[0]?.titleId, "title-under-test");
});

test("the activity stream forwards its subscription start to its caller", () => {
  let deferredOptions: { onStart?: () => void } = {};
  let started = 0;
  const h = harness({
    "@/lib/hooks/use-deferred-ws-subscription": {
      useDeferredWsSubscription: (options: { onStart?: () => void }) => {
        deferredOptions = options;
      },
    },
    "@/lib/graphql/queries": { activitySubscriptionQuery: "subscription {}" },
  });

  h.render("use-activity-event-stream.ts", "useActivityEventStream", {
    titleId: "title-under-test",
    onStart: () => {
      started += 1;
    },
    onEvent: () => {},
  });

  assert.ok(deferredOptions.onStart, "the stream must pass an onStart down");
  deferredOptions.onStart?.();
  assert.equal(started, 1);
});

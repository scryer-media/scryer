import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import test from "node:test";
import ts from "typescript";
import type { InstanceFeatures } from "../types/settings.ts";

const require = createRequire(import.meta.url);

type QueryResult = { data?: { instanceFeatures: Partial<InstanceFeatures> }; error?: Error };
type ProviderValue = { instanceFeatures: InstanceFeatures; instanceFeaturesLoaded: boolean };

test("a failed re-read keeps the last successful switches", async (t) => {
  const browser = new EventTarget();
  const originalWindow = Object.getOwnPropertyDescriptor(globalThis, "window");
  Object.defineProperty(globalThis, "window", { configurable: true, value: browser });
  const restoreWindow = () => {
    if (originalWindow) Object.defineProperty(globalThis, "window", originalWindow);
    else Reflect.deleteProperty(globalThis, "window");
  };

  const state: unknown[] = [];
  let cursor = 0;
  // Effects re-run only when their dependencies change, as in React, so the
  // first-paint read fires once rather than on every render.
  const effectDeps: unknown[][] = [];
  const effectCleanups: Array<(() => void) | undefined> = [];
  let effectCursor = 0;
  let effects: Array<() => void> = [];
  const useState = (initial: unknown) => {
    const index = cursor++;
    if (!(index in state)) state[index] = typeof initial === "function" ? initial() : initial;
    return [state[index], (next: unknown) => {
      state[index] = typeof next === "function" ? next(state[index]) : next;
    }];
  };
  const sameDeps = (previous: unknown[] | undefined, next: unknown[]) =>
    previous !== undefined && previous.length === next.length && previous.every((dep, i) => Object.is(dep, next[i]));
  // Memoized values share the state slots: each call takes the next slot in
  // render order, as React does.
  const memo = (fn: () => unknown, deps: unknown[]) => {
    const index = cursor++;
    if (!(index in state)) state[index] = { deps: undefined, value: undefined };
    const slot = state[index] as { deps: unknown[] | undefined; value: unknown };
    if (!sameDeps(slot.deps, deps)) {
      slot.deps = deps;
      slot.value = fn();
    }
    return slot.value;
  };
  const useEffect = (fn: () => void | (() => void), deps: unknown[]) => {
    const index = effectCursor++;
    if (sameDeps(effectDeps[index], deps)) return;
    effectDeps[index] = deps;
    effects.push(() => {
      effectCleanups[index]?.();
      const cleanup = fn();
      effectCleanups[index] = typeof cleanup === "function" ? cleanup : undefined;
    });
  };
  const context = { current: null as unknown };
  const pending: Array<(result: QueryResult) => void> = [];
  const client = {
    query: () => ({
      toPromise: () => new Promise<QueryResult>((resolve) => pending.push(resolve)),
    }),
  };
  const overrides: Record<string, unknown> = {
    react: {
      useState,
      useRef: (initial: unknown) => useState(() => ({ current: initial }))[0],
      useCallback: (fn: unknown, deps: unknown[]) => memo(() => fn, deps),
      useMemo: memo,
      useEffect,
      createContext: () => context,
      useContext: (value: { current: unknown }) => value.current,
    },
    "react/jsx-runtime": { jsx: (type: unknown, props: unknown) => ({ type, props }) },
    urql: {
      useClient: () => client,
    },
    "@/lib/graphql/queries": { instanceFeaturesQuery: "instanceFeatures" },
    "@/lib/hooks/use-auth": { AUTH_SESSION_CHANGED_EVENT: "auth-changed", getAuthToken: () => "token" },
    "@/lib/runtime-config": { getRuntimeBasePath: () => "/" },
    "@/lib/utils/scheduling": { scheduleAfterFirstPaint: (callback: () => void) => { callback(); return () => {}; } },
  };
  const file = new URL("./instance-features-context.tsx", import.meta.url);
  const code = ts.transpileModule(readFileSync(file, "utf8"), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX },
  }).outputText;
  const module = { exports: {} as { InstanceFeaturesProvider: (props: { children: null }) => { props: { value: ProviderValue } } } };
  new Function("require", "module", "exports", code)((id: string) => overrides[id] ?? require(id), module, module.exports);
  const render = () => {
    cursor = 0;
    effectCursor = 0;
    effects = [];
    const { value } = module.exports.InstanceFeaturesProvider({ children: null }).props;
    effects.forEach((effect) => effect());
    return value;
  };
  t.after(() => {
    effectCleanups.forEach((cleanup) => cleanup?.());
    restoreWindow();
  });
  // The query settles on a later microtask than the resolve call itself.
  const settle = async (result: QueryResult) => {
    pending.shift()?.(result);
    await new Promise((resolve) => setImmediate(resolve));
  };

  let value = render();
  assert.equal(value.instanceFeatures.experimentalFeaturesEnabled, false);
  assert.equal(value.instanceFeaturesLoaded, false);
  assert.equal(pending.length, 1);

  await settle({ data: { instanceFeatures: { experimentalFeaturesEnabled: true } } });
  value = render();
  assert.equal(value.instanceFeatures.experimentalFeaturesEnabled, true);
  assert.equal(value.instanceFeaturesLoaded, true);

  browser.dispatchEvent(new Event("focus"));
  assert.equal(pending.length, 1);
  await settle({ error: new Error("offline") });
  value = render();
  assert.equal(value.instanceFeatures.experimentalFeaturesEnabled, true, "a failed re-read must not hide switched surfaces");
  assert.equal(value.instanceFeaturesLoaded, true);

  browser.dispatchEvent(new Event("auth-changed"));
  value = render();
  assert.equal(value.instanceFeatures.experimentalFeaturesEnabled, false, "a new session starts from the defaults");
  assert.equal(value.instanceFeaturesLoaded, false);
  await settle({ data: { instanceFeatures: { experimentalFeaturesEnabled: true } } });
  assert.equal(render().instanceFeatures.experimentalFeaturesEnabled, true);
});

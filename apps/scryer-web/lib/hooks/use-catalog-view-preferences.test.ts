import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import ts from "typescript";

import * as contentViewMode from "../../components/views/media-content/content-view-mode.ts";
import type { UiSettings } from "../types/settings.ts";
import * as preferences from "../utils/catalog-view-preferences.ts";
import type { useCatalogViewPreferences } from "./use-catalog-view-preferences.ts";

type PreferencesHook = typeof useCatalogViewPreferences;

// Runs the real hook against the shipped gate, with React, the profile, the
// GraphQL client and browser storage stubbed so every save is observable.
function harness(settings: Pick<UiSettings, "catalogViews" | "tableColumns">) {
  const slots: unknown[] = [];
  const effects: (() => void)[] = [];
  let cursor = 0;
  const same = (a: unknown[], b: unknown[]) =>
    a.length === b.length &&
    a.every((value, index) => Object.is(value, b[index]));
  const react = {
    useState(initial: unknown) {
      const index = cursor++;
      if (!(index in slots)) {
        slots[index] = typeof initial === "function" ? initial() : initial;
      }
      return [
        slots[index],
        (next: unknown) => {
          slots[index] = typeof next === "function" ? next(slots[index]) : next;
        },
      ];
    },
    useRef(initial: unknown) {
      const index = cursor++;
      if (!(index in slots)) {
        slots[index] = { current: initial };
      }
      return slots[index];
    },
    useMemo<T>(fn: () => T, deps: unknown[]) {
      const index = cursor++;
      const prior = slots[index] as { deps: unknown[]; value: T } | undefined;
      if (!prior || !same(prior.deps, deps)) {
        slots[index] = { deps, value: fn() };
      }
      return (slots[index] as { value: T }).value;
    },
    useCallback<T>(fn: T, deps: unknown[]) {
      return react.useMemo(() => fn, deps);
    },
    useEffect(fn: () => void | (() => void), deps: unknown[]) {
      const index = cursor++;
      const prior = slots[index] as { deps: unknown[] } | undefined;
      if (!prior || !same(prior.deps, deps)) {
        effects.push(() => {
          fn();
          slots[index] = { deps };
        });
      }
    },
  };
  const mutations: unknown[] = [];
  const settingsWrites: unknown[] = [];
  const client = {
    mutation(_query: string, variables: unknown) {
      mutations.push(variables);
      return { toPromise: () => Promise.resolve({ data: {} }) };
    },
  };
  const uiSettings = {
    uiSettings: settings as UiSettings,
    uiSettingsLoaded: true,
    setUiSettings: (next: unknown) => settingsWrites.push(next),
  };
  const modules: Record<string, unknown> = {
    react,
    urql: { useClient: () => client },
    sonner: { toast: { error() {} } },
    "@/lib/context/ui-settings-context": { useUiSettings: () => uiSettings },
    "@/lib/graphql/mutations": { setMyCatalogViewMutation: "mutation" },
    "@/lib/hooks/use-mobile": { useIsMobile: () => false },
    "@/lib/utils/catalog-view-preferences": preferences,
    "@/components/views/media-content/content-view-mode": contentViewMode,
    "@/components/views/media-content/title-table-shared": {
      TITLE_TABLE_COLUMN_KEYS: [],
      defaultTitleTableVisibleColumnsForView: () => ({}),
      isTitleTableColumnSupportedForView: () => true,
    },
  };
  const code = ts.transpileModule(
    readFileSync(
      new URL("./use-catalog-view-preferences.ts", import.meta.url),
      "utf8",
    ),
    {
      compilerOptions: {
        module: ts.ModuleKind.CommonJS,
        target: ts.ScriptTarget.ES2022,
      },
    },
  ).outputText;
  const module = {
    exports: {} as { useCatalogViewPreferences: PreferencesHook },
  };
  new Function("require", "module", "exports", code)(
    (id: string) => {
      if (!(id in modules)) {
        throw new Error(`unexpected import ${id}`);
      }
      return modules[id];
    },
    module,
    module.exports,
  );
  return {
    mutations,
    settingsWrites,
    render() {
      cursor = 0;
      const result = module.exports.useCatalogViewPreferences(
        "series",
        (key) => key,
      );
      for (const effect of effects.splice(0)) {
        effect();
      }
      return result;
    },
  };
}

function installStorage(t: test.TestContext, stored: Record<string, string>) {
  const storage = new Map(Object.entries(stored));
  const previous = Object.getOwnPropertyDescriptor(globalThis, "window");
  Object.defineProperty(globalThis, "window", {
    configurable: true,
    value: {
      localStorage: {
        getItem: (key: string) => storage.get(key) ?? null,
        setItem: (key: string, value: string) => {
          storage.set(key, value);
        },
      },
    },
  });
  t.after(() => {
    if (previous) {
      Object.defineProperty(globalThis, "window", previous);
    } else {
      Reflect.deleteProperty(globalThis, "window");
    }
  });
  return storage;
}

test("a saved graph view shows posters while the graph is off and is never re-saved", async (t) => {
  if (preferences.DISCOVERY_GRAPH_VIEW_ENABLED) {
    t.skip("the graph view is enabled in this build");
    return;
  }
  const storage = installStorage(t, {});
  const h = harness({
    catalogViews: [
      { deviceClass: "DESKTOP", facet: "SERIES", viewMode: "GRAPH" },
    ],
    tableColumns: [],
  });
  for (let index = 0; index < 3; index += 1) {
    assert.equal(h.render().viewMode, "poster");
  }
  await Promise.resolve();
  assert.deepEqual(h.mutations, []);
  assert.deepEqual(h.settingsWrites, []);
  // The saved choice is left as it was, for a build that shows the graph.
  assert.equal(storage.size, 0);
});

test("a graph view stored in this browser also shows posters while the graph is off", async (t) => {
  if (preferences.DISCOVERY_GRAPH_VIEW_ENABLED) {
    t.skip("the graph view is enabled in this build");
    return;
  }
  const storage = installStorage(t, {
    [`${contentViewMode.CONTENT_VIEW_MODE_STORAGE_KEY}:series`]: "graph",
  });
  const h = harness({ catalogViews: [], tableColumns: [] });
  assert.equal(h.render().viewMode, "poster");
  assert.equal(h.render().viewMode, "poster");
  await Promise.resolve();
  assert.deepEqual(h.mutations, []);
  assert.equal(
    storage.get(`${contentViewMode.CONTENT_VIEW_MODE_STORAGE_KEY}:series`),
    "graph",
  );
});

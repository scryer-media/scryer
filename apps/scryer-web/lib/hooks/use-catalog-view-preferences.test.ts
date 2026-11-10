import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import ts from "typescript";

import * as contentViewMode from "../../components/views/media-content/content-view-mode.ts";
import type { UiSettings } from "../types/settings.ts";
import * as preferences from "../utils/catalog-view-preferences.ts";
import type { useCatalogViewPreferences } from "./use-catalog-view-preferences.ts";

type PreferencesHook = typeof useCatalogViewPreferences;

// Runs the real hook with the experimental-features switch set per test, with React, the profile, the
// GraphQL client and browser storage stubbed so every save is observable.
function harness(
  settings: Pick<UiSettings, "catalogViews" | "tableColumns">,
  experimentalFeaturesEnabled = false,
  // Whether the instance's switches have loaded; a test may flip it.
  instance = { featuresLoaded: true },
) {
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
    "@/lib/context/instance-features-context": {
      useInstanceFeatures: () => ({
        apiExplorerEnabled: false,
        experimentalFeaturesEnabled,
        personalizedDiscoveryEnabled: true,
      }),
      useInstanceFeaturesLoaded: () => instance.featuresLoaded,
    },
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

test("a saved graph view opens with experimental features off without rewriting preferences", async (t) => {
  const storage = installStorage(t, {});
  const h = harness({
    catalogViews: [
      { deviceClass: "DESKTOP", facet: "SERIES", viewMode: "GRAPH" },
    ],
    tableColumns: [],
  });
  for (let index = 0; index < 3; index += 1) {
    const rendered = h.render();
    assert.equal(rendered.viewMode, "graph");
  }
  await Promise.resolve();
  assert.deepEqual(h.mutations, []);
  assert.deepEqual(h.settingsWrites, []);
  // Opening the saved view must not rewrite the profile or browser preference.
  assert.equal(storage.size, 0);
});

test("a saved graph view shows the graph while experimental features are on", async (t) => {
  const storage = installStorage(t, {});
  const h = harness(
    {
      catalogViews: [
        { deviceClass: "DESKTOP", facet: "SERIES", viewMode: "GRAPH" },
      ],
      tableColumns: [],
    },
    true,
  );
  for (let index = 0; index < 3; index += 1) {
    const rendered = h.render();
    assert.equal(rendered.viewMode, "graph");
  }
  await Promise.resolve();
  assert.deepEqual(h.mutations, []);
  assert.deepEqual(h.settingsWrites, []);
  assert.equal(storage.size, 0);
});

for (const enabled of [false, true]) {
  test(`a graph view stored in this browser shows the graph while experimental features are ${enabled ? "on" : "off"}`, async (t) => {
    const storage = installStorage(t, {
      [`${contentViewMode.CONTENT_VIEW_MODE_STORAGE_KEY}:series`]: "graph",
    });
    const h = harness({ catalogViews: [], tableColumns: [] }, enabled);
    const expected = "graph";
    assert.equal(h.render().viewMode, expected);
    assert.equal(h.render().viewMode, expected);
    await Promise.resolve();
    assert.deepEqual(h.mutations, []);
    assert.equal(
      storage.get(`${contentViewMode.CONTENT_VIEW_MODE_STORAGE_KEY}:series`),
      "graph",
    );
  });
}

test("a saved graph view opens immediately before the instance switches load", async (t) => {
  installStorage(t, {});
  const instance = { featuresLoaded: false };
  const settings = {
    catalogViews: [
      {
        deviceClass: "DESKTOP" as const,
        facet: "SERIES" as const,
        viewMode: "GRAPH" as const,
      },
    ],
    tableColumns: [],
  };
  // Before the switches load the defaults say experimental features are off.
  const on = harness(settings, false, instance);
  const pending = on.render();
  assert.equal(pending.viewMode, "graph");
  assert.equal(pending.viewModePending, false);

  // Loaded and on: the graph, no longer pending.
  const loadedOn = harness(settings, true, { featuresLoaded: true }).render();
  assert.equal(loadedOn.viewMode, "graph");
  assert.equal(loadedOn.viewModePending, false);

  // Loaded and off: still the graph, and nothing is re-saved.
  instance.featuresLoaded = true;
  const loadedOff = on.render();
  assert.equal(loadedOff.viewMode, "graph");
  assert.equal(loadedOff.viewModePending, false);
  await Promise.resolve();
  assert.deepEqual(on.mutations, []);
});

test("a non-graph choice never waits on the switches", (t) => {
  installStorage(t, {});
  const rendered = harness(
    {
      catalogViews: [
        { deviceClass: "DESKTOP", facet: "SERIES", viewMode: "COMPACT" },
      ],
      tableColumns: [],
    },
    false,
    { featuresLoaded: false },
  ).render();
  assert.equal(rendered.viewMode, "compact");
  assert.equal(rendered.viewModePending, false);
});

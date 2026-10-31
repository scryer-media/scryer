import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import ts from "typescript";
import * as persistence from "../utils/catalog-filter-persistence.ts";
import type { useCatalogFilterState } from "./use-catalog-filter-state.ts";

type FiltersHook = typeof useCatalogFilterState;
type Facet = Parameters<FiltersHook>[0];

// Run the real hook with controlled commits and timers, including dependency
// tracking, so navigation and debounce behavior never depend on elapsed time.
function harness() {
  const slots: unknown[] = [];
  const effects: (() => void)[] = [];
  const timers = new Map<number, () => void>();
  const cleanups = new Map<number, () => void>();
  const listeners = new Map<string, Set<() => void>>();
  const storage = new Map<string, string>();
  let cursor = 0;
  let timerId = 0;
  const same = (a: unknown[], b: unknown[]) =>
    a.length === b.length &&
    a.every((value, index) => Object.is(value, b[index]));
  const react = {
    useState(initial: unknown) {
      const index = cursor++;
      if (!(index in slots))
        slots[index] = typeof initial === "function" ? initial() : initial;
      return [
        slots[index],
        (next: unknown) => {
          slots[index] = typeof next === "function" ? next(slots[index]) : next;
        },
      ];
    },
    useRef(initial: unknown) {
      const index = cursor++;
      if (!(index in slots)) slots[index] = { current: initial };
      return slots[index];
    },
    useMemo<T>(fn: () => T, deps: unknown[]) {
      const index = cursor++;
      const prior = slots[index] as { deps: unknown[]; value: T } | undefined;
      if (!prior || !same(prior.deps, deps))
        slots[index] = { deps, value: fn() };
      return (slots[index] as { value: T }).value;
    },
    useCallback<T>(fn: T, deps: unknown[]) {
      return react.useMemo(() => fn, deps);
    },
    useEffect(fn: () => void | (() => void), deps: unknown[]) {
      const index = cursor++;
      const prior = slots[index] as
        { deps: unknown[]; cleanup?: () => void } | undefined;
      if (!prior || !same(prior.deps, deps))
        effects.push(() => {
          prior?.cleanup?.();
          const cleanup = fn();
          slots[index] = { deps, cleanup };
          if (cleanup) cleanups.set(index, cleanup);
          else cleanups.delete(index);
        });
    },
  };
  const browser = {
    addEventListener(type: string, callback: () => void) {
      const callbacks = listeners.get(type) ?? new Set<() => void>();
      callbacks.add(callback);
      listeners.set(type, callbacks);
    },
    removeEventListener(type: string, callback: () => void) {
      listeners.get(type)?.delete(callback);
    },
    dispatchEvent(event: Event) {
      for (const callback of [...(listeners.get(event.type) ?? [])]) callback();
      return true;
    },
    localStorage: {
      getItem: (key: string) => storage.get(key) ?? null,
      setItem: (key: string, value: string) => {
        storage.set(key, value);
      },
      removeItem: (key: string) => {
        storage.delete(key);
      },
    },
    setTimeout: (fn: () => void) => {
      timers.set(++timerId, fn);
      return timerId;
    },
    clearTimeout: (id: number) => {
      timers.delete(id);
    },
  };
  const code = ts.transpileModule(
    readFileSync(
      new URL("./use-catalog-filter-state.ts", import.meta.url),
      "utf8",
    ),
    {
      compilerOptions: {
        module: ts.ModuleKind.CommonJS,
        target: ts.ScriptTarget.ES2022,
      },
    },
  ).outputText;
  const module = { exports: {} as { useCatalogFilterState: FiltersHook } };
  new Function("require", "module", "exports", code)(
    (id: string) => (id === "react" ? react : persistence),
    module,
    module.exports,
  );
  return {
    storage,
    browser,
    listeners,
    timers,
    unmount() {
      for (const cleanup of cleanups.values()) cleanup();
      cleanups.clear();
    },
    render(facet: Facet) {
      cursor = 0;
      const result = module.exports.useCatalogFilterState(facet);
      for (const effect of effects.splice(0)) effect();
      return result;
    },
    flushTimers() {
      const pending = [...timers.values()];
      timers.clear();
      for (const callback of pending) callback();
    },
  };
}

function installBrowser(
  t: test.TestContext,
  browser: ReturnType<typeof harness>["browser"],
) {
  const previous = Object.getOwnPropertyDescriptor(globalThis, "window");
  Object.defineProperty(globalThis, "window", {
    configurable: true,
    value: browser,
  });
  t.after(() => {
    if (previous) Object.defineProperty(globalThis, "window", previous);
    else Reflect.deleteProperty(globalThis, "window");
  });
}

test("catalog filters remain independent across all facets and restore on remount", (t) => {
  const h = harness();
  installBrowser(t, h.browser);
  for (const facet of persistence.CATALOG_FILTER_FACETS) {
    const state = h.render(facet);
    state.setTitleFilter(facet);
    state.setSelectedLibraryIds([facet + "-library"]);
    state.setTitleQuickFilters((previous) => ({
      ...previous,
      monitored: true,
      ended: facet === "SERIES",
    }));
    state.setAdvancedTitleFilters((previous) => ({
      ...previous,
      rootFolderIds: [facet + "-root"],
      genreTagKeys: [facet],
      themeTagKeys: [facet + "-theme"],
      userTagLabels: [facet + "-tag"],
      minimumYear: 2000,
      maximumYear: 2025,
      minimumRating: 7.5,
      presences: ["MISSING"],
      needsAttention: true,
    }));
    h.render(facet);
  }
  h.flushTimers();
  const remount = harness();
  for (const facet of [...persistence.CATALOG_FILTER_FACETS].reverse()) {
    const state = h.render(facet);
    assert.equal(state.titleFilter, facet);
    assert.equal(state.debouncedTitleFilter, facet);
    assert.deepEqual(state.selectedLibraryIds, [facet + "-library"]);
    assert.equal(state.titleQuickFilters.monitored, true);
    assert.equal(state.titleQuickFilters.ended, facet === "SERIES");
    assert.deepEqual(state.advancedTitleFilters.genreTagKeys, [facet]);
    const restored = remount.render(facet);
    assert.equal(restored.titleFilter, state.titleFilter);
    assert.deepEqual(restored.titleQuickFilters, state.titleQuickFilters);
    assert.deepEqual(
      restored.effectiveAdvancedTitleFilters,
      state.advancedTitleFilters,
    );
    assert.deepEqual(restored.selectedLibraryIds, state.selectedLibraryIds);
  }
});

test("Clear All removes only the active facet's saved state and clears pending debounce", (t) => {
  const h = harness();
  installBrowser(t, h.browser);
  h.render("MOVIE").setTitleFilter("movie");
  h.render("SERIES").setTitleFilter("series");
  h.render("ANIME").setTitleFilter("anime");
  const movie = h.render("MOVIE");
  movie.setSelectedLibraryIds(["movies"]);
  movie.setTitleQuickFilters((previous) => ({ ...previous, monitored: true }));
  movie.setAdvancedTitleFilters((previous) => ({
    ...previous,
    minimumYear: 2020,
  }));
  h.render("MOVIE").clearFilters();
  const cleared = h.render("MOVIE");
  assert.equal(cleared.titleFilter, "");
  assert.equal(cleared.debouncedTitleFilter, "");
  assert.deepEqual(cleared.selectedLibraryIds, []);
  assert.deepEqual(
    cleared.titleQuickFilters,
    persistence.emptyCatalogFilters().quick,
  );
  assert.deepEqual(
    cleared.effectiveAdvancedTitleFilters,
    persistence.emptyCatalogFilters().advanced,
  );
  h.flushTimers();
  assert.equal(h.render("MOVIE").debouncedTitleFilter, "");
  assert.equal(h.storage.size, 2);
  const remount = harness();
  assert.equal(remount.render("MOVIE").titleFilter, "");
  assert.equal(remount.render("SERIES").titleFilter, "series");
  assert.equal(remount.render("ANIME").titleFilter, "anime");
});

test("pending search and numeric filters never leak into another facet", (t) => {
  const h = harness();
  installBrowser(t, h.browser);
  const movie = h.render("MOVIE");
  movie.setTitleFilter("new search");
  movie.setAdvancedTitleFilters((previous) => ({
    ...previous,
    minimumRating: 8,
  }));
  assert.equal(h.render("MOVIE").debouncedTitleFilter, "");
  assert.equal(h.render("SERIES").debouncedTitleFilter, "");
  h.flushTimers();
  assert.equal(h.render("SERIES").debouncedTitleFilter, "");
  assert.equal(
    h.render("SERIES").effectiveAdvancedTitleFilters.minimumRating,
    null,
  );
  assert.equal(h.render("MOVIE").debouncedTitleFilter, "new search");
  assert.equal(
    h.render("MOVIE").effectiveAdvancedTitleFilters.minimumRating,
    8,
  );
});

test("malformed and older saved filters safely normalize", (t) => {
  const h = harness();
  installBrowser(t, h.browser);
  h.storage.set("scryer:catalog-filters:v1:MOVIE", "{");
  h.storage.set(
    "scryer:catalog-filters:v1:SERIES",
    JSON.stringify({ query: "older", advanced: { genreTagKeys: ["drama"] } }),
  );
  assert.deepEqual(
    persistence.readCatalogFilters("MOVIE"),
    persistence.emptyCatalogFilters(),
  );
  assert.equal(persistence.readCatalogFilters("SERIES").query, "older");
  assert.deepEqual(
    persistence.readCatalogFilters("SERIES").advanced.presences,
    [],
  );
  const invalid = persistence.normalizeCatalogFilters({
    query: 2,
    libraryIds: [null, "id", "id"],
    quick: { monitored: "false" },
    advanced: {
      minimumRating: Infinity,
      minimumYear: 2.5,
      maximumYear: -1,
      presences: ["MISSING", "invalid"],
      needsAttention: "true",
    },
  });
  assert.equal(invalid.query, "");
  assert.deepEqual(invalid.libraryIds, ["id"]);
  assert.equal(invalid.quick.monitored, false);
  assert.equal(invalid.advanced.minimumRating, null);
  assert.equal(invalid.advanced.minimumYear, null);
  assert.equal(invalid.advanced.maximumYear, null);
  assert.deepEqual(invalid.advanced.presences, ["MISSING"]);
  assert.equal(invalid.advanced.needsAttention, false);
});

test("pagehide persists pending filters and unmount removes listeners and timers", (t) => {
  const h = harness();
  installBrowser(t, h.browser);
  h.render("MOVIE").setTitleFilter("before leaving");
  h.render("MOVIE");
  assert.equal(h.listeners.get("pagehide")?.size, 1);
  h.browser.dispatchEvent(new Event("pagehide"));
  assert.equal(persistence.readCatalogFilters("MOVIE").query, "before leaving");
  h.render("MOVIE").setTitleFilter("on unmount");
  h.render("MOVIE");
  h.unmount();
  assert.equal(persistence.readCatalogFilters("MOVIE").query, "on unmount");
  assert.equal(h.listeners.get("pagehide")?.size, 0);
  assert.equal(h.timers.size, 0);
  h.storage.clear();
  h.browser.dispatchEvent(new Event("pagehide"));
  assert.equal(h.storage.size, 0);
});

test("unavailable storage preserves in-memory filtering", (t) => {
  const h = harness();
  Object.defineProperty(h.browser, "localStorage", {
    get() {
      throw new Error("storage disabled");
    },
  });
  installBrowser(t, h.browser);
  h.render("ANIME").setTitleFilter("retained");
  assert.equal(h.render("ANIME").titleFilter, "retained");
  assert.equal(h.render("MOVIE").titleFilter, "");
  h.render("ANIME").clearFilters();
  assert.equal(h.render("ANIME").titleFilter, "");
});

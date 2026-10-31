import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import test from "node:test";
import ts from "typescript";
import * as expiry from "../utils/indexer-search-expiry.ts";
import * as search from "../utils/indexer-search.ts";
import * as rawSearchErrors from "../utils/raw-search-error.ts";
import type { InteractiveSearchProgress } from "../graphql/release-search.ts";
import type { SettingsIndexerSearchSectionProps } from "../../components/views/settings/settings-indexer-search-section.tsx";

const require = createRequire(import.meta.url);

test("expiry clears results and dialog, blocks late snapshots, and permits a new search", (t) => {
  t.mock.timers.enable({
    apis: ["Date", "setTimeout"],
    now: Date.parse("2026-01-01T00:00:00Z"),
  });
  const browser = Object.assign(new EventTarget(), {
    setTimeout,
    clearTimeout,
  });
  const originalWindow = Object.getOwnPropertyDescriptor(globalThis, "window");
  const originalDocument = Object.getOwnPropertyDescriptor(
    globalThis,
    "document",
  );
  Object.defineProperty(globalThis, "window", {
    configurable: true,
    value: browser,
  });
  // No settings reference slot is mounted, so the plugin list portal stays closed.
  Object.defineProperty(globalThis, "document", {
    configurable: true,
    value: Object.assign(new EventTarget(), { getElementById: () => null }),
  });
  t.after(() => {
    if (originalWindow)
      Object.defineProperty(globalThis, "window", originalWindow);
    else Reflect.deleteProperty(globalThis, "window");
    if (originalDocument)
      Object.defineProperty(globalThis, "document", originalDocument);
    else Reflect.deleteProperty(globalThis, "document");
  });

  const state: unknown[] = [];
  let cursor = 0;
  let effects: Array<() => void | (() => void)> = [];
  let cleanups: Array<() => void> = [];
  const useState = (initial: unknown) => {
    const index = cursor++;
    if (!(index in state))
      state[index] = typeof initial === "function" ? initial() : initial;
    return [
      state[index],
      (next: unknown) => {
        state[index] = typeof next === "function" ? next(state[index]) : next;
      },
    ];
  };
  const requests: Array<{
    signal: AbortSignal;
    onUpdate: (value: InteractiveSearchProgress) => void;
  }> = [];
  let downloads = 0;
  const overrides: Record<string, unknown> = {
    react: {
      useState,
      useRef: (initial: unknown) => useState(() => ({ current: initial }))[0],
      useCallback: (fn: unknown) => fn,
      useMemo: (fn: () => unknown) => fn(),
      useEffect: (fn: () => void | (() => void)) => effects.push(fn),
    },
    "react-dom": { createPortal: () => "portal" },
    "react-router": { useSearchParams: () => [new URLSearchParams()] },
    urql: {
      useClient: () => ({
        query: () => ({ toPromise: () => new Promise(() => {}) }),
      }),
    },
    "@/lib/graphql/error-message": {
      userFacingGraphQlErrorMessage: () => "failure",
    },
    "@/lib/context/translate-context": {
      useTranslate: () => (key: string) => key,
    },
    "@/lib/context/global-status-context": { useGlobalStatus: () => () => {} },
    "@/lib/graphql/queries": { indexersQuery: "indexers" },
    "@/lib/graphql/release-search": {
      runIterativeReleaseSearch: (
        _client: unknown,
        _input: unknown,
        options: (typeof requests)[number],
      ) => {
        requests.push(options);
        return new Promise(() => {});
      },
    },
    "@/lib/utils/indexer-search": search,
    "@/lib/utils/raw-search-error": rawSearchErrors,
    "@/lib/utils/indexer-search-expiry": expiry,
    "@/lib/utils/indexer-search-download": {
      downloadIndexerSearchArtifacts: async () => {
        downloads++;
      },
    },
    "@/components/common/grab-dialog": { GrabDialog: "grab-dialog" },
    "@/components/containers/settings/settings-container": {
      SETTINGS_REFERENCE_SLOT_ID: "settings-reference-slot",
    },
    "@/components/views/settings/filtered-plugin-list": {
      FilteredPluginList: "plugin-list",
    },
    "@/components/views/settings/settings-indexer-search-section": {
      SettingsIndexerSearchSection: "search-view",
    },
  };
  const file = new URL(
    "../../components/containers/settings/settings-indexer-search-container.tsx",
    import.meta.url,
  );
  const code = ts.transpileModule(readFileSync(file, "utf8"), {
    compilerOptions: {
      module: ts.ModuleKind.CommonJS,
      jsx: ts.JsxEmit.ReactJSX,
    },
  }).outputText;
  const module = {
    exports: {} as {
      SettingsIndexerSearchContainer: () => {
        props: {
          children: [
            unknown,
            { props: SettingsIndexerSearchSectionProps },
            { props: { open: boolean } },
          ];
        };
      };
    },
  };
  new Function("require", "module", "exports", code)(
    (id: string) => overrides[id] ?? require(id),
    module,
    module.exports,
  );
  const render = () => {
    cleanups.forEach((cleanup) => cleanup());
    cursor = 0;
    effects = [];
    const { children } = module.exports.SettingsIndexerSearchContainer().props;
    cleanups = effects
      .map((effect) => effect())
      .filter(
        (cleanup): cleanup is () => void => typeof cleanup === "function",
      );
    return { view: children[1].props, dialog: children[2].props };
  };
  t.after(() => cleanups.forEach((cleanup) => cleanup()));
  render().view.onQueryChange("Example");
  render().view.onSearch();
  const snapshot: InteractiveSearchProgress = {
    searchId: "first",
    state: "COMPLETED",
    indexers: [],
    startedAt: new Date().toISOString(),
    completedAt: new Date().toISOString(),
    releases: [
      {
        title: "Example",
        source: "fixture",
        link: "https://example.test/release",
        downloadUrl: "https://example.test/release",
        sizeBytes: 100,
        publishedAt: null,
      },
    ],
  };
  requests[0].onUpdate(snapshot);
  let screen = render();
  assert.equal(screen.view.rows.length, 1);
  screen.view.onToggleRow(snapshot.releases[0]);
  screen.view.onGrab(snapshot.releases);
  screen = render();
  assert.equal(screen.dialog.open, true);
  t.mock.timers.tick(5 * 60 * 1000);
  screen = render();
  assert.equal(screen.view.rows.length, 0);
  assert.deepEqual(screen.view.selectedRowKeys, []);
  assert.equal(screen.dialog.open, false);
  assert.equal(requests[0].signal.aborted, true);
  requests[0].onUpdate(snapshot);
  assert.equal(render().view.rows.length, 0);

  render().view.onSearch();
  requests[1].onUpdate({
    ...snapshot,
    searchId: "second",
    completedAt: new Date().toISOString(),
  });
  screen = render();
  assert.equal(screen.view.rows.length, 1);
  // Simulate a suspended tab: advance the wall clock without dispatching timers.
  t.mock.timers.setTime(Date.now() + 5 * 60 * 1000);
  screen.view.onDownload(snapshot.releases);
  assert.equal(downloads, 0);
  assert.equal(render().view.rows.length, 0);

  render().view.onSearch();
  requests[2].onUpdate({
    ...snapshot,
    searchId: "third",
    completedAt: new Date().toISOString(),
  });
  assert.equal(render().view.rows.length, 1);
  t.mock.timers.setTime(Date.now() + 5 * 60 * 1000);
  browser.dispatchEvent(new Event("focus"));
  assert.equal(render().view.rows.length, 0);
});

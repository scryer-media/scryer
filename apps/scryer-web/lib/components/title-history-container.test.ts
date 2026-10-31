import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import test from "node:test";
import ts from "typescript";
import type { TitleHistoryView } from "../../components/views/title-history-view.tsx";

type TitleHistoryViewProps = Parameters<typeof TitleHistoryView>[0];

const require = createRequire(import.meta.url);

type Deferred = {
  variables: { filter: { titleIds: string[] | null } };
  resolve: (value: unknown) => void;
};

type Registration = { run: () => void };

function historyPage(ids: string[]) {
  return {
    data: {
      titleHistory: {
        items: ids.map((id) => ({ id })),
        totalCount: ids.length,
      },
    },
  };
}

async function settle() {
  for (let index = 0; index < 10; index += 1) {
    await Promise.resolve();
  }
}

// Runs the real container with a minimal hook runtime: state and refs persist
// across renders, and memos, callbacks and effects only re-run when their
// dependencies change, so each render issues exactly the loads React would.
function mountContainer() {
  const slots: unknown[] = [];
  let cursor = 0;
  let pendingEffects: Array<() => void> = [];
  const cleanups = new Map<number, () => void>();
  const depsChanged = (index: number, deps: unknown[] | undefined) => {
    const previous = slots[index] as { deps?: unknown[] } | undefined;
    return (
      previous === undefined ||
      deps === undefined ||
      previous.deps === undefined ||
      deps.length !== previous.deps.length ||
      deps.some(
        (value, position) => !Object.is(value, previous.deps?.[position]),
      )
    );
  };
  const react = {
    useState(initial: unknown) {
      const index = cursor++;
      if (!(index in slots)) {
        slots[index] = {
          value: typeof initial === "function" ? initial() : initial,
        };
      }
      const slot = slots[index] as { value: unknown };
      return [
        slot.value,
        (next: unknown) => {
          slot.value = typeof next === "function" ? next(slot.value) : next;
        },
      ];
    },
    useRef(initial: unknown) {
      const index = cursor++;
      if (!(index in slots)) slots[index] = { current: initial };
      return slots[index];
    },
    useMemo(factory: () => unknown, deps?: unknown[]) {
      const index = cursor++;
      if (depsChanged(index, deps)) slots[index] = { deps, value: factory() };
      return (slots[index] as { value: unknown }).value;
    },
    useCallback(callback: unknown, deps?: unknown[]) {
      return react.useMemo(() => callback, deps);
    },
    useEffect(effect: () => void | (() => void), deps?: unknown[]) {
      const index = cursor++;
      if (!depsChanged(index, deps)) return;
      slots[index] = { deps };
      pendingEffects.push(() => {
        cleanups.get(index)?.();
        const cleanup = effect();
        if (typeof cleanup === "function") cleanups.set(index, cleanup);
        else cleanups.delete(index);
      });
    },
    useId: () => "fixture-id",
  };

  const setGlobalStatus = () => {};
  const translate = (key: string) => key;
  const historyRequests: Deferred[] = [];
  let registration: Registration | null = null;
  const client = {
    query(query: string, variables: Deferred["variables"]) {
      if (query === "libraries") {
        return {
          toPromise: () => Promise.resolve({ data: { libraries: [] } }),
        };
      }
      return {
        toPromise: () =>
          new Promise((resolve) => {
            historyRequests.push({ variables, resolve });
          }),
      };
    },
  };
  const overrides: Record<string, unknown> = {
    react,
    urql: { useClient: () => client },
    "@/lib/graphql/queries": {
      librariesQuery: "libraries",
      titleHistoryQuery: "titleHistory",
    },
    "@/lib/graphql/mutations": { retryImportMutation: "retryImport" },
    "@/lib/context/global-status-context": {
      useGlobalStatus: () => setGlobalStatus,
    },
    "@/lib/context/translate-context": { useTranslate: () => translate },
    "@/components/common/title-history-event-meta": {
      WANTED_HISTORY_FILTERS: ["grabbed"],
      domainEventTypesForHistoryEvents: () => ["FIXTURE_EVENT"],
    },
    "@/lib/context/reactive-refresh-context": {
      useReactiveRefresh: () => ({
        registerReactiveRefresh: (next: Registration) => {
          registration = next;
          return () => {};
        },
      }),
    },
    "@/lib/reactive/domain-event-feed": { forEventTypes: () => () => true },
    "@/components/views/title-history-view": {
      TitleHistoryView: "title-history-view",
    },
    "@/lib/utils/library-filter": {
      normalizeLibraryFilterSelection: (current: string[]) => current,
      selectedLibraryIdsToQueryValue: (ids: string[]) =>
        ids.length > 0 ? ids : null,
    },
  };
  const file = new URL(
    "../../components/containers/title-history-container.tsx",
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
      TitleHistoryContainer: (props: object) => {
        props: TitleHistoryViewProps;
      };
    },
  };
  new Function("require", "module", "exports", code)(
    (id: string) => overrides[id] ?? require(id),
    module,
    module.exports,
  );

  const render = () => {
    cursor = 0;
    pendingEffects = [];
    const view = module.exports.TitleHistoryContainer({}).props;
    const effects = pendingEffects;
    pendingEffects = [];
    effects.forEach((effect) => effect());
    return view;
  };
  const unmount = () => {
    cleanups.forEach((cleanup) => cleanup());
    cleanups.clear();
  };
  return {
    render,
    unmount,
    historyRequests,
    liveRefresh: () => {
      assert.ok(registration, "the container registers a live refresh");
      registration.run();
    },
  };
}

const titleRecord = (id: string) => ({ id, name: `Fixture ${id}` }) as never;

test("an older history response never replaces the rows for a newer title", async (t) => {
  const screen = mountContainer();
  t.after(screen.unmount);

  let view = screen.render();
  assert.equal(screen.historyRequests.length, 1);

  view.onSelectedTitleChange(titleRecord("title-b"));
  screen.render();
  assert.equal(screen.historyRequests.length, 2);
  assert.deepEqual(screen.historyRequests[1].variables.filter.titleIds, [
    "title-b",
  ]);

  screen.historyRequests[1].resolve(historyPage(["row-for-title-b"]));
  await settle();
  screen.historyRequests[0].resolve(historyPage(["row-for-every-title"]));
  await settle();

  view = screen.render();
  assert.deepEqual(
    view.events.map((event) => event.id),
    ["row-for-title-b"],
  );
  assert.equal(view.loading, false);
});

test("a live-event refetch keeps the current rows without showing the loading state", async (t) => {
  const screen = mountContainer();
  t.after(screen.unmount);

  let view = screen.render();
  assert.equal(view.loading, true, "the first load shows the loading state");
  screen.historyRequests[0].resolve(historyPage(["first-row"]));
  await settle();
  view = screen.render();
  assert.equal(view.loading, false);

  screen.liveRefresh();
  assert.equal(screen.historyRequests.length, 2);
  view = screen.render();
  assert.equal(
    view.loading,
    false,
    "a background refetch does not flash the list",
  );
  assert.deepEqual(
    view.events.map((event) => event.id),
    ["first-row"],
  );

  screen.historyRequests[1].resolve(historyPage(["first-row", "live-row"]));
  await settle();
  view = screen.render();
  assert.deepEqual(
    view.events.map((event) => event.id),
    ["first-row", "live-row"],
  );

  view.onNextPage();
  screen.render();
  assert.equal(screen.historyRequests.length, 3);
  view = screen.render();
  assert.equal(
    view.loading,
    true,
    "a page change the user made still shows the loading state",
  );
});

test("a live-event refetch that overtakes a user load still clears the loading state", async (t) => {
  const screen = mountContainer();
  t.after(screen.unmount);

  screen.render();
  screen.liveRefresh();
  screen.historyRequests[1].resolve(historyPage(["live-row"]));
  await settle();
  screen.historyRequests[0].resolve(historyPage(["stale-row"]));
  await settle();

  const view = screen.render();
  assert.equal(view.loading, false);
  assert.deepEqual(
    view.events.map((event) => event.id),
    ["live-row"],
  );
});

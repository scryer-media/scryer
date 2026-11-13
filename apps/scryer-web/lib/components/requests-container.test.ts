import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import test from "node:test";
import ts from "typescript";
import type { RequestsView } from "../../components/views/requests-view.tsx";

type RequestsViewProps = Parameters<typeof RequestsView>[0];
type QueryCall = { query: string; variables: Record<string, unknown> };
type MutationCall = { mutation: string; variables: Record<string, unknown> };

const require = createRequire(import.meta.url);

async function settle() {
  for (let index = 0; index < 5; index += 1) {
    await new Promise((resolve) => setImmediate(resolve));
  }
}

function rejectedRequest(id: string, createdAt: string, userId: string) {
  return {
    id,
    libraryId: "library-1",
    facet: "movie",
    status: "REJECTED",
    title: "Fixture Movie",
    externalIds: [{ source: "tmdb", value: "1001" }],
    requesters: [{ userId, username: userId, requestedAt: createdAt }],
    createdAt,
    updatedAt: createdAt,
  };
}

// Runs the real container with a minimal hook runtime: state and refs persist
// across renders, and callbacks and effects only re-run when their
// dependencies change, so each render issues exactly the loads React would.
function mountContainer(adminRequests: unknown[]) {
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
  };

  const auth = { user: { id: "manager-a" } as { id: string } };
  const statuses: Array<{ message: string; level?: string }> = [];
  const queries: QueryCall[] = [];
  const mutations: MutationCall[] = [];
  const client = {
    query(query: string, variables: Record<string, unknown>) {
      queries.push({ query, variables });
      const data =
        query === "mediaRequests"
          ? { mediaRequests: adminRequests }
          : query === "adminLibraries"
            ? { libraries: [{ id: "library-1" }] }
            : { libraries: [] };
      return { toPromise: () => Promise.resolve({ data }) };
    },
    mutation(mutation: string, variables: Record<string, unknown>) {
      mutations.push({ mutation, variables });
      return { toPromise: () => Promise.resolve({ data: {} }) };
    },
  };
  const overrides: Record<string, unknown> = {
    react,
    urql: { useClient: () => client },
    "@/components/views/requests-view": { RequestsView: "requests-view" },
    "@/lib/graphql/mutations": {
      approveMediaRequestMutation: "approve",
      cancelMyMediaRequestMutation: "cancel",
      convertTitleClaimToPermanentMutation: "convertClaim",
      dismissMediaRequestMutation: "dismiss",
      extendTitleClaimMutation: "extendClaim",
      releaseTitleClaimMutation: "releaseClaim",
      reopenMediaRequestMutation: "reopen",
      updateMyMediaRequestMutation: "update",
    },
    "@/lib/graphql/queries": {
      mediaRequestAdminLibrariesQuery: "adminLibraries",
      mediaRequestRequesterLibrariesQuery: "requesterLibraries",
      mediaRequestsQuery: "mediaRequests",
      myMediaRequestsQuery: "myMediaRequests",
      qualityProfileOptionsQuery: "qualityProfiles",
      titleClaimsQuery: "titleClaims",
    },
    "@/lib/context/global-status-context": {
      useGlobalStatus:
        () => (message: string, options?: { level?: string }) => {
          statuses.push({ message, level: options?.level });
        },
    },
    "@/lib/context/translate-context": {
      useTranslate: () => (key: string) => key,
    },
    "@/lib/events/navigation-badges": {
      dispatchNavigationBadgesRefresh: () => {},
      NAVIGATION_BADGES_REFRESH_EVENT: "navigation-badges-refresh",
    },
    "@/lib/hooks/use-auth": { useAuth: () => auth },
    "@/lib/hooks/use-media-requests-subscription": {
      useMediaRequestsSubscription: () => {},
    },
    "@/lib/utils/permissions": {
      hasAnyLibraryPermission: () => true,
      LIBRARY_PERMISSIONS: { manageTitles: "manage_titles" },
    },
    "@/lib/utils/library-filter": {
      normalizeLibraryFilterSelection: (current: string[]) => current,
    },
    "@/lib/utils/media-request-filters": { requesterFilterOptions: () => [] },
    "@/lib/utils/trailing-throttle": {
      createTrailingThrottle: () => ({ request: () => {}, cancel: () => {} }),
    },
  };
  const file = new URL(
    "../../components/containers/requests-container.tsx",
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
      RequestsContainer: (props: object) => { props: RequestsViewProps };
    },
  };
  const fakeWindow = {
    addEventListener: () => {},
    removeEventListener: () => {},
  };
  new Function("require", "module", "exports", "window", code)(
    (id: string) => overrides[id] ?? require(id),
    module,
    module.exports,
    fakeWindow,
  );

  const render = () => {
    cursor = 0;
    pendingEffects = [];
    const view = module.exports.RequestsContainer({}).props;
    const effects = pendingEffects;
    pendingEffects = [];
    effects.forEach((effect) => effect());
    return view;
  };
  const unmount = () => {
    cleanups.forEach((cleanup) => cleanup());
    cleanups.clear();
  };
  return { render, unmount, auth, statuses, queries, mutations };
}

test("reopening a collapsed row reopens every request it stands for", async (t) => {
  const screen = mountContainer([
    rejectedRequest("request-late", "2026-01-02T00:00:00Z", "requester-b"),
    rejectedRequest("request-early", "2026-01-01T00:00:00Z", "requester-a"),
  ]);
  t.after(screen.unmount);

  screen.render();
  await settle();
  let view = screen.render();
  assert.deepEqual(
    view.requests.map((request) => request.id),
    ["request-early"],
  );

  view.onReopen(view.requests[0]);
  await settle();
  view = screen.render();

  assert.deepEqual(
    screen.mutations.map((call) => [call.mutation, call.variables.requestId]),
    [
      ["reopen", "request-early"],
      ["reopen", "request-late"],
    ],
  );
  assert.deepEqual(screen.statuses.at(-1), {
    message: "status.requestReopened",
    level: undefined,
  });
  assert.equal(view.actionRequestId, null);
});

test("switching accounts clears the queue filters chosen by the previous account", async (t) => {
  const screen = mountContainer([]);
  t.after(screen.unmount);

  screen.render();
  await settle();
  let view = screen.render();
  view.onSelectedRequesterIdChange("requester-of-a");
  view.onSelectedLibraryIdsChange(["library-1"]);
  screen.render();
  await settle();
  const filtered = screen.queries
    .filter((call) => call.query === "mediaRequests")
    .at(-1);
  assert.equal(filtered?.variables.requesterUserId, "requester-of-a");

  screen.auth.user = { id: "manager-b" };
  screen.render();
  await settle();
  view = screen.render();
  await settle();

  assert.equal(view.selectedRequesterId, null);
  assert.deepEqual(view.selectedLibraryIds, []);
  const latest = screen.queries
    .filter((call) => call.query === "mediaRequests")
    .at(-1);
  assert.equal(latest?.variables.requesterUserId, null);
  assert.equal(latest?.variables.libraryIds, null);
});

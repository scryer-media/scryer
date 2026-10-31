import assert from "node:assert/strict";
import { existsSync, readFileSync, statSync } from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import test from "node:test";
import ts from "typescript";

const require = createRequire(import.meta.url);
const webRoot = path.resolve(import.meta.dirname, "../..");

type Deferred<T> = { promise: Promise<T>; resolve: (value: T) => void };

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

// Run the real module with React's hooks replaced: effects run at once, state
// is kept in slots, and every listed module is stubbed so no browser, socket,
// or GraphQL client is needed.
function load(file: string, overrides: Record<string, unknown>) {
  const slots: { value?: unknown; set?: (next: unknown) => void }[] = [];
  let cursor = 0;
  // Each write is recorded with the index of the useState call it belongs to.
  const stateWrites: [number, unknown][] = [];
  const react = {
    ...require("react"),
    useState(initial: unknown) {
      const index = cursor++;
      const slot = (slots[index] ??= {});
      if (!slot.set) {
        slot.value =
          typeof initial === "function"
            ? (initial as () => unknown)()
            : initial;
        slot.set = (next: unknown) => {
          slot.value =
            typeof next === "function"
              ? (next as (v: unknown) => unknown)(slot.value)
              : next;
          stateWrites.push([index, slot.value]);
        };
      }
      return [slot.value, slot.set];
    },
    useRef(initial: unknown) {
      const slot = (slots[cursor++] ??= {});
      if (!("value" in slot)) slot.value = { current: initial };
      return slot.value;
    },
    useCallback: (callback: unknown) => callback,
    useMemo: (compute: () => unknown) => compute(),
    useEffect(effect: () => unknown) {
      effect();
    },
  };
  const cache = new Map<string, Record<string, unknown>>();
  function loadFile(target: string): Record<string, unknown> {
    const cached = cache.get(target);
    if (cached) return cached;
    const module = { exports: {} as Record<string, unknown> };
    cache.set(target, module.exports);
    const code = ts.transpileModule(readFileSync(target, "utf8"), {
      compilerOptions: {
        module: ts.ModuleKind.CommonJS,
        target: ts.ScriptTarget.ES2022,
        jsx: ts.JsxEmit.ReactJSX,
      },
      fileName: target,
    }).outputText;
    const localRequire = (id: string): unknown => {
      if (id === "react") return react;
      if (id in overrides) return overrides[id];
      if (!id.startsWith("@/") && !id.startsWith(".")) return require(id);
      const base = id.startsWith("@/")
        ? path.join(webRoot, id.slice(2))
        : path.resolve(path.dirname(target), id);
      const resolved = [base, base + ".ts", base + ".tsx"].find(
        (candidate) => existsSync(candidate) && statSync(candidate).isFile(),
      );
      assert.ok(resolved, id);
      return loadFile(resolved);
    };
    new Function("require", "module", "exports", code)(
      localRequire,
      module,
      module.exports,
    );
    cache.set(target, module.exports);
    return module.exports;
  }
  const exports = loadFile(path.join(webRoot, file));
  return {
    exports,
    stateWrites,
    render<T>(fn: () => T): T {
      cursor = 0;
      return fn();
    },
  };
}

function upgradeStatus(tag: string, version: string) {
  return {
    currentVersion: "0.0.1",
    updateVersion: version,
    updateTag: tag,
    updateAvailable: true,
    installationKind: "DIRECT_MSI",
    managementOwner: "IN_APP",
    eligible: true,
    eligibilityReason: "eligible",
    activeRun: null,
    latestRun: null,
  };
}

const NOTICE_KEY = "smg.scryer_update_notice";

function loadStatusHook() {
  const queries: Deferred<void>[] = [deferred()];
  let queryCount = 0;
  const subscriptions: {
    onChanged: (keys: string[]) => void;
    options: { enabled?: boolean };
  }[] = [];
  const client = {
    query() {
      queryCount += 1;
      queries[queryCount - 1]?.resolve();
      queries.push(deferred());
      return {
        toPromise: async () => ({
          data: { applicationUpgradeStatus: upgradeStatus("v0.0.2", "0.0.2") },
        }),
      };
    },
  };
  const harness = load("lib/hooks/use-application-upgrade-status.ts", {
    urql: { useClient: () => client },
    "@/lib/graphql/queries": { applicationUpgradeStatusQuery: "query Status" },
    "@/lib/graphql/urql-client": { notifyBackendRestarting: () => {} },
    "@/lib/hooks/use-settings-subscription": {
      useSettingsSubscription: (
        onChanged: (keys: string[]) => void,
        options: { enabled?: boolean } = {},
      ) => {
        subscriptions.push({ onChanged, options });
      },
    },
    "@/lib/utils/application-upgrade-progress": {
      normalizeApplicationUpgradeProgress: () => null,
    },
    "@/lib/utils/job-runs": { normalizeJobRun: () => null },
  });
  const hook = harness.exports.useApplicationUpgradeStatus as (
    enabled?: boolean,
    options?: { settingsSubscriptionEnabled?: boolean },
  ) => unknown;
  return {
    render: (
      enabled?: boolean,
      options?: { settingsSubscriptionEnabled?: boolean },
    ) => harness.render(() => hook(enabled, options)),
    queryNumber: (n: number) => queries[n - 1]!.promise,
    queryCount: () => queryCount,
    subscriptions,
  };
}

test("the upgrade status refetches when the stored update notice changes", async () => {
  const hook = loadStatusHook();
  hook.render();
  await hook.queryNumber(1);

  const subscription = hook.subscriptions.at(-1);
  assert.ok(subscription, "the hook must subscribe to settings changes");
  assert.equal(subscription.options.enabled, true);

  subscription.onChanged(["some.other_key"]);
  assert.equal(
    hook.queryCount(),
    1,
    "unrelated settings do not refetch the status",
  );

  subscription.onChanged([NOTICE_KEY]);
  await hook.queryNumber(2);
  assert.equal(hook.queryCount(), 2);
});

test("the upgrade status subscription follows the enabled flags", () => {
  const withoutSubscription = loadStatusHook();
  withoutSubscription.render(true, { settingsSubscriptionEnabled: false });
  assert.equal(
    withoutSubscription.subscriptions.at(-1)?.options.enabled,
    false,
  );

  const disabled = loadStatusHook();
  disabled.render(false);
  assert.equal(disabled.subscriptions.at(-1)?.options.enabled, false);
  assert.equal(disabled.queryCount(), 0);
});

type ElementLike = { type?: unknown; props?: Record<string, unknown> };

function findElement(node: unknown, type: unknown): ElementLike | null {
  if (!node || typeof node !== "object") return null;
  if (Array.isArray(node)) {
    for (const child of node) {
      const found = findElement(child, type);
      if (found) return found;
    }
    return null;
  }
  const element = node as ElementLike;
  if (element.type === type) return element;
  return findElement(element.props?.children, type);
}

function loadUpgradeAction() {
  const mutationCalled = deferred<Record<string, unknown>>();
  const infoToast = deferred<string>();
  const mutations: Record<string, unknown>[] = [];
  const errors: string[] = [];
  function ConfirmDialog() {
    return null;
  }
  const client = {
    mutation(_query: unknown, variables: Record<string, unknown>) {
      mutations.push(variables);
      mutationCalled.resolve(variables);
      return {
        toPromise: async () => ({
          data: { startApplicationUpgrade: { jobRun: { id: "run-1" } } },
        }),
      };
    },
  };
  const harness = load("components/common/application-upgrade.tsx", {
    urql: { useClient: () => client },
    "@/components/common/confirm-dialog": { ConfirmDialog },
    "@/components/root/job-run-provider": {
      useJobRunToasts: () => ({ registerInteractiveJobRun: () => {} }),
    },
    "@/components/ui/button": { Button: () => null },
    "@/components/ui/sonner": {
      toast: {
        error: (message: string) => errors.push(message),
        info: (message: string) => infoToast.resolve(message),
      },
    },
    "@/lib/context/translate-context": {
      useTranslate: () => (key: string) => key,
    },
    "@/lib/context/ui-settings-context": {
      useUiDateTimeFormat: () => "default",
    },
    "@/lib/graphql/queries": {
      startApplicationUpgradeMutation: "mutation Start",
    },
    "@/lib/hooks/use-application-upgrade-status": {
      useApplicationUpgradeStatus: () => ({}),
    },
    "@/lib/utils/activity-utils": { formatByteCount: String },
    "@/lib/utils/application-upgrade-progress": {
      normalizeApplicationUpgradeProgress: () => null,
    },
    "@/lib/utils/date-format": { formatUiDateTime: String },
    "@/lib/utils/job-runs": {
      normalizeJobRun: (value: unknown) => value ?? null,
    },
  });
  const Action = harness.exports.ApplicationUpgradeAction as (
    props: object,
  ) => unknown;
  return {
    confirm(props: object) {
      const tree = harness.render(() => Action(props));
      const dialog = findElement(tree, ConfirmDialog);
      assert.ok(dialog, "the action renders a confirmation dialog");
      (dialog.props!.onConfirm as () => void)();
    },
    mutationCalled: mutationCalled.promise,
    infoToast: infoToast.promise,
    mutations,
    errors,
    stateWrites: harness.stateWrites,
  };
}

test("starting an upgrade sends the refetched target, not the mount-time one", async () => {
  const action = loadUpgradeAction();
  let refreshes = 0;
  action.confirm({
    status: upgradeStatus("v0.0.2", "0.0.2"),
    refreshStatus: async () => {
      refreshes += 1;
      return upgradeStatus("v0.0.5", "0.0.5");
    },
  });

  const variables = await action.mutationCalled;
  assert.equal(refreshes, 1);
  assert.deepEqual(variables, {
    input: { expectedTag: "v0.0.5", expectedVersion: "0.0.5" },
  });
});

test("starting an upgrade stops when the refetched status is no longer startable", async () => {
  const action = loadUpgradeAction();
  let statusChanged = 0;
  action.confirm({
    status: upgradeStatus("v0.0.2", "0.0.2"),
    refreshStatus: async () => ({
      ...upgradeStatus("", ""),
      updateTag: null,
      updateVersion: null,
      updateAvailable: false,
    }),
    onStatusChanged: () => {
      statusChanged += 1;
    },
  });

  assert.equal(await action.infoToast, "appUpgrade.targetChanged");
  assert.equal(
    action.mutations.length,
    0,
    "no mutation is sent for a stale target",
  );
  assert.equal(statusChanged, 1);
  // The dialog's open flag is the action's first piece of state.
  assert.deepEqual(
    action.stateWrites.filter(([index]) => index === 0),
    [[0, false]],
    "the dialog is closed",
  );
  assert.deepEqual(action.errors, []);
});

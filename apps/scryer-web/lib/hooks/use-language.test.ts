import assert from "node:assert/strict";
import { readFileSync, existsSync, statSync } from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import test from "node:test";
import ts from "typescript";

const require = createRequire(import.meta.url);
const webRoot = path.resolve(import.meta.dirname, "../..");

const CHOICE_KEY = "scryer.ui.language.choice";
const PROFILE_HINT_KEY = "scryer.ui.language.profile";

type Settings = { language: string | null };
type Profile = {
  uiSettings: Settings;
  uiSettingsLoaded: boolean;
  uiSettingsLoadError: string | null;
};
type SaveResult = { error?: Error; data?: { setMyUiSettings?: Settings } };
type Save = { language: string | undefined; settle: (result: SaveResult) => Promise<unknown> };
type LanguageApi = { uiLanguage: string; setLanguagePreference: (code: string) => void };

const PENDING: Profile = { uiSettings: { language: null }, uiSettingsLoaded: false, uiSettingsLoadError: null };
const FAILED: Profile = { ...PENDING, uiSettingsLoadError: "offline" };
const loaded = (language: string | null): Profile => ({
  uiSettings: { language },
  uiSettingsLoaded: true,
  uiSettingsLoadError: null,
});

class MemoryStorage {
  private items = new Map<string, string>();
  getItem(key: string) {
    return this.items.get(key) ?? null;
  }
  setItem(key: string, value: string) {
    this.items.set(key, String(value));
  }
  removeItem(key: string) {
    this.items.delete(key);
  }
}

type Slot = {
  value?: unknown;
  set?: (next: unknown) => void;
  current?: unknown;
  deps?: unknown[];
  cleanup?: (() => void) | void;
};
type Instance = {
  slots: Slot[];
  dirty: boolean;
  searchParams: URLSearchParams;
  result?: LanguageApi;
};

function depsChanged(previous: unknown[] | undefined, next: unknown[] | undefined) {
  return (
    !previous ||
    !next ||
    previous.length !== next.length ||
    previous.some((value, index) => !Object.is(value, next[index]))
  );
}

/**
 * One browser tab: its own sessionStorage over the browser's shared
 * localStorage, the real useLanguage module, and a profile the test drives.
 * Hooks run with React's render-then-effects order, and every state change
 * re-renders synchronously until nothing is left to do.
 */
function openTab(options: {
  localStorage: MemoryStorage;
  browserLanguage: string;
  profile: Profile;
  signedOutLogin?: boolean;
  /** A language whose dictionary loads only when the test releases it. */
  heldDictionary?: string;
}) {
  const sessionStorage = new MemoryStorage();
  Object.assign(globalThis, {
    window: { localStorage: options.localStorage, sessionStorage, location: { pathname: "/" } },
    document: {
      documentElement: { lang: "" },
      addEventListener() {},
      removeEventListener() {},
    },
  });
  Object.defineProperty(globalThis, "navigator", {
    value: { language: options.browserLanguage },
    configurable: true,
  });

  let profile = options.profile;
  const saves: Save[] = [];
  const toasts: string[] = [];
  const loads: Promise<unknown>[] = [];
  const instances: Instance[] = [];
  let active: Instance | null = null;
  let cursor = 0;
  let pendingEffects: (() => void)[] = [];
  let flushing = false;
  let scheduled: Promise<void> | null = null;
  const heldLoads: (() => void)[] = [];

  function slot(): Slot {
    assert.ok(active, "hooks only run while rendering");
    const index = cursor++;
    active.slots[index] ??= {};
    return active.slots[index];
  }

  // State set by the hook is batched like React does: a render already in
  // progress picks it up, anything else renders once the current task ends.
  function scheduleFlush() {
    if (flushing || scheduled) return;
    scheduled = Promise.resolve().then(() => {
      scheduled = null;
      flush();
    });
  }

  function markAllDirty() {
    for (const instance of instances) instance.dirty = true;
  }

  // A change made by the test renders at once, as inside React's act().
  function rerenderAll() {
    markAllDirty();
    flush();
  }

  function flush() {
    if (flushing) return;
    flushing = true;
    try {
      // Like a React commit, every dirty instance renders before any effect
      // of that pass runs, so sibling hooks see the same profile.
      for (let pass = 0; instances.some((instance) => instance.dirty); pass += 1) {
        assert.ok(pass < 100, "useLanguage keeps re-rendering");
        pendingEffects = [];
        for (const instance of instances) {
          if (!instance.dirty) continue;
          instance.dirty = false;
          active = instance;
          cursor = 0;
          instance.result = renderLanguage(instance.searchParams);
          active = null;
        }
        for (const effect of pendingEffects) effect();
      }
    } finally {
      flushing = false;
    }
  }

  const react = {
    ...require("react"),
    useState(initial: unknown) {
      const state = slot();
      const owner = active!;
      if (!state.set) {
        state.value = typeof initial === "function" ? (initial as () => unknown)() : initial;
        state.set = (next: unknown) => {
          const value =
            typeof next === "function" ? (next as (value: unknown) => unknown)(state.value) : next;
          if (Object.is(value, state.value)) return;
          state.value = value;
          owner.dirty = true;
          scheduleFlush();
        };
      }
      return [state.value, state.set];
    },
    useRef(initial: unknown) {
      const ref = slot();
      if (!("current" in ref)) ref.current = initial;
      return ref;
    },
    useMemo(compute: () => unknown, deps?: unknown[]) {
      const memo = slot();
      if (!memo.deps || depsChanged(memo.deps, deps)) {
        memo.value = compute();
        memo.deps = deps;
      }
      return memo.value;
    },
    useCallback(callback: unknown, deps?: unknown[]) {
      return react.useMemo(() => callback, deps);
    },
    useEffect(effect: () => (() => void) | void, deps?: unknown[]) {
      const record = slot();
      if (record.deps && !depsChanged(record.deps, deps)) return;
      record.deps = deps ?? [Symbol()];
      pendingEffects.push(() => {
        if (typeof record.cleanup === "function") record.cleanup();
        record.cleanup = effect();
      });
    },
  };

  const cache = new Map<string, Record<string, unknown>>();
  function load(file: string, overrides: Record<string, unknown>): Record<string, unknown> {
    const cached = cache.get(file);
    if (cached) return cached;
    const module = { exports: {} as Record<string, unknown> };
    cache.set(file, module.exports);
    const code = ts.transpileModule(readFileSync(file, "utf8"), {
      compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
    }).outputText;
    const localRequire = (id: string): unknown => {
      if (id === "react") return react;
      if (id in overrides) return overrides[id];
      if (!id.startsWith("@/") && !id.startsWith(".")) return require(id);
      const base = id.startsWith("@/")
        ? path.join(webRoot, id.slice(2))
        : path.resolve(path.dirname(file), id);
      const resolved = [base, base + ".ts", base + ".tsx", path.join(base, "index.ts")].find(
        (candidate) => existsSync(candidate) && statSync(candidate).isFile(),
      );
      assert.ok(resolved, id);
      return load(resolved, overrides);
    };
    new Function("require", "module", "exports", code)(localRequire, module, module.exports);
    cache.set(file, module.exports);
    return module.exports;
  }

  const i18n = load(path.join(webRoot, "lib/i18n/index.ts"), {});
  const overrides: Record<string, unknown> = {
    "@/lib/i18n": {
      ...i18n,
      isLocaleLoaded: () => true,
      loadLocaleDictionary: (code: string) => {
        if (code === options.heldDictionary) {
          return new Promise<object>((settle) => {
            heldLoads.push(() => {
              const done = Promise.resolve({});
              loads.push(done);
              settle(done);
            });
          });
        }
        const done = Promise.resolve({});
        loads.push(done);
        return done;
      },
    },
    "@/lib/constants/settings": { URL_PARAM_LANGUAGE: "lang" },
    "@/lib/graphql/mutations": { setMyUiSettingsMutation: "mutation SetMyUiSettings" },
    "@/lib/context/ui-settings-context": {
      isSignedOutLoginSurface: () => options.signedOutLogin ?? false,
      uiSettingsInputFromSettings: (_settings: Settings, language?: string) => ({ language }),
      useUiSettings: () => ({
        ...profile,
        setUiSettings: (settings: Settings) => {
          profile = { ...profile, uiSettings: settings };
          markAllDirty();
          scheduleFlush();
        },
      }),
    },
    urql: {
      useClient: () => ({
        mutation: (_query: string, variables: { input: { language?: string } }) => ({
          toPromise: () => {
            let resolve: (result: SaveResult) => void = () => {};
            const done = new Promise<SaveResult>((settle) => {
              resolve = settle;
            });
            saves.push({
              language: variables.input.language,
              settle: (result) => {
                resolve(result);
                return done;
              },
            });
            return done;
          },
        }),
      }),
    },
    sonner: { toast: { error: (message: string) => toasts.push(message) } },
  };
  const { useLanguage: renderLanguage } = load(path.join(webRoot, "lib/hooks/use-language.ts"), overrides) as {
    useLanguage: (searchParams: URLSearchParams) => LanguageApi;
  };

  return {
    sessionStorage,
    saves,
    toasts,
    mount(search = "") {
      const instance: Instance = { slots: [], dirty: true, searchParams: new URLSearchParams(search) };
      instances.push(instance);
      flush();
      return {
        get uiLanguage() {
          return instance.result!.uiLanguage;
        },
        pick(code: string) {
          instance.result!.setLanguagePreference(code);
        },
      };
    },
    releaseDictionary() {
      for (const release of heldLoads.splice(0)) release();
    },
    setProfile(next: Profile) {
      profile = next;
      rerenderAll();
    },
    // Waits until every dictionary load the hook started has been applied and
    // every render it caused has run, including loads those renders started.
    // A hook that keeps switching languages fails here instead of hanging.
    async settle() {
      for (let round = 0; ; round += 1) {
        assert.ok(round < 50, "useLanguage keeps switching languages");
        const started = loads.length;
        await Promise.all(loads);
        if (scheduled) await scheduled;
        if (loads.length === started && !scheduled) return;
      }
    },
  };
}

test("a pick on the login page is kept for the next tab", async () => {
  const localStorage = new MemoryStorage();
  const login = openTab({
    localStorage,
    browserLanguage: "es-ES",
    profile: PENDING,
    signedOutLogin: true,
  });
  const loginPage = login.mount();
  assert.equal(loginPage.uiLanguage, "spa");

  loginPage.pick("fra");
  await login.settle();
  assert.equal(loginPage.uiLanguage, "fra");
  assert.equal(login.saves.length, 0, "there is no profile to save on");

  const next = openTab({ localStorage, browserLanguage: "es-ES", profile: PENDING });
  assert.equal(next.mount().uiLanguage, "fra", "a new tab paints in the pick");
});

test("a pick while the profile failed to load holds and is kept for the next tab", async () => {
  const localStorage = new MemoryStorage();
  localStorage.setItem(PROFILE_HINT_KEY, "deu");
  const tab = openTab({ localStorage, browserLanguage: "es-ES", profile: PENDING });
  const page = tab.mount();
  assert.equal(page.uiLanguage, "deu", "the last profile language paints first");

  tab.setProfile(FAILED);
  page.pick("ita");
  await tab.settle();
  assert.equal(page.uiLanguage, "ita");
  assert.equal(tab.saves.length, 0);
  assert.equal(localStorage.getItem(CHOICE_KEY), "ita");

  const next = openTab({ localStorage, browserLanguage: "es-ES", profile: PENDING });
  const nextPage = next.mount();
  next.setProfile(FAILED);
  await next.settle();
  assert.equal(nextPage.uiLanguage, "ita");
});

test("an empty loaded profile takes over the picked language once", async () => {
  const localStorage = new MemoryStorage();
  localStorage.setItem(CHOICE_KEY, "ita");
  const tab = openTab({ localStorage, browserLanguage: "es-ES", profile: PENDING });
  const page = tab.mount();
  assert.equal(page.uiLanguage, "ita");
  assert.equal(tab.saves.length, 0, "nothing is saved while the profile is loading");

  tab.setProfile(loaded(null));
  assert.deepEqual(
    tab.saves.map((save) => save.language),
    ["ita"],
  );
  await tab.saves[0]!.settle({ data: { setMyUiSettings: { language: "ita" } } });
  await tab.settle();
  assert.equal(page.uiLanguage, "ita");

  tab.setProfile(loaded("ita"));
  assert.equal(tab.saves.length, 1);
});

test("every mounted language hook shares the one take-over save", () => {
  const localStorage = new MemoryStorage();
  localStorage.setItem(CHOICE_KEY, "ita");
  const tab = openTab({ localStorage, browserLanguage: "es-ES", profile: PENDING });
  tab.mount();
  tab.mount();

  tab.setProfile(loaded(null));
  assert.equal(tab.saves.length, 1);
});

test("a profile language is never replaced by the picked language", async () => {
  const localStorage = new MemoryStorage();
  localStorage.setItem(CHOICE_KEY, "ita");
  const tab = openTab({ localStorage, browserLanguage: "es-ES", profile: PENDING });
  const page = tab.mount();

  tab.setProfile(loaded("deu"));
  await tab.settle();
  assert.equal(page.uiLanguage, "deu", "the profile language wins");
  assert.equal(tab.saves.length, 0);
  assert.equal(localStorage.getItem(CHOICE_KEY), "deu", "an older pick here does not outlive it");

  const login = openTab({
    localStorage,
    browserLanguage: "es-ES",
    profile: PENDING,
    signedOutLogin: true,
  });
  assert.equal(login.mount().uiLanguage, "deu", "signing out keeps the language");
});

test("a pick whose dictionary arrives after the profile loaded is saved on the profile", async () => {
  const localStorage = new MemoryStorage();
  localStorage.setItem(CHOICE_KEY, "ita");
  const tab = openTab({
    localStorage,
    browserLanguage: "es-ES",
    profile: PENDING,
    heldDictionary: "fra",
  });
  const page = tab.mount();

  page.pick("fra");
  tab.setProfile(loaded(null));
  assert.deepEqual(
    tab.saves.map((save) => save.language),
    ["ita"],
    "the empty profile first takes over the older pick",
  );

  tab.releaseDictionary();
  await tab.settle();
  assert.deepEqual(
    tab.saves.map((save) => save.language),
    ["ita", "fra"],
  );
  await tab.saves[0]!.settle({ data: { setMyUiSettings: { language: "ita" } } });
  await tab.saves[1]!.settle({ data: { setMyUiSettings: { language: "fra" } } });
  await tab.settle();
  assert.equal(page.uiLanguage, "fra");
  assert.equal(localStorage.getItem(CHOICE_KEY), "fra");
  assert.equal(localStorage.getItem(PROFILE_HINT_KEY), "fra");
});

test("a failed profile load saves nothing and shows the picked language", async () => {
  const localStorage = new MemoryStorage();
  localStorage.setItem(CHOICE_KEY, "ita");
  localStorage.setItem(PROFILE_HINT_KEY, "deu");
  const tab = openTab({ localStorage, browserLanguage: "es-ES", profile: PENDING });
  const page = tab.mount();
  assert.equal(page.uiLanguage, "deu", "a loading profile paints in its last language");
  assert.equal(tab.saves.length, 0);

  tab.setProfile(FAILED);
  await tab.settle();
  assert.equal(page.uiLanguage, "ita");
  assert.equal(tab.saves.length, 0);
  assert.equal(localStorage.getItem(PROFILE_HINT_KEY), "deu");
});

test("a failed take-over save waits for the next profile load", async () => {
  const localStorage = new MemoryStorage();
  localStorage.setItem(CHOICE_KEY, "ita");
  const tab = openTab({ localStorage, browserLanguage: "es-ES", profile: PENDING });
  const page = tab.mount();

  tab.setProfile(loaded(null));
  assert.equal(tab.saves.length, 1);
  await tab.saves[0]!.settle({ error: new Error("unavailable") });
  await tab.settle();
  assert.equal(tab.saves.length, 1, "a failed save is not retried in place");
  assert.deepEqual(tab.toasts, [], "nobody asked for this save");
  assert.equal(localStorage.getItem(CHOICE_KEY), "ita");
  assert.equal(page.uiLanguage, "ita");

  tab.setProfile(PENDING);
  tab.setProfile(loaded(null));
  assert.equal(tab.saves.length, 2, "the next load tries again");
  assert.equal(tab.saves[1]!.language, "ita");
});

test("a URL language is never kept for other tabs or saved on the profile", async () => {
  const localStorage = new MemoryStorage();
  const tab = openTab({ localStorage, browserLanguage: "es-ES", profile: PENDING });
  const page = tab.mount("lang=fra");
  tab.setProfile(loaded(null));
  await tab.settle();
  assert.equal(page.uiLanguage, "fra");
  assert.equal(tab.saves.length, 0);
  assert.equal(localStorage.getItem(CHOICE_KEY), null);
  assert.equal(localStorage.getItem(PROFILE_HINT_KEY), null);

  const next = openTab({ localStorage, browserLanguage: "es-ES", profile: PENDING });
  assert.equal(next.mount().uiLanguage, "spa");
});

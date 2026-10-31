import assert from "node:assert/strict";
import { readFileSync, existsSync } from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import test from "node:test";
import ts from "typescript";
import type { UseMediaSettingsResult } from "./use-media-settings.ts";
import type { IndexerRoutingHookResult } from "./use-indexer-routing.ts";

const require = createRequire(import.meta.url);
const webRoot = path.resolve(import.meta.dirname, "../..");

// Exercise the real hook callbacks with deterministic state and deferred I/O,
// without needing a browser or timing-dependent React effects.
function harness(client: object) {
  const state: unknown[] = [];
  let cursor = 0;
  const messages: string[] = [];
  const react = {
    ...require("react"),
    useState(initial: unknown) {
      const index = cursor++;
      if (!(index in state))
        state[index] = typeof initial === "function" ? initial() : initial;
      return [
        state[index],
        (next: unknown) => {
          state[index] = typeof next === "function" ? next(state[index]) : next;
        },
      ];
    },
    useCallback: (fn: unknown) => fn,
    useMemo: (fn: () => unknown) => fn(),
    useEffect: () => {},
  };
  const overrides: Record<string, unknown> = {
    react,
    urql: { useClient: () => client },
    "@/lib/context/translate-context": {
      useTranslate: () => (key: string) => key,
    },
    "@/lib/context/global-status-context": {
      useGlobalStatus: () => (message: string) => messages.push(message),
    },
    "@/lib/hooks/use-settings-subscription": {
      useSettingsSubscription: () => {},
    },
  };
  const cache = new Map<string, { exports: Record<string, unknown> }>();
  function load(file: string): Record<string, unknown> {
    const cached = cache.get(file);
    if (cached) return cached.exports;
    const module = { exports: {} };
    cache.set(file, module);
    const code = ts.transpileModule(readFileSync(file, "utf8"), {
      compilerOptions: {
        module: ts.ModuleKind.CommonJS,
        target: ts.ScriptTarget.ES2022,
      },
    }).outputText;
    const localRequire = (id: string): unknown => {
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
    return module.exports;
  }
  return {
    messages,
    render<T>(file: string, name: string): T {
      cursor = 0;
      const hook = load(path.join(webRoot, "lib/hooks", file))[name] as (
        args: object,
      ) => T;
      return hook({ activeQualityScopeId: "ANIME", view: "anime" });
    },
    load,
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

test(
  "media-only mutation hydration preserves loaded quality profiles and personas",
  { timeout: 10_000 },
  async () => {
    const quality = {
      profiles: [
        {
          id: "custom",
          name: "Custom",
          criteria: {
            qualityTiers: ["1080P"],
            sourceAllowlist: [],
            sourceBlocklist: [],
            videoCodecAllowlist: [],
            videoCodecBlocklist: [],
            audioCodecAllowlist: [],
            audioCodecBlocklist: [],
          },
        },
      ],
      globalProfileId: "custom",
      globalScoringPersona: "QUALITY",
      categorySelections: [{ scope: "ANIME", overrideProfileId: "custom" }],
      categoryPersonaSelections: [
        {
          scope: "ANIME",
          overridePersona: "QUALITY",
          effectivePersona: "QUALITY",
          inheritsGlobal: false,
        },
      ],
    };
    const media = {
      scope: "ANIME",
      libraryPath: "",
      rootFolders: [],
      requiredAudioLanguages: [],
    };
    const h = harness({
      query: () => ({
        toPromise: async () => ({
          data: { qualityProfileSettings: quality, mediaSettings: media },
        }),
      }),
      mutation: () => ({
        toPromise: async () => ({
          data: {
            updateMediaSettings: { ...media, requiredAudioLanguages: ["jpn"] },
          },
        }),
      }),
    });
    const render = () =>
      h.render<UseMediaSettingsResult>(
        "use-media-settings.ts",
        "useMediaSettings",
      );
    await render().refreshMediaSettings();
    const before = render();
    assert.equal(before.globalQualityProfileId, "custom");
    await before.saveCategoryRequiredAudioLanguages(["jpn"]);
    const after = render();
    assert.strictEqual(after.qualityProfiles, before.qualityProfiles);
    assert.strictEqual(
      after.categoryQualityProfileOverrides,
      before.categoryQualityProfileOverrides,
    );
    assert.strictEqual(
      after.categoryPersonaSelections,
      before.categoryPersonaSelections,
    );
    assert.equal(after.globalScoringPersona, before.globalScoringPersona);
    assert.deepEqual(after.categoryRequiredAudioLanguages.ANIME, ["jpn"]);
  },
);

test(
  "failed general save restores the previous value and releases the save lock",
  { timeout: 10_000 },
  async () => {
    const response = deferred<{ error: Error }>();
    const h = harness({
      mutation: () => ({ toPromise: () => response.promise }),
    });
    const render = () =>
      h.render<UseMediaSettingsResult>(
        "use-media-settings.ts",
        "useMediaSettings",
      );
    const before = render();
    before.setCategoryMonitorSpecials((previous) => ({
      ...previous,
      ANIME: "true",
    }));
    const saving = before.saveSetting(
      "system",
      "ANIME",
      "anime.monitor_specials",
      "true",
    );
    assert.equal(render().mediaSettingsSaving, true);
    response.resolve({ error: new Error("save rejected") });
    await saving;
    const after = render();
    assert.equal(after.categoryMonitorSpecials.ANIME, "false");
    assert.equal(after.mediaSettingsSaving, false);
    assert.deepEqual(h.messages, ["save rejected"]);
  },
);

test(
  "indexer edits do not overlap and stale reads cannot replace saved routing",
  { timeout: 10_000 },
  async () => {
    const read = deferred<object>();
    const save = deferred<object>();
    let writes = 0;
    const h = harness({
      query: () => ({ toPromise: () => read.promise }),
      mutation: () => {
        writes++;
        return { toPromise: () => save.promise };
      },
    });
    const render = () =>
      h.render<IndexerRoutingHookResult>(
        "use-indexer-routing.ts",
        "useIndexerRouting",
      );
    render().hydrateIndexerRouting(
      [{ id: "indexer" } as never],
      [
        {
          indexerId: "indexer",
          enabled: true,
          categories: ["5070"],
          priority: 1,
        },
      ],
    );
    const pendingRead = render().refreshIndexerRouting();
    const pendingSave = render().updateIndexerRoutingForScope("indexer", {
      categories: ["5000"],
    });
    await render().updateIndexerRoutingForScope("indexer", {
      categories: ["2000"],
    });
    assert.equal(writes, 1);
    save.resolve({
      data: {
        updateIndexerRouting: [
          {
            indexerId: "indexer",
            enabled: true,
            categories: ["5000"],
            priority: 1,
          },
        ],
      },
    });
    await pendingSave;
    read.resolve({
      data: {
        indexers: [{ id: "indexer" }],
        indexerRouting: [
          {
            indexerId: "indexer",
            enabled: true,
            categories: ["5070"],
            priority: 1,
          },
        ],
      },
    });
    await pendingRead;
    assert.deepEqual(render().activeScopeRouting.indexer.categories, ["5000"]);
  },
);

test("download-client state retains server order and appends newly configured clients", () => {
  const h = harness({});
  const { buildDownloadClientRoutingState } = h.load(
    path.join(webRoot, "lib/utils/download-client-routing.ts"),
  );
  const build = buildDownloadClientRoutingState as (
    clients: object[],
    entries: object[],
  ) => { order: string[] };
  assert.deepEqual(
    build(
      [{ id: "a" }, { id: "b" }, { id: "new" }],
      [{ clientId: "b" }, { clientId: "removed" }, { clientId: "a" }],
    ).order,
    ["b", "a", "new"],
  );
});

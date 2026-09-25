import assert from "node:assert/strict";
import { readFileSync, existsSync, statSync } from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import test from "node:test";
import ts from "typescript";
import type { MetadataTvdbSearchItem } from "@/lib/graphql/smg-queries";
import type * as GlobalSearch from "./use-global-search.ts";

const require = createRequire(import.meta.url);
const webRoot = path.resolve(import.meta.dirname, "../..");

// Load the real module without a browser: transpile each local file on demand
// and stub the React-context and UI modules the pure input builder never uses.
function loadGlobalSearch(): typeof GlobalSearch {
  const overrides: Record<string, unknown> = {
    "@/lib/context/translate-context": { useTranslate: () => (key: string) => key },
    "@/lib/context/global-status-context": { useGlobalStatus: () => () => {} },
    "@/lib/hooks/use-settings-subscription": { useSettingsSubscription: () => {} },
    "@/components/root/catalog-add-toast": { showCatalogAddToast: () => {} },
    "@/lib/graphql/urql-client": { isAbortError: () => false, makeAbortableFetch: () => fetch },
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
        jsx: ts.JsxEmit.ReactJSX,
      },
      fileName: file,
    }).outputText;
    const localRequire = (id: string): unknown => {
      if (id in overrides) return overrides[id];
      if (!id.startsWith("@/") && !id.startsWith(".")) return require(id);
      const base = id.startsWith("@/")
        ? path.join(webRoot, id.slice(2))
        : path.resolve(path.dirname(file), id);
      const resolved = [base, base + ".ts", base + ".tsx", path.join(base, "index.ts")].find(
        (candidate) => existsSync(candidate) && statSync(candidate).isFile(),
      );
      assert.ok(resolved, id);
      return load(resolved);
    };
    new Function("require", "module", "exports", code)(localRequire, module, module.exports);
    return module.exports;
  }
  return load(path.join(webRoot, "lib/hooks/use-global-search.ts")) as unknown as typeof GlobalSearch;
}

const RATING_KEYS = ["normalized", "score", "source", "url", "value", "votes"];

function searchItem(overrides: Partial<MetadataTvdbSearchItem>): MetadataTvdbSearchItem {
  return {
    tvdbId: "1001",
    name: "Synthetic Title",
    imdbId: null,
    slug: null,
    type: "series",
    year: 2020,
    status: null,
    overview: null,
    popularity: null,
    posterUrl: null,
    language: null,
    runtimeMinutes: null,
    sortTitle: null,
    ...overrides,
  };
}

test("media request input drops urql __typename from external ratings", () => {
  const { submitMediaRequestInput } = loadGlobalSearch();
  const externalRatings = [
    { __typename: "TitleExternalRating", source: "imdb", value: 7.5, score: null, normalized: 75, votes: 1200, url: "https://ratings.example/a" },
    { __typename: "TitleExternalRating", source: "tmdb", value: null, score: 81, normalized: 81, votes: null, url: "https://ratings.example/b" },
  ] as unknown as MetadataTvdbSearchItem["externalRatings"];
  const input = submitMediaRequestInput(
    searchItem({
      externalRatings,
      ratingSources: ["imdb", "tmdb"],
      externalIds: [{ __typename: "ExternalId", source: "anidb", value: "42" }] as unknown as MetadataTvdbSearchItem["externalIds"],
    }),
    "ANIME",
    {
      libraryId: "library-1",
      requestedMonitorType: "ADVANCED",
      requestedMonitorSelection: {
        seasonNumbers: [1],
        seriesMovies: [{
          name: "Synthetic Movie",
          externalIds: [{ __typename: "ExternalId", source: "tmdb", value: "7" }],
        }],
      } as unknown as GlobalSearch.MetadataCatalogRequestOptions["requestedMonitorSelection"],
    },
  );

  assert.ok(!JSON.stringify(input).includes("__typename"), JSON.stringify(input));
  assert.equal(input.externalRatings?.length, 2);
  for (const rating of input.externalRatings ?? []) {
    assert.deepEqual(Object.keys(rating).sort(), RATING_KEYS);
  }
  assert.deepEqual(input.externalRatings?.[1], {
    source: "tmdb", value: null, score: 81, normalized: 81, votes: null, url: "https://ratings.example/b",
  });
  assert.deepEqual(input.ratingSources, ["imdb", "tmdb"]);
});

test("media request input leaves external ratings undefined when the result has none", () => {
  const { submitMediaRequestInput } = loadGlobalSearch();
  const input = submitMediaRequestInput(searchItem({}), "SERIES", { libraryId: "library-1" });
  assert.equal(input.externalRatings, undefined);
  assert.ok(!("externalRatings" in JSON.parse(JSON.stringify(input))));
});

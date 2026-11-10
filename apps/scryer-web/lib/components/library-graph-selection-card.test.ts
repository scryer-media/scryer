import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { createElement, type ComponentType, type Context } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createServer } from "vite";
import type { Translate } from "@/components/root/types";
import type { TitleRecord } from "../types/titles.ts";
import type { DiscoveryItem } from "../types/discovery.ts";

test("picked library cards hydrate missing metadata without widening the catalog or leaking stale title details", async () => {
  const state: {
    data: unknown;
    query?: { pause: boolean; variables: { id: string }; query: string };
  } = { data: null };
  const fixtureGlobal = globalThis as typeof globalThis & {
    graphCardFixture?: typeof state;
  };
  fixtureGlobal.graphCardFixture = state;
  const root = fileURLToPath(new URL("../..", import.meta.url));
  const server = await createServer({
    configFile: false,
    root,
    server: { middlewareMode: true },
    appType: "custom",
    ssr: { noExternal: ["urql"] },
    optimizeDeps: { noDiscovery: true, include: [] },
    logLevel: "silent",
    resolve: { alias: { "@": root } },
    plugins: [
      {
        name: "graph-card-query-fixture",
        enforce: "pre",
        resolveId(id) {
          if (id === "urql") return "\0graph-card-query";
        },
        load(id) {
          if (id === "\0graph-card-query")
            return `export function useQuery(options) {
          const state = globalThis.graphCardFixture;
          state.query = options;
          return [{ data: state.data, fetching: false }];
        }`;
        },
      },
    ],
  });
  try {
    const view = await server.ssrLoadModule(
      "/components/views/media-content/library-graph-selection-card.tsx",
    );
    const translation = await server.ssrLoadModule(
      "/lib/context/translate-context.tsx",
    );
    const Card = view.LibraryGraphSelectionCard as ComponentType<{
      label: string;
      title: TitleRecord | null;
      item: DiscoveryItem | null;
      facet: "MOVIE";
      ghost: boolean;
      inLibrary: boolean;
      onClose: () => void;
      children: string;
    }>;
    const TranslateContext =
      translation.TranslateContext as Context<Translate | null>;
    const title = {
      id: "owned-title",
      name: "Fixture",
      facet: "MOVIE",
      monitored: true,
      tags: [],
      libraryId: "library",
    } satisfies TitleRecord;
    const render = (
      owned: TitleRecord | null,
      item: DiscoveryItem | null = null,
    ) =>
      renderToStaticMarkup(
        createElement(
          TranslateContext.Provider,
          { value: (key) => key },
          createElement(Card, {
            label: "Fixture",
            title: owned,
            item,
            facet: "MOVIE",
            ghost: !owned,
            inLibrary: !!owned,
            onClose() {},
            children: "Action",
          }),
        ),
      );
    state.data = {
      title: {
        id: title.id,
        overview: "Owned description",
        backgroundUrl: "https://images.example/background.jpg",
        canonicalTags: [
          { category: "genre", name: "Adventure", isSpoiler: false },
        ],
      },
    };
    const hydrated = render(title);
    assert.match(hydrated, /Owned description/);
    assert.match(hydrated, /https:\/\/images\.example\/background\.jpg/);
    assert.match(hydrated, /Adventure/);
    assert.equal(state.query?.pause, false);
    assert.deepEqual(state.query?.variables, { id: title.id });
    assert.match(state.query!.query, /title\(id: \$id\)/);
    assert.match(state.query!.query, /overview/);
    assert.match(state.query!.query, /backgroundUrl/);

    state.data = {
      title: {
        id: "previous-title",
        overview: "Stale description",
        backgroundUrl: "https://images.example/stale.jpg",
      },
    };
    const switched = render(title);
    assert.doesNotMatch(switched, /Stale description|stale\.jpg/);

    const complete = {
      ...title,
      overview: "Inline description",
      backgroundUrl: null,
      canonicalTags: [],
    };
    assert.match(render(complete), /Inline description/);
    assert.equal(state.query?.pause, true);

    const discovery = {
      overview: "Discovery description",
      backgroundUrl: "https://images.example/discovery.jpg",
      contentType: "MOVIE",
    } as DiscoveryItem;
    const unowned = render(null, discovery);
    assert.equal(state.query?.pause, true);
    assert.match(unowned, /Discovery description/);
    assert.match(unowned, /https:\/\/images\.example\/discovery\.jpg/);
    assert.doesNotMatch(unowned, /Stale description/);
  } finally {
    await server.close();
    delete fixtureGlobal.graphCardFixture;
  }
});

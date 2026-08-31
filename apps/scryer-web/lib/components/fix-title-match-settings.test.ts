import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import {
  createElement,
  type ComponentType,
  type Context,
  type ReactNode,
} from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { Provider, type Client } from "urql";
import { createServer } from "vite";

import type { Translate } from "@/components/root/types";
import type { LibraryRecord, TitleRecord } from "@/lib/types/titles";

const WEB_ROOT = fileURLToPath(new URL("../..", import.meta.url));
const translate: Translate = (key) => key;

type MovieSettingsProps = {
  title: TitleRecord;
  libraries: LibraryRecord[];
  onUpdateTitleOptions: () => Promise<void>;
  onTitleChanged: () => Promise<void>;
  onOpenFixMatch: () => void;
};

type FixMatchCardProps = {
  facet: string;
  idPrefix: string;
  onOpen: () => void;
};

async function renderWithTranslation(
  modulePath: string,
  exportName: string,
  props: Record<string, unknown>,
  withGraphql = false,
): Promise<string> {
  const server = await createServer({
    root: WEB_ROOT,
    server: { middlewareMode: true },
    appType: "custom",
    logLevel: "silent",
  });

  try {
    const [componentModule, contextModule, globalStatusModule] = await Promise.all([
      server.ssrLoadModule(modulePath),
      server.ssrLoadModule("/lib/context/translate-context.tsx"),
      server.ssrLoadModule("/lib/context/global-status-context.tsx"),
    ]);
    const Component = componentModule[exportName] as ComponentType<
      Record<string, unknown>
    >;
    const TranslateContext = contextModule.TranslateContext as Context<
      Translate | null
    >;
    const GlobalStatusContext = globalStatusModule.GlobalStatusContext as Context<
      ((message: string) => void) | null
    >;
    let rendered: ReactNode = createElement(
      GlobalStatusContext.Provider,
      { value: () => {} },
      createElement(Component, props),
    );
    if (withGraphql) {
      rendered = createElement(
        Provider,
        { value: {} as Client },
        rendered,
      );
    }
    return renderToStaticMarkup(
      createElement(TranslateContext.Provider, { value: translate }, rendered),
    );
  } finally {
    await server.close();
  }
}

test("movie settings render the movie Fix Match control", async () => {
  const props: MovieSettingsProps = {
    title: {
      id: "movie-1",
      name: "Wrong Movie",
      facet: "MOVIE",
      libraryId: "movies",
      monitored: true,
      tags: [],
      externalIds: [],
    },
    libraries: [],
    onUpdateTitleOptions: async () => {},
    onTitleChanged: async () => {},
    onOpenFixMatch: () => {},
  };
  const html = await renderWithTranslation(
    "/components/views/media-content/movie-title-settings-panel.tsx",
    "MovieTitleSettingsPanel",
    props,
    true,
  );

  assert.match(html, /id="title-overview-settings-fix-match"/);
  assert.match(html, /title\.fixMatchDescriptionMovie/);
  assert.doesNotMatch(html, /title\.fixMatchDescriptionSeries/);
});

test("shared Fix Match card preserves the series control", async () => {
  const props: FixMatchCardProps = {
    facet: "SERIES",
    idPrefix: "series-overview-settings",
    onOpen: () => {},
  };
  const html = await renderWithTranslation(
    "/components/common/fix-title-match-settings-card.tsx",
    "FixTitleMatchSettingsCard",
    props,
  );

  assert.match(html, /id="series-overview-settings-fix-match"/);
  assert.match(html, /title\.fixMatchDescriptionSeries/);
  assert.doesNotMatch(html, /title\.fixMatchDescriptionMovie/);
});

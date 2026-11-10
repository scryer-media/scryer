import assert from "node:assert/strict";
import test from "node:test";

import type { TitleRecord } from "../types/titles.ts";
import { graphSelectionCatalogTitle } from "./library-graph-selection.ts";

const title: TitleRecord = {
  id: "owned-series",
  name: "Library Series",
  facet: "SERIES",
  libraryId: "series-library",
  monitored: true,
  tags: [],
};

test("an owned graph node resolves without a discovery item", () => {
  assert.equal(
    graphSelectionCatalogTitle(title, null, new Map(), [title]),
    title,
  );
});

test("a discovered title resolves by readable local id before external keys", () => {
  const other = { ...title, id: "other-series" };
  assert.equal(
    graphSelectionCatalogTitle(
      undefined,
      { targetKey: "tvdb:series:42", resolvedTitleId: ` ${title.id} ` },
      new Map([["tvdb:series:42", other]]),
      [title, other],
    ),
    title,
  );
});

test("a discovered title already in the catalog resolves by its external key", () => {
  assert.equal(
    graphSelectionCatalogTitle(
      undefined,
      { targetKey: "tvdb:series:42" },
      new Map([["tvdb:series:42", title]]),
      [title],
    ),
    title,
  );
});

test("a new recommendation never borrows an unrelated library title", () => {
  assert.equal(
    graphSelectionCatalogTitle(
      undefined,
      { targetKey: "tvdb:series:99" },
      new Map([["tvdb:series:42", title]]),
      [title],
    ),
    null,
  );
  assert.equal(
    graphSelectionCatalogTitle(undefined, null, new Map(), [title]),
    null,
  );
});

test("a resolved id outside the readable listing stays on the resolved-title path", () => {
  assert.equal(
    graphSelectionCatalogTitle(
      undefined,
      {
        targetKey: "tvdb:series:99",
        resolvedTitleId: "another-readable-title",
      },
      new Map(),
      [title],
    ),
    null,
  );
});

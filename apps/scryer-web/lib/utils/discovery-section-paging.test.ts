import assert from "node:assert/strict";
import test from "node:test";

import {
  appendDiscoverySectionPage,
  discoveryBrowseSectionId,
  discoverySectionPagingFor,
  discoverySectionQueryKey,
  nextDiscoverySectionPage,
  startDiscoverySectionPaging,
  type DiscoverySectionQuery,
} from "./discovery-section-paging.ts";

type Card = { targetKind: string; targetKey: string; label: string };

test("genre and theme filters open the full pool and clearing them restores home", () => {
  assert.equal(discoveryBrowseSectionId(null, {}), null);
  assert.equal(discoveryBrowseSectionId(null, { minimumYear: 2000 }), null);
  assert.equal(
    discoveryBrowseSectionId(null, { genreTagKeys: ["canonical:genre:drama"] }),
    "browse_all",
  );
  assert.equal(
    discoveryBrowseSectionId(null, { themeTagKeys: ["canonical:theme:heist"] }),
    "browse_all",
  );
  assert.equal(
    discoveryBrowseSectionId(null, {
      genreTagKeys: [],
      themeTagKeys: [],
      minimumYear: 2000,
    }),
    null,
  );
  assert.equal(
    discoveryBrowseSectionId("browse_genre_drama", {
      themeTagKeys: ["canonical:theme:heist"],
    }),
    "browse_genre_drama",
  );
});

function card(index: number, kind = "MOVIE"): Card {
  return {
    targetKind: kind,
    targetKey: `title-${index}`,
    label: `Synthetic Title ${index}`,
  };
}

function cards(from: number, to: number): Card[] {
  return Array.from({ length: to - from }, (_, index) => card(from + index));
}

const GEN = ["context-1", "public-1"];

const QUERY: DiscoverySectionQuery = {
  sectionId: "for_you",
  sort: "RELEVANCE",
  filters: {},
  hideOwned: true,
  hideRequested: false,
};

test("pages append in order and stop at the section total", () => {
  const key = discoverySectionQueryKey(QUERY);
  let state = startDiscoverySectionPaging<Card>(key);
  assert.deepEqual(nextDiscoverySectionPage(state, 2), { offset: 0, limit: 2 });
  state = appendDiscoverySectionPage(state, {
    queryKey: key,
    offset: 0,
    items: cards(0, 2),
    totalCount: 3,
    generationIds: GEN,
  });
  assert.equal(state.totalCount, 3);
  assert.deepEqual(nextDiscoverySectionPage(state, 2), { offset: 2, limit: 2 });
  state = appendDiscoverySectionPage(state, {
    queryKey: key,
    offset: 2,
    items: cards(2, 3),
    totalCount: 3,
    generationIds: GEN,
  });
  assert.deepEqual(
    state.items.map((item) => item.targetKey),
    ["title-0", "title-1", "title-2"],
  );
  assert.equal(state.exhausted, true);
  assert.equal(nextDiscoverySectionPage(state, 2), null);
});

test("a title a later page repeats keeps its first place", () => {
  const key = discoverySectionQueryKey(QUERY);
  let state = startDiscoverySectionPaging<Card>(key);
  state = appendDiscoverySectionPage(state, {
    queryKey: key,
    offset: 0,
    items: cards(0, 3),
    totalCount: 6,
    generationIds: GEN,
  });
  state = appendDiscoverySectionPage(state, {
    queryKey: key,
    offset: 3,
    items: [card(1), card(3), card(3), card(1, "SERIES")],
    totalCount: 6,
    generationIds: GEN,
  });
  assert.deepEqual(
    state.items.map((item) => `${item.targetKind}:${item.targetKey}`),
    [
      "MOVIE:title-0",
      "MOVIE:title-1",
      "MOVIE:title-2",
      "MOVIE:title-3",
      "SERIES:title-1",
    ],
  );
  // The server offset still advances by the page it sent, so the next page
  // does not ask for the repeated rows again.
  assert.equal(state.offset, 7);
  assert.equal(state.exhausted, true);
});

test("a stale or out-of-order page leaves the grid as it is", () => {
  const key = discoverySectionQueryKey(QUERY);
  const state = appendDiscoverySectionPage(
    startDiscoverySectionPaging<Card>(key),
    {
      queryKey: key,
      offset: 0,
      items: cards(0, 2),
      totalCount: 10,
      generationIds: GEN,
    },
  );
  const otherKey = discoverySectionQueryKey({ ...QUERY, sort: "TITLE" });
  assert.equal(
    appendDiscoverySectionPage(state, {
      queryKey: otherKey,
      offset: 2,
      items: cards(2, 4),
      totalCount: 10,
      generationIds: GEN,
    }),
    state,
  );
  assert.equal(
    appendDiscoverySectionPage(state, {
      queryKey: key,
      offset: 0,
      items: cards(0, 2),
      totalCount: 10,
      generationIds: GEN,
    }),
    state,
  );
});

test("an empty page ends paging even when the total says more remain", () => {
  const key = discoverySectionQueryKey(QUERY);
  const state = appendDiscoverySectionPage(
    startDiscoverySectionPaging<Card>(key),
    { queryKey: key, offset: 0, items: [], totalCount: 12, generationIds: GEN },
  );
  assert.equal(state.exhausted, true);
  assert.equal(nextDiscoverySectionPage(state), null);
});

test("changing the sort, filters or toggles resets the cursor", () => {
  const key = discoverySectionQueryKey(QUERY);
  const loaded = appendDiscoverySectionPage(
    startDiscoverySectionPaging<Card>(key),
    {
      queryKey: key,
      offset: 0,
      items: cards(0, 40),
      totalCount: 90,
      generationIds: GEN,
    },
  );
  assert.equal(discoverySectionPagingFor(loaded, key), loaded);
  for (const changed of [
    { ...QUERY, sort: "RATING" as const },
    { ...QUERY, hideOwned: false },
    { ...QUERY, hideRequested: true },
    { ...QUERY, filters: { genreTagKeys: ["drama"] } },
    { ...QUERY, filters: { minimumYear: 2001 } },
    { ...QUERY, sectionId: "top_rated" },
  ]) {
    const changedKey = discoverySectionQueryKey(changed);
    assert.notEqual(changedKey, key);
    const reset = discoverySectionPagingFor(loaded, changedKey);
    assert.equal(reset.offset, 0);
    assert.deepEqual(reset.items, []);
    assert.equal(reset.totalCount, null);
    assert.deepEqual(nextDiscoverySectionPage(reset), {
      offset: 0,
      limit: 40,
    });
  }
});

test("the query key ignores filter list order", () => {
  assert.equal(
    discoverySectionQueryKey({
      ...QUERY,
      filters: { genreTagKeys: ["drama", "comedy"], contentTypes: ["MOVIE"] },
    }),
    discoverySectionQueryKey({
      ...QUERY,
      filters: { contentTypes: ["MOVIE"], genreTagKeys: ["comedy", "drama"] },
    }),
  );
});

test("a page from rebuilt generations restarts the grid from the top", () => {
  const key = discoverySectionQueryKey(QUERY);
  let state = appendDiscoverySectionPage(
    startDiscoverySectionPaging<Card>(key),
    {
      queryKey: key,
      offset: 0,
      items: cards(0, 2),
      totalCount: 6,
      generationIds: GEN,
    },
  );
  state = appendDiscoverySectionPage(state, {
    queryKey: key,
    offset: 2,
    items: cards(2, 4),
    totalCount: 6,
    generationIds: GEN,
  });
  assert.equal(state.offset, 4);
  const restarted = appendDiscoverySectionPage(state, {
    queryKey: key,
    offset: 4,
    items: cards(10, 12),
    totalCount: 6,
    generationIds: ["context-1", "public-2"],
  });
  assert.equal(restarted.queryKey, key);
  assert.equal(restarted.offset, 0);
  assert.deepEqual(restarted.items, []);
  assert.equal(restarted.totalCount, null);
  assert.equal(restarted.generationIds, null);
  assert.deepEqual(nextDiscoverySectionPage(restarted, 2), {
    offset: 0,
    limit: 2,
  });
  const reloaded = appendDiscoverySectionPage(restarted, {
    queryKey: key,
    offset: 0,
    items: cards(10, 12),
    totalCount: 6,
    generationIds: ["context-1", "public-2"],
  });
  assert.deepEqual(
    reloaded.items.map((item) => item.targetKey),
    ["title-10", "title-11"],
  );
});

test("cards without a target key are never folded together", () => {
  const key = discoverySectionQueryKey(QUERY);
  const blank = {
    targetKind: "MOVIE",
    targetKey: "",
    label: "Synthetic Blank",
  };
  const state = appendDiscoverySectionPage(
    startDiscoverySectionPaging<Card>(key),
    {
      queryKey: key,
      offset: 0,
      items: [blank, card(0), { ...blank }, card(0)],
      totalCount: 4,
      generationIds: GEN,
    },
  );
  assert.equal(state.items.length, 3);
  assert.equal(state.offset, 4);
});

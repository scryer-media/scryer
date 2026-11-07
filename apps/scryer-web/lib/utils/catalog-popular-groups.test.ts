import assert from "node:assert/strict";
import test from "node:test";
import type {
  CatalogDiscoveryGroup,
  CatalogDiscoveryItem,
} from "../types/discovery.ts";
import {
  mergeMoviePopularGroups,
  MOVIE_POPULAR_GROUP_ID,
} from "./catalog-popular-groups.ts";

function item(key: string): CatalogDiscoveryItem {
  return {
    id: key,
    targetKey: key,
    targetKind: "MOVIE",
    resolved: false,
    resolvedTitleId: null,
    displayTitle: key,
    originalTitle: null,
    sortTitle: null,
    year: null,
    posterUrl: null,
    contentType: "MOVIE",
    isAdult: false,
    statusTags: [],
    sourceTags: [],
    rankScore: 0,
    ownedInInput: false,
  };
}

function group(
  section: string,
  keys: string[],
  top = false,
): CatalogDiscoveryGroup {
  return {
    id: top ? "public_top_movie" : `public_section_${section}`,
    kind: top ? "PUBLIC_TOP" : "PUBLIC_SECTION",
    surface: "PUBLIC",
    labelKey: top ? section : null,
    labelValue: "Localized source title",
    totalCount: 100,
    items: keys.map(item),
  };
}

test("merges all three popularity sources at their first position without duplicate titles or mutation", () => {
  const collection = {
    ...group("collection", ["collection"]),
    kind: "COMPLETE_COLLECTION" as const,
  };
  const other = group("upcoming_movies", ["upcoming"]);
  const groups = [
    collection,
    group("trending_now", ["a", "b"], true),
    other,
    group("popular_right_now", ["b", "c"]),
    group("popular_movies", ["d", "a"]),
  ];
  const before = structuredClone(groups);
  const result = mergeMoviePopularGroups(groups);
  assert.deepEqual(
    result.map((g) => g.id),
    [collection.id, MOVIE_POPULAR_GROUP_ID, other.id],
  );
  assert.deepEqual(
    result[1].items.map((i) => i.targetKey),
    ["a", "b", "c", "d"],
  );
  assert.equal(result[1].totalCount, 4);
  assert.deepEqual(groups, before);
});

test("each source alone still becomes Popular, including when the top rail is absent", () => {
  for (const section of [
    "trending_now",
    "popular_right_now",
    "popular_movies",
  ]) {
    for (const top of [true, false]) {
      const result = mergeMoviePopularGroups([group(section, ["a"], top)]);
      assert.equal(result.length, 1);
      assert.equal(result[0].id, MOVIE_POPULAR_GROUP_ID);
      assert.equal(result[0].items.length, 1);
    }
  }
  assert.deepEqual(mergeMoviePopularGroups([]), []);
});

test("does not infer a source from display text or merge unrelated and personalized groups", () => {
  const unrelatedTop = group("upcoming_movies", ["a"], true);
  unrelatedTop.labelValue = "Popular Right Now";
  const personalized = {
    ...group("popular_movies", ["b"]),
    surface: "PERSONALIZED" as const,
  };
  const groups = [
    unrelatedTop,
    personalized,
    group("evergreen_popular", ["c"]),
  ];
  assert.deepEqual(mergeMoviePopularGroups(groups), groups);
});

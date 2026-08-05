import assert from "node:assert/strict";
import test from "node:test";

import {
  orderDiscoveryHomeSections,
  type OrderableDiscoveryItem,
  type OrderableDiscoverySection,
} from "./discovery-sections.ts";

function item(key: string, targetKind = "MOVIE"): OrderableDiscoveryItem {
  return { targetKind, targetKey: key };
}

function section(
  sectionType: string,
  // Three distinct items by default: the thin-rail floor hides public rails
  // under MIN_PUBLIC_RAIL_ITEMS, and per-section keys keep the cross-rail
  // dedupe from eating fixture rails unless a test overlaps them on purpose.
  items: readonly OrderableDiscoveryItem[] = [
    item(`${sectionType}-1`),
    item(`${sectionType}-2`),
    item(`${sectionType}-3`),
  ],
): OrderableDiscoverySection {
  return {
    sectionId: sectionType.toLowerCase(),
    sectionType,
    totalCount: items.length,
    items,
  };
}

function order(input: {
  publicSections?: OrderableDiscoverySection[];
  personalizedSections?: OrderableDiscoverySection[];
  completeCollection?: OrderableDiscoverySection | null;
}) {
  return orderDiscoveryHomeSections({
    publicSections: input.publicSections ?? [],
    personalizedSections: input.personalizedSections ?? [],
    completeCollection: input.completeCollection ?? null,
  });
}

function orderedTypes(input: Parameters<typeof order>[0]) {
  return order(input).map((ordered) => ordered.sectionType);
}

function itemKeys(sections: readonly OrderableDiscoverySection[]) {
  return sections.flatMap((entry) =>
    entry.items.map((entryItem) => `${entryItem.targetKind}:${entryItem.targetKey}`),
  );
}

// Every section type Scryer can currently receive, on the surface it arrives on.
const CURRENT_PUBLIC_SECTION_TYPES = [
  "TRENDING_NOW",
  "POPULAR_RIGHT_NOW",
  "POPULAR_MOVIES",
  "POPULAR_SERIES",
  "UPCOMING_MOVIES",
  "UPCOMING_SERIES",
  "UPCOMING_NEXT_SEASON",
  "POPULAR_WITH_ANIME_FANS",
  "ANIME_THIS_WEEK",
  "ANIME_SEASON_STANDOUTS",
  "MOST_ANTICIPATED_ANIME",
  "EVERGREEN_POPULAR",
  "NEW_ON_STREAMING",
  "NEW_ON_PHYSICAL",
  "CURATED",
  "TOP_RATED",
];

test("world shelf renders in a fixed order regardless of feed order", () => {
  assert.deepEqual(
    orderedTypes({
      publicSections: [
        section("CURATED"),
        section("MOST_ANTICIPATED_ANIME"),
        section("EVERGREEN_POPULAR"),
        section("UPCOMING_SERIES"),
        section("POPULAR_RIGHT_NOW"),
        section("UPCOMING_NEXT_SEASON"),
        section("ANIME_SEASON_STANDOUTS"),
        section("TOP_RATED"),
        section("POPULAR_MOVIES"),
        section("UPCOMING_MOVIES"),
        section("POPULAR_SERIES"),
        section("ANIME_THIS_WEEK"),
      ],
    }),
    [
      "POPULAR_RIGHT_NOW",
      "POPULAR_MOVIES",
      "POPULAR_SERIES",
      "UPCOMING_MOVIES",
      "UPCOMING_SERIES",
      "TOP_RATED",
      "EVERGREEN_POPULAR",
      "ANIME_THIS_WEEK",
      "ANIME_SEASON_STANDOUTS",
      "MOST_ANTICIPATED_ANIME",
      "UPCOMING_NEXT_SEASON",
      "CURATED",
    ],
  );
});

test("TOP_RATED takes its world slot when it arrives on the public surface", () => {
  // Scryer routes TOP_RATED to public_sections when no item is personalized.
  assert.deepEqual(
    orderedTypes({
      publicSections: [
        section("EVERGREEN_POPULAR"),
        section("TOP_RATED"),
        section("UPCOMING_MOVIES"),
      ],
    }),
    ["UPCOMING_MOVIES", "TOP_RATED", "EVERGREEN_POPULAR"],
  );
});

test("unrecognised public sections render after the ordered ones, and Editor's Picks stays last", () => {
  assert.deepEqual(
    orderedTypes({
      publicSections: [
        section("CURATED"),
        section("SOME_BRAND_NEW_SMG_RAIL"),
        section("POPULAR_RIGHT_NOW"),
      ],
    }),
    ["POPULAR_RIGHT_NOW", "SOME_BRAND_NEW_SMG_RAIL", "CURATED"],
  );
});

test("promotion rails sit between the personalized shelf and the world shelf", () => {
  assert.deepEqual(
    orderedTypes({
      publicSections: [
        section("POPULAR_RIGHT_NOW"),
        section("NEW_ON_PHYSICAL"),
        section("NEW_ON_STREAMING"),
      ],
      personalizedSections: [section("BECAUSE_YOU_LIKE_GENRE")],
    }),
    [
      "BECAUSE_YOU_LIKE_GENRE",
      "NEW_ON_STREAMING",
      "NEW_ON_PHYSICAL",
      "POPULAR_RIGHT_NOW",
    ],
  );
});

test("personalized shelf leads with the most specific reason", () => {
  assert.deepEqual(
    orderedTypes({
      personalizedSections: [
        section("TOP_RATED"),
        section("TOP_ANIME_THIS_WEEK"),
        section("BECAUSE_YOU_LIKE_GENRE"),
        section("SOMETHING_UNRECOGNISED"),
        section("TOP_MOVIES_THIS_WEEK"),
        section("BECAUSE_YOU_LIKE_TAG"),
        section("TOP_SERIES_THIS_WEEK"),
      ],
      completeCollection: section("COMPLETE_THE_COLLECTION"),
    }),
    [
      "BECAUSE_YOU_LIKE_TAG",
      "BECAUSE_YOU_LIKE_GENRE",
      "TOP_MOVIES_THIS_WEEK",
      "TOP_SERIES_THIS_WEEK",
      "TOP_ANIME_THIS_WEEK",
      "TOP_RATED",
      "COMPLETE_THE_COLLECTION",
      "SOMETHING_UNRECOGNISED",
    ],
  );
});

test("every Because You Like rail of a kind is kept, in payload order", () => {
  const isekai = {
    ...section("BECAUSE_YOU_LIKE_TAG", [item("isekai-1")]),
    sectionId: "isekai",
  };
  const slowBurn = {
    ...section("BECAUSE_YOU_LIKE_TAG", [item("slow-burn-1")]),
    sectionId: "slow_burn",
  };
  const animation = {
    ...section("BECAUSE_YOU_LIKE_GENRE", [item("animation-1")]),
    sectionId: "animation",
  };
  assert.deepEqual(
    order({ personalizedSections: [animation, isekai, slowBurn] }).map(
      (entry) => entry.sectionId,
    ),
    ["isekai", "slow_burn", "animation"],
  );
});

test("FOR_YOU renders only when there is no library context", () => {
  assert.deepEqual(orderedTypes({ personalizedSections: [section("FOR_YOU")] }), [
    "FOR_YOU",
  ]);
  assert.deepEqual(
    orderedTypes({
      personalizedSections: [section("FOR_YOU"), section("BECAUSE_YOU_LIKE_TAG")],
    }),
    ["BECAUSE_YOU_LIKE_TAG"],
  );
  assert.deepEqual(
    orderedTypes({
      personalizedSections: [section("FOR_YOU")],
      completeCollection: section("COMPLETE_THE_COLLECTION"),
    }),
    ["COMPLETE_THE_COLLECTION"],
  );
});

test("retired rails in older payloads never render and never count as context", () => {
  // An old snapshot still carries these; they must not produce a ghost rail, and
  // they must not suppress the FOR_YOU fallback by looking like library context.
  assert.deepEqual(
    orderedTypes({
      personalizedSections: [
        section("BECAUSE_YOU_HAVE"),
        section("MOVIES_FOR_YOU"),
        section("SERIES_FOR_YOU"),
        section("ANIME_FOR_YOU"),
        section("FOR_YOU"),
      ],
    }),
    ["FOR_YOU"],
  );
});

test("legacy section types are recognised regardless of case and whitespace", () => {
  assert.deepEqual(
    orderedTypes({
      personalizedSections: [
        section("  because_you_have  "),
        section("Movies_For_You"),
        section("FOR_YOU"),
      ],
    }),
    ["FOR_YOU"],
  );
  // ...and so are live types.
  assert.deepEqual(
    orderedTypes({ publicSections: [section(" curated "), section("popular_right_now")] }),
    ["popular_right_now", " curated "],
  );
});

test("empty sections are dropped everywhere, including the complete-collection slot", () => {
  assert.deepEqual(
    orderedTypes({
      publicSections: [section("POPULAR_RIGHT_NOW", []), section("CURATED")],
      personalizedSections: [section("BECAUSE_YOU_LIKE_TAG", [])],
      completeCollection: section("COMPLETE_THE_COLLECTION", []),
    }),
    ["CURATED"],
  );
});

test("a legacy complete-collection payload never renders", () => {
  assert.deepEqual(
    orderedTypes({
      personalizedSections: [section("FOR_YOU")],
      completeCollection: section("MOVIES_FOR_YOU"),
    }),
    ["FOR_YOU"],
  );
});

test("duplicate section types all render, in payload order", () => {
  const first = {
    ...section("POPULAR_MOVIES", [item("pm-a1"), item("pm-a2"), item("pm-a3")]),
    sectionId: "popular_movies_a",
  };
  const second = {
    ...section("POPULAR_MOVIES", [item("pm-b1"), item("pm-b2"), item("pm-b3")]),
    sectionId: "popular_movies_b",
  };
  assert.deepEqual(
    order({ publicSections: [first, second] }).map((entry) => entry.sectionId),
    ["popular_movies_a", "popular_movies_b"],
  );
});

test("merge: TRENDING_NOW folds into POPULAR_RIGHT_NOW, target items first", () => {
  const trending = section("TRENDING_NOW", [item("t1"), item("t2")]);
  const rightNow = section("POPULAR_RIGHT_NOW", [item("r1")]);
  const ordered = order({ publicSections: [trending, rightNow] });

  assert.deepEqual(
    ordered.map((entry) => entry.sectionType),
    ["POPULAR_RIGHT_NOW"],
  );
  assert.deepEqual(itemKeys(ordered), [
    "MOVIE:r1",
    "MOVIE:t1",
    "MOVIE:t2",
  ]);
  assert.equal(ordered[0]?.totalCount, 3);
});

test("merge: POPULAR_WITH_ANIME_FANS folds into ANIME_THIS_WEEK", () => {
  const fans = section("POPULAR_WITH_ANIME_FANS", [
    item("f1", "ANIME"),
    item("f2", "ANIME"),
  ]);
  const weekly = section("ANIME_THIS_WEEK", [item("w1", "ANIME")]);
  const ordered = order({
    publicSections: [fans, weekly, section("ANIME_SEASON_STANDOUTS")],
  });

  assert.deepEqual(
    ordered.map((entry) => entry.sectionType),
    ["ANIME_THIS_WEEK", "ANIME_SEASON_STANDOUTS"],
  );
  assert.deepEqual(itemKeys([ordered[0]!]), ["ANIME:w1", "ANIME:f1", "ANIME:f2"]);
});

test("merge: a source whose target is absent renders itself in the target's slot", () => {
  const ordered = order({
    publicSections: [
      section("EVERGREEN_POPULAR"),
      section("TRENDING_NOW", [item("t1"), item("t2"), item("t3")]),
      section("POPULAR_MOVIES"),
    ],
  });
  // TRENDING_NOW takes POPULAR_RIGHT_NOW's slot: ahead of POPULAR_MOVIES.
  assert.deepEqual(
    ordered.map((entry) => entry.sectionType),
    ["TRENDING_NOW", "POPULAR_MOVIES", "EVERGREEN_POPULAR"],
  );
  assert.deepEqual(itemKeys(ordered).includes("MOVIE:t1"), true);
});

test("merge: overlapping items are deduped defensively", () => {
  const trending = section("TRENDING_NOW", [item("shared"), item("t1")]);
  const rightNow = section("POPULAR_RIGHT_NOW", [item("shared"), item("r1")]);
  const ordered = order({ publicSections: [trending, rightNow] });

  assert.deepEqual(itemKeys(ordered), ["MOVIE:shared", "MOVIE:r1", "MOVIE:t1"]);
});

test("presentation invariant: every unique title renders exactly once", () => {
  // Replaces the old "no title is ever deleted" property: with Scryer owning
  // cross-rail dedupe, a title may ARRIVE in several sections but must RENDER
  // exactly once, in its first reading-order home. Unique titles are only
  // withheld by the thin-rail floor; nothing renders twice anywhere.
  const publicSections = CURRENT_PUBLIC_SECTION_TYPES.map((sectionType) =>
    section(sectionType, [
      item(`${sectionType}-a`),
      item(`${sectionType}-b`),
      item(`${sectionType}-c`),
      // Every public rail also carries the same duplicated headline title, the
      // shape the gateway now ships (no server-side cross-section dedupe).
      item("shared-headliner"),
    ]),
  );
  const personalizedSections = [
    section("BECAUSE_YOU_LIKE_TAG", [item("tag-a")]),
    section("BECAUSE_YOU_LIKE_GENRE", [item("genre-a")]),
    section("FOR_YOU", [item("foryou-a")]),
  ];
  const completeCollection = section("COMPLETE_THE_COLLECTION", [
    item("collection-a"),
  ]);

  const ordered = order({
    publicSections,
    personalizedSections,
    completeCollection,
  });
  const renderedKeys = itemKeys(ordered);
  const counts = new Map<string, number>();
  for (const key of renderedKeys) {
    counts.set(key, (counts.get(key) ?? 0) + 1);
  }
  for (const [key, count] of counts) {
    assert.equal(count, 1, `item ${key} rendered ${count} times`);
  }
  assert.equal(counts.get("MOVIE:shared-headliner"), 1);

  const expected = itemKeys([
    ...publicSections,
    // FOR_YOU is a no-context fallback and is legitimately withheld here.
    ...personalizedSections.filter((entry) => entry.sectionType !== "FOR_YOU"),
    completeCollection,
  ]);
  for (const key of expected) {
    assert.ok(counts.has(key), `unique item ${key} vanished from the dashboard`);
  }
});

test("thin-rail floor: a public rail under 3 unique items hides; personalized rails do not", () => {
  const ordered = order({
    publicSections: [
      section("POPULAR_RIGHT_NOW"),
      section("EVERGREEN_POPULAR", [item("e1"), item("e2")]),
    ],
    personalizedSections: [section("BECAUSE_YOU_LIKE_TAG", [item("thin-tag")])],
  });
  assert.deepEqual(
    ordered.map((entry) => entry.sectionType),
    ["BECAUSE_YOU_LIKE_TAG", "POPULAR_RIGHT_NOW"],
  );
});

test("cross-rail dedupe: first reading-order occurrence wins across shelves", () => {
  const shared = item("cross-shelf-title");
  const ordered = order({
    publicSections: [
      section("POPULAR_RIGHT_NOW", [shared, item("p1"), item("p2"), item("p3")]),
    ],
    personalizedSections: [
      section("BECAUSE_YOU_LIKE_TAG", [shared, item("tag-b")]),
    ],
  });
  // The personalized shelf reads first, so it claims the shared title.
  assert.deepEqual(itemKeys([ordered[0]!]), [
    "MOVIE:cross-shelf-title",
    "MOVIE:tag-b",
  ]);
  assert.deepEqual(itemKeys([ordered[1]!]), ["MOVIE:p1", "MOVIE:p2", "MOVIE:p3"]);
});

test("a hero sourced from TRENDING_NOW stays visible in the rendered rails", () => {
  // The server picks the hero from the global top public item, which is almost
  // always a TRENDING_NOW member. If that rail stopped rendering, the view's
  // hero-visibility check would blank the hero and leave a headless dashboard.
  const heroItem = item("hero-title");
  const ordered = order({
    publicSections: [
      section("TRENDING_NOW", [heroItem, item("t2")]),
      section("POPULAR_RIGHT_NOW", [item("r1")]),
    ],
  });
  assert.ok(itemKeys(ordered).includes("MOVIE:hero-title"));
});

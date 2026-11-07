import assert from "node:assert/strict";
import test from "node:test";

import en from "../i18n/locales/en.ts";
import { interpolate } from "../i18n/types.ts";
import type { TitleRecord } from "../types/titles.ts";
import { UNPLACED_CLUSTER_ID } from "../types/library-graph.ts";
import {
  LIBRARY_GRAPH_FIXTURE_NODE_COUNT,
  fixtureSubjectKey,
  fixtureTargetKey,
  libraryGraphFixture,
} from "./library-graph-fixture.ts";
import {
  clusterNodeKey,
  mergeLibraryGraphResult,
} from "./library-graph-merge.ts";
import {
  LIBRARY_GRAPH_MAX_NODE_SIZE,
  LIBRARY_GRAPH_MIN_NODE_SIZE,
  displayedLibraryGraph,
  graphDetailClusters,
  graphDetailPosterSize,
  graphDetailOpacity,
  graphNeighborhoodBounds,
  largeLibraryGraphLayout,
  libraryGraphOverviewPreview,
  joinLibraryGraphToTitles,
  sameTitleRecords,
  libraryGraphClusterName,
  libraryGraphEdgeKindLabel,
  libraryGraphEdgeKindLabels,
  libraryGraphGroupName,
  libraryGraphNamedClusterIds,
  neighborReason,
  neighborReasonLines,
  recommendationSectionName,
  recommendationSections,
  nodeSizesByTitleId,
  targetKeyMatchKeys,
  titleForTargetKey,
  titleMatchKeys,
  titleIndexByMatchKey,
  titleTargetKey,
} from "./library-graph-model.ts";
import { createFixtureLibraryGraphSource } from "./library-graph-source.ts";
import { buildTitlesQuery } from "../graphql/queries.ts";
import {
  titleCatalogProjectionForGraph,
  titleCatalogProjectionForTable,
} from "./title-catalog-query.ts";

test("legend framing includes complete selected neighborhoods and resets to all regions", () => {
  const regions = [
    { id: 1, left: -100, right: -20, top: 10, bottom: 90 },
    { id: 2, left: 50, right: 200, top: -60, bottom: 80 },
    { id: -1, left: 500, right: 600, top: 500, bottom: 600 },
  ];
  assert.deepEqual(graphNeighborhoodBounds(regions, new Set([1])), {
    left: -100,
    right: -20,
    top: 10,
    bottom: 90,
  });
  assert.deepEqual(graphNeighborhoodBounds(regions, new Set([1, 2])), {
    left: -100,
    right: 200,
    top: -60,
    bottom: 90,
  });
  assert.deepEqual(graphNeighborhoodBounds(regions, new Set()), {
    left: -100,
    right: 600,
    top: -60,
    bottom: 600,
  });
  assert.equal(graphNeighborhoodBounds(regions, new Set([99])), null);
  assert.equal(graphNeighborhoodBounds([], new Set()), null);
});

const dictionary: Record<string, string> = en;
const t = (
  key: string,
  values?: Record<string, string | number | boolean | null | undefined>,
) => {
  const template = dictionary[key];
  return template === undefined ? key : interpolate(template, values);
};

function fixtureTitle(index: number, overrides: Partial<TitleRecord> = {}) {
  return {
    id: `title-${index}`,
    name: `Synthetic Title ${index}`,
    facet: "MOVIE",
    libraryId: "library-1",
    monitored: true,
    tags: [],
    externalIds: [{ source: "tmdb", value: String(910_000 + index) }],
    ...overrides,
  } satisfies TitleRecord;
}

function catalog(count: number): TitleRecord[] {
  return Array.from({ length: count }, (_, index) => fixtureTitle(index));
}

test("the fixture is deterministic and covers every shape the view handles", () => {
  const first = libraryGraphFixture({ facet: "MOVIE" });
  assert.deepEqual(libraryGraphFixture({ facet: "MOVIE" }), first);
  assert.ok(first.nodes.length >= 60);
  assert.equal(first.nodes.length, LIBRARY_GRAPH_FIXTURE_NODE_COUNT);
  assert.equal(first.clusters.length, 3);
  assert.equal(
    first.nodes.filter((node) => node.clusterId === UNPLACED_CLUSTER_ID).length,
    1,
  );
  assert.equal(
    first.clusters.filter((cluster) => cluster.labelKey === null).length,
    1,
  );
  const kinds = new Set(first.edges.flatMap((edge) => edge.kinds));
  for (const kind of [
    "semantic",
    "relation:sequel",
    "relation:prequel",
    "relation:co_watch",
    "affinity:theme:isekai",
    "affinity:genre:drama",
  ]) {
    assert.ok(kinds.has(kind), `fixture lacks ${kind}`);
  }
  const keys = new Set(first.nodes.map((node) => node.subjectKey));
  for (const edge of first.edges) {
    assert.ok(keys.has(edge.from) && keys.has(edge.to));
    assert.ok(edge.weight > 0 && edge.weight <= 1);
  }
  for (const cluster of first.clusters) {
    assert.ok(keys.has(cluster.medoid));
  }
});

test("the fixture source answers with the fixture, naming nodes by the given keys", async () => {
  const source = createFixtureLibraryGraphSource(["tmdb:series:7"]);
  const loaded = await source.load({ facet: "SERIES", libraryIds: [] });
  assert.equal(loaded.nodes[0].targetKey, "tmdb:series:7");
  assert.equal(loaded.nodes[1].targetKey, fixtureTargetKey("SERIES", 1));
});

test("titles answer to the server's target keys, kind and all", () => {
  // Keys normalize the way the server writes them.
  assert.deepEqual(targetKeyMatchKeys("TMDB:movie:0042"), ["tmdb:movie:42"]);
  assert.deepEqual(targetKeyMatchKeys("imdb:title:0000006"), [
    "imdb:title:tt0000006",
  ]);
  // A key without a kind names nothing.
  assert.deepEqual(targetKeyMatchKeys("tmdb:42"), []);

  // TMDB and TVDB carry the facet's kind; anime is keyed as series.
  const movie = fixtureTitle(3);
  assert.equal(titleTargetKey(movie), "tmdb:movie:910003");
  const anime = fixtureTitle(4, {
    facet: "ANIME",
    externalIds: [{ source: "tmdb", value: "55" }],
  });
  assert.deepEqual(titleMatchKeys(anime), ["tmdb:series:55"]);

  // MAL, AniDB and AniList are always anime, whatever the title's facet.
  const series = fixtureTitle(8, {
    facet: "SERIES",
    externalIds: [
      { source: "tmdb", value: "123" },
      { source: "MyAnimeList", value: "77" },
      { source: "anidb", value: "88" },
      { source: "AniList", value: "99" },
    ],
    imdbId: "tt0000008",
  });
  assert.deepEqual(titleMatchKeys(series).sort(), [
    "anidb:anime:88",
    "anilist:anime:99",
    "imdb:title:tt0000008",
    "mal:anime:77",
    "tmdb:series:123",
  ]);
  const index = titleIndexByMatchKey([series]);
  assert.equal(titleForTargetKey("mal:anime:77", index), series);
  assert.equal(titleForTargetKey("imdb:title:tt0000008", index), series);
  // The same TMDB id under another kind is another title.
  assert.equal(titleForTargetKey("tmdb:movie:123", index), null);
  assert.equal(titleForTargetKey("mal:series:77", index), null);
  // The server's source order picks the key a title is named by.
  assert.equal(titleTargetKey(series), "tmdb:series:123");

  // IMDb alone names the title under `title`.
  const imdbOnly = fixtureTitle(6, { externalIds: [], imdbId: "tt0000006" });
  assert.equal(titleTargetKey(imdbOnly), "imdb:title:tt0000006");
  assert.equal(
    titleForTargetKey("imdb:title:tt0000006", titleIndexByMatchKey([imdbOnly])),
    imdbOnly,
  );
  assert.equal(titleTargetKey(fixtureTitle(7, { externalIds: [] })), null);
});

test("nodes whose title is filtered out are pruned with their edges", () => {
  const graph = libraryGraphFixture({ facet: "MOVIE" });
  const all = joinLibraryGraphToTitles(graph, catalog(64));
  assert.equal(all.graph.nodes.length, 64);

  // A filter that keeps only the first cluster's titles (the last node, 63,
  // is the unplaced one).
  const firstCluster = catalog(63).filter((_, index) => index % 3 === 0);
  const pruned = joinLibraryGraphToTitles(graph, firstCluster);
  assert.equal(pruned.graph.nodes.length, firstCluster.length);
  assert.ok(pruned.graph.nodes.every((node) => node.clusterId === 0));
  const kept = new Set(pruned.graph.nodes.map((node) => node.subjectKey));
  assert.ok(
    pruned.graph.edges.every(
      (edge) => kept.has(edge.from) && kept.has(edge.to),
    ),
  );
  assert.deepEqual(
    pruned.graph.clusters.map((cluster) => [cluster.id, cluster.size]),
    [[0, firstCluster.length]],
  );
  assert.equal(
    pruned.titleBySubjectKey.get(fixtureSubjectKey(3))?.id,
    "title-3",
  );
});

test("large overviews collapse named neighbourhoods and gather unnamed titles as unplaced", () => {
  const { graph } = joinLibraryGraphToTitles(
    libraryGraphFixture({ facet: "MOVIE" }),
    catalog(64),
  );
  assert.equal(displayedLibraryGraph(graph, new Set()).nodes.length, 64);

  const collapsed = displayedLibraryGraph(graph, new Set(), 60);
  assert.deepEqual(
    collapsed.nodes.map((node) => node.subjectKey).sort(),
    [
      clusterNodeKey(0),
      clusterNodeKey(1),
      clusterNodeKey(UNPLACED_CLUSTER_ID),
    ].sort(),
  );
  // Co-watch bridges join the cluster nodes.
  assert.ok(collapsed.edges.length >= 3);

  const oneOpen = displayedLibraryGraph(graph, new Set([1]), 60);
  assert.equal(oneOpen.nodes.filter((node) => node.clusterId === 1).length, 21);
  assert.equal(
    oneOpen.nodes.filter((node) => node.collapsedClusterId !== undefined)
      .length,
    2,
  );
});

test("neighbourhood filters use OR and preserve every unplaced title on expansion", () => {
  const { graph } = joinLibraryGraphToTitles(
    libraryGraphFixture({ facet: "MOVIE" }),
    catalog(64),
  );
  const expanded = new Set([0, 1, UNPLACED_CLUSTER_ID]);
  const filtered = displayedLibraryGraph(
    graph,
    expanded,
    60,
    graph,
    new Set([0, 1]),
  );
  assert.equal(filtered.nodes.length, 42);
  assert.ok(
    filtered.nodes.every(
      (node) => node.clusterId === 0 || node.clusterId === 1,
    ),
  );
  const visible = new Set(filtered.nodes.map((node) => node.subjectKey));
  assert.ok(
    filtered.edges.every(
      (edge) => visible.has(edge.from) && visible.has(edge.to),
    ),
  );
  const unplaced = displayedLibraryGraph(
    graph,
    expanded,
    60,
    graph,
    new Set([UNPLACED_CLUSTER_ID]),
  );
  assert.equal(unplaced.nodes.length, 22);
  assert.ok(
    unplaced.nodes.every((node) => node.collapsedClusterId === undefined),
  );
  assert.equal(
    displayedLibraryGraph(graph, expanded, 60, graph, new Set()).nodes.length,
    64,
  );
});

test("ten thousand titles retain stable nested positions and collapse to neighbourhoods", () => {
  const fixture = libraryGraphFixture({ facet: "MOVIE" });
  const graph = {
    clusters: fixture.clusters.map((cluster) => ({ ...cluster, size: 3334 })),
    edges: [],
    nodes: Array.from({ length: 10_000 }, (_, index) => ({
      subjectKey: `title:${index}`,
      targetKey: `tmdb:movie:${index}`,
      clusterId: index % 3,
      ghost: false,
    })),
  };
  const layout = largeLibraryGraphLayout(graph);
  const reordered = largeLibraryGraphLayout({
    ...graph,
    nodes: [...graph.nodes].reverse(),
  });
  assert.deepEqual(layout, reordered);
  assert.equal(layout.positions.size, 10_003);
  const named = libraryGraphNamedClusterIds(graph.clusters);
  for (const region of layout.regions) {
    const hull = layout.hulls.get(region.id)!;
    assert.ok(hull.length >= 3);
    const members = graph.nodes.filter(
      (node) =>
        (named.has(node.clusterId) ? node.clusterId : UNPLACED_CLUSTER_ID) ===
        region.id,
    );
    const points = members.map((node) =>
      layout.positions.get(node.subjectKey)!,
    );
    // A cluster boundary must contain its outermost titles before and after
    // expansion; changing the camera cannot repair a smaller overview shape.
    for (const point of points) {
      const crosses = hull.map((a, index) => {
        const b = hull[(index + 1) % hull.length];
        return (b.x - a.x) * (point.y - a.y) - (b.y - a.y) * (point.x - a.x);
      });
      assert.ok(
        crosses.every((cross) => cross >= -1e-6) ||
          crosses.every((cross) => cross <= 1e-6),
      );
    }
    const sampled = layout.dots.get(region.id)!;
    assert.equal(sampled.length, Math.min(600, members.length));
    const actual = new Set(points);
    assert.ok(sampled.every((point) => actual.has(point)));
  }
  const collapsed = displayedLibraryGraph(graph, new Set());
  assert.equal(collapsed.nodes.length, 3);
  assert.ok(
    collapsed.nodes.every((node) => node.collapsedClusterId !== undefined),
  );
  assert.equal(
    collapsed.nodes.reduce((sum, node) => sum + (node.memberCount ?? 0), 0),
    10_000,
  );
  for (const node of graph.nodes) {
    const point = layout.positions.get(node.subjectKey)!;
    const region = layout.regions.find(
      (region) => region.id === (node.clusterId === 2 ? -1 : node.clusterId),
    )!;
    assert.ok(
      point.x >= region.left &&
        point.x <= region.right &&
        point.y >= region.top &&
        point.y <= region.bottom,
    );
  }
});

test("overview preview includes neighborhood connections before catalog metadata", () => {
  const result = libraryGraphFixture({ facet: "MOVIE" });
  const { graph, titleBySubjectKey } = libraryGraphOverviewPreview(result);
  assert.equal(titleBySubjectKey.size, 0);
  assert.equal(graph.nodes.length, result.nodes.length);
  assert.equal(graph.edges, result.edges);
  const collapsed = displayedLibraryGraph(graph, new Set(), 1);
  assert.ok(
    collapsed.nodes.every((node) => node.collapsedClusterId !== undefined),
  );
  assert.ok(collapsed.edges.length > 0);
  const keys = new Set(collapsed.nodes.map((node) => node.subjectKey));
  assert.ok(
    collapsed.edges.every(
      (edge) =>
        edge.from !== edge.to && keys.has(edge.from) && keys.has(edge.to),
    ),
  );
  assert.equal(
    collapsed.nodes.reduce((sum, node) => sum + (node.memberCount ?? 0), 0),
    result.nodes.length,
  );
});

test("detail fades reversibly with eased endpoints inside materialization thresholds", () => {
  const regions = [{ id: 0, left: 0, right: 100, top: 0, bottom: 100 }];
  let expanded: ReadonlySet<number> = new Set();
  for (const spacing of [33, 40, 45, 55, 65, 70, 65, 55, 45, 40, 30]) {
    expanded = graphDetailClusters(expanded, regions, spacing, 800, 600);
    assert.equal(expanded.has(0), true);
    const opacity = graphDetailOpacity(spacing);
    assert.ok(opacity >= 0 && opacity <= 1);
    assert.ok(
      Math.abs(opacity + graphDetailOpacity(110 - spacing) - 1) < 1e-12,
    );
  }
  assert.equal(graphDetailOpacity(40), 0);
  assert.equal(graphDetailOpacity(55), 0.5);
  assert.equal(graphDetailOpacity(70), 1);
  assert.ok(graphDetailOpacity(40.1) < 0.001);
  assert.ok(graphDetailOpacity(69.9) > 0.999);
  assert.equal(graphDetailClusters(expanded, regions, 23, 800, 600).size, 0);
  assert.equal(graphDetailOpacity(23), 0);
});

test("zooming into detail grows posters and spacing together instead of canceling zoom", () => {
  for (const baseSize of [4, 12, 28]) {
    const initial = graphDetailPosterSize(baseSize, 60);
    let previous = initial;
    for (const spacing of [120, 240, 960, 10000]) {
      const size = graphDetailPosterSize(baseSize, spacing);
      assert.ok(size > previous);
      assert.ok(Math.abs(size / spacing - initial / 60) < 1e-9);
      previous = size;
    }
    assert.equal(graphDetailPosterSize(baseSize, 60), initial);
    assert.equal(graphDetailPosterSize(baseSize, NaN), initial);
  }
});

test("zoom detail opens only nearby regions with hysteresis and collapses when zooming out", () => {
  const regions = [
    { id: 0, left: 10, right: 30, top: 10, bottom: 30 },
    { id: 1, left: 2000, right: 2100, top: 10, bottom: 30 },
  ];
  const empty = new Set<number>();
  assert.equal(graphDetailClusters(empty, regions, 30, 800, 600), empty);
  const close = graphDetailClusters(empty, regions, 50, 800, 600);
  assert.deepEqual([...close], [0]);
  assert.equal(graphDetailClusters(close, regions, 35, 800, 600), close);
  assert.equal(graphDetailClusters(close, regions, 20, 800, 600).size, 0);
  assert.equal(
    graphDetailClusters(close, regions.slice(1), 50, 800, 600).size,
    0,
  );
  assert.equal(graphDetailClusters(close, regions, 50, 0, 0), close);
});

test("explored ghosts neither tip the graph into collapsing nor count as cluster members", () => {
  const { graph: owned } = joinLibraryGraphToTitles(
    libraryGraphFixture({ facet: "MOVIE" }),
    catalog(64),
  );
  const anchor = owned.nodes.find((node) => node.clusterId === 1);
  assert.ok(anchor);
  const explored = mergeLibraryGraphResult(owned, {
    generatedAt: "2026-01-01T00:00:00Z",
    nodes: [0, 1, 2, 3].map((index) => ({
      subjectKey: `ghost:tmdb:movie:${9000 + index}`,
      targetKey: `tmdb:movie:${9000 + index}`,
      clusterId: UNPLACED_CLUSTER_ID,
    })),
    edges: [0, 1, 2, 3].map((index) => ({
      from: anchor.subjectKey,
      to: `ghost:tmdb:movie:${9000 + index}`,
      weight: 0.5,
      kinds: ["relation:recommendation"],
    })),
    clusters: [],
  });
  assert.equal(explored.nodes.length, 68);

  // The library alone sits at the threshold, so nothing collapses even though
  // the explored graph is over it.
  const open = displayedLibraryGraph(explored, new Set(), 64, owned);
  assert.equal(open.nodes.length, 68);
  assert.ok(open.nodes.every((node) => node.collapsedClusterId === undefined));

  // Over the threshold, the anchor's collapsed cluster counts only owned
  // titles; the ghosts folded into it are not members.
  const collapsed = displayedLibraryGraph(explored, new Set(), 60, owned);
  const stand = collapsed.nodes.find(
    (node) => node.subjectKey === clusterNodeKey(1),
  );
  assert.equal(
    stand?.memberCount,
    owned.nodes.filter((node) => node.clusterId === 1).length,
  );
});

test("popularity ranks preserve tied midpoints and missing values", () => {
  const titles = [10, 10, 30, 50, 50, undefined].map((popularity, i) =>
    fixtureTitle(i, { popularity }),
  );
  const sizes = nodeSizesByTitleId(titles, "popularity");
  const span = LIBRARY_GRAPH_MAX_NODE_SIZE - LIBRARY_GRAPH_MIN_NODE_SIZE;
  [0.5, 0.5, 2, 3.5, 3.5].forEach((rank, i) =>
    assert.equal(
      sizes.get(`title-${i}`),
      LIBRARY_GRAPH_MIN_NODE_SIZE + (span * rank) / 4,
    ),
  );
  assert.equal(sizes.get("title-5"), LIBRARY_GRAPH_MIN_NODE_SIZE);
});

test("node sizes scale the chosen metric between the size bounds", () => {
  const titles = [
    fixtureTitle(1, {
      ratings: {
        rating: 8,
        ratingSources: ["imdb"],
        externalRatings: [{ source: "imdb", normalized: 0.8, votes: 100_000 }],
      },
      popularity: 5,
    }),
    fixtureTitle(2, {
      ratings: {
        rating: 6,
        ratingSources: ["imdb"],
        externalRatings: [{ source: "imdb", normalized: 0.6, votes: 10 }],
      },
      popularity: 50,
    }),
    fixtureTitle(3),
  ];
  const evidence = nodeSizesByTitleId(titles, "ratingEvidence");
  assert.equal(evidence.get("title-1"), LIBRARY_GRAPH_MAX_NODE_SIZE);
  assert.equal(evidence.get("title-2"), LIBRARY_GRAPH_MIN_NODE_SIZE);
  assert.equal(evidence.get("title-3"), LIBRARY_GRAPH_MIN_NODE_SIZE);

  const popularity = nodeSizesByTitleId(titles, "popularity");
  assert.equal(popularity.get("title-2"), LIBRARY_GRAPH_MAX_NODE_SIZE);
  assert.equal(popularity.get("title-1"), LIBRARY_GRAPH_MIN_NODE_SIZE);

  // Popularity sizes by rank, so one huge outlier does not flatten the rest.
  const skewed = nodeSizesByTitleId(
    [1, 2, 3, 4, 1_000_000].map((popularity, index) =>
      fixtureTitle(index + 1, { popularity }),
    ),
    "popularity",
  );
  const span = LIBRARY_GRAPH_MAX_NODE_SIZE - LIBRARY_GRAPH_MIN_NODE_SIZE;
  assert.equal(skewed.get("title-2"), LIBRARY_GRAPH_MIN_NODE_SIZE + span / 4);
  assert.equal(
    skewed.get("title-4"),
    LIBRARY_GRAPH_MIN_NODE_SIZE + (span * 3) / 4,
  );
  assert.equal(skewed.get("title-5"), LIBRARY_GRAPH_MAX_NODE_SIZE);

  // One rated title alone sits at the midpoint.
  const single = nodeSizesByTitleId([titles[0]], "rating");
  assert.equal(
    single.get("title-1"),
    (LIBRARY_GRAPH_MIN_NODE_SIZE + LIBRARY_GRAPH_MAX_NODE_SIZE) / 2,
  );
});

test("clusters are named from the label table, else by their medoid title", () => {
  const graph = libraryGraphFixture({ facet: "MOVIE" });
  const { titleBySubjectKey, graph: joined } = joinLibraryGraphToTitles(
    graph,
    catalog(64),
  );
  const [isekai, timeTravel, unlabelled] = joined.clusters;
  assert.equal(libraryGraphClusterName(isekai, titleBySubjectKey, t), "Isekai");
  // A key carrying its family still finds the label.
  assert.equal(
    libraryGraphClusterName(timeTravel, titleBySubjectKey, t),
    "Time Travel",
  );
  assert.equal(
    libraryGraphClusterName(unlabelled, titleBySubjectKey, t),
    "Around Synthetic Title 2",
  );
  // An unknown key falls back to the medoid too.
  assert.equal(
    libraryGraphClusterName(
      { ...isekai, labelKey: "not-a-label" },
      titleBySubjectKey,
      t,
    ),
    "Around Synthetic Title 0",
  );
  // Without the medoid title the result's own label is kept.
  assert.equal(
    libraryGraphClusterName(unlabelled, new Map(), t),
    unlabelled.label,
  );
  assert.equal(
    libraryGraphGroupName(
      UNPLACED_CLUSTER_ID,
      joined.clusters,
      titleBySubjectKey,
      t,
    ),
    "Everything else",
  );
});

test("edge kinds read as localized phrases", () => {
  assert.equal(libraryGraphEdgeKindLabel("semantic", t), "Similar");
  assert.equal(libraryGraphEdgeKindLabel("relation:sequel", t), "Sequel");
  assert.equal(
    libraryGraphEdgeKindLabel("relation:co_watch", t),
    "Often watched together",
  );
  assert.equal(libraryGraphEdgeKindLabel("relation:remake", t), "Related");
  // Explored links: a plain recommendation, and a subtype under its type.
  assert.equal(
    libraryGraphEdgeKindLabel("relation:recommendation", t),
    "Recommended",
  );
  assert.equal(
    libraryGraphEdgeKindLabel("relation:sequel:direct", t),
    "Sequel",
  );
  assert.equal(
    libraryGraphEdgeKindLabel("affinity:theme:isekai", t),
    "Shared interest: Isekai",
  );
  assert.equal(
    libraryGraphEdgeKindLabel("affinity:theme:unknown-label", t),
    "Shared interest",
  );
  assert.equal(libraryGraphEdgeKindLabel("mystery", t), "Linked");
  assert.deepEqual(
    libraryGraphEdgeKindLabels(["semantic", "semantic", "relation:sequel"], t),
    ["Similar", "Sequel"],
  );
});

type Selection = Map<string, Selection | null>;

// The fields the catalog query selects for each title, nested selections
// included, read from the query text itself.
function itemSelection(query: string): Selection {
  const start = query.indexOf("items {");
  assert.ok(start >= 0, "query selects no items");
  const tokens = query
    .slice(start + "items {".length)
    .match(/[A-Za-z_][A-Za-z0-9_]*|[{}]/g);
  assert.ok(tokens);
  let position = 0;
  const read = (): Selection => {
    const selection: Selection = new Map();
    let last: string | null = null;
    while (position < tokens.length) {
      const token = tokens[position++];
      if (token === "}") {
        return selection;
      }
      if (token === "{") {
        assert.ok(last);
        selection.set(last, read());
        continue;
      }
      selection.set(token, null);
      last = token;
    }
    return selection;
  };
  return read();
}

// A row as the server sends it for a query: only the selected fields.
function project(value: unknown, selection: Selection): unknown {
  if (Array.isArray(value)) {
    return value.map((entry) => project(entry, selection));
  }
  if (value === null || typeof value !== "object") {
    return value;
  }
  const record = value as Record<string, unknown>;
  const row: Record<string, unknown> = {};
  for (const [field, nested] of selection) {
    if (field in record) {
      row[field] = nested ? project(record[field], nested) : record[field];
    }
  }
  return row;
}

// Everything the server knows about a synthetic title, more than any one
// query asks for.
function serverTitle(index: number) {
  return {
    id: `title-${index}`,
    name: `Synthetic Title ${index}`,
    facet: "MOVIE",
    libraryId: "library-1",
    monitored: true,
    tags: [],
    slug: `synthetic-title-${index}`,
    year: 2000 + (index % 20),
    posterUrl: null,
    imdbId: `tt${String(9_100_000 + index)}`,
    externalIds: [
      { source: "tmdb", kind: "movie", value: String(910_000 + index) },
    ],
    popularity: index,
    ratings: {
      rating: 5 + (index % 5),
      ratingSources: ["imdb"],
      externalRatings: [
        {
          source: "imdb",
          value: "7.0",
          score: 7,
          normalized: 0.7,
          votes: 10 * (index + 1),
          url: null,
        },
      ],
    },
    qualityProfileId: "profile-1",
    rootFolderId: "root-1",
    createdAt: "2026-01-01T00:00:00Z",
  };
}

function catalogRows(query: string, count: number): TitleRecord[] {
  const selection = itemSelection(query);
  return Array.from(
    { length: count },
    (_, index) => project(serverTitle(index), selection) as TitleRecord,
  );
}

test("the graph's catalog query returns rows the graph can join and size", () => {
  const rows = catalogRows(
    buildTitlesQuery(titleCatalogProjectionForGraph()),
    64,
  );
  assert.ok(rows.every((row) => row.backgroundUrl === undefined));
  assert.ok(rows.every((row) => row.tags === undefined));
  assert.ok(rows.every((row) => row.metadataFetchedAt === undefined));
  const { graph, titleBySubjectKey } = joinLibraryGraphToTitles(
    libraryGraphFixture({ facet: "MOVIE" }),
    rows,
  );
  assert.equal(graph.nodes.length, 64);
  assert.equal(titleBySubjectKey.get(fixtureSubjectKey(5))?.id, "title-5");

  // Ratings come back too, so nodes are not all one size.
  const sizes = new Set(nodeSizesByTitleId(rows, "ratingEvidence").values());
  assert.ok(sizes.size > 1);
  assert.ok(sizes.has(LIBRARY_GRAPH_MAX_NODE_SIZE));

  // The fixture source names its nodes after these rows' target keys.
  const keys = rows
    .map((row) => titleTargetKey(row))
    .filter((key): key is string => key !== null);
  assert.equal(keys.length, 64);
});

test("the table's catalog query carries no ids, so it cannot feed the graph", () => {
  const rows = catalogRows(
    buildTitlesQuery(
      titleCatalogProjectionForTable({
        facet: "movie",
        visibleColumns: {},
        sort: { key: "name", direction: "asc" },
      }),
    ),
    8,
  );
  assert.ok(rows.every((row) => titleTargetKey(row) === null));
  const { graph } = joinLibraryGraphToTitles(
    libraryGraphFixture({ facet: "MOVIE" }),
    rows,
  );
  assert.equal(graph.nodes.length, 0);
});

function tag(
  key: string,
  category: string,
  name: string,
  extra: { confidence?: number; isSpoiler?: boolean } = {},
) {
  return {
    key,
    category,
    name,
    confidence: extra.confidence ?? null,
    sources: [],
    sourceTagKeys: [],
    isAdult: false,
    isSpoiler: extra.isSpoiler ?? false,
  };
}

test("a neighbour's reason names its services and the genres and themes both titles share", () => {
  const anchor = [
    tag("canonical:genre:anime", "genre", "Anime"),
    tag("canonical:genre:drama", "genre", "Drama"),
    tag("canonical:theme:time-loop", "theme", "Time Loop"),
    tag("canonical:theme:twist", "theme", "Twist", { isSpoiler: true }),
    tag("canonical:content:violence", "content", "Violence"),
  ];
  const reason = neighborReason(
    anchor,
    {
      sources: ["mal", "trakt", "mal", " ", "zz-service"],
      canonicalTags: [
        tag("canonical:genre:anime", "genre", "Anime"),
        tag("canonical:genre:drama", "genre", "Drama", { confidence: 0.9 }),
        tag("canonical:theme:time-loop", "theme", "Time Loop"),
        tag("canonical:theme:twist", "theme", "Twist"),
        tag("canonical:content:violence", "content", "Violence"),
        tag("canonical:genre:comedy", "genre", "Comedy"),
      ],
    },
    ["relation:recommendation", "relation:sequel"],
  );
  assert.deepEqual(reason, {
    relations: ["relation:sequel"],
    sourceKind: "recommendation",
    sources: ["MyAnimeList", "Trakt", "zz-service"],
    // Themes first; the facet's own genre, spoilers, content tags and
    // unshared tags explain nothing.
    shared: ["Time Loop", "Drama"],
  });
  const lines = neighborReasonLines(reason, t);
  assert.equal(lines.length, 3);
  assert.equal(lines[1], "Recommended on MyAnimeList, Trakt, zz-service");
  assert.equal(lines[2], "In common: Time Loop, Drama");
});

test("a plain recommendation with nothing shared has no reason lines", () => {
  const reason = neighborReason(null, {}, ["relation:recommendation"]);
  assert.deepEqual(reason, {
    relations: [],
    sourceKind: null,
    sources: [],
    shared: [],
  });
  assert.deepEqual(neighborReasonLines(reason, t), []);
});

test("only a recommendation says its services recommended it", () => {
  const recommended = neighborReason(null, { sources: ["tmdb"] }, [
    "relation:recommendation",
  ]);
  assert.deepEqual(neighborReasonLines(recommended, t), [
    "Recommended on TMDB",
  ]);

  const similar = neighborReason(null, { sources: ["tmdb"] }, [
    "relation:similar",
  ]);
  assert.deepEqual(similar.relations, []);
  assert.equal(similar.sourceKind, "similar");
  assert.deepEqual(neighborReasonLines(similar, t), ["Similar on TMDB"]);

  // Both kinds: the services line keeps the recommendation wording and the
  // similar link still shows as a label.
  const both = neighborReason(null, { sources: ["tmdb", "mal"] }, [
    "relation:recommendation",
    "relation:similar",
  ]);
  assert.deepEqual(neighborReasonLines(both, t), [
    "Similar",
    "Recommended on TMDB, MyAnimeList",
  ]);
});

test("a similar link without services keeps its kind label", () => {
  const reason = neighborReason(null, {}, ["relation:similar"]);
  assert.equal(reason.sourceKind, null);
  assert.deepEqual(neighborReasonLines(reason, t), ["Similar"]);
});

test("a link of another kind keeps its label and names no recommender", () => {
  const reason = neighborReason(null, { sources: ["tmdb"] }, [
    "relation:sequel",
  ]);
  assert.equal(reason.sourceKind, null);
  assert.deepEqual(neighborReasonLines(reason, t), ["Sequel"]);
});

test("recommendations group by the genre or theme most of them share", () => {
  const sports = tag("canonical:genre:sports", "genre", "Sports");
  const team = tag("canonical:theme:team", "theme", "Team");
  const magic = tag("canonical:theme:magic", "theme", "Magic");
  const action = tag("canonical:genre:action", "genre", "Action");
  const sections = recommendationSections([
    { key: "r:1", tags: [sports, team, action] },
    { key: "r:2", tags: [magic, action] },
    { key: "r:3", tags: [sports, team, action] },
    { key: "r:4", tags: [magic, action] },
    { key: "r:5", tags: [sports, action] },
    { key: "r:6", tags: null },
  ]);
  // Action is on too many to tell them apart; Sports covers three, Magic two.
  assert.deepEqual(
    sections.map((section) => [section.name, section.keys]),
    [
      ["Sports", ["r:1", "r:3", "r:5"]],
      ["Magic", ["r:2", "r:4"]],
      [null, ["r:6"]],
    ],
  );
});

test("recommendations with nothing that splits them stay one section", () => {
  const drama = tag("canonical:genre:drama", "genre", "Drama");
  const sections = recommendationSections([
    { key: "r:1", tags: [drama] },
    { key: "r:2", tags: [drama] },
    { key: "r:3", tags: [] },
  ]);
  assert.deepEqual(sections, [
    { tagKey: null, name: null, keys: ["r:1", "r:2", "r:3"] },
  ]);
  assert.deepEqual(recommendationSections([]), []);
});

test("same-category neighbourhoods form one section before section limits", () => {
  const items = ["r:1", "r:2", "r:3", "r:4", "r:5", "r:6", "r:7"].map(
    (key) => ({ key, tags: [] }),
  );
  const groups = [
    { labelKey: "genre:adventure", label: "Adventure", keys: ["r:3", "r:1"] },
    { labelKey: "genre:comedy", label: "Comedy", keys: ["r:4", "r:2"] },
    {
      labelKey: "canonical:genre:adventure",
      label: "Adventure",
      keys: ["r:5", "r:1", "r:6", "outside"],
    },
  ];
  const sections = recommendationSections(items, 2, groups);
  assert.deepEqual(sections, [
    {
      tagKey: "genre:adventure",
      name: "Adventure",
      keys: ["r:1", "r:3", "r:5", "r:6"],
    },
    { tagKey: "genre:comedy", name: "Comedy", keys: ["r:2", "r:4"] },
    { tagKey: null, name: null, keys: ["r:7"] },
  ]);
  assert.equal(
    new Set(sections.flatMap((section) => section.keys)).size,
    items.length,
  );
  // A single shared category does not manufacture distinct neighbourhoods.
  assert.deepEqual(recommendationSections(items, 5, [groups[0], groups[2]]), [
    { tagKey: null, name: null, keys: items.map((item) => item.key) },
  ]);
});

test("server neighbourhoods decide the sections when there are any", () => {
  const sports = tag("canonical:genre:sports", "genre", "Sports");
  const items = ["r:1", "r:2", "r:3", "r:4", "r:5", "r:6"].map((key) => ({
    key,
    tags: [sports],
  }));
  const sections = recommendationSections(items, 5, [
    {
      labelKey: "theme:fixture-alpha",
      label: "Fixture Alpha",
      keys: ["r:4", "r:1", "r:9"],
    },
    {
      labelKey: "theme:fixture-bravo",
      label: "Fixture Bravo",
      keys: ["r:2", "r:5"],
    },
    // A neighbourhood known only by the title it centres on is not named.
    { labelKey: null, label: "Synthetic Delta", keys: ["r:3", "r:6"] },
    // One of the given recommendations is not enough to name a section.
    {
      labelKey: "theme:fixture-charlie",
      label: "Fixture Charlie",
      keys: ["r:3"],
    },
  ]);
  assert.deepEqual(sections, [
    // Input order within a section; keys outside the input are ignored.
    {
      tagKey: "theme:fixture-alpha",
      name: "Fixture Alpha",
      keys: ["r:1", "r:4"],
    },
    {
      tagKey: "theme:fixture-bravo",
      name: "Fixture Bravo",
      keys: ["r:2", "r:5"],
    },
    { tagKey: null, name: null, keys: ["r:3", "r:6"] },
  ]);
  assert.equal(recommendationSectionName(sections[2], t), null);

  // Past the limit, a neighbourhood's titles join the rest.
  assert.deepEqual(
    recommendationSections(items, 1, [
      {
        labelKey: "theme:fixture-alpha",
        label: "Fixture Alpha",
        keys: ["r:1", "r:2"],
      },
      {
        labelKey: "theme:fixture-bravo",
        label: "Fixture Bravo",
        keys: ["r:3", "r:4"],
      },
    ]).map((section) => [section.name, section.keys]),
    [[null, ["r:1", "r:2", "r:3", "r:4", "r:5", "r:6"]]],
  );
});

test("tag grouping applies when no server neighbourhood holds a recommendation", () => {
  const sports = tag("canonical:genre:sports", "genre", "Sports");
  const magic = tag("canonical:theme:magic", "theme", "Magic");
  const items = [
    { key: "r:1", tags: [sports] },
    { key: "r:2", tags: [magic] },
    { key: "r:3", tags: [sports] },
    { key: "r:4", tags: [magic] },
  ];
  const byTags = recommendationSections(items);
  assert.deepEqual(
    recommendationSections(items, 5, [
      { labelKey: null, label: "Synthetic Elsewhere", keys: ["r:9"] },
    ]),
    byTags,
  );
  assert.deepEqual(
    byTags.map((section) => [section.name, section.keys]),
    [
      ["Magic", ["r:2", "r:4"]],
      ["Sports", ["r:1", "r:3"]],
    ],
  );
});

test("only labelled clusters of three or more titles are named neighbourhoods", () => {
  const named = libraryGraphNamedClusterIds([
    {
      id: 0,
      labelKey: "genre:fixture-alpha",
      label: "A",
      medoid: "s:a",
      size: 12,
    },
    {
      id: 1,
      labelKey: "genre:fixture-bravo",
      label: "B",
      medoid: "s:b",
      size: 3,
    },
    {
      id: 2,
      labelKey: "genre:fixture-gamma",
      label: "C",
      medoid: "s:c",
      size: 2,
    },
    { id: 3, labelKey: null, label: "Synthetic One", medoid: "s:d", size: 1 },
    { id: 4, labelKey: null, label: "Synthetic Many", medoid: "s:e", size: 9 },
  ]);
  assert.deepEqual([...named].sort(), [0, 1]);
});

test("a refetched listing with the same titles is recognised as unchanged", () => {
  const title = (id: string, name: string) =>
    ({
      id,
      name,
      externalIds: [{ source: "tvdb", value: id }],
      tags: ["a"],
      posterUrl: null,
    }) as unknown as TitleRecord;
  const previous = [title("1", "One"), title("2", "Two")];
  assert.equal(
    sameTitleRecords(previous, [title("1", "One"), title("2", "Two")]),
    true,
  );
  assert.equal(
    sameTitleRecords(previous, [title("1", "One"), title("2", "Deux")]),
    false,
  );
  assert.equal(sameTitleRecords(previous, [title("1", "One")]), false);
  assert.equal(
    sameTitleRecords(previous, [title("2", "Two"), title("1", "One")]),
    false,
  );
  const withTag = title("2", "Two");
  (withTag as unknown as { tags: string[] }).tags = ["a", "b"];
  assert.equal(sameTitleRecords(previous, [title("1", "One"), withTag]), false);
  const withField = { ...title("2", "Two"), year: 2001 } as TitleRecord;
  assert.equal(
    sameTitleRecords(previous, [title("1", "One"), withField]),
    false,
  );
});

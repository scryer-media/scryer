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
  joinLibraryGraphToTitles,
  libraryGraphClusterName,
  libraryGraphEdgeKindLabel,
  libraryGraphEdgeKindLabels,
  libraryGraphGroupName,
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

test("above the threshold every cluster collapses until it is expanded", () => {
  const { graph } = joinLibraryGraphToTitles(
    libraryGraphFixture({ facet: "MOVIE" }),
    catalog(64),
  );
  assert.equal(displayedLibraryGraph(graph, new Set()).nodes.length, 64);

  const collapsed = displayedLibraryGraph(graph, new Set(), 60);
  assert.deepEqual(
    collapsed.nodes.map((node) => node.subjectKey).sort(),
    [
      clusterNodeKey(-1),
      clusterNodeKey(0),
      clusterNodeKey(1),
      clusterNodeKey(2),
    ].sort(),
  );
  // Co-watch bridges join the cluster nodes.
  assert.ok(collapsed.edges.length >= 3);

  const oneOpen = displayedLibraryGraph(graph, new Set([1]), 60);
  assert.equal(oneOpen.nodes.filter((node) => node.clusterId === 1).length, 21);
  assert.equal(
    oneOpen.nodes.filter((node) => node.collapsedClusterId !== undefined)
      .length,
    3,
  );
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
    "Unplaced titles",
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

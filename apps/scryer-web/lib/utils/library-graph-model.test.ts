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
import { clusterNodeKey } from "./library-graph-merge.ts";
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
  titleIndexByMatchKey,
  titleTargetKey,
} from "./library-graph-model.ts";
import { createFixtureLibraryGraphSource } from "./library-graph-source.ts";

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

test("target keys match titles by source and id, with or without the kind", () => {
  assert.deepEqual(targetKeyMatchKeys("TMDB:movie:0042").sort(), [
    "tmdb:0042",
    "tmdb:42",
    "tmdb:movie:0042",
  ]);
  const anime = fixtureTitle(5, {
    facet: "ANIME",
    externalIds: [{ source: "AniList", value: "77" }],
  });
  const index = titleIndexByMatchKey([anime]);
  assert.equal(titleForTargetKey("anilist:anime:77", index), anime);
  assert.equal(titleForTargetKey("anilist:series:77", index), anime);
  assert.equal(titleForTargetKey("anilist:movie:78", index), null);
  const imdbOnly = fixtureTitle(6, { externalIds: [], imdbId: "tt0000006" });
  assert.equal(
    titleForTargetKey("imdb:movie:tt0000006", titleIndexByMatchKey([imdbOnly])),
    imdbOnly,
  );
  assert.equal(titleTargetKey(fixtureTitle(3)), "tmdb:movie:910003");
  assert.equal(titleTargetKey(imdbOnly), "imdb:movie:tt0000006");
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

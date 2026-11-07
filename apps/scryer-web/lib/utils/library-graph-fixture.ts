// A synthetic, deterministic library graph. Tests use it, and so does the graph
// view while the server snapshot it will draw from is not available. Every
// name and key is invented.

import type { Facet } from "../types/titles.ts";
import {
  UNPLACED_CLUSTER_ID,
  type LibraryGraphCluster,
  type LibraryGraphEdge,
  type LibraryGraphNode,
  type LibraryGraphResult,
} from "../types/library-graph.ts";

export const LIBRARY_GRAPH_FIXTURE_NODE_COUNT = 64;
export const LIBRARY_GRAPH_FIXTURE_GENERATED_AT = "2026-01-01T00:00:00Z";
const FIXTURE_ID_BASE = 910_000;

/** The subject key of fixture node `index`. */
export function fixtureSubjectKey(index: number): string {
  return `fixture:subject:${index}`;
}

/** The synthetic target key fixture node `index` carries for a facet. */
export function fixtureTargetKey(facet: Facet, index: number): string {
  // The server keys TMDB anime under series.
  const kind = facet === "MOVIE" ? "movie" : "series";
  return `tmdb:${kind}:${FIXTURE_ID_BASE + index}`;
}

// A small seeded generator so weights are stable from run to run.
function seededRandom(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let value = state;
    value = Math.imul(value ^ (value >>> 15), value | 1);
    value ^= value + Math.imul(value ^ (value >>> 7), value | 61);
    return ((value ^ (value >>> 14)) >>> 0) / 4294967296;
  };
}

const CLUSTER_AFFINITIES = [
  "affinity:theme:isekai",
  "affinity:theme:time-travel",
  "affinity:genre:drama",
] as const;

export type LibraryGraphFixtureOptions = {
  facet: Facet;
  /**
   * Target keys to give the first fixture nodes, in order, so the fixture can
   * stand over a real catalog; the remaining nodes keep synthetic keys.
   */
  targetKeys?: readonly string[];
};

/**
 * Three clusters of 21 nodes and one unplaced node. Cluster 0 and 1 carry a
 * label key, cluster 2 does not and is named by its medoid. The edges cover
 * every kind the view distinguishes: semantic, sequel, prequel, co-watch and
 * theme and genre affinities.
 */
export function libraryGraphFixture(
  options: LibraryGraphFixtureOptions,
): LibraryGraphResult {
  const random = seededRandom(0x5eed);
  const weight = () => Math.round((0.35 + random() * 0.6) * 1000) / 1000;
  const nodes: LibraryGraphNode[] = [];
  for (let index = 0; index < LIBRARY_GRAPH_FIXTURE_NODE_COUNT; index += 1) {
    const unplaced = index === LIBRARY_GRAPH_FIXTURE_NODE_COUNT - 1;
    nodes.push({
      subjectKey: fixtureSubjectKey(index),
      targetKey:
        options.targetKeys?.[index] ?? fixtureTargetKey(options.facet, index),
      clusterId: unplaced ? UNPLACED_CLUSTER_ID : index % 3,
    });
  }

  const members = [0, 1, 2].map((clusterId) =>
    nodes
      .map((node, index) => ({ node, index }))
      .filter(({ node }) => node.clusterId === clusterId)
      .map(({ index }) => index),
  );

  const edges: LibraryGraphEdge[] = [];
  const link = (from: number, to: number, kinds: string[]) => {
    edges.push({
      from: fixtureSubjectKey(from),
      to: fixtureSubjectKey(to),
      weight: weight(),
      kinds,
    });
  };

  members.forEach((group, clusterId) => {
    // A ring of semantic neighbours inside each cluster, every third link also
    // sharing the cluster's affinity.
    group.forEach((index, position) => {
      const next = group[(position + 1) % group.length];
      link(
        index,
        next,
        position % 3 === 0
          ? ["semantic", CLUSTER_AFFINITIES[clusterId]]
          : ["semantic"],
      );
      // A chord across the ring keeps clusters from collapsing into a line.
      if (position % 4 === 0) {
        link(index, group[(position + 7) % group.length], ["semantic"]);
      }
    });
    // A franchise chain in each cluster.
    link(group[0], group[1], ["relation:sequel"]);
    link(group[2], group[1], ["relation:prequel"]);
  });

  // Co-watch bridges between clusters.
  link(members[0][3], members[1][3], ["relation:co_watch"]);
  link(members[1][5], members[2][5], ["relation:co_watch", "semantic"]);
  link(members[2][8], members[0][8], ["relation:co_watch"]);

  // The unplaced node hangs off one title by co-watch only.
  link(LIBRARY_GRAPH_FIXTURE_NODE_COUNT - 1, members[0][4], [
    "relation:co_watch",
  ]);

  const clusters: LibraryGraphCluster[] = [
    { id: 0, labelKey: "isekai", label: "Isekai" },
    { id: 1, labelKey: "theme:time-travel", label: "Time Travel" },
    { id: 2, labelKey: null, label: "Cluster 2" },
  ].map((cluster) => ({
    ...cluster,
    medoid: fixtureSubjectKey(members[cluster.id][0]),
    size: members[cluster.id].length,
  }));

  return {
    generatedAt: LIBRARY_GRAPH_FIXTURE_GENERATED_AT,
    nodes,
    edges,
    clusters,
  };
}

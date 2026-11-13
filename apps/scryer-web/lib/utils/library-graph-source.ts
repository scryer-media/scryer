// Where the graph view gets its library graph. The view depends only on
// LibraryGraphSource: the server source in the app, the fixture in tests.

import type { Client } from "@urql/core";

import type { Facet } from "../types/titles.ts";
import type {
  LibraryGraphCluster,
  LibraryGraphEdge,
  LibraryGraphNode,
  LibraryGraphResult,
} from "../types/library-graph.ts";
import { hasGraphQlErrorReason } from "../graphql/error-message.ts";
import { libraryGraphQuery } from "../graphql/queries.ts";
import { libraryGraphFixture } from "./library-graph-fixture.ts";

export type LibraryGraphRequest = {
  facet: Facet;
  /** The libraries in view; empty means every library the viewer can read. */
  libraryIds: readonly string[];
  /**
   * The graph already drawn for this request. The server is told its build
   * time, and when that is still the newest graph the load returns it as is.
   */
  held?: LibraryGraphResult | null;
};

export interface LibraryGraphSource {
  /** The stored graph, or null when none has been built yet. */
  load(request: LibraryGraphRequest): Promise<LibraryGraphResult | null>;
}

/**
 * A source that answers every request with the synthetic fixture. Target keys
 * passed in name the first fixture nodes, so the fixture can be drawn over a
 * catalog.
 */
export function createFixtureLibraryGraphSource(
  targetKeys?: readonly string[],
): { load(request: LibraryGraphRequest): Promise<LibraryGraphResult> } {
  return {
    load: (request) =>
      Promise.resolve(
        libraryGraphFixture({ facet: request.facet, targetKeys }),
      ),
  };
}

type LibraryGraphPayload = {
  generatedAt: string;
  viewKey: string;
  nodes: LibraryGraphNode[];
  edges: LibraryGraphEdge[];
  clusters: Array<
    Omit<LibraryGraphCluster, "labelKey"> & { labelKey?: string | null }
  >;
  subjectCount: number;
  truncated: boolean;
  /** True when this graph is the one the request said it holds. */
  unchanged?: boolean;
};

type LibraryGraphQueryData = { libraryGraph?: LibraryGraphPayload | null };

type LibraryGraphQueryVariables = {
  input: { facet: Facet; libraryIds?: string[]; knownViewKey?: string };
};

/** The server's payload as a library graph result. */
export function libraryGraphResultFromPayload(
  payload: LibraryGraphPayload,
): LibraryGraphResult {
  return {
    generatedAt: payload.generatedAt,
    viewKey: payload.viewKey,
    nodes: payload.nodes.map((node) => ({
      subjectKey: node.subjectKey,
      targetKey: node.targetKey,
      clusterId: node.clusterId,
    })),
    edges: payload.edges.map((edge) => ({
      from: edge.from,
      to: edge.to,
      weight: edge.weight,
      kinds: [...edge.kinds],
    })),
    clusters: payload.clusters.map((cluster) => ({
      id: cluster.id,
      labelKey: cluster.labelKey?.trim() ? cluster.labelKey : null,
      label: cluster.label,
      medoid: cluster.medoid,
      size: cluster.size,
    })),
    subjectCount: payload.subjectCount,
    truncated: payload.truncated,
  };
}

/** The reason the server gives when the graph was replaced mid-read. */
const LIBRARY_GRAPH_READ_SUPERSEDED = "LIBRARY_GRAPH_READ_SUPERSEDED";

/** How long to wait before reading a graph that was replaced mid-read again. */
export const LIBRARY_GRAPH_SUPERSEDED_RETRY_MS = 750;

const delay = (ms: number) =>
  new Promise<void>((resolve) => setTimeout(resolve, ms));

/**
 * The graph the server built for the facet and libraries. A request error
 * rejects; a graph that has not been built yet resolves to null. A graph
 * replaced while it was being read is read once more after a short wait.
 * When the server says the held graph is still the view it would give, the
 * held graph is returned.
 */
export function createServerLibraryGraphSource(
  client: Pick<Client, "query">,
  wait: (ms: number) => Promise<void> = delay,
): LibraryGraphSource {
  return {
    async load(request) {
      const libraryIds = [...request.libraryIds];
      const held = request.held?.viewKey ? request.held : null;
      const read = () =>
        client
          .query<LibraryGraphQueryData, LibraryGraphQueryVariables>(
            libraryGraphQuery,
            {
              input: {
                facet: request.facet,
                ...(libraryIds.length > 0 ? { libraryIds } : {}),
                ...(held ? { knownViewKey: held.viewKey } : {}),
              },
            },
            { requestPolicy: "network-only" },
          )
          .toPromise();
      let { data, error } = await read();
      if (
        error &&
        hasGraphQlErrorReason(
          error,
          LIBRARY_GRAPH_READ_SUPERSEDED,
          "TEMPORARY_UNAVAILABLE",
        )
      ) {
        await wait(LIBRARY_GRAPH_SUPERSEDED_RETRY_MS);
        ({ data, error } = await read());
      }
      if (error) {
        throw error;
      }
      const payload = data?.libraryGraph;
      if (payload?.unchanged && held) {
        return held;
      }
      return payload ? libraryGraphResultFromPayload(payload) : null;
    },
  };
}

/** The source the catalog's graph view reads from. */
export function libraryGraphSourceForCatalog(
  client: Pick<Client, "query">,
): LibraryGraphSource {
  return createServerLibraryGraphSource(client);
}

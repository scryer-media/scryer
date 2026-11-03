import assert from "node:assert/strict";
import test from "node:test";

import type { Client } from "@urql/core";

import { libraryGraphQuery } from "../graphql/queries.ts";
import {
  createServerLibraryGraphSource,
  libraryGraphSourceForCatalog,
} from "./library-graph-source.ts";

type Answer = { data?: unknown; error?: unknown };

function stubClient(answer: Answer) {
  const calls: Array<{ query: unknown; variables: unknown; context: unknown }> =
    [];
  const client = {
    query(query: unknown, variables: unknown, context: unknown) {
      calls.push({ query, variables, context });
      return { toPromise: () => Promise.resolve(answer) };
    },
  } as unknown as Pick<Client, "query">;
  return { client, calls };
}

const payload = {
  generatedAt: "2026-01-01T00:00:00Z",
  nodes: [
    { subjectKey: "s:1", targetKey: "tmdb:movie:910001", clusterId: 0 },
    { subjectKey: "s:2", targetKey: "tmdb:movie:910002", clusterId: -1 },
  ],
  edges: [{ from: "s:1", to: "s:2", weight: 0.4, kinds: ["semantic"] }],
  clusters: [
    { id: 0, labelKey: "", label: "Synthetic", medoid: "s:1", size: 1 },
    {
      id: 1,
      labelKey: "theme:isekai",
      label: "Isekai",
      medoid: "s:1",
      size: 0,
    },
  ],
  subjectCount: 3,
  truncated: true,
};

test("the server source maps the stored graph and asks for every readable library when none is chosen", async () => {
  const { client, calls } = stubClient({ data: { libraryGraph: payload } });
  const loaded = await createServerLibraryGraphSource(client).load({
    facet: "MOVIE",
    libraryIds: [],
  });
  assert.equal(calls.length, 1);
  assert.equal(calls[0].query, libraryGraphQuery);
  assert.deepEqual(calls[0].variables, { input: { facet: "MOVIE" } });
  assert.deepEqual(calls[0].context, { requestPolicy: "network-only" });
  assert.ok(loaded);
  assert.equal(loaded.subjectCount, 3);
  assert.equal(loaded.truncated, true);
  assert.deepEqual(loaded.nodes, payload.nodes);
  assert.deepEqual(loaded.edges, payload.edges);
  // A blank label key means the cluster is named by its medoid.
  assert.equal(loaded.clusters[0].labelKey, null);
  assert.equal(loaded.clusters[1].labelKey, "theme:isekai");
});

test("the server source passes the chosen libraries", async () => {
  const { client, calls } = stubClient({ data: { libraryGraph: payload } });
  await libraryGraphSourceForCatalog(client).load({
    facet: "ANIME",
    libraryIds: ["lib-1", "lib-2"],
  });
  assert.deepEqual(calls[0].variables, {
    input: { facet: "ANIME", libraryIds: ["lib-1", "lib-2"] },
  });
});

test("a graph not built yet loads as null", async () => {
  const { client } = stubClient({ data: { libraryGraph: null } });
  assert.equal(
    await createServerLibraryGraphSource(client).load({
      facet: "SERIES",
      libraryIds: [],
    }),
    null,
  );
});

test("a request error rejects the load", async () => {
  const failure = new Error("request failed");
  const { client } = stubClient({ error: failure });
  await assert.rejects(
    createServerLibraryGraphSource(client).load({
      facet: "SERIES",
      libraryIds: [],
    }),
    (caught) => caught === failure,
  );
});

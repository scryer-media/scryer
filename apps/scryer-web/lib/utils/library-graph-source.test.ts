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
  viewKey: "v1:synthetic-view",
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

test("a held graph sends its view key and is kept when the server says it is unchanged", async () => {
  const { client: fullClient } = stubClient({
    data: { libraryGraph: payload },
  });
  const held = await createServerLibraryGraphSource(fullClient).load({
    facet: "MOVIE",
    libraryIds: [],
  });
  assert.ok(held);
  const { client, calls } = stubClient({
    data: {
      libraryGraph: {
        generatedAt: payload.generatedAt,
        viewKey: payload.viewKey,
        nodes: [],
        edges: [],
        clusters: [],
        subjectCount: 0,
        truncated: false,
        unchanged: true,
      },
    },
  });
  const loaded = await createServerLibraryGraphSource(client).load({
    facet: "MOVIE",
    libraryIds: ["lib-1"],
    held,
  });
  assert.deepEqual(calls[0].variables, {
    input: {
      facet: "MOVIE",
      libraryIds: ["lib-1"],
      knownViewKey: payload.viewKey,
    },
  });
  assert.equal(loaded, held);
});

test("a held graph is replaced by a newer one and dropped when none is built", async () => {
  const held = {
    generatedAt: "2025-12-31T00:00:00Z",
    viewKey: "v1:older-view",
    nodes: [],
    edges: [],
    clusters: [],
    subjectCount: 0,
    truncated: false,
  };
  const { client } = stubClient({ data: { libraryGraph: payload } });
  const loaded = await createServerLibraryGraphSource(client).load({
    facet: "MOVIE",
    libraryIds: [],
    held,
  });
  assert.ok(loaded);
  assert.equal(loaded.generatedAt, payload.generatedAt);
  assert.equal(loaded.viewKey, payload.viewKey);
  assert.equal(loaded.subjectCount, 3);
  const { client: emptyClient } = stubClient({ data: { libraryGraph: null } });
  assert.equal(
    await createServerLibraryGraphSource(emptyClient).load({
      facet: "MOVIE",
      libraryIds: [],
      held,
    }),
    null,
  );
});

test("a held graph without a view key is not sent", async () => {
  const { client, calls } = stubClient({ data: { libraryGraph: payload } });
  await createServerLibraryGraphSource(client).load({
    facet: "MOVIE",
    libraryIds: [],
    held: {
      generatedAt: "2026-01-01T00:00:00Z",
      nodes: [],
      edges: [],
      clusters: [],
      subjectCount: 0,
      truncated: false,
    },
  });
  assert.deepEqual(calls[0].variables, { input: { facet: "MOVIE" } });
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

function sequenceClient(answers: Answer[]) {
  let calls = 0;
  const client = {
    query() {
      const answer = answers[Math.min(calls, answers.length - 1)];
      calls += 1;
      return { toPromise: () => Promise.resolve(answer) };
    },
  } as unknown as Pick<Client, "query">;
  return { client, calls: () => calls };
}

const superseded = {
  graphQLErrors: [
    {
      message: "replaced",
      extensions: {
        code: "TEMPORARY_UNAVAILABLE",
        reason: "LIBRARY_GRAPH_READ_SUPERSEDED",
      },
    },
  ],
};

test("a graph replaced mid-read is read once more after a wait", async () => {
  const { client, calls } = sequenceClient([
    { error: superseded },
    { data: { libraryGraph: payload } },
  ]);
  const waits: number[] = [];
  const loaded = await createServerLibraryGraphSource(client, async (ms) => {
    waits.push(ms);
  }).load({ facet: "MOVIE", libraryIds: [] });
  assert.equal(calls(), 2);
  assert.equal(waits.length, 1);
  assert.ok(loaded);
});

test("a graph replaced on the retry too rejects, and other errors never retry", async () => {
  const twice = sequenceClient([{ error: superseded }]);
  await assert.rejects(
    createServerLibraryGraphSource(twice.client, async () => {}).load({
      facet: "MOVIE",
      libraryIds: [],
    }),
  );
  assert.equal(twice.calls(), 2);

  const other = sequenceClient([
    {
      error: {
        graphQLErrors: [
          {
            message: "LIBRARY_GRAPH_READ_SUPERSEDED",
            extensions: { code: "TEMPORARY_UNAVAILABLE", reason: "OTHER" },
          },
        ],
      },
    },
  ]);
  await assert.rejects(
    createServerLibraryGraphSource(other.client, async () => {}).load({
      facet: "MOVIE",
      libraryIds: [],
    }),
  );
  assert.equal(other.calls(), 1);
});

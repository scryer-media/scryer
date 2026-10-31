import assert from "node:assert/strict";
import test from "node:test";
import type { IndexerClientRecord } from "../graphql/indexer-clients.ts";
import { sortIndexerClients } from "./indexer-client-sort.ts";

const client = (
  name: string,
  overrides: Partial<IndexerClientRecord> = {},
): IndexerClientRecord => ({
  id: name,
  name,
  enabled: true,
  protocols: ["NEWZNAB"],
  grants: [],
  revision: 1,
  keyActive: true,
  lastUsedAt: null,
  lastOutcome: null,
  createdAt: "2026-01-01T00:00:00Z",
  ...overrides,
});

const names = (clients: IndexerClientRecord[]) =>
  clients.map((value) => value.name);

test("clients sort by each column with name as the tie breaker", () => {
  const clients = [
    client("tool-b", {
      lastUsedAt: "2026-02-01T00:00:00Z",
      grants: [{ indexerId: "x", categories: null }],
    }),
    client("Tool-a", { keyActive: false, enabled: false }),
    client("tool-c", {
      lastUsedAt: "2026-03-01T00:00:00Z",
      protocols: ["TORZNAB", "NEWZNAB"],
    }),
  ];
  assert.deepEqual(
    names(sortIndexerClients(clients, { key: "name", descending: false })),
    ["Tool-a", "tool-b", "tool-c"],
  );
  assert.deepEqual(
    names(sortIndexerClients(clients, { key: "name", descending: true })),
    ["tool-c", "tool-b", "Tool-a"],
  );
  assert.deepEqual(
    names(sortIndexerClients(clients, { key: "lastUsed", descending: true })),
    ["tool-c", "tool-b", "Tool-a"],
  );
  assert.deepEqual(
    names(sortIndexerClients(clients, { key: "grants", descending: true })),
    ["tool-b", "tool-c", "Tool-a"],
  );
  assert.deepEqual(
    names(sortIndexerClients(clients, { key: "key", descending: false })),
    ["Tool-a", "tool-b", "tool-c"],
  );
  assert.deepEqual(
    names(sortIndexerClients(clients, { key: "protocols", descending: true })),
    ["tool-c", "tool-b", "Tool-a"],
  );
});

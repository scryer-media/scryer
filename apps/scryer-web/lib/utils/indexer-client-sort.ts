import type { IndexerClientRecord } from "../graphql/indexer-clients.ts";

export type IndexerClientSortKey =
  "name" | "protocols" | "grants" | "enabled" | "key" | "lastUsed";
export type IndexerClientSort = {
  key: IndexerClientSortKey;
  descending: boolean;
};

/** Name breaks ties, so equal rows keep a stable, readable order. A client never used sorts as oldest. */
export function sortIndexerClients(
  clients: readonly IndexerClientRecord[],
  sort: IndexerClientSort,
): IndexerClientRecord[] {
  const byName = (left: IndexerClientRecord, right: IndexerClientRecord) =>
    left.name.localeCompare(right.name, undefined, {
      sensitivity: "base",
      numeric: true,
    });
  const value = (client: IndexerClientRecord): number | string => {
    switch (sort.key) {
      case "name":
        return 0;
      case "protocols":
        return [...client.protocols].sort().join(",");
      case "grants":
        return client.grants.length;
      case "enabled":
        return client.enabled ? 1 : 0;
      case "key":
        return client.keyActive ? 1 : 0;
      case "lastUsed":
        return client.lastUsedAt
          ? Date.parse(client.lastUsedAt)
          : Number.NEGATIVE_INFINITY;
    }
  };
  const direction = sort.descending ? -1 : 1;
  return [...clients].sort((left, right) => {
    const a = value(left);
    const b = value(right);
    const primary =
      typeof a === "string" && typeof b === "string"
        ? a.localeCompare(b)
        : Number(a) - Number(b);
    return (primary || byName(left, right)) * direction;
  });
}

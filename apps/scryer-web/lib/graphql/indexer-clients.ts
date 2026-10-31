/** Compatibility protocol an indexer client may use. */
export type IndexerClientProtocol = "NEWZNAB" | "TORZNAB";

/** How Scryer finished preparing the most recent request from an indexer client. */
export type IndexerClientRequestOutcome =
  | "CAPS_READY"
  | "SEARCH_READY"
  | "SEARCH_PARTIAL"
  | "ARTIFACT_READY"
  | "REFUSED"
  | "UNAVAILABLE"
  | "FAILED";

export type IndexerClientGrant = {
  indexerId: string;
  categories: string[] | null;
};
export type IndexerClientConfiguration = {
  name: string;
  enabled: boolean;
  protocols: IndexerClientProtocol[];
  grants: IndexerClientGrant[];
};
export type IndexerClientRecord = IndexerClientConfiguration & {
  id: string;
  revision: number;
  keyActive: boolean;
  lastUsedAt: string | null;
  lastOutcome: IndexerClientRequestOutcome | null;
  createdAt: string;
};

const clientFields = `id name enabled protocols grants { indexerId categories }
  revision keyActive lastUsedAt lastOutcome createdAt`;
export const indexerClientsQuery = `query IndexerClients { indexerClients { ${clientFields} } }`;
export const createIndexerClientMutation = `mutation CreateIndexerClient($input: IndexerClientConfigurationInput!) {
  createIndexerClient(input: $input) { apiKey client { ${clientFields} } }
}`;
export const updateIndexerClientMutation = `mutation UpdateIndexerClient($id: ID!, $expectedRevision: Long!, $input: IndexerClientConfigurationInput!) {
  updateIndexerClient(id: $id, expectedRevision: $expectedRevision, input: $input) { ${clientFields} }
}`;
export const rotateIndexerClientKeyMutation = `mutation RotateIndexerClientKey($id: ID!, $expectedRevision: Long!) {
  rotateIndexerClientKey(id: $id, expectedRevision: $expectedRevision) { apiKey client { ${clientFields} } }
}`;
export const revokeIndexerClientKeyMutation = `mutation RevokeIndexerClientKey($id: ID!, $expectedRevision: Long!) {
  revokeIndexerClientKey(id: $id, expectedRevision: $expectedRevision) { ${clientFields} }
}`;

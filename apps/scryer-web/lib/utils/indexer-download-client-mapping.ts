import type { IndexerRecord } from "../types/indexers.ts";
import type {
  IndexerDownloadClientMappingCatalog,
  IndexerDownloadClientMappingClient,
  IndexerDownloadClientMappingCatalogResource,
} from "../types/indexer-download-client-mappings.ts";

export const AUTOMATIC_DOWNLOAD_CLIENT_ID = "__automatic__";

export function beginIndexerDownloadClientCatalogRequest(
  resource: IndexerDownloadClientMappingCatalogResource,
): IndexerDownloadClientMappingCatalogResource {
  return {
    ...resource,
    status: resource.catalog ? "refreshing" : "loading",
    error: null,
  };
}

export function completeIndexerDownloadClientCatalogRequest(
  catalog: IndexerDownloadClientMappingCatalog,
): IndexerDownloadClientMappingCatalogResource {
  return { catalog, status: "ready", error: null };
}

export function failIndexerDownloadClientCatalogRequest(
  resource: IndexerDownloadClientMappingCatalogResource,
  error: string,
): IndexerDownloadClientMappingCatalogResource {
  return { ...resource, status: "error", error };
}

export function isLatestIndexerDownloadClientCatalogRequest(
  requestSequence: number,
  latestRequestSequence: number,
): boolean {
  return requestSequence === latestRequestSequence;
}

const UNAVAILABLE_CLIENT_STATUSES = new Set([
  "ERROR",
  "FAILED",
  "OFFLINE",
  "UNAVAILABLE",
  "UNHEALTHY",
]);

export type IndexerDownloadClientMappingInvalidReason =
  "missing" | "incompatible" | "unavailable";

export type IndexerDownloadClientMappingOption = {
  id: string;
  name: string;
  clientType: string | null;
  enabled: boolean;
  health: string | null;
  status: string | null;
  isCurrent: boolean;
  isCompatible: boolean;
  isMissing: boolean;
  isUnavailable: boolean;
};

export type IndexerDownloadClientMappingViewModel = {
  indexerId: string;
  selectedId: string;
  isNotApplicable: boolean;
  currentDownloadClientId: string | null;
  currentClient: IndexerDownloadClientMappingClient | null;
  isInvalid: boolean;
  invalidReason: IndexerDownloadClientMappingInvalidReason | null;
  isDisabled: boolean;
  compatibleClients: IndexerDownloadClientMappingOption[];
  options: IndexerDownloadClientMappingOption[];
};

function normalizedStatus(value: string | null | undefined): string {
  return (
    value
      ?.trim()
      .replace(/[-\s]+/g, "_")
      .toUpperCase() ?? ""
  );
}

export function isDownloadClientUnavailable(
  client: Pick<
    IndexerDownloadClientMappingClient,
    "isEnabled" | "healthStatus"
  >,
): boolean {
  if (!client.isEnabled) {
    return false;
  }

  return [client.healthStatus].some((value) =>
    UNAVAILABLE_CLIENT_STATUSES.has(normalizedStatus(value)),
  );
}

export function isManagementOnlyIndexer(
  indexer: Pick<IndexerRecord, "isManaged" | "supportsManagedChildrenSync">,
): boolean {
  return indexer.supportsManagedChildrenSync && !indexer.isManaged;
}

function toOption(
  client: IndexerDownloadClientMappingClient | null,
  currentId: string | null,
  compatibleIds: ReadonlySet<string>,
): IndexerDownloadClientMappingOption {
  const id = client?.id ?? currentId ?? "";
  const isCompatible = compatibleIds.has(id);
  return {
    id,
    name: client?.name ?? id,
    clientType: client?.clientType ?? null,
    enabled: client?.isEnabled ?? true,
    health: client?.healthStatus ?? null,
    status: client?.healthStatus ?? null,
    isCurrent: id === currentId,
    isCompatible,
    isMissing: client === null,
    isUnavailable: client ? isDownloadClientUnavailable(client) : false,
  };
}

function buildIndexerDownloadClientMappingViewModel(
  indexerId: string,
  currentDownloadClientId: string | null,
  compatibleClientIds: readonly string[],
  isNotApplicable: boolean,
  catalog: IndexerDownloadClientMappingCatalog,
): IndexerDownloadClientMappingViewModel {
  const clientsById = new Map(
    catalog.clients.map((client) => [client.id, client]),
  );
  const compatibleIds = new Set(compatibleClientIds);
  const compatibleClients = catalog.clients
    .filter((client) => compatibleIds.has(client.id))
    .map((client) => toOption(client, currentDownloadClientId, compatibleIds));
  const currentClient = currentDownloadClientId
    ? (clientsById.get(currentDownloadClientId) ?? null)
    : null;

  let invalidReason: IndexerDownloadClientMappingInvalidReason | null = null;
  if (currentDownloadClientId && currentClient === null) {
    invalidReason = "missing";
  } else if (
    currentDownloadClientId &&
    !compatibleIds.has(currentDownloadClientId)
  ) {
    invalidReason = "incompatible";
  } else if (currentClient && isDownloadClientUnavailable(currentClient)) {
    invalidReason = "unavailable";
  }

  const currentOption = currentDownloadClientId
    ? toOption(currentClient, currentDownloadClientId, compatibleIds)
    : null;
  const options =
    currentOption && invalidReason !== null
      ? [
          currentOption,
          ...compatibleClients.filter(
            (option) => option.id !== currentOption.id,
          ),
        ]
      : compatibleClients;

  return {
    indexerId,
    selectedId: currentDownloadClientId ?? AUTOMATIC_DOWNLOAD_CLIENT_ID,
    isNotApplicable,
    currentDownloadClientId,
    currentClient,
    isInvalid: invalidReason !== null,
    invalidReason,
    isDisabled: currentClient?.isEnabled === false,
    compatibleClients,
    options,
  };
}

export function getIndexerDownloadClientMappingViewModel(
  indexer: Pick<
    IndexerRecord,
    "id" | "isManaged" | "supportsManagedChildrenSync"
  > & {
    downloadClientId?: string | null;
  },
  catalog: IndexerDownloadClientMappingCatalog,
): IndexerDownloadClientMappingViewModel {
  const mapping = catalog.indexers.find((entry) => entry.id === indexer.id);
  return buildIndexerDownloadClientMappingViewModel(
    indexer.id,
    mapping ? mapping.downloadClientId : (indexer.downloadClientId ?? null),
    mapping?.compatibleClientIds ?? [],
    isManagementOnlyIndexer(indexer) || mapping?.supportsMapping === false,
    catalog,
  );
}

export function getIndexerDownloadClientDraftMappingViewModel(
  providerType: string,
  downloadClientId: string | null,
  catalog: IndexerDownloadClientMappingCatalog,
): IndexerDownloadClientMappingViewModel {
  const normalizedProviderType = providerType.trim().toLowerCase();
  const compatibility = catalog.providerCompatibility.find(
    (entry) =>
      entry.providerType.trim().toLowerCase() === normalizedProviderType,
  );
  return buildIndexerDownloadClientMappingViewModel(
    "draft",
    downloadClientId,
    compatibility?.compatibleClientIds ?? [],
    compatibility?.supportsMapping === false,
    catalog,
  );
}

export function normalizeIndexerDownloadClientMappingCatalog(
  rawCatalog: unknown,
): IndexerDownloadClientMappingCatalog {
  if (!rawCatalog || typeof rawCatalog !== "object") {
    return { clients: [], indexers: [], providerCompatibility: [] };
  }

  const catalog = rawCatalog as {
    clients?: unknown;
    indexers?: unknown;
    providerCompatibility?: unknown;
  };
  const clients = Array.isArray(catalog.clients)
    ? catalog.clients
        .filter((client): client is Record<string, unknown> => {
          if (!client || typeof client !== "object") return false;
          const value = client as Record<string, unknown>;
          return typeof value.id === "string" && typeof value.name === "string";
        })
        .map((client) => ({
          id: client.id as string,
          name: client.name as string,
          clientType:
            typeof client.clientType === "string" ? client.clientType : "",
          isEnabled: client.isEnabled !== false,
          healthStatus:
            typeof client.healthStatus === "string" ? client.healthStatus : "",
        }))
    : [];
  const indexers = Array.isArray(catalog.indexers)
    ? catalog.indexers
        .filter((indexer): indexer is Record<string, unknown> => {
          if (!indexer || typeof indexer !== "object") return false;
          const value = indexer as Record<string, unknown>;
          return typeof value.id === "string";
        })
        .map((indexer) => ({
          id: indexer.id as string,
          name: typeof indexer.name === "string" ? indexer.name : "",
          downloadClientId:
            typeof indexer.downloadClientId === "string"
              ? indexer.downloadClientId
              : null,
          protocolFamilies: Array.isArray(indexer.protocolFamilies)
            ? indexer.protocolFamilies.filter(
                (value): value is string => typeof value === "string",
              )
            : [],
          supportsMapping: indexer.supportsMapping !== false,
          compatibleClientIds: Array.isArray(indexer.compatibleClientIds)
            ? indexer.compatibleClientIds.filter(
                (value): value is string => typeof value === "string",
              )
            : [],
        }))
    : [];

  const providerCompatibility = Array.isArray(catalog.providerCompatibility)
    ? catalog.providerCompatibility
        .filter((provider): provider is Record<string, unknown> => {
          if (!provider || typeof provider !== "object") return false;
          return typeof provider.providerType === "string";
        })
        .map((provider) => ({
          providerType: provider.providerType as string,
          protocolFamilies: Array.isArray(provider.protocolFamilies)
            ? provider.protocolFamilies.filter(
                (value): value is string => typeof value === "string",
              )
            : [],
          supportsMapping: provider.supportsMapping !== false,
          compatibleClientIds: Array.isArray(provider.compatibleClientIds)
            ? provider.compatibleClientIds.filter(
                (value): value is string => typeof value === "string",
              )
            : [],
        }))
    : [];

  return { clients, indexers, providerCompatibility };
}

export function updateIndexerDownloadClientMapping(
  catalog: IndexerDownloadClientMappingCatalog,
  indexerId: string,
  downloadClientId: string | null,
): IndexerDownloadClientMappingCatalog {
  let found = false;
  const indexers = catalog.indexers.map((entry) => {
    if (entry.id !== indexerId) return entry;
    found = true;
    return { ...entry, downloadClientId };
  });

  if (!found) {
    indexers.push({
      id: indexerId,
      name: "",
      protocolFamilies: [],
      supportsMapping: true,
      compatibleClientIds: [],
      downloadClientId,
    });
  }

  return { ...catalog, indexers };
}

export function updatePendingIndexerMappingIds(
  pendingIds: ReadonlySet<string>,
  indexerId: string,
  isPending: boolean,
): Set<string> {
  const next = new Set(pendingIds);
  if (isPending) next.add(indexerId);
  else next.delete(indexerId);
  return next;
}

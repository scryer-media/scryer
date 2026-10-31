import assert from "node:assert/strict";
import test from "node:test";

import { indexerDownloadClientMappingCatalogQuery } from "../graphql/queries.ts";
import en from "../i18n/locales/en.ts";
import type {
  IndexerDownloadClientMappingCatalog,
  IndexerDownloadClientMappingClient,
} from "../types/index.ts";
import {
  AUTOMATIC_DOWNLOAD_CLIENT_ID,
  beginIndexerDownloadClientCatalogRequest,
  completeIndexerDownloadClientCatalogRequest,
  failIndexerDownloadClientCatalogRequest,
  getIndexerDownloadClientDraftMappingViewModel,
  getIndexerDownloadClientMappingViewModel,
  isLatestIndexerDownloadClientCatalogRequest,
  isManagementOnlyIndexer,
  updateIndexerDownloadClientMapping,
  updatePendingIndexerMappingIds,
} from "./indexer-download-client-mapping.ts";

const directIndexer = {
  id: "indexer-direct",
  isManaged: false,
  supportsManagedChildrenSync: false,
};

const clients: IndexerDownloadClientMappingClient[] = [
  {
    id: "usenet-a",
    name: "Usenet A",
    clientType: "sabnzbd",
    isEnabled: true,
    healthStatus: "healthy",
  },
  {
    id: "usenet-disabled",
    name: "Disabled Usenet",
    clientType: "nzbget",
    isEnabled: false,
    healthStatus: "disabled",
  },
  {
    id: "torrent-a",
    name: "Torrent A",
    clientType: "qbittorrent",
    isEnabled: true,
    healthStatus: "healthy",
  },
];

function catalog(
  mapping: Partial<
    IndexerDownloadClientMappingCatalog["indexers"][number]
  > = {},
): IndexerDownloadClientMappingCatalog {
  return {
    clients,
    providerCompatibility: [
      {
        providerType: "newznab",
        protocolFamilies: ["usenet"],
        supportsMapping: true,
        compatibleClientIds: ["usenet-a", "usenet-disabled"],
      },
    ],
    indexers: [
      {
        id: directIndexer.id,
        name: "Direct Indexer",
        protocolFamilies: ["usenet"],
        supportsMapping: true,
        compatibleClientIds: ["usenet-a", "usenet-disabled"],
        downloadClientId: null,
        ...mapping,
      },
    ],
  };
}

test("mapping choices include only compatible clients and Automatic first", () => {
  const model = getIndexerDownloadClientMappingViewModel(
    directIndexer,
    catalog(),
  );

  assert.equal(model.selectedId, AUTOMATIC_DOWNLOAD_CLIENT_ID);
  assert.deepEqual(
    model.compatibleClients.map((option) => option.id),
    ["usenet-a", "usenet-disabled"],
  );
  assert.equal(
    model.options.some((option) => option.id === "torrent-a"),
    false,
  );
});

test("create and edit drafts use provider compatibility before an indexer row exists", () => {
  const automatic = getIndexerDownloadClientDraftMappingViewModel(
    "newznab",
    null,
    catalog(),
  );
  assert.equal(automatic.selectedId, AUTOMATIC_DOWNLOAD_CLIENT_ID);
  assert.deepEqual(
    automatic.options.map((option) => option.id),
    ["usenet-a", "usenet-disabled"],
  );

  const incompatible = getIndexerDownloadClientDraftMappingViewModel(
    "newznab",
    "torrent-a",
    catalog(),
  );
  assert.equal(incompatible.invalidReason, "incompatible");
  assert.equal(incompatible.options[0]?.id, "torrent-a");

  const unsupportedCatalog = catalog();
  unsupportedCatalog.providerCompatibility.push({
    providerType: "prowlarr",
    protocolFamilies: [],
    supportsMapping: false,
    compatibleClientIds: [],
  });
  assert.equal(
    getIndexerDownloadClientDraftMappingViewModel(
      "prowlarr",
      null,
      unsupportedCatalog,
    ).isNotApplicable,
    true,
  );
});

test("a fresh first-visit catalog exposes newly created Weaver in table and form controls", () => {
  const firstVisitCatalog = catalog();
  firstVisitCatalog.clients.push({
    id: "weaver",
    name: "Weaver",
    clientType: "weaver",
    isEnabled: true,
    healthStatus: "healthy",
  });
  firstVisitCatalog.providerCompatibility[0]!.compatibleClientIds.push(
    "weaver",
  );
  firstVisitCatalog.indexers[0]!.compatibleClientIds.push("weaver");

  const tableModel = getIndexerDownloadClientMappingViewModel(
    directIndexer,
    firstVisitCatalog,
  );
  const formModel = getIndexerDownloadClientDraftMappingViewModel(
    "newznab",
    null,
    firstVisitCatalog,
  );
  assert.equal(
    tableModel.options.some((option) => option.id === "weaver"),
    true,
  );
  assert.equal(
    formModel.options.some((option) => option.id === "weaver"),
    true,
  );
});

test("compatible disabled clients remain selectable and are marked disabled", () => {
  const model = getIndexerDownloadClientMappingViewModel(
    directIndexer,
    catalog({ downloadClientId: "usenet-disabled" }),
  );

  assert.equal(model.isInvalid, false);
  assert.equal(model.isDisabled, true);
  assert.equal(
    model.options.find((option) => option.id === "usenet-disabled")?.enabled,
    false,
  );
});

test("missing, incompatible, and unavailable current values stay visible as invalid", () => {
  const missing = getIndexerDownloadClientMappingViewModel(
    directIndexer,
    catalog({ downloadClientId: "removed-client" }),
  );
  assert.equal(missing.invalidReason, "missing");
  assert.equal(missing.options[0]?.id, "removed-client");

  const incompatible = getIndexerDownloadClientMappingViewModel(
    directIndexer,
    catalog({ downloadClientId: "torrent-a" }),
  );
  assert.equal(incompatible.invalidReason, "incompatible");
  assert.equal(incompatible.options[0]?.id, "torrent-a");

  const unavailableCatalog = catalog({ downloadClientId: "usenet-a" });
  unavailableCatalog.clients = clients.map((client) =>
    client.id === "usenet-a" ? { ...client, healthStatus: "offline" } : client,
  );
  const unavailable = getIndexerDownloadClientMappingViewModel(
    directIndexer,
    unavailableCatalog,
  );
  assert.equal(unavailable.invalidReason, "unavailable");
  assert.equal(unavailable.options[0]?.id, "usenet-a");
});

test("managed children get mappings while management-only parents do not", () => {
  const parent = {
    id: "prowlarr-parent",
    isManaged: false,
    supportsManagedChildrenSync: true,
  };
  const child = {
    id: "prowlarr-child",
    isManaged: true,
    supportsManagedChildrenSync: false,
  };

  assert.equal(isManagementOnlyIndexer(parent), true);
  assert.equal(
    getIndexerDownloadClientMappingViewModel(parent, catalog()).isNotApplicable,
    true,
  );
  assert.equal(
    getIndexerDownloadClientMappingViewModel(child, {
      ...catalog(),
      indexers: [{ ...catalog().indexers[0]!, id: child.id }],
    }).isNotApplicable,
    false,
  );
});

test("pending state is row-scoped and mapping updates can roll back", () => {
  const pending = updatePendingIndexerMappingIds(
    new Set(["other-row"]),
    directIndexer.id,
    true,
  );
  assert.equal(pending.has(directIndexer.id), true);
  assert.equal(pending.has("other-row"), true);

  const optimistic = updateIndexerDownloadClientMapping(
    catalog(),
    directIndexer.id,
    "usenet-a",
  );
  const rolledBack = updateIndexerDownloadClientMapping(
    optimistic,
    directIndexer.id,
    null,
  );
  assert.equal(rolledBack.indexers[0]?.downloadClientId, null);
  assert.equal(
    updatePendingIndexerMappingIds(pending, directIndexer.id, false).has(
      "other-row",
    ),
    true,
  );
});

test("catalog refresh retains successful data and rejects superseded responses", () => {
  const ready = completeIndexerDownloadClientCatalogRequest(catalog());
  const refreshing = beginIndexerDownloadClientCatalogRequest(ready);
  assert.equal(refreshing.status, "refreshing");
  assert.equal(refreshing.catalog, ready.catalog);

  const failed = failIndexerDownloadClientCatalogRequest(refreshing, "offline");
  assert.equal(failed.status, "error");
  assert.equal(failed.catalog, ready.catalog);
  assert.equal(failed.error, "offline");
  assert.equal(isLatestIndexerDownloadClientCatalogRequest(2, 3), false);
  assert.equal(isLatestIndexerDownloadClientCatalogRequest(3, 3), true);

  const cold = beginIndexerDownloadClientCatalogRequest({
    catalog: null,
    status: "idle",
    error: null,
  });
  assert.equal(cold.status, "loading");
  assert.equal(cold.catalog, null);
});

test("mapping catalog and deletion copy carry the integration contract", () => {
  assert.equal(
    indexerDownloadClientMappingCatalogQuery.includes(
      "indexerDownloadClientMappingCatalog",
    ),
    true,
  );
  assert.equal(
    indexerDownloadClientMappingCatalogQuery.includes("compatibleClientIds"),
    true,
  );
  assert.equal(
    indexerDownloadClientMappingCatalogQuery.includes("providerCompatibility"),
    true,
  );
  assert.equal(
    en["settings.downloadClientDeleteConfirmDescription"]?.includes(
      "Automatic",
    ),
    true,
  );
  assert.equal(
    en["status.downloadClientDeletedWithMappings"]?.includes("{{count}}"),
    true,
  );
});

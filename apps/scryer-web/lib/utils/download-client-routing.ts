import { DOWNLOAD_CLIENT_ROUTING_EMPTY } from "@/lib/constants/nzbget";
import type {
  DownloadClientRecord,
  DownloadClientRoutingEntry,
  DownloadClientRoutingSettings,
  DownloadClientRoutingSettingsByClient,
} from "@/lib/types";

const DOWNLOAD_CLIENT_ROUTING_DISABLED: DownloadClientRoutingSettings = {
  ...DOWNLOAD_CLIENT_ROUTING_EMPTY,
  enabled: false,
};

export function normalizeDownloadClientRoutingEntry(
  entry: DownloadClientRoutingEntry | undefined,
  fallback: DownloadClientRoutingSettings = DOWNLOAD_CLIENT_ROUTING_EMPTY,
): DownloadClientRoutingSettings {
  return {
    enabled: entry?.enabled ?? fallback.enabled,
    category: entry?.category ?? fallback.category,
    recentQueuePriority:
      entry?.recentQueuePriority ?? fallback.recentQueuePriority,
    olderQueuePriority:
      entry?.olderQueuePriority ?? fallback.olderQueuePriority,
    removeCompleted: entry?.removeCompleted ?? fallback.removeCompleted,
    removeFailed: entry?.removeFailed ?? fallback.removeFailed,
    seedingProfileId: entry?.seedingProfileId ?? fallback.seedingProfileId,
  };
}

export function buildDownloadClientRoutingState(
  clients: DownloadClientRecord[],
  routingEntries: DownloadClientRoutingEntry[],
  fallback: DownloadClientRoutingSettings = DOWNLOAD_CLIENT_ROUTING_EMPTY,
): {
  routing: DownloadClientRoutingSettingsByClient;
  order: string[];
} {
  const parsedRouting = Object.fromEntries(
    routingEntries.map((entry) => [
      entry.clientId,
      normalizeDownloadClientRoutingEntry(entry, fallback),
    ]),
  ) as DownloadClientRoutingSettingsByClient;

  const routing: DownloadClientRoutingSettingsByClient = {};
  for (const client of clients) {
    routing[client.id] = parsedRouting[client.id]
      ? { ...parsedRouting[client.id] }
      : { ...fallback };
  }

  return {
    routing,
    order: [
      ...new Set(
        routingEntries
          .map((entry) => entry.clientId)
          .filter((id) => clients.some((client) => client.id === id)),
      ),
      ...clients
        .filter((client) => !parsedRouting[client.id])
        .map((client) => client.id),
    ],
  };
}

export function serializeDownloadClientRoutingEntries(
  clients: DownloadClientRecord[],
  routing: DownloadClientRoutingSettingsByClient,
  order: string[],
): DownloadClientRoutingEntry[] {
  const orderedClientIds = order.filter((clientId) =>
    clients.some((client) => client.id === clientId),
  );
  const seen = new Set(orderedClientIds);
  const allClientIds = [
    ...orderedClientIds,
    ...clients
      .map((client) => client.id)
      .filter((clientId) => !seen.has(clientId)),
  ];

  return allClientIds.map((clientId) => {
    const entry = routing[clientId] ?? DOWNLOAD_CLIENT_ROUTING_EMPTY;
    return {
      clientId,
      enabled: entry.enabled,
      category: entry.category || null,
      recentQueuePriority: entry.recentQueuePriority || null,
      olderQueuePriority: entry.olderQueuePriority || null,
      removeCompleted: entry.removeCompleted,
      removeFailed: entry.removeFailed,
      seedingProfileId: entry.seedingProfileId,
    };
  });
}

export function disabledDownloadClientRoutingSettings(): DownloadClientRoutingSettings {
  return { ...DOWNLOAD_CLIENT_ROUTING_DISABLED };
}

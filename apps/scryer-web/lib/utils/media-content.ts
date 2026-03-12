import type { ViewCategoryId } from "@/lib/types/quality-profiles";
import type {
  DownloadClientRoutingSettings,
  DownloadClientRoutingSettingsByClient,
} from "@/lib/types";

export type SearchableQualityProfileBody = {
  id: string;
  name: string;
};

export type NzbgetRoutingOrder = Record<ViewCategoryId, string[]>;

const EMPTY_ROUTING_ORDER: NzbgetRoutingOrder = {
  movie: [],
  series: [],
  anime: [],
};

export function getDefaultRoutingOrder(): NzbgetRoutingOrder {
  return {
    movie: [...EMPTY_ROUTING_ORDER.movie],
    series: [...EMPTY_ROUTING_ORDER.series],
    anime: [...EMPTY_ROUTING_ORDER.anime],
  };
}


export function areNzbgetRoutingSettingsEqual(
  left: DownloadClientRoutingSettings,
  right: DownloadClientRoutingSettings,
) {
  return (
    left.enabled === right.enabled &&
    left.category === right.category &&
    left.recentQueuePriority === right.recentQueuePriority &&
    left.olderQueuePriority === right.olderQueuePriority &&
    left.removeCompleted === right.removeCompleted &&
    left.removeFailed === right.removeFailed
  );
}

export function areNzbgetRoutingMapsEqual(
  left: DownloadClientRoutingSettingsByClient,
  right: DownloadClientRoutingSettingsByClient,
) {
  const leftClientIds = Object.keys(left);
  const rightClientIds = Object.keys(right);
  if (leftClientIds.length !== rightClientIds.length) {
    return false;
  }

  for (const clientId of leftClientIds) {
    if (!Object.prototype.hasOwnProperty.call(right, clientId)) {
      return false;
    }
    if (!areNzbgetRoutingSettingsEqual(left[clientId], right[clientId])) {
      return false;
    }
  }

  return true;
}

export function areRoutingOrdersEqual(left: string[], right: string[]) {
  if (left.length !== right.length) {
    return false;
  }

  return left.every((value, index) => value === right[index]);
}

export function buildRoutingOrder(
  clientIds: string[],
  scopeRouting: DownloadClientRoutingSettingsByClient,
): string[] {
  const clientIdSet = new Set(Object.keys(scopeRouting));

  const configuredIds = clientIds.filter((clientId) => clientIdSet.has(clientId));
  const unknownIds = clientIds.filter((clientId) => !clientIdSet.has(clientId));
  return [...configuredIds, ...unknownIds];
}

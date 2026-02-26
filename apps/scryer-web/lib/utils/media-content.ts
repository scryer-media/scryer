import type { ViewCategoryId } from "@/lib/types/quality-profiles";
import type {
  NzbgetCategoryRoutingSettings,
  NzbgetClientRoutingSettingsByClient,
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

export function areQualityProfilesEqual(
  left: SearchableQualityProfileBody[],
  right: SearchableQualityProfileBody[],
) {
  if (left.length !== right.length) {
    return false;
  }

  for (let index = 0; index < left.length; index += 1) {
    if (left[index].id !== right[index].id || left[index].name !== right[index].name) {
      return false;
    }
  }

  return true;
}

export function areNzbgetRoutingSettingsEqual(
  left: NzbgetCategoryRoutingSettings,
  right: NzbgetCategoryRoutingSettings,
) {
  const tagsAreEqual =
    left.tags.length === right.tags.length && left.tags.every((tag, index) => tag === right.tags[index]);

  return (
    left.category === right.category &&
    left.recentPriority === right.recentPriority &&
    left.olderPriority === right.olderPriority &&
    tagsAreEqual &&
    left.removeCompleted === right.removeCompleted &&
    left.removeFailed === right.removeFailed
  );
}

export function areNzbgetRoutingMapsEqual(
  left: NzbgetClientRoutingSettingsByClient,
  right: NzbgetClientRoutingSettingsByClient,
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
  scopeRouting: NzbgetClientRoutingSettingsByClient,
): string[] {
  const clientIdSet = new Set(Object.keys(scopeRouting));

  const configuredIds = clientIds.filter((clientId) => clientIdSet.has(clientId));
  const unknownIds = clientIds.filter((clientId) => !clientIdSet.has(clientId));
  return [...configuredIds, ...unknownIds];
}

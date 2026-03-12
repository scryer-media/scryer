import type { AdminSetting } from "@/lib/types";
import type {
  DownloadClientRecord,
  DownloadClientRoutingSettings,
  DownloadClientRoutingSettingsByClient,
} from "@/lib/types";
import {
  DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
  DOWNLOAD_CLIENT_DEFAULT_CATEGORY_SETTING_KEY,
  NZBGET_CLIENT_ROUTING_DEFAULT_ID,
  DOWNLOAD_CLIENT_ROUTING_EMPTY,
  LEGACY_NZBGET_CATEGORY_SETTING_KEY,
  NZBGET_REMOVE_COMPLETED_SETTING_KEY,
  NZBGET_REMOVE_FAILED_SETTING_KEY,
  NZBGET_OLDER_PRIORITY_SETTING_KEY,
  NZBGET_RECENT_PRIORITY_SETTING_KEY,
} from "@/lib/constants/nzbget";
import { getSettingStringFromItems } from "./settings";
import { readConfigValueAsBoolean, readConfigValueAsString } from "./download-clients";

const LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY = "nzbget.client_routing";

export function parseDownloadClientRoutingSetting(rawValue: unknown): DownloadClientRoutingSettings {
  if (!rawValue || typeof rawValue !== "object" || Array.isArray(rawValue)) {
    return DOWNLOAD_CLIENT_ROUTING_EMPTY;
  }
  const value = rawValue as Record<string, unknown>;
  return {
    enabled: readConfigValueAsBoolean(
      value.enabled ?? value.is_enabled ?? value.isEnabled ?? true,
    ),
    category: readConfigValueAsString(value.category),
    recentQueuePriority: readConfigValueAsString(
      value.recentQueuePriority ?? value.recentPriority ?? value.recent_priority,
    ),
    olderQueuePriority: readConfigValueAsString(
      value.olderQueuePriority ?? value.olderPriority ?? value.older_priority,
    ),
    removeCompleted: readConfigValueAsBoolean(
      value.removeCompleted ?? value.remove_completed ?? value.removeComplete,
    ),
    removeFailed: readConfigValueAsBoolean(
      value.removeFailed ?? value.remove_failed ?? value.removeFailure,
    ),
  };
}

export function parseDownloadClientRoutingFromJson(
  rawValue?: string | null,
): DownloadClientRoutingSettingsByClient {
  if (!rawValue) {
    return {};
  }

  try {
    const parsed = JSON.parse(rawValue);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      return {};
    }

    const source = parsed as Record<string, unknown>;
    const output: DownloadClientRoutingSettingsByClient = {};
    for (const [clientId, clientValue] of Object.entries(source)) {
      const clientSettings = parseDownloadClientRoutingSetting(clientValue);
      if (!clientId.trim()) {
        continue;
      }
      output[clientId.trim()] = clientSettings;
    }
    return output;
  } catch (error) {
    if (process.env.NODE_ENV !== "production") {
      console.warn("Failed to parse nzbget category routing JSON", { rawValue, error });
    }
    return {};
  }
}

export function parseLegacyNzbgetCategoryRoutingFromItems(categoryPayloadItems: AdminSetting[]) {
  const category =
    getSettingStringFromItems(categoryPayloadItems, DOWNLOAD_CLIENT_DEFAULT_CATEGORY_SETTING_KEY) ??
    getSettingStringFromItems(categoryPayloadItems, LEGACY_NZBGET_CATEGORY_SETTING_KEY);
  const recentPriority = getSettingStringFromItems(
    categoryPayloadItems,
    NZBGET_RECENT_PRIORITY_SETTING_KEY,
  );
  const olderPriority = getSettingStringFromItems(
    categoryPayloadItems,
    NZBGET_OLDER_PRIORITY_SETTING_KEY,
  );
  const removeCompletedValue = getSettingStringFromItems(
    categoryPayloadItems,
    NZBGET_REMOVE_COMPLETED_SETTING_KEY,
  );
  const removeFailedValue = getSettingStringFromItems(
    categoryPayloadItems,
    NZBGET_REMOVE_FAILED_SETTING_KEY,
  );
  return {
    enabled: true,
    category,
    recentQueuePriority: recentPriority,
    olderQueuePriority: olderPriority,
    removeCompleted: readConfigValueAsBoolean(removeCompletedValue),
    removeFailed: readConfigValueAsBoolean(removeFailedValue),
  };
}

export function getDefaultNzbgetCategoryRoutingValue(rawValue: string | null | undefined) {
  const rawPayload = parseDownloadClientRoutingFromJson(rawValue);
  if (Object.keys(rawPayload).length > 0) {
    return rawPayload;
  }
  return {};
}

export function buildNzbgetCategoryRoutingForScope(
  categoryPayloadItems: AdminSetting[],
  downloadClients: DownloadClientRecord[],
): DownloadClientRoutingSettingsByClient {
  const rawPayload = getDefaultNzbgetCategoryRoutingValue(
    getSettingStringFromItems(
      categoryPayloadItems,
      DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
    ) ??
      getSettingStringFromItems(
        categoryPayloadItems,
        LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY,
      ),
  );
  const nzbgetClients = downloadClients.filter((client) =>
    client.clientType.trim().toLowerCase() === "nzbget",
  );
  const nzbgetClientIds = nzbgetClients.map((client) => client.id);

  if (nzbgetClientIds.length === 0) {
    return rawPayload;
  }

  const ordered: DownloadClientRoutingSettingsByClient = {};
  const assignedClientIds = new Set<string>();
  for (const [clientId, clientRouting] of Object.entries(rawPayload)) {
    const normalizedClientId = clientId.trim();
    if (!normalizedClientId || normalizedClientId === NZBGET_CLIENT_ROUTING_DEFAULT_ID) {
      continue;
    }
    ordered[normalizedClientId] = clientRouting;
    assignedClientIds.add(normalizedClientId);
  }

  const hasClientSpecificRouting = assignedClientIds.size > 0;
  if (!hasClientSpecificRouting) {
    const legacyFallback = parseLegacyNzbgetCategoryRoutingFromItems(categoryPayloadItems);
    const fallbackClientId = nzbgetClientIds[0];
    if (fallbackClientId) {
      ordered[fallbackClientId] = legacyFallback;
      assignedClientIds.add(fallbackClientId);
    }
  }

  for (const clientId of nzbgetClientIds) {
    if (!ordered[clientId]) {
      ordered[clientId] = DOWNLOAD_CLIENT_ROUTING_EMPTY;
      assignedClientIds.add(clientId);
    }
  }

  for (const [clientId, clientRouting] of Object.entries(rawPayload)) {
    const normalizedClientId = clientId.trim();
    if (
      !normalizedClientId ||
      normalizedClientId === NZBGET_CLIENT_ROUTING_DEFAULT_ID ||
      assignedClientIds.has(normalizedClientId)
    ) {
      continue;
    }
    ordered[normalizedClientId] = clientRouting;
    assignedClientIds.add(normalizedClientId);
  }

  return ordered;
}

export const parseNzbgetCategoryRoutingSetting = parseDownloadClientRoutingSetting;
export const parseNzbgetCategoryRoutingFromJson = parseDownloadClientRoutingFromJson;

import type { AdminSetting } from "@/lib/types";
import type {
  DownloadClientRecord,
  NzbgetCategoryRoutingSettings,
  NzbgetClientRoutingSettingsByClient,
} from "@/lib/types";
import {
  NZBGET_CATEGORY_SETTING_KEY,
  NZBGET_CLIENT_ROUTING_DEFAULT_ID,
  NZBGET_CLIENT_ROUTING_EMPTY,
  NZBGET_CLIENT_ROUTING_SETTINGS_KEY,
  NZBGET_REMOVE_COMPLETED_SETTING_KEY,
  NZBGET_REMOVE_FAILED_SETTING_KEY,
  NZBGET_OLDER_PRIORITY_SETTING_KEY,
  NZBGET_RECENT_PRIORITY_SETTING_KEY,
  NZBGET_TAGS_SETTING_KEY,
} from "@/lib/constants/nzbget";
import { getSettingStringFromItems } from "./settings";
import { readConfigValueAsBoolean, readConfigValueAsString } from "./download-clients";

function parseNzbgetTags(rawValue: unknown): string[] {
  if (Array.isArray(rawValue)) {
    return rawValue
      .map((value) => (typeof value === "string" ? value.trim() : ""))
      .filter((value) => value.length > 0);
  }

  if (typeof rawValue === "string") {
    return rawValue
      .split(",")
      .map((value) => value.trim())
      .filter((value) => value.length > 0);
  }

  return [];
}

export function parseNzbgetCategoryRoutingSetting(rawValue: unknown): NzbgetCategoryRoutingSettings {
  if (!rawValue || typeof rawValue !== "object" || Array.isArray(rawValue)) {
    return NZBGET_CLIENT_ROUTING_EMPTY;
  }
  const value = rawValue as Record<string, unknown>;
  return {
    category: readConfigValueAsString(value.category),
    recentPriority: readConfigValueAsString(
      value.recentPriority ?? value.recent_priority,
    ),
    olderPriority: readConfigValueAsString(
      value.olderPriority ?? value.older_priority,
    ),
    removeCompleted: readConfigValueAsBoolean(
      value.removeCompleted ?? value.remove_completed ?? value.removeComplete,
    ),
    removeFailed: readConfigValueAsBoolean(
      value.removeFailed ?? value.remove_failed ?? value.removeFailure,
    ),
    tags: parseNzbgetTags(value.tags),
  };
}

export function parseNzbgetCategoryRoutingFromJson(
  rawValue?: string | null,
): NzbgetClientRoutingSettingsByClient {
  if (!rawValue) {
    return {};
  }

  try {
    const parsed = JSON.parse(rawValue);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      return {};
    }

    const source = parsed as Record<string, unknown>;
    const output: NzbgetClientRoutingSettingsByClient = {};
    for (const [clientId, clientValue] of Object.entries(source)) {
      const clientSettings = parseNzbgetCategoryRoutingSetting(clientValue);
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
  const category = getSettingStringFromItems(categoryPayloadItems, NZBGET_CATEGORY_SETTING_KEY);
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
  const tags = getSettingStringFromItems(categoryPayloadItems, NZBGET_TAGS_SETTING_KEY);
  return {
    category,
    recentPriority,
    olderPriority,
    removeCompleted: readConfigValueAsBoolean(removeCompletedValue),
    removeFailed: readConfigValueAsBoolean(removeFailedValue),
    tags: parseNzbgetTags(tags),
  };
}

export function getDefaultNzbgetCategoryRoutingValue(rawValue: string | null | undefined) {
  const rawPayload = parseNzbgetCategoryRoutingFromJson(rawValue);
  if (Object.keys(rawPayload).length > 0) {
    return rawPayload;
  }
  return {};
}

export function buildNzbgetCategoryRoutingForScope(
  categoryPayloadItems: AdminSetting[],
  downloadClients: DownloadClientRecord[],
): NzbgetClientRoutingSettingsByClient {
  const rawPayload = getDefaultNzbgetCategoryRoutingValue(
    getSettingStringFromItems(categoryPayloadItems, NZBGET_CLIENT_ROUTING_SETTINGS_KEY),
  );
  const nzbgetClients = downloadClients.filter((client) =>
    client.clientType.trim().toLowerCase() === "nzbget",
  );
  const nzbgetClientIds = nzbgetClients.map((client) => client.id);

  if (nzbgetClientIds.length === 0) {
    return rawPayload;
  }

  const ordered: NzbgetClientRoutingSettingsByClient = {};
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
      ordered[clientId] = NZBGET_CLIENT_ROUTING_EMPTY;
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

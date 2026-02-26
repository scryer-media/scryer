import type {
  IndexerCategoryRoutingSettings,
  IndexerRoutingSettingsByIndexer,
} from "@/lib/types";
import { INDEXER_ROUTING_EMPTY } from "@/lib/constants/indexers";
import { readConfigValueAsBoolean } from "@/lib/utils/download-clients";

function parseIndexerCategories(rawValue: unknown): string[] {
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

function parseIndexerPriority(rawValue: unknown): number | null {
  if (typeof rawValue === "number" && Number.isFinite(rawValue)) {
    return rawValue > 0 ? rawValue : null;
  }
  if (typeof rawValue === "string") {
    const parsed = Number(rawValue.trim());
    return Number.isFinite(parsed) && parsed > 0 ? parsed : null;
  }
  return null;
}

export function parseIndexerCategoryRoutingSetting(
  rawValue: unknown,
): IndexerCategoryRoutingSettings {
  if (!rawValue || typeof rawValue !== "object" || Array.isArray(rawValue)) {
    return INDEXER_ROUTING_EMPTY;
  }

  const value = rawValue as Record<string, unknown>;
  const rawEnabled = value.enabled ?? value.is_enabled ?? value.isEnabled;
  const enabled =
    typeof rawEnabled === "undefined"
      ? INDEXER_ROUTING_EMPTY.enabled
      : readConfigValueAsBoolean(rawEnabled);
  const priority = parseIndexerPriority(value.priority);

  return {
    categories: parseIndexerCategories(value.categories ?? value.category),
    enabled,
    priority: priority ?? INDEXER_ROUTING_EMPTY.priority,
  };
}

export function parseIndexerCategoryRoutingFromJson(
  rawValue?: string | null,
): IndexerRoutingSettingsByIndexer {
  if (!rawValue) {
    return {};
  }

  try {
    const parsed = JSON.parse(rawValue);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      return {};
    }

    const source = parsed as Record<string, unknown>;
    const output: IndexerRoutingSettingsByIndexer = {};
    for (const [indexerId, indexerValue] of Object.entries(source)) {
      const normalizedId = indexerId.trim();
      if (!normalizedId) {
        continue;
      }
      output[normalizedId] = parseIndexerCategoryRoutingSetting(indexerValue);
    }
    return output;
  } catch (error) {
    if (process.env.NODE_ENV !== "production") {
      console.warn("Failed to parse indexer category routing JSON", { rawValue, error });
    }
    return {};
  }
}

export function areIndexerRoutingSettingsEqual(
  left: IndexerCategoryRoutingSettings,
  right: IndexerCategoryRoutingSettings,
) {
  const categoriesEqual =
    left.categories.length === right.categories.length &&
    left.categories.every((category, index) => category === right.categories[index]);

  return (
    categoriesEqual &&
    left.enabled === right.enabled &&
    left.priority === right.priority
  );
}

export function areIndexerRoutingMapsEqual(
  left: IndexerRoutingSettingsByIndexer,
  right: IndexerRoutingSettingsByIndexer,
) {
  const leftIds = Object.keys(left);
  const rightIds = Object.keys(right);
  if (leftIds.length !== rightIds.length) {
    return false;
  }

  for (const indexerId of leftIds) {
    if (!Object.prototype.hasOwnProperty.call(right, indexerId)) {
      return false;
    }
    if (!areIndexerRoutingSettingsEqual(left[indexerId], right[indexerId])) {
      return false;
    }
  }

  return true;
}

export function buildIndexerRoutingOrder(
  indexerIds: string[],
  scopeRouting: IndexerRoutingSettingsByIndexer,
): string[] {
  const configuredIds = indexerIds.filter((indexerId) => scopeRouting[indexerId]);
  const missingIds = indexerIds.filter((indexerId) => !scopeRouting[indexerId]);

  const hasExplicitPriority = configuredIds.some(
    (indexerId) => scopeRouting[indexerId]?.priority > 0,
  );
  if (!hasExplicitPriority) {
    return [...configuredIds, ...missingIds];
  }

  const positionById = new Map(indexerIds.map((id, index) => [id, index]));
  const sorted = [...configuredIds].sort((leftId, rightId) => {
    const leftPriority = scopeRouting[leftId]?.priority ?? Number.MAX_SAFE_INTEGER;
    const rightPriority = scopeRouting[rightId]?.priority ?? Number.MAX_SAFE_INTEGER;
    if (leftPriority !== rightPriority) {
      return leftPriority - rightPriority;
    }
    return (positionById.get(leftId) ?? 0) - (positionById.get(rightId) ?? 0);
  });

  return [...sorted, ...missingIds];
}

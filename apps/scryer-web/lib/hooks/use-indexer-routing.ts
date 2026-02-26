import * as React from "react";

import { saveAdminSettingsMutation } from "@/lib/graphql/mutations";
import { indexerRoutingInitQuery } from "@/lib/graphql/queries";
import { INDEXER_ROUTING_SETTINGS_KEY, getDefaultIndexerRouting } from "@/lib/constants/indexers";
import { useClient } from "urql";
import { getSettingStringFromItems } from "@/lib/utils/settings";
import {
  areIndexerRoutingMapsEqual,
  buildIndexerRoutingOrder,
  parseIndexerCategoryRoutingFromJson,
} from "@/lib/utils/indexer-routing";
import { areRoutingOrdersEqual, getDefaultRoutingOrder } from "@/lib/utils/media-content";
import type {
  IndexerRecord,
  IndexerRoutingSettingsByIndexer,
  IndexerRoutingSettingsByScope,
  IndexerCategoryRoutingSettings,
} from "@/lib/types";
import type { ViewCategoryId } from "@/lib/types/quality-profiles";
import type { Translate } from "@/components/root/types";

const DEFAULT_SCOPE_ROUTING_ORDER = getDefaultRoutingOrder();

type Direction = "up" | "down";

type IndexerRoutingHookArgs = {
  activeQualityScopeId: ViewCategoryId;
  setGlobalStatus: (status: string) => void;
  t: Translate;
};

export type IndexerRoutingHookResult = {
  indexers: IndexerRecord[];
  activeScopeRouting: IndexerRoutingSettingsByIndexer;
  activeScopeRoutingOrder: string[];
  indexerRoutingLoading: boolean;
  indexerRoutingSaving: boolean;
  refreshIndexerRouting: () => Promise<void>;
  setIndexerEnabledForScope: (indexerId: string, enabled: boolean) => Promise<void>;
  updateIndexerRoutingForScope: (
    indexerId: string,
    nextValue: Partial<IndexerCategoryRoutingSettings>,
  ) => Promise<void>;
  moveIndexerInScope: (indexerId: string, direction: Direction) => void;
};

export function useIndexerRouting({
  activeQualityScopeId,
  setGlobalStatus,
  t,
}: IndexerRoutingHookArgs): IndexerRoutingHookResult {
  const client = useClient();
  const [indexers, setIndexers] = React.useState<IndexerRecord[]>([]);
  const [indexerRoutingByScope, setIndexerRoutingByScope] =
    React.useState<IndexerRoutingSettingsByScope>({
      movie: {},
      series: {},
      anime: {},
    });
  const [indexerRoutingOrderByScope, setIndexerRoutingOrderByScope] =
    React.useState<Record<ViewCategoryId, string[]>>({ ...DEFAULT_SCOPE_ROUTING_ORDER });
  const [indexerRoutingSaving, setIndexerRoutingSaving] =
    React.useState<Record<ViewCategoryId, boolean>>({ movie: false, series: false, anime: false });
  const [indexerRoutingLoading, setIndexerRoutingLoading] = React.useState(false);

  const activeScopeRouting = indexerRoutingByScope[activeQualityScopeId] ?? {};
  const activeScopeRoutingOrder = indexerRoutingOrderByScope[activeQualityScopeId] ?? [];

  const buildIndexerRoutingPayload = React.useCallback(
    (
      scopeId: ViewCategoryId,
      scopeRouting: IndexerRoutingSettingsByIndexer,
      scopeOrder: string[],
    ): Record<string, IndexerCategoryRoutingSettings> => {
      const payload: Record<string, IndexerCategoryRoutingSettings> = {};
      const scopeDefaults = getDefaultIndexerRouting(scopeId);
      let nextPriority = 1;

      for (const indexerId of scopeOrder) {
        const routing = scopeRouting[indexerId];
        if (!routing) {
          continue;
        }
        payload[indexerId] = { ...routing, priority: nextPriority };
        nextPriority += 1;
      }

      for (const indexer of indexers) {
        if (payload[indexer.id]) {
          continue;
        }
        const routing = scopeRouting[indexer.id] ?? scopeDefaults;
        payload[indexer.id] = { ...routing, priority: nextPriority };
        nextPriority += 1;
      }

      return payload;
    },
    [indexers],
  );

  const saveIndexerRoutingForScope = React.useCallback(
    async (scopeId: ViewCategoryId, scopeRouting: IndexerRoutingSettingsByIndexer, scopeOrder: string[]) => {
      setIndexerRoutingSaving((previous) => ({
        ...previous,
        [scopeId]: true,
      }));

      try {
        const payload = buildIndexerRoutingPayload(scopeId, scopeRouting, scopeOrder);
        const { data: saveData, error: saveError } = await client.mutation(
          saveAdminSettingsMutation,
          {
            input: {
              scope: "system",
              scopeId,
              items: [
                {
                  keyName: INDEXER_ROUTING_SETTINGS_KEY,
                  value: JSON.stringify(payload),
                },
              ],
            },
          },
        ).toPromise();
        if (saveError) throw saveError;

        const rawSavedRoutingValue = getSettingStringFromItems(
          saveData.saveAdminSettings.items,
          INDEXER_ROUTING_SETTINGS_KEY,
        );
        const savedRouting = parseIndexerCategoryRoutingFromJson(rawSavedRoutingValue);
        const normalizedSavedRouting: IndexerRoutingSettingsByIndexer = {};
        const scopeDefaults = getDefaultIndexerRouting(scopeId);
        for (const indexer of indexers) {
          const routing = savedRouting[indexer.id] ?? scopeRouting[indexer.id] ?? scopeDefaults;
          normalizedSavedRouting[indexer.id] = { ...scopeDefaults, ...routing };
        }

        const nextOrder = buildIndexerRoutingOrder(
          indexers.map((indexer) => indexer.id),
          normalizedSavedRouting,
        );

        setIndexerRoutingByScope((previous) => {
          const currentScopeRouting = previous[scopeId] ?? {};
          if (areIndexerRoutingMapsEqual(currentScopeRouting, normalizedSavedRouting)) {
            return previous;
          }
          return {
            ...previous,
            [scopeId]: normalizedSavedRouting,
          };
        });
        setIndexerRoutingOrderByScope((previous) => {
          const currentOrder = previous[scopeId] ?? [];
          if (areRoutingOrdersEqual(currentOrder, nextOrder)) {
            return previous;
          }

          return {
            ...previous,
            [scopeId]: nextOrder,
          };
        });
        setGlobalStatus(t("settings.qualitySettingsSaved"));
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
      } finally {
        setIndexerRoutingSaving((previous) => ({
          ...previous,
          [scopeId]: false,
        }));
      }
    },
    [buildIndexerRoutingPayload, client, indexers, setGlobalStatus, t],
  );

  const refreshIndexerRouting = React.useCallback(async () => {
    setIndexerRoutingLoading(true);
    try {
      const { data, error } = await client.query(indexerRoutingInitQuery, { scopeId: activeQualityScopeId }).toPromise();
      if (error) throw error;

      const indexerList = data.indexers || [];
      const rawRoutingValue = getSettingStringFromItems(
        data.categorySettings.items,
        INDEXER_ROUTING_SETTINGS_KEY,
      );
      const parsedRouting = parseIndexerCategoryRoutingFromJson(rawRoutingValue);

      setIndexers(indexerList);

      const scopeDefaults = getDefaultIndexerRouting(activeQualityScopeId);
      const scopeRouting: IndexerRoutingSettingsByIndexer = {};
      for (const indexer of indexerList) {
        const routing = parsedRouting[indexer.id];
        scopeRouting[indexer.id] = routing
          ? { ...scopeDefaults, ...routing }
          : { ...scopeDefaults };
      }

      setIndexerRoutingByScope((previous) => {
        const currentScopeRouting = previous[activeQualityScopeId] ?? {};
        if (areIndexerRoutingMapsEqual(currentScopeRouting, scopeRouting)) {
          return previous;
        }
        return {
          ...previous,
          [activeQualityScopeId]: scopeRouting,
        };
      });

      const nextOrder = buildIndexerRoutingOrder(
        indexerList.map((indexer: IndexerRecord) => indexer.id),
        scopeRouting,
      );
      setIndexerRoutingOrderByScope((previous) => {
        const currentOrder = previous[activeQualityScopeId] ?? [];
        if (areRoutingOrdersEqual(currentOrder, nextOrder)) {
          return previous;
        }
        return {
          ...previous,
          [activeQualityScopeId]: nextOrder,
        };
      });
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
    } finally {
      setIndexerRoutingLoading(false);
    }
  }, [activeQualityScopeId, client, setGlobalStatus, t]);

  const updateIndexerRoutingForScope = React.useCallback(
    async (indexerId: string, nextValue: Partial<IndexerCategoryRoutingSettings>) => {
      const scopeId = activeQualityScopeId;
      const scopeDefaults = getDefaultIndexerRouting(scopeId);
      const currentScopeRouting = indexerRoutingByScope[scopeId] ?? {};
      const current = currentScopeRouting[indexerId] ?? scopeDefaults;
      const nextScopeRouting = {
        ...currentScopeRouting,
        [indexerId]: {
          ...current,
          ...nextValue,
        },
      };

      const currentOrder = indexerRoutingOrderByScope[scopeId] ?? [];
      const nextOrder = currentOrder.includes(indexerId)
        ? currentOrder
        : [...currentOrder, indexerId];

      setIndexerRoutingByScope((previous) => {
        const scopeRouting = previous[scopeId] ?? {};
        return {
          ...previous,
          [scopeId]: {
            ...scopeRouting,
            [indexerId]: nextScopeRouting[indexerId],
          },
        };
      });

      setIndexerRoutingOrderByScope((previous) => {
        const currentOrder = previous[scopeId] ?? [];
        if (currentOrder.includes(indexerId)) {
          return previous;
        }
        return {
          ...previous,
          [scopeId]: [...currentOrder, indexerId],
        };
      });

      await saveIndexerRoutingForScope(scopeId, nextScopeRouting, nextOrder);
    },
    [activeQualityScopeId, indexerRoutingByScope, indexerRoutingOrderByScope, saveIndexerRoutingForScope],
  );

  const setIndexerEnabledForScope = React.useCallback(
    async (indexerId: string, enabled: boolean) => {
      await updateIndexerRoutingForScope(indexerId, { enabled });
    },
    [updateIndexerRoutingForScope],
  );

  const moveIndexerInScope = React.useCallback(
    (indexerId: string, direction: Direction) => {
      const scopeId = activeQualityScopeId;
      const scopeRouting = indexerRoutingByScope[scopeId] ?? {};

      setIndexerRoutingOrderByScope((previous) => {
        const currentOrder = previous[scopeId] ?? [];
        const index = currentOrder.indexOf(indexerId);
        if (index < 0) {
          return previous;
        }
        const nextIndex = direction === "up" ? index - 1 : index + 1;
        if (nextIndex < 0 || nextIndex >= currentOrder.length) {
          return previous;
        }
        const nextOrder = [...currentOrder];
        [nextOrder[index], nextOrder[nextIndex]] = [nextOrder[nextIndex], nextOrder[index]];

        void saveIndexerRoutingForScope(scopeId, scopeRouting, nextOrder);

        return {
          ...previous,
          [scopeId]: nextOrder,
        };
      });
    },
    [activeQualityScopeId, indexerRoutingByScope, saveIndexerRoutingForScope],
  );

  return {
    indexers,
    activeScopeRouting,
    activeScopeRoutingOrder,
    indexerRoutingLoading,
    indexerRoutingSaving: indexerRoutingSaving[activeQualityScopeId],
    refreshIndexerRouting,
    setIndexerEnabledForScope,
    updateIndexerRoutingForScope,
    moveIndexerInScope,
  };
}

import * as React from "react";

import { saveAdminSettingsMutation } from "@/lib/graphql/mutations";
import { downloadClientRoutingInitQuery } from "@/lib/graphql/queries";
import { NZBGET_CLIENT_ROUTING_EMPTY, NZBGET_CLIENT_ROUTING_SETTINGS_KEY } from "@/lib/constants/nzbget";
import { useClient } from "urql";
import { getSettingStringFromItems } from "@/lib/utils/settings";
import { parseNzbgetCategoryRoutingFromJson } from "@/lib/utils/nzbget-routing";
import {
  buildRoutingOrder,
  getDefaultRoutingOrder,
  areNzbgetRoutingMapsEqual,
  areRoutingOrdersEqual,
  type NzbgetRoutingOrder,
} from "@/lib/utils/media-content";
import type {
  DownloadClientRecord,
  NzbgetClientRoutingSettingsByClient,
  NzbgetClientRoutingSettingsByScope,
  NzbgetCategoryRoutingSettings,
} from "@/lib/types";
import { type ViewCategoryId } from "@/lib/types/quality-profiles";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";

const DEFAULT_SCOPE_ROUTING_ORDER = getDefaultRoutingOrder();

type Direction = "up" | "down";

type DownloadClientRoutingHookArgs = {
  activeQualityScopeId: ViewCategoryId;
};

export type DownloadClientRoutingHookResult = {
  downloadClients: DownloadClientRecord[];
  activeScopeRouting: NzbgetClientRoutingSettingsByClient;
  activeScopeRoutingOrder: string[];
  downloadClientRoutingLoading: boolean;
  downloadClientRoutingSaving: boolean;
  refreshDownloadClientRouting: () => Promise<void>;
  updateDownloadClientRoutingForScope: (
    clientId: string,
    nextValue: Partial<NzbgetCategoryRoutingSettings>,
  ) => void;
  moveDownloadClientInScope: (clientId: string, direction: Direction) => void;
  saveDownloadClientRouting: () => Promise<void>;
};

export function useDownloadClientRouting({
  activeQualityScopeId,
}: DownloadClientRoutingHookArgs): DownloadClientRoutingHookResult {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const [downloadClients, setDownloadClients] = React.useState<DownloadClientRecord[]>([]);
  const [downloadClientRoutingByScope, setDownloadClientRoutingByScope] =
    React.useState<NzbgetClientRoutingSettingsByScope>({
      movie: {},
      series: {},
      anime: {},
    });
  const [downloadClientRoutingOrderByScope, setDownloadClientRoutingOrderByScope] =
    React.useState<NzbgetRoutingOrder>({ ...DEFAULT_SCOPE_ROUTING_ORDER });
  const [downloadClientRoutingSaving, setDownloadClientRoutingSaving] =
    React.useState<Record<ViewCategoryId, boolean>>({ movie: false, series: false, anime: false });
  const [downloadClientRoutingLoading, setDownloadClientRoutingLoading] = React.useState(false);

  const activeScopeRouting = downloadClientRoutingByScope[activeQualityScopeId] ?? {};
  const activeScopeRoutingOrder = downloadClientRoutingOrderByScope[activeQualityScopeId] ?? [];

  const refreshDownloadClientRouting = React.useCallback(async () => {
    setDownloadClientRoutingLoading(true);
    try {
      const { data, error } = await client.query(downloadClientRoutingInitQuery, { scopeId: activeQualityScopeId }).toPromise();
      if (error) throw error;

      const clientBody = { downloadClientConfigs: data.downloadClientConfigs };
      const categoryBody = { adminSettings: data.categorySettings };

      const clients = clientBody.downloadClientConfigs || [];
      const rawRoutingValue = getSettingStringFromItems(
        categoryBody.adminSettings.items,
        NZBGET_CLIENT_ROUTING_SETTINGS_KEY,
      );
      const parsedRouting = parseNzbgetCategoryRoutingFromJson(rawRoutingValue);

      setDownloadClients(clients);

      const scopeRouting: NzbgetClientRoutingSettingsByClient = {};
      for (const client of clients) {
        const clientRouting = parsedRouting[client.id] ?? NZBGET_CLIENT_ROUTING_EMPTY;
        scopeRouting[client.id] = { ...NZBGET_CLIENT_ROUTING_EMPTY, ...clientRouting };
      }

      setDownloadClientRoutingByScope((previous) => {
        const currentScopeRouting = previous[activeQualityScopeId] ?? {};
        if (areNzbgetRoutingMapsEqual(currentScopeRouting, scopeRouting)) {
          return previous;
        }

        return {
          ...previous,
          [activeQualityScopeId]: scopeRouting,
        };
      });
      const nextOrder = buildRoutingOrder(
        clients.map((client: DownloadClientRecord) => client.id),
        scopeRouting,
      );
      setDownloadClientRoutingOrderByScope((previous) => {
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
      setDownloadClientRoutingLoading(false);
    }
  }, [activeQualityScopeId, client, setGlobalStatus, t]);

  const updateDownloadClientRoutingForScope = React.useCallback(
    (clientId: string, nextValue: Partial<NzbgetCategoryRoutingSettings>) => {
      setDownloadClientRoutingByScope((previous) => {
        const scopeRouting = previous[activeQualityScopeId] ?? {};
        const current = scopeRouting[clientId] ?? NZBGET_CLIENT_ROUTING_EMPTY;
        return {
          ...previous,
          [activeQualityScopeId]: {
            ...scopeRouting,
            [clientId]: {
              ...current,
              ...nextValue,
            },
          },
        };
      });

      setDownloadClientRoutingOrderByScope((previous) => {
        const currentOrder = previous[activeQualityScopeId] ?? [];
        if (currentOrder.includes(clientId)) {
          return previous;
        }
        return {
          ...previous,
          [activeQualityScopeId]: [...currentOrder, clientId],
        };
      });
    },
    [activeQualityScopeId],
  );

  const moveDownloadClientInScope = React.useCallback(
    (clientId: string, direction: Direction) => {
      setDownloadClientRoutingOrderByScope((previous) => {
        const currentOrder = previous[activeQualityScopeId] ?? [];
        const index = currentOrder.indexOf(clientId);
        if (index < 0) {
          return previous;
        }

        const nextIndex = direction === "up" ? index - 1 : index + 1;
        if (nextIndex < 0 || nextIndex >= currentOrder.length) {
          return previous;
        }

        const nextOrder = [...currentOrder];
        [nextOrder[index], nextOrder[nextIndex]] = [nextOrder[nextIndex], nextOrder[index]];
        return {
          ...previous,
          [activeQualityScopeId]: nextOrder,
        };
      });
    },
    [activeQualityScopeId],
  );

  const saveDownloadClientRouting = React.useCallback(async () => {
    setDownloadClientRoutingSaving((previous) => ({
      ...previous,
      [activeQualityScopeId]: true,
    }));
    try {
      const scopeRouting = downloadClientRoutingByScope[activeQualityScopeId] ?? {};
      const order = downloadClientRoutingOrderByScope[activeQualityScopeId] ?? [];
      const payload: Record<string, NzbgetCategoryRoutingSettings> = {};

      for (const clientId of order) {
        if (scopeRouting[clientId]) {
          payload[clientId] = scopeRouting[clientId];
        }
      }

      for (const client of downloadClients) {
        if (!payload[client.id]) {
          payload[client.id] = scopeRouting[client.id] ?? NZBGET_CLIENT_ROUTING_EMPTY;
        }
      }

      const { data: saveData, error: saveError } = await client.mutation(
        saveAdminSettingsMutation,
        {
          input: {
            scope: "system",
            scopeId: activeQualityScopeId,
            items: [
              {
                keyName: NZBGET_CLIENT_ROUTING_SETTINGS_KEY,
                value: JSON.stringify(payload),
              },
            ],
          },
        },
      ).toPromise();
      if (saveError) throw saveError;

      const rawSavedRoutingValue = getSettingStringFromItems(
        saveData.saveAdminSettings.items,
        NZBGET_CLIENT_ROUTING_SETTINGS_KEY,
      );
      const savedRouting = parseNzbgetCategoryRoutingFromJson(rawSavedRoutingValue);
      const normalizedSavedRouting: NzbgetClientRoutingSettingsByClient = {};
      for (const client of downloadClients) {
        const routing = savedRouting[client.id] ?? scopeRouting[client.id] ?? NZBGET_CLIENT_ROUTING_EMPTY;
        normalizedSavedRouting[client.id] = { ...NZBGET_CLIENT_ROUTING_EMPTY, ...routing };
      }
      const nextOrder = buildRoutingOrder(
        downloadClients.map((client) => client.id),
        normalizedSavedRouting,
      );

      setDownloadClientRoutingByScope((previous) => {
        const currentScopeRouting = previous[activeQualityScopeId] ?? {};
        if (areNzbgetRoutingMapsEqual(currentScopeRouting, normalizedSavedRouting)) {
          return previous;
        }

        return {
          ...previous,
          [activeQualityScopeId]: normalizedSavedRouting,
        };
      });
      setDownloadClientRoutingOrderByScope((previous) => {
        const currentOrder = previous[activeQualityScopeId] ?? [];
        if (areRoutingOrdersEqual(currentOrder, nextOrder)) {
          return previous;
        }

        return {
          ...previous,
          [activeQualityScopeId]: nextOrder,
        };
      });
      setGlobalStatus(t("settings.qualitySettingsSaved"));
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setDownloadClientRoutingSaving((previous) => ({
        ...previous,
        [activeQualityScopeId]: false,
      }));
    }
  }, [
    activeQualityScopeId,
    downloadClientRoutingByScope,
    downloadClientRoutingOrderByScope,
    downloadClients,
    client,
    setGlobalStatus,
    t,
  ]);

  return {
    downloadClients,
    activeScopeRouting,
    activeScopeRoutingOrder,
    downloadClientRoutingLoading,
    downloadClientRoutingSaving: downloadClientRoutingSaving[activeQualityScopeId],
    refreshDownloadClientRouting,
    updateDownloadClientRoutingForScope,
    moveDownloadClientInScope,
    saveDownloadClientRouting,
  };
}

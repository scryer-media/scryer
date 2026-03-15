import * as React from "react";

import { saveAdminSettingsMutation } from "@/lib/graphql/mutations";
import { downloadClientRoutingInitQuery } from "@/lib/graphql/queries";
import {
  DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
  DOWNLOAD_CLIENT_ROUTING_EMPTY,
} from "@/lib/constants/nzbget";
import { useClient } from "urql";
import { getSettingStringFromItems } from "@/lib/utils/settings";
import { parseDownloadClientRoutingFromJson } from "@/lib/utils/nzbget-routing";
import {
  buildRoutingOrder,
  getDefaultRoutingOrder,
  areNzbgetRoutingMapsEqual,
  areRoutingOrdersEqual,
  type NzbgetRoutingOrder,
} from "@/lib/utils/media-content";
import type {
  AdminSettingsResponse,
  DownloadClientRecord,
  DownloadClientRoutingSettingsByClient,
  DownloadClientRoutingSettingsByScope,
  DownloadClientRoutingSettings,
} from "@/lib/types";
import { type ViewCategoryId } from "@/lib/types/quality-profiles";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useSettingsSubscription } from "@/lib/hooks/use-settings-subscription";

const DEFAULT_SCOPE_ROUTING_ORDER = getDefaultRoutingOrder();
const LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY = "nzbget.client_routing";

type Direction = "up" | "down";

type DownloadClientRoutingHookArgs = {
  activeQualityScopeId: ViewCategoryId;
};

type DownloadClientRoutingUpdateOptions = {
  save?: boolean;
  successMessage?: string;
};

export type DownloadClientRoutingHookResult = {
  downloadClients: DownloadClientRecord[];
  activeScopeRouting: DownloadClientRoutingSettingsByClient;
  activeScopeRoutingOrder: string[];
  downloadClientRoutingLoading: boolean;
  downloadClientRoutingSaving: boolean;
  hydrateDownloadClientRouting: (
    clients: DownloadClientRecord[],
    categorySettings: AdminSettingsResponse,
  ) => void;
  refreshDownloadClientRouting: () => Promise<void>;
  updateDownloadClientRoutingForScope: (
    clientId: string,
    nextValue: Partial<DownloadClientRoutingSettings>,
    options?: DownloadClientRoutingUpdateOptions,
  ) => Promise<void>;
  moveDownloadClientInScope: (clientId: string, direction: Direction) => void;
};

export function useDownloadClientRouting({
  activeQualityScopeId,
}: DownloadClientRoutingHookArgs): DownloadClientRoutingHookResult {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const [downloadClients, setDownloadClients] = React.useState<
    DownloadClientRecord[]
  >([]);
  const [downloadClientRoutingByScope, setDownloadClientRoutingByScope] =
    React.useState<DownloadClientRoutingSettingsByScope>({
      movie: {},
      series: {},
      anime: {},
    });
  const [
    downloadClientRoutingOrderByScope,
    setDownloadClientRoutingOrderByScope,
  ] = React.useState<NzbgetRoutingOrder>({ ...DEFAULT_SCOPE_ROUTING_ORDER });
  const [downloadClientRoutingSaving, setDownloadClientRoutingSaving] =
    React.useState<Record<ViewCategoryId, boolean>>({
      movie: false,
      series: false,
      anime: false,
    });
  const [downloadClientRoutingLoading, setDownloadClientRoutingLoading] =
    React.useState(false);

  const activeScopeRouting =
    downloadClientRoutingByScope[activeQualityScopeId] ?? {};
  const activeScopeRoutingOrder =
    downloadClientRoutingOrderByScope[activeQualityScopeId] ?? [];

  const hydrateDownloadClientRouting = React.useCallback(
    (
      clients: DownloadClientRecord[],
      categorySettings: AdminSettingsResponse,
    ) => {
      const rawRoutingValue = getSettingStringFromItems(
        categorySettings.items,
        DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
      ) ??
        getSettingStringFromItems(
          categorySettings.items,
          LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY,
        );
      const parsedRouting = parseDownloadClientRoutingFromJson(rawRoutingValue);

      setDownloadClients(clients);

      const scopeRouting: DownloadClientRoutingSettingsByClient = {};
      for (const client of clients) {
        const clientRouting =
          parsedRouting[client.id] ?? DOWNLOAD_CLIENT_ROUTING_EMPTY;
        scopeRouting[client.id] = {
          ...DOWNLOAD_CLIENT_ROUTING_EMPTY,
          ...clientRouting,
        };
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
        clients.map((client) => client.id),
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
    },
    [activeQualityScopeId],
  );

  const refreshDownloadClientRouting = React.useCallback(async () => {
    setDownloadClientRoutingLoading(true);
    try {
      const { data, error } = await client
        .query(downloadClientRoutingInitQuery, {
          scopeId: activeQualityScopeId,
        })
        .toPromise();
      if (error) throw error;
      hydrateDownloadClientRouting(
        data.downloadClientConfigs || [],
        data.categorySettings,
      );
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.failedToLoad"),
      );
    } finally {
      setDownloadClientRoutingLoading(false);
    }
  }, [
    activeQualityScopeId,
    client,
    hydrateDownloadClientRouting,
    setGlobalStatus,
    t,
  ]);

  const saveDownloadClientRoutingForScope = React.useCallback(
    async (
      scopeId: ViewCategoryId,
      scopeRouting: DownloadClientRoutingSettingsByClient,
      order: string[],
      successMessage?: string,
    ) => {
      setDownloadClientRoutingSaving((previous) => ({
        ...previous,
        [scopeId]: true,
      }));
      try {
        const payload: Record<string, DownloadClientRoutingSettings> = {};

        for (const clientId of order) {
          if (scopeRouting[clientId]) {
            payload[clientId] = scopeRouting[clientId];
          }
        }

        for (const client of downloadClients) {
          if (!payload[client.id]) {
            payload[client.id] =
              scopeRouting[client.id] ?? DOWNLOAD_CLIENT_ROUTING_EMPTY;
          }
        }

        const { data: saveData, error: saveError } = await client
          .mutation(saveAdminSettingsMutation, {
            input: {
              scope: "system",
              scopeId,
              items: [
                {
                  keyName: DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
                  value: JSON.stringify(payload),
                },
              ],
            },
          })
          .toPromise();
        if (saveError) throw saveError;

        const rawSavedRoutingValue = getSettingStringFromItems(
          saveData.saveAdminSettings.items,
          DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
        );
        const savedRouting =
          parseDownloadClientRoutingFromJson(rawSavedRoutingValue);
        const normalizedSavedRouting: DownloadClientRoutingSettingsByClient = {};
        for (const client of downloadClients) {
          const routing =
            savedRouting[client.id] ??
            scopeRouting[client.id] ??
            DOWNLOAD_CLIENT_ROUTING_EMPTY;
          normalizedSavedRouting[client.id] = {
            ...DOWNLOAD_CLIENT_ROUTING_EMPTY,
            ...routing,
          };
        }
        const nextOrder = buildRoutingOrder(
          downloadClients.map((client) => client.id),
          normalizedSavedRouting,
        );

        setDownloadClientRoutingByScope((previous) => {
          const currentScopeRouting = previous[scopeId] ?? {};
          if (
            areNzbgetRoutingMapsEqual(currentScopeRouting, normalizedSavedRouting)
          ) {
            return previous;
          }

          return {
            ...previous,
            [scopeId]: normalizedSavedRouting,
          };
        });
        setDownloadClientRoutingOrderByScope((previous) => {
          const currentOrder = previous[scopeId] ?? [];
          if (areRoutingOrdersEqual(currentOrder, nextOrder)) {
            return previous;
          }

          return {
            ...previous,
            [scopeId]: nextOrder,
          };
        });
        setGlobalStatus(
          successMessage ?? t("settings.downloadClientRoutingSaved"),
        );
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.failedToUpdate"),
        );
      } finally {
        setDownloadClientRoutingSaving((previous) => ({
          ...previous,
          [scopeId]: false,
        }));
      }
    },
    [client, downloadClients, setGlobalStatus, t],
  );

  const updateDownloadClientRoutingForScope = React.useCallback(
    async (
      clientId: string,
      nextValue: Partial<DownloadClientRoutingSettings>,
      options?: DownloadClientRoutingUpdateOptions,
    ) => {
      const scopeId = activeQualityScopeId;
      const currentScopeRouting = downloadClientRoutingByScope[scopeId] ?? {};
      const current =
        currentScopeRouting[clientId] ?? DOWNLOAD_CLIENT_ROUTING_EMPTY;
      const nextScopeRouting = {
        ...currentScopeRouting,
        [clientId]: {
          ...current,
          ...nextValue,
        },
      };
      const currentOrder = downloadClientRoutingOrderByScope[scopeId] ?? [];
      const nextOrder = currentOrder.includes(clientId)
        ? currentOrder
        : [...currentOrder, clientId];
      const clientName =
        downloadClients.find((client) => client.id === clientId)?.name ??
        t("label.unknown");
      const successMessage =
        options?.successMessage ??
        t("settings.downloadClientRoutingSavedFor", { name: clientName });

      setDownloadClientRoutingByScope((previous) => ({
        ...previous,
        [scopeId]: nextScopeRouting,
      }));

      setDownloadClientRoutingOrderByScope((previous) => {
        const previousOrder = previous[scopeId] ?? [];
        if (previousOrder.includes(clientId)) {
          return previous;
        }

        return {
          ...previous,
          [scopeId]: nextOrder,
        };
      });

      if (options?.save === false) {
        return;
      }

      await saveDownloadClientRoutingForScope(
        scopeId,
        nextScopeRouting,
        nextOrder,
        successMessage,
      );
    },
    [
      activeQualityScopeId,
      downloadClientRoutingByScope,
      downloadClientRoutingOrderByScope,
      downloadClients,
      saveDownloadClientRoutingForScope,
      t,
    ],
  );

  const moveDownloadClientInScope = React.useCallback(
    (clientId: string, direction: Direction) => {
      const scopeId = activeQualityScopeId;
      const scopeRouting = downloadClientRoutingByScope[scopeId] ?? {};
      const currentOrder = downloadClientRoutingOrderByScope[scopeId] ?? [];
      const index = currentOrder.indexOf(clientId);
      if (index < 0) {
        return;
      }
      const clientName =
        downloadClients.find((client) => client.id === clientId)?.name ??
        t("label.unknown");
      const successMessage = t("settings.downloadClientRoutingSavedFor", {
        name: clientName,
      });

      const nextIndex = direction === "up" ? index - 1 : index + 1;
      if (nextIndex < 0 || nextIndex >= currentOrder.length) {
        return;
      }

      const nextOrder = [...currentOrder];
      [nextOrder[index], nextOrder[nextIndex]] = [
        nextOrder[nextIndex],
        nextOrder[index],
      ];

      setDownloadClientRoutingOrderByScope((previous) => {
        const previousOrder = previous[scopeId] ?? [];
        if (areRoutingOrdersEqual(previousOrder, nextOrder)) {
          return previous;
        }

        return {
          ...previous,
          [scopeId]: nextOrder,
        };
      });

      void saveDownloadClientRoutingForScope(
        scopeId,
        scopeRouting,
        nextOrder,
        successMessage,
      );
    },
    [
      activeQualityScopeId,
      downloadClients,
      downloadClientRoutingByScope,
      downloadClientRoutingOrderByScope,
      saveDownloadClientRoutingForScope,
      t,
    ],
  );

  useSettingsSubscription(
    React.useCallback(
      (keys: string[]) => {
        if (
          keys.includes(DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY) ||
          keys.includes(LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY)
        ) {
          void refreshDownloadClientRouting();
        }
      },
      [refreshDownloadClientRouting],
    ),
  );

  return {
    downloadClients,
    activeScopeRouting,
    activeScopeRoutingOrder,
    downloadClientRoutingLoading,
    downloadClientRoutingSaving:
      downloadClientRoutingSaving[activeQualityScopeId],
    hydrateDownloadClientRouting,
    refreshDownloadClientRouting,
    updateDownloadClientRoutingForScope,
    moveDownloadClientInScope,
  };
}

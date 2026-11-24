import { userFacingGraphQlErrorMessage } from "../graphql/error-message.ts";
import { useCallback, useEffect, useRef, useState } from "react";
import type { Client } from "urql";

import {
  pluginsQuery,
  pluginInstallProgressSubscription,
} from "@/lib/graphql/queries";
import {
  beginInstallPluginMutation,
  beginUpgradePluginMutation,
  refreshPluginCatalogMutation,
  togglePluginMutation,
  uninstallPluginMutation,
} from "@/lib/graphql/mutations";
import { wsClient } from "@/lib/graphql/ws-client";
import { useBackoffPoller } from "@/lib/hooks/use-backoff-poller";
import {
  createPluginOperations,
  PLUGIN_OPERATION_RECONCILE_DELAYS_MS,
} from "@/lib/utils/plugin-install-state";
import type {
  PluginInstallProgressRecord,
  RegistryPluginRecord,
} from "@/components/views/settings/settings-plugins-section";

type TranslateFn = (
  key: string,
  values?: Record<string, string | number | boolean | null | undefined>,
) => string;

type PluginInstallProgressSubscriptionResult = {
  data?: {
    pluginInstallProgress?: PluginInstallProgressRecord;
  };
};

function extractPluginMutationErrorMessage(
  error: unknown,
  translate?: Parameters<typeof userFacingGraphQlErrorMessage>[2],
): string | null {
  return userFacingGraphQlErrorMessage(error, "", translate) || null;
}

function extractPluginMutationErrorCode(error: unknown): string | null {
  if (
    error &&
    typeof error === "object" &&
    "graphQLErrors" in error &&
    Array.isArray((error as { graphQLErrors?: unknown[] }).graphQLErrors)
  ) {
    const graphQLErrors = (
      error as {
        graphQLErrors?: Array<{ extensions?: { code?: unknown } }>;
      }
    ).graphQLErrors;
    const code = graphQLErrors?.find(
      (entry) => typeof entry.extensions?.code === "string",
    )?.extensions?.code;
    return typeof code === "string" ? code : null;
  }

  return null;
}

function formatPluginInstallError(
  plugin: RegistryPluginRecord,
  error: unknown,
  t: TranslateFn,
): string {
  const rawMessage = extractPluginMutationErrorMessage(error, t);
  const normalized = rawMessage
    ?.replace(/^\[GraphQL\]\s*/i, "")
    .replace(/^validation:\s*/i, "")
    .trim();

  if (normalized) {
    if (
      extractPluginMutationErrorCode(error) === "PLUGIN_INSTALL_IN_PROGRESS"
    ) {
      return t("status.pluginInstallAlreadyInProgress", { name: plugin.name });
    }
    return t("status.pluginInstallFailedWithReason", {
      name: plugin.name,
      reason: normalized,
    });
  }

  return t("status.failedToUpdate");
}

interface UsePluginManagementArgs {
  client: Client;
  t: TranslateFn;
  refreshProviderOptions: () => Promise<void>;
  catalogVersion?: number;
}

export function usePluginManagement({
  client,
  t,
  refreshProviderOptions,
  catalogVersion = 0,
}: UsePluginManagementArgs) {
  // ── Step 3 (fresh): Plugins ────────────────────────────────────────
  const [plugins, setPlugins] = useState<RegistryPluginRecord[]>([]);
  const [pluginsLoading, setPluginsLoading] = useState(true);
  const [pluginsRefreshing, setPluginsRefreshing] = useState(false);
  const [mutatingPluginIds, setMutatingPluginIds] = useState<string[]>([]);
  const [pluginProgress, setPluginProgress] = useState<
    Record<string, PluginInstallProgressRecord>
  >({});
  const [pluginErrors, setPluginErrors] = useState<Record<string, string>>({});
  const [pluginsError, setPluginsError] = useState<string | null>(null);
  const installProgressSubscriptionsRef = useRef(new Map<string, () => void>());
  // The install and upgrade operations this hook follows, one per plugin.
  // Their progress stream reports them; the plugin list settles any the
  // stream leaves unfinished.
  const [pluginOperations] = useState(createPluginOperations);

  const beginPluginMutation = useCallback((pluginId: string) => {
    setMutatingPluginIds((current) =>
      current.includes(pluginId) ? current : [...current, pluginId],
    );
  }, []);

  const endPluginMutation = useCallback((pluginId: string) => {
    setMutatingPluginIds((current) => current.filter((id) => id !== pluginId));
  }, []);

  const clearPluginProgress = useCallback((pluginId: string) => {
    setPluginProgress((current) => {
      if (!(pluginId in current)) {
        return current;
      }
      const next = { ...current };
      delete next[pluginId];
      return next;
    });
  }, []);

  const stopPluginInstallProgressSubscription = useCallback(
    (pluginId: string) => {
      const unsubscribe = installProgressSubscriptionsRef.current.get(pluginId);
      if (unsubscribe) {
        unsubscribe();
        installProgressSubscriptionsRef.current.delete(pluginId);
      }
    },
    [],
  );

  // End the operations a plugin list loaded after `mark` shows finished.
  const settlePluginOperations = useCallback(
    (nextPlugins: RegistryPluginRecord[], mark: number) => {
      for (const { pluginId, operation } of pluginOperations.settledBy(
        nextPlugins,
        mark,
      )) {
        if (!pluginOperations.end(pluginId, operation)) continue;
        stopPluginInstallProgressSubscription(pluginId);
        clearPluginProgress(pluginId);
        endPluginMutation(pluginId);
      }
    },
    [
      clearPluginProgress,
      endPluginMutation,
      pluginOperations,
      stopPluginInstallProgressSubscription,
    ],
  );

  const loadPlugins = useCallback(
    async (refreshIfEmpty = false) => {
      const mark = pluginOperations.mark();
      const { data, error } = await client
        .query(pluginsQuery, {}, { requestPolicy: "network-only" })
        .toPromise();
      if (error) throw error;

      const nextPlugins = (data?.plugins ?? []) as RegistryPluginRecord[];
      if (nextPlugins.length > 0 || !refreshIfEmpty) {
        setPlugins(nextPlugins);
        settlePluginOperations(nextPlugins, mark);
        return nextPlugins;
      }

      const { data: refreshData, error: refreshError } = await client
        .mutation(refreshPluginCatalogMutation, {})
        .toPromise();
      if (refreshError) throw refreshError;

      const refreshedPlugins = (refreshData?.refreshPluginCatalog ??
        []) as RegistryPluginRecord[];
      setPlugins(refreshedPlugins);
      settlePluginOperations(refreshedPlugins, mark);
      return refreshedPlugins;
    },
    [client, pluginOperations, settlePluginOperations],
  );

  // While an operation is followed, the plugin list is loaded now and then,
  // so progress ends even when its stream never connects, drops, or misses
  // the last step. Stops once nothing is followed and while the tab is hidden.
  const pollPluginOperations = useCallback(async () => {
    if (pluginOperations.size() === 0) return false;
    await loadPlugins(false);
    return pluginOperations.size() > 0;
  }, [loadPlugins, pluginOperations]);
  const pluginOperationsPoller = useBackoffPoller(
    PLUGIN_OPERATION_RECONCILE_DELAYS_MS,
    pollPluginOperations,
  );

  // `refreshProviderOptions` and `t` are called by the reload, but they must
  // not *trigger* one: callers pass them inline, so their identity changes on
  // every parent render and a keystroke in an unrelated form would otherwise
  // refetch the whole plugin list. The reload is driven by the catalog
  // version alone; these refs just keep the latest callables reachable.
  const refreshProviderOptionsRef = useRef(refreshProviderOptions);
  const translateRef = useRef(t);
  useEffect(() => {
    refreshProviderOptionsRef.current = refreshProviderOptions;
    translateRef.current = t;
  }, [refreshProviderOptions, t]);

  // Only the first load blanks the list. A catalog change, such as the one an
  // install announces, refreshes it in place so the list and its scroll
  // position stay put.
  const pluginsLoadedRef = useRef(false);
  useEffect(() => {
    void (async () => {
      if (!pluginsLoadedRef.current) setPluginsLoading(true);
      setPluginsError(null);
      try {
        await Promise.all([
          refreshProviderOptionsRef.current(),
          loadPlugins(true),
        ]);
      } catch (error) {
        setPluginsError(
          userFacingGraphQlErrorMessage(
            error,
            translateRef.current("status.failedToLoad"),
            translateRef.current,
          ),
        );
      } finally {
        pluginsLoadedRef.current = true;
        setPluginsLoading(false);
      }
    })();
  }, [catalogVersion, loadPlugins]);

  useEffect(() => {
    const subscriptions = installProgressSubscriptionsRef.current;
    return () => {
      for (const unsubscribe of subscriptions.values()) {
        unsubscribe();
      }
      subscriptions.clear();
    };
  }, []);

  const refreshPluginsRegistry = useCallback(async () => {
    setPluginsRefreshing(true);
    setPluginsError(null);
    try {
      const { data, error } = await client
        .mutation(refreshPluginCatalogMutation, {})
        .toPromise();
      if (error) throw error;

      setPlugins((data?.refreshPluginCatalog ?? []) as RegistryPluginRecord[]);
    } catch (error) {
      setPluginsError(
        userFacingGraphQlErrorMessage(error, t("status.failedToLoad"), t),
      );
    } finally {
      setPluginsRefreshing(false);
    }
  }, [client, t]);

  const beginLivePluginProgress = useCallback(
    (
      plugin: RegistryPluginRecord,
      initialSnapshot: PluginInstallProgressRecord,
    ) => {
      stopPluginInstallProgressSubscription(plugin.id);
      const operation = pluginOperations.begin(plugin.id);
      const isCurrent = () => pluginOperations.isCurrent(plugin.id, operation);
      setPluginProgress((current) => ({
        ...current,
        [plugin.id]: initialSnapshot,
      }));
      pluginOperationsPoller.restart();
      const reloadPlugins = () => {
        void Promise.all([loadPlugins(false), refreshProviderOptions()]).catch(
          (error) => {
            setPluginsError(
              userFacingGraphQlErrorMessage(error, t("status.failedToLoad"), t),
            );
          },
        );
      };
      const unsubscribe = wsClient.subscribe(
        {
          query: pluginInstallProgressSubscription,
          variables: { pluginId: plugin.id },
        },
        {
          next: (result: PluginInstallProgressSubscriptionResult) => {
            const snapshot = result.data?.pluginInstallProgress;
            if (!snapshot || !isCurrent()) {
              return;
            }
            setPluginProgress((current) => ({
              ...current,
              [plugin.id]: snapshot,
            }));

            if (snapshot.state === "SUCCEEDED" || snapshot.state === "FAILED") {
              if (!pluginOperations.end(plugin.id, operation)) {
                return;
              }
              if (pluginOperations.size() === 0) {
                pluginOperationsPoller.stop();
              }
              stopPluginInstallProgressSubscription(plugin.id);
              void (async () => {
                try {
                  if (snapshot.state === "SUCCEEDED") {
                    setPluginErrors((current) => {
                      const next = { ...current };
                      delete next[plugin.id];
                      return next;
                    });
                    await Promise.all([
                      loadPlugins(false),
                      refreshProviderOptions(),
                    ]);
                  } else {
                    setPluginErrors((current) => ({
                      ...current,
                      [plugin.id]: formatPluginInstallError(
                        plugin,
                        new Error(snapshot.error ?? snapshot.label),
                        t,
                      ),
                    }));
                  }
                } catch (error) {
                  setPluginsError(
                    userFacingGraphQlErrorMessage(
                      error,
                      t("status.failedToLoad"),
                      t,
                    ),
                  );
                } finally {
                  clearPluginProgress(plugin.id);
                  endPluginMutation(plugin.id);
                }
              })();
            }
          },
          // A stream that fails or ends before the last step (it could not
          // connect, the finished snapshot aged out before it attached, an
          // auth change closed the socket) says nothing about the operation
          // itself: the plugin list says whether it is still running.
          error: (error) => {
            if (!isCurrent()) {
              return;
            }
            console.error("[plugins] install progress stream failed:", error);
            installProgressSubscriptionsRef.current.delete(plugin.id);
            reloadPlugins();
          },
          complete: () => {
            if (!isCurrent()) {
              return;
            }
            installProgressSubscriptionsRef.current.delete(plugin.id);
            reloadPlugins();
          },
        },
      );
      if (isCurrent()) {
        installProgressSubscriptionsRef.current.set(plugin.id, unsubscribe);
      } else {
        unsubscribe();
      }
    },
    [
      clearPluginProgress,
      endPluginMutation,
      loadPlugins,
      pluginOperations,
      pluginOperationsPoller,
      refreshProviderOptions,
      stopPluginInstallProgressSubscription,
      t,
    ],
  );

  const installPlugin = useCallback(
    async (plugin: RegistryPluginRecord) => {
      beginPluginMutation(plugin.id);
      setPluginErrors((current) => {
        const next = { ...current };
        delete next[plugin.id];
        return next;
      });
      setPluginsError(null);
      try {
        const { data, error } = await client
          .mutation(beginInstallPluginMutation, { pluginId: plugin.id })
          .toPromise();
        if (error) throw error;
        const snapshot = data?.beginInstallPlugin;
        if (!snapshot) {
          throw new Error("plugin install did not return progress");
        }
        beginLivePluginProgress(plugin, snapshot);
      } catch (error) {
        setPluginErrors((current) => ({
          ...current,
          [plugin.id]: formatPluginInstallError(plugin, error, t),
        }));
        endPluginMutation(plugin.id);
      }
    },
    [
      beginLivePluginProgress,
      beginPluginMutation,
      client,
      endPluginMutation,
      t,
    ],
  );

  const uninstallPlugin = useCallback(
    async (plugin: RegistryPluginRecord) => {
      beginPluginMutation(plugin.id);
      setPluginErrors((current) => {
        const next = { ...current };
        delete next[plugin.id];
        return next;
      });
      setPluginsError(null);
      try {
        const { error } = await client
          .mutation(uninstallPluginMutation, { pluginId: plugin.id })
          .toPromise();
        if (error) throw error;

        await Promise.all([loadPlugins(false), refreshProviderOptions()]);
      } catch (error) {
        setPluginErrors((current) => ({
          ...current,
          [plugin.id]:
            extractPluginMutationErrorMessage(error) ??
            t("status.failedToDelete"),
        }));
      } finally {
        endPluginMutation(plugin.id);
      }
    },
    [
      beginPluginMutation,
      client,
      endPluginMutation,
      loadPlugins,
      refreshProviderOptions,
      t,
    ],
  );

  const togglePlugin = useCallback(
    async (plugin: RegistryPluginRecord) => {
      beginPluginMutation(plugin.id);
      setPluginErrors((current) => {
        const next = { ...current };
        delete next[plugin.id];
        return next;
      });
      setPluginsError(null);
      try {
        const { error } = await client
          .mutation(togglePluginMutation, {
            input: { pluginId: plugin.id, enabled: !plugin.isEnabled },
          })
          .toPromise();
        if (error) throw error;

        await Promise.all([loadPlugins(false), refreshProviderOptions()]);
      } catch (error) {
        setPluginErrors((current) => ({
          ...current,
          [plugin.id]:
            extractPluginMutationErrorMessage(error) ??
            t("status.failedToUpdate"),
        }));
      } finally {
        endPluginMutation(plugin.id);
      }
    },
    [
      beginPluginMutation,
      client,
      endPluginMutation,
      loadPlugins,
      refreshProviderOptions,
      t,
    ],
  );

  const upgradePlugin = useCallback(
    async (plugin: RegistryPluginRecord) => {
      beginPluginMutation(plugin.id);
      setPluginErrors((current) => {
        const next = { ...current };
        delete next[plugin.id];
        return next;
      });
      setPluginsError(null);
      try {
        const { data, error } = await client
          .mutation(beginUpgradePluginMutation, { pluginId: plugin.id })
          .toPromise();
        if (error) throw error;
        const snapshot = data?.beginUpgradePlugin;
        if (!snapshot) {
          throw new Error("plugin upgrade did not return progress");
        }
        beginLivePluginProgress(plugin, snapshot);
      } catch (error) {
        setPluginErrors((current) => ({
          ...current,
          [plugin.id]: formatPluginInstallError(plugin, error, t),
        }));
        endPluginMutation(plugin.id);
      }
    },
    [
      beginLivePluginProgress,
      beginPluginMutation,
      client,
      endPluginMutation,
      t,
    ],
  );

  return {
    plugins,
    pluginsLoading,
    pluginsRefreshing,
    mutatingPluginIds,
    pluginProgress,
    pluginErrors,
    pluginsError,
    refreshPluginsRegistry,
    installPlugin,
    uninstallPlugin,
    togglePlugin,
    upgradePlugin,
  };
}

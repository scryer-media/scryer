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
import { claimPluginTerminalOperation } from "@/lib/utils/plugin-install-state";
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

function extractPluginMutationErrorMessage(error: unknown): string | null {
  if (error && typeof error === "object" && "graphQLErrors" in error) {
    const graphQLErrors = (
      error as { graphQLErrors?: Array<{ message?: string }> }
    ).graphQLErrors;
    const message = graphQLErrors?.find(
      (entry) => typeof entry.message === "string",
    )?.message;
    if (message?.trim()) {
      return message.trim();
    }
  }

  if (error instanceof Error && error.message.trim()) {
    return error.message.trim();
  }

  return null;
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
  const rawMessage = extractPluginMutationErrorMessage(error);
  const normalized = rawMessage
    ?.replace(/^\[GraphQL\]\s*/i, "")
    .replace(/^validation:\s*/i, "")
    .trim();

  if (normalized && /WASM SHA256 mismatch/i.test(normalized)) {
    return t("status.pluginInstallFailedChecksumMismatch", {
      name: plugin.name,
    });
  }

  if (
    normalized &&
    normalized.includes("has sdk_constraint") &&
    normalized.includes("but registry selected")
  ) {
    return t("status.pluginInstallFailedSdkMetadataMismatch", {
      name: plugin.name,
    });
  }

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
  // Whichever of `next`, `error` and `complete` reaches a plugin first owns
  // winding its operation down; the others must not undo that work.
  const terminalPluginOperationsRef = useRef(new Set<string>());

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

  const loadPlugins = useCallback(
    async (refreshIfEmpty = false) => {
      const { data, error } = await client.query(pluginsQuery, {}).toPromise();
      if (error) throw error;

      const nextPlugins = (data?.plugins ?? []) as RegistryPluginRecord[];
      if (nextPlugins.length > 0 || !refreshIfEmpty) {
        setPlugins(nextPlugins);
        return nextPlugins;
      }

      const { data: refreshData, error: refreshError } = await client
        .mutation(refreshPluginCatalogMutation, {})
        .toPromise();
      if (refreshError) throw refreshError;

      const refreshedPlugins = (refreshData?.refreshPluginCatalog ??
        []) as RegistryPluginRecord[];
      setPlugins(refreshedPlugins);
      return refreshedPlugins;
    },
    [client],
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

  useEffect(() => {
    void (async () => {
      setPluginsLoading(true);
      setPluginsError(null);
      try {
        await Promise.all([
          refreshProviderOptionsRef.current(),
          loadPlugins(true),
        ]);
      } catch (error) {
        setPluginsError(
          error instanceof Error
            ? error.message
            : translateRef.current("status.failedToLoad"),
        );
      } finally {
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
        error instanceof Error ? error.message : t("status.failedToLoad"),
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
      terminalPluginOperationsRef.current.delete(plugin.id);
      setPluginProgress((current) => ({
        ...current,
        [plugin.id]: initialSnapshot,
      }));
      const unsubscribe = wsClient.subscribe(
        {
          query: pluginInstallProgressSubscription,
          variables: { pluginId: plugin.id },
        },
        {
          next: (result: PluginInstallProgressSubscriptionResult) => {
            const snapshot = result.data?.pluginInstallProgress;
            if (!snapshot) {
              return;
            }
            setPluginProgress((current) => ({
              ...current,
              [plugin.id]: snapshot,
            }));

            if (snapshot.state === "SUCCEEDED" || snapshot.state === "FAILED") {
              if (
                !claimPluginTerminalOperation(
                  terminalPluginOperationsRef.current,
                  plugin.id,
                )
              ) {
                return;
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
                    error instanceof Error
                      ? error.message
                      : t("status.failedToLoad"),
                  );
                } finally {
                  clearPluginProgress(plugin.id);
                  endPluginMutation(plugin.id);
                }
              })();
            }
          },
          error: (error) => {
            if (
              !claimPluginTerminalOperation(
                terminalPluginOperationsRef.current,
                plugin.id,
              )
            ) {
              return;
            }
            stopPluginInstallProgressSubscription(plugin.id);
            clearPluginProgress(plugin.id);
            endPluginMutation(plugin.id);
            setPluginErrors((current) => ({
              ...current,
              [plugin.id]: formatPluginInstallError(plugin, error, t),
            }));
          },
          complete: () => {
            installProgressSubscriptionsRef.current.delete(plugin.id);
            // A stream that ends without a terminal snapshot — the finished
            // snapshot aged out before this connection attached, a reconnect
            // resubscribed too late, an auth epoch bumped the socket — used to
            // leave the row pinned on "Installing" with nothing left to correct
            // it. Reconcile the way reloading the page did.
            if (
              !claimPluginTerminalOperation(
                terminalPluginOperationsRef.current,
                plugin.id,
              )
            ) {
              return;
            }
            clearPluginProgress(plugin.id);
            endPluginMutation(plugin.id);
            void loadPlugins(false).catch(() => undefined);
          },
        },
      );
      installProgressSubscriptionsRef.current.set(plugin.id, unsubscribe);
    },
    [
      clearPluginProgress,
      endPluginMutation,
      loadPlugins,
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

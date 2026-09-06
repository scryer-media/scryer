import { useCallback, useEffect, useRef, useState } from "react";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { Checkbox } from "@/components/ui/checkbox";
import {
  SettingsPluginsSection,
  type PluginInstallProgressRecord,
  type RegistryPluginRecord,
} from "@/components/views/settings/settings-plugins-section";
import { SETTINGS_HEADER_ACTIONS_SLOT_ID } from "@/components/containers/settings/settings-container";
import { useClient } from "urql";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { dispatchNavigationBadgesRefresh } from "@/lib/events/navigation-badges";
import {
  pluginsQuery,
  pluginAutoUpdateSettingsQuery,
  pluginInstallProgressSubscription,
} from "@/lib/graphql/queries";
import {
  beginInstallPluginMutation,
  beginUpgradePluginMutation,
  refreshPluginCatalogMutation,
  inspectManualPluginRepoMutation,
  installManualPluginMutation,
  installUploadedPluginMutation,
  uninstallPluginMutation,
  togglePluginMutation,
  updatePluginAutoUpdateSettingsMutation,
} from "@/lib/graphql/mutations";
import type { PluginAutoUpdateSettings } from "@/lib/types/settings";
import { useProviderCatalogSubscription } from "@/lib/hooks/use-provider-catalog-subscription";
import { wsClient } from "@/lib/graphql/ws-client";
import {
  applySuccessfulPluginOperationState,
  claimPluginTerminalOperation,
} from "@/lib/utils/plugin-install-state";

type PluginCatalogStatusRecord = {
  refreshState: 'READY' | 'DEGRADED';
  githubAvailable: boolean;
  lastCheckedAt?: string | null;
  outageMessage?: string | null;
  blockedActions: string[];
  restoreWarnings: string[];
  lastError?: string | null;
};

type ManualPluginPreviewRecord = {
  githubRepoUrl: string;
  plugin: RegistryPluginRecord;
};

type PluginInstallProgressSubscriptionResult = {
  data?: {
    pluginInstallProgress?: PluginInstallProgressRecord;
  };
};

type PluginAutoUpdateSettingsQueryResult = {
  pluginAutoUpdateSettings?: PluginAutoUpdateSettings | null;
};

type UpdatePluginAutoUpdateSettingsResult = {
  updatePluginAutoUpdateSettings?: PluginAutoUpdateSettings | null;
};

function extractPluginMutationErrorMessage(error: unknown): string | null {
  if (error && typeof error === "object" && "graphQLErrors" in error) {
    const graphQLErrors = (error as { graphQLErrors?: Array<{ message?: string }> }).graphQLErrors;
    const message = graphQLErrors?.find((entry) => typeof entry.message === "string")?.message;
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
    error
    && typeof error === "object"
    && "graphQLErrors" in error
    && Array.isArray((error as { graphQLErrors?: unknown[] }).graphQLErrors)
  ) {
    const graphQLErrors = (error as {
      graphQLErrors?: Array<{ extensions?: { code?: unknown } }>;
    }).graphQLErrors;
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
  t: ReturnType<typeof useTranslate>,
): string {
  const rawMessage = extractPluginMutationErrorMessage(error);
  const normalized = rawMessage
    ?.replace(/^\[GraphQL\]\s*/i, "")
    .replace(/^validation:\s*/i, "")
    .trim();

  if (normalized && /WASM SHA256 mismatch/i.test(normalized)) {
    return t("status.pluginInstallFailedChecksumMismatch", { name: plugin.name });
  }

  if (
    normalized
    && normalized.includes("has sdk_constraint")
    && normalized.includes("but registry selected")
  ) {
    return t("status.pluginInstallFailedSdkMetadataMismatch", { name: plugin.name });
  }

  if (normalized) {
    if (extractPluginMutationErrorCode(error) === "PLUGIN_INSTALL_IN_PROGRESS") {
      return t("status.pluginInstallAlreadyInProgress", { name: plugin.name });
    }
    return t("status.pluginInstallFailedWithReason", {
      name: plugin.name,
      reason: normalized,
    });
  }

  return t("status.failedToUpdate");
}

function manualPluginFileIsSupported(fileName: string): boolean {
  const normalized = fileName.trim().toLowerCase();
  return normalized.endsWith(".wasm") || normalized.endsWith(".wasm.zst");
}

function encodeBytesBase64(bytes: Uint8Array): string {
  let binary = "";
  const chunkSize = 0x8000;
  for (let index = 0; index < bytes.length; index += chunkSize) {
    binary += String.fromCharCode(...bytes.subarray(index, index + chunkSize));
  }
  return btoa(binary);
}

async function readFileAsBase64(file: File): Promise<string> {
  const buffer = await file.arrayBuffer();
  return encodeBytesBase64(new Uint8Array(buffer));
}

export function SettingsPluginsContainer() {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const [plugins, _setPlugins] = useState<RegistryPluginRecord[]>([]);
  const [catalogStatus, setCatalogStatus] = useState<PluginCatalogStatusRecord | null>(null);
  const [initialLoading, setInitialLoading] = useState(true);
  const [pluginErrors, setPluginErrors] = useState<Record<string, string>>({});
  const [manualRepoUrl, setManualRepoUrl] = useState("");
  const [manualPreview, setManualPreview] = useState<ManualPluginPreviewRecord | null>(null);
  const [manualBusy, setManualBusy] = useState(false);
  const [manualPluginFile, setManualPluginFile] = useState<File | null>(null);
  const [pendingManualUpload, setPendingManualUpload] = useState(false);
  const [manualUploadRiskAccepted, setManualUploadRiskAccepted] = useState(false);
  const [showManualInstall, setShowManualInstall] = useState(false);
  const [headerActionsTarget, setHeaderActionsTarget] = useState<HTMLElement | null>(null);
  const [autoUpdateEnabled, setAutoUpdateEnabled] = useState(false);
  const [autoUpdateLoading, setAutoUpdateLoading] = useState(true);
  const [autoUpdateSaving, setAutoUpdateSaving] = useState(false);

  const setPlugins = useCallback((
    next:
      | RegistryPluginRecord[]
      | ((current: RegistryPluginRecord[]) => RegistryPluginRecord[]),
  ) => {
    _setPlugins(next);
  }, []);
  const [mutatingPluginIds, setMutatingPluginIds] = useState<string[]>([]);
  const [pluginProgress, setPluginProgress] = useState<Record<string, PluginInstallProgressRecord>>({});
  const [refreshing, setRefreshing] = useState(false);
  const [upgradingAll, setUpgradingAll] = useState(false);
  const [pendingUninstall, setPendingUninstall] = useState<RegistryPluginRecord | null>(null);
  const installProgressSubscriptionsRef = useRef(new Map<string, () => void>());
  const terminalPluginOperationsRef = useRef(new Set<string>());
  const pluginProgressRef = useRef<Record<string, PluginInstallProgressRecord>>({});
  const pluginsRefreshPromiseRef = useRef<Promise<void> | null>(null);
  const lastPluginsRefreshCompletedAtRef = useRef(0);

  useEffect(() => {
    setHeaderActionsTarget(document.getElementById(SETTINGS_HEADER_ACTIONS_SLOT_ID));
  }, []);

  useEffect(() => {
    pluginProgressRef.current = pluginProgress;
  }, [pluginProgress]);

  const beginPluginMutation = useCallback((pluginId: string) => {
    setMutatingPluginIds((current) => (
      current.includes(pluginId) ? current : [...current, pluginId]
    ));
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

  const stopPluginInstallProgressSubscription = useCallback((pluginId: string) => {
    const unsubscribe = installProgressSubscriptionsRef.current.get(pluginId);
    if (unsubscribe) {
      unsubscribe();
      installProgressSubscriptionsRef.current.delete(pluginId);
    }
  }, []);

  const applySuccessfulPluginOperation = useCallback(
    (pluginId: string, operationKind: PluginInstallProgressRecord["operationKind"]) => {
      setPlugins((current) => current.map((plugin) => (
        plugin.id === pluginId
          ? applySuccessfulPluginOperationState(plugin, operationKind)
          : plugin
      )));
    },
    [setPlugins],
  );

  const clearPluginBusyState = useCallback((pluginId: string) => {
    setPlugins((current) => current.map((plugin) => (
      plugin.id === pluginId ? { ...plugin, installInProgress: false } : plugin
    )));
  }, [setPlugins]);

  const reconcilePluginOperationState = useCallback((nextPlugins: RegistryPluginRecord[]) => {
    const nextById = new Map(nextPlugins.map((plugin) => [plugin.id, plugin] as const));
    const trackedPluginIds = new Set([
      ...installProgressSubscriptionsRef.current.keys(),
      ...Object.keys(pluginProgressRef.current),
    ]);

    for (const pluginId of trackedPluginIds) {
      const nextPlugin = nextById.get(pluginId);
      if (nextPlugin?.installInProgress) {
        continue;
      }

      stopPluginInstallProgressSubscription(pluginId);
      clearPluginProgress(pluginId);
      endPluginMutation(pluginId);
      setPluginErrors((current) => {
        if (!(pluginId in current)) {
          return current;
        }
        const next = { ...current };
        delete next[pluginId];
        return next;
      });
    }
  }, [clearPluginProgress, endPluginMutation, stopPluginInstallProgressSubscription]);

  useEffect(() => {
    const subscriptions = installProgressSubscriptionsRef.current;
    return () => {
      for (const unsubscribe of subscriptions.values()) {
        unsubscribe();
      }
      subscriptions.clear();
    };
  }, []);

  const refreshPlugins = useCallback(async (reportErrors = true) => {
    if (pluginsRefreshPromiseRef.current) {
      return pluginsRefreshPromiseRef.current;
    }
    const refreshPromise = (async () => {
      try {
        const { data, error } = await client.query(pluginsQuery, {}).toPromise();
        if (error) throw error;
        const nextPlugins = data.plugins || [];
        setPlugins(nextPlugins);
        setCatalogStatus(data.pluginCatalogStatus || null);
        reconcilePluginOperationState(nextPlugins);
      } catch (error) {
        if (reportErrors) {
          setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
        }
      } finally {
        setInitialLoading(false);
        lastPluginsRefreshCompletedAtRef.current = Date.now();
        pluginsRefreshPromiseRef.current = null;
      }
    })();
    pluginsRefreshPromiseRef.current = refreshPromise;
    return refreshPromise;
  }, [client, reconcilePluginOperationState, setGlobalStatus, t, setPlugins]);

  useEffect(() => {
    void refreshPlugins();
  }, [refreshPlugins]);

  const fetchAutoUpdateSettings = useCallback(async () => {
    setAutoUpdateLoading(true);
    try {
      const { data, error } = await client
        .query<PluginAutoUpdateSettingsQueryResult>(pluginAutoUpdateSettingsQuery, {})
        .toPromise();
      if (error) throw error;
      setAutoUpdateEnabled(data?.pluginAutoUpdateSettings?.enabled ?? false);
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
    } finally {
      setAutoUpdateLoading(false);
    }
  }, [client, setGlobalStatus, t]);

  useEffect(() => {
    void fetchAutoUpdateSettings();
  }, [fetchAutoUpdateSettings]);

  const updateAutoUpdateEnabled = useCallback(async (enabled: boolean) => {
    setAutoUpdateSaving(true);
    try {
      const { data, error } = await client
        .mutation<UpdatePluginAutoUpdateSettingsResult>(
          updatePluginAutoUpdateSettingsMutation,
          { input: { enabled } },
        )
        .toPromise();
      if (error) throw error;
      setAutoUpdateEnabled(data?.updatePluginAutoUpdateSettings?.enabled ?? enabled);
      setGlobalStatus(t("status.pluginAutoUpdateSettingsSaved"));
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setAutoUpdateSaving(false);
    }
  }, [client, setGlobalStatus, t]);

  useProviderCatalogSubscription(() => {
    if (
      pluginsRefreshPromiseRef.current
      || Date.now() - lastPluginsRefreshCompletedAtRef.current < 2_000
    ) {
      return;
    }
    void refreshPlugins();
    dispatchNavigationBadgesRefresh();
  });

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
              if (!claimPluginTerminalOperation(terminalPluginOperationsRef.current, plugin.id)) {
                return;
              }
              stopPluginInstallProgressSubscription(plugin.id);
              void (async () => {
                try {
                  if (snapshot.state === "SUCCEEDED") {
                    applySuccessfulPluginOperation(plugin.id, snapshot.operationKind);
                    setPluginErrors((current) => {
                      const next = { ...current };
                      delete next[plugin.id];
                      return next;
                    });
                    setGlobalStatus(
                      snapshot.operationKind === "UPGRADE"
                        ? t("status.pluginUpgraded", {
                          name: plugin.name,
                          version: plugin.latestVersion ?? plugin.version,
                        })
                        : t("status.pluginInstalled", { name: plugin.name }),
                    );
                    await refreshPlugins(false);
                    dispatchNavigationBadgesRefresh();
                  } else {
                    clearPluginBusyState(plugin.id);
                    const message = formatPluginInstallError(
                      plugin,
                      new Error(snapshot.error ?? snapshot.label),
                      t,
                    );
                    setPluginErrors((current) => ({
                      ...current,
                      [plugin.id]: message,
                    }));
                    setGlobalStatus(message);
                  }
                } finally {
                  clearPluginProgress(plugin.id);
                  endPluginMutation(plugin.id);
                }
              })();
            }
          },
          error: (error) => {
            if (!claimPluginTerminalOperation(terminalPluginOperationsRef.current, plugin.id)) {
              return;
            }
            stopPluginInstallProgressSubscription(plugin.id);
            clearPluginBusyState(plugin.id);
            clearPluginProgress(plugin.id);
            endPluginMutation(plugin.id);
            const message = formatPluginInstallError(plugin, error, t);
            setPluginErrors((current) => ({
              ...current,
              [plugin.id]: message,
            }));
            setGlobalStatus(message);
          },
          complete: () => {
            installProgressSubscriptionsRef.current.delete(plugin.id);
            // A stream that ends without a terminal snapshot — the finished
            // snapshot aged out before this connection attached, a reconnect
            // resubscribed too late, an auth epoch bumped the socket — used to
            // leave the row pinned on "Installing" with nothing left to correct
            // it. Reconcile the way reloading the page did.
            if (
              claimPluginTerminalOperation(terminalPluginOperationsRef.current, plugin.id)
            ) {
              clearPluginBusyState(plugin.id);
              clearPluginProgress(plugin.id);
              endPluginMutation(plugin.id);
            }
            void refreshPlugins();
          },
        },
      );
      installProgressSubscriptionsRef.current.set(plugin.id, unsubscribe);
    },
    [
      applySuccessfulPluginOperation,
      clearPluginBusyState,
      clearPluginProgress,
      endPluginMutation,
      refreshPlugins,
      setGlobalStatus,
      stopPluginInstallProgressSubscription,
      t,
    ],
  );

  const refreshRegistry = async () => {
    setRefreshing(true);
    try {
      const { data, error } = await client
        .mutation(refreshPluginCatalogMutation, {})
        .toPromise();
      if (error) throw error;
      setPlugins(data.refreshPluginCatalog || []);
      dispatchNavigationBadgesRefresh();
      setGlobalStatus(t("status.catalogRefreshed"));
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
    } finally {
      setRefreshing(false);
    }
  };

  const togglePlugin = useCallback(
    async (plugin: RegistryPluginRecord) => {
      beginPluginMutation(plugin.id);
      try {
        const { error } = await client
          .mutation(togglePluginMutation, {
            input: { pluginId: plugin.id, enabled: !plugin.isEnabled },
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(
          t("status.pluginToggled", {
            name: plugin.name,
            state: plugin.isEnabled ? t("label.disabled") : t("label.enabled"),
          }),
        );
        await refreshPlugins();
        dispatchNavigationBadgesRefresh();
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
      } finally {
        endPluginMutation(plugin.id);
      }
    },
    [beginPluginMutation, client, endPluginMutation, refreshPlugins, setGlobalStatus, t],
  );

  const installPlugin = async (plugin: RegistryPluginRecord) => {
    beginPluginMutation(plugin.id);
    setPluginErrors((current) => {
      const next = { ...current };
      delete next[plugin.id];
      return next;
    });
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
      const message = formatPluginInstallError(plugin, error, t);
      setPluginErrors((current) => ({
        ...current,
        [plugin.id]: message,
      }));
      setGlobalStatus(message);
      endPluginMutation(plugin.id);
    }
  };

  const uninstallPlugin = (plugin: RegistryPluginRecord) => {
    setPendingUninstall(plugin);
  };

  const onManualPluginFileChange = useCallback((event: React.ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0] ?? null;
    event.target.value = "";
    setManualUploadRiskAccepted(false);
    setPendingManualUpload(false);
    setPluginErrors((current) => {
      const next = { ...current };
      delete next.__manualUpload;
      return next;
    });

    if (!file) {
      setManualPluginFile(null);
      return;
    }

    if (!manualPluginFileIsSupported(file.name)) {
      setManualPluginFile(null);
      const message = t("settings.pluginManualFileUnsupported");
      setPluginErrors((current) => ({ ...current, __manualUpload: message }));
      setGlobalStatus(message);
      return;
    }

    setManualPluginFile(file);
  }, [setGlobalStatus, t]);

  const inspectManualPluginRepo = async () => {
    if (!manualRepoUrl.trim()) return;
    setManualBusy(true);
    setPluginErrors((current) => {
      const next = { ...current };
      delete next.__manual;
      return next;
    });
    try {
      const { data, error } = await client
        .mutation(inspectManualPluginRepoMutation, {
          input: { githubRepoUrl: manualRepoUrl.trim() },
        })
        .toPromise();
      if (error) throw error;
      setManualPreview(data.inspectManualPluginRepo || null);
    } catch (error) {
      const message = error instanceof Error ? error.message : t("status.failedToLoad");
      setPluginErrors((current) => ({ ...current, __manual: message }));
      setGlobalStatus(message);
    } finally {
      setManualBusy(false);
    }
  };

  const requestInstallUploadedPlugin = useCallback(() => {
    if (!manualPluginFile) {
      return;
    }
    setManualUploadRiskAccepted(false);
    setPendingManualUpload(true);
    setPluginErrors((current) => {
      const next = { ...current };
      delete next.__manualUpload;
      return next;
    });
  }, [manualPluginFile]);

  const installUploadedPlugin = useCallback(async () => {
    if (!manualPluginFile) {
      return;
    }
    setManualBusy(true);
    try {
      const wasmBase64 = await readFileAsBase64(manualPluginFile);
      const { data, error } = await client
        .mutation(installUploadedPluginMutation, {
          input: {
            fileName: manualPluginFile.name,
            wasmBase64,
            acknowledgeRisk: true,
          },
        })
        .toPromise();
      if (error) throw error;
      const installation = data?.installUploadedPlugin;
      if (!installation) {
        throw new Error("manual plugin upload did not return an installation");
      }

      setGlobalStatus(t("status.pluginInstalled", { name: installation.name }));
      setManualPluginFile(null);
      setManualPreview(null);
      setManualRepoUrl("");
      setPendingManualUpload(false);
      setManualUploadRiskAccepted(false);
      setShowManualInstall(false);
      setPluginErrors((current) => {
        const next = { ...current };
        delete next.__manualUpload;
        return next;
      });
      await refreshPlugins();
      dispatchNavigationBadgesRefresh();
    } catch (error) {
      const message = extractPluginMutationErrorMessage(error) ?? t("status.failedToUpdate");
      setPluginErrors((current) => ({ ...current, __manualUpload: message }));
      setGlobalStatus(message);
    } finally {
      setManualBusy(false);
    }
  }, [client, manualPluginFile, refreshPlugins, setGlobalStatus, t]);

  const installManualPlugin = async () => {
    const preview = manualPreview;
    if (!preview) return;
    beginPluginMutation(preview.plugin.id);
    setManualBusy(true);
    try {
      const { error } = await client
        .mutation(installManualPluginMutation, {
          input: { githubRepoUrl: preview.githubRepoUrl },
        })
        .toPromise();
      if (error) throw error;
      setGlobalStatus(t("status.pluginInstalled", { name: preview.plugin.name }));
      setManualPreview(null);
      setManualRepoUrl("");
      setShowManualInstall(false);
      await refreshPlugins();
      dispatchNavigationBadgesRefresh();
    } catch (error) {
      const message = formatPluginInstallError(preview.plugin, error, t);
      setPluginErrors((current) => ({ ...current, __manual: message }));
      setGlobalStatus(message);
    } finally {
      setManualBusy(false);
      endPluginMutation(preview.plugin.id);
    }
  };

  const beginPluginUpgrade = async (plugin: RegistryPluginRecord) => {
    beginPluginMutation(plugin.id);
    setPluginErrors((current) => {
      const next = { ...current };
      delete next[plugin.id];
      return next;
    });
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
      return true;
    } catch (error) {
      const message = formatPluginInstallError(plugin, error, t);
      setPluginErrors((current) => ({
        ...current,
        [plugin.id]: message,
      }));
      setGlobalStatus(message);
      endPluginMutation(plugin.id);
      return false;
    } finally {
      // Progress lifecycle owns cleanup after a successful begin.
    }
  };

  const upgradePlugin = async (plugin: RegistryPluginRecord) => {
    await beginPluginUpgrade(plugin);
  };

  const upgradeAllPlugins = async () => {
    const upgradable = plugins.filter(
      (plugin) => plugin.isInstalled && plugin.updateAvailable && !mutatingPluginIds.includes(plugin.id),
    );
    if (upgradable.length === 0) {
      return;
    }

    setUpgradingAll(true);
    try {
      const results = await Promise.all(upgradable.map((plugin) => beginPluginUpgrade(plugin)));
      const startedCount = results.filter(Boolean).length;
      if (startedCount > 0) {
        setGlobalStatus(
          t("status.pluginsUpgradeQueued", {
            count: startedCount,
          }),
        );
      }
    } finally {
      setUpgradingAll(false);
    }
  };

  const confirmUninstall = async () => {
    if (!pendingUninstall) return;
    const plugin = pendingUninstall;
    const isBuiltinOverride = plugin.builtin && plugin.sourceKind === "downloaded";
    beginPluginMutation(plugin.id);
    try {
      const { error } = await client
        .mutation(uninstallPluginMutation, { pluginId: plugin.id })
        .toPromise();
      if (error) throw error;
      setGlobalStatus(
        isBuiltinOverride
          ? t("status.pluginRevertedToBundled", { name: plugin.name })
          : t("status.pluginUninstalled", { name: plugin.name }),
      );
      await refreshPlugins();
      dispatchNavigationBadgesRefresh();
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToDelete"));
    } finally {
      endPluginMutation(plugin.id);
      setPendingUninstall(null);
    }
  };

  const pendingUninstallIsBuiltinOverride =
    pendingUninstall?.builtin && pendingUninstall.sourceKind === "downloaded";
  const blockedRemoteActions = new Set(catalogStatus?.blockedActions ?? []);

  return (
    <>
      <SettingsPluginsSection
        plugins={plugins}
        catalogStatus={catalogStatus}
        initialLoading={initialLoading}
        mutatingPluginIds={mutatingPluginIds}
        pluginProgress={pluginProgress}
        pluginErrors={pluginErrors}
        refreshing={refreshing}
        upgradingAll={upgradingAll}
        manualRepoUrl={manualRepoUrl}
        manualFileName={manualPluginFile?.name ?? null}
        manualPreview={manualPreview}
        manualBusy={manualBusy}
        showManualInstall={showManualInstall}
        headerActionsTarget={headerActionsTarget}
        autoUpdateEnabled={autoUpdateEnabled}
        autoUpdateLoading={autoUpdateLoading}
        autoUpdateSaving={autoUpdateSaving}
        onAutoUpdateEnabledChange={updateAutoUpdateEnabled}
        remoteActionsBlocked={{
          refresh: blockedRemoteActions.has("catalog_refresh"),
          install: blockedRemoteActions.has("install"),
          installManual: blockedRemoteActions.has("install_manual"),
          upgrade: blockedRemoteActions.has("upgrade"),
          inspectManual: blockedRemoteActions.has("manual_repo_inspection"),
        }}
        onManualRepoUrlChange={setManualRepoUrl}
        onToggleManualInstall={() => setShowManualInstall((current) => !current)}
        onManualPluginFileChange={onManualPluginFileChange}
        onInspectManualPluginRepo={inspectManualPluginRepo}
        onRequestInstallUploadedPlugin={requestInstallUploadedPlugin}
        onInstallManualPlugin={installManualPlugin}
        onRefreshRegistry={refreshRegistry}
        onUpgradeAllPlugins={upgradeAllPlugins}
        onTogglePlugin={togglePlugin}
        onInstallPlugin={installPlugin}
        onUninstallPlugin={uninstallPlugin}
        onUpgradePlugin={upgradePlugin}
      />
      <ConfirmDialog
        open={pendingUninstall !== null}
        title={
          pendingUninstallIsBuiltinOverride
            ? t("settings.pluginRevertToBundled")
            : t("settings.pluginUninstall")
        }
        description={
          pendingUninstall
            ? pendingUninstallIsBuiltinOverride
              ? t("settings.pluginRevertToBundledWarning", { name: pendingUninstall.name })
              : t("settings.pluginUninstallWarning", { name: pendingUninstall.name })
            : ""
        }
        confirmLabel={
          pendingUninstallIsBuiltinOverride
            ? t("settings.pluginRevert")
            : t("settings.pluginUninstall")
        }
        cancelLabel={t("label.cancel")}
        isBusy={pendingUninstall ? mutatingPluginIds.includes(pendingUninstall.id) : false}
        onConfirm={confirmUninstall}
        onCancel={() => setPendingUninstall(null)}
      />
      <ConfirmDialog
        open={pendingManualUpload}
        title={t("settings.pluginManualUploadConfirmTitle")}
        description={t("settings.pluginManualUploadConfirmDescription")}
        confirmLabel={t("settings.pluginManualUploadConfirmAction")}
        cancelLabel={t("label.cancel")}
        contentId="settings-plugins-manual-upload-confirm"
        confirmButtonId="settings-plugins-manual-upload-confirm-action"
        cancelButtonId="settings-plugins-manual-upload-confirm-cancel"
        isBusy={manualBusy}
        confirmDisabled={!manualUploadRiskAccepted}
        onConfirm={installUploadedPlugin}
        onCancel={() => {
          setPendingManualUpload(false);
          setManualUploadRiskAccepted(false);
        }}
      >
        <label
          htmlFor="settings-plugins-manual-upload-risk-checkbox"
          className="flex cursor-pointer items-start gap-3 text-sm"
        >
          <Checkbox
            id="settings-plugins-manual-upload-risk-checkbox"
            checked={manualUploadRiskAccepted}
            onCheckedChange={(checked) => setManualUploadRiskAccepted(!!checked)}
          />
          <span>{t("settings.pluginManualUploadConfirmCheckbox")}</span>
        </label>
      </ConfirmDialog>
    </>
  );
}

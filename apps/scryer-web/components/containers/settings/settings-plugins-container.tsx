import { useCallback, useEffect, useState } from "react";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import {
  SettingsPluginsSection,
  type RegistryPluginRecord,
} from "@/components/views/settings/settings-plugins-section";
import { useClient } from "urql";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { pluginsQuery } from "@/lib/graphql/queries";
import {
  refreshPluginRegistryMutation,
  installPluginMutation,
  uninstallPluginMutation,
  togglePluginMutation,
  upgradePluginMutation,
} from "@/lib/graphql/mutations";

export function SettingsPluginsContainer() {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const [plugins, _setPlugins] = useState<RegistryPluginRecord[]>([]);

  const setPlugins = useCallback((next: RegistryPluginRecord[]) => {
    _setPlugins(next);
    const upgradeCount = next.filter((p) => p.isInstalled && p.updateAvailable).length;
    window.dispatchEvent(
      new CustomEvent("scryer:pluginUpgradeCount", { detail: upgradeCount }),
    );
  }, []);
  const [mutatingPluginId, setMutatingPluginId] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [pendingUninstall, setPendingUninstall] = useState<RegistryPluginRecord | null>(null);

  const refreshPlugins = useCallback(async () => {
    try {
      const { data, error } = await client.query(pluginsQuery, {}).toPromise();
      if (error) throw error;
      setPlugins(data.plugins || []);
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
    }
  }, [client, setGlobalStatus, t, setPlugins]);

  useEffect(() => {
    void refreshPlugins();
  }, [refreshPlugins]);

  const refreshRegistry = async () => {
    setRefreshing(true);
    try {
      const { data, error } = await client
        .mutation(refreshPluginRegistryMutation, {})
        .toPromise();
      if (error) throw error;
      setPlugins(data.refreshPluginRegistry || []);
      setGlobalStatus(t("status.registryRefreshed"));
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
    } finally {
      setRefreshing(false);
    }
  };

  const togglePlugin = useCallback(
    async (plugin: RegistryPluginRecord) => {
      setMutatingPluginId(plugin.id);
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
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
      } finally {
        setMutatingPluginId(null);
      }
    },
    [client, refreshPlugins, setGlobalStatus, t],
  );

  const installPlugin = async (plugin: RegistryPluginRecord) => {
    setMutatingPluginId(plugin.id);
    try {
      const { error } = await client
        .mutation(installPluginMutation, {
          input: { pluginId: plugin.id },
        })
        .toPromise();
      if (error) throw error;
      setGlobalStatus(t("status.pluginInstalled", { name: plugin.name }));
      await refreshPlugins();
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setMutatingPluginId(null);
    }
  };

  const uninstallPlugin = (plugin: RegistryPluginRecord) => {
    setPendingUninstall(plugin);
  };

  const upgradePlugin = async (plugin: RegistryPluginRecord) => {
    setMutatingPluginId(plugin.id);
    try {
      const { error } = await client
        .mutation(upgradePluginMutation, {
          input: { pluginId: plugin.id },
        })
        .toPromise();
      if (error) throw error;
      setGlobalStatus(t("status.pluginUpgraded", { name: plugin.name, version: plugin.version }));
      await refreshPlugins();
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setMutatingPluginId(null);
    }
  };

  const confirmUninstall = async () => {
    if (!pendingUninstall) return;
    const plugin = pendingUninstall;
    setMutatingPluginId(plugin.id);
    try {
      const { error } = await client
        .mutation(uninstallPluginMutation, {
          input: { pluginId: plugin.id },
        })
        .toPromise();
      if (error) throw error;
      setGlobalStatus(t("status.pluginUninstalled", { name: plugin.name }));
      await refreshPlugins();
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToDelete"));
    } finally {
      setMutatingPluginId(null);
      setPendingUninstall(null);
    }
  };

  return (
    <>
      <SettingsPluginsSection
        plugins={plugins}
        mutatingPluginId={mutatingPluginId}
        refreshing={refreshing}
        onRefreshRegistry={refreshRegistry}
        onTogglePlugin={togglePlugin}
        onInstallPlugin={installPlugin}
        onUninstallPlugin={uninstallPlugin}
        onUpgradePlugin={upgradePlugin}
      />
      <ConfirmDialog
        open={pendingUninstall !== null}
        title={t("settings.pluginUninstall")}
        description={
          pendingUninstall
            ? t("settings.pluginUninstallWarning", { name: pendingUninstall.name })
            : ""
        }
        confirmLabel={t("settings.pluginUninstall")}
        cancelLabel={t("label.cancel")}
        isBusy={mutatingPluginId !== null}
        onConfirm={confirmUninstall}
        onCancel={() => setPendingUninstall(null)}
      />
    </>
  );
}

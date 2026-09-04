import * as React from "react";
import { useClient } from "urql";
import {
  ArrowUpCircle,
  Download,
  Loader2,
  Power,
  PowerOff,
  RefreshCw,
  Trash2,
} from "lucide-react";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { PluginLogo } from "@/components/common/plugin-visual";
import type { ProviderCatalogFamily } from "@/lib/hooks/use-provider-catalog-subscription";
import { usePluginManagement } from "@/lib/hooks/use-plugin-management";
import {
  isRunningPluginProgress,
  PluginInstallProgressBar,
  type RegistryPluginRecord,
} from "@/components/views/settings/settings-plugins-section";
import { useTranslate } from "@/lib/context/translate-context";
import { Badge } from "@/components/ui/badge";
import { IconButton } from "@/components/ui/icon-button";
import { cn } from "@/lib/utils";

const FILTERED_PLUGIN_PANEL_CLASS =
  "flex min-h-0 flex-col overflow-hidden rounded-[14px] border border-[var(--scry-border)] bg-[var(--scry-surf)] shadow-[0_10px_24px_rgba(0,0,0,0.16)]";
const FILTERED_PLUGIN_HEADER_CLASS =
  "flex flex-row items-center justify-between gap-2 border-b border-[var(--scry-border3)] bg-[linear-gradient(180deg,rgba(255,255,255,0.035),rgba(255,255,255,0))] px-4 py-3";
const FILTERED_PLUGIN_TITLE_CLASS =
  "text-[15px] font-semibold text-[var(--scry-ink2)]";
const FILTERED_PLUGIN_BODY_CLASS =
  "grid grid-cols-[repeat(auto-fit,minmax(min(100%,256px),1fr))] gap-2 p-4";
const FILTERED_PLUGIN_MUTED_CLASS = "text-[var(--scry-muted3)]";

/** Plugin `pluginType` values that belong to each provider-catalog family. */
const EMPTY_PLUGIN_TYPES: readonly string[] = [];

const FAMILY_PLUGIN_TYPES: Record<ProviderCatalogFamily, readonly string[]> = {
  INDEXER: ["indexer", "usenet_indexer", "torrent_indexer"],
  DOWNLOAD_CLIENT: ["download_client"],
  ARCHIVE_EXTRACTOR: ["archive_extractor"],
  NOTIFICATION: ["notification"],
  SUBTITLE: ["subtitle_provider"],
};

export type FilteredPluginListProps = {
  /** Plugin family to show + manage (e.g. "indexer"). */
  family: ProviderCatalogFamily;
  /** Refreshes provider options after a plugin change so new providers appear. */
  refreshProviderOptions: () => Promise<void>;
  /** Bumps when the provider catalog changes for this panel. */
  catalogVersion?: number;
  /** Additional plugin types to show on adjacent product surfaces. */
  extraPluginTypes?: readonly string[];
  /** Panel heading; defaults to "Plugins". */
  title?: string;
  className?: string;
};

/**
 * A self-contained, family-filtered plugin manager for embedding on per-type
 * settings surfaces (indexers, download clients, notifications, subtitles). It
 * lists the family's plugins and lets the user enable/disable, install,
 * uninstall, and upgrade them in place — the same management the full Plugins
 * page offers, scoped to one family.
 */
export function FilteredPluginList({
  family,
  refreshProviderOptions,
  catalogVersion,
  extraPluginTypes = EMPTY_PLUGIN_TYPES,
  title,
  className,
}: FilteredPluginListProps) {
  const client = useClient();
  const t = useTranslate();
  const [upgradingVisiblePlugins, setUpgradingVisiblePlugins] =
    React.useState(false);
  const [pendingUninstall, setPendingUninstall] =
    React.useState<RegistryPluginRecord | null>(null);
  const {
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
  } = usePluginManagement({ client, t, refreshProviderOptions, catalogVersion });

  const allowedTypes = React.useMemo(
    () => new Set([...FAMILY_PLUGIN_TYPES[family], ...extraPluginTypes]),
    [extraPluginTypes, family],
  );
  const familyPlugins = React.useMemo(
    () =>
      plugins
        .filter((plugin) => allowedTypes.has(plugin.pluginType))
        .sort(
          (a, b) =>
            Number(b.isInstalled) - Number(a.isInstalled) ||
            a.name.localeCompare(b.name),
        ),
    [plugins, allowedTypes],
  );
  const visibleUpgradablePlugins = React.useMemo(
    () =>
      familyPlugins.filter(
        (plugin) =>
          plugin.isInstalled &&
          plugin.updateAvailable &&
          !mutatingPluginIds.includes(plugin.id) &&
          !isRunningPluginProgress(pluginProgress[plugin.id]),
      ),
    [familyPlugins, mutatingPluginIds, pluginProgress],
  );
  const visibleUpgradeCount = visibleUpgradablePlugins.length;
  const updateAllLabel = t("settings.pluginsUpdateAll");
  const updateAllTooltip =
    visibleUpgradeCount > 0
      ? `${updateAllLabel} (${visibleUpgradeCount})`
      : updateAllLabel;
  const updateVisiblePlugins = React.useCallback(async () => {
    if (visibleUpgradablePlugins.length === 0) {
      return;
    }
    setUpgradingVisiblePlugins(true);
    try {
      await Promise.allSettled(
        visibleUpgradablePlugins.map((plugin) => upgradePlugin(plugin)),
      );
    } finally {
      setUpgradingVisiblePlugins(false);
    }
  }, [upgradePlugin, visibleUpgradablePlugins]);
  const confirmUninstall = React.useCallback(async () => {
    if (!pendingUninstall) {
      return;
    }
    await uninstallPlugin(pendingUninstall);
    setPendingUninstall(null);
  }, [pendingUninstall, uninstallPlugin]);

  return (
    <>
      <section className={cn(FILTERED_PLUGIN_PANEL_CLASS, className)}>
        <div className={FILTERED_PLUGIN_HEADER_CLASS}>
          <h2 className={FILTERED_PLUGIN_TITLE_CLASS}>
            {title ?? t("settings.plugins")}
          </h2>
          <div className="flex shrink-0 items-center gap-1.5">
            {visibleUpgradeCount > 0 ? (
              <IconButton
                label={updateAllLabel}
                tooltip={updateAllTooltip}
                tone="upgrade"
                onClick={() => void updateVisiblePlugins()}
                disabled={upgradingVisiblePlugins}
              >
                <ArrowUpCircle
                  className={cn(
                    "h-4 w-4",
                    upgradingVisiblePlugins && "animate-spin",
                  )}
                />
              </IconButton>
            ) : null}
            <IconButton
              label={t("settings.pluginsRefresh")}
              tone="neutral"
              onClick={() => void refreshPluginsRegistry()}
              disabled={pluginsRefreshing}
            >
              <RefreshCw
                className={cn("h-4 w-4", pluginsRefreshing && "animate-spin")}
              />
            </IconButton>
          </div>
        </div>
        <div className={FILTERED_PLUGIN_BODY_CLASS}>
          {pluginsError ? (
            <p className="col-span-full text-xs text-[var(--scry-danger-text-soft)]">
              {pluginsError}
            </p>
          ) : null}
          {pluginsLoading ? (
            <div className={`col-span-full flex items-center gap-2 py-6 text-sm ${FILTERED_PLUGIN_MUTED_CLASS}`}>
              <Loader2 className="h-4 w-4 animate-spin" />
              {t("label.loading")}
            </div>
          ) : familyPlugins.length === 0 ? (
            <p className={`col-span-full py-6 text-sm ${FILTERED_PLUGIN_MUTED_CLASS}`}>
              {t("settings.pluginsNoAvailable")}
            </p>
          ) : (
            familyPlugins.map((plugin) => {
              const mutating = mutatingPluginIds.includes(plugin.id);
              const progress = pluginProgress[plugin.id];
              const running = Boolean(
                progress && isRunningPluginProgress(progress),
              );
              const error = pluginErrors[plugin.id];
              const canUninstall = plugin.isInstalled && !plugin.builtin;
              const hasStatusBadges =
                plugin.builtin ||
                plugin.official ||
                plugin.supportTier === "verified_community" ||
                plugin.supportTier === "unverified" ||
                plugin.status === "beta" ||
                plugin.status === "deprecated";
              return (
                <div
                  key={plugin.id}
                  className="min-w-0 rounded-[12px] border border-[var(--scry-line2)] bg-[var(--scry-card2)] p-3"
                >
                  <div className="flex items-center justify-between gap-2">
                    <div className="flex min-w-0 items-start gap-2">
                      <PluginLogo
                        id={plugin.id}
                        name={plugin.name}
                        providerType={plugin.providerType}
                        pluginType={plugin.pluginType}
                        className="h-7 w-7 rounded-lg"
                      />
                      <div className="min-w-0">
                        <div className="flex min-w-0 items-center gap-2">
                          {plugin.isInstalled && plugin.isEnabled ? (
                            <span
                              className="h-1.5 w-1.5 shrink-0 rounded-full bg-[var(--scry-success-solid)]"
                              aria-hidden="true"
                            />
                          ) : null}
                          <span className="truncate text-[13px] font-semibold text-[var(--scry-ink2)]">
                            {plugin.name}
                          </span>
                        </div>
                        {plugin.description ? (
                          <p className={`mt-0.5 line-clamp-2 text-[11.5px] leading-snug ${FILTERED_PLUGIN_MUTED_CLASS}`}>
                            {plugin.description}
                          </p>
                        ) : null}
                        {hasStatusBadges ? (
                          <div className="mt-1.5 flex flex-wrap items-center gap-1">
                            {plugin.builtin ? (
                              <Badge tone="info">{t("settings.pluginBuiltin")}</Badge>
                            ) : null}
                            {plugin.official ? (
                              <Badge tone="info">{t("settings.pluginOfficial")}</Badge>
                            ) : null}
                            {plugin.supportTier === "verified_community" ? (
                              <Badge tone="positive">
                                {t("settings.pluginVerifiedCommunity")}
                              </Badge>
                            ) : null}
                            {plugin.supportTier === "unverified" ? (
                              <Badge tone="warning">
                                {t("settings.pluginUnverified")}
                              </Badge>
                            ) : null}
                            {plugin.status === "beta" ? (
                              <Badge tone="warning">{t("settings.pluginBeta")}</Badge>
                            ) : null}
                            {plugin.status === "deprecated" ? (
                              <Badge tone="negative">
                                {t("settings.pluginDeprecated")}
                              </Badge>
                            ) : null}
                            {plugin.builtin && plugin.sourceKind === "downloaded" ? (
                              <Badge tone="warning">
                                {t("settings.pluginOverride")}
                              </Badge>
                            ) : null}
                          </div>
                        ) : null}
                      </div>
                    </div>
                    {running ? null : (
                      <div className="flex shrink-0 items-center self-center gap-1">
                        {plugin.isInstalled ? (
                          <>
                            <IconButton
                              label={
                                plugin.isEnabled
                                  ? t("label.disable")
                                  : t("label.enable")
                              }
                              tone={plugin.isEnabled ? "disabled" : "enabled"}
                              onClick={() => void togglePlugin(plugin)}
                              disabled={mutating}
                            >
                              {plugin.isEnabled ? (
                                <PowerOff className="h-3.5 w-3.5" />
                              ) : (
                                <Power className="h-3.5 w-3.5" />
                              )}
                            </IconButton>
                          {plugin.updateAvailable ? (
                            <IconButton
                              label={t("settings.pluginUpgrade", {
                                version: plugin.latestVersion ?? plugin.version,
                              })}
                              tone="upgrade"
                              onClick={() => void upgradePlugin(plugin)}
                              disabled={mutating}
                            >
                              <ArrowUpCircle className="h-3.5 w-3.5" />
                            </IconButton>
                          ) : null}
                          {canUninstall ? (
                            <IconButton
                              label={t("settings.pluginUninstall")}
                              tone="delete"
                              onClick={() => setPendingUninstall(plugin)}
                              disabled={mutating}
                            >
                              <Trash2 className="h-3.5 w-3.5" />
                            </IconButton>
                          ) : null}
                        </>
                        ) : (
                          <IconButton
                            type="button"
                          label={t("settings.pluginInstall")}
                          tone="install"
                          onClick={() => void installPlugin(plugin)}
                          disabled={mutating}
                        >
                          {mutating ? (
                            <Loader2 className="h-3.5 w-3.5 animate-spin" />
                          ) : (
                            <Download className="h-3.5 w-3.5" />
                          )}
                        </IconButton>
                      )}
                    </div>
                  )}
                </div>
                {running && progress ? (
                  <div className="mt-2">
                    <PluginInstallProgressBar progress={progress} />
                  </div>
                ) : null}
                {error ? (
                  <p className="mt-1.5 text-[11px] text-[var(--scry-danger-text-soft)]">{error}</p>
                ) : null}
              </div>
              );
            })
          )}
        </div>
      </section>
      <ConfirmDialog
        open={pendingUninstall !== null}
        title={t("settings.pluginUninstall")}
        description={t("settings.pluginUninstallWarning")}
        confirmLabel={t("settings.pluginUninstall")}
        cancelLabel={t("label.cancel")}
        isBusy={pendingUninstall ? mutatingPluginIds.includes(pendingUninstall.id) : false}
        onConfirm={confirmUninstall}
        onCancel={() => setPendingUninstall(null)}
      />
    </>
  );
}

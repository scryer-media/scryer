import { Blocks, Download, Loader2, PlugZap, RefreshCw, Trash2 } from "lucide-react";

import { PluginLogo } from "@/components/common/plugin-visual";
import { Badge } from "@/components/ui/badge";
import { IconButton } from "@/components/ui/icon-button";
import { Progress } from "@/components/ui/progress";
import { formatPluginBytes } from "@/components/views/settings/settings-plugins-section";
import type {
  PluginInstallProgressRecord,
  RegistryPluginRecord,
} from "@/components/views/settings/settings-plugins-section";
import { selectorId } from "@/lib/utils/dom-ids";
import {
  SetupBackButton,
  SetupPanel,
  SetupPrimaryButton,
  SetupStepHeader,
} from "./setup-chrome";

interface SetupPluginsViewProps {
  t: (
    key: string,
    values?: Record<string, string | number | boolean | null | undefined>,
  ) => string;
  plugins: RegistryPluginRecord[];
  loading: boolean;
  refreshing: boolean;
  mutatingPluginIds: string[];
  pluginProgress: Partial<Record<string, PluginInstallProgressRecord>>;
  pluginErrors: Partial<Record<string, string>>;
  error: string | null;
  onRefreshRegistry: () => void;
  onInstallPlugin: (plugin: RegistryPluginRecord) => void;
  onUninstallPlugin: (plugin: RegistryPluginRecord) => void;
  onNext: () => void;
  onBack: () => void;
}

function categoryLabel(
  pluginType: string,
  t: (
    key: string,
    values?: Record<string, string | number | boolean | null | undefined>,
  ) => string,
) {
  if (pluginType === "indexer" || pluginType.endsWith("_indexer")) {
    return t("settings.pluginCategoryIndexer");
  }
  if (pluginType === "download_client") {
    return t("settings.pluginCategoryDownloadClient");
  }
  if (pluginType === "archive_extractor") {
    return t("settings.pluginCategoryArchiveExtractor");
  }
  if (pluginType === "notification") {
    return t("settings.pluginCategoryNotification");
  }
  return pluginType;
}

function categoryKey(pluginType: string) {
  if (pluginType === "indexer" || pluginType.endsWith("_indexer")) {
    return "indexer";
  }
  if (pluginType === "download_client") {
    return "download_client";
  }
  if (pluginType === "archive_extractor") {
    return "archive_extractor";
  }
  if (pluginType === "notification") {
    return "notification";
  }
  return pluginType;
}

function groupPluginsByType(
  plugins: RegistryPluginRecord[],
  t: (
    key: string,
    values?: Record<string, string | number | boolean | null | undefined>,
  ) => string,
) {
  const groups = new Map<
    string,
    { label: string; plugins: RegistryPluginRecord[] }
  >();

  for (const plugin of plugins) {
    const key = categoryKey(plugin.pluginType);
    const existing = groups.get(key);
    if (existing) {
      existing.plugins.push(plugin);
      continue;
    }
    groups.set(key, {
      label: categoryLabel(key, t),
      plugins: [plugin],
    });
  }

  return [...groups.entries()]
    .map(([key, value]) => ({
      key,
      label: value.label,
      plugins: value.plugins.sort((left, right) =>
        left.name.localeCompare(right.name),
      ),
    }))
    .sort((left, right) => left.label.localeCompare(right.label));
}

function canUninstallPlugin(plugin: RegistryPluginRecord) {
  return !plugin.builtin || plugin.sourceKind === "downloaded";
}

function uninstallLabel(plugin: RegistryPluginRecord, t: SetupPluginsViewProps["t"]) {
  return plugin.builtin && plugin.sourceKind === "downloaded"
    ? t("settings.pluginRevertToBundled")
    : t("settings.pluginUninstall");
}

function isRunningPluginProgress(
  progress?: PluginInstallProgressRecord,
): progress is PluginInstallProgressRecord {
  return progress !== undefined
    && progress.state !== "SUCCEEDED"
    && progress.state !== "FAILED";
}

function pluginProgressLabel(
  progress: PluginInstallProgressRecord,
  t: SetupPluginsViewProps["t"],
): string {
  switch (progress.state) {
    case "DOWNLOADING":
      return t("settings.pluginInstallDownloading");
    case "VERIFYING":
      return t("settings.pluginInstallVerifying");
    case "INSTALLING":
      return t("settings.pluginInstallInstalling");
    case "SUCCEEDED":
    case "FAILED":
      return progress.label;
    default:
      return progress.label;
  }
}

export function SetupPluginsView({
  t,
  plugins,
  loading,
  refreshing,
  mutatingPluginIds,
  pluginProgress,
  pluginErrors,
  error,
  onRefreshRegistry,
  onInstallPlugin,
  onUninstallPlugin,
  onNext,
  onBack,
}: SetupPluginsViewProps) {
  const groupedPlugins = groupPluginsByType(
    plugins.filter((plugin) => plugin.official),
    t,
  );

  return (
    <SetupPanel id="setup-plugins-view" className="flex flex-col gap-6">
      <SetupStepHeader
        icon={Blocks}
        title={t("setup.pluginsTitle")}
        subtitle={t("setup.pluginsDescription")}
      />

      <div className="mx-auto w-full max-w-6xl rounded-xl border border-dashed border-border bg-muted/30 px-4 py-3 text-sm">
        <span className="font-medium">{t("setup.pluginsBuiltInTitle")}:</span>{" "}
        <span className="text-muted-foreground">
          {t("setup.pluginsBuiltInDescription")}
        </span>
      </div>

      <div className="mx-auto flex w-full max-w-6xl items-center justify-between gap-4">
        <div>
          <p className="text-sm font-medium">
            {t("setup.pluginsAvailableHeading")}
          </p>
          <p className="text-sm text-muted-foreground">
            {t("setup.pluginsAvailableHint")}
          </p>
        </div>
        <IconButton
          id="setup-plugins-refresh"
          label={refreshing ? t("label.refreshing") : t("label.refresh")}
          tone="neutral"
          disabled={refreshing || loading}
          onClick={onRefreshRegistry}
        >
          {refreshing ? (
            <Loader2 className="h-4 w-4 animate-spin" />
          ) : (
            <RefreshCw className="h-4 w-4" />
          )}
        </IconButton>
      </div>

      {error && (
        <p className="mx-auto w-full max-w-6xl text-sm text-destructive">
          {error}
        </p>
      )}

      {loading ? (
        <div className="mx-auto flex w-full max-w-6xl items-center justify-center gap-2 rounded-xl border border-dashed border-border py-10 text-sm text-muted-foreground">
          <Loader2 className="h-4 w-4 animate-spin" />
          {t("label.loading")}
        </div>
      ) : (
        <div className="mx-auto w-full max-w-6xl">
          {groupedPlugins.length === 0 ? (
            <div className="rounded-xl border border-dashed border-border py-10 text-center text-sm text-muted-foreground">
              {t("setup.pluginsNoneFound")}
            </div>
          ) : (
            <div className="space-y-5">
              {groupedPlugins.map((group) => (
                <section key={group.key} className="space-y-2">
                  <div className="flex items-center gap-2">
                    <span className="shrink-0 text-xs font-semibold uppercase tracking-[0.16em] text-muted-foreground">
                      {group.label}
                    </span>
                    <span className="h-px flex-1 bg-[var(--scry-line2)]" />
                  </div>
                  <div className="grid grid-cols-1 gap-2 sm:grid-cols-2 lg:grid-cols-3">
                    {group.plugins.map((plugin) => {
                      const progress = pluginProgress[plugin.id];
                      const runningProgress = isRunningPluginProgress(progress)
                        ? progress
                        : undefined;
                      const isBusy =
                        mutatingPluginIds.includes(plugin.id) ||
                        plugin.installInProgress;
                      const actionError = pluginErrors[plugin.id];
                      const bytesLabel = formatPluginBytes(plugin.bytes);
                      return (
                        <div
                          key={plugin.id}
                          id={selectorId("setup-plugin-card", plugin.name)}
                          className="min-w-0 rounded-[12px] border border-[var(--scry-line2)] bg-[var(--scry-card2)] p-3"
                        >
                          <div className="flex items-start justify-between gap-2">
                            <div className="flex min-w-0 flex-1 items-start gap-2">
                              <PluginLogo
                                id={plugin.id}
                                name={plugin.name}
                                providerType={plugin.providerType}
                                pluginType={plugin.pluginType}
                                className="h-8 w-8 rounded-lg"
                              />
                              <div className="min-w-0">
                                <div className="flex min-w-0 items-center gap-2">
                                  {plugin.isInstalled ? (
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
                                  <p className="mt-0.5 line-clamp-2 text-[11.5px] leading-snug text-[var(--scry-muted3)]">
                                    {plugin.description}
                                  </p>
                                ) : null}
                                <div className="mt-1.5 flex flex-wrap items-center gap-1">
                                  {plugin.isInstalled ? (
                                    <Badge tone="positive">
                                      {t("settings.pluginInstalled")}
                                    </Badge>
                                  ) : null}
                                  {plugin.status === "beta" ? (
                                    <Badge tone="warning">
                                      {t("settings.pluginBeta")}
                                    </Badge>
                                  ) : null}
                                  {plugin.status === "deprecated" ? (
                                    <Badge tone="negative">
                                      {t("settings.pluginDeprecated")}
                                    </Badge>
                                  ) : null}
                                  {bytesLabel ? (
                                    <Badge
                                      tone="outline"
                                      title={
                                        plugin.bytes != null
                                          ? `${plugin.bytes} bytes`
                                          : undefined
                                      }
                                    >
                                      {bytesLabel}
                                    </Badge>
                                  ) : null}
                                </div>
                              </div>
                            </div>
                            <div className="flex shrink-0 items-center self-start gap-1">
                              {plugin.isInstalled ? (
                                canUninstallPlugin(plugin) ? (
                                  <IconButton
                                    id={selectorId(
                                      "setup-plugin-uninstall",
                                      plugin.name,
                                    )}
                                    label={uninstallLabel(plugin, t)}
                                    tone="delete"
                                    disabled={isBusy}
                                    onClick={() => onUninstallPlugin(plugin)}
                                  >
                                    {isBusy ? (
                                      <Loader2 className="h-3.5 w-3.5 animate-spin" />
                                    ) : (
                                      <Trash2 className="h-3.5 w-3.5" />
                                    )}
                                  </IconButton>
                                ) : null
                              ) : (
                                <IconButton
                                  id={selectorId(
                                    "setup-plugin-install",
                                    plugin.name,
                                  )}
                                  label={
                                    isBusy
                                      ? t("settings.pluginInstalling")
                                      : t("settings.pluginInstall")
                                  }
                                  tone="install"
                                  disabled={isBusy}
                                  onClick={() => onInstallPlugin(plugin)}
                                >
                                  {isBusy ? (
                                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                                  ) : (
                                    <Download className="h-3.5 w-3.5" />
                                  )}
                                </IconButton>
                              )}
                            </div>
                          </div>
                          {runningProgress ? (
                            <div className="mt-2 space-y-1 overflow-hidden">
                              <div className="truncate text-xs leading-tight text-primary">
                                {pluginProgressLabel(runningProgress, t)}
                              </div>
                              <Progress
                                value={
                                  (runningProgress.stepIndex /
                                    Math.max(runningProgress.stepCount, 1)) *
                                  100
                                }
                                className="h-1.5"
                              />
                            </div>
                          ) : null}
                          {actionError ? (
                            <p className="mt-1.5 text-[11px] text-[var(--scry-danger-text-soft)]">
                              {actionError}
                            </p>
                          ) : null}
                        </div>
                      );
                    })}
                  </div>
                </section>
              ))}
            </div>
          )}
        </div>
      )}

      <div className="flex items-center justify-between pt-2">
        <SetupBackButton id="setup-plugins-back" onClick={onBack}>
          {t("setup.back")}
        </SetupBackButton>
        <SetupPrimaryButton id="setup-plugins-next" onClick={onNext}>
          <PlugZap className="h-4 w-4" />
          {t("setup.next")}
        </SetupPrimaryButton>
      </div>
    </SetupPanel>
  );
}

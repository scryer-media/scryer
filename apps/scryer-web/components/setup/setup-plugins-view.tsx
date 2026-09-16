import type { ReactNode } from "react";
import { Blocks, Download, PlugZap, RefreshCw, Trash2 } from "lucide-react";

import { PluginLogo } from "@/components/common/plugin-visual";
import { Badge } from "@/components/ui/badge";
import { IconButton } from "@/components/ui/icon-button";
import { Progress } from "@/components/ui/progress";
import { Switch } from "@/components/ui/switch";
import { formatPluginBytes } from "@/components/views/settings/settings-plugins-section";
import type {
  PluginInstallProgressRecord,
  RegistryPluginRecord,
} from "@/components/views/settings/settings-plugins-section";
import type { SetupRulePackState } from "@/lib/hooks/use-setup-rule-packs";
import { cn } from "@/lib/utils";
import { selectorId } from "@/lib/utils/dom-ids";
import {
  otherSetupPlugins,
  resolveSetupPluginRecommendations,
} from "@/lib/utils/setup-recommendations";
import {
  SetupBackButton,
  SetupPanel,
  SetupPrimaryButton,
  SetupStepHeader,
} from "./setup-chrome";
import { LoadingMark } from "@/components/common/loading-mark";

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
  rulePacks: SetupRulePackState[];
  rulePacksLoading: boolean;
  onRefreshRegistry: () => void;
  onInstallPlugin: (plugin: RegistryPluginRecord) => void;
  onUninstallPlugin: (plugin: RegistryPluginRecord) => void;
  onSetRulePackEnabled: (packId: string, enabled: boolean) => void;
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


type PluginActionProps = {
  plugin: RegistryPluginRecord;
  t: SetupPluginsViewProps["t"];
  busy: boolean;
  onInstallPlugin: (plugin: RegistryPluginRecord) => void;
  onUninstallPlugin: (plugin: RegistryPluginRecord) => void;
};

function SetupPluginAction({
  plugin,
  t,
  busy,
  onInstallPlugin,
  onUninstallPlugin,
}: PluginActionProps) {
  if (plugin.isInstalled) {
    if (!canUninstallPlugin(plugin)) {
      return null;
    }
    return (
      <IconButton
        id={selectorId("setup-plugin-uninstall", plugin.name)}
        label={uninstallLabel(plugin, t)}
        tone="delete"
        disabled={busy}
        onClick={() => onUninstallPlugin(plugin)}
      >
        {busy ? (
          <LoadingMark className="h-3.5 w-3.5" />
        ) : (
          <Trash2 className="h-3.5 w-3.5" />
        )}
      </IconButton>
    );
  }
  return (
    <IconButton
      id={selectorId("setup-plugin-install", plugin.name)}
      label={busy ? t("settings.pluginInstalling") : t("settings.pluginInstall")}
      tone="install"
      disabled={busy}
      onClick={() => onInstallPlugin(plugin)}
    >
      {busy ? (
        <LoadingMark className="h-3.5 w-3.5" />
      ) : (
        <Download className="h-3.5 w-3.5" />
      )}
    </IconButton>
  );
}

function SetupPluginStatus({
  t,
  progress,
  error,
}: {
  t: SetupPluginsViewProps["t"];
  progress?: PluginInstallProgressRecord;
  error?: string;
}) {
  const runningProgress = isRunningPluginProgress(progress) ? progress : undefined;
  return (
    <>
      {runningProgress ? (
        <div className="mt-2 space-y-1 overflow-hidden">
          <div className="truncate text-xs leading-tight text-primary">
            {pluginProgressLabel(runningProgress, t)}
          </div>
          <Progress
            value={
              (runningProgress.stepIndex / Math.max(runningProgress.stepCount, 1)) *
              100
            }
            className="h-1.5"
          />
        </div>
      ) : null}
      {error ? (
        <p className="mt-1.5 text-[11px] text-[var(--scry-danger-text-soft)]">
          {error}
        </p>
      ) : null}
    </>
  );
}

function SetupPluginBadges({
  plugin,
  t,
}: {
  plugin: RegistryPluginRecord;
  t: SetupPluginsViewProps["t"];
}) {
  const bytesLabel = formatPluginBytes(plugin.bytes);
  return (
    <div className="mt-1.5 flex flex-wrap items-center gap-1">
      {plugin.isInstalled ? (
        <Badge tone="positive">{t("settings.pluginInstalled")}</Badge>
      ) : null}
      {plugin.status === "beta" ? (
        <Badge tone="warning">{t("settings.pluginBeta")}</Badge>
      ) : null}
      {plugin.status === "deprecated" ? (
        <Badge tone="negative">{t("settings.pluginDeprecated")}</Badge>
      ) : null}
      {bytesLabel ? (
        <Badge
          tone="outline"
          title={plugin.bytes != null ? `${plugin.bytes} bytes` : undefined}
        >
          {bytesLabel}
        </Badge>
      ) : null}
    </div>
  );
}

type SetupPluginCardProps = Omit<PluginActionProps, "busy"> & {
  mutatingPluginIds: string[];
  pluginProgress: SetupPluginsViewProps["pluginProgress"];
  pluginErrors: SetupPluginsViewProps["pluginErrors"];
  showDescription: boolean;
};

function SetupPluginCard({
  plugin,
  t,
  mutatingPluginIds,
  pluginProgress,
  pluginErrors,
  showDescription,
  onInstallPlugin,
  onUninstallPlugin,
}: SetupPluginCardProps) {
  const busy = mutatingPluginIds.includes(plugin.id) || plugin.installInProgress;
  return (
    <div
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
              <span className="line-clamp-2 break-words text-[13px] font-semibold text-[var(--scry-ink2)]">
                {plugin.name}
              </span>
            </div>
            {showDescription && plugin.description ? (
              <p className="mt-0.5 line-clamp-2 text-[11.5px] leading-snug text-[var(--scry-muted3)]">
                {plugin.description}
              </p>
            ) : null}
            <SetupPluginBadges plugin={plugin} t={t} />
          </div>
        </div>
        <div className="flex shrink-0 items-center self-start gap-1">
          <SetupPluginAction
            plugin={plugin}
            t={t}
            busy={busy}
            onInstallPlugin={onInstallPlugin}
            onUninstallPlugin={onUninstallPlugin}
          />
        </div>
      </div>
      <SetupPluginStatus
        t={t}
        progress={pluginProgress[plugin.id]}
        error={pluginErrors[plugin.id]}
      />
    </div>
  );
}

function SetupSectionHeading({
  title,
  hint,
  children,
}: {
  title: string;
  hint: string;
  children?: ReactNode;
}) {
  return (
    <div className="flex items-center justify-between gap-4">
      <div>
        <p className="text-sm font-medium">{title}</p>
        <p className="text-sm text-muted-foreground">{hint}</p>
      </div>
      {children}
    </div>
  );
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
  rulePacks,
  rulePacksLoading,
  onRefreshRegistry,
  onInstallPlugin,
  onUninstallPlugin,
  onSetRulePackEnabled,
  onNext,
  onBack,
}: SetupPluginsViewProps) {
  const recommendations = resolveSetupPluginRecommendations(plugins);
  const groupedPlugins = groupPluginsByType(otherSetupPlugins(plugins), t);
  const cardProps = {
    t,
    mutatingPluginIds,
    pluginProgress,
    pluginErrors,
    onInstallPlugin,
    onUninstallPlugin,
  };

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

      {error && (
        <p className="mx-auto w-full max-w-6xl text-sm text-destructive">
          {error}
        </p>
      )}

      <section
        id="setup-recommended-plugins"
        className="mx-auto flex w-full max-w-6xl flex-col gap-3"
      >
        <SetupSectionHeading
          title={t("setup.recommendedHeading")}
          hint={t("setup.recommendedHint")}
        >
          <IconButton
            id="setup-plugins-refresh"
            label={refreshing ? t("label.refreshing") : t("label.refresh")}
            tone="neutral"
            disabled={refreshing || loading}
            onClick={onRefreshRegistry}
          >
            {refreshing ? (
              <LoadingMark className="h-4 w-4" />
            ) : (
              <RefreshCw className="h-4 w-4" />
            )}
          </IconButton>
        </SetupSectionHeading>
        {loading ? (
          <div className="flex items-center justify-center gap-2 rounded-xl border border-dashed border-border py-10 text-sm text-muted-foreground">
            <LoadingMark className="h-4 w-4" />
            {t("label.loading")}
          </div>
        ) : recommendations.length === 0 ? (
          <div className="rounded-xl border border-dashed border-border py-10 text-center text-sm text-muted-foreground">
            {t("setup.pluginsNoneFound")}
          </div>
        ) : (
          <div className="grid grid-flow-row-dense grid-cols-1 gap-3 md:grid-cols-2 lg:grid-cols-3">
            {recommendations.map((recommendation) => (
              <div
                key={recommendation.key}
                id={selectorId("setup-recommendation", recommendation.key)}
                className={cn(
                  "flex min-w-0 flex-col gap-2 rounded-[14px] border border-[var(--scry-line2)] bg-[var(--scry-page2)] p-4",
                  recommendation.plugins.length > 1 && "md:col-span-2 lg:col-span-3",
                )}
              >
                <div>
                  <p className="text-sm font-semibold text-[var(--scry-ink2)]">
                    {t(recommendation.titleKey)}
                  </p>
                  {recommendation.reasonKey ? (
                    <p className="mt-0.5 text-[12.5px] leading-snug text-[var(--scry-muted3)]">
                      {t(recommendation.reasonKey)}
                    </p>
                  ) : null}
                </div>
                <div
                  className={cn(
                    "grid grid-cols-1 gap-2",
                    recommendation.plugins.length > 1 && "sm:grid-cols-2",
                    recommendation.plugins.length === 3 && "lg:grid-cols-3",
                    recommendation.plugins.length >= 4 && "lg:grid-cols-4",
                  )}
                >
                  {recommendation.plugins.map((plugin) => (
                    <SetupPluginCard
                      key={plugin.id}
                      plugin={plugin}
                      showDescription={recommendation.reasonKey === null}
                      {...cardProps}
                    />
                  ))}
                </div>
              </div>
            ))}
          </div>
        )}
      </section>

      <section
        id="setup-rule-packs"
        className="mx-auto flex w-full max-w-6xl flex-col gap-3"
      >
        <SetupSectionHeading
          title={t("setup.rulePacksHeading")}
          hint={t("setup.rulePacksHint")}
        />
        <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
          {rulePacks.map((pack) => {
            const switchId = selectorId("setup-rule-pack-toggle", pack.packId);
            const unavailable = !rulePacksLoading && !pack.available;
            return (
              <div
                key={pack.packId}
                id={selectorId("setup-rule-pack", pack.packId)}
                className="flex min-w-0 items-start justify-between gap-4 rounded-[14px] border border-[var(--scry-line2)] bg-[var(--scry-page2)] p-4"
              >
                <div className="min-w-0">
                  <label
                    htmlFor={switchId}
                    className="text-sm font-semibold text-[var(--scry-ink2)]"
                  >
                    {t(pack.titleKey)}
                  </label>
                  <p className="mt-0.5 text-[12.5px] leading-snug text-[var(--scry-muted3)]">
                    {t(pack.reasonKey)}
                  </p>
                  {unavailable ? (
                    <p className="mt-1.5 text-[11px] text-[var(--scry-muted3)]">
                      {t("setup.rulePackUnavailable")}
                    </p>
                  ) : null}
                  {pack.error ? (
                    <p className="mt-1.5 text-[11px] text-[var(--scry-danger-text-soft)]">
                      {pack.error}
                    </p>
                  ) : null}
                </div>
                <div className="flex shrink-0 items-center gap-2">
                  {pack.busy || rulePacksLoading ? (
                    <LoadingMark className="h-4 w-4" />
                  ) : null}
                  <Switch
                    id={switchId}
                    checked={pack.enabled}
                    disabled={pack.busy || rulePacksLoading || unavailable}
                    onCheckedChange={(checked) =>
                      onSetRulePackEnabled(pack.packId, checked)
                    }
                  />
                </div>
              </div>
            );
          })}
        </div>
      </section>

      {!loading && groupedPlugins.length > 0 ? (
        <section
          id="setup-other-plugins"
          className="mx-auto flex w-full max-w-6xl flex-col gap-3"
        >
          <SetupSectionHeading
            title={t("setup.otherPluginsHeading")}
            hint={t("setup.pluginsAvailableHint")}
          />
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
                  {group.plugins.map((plugin) => (
                    <SetupPluginCard
                      key={plugin.id}
                      plugin={plugin}
                      showDescription
                      {...cardProps}
                    />
                  ))}
                </div>
              </section>
            ))}
          </div>
        </section>
      ) : null}

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

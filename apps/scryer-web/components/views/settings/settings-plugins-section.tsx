import { useState, useMemo, useRef } from "react";
import { createPortal } from "react-dom";
import { ArrowUpCircle, Download, ExternalLink, Loader2, Power, PowerOff, RefreshCw, Trash2, Upload } from "lucide-react";
import { PluginLogo } from "@/components/common/plugin-visual";
import { RenderBooleanIcon } from "@/components/common/boolean-icon";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { IconButton } from "@/components/ui/icon-button";
import { TextActionButton } from "@/components/ui/text-action-button";
import { CheckboxField } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { SettingsToggleSwitch } from "@/components/common/settings-toggle-switch";
import { Progress } from "@/components/ui/progress";
import { SingleSelectField } from "@/components/ui/select";
import {
  Table,
  TableActionsCell,
  TableActionsHead,
  TableBody,
  TableCell,
  TableCodeCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useTranslate } from "@/lib/context/translate-context";
import { selectorId } from "@/lib/utils/dom-ids";
import { cn } from "@/lib/utils";
import type { BoxedActionButtonTone } from "@/lib/utils/action-button-styles";

const PLUGIN_PANEL_CLASS =
  "overflow-hidden rounded-[14px] border border-[var(--scry-border)] bg-[var(--scry-surf)] shadow-[0_10px_24px_rgba(0,0,0,0.16)]";
const PLUGIN_PANEL_HEADER_CLASS =
  "border-b border-[var(--scry-border3)] bg-[linear-gradient(180deg,rgba(255,255,255,0.035),rgba(255,255,255,0))] px-4 py-3";
const PLUGIN_PANEL_TITLE_CLASS =
  "text-[15px] font-semibold text-[var(--scry-ink2)]";
const PLUGIN_PANEL_BODY_CLASS = "p-4 sm:p-5";
const PLUGIN_INSET_CLASS =
  "rounded-[12px] border border-[var(--scry-line2)] bg-[var(--scry-card2)]";
const PLUGIN_MUTED_TEXT_CLASS = "text-[var(--scry-muted3)]";
const PLUGIN_TABLE_HEADER_CELL_CLASS =
  "!bg-transparent text-xs font-semibold text-[var(--scry-muted3)]";
const PLUGIN_TABLE_ROW_CLASS =
  "border-[var(--scry-border3)] [&>td]:bg-transparent";
const PLUGIN_TABLE_CELL_CLASS = "align-middle";

export type RegistryPluginRecord = {
  id: string;
  name: string;
  description: string;
  version: string;
  latestVersion?: string | null;
  pluginType: string;
  providerType: string;
  author: string;
  official: boolean;
  publisher?: string | null;
  supportTier?: string | null;
  status?: string | null;
  docsUrl?: string | null;
  sourceRepo?: string | null;
  builtin: boolean;
  sourceKind?: string | null;
  blockedReason?: string | null;
  bytes?: number | null;
  isInstalled: boolean;
  isEnabled: boolean;
  installedVersion: string | null;
  updateAvailable: boolean;
  installInProgress: boolean;
  defaultBaseUrl?: string | null;
};

export type PluginInstallProgressRecord = {
  pluginId: string;
  operationKind: "INSTALL" | "UPGRADE";
  state: "DOWNLOADING" | "VERIFYING" | "INSTALLING" | "SUCCEEDED" | "FAILED";
  label: string;
  stepIndex: number;
  stepCount: number;
  message?: string | null;
  error?: string | null;
};

export type PluginCatalogStatusRecord = {
  refreshState: 'READY' | 'DEGRADED';
  githubAvailable: boolean;
  lastCheckedAt?: string | null;
  outageMessage?: string | null;
  blockedActions: string[];
  restoreWarnings: string[];
  lastError?: string | null;
};

export type ManualPluginPreviewRecord = {
  githubRepoUrl: string;
  plugin: RegistryPluginRecord;
};

type SettingsPluginsSectionProps = {
  plugins: RegistryPluginRecord[];
  catalogStatus: PluginCatalogStatusRecord | null;
  initialLoading: boolean;
  mutatingPluginIds: string[];
  pluginProgress: Partial<Record<string, PluginInstallProgressRecord>>;
  pluginErrors: Partial<Record<string, string>>;
  refreshing: boolean;
  upgradingAll: boolean;
  manualRepoUrl: string;
  manualFileName: string | null;
  manualPreview: ManualPluginPreviewRecord | null;
  manualBusy: boolean;
  showManualInstall: boolean;
  headerActionsTarget?: HTMLElement | null;
  autoUpdateEnabled: boolean;
  autoUpdateLoading: boolean;
  autoUpdateSaving: boolean;
  remoteActionsBlocked: {
    refresh: boolean;
    install: boolean;
    installManual: boolean;
    upgrade: boolean;
    inspectManual: boolean;
  };
  onAutoUpdateEnabledChange: (enabled: boolean) => void;
  onManualRepoUrlChange: (value: string) => void;
  onToggleManualInstall: () => void;
  onManualPluginFileChange: (event: React.ChangeEvent<HTMLInputElement>) => void;
  onInspectManualPluginRepo: () => void;
  onRequestInstallUploadedPlugin: () => void;
  onInstallManualPlugin: () => void;
  onRefreshRegistry: () => void;
  onUpgradeAllPlugins: () => void;
  onTogglePlugin: (plugin: RegistryPluginRecord) => void;
  onInstallPlugin: (plugin: RegistryPluginRecord) => void;
  onUninstallPlugin: (plugin: RegistryPluginRecord) => void;
  onUpgradePlugin: (plugin: RegistryPluginRecord) => void;
};

type FilterState = {
  category: string;
  officialOnly: boolean;
};

type Translate = (key: string, values?: Record<string, string | number | boolean | null | undefined>) => string;

function isDownloadedBuiltinOverride(plugin: RegistryPluginRecord): boolean {
  return plugin.builtin && plugin.sourceKind === "downloaded";
}

function canUninstallPlugin(plugin: RegistryPluginRecord): boolean {
  return !plugin.builtin || plugin.sourceKind === "downloaded";
}

function uninstallLabel(plugin: RegistryPluginRecord, t: Translate): string {
  return isDownloadedBuiltinOverride(plugin)
    ? t("settings.pluginRevertToBundled")
    : t("settings.pluginUninstall");
}

export function formatPluginBytes(bytes?: number | null): string | null {
  if (bytes == null) {
    return null;
  }
  if (bytes < 1024) {
    return `${bytes} B`;
  }
  if (bytes < 1024 * 1024) {
    return `${(bytes / 1024).toFixed(1)} KB`;
  }
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export function isRunningPluginProgress(
  progress?: PluginInstallProgressRecord,
): progress is PluginInstallProgressRecord {
  return progress !== undefined
    && progress.state !== "SUCCEEDED"
    && progress.state !== "FAILED";
}

function pluginProgressLabel(progress: PluginInstallProgressRecord, t: Translate): string {
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

export function PluginInstallProgressBar({
  progress,
  id,
  className = "space-y-1 overflow-hidden",
}: {
  progress: PluginInstallProgressRecord;
  id?: string;
  className?: string;
}) {
  const t = useTranslate();

  return (
    <div className={className}>
      <div className="truncate text-right text-xs leading-tight text-primary">
        {pluginProgressLabel(progress, t)}
      </div>
      <Progress
        id={id}
        value={(progress.stepIndex / Math.max(progress.stepCount, 1)) * 100}
        className="h-1.5"
      />
    </div>
  );
}

function normalizePluginLink(url?: string | null): string | null {
  if (!url) {
    return null;
  }
  const trimmed = url.trim();
  if (!trimmed) {
    return null;
  }
  return trimmed.replace(/\/+$/, "");
}

function PluginActionButton({
  label,
  tone,
  className,
  children,
  ...props
}: Omit<React.ComponentProps<typeof IconButton>, "tone"> & {
  label: string;
  tone: Extract<BoxedActionButtonTone, "install" | "upgrade" | "enabled" | "disabled" | "delete">;
}) {
  return (
    <IconButton
      label={label}
      tone={tone}
      className={className}
      {...props}
    >
      {children}
    </IconButton>
  );
}

function categoryLabel(pluginType: string, t: Translate): string {
  switch (pluginType) {
    case "indexer": return t("settings.pluginCategoryIndexer");
    case "usenet_indexer": return t("settings.pluginCategoryUsenetIndexer");
    case "torrent_indexer": return t("settings.pluginCategoryTorrentIndexer");
    case "download_client": return t("settings.pluginCategoryDownloadClient");
    case "archive_extractor": return t("settings.pluginCategoryArchiveExtractor");
    case "notification": return t("settings.pluginCategoryNotification");
    case "subtitle_provider": return t("settings.pluginCategorySubtitleProvider");
    default: return pluginType;
  }
}

function applyFilters(
  plugins: RegistryPluginRecord[],
  filters: FilterState,
): RegistryPluginRecord[] {
  return plugins
    .filter((p) => filters.category === "all" || p.pluginType === filters.category)
    .filter((p) => !filters.officialOnly || p.official)
    .sort((a, b) => a.name.localeCompare(b.name));
}

function PluginFilters({
  filters,
  categories,
  leadingContent,
  onChange,
}: {
  filters: FilterState;
  categories: string[];
  leadingContent?: React.ReactNode;
  onChange: (filters: FilterState) => void;
}) {
  const t = useTranslate();
  return (
    <div className="flex flex-wrap items-center gap-3">
      {leadingContent}
      <SingleSelectField
        value={filters.category}
        onValueChange={(v) => onChange({ ...filters, category: v })}
        chrome="toolbar"
        size="compact"
        triggerClassName="w-44"
        options={[
          {
            value: "all",
            label: t("settings.pluginAllCategories"),
          },
          ...categories.map((cat) => ({
            value: cat,
            label: categoryLabel(cat, t),
          })),
        ]}
      />
      <CheckboxField
        id="settings-plugins-official-only"
        checked={filters.officialOnly}
        onCheckedChange={(checked) =>
          onChange({ ...filters, officialOnly: !!checked })
        }
        label={t("settings.pluginOfficialOnly")}
        size="compact"
        className="items-center gap-2"
        checkboxClassName="mt-0"
        labelClassName={`text-sm font-medium ${PLUGIN_MUTED_TEXT_CLASS}`}
      />
    </div>
  );
}

function PluginTable({
  plugins,
  mutatingPluginIds,
  pluginProgress,
  pluginErrors,
  showActions,
  onTogglePlugin,
  onInstallPlugin,
  onUninstallPlugin,
  onUpgradePlugin,
  installBlocked,
  upgradeBlocked,
  emptyMessage,
}: {
  plugins: RegistryPluginRecord[];
  mutatingPluginIds: string[];
  pluginProgress: Partial<Record<string, PluginInstallProgressRecord>>;
  pluginErrors: Partial<Record<string, string>>;
  showActions: "installed" | "available";
  onTogglePlugin: (plugin: RegistryPluginRecord) => void;
  onInstallPlugin: (plugin: RegistryPluginRecord) => void;
  onUninstallPlugin: (plugin: RegistryPluginRecord) => void;
  onUpgradePlugin: (plugin: RegistryPluginRecord) => void;
  installBlocked: boolean;
  upgradeBlocked: boolean;
  emptyMessage: string;
}) {
  const t = useTranslate();
  const nameColumnClass =
    showActions === "installed" ? "min-w-[360px] w-[34%]" : "min-w-[420px] w-[42%]";
  const typeColumnClass = "min-w-[150px] text-center";
  const versionColumnClass = "min-w-[150px] text-center";
  const sizeColumnClass = "w-28 min-w-[112px] text-center";
  const statusColumnClass =
    showActions === "installed" ? "min-w-[180px] text-center" : "min-w-[200px] text-center";
  const enabledColumnClass = "w-24 min-w-[96px] text-center";
  const actionsColumnClass =
    showActions === "installed" ? "w-40 min-w-[160px] text-center" : "w-28 min-w-[112px] text-center";
  const tableMinWidthClass =
    showActions === "installed" ? "min-w-[1220px]" : "min-w-[1140px]";

  if (plugins.length === 0) {
    return <p className={`${PLUGIN_PANEL_BODY_CLASS} text-sm ${PLUGIN_MUTED_TEXT_CLASS}`}>{emptyMessage}</p>;
  }

  return (
    <Table
      id={showActions === "installed" ? "settings-plugins-installed-table" : "settings-plugins-available-table"}
      data-plugin-table={showActions}
      overflow="auto"
      layout="auto"
      density="dense"
      className={tableMinWidthClass}
    >
      <TableHeader>
        <TableRow className="border-[var(--scry-border3)] bg-[var(--scry-inset)] hover:bg-[var(--scry-inset)]">
          <TableHead className={cn(nameColumnClass, PLUGIN_TABLE_HEADER_CELL_CLASS)}>{t("label.name")}</TableHead>
          <TableHead className={cn(typeColumnClass, PLUGIN_TABLE_HEADER_CELL_CLASS)}>{t("label.type")}</TableHead>
          <TableHead className={cn(versionColumnClass, PLUGIN_TABLE_HEADER_CELL_CLASS)}>{t("label.version")}</TableHead>
          <TableHead className={cn(sizeColumnClass, PLUGIN_TABLE_HEADER_CELL_CLASS)}>{t("queue.size")}</TableHead>
          <TableHead className={cn(statusColumnClass, PLUGIN_TABLE_HEADER_CELL_CLASS)}>{t("label.status")}</TableHead>
          {showActions === "installed" && (
            <TableHead className={cn(enabledColumnClass, PLUGIN_TABLE_HEADER_CELL_CLASS)}>{t("label.enabled")}</TableHead>
          )}
          <TableActionsHead className={cn(actionsColumnClass, PLUGIN_TABLE_HEADER_CELL_CLASS)}>{t("label.actions")}</TableActionsHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {plugins.map((plugin) => {
          const progress = pluginProgress[plugin.id];
          const runningProgress = isRunningPluginProgress(progress) ? progress : undefined;
          const isBusy = mutatingPluginIds.includes(plugin.id) || plugin.installInProgress;
          const isUpgrading =
            (runningProgress?.operationKind === "UPGRADE")
            || (plugin.installInProgress && showActions === "installed");
          const sourceLink = plugin.sourceRepo;
          const normalizedSourceLink = normalizePluginLink(sourceLink);
          const normalizedDocsLink = normalizePluginLink(plugin.docsUrl);
          const showDocsLink =
            normalizedDocsLink !== null && normalizedDocsLink !== normalizedSourceLink;
          const actionError = pluginErrors[plugin.id];
          const displayVersion =
            showActions === "installed" && plugin.installedVersion
              ? plugin.installedVersion
              : plugin.version;
          const sameVersionOptimizedUpgrade =
            plugin.updateAvailable
            && plugin.isInstalled
            && plugin.installedVersion === plugin.version;
          const bytesLabel = formatPluginBytes(plugin.bytes);
          return (
            <TableRow
              key={plugin.id}
              id={selectorId("settings-plugin-row", plugin.name)}
              data-plugin-table={showActions}
              data-plugin-installed={plugin.isInstalled ? "true" : "false"}
              data-plugin-enabled={plugin.isEnabled ? "true" : "false"}
              data-plugin-update-available={plugin.updateAvailable ? "true" : "false"}
              data-ui="settings-plugin-table-row"
              className={PLUGIN_TABLE_ROW_CLASS}
            >
              <TableCell className={cn(nameColumnClass, PLUGIN_TABLE_CELL_CLASS)}>
                <div className="flex min-w-0 items-center gap-3">
                  <PluginLogo
                    id={plugin.id}
                    name={plugin.name}
                    providerType={plugin.providerType}
                    pluginType={plugin.pluginType}
                    appearance="bare"
                    className="h-10 w-10"
                  />
                  <div className="min-w-0">
                    <div className="font-medium text-[var(--scry-ink2)]">{plugin.name}</div>
                    <div className={`whitespace-normal break-words text-xs ${PLUGIN_MUTED_TEXT_CLASS}`}>
                      {plugin.description}
                    </div>
                    {(sourceLink || showDocsLink) && (
                      <div className="mt-2 flex flex-wrap items-center gap-2 text-xs">
                        {sourceLink && (
                          <a
                            href={sourceLink}
                            target="_blank"
                            rel="noreferrer"
                            className="inline-flex items-center gap-1 text-primary hover:underline"
                          >
                            {t("settings.pluginSource")}
                            <ExternalLink className="h-3 w-3" />
                          </a>
                        )}
                        {showDocsLink && plugin.docsUrl && (
                          <a
                            href={plugin.docsUrl}
                            target="_blank"
                            rel="noreferrer"
                            className="inline-flex items-center gap-1 text-primary hover:underline"
                          >
                            {t("settings.pluginDocs")}
                            <ExternalLink className="h-3 w-3" />
                          </a>
                        )}
                      </div>
                    )}
                  </div>
                </div>
              </TableCell>
              <TableCell className={cn(typeColumnClass, PLUGIN_TABLE_CELL_CLASS, "text-sm text-[var(--scry-ink2)]")}>{categoryLabel(plugin.pluginType, t)}</TableCell>
              <TableCell className={cn(versionColumnClass, PLUGIN_TABLE_CELL_CLASS, "text-sm")}>
                {t("settings.pluginVersion", { version: displayVersion })}
                {plugin.updateAvailable && (
                  <div className="text-xs text-[var(--scry-warning-text)]">
                    {sameVersionOptimizedUpgrade
                      ? t("settings.pluginOptimizedBuildAvailable")
                      : t("settings.pluginUpdateAvailable", { version: plugin.version })}
                  </div>
                )}
                {actionError && (
                  <div className="text-xs text-destructive">
                    {actionError}
                  </div>
                )}
              </TableCell>
              <TableCodeCell
                className={cn(
                  sizeColumnClass,
                  PLUGIN_TABLE_CELL_CLASS,
                  "text-sm",
                  PLUGIN_MUTED_TEXT_CLASS,
                )}
                title={plugin.bytes != null ? `${plugin.bytes} bytes` : undefined}
              >
                {bytesLabel ?? "—"}
              </TableCodeCell>
              <TableCell className={cn(statusColumnClass, PLUGIN_TABLE_CELL_CLASS)}>
                <div className="flex flex-wrap items-center justify-center gap-2">
                  {plugin.builtin && (
                    <Badge tone="info">{t("settings.pluginBuiltin")}</Badge>
                  )}
                  {plugin.official && (
                    <Badge tone="info">{t("settings.pluginOfficial")}</Badge>
                  )}
                  {plugin.supportTier === "verified_community" && (
                    <Badge tone="positive">
                      {t("settings.pluginVerifiedCommunity")}
                    </Badge>
                  )}
                  {plugin.supportTier === "unverified" && (
                    <Badge tone="warning">{t("settings.pluginUnverified")}</Badge>
                  )}
                  {plugin.status === "beta" && (
                    <Badge tone="warning">{t("settings.pluginBeta")}</Badge>
                  )}
                  {plugin.status === "deprecated" && (
                    <Badge tone="negative">{t("settings.pluginDeprecated")}</Badge>
                  )}
                  {isDownloadedBuiltinOverride(plugin) && (
                    <Badge tone="warning">{t("settings.pluginOverride")}</Badge>
                  )}
                </div>
              </TableCell>
              {showActions === "installed" && (
                <TableCell className={cn(enabledColumnClass, PLUGIN_TABLE_CELL_CLASS)}>
                  <RenderBooleanIcon
                    value={plugin.isEnabled}
                    label={`${t("label.enabled")}: ${plugin.name}`}
                  />
                </TableCell>
              )}
              <TableActionsCell className={cn(actionsColumnClass, PLUGIN_TABLE_CELL_CLASS)}>
                {showActions === "installed" ? (
                  <div className="mx-auto flex w-full min-w-0 flex-col items-center gap-2">
                    <div className="flex w-full flex-wrap items-center justify-center gap-1">
                      <PluginActionButton
                        id={selectorId("settings-plugin-toggle", plugin.name)}
                        tone={plugin.isEnabled ? "disabled" : "enabled"}
                        disabled={isBusy}
                        onClick={() => onTogglePlugin(plugin)}
                        label={plugin.isEnabled ? t("label.disable") : t("label.enable")}
                      >
                        {plugin.isEnabled ? (
                          <PowerOff className="h-4 w-4" />
                        ) : (
                          <Power className="h-4 w-4" />
                        )}
                      </PluginActionButton>
                      {plugin.updateAvailable && (
                        <PluginActionButton
                          id={selectorId("settings-plugin-upgrade", plugin.name)}
                          tone="upgrade"
                          disabled={isBusy || upgradeBlocked}
                          onClick={() => onUpgradePlugin(plugin)}
                          label={
                            sameVersionOptimizedUpgrade
                              ? t("settings.pluginInstallOptimizedBuild")
                              : t("settings.pluginUpgrade", { version: plugin.version })
                          }
                        >
                          {isUpgrading ? (
                            <Loader2 className="h-4 w-4 animate-spin" />
                          ) : (
                            <ArrowUpCircle className="h-4 w-4" />
                          )}
                        </PluginActionButton>
                      )}
                      {canUninstallPlugin(plugin) && (
                        <PluginActionButton
                          id={selectorId("settings-plugin-uninstall", plugin.name)}
                          tone="delete"
                          disabled={isBusy}
                          onClick={() => onUninstallPlugin(plugin)}
                          label={uninstallLabel(plugin, t)}
                        >
                          <Trash2 className="h-4 w-4" />
                        </PluginActionButton>
                      )}
                    </div>
                    {runningProgress && (
                      <PluginInstallProgressBar
                        progress={runningProgress}
                        id={selectorId("settings-plugin-progress", plugin.name)}
                        className="w-full space-y-1 overflow-hidden"
                      />
                    )}
                  </div>
                ) : (
                  <div className="mx-auto flex w-full min-w-0 flex-col items-center gap-2">
                    <div className="min-w-0 self-stretch">
                      {runningProgress ? (
                        <PluginInstallProgressBar progress={runningProgress} />
                      ) : null}
                    </div>
                    <div className="flex justify-center">
                      <PluginActionButton
                        id={selectorId("settings-plugin-install", plugin.name)}
                        tone="install"
                        disabled={isBusy || installBlocked}
                        onClick={() => onInstallPlugin(plugin)}
                        label={t("settings.pluginInstall")}
                      >
                        {isBusy ? (
                          <Loader2 className="h-4 w-4 animate-spin" />
                        ) : (
                          <Download className="h-4 w-4" />
                        )}
                      </PluginActionButton>
                    </div>
                  </div>
                )}
              </TableActionsCell>
            </TableRow>
          );
        })}
      </TableBody>
    </Table>
  );
}

export function SettingsPluginsSection({
  plugins,
  catalogStatus,
  initialLoading,
  mutatingPluginIds,
  pluginProgress,
  pluginErrors,
  refreshing,
  upgradingAll,
  manualRepoUrl,
  manualFileName,
  manualPreview,
  manualBusy,
  showManualInstall,
  headerActionsTarget,
  autoUpdateEnabled,
  autoUpdateLoading,
  autoUpdateSaving,
  remoteActionsBlocked,
  onAutoUpdateEnabledChange,
  onManualRepoUrlChange,
  onToggleManualInstall,
  onManualPluginFileChange,
  onInspectManualPluginRepo,
  onRequestInstallUploadedPlugin,
  onInstallManualPlugin,
  onRefreshRegistry,
  onUpgradeAllPlugins,
  onTogglePlugin,
  onInstallPlugin,
  onUninstallPlugin,
  onUpgradePlugin,
}: SettingsPluginsSectionProps) {
  const t = useTranslate();
  const [installedFilters, setInstalledFilters] = useState<FilterState>({
    category: "all",
    officialOnly: false,
  });
  const manualPluginFileInputRef = useRef<HTMLInputElement | null>(null);
  const [availableFilters, setAvailableFilters] = useState<FilterState>({
    category: "all",
    officialOnly: false,
  });

  const installed = useMemo(() => plugins.filter((p) => p.isInstalled), [plugins]);
  const available = useMemo(() => plugins.filter((p) => !p.isInstalled), [plugins]);
  const enabledInstalled = useMemo(
    () => installed.filter((p) => p.isEnabled),
    [installed],
  );
  const runtimeMemoryBytes = useMemo(
    () => enabledInstalled.reduce((total, plugin) => total + (plugin.bytes ?? 0), 0),
    [enabledInstalled],
  );
  const runtimeMemoryLabel = formatPluginBytes(runtimeMemoryBytes) ?? "—";
  const allCategories = useMemo(
    () => [...new Set(plugins.map((p) => p.pluginType))].sort(),
    [plugins],
  );

  const filteredInstalled = useMemo(
    () => applyFilters(installed, installedFilters),
    [installed, installedFilters],
  );
  const filteredAvailable = useMemo(
    () => applyFilters(available, availableFilters),
    [available, availableFilters],
  );

  const upgradeCount = installed.filter((p) => p.updateAvailable).length;
  const toolbar = (
    <div className="flex min-h-10 flex-wrap items-center justify-end gap-2 sm:min-w-[29rem]">
      <TextActionButton
        id="settings-plugins-upgrade-all"
        tone="upgrade"
        size="sm"
        disabled={upgradingAll || remoteActionsBlocked.upgrade || upgradeCount === 0}
        onClick={onUpgradeAllPlugins}
        leadingIcon={<ArrowUpCircle className={`h-4 w-4 ${upgradingAll ? "animate-spin" : ""}`} />}
      >
        {upgradingAll ? t("settings.pluginsUpdatingAll") : t("settings.pluginsUpdateAll")}
        {upgradeCount > 0 ? (
          <Badge
            tone="info"
            className="ml-2 h-5 min-w-5 rounded-full px-1.5 text-[11px]"
          >
            {upgradeCount}
          </Badge>
        ) : null}
      </TextActionButton>
      <Button
        id="settings-plugins-manual-toggle"
        variant="outline"
        size="sm"
        className="h-9"
        onClick={onToggleManualInstall}
      >
        {t("settings.pluginInstallManually")}
      </Button>
      <Button
        id="settings-plugins-refresh"
        variant="outline"
        size="sm"
        className="h-9"
        disabled={refreshing || remoteActionsBlocked.refresh}
        onClick={onRefreshRegistry}
      >
        <RefreshCw className={`mr-2 h-4 w-4 ${refreshing ? "animate-spin" : ""}`} />
        {refreshing ? t("label.refreshing") : t("settings.pluginsRefresh")}
      </Button>
    </div>
  );

  return (
    <div className="space-y-4 text-sm">
      {headerActionsTarget ? (
        createPortal(toolbar, headerActionsTarget)
      ) : null}

      {catalogStatus?.outageMessage && (
        <div className="rounded-lg border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] p-3 text-sm text-[var(--scry-warning-text)]">
          {catalogStatus.outageMessage}
        </div>
      )}

      {catalogStatus && catalogStatus.restoreWarnings.length > 0 && (
        <div className="rounded-lg border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] p-3 text-sm text-[var(--scry-warning-text)]">
          <div className="font-medium">Restore warnings</div>
          <ul className="mt-2 list-disc space-y-1 pl-5">
            {catalogStatus.restoreWarnings.map((warning) => (
              <li key={warning}>{warning}</li>
            ))}
          </ul>
        </div>
      )}

      <section className={PLUGIN_PANEL_CLASS} id="settings-plugins-auto-update">
        <div className={`${PLUGIN_PANEL_BODY_CLASS} flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between`}>
          <div className="space-y-1">
            <Label htmlFor="settings-plugins-auto-update-toggle">
              {t("settings.pluginAutoUpdateEnabled")}
            </Label>
            <p className={`text-xs ${PLUGIN_MUTED_TEXT_CLASS}`}>
              {t("settings.pluginAutoUpdateEnabledHelp")}
            </p>
          </div>
          <div className="flex items-center gap-2">
            {autoUpdateSaving ? (
              <Loader2 className={`h-4 w-4 animate-spin ${PLUGIN_MUTED_TEXT_CLASS}`} />
            ) : null}
            <SettingsToggleSwitch
              id="settings-plugins-auto-update-toggle"
              checked={autoUpdateEnabled}
              disabled={autoUpdateLoading || autoUpdateSaving}
              ariaLabel={t("settings.pluginAutoUpdateEnabled")}
              onChange={onAutoUpdateEnabledChange}
            />
          </div>
        </div>
      </section>

      {initialLoading ? (
        <div className={`${PLUGIN_PANEL_CLASS} flex min-h-48 items-center justify-center`}>
          <div className={`flex items-center gap-3 text-sm ${PLUGIN_MUTED_TEXT_CLASS}`}>
            <Loader2 className="h-5 w-5 animate-spin" />
            <span>{t("label.loading")}</span>
          </div>
        </div>
      ) : null}

      {!initialLoading && showManualInstall ? (
        <section className={PLUGIN_PANEL_CLASS}>
          <div className={PLUGIN_PANEL_HEADER_CLASS}>
            <h2 className={PLUGIN_PANEL_TITLE_CLASS}>
              {t("settings.pluginInstallManually")}
            </h2>
          </div>
          <div className={`${PLUGIN_PANEL_BODY_CLASS} space-y-4`}>
            <div className={`${PLUGIN_INSET_CLASS} p-4`}>
              <div className="space-y-1">
                <h3 className="text-sm font-semibold text-[var(--scry-ink2)]">{t("settings.pluginManualUploadTitle")}</h3>
                <p className={`text-sm ${PLUGIN_MUTED_TEXT_CLASS}`}>
                  {t("settings.pluginManualUploadHelp")}
                </p>
              </div>
              <input
                id="settings-plugins-manual-file-input"
                ref={manualPluginFileInputRef}
                type="file"
                accept=".wasm,.zst"
                className="hidden"
                onChange={onManualPluginFileChange}
              />
              <div className="mt-3 flex flex-col gap-3 md:flex-row md:items-center">
                <Button
                  id="settings-plugins-manual-file-select"
                  type="button"
                  variant="outline"
                  onClick={() => manualPluginFileInputRef.current?.click()}
                  disabled={manualBusy}
                >
                  <Upload className="mr-2 h-4 w-4" />
                  {t("settings.pluginManualUploadSelect")}
                </Button>
                <Button
                  id="settings-plugins-manual-file-install"
                  type="button"
                  disabled={manualBusy || !manualFileName}
                  onClick={onRequestInstallUploadedPlugin}
                >
                  {manualBusy ? t("label.loading") : t("settings.pluginManualUploadInstall")}
                </Button>
              </div>
              <div className={`mt-3 space-y-1 text-sm ${PLUGIN_MUTED_TEXT_CLASS}`}>
                <p id="settings-plugins-manual-file-name">
                  {manualFileName ?? t("settings.pluginManualUploadNoFile")}
                </p>
                <p>{t("settings.pluginManualUploadFormats")}</p>
              </div>
              {pluginErrors.__manualUpload && (
                <p className="text-sm text-destructive">{pluginErrors.__manualUpload}</p>
              )}
            </div>

            <div className={`${PLUGIN_INSET_CLASS} p-4`}>
              <div className="space-y-1">
                <h3 className="text-sm font-semibold text-[var(--scry-ink2)]">{t("settings.pluginManualRepoTitle")}</h3>
                <p className={`text-sm ${PLUGIN_MUTED_TEXT_CLASS}`}>
                  {t("settings.pluginManualRepoHelp")}
                </p>
              </div>
              <div className="mt-3 flex flex-col gap-3 md:flex-row md:items-end">
                <label className="flex-1 space-y-1 text-sm">
                  <span className="font-medium text-[var(--scry-ink2)]">{t("settings.pluginManualRepoUrl")}</span>
                  <Input
                    id="settings-plugins-manual-repo-url"
                    value={manualRepoUrl}
                    onChange={(event) => onManualRepoUrlChange(event.target.value)}
                    placeholder="https://github.com/example/scryer-plugin-example"
                  />
                </label>
                <Button
                  id="settings-plugins-manual-repo-inspect"
                  type="button"
                  disabled={manualBusy || remoteActionsBlocked.inspectManual || !manualRepoUrl.trim()}
                  onClick={onInspectManualPluginRepo}
                >
                  {manualBusy ? t("label.loading") : t("settings.pluginInspectManual")}
                </Button>
              </div>
            </div>
            {pluginErrors.__manual ? (
              <p className="text-sm text-destructive">{pluginErrors.__manual}</p>
            ) : null}
            {manualPreview ? (
              <div className={`${PLUGIN_INSET_CLASS} flex flex-col gap-3 p-4 md:flex-row md:items-center md:justify-between`}>
                <div>
                  <div className="font-medium text-[var(--scry-ink2)]">{manualPreview.plugin.name}</div>
                  <div className={`text-sm ${PLUGIN_MUTED_TEXT_CLASS}`}>
                    {manualPreview.plugin.description}
                  </div>
                  <div className="mt-1 text-xs text-[var(--scry-warning-text)]">
                    {t("settings.pluginUnverified")}
                  </div>
                </div>
                <Button
                  id="settings-plugins-manual-repo-install"
                  type="button"
                  disabled={manualBusy || remoteActionsBlocked.installManual}
                  onClick={onInstallManualPlugin}
                >
                  {t("settings.pluginInstall")}
                </Button>
              </div>
            ) : null}
          </div>
        </section>
      ) : null}

      {!initialLoading && plugins.length === 0 ? (
        <div className={PLUGIN_PANEL_CLASS}>
          <p className={`${PLUGIN_PANEL_BODY_CLASS} text-sm ${PLUGIN_MUTED_TEXT_CLASS}`}>
            {t("settings.pluginsNoPlugins")}
          </p>
        </div>
      ) : !initialLoading ? (
        <>
          <section className={PLUGIN_PANEL_CLASS}>
            <div className={`${PLUGIN_PANEL_HEADER_CLASS} flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between`}>
              <div className="flex flex-wrap items-baseline gap-2">
                <h2 className={PLUGIN_PANEL_TITLE_CLASS}>{t("settings.pluginsInstalled")}</h2>
                <span
                  id="settings-plugins-runtime-memory-estimate"
                  className={`rounded-full border border-[var(--scry-border3)] bg-[var(--scry-inset)] px-2 py-0.5 text-[11px] font-medium ${PLUGIN_MUTED_TEXT_CLASS}`}
                  title={`${runtimeMemoryBytes} bytes`}
                >
                  {t("settings.pluginRuntimeMemoryEstimate")}:{" "}
                  <span className="text-[var(--scry-ink2)]">{runtimeMemoryLabel}</span>
                </span>
              </div>
              <PluginFilters
                filters={installedFilters}
                categories={allCategories}
                onChange={setInstalledFilters}
              />
            </div>
            <PluginTable
              plugins={filteredInstalled}
              mutatingPluginIds={mutatingPluginIds}
              pluginProgress={pluginProgress}
              pluginErrors={pluginErrors}
              showActions="installed"
              onTogglePlugin={onTogglePlugin}
              onInstallPlugin={onInstallPlugin}
              onUninstallPlugin={onUninstallPlugin}
              onUpgradePlugin={onUpgradePlugin}
              installBlocked={remoteActionsBlocked.install}
              upgradeBlocked={remoteActionsBlocked.upgrade}
              emptyMessage={t("settings.pluginsNoInstalled")}
            />
          </section>

          <section className={PLUGIN_PANEL_CLASS}>
            <div className={`${PLUGIN_PANEL_HEADER_CLASS} flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between`}>
              <h2 className={PLUGIN_PANEL_TITLE_CLASS}>{t("settings.pluginsAvailable")}</h2>
              <PluginFilters
                filters={availableFilters}
                categories={allCategories}
                onChange={setAvailableFilters}
              />
            </div>
            <PluginTable
              plugins={filteredAvailable}
              mutatingPluginIds={mutatingPluginIds}
              pluginProgress={pluginProgress}
              pluginErrors={pluginErrors}
              showActions="available"
              onTogglePlugin={onTogglePlugin}
              onInstallPlugin={onInstallPlugin}
              onUninstallPlugin={onUninstallPlugin}
              onUpgradePlugin={onUpgradePlugin}
              installBlocked={remoteActionsBlocked.install}
              upgradeBlocked={remoteActionsBlocked.upgrade}
              emptyMessage={t("settings.pluginsNoAvailable")}
            />
          </section>
        </>
      ) : null}
    </div>
  );
}

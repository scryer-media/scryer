import { useState, useMemo } from "react";
import { ArrowUpCircle, Download, Power, PowerOff, RefreshCw, Trash2 } from "lucide-react";
import { RenderBooleanIcon } from "@/components/common/boolean-icon";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useTranslate } from "@/lib/context/translate-context";
import { cn } from "@/lib/utils";
import {
  boxedActionButtonBaseClass,
  boxedActionButtonToneClass,
  type BoxedActionButtonTone,
} from "@/lib/utils/action-button-styles";

export type RegistryPluginRecord = {
  id: string;
  name: string;
  description: string;
  version: string;
  pluginType: string;
  providerType: string;
  author: string;
  official: boolean;
  builtin: boolean;
  sourceUrl: string | null;
  isInstalled: boolean;
  isEnabled: boolean;
  installedVersion: string | null;
  updateAvailable: boolean;
};

type SettingsPluginsSectionProps = {
  plugins: RegistryPluginRecord[];
  mutatingPluginId: string | null;
  refreshing: boolean;
  onRefreshRegistry: () => void;
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

function PluginActionButton({
  label,
  tone,
  className,
  children,
  ...props
}: React.ComponentProps<typeof Button> & {
  label: string;
  tone: Extract<BoxedActionButtonTone, "install" | "upgrade" | "enabled" | "disabled" | "delete">;
}) {
  return (
    <Button
      type="button"
      size="icon-sm"
      variant="secondary"
      title={label}
      aria-label={label}
      className={cn(
        boxedActionButtonBaseClass,
        boxedActionButtonToneClass[tone],
        className,
      )}
      {...props}
    >
      {children}
    </Button>
  );
}

function categoryLabel(pluginType: string, t: Translate): string {
  switch (pluginType) {
    case "indexer": return t("settings.pluginCategoryIndexer");
    case "download_client": return t("settings.pluginCategoryDownloadClient");
    case "notification": return t("settings.pluginCategoryNotification");
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
  onChange,
}: {
  filters: FilterState;
  categories: string[];
  onChange: (filters: FilterState) => void;
}) {
  const t = useTranslate();
  return (
    <div className="flex items-center gap-3">
      <Select
        value={filters.category}
        onValueChange={(v) => onChange({ ...filters, category: v })}
      >
        <SelectTrigger className="h-8 w-44 text-sm">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="all">{t("settings.pluginAllCategories")}</SelectItem>
          {categories.map((cat) => (
            <SelectItem key={cat} value={cat}>
              {categoryLabel(cat, t)}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      <label className="flex cursor-pointer select-none items-center gap-1.5 text-sm text-muted-foreground">
        <Checkbox
          checked={filters.officialOnly}
          onCheckedChange={(checked) => onChange({ ...filters, officialOnly: !!checked })}
        />
        {t("settings.pluginOfficialOnly")}
      </label>
    </div>
  );
}

function PluginTable({
  plugins,
  mutatingPluginId,
  showActions,
  onTogglePlugin,
  onInstallPlugin,
  onUninstallPlugin,
  onUpgradePlugin,
  emptyMessage,
}: {
  plugins: RegistryPluginRecord[];
  mutatingPluginId: string | null;
  showActions: "installed" | "available";
  onTogglePlugin: (plugin: RegistryPluginRecord) => void;
  onInstallPlugin: (plugin: RegistryPluginRecord) => void;
  onUninstallPlugin: (plugin: RegistryPluginRecord) => void;
  onUpgradePlugin: (plugin: RegistryPluginRecord) => void;
  emptyMessage: string;
}) {
  const t = useTranslate();
  if (plugins.length === 0) {
    return <p className="py-4 text-sm text-muted-foreground">{emptyMessage}</p>;
  }

  return (
    <Table>
      <TableHeader>
        <TableRow>
          <TableHead>{t("label.name")}</TableHead>
          <TableHead>{t("label.type")}</TableHead>
          <TableHead>{t("label.version")}</TableHead>
          <TableHead>{t("label.status")}</TableHead>
          {showActions === "installed" && <TableHead>{t("label.enabled")}</TableHead>}
          <TableHead className="text-right">{t("label.actions")}</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {plugins.map((plugin) => {
          const isBusy = mutatingPluginId === plugin.id;
          const displayVersion =
            showActions === "installed" && plugin.installedVersion
              ? plugin.installedVersion
              : plugin.version;
          return (
            <TableRow key={plugin.id}>
              <TableCell>
                <div>
                  <div className="font-medium">{plugin.name}</div>
                  <div className="max-w-[300px] truncate text-xs text-muted-foreground">
                    {plugin.description}
                  </div>
                </div>
              </TableCell>
              <TableCell className="text-sm">{categoryLabel(plugin.pluginType, t)}</TableCell>
              <TableCell className="text-sm">
                {t("settings.pluginVersion", { version: displayVersion })}
                {plugin.updateAvailable && (
                  <div className="text-xs text-yellow-400">
                    {t("settings.pluginUpdateAvailable", { version: plugin.version })}
                  </div>
                )}
              </TableCell>
              <TableCell>
                <div className="flex items-center gap-2">
                  {plugin.builtin && (
                    <span className="rounded bg-blue-900/40 px-1.5 py-0.5 text-xs text-blue-300">
                      {t("settings.pluginBuiltin")}
                    </span>
                  )}
                  {plugin.official && (
                    <span className="rounded bg-purple-900/40 px-1.5 py-0.5 text-xs text-purple-300">
                      {t("settings.pluginOfficial")}
                    </span>
                  )}
                </div>
              </TableCell>
              {showActions === "installed" && (
                <TableCell className="text-center">
                  <RenderBooleanIcon
                    value={plugin.isEnabled}
                    label={`${t("label.enabled")}: ${plugin.name}`}
                  />
                </TableCell>
              )}
              <TableCell className="text-right">
                <div className="flex items-center justify-end gap-1">
                  {showActions === "installed" ? (
                    <>
                      <PluginActionButton
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
                          tone="upgrade"
                          disabled={isBusy}
                          onClick={() => onUpgradePlugin(plugin)}
                          label={t("settings.pluginUpgrade", { version: plugin.version })}
                        >
                          <ArrowUpCircle className="h-4 w-4" />
                        </PluginActionButton>
                      )}
                      {!plugin.builtin && (
                        <PluginActionButton
                          tone="delete"
                          disabled={isBusy}
                          onClick={() => onUninstallPlugin(plugin)}
                          label={t("settings.pluginUninstall")}
                        >
                          <Trash2 className="h-4 w-4" />
                        </PluginActionButton>
                      )}
                    </>
                  ) : (
                    <PluginActionButton
                      tone="install"
                      disabled={isBusy}
                      onClick={() => onInstallPlugin(plugin)}
                      label={t("settings.pluginInstall")}
                    >
                      <Download className="h-4 w-4" />
                    </PluginActionButton>
                  )}
                </div>
              </TableCell>
            </TableRow>
          );
        })}
      </TableBody>
    </Table>
  );
}

export function SettingsPluginsSection({
  plugins,
  mutatingPluginId,
  refreshing,
  onRefreshRegistry,
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
  const [availableFilters, setAvailableFilters] = useState<FilterState>({
    category: "all",
    officialOnly: false,
  });

  const installed = useMemo(() => plugins.filter((p) => p.isInstalled), [plugins]);
  const available = useMemo(() => plugins.filter((p) => !p.isInstalled), [plugins]);
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

  return (
    <div className="space-y-8">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <p className="text-sm text-muted-foreground">{t("settings.pluginsSection")}</p>
          {upgradeCount > 0 && (
            <span className="inline-flex h-5 min-w-5 items-center justify-center rounded-full bg-red-600 px-1.5 text-[11px] font-medium text-white">
              {upgradeCount}
            </span>
          )}
        </div>
        <Button variant="outline" size="sm" disabled={refreshing} onClick={onRefreshRegistry}>
          <RefreshCw className={`mr-2 h-4 w-4 ${refreshing ? "animate-spin" : ""}`} />
          {refreshing ? t("label.refreshing") : t("settings.pluginsRefresh")}
        </Button>
      </div>

      {plugins.length === 0 ? (
        <p className="py-4 text-sm text-muted-foreground">{t("settings.pluginsNoPlugins")}</p>
      ) : (
        <>
          <div className="space-y-3">
            <div className="flex items-center justify-between">
              <h3 className="text-sm font-medium">{t("settings.pluginsInstalled")}</h3>
              <PluginFilters
                filters={installedFilters}
                categories={allCategories}
                onChange={setInstalledFilters}
              />
            </div>
            <PluginTable
              plugins={filteredInstalled}
              mutatingPluginId={mutatingPluginId}
              showActions="installed"
              onTogglePlugin={onTogglePlugin}
              onInstallPlugin={onInstallPlugin}
              onUninstallPlugin={onUninstallPlugin}
              onUpgradePlugin={onUpgradePlugin}
              emptyMessage={t("settings.pluginsNoInstalled")}
            />
          </div>

          <div className="space-y-3">
            <div className="flex items-center justify-between">
              <h3 className="text-sm font-medium">{t("settings.pluginsAvailable")}</h3>
              <PluginFilters
                filters={availableFilters}
                categories={allCategories}
                onChange={setAvailableFilters}
              />
            </div>
            <PluginTable
              plugins={filteredAvailable}
              mutatingPluginId={mutatingPluginId}
              showActions="available"
              onTogglePlugin={onTogglePlugin}
              onInstallPlugin={onInstallPlugin}
              onUninstallPlugin={onUninstallPlugin}
              onUpgradePlugin={onUpgradePlugin}
              emptyMessage={t("settings.pluginsNoAvailable")}
            />
          </div>
        </>
      )}
    </div>
  );
}

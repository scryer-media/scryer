import { Fragment } from "react";
import { Download, Loader2, PlugZap, RefreshCw, Trash2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import type { RegistryPluginRecord } from "@/components/views/settings/settings-plugins-section";

interface SetupPluginsViewProps {
  t: (
    key: string,
    values?: Record<string, string | number | boolean | null | undefined>,
  ) => string;
  plugins: RegistryPluginRecord[];
  loading: boolean;
  refreshing: boolean;
  mutatingPluginId: string | null;
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

export function SetupPluginsView({
  t,
  plugins,
  loading,
  refreshing,
  mutatingPluginId,
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
    <div className="flex flex-col gap-6">
      <div className="text-center">
        <h2 className="text-xl font-semibold">{t("setup.pluginsTitle")}</h2>
        <p className="mt-1 text-sm text-muted-foreground">
          {t("setup.pluginsDescription")}
        </p>
      </div>

      <div className="mx-auto w-full max-w-5xl rounded-xl border border-dashed border-border bg-muted/30 px-4 py-3 text-sm">
        <span className="font-medium">{t("setup.pluginsBuiltInTitle")}:</span>{" "}
        <span className="text-muted-foreground">
          {t("setup.pluginsBuiltInDescription")}
        </span>
      </div>

      <div className="mx-auto flex w-full max-w-5xl items-center justify-between gap-4">
        <div>
          <p className="text-sm font-medium">
            {t("setup.pluginsAvailableHeading")}
          </p>
          <p className="text-sm text-muted-foreground">
            {t("setup.pluginsAvailableHint")}
          </p>
        </div>
        <Button
          variant="outline"
          size="sm"
          disabled={refreshing || loading}
          onClick={onRefreshRegistry}
        >
          {refreshing ? (
            <Loader2 className="mr-2 h-4 w-4 animate-spin" />
          ) : (
            <RefreshCw className="mr-2 h-4 w-4" />
          )}
          {refreshing ? t("label.refreshing") : t("label.refresh")}
        </Button>
      </div>

      {error && (
        <p className="mx-auto w-full max-w-5xl text-sm text-destructive">
          {error}
        </p>
      )}

      {loading ? (
        <div className="mx-auto flex w-full max-w-5xl items-center justify-center gap-2 rounded-xl border border-dashed border-border py-10 text-sm text-muted-foreground">
          <Loader2 className="h-4 w-4 animate-spin" />
          {t("label.loading")}
        </div>
      ) : (
        <div className="mx-auto w-full max-w-5xl">
          {groupedPlugins.length === 0 ? (
            <div className="rounded-xl border border-dashed border-border py-10 text-center text-sm text-muted-foreground">
              {t("setup.pluginsNoneFound")}
            </div>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>{t("label.name")}</TableHead>
                  <TableHead className="w-[140px] text-right">
                    {t("label.actions")}
                  </TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {groupedPlugins.map((group) => (
                  <Fragment key={group.key}>
                    <TableRow className="bg-muted/35 hover:bg-muted/35">
                      <TableCell colSpan={2}>
                        <span className="text-xs font-semibold uppercase tracking-[0.16em] text-muted-foreground">
                          {group.label}
                        </span>
                      </TableCell>
                    </TableRow>
                    {group.plugins.map((plugin) => {
                      const isBusy = mutatingPluginId === plugin.id;
                      return (
                        <TableRow key={plugin.id}>
                          <TableCell className="min-w-[260px]">
                            <div className="space-y-1">
                              <span className="font-medium">{plugin.name}</span>
                              <p className="text-xs text-muted-foreground">
                                {plugin.description}
                              </p>
                            </div>
                          </TableCell>
                          <TableCell className="text-right">
                            {plugin.isInstalled ? (
                              <div className="flex items-center justify-end gap-2">
                                <span className="text-sm text-muted-foreground">
                                  {t("settings.pluginInstalled")}
                                </span>
                                {!plugin.builtin && (
                                  <Button
                                    variant="ghost"
                                    size="icon"
                                    disabled={isBusy}
                                    onClick={() => onUninstallPlugin(plugin)}
                                    title={t("settings.pluginUninstall")}
                                  >
                                    {isBusy ? (
                                      <Loader2 className="h-4 w-4 animate-spin text-destructive" />
                                    ) : (
                                      <Trash2 className="h-4 w-4 text-destructive" />
                                    )}
                                  </Button>
                                )}
                              </div>
                            ) : (
                              <Button
                                variant="outline"
                                size="sm"
                                disabled={isBusy}
                                onClick={() => onInstallPlugin(plugin)}
                              >
                                {isBusy ? (
                                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                                ) : (
                                  <Download className="mr-2 h-4 w-4" />
                                )}
                                {isBusy
                                  ? t("settings.pluginInstalling")
                                  : t("settings.pluginInstall")}
                              </Button>
                            )}
                          </TableCell>
                        </TableRow>
                      );
                    })}
                  </Fragment>
                ))}
              </TableBody>
            </Table>
          )}
        </div>
      )}

      <div className="flex items-center justify-between pt-2">
        <Button variant="ghost" onClick={onBack}>
          {t("setup.back")}
        </Button>
        <Button onClick={onNext}>
          <PlugZap className="mr-2 h-4 w-4" />
          {t("setup.next")}
        </Button>
      </div>
    </div>
  );
}

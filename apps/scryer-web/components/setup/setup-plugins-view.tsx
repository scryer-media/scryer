import { Check, Download, Loader2, PlugZap, RefreshCw } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
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
  switch (pluginType) {
    case "indexer":
      return t("settings.pluginCategoryIndexer");
    case "download_client":
      return t("settings.pluginCategoryDownloadClient");
    case "notification":
      return t("settings.pluginCategoryNotification");
    default:
      return pluginType;
  }
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
  onNext,
  onBack,
}: SetupPluginsViewProps) {
  const acquisitionPlugins = plugins
    .filter((plugin) => plugin.pluginType === "download_client" || plugin.pluginType === "indexer")
    .sort((left, right) => left.name.localeCompare(right.name));
  const installedPlugins = acquisitionPlugins.filter((plugin) => plugin.isInstalled);
  const availablePlugins = acquisitionPlugins.filter((plugin) => !plugin.isInstalled);

  return (
    <div className="flex flex-col gap-6">
      <div className="text-center">
        <h2 className="text-xl font-semibold">{t("setup.pluginsTitle")}</h2>
        <p className="mt-1 text-sm text-muted-foreground">{t("setup.pluginsDescription")}</p>
      </div>

      <Card className="mx-auto w-full max-w-3xl border-dashed bg-muted/30">
        <CardContent className="flex flex-col gap-4 p-5 md:flex-row md:items-center md:justify-between">
          <div>
            <p className="font-medium">{t("setup.pluginsBuiltInTitle")}</p>
            <p className="text-sm text-muted-foreground">{t("setup.pluginsBuiltInDescription")}</p>
          </div>
          <div className="flex flex-wrap gap-2">
            <span className="rounded-full border border-border bg-background px-3 py-1 text-sm">
              NZBGet
            </span>
            <span className="rounded-full border border-border bg-background px-3 py-1 text-sm">
              SABnzbd
            </span>
          </div>
        </CardContent>
      </Card>

      <div className="mx-auto flex w-full max-w-3xl items-center justify-between gap-4">
        <div>
          <p className="text-sm font-medium">{t("setup.pluginsAvailableHeading")}</p>
          <p className="text-sm text-muted-foreground">{t("setup.pluginsAvailableHint")}</p>
        </div>
        <Button variant="outline" size="sm" disabled={refreshing || loading} onClick={onRefreshRegistry}>
          {refreshing ? (
            <Loader2 className="mr-2 h-4 w-4 animate-spin" />
          ) : (
            <RefreshCw className="mr-2 h-4 w-4" />
          )}
          {refreshing ? t("label.refreshing") : t("label.refresh")}
        </Button>
      </div>

      {error && (
        <p className="mx-auto w-full max-w-3xl text-sm text-destructive">{error}</p>
      )}

      {loading ? (
        <div className="mx-auto flex w-full max-w-3xl items-center justify-center gap-2 rounded-xl border border-dashed border-border py-10 text-sm text-muted-foreground">
          <Loader2 className="h-4 w-4 animate-spin" />
          {t("status.loading")}
        </div>
      ) : (
        <div className="mx-auto grid w-full max-w-3xl gap-4">
          {installedPlugins.length > 0 && (
            <div className="space-y-3">
              <p className="text-sm font-medium">{t("setup.pluginsInstalledHeading")}</p>
              <div className="grid gap-3">
                {installedPlugins.map((plugin) => (
                  <Card key={plugin.id}>
                    <CardContent className="flex flex-col gap-4 p-5 md:flex-row md:items-start md:justify-between">
                      <div className="space-y-2">
                        <div className="flex flex-wrap items-center gap-2">
                          <p className="font-medium">{plugin.name}</p>
                          <span className="rounded-full bg-emerald-500/15 px-2 py-0.5 text-xs text-emerald-600">
                            {t("settings.pluginInstalled")}
                          </span>
                          <span className="rounded-full bg-muted px-2 py-0.5 text-xs text-muted-foreground">
                            {categoryLabel(plugin.pluginType, t)}
                          </span>
                        </div>
                        <p className="text-sm text-muted-foreground">{plugin.description}</p>
                      </div>
                      <div className="flex items-center gap-2 text-sm text-emerald-600">
                        <Check className="h-4 w-4" />
                        {t("settings.pluginVersion", {
                          version: plugin.installedVersion ?? plugin.version,
                        })}
                      </div>
                    </CardContent>
                  </Card>
                ))}
              </div>
            </div>
          )}

          <div className="space-y-3">
            <p className="text-sm font-medium">{t("setup.pluginsAvailable")}</p>
            {availablePlugins.length === 0 ? (
              <div className="rounded-xl border border-dashed border-border py-10 text-center text-sm text-muted-foreground">
                {acquisitionPlugins.length === 0
                  ? t("setup.pluginsNoneFound")
                  : t("setup.pluginsNoneAvailable")}
              </div>
            ) : (
              <div className="grid gap-3">
                {availablePlugins.map((plugin) => {
                  const isBusy = mutatingPluginId === plugin.id;
                  return (
                    <Card key={plugin.id}>
                      <CardContent className="flex flex-col gap-4 p-5 md:flex-row md:items-start md:justify-between">
                        <div className="space-y-2">
                          <div className="flex flex-wrap items-center gap-2">
                            <p className="font-medium">{plugin.name}</p>
                            <span className="rounded-full bg-muted px-2 py-0.5 text-xs text-muted-foreground">
                              {categoryLabel(plugin.pluginType, t)}
                            </span>
                            {plugin.official && (
                              <span className="rounded-full bg-blue-500/15 px-2 py-0.5 text-xs text-blue-600">
                                {t("settings.pluginOfficial")}
                              </span>
                            )}
                          </div>
                          <p className="text-sm text-muted-foreground">{plugin.description}</p>
                          <p className="text-xs text-muted-foreground">
                            {t("settings.pluginVersion", { version: plugin.version })}
                          </p>
                        </div>
                        <Button
                          variant="outline"
                          disabled={isBusy}
                          onClick={() => onInstallPlugin(plugin)}
                        >
                          {isBusy ? (
                            <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                          ) : (
                            <Download className="mr-2 h-4 w-4" />
                          )}
                          {isBusy ? t("settings.pluginInstalling") : t("settings.pluginInstall")}
                        </Button>
                      </CardContent>
                    </Card>
                  );
                })}
              </div>
            )}
          </div>
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

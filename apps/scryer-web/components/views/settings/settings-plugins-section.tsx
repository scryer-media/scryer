import { Download, Power, RefreshCw, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import type { Translate } from "@/components/root/types";

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
};

type SettingsPluginsSectionProps = {
  t: Translate;
  plugins: RegistryPluginRecord[];
  mutatingPluginId: string | null;
  refreshing: boolean;
  onRefreshRegistry: () => void;
  onTogglePlugin: (plugin: RegistryPluginRecord) => void;
  onInstallPlugin: (plugin: RegistryPluginRecord) => void;
  onUninstallPlugin: (plugin: RegistryPluginRecord) => void;
};

function StatusBadge({ plugin, t }: { plugin: RegistryPluginRecord; t: Translate }) {
  if (plugin.builtin) {
    return (
      <span className="rounded bg-blue-900/40 px-1.5 py-0.5 text-xs text-blue-300">
        {t("settings.pluginBuiltin")}
      </span>
    );
  }
  if (plugin.isInstalled) {
    return (
      <span className="rounded bg-green-900/40 px-1.5 py-0.5 text-xs text-green-300">
        {t("settings.pluginInstalled")}
      </span>
    );
  }
  return (
    <span className="rounded bg-muted px-1.5 py-0.5 text-xs text-muted-foreground">
      {t("settings.pluginNotInstalled")}
    </span>
  );
}

export function SettingsPluginsSection({
  t,
  plugins,
  mutatingPluginId,
  refreshing,
  onRefreshRegistry,
  onTogglePlugin,
  onInstallPlugin,
  onUninstallPlugin,
}: SettingsPluginsSectionProps) {
  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <p className="text-sm text-muted-foreground">{t("settings.pluginsSection")}</p>
        <Button
          variant="outline"
          size="sm"
          disabled={refreshing}
          onClick={onRefreshRegistry}
        >
          <RefreshCw className={`mr-2 h-4 w-4 ${refreshing ? "animate-spin" : ""}`} />
          {refreshing ? t("settings.pluginsRefreshing") : t("settings.pluginsRefresh")}
        </Button>
      </div>

      {plugins.length === 0 ? (
        <p className="text-sm text-muted-foreground py-4">
          {t("settings.pluginsNoPlugins")}
        </p>
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>{t("label.name")}</TableHead>
              <TableHead>{t("label.version")}</TableHead>
              <TableHead>{t("label.type")}</TableHead>
              <TableHead>{t("label.status")}</TableHead>
              <TableHead>{t("label.enabled")}</TableHead>
              <TableHead className="text-right">{t("label.actions")}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {plugins.map((plugin) => {
              const isBusy = mutatingPluginId === plugin.id;
              return (
                <TableRow key={plugin.id}>
                  <TableCell>
                    <div>
                      <div className="font-medium">{plugin.name}</div>
                      <div className="text-xs text-muted-foreground max-w-[300px] truncate">
                        {plugin.description}
                      </div>
                    </div>
                  </TableCell>
                  <TableCell className="text-sm">
                    {t("settings.pluginVersion", { version: plugin.version })}
                    {plugin.isInstalled &&
                      plugin.installedVersion &&
                      plugin.installedVersion !== plugin.version && (
                        <div className="text-xs text-yellow-400">
                          {t("settings.pluginUpdateAvailable", { version: plugin.version })}
                        </div>
                      )}
                  </TableCell>
                  <TableCell className="text-sm">{plugin.providerType}</TableCell>
                  <TableCell>
                    <div className="flex items-center gap-2">
                      <StatusBadge plugin={plugin} t={t} />
                      {plugin.official && (
                        <span className="rounded bg-purple-900/40 px-1.5 py-0.5 text-xs text-purple-300">
                          {t("settings.pluginOfficial")}
                        </span>
                      )}
                    </div>
                  </TableCell>
                  <TableCell>
                    {plugin.isInstalled && (
                      <Button
                        variant="ghost"
                        size="icon"
                        disabled={isBusy}
                        onClick={() => onTogglePlugin(plugin)}
                        title={plugin.isEnabled ? t("label.disabled") : t("label.enabled")}
                      >
                        <Power
                          className={`h-4 w-4 ${plugin.isEnabled ? "text-green-400" : "text-muted-foreground"}`}
                        />
                      </Button>
                    )}
                  </TableCell>
                  <TableCell className="text-right">
                    {!plugin.isInstalled ? (
                      <Button
                        variant="outline"
                        size="sm"
                        disabled={isBusy}
                        onClick={() => onInstallPlugin(plugin)}
                      >
                        <Download className="mr-1 h-3 w-3" />
                        {t("settings.pluginInstall")}
                      </Button>
                    ) : !plugin.builtin ? (
                      <Button
                        variant="ghost"
                        size="icon"
                        disabled={isBusy}
                        onClick={() => onUninstallPlugin(plugin)}
                        title={t("settings.pluginUninstall")}
                      >
                        <Trash2 className="h-4 w-4 text-destructive" />
                      </Button>
                    ) : null}
                  </TableCell>
                </TableRow>
              );
            })}
          </TableBody>
        </Table>
      )}
    </div>
  );
}

export type PluginInstallOperationKind = "INSTALL" | "UPGRADE";

type PluginInstallState = {
  installInProgress: boolean;
  isInstalled: boolean;
  updateAvailable: boolean;
  installedVersion?: string | null;
  latestVersion?: string | null;
  version: string;
};

export function applySuccessfulPluginOperationState<
  T extends PluginInstallState,
>(plugin: T, operationKind: PluginInstallOperationKind): T {
  return {
    ...plugin,
    installInProgress: false,
    isInstalled: true,
    updateAvailable:
      operationKind === "UPGRADE" ? false : plugin.updateAvailable,
    installedVersion:
      operationKind === "UPGRADE"
        ? (plugin.latestVersion ?? plugin.version)
        : plugin.installedVersion,
  };
}

export function claimPluginTerminalOperation(
  claimedPluginIds: Set<string>,
  pluginId: string,
): boolean {
  if (claimedPluginIds.has(pluginId)) {
    return false;
  }
  claimedPluginIds.add(pluginId);
  return true;
}

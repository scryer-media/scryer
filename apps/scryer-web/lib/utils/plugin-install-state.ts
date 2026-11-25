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

/**
 * The install and upgrade operations a page is following, one per plugin.
 *
 * Each start gets a new number, so word about an earlier operation, from its
 * progress stream or from a plugin list loaded before the start, cannot end a
 * later one. Whatever ends an operation first owns winding it down.
 */
export type PluginOperations = {
  /** Follow a new operation on the plugin, replacing any earlier one. */
  begin: (pluginId: string) => number;
  isCurrent: (pluginId: string, operation: number) => boolean;
  /**
   * Stop following the operation. False when it already ended or a newer one
   * replaced it, so the caller leaves the plugin alone.
   */
  end: (pluginId: string, operation: number) => boolean;
  /** Taken just before the plugin list is loaded. */
  mark: () => number;
  /**
   * The operations the plugin list loaded after `mark` shows finished: the
   * plugin is not busy, or is no longer listed. Operations started after the
   * mark are left alone, since the list may predate them.
   */
  settledBy: (
    plugins: readonly { id: string; installInProgress: boolean }[],
    mark: number,
  ) => { pluginId: string; operation: number }[];
  size: () => number;
};

export function createPluginOperations(): PluginOperations {
  let lastOperation = 0;
  const operationByPlugin = new Map<string, number>();
  return {
    begin(pluginId) {
      lastOperation += 1;
      operationByPlugin.set(pluginId, lastOperation);
      return lastOperation;
    },
    isCurrent: (pluginId, operation) =>
      operationByPlugin.get(pluginId) === operation,
    end(pluginId, operation) {
      if (operationByPlugin.get(pluginId) !== operation) return false;
      operationByPlugin.delete(pluginId);
      return true;
    },
    mark: () => lastOperation,
    settledBy(plugins, mark) {
      const busy = new Set(
        plugins
          .filter((plugin) => plugin.installInProgress)
          .map((plugin) => plugin.id),
      );
      return [...operationByPlugin]
        .filter(([pluginId, operation]) => {
          return operation <= mark && !busy.has(pluginId);
        })
        .map(([pluginId, operation]) => ({ pluginId, operation }));
    },
    size: () => operationByPlugin.size,
  };
}

/** Waits between plugin list loads while an operation is followed. */
export const PLUGIN_OPERATION_RECONCILE_DELAYS_MS = [
  2_000, 5_000, 15_000, 30_000,
] as const;

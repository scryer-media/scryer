import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import {
  SettingsRecycleBinSection,
  type RecycledItem,
} from "@/components/views/settings/settings-recycle-bin-section";
import { useClient } from "urql";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { librariesQuery, recycledItemsQuery, recycleBinSettingsQuery } from "@/lib/graphql/queries";
import {
  restoreRecycledItemMutation,
  deleteRecycledItemMutation,
  emptyRecycleBinMutation,
  updateRecycleBinSettingsMutation,
} from "@/lib/graphql/mutations";
import { useAuth } from "@/lib/hooks/use-auth";
import { useJobRunToasts } from "@/components/root/job-run-provider";
import type { LibraryRecord } from "@/lib/types";
import { normalizeJobRun } from "@/lib/utils/job-runs";
import {
  APP_PERMISSIONS,
  LIBRARY_PERMISSIONS,
  hasAnyAppPermission,
} from "@/lib/utils/permissions";
import {
  normalizeLibraryFilterSelection,
  selectedLibraryIdsToQueryValue,
} from "@/lib/utils/library-filter";

type PendingAction = { type: "delete"; item: RecycledItem } | { type: "empty"; count: number };

type RecycleBinSettings = {
  enabled: boolean;
};

type RecycleBinSettingsQueryResult = {
  recycleBinSettings?: RecycleBinSettings | null;
};

type RecycledItemsQueryResult = {
  recycledItems?: {
    items: RecycledItem[];
    totalCount: number;
  } | null;
};

type LibrariesQueryResult = {
  libraries?: LibraryRecord[] | null;
};

type UpdateRecycleBinSettingsResult = {
  updateRecycleBinSettings?: RecycleBinSettings | null;
};

export function SettingsRecycleBinContainer() {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const { user } = useAuth();
  const canManageConfig = hasAnyAppPermission(user, [APP_PERMISSIONS.manageSystemSettings]);
  const manageTitleLibraryIds = useMemo(
    () =>
      new Set(
        (user?.libraryPermissions ?? [])
          .filter((grant) => grant.permissions.includes(LIBRARY_PERMISSIONS.manageTitles))
          .map((grant) => grant.libraryId),
      ),
    [user],
  );
  const canManageItems = manageTitleLibraryIds.size > 0;
  const [items, setItems] = useState<RecycledItem[]>([]);
  const [totalCount, setTotalCount] = useState(0);
  const [settings, setSettings] = useState<RecycleBinSettings>({ enabled: true });
  const [settingsLoading, setSettingsLoading] = useState(true);
  const [settingsSaving, setSettingsSaving] = useState(false);
  const [itemsLoading, setItemsLoading] = useState(false);
  const [libraries, setLibraries] = useState<LibraryRecord[]>([]);
  const [librariesLoading, setLibrariesLoading] = useState(false);
  const [selectedLibraryIds, setSelectedLibraryIds] = useState<string[]>([]);
  const [mutatingId, setMutatingId] = useState<string | null>(null);
  const [pendingRestoreIds, setPendingRestoreIds] = useState<Set<string>>(
    () => new Set(),
  );
  const restoreUnregistersRef = useRef(new Set<() => void>());
  const [pendingAction, setPendingAction] = useState<PendingAction | null>(null);
  const { registerInteractiveJobRun } = useJobRunToasts();

  const fetchSettings = useCallback(async () => {
    setSettingsLoading(true);
    try {
      const { data, error } = await client
        .query<RecycleBinSettingsQueryResult>(recycleBinSettingsQuery, {})
        .toPromise();
      if (error) throw error;
      setSettings(data?.recycleBinSettings ?? { enabled: true });
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
    } finally {
      setSettingsLoading(false);
    }
  }, [client, setGlobalStatus, t]);

  useEffect(() => {
    void fetchSettings();
  }, [fetchSettings]);

  useEffect(() => {
    if (!canManageItems) {
      setLibraries([]);
      setSelectedLibraryIds([]);
      setLibrariesLoading(false);
      return;
    }

    let cancelled = false;
    setLibrariesLoading(true);
    void client
      .query<LibrariesQueryResult>(
        librariesQuery,
        { facet: null, permission: "MANAGE_TITLES" },
        { requestPolicy: "network-only" },
      )
      .toPromise()
      .then(({ data, error }) => {
        if (cancelled) return;
        if (error) throw error;
        const nextLibraries = (data?.libraries ?? []).filter((library) =>
          manageTitleLibraryIds.has(library.id),
        );
        setLibraries(nextLibraries);
        setSelectedLibraryIds((current) =>
          normalizeLibraryFilterSelection(current, nextLibraries),
        );
      })
      .catch((error) => {
        if (!cancelled) {
          setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
        }
      })
      .finally(() => {
        if (!cancelled) {
          setLibrariesLoading(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [canManageItems, client, manageTitleLibraryIds, setGlobalStatus, t]);

  const fetchItems = useCallback(async () => {
    if (settingsLoading || !settings.enabled || !canManageItems) {
      setItems([]);
      setTotalCount(0);
      setItemsLoading(false);
      return;
    }

    setItemsLoading(true);
    try {
      const { data, error } = await client
        .query<RecycledItemsQueryResult>(
          recycledItemsQuery,
          { libraryIds: selectedLibraryIdsToQueryValue(selectedLibraryIds) },
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (error) throw error;
      setItems(data?.recycledItems?.items ?? []);
      setTotalCount(data?.recycledItems?.totalCount ?? 0);
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
    } finally {
      setItemsLoading(false);
    }
  }, [
    canManageItems,
    client,
    selectedLibraryIds,
    setGlobalStatus,
    settings.enabled,
    settingsLoading,
    t,
  ]);

  useEffect(() => {
    void fetchItems();
  }, [fetchItems]);

  const updateEnabled = async (enabled: boolean) => {
    if (!canManageConfig) return;
    setSettingsSaving(true);
    try {
      const { data, error } = await client
        .mutation<UpdateRecycleBinSettingsResult>(updateRecycleBinSettingsMutation, {
          input: { enabled },
        })
        .toPromise();
      if (error) throw error;
      setSettings(data?.updateRecycleBinSettings ?? { enabled });
      if (!enabled) {
        setItems([]);
        setTotalCount(0);
      }
      setGlobalStatus(t("status.recycleBinSettingsSaved"));
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setSettingsSaving(false);
    }
  };

  const restoreItem = async (item: RecycledItem) => {
    if (
      !canManageItems ||
      !manageTitleLibraryIds.has(item.libraryId) ||
      pendingRestoreIds.has(item.id)
    ) return;
    setMutatingId(item.id);
    try {
      const { data, error } = await client
        .mutation<{
          restoreRecycledItem?: { jobRun?: unknown };
        }>(restoreRecycledItemMutation, { id: item.id })
        .toPromise();
      if (error) throw error;
      const run = normalizeJobRun(data?.restoreRecycledItem?.jobRun);
      if (!run) throw new Error(t("status.apiError"));
      setPendingRestoreIds((current) => new Set(current).add(item.id));
      const unregister = registerInteractiveJobRun(run, () => {
        unregister();
        restoreUnregistersRef.current.delete(unregister);
        setPendingRestoreIds((current) => {
          const next = new Set(current);
          next.delete(item.id);
          return next;
        });
        void fetchItems();
      });
      restoreUnregistersRef.current.add(unregister);
      setGlobalStatus(`Queued restore for ${item.originalPath}.`);
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setMutatingId(null);
    }
  };

  useEffect(
    () => () => {
      for (const unregister of restoreUnregistersRef.current) {
        unregister();
      }
      restoreUnregistersRef.current.clear();
    },
    [],
  );

  const requestDelete = (item: RecycledItem) => {
    if (!canManageItems || !manageTitleLibraryIds.has(item.libraryId)) return;
    setPendingAction({ type: "delete", item });
  };

  const confirmDelete = async () => {
    if (!pendingAction || pendingAction.type !== "delete") return;
    if (!canManageItems) {
      setPendingAction(null);
      return;
    }
    const item = pendingAction.item;
    if (!manageTitleLibraryIds.has(item.libraryId)) {
      setPendingAction(null);
      return;
    }
    setMutatingId(item.id);
    try {
      const { data, error } = await client
        .mutation<{ deleteRecycledItem?: { deleted?: boolean } }>(
          deleteRecycledItemMutation,
          { id: item.id },
        )
        .toPromise();
      if (error) throw error;
      setGlobalStatus(
        data?.deleteRecycledItem?.deleted === false
          ? t("status.recycleBinQuarantined")
          : t("status.recycleBinDeleted"),
      );
      await fetchItems();
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToDelete"));
    } finally {
      setMutatingId(null);
      setPendingAction(null);
    }
  };

  const requestEmpty = () => {
    if (!canManageItems) return;
    setPendingAction({ type: "empty", count: totalCount });
  };

  const confirmEmpty = async () => {
    if (!canManageItems) {
      setPendingAction(null);
      return;
    }
    setMutatingId("__empty__");
    try {
      const { data, error } = await client
        .mutation(emptyRecycleBinMutation, {
          libraryIds: selectedLibraryIdsToQueryValue(selectedLibraryIds),
        })
        .toPromise();
      if (error) throw error;
      const count = data?.emptyRecycleBin?.purgedCount ?? 0;
      setGlobalStatus(t("status.recycleBinEmptied", { count }));
      await fetchItems();
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToDelete"));
    } finally {
      setMutatingId(null);
      setPendingAction(null);
    }
  };

  return (
    <>
      <SettingsRecycleBinSection
        enabled={settings.enabled}
        settingsLoading={settingsLoading}
        settingsSaving={settingsSaving}
        canManageConfig={canManageConfig}
        canManageItems={canManageItems}
        libraries={libraries}
        librariesLoading={librariesLoading}
        selectedLibraryIds={selectedLibraryIds}
        items={items}
        totalCount={totalCount}
        loading={itemsLoading}
        mutatingId={mutatingId}
        pendingRestoreIds={pendingRestoreIds}
        onEnabledChange={updateEnabled}
        onSelectedLibraryIdsChange={setSelectedLibraryIds}
        onRestore={restoreItem}
        onDelete={requestDelete}
        onEmptyAll={requestEmpty}
      />
      <ConfirmDialog
        open={pendingAction !== null}
        title={
          pendingAction?.type === "empty"
            ? t("settings.recycleBinEmptyAll")
            : t("settings.recycleBinDelete")
        }
        description={
          pendingAction?.type === "empty"
            ? t("settings.recycleBinEmptyConfirm", { count: pendingAction.count })
            : t("settings.recycleBinDeleteConfirm")
        }
        confirmLabel={
          pendingAction?.type === "empty"
            ? t("settings.recycleBinEmptyAll")
            : t("settings.recycleBinDelete")
        }
        cancelLabel={t("label.cancel")}
        contentId="settings-recycle-bin-confirm-dialog"
        confirmButtonId={
          pendingAction?.type === "empty"
            ? "settings-recycle-bin-empty-confirm"
            : "settings-recycle-bin-delete-confirm"
        }
        cancelButtonId="settings-recycle-bin-confirm-cancel"
        isBusy={mutatingId !== null}
        onConfirm={pendingAction?.type === "empty" ? confirmEmpty : confirmDelete}
        onCancel={() => setPendingAction(null)}
      />
    </>
  );
}

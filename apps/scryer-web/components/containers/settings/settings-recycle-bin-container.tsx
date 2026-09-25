import { useCallback, useEffect, useRef, useState } from "react";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import {
  SettingsRecycleBinSection,
  type RecycledItem,
} from "@/components/views/settings/settings-recycle-bin-section";
import { useClient } from "urql";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import {
  librariesQuery,
  previewRestoreRecycledItemsQuery,
  recycledItemsQuery,
  recycleBinSettingsQuery,
} from "@/lib/graphql/queries";
import {
  deleteRecycledItemsMutation,
  emptyRecycleBinMutation,
  restoreRecycledItemsMutation,
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
  hasAnyLibraryPermission,
  hasLibraryPermission,
} from "@/lib/utils/permissions";
import {
  normalizeLibraryFilterSelection,
  selectedLibraryIdsToQueryValue,
} from "@/lib/utils/library-filter";
import {
  buildRecycleBinSettingsInput,
  type RecycleBinSettingsChanges,
} from "@/lib/utils/recycle-bin";
import { LoadingMark } from "@/components/common/loading-mark";

const RECYCLE_BATCH_MAX_ITEMS = 250;

type RestoreConflictPolicy = "KEEP_BOTH" | "REPLACE_EXISTING";
type PendingAction =
  | {
      type: "restore";
      items: RecycledItem[];
      fingerprint: string;
      occupiedCount: number;
      conflictPolicy: RestoreConflictPolicy;
    }
  | { type: "delete"; items: RecycledItem[] }
  | { type: "empty"; count: number };

type RecycleBinSettings = {
  enabled: boolean;
  path: string | null;
  retentionDays: number;
  effectivePaths: string[];
  validationError: string | null;
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

type RestorePreviewResult = {
  previewRestoreRecycledItems?: {
    fingerprint: string;
    items: Array<{ id: string; destinationOccupied: boolean }>;
  } | null;
};

type LibrariesQueryResult = {
  libraries?: LibraryRecord[] | null;
};

type UpdateRecycleBinSettingsResult = {
  updateRecycleBinSettings?: RecycleBinSettings | null;
};

function uniqueItems(items: RecycledItem[]): RecycledItem[] {
  const byId = new Map(items.map((item) => [item.id, item]));
  return Array.from(byId.values());
}

export function SettingsRecycleBinContainer() {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const { user } = useAuth();
  const canManageConfig = hasAnyAppPermission(user, [APP_PERMISSIONS.manageSystemSettings]);
  const canManageLibraryItems = useCallback(
    (libraryId: string) =>
      hasLibraryPermission(user, libraryId, LIBRARY_PERMISSIONS.manageTitles),
    [user],
  );
  const canManageItems = hasAnyLibraryPermission(user, LIBRARY_PERMISSIONS.manageTitles);
  const [items, setItems] = useState<RecycledItem[]>([]);
  const [totalCount, setTotalCount] = useState(0);
  const [itemsRefreshRevision, setItemsRefreshRevision] = useState(0);
  const [settings, setSettings] = useState<RecycleBinSettings | null>(null);
  const [settingsLoading, setSettingsLoading] = useState(true);
  const [settingsSaving, setSettingsSaving] = useState(false);
  const [itemsLoading, setItemsLoading] = useState(false);
  const [libraries, setLibraries] = useState<LibraryRecord[]>([]);
  const [librariesLoading, setLibrariesLoading] = useState(false);
  const [selectedLibraryIds, setSelectedLibraryIds] = useState<string[]>([]);
  const [selectedItemIds, setSelectedItemIds] = useState<Set<string>>(() => new Set());
  const [pendingItemIds, setPendingItemIds] = useState<Set<string>>(() => new Set());
  const [mutatingId, setMutatingId] = useState<string | null>(null);
  const [pendingAction, setPendingAction] = useState<PendingAction | null>(null);
  const jobUnregistersRef = useRef(new Set<() => void>());
  const { registerInteractiveJobRun } = useJobRunToasts();

  const fetchSettings = useCallback(async () => {
    setSettingsLoading(true);
    try {
      const { data, error } = await client
        .query<RecycleBinSettingsQueryResult>(recycleBinSettingsQuery, {})
        .toPromise();
      if (error) throw error;
      // No controls render until the stored settings are known, so a failed
      // load can never be saved back as defaults.
      if (!data?.recycleBinSettings) throw new Error(t("status.apiError"));
      setSettings(data.recycleBinSettings);
    } catch (error) {
      setSettings(null);
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
          canManageLibraryItems(library.id),
        );
        setLibraries(nextLibraries);
        setSelectedLibraryIds((current) => normalizeLibraryFilterSelection(current, nextLibraries));
      })
      .catch((error) => {
        if (!cancelled) setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
      })
      .finally(() => {
        if (!cancelled) setLibrariesLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [canManageItems, canManageLibraryItems, client, setGlobalStatus, t]);

  const fetchItems = useCallback(async () => {
    if (settingsLoading || !settings?.enabled || !canManageItems) {
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
      const nextItems = data?.recycledItems?.items ?? [];
      const nextIds = new Set(nextItems.map((item) => item.id));
      setItems(nextItems);
      setTotalCount(data?.recycledItems?.totalCount ?? 0);
      setSelectedItemIds((current) => new Set(Array.from(current).filter((id) => nextIds.has(id))));
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
    } finally {
      setItemsLoading(false);
    }
  }, [canManageItems, client, selectedLibraryIds, setGlobalStatus, settings?.enabled, settingsLoading, t]);

  useEffect(() => {
    void fetchItems();
  }, [fetchItems, itemsRefreshRevision]);

  const saveSettings = async (changes: RecycleBinSettingsChanges) => {
    if (!canManageConfig || !settings) return;
    setSettingsSaving(true);
    try {
      const { data, error } = await client
        .mutation<UpdateRecycleBinSettingsResult>(updateRecycleBinSettingsMutation, {
          input: buildRecycleBinSettingsInput(changes),
        })
        .toPromise();
      if (error) throw error;
      const saved = data?.updateRecycleBinSettings;
      if (!saved) throw new Error(t("status.apiError"));
      setSettings(saved);
      if (!saved.enabled) {
        setItems([]);
        setTotalCount(0);
        setSelectedItemIds(new Set());
      }
      setGlobalStatus(t("status.recycleBinSettingsSaved"));
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setSettingsSaving(false);
    }
  };

  const registerBatchJob = useCallback(
    (runValue: unknown, entryIds: string[], message: string) => {
      const run = normalizeJobRun(runValue);
      if (!run) throw new Error(t("status.apiError"));
      setPendingItemIds((current) => new Set([...current, ...entryIds]));
      const unregister = registerInteractiveJobRun(run, () => {
        unregister();
        jobUnregistersRef.current.delete(unregister);
        setPendingItemIds((current) => {
          const next = new Set(current);
          for (const entryId of entryIds) next.delete(entryId);
          return next;
        });
        setItemsRefreshRevision((current) => current + 1);
      });
      jobUnregistersRef.current.add(unregister);
      setGlobalStatus(message);
    },
    [registerInteractiveJobRun, setGlobalStatus, t],
  );

  useEffect(
    () => () => {
      for (const unregister of jobUnregistersRef.current) unregister();
      jobUnregistersRef.current.clear();
    },
    [],
  );

  const validateBatchItems = (requested: RecycledItem[]): RecycledItem[] | null => {
    const targets = uniqueItems(requested).filter(
      (item) => canManageLibraryItems(item.libraryId) && !pendingItemIds.has(item.id),
    );
    if (targets.length === 0) return null;
    if (targets.length > RECYCLE_BATCH_MAX_ITEMS) {
      setGlobalStatus(t("settings.recycleBinBatchLimit", { count: RECYCLE_BATCH_MAX_ITEMS }));
      return null;
    }
    return targets;
  };

  const requestRestore = async (requested: RecycledItem[]) => {
    if (!canManageItems) return;
    const targets = validateBatchItems(requested);
    if (!targets) return;
    setMutatingId("__batch__");
    try {
      const { data, error } = await client
        .query<RestorePreviewResult>(
          previewRestoreRecycledItemsQuery,
          { ids: targets.map((item) => item.id) },
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (error) throw error;
      const preview = data?.previewRestoreRecycledItems;
      if (!preview) throw new Error(t("status.apiError"));
      setPendingAction({
        type: "restore",
        items: targets,
        fingerprint: preview.fingerprint,
        occupiedCount: preview.items.filter((item) => item.destinationOccupied).length,
        conflictPolicy: "KEEP_BOTH",
      });
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setMutatingId(null);
    }
  };

  const requestDelete = (requested: RecycledItem[]) => {
    if (!canManageItems) return;
    const targets = validateBatchItems(requested);
    if (targets) setPendingAction({ type: "delete", items: targets });
  };

  const confirmRestore = async () => {
    if (!pendingAction || pendingAction.type !== "restore") return;
    setMutatingId("__batch__");
    try {
      const { data, error } = await client
        .mutation<{ restoreRecycledItems?: { ids?: string[]; jobRun?: unknown } }>(
          restoreRecycledItemsMutation,
          {
            input: {
              ids: pendingAction.items.map((item) => item.id),
              conflictPolicy: pendingAction.conflictPolicy,
              previewFingerprint: pendingAction.fingerprint,
            },
          },
        )
        .toPromise();
      if (error) throw error;
      const payload = data?.restoreRecycledItems;
      registerBatchJob(
        payload?.jobRun,
        payload?.ids ?? pendingAction.items.map((item) => item.id),
        t(
          pendingAction.items.length === 1
            ? "settings.recycleBinRestoreQueuedOne"
            : "settings.recycleBinRestoreQueuedOther",
          { count: pendingAction.items.length },
        ),
      );
      setPendingAction(null);
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setMutatingId(null);
    }
  };

  const confirmDelete = async () => {
    if (!pendingAction || pendingAction.type !== "delete") return;
    setMutatingId("__batch__");
    try {
      const { data, error } = await client
        .mutation<{ deleteRecycledItems?: { ids?: string[]; jobRun?: unknown } }>(
          deleteRecycledItemsMutation,
          { input: { ids: pendingAction.items.map((item) => item.id) } },
        )
        .toPromise();
      if (error) throw error;
      const payload = data?.deleteRecycledItems;
      registerBatchJob(
        payload?.jobRun,
        payload?.ids ?? pendingAction.items.map((item) => item.id),
        t(
          pendingAction.items.length === 1
            ? "settings.recycleBinDeleteQueuedOne"
            : "settings.recycleBinDeleteQueuedOther",
          { count: pendingAction.items.length },
        ),
      );
      setPendingAction(null);
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToDelete"));
    } finally {
      setMutatingId(null);
    }
  };

  const requestEmpty = () => {
    if (canManageItems) setPendingAction({ type: "empty", count: totalCount });
  };

  const confirmEmpty = async () => {
    if (!canManageItems || pendingAction?.type !== "empty") return;
    setMutatingId("__empty__");
    try {
      const { data, error } = await client
        .mutation<{ emptyRecycleBin?: { jobRun?: unknown } }>(
          emptyRecycleBinMutation,
          { libraryIds: selectedLibraryIdsToQueryValue(selectedLibraryIds) },
        )
        .toPromise();
      if (error) throw error;
      registerBatchJob(
        data?.emptyRecycleBin?.jobRun,
        items.map((item) => item.id),
        t(
          pendingAction.count === 1
            ? "settings.recycleBinDeleteQueuedOne"
            : "settings.recycleBinDeleteQueuedOther",
          { count: pendingAction.count },
        ),
      );
      setPendingAction(null);
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToDelete"));
    } finally {
      setMutatingId(null);
    }
  };

  const actionTitle =
    pendingAction?.type === "restore"
      ? t("settings.recycleBinRestoreSelectedTitle")
      : pendingAction?.type === "empty"
        ? t("settings.recycleBinEmptyAll")
        : t("settings.recycleBinDelete");
  const actionDescription =
    pendingAction?.type === "restore"
      ? pendingAction.occupiedCount > 0
        ? t(
            pendingAction.occupiedCount === 1
              ? "settings.recycleBinRestoreOccupiedOne"
              : "settings.recycleBinRestoreOccupiedOther",
            { count: pendingAction.occupiedCount },
          )
        : t(
            pendingAction.items.length === 1
              ? "settings.recycleBinRestoreDescriptionOne"
              : "settings.recycleBinRestoreDescriptionOther",
            { count: pendingAction.items.length },
          )
      : pendingAction?.type === "empty"
        ? t("settings.recycleBinEmptyConfirm", { count: pendingAction.count })
        : t(
            pendingAction?.items.length === 1
              ? "settings.recycleBinDeleteSelectedConfirmOne"
              : "settings.recycleBinDeleteSelectedConfirmOther",
            { count: pendingAction?.items.length ?? 0 },
          );

  if (!settings) {
    return settingsLoading ? (
      <div className="flex items-center gap-2 text-sm text-muted-foreground">
        <LoadingMark className="h-4 w-4" />
        {t("label.loading")}
      </div>
    ) : (
      <p id="settings-recycle-bin-load-error" role="alert" className="text-sm text-[var(--scry-danger-text)]">
        {t("status.failedToLoad")}
      </p>
    );
  }

  return (
    <>
      <SettingsRecycleBinSection
        enabled={settings.enabled}
        path={settings.path}
        retentionDays={settings.retentionDays}
        effectivePaths={settings.effectivePaths}
        validationError={settings.validationError}
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
        pendingItemIds={pendingItemIds}
        selectedItemIds={selectedItemIds}
        onEnabledChange={(enabled) => void saveSettings({ enabled })}
        onLocationSave={(path, retentionDays) => void saveSettings({ path, retentionDays })}
        onSelectedLibraryIdsChange={setSelectedLibraryIds}
        onSelectedItemIdsChange={(ids) => setSelectedItemIds(new Set(ids))}
        onRestoreItems={requestRestore}
        onDeleteItems={requestDelete}
        onEmptyAll={requestEmpty}
      />
      <ConfirmDialog
        open={pendingAction !== null}
        title={actionTitle}
        description={actionDescription}
        confirmLabel={
          pendingAction?.type === "restore"
            ? pendingAction.conflictPolicy === "REPLACE_EXISTING"
              ? t("settings.recycleBinReplaceExistingAndRestore")
              : t("settings.recycleBinKeepBothAndRestore")
            : pendingAction?.type === "empty"
              ? t("settings.recycleBinEmptyAll")
              : t("settings.recycleBinDelete")
        }
        cancelLabel={t("label.cancel")}
        contentId="settings-recycle-bin-confirm-dialog"
        confirmButtonId="settings-recycle-bin-confirm"
        cancelButtonId="settings-recycle-bin-confirm-cancel"
        confirmButtonVariant={pendingAction?.type === "restore" && pendingAction.conflictPolicy === "KEEP_BOTH" ? "default" : "destructive"}
        isBusy={mutatingId !== null}
        onConfirm={
          pendingAction?.type === "restore"
            ? confirmRestore
            : pendingAction?.type === "empty"
              ? confirmEmpty
              : confirmDelete
        }
        onCancel={() => setPendingAction(null)}
      >
        {pendingAction?.type === "restore" && pendingAction.occupiedCount > 0 ? (
          <div className="space-y-2 text-sm">
            <label className="flex cursor-pointer items-start gap-2">
              <input
                type="radio"
                name="settings-recycle-bin-conflict-policy"
                checked={pendingAction.conflictPolicy === "KEEP_BOTH"}
                onChange={() => setPendingAction({ ...pendingAction, conflictPolicy: "KEEP_BOTH" })}
              />
              <span>
                <strong>{t("settings.recycleBinKeepBoth")}</strong>
                <br />
                <span className="text-xs text-muted-foreground">
                  {t("settings.recycleBinKeepBothDescription")}
                </span>
              </span>
            </label>
            <label className="flex cursor-pointer items-start gap-2">
              <input
                type="radio"
                name="settings-recycle-bin-conflict-policy"
                checked={pendingAction.conflictPolicy === "REPLACE_EXISTING"}
                onChange={() => setPendingAction({ ...pendingAction, conflictPolicy: "REPLACE_EXISTING" })}
              />
              <span>
                <strong>{t("settings.recycleBinReplaceExisting")}</strong>
                <br />
                <span className="text-xs text-muted-foreground">
                  {t("settings.recycleBinReplaceExistingDescription")}
                </span>
              </span>
            </label>
          </div>
        ) : null}
      </ConfirmDialog>
    </>
  );
}

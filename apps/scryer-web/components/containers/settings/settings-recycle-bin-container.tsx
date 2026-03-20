import { useCallback, useEffect, useState } from "react";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import {
  SettingsRecycleBinSection,
  type RecycledItem,
} from "@/components/views/settings/settings-recycle-bin-section";
import { useClient } from "urql";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { recycledItemsQuery } from "@/lib/graphql/queries";
import {
  restoreRecycledItemMutation,
  deleteRecycledItemMutation,
  emptyRecycleBinMutation,
} from "@/lib/graphql/mutations";

type PendingAction = { type: "delete"; item: RecycledItem } | { type: "empty"; count: number };

export function SettingsRecycleBinContainer() {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const [items, setItems] = useState<RecycledItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [mutatingId, setMutatingId] = useState<string | null>(null);
  const [pendingAction, setPendingAction] = useState<PendingAction | null>(null);

  const fetchItems = useCallback(async () => {
    try {
      const { data, error } = await client.query(recycledItemsQuery, {}).toPromise();
      if (error) throw error;
      setItems(data?.recycledItems?.items ?? []);
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
    } finally {
      setLoading(false);
    }
  }, [client, setGlobalStatus, t]);

  useEffect(() => {
    void fetchItems();
  }, [fetchItems]);

  const restoreItem = async (item: RecycledItem) => {
    setMutatingId(item.id);
    try {
      const { error } = await client
        .mutation(restoreRecycledItemMutation, { id: item.id })
        .toPromise();
      if (error) throw error;
      setGlobalStatus(t("status.recycleBinRestored", { path: item.originalPath }));
      await fetchItems();
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setMutatingId(null);
    }
  };

  const requestDelete = (item: RecycledItem) => {
    setPendingAction({ type: "delete", item });
  };

  const confirmDelete = async () => {
    if (!pendingAction || pendingAction.type !== "delete") return;
    const item = pendingAction.item;
    setMutatingId(item.id);
    try {
      const { error } = await client
        .mutation(deleteRecycledItemMutation, { id: item.id })
        .toPromise();
      if (error) throw error;
      setGlobalStatus(t("status.recycleBinDeleted"));
      await fetchItems();
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToDelete"));
    } finally {
      setMutatingId(null);
      setPendingAction(null);
    }
  };

  const requestEmpty = () => {
    setPendingAction({ type: "empty", count: items.length });
  };

  const confirmEmpty = async () => {
    setMutatingId("__empty__");
    try {
      const { data, error } = await client
        .mutation(emptyRecycleBinMutation, {})
        .toPromise();
      if (error) throw error;
      const count = data?.emptyRecycleBin ?? 0;
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
        items={items}
        loading={loading}
        mutatingId={mutatingId}
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
        isBusy={mutatingId !== null}
        onConfirm={pendingAction?.type === "empty" ? confirmEmpty : confirmDelete}
        onCancel={() => setPendingAction(null)}
      />
    </>
  );
}

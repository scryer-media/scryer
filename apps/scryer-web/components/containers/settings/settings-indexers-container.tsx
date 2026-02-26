
import { type ComponentProps, type FormEvent, useCallback, useEffect, useState } from "react";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { SettingsIndexersSection } from "@/components/views/settings/settings-indexers-section";
import { useClient } from "urql";
import type { Translate } from "@/components/root/types";
import type { IndexerRecord } from "@/lib/types";
import { indexersQuery } from "@/lib/graphql/queries";
import {
  createIndexerMutation,
  deleteIndexerMutation,
  updateIndexerMutation,
} from "@/lib/graphql/mutations";

type SettingsIndexersContainerProps = {
  t: Translate;
  setGlobalStatus: (status: string) => void;
};

type SettingsIndexersSectionProps = ComponentProps<typeof SettingsIndexersSection>;

const INDEXER_INITIAL_DRAFT = {
  name: "",
  providerType: "nzbgeek",
  baseUrl: "https://api.nzbgeek.info/api",
  apiKey: "",
  isEnabled: true,
  enableInteractiveSearch: true,
  enableAutoSearch: true,
};

export function SettingsIndexersContainer({
  t,
  setGlobalStatus,
}: SettingsIndexersContainerProps) {
  const client = useClient();
  const [settingsIndexers, setSettingsIndexers] = useState<IndexerRecord[]>([]);
  const [settingsIndexerFilter, setSettingsIndexerFilter] = useState("");
  const [mutatingIndexerId, setMutatingIndexerId] = useState<string | null>(null);
  const [editingIndexerId, setEditingIndexerId] = useState<string | null>(null);
  const [pendingDeleteIndexer, setPendingDeleteIndexer] = useState<IndexerRecord | null>(null);
  const [indexerDraft, setIndexerDraft] = useState<SettingsIndexersSectionProps["indexerDraft"]>(
    () => ({ ...INDEXER_INITIAL_DRAFT }),
  );

  const resetIndexerDraft = useCallback(() => {
    setEditingIndexerId(null);
    setIndexerDraft(() => ({ ...INDEXER_INITIAL_DRAFT }));
  }, []);

  const refreshIndexers = useCallback(async () => {
    try {
      const { data, error } = await client.query(indexersQuery, {
        providerType: settingsIndexerFilter || undefined,
      }).toPromise();
      if (error) throw error;
      setSettingsIndexers(data.indexers || []);
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
    }
  }, [client, settingsIndexerFilter, setGlobalStatus, t]);

  useEffect(() => {
    void refreshIndexers();
  }, [refreshIndexers]);

  const submitIndexer = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const payload = {
      name: indexerDraft.name.trim(),
      providerType: indexerDraft.providerType.trim(),
      baseUrl: indexerDraft.baseUrl.trim(),
      apiKey: indexerDraft.apiKey.trim(),
      isEnabled: indexerDraft.isEnabled,
      enableInteractiveSearch: indexerDraft.enableInteractiveSearch,
      enableAutoSearch: indexerDraft.enableAutoSearch,
    };

    if (!payload.name || !payload.providerType || !payload.baseUrl) {
      setGlobalStatus(t("form.indexerValidation"));
      return;
    }

    setMutatingIndexerId(editingIndexerId || "new");
    try {
      if (editingIndexerId) {
        const { error } = await client.mutation(updateIndexerMutation, {
          input: {
            id: editingIndexerId,
            name: payload.name,
            providerType: payload.providerType,
            baseUrl: payload.baseUrl,
            apiKey: payload.apiKey || undefined,
            isEnabled: payload.isEnabled,
            enableInteractiveSearch: payload.enableInteractiveSearch,
            enableAutoSearch: payload.enableAutoSearch,
          },
        }).toPromise();
        if (error) throw error;
        setGlobalStatus(t("settings.indexerUpdated"));
      } else {
        const { error } = await client.mutation(createIndexerMutation, {
          input: {
            name: payload.name,
            providerType: payload.providerType,
            baseUrl: payload.baseUrl,
            apiKey: payload.apiKey || undefined,
            isEnabled: payload.isEnabled,
            enableInteractiveSearch: payload.enableInteractiveSearch,
            enableAutoSearch: payload.enableAutoSearch,
          },
        }).toPromise();
        if (error) throw error;
        setGlobalStatus(t("settings.indexerCreated"));
      }
      resetIndexerDraft();
      await refreshIndexers();
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setMutatingIndexerId(null);
    }
  };

  const editIndexer = (indexer: IndexerRecord) => {
    setEditingIndexerId(indexer.id);
    setIndexerDraft({
      name: indexer.name,
      providerType: indexer.providerType,
      baseUrl: indexer.baseUrl,
      apiKey: "",
      isEnabled: indexer.isEnabled,
      enableInteractiveSearch: indexer.enableInteractiveSearch,
      enableAutoSearch: indexer.enableAutoSearch,
    });
    setGlobalStatus(t("status.editingIndexer", { name: indexer.name }));
  };

  const deleteIndexer = async (indexer: IndexerRecord) => {
    setPendingDeleteIndexer(indexer);
  };

  const toggleIndexerEnabled = useCallback(async (indexer: IndexerRecord) => {
    const nextIsEnabled = !indexer.isEnabled;
    setMutatingIndexerId(indexer.id);
    try {
      const { error } = await client.mutation(updateIndexerMutation, {
        input: {
          id: indexer.id,
          isEnabled: nextIsEnabled,
        },
      }).toPromise();
      if (error) throw error;
      setGlobalStatus(t("status.indexerUpdated"));
      await refreshIndexers();
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setMutatingIndexerId(null);
    }
  }, [client, refreshIndexers, setGlobalStatus, t]);

  const confirmDeleteIndexer = async () => {
    if (!pendingDeleteIndexer) {
      return;
    }
    const indexer = pendingDeleteIndexer;
    setMutatingIndexerId(indexer.id);
    try {
      const { error } = await client.mutation(deleteIndexerMutation, {
        input: { id: indexer.id },
      }).toPromise();
      if (error) throw error;
      setGlobalStatus(t("status.indexerDeleted", { name: indexer.name }));
      await refreshIndexers();
      if (editingIndexerId === indexer.id) {
        resetIndexerDraft();
      }
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToDelete"));
    } finally {
      setMutatingIndexerId(null);
      setPendingDeleteIndexer(null);
    }
  };

  return (
    <>
      <SettingsIndexersSection
        t={t}
        editingIndexerId={editingIndexerId}
        indexerDraft={indexerDraft}
        setIndexerDraft={setIndexerDraft}
        submitIndexer={submitIndexer}
        mutatingIndexerId={mutatingIndexerId}
        resetIndexerDraft={resetIndexerDraft}
        settingsIndexerFilter={settingsIndexerFilter}
        setSettingsIndexerFilter={setSettingsIndexerFilter}
        settingsIndexers={settingsIndexers}
        editIndexer={editIndexer}
        toggleIndexerEnabled={toggleIndexerEnabled}
        deleteIndexer={deleteIndexer}
      />
      <ConfirmDialog
        open={pendingDeleteIndexer !== null}
        title={t("label.delete")}
        description={
          pendingDeleteIndexer ? t("status.deletingIndexer", { name: pendingDeleteIndexer.name }) : ""
        }
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        isBusy={mutatingIndexerId !== null}
        onConfirm={confirmDeleteIndexer}
        onCancel={() => setPendingDeleteIndexer(null)}
      />
    </>
  );
}

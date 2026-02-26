
import { type ComponentProps, useCallback, useEffect, useState } from "react";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { SettingsDownloadClientsSection } from "@/components/views/settings/settings-download-clients-section";
import {
  deleteDownloadClientMutation,
  createDownloadClientMutation,
  testDownloadClientConnectionMutation,
  updateDownloadClientMutation,
} from "@/lib/graphql/mutations";
import { downloadClientsQuery } from "@/lib/graphql/queries";
import { DEFAULT_DOWNLOAD_CLIENT_DRAFT } from "@/lib/constants/download-clients";
import { useClient } from "urql";
import type { Translate } from "@/components/root/types";
import {
  buildDownloadClientBaseUrl,
  buildDownloadClientConfigJson,
  buildDownloadClientDraftFromRecord,
  normalizeDownloadClientType,
} from "@/lib/utils/download-clients";
import type { DownloadClientRecord, DownloadClientDraft } from "@/lib/types";

type SettingsDownloadClientsSectionProps = ComponentProps<typeof SettingsDownloadClientsSection>;
type SettingsDownloadClientsContainerProps = {
  t: Translate;
  setGlobalStatus: (status: string) => void;
};

export function SettingsDownloadClientsContainer({
  t,
  setGlobalStatus,
}: SettingsDownloadClientsContainerProps) {
  const client = useClient();
  const [settingsDownloadClients, setSettingsDownloadClients] = useState<SettingsDownloadClientsSectionProps["settingsDownloadClients"]>(
    [],
  );
  const [downloadClientDraft, setDownloadClientDraft] = useState<DownloadClientDraft>(() => ({
    ...DEFAULT_DOWNLOAD_CLIENT_DRAFT,
  }));
  const [editingDownloadClientId, setEditingDownloadClientId] = useState<string | null>(null);
  const [mutatingDownloadClientId, setMutatingDownloadClientId] = useState<string | null>(null);
  const [isTestingDownloadClientConnection, setIsTestingDownloadClientConnection] = useState(false);
  const [pendingDeleteDownloadClient, setPendingDeleteDownloadClient] = useState<DownloadClientRecord | null>(null);

  const getDownloadClientErrorMessage = useCallback(
    (error: unknown, fallback: string) => (error instanceof Error ? error.message : fallback),
    [],
  );

  const resetDownloadClientDraft = useCallback(() => {
    setEditingDownloadClientId(null);
    setDownloadClientDraft({
      ...DEFAULT_DOWNLOAD_CLIENT_DRAFT,
      isEnabled: true,
    });
  }, []);

  const refreshDownloadClients = useCallback(async () => {
    try {
      const { data, error } = await client.query(downloadClientsQuery, {}).toPromise();
      if (error) throw error;
      setSettingsDownloadClients(data.downloadClientConfigs || []);
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
    }
  }, [client, setGlobalStatus, t]);

  useEffect(() => {
    void refreshDownloadClients();
  }, [refreshDownloadClients]);

  const submitDownloadClient = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const payload = {
      name: downloadClientDraft.name.trim(),
      clientType: normalizeDownloadClientType(downloadClientDraft.clientType),
      host: downloadClientDraft.host.trim(),
      port: downloadClientDraft.port.trim(),
      baseUrl: buildDownloadClientBaseUrl(downloadClientDraft),
      configJson: buildDownloadClientConfigJson(downloadClientDraft),
      isEnabled: downloadClientDraft.isEnabled,
    };

    if (!payload.name || !payload.host) {
      const message = t("settings.downloadClientValidation");
      setGlobalStatus(message);
      return;
    }
    if (!payload.baseUrl) {
      const message = t("settings.downloadClientBaseUrlRequired");
      setGlobalStatus(message);
      return;
    }

    setMutatingDownloadClientId(editingDownloadClientId || "new");
    try {
      if (payload.clientType === "nzbget") {
        setGlobalStatus(t("status.testingDownloadClient"));
        const { data: testData, error: testError } = await client.mutation(
          testDownloadClientConnectionMutation,
          {
            input: {
              clientType: payload.clientType,
              baseUrl: payload.baseUrl,
              configJson: payload.configJson,
            },
          },
        ).toPromise();
        if (testError) throw testError;
        if (!testData.testDownloadClientConnection) {
          throw new Error(t("status.downloadClientConnectionTestFailed"));
        }
        const passedMessage = t("status.downloadClientConnectionTestPassed");
        setGlobalStatus(passedMessage);
      }

      if (editingDownloadClientId) {
        const { error } = await client.mutation(updateDownloadClientMutation, {
          input: {
            id: editingDownloadClientId,
            name: payload.name,
            clientType: payload.clientType,
            baseUrl: payload.baseUrl,
            configJson: payload.configJson,
            isEnabled: payload.isEnabled,
          },
        }).toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.downloadClientUpdated"));
      } else {
        const { error } = await client.mutation(
          createDownloadClientMutation,
          {
            input: {
              name: payload.name,
              clientType: payload.clientType,
              baseUrl: payload.baseUrl,
              configJson: payload.configJson,
              isEnabled: payload.isEnabled,
            },
          },
        ).toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.downloadClientCreated"));
      }
      resetDownloadClientDraft();
      await refreshDownloadClients();
    } catch (error) {
      const message = getDownloadClientErrorMessage(error, t("status.failedToUpdate"));
      setGlobalStatus(message);
    } finally {
      setMutatingDownloadClientId(null);
    }
  };

  const testDownloadClientConnection = async () => {
    const payload = {
      name: downloadClientDraft.name.trim(),
      clientType: normalizeDownloadClientType(downloadClientDraft.clientType),
      host: downloadClientDraft.host.trim(),
      baseUrl: buildDownloadClientBaseUrl(downloadClientDraft),
      configJson: buildDownloadClientConfigJson(downloadClientDraft),
    };

    if (!payload.name || !payload.host) {
      const message = t("settings.downloadClientValidation");
      setGlobalStatus(message);
      return;
    }

    if (!payload.baseUrl) {
      const message = t("settings.downloadClientBaseUrlRequired");
      setGlobalStatus(message);
      return;
    }

    setIsTestingDownloadClientConnection(true);
    try {
      setGlobalStatus(t("status.testingDownloadClient"));
      const { data: testData, error: testError } = await client.mutation(
        testDownloadClientConnectionMutation,
        {
          input: {
            clientType: payload.clientType,
            baseUrl: payload.baseUrl,
            configJson: payload.configJson,
          },
        },
      ).toPromise();
      if (testError) throw testError;
      if (!testData.testDownloadClientConnection) {
        throw new Error(t("status.downloadClientConnectionTestFailed"));
      }
      const successMessage = t("status.downloadClientConnectionTestPassed");
      setGlobalStatus(successMessage);
    } catch (error) {
      const message = getDownloadClientErrorMessage(error, t("status.failedToUpdate"));
      setGlobalStatus(message);
    } finally {
      setIsTestingDownloadClientConnection(false);
    }
  };

  const editDownloadClient = useCallback((downloadClient: DownloadClientRecord) => {
    setEditingDownloadClientId(downloadClient.id);
    setDownloadClientDraft(buildDownloadClientDraftFromRecord(downloadClient));
    setGlobalStatus(t("status.editingDownloadClient", { name: downloadClient.name }));
  }, [setGlobalStatus, t]);

  const toggleDownloadClientEnabled = useCallback(async (downloadClient: DownloadClientRecord) => {
    const nextIsEnabled = !downloadClient.isEnabled;
    setMutatingDownloadClientId(downloadClient.id);
    try {
      const { error } = await client.mutation(updateDownloadClientMutation, {
        input: {
          id: downloadClient.id,
          isEnabled: nextIsEnabled,
        },
      }).toPromise();
      if (error) throw error;
      setGlobalStatus(t("status.downloadClientUpdated"));
      await refreshDownloadClients();
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setMutatingDownloadClientId(null);
    }
  }, [client, refreshDownloadClients, setGlobalStatus, t]);

  const deleteDownloadClient = useCallback(async (downloadClient: DownloadClientRecord) => {
    setPendingDeleteDownloadClient(downloadClient);
  }, []);

  const confirmDeleteDownloadClient = useCallback(async () => {
    if (!pendingDeleteDownloadClient) {
      return;
    }
    const downloadClient = pendingDeleteDownloadClient;
    setMutatingDownloadClientId(downloadClient.id);
    try {
      const { error } = await client.mutation(deleteDownloadClientMutation, {
        input: { id: downloadClient.id },
      }).toPromise();
      if (error) throw error;
      setGlobalStatus(t("status.downloadClientDeleted", { name: downloadClient.name }));
      await refreshDownloadClients();
      if (editingDownloadClientId === downloadClient.id) {
        resetDownloadClientDraft();
      }
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToDelete"));
    } finally {
      setMutatingDownloadClientId(null);
      setPendingDeleteDownloadClient(null);
    }
  }, [editingDownloadClientId, pendingDeleteDownloadClient, refreshDownloadClients, resetDownloadClientDraft, client, setGlobalStatus, t]);

  return (
    <>
      <SettingsDownloadClientsSection
        t={t}
        editingDownloadClientId={editingDownloadClientId}
        downloadClientDraft={downloadClientDraft}
        setDownloadClientDraft={setDownloadClientDraft}
        submitDownloadClient={submitDownloadClient}
        testDownloadClientConnection={testDownloadClientConnection}
        isTestingDownloadClientConnection={isTestingDownloadClientConnection}
        mutatingDownloadClientId={mutatingDownloadClientId}
        resetDownloadClientDraft={resetDownloadClientDraft}
        settingsDownloadClients={settingsDownloadClients}
        editDownloadClient={editDownloadClient}
        toggleDownloadClientEnabled={toggleDownloadClientEnabled}
        deleteDownloadClient={deleteDownloadClient}
      />
      <ConfirmDialog
        open={pendingDeleteDownloadClient !== null}
        title={t("label.delete")}
        description={
          pendingDeleteDownloadClient
            ? t("status.deletingDownloadClient", { name: pendingDeleteDownloadClient.name })
            : ""
        }
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        isBusy={mutatingDownloadClientId !== null}
        onConfirm={confirmDeleteDownloadClient}
        onCancel={() => setPendingDeleteDownloadClient(null)}
      />
    </>
  );
}

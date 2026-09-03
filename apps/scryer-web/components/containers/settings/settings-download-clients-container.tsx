
import { type ComponentProps, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { FilteredPluginList } from "@/components/views/settings/filtered-plugin-list";
import { SETTINGS_REFERENCE_SLOT_ID } from "@/components/containers/settings/settings-container";
import { SettingsDownloadClientsSection } from "@/components/views/settings/settings-download-clients-section";
import {
  createDownloadClientMutation,
  deleteDownloadClientMutation,
  reorderDownloadClientsMutation,
  testDownloadClientConnectionMutation,
  updateDownloadClientMutation,
} from "@/lib/graphql/mutations";
import {
  downloadClientProviderTypesQuery,
  downloadClientsInitQuery,
  downloadClientsQuery,
} from "@/lib/graphql/queries";
import { DEFAULT_DOWNLOAD_CLIENT_DRAFT } from "@/lib/constants/download-clients";
import { useClient } from "urql";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import {
  isReportedConnectionFeedbackError,
  runConnectionFeedback,
} from "@/lib/utils/connection-feedback";
import {
  buildDownloadClientBaseUrl,
  buildDownloadClientConfigValues,
  normalizeDownloadClientDraft,
  buildDownloadClientDraftFromRecord,
  buildDownloadClientTypeOptions,
  ensureDownloadClientTypeOption,
  isBuiltInDownloadClientType,
  normalizeDownloadClientType,
} from "@/lib/utils/download-clients";
import { buildDownloadClientConnectionTestInput } from "@/lib/utils/settings-mutation-inputs";
import {
  localPathStyleFromRuntimeValue,
  type LocalPathStyle,
} from "@/lib/utils/local-path-style";
import type {
  DownloadClientRecord,
  DownloadClientDraft,
  DownloadClientTypeOption,
  ProviderTypeInfo,
} from "@/lib/types";

type SettingsDownloadClientsSectionProps = ComponentProps<typeof SettingsDownloadClientsSection>;
const DOWNLOAD_CLIENT_ADJACENT_PLUGIN_TYPES = ["archive_extractor"] as const;

type SettingsDownloadClientsContainerProps = {
  providerCatalogVersion?: number;
  onDownloadClientsChanged?: () => Promise<void> | void;
};

type PendingDownloadClientEditorAction =
  | { type: "create" }
  | { type: "edit"; downloadClient: DownloadClientRecord }
  | { type: "close" }
  | null;

function cloneDownloadClientDraft(
  draft: DownloadClientDraft,
): DownloadClientDraft {
  return { ...draft, configValues: { ...draft.configValues } };
}

export function SettingsDownloadClientsContainer({
  providerCatalogVersion = 0,
  onDownloadClientsChanged,
}: SettingsDownloadClientsContainerProps) {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const [settingsDownloadClients, setSettingsDownloadClients] = useState<SettingsDownloadClientsSectionProps["settingsDownloadClients"]>(
    [],
  );
  const [pluginsTarget, setPluginsTarget] = useState<HTMLElement | null>(null);
  useEffect(() => {
    setPluginsTarget(document.getElementById(SETTINGS_REFERENCE_SLOT_ID));
  }, []);
  const [downloadClientTypeOptions, setDownloadClientTypeOptions] = useState<DownloadClientTypeOption[]>(
    () => buildDownloadClientTypeOptions([]),
  );
  const [downloadClientDraft, setDownloadClientDraft] = useState<DownloadClientDraft>(() => ({
    ...DEFAULT_DOWNLOAD_CLIENT_DRAFT,
  }));
  const [editingDownloadClientId, setEditingDownloadClientId] = useState<string | null>(null);
  const [mutatingDownloadClientId, setMutatingDownloadClientId] = useState<string | null>(null);
  const [isTestingDownloadClientConnection, setIsTestingDownloadClientConnection] = useState(false);
  const [pendingDeleteDownloadClient, setPendingDeleteDownloadClient] = useState<DownloadClientRecord | null>(null);
  const [downloadClientOrder, setDownloadClientOrder] = useState<string[]>([]);
  const [isSavingOrder, setIsSavingOrder] = useState(false);
  const [isEditorOpen, setIsEditorOpen] = useState(false);
  const [editorMode, setEditorMode] = useState<"create" | "edit">("create");
  const [localPathStyle, setLocalPathStyle] =
    useState<LocalPathStyle | undefined>(undefined);
  const [pendingEditorAction, setPendingEditorAction] =
    useState<PendingDownloadClientEditorAction>(null);
  const [draftBaseline, setDraftBaseline] = useState<DownloadClientDraft>(() =>
    cloneDownloadClientDraft({
      ...DEFAULT_DOWNLOAD_CLIENT_DRAFT,
    }),
  );
  const [awaitingBaselineSync, setAwaitingBaselineSync] = useState(false);
  const providerCatalogVersionRef = useRef(providerCatalogVersion);

  const getDownloadClientErrorMessage = useCallback(
    (error: unknown, fallback: string) => (error instanceof Error ? error.message : fallback),
    [],
  );

  const resetDownloadClientDraft = useCallback(() => {
    setEditingDownloadClientId(null);
    setDownloadClientDraft(cloneDownloadClientDraft({
      ...DEFAULT_DOWNLOAD_CLIENT_DRAFT,
      isEnabled: true,
    }));
  }, []);

  useEffect(() => {
    if (!awaitingBaselineSync) {
      return;
    }

    setDraftBaseline(cloneDownloadClientDraft(downloadClientDraft));
    setAwaitingBaselineSync(false);
  }, [awaitingBaselineSync, downloadClientDraft]);

  const isDraftDirty =
    JSON.stringify(downloadClientDraft) !== JSON.stringify(draftBaseline);

  const refreshDownloadClients = useCallback(async () => {
    try {
      const { data, error } = await client
        .query(downloadClientsQuery, {}, { requestPolicy: "network-only" })
        .toPromise();
      if (error) throw error;
      const clients: DownloadClientRecord[] = data.downloadClientConfigs || [];
      setSettingsDownloadClients(clients);
      setDownloadClientOrder(clients.map((c) => c.id));
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
    }
  }, [client, setGlobalStatus, t]);

  const refreshProviderTypes = useCallback(async () => {
    const { data, error } = await client
      .query(downloadClientProviderTypesQuery, {}, { requestPolicy: "network-only" })
      .toPromise();
    if (error) throw error;
    setDownloadClientTypeOptions(
      buildDownloadClientTypeOptions(
        (data?.downloadClientProviderTypes as ProviderTypeInfo[] | undefined) ?? [],
      ),
    );
  }, [client]);

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      try {
        const { data, error } = await client
          .query(downloadClientsInitQuery, {}, { requestPolicy: "network-only" })
          .toPromise();
        if (error && !data?.downloadClientConfigs) throw error;
        if (cancelled) return;
        const clients: DownloadClientRecord[] = data?.downloadClientConfigs || [];
        setSettingsDownloadClients(clients);
        setDownloadClientOrder(clients.map((clientRecord) => clientRecord.id));
        setLocalPathStyle(
          localPathStyleFromRuntimeValue(data?.runtimeInfo?.runtimePathStyle),
        );
        setDownloadClientTypeOptions(
          buildDownloadClientTypeOptions(
            (data?.downloadClientProviderTypes as ProviderTypeInfo[] | undefined) ?? [],
          ),
        );
      } catch (error) {
        setDownloadClientTypeOptions(buildDownloadClientTypeOptions([]));
        setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
      }
    };
    void load();
    return () => {
      cancelled = true;
    };
  }, [client, setGlobalStatus, t]);

  useEffect(() => {
    if (providerCatalogVersion === providerCatalogVersionRef.current) {
      return;
    }

    providerCatalogVersionRef.current = providerCatalogVersion;
    void refreshProviderTypes().catch((error: unknown) => {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
    });
  }, [providerCatalogVersion, refreshProviderTypes, setGlobalStatus, t]);

  useEffect(() => {
    if (editingDownloadClientId) {
      return;
    }

    setDownloadClientDraft((prev) => {
      const normalizedClientType = normalizeDownloadClientType(prev.clientType);
      const configuredOption =
        downloadClientTypeOptions.find(
          (option) => option.value === normalizedClientType,
        ) ?? null;
      const nextOption = configuredOption ?? downloadClientTypeOptions[0] ?? null;

      if (!nextOption) {
        return prev;
      }

      const previousLabel = configuredOption?.label ?? prev.clientType.trim();
      const shouldAutofillName =
        prev.name.trim().length === 0 || prev.name === previousLabel;
      const nextClientType = configuredOption ? prev.clientType : nextOption.value;
      const nextName = shouldAutofillName ? nextOption.label : prev.name;

      if (nextClientType === prev.clientType && nextName === prev.name) {
        return prev;
      }

      return {
        ...prev,
        clientType: nextClientType,
        name: nextName,
      };
    });
  }, [downloadClientTypeOptions, editingDownloadClientId]);

  const availableDownloadClientTypeOptions = useMemo(
    () => ensureDownloadClientTypeOption(downloadClientTypeOptions, downloadClientDraft.clientType),
    [downloadClientDraft.clientType, downloadClientTypeOptions],
  );

  const selectedDownloadClientLabel = useMemo(() => {
    const normalizedClientType = normalizeDownloadClientType(downloadClientDraft.clientType, "");
    const configuredClientLabel = downloadClientDraft.clientType.trim();
    return (
      availableDownloadClientTypeOptions.find((option) => option.value === normalizedClientType)?.label ??
      (configuredClientLabel || "Download client")
    );
  }, [availableDownloadClientTypeOptions, downloadClientDraft.clientType]);
  const selectedDownloadClientConfigFields = useMemo(() => {
    const normalizedClientType = normalizeDownloadClientType(downloadClientDraft.clientType, "");
    return (
      availableDownloadClientTypeOptions.find((option) => option.value === normalizedClientType)
        ?.configFields ?? []
    );
  }, [availableDownloadClientTypeOptions, downloadClientDraft.clientType]);
  const editingStoredSecretKeys = useMemo(
    () =>
      new Set<string>(
        settingsDownloadClients.find(
          (downloadClient) => downloadClient.id === editingDownloadClientId,
        )
          ?.storedSecretKeys ?? [],
      ),
    [settingsDownloadClients, editingDownloadClientId],
  );

  const openCreateEditor = useCallback(() => {
    resetDownloadClientDraft();
    setEditorMode("create");
    setIsEditorOpen(true);
    setAwaitingBaselineSync(true);
  }, [resetDownloadClientDraft]);

  const submitDownloadClient = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    // Parse on save: a URL pasted into the host box is taken apart into the
    // host, port, URL-base and SSL fields, and the form is put back with them
    // so the operator sees where their address went.
    const draft = normalizeDownloadClientDraft(downloadClientDraft);
    setDownloadClientDraft(draft);
    const payload = {
      name: draft.name.trim(),
      clientType: normalizeDownloadClientType(draft.clientType),
      host: draft.host.trim(),
      port: draft.port.trim(),
      config: buildDownloadClientConfigValues(
        draft,
        selectedDownloadClientConfigFields,
        editingStoredSecretKeys,
      ),
      isEnabled: draft.isEnabled,
    };

    if (!payload.name || !payload.host) {
      const message = t("settings.downloadClientValidation");
      setGlobalStatus(message);
      return;
    }

    setMutatingDownloadClientId(editingDownloadClientId || "new");
    try {
      if (isBuiltInDownloadClientType(payload.clientType)) {
        await runConnectionFeedback({
          setGlobalStatus,
          startMessage: t("status.testingDownloadClient", {
            client: selectedDownloadClientLabel,
          }),
          successMessage: t("status.downloadClientConnectionTestPassed", {
            client: selectedDownloadClientLabel,
          }),
          failureFallbackMessage: t("status.downloadClientConnectionTestFailed", {
            client: selectedDownloadClientLabel,
          }),
          announceSuccess: false,
          run: async () => {
            const { data: testData, error: testError } = await client
              .mutation(testDownloadClientConnectionMutation, {
                input: buildDownloadClientConnectionTestInput(
                  editingDownloadClientId,
                  payload.clientType,
                  payload.config,
                ),
              })
              .toPromise();
            if (testError) throw testError;
            const validation = testData?.testDownloadClientConnection;
            if (validation?.status !== "ok") {
              throw new Error(
                validation?.message ?? t("status.downloadClientConnectionTestFailed", {
                  client: selectedDownloadClientLabel,
                }),
              );
            }
          },
        });
      }

      if (editingDownloadClientId) {
        const { error } = await client.mutation(updateDownloadClientMutation, {
          input: {
            id: editingDownloadClientId,
            name: payload.name,
            clientType: payload.clientType,
            config: payload.config,
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
              config: payload.config,
              isEnabled: payload.isEnabled,
            },
          },
        ).toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.downloadClientCreated"));
      }
      resetDownloadClientDraft();
      setIsEditorOpen(false);
      setEditorMode("create");
      setAwaitingBaselineSync(true);
      void onDownloadClientsChanged?.();
      await refreshDownloadClients();
    } catch (error) {
      if (!isReportedConnectionFeedbackError(error)) {
        const message = getDownloadClientErrorMessage(
          error,
          t("status.failedToUpdate"),
        );
        setGlobalStatus(message);
      }
    } finally {
      setMutatingDownloadClientId(null);
    }
  };

  const testDownloadClientConnection = async () => {
    // The test dials what a save would store, so it normalizes the same way.
    const draft = normalizeDownloadClientDraft(downloadClientDraft);
    setDownloadClientDraft(draft);
    const payload = {
      name: draft.name.trim(),
      clientType: normalizeDownloadClientType(draft.clientType),
      host: draft.host.trim(),
      baseUrl: buildDownloadClientBaseUrl(draft),
      config: buildDownloadClientConfigValues(
        draft,
        selectedDownloadClientConfigFields,
        editingStoredSecretKeys,
      ),
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
      await runConnectionFeedback({
        setGlobalStatus,
        startMessage: t("status.testingDownloadClient", {
          client: selectedDownloadClientLabel,
        }),
        successMessage: t("status.downloadClientConnectionTestPassed", {
          client: selectedDownloadClientLabel,
        }),
        failureFallbackMessage: t("status.downloadClientConnectionTestFailed", {
          client: selectedDownloadClientLabel,
        }),
        run: async () => {
          const { data: testData, error: testError } = await client
            .mutation(testDownloadClientConnectionMutation, {
              input: buildDownloadClientConnectionTestInput(
                editingDownloadClientId,
                payload.clientType,
                payload.config,
              ),
            })
            .toPromise();
          if (testError) throw testError;
          const validation = testData?.testDownloadClientConnection;
          if (validation?.status !== "ok") {
            throw new Error(
              validation?.message ?? t("status.downloadClientConnectionTestFailed", {
                client: selectedDownloadClientLabel,
              }),
            );
          }
        },
      });
    } catch {
      // Connection feedback is already surfaced through the shared helper.
    } finally {
      setIsTestingDownloadClientConnection(false);
    }
  };

  const moveDownloadClient = useCallback(async (clientId: string, direction: "up" | "down") => {
    if (isSavingOrder) {
      return;
    }

    const currentOrder =
      downloadClientOrder.length > 0
        ? downloadClientOrder
        : settingsDownloadClients.map((downloadClient) => downloadClient.id);
    const index = currentOrder.indexOf(clientId);
    if (index < 0) {
      return;
    }

    const nextIndex = direction === "up" ? index - 1 : index + 1;
    if (nextIndex < 0 || nextIndex >= currentOrder.length) {
      return;
    }

    const nextOrder = [...currentOrder];
    [nextOrder[index], nextOrder[nextIndex]] = [nextOrder[nextIndex], nextOrder[index]];
    setDownloadClientOrder(nextOrder);
    setIsSavingOrder(true);

    try {
      const { error } = await client.mutation(reorderDownloadClientsMutation, {
        input: { ids: nextOrder },
      }).toPromise();
      if (error) {
        throw error;
      }
      await refreshDownloadClients();
    } catch (error) {
      setDownloadClientOrder(currentOrder);
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setIsSavingOrder(false);
    }
  }, [client, downloadClientOrder, isSavingOrder, refreshDownloadClients, setGlobalStatus, settingsDownloadClients, t]);

  const editDownloadClient = useCallback((downloadClient: DownloadClientRecord) => {
    setEditingDownloadClientId(downloadClient.id);
    setDownloadClientDraft(buildDownloadClientDraftFromRecord(downloadClient));
    setGlobalStatus(t("status.editingDownloadClient", { name: downloadClient.name }));
  }, [setGlobalStatus, t]);

  const openEditEditor = useCallback((downloadClient: DownloadClientRecord) => {
    editDownloadClient(downloadClient);
    setEditorMode("edit");
    setIsEditorOpen(true);
    setAwaitingBaselineSync(true);
  }, [editDownloadClient]);

  const requestCreateEditor = useCallback(() => {
    if (!isEditorOpen || !isDraftDirty) {
      openCreateEditor();
      return;
    }

    setPendingEditorAction({ type: "create" });
  }, [isDraftDirty, isEditorOpen, openCreateEditor]);

  const requestEditDownloadClient = useCallback((downloadClient: DownloadClientRecord) => {
    if (!isEditorOpen || !isDraftDirty) {
      openEditEditor(downloadClient);
      return;
    }

    setPendingEditorAction({ type: "edit", downloadClient });
  }, [isDraftDirty, isEditorOpen, openEditEditor]);

  const requestCloseEditor = useCallback(() => {
    if (!isEditorOpen) {
      return;
    }

    if (!isDraftDirty) {
      setIsEditorOpen(false);
      setEditorMode("create");
      resetDownloadClientDraft();
      setAwaitingBaselineSync(true);
      return;
    }

    setPendingEditorAction({ type: "close" });
  }, [isDraftDirty, isEditorOpen, resetDownloadClientDraft]);

  const confirmPendingEditorAction = useCallback(() => {
    if (!pendingEditorAction) {
      return;
    }

    if (pendingEditorAction.type === "create") {
      openCreateEditor();
    } else if (pendingEditorAction.type === "edit") {
      openEditEditor(pendingEditorAction.downloadClient);
    } else {
      setIsEditorOpen(false);
      setEditorMode("create");
      resetDownloadClientDraft();
      setAwaitingBaselineSync(true);
    }

    setPendingEditorAction(null);
  }, [openCreateEditor, openEditEditor, pendingEditorAction, resetDownloadClientDraft]);

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
      void onDownloadClientsChanged?.();
      await refreshDownloadClients();
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setMutatingDownloadClientId(null);
    }
  }, [client, onDownloadClientsChanged, refreshDownloadClients, setGlobalStatus, t]);

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
      const { data, error } = await client.mutation(deleteDownloadClientMutation, {
        id: downloadClient.id,
      }).toPromise();
      if (error) throw error;
      const clearedIndexerMappingCount =
        data?.deleteDownloadClientConfig?.clearedIndexerMappingCount ?? 0;
      void onDownloadClientsChanged?.();
      await refreshDownloadClients();
      setGlobalStatus(
        t("status.downloadClientDeletedWithMappings", {
          name: downloadClient.name,
          count: clearedIndexerMappingCount,
        }),
      );
      if (editingDownloadClientId === downloadClient.id) {
        resetDownloadClientDraft();
        setIsEditorOpen(false);
        setEditorMode("create");
        setAwaitingBaselineSync(true);
      }
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToDelete"));
    } finally {
      setMutatingDownloadClientId(null);
      setPendingDeleteDownloadClient(null);
    }
  }, [
    client,
    editingDownloadClientId,
    pendingDeleteDownloadClient,
    onDownloadClientsChanged,
    refreshDownloadClients,
    resetDownloadClientDraft,
    setGlobalStatus,
    t,
  ]);

  return (
    <>
      {pluginsTarget
        ? createPortal(
            <FilteredPluginList
              family="DOWNLOAD_CLIENT"
              catalogVersion={providerCatalogVersion}
              extraPluginTypes={DOWNLOAD_CLIENT_ADJACENT_PLUGIN_TYPES}
              refreshProviderOptions={refreshProviderTypes}
            />,
            pluginsTarget,
          )
        : null}
      <SettingsDownloadClientsSection
        editingDownloadClientId={editingDownloadClientId}
        downloadClientTypeOptions={availableDownloadClientTypeOptions}
        downloadClientDraft={downloadClientDraft}
        setDownloadClientDraft={setDownloadClientDraft}
        submitDownloadClient={submitDownloadClient}
        testDownloadClientConnection={testDownloadClientConnection}
        isTestingDownloadClientConnection={isTestingDownloadClientConnection}
        mutatingDownloadClientId={mutatingDownloadClientId}
        resetDownloadClientDraft={requestCloseEditor}
        settingsDownloadClients={settingsDownloadClients}
        editDownloadClient={requestEditDownloadClient}
        toggleDownloadClientEnabled={toggleDownloadClientEnabled}
        deleteDownloadClient={deleteDownloadClient}
        downloadClientOrder={downloadClientOrder}
      moveDownloadClient={moveDownloadClient}
      isSavingOrder={isSavingOrder}
      isEditorOpen={isEditorOpen}
      editorMode={editorMode}
      localPathStyle={localPathStyle}
      startCreateDownloadClient={requestCreateEditor}
    />
      <ConfirmDialog
        open={pendingEditorAction !== null}
        title={t("settings.downloadClientConfirmDiscardTitle")}
        description={t("settings.downloadClientConfirmDiscardDescription")}
        confirmLabel={
          pendingEditorAction?.type === "create"
            ? t("settings.downloadClientCreateNew")
            : pendingEditorAction?.type === "edit"
              ? t("label.edit")
              : t("label.discard")
        }
        cancelLabel={t("label.cancel")}
        confirmButtonId="settings-download-client-editor-action-confirm"
        cancelButtonId="settings-download-client-editor-action-cancel"
        isBusy={mutatingDownloadClientId !== null}
        onConfirm={confirmPendingEditorAction}
        onCancel={() => setPendingEditorAction(null)}
      />
      <ConfirmDialog
        open={pendingDeleteDownloadClient !== null}
        contentId="settings-download-client-delete-dialog"
        title={t("label.delete")}
        description={
          pendingDeleteDownloadClient
            ? t("settings.downloadClientDeleteConfirmDescription", {
                name: pendingDeleteDownloadClient.name,
              })
            : ""
        }
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        confirmButtonId="settings-download-client-delete-confirm"
        cancelButtonId="settings-download-client-delete-cancel"
        isBusy={mutatingDownloadClientId !== null}
        onConfirm={confirmDeleteDownloadClient}
        onCancel={() => setPendingDeleteDownloadClient(null)}
      />
    </>
  );
}

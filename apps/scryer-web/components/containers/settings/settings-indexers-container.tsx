import {
  type ComponentProps,
  type FormEvent,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { SettingsIndexersSection } from "@/components/views/settings/settings-indexers-section";
import { FilteredPluginList } from "@/components/views/settings/filtered-plugin-list";
import { SETTINGS_REFERENCE_SLOT_ID } from "@/components/containers/settings/settings-container";
import { useClient } from "urql";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import type {
  ConfigFieldDef,
  IndexerProxyDraft,
  IndexerProxyRecord,
  IndexerRecord,
  ProviderTypeInfo,
} from "@/lib/types";
import { runConnectionFeedback } from "@/lib/utils/connection-feedback";
import {
  buildCreateIndexerProxyInput,
  buildUpdateIndexerProxyInput,
} from "@/lib/utils/settings-mutation-inputs";
import {
  indexerProviderTypesQuery,
  indexerProxyConfigsQuery,
  indexersInitQuery,
  indexersQuery,
} from "@/lib/graphql/queries";
import {
  createIndexerMutation,
  createIndexerProxyConfigMutation,
  deleteIndexerMutation,
  deleteIndexerProxyConfigMutation,
  syncIndexerConfigMutation,
  testIndexerConnectionMutation,
  testIndexerProxyConfigMutation,
  updateIndexerMutation,
  updateIndexerProxyConfigMutation,
} from "@/lib/graphql/mutations";
import {
  providerConfigRecordToValues,
  providerConfigValuesToRecord,
} from "@/lib/utils/provider-config";

type SettingsIndexersSectionProps = ComponentProps<
  typeof SettingsIndexersSection
>;

const INDEXER_INITIAL_DRAFT = {
  name: "",
  providerType: "",
  indexerProxyConfigId: null as string | null,
  storedSecretKeys: [] as string[],
  isEnabled: true,
  enableInteractiveSearch: true,
  enableAutoSearch: true,
  configValues: {} as Record<string, string>,
};

const INDEXER_PROXY_INITIAL_DRAFT: IndexerProxyDraft = {
  providerType: "byparr",
  name: "",
  baseUrl: "http://localhost:8191",
  requestTimeoutSeconds: 60,
  isEnabled: true,
};

function serializeConfigValues(
  fields: ConfigFieldDef[],
  configValues: Record<string, string>,
  storedSecretKeys: string[] = [],
): ReturnType<typeof providerConfigRecordToValues> | undefined {
  const entries: Record<string, string> = {};
  const storedSecretKeySet = new Set(storedSecretKeys);

  if (fields.length === 0) {
    for (const [key, value] of Object.entries(configValues)) {
      if (value.trim() !== "") {
        entries[key] = value;
      }
    }
    return Object.keys(entries).length > 0
      ? providerConfigRecordToValues(entries)
      : undefined;
  }

  const fieldKeySet = new Set(fields.map((field) => field.key));
  const secretInputKeys = fields
    .filter((field) => field.fieldType === "PASSWORD")
    .map((field) => field.key);
  for (const [key, value] of Object.entries(configValues)) {
    if (!fieldKeySet.has(key) && value.trim() !== "") {
      entries[key] = value;
    }
  }

  for (const field of fields) {
    if (field.valueSource === "HOST_BINDING") {
      continue;
    }

    const isStoredSecret =
      field.fieldType === "PASSWORD" && storedSecretKeySet.has(field.key);
    let nextValue =
      configValues[field.key] ??
      field.defaultValue ??
      (field.fieldType === "BOOL" ? "false" : "");

    if (isStoredSecret && nextValue.trim() === "") {
      continue;
    }

    if (field.fieldType === "BOOL") {
      entries[field.key] = nextValue.trim() || field.defaultValue || "false";
      continue;
    }

    if (nextValue.trim() === "" && field.defaultValue) {
      nextValue = field.defaultValue;
    }

    if (nextValue.trim() !== "") {
      entries[field.key] = nextValue;
    }
  }

  return Object.keys(entries).length > 0
    ? providerConfigRecordToValues(entries, secretInputKeys)
    : undefined;
}

function buildDraftConfigValues(
  fields: ConfigFieldDef[],
  parsedConfigValues: Record<string, string>,
  storedSecretKeys: string[] = [],
): Record<string, string> {
  if (fields.length === 0) {
    return { ...parsedConfigValues };
  }

  const nextValues = { ...parsedConfigValues };
  const storedSecretKeySet = new Set(storedSecretKeys);
  for (const field of fields) {
    if (field.valueSource === "HOST_BINDING") {
      continue;
    }

    if (field.fieldType === "PASSWORD" && storedSecretKeySet.has(field.key)) {
      nextValues[field.key] = "";
      continue;
    }

    nextValues[field.key] =
      parsedConfigValues[field.key] ??
      field.defaultValue ??
      (field.fieldType === "BOOL" ? "false" : "");
  }

  return nextValues;
}

function findMissingRequiredConfigField(
  fields: ConfigFieldDef[],
  configValues: Record<string, string>,
  storedSecretKeys: string[] = [],
): ConfigFieldDef | null {
  const storedSecretKeySet = new Set(storedSecretKeys);
  for (const field of fields) {
    if (!field.required || field.valueSource === "HOST_BINDING") {
      continue;
    }

    const nextValue =
      configValues[field.key] ??
      field.defaultValue ??
      (field.fieldType === "BOOL" ? "false" : "");

    if (
      field.fieldType === "PASSWORD" &&
      storedSecretKeySet.has(field.key) &&
      nextValue.trim() === ""
    ) {
      continue;
    }

    if (field.fieldType !== "BOOL" && nextValue.trim() === "") {
      return field;
    }
  }

  return null;
}

type SettingsIndexersContainerProps = {
  providerCatalogVersion?: number;
};

type PendingIndexerEditorAction =
  | { type: "create" }
  | { type: "edit"; indexer: IndexerRecord }
  | { type: "close" }
  | null;

function cloneIndexerDraft(
  draft: SettingsIndexersSectionProps["indexerDraft"],
): SettingsIndexersSectionProps["indexerDraft"] {
  return {
    ...draft,
    storedSecretKeys: [...draft.storedSecretKeys],
    configValues: { ...draft.configValues },
  };
}

export function SettingsIndexersContainer({
  providerCatalogVersion = 0,
}: SettingsIndexersContainerProps) {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const [settingsIndexers, setSettingsIndexers] = useState<IndexerRecord[]>([]);
  const [indexerProxyConfigs, setIndexerProxyConfigs] = useState<
    IndexerProxyRecord[]
  >([]);
  const [settingsIndexerFilter, setSettingsIndexerFilter] = useState("");
  const [mutatingIndexerId, setMutatingIndexerId] = useState<string | null>(
    null,
  );
  const [editingIndexerId, setEditingIndexerId] = useState<string | null>(null);
  const [pendingDeleteIndexer, setPendingDeleteIndexer] =
    useState<IndexerRecord | null>(null);
  const [pendingDeleteProxy, setPendingDeleteProxy] =
    useState<IndexerProxyRecord | null>(null);
  const [isTestingConnection, setIsTestingConnection] = useState(false);
  const [editingProxyId, setEditingProxyId] = useState<string | null>(null);
  const [isProxyEditorOpen, setIsProxyEditorOpen] = useState(false);
  const [mutatingProxyId, setMutatingProxyId] = useState<string | null>(null);
  const [testingProxyId, setTestingProxyId] = useState<string | null>(null);
  const [indexerProxyDraft, setIndexerProxyDraft] =
    useState<IndexerProxyDraft>(() => ({ ...INDEXER_PROXY_INITIAL_DRAFT }));
  const defaultIndexerProxyConfigId = useMemo(
    () => indexerProxyConfigs.find((proxy) => proxy.isEnabled)?.id ?? null,
    [indexerProxyConfigs],
  );
  const [providerTypes, setProviderTypes] = useState<ProviderTypeInfo[]>([]);
  const [pluginsTarget, setPluginsTarget] = useState<HTMLElement | null>(null);
  useEffect(() => {
    setPluginsTarget(document.getElementById(SETTINGS_REFERENCE_SLOT_ID));
  }, []);
  const [indexerDraft, setIndexerDraft] = useState<
    SettingsIndexersSectionProps["indexerDraft"]
  >(() => cloneIndexerDraft(INDEXER_INITIAL_DRAFT));
  const [isEditorOpen, setIsEditorOpen] = useState(false);
  const [editorMode, setEditorMode] = useState<"create" | "edit">("create");
  const [pendingEditorAction, setPendingEditorAction] =
    useState<PendingIndexerEditorAction>(null);
  const [draftBaseline, setDraftBaseline] = useState<
    SettingsIndexersSectionProps["indexerDraft"]
  >(() => cloneIndexerDraft(INDEXER_INITIAL_DRAFT));
  const [awaitingBaselineSync, setAwaitingBaselineSync] = useState(false);
  const didMountRef = useRef(false);
  const providerCatalogVersionRef = useRef(providerCatalogVersion);

  const resetIndexerDraft = useCallback(() => {
    setEditingIndexerId(null);
    setIndexerDraft(() =>
      cloneIndexerDraft({
        ...INDEXER_INITIAL_DRAFT,
        indexerProxyConfigId: defaultIndexerProxyConfigId,
      }),
    );
  }, [defaultIndexerProxyConfigId]);

  useEffect(() => {
    if (!awaitingBaselineSync) {
      return;
    }

    setDraftBaseline(cloneIndexerDraft(indexerDraft));
    setAwaitingBaselineSync(false);
  }, [awaitingBaselineSync, indexerDraft]);

  const isDraftDirty =
    JSON.stringify(indexerDraft) !== JSON.stringify(draftBaseline);

  const refreshIndexers = useCallback(async () => {
    try {
      const { data, error } = await client
        .query(indexersQuery, {
          providerType: settingsIndexerFilter || undefined,
        }, {
          requestPolicy: "network-only",
        })
        .toPromise();
      if (error) throw error;
      setSettingsIndexers(data.indexers || []);
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.failedToLoad"),
      );
    }
  }, [client, settingsIndexerFilter, setGlobalStatus, t]);

  const refreshIndexerProxyConfigs = useCallback(async () => {
    try {
      const { data, error } = await client
        .query(indexerProxyConfigsQuery, {}, { requestPolicy: "network-only" })
        .toPromise();
      if (error) throw error;
      setIndexerProxyConfigs(data?.indexerProxyConfigs || []);
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.failedToLoad"),
      );
    }
  }, [client, setGlobalStatus, t]);

  const refreshProviderTypes = useCallback(async () => {
    const { data, error } = await client
      .query(indexerProviderTypesQuery, {}, { requestPolicy: "network-only" })
      .toPromise();
    if (error) throw error;
    setProviderTypes(data?.indexerProviderTypes || []);
  }, [client]);

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      try {
        const { data, error } = await client
          .query(indexersInitQuery, {}, { requestPolicy: "network-only" })
          .toPromise();
        if (error && !data?.indexers) throw error;
        if (cancelled) return;
        setSettingsIndexers(data?.indexers || []);
        setIndexerProxyConfigs(data?.indexerProxyConfigs || []);
        setProviderTypes(data?.indexerProviderTypes || []);
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.failedToLoad"),
        );
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
    void Promise.all([
      refreshProviderTypes(),
      refreshIndexers(),
      refreshIndexerProxyConfigs(),
    ]).catch((error: unknown) => {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.failedToLoad"),
      );
    });
  }, [
    providerCatalogVersion,
    refreshIndexerProxyConfigs,
    refreshIndexers,
    refreshProviderTypes,
    setGlobalStatus,
    t,
  ]);

  useEffect(() => {
    if (editingIndexerId || providerTypes.length === 0) {
      return;
    }

    setIndexerDraft((prev) => {
      const configuredProvider =
        providerTypes.find(
          (providerType) => providerType.providerType === prev.providerType,
        ) ?? null;
      const nextProvider = configuredProvider ?? providerTypes[0] ?? null;
      if (!nextProvider) {
        return prev;
      }

      const shouldAutofillName =
        prev.name.trim().length === 0 ||
        prev.name === (configuredProvider?.name ?? prev.providerType);
      const nextProviderType = configuredProvider
        ? prev.providerType
        : nextProvider.providerType;
      const nextName = shouldAutofillName ? nextProvider.name : prev.name;

      if (nextProviderType === prev.providerType && nextName === prev.name) {
        return prev;
      }

      return {
        ...prev,
        providerType: nextProviderType,
        name: nextName,
        configValues:
          nextProviderType === prev.providerType
            ? prev.configValues
            : buildDraftConfigValues(nextProvider.configFields, {}),
      };
    });
  }, [editingIndexerId, providerTypes]);

  useEffect(() => {
    if (!didMountRef.current) {
      didMountRef.current = true;
      return;
    }
    void refreshIndexers();
  }, [refreshIndexers]);

  const openCreateEditor = useCallback(() => {
    resetIndexerDraft();
    setEditorMode("create");
    setIsEditorOpen(true);
    setAwaitingBaselineSync(true);
  }, [resetIndexerDraft]);

  const submitIndexer = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const normalizedProviderType = indexerDraft.providerType.trim().toLowerCase();
    const selectedProvider =
      providerTypes.find((pt) => pt.providerType === normalizedProviderType) ?? null;
    const missingRequiredConfigField = findMissingRequiredConfigField(
      selectedProvider?.configFields ?? [],
      indexerDraft.configValues,
      indexerDraft.storedSecretKeys,
    );
    const payload = {
      name: indexerDraft.name.trim(),
      providerType: normalizedProviderType,
      indexerProxyConfigId: indexerDraft.indexerProxyConfigId,
      isEnabled: indexerDraft.isEnabled,
      enableInteractiveSearch: indexerDraft.enableInteractiveSearch,
      enableAutoSearch: indexerDraft.enableAutoSearch,
      config: serializeConfigValues(
        selectedProvider?.configFields ?? [],
        indexerDraft.configValues,
        indexerDraft.storedSecretKeys,
      ),
    };

    if (!payload.name || !payload.providerType) {
      setGlobalStatus(t("form.indexerValidation"));
      return;
    }

    if (missingRequiredConfigField) {
      setGlobalStatus(`${missingRequiredConfigField.label}: ${t("setup.required")}`);
      return;
    }

    setMutatingIndexerId(editingIndexerId || "new");
    try {
      if (editingIndexerId) {
        const { error } = await client
          .mutation(updateIndexerMutation, {
            input: {
              id: editingIndexerId,
              name: payload.name,
              providerType: payload.providerType,
              indexerProxyConfigId: payload.indexerProxyConfigId,
              isEnabled: payload.isEnabled,
              enableInteractiveSearch: payload.enableInteractiveSearch,
              enableAutoSearch: payload.enableAutoSearch,
              config: payload.config,
            },
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.indexerUpdated"));
      } else {
        const { error } = await client
          .mutation(createIndexerMutation, {
            input: {
              name: payload.name,
              providerType: payload.providerType,
              indexerProxyConfigId: payload.indexerProxyConfigId,
              isEnabled: payload.isEnabled,
              enableInteractiveSearch: payload.enableInteractiveSearch,
              enableAutoSearch: payload.enableAutoSearch,
              config: payload.config,
            },
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.indexerCreated"));
      }
      resetIndexerDraft();
      setIsEditorOpen(false);
      setEditorMode("create");
      setAwaitingBaselineSync(true);
      await refreshIndexers();
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.failedToUpdate"),
      );
    } finally {
      setMutatingIndexerId(null);
    }
  };

  const editIndexer = useCallback((indexer: IndexerRecord) => {
    if (indexer.isManaged) {
      setGlobalStatus(t("settings.managedIndexerReadOnly"));
      return;
    }
    const selectedProvider =
      providerTypes.find(
        (providerType) =>
          providerType.providerType === indexer.providerType.trim().toLowerCase(),
      ) ?? null;
    const parsedConfigValues = providerConfigValuesToRecord(indexer.config);
    setEditingIndexerId(indexer.id);
    setIndexerDraft({
      name: indexer.name,
      providerType: indexer.providerType,
      indexerProxyConfigId: indexer.indexerProxyConfigId ?? null,
      storedSecretKeys: indexer.storedSecretKeys,
      isEnabled: indexer.isEnabled,
      enableInteractiveSearch: indexer.enableInteractiveSearch,
      enableAutoSearch: indexer.enableAutoSearch,
      configValues: buildDraftConfigValues(
        selectedProvider?.configFields ?? [],
        parsedConfigValues,
        indexer.storedSecretKeys,
      ),
    });
    setGlobalStatus(t("status.editingIndexer", { name: indexer.name }));
  }, [providerTypes, setGlobalStatus, t]);

  const openEditEditor = useCallback((indexer: IndexerRecord) => {
    editIndexer(indexer);
    setEditorMode("edit");
    setIsEditorOpen(true);
    setAwaitingBaselineSync(true);
  }, [editIndexer]);

  const requestCreateEditor = useCallback(() => {
    if (!isEditorOpen || !isDraftDirty) {
      openCreateEditor();
      return;
    }

    setPendingEditorAction({ type: "create" });
  }, [isDraftDirty, isEditorOpen, openCreateEditor]);

  const requestEditIndexer = useCallback((indexer: IndexerRecord) => {
    if (!isEditorOpen || !isDraftDirty) {
      openEditEditor(indexer);
      return;
    }

    setPendingEditorAction({ type: "edit", indexer });
  }, [isDraftDirty, isEditorOpen, openEditEditor]);

  const requestCloseEditor = useCallback(() => {
    if (!isEditorOpen) {
      return;
    }

    if (!isDraftDirty) {
      setIsEditorOpen(false);
      setEditorMode("create");
      resetIndexerDraft();
      setAwaitingBaselineSync(true);
      return;
    }

    setPendingEditorAction({ type: "close" });
  }, [isDraftDirty, isEditorOpen, resetIndexerDraft]);

  const confirmPendingEditorAction = useCallback(() => {
    if (!pendingEditorAction) {
      return;
    }

    if (pendingEditorAction.type === "create") {
      openCreateEditor();
    } else if (pendingEditorAction.type === "edit") {
      openEditEditor(pendingEditorAction.indexer);
    } else {
      setIsEditorOpen(false);
      setEditorMode("create");
      resetIndexerDraft();
      setAwaitingBaselineSync(true);
    }

    setPendingEditorAction(null);
  }, [openCreateEditor, openEditEditor, pendingEditorAction, resetIndexerDraft]);

  const deleteIndexer = async (indexer: IndexerRecord) => {
    if (indexer.isManaged) {
      setGlobalStatus(t("settings.managedIndexerReadOnly"));
      return;
    }
    setPendingDeleteIndexer(indexer);
  };

  const toggleIndexerEnabled = useCallback(
    async (indexer: IndexerRecord) => {
      if (indexer.isManaged) {
        setGlobalStatus(t("settings.managedIndexerReadOnly"));
        return;
      }
      const nextIsEnabled = !indexer.isEnabled;
      setMutatingIndexerId(indexer.id);
      try {
        const { error } = await client
          .mutation(updateIndexerMutation, {
            input: {
              id: indexer.id,
              isEnabled: nextIsEnabled,
            },
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.indexerUpdated"));
        await refreshIndexers();
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.failedToUpdate"),
        );
      } finally {
        setMutatingIndexerId(null);
      }
    },
    [client, refreshIndexers, setGlobalStatus, t],
  );

  const syncIndexer = useCallback(
    async (indexer: IndexerRecord) => {
      if (!indexer.supportsManagedChildrenSync || indexer.isManaged) {
        return;
      }
      setMutatingIndexerId(indexer.id);
      try {
        const { error } = await client
          .mutation(syncIndexerConfigMutation, {
            id: indexer.id,
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.indexerSynced", { name: indexer.name }));
        await refreshIndexers();
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.failedToUpdate"),
        );
      } finally {
        setMutatingIndexerId(null);
      }
    },
    [client, refreshIndexers, setGlobalStatus, t],
  );

  const confirmDeleteIndexer = async () => {
    if (!pendingDeleteIndexer) {
      return;
    }
    const indexer = pendingDeleteIndexer;
    setMutatingIndexerId(indexer.id);
    try {
      const { error } = await client
        .mutation(deleteIndexerMutation, {
          id: indexer.id,
        })
        .toPromise();
      if (error) throw error;
      setGlobalStatus(t("status.indexerDeleted", { name: indexer.name }));
      await refreshIndexers();
      if (editingIndexerId === indexer.id) {
        resetIndexerDraft();
        setIsEditorOpen(false);
        setEditorMode("create");
        setAwaitingBaselineSync(true);
      }
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.failedToDelete"),
      );
    } finally {
      setMutatingIndexerId(null);
      setPendingDeleteIndexer(null);
    }
  };

  const testIndexerConnection = async () => {
    const normalizedProviderType = indexerDraft.providerType.trim().toLowerCase();
    const selectedProvider =
      providerTypes.find((pt) => pt.providerType === normalizedProviderType) ?? null;
    const missingRequiredConfigField = findMissingRequiredConfigField(
      selectedProvider?.configFields ?? [],
      indexerDraft.configValues,
      indexerDraft.storedSecretKeys,
    );
    const payload = {
      providerType: normalizedProviderType,
      indexerProxyConfigId: indexerDraft.indexerProxyConfigId,
      config: serializeConfigValues(
        selectedProvider?.configFields ?? [],
        indexerDraft.configValues,
        indexerDraft.storedSecretKeys,
      ),
      indexerId: editingIndexerId ?? undefined,
    };

    if (!payload.providerType) {
      setGlobalStatus(t("form.indexerValidation"));
      return;
    }
    if (missingRequiredConfigField) {
      setGlobalStatus(`${missingRequiredConfigField.label}: ${t("setup.required")}`);
      return;
    }
    setIsTestingConnection(true);
    try {
      await runConnectionFeedback({
        setGlobalStatus,
        startMessage: t("status.testingIndexerConnection"),
        successMessage: t("status.indexerConnectionTestPassed"),
        failureFallbackMessage: t("status.indexerConnectionTestFailed"),
        run: async () => {
          const { data: testData, error: testError } = await client
            .mutation(testIndexerConnectionMutation, { input: payload })
            .toPromise();
          if (testError) throw testError;
          const validation = testData?.testIndexerConnection;
          if (validation?.status !== "ok") {
            throw new Error(
              validation?.message ?? t("status.indexerConnectionTestFailed"),
            );
          }
          await refreshIndexers();
        },
      });
    } catch {
      // Connection feedback is already surfaced through the shared helper.
    } finally {
      setIsTestingConnection(false);
    }
  };

  const resetIndexerProxyDraft = useCallback(() => {
    setEditingProxyId(null);
    setIsProxyEditorOpen(false);
    setIndexerProxyDraft({ ...INDEXER_PROXY_INITIAL_DRAFT });
  }, []);

  const editIndexerProxy = useCallback((proxy: IndexerProxyRecord) => {
    setEditingProxyId(proxy.id);
    setIsProxyEditorOpen(true);
    setIndexerProxyDraft({
      providerType: proxy.providerType === "trawl" ? "trawl" : "byparr",
      name: proxy.name,
      baseUrl: proxy.baseUrl,
      requestTimeoutSeconds: proxy.requestTimeoutSeconds,
      isEnabled: proxy.isEnabled,
    });
    setGlobalStatus(`Editing indexer proxy ${proxy.name}`);
  }, [setGlobalStatus]);

  const startCreateIndexerProxy = useCallback(() => {
    setEditingProxyId(null);
    setIndexerProxyDraft({ ...INDEXER_PROXY_INITIAL_DRAFT });
    setIsProxyEditorOpen(true);
  }, []);

  const submitIndexerProxy = useCallback(async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const name = indexerProxyDraft.name.trim();
    const baseUrl = indexerProxyDraft.baseUrl.trim();
    if (!name || !baseUrl) {
      setGlobalStatus(t("form.indexerValidation"));
      return;
    }

    setMutatingProxyId(editingProxyId || "new");
    try {
      if (editingProxyId) {
        const { error } = await client
          .mutation(updateIndexerProxyConfigMutation, {
            input: buildUpdateIndexerProxyInput(editingProxyId, indexerProxyDraft),
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus("Indexer proxy updated");
      } else {
        const { error } = await client
          .mutation(createIndexerProxyConfigMutation, {
            input: buildCreateIndexerProxyInput(indexerProxyDraft),
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus("Indexer proxy created");
      }
      resetIndexerProxyDraft();
      await Promise.all([refreshIndexerProxyConfigs(), refreshIndexers()]);
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.failedToUpdate"),
      );
    } finally {
      setMutatingProxyId(null);
    }
  }, [
    client,
    editingProxyId,
    indexerProxyDraft,
    refreshIndexerProxyConfigs,
    refreshIndexers,
    resetIndexerProxyDraft,
    setGlobalStatus,
    t,
  ]);

  const testIndexerProxy = useCallback(async (proxy: IndexerProxyRecord) => {
    setTestingProxyId(proxy.id);
    try {
      const { data, error } = await client
        .mutation(testIndexerProxyConfigMutation, { id: proxy.id })
        .toPromise();
      if (error) throw error;
      const result = data?.testIndexerProxyConfig;
      setGlobalStatus(
        result?.message ||
          (result?.ok ? "Indexer proxy test passed" : "Indexer proxy test failed"),
      );
      await refreshIndexerProxyConfigs();
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : "Indexer proxy test failed",
      );
    } finally {
      setTestingProxyId(null);
    }
  }, [client, refreshIndexerProxyConfigs, setGlobalStatus]);

  const deleteIndexerProxy = useCallback((proxy: IndexerProxyRecord) => {
    setPendingDeleteProxy(proxy);
  }, []);

  const confirmDeleteIndexerProxy = useCallback(async () => {
    if (!pendingDeleteProxy) {
      return;
    }
    const proxy = pendingDeleteProxy;
    setMutatingProxyId(proxy.id);
    try {
      const { error } = await client
        .mutation(deleteIndexerProxyConfigMutation, { id: proxy.id })
        .toPromise();
      if (error) throw error;
      setGlobalStatus("Indexer proxy deleted");
      if (editingProxyId === proxy.id) {
        resetIndexerProxyDraft();
      }
      await Promise.all([refreshIndexerProxyConfigs(), refreshIndexers()]);
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.failedToDelete"),
      );
    } finally {
      setMutatingProxyId(null);
      setPendingDeleteProxy(null);
    }
  }, [
    client,
    editingProxyId,
    pendingDeleteProxy,
    refreshIndexerProxyConfigs,
    refreshIndexers,
    resetIndexerProxyDraft,
    setGlobalStatus,
    t,
  ]);

  return (
    <>
      {pluginsTarget
        ? createPortal(
            <FilteredPluginList
              family="INDEXER"
              refreshProviderOptions={refreshProviderTypes}
            />,
            pluginsTarget,
          )
        : null}
      <SettingsIndexersSection
        editingIndexerId={editingIndexerId}
        indexerDraft={indexerDraft}
        setIndexerDraft={setIndexerDraft}
        submitIndexer={submitIndexer}
        mutatingIndexerId={mutatingIndexerId}
        resetIndexerDraft={requestCloseEditor}
        settingsIndexerFilter={settingsIndexerFilter}
        setSettingsIndexerFilter={setSettingsIndexerFilter}
        settingsIndexers={settingsIndexers}
        indexerProxyConfigs={indexerProxyConfigs}
        indexerProxyDraft={indexerProxyDraft}
        setIndexerProxyDraft={setIndexerProxyDraft}
        editingProxyId={editingProxyId}
        isProxyEditorOpen={isProxyEditorOpen}
        mutatingProxyId={mutatingProxyId}
        testingProxyId={testingProxyId}
        submitIndexerProxy={submitIndexerProxy}
        resetIndexerProxyDraft={resetIndexerProxyDraft}
        startCreateIndexerProxy={startCreateIndexerProxy}
        editIndexerProxy={editIndexerProxy}
        testIndexerProxy={testIndexerProxy}
        deleteIndexerProxy={deleteIndexerProxy}
        editIndexer={requestEditIndexer}
        toggleIndexerEnabled={toggleIndexerEnabled}
        deleteIndexer={deleteIndexer}
        syncIndexer={syncIndexer}
        providerTypes={providerTypes}
        testIndexerConnection={testIndexerConnection}
        isTestingConnection={isTestingConnection}
        isEditorOpen={isEditorOpen}
        editorMode={editorMode}
        startCreateIndexer={requestCreateEditor}
      />
      <ConfirmDialog
        open={pendingEditorAction !== null}
        title={t("settings.indexerConfirmDiscardTitle")}
        description={t("settings.indexerConfirmDiscardDescription")}
        confirmLabel={
          pendingEditorAction?.type === "create"
            ? t("settings.indexerCreateNew")
            : pendingEditorAction?.type === "edit"
              ? t("label.edit")
              : t("label.discard")
        }
        cancelLabel={t("label.cancel")}
        isBusy={mutatingIndexerId !== null}
        onConfirm={confirmPendingEditorAction}
        onCancel={() => setPendingEditorAction(null)}
      />
      <ConfirmDialog
        open={pendingDeleteIndexer !== null}
        contentId="settings-indexer-delete-dialog"
        title={t("label.delete")}
        description={
          pendingDeleteIndexer
            ? t("status.deletingIndexer", { name: pendingDeleteIndexer.name })
            : ""
        }
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        confirmButtonId="settings-indexer-delete-confirm"
        cancelButtonId="settings-indexer-delete-cancel"
        isBusy={mutatingIndexerId !== null}
        onConfirm={confirmDeleteIndexer}
        onCancel={() => setPendingDeleteIndexer(null)}
      />
      <ConfirmDialog
        open={pendingDeleteProxy !== null}
        contentId="settings-indexer-proxy-delete-dialog"
        title={t("label.delete")}
        description={
          pendingDeleteProxy
            ? `Delete indexer proxy ${pendingDeleteProxy.name}?`
            : ""
        }
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        confirmButtonId="settings-indexer-proxy-delete-confirm"
        cancelButtonId="settings-indexer-proxy-delete-cancel"
        isBusy={mutatingProxyId !== null}
        onConfirm={confirmDeleteIndexerProxy}
        onCancel={() => setPendingDeleteProxy(null)}
      />
    </>
  );
}

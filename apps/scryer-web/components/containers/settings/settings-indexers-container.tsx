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
import { useIndexersSubscription } from "@/lib/hooks/use-indexers-subscription";
import type { IndexerSettingsTab } from "@/components/root/types";
import type {
  ConfigFieldDef,
  ProxyDraft,
  ProxyProviderTypeValue,
  ProxyRecord,
  IndexerRecord,
  ProviderTypeInfo,
  IndexerDownloadClientMappingCatalog,
  IndexerDownloadClientMappingCatalogResource,
} from "@/lib/types";
import {
  isProxyProviderType,
  supportsProxyRemoteDns,
} from "@/lib/types";
import { runConnectionFeedback } from "@/lib/utils/connection-feedback";
import {
  buildCreateProxyInput,
  buildUpdateProxyInput,
} from "@/lib/utils/settings-mutation-inputs";
import {
  getIndexerDownloadClientDraftMappingViewModel,
  updateIndexerDownloadClientMapping,
  updatePendingIndexerMappingIds,
} from "@/lib/utils/indexer-download-client-mapping";
import {
  indexerProviderTypesQuery,
  proxyConfigsQuery,
  indexersInitQuery,
  indexersQuery,
} from "@/lib/graphql/queries";
import {
  createIndexerMutation,
  createProxyConfigMutation,
  deleteIndexerMutation,
  deleteProxyConfigMutation,
  syncIndexerConfigMutation,
  setIndexerDownloadClientMappingMutation,
  setIndexerSeedingProfileMutation,
  testIndexerConnectionMutation,
  testProxyConfigMutation,
  updateIndexerMutation,
  updateProxyConfigMutation,
} from "@/lib/graphql/mutations";
import {
  providerConfigRecordToValues,
  providerConfigValuesToRecord,
} from "@/lib/utils/provider-config";
import { useSeedingProfileOptions } from "@/lib/hooks/use-seeding-profile-options";

type SettingsIndexersSectionProps = ComponentProps<
  typeof SettingsIndexersSection
>;

const INDEXER_INITIAL_DRAFT = {
  name: "",
  providerType: "",
  proxyConfigId: null as string | null,
  downloadClientId: null as string | null,
  seedingProfileId: null as string | null,
  storedSecretKeys: [] as string[],
  isEnabled: true,
  enableInteractiveSearch: true,
  enableAutoSearch: true,
  configValues: {} as Record<string, string>,
};

const PROXY_INITIAL_DRAFT: ProxyDraft = {
  providerType: "byparr",
  name: "",
  baseUrl: "http://localhost:8191",
  requestTimeoutSeconds: 60,
  username: "",
  password: "",
  hasStoredCredentials: false,
  clearCredentials: false,
  remoteDns: false,
  isEnabled: true,
};

/**
 * Each provider's base URL has to match its own scheme, so switching provider
 * in the editor reseeds the placeholder rather than leaving a solver URL on a
 * SOCKS row.
 */
const PROXY_DEFAULT_BASE_URLS: Record<
  ProxyProviderTypeValue,
  string
> = {
  byparr: "http://localhost:8191",
  trawl: "http://localhost:8191",
  http: "http://localhost:3128",
  socks4: "socks4://localhost:1080",
  socks5: "socks5://localhost:1080",
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
  indexerSettingsTab?: IndexerSettingsTab;
  providerCatalogVersion?: number;
  indexerDownloadClientMappingCatalogResource: IndexerDownloadClientMappingCatalogResource;
  updateIndexerDownloadClientMappingCatalog: (
    updater: (
      catalog: IndexerDownloadClientMappingCatalog,
    ) => IndexerDownloadClientMappingCatalog,
  ) => void;
  refreshIndexerDownloadClientMappingCatalog: () => Promise<void>;
};

const EMPTY_INDEXER_DOWNLOAD_CLIENT_MAPPING_CATALOG: IndexerDownloadClientMappingCatalog = {
  clients: [],
  indexers: [],
  providerCompatibility: [],
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
  indexerSettingsTab = "indexers",
  providerCatalogVersion = 0,
  indexerDownloadClientMappingCatalogResource,
  updateIndexerDownloadClientMappingCatalog,
  refreshIndexerDownloadClientMappingCatalog,
}: SettingsIndexersContainerProps) {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const [settingsIndexers, setSettingsIndexers] = useState<IndexerRecord[]>([]);
  const indexerDownloadClientMappingCatalog =
    indexerDownloadClientMappingCatalogResource.catalog ??
    EMPTY_INDEXER_DOWNLOAD_CLIENT_MAPPING_CATALOG;
  const [mutatingIndexerMappingIds, setMutatingIndexerMappingIds] = useState<Set<string>>(
    () => new Set(),
  );
  const { options: seedingProfileOptions } = useSeedingProfileOptions();
  const [
    mutatingIndexerSeedingProfileIds,
    setMutatingIndexerSeedingProfileIds,
  ] = useState<Set<string>>(() => new Set());
  const [proxyConfigs, setProxyConfigs] = useState<
    ProxyRecord[]
  >([]);
  const [settingsIndexerFilter, setSettingsIndexerFilter] = useState("");
  const [mutatingIndexerId, setMutatingIndexerId] = useState<string | null>(
    null,
  );
  const [editingIndexerId, setEditingIndexerId] = useState<string | null>(null);
  const [pendingDeleteIndexer, setPendingDeleteIndexer] =
    useState<IndexerRecord | null>(null);
  const [pendingDeleteProxy, setPendingDeleteProxy] =
    useState<ProxyRecord | null>(null);
  const [isTestingConnection, setIsTestingConnection] = useState(false);
  const [editingProxyId, setEditingProxyId] = useState<string | null>(null);
  const [isProxyEditorOpen, setIsProxyEditorOpen] = useState(false);
  const [mutatingProxyId, setMutatingProxyId] = useState<string | null>(null);
  const [testingProxyId, setTestingProxyId] = useState<string | null>(null);
  const [proxyDraft, setProxyDraft] =
    useState<ProxyDraft>(() => ({ ...PROXY_INITIAL_DRAFT }));
  const defaultProxyConfigId = useMemo(
    () => proxyConfigs.find((proxy) => proxy.isEnabled)?.id ?? null,
    [proxyConfigs],
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
        proxyConfigId: defaultProxyConfigId,
      }),
    );
  }, [defaultProxyConfigId]);

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

  const refreshProxyConfigs = useCallback(async () => {
    try {
      const { data, error } = await client
        .query(proxyConfigsQuery, {}, { requestPolicy: "network-only" })
        .toPromise();
      if (error) throw error;
      setProxyConfigs(data?.proxyConfigs || []);
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
        setProxyConfigs(data?.proxyConfigs || []);
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

  useIndexersSubscription(() => {
    void refreshIndexers();
    void refreshIndexerDownloadClientMappingCatalog();
  });

  useEffect(() => {
    if (providerCatalogVersion === providerCatalogVersionRef.current) {
      return;
    }

    providerCatalogVersionRef.current = providerCatalogVersion;
    void Promise.all([
      refreshProviderTypes(),
      refreshIndexers(),
      refreshIndexerDownloadClientMappingCatalog(),
      refreshProxyConfigs(),
    ]).catch((error: unknown) => {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.failedToLoad"),
      );
    });
  }, [
    providerCatalogVersion,
    refreshIndexerDownloadClientMappingCatalog,
    refreshProxyConfigs,
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

  // Editor-form variant of the assignment: no optimistic row update and no
  // status toast, because the surrounding save already reports its own result.
  const applyIndexerSeedingProfile = useCallback(
    async (indexerId: string, seedingProfileId: string | null) => {
      const { error } = await client
        .mutation(setIndexerSeedingProfileMutation, {
          input: { indexerId, seedingProfileId },
        })
        .toPromise();
      if (error) throw error;
    },
    [client],
  );

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
      proxyConfigId: indexerDraft.proxyConfigId,
      downloadClientId: indexerDraft.downloadClientId,
      seedingProfileId: indexerDraft.seedingProfileId,
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

    if (indexerDownloadClientMappingCatalogResource.catalog) {
      const mappingModel = getIndexerDownloadClientDraftMappingViewModel(
        payload.providerType,
        payload.downloadClientId,
        indexerDownloadClientMappingCatalogResource.catalog,
      );
      if (
        payload.downloadClientId &&
        (mappingModel.isNotApplicable ||
          mappingModel.invalidReason === "missing" ||
          mappingModel.invalidReason === "incompatible")
      ) {
        setGlobalStatus(t("settings.indexerDownloadClientSelectionInvalid"));
        return;
      }
    }

    setMutatingIndexerId(editingIndexerId || "new");
    try {
      if (editingIndexerId) {
        const existingIndexer = settingsIndexers.find(
          (indexer) => indexer.id === editingIndexerId,
        );
        const existingDownloadClientId = existingIndexer?.downloadClientId ?? null;
        const { error } = await client
          .mutation(updateIndexerMutation, {
            input: {
              id: editingIndexerId,
              name: payload.name,
              providerType: payload.providerType,
              proxyConfigId: payload.proxyConfigId,
              isEnabled: payload.isEnabled,
              enableInteractiveSearch: payload.enableInteractiveSearch,
              enableAutoSearch: payload.enableAutoSearch,
              config: payload.config,
              ...(payload.downloadClientId !== existingDownloadClientId
                ? { downloadClientId: payload.downloadClientId }
                : {}),
            },
          })
          .toPromise();
        if (error) throw error;
        // Seeding-profile assignment is not part of the indexer input: it goes
        // through its own mutation so the torrent-capability check stays
        // single-sourced server-side.
        if (
          payload.seedingProfileId !== (existingIndexer?.seedingProfileId ?? null)
        ) {
          await applyIndexerSeedingProfile(
            editingIndexerId,
            payload.seedingProfileId,
          );
        }
        setGlobalStatus(t("status.indexerUpdated"));
      } else {
        const { data, error } = await client
          .mutation(createIndexerMutation, {
            input: {
              name: payload.name,
              providerType: payload.providerType,
              proxyConfigId: payload.proxyConfigId,
              isEnabled: payload.isEnabled,
              enableInteractiveSearch: payload.enableInteractiveSearch,
              enableAutoSearch: payload.enableAutoSearch,
              downloadClientId: payload.downloadClientId,
              config: payload.config,
            },
          })
          .toPromise();
        if (error) throw error;
        const createdIndexerId = data?.createIndexerConfig?.id as
          | string
          | undefined;
        if (payload.seedingProfileId && createdIndexerId) {
          await applyIndexerSeedingProfile(
            createdIndexerId,
            payload.seedingProfileId,
          );
        }
        setGlobalStatus(t("status.indexerCreated"));
      }
      resetIndexerDraft();
      setIsEditorOpen(false);
      setEditorMode("create");
      setAwaitingBaselineSync(true);
      await Promise.all([
        refreshIndexers(),
        refreshIndexerDownloadClientMappingCatalog(),
      ]);
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
      proxyConfigId: indexer.proxyConfigId ?? null,
      downloadClientId: indexer.downloadClientId ?? null,
      seedingProfileId: indexer.seedingProfileId ?? null,
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

  const setIndexerDownloadClientMapping = useCallback(
    async (indexerId: string, downloadClientId: string | null) => {
      const indexerName =
        settingsIndexers.find((indexer) => indexer.id === indexerId)?.name ??
        indexerId;
      const previousMapping = indexerDownloadClientMappingCatalog.indexers.find(
        (entry) => entry.id === indexerId,
      );
      const previousDownloadClientId = previousMapping?.downloadClientId ?? null;
      const selectedClient = downloadClientId
        ? indexerDownloadClientMappingCatalog.clients.find(
            (clientRecord) => clientRecord.id === downloadClientId,
          )
        : null;

      setMutatingIndexerMappingIds((previous) =>
        updatePendingIndexerMappingIds(previous, indexerId, true),
      );
      updateIndexerDownloadClientMappingCatalog((previous) =>
        updateIndexerDownloadClientMapping(previous, indexerId, downloadClientId),
      );
      if (selectedClient?.isEnabled === false) {
        setGlobalStatus(
          t("settings.indexerDownloadClientDisabledWarning", {
            name: selectedClient.name,
          }),
        );
      }
      setGlobalStatus(t("status.indexerDownloadClientMappingSaving"));

      try {
        const { data, error } = await client
          .mutation(setIndexerDownloadClientMappingMutation, {
            input: {
              indexerId,
              downloadClientId,
            },
          })
          .toPromise();
        if (error) throw error;

        const response = data?.setIndexerDownloadClientMapping;
        const resolvedDownloadClientId = response
          ? response.downloadClientId ?? null
          : downloadClientId;
        updateIndexerDownloadClientMappingCatalog((previous) =>
          updateIndexerDownloadClientMapping(
            previous,
            indexerId,
            resolvedDownloadClientId,
          ),
        );
        setGlobalStatus(
          t("status.indexerDownloadClientMappingSaved", {
            name: indexerName,
          }),
        );
        await Promise.all([
          refreshIndexers(),
          refreshIndexerDownloadClientMappingCatalog(),
        ]);
      } catch (error) {
        updateIndexerDownloadClientMappingCatalog((previous) =>
          updateIndexerDownloadClientMapping(
            previous,
            indexerId,
            previousMapping ? previousDownloadClientId : null,
          ),
        );
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.failedToUpdate"),
        );
      } finally {
        setMutatingIndexerMappingIds((previous) =>
          updatePendingIndexerMappingIds(previous, indexerId, false),
        );
      }
    },
    [
      client,
      indexerDownloadClientMappingCatalog,
      refreshIndexerDownloadClientMappingCatalog,
      refreshIndexers,
      setGlobalStatus,
      settingsIndexers,
      t,
      updateIndexerDownloadClientMappingCatalog,
    ],
  );

  const setIndexerSeedingProfile = useCallback(
    async (indexerId: string, seedingProfileId: string | null) => {
      const indexer = settingsIndexers.find((entry) => entry.id === indexerId);
      const indexerName = indexer?.name ?? indexerId;
      const previousSeedingProfileId = indexer?.seedingProfileId ?? null;

      setMutatingIndexerSeedingProfileIds((previous) =>
        updatePendingIndexerMappingIds(previous, indexerId, true),
      );
      setSettingsIndexers((previous) =>
        previous.map((entry) =>
          entry.id === indexerId ? { ...entry, seedingProfileId } : entry,
        ),
      );
      setGlobalStatus(t("status.indexerSeedingProfileSaving"));

      try {
        const { data, error } = await client
          .mutation(setIndexerSeedingProfileMutation, {
            input: { indexerId, seedingProfileId },
          })
          .toPromise();
        if (error) throw error;

        const resolvedSeedingProfileId =
          data?.setIndexerSeedingProfile?.seedingProfileId ?? null;
        setSettingsIndexers((previous) =>
          previous.map((entry) =>
            entry.id === indexerId
              ? { ...entry, seedingProfileId: resolvedSeedingProfileId }
              : entry,
          ),
        );
        setGlobalStatus(
          t("status.indexerSeedingProfileSaved", { name: indexerName }),
        );
      } catch (error) {
        setSettingsIndexers((previous) =>
          previous.map((entry) =>
            entry.id === indexerId
              ? { ...entry, seedingProfileId: previousSeedingProfileId }
              : entry,
          ),
        );
        // The backend rejects non-torrent indexers by name; keep that wording.
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.failedToUpdate"),
        );
      } finally {
        setMutatingIndexerSeedingProfileIds((previous) =>
          updatePendingIndexerMappingIds(previous, indexerId, false),
        );
      }
    },
    [client, setGlobalStatus, settingsIndexers, t],
  );

  const toggleIndexerEnabled = useCallback(
    async (indexer: IndexerRecord) => {
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
      proxyConfigId: indexerDraft.proxyConfigId,
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

  const resetProxyDraft = useCallback(() => {
    setEditingProxyId(null);
    setIsProxyEditorOpen(false);
    setProxyDraft({ ...PROXY_INITIAL_DRAFT });
  }, []);

  const editProxy = useCallback((proxy: ProxyRecord) => {
    setEditingProxyId(proxy.id);
    setIsProxyEditorOpen(true);
    setProxyDraft({
      providerType: isProxyProviderType(proxy.providerType)
        ? proxy.providerType
        : "byparr",
      name: proxy.name,
      baseUrl: proxy.baseUrl,
      requestTimeoutSeconds: proxy.requestTimeoutSeconds,
      // Credentials are write-only: they are never read back, so the editor
      // opens with blank inputs meaning "leave the stored secret alone".
      username: "",
      password: "",
      hasStoredCredentials: proxy.hasCredentials,
      clearCredentials: false,
      remoteDns: proxy.remoteDns,
      isEnabled: proxy.isEnabled,
    });
    setGlobalStatus(`Editing proxy ${proxy.name}`);
  }, [setGlobalStatus]);

  const startCreateProxy = useCallback(() => {
    setEditingProxyId(null);
    setProxyDraft({ ...PROXY_INITIAL_DRAFT });
    setIsProxyEditorOpen(true);
  }, []);

  const changeProxyProvider = useCallback(
    (providerType: ProxyProviderTypeValue) => {
      setProxyDraft((prev) => {
        if (prev.providerType === providerType) {
          return prev;
        }
        const previousDefault =
          PROXY_DEFAULT_BASE_URLS[prev.providerType];
        return {
          ...prev,
          providerType,
          // Only reseed a URL the operator has not customized; a typed value
          // survives so switching provider by accident costs nothing.
          baseUrl:
            prev.baseUrl.trim() === "" || prev.baseUrl === previousDefault
              ? PROXY_DEFAULT_BASE_URLS[providerType]
              : prev.baseUrl,
          // Fields the new provider rejects must not linger in the draft.
          username: "",
          password: "",
          clearCredentials: false,
          remoteDns: supportsProxyRemoteDns(providerType)
            ? prev.remoteDns
            : false,
        };
      });
    },
    [],
  );

  const submitProxy = useCallback(async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const name = proxyDraft.name.trim();
    const baseUrl = proxyDraft.baseUrl.trim();
    if (!name || !baseUrl) {
      setGlobalStatus(t("form.indexerValidation"));
      return;
    }

    setMutatingProxyId(editingProxyId || "new");
    try {
      if (editingProxyId) {
        const { error } = await client
          .mutation(updateProxyConfigMutation, {
            input: buildUpdateProxyInput(editingProxyId, proxyDraft),
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus("Proxy updated");
      } else {
        const { error } = await client
          .mutation(createProxyConfigMutation, {
            input: buildCreateProxyInput(proxyDraft),
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus("Proxy created");
      }
      resetProxyDraft();
      await Promise.all([refreshProxyConfigs(), refreshIndexers()]);
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
    proxyDraft,
    refreshProxyConfigs,
    refreshIndexers,
    resetProxyDraft,
    setGlobalStatus,
    t,
  ]);

  const testProxy = useCallback(async (proxy: ProxyRecord) => {
    setTestingProxyId(proxy.id);
    try {
      const { data, error } = await client
        .mutation(testProxyConfigMutation, { id: proxy.id })
        .toPromise();
      if (error) throw error;
      const result = data?.testProxyConfig;
      setGlobalStatus(
        result?.message ||
          (result?.ok ? "Proxy test passed" : "Proxy test failed"),
      );
      await refreshProxyConfigs();
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : "Proxy test failed",
      );
    } finally {
      setTestingProxyId(null);
    }
  }, [client, refreshProxyConfigs, setGlobalStatus]);

  const deleteProxy = useCallback((proxy: ProxyRecord) => {
    setPendingDeleteProxy(proxy);
  }, []);

  const confirmDeleteProxy = useCallback(async () => {
    if (!pendingDeleteProxy) {
      return;
    }
    const proxy = pendingDeleteProxy;
    setMutatingProxyId(proxy.id);
    try {
      const { error } = await client
        .mutation(deleteProxyConfigMutation, { id: proxy.id })
        .toPromise();
      if (error) throw error;
      setGlobalStatus("Proxy deleted");
      if (editingProxyId === proxy.id) {
        resetProxyDraft();
      }
      await Promise.all([refreshProxyConfigs(), refreshIndexers()]);
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
    refreshProxyConfigs,
    refreshIndexers,
    resetProxyDraft,
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
        indexerSettingsTab={indexerSettingsTab}
        editingIndexerId={editingIndexerId}
        indexerDraft={indexerDraft}
        setIndexerDraft={setIndexerDraft}
        submitIndexer={submitIndexer}
        mutatingIndexerId={mutatingIndexerId}
        resetIndexerDraft={requestCloseEditor}
        settingsIndexerFilter={settingsIndexerFilter}
        setSettingsIndexerFilter={setSettingsIndexerFilter}
        settingsIndexers={settingsIndexers}
        indexerDownloadClientMappingCatalogResource={
          indexerDownloadClientMappingCatalogResource
        }
        refreshIndexerDownloadClientMappingCatalog={
          refreshIndexerDownloadClientMappingCatalog
        }
        mutatingIndexerMappingIds={mutatingIndexerMappingIds}
        setIndexerDownloadClientMapping={setIndexerDownloadClientMapping}
        seedingProfileOptions={seedingProfileOptions}
        mutatingIndexerSeedingProfileIds={mutatingIndexerSeedingProfileIds}
        setIndexerSeedingProfile={setIndexerSeedingProfile}
        proxyConfigs={proxyConfigs}
        proxyDraft={proxyDraft}
        setProxyDraft={setProxyDraft}
        editingProxyId={editingProxyId}
        isProxyEditorOpen={isProxyEditorOpen}
        mutatingProxyId={mutatingProxyId}
        testingProxyId={testingProxyId}
        submitProxy={submitProxy}
        resetProxyDraft={resetProxyDraft}
        startCreateProxy={startCreateProxy}
        changeProxyProvider={changeProxyProvider}
        editProxy={editProxy}
        testProxy={testProxy}
        deleteProxy={deleteProxy}
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
            ? `Delete proxy ${pendingDeleteProxy.name}?`
            : ""
        }
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        confirmButtonId="settings-indexer-proxy-delete-confirm"
        cancelButtonId="settings-indexer-proxy-delete-cancel"
        isBusy={mutatingProxyId !== null}
        onConfirm={confirmDeleteProxy}
        onCancel={() => setPendingDeleteProxy(null)}
      />
    </>
  );
}

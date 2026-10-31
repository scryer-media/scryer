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
  LoginChallengeStatus,
  ProxyRecord,
  IndexerRecord,
  ProviderTypeInfo,
  IndexerDownloadClientMappingCatalog,
  IndexerDownloadClientMappingCatalogResource,
  IndexerCategoryRoutingSettings,
  IndexerRoutingEntry,
  IndexerRoutingSettingsByIndexer,
  IndexerRoutingSettingsByScope,
} from "@/lib/types";
import type { ViewCategoryId } from "@/lib/types/quality-profiles";
import {
  isConfigFieldRequired,
  isConfigFieldVisible,
} from "@/lib/utils/provider-config-fields";
import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import { runConnectionFeedback } from "@/lib/utils/connection-feedback";
import { normalizeIndexerConfigValues } from "@/lib/utils/url-input";
import {
  getIndexerDownloadClientDraftMappingViewModel,
  updateIndexerDownloadClientMapping,
  updatePendingIndexerMappingIds,
} from "@/lib/utils/indexer-download-client-mapping";
import {
  indexerProviderTypesQuery,
  pluginConfigFieldsQuery,
  pluginLoginChallengeQuery,
  proxyConfigsQuery,
  indexersInitQuery,
  indexersQuery,
  indexerRoutingAllScopesQuery,
} from "@/lib/graphql/queries";
import {
  createIndexerMutation,
  deleteIndexerMutation,
  syncIndexerConfigMutation,
  setIndexerDownloadClientMappingMutation,
  setIndexerSeedingProfileMutation,
  testIndexerConnectionMutation,
  updateIndexerMutation,
  updateIndexerRoutingMutation,
} from "@/lib/graphql/mutations";
import {
  providerConfigRecordToValues,
  providerConfigValuesToRecord,
} from "@/lib/utils/provider-config";
import { useSeedingProfileOptions } from "@/lib/hooks/use-seeding-profile-options";
import {
  createRequestEpoch,
  dynamicConfigFieldsFor,
  dynamicTriggerSignature,
  dynamicTriggerValues,
  mergeDynamicConfigFields,
  omitHiddenConfigValues,
  reconcileDynamicConfigValues,
  type DynamicConfigFields,
  type DynamicConfigFieldsStatus,
  type ProviderDynamicConfigFields,
} from "@/lib/utils/dynamic-config-fields";
import { loginChallengeErrorMessage } from "@/lib/utils/login-challenge-error-message";
import { getDefaultIndexerRouting } from "@/lib/constants/indexers";
import {
  buildIndexerSavePayload,
  indexerQueryBudgetDraftValue,
  indexerRateLimitSecondsDraftValue,
  parseIndexerQueryBudget,
  parseIndexerRateLimitSeconds,
} from "@/lib/utils/indexer-payload";

type SettingsIndexersSectionProps = ComponentProps<
  typeof SettingsIndexersSection
>;

/// How long the form waits after the last trigger-field change before asking the
/// plugin for its fields, so typing does not fire one request per keystroke.
const DYNAMIC_CONFIG_FIELDS_DEBOUNCE_MS = 300;

const INDEXER_INITIAL_DRAFT = {
  name: "",
  providerType: "",
  proxyConfigId: null as string | null,
  downloadClientId: null as string | null,
  seedingProfileId: null as string | null,
  storedSecretKeys: [] as string[],
  rateLimitSeconds: "",
  maxQueriesPerMinute: "",
  isEnabled: true,
  enableInteractiveSearch: true,
  enableAutoSearch: true,
  configValues: {} as Record<string, string>,
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

    // Hidden fields keep their draft value so toggling the field they depend on
    // does not lose typing, but they are not part of what gets saved.
    if (!isConfigFieldVisible(field, configValues)) {
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
    // Through the shared evaluator: a hidden field is never missing, and
    // `required_when` can raise one that declared `required: false`.
    if (
      field.valueSource === "HOST_BINDING" ||
      !isConfigFieldRequired(field, configValues)
    ) {
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

const EMPTY_INDEXER_DOWNLOAD_CLIENT_MAPPING_CATALOG: IndexerDownloadClientMappingCatalog =
  {
    clients: [],
    indexers: [],
    providerCompatibility: [],
  };

type PendingIndexerEditorAction =
  | { type: "create" }
  | { type: "edit"; indexer: IndexerRecord }
  | { type: "close" }
  | null;

function emptyIndexerRoutingByScope(): IndexerRoutingSettingsByScope {
  return { MOVIE: {}, SERIES: {}, ANIME: {} };
}

function indexerRoutingEntriesToMap(
  entries: IndexerRoutingEntry[] | null | undefined,
): IndexerRoutingSettingsByIndexer {
  return Object.fromEntries(
    (entries ?? []).map((entry) => [
      entry.indexerId,
      {
        categories: entry.categories,
        enabled: entry.enabled,
        priority: entry.priority,
      },
    ]),
  ) as IndexerRoutingSettingsByIndexer;
}

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
  const [mutatingIndexerMappingIds, setMutatingIndexerMappingIds] = useState<
    Set<string>
  >(() => new Set());
  const { options: seedingProfileOptions } = useSeedingProfileOptions();
  const [
    mutatingIndexerSeedingProfileIds,
    setMutatingIndexerSeedingProfileIds,
  ] = useState<Set<string>>(() => new Set());
  const [mutatingIndexerProxyIds, setMutatingIndexerProxyIds] = useState<
    Set<string>
  >(() => new Set());
  const [proxyConfigs, setProxyConfigs] = useState<ProxyRecord[]>([]);
  const [indexerRoutingByScope, setIndexerRoutingByScope] =
    useState<IndexerRoutingSettingsByScope>(emptyIndexerRoutingByScope);
  const [indexerRoutingLoaded, setIndexerRoutingLoaded] = useState(false);
  const [indexerRoutingLoading, setIndexerRoutingLoading] = useState(false);
  const [mutatingIndexerRoutingScopes, setMutatingIndexerRoutingScopes] =
    useState<Set<ViewCategoryId>>(() => new Set());
  const indexerRoutingLoadPromiseRef = useRef<Promise<void> | null>(null);
  const [settingsIndexerFilter, setSettingsIndexerFilter] = useState("");
  const [mutatingIndexerId, setMutatingIndexerId] = useState<string | null>(
    null,
  );
  const [editingIndexerId, setEditingIndexerId] = useState<string | null>(null);
  const [pendingDeleteIndexer, setPendingDeleteIndexer] =
    useState<IndexerRecord | null>(null);
  const [isTestingConnection, setIsTestingConnection] = useState(false);
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
    const newznabProvider =
      providerTypes.find(
        (providerType) =>
          providerType.providerType.trim().toLowerCase() === "newznab",
      ) ?? null;
    setEditingIndexerId(null);
    setIndexerDraft(() =>
      cloneIndexerDraft({
        ...INDEXER_INITIAL_DRAFT,
        providerType: newznabProvider?.providerType ?? "newznab",
        name: newznabProvider?.name ?? "Newznab Indexer",
        configValues: newznabProvider
          ? buildDraftConfigValues(newznabProvider.configFields, {})
          : {},
      }),
    );
  }, [providerTypes]);

  useEffect(() => {
    if (!awaitingBaselineSync) {
      return;
    }

    setDraftBaseline(cloneIndexerDraft(indexerDraft));
    setAwaitingBaselineSync(false);
  }, [awaitingBaselineSync, indexerDraft]);

  const isDraftDirty =
    JSON.stringify(indexerDraft) !== JSON.stringify(draftBaseline);

  // Fields the selected plugin adds for the values entered so far. Asked for on
  // open and whenever a trigger value changes; the request epoch makes sure a
  // slow answer for an earlier selection never lands over a later one.
  // Held with the provider they were answered for, so switching the provider
  // type never shows or reconciles against the previous provider's fields.
  const [heldDynamicConfigFields, setHeldDynamicConfigFields] =
    useState<ProviderDynamicConfigFields | null>(null);
  const [dynamicConfigFieldsStatus, setDynamicConfigFieldsStatus] =
    useState<DynamicConfigFieldsStatus>({ state: "idle" });
  const heldDynamicConfigFieldsRef = useRef<ProviderDynamicConfigFields | null>(
    null,
  );
  const dynamicConfigFieldsEpochRef = useRef(createRequestEpoch());
  const normalizedDraftProviderType = indexerDraft.providerType
    .trim()
    .toLowerCase();
  const dynamicConfigFields = dynamicConfigFieldsFor(
    heldDynamicConfigFields,
    normalizedDraftProviderType,
  );
  const staticSelectedProvider =
    providerTypes.find(
      (pt) => pt.providerType === normalizedDraftProviderType,
    ) ?? null;
  const dynamicTriggerKeys = staticSelectedProvider?.dynamicConfigTriggerKeys;
  const dynamicTriggers =
    isEditorOpen && dynamicTriggerKeys && dynamicTriggerKeys.length > 0
      ? dynamicTriggerValues(
          dynamicTriggerKeys,
          staticSelectedProvider?.configFields ?? [],
          indexerDraft.configValues,
        )
      : null;
  const dynamicTriggersSignature = dynamicTriggers
    ? dynamicTriggerSignature(dynamicTriggers)
    : null;

  const applyDynamicConfigFields = useCallback(
    (providerType: string, next: DynamicConfigFields | null) => {
      const previous = dynamicConfigFieldsFor(
        heldDynamicConfigFieldsRef.current,
        providerType,
      );
      const held = next ? { providerType, fields: next } : null;
      heldDynamicConfigFieldsRef.current = held;
      setHeldDynamicConfigFields(held);
      // The baseline moves with the draft, so fields arriving for a stored
      // selection do not count as an unsaved edit.
      setIndexerDraft((prev) => {
        const configValues = reconcileDynamicConfigValues(
          prev.configValues,
          previous,
          next,
        );
        return configValues === prev.configValues
          ? prev
          : { ...prev, configValues };
      });
      setDraftBaseline((prev) => {
        const configValues = reconcileDynamicConfigValues(
          prev.configValues,
          previous,
          next,
        );
        return configValues === prev.configValues
          ? prev
          : { ...prev, configValues };
      });
    },
    [],
  );

  useEffect(() => {
    const epoch = dynamicConfigFieldsEpochRef.current.next();
    if (
      heldDynamicConfigFieldsRef.current !== null &&
      heldDynamicConfigFieldsRef.current.providerType !==
        normalizedDraftProviderType
    ) {
      heldDynamicConfigFieldsRef.current = null;
      setHeldDynamicConfigFields(null);
    }
    if (dynamicTriggersSignature === null || !normalizedDraftProviderType) {
      heldDynamicConfigFieldsRef.current = null;
      setHeldDynamicConfigFields(null);
      setDynamicConfigFieldsStatus({ state: "idle" });
      return;
    }
    const triggers = Object.fromEntries(
      JSON.parse(dynamicTriggersSignature) as Array<[string, string]>,
    );
    setDynamicConfigFieldsStatus({ state: "loading" });
    const timer = setTimeout(() => {
      void (async () => {
        try {
          const { data, error } = await client
            .query(
              pluginConfigFieldsQuery,
              {
                providerType: normalizedDraftProviderType,
                values: providerConfigRecordToValues(triggers),
              },
              { requestPolicy: "network-only" },
            )
            .toPromise();
          if (!dynamicConfigFieldsEpochRef.current.isCurrent(epoch)) {
            return;
          }
          if (error) throw error;
          const answer = data?.pluginConfigFields as
            DynamicConfigFields | undefined;
          applyDynamicConfigFields(
            normalizedDraftProviderType,
            answer &&
              (answer.fields.length > 0 || answer.hiddenFieldKeys.length > 0)
              ? answer
              : null,
          );
          setDynamicConfigFieldsStatus({ state: "idle" });
        } catch (error) {
          if (!dynamicConfigFieldsEpochRef.current.isCurrent(epoch)) {
            return;
          }
          // Keep the draft values: the save path still validates them, and
          // the next answer reconciles them.
          setDynamicConfigFieldsStatus({
            state: "error",
            message: userFacingGraphQlErrorMessage(
              error,
              t("status.failedToLoad"),
              t,
            ),
          });
        }
      })();
    }, DYNAMIC_CONFIG_FIELDS_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [
    applyDynamicConfigFields,
    client,
    dynamicTriggersSignature,
    normalizedDraftProviderType,
    t,
  ]);

  const effectiveProviderTypes = useMemo(
    () =>
      dynamicConfigFields && staticSelectedProvider
        ? providerTypes.map((pt) =>
            pt === staticSelectedProvider
              ? {
                  ...pt,
                  configFields: mergeDynamicConfigFields(
                    pt.configFields,
                    dynamicConfigFields,
                  ),
                }
              : pt,
          )
        : providerTypes,
    [dynamicConfigFields, providerTypes, staticSelectedProvider],
  );

  // The sign-in image a provider's login asks the operator to read. It is
  // fetched for the draft (the server restores stored secrets for an indexer
  // being edited) when the editor opens, when any value but the answer
  // (address, credentials, selection, proxy) or the indexer changes, after a
  // failed save, and on request. It is never
  // stored; the server carries the sign-in it started to the save.
  const [loginChallenge, setLoginChallenge] = useState<LoginChallengeStatus>({
    state: "idle",
  });
  const [loginChallengeNonce, setLoginChallengeNonce] = useState(0);
  const loginChallengeEpochRef = useRef(createRequestEpoch());
  const indexerDraftRef = useRef(indexerDraft);
  const effectiveSelectedProvider = isEditorOpen
    ? (effectiveProviderTypes.find(
        (pt) => pt.providerType === normalizedDraftProviderType,
      ) ?? null)
    : null;
  const effectiveSelectedProviderRef = useRef(effectiveSelectedProvider);
  // The fetch reads the draft as it stands when it fires, without re-running
  // for every keystroke in a field the image does not depend on.
  useEffect(() => {
    indexerDraftRef.current = indexerDraft;
    effectiveSelectedProviderRef.current = effectiveSelectedProvider;
  });
  const loginChallengeAnswerKey =
    effectiveSelectedProvider?.configFields.find(
      (field) => field.role === "LOGIN_CHALLENGE_ANSWER",
    )?.key ?? null;
  // The image belongs to the configuration it was fetched for: the server
  // keys the sign-in it carries to the save by every value but the answer,
  // so the fetch follows every non-answer value, credentials included.
  const loginChallengeConfigSignature =
    loginChallengeAnswerKey === null
      ? null
      : JSON.stringify(
          Object.entries(indexerDraft.configValues)
            .filter(([key]) => key !== loginChallengeAnswerKey)
            .map(([key, value]) => [key, value.trim()])
            .sort(([left], [right]) => left.localeCompare(right)),
        );
  const loginChallengeSignature =
    loginChallengeAnswerKey === null
      ? null
      : JSON.stringify([
          normalizedDraftProviderType,
          editingIndexerId,
          indexerDraft.proxyConfigId,
          loginChallengeAnswerKey,
          dynamicTriggersSignature,
          loginChallengeConfigSignature,
          loginChallengeNonce,
        ]);

  useEffect(() => {
    const epoch = loginChallengeEpochRef.current.next();
    if (loginChallengeSignature === null) {
      setLoginChallenge({ state: "idle" });
      return;
    }
    setLoginChallenge({ state: "loading" });
    const timer = setTimeout(() => {
      void (async () => {
        try {
          const draft = indexerDraftRef.current;
          const provider = effectiveSelectedProviderRef.current;
          const fields = provider?.configFields ?? [];
          const answerKeys = new Set(
            fields
              .filter((field) => field.role === "LOGIN_CHALLENGE_ANSWER")
              .map((field) => field.key),
          );
          const configValues = Object.fromEntries(
            Object.entries(
              omitHiddenConfigValues(
                normalizeIndexerConfigValues(fields, draft.configValues),
                dynamicConfigFieldsFor(
                  heldDynamicConfigFieldsRef.current,
                  normalizedDraftProviderType,
                ),
              ),
            ).filter(([key]) => !answerKeys.has(key)),
          );
          const values =
            serializeConfigValues(
              fields.filter((field) => !answerKeys.has(field.key)),
              configValues,
              draft.storedSecretKeys,
            ) ?? [];
          const { data, error } = await client
            .query(
              pluginLoginChallengeQuery,
              {
                providerType: normalizedDraftProviderType,
                values,
                indexerId: editingIndexerId ?? undefined,
                proxyConfigId: draft.proxyConfigId,
              },
              { requestPolicy: "network-only" },
            )
            .toPromise();
          if (!loginChallengeEpochRef.current.isCurrent(epoch)) {
            return;
          }
          if (error) throw error;
          const challenge = data?.pluginConfigFields?.loginChallenge as
            { contentType: string; imageData: string } | null | undefined;
          setLoginChallenge(
            challenge
              ? {
                  state: "ready",
                  contentType: challenge.contentType,
                  imageData: challenge.imageData,
                }
              : { state: "none" },
          );
        } catch (error) {
          if (!loginChallengeEpochRef.current.isCurrent(epoch)) {
            return;
          }
          setLoginChallenge({
            state: "error",
            message: loginChallengeErrorMessage(
              error,
              t,
              t("status.failedToLoad"),
            ),
          });
        }
      })();
    }, DYNAMIC_CONFIG_FIELDS_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [
    client,
    editingIndexerId,
    loginChallengeSignature,
    normalizedDraftProviderType,
    t,
  ]);

  const refreshLoginChallenge = useCallback(() => {
    setLoginChallengeNonce((nonce) => nonce + 1);
  }, []);

  const refreshIndexers = useCallback(async () => {
    try {
      const { data, error } = await client
        .query(
          indexersQuery,
          {
            providerType: settingsIndexerFilter || undefined,
          },
          {
            requestPolicy: "network-only",
          },
        )
        .toPromise();
      if (error) throw error;
      setSettingsIndexers(data.indexers || []);
    } catch (error) {
      setGlobalStatus(
        userFacingGraphQlErrorMessage(error, t("status.failedToLoad"), t),
        { level: "ERROR" },
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
        userFacingGraphQlErrorMessage(error, t("status.failedToLoad"), t),
        { level: "ERROR" },
      );
    }
  }, [client, setGlobalStatus, t]);

  const loadIndexerRouting = useCallback(async () => {
    if (indexerRoutingLoaded) {
      return;
    }
    if (indexerRoutingLoadPromiseRef.current) {
      return indexerRoutingLoadPromiseRef.current;
    }

    setIndexerRoutingLoading(true);
    const request = (async () => {
      try {
        const { data, error } = await client
          .query(
            indexerRoutingAllScopesQuery,
            {},
            { requestPolicy: "network-only" },
          )
          .toPromise();
        if (error) throw error;

        setIndexerRoutingByScope({
          MOVIE: indexerRoutingEntriesToMap(data?.movie),
          SERIES: indexerRoutingEntriesToMap(data?.series),
          ANIME: indexerRoutingEntriesToMap(data?.anime),
        });
        setIndexerRoutingLoaded(true);
      } catch (error) {
        setGlobalStatus(
          userFacingGraphQlErrorMessage(error, t("status.failedToLoad"), t),
          { level: "ERROR" },
        );
      } finally {
        setIndexerRoutingLoading(false);
        indexerRoutingLoadPromiseRef.current = null;
      }
    })();
    indexerRoutingLoadPromiseRef.current = request;
    return request;
  }, [client, indexerRoutingLoaded, setGlobalStatus, t]);

  const updateIndexerRoutingForScope = useCallback(
    async (
      scope: ViewCategoryId,
      indexerId: string,
      nextValue: Partial<IndexerCategoryRoutingSettings>,
    ) => {
      const previousScopeRouting = indexerRoutingByScope[scope] ?? {};
      const currentRouting =
        previousScopeRouting[indexerId] ?? getDefaultIndexerRouting(scope);
      const nextRouting = { ...currentRouting, ...nextValue };
      const nextScopeRouting = {
        ...previousScopeRouting,
        [indexerId]: nextRouting,
      };

      setIndexerRoutingByScope((previous) => ({
        ...previous,
        [scope]: nextScopeRouting,
      }));
      setMutatingIndexerRoutingScopes((previous) => {
        const next = new Set(previous);
        next.add(scope);
        return next;
      });

      try {
        // The mutation replaces the scope's whole routing map, so it needs
        // every live indexer, not just the rows a provider filter left on
        // screen: a hidden indexer left out would lose its routing. Only live
        // indexers, though; a deleted one must not be resubmitted.
        let liveIndexers = settingsIndexers;
        if (settingsIndexerFilter) {
          const { data: unfiltered, error: listError } = await client
            .query(indexersQuery, {}, { requestPolicy: "network-only" })
            .toPromise();
          if (listError) throw listError;
          liveIndexers = unfiltered?.indexers ?? [];
        }
        const indexerIds = new Set(liveIndexers.map((indexer) => indexer.id));
        const { data, error } = await client
          .mutation(updateIndexerRoutingMutation, {
            input: {
              scope,
              entries: Array.from(indexerIds, (id) => {
                const routing =
                  id === indexerId
                    ? nextRouting
                    : (nextScopeRouting[id] ?? getDefaultIndexerRouting(scope));
                return {
                  indexerId: id,
                  enabled: routing.enabled,
                  categories: routing.categories,
                  // Inline changes intentionally retain the server's priority.
                  priority: routing.priority,
                };
              }),
            },
          })
          .toPromise();
        if (error) throw error;

        setIndexerRoutingByScope((previous) => ({
          ...previous,
          [scope]: indexerRoutingEntriesToMap(data?.updateIndexerRouting),
        }));
        setGlobalStatus(t("settings.qualitySettingsSaved"), {
          level: "SUCCESS",
        });
      } catch (error) {
        setIndexerRoutingByScope((previous) => ({
          ...previous,
          [scope]: previousScopeRouting,
        }));
        setGlobalStatus(
          userFacingGraphQlErrorMessage(error, t("status.failedToUpdate"), t),
          { level: "ERROR" },
        );
      } finally {
        setMutatingIndexerRoutingScopes((previous) => {
          const next = new Set(previous);
          next.delete(scope);
          return next;
        });
      }
    },
    [
      client,
      indexerRoutingByScope,
      setGlobalStatus,
      settingsIndexerFilter,
      settingsIndexers,
      t,
    ],
  );

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
          userFacingGraphQlErrorMessage(error, t("status.failedToLoad"), t),
          { level: "ERROR" },
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
        userFacingGraphQlErrorMessage(error, t("status.failedToLoad"), t),
        { level: "ERROR" },
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
      const nextProvider =
        configuredProvider ??
        providerTypes.find(
          (providerType) =>
            providerType.providerType.trim().toLowerCase() === "newznab",
        ) ??
        providerTypes[0] ??
        null;
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

      const shouldInitializeConfig =
        nextProviderType === prev.providerType &&
        Object.keys(prev.configValues).length === 0 &&
        nextProvider.configFields.length > 0;

      if (
        nextProviderType === prev.providerType &&
        nextName === prev.name &&
        !shouldInitializeConfig
      ) {
        return prev;
      }

      return {
        ...prev,
        providerType: nextProviderType,
        name: nextName,
        configValues:
          nextProviderType === prev.providerType && !shouldInitializeConfig
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
    const normalizedProviderType = indexerDraft.providerType
      .trim()
      .toLowerCase();
    const selectedProvider =
      effectiveProviderTypes.find(
        (pt) => pt.providerType === normalizedProviderType,
      ) ?? null;
    // Parse on save: an operator pastes a tracker's address out of an email or
    // their browser, so `nzbgeek.info` and `192.168.1.5:9117` mean what they
    // plainly mean. The form is put back with the scheme that was chosen, so
    // nothing is decided behind their back.
    const draftConfigValues = normalizeIndexerConfigValues(
      selectedProvider?.configFields ?? [],
      indexerDraft.configValues,
    );
    if (draftConfigValues !== indexerDraft.configValues) {
      setIndexerDraft((prev) => ({ ...prev, configValues: draftConfigValues }));
    }
    const configValues = omitHiddenConfigValues(
      draftConfigValues,
      dynamicConfigFields,
    );
    const missingRequiredConfigField = findMissingRequiredConfigField(
      selectedProvider?.configFields ?? [],
      configValues,
      indexerDraft.storedSecretKeys,
    );
    const payload = buildIndexerSavePayload(
      indexerDraft,
      normalizedProviderType,
      serializeConfigValues(
        selectedProvider?.configFields ?? [],
        configValues,
        indexerDraft.storedSecretKeys,
      ),
    );

    if (!payload.name || !payload.providerType) {
      setGlobalStatus(t("form.indexerValidation"));
      return;
    }

    if (!parseIndexerRateLimitSeconds(indexerDraft.rateLimitSeconds).valid) {
      setGlobalStatus(t("form.indexerRateLimitSecondsInvalid"));
      return;
    }

    if (!parseIndexerQueryBudget(indexerDraft.maxQueriesPerMinute).valid) {
      setGlobalStatus(t("form.indexerMaxQueriesPerMinuteInvalid"));
      return;
    }

    if (missingRequiredConfigField) {
      setGlobalStatus(
        `${missingRequiredConfigField.label}: ${t("setup.required")}`,
      );
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
        const existingDownloadClientId =
          existingIndexer?.downloadClientId ?? null;
        const { error } = await client
          .mutation(updateIndexerMutation, {
            input: {
              id: editingIndexerId,
              name: payload.name,
              providerType: payload.providerType,
              proxyConfigId: payload.proxyConfigId,
              rateLimitSeconds: payload.rateLimitSeconds,
              maxQueriesPerMinute: payload.maxQueriesPerMinute,
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
          payload.seedingProfileId !==
          (existingIndexer?.seedingProfileId ?? null)
        ) {
          await applyIndexerSeedingProfile(
            editingIndexerId,
            payload.seedingProfileId,
          );
        }
        setGlobalStatus(t("status.indexerUpdated"), { level: "SUCCESS" });
      } else {
        const { data, error } = await client
          .mutation(createIndexerMutation, {
            input: {
              name: payload.name,
              providerType: payload.providerType,
              proxyConfigId: payload.proxyConfigId,
              rateLimitSeconds: payload.rateLimitSeconds,
              maxQueriesPerMinute: payload.maxQueriesPerMinute,
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
          string | undefined;
        if (payload.seedingProfileId && createdIndexerId) {
          await applyIndexerSeedingProfile(
            createdIndexerId,
            payload.seedingProfileId,
          );
        }
        setGlobalStatus(t("status.indexerCreated"), { level: "SUCCESS" });
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
        userFacingGraphQlErrorMessage(error, t("status.failedToUpdate"), t),
        { level: "ERROR" },
      );
      // A sign-in image answers once: a refused save needs a fresh one.
      if (
        loginChallenge.state === "ready" ||
        loginChallenge.state === "error"
      ) {
        refreshLoginChallenge();
      }
    } finally {
      setMutatingIndexerId(null);
    }
  };

  const editIndexer = useCallback(
    (indexer: IndexerRecord) => {
      if (indexer.isManaged) {
        setGlobalStatus(t("settings.managedIndexerReadOnly"));
        return;
      }
      const selectedProvider =
        providerTypes.find(
          (providerType) =>
            providerType.providerType ===
            indexer.providerType.trim().toLowerCase(),
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
        rateLimitSeconds: indexerRateLimitSecondsDraftValue(indexer),
        maxQueriesPerMinute: indexerQueryBudgetDraftValue(indexer),
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
    },
    [providerTypes, setGlobalStatus, t],
  );

  const openEditEditor = useCallback(
    (indexer: IndexerRecord) => {
      editIndexer(indexer);
      setEditorMode("edit");
      setIsEditorOpen(true);
      setAwaitingBaselineSync(true);
    },
    [editIndexer],
  );

  const requestCreateEditor = useCallback(() => {
    if (!isEditorOpen || !isDraftDirty) {
      openCreateEditor();
      return;
    }

    setPendingEditorAction({ type: "create" });
  }, [isDraftDirty, isEditorOpen, openCreateEditor]);

  const requestEditIndexer = useCallback(
    (indexer: IndexerRecord) => {
      if (!isEditorOpen || !isDraftDirty) {
        openEditEditor(indexer);
        return;
      }

      setPendingEditorAction({ type: "edit", indexer });
    },
    [isDraftDirty, isEditorOpen, openEditEditor],
  );

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
  }, [
    openCreateEditor,
    openEditEditor,
    pendingEditorAction,
    resetIndexerDraft,
  ]);

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
      const previousDownloadClientId =
        previousMapping?.downloadClientId ?? null;
      const selectedClient = downloadClientId
        ? indexerDownloadClientMappingCatalog.clients.find(
            (clientRecord) => clientRecord.id === downloadClientId,
          )
        : null;

      setMutatingIndexerMappingIds((previous) =>
        updatePendingIndexerMappingIds(previous, indexerId, true),
      );
      updateIndexerDownloadClientMappingCatalog((previous) =>
        updateIndexerDownloadClientMapping(
          previous,
          indexerId,
          downloadClientId,
        ),
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
          ? (response.downloadClientId ?? null)
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
          { level: "SUCCESS" },
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
          userFacingGraphQlErrorMessage(error, t("status.failedToUpdate"), t),
          { level: "ERROR" },
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
          { level: "SUCCESS" },
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
          userFacingGraphQlErrorMessage(error, t("status.failedToUpdate"), t),
          { level: "ERROR" },
        );
      } finally {
        setMutatingIndexerSeedingProfileIds((previous) =>
          updatePendingIndexerMappingIds(previous, indexerId, false),
        );
      }
    },
    [client, setGlobalStatus, settingsIndexers, t],
  );

  const setIndexerProxyAssignment = useCallback(
    async (indexerId: string, proxyConfigId: string | null) => {
      const indexer = settingsIndexers.find((entry) => entry.id === indexerId);
      const previousProxyConfigId = indexer?.proxyConfigId ?? null;

      setMutatingIndexerProxyIds((previous) =>
        updatePendingIndexerMappingIds(previous, indexerId, true),
      );
      setSettingsIndexers((previous) =>
        previous.map((entry) =>
          entry.id === indexerId ? { ...entry, proxyConfigId } : entry,
        ),
      );

      try {
        const { error } = await client
          .mutation(updateIndexerMutation, {
            input: { id: indexerId, proxyConfigId },
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.indexerUpdated"), { level: "SUCCESS" });
        await refreshIndexers();
      } catch (error) {
        setSettingsIndexers((previous) =>
          previous.map((entry) =>
            entry.id === indexerId
              ? { ...entry, proxyConfigId: previousProxyConfigId }
              : entry,
          ),
        );
        setGlobalStatus(
          userFacingGraphQlErrorMessage(error, t("status.failedToUpdate"), t),
          { level: "ERROR" },
        );
      } finally {
        setMutatingIndexerProxyIds((previous) =>
          updatePendingIndexerMappingIds(previous, indexerId, false),
        );
      }
    },
    [client, refreshIndexers, setGlobalStatus, settingsIndexers, t],
  );

  const updateIndexerToggles = useCallback(
    async (
      indexer: IndexerRecord,
      nextValue: Partial<
        Pick<
          IndexerRecord,
          "isEnabled" | "enableInteractiveSearch" | "enableAutoSearch"
        >
      >,
    ) => {
      setMutatingIndexerId(indexer.id);
      try {
        const { error } = await client
          .mutation(updateIndexerMutation, {
            input: {
              id: indexer.id,
              ...nextValue,
            },
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.indexerUpdated"), { level: "SUCCESS" });
        await refreshIndexers();
      } catch (error) {
        setGlobalStatus(
          userFacingGraphQlErrorMessage(error, t("status.failedToUpdate"), t),
          { level: "ERROR" },
        );
      } finally {
        setMutatingIndexerId(null);
      }
    },
    [client, refreshIndexers, setGlobalStatus, t],
  );

  // Managed children take their rate limits from the row rather than the
  // editor, which the parent sync owns. Returns whether the save landed so
  // the row control knows when to close.
  const updateIndexerRateLimits = useCallback(
    async (
      indexer: IndexerRecord,
      draft: { rateLimitSeconds: string; maxQueriesPerMinute: string },
    ): Promise<boolean> => {
      const interval = parseIndexerRateLimitSeconds(draft.rateLimitSeconds);
      if (!interval.valid) {
        setGlobalStatus(t("form.indexerRateLimitSecondsInvalid"));
        return false;
      }
      const budget = parseIndexerQueryBudget(draft.maxQueriesPerMinute);
      if (!budget.valid) {
        setGlobalStatus(t("form.indexerMaxQueriesPerMinuteInvalid"));
        return false;
      }
      setMutatingIndexerId(indexer.id);
      try {
        const { error } = await client
          .mutation(updateIndexerMutation, {
            input: {
              id: indexer.id,
              rateLimitSeconds: interval.value,
              maxQueriesPerMinute: budget.value,
            },
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.indexerUpdated"), { level: "SUCCESS" });
        await refreshIndexers();
        return true;
      } catch (error) {
        setGlobalStatus(
          userFacingGraphQlErrorMessage(error, t("status.failedToUpdate"), t),
          { level: "ERROR" },
        );
        return false;
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
          userFacingGraphQlErrorMessage(error, t("status.failedToUpdate"), t),
          { level: "ERROR" },
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
      setGlobalStatus(t("status.indexerDeleted", { name: indexer.name }), {
        level: "SUCCESS",
      });
      await refreshIndexers();
      if (editingIndexerId === indexer.id) {
        resetIndexerDraft();
        setIsEditorOpen(false);
        setEditorMode("create");
        setAwaitingBaselineSync(true);
      }
    } catch (error) {
      setGlobalStatus(
        userFacingGraphQlErrorMessage(error, t("status.failedToDelete"), t),
        { level: "ERROR" },
      );
    } finally {
      setMutatingIndexerId(null);
      setPendingDeleteIndexer(null);
    }
  };

  const testIndexerConnection = async () => {
    const normalizedProviderType = indexerDraft.providerType
      .trim()
      .toLowerCase();
    const selectedProvider =
      effectiveProviderTypes.find(
        (pt) => pt.providerType === normalizedProviderType,
      ) ?? null;
    // Parse on save: an operator pastes a tracker's address out of an email or
    // their browser, so `nzbgeek.info` and `192.168.1.5:9117` mean what they
    // plainly mean. The form is put back with the scheme that was chosen, so
    // nothing is decided behind their back.
    const draftConfigValues = normalizeIndexerConfigValues(
      selectedProvider?.configFields ?? [],
      indexerDraft.configValues,
    );
    if (draftConfigValues !== indexerDraft.configValues) {
      setIndexerDraft((prev) => ({ ...prev, configValues: draftConfigValues }));
    }
    const configValues = omitHiddenConfigValues(
      draftConfigValues,
      dynamicConfigFields,
    );
    const missingRequiredConfigField = findMissingRequiredConfigField(
      selectedProvider?.configFields ?? [],
      configValues,
      indexerDraft.storedSecretKeys,
    );
    const payload = {
      providerType: normalizedProviderType,
      proxyConfigId: indexerDraft.proxyConfigId,
      config: serializeConfigValues(
        selectedProvider?.configFields ?? [],
        configValues,
        indexerDraft.storedSecretKeys,
      ),
      indexerId: editingIndexerId ?? undefined,
    };

    if (!payload.providerType) {
      setGlobalStatus(t("form.indexerValidation"));
      return;
    }
    if (missingRequiredConfigField) {
      setGlobalStatus(
        `${missingRequiredConfigField.label}: ${t("setup.required")}`,
      );
      return;
    }
    setIsTestingConnection(true);
    try {
      await runConnectionFeedback({
        setGlobalStatus,
        t,
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
        mutatingIndexerProxyIds={mutatingIndexerProxyIds}
        setIndexerProxyAssignment={setIndexerProxyAssignment}
        proxyConfigs={proxyConfigs}
        indexerRoutingByScope={indexerRoutingByScope}
        indexerRoutingLoaded={indexerRoutingLoaded}
        indexerRoutingLoading={indexerRoutingLoading}
        mutatingIndexerRoutingScopes={mutatingIndexerRoutingScopes}
        loadIndexerRouting={loadIndexerRouting}
        updateIndexerRoutingForScope={updateIndexerRoutingForScope}
        editIndexer={requestEditIndexer}
        updateIndexerToggles={updateIndexerToggles}
        updateIndexerRateLimits={updateIndexerRateLimits}
        deleteIndexer={deleteIndexer}
        syncIndexer={syncIndexer}
        providerTypes={effectiveProviderTypes}
        dynamicConfigFieldsStatus={dynamicConfigFieldsStatus}
        loginChallenge={loginChallenge}
        refreshLoginChallenge={refreshLoginChallenge}
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
    </>
  );
}

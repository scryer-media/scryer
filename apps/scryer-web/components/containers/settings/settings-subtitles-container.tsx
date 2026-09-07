import * as React from "react";
import { createPortal } from "react-dom";
import { Loader2, Subtitles } from "lucide-react";
import { useClient } from "urql";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { SettingsToggleSwitch } from "@/components/common/settings-toggle-switch";
import { SETTINGS_REFERENCE_SLOT_ID } from "@/components/containers/settings/settings-container";
import { FilteredPluginList } from "@/components/views/settings/filtered-plugin-list";
import { SettingsSubtitleProvidersSection } from "@/components/views/settings/settings-subtitle-providers-section";
import { SettingsSubtitlesSection } from "@/components/views/settings/settings-subtitles-section";
import type {
  PluginInstallProgressRecord,
  RegistryPluginRecord,
} from "@/components/views/settings/settings-plugins-section";
import { claimPluginTerminalOperation } from "@/lib/utils/plugin-install-state";
import {
  pluginInstallProgressSubscription,
  pluginsQuery,
  subtitleProviderConfigsQuery,
  subtitleSettingsInitQuery,
  subtitleProviderTypesQuery,
} from "@/lib/graphql/queries";
import {
  beginInstallPluginMutation,
  createSubtitleProviderConfigMutation,
  deleteSubtitleProviderConfigMutation,
  testSubtitleProviderConnectionMutation,
  updateSubtitleProviderConfigMutation,
  updateSubtitleSettingsMutation,
} from "@/lib/graphql/mutations";
import { providerConfigRecordToValues } from "@/lib/utils/provider-config";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { wsClient } from "@/lib/graphql/ws-client";
import { runConnectionFeedback } from "@/lib/utils/connection-feedback";
import type {
  ConfigFieldDef,
  SubtitleProviderConfigRecord,
  SubtitleProviderDraft,
  SubtitleProviderTypeInfo,
  SubtitleProviderValidationResult,
  SubtitleSettings,
} from "@/lib/types";

const DEFAULTS: SubtitleSettings = {
  enabled: false,
  languages: [],
  autoDownloadOnImport: false,
  minimumScoreSeries: 90,
  minimumScoreMovie: 70,
  searchIntervalHours: 6,
  includeAiTranslated: false,
  includeMachineTranslated: false,
  syncEnabled: true,
  syncThresholdSeries: 90,
  syncThresholdMovie: 70,
  syncMaxOffsetSeconds: 60,
};

const ENHANCED_SUBTITLE_SYNC_PLUGIN_ID = "enhanced-subtitle-sync";
const ENHANCED_SUBTITLE_SYNC_PLUGIN_NAME = "Enhanced Subtitle Sync";
const SUBTITLES_PANEL_CLASS =
  "overflow-hidden rounded-[14px] border border-[var(--scry-border)] bg-[var(--scry-surf)] shadow-[0_10px_24px_rgba(0,0,0,0.16)]";
const SUBTITLES_PANEL_HEADER_CLASS =
  "flex flex-col gap-4 border-b border-[var(--scry-border3)] bg-[linear-gradient(180deg,rgba(255,255,255,0.035),rgba(255,255,255,0))] px-4 py-3 sm:flex-row sm:items-center sm:justify-between";
const SUBTITLES_PANEL_TITLE_CLASS =
  "text-[15px] font-semibold text-[var(--scry-ink2)]";
const SUBTITLES_PANEL_BODY_CLASS = "p-4 sm:p-5";
const SUBTITLES_MUTED_TEXT_CLASS = "text-[var(--scry-muted3)]";

type PluginInstallProgressSubscriptionResult = {
  data?: {
    pluginInstallProgress?: PluginInstallProgressRecord;
  };
};

const DEFAULT_PROVIDER_DRAFT: SubtitleProviderDraft = {
  name: "",
  providerType: "",
  isEnabled: true,
  enabledFacets: [],
  configValues: {},
  persistedConfigValues: {},
  storedSecretKeys: [],
  configDirty: false,
};

const LEGACY_OPENSUBTITLES_CONFIG_KEYS = new Set([
  "include_ai_translated",
  "include_machine_translated",
]);

function sanitizePersistedConfigValues(
  providerType: string,
  configValues: Record<string, string>,
): Record<string, string> {
  if (providerType.trim().toLowerCase() !== "opensubtitles") {
    return configValues;
  }

  return Object.fromEntries(
    Object.entries(configValues).filter(
      ([key]) => !LEGACY_OPENSUBTITLES_CONFIG_KEYS.has(key),
    ),
  );
}

function buildDraftConfigValues(
  fields: ConfigFieldDef[],
  parsedConfigValues: Record<string, string>,
): Record<string, string> {
  if (fields.length === 0) {
    return { ...parsedConfigValues };
  }

  const nextValues: Record<string, string> = {};
  for (const field of fields) {
    if (field.valueSource === "HOST_BINDING") {
      continue;
    }
    nextValues[field.key] =
      parsedConfigValues[field.key] ??
      field.defaultValue ??
      (field.fieldType === "BOOL" ? "false" : "");
  }
  return nextValues;
}

function serializeProviderConfigValues(
  fields: ConfigFieldDef[],
  configValues: Record<string, string>,
  persistedConfigValues: Record<string, string>,
): ReturnType<typeof providerConfigRecordToValues> {
  const entries: Record<string, string> = {};

  if (fields.length === 0) {
    for (const [key, value] of Object.entries(configValues)) {
      if (value.trim() !== "") {
        entries[key] = value;
      }
    }
    return providerConfigRecordToValues(entries);
  }

  const fieldKeySet = new Set(fields.map((field) => field.key));
  const secretInputKeys = fields
    .filter((field) => field.fieldType === "PASSWORD")
    .map((field) => field.key);
  for (const [key, value] of Object.entries(persistedConfigValues)) {
    if (!fieldKeySet.has(key) && value.trim() !== "") {
      entries[key] = value;
    }
  }

  for (const field of fields) {
    if (field.valueSource === "HOST_BINDING") {
      continue;
    }

    let nextValue = configValues[field.key] ?? "";
    const isSecretField = field.fieldType === "PASSWORD";

    if (isSecretField && nextValue.trim() === "") {
      continue;
    }

    if (field.fieldType === "BOOL") {
      entries[field.key] =
        nextValue.trim() || field.defaultValue || "false";
      continue;
    }

    if (nextValue.trim() === "" && field.defaultValue) {
      nextValue = field.defaultValue;
    }

    if (nextValue.trim() !== "") {
      entries[field.key] = nextValue;
    }
  }

  return providerConfigRecordToValues(entries, secretInputKeys);
}

type SettingsSubtitlesContainerProps = {
  providerCatalogVersion?: number;
};

type PendingSubtitleProviderEditorAction =
  | { type: "create" }
  | { type: "edit"; provider: SubtitleProviderConfigRecord }
  | { type: "close" }
  | null;

function cloneProviderDraft(draft: SubtitleProviderDraft): SubtitleProviderDraft {
  return {
    ...draft,
    enabledFacets: [...draft.enabledFacets],
    configValues: { ...draft.configValues },
    persistedConfigValues: { ...draft.persistedConfigValues },
    storedSecretKeys: [...draft.storedSecretKeys],
  };
}

export function SettingsSubtitlesContainer({
  providerCatalogVersion = 0,
}: SettingsSubtitlesContainerProps) {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const [settings, setSettings] = React.useState<SubtitleSettings>(DEFAULTS);
  const [saving, setSaving] = React.useState(false);
  const [loading, setLoading] = React.useState(true);
  const [plugins, setPlugins] = React.useState<RegistryPluginRecord[]>([]);
  const [pluginsTarget, setPluginsTarget] = React.useState<HTMLElement | null>(null);
  const [pluginsLoading, setPluginsLoading] = React.useState(true);
  const [syncPluginInstallError, setSyncPluginInstallError] = React.useState<string | null>(
    null,
  );
  const [syncPluginProgress, setSyncPluginProgress] =
    React.useState<PluginInstallProgressRecord | null>(null);
  const [installingSyncPlugin, setInstallingSyncPlugin] = React.useState(false);
  const [providerTypes, setProviderTypes] = React.useState<SubtitleProviderTypeInfo[]>([]);
  const [providerConfigs, setProviderConfigs] = React.useState<SubtitleProviderConfigRecord[]>([]);
  const [providerDraft, setProviderDraft] = React.useState<SubtitleProviderDraft>(
    DEFAULT_PROVIDER_DRAFT,
  );
  const [editingProviderId, setEditingProviderId] = React.useState<string | null>(null);
  const [mutatingProviderId, setMutatingProviderId] = React.useState<string | null>(null);
  const [pendingDeleteProvider, setPendingDeleteProvider] =
    React.useState<SubtitleProviderConfigRecord | null>(null);
  const [isTestingProviderConnection, setIsTestingProviderConnection] =
    React.useState(false);
  const [isProviderEditorOpen, setIsProviderEditorOpen] = React.useState(false);
  const [providerEditorMode, setProviderEditorMode] =
    React.useState<"create" | "edit">("create");
  const [pendingProviderEditorAction, setPendingProviderEditorAction] =
    React.useState<PendingSubtitleProviderEditorAction>(null);
  const [providerDraftBaseline, setProviderDraftBaseline] =
    React.useState<SubtitleProviderDraft>(() =>
      cloneProviderDraft(DEFAULT_PROVIDER_DRAFT),
    );
  const [awaitingProviderBaselineSync, setAwaitingProviderBaselineSync] =
    React.useState(false);
  const [subtitlesExpanded, setSubtitlesExpanded] = React.useState(true);
  const loadedRef = React.useRef(false);
  const lastSubmittedSettingsRef = React.useRef<SubtitleSettings | null>(null);
  const providerCatalogVersionRef = React.useRef(providerCatalogVersion);
  const syncPluginProgressSubscriptionRef = React.useRef<(() => void) | null>(
    null,
  );
  // Whichever of `next`, `error` and `complete` reaches the plugin first owns
  // winding the install down; the others must not undo that work.
  const syncPluginTerminalRef = React.useRef(new Set<string>());

  React.useEffect(() => {
    setPluginsTarget(document.getElementById(SETTINGS_REFERENCE_SLOT_ID));
  }, []);

  const resetProviderDraft = React.useCallback(() => {
    setEditingProviderId(null);
    setProviderDraft(cloneProviderDraft(DEFAULT_PROVIDER_DRAFT));
  }, []);

  React.useEffect(() => {
    if (!awaitingProviderBaselineSync) {
      return;
    }

    setProviderDraftBaseline(cloneProviderDraft(providerDraft));
    setAwaitingProviderBaselineSync(false);
  }, [awaitingProviderBaselineSync, providerDraft]);

  React.useEffect(() => {
    setSubtitlesExpanded(settings.enabled);
  }, [settings.enabled]);

  const isProviderDraftDirty =
    JSON.stringify(providerDraft) !== JSON.stringify(providerDraftBaseline);

  const loadPlugins = React.useCallback(async () => {
    const { data, error } = await client
      .query(pluginsQuery, {}, { requestPolicy: "network-only" })
      .toPromise();
    if (error) {
      throw error;
    }
    return (data?.plugins ?? []) as RegistryPluginRecord[];
  }, [client]);

  const stopSyncPluginProgressSubscription = React.useCallback(() => {
    const unsubscribe = syncPluginProgressSubscriptionRef.current;
    if (unsubscribe) {
      unsubscribe();
      syncPluginProgressSubscriptionRef.current = null;
    }
  }, []);

  const refreshProviderConfigs = React.useCallback(async () => {
    const { data, error } = await client
      .query(subtitleProviderConfigsQuery, {}, { requestPolicy: "network-only" })
      .toPromise();
    if (error) {
      throw error;
    }
    setProviderConfigs(
      (data?.subtitleProviderConfigs ?? []) as SubtitleProviderConfigRecord[],
    );
  }, [client]);

  const refreshProviderTypes = React.useCallback(async () => {
    const { data, error } = await client
      .query(subtitleProviderTypesQuery, {}, { requestPolicy: "network-only" })
      .toPromise();
    if (error) {
      throw error;
    }
    setProviderTypes(
      (data?.subtitleProviderTypes ?? []) as SubtitleProviderTypeInfo[],
    );
  }, [client]);

  React.useEffect(() => {
    let cancelled = false;
    setPluginsLoading(true);
    loadPlugins()
      .then((nextPlugins) => {
        if (cancelled) {
          return;
        }
        setPlugins(nextPlugins);
        setSyncPluginInstallError(null);
      })
      .catch((error: unknown) => {
        if (cancelled) {
          return;
        }
        const message =
          error instanceof Error ? error.message : t("settings.sub.syncPluginLoadFailed");
        setPlugins([]);
        setSyncPluginInstallError(message);
      })
      .finally(() => {
        if (!cancelled) {
          setPluginsLoading(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [loadPlugins, t]);

  React.useEffect(
    () => () => {
      stopSyncPluginProgressSubscription();
    },
    [stopSyncPluginProgressSubscription],
  );

  React.useEffect(() => {
    if (editingProviderId || providerTypes.length === 0) {
      return;
    }

    setProviderDraft((previous) => {
      const configuredProvider =
        providerTypes.find(
          (providerType) => providerType.providerType === previous.providerType,
        ) ?? null;
      const nextProvider = configuredProvider ?? providerTypes[0] ?? null;
      if (!nextProvider) {
        return previous;
      }

      const shouldAutofillName =
        previous.name.trim().length === 0 ||
        previous.name === (configuredProvider?.name ?? previous.providerType);
      const nextProviderType = configuredProvider
        ? previous.providerType
        : nextProvider.providerType;
      const nextName = shouldAutofillName ? nextProvider.name : previous.name;

      if (
        nextProviderType === previous.providerType &&
        nextName === previous.name
      ) {
        return previous;
      }

      return {
        ...previous,
        providerType: nextProviderType,
        name: nextName,
      };
    });
  }, [editingProviderId, providerTypes]);

  React.useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const { data, error } = await client
          .query(subtitleSettingsInitQuery, {}, { requestPolicy: "network-only" })
          .toPromise();
        if (error) {
          throw error;
        }
        if (cancelled) {
          return;
        }

        const payload = data?.subtitleSettings;
        const nextSettings = {
          ...DEFAULTS,
          ...(payload ?? {}),
        };
        lastSubmittedSettingsRef.current = nextSettings;
        setSettings(nextSettings);
        setProviderTypes(
          (data?.subtitleProviderTypes ?? []) as SubtitleProviderTypeInfo[],
        );
        setProviderConfigs(
          (data?.subtitleProviderConfigs ?? []) as SubtitleProviderConfigRecord[],
        );
        loadedRef.current = true;
      } catch {
        // Keep defaults on failure.
      } finally {
        if (!cancelled) {
          setLoading(false);
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [client]);

  React.useEffect(() => {
    if (providerCatalogVersion === providerCatalogVersionRef.current) {
      return;
    }

    providerCatalogVersionRef.current = providerCatalogVersion;
    void refreshProviderTypes().catch((error: unknown) => {
      const message =
        error instanceof Error ? error.message : t("status.failedToLoad");
      setGlobalStatus(message);
    });
  }, [providerCatalogVersion, refreshProviderTypes, setGlobalStatus, t]);

  const syncPlugin = React.useMemo(
    () =>
      plugins.find(
        (plugin) =>
          plugin.id === ENHANCED_SUBTITLE_SYNC_PLUGIN_ID ||
          plugin.providerType === ENHANCED_SUBTITLE_SYNC_PLUGIN_ID,
      ) ?? null,
    [plugins],
  );
  const syncPluginName = syncPlugin?.name ?? ENHANCED_SUBTITLE_SYNC_PLUGIN_NAME;
  const syncPluginActive = syncPlugin?.isInstalled === true && syncPlugin.isEnabled;
  const syncPluginInstallBusy =
    installingSyncPlugin || syncPlugin?.installInProgress === true;
  const syncPluginBlockedReason =
    syncPlugin?.isInstalled === true && !syncPlugin.isEnabled
      ? t("settings.sub.syncPluginDisabled")
      : syncPlugin?.blockedReason
        ? t("settings.sub.syncPluginBlocked", { reason: syncPlugin.blockedReason })
        : null;

  const beginSyncPluginProgress = React.useCallback(
    (
      plugin: RegistryPluginRecord,
      initialSnapshot: PluginInstallProgressRecord,
    ) => {
      stopSyncPluginProgressSubscription();
      syncPluginTerminalRef.current.delete(plugin.id);
      setSyncPluginProgress(initialSnapshot);
      const unsubscribe = wsClient.subscribe(
        {
          query: pluginInstallProgressSubscription,
          variables: { pluginId: plugin.id },
        },
        {
          next: (result: PluginInstallProgressSubscriptionResult) => {
            const snapshot = result.data?.pluginInstallProgress;
            if (!snapshot) {
              return;
            }
            setSyncPluginProgress(snapshot);

            if (snapshot.state === "SUCCEEDED" || snapshot.state === "FAILED") {
              if (
                !claimPluginTerminalOperation(
                  syncPluginTerminalRef.current,
                  plugin.id,
                )
              ) {
                return;
              }
              stopSyncPluginProgressSubscription();
              void (async () => {
                try {
                  if (snapshot.state === "SUCCEEDED") {
                    setSyncPluginInstallError(null);
                    const nextPlugins = await loadPlugins();
                    setPlugins(nextPlugins);
                    await refreshProviderTypes();
                    setGlobalStatus(
                      t("settings.sub.syncPluginInstalled", {
                        plugin: plugin.name,
                      }),
                    );
                  } else {
                    const detail = snapshot.error ?? snapshot.label;
                    const message = t("settings.sub.syncPluginInstallFailed", {
                      plugin: plugin.name,
                      error: detail,
                    });
                    setSyncPluginInstallError(message);
                    setGlobalStatus(message);
                  }
                } catch (error: unknown) {
                  const detail = error instanceof Error ? error.message : String(error);
                  const message = t("settings.sub.syncPluginInstallFailed", {
                    plugin: plugin.name,
                    error: detail,
                  });
                  setSyncPluginInstallError(message);
                  setGlobalStatus(message);
                } finally {
                  setInstallingSyncPlugin(false);
                  setSyncPluginProgress(null);
                }
              })();
            }
          },
          error: (error) => {
            if (
              !claimPluginTerminalOperation(
                syncPluginTerminalRef.current,
                plugin.id,
              )
            ) {
              return;
            }
            stopSyncPluginProgressSubscription();
            setInstallingSyncPlugin(false);
            setSyncPluginProgress(null);
            const detail = error instanceof Error ? error.message : String(error);
            const message = t("settings.sub.syncPluginInstallFailed", {
              plugin: plugin.name,
              error: detail,
            });
            setSyncPluginInstallError(message);
            setGlobalStatus(message);
          },
          complete: () => {
            syncPluginProgressSubscriptionRef.current = null;
            // A stream that ends without a terminal snapshot — the finished
            // snapshot aged out before this connection attached, a reconnect
            // resubscribed too late, an auth epoch bumped the socket — used to
            // leave the button pinned on "Installing" with nothing left to
            // correct it. Reconcile the way reloading the page did.
            if (
              !claimPluginTerminalOperation(
                syncPluginTerminalRef.current,
                plugin.id,
              )
            ) {
              return;
            }
            setInstallingSyncPlugin(false);
            setSyncPluginProgress(null);
            void loadPlugins()
              .then((nextPlugins) => setPlugins(nextPlugins))
              .catch(() => undefined);
          },
        },
      );
      syncPluginProgressSubscriptionRef.current = unsubscribe;
    },
    [
      loadPlugins,
      refreshProviderTypes,
      setGlobalStatus,
      stopSyncPluginProgressSubscription,
      t,
    ],
  );

  const installSyncPlugin = React.useCallback(async () => {
    if (
      !syncPlugin ||
      syncPlugin.blockedReason ||
      syncPlugin.isInstalled ||
      syncPlugin.installInProgress
    ) {
      return;
    }

    setInstallingSyncPlugin(true);
    setSyncPluginInstallError(null);
    setSyncPluginProgress(null);
    try {
	      const { data, error } = await client
	        .mutation(beginInstallPluginMutation, { pluginId: syncPlugin.id })
        .toPromise();
      if (error) {
        throw error;
      }
      const snapshot = data?.beginInstallPlugin as
        | PluginInstallProgressRecord
        | undefined;
      if (!snapshot) {
        throw new Error("plugin install did not return progress");
      }
      beginSyncPluginProgress(syncPlugin, snapshot);
    } catch (error: unknown) {
      const detail = error instanceof Error ? error.message : String(error);
      const message = t("settings.sub.syncPluginInstallFailed", {
        plugin: syncPluginName,
        error: detail,
      });
      setSyncPluginInstallError(message);
      setGlobalStatus(message);
      setInstallingSyncPlugin(false);
    }
  }, [
    beginSyncPluginProgress,
    client,
    setGlobalStatus,
    syncPlugin,
    syncPluginName,
    t,
  ]);

  React.useEffect(() => {
    if (
      !loadedRef.current ||
      settings === lastSubmittedSettingsRef.current
    ) {
      return;
    }

    lastSubmittedSettingsRef.current = settings;
    setSaving(true);
    client
      .mutation(updateSubtitleSettingsMutation, {
        input: {
          enabled: settings.enabled,
          languages: settings.languages.map((language) => ({
            code: language.code,
            hearingImpaired: language.hearingImpaired,
            forced: language.forced,
          })),
          autoDownloadOnImport: settings.autoDownloadOnImport,
          minimumScoreSeries: settings.minimumScoreSeries,
          minimumScoreMovie: settings.minimumScoreMovie,
          searchIntervalHours: settings.searchIntervalHours,
          includeAiTranslated: settings.includeAiTranslated,
          includeMachineTranslated: settings.includeMachineTranslated,
          syncEnabled: settings.syncEnabled,
          syncThresholdSeries: settings.syncThresholdSeries,
          syncThresholdMovie: settings.syncThresholdMovie,
          syncMaxOffsetSeconds: settings.syncMaxOffsetSeconds,
        },
      })
      .toPromise()
      .then(({ error }) => {
        if (error) {
          const message = error.message || t("status.failedToUpdate");
          setGlobalStatus(message);
        }
      })
      .catch((error: unknown) => {
        const message =
          error instanceof Error ? error.message : t("status.failedToUpdate");
        setGlobalStatus(message);
      })
      .finally(() => setSaving(false));
  }, [client, setGlobalStatus, settings, t]);

  const submitProvider = React.useCallback(
    async (event: React.FormEvent<HTMLFormElement>) => {
      event.preventDefault();

      const normalizedProviderType = providerDraft.providerType.trim().toLowerCase();
      const selectedProvider =
        providerTypes.find(
          (providerType) => providerType.providerType === normalizedProviderType,
        ) ?? null;
      const persistedConfigValues = sanitizePersistedConfigValues(
        normalizedProviderType,
        providerDraft.persistedConfigValues,
      );
      const payload = {
        name: providerDraft.name.trim(),
        providerType: normalizedProviderType,
        isEnabled: providerDraft.isEnabled,
        enabledFacets: providerDraft.enabledFacets,
      };
      const config = serializeProviderConfigValues(
        selectedProvider?.configFields ?? [],
        providerDraft.configValues,
        persistedConfigValues,
      );

      if (!payload.name || !payload.providerType) {
        setGlobalStatus(t("form.subtitleProviderValidation"));
        return;
      }

      setMutatingProviderId(editingProviderId || "new");
      try {
        if (editingProviderId) {
          const { error } = await client
            .mutation(updateSubtitleProviderConfigMutation, {
              input: {
                id: editingProviderId,
                name: payload.name,
                providerType: payload.providerType,
                config: providerDraft.configDirty ? config : undefined,
                enabledFacets: payload.enabledFacets,
                isEnabled: payload.isEnabled,
              },
            })
            .toPromise();
          if (error) {
            throw error;
          }
          setGlobalStatus(t("settings.subtitleProviderUpdated"));
        } else {
          const { error } = await client
            .mutation(createSubtitleProviderConfigMutation, {
              input: {
                name: payload.name,
                providerType: payload.providerType,
                config,
                enabledFacets: payload.enabledFacets,
                isEnabled: payload.isEnabled,
              },
            })
            .toPromise();
          if (error) {
            throw error;
          }
          setGlobalStatus(t("settings.subtitleProviderCreated"));
        }
        resetProviderDraft();
        setIsProviderEditorOpen(false);
        setProviderEditorMode("create");
        setAwaitingProviderBaselineSync(true);
        await refreshProviderConfigs();
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.failedToUpdate"),
        );
      } finally {
        setMutatingProviderId(null);
      }
    },
    [
      client,
      editingProviderId,
      providerDraft,
      providerTypes,
      refreshProviderConfigs,
      resetProviderDraft,
      setGlobalStatus,
      t,
    ],
  );

  const editProvider = React.useCallback(
    (provider: SubtitleProviderConfigRecord) => {
      const persistedConfigValues = sanitizePersistedConfigValues(
        provider.providerType,
        {},
      );
      const selectedProvider =
        providerTypes.find(
          (providerType) =>
            providerType.providerType === provider.providerType.toLowerCase(),
        ) ?? null;

      setEditingProviderId(provider.id);
      setProviderDraft({
        name: provider.name,
        providerType: provider.providerType,
        isEnabled: provider.isEnabled,
        enabledFacets: provider.enabledFacets ?? [],
        configValues: buildDraftConfigValues(
          selectedProvider?.configFields ?? [],
          persistedConfigValues,
        ),
        persistedConfigValues,
        storedSecretKeys: provider.storedSecretKeys ?? [],
        configDirty: false,
      });
      setGlobalStatus(t("status.editingSubtitleProvider", { name: provider.name }));
    },
    [providerTypes, setGlobalStatus, t],
  );

  const openCreateProviderEditor = React.useCallback(() => {
    resetProviderDraft();
    setProviderEditorMode("create");
    setIsProviderEditorOpen(true);
    setAwaitingProviderBaselineSync(true);
  }, [resetProviderDraft]);

  const openEditProviderEditor = React.useCallback(
    (provider: SubtitleProviderConfigRecord) => {
      editProvider(provider);
      setProviderEditorMode("edit");
      setIsProviderEditorOpen(true);
      setAwaitingProviderBaselineSync(true);
    },
    [editProvider],
  );

  const requestCreateProviderEditor = React.useCallback(() => {
    if (!isProviderEditorOpen || !isProviderDraftDirty) {
      openCreateProviderEditor();
      return;
    }

    setPendingProviderEditorAction({ type: "create" });
  }, [
    isProviderDraftDirty,
    isProviderEditorOpen,
    openCreateProviderEditor,
  ]);

  const requestEditProvider = React.useCallback(
    (provider: SubtitleProviderConfigRecord) => {
      if (!isProviderEditorOpen || !isProviderDraftDirty) {
        openEditProviderEditor(provider);
        return;
      }

      setPendingProviderEditorAction({ type: "edit", provider });
    },
    [isProviderDraftDirty, isProviderEditorOpen, openEditProviderEditor],
  );

  const requestCloseProviderEditor = React.useCallback(() => {
    if (!isProviderEditorOpen) {
      return;
    }

    if (!isProviderDraftDirty) {
      setIsProviderEditorOpen(false);
      setProviderEditorMode("create");
      resetProviderDraft();
      setAwaitingProviderBaselineSync(true);
      return;
    }

    setPendingProviderEditorAction({ type: "close" });
  }, [isProviderDraftDirty, isProviderEditorOpen, resetProviderDraft]);

  const confirmPendingProviderEditorAction = React.useCallback(() => {
    if (!pendingProviderEditorAction) {
      return;
    }

    if (pendingProviderEditorAction.type === "create") {
      openCreateProviderEditor();
    } else if (pendingProviderEditorAction.type === "edit") {
      openEditProviderEditor(pendingProviderEditorAction.provider);
    } else {
      setIsProviderEditorOpen(false);
      setProviderEditorMode("create");
      resetProviderDraft();
      setAwaitingProviderBaselineSync(true);
    }

    setPendingProviderEditorAction(null);
  }, [
    openCreateProviderEditor,
    openEditProviderEditor,
    pendingProviderEditorAction,
    resetProviderDraft,
  ]);

  const toggleProviderEnabled = React.useCallback(
    async (provider: SubtitleProviderConfigRecord) => {
      setMutatingProviderId(provider.id);
      try {
        const { error } = await client
          .mutation(updateSubtitleProviderConfigMutation, {
            input: {
              id: provider.id,
              isEnabled: !provider.isEnabled,
              enabledFacets: provider.enabledFacets ?? [],
            },
          })
          .toPromise();
        if (error) {
          throw error;
        }
        setGlobalStatus(t("settings.subtitleProviderUpdated"));
        await refreshProviderConfigs();
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.failedToUpdate"),
        );
      } finally {
        setMutatingProviderId(null);
      }
    },
    [client, refreshProviderConfigs, setGlobalStatus, t],
  );

  const deleteProvider = React.useCallback(
    async (provider: SubtitleProviderConfigRecord) => {
      setPendingDeleteProvider(provider);
    },
    [],
  );

  const confirmDeleteProvider = React.useCallback(async () => {
    if (!pendingDeleteProvider) {
      return;
    }

    setMutatingProviderId(pendingDeleteProvider.id);
    try {
      const { error } = await client
        .mutation(deleteSubtitleProviderConfigMutation, {
          id: pendingDeleteProvider.id,
        })
        .toPromise();
      if (error) {
        throw error;
      }
      setGlobalStatus(
        t("settings.subtitleProviderDeleted", {
          name: pendingDeleteProvider.name,
        }),
      );
      await refreshProviderConfigs();
      if (editingProviderId === pendingDeleteProvider.id) {
        resetProviderDraft();
        setIsProviderEditorOpen(false);
        setProviderEditorMode("create");
        setAwaitingProviderBaselineSync(true);
      }
    } catch (error) {
      setGlobalStatus(
        error instanceof Error ? error.message : t("status.failedToDelete"),
      );
    } finally {
      setMutatingProviderId(null);
      setPendingDeleteProvider(null);
    }
  }, [
    client,
    editingProviderId,
    pendingDeleteProvider,
    refreshProviderConfigs,
    resetProviderDraft,
    setGlobalStatus,
    t,
  ]);

  const testProviderConnection = React.useCallback(async () => {
    const normalizedProviderType = providerDraft.providerType.trim().toLowerCase();
    const selectedProvider =
      providerTypes.find(
        (providerType) => providerType.providerType === normalizedProviderType,
      ) ?? null;
    const persistedConfigValues = sanitizePersistedConfigValues(
      normalizedProviderType,
      providerDraft.persistedConfigValues,
    );
    if (!normalizedProviderType) {
      setGlobalStatus(t("form.subtitleProviderValidation"));
      return;
    }

    setIsTestingProviderConnection(true);
    try {
      await runConnectionFeedback({
        setGlobalStatus,
        startMessage: t("status.testingSubtitleProviderConnection"),
        successMessage: t("status.subtitleProviderConnectionTestPassed"),
        failureFallbackMessage: t("status.subtitleProviderConnectionTestFailed"),
        run: async () => {
          const { data, error } = await client
            .mutation(testSubtitleProviderConnectionMutation, {
              input: {
                id: editingProviderId ?? undefined,
                providerType: normalizedProviderType,
                config: serializeProviderConfigValues(
                  selectedProvider?.configFields ?? [],
                  providerDraft.configValues,
                  persistedConfigValues,
                ),
              },
            })
            .toPromise();
          if (error) {
            throw error;
          }

          const validation = data?.testSubtitleProviderConnection as
            | SubtitleProviderValidationResult
            | undefined;
          const success =
            validation?.status === "valid" || validation?.status === "ok";
          if (!success) {
            throw new Error(
              validation?.message ||
                t("status.subtitleProviderConnectionTestFailed"),
            );
          }

          return (
            validation?.message ||
            t("status.subtitleProviderConnectionTestPassed")
          );
        },
      });
    } catch {
      // Connection feedback is already surfaced through the shared helper.
    } finally {
      setIsTestingProviderConnection(false);
    }
  }, [client, editingProviderId, providerDraft, providerTypes, setGlobalStatus, t]);

  return (
    <>
      {pluginsTarget
        ? createPortal(
            <FilteredPluginList
              family="SUBTITLE"
              refreshProviderOptions={refreshProviderTypes}
            />,
            pluginsTarget,
          )
        : null}
      <div className="space-y-4">
        <section id="settings-subtitles-section" className={SUBTITLES_PANEL_CLASS}>
          <div className={SUBTITLES_PANEL_HEADER_CLASS}>
            <button
              type="button"
              className="flex min-w-0 flex-1 items-start gap-3 text-left"
              onClick={() => setSubtitlesExpanded((current) => !current)}
              aria-expanded={subtitlesExpanded}
            >
              <div className="min-w-0 flex-1 space-y-1">
                <h2 className={`flex items-center gap-2 ${SUBTITLES_PANEL_TITLE_CLASS}`}>
                  <Subtitles className="h-4 w-4" />
                  {t("settings.subtitles")}
                  {saving ? (
                    <Loader2 className={`h-3.5 w-3.5 animate-spin ${SUBTITLES_MUTED_TEXT_CLASS}`} />
                  ) : null}
                </h2>
                <p className={`text-sm ${SUBTITLES_MUTED_TEXT_CLASS}`}>
                  {t("settings.subtitlesDescription")}
                </p>
              </div>
            </button>
            <div className="flex shrink-0 justify-end">
              <SettingsToggleSwitch
                id="settings-subtitles-enabled"
                checked={settings.enabled}
                disabled={loading}
                size="lg"
                ariaLabel={
                  settings.enabled ? t("label.enabled") : t("label.disabled")
                }
                onChange={(nextValue) =>
                  setSettings({ ...settings, enabled: nextValue })
                }
              />
            </div>
          </div>
          {subtitlesExpanded ? (
            <div className={`${SUBTITLES_PANEL_BODY_CLASS} space-y-6`}>
              <SettingsSubtitlesSection
                settings={settings}
                setSettings={setSettings}
                loading={loading}
                syncPluginActive={syncPluginActive}
                syncPluginAvailable={syncPlugin !== null}
                syncPluginBlockedReason={syncPluginBlockedReason}
                syncPluginError={syncPluginInstallError}
                syncPluginInstalling={syncPluginInstallBusy}
                syncPluginLoading={pluginsLoading}
                syncPluginName={syncPluginName}
                syncPluginProgress={syncPluginProgress}
                onInstallSyncPlugin={installSyncPlugin}
              />
              {!loading ? (
                <SettingsSubtitleProvidersSection
                  editingProviderId={editingProviderId}
                  providerDraft={providerDraft}
                  setProviderDraft={setProviderDraft}
                  submitProvider={submitProvider}
                  mutatingProviderId={mutatingProviderId}
                  resetProviderDraft={requestCloseProviderEditor}
                  providerConfigs={providerConfigs}
                  editProvider={requestEditProvider}
                  toggleProviderEnabled={toggleProviderEnabled}
                  deleteProvider={deleteProvider}
                  providerTypes={providerTypes}
                  testProviderConnection={testProviderConnection}
                  isTestingConnection={isTestingProviderConnection}
                  isEditorOpen={isProviderEditorOpen}
                  editorMode={providerEditorMode}
                  startCreateProvider={requestCreateProviderEditor}
                />
              ) : null}
            </div>
          ) : null}
        </section>
      </div>
      <ConfirmDialog
        open={pendingProviderEditorAction !== null}
        title={t("settings.subtitleProviderConfirmDiscardTitle")}
        description={t("settings.subtitleProviderConfirmDiscardDescription")}
        confirmLabel={
          pendingProviderEditorAction?.type === "create"
            ? t("settings.subtitleProviderCreateNew")
            : pendingProviderEditorAction?.type === "edit"
              ? t("label.edit")
              : t("label.discard")
        }
        cancelLabel={t("label.cancel")}
        isBusy={mutatingProviderId !== null}
        onConfirm={confirmPendingProviderEditorAction}
        onCancel={() => setPendingProviderEditorAction(null)}
      />
      <ConfirmDialog
        open={pendingDeleteProvider !== null}
        title={t("label.delete")}
        description={
          pendingDeleteProvider
            ? t("status.deletingSubtitleProvider", {
                name: pendingDeleteProvider.name,
              })
            : ""
        }
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        isBusy={mutatingProviderId !== null}
        onConfirm={confirmDeleteProvider}
        onCancel={() => setPendingDeleteProvider(null)}
      />
    </>
  );
}

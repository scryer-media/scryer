
import { type FormEvent, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { FilteredPluginList } from "@/components/views/settings/filtered-plugin-list";
import { SettingsNotificationsSection } from "@/components/views/settings/settings-notifications-section";
import { SETTINGS_REFERENCE_SLOT_ID } from "@/components/containers/settings/settings-container";
import { useClient } from "urql";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { runConnectionFeedback } from "@/lib/utils/connection-feedback";
import {
  providerConfigRecordToValuesWithDefaults,
  providerConfigValuesToRecord,
} from "@/lib/utils/provider-config";
import type {
  NotificationChannel,
  NotificationChannelDraft,
  NotificationProviderType,
  NotificationSubscription,
  NotificationSubscriptionDraft,
  NotificationSubscriptionRow,
  NotificationTarget,
  TitleRecord,
} from "@/lib/types";
import {
  notificationChannelsQuery,
  notificationTargetsQuery,
  notificationProviderTypesQuery,
  notificationSubscriptionsQuery,
  notificationsInitQuery,
  titleAutocompleteSelectionQuery,
} from "@/lib/graphql/queries";
import {
  localPathStyleFromRuntimeValue,
  type LocalPathStyle,
} from "@/lib/utils/local-path-style";
import {
  createNotificationChannelMutation,
  updateNotificationChannelMutation,
  deleteNotificationChannelMutation,
  testNotificationChannelMutation,
  createNotificationSubscriptionMutation,
  updateNotificationSubscriptionMutation,
  deleteNotificationSubscriptionMutation,
} from "@/lib/graphql/mutations";

const CHANNEL_INITIAL_DRAFT: NotificationChannelDraft = {
  name: "",
  channelType: "",
  mediaServerConnectionId: "",
  isEnabled: true,
  configValues: {},
};

const SUBSCRIPTION_INITIAL_DRAFT: NotificationSubscriptionDraft = {
  targetKind: "plugin_channel",
  targetId: "",
  eventTypes: [],
  scope: "global",
  facetScopeIds: [],
  titleScopeId: "",
  titleScopeTitle: null,
  isEnabled: true,
};

const FACET_SCOPE_ORDER = ["movie", "series", "anime"] as const;

function isFacetScopeId(value: string): value is (typeof FACET_SCOPE_ORDER)[number] {
  return FACET_SCOPE_ORDER.includes(value as (typeof FACET_SCOPE_ORDER)[number]);
}

function parseFacetScopeIds(scopeId: string | null | undefined): string[] {
  if (!scopeId) {
    return [];
  }

  const selected = new Set<string>();
  for (const token of scopeId.split(",")) {
    const normalized = token.trim().toLowerCase();
    if (isFacetScopeId(normalized)) {
      selected.add(normalized);
    }
  }

  return FACET_SCOPE_ORDER.filter((facet) => selected.has(facet));
}

function serializeFacetScopeIds(scopeIds: Iterable<string>): string {
  const selected = new Set(scopeIds);
  return FACET_SCOPE_ORDER.filter((facet) => selected.has(facet)).join(",");
}

type NotificationSubscriptionSpec = {
  targetKind: NotificationSubscriptionDraft["targetKind"];
  targetId: string;
  eventType: string;
  scope: string;
  scopeId?: string;
  isEnabled: boolean;
};

function subscriptionSpecKey(spec: Pick<NotificationSubscriptionSpec, "targetKind" | "targetId" | "eventType" | "scope" | "scopeId">): string {
  return [spec.targetKind, spec.targetId, spec.eventType, spec.scope, spec.scopeId ?? ""].join("::");
}

function subscriptionRowKey(subscription: NotificationSubscription): string {
  return [
    subscription.targetKind,
    subscription.targetId,
    subscription.scope,
    subscription.scopeId ?? "",
    subscription.isEnabled ? "1" : "0",
  ].join("::");
}

function buildNotificationSubscriptionRows(
  subscriptions: NotificationSubscription[],
): NotificationSubscriptionRow[] {
  const groups = new Map<
    string,
    NotificationSubscriptionRow & { eventTypeSet: Set<string> }
  >();

  for (const subscription of subscriptions) {
    const key = subscriptionRowKey(subscription);
    const existing = groups.get(key);
    if (existing) {
      existing.eventTypeSet.add(subscription.eventType);
      existing.subscriptionIds.push(subscription.id);
      continue;
    }

    groups.set(key, {
      id: key,
      channelId: subscription.channelId,
      targetKind: subscription.targetKind,
      targetId: subscription.targetId,
      eventTypes: [],
      scope: subscription.scope,
      scopeId: subscription.scopeId,
      isEnabled: subscription.isEnabled,
      subscriptionIds: [subscription.id],
      eventTypeSet: new Set([subscription.eventType]),
    });
  }

  return Array.from(groups.values()).map(({ eventTypeSet, ...row }) => ({
    ...row,
    eventTypes: Array.from(eventTypeSet).sort(),
  }));
}

function buildNotificationSubscriptionSpecs(
  draft: NotificationSubscriptionDraft,
): NotificationSubscriptionSpec[] {
  const normalizedEventTypes = Array.from(
    new Set(draft.eventTypes.map((eventType) => eventType.trim()).filter(Boolean)),
  );
  const normalizedScope = draft.scope.trim();
  const normalizedFacetScopeId = serializeFacetScopeIds(draft.facetScopeIds);
  const normalizedTitleScopeId = draft.titleScopeId.trim();
  const scopeSpec =
    normalizedScope === "global"
      ? { scope: normalizedScope, scopeId: undefined }
      : normalizedScope === "facet"
        ? normalizedFacetScopeId
          ? { scope: normalizedScope, scopeId: normalizedFacetScopeId }
          : null
        : normalizedScope === "title"
          ? normalizedTitleScopeId
            ? { scope: normalizedScope, scopeId: normalizedTitleScopeId }
            : null
          : null;

  if (!scopeSpec) {
    return [];
  }

  return normalizedEventTypes.map((eventType) => ({
    targetKind: draft.targetKind,
    targetId: draft.targetId,
    eventType,
    scope: scopeSpec.scope,
    scopeId: scopeSpec.scopeId,
    isEnabled: draft.isEnabled,
  }));
}

type SettingsNotificationsContainerProps = {
  providerCatalogVersion?: number;
};

type PendingNotificationChannelEditorAction =
  | { type: "create" }
  | { type: "edit"; channel: NotificationChannel }
  | { type: "close" }
  | null;

type PendingNotificationSubscriptionEditorAction =
  | { type: "create"; target?: Pick<NotificationTarget, "targetKind" | "id"> }
  | { type: "edit"; subscription: NotificationSubscriptionRow }
  | { type: "close" }
  | null;

function cloneChannelDraft(draft: NotificationChannelDraft): NotificationChannelDraft {
  return {
    ...draft,
    configValues: { ...draft.configValues },
  };
}

function cloneSubscriptionDraft(
  draft: NotificationSubscriptionDraft,
): NotificationSubscriptionDraft {
  return {
    ...draft,
    eventTypes: [...draft.eventTypes],
    facetScopeIds: [...draft.facetScopeIds],
    titleScopeTitle: draft.titleScopeTitle,
  };
}

export function SettingsNotificationsContainer({
  providerCatalogVersion = 0,
}: SettingsNotificationsContainerProps) {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();

  // --- Channel state ---
  const [channels, setChannels] = useState<NotificationChannel[]>([]);
  const [localPathStyle, setLocalPathStyle] = useState<
    LocalPathStyle | undefined
  >(undefined);
  const [editingChannelId, setEditingChannelId] = useState<string | null>(null);
  const [mutatingChannelId, setMutatingChannelId] = useState<string | null>(null);
  const [pendingDeleteChannel, setPendingDeleteChannel] = useState<NotificationChannel | null>(null);
  const [channelDraft, setChannelDraft] = useState<NotificationChannelDraft>(() => cloneChannelDraft(CHANNEL_INITIAL_DRAFT));
  const [providerTypes, setProviderTypes] = useState<NotificationProviderType[]>([]);
  const [pluginsTarget, setPluginsTarget] = useState<HTMLElement | null>(null);
  useEffect(() => {
    setPluginsTarget(document.getElementById(SETTINGS_REFERENCE_SLOT_ID));
  }, []);
  const [notificationTargets, setNotificationTargets] = useState<NotificationTarget[]>([]);
  const [testingChannelId, setTestingChannelId] = useState<string | null>(null);
  const [isChannelEditorOpen, setIsChannelEditorOpen] = useState(false);
  const [channelEditorMode, setChannelEditorMode] = useState<"create" | "edit">("create");
  const [pendingChannelEditorAction, setPendingChannelEditorAction] =
    useState<PendingNotificationChannelEditorAction>(null);
  const [channelDraftBaseline, setChannelDraftBaseline] =
    useState<NotificationChannelDraft>(() => cloneChannelDraft(CHANNEL_INITIAL_DRAFT));
  const [awaitingChannelBaselineSync, setAwaitingChannelBaselineSync] =
    useState(false);

  // --- Subscription state ---
  const [subscriptions, setSubscriptions] = useState<NotificationSubscription[]>([]);
  const [editingSubscriptionId, setEditingSubscriptionId] = useState<string | null>(null);
  const [mutatingSubscriptionId, setMutatingSubscriptionId] = useState<string | null>(null);
  const [pendingDeleteSubscription, setPendingDeleteSubscription] = useState<NotificationSubscriptionRow | null>(null);
  const [subscriptionDraft, setSubscriptionDraft] =
    useState<NotificationSubscriptionDraft>(() =>
      cloneSubscriptionDraft(SUBSCRIPTION_INITIAL_DRAFT),
    );
  const [eventTypes, setEventTypes] = useState<string[]>([]);
  const [subscriptionTitlesById, setSubscriptionTitlesById] = useState<Record<string, TitleRecord | null>>({});
  const [isSubscriptionEditorOpen, setIsSubscriptionEditorOpen] = useState(false);
  const [subscriptionEditorMode, setSubscriptionEditorMode] =
    useState<"create" | "edit">("create");
  const [pendingSubscriptionEditorAction, setPendingSubscriptionEditorAction] =
    useState<PendingNotificationSubscriptionEditorAction>(null);
  const [subscriptionDraftBaseline, setSubscriptionDraftBaseline] =
    useState<NotificationSubscriptionDraft>(() =>
      cloneSubscriptionDraft(SUBSCRIPTION_INITIAL_DRAFT),
    );
  const [awaitingSubscriptionBaselineSync, setAwaitingSubscriptionBaselineSync] =
    useState(false);
  const providerCatalogVersionRef = useRef(providerCatalogVersion);
  const subscriptionRows = useMemo(
    () => buildNotificationSubscriptionRows(subscriptions),
    [subscriptions],
  );
  const titleScopedSubscriptionIds = useMemo(
    () =>
      Array.from(
        new Set(
          subscriptions
            .filter((subscription) => subscription.scope === "title" && !!subscription.scopeId)
            .map((subscription) => subscription.scopeId as string),
        ),
      ),
    [subscriptions],
  );
  const isChannelDraftDirty =
    JSON.stringify(channelDraft) !== JSON.stringify(channelDraftBaseline);
  const isSubscriptionDraftDirty =
    JSON.stringify(subscriptionDraft) !== JSON.stringify(subscriptionDraftBaseline);

  useEffect(() => {
    if (!awaitingChannelBaselineSync) {
      return;
    }

    setChannelDraftBaseline(cloneChannelDraft(channelDraft));
    setAwaitingChannelBaselineSync(false);
  }, [awaitingChannelBaselineSync, channelDraft]);

  useEffect(() => {
    if (!awaitingSubscriptionBaselineSync) {
      return;
    }

    setSubscriptionDraftBaseline(cloneSubscriptionDraft(subscriptionDraft));
    setAwaitingSubscriptionBaselineSync(false);
  }, [awaitingSubscriptionBaselineSync, subscriptionDraft]);

  // --- Fetch data ---
  const refreshChannels = useCallback(async () => {
    try {
      const { data, error } = await client
        .query(notificationChannelsQuery, {}, { requestPolicy: "network-only" })
        .toPromise();
      if (error) throw error;
      setChannels(data.notificationChannels || []);
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"), { level: "ERROR" });
    }
  }, [client, setGlobalStatus, t]);

  const refreshSubscriptions = useCallback(async () => {
    try {
      const { data, error } = await client
        .query(notificationSubscriptionsQuery, {}, { requestPolicy: "network-only" })
        .toPromise();
      if (error) throw error;
      setSubscriptions(data.notificationSubscriptions || []);
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"), { level: "ERROR" });
    }
  }, [client, setGlobalStatus, t]);

  const refreshProviderTypes = useCallback(async () => {
    const { data, error } = await client
      .query(notificationProviderTypesQuery, {}, { requestPolicy: "network-only" })
      .toPromise();
    if (error) throw error;
    setProviderTypes(data?.notificationProviderTypes || []);
  }, [client]);

  const refreshNotificationTargets = useCallback(async () => {
    const { data, error } = await client
      .query(notificationTargetsQuery, {}, { requestPolicy: "network-only" })
      .toPromise();
    if (error) throw error;
    setNotificationTargets((data?.notificationTargets ?? []) as NotificationTarget[]);
  }, [client]);

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      try {
        const { data, error } = await client
          .query(notificationsInitQuery, {}, { requestPolicy: "network-only" })
          .toPromise();
        if (error && !data?.notificationChannels && !data?.notificationSubscriptions) throw error;
        if (cancelled) return;
        setChannels(data?.notificationChannels || []);
        setNotificationTargets((data?.notificationTargets ?? []) as NotificationTarget[]);
        setSubscriptions(data?.notificationSubscriptions || []);
        setProviderTypes(data?.notificationProviderTypes || []);
        setEventTypes(data?.notificationEventTypes || []);
        setLocalPathStyle(
          localPathStyleFromRuntimeValue(data?.runtimeInfo?.runtimePathStyle),
        );
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"), { level: "ERROR" });
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
    void Promise.all([refreshProviderTypes(), refreshNotificationTargets()]).catch((error: unknown) => {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"), { level: "ERROR" });
    });
  }, [providerCatalogVersion, refreshNotificationTargets, refreshProviderTypes, setGlobalStatus, t]);

  useEffect(() => {
    setSubscriptionTitlesById((current) => {
      const next: Record<string, TitleRecord | null> = {};
      for (const titleId of titleScopedSubscriptionIds) {
        if (Object.prototype.hasOwnProperty.call(current, titleId)) {
          next[titleId] = current[titleId];
        }
      }
      return next;
    });
  }, [titleScopedSubscriptionIds]);

  useEffect(() => {
    const unresolvedIds = titleScopedSubscriptionIds.filter(
      (titleId) => !Object.prototype.hasOwnProperty.call(subscriptionTitlesById, titleId),
    );

    if (unresolvedIds.length === 0) {
      return;
    }

    let cancelled = false;

    void Promise.all(
      unresolvedIds.map(async (titleId) => {
        const { data, error } = await client
      .query<{ title?: TitleRecord | null }>(titleAutocompleteSelectionQuery, { id: titleId })
          .toPromise();
        if (error) {
          return [titleId, null] as const;
        }
        return [titleId, (data?.title as TitleRecord | null) ?? null] as const;
      }),
    ).then((entries) => {
      if (cancelled) {
        return;
      }

      setSubscriptionTitlesById((current) => {
        const next = { ...current };
        for (const [titleId, title] of entries) {
          next[titleId] = title;
        }
        return next;
      });
    });

    return () => {
      cancelled = true;
    };
  }, [client, subscriptionTitlesById, titleScopedSubscriptionIds]);

  // --- Channel CRUD ---
  const resetChannelDraft = useCallback(() => {
    setEditingChannelId(null);
    setChannelDraft(() => cloneChannelDraft(CHANNEL_INITIAL_DRAFT));
  }, []);

  const submitChannel = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const selectedProvider =
      providerTypes.find((pt) => pt.providerType === channelDraft.channelType.trim()) ?? null;
    const payload = {
      name: channelDraft.name.trim(),
      channelType: channelDraft.channelType.trim(),
      mediaServerConnectionId: undefined,
      config: providerConfigRecordToValuesWithDefaults(
        channelDraft.configValues,
        selectedProvider?.configFields ?? [],
      ),
      isEnabled: channelDraft.isEnabled,
    };

    if (!payload.name || !payload.channelType) {
      setGlobalStatus(t("status.failedToCreate"), { level: "ERROR" });
      return;
    }

    setMutatingChannelId(editingChannelId || "new");
    try {
      if (editingChannelId) {
        const { error } = await client.mutation(updateNotificationChannelMutation, {
          input: {
            id: editingChannelId,
            name: payload.name,
            mediaServerConnectionId: payload.mediaServerConnectionId ?? null,
            config: payload.config,
            isEnabled: payload.isEnabled,
          },
        }).toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.notificationChannelUpdated"), { level: "SUCCESS" });
      } else {
        const { error } = await client.mutation(createNotificationChannelMutation, {
          input: {
            name: payload.name,
            channelType: payload.channelType,
            mediaServerConnectionId: payload.mediaServerConnectionId,
            config: payload.config,
            isEnabled: payload.isEnabled,
          },
        }).toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.notificationChannelCreated"), { level: "SUCCESS" });
      }
      resetChannelDraft();
      setIsChannelEditorOpen(false);
      setChannelEditorMode("create");
      setAwaitingChannelBaselineSync(true);
      await refreshChannels();
      await refreshNotificationTargets();
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"), { level: "ERROR" });
    } finally {
      setMutatingChannelId(null);
    }
  };

  const editChannel = useCallback((channel: NotificationChannel) => {
    setEditingChannelId(channel.id);
    const provider = providerTypes.find((pt) => pt.providerType === channel.channelType);
    setChannelDraft({
      name: provider?.name ?? channel.name,
      channelType: channel.channelType,
      mediaServerConnectionId: channel.mediaServerConnectionId ?? "",
      isEnabled: channel.isEnabled,
      configValues: providerConfigValuesToRecord(channel.config),
    });
    setGlobalStatus(t("status.editingNotificationChannel", { name: channel.name }));
  }, [providerTypes, setGlobalStatus, t]);

  const openCreateChannelEditor = useCallback(() => {
    resetChannelDraft();
    setChannelEditorMode("create");
    setIsChannelEditorOpen(true);
    setAwaitingChannelBaselineSync(true);
  }, [resetChannelDraft]);

  const openEditChannelEditor = useCallback((channel: NotificationChannel) => {
    editChannel(channel);
    setChannelEditorMode("edit");
    setIsChannelEditorOpen(true);
    setAwaitingChannelBaselineSync(true);
  }, [editChannel]);

  const requestCreateChannelEditor = useCallback(() => {
    if (!isChannelEditorOpen || !isChannelDraftDirty) {
      openCreateChannelEditor();
      return;
    }

    setPendingChannelEditorAction({ type: "create" });
  }, [isChannelDraftDirty, isChannelEditorOpen, openCreateChannelEditor]);

  const requestEditChannel = useCallback((channel: NotificationChannel) => {
    if (!isChannelEditorOpen || !isChannelDraftDirty) {
      openEditChannelEditor(channel);
      return;
    }

    setPendingChannelEditorAction({ type: "edit", channel });
  }, [isChannelDraftDirty, isChannelEditorOpen, openEditChannelEditor]);

  const requestCloseChannelEditor = useCallback(() => {
    if (!isChannelEditorOpen) {
      return;
    }

    if (!isChannelDraftDirty) {
      setIsChannelEditorOpen(false);
      setChannelEditorMode("create");
      resetChannelDraft();
      setAwaitingChannelBaselineSync(true);
      return;
    }

    setPendingChannelEditorAction({ type: "close" });
  }, [isChannelDraftDirty, isChannelEditorOpen, resetChannelDraft]);

  const confirmPendingChannelEditorAction = useCallback(() => {
    if (!pendingChannelEditorAction) {
      return;
    }

    if (pendingChannelEditorAction.type === "create") {
      openCreateChannelEditor();
    } else if (pendingChannelEditorAction.type === "edit") {
      openEditChannelEditor(pendingChannelEditorAction.channel);
    } else {
      setIsChannelEditorOpen(false);
      setChannelEditorMode("create");
      resetChannelDraft();
      setAwaitingChannelBaselineSync(true);
    }

    setPendingChannelEditorAction(null);
  }, [
    openCreateChannelEditor,
    openEditChannelEditor,
    pendingChannelEditorAction,
    resetChannelDraft,
  ]);

  const deleteChannel = (channel: NotificationChannel) => {
    setPendingDeleteChannel(channel);
  };

  const confirmDeleteChannel = async () => {
    if (!pendingDeleteChannel) return;
    const channel = pendingDeleteChannel;
    setMutatingChannelId(channel.id);
    try {
      const { error } = await client.mutation(deleteNotificationChannelMutation, {
        id: channel.id,
      }).toPromise();
      if (error) throw error;
      setGlobalStatus(t("status.notificationChannelDeleted", { name: channel.name }), { level: "SUCCESS" });
      await refreshChannels();
      await refreshNotificationTargets();
      await refreshSubscriptions();
      if (editingChannelId === channel.id) {
        resetChannelDraft();
        setIsChannelEditorOpen(false);
        setChannelEditorMode("create");
        setAwaitingChannelBaselineSync(true);
      }
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToDelete"), { level: "ERROR" });
    } finally {
      setMutatingChannelId(null);
      setPendingDeleteChannel(null);
    }
  };

  const toggleChannelEnabled = useCallback(async (channel: NotificationChannel) => {
    const nextIsEnabled = !channel.isEnabled;
    setMutatingChannelId(channel.id);
    setChannels((current) => current.map((existing) => (
      existing.id === channel.id ? { ...existing, isEnabled: nextIsEnabled } : existing
    )));
    if (editingChannelId === channel.id) {
      setChannelDraft((current) => ({
        ...current,
        isEnabled: nextIsEnabled,
      }));
    }
    try {
      const { error } = await client.mutation(updateNotificationChannelMutation, {
        input: {
          id: channel.id,
          isEnabled: nextIsEnabled,
        },
      }).toPromise();
      if (error) throw error;
      setGlobalStatus(t("status.notificationChannelUpdated"), { level: "SUCCESS" });
      await refreshChannels();
      await refreshNotificationTargets();
    } catch (error) {
      setChannels((current) => current.map((existing) => (
        existing.id === channel.id ? { ...existing, isEnabled: channel.isEnabled } : existing
      )));
      if (editingChannelId === channel.id) {
        setChannelDraft((current) => ({
          ...current,
          isEnabled: channel.isEnabled,
        }));
      }
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"), { level: "ERROR" });
    } finally {
      setMutatingChannelId(null);
    }
  }, [client, editingChannelId, refreshChannels, refreshNotificationTargets, setGlobalStatus, t]);

  const testChannel = useCallback(async (channel: NotificationChannel) => {
    setTestingChannelId(channel.id);
    try {
      await runConnectionFeedback({
        setGlobalStatus,
        successMessage: t("settings.notificationTestSuccess"),
        failureFallbackMessage: t("settings.notificationTestFailed"),
        run: async () => {
          const { data, error } = await client.mutation(testNotificationChannelMutation, {
            id: channel.id,
          }).toPromise();
          if (error) throw error;
          const validation = data?.testNotificationChannel;
          if (validation?.status !== "ok") {
            throw new Error(validation?.message ?? t("settings.notificationTestFailed"));
          }
          return validation.message ?? t("settings.notificationTestSuccess");
        },
      });
    } catch {
      // Shared connection feedback already surfaced the failure.
    } finally {
      setTestingChannelId(null);
    }
  }, [client, setGlobalStatus, t]);

  // --- Subscription CRUD ---
  const resetSubscriptionDraft = useCallback(() => {
    setEditingSubscriptionId(null);
    setSubscriptionDraft(() => cloneSubscriptionDraft(SUBSCRIPTION_INITIAL_DRAFT));
  }, []);

  const submitSubscription = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const desiredSpecs = buildNotificationSubscriptionSpecs(subscriptionDraft);

    if (!subscriptionDraft.targetId || desiredSpecs.length === 0) {
      setGlobalStatus(t("status.failedToCreate"), { level: "ERROR" });
      return;
    }

    setMutatingSubscriptionId(editingSubscriptionId || "new");
    try {
      if (editingSubscriptionId) {
        const editingRow = subscriptionRows.find((row) => row.id === editingSubscriptionId);
        if (!editingRow) {
          throw new Error(t("status.failedToUpdate"));
        }

        const existingSubscriptions = subscriptions.filter((subscription) =>
          editingRow.subscriptionIds.includes(subscription.id),
        );
        const existingByKey = new Map(
          existingSubscriptions.map((subscription) => [
            subscriptionSpecKey({
              targetKind: subscription.targetKind,
              targetId: subscription.targetId,
              eventType: subscription.eventType,
              scope: subscription.scope,
              scopeId: subscription.scopeId ?? undefined,
            }),
            subscription,
          ]),
        );
        const desiredByKey = new Map(
          desiredSpecs.map((spec) => [subscriptionSpecKey(spec), spec]),
        );

        for (const spec of desiredSpecs) {
          const key = subscriptionSpecKey(spec);
          const existing = existingByKey.get(key);
          if (!existing) {
            const { error } = await client.mutation(createNotificationSubscriptionMutation, {
              input: {
                channelId: spec.targetKind === "plugin_channel" ? spec.targetId : undefined,
                targetKind: spec.targetKind,
                targetId: spec.targetId,
                eventType: spec.eventType,
                scope: spec.scope,
                scopeId: spec.scopeId,
                isEnabled: spec.isEnabled,
              },
            }).toPromise();
            if (error) throw error;
            continue;
          }

          if (existing.isEnabled !== spec.isEnabled) {
            const { error } = await client.mutation(updateNotificationSubscriptionMutation, {
              input: {
                id: existing.id,
                isEnabled: spec.isEnabled,
              },
            }).toPromise();
            if (error) throw error;
          }
        }

        for (const subscription of existingSubscriptions) {
          const key = subscriptionSpecKey({
            targetKind: subscription.targetKind,
            targetId: subscription.targetId,
            eventType: subscription.eventType,
            scope: subscription.scope,
            scopeId: subscription.scopeId ?? undefined,
          });
          if (desiredByKey.has(key)) {
            continue;
          }

          const { error } = await client.mutation(deleteNotificationSubscriptionMutation, {
            id: subscription.id,
          }).toPromise();
          if (error) throw error;
        }

        setGlobalStatus(t("status.notificationSubscriptionUpdated"), { level: "SUCCESS" });
      } else {
        for (const spec of desiredSpecs) {
          const { error } = await client.mutation(createNotificationSubscriptionMutation, {
            input: {
              channelId: spec.targetKind === "plugin_channel" ? spec.targetId : undefined,
              targetKind: spec.targetKind,
              targetId: spec.targetId,
              eventType: spec.eventType,
              scope: spec.scope,
              scopeId: spec.scopeId,
              isEnabled: spec.isEnabled,
            },
          }).toPromise();
          if (error) throw error;
        }
        setGlobalStatus(t("status.notificationSubscriptionCreated"), { level: "SUCCESS" });
      }
      resetSubscriptionDraft();
      setIsSubscriptionEditorOpen(false);
      setSubscriptionEditorMode("create");
      setAwaitingSubscriptionBaselineSync(true);
      await refreshSubscriptions();
    } catch (error) {
      await refreshSubscriptions().catch(() => undefined);
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"), { level: "ERROR" });
    } finally {
      setMutatingSubscriptionId(null);
    }
  };

  const editSubscription = useCallback((sub: NotificationSubscriptionRow) => {
    setEditingSubscriptionId(sub.id);
    setSubscriptionDraft({
      targetKind: sub.targetKind,
      targetId: sub.targetId,
      eventTypes: [...sub.eventTypes],
      scope: sub.scope,
      facetScopeIds: sub.scope === "facet" ? parseFacetScopeIds(sub.scopeId) : [],
      titleScopeId: sub.scope === "title" ? sub.scopeId || "" : "",
      titleScopeTitle:
        sub.scope === "title" && sub.scopeId
          ? subscriptionTitlesById[sub.scopeId] ?? null
          : null,
      isEnabled: sub.isEnabled,
    });
    setGlobalStatus(t("status.editingNotificationSubscription"));
  }, [setGlobalStatus, subscriptionTitlesById, t]);

  const openCreateSubscriptionEditor = useCallback((target?: Pick<NotificationTarget, "targetKind" | "id">) => {
    setSubscriptionDraft({
      ...cloneSubscriptionDraft(SUBSCRIPTION_INITIAL_DRAFT),
      targetKind: target?.targetKind ?? SUBSCRIPTION_INITIAL_DRAFT.targetKind,
      targetId: target?.id ?? SUBSCRIPTION_INITIAL_DRAFT.targetId,
    });
    setSubscriptionEditorMode("create");
    setIsSubscriptionEditorOpen(true);
    setAwaitingSubscriptionBaselineSync(true);
  }, []);

  const openEditSubscriptionEditor = useCallback((sub: NotificationSubscriptionRow) => {
    editSubscription(sub);
    setSubscriptionEditorMode("edit");
    setIsSubscriptionEditorOpen(true);
    setAwaitingSubscriptionBaselineSync(true);
  }, [editSubscription]);

  const requestCreateSubscriptionEditor = useCallback((target?: Pick<NotificationTarget, "targetKind" | "id">) => {
    if (!isSubscriptionEditorOpen || !isSubscriptionDraftDirty) {
      openCreateSubscriptionEditor(target);
      return;
    }

    setPendingSubscriptionEditorAction({ type: "create", target });
  }, [
    isSubscriptionDraftDirty,
    isSubscriptionEditorOpen,
    openCreateSubscriptionEditor,
  ]);

  const requestEditSubscription = useCallback((sub: NotificationSubscriptionRow) => {
    if (!isSubscriptionEditorOpen || !isSubscriptionDraftDirty) {
      openEditSubscriptionEditor(sub);
      return;
    }

    setPendingSubscriptionEditorAction({ type: "edit", subscription: sub });
  }, [
    isSubscriptionDraftDirty,
    isSubscriptionEditorOpen,
    openEditSubscriptionEditor,
  ]);

  const requestCloseSubscriptionEditor = useCallback(() => {
    if (!isSubscriptionEditorOpen) {
      return;
    }

    if (!isSubscriptionDraftDirty) {
      setIsSubscriptionEditorOpen(false);
      setSubscriptionEditorMode("create");
      resetSubscriptionDraft();
      setAwaitingSubscriptionBaselineSync(true);
      return;
    }

    setPendingSubscriptionEditorAction({ type: "close" });
  }, [isSubscriptionDraftDirty, isSubscriptionEditorOpen, resetSubscriptionDraft]);

  const confirmPendingSubscriptionEditorAction = useCallback(() => {
    if (!pendingSubscriptionEditorAction) {
      return;
    }

    if (pendingSubscriptionEditorAction.type === "create") {
      openCreateSubscriptionEditor(pendingSubscriptionEditorAction.target);
    } else if (pendingSubscriptionEditorAction.type === "edit") {
      openEditSubscriptionEditor(pendingSubscriptionEditorAction.subscription);
    } else {
      setIsSubscriptionEditorOpen(false);
      setSubscriptionEditorMode("create");
      resetSubscriptionDraft();
      setAwaitingSubscriptionBaselineSync(true);
    }

    setPendingSubscriptionEditorAction(null);
  }, [
    openCreateSubscriptionEditor,
    openEditSubscriptionEditor,
    pendingSubscriptionEditorAction,
    resetSubscriptionDraft,
  ]);

  const deleteSubscription = (sub: NotificationSubscriptionRow) => {
    setPendingDeleteSubscription(sub);
  };

  const confirmDeleteSubscription = async () => {
    if (!pendingDeleteSubscription) return;
    const sub = pendingDeleteSubscription;
    setMutatingSubscriptionId(sub.id);
    try {
      for (const subscriptionId of sub.subscriptionIds) {
        const { error } = await client.mutation(deleteNotificationSubscriptionMutation, {
          id: subscriptionId,
        }).toPromise();
        if (error) throw error;
      }
      setGlobalStatus(t("status.notificationSubscriptionDeleted"), { level: "SUCCESS" });
      await refreshSubscriptions();
      if (editingSubscriptionId === sub.id) {
        resetSubscriptionDraft();
        setIsSubscriptionEditorOpen(false);
        setSubscriptionEditorMode("create");
        setAwaitingSubscriptionBaselineSync(true);
      }
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToDelete"), { level: "ERROR" });
    } finally {
      setMutatingSubscriptionId(null);
      setPendingDeleteSubscription(null);
    }
  };

  const toggleSubscriptionEnabled = useCallback(async (sub: NotificationSubscriptionRow) => {
    setMutatingSubscriptionId(sub.id);
    try {
      for (const subscriptionId of sub.subscriptionIds) {
        const { error } = await client.mutation(updateNotificationSubscriptionMutation, {
          input: {
            id: subscriptionId,
            isEnabled: !sub.isEnabled,
          },
        }).toPromise();
        if (error) throw error;
      }
      setGlobalStatus(t("status.notificationSubscriptionUpdated"), { level: "SUCCESS" });
      await refreshSubscriptions();
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"), { level: "ERROR" });
    } finally {
      setMutatingSubscriptionId(null);
    }
  }, [client, refreshSubscriptions, setGlobalStatus, t]);

  return (
    <>
      {pluginsTarget
        ? createPortal(
            <FilteredPluginList
              family="NOTIFICATION"
              refreshProviderOptions={refreshProviderTypes}
            />,
            pluginsTarget,
          )
        : null}
      <SettingsNotificationsSection
        channels={channels}
        localPathStyle={localPathStyle}
        editingChannelId={editingChannelId}
        channelDraft={channelDraft}
        setChannelDraft={setChannelDraft}
        submitChannel={submitChannel}
        mutatingChannelId={mutatingChannelId}
        resetChannelDraft={requestCloseChannelEditor}
        editChannel={requestEditChannel}
        toggleChannelEnabled={toggleChannelEnabled}
        deleteChannel={deleteChannel}
        testChannel={testChannel}
        testingChannelId={testingChannelId}
        isChannelEditorOpen={isChannelEditorOpen}
        channelEditorMode={channelEditorMode}
        startCreateChannel={requestCreateChannelEditor}
        providerTypes={providerTypes}
        notificationTargets={notificationTargets}
        subscriptions={subscriptionRows}
        subscriptionTitlesById={subscriptionTitlesById}
        editingSubscriptionId={editingSubscriptionId}
        subscriptionDraft={subscriptionDraft}
        setSubscriptionDraft={setSubscriptionDraft}
        submitSubscription={submitSubscription}
        mutatingSubscriptionId={mutatingSubscriptionId}
        resetSubscriptionDraft={requestCloseSubscriptionEditor}
        editSubscription={requestEditSubscription}
        toggleSubscriptionEnabled={toggleSubscriptionEnabled}
        deleteSubscription={deleteSubscription}
        eventTypes={eventTypes}
        isSubscriptionEditorOpen={isSubscriptionEditorOpen}
        subscriptionEditorMode={subscriptionEditorMode}
        startCreateSubscription={requestCreateSubscriptionEditor}
      />
      <ConfirmDialog
        open={pendingChannelEditorAction !== null}
        title={t("settings.notificationChannelConfirmDiscardTitle")}
        description={t("settings.notificationChannelConfirmDiscardDescription")}
        confirmLabel={
          pendingChannelEditorAction?.type === "create"
            ? t("settings.notificationChannelCreateNew")
            : pendingChannelEditorAction?.type === "edit"
              ? t("label.edit")
              : t("label.discard")
        }
        cancelLabel={t("label.cancel")}
        isBusy={mutatingChannelId !== null}
        onConfirm={confirmPendingChannelEditorAction}
        onCancel={() => setPendingChannelEditorAction(null)}
      />
      <ConfirmDialog
        open={pendingDeleteChannel !== null}
        title={t("label.delete")}
        description={
          pendingDeleteChannel ? t("status.deletingNotificationChannel", { name: pendingDeleteChannel.name }) : ""
        }
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        isBusy={mutatingChannelId !== null}
        onConfirm={confirmDeleteChannel}
        onCancel={() => setPendingDeleteChannel(null)}
      />
      <ConfirmDialog
        open={pendingSubscriptionEditorAction !== null}
        title={t("settings.notificationSubscriptionConfirmDiscardTitle")}
        description={t("settings.notificationSubscriptionConfirmDiscardDescription")}
        confirmLabel={
          pendingSubscriptionEditorAction?.type === "create"
            ? t("settings.notificationSubscriptionCreateNew")
            : pendingSubscriptionEditorAction?.type === "edit"
              ? t("label.edit")
              : t("label.discard")
        }
        cancelLabel={t("label.cancel")}
        isBusy={mutatingSubscriptionId !== null}
        onConfirm={confirmPendingSubscriptionEditorAction}
        onCancel={() => setPendingSubscriptionEditorAction(null)}
      />
      <ConfirmDialog
        open={pendingDeleteSubscription !== null}
        title={t("label.delete")}
        description={t("status.deletingNotificationSubscription")}
        confirmLabel={t("label.delete")}
        cancelLabel={t("label.cancel")}
        isBusy={mutatingSubscriptionId !== null}
        onConfirm={confirmDeleteSubscription}
        onCancel={() => setPendingDeleteSubscription(null)}
      />
    </>
  );
}


import { type FormEvent, useCallback, useEffect, useState } from "react";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { SettingsNotificationsSection } from "@/components/views/settings/settings-notifications-section";
import { useClient } from "urql";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import type {
  NotificationChannel,
  NotificationChannelDraft,
  NotificationProviderType,
  NotificationSubscription,
  NotificationSubscriptionDraft,
} from "@/lib/types";
import {
  notificationChannelsQuery,
  notificationSubscriptionsQuery,
  notificationsInitQuery,
} from "@/lib/graphql/queries";
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
  isEnabled: true,
  configValues: {},
};

const SUBSCRIPTION_INITIAL_DRAFT: NotificationSubscriptionDraft = {
  channelId: "",
  eventType: "",
  scope: "global",
  scopeId: "",
  isEnabled: true,
};

function serializeConfigJson(configValues: Record<string, string>): string | undefined {
  const nonEmpty = Object.fromEntries(
    Object.entries(configValues).filter(([, v]) => v !== ""),
  );
  return Object.keys(nonEmpty).length > 0 ? JSON.stringify(nonEmpty) : undefined;
}

function parseConfigJson(configJson: string | null): Record<string, string> {
  if (!configJson) return {};
  try {
    return JSON.parse(configJson) as Record<string, string>;
  } catch {
    return {};
  }
}

export function SettingsNotificationsContainer() {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();

  // --- Channel state ---
  const [channels, setChannels] = useState<NotificationChannel[]>([]);
  const [editingChannelId, setEditingChannelId] = useState<string | null>(null);
  const [mutatingChannelId, setMutatingChannelId] = useState<string | null>(null);
  const [pendingDeleteChannel, setPendingDeleteChannel] = useState<NotificationChannel | null>(null);
  const [channelDraft, setChannelDraft] = useState<NotificationChannelDraft>(() => ({ ...CHANNEL_INITIAL_DRAFT }));
  const [providerTypes, setProviderTypes] = useState<NotificationProviderType[]>([]);
  const [testingChannelId, setTestingChannelId] = useState<string | null>(null);

  // --- Subscription state ---
  const [subscriptions, setSubscriptions] = useState<NotificationSubscription[]>([]);
  const [editingSubscriptionId, setEditingSubscriptionId] = useState<string | null>(null);
  const [mutatingSubscriptionId, setMutatingSubscriptionId] = useState<string | null>(null);
  const [pendingDeleteSubscription, setPendingDeleteSubscription] = useState<NotificationSubscription | null>(null);
  const [subscriptionDraft, setSubscriptionDraft] = useState<NotificationSubscriptionDraft>(() => ({ ...SUBSCRIPTION_INITIAL_DRAFT }));
  const [eventTypes, setEventTypes] = useState<string[]>([]);

  // --- Fetch data ---
  const refreshChannels = useCallback(async () => {
    try {
      const { data, error } = await client.query(notificationChannelsQuery, {}).toPromise();
      if (error) throw error;
      setChannels(data.notificationChannels || []);
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
    }
  }, [client, setGlobalStatus, t]);

  const refreshSubscriptions = useCallback(async () => {
    try {
      const { data, error } = await client.query(notificationSubscriptionsQuery, {}).toPromise();
      if (error) throw error;
      setSubscriptions(data.notificationSubscriptions || []);
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
    }
  }, [client, setGlobalStatus, t]);

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      try {
        const { data, error } = await client.query(notificationsInitQuery, {}).toPromise();
        if (error && !data?.notificationChannels && !data?.notificationSubscriptions) throw error;
        if (cancelled) return;
        setChannels(data?.notificationChannels || []);
        setSubscriptions(data?.notificationSubscriptions || []);
        setProviderTypes(data?.notificationProviderTypes || []);
        setEventTypes(data?.notificationEventTypes || []);
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
      }
    };
    void load();
    return () => {
      cancelled = true;
    };
  }, [client, setGlobalStatus, t]);

  // --- Channel CRUD ---
  const resetChannelDraft = useCallback(() => {
    setEditingChannelId(null);
    setChannelDraft(() => ({ ...CHANNEL_INITIAL_DRAFT }));
  }, []);

  const submitChannel = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const payload = {
      name: channelDraft.name.trim(),
      channelType: channelDraft.channelType.trim(),
      configJson: serializeConfigJson(channelDraft.configValues),
      isEnabled: channelDraft.isEnabled,
    };

    if (!payload.name || !payload.channelType) {
      setGlobalStatus(t("status.failedToCreate"));
      return;
    }

    setMutatingChannelId(editingChannelId || "new");
    try {
      if (editingChannelId) {
        const { error } = await client.mutation(updateNotificationChannelMutation, {
          input: {
            id: editingChannelId,
            name: payload.name,
            configJson: payload.configJson,
            isEnabled: payload.isEnabled,
          },
        }).toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.notificationChannelUpdated"));
      } else {
        const { error } = await client.mutation(createNotificationChannelMutation, {
          input: {
            name: payload.name,
            channelType: payload.channelType,
            configJson: payload.configJson,
            isEnabled: payload.isEnabled,
          },
        }).toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.notificationChannelCreated"));
      }
      resetChannelDraft();
      await refreshChannels();
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setMutatingChannelId(null);
    }
  };

  const editChannel = (channel: NotificationChannel) => {
    setEditingChannelId(channel.id);
    setChannelDraft({
      name: channel.name,
      channelType: channel.channelType,
      isEnabled: channel.isEnabled,
      configValues: parseConfigJson(channel.configJson),
    });
    setGlobalStatus(t("status.editingNotificationChannel", { name: channel.name }));
  };

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
      setGlobalStatus(t("status.notificationChannelDeleted", { name: channel.name }));
      await refreshChannels();
      await refreshSubscriptions();
      if (editingChannelId === channel.id) {
        resetChannelDraft();
      }
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToDelete"));
    } finally {
      setMutatingChannelId(null);
      setPendingDeleteChannel(null);
    }
  };

  const toggleChannelEnabled = useCallback(async (channel: NotificationChannel) => {
    setMutatingChannelId(channel.id);
    try {
      const { error } = await client.mutation(updateNotificationChannelMutation, {
        input: {
          id: channel.id,
          isEnabled: !channel.isEnabled,
        },
      }).toPromise();
      if (error) throw error;
      setGlobalStatus(t("status.notificationChannelUpdated"));
      await refreshChannels();
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setMutatingChannelId(null);
    }
  }, [client, refreshChannels, setGlobalStatus, t]);

  const testChannel = useCallback(async (channel: NotificationChannel) => {
    setTestingChannelId(channel.id);
    try {
      const { data, error } = await client.mutation(testNotificationChannelMutation, {
        id: channel.id,
      }).toPromise();
      if (error) throw error;
      setGlobalStatus(
        data?.testNotificationChannel
          ? t("settings.notificationTestSuccess")
          : t("settings.notificationTestFailed"),
      );
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("settings.notificationTestFailed"));
    } finally {
      setTestingChannelId(null);
    }
  }, [client, setGlobalStatus, t]);

  // --- Subscription CRUD ---
  const resetSubscriptionDraft = useCallback(() => {
    setEditingSubscriptionId(null);
    setSubscriptionDraft(() => ({ ...SUBSCRIPTION_INITIAL_DRAFT }));
  }, []);

  const submitSubscription = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const payload = {
      channelId: subscriptionDraft.channelId,
      eventType: subscriptionDraft.eventType,
      scope: subscriptionDraft.scope,
      scopeId: subscriptionDraft.scopeId || undefined,
      isEnabled: subscriptionDraft.isEnabled,
    };

    if (!payload.channelId || !payload.eventType) {
      setGlobalStatus(t("status.failedToCreate"));
      return;
    }

    setMutatingSubscriptionId(editingSubscriptionId || "new");
    try {
      if (editingSubscriptionId) {
        const { error } = await client.mutation(updateNotificationSubscriptionMutation, {
          input: {
            id: editingSubscriptionId,
            eventType: payload.eventType,
            scope: payload.scope,
            scopeId: payload.scopeId,
            isEnabled: payload.isEnabled,
          },
        }).toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.notificationSubscriptionUpdated"));
      } else {
        const { error } = await client.mutation(createNotificationSubscriptionMutation, {
          input: {
            channelId: payload.channelId,
            eventType: payload.eventType,
            scope: payload.scope,
            scopeId: payload.scopeId,
            isEnabled: payload.isEnabled,
          },
        }).toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.notificationSubscriptionCreated"));
      }
      resetSubscriptionDraft();
      await refreshSubscriptions();
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setMutatingSubscriptionId(null);
    }
  };

  const editSubscription = (sub: NotificationSubscription) => {
    setEditingSubscriptionId(sub.id);
    setSubscriptionDraft({
      channelId: sub.channelId,
      eventType: sub.eventType,
      scope: sub.scope,
      scopeId: sub.scopeId || "",
      isEnabled: sub.isEnabled,
    });
    setGlobalStatus(t("status.editingNotificationSubscription"));
  };

  const deleteSubscription = (sub: NotificationSubscription) => {
    setPendingDeleteSubscription(sub);
  };

  const confirmDeleteSubscription = async () => {
    if (!pendingDeleteSubscription) return;
    const sub = pendingDeleteSubscription;
    setMutatingSubscriptionId(sub.id);
    try {
      const { error } = await client.mutation(deleteNotificationSubscriptionMutation, {
        id: sub.id,
      }).toPromise();
      if (error) throw error;
      setGlobalStatus(t("status.notificationSubscriptionDeleted"));
      await refreshSubscriptions();
      if (editingSubscriptionId === sub.id) {
        resetSubscriptionDraft();
      }
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToDelete"));
    } finally {
      setMutatingSubscriptionId(null);
      setPendingDeleteSubscription(null);
    }
  };

  const toggleSubscriptionEnabled = useCallback(async (sub: NotificationSubscription) => {
    setMutatingSubscriptionId(sub.id);
    try {
      const { error } = await client.mutation(updateNotificationSubscriptionMutation, {
        input: {
          id: sub.id,
          isEnabled: !sub.isEnabled,
        },
      }).toPromise();
      if (error) throw error;
      setGlobalStatus(t("status.notificationSubscriptionUpdated"));
      await refreshSubscriptions();
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.failedToUpdate"));
    } finally {
      setMutatingSubscriptionId(null);
    }
  }, [client, refreshSubscriptions, setGlobalStatus, t]);

  return (
    <>
      <SettingsNotificationsSection
        channels={channels}
        editingChannelId={editingChannelId}
        channelDraft={channelDraft}
        setChannelDraft={setChannelDraft}
        submitChannel={submitChannel}
        mutatingChannelId={mutatingChannelId}
        resetChannelDraft={resetChannelDraft}
        editChannel={editChannel}
        toggleChannelEnabled={toggleChannelEnabled}
        deleteChannel={deleteChannel}
        testChannel={testChannel}
        testingChannelId={testingChannelId}
        providerTypes={providerTypes}
        subscriptions={subscriptions}
        editingSubscriptionId={editingSubscriptionId}
        subscriptionDraft={subscriptionDraft}
        setSubscriptionDraft={setSubscriptionDraft}
        submitSubscription={submitSubscription}
        mutatingSubscriptionId={mutatingSubscriptionId}
        resetSubscriptionDraft={resetSubscriptionDraft}
        editSubscription={editSubscription}
        toggleSubscriptionEnabled={toggleSubscriptionEnabled}
        deleteSubscription={deleteSubscription}
        eventTypes={eventTypes}
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

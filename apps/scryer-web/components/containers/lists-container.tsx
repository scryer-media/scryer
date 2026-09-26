import * as React from "react";
import { useLocation, useNavigate } from "react-router";
import { useClient } from "urql";

import type { ListsSection } from "@/components/root/types";
import {
  ListsView,
  type ListDetailState,
  type ListRouteOptions,
} from "@/components/views/lists/lists-view";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import {
  addListExclusionMutation,
  removeListExclusionMutation,
  setListSubscriptionEnabledMutation,
  subscribeListMutation,
  syncAllListsMutation,
  syncListSubscriptionMutation,
  unsubscribeListMutation,
  updateListProviderSettingsMutation,
  updateListSubscriptionMutation,
} from "@/lib/graphql/mutations";
import {
  listExclusionsQuery,
  listProviderSettingsQuery,
  listProvidersQuery,
  listRouteOptionsQuery,
  listSourcePreviewQuery,
  listSubscriptionDetailQuery,
  listSubscriptionPreviewQuery,
  listSubscriptionsQuery,
  listUrlPreviewQuery,
} from "@/lib/graphql/queries";
import type {
  ListExclusion,
  ListMembershipPage,
  ListPreview,
  ListProviderManifest,
  ListProviderSettingChange,
  ListProviderSettings,
  ListSourceDraft,
  ListSubscription,
  ListSubscriptionDraft,
  ListSyncRun,
} from "@/lib/types/lists";
import type { LibraryRecord } from "@/lib/types/titles";
import {
  draftToSubscribeInput,
  draftToUpdateInput,
  listSyncPollDelayMs,
  listSyncWatchSettled,
  type AddListExclusionInput,
  type ListSyncWatchSnapshot,
} from "@/lib/utils/lists";
import { buildListsPath, listsSectionFromPath } from "@/lib/utils/routing";

const MEMBERSHIP_PAGE_SIZE = 50;
const SYNC_RUN_LIMIT = 20;

type ListsContainerProps = {
  canManageLists: boolean;
};

export function ListsContainer({ canManageLists }: ListsContainerProps) {
  const client = useClient();
  const t = useTranslate();
  const setGlobalStatus = useGlobalStatus();
  const location = useLocation();
  const navigate = useNavigate();
  const requestedSection = listsSectionFromPath(location.pathname);
  const section: ListsSection = requestedSection;

  const [providers, setProviders] = React.useState<ListProviderManifest[]>([]);
  const [providerSettings, setProviderSettings] = React.useState<ListProviderSettings[] | null>(null);
  const [subscriptions, setSubscriptions] = React.useState<ListSubscription[]>([]);
  const [exclusions, setExclusions] = React.useState<ListExclusion[]>([]);
  const [routeOptions, setRouteOptions] = React.useState<ListRouteOptions>({
    libraries: [],
    qualityProfiles: [],
  });
  const [loading, setLoading] = React.useState(true);
  const [loadError, setLoadError] = React.useState<string | null>(null);
  const [exclusionsLoading, setExclusionsLoading] = React.useState(false);
  const [busyIds, setBusyIds] = React.useState<ReadonlySet<string>>(new Set());
  const [detail, setDetail] = React.useState<ListDetailState | null>(null);

  const markBusy = React.useCallback((id: string, busy: boolean) => {
    setBusyIds((current) => {
      const next = new Set(current);
      if (busy) next.add(id);
      else next.delete(id);
      return next;
    });
  }, []);

  const loadSubscriptions = React.useCallback(async (): Promise<ListSubscription[]> => {
    const result = await client
      .query(listSubscriptionsQuery, {}, { requestPolicy: "network-only" })
      .toPromise();
    if (result.error) {
      throw result.error;
    }
    const loaded = (result.data?.listSubscriptions ?? []) as ListSubscription[];
    setSubscriptions(loaded);
    return loaded;
  }, [client]);

  const loadPage = React.useCallback(async () => {
    setLoading(true);
    setLoadError(null);
    try {
      const [providersResult] = await Promise.all([
        client.query(listProvidersQuery, {}).toPromise(),
        loadSubscriptions(),
      ]);
      if (providersResult.error) throw providersResult.error;
      setProviders((providersResult.data?.listProviders ?? []) as ListProviderManifest[]);
      if (canManageLists) {
        const optionsResult = await client.query(listRouteOptionsQuery, {}).toPromise();
        if (optionsResult.error) throw optionsResult.error;
        setRouteOptions({
          libraries: (optionsResult.data?.libraries ?? []) as LibraryRecord[],
          qualityProfiles: (optionsResult.data?.qualityProfileSettings?.profiles ?? []) as Array<{
            id: string;
            name: string;
          }>,
        });
      }
    } catch (error) {
      setLoadError(userFacingGraphQlErrorMessage(error, t("status.failedToLoad")));
    } finally {
      setLoading(false);
    }
  }, [canManageLists, client, loadSubscriptions, t]);

  const loadExclusions = React.useCallback(async () => {
    if (!canManageLists) return;
    setExclusionsLoading(true);
    try {
      const result = await client
        .query(listExclusionsQuery, {}, { requestPolicy: "network-only" })
        .toPromise();
      if (result.error) throw result.error;
      setExclusions((result.data?.listExclusions ?? []) as ListExclusion[]);
    } catch (error) {
      setGlobalStatus(userFacingGraphQlErrorMessage(error, t("status.failedToLoad")));
    } finally {
      setExclusionsLoading(false);
    }
  }, [canManageLists, client, setGlobalStatus, t]);

  const loadProviderSettings = React.useCallback(async () => {
    if (!canManageLists) return;
    try {
      const result = await client
        .query(listProviderSettingsQuery, {}, { requestPolicy: "network-only" })
        .toPromise();
      if (result.error) throw result.error;
      setProviderSettings((result.data?.listProviderSettings ?? []) as ListProviderSettings[]);
    } catch (error) {
      setGlobalStatus(userFacingGraphQlErrorMessage(error, t("status.failedToLoad")));
    }
  }, [canManageLists, client, setGlobalStatus, t]);

  React.useEffect(() => {
    void loadPage();
  }, [loadPage]);

  React.useEffect(() => {
    void loadProviderSettings();
  }, [loadProviderSettings]);

  const saveProviderSettings = React.useCallback(
    async (provider: string, changes: ListProviderSettingChange[]): Promise<boolean> => {
      try {
        const result = await client
          .mutation(updateListProviderSettingsMutation, { provider, changes })
          .toPromise();
        if (result.error) throw result.error;
        const saved = result.data?.updateListProviderSettings as ListProviderSettings | undefined;
        if (saved) {
          setProviderSettings((current) => [
            ...(current ?? []).filter((entry) => entry.providerType !== saved.providerType),
            saved,
          ]);
        }
        setGlobalStatus(t("lists.providerSettings.saved"));
        return true;
      } catch (error) {
        setGlobalStatus(userFacingGraphQlErrorMessage(error, t("status.failedToUpdate")));
        return false;
      }
    },
    [client, setGlobalStatus, t],
  );

  React.useEffect(() => {
    if (section === "exclusions") {
      void loadExclusions();
    }
  }, [loadExclusions, section]);

  const refreshAfterChange = React.useCallback(async () => {
    try {
      await loadSubscriptions();
    } catch (error) {
      setGlobalStatus(userFacingGraphQlErrorMessage(error, t("status.failedToLoad")));
    }
  }, [loadSubscriptions, setGlobalStatus, t]);

  // Only the newest panel load may land; a background refresh also yields to
  // any panel load (opening, paging) started while it was in flight.
  const detailRequestRef = React.useRef(0);
  const detailLoadingRef = React.useRef(false);

  const loadDetail = React.useCallback(
    async (
      id: string,
      membershipOffset = 0,
      { background = false }: { background?: boolean } = {},
    ): Promise<ListSyncRun[] | null> => {
      // A background refresh only updates a panel that is still showing this
      // list; it never opens the panel or shows it as loading.
      const request = background ? detailRequestRef.current : ++detailRequestRef.current;
      const isCurrent = () =>
        request === detailRequestRef.current && (!background || !detailLoadingRef.current);
      if (!background) {
        detailLoadingRef.current = true;
        setDetail((current) =>
          current?.id === id
            ? { ...current, loading: true }
            : { id, loading: true, subscription: null, memberships: null, runs: [], membershipOffset: 0, error: null },
        );
      }
      try {
        const result = await client
          .query(
            listSubscriptionDetailQuery,
            {
              id,
              membershipLimit: MEMBERSHIP_PAGE_SIZE,
              membershipOffset,
              runLimit: SYNC_RUN_LIMIT,
            },
            { requestPolicy: "network-only" },
          )
          .toPromise();
        if (result.error) throw result.error;
        const runs = (result.data?.listSyncRuns ?? []) as ListSyncRun[];
        if (!isCurrent()) return runs;
        if (!background) detailLoadingRef.current = false;
        setDetail((current) =>
          current?.id !== id
            ? current
            : {
                id,
                loading: false,
                subscription: (result.data?.listSubscription ?? null) as ListSubscription | null,
                memberships: (result.data?.listSubscriptionMemberships ?? null) as ListMembershipPage | null,
                runs,
                membershipOffset,
                error: null,
              },
        );
        return runs;
      } catch (error) {
        if (background || !isCurrent()) return null;
        detailLoadingRef.current = false;
        const message = userFacingGraphQlErrorMessage(error, t("status.failedToLoad"));
        setDetail((current) => (current?.id !== id ? current : { ...current, loading: false, error: message }));
        return null;
      }
    },
    [client, t],
  );

  const openDetail = React.useCallback(
    (id: string | null) => {
      if (!id) {
        setDetail(null);
        return;
      }
      void loadDetail(id);
    },
    [loadDetail],
  );

  const runPreview = React.useCallback(
    async (query: string, variables: Record<string, unknown>, field: string): Promise<ListPreview | null> => {
      try {
        const result = await client
          .query(query, variables, { requestPolicy: "network-only" })
          .toPromise();
        if (result.error) throw result.error;
        return (result.data?.[field] ?? null) as ListPreview | null;
      } catch (error) {
        setGlobalStatus(userFacingGraphQlErrorMessage(error, t("lists.preview.failed")));
        return null;
      }
    },
    [client, setGlobalStatus, t],
  );

  const previewUrl = React.useCallback(
    (url: string) => runPreview(listUrlPreviewQuery, { url }, "listUrlPreview"),
    [runPreview],
  );

  const previewSource = React.useCallback(
    (source: ListSourceDraft) =>
      runPreview(
        listSourcePreviewQuery,
        {
          input: {
            provider: source.provider,
            sourceType: source.sourceType,
            params: source.params.filter((param) => param.value.trim()),
            url: source.url,
          },
        },
        "listSourcePreview",
      ),
    [runPreview],
  );

  const previewSubscription = React.useCallback(
    (id: string) => runPreview(listSubscriptionPreviewQuery, { id }, "listSubscriptionPreview"),
    [runPreview],
  );

  const subscribe = React.useCallback(
    async (source: ListSourceDraft, draft: ListSubscriptionDraft): Promise<boolean> => {
      try {
        const result = await client
          .mutation(subscribeListMutation, { input: draftToSubscribeInput(source, draft) })
          .toPromise();
        if (result.error) throw result.error;
        setGlobalStatus(t("lists.status.followed", { name: draft.name.trim() }));
        await refreshAfterChange();
        return true;
      } catch (error) {
        setGlobalStatus(userFacingGraphQlErrorMessage(error, t("status.failedToUpdate")));
        return false;
      }
    },
    [client, refreshAfterChange, setGlobalStatus, t],
  );

  const update = React.useCallback(
    async (id: string, draft: ListSubscriptionDraft): Promise<boolean> => {
      markBusy(id, true);
      try {
        const result = await client
          .mutation(updateListSubscriptionMutation, { id, input: draftToUpdateInput(draft) })
          .toPromise();
        if (result.error) throw result.error;
        setGlobalStatus(t("lists.status.updated", { name: draft.name.trim() }));
        await refreshAfterChange();
        if (detail?.id === id) void loadDetail(id, detail.membershipOffset);
        return true;
      } catch (error) {
        setGlobalStatus(userFacingGraphQlErrorMessage(error, t("status.failedToUpdate")));
        return false;
      } finally {
        markBusy(id, false);
      }
    },
    [client, detail, loadDetail, markBusy, refreshAfterChange, setGlobalStatus, t],
  );

  const setEnabled = React.useCallback(
    async (subscription: ListSubscription, enabled: boolean) => {
      markBusy(subscription.id, true);
      setSubscriptions((current) =>
        current.map((entry) => (entry.id === subscription.id ? { ...entry, enabled } : entry)),
      );
      try {
        const result = await client
          .mutation(setListSubscriptionEnabledMutation, { id: subscription.id, enabled })
          .toPromise();
        if (result.error) throw result.error;
      } catch (error) {
        setGlobalStatus(userFacingGraphQlErrorMessage(error, t("status.failedToUpdate")));
      } finally {
        markBusy(subscription.id, false);
        await refreshAfterChange();
        if (detail?.id === subscription.id) void loadDetail(subscription.id, detail.membershipOffset);
      }
    },
    [client, detail, loadDetail, markBusy, refreshAfterChange, setGlobalStatus, t],
  );

  // "Sync now" only queues a sync, so the table and an open detail panel keep
  // refreshing, backing off, until the sync visibly finishes or the budget runs
  // out. Detail refreshes stop as soon as the panel closes or shows another list.
  const detailRef = React.useRef(detail);
  React.useEffect(() => {
    detailRef.current = detail;
  }, [detail]);
  const syncWatchTimersRef = React.useRef(new Map<string, ReturnType<typeof setTimeout>>());

  React.useEffect(() => {
    const timers = syncWatchTimersRef.current;
    return () => {
      for (const timer of timers.values()) clearTimeout(timer);
      timers.clear();
    };
  }, []);

  const watchSync = React.useCallback(
    (subscription: ListSubscription) => {
      const id = subscription.id;
      const timers = syncWatchTimersRef.current;
      const existing = timers.get(id);
      if (existing !== undefined) clearTimeout(existing);

      const openDetail = detailRef.current;
      const baseline: ListSyncWatchSnapshot = {
        lastAt: subscription.sync.lastAt,
        state: subscription.sync.state,
        runIds: openDetail?.id === id && openDetail.subscription ? openDetail.runs.map((run) => run.id) : null,
      };
      const startedAt = Date.now();
      let attempt = 0;

      const schedule = () => {
        const delay = listSyncPollDelayMs(attempt, Date.now() - startedAt);
        attempt += 1;
        if (delay === null) {
          timers.delete(id);
          return;
        }
        timers.set(id, setTimeout(() => void tick(), delay));
      };

      const tick = async () => {
        let current: ListSubscription | undefined;
        try {
          current = (await loadSubscriptions()).find((entry) => entry.id === id);
        } catch {
          // A failed refresh is retried on the next tick.
        }
        if (!timers.has(id)) return;
        if (!current) {
          timers.delete(id);
          return;
        }
        const shown = detailRef.current;
        const runs =
          shown?.id === id ? await loadDetail(id, shown.membershipOffset, { background: true }) : null;
        if (!timers.has(id)) return;
        const settled = listSyncWatchSettled(baseline, {
          lastAt: current.sync.lastAt,
          state: current.sync.state,
          runIds: runs ? runs.map((run) => run.id) : null,
        });
        if (settled) {
          timers.delete(id);
          return;
        }
        schedule();
      };

      schedule();
    },
    [loadDetail, loadSubscriptions],
  );

  const syncNow = React.useCallback(
    async (subscription: ListSubscription) => {
      markBusy(subscription.id, true);
      try {
        const result = await client
          .mutation(syncListSubscriptionMutation, { id: subscription.id })
          .toPromise();
        if (result.error) throw result.error;
        setGlobalStatus(t("lists.status.syncQueued", { name: subscription.name }));
        watchSync(subscription);
      } catch (error) {
        setGlobalStatus(userFacingGraphQlErrorMessage(error, t("status.failedToUpdate")));
      } finally {
        markBusy(subscription.id, false);
      }
    },
    [client, markBusy, setGlobalStatus, t, watchSync],
  );

  const syncAll = React.useCallback(async () => {
    try {
      const result = await client.mutation(syncAllListsMutation, { scope: "PUBLIC" }).toPromise();
      if (result.error) throw result.error;
      setGlobalStatus(t("lists.status.syncAllQueued"));
    } catch (error) {
      setGlobalStatus(userFacingGraphQlErrorMessage(error, t("status.failedToUpdate")));
    }
  }, [client, setGlobalStatus, t]);

  const unsubscribe = React.useCallback(
    async (subscription: ListSubscription): Promise<boolean> => {
      markBusy(subscription.id, true);
      try {
        const result = await client
          .mutation(unsubscribeListMutation, { id: subscription.id })
          .toPromise();
        if (result.error) throw result.error;
        setGlobalStatus(t("lists.status.unfollowed", { name: subscription.name }));
        if (detail?.id === subscription.id) setDetail(null);
        await refreshAfterChange();
        return true;
      } catch (error) {
        setGlobalStatus(userFacingGraphQlErrorMessage(error, t("status.failedToUpdate")));
        return false;
      } finally {
        markBusy(subscription.id, false);
      }
    },
    [client, detail, markBusy, refreshAfterChange, setGlobalStatus, t],
  );

  const addExclusion = React.useCallback(
    async (input: AddListExclusionInput): Promise<boolean> => {
      try {
        const result = await client.mutation(addListExclusionMutation, { input }).toPromise();
        if (result.error) throw result.error;
        setGlobalStatus(t("lists.exclusions.added", { name: input.displayTitle }));
        await loadExclusions();
        return true;
      } catch (error) {
        setGlobalStatus(userFacingGraphQlErrorMessage(error, t("status.failedToUpdate")));
        return false;
      }
    },
    [client, loadExclusions, setGlobalStatus, t],
  );

  const removeExclusion = React.useCallback(
    async (exclusion: ListExclusion): Promise<boolean> => {
      markBusy(exclusion.id, true);
      try {
        const result = await client
          .mutation(removeListExclusionMutation, { id: exclusion.id })
          .toPromise();
        if (result.error) throw result.error;
        setGlobalStatus(t("lists.exclusions.removed", { name: exclusion.displayTitle }));
        setExclusions((current) => current.filter((entry) => entry.id !== exclusion.id));
        return true;
      } catch (error) {
        setGlobalStatus(userFacingGraphQlErrorMessage(error, t("status.failedToUpdate")));
        return false;
      } finally {
        markBusy(exclusion.id, false);
      }
    },
    [client, markBusy, setGlobalStatus, t],
  );

  const changeSection = React.useCallback(
    (next: ListsSection) => {
      navigate(buildListsPath(next));
    },
    [navigate],
  );

  return (
    <ListsView
      section={section}
      onSectionChange={changeSection}
      canManageLists={canManageLists}
      loading={loading}
      loadError={loadError}
      onRetry={() => void loadPage()}
      providers={providers}
      subscriptions={subscriptions}
      routeOptions={routeOptions}
      busyIds={busyIds}
      detail={detail}
      onOpenDetail={openDetail}
      onDetailPage={(id, offset) => void loadDetail(id, offset)}
      membershipPageSize={MEMBERSHIP_PAGE_SIZE}
      onPreviewUrl={previewUrl}
      onPreviewSource={previewSource}
      onPreviewSubscription={previewSubscription}
      onSubscribe={subscribe}
      onUpdate={update}
      onSetEnabled={(subscription, enabled) => void setEnabled(subscription, enabled)}
      onSyncNow={(subscription) => void syncNow(subscription)}
      onSyncAll={() => void syncAll()}
      onUnsubscribe={unsubscribe}
      exclusions={exclusions}
      exclusionsLoading={exclusionsLoading}
      onAddExclusion={addExclusion}
      onRemoveExclusion={removeExclusion}
      providerSettings={providerSettings}
      onSaveProviderSettings={canManageLists ? saveProviderSettings : undefined}
    />
  );
}

import * as React from "react";
import { useLocation, useNavigate } from "react-router";
import { useClient } from "urql";

import type { ListsSection } from "@/components/root/types";
import {
  ListsView,
  type ListDetailState,
  type ListTabCounts,
  type ListRouteOptions,
} from "@/components/views/lists/lists-view";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { useExperimentalFeaturesEnabled } from "@/lib/context/instance-features-context";
import { useSessionUser } from "@/lib/hooks/use-auth";
import { useListAccountLink } from "@/lib/hooks/use-list-account-link";
import { APP_PERMISSIONS, hasAppPermission } from "@/lib/utils/permissions";
import { listAccountQuery, listTabCountsQuery, myListAccountsQuery, myListSubscriptionsQuery, personalListRouteOptionsQuery, unlinkListAccountMutation } from "@/lib/graphql/list-accounts";
import { listErrorMessage } from "@/lib/utils/list-error-message";
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
  listLibraryEpisodePoliciesQuery,
  listRouteOptionsQuery,
  listSourcePreviewQuery,
  listSubscriptionDetailQuery,
  listSubscriptionPreviewQuery,
  listSubscriptionsQuery,
  listUrlPreviewQuery,
} from "@/lib/graphql/queries";
import type {
  ListAccount,
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
  listParamInput,
  listFilterInput,
  listSyncWatchSchedule,
  listSyncWatchSettled,
  LIST_SYNC_RUNS_MAX,
  LIST_SYNC_RUNS_SHOWN,
  shownListSyncRuns,
  type AddListExclusionInput,
  type ListSyncWatchSnapshot,
} from "@/lib/utils/lists";
import { buildListsPath, listsSectionFromPath } from "@/lib/utils/routing";

const MEMBERSHIP_PAGE_SIZE = 50;

/**
 * The runs a sync watch compares. Expanding the history mid-watch loads older
 * runs, which must not read as a new sync.
 */
function newestRunIds(runs: readonly ListSyncRun[]): string[] {
  return runs.slice(0, LIST_SYNC_RUNS_SHOWN).map((run) => run.id);
}

type ListsContainerProps = {
  canManageLists: boolean;
};

export function ListsContainer({ canManageLists }: ListsContainerProps) {
  const user = useSessionUser();
  const location = useLocation();
  const section = listsSectionFromPath(location.pathname);
  const [tabCounts, reloadTabCounts] = useListTabCounts(canManageLists, section);
  return (
    <ListsContainerBody
      key={`${user?.id ?? "anonymous"}:${section}`}
      canManageLists={canManageLists}
      tabCounts={tabCounts}
      onTabCountsChanged={reloadTabCounts}
    />
  );
}

/**
 * How many entries each section holds, for the tab strip. Counted again on
 * every section change, so the sections left behind stay current; the section
 * on screen is counted from what it has loaded.
 */
function useListTabCounts(canManageLists: boolean, section: ListsSection): [ListTabCounts, () => void] {
  const client = useClient();
  const user = useSessionUser();
  const userId = user?.id;
  const experimentalFeaturesEnabled = useExperimentalFeaturesEnabled();
  const providerApps = experimentalFeaturesEnabled && hasAppPermission(user, APP_PERMISSIONS.manageSystemSettings);
  const [counts, setCounts] = React.useState<ListTabCounts>({});
  const [changes, setChanges] = React.useState(0);

  React.useEffect(() => {
    let active = true;
    void client
      .query(
        listTabCountsQuery,
        { personal: experimentalFeaturesEnabled, exclusions: canManageLists, providerApps },
        { requestPolicy: "network-only" },
      )
      .toPromise()
      .then(({ data }) => {
        // A count is decoration: one that cannot be read is left off the tab.
        if (!active || !data) return;
        setCounts({
          public: data.listSubscriptions?.length,
          personal: data.myListSubscriptions?.length,
          exclusions: data.listExclusions?.length,
          providerApps: (data.listProviderApps as Array<{ enabled: boolean }> | undefined)?.filter((app) => app.enabled).length,
        });
      });
    return () => {
      active = false;
    };
  }, [canManageLists, changes, client, experimentalFeaturesEnabled, providerApps, section, userId]);

  const reload = React.useCallback(() => setChanges((value) => value + 1), []);
  return [counts, reload];
}

type ListsContainerBodyProps = ListsContainerProps & {
  tabCounts: ListTabCounts;
  onTabCountsChanged: () => void;
};

function ListsContainerBody({ canManageLists, tabCounts, onTabCountsChanged }: ListsContainerBodyProps) {
  const client = useClient();
  const t = useTranslate();
  const setGlobalStatus = useGlobalStatus();
  const location = useLocation();
  const navigate = useNavigate();
  const requestedSection = listsSectionFromPath(location.pathname);
  const section: ListsSection = requestedSection;
  const experimentalFeaturesEnabled = useExperimentalFeaturesEnabled();
  const user = useSessionUser();
  const canManageProviderApps = hasAppPermission(user, APP_PERMISSIONS.manageSystemSettings);
  const personal = section === "personal";

  const [providers, setProviders] = React.useState<ListProviderManifest[]>([]);
  const [accounts, setAccounts] = React.useState<ListAccount[]>([]);
  const [managedAccount, setManagedAccount] = React.useState<ListAccount | null>(null);
  const [accountLoading, setAccountLoading] = React.useState(false);
  const [providerSettings, setProviderSettings] = React.useState<ListProviderSettings[] | null>(null);
  const [publicSubscriptions, setPublicSubscriptions] = React.useState<ListSubscription[]>([]);
  const [personalSubscriptions, setPersonalSubscriptions] = React.useState<ListSubscription[]>([]);
  const subscriptions = personal ? personalSubscriptions : publicSubscriptions;
  const setSubscriptions = personal ? setPersonalSubscriptions : setPublicSubscriptions;
  const [exclusions, setExclusions] = React.useState<ListExclusion[]>([]);
  const [routeOptions, setRouteOptions] = React.useState<ListRouteOptions>({
    libraries: [],
    qualityProfiles: [],
  });
  const [loading, setLoading] = React.useState(true);
  const [loadError, setLoadError] = React.useState<string | null>(null);
  const [exclusionsLoading, setExclusionsLoading] = React.useState(false);
  const [exclusionsLoaded, setExclusionsLoaded] = React.useState(false);
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
      .query(personal ? myListSubscriptionsQuery : listSubscriptionsQuery, {}, { requestPolicy: "network-only" })
      .toPromise();
    if (result.error) {
      throw result.error;
    }
    const loaded = (personal ? result.data?.myListSubscriptions : result.data?.listSubscriptions) ?? [];
    setSubscriptions(loaded);
    return loaded;
  }, [client, personal, setSubscriptions]);

  const loadAccounts = React.useCallback(async (isCurrent: () => boolean = () => true) => {
    const result = await client.query(myListAccountsQuery, {}, { requestPolicy: "network-only" }).toPromise();
    if (!isCurrent()) return;
    if (result.error) throw result.error;
    setAccounts((result.data?.myListAccounts ?? []) as ListAccount[]);
  }, [client]);
  const accountLink = useListAccountLink(loadAccounts);

  const manageAccount = async (account: ListAccount | null) => {
    setManagedAccount(account);
    if (!account) return;
    setAccountLoading(true);
    try {
      const result = await client.query(listAccountQuery, { id: account.id }, { requestPolicy: "network-only" }).toPromise();
      if (result.error) throw result.error;
      setManagedAccount((current) => current?.id === account.id ? result.data?.listAccount ?? null : current);
    } catch (error) {
      setGlobalStatus(listErrorMessage(error, t, t("status.failedToLoad")), { level: "ERROR" });
    } finally { setAccountLoading(false); }
  };

  const unlinkAccount = async (account: ListAccount) => {
    markBusy(account.id, true);
    try {
      const result = await client.mutation(unlinkListAccountMutation, { id: account.id }).toPromise();
      if (result.error) throw result.error;
      setManagedAccount(null);
      await Promise.all([loadAccounts(), loadSubscriptions()]);
      setGlobalStatus(t("lists.accounts.unlinked"));
      return true;
    } catch (error) {
      setGlobalStatus(listErrorMessage(error, t, t("status.failedToUpdate")), { level: "ERROR" });
      return false;
    } finally { markBusy(account.id, false); }
  };

  const loadPage = React.useCallback(async () => {
    setLoading(true);
    setLoadError(null);
    try {
      const [providersResult] = await Promise.all([
        client.query(listProvidersQuery, {}).toPromise(),
        loadSubscriptions(),
        ...(personal && experimentalFeaturesEnabled ? [loadAccounts()] : []),
      ]);
      if (providersResult.error) throw providersResult.error;
      setProviders((providersResult.data?.listProviders ?? []) as ListProviderManifest[]);
      if (canManageLists || personal) {
        const [optionsResult, policiesResult] = await Promise.all([
          client.query(personal ? personalListRouteOptionsQuery : listRouteOptionsQuery, {}).toPromise(),
          // Read apart from the routing choices: effective library settings need
          // library-management permission, and lacking it only leaves the
          // "inherit" choices unlabelled.
          client.query(listLibraryEpisodePoliciesQuery, {}, { requestPolicy: "network-only" }).toPromise(),
        ]);
        const settingsByLibraryId = new Map(
          ((policiesResult.data?.libraries ?? []) as Array<Pick<LibraryRecord, "id" | "settings">>).map(
            (library) => [library.id, library.settings],
          ),
        );
        const withSettings = (library: LibraryRecord): LibraryRecord => ({
          ...library,
          settings: settingsByLibraryId.get(library.id) ?? null,
        });
        // Libraries and profiles only feed the follow form's routing choices;
        // failing to read them must not take the lists themselves down.
        if (optionsResult.error) {
          setRouteOptions({ libraries: [], qualityProfiles: [] });
          setGlobalStatus(
            listErrorMessage(optionsResult.error, t, t("status.failedToLoad")),
            { level: "ERROR" },
          );
          return;
        }
        setRouteOptions({
          libraries: personal
            ? [...new Map([...(optionsResult.data?.requestableLibraries ?? []), ...(optionsResult.data?.manageableLibraries ?? [])].map((library: LibraryRecord) => [library.id, withSettings(library)])).values()]
            : ((optionsResult.data?.libraries ?? []) as LibraryRecord[]).map(withSettings),
          qualityProfiles: (optionsResult.data?.qualityProfileSettings?.profiles ?? []) as Array<{
            id: string;
            name: string;
          }>,
        });
      }
    } catch (error) {
      setLoadError(listErrorMessage(error, t, t("status.failedToLoad")));
    } finally {
      setLoading(false);
    }
  }, [canManageLists, client, experimentalFeaturesEnabled, loadAccounts, loadSubscriptions, personal, setGlobalStatus, t]);

  const loadExclusions = React.useCallback(async () => {
    if (!canManageLists) return;
    setExclusionsLoading(true);
    try {
      const result = await client
        .query(listExclusionsQuery, {}, { requestPolicy: "network-only" })
        .toPromise();
      if (result.error) throw result.error;
      setExclusions((result.data?.listExclusions ?? []) as ListExclusion[]);
      setExclusionsLoaded(true);
    } catch (error) {
      setGlobalStatus(listErrorMessage(error, t, t("status.failedToLoad")), { level: "ERROR" });
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
      setGlobalStatus(listErrorMessage(error, t, t("status.failedToLoad")), { level: "ERROR" });
    }
  }, [canManageLists, client, setGlobalStatus, t]);

  React.useEffect(() => {
    void loadPage();
  }, [loadPage]);

  React.useEffect(() => {
    void loadProviderSettings();
  }, [loadProviderSettings]);

  const refreshProviders = React.useCallback(async () => {
    const result = await client.query(listProvidersQuery, {}, { requestPolicy: "network-only" }).toPromise();
    if (result.error) throw result.error;
    setProviders((result.data?.listProviders ?? []) as ListProviderManifest[]);
    await loadProviderSettings();
  }, [client, loadProviderSettings]);

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
        setGlobalStatus(t("lists.providerSettings.saved"), { level: "SUCCESS" });
        return true;
      } catch (error) {
        setGlobalStatus(listErrorMessage(error, t, t("status.failedToUpdate")), { level: "ERROR" });
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
      setGlobalStatus(listErrorMessage(error, t, t("status.failedToLoad")), { level: "ERROR" });
    }
  }, [loadSubscriptions, setGlobalStatus, t]);

  // Only the newest panel load may land; a background refresh also yields to
  // any panel load (opening, paging) started while it was in flight.
  const detailRequestRef = React.useRef(0);
  const detailLoadingRef = React.useRef(false);
  // The list whose full sync history was asked for; every reload of that
  // panel keeps it expanded until the panel closes.
  const expandedRunsRef = React.useRef<string | null>(null);

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
            : { id, loading: true, subscription: null, memberships: null, runs: [], moreRuns: false, membershipOffset: 0, error: null },
        );
      }
      const expanded = expandedRunsRef.current === id;
      try {
        const result = await client
          .query(
            listSubscriptionDetailQuery,
            {
              id,
              membershipLimit: MEMBERSHIP_PAGE_SIZE,
              membershipOffset,
              runLimit: expanded ? LIST_SYNC_RUNS_MAX : LIST_SYNC_RUNS_SHOWN + 1,
            },
            { requestPolicy: "network-only" },
          )
          .toPromise();
        if (result.error) throw result.error;
        const { runs, more: moreRuns } = shownListSyncRuns(
          (result.data?.listSyncRuns ?? []) as ListSyncRun[],
          expanded,
        );
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
                moreRuns,
                membershipOffset,
                error: null,
              },
        );
        return runs;
      } catch (error) {
        if (background || !isCurrent()) return null;
        detailLoadingRef.current = false;
        const message = listErrorMessage(error, t, t("status.failedToLoad"));
        setDetail((current) => (current?.id !== id ? current : { ...current, loading: false, error: message }));
        return null;
      }
    },
    [client, t],
  );

  const openDetail = React.useCallback(
    (id: string | null) => {
      expandedRunsRef.current = null;
      if (!id) {
        setDetail(null);
        return;
      }
      void loadDetail(id);
    },
    [loadDetail],
  );

  const showAllRuns = React.useCallback(
    (id: string) => {
      const shown = detailRef.current;
      if (shown?.id !== id) return;
      expandedRunsRef.current = id;
      void loadDetail(id, shown.membershipOffset);
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
        setGlobalStatus(listErrorMessage(error, t, t("lists.preview.failed")), { level: "ERROR" });
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
          filters: (source.previewFilters ?? []).map(listFilterInput),
          kinds: source.previewKinds ?? null,
          maxPerSync: source.previewMaxPerSync ?? null,
          input: {
            provider: source.provider,
            sourceType: source.sourceType,
            params: source.params.filter((param) => param.value.trim()).map(listParamInput),
            url: source.url,
            credentialId: source.credentialId ?? null,
          },
        },
        "listSourcePreview",
      ),
    [runPreview],
  );

  const previewSubscription = React.useCallback(
    (id: string, draft?: ListSubscriptionDraft) => runPreview(listSubscriptionPreviewQuery, {
      id,
      filters: draft?.filters.map(listFilterInput) ?? null,
      kinds: draft?.kinds ?? null,
      ...(draft ? { maxPerSync: draft.maxPerSync } : {}),
    }, "listSubscriptionPreview"),
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
        setGlobalStatus(listErrorMessage(error, t, t("status.failedToUpdate")), { level: "ERROR" });
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
        setGlobalStatus(t("lists.status.updated", { name: draft.name.trim() }), { level: "SUCCESS" });
        await refreshAfterChange();
        if (detail?.id === id) void loadDetail(id, detail.membershipOffset);
        return true;
      } catch (error) {
        setGlobalStatus(listErrorMessage(error, t, t("status.failedToUpdate")), { level: "ERROR" });
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
        setGlobalStatus(listErrorMessage(error, t, t("status.failedToUpdate")), { level: "ERROR" });
      } finally {
        markBusy(subscription.id, false);
        await refreshAfterChange();
        if (detail?.id === subscription.id) void loadDetail(subscription.id, detail.membershipOffset);
      }
    },
    [client, detail, loadDetail, markBusy, refreshAfterChange, setGlobalStatus, setSubscriptions, t],
  );

  // "Sync now" and "Sync all" only queue syncs, so the table and an open detail
  // panel keep refreshing, backing off, until each queued sync visibly finishes
  // or its budget runs out. One poll serves every watched list, so queuing many
  // lists costs one table refresh per tick, not one per list. Detail refreshes
  // stop as soon as the panel closes or shows a list no longer watched.
  const detailRef = React.useRef(detail);
  React.useEffect(() => {
    detailRef.current = detail;
  }, [detail]);
  const syncWatchesRef = React.useRef(
    new Map<string, { baseline: ListSyncWatchSnapshot; startedAt: number }>(),
  );
  const syncWatchTimerRef = React.useRef<ReturnType<typeof setTimeout> | null>(null);
  // Bumped whenever the poll restarts or the page unmounts; a tick that
  // started under an older generation drops its result.
  const syncWatchGenerationRef = React.useRef(0);

  React.useEffect(() => {
    const watches = syncWatchesRef.current;
    return () => {
      syncWatchGenerationRef.current += 1;
      if (syncWatchTimerRef.current !== null) clearTimeout(syncWatchTimerRef.current);
      syncWatchTimerRef.current = null;
      watches.clear();
    };
  }, []);

  const watchSync = React.useCallback(
    (queued: readonly ListSubscription[]) => {
      if (queued.length === 0) return;
      const watches = syncWatchesRef.current;
      const shownDetail = detailRef.current;
      const now = Date.now();
      for (const subscription of queued) {
        watches.set(subscription.id, {
          baseline: {
            lastAt: subscription.sync.lastAt,
            state: subscription.sync.state,
            runIds:
              shownDetail?.id === subscription.id && shownDetail.subscription
                ? newestRunIds(shownDetail.runs)
                : null,
          },
          startedAt: now,
        });
      }

      // A newly queued sync restarts the poll at its quickest refresh.
      const generation = ++syncWatchGenerationRef.current;
      if (syncWatchTimerRef.current !== null) clearTimeout(syncWatchTimerRef.current);
      syncWatchTimerRef.current = null;
      let attempt = 0;

      const schedule = () => {
        const startedAtById = new Map(
          [...watches].map(([id, watch]) => [id, watch.startedAt] as const),
        );
        const next = listSyncWatchSchedule(attempt, startedAtById, Date.now());
        attempt += 1;
        const keep = new Set(next?.keep ?? []);
        for (const id of [...watches.keys()]) {
          if (!keep.has(id)) watches.delete(id);
        }
        syncWatchTimerRef.current = next ? setTimeout(() => void tick(), next.delay) : null;
      };

      const tick = async () => {
        syncWatchTimerRef.current = null;
        let loaded: ListSubscription[] | null = null;
        try {
          loaded = await loadSubscriptions();
        } catch {
          // A failed refresh is retried on the next tick.
        }
        if (generation !== syncWatchGenerationRef.current) return;
        if (!loaded) {
          schedule();
          return;
        }
        const shown = detailRef.current;
        const runs =
          shown && watches.has(shown.id)
            ? await loadDetail(shown.id, shown.membershipOffset, { background: true })
            : null;
        if (generation !== syncWatchGenerationRef.current) return;
        for (const [id, watch] of [...watches]) {
          const current = loaded.find((entry) => entry.id === id);
          if (
            !current ||
            listSyncWatchSettled(watch.baseline, {
              lastAt: current.sync.lastAt,
              state: current.sync.state,
              runIds: runs && shown?.id === id ? newestRunIds(runs) : null,
            })
          ) {
            watches.delete(id);
          }
        }
        if (watches.size > 0) schedule();
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
        watchSync([subscription]);
      } catch (error) {
        setGlobalStatus(listErrorMessage(error, t, t("status.failedToUpdate")), { level: "ERROR" });
      } finally {
        markBusy(subscription.id, false);
      }
    },
    [client, markBusy, setGlobalStatus, t, watchSync],
  );

  const syncAll = React.useCallback(async () => {
    try {
      const result = await client.mutation(syncAllListsMutation, { scope: personal ? "PERSONAL" : "PUBLIC" }).toPromise();
      if (result.error) throw result.error;
      setGlobalStatus(t("lists.status.syncAllQueued"));
      const queuedIds = new Set<string>(result.data?.syncAllLists?.subscriptionIds ?? []);
      watchSync(subscriptions.filter((subscription) => queuedIds.has(subscription.id)));
    } catch (error) {
      setGlobalStatus(listErrorMessage(error, t, t("status.failedToUpdate")), { level: "ERROR" });
    }
  }, [client, personal, setGlobalStatus, subscriptions, t, watchSync]);

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
        setGlobalStatus(listErrorMessage(error, t, t("status.failedToUpdate")), { level: "ERROR" });
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
        setGlobalStatus(listErrorMessage(error, t, t("status.failedToUpdate")), { level: "ERROR" });
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
        setGlobalStatus(listErrorMessage(error, t, t("status.failedToUpdate")), { level: "ERROR" });
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

  const shownTabCounts: ListTabCounts = {
    ...tabCounts,
    ...(loading || loadError ? {} : { [personal ? "personal" : "public"]: subscriptions.length }),
    ...(exclusionsLoaded ? { exclusions: exclusions.length } : {}),
  };

  return (
    <ListsView
      section={section}
      onSectionChange={changeSection}
      tabCounts={shownTabCounts}
      onProviderAppsChanged={onTabCountsChanged}
      canManageLists={canManageLists}
      experimentalFeaturesEnabled={experimentalFeaturesEnabled}
      canManageProviderApps={canManageProviderApps}
      onRefreshProviders={refreshProviders}
      accounts={accounts}
      managedAccount={managedAccount}
      accountLoading={accountLoading}
      linkingProvider={accountLink.provider}
      accountLinkError={accountLink.error}
      onLinkAccount={(provider) => void accountLink.start(provider)}
      onCancelLink={accountLink.cancel}
      onManageAccount={(account) => void manageAccount(account)}
      onUnlinkAccount={unlinkAccount}
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
      onShowAllRuns={showAllRuns}
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

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
import { useSessionUser } from "@/lib/hooks/use-auth";
import type { GutterPanelMemory } from "@/lib/hooks/use-gutter-panel";
import { useListAccountLink } from "@/lib/hooks/use-list-account-link";
import { APP_PERMISSIONS, hasAppPermission } from "@/lib/utils/permissions";
import {
  listAccountQuery,
  listTabCountsQuery,
  myListAccountsQuery,
  myListSubscriptionsQuery,
  personalListRouteOptionsQuery,
  unlinkListAccountMutation,
} from "@/lib/graphql/list-accounts";
import { hasGraphQlErrorCode } from "@/lib/graphql/error-message";
import {
  listErrorMessage,
  listRefusalKey,
} from "@/lib/utils/list-error-message";
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
  LIST_SYNC_RUNS_MAX,
  LIST_SYNC_RUNS_SHOWN,
  shownListSyncRuns,
  followedListsChanged,
  latestRead,
  listAccountsChanged,
  listExclusionsChanged,
  listProvidersChanged,
  listTabCountsChanged,
  type AddListExclusionInput,
} from "@/lib/utils/lists";
import { useListsReactiveRefresh } from "@/lib/hooks/use-lists-reactive-refresh";
import { buildListsPath, listsSectionFromPath } from "@/lib/utils/routing";

const MEMBERSHIP_PAGE_SIZE = 50;

type ListsContainerProps = {
  canManageLists: boolean;
};

export function ListsContainer({ canManageLists }: ListsContainerProps) {
  const user = useSessionUser();
  const location = useLocation();
  const section = listsSectionFromPath(location.pathname);
  const tabCounts = useListTabCounts(canManageLists, section);
  // The body remounts on every tab; the plugins panel stays as it was.
  const pluginsPanelMemory: GutterPanelMemory = React.useRef(null);
  return (
    <ListsContainerBody
      key={`${user?.id ?? "anonymous"}:${section}`}
      canManageLists={canManageLists}
      tabCounts={tabCounts}
      pluginsPanelMemory={pluginsPanelMemory}
    />
  );
}

/**
 * How many entries each section holds, for the tab strip. Counted again on
 * every section change and whenever a change to one of them is announced, so
 * the sections left behind stay current; the section on screen is counted
 * from what it has loaded.
 */
function useListTabCounts(
  canManageLists: boolean,
  section: ListsSection,
): ListTabCounts {
  const client = useClient();
  const user = useSessionUser();
  const userId = user?.id;
  const [counts, setCounts] = React.useState<ListTabCounts>({});
  const [changes, setChanges] = React.useState(0);

  React.useEffect(() => {
    let active = true;
    void client
      .query(
        listTabCountsQuery,
        { personal: true, exclusions: canManageLists },
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
        });
      });
    return () => {
      active = false;
    };
  }, [canManageLists, changes, client, section, userId]);

  const reload = React.useCallback(() => setChanges((value) => value + 1), []);
  useListsReactiveRefresh(listTabCountsChanged, reload);
  return counts;
}

type ListsContainerBodyProps = ListsContainerProps & {
  tabCounts: ListTabCounts;
  pluginsPanelMemory: GutterPanelMemory;
};

function ListsContainerBody({
  canManageLists,
  tabCounts,
  pluginsPanelMemory,
}: ListsContainerBodyProps) {
  const client = useClient();
  const t = useTranslate();
  const setGlobalStatus = useGlobalStatus();
  const location = useLocation();
  const navigate = useNavigate();
  const requestedSection = listsSectionFromPath(location.pathname);
  const section: ListsSection = requestedSection;
  const user = useSessionUser();
  const canManageProviderApps = hasAppPermission(
    user,
    APP_PERMISSIONS.manageSystemSettings,
  );
  const personal = section === "personal";

  const [providers, setProviders] = React.useState<ListProviderManifest[]>([]);
  const [accounts, setAccounts] = React.useState<ListAccount[]>([]);
  const [managedAccounts, setManagedAccounts] = React.useState<ListAccount[]>(
    [],
  );
  // The latest read of an open provider's accounts; an earlier one still in flight is dropped.
  const manageRun = React.useRef(0);
  const [accountLoading, setAccountLoading] = React.useState(false);
  const [accountReadErrors, setAccountReadErrors] = React.useState<
    Readonly<Record<string, string>>
  >({});
  const [providerSettings, setProviderSettings] = React.useState<
    ListProviderSettings[] | null
  >(null);
  const [publicSubscriptions, setPublicSubscriptions] = React.useState<
    ListSubscription[]
  >([]);
  const [personalSubscriptions, setPersonalSubscriptions] = React.useState<
    ListSubscription[]
  >([]);
  const subscriptions = personal ? personalSubscriptions : publicSubscriptions;
  const setSubscriptions = personal
    ? setPersonalSubscriptions
    : setPublicSubscriptions;
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

  // Only the newest read of each kind may land. A refresh started by a live
  // event can race the first read, an earlier refresh, or a change made here.
  const [reads] = React.useState(() => ({
    subscriptions: latestRead(),
    accounts: latestRead(),
    exclusions: latestRead(),
    providers: latestRead(),
    providerSettings: latestRead(),
  }));

  const markBusy = React.useCallback((id: string, busy: boolean) => {
    setBusyIds((current) => {
      const next = new Set(current);
      if (busy) next.add(id);
      else next.delete(id);
      return next;
    });
  }, []);

  /** Reads the followed lists. Null when a newer read replaced this one. */
  const loadSubscriptions = React.useCallback(async (): Promise<
    ListSubscription[] | null
  > => {
    const isNewest = reads.subscriptions.begin();
    const result = await client
      .query(
        personal ? myListSubscriptionsQuery : listSubscriptionsQuery,
        {},
        { requestPolicy: "network-only" },
      )
      .toPromise();
    if (!isNewest()) return null;
    if (result.error) {
      throw result.error;
    }
    const loaded =
      (personal
        ? result.data?.myListSubscriptions
        : result.data?.listSubscriptions) ?? [];
    setSubscriptions(loaded);
    return loaded;
  }, [client, personal, reads, setSubscriptions]);

  const loadAccounts = React.useCallback(
    async (isCurrent: () => boolean = () => true) => {
      const isNewest = reads.accounts.begin();
      const result = await client
        .query(myListAccountsQuery, {}, { requestPolicy: "network-only" })
        .toPromise();
      if (!isCurrent() || !isNewest()) return;
      if (result.error) throw result.error;
      setAccounts((result.data?.myListAccounts ?? []) as ListAccount[]);
    },
    [client, reads],
  );
  const accountLink = useListAccountLink(loadAccounts);

  const manageAccounts = async (open: ListAccount[]) => {
    const run = ++manageRun.current;
    setManagedAccounts([]);
    setAccountReadErrors({});
    setAccountLoading(open.length > 0);
    if (open.length === 0) return;
    // An account that cannot be read keeps its place without lists, and the others still load.
    // Nobody asked for this read, so why it failed is said beside the account rather than announced.
    const failed: Record<string, string> = {};
    const loaded = await Promise.all(
      open.map(async (account): Promise<ListAccount | null> => {
        try {
          const result = await client
            .query(
              listAccountQuery,
              { id: account.id },
              { requestPolicy: "network-only" },
            )
            .toPromise();
          if (result.error) throw result.error;
          return result.data?.listAccount ?? null;
        } catch (error) {
          const refusal = listRefusalKey(error);
          failed[account.id] = t(
            refusal ??
              (hasGraphQlErrorCode(error, "TEMPORARY_UNAVAILABLE")
                ? "lists.refusal.accountAuthProviderUnavailable"
                : "status.failedToLoad"),
          );
          return null;
        }
      }),
    );
    if (run !== manageRun.current) return;
    setManagedAccounts(
      loaded.filter((account): account is ListAccount => account !== null),
    );
    setAccountReadErrors(failed);
    setAccountLoading(false);
  };

  const unlinkAccount = async (account: ListAccount) => {
    markBusy(account.id, true);
    try {
      const result = await client
        .mutation(unlinkListAccountMutation, { id: account.id })
        .toPromise();
      if (result.error) throw result.error;
      setManagedAccounts((current) =>
        current.filter((entry) => entry.id !== account.id),
      );
      await Promise.all([loadAccounts(), loadSubscriptions()]);
      setGlobalStatus(t("lists.accounts.unlinked"));
      return true;
    } catch (error) {
      setGlobalStatus(listErrorMessage(error, t, t("status.failedToUpdate")), {
        level: "ERROR",
      });
      return false;
    } finally {
      markBusy(account.id, false);
    }
  };

  const loadPage = React.useCallback(async () => {
    setLoading(true);
    setLoadError(null);
    try {
      const providersNewest = reads.providers.begin();
      const [providersResult] = await Promise.all([
        client.query(listProvidersQuery, {}).toPromise(),
        loadSubscriptions(),
        ...(personal ? [loadAccounts()] : []),
      ]);
      if (providersResult.error) throw providersResult.error;
      if (providersNewest()) {
        setProviders(
          (providersResult.data?.listProviders ?? []) as ListProviderManifest[],
        );
      }
      if (canManageLists || personal) {
        const [optionsResult, policiesResult] = await Promise.all([
          client
            .query(
              personal ? personalListRouteOptionsQuery : listRouteOptionsQuery,
              {},
            )
            .toPromise(),
          // Read apart from the routing choices: effective library settings need
          // library-management permission, and lacking it only leaves the
          // "inherit" choices unlabelled.
          client
            .query(
              listLibraryEpisodePoliciesQuery,
              {},
              { requestPolicy: "network-only" },
            )
            .toPromise(),
        ]);
        const settingsByLibraryId = new Map(
          (
            (policiesResult.data?.libraries ?? []) as Array<
              Pick<LibraryRecord, "id" | "settings">
            >
          ).map((library) => [library.id, library.settings]),
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
            ? [
                ...new Map(
                  [
                    ...(optionsResult.data?.requestableLibraries ?? []),
                    ...(optionsResult.data?.manageableLibraries ?? []),
                  ].map((library: LibraryRecord) => [
                    library.id,
                    withSettings(library),
                  ]),
                ).values(),
              ]
            : ((optionsResult.data?.libraries ?? []) as LibraryRecord[]).map(
                withSettings,
              ),
          qualityProfiles: (optionsResult.data?.qualityProfileSettings
            ?.profiles ?? []) as Array<{
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
  }, [
    canManageLists,
    client,
    loadAccounts,
    loadSubscriptions,
    personal,
    reads,
    setGlobalStatus,
    t,
  ]);

  // A background read leaves the table on screen rather than showing it as loading.
  const loadExclusions = React.useCallback(
    async ({ background = false }: { background?: boolean } = {}) => {
      if (!canManageLists) return;
      const isNewest = reads.exclusions.begin();
      if (!background) setExclusionsLoading(true);
      try {
        const result = await client
          .query(listExclusionsQuery, {}, { requestPolicy: "network-only" })
          .toPromise();
        if (!isNewest()) return;
        if (result.error) throw result.error;
        setExclusions((result.data?.listExclusions ?? []) as ListExclusion[]);
        setExclusionsLoaded(true);
      } catch (error) {
        setGlobalStatus(listErrorMessage(error, t, t("status.failedToLoad")), {
          level: "ERROR",
        });
      } finally {
        if (!background) setExclusionsLoading(false);
      }
    },
    [canManageLists, client, reads, setGlobalStatus, t],
  );

  const loadProviderSettings = React.useCallback(async () => {
    if (!canManageLists) return;
    const isNewest = reads.providerSettings.begin();
    try {
      const result = await client
        .query(listProviderSettingsQuery, {}, { requestPolicy: "network-only" })
        .toPromise();
      if (!isNewest()) return;
      if (result.error) throw result.error;
      setProviderSettings(
        (result.data?.listProviderSettings ?? []) as ListProviderSettings[],
      );
    } catch (error) {
      setGlobalStatus(listErrorMessage(error, t, t("status.failedToLoad")), {
        level: "ERROR",
      });
    }
  }, [canManageLists, client, reads, setGlobalStatus, t]);

  React.useEffect(() => {
    void loadPage();
  }, [loadPage]);

  React.useEffect(() => {
    void loadProviderSettings();
  }, [loadProviderSettings]);

  const refreshProviders = React.useCallback(async () => {
    const isNewest = reads.providers.begin();
    const result = await client
      .query(listProvidersQuery, {}, { requestPolicy: "network-only" })
      .toPromise();
    if (isNewest()) {
      if (result.error) throw result.error;
      setProviders(
        (result.data?.listProviders ?? []) as ListProviderManifest[],
      );
    }
    await loadProviderSettings();
  }, [client, loadProviderSettings, reads]);

  const saveProviderSettings = React.useCallback(
    async (
      provider: string,
      changes: ListProviderSettingChange[],
    ): Promise<boolean> => {
      try {
        const result = await client
          .mutation(updateListProviderSettingsMutation, { provider, changes })
          .toPromise();
        if (result.error) throw result.error;
        const saved = result.data?.updateListProviderSettings as
          ListProviderSettings | undefined;
        if (saved) {
          // A read still in flight predates this save and must not undo it.
          reads.providerSettings.invalidate();
          setProviderSettings((current) => [
            ...(current ?? []).filter(
              (entry) => entry.providerType !== saved.providerType,
            ),
            saved,
          ]);
        }
        setGlobalStatus(t("lists.providerSettings.saved"), {
          level: "SUCCESS",
        });
        return true;
      } catch (error) {
        setGlobalStatus(
          listErrorMessage(error, t, t("status.failedToUpdate")),
          { level: "ERROR" },
        );
        return false;
      }
    },
    [client, reads, setGlobalStatus, t],
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
      setGlobalStatus(listErrorMessage(error, t, t("status.failedToLoad")), {
        level: "ERROR",
      });
    }
  }, [loadSubscriptions, setGlobalStatus, t]);

  // Only the newest panel load may land; a background refresh also yields to
  // any panel load (opening, paging) started while it was in flight.
  const detailRequestRef = React.useRef(0);
  const detailLoadingRef = React.useRef(false);
  // Background refreshes are ordered among themselves too. One asked for
  // while a panel load is in flight is owed: that load may have read before
  // the change it was asked for, so the panel is read once more when it lands.
  const detailBackgroundRef = React.useRef(0);
  const detailRefreshOwedRef = React.useRef(false);
  // The list whose full sync history was asked for; every reload of that
  // panel keeps it expanded until the panel closes.
  const expandedRunsRef = React.useRef<string | null>(null);

  const loadDetail = React.useCallback(
    (
      id: string,
      membershipOffset = 0,
      { background = false }: { background?: boolean } = {},
    ): Promise<ListSyncRun[] | null> => {
      const read = async (
        background: boolean,
      ): Promise<ListSyncRun[] | null> => {
        // A background refresh only updates a panel that is still showing this
        // list; it never opens the panel or shows it as loading.
        if (background && detailLoadingRef.current) {
          detailRefreshOwedRef.current = true;
          return null;
        }
        const request = background
          ? detailRequestRef.current
          : ++detailRequestRef.current;
        const backgroundRequest = background
          ? ++detailBackgroundRef.current
          : detailBackgroundRef.current;
        const isCurrent = () =>
          request === detailRequestRef.current &&
          (!background ||
            (!detailLoadingRef.current &&
              backgroundRequest === detailBackgroundRef.current));
        if (!background) {
          detailLoadingRef.current = true;
          // This load reads after every change heard so far.
          detailRefreshOwedRef.current = false;
          setDetail((current) =>
            current?.id === id
              ? { ...current, loading: true }
              : {
                  id,
                  loading: true,
                  subscription: null,
                  memberships: null,
                  runs: [],
                  moreRuns: false,
                  membershipOffset: 0,
                  error: null,
                },
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
                runLimit: expanded
                  ? LIST_SYNC_RUNS_MAX
                  : LIST_SYNC_RUNS_SHOWN + 1,
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
                  subscription: (result.data?.listSubscription ??
                    null) as ListSubscription | null,
                  memberships: (result.data?.listSubscriptionMemberships ??
                    null) as ListMembershipPage | null,
                  runs,
                  moreRuns,
                  membershipOffset,
                  error: null,
                },
          );
          if (!background && detailRefreshOwedRef.current) {
            detailRefreshOwedRef.current = false;
            void read(true);
          }
          return runs;
        } catch (error) {
          if (background || !isCurrent()) return null;
          detailLoadingRef.current = false;
          const message = listErrorMessage(error, t, t("status.failedToLoad"));
          setDetail((current) =>
            current?.id !== id
              ? current
              : { ...current, loading: false, error: message },
          );
          return null;
        }
      };
      return read(background);
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
    async (
      query: string,
      variables: Record<string, unknown>,
      field: string,
    ): Promise<ListPreview | null> => {
      try {
        const result = await client
          .query(query, variables, { requestPolicy: "network-only" })
          .toPromise();
        if (result.error) throw result.error;
        return (result.data?.[field] ?? null) as ListPreview | null;
      } catch (error) {
        setGlobalStatus(listErrorMessage(error, t, t("lists.preview.failed")), {
          level: "ERROR",
        });
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
            params: source.params
              .filter((param) => param.value.trim())
              .map(listParamInput),
            url: source.url,
            credentialId: source.credentialId ?? null,
          },
        },
        "listSourcePreview",
      ),
    [runPreview],
  );

  const previewSubscription = React.useCallback(
    (id: string, draft?: ListSubscriptionDraft) =>
      runPreview(
        listSubscriptionPreviewQuery,
        {
          id,
          filters: draft?.filters.map(listFilterInput) ?? null,
          kinds: draft?.kinds ?? null,
          ...(draft ? { maxPerSync: draft.maxPerSync } : {}),
        },
        "listSubscriptionPreview",
      ),
    [runPreview],
  );

  const subscribe = React.useCallback(
    async (
      source: ListSourceDraft,
      draft: ListSubscriptionDraft,
    ): Promise<boolean> => {
      try {
        const result = await client
          .mutation(subscribeListMutation, {
            input: draftToSubscribeInput(source, draft),
          })
          .toPromise();
        if (result.error) throw result.error;
        setGlobalStatus(
          t("lists.status.followed", { name: draft.name.trim() }),
        );
        await refreshAfterChange();
        return true;
      } catch (error) {
        setGlobalStatus(
          listErrorMessage(error, t, t("status.failedToUpdate")),
          { level: "ERROR" },
        );
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
          .mutation(updateListSubscriptionMutation, {
            id,
            input: draftToUpdateInput(draft),
          })
          .toPromise();
        if (result.error) throw result.error;
        setGlobalStatus(
          t("lists.status.updated", { name: draft.name.trim() }),
          { level: "SUCCESS" },
        );
        await refreshAfterChange();
        if (detail?.id === id) void loadDetail(id, detail.membershipOffset);
        return true;
      } catch (error) {
        setGlobalStatus(
          listErrorMessage(error, t, t("status.failedToUpdate")),
          { level: "ERROR" },
        );
        return false;
      } finally {
        markBusy(id, false);
      }
    },
    [
      client,
      detail,
      loadDetail,
      markBusy,
      refreshAfterChange,
      setGlobalStatus,
      t,
    ],
  );

  const setEnabled = React.useCallback(
    async (subscription: ListSubscription, enabled: boolean) => {
      markBusy(subscription.id, true);
      // A read still in flight predates this change and must not undo it.
      reads.subscriptions.invalidate();
      setSubscriptions((current) =>
        current.map((entry) =>
          entry.id === subscription.id ? { ...entry, enabled } : entry,
        ),
      );
      try {
        const result = await client
          .mutation(setListSubscriptionEnabledMutation, {
            id: subscription.id,
            enabled,
          })
          .toPromise();
        if (result.error) throw result.error;
      } catch (error) {
        setGlobalStatus(
          listErrorMessage(error, t, t("status.failedToUpdate")),
          { level: "ERROR" },
        );
      } finally {
        markBusy(subscription.id, false);
        await refreshAfterChange();
        if (detail?.id === subscription.id)
          void loadDetail(subscription.id, detail.membershipOffset);
      }
    },
    [
      client,
      detail,
      loadDetail,
      markBusy,
      reads,
      refreshAfterChange,
      setGlobalStatus,
      setSubscriptions,
      t,
    ],
  );

  const detailRef = React.useRef(detail);
  React.useEffect(() => {
    detailRef.current = detail;
  }, [detail]);

  // Live refresh. What this page shows is read again whenever a change to it
  // is announced, whoever made it: a sync ending (which "Sync now", "Sync
  // all" and a new follow only queue), a list followed, edited, turned off
  // or unfollowed in another session, an account linked or unlinked. Each
  // announcement is written after its change is stored, and only the newest
  // read lands, so the page cannot settle on something older than the server.
  useListsReactiveRefresh(followedListsChanged, () => {
    const shown = detailRef.current;
    if (shown)
      void loadDetail(shown.id, shown.membershipOffset, { background: true });
    void loadSubscriptions()
      .then((loaded) => {
        const open = detailRef.current;
        if (!loaded || !open || loaded.some((entry) => entry.id === open.id))
          return;
        // The list on the panel was unfollowed elsewhere; nothing is left to show.
        detailRequestRef.current += 1;
        detailLoadingRef.current = false;
        expandedRunsRef.current = null;
        setDetail((current) => (current?.id === open.id ? null : current));
      })
      .catch(() => {
        // A background read that fails leaves what is shown; the next change reads again.
      });
  });
  useListsReactiveRefresh(
    listAccountsChanged,
    () => {
      void loadAccounts().catch(() => {});
    },
    { pause: !personal },
  );
  useListsReactiveRefresh(
    listExclusionsChanged,
    () => {
      void loadExclusions({ background: true });
    },
    { pause: !canManageLists || !exclusionsLoaded },
  );
  useListsReactiveRefresh(listProvidersChanged, () => {
    void refreshProviders().catch(() => {});
  });

  const syncNow = React.useCallback(
    async (subscription: ListSubscription) => {
      markBusy(subscription.id, true);
      try {
        const result = await client
          .mutation(syncListSubscriptionMutation, { id: subscription.id })
          .toPromise();
        if (result.error) throw result.error;
        setGlobalStatus(
          t("lists.status.syncQueued", { name: subscription.name }),
        );
      } catch (error) {
        setGlobalStatus(
          listErrorMessage(error, t, t("status.failedToUpdate")),
          { level: "ERROR" },
        );
      } finally {
        markBusy(subscription.id, false);
      }
    },
    [client, markBusy, setGlobalStatus, t],
  );

  const syncAll = React.useCallback(async () => {
    try {
      const result = await client
        .mutation(syncAllListsMutation, {
          scope: personal ? "PERSONAL" : "PUBLIC",
        })
        .toPromise();
      if (result.error) throw result.error;
      setGlobalStatus(t("lists.status.syncAllQueued"));
    } catch (error) {
      setGlobalStatus(listErrorMessage(error, t, t("status.failedToUpdate")), {
        level: "ERROR",
      });
    }
  }, [client, personal, setGlobalStatus, t]);

  const unsubscribe = React.useCallback(
    async (subscription: ListSubscription): Promise<boolean> => {
      markBusy(subscription.id, true);
      try {
        const result = await client
          .mutation(unsubscribeListMutation, { id: subscription.id })
          .toPromise();
        if (result.error) throw result.error;
        setGlobalStatus(
          t("lists.status.unfollowed", { name: subscription.name }),
        );
        if (detail?.id === subscription.id) setDetail(null);
        await refreshAfterChange();
        return true;
      } catch (error) {
        setGlobalStatus(
          listErrorMessage(error, t, t("status.failedToUpdate")),
          { level: "ERROR" },
        );
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
        const result = await client
          .mutation(addListExclusionMutation, { input })
          .toPromise();
        if (result.error) throw result.error;
        setGlobalStatus(
          t("lists.exclusions.added", { name: input.displayTitle }),
        );
        await loadExclusions();
        return true;
      } catch (error) {
        setGlobalStatus(
          listErrorMessage(error, t, t("status.failedToUpdate")),
          { level: "ERROR" },
        );
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
        setGlobalStatus(
          t("lists.exclusions.removed", { name: exclusion.displayTitle }),
        );
        // A read still in flight predates this removal and must not bring it back.
        reads.exclusions.invalidate();
        setExclusions((current) =>
          current.filter((entry) => entry.id !== exclusion.id),
        );
        return true;
      } catch (error) {
        setGlobalStatus(
          listErrorMessage(error, t, t("status.failedToUpdate")),
          { level: "ERROR" },
        );
        return false;
      } finally {
        markBusy(exclusion.id, false);
      }
    },
    [client, markBusy, reads, setGlobalStatus, t],
  );

  const changeSection = React.useCallback(
    (next: ListsSection) => {
      navigate(buildListsPath(next));
    },
    [navigate],
  );

  const shownTabCounts: ListTabCounts = {
    ...tabCounts,
    ...(loading || loadError
      ? {}
      : { [personal ? "personal" : "public"]: subscriptions.length }),
    ...(exclusionsLoaded ? { exclusions: exclusions.length } : {}),
  };

  return (
    <ListsView
      section={section}
      onSectionChange={changeSection}
      tabCounts={shownTabCounts}
      canManageLists={canManageLists}
      canManageProviderApps={canManageProviderApps}
      pluginsPanelMemory={pluginsPanelMemory}
      onRefreshProviders={refreshProviders}
      accounts={accounts}
      managedAccounts={managedAccounts}
      accountLoading={accountLoading}
      accountReadErrors={accountReadErrors}
      linkingProvider={accountLink.provider}
      checkingAccountKey={accountLink.checkingKey}
      accountLinkError={accountLink.error}
      onLinkAccount={(provider, memberKey) =>
        void accountLink.start(provider, memberKey)
      }
      onCancelLink={accountLink.cancel}
      onManageAccounts={(open) => void manageAccounts(open)}
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
      onSetEnabled={(subscription, enabled) =>
        void setEnabled(subscription, enabled)
      }
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

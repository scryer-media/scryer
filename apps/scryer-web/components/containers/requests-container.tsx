import * as React from "react";
import { useClient } from "urql";

import { RequestsView } from "@/components/views/requests-view";
import {
  approveMediaRequestMutation,
  cancelMyMediaRequestMutation,
  convertTitleClaimToPermanentMutation,
  dismissMediaRequestMutation,
  extendTitleClaimMutation,
  releaseTitleClaimMutation,
  updateMyMediaRequestMutation,
} from "@/lib/graphql/mutations";
import {
  mediaRequestAdminLibrariesQuery,
  mediaRequestRequesterLibrariesQuery,
  mediaRequestsQuery,
  myMediaRequestsQuery,
  qualityProfileOptionsQuery,
  titleClaimsQuery,
} from "@/lib/graphql/queries";
import type { Facet, LibraryRecord, MediaRequestRecord } from "@/lib/types";
import type { TitleClaimRecord } from "@/lib/types/request-rule-sets";
import type { MonitorSelectionDraft } from "@/lib/types/titles";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import {
  dispatchNavigationBadgesRefresh,
  NAVIGATION_BADGES_REFRESH_EVENT,
  type NavigationBadgesRefreshDetail,
} from "@/lib/events/navigation-badges";
import { useAuth } from "@/lib/hooks/use-auth";
import { useMediaRequestsSubscription } from "@/lib/hooks/use-media-requests-subscription";
import {
  hasAnyLibraryPermission,
  LIBRARY_PERMISSIONS,
} from "@/lib/utils/permissions";
import { normalizeLibraryFilterSelection } from "@/lib/utils/library-filter";

type RequestsContainerProps = {
  facet?: Facet | null;
};

type RequestsMode = "admin" | "mine";
type RequestStatusFilter = "all" | MediaRequestRecord["status"];

type QualityProfileOption = {
  id: string;
  name: string;
};

type UpdateRequestValues = {
  requestedQualityProfileId: string;
  requestedMonitorType?: string;
  requestedMonitorSelection?: MonitorSelectionDraft;
};

type ApproveRequestValues = {
  qualityProfileId: string;
  monitorType?: string;
  monitorSelection?: MonitorSelectionDraft;
  leaseDays?: number;
  leaseForever?: boolean;
  tags?: string[];
};

function sameStringArray(left: string[], right: string[]): boolean {
  return left.length === right.length && left.every((value, index) => value === right[index]);
}

function externalIdKey(source: string, value: string): string {
  return `${source.trim().toLowerCase()}:${value.trim()}`;
}

function requestsOverlap(left: MediaRequestRecord, right: MediaRequestRecord): boolean {
  if (left.libraryId !== right.libraryId || left.facet !== right.facet) {
    return false;
  }

  const rightIds = new Set(
    right.externalIds.map((externalId) =>
      externalIdKey(externalId.source, externalId.value),
    ),
  );
  return left.externalIds.some((externalId) =>
    rightIds.has(externalIdKey(externalId.source, externalId.value)),
  );
}

function collapseRequestGroup(group: MediaRequestRecord[]): MediaRequestRecord {
  const sorted = [...group].sort(
    (a, b) => Date.parse(a.createdAt) - Date.parse(b.createdAt),
  );
  const base = sorted[0];
  const externalIds = new Map<string, MediaRequestRecord["externalIds"][number]>();
  const requesters = new Map<string, MediaRequestRecord["requesters"][number]>();
  let updatedAt = base.updatedAt;

  for (const request of sorted) {
    if (Date.parse(request.updatedAt) > Date.parse(updatedAt)) {
      updatedAt = request.updatedAt;
    }
    for (const externalId of request.externalIds) {
      externalIds.set(externalIdKey(externalId.source, externalId.value), externalId);
    }
    for (const requester of request.requesters) {
      const existing = requesters.get(requester.userId);
      if (!existing || Date.parse(requester.requestedAt) < Date.parse(existing.requestedAt)) {
        requesters.set(requester.userId, requester);
      }
    }
  }

  return {
    ...base,
    externalIds: Array.from(externalIds.values()).sort((a, b) =>
      externalIdKey(a.source, a.value).localeCompare(externalIdKey(b.source, b.value)),
    ),
    requesters: Array.from(requesters.values()).sort(
      (a, b) => Date.parse(a.requestedAt) - Date.parse(b.requestedAt),
    ),
    updatedAt,
  };
}

function collapseMediaRequests(requests: MediaRequestRecord[]): MediaRequestRecord[] {
  const groups: MediaRequestRecord[][] = [];

  for (const request of requests) {
    const matchingIndexes = groups
      .map((group, index) => ({ group, index }))
      .filter(({ group }) => group.some((candidate) => requestsOverlap(candidate, request)))
      .map(({ index }) => index);

    if (matchingIndexes.length === 0) {
      groups.push([request]);
      continue;
    }

    const targetIndex = matchingIndexes[0];
    groups[targetIndex].push(request);
    for (const index of matchingIndexes.slice(1).reverse()) {
      groups[targetIndex].push(...groups[index]);
      groups.splice(index, 1);
    }
  }

  return groups
    .map(collapseRequestGroup)
    .sort((a, b) => Date.parse(b.updatedAt) - Date.parse(a.updatedAt));
}

const RECENT_ACTION_EVENT_WINDOW_MS = 10_000;

export function RequestsContainer({ facet }: RequestsContainerProps) {
  const client = useClient();
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const { user } = useAuth();
  const canManageAnyTitle = hasAnyLibraryPermission(user, LIBRARY_PERMISSIONS.manageTitles);
  const [mode, setMode] = React.useState<RequestsMode>(
    canManageAnyTitle ? "admin" : "mine",
  );
  const [statusFilter, setStatusFilter] = React.useState<RequestStatusFilter>(
    canManageAnyTitle ? "PENDING" : "all",
  );
  const [adminLibraries, setAdminLibraries] = React.useState<LibraryRecord[]>([]);
  const [requesterLibraries, setRequesterLibraries] = React.useState<LibraryRecord[]>([]);
  const [selectedLibraryIds, setSelectedLibraryIds] = React.useState<string[]>([]);
  const [requests, setRequests] = React.useState<MediaRequestRecord[]>([]);
  const [qualityProfileOptions, setQualityProfileOptions] = React.useState<
    QualityProfileOption[]
  >([]);
  const [loading, setLoading] = React.useState(false);
  const [actionRequestId, setActionRequestId] = React.useState<string | null>(null);
  const [claimsByRequestId, setClaimsByRequestId] = React.useState<
    Record<string, TitleClaimRecord[]>
  >({});
  const [claimsLoadingRequestId, setClaimsLoadingRequestId] = React.useState<
    string | null
  >(null);
  const [claimActionId, setClaimActionId] = React.useState<string | null>(null);
  /// Which request each loaded claim list belongs to, so a claim mutation can
  /// reload exactly the panel the operator is looking at.
  const claimRequestIdByClaimIdRef = React.useRef(new Map<string, string>());
  const refreshSeqRef = React.useRef(0);
  const librariesRef = React.useRef<LibraryRecord[]>([]);
  const adminLibrariesRef = React.useRef<LibraryRecord[]>([]);
  const requesterLibrariesRef = React.useRef<LibraryRecord[]>([]);
  const requestFacet = facet ?? null;
  const refreshContextKey = `${user?.id ?? ""}|${requestFacet ?? "all"}|${mode}|${statusFilter}`;
  const refreshContextRef = React.useRef(refreshContextKey);
  // Libraries only change with the viewer or the facet, so they are fetched
  // once per key and every other refresh (pulses, subscription events, filter
  // changes, post-action reloads) only re-reads the request list.
  const librariesKey = `${user?.id ?? ""}|${requestFacet ?? "all"}|${canManageAnyTitle}`;
  const loadedLibrariesKeyRef = React.useRef<string | null>(null);
  // Requests this container just acted on. The mutation handler already
  // refreshes the list, so the subscription echo for the same request is
  // redundant and skipped.
  const recentlyActedRequestIdsRef = React.useRef(new Map<string, number>());
  const libraries = mode === "admin" ? adminLibraries : requesterLibraries;
  const canShowAdminMode = adminLibraries.length > 0;
  const canShowRequesterMode = requesterLibraries.length > 0;

  React.useEffect(() => {
    librariesRef.current = libraries;
  }, [libraries]);

  React.useEffect(() => {
    refreshContextRef.current = refreshContextKey;
  }, [refreshContextKey]);

  React.useEffect(() => {
    setMode(canManageAnyTitle ? "admin" : "mine");
    setStatusFilter(canManageAnyTitle ? "PENDING" : "all");
    setAdminLibraries([]);
    setRequesterLibraries([]);
    adminLibrariesRef.current = [];
    requesterLibrariesRef.current = [];
    loadedLibrariesKeyRef.current = null;
    setRequests([]);
  }, [canManageAnyTitle, facet, user?.id]);

  const markRecentlyActed = React.useCallback((requestId: string) => {
    const now = Date.now();
    const recent = recentlyActedRequestIdsRef.current;
    for (const [id, actedAt] of recent) {
      if (now - actedAt > RECENT_ACTION_EVENT_WINDOW_MS) {
        recent.delete(id);
      }
    }
    recent.set(requestId, now);
  }, []);

  const wasRecentlyActed = React.useCallback((requestId: string) => {
    const actedAt = recentlyActedRequestIdsRef.current.get(requestId);
    if (actedAt === undefined) {
      return false;
    }
    if (Date.now() - actedAt > RECENT_ACTION_EVENT_WINDOW_MS) {
      recentlyActedRequestIdsRef.current.delete(requestId);
      return false;
    }
    return true;
  }, []);

  const refresh = React.useCallback(async (options?: { includeLibraries?: boolean }) => {
    const refreshSeq = ++refreshSeqRef.current;
    const refreshContext = refreshContextKey;
    const includeLibraries =
      options?.includeLibraries === true || loadedLibrariesKeyRef.current !== librariesKey;
    setLoading(true);
    try {
      let nextAdminLibraries = adminLibrariesRef.current;
      let nextRequesterLibraries = requesterLibrariesRef.current;
      if (includeLibraries) {
        const [adminLibrariesResult, requesterLibrariesResult] = await Promise.all([
          client.query(mediaRequestAdminLibrariesQuery, {
            facet: requestFacet,
          }).toPromise(),
          client.query(mediaRequestRequesterLibrariesQuery, {
            facet: requestFacet,
          }).toPromise(),
        ]);
        if (
          refreshSeq !== refreshSeqRef.current ||
          refreshContext !== refreshContextRef.current
        ) {
          return;
        }

        if (adminLibrariesResult.error || requesterLibrariesResult.error) {
          setGlobalStatus(
            adminLibrariesResult.error?.message ||
              requesterLibrariesResult.error?.message ||
              t("status.apiError"),
          );
          return;
        }

        nextAdminLibraries = (adminLibrariesResult.data?.libraries ?? []) as LibraryRecord[];
        nextRequesterLibraries = (requesterLibrariesResult.data?.libraries ?? []) as LibraryRecord[];
        adminLibrariesRef.current = nextAdminLibraries;
        requesterLibrariesRef.current = nextRequesterLibraries;
        loadedLibrariesKeyRef.current = librariesKey;
        setAdminLibraries(nextAdminLibraries);
        setRequesterLibraries(nextRequesterLibraries);
      }

      const nextMode =
        mode === "admin" && nextAdminLibraries.length === 0 && nextRequesterLibraries.length > 0
          ? "mine"
          : mode === "mine" && nextRequesterLibraries.length === 0 && nextAdminLibraries.length > 0
            ? "admin"
            : mode;
      if (nextMode !== mode) {
        setMode(nextMode);
        setSelectedLibraryIds([]);
        setRequests([]);
        return;
      }

      const nextLibraries = nextMode === "admin" ? nextAdminLibraries : nextRequesterLibraries;
      const normalizedSelectedLibraryIds = normalizeLibraryFilterSelection(
        selectedLibraryIds,
        nextLibraries,
      );
      if (!sameStringArray(normalizedSelectedLibraryIds, selectedLibraryIds)) {
        setSelectedLibraryIds(normalizedSelectedLibraryIds);
      }

      const requestsQuery = nextMode === "admin" ? mediaRequestsQuery : myMediaRequestsQuery;
      const requestStatus = statusFilter === "all" ? null : statusFilter;
      const requestsResult = await client.query(requestsQuery, {
        facet: requestFacet,
        libraryIds:
          normalizedSelectedLibraryIds.length > 0
            ? normalizedSelectedLibraryIds
            : null,
        status: requestStatus,
      }).toPromise();
      if (
        refreshSeq !== refreshSeqRef.current ||
        refreshContext !== refreshContextRef.current
      ) {
        return;
      }
      if (requestsResult.error) {
        setGlobalStatus(requestsResult.error.message || t("status.apiError"));
        return;
      }
      const loadedRequests =
        nextMode === "admin"
          ? requestsResult.data?.mediaRequests
          : requestsResult.data?.myMediaRequests;
      setRequests(
        nextMode === "admin"
          ? collapseMediaRequests((loadedRequests ?? []) as MediaRequestRecord[])
          : ((loadedRequests ?? []) as MediaRequestRecord[]),
      );
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.apiError"));
    } finally {
      if (
        refreshSeq === refreshSeqRef.current &&
        refreshContext === refreshContextRef.current
      ) {
        setLoading(false);
      }
    }
  }, [client, librariesKey, mode, refreshContextKey, requestFacet, selectedLibraryIds, setGlobalStatus, statusFilter, t]);

  const refreshQualityProfileOptions = React.useCallback(async () => {
    try {
      const qualityProfilesResult = await client
        .query(qualityProfileOptionsQuery, {})
        .toPromise();
      if (qualityProfilesResult.error) throw qualityProfilesResult.error;
      setQualityProfileOptions(
        (
          qualityProfilesResult.data?.qualityProfileSettings?.profiles ?? []
        ).map((profile: QualityProfileOption) => ({
          id: profile.id,
          name: profile.name || profile.id,
        })),
      );
    } catch (error) {
      setGlobalStatus(error instanceof Error ? error.message : t("status.apiError"));
    }
  }, [client, setGlobalStatus, t]);

  React.useEffect(() => {
    void refresh();
  }, [refresh]);

  useMediaRequestsSubscription((event) => {
    if (event?.requestId && wasRecentlyActed(event.requestId)) {
      return;
    }
    void refresh();
  });

  React.useEffect(() => {
    const handleNavigationBadgePulse = (event: Event) => {
      if (!(event instanceof CustomEvent)) {
        return;
      }
      const source = (event as CustomEvent<NavigationBadgesRefreshDetail>)
        .detail?.source;
      if (source === "poll" || source === "focus") {
        void refresh();
      }
    };

    window.addEventListener(
      NAVIGATION_BADGES_REFRESH_EVENT,
      handleNavigationBadgePulse,
    );
    return () => {
      window.removeEventListener(
        NAVIGATION_BADGES_REFRESH_EVENT,
        handleNavigationBadgePulse,
      );
    };
  }, [refresh]);

  React.useEffect(() => {
    setSelectedLibraryIds([]);
  }, [facet, mode]);

  const changeMode = React.useCallback((nextMode: RequestsMode) => {
    setMode(nextMode);
    setStatusFilter(nextMode === "admin" ? "PENDING" : "all");
  }, []);

  const approveRequest = React.useCallback(
    async (request: MediaRequestRecord, values: ApproveRequestValues) => {
      if (actionRequestId) {
        return;
      }

      setActionRequestId(request.id);
      markRecentlyActed(request.id);
      try {
        const { data, error } = await client
          .mutation(approveMediaRequestMutation, {
            input: {
              requestId: request.id,
              qualityProfileId: values.qualityProfileId,
              monitorType: values.monitorType ?? null,
              monitorSelection: values.monitorSelection ?? null,
              /// Exactly one of the two, or neither: the API refuses a day
              /// count and a forever flag together, because an approver who
              /// sent both has not said what they want. Neither means "keep
              /// what the requester asked for".
              leaseDays: values.leaseDays ?? null,
              leaseForever: values.leaseForever ?? null,
              tags: values.tags ?? null,
            },
          })
          .toPromise();
        if (error) throw error;
        const searchError = data?.approveMediaRequest?.searchError;
        /// The approval is deliberately not rolled back when the retention
        /// claim could not be written, so the failure is reported rather than
        /// swallowed: the title exists, and nothing is holding it.
        const claimError = data?.approveMediaRequest?.claimError;
        setGlobalStatus(
          claimError
            ? t("status.requestApprovedClaimFailed", {
                name: request.title,
                error: claimError,
              })
            : searchError
              ? t("status.requestApprovedSearchFailed", {
                  name: request.title,
                  error: searchError,
                })
              : t("status.requestApproved", { name: request.title }),
        );
        dispatchNavigationBadgesRefresh();
        await refresh();
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.apiError"));
      } finally {
        setActionRequestId(null);
      }
    },
    [actionRequestId, client, markRecentlyActed, refresh, setGlobalStatus, t],
  );

  const dismissRequest = React.useCallback(
    async (request: MediaRequestRecord) => {
      if (actionRequestId) {
        return;
      }

      setActionRequestId(request.id);
      markRecentlyActed(request.id);
      try {
        const { error } = await client
          .mutation(dismissMediaRequestMutation, { requestId: request.id })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.requestDismissed", { name: request.title }));
        dispatchNavigationBadgesRefresh();
        await refresh();
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.apiError"));
      } finally {
        setActionRequestId(null);
      }
    },
    [actionRequestId, client, markRecentlyActed, refresh, setGlobalStatus, t],
  );

  const updateRequest = React.useCallback(
    async (request: MediaRequestRecord, values: UpdateRequestValues) => {
      if (actionRequestId) {
        return;
      }

      setActionRequestId(request.id);
      markRecentlyActed(request.id);
      try {
        const { error } = await client
          .mutation(updateMyMediaRequestMutation, {
            input: {
              requestId: request.id,
              requestedQualityProfileId: values.requestedQualityProfileId,
              requestedMonitorType: values.requestedMonitorType ?? null,
              requestedMonitorSelection: values.requestedMonitorSelection ?? null,
            },
          })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.requestUpdated", { name: request.title }));
        dispatchNavigationBadgesRefresh();
        await refresh();
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.apiError"));
      } finally {
        setActionRequestId(null);
      }
    },
    [actionRequestId, client, markRecentlyActed, refresh, setGlobalStatus, t],
  );

  const cancelRequest = React.useCallback(
    async (request: MediaRequestRecord) => {
      if (actionRequestId) {
        return;
      }

      setActionRequestId(request.id);
      markRecentlyActed(request.id);
      try {
        const { error } = await client
          .mutation(cancelMyMediaRequestMutation, { requestId: request.id })
          .toPromise();
        if (error) throw error;
        setGlobalStatus(t("status.requestCanceled", { name: request.title }));
        dispatchNavigationBadgesRefresh();
        await refresh();
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.apiError"));
      } finally {
        setActionRequestId(null);
      }
    },
    [actionRequestId, client, markRecentlyActed, refresh, setGlobalStatus, t],
  );

  /// Read the holds on the title one request created. Only ever called when an
  /// operator opens a row's claims panel: a page of approved requests would
  /// otherwise be one query per row for a panel nobody looked at.
  const loadClaims = React.useCallback(
    async (request: MediaRequestRecord) => {
      const titleId = request.createdTitleId?.trim();
      if (!titleId) {
        setClaimsByRequestId((prev) => ({ ...prev, [request.id]: [] }));
        return;
      }
      setClaimsLoadingRequestId(request.id);
      try {
        const { data, error } = await client
          .query(titleClaimsQuery, { titleId }, { requestPolicy: "network-only" })
          .toPromise();
        if (error) throw error;
        const claims = (data?.titleClaims ?? []) as TitleClaimRecord[];
        for (const claim of claims) {
          claimRequestIdByClaimIdRef.current.set(claim.id, request.id);
        }
        setClaimsByRequestId((prev) => ({ ...prev, [request.id]: claims }));
      } catch (error) {
        setClaimsByRequestId((prev) => ({ ...prev, [request.id]: [] }));
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.apiError"),
        );
      } finally {
        setClaimsLoadingRequestId(null);
      }
    },
    [client, setGlobalStatus, t],
  );

  /// One shape for the three claim operations. Each reloads the panel it acted
  /// on rather than patching the row: converting a lease writes a *second*
  /// claim and leaves the first as history, so what the panel should now show
  /// is not derivable from the single claim the mutation returned.
  const runClaimAction = React.useCallback(
    async (
      claim: TitleClaimRecord,
      run: () => Promise<{ error?: unknown }>,
      successKey: string,
    ) => {
      if (claimActionId) {
        return;
      }
      setClaimActionId(claim.id);
      try {
        const { error } = await run();
        if (error) throw error;
        setGlobalStatus(t(successKey));
        const requestId = claimRequestIdByClaimIdRef.current.get(claim.id);
        const request = requestId
          ? requests.find((candidate) => candidate.id === requestId)
          : undefined;
        if (request) {
          await loadClaims(request);
        }
        await refresh();
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.apiError"),
        );
      } finally {
        setClaimActionId(null);
      }
    },
    [claimActionId, loadClaims, refresh, requests, setGlobalStatus, t],
  );

  const extendClaim = React.useCallback(
    (claim: TitleClaimRecord, expiresAt: string) =>
      runClaimAction(
        claim,
        () =>
          client
            .mutation(extendTitleClaimMutation, {
              input: { claimId: claim.id, expiresAt },
            })
            .toPromise(),
        "status.claimExtended",
      ),
    [client, runClaimAction],
  );

  const convertClaim = React.useCallback(
    (claim: TitleClaimRecord) =>
      runClaimAction(
        claim,
        () =>
          client
            .mutation(convertTitleClaimToPermanentMutation, {
              input: { claimId: claim.id },
            })
            .toPromise(),
        "status.claimMadePermanent",
      ),
    [client, runClaimAction],
  );

  const releaseClaim = React.useCallback(
    (claim: TitleClaimRecord, reason: string) =>
      runClaimAction(
        claim,
        () =>
          client
            .mutation(releaseTitleClaimMutation, {
              input: { claimId: claim.id, reason },
            })
            .toPromise(),
        "status.claimReleased",
      ),
    [client, runClaimAction],
  );

  return (
    <RequestsView
      mode={mode}
      canShowAdminMode={canShowAdminMode}
      canShowRequesterMode={canShowRequesterMode}
      onModeChange={changeMode}
      statusFilter={statusFilter}
      onStatusFilterChange={setStatusFilter}
      libraries={libraries}
      selectedLibraryIds={selectedLibraryIds}
      onSelectedLibraryIdsChange={setSelectedLibraryIds}
      requests={requests}
      qualityProfileOptions={qualityProfileOptions}
      loading={loading}
      actionRequestId={actionRequestId}
      onRefresh={() => void refresh({ includeLibraries: true })}
      onLoadQualityProfileOptions={() => void refreshQualityProfileOptions()}
      onApprove={(request, values) =>
        void approveRequest(request, values)
      }
      onDismiss={(request) => void dismissRequest(request)}
      onUpdateRequest={(request, values) => void updateRequest(request, values)}
      onCancelRequest={(request) => void cancelRequest(request)}
      claimsByRequestId={claimsByRequestId}
      claimsLoadingRequestId={claimsLoadingRequestId}
      onLoadClaims={(request) => void loadClaims(request)}
      onExtendClaim={(claim, expiresAt) => void extendClaim(claim, expiresAt)}
      onConvertClaim={(claim) => void convertClaim(claim)}
      onReleaseClaim={(claim, reason) => void releaseClaim(claim, reason)}
      claimActionId={claimActionId}
    />
  );
}

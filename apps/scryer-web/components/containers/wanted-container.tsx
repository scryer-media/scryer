import { memo, useCallback, useEffect, useRef, useState } from "react";
import type { OverviewTitleTarget, ViewId, WantedSection } from "@/components/root/types";
import { useClient, useMutation } from "urql";
import { WantedView } from "@/components/views/wanted-view";
import type { CutoffUnmetItem } from "@/components/views/cutoff-unmet-view";
import {
  acquisitionSearchJobQuery,
  cutoffUnmetTitlesPageQuery,
  librariesQuery,
  pendingReleasesQuery,
  releaseDecisionsQuery,
  wantedItemsQuery,
  wantedNavigationCountsQuery,
} from "@/lib/graphql/queries";
import { runIterativeReleaseSearch } from "@/lib/graphql/release-search";
import {
  cancelAcquisitionSearchMutation,
  triggerAcquisitionSearchMutation,
  triggerTitleMismatchRecoverySearchMutation,
  pauseWantedItemMutation,
  resumeWantedItemMutation,
  queueBestReleaseMutation,
  queueReplacementMutation,
  forceGrabPendingReleaseMutation,
  dismissPendingReleaseMutation,
} from "@/lib/graphql/mutations";
import type {
  AcquisitionSearchJob,
  PendingReleaseItem,
  LibraryRecord,
  Release,
  ReleaseDecisionItem,
  TitleRecord,
  WantedItem,
} from "@/lib/types";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { useDownloadConflictConfirmation } from "@/components/common/download-conflict-confirmation";
import {
  assertNoReplaceConflict,
  retryWithReplaceOnConflict,
} from "@/lib/utils/download-conflicts";
import {
  normalizeLibraryFilterSelection,
  selectedLibraryIdsToQueryValue,
} from "@/lib/utils/library-filter";
import { userFacingGraphQlErrorMessage } from "@/lib/graphql/error-message";
import { autoSearchOutcomeMessage } from "@/lib/utils/auto-search-outcome";
import { releaseQueueScopeInput } from "@/lib/utils/release-queue-scope";

type WantedContainerProps = {
  wantedSection: WantedSection;
  onOpenOverview?: (
    targetView: ViewId,
    overviewTarget: OverviewTitleTarget,
    episodeId?: string,
  ) => void;
};

const PENDING_RELEASE_PAGE_SIZE = 300;
const CUTOFF_PAGE_SIZE = 50;
// The interactive acquisition-search job runs server-side; its id
// is kept in sessionStorage so progress survives navigation and reload.
const ACQUISITION_SEARCH_JOB_STORAGE_KEY = "scryer.acquisitionSearchJobId";
const ACQUISITION_SEARCH_POLL_INTERVAL_MS = 2_000;

function storedAcquisitionSearchJobId(): string | null {
  try {
    return window.sessionStorage.getItem(ACQUISITION_SEARCH_JOB_STORAGE_KEY);
  } catch {
    return null;
  }
}

function storeAcquisitionSearchJobId(id: string | null) {
  try {
    if (id) {
      window.sessionStorage.setItem(ACQUISITION_SEARCH_JOB_STORAGE_KEY, id);
    } else {
      window.sessionStorage.removeItem(ACQUISITION_SEARCH_JOB_STORAGE_KEY);
    }
  } catch {
    // sessionStorage unavailable — progress simply will not survive reloads.
  }
}

function cutoffItemKey(item: CutoffUnmetItem) {
  return item.episodeId?.trim() || item.titleId;
}

function cutoffItemEpisodeCode(item: CutoffUnmetItem): string | null {
  const seasonDigits = item.seasonNumber?.match(/\d+/)?.[0] ?? null;
  const episodeDigits = item.episodeNumber?.match(/\d+/)?.[0] ?? null;
  if (!seasonDigits || !episodeDigits) {
    return null;
  }
  return `S${seasonDigits.padStart(2, "0")}E${episodeDigits.padStart(2, "0")}`;
}

function cutoffItemLabel(item: CutoffUnmetItem) {
  const episodeCode = cutoffItemEpisodeCode(item);
  return episodeCode ? `${item.titleName} ${episodeCode}` : item.titleName;
}

function cutoffConflictMessage(item: CutoffUnmetItem) {
  return item.episodeId
    ? "A download is already in progress for this episode."
    : "A download is already in progress for this title.";
}

function cutoffQueueScope(item: CutoffUnmetItem) {
  return item.episodeId?.trim() ? { episode: item.episodeId.trim() } : { title: true };
}

export const WantedContainer = memo(function WantedContainer({
  wantedSection,
  onOpenOverview,
}: WantedContainerProps) {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();
  const { confirmReplaceConflict, replaceConflictDialog } =
    useDownloadConflictConfirmation();

  // --- Wanted items state ---
  const [items, setItems] = useState<WantedItem[]>([]);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(false);
  const [selectedTitle, setSelectedTitle] = useState<TitleRecord | null>(null);
  const [libraries, setLibraries] = useState<LibraryRecord[]>([]);
  const [librariesLoading, setLibrariesLoading] = useState(false);
  const [selectedLibraryIds, setSelectedLibraryIds] = useState<string[]>([]);
  const [offset, setOffset] = useState(0);
  const limit = 50;

  const [expandedItemId, setExpandedItemId] = useState<string | null>(null);
  const [decisions, setDecisions] = useState<ReleaseDecisionItem[]>([]);
  const [decisionsLoading, setDecisionsLoading] = useState(false);
  const [standbyReleases, setStandbyReleases] = useState<PendingReleaseItem[]>([]);
  const [standbyLoading, setStandbyLoading] = useState(false);

  const [, executeTriggerAcquisitionSearch] = useMutation(triggerAcquisitionSearchMutation);
  const [, executeCancelAcquisitionSearch] = useMutation(cancelAcquisitionSearchMutation);
  const [, executePause] = useMutation(pauseWantedItemMutation);
  const [, executeResume] = useMutation(resumeWantedItemMutation);
  const [, executeMismatchRecovery] = useMutation(triggerTitleMismatchRecoverySearchMutation);

  // --- Cutoff (Upgrades) state ---
  const [cutoffItems, setCutoffItems] = useState<CutoffUnmetItem[]>([]);
  const [cutoffTotal, setCutoffTotal] = useState(0);
  const [cutoffOffset, setCutoffOffset] = useState(0);
  const [cutoffLoading, setCutoffLoading] = useState(false);
  const [cutoffFacetFilter, setCutoffFacetFilter] = useState<string | undefined>(undefined);
  const [cutoffAutoSearchingId, setCutoffAutoSearchingId] = useState<string | null>(null);
  const [cutoffInteractiveSearchingId, setCutoffInteractiveSearchingId] = useState<string | null>(null);
  const [cutoffActiveInteractiveItemId, setCutoffActiveInteractiveItemId] = useState<string | null>(null);
  const [cutoffSearchResultsByItemId, setCutoffSearchResultsByItemId] = useState<
    Record<string, Release[]>
  >({});

  // --- Interactive acquisition-search job ---
  const [searchJob, setSearchJob] = useState<AcquisitionSearchJob | null>(null);
  const [searchJobStarting, setSearchJobStarting] = useState(false);
  const searchJobIdRef = useRef<string | null>(null);

  // --- Pending releases state ---
  const [pendingItems, setPendingItems] = useState<PendingReleaseItem[]>([]);
  const [pendingTotal, setPendingTotal] = useState(0);
  const [pendingHasMore, setPendingHasMore] = useState(false);
  const [pendingNextOffset, setPendingNextOffset] = useState(0);
  const [pendingLoading, setPendingLoading] = useState(false);
  const [pendingLoadingMore, setPendingLoadingMore] = useState(false);
  const pendingLoadInFlightRef = useRef(false);
  const [, executeForceGrab] = useMutation(forceGrabPendingReleaseMutation);
  const [, executeDismiss] = useMutation(dismissPendingReleaseMutation);

  const refreshWantedNavigationCounts = useCallback(async () => {
    try {
      const { data, error } = await client
        .query(
          wantedNavigationCountsQuery,
          {
            libraryIds: selectedLibraryIdsToQueryValue(selectedLibraryIds),
            titleSearch: selectedTitle?.name?.trim() || null,
            cutoffFacet: cutoffFacetFilter ?? null,
          },
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (error) throw error;

      setTotal(Number(data?.wantedItems?.totalCount ?? 0));
      setCutoffTotal(Number(data?.cutoffUnmetTitlesPage?.totalCount ?? 0));
      setPendingTotal(Number(data?.pendingReleases?.totalCount ?? 0));
    } catch (error) {
      console.warn("Failed to refresh wanted navigation counts", error);
    }
  }, [client, cutoffFacetFilter, selectedLibraryIds, selectedTitle]);

  useEffect(() => {
    void refreshWantedNavigationCounts();
  }, [refreshWantedNavigationCounts]);

  const refreshPending = useCallback(async () => {
    pendingLoadInFlightRef.current = false;
    setPendingLoading(true);
    try {
      const { data, error } = await client
        .query(
          pendingReleasesQuery,
          { filter: null, limit: PENDING_RELEASE_PAGE_SIZE, offset: 0 },
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (error) throw error;
      const page = data?.pendingReleases ?? {};
      const nextItems = (page.items ?? []) as PendingReleaseItem[];
      setPendingItems(nextItems);
      setPendingTotal(typeof page.totalCount === "number" ? page.totalCount : nextItems.length);
      setPendingHasMore(Boolean(page.hasMore));
      setPendingNextOffset(nextItems.length);
    } catch (error) {
      const message = error instanceof Error ? error.message : t("status.failedToLoad");
      setGlobalStatus(message);
    } finally {
      setPendingLoading(false);
    }
  }, [client, t, setGlobalStatus]);

  const loadMorePending = useCallback(async () => {
    if (!pendingHasMore || pendingLoadingMore || pendingLoadInFlightRef.current) {
      return;
    }

    pendingLoadInFlightRef.current = true;
    setPendingLoadingMore(true);
    try {
      const { data, error } = await client
        .query(
          pendingReleasesQuery,
          {
            filter: null,
            limit: PENDING_RELEASE_PAGE_SIZE,
            offset: pendingNextOffset,
          },
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (error) throw error;

      const page = data?.pendingReleases ?? {};
      const nextItems = (page.items ?? []) as PendingReleaseItem[];
      setPendingItems((current) => {
        const seen = new Set(current.map((item) => item.id));
        return [...current, ...nextItems.filter((item) => !seen.has(item.id))];
      });
      setPendingTotal(typeof page.totalCount === "number" ? page.totalCount : pendingTotal);
      setPendingHasMore(Boolean(page.hasMore));
      setPendingNextOffset(pendingNextOffset + nextItems.length);
    } catch (error) {
      const message = error instanceof Error ? error.message : t("status.failedToLoad");
      setGlobalStatus(message);
    } finally {
      pendingLoadInFlightRef.current = false;
      setPendingLoadingMore(false);
    }
  }, [
    client,
    pendingHasMore,
    pendingLoadingMore,
    pendingNextOffset,
    pendingTotal,
    setGlobalStatus,
    t,
  ]);

  useEffect(() => {
    if (wantedSection === "pending") {
      void refreshPending();
    }
  }, [refreshPending, wantedSection]);

  const forceGrabPending = useCallback(
    async (id: string) => {
      const { data, error } = await executeForceGrab({ id });
      if (error) {
        setGlobalStatus(error.message);
      } else if (data?.forceGrabPendingRelease?.grabbed === false) {
        setGlobalStatus(t("pending.grabRejected"));
        void refreshPending();
      } else {
        setGlobalStatus(t("pending.grabbed"));
        void refreshPending();
      }
    },
    [executeForceGrab, refreshPending, setGlobalStatus, t],
  );

  const dismissPending = useCallback(
    async (id: string) => {
      const { error } = await executeDismiss({ id });
      if (error) {
        setGlobalStatus(error.message);
      } else {
        setGlobalStatus(t("pending.dismissed"));
        void refreshPending();
      }
    },
    [executeDismiss, refreshPending, setGlobalStatus, t],
  );

  // --- Shared library filters ---

  useEffect(() => {
    if (wantedSection !== "wanted" && wantedSection !== "cutoff") {
      return;
    }
    let cancelled = false;
    setLibrariesLoading(true);
    void client
      .query(
        librariesQuery,
        { facet: null, permission: "VIEW" },
        { requestPolicy: "network-only" },
      )
      .toPromise()
      .then(({ data, error }) => {
        if (cancelled) {
          return;
        }
        if (error) {
          throw error;
        }
        const nextLibraries = (data?.libraries ?? []) as LibraryRecord[];
        setLibraries(nextLibraries);
        setSelectedLibraryIds((current) =>
          normalizeLibraryFilterSelection(current, nextLibraries),
        );
      })
      .catch((error) => {
        if (!cancelled) {
          setGlobalStatus(error instanceof Error ? error.message : t("status.failedToLoad"));
        }
      })
      .finally(() => {
        if (!cancelled) {
          setLibrariesLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [client, setGlobalStatus, t, wantedSection]);

  const refreshItems = useCallback(async () => {
    setLoading(true);
    try {
      // The derived Missing view: the state-row status/media-type
      // filters are gone; the title picker narrows via the name-based titleSearch.
      const { data, error } = await client
        .query(wantedItemsQuery, {
          wantedKind: "MISSING",
          facet: null,
          libraryIds: selectedLibraryIdsToQueryValue(selectedLibraryIds),
          titleSearch: selectedTitle?.name?.trim() || null,
          limit,
          offset,
        })
        .toPromise();
      if (error) throw error;
      setItems(data?.wantedItems?.items ?? []);
      setTotal(data?.wantedItems?.totalCount ?? 0);
    } catch (error) {
      const message = error instanceof Error ? error.message : t("status.failedToLoad");
      setGlobalStatus(message);
    } finally {
      setLoading(false);
    }
  }, [client, selectedTitle, selectedLibraryIds, offset, t, setGlobalStatus]);

  useEffect(() => {
    if (wantedSection === "wanted") {
      void refreshItems();
    }
  }, [refreshItems, wantedSection]);

  const handleSelectedTitleChange = useCallback((title: TitleRecord | null) => {
    setOffset(0);
    setSelectedTitle(title);
  }, []);

  // --- Cutoff data fetching ---

  const refreshCutoff = useCallback(async () => {
    setCutoffLoading(true);
    try {
      const { data, error } = await client
        .query(cutoffUnmetTitlesPageQuery, {
          facet: cutoffFacetFilter ?? null,
          libraryIds: selectedLibraryIdsToQueryValue(selectedLibraryIds),
          limit: CUTOFF_PAGE_SIZE,
          offset: cutoffOffset,
        })
        .toPromise();
      if (error) throw error;
      setCutoffItems(data?.cutoffUnmetTitlesPage?.items ?? []);
      setCutoffTotal(data?.cutoffUnmetTitlesPage?.totalCount ?? 0);
    } catch (error) {
      const message = error instanceof Error ? error.message : t("status.failedToLoad");
      setGlobalStatus(message);
    } finally {
      setCutoffLoading(false);
    }
  }, [client, cutoffFacetFilter, cutoffOffset, selectedLibraryIds, t, setGlobalStatus]);

  useEffect(() => {
    if (wantedSection === "cutoff") {
      void refreshCutoff();
    }
  }, [refreshCutoff, wantedSection]);

  // --- Wanted actions ---

  const loadItemDetails = useCallback(
    async (wantedItemId: string, standbyCount: number) => {
      if (expandedItemId === wantedItemId) {
        setExpandedItemId(null);
        return;
      }
      setExpandedItemId(wantedItemId);
      setDecisionsLoading(true);
      setStandbyLoading(standbyCount > 0);
      try {
        const [decisionsResult, standbyResult] = await Promise.all([
          client.query(releaseDecisionsQuery, { wantedItemId, limit: 20 }).toPromise(),
          standbyCount > 0
            ? client
                .query(pendingReleasesQuery, {
                  filter: { wantedItemId, statuses: ["STANDBY"] },
                  limit: Math.min(standbyCount, 300),
                  offset: 0,
                })
                .toPromise()
            : Promise.resolve({ data: null, error: null }),
        ]);
        if (decisionsResult.error) throw decisionsResult.error;
        if (standbyResult.error) throw standbyResult.error;
        setDecisions(decisionsResult.data?.wantedItem?.releaseDecisions?.items ?? []);
        setStandbyReleases(standbyResult.data?.pendingReleases?.items ?? []);
      } catch {
        setDecisions([]);
        setStandbyReleases([]);
      } finally {
        setDecisionsLoading(false);
        setStandbyLoading(false);
      }
    },
    [client, expandedItemId],
  );

  // --- Interactive acquisition-search job ---

  const applySearchJobSnapshot = useCallback(
    (job: AcquisitionSearchJob | null) => {
      setSearchJob(job);
      if (!job || job.state !== "RUNNING") {
        searchJobIdRef.current = null;
        storeAcquisitionSearchJobId(null);
      }
      if (job && job.state !== "RUNNING") {
        setGlobalStatus(
          job.state === "CANCELLED"
            ? t("wanted.searchJobCancelled", {
                processed: job.processed,
                grabbed: job.grabbedCount,
              })
            : t("wanted.searchJobComplete", {
                processed: job.processed,
                grabbed: job.grabbedCount,
                failed: job.failedCount,
              }),
        );
      }
    },
    [setGlobalStatus, t],
  );

  const startAcquisitionSearch = useCallback(
    async (input: {
      wantedKind?: "MISSING" | "CUTOFF_UPGRADE";
      facet?: string | null;
      libraryIds?: string[] | null;
      wantedItemId?: string;
    }) => {
      setSearchJobStarting(true);
      try {
        const { data, error } = await executeTriggerAcquisitionSearch({ input });
        if (error) throw error;
        const job = data?.triggerAcquisitionSearch as AcquisitionSearchJob | undefined;
        if (job) {
          searchJobIdRef.current = job.id;
          storeAcquisitionSearchJobId(job.id);
          setSearchJob(job);
          setGlobalStatus(t("wanted.searchJobStarted"));
        }
      } catch (error) {
        setGlobalStatus(userFacingGraphQlErrorMessage(error, t("status.queueFailed")));
      } finally {
        setSearchJobStarting(false);
      }
    },
    [executeTriggerAcquisitionSearch, setGlobalStatus, t],
  );

  // Poll the running job (2s) so progress survives navigation; the id is
  // rehydrated from sessionStorage on mount.
  useEffect(() => {
    const activeId = searchJob?.state === "RUNNING" ? searchJob.id : null;
    if (!activeId) {
      return;
    }
    let cancelled = false;
    const poll = async () => {
      try {
        const { data, error } = await client
          .query(
            acquisitionSearchJobQuery,
            { id: activeId },
            { requestPolicy: "network-only" },
          )
          .toPromise();
        if (cancelled) return;
        if (error) throw error;
        const job = (data?.acquisitionSearchJob ?? null) as AcquisitionSearchJob | null;
        applySearchJobSnapshot(job);
        if (job && job.state !== "RUNNING") {
          void refreshItems();
          void refreshCutoff();
        }
      } catch {
        // transient poll failure — keep polling
      }
    };
    const interval = window.setInterval(() => void poll(), ACQUISITION_SEARCH_POLL_INTERVAL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(interval);
    };
  }, [applySearchJobSnapshot, client, refreshCutoff, refreshItems, searchJob?.id, searchJob?.state]);

  useEffect(() => {
    const storedId = storedAcquisitionSearchJobId();
    if (!storedId || searchJobIdRef.current === storedId) {
      return;
    }
    searchJobIdRef.current = storedId;
    void client
      .query(acquisitionSearchJobQuery, { id: storedId }, { requestPolicy: "network-only" })
      .toPromise()
      .then(({ data }) => {
        const job = (data?.acquisitionSearchJob ?? null) as AcquisitionSearchJob | null;
        if (job && job.state === "RUNNING") {
          setSearchJob(job);
        } else {
          searchJobIdRef.current = null;
          storeAcquisitionSearchJobId(null);
        }
      })
      .catch(() => {
        searchJobIdRef.current = null;
        storeAcquisitionSearchJobId(null);
      });
  }, [client]);

  const cancelAcquisitionSearch = useCallback(async () => {
    const id = searchJob?.id ?? searchJobIdRef.current;
    if (!id) {
      return;
    }
    const { error } = await executeCancelAcquisitionSearch({ id });
    if (error) {
      setGlobalStatus(error.message);
    }
  }, [executeCancelAcquisitionSearch, searchJob?.id, setGlobalStatus]);

  // Per-item "Search now": one-scope interactive job — the id is the scope
  // identity (state-row id or convergence scope key), resolved server-side.
  const triggerSearch = useCallback(
    async (id: string) => {
      await startAcquisitionSearch({ wantedItemId: id });
    },
    [startAcquisitionSearch],
  );

  const pauseItem = useCallback(
    async (id: string) => {
      const { error } = await executePause({ id });
      if (error) {
        setGlobalStatus(error.message);
      } else {
        void refreshItems();
      }
    },
    [executePause, refreshItems, setGlobalStatus],
  );

  const resumeItem = useCallback(
    async (id: string) => {
      const { error } = await executeResume({ id });
      if (error) {
        setGlobalStatus(error.message);
      } else {
        void refreshItems();
      }
    },
    [executeResume, refreshItems, setGlobalStatus],
  );

  const triggerMismatchRecovery = useCallback(
    async (titleId: string) => {
      const { data, error } = await executeMismatchRecovery({ titleId });
      if (error) {
        setGlobalStatus(error.message);
      } else {
        setGlobalStatus(
          t("status.mismatchRecoveryQueued", {
            count: data?.triggerTitleMismatchRecoverySearch?.queuedCount ?? 0,
          }),
        );
        void refreshItems();
      }
    },
    [executeMismatchRecovery, refreshItems, setGlobalStatus, t],
  );

  // --- Cutoff search actions ---

  const searchAndQueueCutoffItem = useCallback(
    async (
      cutoffItem: CutoffUnmetItem,
      options: { allowReplaceConfirmation?: boolean } = {},
    ) => {
      const input = {
        titleId: cutoffItem.titleId,
        scope: cutoffQueueScope(cutoffItem),
      };
      const submit = async (nextInput: typeof input & { replaceInProgress?: boolean }) => {
        const { data, error } = await client
          .mutation(queueBestReleaseMutation, { input: nextInput })
          .toPromise();
        if (error) throw error;
        return data?.queueBestRelease;
      };
      const payload = options.allowReplaceConfirmation
        ? await retryWithReplaceOnConflict(
            input,
            submit,
            cutoffConflictMessage(cutoffItem),
            confirmReplaceConflict,
          )
        : await submit(input);
      assertNoReplaceConflict(payload, cutoffConflictMessage(cutoffItem));
      setGlobalStatus(t("cutoff.searchTriggered", { name: cutoffItemLabel(cutoffItem) }));
    },
    [client, confirmReplaceConflict, t, setGlobalStatus],
  );

  const cutoffTriggerAutoSearch = useCallback(
    async (item: CutoffUnmetItem) => {
      const itemKey = cutoffItemKey(item);
      setCutoffAutoSearchingId(itemKey);
      try {
        await searchAndQueueCutoffItem(item, { allowReplaceConfirmation: true });
      } catch (error) {
        setGlobalStatus(
          autoSearchOutcomeMessage(error, t, cutoffItemLabel(item)) ??
            userFacingGraphQlErrorMessage(error, t("status.queueFailed")),
        );
      } finally {
        setCutoffAutoSearchingId(null);
      }
    },
    [searchAndQueueCutoffItem, setGlobalStatus, t],
  );

  const cutoffTriggerInteractiveSearch = useCallback(
    async (item: CutoffUnmetItem) => {
      const itemKey = cutoffItemKey(item);
      setCutoffInteractiveSearchingId(itemKey);
      // Stream partial results into the picker as indexers complete; the
      // picker opens on the first non-empty partial, the toast at completion.
      const onUpdate = (snapshot: { releases: Release[] }) => {
        setCutoffSearchResultsByItemId((current) => ({
          ...current,
          [itemKey]: snapshot.releases,
        }));
        if (snapshot.releases.length > 0) {
          setCutoffActiveInteractiveItemId(itemKey);
        }
      };
      try {
        if (item.episodeId) {
          const season = item.seasonNumber?.trim();
          const episode = item.episodeNumber?.trim();
          if (!season || !episode) {
            throw new Error("Episode search is unavailable because the episode numbers are missing.");
          }
          const results = await runIterativeReleaseSearch(client, {
            titleId: item.titleId,
            season,
            episode,
          }, { onUpdate });
          setCutoffSearchResultsByItemId((current) => ({ ...current, [itemKey]: results }));
          setCutoffActiveInteractiveItemId(itemKey);
          setGlobalStatus(t("status.foundNzb", { count: results.length }));
        } else {
          const results = await runIterativeReleaseSearch(
            client,
            { titleId: item.titleId },
            { onUpdate },
          );
          setCutoffSearchResultsByItemId((current) => ({ ...current, [itemKey]: results }));
          setCutoffActiveInteractiveItemId(itemKey);
          setGlobalStatus(t("status.foundNzb", { count: results.length }));
        }
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.apiError"));
      } finally {
        setCutoffInteractiveSearchingId(null);
      }
    },
    [client, setGlobalStatus, t],
  );

  const cutoffQueueRelease = useCallback(
    async (item: CutoffUnmetItem, release: Release) => {
      if (!release.candidateToken) {
        setGlobalStatus(t("status.releaseMissingCandidateToken"));
        return;
      }

      const conflictMessage = cutoffConflictMessage(item);
      const input = {
        titleId: item.titleId,
        scope: releaseQueueScopeInput(release, cutoffQueueScope(item)),
        candidateToken: release.candidateToken,
        sizeBytes: release.sizeBytes ?? null,
      };

      try {
        const payload = await retryWithReplaceOnConflict(
          input,
          async (nextInput) => {
            const { data, error } = await client
              .mutation(queueReplacementMutation, { input: nextInput })
              .toPromise();
            if (error) throw error;
            return data?.queueReplacementRelease;
          },
          conflictMessage,
          confirmReplaceConflict,
        );
        assertNoReplaceConflict(payload, conflictMessage);
        setGlobalStatus(t("status.queueSuccess", { name: release.title }));
      } catch (error) {
        setGlobalStatus(userFacingGraphQlErrorMessage(error, t("status.queueFailed")));
      }
    },
    [client, confirmReplaceConflict, setGlobalStatus, t],
  );

  // "Search All" is one server job over the filtered Upgrades scope set
  // — progress/cancel survive navigation and reload.
  const cutoffBulkSearch = useCallback(() => {
    void startAcquisitionSearch({
      wantedKind: "CUTOFF_UPGRADE",
      facet: cutoffFacetFilter ?? null,
      libraryIds: selectedLibraryIdsToQueryValue(selectedLibraryIds),
    });
  }, [cutoffFacetFilter, selectedLibraryIds, startAcquisitionSearch]);

  const handleCutoffFacetFilterChange = useCallback((value: string | undefined) => {
    setCutoffOffset(0);
    setCutoffFacetFilter(value);
  }, []);

  const handleCutoffLibraryIdsChange = useCallback((libraryIds: string[]) => {
    setCutoffOffset(0);
    setSelectedLibraryIds(libraryIds);
  }, []);

  return (
    <>
      <div className="flex h-full min-h-0 flex-col">
      <WantedView
        section={wantedSection}
        onOpenOverview={onOpenOverview}
        wantedState={{
          items,
          total,
          loading,
          selectedTitle,
          setSelectedTitle: handleSelectedTitleChange,
          libraries,
          librariesLoading,
          selectedLibraryIds,
          setSelectedLibraryIds,
          offset,
          setOffset,
          limit,
          refreshItems,
          expandedItemId,
          decisions,
          decisionsLoading,
          standbyReleases,
          standbyLoading,
          loadItemDetails,
          triggerSearch,
          pauseItem,
          resumeItem,
          triggerMismatchRecovery,
        }}
        cutoffState={{
          items: cutoffItems,
          total: cutoffTotal,
          offset: cutoffOffset,
          setOffset: setCutoffOffset,
          limit: CUTOFF_PAGE_SIZE,
          loading: cutoffLoading,
          facetFilter: cutoffFacetFilter,
          setFacetFilter: handleCutoffFacetFilterChange,
          libraries,
          librariesLoading,
          selectedLibraryIds,
          setSelectedLibraryIds: handleCutoffLibraryIdsChange,
          autoSearchingId: cutoffAutoSearchingId,
          interactiveSearchingId: cutoffInteractiveSearchingId,
          activeInteractiveItemId: cutoffActiveInteractiveItemId,
          searchResultsByItemId: cutoffSearchResultsByItemId,
          searchJob,
          searchJobStarting,
          triggerBulkSearch: cutoffBulkSearch,
          cancelBulkSearch: cancelAcquisitionSearch,
          triggerAutoSearch: cutoffTriggerAutoSearch,
          triggerInteractiveSearch: cutoffTriggerInteractiveSearch,
          queueRelease: cutoffQueueRelease,
        }}
        pendingState={{
          items: pendingItems,
          total: pendingTotal,
          loading: pendingLoading,
          hasMore: pendingHasMore,
          loadingMore: pendingLoadingMore,
          refreshItems: refreshPending,
          loadMoreItems: loadMorePending,
          forceGrab: forceGrabPending,
          dismiss: dismissPending,
        }}
      />
      </div>
      {replaceConflictDialog}
    </>
  );
});

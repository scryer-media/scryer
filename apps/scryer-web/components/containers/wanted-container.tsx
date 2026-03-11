import { memo, useCallback, useEffect, useRef, useState } from "react";
import { useClient, useMutation } from "urql";
import { WantedView } from "@/components/views/wanted-view";
import type { CutoffUnmetItem } from "@/components/views/cutoff-unmet-view";
import {
  wantedItemsQuery,
  releaseDecisionsQuery,
  titlesQuery,
  adminSettingsQuery,
  searchQuery,
  calendarEpisodesQuery,
  pendingReleasesQuery,
} from "@/lib/graphql/queries";
import {
  triggerWantedSearchMutation,
  pauseWantedItemMutation,
  resumeWantedItemMutation,
  resetWantedItemMutation,
  queueExistingMutation,
  forceGrabPendingReleaseMutation,
  dismissPendingReleaseMutation,
} from "@/lib/graphql/mutations";
import { resolveQualityProfileCatalogState } from "@/lib/utils/quality-profiles";
import {
  QUALITY_PROFILE_CATALOG_KEY,
  QUALITY_PROFILE_ID_KEY,
  QUALITY_PROFILE_INHERIT_VALUE,
} from "@/lib/constants/settings";
import type { WantedItem, ReleaseDecisionItem, TitleRecord, Release, PendingReleaseItem } from "@/lib/types";
import type { ParsedQualityProfileEntry } from "@/lib/types/quality-profiles";
import { FACETS_BY_ID } from "@/lib/facets/registry";
import type { ViewId } from "@/components/root/types";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";

export type WantedTab = "wanted" | "cutoff" | "calendar" | "pending";

type WantedContainerProps = {
  onOpenOverview?: (targetView: ViewId, titleId: string, episodeId?: string) => void;
};

type AdminSettingsItem = {
  keyName: string;
  effectiveValueJson: string | null;
  scopeId: string | null;
};

function extractSettingValue(items: AdminSettingsItem[], key: string, scopeId?: string): string | null {
  const match = items.find(
    (i) => i.keyName === key && (scopeId ? i.scopeId === scopeId : true),
  );
  return match?.effectiveValueJson ?? null;
}

function computeCutoffUnmetItems(
  titles: TitleRecord[],
  profileEntries: ParsedQualityProfileEntry[],
  profileIdByScope: Record<string, string>,
  globalProfileId: string | null,
): CutoffUnmetItem[] {
  const profileMap = new Map(profileEntries.map((e) => [e.id, e]));

  const resolveProfile = (scopeId: string): ParsedQualityProfileEntry | null => {
    const scopeProfileId = profileIdByScope[scopeId];
    if (scopeProfileId && scopeProfileId !== QUALITY_PROFILE_INHERIT_VALUE) {
      const p = profileMap.get(scopeProfileId);
      if (p) return p;
    }
    if (globalProfileId && globalProfileId !== QUALITY_PROFILE_INHERIT_VALUE) {
      return profileMap.get(globalProfileId) ?? null;
    }
    return profileEntries[0] ?? null;
  };

  const result: CutoffUnmetItem[] = [];

  for (const title of titles) {
    if (!title.monitored || !title.qualityTier) continue;

    const scopeId = title.facet === "movie" ? "movie" : title.facet === "tv" ? "series" : "anime";
    const profile = resolveProfile(scopeId);
    if (!profile || !profile.criteria.allow_upgrades) continue;

    const tiers = profile.criteria.quality_tiers;
    if (tiers.length === 0) continue;

    const targetTier = tiers[0];
    const currentIndex = tiers.findIndex(
      (t) => t.toUpperCase() === title.qualityTier!.toUpperCase(),
    );

    // Already at best tier
    if (currentIndex === 0) continue;

    result.push({
      id: title.id,
      name: title.name,
      facet: title.facet,
      posterUrl: title.posterUrl,
      currentTier: title.qualityTier,
      targetTier,
    });
  }

  return result;
}

const VALID_TABS = new Set<WantedTab>(["wanted", "cutoff", "calendar", "pending"]);

function readTabFromUrl(): WantedTab {
  if (typeof window === "undefined") return "wanted";
  const params = new URLSearchParams(window.location.search);
  const t = params.get("tab") as WantedTab | null;
  return t && VALID_TABS.has(t) ? t : "wanted";
}

export const WantedContainer = memo(function WantedContainer({ onOpenOverview }: WantedContainerProps) {
  const setGlobalStatus = useGlobalStatus();
  const t = useTranslate();
  const client = useClient();

  // --- Tab state (synced with URL ?tab= param) ---
  const [tab, setTabRaw] = useState<WantedTab>(readTabFromUrl);

  const setTab = useCallback((next: WantedTab) => {
    setTabRaw(next);
    if (typeof window === "undefined") return;
    const params = new URLSearchParams(window.location.search);
    if (next === "wanted") {
      params.delete("tab");
    } else {
      params.set("tab", next);
    }
    const query = params.toString();
    const path = `${window.location.pathname}${query ? `?${query}` : ""}`;
    window.history.replaceState({}, "", path);
  }, []);

  // --- Wanted items state ---
  const [items, setItems] = useState<WantedItem[]>([]);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(false);
  const [statusFilter, setStatusFilter] = useState<string | undefined>(undefined);
  const [mediaTypeFilter, setMediaTypeFilter] = useState<string | undefined>(undefined);
  const [offset, setOffset] = useState(0);
  const limit = 50;

  const [expandedItemId, setExpandedItemId] = useState<string | null>(null);
  const [decisions, setDecisions] = useState<ReleaseDecisionItem[]>([]);
  const [decisionsLoading, setDecisionsLoading] = useState(false);

  const [, executeTriggerSearch] = useMutation(triggerWantedSearchMutation);
  const [, executePause] = useMutation(pauseWantedItemMutation);
  const [, executeResume] = useMutation(resumeWantedItemMutation);
  const [, executeReset] = useMutation(resetWantedItemMutation);

  // --- Cutoff state ---
  const [cutoffItems, setCutoffItems] = useState<CutoffUnmetItem[]>([]);
  const [cutoffLoading, setCutoffLoading] = useState(false);
  const [cutoffFacetFilter, setCutoffFacetFilter] = useState<string | undefined>(undefined);
  const [cutoffSearchingId, setCutoffSearchingId] = useState<string | null>(null);
  const [bulkSearching, setBulkSearching] = useState(false);
  const [bulkProgress, setBulkProgress] = useState<{ current: number; total: number } | null>(null);
  const bulkCancelRef = useRef(false);
  // Keep a ref to the full title list so search can access externalIds
  const titlesRef = useRef<TitleRecord[]>([]);

  // --- Calendar state ---
  type CalendarEpisodeItem = {
    id: string;
    titleId: string;
    titleName: string;
    titleFacet: string;
    seasonNumber: string | null;
    episodeNumber: string | null;
    episodeTitle: string | null;
    airDate: string | null;
    monitored: boolean;
  };
  const [calendarEpisodes, setCalendarEpisodes] = useState<CalendarEpisodeItem[]>([]);
  const [calendarLoading, setCalendarLoading] = useState(false);
  const [calendarRange, setCalendarRange] = useState<{ start: string; end: string } | null>(null);

  const refreshCalendar = useCallback(
    async (start: string, end: string) => {
      setCalendarLoading(true);
      try {
        const { data } = await client
          .query(calendarEpisodesQuery, { startDate: start, endDate: end })
          .toPromise();
        setCalendarEpisodes(data?.calendarEpisodes ?? []);
      } finally {
        setCalendarLoading(false);
      }
    },
    [client],
  );

  const handleCalendarDateRangeChange = useCallback(
    (start: string, end: string) => {
      setCalendarRange({ start, end });
    },
    [],
  );

  useEffect(() => {
    if (tab === "calendar" && calendarRange) {
      void refreshCalendar(calendarRange.start, calendarRange.end);
    }
  }, [tab, calendarRange, refreshCalendar]);

  // --- Pending releases state ---
  const [pendingItems, setPendingItems] = useState<PendingReleaseItem[]>([]);
  const [pendingLoading, setPendingLoading] = useState(false);
  const [, executeForceGrab] = useMutation(forceGrabPendingReleaseMutation);
  const [, executeDismiss] = useMutation(dismissPendingReleaseMutation);

  const refreshPending = useCallback(async () => {
    setPendingLoading(true);
    try {
      const { data, error } = await client.query(pendingReleasesQuery, {}).toPromise();
      if (error) throw error;
      setPendingItems(data?.pendingReleases ?? []);
    } catch (error) {
      const message = error instanceof Error ? error.message : t("status.failedToLoad");
      setGlobalStatus(message);
    } finally {
      setPendingLoading(false);
    }
  }, [client, t, setGlobalStatus]);

  useEffect(() => {
    if (tab === "pending") {
      void refreshPending();
    }
  }, [tab, refreshPending]);

  const forceGrabPending = useCallback(
    async (id: string) => {
      const { error } = await executeForceGrab({ input: { id } });
      if (error) {
        setGlobalStatus(error.message);
      } else {
        setGlobalStatus(t("pending.grabbed"));
        void refreshPending();
      }
    },
    [executeForceGrab, refreshPending, setGlobalStatus, t],
  );

  const dismissPending = useCallback(
    async (id: string) => {
      const { error } = await executeDismiss({ input: { id } });
      if (error) {
        setGlobalStatus(error.message);
      } else {
        setGlobalStatus(t("pending.dismissed"));
        void refreshPending();
      }
    },
    [executeDismiss, refreshPending, setGlobalStatus, t],
  );

  // --- Wanted data fetching ---

  const refreshItems = useCallback(async () => {
    setLoading(true);
    try {
      const { data, error } = await client
        .query(wantedItemsQuery, {
          status: statusFilter,
          mediaType: mediaTypeFilter,
          limit,
          offset,
        })
        .toPromise();
      if (error) throw error;
      setItems(data?.wantedItems?.items ?? []);
      setTotal(data?.wantedItems?.total ?? 0);
    } catch (error) {
      const message = error instanceof Error ? error.message : t("status.failedToLoad");
      setGlobalStatus(message);
    } finally {
      setLoading(false);
    }
  }, [client, statusFilter, mediaTypeFilter, offset, t, setGlobalStatus]);

  useEffect(() => {
    if (tab === "wanted") {
      void refreshItems();
    }
  }, [tab, refreshItems]);

  // --- Cutoff data fetching ---

  const refreshCutoff = useCallback(async () => {
    setCutoffLoading(true);
    try {
      // Fetch titles and quality profile settings in parallel
      const [titlesResult, systemSettingsResult, movieSettingsResult, seriesSettingsResult, animeSettingsResult] =
        await Promise.all([
          client.query(titlesQuery, {}).toPromise(),
          client.query(adminSettingsQuery, { scope: "system", category: "media" }).toPromise(),
          client.query(adminSettingsQuery, { scope: "system", scopeId: "movie", category: "media" }).toPromise(),
          client.query(adminSettingsQuery, { scope: "system", scopeId: "series", category: "media" }).toPromise(),
          client.query(adminSettingsQuery, { scope: "system", scopeId: "anime", category: "media" }).toPromise(),
        ]);

      if (titlesResult.error) throw titlesResult.error;

      const titles: TitleRecord[] = titlesResult.data?.titles ?? [];
      titlesRef.current = titles;

      // Parse quality profile catalog from system settings
      const systemItems: AdminSettingsItem[] = systemSettingsResult.data?.adminSettings?.items ?? [];
      const catalogRaw = extractSettingValue(systemItems, QUALITY_PROFILE_CATALOG_KEY);
      const globalProfileIdRaw = extractSettingValue(systemItems, QUALITY_PROFILE_ID_KEY);

      let catalogJson = "";
      if (catalogRaw) {
        try {
          catalogJson = typeof JSON.parse(catalogRaw) === "string" ? JSON.parse(catalogRaw) : catalogRaw;
        } catch {
          catalogJson = catalogRaw;
        }
      }

      const { entries } = resolveQualityProfileCatalogState(catalogJson);

      let globalProfileId: string | null = null;
      if (globalProfileIdRaw) {
        try {
          const parsed = JSON.parse(globalProfileIdRaw);
          globalProfileId = typeof parsed === "string" ? parsed : null;
        } catch {
          globalProfileId = globalProfileIdRaw;
        }
      }

      // Extract per-scope profile IDs
      const profileIdByScope: Record<string, string> = {};
      for (const [scopeId, result] of [
        ["movie", movieSettingsResult],
        ["series", seriesSettingsResult],
        ["anime", animeSettingsResult],
      ] as const) {
        const scopeItems: AdminSettingsItem[] = result.data?.adminSettings?.items ?? [];
        const raw = extractSettingValue(scopeItems, QUALITY_PROFILE_ID_KEY, scopeId);
        if (raw) {
          try {
            const parsed = JSON.parse(raw);
            if (typeof parsed === "string") profileIdByScope[scopeId] = parsed;
          } catch {
            profileIdByScope[scopeId] = raw;
          }
        }
      }

      const computed = computeCutoffUnmetItems(titles, entries, profileIdByScope, globalProfileId);
      setCutoffItems(computed);
    } catch (error) {
      const message = error instanceof Error ? error.message : t("status.failedToLoad");
      setGlobalStatus(message);
    } finally {
      setCutoffLoading(false);
    }
  }, [client, t, setGlobalStatus]);

  useEffect(() => {
    if (tab === "cutoff") {
      void refreshCutoff();
    }
  }, [tab, refreshCutoff]);

  // --- Wanted actions ---

  const loadDecisions = useCallback(
    async (wantedItemId: string) => {
      if (expandedItemId === wantedItemId) {
        setExpandedItemId(null);
        return;
      }
      setExpandedItemId(wantedItemId);
      setDecisionsLoading(true);
      try {
        const { data, error } = await client
          .query(releaseDecisionsQuery, { wantedItemId, limit: 20 })
          .toPromise();
        if (error) throw error;
        setDecisions(data?.releaseDecisions ?? []);
      } catch {
        setDecisions([]);
      } finally {
        setDecisionsLoading(false);
      }
    },
    [client, expandedItemId],
  );

  const triggerSearch = useCallback(
    async (id: string) => {
      const { error } = await executeTriggerSearch({ input: { wantedItemId: id } });
      if (error) {
        setGlobalStatus(error.message);
      } else {
        setGlobalStatus(t("wanted.searchTriggered"));
        void refreshItems();
      }
    },
    [executeTriggerSearch, refreshItems, setGlobalStatus, t],
  );

  const pauseItem = useCallback(
    async (id: string) => {
      const { error } = await executePause({ input: { wantedItemId: id } });
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
      const { error } = await executeResume({ input: { wantedItemId: id } });
      if (error) {
        setGlobalStatus(error.message);
      } else {
        void refreshItems();
      }
    },
    [executeResume, refreshItems, setGlobalStatus],
  );

  const resetItem = useCallback(
    async (id: string) => {
      const { error } = await executeReset({ input: { wantedItemId: id } });
      if (error) {
        setGlobalStatus(error.message);
      } else {
        void refreshItems();
      }
    },
    [executeReset, refreshItems, setGlobalStatus],
  );

  // --- Cutoff search actions ---

  const searchAndQueueTitle = useCallback(
    async (cutoffItem: CutoffUnmetItem) => {
      const title = titlesRef.current.find((t) => t.id === cutoffItem.id);
      if (!title) return;

      const imdbId =
        title.externalIds
          ?.find((e) => e.source.toLowerCase() === "imdb")
          ?.value?.trim() || null;
      const tvdbId =
        title.externalIds
          ?.find((e) => e.source.toLowerCase() === "tvdb")
          ?.value?.trim() || null;

      const { data, error } = await client
        .query(searchQuery, {
          query: title.name,
          imdbId,
          tvdbId,
          category: title.facet === "movie" ? "movie" : title.facet === "tv" ? "tv" : "anime",
          limit: title.facet === "movie" ? 50 : 15,
        })
        .toPromise();

      if (error) throw error;

      const results: Release[] = data?.searchIndexers ?? [];
      const top = results.find((r) => r.qualityProfileDecision?.allowed ?? true);
      if (!top) {
        setGlobalStatus(t("status.noReleaseForTitle", { name: title.name }));
        return;
      }

      const sourceHint = top.downloadUrl || top.link;
      if (!sourceHint) {
        setGlobalStatus(t("status.noSource", { name: title.name }));
        return;
      }

      const { error: queueError } = await client
        .mutation(queueExistingMutation, {
          input: {
            titleId: title.id,
            sourceHint,
            sourceKind: top.sourceKind ?? null,
            sourceTitle: top.title,
          },
        })
        .toPromise();

      if (queueError) throw queueError;
      setGlobalStatus(t("cutoff.searchTriggered", { name: title.name }));
    },
    [client, t, setGlobalStatus],
  );

  const cutoffTriggerSearch = useCallback(
    async (item: CutoffUnmetItem) => {
      setCutoffSearchingId(item.id);
      try {
        await searchAndQueueTitle(item);
      } catch (error) {
        setGlobalStatus(error instanceof Error ? error.message : t("status.queueFailed"));
      } finally {
        setCutoffSearchingId(null);
      }
    },
    [searchAndQueueTitle, setGlobalStatus, t],
  );

  const cutoffBulkSearch = useCallback(() => {
    bulkCancelRef.current = false;
    setBulkSearching(true);

    const filtered = cutoffFacetFilter
      ? cutoffItems.filter((i) => i.facet === cutoffFacetFilter)
      : cutoffItems;

    setBulkProgress({ current: 0, total: filtered.length });

    void (async () => {
      let searched = 0;
      for (const item of filtered) {
        if (bulkCancelRef.current) break;
        searched++;
        setBulkProgress({ current: searched, total: filtered.length });
        try {
          await searchAndQueueTitle(item);
        } catch {
          // continue to next title on error
        }
      }
      setBulkSearching(false);
      setBulkProgress(null);
      setGlobalStatus(t("cutoff.bulkComplete", { searched, total: filtered.length }));
    })();
  }, [cutoffItems, cutoffFacetFilter, searchAndQueueTitle, setGlobalStatus, t]);

  const cancelBulkSearch = useCallback(() => {
    bulkCancelRef.current = true;
  }, []);

  const handleCalendarEpisodeClick = useCallback(
    (episode: CalendarEpisodeItem) => {
      const facet = FACETS_BY_ID.get(episode.titleFacet as import("@/lib/types/titles").Facet);
      if (!facet || !onOpenOverview) return;
      onOpenOverview(facet.viewId as ViewId, episode.titleId, episode.id);
    },
    [onOpenOverview],
  );

  return (
    <WantedView
      tab={tab}
      onTabChange={setTab}
      wantedState={{
        items,
        total,
        loading,
        statusFilter,
        setStatusFilter,
        mediaTypeFilter,
        setMediaTypeFilter,
        offset,
        setOffset,
        limit,
        refreshItems,
        expandedItemId,
        decisions,
        decisionsLoading,
        loadDecisions,
        triggerSearch,
        pauseItem,
        resumeItem,
        resetItem,
      }}
      cutoffState={{
        items: cutoffItems,
        loading: cutoffLoading,
        facetFilter: cutoffFacetFilter,
        setFacetFilter: setCutoffFacetFilter,
        searchingId: cutoffSearchingId,
        bulkSearching,
        bulkProgress,
        triggerSearch: cutoffTriggerSearch,
        triggerBulkSearch: cutoffBulkSearch,
        cancelBulkSearch,
      }}
      calendarState={{
        episodes: calendarEpisodes,
        loading: calendarLoading,
        onDateRangeChange: handleCalendarDateRangeChange,
        onEpisodeClick: handleCalendarEpisodeClick,
      }}
      pendingState={{
        items: pendingItems,
        loading: pendingLoading,
        refreshItems: refreshPending,
        forceGrab: forceGrabPending,
        dismiss: dismissPending,
      }}
    />
  );
});

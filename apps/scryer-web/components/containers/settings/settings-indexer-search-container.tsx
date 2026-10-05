// State and data for the Indexers › Search pane (spec 0002, WP4).
// The search itself is the existing interactive-release-search job: one start,
// one poll loop, one cancel. Everything the pane shows on top of it — facets,
// sorting, the advanced limits and the retry merge — is derived here from the
// snapshots that loop reports, so partial results render as they arrive.
import * as React from "react";
import { useSearchParams } from "react-router";
import { useClient } from "urql";

import { GrabDialog } from "@/components/common/grab-dialog";
import {
  SettingsIndexerSearchSection,
  type IndexerSearchAdvancedLimits,
  type IndexerSearchIndexerOption,
} from "@/components/views/settings/settings-indexer-search-section";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { indexersQuery } from "@/lib/graphql/queries";
import {
  runIterativeReleaseSearch,
  type InteractiveSearchIndexerProgress,
} from "@/lib/graphql/release-search";
import type { IndexerRecord, Release } from "@/lib/types";
import {
  earliestSearchExpiry,
  expireIndexerSearches,
  indexerSearchExpiresAt,
} from "@/lib/utils/indexer-search-expiry";
import {
  downloadIndexerSearchArtifacts,
  type IndexerSearchArtifactTarget,
} from "@/lib/utils/indexer-search-download";
import {
  addSavedIndexerSearch,
  buildIndexerSearchFacets,
  downloadableReleases,
  filterIndexerSearchReleases,
  indexerSearchRowKey,
  mergeIndexerProgress,
  mergeIndexerSearchReleases,
  parseCategoryList,
  rawSearchPageSize,
  readSavedIndexerSearches,
  releaseSizeBoundsGiB,
  sortIndexerSearchReleases,
  summarizeIndexerHealth,
  writeSavedIndexerSearches,
  type IndexerSearchSortKey,
  type SavedIndexerSearch,
} from "@/lib/utils/indexer-search";

/** Stable identity for the closed grab dialog, so it never re-renders on it. */
const NO_GRAB_TARGETS: Release[] = [];

const EMPTY_ADVANCED: IndexerSearchAdvancedLimits = {
  minSizeGiB: "",
  maxSizeGiB: "",
  minSeeders: "",
  maxAgeDays: "",
  limit: "",
};

function positiveNumberOrNull(raw: string): number | null {
  const value = Number(raw.trim());
  if (!raw.trim() || Number.isNaN(value) || value < 0) {
    return null;
  }
  return value;
}

export function SettingsIndexerSearchContainer() {
  const client = useClient();
  const t = useTranslate();
  const setGlobalStatus = useGlobalStatus();
  const [searchParams] = useSearchParams();
  const presetIndexerId = searchParams.get("indexer");

  const [indexerOptions, setIndexerOptions] = React.useState<
    IndexerSearchIndexerOption[]
  >([]);
  const [query, setQuery] = React.useState("");
  const [selectedIndexerIds, setSelectedIndexerIds] = React.useState<string[]>(
    presetIndexerId ? [presetIndexerId] : [],
  );
  const [categories, setCategories] = React.useState("");
  const [advancedOpen, setAdvancedOpen] = React.useState(false);
  const [advanced, setAdvanced] =
    React.useState<IndexerSearchAdvancedLimits>(EMPTY_ADVANCED);
  const [savedSearches, setSavedSearches] = React.useState<SavedIndexerSearch[]>(
    [],
  );

  const [releases, setReleases] = React.useState<Release[]>([]);
  const [indexers, setIndexers] = React.useState<
    InteractiveSearchIndexerProgress[]
  >([]);
  const [searching, setSearching] = React.useState(false);
  const [hasSearched, setHasSearched] = React.useState(false);
  // Frozen per snapshot so ages and age sorting stay pure across renders.
  const [nowMs, setNowMs] = React.useState(() => Date.now());

  const [selectedFacets, setSelectedFacets] = React.useState<string[]>([]);
  const [sizeRangeGiB, setSizeRangeGiB] = React.useState<
    [number, number] | null
  >(null);
  const [sort, setSort] = React.useState<IndexerSearchSortKey>("age-asc");
  const [selectedRowKeys, setSelectedRowKeys] = React.useState<string[]>([]);
  const [expandedRowKey, setExpandedRowKey] = React.useState<string | null>(
    null,
  );
  const [grabTargets, setGrabTargets] = React.useState<Release[] | null>(null);
  const [downloading, setDownloading] = React.useState(false);
  // A grab names its release by (searchId, downloadUrl), and "retry failed"
  // mints a second job whose rows merge into this same table, so each row keeps
  // the id of the job it actually arrived on.
  const [searchIdByRowKey, setSearchIdByRowKey] = React.useState<
    ReadonlyMap<string, string>
  >(() => new Map());

  const searchAbortRef = React.useRef<AbortController | null>(null);
  // The search the current controller is polling, once its first snapshot names it.
  const activeSearchIdRef = React.useRef<string | null>(null);
  const deadlinesRef = React.useRef(new Map<string, number>());
  // Mirrors searchIdByRowKey so expiry can prune rows synchronously.
  const rowOwnersRef = React.useRef<ReadonlyMap<string, string>>(new Map());
  const [expiresAt, setExpiresAt] = React.useState<number | null>(null);

  const publishRowOwners = React.useCallback((next: ReadonlyMap<string, string>) => {
    rowOwnersRef.current = next;
    setSearchIdByRowKey(next);
  }, []);

  // Each search expires on its own deadline; returns whether any did.
  const expireResults = React.useCallback(() => {
    const expiry = expireIndexerSearches({
      deadlines: deadlinesRef.current,
      rowOwners: rowOwnersRef.current,
      activeSearchId: activeSearchIdRef.current,
      now: Date.now(),
    });
    if (!expiry) return false;
    deadlinesRef.current = expiry.deadlines;
    if (expiry.activeExpired) {
      searchAbortRef.current?.abort();
      searchAbortRef.current = null;
      activeSearchIdRef.current = null;
      setSearching(false);
    }
    if (expiry.deadlines.size === 0 && searchAbortRef.current === null) {
      setExpiresAt(null);
      setReleases([]);
      setIndexers([]);
      publishRowOwners(new Map());
      setSelectedRowKeys([]);
      setExpandedRowKey(null);
      setGrabTargets(null);
      setSelectedFacets([]);
      setSizeRangeGiB(null);
      setSearching(false);
      setHasSearched(false);
      return true;
    }
    const { isRowLive } = expiry;
    setExpiresAt(earliestSearchExpiry(expiry.deadlines));
    setReleases((current) =>
      current.filter((release) => isRowLive(indexerSearchRowKey(release))),
    );
    publishRowOwners(expiry.rowOwners);
    setSelectedRowKeys((current) => current.filter(isRowLive));
    setExpandedRowKey((current) =>
      current !== null && isRowLive(current) ? current : null,
    );
    setGrabTargets((current) =>
      current?.every((release) => isRowLive(indexerSearchRowKey(release)))
        ? current
        : null,
    );
    return true;
  }, [publishRowOwners]);

  React.useEffect(() => {
    if (expiresAt === null) return;
    const timer = window.setTimeout(expireResults, Math.max(0, expiresAt - Date.now()));
    // Background tabs can suspend timers. Recheck before users resume work.
    window.addEventListener("focus", expireResults);
    document.addEventListener("visibilitychange", expireResults);
    return () => {
      window.clearTimeout(timer);
      window.removeEventListener("focus", expireResults);
      document.removeEventListener("visibilitychange", expireResults);
    };
  }, [expireResults, expiresAt]);

  React.useEffect(() => {
    setSavedSearches(readSavedIndexerSearches());
  }, []);

  React.useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const { data, error } = await client
          .query(indexersQuery, {}, { requestPolicy: "cache-first" })
          .toPromise();
        if (error) throw error;
        if (cancelled) {
          return;
        }
        const records = (data?.indexers ?? []) as IndexerRecord[];
        // A raw query asks every enabled indexer; the interactive-search flag
        // only scopes title searches.
        setIndexerOptions(
          records
            .filter((record) => record.isEnabled && !record.supportsManagedChildrenSync)
            .map((record) => ({ id: record.id, name: record.name })),
        );
      } catch (error) {
        if (cancelled) {
          return;
        }
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.failedToLoad"),
        );
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [client, setGlobalStatus, t]);

  React.useEffect(
    () => () => {
      searchAbortRef.current?.abort();
    },
    [],
  );

  const runSearch = React.useCallback(
    async (retryIndexerIds?: string[]) => {
      const trimmedQuery = query.trim();
      if (!trimmedQuery) {
        return;
      }
      const isRetry = retryIndexerIds != null;
      if (isRetry && expireResults()) return;
      const baseIndexers = isRetry ? indexers : [];

      searchAbortRef.current?.abort();
      const controller = new AbortController();
      searchAbortRef.current = controller;
      activeSearchIdRef.current = null;

      if (!isRetry) {
        deadlinesRef.current = new Map();
        setExpiresAt(null);
        setGrabTargets(null);
        setReleases([]);
        setIndexers([]);
        setSelectedRowKeys([]);
        setExpandedRowKey(null);
        setSelectedFacets([]);
        setSizeRangeGiB(null);
        publishRowOwners(new Map());
      }
      setHasSearched(true);
      setSearching(true);

      const scopedIndexerIds = retryIndexerIds ?? selectedIndexerIds;
      const parsedCategories = parseCategoryList(categories);

      try {
        await runIterativeReleaseSearch(
          client,
          {
            query: trimmedQuery,
            indexerIds:
              scopedIndexerIds.length > 0 ? scopedIndexerIds : undefined,
            categories:
              parsedCategories.length > 0 ? parsedCategories : undefined,
            limit: rawSearchPageSize(advanced.limit),
          },
          {
            signal: controller.signal,
            onUpdate: (snapshot) => {
              if (controller.signal.aborted || searchAbortRef.current !== controller) return;
              // Earlier searches may expire while this one runs; only this
              // search's own deadline stops it.
              expireResults();
              if (controller.signal.aborted) return;
              activeSearchIdRef.current = snapshot.searchId;
              deadlinesRef.current.set(snapshot.searchId, indexerSearchExpiresAt(snapshot));
              expireResults();
              if (controller.signal.aborted) return;
              setExpiresAt(earliestSearchExpiry(deadlinesRef.current));
              setNowMs(Date.now());
              const nextOwners = new Map(rowOwnersRef.current);
              for (const release of snapshot.releases) {
                nextOwners.set(indexerSearchRowKey(release), snapshot.searchId);
              }
              publishRowOwners(nextOwners);
              // A retry merges into the rows still live, so rows of a search
              // that expired meanwhile do not come back.
              setReleases((current) =>
                isRetry
                  ? mergeIndexerSearchReleases(current, snapshot.releases)
                  : snapshot.releases,
              );
              setIndexers(
                isRetry
                  ? mergeIndexerProgress(baseIndexers, snapshot.indexers)
                  : snapshot.indexers,
              );
            },
          },
        );
      } catch (error) {
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.apiError"),
        );
      } finally {
        if (searchAbortRef.current === controller) {
          searchAbortRef.current = null;
          activeSearchIdRef.current = null;
          setSearching(false);
        }
      }
    },
    [
      advanced.limit,
      categories,
      client,
      expireResults,
      indexers,
      publishRowOwners,
      query,
      selectedIndexerIds,
      setGlobalStatus,
      t,
    ],
  );

  const handleSearch = React.useCallback(() => {
    void runSearch();
  }, [runSearch]);

  const handleCancelSearch = React.useCallback(() => {
    searchAbortRef.current?.abort();
    searchAbortRef.current = null;
    activeSearchIdRef.current = null;
    setSearching(false);
  }, []);

  const handleRetryFailed = React.useCallback(() => {
    const failed = summarizeIndexerHealth(indexers).failedIndexerIds;
    if (failed.length === 0) {
      return;
    }
    void runSearch(failed);
  }, [indexers, runSearch]);

  const handleToggleFacet = React.useCallback((facetKey: string) => {
    setSelectedFacets((current) =>
      current.includes(facetKey)
        ? current.filter((entry) => entry !== facetKey)
        : [...current, facetKey],
    );
  }, []);

  const handleResetRefine = React.useCallback(() => {
    setSelectedFacets([]);
    setSizeRangeGiB(null);
  }, []);

  const handleToggleRow = React.useCallback((release: Release) => {
    const key = indexerSearchRowKey(release);
    setSelectedRowKeys((current) =>
      current.includes(key)
        ? current.filter((entry) => entry !== key)
        : [...current, key],
    );
  }, []);

  const handleToggleExpanded = React.useCallback((release: Release) => {
    const key = indexerSearchRowKey(release);
    setExpandedRowKey((current) => (current === key ? null : key));
  }, []);

  const handleSaveSearch = React.useCallback(() => {
    setSavedSearches((current) => {
      const next = addSavedIndexerSearch(current, {
        query,
        indexerIds: selectedIndexerIds,
        categories: parseCategoryList(categories),
      });
      writeSavedIndexerSearches(next);
      return next;
    });
  }, [categories, query, selectedIndexerIds]);

  const handleApplySavedSearch = React.useCallback(
    (index: number) => {
      const entry = savedSearches[index];
      if (!entry) {
        return;
      }
      setQuery(entry.query);
      setSelectedIndexerIds(entry.indexerIds);
      setCategories(entry.categories.join(", "));
    },
    [savedSearches],
  );

  const handleRemoveSavedSearch = React.useCallback((index: number) => {
    setSavedSearches((current) => {
      const next = current.filter((_, position) => position !== index);
      writeSavedIndexerSearches(next);
      return next;
    });
  }, []);

  const handleGrab = React.useCallback((grabbed: Release[]) => {
    if (expireResults()) return;
    setGrabTargets(grabbed.length > 0 ? grabbed : null);
  }, [expireResults]);

  // Rows stay in the table after a grab: the same release may legitimately be
  // grabbed again for a second title.
  const handleGrabbed = React.useCallback(() => {
    setSelectedRowKeys([]);
  }, []);

  // The raw file(s) go straight to the browser (D17): nothing is queued, so
  // success needs no toast — the browser's own download is the confirmation.
  const handleDownload = React.useCallback(
    (targets: Release[]) => {
      if (expireResults()) return;
      const downloadable = downloadableReleases(targets);
      if (downloadable.length === 0) {
        return;
      }
      // "Retry failed" mints a second job whose rows merge into this table, so
      // each release names its own job and the whole selection is one file.
      const releases: IndexerSearchArtifactTarget[] = [];
      for (const release of downloadable) {
        const searchId = searchIdByRowKey.get(indexerSearchRowKey(release));
        const downloadUrl = release.downloadUrl ?? release.link;
        if (searchId && downloadUrl) {
          releases.push({ searchId, downloadUrl });
        }
      }
      if (releases.length === 0) {
        return;
      }

      setDownloading(true);
      void (async () => {
        try {
          await downloadIndexerSearchArtifacts({
            releases,
            failureMessage: t("status.failedToLoad"),
          });
        } catch (error) {
          setGlobalStatus(
            error instanceof Error ? error.message : t("status.failedToLoad"),
          );
        } finally {
          setDownloading(false);
        }
      })();
    },
    [expireResults, searchIdByRowKey, setGlobalStatus, t],
  );

  const facetGroups = React.useMemo(
    () => buildIndexerSearchFacets(releases),
    [releases],
  );
  const sizeBoundsGiB = React.useMemo(
    () => releaseSizeBoundsGiB(releases),
    [releases],
  );
  const filteredReleases = React.useMemo(
    () =>
      filterIndexerSearchReleases(
        releases,
        {
          facets: selectedFacets,
          minSizeGiB: positiveNumberOrNull(advanced.minSizeGiB),
          maxSizeGiB: positiveNumberOrNull(advanced.maxSizeGiB),
          minSeeders: positiveNumberOrNull(advanced.minSeeders),
          maxAgeDays: positiveNumberOrNull(advanced.maxAgeDays),
          sizeRangeGiB,
        },
        nowMs,
      ),
    [
      advanced.maxAgeDays,
      advanced.maxSizeGiB,
      advanced.minSeeders,
      advanced.minSizeGiB,
      nowMs,
      releases,
      selectedFacets,
      sizeRangeGiB,
    ],
  );
  const rows = React.useMemo(
    () => sortIndexerSearchReleases(filteredReleases, sort),
    [filteredReleases, sort],
  );
  const savedSearchLabels = React.useMemo(
    () => savedSearches.map((entry) => entry.query),
    [savedSearches],
  );

  return (
    <>
      <SettingsIndexerSearchSection
        query={query}
        onQueryChange={setQuery}
        indexerOptions={indexerOptions}
        selectedIndexerIds={selectedIndexerIds}
        onSelectedIndexerIdsChange={setSelectedIndexerIds}
        categories={categories}
        onCategoriesChange={setCategories}
        advancedOpen={advancedOpen}
        onAdvancedOpenChange={setAdvancedOpen}
        advanced={advanced}
        onAdvancedChange={setAdvanced}
        savedSearchLabels={savedSearchLabels}
        onSaveSearch={handleSaveSearch}
        onApplySavedSearch={handleApplySavedSearch}
        onRemoveSavedSearch={handleRemoveSavedSearch}
        onSearch={handleSearch}
        onCancelSearch={handleCancelSearch}
        searching={searching}
        hasSearched={hasSearched}
        indexers={indexers}
        onRetryFailed={handleRetryFailed}
        facetGroups={facetGroups}
        selectedFacets={selectedFacets}
        onToggleFacet={handleToggleFacet}
        onResetRefine={handleResetRefine}
        sizeBoundsGiB={sizeBoundsGiB}
        sizeRangeGiB={sizeRangeGiB}
        onSizeRangeChange={setSizeRangeGiB}
        sort={sort}
        onSortChange={setSort}
        matchedCount={releases.length}
        rows={rows}
        nowMs={nowMs}
        selectedRowKeys={selectedRowKeys}
        onToggleRow={handleToggleRow}
        expandedRowKey={expandedRowKey}
        onToggleExpanded={handleToggleExpanded}
        onGrab={handleGrab}
        onDownload={handleDownload}
        downloading={downloading}
      />
      <GrabDialog
        open={grabTargets !== null}
        onOpenChange={(open) => {
          if (!open) {
            setGrabTargets(null);
          }
        }}
        releases={grabTargets ?? NO_GRAB_TARGETS}
        searchIdByRowKey={searchIdByRowKey}
        onGrabbed={handleGrabbed}
      />
    </>
  );
}

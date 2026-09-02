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
  type InteractiveSearchKind,
} from "@/lib/graphql/release-search";
import type { IndexerRecord, Release } from "@/lib/types";
import {
  downloadIndexerSearchArtifacts,
  type IndexerSearchArtifactTarget,
} from "@/lib/utils/indexer-search-download";
import {
  addSavedIndexerSearch,
  buildIndexerSearchFacets,
  downloadableReleases,
  filterIndexerSearchReleases,
  indexerPriorityByName,
  indexerSearchRowKey,
  isReleaseRejected,
  mergeIndexerProgress,
  mergeIndexerSearchReleases,
  parseCategoryList,
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
  const [kind, setKind] = React.useState<InteractiveSearchKind>("MOVIE");
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
  const [sort, setSort] = React.useState<IndexerSearchSortKey>("newest");
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
        setIndexerOptions(
          records
            .filter(
              (record) =>
                record.isEnabled &&
                record.enableInteractiveSearch &&
                !record.supportsManagedChildrenSync,
            )
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
      const baseReleases = isRetry ? releases : [];
      const baseIndexers = isRetry ? indexers : [];

      searchAbortRef.current?.abort();
      const controller = new AbortController();
      searchAbortRef.current = controller;

      if (!isRetry) {
        setReleases([]);
        setIndexers([]);
        setSelectedRowKeys([]);
        setExpandedRowKey(null);
        setSelectedFacets([]);
        setSizeRangeGiB(null);
        setSearchIdByRowKey(new Map());
      }
      setHasSearched(true);
      setSearching(true);

      const scopedIndexerIds = retryIndexerIds ?? selectedIndexerIds;
      const parsedCategories = parseCategoryList(categories);
      const limit = positiveNumberOrNull(advanced.limit);

      try {
        await runIterativeReleaseSearch(
          client,
          {
            query: trimmedQuery,
            kind,
            indexerIds:
              scopedIndexerIds.length > 0 ? scopedIndexerIds : undefined,
            categories:
              parsedCategories.length > 0 ? parsedCategories : undefined,
            limit: limit ?? undefined,
          },
          {
            signal: controller.signal,
            onUpdate: (snapshot) => {
              setNowMs(Date.now());
              setSearchIdByRowKey((current) => {
                const next = new Map(current);
                for (const release of snapshot.releases) {
                  next.set(indexerSearchRowKey(release), snapshot.searchId);
                }
                return next;
              });
              setReleases(
                isRetry
                  ? mergeIndexerSearchReleases(baseReleases, snapshot.releases)
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
          setSearching(false);
        }
      }
    },
    [
      advanced.limit,
      categories,
      client,
      indexers,
      kind,
      query,
      releases,
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
        kind,
        indexerIds: selectedIndexerIds,
        categories: parseCategoryList(categories),
      });
      writeSavedIndexerSearches(next);
      return next;
    });
  }, [categories, kind, query, selectedIndexerIds]);

  const handleApplySavedSearch = React.useCallback(
    (index: number) => {
      const entry = savedSearches[index];
      if (!entry) {
        return;
      }
      setQuery(entry.query);
      setKind(entry.kind as InteractiveSearchKind);
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
    setGrabTargets(grabbed.length > 0 ? grabbed : null);
  }, []);

  // Rows stay in the table after a grab: the same release may legitimately be
  // grabbed again for a second title.
  const handleGrabbed = React.useCallback(() => {
    setSelectedRowKeys([]);
  }, []);

  // The raw file(s) go straight to the browser (D17): nothing is queued, so
  // success needs no toast — the browser's own download is the confirmation.
  const handleDownload = React.useCallback(
    (targets: Release[]) => {
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
    [searchIdByRowKey, setGlobalStatus, t],
  );

  const facetGroups = React.useMemo(
    () => buildIndexerSearchFacets(releases),
    [releases],
  );
  const sizeBoundsGiB = React.useMemo(
    () => releaseSizeBoundsGiB(releases),
    [releases],
  );
  const priorityByIndexer = React.useMemo(
    () => indexerPriorityByName(indexers),
    [indexers],
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
    () => sortIndexerSearchReleases(filteredReleases, sort, priorityByIndexer),
    [filteredReleases, priorityByIndexer, sort],
  );
  const passingCount = React.useMemo(
    () => rows.filter((release) => !isReleaseRejected(release)).length,
    [rows],
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
        kind={kind}
        onKindChange={setKind}
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
        passingCount={passingCount}
        rows={rows}
        priorityByIndexer={priorityByIndexer}
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
        initialQuery={query}
        kind={kind}
        onGrabbed={handleGrabbed}
      />
    </>
  );
}
